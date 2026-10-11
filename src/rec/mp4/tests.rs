use super::parse::{boxes, Bx};
use super::*;

const SPS: [u8; 8] = [0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16];
const PPS: [u8; 4] = [0x68, 0xce, 0x3c, 0x80];

/// An Annex-B access unit: SPS, PPS (with an IDR) and a slice of `n` bytes.
fn annexb(idr: bool, n: usize, fill: u8) -> Vec<u8> {
    let mut v = Vec::new();
    if idr {
        v.extend_from_slice(&[0, 0, 0, 1]);
        v.extend_from_slice(&SPS);
        v.extend_from_slice(&[0, 0, 0, 1]);
        v.extend_from_slice(&PPS);
    }
    v.extend_from_slice(&[0, 0, 1]);
    v.push(if idr { 0x65 } else { 0x41 });
    v.extend(std::iter::repeat_n(fill, n));
    v
}

#[test]
fn annexb_becomes_length_prefixed_and_parameter_sets_come_out() {
    // A 4-byte start code, an AUD, a 3-byte one, trailing zeros.
    let mut s = vec![0, 0, 0, 1, 0x09, 0xf0, 0, 0, 0, 1];
    s.extend_from_slice(&SPS);
    s.extend_from_slice(&[0, 0, 1]);
    s.extend_from_slice(&PPS);
    s.extend_from_slice(&[0, 0, 0, 0, 1, 0x65, 1, 2, 3, 0, 0, 1, 0x65, 9, 0]);
    let nals = nal_units(&s);
    assert_eq!(nals.len(), 5);
    assert_eq!(nals[1], &SPS);
    assert_eq!(nals[2], &PPS, "the zero before the 4-byte start code is not the PPS's");
    assert_eq!(nals[4], &[0x65, 9], "a trailing zero is dropped");
    let au = access_unit(&s);
    assert_eq!(au.sps.as_deref(), Some(&SPS[..]));
    assert_eq!(au.pps.as_deref(), Some(&PPS[..]));
    assert!(au.idr);
    assert_eq!(au.avcc, [0, 0, 0, 4, 0x65, 1, 2, 3, 0, 0, 0, 2, 0x65, 9]);
    let p = access_unit(&annexb(false, 3, 7));
    assert!(!p.idr && p.sps.is_none() && p.pps.is_none());
    assert_eq!(p.avcc, [0, 0, 0, 4, 0x41, 7, 7, 7]);
    assert!(nal_units(&[0, 0, 0]).is_empty());
    assert!(nal_units(&[1, 2, 3]).is_empty(), "no start code");
}

#[test]
fn avcc_and_dops_bodies() {
    let b = avcc_body(&SPS, &PPS).unwrap();
    assert_eq!(&b[..6], &[1, 0x42, 0xc0, 0x1f, 0xff, 0xe1]);
    assert_eq!(u16::from_be_bytes([b[6], b[7]]), 8);
    assert_eq!(&b[8..16], &SPS);
    assert_eq!(b[16], 1);
    assert_eq!(u16::from_be_bytes([b[17], b[18]]), 4);
    assert_eq!(&b[19..], &PPS);
    assert!(avcc_body(&[0x67, 0x42], &PPS).is_none(), "too short for the profile");
    assert!(avcc_body(&SPS, &[]).is_none());
    assert_eq!(dops_body(2, 312), [0, 2, 0x01, 0x38, 0, 0, 0xbb, 0x80, 0, 0, 0]);
}

/// A 2 s file: 30 fps picture with an IDR every second (one frame late,
/// so `stts` has two runs), and 2.01 s of Opus packets (fake bytes).
/// The file, the pictures and sound packets in it, and how it finished.
type Synthetic = (Vec<u8>, Vec<Vec<u8>>, Vec<Vec<u8>>, Finished);

fn synthetic() -> Synthetic {
    let mut w = Mp4Writer::new(io::Cursor::new(Vec::new()), Some(VideoParams { w: 640, h: 360 }), Some(AudioParams { pre_skip: 312 })).unwrap();
    let mut vs = Vec::new();
    let mut as_ = Vec::new();
    let mut a = 0;
    for i in 0..60u64 {
        let au = access_unit(&annexb(i % 30 == 0, 100 + i as usize, i as u8));
        if let (Some(s), Some(p)) = (&au.sps, &au.pps) {
            w.set_avc_config(s, p);
        }
        let t = if i == 59 { 59 * 3000 + 1500 } else { i * 3000 };
        w.write_video(&au.avcc, t, au.idr).unwrap();
        vs.push(au.avcc);
        // Sound in step: packets up to this picture's time.
        while u64::from(OPUS_FRAME) * a < (t + 3000) * 48 / 90 + 312 {
            let pk = vec![0xa0 | (a as u8 & 0xf); 20 + (a as usize % 7)];
            w.write_audio(&pk).unwrap();
            as_.push(pk);
            a += 1;
        }
    }
    w.set_audio_length(96_480); // 2.01 s
    let (c, done) = w.finish(60 * 3000).unwrap();
    (c.into_inner(), vs, as_, done)
}

