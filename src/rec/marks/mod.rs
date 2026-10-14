//! Show clicks and keystrokes in recordings (Settings › Recording "Show
//! clicks", "Show keystrokes", "Show all keys"): a ring pulses where the
//! pointer clicked (left and right in their own colors), and a pill at
//! the bottom centre shows the last key chords ("Ctrl + Shift + S"),
//! fading [`KEY_FADE_AFTER`] after the last one. Drawn into the recorded
//! frames only, never on screen ([`Marked`] wraps the frame source).
//!
//! Input comes from global hooks installed only while a recording runs
//! (from its first frame until the source is dropped): Windows
//! `WH_MOUSE_LL` / `WH_KEYBOARD_LL`, X11 XInput2 raw events, macOS a
//! listen-only CGEventTap (compile-only). Nothing is written anywhere:
//! events live in memory for the 1.8 s they are drawn and are dropped
//! after. By default only chords with Ctrl, Alt or Win (Super, Cmd) are
//! shown, so typed text never is; "Show all keys" shows every key, but on
//! Windows never while a password field has the focus (UI Automation's
//! IsPassword on the focused element).
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]

#[cfg(windows)]
mod win;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "macos")]
mod macos;

use super::{Frame, FrameSource};
use crate::capture::IRect;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The words (one place, for the translations).
pub mod text {
    use crate::i18n::Key;
    /// Settings › Recording: one row of three checkboxes.
    pub const SHOW: Key = key!("settings.recording.show");
    pub const SHOW_NOTE: Key = key!("settings.recording.show.help");
    pub const CLICKS: Key = key!("settings.recording.clicks");
    pub const KEYS: Key = key!("settings.recording.keystrokes");
    pub const ALL_KEYS: Key = key!("settings.recording.all_keys");
    pub const UNAVAILABLE: Key = key!("rec.marks.unavailable");
    /// Said as All keys is turned on, and once when a recording starts.
    pub const ALL_KEYS_WARNING: Key = key!("rec.marks.all_keys_warning");
    pub const NO_DETECTION_LINUX: Key = key!("rec.marks.no_detection_linux");
    /// Under the row while All keys is on.
    pub const ALL_KEYS_SHORT: Key = key!("settings.recording.all_keys.help");

    /// The warning for this platform.
    pub fn all_keys_warning() -> String {
        if cfg!(target_os = "linux") { format!("{ALL_KEYS_WARNING} {NO_DETECTION_LINUX}") } else { ALL_KEYS_WARNING.get().to_string() }
    }
}

/// True the first time it is asked in this run (the All keys warning).
#[cfg_attr(test, allow(dead_code))] // the app warns outside tests
pub fn warn_once() -> bool {
    static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    !SAID.swap(true, std::sync::atomic::Ordering::SeqCst)
}

/// How long a click's ring pulses.
pub const RING_LIFE: Duration = Duration::from_millis(450);
/// The key pill stays this long after the last chord, then fades.
pub const KEY_FADE_AFTER: Duration = Duration::from_millis(1500);
pub const KEY_FADE: Duration = Duration::from_millis(300);
/// Chords kept on the pill.
pub const MAX_CHORDS: usize = 3;
/// The rings' colors: left and right button (the drawing palette's
/// yellow and blue).
pub const LEFT: [u8; 3] = [0xFF, 0xC5, 0x32];
pub const RIGHT: [u8; 3] = [0x3B, 0x82, 0xF6];
/// Ring radius from / to, and its width (logical px).
const RING_R0: f32 = 6.0;
const RING_R1: f32 = 26.0;
const RING_W: f32 = 3.0;
/// The pill: text size, padding, height, distance from the bottom.
const PILL_TEXT: f32 = 16.0;
const PILL_PAD: f32 = 14.0;
const PILL_H: f32 = 36.0;
const PILL_BOTTOM: f32 = 40.0;

/// What a recording shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkOpts {
    pub clicks: bool,
    pub keys: bool,
    /// Every key, not only chords with Ctrl / Alt / Win.
    pub all_keys: bool,
}

impl MarkOpts {
    #[cfg_attr(test, allow(dead_code))] // the app arms recordings outside tests
    pub fn from_config(cfg: &crate::config::Config) -> MarkOpts {
        MarkOpts { clicks: cfg.rec_show_clicks, keys: cfg.rec_show_keys, all_keys: cfg.rec_show_all_keys }
    }

