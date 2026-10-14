//! The ruler (U) and Read QR code (Q) in the overlay, driven the way the
//! window drives the editor; plus their preview PNGs.

use super::tests::{edit_of, preview_app, settled_frame};
use super::*;

fn ed(a: &mut App) -> &mut Edit {
    edit_of(a)
}

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };
const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };
const BIG: FRect = FRect { x: 100.0, y: 80.0, w: 1240.0, h: 700.0 };
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;

fn app() -> App {
    let mut a = preview_app(theme::DARK, Some(BIG));
    a.focus_tries = u8::MAX;
    a
}

fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

struct HeldMods;
fn hold(m: Mods) -> HeldMods {
    FAKE_MODS.with(|c| c.set(Some(m)));
    HeldMods
}
impl Drop for HeldMods {
    fn drop(&mut self) {
        FAKE_MODS.with(|c| c.set(Some(Mods::default())));
    }
}

fn drag(a: &mut App, from: (i32, i32), to: (i32, i32)) {
    a.on_event(Ev::Move { x: from.0, y: from.1 });
    a.on_event(Ev::Down { x: from.0, y: from.1 });
    a.on_event(Ev::Move { x: (from.0 + to.0) / 2, y: (from.1 + to.1) / 2 });
    a.on_event(Ev::Move { x: to.0, y: to.1 });
    a.on_event(Ev::Up { x: to.0, y: to.1 });
}

#[test]
fn u_picks_the_ruler_and_a_drag_measures() {
    let mut a = app();
    key_m(&mut a, 'U' as u32, NONE);
    assert_eq!(ed(&mut a).tool, Some(Tool::Ruler));
    let n0 = ed(&mut a).objects.len();
    drag(&mut a, (300, 300), (600, 400));
    let r = ed(&mut a).ruler.expect("a measurement");
    assert!(r.done && !r.area);
    assert_eq!((r.a, r.b), (Pt::new(300.0, 300.0), Pt::new(600.0, 400.0)));
    assert_eq!(ed(&mut a).objects.len(), n0, "nothing drawn yet");
    // its label is in the scene; the magnifier shows with the ruler
    let _ = settled_frame(&mut a);
    let sc = ed(&mut a).scene.clone().unwrap();
    let u = sc.ruler.as_ref().expect("ruler in the scene");
    assert!(u.lines[0].starts_with("316.2 px"), "{:?}", u.lines);
    assert!(u.hint);
    assert!(ed(&mut a).mag_on() || !ed(&mut a).cfg.magnifier);
    // Enter keeps it: a dimension line (two arrows) and its label
    key_m(&mut a, VK_RETURN, NONE);
    let objs = ed(&mut a).objects[n0..].to_vec();
    assert_eq!(objs.len(), 3, "{objs:?}");
    assert!(matches!(objs[0], Obj::Arrow { .. }) && matches!(objs[1], Obj::Arrow { .. }));
    assert!(matches!(&objs[2], Obj::Text { text, .. } if text == "316.2 px"));
    assert!(ed(&mut a).ruler.is_none() && !ed(&mut a).done, "the capture is not accepted");
    // one undo step takes all three away
    a.on_event(Ev::Key { vk: 'Z' as u32, up: false, repeat: false, mods: CTRL });
    assert_eq!(ed(&mut a).objects.len(), n0);
}

#[test]
fn esc_discards_the_measurement_and_keeps_the_tool() {
    let mut a = app();
    key_m(&mut a, 'U' as u32, NONE);
    drag(&mut a, (300, 300), (400, 300));
    assert!(ed(&mut a).ruler.is_some());
    key_m(&mut a, VK_ESCAPE, NONE);
    assert!(ed(&mut a).ruler.is_none());
    assert_eq!(ed(&mut a).tool, Some(Tool::Ruler));
    assert!(!ed(&mut a).cancelled, "the capture stays");
    // another tool drops a measurement too
    drag(&mut a, (300, 300), (400, 300));
    key_m(&mut a, 'L' as u32, NONE);
    let _ = settled_frame(&mut a);
    assert!(ed(&mut a).ruler.is_none());
}

#[test]
fn shift_snaps_and_ctrl_measures_a_rect() {
    let mut a = app();
    key_m(&mut a, 'U' as u32, NONE);
    {
        let _g = hold(SHIFT);
        drag(&mut a, (300, 300), (500, 312));
    }
    let r = ed(&mut a).ruler.unwrap();
    assert!((r.b.y - 300.0).abs() < 0.01, "snapped level: {:?}", r.b);
    {
        let _g = hold(CTRL);
        drag(&mut a, (300, 300), (500, 400));
    }
    let r = ed(&mut a).ruler.unwrap();
    assert!(r.area);
    let _ = settled_frame(&mut a);
    let sc = ed(&mut a).scene.clone().unwrap();
    assert_eq!(sc.ruler.as_ref().unwrap().lines[0], "200 \u{d7} 100 px");
    key_m(&mut a, VK_RETURN, NONE);
    assert!(ed(&mut a).objects.iter().any(|o| matches!(o, Obj::Rect { r, .. } if r.w == 200.0 && r.h == 100.0)));
}

#[test]
fn q_reads_qr_codes_without_saving() {
    let mut a = app();
    let e = ed(&mut a);
    e.read_qr();
    assert!(e.done && !e.cancelled);
    assert_eq!(e.tasks, vec![Task::Qr]);
    // Q runs it: the overlay closes (no QR code in the test picture)
    let mut a = app();
    key_m(&mut a, 'Q' as u32, NONE);
    assert!(!matches!(a.st, State::Edit(_)));
    // The Copy text menu lists it.
    assert!(toolbar::COPY_MENU.iter().any(|(act, _)| *act == Act::ReadQr));
}

#[test]
fn the_ruler_has_an_icon_and_tip() {
    // A key only (the bar keeps the boards' width), like Invert.
    assert!(!toolbar::TOOL_ORDER.contains(&Tool::Ruler) && tools::ALL.contains(&Tool::Ruler));
    assert_eq!(chrome::act_icon(Act::Tool(Tool::Ruler)), "ruler");
    assert_eq!(chrome::act_icon(Act::ReadQr), "qr");
}

#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_ruler_previews() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let save = |img: &PixBuf, name: &str| std::fs::write(dir.join(name), img.to_png().unwrap()).unwrap();
    for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
        let mut a = preview_app(th, Some(BIG));
        a.focus_tries = u8::MAX;
        key_m(&mut a, 'U' as u32, NONE);
        drag(&mut a, (300, 300), (700, 460));
        save(&settled_frame(&mut a), &format!("{tn}-ruler-line.png"));
        {
            let _g = hold(CTRL);
            drag(&mut a, (400, 200), (800, 500));
        }
        save(&settled_frame(&mut a), &format!("{tn}-ruler-area.png"));
        key_m(&mut a, VK_RETURN, NONE);
        drag(&mut a, (300, 600), (900, 600));
        key_m(&mut a, VK_RETURN, NONE);
        save(&settled_frame(&mut a), &format!("{tn}-ruler-kept.png"));
    }
}
