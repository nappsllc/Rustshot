//! The History window without a window: its state (selection, search,
//! groups, Esc, the panel), the layout's numbers from the boards, and
//! preview PNGs of every board state, dark and light
//! (`RUSTSHOT_PREVIEW_DIR`).

use super::draw::{self, Target};
use super::window::{self, Press};
use super::*;
use crate::history::tests::TempRoot;
use crate::history::{Kind, Meta};
use crate::objects::{FRect, Pt};
use crate::theme::{DARK, LIGHT, Theme};
use crate::ui::{FocusState, Input};
use crate::wind::{Ev, Mods};

pub(crate) const TODAY: (i32, u32, u32) = (2026, 10, 12);

/// A mock page (a window with a few blocks) as a thumbnail.
fn page(w: u32, h: u32, bg: [u8; 3], accent: [u8; 3]) -> PixBuf {
    let mut img = PixBuf::from_pixel(w, h, [bg[0], bg[1], bg[2], 255]);
    let mut rect = |x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 3]| {
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                let i = ((y * w + x) * 4) as usize;
                img.as_raw_mut()[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    };
    rect(0, 0, w, h / 9, [255, 255, 255]);
    rect(w / 12, h / 4, w / 2, h / 4 + 8, [0xDA, 0xDE, 0xE6]);
    rect(w / 12, h / 3 + 4, w - w / 12, h - h / 6, accent);
    img
}

/// (title, app, time, kind, edited, size, page colors)
type Spec = (&'static str, &'static str, (i32, u32, u32, u32, u32, u32), Kind, bool, (u32, u32), [u8; 3], [u8; 3]);

const SPECS: [Spec; 10] = [
    ("Landing page", "Figma", (2026, 10, 12, 14, 32, 0), Kind::Shot, false, (1440, 900), [0xF4, 0xF5, 0xF8], [0xE8, 0xEA, 0xF1]),
    ("rustshot/src/overlay.rs", "VS Code", (2026, 10, 12, 13, 5, 0), Kind::Mp4, false, (1920, 1080), [0x1E, 0x22, 0x30], [0x3A, 0x40, 0x52]),
    ("Stripe Dashboard", "Chrome", (2026, 10, 12, 11, 48, 0), Kind::Shot, true, (1280, 720), [0xF4, 0xF5, 0xF8], [0xDC, 0xE6, 0xF5]),
    ("#design-review", "Slack", (2026, 10, 12, 9, 14, 7), Kind::Shot, true, (1180, 760), [0xF6, 0xF7, 0xF9], [0xFF, 0xFF, 0xFF]),
    ("invoice-0042.pdf", "Preview", (2026, 10, 11, 19, 50, 0), Kind::Shot, false, (820, 540), [0xFF, 0xFF, 0xFF], [0xDA, 0xDE, 0xE6]),
    ("cargo run", "Terminal", (2026, 10, 11, 17, 21, 0), Kind::Gif, false, (960, 540), [0x2B, 0x2D, 0x36], [0x3B, 0x82, 0xF6]),
    ("Q4 roadmap", "Notion", (2026, 10, 11, 16, 2, 0), Kind::Shot, false, (1440, 900), [0xFF, 0xFF, 0xFF], [0xF2, 0xE8, 0xD8]),
    ("Weekly sync", "Zoom", (2026, 10, 11, 10, 30, 0), Kind::Mp4, false, (1920, 1080), [0xF4, 0xF5, 0xF8], [0xDC, 0xE6, 0xF5]),
    ("Stripe \u{b7} Payments \u{b7} pi_3Q8x", "Chrome", (2026, 10, 9, 16, 40, 0), Kind::Shot, false, (1440, 900), [0xF4, 0xF5, 0xF8], [0xE8, 0xEA, 0xF1]),
    ("billing/stripe_webhook.rs", "VS Code", (2026, 10, 9, 10, 2, 0), Kind::Shot, false, (1920, 1080), [0x1E, 0x22, 0x30], [0x3A, 0x40, 0x52]),
];

