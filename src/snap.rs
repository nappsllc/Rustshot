//! Window snapping: the windows on the screen when a capture starts, so the
//! overlay can outline the one under the pointer, select it with a click and
//! step into its parts with Tab.
//!
//! [`take`] lists them once, before the overlay shows (its own windows are
//! left out), front to back, each with its child parts (Windows child
//! windows, X11 subwindows; macOS lists none), and [`Snapshot::build`] moves
//! them into the shot's image coordinates, clipped to it (every part to its
//! window). Hit testing ([`Snapshot::chain`]) is pure over that list:
//! hovering costs a scan of a few hundred rects and no system call. Off on
//! Wayland (no window positions) and with `snap_windows = false`.
//!
//! The per-OS listing is in `snap_win.rs` / `snap_linux.rs` /
//! `snap_macos.rs`; the decisions they make about a single window (which
//! ones count, which rect) are the pure functions here, tested everywhere.

/// A rect in pixels, end-exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct R {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl R {
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> R {
        R { x0, y0, x1, y1 }
    }

    /// From a position and a size.
    #[cfg_attr(windows, allow(dead_code))]
    pub fn xywh(x: i32, y: i32, w: i32, h: i32) -> R {
        R::new(x, y, x.saturating_add(w), y.saturating_add(h))
    }

    pub fn w(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    pub fn h(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn intersect(&self, o: &R) -> R {
        R::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1))
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    pub fn offset(&self, dx: i32, dy: i32) -> R {
        R::new(self.x0 + dx, self.y0 + dy, self.x1 + dx, self.y1 + dy)
    }

    /// Grown by `l`, `t`, `r`, `b` on each side (negative: shrunk).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn grow(&self, l: i32, t: i32, r: i32, b: i32) -> R {
        R::new(self.x0 - l, self.y0 - t, self.x1 + r, self.y1 + b)
    }
}

/// A window (or a part of one) as the platform lists it: its rect in global
/// (virtual-screen physical) pixels and its parts, front to back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub r: R,
    pub kids: Vec<Node>,
}

impl Node {
    pub fn new(r: R, kids: Vec<Node>) -> Node {
        Node { r, kids }
    }
}

/// Most rects a snapshot keeps (a tree of thousands of controls is cut).
pub const MAX_NODES: usize = 20_000;
/// Deepest part a snapshot keeps (window = 0).
pub const MAX_DEPTH: usize = 8;

/// The windows of one capture in image coordinates, front to back, each
/// clipped to the shot and every part to its window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    rects: Vec<R>,
    kids: Vec<Vec<u32>>,
    tops: Vec<u32>,
}

impl Snapshot {
    /// From the platform's windows (front to back, global pixels) for a
    /// shot at `origin` of `size`. Empty rects are dropped with their
    /// parts; a part covering all of its window (but a border of up to
    /// 2 px) is not a step of its own (its parts take its place).
    pub fn build(tops: &[Node], origin: (i32, i32), size: (u32, u32)) -> Snapshot {
        let mut s = Snapshot::default();
        let shot = R::new(0, 0, size.0 as i32, size.1 as i32);
        let d = (-origin.0, -origin.1);
        for t in tops {
            if s.rects.len() >= MAX_NODES {
                break;
            }
            let r = t.r.offset(d.0, d.1).intersect(&shot);
            if r.is_empty() {
                continue;
            }
            let id = s.push(r);
            let mut kids = Vec::new();
            s.add_kids(&t.kids, r, d, 1, &mut kids);
            s.kids[id as usize] = kids;
            s.tops.push(id);
        }
        s
    }

    fn push(&mut self, r: R) -> u32 {
        self.rects.push(r);
        self.kids.push(Vec::new());
        (self.rects.len() - 1) as u32
    }

