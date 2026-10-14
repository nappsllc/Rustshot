//! Pictures of a recording for the trim window: its thumbnails and the
//! preview while it plays. GIF with our own decoder ([`super::gif`]);
//! MP4 through the platform: Windows Media Foundation's source reader
//! (NV12, converted here), Linux ffmpeg (one picture a call, or a stream
//! while playing). Without a decoder the window still trims, without
//! pictures.

use super::gif::{Index, Player};
use crate::pixbuf::PixBuf;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Gives pictures of a recording.
pub trait Decoder: Send {
    fn length(&self) -> Duration;
    /// The picture shown at `t`, fitted into `max` (aspect kept).
    fn frame_at(&mut self, t: Duration, max: (u32, u32)) -> Option<PixBuf>;
}

/// The decoder for `path` (MP4 or GIF).
pub fn open(path: &Path) -> Result<Box<dyn Decoder>> {
    match super::Kind::of(path) {
        Some(super::Kind::Gif) => {
            let idx = Index::read(&mut std::io::BufReader::new(std::fs::File::open(path)?))?;
            Ok(Box::new(GifDecoder { p: Player::new(Arc::new(idx)) }))
        }
        Some(super::Kind::Mp4) => open_mp4(path),
        None => anyhow::bail!("not a recording"),
    }
}

fn fit(p: PixBuf, max: (u32, u32)) -> PixBuf {
    let (w, h) = p.dimensions();
    if w <= max.0 && h <= max.1 {
        return p;
    }
    crate::scale::fit(&p, max.0.max(1), max.1.max(1))
}

pub struct GifDecoder {
    p: Player,
}

impl Decoder for GifDecoder {
    fn length(&self) -> Duration {
        self.p.idx.length()
    }

    fn frame_at(&mut self, t: Duration, max: (u32, u32)) -> Option<PixBuf> {
        let i = self.p.idx.frame_at(t);
        let (w, h) = self.p.size();
        // Transparent holes show as the trim window's dark gray.
        let mut px = self.p.frame(i).to_vec();
        for q in px.as_chunks_mut::<4>().0 {
            if q[3] == 0 {
                *q = [24, 24, 28, 255];
            }
        }
        Some(fit(PixBuf::from_raw(w, h, px)?, max))
    }
}

