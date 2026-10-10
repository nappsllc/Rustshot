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
    assert_eq!((o.format, o.fps(), o.fps_choices()), (RecFormat::Mp4, 30, [30, 60]));
    assert!(o.system && !o.mic && o.mic_dev.is_empty() && !o.camera);
    let cfg = Config { rec_format: "gif".into(), rec_gif_fps: 10, rec_fps: 60, rec_mic: true, ..Config::default() };
    let mut o = RecOpts::from_config(&cfg);
    assert_eq!((o.format, o.fps(), o.fps_choices()), (RecFormat::Gif, 10, [10, 15]));
    o.set_fps(60); // not a GIF rate: ignored
    assert_eq!(o.fps(), 10);
    o.set_fps(15);
    assert_eq!(o.fps(), 15);
    o.format = RecFormat::Mp4;
    assert_eq!(o.fps(), 60, "each format keeps its own rate");
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
    Geom { area: MON, monitors: vec![(MON, 1.0), ((1920, 0, 2560, 1440), 2.0)], scale: 1.0 }
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
    let g = Geom { area: (1700, 100, 200, 200), monitors: vec![(MON, 1.0)], scale: 1.0 };
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
    let mut m = Muted { inner: Tone(vec![0.5; 8]), muted: muted.clone() };
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
                ("recording", bar::BarView { elapsed: secs(83.0), paused: false, mic: Some(false), stopping: false }, Some(bar::BarAct::Stop)),
                ("paused", bar::BarView { elapsed: secs(754.0), paused: true, mic: Some(true), stopping: false }, None),
                ("nomic", bar::BarView { elapsed: secs(5.0), paused: false, mic: None, stopping: false }, Some(bar::BarAct::Pause)),
                ("stopping", bar::BarView { elapsed: secs(9.0), paused: false, mic: None, stopping: true }, None),
            ];
            for (vn, v, hover) in views {
                let l = bar::layout(&ui, &v);
                let (w, h) = (l.size.0 as usize, l.size.1 as usize);
                let px = render_layer(w, h, |f| bar::draw(f, &ui, &l, &v, hover, None));
                save(&format!("{tn}-bar-{vn}@{s}x.png"), w, h, &px, bg);
            }
            for n in [3, 1] {
                let (w, h) = countdown::size(s);
                let px = render_layer(w as usize, h as usize, |f| countdown::draw(f, &ui, n));
                save(&format!("{tn}-countdown-{n}@{s}x.png"), w as usize, h as usize, &px, bg);
            }
        }
        // The tray while recording: the icon (enlarged) and the menu.
        let ui = Ui { th, s: 2.0, font: Some(crate::fonts::ui_font()) };
        let items = crate::tray::menu(Some(false), true);
        let (w, h) = (360usize, 80 + 56 * items.len());
        let px = render_layer(w, h, |f| {
            let fg = if th.dark { (255, 255, 255) } else { (27, 28, 32) };
            let g = crate::tray::tray_glyph_rgba_rec(32, fg, true);
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
                f.draw_text(font, 26.0, it.label(), 34.0, y, th.text);
            }
        });
        save(&format!("{tn}-tray-recording.png"), w, h, &px, bg);
    }
}