    fn add_kids(&mut self, kids: &[Node], parent: R, d: (i32, i32), depth: usize, out: &mut Vec<u32>) {
        if depth > MAX_DEPTH {
            return;
        }
        for k in kids {
            if self.rects.len() >= MAX_NODES {
                return;
            }
            let r = k.r.offset(d.0, d.1).intersect(&parent);
            if r.is_empty() {
                continue;
            }
            if covers(r, parent) {
                self.add_kids(&k.kids, parent, d, depth + 1, out);
                continue;
            }
            let id = self.push(r);
            let mut sub = Vec::new();
            self.add_kids(&k.kids, r, d, depth + 1, &mut sub);
            self.kids[id as usize] = sub;
            out.push(id);
        }
    }

    /// The window under (`x`, `y`), then the part of it there, the part of
    /// that, ...: outermost first. Empty over no window (the desktop).
    pub fn chain(&self, x: i32, y: i32) -> Vec<R> {
        let mut out = Vec::new();
        let mut level: &[u32] = &self.tops;
        while let Some(&id) = level.iter().find(|&&i| self.rects[i as usize].contains(x, y)) {
            out.push(self.rects[id as usize]);
            level = &self.kids[id as usize];
        }
        out
    }

    /// Top-level windows kept.
    pub fn windows(&self) -> usize {
        self.tops.len()
    }

    /// Parts kept (every rect but the windows).
    pub fn parts(&self) -> usize {
        self.rects.len() - self.tops.len()
    }

    /// The windows' rects, front to back.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn window_rects(&self) -> Vec<R> {
        self.tops.iter().map(|&i| self.rects[i as usize]).collect()
    }

    /// The direct parts of window `i` (an index into [`Snapshot::window_rects`]).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn part_rects(&self, i: usize) -> Vec<R> {
        self.tops.get(i).map_or(Vec::new(), |&t| self.kids[t as usize].iter().map(|&k| self.rects[k as usize]).collect())
    }
}

/// A part that is all of its window but a border of at most 2 px (an X11
/// window's border, a form inside its shell) is no step of its own.
fn covers(part: R, parent: R) -> bool {
    part.x0 - parent.x0 <= 2 && part.y0 - parent.y0 <= 2 && parent.x1 - part.x1 <= 2 && parent.y1 - part.y1 <= 2
}

/// What the overlay outlines: the chain under the pointer and how deep
/// into it Tab went.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hover {
    /// Outermost first (see [`Snapshot::chain`]); the fallback (the
    /// monitor) when over no window.
    pub chain: Vec<R>,
    /// The depth Tab asked for; what is outlined is as deep as the chain
    /// under the pointer goes ([`Hover::depth`]).
    pub want: usize,
}

impl Hover {
    /// The depth outlined.
    pub fn depth(&self) -> usize {
        self.want.min(self.chain.len().saturating_sub(1))
    }

    /// The rect outlined.
    pub fn rect(&self) -> Option<R> {
        self.chain.get(self.depth()).copied()
    }

    /// There is a part deeper than the rect outlined.
    pub fn has_parts(&self) -> bool {
        self.depth() + 1 < self.chain.len()
    }

    /// Tab: one part deeper (false when there is none).
    pub fn deeper(&mut self) -> bool {
        let d = self.depth();
        if d + 1 < self.chain.len() {
            self.want = d + 1;
            true
        } else {
            false
        }
    }

    /// Shift+Tab: one step back out (false at the window).
    pub fn outer(&mut self) -> bool {
        let d = self.depth();
        self.want = d.saturating_sub(1);
        d > 0
    }

    /// The pointer moved to where `chain` is: over the same window the
    /// depth Tab chose holds; over another one it starts from the window.
    pub fn moved(&self, chain: Vec<R>) -> Hover {
        let want = if chain.first() == self.chain.first() { self.want } else { 0 };
        Hover { chain, want }
    }
}

// --- per-window decisions (pure; the platform code fills them in) ---------

/// What Windows says about a top-level window (`EnumWindows`).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default)]
pub struct WinInfo {
    pub visible: bool,
    pub iconic: bool,
    /// `DWMWA_CLOAKED`: on another virtual desktop, a suspended UWP app, ...
    pub cloaked: bool,
    /// One of ours (the overlay, Settings).
    pub own: bool,
    /// The desktop or shell window (`Progman`, `WorkerW`): the "whole
    /// screen" fallback, never a window of its own.
    pub shell: bool,
    pub tool: bool,
    /// Click-through (`WS_EX_TRANSPARENT`) or a layered window at alpha 0:
    /// nothing a user sees as a window (overlays of other apps).
    pub transparent: bool,
    /// `GetWindowRect` (with the invisible resize borders).
    pub rect: R,
    /// `DWMWA_EXTENDED_FRAME_BOUNDS`: the frame as drawn (no invisible
    /// borders, no shadow), when DWM has one.
    pub frame: Option<R>,
}

