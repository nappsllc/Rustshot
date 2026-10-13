//! The drawing-tools features: the size readout's preview, per-tool size
//! and color memory (restored by the next capture, written to the config
//! after a pause or when the overlay closes), the color picker (hue ring,
//! square, hex field, opacity, eyedropper, recent colors) and the camera
//! ring's controls; plus their preview PNGs.

use super::tests::{annotated, assert_dirty_covers, edit_of, preview_app, preview_app_with, settled, settled_frame, snap_all, synthetic_shot};
use super::*;
use pick::{Part, Target};

fn ed(a: &mut App) -> &mut Edit {
    edit_of(a)
}

fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

/// Run `act` as a toolbar press would (without the point).
fn act(a: &mut App, act: Act) {
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { panic!("no editor") };
    let mut e = *e;
    a.apply_act(&mut e, act);
    a.st = State::Edit(Box::new(e));
}

/// The rect of the first toolbar item `f` matches (laid out first).
fn item(a: &mut App, f: impl Fn(&toolbar::Kind) -> bool) -> FRect {
    let m = a.mouse;
    ed(a).prepare(None, m, Instant::now());
    let tb = ed(a).toolbar.as_ref().expect("toolbar");
    tb.items.iter().find(|i| f(&i.kind)).map(|i| i.r).unwrap_or_else(|| panic!("no such item"))
}

fn mid(r: FRect) -> (i32, i32) {
    ((r.x + r.w / 2.0) as i32, (r.y + r.h / 2.0) as i32)
}

fn down(a: &mut App, (x, y): (i32, i32)) {
    a.on_event(Ev::Down { x, y });
}

fn up(a: &mut App, (x, y): (i32, i32)) {
    a.on_event(Ev::Up { x, y });
}

fn mv(a: &mut App, (x, y): (i32, i32)) {
    a.on_event(Ev::Move { x, y });
}

fn click(a: &mut App, p: (i32, i32)) {
    down(a, p);
    up(a, p);
}

fn key(a: &mut App, vk: u32) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods: Mods::default() });
}

fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

fn type_str(a: &mut App, s: &str) {
    for u in s.encode_utf16() {
        a.on_event(Ev::Char(u));
    }
}

const BIG: FRect = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };
/// A pixel of the blue block in the synthetic shot (inside `BIG`).
const BLUE_AT: (i32, i32) = (520, 600);
const BLUE: C4 = C4::rgb(92, 140, 230);

fn app_big() -> App {
    let mut a = preview_app(theme::DARK, Some(BIG));
    a.focus_tries = u8::MAX;
    a
}

/// A scratch config file (removed on drop).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str, text: &str) -> Scratch {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("rustshot-tools-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("config.toml");
        std::fs::write(&p, text).unwrap();
        Scratch(p)
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.0).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(d) = self.0.parent() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

// ---- translucent objects --------------------------------------------------

/// A 50 % black arrow through the export: around the head's base every
/// covered pixel is the capture at half brightness, blended once (a
/// double blend would leave a darker semicircle where the shaft's cap
/// sits under the head).
#[test]
fn translucent_arrow_through_the_export() {
    let mut a = app_big();
    let plain = ed(&mut a).export(BIG);
    let e = ed(&mut a);
    e.objects.push(Obj::Arrow { a: Pt::new(300.0, 400.0), b: Pt::new(700.0, 400.0), color: C4::new(0, 0, 0, 128), width: 8.0 });
    e.dirty = true;
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.finish(&mut e, false);
    let State::Finish(job) = &a.st else { panic!("finish job") };
    // Image (676, 400) is the base (head length 24); the crop starts at BIG.
    let (ox, oy) = (BIG.x as u32, BIG.y as u32);
    for y in 397..403u32 {
        for x in 668..684u32 {
            let (got, under) = (job.img.get_pixel(x - ox, y - oy), plain.get_pixel(x - ox, y - oy));
            for c in 0..3 {
                let want = (under[c] as f32 * (1.0 - 128.0 / 255.0)).round() as i32;
                assert!((got[c] as i32 - want).abs() <= 1, "({x}, {y}) channel {c}: {} over {}, want {want}", got[c], under[c]);
            }
        }
    }
}

// ---- per-tool memory ---------------------------------------------------------

/// Each tool keeps its own size and color; the next capture starts from
/// them (and from the defaults for tools never changed).
#[test]
fn tools_remember_size_and_color_into_the_next_capture() {
    let mut a = app_big();
    a.kind = RunKind::Daemon;
    let green = C4::rgb(0, 200, 80);
    key(&mut a, 'L' as u32); // Line
    act(&mut a, Act::Size(1));
    act(&mut a, Act::Size(1));
    act(&mut a, Act::Color(green));
    key(&mut a, 'R' as u32); // Rectangle: its own
    assert_eq!((ed(&mut a).mem.size(Tool::Rect), ed(&mut a).color()), (3.0, C4::rgb(240, 68, 56)));
    act(&mut a, Act::Size(-1));
    // The marker is translucent on top of a translucent color.
    key(&mut a, 'M' as u32);
    act(&mut a, Act::Color(C4::new(250, 220, 0, 128)));
    assert_eq!(super::marker_color(ed(&mut a).color()).a, 45);
    // Pixelate: a size, no color (its palette button is off).
    key(&mut a, 'X' as u32);
    act(&mut a, Act::Size(1));
    let before = ed(&mut a).mem.clone();
    act(&mut a, Act::Color(green));
    assert_eq!(ed(&mut a).mem, before, "no color for pixelate");
    // Blur: a size of its own, no color either.
    key(&mut a, 'B' as u32);
    assert_eq!(ed(&mut a).mem.size(Tool::Blur), super::tools::BLUR_SIZE);
    act(&mut a, Act::Size(1));
    act(&mut a, Act::Size(1));
    let before = ed(&mut a).mem.clone();
    act(&mut a, Act::Color(green));
    assert_eq!(ed(&mut a).mem, before, "no color for blur");
    // The text tool's style: bold and serif, remembered.
    key(&mut a, 'T' as u32);
    act(&mut a, Act::ObjBold);
    act(&mut a, Act::ObjFont(crate::text::Family::Serif));
    act(&mut a, Act::ObjAlign(crate::text::Align::Right));
    key(&mut a, key_esc());
    key(&mut a, key_esc()); // the tool off, then the capture closes
    assert!(matches!(a.st, State::Hidden));
    assert_eq!(a.cfg.tool_sizes.get("line"), Some(&5.0));
    assert_eq!(a.cfg.tool_colors.get("line").map(String::as_str), Some("#00c850"));
    assert_eq!(a.cfg.tool_sizes.get("rect"), Some(&2.0));
    assert_eq!(a.cfg.tool_colors.get("marker").map(String::as_str), Some("#fadc0080"));
    assert_eq!(a.cfg.tool_sizes.get("pixelate"), Some(&13.0));
    assert!(!a.cfg.tool_colors.contains_key("pixelate") && !a.cfg.tool_sizes.contains_key("arrow"));
    assert_eq!(a.cfg.tool_sizes.get("blur"), Some(&(super::tools::BLUR_SIZE + 2.0)));
    assert!(!a.cfg.tool_colors.contains_key("blur"));
    assert_eq!(a.cfg.tool_fonts.get("text").map(String::as_str), Some("serif"));
    assert_eq!(a.cfg.tool_text_styles.get("text").map(String::as_str), Some("bold"));
    assert_eq!(a.cfg.tool_aligns.get("text").map(String::as_str), Some("right"));
    assert!(!a.cfg.tool_fonts.contains_key("callout"), "the callout keeps its own");
    // The next capture.
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    a.pending = Some(Pending::editor());
    a.on_event(Ev::Timer);
    let e = ed(&mut a);
    assert_eq!((e.mem.size(Tool::Line), e.mem.color(Tool::Line)), (5.0, green));
    assert_eq!(e.mem.size(Tool::Rect), 2.0);
    assert_eq!(e.mem.color(Tool::Marker), C4::new(250, 220, 0, 128));
    assert_eq!(e.mem.size(Tool::Pixelate), 13.0);
    assert_eq!(e.mem.size(Tool::Blur), super::tools::BLUR_SIZE + 2.0);
    let want = crate::text::TextFmt { bold: true, family: crate::text::Family::Serif, align: crate::text::Align::Right, ..Default::default() };
    assert_eq!((e.mem.fmt(Tool::Text), e.mem.fmt(Tool::Callout)), (want, Default::default()));
    assert_eq!((e.mem.size(Tool::Arrow), e.mem.color(Tool::Arrow)), (3.0, C4::rgb(240, 68, 56)), "never changed: the defaults");
    // A line drawn now has the remembered width and color.
    key(&mut a, 'L' as u32);
    down(&mut a, (300, 300));
    mv(&mut a, (400, 350));
    up(&mut a, (400, 350));
    assert!(matches!(ed(&mut a).objects[..], [Obj::Line { color, width: 5.0, .. }] if color == green));
}