fn track<'a>(moov: &'a Bx, handler: &[u8; 4]) -> &'a Bx {
    moov.kids.iter().filter(|k| &k.kind == b"trak").find(|t| &t.get(&[b"mdia", b"hdlr"]).body[8..12] == handler).expect("track")
}

#[test]
fn the_box_tree_parses_back_with_moov_first() {
    let (file, vs, as_, done) = synthetic();
    assert_eq!(done, Finished::Done);
    let top = boxes(&file, 0, file.len());
    let kinds: Vec<&[u8; 4]> = top.iter().map(|b| &b.kind).collect();
    assert_eq!(kinds, [b"ftyp", b"moov", b"mdat"], "faststart: moov before mdat");
    assert_eq!(top.iter().map(|b| b.size).sum::<usize>(), file.len(), "the boxes cover the file");
    let ftyp = &top[0];
    assert_eq!(&ftyp.body[..4], b"isom");
    assert_eq!(&ftyp.body[8..], b"isomiso6mp41");
    let mdat = &top[2];
    assert_eq!(&file[mdat.at..mdat.at + 4], &[0, 0, 0, 1], "64-bit mdat size");
    let payload: usize = vs.iter().chain(&as_).map(|s| s.len()).sum();
    assert_eq!(mdat.size, payload + 16);
    // ffmpeg's check sees the index too.
    assert!(crate::rec::linux::ffmpeg::moov_in(io::Cursor::new(&file), file.len() as u64));

    let moov = &top[1];
    let mvhd = moov.get(&[b"mvhd"]);
    assert_eq!(mvhd.u32(12), MOVIE_TIMESCALE);
    assert_eq!(mvhd.u32(16), 2010, "the longer track (sound, 2.01 s)");
    assert_eq!(mvhd.u32(96), 3, "next track id");

    // The picture.
    let v = track(moov, b"vide");
    let tkhd = v.get(&[b"tkhd"]);
    assert_eq!(tkhd.u32(0) & 0xff_ffff, 3, "enabled, in movie");
    assert_eq!(tkhd.u32(12), 1);
    assert_eq!(tkhd.u32(20), 2000);
    assert_eq!((tkhd.u32(76), tkhd.u32(80)), (640 << 16, 360 << 16));
    let mdhd = v.get(&[b"mdia", b"mdhd"]);
    assert_eq!((mdhd.u32(12), mdhd.u32(16)), (VIDEO_TIMESCALE, 180_000));
    let stbl = v.get(&[b"mdia", b"minf", b"stbl"]);
    let avc1 = stbl.get(&[b"stsd", b"avc1"]);
    assert_eq!((avc1.u16(24), avc1.u16(26)), (640, 360));
    let avcc = avc1.get(&[b"avcC"]);
    assert_eq!(avcc.body, avcc_body(&SPS, &PPS).unwrap());
    assert_eq!(avcc.body[4] & 3, 3, "4-byte lengths");
    let colr = avc1.get(&[b"colr"]);
    assert_eq!(&colr.body, &[b'n', b'c', b'l', b'x', 0, 1, 0, 1, 0, 1, 0]);
    let stts = stbl.get(&[b"stts"]).table(4, 2);
    assert_eq!(stts, [vec![58, 3000], vec![1, 4500], vec![1, 1500]], "frame 59 came half a frame late");
    assert_eq!(stbl.get(&[b"stss"]).table(4, 1), [vec![1], vec![31]]);
    let stsz = stbl.get(&[b"stsz"]);
    assert_eq!(stsz.u32(4), 0);
    let sizes: Vec<u64> = stsz.table(8, 1).into_iter().map(|e| e[0]).collect();
    assert_eq!(sizes, vs.iter().map(|s| s.len() as u64).collect::<Vec<_>>());
    assert_eq!(stbl.get(&[b"stsc"]).table(4, 3), [vec![1, 1, 1]]);
    let stco = stbl.get(&[b"stco"]).table(4, 1);
    for (o, s) in stco.iter().zip(&vs) {
        assert_eq!(&file[o[0] as usize..o[0] as usize + s.len()], &s[..], "the offset points at the sample");
    }
    assert!(!v.has(b"edts"));

    // The sound.
    let a = track(moov, b"soun");
    let tkhd = a.get(&[b"tkhd"]);
    assert_eq!(tkhd.u32(12), 2);
    assert_eq!(tkhd.u16(36), 0x0100, "volume");
    let elst = a.get(&[b"edts", b"elst"]);
    assert_eq!(elst.u32(4), 1);
    assert_eq!((elst.u32(8), elst.u32(12)), (2010, 312), "pre-skip skipped, ends at the real length");
    assert_eq!(elst.u16(16), 1);
    let mdhd = a.get(&[b"mdia", b"mdhd"]);
    assert_eq!((mdhd.u32(12), mdhd.u32(16)), (AUDIO_TIMESCALE, as_.len() as u32 * OPUS_FRAME));
    let stbl = a.get(&[b"mdia", b"minf", b"stbl"]);
    let opus = stbl.get(&[b"stsd", b"Opus"]);
    assert_eq!((opus.u16(16), opus.u16(18)), (2, 16));
    assert_eq!(opus.u32(24), 48_000 << 16);
    let dops = opus.get(&[b"dOps"]);
    assert_eq!(dops.u8(0), 0, "version");
    assert_eq!(dops.u8(1), 2, "channels");
    assert_eq!(dops.u16(2), 312, "pre-skip");
    assert_eq!(dops.u32(4), 48_000);
    assert_eq!(dops.u8(10), 0, "mapping family");
    assert_eq!(stbl.get(&[b"stts"]).table(4, 2), [vec![as_.len() as u64, 960]]);
    let stco = stbl.get(&[b"stco"]).table(4, 1);
    assert_eq!(stco.len(), as_.len());
    for (o, s) in stco.iter().zip(&as_) {
        assert_eq!(&file[o[0] as usize..o[0] as usize + s.len()], &s[..]);
    }
    assert!(a.get(&[b"mdia", b"minf"]).has(b"smhd"));
}

