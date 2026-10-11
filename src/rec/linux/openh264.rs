//! Cisco's OpenH264, for MP4 on Linux without ffmpeg: downloaded on the
//! user's request ([`download`]) from Cisco (the only URLs allowed are the
//! ones pinned in [`crate::rec::openh264_pins`]), verified, unpacked and
//! kept in `$XDG_DATA_HOME/rustshot/openh264/` (mode 0700), then loaded
//! with `dlopen` by that absolute path only ([`OpenH264::load`]).
//!
//! Download and verification: the `.bz2` is fetched with curl (plain HTTP,
//! no redirects, cut off past the pinned size) into the private folder;
//! its SHA-256 must be the pinned one *before* it is unpacked. It is
//! unpacked by the system's `/usr/bin/bzip2 -dc` (a fixed path, run
//! without a shell, the output cut off past the pinned size; without it
//! the user is told "bzip2 is needed to unpack OpenH264"). The library's
//! own SHA-256 is checked after unpacking and again every time before it
//! is loaded, and the loaded library must report the pinned version.
//!
//! The encoder ([`H264Enc`]): I420 in (BT.709, limited range, signalled in
//! the SPS), Annex-B out; bit rate by quality ([`crate::rec::bitrate`]),
//! an IDR every 2 s, one SPS/PPS id for the whole stream (so one `avcC`
//! describes it), no frame skipping.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use crate::rec::openh264_pins::Pin;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The bzip2 used to unpack the download (a fixed path, not `PATH`).
pub const BZIP2: &str = "/usr/bin/bzip2";
/// What the user is told without it.
pub const NO_BZIP2: &str = "bzip2 is needed to unpack OpenH264";

/// Where the library is kept: `$XDG_DATA_HOME/rustshot/openh264` (when
/// that is absolute), else `~/.local/share/rustshot/openh264`.
pub fn dir_from(xdg_data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match xdg_data_home.map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(p) => p,
        None => PathBuf::from(home?).join(".local/share"),
    };
    base.is_absolute().then(|| base.join("rustshot").join("openh264"))
}