/// The rect a top-level window snaps to, `None` when it does not count.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn win_top_rect(i: &WinInfo) -> Option<R> {
    if !i.visible || i.iconic || i.cloaked || i.own || i.shell || i.transparent {
        return None;
    }
    let r = match i.frame {
        Some(f) if !f.is_empty() => f,
        _ => i.rect,
    };
    // Tool windows with no real extent (1-3 px helpers, hidden hosts).
    let min = if i.tool { 4 } else { 1 };
    (r.w() >= min && r.h() >= min).then_some(r)
}

/// An X11 window's rect: the client window `client` (root coordinates)
/// grown by the window manager's frame (`_NET_FRAME_EXTENTS`: left, right,
/// top, bottom) and shrunk by the client-side shadow (`_GTK_FRAME_EXTENTS`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn x11_rect(client: R, net: Option<[i32; 4]>, gtk: Option<[i32; 4]>) -> R {
    let mut r = client;
    if let Some([l, rt, t, b]) = net {
        r = r.grow(l.max(0), t.max(0), rt.max(0), b.max(0));
    }
    if let Some([l, rt, t, b]) = gtk {
        let s = r.grow(-l.max(0), -t.max(0), -rt.max(0), -b.max(0));
        if !s.is_empty() {
            r = s;
        }
    }
    r
}

/// Whether a `CGWindowListCopyWindowInfo` entry counts: someone else's,
/// visible, not tiny, on the normal layer (0), a floating or modal panel
/// (3-8) or a popup menu / tooltip (101-200); not the Dock (20), the menu
/// bar (24, 25), the desktop or the screen saver.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mac_keep(layer: i64, alpha: f64, own: bool, w: f64, h: f64) -> bool {
    let layer_ok = matches!(layer, 0 | 3..=8 | 101..=200);
    !own && layer_ok && alpha > 0.0 && w >= 1.0 && h >= 1.0
}

// --- taking a snapshot ------------------------------------------------------

#[cfg(windows)]
#[path = "snap_win.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "snap_linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "snap_macos.rs"]
mod imp;

/// Get the first snapshot's one-time costs (Windows: loading dwmapi and
/// its first DWM query) out of the way on a background thread.
pub fn warm_up() {
    let _ = std::thread::Builder::new().name("snap-warm".into()).spawn(imp::warm);
}

