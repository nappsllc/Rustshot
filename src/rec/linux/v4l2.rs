//! V4L2 cameras (X11 sessions): `/dev/videoN` capture devices, YUYV (or
//! NV12) at 1280×720 (640×480 when the camera cannot do 720p at 15 fps or
//! more), memory-mapped streaming (`REQBUFS` / `QBUF` / `STREAMON` /
//! `DQBUF`), pictures converted to BGRA by `cam`'s converters. A camera
//! that offers only MJPG gets the same message as on Windows.
//!
//! The ioctl numbers are computed as `<asm-generic/ioctl.h>` does (x86_64
//! and aarch64 share it) and the structs are the 64-bit layouts; both are
//! unit-tested against the kernel's values.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // Linux; the pure parts are tested everywhere

use crate::cam::{MJPG_ONLY, NO_FORMAT};

const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

/// `_IOC(dir, type, nr, size)`.
pub const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr as u32
}

/// `struct v4l2_capability`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Capability {
    pub driver: [u8; 16],
    pub card: [u8; 32],
    pub bus_info: [u8; 32],
    pub version: u32,
    pub capabilities: u32,
    pub device_caps: u32,
    pub reserved: [u32; 3],
}

/// `struct v4l2_fmtdesc`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FmtDesc {
    pub index: u32,
    pub ty: u32,
    pub flags: u32,
    pub description: [u8; 32],
    pub pixelformat: u32,
    pub mbus_code: u32,
    pub reserved: [u32; 3],
}

/// `struct v4l2_pix_format`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PixFormat {
    pub width: u32,
    pub height: u32,
    pub pixelformat: u32,
    pub field: u32,
    pub bytesperline: u32,
    pub sizeimage: u32,
    pub colorspace: u32,
    pub priv_: u32,
    pub flags: u32,
    pub ycbcr_enc: u32,
    pub quantization: u32,
    pub xfer_func: u32,
}

/// `struct v4l2_format` (the union is 200 bytes, 8-aligned: it holds
/// pointers in `v4l2_window`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Format {
    pub ty: u32,
    pub _pad: u32,
    pub fmt: [u64; 25],
}

impl Format {
    pub fn pix(&self) -> PixFormat {
        // SAFETY: PixFormat (48 bytes of u32) fits at the start of the union.
        unsafe { (self.fmt.as_ptr() as *const PixFormat).read_unaligned() }
    }

    pub fn set_pix(&mut self, p: PixFormat) {
        // SAFETY: as above.
        unsafe { (self.fmt.as_mut_ptr() as *mut PixFormat).write_unaligned(p) }
    }
}

/// `struct v4l2_requestbuffers`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RequestBuffers {
    pub count: u32,
    pub ty: u32,
    pub memory: u32,
    pub capabilities: u32,
    pub flags: u8,
    pub reserved: [u8; 3],
}

/// `struct v4l2_buffer` (64-bit: `struct timeval` is two longs; the `m`
/// union is 8 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Buffer {
    pub index: u32,
    pub ty: u32,
    pub bytesused: u32,
    pub flags: u32,
    pub field: u32,
    pub _pad: u32,
    pub timestamp: [i64; 2],
    pub timecode: [u32; 4],
    pub sequence: u32,
    pub memory: u32,
    /// `m.offset` for MMAP buffers (the union's first 4 bytes).
    pub m: u64,
    pub length: u32,
    pub reserved2: u32,
    pub request_fd: u32,
    pub _pad2: u32,
}

/// `struct v4l2_streamparm` with the capture member.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StreamParm {
    pub ty: u32,
    pub capability: u32,
    pub capturemode: u32,
    pub numerator: u32,
    pub denominator: u32,
    pub extendedmode: u32,
    pub readbuffers: u32,
    pub reserved: [u32; 44],
}

