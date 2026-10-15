use super::*;
use crate::config::Config;

fn t0() -> Instant {
    Instant::now()
}

fn secs(n: f64) -> Duration {
    Duration::from_secs_f64(n)
}

#[test]
fn opts_follow_the_config_and_the_format() {
    let cfg = Config::default();
    let o = RecOpts::from_config(&cfg);
    assert_eq!((o.format, o.fps(), o.fps_choices()), (RecFormat::Mp4, 30, &[30, 60][..]));
    assert!(o.system && !o.mic && o.mic_dev.is_empty() && !o.camera);
    let cfg = Config { rec_format: "gif".into(), rec_gif_fps: 10, rec_fps: 60, rec_mic: true, ..Config::default() };
    let mut o = RecOpts::from_config(&cfg);
    assert_eq!((o.format, o.fps(), o.fps_choices()), (RecFormat::Gif, 10, &[10, 15, 24][..]));
    assert_eq!((o.gif_max, o.countdown), (30, true));
    assert_eq!(o.max_len(), Some(Duration::from_secs(30)), "a GIF stops by itself");
    o.set_fps(60); // not a GIF rate: ignored
    assert_eq!(o.fps(), 10);
    o.set_fps(15);
    assert_eq!(o.fps(), 15);
    o.set_fps(24);
    assert_eq!(o.fps(), 24, "GIF at 24 fps");
    o.format = RecFormat::Mp4;
    assert_eq!(o.fps(), 60, "each format keeps its own rate");
    assert_eq!(o.max_len(), None, "an MP4 runs until stopped");
}

#[test]
fn spec_from_opts() {
    let cfg = Config { save_path: "C:/shots".into(), rec_quality: "high".into(), ..Config::default() };
    let mut o = RecOpts::from_config(&cfg);
    o.mic = true;
    o.mic_dev = "{dev}".into();
    let now = (2026, 10, 10, 12, 0, 0);
    let s = o.spec(&cfg, (10, 20, 300, 200), now);
    assert_eq!((s.format, s.fps, s.area), (RecFormat::Mp4, 30, (10, 20, 300, 200)));
    assert_eq!(s.quality, crate::rec::Quality::High);
    assert!(s.system_audio);
    assert_eq!(s.mic.as_deref(), Some("{dev}"));
    assert_eq!(s.out.extension().unwrap(), "mp4");
    assert_eq!(s.out, crate::export::auto_save_path_ext(&cfg, now, "mp4"));
    o.format = RecFormat::Gif;
    let g = o.spec(&cfg, (10, 20, 300, 200), now);
    assert_eq!((g.fps, g.system_audio, g.mic.is_none()), (15, false, true), "a GIF has no sound");
    assert_eq!(g.out.extension().unwrap(), "gif");
}

#[test]
fn machine_countdown_record_pause_stop() {
    let t = t0();
    let mut m = Machine::new();
    assert_eq!(m.phase(), Phase::Idle);
    assert_eq!(m.start(true, t), None);
    assert_eq!(m.phase(), Phase::Countdown);
    assert_eq!(m.countdown(t), Some(3));
    assert_eq!(m.countdown(t + secs(0.5)), Some(3));
    assert_eq!(m.countdown(t + secs(1.2)), Some(2));
    assert_eq!(m.countdown(t + secs(2.9)), Some(1));
    assert_eq!(m.tick(t + secs(2.9)), None);
    assert_eq!(m.start(true, t), None, "already running");
    assert_eq!(m.tick(t + secs(3.0)), Some(Effect::Go));
    assert_eq!((m.phase(), m.countdown(t + secs(3.0))), (Phase::Recording, None));
    assert_eq!(m.tick(t + secs(3.5)), None, "once");
    assert_eq!(m.elapsed(t + secs(5.0)), secs(2.0));
    // Pause ⇄ resume; paused time is not counted.
    assert_eq!(m.toggle_pause(t + secs(5.0)), Some(Effect::Pause));
    assert_eq!(m.phase(), Phase::Paused);
    assert_eq!(m.elapsed(t + secs(9.0)), secs(2.0), "frozen while paused");
    assert_eq!(m.toggle_pause(t + secs(9.0)), Some(Effect::Resume));
    assert_eq!(m.elapsed(t + secs(10.0)), secs(3.0));
    // Stop: once; the timer freezes; the end makes it idle.
    assert_eq!(m.stop(t + secs(11.0)), Some(Effect::Stop));
    assert_eq!(m.phase(), Phase::Stopping);
    assert_eq!(m.stop(t + secs(11.5)), None);
    assert_eq!(m.toggle_pause(t + secs(11.5)), None);
    assert_eq!(m.elapsed(t + secs(20.0)), secs(4.0));
    m.finish();
    assert_eq!(m.phase(), Phase::Idle);
    // And it can run again.
    assert_eq!(m.start(false, t + secs(30.0)), Some(Effect::Go));
    assert_eq!(m.elapsed(t + secs(31.0)), secs(1.0));
}

