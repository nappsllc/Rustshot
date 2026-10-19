use super::draw::{page_actions, page_of, page_rows, rail_area, viewport, FOOT_H, HDR_H, KEY_COL_GAP, KEY_PAGES, RAIL, RAIL_W, REC_ROWS};
use super::*;
use crate::theme::{DARK, LIGHT};
use crate::ui::preview;

const NOW: export::Tm = (2026, 10, 10, 14, 30, 5);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rustshot-set-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn custom() -> Config {
    Config {
        save_path: r"D:\Shots".into(),
        save_subfolder: false,
        subfolder_pattern: "%Y/%m".into(),
        filename_pattern: "shot_%H%M".into(),
        save_format: "jpg".into(),
        jpeg_quality: 75,
        save_dialog: true,
        theme: "light".into(),
        ui_color: "#f2a65a".into(),
        renderer: "software".into(),
        check_updates: false,
        capture_hotkey: "Ctrl+Alt+F7".into(),
        quit_hotkey: "Ctrl+Alt+Shift+F8".into(),
        capture_full_hotkey: "Ctrl+Alt+F".into(),
        capture_window_hotkey: "".into(),
        repeat_hotkey: "Ctrl+Alt+P".into(),
        show_tray: false,
        copy_after_capture: true,
        shutter_sound: true,
        upload_client_id: "abc123".into(),
        copy_url_after_upload: false,
        skip_version: "0.1.9".into(),
        shortcuts: [("save".to_string(), "Ctrl+Alt+S".to_string()), ("bogus".to_string(), "Q".to_string())]
            .into_iter()
            .collect(),
        ..Config::default()
    }
}

#[test]
fn form_round_trip_keeps_every_value() {
    for cfg in [Config::default(), custom(), Config { ui_color: "#123456".into(), ..Config::default() }] {
        let f = Form::new(&cfg, Some(true));
        assert_eq!(config::to_toml(&f.apply(&cfg, &cfg)), config::to_toml(&cfg));
    }
    // Values the form cannot show survive when left alone.
    let odd = Config {
        theme: "Midnight".into(),
        renderer: "vulkan".into(),
        capture_hotkey: "Hyper+X".into(),
        save_format: "JPEG".into(),
        ui_color: "not a color".into(),
        ..Config::default()
    };
    let f = Form::new(&odd, None);
    assert_eq!((f.theme, f.format, f.capture, f.accent), (0, 1, None, 0));
    assert_eq!(config::to_toml(&f.apply(&odd, &odd)), config::to_toml(&odd));
}

#[test]
fn form_writes_what_changed() {
    let cfg = Config::default();
    let mut f = Form::new(&cfg, None);
    f.theme = 2;
    f.renderer = 1;
    f.check_updates = false;
    f.capture = Chord::parse("Ctrl+Alt+F9");
    f.quit = None;
    f.full = Chord::parse("Ctrl+Alt+F1");
    f.window = None;
    f.repeat = Chord::parse("Ctrl+Alt+F2");
    f.show_tray = false;
    f.copy_after = true;
    f.shutter = true;
    f.folder.set(r"E:\caps");
    f.subfolder = false;
    f.subfolder_pattern.set("%Y");
    f.format = 2;
    f.quality = 50;
    f.ask = true;
    f.filename.set("x_%S");
    f.up.client_id.set(" mine ");
    f.up.copy_url = false;
    f.keys.set(Action::ToolPencil, vec![Chord::parse("Shift+K").unwrap()]);
    f.keys.set(Action::Copy, vec![]);
    let c = f.apply(&cfg, &cfg);
    // The renderer choice only exists on Windows; elsewhere the form already
    // starts at "software", so the default spelling is kept.
    let renderer = if cfg!(windows) { "software" } else { "gdi" };
    assert_eq!((c.theme.as_str(), c.renderer.as_str(), c.check_updates), ("light", renderer, false));
    assert_eq!((c.capture_hotkey.as_str(), c.quit_hotkey.as_str()), ("Ctrl+Alt+F9", ""));
    assert_eq!((c.capture_full_hotkey.as_str(), c.capture_window_hotkey.as_str(), c.repeat_hotkey.as_str()), ("Ctrl+Alt+F1", "", "Ctrl+Alt+F2"));
    assert_eq!((c.show_tray, c.copy_after_capture, c.shutter_sound), (false, true, true));
    assert_eq!((c.save_path.as_str(), c.save_subfolder, c.subfolder_pattern.as_str()), (r"E:\caps", false, "%Y"));
    assert_eq!((c.save_format.as_str(), c.jpeg_quality, c.save_dialog), ("bmp", 50, true));
    assert_eq!(c.filename_pattern, "x_%S");
    assert_eq!((c.upload_client_id.as_str(), c.copy_url_after_upload), ("mine", false));
    assert_eq!(c.shortcuts.len(), 2);
    assert_eq!((c.shortcuts["tool_pencil"].as_str(), c.shortcuts["copy"].as_str()), ("Shift+K", ""));
    // And it reads back as the same form.
    let back = Form::new(&c, None);
    assert_eq!((back.theme, back.renderer, back.format, back.quality), (2, 1, 2, 50));
    assert_eq!(back.keys, f.keys);
}

/// Accent: the swatches write their `ui_color` (Periwinkle: none), a
/// custom color is written only when it parses.
#[test]
fn accent_swatches_and_custom_color() {
    let cfg = Config::default();
    assert_eq!(Form::new(&cfg, None).accent, 0);
    assert_eq!(Form::new(&Config { ui_color: "#5CC8C2".into(), ..cfg.clone() }, None).accent, 1);
    assert_eq!(Form::new(&Config { ui_color: "#f2a65a".into(), ..cfg.clone() }, None).accent, 2);
    let c = Form::new(&Config { ui_color: "#123456".into(), ..cfg.clone() }, None);
    assert_eq!((c.accent, c.accent_hex.text.as_str()), (ACCENT_CUSTOM, "#123456"));
    let mut f = Form::new(&cfg, None);
    f.accent = 1;
    assert_eq!(f.apply(&cfg, &cfg).ui_color, "#5cc8c2");
    f.accent = 2;
    assert_eq!(f.apply(&cfg, &cfg).ui_color, "#f2a65a");
    f.accent = ACCENT_CUSTOM;
    f.accent_hex.set("#00ff88");
    assert_eq!(f.apply(&cfg, &cfg).ui_color, "#00ff88");
    f.accent_hex.set("#zz");
    assert_eq!(f.apply(&cfg, &cfg).ui_color, "", "not a color: not written");
    let teal = Config { ui_color: "#5cc8c2".into(), ..cfg.clone() };
    let mut f = Form::new(&teal, None);
    f.accent = 0;
    assert_eq!(f.apply(&teal, &teal).ui_color, "", "back to the theme's");
}

#[test]
fn untouched_values_follow_the_file() {
    // The file changed while the window was open (Skip this version, an
    // edit by hand): only the user's own change is written over it.
    let loaded = Config::default();
    let mut f = Form::new(&loaded, None);
    f.format = 1;
    let disk = Config { skip_version: "0.2.0".into(), theme: "dark".into(), ..Config::default() };
    let c = f.apply(&loaded, &disk);
    assert_eq!((c.save_format.as_str(), c.skip_version.as_str(), c.theme.as_str()), ("jpg", "0.2.0", "dark"));
    // Unknown shortcut entries stay when the keymap is edited.
    let cfg = custom();
    let mut f = Form::new(&cfg, None);
    f.keys.set(Action::Undo, vec![Chord::parse("F2").unwrap()]);
    let c = f.apply(&cfg, &cfg);
    assert_eq!(c.shortcuts.get("bogus").map(String::as_str), Some("Q"));
    assert_eq!(c.shortcuts["undo"], "F2");
    assert_eq!(c.shortcuts["save"], "Ctrl+Alt+S");
}

#[test]
fn empty_save_path_shows_the_default_folder() {
    let cfg = Config::default();
    let f = Form::new(&cfg, None);
    assert_eq!(f.folder.text, export::default_save_dir(&cfg).display().to_string());
    assert_eq!(f.apply(&cfg, &cfg).save_path, "", "left alone it stays the default");
}

#[test]
fn tokens_insert_at_the_caret() {
    let mut f = Form::new(&Config::default(), None);
    f.filename.set("ab");
    f.filename.select(1..1);
    f.insert(Pattern::File, "%H");
    assert_eq!(f.filename.text, "a%Hb");
    assert_eq!(f.filename.caret(), 3);
    f.insert(Pattern::File, TOKENS[5].1);
    assert_eq!(f.filename.text, "a%H%d-%m-%Yb");
    // A selection is replaced.
    f.filename.select(0..1);
    f.insert(Pattern::File, "%C");
    assert_eq!(f.filename.text, "%C%H%d-%m-%Yb");
    f.subfolder_pattern.set("%F");
    f.insert(Pattern::Sub, "/%V");
    assert_eq!(f.subfolder_pattern.text, "%F/%V");
    // Every token expands to something (none is left as a literal %X).
    for (label, tok) in TOKENS {
        let s = export::format_filename(tok);
        assert!(!s.contains('%') && !s.is_empty(), "{label}: {s}");
    }
}

#[test]
fn preview_is_the_auto_save_path() {
    let cfg = custom();
    let mut f = Form::new(&cfg, None);
    assert_eq!(f.preview(&cfg, NOW), export::auto_save_path(&cfg, NOW));
    f.subfolder = true;
    f.format = 0;
    f.filename.set("%F %H");
    let edited = Config { save_subfolder: true, save_format: "png".into(), filename_pattern: "%F %H".into(), ..cfg.clone() };
    assert_eq!(f.preview(&cfg, NOW), export::auto_save_path(&edited, NOW));
    assert_eq!(f.preview(&cfg, NOW), PathBuf::from(r"D:\Shots").join("2026").join("10").join("2026-10-10 14.png"));
}

/// Overlay keys in a conflict are marked and get a message; globals that
/// share a chord, or a bare global key, get theirs.
#[test]
fn conflicts_and_global_notes() {
    let mut f = Form::new(&Config::default(), None);
    assert!(f.conflicting().is_empty());
    assert_eq!(f.conflict_note(Action::ToolPencil), None);
    f.keys.set(Action::Cancel, vec![Chord::parse("P").unwrap()]);
    assert_eq!(f.conflicting(), [Action::ToolPencil, Action::Cancel]);
    assert_eq!(f.conflict_note(Action::Cancel).unwrap(), "Already used by Pencil. Choose another.");
    assert_eq!(f.conflict_note(Action::ToolPencil).unwrap(), "Already used by Cancel. Choose another.");
    // Defaults: no global problem.
    for slot in 0..hotkey::SLOTS {
        assert_eq!(f.global_note(slot), None, "slot {slot}");
    }
    f.repeat = f.capture;
    assert_eq!(f.global_note(5).unwrap(), "Already used by Capture region. Choose another.");
    assert_eq!(f.global_note(0).unwrap(), "Already used by Repeat last capture. Choose another.");
    f.repeat = Chord::parse("X");
    assert!(f.global_note(5).unwrap().contains("every app"));
    f.repeat = Chord::parse("PrintScreen");
    assert_eq!(f.global_note(5), None);
    f.rec.hotkey = f.quit;
    assert!(f.global_note(2).unwrap().contains("Quit Rustshot"));
}

/// Reset to defaults puts every overlay key and global hotkey back.
#[test]
fn reset_to_defaults_covers_globals_too() {
    let mut f = Form::new(&custom(), None);
    f.keys.set(Action::Save, vec![]);
    f.reset_keys();
    assert_eq!(f.keys, Keymap::defaults());
    let d = Form::new(&Config::default(), None);
    for slot in 0..hotkey::SLOTS {
        assert_eq!(f.global(slot), d.global(slot), "slot {slot}");
    }
}

fn settings_at(path: &Path) -> Settings {
    let mut s = Settings::open(path.to_path_buf(), None);
    s.set_autostart = |_| Err("not in tests".into());
    s.set_screenclip = |_| Err("not in tests".into());
    s.read_handler = || Handler::None;
    s
}

/// The Win+Shift+S toggle: applied on Apply through its setter (never
/// the real registry here), kept across the form rebuild, a failure said
/// in the window; hidden where it is not offered.
#[test]
fn screenclip_toggle_applies_on_save() {
    use std::sync::atomic::{AtomicU8, Ordering};
    static LAST: AtomicU8 = AtomicU8::new(9);
    let dir = scratch("screenclip");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p).with_screenclip(Some(false), Handler::None);
    s.set_screenclip = |on| {
        LAST.store(on as u8, Ordering::SeqCst);
        Ok(())
    };
    s.read_handler = || Handler::Ours;
    assert!(!s.dirty());
    s.form.screenclip = Some(true);
    assert!(s.dirty());
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]));
    assert_eq!(LAST.load(Ordering::SeqCst), 1);
    assert_eq!(s.form.screenclip, Some(true), "kept across the rebuild");
    assert_eq!(s.handler, Handler::Ours, "read again after the change");
    assert!(s.modal.is_none(), "already the default: no guide");
    assert!(!s.dirty());
    s.form.screenclip = Some(false);
    s.on(Click::Apply);
    assert_eq!(LAST.load(Ordering::SeqCst), 0);
    assert!(!s.dirty());
    // A failure: the error shows, the toggle stays as asked (still dirty).
    s.set_screenclip = |_| Err("Access is denied.".into());
    s.form.screenclip = Some(true);
    s.on(Click::Apply);
    assert!(matches!(&s.modal, Some(Modal::Error(e)) if e.contains("Win+Shift+S") && e.contains("denied")));
    assert!(s.dirty());
    // Not offered: no row, never dirty.
    let s = settings_at(&p);
    assert_eq!(s.form.screenclip, None);
    assert!(!s.dirty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The download switch: Settings starts from the file's value (a broken
/// file: off), the switch itself changes nothing until saved, Cancel
/// leaves downloads as they were, Apply sets them.
#[test]
fn download_switch_applies_on_save() {
    let dir = scratch("download-switch");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config { download_data: false, ..Config::default() }).unwrap();
    let mut s = settings_at(&p);
    s.section = GENERAL;
    let _ = crate::fetch::take_last_set();
    let mut focus = FocusState::default();
    click(&mut s, &mut focus, "download_data", 1.0);
    assert!(s.form.download_data, "the switch moved");
    assert_eq!(crate::fetch::take_last_set(), None, "not applied before a save");
    assert_eq!(s.on(Click::Escape), vec![Act::Close]);
    assert_eq!(crate::fetch::take_last_set(), None, "Cancel: as it was");
    let mut s = settings_at(&p);
    s.form.download_data = true;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(c)] if c.download_data));
    assert_eq!(crate::fetch::take_last_set(), Some(true), "applied on save");
    s.form.download_data = false;
    s.on(Click::Apply);
    assert_eq!(crate::fetch::take_last_set(), Some(false));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The fake handler the guide tests read: 0 none, 1 the Snipping Tool,
/// 2 a missing app, 3 Rustshot. One test at a time sets it.
static FAKE_HANDLER: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static GUIDE_LOCK: Mutex<()> = Mutex::new(());

fn fake_handler() -> Handler {
    match FAKE_HANDLER.load(std::sync::atomic::Ordering::SeqCst) {
        1 => Handler::Other { progid: "AppXsnip".into(), name: "Snipping Tool".into() },
        2 => Handler::Broken { progid: "AppXgone".into() },
        3 => Handler::Ours,
        _ => Handler::None,
    }
}

fn set_fake(v: u8) {
    FAKE_HANDLER.store(v, std::sync::atomic::Ordering::SeqCst);
}

fn guide_settings(p: &Path) -> Settings {
    let mut s = settings_at(p).with_screenclip(Some(false), Handler::None);
    s.set_screenclip = |_| Ok(());
    s.read_handler = fake_handler;
    s
}

