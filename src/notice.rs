#![cfg_attr(not(windows), allow(dead_code))] // only Windows has the notice window yet
//! The confirmation toast after a capture: the overlay closes at once, so
//! a small notice ("Copied to clipboard", "Saved · shot.png",
//! "Uploading…" then "Link copied · i.imgur.com/…") says what happened,
//! bottom-centre on the monitor where the capture was. It never takes
//! focus or clicks and is excluded from screen capture.
//!
//! Windows draws it in a layered window ([`win`]); elsewhere there is no
//! window yet (`show` returns false and the caller falls back to stdout
//! and, for an upload, `tray::notify`).

use crate::editor::style::{toast, toast_size, ToastKind, Ui};
use crate::export::Done;
use crate::objects::FRect;
use crate::theme::Theme;
use std::time::Duration;

/// Visible time before the fade-out: success, and errors.
pub const TTL: Duration = Duration::from_millis(1800);
pub const ERROR_TTL: Duration = Duration::from_millis(4000);
/// "Uploading…" stays up while the upload runs, at most this long (a
/// later result is a tray balloon).
pub const UPLOAD_TTL: Duration = Duration::from_secs(30);
pub const FADE_IN: Duration = Duration::from_millis(120);
pub const FADE_OUT: Duration = Duration::from_millis(250);

/// Longest reason shown (an HTTP error body can be long).
const REASON_MAX: usize = 90;

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub text: String,
    pub kind: ToastKind,
    /// Visible time before the fade-out.
    pub ttl: Duration,
}

impl Note {
    fn new(text: String, kind: ToastKind) -> Note {
        let ttl = if kind == ToastKind::Error { ERROR_TTL } else { TTL };
        Note { text, kind, ttl }
    }
}

fn reason(e: &str) -> String {
    let e = e.trim();
    if e.chars().count() <= REASON_MAX {
        return e.to_string();
    }
    let mut s: String = e.chars().take(REASON_MAX - 1).collect();
    s.push('\u{2026}');
    s
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// One line for what an export did (`uploading`: an upload started).
/// A single action reads in full ("Copied to clipboard"); several are
/// joined short ("Copied · Saved shot.png · Uploading…"). None when
/// nothing is worth saying (a cancelled save dialog, CLI-only tasks).
pub fn compose(done: &[Done], uploading: bool) -> Option<Note> {
    let single = done.len() + usize::from(uploading) == 1;
    let mut parts = Vec::new();
    let mut failed = false;
    for d in done {
        parts.push(match d {
            Done::Copied if single => "Copied to clipboard".to_string(),
            Done::Copied => "Copied".to_string(),
            Done::Saved(p) if single => format!("Saved \u{b7} {}", file_name(p)),
            Done::Saved(p) => format!("Saved {}", file_name(p)),
            Done::Failed(what, e) => {
                failed = true;
                format!("{what} failed: {}", reason(e))
            }
        });
    }
    if uploading {
        parts.push("Uploading\u{2026}".to_string());
    }
    if parts.is_empty() {
        return None;
    }
    let text = parts.join(" \u{b7} ");
    Some(if failed {
        Note::new(text, ToastKind::Error)
    } else if uploading {
        let kind = if done.is_empty() { ToastKind::Info } else { ToastKind::Success };
        Note { text, kind, ttl: UPLOAD_TTL }
    } else {
        Note::new(text, ToastKind::Success)
    })
}

/// The finished upload: "Link copied · i.imgur.com/x.png" (or "Uploaded ·
/// …" when the link was not copied), or "Upload failed: <reason>".
pub fn upload_note(res: &Result<String, String>, copied: bool) -> Note {
    match res {
        Ok(url) => {
            let short = url.trim_start_matches("https://").trim_start_matches("http://");
            let head = if copied { "Link copied" } else { "Uploaded" };
            Note::new(format!("{head} \u{b7} {short}"), ToastKind::Success)
        }
        Err(e) => Note::new(format!("Upload failed: {}", reason(e)), ToastKind::Error),
    }
}

/// Room around the panel for its shadow (logical px): sides, top (also
/// the 4 px rise). The bottom room is the toast's own 28 px gap.
const SIDE: f32 = 14.0;
const TOP: f32 = 16.0;
const BOTTOM: f32 = 28.0;

/// The window size for `text` at scale `s` (physical px).
pub fn size(text: &str, s: f32) -> (u32, u32) {
    let ui = Ui { th: &crate::theme::DARK, s, font: Some(crate::fonts::ui_font()) };
    let (w, h) = toast_size(&ui, text);
    ((w + 2.0 * SIDE * s).ceil() as u32, (h + (TOP + BOTTOM) * s).ceil() as u32)
}

/// Where a `size` window goes on a monitor's work area (physical px):
/// centred, its bottom on the work area's (the panel then sits the
/// toast's 28 px above it, clear of the taskbar).
pub fn place(work: crate::capture::IRect, (w, h): (u32, u32)) -> (i32, i32) {
    let x = work.0 + (work.2 as i32 - w as i32) / 2;
    let y = work.1 + work.3 as i32 - h as i32;
    (x, y)
}

/// Opacity `age` into a note shown for `ttl`; None once it is gone.
pub fn opacity(age: Duration, ttl: Duration) -> Option<f32> {
    if age >= ttl + FADE_OUT {
        return None;
    }
    let fade_in = crate::anim::ease_out(age.as_secs_f32() / FADE_IN.as_secs_f32());
    let out = 1.0 - age.saturating_sub(ttl).as_secs_f32() / FADE_OUT.as_secs_f32();
    Some(fade_in.min(out).clamp(0.0, 1.0))
}

/// Draw `note` at opacity `k` into a window of [`size`] (premultiplied BGRA).
pub fn render(note: &Note, th: &Theme, s: f32, k: f32) -> (u32, u32, Vec<u8>) {
    let (w, h) = size(&note.text, s);
    let ui = Ui { th, s, font: Some(crate::fonts::ui_font()) };
    let area = FRect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 };
    let px = crate::rec_ui::render_layer(w as usize, h as usize, |f| toast(f, &ui, &note.text, note.kind, area, k));
    (w, h, px)
}

