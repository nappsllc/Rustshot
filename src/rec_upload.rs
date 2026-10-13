//! Recording uploads behind the recording toast's **Copy link**: to the
//! configured destination (Imgur's video upload, or the S3 bucket), on a
//! worker. With *Upload recordings automatically* on, the upload starts
//! as soon as the recording is saved ([`start`]), so the link is ready
//! (or nearly) when Copy link is clicked; otherwise the click starts it.
//! Either way a click while it runs waits for it, and a click after it
//! finished copies the link at once. A failed upload is tried again by
//! the next click.
//!
//! [`Uploads`] is the bookkeeping (pure, tested); the glue below runs it
//! with the real upload and clipboard.

use crate::editor::style::ToastKind;
use crate::notice::Note;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Told the result once the upload is done (the toast's next note).
pub type Waiter = Box<dyn FnOnce(&Result<String, String>) + Send>;

/// One recording's upload.
enum Up {
    Running(Vec<Waiter>),
    Done(Result<String, String>),
}

/// What a Copy link click finds.
#[derive(Debug, PartialEq)]
pub enum Ask {
    /// Uploaded: this link, at once.
    Ready(String),
    /// Running: told when done.
    Wait,
    /// Not started (or failed before): start it, told when done.
    Start,
}

/// The recordings' uploads (the newest [`KEEP`]).
#[derive(Default)]
pub struct Uploads {
    list: Vec<(PathBuf, Up)>,
}

/// How many finished uploads are remembered.
pub const KEEP: usize = 8;

impl Uploads {
    fn find(&mut self, file: &Path) -> Option<&mut Up> {
        self.list.iter_mut().find(|(p, _)| p == file).map(|(_, u)| u)
    }

    /// Start uploading `file` unless it is uploading or uploaded; whether
    /// the caller must start it.
    pub fn begin(&mut self, file: &Path) -> bool {
        match self.find(file) {
            Some(Up::Running(_)) | Some(Up::Done(Ok(_))) => false,
            Some(u) => {
                *u = Up::Running(Vec::new());
                true
            }
            None => {
                self.list.push((file.to_path_buf(), Up::Running(Vec::new())));
                if self.list.len() > KEEP {
                    // Drop the oldest finished one (a running one has a worker).
                    if let Some(i) = self.list.iter().position(|(_, u)| matches!(u, Up::Done(_))) {
                        self.list.remove(i);
                    }
                }
                true
            }
        }
    }

    /// Copy link on `file`'s toast; `w` is told the result unless it is
    /// [`Ask::Ready`] already.
    pub fn ask(&mut self, file: &Path, w: Waiter) -> Ask {
        if let Some(Up::Done(Ok(link))) = self.find(file) {
            return Ask::Ready(link.clone());
        }
        let start = self.begin(file);
        if let Some(Up::Running(ws)) = self.find(file) {
            ws.push(w);
        }
        if start { Ask::Start } else { Ask::Wait }
    }

    /// `file`'s upload ended: whoever waits for it.
    pub fn finish(&mut self, file: &Path, res: Result<String, String>) -> Vec<Waiter> {
        match self.find(file) {
            Some(u) => match std::mem::replace(u, Up::Done(res)) {
                Up::Running(ws) => ws,
                Up::Done(_) => Vec::new(),
            },
            None => {
                self.list.push((file.to_path_buf(), Up::Done(res)));
                Vec::new()
            }
        }
    }
}

static UPLOADS: Mutex<Uploads> = Mutex::new(Uploads { list: Vec::new() });

fn lock() -> std::sync::MutexGuard<'static, Uploads> {
    UPLOADS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Upload `file` now on a worker (*Upload recordings automatically*);
/// its record is filed with History entry `entry` when done.
pub fn start(file: &Path, entry: Option<String>) {
    if lock().begin(file) {
        spawn(file.to_path_buf(), entry);
    }
}

fn spawn(path: PathBuf, entry: Option<String>) {
    let cfg = crate::config::load();
    let p2 = path.clone();
    let spawned = std::thread::Builder::new().name("upload-recording".into()).spawn(move || {
        let res = crate::notice::upload_recording(&path, &cfg);
        if let Ok(link) = &res {
            // Its record: the recording's History entry, or uploads.json.
            let root = crate::history::find_root();
            let at = root.as_deref().zip(entry.as_deref()).filter(|_| cfg.history);
            crate::uploads::finished(link, at);
        }
        let waiters = lock().finish(&path, res.clone());
        for w in waiters {
            w(&res);
        }
    });
    if let Err(e) = spawned {
        for w in lock().finish(&p2, Err(format!("{e}"))) {
            w(&Err(format!("{e}")));
        }
    }
}

/// The note after a Copy link click: the link copied now, else
/// "Uploading…" with the result on `done` later.
pub fn copy_link(file: &Path, entry: Option<String>, done: impl FnOnce(Note) + Send + 'static) -> Note {
    let w: Waiter = Box::new(move |res: &Result<String, String>| {
        let copied = res.as_ref().is_ok_and(|u| crate::export::copy_text_to_clipboard(u).is_ok());
        done(crate::notice::upload_note(res, copied));
    });
    let ask = lock().ask(file, w);
    match ask {
        Ask::Ready(link) => {
            let copied = crate::export::copy_text_to_clipboard(&link).is_ok();
            crate::notice::upload_note(&Ok(link), copied)
        }
        Ask::Start => {
            spawn(file.to_path_buf(), entry);
            uploading()
        }
        Ask::Wait => uploading(),
    }
}

fn uploading() -> Note {
    Note { ttl: crate::notice::UPLOAD_TTL, ..Note::plain("Uploading\u{2026}", ToastKind::Info) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn told(log: &Arc<Mutex<Vec<String>>>, tag: &'static str) -> Waiter {
        let log = log.clone();
        Box::new(move |r: &Result<String, String>| log.lock().unwrap().push(format!("{tag}: {}", r.clone().unwrap_or_else(|e| e))))
    }

    /// Started at Stop, the link is ready for the click; a click while it
    /// runs waits; a failure is tried again by the next click.
    #[test]
    fn copy_link_waits_for_or_reuses_the_upload() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut u = Uploads::default();
        let a = Path::new("a.mp4");
        assert!(u.begin(a), "started at Stop");
        assert!(!u.begin(a), "once");
        assert_eq!(u.ask(a, told(&log, "click1")), Ask::Wait);
        assert_eq!(u.ask(a, told(&log, "click2")), Ask::Wait);
        let ws = u.finish(a, Ok("https://cdn.example.com/a.mp4".into()));
        assert_eq!(ws.len(), 2);
        ws.into_iter().for_each(|w| w(&Ok("https://cdn.example.com/a.mp4".into())));
        assert_eq!(*log.lock().unwrap(), ["click1: https://cdn.example.com/a.mp4", "click2: https://cdn.example.com/a.mp4"]);
        assert_eq!(u.ask(a, told(&log, "late")), Ask::Ready("https://cdn.example.com/a.mp4".into()));
        assert!(!u.begin(a), "never uploaded twice");

        let b = Path::new("b.mp4");
        assert_eq!(u.ask(b, told(&log, "b")), Ask::Start, "a click starts it");
        assert_eq!(u.finish(b, Err("offline".into())).len(), 1);
        assert_eq!(u.ask(b, told(&log, "retry")), Ask::Start, "a failure is tried again");
    }

    #[test]
    fn only_the_newest_finished_uploads_are_kept() {
        let mut u = Uploads::default();
        for i in 0..20 {
            let p = PathBuf::from(format!("{i}.mp4"));
            u.begin(&p);
            u.finish(&p, Ok(format!("l{i}")));
        }
        assert!(u.list.len() <= KEEP + 1);
        assert!(u.list.iter().any(|(p, _)| p == Path::new("19.mp4")));
    }
}
