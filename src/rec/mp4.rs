//! A small MP4 muxer for the Linux encoder without ffmpeg (OpenH264 video,
//! Opus sound): platform-neutral and unit-tested everywhere.
//!
//! The file is `ftyp` (isom, iso6, mp41), then one `mdat` streamed to disk
//! as the samples come (its 64-bit size patched at the end), then `moov`.
//! Each sample is its own chunk (`stsc` has one entry), so the sample
//! tables are `stsz` sizes and `stco` offsets (`co64` once an offset passes
//! 4 GB); the picture's sample times are kept (variable frame rate:
//! `stts` is run-length coded from the frame timestamps), with `stss` for
//! the IDR frames. Opus has an edit list that skips the encoder's
//! pre-skip and ends at the real end of the sound.
//!
//! Faststart: [`Mp4Writer::finish`] first writes `moov` after `mdat` (the
//! file plays from then on) and then moves it to the front *in place*: the
//! file grows by the size of the new `moov`, `mdat` is copied towards the
//! end from its last byte backwards in 1 MiB blocks, and the new `moov`
//! (offsets moved by its size) goes in front of it. Memory stays bounded
//! (one block plus the sample tables, about 16 bytes a sample) and no
//! second copy of the recording is needed on disk. If the file cannot grow
//! (or the disk has less than the new `moov` plus [`FREE_MARGIN`] free) the
//! recording is kept with `moov` at the end (it still plays).
//!
//! Not growing is the only graceful failure: once the move has started, a
//! crash, a kill or ENOSPC half-way leaves `mdat` partly shifted and the
//! file unplayable (the tail `moov` is overwritten by the move). ffmpeg's
//! `-movflags +faststart` has the same window; the free-space check before
//! the move keeps ENOSPC out of it in practice.
//!
//! Annex-B to AVCC conversion and `avcC`/`dOps` building are here too
//! ([`access_unit`], [`avcc_body`], [`dops_body`]).
#![cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))] // used by the Linux OpenH264 encoder

use std::io::{self, Read, Seek, SeekFrom, Write};

/// The picture's track time scale (ticks a second).
pub const VIDEO_TIMESCALE: u32 = 90_000;
/// The sound's: its sample rate.
pub const AUDIO_TIMESCALE: u32 = 48_000;
/// `mvhd`'s (edit lists and track headers count in it).
pub const MOVIE_TIMESCALE: u32 = 1000;
/// Samples in one Opus packet (20 ms at 48 kHz).
pub const OPUS_FRAME: u32 = 960;

const VIDEO_ID: u32 = 1;
const AUDIO_ID: u32 = 2;
/// The block moved at a time when `moov` goes to the front (tests move
/// small blocks so that every file goes through the many-block path).
const MOVE_BLOCK: usize = if cfg!(test) { 64 } else { 1 << 20 };
/// Free space kept beyond the new `moov` before moving it to the front.
pub const FREE_MARGIN: u64 = 1 << 20;
/// `mdat`'s header: 32-bit size 1, then a 64-bit size.
const MDAT_HEAD: u64 = 16;

/// Where the muxer writes: a file (or a buffer in tests).
pub trait Media: Read + Write + Seek {
    fn set_len(&mut self, n: u64) -> io::Result<()>;
    /// Bytes free for this file on its disk, when known.
    fn free_space(&self) -> Option<u64> {
        None
    }
}

impl Media for std::fs::File {
    fn set_len(&mut self, n: u64) -> io::Result<()> {
        std::fs::File::set_len(self, n)
    }
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    fn free_space(&self) -> Option<u64> {
        statvfs_free(self)
    }
}