#[test]
fn machine_stop_while_paused_and_during_the_countdown() {
    let t = t0();
    let mut m = Machine::new();
    assert_eq!(m.start(false, t), Some(Effect::Go));
    m.toggle_pause(t + secs(1.0));
    assert_eq!(m.stop(t + secs(5.0)), Some(Effect::Stop));
    assert_eq!(m.elapsed(t + secs(9.0)), secs(1.0));
    let mut m = Machine::new();
    m.start(true, t);
    assert_eq!(m.toggle_pause(t + secs(1.0)), None, "nothing to pause yet");
    assert_eq!(m.stop(t + secs(1.0)), Some(Effect::Cancel));
    assert_eq!(m.tick(t + secs(4.0)), None, "a cancelled countdown never goes");
    assert_eq!(m.phase(), Phase::Stopping);
}

#[test]
fn times() {
    assert_eq!(fmt_time(secs(0.4)), "0:00");
    assert_eq!(fmt_time(secs(7.9)), "0:07");
    assert_eq!(fmt_time(secs(754.0)), "12:34");
    assert_eq!(fmt_time(secs(3723.0)), "1:02:03");
}

const MON: IRect = (0, 0, 1920, 1080);
const BAR: (i32, i32) = (180, 44);

#[test]
fn bar_goes_where_there_is_most_room() {
    // Plenty everywhere, most below: below, centred.
    assert_eq!(place_bar((800, 200, 320, 200), &[MON], BAR, 8, 8), (870, 408));
    // At the bottom edge: above.
    assert_eq!(place_bar((800, 900, 320, 180), &[MON], BAR, 8, 8), (870, 848));
    // Tall area at the left, touching top and bottom: right side, bottom-aligned.
    assert_eq!(place_bar((0, 0, 600, 1080), &[MON], BAR, 8, 8), (608, 1080 - 8 - 44));
    // At the right, full height: left side.
    assert_eq!(place_bar((1300, 0, 620, 1080), &[MON], BAR, 8, 8), (1300 - 8 - 180, 1028));
    // Touching the top, wide: below.
    assert_eq!(place_bar((100, 0, 1600, 300), &[MON], BAR, 8, 8), (810, 308));
    // A small area at the top left: more room to its right than below it.
    assert_eq!(place_bar((100, 0, 400, 300), &[MON], BAR, 8, 8), (508, 256));
    // Too little room anywhere outside: inside at the bottom centre.
    assert_eq!(place_bar(MON, &[MON], BAR, 8, 8), (870, 1080 - 8 - 44));
    // Near the left edge of a narrow monitor: below, kept on screen.
    assert_eq!(place_bar((0, 100, 60, 60), &[(0, 0, 400, 1080)], BAR, 8, 8), (8, 168));
}

#[test]
fn x11_bar_stays_out_of_the_recording_or_shrinks_to_a_corner_pill() {
    // Beside the area: as everywhere.
    assert_eq!(place_bar_away((800, 200, 320, 200), &[MON], BAR, 8, 8), Some((870, 408)));
    // Full screen on the only monitor: nowhere; the pill goes top-right inside.
    assert_eq!(place_bar_away(MON, &[MON], BAR, 8, 8), None);
    assert_eq!(corner_pill(MON, (90, 44), 8), (1920 - 8 - 90, 8));
    // Full screen with a monitor that is not beside it (below-left, apart
    // from the probe points): the bottom centre of that monitor.
    let other: IRect = (-1280, 1080, 1280, 720);
    assert_eq!(place_bar_away(MON, &[MON, other], BAR, 8, 8), Some((-1280 + (1280 - 180) / 2, 1080 + 720 - 8 - 44)));
    // A monitor overlapping the area (mirrored) does not count.
    assert_eq!(place_bar_away(MON, &[MON, (0, 0, 1280, 720)], BAR, 8, 8), None);
}

#[test]
fn x11_countdown_goes_before_the_first_frame() {
    let t = t0();
    let mut m = Machine::new();
    m.start(true, t);
    assert_eq!(m.countdown_shown(t + secs(2.9)), Some(1));
    // In the last HIDE_BEFORE_GO it is already down (a compositor repaints by Go).
    let late = t + COUNTDOWN - HIDE_BEFORE_GO;
    assert_eq!(m.countdown(late), Some(1));
    assert_eq!(m.countdown_shown(late), None);
    assert!(HIDE_BEFORE_GO >= Duration::from_millis(50));
    assert_eq!(m.tick(t + COUNTDOWN), Some(Effect::Go));
    assert_eq!(m.countdown_shown(t + COUNTDOWN), None);
}