fn key_esc() -> u32 {
    wind::key::ESCAPE
}

/// Writes wait for a pause (wheel scrolls coalesce) and keep the rest of
/// the file; an overlay that closes writes what is left at once.
#[test]
fn remembered_settings_are_written_after_a_pause_and_on_close() {
    let file = Scratch::new("debounce", "# my settings\ntheme = \"dark\" # keep\n\n[shortcuts]\nsave = \"Ctrl+S\"\n");
    let mut a = app_big();
    ed(&mut a).cfg_path = Some(file.0.clone());
    key(&mut a, 'L' as u32);
    for _ in 0..3 {
        act(&mut a, Act::Size(1));
    }
    a.pump();
    assert!(!file.text().contains("tool_line_size"), "not before the pause");
    assert_eq!(ed(&mut a).pending.get("tool_line_size"), Some(&Some("6".to_string())), "coalesced");
    // The pause is over.
    ed(&mut a).pending.set("tool_line_size".into(), Some("6".into()), Instant::now() - Duration::from_secs(1));
    a.pump();
    assert_eq!(
        file.text(),
        "# my settings\ntheme = \"dark\" # keep\ntool_line_size = 6\n\n[shortcuts]\nsave = \"Ctrl+S\"\n",
        "only the key's line, before the table"
    );
    assert!(ed(&mut a).pending.is_empty());
    // Changed again, then closed before the pause: written on close.
    act(&mut a, Act::Color(C4::rgb(1, 2, 3)));
    act(&mut a, Act::Size(1));
    key(&mut a, key_esc());
    key(&mut a, key_esc());
    let cfg = crate::config::parse_config(&file.text()).unwrap();
    assert_eq!(cfg.tool_sizes.get("line"), Some(&7.0));
    assert_eq!(cfg.tool_colors.get("line").map(String::as_str), Some("#010203"));
    assert!(file.text().starts_with("# my settings\ntheme = \"dark\" # keep\n"), "comments kept");
    assert_eq!(cfg.shortcuts.get("save").map(String::as_str), Some("Ctrl+S"));
}

/// A file broken meanwhile is never written over.
#[test]
fn a_broken_config_is_left_alone() {
    let file = Scratch::new("broken", "theme = \"dark\"\noops\n");
    let mut a = app_big();
    ed(&mut a).cfg_path = Some(file.0.clone());
    act(&mut a, Act::Size(1));
    key(&mut a, key_esc());
    assert_eq!(file.text(), "theme = \"dark\"\noops\n");
}

/// The readout and its preview follow the tool; Invert has neither a
/// size nor a color, Pixelate no color.
#[test]
fn readout_follows_the_tool() {
    let mut a = app_big();
    key(&mut a, 'T' as u32);
    let e = ed(&mut a);
    e.prepare(None, (0, 0), Instant::now());
    let b = e.scene.as_ref().unwrap().bar.as_ref().unwrap().clone();
    assert_eq!((b.value.as_str(), b.ptool, b.size), ("16", Tool::Text, 16.0));
    key(&mut a, 'I' as u32);
    let e = ed(&mut a);
    e.prepare(None, (0, 0), Instant::now());
    let tb = e.toolbar.as_ref().unwrap();
    let dis = |k: toolbar::Kind| tb.items.iter().find(|i| i.kind == k).unwrap().disabled;
    assert!(dis(toolbar::Kind::Btn(Act::Size(1))) && dis(toolbar::Kind::Value) && dis(toolbar::Kind::Btn(Act::Palette)));
    key(&mut a, 'X' as u32);
    let e = ed(&mut a);
    e.prepare(None, (0, 0), Instant::now());
    let tb = e.toolbar.as_ref().unwrap();
    let dis = |k: toolbar::Kind| tb.items.iter().find(|i| i.kind == k).unwrap().disabled;
    assert!(!dis(toolbar::Kind::Btn(Act::Size(1))) && dis(toolbar::Kind::Btn(Act::Palette)));
    // Deselected: the last tool's settings stay on show.
    key(&mut a, 'X' as u32);
    assert_eq!((ed(&mut a).tool, ed(&mut a).cur_tool()), (None, Tool::Pixelate));
}