/// `fstatvfs`: blocks free to unprivileged users times the fragment size.
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
fn statvfs_free(f: &std::fs::File) -> Option<u64> {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn fstatvfs(fd: i32, buf: *mut u64) -> i32;
    }
    // glibc/musl `struct statvfs` on 64-bit Linux: f_bsize, f_frsize,
    // f_blocks, f_bfree, f_bavail, ... (all 8 bytes), 112 bytes in all;
    // the buffer is larger to be safe.
    let mut buf = [0u64; 32];
    // SAFETY: the fd is open for the call; the buffer outsizes the struct.
    if unsafe { fstatvfs(f.as_raw_fd(), buf.as_mut_ptr()) } != 0 {
        return None;
    }
    Some(buf[4].saturating_mul(buf[1]))
}

impl Media for io::Cursor<Vec<u8>> {
    fn set_len(&mut self, n: u64) -> io::Result<()> {
        self.get_mut().resize(n as usize, 0);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// H.264 access units
// ---------------------------------------------------------------------------

/// The NAL units of an Annex-B stream (start codes `00 00 01` or
/// `00 00 00 01`; trailing zero bytes before a start code dropped).
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let begin = s + 3;
        let mut end = starts.get(k + 1).copied().unwrap_or(data.len());
        while end > begin && data[end - 1] == 0 {
            end -= 1;
        }
        if end > begin {
            out.push(&data[begin..end]);
        }
    }
    out
}

/// One encoded picture as MP4 wants it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccessUnit {
    /// The picture's NAL units, each after a 4-byte big-endian length
    /// (parameter sets and access unit delimiters left out).
    pub avcc: Vec<u8>,
    /// The sequence and picture parameter sets in it, if any.
    pub sps: Option<Vec<u8>>,
    pub pps: Option<Vec<u8>>,
    /// It has an IDR slice (a sync sample).
    pub idr: bool,
}

/// An Annex-B access unit as AVCC (see [`AccessUnit`]).
pub fn access_unit(annexb: &[u8]) -> AccessUnit {
    let mut au = AccessUnit::default();
    for nal in nal_units(annexb) {
        match nal[0] & 0x1f {
            7 => au.sps = Some(nal.to_vec()),
            8 => au.pps = Some(nal.to_vec()),
            9 => {} // access unit delimiter
            t => {
                au.idr |= t == 5;
                au.avcc.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                au.avcc.extend_from_slice(nal);
            }
        }
    }
    au
}

/// The body of an `avcC` box for one SPS and one PPS (4-byte lengths);
/// `None` when the SPS is too short to hold the profile and level.
pub fn avcc_body(sps: &[u8], pps: &[u8]) -> Option<Vec<u8>> {
    if sps.len() < 4 || pps.is_empty() || sps.len() > 0xffff || pps.len() > 0xffff {
        return None;
    }
    let mut b = vec![1, sps[1], sps[2], sps[3], 0xfc | 3, 0xe0 | 1];
    b.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    b.extend_from_slice(sps);
    b.push(1);
    b.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    b.extend_from_slice(pps);
    Some(b)
}

/// The body of a `dOps` box (Opus in ISO BMFF): stereo, 48 kHz, no gain,
/// channel mapping family 0.
pub fn dops_body(channels: u8, pre_skip: u16) -> Vec<u8> {
    let mut b = vec![0, channels];
    b.extend_from_slice(&pre_skip.to_be_bytes());
    b.extend_from_slice(&AUDIO_TIMESCALE.to_be_bytes());
    b.extend_from_slice(&0i16.to_be_bytes());
    b.push(0);
    b
}

// ---------------------------------------------------------------------------
// Boxes
// ---------------------------------------------------------------------------

fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_be_bytes());
}
fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_be_bytes());
}
fn put64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 8);
    put32(&mut b, (body.len() + 8) as u32);
    b.extend_from_slice(kind);
    b.extend_from_slice(body);
    b
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 4);
    put32(&mut b, (u32::from(version) << 24) | (flags & 0xff_ffff));
    b.extend_from_slice(body);
    boxed(kind, &b)
}

fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn scale(t: u64, from: u32, to: u32) -> u64 {
    ((u128::from(t) * u128::from(to) + u128::from(from) / 2) / u128::from(from.max(1))) as u64
}