/// The windows on screen now for a shot at `origin` of `size` (`None`
/// where windows cannot be listed: Wayland, an X error). Timed in the
/// `RUSTSHOT_PERFLOG`.
#[cfg_attr(test, allow(dead_code))]
pub fn take(origin: (i32, i32), size: (u32, u32)) -> Option<Snapshot> {
    let t0 = std::time::Instant::now();
    let within = R::xywh(origin.0, origin.1, size.0 as i32, size.1 as i32);
    let tops = imp::windows(Some(std::process::id()), within)?;
    let listed = t0.elapsed().as_secs_f64() * 1000.0;
    let s = Snapshot::build(&tops, origin, size);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    crate::perf::note(&format!("snap {ms:.3} list {listed:.3} windows {} parts {}", s.windows(), s.parts()));
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: i32, y: i32, w: i32, h: i32, kids: Vec<Node>) -> Node {
        Node::new(R::xywh(x, y, w, h), kids)
    }

    fn rr(x: i32, y: i32, w: i32, h: i32) -> R {
        R::xywh(x, y, w, h)
    }

    #[test]
    fn the_front_window_wins_and_popups_are_their_own() {
        // A menu (front), a dialog over the editor window, the editor.
        let menu = n(300, 200, 160, 240, vec![]);
        let dialog = n(400, 300, 400, 300, vec![]);
        let editor = n(100, 100, 1200, 800, vec![]);
        let s = Snapshot::build(&[menu, dialog, editor], (0, 0), (1920, 1080));
        assert_eq!(s.windows(), 3);
        assert_eq!(s.chain(350, 250), vec![rr(300, 200, 160, 240)], "the popup alone");
        assert_eq!(s.chain(450, 320), vec![rr(300, 200, 160, 240)], "the menu over the dialog");
        assert_eq!(s.chain(700, 500), vec![rr(400, 300, 400, 300)], "the dialog over the editor");
        assert_eq!(s.chain(150, 150), vec![rr(100, 100, 1200, 800)]);
        assert!(s.chain(1500, 950).is_empty(), "the desktop");
        // Edges: end-exclusive.
        assert_eq!(s.chain(1299, 899), vec![rr(100, 100, 1200, 800)]);
        assert!(s.chain(1300, 899).is_empty());
    }

    #[test]
    fn parts_nest_and_are_clipped_to_their_window() {
        // Window > sidebar (with a list) and a panel running past the window.
        let list = n(110, 150, 180, 300, vec![n(110, 150, 180, 20, vec![])]);
        let side = n(100, 140, 200, 660, vec![list]);
        let panel = n(300, 140, 1200, 200, vec![]);
        let w = n(100, 100, 1000, 700, vec![side, panel]);
        let s = Snapshot::build(&[w], (0, 0), (1920, 1080));
        assert_eq!(s.parts(), 4);
        assert_eq!(s.part_rects(0), vec![rr(100, 140, 200, 660), rr(300, 140, 800, 200)], "clipped to the window");
        assert_eq!(
            s.chain(120, 155),
            vec![rr(100, 100, 1000, 700), rr(100, 140, 200, 660), rr(110, 150, 180, 300), rr(110, 150, 180, 20)]
        );
        assert_eq!(s.chain(1050, 200), vec![rr(100, 100, 1000, 700), rr(300, 140, 800, 200)]);
        assert_eq!(s.chain(1150, 200), Vec::<R>::new(), "the clipped-off end is not the window");
        assert_eq!(s.chain(600, 600), vec![rr(100, 100, 1000, 700)], "no part there");
    }

    #[test]
    fn a_part_filling_its_window_is_not_a_step() {
        // A client-area host covering its window, holding two panes.
        let host = n(0, 0, 800, 600, vec![n(0, 0, 200, 600, vec![]), n(200, 0, 600, 600, vec![])]);
        let s = Snapshot::build(&[n(0, 0, 800, 600, vec![host])], (0, 0), (800, 600));
        assert_eq!(s.part_rects(0), vec![rr(0, 0, 200, 600), rr(200, 0, 600, 600)]);
        assert_eq!(s.chain(10, 10), vec![rr(0, 0, 800, 600), rr(0, 0, 200, 600)]);
        // An X11 shell with a 1 px border around its form: the form is no
        // step either, its button is.
        let form = n(301, 201, 640, 360, vec![n(305, 530, 34, 19, vec![])]);
        let s = Snapshot::build(&[n(300, 200, 642, 362, vec![form])], (0, 0), (1920, 1080));
        assert_eq!(s.part_rects(0), vec![rr(305, 530, 34, 19)]);
        // 3 px in is a part.
        let s = Snapshot::build(&[n(0, 0, 100, 100, vec![n(3, 3, 94, 94, vec![])])], (0, 0), (200, 200));
        assert_eq!(s.parts(), 1);
    }

    #[test]
    fn negative_monitors_move_into_the_image_and_off_shot_windows_drop() {
        // A monitor left of and above the primary: the shot starts at
        // (-1920, -200); a window there, one across both, one off the shot.
        let left = n(-1800, -100, 800, 600, vec![n(-1790, -60, 100, 100, vec![])]);
        let across = n(-400, 100, 1000, 500, vec![]);
        let off = n(-5000, 0, 400, 300, vec![]);
        let s = Snapshot::build(&[left, across, off], (-1920, -200), (4480, 1640));
        assert_eq!(s.window_rects(), vec![rr(120, 100, 800, 600), rr(1520, 300, 1000, 500)]);
        assert_eq!(s.chain(135, 145), vec![rr(120, 100, 800, 600), rr(130, 140, 100, 100)]);
        // Partly off the shot: clipped to it.
        let s = Snapshot::build(&[n(-2000, -300, 400, 400, vec![])], (-1920, -200), (4480, 1640));
        assert_eq!(s.window_rects(), vec![rr(0, 0, 320, 300)]);
        // Empty and zero-size windows and parts are gone.
        let s = Snapshot::build(&[n(10, 10, 0, 50, vec![]), n(0, 0, 100, 100, vec![n(5, 5, 0, 0, vec![])])], (0, 0), (200, 200));
        assert_eq!((s.windows(), s.parts()), (1, 0));
    }

    #[test]
    fn the_snapshot_is_capped() {
        let wide: Vec<Node> = (0..MAX_NODES + 50).map(|i| n((i % 100) as i32, 0, 10, 10, vec![])).collect();
        let s = Snapshot::build(&wide, (0, 0), (200, 200));
        assert_eq!(s.windows(), MAX_NODES);
        // Depth: a chain of parts one inside the other stops at MAX_DEPTH.
        let mut deep = n(0, 0, 3, 3, vec![]);
        for i in 1..20 {
            deep = n(0, 0, 3 * (i + 1), 3 * (i + 1), vec![deep]);
        }
        let s = Snapshot::build(&[deep], (0, 0), (100, 100));
        assert_eq!(s.chain(0, 0).len(), MAX_DEPTH + 1);
    }

    #[test]
    fn tab_steps_in_and_out_and_holds_over_the_window() {
        let w = rr(0, 0, 1000, 800);
        let (side, list, item) = (rr(0, 40, 200, 760), rr(0, 60, 200, 400), rr(0, 60, 200, 20));
        let mut h = Hover::default().moved(vec![w, side, list, item]);
        assert_eq!((h.depth(), h.rect(), h.has_parts()), (0, Some(w), true));
        assert!(h.deeper());
        assert_eq!(h.rect(), Some(side));
        assert!(h.deeper() && h.deeper());
        assert_eq!(h.rect(), Some(item));
        assert!(!h.deeper(), "nothing deeper");
        assert!(!h.has_parts());
        assert!(h.outer());
        assert_eq!(h.rect(), Some(list));
        // Moving within the window: as deep as the chain there goes, and
        // back to the chosen depth where it goes deeper again.
        let h2 = h.moved(vec![w, rr(200, 40, 800, 100)]);
        assert_eq!(h2.rect(), Some(rr(200, 40, 800, 100)));
        let h3 = h2.moved(vec![w, side, list, item]);
        assert_eq!(h3.rect(), Some(list));
        // Another window: from the whole window again.
        let other = h3.moved(vec![rr(1000, 0, 400, 400), rr(1010, 10, 50, 50)]);
        assert_eq!((other.depth(), other.rect()), (0, Some(rr(1000, 0, 400, 400))));
        // Shift+Tab at the window stays there.
        let mut o = other;
        assert!(!o.outer());
        assert_eq!(o.rect(), Some(rr(1000, 0, 400, 400)));
        assert_eq!(Hover::default().rect(), None);
    }

    #[test]
    fn windows_top_levels_use_the_dwm_frame_and_skip_what_is_not_a_window() {
        // A Windows 11 window: GetWindowRect has 7 px invisible borders
        // left/right/bottom; the DWM frame is what is drawn.
        let base = WinInfo {
            visible: true,
            rect: rr(93, 100, 1214, 807),
            frame: Some(rr(100, 100, 1200, 800)),
            ..WinInfo::default()
        };
        assert_eq!(win_top_rect(&base), Some(rr(100, 100, 1200, 800)));
        assert_eq!(win_top_rect(&WinInfo { frame: None, ..base }), Some(rr(93, 100, 1214, 807)), "no DWM frame");
        assert_eq!(win_top_rect(&WinInfo { frame: Some(R::default()), ..base }), Some(rr(93, 100, 1214, 807)));
        for (what, i) in [
            ("invisible", WinInfo { visible: false, ..base }),
            ("minimised", WinInfo { iconic: true, ..base }),
            ("cloaked", WinInfo { cloaked: true, ..base }),
            ("ours", WinInfo { own: true, ..base }),
            ("the desktop", WinInfo { shell: true, ..base }),
            ("click-through", WinInfo { transparent: true, ..base }),
            ("zero size", WinInfo { rect: rr(5, 5, 0, 0), frame: None, ..base }),
            ("a 1 px tool window", WinInfo { tool: true, rect: rr(0, 0, 1, 1), frame: None, ..base }),
        ] {
            assert_eq!(win_top_rect(&i), None, "{what}");
        }
        // A tooltip (a tool window with an extent) counts.
        let tip = WinInfo { tool: true, rect: rr(400, 400, 120, 24), frame: None, ..base };
        assert_eq!(win_top_rect(&tip), Some(rr(400, 400, 120, 24)));
    }

    #[test]
    fn x11_frames_add_decorations_and_drop_client_side_shadows() {
        let client = rr(100, 130, 800, 600);
        assert_eq!(x11_rect(client, None, None), client);
        // A reparenting WM: 4 px borders and a 26 px title above.
        assert_eq!(x11_rect(client, Some([4, 4, 30, 4]), None), R::new(96, 100, 904, 734));
        // A GTK window drawing its own 24 px shadow (and a 6 px top one).
        assert_eq!(x11_rect(rr(76, 106, 848, 648), None, Some([24, 24, 6, 42])), R::new(100, 112, 900, 712));
        // Nonsense extents never turn the rect inside out.
        assert_eq!(x11_rect(rr(0, 0, 10, 10), None, Some([20, 20, 20, 20])), rr(0, 0, 10, 10));
        assert_eq!(x11_rect(client, Some([-5, -5, -5, -5]), None), client);
    }

    #[test]
    fn macos_keeps_normal_windows_panels_and_popups() {
        assert!(mac_keep(0, 1.0, false, 800.0, 600.0));
        assert!(mac_keep(3, 1.0, false, 300.0, 200.0), "floating panel");
        assert!(mac_keep(101, 1.0, false, 200.0, 300.0), "popup menu");
        assert!(!mac_keep(0, 1.0, true, 800.0, 600.0), "ours");
        assert!(!mac_keep(0, 0.0, false, 800.0, 600.0), "transparent");
        assert!(!mac_keep(20, 1.0, false, 1440.0, 80.0), "the Dock");
        assert!(!mac_keep(24, 1.0, false, 1440.0, 25.0), "the menu bar");
        assert!(!mac_keep(-2147483623, 1.0, false, 1440.0, 900.0), "the desktop");
        assert!(!mac_keep(0, 1.0, false, 0.0, 600.0), "zero width");
    }

    /// The snapshot of the live desktop: some windows, every rect inside
    /// the shot. On Linux under Xvfb (CI) this lists whatever test windows
    /// are up; on Wayland there is none.
    #[test]
    #[ignore = "live display access"]
    fn live_snapshot_lists_windows() {
        crate::capture::enable_dpi_awareness();
        let mons = crate::capture::monitors().expect("monitors");
        let x0 = mons.iter().map(|m| m.x).min().unwrap();
        let y0 = mons.iter().map(|m| m.y).min().unwrap();
        let x1 = mons.iter().map(|m| m.x + m.w as i32).max().unwrap();
        let y1 = mons.iter().map(|m| m.y + m.h as i32).max().unwrap();
        let size = ((x1 - x0) as u32, (y1 - y0) as u32);
        let t0 = std::time::Instant::now();
        let Some(s) = take((x0, y0), size) else {
            eprintln!("SKIP: no window list here (Wayland?)");
            return;
        };
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        eprintln!("snapshot: {ms:.2} ms, {} windows, {} parts", s.windows(), s.parts());
        let shot = R::new(0, 0, size.0 as i32, size.1 as i32);
        for r in s.window_rects() {
            assert_eq!(r.intersect(&shot), r, "{r:?} inside the shot");
        }
    }
}