/// The preview draws at the real width, capped to its box.
#[test]
fn size_preview_geometry() {
    let th = theme::DARK;
    let ui = chrome::Ui { th: &th, s: 1.0, font: Some(fonts::ui_font()) };
    let item = FRect { x: 10.0, y: 10.0, w: 76.0, h: 32.0 };
    let b = chrome::preview_box(&ui, item);
    assert_eq!((b.x, b.w, b.h), (42.0, 40.0, 24.0));
    assert!(b.x1() <= item.x1() && b.y >= item.y && b.y1() <= item.y1());
    // The stroke's rows: as many as the size, at most the box height.
    let rows = |tool: Tool, size: f32| {
        let mut d = vec![0u8; 100 * 60 * 4];
        let mut f = crate::uifb::Fb::new(&mut d, 100);
        chrome::size_preview(&mut f, &ui, b, tool, size, C4::rgb(255, 0, 0), 1.0);
        let col = (b.x + b.w / 2.0) as usize;
        (0..60).filter(|y| d[(y * 100 + col) * 4] > 128).count()
    };
    assert_eq!(rows(Tool::Line, 1.0), 1);
    assert_eq!(rows(Tool::Line, 4.0), 4);
    assert_eq!(rows(Tool::Rect, 9.0), 9);
    assert_eq!(rows(Tool::Line, 50.0), 24, "capped to the box");
    assert_eq!(rows(Tool::Invert, 12.0), 0);
    // Pixelate: blocks of the size (two rows at most).
    let mut d = vec![0u8; 100 * 60 * 4];
    let mut f = crate::uifb::Fb::new(&mut d, 100);
    chrome::size_preview(&mut f, &ui, b, Tool::Pixelate, 6.0, C4::rgb(255, 0, 0), 1.0);
    let y = (b.y + 1.0) as usize;
    let row: Vec<u8> = (b.x as usize..b.x1() as usize).map(|x| d[(y * 100 + x) * 4]).collect();
    let edges = row.windows(2).filter(|w| w[0] != w[1]).count();
    assert_eq!(edges, 6, "40 px of 6 px blocks: seven blocks, six edges");
}

// ---- the color picker -------------------------------------------------------

fn open_picker(a: &mut App) {
    act(a, Act::Palette);
    let rb = item(a, |k| matches!(k, toolbar::Kind::Rainbow(_)));
    click(a, mid(rb));
    assert!(ed(a).pick.is_some() && !ed(a).palette_open, "the rainbow opens the picker");
}

#[test]
fn picker_drags_live_esc_reverts_enter_keeps() {
    let mut a = app_big();
    key(&mut a, 'L' as u32);
    let orig = ed(&mut a).color();
    open_picker(&mut a);
    let hue = item(&mut a, |k| *k == toolbar::Kind::PickHue);
    let (cx, cy) = (hue.x + hue.w / 2.0, hue.y + hue.h / 2.0);
    let ring_r = hue.w / 2.0 - 8.0;
    // The ring's right point: hue 90 (red at the top, clockwise).
    down(&mut a, ((cx + ring_r) as i32, cy as i32));
    assert!(matches!(ed(&mut a).interact, Interact::Pick(Part::Hue)));
    let h = ed(&mut a).pick.as_ref().unwrap().h;
    assert!((h - 90.0).abs() < 1.5, "{h}");
    // Dragged on round to the bottom: hue 180, shown on the tool live.
    mv(&mut a, (cx as i32, (cy + ring_r) as i32));
    up(&mut a, (cx as i32, (cy + ring_r) as i32));
    let p = ed(&mut a).pick.clone().unwrap();
    assert!((p.h - 180.0).abs() < 1.5 && p.focus == Some(Part::Hue));
    assert_eq!(ed(&mut a).color(), p.color(), "live");
    assert_ne!(p.color(), orig);
    key(&mut a, key_esc());
    assert!(ed(&mut a).pick.is_none());
    assert_eq!(ed(&mut a).color(), orig, "Esc puts it back");
    assert!(ed(&mut a).pending.is_empty() && ed(&mut a).recents().is_empty());
    // The square's top right corner: full saturation and value; Enter keeps.
    open_picker(&mut a);
    let sv = item(&mut a, |k| *k == toolbar::Kind::PickSv);
    click(&mut a, ((sv.x1() - 1.0) as i32, sv.y as i32));
    let c = ed(&mut a).pick.as_ref().unwrap().color();
    assert!(c.r == 255 || c.g == 255 || c.b == 255, "{c:?}");
    key(&mut a, wind::key::RETURN);
    assert!(ed(&mut a).pick.is_none() && !ed(&mut a).done, "Enter closes the picker, not the capture");
    assert_eq!(ed(&mut a).color(), c);
    assert_eq!(ed(&mut a).recents(), [c], "a recent color");
    assert!(ed(&mut a).pending.get("tool_line_color").is_some() && ed(&mut a).pending.get("recent_colors").is_some());
    // A click outside closes it, keeping the color, and does nothing else.
    open_picker(&mut a);
    let objs = ed(&mut a).objects.len();
    click(&mut a, (400, 300));
    assert!(ed(&mut a).pick.is_none() && matches!(ed(&mut a).interact, Interact::None));
    assert_eq!(ed(&mut a).objects.len(), objs, "no line drawn");
    // A tool button closes it and acts.
    open_picker(&mut a);
    let rect = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Tool(Tool::Rect)));
    click(&mut a, mid(rect));
    assert!(ed(&mut a).pick.is_none() && ed(&mut a).tool == Some(Tool::Rect));
}

#[test]
fn picker_keyboard_hex_and_opacity() {
    let mut a = app_big();
    key(&mut a, 'L' as u32);
    open_picker(&mut a);
    // Typing goes to the hex field (it takes the focus, the text replaced).
    type_str(&mut a, "#00ff00");
    let p = ed(&mut a).pick.clone().unwrap();
    assert_eq!((p.focus, p.hex.as_str(), p.color()), (Some(Part::Hex), "#00ff00", C4::rgb(0, 255, 0)));
    assert_eq!(ed(&mut a).color(), C4::rgb(0, 255, 0), "live");
    // Backspace edits; "rgb(…)" reads too.
    for _ in 0..7 {
        key(&mut a, wind::key::BACK);
    }
    type_str(&mut a, "RGB(10, 20, 30)");
    assert_eq!(ed(&mut a).color(), C4::rgb(10, 20, 30));
    type_str(&mut a, "x");
    assert!(!ed(&mut a).pick.as_ref().unwrap().hex_ok);
    key(&mut a, wind::key::RETURN);
    assert!(ed(&mut a).pick.is_some(), "Enter waits for a color that reads");
    key(&mut a, wind::key::BACK);
    // Tab to the opacity slider; Shift+Left takes 10 % off.
    key(&mut a, wind::key::TAB);
    assert_eq!(ed(&mut a).pick.as_ref().unwrap().focus, Some(Part::Alpha));
    let shift = Mods { shift: true, ..Mods::default() };
    key_m(&mut a, wind::key::LEFT, shift);
    key_m(&mut a, wind::key::LEFT, shift);
    assert_eq!(ed(&mut a).pick.as_ref().unwrap().alpha_pct(), 80);
    // Shift+Tab back to the field, twice more to the square, the ring.
    key_m(&mut a, wind::key::TAB, shift);
    key_m(&mut a, wind::key::TAB, shift);
    assert_eq!(ed(&mut a).pick.as_ref().unwrap().focus, Some(Part::Sv));
    key(&mut a, wind::key::DOWN);
    // Tool shortcuts do nothing while the picker is open.
    key(&mut a, 'R' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Line));
    key(&mut a, wind::key::RETURN);
    let c = ed(&mut a).color();
    assert_eq!(c.a, 204, "80 % opaque");
    // Drawn translucent.
    down(&mut a, (300, 300));
    mv(&mut a, (420, 330));
    up(&mut a, (420, 330));
    assert!(matches!(ed(&mut a).objects[..], [Obj::Line { color, .. }] if color.a == 204));
}