/// A store with the boards' captures.
pub(crate) fn fixture(tag: &str) -> (TempRoot, Store) {
    let root = TempRoot::new(tag);
    let st = Store::new(&root.0);
    for (title, app, t, kind, edited, (w, hh), bg, ac) in SPECS {
        let (_, dir) = st.reserve(t).unwrap();
        let pic = page(w / 8, hh / 8, bg, ac);
        let meta = Meta {
            captured: t,
            kind,
            title: title.into(),
            app: app.into(),
            monitor: "Built-in Display (2\u{d7})".into(),
            scale: 2.0,
            edited,
            duration_ms: if kind == Kind::Shot { 0 } else { 42_000 },
            saved: Some(PathBuf::from(format!("/home/u/Pictures/rustshot/{:04}-{:02}-{:02}_{:02}-{:02}.png", t.0, t.1, t.2, t.3, t.4))),
            uploaded: (title == "#design-review").then(|| "https://rs.sh/a8Kq2".into()),
            ..Meta::default()
        };
        if kind == Kind::Shot {
            let objs = if edited {
                vec![
                    crate::objects::Obj::Arrow { a: Pt::new(20.0, 70.0), b: Pt::new(90.0, 30.0), color: crate::uifb::C4::rgb(240, 68, 56), width: 3.0 },
                    crate::objects::Obj::Callout { fmt: Default::default(),
                        r: FRect { x: 100.0, y: 10.0, w: 40.0, h: 24.0 },
                        tail: Pt::new(105.0, 50.0),
                        text: String::new(),
                        color: crate::uifb::C4::rgb(240, 68, 56),
                        size: 12.0,
                        style: crate::objects::CalloutStyle::Rounded,
                        fill: true,
                    },
                ]
            } else {
                Vec::new()
            };
            st.write_shot(&dir, &pic, &objs, None, &meta).unwrap();
            // The capture's real size (the fixture keeps small pictures).
            st.update(&format!("{:04}-{:02}-{:02}/{}", t.0, t.1, t.2, dir.file_name().unwrap().to_string_lossy()), |mm| {
                mm.w = w;
                mm.h = hh;
                mm.bytes = 486_000;
                mm.format = "PNG".into();
            })
            .unwrap();
        } else {
            let meta = Meta { w, h: hh, format: kind.label().into(), file: Some(PathBuf::from("/home/u/Videos/rec.mp4")), saved: None, ..meta };
            st.write_recording(&dir, Some(&pic), &meta).unwrap();
        }
    }
    (root, st)
}

fn history(st: &Store) -> History {
    let cfg = Config::default();
    History::new(st.clone(), cfg, TODAY)
}

/// Two frames (the layout and focus first, then the picture with `inp`).
fn render(h: &mut History, th: &Theme, w: u32, hh: u32, k: f32, inp: &Input) -> (PixBuf, Vec<Act>) {
    let mut focus = FocusState::default();
    render_with(h, th, w, hh, k, inp, &mut focus)
}

fn render_with(h: &mut History, th: &Theme, w: u32, hh: u32, k: f32, inp: &Input, focus: &mut FocusState) -> (PixBuf, Vec<Act>) {
    let mut img = PixBuf::new((w as f32 * k).round() as u32, (hh as f32 * k).round() as u32);
    let now = std::time::Instant::now();
    window::frame(&mut img, th, &Input::default(), focus, h, k, now);
    let (acts, _) = window::frame(&mut img, th, inp, focus, h, k, now);
    (img, acts)
}

fn at(x: f32, y: f32, k: f32) -> Input {
    let mut inp = Input::default();
    inp.feed(&Ev::Move { x: (x * k) as i32, y: (y * k) as i32 });
    inp
}

fn click_at(x: f32, y: f32, k: f32, mods: Mods) -> Input {
    let mut inp = Input::default();
    let (x, y) = ((x * k) as i32, (y * k) as i32);
    inp.feed(&Ev::Key { vk: 0x10, up: false, repeat: false, mods });
    inp.keys.clear();
    for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
        inp.feed(&e);
    }
    inp
}

fn layout_of(h: &History, w: f32, hh: f32) -> draw::Lay {
    let th = DARK;
    let mut img = PixBuf::new(w as u32, hh as u32);
    let inp = Input::default();
    let mut focus = FocusState::default();
    let mut clip = crate::ui::preview::FakeClip::default();
    let fb = crate::uifb::Fb::new(img.as_raw_mut(), w as usize);
    let ui = crate::ui::Ui::new(fb, &th, &inp, &mut focus, &mut clip, 1.0, FRect { x: 0.0, y: 0.0, w, h: hh });
    draw::layout(&ui, h, w, hh)
}