#[cfg(windows)]
#[path = "notice_win.rs"]
mod win;

/// Show `note` near global point `at` (the capture's centre), replacing
/// the one up; false when there is no notice window here.
pub fn show(note: Note, at: (i32, i32), th: Theme) -> bool {
    #[cfg(test)]
    {
        let _ = th;
        TEST.with(|t| t.borrow_mut().push((note, at)));
        true
    }
    #[cfg(all(windows, not(test)))]
    {
        win::show(note, at, th)
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = (note, at, th);
        false
    }
}

/// Put `note` in place of the one up (same monitor and theme); false when
/// none is up any more.
pub fn replace(note: Note) -> bool {
    #[cfg(test)]
    {
        TEST.with(|t| {
            let mut t = t.borrow_mut();
            let Some(at) = t.last().map(|l| l.1) else { return false };
            t.push((note, at));
            true
        })
    }
    #[cfg(all(windows, not(test)))]
    {
        win::replace(note)
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = note;
        false
    }
}

/// Take the note down (a new capture is starting).
pub fn hide() {
    #[cfg(test)]
    TEST.with(|t| t.borrow_mut().clear());
    #[cfg(all(windows, not(test)))]
    win::hide();
}

#[cfg(test)]
thread_local! {
    static TEST: std::cell::RefCell<Vec<(Note, (i32, i32))>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Tests: the notes shown on this thread since the last call (oldest first).
#[cfg(test)]
pub fn take_shown() -> Vec<(Note, (i32, i32))> {
    TEST.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn saved(n: &str) -> Done {
        Done::Saved(PathBuf::from("D:\\Downloads").join(n))
    }

    #[test]
    fn single_actions_read_in_full() {
        let n = compose(&[Done::Copied], false).unwrap();
        assert_eq!((n.text.as_str(), n.kind, n.ttl), ("Copied to clipboard", ToastKind::Success, TTL));
        let n = compose(&[saved("shot.png")], false).unwrap();
        assert_eq!(n.text, "Saved \u{b7} shot.png");
        let n = compose(&[], true).unwrap();
        assert_eq!((n.text.as_str(), n.kind, n.ttl), ("Uploading\u{2026}", ToastKind::Info, UPLOAD_TTL));
        let n = compose(&[Done::Failed("Save", "access denied".into())], false).unwrap();
        assert_eq!((n.text.as_str(), n.kind, n.ttl), ("Save failed: access denied", ToastKind::Error, ERROR_TTL));
        assert_eq!(compose(&[], false), None, "nothing to say (a cancelled save dialog)");
    }

    #[test]
    fn several_actions_make_one_line() {
        let n = compose(&[Done::Copied, saved("a.jpg")], false).unwrap();
        assert_eq!((n.text.as_str(), n.kind), ("Copied \u{b7} Saved a.jpg", ToastKind::Success));
        let n = compose(&[Done::Copied], true).unwrap();
        assert_eq!((n.text.as_str(), n.kind, n.ttl), ("Copied \u{b7} Uploading\u{2026}", ToastKind::Success, UPLOAD_TTL));
        let n = compose(&[Done::Copied, Done::Failed("Save", "disk full".into())], false).unwrap();
        assert_eq!((n.text.as_str(), n.kind, n.ttl), ("Copied \u{b7} Save failed: disk full", ToastKind::Error, ERROR_TTL));
    }

    #[test]
    fn upload_results() {
        let n = upload_note(&Ok("https://i.imgur.com/abc.png".into()), true);
        assert_eq!((n.text.as_str(), n.kind, n.ttl), ("Link copied \u{b7} i.imgur.com/abc.png", ToastKind::Success, TTL));
        assert_eq!(upload_note(&Ok("https://i.imgur.com/abc.png".into()), false).text, "Uploaded \u{b7} i.imgur.com/abc.png");
        let n = upload_note(&Err("HTTP 429".into()), true);
        assert_eq!((n.text.as_str(), n.kind, n.ttl), ("Upload failed: HTTP 429", ToastKind::Error, ERROR_TTL));
        let long = upload_note(&Err("x".repeat(500)), true).text;
        assert_eq!(long.chars().count(), "Upload failed: ".len() + REASON_MAX);
        assert!(long.ends_with('\u{2026}'));
    }

    #[test]
    fn placed_bottom_centre_of_the_work_area() {
        // A 1920x1080 monitor right of the primary, taskbar 48 px.
        let work = (1920, 0, 1920, 1032);
        assert_eq!(place(work, (300, 80)), (1920 + 810, 1032 - 80));
        // Monitors above/left of the primary have negative origins.
        assert_eq!(place((-1280, -1024, 1280, 984), (200, 80)), (-1280 + 540, -1024 + 984 - 80));
    }

    #[test]
    fn fades_in_holds_and_fades_out() {
        let ms = Duration::from_millis;
        assert_eq!(opacity(ms(0), TTL), Some(0.0));
        assert_eq!(opacity(ms(500), TTL), Some(1.0));
        let k = opacity(TTL + FADE_OUT / 2, TTL).unwrap();
        assert!((k - 0.5).abs() < 0.01, "{k}");
        assert_eq!(opacity(TTL + FADE_OUT, TTL), None);
    }

    /// Preview PNGs of each notice (dark and light, 1x and 2x) under
    /// `RUSTSHOT_PREVIEW_DIR`.
    #[test]
    #[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
    fn render_notice_previews() {
        let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let notes = [
            ("copied", compose(&[Done::Copied], false)),
            ("saved", compose(&[saved("2026-10-10_19-48.jpg")], false)),
            ("copied-saved", compose(&[Done::Copied, saved("2026-10-10_19-48.jpg")], false)),
            ("uploading", compose(&[], true)),
            ("link-copied", Some(upload_note(&Ok("https://i.imgur.com/aB3xYz9.png".into()), true))),
            ("upload-failed", Some(upload_note(&Err("WinHttpSendRequest: The operation timed out".into()), true))),
            ("save-failed", compose(&[Done::Failed("Save", "access denied (os error 5)".into())], false)),
        ];
        for (tn, th, bg) in [("dark", &crate::theme::DARK, [38u8, 44, 58]), ("light", &crate::theme::LIGHT, [214, 222, 232])] {
            for s in [1.0f32, 2.0] {
                for (name, note) in &notes {
                    let (w, h, px) = render(note.as_ref().unwrap(), th, s, 1.0);
                    let img = crate::pixbuf::PixBuf::from_raw(w, h, crate::rec_ui::layer_over(&px, bg)).unwrap();
                    std::fs::write(dir.join(format!("{tn}-notice-{name}@{s}x.png")), img.to_png().unwrap()).unwrap();
                }
            }
        }
    }

    #[test]
    fn the_window_fits_the_panel_and_its_shadow() {
        for th in [&crate::theme::DARK, &crate::theme::LIGHT] {
            for s in [1.0, 1.5] {
                let note = Note::new("Saved \u{b7} 2026-10-10_19-48.jpg".into(), ToastKind::Success);
                let (w, h, px) = render(&note, th, s, 1.0);
                assert_eq!((w, h), size(&note.text, s));
                let a = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
                // Edges stay clear: the shadow fits in the window.
                for x in 0..w {
                    assert!(a(x, 0) < 8 && a(x, h - 1) < 8, "top/bottom edge at x={x} s={s}");
                }
                for y in 0..h {
                    assert!(a(0, y) < 8 && a(w - 1, y) < 8, "side edge at y={y} s={s}");
                }
                let mid = (TOP * s + 18.0 * s) as u32;
                assert!(a(w / 2, mid) > 240, "the panel is opaque");
            }
        }
    }
}
