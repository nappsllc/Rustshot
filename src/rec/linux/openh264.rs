//! Cisco's OpenH264, for MP4 on Linux without ffmpeg: downloaded on the
//! user's request ([`download`]) from Cisco (the only URLs allowed are the
//! ones pinned in [`crate::rec::openh264_pins`]), verified, unpacked and
//! kept in `$XDG_DATA_HOME/rustshot/openh264/` (mode 0700), then loaded
//! with `dlopen` by that absolute path only ([`OpenH264::load`]).
//!
//! Download and verification: the `.bz2` is fetched with curl (plain HTTP,
//! no redirects, cut off past the pinned size) into the private folder;
//! its SHA-256 must be the pinned one *before* it is unpacked. It is
//! unpacked by the system's `/usr/bin/bzip2 -dc` (a fixed path, or the
//! one inside the snap, `$SNAP/usr/bin/bzip2`: [`bzip2_in`]; run
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
pub const NO_BZIP2: crate::i18n::Key = key!("codec.no_bzip2");

/// The bzip2 to run: [`BZIP2`], else (in a snap, whose base may lack it)
/// the one the snap ships, `$SNAP/usr/bin/bzip2` (`snap`: `$SNAP`, an
/// absolute, read-only install folder). `is_file` says what exists.
pub fn bzip2_in(snap: Option<OsString>, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let sys = PathBuf::from(BZIP2);
    if is_file(&sys) {
        return Some(sys);
    }
    let snap = PathBuf::from(snap?);
    let p = snap.join("usr/bin/bzip2");
    (snap.is_absolute() && is_file(&p)).then_some(p)
}

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

/// Whether the downloaded library is there and is the pinned file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    Missing,
    Good,
    /// It was there but failed its check (why); it has been deleted.
    Bad(String),
}

/// The library of `pin` in `dir`, checked by size and SHA-256 (see
/// [`verify_lib`]); one that fails is deleted, so the dialog's Download
/// fetches it again.
pub fn check_in(dir: &Path, pin: &Pin) -> Installed {
    let p = lib_path(dir, pin);
    if !p.exists() {
        return Installed::Missing;
    }
    match verify_lib(&p, pin) {
        Ok(()) => Installed::Good,
        Err(e) => {
            let _ = std::fs::remove_file(&p);
            Installed::Bad(e)
        }
    }
}

/// [`check_in`] for this machine and user.
pub fn check() -> Installed {
    match (crate::rec::openh264_pins::this_machine(), dir()) {
        (Some(pin), Some(d)) => check_in(&d, pin),
        _ => Installed::Missing,
    }
}

/// The downloaded library for this machine, if it is there (by its size;
/// the hash is checked when it is loaded).
pub fn installed() -> Option<PathBuf> {
    let pin = crate::rec::openh264_pins::this_machine()?;
    let p = lib_path(&dir()?, pin);
    std::fs::metadata(&p).ok().filter(|m| m.is_file() && m.len() == pin.so_size).map(|_| p)
}

/// The name of attempt `attempt`'s download in the library's folder: its
/// own per process and attempt, so that a cancelled attempt still winding
/// down never deletes or unpacks over the file of the next one.
pub fn download_name(pin: &Pin, pid: u32, attempt: u64) -> String {
    format!("{}.{pid}-{attempt}.download", pin.file)
}

/// Whether `name` is a download left behind ([`download_name`], or its
/// `.part` while unpacking) by a process that is gone: not `me`, and
/// `alive(pid)` says no.
pub fn stale_download(name: &str, me: u32, alive: impl Fn(u32) -> bool) -> bool {
    let Some(stem) = name.strip_suffix(".download").or_else(|| name.strip_suffix(".download.part")) else { return false };
    let Some((_, tag)) = stem.rsplit_once('.') else { return false };
    let Some(pid) = tag.split_once('-').and_then(|(p, a)| (!a.is_empty() && a.bytes().all(|b| b.is_ascii_digit())).then_some(p)).and_then(|p| p.parse::<u32>().ok()) else {
        return false;
    };
    pid != me && !alive(pid)
}