#[test]
fn picker_slider_and_recents_by_mouse() {
    let mut a = app_big();
    ed(&mut a).cfg.recent_colors = vec!["#112233".into(), "#445566cc".into()];
    open_picker(&mut a);
    let s = item(&mut a, |k| *k == toolbar::Kind::PickAlpha);
    click(&mut a, ((s.x + 2.0) as i32, (s.y + s.h / 2.0) as i32));
    assert_eq!(ed(&mut a).pick.as_ref().unwrap().a, 0, "the track's left end");
    click(&mut a, ((s.x1() - 2.0) as i32, (s.y + s.h / 2.0) as i32));
    assert_eq!(ed(&mut a).pick.as_ref().unwrap().a, 255);
    let rc = item(&mut a, |k| matches!(k, toolbar::Kind::Recent { i: 1, .. }));
    click(&mut a, mid(rc));
    assert_eq!(ed(&mut a).color(), C4::new(0x44, 0x55, 0x66, 0xcc));
    key(&mut a, wind::key::RETURN);
    assert_eq!(ed(&mut a).recents(), [C4::new(0x44, 0x55, 0x66, 0xcc), C4::rgb(0x11, 0x22, 0x33)], "to the front, once");
    // At most eight.
    ed(&mut a).cfg.recent_colors = (0..8).map(|i| format!("#0000{i:02x}")).collect();
    open_picker(&mut a);
    type_str(&mut a, "#abcdef");
    key(&mut a, wind::key::RETURN);
    let r = ed(&mut a).recents();
    assert_eq!((r.len(), r[0]), (8, C4::new(0xab, 0xcd, 0xef, 0xcc)), "the field sets the color, not its opacity");
}

/// The eyedropper: the loupe follows the pointer, a click takes the
/// pixel's color (the capture's, not dimmed), Esc stops it first.
#[test]
fn eyedropper_samples_the_screenshot() {
    let mut a = app_big();
    key(&mut a, 'L' as u32);
    open_picker(&mut a);
    let eye = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Eyedrop));
    click(&mut a, mid(eye));
    assert!(ed(&mut a).pick.as_ref().unwrap().eyedrop);
    assert_eq!(a.cursor(), Cursor::Cross);
    mv(&mut a, BLUE_AT);
    let e = ed(&mut a);
    e.prepare(None, BLUE_AT, Instant::now());
    let l = e.scene.as_ref().unwrap().loupe.clone().expect("loupe");
    assert_eq!(l.px[chrome::LOUPE_N * chrome::LOUPE_N / 2], BLUE);
    // Below right of the pointer would cover the picker: on another side.
    let t = e.toolbar.as_ref().unwrap();
    for o in std::iter::once(t.bar).chain(t.pop) {
        assert!(l.r.x1() <= o.x || o.x1() <= l.r.x || l.r.y1() <= o.y || o.y1() <= l.r.y, "{:?} over {o:?}", l.r);
    }
    let (px, py) = (BLUE_AT.0 as f32, BLUE_AT.1 as f32);
    assert!((l.r.x > px || l.r.x1() < px) && (l.r.y > py || l.r.y1() < py), "beside the pointer");
    // Esc: the eyedropper off, the picker still open.
    key(&mut a, key_esc());
    assert!(ed(&mut a).pick.as_ref().is_some_and(|p| !p.eyedrop) && ed(&mut a).loupe.is_none());
    click(&mut a, mid(eye));
    // Outside the selection the pixel is taken undimmed.
    mv(&mut a, (1250, 420));
    click(&mut a, (1250, 420));
    let p = ed(&mut a).pick.clone().unwrap();
    assert!(!p.eyedrop);
    assert_eq!(p.color().with_alpha(255), C4::rgb(245, 246, 248), "the window's white");
    click(&mut a, mid(eye));
    click(&mut a, BLUE_AT);
    assert_eq!(ed(&mut a).color(), BLUE, "live on the tool");
    key(&mut a, wind::key::RETURN);
    assert_eq!(ed(&mut a).color(), BLUE);
    // Off the image: black.
    assert_eq!(ed(&mut a).sample(Pt::new(-5.0, -5.0)), C4::rgb(0, 0, 0));
}

/// The loupe stays on the monitor near its edges, and off the toolbar.
#[test]
fn loupe_flips_at_the_edges() {
    let th = theme::DARK;
    let ui = chrome::Ui { th: &th, s: 1.0, font: None };
    let area = r(0.0, 0.0, 1000.0, 800.0);
    let l = chrome::loupe_rect(&ui, Pt::new(100.0, 100.0), area, &[]);
    assert_eq!((l.x, l.y), (120.0, 120.0));
    let l = chrome::loupe_rect(&ui, Pt::new(990.0, 790.0), area, &[]);
    assert!(l.x1() <= 970.0 && l.y1() <= 770.0, "{l:?}");
    // A bar below right of the pointer: the loupe goes to the other side.
    let bar = r(100.0, 320.0, 600.0, 48.0);
    let l = chrome::loupe_rect(&ui, Pt::new(120.0, 300.0), area, &[bar]);
    assert!(l.y1() <= bar.y && l.x == 140.0, "above the bar: {l:?}");
    let l = chrome::loupe_rect(&ui, Pt::new(300.0, 400.0), area, &[r(320.0, 300.0, 200.0, 300.0)]);
    assert!(l.x1() <= 320.0 && l.y == 420.0, "left of the popover: {l:?}");
    // No room on the other side: on the monitor wins (over the bar).
    let l = chrome::loupe_rect(&ui, Pt::new(10.0, 10.0), area, &[r(0.0, 0.0, 1000.0, 800.0)]);
    assert_eq!((l.x, l.y), (30.0, 30.0));
}