/// [`dir_from`] for this user.
pub fn dir() -> Option<PathBuf> {
    dir_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

/// The unpacked library of `pin` in `dir`.
pub fn lib_path(dir: &Path, pin: &Pin) -> PathBuf {
    dir.join(pin.so)
}

/// The downloaded library for this machine, if it is there (by its size;
/// the hash is checked when it is loaded).
pub fn installed() -> Option<PathBuf> {
    let pin = crate::rec::openh264_pins::this_machine()?;
    let p = lib_path(&dir()?, pin);
    std::fs::metadata(&p).ok().filter(|m| m.is_file() && m.len() == pin.so_size).map(|_| p)
}

fn hex_digest(p: &Path) -> Result<String, String> {
    crate::sha256::file_digest(p).map(|d| crate::sha256::hex(&d)).map_err(|e| format!("read {}: {e}", p.display()))
}

/// Check the downloaded `bz2` against `pin`, unpack it with `unpack(src,
/// dst)` into `dest` (through `dest.part`) and check the result. The
/// download is refused (and deleted) unless its SHA-256 is the pinned one,
/// before anything reads it; a library that does not match is deleted.
/// The download is removed in every case.
pub fn verify_and_unpack(
    bz2: &Path,
    pin: &Pin,
    dest: &Path,
    unpack: impl FnOnce(&Path, &Path, u64) -> Result<(), String>,
) -> Result<(), String> {
    let r = (|| {
        let len = std::fs::metadata(bz2).map_err(|e| format!("read {}: {e}", bz2.display()))?.len();
        if len != pin.bz2_size || hex_digest(bz2)? != pin.bz2_sha256 {
            return Err(format!("{} failed its SHA-256 check; refusing it", pin.file));
        }
        let mut part = dest.as_os_str().to_os_string();
        part.push(".part");
        let part = PathBuf::from(part);
        let _ = std::fs::remove_file(&part);
        let r = unpack(bz2, &part, pin.so_size).and_then(|()| {
            let len = std::fs::metadata(&part).map_err(|e| e.to_string())?.len();
            if len != pin.so_size || hex_digest(&part)? != pin.so_sha256 {
                return Err(format!("{} does not match its SHA-256 after unpacking; refusing it", pin.so));
            }
            std::fs::rename(&part, dest).map_err(|e| format!("move {} into place: {e}", pin.so))
        });
        if r.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        r
    })();
    let _ = std::fs::remove_file(bz2);
    r
}

/// Whether `lib` is the pinned library (size and SHA-256).
pub fn verify_lib(lib: &Path, pin: &Pin) -> Result<(), String> {
    let len = std::fs::metadata(lib).map_err(|e| format!("{}: {e}", lib.display()))?.len();
    if len != pin.so_size || hex_digest(lib)? != pin.so_sha256 {
        return Err(format!("{} does not match its SHA-256; download OpenH264 again", lib.display()));
    }
    Ok(())
}

/// Bit rate for the picture: the quality's (as Media Foundation and
/// ffmpeg's openh264 use), and an IDR every two seconds.
pub fn intra_period(fps: u32) -> u32 {
    fps.max(1) * 2
}

#[cfg(target_os = "linux")]
pub use imp::{download, H264Enc, OpenH264};
#[cfg(all(target_os = "linux", test))]
pub use imp::unpack_bzip2;

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::rec::linux::{child, dl::Lib};
    use crate::rec::openh264_pins::{self, VERSION};
    use anyhow::{anyhow, bail, Result};
    use core::ffi::{c_int, c_uint, c_void};
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::Arc;

    /// Unpack `src` (bzip2) into `dst` with [`BZIP2`], at most `max`
    /// bytes (the pinned size; more is refused).
    pub fn unpack_bzip2(src: &Path, dst: &Path, max: u64) -> Result<(), String> {
        if !Path::new(BZIP2).is_file() {
            return Err(NO_BZIP2.into());
        }
        let input = std::fs::File::open(src).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(BZIP2);
        cmd.arg("-dc").stdin(input).stdout(Stdio::piped()).stderr(Stdio::null());
        child::prepare(&mut cmd, None);
        let mut ch = cmd.spawn().map_err(|e| format!("run {BZIP2}: {e}"))?;
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        let mut got = 0u64;
        let mut buf = [0u8; 64 * 1024];
        let mut r: Result<(), String> = Ok(());
        if let Some(mut so) = ch.stdout.take() {
            loop {
                match so.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        got += n as u64;
                        if got > max {
                            r = Err("the unpacked OpenH264 is larger than expected; refusing it".into());
                            break;
                        }
                        if let Err(e) = out.write_all(&buf[..n]) {
                            r = Err(e.to_string());
                            break;
                        }
                    }
                    Err(e) => {
                        r = Err(e.to_string());
                        break;
                    }
                }
            }
        }
        if r.is_err() {
            let _ = ch.kill(); // our own child, by its PID
        }
        let st = ch.wait().map_err(|e| e.to_string())?;
        r?;
        if !st.success() {
            return Err(format!("{BZIP2} could not unpack OpenH264 ({st})"));
        }
        out.sync_all().map_err(|e| e.to_string())
    }

    /// Download, verify and unpack the pinned OpenH264 for this machine;
    /// `progress(got, total)` returning false cancels. The library's path.
    pub fn download(progress: &dyn Fn(u64, Option<u64>) -> bool) -> Result<PathBuf, String> {
        let pin = openh264_pins::this_machine()
            .ok_or_else(|| format!("there is no OpenH264 download for this machine ({})", std::env::consts::ARCH))?;
        let dir = dir().ok_or("no home folder to keep OpenH264 in")?;
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let url = pin.url();
        if !openh264_pins::is_pinned_url(&url) {
            return Err("refusing to download from an unexpected URL".into());
        }
        let bz2 = dir.join(format!("{}.download", pin.file));
        let total = Some(pin.bz2_size);
        crate::export::download_http_exact(&url, &bz2, pin.bz2_size, &|got, _| got <= pin.bz2_size && progress(got, total))?;
        let dest = lib_path(&dir, pin);
        verify_and_unpack(&bz2, pin, &dest, unpack_bzip2)?;
        Ok(dest)
    }

    // --- The C API (codec_api.h / codec_app_def.h of OpenH264 2.6.0) ------

    const MAX_SLICES_NUM_TMP: usize = 35;
    const MAX_SPATIAL_LAYER_NUM: usize = 4;
    const MAX_LAYER_NUM_OF_FRAME: usize = 128;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SliceArgument {
        mode: c_int,
        num: c_uint,
        mb_num: [c_uint; MAX_SLICES_NUM_TMP],
        size_constraint: c_uint,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SpatialLayerConfig {
        video_width: c_int,
        video_height: c_int,
        frame_rate: f32,
        spatial_bitrate: c_int,
        max_spatial_bitrate: c_int,
        profile_idc: c_int,
        level_idc: c_int,
        dlayer_qp: c_int,
        slice: SliceArgument,
        video_signal_type_present: bool,
        video_format: u8,
        full_range: bool,
        color_description_present: bool,
        color_primaries: u8,
        transfer_characteristics: u8,
        color_matrix: u8,
        aspect_ratio_present: bool,
        aspect_ratio: c_int,
        aspect_ratio_ext_width: u16,
        aspect_ratio_ext_height: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EncParamExt {
        usage_type: c_int,
        pic_width: c_int,
        pic_height: c_int,
        target_bitrate: c_int,
        rc_mode: c_int,
        max_frame_rate: f32,
        temporal_layer_num: c_int,
        spatial_layer_num: c_int,
        spatial_layers: [SpatialLayerConfig; MAX_SPATIAL_LAYER_NUM],
        complexity_mode: c_int,
        intra_period: c_uint,
        num_ref_frame: c_int,
        sps_pps_id_strategy: c_int,
        prefix_nal_adding_ctrl: bool,
        enable_ssei: bool,
        simulcast_avc: bool,
        padding_flag: c_int,
        entropy_coding_mode_flag: c_int,
        enable_frame_skip: bool,
        max_bitrate: c_int,
        max_qp: c_int,
        min_qp: c_int,
        max_nal_size: c_uint,
        enable_long_term_reference: bool,
        ltr_ref_num: c_int,
        ltr_mark_period: c_uint,
        multiple_thread_idc: u16,
        use_load_balancing: bool,
        loop_filter_disable_idc: c_int,
        loop_filter_alpha_c0_offset: c_int,
        loop_filter_beta_offset: c_int,
        enable_denoise: bool,
        enable_background_detection: bool,
        enable_adaptive_quant: bool,
        enable_frame_cropping_flag: bool,
        enable_scene_change_detect: bool,
        is_lossless_link: bool,
        fix_rc_overshoot: bool,
        idr_bitrate_ratio: c_int,
        psnr_y: bool,
        psnr_u: bool,
        psnr_v: bool,
    }

    /// The parameters with room behind them: should the library's
    /// structure be larger than ours, `GetDefaultParams` writes into the
    /// guard instead of past the allocation.
    #[repr(C)]
    struct Params {
        p: EncParamExt,
        _guard: [u8; 1024],
    }

    #[repr(C)]
    struct SourcePicture {
        color_format: c_int,
        stride: [c_int; 4],
        data: [*mut u8; 4],
        pic_width: c_int,
        pic_height: c_int,
        timestamp: i64,
        psnr_y: bool,
        psnr_u: bool,
        psnr_v: bool,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LayerBsInfo {
        temporal_id: u8,
        spatial_id: u8,
        quality_id: u8,
        frame_type: c_int,
        layer_type: u8,
        sub_seq_id: c_int,
        nal_count: c_int,
        nal_length: *mut c_int,
        bs_buf: *mut u8,
        psnr: [f32; 3],
    }

    #[repr(C)]
    struct FrameBsInfo {
        layer_num: c_int,
        layers: [LayerBsInfo; MAX_LAYER_NUM_OF_FRAME],
        frame_type: c_int,
        frame_size: c_int,
        timestamp: i64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Version {
        major: c_uint,
        minor: c_uint,
        revision: c_uint,
        reserved: c_uint,
    }

    type Enc = *mut *const Vtbl;

    /// `ISVCEncoder`'s methods, in order (the C++ class's virtual table).
    #[repr(C)]
    struct Vtbl {
        initialize: unsafe extern "C" fn(Enc, *const c_void) -> c_int,
        initialize_ext: unsafe extern "C" fn(Enc, *const EncParamExt) -> c_int,
        get_default_params: unsafe extern "C" fn(Enc, *mut EncParamExt) -> c_int,
        uninitialize: unsafe extern "C" fn(Enc) -> c_int,
        encode_frame: unsafe extern "C" fn(Enc, *const SourcePicture, *mut FrameBsInfo) -> c_int,
        encode_parameter_sets: unsafe extern "C" fn(Enc, *mut FrameBsInfo) -> c_int,
        force_intra_frame: unsafe extern "C" fn(Enc, bool) -> c_int,
        set_option: unsafe extern "C" fn(Enc, c_int, *mut c_void) -> c_int,
        get_option: unsafe extern "C" fn(Enc, c_int, *mut c_void) -> c_int,
    }

    const CAMERA_VIDEO_REAL_TIME: c_int = 0;
    const RC_BITRATE_MODE: c_int = 1;
    const LOW_COMPLEXITY: c_int = 0;
    const CONSTANT_ID: c_int = 0;
    const ENCODER_OPTION_DATAFORMAT: c_int = 0;
    const VIDEO_FORMAT_I420: c_int = 23;
    const VF_UNDEF: u8 = 5;
    const BT709: u8 = 1;
    const FRAME_TYPE_INVALID: c_int = 0;
    const FRAME_TYPE_SKIP: c_int = 4;

    type Create = unsafe extern "C" fn(*mut Enc) -> c_int;
    type Destroy = unsafe extern "C" fn(Enc);
    type GetVersion = unsafe extern "C" fn(*mut Version);

    /// The loaded library.
    pub struct OpenH264 {
        create: Create,
        destroy: Destroy,
        _lib: Lib,
    }

    impl OpenH264 {
        /// Load the downloaded library: by its absolute path, after
        /// checking it is the pinned file, and only if it reports the
        /// pinned version.
        pub fn load() -> Result<Arc<OpenH264>> {
            let pin = openh264_pins::this_machine().ok_or_else(|| anyhow!("OpenH264 is not available for {}", std::env::consts::ARCH))?;
            let path = lib_path(&dir().ok_or_else(|| anyhow!("no home folder"))?, pin);
            if !path.is_absolute() {
                bail!("OpenH264's path is not absolute");
            }
            verify_lib(&path, pin).map_err(|e| anyhow!(e))?;
            let lib = Lib::open(path.to_str().ok_or_else(|| anyhow!("OpenH264's path is not UTF-8"))?)?;
            // SAFETY: the prototypes of codec_api.h.
            unsafe {
                let version: GetVersion = lib.sym("WelsGetCodecVersionEx")?;
                let mut v = Version::default();
                version(&mut v);
                if (v.major, v.minor, v.revision) != VERSION {
                    bail!("OpenH264 {}.{}.{} is not the expected {}.{}.{}", v.major, v.minor, v.revision, VERSION.0, VERSION.1, VERSION.2);
                }
                Ok(Arc::new(OpenH264 { create: lib.sym("WelsCreateSVCEncoder")?, destroy: lib.sym("WelsDestroySVCEncoder")?, _lib: lib }))
            }
        }
    }

    /// One encoder (see the module docs).
    pub struct H264Enc {
        lib: Arc<OpenH264>,
        enc: Enc,
        w: u32,
        h: u32,
        info: Box<FrameBsInfo>,
    }

    // SAFETY: the encoder is used by its one owner only.
    unsafe impl Send for H264Enc {}

    impl H264Enc {
        /// A `w`×`h` (even) encoder at `fps` and `bitrate` bit/s.
        pub fn new(lib: Arc<OpenH264>, w: u32, h: u32, fps: u32, bitrate: u32) -> Result<H264Enc> {
            let mut enc: Enc = std::ptr::null_mut();
            // SAFETY: `enc` receives the new encoder.
            if unsafe { (lib.create)(&mut enc) } != 0 || enc.is_null() {
                bail!("WelsCreateSVCEncoder failed");
            }
            // SAFETY: an all-zero FrameBsInfo is valid (null pointers, zero counts).
            let info: Box<FrameBsInfo> = unsafe { Box::new(std::mem::zeroed()) };
            let mut e = H264Enc { lib, enc, w, h, info };
            e.init(fps, bitrate)?;
            Ok(e)
        }

        fn vt(&self) -> &Vtbl {
            // SAFETY: a live encoder object starts with its vtable pointer.
            unsafe { &**self.enc }
        }

        fn init(&mut self, fps: u32, bitrate: u32) -> Result<()> {
            // SAFETY: zeroed parameters are filled by GetDefaultParams; the
            // guard behind them catches a larger library structure.
            let mut params: Box<Params> = unsafe { Box::new(std::mem::zeroed()) };
            let vt = self.vt();
            let r = unsafe { (vt.get_default_params)(self.enc, &mut params.p) };
            if r != 0 {
                bail!("OpenH264 GetDefaultParams failed ({r})");
            }
            let p = &mut params.p;
            let (w, h) = (self.w as c_int, self.h as c_int);
            p.usage_type = CAMERA_VIDEO_REAL_TIME;
            p.pic_width = w;
            p.pic_height = h;
            p.target_bitrate = bitrate as c_int;
            p.rc_mode = RC_BITRATE_MODE;
            p.max_frame_rate = fps as f32;
            p.temporal_layer_num = 1;
            p.spatial_layer_num = 1;
            p.complexity_mode = LOW_COMPLEXITY;
            p.intra_period = intra_period(fps);
            p.sps_pps_id_strategy = CONSTANT_ID;
            p.prefix_nal_adding_ctrl = false;
            p.enable_ssei = false;
            p.enable_frame_skip = false;
            p.multiple_thread_idc = 1;
            let l = &mut p.spatial_layers[0];
            l.video_width = w;
            l.video_height = h;
            l.frame_rate = fps as f32;
            l.spatial_bitrate = bitrate as c_int;
            l.video_signal_type_present = true;
            l.video_format = VF_UNDEF;
            l.full_range = false;
            l.color_description_present = true;
            l.color_primaries = BT709;
            l.transfer_characteristics = BT709;
            l.color_matrix = BT709;
            // SAFETY: valid parameters for this encoder.
            let r = unsafe { (vt.initialize_ext)(self.enc, &params.p) };
            if r != 0 {
                bail!("OpenH264 could not start for {}x{} at {fps} fps ({r})", self.w, self.h);
            }
            let mut fmt: c_int = VIDEO_FORMAT_I420;
            // SAFETY: the data format option takes an int.
            unsafe { (vt.set_option)(self.enc, ENCODER_OPTION_DATAFORMAT, &mut fmt as *mut c_int as *mut c_void) };
            Ok(())
        }

        /// Encode one I420 picture (Y, U, V planes one after the other)
        /// stamped `ms`; its Annex-B bytes, or `None` when the encoder
        /// dropped it.
        pub fn encode(&mut self, i420: &[u8], ms: i64) -> Result<Option<Vec<u8>>> {
            let (w, h) = (self.w as usize, self.h as usize);
            assert!(i420.len() >= w * h * 3 / 2, "short I420 picture");
            let y = i420.as_ptr() as *mut u8;
            // SAFETY: offsets inside `i420` (checked above).
            let (u, v) = unsafe { (y.add(w * h), y.add(w * h + w * h / 4)) };
            let pic = SourcePicture {
                color_format: VIDEO_FORMAT_I420,
                stride: [w as c_int, (w / 2) as c_int, (w / 2) as c_int, 0],
                data: [y, u, v, std::ptr::null_mut()],
                pic_width: w as c_int,
                pic_height: h as c_int,
                timestamp: ms,
                psnr_y: false,
                psnr_u: false,
                psnr_v: false,
            };
            let vt = self.vt();
            // SAFETY: the picture's planes stay alive for the call (the
            // encoder only reads them); `info` is ours.
            let r = unsafe { (vt.encode_frame)(self.enc, &pic, &mut *self.info) };
            if r != 0 {
                bail!("OpenH264 could not encode a frame ({r})");
            }
            let info = &*self.info;
            if info.frame_type == FRAME_TYPE_SKIP || info.frame_type == FRAME_TYPE_INVALID {
                return Ok(None);
            }
            let mut out = Vec::with_capacity(info.frame_size.max(0) as usize);
            for l in &info.layers[..(info.layer_num.clamp(0, MAX_LAYER_NUM_OF_FRAME as c_int) as usize)] {
                if l.bs_buf.is_null() || l.nal_length.is_null() || l.nal_count <= 0 {
                    continue;
                }
                // SAFETY: the encoder fills `nal_count` lengths and a buffer
                // of their sum for each layer, valid until the next call.
                let lens = unsafe { std::slice::from_raw_parts(l.nal_length, l.nal_count as usize) };
                let n: usize = lens.iter().map(|&x| x.max(0) as usize).sum();
                out.extend_from_slice(unsafe { std::slice::from_raw_parts(l.bs_buf, n) });
            }
            Ok((!out.is_empty()).then_some(out))
        }

        pub fn size(&self) -> (u32, u32) {
            (self.w, self.h)
        }
    }

    impl Drop for H264Enc {
        fn drop(&mut self) {
            // SAFETY: created by WelsCreateSVCEncoder, destroyed once.
            unsafe {
                (self.vt().uninitialize)(self.enc);
                (self.lib.destroy)(self.enc);
            }
        }
    }

    #[cfg(test)]
    mod layout {
        use super::*;

        /// The structures' sizes on x86_64 / aarch64 Linux, as the C
        /// compiler lays out OpenH264 2.6.0's headers.
        #[test]
        fn structures_have_the_c_sizes() {
            assert_eq!(std::mem::size_of::<SliceArgument>(), 152);
            assert_eq!(std::mem::size_of::<SpatialLayerConfig>(), 200);
            assert_eq!(std::mem::size_of::<SourcePicture>(), 80);
            assert_eq!(std::mem::size_of::<LayerBsInfo>(), 56);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rec::openh264_pins::PINS;

    #[test]
    fn the_library_lives_in_the_xdg_data_folder() {
        // Absolute paths on the machine running the test.
        let (data, home) = if cfg!(windows) { (r"C:\data", r"C:\home\u") } else { ("/data", "/home/u") };
        let d = dir_from(Some(data.into()), Some(home.into())).unwrap();
        assert_eq!(d, Path::new(data).join("rustshot").join("openh264"));
        let d = dir_from(Some("relative".into()), Some(home.into())).unwrap();
        assert_eq!(d, Path::new(home).join(".local/share").join("rustshot").join("openh264"));
        assert!(dir_from(None, None).is_none());
        assert!(dir_from(None, Some("rel".into())).is_none(), "a relative home is not used");
        assert!(lib_path(Path::new("/x"), &PINS[0]).ends_with("libopenh264-2.6.0-linux64.8.so"));
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustshot-oh264-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A pin for a local fixture (`data`), its "library" being `so`.
    fn fixture_pin(data: &[u8], so: &[u8]) -> Pin {
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        Pin {
            arch: "test",
            file: "fixture.so.bz2",
            bz2_sha256: leak(crate::sha256::hex(&crate::sha256::digest(data))),
            bz2_size: data.len() as u64,
            so: "fixture.so",
            so_sha256: leak(crate::sha256::hex(&crate::sha256::digest(so))),
            so_size: so.len() as u64,
        }
    }

    #[test]
    fn a_download_with_the_wrong_hash_is_refused_before_unpacking() {
        let d = scratch("wrong");
        let bz2 = d.join("fixture.so.bz2.download");
        std::fs::write(&bz2, b"not what was pinned").unwrap();
        let pin = fixture_pin(b"what was pinned....", b"lib");
        let dest = d.join("fixture.so");
        let unpacked = std::cell::Cell::new(false);
        let r = verify_and_unpack(&bz2, &pin, &dest, |_, _, _| {
            unpacked.set(true);
            Ok(())
        });
        assert!(r.unwrap_err().contains("SHA-256"));
        assert!(!unpacked.get(), "never unpacked");
        assert!(!bz2.exists() && !dest.exists(), "the download is deleted");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_right_hash_is_unpacked_and_the_result_checked() {
        let d = scratch("right");
        let bz2 = d.join("fixture.so.bz2.download");
        std::fs::write(&bz2, b"pinned bytes").unwrap();
        let pin = fixture_pin(b"pinned bytes", b"the library");
        let dest = d.join("fixture.so");
        verify_and_unpack(&bz2, &pin, &dest, |_, out, max| {
            assert_eq!(max, 11);
            std::fs::write(out, b"the library").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"the library");
        assert!(!bz2.exists());
        verify_lib(&dest, &pin).unwrap();
        // A tampered library is refused, at unpacking and at loading.
        std::fs::write(&bz2, b"pinned bytes").unwrap();
        let dest2 = d.join("other.so");
        let r = verify_and_unpack(&bz2, &pin, &dest2, |_, out, _| std::fs::write(out, b"the librarX").map_err(|e| e.to_string()));
        assert!(r.unwrap_err().contains("after unpacking"));
        assert!(!dest2.exists());
        std::fs::write(&dest, b"the librarX").unwrap();
        assert!(verify_lib(&dest, &pin).is_err());
        // The unpacker's error (no bzip2) comes through.
        std::fs::write(&bz2, b"pinned bytes").unwrap();
        let r = verify_and_unpack(&bz2, &pin, &dest2, |_, _, _| Err(NO_BZIP2.into()));
        assert_eq!(r.unwrap_err(), NO_BZIP2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_idr_every_two_seconds() {
        assert_eq!(intra_period(30), 60);
        assert_eq!(intra_period(60), 120);
        assert_eq!(intra_period(0), 2);
    }
}
