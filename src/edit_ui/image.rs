//! The Edit window's Image menu (from the status bar's size chip, and
//! its keys) and its two dialogs: Resize… (width, height, percent, keep
//! aspect ratio, resampling, a live size estimate) and Canvas… (padding,
//! background solid or presentation, corner radius, shadow, a preview).
//! Every change is one undo step ([`crate::editor::host::Canvas`]).

use crate::imgedit::{frame, Bg, Filter, Pad, Resize};
use crate::keymap::Action;
use crate::objects::FRect;
use crate::pixbuf::PixBuf;
use crate::ui::compact::Tok;
use crate::ui::{TextState, Ui};
use crate::uifb::C4;

/// What the Image menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImgAct {
    Resize,
    RotateLeft,
    RotateRight,
    FlipH,
    FlipV,
    Canvas,
    /// Watermark, drop shadow, torn edge, reflection ([`super::effects_dlg`]).
    Effects,
    Reset,
}

/// The menu's rows: what, label, icon, the key shown.
pub const MENU: [(ImgAct, crate::i18n::Key, &str, Action); 8] = [
    (ImgAct::Resize, key!("edit.img.resize"), "scaling", Action::ImageResize),
    (ImgAct::RotateLeft, key!("edit.img.rotate_left"), "rotate-ccw", Action::RotateLeft),
    (ImgAct::RotateRight, key!("edit.img.rotate_right"), "rotate-cw", Action::RotateRight),
    (ImgAct::FlipH, key!("edit.img.flip_h"), "flip-h", Action::FlipHorizontal),
    (ImgAct::FlipV, key!("edit.img.flip_v"), "flip-v", Action::FlipVertical),
    (ImgAct::Canvas, key!("edit.img.canvas"), "frame", Action::ImageCanvas),
    (ImgAct::Effects, key!("edit.img.effects"), "sparkles", Action::ImageEffects),
    (ImgAct::Reset, key!("edit.img.reset"), "undo", Action::ImageReset),
];

/// The menu's row height and width (logical px).
pub const MENU_ROW: f32 = 30.0;
pub const MENU_W: f32 = 236.0;

/// The image action bound to keymap action `a`.
pub fn act_of(a: Action) -> Option<ImgAct> {
    MENU.iter().find(|m| m.3 == a).map(|m| m.0)
}

/// A whole number of px out of a field ("1920", " 1 920 ").
fn parse_px(s: &str) -> Option<u32> {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    t.parse::<u32>().ok()
}

/// A percent out of a field ("50", "50 %", "33.3").
fn parse_pct(s: &str) -> Option<f64> {
    let t: String = s.chars().filter(|c| !c.is_whitespace() && *c != '%').collect();
    t.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0)
}

fn pct_text(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r:.1}") }
}

/// The Resize dialog's fields.
pub struct ResizeForm {
    /// The picture it resizes (the crop, turned).
    pub src: (u32, u32),
    pub w: TextState,
    pub h: TextState,
    pub pct: TextState,
    pub lock: bool,
    /// 0: Lanczos-3, 1: area average.
    pub filter: usize,
    /// The fields' texts as last seen (which one changed drives the rest).
    seen: [String; 3],
    /// The estimate for (w, h, filter): bytes.
    est: Option<((u32, u32, usize), u64)>,
    /// The canvas round the resized picture (its pixels count toward the cap).
    pub canvas: Option<Pad>,
}

impl ResizeForm {
    /// The fields hold a size, but it is past [`crate::imgedit::MAX_PIXELS`]
    /// with the canvas.
    pub fn over_cap(&self) -> bool {
        match (parse_px(&self.w.text), parse_px(&self.h.text)) {
            (Some(w), Some(h)) => !crate::imgedit::within_cap(self.canvas.map_or((w, h), |c| c.size((w, h)))),
            _ => false,
        }
    }

