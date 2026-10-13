//! Work this process runs on threads of its own (an Edit or History
//! window's upload, Save As, Save to Desktop, Open with, Copy text, an
//! upload's delete): counted, so a run that quits once nothing is left
//! (on demand, or a daemon whose tray icon was turned off) waits for it.

use std::sync::atomic::{AtomicUsize, Ordering};

static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// Run `f` on a new thread, counted until it returns (or panics).
pub fn spawn(f: impl FnOnce() + Send + 'static) {
    let guard = Guard::new();
    std::thread::spawn(move || {
        let _guard = guard;
        f();
    });
}

/// How many such threads are running now.
pub fn count() -> usize {
    RUNNING.load(Ordering::SeqCst)
}

/// One counted piece of work; uncounted when dropped.
struct Guard;

impl Guard {
    fn new() -> Guard {
        RUNNING.fetch_add(1, Ordering::SeqCst);
        Guard
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_until_done() {
        let (go, wait) = crate::chan::channel::<()>();
        let (done, finished) = crate::chan::channel::<()>();
        super::spawn(move || {
            let _ = wait.recv();
            let _ = done.send(());
        });
        // (Other tests may count work of their own meanwhile.)
        assert!(super::count() >= 1, "counted at once, while it runs");
        go.send(()).unwrap();
        finished.recv().unwrap();
        // A guard that drops on a panic too.
        let g = super::Guard::new();
        let n = super::count();
        drop(g);
        assert!(super::count() < n || n > 1);
    }
}