/// Turning the toggle on while Windows opens something else shows the
/// guide (and OK leaves the window open for it); Open Default apps keeps
/// it; back in the window with Rustshot chosen, it closes with a toast
/// and a ticked "Don't show again" is written.
#[test]
fn screenclip_guide_flow() {
    let _g = GUIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("guide");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = guide_settings(&p);
    set_fake(2);
    s.form.screenclip = Some(true);
    assert!(matches!(&s.on(Click::Ok)[..], [Act::Saved(_)]), "no Close under the guide");
    assert_eq!(s.modal, Some(Modal::Guide { broken: true, dont_show: false }));
    let (title, text) = s.guide_texts(true);
    assert_eq!(title, "One more step to use Win+Shift+S");
    assert!(text.starts_with(crate::screenclip_win::GUIDE_BROKEN.get()) && text.contains("Choose defaults by link type"));
    // Enter is Open Default apps; the guide stays.
    assert_eq!(s.on(Click::Enter), vec![Act::OpenDefaultApps]);
    assert!(s.guide_away && matches!(s.modal, Some(Modal::Guide { .. })));
    // Back, nothing changed.
    assert!(!s.refocused());
    assert!(s.toast.is_none());
    // Back, the Snipping Tool chosen: no longer broken, still the guide.
    set_fake(1);
    assert!(s.refocused());
    assert_eq!(s.modal, Some(Modal::Guide { broken: false, dont_show: false }));
    assert_eq!(s.on(Click::DontShow), vec![]);
    assert_eq!(s.modal, Some(Modal::Guide { broken: false, dont_show: true }));
    // Back with Rustshot chosen: closed, the toast, the hint off on disk.
    set_fake(3);
    assert!(s.refocused());
    assert!(s.modal.is_none() && !s.guide_away);
    assert_eq!(s.toast.as_ref().map(|t| t.0.as_str()), Some("Win+Shift+S now opens Rustshot"));
    assert!(s.toast_live());
    assert!(!config::read_at(&p).unwrap().unwrap().screenclip_hint);
    assert!(!s.dirty(), "the hint is not a form change");
    // Rustshot already chosen: the guide does not show.
    s.open_guide();
    assert!(s.modal.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Remind me later (and Esc, and the close button) closes the guide and
/// writes nothing; the guide waits behind another message; off, it
/// never shows.
#[test]
fn screenclip_guide_later() {
    let _g = GUIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("guide-later");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let before = std::fs::read_to_string(&p).unwrap();
    set_fake(1);
    let mut s = guide_settings(&p);
    s.open_guide();
    assert!(s.modal.is_none(), "not registered: no guide");
    s.form.screenclip = Some(true);
    s.on(Click::Apply);
    assert_eq!(s.modal, Some(Modal::Guide { broken: false, dont_show: false }));
    assert!(s.guide_texts(false).1.starts_with("Windows lets only you choose"));
    assert_eq!(s.on(Click::RemindLater), vec![]);
    assert!(s.modal.is_none());
    s.open_guide();
    s.on(Click::DontShow);
    s.on(Click::DontShow); // unticked again
    assert_eq!(s.on(Click::Escape), vec![]);
    assert!(s.modal.is_none());
    s.open_guide();
    assert_eq!(s.on(Click::Cancel), vec![Act::Close]);
    assert!(config::read_at(&p).unwrap().unwrap().screenclip_hint);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), before, "nothing written");
    // Behind an error: shown once it is dismissed.
    s.notify("Couldn't register Ctrl+Alt+F9 \u{2014} it's in use by another app.".into());
    s.open_guide();
    assert!(matches!(s.modal, Some(Modal::Error(_))));
    s.on(Click::Back);
    assert_eq!(s.modal, Some(Modal::Guide { broken: false, dont_show: false }));
    // The red line's click opens it too.
    s.modal = None;
    set_fake(2);
    assert_eq!(s.on(Click::FixScreenclip), vec![]);
    assert_eq!(s.modal, Some(Modal::Guide { broken: true, dont_show: false }));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Review fixes: a "Don't show again" that cannot be saved keeps the
/// window open with the error; focus changes read nothing unless the
/// guide is up (or Default apps was opened); Default apps failing to open
/// shows in the guide.
#[test]
fn screenclip_guide_edges() {
    let _g = GUIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("guide-edges");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    set_fake(1);
    let mut s = guide_settings(&p);
    s.form.screenclip = Some(true);
    s.on(Click::Apply);
    assert!(matches!(s.modal, Some(Modal::Guide { .. })));
    s.on(Click::DontShow);
    // The file broke meanwhile: the hint is not written, the window stays.
    std::fs::write(&p, "jpeg_quality = high\n").unwrap();
    let acts = s.on(Click::Cancel);
    assert!(!acts.contains(&Act::Close), "{acts:?}");
    assert!(matches!(&s.modal, Some(Modal::Error(e)) if e.contains("Could not save")));
    // No guide, not away: a focus change reads nothing.
    s.modal = None;
    set_fake(3);
    assert!(!s.refocused());
    assert_ne!(s.handler, Handler::Ours, "not read");
    // Default apps opened, the guide closed meanwhile: still read on return.
    set_fake(1);
    s.open_guide();
    s.on(Click::OpenDefaultApps);
    s.modal = None;
    set_fake(3);
    assert!(s.refocused());
    assert_eq!(s.handler, Handler::Ours);
    assert!(s.toast.is_some());
    // Default apps did not open: the guide says so, nothing waits.
    set_fake(1);
    s.toast = None;
    s.open_guide();
    s.on(Click::OpenDefaultApps);
    let text = crate::screenclip_win::open_failed_text("Access is denied.");
    assert_eq!(text, "Couldn't open Default apps (Access is denied). Open Settings > Apps > Default apps yourself.");
    s.default_apps_failed(text.clone());
    assert_eq!(s.guide_error.as_deref(), Some(text.as_str()));
    assert!(!s.guide_away);
    s.on(Click::RemindLater);
    assert!(s.guide_error.is_none());
    // With no guide up it is a message.
    s.default_apps_failed(text);
    assert!(matches!(&s.modal, Some(Modal::Error(e)) if e.starts_with("Couldn't open Default apps")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Capture's Win+Shift+S row: the red "click to fix" line is a link that
/// opens the guide; the other states are plain text; nothing while the
/// toggle is off or not applied yet.
#[test]
fn screenclip_row_line() {
    let _g = GUIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_fake(2);
    let mut s = sample().with_screenclip(Some(true), Handler::Broken { progid: "AppXgone".into() });
    s.read_handler = fake_handler;
    s.section = CAPTURE;
    let mut focus = FocusState::default();
    reveal(&mut s, &mut focus, "screenclip");
    assert_eq!(click(&mut s, &mut focus, "screenclip_fix", 1.0), Some(Click::FixScreenclip));
    s.on(Click::FixScreenclip);
    assert_eq!(s.modal, Some(Modal::Guide { broken: true, dont_show: false }));
    for h in [Handler::Ours, Handler::None, Handler::Other { progid: "AppXsnip".into(), name: "Snipping Tool".into() }] {
        let mut s = sample().with_screenclip(Some(true), h);
        s.section = CAPTURE;
        let mut focus = FocusState::default();
        reveal(&mut s, &mut focus, "screenclip");
        assert!(focus.rect_of("screenclip_fix").is_none());
    }
    // The tray icon off and another app chosen: the red line, a link.
    set_fake(1);
    let mut s = sample().with_screenclip(Some(true), fake_handler());
    s.read_handler = fake_handler;
    s.form.show_tray = false;
    s.section = CAPTURE;
    let mut focus = FocusState::default();
    reveal(&mut s, &mut focus, "screenclip");
    assert_eq!(click(&mut s, &mut focus, "screenclip_fix", 1.0), Some(Click::FixScreenclip));
    s.on(Click::FixScreenclip);
    assert_eq!(s.modal, Some(Modal::Guide { broken: false, dont_show: false }));
    // Saved off, turned on in the form: no line until applied.
    let mut s = sample().with_screenclip(Some(false), Handler::Broken { progid: "AppXgone".into() });
    s.form.screenclip = Some(true);
    s.section = CAPTURE;
    let mut focus = FocusState::default();
    reveal(&mut s, &mut focus, "screenclip");
    assert!(focus.rect_of("screenclip_fix").is_none());
    // The tray icon off and the toggle off: the red line all the same;
    // its click registers (as Apply would) and shows the guide.
    {
        use std::sync::atomic::{AtomicU8, Ordering};
        static SET: AtomicU8 = AtomicU8::new(9);
        set_fake(0);
        let mut s = sample().with_screenclip(Some(false), Handler::None);
        s.read_handler = fake_handler;
        s.set_screenclip = |on| {
            SET.store(on as u8, Ordering::SeqCst);
            Ok(())
        };
        s.form.show_tray = false;
        s.section = CAPTURE;
        let mut focus = FocusState::default();
        reveal(&mut s, &mut focus, "screenclip");
        assert_eq!(click(&mut s, &mut focus, "screenclip_fix", 1.0), Some(Click::FixScreenclip));
        s.on(Click::FixScreenclip);
        assert_eq!(SET.load(Ordering::SeqCst), 1);
        assert_eq!((s.screenclip, s.form.screenclip), (Some(true), Some(true)));
        assert_eq!(s.modal, Some(Modal::Guide { broken: false, dont_show: false }));
        // A failure says so and leaves it off.
        let mut s = sample().with_screenclip(Some(false), Handler::None);
        s.set_screenclip = |_| Err("Access is denied.".into());
        s.on(Click::FixScreenclip);
        assert_eq!(s.screenclip, Some(false));
        assert!(matches!(&s.modal, Some(Modal::Error(e)) if e.contains("Win+Shift+S")));
        // Registered elsewhere (show_setup_guide): the window follows.
        let mut s = sample().with_screenclip(Some(false), Handler::None);
        s.sync_screenclip(Some(true));
        assert_eq!((s.screenclip, s.form.screenclip), (Some(true), Some(true)));
        assert!(!s.dirty());
    }
    // The guide's controls click through.
    for k in [1.0f32, 1.5] {
        let mut s = sample();
        s.modal = Some(Modal::Guide { broken: true, dont_show: false });
        let mut focus = FocusState::default();
        assert_eq!(click(&mut s, &mut focus, "q_dont_show", k), Some(Click::DontShow));
        assert_eq!(click(&mut s, &mut focus, "q_open_default_apps", k), Some(Click::OpenDefaultApps));
        assert_eq!(click(&mut s, &mut focus, "q_later", k), Some(Click::RemindLater));
    }
}

/// After a save the form is rebuilt from what was written: nothing stale
/// or dirty, the folder trimmed.
#[test]
fn apply_rebuilds_the_form() {
    let dir = scratch("rebuild");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p);
    s.form.folder.set(r"  E:\caps  ");
    assert!(s.dirty());
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(c)] if c.save_path == r"E:\caps"));
    assert_eq!(s.form.folder.text, r"E:\caps");
    assert!(!s.dirty());
    // Only spaces around it: not a change.
    s.form.folder.set(r" E:\caps ");
    assert!(!s.dirty());
    let _ = std::fs::remove_dir_all(&dir);
}

const BROKEN: &str = "theme = \"dark\"\noops\n";

/// The UI size dropdown: Auto by default, written on Apply, kept as it
/// was when left alone.
#[test]
fn ui_size_is_a_capture_setting() {
    let cfg = Config::default();
    let f = Form::new(&cfg, None);
    assert_eq!(uisize::LABELS[f.ui_size], "Auto");
    assert_eq!(Form::new(&Config { ui_scale: UiSize::Pct(150), ..cfg.clone() }, None).ui_size, 5);
    let mut f = Form::new(&cfg, None);
    f.ui_size = 4;
    assert_eq!(f.apply(&cfg, &cfg).ui_scale, UiSize::Pct(125));
    // Left alone: the file's own value stays (the overlay set it meanwhile).
    let f = Form::new(&cfg, None);
    let onto = Config { ui_scale: UiSize::Pct(90), ..cfg.clone() };
    assert_eq!(f.apply(&cfg, &onto).ui_scale, UiSize::Pct(90));
    // Apply writes it.
    let dir = scratch("uisize");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p);
    s.form.ui_size = 6;
    assert!(s.dirty());
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(c)] if c.ui_scale == UiSize::Pct(200)));
    assert_eq!(config::read_at(&p).unwrap().unwrap().ui_scale, UiSize::Pct(200));
    assert!(!s.dirty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Reset tool sizes & colors" removes just those lines (comments and
/// everything else stay) and makes the daemon reload; a broken file is
/// never touched.
#[test]
fn reset_tools_removes_only_their_keys() {
    let dir = scratch("reset-tools");
    let p = dir.join("config.toml");
    let text = "# mine\ntheme = \"dark\" # why\ntool_line_size = 7\ntool_line_color = \"#00ff00\"\ntool_pixelate_size = 20\n\
                recent_colors = [\"#123456\"]\nrec_camera_ring_width = 5\n\n[shortcuts]\nsave = \"Ctrl+S\"\n";
    std::fs::write(&p, text).unwrap();
    let mut s = settings_at(&p);
    assert_eq!(s.loaded.tool_sizes.len(), 2);
    assert_eq!(s.on(Click::ResetTools), vec![Act::Reload]);
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "# mine\ntheme = \"dark\" # why\nrecent_colors = [\"#123456\"]\nrec_camera_ring_width = 5\n\n[shortcuts]\nsave = \"Ctrl+S\"\n"
    );
    assert!(s.loaded.tool_sizes.is_empty() && s.loaded.tool_colors.is_empty() && s.tools_reset);
    assert!(!s.dirty(), "nothing else to save");
    // Broken meanwhile: left alone, the window shows the error.
    std::fs::write(&p, BROKEN).unwrap();
    assert_eq!(s.on(Click::ResetTools), vec![]);
    assert!(s.broken());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), BROKEN);
    // No file: nothing to reset, nothing written.
    let gone = dir.join("none.toml");
    let mut s = Settings::new(Config::default(), None, gone.clone());
    assert_eq!(s.on(Click::ResetTools), vec![]);
    assert!(!gone.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file that does not parse opens in the error state: nothing can be
/// written over it, only opened, reset or closed.
#[test]
fn broken_file_is_never_written() {
    let dir = scratch("broken");
    let p = dir.join("config.toml");
    std::fs::write(&p, BROKEN).unwrap();
    let mut s = settings_at(&p);
    assert_eq!(s.modal, Some(Modal::Broken("line 2: expected key = value".into())));
    assert_eq!(broken_text("line 2: expected key = value"), "config.toml has an error (line 2: expected key = value). Fix it or reset it.");
    s.form.theme = 2; // as if edited anyway
    for c in [Click::Ok, Click::Apply, Click::Enter, Click::Save, Click::Back, Click::Restore, Click::ResetAll, Click::Replace, Click::CheckUpdates] {
        assert_eq!(s.on(c), vec![], "{c:?}");
        assert!(s.broken(), "{c:?}");
    }
    assert_eq!(std::fs::read_to_string(&p).unwrap(), BROKEN);
    assert!(!backup_path(&p).exists());
    assert_eq!(s.on(Click::OpenConfig), vec![Act::OpenConfig]);
    assert_eq!(s.on(Click::Close), vec![Act::Close]);
    assert_eq!(s.on(Click::Escape), vec![Act::Close]);
    assert_eq!(s.on(Click::Cancel), vec![Act::Close], "the window's close button");
    // Fixed in an editor: the window reads it again and works.
    std::fs::write(&p, "theme = \"light\"\n").unwrap();
    assert!(s.recheck());
    assert_eq!((s.modal.clone(), s.form.theme, s.stale), (None, 2, false));
    assert!(!s.dirty(), "the form is the fixed file's");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Reset to defaults asks again; Replace keeps the old file as
/// config.toml.bak and writes the defaults; Cancel goes back.
#[test]
fn reset_writes_defaults_and_a_backup() {
    let dir = scratch("reset");
    let p = dir.join("config.toml");
    std::fs::write(&p, BROKEN).unwrap();
    let mut s = settings_at(&p);
    assert_eq!(s.on(Click::ResetDefaults), vec![]);
    assert_eq!(s.modal, Some(Modal::Reset("line 2: expected key = value".into())));
    assert_eq!(s.on(Click::Enter), vec![], "no reset by a stray Enter");
    assert_eq!(s.on(Click::Escape), vec![]);
    assert!(matches!(s.modal, Some(Modal::Broken(_))), "Esc goes back to the error");
    s.on(Click::ResetDefaults);
    assert_eq!(s.on(Click::Back), vec![]);
    assert!(matches!(s.modal, Some(Modal::Broken(_))));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), BROKEN);
    s.on(Click::ResetDefaults);
    assert_eq!(backup_path(&p), dir.join("config.toml.bak"));
    let acts = s.on(Click::Replace);
    assert!(matches!(&acts[..], [Act::Saved(c)] if **c == Config::default()), "{acts:?}");
    assert_eq!(s.modal, None);
    assert_eq!(std::fs::read_to_string(dir.join("config.toml.bak")).unwrap(), BROKEN);
    assert_eq!(config::read_at(&p), Ok(Some(Config::default())));
    // The window works on the new file.
    assert!(!s.dirty());
    s.form.ask = true;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(c)] if c.save_dialog));
    // Broken again and reset again: the first backup is not overwritten.
    std::fs::write(&p, "oops 2\n").unwrap();
    let mut s = settings_at(&p);
    s.on(Click::ResetDefaults);
    assert!(matches!(&s.on(Click::Replace)[..], [Act::Saved(_)]));
    assert_eq!(std::fs::read_to_string(dir.join("config.toml.bak")).unwrap(), BROKEN);
    assert_eq!(std::fs::read_to_string(dir.join("config.toml.bak.1")).unwrap(), "oops 2\n");
    assert_eq!(backup_path(&p), dir.join("config.toml.bak.2"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The file was fixed in an editor while the reset question was open:
/// Replace goes back to the form and writes nothing.
#[test]
fn reset_of_a_fixed_file_writes_nothing() {
    let dir = scratch("reset-fixed");
    let p = dir.join("config.toml");
    std::fs::write(&p, BROKEN).unwrap();
    let mut s = settings_at(&p);
    s.on(Click::ResetDefaults);
    let fixed = "theme = \"light\"\nmystery = 1\n";
    std::fs::write(&p, fixed).unwrap();
    assert_eq!(s.on(Click::Replace), vec![]);
    assert_eq!((s.modal.clone(), s.stale, s.form.theme), (None, false, 2));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), fixed, "not overwritten");
    assert!(!dir.join("config.toml.bak").exists(), "no backup either");
    // Still broken, but differently: the reset goes ahead with the new file.
    std::fs::write(&p, "oops\n").unwrap();
    let mut s = settings_at(&p);
    s.on(Click::ResetDefaults);
    std::fs::write(&p, "still broken\n").unwrap();
    assert!(matches!(&s.on(Click::Replace)[..], [Act::Saved(_)]));
    assert_eq!(std::fs::read_to_string(dir.join("config.toml.bak")).unwrap(), "still broken\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The overlay helpers' settings reach the file: Apply writes them, a
/// fresh read gets them back, and a later save asks no rewrite question.
#[test]
fn overlay_helper_settings_are_saved() {
    let dir = scratch("overlay-helpers");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p);
    s.form.magnifier = false;
    s.form.magnifier_square = true;
    s.form.accept = 1;
    s.form.hints = false;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]));
    let text = std::fs::read_to_string(&p).unwrap();
    let back = config::parse_config(&text).unwrap();
    assert!(!back.magnifier && back.magnifier_square && back.accept_on_select == "copy" && !back.overlay_hints, "{text}");
    assert!(!config::rewrite_loses(&text), "nothing the writer drops");
    assert!(text.contains("accept_on_select = \"copy\""));
    let mut s = settings_at(&p);
    assert_eq!((s.form.magnifier, s.form.magnifier_square, s.form.accept, s.form.hints), (false, true, 1, false));
    s.form.accept = 2;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(c)] if c.accept_on_select == "save"));
    // Copy text: "Let me select" by default; "Copy all at once" is saved.
    let mut s = settings_at(&p);
    assert_eq!(COPY_TEXT[s.form.copy_text].1, "select");
    s.form.copy_text = 0;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(c)] if c.copy_text_mode == "all"));
    assert_eq!(settings_at(&p).form.copy_text, 0);
    // Redact style: Solid by default; Blur is saved.
    let mut s = settings_at(&p);
    assert_eq!(REDACT_STYLES[s.form.redact_style].1, "solid");
    s.form.redact_style = 1;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(c)] if c.redact_style == "blur"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Capture › Image: Trim margins, Add a border (on/off, width, color),