    pub fn new(src: (u32, u32), cur: Option<Resize>) -> ResizeForm {
        let (w, h) = cur.map_or(src, |r| r.target(src));
        let filter = usize::from(cur.is_some_and(|r| r.filter == Filter::Area));
        let lock = (w as u64 * src.1 as u64).abs_diff(h as u64 * src.0 as u64) <= src.0.max(src.1) as u64;
        let mut f = ResizeForm {
            src,
            w: TextState::new(&w.to_string()),
            h: TextState::new(&h.to_string()),
            pct: TextState::new(&pct_text(w as f64 * 100.0 / src.0.max(1) as f64)),
            lock,
            filter,
            seen: Default::default(),
            est: None,
            canvas: None,
        };
        f.seen = [f.w.text.clone(), f.h.text.clone(), f.pct.text.clone()];
        f.w.select_all();
        f
    }

    /// One field was typed in: the others follow it (the height and the
    /// percent from the width with the aspect kept, and so on).
    pub fn sync(&mut self) {
        let (sw, sh) = (self.src.0.max(1) as f64, self.src.1.max(1) as f64);
        if self.w.text != self.seen[0] {
            if let Some(w) = parse_px(&self.w.text) {
                if self.lock {
                    self.h.set(&((w as f64 * sh / sw).round().max(1.0) as u32).to_string());
                }
                self.pct.set(&pct_text(w as f64 * 100.0 / sw));
            }
        } else if self.h.text != self.seen[1] {
            if let Some(h) = parse_px(&self.h.text) {
                if self.lock {
                    let w = (h as f64 * sw / sh).round().max(1.0) as u32;
                    self.w.set(&w.to_string());
                    self.pct.set(&pct_text(w as f64 * 100.0 / sw));
                } else {
                    self.pct.set(&pct_text(h as f64 * 100.0 / sh));
                }
            }
        } else if self.pct.text != self.seen[2]
            && let Some(p) = parse_pct(&self.pct.text)
        {
            self.w.set(&((sw * p / 100.0).round().max(1.0) as u32).to_string());
            self.h.set(&((sh * p / 100.0).round().max(1.0) as u32).to_string());
        }
        self.seen = [self.w.text.clone(), self.h.text.clone(), self.pct.text.clone()];
    }

    /// Keep aspect ratio turned on: the height follows the width again.
    pub fn relock(&mut self) {
        if self.lock
            && let Some(w) = parse_px(&self.w.text)
        {
            let h = (w as f64 * self.src.1 as f64 / self.src.0.max(1) as f64).round().max(1.0) as u32;
            self.h.set(&h.to_string());
            self.seen[1] = self.h.text.clone();
        }
    }

    /// The size asked for, when both fields hold one in range.
    pub fn size(&self) -> Option<(u32, u32)> {
        let ok = |v: u32| (1..=crate::imgedit::MAX_SIDE).contains(&v);
        let (w, h) = (parse_px(&self.w.text)?, parse_px(&self.h.text)?);
        (ok(w) && ok(h) && !self.over_cap()).then_some((w, h))
    }

    pub fn filter(&self) -> Filter {
        if self.filter == 1 { Filter::Area } else { Filter::Lanczos3 }
    }

    /// What Apply sets: `None` when the fields are not a size; `Some(None)`
    /// for the picture's own size (no resize).
    pub fn result(&self) -> Option<Option<Resize>> {
        let (w, h) = self.size()?;
        Some(((w, h) != self.src).then_some(Resize { w, h, from: self.src, filter: self.filter() }))
    }

    /// It would enlarge the picture (shown, never done silently).
    pub fn enlarges(&self) -> bool {
        self.size().is_some_and(|(w, h)| w > self.src.0 || h > self.src.1)
    }