pub const VIDIOC_QUERYCAP: u32 = ioc(IOC_READ, b'V', 0, size_of::<Capability>());
pub const VIDIOC_ENUM_FMT: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 2, size_of::<FmtDesc>());
pub const VIDIOC_G_FMT: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 4, size_of::<Format>());
pub const VIDIOC_S_FMT: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 5, size_of::<Format>());
pub const VIDIOC_REQBUFS: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 8, size_of::<RequestBuffers>());
pub const VIDIOC_QUERYBUF: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 9, size_of::<Buffer>());
pub const VIDIOC_QBUF: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 15, size_of::<Buffer>());
pub const VIDIOC_DQBUF: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 17, size_of::<Buffer>());
pub const VIDIOC_STREAMON: u32 = ioc(IOC_WRITE, b'V', 18, size_of::<i32>());
pub const VIDIOC_STREAMOFF: u32 = ioc(IOC_WRITE, b'V', 19, size_of::<i32>());
pub const VIDIOC_G_PARM: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 21, size_of::<StreamParm>());
pub const VIDIOC_S_PARM: u32 = ioc(IOC_READ | IOC_WRITE, b'V', 22, size_of::<StreamParm>());

pub const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
pub const MEMORY_MMAP: u32 = 1;
pub const CAP_VIDEO_CAPTURE: u32 = 0x1;
pub const CAP_STREAMING: u32 = 0x0400_0000;
pub const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
pub const FIELD_NONE: u32 = 1;
pub const YCBCR_ENC_709: u32 = 2;
pub const QUANTIZATION_FULL: u32 = 1;

pub const fn fourcc(s: &[u8; 4]) -> u32 {
    s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24
}

pub const YUYV: u32 = fourcc(b"YUYV");
pub const NV12: u32 = fourcc(b"NV12");
pub const MJPG: u32 = fourcc(b"MJPG");
pub const JPEG: u32 = fourcc(b"JPEG");

/// The picture format to ask for among what the camera lists: YUYV, else
/// NV12; a camera with only (M)JPEG gets [`MJPG_ONLY`].
pub fn pick_format(listed: &[u32]) -> Result<u32, &'static str> {
    if listed.contains(&YUYV) {
        Ok(YUYV)
    } else if listed.contains(&NV12) {
        Ok(NV12)
    } else if listed.iter().any(|f| *f == MJPG || *f == JPEG) {
        Err(MJPG_ONLY.get())
    } else {
        Err(NO_FORMAT.get())
    }
}

/// Whether a capture device (not a metadata node) that can stream.
pub fn is_camera(c: &Capability) -> bool {
    let caps = if c.capabilities & CAP_DEVICE_CAPS != 0 { c.device_caps } else { c.capabilities };
    caps & CAP_VIDEO_CAPTURE != 0 && caps & CAP_STREAMING != 0
}

/// A NUL-terminated name field.
pub fn cstr(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).trim().to_string()
}

/// How the camera's YUV is read (BT.601 limited unless it says otherwise).
pub fn yuv_of(p: &PixFormat) -> crate::cam::Yuv {
    let matrix = if p.ycbcr_enc == YCBCR_ENC_709 { crate::cam::Matrix::Bt709 } else { crate::cam::Matrix::Bt601 };
    crate::cam::Yuv { matrix, full: p.quantization == QUANTIZATION_FULL }
}

#[cfg(target_os = "linux")]
pub use dev::{list, V4l2Cam};