    pub fn any(&self) -> bool {
        self.clicks || self.keys
    }
}

/// The options of the next recording (set by the app as it starts one,
/// taken by the recording worker when its frame source opens).
static ARMED: Mutex<Option<MarkOpts>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The next recording shows `o`.
pub fn arm(o: MarkOpts) {
    *lock(&ARMED) = o.any().then_some(o);
}

fn take_armed() -> Option<MarkOpts> {
    lock(&ARMED).take()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// The modifiers that make a chord, read when the key went down. `ctrl`
/// and `alt` are real ones: the Ctrl + Alt that AltGr reports (Windows)
/// is neither, so typed @, €, ą never reads as a shortcut.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

/// `vk` is a modifier (Win32 numbering, either side; AltGr is RMenu).
pub fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

/// A key's name on the pill (Win32 numbering): the shortcut table's
/// names, plus the punctuation keys (US layout) and a few more.
pub fn key_label(vk: u32) -> Option<String> {
    if let Some(n) = crate::hotkey::key_name(vk) {
        return Some(match n.as_str() {
            "Up" => "\u{2191}".into(),
            "Down" => "\u{2193}".into(),
            "Left" => "\u{2190}".into(),
            "Right" => "\u{2192}".into(),
            _ => n,
        });
    }
    let s = match vk {
        0xBA => ";",
        0xBC => ",",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDC => "\\",
        0xDE => "'",
        0x14 => "CapsLock",
        0x6A => "Numpad*",
        0x6F => "Numpad/",
        0x6E => "Numpad.",
        0x7C..=0x87 => return Some(format!("F{}", vk - 0x70 + 1)),
        _ => return None,
    };
    Some(s.into())
}

/// "Ctrl + Shift + S" for `vk` with `held`; `None` when it is not shown:
/// a modifier alone, an unnamed key, or (without `all`) a key without
/// Ctrl, Alt or Win.
pub fn chord_text(held: Held, vk: u32, all: bool) -> Option<String> {
    if is_modifier(vk) {
        return None;
    }
    if !(all || held.ctrl || held.alt || held.win) {
        return None;
    }
    let key = key_label(vk)?;
    let mut parts: Vec<&str> = Vec::new();
    if held.ctrl {
        parts.push("Ctrl");
    }
    if held.alt {
        parts.push("Alt");
    }
    if held.shift {
        parts.push("Shift");
    }
    if held.win {
        parts.push(crate::keymap::META_NAME);
    }
    let mut s = parts.join(" + ");
    if !s.is_empty() {
        s.push_str(" + ");
    }
    s.push_str(&key);
    Some(s)
}

/// A click being shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    /// Screen px.
    pub x: i32,
    pub y: i32,
    pub button: Button,
    pub at: Instant,
}

#[derive(Default)]
struct State {
    clicks: VecDeque<Click>,
    /// The chords on the pill (with a repeat count), and when the last came.
    chords: Vec<(String, u32)>,
    last_chord: Option<Instant>,
    /// Paused, or the secure desktop is up: nothing is taken or shown.
    suspended: bool,
}

/// The events of a recording, shared between the hooks and the source.
pub struct Marks {
    pub opts: MarkOpts,
    st: Mutex<State>,
}

/// What to draw now.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// Clicks with their age (0 → 1 over [`RING_LIFE`]).
    pub rings: Vec<(Click, f32)>,
    /// The pill's text and its opacity.
    pub pill: Option<(String, f32)>,
}

impl Scene {
    pub fn is_empty(&self) -> bool {
        self.rings.is_empty() && self.pill.is_none()
    }
}

/// A key event older than this when it is filed is dropped, not drawn
/// late (its moment, and the focus it was typed into, are gone).
#[cfg_attr(not(windows), allow(dead_code))]
pub const STALE: Duration = Duration::from_millis(400);

impl Marks {
    pub fn new(opts: MarkOpts) -> Marks {
        Marks { opts, st: Mutex::new(State::default()) }
    }

    /// Forget everything shown; while `on`, take nothing and show nothing
    /// (a pause, the secure desktop). Both edges clear.
    pub fn suspend(&self, on: bool) {
        let mut st = lock(&self.st);
        st.clicks.clear();
        st.chords.clear();
        st.last_chord = None;
        st.suspended = on;
    }

