#![cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))] // no notice window elsewhere
//! The confirmation toast after a capture: the overlay closes at once, so
//! a small notice ("Copied to clipboard", "Saved · shot.png",
//! "Uploading…" then "Link copied · i.imgur.com/…") says what happened,
//! bottom-centre on the work area of the monitor where the capture was.
//! It never takes focus; a plain note lets clicks through, one with
//! buttons (after a recording) takes them.
//!
//! - Windows: a layered window, excluded from capture (`notice_win.rs`).
//! - Linux X11: an override-redirect ARGB window on its own thread and
//!   Display (`notice_x11.rs`); X11 cannot exclude it from capture, so
//!   [`hide`] waits until it is unmapped before a capture starts.
//! - macOS: a transparent non-activating panel on the main thread
//!   (`notice_macos.rs`), ticked by the daemon's driver ([`tick`]).
//! - Wayland: no window (`show` returns false; the caller falls back to
//!   stdout and `tray::notify`): an XWayland toplevel cannot place itself,
//!   let clicks through or stay unfocused.

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

/// A note with buttons stays this long (and while the pointer is on it).
pub const ACTION_TTL: Duration = Duration::from_secs(6);

/// A button on a note (the toast after a recording).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteAct {
    /// Upload the file and copy its link (only with upload configured).
    CopyLink,
    /// Copy the file itself (a GIF; Windows: as a file drop, CF_HDROP).
    Copy,
    Open,
    ShowInFolder,
}

impl NoteAct {
    pub fn label(self) -> &'static str {
        match self {
            NoteAct::CopyLink => "Copy link",
            NoteAct::Copy => "Copy",
            NoteAct::Open => "Open",
            NoteAct::ShowInFolder => "Show in folder",
        }
    }

    /// The first button is the accent one.
    fn primary(self) -> bool {
        matches!(self, NoteAct::CopyLink | NoteAct::Copy)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub text: String,
    pub kind: ToastKind,
    /// Visible time before the fade-out.
    pub ttl: Duration,
    /// Buttons after the text: then the note takes clicks (it is not
    /// click-through) and stays [`ACTION_TTL`].
    pub actions: Vec<NoteAct>,
    /// The file the buttons act on.
    pub file: Option<std::path::PathBuf>,
}

impl Note {
    fn new(text: String, kind: ToastKind) -> Note {
        let ttl = if kind == ToastKind::Error { ERROR_TTL } else { TTL };
        Note { text, kind, ttl, actions: Vec::new(), file: None }
    }

    /// A plain note (no buttons): `text` with the usual time.
    pub fn plain(text: impl Into<String>, kind: ToastKind) -> Note {
        Note::new(text.into(), kind)
    }
}

/// "00:42", "12:07", "1:02:03".
fn clock(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// "820 KB", "1.4 MB", "36 MB".
pub fn file_size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb < 1.0 {
        format!("{} KB", (bytes as f64 / 1000.0).round().max(1.0) as u64)
    } else if mb < 10.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{mb:.0} MB")
    }
}

