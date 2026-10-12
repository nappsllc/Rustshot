//! macOS: the notice in a transparent, non-activating panel
//! ([`crate::wind::Layer`]) above other windows and out of screen capture
//! (`NSWindowSharingNone`), bottom-centre of the screen's visible frame
//! (clear of the Dock and the menu bar). A plain note lets clicks
//! through; one with buttons takes them. AppKit windows live on the main
//! thread, where the daemon's driver calls [`show`] and, every tick,
//! [`tick`] (which fades the note and carries out its clicks).
//!
//! Geometry: callers pass the shared code's pixels (points × the backing
//! factor, see `capture::pts`); the panel is placed in points and drawn at
//! the screen's density, the UI size on top.
#![cfg_attr(test, allow(dead_code))] // tests record notes instead (`notice::take_shown`)

use super::{act_at, act_layout, opacity, place_note, render_note, work_area, ActLayout, Next, Note, NoteAct, FADE_IN};
use crate::capture::{CGRect, IRect};
use crate::objects::Pt;
use crate::theme::Theme;
use crate::wind::{msg0, objc_cls, objc_sel, AuxKind, Layer};
use core::ffi::c_void;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Instant;

/// Clicks on the panel (window px), for the next tick.
#[derive(Default)]
struct Input {
    /// Pixels per point it is drawn at.
    scale: f64,
    pressed: Option<Pt>,
    clicks: Vec<Pt>,
}

struct Up {
    note: Note,
    th: Theme,
    /// Pixels per point, and the UI scale (pixels per logical px).
    px: f32,
    s: f32,
    work: IRect,
    since: Instant,
    k: Option<f32>,
    layout: Option<ActLayout>,
}

struct Win {
    layer: Layer,
    /// The layer lets clicks through (a plain note).
    through: bool,
    input: Rc<RefCell<Input>>,
}

thread_local! {
    static WIN: RefCell<Option<Win>> = const { RefCell::new(None) };
    static UP: RefCell<Option<Up>> = const { RefCell::new(None) };
}

/// A note an action's thread finished with (an upload), for the main thread.
static LATER: Mutex<Option<Note>> = Mutex::new(None);

/// A layer that lets clicks through or takes them (made again when that
/// changes: AppKit's flag is set once, at creation, in `Layer::new`).
fn layer_for(through: bool) -> Option<()> {
    WIN.with(|w| {
        let mut w = w.borrow_mut();
        if w.as_ref().is_some_and(|w| w.through == through) {
            return Some(());
        }
        *w = None;
        let layer = Layer::new(through, true).map_err(|e| eprintln!("rustshot: no notice window: {e:#}")).ok()?;
        let input = Rc::new(RefCell::new(Input { scale: 1.0, ..Default::default() }));
        let inp = input.clone();
        layer.on_pointer(move |ev| {
            let mut b = inp.borrow_mut();
            let p = Pt::new((ev.local.0 * b.scale) as f32, (ev.local.1 * b.scale) as f32);
            match ev.kind {
                AuxKind::Down => b.pressed = Some(p),
                AuxKind::Drag => {}
                AuxKind::Up => {
                    if b.pressed.take().is_some() {
                        b.clicks.push(p);
                    }
                }
            }
        });
        *w = Some(Win { layer, through, input });
        Some(())
    })
}

/// Screens' frames and visible frames (Quartz points, y down).
fn screens() -> Vec<(IRect, IRect)> {
    let flip = crate::capture::main_display_height();
    let q = |r: CGRect| -> IRect {
        let y = flip - (r.origin.y + r.size.height);
        (r.origin.x.round() as i32, y.round() as i32, r.size.width.max(0.0).round() as u32, r.size.height.max(0.0).round() as u32)
    };
    // SAFETY: AppKit on the main thread; NSScreen's array is autoreleased.
    unsafe {
        let arr: *mut c_void = msg0(objc_cls(c"NSScreen"), objc_sel(c"screens"));
        crate::macos_objc::items(arr)
            .into_iter()
            .map(|s| {
                let f: CGRect = crate::macos_objc::msg_big0(s, objc_sel(c"frame"));
                let v: CGRect = crate::macos_objc::msg_big0(s, objc_sel(c"visibleFrame"));
                (q(f), q(v))
            })
            .collect()
    }
}

/// The visible frame (points) of the screen at `at` (points).
fn work_at(at: (i32, i32)) -> IRect {
    let sc = screens();
    let mons: Vec<IRect> = sc.iter().map(|s| s.0).collect();
    let mon = work_area(&mons, None, at);
    let vis = sc.iter().find(|s| s.0 == mon).map(|s| s.1);
    work_area(&[mon], vis, at)
}