    /// A button went down at (x, y) (screen px) at `at`.
    pub fn click(&self, x: i32, y: i32, button: Button, at: Instant) {
        if !self.opts.clicks {
            return;
        }
        let mut st = lock(&self.st);
        if st.suspended {
            return;
        }
        st.clicks.retain(|c| at.saturating_duration_since(c.at) < RING_LIFE);
        if st.clicks.len() < 16 {
            st.clicks.push_back(Click { x, y, button, at });
        }
    }

    /// A key went down (Win32 numbering) at `at`, with the modifiers
    /// `held` read at that moment (never derived from earlier events: a
    /// key-up lost to the lock screen must not leave one stuck). `secret`:
    /// a password field has (or may have) the focus: never shown.
    pub fn key(&self, vk: u32, held: Held, secret: bool, at: Instant) {
        if !self.opts.keys || secret {
            return;
        }
        let Some(text) = chord_text(held, vk, self.opts.all_keys) else { return };
        let mut st = lock(&self.st);
        if st.suspended {
            return;
        }
        let fresh = st.last_chord.is_none_or(|t| at.saturating_duration_since(t) >= KEY_FADE_AFTER);
        if fresh {
            st.chords.clear();
        }
        match st.chords.last_mut() {
            Some((t, n)) if *t == text => *n += 1,
            _ => {
                st.chords.push((text, 1));
                if st.chords.len() > MAX_CHORDS {
                    st.chords.remove(0);
                }
            }
        }
        st.last_chord = Some(at);
    }

    /// What shows at `now`.
    pub fn scene(&self, now: Instant) -> Scene {
        let mut st = lock(&self.st);
        st.clicks.retain(|c| now.saturating_duration_since(c.at) < RING_LIFE);
        let rings = st.clicks.iter().map(|c| (*c, now.saturating_duration_since(c.at).as_secs_f32() / RING_LIFE.as_secs_f32())).collect();
        let pill = st.last_chord.and_then(|t| {
            let age = now.saturating_duration_since(t);
            let alpha = if age <= KEY_FADE_AFTER {
                1.0
            } else {
                1.0 - (age - KEY_FADE_AFTER).as_secs_f32() / KEY_FADE.as_secs_f32()
            };
            (alpha > 0.0 && !st.chords.is_empty()).then(|| (pill_text(&st.chords), alpha))
        });
        if pill.is_none() && st.last_chord.is_some_and(|t| now.saturating_duration_since(t) > KEY_FADE_AFTER + KEY_FADE) {
            // Gone from the pill: gone from memory too.
            st.chords.clear();
            st.last_chord = None;
        }
        Scene { rings, pill }
    }
}

/// "Ctrl + C   Ctrl + V ×2".
pub fn pill_text(chords: &[(String, u32)]) -> String {
    chords.iter().map(|(t, n)| if *n > 1 { format!("{t} \u{d7}{n}") } else { t.clone() }).collect::<Vec<_>>().join("   ")
}

// ---------------------------------------------------------------------------
// Drawing into a frame
// ---------------------------------------------------------------------------

/// A color to blend: RGB (unpremultiplied) and the same in BT.709
/// limited-range YUV (worked out once, not per pixel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Paint {
    rgb: [u8; 3],
    yuv: [u8; 3],
}

impl Paint {
    fn of(rgb: [u8; 3]) -> Paint {
        Paint { rgb, yuv: yuv(rgb[0], rgb[1], rgb[2]) }
    }
}

/// A picture to draw into: BGRA, or NV12 (BT.709, limited range).
trait Target {
    fn size(&self) -> (u32, u32);
    /// Blend `c` at opacity `a` over pixel (x, y) (inside).
    fn blend(&mut self, x: u32, y: u32, c: Paint, a: u8);
}

struct Bgra<'a> {
    px: &'a mut [u8],
    w: u32,
    h: u32,
}

#[inline]
fn mix(d: u8, s: u8, a: u32) -> u8 {
    ((d as u32 * (255 - a) + s as u32 * a + 127) / 255) as u8
}

impl Target for Bgra<'_> {
    fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }
    #[inline]
    fn blend(&mut self, x: u32, y: u32, c: Paint, a: u8) {
        let i = (y as usize * self.w as usize + x as usize) * 4;
        let a = a as u32;
        let [r, g, b] = c.rgb;
        let d = &mut self.px[i..i + 3];
        d[0] = mix(d[0], b, a);
        d[1] = mix(d[1], g, a);
        d[2] = mix(d[2], r, a);
    }
}