#[test]
fn bar_on_multi_monitor_setups() {
    let right: IRect = (1920, 0, 1280, 1024);
    let mons = [MON, right];
    // A full-screen recording of the left monitor: the bar goes on the right one.
    let (x, y) = place_bar(MON, &mons, BAR, 8, 8);
    assert_eq!(x, 1928);
    assert!(y >= 8 && y + 44 <= 1024 - 8, "{y}");
    // Of the right monitor: on the left one.
    let (x, _) = place_bar(right, &mons, BAR, 8, 8);
    assert_eq!(x, 1920 - 8 - 180);
    // An area on the right monitor near its bottom: above, on that monitor.
    let (x, y) = place_bar((2000, 900, 400, 120), &mons, BAR, 8, 8);
    assert_eq!((x, y), (2110, 848));
    // A monitor to the left at negative coordinates.
    let left: IRect = (-1280, 0, 1280, 1024);
    let (x, _) = place_bar(MON, &[left, MON], BAR, 8, 8);
    assert_eq!(x, -8 - 180);
}

/// A 100 % monitor and a 200 % one to its right.
fn mixed() -> Geom {
    Geom { area: MON, monitors: vec![(MON, 1.0), ((1920, 0, 2560, 1440), 2.0)], scale: 1.0, mult: 1.0 }
}

/// The bar's panel at scale `s` (`wide`: the timer past an hour).
fn panel(wide: bool) -> impl Fn(f32) -> (i32, i32) {
    move |s| (((if wide { 220.0 } else { 180.0 }) * s) as i32, (44.0 * s) as i32)
}

#[test]
fn geom_scale_per_monitor() {
    let g = mixed();
    assert_eq!((g.scale_at(10, 10), g.scale_at(2000, 10), g.scale_at(-5, 0)), (1.0, 2.0, 1.0));
    assert_eq!(g.area_scale(), 1.0);
    assert_eq!(g.border_monitors(), vec![(MON, 2), ((1920, 0, 2560, 1440), 4)]);
    assert_eq!(Geom { area: (2000, 100, 400, 300), ..mixed() }.area_scale(), 2.0);
}

/// Mixed DPI: a full-screen recording of the 100 % monitor puts the bar on
/// the 200 % one, drawn (and sized, and placed) at 200 %.
#[test]
fn bar_takes_the_dpi_of_the_monitor_it_lands_on() {
    let spot = spot_bar(&mixed(), None, panel(false));
    assert_eq!(spot, BarSpot { scale: 2.0, size: (360, 88), pos: (1936, 1080 - 88) });
    // An area on the 200 % monitor: 200 % from the start, below the area.
    let g = Geom { area: (2000, 100, 2000, 300), ..mixed() };
    let spot = spot_bar(&g, None, panel(false));
    assert_eq!(spot, BarSpot { scale: 2.0, size: (360, 88), pos: (3000 - 180, 416) });
    // On the 100 % monitor: 100 %.
    let g = Geom { area: (100, 100, 400, 300), ..mixed() };
    assert_eq!(spot_bar(&g, None, panel(false)).scale, 1.0);
}

/// The bar stays put while its size does not change, and is placed again
/// (re-centred) when it grows (the timer passed an hour).
#[test]
fn bar_is_placed_again_when_its_width_changes() {
    let g = Geom { area: (2000, 100, 2000, 300), ..mixed() };
    let first = spot_bar(&g, None, panel(false));
    assert_eq!(spot_bar(&g, Some(first), panel(false)), first, "same size: kept");
    let wide = spot_bar(&g, Some(first), panel(true));
    assert_eq!(wide, BarSpot { scale: 2.0, size: (440, 88), pos: (3000 - 220, 416) });
    // Near a monitor's edge the wider bar is placed again, on screen.
    let g = Geom { area: (1700, 100, 200, 200), monitors: vec![(MON, 1.0)], scale: 1.0, mult: 1.0 };
    let narrow = spot_bar(&g, None, panel(false));
    let wide = spot_bar(&g, Some(narrow), panel(true));
    assert!(wide.pos.0 + wide.size.0 <= 1920 - 8, "{wide:?}");
    assert_ne!(wide.pos, narrow.pos);
}

#[test]
fn layer_alpha_from_black_and_white() {
    let px = render_layer(4, 1, |f| {
        f.fill_rect(0, 0, 1, 1, C4::rgb(255, 0, 0));
        f.fill_rect(1, 0, 1, 1, C4::new(0, 0, 255, 128));
    });
    assert_eq!(&px[0..4], &[0, 0, 255, 255], "opaque red as BGRA");
    let a = px[7];
    assert!((126..=130).contains(&a), "half alpha: {a}");
    assert!(px[4] <= a && px[4] >= a - 2, "premultiplied blue");
    assert_eq!(&px[8..16], &[0; 8], "untouched pixels are transparent");
    let over = layer_over(&px, [255, 255, 255]);
    assert_eq!(&over[0..4], &[255, 0, 0, 255]);
}