/// HDR screenshots write their keys; a border turned off keeps its color,
/// a color that does not parse is not written.
#[test]
fn image_rows_write_their_keys() {
    let cfg = Config::default();
    let f = Form::new(&cfg, None);
    assert!(!f.trim && !f.border && f.border_w == BORDER_DEFAULT && f.hdr == 0 && f.border_hex.text == "#000000");
    let mut f = Form::new(&cfg, None);
    f.trim = true;
    f.border = true;
    f.border_w = 5;
    f.border_hex.set("#3366ff");
    f.hdr = 1;
    let c = f.apply(&cfg, &cfg);
    assert!(c.trim_margins && c.border_width == 5 && c.border_color == "#3366ff" && c.hdr_screenshots == "keep");
    let back = Form::new(&c, None);
    assert!(back.trim && back.border && back.border_w == 5 && back.hdr == 1);
    assert_eq!(config::to_toml(&back.apply(&c, &c)), config::to_toml(&c), "round trip");
    let mut off = Form::new(&c, None);
    off.border = false;
    off.border_hex.set("blue");
    let c2 = off.apply(&c, &c);
    assert_eq!((c2.border_width, c2.border_color.as_str()), (0, "#3366ff"));
    // In the window: the switches and the segment click.
    let mut s = sample();
    s.section = CAPTURE;
    let mut focus = FocusState::default();
    reveal(&mut s, &mut focus, "trim_margins");
    click(&mut s, &mut focus, "trim_margins", 1.0);
    assert!(s.form.trim);
    // The border and HDR are in Saving › Format.
    s.section = SAVING;
    reveal(&mut s, &mut focus, "border");
    click(&mut s, &mut focus, "border", 1.0);
    assert!(s.form.border);
    assert!(focus.rect_of("border_width").is_some() && focus.rect_of("border_color").is_some());
    if cfg!(windows) {
        reveal(&mut s, &mut focus, "hdr_screenshots");
        let r = focus.rect_of("hdr_screenshots").unwrap();
        let (x, y) = ((r.x1() - 12.0) as i32, (r.y + r.h / 2.0) as i32);
        let mut input = Input::default();
        for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
            input.feed(&e);
        }
        render(&mut s, &DARK, 1.0, &input, &mut focus);
        assert_eq!(s.form.hdr, 1, "Keep as captured");
    } else {
        render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
        assert!(focus.rect_of("hdr_screenshots").is_none(), "Windows only");
    }
    let c = s.form.apply(&s.loaded, &s.loaded);
    assert!(c.trim_margins && c.border_width == u32::from(BORDER_DEFAULT));
}

/// Capture "Mark windows hidden from screenshots": on by default, its
/// switch writes `mark_hidden_windows`; none on Linux (no window there
/// can hide from captures).
#[test]
fn the_hidden_windows_switch_writes_its_key() {
    let cfg = Config::default();
    assert!(Form::new(&cfg, None).mark_hidden, "on by default");
    let mut s = sample();
    s.section = CAPTURE;
    let mut focus = FocusState::default();
    if cfg!(target_os = "linux") {
        render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
        assert!(focus.rect_of("mark_hidden_windows").is_none());
        return;
    }
    reveal(&mut s, &mut focus, "mark_hidden_windows");
    click(&mut s, &mut focus, "mark_hidden_windows", 1.0);
    assert!(!s.form.mark_hidden);
    let c = s.form.apply(&s.loaded, &s.loaded);
    assert!(!c.mark_hidden_windows);
    assert!(config::to_toml(&c).contains("mark_hidden_windows = false"));
    assert!(Form::new(&c, None).apply(&c, &c).mark_hidden_windows == c.mark_hidden_windows, "round trip");
}

/// An Edit window image key on a global hotkey's chord (an old file's
/// Ctrl+Shift+R record hotkey) or on an overlay action's is shown as a
/// conflict.
#[test]
fn image_keys_clash_with_globals_and_overlay_actions() {
    let cfg = Config { record_hotkey: "Ctrl+Shift+R".into(), ..Config::default() };
    let f = Form::new(&cfg, None);
    assert_eq!(f.global_clash(Action::ImageResize), Some("Record video"));
    assert!(f.conflicting().contains(&Action::ImageResize));
    assert_eq!(f.conflict_note(Action::ImageResize).as_deref(), Some("Already used by Record video. Choose another."));
    assert_eq!(f.global_clash(Action::RotateLeft), None);
    assert_eq!(f.global_clash(Action::Redo), None, "only the image keys");
    let mut f = Form::new(&Config::default(), None);
    assert!(f.conflicting().is_empty());
    f.keys.set(Action::Redo, vec![crate::keymap::Chord::parse("Ctrl+R").unwrap()]);
    assert!(f.conflicting().contains(&Action::RotateRight) && f.conflicting().contains(&Action::Redo));
    assert_eq!(f.conflict_note(Action::Redo).as_deref(), Some("Already used by Rotate right. Choose another."));
}

/// Saving "Save at": screen pixels by default; 1× and a custom
/// percent are written and read back; the percent slider is live only
/// for Custom.
#[test]
fn save_at_round_trips() {
    let cfg = Config::default();
    let f = Form::new(&cfg, None);
    assert_eq!((SAVE_AT[f.save_at].1, f.save_pct), ("screen", 50));
    let mut f = Form::new(&cfg, None);
    f.save_at = 2;
    f.save_pct = 75;
    let c = f.apply(&cfg, &cfg);
    assert_eq!((c.save_scale.as_str(), c.save_scale_percent), ("custom", 75));
    let back = Form::new(&c, None);
    assert_eq!((back.save_at, back.save_pct), (2, 75));
    assert_eq!(config::to_toml(&back.apply(&c, &c)), config::to_toml(&c), "round trip");
    let mut one = Form::new(&c, None);
    one.save_at = 1;
    assert_eq!(one.apply(&c, &c).save_scale, "logical");
    // In the window: the segment's "1×", then Custom with its slider.
    let dir = std::env::temp_dir().join(format!("rustshot-saveat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p);
    s.section = SAVING;
    let mut focus = FocusState::default();
    reveal(&mut s, &mut focus, "save_at");
    let r = focus.rect_of("save_at").unwrap();
    let seg = |s: &mut Settings, focus: &mut FocusState, frac: f32| {
        let (x, y) = ((r.x + r.w * frac) as i32, (r.y + r.h / 2.0) as i32);
        let mut input = Input::default();
        for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
            input.feed(&e);
        }
        render(s, &DARK, 1.0, &input, focus);
    };
    seg(&mut s, &mut focus, 0.58);
    assert_eq!(s.form.save_at, 1, "1×");
    seg(&mut s, &mut focus, 0.9);
    assert_eq!(s.form.save_at, 2, "Custom");
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    assert!(focus.rect_of("save_pct").is_some());
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(c)] if c.save_scale == "custom" && c.save_scale_percent == 50));
    assert_eq!(settings_at(&p).form.save_at, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Poll the Upload section's workers until the secret is saved (or 10 s);
/// the window's acts from it.
fn settle_save(s: &mut Settings) -> Vec<Act> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut acts = Vec::new();
    while s.upload.saving() && std::time::Instant::now() < until {
        acts.extend(s.poll_upload().1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    acts
}

/// Upload › S3-compatible: every value saves and reads back; the typed
/// secret (trimmed) goes to the credential store (here the in-memory
/// one) on a worker, under the endpoint, bucket and key id, never into
/// config.toml, and the field empties.
#[test]
fn s3_settings_save_and_the_secret_goes_to_the_store() {
    let dir = scratch("s3-save");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p);
    assert_eq!(s.form.up.dest, 0, "Imgur by default");
    let f = &mut s.form.up;
    f.dest = upload::S3;
    f.endpoint.set(" https://acc.r2.cloudflarestorage.com ");
    f.region.set("auto");
    f.bucket.set("rs-save-test");
    f.access_key.set("AKIDSAVE");
    f.secret.set(" s3cr3t+Key/forTest ");
    f.public_url.set("https://cdn.example.com");
    f.prefix.set("shots/%Y/");
    f.path_style = false;
    f.auto_rec = true;
    assert!(s.dirty());
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]), "{:?}", s.modal);
    assert!(s.upload.saving(), "on a worker, not the window's thread");
    assert!(settle_save(&mut s).is_empty(), "Apply keeps the window open");
    assert!(s.modal.is_none(), "{:?}", s.modal);
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(!text.contains("s3cr3t"), "never in the file: {text}");
    let c = config::parse_config(&text).unwrap();
    assert_eq!(
        (c.upload_destination.as_str(), c.s3_endpoint.as_str(), c.s3_bucket.as_str(), c.s3_access_key_id.as_str()),
        ("s3", "https://acc.r2.cloudflarestorage.com", "rs-save-test", "AKIDSAVE")
    );
    assert_eq!((c.s3_public_url.as_str(), c.s3_key_prefix.as_str(), c.s3_path_style, c.upload_recordings), ("https://cdn.example.com", "shots/%Y/", false, true));
    let name = "rustshot:s3:rs-save-test@acc.r2.cloudflarestorage.com/AKIDSAVE";
    assert_eq!(crate::keystore::os().get(name).unwrap().as_deref(), Some("s3cr3t+Key/forTest"), "trimmed");
    assert!(s.form.up.secret.text.is_empty() && !s.dirty(), "the field empties");
    assert_eq!(s.upload.stored(s.form.up.slot()), Some(true));
    assert!(!format!("{:?}", s.form).contains("s3cr3t"), "never in a Debug print");
    // Read back into a new window.
    let s2 = settings_at(&p);
    assert_eq!((s2.form.up.dest, s2.form.up.bucket.text.as_str(), s2.form.up.path_style), (upload::S3, "rs-save-test", false));
    // OK closes once the secret is kept.
    let mut s = settings_at(&p);
    s.form.up.secret.set("second");
    assert!(matches!(&s.on(Click::Ok)[..], [Act::Saved(_)]), "not closed yet");
    assert_eq!(settle_save(&mut s), vec![Act::Close]);
    assert_eq!(crate::keystore::os().get(name).unwrap().as_deref(), Some("second"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A secret without a bucket or a key id is not kept anywhere: the window
/// says why and keeps it typed.
#[test]
fn a_secret_needs_a_bucket() {
    let dir = scratch("s3-nobucket");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p);
    s.form.up.secret.set("orphan");
    s.on(Click::Apply);
    assert!(matches!(&s.modal, Some(Modal::Error(e)) if e.contains("bucket")), "{:?}", s.modal);
    assert!(!s.upload.saving());
    assert_eq!(s.form.up.secret.text, "orphan");
    assert!(!std::fs::read_to_string(&p).unwrap().contains("orphan"));
    let slot = ("h".to_string(), "b".to_string(), "k".to_string());
    assert!(upload::store_secret(crate::keystore::os(), &slot, " \n").is_ok(), "nothing typed: nothing to do");
    assert_eq!(crate::keystore::os().get("rustshot:s3:b@h/k"), Ok(None));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Whether a secret is stored is asked on a worker, once the slot has
/// stayed as typed for a moment, and remembered per (endpoint, bucket,
/// key id); drawing never asks the store.
#[test]
fn the_stored_check_is_settled_and_cached() {
    let slot = ("acc.r2.cloudflarestorage.com".to_string(), "rs-check".to_string(), "AKC".to_string());
    crate::keystore::os().set("rustshot:s3:rs-check@acc.r2.cloudflarestorage.com/AKC", "", "x").unwrap();
    let mut st = upload::State::default();
    assert_eq!(st.stored(Some(slot.clone())), None, "not known while drawing");
    assert!(!st.poll().changed, "not settled yet: not asked");
    st.settle_now();
    st.poll();
    st.wait();
    assert_eq!(st.stored(Some(slot.clone())), Some(true));
    let other = (slot.0.clone(), slot.1.clone(), "AKOTHER".to_string());
    assert_eq!(st.stored(Some(other.clone())), None, "another key id is another slot");
    st.settle_now();
    st.poll();
    st.wait();
    assert_eq!(st.stored(Some(other)), Some(false));
    assert_eq!(st.stored(None), None);
    assert_eq!(upload::secret_help(false, None, false), format!("Kept in {}", crate::keystore::place()));
    assert!(upload::secret_help(false, Some(true), true).starts_with("Saving"));
}

/// No public URL base: a warning, and the copied link is the endpoint's.
#[test]
fn an_empty_public_url_warns() {
    assert_eq!(upload::public_help(" "), (upload::PUBLIC_WARNING.get(), true));
    assert!(upload::PUBLIC_WARNING.starts_with("Links won't be public unless the bucket is public"));
    assert!(!upload::public_help("https://cdn.example.com").1);
}

/// Test connection runs on a worker with the form's values and says how
/// it went (tests: never over the network, so it fails and says why).
#[test]
fn test_connection_reports_on_the_row() {
    let mut s = sample();
    s.section = S3_STORAGE;
    s.form.up.dest = upload::S3;
    let mut focus = FocusState::default();
    reveal(&mut s, &mut focus, "s3_test");
    assert!(!upload::probe_help(None).1);
    assert_eq!(click(&mut s, &mut focus, "s3_test", 1.0), Some(Click::TestUpload));
    assert_eq!(s.on(Click::TestUpload), vec![]);
    assert_eq!(s.upload.probe, Some(upload::Probe::Running));
    s.upload.wait();
    // No endpoint yet: it says so.
    assert_eq!(s.upload.probe, Some(upload::Probe::Failed("no endpoint is set".into())));
    s.form.up.endpoint.set("https://acc.r2.cloudflarestorage.com");
    s.form.up.bucket.set("rs-probe");
    s.form.up.access_key.set("AK");
    s.form.up.secret.set("typed");
    s.on(Click::TestUpload);
    s.upload.wait();
    // Tried ATTEMPTS times (no waits in tests), then the transport's reason.
    assert_eq!(s.upload.probe, Some(upload::Probe::Failed("upload failed: no network in tests".into())));
    let (text, err) = upload::probe_help(s.upload.probe.as_ref());
    assert_eq!((text.as_str(), err), ("Upload failed: no network in tests", true));
    assert!(!s.upload.poll().changed, "answered once");
}

// The file breaks while the window is open: Save refuses to write
/// (never merges onto the defaults) and shows the error; the edits stay
/// and save once the file is fixed.
#[test]
fn file_broken_before_save_is_not_written() {
    let dir = scratch("broken-later");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config { theme: "dark".into(), ..Config::default() }).unwrap();
    let mut s = settings_at(&p);
    s.form.format = 1;
    std::fs::write(&p, BROKEN).unwrap();
    assert_eq!(s.on(Click::Ok), vec![]);
    assert!(matches!(&s.modal, Some(Modal::Broken(e)) if e.starts_with("line 2")));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), BROKEN);
    // Broken between the rewrite question and its Save, too.
    std::fs::write(&p, "# hand-edited\ntheme = \"dark\"\n").unwrap();
    assert!(s.recheck());
    assert_eq!(s.form.format, 1, "the edit is kept");
    assert_eq!(s.on(Click::Apply), vec![]);
    assert_eq!(s.modal, Some(Modal::Rewrite { close: false }));
    std::fs::write(&p, BROKEN).unwrap();
    assert_eq!(s.on(Click::Save), vec![]);
    assert!(s.broken());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), BROKEN);
    // Fixed: it saves onto the fixed file.
    std::fs::write(&p, "theme = \"light\"\n").unwrap();
    assert!(s.recheck());
    let acts = s.on(Click::Apply);
    assert!(matches!(&acts[..], [Act::Saved(c)] if c.save_format == "jpg" && c.theme == "light"), "{acts:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A message (a hotkey the daemon could not register) shows at once, or
/// after the open question.
#[test]
fn notes_show_in_the_window() {
    let mut s = sample();
    s.notify("one".into());
    assert_eq!(s.modal, Some(Modal::Error("one".into())));
    s.notify("two".into());
    assert_eq!(s.modal, Some(Modal::Error("one\ntwo".into())));
    s.on(Click::Back);
    s.modal = Some(Modal::Rewrite { close: false });
    s.notify("three".into());
    assert_eq!(s.modal, Some(Modal::Rewrite { close: false }));
    s.on(Click::Back);
    assert_eq!(s.modal, Some(Modal::Error("three".into())));
}

#[test]
fn folder_notes() {
    assert_eq!(folder_note(""), None);
    assert!(folder_note("shots").unwrap().contains("relative"));
    assert!(folder_note(r"shots\x").unwrap().contains("relative"));
    let here = std::env::temp_dir().display().to_string();
    assert_eq!(folder_note(&here), None);
    #[cfg(windows)]
    {
        assert_eq!(
            folder_note(r"\shots").as_deref(),
            Some("A folder without a drive is saved relative to the root of the current drive."),
            "no drive"
        );
        assert_eq!(folder_kind(r"C:\Users\x"), FolderKind::Root(PathBuf::from(r"C:\")));
        assert!(!remote_drive(Path::new(r"C:\")), "the system drive is local");
        // A drive letter that is not there (the last one free, if any).
        if let Some(d) = ('D'..='Z').rev().find(|d| !Path::new(&format!(r"{d}:\")).exists()) {
            assert_eq!(folder_note(&format!(r"{d}:\shots")), Some(format!("Drive {d}: is not on this computer.")));
        }
        assert_eq!(folder_note(r"\\server\share\x"), None, "shares are not probed");
    }
    #[cfg(unix)]
    assert_eq!(folder_note("/no-such-root-rustshot/x"), Some("/no-such-root-rustshot does not exist.".into()));
}

/// The drive is probed once per root: typing within it reads the cache.
#[test]
fn folder_probe_is_cached_by_root() {
    let mut s = sample();
    let here = std::env::temp_dir();
    let FolderKind::Root(root) = folder_kind(&here.display().to_string()) else { panic!("an absolute folder") };
    // A cached answer for the root stands in for the disk.
    s.folder_probe = Some((root.clone(), Some("cached".into())));
    for t in [here.display().to_string(), here.join("a").display().to_string(), here.join("ab").display().to_string()] {
        s.form.folder.set(&t);
        assert_eq!(s.folder_note().as_deref(), Some("cached"), "{t}");
    }
    // A relative folder needs no probe and leaves the cache alone.
    s.form.folder.set("shots");
    assert!(s.folder_note().unwrap().contains("relative"));
    assert_eq!(s.folder_probe.as_ref().map(|p| &p.0), Some(&root));
}

/// A hotkey the daemon could not register: a note under its row, and
/// Apply / OK ask the daemon to try again though nothing changed.
#[test]
fn unregistered_hotkey_can_be_retried() {
    use crate::hotkey::{Cause, Failure};
    let dir = scratch("retry");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p);
    assert!(!s.retry_hotkeys());
    assert_eq!((s.hotkey_slot_note(0), s.hotkey_slot_note(1)), (None, None));
    assert_eq!(s.on(Click::Apply), vec![]);
    s.hotkey_failures = vec![
        Failure { slot: 0, wanted: "Ctrl+Alt+F9".into(), cause: Cause::InUse, kept: Some("Ctrl+Alt+Shift+Q".into()), ..Failure::default() },
        Failure { slot: 1, wanted: "Ctrl+Alt+F10".into(), cause: Cause::Unknown, ..Failure::default() },
        Failure { slot: 5, wanted: "Ctrl+Shift+P".into(), cause: Cause::InUse, ..Failure::default() },
    ];
    assert!(s.retry_hotkeys() && !s.dirty());
    assert_eq!(s.hotkey_slot_note(0).as_deref(), Some("In use by another app \u{2014} using Ctrl+Alt+Shift+Q"));
    assert_eq!(s.hotkey_slot_note(1).as_deref(), Some("Not registered"));
    assert_eq!(s.global_message(5).as_deref(), Some("In use by another app. Choose another."));
    assert_eq!(s.on(Click::Apply), vec![Act::Reload]);
    assert_eq!(s.on(Click::Ok), vec![Act::Reload, Act::Close]);
    assert_eq!(s.on(Click::Enter), vec![Act::Reload, Act::Close]);
    assert_eq!(s.on(Click::Cancel), vec![Act::Close], "Cancel never retries");
    // With a change it saves (the save makes the daemon reload anyway).
    s.form.ask = true;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]));
    // Apply is enabled for the retry: a click on it lands.
    let mut s = sample();
    s.hotkey_failures = vec![Failure { slot: 0, wanted: "Ctrl+Alt+F9".into(), cause: Cause::InUse, ..Failure::default() }];
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "apply", 1.0), Some(Click::Apply));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The user's case: Record still unregistered, full screen fallen back
/// (a summary: the key to change first, how to change it once).
const HOTKEY_TEXT: &str = "2 shortcuts are in use by another app\nRecord video: Shift+Win+K \u{2014} not set\nCapture full screen: Shift+Win+F \u{2192} now Shift+Win+Y\nPick another key: click Change shortcuts\u{2026}, click the key and press a new one.";

/// Another app holds a global key: its row shows red with "In use by
/// another app" (and what is used meanwhile, or a free key); a new key
/// recorded in its place clears that until it is saved.
#[test]
fn rows_show_keys_in_use_by_another_app() {
    use crate::hotkey::{Cause, Failure};
    let mut s = sample();
    assert!(!s.global_err(3) && s.global_message(3).is_none());
    s.hotkey_failures = vec![
        Failure { slot: 3, wanted: "Shift+Meta+F".into(), cause: Cause::InUse, alt: Some("Shift+Meta+Y".into()), ..Failure::default() },
        Failure { slot: 2, wanted: "Shift+Meta+G".into(), cause: Cause::InUse, free: Some("Shift+Meta+J".into()), ..Failure::default() },
        Failure { slot: 4, wanted: "Shift+Meta+E".into(), cause: Cause::InUse, ..Failure::default() },
        Failure { slot: 7, wanted: "Shift+Meta+H".into(), cause: Cause::Unknown, ..Failure::default() },
    ];
    let l = crate::hotkey::chord_label;
    assert_eq!(s.global_message(3).unwrap(), format!("In use by another app \u{2014} using {} for now", l("Shift+Meta+Y")));
    assert_eq!(s.global_message(2).unwrap(), format!("In use by another app. {} is free.", l("Shift+Meta+J")));
    assert_eq!(s.global_message(4).as_deref(), Some("In use by another app. Choose another."));
    assert_eq!(s.global_message(7).as_deref(), Some("Not registered"));
    assert!(s.global_err(3) && s.global_err(2) && s.global_err(4));
    assert!(!s.global_err(7) && !s.global_err(0), "not in use: no red");
    // A new key recorded in the row: its own state, not the old failure.
    s.form.full = Chord::parse("Ctrl+Alt+F");
    assert!(!s.global_err(3) && s.global_message(3).is_none());
    // The red keys and the note are drawn.
    let mut s2 = sample();
    s2.section = KEYS_GLOBAL;
    s2.hotkey_failures = s.hotkey_failures.clone();
    let (img, _) = render(&mut s2, &DARK, 1.0, &Input::default(), &mut FocusState::default());
    preview::save("settings-keys-in-use-dark.png", &img);
}

/// The hotkey dialog: OK closes it; "Change shortcuts…" goes to Key
/// bindings with the first row needing a new key recording; a second
/// note joins the open dialog; an error turns into one.
#[test]
fn hotkey_dialog_changes_shortcuts() {
    let mut s = sample();
    s.notify_hotkeys(HOTKEY_TEXT.into(), Some("global2".into()));
    assert_eq!(s.modal, Some(Modal::Hotkeys { text: HOTKEY_TEXT.into(), field: Some("global2".into()) }));
    assert_eq!(s.on(Click::Enter), vec![], "Enter is OK");
    assert_eq!(s.modal, None);
    assert_eq!(s.section, GENERAL);
    // Both buttons are drawn and land.
    s.notify_hotkeys("one".into(), None);
    s.notify_hotkeys("two".into(), Some("global2".into()));
    assert_eq!(s.modal, Some(Modal::Hotkeys { text: "one\ntwo".into(), field: Some("global2".into()) }));
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "q_change", 1.0), Some(Click::ChangeShortcuts));
    assert_eq!(s.on(Click::ChangeShortcuts), vec![]);
    assert_eq!((s.modal.clone(), s.section), (None, KEYS_RECORDING), "Record video is in Shortcuts › Recording");
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    assert!(focus.is_recording("global2"), "the Record video key records");
    assert_eq!(s.record_field, None);
    // An error dialog open: the note joins it, with the button.
    let mut s = sample();
    s.notify("Could not save".into());
    s.notify_hotkeys("x".into(), Some("global3".into()));
    assert_eq!(s.modal, Some(Modal::Hotkeys { text: "Could not save\nx".into(), field: Some("global3".into()) }));
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "q_ok", 1.0), Some(Click::Back));
    // Behind another question: shown (with the button) once it closes.
    let mut s = sample();
    s.modal = Some(Modal::Rewrite { close: false });
    s.notify_hotkeys("later".into(), Some("global0".into()));
    s.on(Click::Back);
    assert_eq!(s.modal, Some(Modal::Hotkeys { text: "later".into(), field: Some("global0".into()) }));
}