#[cfg(target_os = "linux")]
mod dev {
    use super::*;
    use crate::cam::{CamDevice, CamFrame, CamSource, Fmt};
    use anyhow::{anyhow, bail, Context, Result};
    use core::ffi::{c_int, c_short, c_ulong, c_void};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: c_short,
        revents: c_short,
    }

    unsafe extern "C" {
        fn ioctl(fd: c_int, req: c_ulong, ...) -> c_int;
        fn open(path: *const core::ffi::c_char, flags: c_int, ...) -> c_int;
        fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> c_int;
        fn poll(fds: *mut PollFd, n: u64, timeout: c_int) -> c_int;
    }

    const O_RDWR: c_int = 2;
    const O_NONBLOCK: c_int = 0o4000;
    const O_CLOEXEC: c_int = 0o2_000_000;
    const PROT_READ: c_int = 1;
    const MAP_SHARED: c_int = 1;
    const POLLIN: c_short = 1;
    const EINTR: i32 = 4;
    const EAGAIN: i32 = 11;

    /// `ioctl` retried on EINTR.
    fn xioctl<T>(fd: &OwnedFd, req: u32, arg: &mut T) -> std::io::Result<()> {
        loop {
            // SAFETY: `arg` is the struct this request takes.
            let r = unsafe { ioctl(fd.as_raw_fd(), req as c_ulong, arg as *mut T as *mut c_void) };
            if r == 0 {
                return Ok(());
            }
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(EINTR) {
                return Err(e);
            }
        }
    }

    fn open_dev(path: &str) -> std::io::Result<OwnedFd> {
        let c = std::ffi::CString::new(path).map_err(|_| std::io::Error::other("bad path"))?;
        // SAFETY: a valid C string.
        let fd = unsafe { open(c.as_ptr(), O_RDWR | O_NONBLOCK | O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: just opened, owned by nobody else.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn query(fd: &OwnedFd) -> Option<Capability> {
        // SAFETY: plain-old-data.
        let mut cap: Capability = unsafe { core::mem::zeroed() };
        xioctl(fd, VIDIOC_QUERYCAP, &mut cap).ok()?;
        Some(cap)
    }

    /// The capture devices `/dev/video0..63`.
    pub fn list() -> Vec<CamDevice> {
        let mut out = Vec::new();
        for n in 0..64 {
            let path = format!("/dev/video{n}");
            if !std::path::Path::new(&path).exists() {
                continue;
            }
            let Ok(fd) = open_dev(&path) else { continue };
            if let Some(c) = query(&fd).filter(is_camera) {
                let name = cstr(&c.card);
                out.push(CamDevice { id: path, name: if name.is_empty() { tr!("cam.numbered", n) } else { name } });
            }
        }
        out
    }

    fn formats(fd: &OwnedFd) -> Vec<u32> {
        let mut out = Vec::new();
        for index in 0..64 {
            // SAFETY: plain-old-data.
            let mut d: FmtDesc = unsafe { core::mem::zeroed() };
            d.index = index;
            d.ty = BUF_TYPE_VIDEO_CAPTURE;
            if xioctl(fd, VIDIOC_ENUM_FMT, &mut d).is_err() {
                break;
            }
            out.push(d.pixelformat);
        }
        out
    }

    fn set_format(fd: &OwnedFd, fourcc: u32, w: u32, h: u32) -> std::io::Result<PixFormat> {
        // SAFETY: plain-old-data.
        let mut f: Format = unsafe { core::mem::zeroed() };
        f.ty = BUF_TYPE_VIDEO_CAPTURE;
        f.set_pix(PixFormat { width: w, height: h, pixelformat: fourcc, field: FIELD_NONE, ..Default::default() });
        xioctl(fd, VIDIOC_S_FMT, &mut f)?;
        Ok(f.pix())
    }

    /// Ask for 30 fps; the rate the camera says it will deliver.
    fn set_fps(fd: &OwnedFd, fps: u32) -> Option<f32> {
        // SAFETY: plain-old-data.
        let mut p: StreamParm = unsafe { core::mem::zeroed() };
        p.ty = BUF_TYPE_VIDEO_CAPTURE;
        p.numerator = 1;
        p.denominator = fps;
        xioctl(fd, VIDIOC_S_PARM, &mut p).ok()?;
        (p.numerator > 0).then(|| p.denominator as f32 / p.numerator as f32)
    }

    struct Map {
        ptr: *mut c_void,
        len: usize,
    }

    /// An open V4L2 camera, streaming.
    pub struct V4l2Cam {
        fd: OwnedFd,
        maps: Vec<Map>,
        pix: PixFormat,
        fmt: Fmt,
        yuv: crate::cam::Yuv,
    }

    // SAFETY: the mappings belong to this camera and are used by its owner.
    unsafe impl Send for V4l2Cam {}

    impl V4l2Cam {
        /// Open camera `id` (a device path; "" = the first camera).
        pub fn open(id: &str) -> Result<V4l2Cam> {
            let path = if id.is_empty() {
                list().into_iter().next().map(|d| d.id).ok_or_else(|| anyhow!("{}", crate::cam::NO_CAMERA))?
            } else {
                id.to_string()
            };
            let fd = open_dev(&path).with_context(|| format!("open {path}"))?;
            let cap = query(&fd).ok_or_else(|| anyhow!("{path} is not a V4L2 device"))?;
            if !is_camera(&cap) {
                bail!("{path} is not a camera that can stream");
            }
            let fourcc = pick_format(&formats(&fd)).map_err(|e| anyhow!("{e}"))?;
            let (pw, ph) = crate::cam::PREF;
            let mut pix = match set_format(&fd, fourcc, pw, ph) {
                Ok(p) => p,
                Err(_) => set_format(&fd, fourcc, 640, 480).with_context(|| format!("set the picture format of {path}"))?,
            };
            let fps = set_fps(&fd, 30);
            // A USB 2 camera often does 720p YUYV at 10 fps only: 480p then.
            if fps.is_some_and(|f| f < 14.5) && pix.width > 640 {
                if let Ok(p) = set_format(&fd, fourcc, 640, 480) {
                    pix = p;
                }
                let _ = set_fps(&fd, 30);
            }
            if pix.pixelformat != fourcc {
                bail!("{}", if pix.pixelformat == MJPG { MJPG_ONLY } else { NO_FORMAT });
            }
            let fmt = if fourcc == NV12 { Fmt::Nv12 } else { Fmt::Yuy2 };
            let mut cam = V4l2Cam { fd, maps: Vec::new(), pix, fmt, yuv: yuv_of(&pix) };
            cam.start().with_context(|| format!("start {path}"))?;
            eprintln!("rustshot: camera {path} ({}) {}x{} {:?}", cstr(&cap.card), pix.width, pix.height, fmt);
            Ok(cam)
        }

        fn start(&mut self) -> std::io::Result<()> {
            let mut rb = RequestBuffers { count: 4, ty: BUF_TYPE_VIDEO_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
            xioctl(&self.fd, VIDIOC_REQBUFS, &mut rb)?;
            if rb.count < 2 {
                return Err(std::io::Error::other("the camera gave too few buffers"));
            }
            for index in 0..rb.count {
                let mut b = Buffer { index, ty: BUF_TYPE_VIDEO_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
                xioctl(&self.fd, VIDIOC_QUERYBUF, &mut b)?;
                let off = (b.m & 0xFFFF_FFFF) as i64;
                // SAFETY: mapping the buffer the driver described.
                let ptr = unsafe { mmap(core::ptr::null_mut(), b.length as usize, PROT_READ, MAP_SHARED, self.fd.as_raw_fd(), off) };
                if ptr as isize == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                self.maps.push(Map { ptr, len: b.length as usize });
                xioctl(&self.fd, VIDIOC_QBUF, &mut b)?;
            }
            let mut ty = BUF_TYPE_VIDEO_CAPTURE as i32;
            xioctl(&self.fd, VIDIOC_STREAMON, &mut ty)
        }
    }

    impl CamSource for V4l2Cam {
        fn size(&self) -> (u32, u32) {
            (self.pix.width, self.pix.height)
        }

        fn read(&mut self, into: &mut CamFrame) -> Result<bool> {
            let mut p = PollFd { fd: self.fd.as_raw_fd(), events: POLLIN, revents: 0 };
            // SAFETY: one valid pollfd.
            let r = unsafe { poll(&mut p, 1, 200) };
            if r <= 0 {
                return Ok(false);
            }
            let mut b = Buffer { ty: BUF_TYPE_VIDEO_CAPTURE, memory: MEMORY_MMAP, ..Default::default() };
            if let Err(e) = xioctl(&self.fd, VIDIOC_DQBUF, &mut b) {
                if e.raw_os_error() == Some(EAGAIN) {
                    return Ok(false);
                }
                return Err(anyhow!("the camera stopped: {e}"));
            }
            let ok = (|| {
                let m = self.maps.get(b.index as usize)?;
                let n = (b.bytesused as usize).min(m.len);
                // SAFETY: the mapping is `m.len` long; the driver filled `n`.
                let data = unsafe { core::slice::from_raw_parts(m.ptr as *const u8, n) };
                let (w, h) = (self.pix.width, self.pix.height);
                let stride = self.pix.bytesperline as usize;
                let l = crate::cam::layout(self.fmt, w, h, h, stride.max(1), n)?;
                match self.fmt {
                    Fmt::Nv12 => crate::cam::nv12_to_bgra(&data[..l.uv], &data[l.uv..l.len], l.stride, w, h, self.yuv, &mut into.bgra)?,
                    _ => crate::cam::yuy2_to_bgra(data, l.stride, w, h, self.yuv, &mut into.bgra)?,
                }
                into.w = w & !1;
                into.h = if self.fmt == Fmt::Nv12 { h & !1 } else { h };
                Some(())
            })();
            xioctl(&self.fd, VIDIOC_QBUF, &mut b).map_err(|e| anyhow!("the camera stopped: {e}"))?;
            Ok(ok.is_some())
        }
    }

    impl Drop for V4l2Cam {
        fn drop(&mut self) {
            let mut ty = BUF_TYPE_VIDEO_CAPTURE as i32;
            let _ = xioctl(&self.fd, VIDIOC_STREAMOFF, &mut ty);
            for m in &self.maps {
                // SAFETY: mapped by `start`.
                unsafe {
                    munmap(m.ptr, m.len);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_the_kernel() {
        // From <linux/videodev2.h> on x86_64 / aarch64.
        assert_eq!(VIDIOC_QUERYCAP, 0x8068_5600);
        assert_eq!(VIDIOC_ENUM_FMT, 0xC040_5602);
        assert_eq!(VIDIOC_G_FMT, 0xC0D0_5604);
        assert_eq!(VIDIOC_S_FMT, 0xC0D0_5605);
        assert_eq!(VIDIOC_REQBUFS, 0xC014_5608);
        assert_eq!(VIDIOC_QUERYBUF, 0xC058_5609);
        assert_eq!(VIDIOC_QBUF, 0xC058_560F);
        assert_eq!(VIDIOC_DQBUF, 0xC058_5611);
        assert_eq!(VIDIOC_STREAMON, 0x4004_5612);
        assert_eq!(VIDIOC_STREAMOFF, 0x4004_5613);
        assert_eq!(VIDIOC_G_PARM, 0xC0CC_5615);
        assert_eq!(VIDIOC_S_PARM, 0xC0CC_5616);
    }

    #[test]
    fn structs_have_the_64_bit_layouts() {
        assert_eq!(size_of::<Capability>(), 104);
        assert_eq!(size_of::<FmtDesc>(), 64);
        assert_eq!(size_of::<PixFormat>(), 48);
        assert_eq!(size_of::<Format>(), 208);
        assert_eq!(size_of::<RequestBuffers>(), 20);
        assert_eq!(size_of::<Buffer>(), 88);
        assert_eq!(core::mem::offset_of!(Buffer, timestamp), 24);
        assert_eq!(core::mem::offset_of!(Buffer, m), 64);
        assert_eq!(core::mem::offset_of!(Buffer, length), 72);
        assert_eq!(size_of::<StreamParm>(), 204);
        assert_eq!(core::mem::offset_of!(StreamParm, denominator), 16);
    }

    #[test]
    fn formats_are_picked_like_on_windows() {
        assert_eq!(YUYV, 0x5659_5559);
        assert_eq!(pick_format(&[MJPG, YUYV]), Ok(YUYV));
        assert_eq!(pick_format(&[NV12, MJPG]), Ok(NV12));
        assert_eq!(pick_format(&[MJPG]), Err(MJPG_ONLY.get()));
        assert_eq!(pick_format(&[fourcc(b"H264")]), Err(NO_FORMAT.get()));
        assert_eq!(pick_format(&[]), Err(NO_FORMAT.get()));
    }

    #[test]
    fn capture_nodes_are_told_from_metadata_nodes() {
        // SAFETY: plain-old-data.
        let mut c: Capability = unsafe { core::mem::zeroed() };
        c.capabilities = CAP_DEVICE_CAPS | CAP_VIDEO_CAPTURE | CAP_STREAMING | 0x0080_0000;
        c.device_caps = CAP_VIDEO_CAPTURE | CAP_STREAMING;
        assert!(is_camera(&c));
        c.device_caps = 0x0080_0000 | CAP_STREAMING; // metadata capture only
        assert!(!is_camera(&c));
        c.card[..7].copy_from_slice(b"HD Cam\0");
        assert_eq!(cstr(&c.card), "HD Cam");
    }

    #[test]
    fn yuyv_pictures_convert_with_their_color_space() {
        // One YUYV pair, white and black, BT.601 limited.
        let src = [235u8, 128, 16, 128];
        let mut out = Vec::new();
        let p = PixFormat { width: 2, height: 1, ..Default::default() };
        crate::cam::yuy2_to_bgra(&src, 4, 2, 1, yuv_of(&p), &mut out);
        assert_eq!(out, [255, 255, 255, 255, 0, 0, 0, 255]);
        let full = PixFormat { ycbcr_enc: YCBCR_ENC_709, quantization: QUANTIZATION_FULL, ..Default::default() };
        assert_eq!(yuv_of(&full), crate::cam::Yuv { matrix: crate::cam::Matrix::Bt709, full: true });
    }
}