struct Tone(Vec<f32>);

impl AudioSource for Tone {
    fn rate(&self) -> u32 {
        48_000
    }
    fn channels(&self) -> u16 {
        2
    }
    fn read(&mut self, out: &mut Vec<f32>) -> usize {
        out.extend_from_slice(&self.0);
        self.0.len()
    }
}

#[test]
fn muted_source_reads_silence() {
    let muted = Arc::new(AtomicBool::new(false));
    let mut m = Muted { inner: Tone(vec![0.5; 8]), muted: muted.clone(), level: None };
    let mut out = vec![1.0];
    assert_eq!(m.read(&mut out), 8);
    assert_eq!(out[1], 0.5);
    muted.store(true, Ordering::SeqCst);
    let mut out = vec![1.0];
    assert_eq!(m.read(&mut out), 8, "same timing");
    assert_eq!(out[0], 1.0, "what was there is kept");
    assert!(out[1..].iter().all(|&v| v == 0.0));
    assert_eq!((m.rate(), m.channels()), (48_000, 2));
}

/// The bar offers Mute only once the microphone's source opened (on the
/// worker, after the bar is up), and muting reaches that source.
#[test]
fn mute_appears_once_the_microphone_opened() {
    let t = t0();
    let mut r = Run::fake(false, false, t);
    assert_eq!(r.view(t).mic, None);
    r.toggle_mute();
    assert!(!r.muted(), "nothing to mute yet");
    let mut m = r.mic.opened(Tone(vec![0.5; 4]));
    assert_eq!(r.view(t).mic, Some(false));
    r.toggle_mute();
    assert_eq!(r.view(t).mic, Some(true));
    let mut out = Vec::new();
    m.read(&mut out);
    assert!(out.iter().all(|&v| v == 0.0), "the source is muted");
}

#[test]
fn run_effects_and_end() {
    let t = t0();
    let mut r = Run::fake(true, true, t);
    assert_eq!(r.phase(), Phase::Countdown);
    assert!(r.log.is_empty());
    // Stop during the countdown cancels.
    r.stop(t);
    assert_eq!(r.log, [Effect::Cancel]);
    r.fake_end = Some(End::Cancelled);
    assert_eq!(r.tick(t), Some(End::Cancelled));
    assert_eq!(r.phase(), Phase::Idle);
    // Without the countdown: Go at once; pause, mute, stop, the file.
    let mut r = Run::fake(false, true, t);
    assert_eq!(r.log, [Effect::Go]);
    r.toggle_pause(t + secs(1.0));
    assert!(r.view(t + secs(2.0)).paused);
    r.toggle_mute();
    assert_eq!(r.view(t).mic, Some(true));
    r.toggle_pause(t + secs(2.0));
    r.stop(t + secs(3.0));
    assert_eq!(r.log, [Effect::Go, Effect::Pause, Effect::Resume, Effect::Stop]);
    assert!(r.view(t + secs(3.0)).stopping);
    assert_eq!(r.tick(t + secs(3.1)), None, "waits for the file");
    let saved = End::Saved { path: "x.mp4".into(), warnings: vec!["microphone not recorded: gone".into()] };
    r.fake_end = Some(saved.clone());
    assert_eq!(r.tick(t + secs(4.0)), Some(saved));
    // No microphone: nothing to mute.
    let mut r = Run::fake(false, false, t);
    r.toggle_mute();
    assert_eq!(r.view(t).mic, None);
    assert!(!r.muted());
}

#[test]
fn countdown_ticks_into_go() {
    let t = t0();
    let mut r = Run::fake(true, false, t);
    assert_eq!(r.tick(t + secs(1.0)), None);
    assert!(r.log.is_empty());
    assert_eq!(r.tick(t + secs(3.0)), None);
    assert_eq!(r.log, [Effect::Go]);
    assert_eq!(r.phase(), Phase::Recording);
}