    /// The file size estimate for `pic` (the picture before the resize)
    /// saved as `fmt`: a tile from its middle resized and encoded, scaled
    /// up to the whole (cached for the same size and filter).
    pub fn estimate(&mut self, pic: &PixBuf, fmt: crate::export::Format, quality: u8) -> Option<u64> {
        let (w, h) = self.size()?;
        let key = (w, h, self.filter);
        if let Some((k, v)) = self.est
            && k == key
        {
            return Some(v);
        }
        let v = estimate_bytes(pic, (w, h), self.filter(), fmt, quality)?;
        self.est = Some((key, v));
        Some(v)
    }
}

/// Bytes `pic` resized to `size` takes as `fmt`, estimated from a tile.
pub fn estimate_bytes(pic: &PixBuf, (w, h): (u32, u32), f: Filter, fmt: crate::export::Format, quality: u8) -> Option<u64> {
    let (sw, sh) = pic.dimensions();
    if sw == 0 || sh == 0 {
        return None;
    }
    const T: u32 = 256;
    let (tw, th) = (w.min(T), h.min(T));
    // The source region that becomes the tile, centered.
    let rw = ((tw as f64 * sw as f64 / w as f64).round() as u32).clamp(1, sw);
    let rh = ((th as f64 * sh as f64 / h as f64).round() as u32).clamp(1, sh);
    let (x, y) = ((sw - rw) / 2, (sh - rh) / 2);
    let tile = crate::imgedit::resample::resize(&pic.crop(x, y, rw, rh), tw, th, f);
    let bytes = crate::export::encode(&tile, fmt, quality).ok()?.len() as f64;
    Some((bytes * (w as f64 * h as f64) / (tw as f64 * th as f64)).round() as u64)
}

/// "840 KB", "1.2 MB".
pub fn bytes_text(b: u64) -> String {
    if b < 1024 * 1024 { tr!("size.kb", n = b.div_ceil(1024).max(1)) } else { tr!("size.mb", n = format!("{:.1}", b as f64 / (1024.0 * 1024.0))) }
}

/// The Canvas dialog's fields.
pub struct CanvasForm {
    pub pad: TextState,
    /// Padding per side (else `pad` on all four).
    pub sides: bool,
    pub side: [TextState; 4],
    /// 0: solid color, 1: presentation.
    pub bg: usize,
    pub color: TextState,
    pub radius: u8,
    pub shadow: bool,
    /// The preview for the fields as they were (their key).
    preview: Option<(String, PixBuf)>,
    /// The picture the canvas goes round (after the resize): its pixels
    /// count toward the cap.
    pub src: Option<(u32, u32)>,
}

/// The solid backgrounds offered as swatches.
pub const SWATCHES: [(crate::i18n::Key, &str); 4] = [(key!("color.white"), "#ffffff"), (key!("color.light_gray"), "#eef0f4"), (key!("color.black"), "#000000"), (key!("color.transparent"), "#00000000")];