fn mvhd(duration: u64, next_id: u32) -> Vec<u8> {
    let v1 = duration > u64::from(u32::MAX);
    let mut b = Vec::new();
    if v1 {
        put64(&mut b, 0);
        put64(&mut b, 0);
        put32(&mut b, MOVIE_TIMESCALE);
        put64(&mut b, duration);
    } else {
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, MOVIE_TIMESCALE);
        put32(&mut b, duration as u32);
    }
    put32(&mut b, 0x0001_0000); // rate 1.0
    put16(&mut b, 0x0100); // volume 1.0
    b.extend_from_slice(&[0; 10]);
    for m in MATRIX {
        put32(&mut b, m);
    }
    b.extend_from_slice(&[0; 24]);
    put32(&mut b, next_id);
    full(b"mvhd", u8::from(v1), 0, &b)
}

fn tkhd(id: u32, duration: u64, audio: bool, w: u32, h: u32) -> Vec<u8> {
    let v1 = duration > u64::from(u32::MAX);
    let mut b = Vec::new();
    if v1 {
        put64(&mut b, 0);
        put64(&mut b, 0);
        put32(&mut b, id);
        put32(&mut b, 0);
        put64(&mut b, duration);
    } else {
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, id);
        put32(&mut b, 0);
        put32(&mut b, duration as u32);
    }
    b.extend_from_slice(&[0; 8]);
    put16(&mut b, 0); // layer
    put16(&mut b, u16::from(audio)); // alternate group
    put16(&mut b, if audio { 0x0100 } else { 0 });
    put16(&mut b, 0);
    for m in MATRIX {
        put32(&mut b, m);
    }
    put32(&mut b, w << 16);
    put32(&mut b, h << 16);
    full(b"tkhd", u8::from(v1), 3, &b) // enabled, in movie
}

fn mdhd(timescale: u32, duration: u64) -> Vec<u8> {
    let v1 = duration > u64::from(u32::MAX);
    let mut b = Vec::new();
    if v1 {
        put64(&mut b, 0);
        put64(&mut b, 0);
        put32(&mut b, timescale);
        put64(&mut b, duration);
    } else {
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, timescale);
        put32(&mut b, duration as u32);
    }
    put16(&mut b, 0x55c4); // "und"
    put16(&mut b, 0);
    full(b"mdhd", u8::from(v1), 0, &b)
}

fn hdlr(kind: &[u8; 4], name: &str) -> Vec<u8> {
    let mut b = Vec::new();
    put32(&mut b, 0);
    b.extend_from_slice(kind);
    b.extend_from_slice(&[0; 12]);
    b.extend_from_slice(name.as_bytes());
    b.push(0);
    full(b"hdlr", 0, 0, &b)
}

fn dinf() -> Vec<u8> {
    let mut d = Vec::new();
    put32(&mut d, 1);
    d.extend(full(b"url ", 0, 1, &[])); // in this file
    boxed(b"dinf", &full(b"dref", 0, 0, &d))
}

fn avc1(w: u32, h: u32, avcc: &[u8]) -> Vec<u8> {
    let mut b = vec![0; 6];
    put16(&mut b, 1); // data reference index
    b.extend_from_slice(&[0; 16]);
    put16(&mut b, w as u16);
    put16(&mut b, h as u16);
    put32(&mut b, 0x0048_0000); // 72 dpi
    put32(&mut b, 0x0048_0000);
    put32(&mut b, 0);
    put16(&mut b, 1); // frame count
    let mut name = [0u8; 32];
    let n = b"OpenH264";
    name[0] = n.len() as u8;
    name[1..=n.len()].copy_from_slice(n);
    b.extend_from_slice(&name);
    put16(&mut b, 0x18);
    put16(&mut b, 0xffff);
    b.extend(boxed(b"avcC", avcc));
    // BT.709, limited range (what `rec::yuv` produces).
    let mut c = b"nclx".to_vec();
    put16(&mut c, 1);
    put16(&mut c, 1);
    put16(&mut c, 1);
    c.push(0);
    b.extend(boxed(b"colr", &c));
    boxed(b"avc1", &b)
}

