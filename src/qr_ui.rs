//! Read QR code (the overlay's Copy text menu and Q, the Read QR code
//! hotkey, the Edit window): the selection is decoded on a worker thread
//! ([`spawn`]), and the daemon's loop shows the result ([`poll`]) as a
//! toast with the text and Copy / Open link. Open link is offered only for
//! an http or https link, which the toast shows first; nothing opens
//! until it is clicked. The buttons act on the last result read.

use crate::editor::style::ToastKind;
use crate::notice::{ACTION_TTL, Note, NoteAct};
use crate::pixbuf::PixBuf;
use crate::theme::Theme;
use std::sync::Mutex;

/// The reads whose toasts may still be up (newest last): each toast's
/// buttons act on its own read (its id is in the note's `entry`, `qr:<id>`).
static READS: Mutex<Vec<(u64, Vec<String>)>> = Mutex::new(Vec::new());
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
const KEEP: usize = 8;

/// Results waiting for the daemon's loop: the note, where, the theme.
type Done = (Note, (i32, i32), Theme);
/// (With the thread that asked: its loop shows it, never another's.)
static DONE: Mutex<Vec<(std::thread::ThreadId, Done)>> = Mutex::new(Vec::new());

/// The longest text a toast shows of a code (and of a link's path).
const SHOW_MAX: usize = 140;
const PATH_MAX: usize = 60;

/// Control and bidi-override characters as `\u{..}` (a terminal or a
/// toast never runs or reorders them).
pub fn printable(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let bidi = ('\u{202a}'..='\u{202e}').contains(&c) || ('\u{2066}'..='\u{2069}').contains(&c) || c == '\u{200e}' || c == '\u{200f}';
        if (c.is_control() && c != '\n' && c != '\t') || bidi {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}\u{2026}", s.chars().take(max).collect::<String>()) }
}

fn shown(s: &str) -> String {
    let one: String = printable(s).split_whitespace().collect::<Vec<_>>().join(" ");
    cut(&one, SHOW_MAX)
}

/// A link as the toast shows it: the scheme, host and port in full (an
/// IDN host as punycode, its Unicode form beside it), then the path cut.
pub fn shown_link(url: &str) -> Option<String> {
    let p = crate::qr::link_parts(url)?;
    let mut s = p.head.clone();
    if let Some(u) = &p.unicode {
        s.push_str(&format!(" ({})", printable(u)));
    }
    if !p.rest.is_empty() && p.rest != "/" {
        s.push_str(&cut(&printable(p.rest), PATH_MAX));
    }
    Some(s)
}

/// The first code that is a link Open link may open.
pub fn link_of(texts: &[String]) -> Option<String> {
    texts.iter().find_map(|s| crate::qr::openable_link(s).map(str::to_string))
}

/// The toast for `texts` (the codes found, in reading order); its buttons
/// act on them.
pub fn note(texts: Vec<String>) -> Note {
    if texts.is_empty() {
        return Note::plain(tr!("qr.none"), ToastKind::Info);
    }
    let link = link_of(&texts);
    // The link first (what Open link would open), then the rest.
    let mut parts: Vec<String> = Vec::new();
    if let Some(l) = link.as_deref().and_then(shown_link) {
        parts.push(l);
    }
    let mut skipped = false;
    for s in &texts {
        if !skipped && link.is_some() && link.as_deref() == crate::qr::openable_link(s) {
            skipped = true;
            continue;
        }
        parts.push(shown(s));
    }
    let text = format!("{} \u{b7} {}", trn!("qr.found", texts.len()), parts.join(" \u{b7} "));
    let mut actions = vec![NoteAct::QrCopy];
    if link.is_some() {
        actions.push(NoteAct::QrOpen);
    }
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    {
        let mut r = READS.lock().unwrap_or_else(|e| e.into_inner());
        r.push((id, texts));
        let n = r.len();
        if n > KEEP {
            r.drain(..n - KEEP);
        }
    }
    let mut n = Note::plain(text, ToastKind::Success);
    n.ttl = ACTION_TTL;
    n.actions = actions;
    n.entry = Some(format!("qr:{id}"));
    n
}

/// The codes of the read a toast shows (its `entry`).
fn texts_of(entry: Option<&str>) -> Vec<String> {
    let Some(id) = entry.and_then(|e| e.strip_prefix("qr:")).and_then(|v| v.parse::<u64>().ok()) else { return Vec::new() };
    READS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(i, _)| *i == id).map(|(_, t)| t.clone()).unwrap_or_default()
}