impl CanvasForm {
    pub fn new(cur: Option<Pad>) -> CanvasForm {
        let p = cur.unwrap_or_else(Pad::presentation);
        let color = match p.bg {
            Bg::Solid(c) if c.a == 255 => format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
            Bg::Solid(c) => format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a),
            Bg::Presentation => "#ffffff".into(),
        };
        CanvasForm {
            pad: TextState::new(&p.top.to_string()),
            sides: !p.is_uniform(),
            side: [p.top, p.right, p.bottom, p.left].map(|v| TextState::new(&v.to_string())),
            bg: usize::from(matches!(p.bg, Bg::Presentation)),
            color: TextState::new(&color),
            radius: p.radius.round().clamp(0.0, 48.0) as u8,
            shadow: p.shadow,
            preview: None,
            src: None,
        }
    }

    /// The padding is a number but the picture with it is past the cap.
    pub fn over_cap(&self) -> bool {
        match (self.fields(), self.src) {
            (Some(p), Some(s)) => !crate::imgedit::within_cap(p.size(s)),
            _ => false,
        }
    }

    /// The background picked: the presentation look comes with its
    /// corners and shadow.
    pub fn pick_bg(&mut self, bg: usize) {
        if bg == 1 && self.bg != 1 {
            self.radius = frame::PRESENT_RADIUS as u8;
            self.shadow = true;
        }
        self.bg = bg;
    }

    /// The canvas asked for (`None`: a field is not a number or a color,
    /// or the picture with it would be past the cap).
    pub fn result(&self) -> Option<Pad> {
        self.fields().filter(|_| !self.over_cap())
    }

    fn fields(&self) -> Option<Pad> {
        let px = |t: &TextState| parse_px(&t.text).filter(|v| *v <= frame::MAX_PAD);
        let [t, r, b, l] = if self.sides {
            [px(&self.side[0])?, px(&self.side[1])?, px(&self.side[2])?, px(&self.side[3])?]
        } else {
            let p = px(&self.pad)?;
            [p; 4]
        };
        let bg = if self.bg == 1 {
            Bg::Presentation
        } else {
            let (r, g, b, a) = crate::config::parse_color(self.color.text.trim())?;
            Bg::Solid(C4::new(r, g, b, a))
        };
        Some(Pad { top: t, right: r, bottom: b, left: l, bg, radius: self.radius as f32, shadow: self.shadow })
    }

    /// The preview of `pad` round `small` (the picture at `k` of its
    /// size), made again only when the fields change.
    pub fn preview(&mut self, small: &PixBuf, k: f32) -> Option<&PixBuf> {
        let p = self.result()?;
        let key = format!("{p:?}");
        if self.preview.as_ref().is_none_or(|(k0, _)| *k0 != key) {
            let s = |v: u32| (v as f32 * k).round() as u32;
            let q = Pad { top: s(p.top), right: s(p.right), bottom: s(p.bottom), left: s(p.left), radius: p.radius * k, ..p };
            self.preview = Some((key, frame::apply_at(small, &q, k)));
        }
        self.preview.as_ref().map(|(_, i)| i)
    }
}

/// An open image dialog.
pub enum ImgDialog {
    Resize(ResizeForm),
    Canvas(CanvasForm),
    Effects(super::effects_dlg::EffectsForm),
}

/// A dialog's answer this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DlgOut {
    Apply,
    Cancel,
    /// Canvas: none; Resize: the picture's own size; Effects: none.
    Remove,
    /// Effects: keep the watermark for every export (the dialog stays).
    ForExports,
}

/// The dialog's card (logical px) in a `w`×`h` window.
pub fn card(d: &ImgDialog, w: f32, h: f32) -> FRect {
    let (cw, ch) = match d {
        ImgDialog::Resize(f) => (440.0, if f.enlarges() { 286.0 } else { 262.0 }),
        ImgDialog::Canvas(f) => (480.0, if f.sides { 512.0 } else { 476.0 }),
        ImgDialog::Effects(_) => (520.0, super::effects_dlg::CARD_H),
    };
    FRect { x: ((w - cw) / 2.0).round(), y: ((h - ch) / 2.0).max(8.0).round(), w: cw, h: ch }
}

const LABEL_W: f32 = 96.0;

/// A row's label in the label column.
fn label(ui: &mut Ui, s: &str, tok: &Tok) {
    let h = ui.ctl_h;
    let r = ui.alloc(Some(LABEL_W), h);
    let size = 12.0;
    ui.text_at(size, s, r.x, r.y + r.h / 2.0, tok.tx2, false);
}

/// Text after a field ("px", "%").
fn unit(ui: &mut Ui, s: &str, tok: &Tok) {
    let w = ui.text_w(12.0, s).ceil();
    let r = ui.alloc(Some(w), ui.ctl_h);
    ui.text_at(12.0, s, r.x, r.y + r.h / 2.0, tok.tx3, false);
}