fn centre(r: FRect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

#[test]
fn groups_follow_the_days_and_the_search() {
    let (_r, st) = fixture("groups");
    let mut h = history(&st);
    assert_eq!(h.entries.len(), 10);
    let g = h.groups();
    let heads: Vec<(&str, Option<&str>, usize)> = g.iter().map(|g| (g.title.as_str(), g.small.as_deref(), g.items.len())).collect();
    assert_eq!(heads, [("Today", Some("\u{b7} 4 captures"), 4), ("Yesterday", Some("\u{b7} Sun 11 Oct"), 4), ("Fri 9 Oct", None, 2)]);
    // Search: one group of results, the matches highlighted.
    h.search.set("stripe");
    let g = h.groups();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].title, "3 results for \"stripe\"");
    let titles: Vec<String> = g[0].items.iter().map(|(i, _)| history::title(&h.entries[*i].meta)).collect();
    assert_eq!(titles, ["Chrome \u{2014} Stripe Dashboard", "Chrome \u{2014} Stripe \u{b7} Payments \u{b7} pi_3Q8x", "VS Code \u{2014} billing/stripe_webhook.rs"]);
    let (i, hl) = &g[0].items[0];
    assert_eq!(&history::title(&h.entries[*i].meta)[hl[0].clone()], "Stripe");
    // Esc clears the search, then the selection.
    h.sel = vec![h.entries[0].id.clone()];
    assert!(h.escape());
    assert!(h.search.text.is_empty() && h.sel.len() == 1);
    assert!(h.escape());
    assert!(h.sel.is_empty());
    assert!(!h.escape());
    // The filter.
    h.filter = Filter::Recordings;
    assert_eq!(h.shown().len(), 3);
    assert_eq!(h.groups()[0].small.as_deref(), Some("\u{b7} 1 capture"));
}

#[test]
fn clicks_select_one_toggle_and_extend() {
    let (_r, st) = fixture("select");
    let mut h = history(&st);
    let order: Vec<String> = h.entries.iter().map(|e| e.id.clone()).collect();
    h.click(&order[2], false, false, &order);
    assert_eq!(h.sel, [order[2].clone()]);
    assert!(h.single().is_some());
    h.click(&order[5], false, true, &order);
    assert_eq!(h.sel, order[2..=5].to_vec(), "Shift: the range");
    h.click(&order[3], true, false, &order);
    assert_eq!(h.sel, [order[2].clone(), order[4].clone(), order[5].clone()], "Ctrl toggles");
    h.click(&order[0], true, false, &order);
    assert_eq!(h.sel.len(), 4);
    assert!(h.single().is_none());
    h.click(&order[1], false, false, &order);
    assert_eq!(h.sel, [order[1].clone()]);
}

