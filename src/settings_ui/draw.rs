//! Painting the Settings window as on the Settings board: the 188 px
//! rail (14 sections, a Shortcuts group), a 48 px header, the section body
//! that fits without scrolling (rows of the board's `.r`: a 12.5/500
//! label with an 11/400 helper line, controls on the right) and the
//! footer bar with Open config file and OK / Cancel /
//! Apply; questions over the dimmed window.

use super::*;
use crate::i18n::Key;
use crate::ui::compact::{RailStyle, Slot, Tok, CSS};
use crate::ui::controls::wrap;
use crate::uifb::C4;

/// The product name in the rail and on About (capital R, though the
/// boards write it in lowercase).
pub const NAME: &str = "Rustshot";
/// Layout of the window (logical px).
pub const RAIL_W: f32 = 188.0;
pub const HDR_H: f32 = 48.0;
pub const FOOT_H: f32 = 44.0;
/// The body's side padding (Key bindings: 14, its rows pad themselves).
pub const PAD_X: f32 = 22.0;
pub const KEYS_PAD_X: f32 = 14.0;
pub const BODY_TOP: f32 = 8.0;
pub const BODY_BOTTOM: f32 = 12.0;
/// A row's height and its controls' height.
pub const ROW_H: f32 = 36.0;
pub const CTL: f32 = 28.0;
/// The Win+Shift+S row's helper line (the line under it says which app
/// Windows opens: [`crate::screenclip_win::state_line`]).
pub const SCREENCLIP_HELP: Key = key!("settings.capture.screenclip.help");
/// The field width of Save to and the like.
pub const FIELD_W: f32 = 228.0;

/// The section body's viewport (between the header and the footer).
pub fn viewport() -> FRect {
    FRect { x: RAIL_W, y: HDR_H, w: W as f32 - RAIL_W, h: H as f32 - HDR_H - FOOT_H }
}

/// The rail: 15 sections and the Shortcuts label need items a little
/// shorter than the board's 32 px (26, 28 apart).
pub const RAIL: RailStyle<'static> = RailStyle { item: 26.0, pitch: 28.0, groups: &RAIL_GROUPS };

/// Where the rail's items go (from the top: the title bar names the app).
pub fn rail_area() -> FRect {
    FRect { x: 10.0, y: 10.0, w: RAIL_W - 20.0, h: H as f32 - 14.0 - 10.0 }
}

/// Logical → physical px (rounded).
fn phys(ui: &Ui, r: FRect) -> FRect {
    let k = ui.k;
    FRect { x: (r.x * k).round(), y: (r.y * k).round(), w: (r.w * k).round(), h: (r.h * k).round() }
}

fn fill_rect(ui: &mut Ui, r: FRect, c: C4) {
    let p = phys(ui, r);
    ui.fb.fill_rect(p.x as i32, p.y as i32, p.w as i32, p.h as i32, c);
}

/// A 1 px line (logical rect, at least one physical pixel thick).
fn line(ui: &mut Ui, x: f32, y: f32, w: f32, h: f32, c: C4) {
    let k = ui.k;
    let lw = k.round().max(1.0) as i32;
    let (px, py) = ((x * k).round() as i32, (y * k).round() as i32);
    let (pw, ph) = if h <= 1.0 { ((w * k).round() as i32, lw) } else { (lw, (h * k).round() as i32) };
    ui.fb.fill_rect(px, py, pw, ph, c);
}

/// The app icon at `(x, y)`, `size` logical px.
fn mark(ui: &mut Ui, x: f32, y: f32, size: f32) {
    let r = phys(ui, FRect { x, y, w: size, h: size });
    crate::tray::draw_app_icon(&mut ui.fb, r.x as i32, r.y as i32, r.w as u32);
}

/// A board row (`.r`): `label` (and `help`, wrapped into the lines `h`
/// has room for; `help_err` in the error color) on the left, the
/// controls `f` draws laid out right to left on the right.
pub fn srow_ex(ui: &mut Ui, tok: &Tok, label: &str, help: Option<&str>, help_err: bool, h: f32, f: impl FnOnce(&mut Ui)) {
    let b = ui.bounds();
    let y = ui.cursor_y();
    let cy = y + h / 2.0;
    let ctl = FRect { x: b.x, y: cy - CTL / 2.0, w: b.w, h: CTL };
    // The label follows the row, not a control the row turned off inside
    // (the Camera switch off greys its device list, never "Camera").
    let on = ui.enabled;
    let free = ui.area(ctl, |ui| {
        ui.lay.gap = 12.0;
        ui.row_rtl(|ui| {
            f(ui);
            ui.row_free()
        })
    });
    ui.enabled = on;
    let lw = (free - 12.0 - 2.0).max(40.0);
    let k = if on { 1.0 } else { 0.4 };
    let max_lines = (((h - 16.0) / 13.0).floor() as usize).max(1);
    let lines: Vec<String> = match help {
        Some(t) => {
            let mut l = wrap_at_breaks(t, lw, |s| ui.text_w(11.0, s));
            if l.len() > max_lines {
                l.truncate(max_lines);
                let last = l.pop().unwrap_or_default();
                l.push(ui.fit_text(11.0, &format!("{last}\u{2026}"), lw));
            }
            l
        }
        None => Vec::new(),
    };
    let block = 15.0 + if lines.is_empty() { 0.0 } else { 1.0 + 13.0 * lines.len() as f32 };
    let top = cy - block / 2.0;
    let x = ui.px(b.x + 2.0);
    let label = ui.fit_text(12.5, label, lw);
    ui.text_at(12.5, &label, x, ui.px(top + 7.5), tok.tx.fade(k), false);
    let hc = if help_err { tok.er } else { tok.tx2 };
    for (i, l) in lines.iter().enumerate() {
        ui.text_at(11.0, l, x, ui.px(top + 16.0 + 6.5 + 13.0 * i as f32), hc.fade(k), false);
    }
    ui.space(h + 2.0);
}