/// Paint the open dialog (`pic`: the picture before the resize and the
/// canvas; `small`: it fitted for the preview, at `k`).
pub fn paint(ui: &mut Ui, d: &mut ImgDialog, win: (f32, f32), pic: &PixBuf, small: &(PixBuf, f32), cfg: &crate::config::Config, tok: &Tok) -> Option<DlgOut> {
    let r = card(d, win.0, win.1);
    ui.modal_card(r);
    let inner = FRect { x: r.x + 20.0, y: r.y + 18.0, w: r.w - 40.0, h: r.h - 36.0 };
    let mut out = None;
    let (saved_h, saved_px, saved_rad) = (ui.ctl_h, ui.text_px, ui.radius);
    ui.ctl_h = 28.0;
    ui.text_px = 12.0 * crate::ui::compact::CSS;
    ui.radius = 7.0;
    ui.area(inner, |ui| {
        let title = match d {
            ImgDialog::Resize(_) => "Resize image",
            ImgDialog::Canvas(_) => "Canvas",
            ImgDialog::Effects(_) => tr!("fx.title"),
        };
        let t = ui.alloc(None, 24.0);
        ui.text_at(14.0, title, t.x, t.y + t.h / 2.0, tok.tx, true);
        ui.space(4.0);
        match d {
            ImgDialog::Resize(f) => resize_body(ui, f, pic, cfg, tok, &mut out),
            ImgDialog::Canvas(f) => canvas_body(ui, f, small, tok, &mut out),
            ImgDialog::Effects(f) => {
                let mut extra = None;
                super::effects_dlg::body(ui, f, small, tok, &mut out, &mut extra);
                if extra.is_some() && out.is_none() {
                    out = Some(DlgOut::ForExports);
                }
            }
        }
    });
    ui.ctl_h = saved_h;
    ui.text_px = saved_px;
    ui.radius = saved_rad;
    out
}

fn resize_body(ui: &mut Ui, f: &mut ResizeForm, pic: &PixBuf, cfg: &crate::config::Config, tok: &Tok, out: &mut Option<DlgOut>) {
    ui.row(|ui| {
        label(ui, tr!("edit.img.width"), tok);
        ui.width(84.0).text_field("rs_w", &mut f.w);
        unit(ui, "px", tok);
        ui.space(14.0);
        ui.width(46.0).alloc(Some(46.0), 28.0);
        let r = ui.last_alloc;
        ui.text_at(12.0, tr!("edit.img.height"), r.x, r.y + r.h / 2.0, tok.tx2, false);
        ui.width(84.0).text_field("rs_h", &mut f.h);
        unit(ui, "px", tok);
    });
    ui.row(|ui| {
        label(ui, tr!("edit.img.scale"), tok);
        ui.width(84.0).text_field("rs_pct", &mut f.pct);
        unit(ui, "%", tok);
        ui.space(14.0);
        if ui.checkbox("rs_lock", tr!("edit.img.keep_aspect"), &mut f.lock, tok) {
            f.relock();
        }
    });
    f.sync();
    ui.row(|ui| {
        label(ui, tr!("edit.img.resampling"), tok);
        ui.seg("rs_filter", &[tr!("edit.img.lanczos"), tr!("edit.img.area")], &mut f.filter, tok);
    });
    // "3840 × 2160 → 1920 × 1080 · about 1.2 MB as PNG"
    let fmt = crate::export::Format::from_config(cfg);
    let line = match f.size() {
        Some((w, h)) => {
            let est = f.estimate(pic, fmt, cfg.jpeg_quality).map(|b| format!(" {}", tr!("edit.img.estimate", size = bytes_text(b), format = fmt.ext().to_uppercase()))).unwrap_or_default();
            format!("{}{est}", tr!("edit.img.new_size", sw = f.src.0, sh = f.src.1, w, h))
        }
        None if f.over_cap() => tr!("edit.img.too_large", mp = crate::imgedit::MAX_PIXELS / 1_000_000),
        None => tr!("edit.img.enter_size", max = crate::imgedit::MAX_SIDE),
    };
    let r = ui.alloc(None, 22.0);
    ui.text_at(11.5, &line, r.x, r.y + r.h / 2.0, if f.size().is_some() { tok.tx2 } else { tok.er }, false);
    if f.enlarges() {
        let r = ui.alloc(None, 18.0);
        ui.text_at(11.5, tr!("edit.img.enlarged"), r.x, r.y + r.h / 2.0, tok.er, false);
    }
    ui.space(6.0);
    ui.row_rtl(|ui| {
        let ok = f.size().is_some();
        let on = ui.enabled;
        ui.enabled = on && ok;
        if ui.cbutton("rs_apply", tr!("common.apply"), None, true, tok) {
            *out = Some(DlgOut::Apply);
        }
        ui.enabled = on;
        if ui.cbutton("rs_cancel", tr!("common.cancel"), None, false, tok) {
            *out = Some(DlgOut::Cancel);
        }
        if ui.cbutton("rs_orig", tr!("edit.img.original"), None, false, tok) {
            *out = Some(DlgOut::Remove);
        }
    });
}

