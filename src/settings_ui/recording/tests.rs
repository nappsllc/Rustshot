use super::*;
use crate::settings_ui::draw::{paint, viewport};
use crate::settings_ui::{Act, Click, Form, Settings, CAMERA, RECORDING, W, H};
use crate::theme::{Theme, DARK, LIGHT};
use crate::ui::{preview, FocusState, Input};
use std::path::Path;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rustshot-setrec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every recording key away from its default.
fn custom() -> Config {
    Config {
        rec_format: "gif".into(),
        rec_fps: 60,
        rec_gif_fps: 24,
        rec_gif_max: 60,
        rec_quality: "high".into(),
        rec_system_audio: false,
        rec_mic: true,
        rec_mic_device: "{mic-2}".into(),
        rec_camera: true,
        rec_camera_device: "{cam-2}".into(),
        rec_camera_shape: "rounded".into(),
        rec_camera_size: "xl".into(),
        rec_camera_mirror: false,
        rec_countdown: false,
        record_hotkey: "Ctrl+Alt+R".into(),
        rec_linux_encoder: "openh264".into(),
        rec_camera_ring_color: "#3b82f6".into(),
        rec_camera_ring_width: 7,
        ..Config::default()
    }
}

#[test]
fn form_round_trip_keeps_every_recording_value() {
    for cfg in [Config::default(), custom()] {
        let f = Form::new(&cfg, None);
        assert_eq!(config::to_toml(&f.apply(&cfg, &cfg)), config::to_toml(&cfg));
        // And the form made from the result is the same form.
        assert_eq!(RecForm::new(&f.apply(&cfg, &cfg)), f.rec);
    }
    // Spellings the form cannot show survive when left alone.
    let odd = Config {
        record_hotkey: "Hyper+R".into(),
        rec_camera_ring_color: "#ABCDEF".into(),
        rec_mic_device: "gone".into(),
        ..Config::default()
    };
    let f = Form::new(&odd, None);
    assert_eq!(f.rec.hotkey, None);
    assert_eq!(config::to_toml(&f.apply(&odd, &odd)), config::to_toml(&odd));
}

/// Each control writes its own key, and only that key.
#[test]
fn form_writes_each_recording_key() {
    let d = Config::default();
    let c = custom();
    type Edit = Box<dyn Fn(&mut RecForm)>;
    let edits: Vec<(&str, Edit)> = vec![
        ("rec_format", Box::new(|f| f.format = 1)),
        ("rec_fps", Box::new(|f| f.fps = 1)),
        ("rec_gif_fps", Box::new(|f| f.gif_fps = 2)),
        ("rec_gif_max", Box::new(|f| f.gif_max = 2)),
        ("rec_quality", Box::new(|f| f.quality = 2)),
        ("rec_system_audio", Box::new(|f| f.system = false)),
        ("rec_mic", Box::new(|f| f.mic = true)),
        ("rec_mic_device", Box::new(|f| f.mic_dev = "{mic-2}".into())),
        ("rec_camera", Box::new(|f| f.camera = true)),
        ("rec_camera_device", Box::new(|f| f.cam_dev = "{cam-2}".into())),
        ("rec_camera_shape", Box::new(|f| f.shape = 1)),
        ("rec_camera_size", Box::new(|f| f.size = 3)),
        ("rec_camera_mirror", Box::new(|f| f.mirror = false)),
        ("rec_countdown", Box::new(|f| f.countdown = false)),
        ("record_hotkey", Box::new(|f| f.hotkey = Chord::parse("Ctrl+Alt+R"))),
        ("rec_linux_encoder", Box::new(|f| f.encoder = 2)),
        ("rec_camera_ring_color", Box::new(|f| f.set_ring(6))),
        ("rec_camera_ring_width", Box::new(|f| f.ring_w = 7)),
    ];
    let line = |t: &str, k: &str| t.lines().find(|l| l.starts_with(&format!("{k} ="))).map(str::to_string);
    for (key, edit) in &edits {
        let mut f = Form::new(&d, None);
        edit(&mut f.rec);
        let out = config::to_toml(&f.apply(&d, &d));
        let (base, want) = (config::to_toml(&d), config::to_toml(&c));
        for l in base.lines() {
            let k = l.split(" =").next().unwrap();
            if k == *key {
                assert_eq!(line(&out, k), line(&want, k), "{key}");
            } else {
                assert_eq!(line(&out, k), Some(l.to_string()), "{key} changed {k}");
            }
        }
    }
    // Clearing the hotkey writes an empty one.
    let mut f = Form::new(&d, None);
    f.rec.hotkey = None;
    assert_eq!(f.apply(&d, &d).record_hotkey, "");
    // A value left alone keeps the file's newer one.
    let f = Form::new(&d, None);
    let onto = Config { rec_camera_ring_width: 9, ..d.clone() };
    assert_eq!(f.apply(&d, &onto).rec_camera_ring_width, 9);
}

