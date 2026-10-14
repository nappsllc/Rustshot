//! Windows: Media Foundation's MP4 source (the Films & TV app, Media
//! Player, the trim window's own preview) ignores edit lists, so an
//! edit-list trim would show it from the key frame before the start. Here
//! the start is made exact in the samples: the frames from the start to
//! the next key frame are decoded and encoded again with the same H.264
//! encoder (so their parameter sets match the rest), and the rest is
//! copied; the head is decoded straight into the encoder, a picture at a
//! time. The end is cut in decode order, the sound from the packet at the
//! start (through its own edit list). When the new frames' parameter sets
//! do not match
//! (another encoder, settings that cannot be told), the edit-list trim is
//! used instead.

use super::mp4::{write_movie, Keep, Movie, OutSample, OutTrack, Track};
use crate::rec::win::{mf, MfEncoder};
use crate::rec::{Frame, Nv12, Quality, VideoEncoder};
use anyhow::{bail, ensure, Context, Result};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Variant::VT_I8;

/// The `avcC` box inside a track's sample descriptions.
fn avcc(stsd: &[u8]) -> Option<&[u8]> {
    let at = stsd.windows(4).position(|w| w == b"avcC")?;
    let size = u32::from_be_bytes(stsd.get(at - 4..at)?.try_into().ok()?) as usize;
    stsd.get(at - 4..at - 4 + size)
}

/// The source's pictures from a time on, one at a time (tight NV12): the
/// head is decoded straight into the encoder, never held whole (a 4K60
/// GOP would be gigabytes).
struct Pictures {
    r: IMFSourceReader,
    w: u32,
    h: u32,
    stride: usize,
}

const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

impl Pictures {
    /// From `from` (100 ns) on.
    fn open(path: &Path, from: i64) -> Result<Pictures> {
        unsafe {
            let r = mf::open_reader(path)?;
            let _ = r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
            r.SetStreamSelection(VIDEO, true)?;
            let want = mf::new_media_type()?;
            want.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            want.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            r.SetCurrentMediaType(VIDEO, None, &want)?;
            let cur = r.GetCurrentMediaType(VIDEO)?;
            let fs = cur.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let (w, h) = ((fs >> 32) as u32, fs as u32);
            let stride = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).map_or(w as usize, |s| (s as i32).unsigned_abs() as usize);
            let mut pv = PROPVARIANT::default();
            let inner = &mut *pv.Anonymous.Anonymous;
            inner.vt = VT_I8;
            inner.Anonymous.hVal = from.max(0);
            r.SetCurrentPosition(&GUID::zeroed(), &pv)?;
            Ok(Pictures { r, w, h, stride })
        }
    }

    /// The next picture shown at or after `from` and before `to`, with its
    /// time (100 ns); `None` past `to` or at the end.
    fn next(&mut self, from: i64, to: i64) -> Result<Option<(i64, Vec<u8>)>> {
        unsafe {
            for _ in 0..100_000 {
                let (mut flags, mut ts, mut sample) = (0u32, 0i64, None);
                self.r.ReadSample(VIDEO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 || ts >= to {
                    return Ok(None);
                }
                let Some(s) = sample else { continue };
                if ts < from {
                    continue;
                }
                let buf = s.ConvertToContiguousBuffer()?;
                let (mut data, mut len) = (std::ptr::null_mut(), 0u32);
                buf.Lock(&mut data, None, Some(&mut len))?;
                let src = std::slice::from_raw_parts(data, len as usize);
                let (wu, hu) = (self.w as usize, self.h as usize);
                let rows = hu + hu / 2;
                let ok = src.len() >= self.stride * rows;
                let mut tight = Vec::new();
                if ok {
                    tight.reserve_exact(wu * rows);
                    for y in 0..rows {
                        tight.extend_from_slice(&src[y * self.stride..y * self.stride + wu]);
                    }
                }
                let _ = buf.Unlock();
                ensure!(ok, "a decoded picture is short");
                return Ok(Some((ts, tight)));
            }
            Ok(None)
        }
    }
}

