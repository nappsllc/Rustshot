//! Trim before save (Settings › Recording "Trim after recording": Ask):
//! after Stop, a window with the recording's preview, a timeline of
//! thumbnails with start and end handles and a playhead, Play / Pause,
//! and "Save trimmed" / "Keep full length". Keys: Space plays or pauses,
//! I / O set the start / end at the playhead, ← / → move the playhead a
//! frame (Shift: a second), Enter saves trimmed, Esc keeps the full length.
//!
//! [`Trim`] is the window's state machine (pure, tested); [`layout`] and
//! [`paint`] draw it; the window itself (`window.rs`) feeds it events and
//! pictures from a decoding worker (`crate::rec::cut::decode`). The cut
//! is made after the window closes ([`crate::rec::cut::trim_file`]); the
//! trimmed file then takes the recording's name.
#![cfg_attr(target_os = "macos", allow(dead_code))]

#[cfg(not(target_os = "macos"))]
mod window;

use crate::objects::FRect;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The words (keys of `lang/en.txt`; "Selected {time}" is `trim.selected`).
pub mod text {
    use crate::i18n::Key;
    pub const TITLE: Key = key!("trim.title");
    pub const SAVE: Key = key!("trim.save");
    pub const KEEP: Key = key!("trim.keep");
    pub const NO_PREVIEW: Key = key!("trim.no_preview");
    pub const LOADING: Key = key!("trim.loading");
    pub const HINT: Key = key!("trim.hint");
    pub const FAILED: Key = key!("trim.failed");
}

/// The shortest selection (anything shorter is no recording).
pub const MIN_LEN: Duration = Duration::from_millis(200);

/// What a drag on the timeline moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    Start,
    End,
    Head,
}

/// How the window was closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Save `start..end` (in the file's time).
    Trimmed { start: Duration, end: Duration },
    /// Keep the file as it is (also: the window was closed).
    Full,
}

/// The trim window's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Trim {
    pub len: Duration,
    pub start: Duration,
    pub end: Duration,
    /// Where the preview is.
    pub head: Duration,
    pub grab: Option<Grab>,
    /// Playing since (wall clock, the head then).
    playing: Option<(Instant, Duration)>,
    /// One frame's length (the keys' small step).
    pub frame: Duration,
}

impl Trim {
    pub fn new(len: Duration, fps: u32) -> Trim {
        let frame = Duration::from_secs_f64(1.0 / fps.clamp(1, 120) as f64);
        Trim { len, start: Duration::ZERO, end: len, head: Duration::ZERO, grab: None, playing: None, frame }
    }

    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// The selection's length.
    pub fn selected(&self) -> Duration {
        self.end.saturating_sub(self.start)
    }

    /// Something is cut off (Save trimmed does something).
    pub fn trims(&self) -> bool {
        self.start > Duration::ZERO || self.end < self.len
    }

    fn clamp(&self, t: Duration) -> Duration {
        t.min(self.len)
    }

    /// Move the start to `t` (kept [`MIN_LEN`] before the end); the head
    /// follows it.
    pub fn set_start(&mut self, t: Duration) {
        self.start = self.clamp(t).min(self.end.saturating_sub(MIN_LEN));
        self.head = self.start;
        self.pause_at(self.head);
    }

    /// Move the end to `t` (kept [`MIN_LEN`] after the start); the head
    /// shows the last picture before it.
    pub fn set_end(&mut self, t: Duration) {
        self.end = self.clamp(t).max(self.start + MIN_LEN).min(self.len);
        self.head = self.end.saturating_sub(self.frame).max(self.start);
        self.pause_at(self.head);
    }

    /// Put the head at `t` (anywhere on the recording).
    pub fn seek(&mut self, t: Duration) {
        self.head = self.clamp(t);
        if self.playing.is_some() {
            self.playing = Some((Instant::now(), self.head));
        }
    }

    fn pause_at(&mut self, t: Duration) {
        self.playing = None;
        self.head = t;
    }

    /// A press on the timeline at `t`; `near` is the handle within reach
    /// of the pointer (the window measures that in pixels).
    pub fn press(&mut self, t: Duration, near: Option<Grab>) {
        let g = near.unwrap_or(Grab::Head);
        self.grab = Some(g);
        self.drag(t);
    }

    /// The pointer moved to `t` while pressed.
    pub fn drag(&mut self, t: Duration) {
        match self.grab {
            Some(Grab::Start) => self.set_start(t),
            Some(Grab::End) => self.set_end(t),
            Some(Grab::Head) => self.seek(t),
            None => {}
        }
    }

    pub fn release(&mut self) {
        self.grab = None;
    }

    /// Play from the head (from the start when it is at or past the end of
    /// the selection, or before its start), or pause.
    pub fn toggle_play(&mut self, now: Instant) {
        if self.playing.take().is_some() {
            return;
        }
        if self.head + self.frame >= self.end || self.head < self.start {
            self.head = self.start;
        }
        self.playing = Some((now, self.head));
    }