/// Delete the downloads in `dir` that [`stale_download`] says were left
/// behind (an attempt killed mid-way, a crash); the files removed.
pub fn sweep_in(dir: &Path, me: u32, alive: impl Fn(u32) -> bool) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if e.file_type().is_ok_and(|t| t.is_file()) && stale_download(name, me, &alive) && std::fs::remove_file(e.path()).is_ok() {
            out.push(e.path());
        }
    }
    out
}

/// [`sweep_in`] for this user (the daemon, at start): a process is alive
/// while `/proc/<pid>` exists.
pub fn sweep() {
    if let Some(d) = dir() {
        for p in sweep_in(&d, std::process::id(), |pid| Path::new(&format!("/proc/{pid}")).exists()) {
            eprintln!("rustshot: removed an unfinished OpenH264 download: {}", p.display());
        }
    }
}

fn hex_digest(p: &Path) -> Result<String, String> {
    crate::sha256::file_digest(p).map(|d| crate::sha256::hex(&d)).map_err(|e| format!("read {}: {e}", p.display()))
}

/// Check the downloaded `bz2` against `pin`, unpack it with `unpack(src,
/// dst)` into `dest` (through `<bz2>.part`) and check the result. The
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
        // Unpacked next to the download (its own name per attempt), then
        // renamed into place.
        let mut part = bz2.as_os_str().to_os_string();
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
        let Some(bzip2) = bzip2_in(std::env::var_os("SNAP"), Path::is_file) else {
            return Err(NO_BZIP2.into());
        };
        let input = std::fs::File::open(src).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&bzip2);
        cmd.arg("-dc").stdin(input).stdout(Stdio::piped()).stderr(Stdio::null());
        child::prepare(&mut cmd, None);
        let mut ch = cmd.spawn().map_err(|e| format!("run {}: {e}", bzip2.display()))?;
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
            return Err(format!("{} could not unpack OpenH264 ({st})", bzip2.display()));
        }
        out.sync_all().map_err(|e| e.to_string())
    }

    /// Download, verify and unpack the pinned OpenH264 for this machine;
    /// `progress(got, total)` returning false cancels. The library's path.
    ///
    /// `attempt` names this attempt's download file ([`download_name`]).
    pub fn download(attempt: u64, progress: &dyn Fn(u64, Option<u64>) -> bool) -> Result<PathBuf, String> {
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
        let bz2 = dir.join(download_name(pin, std::process::id(), attempt));
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
            let r = unsafe { (vt.set_option)(self.enc, ENCODER_OPTION_DATAFORMAT, &mut fmt as *mut c_int as *mut c_void) };
            if r != 0 {
                bail!("OpenH264 does not take I420 pictures (SetOption DATAFORMAT failed: {r})");
            }
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
        use std::mem::{offset_of, size_of};

        // Every size and field offset below is worked out by hand from
        // OpenH264 2.6.0's codec/api/wels/codec_app_def.h
        // (https://github.com/cisco/openh264/blob/v2.6.0/codec/api/wels/codec_app_def.h)
        // for LP64 Linux (x86_64, aarch64): int, float and enums 4 bytes,
        // bool and unsigned char 1, unsigned short 2, long long and
        // pointers 8, each field at its C alignment. Where they are in
        // the v2.6.0 file (line numbers approximate): MAX_SPATIAL_LAYER_NUM
        // 4 (l. 43), MAX_LAYER_NUM_OF_FRAME 128 (l. 46), MAX_SLICES_NUM_TMP
        // 35 (l. 54); SSliceArgument l. 345-352, SSpatialLayerConfig
        // l. 452-485, SEncParamExt l. 537-595, SLayerBSInfo l. 623-639,
        // SFrameBSInfo l. 644-651, SSourcePicture l. 656-666;
        // OpenH264Version (4 unsigned ints) and the 9 ISVCEncoder methods
        // are in codec_api.h / codec_ver.h. A field added or moved in the
        // middle of a structure shifts the offsets after it and fails
        // here; the version check at load guards the library's side.

        fn check(name: &str, got: &[(usize, usize)]) {
            for (k, (got, want)) in got.iter().enumerate() {
                assert_eq!(got, want, "{name}: field {k}");
            }
        }

        #[test]
        fn slice_argument() {
            type S = SliceArgument;
            assert_eq!(size_of::<S>(), 152);
            check(
                "SSliceArgument",
                &[
                    (offset_of!(S, mode), 0),             // uiSliceMode
                    (offset_of!(S, num), 4),              // uiSliceNum
                    (offset_of!(S, mb_num), 8),           // uiSliceMbNum[35]
                    (offset_of!(S, size_constraint), 148), // uiSliceSizeConstraint
                ],
            );
        }

        #[test]
        fn spatial_layer_config() {
            type S = SpatialLayerConfig;
            assert_eq!(size_of::<S>(), 200);
            check(
                "SSpatialLayerConfig",
                &[
                    (offset_of!(S, video_width), 0),
                    (offset_of!(S, video_height), 4),
                    (offset_of!(S, frame_rate), 8),
                    (offset_of!(S, spatial_bitrate), 12),
                    (offset_of!(S, max_spatial_bitrate), 16),
                    (offset_of!(S, profile_idc), 20),
                    (offset_of!(S, level_idc), 24),
                    (offset_of!(S, dlayer_qp), 28),
                    (offset_of!(S, slice), 32),
                    (offset_of!(S, video_signal_type_present), 184),
                    (offset_of!(S, video_format), 185),
                    (offset_of!(S, full_range), 186),
                    (offset_of!(S, color_description_present), 187),
                    (offset_of!(S, color_primaries), 188),
                    (offset_of!(S, transfer_characteristics), 189),
                    (offset_of!(S, color_matrix), 190),
                    (offset_of!(S, aspect_ratio_present), 191),
                    (offset_of!(S, aspect_ratio), 192),
                    (offset_of!(S, aspect_ratio_ext_width), 196),
                    (offset_of!(S, aspect_ratio_ext_height), 198),
                ],
            );
        }

        #[test]
        fn enc_param_ext() {
            type S = EncParamExt;
            assert_eq!(size_of::<S>(), 924);
            check(
                "SEncParamExt",
                &[
                    (offset_of!(S, usage_type), 0),
                    (offset_of!(S, pic_width), 4),
                    (offset_of!(S, pic_height), 8),
                    (offset_of!(S, target_bitrate), 12),
                    (offset_of!(S, rc_mode), 16),
                    (offset_of!(S, max_frame_rate), 20),
                    (offset_of!(S, temporal_layer_num), 24),
                    (offset_of!(S, spatial_layer_num), 28),
                    (offset_of!(S, spatial_layers), 32), // 4 x 200
                    (offset_of!(S, complexity_mode), 832),
                    (offset_of!(S, intra_period), 836),
                    (offset_of!(S, num_ref_frame), 840),
                    (offset_of!(S, sps_pps_id_strategy), 844),
                    (offset_of!(S, prefix_nal_adding_ctrl), 848),
                    (offset_of!(S, enable_ssei), 849),
                    (offset_of!(S, simulcast_avc), 850),
                    (offset_of!(S, padding_flag), 852),
                    (offset_of!(S, entropy_coding_mode_flag), 856),
                    (offset_of!(S, enable_frame_skip), 860),
                    (offset_of!(S, max_bitrate), 864),
                    (offset_of!(S, max_qp), 868),
                    (offset_of!(S, min_qp), 872),
                    (offset_of!(S, max_nal_size), 876),
                    (offset_of!(S, enable_long_term_reference), 880),
                    (offset_of!(S, ltr_ref_num), 884),
                    (offset_of!(S, ltr_mark_period), 888),
                    (offset_of!(S, multiple_thread_idc), 892),
                    (offset_of!(S, use_load_balancing), 894),
                    (offset_of!(S, loop_filter_disable_idc), 896),
                    (offset_of!(S, loop_filter_alpha_c0_offset), 900),
                    (offset_of!(S, loop_filter_beta_offset), 904),
                    (offset_of!(S, enable_denoise), 908),
                    (offset_of!(S, enable_background_detection), 909),
                    (offset_of!(S, enable_adaptive_quant), 910),
                    (offset_of!(S, enable_frame_cropping_flag), 911),
                    (offset_of!(S, enable_scene_change_detect), 912),
                    (offset_of!(S, is_lossless_link), 913),
                    (offset_of!(S, fix_rc_overshoot), 914),
                    (offset_of!(S, idr_bitrate_ratio), 916),
                    (offset_of!(S, psnr_y), 920),
                    (offset_of!(S, psnr_u), 921),
                    (offset_of!(S, psnr_v), 922),
                ],
            );
        }

        #[test]
        fn source_picture() {
            type S = SourcePicture;
            assert_eq!(size_of::<S>(), 80);
            check(
                "SSourcePicture",
                &[
                    (offset_of!(S, color_format), 0),
                    (offset_of!(S, stride), 4),
                    (offset_of!(S, data), 24),
                    (offset_of!(S, pic_width), 56),
                    (offset_of!(S, pic_height), 60),
                    (offset_of!(S, timestamp), 64),
                    (offset_of!(S, psnr_y), 72),
                    (offset_of!(S, psnr_u), 73),
                    (offset_of!(S, psnr_v), 74),
                ],
            );
        }

        #[test]
        fn layer_and_frame_bitstream_info() {
            type L = LayerBsInfo;
            assert_eq!(size_of::<L>(), 56);
            check(
                "SLayerBSInfo",
                &[
                    (offset_of!(L, temporal_id), 0),
                    (offset_of!(L, spatial_id), 1),
                    (offset_of!(L, quality_id), 2),
                    (offset_of!(L, frame_type), 4),
                    (offset_of!(L, layer_type), 8),
                    (offset_of!(L, sub_seq_id), 12),
                    (offset_of!(L, nal_count), 16),
                    (offset_of!(L, nal_length), 24),
                    (offset_of!(L, bs_buf), 32),
                    (offset_of!(L, psnr), 40),
                ],
            );
            type F = FrameBsInfo;
            assert_eq!(size_of::<F>(), 7192);
            check(
                "SFrameBSInfo",
                &[
                    (offset_of!(F, layer_num), 0),
                    (offset_of!(F, layers), 8), // 128 x 56
                    (offset_of!(F, frame_type), 7176),
                    (offset_of!(F, frame_size), 7180),
                    (offset_of!(F, timestamp), 7184),
                ],
            );
        }

        #[test]
        fn version_and_vtable() {
            assert_eq!(size_of::<Version>(), 16);
            check("OpenH264Version", &[(offset_of!(Version, minor), 4), (offset_of!(Version, revision), 8), (offset_of!(Version, reserved), 12)]);
            assert_eq!(size_of::<Vtbl>(), 9 * 8);
            check(
                "ISVCEncoderVtbl",
                &[
                    (offset_of!(Vtbl, initialize), 0),
                    (offset_of!(Vtbl, initialize_ext), 8),
                    (offset_of!(Vtbl, get_default_params), 16),
                    (offset_of!(Vtbl, uninitialize), 24),
                    (offset_of!(Vtbl, encode_frame), 32),
                    (offset_of!(Vtbl, encode_parameter_sets), 40),
                    (offset_of!(Vtbl, force_intra_frame), 48),
                    (offset_of!(Vtbl, set_option), 56),
                    (offset_of!(Vtbl, get_option), 64),
                ],
            );
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
    fn bzip2_is_the_systems_else_the_snaps() {
        let root = if cfg!(windows) { "C:/snap/rustshot/7" } else { "/snap/rustshot/7" };
        let sys = |q: &Path| q == Path::new(BZIP2);
        assert_eq!(bzip2_in(None, sys), Some(PathBuf::from(BZIP2)));
        assert_eq!(bzip2_in(Some(root.into()), sys), Some(PathBuf::from(BZIP2)));
        let snapped = Path::new(root).join("usr/bin/bzip2");
        let has = |q: &Path| q == snapped;
        assert_eq!(bzip2_in(Some(root.into()), has), Some(snapped.clone()));
        assert_eq!(bzip2_in(None, has), None);
        // A relative $SNAP is not trusted.
        assert_eq!(bzip2_in(Some("rustshot".into()), |q: &Path| q.ends_with("usr/bin/bzip2") && q != Path::new(BZIP2)), None);
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
    fn a_bad_library_is_deleted_and_reported() {
        let d = scratch("check");
        let pin = fixture_pin(b"unused", b"the library");
        assert_eq!(check_in(&d, &pin), Installed::Missing);
        std::fs::write(d.join("fixture.so"), b"the library").unwrap();
        assert_eq!(check_in(&d, &pin), Installed::Good);
        assert!(d.join("fixture.so").exists());
        std::fs::write(d.join("fixture.so"), b"the librarX").unwrap();
        assert!(matches!(check_in(&d, &pin), Installed::Bad(e) if e.contains("SHA-256")));
        assert!(!d.join("fixture.so").exists(), "deleted");
        assert_eq!(check_in(&d, &pin), Installed::Missing, "so Download fetches it again");
        std::fs::write(d.join("fixture.so"), b"short").unwrap();
        assert!(matches!(check_in(&d, &pin), Installed::Bad(_)), "wrong size");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_attempt_downloads_to_its_own_file() {
        let a = download_name(&PINS[0], 42, 1);
        assert_eq!(a, "libopenh264-2.6.0-linux64.8.so.bz2.42-1.download");
        assert_ne!(a, download_name(&PINS[0], 42, 2));
        assert_ne!(a, download_name(&PINS[0], 43, 1));
        // The unpacked part is named after the download, not the library.
        let d = scratch("attempts");
        let pin = fixture_pin(b"pinned bytes", b"the library");
        let dest = d.join("fixture.so");
        let bz2 = d.join(download_name(&pin, 1, 7));
        std::fs::write(&bz2, b"pinned bytes").unwrap();
        let other = d.join(format!("{}.part", download_name(&pin, 1, 8)));
        std::fs::write(&other, b"attempt 8's").unwrap();
        verify_and_unpack(&bz2, &pin, &dest, |_, out, _| {
            assert_eq!(out.file_name().unwrap().to_str().unwrap(), format!("{}.part", download_name(&pin, 1, 7)));
            std::fs::write(out, b"the library").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"the library");
        assert_eq!(std::fs::read(&other).unwrap(), b"attempt 8's", "another attempt's file is left alone");
        let _ = std::fs::remove_dir_all(&d);
    }


    /// Downloads of a process that is gone are swept at start; this
    /// process's, a live one's and other files stay.
    #[test]
    fn stale_downloads_are_swept() {
        let alive = |pid: u32| pid == 77;
        assert!(stale_download("libopenh264-2.6.0-linux64.8.so.bz2.42-1.download", 9, alive));
        assert!(stale_download("libopenh264-2.6.0-linux64.8.so.bz2.42-3.download.part", 9, alive));
        assert!(!stale_download("libopenh264-2.6.0-linux64.8.so.bz2.9-1.download", 9, alive), "ours");
        assert!(!stale_download("libopenh264-2.6.0-linux64.8.so.bz2.77-1.download", 9, alive), "a live process's");
        for keep in ["libopenh264-2.6.0-linux64.8.so", "x.download", "a.b.download", "a.x-1.download", "a.1-.download", "notes.txt"] {
            assert!(!stale_download(keep, 9, alive), "{keep}");
        }
        let d = std::env::temp_dir().join(format!("rustshot-oh264-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let names = ["lib.so.bz2.42-1.download", "lib.so.bz2.42-2.download.part", "lib.so.bz2.77-1.download", "lib.so"];
        for n in names {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        let mut gone: Vec<String> = sweep_in(&d, 9, alive).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        gone.sort();
        assert_eq!(gone, ["lib.so.bz2.42-1.download", "lib.so.bz2.42-2.download.part"]);
        assert!(d.join("lib.so").exists() && d.join("lib.so.bz2.77-1.download").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_idr_every_two_seconds() {
        assert_eq!(intra_period(30), 60);
        assert_eq!(intra_period(60), 120);
        assert_eq!(intra_period(0), 2);
    }
}