/// Five monitors around the selection's: with the pointer on another one,
/// the loupe stays on the monitor under the pointer (not the selection's).
#[test]
fn loupe_uses_the_monitor_under_the_pointer() {
    let mut shot = synthetic_shot();
    let (c, left, right, above, below) =
        ((480, 300, 480, 300), (0, 300, 480, 300), (960, 300, 480, 300), (480, 0, 480, 300), (480, 600, 480, 300));
    shot.monitors = vec![c, left, right, above, below];
    let mut a = preview_app_with(theme::DARK, Some(r(560.0, 380.0, 300.0, 120.0)), shot);
    a.focus_tries = u8::MAX;
    key(&mut a, 'L' as u32);
    open_picker(&mut a);
    let mon = |m: (i32, i32, u32, u32)| r(m.0 as f32, m.1 as f32, m.2 as f32, m.3 as f32);
    for (name, m, p) in [
        ("left, near its right edge", left, (470, 450)),
        ("right, near its top-right", right, (1430, 310)),
        ("above, near its bottom", above, (700, 290)),
        ("below, near its top", below, (700, 610)),
        ("below, bottom-left corner", below, (485, 895)),
    ] {
        let e = ed(&mut a);
        e.pick.as_mut().unwrap().eyedrop = true;
        e.loupe = Some(Pt::new(p.0 as f32, p.1 as f32));
        e.prepare(None, p, Instant::now());
        let l = e.scene.as_ref().unwrap().loupe.clone().unwrap_or_else(|| panic!("{name}: loupe")).r;
        let m = mon(m);
        assert!(l.x >= m.x && l.y >= m.y && l.x1() <= m.x1() && l.y1() <= m.y1(), "{name}: {l:?} on {m:?}");
        if let Some(t) = e.toolbar.as_ref() {
            assert!(!toolbar::hit(t.bar, Pt::new(l.x + l.w / 2.0, l.y + l.h / 2.0)), "{name}: on the bar");
        }
    }
}

// ---- dirty rects -------------------------------------------------------------

#[test]
fn dirty_rects_cover_the_new_chrome() {
    let mut a = annotated(theme::DARK, BIG);
    settled(&mut a);
    assert_dirty_covers(&mut a, "size step", |a| act(a, Act::Size(1)));
    assert_dirty_covers(&mut a, "tool switch preview", |a| key(a, 'T' as u32));
    assert_dirty_covers(&mut a, "pixelate preview", |a| key(a, 'B' as u32));
    assert_dirty_covers(&mut a, "palette with the rainbow", |a| act(a, Act::Palette));
    snap_all(&mut a);
    assert_dirty_covers(&mut a, "picker opens", |a| {
        key(a, 'L' as u32);
        act(a, Act::Wheel);
    });
    snap_all(&mut a);
    let hue = item(&mut a, |k| *k == toolbar::Kind::PickHue);
    assert_dirty_covers(&mut a, "hue drag", |a| down(a, ((hue.x + hue.w - 6.0) as i32, (hue.y + hue.h / 2.0) as i32)));
    assert_dirty_covers(&mut a, "hue drag on", |a| mv(a, ((hue.x + hue.w / 2.0) as i32, (hue.y + hue.h - 6.0) as i32)));
    assert_dirty_covers(&mut a, "release", |a| up(a, (0, 0)));
    assert_dirty_covers(&mut a, "typing", |a| type_str(a, "#3b82f6"));
    assert_dirty_covers(&mut a, "focus", |a| key(a, wind::key::TAB));
    let eye = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Eyedrop));
    assert_dirty_covers(&mut a, "eyedropper", |a| click(a, mid(eye)));
    assert_dirty_covers(&mut a, "loupe shows", |a| mv(a, BLUE_AT));
    assert_dirty_covers(&mut a, "loupe moves", |a| mv(a, (BLUE_AT.0 + 37, BLUE_AT.1 - 11)));
    assert_dirty_covers(&mut a, "loupe at the edge", |a| mv(a, (1430, 890)));
    assert_dirty_covers(&mut a, "picked", |a| click(a, BLUE_AT));
    assert_dirty_covers(&mut a, "picker closes", |a| key(a, wind::key::RETURN));
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn dirty_rects_cover_the_ring_controls() {
    let mut a = annotated(theme::DARK, BIG);
    settled(&mut a);
    assert_dirty_covers(&mut a, "record popover", |a| {
        let e = ed(a);
        e.rec.open = true;
        e.rec.load_cams();
        e.rec.opts.camera = true;
    });
    snap_all(&mut a);
    assert_dirty_covers(&mut a, "ring wider", |a| act(a, Act::RingSize(1)));
    assert_dirty_covers(&mut a, "ring off", |a| ed(a).rec.opts.cam_ring_w = 0);
    assert_dirty_covers(&mut a, "ring palette", |a| act(a, Act::RingPalette));
    assert_dirty_covers(&mut a, "ring color", |a| act(a, Act::RingColor(Some(C4::rgb(45, 192, 111)))));
    assert_dirty_covers(&mut a, "ring picker", |a| act(a, Act::RingWheel));
    snap_all(&mut a);
    assert_dirty_covers(&mut a, "back to the popover", |a| key(a, key_esc()));
}

// ---- the camera ring ---------------------------------------------------------