/// The daemon's routing: a key the user must change opens the dialog at
/// its row; only fallbacks are a balloon.
#[test]
fn hotkey_notes_route_to_dialog_or_balloon() {
    use crate::editor::{HotkeyNote, hotkey_note};
    use crate::hotkey::{Cause, Failure};
    assert_eq!(hotkey_note(&[]), None);
    let alt = Failure { slot: 3, wanted: "Shift+Meta+F".into(), cause: Cause::InUse, alt: Some("Shift+Meta+Y".into()), ..Failure::default() };
    let stuck = Failure { slot: 2, wanted: "Shift+Meta+K".into(), cause: Cause::InUse, free: Some("Shift+Meta+J".into()), ..Failure::default() };
    assert_eq!(hotkey_note(std::slice::from_ref(&alt)), Some(HotkeyNote::Balloon(alt.message())));
    // A while-recording chord never opens the dialog (it would cover the
    // recording): a note only.
    let rec = Failure { slot: crate::hotkey::REC_SLOT_BASE + 1, wanted: "Ctrl+Alt+Shift+M".into(), cause: Cause::InUse, ..Failure::default() };
    assert!(rec.needs_action());
    assert_eq!(hotkey_note(std::slice::from_ref(&rec)), Some(HotkeyNote::Balloon(rec.message())));
    let both = [alt.clone(), stuck.clone()];
    let Some(HotkeyNote::Dialog { text, field }) = hotkey_note(&both) else { panic!() };
    assert_eq!(field.as_deref(), Some("global2"));
    assert_eq!(text, crate::hotkey::summary(&both, crate::hotkey::DIALOG_LINES));
    #[cfg(windows)]
    assert_eq!(text, HOTKEY_TEXT);
    // Fallbacks only: a balloon, shorter still (Windows keeps 255
    // characters of it), every one counted.
    let alts: Vec<Failure> = [(0, "Shift+Meta+X", "Shift+Meta+Q"), (2, "Shift+Meta+G", "Shift+Meta+J"), (3, "Shift+Meta+F", "Shift+Meta+Y"), (4, "Shift+Meta+E", "Shift+Meta+Q"), (6, "Shift+Meta+Z", "Shift+Meta+Y"), (7, "Shift+Meta+H", "Shift+Meta+J")]
        .map(|(slot, w, a)| Failure { slot, wanted: w.into(), cause: Cause::InUse, alt: Some(a.into()), ..Failure::default() })
        .to_vec();
    for lang in crate::ui::preview::langs() {
        crate::i18n::with_lang(lang, || {
            let Some(HotkeyNote::Balloon(text)) = hotkey_note(&alts) else { panic!("{lang}: a balloon") };
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), 1 + crate::hotkey::BALLOON_LINES, "{lang}: {text}");
            assert_eq!(lines[0], trn!("hotkey.fail.summary_in_use", 6), "{lang}");
            assert_eq!(*lines.last().unwrap(), trn!("hotkey.fail.more", 6 - (crate::hotkey::BALLOON_LINES - 1)), "{lang}");
            assert!(!text.contains(tr!("hotkey.fail.pick_any")), "{lang}: nothing to change");
            assert!(text.encode_utf16().count() < 256, "{lang}: {} UTF-16 units", text.encode_utf16().count());
        });
    }
}

/// The tray-icon-off question: one more line (and "Set up…") while
/// Win+Shift+S does not open Rustshot; "Set up…" keeps the icon and goes
/// to the Win+Shift+S row. Saving the icon on again starts the daemon.
/// Windows only: elsewhere tray off keeps a hidden daemon.
#[cfg(windows)]
#[test]
fn tray_off_question_variants() {
    // Registered and chosen: the plain text, Turn off / Cancel.
    let mut s = sample(); // screenclip on, not waiting
    assert_eq!(s.screenclip_ready(), Some(true));
    assert_eq!(s.hide_tray_text(), HIDE_TRAY);
    s.ask_hide_tray();
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    assert!(focus.rect_of("q_hide").is_some() && focus.rect_of("q_setup").is_none());
    // Registered but Windows still opens the Snipping Tool, or not
    // registered: the setup line and its button.
    for state in [Some(true), Some(false)] {
        let mut s = sample().with_screenclip(state, Handler::None);
        s.read_handler = || Handler::None;
        s.set_screenclip = |_| Ok(()); // never the real registry
        assert_eq!(s.screenclip_ready(), Some(false));
        assert_eq!(s.hide_tray_text(), format!("{HIDE_TRAY}\n{HIDE_TRAY_SETUP}"));
        assert_eq!(HIDE_TRAY_SETUP, "Rustshot isn't set as your screenshot app yet \u{2014} Set up\u{2026}");
        s.form.show_tray = false;
        s.ask_hide_tray();
        let mut focus = FocusState::default();
        assert_eq!(click(&mut s, &mut focus, "q_setup", 1.0), Some(Click::ScreenclipSetup));
        s.on(Click::ScreenclipSetup);
        assert!(s.form.show_tray, "the icon stays for now");
        assert_eq!(s.section, CAPTURE);
        assert!(matches!(s.modal, Some(Modal::Guide { .. })), "the Win+Shift+S guide: {:?}", s.modal);
        s.modal = None; // the guide closed: the row is there
        render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
        assert!(focus.is_focused("screenclip") && s.focus_field.is_none(), "the Win+Shift+S row");
        assert_eq!(s.screenclip, Some(true), "turned on for the guide");
    }
    // Not available here (no handler to set up): nothing extra.
    let s = sample().with_screenclip(None, Handler::Ours);
    assert_eq!((s.screenclip_ready(), s.hide_tray_text()), (None, HIDE_TRAY.to_string()));
    // The row's help.
    assert_eq!(TRAY_HELP, "Off: no background process; only Win+Shift+S / Print Screen (when set up)");
    // Saved off, then on again: the daemon is started (Windows).
    let dir = scratch("trayon");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config { show_tray: false, ..Config::default() }).unwrap();
    let mut s = settings_at(&p);
    s.confirmed = true;
    s.form.show_tray = true;
    let acts = s.on(Click::Apply);
    assert_eq!(acts.contains(&Act::StartDaemon), cfg!(windows), "{acts:?}");
    s.form.ask = !s.form.ask;
    assert!(!s.on(Click::Apply).contains(&Act::StartDaemon), "only when it was turned on");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The one-time notice for an older file with the tray icon off: Set up
/// Win+Shift+S, Show tray icon again (saved; the daemon starts), OK
/// (closes). Turning it off here marks the file, so it never shows.
/// The Store build keeps a hidden daemon: its question and help say so.
#[test]
fn tray_off_notice_and_store_build() {
    assert_eq!(TRAY_OFF_NOTICE, "Rustshot no longer runs in the background while the tray icon is off. Win+Shift+S and Print Screen can still open it.");
    let dir = scratch("traynotice");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config { show_tray: false, ..Config::default() }).unwrap();
    let mut s = settings_at(&p);
    s.no_background = true;
    s.confirmed = true;
    s.show_tray_off_notice();
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    for id in ["q_setup", "q_tray_on", "q_ok"] {
        assert!(focus.rect_of(id).is_some(), "{id}");
    }
    assert_eq!(s.on(Click::Enter), vec![Act::Close], "Enter is OK");
    s.show_tray_off_notice();
    let acts = s.on(Click::ShowTrayAgain);
    assert!(matches!(&acts[..], [Act::Saved(c), Act::StartDaemon, Act::Close] if c.show_tray), "{acts:?}");
    assert!(config::read_at(&p).unwrap().unwrap().show_tray);
    // Turned off here: marked (the notice is for older files only).
    s.form.show_tray = false;
    s.on(Click::Apply);
    let saved = config::read_at(&p).unwrap().unwrap();
    assert!(!saved.show_tray && saved.tray_off_mode_seen);
    // The Store build: the icon off keeps the daemon; nothing to start.
    let mut s = sample();
    s.no_background = false;
    assert_eq!(s.hide_tray_text(), HIDE_TRAY_KEEP);
    assert_eq!(s.tray_help(), TRAY_HELP_KEEP);
    s.no_background = true;
    assert_eq!(s.tray_help(), TRAY_HELP);
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Change shortcuts…" opens the shortcut section that lists the row,
/// its key box recording: a while-recording chord, a global, an overlay
/// tool or action key.
#[test]
fn change_shortcuts_reaches_every_row() {
    for (field, sec) in [("key_rec_mute", KEYS_RECORDING), ("global3", KEYS_GLOBAL), ("global2", KEYS_RECORDING), ("key_tool_text", KEYS_TOOLS), ("key_undo", KEYS_EDITING), ("key_accept", KEYS_ACTIONS), ("key_zoom_in", KEYS_EDIT_WINDOW)] {
        let mut s = sample();
        s.notify_hotkeys("x".into(), Some(field.into()));
        s.on(Click::ChangeShortcuts);
        assert_eq!((s.section, s.record_field.as_deref()), (sec, Some(field)), "{field}");
        let mut focus = FocusState::default();
        render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
        assert!(focus.is_recording(field), "{field}");
    }
    assert_eq!(section_of_field("nonsense"), KEYS_GLOBAL);
}

