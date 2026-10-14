//! `RUSTSHOT_PERFLOG=<path>`: per-event timings of the capture overlay,
//! one line per paint appended to `<path>`. Off (one relaxed load per
//! probe) when the variable is unset.
//!
//! A line: `t <ms>` (since the first line), `paint <ms>` (the whole
//! WM_PAINT), `lag <ms>` (from the oldest mouse move that invalidated
//! something not yet on screen to the end of this paint), `age <ms>` (from
//! the newest such move: how old the pointer position drawn is), `moves
//! <n>` (move messages since the last paint; the `move` probe counts the
//! ones handled), `dmg <n>/<px>` (rects and pixels
//! of the last damage), `upd <n>/<px>` (the update region painted), then
//! each probe's total ms and call count since the last paint (probes
//! nest: `move` includes `ptr`, `damage` and `inval`; `damage` includes
//! `bake` (re-baking edited objects); `bar` its `text`;
//! `snap` is a hover's window hit test), and for a paint of a pointer
//! move paced to the display `slack <ms>` (how long before the vblank it
//! was made for it was done; below 0: late, shown a refresh later). Only the Windows overlay (GDI
//! renderer) writes paint lines; a capture's window snapshot adds one
//! `snap <ms> list <ms> windows <n> parts <n>` line on every OS.
//!
//! A move's time is when it arrived, or, for a synthetic move whose
//! wParam carries a stamp in its high 32 bits (QueryPerformanceCounter in
//! microseconds, mod 2^32: what the live perf harness posts), when it was
//! sent, so the time it waited in the queue counts too.
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
    /// Re-baking the objects (in `damage`).
    Bake,
}

const N: usize = K::Bake as usize + 1;
pub const NAMES: [&str; N] = ["move", "damage", "compose", "bg", "sel", "label", "bar", "text", "blit", "put", "ptr", "inval", "snap", "bake"];

/// 0 = not looked up yet, 1 = off, 2 = on.
static STATE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static ACC: RefCell<[(f64, u32); N]> = const { RefCell::new([(0.0, 0); N]) };
    /// The moves being handled: the oldest one's time and the newest's.
    static IN_MOVE: Cell<Option<(Instant, Instant)>> = const { Cell::new(None) };
    /// The oldest move whose damage is not on screen yet.
    static FIRST_MOVE: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The newest move whose damage is not on screen yet.
    static LAST_MOVE: Cell<Option<Instant>> = const { Cell::new(None) };
    static MOVES: Cell<u32> = const { Cell::new(0) };
    static DMG: Cell<(u32, u64)> = const { Cell::new((0, 0)) };
    static UPD: Cell<(u32, u64)> = const { Cell::new((0, 0)) };
    /// The vblank this paint is for (`frame_for`).
    static TARGET: Cell<Option<f64>> = const { Cell::new(None) };
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

/// A move message's time (`None` while off): its stamp (see the module
/// docs) when `wparam` has one, else now.
#[inline]
pub fn move_time(wparam: usize) -> Option<Instant> {
    if !on() {
        return None;
    }
    let now = Instant::now();
    let stamp = (wparam as u64 >> 32) as u32;
    if stamp == 0 {
        return Some(now);
    }
    let waited = qpc_us().wrapping_sub(stamp);
    // A stamp from the future or minutes old is not one.
    Some(if waited < 60_000_000 { now - std::time::Duration::from_micros(waited as u64) } else { now })
}

/// QueryPerformanceCounter and its frequency.
#[cfg(windows)]
fn qpc() -> (i64, i64) {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let (mut c, mut f) = (0i64, 0i64);
    // SAFETY: both write an i64 we own.
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
        let _ = QueryPerformanceFrequency(&mut f);
    }
    (c, f)
}

#[cfg(not(windows))]
fn qpc() -> (i64, i64) {
    (0, 0)
}

/// QueryPerformanceCounter in microseconds, mod 2^32.
fn qpc_us() -> u32 {
    let (c, f) = qpc();
    if f <= 0 {
        return 0;
    }
    ((c as i128 * 1_000_000 / f as i128) as u64) as u32
}

/// QueryPerformanceCounter in milliseconds (the clock of DWM's timing).
pub fn qpc_ms() -> f64 {
    let (c, f) = qpc();
    if f <= 0 { 0.0 } else { c as f64 * 1000.0 / f as f64 }
}

/// The paint being made is for the vblank at `ms` ([`qpc_ms`]): its line
/// gets `slack <ms>`, how long before that vblank it was done.
#[inline]
pub fn frame_for(ms: f64) {
    if on() {
        TARGET.set(Some(ms));
    }
}

/// A move message came in (counted for the paint line).
#[inline]
pub fn move_arrived() {
    if on() {
        MOVES.set(MOVES.get() + 1);
    }
}

/// Moves are handled (the oldest came at `first`, the newest at `last`,
/// see [`move_time`]); hold the result while handling them.
#[inline]
pub fn mouse_move(times: Option<(Instant, Instant)>) -> MoveSpan {
    let s = span(K::Move);
    if s.0.is_some() {
        IN_MOVE.set(times);
    }
    MoveSpan(s)
}

/// The window was invalidated: during a move, the lag clock starts (if
/// it is not running) and the newest move drawn is this one.
#[inline]
pub fn invalidated() {
    if !on() {
        return;
    }
    if let Some((first, last)) = IN_MOVE.get() {
        if FIRST_MOVE.get().is_none() {
            FIRST_MOVE.set(Some(first));
        }
        LAST_MOVE.set(Some(last));
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
    let age = LAST_MOVE.take().map_or(0.0, ms);
    let moves = MOVES.replace(0);
    let (n, px) = DMG.replace((0, 0));
    let (un, upx) = UPD.replace((0, 0));
    let slack = TARGET.take().map(|v| v - qpc_ms());
    let acc = take();
    write_line(now, |t| {
        let mut line = format!("t {t:.1} paint {paint:.3} lag {lag:.3} age {age:.3} moves {moves} dmg {n}/{px} upd {un}/{upx}");
        if let Some(sl) = slack {
            line += &format!(" slack {sl:.3}");
        }
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