/// The Ring row: thinner/thicker (0 = none), a palette with the accent
/// and the rainbow (the picker, back to the popover after); applied to
/// the bubble's look and remembered.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn ring_controls_change_the_bubble() {
    let mut a = app_big();
    let rec = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Record));
    click(&mut a, mid(rec));
    let cam = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    click(&mut a, mid(cam));
    assert!(ed(&mut a).rec.opts.camera);
    // What the bubble shows (after the pump that keeps it in step), which
    // is what the popover asks for.
    let look = |a: &mut App| {
        a.pump();
        let e = ed(a);
        let want = e.cam_want(&e.rec_geom(&[])).look;
        let shown = e.rec.cam.shown.as_ref().map(|s| s.1).expect("the bubble is shown");
        assert_eq!(shown, want, "the bubble shows the look");
        shown
    };
    assert_eq!((look(&mut a).ring, look(&mut a).ring_w), (None, 2), "the accent, 2 px");
    // The ring's swatch in the Camera row opens its palette: the width
    // and the colors.
    let sw = item(&mut a, |k| matches!(k, toolbar::Kind::Swatch { act: Act::RingPalette, .. }));
    click(&mut a, mid(sw));
    assert!(ed(&mut a).rec.ring_pal);
    let plus = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::RingSize(1)));
    click(&mut a, mid(plus));
    assert_eq!(look(&mut a).ring_w, 3);
    let minus = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::RingSize(-1)));
    for _ in 0..6 {
        click(&mut a, mid(minus));
    }
    assert_eq!(look(&mut a).ring_w, 0, "no ring, no lower");
    let minus_item = |a: &mut App| {
        let m = a.mouse;
        ed(a).prepare(None, m, Instant::now());
        ed(a).toolbar.as_ref().unwrap().items.iter().find(|i| i.kind == toolbar::Kind::Btn(Act::RingSize(-1))).unwrap().disabled
    };
    assert!(minus_item(&mut a), "thinner is off at 0");
    click(&mut a, mid(plus));
    // The palette: the accent first, the presets, the rainbow last.
    assert!(ed(&mut a).rec.ring_pal, "still open");
    let green = item(&mut a, |k| matches!(k, toolbar::Kind::Swatch { act: Act::RingColor(Some(c)), .. } if *c == C4::rgb(0x2d, 0xc0, 0x6f)));
    click(&mut a, mid(green));
    assert!(!ed(&mut a).rec.ring_pal);
    assert_eq!(look(&mut a).ring, Some(C4::rgb(0x2d, 0xc0, 0x6f)));
    assert_eq!(ed(&mut a).cfg.rec_camera_ring_color, "#2dc06f");
    assert_eq!(ed(&mut a).pending.get("rec_camera_ring_width"), Some(&Some("1".to_string())));
    // The picker for the ring sits where the popover was; Esc puts the
    // color back and the popover returns.
    click(&mut a, mid(sw));
    let rb = item(&mut a, |k| *k == toolbar::Kind::Rainbow(Act::RingWheel));
    click(&mut a, mid(rb));
    let p = ed(&mut a).pick.clone().expect("picker");
    assert_eq!((p.target, p.has_alpha()), (Target::Ring, false));
    item(&mut a, |k| *k == toolbar::Kind::PickHue);
    assert!(!ed(&mut a).toolbar.as_ref().unwrap().items.iter().any(|i| i.kind == toolbar::Kind::PickAlpha), "no opacity for the ring");
    type_str(&mut a, "#ff0000");
    assert_eq!(look(&mut a).ring, Some(C4::rgb(255, 0, 0)), "live on the bubble");
    key(&mut a, key_esc());
    assert_eq!(look(&mut a).ring, Some(C4::rgb(0x2d, 0xc0, 0x6f)));
    assert!(ed(&mut a).pick.is_none() && ed(&mut a).rec.open);
    // The accent again.
    let sw = item(&mut a, |k| matches!(k, toolbar::Kind::Swatch { act: Act::RingPalette, .. }));
    click(&mut a, mid(sw));
    let acc = item(&mut a, |k| matches!(k, toolbar::Kind::Swatch { act: Act::RingColor(None), .. }));
    click(&mut a, mid(acc));
    assert_eq!(look(&mut a).ring, None);
    assert_eq!(ed(&mut a).cfg.rec_camera_ring_color, "");
    // Remembered for the next capture (and the recording's bubble).
    key(&mut a, key_esc());
    key(&mut a, key_esc());
    assert_eq!(a.cfg.rec_camera_ring_width, 1);
    assert_eq!(a.rec_opts.as_ref().map(|o| o.cam_ring_w), Some(1));
}

/// The ring's picker dismissed without a change keeps the ring on the
/// accent (`None`, not the accent's color) and adds no recent color.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn ring_picker_closed_unchanged_keeps_the_accent() {
    let mut a = app_big();
    ed(&mut a).cfg.recent_colors = vec!["#123456".into()];
    let rec = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Record));
    click(&mut a, mid(rec));
    let cam = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    click(&mut a, mid(cam));
    assert_eq!(ed(&mut a).rec.opts.cam_ring, None);
    let open_ring = |a: &mut App| {
        let sw = item(a, |k| matches!(k, toolbar::Kind::Swatch { act: Act::RingPalette, .. }));
        click(a, mid(sw));
        let rb = item(a, |k| *k == toolbar::Kind::Rainbow(Act::RingWheel));
        click(a, mid(rb));
        assert_eq!(ed(a).pick.as_ref().map(|p| p.target), Some(Target::Ring));
    };
    // Enter at once.
    open_ring(&mut a);
    key(&mut a, wind::key::RETURN);
    // Moved away and back, then a click outside.
    open_ring(&mut a);
    key(&mut a, wind::key::RIGHT);
    key(&mut a, wind::key::LEFT);
    assert_eq!(ed(&mut a).pick.as_ref().unwrap().color(), ed(&mut a).th.accent);
    click(&mut a, (BIG.x as i32 + 30, BIG.y as i32 + 30));
    let e = ed(&mut a);
    assert!(e.pick.is_none());
    assert_eq!(e.rec.opts.cam_ring, None, "still the accent");
    assert_eq!(e.cfg.recent_colors, ["#123456"], "no recent");
    assert_eq!(e.pending.get("rec_camera_ring_color"), None, "nothing to write");
    // Changed: kept, remembered, a recent one.
    open_ring(&mut a);
    type_str(&mut a, "#00ff00");
    key(&mut a, wind::key::RETURN);
    let e = ed(&mut a);
    assert_eq!(e.rec.opts.cam_ring, Some(C4::rgb(0, 255, 0)));
    assert_eq!(e.cfg.recent_colors, ["#00ff00", "#123456"]);
}

/// The tool's picker closed on the color it opened with remembers
/// nothing and adds no recent color.
#[test]
fn tool_picker_closed_unchanged_remembers_nothing() {
    let mut a = app_big();
    ed(&mut a).cfg.recent_colors = vec!["#00ff00".into(), "#123456".into()];
    key(&mut a, 'L' as u32);
    ed(&mut a).pending.take();
    open_picker(&mut a);
    key(&mut a, wind::key::RETURN);
    let e = ed(&mut a);
    assert_eq!(e.cfg.recent_colors, ["#00ff00", "#123456"], "no recent");
    assert!(e.pending.is_empty(), "nothing remembered");
}

/// Quitting the daemon under an open picker keeps the picked color (as
/// closing the overlay does) and writes it before the process goes.
#[test]
fn quitting_keeps_the_picked_color_and_flushes() {
    let file = Scratch::new("quit", "theme = \"dark\"\n");
    let mut a = app_big();
    a.kind = RunKind::Daemon;
    ed(&mut a).cfg_path = Some(file.0.clone());
    key(&mut a, 'L' as u32);
    open_picker(&mut a);
    type_str(&mut a, "#0a0b0c");
    assert!(!file.text().contains("tool_line_color"), "not yet");
    a.save_before_exit();
    let e = ed(&mut a);
    assert!(e.pick.is_none() && e.pending.is_empty());
    assert_eq!(e.mem.color(Tool::Line), C4::rgb(10, 11, 12));
    let cfg = crate::config::parse_config(&file.text()).unwrap();
    assert_eq!(cfg.tool_colors.get("line").map(String::as_str), Some("#0a0b0c"));
    assert_eq!(cfg.recent_colors, ["#0a0b0c"]);
}

