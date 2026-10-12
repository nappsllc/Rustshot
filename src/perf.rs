//! `RUSTSHOT_PERFLOG=<path>`: per-event timings of the capture overlay,
//! one line per paint appended to `<path>`. Off (one relaxed load per
//! probe) when the variable is unset.
//!
//! A line: `t <ms>` (since the first line), `paint <ms>` (the whole
//! WM_PAINT), `lag <ms>` (from the first mouse move that invalidated
//! something not yet on screen to the end of this paint), `moves <n>`
//! (moves handled since the last paint), `dmg <n>/<px>` (rects and pixels
//! of the last damage), `upd <n>/<px>` (the update region painted), then
//! each probe's total ms and call count since the last paint (probes
//! nest: `move` includes `ptr`, `damage` and `inval`; `bar` its `text`;
//! `snap` is a hover's window hit test). Only the Windows overlay (GDI
//! renderer) writes paint lines; a capture's window snapshot adds one
//! `snap <ms> list <ms> windows <n> parts <n>` line on every OS.
#![cfg_attr(not(windows), allow(dead_code))]

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

/// What a probe measures.
#[derive(Clone, Copy, Debug)]
pub enum K {
    /// One mouse move: the driver's handling plus its damage.
    Move,
    /// `damage()`: scene prepare plus dirty rects.
    Damage,
    /// `compose_rect` calls (chrome and slow backdrop parts).
    Compose,
    /// Backdrop fill of composed rects.
    Bg,
    /// Selection outline and handles.
    Sel,
    /// The size label.
    Label,
    /// The toolbar (and its popover).
    Bar,
    /// UI text drawn in the chrome.
    Text,
    /// BitBlt / AlphaBlend from the capture bitmaps.
    Blit,
    /// SetDIBitsToDevice of composed buffers.
    Put,
    /// The editor's drag step (`pointer_move`).
    Ptr,
    /// Invalidating the damage.
    Inval,
    /// Window snapping: the hit test of a hover.
    Snap,
}

const N: usize = K::Snap as usize + 1;
pub const NAMES: [&str; N] = ["move", "damage", "compose", "bg", "sel", "label", "bar", "text", "blit", "put", "ptr", "inval", "snap"];

/// 0 = not looked up yet, 1 = off, 2 = on.
static STATE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static ACC: RefCell<[(f64, u32); N]> = const { RefCell::new([(0.0, 0); N]) };
    /// Start of the mouse move being handled.
    static IN_MOVE: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Start of the first move whose damage is not on screen yet.
    static FIRST_MOVE: Cell<Option<Instant>> = const { Cell::new(None) };
    static MOVES: Cell<u32> = const { Cell::new(0) };
    static DMG: Cell<(u32, u64)> = const { Cell::new((0, 0)) };
    static UPD: Cell<(u32, u64)> = const { Cell::new((0, 0)) };
    static OUT: RefCell<Option<(std::fs::File, Instant)>> = const { RefCell::new(None) };
}

/// Whether probes record.
#[inline]
pub fn on() -> bool {
    match STATE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var_os("RUSTSHOT_PERFLOG").is_some_and(|p| !p.is_empty());
            STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Record without a log file (benches read [`take`]).
#[cfg(all(test, windows))]
pub fn force_on() {
    STATE.store(2, Ordering::Relaxed);
}

/// A running probe; adds its time to `K` when dropped.
pub struct Span(Option<(K, Instant)>);

#[inline]
pub fn span(k: K) -> Span {
    Span(on().then(|| (k, Instant::now())))
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((k, t0)) = self.0 {
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            ACC.with_borrow_mut(|a| {
                a[k as usize].0 += ms;
                a[k as usize].1 += 1;
            });
        }
    }
}

/// The handling of one mouse move, until dropped.
pub struct MoveSpan(#[allow(dead_code)] Span);

impl Drop for MoveSpan {
    fn drop(&mut self) {
        IN_MOVE.set(None);
    }
}

/// A mouse move arrived; hold the result while handling it.
#[inline]
pub fn mouse_move() -> MoveSpan {
    let s = span(K::Move);
    if let Some((_, t0)) = s.0 {
        IN_MOVE.set(Some(t0));
        MOVES.set(MOVES.get() + 1);
    }
    MoveSpan(s)
}

/// The window was invalidated: during a move, the lag clock starts (if
/// it is not running).
#[inline]
pub fn invalidated() {
    if on() && FIRST_MOVE.get().is_none() {
        FIRST_MOVE.set(IN_MOVE.get());
    }
}

/// The damage just computed: `n` rects covering `px` pixels.
#[inline]
pub fn damage(n: usize, px: u64) {
    if on() {
        DMG.set((n as u32, px));
    }
}

/// The paint's update region: `n` rects covering `px` pixels.
#[inline]
pub fn update(n: usize, px: u64) {
    if on() {
        UPD.set((n as u32, px));
    }
}

/// The probe totals (ms, calls) since the last call, reset.
pub fn take() -> [(f64, u32); N] {
    ACC.with_borrow_mut(std::mem::take)
}

/// A paint that started at `t0` just finished: append its line, reset.
pub fn painted(t0: Instant) {
    if !on() {
        return;
    }
    let now = Instant::now();
    let ms = |a: Instant| (now - a).as_secs_f64() * 1000.0;
    let paint = ms(t0);
    let lag = FIRST_MOVE.take().map_or(0.0, ms);
    let moves = MOVES.replace(0);
    let (n, px) = DMG.replace((0, 0));
    let (un, upx) = UPD.replace((0, 0));
    let acc = take();
    write_line(now, |t| {
        let mut line = format!("t {t:.1} paint {paint:.3} lag {lag:.3} moves {moves} dmg {n}/{px} upd {un}/{upx}");
        for (name, (ms, c)) in NAMES.iter().zip(acc) {
            if c > 0 {
                line += &format!(" {name} {ms:.3}/{c}");
            }
        }
        line
    });
}

/// A one-off line (`t <ms> <text>`), e.g. the window snapshot's time.
pub fn note(text: &str) {
    if on() {
        write_line(Instant::now(), |t| format!("t {t:.1} {text}"));
    }
}

/// Append the line `f` makes (given the log's clock, ms) to the log.
fn write_line(now: Instant, f: impl FnOnce(f64) -> String) {
    OUT.with_borrow_mut(|out| {
        if out.is_none() {
            let Some(path) = std::env::var_os("RUSTSHOT_PERFLOG") else { return };
            let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
            *out = file.map(|file| (file, now));
        }
        let Some((file, start)) = out.as_mut() else { return };
        let t = now.saturating_duration_since(*start).as_secs_f64() * 1000.0;
        let mut line = f(t);
        line.push('\n');
        let _ = file.write_all(line.as_bytes());
    });
}