/// Copy: every code's text of that toast's read, one per line.
pub fn copy_for(entry: Option<&str>) -> Note {
    let all = texts_of(entry).join("\n");
    if all.is_empty() {
        return Note::plain(tr!("qr.none"), ToastKind::Info);
    }
    #[cfg(test)]
    let res: anyhow::Result<()> = Ok(());
    #[cfg(not(test))]
    let res = crate::export::copy_text_to_clipboard(&all);
    match res {
        Ok(()) => Note::plain(tr!("qr.copied"), ToastKind::Success),
        Err(e) => Note::plain(format!("Copy failed: {e:#}"), ToastKind::Error),
    }
}

/// Open link: the link that toast showed, if it is http or https.
pub fn open_for(entry: Option<&str>) {
    if let Some(url) = link_of(&texts_of(entry)) {
        open_url(&url);
    }
}
/// Hand an http(s) URL to the browser (anything else is refused).
fn open_url(url: &str) {
    let Some(url) = crate::qr::openable_link(url) else { return };
    #[cfg(test)]
    {
        OPENED.with(|o| o.borrow_mut().push(url.to_string()));
    }
    #[cfg(all(windows, not(test)))]
    {
        use std::ffi::OsStr;
        if let Some(exe) = crate::proc_win::system_exe("rundll32.exe") {
            let _ = crate::proc_win::spawn_detached(&exe, &[OsStr::new("url.dll,FileProtocolHandler"), OsStr::new(url)], &[]);
        }
    }
    #[cfg(all(target_os = "macos", not(test)))]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(not(any(windows, target_os = "macos")), not(test)))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

#[cfg(test)]
thread_local! {
    pub static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Read the codes in `img` off the UI thread; [`poll`] hands the note
/// back (to show at `at`).
pub fn spawn(img: PixBuf, at: (i32, i32), th: Theme) {
    let me = std::thread::current().id();
    crate::busy::spawn(move || {
        let texts = crate::qr::read(&img);
        let n = note(texts);
        DONE.lock().unwrap_or_else(|e| e.into_inner()).push((me, (n, at, th)));
    });
}

/// The notes of the reads that finished.
pub fn poll() -> Vec<Done> {
    let me = std::thread::current().id();
    let mut all = DONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *all).into_iter().partition(|(t, _)| *t == me);
    *all = rest;
    mine.into_iter().map(|(_, d)| d).collect()
}

/// Show what [`poll`] returned (the notice, else a tray balloon).
pub fn show_done() {
    for (n, at, th) in poll() {
        let text = n.text.clone();
        if !crate::notice::show(n, at, th) {
            crate::tray::notify(&text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toast_shows_the_link_first_and_never_opens_it_by_itself() {
        OPENED.with(|o| o.borrow_mut().clear());
        let n = note(vec!["hello".into(), "https://example.com/a".into()]);
        assert!(n.text.starts_with("2 QR codes \u{b7} https://example.com/a"), "{}", n.text);
        assert!(n.text.contains("hello"));
        assert_eq!(n.actions, vec![NoteAct::QrCopy, NoteAct::QrOpen]);
        assert!(OPENED.with(|o| o.borrow().is_empty()), "not opened");
        let first = n.entry.clone();
        // no link: no Open link
        let m = note(vec!["javascript:alert(1)".into()]);
        assert_eq!(m.actions, vec![NoteAct::QrCopy]);
        open_for(m.entry.as_deref());
        assert!(OPENED.with(|o| o.borrow().is_empty()));
        // each toast acts on its own read, not the newest
        open_for(first.as_deref());
        assert_eq!(OPENED.with(|o| o.borrow().clone()), vec!["https://example.com/a".to_string()]);
        assert_eq!(note(Vec::new()).text, tr!("qr.none"));
        assert!(note(vec!["x".repeat(500)]).text.chars().count() < 200);
    }

    /// The host is never cut off, an IDN shows its punycode, controls are
    /// shown as escapes; user info gets no Open link.
    #[test]
    fn the_link_head_is_shown_in_full() {
        let long = format!("https://accounts.google.com.sign-in-{}.evil.example/{}", vec!["0".repeat(50); 3].join("."), "p".repeat(200));
        let n = note(vec![long]);
        assert!(n.text.contains(".evil.example/ppp"), "{}", n.text);
        assert!(n.text.ends_with('\u{2026}'));
        let n = note(vec!["https://\u{430}pple.com/".into()]);
        assert!(n.text.contains("https://xn--pple-43d.com (https://\u{430}pple.com)"), "{}", n.text);
        let n = note(vec!["https://www.paypal.com-x@evil.example/".into()]);
        assert_eq!(n.actions, vec![NoteAct::QrCopy]);
        assert_eq!(printable("a\u{1b}]52;c;x\u{7}b\u{202e}"), "a\\u{1b}]52;c;x\\u{7}b\\u{202e}");
        assert!(!note(vec!["\u{1b}[2J".into()]).text.contains('\u{1b}'));
    }
}