#[test]
fn layout_numbers_from_the_board() {
    let (_r, st) = fixture("layout");
    let mut h = history(&st);
    // 1280 x 720 with one selected: four cards a row, the panel 320 wide.
    h.sel = vec![h.entries[3].id.clone()];
    let l = layout_of(&h, 1280.0, 720.0);
    assert_eq!(l.cols, 4);
    assert_eq!(l.panel, Some(FRect { x: 960.0, y: 48.0, w: 320.0, h: 720.0 - 48.0 - 28.0 }));
    assert_eq!(l.items[0].r.x, 24.0);
    assert_eq!(l.items[0].thumb.h, 140.0);
    assert_eq!((l.items[0].r.w, l.items[1].r.x - l.items[0].r.x1()), (216.0, 16.0));
    assert_eq!(l.groups.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Today", "Yesterday", "Fri 9 Oct"]);
    assert_eq!(l.items[0].r.y, 30.0 + 4.0, "under its day header");
    // Below 900 px the panel collapses.
    let l = layout_of(&h, 899.0, 600.0);
    assert!(l.panel.is_none());
    // The list: 56 px rows with 72 x 48 thumbs.
    h.view = View::List;
    let l = layout_of(&h, 960.0, 640.0);
    assert_eq!((l.items[0].r.h, l.items[0].thumb.w, l.items[0].thumb.h), (56.0, 72.0, 48.0));
    assert_eq!(l.items[1].r.y - l.items[0].r.y, 56.0);
    assert!(l.content_h > 10.0 * 56.0);
}

#[test]
fn the_pointer_selects_edits_and_marquees() {
    let (_r, st) = fixture("pointer");
    let mut h = history(&st);
    let k = 1.0;
    let l = layout_of(&h, 1280.0, 720.0);
    let card = l.on_screen(l.items[2].r, 0.0);
    let (x, y) = centre(card);
    // A click selects, the details open.
    render(&mut h, &DARK, 1280, 720, k, &click_at(x, y, k, Mods::default()));
    assert_eq!(h.sel, [h.entries[2].id.clone()]);
    // A second click soon after edits it (the first one "just now": a
    // slow test run must not make it two single clicks).
    h.ptr.last_click = Some((h.entries[2].id.clone(), std::time::Instant::now()));
    let (_, acts) = render(&mut h, &DARK, 1280, 720, k, &click_at(x, y, k, Mods::default()));
    assert_eq!(acts, [Act::Edit(h.entries[2].id.clone())]);
    // The hover strip's Copy.
    let l = layout_of(&h, 1280.0, 720.0);
    let e = h.entries[0].clone();
    let q = draw::quick(&l, &l.items[0], &e, 0.0, false);
    assert_eq!(q.iter().map(|(_, a)| *a).collect::<Vec<_>>()[..2], [draw::Quick::Edit, draw::Quick::Copy]);
    let (cx, cy) = centre(q[1].0);
    let (_, acts) = render(&mut h, &DARK, 1280, 720, k, &click_at(cx, cy, k, Mods::default()));
    assert_eq!(acts, [Act::Copy(vec![e.id.clone()])]);
    // A marquee over the first two cards of Yesterday.
    h.sel.clear();
    let l = layout_of(&h, 1280.0, 720.0);
    let a = l.on_screen(l.items[4].r, 0.0);
    let b = l.on_screen(l.items[5].r, 0.0);
    let mut focus = FocusState::default();
    let start = (a.x - 8.0, a.y - 6.0);
    let mut inp = Input::default();
    inp.feed(&Ev::Move { x: start.0 as i32, y: start.1 as i32 });
    inp.feed(&Ev::Down { x: start.0 as i32, y: start.1 as i32 });
    render_with(&mut h, &DARK, 1280, 720, k, &inp, &mut focus);
    assert!(matches!(h.ptr.press, Some((Press::Marquee(_), _))));
    let mut inp = Input::default();
    inp.held = true;
    inp.feed(&Ev::Move { x: (b.x + 20.0) as i32, y: (b.y + 20.0) as i32 });
    inp.held = true;
    render_with(&mut h, &DARK, 1280, 720, k, &inp, &mut focus);
    assert_eq!(h.sel, [h.entries[4].id.clone(), h.entries[5].id.clone()]);
    // Delete with the keyboard: two ask first ("Delete 2 captures?").
    let mut inp = Input::default();
    inp.feed(&Ev::Key { vk: crate::wind::key::DELETE, up: false, repeat: false, mods: Mods::default() });
    let (_, acts) = render(&mut h, &DARK, 1280, 720, k, &inp);
    assert!(acts.is_empty());
    let two = vec![h.entries[4].id.clone(), h.entries[5].id.clone()];
    assert_eq!(h.ask_delete.as_ref(), Some(&two));
    assert_eq!(h.dialog_texts().0, "Delete 2 captures?");
    // Esc keeps them; Del again and the button deletes.
    let mut esc = Input::default();
    esc.feed(&Ev::Key { vk: crate::wind::key::ESCAPE, up: false, repeat: false, mods: Mods::default() });
    render(&mut h, &DARK, 1280, 720, k, &esc);
    assert!(h.ask_delete.is_none() && h.sel.len() == 2);
    render(&mut h, &DARK, 1280, 720, k, &inp);
    let (r, _, _, ok) = {
        let ui = layout_ui();
        let l = layout_of(&h, 1280.0, 720.0);
        draw::dialog_lay(&ui, &l, &h)
    };
    assert!(r.w > 0.0);
    let (ox, oy) = centre(ok);
    let (_, acts) = render(&mut h, &DARK, 1280, 720, k, &click_at(ox, oy, k, Mods::default()));
    assert_eq!(acts, [Act::Delete(two)]);
    // One deletes at once.
    assert_eq!(h.delete(vec!["x".into()]), [Act::Delete(vec!["x".into()])]);
    // The menu: Clear history… asks.
    let l = layout_of(&h, 1280.0, 720.0);
    let (mx, my) = centre(l.more);
    render(&mut h, &DARK, 1280, 720, k, &click_at(mx, my, k, Mods::default()));
    assert_eq!(h.menu, Some(Menu::More));
    let items = vec![(l.more, Target::More)];
    let (_, rows) = draw::menu_lay(&l, &h, &items).expect("menu");
    let (rx, ry) = centre(rows[0].0);
    render(&mut h, &DARK, 1280, 720, k, &click_at(rx, ry, k, Mods::default()));
    assert_eq!(h.clear, Some(false));
    // Enter confirms (saved files stay unless ticked).
    let mut inp = Input::default();
    inp.feed(&Ev::Key { vk: crate::wind::key::RETURN, up: false, repeat: false, mods: Mods::default() });
    let (_, acts) = render(&mut h, &DARK, 1280, 720, k, &inp);
    assert_eq!(acts, [Act::Clear(false)]);
}

#[test]
fn texts_of_the_footer_and_the_question() {
    let (_r, st) = fixture("texts");
    let h = history(&st);
    let (keep, used, count, f) = h.footer();
    assert_eq!(keep, "Keeping 30 days");
    assert!(used.ends_with(" of 2 GB"), "{used}");
    assert_eq!(count, "10 captures");
    assert!((0.0..=1.0).contains(&f));
    let (p, _) = h.clear_texts();
    assert_eq!(p, "Removes all 10 captures from History. This can't be undone.");
    assert_eq!(draw::hotkey_caps("Shift+Meta+X"), [crate::keymap::META_NAME, "Shift", "X"]);
}

/// Every board state, dark and light, at 100 % and 150 %.
#[test]
fn preview_history_pngs() {
    for k in [1.0f32, 1.5] {
        for (tn, th) in [("dark", DARK), ("light", LIGHT)] {
            let name = |what: &str| if k == 1.0 { format!("history-{what}-{tn}.png") } else { format!("history-{what}-{tn}-{}.png", (k * 100.0) as u32) };
            let (_r, st) = fixture(&format!("pv-{tn}-{k}"));
            // The board: 1280 x 720, the fourth card selected, the third hovered.
            let mut h = history(&st);
            h.sel = vec![h.entries[3].id.clone()];
            let l = layout_of(&h, 1280.0, 720.0);
            let (hx, hy) = centre(l.on_screen(l.items[2].thumb, 0.0));
            let (img, _) = render(&mut h, &th, 1280, 720, k, &at(hx, hy + 50.0, k));
            crate::ui::preview::save(&name("grid"), &img);
            // The default size, nothing selected.
            let mut h = history(&st);
            let (img, _) = render(&mut h, &th, W, H, k, &Input::default());
            crate::ui::preview::save(&name("default"), &img);
            // List view, a row hovered.
            let mut h = history(&st);
            h.view = View::List;
            let l = layout_of(&h, 896.0, 400.0);
            let (rx, ry) = centre(l.on_screen(l.items[1].r, 0.0));
            let (img, _) = render(&mut h, &th, 896, 400, k, &at(rx, ry, k));
            crate::ui::preview::save(&name("list"), &img);
            // Search.
            let mut h = history(&st);
            h.view = View::List;
            h.search.set("stripe");
            h.focus_search = true;
            let (img, _) = render(&mut h, &th, 896, 400, k, &Input::default());
            crate::ui::preview::save(&name("search"), &img);
            // Multi-select.
            let mut h = history(&st);
            h.sel = [0, 1, 3].iter().map(|i| h.entries[*i].id.clone()).collect();
            let (img, _) = render(&mut h, &th, 896, 400, k, &Input::default());
            crate::ui::preview::save(&name("multi"), &img);
            // Clear history?
            let mut h = history(&st);
            h.clear = Some(false);
            let (img, _) = render(&mut h, &th, 896, 400, k, &Input::default());
            crate::ui::preview::save(&name("clear"), &img);
            // A card's menu.
            let mut h = history(&st);
            h.menu = Some(Menu::Card(h.entries[0].id.clone()));
            let (img, _) = render(&mut h, &th, 960, 640, k, &Input::default());
            crate::ui::preview::save(&name("card-menu"), &img);
            // Empty, and off.
            let empty = TempRoot::new(&format!("pv-empty-{tn}-{k}"));
            let mut h = History::new(Store::new(&empty.0), Config::default(), TODAY);
            let (img, _) = render(&mut h, &th, 896, 400, k, &Input::default());
            crate::ui::preview::save(&name("empty"), &img);
            let mut h = History::new(Store::new(&empty.0), Config { history: false, ..Config::default() }, TODAY);
            let (img, _) = render(&mut h, &th, 896, 400, k, &Input::default());
            crate::ui::preview::save(&name("off"), &img);
            // The off state's switch turns it on.
            let l = layout_of(&h, 896.0, 400.0);
            let sw = draw::empty_lay(&layout_ui(), &l, &h);
            let (x, y) = centre(sw);
            let (_, acts) = render(&mut h, &th, 896, 400, k, &click_at(x, y, k, Mods::default()));
            assert_eq!(acts, [Act::TurnOn]);
        }
    }
}

/// History off with captures still kept: "10 captures still kept ·
/// Clear history…", whose link asks the usual question.
#[test]
fn the_off_state_offers_to_clear_what_is_kept() {
    let (_r, st) = fixture("offkept");
    let mut h = History::new(st.clone(), Config { history: false, ..Config::default() }, TODAY);
    let l = layout_of(&h, 960.0, 640.0);
    let (pre, _, _, link) = draw::off_kept_lay(&layout_ui(), &l, &h).expect("captures are kept");
    assert_eq!(pre, "10 captures still kept \u{b7} ");
    let (x, y) = centre(link);
    render(&mut h, &DARK, 960, 640, 1.0, &click_at(x, y, 1.0, Mods::default()));
    assert_eq!(h.clear, Some(false));
    // Nothing kept: no row.
    let empty = TempRoot::new("offempty");
    let h = History::new(Store::new(&empty.0), Config { history: false, ..Config::default() }, TODAY);
    assert!(draw::off_kept_lay(&layout_ui(), &layout_of(&h, 960.0, 640.0), &h).is_none());
}

/// The decoded thumbnails stay within [`THUMBS_KEPT`] (the least recently
/// used go) and an edited capture's is decoded again.
#[test]
fn thumbnails_are_bounded_and_follow_edits() {
    let (_r, st) = fixture("thumbs");
    let mut h = history(&st);
    let id = h.entries[0].id.clone();
    assert!(h.thumb_of(&id).is_some());
    // Full of others, used before it.
    for i in 0..THUMBS_KEPT - 1 {
        h.thumbs.insert(format!("fake-{i}"), Thumb { pic: None, used: 0, stamp: None });
    }
    let _ = h.thumb_of(&id); // just used
    let others: Vec<String> = h.entries[1..4].iter().map(|e| e.id.clone()).collect();
    for o in &others {
        assert!(h.thumb_of(o).is_some());
        assert_eq!(h.thumbs.len(), THUMBS_KEPT);
    }
    assert!(h.thumbs.contains_key(&id), "the recently used one stays");
    assert_eq!(h.thumbs.keys().filter(|k| k.starts_with("fake-")).count(), THUMBS_KEPT - 1 - 3, "the least recently used went");
    // An edit writes a new thumb.png: the cached one goes on the next listing.
    let e = st.get(&id).unwrap();
    let before = h.thumb_of(&id).unwrap().clone();
    let objs = vec![crate::objects::Obj::Rect { r: FRect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 }, color: crate::uifb::C4::rgb(240, 68, 56), width: 40.0 }];
    st.save_edit(&e.id, &objs, None).unwrap();
    h.reload();
    assert!(!h.thumbs.contains_key(&id), "dropped");
    assert_ne!(h.thumb_of(&id).unwrap().as_raw(), before.as_raw(), "the new one");
}