/// Preview PNGs of the control bar, the countdown and the tray while
/// recording (dark and light) under `RUSTSHOT_PREVIEW_DIR`.
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_recording_previews() {
    use crate::editor::style::Ui;
    use crate::pixbuf::PixBuf;
    use crate::theme::{DARK, LIGHT};
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let save = |name: &str, w: usize, h: usize, bgra: &[u8], bg: [u8; 3]| {
        let img = PixBuf::from_raw(w as u32, h as u32, layer_over(bgra, bg)).unwrap();
        std::fs::write(dir.join(name), img.to_png().unwrap()).unwrap();
    };
    for (tn, th, bg) in [("dark", &DARK, [38u8, 44, 58]), ("light", &LIGHT, [214, 222, 232])] {
        for s in [1.0f32, 2.0] {
            let ui = Ui { th, s, font: Some(crate::fonts::ui_font()) };
            let views = [
                ("recording", bar::BarView { elapsed: secs(83.0), paused: false, mic: Some(false), cam: None, system: None, stopping: false, compact: false, asking: false, hidden: false }, Some(bar::BarAct::Stop)),
                ("paused", bar::BarView { elapsed: secs(754.0), paused: true, mic: Some(true), cam: Some(true), system: Some(true), stopping: false, compact: false, asking: false, hidden: false }, None),
                ("nomic", bar::BarView { elapsed: secs(5.0), paused: false, mic: None, cam: None, system: None, stopping: false, compact: false, asking: false, hidden: false }, Some(bar::BarAct::Pause)),
                ("stopping", bar::BarView { elapsed: secs(9.0), paused: false, mic: None, cam: None, system: None, stopping: true, compact: false, asking: false, hidden: false }, None),
                ("compact", bar::BarView { elapsed: secs(42.0), paused: false, mic: Some(false), cam: None, system: None, stopping: false, compact: true, asking: false, hidden: false }, None),
                ("asking", bar::BarView { elapsed: secs(0.0), paused: false, mic: None, cam: None, system: None, stopping: false, compact: false, asking: true, hidden: false }, None),
            ];
            for (vn, v, hover) in views {
                let l = bar::layout(&ui, &v);
                let (w, h) = (l.size.0 as usize, l.size.1 as usize);
                let px = render_layer(w, h, |f| bar::draw(f, &ui, &l, &v, hover, None));
                save(&format!("{tn}-bar-{vn}@{s}x.png"), w, h, &px, bg);
            }
            for (n, frac) in [(3, 1.0), (2, 0.6), (1, 0.25)] {
                let (w, h) = countdown::size(s);
                let px = render_layer(w as usize, h as usize, |f| countdown::draw(f, &ui, n, frac));
                save(&format!("{tn}-countdown-{n}@{s}x.png"), w as usize, h as usize, &px, bg);
            }
        }
        // The tray while recording: the icon (enlarged) and the menu.
        let ui = Ui { th, s: 2.0, font: Some(crate::fonts::ui_font()) };
        let rec = crate::tray::RecMenu { mic: Some(false), ..Default::default() };
        let items = crate::tray::menu(Some(false), Some(rec));
        let (w, h) = (360usize, 80 + 56 * items.len());
        let px = render_layer(w, h, |f| {
            let g = crate::tray::tray_icon_rgba(32, true);
            for y in 0..64usize {
                for x in 0..64usize {
                    let p = &g[((y / 2) * 32 + x / 2) * 4..][..4];
                    f.blend_px(20 + x as i32, 8 + y as i32, C4::new(p[0], p[1], p[2], p[3]));
                }
            }
            let r = crate::objects::FRect { x: 10.0, y: 76.0, w: 340.0, h: 56.0 * items.len() as f32 };
            crate::editor::style::surface(f, &ui, r, 6.0, &[0, 2], 1.0);
            for (i, it) in items.iter().enumerate() {
                let font = crate::fonts::ui_font();
                let y = r.y + 14.0 + 56.0 * i as f32;
                f.draw_text(font, 26.0, &it.label(), 34.0, y, th.text);
            }
        });
        save(&format!("{tn}-tray-recording.png"), w, h, &px, bg);
    }
}

/// A camera that stopped during the recording is said with the file.
#[test]
fn camera_failure_is_a_warning_of_the_saved_file() {
    let path = std::path::PathBuf::from("a.mp4");
    let saved = End::Saved { path: path.clone(), warnings: vec!["microphone not recorded: gone".into()] };
    assert_eq!(with_camera_error(saved.clone(), None), saved);
    assert_eq!(
        with_camera_error(saved, Some("The camera was disconnected or taken by another app".into())),
        End::Saved {
            path,
            warnings: vec![
                "microphone not recorded: gone".into(),
                "camera stopped: The camera was disconnected or taken by another app".into()
            ]
        }
    );
    assert_eq!(with_camera_error(End::Cancelled, Some("x".into())), End::Cancelled);
    assert_eq!(with_camera_error(End::Failed("f".into()), Some("x".into())), End::Failed("f".into()));
}

#[test]
fn x11_sessions_are_told_from_wayland_ones() {
    use std::ffi::OsStr;
    assert!(x11_session(None, None));
    assert!(x11_session(None, Some("x11")));
    assert!(x11_session(Some(OsStr::new("")), Some("tty")));
    assert!(!x11_session(Some(OsStr::new("wayland-0")), Some("x11")), "XWayland under a Wayland compositor");
    assert!(!x11_session(None, Some("wayland")));
    assert!(!x11_session(None, Some("Wayland")));
}

