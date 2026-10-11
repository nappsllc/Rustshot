//! The recording worker thread, shared by the platforms' recorders: opens
//! the pipeline's [`Parts`] for a spec, waits for the countdown, runs the
//! [`Session`] and finishes the file.

use super::End;
use crate::rec::{AudioSource, FrameSource, RecSpec, Session, VideoEncoder};
use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

pub(crate) enum Cmd {
    Go,
    Pause,
    Resume,
    Stop,
    Cancel,
}

/// A warning that may come up while recording (a sound source that
/// stopped): asked once the file is saved.
pub(crate) type Late = Box<dyn Fn() -> Option<String> + Send>;

/// What a recording is made of: the capture, the sound, the encoder (it
/// created `.part`), what could not be opened ("microphone not
/// recorded: <reason>") and what may go wrong later ([`Late`]).
pub(crate) struct Parts {
    pub(crate) src: Box<dyn FrameSource>,
    pub(crate) audio: Vec<Box<dyn AudioSource>>,
    pub(crate) enc: Box<dyn VideoEncoder>,
    pub(crate) warnings: Vec<String>,
    pub(crate) late: Vec<Late>,
}

/// Opens the [`Parts`] for a spec on the worker thread (the platform's
/// `prepare`; tests inject their own).
pub(crate) type Opener = Box<dyn FnOnce(&RecSpec) -> Result<Parts> + Send>;

pub(crate) fn work(spec: RecSpec, open: Opener, rx: Receiver<Cmd>, ready: &AtomicBool) -> End {
    #[cfg(windows)]
    crate::rec::win::com_init();
    let part = spec.part_path();
    let Parts { src, mut audio, enc, mut warnings, late } = match open(&spec) {
        Ok(p) => p,
        // Called off while opening (the portal's dialog closed, or Stop).
        Err(e) if e.is::<super::NotStarted>() => {
            let _ = std::fs::remove_file(&part);
            return End::Cancelled;
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return End::Failed(format!("{e:#}"));
        }
    };
    ready.store(true, Ordering::SeqCst);
    loop {
        match rx.recv() {
            Ok(Cmd::Go) => break,
            Ok(Cmd::Cancel | Cmd::Stop) | Err(_) => {
                drop(enc); // closes `.part`
                let _ = std::fs::remove_file(&part);
                return End::Cancelled;
            }
            Ok(Cmd::Pause | Cmd::Resume) => {}
        }
    }
    // Sound captured during the countdown is not part of the recording.
    let mut scratch = Vec::new();
    for a in &mut audio {
        while a.read(&mut scratch) > 0 {
            scratch.clear();
        }
    }
    let session = match Session::start(spec, src, audio, enc) {
        Ok(s) => s,
        Err(e) => {
            // The session dropped the encoder (closing `.part`): remove it.
            let _ = std::fs::remove_file(&part);
            return End::Failed(format!("{e:#}"));
        }
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Cmd::Pause) => session.pause(),
            Ok(Cmd::Resume) => session.resume(),
            Ok(Cmd::Go) => {}
            Ok(Cmd::Stop | Cmd::Cancel) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                // The encoder failed: stop now and keep what was written.
                if session.stats().failed {
                    break;
                }
            }
        }
    }
    match session.stop() {
        Ok(path) => {
            warnings.extend(late.iter().filter_map(|w| w()));
            End::Saved { path, warnings }
        }
        Err(e) => End::Failed(format!("{e:#}")),
    }
}

pub(crate) struct Worker {
    pub(crate) tx: Sender<Cmd>,
    pub(crate) rx: Receiver<End>,
    /// The parts opened (on Wayland: the screen was chosen in the portal's
    /// dialog); the countdown may begin.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Windows asks nothing first
    pub(crate) ready: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub(crate) fn spawn(spec: RecSpec, open: Opener) -> Result<Worker> {
        let (tx, crx) = mpsc::channel();
        let (etx, rx) = mpsc::channel();
        let ready = Arc::new(AtomicBool::new(false));
        let r2 = ready.clone();
        let join = std::thread::Builder::new()
            .name("rec-worker".into())
            .spawn(move || {
                let _ = etx.send(work(spec, open, crx, &r2));
            })
            .context("start the recording thread")?;
        Ok(Worker { tx, rx, ready, join: Some(join) })
    }
}

impl Drop for Worker {
    /// The process is going away mid-recording: finish the file first.
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Worker {
    /// Tell the worker what the state machine decided.
    pub(crate) fn send(&self, e: super::Effect) {
        let c = match e {
            super::Effect::Go => Cmd::Go,
            super::Effect::Pause => Cmd::Pause,
            super::Effect::Resume => Cmd::Resume,
            super::Effect::Stop => Cmd::Stop,
            super::Effect::Cancel => Cmd::Cancel,
        };
        let _ = self.tx.send(c);
    }
}