/// A frame's kit for measuring (layout helpers that need fonts).
fn layout_ui() -> crate::ui::Ui<'static> {
    let img: &'static mut Vec<u8> = Box::leak(Box::new(vec![0u8; 4]));
    let inp: &'static Input = Box::leak(Box::new(Input::default()));
    let focus: &'static mut FocusState = Box::leak(Box::new(FocusState::default()));
    let clip: &'static mut crate::ui::preview::FakeClip = Box::leak(Box::new(crate::ui::preview::FakeClip::default()));
    let th: &'static Theme = &DARK;
    crate::ui::Ui::new(crate::uifb::Fb::new(img, 1), th, inp, focus, clip, 1.0, FRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 })
}

/// Smoke (needs a display): the real History window opens on the
/// fixture, draws and closes after about a second.
/// `cargo test history_window_smoke -- --ignored --test-threads=1`
#[cfg(not(target_os = "macos"))]
#[test]
#[ignore = "opens a window"]
fn history_window_smoke() {
    if !crate::wind::has_display() {
        eprintln!("no display; skipped");
        return;
    }
    let _guard = crate::wind::test_window_lock();
    let (_r, st) = fixture("smoke");
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut w = window::Window::new(history(&st), rx);
    let mut d = crate::wind::AutoClose::new(&mut w, 1000);
    let spec = crate::wind::WindowSpec { title: "Rustshot History".into(), w: W, h: H, resizable: true, min: MIN };
    crate::wind::run_window(spec, &mut d).expect("history window");
    assert!(d.frames > 0, "drew a frame");
}
