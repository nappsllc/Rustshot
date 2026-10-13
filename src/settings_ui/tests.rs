use super::draw::{rail_area, viewport, ACTION_ROWS, FOOT_H, HDR_H, RAIL, RAIL_W, REC_ROWS, TOOL_ROWS};
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
    f.upload_id.set(" mine ");
    f.copy_url = false;
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
    let mut s = settings_at(&p).with_screenclip(Some(false), false);
    s.set_screenclip = |on| {
        LAST.store(on as u8, Ordering::SeqCst);
        Ok(())
    };
    assert!(!s.dirty());
    s.form.screenclip = Some(true);
    assert!(s.dirty());
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]));
    assert_eq!(LAST.load(Ordering::SeqCst), 1);
    assert_eq!(s.form.screenclip, Some(true), "kept across the rebuild");
    assert!(s.screenclip_wait && !s.dirty());
    s.form.screenclip = Some(false);
    s.on(Click::Apply);
    assert_eq!(LAST.load(Ordering::SeqCst), 0);
    assert!(!s.screenclip_wait && !s.dirty());
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
    let _ = std::fs::remove_dir_all(&dir);
}

/// The file breaks while the window is open: Save refuses to write
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
        Failure { slot: 0, wanted: "Ctrl+Alt+F9".into(), cause: Cause::InUse, kept: Some("Ctrl+Alt+Shift+Q".into()) },
        Failure { slot: 1, wanted: "Ctrl+Alt+F10".into(), cause: Cause::Unknown, kept: None },
        Failure { slot: 5, wanted: "Ctrl+Shift+P".into(), cause: Cause::InUse, kept: None },
    ];
    assert!(s.retry_hotkeys() && !s.dirty());
    assert_eq!(s.hotkey_slot_note(0).as_deref(), Some("Not registered \u{2014} using Ctrl+Alt+Shift+Q"));
    assert_eq!(s.hotkey_slot_note(1).as_deref(), Some("Not registered"));
    assert_eq!(s.global_message(5).as_deref(), Some("Already used by another app. Choose another."));
    assert_eq!(s.on(Click::Apply), vec![Act::Reload]);
    assert_eq!(s.on(Click::Ok), vec![Act::Reload, Act::Close]);
    assert_eq!(s.on(Click::Enter), vec![Act::Reload, Act::Close]);
    assert_eq!(s.on(Click::Cancel), vec![Act::Close], "Cancel never retries");
    // With a change it saves (the save makes the daemon reload anyway).
    s.form.ask = true;
    assert!(matches!(&s.on(Click::Apply)[..], [Act::Saved(_)]));
    // Apply is enabled for the retry: a click on it lands.
    let mut s = sample();
    s.hotkey_failures = vec![Failure { slot: 0, wanted: "Ctrl+Alt+F9".into(), cause: Cause::InUse, kept: None }];
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "apply", 1.0), Some(Click::Apply));
    let _ = std::fs::remove_dir_all(&dir);
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
    assert_eq!(s.on(Click::Link("https://powernapps.net")), vec![Act::Link("https://powernapps.net")]);
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
    s.hotkey_failures = vec![crate::hotkey::Failure { slot: 0, wanted: "Ctrl+Shift+S".into(), cause: crate::hotkey::Cause::InUse, kept: None }];
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
    let mut s = Settings::new(preview_cfg(), Some(true), PathBuf::from("config.toml")).with_screenclip(Some(true), false);
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
/// without a working capture hotkey, Win+Shift+S waiting for Windows,
/// the Linux encoder row, no camera, every global hotkey taken by another
/// app, and every overlay and while-recording key on the same bare P.
fn crowded() -> Settings {
    use crate::hotkey::{Cause, Failure};
    let mut s = sample();
    s.form.folder.set("shots");
    s.form.show_tray = false;
    s.screenclip_wait = true;
    s.tools_reset = true;
    s.rec_lists.linux = true;
    s.rec_lists.found = Some(recording::Found::default());
    s.form.rec.format = 1;
    s.hotkey_failures = (0..hotkey::SLOTS).map(|slot| Failure { slot, wanted: "Ctrl+Shift+S".into(), cause: Cause::InUse, kept: None }).collect();
    let p = Chord::parse("P").unwrap();
    for a in Action::ALL {
        s.form.keys.set(a, vec![p]);
    }
    s
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
            s.hotkey_failures = vec![crate::hotkey::Failure { slot: 5, wanted: "Ctrl+Shift+P".into(), cause: crate::hotkey::Cause::InUse, kept: None }];
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
            // Saving: a drive that is not there, JPEG chosen.
            let mut s = sample();
            s.section = SAVING;
            s.form.format = 1;
            s.form.folder.set(if cfg!(windows) { r"Q:\shots" } else { "/no-such-root/shots" });
            let root = PathBuf::from(if cfg!(windows) { r"Q:\" } else { "/no-such-root" });
            s.folder_probe = Some((root, Some("Drive Q: is not on this computer.".into())));
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("saving-states", tname, k), &img);
            // Win+Shift+S just turned on: Windows has not been told yet.
            let mut s = sample();
            s.form.screenclip = Some(true);
            s.screenclip_wait = true;
            s.section = CAPTURE;
            let (img, _) = render(&mut s, th, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("capture-screenclip", tname, k), &img);
            // Teal accent (previewed before Apply), the tools just reset.
            let mut s = sample();
            s.form.accent = 1;
            s.section = OVERLAY;
            s.tools_reset = true;
            s.form.theme = if th.dark { 1 } else { 2 };
            let th2 = s.preview_theme();
            let (img, _) = render(&mut s, &th2, k, &Input::default(), &mut FocusState::default());
            preview::save(&png("overlay-teal-reset", tname, k), &img);
            // The questions.
            for (name, m) in [
                ("rewrite", Modal::Rewrite { close: true }),
                ("error", Modal::Error(r"Could not save C:\Users\denis\AppData\Roaming\rustshot\config.toml: Access is denied. (os error 5)".into())),
                ("hotkey-failed", Modal::Error("Couldn't register Ctrl+Shift+S \u{2014} it's in use by another app; kept Shift+Win+X.".into())),
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
fn every_section_fits() {
    let vp = viewport();
    for k in [1.0f32, 2.0, 3.0] {
        for th in [&DARK, &LIGHT] {
            for (i, title) in TITLES.iter().enumerate() {
                for (state, mut s) in [("default", sample()), ("crowded", crowded())] {
                    s.section = i;
                    let mut focus = FocusState::default();
                    render(&mut s, th, k, &Input::default(), &mut focus);
                    let h = s.content_h[i];
                    assert!(h > 100.0 && h <= vp.h, "{} ({state}) at {k}: {h} of {}", title, vp.h);
                    for r in focus.all_rects() {
                        let (x0, y0, x1, y1) = (r.x / k, r.y / k, r.x1() / k, r.y1() / k);
                        assert!(x0 >= 0.0 && x1 <= W as f32 + 0.5 && y0 >= 0.0 && y1 <= H as f32 + 0.5, "{} ({state}): {r:?}", title);
                        if x0 >= RAIL_W {
                            let bar = y1 <= HDR_H + 0.5 || y0 >= H as f32 - FOOT_H - 0.5;
                            assert!(bar || (y0 >= vp.y - 0.5 && y1 <= vp.y1() + 0.5), "{} ({state}): {r:?} outside the body", title);
                        }
                    }
                }
            }
        }
    }
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

/// Every global hotkey and every overlay / while-recording key is drawn
/// in the section that lists it (the deep links' targets).
#[test]
fn every_key_is_in_its_section() {
    let mut drawn: Vec<(usize, FocusState)> = Vec::new();
    for sec in KEYS_GLOBAL..=KEYS_ACTIONS {
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
        } else if TOOL_ROWS.iter().any(|(_, acts)| acts.contains(&a)) {
            KEYS_TOOLS
        } else {
            KEYS_ACTIONS
        };
        assert_eq!(home(&format!("key_{}", a.id())), vec![want], "{a:?}");
    }
    assert_eq!(section_of_slot(2), KEYS_RECORDING, "Record video sits with the recording keys");
}

/// The key presses tests/e2e.rs `settings_window_saves_and_the_daemon_reloads`
/// posts land where it expects (its constants, repeated here): the rail →
/// Saving, 2 Tabs to Save to, 2 more to Format; the rail → Overlay
/// actions (End, Up), 6 Tabs to Accept's cap, 18 more to OK.
#[test]
fn e2e_tab_counts() {
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
    key(&mut s, &mut focus, wind::key::TAB, 2);
    assert!(focus.is_focused("folder"));
    key(&mut s, &mut focus, wind::key::TAB, 2);
    assert!(focus.is_focused("format_all"));
    let mut s = sample();
    let mut focus = FocusState::default();
    render(&mut s, &DARK, 1.0, &Input::default(), &mut focus);
    key(&mut s, &mut focus, wind::key::TAB, 1);
    key(&mut s, &mut focus, wind::key::END, 1);
    key(&mut s, &mut focus, wind::key::UP, 1);
    assert_eq!(s.section, KEYS_ACTIONS);
    key(&mut s, &mut focus, wind::key::TAB, 6);
    assert!(focus.is_focused("key_accept"));
    s.form.ask = true; // (Apply is enabled once J is recorded)
    key(&mut s, &mut focus, wind::key::TAB, 18);
    assert!(focus.is_focused("ok"));
}

/// The client points tools/sandbox-test/v030.ps1 clicks at 100 %: the
/// rail's Capture (60, 96), then Capture › Play shutter sound (641, 304)
/// (Win+Shift+S offered and off, as in a fresh sandbox).
#[test]
fn sandbox_click_points() {
    let mut s = sample().with_screenclip(Some(false), false);
    let mut focus = FocusState::default();
    for (x, y) in [(60, 96), (641, 304)] {
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
        assert_eq!(s.section, KEYS_ACTIONS);
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
    assert_eq!(s.form.rec_note(Action::RecPause).as_deref(), Some(BARE_NOTE));
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

/// The overlay rows (tools, actions) and the while-recording rows list
/// every keymap action once (each in its own group), at most 7 rows a
/// section.
#[test]
fn key_rows_cover_every_action() {
    let mut all: Vec<Action> = TOOL_ROWS.iter().chain(ACTION_ROWS.iter()).flat_map(|(_, a)| a.iter().copied()).collect();
    assert!(all.iter().all(|a| !a.while_recording()));
    assert!(REC_ROWS.iter().all(|(_, a)| a.while_recording()));
    all.extend(REC_ROWS.iter().map(|(_, a)| *a));
    assert_eq!(all.len(), Action::ALL.len());
    all.dedup();
    for a in Action::ALL {
        assert!(all.contains(&a), "{a:?}");
    }
    assert!(TOOL_ROWS.len() <= 7 && ACTION_ROWS.len() <= 7);
}

/// About: the links open only their fixed URLs, Check for updates the
/// update dialog.
#[test]
fn about_links() {
    let mut s = sample();
    s.section = ABOUT;
    let mut focus = FocusState::default();
    assert_eq!(click(&mut s, &mut focus, "check_updates_now", 1.0), Some(Click::CheckUpdates));
    for (label, url) in crate::actions::LINKS.iter().take(5) {
        assert_eq!(click(&mut s, &mut focus, &format!("link_{label}"), 1.0), Some(Click::Link(url)), "{label}");
    }
    assert_eq!(click(&mut s, &mut focus, "link_powernapps", 1.0), Some(Click::Link("https://powernapps.net")));
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
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut w = Window::new(s, crate::theme::resolve(&Config::default()), rx);
    let mut d = crate::wind::AutoClose::new(&mut w, 1000);
    let spec = WindowSpec { title: "Rustshot Settings".into(), w: W, h: H, resizable: false, min: (W, H) };
    wind::run_window(spec, &mut d).expect("settings window");
    assert!(d.frames > 0, "drew a frame");
    let _ = std::fs::remove_dir_all(&dir);
}