/// The redesign's choices: GIF at 24 fps, a GIF's max length (15 / 30 /
/// 60 s, 30 by default), the XL bubble, a 2 px ring by default.
#[test]
fn redesign_recording_choices() {
    let f = RecForm::new(&Config::default());
    assert_eq!((GIF_MAX[f.gif_max].1, SIZES[f.size].1, f.ring_w), (30, "m", 2));
    assert_eq!(labels(&SIZES), ["120", "160", "200", "240"]);
    let f = RecForm::new(&Config { rec_gif_fps: 24, rec_gif_max: 15, rec_camera_size: "xl".into(), ..Config::default() });
    assert_eq!((GIF_FPS[f.gif_fps].1, GIF_MAX[f.gif_max].1, SIZES[f.size].1), (24, 15, "xl"));
    for (n, _) in GIF_MAX {
        let c = Config { rec_gif_max: n.trim_end_matches(" s").parse().unwrap(), ..Config::default() };
        assert_eq!(config::parse_config(&config::to_toml(&c)).unwrap().rec_gif_max, c.rec_gif_max);
    }
}

#[test]
fn ring_choices() {
    let mut f = RecForm::new(&Config::default());
    let (items, sel) = f.ring_items();
    assert_eq!((items.len(), sel, items[0].as_str()), (11, 0, "Accent"));
    f.set_ring(1);
    assert_eq!((f.ring.as_str(), f.ring_items().1), ("#f04438", 1));
    f.set_ring(0);
    assert_eq!(f.ring, "");
    // A color from the picker (not in the list) gets its own item, kept when picked.
    let mut f = RecForm::new(&Config { rec_camera_ring_color: "#ABCDEF".into(), ..Config::default() });
    let (items, sel) = f.ring_items();
    assert_eq!((items.len(), sel, items[11].as_str()), (12, 11, "Custom #abcdef"));
    f.set_ring(11);
    assert_eq!(f.ring, "#abcdef");
    // The width is capped.
    assert_eq!(RecForm::new(&Config { rec_camera_ring_width: 99, ..Config::default() }).ring_w, config::RING_WIDTH_MAX as u8);
}

#[test]
fn device_lists_keep_the_saved_device() {
    let devs = vec![("{a}".to_string(), "Alpha".to_string()), ("{b}".to_string(), "Beta".to_string())];
    let (n, ids, sel) = device_items(DEFAULT_MIC, Some(&devs), "");
    assert_eq!((n, ids, sel), (vec![DEFAULT_MIC.to_string(), "Alpha".into(), "Beta".into()], vec!["".to_string(), "{a}".into(), "{b}".into()], 0));
    assert_eq!(device_items(DEFAULT_MIC, Some(&devs), "{b}").2, 2);
    let (n, ids, sel) = device_items(DEFAULT_MIC, Some(&devs), "{gone}");
    assert_eq!((n[3].as_str(), ids[3].as_str(), sel), ("Saved device (not connected)", "{gone}", 3));
    // Not listed yet.
    let (n, _, sel) = device_items(DEFAULT_CAM, None, "{cam}");
    assert_eq!((n, sel), (vec![DEFAULT_CAM.to_string(), "Saved device".into()], 1));
}

#[test]
fn encoder_status_lines() {
    let ff = Path::new("/usr/bin/ffmpeg");
    assert_eq!(encoder_status(0, Some(ff), true), "ffmpeg found at /usr/bin/ffmpeg");
    assert_eq!(encoder_status(0, None, true), "OpenH264 downloaded");
    assert_eq!(encoder_status(1, Some(ff), false), "ffmpeg found at /usr/bin/ffmpeg");
    assert_eq!(encoder_status(1, None, true), "OpenH264 downloaded");
    assert_eq!(encoder_status(2, Some(ff), true), "OpenH264 downloaded");
    assert_eq!(encoder_status(2, Some(ff), false), "ffmpeg found at /usr/bin/ffmpeg");
    for sel in 0..3 {
        assert!(encoder_status(sel, None, false).starts_with("Not available"), "{sel}");
    }
}

/// A change on the tab is saved by Apply and read back.
#[test]
fn apply_saves_the_recording_tab() {
    let dir = scratch("apply");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = Settings::open(p.clone(), None);
    s.section = RECORDING;
    assert!(!s.dirty());
    s.form.rec.format = 1;
    s.form.rec.countdown = false;
    s.form.rec.hotkey = Chord::parse("Ctrl+Alt+F7");
    assert!(s.dirty());
    let acts = s.on(Click::Apply);
    assert!(matches!(&acts[..], [Act::Saved(c)] if c.rec_format == "gif" && !c.rec_countdown), "{acts:?}");
    let back = config::read_at(&p).unwrap().unwrap();
    assert_eq!((back.rec_format.as_str(), back.rec_countdown, back.record_hotkey.as_str()), ("gif", false, "Ctrl+Alt+F7"));
    assert!(!s.dirty());
    let _ = std::fs::remove_dir_all(&dir);
}

// --- Rendering ---

fn sample(cfg: Config) -> Settings {
    let mut s = Settings::new(cfg, Some(true), PathBuf::from("config.toml"));
    s.section = RECORDING;
    s
}