struct Nv12<'a> {
    px: &'a mut [u8],
    w: u32,
    h: u32,
}

/// BT.709 limited range of an RGB color: [Y, U, V].
fn yuv(r: u8, g: u8, b: u8) -> [u8; 3] {
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let y = 16.0 + 0.1826 * r + 0.6142 * g + 0.0620 * b;
    let u = 128.0 - 0.1006 * r - 0.3386 * g + 0.4392 * b;
    let v = 128.0 + 0.4392 * r - 0.3989 * g - 0.0403 * b;
    [y.round() as u8, u.round() as u8, v.round() as u8]
}

impl Target for Nv12<'_> {
    fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }
    #[inline]
    fn blend(&mut self, x: u32, y: u32, c: Paint, a: u8) {
        let a = a as u32;
        let w = self.w as usize;
        let yi = y as usize * w + x as usize;
        self.px[yi] = mix(self.px[yi], c.yuv[0], a);
        // Its 2x2 block's chroma takes a quarter of the blend.
        let ci = w * self.h as usize + (y as usize / 2) * w + (x as usize / 2) * 2;
        let q = a / 4;
        self.px[ci] = mix(self.px[ci], c.yuv[1], q);
        self.px[ci + 1] = mix(self.px[ci + 1], c.yuv[2], q);
    }
}

/// A sprite: each pixel's color and opacity (unpremultiplied).
#[derive(Clone, Debug)]
struct Patch {
    w: u32,
    h: u32,
    px: Vec<(Paint, u8)>,
}

impl Patch {
    /// From premultiplied BGRA (what `render_layer` draws).
    fn of_layer(w: u32, h: u32, bgra: &[u8]) -> Patch {
        let px = bgra
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let a = p[3] as u32;
                let un = |c: u8| (c as u32 * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
                (Paint::of([un(p[2]), un(p[1]), un(p[0])]), p[3])
            })
            .collect();
        Patch { w, h, px }
    }
}

/// Draws scenes; keeps the pill's picture while its text is the same.
pub struct Painter {
    /// Logical px → frame px.
    scale: f32,
    pill: Option<(String, u32, Patch)>,
    /// NV12 pictures drawn into before, reused once the encoder let go of
    /// them (no allocation, so no page faults, a frame).
    pool: Vec<Arc<Vec<u8>>>,
}

/// Pictures kept for reuse.
const POOL: usize = 3;

impl Painter {
    pub fn new(scale: f32) -> Painter {
        Painter { scale: scale.max(0.5), pill: None, pool: Vec::new() }
    }

    /// A picture of ours holding a copy of `src`.
    fn copy_of(&mut self, src: &[u8]) -> Arc<Vec<u8>> {
        let mut buf = match self.pool.iter().position(|b| Arc::strong_count(b) == 1) {
            Some(i) => self.pool.remove(i),
            None => Arc::new(Vec::with_capacity(src.len())),
        };
        if let Some(v) = Arc::get_mut(&mut buf) {
            v.clear();
            v.extend_from_slice(src);
        }
        buf
    }

    /// Draw `scene` into `f`, whose picture shows `area` (screen px).
    pub fn draw(&mut self, f: &mut Frame, area: IRect, scene: &Scene) {
        if scene.is_empty() {
            return;
        }
        if let Some(nv) = f.nv12.as_mut() {
            let (w, h) = (nv.w, nv.h);
            let k = w as f32 / f.w.max(1) as f32;
            // Shared with the source (it hands the same picture out when
            // nothing changed): drawn into a copy of ours.
            let mut buf = std::mem::take(&mut nv.data);
            let shared = Arc::get_mut(&mut buf).is_none();
            if shared {
                let src = buf;
                buf = self.copy_of(&src);
            }
            if let Some(data) = Arc::get_mut(&mut buf)
                && data.len() >= super::yuv::size(w, h)
            {
                self.paint(&mut Nv12 { px: data, w, h }, area, k, scene);
            }
            nv.data = if shared {
                let out = buf.clone();
                self.pool.push(buf);
                if self.pool.len() > POOL {
                    self.pool.remove(0);
                }
                out
            } else {
                buf
            };
        } else if f.bgra.len() >= f.w as usize * f.h as usize * 4 {
            let (w, h) = (f.w, f.h);
            self.paint(&mut Bgra { px: &mut f.bgra, w, h }, area, 1.0, scene);
        }
    }