/// `t` in lines of at most `w` (in `measure`'s unit), broken after
/// spaces, `-`, `_` and `.` (file names wrap like the board's example),
/// or anywhere in a piece wider than a line.
pub fn wrap_at_breaks(t: &str, w: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in t.chars() {
        cur.push(ch);
        if matches!(ch, ' ' | '-' | '_' | '.') {
            pieces.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    let mut lines = vec![String::new()];
    for p in pieces {
        let line = lines.last_mut().expect("a line");
        let joined = format!("{line}{p}");
        if line.is_empty() || measure(joined.trim_end()) <= w {
            *line = joined;
        } else {
            lines.push(p);
        }
    }
    let lines: Vec<String> = lines.into_iter().map(|l| l.trim_end().to_string()).collect();
    lines.into_iter().flat_map(|l| if measure(&l) <= w { vec![l] } else { wrap(&l, w, &measure) }).collect()
}

/// [`srow_ex`] 36 px tall.
pub fn srow(ui: &mut Ui, tok: &Tok, label: &str, help: Option<&str>, f: impl FnOnce(&mut Ui)) {
    srow_ex(ui, tok, label, help, false, ROW_H, f);
}

/// A one-line note under a row, right-aligned under its controls (or
/// left under a key-binding row: `left`), with the info glyph when
/// `err`.
pub fn note_line(ui: &mut Ui, tok: &Tok, text: &str, err: bool, left: Option<f32>) {
    let b = ui.bounds();
    let y = ui.cursor_y();
    let c = if err { tok.er } else { tok.tx2 };
    let cy = ui.px(y + 7.0);
    let fit = ui.fit_text(11.0, text, b.w - 30.0);
    let tw = ui.text_w(11.0, &fit);
    let x = match left {
        Some(l) => b.x + l,
        None => b.x1() - tw - if err { 19.0 } else { 0.0 },
    };
    let mut tx = ui.px(x);
    if err {
        let s = ui.px(13.0);
        crate::icon_path::draw_icon(&mut ui.fb, "info", tx, (cy - s / 2.0).round(), s, c);
        tx += ui.px(19.0);
    }
    ui.text_at(11.0, &fit, tx, cy, c, false);
    ui.space(18.0 + 2.0);
}

/// The Win+Shift+S row's state line, right-aligned like [`note_line`]:
/// green when it opens Rustshot, red (a link that opens the guide) when
/// it is set to a missing app. Returns whether that link was clicked.
pub fn screenclip_line(ui: &mut Ui, tok: &Tok, text: &str, tone: crate::screenclip_win::Tone) -> bool {
    use crate::screenclip_win::Tone;
    let b = ui.bounds();
    let y = ui.cursor_y();
    let fit = ui.fit_text(11.0, text, b.w - 30.0);
    let tw = ui.text_w(11.0, &fit).ceil();
    let mut clicked = false;
    match tone {
        Tone::Bad => {
            let saved = ui.ctl_h;
            ui.ctl_h = 14.0;
            clicked = ui.place(FRect { x: b.x1() - tw, y, w: tw, h: 14.0 }).clink_in("screenclip_fix", &fit, 11.0, false, tok.er);
            ui.ctl_h = saved;
        }
        Tone::Good | Tone::Plain => {
            let c = if tone == Tone::Good { ui.theme.success } else { tok.tx2 };
            let (x, cy) = (ui.px(b.x1() - tw), ui.px(y + 7.0));
            ui.text_at(11.0, &fit, x, cy, c, false);
        }
    }
    ui.space(18.0 + 2.0);
    clicked
}

/// A one-line note under a row's control, right-aligned like
/// [`note_line`], that ends in off-site link `label` (Upload's "Get a
/// key"). Returns whether the link was clicked.
pub fn note_link_line(ui: &mut Ui, tok: &Tok, id: &str, text: &str, label: &str) -> bool {
    let b = ui.bounds();
    let y = ui.cursor_y();
    let (gap, lw) = (6.0, (ui.text_w(11.0, label) + 19.0).ceil());
    let fit = ui.fit_text(11.0, text, (b.w - lw - gap).max(40.0));
    let tw = ui.text_w(11.0, &fit).ceil();
    let (tx, cy) = (ui.px(b.x1() - lw - gap - tw), ui.px(y + 7.0));
    ui.text_at(11.0, &fit, tx, cy, tok.tx2, false);
    let saved = ui.ctl_h;
    ui.ctl_h = 14.0;
    let clicked = ui.place(FRect { x: b.x1() - lw, y, w: lw, h: 14.0 }).clink(id, label, 11.0, true, tok);
    ui.ctl_h = saved;
    ui.space(18.0 + 2.0);
    clicked
}


/// A section heading indented `x` (Key bindings: 8).
fn sec_at(ui: &mut Ui, tok: &Tok, s: &str, x: f32) {
    if x <= 0.0 {
        ui.sec(s, tok);
    } else {
        let b = ui.bounds();
        let y = ui.cursor_y();
        ui.area(FRect { x: b.x + x, y, w: b.w - x, h: 33.0 }, |ui| ui.sec(s, tok));
        ui.space(33.0 + 2.0);
    }
}

/// One frame of the window; returns what was clicked.
pub fn paint(ui: &mut Ui, s: &mut Settings) -> Option<Click> {
    let th = *ui.theme;
    let tok = Tok::of(&th);
    let mut click = None;
    let modal = s.modal.clone();
    ui.enabled = modal.is_none();
    ui.text_px = 12.0 * CSS;
    ui.radius = 7.0;
    ui.ctl_h = CTL;
    ui.field_fill = Some(if th.dark { tok.sf2 } else { C4::rgb(255, 255, 255) });
    let (w, h) = (W as f32, H as f32);
    fill_rect(ui, FRect { x: 0.0, y: 0.0, w, h }, tok.win);

    // The rail.
    fill_rect(ui, FRect { x: 0.0, y: 0.0, w: RAIL_W, h }, tok.side);
    line(ui, RAIL_W - 1.0, 0.0, 1.0, h, tok.sep);
    let items: Vec<(&str, &str)> = SECTIONS.iter().map(|(i, k)| (*i, k.get())).collect();
    ui.rail("rail", &items, ABOUT, rail_area(), &mut s.section, &RAIL, &tok);

    // A form that never saw the file (it did not parse) is not shown.
    if !s.stale {
        body(ui, s, &tok, &mut click);
        header(ui, s, &tok, &mut click);
        footer(ui, s, &tok, &mut click);
    }

    if let Some(m) = modal {
        ui.enabled = true;
        ui.text_px = 13.0;
        ui.radius = 8.0;
        ui.ctl_h = crate::ui::layout::H;
        ui.field_fill = None;
        if let Some(c) = question(ui, s, &m) {
            click = Some(c);
        }
    }
    toast(ui, s);
    click
}

/// The toast over the footer ([`Settings::toast`]): fades in, holds,
/// fades out.
fn toast(ui: &mut Ui, s: &Settings) {
    let Some((msg, at)) = &s.toast else { return };
    let age = at.elapsed().as_secs_f32() * 1000.0;
    let k = crate::anim::ease_out(age / 120.0).min((1.0 - (age - (TOAST_MS as f32 - 150.0)) / 150.0).clamp(0.0, 1.0));
    toast_at(ui, msg, k);
}

/// The toast with opacity `k` (tests draw it fully shown).
pub fn toast_at(ui: &mut Ui, msg: &str, k: f32) {
    if k <= 0.0 {
        return;
    }
    let cu = crate::editor::style::Ui { th: ui.theme, s: ui.k, font: Some(&crate::fonts::UI) };
    let area = phys(ui, FRect { x: RAIL_W, y: 0.0, w: W as f32 - RAIL_W, h: H as f32 - FOOT_H + 16.0 });
    // The All keys warning is no success.
    let kind = if *msg == crate::rec::marks::text::all_keys_warning() { crate::editor::style::ToastKind::Info } else { crate::editor::style::ToastKind::Success };
    crate::editor::style::toast(&mut ui.fb, &cu, msg, kind, area, k);
}

/// The section body: every section fits between the header and the
/// footer (`every_section_fits`), so nothing scrolls; its height is kept
/// for that test.
fn body(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let vp = viewport();
    let sec = s.section;
    let x0 = if is_keys(sec) { KEYS_PAD_X } else { PAD_X };
    let top = HDR_H + BODY_TOP;
    let area = FRect { x: RAIL_W + x0, y: top, w: vp.w - 2.0 * x0, h: 4000.0 };
    // (Anything past the body would be under the footer: not clickable.)
    ui.input_clip = Some(phys(ui, vp));
    // "Change shortcuts…" in the hotkey dialog: that row's box records
    // (`change_shortcuts` opened the section that lists it).
    if is_keys(sec)
        && ui.enabled
        && let Some(id) = s.record_field.take()
    {
        ui.focus.record(&id);
    }
    let end = ui.area(area, |ui| {
        ui.lay.gap = 2.0;
        match sec {
            GENERAL => general(ui, s, tok),
            CAPTURE => capture(ui, s, tok, click),
            OVERLAY => overlay(ui, s, tok, click),
            SAVING => saving(ui, s, tok, click),
            FILE_NAMES => file_names(ui, s, tok, click),
            HISTORY => history(ui, s, tok, click),
            RECORDING => recording::section(ui, s, tok),
            CAMERA => recording::camera(ui, s, tok),
            UPLOAD => super::upload::section(ui, s, tok, click),
            S3_STORAGE => super::upload::s3_section(ui, s, tok, click),
            KEYS_GLOBAL => keys_global(ui, s, tok),
            KEYS_RECORDING => keys_recording(ui, s, tok),
            KEYS_TOOLS => overlay_keys(ui, s, tok, &TOOL_ROWS),
            KEYS_ACTIONS => overlay_keys(ui, s, tok, &ACTION_ROWS),
            _ => about(ui, tok, click),
        }
        ui.cursor_y()
    });
    ui.input_clip = None;
    s.content_h[sec] = end - 2.0 - top + BODY_TOP + BODY_BOTTOM;
}

fn header(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let w = W as f32 - RAIL_W;
    fill_rect(ui, FRect { x: RAIL_W, y: 0.0, w, h: HDR_H }, tok.win);
    line(ui, RAIL_W, HDR_H - 1.0, w, 1.0, tok.sep);
    let (x, cy) = (ui.px(RAIL_W + PAD_X), ui.px(HDR_H / 2.0));
    // The title ends 12 px before the Reset button: a size smaller when
    // long, then with the button as its icon alone, cut only after that.
    let keys = is_keys(s.section);
    let full = if keys { ui.cbutton_width(tr!("settings.header.reset_to_defaults"), true) } else { 0.0 };
    let room = |bw: f32| W as f32 - PAD_X - bw - 12.0 - (RAIL_W + PAD_X);
    let t = &TITLES[s.section];
    let (size, bw) = if ui.text_w(14.0, t) <= room(full) {
        (14.0, full)
    } else if ui.text_w(12.5, t) <= room(full) || !keys {
        (12.5, full)
    } else {
        (12.5, 32.0)
    };
    let title = ui.fit_text(size, t, room(bw));
    ui.text_at(size, &title, x, cy, tok.tx, true);
    if keys {
        let label = if bw == full { tr!("settings.header.reset_to_defaults") } else { "" };
        let r = FRect { x: W as f32 - PAD_X - bw, y: (HDR_H - CTL) / 2.0, w: bw, h: CTL };
        if ui.place(r).cbutton("reset_all", label, Some("set-rotate"), false, tok) {
            *click = Some(Click::ResetAll);
        }
    }
}

fn footer(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let (w, y) = (W as f32 - RAIL_W, H as f32 - FOOT_H);
    fill_rect(ui, FRect { x: RAIL_W, y, w, h: FOOT_H }, tok.win);
    line(ui, RAIL_W, y, w, 1.0, tok.sep);
    let bar = FRect { x: RAIL_W + PAD_X, y: y + (FOOT_H - CTL) / 2.0, w: w - 2.0 * PAD_X, h: CTL };
    let dirty = s.dirty() || s.retry_hotkeys();
    let btns: f32 = [tr!("common.apply"), tr!("common.cancel"), tr!("common.ok")].iter().map(|l| ui.cbutton_width(l, false).max(72.0)).sum::<f32>() + 2.0 * 8.0;
    // The link takes what the buttons leave (cut with "…" past that).
    let link = ui.fit_text(12.0, tr!("settings.footer.open_config_file"), (bar.w - btns - 16.0).max(40.0));
    ui.area(bar, |ui| {
        ui.row(|ui| {
            if ui.clink("open_config", &link, 12.0, false, tok) {
                *click = Some(Click::OpenConfig);
            }
        })
    });
    ui.area(bar, |ui| {
        ui.lay.gap = 8.0;
        ui.row_rtl(|ui| {
            let on = ui.enabled;
            for (id, label, c) in [("apply", tr!("common.apply"), Click::Apply), ("cancel", tr!("common.cancel"), Click::Cancel), ("ok", tr!("common.ok"), Click::Ok)] {
                ui.enabled = on && (c != Click::Apply || dirty);
                let w = ui.cbutton_width(label, false).max(72.0);
                if ui.width(w).cbutton(id, label, None, c == Click::Ok, tok) {
                    *click = Some(c);
                }
            }
            ui.enabled = on;
        })
    });
}

/// What a [`dialog`] shows: the window's one card style for questions,
/// errors (a hotkey that did not register) and messages.
pub struct Dialog<'a> {
    pub icon: &'a str,
    pub icon_color: C4,
    /// A bold line over the text.
    pub title: Option<&'a str>,
    pub text: &'a str,
    /// (id, label, click); the first is the default (primary) one.
    pub buttons: &'a [(&'a str, &'a str, Click)],
    /// A checkbox left of the buttons: (id, label, ticked, its click).
    pub check: Option<(&'a str, &'a str, bool, Click)>,
    /// The card's least width (logical px).
    pub min_w: f32,
    /// The tallest the text may grow before it is cut.
    pub max_text_h: f32,
    /// A red line under the text (something the dialog's button could not do).
    pub error: Option<&'a str>,
}

/// Paint `d` centred over the dimmed form; returns the click.
pub fn dialog(ui: &mut Ui, d: &Dialog) -> Option<Click> {
    use crate::ui::layout::{GAP, H as QROW};
    let b = ui.bounds();
    let tok = Tok::of(ui.theme);
    let (cp, icon_w) = (24.0f32, 24.0 + 12.0);
    let check_w = d.check.map_or(0.0, |(_, l, _, _)| 16.0 + 8.0 + ui.text_w(12.0, l) + 2.0 * GAP);
    let total: f32 = d.buttons.iter().map(|b| ui.button_width(b.1)).sum::<f32>() + GAP * (d.buttons.len() as f32 - 1.0);
    let cw = (total + check_w + 2.0 * cp).max(d.min_w);
    let text_w = cw - 2.0 * cp - icon_w;
    let title_h = if d.title.is_some() { 24.0 } else { 0.0 };
    let th = ui.paragraph_height(d.text, text_w).clamp(24.0, d.max_text_h);
    let err_lines = d.error.map_or(Vec::new(), |e| wrap(e, text_w, |s| ui.text_w(12.0, s)));
    let err_h = if err_lines.is_empty() { 0.0 } else { 8.0 + 16.0 * err_lines.len() as f32 };
    let ch = cp + title_h + th + err_h + 24.0 + QROW + cp;
    let card = FRect { x: ((b.w - cw) / 2.0).round(), y: ((b.h - ch) / 2.0).round(), w: cw, h: ch };
    ui.modal_card(card);
    ui.area(FRect { x: card.x + cp, y: card.y + cp, w: 24.0, h: 24.0 }, |ui| ui.icon(d.icon, 24.0, d.icon_color));
    let tx = card.x + cp + icon_w;
    if let Some(t) = d.title {
        let fit = ui.fit_text(14.0, t, text_w);
        let (x, cy) = (ui.px(tx), ui.px(card.y + cp + 12.0));
        ui.text_at(14.0, &fit, x, cy, tok.tx, true);
    }
    let ty = card.y + cp + title_h + if title_h == 0.0 && th <= 24.0 { 2.0 } else { 0.0 };
    ui.area(FRect { x: tx, y: ty, w: text_w, h: th }, |ui| ui.height(th).paragraph(d.text, false));
    for (i, l) in err_lines.iter().enumerate() {
        let (x, cy) = (ui.px(tx), ui.px(ty + th + 8.0 + 8.0 + 16.0 * i as f32));
        ui.text_at(12.0, l, x, cy, tok.er, false);
    }
    let bar = FRect { x: card.x + cp, y: card.y1() - cp - QROW, w: cw - 2.0 * cp, h: QROW };
    let mut click = None;
    if let Some((id, label, mut on, c)) = d.check {
        ui.area(bar, |ui| {
            ui.row(|ui| {
                if ui.checkbox(id, label, &mut on, &tok) {
                    click = Some(c);
                }
            })
        });
    }
    ui.area(bar, |ui| {
        ui.row(|ui| {
            ui.space(bar.w - total);
            for (i, &(id, label, c)) in d.buttons.iter().enumerate() {
                if ui.button(id, label, i == 0) {
                    click = Some(c);
                }
            }
        })
    });
    click
}

/// The question, error, broken-file state or guide, as a [`dialog`].
fn question(ui: &mut Ui, s: &Settings, m: &Modal) -> Option<Click> {
    if let Modal::Guide { broken, dont_show } = *m {
        use crate::screenclip_win::{GUIDE_DONT_SHOW, GUIDE_LATER, GUIDE_OPEN};
        let (title, text) = s.guide_texts(broken);
        let icon_color = if broken { ui.theme.error } else { ui.theme.accent_fg };
        return dialog(
            ui,
            &Dialog {
                icon: if broken { "alert" } else { "info" },
                icon_color,
                title: Some(title),
                text: &text,
                buttons: &[("q_open_default_apps", GUIDE_OPEN.get(), Click::OpenDefaultApps), ("q_later", GUIDE_LATER.get(), Click::RemindLater)],
                check: Some(("q_dont_show", GUIDE_DONT_SHOW.get(), dont_show, Click::DontShow)),
                min_w: 500.0,
                max_text_h: 260.0,
                error: s.guide_error.as_deref(),
            },
        );
    }
    // The tray-icon-off question's text, and whether Win+Shift+S still
    // needs setting up (its "Set up…").
    let (text_for_tray, setup) = (s.hide_tray_text(), s.screenclip_ready() == Some(false));
    type Btns = Vec<(&'static str, &'static str, Click)>;
    let (icon, text, btns): (&str, String, Btns) = match m {
        Modal::Guide { .. } => unreachable!("drawn above"),
        Modal::Rewrite { .. } => {
            ("info", REWRITE.get().into(), vec![("q_save", tr!("common.save"), Click::Save), ("q_cancel", tr!("common.cancel"), Click::Back)])
        }
        Modal::Error(e) => ("alert", e.clone(), vec![("q_ok", tr!("common.ok"), Click::Back)]),
        Modal::HideTray if setup => (
            "info",
            text_for_tray,
            vec![("q_hide", tr!("settings.q.turn_off"), Click::HideTray), ("q_setup", tr!("settings.q.set_up"), Click::ScreenclipSetup), ("q_cancel", tr!("common.cancel"), Click::KeepTray)],
        ),
        Modal::HideTray => ("info", text_for_tray, vec![("q_hide", tr!("settings.q.turn_off"), Click::HideTray), ("q_cancel", tr!("common.cancel"), Click::KeepTray)]),
        Modal::TrayOffNotice => (
            "info",
            TRAY_OFF_NOTICE.get().into(),
            vec![("q_setup", tr!("settings.q.set_up_screenclip"), Click::ScreenclipSetup), ("q_tray_on", tr!("settings.q.show_tray_again"), Click::ShowTrayAgain), ("q_ok", tr!("common.ok"), Click::NoticeOk)],
        ),
        Modal::Hotkeys { text, .. } => (
            "alert",
            text.clone(),
            vec![("q_ok", tr!("common.ok"), Click::Back), ("q_change", tr!("settings.q.change_shortcuts"), Click::ChangeShortcuts)],
        ),
        Modal::Broken(e) => (
            "alert",
            broken_text(e),
            vec![
                ("q_open", tr!("settings.footer.open_config_file"), Click::OpenConfig),
                ("q_reset", tr!("settings.q.reset_to_defaults"), Click::ResetDefaults),
                ("q_close", tr!("common.close"), Click::Close),
            ],
        ),
        Modal::Reset(_) => ("alert", RESET.get().into(), vec![("q_replace", tr!("settings.q.replace"), Click::Replace), ("q_cancel", tr!("common.cancel"), Click::Back)]),
    };
    let icon_color = if matches!(m, Modal::Rewrite { .. } | Modal::HideTray | Modal::TrayOffNotice) { ui.theme.accent_fg } else { ui.theme.error };
    dialog(ui, &Dialog { icon, icon_color, title: None, text: &text, buttons: &btns, check: None, min_w: 420.0, max_text_h: 160.0, error: None })
}


// --- Sections ---------------------------------------------------------------

fn labels<T, const N: usize>(list: &[(Key, T); N]) -> [&'static str; N] {
    std::array::from_fn(|i| list[i].0.get())
}

/// General: how the app looks and runs.
fn general(ui: &mut Ui, s: &mut Settings, tok: &Tok) {
    let tray_row = s.tray_row;
    let warn = s.tray_warning();
    let tray_help = s.tray_help();
    let f = &mut s.form;
    ui.sec(tr!("settings.general.appearance"), tok);
    srow(ui, tok, tr!("settings.general.theme"), None, |ui| {
        ui.seg("theme", &labels(&THEMES), &mut f.theme, tok);
    });
    // The picked language (System default: the system's), downloaded when
    // it is not here yet: its state under the row, Retry after a failure.
    let choices = language_choices();
    let code = choices.get(f.language).map_or("", |c| c.0);
    let code = if code.is_empty() { crate::i18n::system_language() } else { code };
    let (help, err, retry) = match crate::i18n::fetch::status(code) {
        crate::i18n::fetch::Status::Downloading => (Some(tr!("settings.general.language.downloading").to_string()), false, false),
        crate::i18n::fetch::Status::Failed(e) => (Some(tr!("settings.general.language.failed", e)), true, true),
        _ if f.language == 0 => (Some(tr!("settings.general.language.system_is", name = crate::i18n::name_of(code))), false, false),
        _ => (None, false, false),
    };
    let mut picked = false;
    let mut again = false;
    srow_ex(ui, tok, tr!("settings.general.language"), help.as_deref(), err, ROW_H, |ui| {
        let names: Vec<&str> = choices.iter().map(|l| l.1).collect();
        picked = ui.width(FIELD_W).dropdown("language", &names, &mut f.language);
        if retry {
            again = ui.cbutton("language_retry", tr!("settings.general.language.retry"), None, false, tok);
        }
    });
    if picked || again {
        let code = choices.get(f.language).map_or("", |c| c.0);
        let code = if code.is_empty() { crate::i18n::system_language() } else { code };
        crate::i18n::fetch::request(code);
    }
    srow(ui, tok, tr!("settings.general.accent"), Some(tr!("settings.general.accent.help")), |ui| {
        ui.lay.gap = 10.0;
        let accents = crate::theme::ACCENTS;
        let custom = config::parse_color(f.accent_hex.text.trim()).map(|(r, g, b, _)| C4::rgb(r, g, b));
        if ui.swatch("accent_custom", custom, f.accent == ACCENT_CUSTOM, tok) {
            f.accent = ACCENT_CUSTOM;
            if f.accent_hex.text.trim().is_empty() {
                f.accent_hex.set("#8b93ff");
            }
            ui.focus.focus("accent_hex");
        }
        for (i, a) in accents.iter().enumerate().rev() {
            if ui.swatch(&format!("accent_{}", a.0.to_lowercase()), Some(a.1), f.accent == i + 1, tok) {
                f.accent = i + 1;
            }
        }
        let peri = crate::theme::DARK.accent;
        if ui.swatch("accent_periwinkle", Some(peri), f.accent == 0, tok) {
            f.accent = 0;
        }
        if f.accent == ACCENT_CUSTOM {
            ui.space(4.0);
            ui.width(92.0).text_field("accent_hex", &mut f.accent_hex);
        }
    });
    ui.sec(tr!("settings.general.behavior"), tok);
    if let Some(on) = f.autostart.as_mut() {
        srow(ui, tok, tr!("settings.general.launch_at_login"), None, |ui| {
            ui.switch("autostart", on, tok);
        });
    }
    let mut hid = false;
    if tray_row {
        // Windows: off is no background process at all.
        let help = if cfg!(windows) { Some(tray_help) } else { warn.then_some(tr!("settings.general.no_capture_hotkey")) };
        // (Windows: two lines of plain help; elsewhere the warning, red.)
        let (err, h) = if cfg!(windows) { (false, 44.0) } else { (true, ROW_H) };
        srow_ex(ui, tok, tr!("settings.general.show_icon_in_tray_menu"), help, err, h, |ui| {
            hid = ui.switch("show_tray", &mut f.show_tray, tok) && !f.show_tray;
        });
    }
    ui.sec(tr!("settings.general.updates"), tok);
    srow(ui, tok, tr!("settings.general.check_for_updates_automatically"), Some(tr!("settings.general.check_for_updates_automatically.help")), |ui| {
        ui.switch("check_updates", &mut f.check_updates, tok);
    });
    srow(ui, tok, tr!("settings.general.download_data"), Some(tr!("settings.general.download_data.help")), |ui| {
        // Applied when saved (Cancel leaves downloads as they were).
        ui.switch("download_data", &mut f.download_data, tok);
    });
    // Turned off: say how to get back before it goes.
    if hid {
        s.ask_hide_tray();
    }
}

/// Capture: how a capture starts and selects, and what follows it.
fn capture(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    // "Set up…" (tray icon off): the Win+Shift+S row, focused.
    if ui.enabled
        && let Some(id) = s.focus_field.take()
    {
        ui.focus.focus(&id);
    }
    let wayland = wayland();
    // Which app Win+Shift+S opens, once the toggle is saved on. With the
    // tray icon off, red unless it is Rustshot (no other way in), even
    // while the toggle is off: its click turns it on.
    let registered = s.screenclip == Some(true);
    let clip_line = if !s.form.show_tray {
        let h = if registered { s.handler.clone() } else { Handler::None };
        Some(crate::screenclip_win::state_line(&h, false))
    } else {
        (registered && s.form.screenclip == Some(true)).then(|| crate::screenclip_win::state_line(&s.handler, true))
    };
    let f = &mut s.form;
    ui.sec(tr!("settings.capture.selecting"), tok);
    if let Some(on) = f.screenclip.as_mut() {
        srow_ex(ui, tok, tr!("settings.capture.use_win_shift_s_and"), Some(SCREENCLIP_HELP.get()), false, ROW_H, |ui| {
            ui.switch("screenclip", on, tok);
        });
        if let Some((text, tone)) = &clip_line
            && screenclip_line(ui, tok, text, *tone)
        {
            *click = Some(Click::FixScreenclip);
        }
    }
    let help = if wayland { tr!("settings.capture.not_on_wayland.help") } else { tr!("settings.capture.snap_to_windows.help") };
    srow(ui, tok, tr!("settings.capture.snap_to_windows"), Some(help), |ui| {
        ui.switch("snap_windows", &mut f.snap, tok);
    });
    // (Two helper lines: the segmented control leaves little room.)
    srow_ex(ui, tok, tr!("settings.capture.accept_on_select"), Some(tr!("settings.capture.accept_on_select.help")), false, 44.0, |ui| {
        ui.seg("accept_on_select", &labels(&ACCEPTS), &mut f.accept, tok);
    });
    ui.sec(tr!("settings.capture.after_capture"), tok);
    srow(ui, tok, tr!("settings.capture.copy_to_clipboard_after_capture"), Some(tr!("settings.capture.copy_to_clipboard_after_capture.help")), |ui| {
        ui.switch("copy_after_capture", &mut f.copy_after, tok);
    });
    srow(ui, tok, tr!("settings.capture.play_shutter_sound"), None, |ui| {
        ui.switch("shutter_sound", &mut f.shutter, tok);
    });
    srow(ui, tok, tr!("settings.capture.show_thumbnail_after_capture"), None, |ui| {
        ui.switch("show_thumbnail", &mut f.thumbnail, tok);
    });
    let by_hand = f.keys.chords(Action::Trim).first().map_or_else(|| tr!("settings.capture.trim.by_menu").to_string(), |c| c.label());
    let help = tr!("settings.capture.trim.help", by_hand);
    srow(ui, tok, tr!("settings.capture.trim_margins_automatically"), Some(&help), |ui| {
        ui.switch("trim_margins", &mut f.trim, tok);
    });
    srow(ui, tok, tr!("settings.capture.include_pointer"), Some(tr!("settings.capture.include_pointer_help")), |ui| {
        ui.switch("include_cursor", &mut f.cursor, tok);
    });
}

/// Overlay: what shows while selecting, and how the toolbar draws.
fn overlay(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let show_renderer = s.renderer;
    let done = s.tools_reset;
    let f = &mut s.form;
    ui.sec(tr!("settings.overlay.while_selecting"), tok);
    srow(ui, tok, tr!("settings.overlay.show_magnifier"), Some(tr!("settings.overlay.show_magnifier.help")), |ui| {
        ui.switch("magnifier", &mut f.magnifier, tok);
    });
    let on = ui.enabled;
    ui.enabled = on && f.magnifier;
    srow(ui, tok, tr!("settings.overlay.square_magnifier"), Some(tr!("settings.overlay.square_magnifier.help")), |ui| {
        ui.switch("magnifier_square", &mut f.magnifier_square, tok);
    });
    ui.enabled = on;
    srow(ui, tok, tr!("settings.overlay.show_hints_in_the_overlay"), Some(tr!("settings.overlay.show_hints_in_the_overlay.help")), |ui| {
        ui.switch("overlay_hints", &mut f.hints, tok);
    });
    ui.sec(tr!("settings.overlay.toolbar"), tok);
    srow_ex(ui, tok, tr!("settings.overlay.copy_text"), Some(tr!("settings.overlay.copy_text.help")), false, 44.0, |ui| {
        ui.seg("copy_text_mode", &labels(&COPY_TEXT), &mut f.copy_text, tok);
    });
    srow_ex(ui, tok, tr!("settings.overlay.redact_style"), Some(tr!("settings.overlay.redact_style.help")), false, 44.0, |ui| {
        ui.seg("redact_style", &labels(&REDACT_STYLES), &mut f.redact_style, tok);
    });
    srow(ui, tok, tr!("settings.overlay.ui_size"), Some(tr!("settings.overlay.ui_size.help")), |ui| {
        ui.width(120.0).dropdown("ui_size", &uisize::LABELS.map(|k| k.get()), &mut f.ui_size);
    });
    let help = if done { tr!("settings.overlay.done_the_next_capture_starts.help") } else { tr!("settings.overlay.tool_sizes.help") };
    srow(ui, tok, tr!("settings.overlay.tool_sizes_colors"), Some(help), |ui| {
        if ui.cbutton("reset_tools", tr!("settings.overlay.reset"), Some("set-rotate"), false, tok) {
            *click = Some(Click::ResetTools);
        }
    });
    if show_renderer {
        srow(ui, tok, tr!("settings.overlay.renderer"), Some(tr!("settings.overlay.renderer.help")), |ui| {
            ui.seg("renderer", &labels(&RENDERERS), &mut f.renderer, tok);
        });
    }
}

/// Saving: where captures go and in which format.
fn saving(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let folder_note = s.folder_note();
    let f = &mut s.form;
    ui.sec(tr!("settings.saving.folder"), tok);
    srow(ui, tok, tr!("settings.saving.save_to"), None, |ui| {
        if ui.cbutton("browse", tr!("settings.saving.change"), None, false, tok) {
            *click = Some(Click::Browse);
        }
        ui.field_icon = Some("set-folder");
        ui.width(FIELD_W).text_field("folder", &mut f.folder);
    });
    if let Some(n) = &folder_note {
        note_line(ui, tok, n, false, None);
    }
    srow(ui, tok, tr!("settings.saving.ask_where_to_save"), Some(tr!("settings.saving.ask_where_to_save.help")), |ui| {
        ui.switch("ask", &mut f.ask, tok);
    });
    ui.sec(tr!("settings.saving.format"), tok);
    srow(ui, tok, tr!("settings.saving.format"), None, |ui| {
        ui.seg("format_all", &FORMATS.map(|f| f.0), &mut f.format, tok);
    });
    let on = ui.enabled;
    ui.enabled = on && f.format == 1;
    srow(ui, tok, tr!("settings.saving.jpeg_quality"), None, |ui| {
        ui.width(200.0).slider("quality", &mut f.quality, 1, 100);
    });
    ui.enabled = on;
    srow(ui, tok, tr!("settings.saving.save_at"), Some(tr!("settings.saving.save_at.help")), |ui| {
        let on = ui.enabled;
        ui.enabled = on && f.save_at == 2;
        let w = ui.text_w(12.0, "%").ceil();
        let r = ui.alloc(Some(w), CTL);
        let c = if ui.enabled { tok.tx2 } else { tok.tx3 };
        ui.text_at(12.0, "%", r.x, r.y + r.h / 2.0, c, false);
        ui.width(110.0).slider("save_pct", &mut f.save_pct, 10, 100);
        ui.enabled = on;
        ui.seg("save_at", &labels(&SAVE_AT), &mut f.save_at, tok);
    });
    image_rows(ui, f, tok);
}

/// Saving › Format, after the format: the export border and HDR
/// screenshots (Windows).
fn image_rows(ui: &mut Ui, f: &mut crate::settings_ui::Form, tok: &Tok) {
    srow(ui, tok, tr!("settings.image_rows.add_a_border"), Some(tr!("settings.image_rows.add_a_border.help")), |ui| {
        ui.switch("border", &mut f.border, tok);
        let on = ui.enabled;
        ui.enabled = on && f.border;
        ui.width(120.0).slider("border_width", &mut f.border_w, 1, config::BORDER_MAX as u8);
        ui.space(4.0);
        ui.width(92.0).text_field("border_color", &mut f.border_hex);
        let c = config::parse_color(f.border_hex.text.trim()).map(|(r, g, b, _)| C4::rgb(r, g, b));
        if ui.swatch("border_swatch", c, false, tok) {
            ui.focus.focus("border_color");
        }
        ui.enabled = on;
    });
    // A PNG chosen in Edit › Effects wins over the text: say so, with a
    // way back to the text.
    let image = f.watermark_image.trim().to_string();
    let help = if image.is_empty() {
        tr!("settings.saving.watermark_help").to_string()
    } else {
        let name = std::path::Path::new(&image).file_name().map_or(image.clone(), |n| n.to_string_lossy().into_owned());
        match crate::imgedit::effects::image_problem(&image) {
            Some(p) => p,
            None => tr!("settings.saving.watermark_image", name),
        }
    };
    srow(ui, tok, tr!("settings.saving.watermark"), Some(&help), |ui| {
        ui.switch("export_watermark", &mut f.watermark, tok);
        let on = ui.enabled;
        ui.enabled = on && f.watermark;
        if image.is_empty() {
            ui.width(150.0).text_field("watermark_text", &mut f.watermark_text);
        } else if ui.cbutton("watermark_clear", tr!("settings.saving.watermark_clear"), None, false, tok) {
            f.watermark_image.clear();
        }
        ui.enabled = on;
    });
    if cfg!(windows) {
        srow(ui, tok, tr!("settings.image_rows.hdr_screenshots"), Some(tr!("settings.image_rows.hdr_screenshots.help")), |ui| {
            ui.seg("hdr_screenshots", &labels(&HDR_MODES), &mut f.hdr, tok);
        });
    }
}

/// File names: the daily subfolder and the file-name pattern, with the
/// token buttons that insert into either.
fn file_names(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let base = s.loaded.clone();
    let f = &mut s.form;
    let mut target = s.pattern;
    let mut token = None;
    ui.sec(tr!("settings.file_names.folders"), tok);
    srow(ui, tok, tr!("settings.file_names.daily_subfolders"), Some(tr!("settings.file_names.daily_subfolders.help")), |ui| {
        ui.switch("subfolder", &mut f.subfolder, tok);
        let on = ui.enabled;
        ui.enabled = on && f.subfolder;
        ui.width(160.0).text_field("subfolder_pattern", &mut f.subfolder_pattern);
        ui.enabled = on;
    });
    ui.sec(tr!("settings.file_names.file_name"), tok);
    srow(ui, tok, tr!("settings.file_names.pattern"), None, |ui| {
        if ui.cbutton("clear", tr!("settings.file_names.clear"), None, false, tok) {
            *click = Some(Click::Clear);
        }
        if ui.cbutton("restore", tr!("settings.file_names.restore"), None, false, tok) {
            *click = Some(Click::Restore);
        }
        let w = ui.row_free() - 100.0;
        ui.width(w).text_field("filename_pattern", &mut f.filename);
    });
    let path = f.preview(&base, export::local_ymdhms()).display().to_string();
    {
        let b = ui.bounds();
        let y = ui.cursor_y();
        let x = ui.px(b.x + 2.0);
        let label = &format!("{} ", tr!("settings.file_names.saves_as"));
        let lw = ui.text_w(11.0, label);
        ui.text_at(11.0, label, x, ui.px(y + 8.0), tok.tx2, false);
        let shown = fit_start(ui, &path, b.w - lw - 4.0);
        ui.text_at(11.0, &shown, x + ui.px(lw), ui.px(y + 8.0), tok.tx, false);
        ui.space(18.0 + 6.0);
    }
    let gy = ui.cursor_y();
    let b = ui.bounds();
    let cols = 3usize;
    let rows = TOKENS.len().div_ceil(cols);
    let (bh, gap) = (26.0, 6.0);
    let cw = ((b.w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let saved_h = ui.ctl_h;
    ui.ctl_h = bh;
    for (i, (label, _)) in TOKENS.iter().enumerate() {
        let (c, r) = ((i / rows) as f32, (i % rows) as f32);
        let rect = FRect { x: b.x + c * (cw + gap), y: gy + r * (bh + gap), w: cw, h: bh };
        if ui.place(rect).cbutton_ex(&format!("token{i}"), label, None, false, 11.0, tok) {
            token = Some(i);
        }
    }
    ui.ctl_h = saved_h;
    ui.space(rows as f32 * (bh + gap) - gap + 2.0);
    if ui.focus.is_focused("subfolder_pattern") {
        target = Pattern::Sub;
    } else if ui.focus.is_focused("filename_pattern") {
        target = Pattern::File;
    }
    if let Some(i) = token {
        if target == Pattern::Sub && !f.subfolder {
            target = Pattern::File;
        }
        f.insert(target, TOKENS[i].1);
        ui.focus.focus(if target == Pattern::Sub { "subfolder_pattern" } else { "filename_pattern" });
    }
    s.pattern = target;
}

/// History: what the History window keeps.
fn history(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let f = &mut s.form;
    ui.sec(tr!("settings.history.captures"), tok);
    srow(ui, tok, tr!("settings.history.keep_a_history_of_captures"), Some(tr!("settings.history.keep_a_history_of_captures.help")), |ui| {
        ui.switch("history", &mut f.history, tok);
    });
    let on = ui.enabled;
    ui.enabled = on && f.history;
    let days: Vec<&str> = HISTORY_DAYS.iter().map(|d| d.1.get()).collect();
    srow(ui, tok, tr!("settings.history.keep_for"), Some(tr!("settings.history.keep_for.help")), |ui| {
        ui.seg("history_days", &days, &mut f.history_days, tok);
    });
    let sizes: Vec<&str> = HISTORY_MB.iter().map(|d| d.1.get()).collect();
    srow(ui, tok, tr!("settings.history.up_to"), Some(tr!("settings.history.up_to.help")), |ui| {
        ui.seg("history_size", &sizes, &mut f.history_size, tok);
    });
    ui.enabled = on;
    srow(ui, tok, tr!("settings.history.clear_history"), Some(tr!("settings.history.clear_history.help")), |ui| {
        if ui.cbutton("clear_history", tr!("settings.history.clear_history_2"), None, false, tok) {
            *click = Some(Click::ClearHistory);
        }
    });
}

/// `s` cut at the start with an ellipsis to fit `w` (logical px) at 11 px.
fn fit_start(ui: &Ui, s: &str, w: f32) -> String {
    if ui.text_w(11.0, s) <= w {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() && ui.text_w(11.0, &format!("\u{2026}{}", chars.iter().collect::<String>())) > w {
        chars.remove(0);
    }
    format!("\u{2026}{}", chars.iter().collect::<String>())
}

fn wayland() -> bool {
    #[cfg(target_os = "linux")]
    return crate::rec::linux::portal::wayland_now();
    #[cfg(not(target_os = "linux"))]
    false
}

/// Shortcuts › Overlay tools: (label, actions).
pub const TOOL_ROWS: [(Key, &[Action]); 7] = [
    (
        key!("settings.keys.row.tools"),
        &[
            Action::ToolPencil,
            Action::ToolLine,
            Action::ToolArrow,
            Action::ToolRectangle,
            Action::ToolCircle,
            Action::ToolMarker,
            Action::ToolText,
            Action::ToolCounter,
            Action::ToolPixelate,
            Action::ToolBlur,
            Action::ToolInvert,
        ],
    ),
    (
        key!("settings.keys.row.select"),
        &[Action::ToolSelect, Action::ToolCallout, Action::ToolSpotlight, Action::ToolMagnify, Action::ToolRuler],
    ),
    (
        key!("settings.keys.row.objects"),
        &[Action::DeleteObject, Action::Duplicate, Action::BringFront, Action::SendBack],
    ),
    (key!("settings.keys.row.crop"), &[Action::ToolCrop, Action::Trim, Action::Redact, Action::SelectFreeform, Action::ReadQr]),
    (key!("settings.keys.row.size"), &[Action::SizeDown, Action::SizeUp, Action::TogglePalette]),
    (key!("settings.keys.row.typing"), &[Action::TextBold, Action::TextItalic, Action::TextUnderline]),
    (key!("settings.keys.row.undo"), &[Action::Undo, Action::Redo, Action::SelectAll]),
];

/// Shortcuts › Overlay actions: (label, actions). The Edit window's own
/// keys (zoom, image edits) end the list.
pub const ACTION_ROWS: [(Key, &[Action]); 7] = [
    (key!("settings.keys.row.copy"), &[Action::Copy, Action::Save, Action::Upload, Action::CopyText, Action::CopyColor]),
    (key!("settings.keys.row.save_as"), &[Action::SaveAs, Action::SaveDesktop, Action::OpenWith, Action::ScrollCapture]),
    SEND_ROW,
    (key!("settings.keys.row.ui_size"), &[Action::UiBigger, Action::UiSmaller, Action::UiReset]),
    (key!("settings.keys.row.zoom"), &[Action::ZoomIn, Action::ZoomOut, Action::ZoomFit, Action::ZoomActual]),
    (key!("settings.keys.row.rotate"), &[Action::RotateLeft, Action::RotateRight, Action::FlipHorizontal, Action::FlipVertical]),
    (key!("settings.keys.row.image"), &[Action::ImageResize, Action::ImageCanvas, Action::ImageEffects, Action::ImageReset]),
];

/// Share (with a share sheet: not on Linux), Pin to screen, Record, and
/// Accept · Cancel.
#[cfg(not(target_os = "linux"))]
const SEND_ROW: (Key, &[Action]) =
    (key!("settings.keys.row.send"), &[Action::Share, Action::Pin, Action::Record, Action::Accept, Action::Cancel]);
#[cfg(target_os = "linux")]
const SEND_ROW: (Key, &[Action]) = (key!("settings.keys.row.send_linux"), &[Action::Pin, Action::Record, Action::Accept, Action::Cancel]);

/// The while-recording rows of Shortcuts › Recording: (label, action).
pub const REC_ROWS: [(Key, Action); 3] = [
    (key!("hotkey.rec_pause"), Action::RecPause),
    (key!("hotkey.rec_mute"), Action::RecMute),
    (key!("hotkey.rec_hide"), Action::RecHidePill),
];

/// The global hotkey rows of `slots` ([`GLOBALS`] order), with their notes.
fn global_rows(ui: &mut Ui, s: &mut Settings, tok: &Tok, slots: &[usize]) {
    ui.allow_meta = true;
    for (label, slot) in GLOBALS.iter().filter(|(_, slot)| slots.contains(slot)) {
        // Read QR code shares Copy text's row (the section has no room
        // for a ninth): two caps, one note.
        if *slot == 8 && slots.contains(&6) {
            continue;
        }
        if *slot == 6 && slots.contains(&8) {
            let qr = tr!("action.read_qr");
            let msg = match (s.global_message(6), s.global_message(8)) {
                (Some(a), Some(b)) => Some(format!("{a} \u{b7} {qr}: {b}")),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(format!("{qr}: {b}")),
                (None, None) => None,
            };
            let (e6, e8) = (s.global_err(6), s.global_err(8));
            let (mut c6, mut c8) = (*s.form.global_mut(6), *s.form.global_mut(8));
            let mut pair = [Slot { id: "global6".into(), chord: &mut c6, err: e6 }, Slot { id: "global8".into(), chord: &mut c8, err: e8 }];
            ui.key_row(tr!("settings.keys.row.copy_text_qr"), &["Copy text from area", "Read QR code from area"], &mut pair, true, 34.0, tok);
            *s.form.global_mut(6) = c6;
            *s.form.global_mut(8) = c8;
            if let Some(m) = msg {
                note_line(ui, tok, &m, true, Some(8.0));
            }
            continue;
        }
        let msg = s.global_message(*slot);
        let err = s.global_err(*slot);
        let chord = s.form.global_mut(*slot);
        let mut slots = [Slot { id: format!("global{slot}"), chord, err }];
        ui.key_row(label.get(), &[], &mut slots, true, 34.0, tok);
        if let Some(m) = msg {
            note_line(ui, tok, &m, true, Some(8.0));
        }
    }
    ui.allow_meta = false;
}

/// Shortcuts › Global: the hotkeys that work anywhere (the record
/// hotkey is in Shortcuts › Recording).
fn keys_global(ui: &mut Ui, s: &mut Settings, tok: &Tok) {
    let slots: Vec<usize> = GLOBALS.iter().map(|(_, slot)| *slot).filter(|slot| section_of_slot(*slot) == KEYS_GLOBAL).collect();
    global_rows(ui, s, tok, &slots);
}

/// Shortcuts › Recording: the record hotkey, then the keys that work
/// anywhere while a recording runs (the pill never takes the keyboard;
/// stopping is the record hotkey).
fn keys_recording(ui: &mut Ui, s: &mut Settings, tok: &Tok) {
    sec_at(ui, tok, tr!("settings.keys_recording.start_stop"), 8.0);
    global_rows(ui, s, tok, &[2]);
    ui.allow_meta = true;
    sec_at(ui, tok, tr!("settings.keys_recording.while_recording"), 8.0);
    for (label, a) in REC_ROWS {
        let note = s.form.rec_note(a);
        let mut chord = s.form.keys.chords(a).first().copied();
        let changed = {
            let err = s.form.rec_err(a);
            let mut slots = [Slot { id: format!("key_{}", a.id()), chord: &mut chord, err }];
            ui.key_row(label.get(), &[], &mut slots, true, 34.0, tok)
        };
        if changed.is_some() {
            s.form.keys.set(a, chord.into_iter().collect());
        }
        if let Some(m) = note {
            note_line(ui, tok, &m, true, Some(8.0));
        }
    }
    ui.allow_meta = false;
}

/// Shortcuts › Overlay tools / Overlay actions: single keys while the
/// overlay is open, a conflict noted under its row.
fn overlay_keys(ui: &mut Ui, s: &mut Settings, tok: &Tok, rows: &[(Key, &[Action])]) {
    let bad = s.form.conflicting();
    for (label, acts) in rows {
        let mut chords: Vec<Option<Chord>> = acts.iter().map(|a| s.form.keys.chords(*a).first().copied()).collect();
        let names: Vec<&str> = acts.iter().map(|a| a.label()).collect();
        let changed = {
            let mut slots: Vec<Slot> = chords
                .iter_mut()
                .zip(acts.iter())
                .map(|(c, a)| Slot { id: format!("key_{}", a.id()), chord: c, err: bad.contains(a) })
                .collect();
            ui.key_row(label.get(), &names, &mut slots, false, 30.0, tok)
        };
        if let Some(i) = changed {
            s.form.keys.set(acts[i], chords[i].into_iter().collect());
        }
        if let Some(n) = acts.iter().find_map(|a| s.form.conflict_note(*a).map(|n| (a, n))).map(|(a, note)| tr!("settings.keys.conflict", action = a.label(), note)) {
            note_line(ui, tok, &n, true, Some(8.0));
        }
    }
}

/// The More from PowerNapps tiles: none until there are real apps (the
/// row is hidden while the list is empty).
pub const MORE_APPS: [(&str, &str, &str); 0] = [];

fn about(ui: &mut Ui, tok: &Tok, click: &mut Option<Click>) {
    let b = ui.bounds();
    let y = ui.cursor_y();
    // Hero: the 72 px mark, the name, the version line, the description.
    mark(ui, b.x, y + 18.0, 72.0);
    let tx = b.x + 72.0 + 18.0;
    let desc = tr!("settings.about.description");
    let dw = (b.x1() - tx).min(380.0);
    let lines = wrap(desc, dw, |t| ui.text_w(12.0, t));
    let block = 24.0 + 4.0 + 16.0 + 4.0 + 18.0 * lines.len() as f32;
    let top = y + 18.0 + (72.0 - block).max(0.0) / 2.0;
    let (px, py) = (ui.px(tx), ui.px(top + 12.0));
    ui.text_at(20.0, NAME, px, py, tok.tx, true);
    let ver = format!("{} ", tr!("settings.about.version", version = env!("CARGO_PKG_VERSION")));
    let vy = top + 24.0 + 4.0;
    ui.text_at(12.0, &ver, px, ui.px(vy + 8.0), tok.tx2, false);
    let vw = ui.text_w(12.0, &ver);
    let saved = ui.ctl_h;
    ui.ctl_h = 16.0;
    let link = FRect { x: tx + vw, y: vy, w: ui.text_w(12.0, tr!("settings.about.check_for_updates")) + 2.0, h: 16.0 };
    if ui.place(link).clink("check_updates_now", tr!("settings.about.check_for_updates"), 12.0, false, tok) {
        *click = Some(Click::CheckUpdates);
    }
    ui.ctl_h = saved;
    for (i, l) in lines.iter().enumerate() {
        ui.text_at(12.0, l, px, ui.px(vy + 16.0 + 4.0 + 9.0 + 18.0 * i as f32), tok.tx2, false);
    }
    ui.space(18.0 + 72.0f32.max(block) + 14.0);

    // The links, wrapping like the board's row; a line under them.
    let gap = 18.0;
    let mut x = 0.0;
    let mut rows: Vec<Vec<(usize, &str, &str, bool)>> = vec![Vec::new()];
    for (i, (label, url)) in crate::actions::LINKS.iter().take(5).map(|(k, u)| (k.get(), *u)).enumerate() {
        // Website, Source on GitHub, Report an issue: off-site.
        let ext = i < 3;
        let w = ui.text_w(12.5, label) + if ext { 19.0 } else { 0.0 };
        if x > 0.0 && x + w > b.w {
            rows.push(Vec::new());
            x = 0.0;
        }
        rows.last_mut().expect("a row").push((i, label, url, ext));
        x += w + gap;
    }
    ui.space(2.0);
    let (saved_gap, saved_h) = (ui.lay.gap, ui.ctl_h);
    ui.ctl_h = 16.0;
    for r in &rows {
        ui.lay.gap = gap;
        ui.row(|ui| {
            for (i, label, url, ext) in r {
                if ui.clink(&format!("link_{i}"), label, 12.5, *ext, tok) {
                    *click = Some(Click::Link(url));
                }
            }
        });
    }
    ui.lay.gap = saved_gap;
    ui.ctl_h = saved_h;
    let ly = ui.cursor_y() - gap + 12.0;
    line(ui, b.x, ly, b.w, 1.0, tok.sep);
    ui.space(12.0 - gap + 6.0);

    // More from PowerNapps, with its link on the right.
    let y = ui.cursor_y();
    ui.sec(tr!("settings.about.more_from_powernapps"), tok);
    let lw = ui.text_w(11.5, "powernapps.net") + 19.0;
    let r = FRect { x: b.x1() - lw, y: y + 14.0 + 6.5 - 8.0, w: lw, h: 16.0 };
    let saved = ui.ctl_h;
    ui.ctl_h = 16.0;
    if ui.place(r).clink("link_powernapps", "powernapps.net", 11.5, true, tok) {
        *click = Some(Click::Link(crate::actions::POWERNAPPS));
    }
    ui.ctl_h = saved;
    // (The tiles row: hidden while MORE_APPS is empty.)
    let _ = MORE_APPS.len();
    let y = ui.cursor_y();
    let (fx, fy) = (ui.px(b.x), ui.px(y + 14.0 + 6.0));
    // The UI font has no copyright sign: a ring with a small c.
    let r = ui.px(4.6);
    let (cx, c) = (fx + r, tok.tx3);
    ui.fb.stroke_circle(cx, fy, r, ui.px(0.9), c);
    let cw = ui.text_w(7.0, "c");
    ui.text_at(7.0, "c", cx - ui.px(cw) / 2.0, fy, c, false);
    let text = tr!("settings.about.footer");
    ui.text_at(11.0, text, fx + 2.0 * r + ui.px(4.0), fy, c, false);
    ui.space(14.0 + 14.0);
}