fn opus_entry(pre_skip: u16) -> Vec<u8> {
    let mut b = vec![0; 6];
    put16(&mut b, 1);
    b.extend_from_slice(&[0; 8]);
    put16(&mut b, 2); // channels
    put16(&mut b, 16); // sample size
    put32(&mut b, 0);
    put32(&mut b, AUDIO_TIMESCALE << 16);
    b.extend(boxed(b"dOps", &dops_body(2, pre_skip)));
    boxed(b"Opus", &b)
}

fn stsd(entry: Vec<u8>) -> Vec<u8> {
    let mut b = Vec::new();
    put32(&mut b, 1);
    b.extend(entry);
    full(b"stsd", 0, 0, &b)
}

/// `stts` from sample durations (run-length coded).
fn stts(durations: impl Iterator<Item = u32>) -> Vec<u8> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for d in durations {
        match runs.last_mut() {
            Some((n, v)) if *v == d => *n += 1,
            _ => runs.push((1, d)),
        }
    }
    let mut b = Vec::new();
    put32(&mut b, runs.len() as u32);
    for (n, d) in runs {
        put32(&mut b, n);
        put32(&mut b, d);
    }
    full(b"stts", 0, 0, &b)
}

/// One sample per chunk: `stsc`, `stsz` and `stco`/`co64`.
fn sample_tables(sizes: &[u32], offsets: &[u64], shift: u64) -> Vec<u8> {
    let mut c = Vec::new();
    put32(&mut c, 1);
    put32(&mut c, 1);
    put32(&mut c, 1);
    put32(&mut c, 1);
    let mut z = Vec::new();
    put32(&mut z, 0);
    put32(&mut z, sizes.len() as u32);
    for &s in sizes {
        put32(&mut z, s);
    }
    let big = offsets.iter().any(|&o| o + shift > u64::from(u32::MAX));
    let mut o = Vec::new();
    put32(&mut o, offsets.len() as u32);
    for &off in offsets {
        if big {
            put64(&mut o, off + shift);
        } else {
            put32(&mut o, (off + shift) as u32);
        }
    }
    concat(&[full(b"stsc", 0, 0, &c), full(b"stsz", 0, 0, &z), full(if big { b"co64" } else { b"stco" }, 0, 0, &o)])
}

fn elst(segment: u64, media_time: u64) -> Vec<u8> {
    let v1 = segment > u64::from(u32::MAX) || media_time > i32::MAX as u64;
    let mut b = Vec::new();
    put32(&mut b, 1);
    if v1 {
        put64(&mut b, segment);
        put64(&mut b, media_time);
    } else {
        put32(&mut b, segment as u32);
        put32(&mut b, media_time as u32);
    }
    put16(&mut b, 1);
    put16(&mut b, 0);
    boxed(b"edts", &full(b"elst", u8::from(v1), 0, &b))
}

fn trak(tkhd: Vec<u8>, edts: Option<Vec<u8>>, mdhd: Vec<u8>, hdlr: Vec<u8>, mhd: Vec<u8>, stbl: Vec<u8>) -> Vec<u8> {
    let minf = boxed(b"minf", &concat(&[mhd, dinf(), boxed(b"stbl", &stbl)]));
    let mdia = boxed(b"mdia", &concat(&[mdhd, hdlr, minf]));
    boxed(b"trak", &concat(&[tkhd, edts.unwrap_or_default(), mdia]))
}

// ---------------------------------------------------------------------------
// The writer
// ---------------------------------------------------------------------------

/// The picture: its size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoParams {
    pub w: u32,
    pub h: u32,
}

/// The sound: Opus, stereo, 48 kHz, with the encoder's pre-skip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioParams {
    pub pre_skip: u16,
}

#[derive(Default)]
struct Samples {
    sizes: Vec<u32>,
    offsets: Vec<u64>,
}