    /// `k`: target px per area px (an NV12 picture scaled to fit).
    fn paint<T: Target>(&mut self, t: &mut T, area: IRect, k: f32, scene: &Scene) {
        let s = self.scale * k;
        for (c, age) in &scene.rings {
            let (x, y) = ((c.x - area.0) as f32 * k, (c.y - area.1) as f32 * k);
            ring(t, x, y, *age, c.button, s);
        }
        if let Some((text, alpha)) = &scene.pill {
            let key = (s * 100.0).round() as u32;
            if self.pill.as_ref().is_none_or(|(t0, k0, _)| t0 != text || *k0 != key) {
                self.pill = Some((text.clone(), key, pill_patch(text, s)));
            }
            if let Some((_, _, p)) = &self.pill {
                let (tw, th) = t.size();
                let x = (tw as i32 - p.w as i32) / 2;
                let y = th as i32 - p.h as i32 - (PILL_BOTTOM * s).round() as i32;
                blit(t, p, x, y, *alpha);
            }
        }
    }
}

/// One click's ring at (x, y) (target px), `age` 0 → 1.
fn ring<T: Target>(t: &mut T, x: f32, y: f32, age: f32, b: Button, s: f32) {
    let age = age.clamp(0.0, 1.0);
    // Ease out: fast first, then slower.
    let e = 1.0 - (1.0 - age) * (1.0 - age);
    let r = (RING_R0 + (RING_R1 - RING_R0) * e) * s;
    let hw = RING_W * s / 2.0;
    let fade = 1.0 - age;
    let color = Paint::of(if b == Button::Left { LEFT } else { RIGHT });
    let (tw, th) = t.size();
    let reach = r + hw + 1.0;
    let (x0, x1) = ((x - reach).floor().max(0.0) as u32, ((x + reach).ceil().max(0.0) as u32).min(tw));
    let (y0, y1) = ((y - reach).floor().max(0.0) as u32, ((y + reach).ceil().max(0.0) as u32).min(th));
    for py in y0..y1 {
        let dy = py as f32 + 0.5 - y;
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - x;
            let d = (dx * dx + dy * dy).sqrt();
            // The ring, and a faint disc inside it.
            let ring = (hw + 0.5 - (d - r).abs()).clamp(0.0, 1.0);
            let disc = if d < r { 0.22 } else { 0.0 };
            let a = (ring.max(disc) * fade * 255.0).round() as u32;
            if a > 0 {
                t.blend(px, py, color, a.min(255) as u8);
            }
        }
    }
}

/// The key pill as a sprite at scale `s`: a dark rounded panel, white text.
fn pill_patch(text: &str, s: f32) -> Patch {
    let font = crate::fonts::ui_font();
    let px = PILL_TEXT * s;
    let tw = crate::uifb::text_width(font, px, text);
    let (w, h) = ((tw + 2.0 * PILL_PAD * s).ceil() as u32, (PILL_H * s).ceil() as u32);
    let bgra = crate::rec_ui::render_layer(w as usize, h as usize, |f| {
        f.fill_rounded(0, 0, w as i32, h as i32, h as f32 / 2.0, crate::uifb::C4::new(18, 18, 22, 222));
        let ty = (h as f32 - crate::uifb::text_height(font, px)) / 2.0;
        f.draw_text(font, px, text, PILL_PAD * s, ty, crate::uifb::C4::rgb(255, 255, 255));
    });
    Patch::of_layer(w, h, &bgra)
}

/// Blend `p` at (x, y) (may hang off the edges) with opacity `alpha`.
fn blit<T: Target>(t: &mut T, p: &Patch, x: i32, y: i32, alpha: f32) {
    let k = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
    let (tw, th) = t.size();
    let (cx0, cx1) = ((-x).max(0), (tw as i32 - x).min(p.w as i32));
    for py in 0..p.h as i32 {
        let ty = y + py;
        if ty < 0 || ty >= th as i32 {
            continue;
        }
        let row = &p.px[py as usize * p.w as usize..(py as usize + 1) * p.w as usize];
        for px in cx0..cx1 {
            let (c, a) = row[px as usize];
            let a = (a as u32 * k) >> 8;
            if a > 0 {
                t.blend((x + px) as u32, ty as u32, c, a as u8);
            }
        }
    }
}


// ---------------------------------------------------------------------------
// The frame source
// ---------------------------------------------------------------------------

/// The platform's hooks feeding a [`Marks`]; removed when dropped.
pub trait Hooks: Send {}