    /// Advance while playing: the selection plays once, then it pauses at
    /// its end. True while playing (the window keeps ticking fast).
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some((t0, h0)) = self.playing else { return false };
        let h = h0 + now.saturating_duration_since(t0);
        if h >= self.end {
            self.head = self.end.saturating_sub(self.frame).max(self.start);
            self.playing = None;
            return false;
        }
        self.head = h;
        true
    }

    /// A key: what it changed, and the choice when it closes the window.
    pub fn key(&mut self, vk: u32, shift: bool, now: Instant) -> Option<Choice> {
        use crate::wind::key;
        let step = if shift { Duration::from_secs(1) } else { self.frame };
        match vk {
            key::SPACE => self.toggle_play(now),
            0x49 => self.set_start(self.head), // I
            0x4F => self.set_end(self.head + self.frame), // O: the head's frame stays in
            key::LEFT => self.seek(self.head.saturating_sub(step)),
            key::RIGHT => self.seek(self.head + step),
            key::HOME => self.seek(self.start),
            key::END => self.seek(self.end.saturating_sub(self.frame)),
            key::RETURN => return Some(self.choice()),
            key::ESCAPE => return Some(Choice::Full),
            _ => {}
        }
        None
    }

    /// Save trimmed: the selection, or the full length when nothing is cut.
    pub fn choice(&self) -> Choice {
        if self.trims() { Choice::Trimmed { start: self.start, end: self.end } } else { Choice::Full }
    }
}

/// "0:03.2" (minutes, seconds, tenths); "1:02:03.4" past an hour.
pub fn fmt_time(d: Duration) -> String {
    let t = d.as_millis() / 100;
    let (tenth, s) = (t % 10, t / 10);
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{tenth}") } else { format!("{m}:{s:02}.{tenth}") }
}

// ---------------------------------------------------------------------------
// Layout (logical px)
// ---------------------------------------------------------------------------

/// The window's size (logical px).
pub const W: u32 = 760;
pub const H: u32 = 540;
pub const PAD: f32 = 20.0;
/// The timeline's height and its handles' width.
pub const STRIP_H: f32 = 56.0;
pub const HANDLE_W: f32 = 10.0;
/// How near (logical px) a press must be to take a handle.
pub const REACH: f32 = 12.0;
/// Thumbnails on the timeline.
pub const THUMBS: usize = 10;

/// Where everything is (logical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lay {
    pub preview: FRect,
    pub play: FRect,
    pub strip: FRect,
    pub footer: f32,
}

pub fn layout() -> Lay {
    let (w, h) = (W as f32, H as f32);
    let footer = h - 60.0;
    let strip = FRect { x: PAD, y: footer - 20.0 - STRIP_H, w: w - 2.0 * PAD, h: STRIP_H };
    let play = FRect { x: PAD, y: strip.y - 12.0 - 32.0, w: 32.0, h: 32.0 };
    let preview = FRect { x: PAD, y: 16.0, w: w - 2.0 * PAD, h: play.y - 12.0 - 16.0 };
    Lay { preview, play, strip, footer }
}

/// The time at logical x on the strip.
pub fn time_at(l: &Lay, len: Duration, x: f32) -> Duration {
    let f = ((x - l.strip.x) / l.strip.w).clamp(0.0, 1.0);
    Duration::from_secs_f64(len.as_secs_f64() * f as f64)
}

/// The logical x of time `t` on the strip.
pub fn x_of(l: &Lay, len: Duration, t: Duration) -> f32 {
    let f = if len.is_zero() { 0.0 } else { (t.as_secs_f64() / len.as_secs_f64()).clamp(0.0, 1.0) as f32 };
    l.strip.x + l.strip.w * f
}

/// The handle within [`REACH`] of logical x (the nearer when both are).
pub fn near_handle(l: &Lay, t: &Trim, x: f32) -> Option<Grab> {
    let (sx, ex) = (x_of(l, t.len, t.start), x_of(l, t.len, t.end));
    let (ds, de) = ((x - sx).abs(), (x - ex).abs());
    match (ds <= REACH, de <= REACH) {
        (true, true) => Some(if ds <= de { Grab::Start } else { Grab::End }),
        (true, false) => Some(Grab::Start),
        (false, true) => Some(Grab::End),
        _ => None,
    }
}

/// What came of the window: the recording's path now and its length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The trimmed recording (under the recording's name) and its length.
    Trimmed(PathBuf, Duration),
    /// Kept as it was.
    Full,
    /// The trim failed: the recording is kept as it was.
    Failed(String),
}

/// Carry out `c` on `path`: the trimmed file takes the recording's name.
pub fn apply(path: &Path, c: Choice) -> Outcome {
    let Choice::Trimmed { start, end } = c else { return Outcome::Full };
    match crate::rec::cut::trim_file(path, start, end) {
        // The trimmed file (synced, read back) replaces the recording in
        // one rename: the name stays, and the original is whole until then.
        Ok(tmp) => match crate::rec::cut::replace_with(path, &tmp) {
            Ok(()) => Outcome::Trimmed(path.to_path_buf(), end - start),
            Err(e) => {
                eprintln!("rustshot: {e:#}; the trimmed recording is {}", tmp.display());
                Outcome::Trimmed(tmp, end - start)
            }
        },
        Err(e) => Outcome::Failed(format!("{}: {e:#}", text::FAILED)),
    }
}

/// Open the trim window for `path` (on its own thread); the outcome comes
/// on the receiver once it closes and the cut is made. `None` where there
/// is no such window (macOS for now).
pub fn open(path: &Path, th: crate::theme::Theme) -> Option<crate::chan::Receiver<Outcome>> {
    #[cfg(not(target_os = "macos"))]
    {
        window::open(path, th)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (path, th);
        None
    }
}

#[cfg(test)]
mod tests;