impl Samples {
    fn push(&mut self, size: u32, at: u64) {
        self.sizes.push(size);
        self.offsets.push(at);
    }
}

struct Video {
    p: VideoParams,
    /// SPS and PPS (from the first IDR).
    config: Option<(Vec<u8>, Vec<u8>)>,
    s: Samples,
    /// Decode times (90 kHz), strictly increasing.
    times: Vec<u64>,
    /// 1-based numbers of the sync samples.
    sync: Vec<u32>,
}

struct Audio {
    p: AudioParams,
    s: Samples,
    /// The real length of the sound in samples (without pre-skip and the
    /// padding of the last packet).
    length: u64,
}

/// How [`Mp4Writer::finish`] went.
#[derive(Debug, PartialEq, Eq)]
pub enum Finished {
    /// The file plays, with `moov` in front.
    Done,
    /// The file plays, but not everything is in it or `moov` stayed at the
    /// end: why.
    Partly(String),
}

/// Streams samples into an MP4 (see the module docs).
pub struct Mp4Writer<F: Media> {
    f: F,
    /// Where `mdat` starts (its header).
    mdat_at: u64,
    /// The end of what was written (and kept).
    pos: u64,
    video: Option<Video>,
    audio: Option<Audio>,
    /// A write failed: nothing more is written; the file ends at `pos`.
    failed: Option<String>,
}