/// Start the hooks for `marks` (they run until the guard is dropped).
fn start_hooks(marks: Arc<Marks>) -> anyhow::Result<Box<dyn Hooks>> {
    #[cfg(windows)]
    {
        win::start(marks)
    }
    #[cfg(target_os = "linux")]
    {
        x11::start(marks)
    }
    #[cfg(target_os = "macos")]
    {
        macos::start(marks)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = marks;
        anyhow::bail!("not available on this platform")
    }
}

/// A frame source with clicks and keys drawn into its frames.
pub struct Marked {
    inner: Box<dyn FrameSource>,
    marks: Arc<Marks>,
    area: IRect,
    painter: Painter,
    /// Installed with the first frame, removed with the source.
    hooks: Option<Box<dyn Hooks>>,
    tried: bool,
    /// Why the hooks did not start.
    failed: Arc<Mutex<Option<String>>>,
    paused: bool,
    /// The secure desktop is up, and when that was last checked.
    secure: bool,
    checked: Option<Instant>,
}

impl Marked {
    pub fn new(inner: Box<dyn FrameSource>, marks: Arc<Marks>, area: IRect, scale: f32) -> Marked {
        Marked {
            inner,
            marks,
            area,
            painter: Painter::new(scale),
            hooks: None,
            tried: false,
            failed: Arc::default(),
            paused: false,
            secure: false,
            checked: None,
        }
    }
}

impl FrameSource for Marked {
    fn next(&mut self, deadline: Instant) -> Option<Frame> {
        if !self.tried {
            self.tried = true;
            match start_hooks(self.marks.clone()) {
                Ok(h) => self.hooks = Some(h),
                Err(e) => {
                    eprintln!("rustshot: {}: {e:#}", text::UNAVAILABLE);
                    *lock(&self.failed) = Some(format!("{}: {e:#}", text::UNAVAILABLE));
                }
            }
        }
        // The lock screen, UAC, Ctrl+Alt+Del: nothing is shown or taken
        // while they are up, and what came before is forgotten both ways.
        let now = Instant::now();
        if self.hooks.is_some() && self.checked.is_none_or(|t| now.saturating_duration_since(t) >= SECURE_POLL) {
            self.checked = Some(now);
            let secure = secure_desktop();
            if secure != self.secure {
                self.secure = secure;
                self.sync_suspend();
            }
        }
        let mut f = self.inner.next(deadline)?;
        let scene = self.marks.scene(Instant::now());
        self.painter.draw(&mut f, self.area, &scene);
        Some(f)
    }

    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    fn ended(&self) -> Option<String> {
        self.inner.ended()
    }

    fn paused(&mut self, on: bool) {
        self.paused = on;
        self.sync_suspend();
        self.inner.paused(on);
    }
}

impl Marked {
    fn sync_suspend(&self) {
        self.marks.suspend(self.paused || self.secure);
    }
}

/// How often the input desktop is checked.
const SECURE_POLL: Duration = Duration::from_millis(200);

/// The input desktop is not the user's (Windows: Winlogon, a UAC prompt).
fn secure_desktop() -> bool {
    #[cfg(windows)]
    {
        win::secure_desktop()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The UI scale at the centre of `area` (the marks' logical px).
fn scale_at(area: IRect) -> f32 {
    #[cfg(test)]
    {
        let _ = area;
        1.0
    }
    #[cfg(not(test))]
    {
        let (cx, cy) = (area.0 + area.2 as i32 / 2, area.1 + area.3 as i32 / 2);
        crate::capture::monitors().unwrap_or_default().iter().find(|m| m.contains(cx, cy)).map_or(1.0, |m| m.scale)
    }
}

/// A warning asked for once the file is saved.
pub type Late = Box<dyn Fn() -> Option<String> + Send>;

/// `src` with the armed recording's clicks and keys (as is when none
/// are armed); and a warning to say once the file is saved when the hooks
/// could not start.
pub fn wrap(src: Box<dyn FrameSource>, area: IRect) -> (Box<dyn FrameSource>, Option<Late>) {
    let Some(opts) = take_armed() else { return (src, None) };
    let m = Marked::new(src, Arc::new(Marks::new(opts)), area, scale_at(area));
    let failed = m.failed.clone();
    let late: Late = Box::new(move || lock(&failed).clone());
    (Box::new(m), Some(late))
}

#[cfg(test)]
mod tests;