/// Two frames (focus order, then the picture); returns the image and
/// the section's content height (logical px).
fn render(s: &mut Settings, th: &Theme, k: f32) -> (crate::pixbuf::PixBuf, f32) {
    let mut focus = FocusState::default();
    preview::render(W, H, k, th, &Input::default(), &mut focus, |ui| {
        paint(ui, s);
    });
    let img = preview::render(W, H, k, th, &Input::default(), &mut focus, |ui| {
        paint(ui, s);
    });
    (img, s.content_h[s.section])
}

/// Recording and Camera fit the body in every state, the tallest (Linux
/// encoder row, a GIF's notes, no camera) included.
#[test]
fn recording_sections_fit() {
    for sec in [RECORDING, CAMERA] {
        let mut s = sample(Config { rec_format: "gif".into(), ..Config::default() });
        s.section = sec;
        s.rec_lists.linux = true;
        s.rec_lists.found = Some(Found { cams: Vec::new(), ..find() });
        let (_, h) = render(&mut s, &DARK, 1.0);
        assert!(h > 100.0 && h <= viewport().h, "section {sec}: {h}");
    }
}

#[test]
fn preview_recording_section_pngs() {
    let ffmpeg = Found { ffmpeg: Some("/usr/bin/ffmpeg".into()), ..find() };
    for (tname, th) in [("dark", &DARK), ("light", &LIGHT)] {
        for k in [1.0f32, 1.5] {
            let sfx = if k == 1.0 { String::new() } else { "-150".into() };
            let mut s = sample(Config { rec_mic: true, rec_camera: true, ..Config::default() });
            let (img, _) = render(&mut s, th, k);
            preview::save(&format!("settings-recording-video-{tname}{sfx}.png"), &img);
            s.section = CAMERA;
            let (img, _) = render(&mut s, th, k);
            preview::save(&format!("settings-recording-camera-{tname}{sfx}.png"), &img);
        }

        // GIF: no quality, no sound; a colored ring.
        let mut s = sample(Config { rec_format: "gif".into(), rec_camera_ring_color: "#2dc06f".into(), rec_camera_ring_width: 6, ..Config::default() });
        let (img, _) = render(&mut s, th, 1.0);
        preview::save(&format!("settings-recording-gif-{tname}.png"), &img);
        s.section = CAMERA;
        let (img, _) = render(&mut s, th, 1.0);
        preview::save(&format!("settings-recording-gif-camera-{tname}.png"), &img);

        // Linux: the encoder row, three states.
        for (name, found) in [
            ("ffmpeg", ffmpeg.clone()),
            ("openh264", Found { openh264: true, ..find() }),
            ("none", find()),
        ] {
            let enc = if name == "openh264" { "openh264" } else { "auto" };
            let mut s = sample(Config { rec_linux_encoder: enc.into(), ..Config::default() });
            s.rec_lists.linux = true;
            s.rec_lists.found = Some(found);
            let (img, _) = render(&mut s, th, 1.0);
            preview::save(&format!("settings-recording-linux-{name}-{tname}.png"), &img);
        }

        // No camera, a saved microphone that is gone.
        let mut s = sample(Config { rec_mic: true, rec_mic_device: "{unplugged}".into(), ..Config::default() });
        s.rec_lists.found = Some(Found { cams: Vec::new(), ..find() });
        let (img, _) = render(&mut s, th, 1.0);
        preview::save(&format!("settings-recording-unplugged-{tname}.png"), &img);
        s.section = CAMERA;
        let (img, _) = render(&mut s, th, 1.0);
        preview::save(&format!("settings-recording-nocam-{tname}.png"), &img);
    }
}

/// A GIF can have the camera bubble: only the sound rows are off. The
/// Camera row's label reads the same with the camera on or off (the
/// switch greys its device list, never the row), and so does every
/// other label of the section.
#[test]
fn a_gif_keeps_the_camera_rows_enabled() {
    let cams = Found { cams: vec![("{cam-1}".into(), "Integrated Camera".into())], ..find() };
    let left = |camera: bool, format: &str, section: usize| {
        let mut s = sample(Config { rec_format: format.into(), rec_camera: camera, ..Config::default() });
        s.rec_lists.found = Some(cams.clone());
        s.section = section;
        let (img, _) = render(&mut s, &DARK, 1.0);
        // The labels' half (the controls are right-aligned).
        let w = img.width() as usize;
        img.as_raw().chunks_exact(w * 4).flat_map(|r| r[..w * 4 * 45 / 100].to_vec()).collect::<Vec<u8>>()
    };
    assert!(left(false, "gif", CAMERA) == left(true, "gif", CAMERA), "the Camera label greys with the switch off");
    assert!(left(false, "mp4", CAMERA) == left(true, "mp4", CAMERA));
    assert!(left(true, "gif", CAMERA) == left(true, "mp4", CAMERA), "a GIF keeps the camera rows");
    // And GIF still greys the sound rows (the left side differs from MP4).
    assert!(left(true, "gif", RECORDING) != left(true, "mp4", RECORDING), "GIF greys System sound and Microphone");
}