/// NV12 (BT.709 limited range, `stride` bytes a row) → RGBA.
pub fn nv12_to_rgba(nv: &[u8], w: u32, h: u32, stride: usize) -> Option<PixBuf> {
    let (w, h) = (w as usize, h as usize);
    if nv.len() < stride * h + stride * h.div_ceil(2) {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    let uv0 = stride * h;
    for y in 0..h {
        for x in 0..w {
            let yv = nv[y * stride + x] as f32 - 16.0;
            let ci = uv0 + (y / 2) * stride + (x / 2) * 2;
            let (u, v) = (nv[ci] as f32 - 128.0, nv[ci + 1] as f32 - 128.0);
            let c = 1.164 * yv;
            let r = c + 1.793 * v;
            let g = c - 0.213 * u - 0.533 * v;
            let b = c + 2.112 * u;
            let o = (y * w + x) * 4;
            let q = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            out[o..o + 4].copy_from_slice(&[q(r), q(g), q(b), 255]);
        }
    }
    PixBuf::from_raw(w as u32, h as u32, out)
}

#[cfg(windows)]
fn open_mp4(path: &Path) -> Result<Box<dyn Decoder>> {
    Ok(Box::new(win::MfDecoder::open(path)?))
}

#[cfg(target_os = "linux")]
fn open_mp4(path: &Path) -> Result<Box<dyn Decoder>> {
    Ok(Box::new(linux::FfDecoder::open(path)?))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_mp4(_path: &Path) -> Result<Box<dyn Decoder>> {
    anyhow::bail!("no MP4 decoder here yet")
}

#[cfg(windows)]
mod win {
    //! Media Foundation's source reader, NV12 out; used on one MTA thread
    //! (the trim window's decoding worker).
    use super::{fit, nv12_to_rgba, Decoder};
    use crate::pixbuf::PixBuf;
    use crate::rec::win::mf;
    use anyhow::Result;
    use std::path::Path;
    use std::time::Duration;
    use windows::core::GUID;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToUInt64, PROPVARIANT};
    use windows::Win32::System::Variant::VT_I8;


    pub struct MfDecoder {
        reader: IMFSourceReader,
        size: (u32, u32),
        stride: usize,
        length: Duration,
        /// The time of the last picture read (100 ns; before 0 for the
        /// frames an edit list hides): reading on is cheaper than seeking
        /// for a near later time.
        at: Option<i64>,
        last: Option<(i64, PixBuf)>,
    }

    // SAFETY: made and used on the decoding worker's thread only (the MTA).
    unsafe impl Send for MfDecoder {}

    const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    /// 20 ms in 100 ns: a picture this close to the time asked will do.
    const NEAR: i64 = 200_000;

    impl MfDecoder {
        pub fn open(path: &Path) -> Result<MfDecoder> {
            crate::rec::win::com_init();
            unsafe {
                let reader = mf::open_reader(path)?;
                let mut pv = reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)?;
                let hns = PropVariantToUInt64(&pv);
                let _ = PropVariantClear(&mut pv);
                let length = Duration::from_nanos(hns? * 100);
                let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
                reader.SetStreamSelection(VIDEO, true)?;
                let want = mf::new_media_type()?;
                want.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                want.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                reader.SetCurrentMediaType(VIDEO, None, &want)?;
                let cur = reader.GetCurrentMediaType(VIDEO)?;
                let fs = cur.GetUINT64(&MF_MT_FRAME_SIZE)?;
                let size = ((fs >> 32) as u32, fs as u32);
                let stride = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).map_or(size.0 as usize, |s| (s as i32).unsigned_abs() as usize);
                Ok(MfDecoder { reader, size, stride, length, at: None, last: None })
            }
        }

        fn seek(&mut self, hns: i64) -> Result<()> {
            let mut pv = PROPVARIANT::default();
            unsafe {
                let inner = &mut *pv.Anonymous.Anonymous;
                inner.vt = VT_I8;
                inner.Anonymous.hVal = hns.max(0);
                self.reader.SetCurrentPosition(&GUID::zeroed(), &pv)?;
            }
            self.at = None;
            Ok(())
        }

        /// The next picture and its time (100 ns).
        fn read(&mut self) -> Option<(i64, PixBuf)> {
            unsafe {
                for _ in 0..4 {
                    let (mut flags, mut ts, mut sample) = (0u32, 0i64, None);
                    self.reader.ReadSample(VIDEO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample)).ok()?;
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        return None;
                    }
                    let Some(s) = sample else { continue };
                    let buf = s.ConvertToContiguousBuffer().ok()?;
                    let (mut data, mut len) = (std::ptr::null_mut(), 0u32);
                    buf.Lock(&mut data, None, Some(&mut len)).ok()?;
                    let px = nv12_to_rgba(std::slice::from_raw_parts(data, len as usize), self.size.0, self.size.1, self.stride);
                    let _ = buf.Unlock();
                    self.at = Some(ts);
                    return px.map(|p| (ts, p));
                }
                None
            }
        }
    }

    impl Decoder for MfDecoder {
        fn length(&self) -> Duration {
            self.length
        }

        fn frame_at(&mut self, t: Duration, max: (u32, u32)) -> Option<PixBuf> {
            let want = (t.as_nanos() / 100) as i64;
            if let Some((lt, p)) = &self.last
                && *lt <= want
                && self.at.is_some_and(|a| want < a + NEAR)
            {
                return Some(fit(p.clone(), max));
            }
            // Read on for up to a second ahead; seek otherwise.
            let near = self.at.is_some_and(|a| want >= a && want < a + 10_000_000);
            if !near {
                self.seek(want).ok()?;
            }
            let mut best = None;
            for _ in 0..600 {
                let Some((ft, p)) = self.read() else { break };
                let done = ft + NEAR >= want;
                best = Some((ft, p));
                if done {
                    break;
                }
            }
            let (ft, p) = best.or_else(|| self.last.clone())?;
            self.last = Some((ft, p.clone()));
            Some(fit(p, max))
        }
    }
}