#[test]
fn apply_and_ok_save_only_when_dirty() {
    let dir = scratch("apply");
    let p = dir.join("config.toml");
    config::save_at(&p, &Config::default()).unwrap();
    let mut s = settings_at(&p);
    assert!(!s.dirty());
    assert_eq!(s.on(Click::Apply), vec![]);
    s.form.format = 1;
    assert!(s.dirty());
    let acts = s.on(Click::Apply);
    assert!(matches!(&acts[..], [Act::Saved(c)] if c.save_format == "jpg"), "{acts:?}");
    assert!(!s.dirty());
    assert_eq!(config::parse_config(&std::fs::read_to_string(&p).unwrap()).unwrap().save_format, "jpg");
    // OK with nothing changed just closes; with a change it saves and closes.
    assert_eq!(s.on(Click::Ok), vec![Act::Close]);
    s.form.quality = 40;
    let acts = s.on(Click::Enter);
    assert!(matches!(&acts[..], [Act::Saved(c), Act::Close] if c.jpeg_quality == 40), "{acts:?}");
    // Cancel / Esc / the close button discard.
    s.form.quality = 41;
    assert_eq!(s.on(Click::Escape), vec![Act::Close]);
    assert_eq!(config::parse_config(&std::fs::read_to_string(&p).unwrap()).unwrap().jpeg_quality, 40);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn first_save_over_comments_asks_once() {
    let dir = scratch("confirm");
    let p = dir.join("config.toml");
    std::fs::write(&p, "# my settings\ntheme = \"dark\"\nmystery = 1\n").unwrap();
    let mut s = settings_at(&p);
    s.form.ask = true;
    // Asked; Cancel leaves the file alone.
    assert_eq!(s.on(Click::Apply), vec![]);
    assert_eq!(s.modal, Some(Modal::Rewrite { close: false }));
    assert_eq!(s.on(Click::Escape), vec![]);
    assert_eq!(s.modal, None);
    assert!(std::fs::read_to_string(&p).unwrap().starts_with("# my settings"));
    // OK asks too; Save (Enter) writes and closes.
    assert_eq!(s.on(Click::Ok), vec![]);
    assert_eq!(s.modal, Some(Modal::Rewrite { close: true }));
    let acts = s.on(Click::Enter);
    assert!(matches!(&acts[..], [Act::Saved(_), Act::Close]), "{acts:?}");
    let text = std::fs::read_to_string(&p).unwrap();
    let c = config::parse_config(&text).unwrap();
    assert!(c.save_dialog && c.theme == "dark", "values kept");
    assert!(!text.contains("# my settings") && !text.contains("mystery"));
    // Not asked again, even if the file gains a comment meanwhile.
    std::fs::write(&p, format!("# again\n{text}")).unwrap();
    s.form.ask = false;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]));
    // A file rustshot wrote itself is never asked about.
    let mut s = settings_at(&p);
    s.form.theme = 2;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]));
    // Neither is a missing file (it is created).
    let q = dir.join("new").join("config.toml");
    let mut s = settings_at(&q);
    s.form.theme = 2;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]));
    assert_eq!(config::parse_config(&std::fs::read_to_string(&q).unwrap()).unwrap().theme, "light");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_errors_show_in_the_window() {
    let dir = scratch("err");
    // The config's folder cannot be created (a file is in the way).
    std::fs::write(dir.join("blocker"), "").unwrap();
    let mut s = settings_at(&dir.join("blocker").join("sub").join("config.toml"));
    assert_eq!(s.modal, None, "no file there yet");
    s.form.theme = 1;
    assert_eq!(s.on(Click::Ok), vec![]);
    assert!(matches!(&s.modal, Some(Modal::Error(e)) if e.starts_with("Could not save")));
    assert!(s.dirty(), "nothing was saved");
    assert_eq!(s.on(Click::Enter), vec![], "Enter dismisses the error");
    assert_eq!(s.modal, None);
    // Launch at login failing after the file was saved: the error shows,
    // the window stays.
    let p = dir.join("config.toml");
    let mut s = settings_at(&p);
    s.autostart = Some(false);
    s.form.autostart = Some(true);
    let acts = s.on(Click::Ok);
    assert!(matches!(&acts[..], [Act::Saved(_)]), "{acts:?}");
    assert!(matches!(&s.modal, Some(Modal::Error(e)) if e.contains("Launch at login")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restore_clear_reset_and_links() {
    let mut s = settings_at(&scratch("rc").join("config.toml"));
    s.on(Click::Clear);
    assert_eq!(s.form.filename.text, "");
    s.on(Click::Restore);
    assert_eq!(s.form.filename.text, "%F_%H-%M");
    s.form.keys.set(Action::Save, vec![]);
    s.form.capture = None;
    s.on(Click::ResetAll);
    assert_eq!(s.form.keys, Keymap::defaults());
    assert_eq!(s.form.capture, Chord::parse("Shift+Meta+X"));
    assert_eq!(s.on(Click::Browse), vec![Act::Browse]);
    assert_eq!(s.on(Click::OpenConfig), vec![Act::OpenConfig]);
    assert_eq!(s.on(Click::CheckUpdates), vec![Act::CheckUpdates]);
    assert_eq!(s.on(Click::Link(crate::actions::POWERNAPPS)), vec![Act::Link(crate::actions::POWERNAPPS)]);
}

/// Capture › History and the History hotkey row write their keys; a
/// value the choices do not have stays as it is.
#[test]
fn history_rows_write_their_keys() {
    let cfg = Config::default();
    let mut f = Form::new(&cfg, None);
    assert!(f.history && HISTORY_DAYS[f.history_days].0 == 30 && HISTORY_MB[f.history_size].0 == 2048);
    f.history = false;
    f.history_days = 0;
    f.history_size = 3;
    *f.global_mut(7) = Chord::parse("Ctrl+Alt+H");
    let c = f.apply(&cfg, &cfg);
    assert_eq!((c.history, c.history_days, c.history_max_mb, c.history_hotkey.as_str()), (false, 7, 10240, "Ctrl+Alt+H"));
    let odd = Config { history_days: 12, ..Config::default() };
    let f = Form::new(&odd, None);
    assert_eq!(f.history_days, usize::MAX);
    assert_eq!(f.apply(&odd, &odd).history_days, 12);
    assert!(GLOBALS.iter().any(|(l, s)| *l == "Open History" && *s == 7));
}

/// "Show icon in tray" off with no capture hotkey (or one that did not
/// register): the row warns.
#[test]
fn hidden_tray_without_a_hotkey_warns() {
    let mut s = sample();
    assert!(!s.tray_warning());
    s.form.show_tray = false;
    assert!(!s.tray_warning(), "the capture hotkey still works");
    s.form.capture = None;
    assert!(s.tray_warning());
    s.form.capture = Chord::parse("Ctrl+Shift+S");
    s.hotkey_failures = vec![crate::hotkey::Failure { slot: 0, wanted: "Ctrl+Shift+S".into(), cause: crate::hotkey::Cause::InUse, ..Default::default() }];
    assert!(s.tray_warning());
}

// --- Rendering ---

/// Two frames (focus order, then the picture); returns the image and the
/// click of the second.
fn render(s: &mut Settings, th: &Theme, k: f32, input: &Input, focus: &mut FocusState) -> (PixBuf, Option<Click>) {
    preview::render(W, H, k, th, &Input::default(), focus, |ui| {
        draw::paint(ui, s);
    });
    let mut click = None;
    let img = preview::render(W, H, k, th, input, focus, |ui| click = draw::paint(ui, s));
    (img, click)
}

/// A click at the centre of control `id` as last drawn.
fn click(s: &mut Settings, focus: &mut FocusState, id: &str, k: f32) -> Option<Click> {
    render(s, &DARK, k, &Input::default(), focus);
    let r = focus.rect_of(id).unwrap_or_else(|| panic!("{id} drawn"));
    let (x, y) = ((r.x + r.w / 2.0) as i32, (r.y + r.h / 2.0) as i32);
    let mut input = Input::default();
    for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
        input.feed(&e);
    }
    render(s, &DARK, k, &input, focus).1
}

/// Draw the section and check control `id` is in it (nothing scrolls:
/// every control is in view).
fn reveal(s: &mut Settings, focus: &mut FocusState, id: &str) {
    render(s, &DARK, 1.0, &Input::default(), focus);
    assert!(focus.rect_of(id).is_some(), "{id} drawn in section {}", s.section);
}

/// A click near the right end of control `id` (a segmented control's
/// last choice).
fn click_right(s: &mut Settings, focus: &mut FocusState, id: &str) {
    render(s, &DARK, 1.0, &Input::default(), focus);
    let r = focus.rect_of(id).unwrap_or_else(|| panic!("{id} drawn"));
    let (x, y) = ((r.x1() - 12.0) as i32, (r.y + r.h / 2.0) as i32);
    let mut input = Input::default();
    for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
        input.feed(&e);
    }
    render(s, &DARK, 1.0, &input, focus);
}

fn press(vk: u32, mods: wind::Mods) -> Input {
    let mut input = Input::default();
    input.feed(&Ev::Key { vk, up: false, repeat: false, mods });
    input
}

fn preview_cfg() -> Config {
    // An absolute path on every platform, so no folder note shifts the layout.
    let save_path = if cfg!(windows) { r"C:\Users\denis\Pictures\rustshot" } else { "/home/denis/Pictures/rustshot" };
    Config { save_path: save_path.into(), copy_after_capture: true, ..Config::default() }
}

fn sample() -> Settings {
    let mut s = Settings::new(preview_cfg(), Some(true), PathBuf::from("config.toml")).with_screenclip(Some(true), Handler::Ours);
    s.read_handler = || Handler::Ours;
    s.renderer = true;
    s.tray_row = true;
    s.rec_lists.linux = false;
    s
}

/// The name of a preview PNG: `settings-<what>-<theme>[-150].png`.
fn png(what: &str, tname: &str, k: f32) -> String {
    if k == 1.0 { format!("settings-{what}-{tname}.png") } else { format!("settings-{what}-{tname}-{}.png", (k * 100.0) as u32) }
}

/// A section's name in file names: "shortcuts-overlay-tools".
fn slug(i: usize) -> String {
    TITLES[i].to_lowercase().replace(" \u{b7} ", "-").replace(' ', "-")
}

/// The window with every note and warning a section can show at once
/// (the longest it gets): a relative save folder, the tray hidden
/// without a working capture hotkey (Win+Shift+S's red line under its
/// toggle), the Linux encoder row, no camera, S3 chosen with no public
/// URL and a failed test, every global hotkey taken by another app, and
/// every overlay and while-recording key on the same bare P.
fn crowded() -> Settings {
    use crate::hotkey::{Cause, Failure};
    let mut s = sample().with_screenclip(Some(true), Handler::Broken { progid: "AppXgone".into() });
    s.form.folder.set("shots");
    s.form.show_tray = false;
    s.tools_reset = true;
    s.form.border = true;
    s.rec_lists.linux = true;
    s.rec_lists.found = Some(recording::Found::default());
    s.form.rec.format = 1;
    s.form.up.dest = upload::S3;
    s.upload.probe = Some(upload::Probe::Failed("upload failed: the bucket does not exist or this access key cannot write to it (NoSuchBucket)".into()));
    s.hotkey_failures = (0..hotkey::SLOTS)
        .map(|slot| Failure { slot, wanted: "Ctrl+Shift+S".into(), cause: Cause::InUse, kept: None, alt: None, free: Some("Ctrl+Alt+Shift+F12".into()) })
        .collect();
    let p = Chord::parse("P").unwrap();
    for a in Action::ALL {
        s.form.keys.set(a, vec![p]);
    }
    s
}

/// General › Language: a language not downloaded yet shows its
/// download's state under the row, and Retry after a failure.
#[test]
fn language_row_shows_the_download() {
    use crate::i18n::fetch::{set_status, Status};
    let i = language_choices().iter().position(|c| c.0 == "zz-ui").unwrap();
    for (st, retry) in [(Status::Failed("offline".into()), true), (Status::Downloading, false)] {
        set_status("zz-ui", st.clone());
        let mut s = sample();
        s.section = GENERAL;
        s.form.language = i;
        let mut focus = FocusState::default();
        let (img, _) = render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
        assert_eq!(focus.rect_of("language_retry").is_some(), retry, "{st:?}");
        preview::save(&format!("settings-language-{}.png", if retry { "failed" } else { "downloading" }), &img);
    }
}

/// A right-to-left language mirrors the window: the rail on the right
/// (its selected item where the English one is, mirrored), while a long
/// left-to-right language does not move it; a click lands on the control
/// drawn under it.
#[test]
fn right_to_left_mirrors_the_window() {
    let shot = |lang: &str| {
        crate::i18n::with_lang(lang, || {
            let mut s = sample();
            s.section = GENERAL;
            render(&mut s, &DARK, 1.0, &Input::default(), &mut FocusState::default()).0
        })
    };
    let (en, qps, rtl) = (shot("en"), shot("qps"), shot("qps-rtl"));
    let px = |img: &PixBuf, x: u32, y: u32| img.get_pixel(x, y);
    // The selected rail item's fill, left of its icon.
    let (x, y) = (14, 64);
    assert_eq!(px(&en, x, y), px(&qps, x, y));
    assert_eq!(px(&en, x, y), px(&rtl, W - 1 - x, y), "mirrored");
    assert_ne!(px(&en, x, y), px(&rtl, x, y));
    // The pointer is mirrored too: a press where the Theme control is
    // drawn (left in a right-to-left window) reaches it.
    crate::i18n::with_lang("qps-rtl", || {
        let mut s = sample();
        s.section = GENERAL;
        let mut focus = FocusState::default();
        render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
        let r = focus.rect_of("language").expect("the language dropdown");
        let screen_x = W as i32 - 1 - (r.x + r.w / 2.0) as i32;
        let ev = crate::ui::rtl_ev(crate::wind::Ev::Down { x: screen_x, y: (r.y + r.h / 2.0) as i32 }, W);
        let crate::wind::Ev::Down { x, .. } = ev else { unreachable!() };
        assert!((x as f32) >= r.x && (x as f32) < r.x1(), "{x} in {r:?}");
    });
}

/// The most hotkeys that can fail at once (every global and every
/// while-recording one).
const MAX_FAILURES: usize = hotkey::SLOTS + hotkey::REC_SLOTS;

/// The first `n` of every hotkey failing at once (the defaults taken by
/// other apps): fallbacks, keys left unset, a kept one, an unconfirmed
/// one and the while-recording keys. 1: Quit unset (the dialog); 4: the
/// user's report (Capture region and Record video on fallbacks, Quit and
/// full screen unset).
fn conflicts(n: usize) -> Vec<crate::hotkey::Failure> {
    use crate::hotkey::{Cause, Failure, REC_SLOT_BASE};
    let f = |slot: usize, wanted: &str, cause: Cause| Failure { slot, wanted: wanted.into(), cause, ..Failure::default() };
    let all = [
        f(1, "Ctrl+Alt+Shift+Q", Cause::InUse),
        Failure { alt: Some("Shift+Meta+Q".into()), ..f(0, "Shift+Meta+X", Cause::InUse) },
        Failure { alt: Some("Shift+Meta+J".into()), ..f(2, "Shift+Meta+G", Cause::InUse) },
        Failure { free: Some("Shift+Meta+Y".into()), ..f(3, "Shift+Meta+F", Cause::InUse) },
        f(4, "Shift+Meta+E", Cause::InUse),
        Failure { kept: Some("Ctrl+Alt+R".into()), ..f(5, "Ctrl+Shift+R", Cause::InUse) },
        f(6, "Shift+Meta+Z", Cause::Unknown),
        f(7, "Shift+Meta+H", Cause::InUse),
        f(8, "Shift+Meta+O", Cause::InUse),
        f(REC_SLOT_BASE, "Ctrl+Alt+Shift+P", Cause::InUse),
        f(REC_SLOT_BASE + 1, "Ctrl+Alt+Shift+M", Cause::InUse),
        f(REC_SLOT_BASE + 2, "Ctrl+Alt+Shift+H", Cause::InUse),
    ];
    assert_eq!(all.len(), MAX_FAILURES);
    all[..n].to_vec()
}

/// The hotkey dialog over Shortcuts › Global with 1, 4 and every hotkey
/// taken by other apps (the most that fail at once) fits the window in
/// English, the long pseudo-languages and the RUSTSHOT_FIT_LANGS
/// catalogs: no line cut, its buttons inside the window. One failure
/// keeps its sentence; several are a heading, a short line each (past
/// [`hotkey::DIALOG_LINES`], "and N more") and how to pick a key once
/// (RUSTSHOT_PREVIEW_DIR: `after/conflicts-<n>-<lang>.png`).
#[test]
fn hotkey_conflicts_fit() {
    use crate::editor::{HotkeyNote, hotkey_note};
    use crate::hotkey::DIALOG_LINES;
    for lang in preview::langs() {
        crate::i18n::with_lang(lang, || {
            for n in [1, 4, MAX_FAILURES] {
                let failed = conflicts(n);
                let Some(HotkeyNote::Dialog { text, field }) = hotkey_note(&failed) else { panic!("{lang} {n}: the dialog") };
                let lines: Vec<&str> = text.lines().collect();
                if n == 1 {
                    assert_eq!(text, failed[0].message(), "{lang}: one failure keeps its sentence");
                } else {
                    let in_use = failed.iter().all(|f| f.cause == hotkey::Cause::InUse);
                    let head = if in_use { trn!("hotkey.fail.summary_in_use", n) } else { trn!("hotkey.fail.summary", n) };
                    assert_eq!(lines[0], head, "{lang} {n}: the heading");
                    assert_eq!(text.matches(tr!("hotkey.fail.pick_any")).count(), 1, "{lang} {n}: how to pick a key, once");
                    assert_eq!(*lines.last().unwrap(), tr!("hotkey.fail.pick_any"), "{lang} {n}");
                    let shown = n.min(DIALOG_LINES - usize::from(n > DIALOG_LINES));
                    assert_eq!(lines.len(), 1 + shown + usize::from(n > DIALOG_LINES) + 1, "{lang} {n}: {text}");
                    if n > DIALOG_LINES {
                        assert_eq!(lines[lines.len() - 2], trn!("hotkey.fail.more", n - shown), "{lang} {n}");
                    }
                    // The keys to change come first.
                    assert!(lines[1].contains(&crate::hotkey::chord_label("Ctrl+Alt+Shift+Q")), "{lang} {n}: {}", lines[1]);
                }
                for k in [1.0f32, 2.0] {
                    let mut s = sample();
                    s.section = KEYS_GLOBAL;
                    s.hotkey_failures = failed.iter().filter(|f| f.slot < hotkey::SLOTS).cloned().collect();
                    s.modal = Some(Modal::Hotkeys { text: text.clone(), field: field.clone() });
                    let mut focus = FocusState::default();
                    let (img, _) = render(&mut s, &DARK, k, &Input::default(), &mut focus);
                    if k == 1.0 {
                        let tag = if n == MAX_FAILURES { "max".to_string() } else { n.to_string() };
                        preview::save(&format!("after/conflicts-{tag}-{lang}.png"), &img);
                    }
                    // No text cut or past the edge; the buttons inside the
                    // window, side by side (the page's rows lie under the
                    // dimming).
                    let what = format!("{lang}: {n} hotkeys taken, at {k}");
                    let problems = preview::take_problems();
                    assert!(problems.is_empty(), "{what}: {problems:#?}");
                    let b = ["q_ok", "q_change"].map(|id| focus.rect_of(id).unwrap_or_else(|| panic!("{what}: {id}"))).map(|r| FRect { x: r.x / k, y: r.y / k, w: r.w / k, h: r.h / k });
                    for r in &b {
                        assert!(r.x >= 0.0 && r.y >= 0.0 && r.x1() <= W as f32 + 0.5 && r.y1() <= H as f32 + 0.5, "{what}: {r:?}");
                    }
                    assert!(b[0].x1().min(b[1].x1()) - b[0].x.max(b[1].x) <= 1.0, "{what}: {b:?} overlap");
                }
            }
        });
    }
}

/// A note too tall for the window (notes joined to the hotkey dialog)
/// keeps whole lines: its first, as many as fit, "and N more", its last.
#[test]
fn joined_notes_fit_in_whole_lines() {
    let text = (0..60).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let mut s = sample();
    s.modal = Some(Modal::Hotkeys { text: text.clone(), field: None });
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    // Nothing cut: whole lines left out (and noted as such).
    let problems = preview::take_problems();
    assert!(!problems.is_empty() && problems.iter().all(|p| p.contains("lines left out")), "60 lines: {problems:#?}");
    let drawn: Vec<String> = preview::drawn_texts().into_iter().map(|t| t.0).collect();
    assert!(drawn.iter().any(|t| t == "line 0") && drawn.iter().any(|t| t == "line 59"), "{drawn:?}");
    let more = drawn.iter().find(|t| t.starts_with("and ")).expect("and N more");
    let kept = drawn.iter().filter(|t| t.starts_with("line ")).count();
    assert_eq!(*more, trn!("hotkey.fail.more", 60 - kept), "{drawn:?}");
    // Short enough: as it is.
    assert_eq!(super::draw::fit_lines("a\nb\nc", 100.0, |_| 60.0), "a\nb\nc");
}

/// Every question and message of the window fits it in English and the
/// long pseudo-languages: its buttons and checkbox inside the window,
/// none over another (RUSTSHOT_PREVIEW_DIR: each, in each language).
#[test]
fn dialogs_fit_in_long_languages() {
    let modals = || {
        [
            Modal::Rewrite { close: false },
            Modal::Error("Could not save C:\\Users\\someone\\AppData\\Roaming\\rustshot\\config.toml: access is denied".into()),
            Modal::Hotkeys { text: "Win+Shift+S is used by another app.".into(), field: None },
            Modal::HideTray,
            Modal::TrayOffNotice,
            Modal::Broken("line 3: expected key = value".into()),
            Modal::Reset("line 3".into()),
            Modal::Guide { broken: true, dont_show: false },
        ]
    };
    for lang in crate::ui::preview::langs() {
        crate::i18n::with_lang(lang, || {
            for k in [1.0f32, 2.0] {
                for (n, m) in modals().into_iter().enumerate() {
                    let mut s = sample();
                    s.modal = Some(m);
                    let mut focus = FocusState::default();
                    let (img, _) = render(&mut s, &DARK, k, &Input::default(), &mut focus);
                    if k == 1.0 {
                        preview::save(&format!("settings-dialog-{n}-{lang}.png"), &img);
                    }
                    let rects: Vec<FRect> = focus.all_rects().into_iter().map(|r| FRect { x: r.x / k, y: r.y / k, w: r.w / k, h: r.h / k }).collect();
                    assert!(!rects.is_empty(), "{lang} dialog {n}: no buttons");
                    for (a, ra) in rects.iter().enumerate() {
                        assert!(ra.x >= 0.0 && ra.y >= 0.0 && ra.x1() <= W as f32 + 0.5 && ra.y1() <= H as f32 + 0.5, "{lang} dialog {n} at {k}: {ra:?}");
                        for rb in &rects[a + 1..] {
                            let ow = ra.x1().min(rb.x1()) - ra.x.max(rb.x);
                            let oh = ra.y1().min(rb.y1()) - ra.y.max(rb.y);
                            assert!(ow <= 1.0 || oh <= 1.0, "{lang} dialog {n} at {k}: {ra:?} and {rb:?} overlap");
                        }
                    }
                }
            }
        });
    }
}

/// The cost of a frame in each language (General, warm caches):
/// `cargo test --release language_frame_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark"]
fn language_frame_cost() {
    for lang in ["en", "qps", "qps-rtl", "qps-ar"] {
        crate::i18n::with_lang(lang, || {
            let mut s = sample();
            s.section = GENERAL;
            render(&mut s, &DARK, 1.5, &Input::default(), &mut FocusState::default());
            let t = std::time::Instant::now();
            let n = 50;
            for _ in 0..n {
                render(&mut s, &DARK, 1.5, &Input::default(), &mut FocusState::default());
            }
            eprintln!("{lang}: {:.2} ms a frame", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        });
    }
}

/// The Microsoft Store screenshot: Shortcuts › Global, dark, at 150 %, in
/// each of [`preview::store_langs`] (`<lang>/`, RUSTSHOT_PREVIEW_DIR).
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn store_screenshot_settings() {
    for lang in preview::store_langs() {
        crate::i18n::with_lang(lang, || {
            let mut s = sample();
            s.section = KEYS_GLOBAL;
            let (img, _) = render(&mut s, &DARK, 1.5, &Input::default(), &mut FocusState::default());
            preview::save_lang(lang, "settings-shortcuts-global-dark-150.png", &img);
        });
    }
}

/// macOS (board 12): Settings' surface runs to the top edge under the
/// transparent title bar. The layout starts below its 28 px strip (the
/// rail's mark and the header's title drop clear of the traffic lights)
/// and the window is that much taller, so every section fits as it does
/// elsewhere; the strip continues the rail, its border and the header,
/// and the pointer lands where the layout drew. Writes
/// `settings-macos-*.png`.
#[test]
fn settings_under_the_macos_title_bar() {
    let top = crate::wind::Overlap::MACOS.top as usize;
    let row = |img: &PixBuf, y: usize| img.as_raw()[y * W as usize * 4..(y + 1) * W as usize * 4].to_vec();
    for (tn, th) in [("dark", &DARK), ("light", &LIGHT)] {
        for i in [GENERAL, ABOUT] {
            let mut under = PixBuf::new(W, H + top as u32);
            let mut s = sample();
            s.section = i;
            super::frame(&mut under, th, &Input::default(), &mut FocusState::default(), &mut s, 1.0, top);
            let mut plain = PixBuf::new(W, H);
            let mut s = sample();
            s.section = i;
            super::frame(&mut plain, th, &Input::default(), &mut FocusState::default(), &mut s, 1.0, 0);
            assert!(under.as_raw()[top * W as usize * 4..] == *plain.as_raw(), "{tn} {i}: the layout below the strip is the usual one");
            for y in 0..top {
                assert!(row(&under, y) == row(&plain, 0), "{tn} {i}: strip row {y} continues the top row");
            }
            if i == GENERAL {
                let mut img = under.clone();
                crate::ui::preview::traffic_lights(&mut img, 1.0);
                crate::ui::preview::save(&format!("settings-macos-{tn}.png"), &img);
            }
        }
    }
    // The pointer in the layout's coordinates.
    match crate::ui::lift_ev(Ev::Down { x: 5, y: 40 }, top as i32) {
        Ev::Down { x, y } => assert_eq!((x, y), (5, 40 - top as i32)),
        e => panic!("{e:?}"),
    }
    assert!(matches!(crate::ui::lift_ev(Ev::Timer, 28), Ev::Timer));
    assert_eq!(super::win_h(), H + crate::wind::Overlap::native().top as u32);
}

/// Every section in the pseudo-languages and the RUSTSHOT_FIT_LANGS
/// catalogs (RUSTSHOT_PREVIEW_DIR).
#[test]
fn preview_settings_pseudo_pngs() {
    for lang in ["qps", "qps-rtl"].into_iter().chain(crate::i18n::fit_langs()) {
        crate::i18n::with_lang(lang, || {
            for i in 0..SECTIONS.len() {
                let mut s = sample();
                s.section = i;
                let (img, _) = render(&mut s, &DARK, 1.0, &Input::default(), &mut FocusState::default());
                preview::save(&format!("settings-{i:02}-{lang}.png"), &img);
            }
        });
    }
}

#[test]
fn preview_settings_pngs() {
    for k in [1.0f32, 1.5] {
        for (tname, th) in [("dark", &DARK), ("light", &LIGHT)] {
            for i in 0..SECTIONS.len() {
                let mut s = sample();
                s.section = i;
                let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
                preview::save(&png(&slug(i), tname, k), &img);
                if k == 1.0 {
                    let mut s = crowded();
                    s.section = i;
                    let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
                    preview::save(&png(&format!("{}-crowded", slug(i)), tname, k), &img);
                }
            }
            // Global shortcuts as on the board: full screen recording,
            // repeat last capture in use by another app.
            let mut s = sample();
            s.section = KEYS_GLOBAL;
            s.form.repeat = Chord::parse("Ctrl+Shift+P");
            s.loaded.repeat_hotkey = "Ctrl+Shift+P".into();
            s.hotkey_failures = vec![crate::hotkey::Failure { slot: 5, wanted: "Ctrl+Shift+P".into(), cause: crate::hotkey::Cause::InUse, ..Default::default() }];
            let mut focus = FocusState::default();
            render(&mut s, th, k, &Input::default(), &mut focus);
            focus.record("global3");
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut focus);
            preview::save(&png("keys-board", tname, k), &img);
            // An overlay conflict (Cancel on P), and recording a tool key.
            for (sec, name) in [(KEYS_TOOLS, "keys-conflict"), (KEYS_ACTIONS, "keys-actions-conflict")] {
                let mut s = sample();
                s.section = sec;
                s.form.keys.set(Action::Cancel, vec![Chord::parse("P").unwrap()]);
                let mut focus = FocusState::default();
                render(&mut s, th, k, &Input::default(), &mut focus);
                focus.record(if sec == KEYS_TOOLS { "key_tool_text" } else { "key_copy" });
                let (img, _) = render(&mut s, th, k, &Input::default(), &mut focus);
                preview::save(&png(name, tname, k), &img);
            }
            // General: a custom accent, the tray hidden without a hotkey.
            let mut s = sample();
            s.form.accent = ACCENT_CUSTOM;
            s.form.accent_hex.set("#e07a4f");
            s.form.show_tray = false;
            s.form.capture = None;
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("general-states", tname, k), &img);
            // Saving: a drive that is not there, JPEG chosen, a 4 px border.
            let mut s = sample();
            s.section = SAVING;
            s.form.format = 1;
            s.form.border = true;
            s.form.border_w = 4;
            s.form.border_hex.set("#f04438");
            s.form.folder.set(if cfg!(windows) { r"Q:\shots" } else { "/no-such-root/shots" });
            let root = PathBuf::from(if cfg!(windows) { r"Q:\" } else { "/no-such-root" });
            s.folder_probe = Some((root, Some("Drive Q: is not on this computer.".into())));
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("saving-states", tname, k), &img);
            // Win+Shift+S's line under its toggle, per state.
            for (name, h) in [
                ("capture-screenclip", Handler::Ours),
                ("capture-screenclip-other", Handler::Other { progid: "AppXsnip".into(), name: "Snipping Tool".into() }),
                ("capture-screenclip-broken", Handler::Broken { progid: "AppXgone".into() }),
                ("capture-screenclip-unset", Handler::None),
            ] {
                let mut s = sample().with_screenclip(Some(true), h);
                s.section = CAPTURE;
                let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
                preview::save(&png(name, tname, k), &img);
            }
            let mut s = sample().with_screenclip(Some(true), Handler::None);
            s.form.show_tray = false;
            s.section = CAPTURE;
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("capture-screenclip-no-tray", tname, k), &img);
            // The tray icon off with the toggle off too.
            let mut s = sample().with_screenclip(Some(false), Handler::None);
            s.form.show_tray = false;
            s.section = CAPTURE;
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("capture-screenclip-no-tray-off", tname, k), &img);
            // The guide, and the toast once Rustshot is chosen.
            for (name, broken, dont_show) in [("screenclip-guide", false, false), ("screenclip-guide-broken", true, true)] {
                let mut s = sample();
                s.section = CAPTURE;
                s.modal = Some(Modal::Guide { broken, dont_show });
                let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
                preview::save(&png(name, tname, k), &img);
            }
            let mut s = sample();
            s.section = CAPTURE;
            s.modal = Some(Modal::Guide { broken: false, dont_show: false });
            s.guide_error = Some(crate::screenclip_win::open_failed_text("The system cannot find the file specified."));
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("screenclip-guide-error", tname, k), &img);
            let mut s = sample();
            s.section = CAPTURE;
            let shown = std::time::Instant::now().checked_sub(std::time::Duration::from_millis(600)).unwrap();
            s.toast = Some((crate::screenclip_win::NOW_OURS.into(), shown));
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("capture-screenclip-toast", tname, k), &img);
            // Capture with trim on.
            let mut s = sample();
            s.section = CAPTURE;
            s.form.trim = true;
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("capture-trim", tname, k), &img);
            // Teal accent (previewed before Apply), the tools just reset.
            let mut s = sample();
            s.form.accent = 1;
            s.section = OVERLAY;
            s.tools_reset = true;
            s.form.theme = if th.dark { 1 } else { 2 };
            let th2 = s.preview_theme();
            let (img, _) = render(&mut s, &th2, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("overlay-teal-reset", tname, k), &img);
            // Upload › Imgur with the shared built-in key: its line and
            // "Get a key".
            let mut s = sample();
            s.section = UPLOAD;
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("upload-imgur", tname, k), &img);
            // Upload › S3-compatible, filled in, a secret stored and the
            // test failed; then one being typed.
            let mut s = sample();
            s.section = UPLOAD;
            let f = &mut s.form.up;
            f.dest = upload::S3;
            f.endpoint.set("https://4f1c0e.r2.cloudflarestorage.com");
            f.bucket.set("rs-preview-shots");
            f.access_key.set("8a1d3c9e2b7f40d6");
            f.public_url.set("https://cdn.example.com");
            let _ = crate::keystore::os().set("rustshot:s3:rs-preview-shots@4f1c0e.r2.cloudflarestorage.com/8a1d3c9e2b7f40d6", "", "x");
            // The stored check answered (it runs on a worker).
            render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            s.upload.settle_now();
            s.upload.poll();
            s.upload.wait();
            s.upload.probe = Some(upload::Probe::Failed("upload failed: Access Denied (AccessDenied)".into()));
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("upload-s3", tname, k), &img);
            s.section = S3_STORAGE;
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("s3-storage-failed", tname, k), &img);
            s.form.up.secret.set("wJalrXUtnFEMI/K7MDENG");
            s.upload.probe = Some(upload::Probe::Passed);
            let mut focus = FocusState::default();
            render(&mut s, th, k, &Input::default(), &mut focus);
            focus.focus("s3_secret");
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut focus);
            preview::save(&png("s3-storage-typing", tname, k), &img);
            // The questions.
            for (name, m) in [
                ("rewrite", Modal::Rewrite { close: true }),
                ("error", Modal::Error(r"Could not save C:\Users\denis\AppData\Roaming\rustshot\config.toml: Access is denied. (os error 5)".into())),
                ("hotkey-failed", Modal::Hotkeys { text: HOTKEY_TEXT.into(), field: Some("global3".into()) }),
                ("hide-tray", Modal::HideTray),
            ] {
                let mut s = sample();
                s.modal = Some(m);
                let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
                preview::save(&png(name, tname, k), &img);
            }
            let mut s = sample();
            s.stale = true;
            s.modal = Some(Modal::Broken("line 7: invalid integer for 'jpeg_quality': high".into()));
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("invalid", tname, k), &img);
            s.modal = Some(Modal::Reset(String::new()));
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("reset", tname, k), &img);
        }
    }
}

