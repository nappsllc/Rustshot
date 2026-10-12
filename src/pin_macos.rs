//! macOS: each pin a borderless, non-activating panel at the floating
//! level (`wind::Layer`, above other windows, on every Space) on the main
//! thread. A drag moves it and a double-click closes it, handled on the
//! daemon driver's ticks ([`tick`]); the wheel, the keyboard and the menu
//! are not wired up here yet (the panel reports presses only).
//!
//! Geometry: the shared code's pixels are points × the backing factor
//! (`capture::backing`); the panel is placed in points and drawn at that
//! factor, so a pin at 100 % shows the capture pixel for pixel.
#![cfg_attr(test, allow(dead_code))] // tests record pins instead (`pin::take_opened`)

use super::{dragged, is_double, is_drag, render, Frame};
use crate::pixbuf::PixBuf;
use crate::theme::Theme;
use crate::wind::{AuxKind, Layer};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

type Events = Rc<RefCell<Vec<(AuxKind, (f64, f64))>>>;

struct Pin {
    layer: Layer,
    /// Pixels per point.
    k: f64,
    s: f32,
    /// The image's top-left (pixels).
    origin: (i32, i32),
    px: Vec<u8>,
    size: (u32, u32),
    /// Pointer events since the last tick (kind, global points).
    events: Events,
    /// A press: where (pixels) and the origin then; and whether it moved.
    press: Option<((i32, i32), (i32, i32))>,
    moved: bool,
    /// The last click (time, pixels), for a double-click.
    last: Option<(Instant, (i32, i32))>,
}

impl Pin {
    fn show(&mut self) {
        let (x, y) = Frame::at(self.s).window_pos(self.origin);
        let pts = (f64::from(x) / self.k, f64::from(y) / self.k);
        self.layer.put(pts, self.k, self.size.0, self.size.1, &self.px);
    }
}

thread_local! {
    static PINS: RefCell<Vec<Pin>> = const { RefCell::new(Vec::new()) };
}

fn report() {
    super::set_count(PINS.with(|p| p.borrow().len()));
}

pub fn open(img: Arc<PixBuf>, at: (i32, i32), th: Theme) -> bool {
    if !crate::macos_objc::is_main_thread() {
        return false;
    }
    let layer = match Layer::new(false, false) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("rustshot: no pin window: {e:#}");
            return false;
        }
    };
    let events: Events = Rc::new(RefCell::new(Vec::new()));
    let ev = events.clone();
    layer.on_pointer(move |e| ev.borrow_mut().push((e.kind, e.global)));
    let k = crate::capture::backing().max(1.0);
    let s = crate::uisize::apply(k as f32);
    let (w, h, px) = render(&img, &th, s);
    let mut pin = Pin { layer, k, s, origin: at, px, size: (w, h), events, press: None, moved: false, last: None };
    pin.show();
    PINS.with(|p| p.borrow_mut().push(pin));
    report();
    true
}

pub fn close_all() {
    let pins = PINS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    drop(pins);
    report();
}

/// The pins' pointer input since the last tick: drags move them, a
/// double-click closes one.
pub fn tick() {
    let mut closed = Vec::new();
    PINS.with(|p| {
        let Ok(mut pins) = p.try_borrow_mut() else { return };
        for (i, pin) in pins.iter_mut().enumerate() {
            let evs = std::mem::take(&mut *pin.events.borrow_mut());
            for (kind, g) in evs {
                let at = ((g.0 * pin.k).round() as i32, (g.1 * pin.k).round() as i32);
                match kind {
                    AuxKind::Down => {
                        pin.press = Some((at, pin.origin));
                        pin.moved = false;
                    }
                    AuxKind::Drag => {
                        if let Some((press, o)) = pin.press
                            && (pin.moved || is_drag(press, at))
                        {
                            pin.moved = true;
                            pin.origin = dragged(o, press, at);
                            pin.show();
                        }
                    }
                    AuxKind::Up => {
                        let click = pin.press.take().is_some() && !pin.moved;
                        if !click {
                            continue;
                        }
                        let now = Instant::now();
                        if let Some((t, p)) = pin.last
                            && is_double(now.duration_since(t).as_millis(), (at.0 - p.0, at.1 - p.1))
                        {
                            closed.push(i);
                            break;
                        }
                        pin.last = Some((now, at));
                    }
                }
            }
        }
        // Dropped here, out of the panels' own event handling.
        for i in closed.iter().rev() {
            pins.remove(*i);
        }
    });
    if !closed.is_empty() {
        report();
    }
}