/// The toast after a recording: "Saved <file> · 00:42 · 1.4 MB" (what was
/// left out after it) with Copy link (an MP4, when uploading is set up) or
/// Copy (a GIF: the file itself), Open and Show in folder; it stays
/// [`ACTION_TTL`].
pub fn recording_saved(path: &std::path::Path, length: Duration, bytes: u64, warnings: &[String], upload: bool) -> Note {
    let mut text = format!("Saved {} \u{b7} {} \u{b7} {}", file_name(path), clock(length), file_size(bytes));
    for w in warnings {
        text.push_str(" \u{2014} ");
        text.push_str(&reason(w));
    }
    let gif = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif"));
    let mut actions = Vec::new();
    if gif {
        actions.push(NoteAct::Copy);
    } else if upload {
        actions.push(NoteAct::CopyLink);
    }
    actions.extend([NoteAct::Open, NoteAct::ShowInFolder]);
    let kind = if warnings.is_empty() { ToastKind::Success } else { ToastKind::Error };
    Note { text, kind, ttl: ACTION_TTL, actions, file: Some(path.to_path_buf()) }
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
            Done::SavedDesktop(p) if single => format!("Saved to Desktop \u{b7} {}", file_name(p)),
            Done::SavedDesktop(p) => format!("Saved {} to Desktop", file_name(p)),
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
        Note { ttl: UPLOAD_TTL, ..Note::new(text, kind) }
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

/// A note with buttons in its window (physical px): the window's size,
/// the panel, where the text starts, and the buttons.
#[derive(Clone, Debug, PartialEq)]
pub struct ActLayout {
    pub size: (u32, u32),
    pub panel: FRect,
    pub text_x: f32,
    pub buttons: Vec<(NoteAct, FRect)>,
}

/// Board metrics (logical px): the panel 36 high, padding 10 left and 6
/// right, the icon 16 and 8 to the text, buttons 24 high 8 apart with 10
/// inside, 11.5 px labels.
const ACT_H: f32 = 36.0;
const BTN_H: f32 = 24.0;
const BTN_PAD: f32 = 10.0;
const BTN_TEXT: f32 = 11.5;

fn text_w(s: f32, size: f32, t: &str) -> f32 {
    crate::uifb::text_width(crate::fonts::ui_font(), size * s, t)
}

/// Lay out `note`'s text and buttons at scale `s`.
pub fn act_layout(note: &Note, s: f32) -> ActLayout {
    let px = |v: f32| v * s;
    let (x0, y0) = (px(SIDE), px(TOP));
    let text_x = x0 + px(10.0 + 16.0 + 8.0);
    let mut x = text_x + text_w(s, 12.0, &note.text);
    let mut buttons = Vec::new();
    for a in &note.actions {
        x += px(8.0);
        let w = (text_w(s, BTN_TEXT, a.label()) + px(2.0 * BTN_PAD)).ceil();
        buttons.push((*a, FRect { x: x.round(), y: (y0 + px(ACT_H - BTN_H) / 2.0).round(), w, h: px(BTN_H).round() }));
        x += w;
    }
    let panel = FRect { x: x0, y: y0, w: (x + px(6.0) - x0).ceil(), h: px(ACT_H).round() };
    let size = ((panel.x1() + px(SIDE)).ceil() as u32, (panel.y1() + px(BOTTOM)).ceil() as u32);
    ActLayout { size, panel, text_x, buttons }
}

/// The button of `l` under `p` (window px).
pub fn act_at(l: &ActLayout, p: crate::objects::Pt) -> Option<NoteAct> {
    l.buttons.iter().find(|(_, r)| p.x >= r.x && p.x < r.x1() && p.y >= r.y && p.y < r.y1()).map(|b| b.0)
}

/// The window size for `note` (with its buttons, if any).
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS sizes from the pixels
pub fn note_size(note: &Note, s: f32) -> (u32, u32) {
    if note.actions.is_empty() { size(&note.text, s) } else { act_layout(note, s).size }
}

/// Draw a note with buttons (the board's "After stopping" toast); `hover`
/// is the button under the pointer.
fn draw_actions(f: &mut crate::uifb::Fb, ui: &Ui, note: &Note, l: &ActLayout, hover: Option<NoteAct>, k: f32) {
    use crate::editor::style::{fill, ring, surface, ALL_LAYERS};
    let th = ui.th;
    let s = ui.s;
    let font = crate::fonts::ui_font();
    let rise = 4.0 * s * (1.0 - k);
    let mv = |r: FRect| FRect { y: r.y + rise, ..r };
    let panel = mv(l.panel);
    surface(f, ui, panel, 12.0, &ALL_LAYERS, k);
    let cy = panel.y + panel.h / 2.0;
    let (icon, color) = match note.kind {
        ToastKind::Info => ("info", th.accent),
        ToastKind::Success => ("okc", th.success),
        ToastKind::Error => ("alert", th.error),
    };
    let isz = 16.0 * s;
    crate::icon_path::draw_icon(f, icon, panel.x + 10.0 * s, cy - isz / 2.0, isz, color.fade(k));
    let tpx = 12.0 * s;
    f.draw_text(font, tpx, &note.text, l.text_x, cy - crate::uifb::text_height(font, tpx) / 2.0, th.text.fade(k));
    for &(a, r) in &l.buttons {
        let r = mv(r);
        let rad = 6.0 * s;
        let hot = hover == Some(a);
        let fg = if a.primary() {
            fill(f, r, rad, if hot { th.accent_bg_hover } else { th.accent_bg }.fade(k));
            ring(f, ui, r, rad, th.accent_ring.fade(k));
            th.accent_fg
        } else {
            fill(f, r, rad, if hot { th.bg_pressed } else { th.bg_hover }.fade(k));
            th.text
        };
        let bpx = BTN_TEXT * s;
        let tx = r.x + (r.w - crate::uifb::text_width(font, bpx, a.label())) / 2.0;
        f.draw_text(font, bpx, a.label(), tx, r.y + r.h / 2.0 - crate::uifb::text_height(font, bpx) / 2.0, fg.fade(k));
    }
}

/// [`render`] for any note, `hover` over one of its buttons.
pub fn render_note(note: &Note, th: &Theme, s: f32, k: f32, hover: Option<NoteAct>) -> (u32, u32, Vec<u8>) {
    if note.actions.is_empty() {
        return render(note, th, s, k);
    }
    let l = act_layout(note, s);
    let ui = Ui { th, s, font: Some(crate::fonts::ui_font()) };
    let (w, h) = l.size;
    let px = crate::rec_ui::render_layer(w as usize, h as usize, |f| draw_actions(f, &ui, note, &l, hover, k));
    (w, h, px)
}

/// Whether uploading is set up (an Imgur client id): Copy link is offered.
pub fn upload_configured(cfg: &crate::config::Config) -> bool {
    !cfg.upload_client_id.trim().is_empty()
}

/// Carry out button `a` of a note about `file`: the note to show next, if
/// any ("Copied", "Uploading…"; an upload's result follows on `done`).
pub fn run_action(a: NoteAct, file: &std::path::Path, done: impl FnOnce(Note) + Send + 'static) -> Option<Note> {
    match a {
        NoteAct::Open => {
            crate::actions::open_path(file);
            None
        }
        NoteAct::ShowInFolder => {
            show_in_folder(file);
            None
        }
        NoteAct::Copy => Some(match copy_file(file) {
            Ok(()) => Note::plain("Copied to clipboard", ToastKind::Success),
            Err(e) => Note::plain(format!("Copy failed: {}", reason(&format!("{e:#}"))), ToastKind::Error),
        }),
        NoteAct::CopyLink => {
            let path = file.to_path_buf();
            let cfg = crate::config::load();
            let spawned = std::thread::Builder::new().name("upload-recording".into()).spawn(move || {
                let res = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display())).and_then(|b| upload(&b, &cfg.upload_client_id));
                let copied = res.as_ref().is_ok_and(|u| crate::export::copy_text_to_clipboard(u).is_ok());
                done(upload_note(&res, copied));
            });
            Some(match spawned {
                Ok(_) => Note { ttl: UPLOAD_TTL, ..Note::plain("Uploading\u{2026}", ToastKind::Info) },
                Err(e) => Note::plain(format!("Upload failed: {e}"), ToastKind::Error),
            })
        }
    }
}