/// Every section fits the body without scrolling, as it opens and at its
/// longest ([`crowded`]), at 100 % and up to the largest UI size on a
/// 150 % display (the window scales with both), in both themes: its
/// content ends above the footer and every control sits inside the body
/// (or the header and footer bars).
#[test]
/// Every section fits the body without scrolling, in English and in the
/// pseudo-languages (40 % longer, accented / right to left): no section
/// taller than the body, every control inside the window, on its side of
/// the rail edge, and no two controls overlapping.
fn every_section_fits() {
    for lang in crate::ui::preview::langs() {
        crate::i18n::with_lang(lang, || sections_fit(lang));
    }
}

fn sections_fit(lang: &str) {
    let vp = viewport();
    // (The pseudo-languages in one theme: the theme moves nothing.)
    let themes: &[&Theme] = if lang == "en" { &[&DARK, &LIGHT] } else { &[&DARK] };
    for k in [1.0f32, 2.0, 3.0] {
        for th in themes {
            for (i, title) in TITLES.iter().enumerate() {
                for (state, mut s) in [("default", sample()), ("crowded", crowded())] {
                    s.section = i;
                    let mut focus = FocusState::default();
                    render(&mut s, th, k, &Input::default(), &mut focus);
                    let h = s.content_h[i];
                    assert!(h > 100.0 && h <= vp.h, "{lang} {} ({state}) at {k}: {h} of {}", title, vp.h);
                    let rects: Vec<FRect> = focus.all_rects().into_iter().map(|r| FRect { x: r.x / k, y: r.y / k, w: r.w / k, h: r.h / k }).collect();
                    for r in &rects {
                        let (x0, y0, x1, y1) = (r.x, r.y, r.x1(), r.y1());
                        assert!(x0 >= 0.0 && x1 <= W as f32 + 0.5 && y0 >= 0.0 && y1 <= H as f32 + 0.5, "{lang} {} ({state}) at {k}: {r:?}", title);
                        assert!(x1 <= RAIL_W + 0.5 || x0 >= RAIL_W - 0.5, "{lang} {} ({state}) at {k}: {r:?} across the rail edge", title);
                        if x0 >= RAIL_W {
                            let bar = y1 <= HDR_H + 0.5 || y0 >= H as f32 - FOOT_H - 0.5;
                            assert!(bar || (y0 >= vp.y - 0.5 && y1 <= vp.y1() + 0.5), "{lang} {} ({state}) at {k}: {r:?} outside the body", title);
                        }
                    }
                    for (a, ra) in rects.iter().enumerate() {
                        for rb in &rects[a + 1..] {
                            let ow = ra.x1().min(rb.x1()) - ra.x.max(rb.x);
                            let oh = ra.y1().min(rb.y1()) - ra.y.max(rb.y);
                            assert!(ow <= 1.0 || oh <= 1.0, "{lang} {} ({state}) at {k}: {ra:?} and {rb:?} overlap", title);
                        }
                    }
                }
            }
        }
    }
}

