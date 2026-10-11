use super::*;
use std::mem::{offset_of, size_of};

#[test]
fn coremedia_structs_have_the_c_layout() {
    assert_eq!(size_of::<CMTime>(), 24);
    assert_eq!(offset_of!(CMTime, timescale), 8);
    assert_eq!(offset_of!(CMTime, flags), 12);
    assert_eq!(offset_of!(CMTime, epoch), 16);
    assert_eq!(size_of::<Asbd>(), 40);
    assert_eq!(offset_of!(Asbd, format_id), 8);
    assert_eq!(offset_of!(Asbd, bits_per_channel), 32);
    assert_eq!(size_of::<AudioBuffer>(), 16);
    assert_eq!(offset_of!(AudioBufferList, buffers), 8);
    assert_eq!(size_of::<AudioBufferList>(), 8 + 16 * AudioBufferList::MAX);
}

#[test]
fn four_character_codes() {
    assert_eq!(BGRA, 0x4247_5241);
    assert_eq!(LPCM, 0x6C70_636D);
    assert_eq!(AAC, 0x6161_6320);
}

#[test]
fn cmtime_seconds_only_when_valid_and_finite() {
    assert_eq!(CMTime::new(3, 2).seconds(), Some(1.5));
    assert_eq!(CMTime { flags: 0, ..CMTime::new(1, 1) }.seconds(), None, "invalid");
    assert_eq!(CMTime { flags: CMTime::VALID | 4, ..CMTime::new(1, 1) }.seconds(), None, "positive infinity");
    assert_eq!(CMTime::new(1, 0).seconds(), None);
    assert_eq!(CMTime { flags: CMTime::VALID | CMTime::ROUNDED, ..CMTime::new(3, 2) }.seconds(), Some(1.5), "rounded is still usable");
    assert_eq!(CMTime { flags: CMTime::VALID | 2, ..CMTime::new(1, 4) }.seconds(), Some(0.25));
    assert_eq!(CMTime { flags: CMTime::VALID | 8, ..CMTime::new(1, 1) }.seconds(), None, "negative infinity");
    assert_eq!(CMTime { flags: CMTime::VALID | 16, ..CMTime::new(1, 1) }.seconds(), None, "indefinite");
}

#[test]
fn host_time_maps_to_instants() {
    let now = Instant::now();
    let at = instant_at(100.0, 100.25, now);
    assert_eq!(now.duration_since(at), Duration::from_millis(250));
    assert_eq!(instant_at(101.0, 100.0, now), now, "the future is now");
    assert_eq!(instant_at(f64::NAN, 100.0, now), now);
}

#[test]
fn f32_stereo_format_is_packed_interleaved() {
    let a = Asbd::f32_stereo(48_000);
    assert_eq!(a.format_flags, FLAG_FLOAT | FLAG_PACKED);
    assert_eq!((a.bytes_per_frame, a.channels_per_frame, a.bits_per_channel), (8, 2, 32));
    let p = pcm_of(&a).unwrap();
    assert_eq!(p, Pcm { float: true, bytes: 4, big: false, planar: false, channels: 2 });
}