/// Wayland: the portal's dialog first (asking), then the countdown; Stop
/// while asking calls it off.
#[test]
fn asking_comes_before_the_countdown_and_stop_calls_it_off() {
    let t = t0();
    let mut m = Machine::new();
    m.ask();
    assert_eq!(m.phase(), Phase::Asking);
    assert_eq!(m.tick(t + secs(10.0)), None, "no countdown while asking");
    assert_eq!(m.countdown(t), None);
    assert_eq!(m.asked(true, t), None);
    assert_eq!(m.phase(), Phase::Countdown);
    assert_eq!(m.tick(t + secs(3.0)), Some(Effect::Go));
    // Without a countdown: straight to Go.
    let mut m = Machine::new();
    m.ask();
    assert_eq!(m.asked(false, t), Some(Effect::Go));
    assert_eq!(m.asked(false, t), None, "once");
    // Stop while asking.
    let mut m = Machine::new();
    m.ask();
    assert_eq!(m.stop(t), Some(Effect::Cancel));
    assert_eq!(m.phase(), Phase::Stopping);
    assert_eq!(m.asked(true, t), None, "too late");

    // The run: the bar says Asking with only Stop; the parts open, 3-2-1.
    let mut r = Run::fake_asking(true);
    assert!(r.view(t).asking);
    assert_eq!(r.tick(t), None);
    assert!(r.log.is_empty() && r.phase() == Phase::Asking);
    r.fake_ready = true;
    assert_eq!(r.tick(t), None);
    assert_eq!(r.phase(), Phase::Countdown);
    assert!(!r.view(t).asking);
    // Stopped while asking: cancelled, nothing recorded.
    let mut r = Run::fake_asking(false);
    r.stop(t);
    assert_eq!(r.log, [Effect::Cancel]);
    r.fake_end = Some(End::Cancelled);
    assert_eq!(r.tick(t), Some(End::Cancelled));
}

#[test]
fn misplaced_windows_get_a_hint() {
    let area = (100, 100, 800, 600);
    assert_eq!(placement_hint(area, None, None), None, "nothing known yet");
    assert_eq!(placement_hint(area, Some((100, 720, 200, 44)), Some((116, 444, 240, 240))), None, "bar below, bubble inside");
    assert_eq!(placement_hint(area, None, Some((0, 0, 240, 240))), Some(BUBBLE_OUTSIDE.get()));
    assert_eq!(placement_hint(area, None, Some((800, 500, 240, 240))), Some(BUBBLE_OUTSIDE.get()), "half out");
    assert_eq!(placement_hint(area, Some((400, 300, 200, 44)), None), Some(BAR_INSIDE.get()));
    assert_eq!(placement_hint(area, Some((0, 0, 100, 100)), None), None, "touching the corner is outside");
}

#[test]
fn macos_geometry_converts_to_points() {
    let g = Geom {
        area: (200, 100, 1281, 721),
        monitors: vec![((0, 0, 2880, 1800), 2.0), ((2880, 0, 3840, 2160), 2.0)],
        scale: 2.0,
        mult: 1.0,
    };
    let p = g.in_points(2.0);
    assert_eq!(p.area, (100, 50, 641, 361), "edges rounded");
    assert_eq!(p.monitors, vec![((0, 0, 1440, 900), 1.0), ((1440, 0, 1920, 1080), 1.0)]);
    assert_eq!(p.scale, 1.0);
    assert_eq!(g.in_points(1.0).area, g.area);
}