/// The shortcut pages' labels (a long translation wrapped beside its
/// cap) never run into their key caps, at every UI scale, in English, the
/// pseudo-languages and the `RUSTSHOT_FIT_LANGS` catalogs; in English and
/// the real languages every row's label fits on one line beside its cap
/// in its column (the pages' two columns are slim enough), and every row
/// sits in its column.
#[test]
fn key_row_labels_fit_clear_of_their_caps() {
    let mut wrapped = Vec::new();
    for lang in preview::langs() {
        crate::i18n::with_lang(lang, || {
            for k in [1.0f32, 1.5, 2.0] {
                for (sec, page) in &KEY_PAGES {
                    let rows: Vec<(crate::i18n::Key, Action)> = page_rows(page).collect();
                    let mut s = sample();
                    s.section = *sec;
                    let mut focus = FocusState::default();
                    render(&mut s, &DARK, k, &Input::default(), &mut focus);
                    let mut widths = Vec::new();
                    let img = preview::render(W, H, k, &DARK, &Input::default(), &mut focus, |ui| {
                        draw::paint(ui, &mut s);
                        widths = rows.iter().map(|(l, _)| ui.text_w(12.0, l.get())).collect();
                    });
                    if k == 1.0 {
                        preview::save(&format!("settings-key-rows-{sec}-{lang}.png"), &img);
                    }
                    let problems = preview::take_problems();
                    assert!(problems.is_empty(), "{lang} {sec} at {k}: {problems:#?}");
                    // (A one-key row's box is the whole row: a click anywhere records.)
                    let boxes: Vec<FRect> = rows.iter().filter_map(|(_, a)| focus.rect_of(&format!("key_{}", a.id()))).collect();
                    assert_eq!(boxes.len(), rows.len(), "{lang} {sec}: every row drawn");
                    // Each row inside its column of the body.
                    let vp = viewport();
                    let cw = draw::key_col_w(vp.w - 2.0 * draw::KEYS_PAD_X, page.len());
                    let left = RAIL_W + draw::KEYS_PAD_X;
                    for b in &boxes {
                        let (x0, x1) = (b.x / k, b.x1() / k);
                        let col = (0..page.len()).any(|c| {
                            let cx = left + c as f32 * (cw + KEY_COL_GAP);
                            x0 >= cx - 0.5 && x1 <= cx + cw + 0.5
                        });
                        assert!(col, "{lang} {sec} at {k}: {b:?} in a column");
                    }
                    let texts = preview::drawn_texts();
                    for (((l, a), b), need) in rows.iter().zip(&boxes).zip(&widths) {
                        let label = l.get();
                        let cap = crate::ui::compact::caps_of(s.form.keys.chords(*a).first(), false).remove(0);
                        let mine: Vec<&(String, FRect)> = texts.iter().filter(|(_, t)| t.x >= b.x - 1.0 && t.x1() <= b.x1() + 1.0 && (b.y..b.y1()).contains(&(t.y + t.h / 2.0))).collect();
                        // The cap's text, rightmost; its key cap starts 7 px
                        // before it. (A symbol's cap, "[", is not noted: the
                        // narrowest cap, 22 px, from the row's 8 px padding.)
                        let c = mine.iter().filter(|(t, _)| *t == cap).map(|(_, r)| *r).max_by(|p, q| p.x.total_cmp(&q.x));
                        let cap_x = c.map_or(b.x1() - 30.0 * k, |c| c.x - 7.0 * k);
                        let lines: Vec<&&(String, FRect)> = mine.iter().filter(|(t, r)| !(*t == cap && r.x >= cap_x)).collect();
                        assert!(!lines.is_empty() && lines.iter().all(|(t, _)| label.contains(t.trim())), "{lang} {sec} at {k}: {label:?} drawn ({lines:?})");
                        for (t, r) in &lines {
                            assert!(r.x1() <= cap_x + 0.5, "{lang} {sec} at {k}: {t:?} at {r:?} runs into the cap at {cap_x}");
                        }
                        // One line beside the cap in the real languages.
                        // (Reported once a language and label, with the width it
                        // needs and the room it has.)
                        let one_line = lines.len() == 1 && lines[0].0 == label;
                        let known = wrapped.iter().any(|(l, t, ..): &(&str, String, f32, f32)| *l == lang && t == label);
                        if !(lang.starts_with("qps") || one_line || known) {
                            wrapped.push((lang, label.to_string(), need.ceil(), ((cap_x - b.x) / k - 12.0).floor()));
                        }
                    }
                }
            }
        });
    }
    assert!(wrapped.is_empty(), "labels wrapped beside their caps (shorten them): {wrapped:#?}");
}

/// The rail fits its column, About at the bottom clear of the last item,
/// and the Shortcuts label above the shortcut sections.
#[test]
fn rail_fits() {
    let area = rail_area();
    let (tops, labels) = crate::ui::compact::rail_layout(SECTIONS.len(), ABOUT, area, &RAIL);
    let last = tops[ABOUT - 1] + RAIL.item;
    assert!(last + 8.0 <= tops[ABOUT], "the last item {last} clear of About at {}", tops[ABOUT]);
    assert!(tops[ABOUT] + RAIL.item <= area.y1() + 0.5);
    assert_eq!(labels.len(), 1);
    assert!(labels[0].0 < tops[KEYS_GLOBAL] && labels[0].0 > tops[KEYS_GLOBAL - 1]);
}

/// The rail's names end inside its items (10 px clear of the rail's
/// edge) in English, the pseudo-languages and the `RUSTSHOT_FIT_LANGS`
/// catalogs, at 100 % and 150 %.
#[test]
fn rail_names_fit_in_every_language() {
    let mut long = Vec::new();
    for lang in preview::langs() {
        crate::i18n::with_lang(lang, || {
            for k in [1.0f32, 1.5] {
                // (The pseudo-languages' rail runs long: not real names.)
                if lang.starts_with("qps") {
                    continue;
                }
                let mut s = sample();
                render(&mut s, &DARK, k, &Input::default(), &mut FocusState::default());
                for (t, r) in preview::drawn_texts() {
                    if r.x < RAIL_W * k && r.x1() > (RAIL_W - 10.0) * k + 0.5 && !long.contains(&(lang, t.clone())) {
                        long.push((lang, t));
                    }
                }
            }
        });
    }
    assert!(long.is_empty(), "rail names past their items: {long:#?}");
}

/// Every global hotkey and every overlay / while-recording key is drawn
/// in the section that lists it (the deep links' targets).
#[test]
fn every_key_is_in_its_section() {
    let mut drawn: Vec<(usize, FocusState)> = Vec::new();
    for sec in KEYS_GLOBAL..=KEYS_EDIT_WINDOW {
        let mut s = sample();
        s.section = sec;
        let mut focus = FocusState::default();
        render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
        drawn.push((sec, focus));
    }
    let home = |id: &str| drawn.iter().filter(|(_, f)| f.rect_of(id).is_some()).map(|(s, _)| *s).collect::<Vec<_>>();
    for (label, slot) in GLOBALS {
        assert_eq!(home(&format!("global{slot}")), vec![section_of_slot(slot)], "{label}");
    }
    for a in Action::ALL {
        let want = if a.while_recording() {
            KEYS_RECORDING
        } else {
            KEY_PAGES.iter().find(|(_, p)| page_actions(p).any(|x| x == a)).map(|(sec, _)| *sec).unwrap_or(usize::MAX)
        };
        // (Linux has no share sheet: no Share key.)
        let want = if cfg!(target_os = "linux") && a == Action::Share { vec![] } else { vec![want] };
        assert_eq!(home(&format!("key_{}", a.id())), want, "{a:?}");
        if let Some(w) = want.first() {
            assert_eq!(section_of_action(a), *w, "{a:?}: the deep link's section");
        }
    }
    assert_eq!(section_of_slot(2), KEYS_RECORDING, "Record video sits with the recording keys");
}

/// The rail keys tests/e2e.rs `settings_window_saves_and_the_daemon_reloads`
/// posts land where it expects (its constants, repeated here): Down ×3
/// from General is Saving, with Save to and Format; End, Up, Up is
/// Overlay actions (Edit window is the last shortcut section), with
/// Accept's cap and then OK further on (it tabs to them by id).
#[test]
fn e2e_rail_keys() {
    let key = |s: &mut Settings, focus: &mut FocusState, vk: u32, n: usize| {
        for _ in 0..n {
            render(s, &DARK, 1.0, &press(vk, Default::default()), focus);
            render(s, &DARK, 1.0, &Input::default(), focus);
        }
    };
    let mut s = sample();
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    key(&mut s, &mut focus, wind::key::TAB, 1);
    assert!(focus.is_focused("rail"));
    key(&mut s, &mut focus, wind::key::DOWN, 3);
    assert_eq!(s.section, SAVING);
    assert!(focus.rect_of("folder").is_some() && focus.rect_of("format_all").is_some());
    let mut s = sample();
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    key(&mut s, &mut focus, wind::key::TAB, 1);
    key(&mut s, &mut focus, wind::key::END, 1);
    key(&mut s, &mut focus, wind::key::UP, 2);
    assert_eq!(s.section, KEYS_ACTIONS);
    assert!(focus.rect_of("key_accept").is_some());
    // Tab reaches every control of the section and the footer, OK too.
    let mut s = sample();
    s.section = KEYS_ACTIONS;
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    focus.focus("key_accept");
    let mut reached = false;
    for _ in 0..40 {
        key(&mut s, &mut focus, wind::key::TAB, 1);
        if focus.is_focused("ok") {
            reached = true;
            break;
        }
    }
    assert!(reached, "OK reached by Tab from Accept's cap");
}

/// The client points tools/sandbox-test/v030.ps1 clicks at 100 %: the
/// rail's Capture (60, 93), then Capture › Play shutter sound (641, 304)
/// (Win+Shift+S offered and off, as in a fresh sandbox).
#[test]
fn sandbox_click_points() {
    let mut s = sample().with_screenclip(Some(false), Handler::None);
    let mut focus = FocusState::default();
    for (x, y) in [(60, 51), (641, 304)] {
        render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
        let mut input = Input::default();
        for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
            input.feed(&e);
        }
        render(&mut s, &DARK, 1.0, &input, &mut focus);
    }
    assert_eq!(s.section, CAPTURE);
    assert!(s.form.shutter, "Play shutter sound turned on");
}

/// The rail: a click picks a section, Up/Down move while it has focus.
#[test]
fn rail_switches_sections() {
    let (tops, _) = crate::ui::compact::rail_layout(SECTIONS.len(), ABOUT, rail_area(), &RAIL);
    for k in [1.0f32, 1.5] {
        let mut s = sample();
        let mut focus = FocusState::default();
        render(&mut s, &DARK, k, &Input::default(), &mut focus);
        // Each item where it is drawn (About at the bottom of the rail).
        for (i, top) in tops.iter().enumerate().rev() {
            let (x, y) = ((60.0 * k) as i32, ((top + RAIL.item / 2.0) * k) as i32);
            let mut input = Input::default();
            for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
                input.feed(&e);
            }
            render(&mut s, &DARK, k, &input, &mut focus);
            assert_eq!(s.section, i, "k={k}");
        }
        assert!(tops[ABOUT] * k >= (H as f32 - 14.0 - 32.0) * k, "About at the bottom");
        render(&mut s, &DARK, k, &press(wind::key::END, Default::default()), &mut focus);
        assert_eq!(s.section, ABOUT);
        render(&mut s, &DARK, k, &press(wind::key::UP, Default::default()), &mut focus);
        assert_eq!(s.section, KEYS_EDIT_WINDOW);
        render(&mut s, &DARK, k, &press(wind::key::HOME, Default::default()), &mut focus);
        assert_eq!(s.section, GENERAL);
    }
}

/// The footer buttons click where drawn, at 100 % and 150 %: Apply
/// (disabled until something changes), Cancel, OK; the config link.
#[test]
fn footer_clicks_where_drawn() {
    for k in [1.0f32, 1.5] {
        let mut s = sample();
        let mut focus = FocusState::default();
        assert_eq!(click(&mut s, &mut focus, "apply", k), None, "Apply is disabled while nothing changed");
        s.form.ask = true;
        assert_eq!(click(&mut s, &mut focus, "apply", k), Some(Click::Apply), "k={k}");
        assert_eq!(click(&mut s, &mut focus, "cancel", k), Some(Click::Cancel));
        assert_eq!(click(&mut s, &mut focus, "ok", k), Some(Click::Ok));
        assert_eq!(click(&mut s, &mut focus, "open_config", k), Some(Click::OpenConfig));
        let r = focus.rect_of("apply").unwrap();
        assert!(r.x1() <= (W as f32 - 21.0) * k && r.y >= (H as f32 - FOOT_H) * k, "{r:?}");
    }
}