pub fn show(note: Note, at: (i32, i32), th: Theme) -> bool {
    if !crate::macos_objc::is_main_thread() {
        return false;
    }
    // The shared code's pixels to points.
    let k = crate::capture::backing().max(1.0);
    let at = ((f64::from(at.0) / k).round() as i32, (f64::from(at.1) / k).round() as i32);
    let work = work_at(at);
    let px = crate::rec::macos::backing_scale_at(f64::from(work.0 + work.2 as i32 / 2), f64::from(work.1 + work.3 as i32 / 2)) as f32;
    let px = px.clamp(1.0, 4.0);
    put_up(note, th, px, work, Instant::now())
}

fn put_up(note: Note, th: Theme, px: f32, work: IRect, since: Instant) -> bool {
    if layer_for(note.actions.is_empty()).is_none() {
        return false;
    }
    let s = crate::uisize::apply(px);
    let layout = (!note.actions.is_empty()).then(|| act_layout(&note, s));
    WIN.with(|w| {
        if let Some(w) = w.borrow().as_ref() {
            let mut i = w.input.borrow_mut();
            i.scale = f64::from(px);
            i.clicks.clear();
            i.pressed = None;
        }
    });
    UP.with(|u| *u.borrow_mut() = Some(Up { note, th, px, s, work, since, k: None, layout }));
    tick();
    true
}

pub fn replace(note: Note) -> bool {
    let Some((th, px, work)) = UP.with(|u| u.borrow().as_ref().map(|u| (u.th, u.px, u.work))) else { return false };
    let now = Instant::now();
    // On screen already: no second fade-in.
    put_up(note, th, px, work, now.checked_sub(FADE_IN).unwrap_or(now))
}

pub fn hide() {
    UP.with(|u| *u.borrow_mut() = None);
    super::forget_picture();
    WIN.with(|w| {
        if let Some(w) = w.borrow_mut().as_mut() {
            w.layer.hide();
        }
    });
}

/// A click at `p` (window px) on a button: carry it out.
fn click(p: Pt) {
    let job = UP.with(|u| {
        let u = u.borrow();
        let up = u.as_ref()?;
        Some((act_at(up.layout.as_ref()?, p)?, up.note.clone(), up.th))
    });
    let Some((a, note, th)): Option<(NoteAct, Note, Theme)> = job else { return };
    let next = super::run_note_action(&note, a, th, |n| {
        *LATER.lock().unwrap_or_else(|e| e.into_inner()) = Some(n);
    });
    match next {
        Next::Keep => {}
        Next::Show(n) => {
            replace(n);
        }
        Next::Hide => hide(),
    }
}

/// Advance the note (main thread, every driver tick): its clicks, a
/// finished upload, the fade. True while one is up (the driver ticks fast).
pub fn tick() -> bool {
    if let Some(n) = LATER.lock().unwrap_or_else(|e| e.into_inner()).take()
        && !replace(n.clone())
    {
        crate::tray::notify(&n.text);
    }
    let clicks = WIN.with(|w| w.borrow().as_ref().map(|w| std::mem::take(&mut w.input.borrow_mut().clicks)).unwrap_or_default());
    for c in clicks {
        click(c);
    }
    let frame = UP.with(|u| {
        let mut u = u.borrow_mut();
        let up = u.as_mut()?;
        let Some(k) = opacity(up.since.elapsed(), up.note.ttl) else {
            *u = None;
            return Some(None);
        };
        if up.k.is_some_and(|old| (old - k).abs() < 0.004) {
            return Some(Some(None)); // holding: no new frame
        }
        up.k = Some(k);
        // No hover here (the panel reports presses only): a thumbnail
        // shows its buttons all along.
        let hover = up.note.thumb.as_ref().map(|_| NoteAct::Card);
        let (w, h, px) = render_note(&up.note, &up.th, up.s, k, hover);
        let pts = ((w as f32 / up.px).round() as u32, (h as f32 / up.px).round() as u32);
        let (x, y) = place_note(up.work, &up.note, pts);
        Some(Some(Some((x, y, w, h, px, up.px))))
    });
    match frame {
        None => false,
        // Over: down.
        Some(None) => {
            hide();
            false
        }
        Some(Some(None)) => true,
        Some(Some(Some((x, y, w, h, px, k)))) => {
            WIN.with(|win| {
                if let Some(win) = win.borrow_mut().as_mut() {
                    win.layer.put((f64::from(x), f64::from(y)), f64::from(k), w, h, &px);
                }
            });
            true
        }
    }
}