/// The bar, its pill and the countdown at the smallest and the largest
/// UI scale: sizes follow the scale, and the bar is placed with a gap
/// that follows it too.
#[test]
fn bar_pill_and_countdown_geometry_at_a_scale() {
    use crate::editor::style::Ui;
    let th = crate::theme::DARK;
    let view = bar::BarView { elapsed: secs(65.0), paused: false, mic: Some(false), cam: None, system: None, stopping: false, compact: false, asking: false, hidden: false };
    let pill = bar::BarView { compact: true, ..view.clone() };
    let at = |s: f32, v: &bar::BarView| bar::layout(&Ui { th: &th, s, font: None }, v);
    let (full1, pill1) = (at(1.0, &view), at(1.0, &pill));
    assert!(pill1.size.0 < full1.size.0);
    for s in [0.75f32, 2.0] {
        for (v, one) in [(&view, &full1), (&pill, &pill1)] {
            let l = at(s, v);
            let near = |a: f32, b: f32| (a - b * s).abs() <= 1.0;
            assert!(near(l.panel.w, one.panel.w) && near(l.panel.h, one.panel.h), "{s}: {:?} vs {:?}", l.panel, one.panel);
            assert!(near(l.size.0 as f32, one.size.0 as f32) && near(l.size.1 as f32, one.size.1 as f32), "{s}: {:?}", l.size);
            assert_eq!(l.buttons.len(), one.buttons.len());
            assert!((l.panel.x - bar::MARGIN * s).abs() < 0.01, "the panel sits MARGIN (scaled) inside");
        }
        assert_eq!(countdown::size(s), ((144.0 * s).ceil() as u32, (144.0 * s).ceil() as u32), "{s}");
        // Drawn: the round panel is inside the window, nothing at the corner.
        let ui = Ui { th: &th, s, font: Some(crate::fonts::ui_font()) };
        let (w, h) = countdown::size(s);
        let px = render_layer(w as usize, h as usize, |f| countdown::draw(f, &ui, 3, 1.0));
        assert_eq!(px[3], 0, "corner transparent at {s}");
        assert!(px.chunks(4).any(|p| p[3] > 200), "panel drawn at {s}");
        // Placed below the area, the gap (8 logical px) scaled.
        let g = Geom { area: (100, 100, 1700, 500), monitors: vec![(MON, s)], scale: s, mult: s };
        let l = at(s, &view);
        let spot = spot_bar(&g, None, |k| {
            let l = bar::layout(&Ui { th: &th, s: k, font: None }, &view);
            (l.size.0 as i32, l.size.1 as i32)
        });
        assert_eq!(spot.scale, s);
        assert_eq!(spot.pos.1, 600 + (8.0 * s).round() as i32, "below the area by the scaled gap");
        assert_eq!(spot.size, (l.size.0 as i32, l.size.1 as i32));
    }
}

/// The border's thickness follows the monitor's OS scale only, never the
/// UI size (as on macOS, where it is in points).
#[test]
fn border_ignores_the_ui_size() {
    for mult in [0.75f32, 1.0, 1.25, 2.0] {
        let g = Geom {
            area: MON,
            monitors: vec![(MON, 1.0 * mult), ((1920, 0, 2560, 1440), 1.25 * mult), ((4480, 0, 1920, 1080), 2.0 * mult)],
            scale: mult,
            mult,
        };
        let t: Vec<i32> = g.border_monitors().iter().map(|m| m.1).collect();
        assert_eq!(t, [2, 3, 4], "UI size {mult}");
    }
}

/// The countdown's arc: full at the start of each second, draining to
/// none by its end; the number 3, 2, 1.
#[test]
fn countdown_arc_drains_each_second() {
    let t = t0();
    let mut m = Machine::new();
    m.start(true, t);
    let at = |ms: u64| m.countdown_frac(t + Duration::from_millis(ms)).unwrap();
    assert_eq!(at(0), (3, 1.0));
    let (n, f) = at(250);
    assert!(n == 3 && (f - 0.75).abs() < 1e-3, "{n} {f}");
    let (n, f) = at(1500);
    assert!(n == 2 && (f - 0.5).abs() < 1e-3, "{n} {f}");
    let (n, f) = at(2999);
    assert!(n == 1 && f < 0.01, "{n} {f}");
    m.tick(t + COUNTDOWN);
    assert_eq!(m.countdown_frac(t + COUNTDOWN), None, "recording: no countdown");
}

/// A GIF's max length stops the recording once, paused time not counted.
#[test]
fn max_length_auto_stops_once() {
    let t = t0();
    let mut m = Machine::new();
    m.start(false, t);
    let max = Some(Duration::from_secs(15));
    assert_eq!(m.auto_stop(max, t + secs(14.9)), None);
    m.toggle_pause(t + secs(10.0));
    assert_eq!(m.auto_stop(max, t + secs(30.0)), None, "paused: not running");
    m.toggle_pause(t + secs(20.0));
    assert_eq!(m.auto_stop(max, t + secs(24.9)), None, "14.9 s recorded");
    assert_eq!(m.auto_stop(max, t + secs(25.0)), Some(Effect::Stop));
    assert_eq!(m.phase(), Phase::Stopping);
    assert_eq!(m.auto_stop(max, t + secs(26.0)), None, "once");
    let mut m = Machine::new();
    m.start(false, t);
    assert_eq!(m.auto_stop(None, t + secs(3600.0)), None, "MP4: no cap");
}