#[test]
fn stereo_layout_has_the_stereo_tag() {
    let b = stereo_layout();
    assert_eq!(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]), 0x0065_0002);
    assert!(b[4..].iter().all(|&x| x == 0));
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[test]
fn pcm_converts_to_interleaved_stereo() {
    let mut out = Vec::new();
    // Non-interleaved float stereo (what ScreenCaptureKit sends).
    let planar = Pcm { float: true, bytes: 4, big: false, planar: true, channels: 2 };
    let (l, r) = (f32s(&[0.1, 0.2]), f32s(&[-0.1, -0.2]));
    assert_eq!(to_stereo(&planar, &[&l, &r], &mut out), 2);
    assert_eq!(out, vec![0.1, -0.1, 0.2, -0.2]);
    // Interleaved s16 mono: doubled.
    out.clear();
    let mono = Pcm { float: false, bytes: 2, big: false, planar: false, channels: 1 };
    let s: Vec<u8> = [16384i16, -32768].iter().flat_map(|x| x.to_le_bytes()).collect();
    assert_eq!(to_stereo(&mono, &[&s], &mut out), 2);
    assert_eq!(out, vec![0.5, 0.5, -1.0, -1.0]);
    // Four interleaved channels: the first two.
    out.clear();
    let quad = Pcm { float: true, bytes: 4, big: false, planar: false, channels: 4 };
    let d = f32s(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    assert_eq!(to_stereo(&quad, &[&d], &mut out), 2, "a partial frame is left out");
    assert_eq!(out, vec![1.0, 2.0, 5.0, 6.0]);
    // Big-endian s32.
    out.clear();
    let be = Pcm { float: false, bytes: 4, big: true, planar: false, channels: 2 };
    let d: Vec<u8> = [i32::MIN, 0].iter().flat_map(|x| x.to_be_bytes()).collect();
    assert_eq!(to_stereo(&be, &[&d], &mut out), 1);
    assert_eq!(out, vec![-1.0, 0.0]);
    // A planar stereo list missing its second buffer gives nothing.
    out.clear();
    assert_eq!(to_stereo(&planar, &[&l], &mut out), 0);
}

#[test]
fn only_linear_pcm_of_known_sizes_is_read() {
    let mut a = Asbd::f32_stereo(44_100);
    a.format_id = AAC;
    assert!(pcm_of(&a).is_err());
    let mut a = Asbd::f32_stereo(44_100);
    a.format_flags = FLAG_PACKED; // integer
    a.bits_per_channel = 24;
    assert!(pcm_of(&a).is_err());
    a.bits_per_channel = 16;
    a.format_flags |= FLAG_NON_INTERLEAVED | FLAG_BIG_ENDIAN;
    assert_eq!(pcm_of(&a).unwrap(), Pcm { float: false, bytes: 2, big: true, planar: true, channels: 2 });
}

#[test]
fn packets_follow_on_within_two_ms() {
    assert!(!follows(None, 1.0));
    assert!(follows(Some(1.0), 1.0015));
    assert!(!follows(Some(1.0), 1.003));
    assert!(!follows(Some(1.0), 0.99));
}

fn r(x: f64, y: f64, w: f64, h: f64) -> RectF {
    RectF { x, y, w, h }
}

#[test]
fn the_display_with_the_area_centre_is_recorded() {
    // Main 1440x900 pt, a second one to its right, a third above.
    let d = [r(0.0, 0.0, 1440.0, 900.0), r(1440.0, 0.0, 1920.0, 1080.0), r(0.0, -1080.0, 1920.0, 1080.0)];
    assert_eq!(pick_display((100, 100, 200, 200), &d), Some(0));
    assert_eq!(pick_display((1300, 100, 400, 200), &d), Some(1), "centre at x 1500");
    assert_eq!(pick_display((10, -500, 50, 50), &d), Some(2));
    // Centre in a gap: the one it overlaps most.
    let gap = [r(0.0, 0.0, 100.0, 100.0), r(200.0, 0.0, 100.0, 100.0)];
    assert_eq!(pick_display((90, 0, 160, 50), &gap), Some(1));
    assert_eq!(pick_display((500, 500, 10, 10), &gap), None);
}

#[test]
fn the_source_rect_is_in_display_points_and_the_frame_in_pixels() {
    let disp = r(1440.0, -100.0, 1920.0, 1080.0);
    let g = stream_geom((1500, 0, 801, 451), disp, 2.0, true).unwrap();
    assert_eq!(g.src, r(60.0, 100.0, 801.0, 451.0));
    assert_eq!((g.w, g.h), (1602, 902), "Retina pixels, even");
    // GIF: full size, odd sizes kept.
    let g = stream_geom((1500, 0, 801, 451), disp, 1.0, false).unwrap();
    assert_eq!((g.w, g.h), (801, 451));
    // Clipped to the display.
    let g = stream_geom((3300, 900, 200, 200), disp, 1.0, false).unwrap();
    assert_eq!(g.src, r(1860.0, 1000.0, 60.0, 80.0));
    // A 5K display at 2x for MP4: scaled to fit 4096x2304.
    let g = stream_geom((0, 0, 2560, 1440), r(0.0, 0.0, 2560.0, 1440.0), 2.0, true).unwrap();
    assert_eq!((g.w, g.h), (4096, 2304));
    assert!(stream_geom((0, 0, 10, 10), disp, 2.0, true).is_none());
    // An unknown scale counts as 1.
    let g = stream_geom((1440, -100, 100, 100), disp, f64::NAN, false).unwrap();
    assert_eq!((g.w, g.h), (100, 100));
}

#[test]
fn our_windows_are_filtered_out_but_the_bubble() {
    let me = 42;
    let wins = [(10, 7), (11, me), (12, me), (13, 9)];
    // Our application is listed: exclude it, except the bubble (12).
    assert_eq!(plan_filter(&[7, me, 9], &wins, me, &[12]), FilterPlan::App { app: 1, except: vec![2] });
    assert_eq!(plan_filter(&[me], &wins, me, &[]), FilterPlan::App { app: 0, except: vec![] });
    // A window of another app with the bubble's id is not ours.
    assert_eq!(plan_filter(&[me], &[(12, 7)], me, &[12]), FilterPlan::App { app: 0, except: vec![] });
    // Not listed: exclude our windows one by one, the bubble kept.
    assert_eq!(plan_filter(&[7, 9], &wins, me, &[12]), FilterPlan::Windows(vec![1]));
    assert_eq!(plan_filter(&[], &wins, me, &[]), FilterPlan::Windows(vec![1, 2]));
}

#[test]
fn video_times_strictly_increase() {
    let ms = Duration::from_millis;
    assert_eq!(video_pts(ms(0), None), 0);
    assert_eq!(video_pts(ms(33), Some(0)), 33_000);
    assert_eq!(video_pts(ms(33), Some(33_000)), 33_001, "a repeated time (pause) moves on by one");
    assert_eq!(video_pts(ms(10), Some(33_001)), 33_002, "never back");
}

#[test]
fn sound_times_are_sample_indices() {
    assert_eq!(audio_pts(Duration::ZERO), 0);
    assert_eq!(audio_pts(Duration::from_secs(1)), 48_000);
    assert_eq!(audio_pts(Duration::from_micros(20_833)), 1000);
}

#[test]
fn rows_are_copied_without_their_padding() {
    // 2x2 BGRA with 12-byte rows (4 padding bytes).
    let src = [1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16];
    let mut out = vec![99];
    assert!(copy_rows(&src, 12, 2, 2, &mut out));
    assert_eq!(out, (1..=16).collect::<Vec<u8>>());
    assert!(!copy_rows(&src[..19], 12, 2, 2, &mut out), "too short");
    assert!(!copy_rows(&src, 4, 2, 2, &mut out), "stride below a row");
}

#[test]
fn a_frame_fills_a_padded_buffer_and_black_elsewhere() {
    // 1x1 picture into a 2x2 buffer with 12-byte rows.
    let mut dst = vec![7u8; 24];
    fill_rows(&mut dst, 12, (2, 2), &[1, 2, 3, 4], (1, 1));
    assert_eq!(&dst[..8], &[1, 2, 3, 4, 0, 0, 0, 255]);
    assert_eq!(&dst[8..12], &[7, 7, 7, 7], "padding untouched");
    assert_eq!(&dst[12..20], &[0, 0, 0, 255, 0, 0, 0, 255]);
    // Same size: copied as is.
    let mut dst = vec![0u8; 8];
    fill_rows(&mut dst, 8, (2, 1), &[1, 2, 3, 4, 5, 6, 7, 8], (2, 1));
    assert_eq!(dst, vec![1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
#[cfg(not(target_os = "macos"))]
fn recording_is_unavailable_off_macos() {
    assert!(!available());
}

#[test]
fn only_complete_frames_are_new() {
    assert!(frame_is_new(Some(FRAME_COMPLETE)));
    assert!(frame_is_new(None), "no status: the image decides");
    for idle in 1..=5 {
        assert!(!frame_is_new(Some(idle)), "status {idle}");
    }
}

#[test]
fn the_sound_queue_is_bounded_and_drops_the_oldest() {
    let ms = Duration::from_millis;
    let mut q = SoundQueue::new(4);
    q.push(&[0.0; 4], ms(0)); // 2 frames
    q.push(&[1.0; 4], ms(1));
    assert_eq!((q.frames(), q.dropped), (4, 0));
    q.push(&[2.0; 2], ms(2));
    assert_eq!((q.frames(), q.dropped), (3, 2), "the oldest packet went");
    assert_eq!(q.pop().unwrap(), (vec![1.0; 4], ms(1)));
    assert_eq!(q.frames(), 1);
    assert_eq!(q.pop().unwrap().1, ms(2));
    assert!(q.is_empty() && q.pop().is_none());
    // One packet larger than the cap is kept (the queue never empties itself).
    q.push(&[0.0; 20], ms(3));
    assert_eq!(q.frames(), 10);
    q.push(&[0.0; 1], ms(4));
    assert_eq!(q.frames(), 10, "half a frame is nothing");
}
