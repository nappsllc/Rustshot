use super::*;
use crate::rec::mp4::parse::{boxes, Bx as PBx};
use crate::rec::mp4::{AudioParams, Mp4Writer, VideoParams};
use std::io::Cursor;

const SPS: [u8; 8] = [0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16];
const PPS: [u8; 4] = [0x68, 0xce, 0x3c, 0x80];

/// A recording from our muxer: `secs` of 30 fps video with a key frame
/// every second (each sample's bytes say its number), and Opus sound in
/// 20 ms packets with a pre-skip of 312.
fn recording(secs: u32) -> Vec<u8> {
    let mut w = Mp4Writer::new(Cursor::new(Vec::new()), Some(VideoParams { w: 64, h: 64 }), Some(AudioParams { pre_skip: 312 })).unwrap();
    w.set_avc_config(&SPS, &PPS);
    let frames = secs * 30;
    let packets = secs * 50;
    let (mut f, mut p) = (0u32, 0u32);
    // Interleaved as a recorder would: by time.
    while f < frames || p < packets {
        let tv = f as u64 * 3000; // 90 kHz
        let ta = p as u64 * 1800; // 20 ms in 90 kHz
        if f < frames && (tv <= ta || p >= packets) {
            let mut s = vec![0, 0, 0, 8, if f % 30 == 0 { 0x65 } else { 0x41 }, 0xEE];
            s.extend_from_slice(&f.to_be_bytes());
            w.write_video(&s, tv, f % 30 == 0).unwrap();
            f += 1;
        } else {
            let mut pk = vec![0xA0u8];
            pk.extend_from_slice(&p.to_be_bytes());
            w.write_audio(&pk).unwrap();
            p += 1;
        }
    }
    w.set_audio_length(packets as u64 * 960 - 312);
    let (c, _) = w.finish(frames as u64 * 3000).unwrap();
    c.into_inner()
}

fn moov(file: &[u8]) -> PBx {
    boxes(file, 0, file.len()).into_iter().find(|b| &b.kind == b"moov").expect("moov")
}

fn traks(m: &PBx) -> Vec<&PBx> {
    m.kids.iter().filter(|b| &b.kind == b"trak").collect()
}

/// The bytes of sample `i` of track `t` (its stsz and co64/stco).
fn sample(file: &[u8], t: &PBx, i: usize) -> Vec<u8> {
    let stbl = t.get(&[b"mdia", b"minf", b"stbl"]);
    let sizes = stbl.get(&[b"stsz"]).table(8, 1);
    let offs = if stbl.has(b"co64") { stbl.get(&[b"co64"]).table(4, 1) } else { stbl.get(&[b"stco"]).table(4, 1) };
    let (at, n) = (offs[i][0] as usize, sizes[i][0] as usize);
    file[at..at + n].to_vec()
}

fn trim_bytes(src: &[u8], a: f64, b: f64) -> Result<(Vec<u8>, Trimmed)> {
    let mut out = Vec::new();
    let t = trim(&mut Cursor::new(src.to_vec()), &mut out, Duration::from_secs_f64(a), Duration::from_secs_f64(b))?;
    Ok((out, t))
}

#[test]
fn a_trim_copies_from_the_key_frame_and_edits_the_start() {
    let src = recording(5);
    let (out, t) = trim_bytes(&src, 1.5, 3.2).unwrap();
    assert_eq!(t.bytes as usize, out.len());
    let m = moov(&out);
    let tr = traks(&m);
    assert_eq!(tr.len(), 2);
    // Video: frames 30 (the key frame at 1 s) up to the key frame at 4 s.
    let v = tr[0];
    let stbl = v.get(&[b"mdia", b"minf", b"stbl"]);
    assert_eq!(stbl.get(&[b"stsz"]).u32(8), 90);
    assert_eq!(stbl.get(&[b"stss"]).table(4, 1), vec![vec![1], vec![31], vec![61]]);
    for i in [0usize, 1, 45, 89] {
        let s = sample(&out, v, i);
        assert_eq!(u32::from_be_bytes(s[6..10].try_into().unwrap()), 30 + i as u32, "video sample {i}");
    }
    assert_eq!((t.video_samples, t.lead_in), (90, 15), "half a second decoded before the start");
    // Its edit list: 0.5 s into the kept media (90 kHz), shown for 1.7 s.
    let elst = v.get(&[b"edts", b"elst"]);
    assert_eq!(elst.u32(4), 1);
    assert_eq!((elst.u32(8), elst.u32(12)), (1700, 45_000));
    assert_eq!(m.get(&[b"mvhd"]).u32(16), 1700, "the movie lasts 1.7 s");
    // Sound: from 80 ms before the start, past its pre-skip.
    let a = tr[1];
    let ms = 312 + 72_000; // the start in the media (48 kHz)
    let first = (ms - 3840) / 960; // packet 71
    let s0 = sample(&out, a, 0);
    assert_eq!(u32::from_be_bytes(s0[1..5].try_into().unwrap()), first as u32);
    let ae = a.get(&[b"edts", b"elst"]);
    assert_eq!((ae.u32(8), ae.u32(12)), (1700, ms as u32 - first as u32 * 960));
    let n_audio = a.get(&[b"mdia", b"minf", b"stbl", b"stsz"]).u32(8) as usize;
    assert_eq!(first + n_audio, (312 + 153_600usize).div_ceil(960), "up to the packet that holds the end");
    assert_eq!(duration(&mut Cursor::new(out.clone())).unwrap(), Duration::from_millis(1700));
}