#[test]
fn pictures_before_the_parameter_sets_are_dropped_and_times_increase() {
    let mut w = Mp4Writer::new(io::Cursor::new(Vec::new()), Some(VideoParams { w: 2, h: 2 }), None).unwrap();
    w.write_video(&[0, 0, 0, 1, 0x41], 0, false).unwrap();
    assert_eq!(w.counts(), (0, 0));
    w.set_avc_config(&SPS, &PPS);
    w.write_video(&[0, 0, 0, 1, 0x65], 0, true).unwrap();
    w.write_video(&[0, 0, 0, 1, 0x41], 0, false).unwrap(); // same time: one tick later
    assert_eq!(w.counts(), (2, 0));
    let (c, _) = w.finish(0).unwrap();
    let file = c.into_inner();
    let top = boxes(&file, 0, file.len());
    let v = track(&top[1], b"vide");
    assert_eq!(v.get(&[b"mdia", b"minf", b"stbl", b"stts"]).table(4, 2), [vec![2, 1]]);
}

/// A file that refuses to grow past `cap` bytes, or to be written past it.
struct Capped(io::Cursor<Vec<u8>>, u64);

impl Read for Capped {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.0.read(b)
    }
}
impl Write for Capped {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.0.position() + b.len() as u64 > self.1 {
            return Err(io::Error::other("disk full"));
        }
        self.0.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Capped {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.0.seek(p)
    }
}
impl Media for Capped {
    fn set_len(&mut self, n: u64) -> io::Result<()> {
        if n > self.1 {
            return Err(io::Error::other("disk full"));
        }
        self.0.set_len(n)
    }
}

fn write_frames(w: &mut Mp4Writer<Capped>, n: u64) {
    for i in 0..n {
        let au = access_unit(&annexb(i == 0, 1000, 1));
        if let (Some(s), Some(p)) = (&au.sps, &au.pps) {
            w.set_avc_config(s, p);
        }
        let _ = w.write_video(&au.avcc, i * 3000, au.idr);
    }
}

#[test]
fn a_failed_write_keeps_what_came_before_and_says_so() {
    // Room for about 10 pictures and both indexes.
    let mut w = Mp4Writer::new(Capped(io::Cursor::new(Vec::new()), 14_000), Some(VideoParams { w: 64, h: 64 }), None).unwrap();
    write_frames(&mut w, 30);
    let ok = w.counts().0;
    assert!((5..30).contains(&ok), "{ok}");
    assert!(w.failed().is_some());
    let (c, done) = w.finish(30 * 3000).unwrap();
    assert!(matches!(&done, Finished::Partly(e) if e.contains("disk full")), "{done:?}");
    let file = c.0.into_inner();
    let top = boxes(&file, 0, file.len());
    let v = track(top.iter().find(|b| &b.kind == b"moov").unwrap(), b"vide");
    let stsz = v.get(&[b"mdia", b"minf", b"stbl", b"stsz"]);
    assert_eq!(stsz.u32(8) as usize, ok, "only the pictures written are indexed");
}

#[test]
fn nothing_recorded_is_an_error() {
    let w = Mp4Writer::new(io::Cursor::new(Vec::new()), Some(VideoParams { w: 2, h: 2 }), None).unwrap();
    assert!(w.finish(0).is_err());
}

#[test]
fn big_offsets_use_co64_and_long_durations_version_1() {
    let t = sample_tables(&[10, 20], &[8, u64::from(u32::MAX) - 4], 0);
    let b = boxes(&t, 0, t.len());
    assert_eq!(b[2].kind, *b"stco", "still fits");
    let t = sample_tables(&[10, 20], &[8, u64::from(u32::MAX) - 4], 16);
    let b = boxes(&t, 0, t.len());
    assert_eq!(b[2].kind, *b"co64");
    assert_eq!(b[2].table(4, 1), [vec![24], vec![u64::from(u32::MAX) + 12]]);
    let m = mdhd(VIDEO_TIMESCALE, 1 << 33);
    let b = &boxes(&m, 0, m.len())[0];
    assert_eq!(b.u8(0), 1);
    assert_eq!(b.u64(24), 1 << 33);
}