/// A file removed when dropped (the encoded head, whatever happens).
struct Temp(std::path::PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The source's pictures from `from` to before `to` (100 ns), encoded
/// again into `tmp` with the first settings that give the parameter sets
/// `want`; the head's movie, or `None`. Each try decodes the head again,
/// one picture at a time.
fn encode_head(src: &Path, tmp: &Path, from: i64, to: i64, fps: &[u32], want: &[u8]) -> Result<Option<Movie>> {
    for &f in fps {
        for q in [Quality::Medium, Quality::High, Quality::Low] {
            let mut p = Pictures::open(src, from)?;
            let (w, h) = (p.w, p.h);
            let mut e = MfEncoder::create(tmp, w, h, f, q, false)?;
            let mut n = 0;
            while let Some((ts, nv)) = p.next(from, to)? {
                let at = Duration::from_nanos(((ts - from).max(0) as u64) * 100);
                let pic = Nv12 { w, h, data: Arc::new(nv) };
                e.push_video(&Frame { w, h, bgra: Vec::new(), ts: at, nv12: Some(pic) })?;
                n += 1;
            }
            ensure!(n > 0, "nothing decoded at the start");
            Box::new(e).finish()?;
            let m = Movie::read(&mut std::io::BufReader::new(std::fs::File::open(tmp)?))?;
            let same = {
                let (_, ts) = m.tracks()?;
                ts.iter().find(|t| t.video).and_then(|t| avcc(t.stsd.raw)).is_some_and(|a| a == want)
            };
            if same {
                return Ok(Some(m));
            }
        }
    }
    Ok(None)
}

fn ticks(d: Duration, scale: u32) -> i64 {
    (d.as_nanos() * scale as u128 / 1_000_000_000) as i64
}

fn hns(t: i64, scale: u32) -> i64 {
    (t as i128 * 10_000_000 / scale.max(1) as i128) as i64
}

/// The sound kept: from the packet at the start (through the track's own
/// edit list: AAC priming, a microphone that opened late) to the end; its
/// edit list skips the rest of that packet.
fn audio_out<'t, 'a>(t: &'t Track<'a>, movie: u32, start: Duration, end: Duration) -> Option<OutTrack<'t, 'a>> {
    let k = super::mp4::plan(&t.s, t.edit, t.timescale, movie, false, start, end, Duration::ZERO)?;
    Some(OutTrack::kept(t, k))
}

/// Trim `src` to `dst` exactly, encoding the head again when the start is
/// not on a key frame. Errors when that cannot be done (the caller then
/// trims with edit lists).
pub fn trim(src: &Path, dst: &Path, start: Duration, end: Duration) -> Result<()> {
    ensure!(end > start, "the trim is empty");
    crate::rec::win::com_init();
    let m = Movie::read(&mut std::io::BufReader::new(std::fs::File::open(src)?))?;
    let (movie, tracks) = m.tracks()?;
    let v = tracks.iter().find(|t| t.video).context("no video")?;
    ensure!(v.edit == Default::default(), "the recording has an edit list");
    let s = &v.s;
    let n = s.len();
    let dts = s.dts();
    let cts: Vec<i64> = (0..n).map(|i| s.cts(i, dts[i])).collect();
    let ts = v.timescale;
    let frame = (s.durations.iter().map(|d| *d as u64).sum::<u64>() / n.max(1) as u64).max(1) as i64;
    let (st, et) = (ticks(start, ts), ticks(end, ts));
    // The key frame at (half a frame after) or before the start.
    let k0 = (0..n).rev().find(|&i| s.is_sync(i) && cts[i] <= st + frame / 2).or_else(|| (0..n).find(|&i| s.is_sync(i))).context("no key frame")?;
    let last = (k0..n).rev().find(|&i| cts[i] < et).map_or(k0 + 1, |i| i + 1);
    let mut video = Vec::new();
    let mut head_movie = None;
    let tail_from = if (cts[k0] - st).abs() <= frame / 2 {
        // On a key frame: no frame to make again.
        k0
    } else {
        let k1 = (k0 + 1..n).find(|&i| s.is_sync(i)).unwrap_or(n);
        // Up to the next key frame, or the end when that comes first.
        let to = if k1 < n { hns(cts[k1], ts) } else { i64::MAX }.min(hns(et, ts));
        let want = avcc(v.stsd.raw).context("not H.264")?.to_vec();
        let measured = (ts as i64 / frame) as u32;
        // Removed however this ends.
        let tmp = Temp(dst.with_extension("head.mp4"));
        let fps: Vec<u32> = [measured, 30, 60].into_iter().fold(Vec::new(), |mut a, f| {
            if f > 0 && !a.contains(&f) {
                a.push(f);
            }
            a
        });
        let Some(hm) = encode_head(src, &tmp.0, hns(st, ts), to, &fps, &want)? else {
            bail!("the encoder's parameter sets differ");
        };
        head_movie = Some((hm, tmp));
        k1
    };
    // The head's samples (source 1), then the tail's (source 0).
    if let Some((hm, _)) = &head_movie {
        let (_, htracks) = hm.tracks()?;
        let h = htracks.iter().find(|t| t.video).context("no video in the new head")?;
        ensure!(h.timescale == ts, "the new head has another time scale");
        let hs = &h.s;
        let d_h = hs.ctts.as_ref().map_or(0, |(c, _)| c[0]) as i64;
        for i in 0..hs.len() {
            video.push(OutSample {
                src: 1,
                off: hs.offsets[i],
                size: hs.sizes[i],
                dur: hs.durations[i],
                cto: hs.ctts.as_ref().map_or(0, |(c, _)| c[i]),
                sync: hs.is_sync(i),
                desc: 1,
            });
        }
        // The tail's decode times go on from the head's: its first sample
        // decodes at its old time less the start (plus the head's delay).
        if tail_from < n {
            let tail_dts = dts[tail_from] as i64 - (st - d_h);
            let before: i64 = video[..video.len() - 1].iter().map(|x| x.dur as i64).sum();
            let gap = tail_dts - before;
            ensure!(gap > 0, "the new head runs into the rest");
            if let Some(l) = video.last_mut() {
                l.dur = gap as u32;
            }
        }
    }
    for i in tail_from..last.max(tail_from) {
        video.push(OutSample {
            src: 0,
            off: s.offsets[i],
            size: s.sizes[i],
            dur: s.durations[i],
            cto: s.ctts.as_ref().map_or(0, |(c, _)| c[i]),
            sync: s.is_sync(i),
            desc: s.desc[i],
        });
    }
    ensure!(!video.is_empty(), "nothing to keep");
    let shown = ticks(end - start, movie) as u64;
    let vk = Keep { first: 0, last: video.len(), empty: 0, media_time: 0, shown };
    let mut outs = vec![OutTrack { t: v, samples: video, ctts: s.ctts.as_ref().map(|c| c.1).or(Some(0)), stss: true, edit: vk, stsd: None }];
    outs.extend(tracks.iter().filter(|t| !t.video).filter_map(|t| audio_out(t, movie, start, end)));
    let mut out = std::io::BufWriter::new(std::fs::File::create(dst)?);
    let mut src_f = std::io::BufReader::new(std::fs::File::open(src)?);
    match &head_movie {
        Some((_, tmp)) => {
            let mut head_f = std::io::BufReader::new(std::fs::File::open(&tmp.0)?);
            write_movie(&m, &outs, &mut [&mut src_f, &mut head_f], &mut out)?;
        }
        None => {
            write_movie(&m, &outs, &mut [&mut src_f], &mut out)?;
        }
    }
    Ok(())
}