/// The pill's toggles: pause (amber, Resume), the microphone, the camera
/// (hidden), the system sound; each only when there is one; the compact
/// pill for a narrow region or on the timer's click.
#[test]
fn pill_states_follow_the_buttons() {
    let t = t0();
    let mut run = Run::fake_with(false, true, true, false, t);
    let v = run.view(t);
    assert_eq!((v.paused, v.mic, v.cam, v.system, v.compact), (false, Some(false), Some(false), None, false));
    run.act(bar::BarAct::Pause, t + secs(1.0));
    assert!(run.view(t).paused);
    run.act(bar::BarAct::Pause, t + secs(2.0));
    assert!(!run.view(t).paused);
    run.act(bar::BarAct::Mute, t);
    assert_eq!(run.view(t).mic, Some(true), "muted");
    run.act(bar::BarAct::Camera, t);
    assert_eq!(run.view(t).cam, Some(true), "camera hidden");
    run.act(bar::BarAct::Camera, t);
    assert_eq!(run.view(t).cam, Some(false));
    run.act(bar::BarAct::System, t);
    assert_eq!(run.view(t).system, None, "no system sound: nothing to mute");
    run.mic_flags().sys_open.store(true, Ordering::SeqCst);
    run.act(bar::BarAct::System, t);
    assert_eq!(run.view(t).system, Some(true));
    run.act(bar::BarAct::Compact, t);
    assert!(run.view(t).compact, "H / the timer");
    run.act(bar::BarAct::Compact, t);
    assert!(!run.view(t).compact);
    let narrow = Run::fake_with(false, false, false, true, t);
    assert!(narrow.view(t).compact, "a region under 520 px");
    let none = Run::fake(false, false, t);
    assert_eq!((none.view(t).mic, none.view(t).cam), (None, None));
}

/// Discard stops the recording and deletes the file: it ends cancelled.
#[test]
fn discard_deletes_the_file() {
    let dir = std::env::temp_dir().join(format!("rustshot-discard-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.mp4");
    std::fs::write(&file, b"x").unwrap();
    let t = t0();
    let mut run = Run::fake(false, false, t);
    run.act(bar::BarAct::Discard, t + secs(2.0));
    assert_eq!(run.phase(), Phase::Stopping);
    assert_eq!(run.log, [Effect::Go, Effect::Stop]);
    run.fake_end = Some(End::Saved { path: file.clone(), warnings: vec![] });
    assert_eq!(run.tick(t + secs(3.0)), Some(End::Cancelled));
    assert!(!file.exists(), "deleted");
    // Stop keeps it.
    std::fs::write(&file, b"x").unwrap();
    let mut run = Run::fake(false, false, t);
    run.act(bar::BarAct::Stop, t + secs(2.0));
    run.fake_end = Some(End::Saved { path: file.clone(), warnings: vec![] });
    assert!(matches!(run.tick(t + secs(3.0)), Some(End::Saved { .. })));
    assert!(file.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Silent (unmuted) for 5 s while recording: "No mic input", once; sound
/// or muting resets the watch.
#[test]
fn silent_microphone_is_said_once() {
    let t = t0();
    let mut run = Run::fake(false, true, t);
    let peak = |run: &Run, v: f32| run.mic_flags().level.store(v.to_bits(), Ordering::Relaxed);
    run.tick(t);
    run.tick(t + secs(4.9));
    assert_eq!(run.take_notice(), None);
    peak(&run, 0.2);
    run.tick(t + secs(5.0));
    peak(&run, 0.0);
    run.tick(t + secs(6.0));
    run.tick(t + secs(10.9));
    assert_eq!(run.take_notice(), None, "heard at 5 s: the watch starts again");
    run.tick(t + secs(11.0));
    assert_eq!(run.take_notice(), Some(NO_MIC_INPUT.get()));
    run.tick(t + secs(30.0));
    assert_eq!(run.take_notice(), None, "once per recording");
    // Muted: not a problem.
    let mut run = Run::fake(false, true, t);
    run.toggle_mute();
    run.tick(t);
    run.tick(t + secs(20.0));
    assert_eq!(run.take_notice(), None);
    // No microphone: nothing to watch.
    let mut run = Run::fake(false, false, t);
    run.tick(t);
    run.tick(t + secs(20.0));
    assert_eq!(run.take_notice(), None);
}

/// The estimate toast: the encoded height, the rate and MB a minute.
#[test]
fn estimate_follows_size_rate_and_format() {
    let mut o = RecOpts::from_config(&Config::default());
    o.mic = false;
    o.system = false;
    assert_eq!(o.estimate((1920, 1080), crate::rec::Quality::Medium), "1080p \u{b7} 30 fps \u{b7} est. 47 MB / min");
    o.system = true;
    let with_sound = o.estimate((1920, 1080), crate::rec::Quality::Medium);
    assert!(with_sound.ends_with("est. 48 MB / min"), "{with_sound}");
    assert!(o.estimate((640, 360), crate::rec::Quality::Low).starts_with("360p \u{b7} 30 fps \u{b7} est. "));
    o.format = RecFormat::Gif;
    o.fps_gif = 10;
    assert_eq!(o.estimate((400, 300), crate::rec::Quality::Medium), "300p \u{b7} 10 fps \u{b7} est. 3.6 MB / min");
}
