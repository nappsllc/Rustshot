//! Painting the Settings window as on the Settings board: the 188 px
//! rail, a 48 px header, the scrolling section body (rows of the board's
//! `.r`: a 12.5/500 label with an 11/400 helper line, controls on the
//! right) and the footer bar with Open config file and OK / Cancel /
//! Apply; questions over the dimmed window.

use super::*;
use crate::ui::compact::{Slot, Tok, CSS};
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
pub const SCREENCLIP_HELP: &str = "Replaces the Snipping Tool; works even when Rustshot isn't running";
/// The field width of Save to and the like.
pub const FIELD_W: f32 = 228.0;

/// The section body's viewport (between the header and the footer).
pub fn viewport() -> FRect {
    FRect { x: RAIL_W, y: HDR_H, w: W as f32 - RAIL_W, h: H as f32 - HDR_H - FOOT_H }
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

/// The app mark (the board's `s-mark`) at `(x, y)`, `size` logical px.
fn mark(ui: &mut Ui, x: f32, y: f32, size: f32) {
    let r = phys(ui, FRect { x, y, w: size, h: size });
    let k = r.w / 24.0;
    ui.fb.fill_rounded(r.x as i32, r.y as i32, r.w as i32, r.h as i32, 5.4 * k, C4::rgb(0x1B, 0x1C, 0x20));
    ui.fb.stroke_rounded(FRect { x: r.x + 0.5 * k, y: r.y + 0.5 * k, w: r.w - k, h: r.h - k }, 5.0 * k, k.max(1.0), C4::new(255, 255, 255, 26));
    crate::icon_path::draw_icon(&mut ui.fb, "set-mark", r.x, r.y, r.w, C4::rgb(0xEC, 0xEE, 0xF3));
    ui.fb.fill_circle(r.x + 12.0 * k, r.y + 12.0 * k, 3.5 * k, C4::rgb(0x8B, 0x93, 0xFF));
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
    mark(ui, 18.0, 18.0, 20.0);
    let (tx, ty) = (ui.px(46.0), ui.px(28.0));
    ui.text_at(13.0, NAME, tx, ty, tok.tx, true);
    let before = s.section;
    let rail = FRect { x: 10.0, y: 52.0, w: RAIL_W - 20.0, h: h - 14.0 - 52.0 };
    ui.rail("rail", &SECTIONS, ABOUT, rail, &mut s.section, &tok);
    if s.section != before {
        s.scroll[s.section] = 0.0;
    }

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
    crate::editor::style::toast(&mut ui.fb, &cu, msg, crate::editor::style::ToastKind::Success, area, k);
}

/// The scrolled section body.
fn body(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let vp = viewport();
    let sec = s.section;
    let max = (s.content_h[sec] - vp.h).max(0.0);
    let pvp = phys(ui, vp);
    if ui.enabled && ui.input.wheel != 0 && crate::ui::contains(pvp, ui.input.mouse) {
        s.scroll[sec] -= ui.input.wheel as f32 / 120.0 * 48.0;
    }
    // (Not measured yet: the first frame keeps the offset asked for.)
    if s.content_h[sec] > 0.0 {
        s.scroll[sec] = s.scroll[sec].clamp(0.0, max);
    }
    let x0 = if sec == KEYS { KEYS_PAD_X } else { PAD_X };
    let top = HDR_H + BODY_TOP - s.scroll[sec];
    let area = FRect { x: RAIL_W + x0, y: top, w: vp.w - 2.0 * x0, h: 4000.0 };
    ui.input_clip = Some(pvp);
    ui.focus_rect = None;
    let end = ui.area(area, |ui| {
        ui.lay.gap = 2.0;
        match sec {
            GENERAL => general(ui, s, tok, click),
            CAPTURE => capture(ui, s, tok, click),
            RECORDING => recording::section(ui, s, tok),
            KEYS => keys(ui, s, tok),
            UPLOAD => super::upload::section(ui, s, tok, click),
            _ => about(ui, s, tok, click),
        }
        ui.cursor_y()
    });
    ui.input_clip = None;
    s.content_h[sec] = end - 2.0 - top + BODY_BOTTOM;
    // A control tabbed to out of view: scrolled in.
    if let Some(fr) = ui.focus_rect
        && fr.x >= ui.px(RAIL_W)
    {
        let (y0, y1) = (fr.y / ui.k, fr.y1() / ui.k);
        let (lo, hi) = (vp.y + 4.0, vp.y1() - 4.0);
        let d = if y0 < lo { y0 - lo } else if y1 > hi { y1 - hi } else { 0.0 };
        if d != 0.0 {
            s.scroll[sec] = (s.scroll[sec] + d).clamp(0.0, (s.content_h[sec] - vp.h).max(0.0));
            ui.request_redraw();
        }
    }
    // The scroll thumb.
    let max = (s.content_h[sec] - vp.h).max(0.0);
    if max > 0.0 {
        let th_h = (vp.h * vp.h / s.content_h[sec]).max(24.0);
        let y = vp.y + 2.0 + (vp.h - 4.0 - th_h) * (s.scroll[sec] / max);
        let r = phys(ui, FRect { x: W as f32 - 7.0, y, w: 4.0, h: th_h });
        ui.fb.fill_rounded(r.x as i32, r.y as i32, r.w as i32, r.h as i32, r.w / 2.0, tok.tx3.fade(0.5));
    }
}

fn header(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let w = W as f32 - RAIL_W;
    fill_rect(ui, FRect { x: RAIL_W, y: 0.0, w, h: HDR_H }, tok.win);
    line(ui, RAIL_W, HDR_H - 1.0, w, 1.0, tok.sep);
    let (x, cy) = (ui.px(RAIL_W + PAD_X), ui.px(HDR_H / 2.0));
    ui.text_at(14.0, SECTIONS[s.section].1, x, cy, tok.tx, true);
    if s.section == KEYS {
        let bw = ui.cbutton_width("Reset to defaults", true);
        let r = FRect { x: W as f32 - PAD_X - bw, y: (HDR_H - CTL) / 2.0, w: bw, h: CTL };
        if ui.place(r).cbutton("reset_all", "Reset to defaults", Some("set-rotate"), false, tok) {
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
    ui.area(bar, |ui| {
        ui.row(|ui| {
            if ui.clink("open_config", "Open config file", 12.0, false, tok) {
                *click = Some(Click::OpenConfig);
            }
        })
    });
    ui.area(bar, |ui| {
        ui.lay.gap = 8.0;
        ui.row_rtl(|ui| {
            let on = ui.enabled;
            for (id, label, c) in [("apply", "Apply", Click::Apply), ("cancel", "Cancel", Click::Cancel), ("ok", "OK", Click::Ok)] {
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
                buttons: &[("q_open_default_apps", GUIDE_OPEN, Click::OpenDefaultApps), ("q_later", GUIDE_LATER, Click::RemindLater)],
                check: Some(("q_dont_show", GUIDE_DONT_SHOW, dont_show, Click::DontShow)),
                min_w: 500.0,
                max_text_h: 260.0,
                error: s.guide_error.as_deref(),
            },
        );
    }
    // The tray-icon-off question's text, and whether Win+Shift+S still
    // needs setting up (its "Set up…").
    let (text_for_tray, setup) = (s.hide_tray_text(), s.screenclip_ready() == Some(false));
    type Btns = &'static [(&'static str, &'static str, Click)];
    let (icon, text, btns): (&str, String, Btns) = match m {
        Modal::Guide { .. } => unreachable!("drawn above"),
        Modal::Rewrite { .. } => {
            ("info", REWRITE.into(), &[("q_save", "Save", Click::Save), ("q_cancel", "Cancel", Click::Back)])
        }
        Modal::Error(e) => ("alert", e.clone(), &[("q_ok", "OK", Click::Back)]),
        Modal::HideTray if setup => (
            "info",
            text_for_tray,
            &[("q_hide", "Turn off", Click::HideTray), ("q_setup", "Set up\u{2026}", Click::ScreenclipSetup), ("q_cancel", "Cancel", Click::KeepTray)],
        ),
        Modal::HideTray => ("info", text_for_tray, &[("q_hide", "Turn off", Click::HideTray), ("q_cancel", "Cancel", Click::KeepTray)]),
        Modal::TrayOffNotice => (
            "info",
            TRAY_OFF_NOTICE.into(),
            &[("q_setup", "Set up Win+Shift+S", Click::ScreenclipSetup), ("q_tray_on", "Show tray icon again", Click::ShowTrayAgain), ("q_ok", "OK", Click::NoticeOk)],
        ),
        Modal::Hotkeys { text, .. } => (
            "alert",
            text.clone(),
            &[("q_ok", "OK", Click::Back), ("q_change", "Change shortcuts\u{2026}", Click::ChangeShortcuts)],
        ),
        Modal::Broken(e) => (
            "alert",
            broken_text(e),
            &[
                ("q_open", "Open config file", Click::OpenConfig),
                ("q_reset", "Reset to defaults…", Click::ResetDefaults),
                ("q_close", "Close", Click::Close),
            ],
        ),
        Modal::Reset(_) => ("alert", RESET.into(), &[("q_replace", "Replace", Click::Replace), ("q_cancel", "Cancel", Click::Back)]),
    };
    let icon_color = if matches!(m, Modal::Rewrite { .. } | Modal::HideTray | Modal::TrayOffNotice) { ui.theme.accent_fg } else { ui.theme.error };
    dialog(ui, &Dialog { icon, icon_color, title: None, text: &text, buttons: btns, check: None, min_w: 420.0, max_text_h: 160.0, error: None })
}

// --- Sections ---------------------------------------------------------------

fn labels<T, const N: usize>(list: &[(&'static str, T); N]) -> [&'static str; N] {
    std::array::from_fn(|i| list[i].0)
}

/// The file name the preview path ends in.
fn example_name(s: &Settings) -> String {
    let p = s.form.preview(&s.loaded, export::local_ymdhms());
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn general(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let folder_note = s.folder_note();
    let example = example_name(s);
    let tray_row = s.tray_row;
    let warn = s.tray_warning();
    let tray_help = s.tray_help();
    let f = &mut s.form;
    ui.sec("Appearance", tok);
    srow(ui, tok, "Theme", None, |ui| {
        ui.seg("theme", &labels(&THEMES), &mut f.theme, tok);
    });
    srow(ui, tok, "Accent", Some("Used for the active tool, selection and handles"), |ui| {
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
    ui.sec("Behavior", tok);
    if let Some(on) = f.autostart.as_mut() {
        srow(ui, tok, "Launch at login", None, |ui| {
            ui.switch("autostart", on, tok);
        });
    }
    let mut hid = false;
    if tray_row {
        // Windows: off is no background process at all.
        let help = if cfg!(windows) { Some(tray_help) } else { warn.then_some("No capture hotkey: start Rustshot again to capture") };
        // (Windows: two lines of plain help; elsewhere the warning, red.)
        let (err, h) = if cfg!(windows) { (false, 44.0) } else { (true, ROW_H) };
        srow_ex(ui, tok, "Show icon in tray / menu bar", help, err, h, |ui| {
            hid = ui.switch("show_tray", &mut f.show_tray, tok) && !f.show_tray;
        });
    }
    srow(ui, tok, "Copy to clipboard after capture", Some("Also when saving or uploading"), |ui| {
        ui.switch("copy_after_capture", &mut f.copy_after, tok);
    });
    srow(ui, tok, "Play shutter sound", None, |ui| {
        ui.switch("shutter_sound", &mut f.shutter, tok);
    });
    srow(ui, tok, "Show thumbnail after capture", None, |ui| {
        ui.switch("show_thumbnail", &mut f.thumbnail, tok);
    });
    srow(ui, tok, "Show hints in the overlay", Some("The keys, for a few seconds when it opens"), |ui| {
        ui.switch("overlay_hints", &mut f.hints, tok);
    });
    ui.sec("Saving", tok);
    srow(ui, tok, "Save to", None, |ui| {
        if ui.cbutton("browse", "Change\u{2026}", None, false, tok) {
            *click = Some(Click::Browse);
        }
        ui.field_icon = Some("set-folder");
        ui.width(FIELD_W).text_field("folder", &mut f.folder);
    });
    if let Some(n) = &folder_note {
        note_line(ui, tok, n, false, None);
    }
    srow_ex(ui, tok, "File name", Some(&example), false, 44.0, |ui| {
        let mut fmt = if f.format < 2 { f.format } else { usize::MAX };
        if ui.seg("format", &["PNG", "JPG"], &mut fmt, tok) && fmt < 2 {
            f.format = fmt;
        }
        let w = ui.row_free() - 112.0;
        ui.width(w.min(FIELD_W)).text_field("filename", &mut f.filename);
    });
    if ui.focus.is_focused("filename") {
        s.pattern = Pattern::File;
    }
    srow_ex(ui, tok, "Save at", Some("History keeps the original"), false, 44.0, |ui| {
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
    // Turned off: say how to get back before it goes.
    if hid {
        s.ask_hide_tray();
    }
}

fn capture(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    // "Set up…" (tray icon off): the Win+Shift+S row, focused and in view.
    if ui.enabled
        && let Some(id) = s.focus_field.take()
    {
        ui.focus.focus(&id);
    }
    let base = s.loaded.clone();
    let wayland = wayland();
    let show_renderer = s.renderer;
    let done = s.tools_reset;
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
    let mut target = s.pattern;
    let mut token = None;
    ui.sec("Files", tok);
    srow(ui, tok, "Daily subfolders", Some("A folder per day inside Save to"), |ui| {
        ui.switch("subfolder", &mut f.subfolder, tok);
        let on = ui.enabled;
        ui.enabled = on && f.subfolder;
        ui.width(160.0).text_field("subfolder_pattern", &mut f.subfolder_pattern);
        ui.enabled = on;
    });
    srow(ui, tok, "Ask where to save", Some("Ctrl+Shift+S in the overlay always asks"), |ui| {
        ui.switch("ask", &mut f.ask, tok);
    });
    srow(ui, tok, "Format", None, |ui| {
        ui.seg("format_all", &labels(&FORMATS), &mut f.format, tok);
    });
    let on = ui.enabled;
    ui.enabled = on && f.format == 1;
    srow(ui, tok, "JPEG quality", None, |ui| {
        ui.width(200.0).slider("quality", &mut f.quality, 1, 100);
    });
    ui.enabled = on;

    ui.sec("File name", tok);
    srow(ui, tok, "Pattern", None, |ui| {
        if ui.cbutton("clear", "Clear", None, false, tok) {
            *click = Some(Click::Clear);
        }
        if ui.cbutton("restore", "Restore", None, false, tok) {
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
        let label = "Saves as ";
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

    let f = &mut s.form;
    ui.sec("Overlay", tok);
    srow(ui, tok, "UI size", Some("Toolbar and labels; Auto follows each monitor"), |ui| {
        ui.width(120.0).dropdown("ui_size", &uisize::LABELS, &mut f.ui_size);
    });
    let help = if wayland { "Not on Wayland" } else { "Outline the window under the pointer; a click selects it" };
    srow(ui, tok, "Snap to windows", Some(help), |ui| {
        ui.switch("snap_windows", &mut f.snap, tok);
    });
    srow(ui, tok, "Show magnifier", Some("Zoomed pixels and a crosshair while selecting; H copies the color"), |ui| {
        ui.switch("magnifier", &mut f.magnifier, tok);
    });
    let on = ui.enabled;
    ui.enabled = on && f.magnifier;
    srow(ui, tok, "Square magnifier", Some("Round when off"), |ui| {
        ui.switch("magnifier_square", &mut f.magnifier_square, tok);
    });
    ui.enabled = on;
    srow(ui, tok, "Accept on select", Some("Copy or save as soon as a region is drawn; hold Alt to adjust"), |ui| {
        ui.seg("accept_on_select", &labels(&ACCEPTS), &mut f.accept, tok);
    });
    srow(ui, tok, "Copy text", Some("The toolbar's Copy text button; the Copy text hotkey always copies all"), |ui| {
        ui.seg("copy_text_mode", &labels(&COPY_TEXT), &mut f.copy_text, tok);
    });
    srow(ui, tok, "Redact style", Some("What Quick Redact puts over sensitive text; Solid can never be read back"), |ui| {
        ui.seg("redact_style", &labels(&REDACT_STYLES), &mut f.redact_style, tok);
    });
    if let Some(on) = f.screenclip.as_mut() {
        srow_ex(ui, tok, "Use Win+Shift+S and Print Screen", Some(SCREENCLIP_HELP), false, ROW_H, |ui| {
            ui.switch("screenclip", on, tok);
        });
        if let Some((text, tone)) = &clip_line
            && screenclip_line(ui, tok, text, *tone)
        {
            *click = Some(Click::FixScreenclip);
        }
    }
    if show_renderer {
        srow(ui, tok, "Renderer", Some("GDI repaints only what changed"), |ui| {
            ui.seg("renderer", &labels(&RENDERERS), &mut f.renderer, tok);
        });
    }
    let help = if done { "Done: the next capture starts from the defaults" } else { "Each tool remembers its last size and color" };
    srow(ui, tok, "Tool sizes & colors", Some(help), |ui| {
        if ui.cbutton("reset_tools", "Reset", Some("set-rotate"), false, tok) {
            *click = Some(Click::ResetTools);
        }
    });
    image_rows(ui, f, tok);
    ui.sec("History", tok);
    srow(ui, tok, "Keep a history of captures", Some("Edit them again later; saved and uploaded files are not affected"), |ui| {
        ui.switch("history", &mut f.history, tok);
    });
    let on = ui.enabled;
    ui.enabled = on && f.history;
    let days: Vec<&str> = HISTORY_DAYS.iter().map(|d| d.1).collect();
    srow(ui, tok, "Keep for", Some("Older captures are removed"), |ui| {
        ui.seg("history_days", &days, &mut f.history_days, tok);
    });
    let sizes: Vec<&str> = HISTORY_MB.iter().map(|d| d.1).collect();
    srow(ui, tok, "Up to", Some("The oldest go first when it is full"), |ui| {
        ui.seg("history_size", &sizes, &mut f.history_size, tok);
    });
    ui.enabled = on;
    srow(ui, tok, "Clear history", Some("Remove every capture History keeps"), |ui| {
        if ui.cbutton("clear_history", "Clear history\u{2026}", None, false, tok) {
            *click = Some(Click::ClearHistory);
        }
    });
}

/// Capture › Image: trim margins, the export border, HDR screenshots
/// (Windows).
fn image_rows(ui: &mut Ui, f: &mut crate::settings_ui::Form, tok: &Tok) {
    ui.sec("Image", tok);
    let by_hand = f.keys.chords(Action::Trim).first().map_or_else(|| "the size label's menu".to_string(), |c| c.label());
    let help = format!("Cut uniform margins off each new region or window ({by_hand} by hand)");
    srow(ui, tok, "Trim margins automatically", Some(&help), |ui| {
        ui.switch("trim_margins", &mut f.trim, tok);
    });
    srow(ui, tok, "Add a border", Some("On every export"), |ui| {
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
    if cfg!(windows) {
        srow(ui, tok, "HDR screenshots", Some("HDR displays; recordings too"), |ui| {
            ui.seg("hdr_screenshots", &labels(&HDR_MODES), &mut f.hdr, tok);
        });
    }
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

/// The overlay rows of Key bindings: (label, actions).
pub const KEY_ROWS: [(&str, &[Action]); 19] = [
    (
        "Tools",
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
        "Select \u{b7} Callout \u{b7} Spotlight \u{b7} Magnify",
        &[Action::ToolSelect, Action::ToolCallout, Action::ToolSpotlight, Action::ToolMagnify],
    ),
    (
        "Selected objects: delete \u{b7} duplicate \u{b7} front \u{b7} back",
        &[Action::DeleteObject, Action::Duplicate, Action::BringFront, Action::SendBack],
    ),
    ("Stroke size - / +", &[Action::SizeDown, Action::SizeUp]),
    ("Typing: bold \u{b7} italic \u{b7} underline", &[Action::TextBold, Action::TextItalic, Action::TextUnderline]),
    ("Copy \u{b7} Save \u{b7} Upload", &[Action::Copy, Action::Save, Action::Upload]),
    ("Copy text \u{b7} Copy color", &[Action::CopyText, Action::CopyColor]),
    ("Crop \u{b7} Quick Redact \u{b7} Freeform selection", &[Action::ToolCrop, Action::Redact, Action::SelectFreeform]),
    ("Accept \u{b7} Cancel", &[Action::Accept, Action::Cancel]),
    SAVE_ROW,
    ("Undo \u{b7} Redo", &[Action::Undo, Action::Redo]),
    ("Select all \u{b7} Palette", &[Action::SelectAll, Action::TogglePalette]),
    ("Record", &[Action::Record]),
    ("Pin to screen", &[Action::Pin]),
    ("Trim to content", &[Action::Trim]),
    ("UI size + \u{b7} - \u{b7} Auto", &[Action::UiBigger, Action::UiSmaller, Action::UiReset]),
    ("Edit window zoom + \u{b7} - \u{b7} Fit \u{b7} 1:1", &[Action::ZoomIn, Action::ZoomOut, Action::ZoomFit, Action::ZoomActual]),
    (
        "Edit window: rotate left \u{b7} right \u{b7} flip horizontal \u{b7} vertical",
        &[Action::RotateLeft, Action::RotateRight, Action::FlipHorizontal, Action::FlipVertical],
    ),
    ("Edit window: resize \u{b7} canvas \u{b7} reset image edits", &[Action::ImageResize, Action::ImageCanvas, Action::ImageReset]),
];

/// Save As, Save to Desktop, Open with and, with a share sheet (not on
/// Linux), Share.
#[cfg(not(target_os = "linux"))]
const SAVE_ROW: (&str, &[Action]) =
    ("Save As \u{b7} Save to Desktop \u{b7} Open with \u{b7} Share", &[Action::SaveAs, Action::SaveDesktop, Action::OpenWith, Action::Share]);
#[cfg(target_os = "linux")]
const SAVE_ROW: (&str, &[Action]) = ("Save As \u{b7} Save to Desktop \u{b7} Open with", &[Action::SaveAs, Action::SaveDesktop, Action::OpenWith]);

/// The while-recording rows of Key bindings: (label, action).
pub const REC_ROWS: [(&str, Action); 3] = [
    ("Pause / resume recording", Action::RecPause),
    ("Mute microphone", Action::RecMute),
    ("Hide / show recording controls", Action::RecHidePill),
];

fn keys(ui: &mut Ui, s: &mut Settings, tok: &Tok) {
    sec_at(ui, tok, "Global \u{b7} work anywhere", 8.0);
    let msgs: Vec<Option<String>> = GLOBALS.iter().map(|(_, slot)| s.global_message(*slot)).collect();
    let errs: Vec<bool> = GLOBALS.iter().map(|(_, slot)| s.global_err(*slot)).collect();
    // "Change shortcuts…" in the hotkey dialog: that row's box records.
    if ui.enabled
        && let Some(id) = s.record_field.take()
    {
        ui.focus.record(&id);
    }
    ui.allow_meta = true;
    for (((label, slot), msg), err) in GLOBALS.iter().zip(&msgs).zip(errs) {
        let chord = s.form.global_mut(*slot);
        let mut slots = [Slot { id: format!("global{slot}"), chord, err }];
        ui.key_row(label, &[], &mut slots, true, 34.0, tok);
        if let Some(m) = msg {
            note_line(ui, tok, m, true, Some(8.0));
        }
    }
    ui.allow_meta = false;
    sec_at(ui, tok, "In the overlay \u{b7} single keys", 8.0);
    let bad = s.form.conflicting();
    for (label, acts) in KEY_ROWS {
        let mut chords: Vec<Option<Chord>> = acts.iter().map(|a| s.form.keys.chords(*a).first().copied()).collect();
        let names: Vec<&str> = acts.iter().map(|a| a.label()).collect();
        let changed = {
            let mut slots: Vec<Slot> = chords
                .iter_mut()
                .zip(acts.iter())
                .map(|(c, a)| Slot { id: format!("key_{}", a.id()), chord: c, err: bad.contains(a) })
                .collect();
            ui.key_row(label, &names, &mut slots, false, 30.0, tok)
        };
        if let Some(i) = changed {
            s.form.keys.set(acts[i], chords[i].into_iter().collect());
        }
        if let Some(n) = acts.iter().find_map(|a| s.form.conflict_note(*a).map(|n| (a, n))).map(|(a, n)| format!("{}: {n}", a.label())) {
            note_line(ui, tok, &n, true, Some(8.0));
        }
    }
    ui.allow_meta = true;
    // Global too, but only while a recording runs (the pill never takes
    // the keyboard); stopping is the record hotkey.
    sec_at(ui, tok, "While recording \u{b7} work anywhere", 8.0);
    for (label, a) in REC_ROWS {
        let note = s.form.rec_note(a);
        let mut chord = s.form.keys.chords(a).first().copied();
        let changed = {
            let err = note.as_deref().is_some_and(|m| m.starts_with("Already"));
            let mut slots = [Slot { id: format!("key_{}", a.id()), chord: &mut chord, err }];
            ui.key_row(label, &[], &mut slots, true, 34.0, tok)
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

/// The More from PowerNapps tiles: none until there are real apps (the
/// row is hidden while the list is empty).
pub const MORE_APPS: [(&str, &str, &str); 0] = [];

fn about(ui: &mut Ui, s: &mut Settings, tok: &Tok, click: &mut Option<Click>) {
    let b = ui.bounds();
    let y = ui.cursor_y();
    // Hero: the 72 px mark, the name, the version line, the description.
    mark(ui, b.x, y + 18.0, 72.0);
    let tx = b.x + 72.0 + 18.0;
    let desc = "A fast, tiny screenshot and screen-recording tool. Draws its own pixels, runs on Windows, macOS and Linux, and never asks you to sign in.";
    let dw = (b.x1() - tx).min(380.0);
    let lines = wrap(desc, dw, |t| ui.text_w(12.0, t));
    let block = 24.0 + 4.0 + 16.0 + 4.0 + 18.0 * lines.len() as f32;
    let top = y + 18.0 + (72.0 - block).max(0.0) / 2.0;
    let (px, py) = (ui.px(tx), ui.px(top + 12.0));
    ui.text_at(20.0, NAME, px, py, tok.tx, true);
    let ver = format!("Version {} \u{b7} ", env!("CARGO_PKG_VERSION"));
    let vy = top + 24.0 + 4.0;
    ui.text_at(12.0, &ver, px, ui.px(vy + 8.0), tok.tx2, false);
    let vw = ui.text_w(12.0, &ver);
    let saved = ui.ctl_h;
    ui.ctl_h = 16.0;
    let link = FRect { x: tx + vw, y: vy, w: ui.text_w(12.0, "Check for updates") + 2.0, h: 16.0 };
    if ui.place(link).clink("check_updates_now", "Check for updates", 12.0, false, tok) {
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
    let mut rows: Vec<Vec<(&str, &str, bool)>> = vec![Vec::new()];
    for (label, url) in crate::actions::LINKS.iter().take(5) {
        let ext = matches!(*label, "Website" | "Source on GitHub" | "Report an issue");
        let w = ui.text_w(12.5, label) + if ext { 19.0 } else { 0.0 };
        if x > 0.0 && x + w > b.w {
            rows.push(Vec::new());
            x = 0.0;
        }
        rows.last_mut().expect("a row").push((label, url, ext));
        x += w + gap;
    }
    ui.space(2.0);
    let (saved_gap, saved_h) = (ui.lay.gap, ui.ctl_h);
    ui.ctl_h = 16.0;
    for r in &rows {
        ui.lay.gap = gap;
        ui.row(|ui| {
            for (label, url, ext) in r {
                if ui.clink(&format!("link_{label}"), label, 12.5, *ext, tok) {
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
    srow(ui, tok, "Check for updates automatically", Some("Once a day, from GitHub"), |ui| {
        ui.switch("check_updates", &mut s.form.check_updates, tok);
    });

    // More from PowerNapps, with its link on the right.
    let y = ui.cursor_y();
    ui.sec("More from PowerNapps", tok);
    let lw = ui.text_w(11.5, "powernapps.net") + 19.0;
    let r = FRect { x: b.x1() - lw, y: y + 14.0 + 6.5 - 8.0, w: lw, h: 16.0 };
    let saved = ui.ctl_h;
    ui.ctl_h = 16.0;
    if ui.place(r).clink("link_powernapps", "powernapps.net", 11.5, true, tok) {
        *click = Some(Click::Link("https://powernapps.net"));
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
    let text = "2026 PowerNapps \u{b7} Made in Los Angeles \u{b7} GPL-3.0 License";
    ui.text_at(11.0, text, fx + 2.0 * r + ui.px(4.0), fy, c, false);
    ui.space(14.0 + 14.0);
}
