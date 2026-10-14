//! Read QR code (the overlay's Copy text menu and Q, the Read QR code
//! hotkey, the Edit window): the selection is decoded on a worker thread
//! ([`spawn`]), and the daemon's loop shows the result ([`poll`]) as a
//! toast with the text and Copy / Open link. Open link is offered only for
//! an http or https link, which the toast shows first; nothing opens
//! until it is clicked. The buttons act on the last result read.

use crate::editor::style::ToastKind;
use crate::notice::{ACTION_TTL, Note, NoteAct};
use crate::pixbuf::PixBuf;
use crate::sharex_text as t;
use crate::theme::Theme;
use std::sync::Mutex;

/// The last codes read (what Copy and Open link act on).
static LAST: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Results waiting for the daemon's loop: the note, where, the theme.
type Done = (Note, (i32, i32), Theme);
/// (With the thread that asked: its loop shows it, never another's.)
static DONE: Mutex<Vec<(std::thread::ThreadId, Done)>> = Mutex::new(Vec::new());

/// The longest text a toast shows of a code.
const SHOW_MAX: usize = 140;

fn shown(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= SHOW_MAX { one } else { format!("{}\u{2026}", one.chars().take(SHOW_MAX).collect::<String>()) }
}

/// The first code that is a link Open link may open.
pub fn link_of(texts: &[String]) -> Option<String> {
    texts.iter().find_map(|s| crate::qr::openable_link(s).map(str::to_string))
}

/// The toast for `texts` (the codes found, in reading order); they become
/// what its buttons act on.
pub fn note(texts: Vec<String>) -> Note {
    if texts.is_empty() {
        return Note::plain(t::QR_NONE, ToastKind::Info);
    }
    let link = link_of(&texts);
    // The link first (what Open link would open), then the rest.
    let mut parts: Vec<String> = Vec::new();
    if let Some(l) = &link {
        parts.push(shown(l));
    }
    let mut skipped = false;
    for s in &texts {
        if !skipped && link.is_some() && link.as_deref() == crate::qr::openable_link(s) {
            skipped = true;
            continue;
        }
        parts.push(shown(s));
    }
    let text = format!("{} \u{b7} {}", t::qr_found(texts.len()), parts.join(" \u{b7} "));
    let mut actions = vec![NoteAct::QrCopy];
    if link.is_some() {
        actions.push(NoteAct::QrOpen);
    }
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = texts;
    let mut n = Note::plain(text, ToastKind::Success);
    n.ttl = ACTION_TTL;
    n.actions = actions;
    n
}

/// Copy: every code's text, one per line.
pub fn copy_last() -> Note {
    let all = LAST.lock().unwrap_or_else(|e| e.into_inner()).join("\n");
    if all.is_empty() {
        return Note::plain(t::QR_NONE, ToastKind::Info);
    }
    #[cfg(test)]
    let res: anyhow::Result<()> = Ok(());
    #[cfg(not(test))]
    let res = crate::export::copy_text_to_clipboard(&all);
    match res {
        Ok(()) => Note::plain(t::QR_COPIED, ToastKind::Success),
        Err(e) => Note::plain(format!("Copy failed: {e:#}"), ToastKind::Error),
    }
}

/// Open link: the link the toast showed, if it is http or https.
pub fn open_last() {
    let link = link_of(&LAST.lock().unwrap_or_else(|e| e.into_inner()));
    if let Some(url) = link {
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
        open_last();
        assert_eq!(OPENED.with(|o| o.borrow().clone()), vec!["https://example.com/a".to_string()]);
        // no link: no Open link
        let n = note(vec!["javascript:alert(1)".into()]);
        assert_eq!(n.actions, vec![NoteAct::QrCopy]);
        OPENED.with(|o| o.borrow_mut().clear());
        open_last();
        assert!(OPENED.with(|o| o.borrow().is_empty()));
        assert_eq!(note(Vec::new()).text, t::QR_NONE);
        assert!(note(vec!["x".repeat(500)]).text.chars().count() < 200);
    }
}