/// Space (the palette's key) and the wheel do nothing for the tools
/// without a color.
#[test]
fn no_palette_for_pixelate_and_invert() {
    let mut a = app_big();
    for k in ['B', 'I'] {
        key(&mut a, k as u32);
        assert!(!tools::has_color(ed(&mut a).cur_tool()));
        act(&mut a, Act::Palette);
        assert!(!ed(&mut a).palette_open, "{k}: no palette");
        act(&mut a, Act::Wheel);
        assert!(ed(&mut a).pick.is_none(), "{k}: no picker");
    }
    key(&mut a, 'L' as u32);
    act(&mut a, Act::Palette);
    assert!(ed(&mut a).palette_open);
}

/// A popover below the bar puts its controls' tooltips below them, never
/// over the bar.
#[test]
fn popover_tips_stay_off_the_bar() {
    let mut a = app_big();
    act(&mut a, Act::Palette);
    let rb = item(&mut a, |k| matches!(k, toolbar::Kind::Rainbow(_)));
    let (bar, pop) = {
        let t = ed(&mut a).toolbar.as_ref().unwrap();
        (t.bar, t.pop.expect("palette"))
    };
    assert!(pop.y >= bar.y1(), "the palette is below the bar here");
    let i = ed(&mut a).toolbar.as_ref().unwrap().items.iter().position(|it| it.r == rb).unwrap();
    let e = ed(&mut a);
    e.hover = Some(i);
    e.hover_at = Instant::now() - Duration::from_secs(2);
    let p = mid(rb);
    e.prepare(None, p, Instant::now());
    let tip = e.scene.as_ref().unwrap().tip.clone().expect("tooltip");
    assert!(tip.below, "below the swatch");
    // A bar button's tip stays above it.
    let copy = e.toolbar.as_ref().unwrap().items.iter().position(|it| it.kind == toolbar::Kind::Btn(Act::Copy)).unwrap();
    e.hover = Some(copy);
    e.hover_at = Instant::now() - Duration::from_secs(2);
    e.prepare(None, p, Instant::now());
    assert!(!e.scene.as_ref().unwrap().tip.clone().expect("tooltip").below);
}

/// The panel's Background row: a choice goes to the bubble's look at
/// once and is remembered (`rec_camera_bg`) for the next capture; the
/// config's value starts it. A build that cannot blur ignores it.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn background_choice_reaches_the_bubble_and_is_remembered() {
    use crate::cam::bg::Bg;
    crate::cam::bg::set_available(Some(true));
    let mut a = app_big();
    let rec = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Record));
    click(&mut a, mid(rec));
    let cam = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    click(&mut a, mid(cam));
    let look = |a: &mut App| {
        a.pump();
        ed(a).rec.cam.shown.as_ref().map(|s| s.1).expect("the bubble is shown")
    };
    assert_eq!(look(&mut a).bg, Bg::Off);
    let strong = item(&mut a, |k| matches!(k, toolbar::Kind::Seg { act: Act::RecBg(Bg::Strong), .. }));
    click(&mut a, mid(strong));
    assert_eq!(look(&mut a).bg, Bg::Strong);
    assert_eq!(ed(&mut a).pending.get("rec_camera_bg"), Some(&Some("\"strong\"".to_string())));
    key(&mut a, key_esc());
    key(&mut a, key_esc());
    assert_eq!(a.cfg.rec_camera_bg, "strong");
    assert_eq!(a.rec_opts.as_ref().map(|o| o.cam_bg), Some(Bg::Strong));
    assert_eq!(RecOpts::from_config(&Config { rec_camera_bg: "light".into(), ..Config::default() }).cam_look().bg, Bg::Light);
    // Unavailable: the row has no choices, and the action does nothing.
    crate::cam::bg::set_available(Some(false));
    let mut a = app_big();
    let rec = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Record));
    click(&mut a, mid(rec));
    let cam = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    click(&mut a, mid(cam));
    item(&mut a, |k| matches!(k, toolbar::Kind::Row { act: Act::RecBg(_), value: Some(v), .. } if *v == crate::cam::bg::UNAVAILABLE.get()));
    let tb = ed(&mut a).toolbar.clone().unwrap();
    assert!(!tb.items.iter().any(|i| matches!(i.kind, toolbar::Kind::Seg { act: Act::RecBg(_), .. })));
    crate::cam::bg::set_available(None);
}

/// Ring settings from the config reach the bubble's look.
#[test]
fn ring_settings_from_the_config() {
    let cfg = Config { rec_camera_ring_color: "#00ff00".into(), rec_camera_ring_width: 6, ..Config::default() };
    let look = RecOpts::from_config(&cfg).cam_look();
    assert_eq!((look.ring, look.ring_w), (Some(C4::rgb(0, 255, 0)), 6));
    let look = RecOpts::from_config(&Config::default()).cam_look();
    assert_eq!((look.ring, look.ring_w), (None, 2), "the board's 2 px ring");
}

// ---- previews ----------------------------------------------------------------

/// The synthetic shot at twice the size and UI scale 2.
fn shot2x() -> Shot {
    let s = synthetic_shot();
    let (w, h) = (s.size.0 * 2, s.size.1 * 2);
    let mut img = PixBuf::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = s.image.get_pixel(x / 2, y / 2);
            let i = (y as usize * w as usize + x as usize) * 4;
            img.as_raw_mut()[i..i + 4].copy_from_slice(&p);
        }
    }
    Shot { origin: (0, 0), size: (w, h), scale: 2.0, image: img, monitors: vec![(0, 0, w, h)] }
}

/// `annotated` on any shot (`k`: its scale, which the objects follow).
fn annot(th: Theme, sel: FRect, shot: Shot) -> App {
    let k = shot.scale;
    let mut a = preview_app_with(th, Some(sel), shot);
    let e = ed(&mut a);
    e.tool = Some(Tool::Arrow);
    e.last_tool = Tool::Arrow;
    let red = C4::rgb(240, 68, 56);
    e.objects.push(Obj::Rect { r: r(sel.x + 40.0 * k, sel.y + 40.0 * k, sel.w * 0.35, sel.h * 0.3), color: red, width: 3.0 * k });
    e.objects.push(Obj::Arrow {
        a: Pt::new(sel.x + sel.w * 0.8, sel.y + sel.h * 0.8),
        b: Pt::new(sel.x + sel.w * 0.5, sel.y + sel.h * 0.5),
        color: red,
        width: 4.0 * k,
    });
    e.dirty = true;
    a
}

