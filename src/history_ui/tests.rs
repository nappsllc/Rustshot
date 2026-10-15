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
            monitor: "Built-in Display".into(),
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
                    crate::objects::Obj::Callout {
                        fmt: Default::default(),
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

/// Fail closed: a capture whose annotations.json is there but cannot be
/// read (broken, or from a newer build) is never copied, saved, pinned,
/// opened or put in a folder as its bare original; with readable
/// annotations and its saved file gone, Open uses the annotated picture.
#[test]
fn unreadable_annotations_never_fall_back_to_the_original() {
    use crate::export::Task;
    let (root, st) = fixture("hist-failclosed");
    let e = st.list().into_iter().find(|e| e.meta.edited && !e.meta.kind.is_recording()).expect("an edited shot");
    // Readable annotations, saved file gone: Open bakes them.
    let f = file_of(&st, &e).expect("a file");
    assert_eq!(f.file_name().unwrap(), "picture.png");
    let want = picture(&st, &e).unwrap();
    assert!(PixBuf::from_png(&std::fs::read(&f).unwrap()).unwrap() == want);
    assert!(want != st.original(&e).unwrap(), "the arrow is in it");
    let cfg = Config { save_path: root.0.join("out").display().to_string(), ..Config::default() };
    for bad in ["{\"version\":3,\"objects\":[]}", "{\"version\":1,\"objects\":[{\"type\":\"pixelate\"", ""] {
        std::fs::write(e.annotations(), bad).unwrap();
        assert!(picture(&st, &e).is_err(), "{bad:?}");
        assert!(file_of(&st, &e).is_err(), "{bad:?}: no file to open");
        for task in [Task::Copy, Task::Save { path: None, ask: false }, Task::Pin] {
            let d = run_export(&st, &e.id, task.clone(), &cfg);
            assert_eq!(d.kind, crate::editor::style::ToastKind::Error, "{bad:?} {task:?}: {}", d.text);
            assert!(d.text.contains("annotations"), "{}", d.text);
        }
        let dir = root.0.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        let d = save_to_folder(&st, std::slice::from_ref(&e.id), &dir);
        assert_eq!(d.kind, crate::editor::style::ToastKind::Error, "{}", d.text);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing written");
    }
    // No annotations file at all: the original is the picture.
    let plain = st.list().into_iter().find(|e| !e.meta.edited && !e.meta.kind.is_recording()).unwrap();
    let _ = std::fs::remove_file(plain.annotations());
    assert!(picture(&st, &plain).unwrap() == st.original(&plain).unwrap());
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

/// Give the board's uploaded capture a kept deletehash (its record).
fn with_record(st: &Store) -> String {
    let e = st.list().into_iter().find(|e| e.meta.uploaded.is_some()).expect("an uploaded capture");
    st.update(&e.id, |m| {
        m.upload = Some(crate::uploads::Info { deletehash: crate::uploads::Secret::new("dhKq2"), time: 1, kind: crate::uploads::UpKind::Image, thumb: None, s3: None })
    })
    .unwrap();
    e.id
}

/// "Uploaded" keeps the captures with a link; their cards' ⋯ menu adds
/// Copy link, and Delete upload… once the deletehash is kept.
#[test]
fn uploaded_filter_and_card_menu() {
    let (_r, st) = fixture("uploaded");
    let mut h = history(&st);
    h.filter = Filter::Uploaded;
    let shown: Vec<String> = h.shown().iter().map(|(i, _)| history::title(&h.entries[*i].meta)).collect();
    assert_eq!(shown, ["Slack \u{2014} #design-review"]);
    assert_eq!(h.groups().len(), 1);
    assert_eq!((draw::FILTERS[3].0, draw::FILTERS[3].1.get()), (Filter::Uploaded, "Uploaded"));
    let up = h.entries.iter().find(|e| e.meta.uploaded.is_some()).unwrap().clone();
    let plain = h.entries.iter().find(|e| e.meta.uploaded.is_none() && !e.meta.kind.is_recording()).unwrap().clone();
    let rec = h.entries.iter().find(|e| e.meta.kind.is_recording()).unwrap().clone();
    use draw::MenuAct as M;
    // Share after Open with where the OS has a share sheet (not Linux).
    let acts = |e: &crate::history::Entry| -> Vec<M> { draw::card_acts(&h, e).into_iter().filter(|a| *a != M::Share).collect() };
    for e in [&plain, &rec, &up] {
        let all = draw::card_acts(&h, e);
        let ow = all.iter().position(|a| *a == M::OpenWith).unwrap();
        assert_eq!(all.get(ow + 1) == Some(&M::Share), crate::share::supported(), "{all:?}");
    }
    assert_eq!(acts(&plain), [M::SaveAs, M::OpenWith, M::ShowInFolder, M::CopyPath, M::Delete]);
    assert_eq!(acts(&rec), [M::OpenWith, M::ShowInFolder, M::CopyPath, M::Delete]);
    // Uploaded before records were kept: no deletehash, no delete.
    assert_eq!(acts(&up), [M::SaveAs, M::OpenWith, M::ShowInFolder, M::CopyPath, M::CopyLink, M::Delete]);
    with_record(&st);
    h.reload();
    let up = h.entry(&up.id).unwrap().clone();
    let acts = |e: &crate::history::Entry| -> Vec<M> { draw::card_acts(&h, e).into_iter().filter(|a| *a != M::Share).collect() };
    assert_eq!(acts(&up), [M::SaveAs, M::OpenWith, M::ShowInFolder, M::CopyPath, M::CopyLink, M::DeleteUpload, M::Delete]);
    assert_eq!(M::Share.label(), "Share\u{2026}");
    assert_eq!(M::DeleteUpload.label(), "Delete upload\u{2026}");
    assert_eq!(M::OpenWith.label(), "Open with\u{2026}");
}

/// Delete upload… asks first; its button deletes (a fake Imgur):
/// the link and record go and the toast says so; a failure says why and
/// keeps them.
#[test]
fn deleting_an_upload_asks_then_deletes() {
    let (_r, st) = fixture("unupload");
    let id = with_record(&st);
    let mut h = history(&st);
    h.ask_unupload = Some(id.clone());
    assert!(h.asking());
    let (title, para, check, ok) = h.dialog_texts();
    assert_eq!((title.as_str(), check, ok), ("Delete upload?", None, "Delete upload"));
    assert!(para.starts_with("rs.sh/a8Kq2 stops working"), "{para}");
    assert_eq!(h.dialog_ok(), [Act::DeleteUpload(id.clone())]);
    assert!(!h.asking());
    h.ask_unupload = Some(id.clone());
    assert!(h.escape() && h.ask_unupload.is_none(), "Esc answers no");

    let no_s3 = |_: &crate::s3::Object| -> Result<(), String> { panic!("an Imgur upload") };
    let refused = |_: &str, _: &str, _: &[(&str, &str)]| Ok((403, r#"{"data":{"error":"Permission denied"},"success":false}"#.to_string()));
    let d = delete_upload(&st, &id, "cid", &refused, &no_s3);
    assert_eq!((d.text.as_str(), d.kind), ("Couldn't delete from Imgur: Permission denied", crate::editor::style::ToastKind::Error));
    assert!(st.get(&id).unwrap().meta.uploaded.is_some(), "kept after a failure");

    let asked = std::cell::RefCell::new(String::new());
    let ok_http = |h: &str, p: &str, _: &[(&str, &str)]| {
        *asked.borrow_mut() = format!("{h}{p}");
        Ok((200, r#"{"data":true,"success":true,"status":200}"#.to_string()))
    };
    let d = delete_upload(&st, &id, "cid", &ok_http, &no_s3);
    assert_eq!((d.text.as_str(), d.changed), ("Deleted from Imgur", true));
    assert_eq!(*asked.borrow(), "api.imgur.com/3/image/dhKq2");
    let m = st.get(&id).unwrap().meta;
    assert_eq!((m.uploaded, m.upload), (None, None));
    assert!(delete_upload(&st, &id, "cid", &ok_http, &no_s3).text.contains("no upload"), "nothing left to delete");
}

/// The header still fits at the smallest window with the fourth filter.
#[test]
fn the_header_fits_at_the_minimum_width() {
    let (_r, st) = fixture("header");
    let mut h = history(&st);
    h.sel = vec![h.entries[0].id.clone()];
    for w in [MIN.0 as f32, 900.0, 960.0] {
        let l = layout_of(&h, w, 600.0);
        let last = l.views.last().map_or(l.filters.last().unwrap().0, |v| v.0);
        let right = l.panel_btn.unwrap_or(l.more);
        assert!(last.x1() + 6.0 <= right.x, "at {w}: {last:?} runs into {right:?}");
        assert!(l.search.unwrap().x1() <= l.filters[0].0.x, "at {w}");
    }
}

/// macOS (board 12): the content runs under the transparent title bar,
/// so the header row starts at x 84, clear of the traffic lights' 72 x
/// 28 zone, and still fits at the smallest window; a press in the top
/// 28 px drags the window except on the header's controls. Right to
/// left the zone is the layout's trailing end (the frame is mirrored,
/// the lights stay at the left). Writes `history-macos-*.png`.
#[test]
fn the_header_clears_the_macos_traffic_lights() {
    let (_r, st) = fixture("mac-header");
    let zone = FRect { x: 0.0, y: 0.0, w: 72.0, h: 28.0 };
    let meets = |a: FRect, b: FRect| a.x < b.x1() && b.x < a.x1() && a.y < b.y1() && b.y < a.y1();
    let (mw, mh) = min_size(crate::wind::Overlap::MACOS);
    assert_eq!((mw, mh), (MIN.0 + 66, MIN.1), "the smallest window keeps the row's room");
    assert_eq!(min_size(crate::wind::Overlap::NONE), MIN);
    for lang in crate::ui::preview::langs() {
        crate::i18n::with_lang(lang, || {
            let rtl = crate::i18n::is_rtl();
            let mut h = history(&st);
            h.chrome = crate::wind::Overlap::MACOS;
            h.sel = vec![h.entries[0].id.clone()];
            // Nothing cut short or overlapping at the smallest window.
            crate::ui::preview::begin_check(mw);
            let mut focus = FocusState::default();
            render_with(&mut h, &DARK, mw, mh, 1.0, &Input::default(), &mut focus);
            crate::ui::preview::assert_fits(&focus, mw, mh, 1.0, &format!("history macos {lang}"));
            for w in [mw as f32, W as f32] {
                let l = layout_of(&h, w, H as f32);
                // On screen: the layout mirrored right to left.
                let screen = |r: FRect| if rtl { FRect { x: w - r.x1(), ..r } } else { r };
                let mut parts: Vec<FRect> = l.filters.iter().map(|f| f.0).chain(l.views.iter().map(|v| v.0)).collect();
                parts.extend([Some(l.more), l.search, l.panel_btn, l.slider].into_iter().flatten());
                for r in &parts {
                    assert!(!meets(screen(*r), zone), "{lang} at {w}: {r:?} under the traffic lights");
                }
                if !rtl {
                    assert_eq!(l.title_x, 84.0, "the row starts 12 past the zone");
                    assert!(l.search.unwrap().x >= 84.0);
                } else {
                    assert_eq!(l.title_x, 18.0);
                    assert!(w - l.more.x1() >= 84.0, "{lang} at {w}: {:?}", l.more);
                }
                let last = l.views.last().map_or(l.filters.last().unwrap().0, |v| v.0);
                let right = l.panel_btn.unwrap_or(l.more);
                assert!(last.x1() + 6.0 <= right.x, "{lang} at {w}: {last:?} runs into {right:?}");
            }
            // The drag strip: the empty header drags, its controls do not.
            let (mut img, _) = render(&mut h, &DARK, W, H, 1.0, &Input::default());
            let l = layout_of(&h, W as f32, H as f32);
            let (mx, my) = centre(l.more);
            assert!(!h.drags_window_at(mx, my), "⋯ is ours");
            let s = l.search.unwrap();
            assert!(!h.drags_window_at(s.x + 4.0, s.y + 2.0), "the search field is ours");
            if lang == "en" {
                let gap = l.views.last().unwrap().0.x1() + 4.0;
                assert!(h.drags_window_at(gap, 6.0), "the header's empty stretch drags");
                crate::ui::preview::traffic_lights(&mut img, 1.0);
                crate::ui::preview::save("history-macos-dark.png", &img);
                let (mut img, _) = render(&mut h, &LIGHT, W, H, 1.0, &Input::default());
                crate::ui::preview::traffic_lights(&mut img, 1.0);
                crate::ui::preview::save("history-macos-light.png", &img);
            }
        });
    }
}

/// Open with… opens the saved file of an unedited capture, else a
/// temporary picture (no chooser in tests).
#[test]
fn open_with_picks_the_file() {
    let (_r, st) = fixture("openwith");
    let dir = std::env::temp_dir().join(format!("rustshot-hist-ow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let saved = dir.join("kept.png");
    std::fs::write(&saved, b"png").unwrap();
    let e = st.list().into_iter().find(|e| !e.meta.edited && !e.meta.kind.is_recording()).unwrap();
    st.update(&e.id, |m| m.saved = Some(saved.clone())).unwrap();
    crate::open_with::OPENED.with(|o| o.borrow_mut().clear());
    assert_eq!(open_with(&st, &e.id, &Config::default(), None).kind, crate::editor::style::ToastKind::Success);
    let edited = st.list().into_iter().find(|e| e.meta.edited).unwrap();
    open_with(&st, &edited.id, &Config::default(), None);
    let opened = crate::open_with::OPENED.with(|o| o.borrow().clone());
    assert_eq!(opened[0], saved);
    assert!(opened[1].starts_with(crate::open_with::temp_dir()), "an edited one as it looks now");
    let _ = std::fs::remove_file(&opened[1]);
    let _ = std::fs::remove_dir_all(&dir);
    // Ctrl+O asks for it.
    let mut h = history(&st);
    h.sel = vec![e.id.clone()];
    let mut inp = Input::default();
    inp.feed(&Ev::Key { vk: 'O' as u32, up: false, repeat: false, mods: Mods { ctrl: true, ..Mods::default() } });
    let (_, acts) = render(&mut h, &DARK, 960, 640, 1.0, &inp);
    assert_eq!(acts, [Act::OpenWith(e.id)]);
}

/// Share… hands the same file as Open with to the sheet (none comes up
/// in tests): the saved file of an unedited capture, a recording's file,
/// else a temporary picture; Ctrl+Shift+E asks for it where there is a
/// sheet.
#[test]
fn share_hands_the_file() {
    let (_r, st) = fixture("share");
    let dir = std::env::temp_dir().join(format!("rustshot-hist-share-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let saved = dir.join("kept.png");
    std::fs::write(&saved, b"png").unwrap();
    let e = st.list().into_iter().find(|e| !e.meta.edited && !e.meta.kind.is_recording()).unwrap();
    st.update(&e.id, |m| m.saved = Some(saved.clone())).unwrap();
    crate::share::SHARED.with(|s| s.borrow_mut().clear());
    let edited = st.list().into_iter().find(|e| e.meta.edited).unwrap();
    let ok = crate::editor::style::ToastKind::Success;
    if crate::share::supported() {
        assert_eq!(share(&st, &e.id, &Config::default(), None).kind, ok);
        assert_eq!(share(&st, &edited.id, &Config::default(), None).kind, ok);
        let shared = crate::share::SHARED.with(|s| s.borrow().clone());
        assert_eq!(shared[0], saved);
        assert!(shared[1].starts_with(crate::open_with::temp_dir()) && shared[1].extension().unwrap() == "png");
        let _ = std::fs::remove_file(&shared[1]);
    } else {
        assert_ne!(share(&st, &e.id, &Config::default(), None).kind, ok, "no sheet here");
    }
    assert_ne!(share(&st, "missing", &Config::default(), None).kind, ok);
    let _ = std::fs::remove_dir_all(&dir);
    let mut h = history(&st);
    h.sel = vec![e.id.clone()];
    let mut inp = Input::default();
    inp.feed(&Ev::Key { vk: 'E' as u32, up: false, repeat: false, mods: Mods { ctrl: true, shift: true, ..Mods::default() } });
    let (_, acts) = render(&mut h, &DARK, 960, 640, 1.0, &inp);
    if crate::share::supported() {
        assert_eq!(acts, [Act::Share(e.id)]);
    } else {
        assert!(acts.is_empty(), "{acts:?}");
    }
}

#[test]
fn preview_history_upload_pngs() {
    for (tn, th) in [("dark", DARK), ("light", LIGHT)] {
        let (_r, st) = fixture(&format!("pv-up-{tn}"));
        let id = with_record(&st);
        // The Uploaded filter, the card's badge and its menu.
        let mut h = history(&st);
        h.filter = Filter::Uploaded;
        h.menu = Some(Menu::Card(id.clone()));
        let (img, _) = render(&mut h, &th, 960, 640, 1.0, &Input::default());
        crate::ui::preview::save(&format!("history-uploaded-menu-{tn}.png"), &img);
        // All, with the badge among the others; the list's cloud.
        let mut h = history(&st);
        let (img, _) = render(&mut h, &th, 960, 640, 1.0, &Input::default());
        crate::ui::preview::save(&format!("history-uploaded-badge-{tn}.png"), &img);
        h.view = View::List;
        let (img, _) = render(&mut h, &th, 960, 640, 1.0, &Input::default());
        crate::ui::preview::save(&format!("history-uploaded-list-{tn}.png"), &img);
        // The question.
        let mut h = history(&st);
        h.ask_unupload = Some(id);
        let (img, _) = render(&mut h, &th, 960, 640, 1.0, &Input::default());
        crate::ui::preview::save(&format!("history-unupload-{tn}.png"), &img);
        // The smallest window.
        let mut h = history(&st);
        let (img, _) = render(&mut h, &th, MIN.0, MIN.1, 1.0, &Input::default());
        crate::ui::preview::save(&format!("history-min-{tn}.png"), &img);
    }
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
    let (_tx, rx) = crate::chan::channel();
    let mut w = window::Window::new(history(&st), rx);
    let mut d = crate::wind::AutoClose::new(&mut w, 1000);
    let spec = crate::wind::WindowSpec { title: "Rustshot History".into(), w: W, h: H, resizable: true, min: MIN, controls: crate::wind::Controls::MinMaxClose };
    crate::wind::run_window(spec, &mut d).expect("history window");
    assert!(d.frames > 0, "drew a frame");
}

/// The window at its smallest fits in English and the long
/// pseudo-languages: the grid, the list, a search, several selected,
/// Clear history?, a card's menu, empty and off (previews per language).
#[test]
fn fits_in_long_languages() {
    let (w, hh) = MIN;
    for lang in crate::ui::preview::langs() {
        crate::i18n::with_lang(lang, || {
            for k in [1.0f32, 1.5] {
                let (_r, st) = fixture(&format!("fit-{lang}-{k}"));
                let empty = TempRoot::new(&format!("fit-empty-{lang}-{k}"));
                let states: Vec<(&str, History)> = vec![
                    ("grid", history(&st)),
                    ("list", {
                        let mut h = history(&st);
                        h.view = View::List;
                        h
                    }),
                    ("search", {
                        let mut h = history(&st);
                        h.search.set("stripe");
                        h.focus_search = true;
                        h
                    }),
                    ("multi", {
                        let mut h = history(&st);
                        h.sel = [0, 1, 3].iter().map(|i| h.entries[*i].id.clone()).collect();
                        h
                    }),
                    ("clear", {
                        let mut h = history(&st);
                        h.clear = Some(false);
                        h
                    }),
                    ("card-menu", {
                        let mut h = history(&st);
                        h.menu = Some(Menu::Card(h.entries[0].id.clone()));
                        h
                    }),
                    ("empty", History::new(Store::new(&empty.0), Config::default(), TODAY)),
                    ("off", History::new(Store::new(&empty.0), Config { history: false, ..Config::default() }, TODAY)),
                ];
                for (name, mut h) in states {
                    crate::ui::preview::begin_check((w as f32 * k).round() as u32);
                    let mut focus = FocusState::default();
                    let (img, _) = render_with(&mut h, &DARK, w, hh, k, &Input::default(), &mut focus);
                    // (Saved first: a failure has its picture.)
                    if k == 1.0 {
                        crate::ui::preview::save(&format!("history-{name}-{lang}.png"), &img);
                    }
                    crate::ui::preview::assert_fits(&focus, w, hh, k, &format!("history {name} {lang} {k}x"));
                }
            }
        });
    }
}

/// The details panel's footer: Edit and Open file share the row 8 px
/// apart and 8 px before Delete (as on the board), and their labels fit
/// in every fit language; the window with the panel open fits.
#[test]
fn details_panel_fits_in_long_languages() {
    for lang in crate::ui::preview::langs() {
        crate::i18n::with_lang(lang, || {
            let (_r, st) = fixture(&format!("panel-fit-{lang}"));
            let mut h = history(&st);
            h.sel = vec![h.entries[3].id.clone()];
            let p = layout_of(&h, 1280.0, 720.0).panel.expect("the details panel");
            for rec in [false, true] {
                let pl = draw::panel_lay(p, rec);
                assert!(pl.open.x - pl.edit.x1() >= 8.0 && pl.delete.x - pl.open.x1() >= 8.0, "{lang}: {:?} {:?} {:?}", pl.edit, pl.open, pl.delete);
                let (le, lo) = draw::panel_labels(rec);
                for (r, label) in [(pl.edit, le), (pl.open, lo)] {
                    let w = crate::uifb::text_width(&crate::fonts::UI, 12.0 * crate::ui::compact::CSS, label);
                    assert!(w + 20.0 + 8.0 <= r.w, "{lang}: {label:?} in {:.0}", r.w);
                }
            }
            crate::ui::preview::begin_check(1280);
            let mut focus = FocusState::default();
            render_with(&mut h, &DARK, 1280, 720, 1.0, &Input::default(), &mut focus);
            crate::ui::preview::assert_fits(&focus, 1280, 720, 1.0, &format!("history details {lang}"));
        });
    }
}

/// The Microsoft Store screenshot (docs/store/screenshots): the grid at
/// 125 %, a capture selected and another hovered, with Windows paths;
/// in each of [`crate::ui::preview::store_langs`] (`<lang>/`).
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn store_screenshot_history() {
    let k = 1.25;
    for lang in crate::ui::preview::store_langs() {
        crate::i18n::with_lang(lang, || {
            let (_r, st) = fixture(&format!("store-{lang}"));
            for e in st.list() {
                st.update(&e.id, |m| {
                    m.monitor = "Display 1".into();
                    m.saved = m.saved.as_ref().and_then(|p| p.file_name()).map(|n| PathBuf::from(format!("C:\\Users\\you\\Pictures\\rustshot\\{}", n.to_string_lossy())));
                    m.file = m.file.as_ref().map(|_| PathBuf::from("C:\\Users\\you\\Videos\\rustshot\\rec.mp4"));
                })
                .unwrap();
            }
            let mut h = history(&st);
            h.sel = vec![h.entries[3].id.clone()];
            let l = layout_of(&h, 1280.0, 720.0);
            let (hx, hy) = centre(l.on_screen(l.items[2].thumb, 0.0));
            let (img, _) = render(&mut h, &DARK, 1280, 720, k, &at(hx, hy + 50.0, k));
            crate::ui::preview::save_lang(lang, "store-history-dark.png", &img);
        });
    }
}

/// The Monitor row: the kept name with its scale, an unknown monitor
/// (nothing kept) named in the language shown, a dash with nothing known.
#[test]
fn monitor_row_names_an_unknown_monitor_when_shown() {
    let m = |monitor: &str, scale: f32| Meta { monitor: monitor.into(), scale, ..Meta::default() };
    assert_eq!(draw::monitor_row(&m("DELL U2720Q", 1.5)), "DELL U2720Q (1.5\u{d7})");
    assert_eq!(draw::monitor_row(&m("", 0.0)), "\u{2014}");
    assert_eq!(draw::monitor_row(&m("", 1.0)), "Display");
    for lang in crate::ui::preview::langs() {
        crate::i18n::with_lang(lang, || {
            assert_eq!(draw::monitor_row(&m("", 2.0)), format!("{} (2\u{d7})", tr!("history.display")), "{lang}");
        });
    }
}