#[test]
fn trimming_everything_keeps_every_sample() {
    let src = recording(2);
    let (out, t) = trim_bytes(&src, 0.0, 2.0).unwrap();
    assert_eq!((t.video_samples, t.lead_in), (60, 0));
    let m = moov(&out);
    for (k, tr) in traks(&m).into_iter().enumerate() {
        let n = tr.get(&[b"mdia", b"minf", b"stbl", b"stsz"]).u32(8);
        assert_eq!(n, if k == 0 { 60 } else { 100 });
    }
    // Trimmed again: the same.
    let (again, _) = trim_bytes(&out, 0.0, 2.0).unwrap();
    assert_eq!(moov(&again).get(&[b"mvhd"]).u32(16), 2000);
}

#[test]
fn key_times_and_duration() {
    let src = recording(3);
    let k = key_times(&mut Cursor::new(src.clone())).unwrap();
    assert_eq!(k, vec![Duration::ZERO, Duration::from_secs(1), Duration::from_secs(2)]);
    let d = duration(&mut Cursor::new(src)).unwrap();
    assert!((d.as_secs_f64() - 3.0).abs() < 0.05, "{d:?}");
}

#[test]
fn what_cannot_be_trimmed_says_so() {
    let src = recording(2);
    assert!(trim_bytes(&src, 1.0, 1.0).is_err(), "empty");
    assert!(trim_bytes(b"not an mp4 at all", 0.0, 1.0).is_err());
    // A fragmented file (an mvex in moov).
    let mut frag = Vec::new();
    frag.extend(boxed(b"ftyp", b"isom\0\0\0\0isom"));
    let mut mv = boxed(b"mvhd", &[0u8; 100]);
    mv.extend(boxed(b"mvex", &[]));
    frag.extend(boxed(b"moov", &mv));
    let e = trim_bytes(&frag, 0.0, 1.0).unwrap_err();
    assert!(format!("{e:#}").contains("fragmented"), "{e:#}");
}

/// B-frames: the start key frame is the last one *shown* at or before the
/// start; the end keeps the GOP that shows the end.
#[test]
fn plan_with_composition_offsets_and_empty_edits() {
    // Decode order I P B B | I P B B | I ...: 8 frames, 1000 ticks each.
    let n = 12;
    let s = Samples {
        sizes: vec![10; n],
        offsets: (0..n as u64).map(|i| i * 10).collect(),
        durations: vec![1000; n],
        ctts: Some(((0..n).map(|i| if i % 4 == 0 { 1000 } else if i % 4 == 1 { 3000 } else { 0 }).collect(), 0)),
        sync: Some((0..n).map(|i| i % 4 == 0).collect()),
        desc: vec![1; n],
    };
    // Media starts at 1000 (the first shown frame); timescale 1000 = ms.
    let edit = Edit { empty: 0, media_time: 1000 };
    let k = plan(&s, edit, 1000, 1000, true, Duration::from_millis(4500), Duration::from_millis(6200), Duration::ZERO).unwrap();
    // 4.5 s is media 5500: the key frame at decode 4 shows at 5000.
    assert_eq!((k.first, k.last), (4, 8), "the GOP from 4 shows up to 8 s: it holds the end");
    assert_eq!(k.media_time, 5500 - 4000);
    assert_eq!(k.shown, 1700);
    // A clip that starts with 2 s of nothing (an empty edit).
    let edit = Edit { empty: 2000, media_time: 0 };
    let a = Samples { sizes: vec![4; 100], offsets: (0..100).collect(), durations: vec![480; 100], ctts: None, sync: None, desc: vec![1; 100] };
    let k = plan(&a, edit, 48_000, 1000, false, Duration::from_millis(500), Duration::from_millis(3000), Duration::ZERO).unwrap();
    assert_eq!(k.empty, 1500, "the rest of the nothing first");
    assert_eq!((k.first, k.media_time, k.shown), (0, 0, 1000));
    // Starting after the nothing.
    let k = plan(&a, edit, 48_000, 1000, false, Duration::from_millis(2500), Duration::from_millis(3000), Duration::ZERO).unwrap();
    assert_eq!(k.empty, 0);
    assert_eq!((k.first, k.media_time), (50, 0), "media 0.5 s is packet 50");
    assert!(plan(&a, edit, 48_000, 1000, false, Duration::from_millis(500), Duration::from_millis(1500), Duration::ZERO).is_none(), "only nothing");
}

#[test]
fn edit_lists_and_runs() {
    let mut b = Vec::new();
    put32(&mut b, 2);
    for (d, mt) in [(500u32, -1i32), (9000, 312)] {
        put32(&mut b, d);
        put32(&mut b, mt as u32);
        put32(&mut b, 0x10000);
    }
    let mut body = vec![0, 0, 0, 0];
    body.extend(b);
    assert_eq!(read_edit(&body).unwrap(), Edit { empty: 500, media_time: 312 });
    assert_eq!(runs([3, 3, 3, 5, 3].into_iter()), vec![(3, 3), (1, 5), (1, 3)]);
    let wide = elst(&Keep { first: 0, last: 1, empty: 0, media_time: 1 << 40, shown: 10 });
    assert_eq!(wide[16], 1, "a 64-bit edit list when the values need it");
}