fn canvas_body(ui: &mut Ui, f: &mut CanvasForm, small: &(PixBuf, f32), tok: &Tok, out: &mut Option<DlgOut>) {
    ui.row(|ui| {
        label(ui, tr!("rec.panel.background"), tok);
        let mut bg = f.bg;
        if ui.seg("cv_bg", &[tr!("edit.img.solid"), tr!("edit.img.presentation")], &mut bg, tok) {
            f.pick_bg(bg);
        }
    });
    let on = ui.enabled;
    ui.row(|ui| {
        label(ui, tr!("tip.color"), tok);
        ui.enabled = on && f.bg == 0;
        let cur = crate::config::parse_color(f.color.text.trim());
        for (i, (_, hex)) in SWATCHES.iter().enumerate() {
            let c = crate::config::parse_color(hex).map(|(r, g, b, a)| C4::new(r, g, b, a));
            let shown = c.map(|c| if c.a == 0 { C4::new(128, 128, 128, 90) } else { c });
            let picked = cur.is_some() && cur == crate::config::parse_color(hex);
            if ui.swatch(&format!("cv_sw{i}"), shown, picked && f.bg == 0, tok) {
                f.color.set(hex);
            }
        }
        ui.space(6.0);
        ui.width(110.0).text_field("cv_color", &mut f.color);
        ui.enabled = on;
    });
    ui.row(|ui| {
        label(ui, tr!("edit.img.padding"), tok);
        ui.enabled = on && !f.sides;
        ui.width(70.0).text_field("cv_pad", &mut f.pad);
        unit(ui, "px", tok);
        ui.enabled = on;
        ui.space(14.0);
        if ui.checkbox("cv_sides", tr!("edit.img.each_side"), &mut f.sides, tok) && f.sides {
            for s in &mut f.side {
                s.set(&f.pad.text);
            }
        }
    });
    if f.sides {
        ui.row(|ui| {
            ui.space(LABEL_W + 8.0);
            for (i, name) in [tr!("edit.img.top"), tr!("edit.img.right"), tr!("edit.img.bottom"), tr!("edit.img.left")].iter().enumerate() {
                let w = ui.text_w(11.5, name).ceil();
                let r = ui.alloc(Some(w), 28.0);
                ui.text_at(11.5, name, r.x, r.y + r.h / 2.0, tok.tx3, false);
                ui.width(48.0).text_field(&format!("cv_side{i}"), &mut f.side[i]);
            }
        });
    }
    ui.row(|ui| {
        label(ui, tr!("edit.img.corners"), tok);
        ui.width(180.0).slider("cv_radius", &mut f.radius, 0, 48);
        unit(ui, "px", tok);
    });
    ui.row(|ui| {
        label(ui, tr!("edit.img.shadow"), tok);
        ui.switch("cv_shadow", &mut f.shadow, tok);
        unit(ui, tr!("edit.img.shadow_spec"), tok);
    });
    // The preview, centered in its box.
    let pb = ui.alloc(None, 170.0);
    let k = ui.k;
    let bx = FRect { x: pb.x, y: pb.y, w: pb.w, h: pb.h };
    crate::ui::paint::fill(&mut ui.fb, bx, 8.0 * k, tok.sf2);
    match f.preview(&small.0, small.1) {
        Some(img) => {
            // Fitted into the box (the preview is small already).
            let (iw, ih) = img.dimensions();
            let fit = crate::scale::fit_size(iw, ih, (bx.w - 16.0 * k).max(1.0) as u32, (bx.h - 16.0 * k).max(1.0) as u32);
            let shown = if fit == (iw, ih) { img.clone() } else { crate::imgedit::resample::resize(img, fit.0, fit.1, Filter::Area) };
            let (x0, y0) = ((bx.x + (bx.w - fit.0 as f32) / 2.0).round() as i32, (bx.y + (bx.h - fit.1 as f32) / 2.0).round() as i32);
            for (y, row) in shown.as_raw().chunks_exact(fit.0 as usize * 4).enumerate() {
                for (x, p) in row.as_chunks::<4>().0.iter().enumerate() {
                    ui.fb.blend_px(x0 + x as i32, y0 + y as i32, C4::new(p[0], p[1], p[2], p[3]));
                }
            }
        }
        None => {
            let msg = if f.over_cap() {
                tr!("edit.img.too_large_pad", mp = crate::imgedit::MAX_PIXELS / 1_000_000)
            } else {
                tr!("edit.img.enter_padding").to_string()
            };
            ui.text_at(12.0, &msg, bx.x + 12.0 * k, bx.y + bx.h / 2.0, tok.er, false);
        }
    }
    ui.space(4.0);
    ui.row_rtl(|ui| {
        let ok = f.result().is_some();
        ui.enabled = on && ok;
        if ui.cbutton("cv_apply", tr!("common.apply"), None, true, tok) {
            *out = Some(DlgOut::Apply);
        }
        ui.enabled = on;
        if ui.cbutton("cv_cancel", tr!("common.cancel"), None, false, tok) {
            *out = Some(DlgOut::Cancel);
        }
        if ui.cbutton("cv_remove", tr!("edit.img.no_canvas"), None, false, tok) {
            *out = Some(DlgOut::Remove);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_resize_fields_follow_each_other() {
        let mut f = ResizeForm::new((1920, 1080), None);
        assert_eq!((f.w.text.as_str(), f.h.text.as_str(), f.pct.text.as_str(), f.lock), ("1920", "1080", "100", true));
        assert_eq!(f.result(), Some(None), "its own size: no resize");
        f.w.set("960");
        f.sync();
        assert_eq!((f.h.text.as_str(), f.pct.text.as_str()), ("540", "50"));
        f.pct.set("25 %");
        f.sync();
        assert_eq!((f.w.text.as_str(), f.h.text.as_str()), ("480", "270"));
        f.h.set("540");
        f.sync();
        assert_eq!((f.w.text.as_str(), f.pct.text.as_str()), ("960", "50"));
        f.lock = false;
        f.h.set("100");
        f.sync();
        assert_eq!(f.w.text, "960", "unlocked: the width stays");
        assert_eq!(f.result(), Some(Some(Resize { w: 960, h: 100, from: (1920, 1080), filter: Filter::Lanczos3 })));
        f.lock = true;
        f.relock();
        assert_eq!(f.h.text, "540");
        f.filter = 1;
        assert_eq!(f.result().unwrap().unwrap().filter, Filter::Area);
        f.w.set("4000");
        f.sync();
        assert!(f.enlarges());
        f.w.set("abc");
        f.sync();
        assert_eq!(f.result(), None, "not a size");
        f.w.set("0");
        assert_eq!(f.result(), None);
        // Past 200 MP (with the canvas): refused, and said so.
        f.w.set("20000");
        f.h.set("20000");
        assert!(f.over_cap() && f.size().is_none());
        f.w.set("14000");
        f.h.set("14000");
        assert!(f.size().is_some());
        f.canvas = Some(Pad::uniform(2000, Bg::Presentation, 12.0, true));
        assert!(f.over_cap() && f.size().is_none(), "the canvas counts");
        // Opened again on a resize: its size and filter.
        let g = ResizeForm::new((1920, 1080), Some(Resize { w: 640, h: 360, from: (1920, 1080), filter: Filter::Area }));
        assert_eq!((g.w.text.as_str(), g.h.text.as_str(), g.filter), ("640", "360", 1));
    }

    #[test]
    fn size_estimates_scale_with_the_area() {
        let mut pic = PixBuf::new(800, 600);
        let mut s = 7u32;
        for p in pic.as_raw_mut().as_chunks_mut::<4>().0 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *p = [s as u8, (s >> 8) as u8, (s >> 16) as u8, 255];
        }
        let fmt = crate::export::Format::Png;
        let big = estimate_bytes(&pic, (800, 600), Filter::Lanczos3, fmt, 90).unwrap();
        let small = estimate_bytes(&pic, (400, 300), Filter::Lanczos3, fmt, 90).unwrap();
        let real = crate::export::encode(&pic, fmt, 90).unwrap().len() as f64;
        assert!((big as f64 / real - 1.0).abs() < 0.25, "estimate {big} vs {real}");
        assert!(small < big / 2, "{small} vs {big}");
        assert_eq!(bytes_text(500), "1 KB");
        assert_eq!(bytes_text(900 * 1024), "900 KB");
        assert_eq!(bytes_text(1_300_000), "1.2 MB");
    }

    #[test]
    fn the_canvas_fields() {
        let mut f = CanvasForm::new(None);
        assert_eq!(f.result(), Some(Pad::presentation()), "presentation by default");
        f.pick_bg(0);
        f.color.set("#102030");
        f.pad.set("10");
        f.radius = 0;
        f.shadow = false;
        assert_eq!(f.result(), Some(Pad::uniform(10, Bg::Solid(C4::rgb(16, 32, 48)), 0.0, false)));
        f.sides = true;
        for (s, v) in f.side.iter_mut().zip(["1", "2", "3", "4"]) {
            s.set(v);
        }
        assert_eq!(f.result().map(|p| (p.top, p.right, p.bottom, p.left)), Some((1, 2, 3, 4)));
        f.color.set("nope");
        assert_eq!(f.result(), None);
        f.pick_bg(1);
        assert!(f.shadow && f.radius == 12, "the presentation look");
        assert!(matches!(f.result().unwrap().bg, Bg::Presentation));
        // The preview is cached for the same fields.
        let small = PixBuf::from_pixel(40, 30, [200, 30, 30, 255]);
        let a = f.preview(&small, 0.5).unwrap().dimensions();
        assert_eq!(a, (40 + 1 + 2, 30 + 2 + 1), "the padding at half size: {a:?}");
        // Round a picture near the cap: padding past it is refused.
        f.sides = false;
        f.pad.set("2000");
        f.src = Some((14_000, 10_000));
        assert!(f.over_cap() && f.result().is_none());
        f.pad.set("10");
        assert!(!f.over_cap() && f.result().is_some());
        // Back on a saved canvas.
        let g = CanvasForm::new(Some(Pad { top: 5, right: 6, bottom: 7, left: 8, bg: Bg::Solid(C4::new(1, 2, 3, 0)), radius: 4.0, shadow: false }));
        assert!(g.sides && g.color.text == "#01020300" && g.radius == 4);
        assert!(MENU.iter().all(|m| act_of(m.3) == Some(m.0)));
    }
}