/// Upload `bytes` (tests never do).
fn upload(bytes: &[u8], client_id: &str) -> Result<String, String> {
    #[cfg(test)]
    {
        let _ = (bytes, client_id);
        Err("no uploader in tests".into())
    }
    #[cfg(not(test))]
    {
        crate::export::do_upload(bytes, client_id)
    }
}

/// Open the folder with `file` selected (Windows Explorer, macOS Finder;
/// elsewhere the folder).
pub fn show_in_folder(file: &std::path::Path) {
    #[cfg(test)]
    {
        let _ = file;
    }
    #[cfg(all(windows, not(test)))]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let args: Vec<u16> = std::ffi::OsString::from(format!("/select,\"{}\"", file.display())).encode_wide().chain(Some(0)).collect();
        unsafe {
            ShellExecuteW(None, w!("open"), w!("explorer.exe"), PCWSTR(args.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(file).spawn();
    }
    #[cfg(all(not(windows), not(target_os = "macos"), not(test)))]
    {
        if let Some(dir) = file.parent() {
            crate::actions::open_path(dir);
        }
    }
}

/// Put `file` itself on the clipboard: on Windows as a file drop
/// (`CF_HDROP`, what Explorer and chat apps paste); elsewhere its path.
pub fn copy_file(file: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(test)]
    {
        let _ = file;
        Ok(())
    }
    #[cfg(all(windows, not(test)))]
    {
        copy_hdrop(file)
    }
    #[cfg(all(not(windows), not(test)))]
    {
        crate::export::copy_text_to_clipboard(&file.display().to_string())
    }
}

/// The `DROPFILES` block for `file`: the header, then the path in UTF-16,
/// double NUL-terminated.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn hdrop_bytes(file: &std::path::Path) -> Vec<u8> {
    // DROPFILES: pFiles (u32), pt (2 × i32), fNC (i32), fWide (i32).
    let mut out = Vec::new();
    out.extend_from_slice(&20u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&1i32.to_le_bytes());
    for u in file.as_os_str().to_string_lossy().encode_utf16().chain([0, 0]) {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

#[cfg(all(windows, not(test)))]
fn copy_hdrop(file: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    const CF_HDROP: u32 = 15;
    let bytes = hdrop_bytes(file);
    unsafe {
        OpenClipboard(None).context("open the clipboard")?;
        let r = (|| -> anyhow::Result<()> {
            EmptyClipboard().context("empty the clipboard")?;
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).context("GlobalAlloc")?;
            let p = GlobalLock(h) as *mut u8;
            if p.is_null() {
                anyhow::bail!("GlobalLock failed");
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            let _ = GlobalUnlock(h);
            // The clipboard owns the memory from here.
            SetClipboardData(CF_HDROP, Some(HANDLE(h.0))).context("SetClipboardData")?;
            Ok(())
        })();
        let _ = CloseClipboard();
        r
    }
}

/// Where a `size` window goes on a monitor's work area (physical px):
/// centred, its bottom on the work area's (the panel then sits the
/// toast's 28 px above it, clear of the taskbar).
pub fn place(work: crate::capture::IRect, (w, h): (u32, u32)) -> (i32, i32) {
    let x = work.0 + (work.2 as i32 - w as i32) / 2;
    let y = work.1 + work.3 as i32 - h as i32;
    (x, y)
}

/// The rect to [`place`] a note in: the work area of the monitor with
/// `at` (else the nearest one), i.e. the monitor clipped to the desktop's
/// work area `net` (X11 `_NET_WORKAREA` spans every monitor; macOS: the
/// screen's visible frame) when they overlap, else the whole monitor.
#[cfg_attr(windows, allow(dead_code))] // Windows asks the monitor (rcWork)
pub fn work_area(mons: &[crate::capture::IRect], net: Option<crate::capture::IRect>, at: (i32, i32)) -> crate::capture::IRect {
    let dist = |m: &crate::capture::IRect| {
        let dx = (m.0 - at.0).max(at.0 - (m.0 + m.2 as i32 - 1)).max(0) as i64;
        let dy = (m.1 - at.1).max(at.1 - (m.1 + m.3 as i32 - 1)).max(0) as i64;
        dx * dx + dy * dy
    };
    let Some(mon) = mons.iter().filter(|m| m.2 > 0 && m.3 > 0).min_by_key(|m| dist(m)).copied() else {
        return net.unwrap_or((at.0, at.1, 0, 0));
    };
    let Some(n) = net else { return mon };
    let (x0, y0) = (mon.0.max(n.0), mon.1.max(n.1));
    let (x1, y1) = ((mon.0 + mon.2 as i32).min(n.0 + n.2 as i32), (mon.1 + mon.3 as i32).min(n.1 + n.3 as i32));
    if x1 > x0 && y1 > y0 { (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32) } else { mon }
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
#[cfg(target_os = "linux")]
#[path = "notice_x11.rs"]
mod x11;
#[cfg(target_os = "macos")]
#[path = "notice_macos.rs"]
mod macos;

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
    #[cfg(all(target_os = "linux", not(test)))]
    {
        x11::show(note, at, th)
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos::show(note, at, th)
    }
    #[cfg(all(not(any(windows, target_os = "linux", target_os = "macos")), not(test)))]
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
    #[cfg(all(target_os = "linux", not(test)))]
    {
        x11::replace(note)
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos::replace(note)
    }
    #[cfg(all(not(any(windows, target_os = "linux", target_os = "macos")), not(test)))]
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
    #[cfg(all(target_os = "linux", not(test)))]
    x11::hide();
    #[cfg(all(target_os = "macos", not(test)))]
    macos::hide();
}

/// Advance the note where the driver runs it (macOS: AppKit's main
/// thread; Windows and X11 have their own timers). Some(fast) when the
/// driver's timer should switch: fast while a note is up, idle after.
pub fn tick() -> Option<bool> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        thread_local! {
            static FAST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        let up = macos::tick();
        (FAST.with(|f| f.replace(up)) != up).then_some(up)
    }
    #[cfg(not(all(target_os = "macos", not(test))))]
    {
        None
    }
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
        let desk = Done::SavedDesktop(PathBuf::from("/home/me/Desktop").join("b.png"));
        let n = compose(&[Done::Copied, desk.clone()], false).unwrap();
        assert_eq!((n.text.as_str(), n.kind), ("Copied \u{b7} Saved b.png to Desktop", ToastKind::Success));
        let n = compose(&[desk], false).unwrap();
        assert_eq!((n.text.as_str(), n.kind), ("Saved to Desktop \u{b7} b.png", ToastKind::Success));
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

    /// X11's `_NET_WORKAREA` spans the desktop: each monitor gets its part
    /// (a panel on one monitor only shortens that one); a point off every
    /// monitor goes to the nearest.
    #[test]
    fn the_work_area_is_the_monitor_less_panels() {
        let mons = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080)];
        // A 40 px bottom panel across both monitors.
        let net = Some((0, 0, 3840, 1040));
        assert_eq!(work_area(&mons, net, (100, 100)), (0, 0, 1920, 1040));
        assert_eq!(work_area(&mons, net, (2000, 900)), (1920, 0, 1920, 1040));
        assert_eq!(work_area(&mons, None, (2000, 900)), (1920, 0, 1920, 1080), "no window manager: the whole monitor");
        assert_eq!(work_area(&mons, net, (5000, 500)), (1920, 0, 1920, 1040), "nearest monitor");
        assert_eq!(work_area(&mons, Some((0, 0, 1920, 1040)), (2000, 10)), (1920, 0, 1920, 1080), "no overlap: the monitor");
        assert_eq!(work_area(&[], net, (5, 5)), (0, 0, 3840, 1040));
        let (x, y) = place(work_area(&mons, net, (2000, 900)), (300, 80));
        assert_eq!((x, y), (1920 + 810, 1040 - 80), "bottom-centre, above the panel");
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
            ("saved-desktop", compose(&[Done::SavedDesktop(PathBuf::from("/home/me/Desktop/2026-10-11_14-02.png"))], false)),
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

    /// After a recording: "Saved <file> · 00:42 · 1.4 MB" with Copy link
    /// (an MP4, uploading set up) or Copy (a GIF: the file), Open and Show
    /// in folder; it stays 6 s.
    #[test]
    fn the_toast_after_a_recording_has_buttons() {
        let mp4 = PathBuf::from("shots").join("rustshot-2026-10-10-2131.mp4");
        let n = recording_saved(&mp4, Duration::from_secs(42), 1_400_000, &[], true);
        assert_eq!(n.text, "Saved rustshot-2026-10-10-2131.mp4 \u{b7} 00:42 \u{b7} 1.4 MB");
        assert_eq!((n.kind, n.ttl), (ToastKind::Success, ACTION_TTL));
        assert_eq!(n.actions, [NoteAct::CopyLink, NoteAct::Open, NoteAct::ShowInFolder]);
        assert_eq!(n.file.as_deref(), Some(mp4.as_path()));
        let n = recording_saved(&mp4, Duration::from_secs(3725), 36_200_000, &[], false);
        assert_eq!(n.actions, [NoteAct::Open, NoteAct::ShowInFolder], "no upload set up: no Copy link");
        assert!(n.text.ends_with("1:02:05 \u{b7} 36 MB"), "{}", n.text);
        let gif = PathBuf::from("/home/me/Videos/clip.gif");
        let n = recording_saved(&gif, Duration::from_secs(7), 820_400, &["microphone not recorded: busy".into()], true);
        assert_eq!(n.actions, [NoteAct::Copy, NoteAct::Open, NoteAct::ShowInFolder], "a GIF: Copy copies the file");
        assert_eq!(n.text, "Saved clip.gif \u{b7} 00:07 \u{b7} 820 KB \u{2014} microphone not recorded: busy");
        assert_eq!(n.kind, ToastKind::Error, "something was left out");
        assert_eq!(NoteAct::ShowInFolder.label(), "Show in folder");
        assert!(upload_configured(&crate::config::Config::default()));
        assert!(!upload_configured(&crate::config::Config { upload_client_id: " ".into(), ..Default::default() }));
    }

    /// The buttons: 24 px high, 8 px apart, inside the 36 px panel, right
    /// of the text; hits find them; the window holds them and the shadow.
    #[test]
    fn note_buttons_layout_and_hits() {
        let n = recording_saved(&PathBuf::from("a.mp4"), Duration::from_secs(1), 10, &[], true);
        for s in [1.0f32, 2.0] {
            let l = act_layout(&n, s);
            assert_eq!(l.buttons.len(), 3);
            assert_eq!(l.panel.h, (36.0 * s).round());
            let mut x = l.text_x;
            for (a, r) in &l.buttons {
                assert_eq!(r.h, (24.0 * s).round(), "{a:?}");
                assert!(r.x > x && r.y >= l.panel.y && r.y1() <= l.panel.y1() && r.x1() <= l.panel.x1(), "{a:?} {r:?}");
                assert_eq!(act_at(&l, crate::objects::Pt::new(r.x + 2.0, r.y + 2.0)), Some(*a));
                x = r.x1();
            }
            assert_eq!(act_at(&l, crate::objects::Pt::new(l.text_x + 1.0, l.panel.y + 4.0)), None, "the text is no button");
            assert_eq!(note_size(&n, s), l.size);
            let (w, h, px) = render_note(&n, &crate::theme::DARK, s, 1.0, Some(NoteAct::Open));
            assert_eq!((w, h), l.size);
            assert!(px[3] < 8 && px[(w * h * 4 - 1) as usize] < 8, "the corners are clear");
        }
        // A plain note: the plain layout.
        let p = Note::plain("Copied to clipboard", ToastKind::Success);
        assert_eq!(note_size(&p, 1.0), size(&p.text, 1.0));
    }

    /// CF_HDROP: a DROPFILES header (files at byte 20, wide), then the
    /// path in UTF-16 with a double NUL.
    #[test]
    fn hdrop_block() {
        let b = hdrop_bytes(std::path::Path::new("C:\\a.gif"));
        assert_eq!(&b[..4], &20u32.to_le_bytes());
        assert_eq!(&b[16..20], &1i32.to_le_bytes(), "wide");
        let wide: Vec<u16> = b[20..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        assert_eq!(String::from_utf16_lossy(&wide[..wide.len() - 2]), "C:\\a.gif");
        assert_eq!(&wide[wide.len() - 2..], &[0, 0]);
    }

    /// The after-stop toast and "No mic input", dark and light, 1x and 2x.
    #[test]
    #[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
    fn render_recording_notice_previews() {
        let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let notes = [
            ("saved-mp4", recording_saved(&PathBuf::from("rustshot-2026-10-10-2131.mp4"), Duration::from_secs(42), 1_400_000, &[], true), Some(NoteAct::CopyLink)),
            ("saved-gif", recording_saved(&PathBuf::from("rustshot-2026-10-10-2131.gif"), Duration::from_secs(12), 3_100_000, &[], true), None),
            ("no-mic-input", Note::plain(crate::rec_ui::NO_MIC_INPUT, ToastKind::Error), None),
        ];
        for (tn, th, bg) in [("dark", &crate::theme::DARK, [38u8, 44, 58]), ("light", &crate::theme::LIGHT, [214, 222, 232])] {
            for s in [1.0f32, 2.0] {
                for (name, note, hover) in &notes {
                    let (w, h, px) = render_note(note, th, s, 1.0, *hover);
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