/// General's switches and swatches act where drawn.
#[test]
fn general_controls_click() {
    let mut s = sample();
    let mut focus = FocusState::default();
    click(&mut s, &mut focus, "show_tray", 1.0);
    assert!(!s.form.show_tray);
    // Turning the tray icon off asks first, saying what goes and how to
    // get back in.
    assert_eq!(s.modal, Some(Modal::HideTray));
    assert!(HIDE_TRAY.contains("Settings"));
    #[cfg(windows)]
    assert_eq!(
        HIDE_TRAY,
        "Rustshot won't run in the background. Win+Shift+S and Print Screen still open it if Rustshot is set as your screenshot app (Settings > Capture); other global shortcuts stop working. To open Settings, start Rustshot from the Start menu."
    );
    let mut qfocus = FocusState::default();
    assert_eq!(click(&mut s, &mut qfocus, "q_cancel", 1.0), Some(Click::KeepTray));
    assert_eq!(s.on(Click::KeepTray), vec![]);
    assert!(s.form.show_tray && s.modal.is_none(), "Cancel: the icon stays");
    // Esc keeps it too; Hide icon (or Enter) hides it.
    s.form.show_tray = false;
    s.ask_hide_tray();
    s.on(Click::Escape);
    assert!(s.form.show_tray && s.modal.is_none());
    s.form.show_tray = false;
    s.ask_hide_tray();
    assert_eq!(click(&mut s, &mut qfocus, "q_hide", 1.0), Some(Click::HideTray));
    s.on(Click::HideTray);
    assert!(!s.form.show_tray && s.modal.is_none());
    s.form.show_tray = false;
    s.ask_hide_tray();
    s.on(Click::Enter);
    assert!(!s.form.show_tray && s.modal.is_none());
    // Turning it back on asks nothing.
    click(&mut s, &mut focus, "show_tray", 1.0);
    assert!(s.form.show_tray && s.modal.is_none());
    click(&mut s, &mut focus, "autostart", 1.0);
    assert_eq!(s.form.autostart, Some(false));
    click(&mut s, &mut focus, "check_updates", 1.0);
    assert!(!s.form.check_updates, "update checks are in General");
    click(&mut s, &mut focus, "accent_teal", 1.5);
    assert_eq!(s.form.accent, 1);
    s.form.theme = 1; // Dark
    assert_eq!(s.preview_theme().accent, crate::uifb::C4::rgb(0x5C, 0xC8, 0xC2), "previewed before Apply");
    s.form.theme = 0;
    click(&mut s, &mut focus, "accent_custom", 1.0);
    assert_eq!(s.form.accent, ACCENT_CUSTOM);
    assert!(focus.is_focused("accent_hex"));
    // Theme: Light.
    click_right(&mut s, &mut focus, "theme");
    assert_eq!(s.form.theme, 2, "Light");
    assert!(s.dirty());
}

/// Capture's controls: Win+Shift+S, snapping, Accept on select, and
/// what happens after a capture.
#[test]
fn capture_controls_click() {
    let mut s = sample();
    s.section = CAPTURE;
    let mut focus = FocusState::default();
    click(&mut s, &mut focus, "shutter_sound", 1.0);
    assert!(s.form.shutter);
    click(&mut s, &mut focus, "copy_after_capture", 1.0);
    assert!(!s.form.copy_after);
    // Show thumbnail after capture: on by default, a click turns it off.
    assert!(s.form.thumbnail);
    click(&mut s, &mut focus, "show_thumbnail", 1.0);
    assert!(!s.form.thumbnail);
    assert!(!s.form.apply(&s.loaded, &s.loaded).show_thumbnail, "written as show_thumbnail = false");
    click(&mut s, &mut focus, "snap_windows", 1.0);
    assert!(!s.form.snap);
    assert_eq!(s.form.accept, 0);
    click_right(&mut s, &mut focus, "accept_on_select");
    assert_eq!(s.form.accept, 2, "Save");
    assert_eq!(s.form.apply(&s.loaded, &s.loaded).accept_on_select, "save");
    // The Win+Shift+S switch lives here (sample() offers it), not in General.
    assert_eq!(s.form.screenclip, Some(true));
    click(&mut s, &mut focus, "screenclip", 1.0);
    assert_eq!(s.form.screenclip, Some(false));
    let mut g = sample();
    let mut gfocus = FocusState::default();
    render(&mut g, &DARK, 1.0, &Input::default(), &mut gfocus);
    assert!(gfocus.rect_of("screenclip").is_none(), "not in General");
    assert!(gfocus.rect_of("autostart").is_some());
}

/// Overlay's controls: the magnifier (round unless Square), the hint
/// card, the UI size, Reset tool sizes.
#[test]
fn overlay_controls_click() {
    let mut s = sample();
    s.section = OVERLAY;
    let mut focus = FocusState::default();
    assert!(s.form.magnifier && !s.form.magnifier_square && s.form.hints);
    click(&mut s, &mut focus, "magnifier_square", 1.0);
    assert!(s.form.magnifier_square);
    click(&mut s, &mut focus, "magnifier", 1.0);
    assert!(!s.form.magnifier);
    click(&mut s, &mut focus, "overlay_hints", 1.0);
    assert!(!s.form.hints);
    let c = s.form.apply(&s.loaded, &s.loaded);
    assert!(!c.magnifier && c.magnifier_square && !c.overlay_hints);
    assert_eq!(click(&mut s, &mut focus, "reset_tools", 1.0), Some(Click::ResetTools));
    assert!(focus.rect_of("ui_size").is_some() && focus.rect_of("renderer").is_some());
}

/// Saving's controls: the folder picker, Ask, the format and quality.
#[test]
fn saving_controls_click() {
    let mut s = sample();
    s.section = SAVING;
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "browse", 1.0), Some(Click::Browse));
    click(&mut s, &mut focus, "ask", 1.0);
    assert!(s.form.ask);
    click_right(&mut s, &mut focus, "format_all");
    assert_eq!(s.form.format, 2, "BMP");
    assert!(focus.rect_of("folder").is_some() && focus.rect_of("quality").is_some());
}

/// File names: a token click goes into the pattern at its caret, or into
/// the daily subfolder pattern while that has focus.
#[test]
fn file_name_tokens_click() {
    let mut s = sample();
    s.section = FILE_NAMES;
    s.form.filename.set("ab");
    s.form.filename.select(1..1);
    let mut focus = FocusState::default();
    click(&mut s, &mut focus, "token6", 1.0); // Hour (00-23)
    assert_eq!(s.form.filename.text, "a%Hb");
    assert!(focus.is_focused("filename_pattern"));
    assert_eq!(click(&mut s, &mut focus, "restore", 1.0), Some(Click::Restore));
    s.form.subfolder = true;
    s.form.subfolder_pattern.set("%Y");
    click(&mut s, &mut focus, "subfolder_pattern", 1.0);
    click(&mut s, &mut focus, "token10", 1.0); // Second (00-59)
    assert!(s.form.subfolder_pattern.text.contains("%S"), "{}", s.form.subfolder_pattern.text);
}

/// History's rows act where drawn.
#[test]
fn history_controls_click() {
    let mut s = sample();
    s.section = HISTORY;
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "clear_history", 1.0), Some(Click::ClearHistory));
    click_right(&mut s, &mut focus, "history_days");
    assert_eq!(HISTORY_DAYS[s.form.history_days].0, 365);
    click(&mut s, &mut focus, "history", 1.0);
    assert!(!s.form.history);
}

/// A global row: a click records ("Press shortcut…"), the next chord
/// binds it, Esc cancels, Backspace unbinds.
#[test]
fn key_rows_record_then_rebind() {
    let mut s = sample();
    s.section = KEYS_GLOBAL;
    let mut focus = FocusState::default();
    click(&mut s, &mut focus, "global3", 1.0);
    assert!(focus.is_recording("global3"));
    let ctrl_alt = wind::Mods { ctrl: true, alt: true, shift: false };
    render(&mut s, &DARK, 1.0, &press('F' as u32, ctrl_alt), &mut focus);
    assert_eq!(s.form.full, Chord::parse("Ctrl+Alt+F"));
    assert!(!focus.is_recording("global3"));
    // Esc cancels.
    focus.record("global3");
    render(&mut s, &DARK, 1.0, &press(wind::key::ESCAPE, Default::default()), &mut focus);
    assert_eq!(s.form.full, Chord::parse("Ctrl+Alt+F"));
    assert!(!focus.is_recording("global3"));
    // Backspace unbinds.
    focus.record("global3");
    render(&mut s, &DARK, 1.0, &press(wind::key::BACK, Default::default()), &mut focus);
    assert_eq!(s.form.full, None);
    // Reset to defaults in the header.
    assert_eq!(click(&mut s, &mut focus, "reset_all", 1.0), Some(Click::ResetAll));
    // An overlay cap: Text's, in Overlay tools.
    let mut s = sample();
    s.section = KEYS_TOOLS;
    let mut focus = FocusState::default();
    click(&mut s, &mut focus, "key_tool_text", 1.0);
    assert!(focus.is_recording("key_tool_text"));
    render(&mut s, &DARK, 1.0, &press('Y' as u32, Default::default()), &mut focus);
    assert_eq!(s.form.keys.chords(Action::ToolText), [Chord::parse("Y").unwrap()]);
    assert_eq!(click(&mut s, &mut focus, "reset_all", 1.0), Some(Click::ResetAll));
}

/// The while-recording rows: their defaults have no note; a chord a
/// global hotkey has, another recording chord's, or a bare key, has one.
#[test]
fn while_recording_rows_notes() {
    let mut s = sample();
    for (_, a) in REC_ROWS {
        assert_eq!(s.form.rec_note(a), None, "{a:?}");
    }
    s.form.keys.set(Action::RecPause, vec![Chord::parse("Shift+Meta+G").unwrap()]);
    assert_eq!(s.form.rec_note(Action::RecPause).as_deref(), Some("Already used by Record video. Choose another."));
    s.form.keys.set(Action::RecPause, vec![Chord::parse("Ctrl+Alt+Shift+M").unwrap()]);
    assert_eq!(s.form.rec_note(Action::RecPause).as_deref(), Some("Already used by Mute microphone. Choose another."));
    s.form.keys.set(Action::RecPause, vec![Chord::parse("P").unwrap()]);
    assert_eq!(s.form.rec_note(Action::RecPause).as_deref(), Some(BARE_NOTE.get()));
    // The same chord as an overlay key is fine (the overlay is closed while recording).
    s.form.keys.set(Action::RecPause, vec![Chord::parse("Ctrl+S").unwrap()]);
    assert_eq!(s.form.rec_note(Action::RecPause), None);
    // Saved like the overlay keys, in [shortcuts].
    s.section = KEYS_RECORDING;
    let mut focus = FocusState::default();
    click(&mut s, &mut focus, "key_rec_mute", 1.0);
    assert!(focus.is_recording("key_rec_mute"));
    let ctrl_alt = wind::Mods { ctrl: true, alt: true, shift: false };
    render(&mut s, &DARK, 1.0, &press('K' as u32, ctrl_alt), &mut focus);
    assert_eq!(s.form.keys.chords(Action::RecMute), [Chord::parse("Ctrl+Alt+K").unwrap()]);
    // The record hotkey is with them.
    click(&mut s, &mut focus, "global2", 1.0);
    assert!(focus.is_recording("global2"));
}

/// The shortcut pages and the while-recording rows list every keymap
/// action once, one action a row, at most 10 rows a column (12 in a
/// page of one column).
#[test]
fn key_rows_cover_every_action() {
    let mut all: Vec<Action> = KEY_PAGES.iter().flat_map(|(_, p)| page_actions(p)).collect();
    assert!(all.iter().all(|a| !a.while_recording()));
    assert!(REC_ROWS.iter().all(|(_, a)| a.while_recording()));
    all.extend(REC_ROWS.iter().map(|(_, a)| *a));
    // Share only where there is a share sheet (not on Linux).
    let want: Vec<Action> = Action::ALL.into_iter().filter(|a| *a != Action::Share || crate::share::supported()).collect();
    assert_eq!(all.len(), want.len());
    assert_eq!(all.contains(&Action::Share), crate::share::supported());
    for a in want {
        assert_eq!(all.iter().filter(|x| **x == a).count(), 1, "{a:?}");
    }
    for (sec, page) in &KEY_PAGES {
        assert_eq!(page_of(*sec).map(|p| page_actions(p).collect::<Vec<_>>()), Some(page_actions(page).collect()));
        assert!((1..=2).contains(&page.len()));
        for col in page.iter() {
            let rows = col.iter().filter(|i| matches!(i, draw::KeyItem::Row(..))).count();
            assert!(rows <= if page.len() == 1 { 12 } else { 10 }, "{sec}: {rows} rows in a column");
        }
    }
}

/// About: the links open only their fixed URLs, Check for updates the
/// update dialog.
#[test]
fn about_links() {
    let mut s = sample();
    s.section = ABOUT;
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "check_updates_now", 1.0), Some(Click::CheckUpdates));
    for (i, (label, url)) in crate::actions::LINKS.iter().take(5).enumerate() {
        assert_eq!(click(&mut s, &mut focus, &format!("link_{i}"), 1.0), Some(Click::Link(url)), "{}", label.get());
    }
    assert_eq!(click(&mut s, &mut focus, "link_powernapps", 1.0), Some(Click::Link(crate::actions::POWERNAPPS)));
}

/// Upload: with the shared built-in Imgur key, a line under the field
/// says so and "Get a key" opens Imgur's app registration (one of the
/// links `open_link` allows); with a key of one's own, or S3, no line.
#[test]
fn upload_get_a_key_link() {
    let mut s = sample();
    s.section = UPLOAD;
    assert!(upload::builtin_key(&s.form.up.client_id.text), "the default is the built-in key");
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "imgur_get_key", 1.0), Some(Click::Link(crate::actions::IMGUR_ADD_CLIENT)));
    assert_eq!(s.on(Click::Link(crate::actions::IMGUR_ADD_CLIENT)), vec![Act::Link(crate::actions::IMGUR_ADD_CLIENT)]);
    assert!(crate::actions::LINKS.iter().any(|(k, u)| *u == "https://api.imgur.com/oauth2/addclient" && k.get() == "Get a key"));
    // The link sits under the field, inside the body.
    let field = focus.rect_of("upload_client_id").expect("the field");
    let link = focus.rect_of("imgur_get_key").expect("the link");
    assert!(link.y >= field.y1() && link.x1() <= field.x1() + 0.5, "{field:?} {link:?}");
    s.form.up.client_id.set(" abc123 ");
    assert!(!upload::builtin_key(&s.form.up.client_id.text));
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    assert!(focus.rect_of("imgur_get_key").is_none(), "an own key: no line");
    s.form.up.client_id.set(crate::config::BUILTIN_IMGUR_CLIENT_ID);
    s.form.up.dest = upload::S3;
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    assert!(focus.rect_of("imgur_get_key").is_none(), "S3: no Imgur line");
}

/// The built-in key's line is one line in English and the
/// `RUSTSHOT_FIT_LANGS` catalogs: its text is never cut short with "…"
/// next to "Get a key". (The 40 %-longer pseudo-languages cut it; the
/// layout still fits, `every_section_fits`.)
#[test]
fn upload_builtin_key_line_fits() {
    let mut cut = Vec::new();
    for lang in crate::ui::preview::langs().into_iter().filter(|l| !l.starts_with("qps")) {
        crate::i18n::with_lang(lang, || {
            let mut s = sample();
            s.section = UPLOAD;
            for k in [1.0f32, 2.0] {
                let mut focus = FocusState::default();
                render(&mut s, &DARK, k, &Input::default(), &mut focus);
                let field = focus.rect_of("upload_client_id").expect("the field");
                let link = focus.rect_of("imgur_get_key").expect("the link");
                assert!(link.y >= field.y1(), "{lang}");
                let body_w = viewport().w - 2.0 * super::draw::PAD_X;
                // As the kit measures it (`Ui::text_w`).
                let text_w = crate::uifb::text_width(&crate::fonts::UI, 11.0 * crate::ui::compact::CSS * k, tr!("settings.upload.builtin_key")) / k;
                let over = text_w + 6.0 + link.w / k - body_w;
                if over > 0.0 {
                    cut.push(format!("{lang} at {k}: {over:.0} px over"));
                }
            }
        });
    }
    assert!(cut.is_empty(), "{cut:#?}");
}

/// Smoke (needs a display; CI runs it under `xvfb-run` on Linux): the real
/// Settings window opens, draws and closes after about a second.
/// `cargo test window_smoke -- --ignored --test-threads=1`
#[cfg(not(target_os = "macos"))] // macOS windows need the main thread
#[test]
#[ignore = "opens a window"]
fn settings_window_smoke() {
    if !crate::wind::has_display() {
        eprintln!("no display; skipped");
        return;
    }
    let _guard = crate::wind::test_window_lock();
    let dir = scratch("smoke");
    let s = Settings::new(Config::default(), None, dir.join("config.toml"));
    let (_tx, rx) = crate::chan::channel();
    let mut w = Window::new(s, crate::theme::resolve(&Config::default()), rx);
    let mut d = crate::wind::AutoClose::new(&mut w, 1000);
    let spec = WindowSpec { title: "Rustshot Settings".into(), w: W, h: H, resizable: false, min: (W, H), controls: wind::Controls::MinClose };
    wind::run_window(spec, &mut d).expect("settings window");
    assert!(d.frames > 0, "drew a frame");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Saving › Watermark on export: a PNG chosen in Edit › Effects shows
/// with "Use text", which clears it.
#[test]
fn the_watermark_image_shows_and_clears() {
    let cfg = Config { export_watermark: true, watermark_image: "C:\\nope\\logo.png".into(), ..Config::default() };
    let mut s = Settings::new(cfg.clone(), Some(true), PathBuf::from("config.toml"));
    s.section = SAVING;
    let mut focus = FocusState::default();
    reveal(&mut s, &mut focus, "watermark_clear");
    assert!(focus.rect_of("watermark_text").is_none());
    click(&mut s, &mut focus, "watermark_clear", 0.5);
    assert!(s.form.watermark_image.is_empty());
    let out = s.form.apply(&s.loaded, &s.loaded);
    assert!(out.watermark_image.is_empty() && out.export_watermark);
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    assert!(focus.rect_of("watermark_text").is_some(), "back to the text");
}