#[cfg(target_os = "linux")]
mod linux {
    //! ffmpeg: a picture at a time (`-ss t -frames:v 1`), raw RGBA on a
    //! pipe, scaled by ffmpeg to the size asked.
    use super::Decoder;
    use crate::pixbuf::PixBuf;
    use anyhow::{bail, Context, Result};
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    pub struct FfDecoder {
        exe: PathBuf,
        path: PathBuf,
        length: Duration,
        size: (u32, u32),
    }

    impl FfDecoder {
        pub fn open(path: &Path) -> Result<FfDecoder> {
            let exe = crate::rec::linux::ffmpeg::find().context("ffmpeg is not installed: no pictures")?;
            let length = super::super::length(path)?;
            let size = mp4_size(path).unwrap_or((1280, 720));
            Ok(FfDecoder { exe, path: path.to_path_buf(), length, size })
        }
    }

    /// The picture size from the first video track's `tkhd` (16.16 fixed).
    /// Only `moov` is read, never the whole file.
    fn mp4_size(path: &Path) -> Option<(u32, u32)> {
        let mut f = std::io::BufReader::new(std::fs::File::open(path).ok()?);
        crate::rec::cut::mp4::video_size(&mut f).ok().flatten()
    }

    impl Decoder for FfDecoder {
        fn length(&self) -> Duration {
            self.length
        }

        fn frame_at(&mut self, t: Duration, max: (u32, u32)) -> Option<PixBuf> {
            let k = (max.0 as f32 / self.size.0 as f32).min(max.1 as f32 / self.size.1 as f32).min(1.0);
            let (w, h) = (((self.size.0 as f32 * k) as u32).max(2) & !1, ((self.size.1 as f32 * k) as u32).max(2) & !1);
            let run = || -> Result<Vec<u8>> {
                let mut c = Command::new(&self.exe)
                    .args(["-v", "error", "-nostdin", "-ss"])
                    .arg(format!("{:.3}", t.as_secs_f64()))
                    .arg("-i")
                    .arg(&self.path)
                    .args(["-frames:v", "1", "-vf"])
                    .arg(format!("scale={w}:{h}"))
                    .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()?;
                let mut out = Vec::new();
                c.stdout.take().context("no pipe")?.read_to_end(&mut out)?;
                let _ = c.wait();
                if out.len() < (w * h * 4) as usize {
                    bail!("no picture");
                }
                out.truncate((w * h * 4) as usize);
                Ok(out)
            };
            PixBuf::from_raw(w, h, run().ok()?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_converts_to_rgb() {
        // 2x2 white-ish, black, a red and the chroma of gray.
        let (w, h) = (2u32, 2u32);
        let nv = vec![235, 16, 81, 145, 128, 128];
        let p = nv12_to_rgba(&nv, w, h, 2).unwrap();
        assert_eq!(p.get_pixel(0, 0)[..3], [255, 255, 255]);
        assert_eq!(p.get_pixel(1, 0)[..3], [0, 0, 0]);
        assert!(nv12_to_rgba(&nv[..4], w, h, 2).is_none(), "too short");
    }

    #[test]
    fn gif_pictures_come_fitted() {
        let mut enc = crate::rec::gif::GifEncoder::new(std::io::Cursor::new(Vec::new()), 80, 40, 10).unwrap();
        for k in 0..3u64 {
            let px = [k as u8 * 80, 60, 60, 255].repeat(80 * 40);
            use crate::rec::VideoEncoder;
            enc.push_video(&crate::rec::Frame::bgra(80, 40, px, Duration::from_millis(k * 100))).unwrap();
        }
        let bytes = enc.into_inner().unwrap().into_inner();
        let dir = std::env::temp_dir().join(format!("rustshot-decode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("clip.gif");
        std::fs::write(&p, bytes).unwrap();
        let mut d = open(&p).unwrap();
        assert_eq!(d.length(), Duration::from_millis(300));
        let pic = d.frame_at(Duration::from_millis(150), (40, 40)).unwrap();
        assert_eq!(pic.dimensions(), (40, 20));
        assert_eq!(pic.get_pixel(20, 10)[2], 80, "frame 1's blue (BGRA 80 → RGBA b)");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