impl<F: Media> Mp4Writer<F> {
    /// Start the file: `ftyp` and the `mdat` header.
    pub fn new(mut f: F, video: Option<VideoParams>, audio: Option<AudioParams>) -> io::Result<Mp4Writer<F>> {
        let mut ftyp = b"isom".to_vec();
        put32(&mut ftyp, 0x200);
        for b in [b"isom", b"iso6", b"mp41"] {
            ftyp.extend_from_slice(b);
        }
        let ftyp = boxed(b"ftyp", &ftyp);
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&ftyp)?;
        let mdat_at = ftyp.len() as u64;
        let mut head = Vec::new();
        put32(&mut head, 1);
        head.extend_from_slice(b"mdat");
        put64(&mut head, MDAT_HEAD);
        f.write_all(&head)?;
        Ok(Mp4Writer {
            f,
            mdat_at,
            pos: mdat_at + MDAT_HEAD,
            video: video.map(|p| Video { p, config: None, s: Samples::default(), times: Vec::new(), sync: Vec::new() }),
            audio: audio.map(|p| Audio { p, s: Samples::default(), length: 0 }),
            failed: None,
        })
    }

    /// Whether a write failed (the file then ends with what came before).
    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    fn append(&mut self, data: &[u8]) -> io::Result<Option<u64>> {
        if self.failed.is_some() {
            return Ok(None);
        }
        let at = self.pos;
        let r = self.f.seek(SeekFrom::Start(at)).and_then(|_| self.f.write_all(data));
        match r {
            Ok(()) => {
                self.pos += data.len() as u64;
                Ok(Some(at))
            }
            Err(e) => {
                self.failed = Some(e.to_string());
                Err(e)
            }
        }
    }

    /// One picture (AVCC) at decode time `time` (90 kHz; the first should
    /// be 0, later ones increase). The first picture must be an IDR with
    /// its parameter sets (see [`Mp4Writer::set_avc_config`]); pictures
    /// before the configuration are dropped.
    pub fn write_video(&mut self, avcc: &[u8], time: u64, sync: bool) -> io::Result<()> {
        let Some(v) = self.video.as_ref() else { return Ok(()) };
        if v.config.is_none() || avcc.is_empty() {
            return Ok(());
        }
        let time = match v.times.last() {
            Some(&t) if time <= t => t + 1,
            _ => time,
        };
        if let Some(at) = self.append(avcc)? {
            let v = self.video.as_mut().expect("video track");
            v.s.push(avcc.len() as u32, at);
            v.times.push(time);
            if sync {
                v.sync.push(v.times.len() as u32);
            }
        }
        Ok(())
    }

    /// The SPS and PPS of the stream (the first ones count).
    pub fn set_avc_config(&mut self, sps: &[u8], pps: &[u8]) {
        if let Some(v) = self.video.as_mut()
            && v.config.is_none()
            && avcc_body(sps, pps).is_some()
        {
            v.config = Some((sps.to_vec(), pps.to_vec()));
        }
    }

    /// One Opus packet ([`OPUS_FRAME`] samples).
    pub fn write_audio(&mut self, packet: &[u8]) -> io::Result<()> {
        if self.audio.is_none() || packet.is_empty() {
            return Ok(());
        }
        if let Some(at) = self.append(packet)? {
            self.audio.as_mut().expect("audio track").s.push(packet.len() as u32, at);
        }
        Ok(())
    }

    /// The sound's real length in samples (what the packets hold without
    /// pre-skip and the last packet's padding).
    pub fn set_audio_length(&mut self, samples: u64) {
        if let Some(a) = self.audio.as_mut() {
            a.length = samples;
        }
    }

    /// Pictures and sound packets written.
    pub fn counts(&self) -> (usize, usize) {
        (self.video.as_ref().map_or(0, |v| v.times.len()), self.audio.as_ref().map_or(0, |a| a.s.sizes.len()))
    }

    /// The `moov` box with chunk offsets moved by `shift`; `video_end` is
    /// when the last picture ends (90 kHz).
    fn moov(&self, video_end: u64, shift: u64) -> Vec<u8> {
        let mut traks = Vec::new();
        let mut movie = 0u64;
        if let Some(v) = &self.video
            && let Some((sps, pps)) = &v.config
            && !v.times.is_empty()
        {
            let last = *v.times.last().expect("a picture");
            let end = video_end.max(last + 1);
            let durs = v.times.windows(2).map(|w| (w[1] - w[0]) as u32).chain(std::iter::once((end - last) as u32));
            let mut stss = Vec::new();
            put32(&mut stss, v.sync.len() as u32);
            for &n in &v.sync {
                put32(&mut stss, n);
            }
            let avcc = avcc_body(sps, pps).expect("checked in set_avc_config");
            let stbl = concat(&[
                stsd(avc1(v.p.w, v.p.h, &avcc)),
                stts(durs),
                full(b"stss", 0, 0, &stss),
                sample_tables(&v.s.sizes, &v.s.offsets, shift),
            ]);
            let dur = scale(end, VIDEO_TIMESCALE, MOVIE_TIMESCALE);
            movie = movie.max(dur);
            let mut vmhd = Vec::new();
            put16(&mut vmhd, 0);
            vmhd.extend_from_slice(&[0; 6]);
            traks.push(trak(
                tkhd(VIDEO_ID, dur, false, v.p.w, v.p.h),
                None,
                mdhd(VIDEO_TIMESCALE, end),
                hdlr(b"vide", "VideoHandler"),
                full(b"vmhd", 0, 1, &vmhd),
                stbl,
            ));
        }
        if let Some(a) = &self.audio
            && !a.s.sizes.is_empty()
        {
            let n = a.s.sizes.len() as u64;
            let media = n * u64::from(OPUS_FRAME);
            let pre = u64::from(a.p.pre_skip);
            let real = a.length.min(media.saturating_sub(pre));
            let seg = scale(real, AUDIO_TIMESCALE, MOVIE_TIMESCALE);
            movie = movie.max(seg);
            let stbl = concat(&[
                stsd(opus_entry(a.p.pre_skip)),
                stts(std::iter::repeat_n(OPUS_FRAME, a.s.sizes.len())),
                sample_tables(&a.s.sizes, &a.s.offsets, shift),
            ]);
            traks.push(trak(
                tkhd(AUDIO_ID, seg, true, 0, 0),
                Some(elst(seg, pre)),
                mdhd(AUDIO_TIMESCALE, media),
                hdlr(b"soun", "SoundHandler"),
                full(b"smhd", 0, 0, &[0; 4]),
                stbl,
            ));
        }
        let mut body = mvhd(movie, AUDIO_ID + 1);
        for t in traks {
            body.extend(t);
        }
        boxed(b"moov", &body)
    }

    /// The `moov` that goes in front: its offsets are moved by its own
    /// size, which depends on whether they still fit 32 bits once moved.
    /// A fixed point: `co64` never shrinks, so the size only grows, and it
    /// grows at most once per track.
    fn front_moov(&self, video_end: u64) -> Vec<u8> {
        let mut shift = 0;
        loop {
            let m = self.moov(video_end, shift);
            if m.len() as u64 == shift {
                return m;
            }
            shift = m.len() as u64;
        }
    }

    /// Finish the file (see the module docs): `video_end` is when the
    /// last picture ends (90 kHz; at least one tick after its start).
    /// `Err` means the file does not play (nothing usable was written, or
    /// moving `moov` failed half-way); [`Finished::Partly`] that it plays
    /// but a write failed before (it holds what came before) or `moov`
    /// stayed at the end.
    pub fn finish(mut self, video_end: u64) -> io::Result<(F, Finished)> {
        let earlier = self.failed.take();
        let playable = self.video.as_ref().is_some_and(|v| v.config.is_some() && !v.times.is_empty())
            || self.audio.as_ref().is_some_and(|a| !a.s.sizes.is_empty());
        if !playable {
            return Err(io::Error::other(earlier.unwrap_or_else(|| "nothing was recorded".into())));
        }
        // mdat ends where the last kept sample ends.
        let end = self.pos;
        self.f.set_len(end)?;
        self.f.seek(SeekFrom::Start(self.mdat_at + 8))?;
        self.f.write_all(&(end - self.mdat_at).to_be_bytes())?;
        // moov at the end: the file plays from here on.
        let tail = self.moov(video_end, 0);
        self.f.seek(SeekFrom::Start(end))?;
        self.f.write_all(&tail)?;
        self.f.flush()?;
        // Then in front.
        let front = self.front_moov(video_end);
        let shift = front.len() as u64;
        let note = |why: String| match &earlier {
            Some(e) => format!("{e}; {why}"),
            None => why,
        };
        if let Some(free) = self.f.free_space()
            && free < shift + FREE_MARGIN
        {
            eprintln!("rustshot: {free} bytes free, the MP4 index ({shift} bytes) stays at the end of the file");
            return Ok((self.f, Finished::Partly(note(format!("the index stays at the end of the file: only {free} bytes free")))));
        }
        // Growing is the last step that can fail gracefully (see the
        // module docs): from the first block moved on, a crash or a failed
        // write leaves the file unplayable.
        if let Err(e) = self.f.set_len(end + shift) {
            // The tail moov is intact (growing failed before any move).
            let _ = self.f.set_len(end + tail.len() as u64);
            return Ok((self.f, Finished::Partly(note(format!("the index stays at the end of the file: {e}")))));
        }
        // From here a failure leaves the file broken.
        #[allow(clippy::useless_vec)] // 1 MiB outside tests: on the heap
        let mut buf = vec![0u8; MOVE_BLOCK];
        let mut hi = end;
        while hi > self.mdat_at {
            let n = usize::try_from(hi - self.mdat_at).map_or(MOVE_BLOCK, |r| r.min(MOVE_BLOCK));
            let lo = hi - n as u64;
            self.f.seek(SeekFrom::Start(lo))?;
            self.f.read_exact(&mut buf[..n])?;
            self.f.seek(SeekFrom::Start(lo + shift))?;
            self.f.write_all(&buf[..n])?;
            hi = lo;
        }
        self.f.seek(SeekFrom::Start(self.mdat_at))?;
        self.f.write_all(&front)?;
        self.f.flush()?;
        let done = match earlier {
            Some(e) => Finished::Partly(e),
            None => Finished::Done,
        };
        Ok((self.f, done))
    }
}

#[cfg(test)]
pub(crate) mod parse;

#[cfg(test)]
mod tests;