fn paste(dst: &mut PixBuf, src: &PixBuf, x0: u32, y0: u32) {
    let w = dst.width();
    for y in 0..src.height() {
        for x in 0..src.width() {
            let (dx, dy) = (x0 + x, y0 + y);
            if dx < w && dy < dst.height() {
                let i = (dy as usize * w as usize + dx as usize) * 4;
                dst.as_raw_mut()[i..i + 4].copy_from_slice(&src.get_pixel(x, y));
            }
        }
    }
}

#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_tools_previews() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let save = |img: &PixBuf, name: &str| std::fs::write(dir.join(name), img.to_png().unwrap()).unwrap();
    for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
        for (sn, k) in [("1x", 1.0f32), ("2x", 2.0)] {
            let shot = || if k == 1.0 { synthetic_shot() } else { shot2x() };
            let sel = |x: f32, y: f32, w: f32, h: f32| r(x * k, y * k, w * k, h * k);
            let big = sel(240.0, 140.0, 960.0, 540.0);
            let name = |what: &str| format!("{tn}-{sn}-{what}.png");
            // The toolbar centred, and at the edges.
            for (what, s) in [
                ("toolbar-centred", big),
                ("toolbar-right-edge", sel(1180.0, 200.0, 250.0, 300.0)),
                ("toolbar-left-edge", sel(8.0, 200.0, 250.0, 300.0)),
                ("toolbar-narrow", sel(660.0, 300.0, 120.0, 90.0)),
                ("toolbar-above", sel(300.0, 700.0, 840.0, 160.0)),
                ("toolbar-inside", sel(0.0, 0.0, 1440.0, 900.0)),
            ] {
                let mut a = annot(th, s, shot());
                save(&settled_frame(&mut a), &name(what));
            }
            // The palette with the rainbow swatch.
            let mut a = annot(th, big, shot());
            ed(&mut a).palette_open = true;
            settled_frame(&mut a);
            let rb = item(&mut a, |k| matches!(k, toolbar::Kind::Rainbow(_)));
            let i = ed(&mut a).toolbar.as_ref().unwrap().items.iter().position(|it| it.r == rb);
            ed(&mut a).hover = i;
            ed(&mut a).hover_at = Instant::now() - Duration::from_secs(2);
            save(&settled_frame(&mut a), &name("palette-rainbow"));
            // The picker: fresh; typing a color that does not read; with
            // recents and 60 % opacity, the slider focused; the eyedropper.
            let recents: Vec<String> = ["#f04438", "#3b82f6cc", "#2dc06f", "#111318", "#ffffff", "#ec489966"].iter().map(|s| s.to_string()).collect();
            for (what, setup) in [
                ("picker", 0),
                ("picker-hex-invalid", 1),
                ("picker-recents-alpha", 2),
                ("picker-eyedropper", 3),
                ("picker-hue-focus", 4),
            ] {
                let mut a = annot(th, big, shot());
                if setup >= 2 {
                    ed(&mut a).cfg.recent_colors = recents.clone();
                }
                ed(&mut a).tool = Some(Tool::Line);
                ed(&mut a).open_picker(Target::Tool);
                let e = ed(&mut a);
                let p = e.pick.as_mut().unwrap();
                match setup {
                    1 => {
                        p.focus = Some(Part::Hex);
                        p.hex = "#3b8zz".into();
                        p.hex_ok = false;
                    }
                    2 => {
                        p.set_color(C4::new(59, 130, 246, 153));
                        p.focus = Some(Part::Alpha);
                    }
                    3 => {
                        p.eyedrop = true;
                        e.loupe = Some(Pt::new(BLUE_AT.0 as f32 * k, BLUE_AT.1 as f32 * k));
                    }
                    4 => {
                        p.set_color(C4::rgb(139, 92, 246));
                        p.focus = Some(Part::Hue);
                    }
                    _ => {}
                }
                e.pick_changed();
                save(&settled_frame(&mut a), &name(what));
            }
            // The camera row with the ring controls; its palette; no ring;
            // the picker for the ring.
            for (what, setup) in [("record-ring", 0), ("record-ring-palette", 1), ("record-ring-off", 2), ("record-ring-picker", 3)] {
                let mut a = annot(th, big, shot());
                let e = ed(&mut a);
                e.rec.open = true;
                e.rec.load_cams();
                e.rec.opts.camera = true;
                match setup {
                    1 => {
                        e.rec.ring_pal = true;
                        e.rec.opts.cam_ring = Some(C4::rgb(0x2d, 0xc0, 0x6f));
                    }
                    2 => e.rec.opts.cam_ring_w = 0,
                    3 => {
                        e.rec.opts.cam_ring_w = 6;
                        e.open_picker(Target::Ring);
                    }
                    _ => e.rec.opts.cam_ring_w = 5,
                }
                save(&settled_frame(&mut a), &name(what));
            }
            // The size readout per tool at several sizes: one sheet.
            let sizes = [1.0f32, 3.0, 8.0, 16.0, 24.0, 50.0, 96.0];
            let mut cells = Vec::new();
            for t in toolbar::TOOL_ORDER {
                for &v in &sizes {
                    let mut a = annot(th, big, shot());
                    let e = ed(&mut a);
                    e.tool = Some(t);
                    let (lo, hi) = tools::range(t);
                    e.mem.set_size(t, v.clamp(lo, hi));
                    e.mem.set_color(t, if t == Tool::Marker { C4::rgb(250, 220, 0) } else { C4::rgb(240, 68, 56) });
                    let img = settled_frame(&mut a);
                    let tb = ed(&mut a).toolbar.clone().unwrap();
                    let at = |a: Act| tb.items.iter().find(|i| i.kind == toolbar::Kind::Btn(a)).unwrap().r;
                    let (m, p) = (at(Act::Size(-1)), at(Act::Palette));
                    let pad = 4.0 * k;
                    cells.push(img.crop((m.x - pad) as u32, (m.y - pad) as u32, (p.x1() - m.x + 2.0 * pad) as u32, (m.h + 2.0 * pad) as u32));
                }
            }
            let (cw, ch) = (cells[0].width(), cells[0].height());
            let cols = sizes.len() as u32;
            let rows = toolbar::TOOL_ORDER.len() as u32;
            let mut sheet = PixBuf::from_pixel(cols * cw, rows * ch, [0, 0, 0, 255]);
            for (i, c) in cells.iter().enumerate() {
                paste(&mut sheet, c, (i as u32 % cols) * cw, (i as u32 / cols) * ch);
            }
            save(&sheet, &name("size-preview-sheet"));
        }
    }
}
