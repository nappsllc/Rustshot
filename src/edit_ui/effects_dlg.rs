//! The Edit window's Effects… dialog (Image menu): Watermark, Drop
//! shadow, Torn edge and Reflection on tabs, each with its On switch, and
//! a live preview of the picture with its canvas and the effects. Apply
//! is one undo step; "Use for every export" keeps the watermark for
//! Settings › Saving "Watermark on export".

use crate::config::Config;
use crate::imgedit::effects::{self, Effects, Reflection, Shadow, Str, Torn, Watermark};
use crate::imgedit::{Filter, Pad, frame};
use crate::pixbuf::PixBuf;
use crate::ui::compact::Tok;
use crate::ui::{TextState, Ui};
use crate::uifb::C4;

/// The anchors' labels (the dropdown), in [`effects::ANCHORS`] order.
pub const ANCHOR_LABELS: [crate::i18n::Key; 9] = [
    key!("fx.anchor.top_left"),
    key!("fx.anchor.top"),
    key!("fx.anchor.top_right"),
    key!("fx.anchor.left"),
    key!("fx.anchor.center"),
    key!("fx.anchor.right"),
    key!("fx.anchor.bottom_left"),
    key!("fx.anchor.bottom"),
    key!("fx.anchor.bottom_right"),
];

/// The dialog's fields.
pub struct EffectsForm {
    pub tab: usize,
    /// Each effect on (Watermark, Drop shadow, Torn edge, Reflection).
    pub on: [bool; 4],
    /// 0: text, 1: a PNG image.
    pub wm_kind: usize,
    pub wm_text: TextState,
    pub wm_path: TextState,
    pub anchor: usize,
    pub tiled: bool,
    pub wm_size: u8,
    pub wm_opacity: u8,
    pub wm_margin: TextState,
    pub wm_color: TextState,
    pub sh_dx: TextState,
    pub sh_dy: TextState,
    pub sh_blur: u8,
    pub sh_opacity: u8,
    pub sh_color: TextState,
    pub sides: [bool; 4],
    pub depth: u8,
    pub tooth: u8,
    pub seed: u32,
    pub rf_height: u8,
    pub rf_opacity: u8,
    pub rf_gap: TextState,
    /// The canvas the effects come after (the preview shows it).
    pub canvas: Option<Pad>,
    /// "Use for every export" was clicked.
    pub saved: bool,
    preview: Option<(String, PixBuf)>,
}

fn hex(c: C4) -> String {
    if c.a == 255 { format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b) } else { format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a) }
}

fn pct(v: f32) -> u8 {
    (v * 100.0).round().clamp(0.0, 100.0) as u8
}

impl EffectsForm {
    /// The fields for the effects `cur` (a watermark not set yet starts
    /// from the export watermark's settings).
    pub fn new(cur: Effects, canvas: Option<Pad>, cfg: &Config) -> EffectsForm {
        let wm = cur.watermark.or_else(|| crate::export::export_watermark(&Config { export_watermark: true, ..cfg.clone() })).unwrap_or_default();
        let sh = cur.shadow.unwrap_or_default();
        let tn = cur.torn.unwrap_or_default();
        let rf = cur.reflection.unwrap_or_default();
        let on = [cur.watermark.is_some_and(|w| !w.text.is_empty()), cur.shadow.is_some(), cur.torn.is_some_and(|t| t.sides != 0), cur.reflection.is_some()];
        let side = |b: u8| tn.sides & b != 0;
        EffectsForm {
            tab: on.iter().position(|o| *o).unwrap_or(0),
            on,
            wm_kind: usize::from(wm.image),
            wm_text: TextState::new(if wm.image { "" } else { wm.text.as_str() }),
            wm_path: TextState::new(if wm.image { wm.text.as_str() } else { cfg.watermark_image.as_str() }),
            anchor: wm.anchor.min(8) as usize,
            tiled: wm.tiled,
            wm_size: wm.size.round().clamp(1.0, 100.0) as u8,
            wm_opacity: pct(wm.opacity),
            wm_margin: TextState::new(&wm.margin.to_string()),
            wm_color: TextState::new(&hex(wm.color)),
            sh_dx: TextState::new(&sh.dx.to_string()),
            sh_dy: TextState::new(&sh.dy.to_string()),
            sh_blur: sh.blur.round().clamp(0.0, 100.0) as u8,
            sh_opacity: pct(sh.opacity),
            sh_color: TextState::new(&hex(sh.color)),
            sides: [side(effects::TOP), side(effects::RIGHT), side(effects::BOTTOM), side(effects::LEFT)],
            depth: tn.depth.clamp(1, 100) as u8,
            tooth: tn.tooth.clamp(2, 100) as u8,
            seed: tn.seed,
            rf_height: rf.height.round().clamp(1.0, 100.0) as u8,
            rf_opacity: pct(rf.opacity),
            rf_gap: TextState::new(&rf.gap.to_string()),
            canvas,
            saved: false,
            preview: None,
        }
    }

    fn color(t: &TextState) -> Option<C4> {
        crate::config::parse_color(t.text.trim()).map(|(r, g, b, a)| C4::new(r, g, b, a))
    }

    fn num<T: std::str::FromStr>(t: &TextState) -> Option<T> {
        t.text.trim().trim_end_matches("px").trim().parse().ok()
    }

    /// The watermark as the fields say (`None`: a field is wrong).
    pub fn watermark(&self) -> Option<Watermark> {
        let image = self.wm_kind == 1;
        let text = if image { self.wm_path.text.trim() } else { self.wm_text.text.trim() };
        let margin: u32 = Self::num(&self.wm_margin).filter(|m| *m <= effects::MAX_MARGIN)?;
        Some(Watermark {
            image,
            text: Str::new(text),
            anchor: self.anchor.min(8) as u8,
            margin,
            opacity: self.wm_opacity as f32 / 100.0,
            size: self.wm_size.clamp(1, 100) as f32,
            tiled: self.tiled,
            color: Self::color(&self.wm_color)?,
        })
    }

    /// The effects asked for (`None`: a field of an effect that is on is
    /// not a number or a color).
    pub fn result(&self) -> Option<Effects> {
        let mut e = Effects::default();
        if self.on[0] {
            e.watermark = Some(self.watermark()?).filter(|w| !w.text.is_empty());
        }
        if self.on[1] {
            let off = |t: &TextState| Self::num::<i32>(t).filter(|v| v.abs() <= effects::MAX_OFFSET);
            e.shadow = Some(Shadow { dx: off(&self.sh_dx)?, dy: off(&self.sh_dy)?, blur: self.sh_blur as f32, color: Self::color(&self.sh_color)?, opacity: self.sh_opacity as f32 / 100.0 });
        }
        if self.on[2] {
            let bits = [effects::TOP, effects::RIGHT, effects::BOTTOM, effects::LEFT];
            let sides = self.sides.iter().zip(bits).filter(|(on, _)| **on).fold(0, |a, (_, b)| a | b);
            e.torn = Some(Torn { sides, depth: self.depth as u32, tooth: self.tooth as u32, seed: self.seed }).filter(|t| t.sides != 0);
        }
        if self.on[3] {
            let gap: u32 = Self::num(&self.rf_gap).filter(|g| *g <= effects::MAX_MARGIN)?;
            e.reflection = Some(Reflection { height: self.rf_height as f32, opacity: self.rf_opacity as f32 / 100.0, gap });
        }
        Some(e)
    }

    /// The config edits "Use for every export" makes (the watermark's
    /// settings, and Watermark on export on).
    pub fn export_edits(&self) -> Option<Vec<(String, Option<String>)>> {
        let w = self.watermark()?;
        let q = crate::config::toml_str;
        let (text, image) = if w.image { (String::new(), w.text.as_str().to_string()) } else { (w.text.as_str().to_string(), String::new()) };
        Some(vec![
            ("export_watermark".into(), Some("true".into())),
            ("watermark_text".into(), Some(q(&text))),
            ("watermark_image".into(), Some(q(&image))),
            ("watermark_anchor".into(), Some(q(effects::ANCHORS[w.anchor as usize]))),
            ("watermark_margin".into(), Some(w.margin.to_string())),
            ("watermark_opacity".into(), Some(pct(w.opacity).to_string())),
            ("watermark_size".into(), Some((w.size.round() as u32).to_string())),
            ("watermark_tiled".into(), Some(w.tiled.to_string())),
            ("watermark_color".into(), Some(q(&hex(w.color)))),
        ])
    }

    /// The same edits on a config in memory.
    pub fn apply_export(&self, cfg: &mut Config) {
        let Some(w) = self.watermark() else { return };
        cfg.export_watermark = true;
        if w.image {
            cfg.watermark_image = w.text.as_str().to_string();
            cfg.watermark_text.clear();
        } else {
            cfg.watermark_text = w.text.as_str().to_string();
            cfg.watermark_image.clear();
        }
        cfg.watermark_anchor = effects::ANCHORS[w.anchor as usize].to_string();
        cfg.watermark_margin = w.margin;
        cfg.watermark_opacity = pct(w.opacity) as u32;
        cfg.watermark_size = w.size.round() as u32;
        cfg.watermark_tiled = w.tiled;
        cfg.watermark_color = hex(w.color);
    }

    /// `small` (the picture at `k` of its size) with its canvas and the
    /// effects, made again only when the fields change.
    pub fn preview(&mut self, small: &PixBuf, k: f32) -> Option<&PixBuf> {
        let e = self.result()?;
        let key = format!("{e:?}{:?}{k}", self.canvas);
        if self.preview.as_ref().is_none_or(|(k0, _)| *k0 != key) {
            let s = |v: u32| (v as f32 * k).round() as u32;
            let img = match self.canvas {
                Some(p) if !p.is_noop() => {
                    let q = Pad { top: s(p.top), right: s(p.right), bottom: s(p.bottom), left: s(p.left), radius: p.radius * k, ..p };
                    frame::apply_at(small, &q, k)
                }
                _ => small.clone(),
            };
            self.preview = Some((key, effects::apply(img, &e, k)));
        }
        self.preview.as_ref().map(|(_, i)| i)
    }
}

/// The card's height (logical px).
pub const CARD_H: f32 = 520.0;
const LABEL_W: f32 = 96.0;

fn label(ui: &mut Ui, s: &str, tok: &Tok) {
    let h = ui.ctl_h;
    let r = ui.alloc(Some(LABEL_W), h);
    ui.text_at(12.0, s, r.x, r.y + r.h / 2.0, tok.tx2, false);
}

fn unit(ui: &mut Ui, s: &str, tok: &Tok) {
    let w = ui.text_w(12.0, s).ceil();
    let r = ui.alloc(Some(w), ui.ctl_h);
    ui.text_at(12.0, s, r.x, r.y + r.h / 2.0, tok.tx3, false);
}

/// What the body asks for besides Apply / Cancel / No effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extra {
    /// "Use for every export".
    ForExports,
}

/// The dialog's body under its title.
pub fn body(ui: &mut Ui, f: &mut EffectsForm, small: &(PixBuf, f32), tok: &Tok, out: &mut Option<super::image::DlgOut>, extra: &mut Option<Extra>) {
    use super::image::DlgOut;
    ui.row(|ui| {
        ui.seg("fx_tab", &[tr!("fx.tab.watermark"), tr!("fx.tab.shadow"), tr!("fx.tab.torn"), tr!("fx.tab.reflection")], &mut f.tab, tok);
    });
    let on = ui.enabled;
    let tab = f.tab.min(3);
    ui.row(|ui| {
        label(ui, tr!("fx.on"), tok);
        ui.switch(&format!("fx_on{tab}"), &mut f.on[tab], tok);
        if tab == 0 {
            ui.space(14.0);
            ui.enabled = on && f.on[0];
            ui.seg("fx_wm_kind", &[tr!("fx.text"), tr!("fx.image")], &mut f.wm_kind, tok);
            ui.enabled = on;
        }
    });
    ui.enabled = on && f.on[tab];
    match tab {
        0 => {
            ui.row(|ui| {
                label(ui, if f.wm_kind == 1 { tr!("fx.image") } else { tr!("fx.text") }, tok);
                let st = if f.wm_kind == 1 { &mut f.wm_path } else { &mut f.wm_text };
                ui.width(320.0).text_field("fx_wm_text", st);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.position"), tok);
                ui.width(150.0).dropdown("fx_anchor", &ANCHOR_LABELS.map(|k| k.get()), &mut f.anchor);
                ui.space(14.0);
                ui.checkbox("fx_tiled", tr!("fx.tiled"), &mut f.tiled, tok);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.size"), tok);
                ui.width(120.0).slider("fx_wm_size", &mut f.wm_size, 1, 100);
                unit(ui, "%", tok);
                ui.space(10.0);
                unit(ui, tr!("fx.opacity"), tok);
                ui.space(6.0);
                ui.width(100.0).slider("fx_wm_opacity", &mut f.wm_opacity, 0, 100);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.margin"), tok);
                ui.width(56.0).text_field("fx_wm_margin", &mut f.wm_margin);
                unit(ui, "px", tok);
                ui.space(14.0);
                unit(ui, tr!("fx.color"), tok);
                ui.space(6.0);
                ui.width(84.0).text_field("fx_wm_color", &mut f.wm_color);
            });
        }
        1 => {
            ui.row(|ui| {
                label(ui, tr!("fx.offset"), tok);
                unit(ui, "x", tok);
                ui.space(4.0);
                ui.width(56.0).text_field("fx_sh_dx", &mut f.sh_dx);
                ui.space(10.0);
                unit(ui, "y", tok);
                ui.space(4.0);
                ui.width(56.0).text_field("fx_sh_dy", &mut f.sh_dy);
                unit(ui, "px", tok);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.blur"), tok);
                ui.width(200.0).slider("fx_sh_blur", &mut f.sh_blur, 0, 100);
                unit(ui, "px", tok);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.opacity"), tok);
                ui.width(200.0).slider("fx_sh_opacity", &mut f.sh_opacity, 0, 100);
                unit(ui, "%", tok);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.color"), tok);
                ui.width(110.0).text_field("fx_sh_color", &mut f.sh_color);
            });
        }
        2 => {
            ui.row(|ui| {
                label(ui, tr!("fx.sides"), tok);
                for (i, name) in [tr!("edit.img.top"), tr!("edit.img.right"), tr!("edit.img.bottom"), tr!("edit.img.left")].iter().enumerate() {
                    ui.checkbox(&format!("fx_side{i}"), name, &mut f.sides[i], tok);
                    ui.space(8.0);
                }
            });
            ui.row(|ui| {
                label(ui, tr!("fx.depth"), tok);
                ui.width(200.0).slider("fx_depth", &mut f.depth, 1, 100);
                unit(ui, "px", tok);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.teeth"), tok);
                ui.width(200.0).slider("fx_tooth", &mut f.tooth, 2, 100);
                unit(ui, "px", tok);
            });
            ui.row(|ui| {
                label(ui, "", tok);
                if ui.cbutton("fx_seed", tr!("fx.new_pattern"), Some("set-rotate"), false, tok) {
                    f.seed = f.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 16_777_216;
                }
            });
        }
        _ => {
            ui.row(|ui| {
                label(ui, tr!("fx.height"), tok);
                ui.width(200.0).slider("fx_rf_height", &mut f.rf_height, 1, 100);
                unit(ui, "%", tok);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.opacity"), tok);
                ui.width(200.0).slider("fx_rf_opacity", &mut f.rf_opacity, 0, 100);
                unit(ui, "%", tok);
            });
            ui.row(|ui| {
                label(ui, tr!("fx.gap"), tok);
                ui.width(56.0).text_field("fx_rf_gap", &mut f.rf_gap);
                unit(ui, "px", tok);
            });
        }
    }
    ui.enabled = on;
    // The rows above take the same room on every tab (the preview stays put).
    let rows = [4usize, 4, 4, 3][tab];
    if rows < 4 {
        ui.space((ui.ctl_h + 8.0) * (4 - rows) as f32);
    }
    // The preview.
    let pb = ui.alloc(None, 176.0);
    let k = ui.k;
    crate::ui::paint::fill(&mut ui.fb, pb, 8.0 * k, tok.sf2);
    match f.preview(&small.0, small.1) {
        Some(img) => {
            let (iw, ih) = img.dimensions();
            let fit = crate::scale::fit_size(iw, ih, (pb.w - 16.0 * k).max(1.0) as u32, (pb.h - 16.0 * k).max(1.0) as u32);
            let shown = if fit == (iw, ih) { img.clone() } else { crate::imgedit::resample::resize(img, fit.0, fit.1, Filter::Area) };
            let (x0, y0) = ((pb.x + (pb.w - fit.0 as f32) / 2.0).round() as i32, (pb.y + (pb.h - fit.1 as f32) / 2.0).round() as i32);
            for (y, row) in shown.as_raw().chunks_exact(fit.0 as usize * 4).enumerate() {
                for (x, p) in row.as_chunks::<4>().0.iter().enumerate() {
                    ui.fb.blend_px(x0 + x as i32, y0 + y as i32, C4::new(p[0], p[1], p[2], p[3]));
                }
            }
        }
        None => {
            ui.text_at(12.0, tr!("fx.bad"), pb.x + 12.0 * k, pb.y + pb.h / 2.0, tok.er, false);
        }
    }
    if f.saved {
        let r = ui.alloc(None, 18.0);
        ui.text_at(11.5, tr!("fx.saved"), r.x, r.y + r.h / 2.0, tok.tx3, false);
    } else {
        ui.space(18.0);
    }
    ui.row_rtl(|ui| {
        let ok = f.result().is_some();
        ui.enabled = on && ok;
        if ui.cbutton("fx_apply", tr!("fx.apply"), None, true, tok) {
            *out = Some(DlgOut::Apply);
        }
        ui.enabled = on;
        if ui.cbutton("fx_cancel", tr!("fx.cancel"), None, false, tok) {
            *out = Some(DlgOut::Cancel);
        }
        if ui.cbutton("fx_remove", tr!("fx.remove"), None, false, tok) {
            *out = Some(DlgOut::Remove);
        }
        // The watermark tab: keep it for every export.
        if tab == 0 {
            ui.enabled = on && f.on[0];
            if ui.cbutton("fx_for_exports", tr!("fx.for_exports"), None, false, tok) {
                *extra = Some(Extra::ForExports);
            }
            ui.enabled = on;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fields_make_the_effects() {
        let cfg = Config::default();
        let mut f = EffectsForm::new(Effects::default(), None, &cfg);
        assert_eq!(f.on, [false; 4]);
        assert_eq!(f.result(), Some(Effects::default()));
        f.on[0] = true;
        f.wm_text.set("© me");
        f.anchor = 4;
        f.wm_opacity = 50;
        let w = f.result().unwrap().watermark.unwrap();
        assert!(w.text.as_str() == "© me" && w.anchor == 4 && w.opacity == 0.5 && !w.image);
        f.on[1] = true;
        f.sh_dx.set("-4");
        assert_eq!(f.result().unwrap().shadow.unwrap().dx, -4);
        f.sh_dx.set("nope");
        assert!(f.result().is_none(), "a bad offset");
        f.sh_dx.set("4");
        f.on[2] = true;
        f.sides = [false, true, true, false];
        assert_eq!(f.result().unwrap().torn.unwrap().sides, effects::RIGHT | effects::BOTTOM);
        f.sides = [false; 4];
        assert!(f.result().unwrap().torn.is_none(), "no side: none");
        f.on[3] = true;
        f.rf_gap.set("3 px");
        assert_eq!(f.result().unwrap().reflection.unwrap().gap, 3);
        // Opened again on the same effects: the same fields.
        let e = f.result().unwrap();
        let g = EffectsForm::new(e, None, &cfg);
        assert_eq!(g.result(), Some(e));
    }

    #[test]
    fn use_for_every_export_writes_the_watermark_settings() {
        let mut f = EffectsForm::new(Effects::default(), None, &Config::default());
        f.wm_text.set("Mine");
        f.tiled = true;
        let mut cfg = Config::default();
        f.apply_export(&mut cfg);
        assert!(cfg.export_watermark && cfg.watermark_text == "Mine" && cfg.watermark_tiled && cfg.watermark_image.is_empty());
        let edits = f.export_edits().unwrap();
        assert!(edits.contains(&("watermark_text".into(), Some("\"Mine\"".into()))));
        // what export uses now
        assert_eq!(crate::export::export_watermark(&cfg).unwrap().text.as_str(), "Mine");
        // A new dialog starts from the export watermark.
        let g = EffectsForm::new(Effects::default(), None, &cfg);
        assert_eq!(g.wm_text.text, "Mine");
        assert!(!g.on[0], "but off for this picture");
    }

    #[test]
    fn the_preview_follows_the_canvas() {
        let mut f = EffectsForm::new(Effects::default(), Some(Pad::uniform(10, crate::imgedit::Bg::Solid(C4::rgb(255, 255, 255)), 0.0, false)), &Config::default());
        f.on[3] = true;
        let small = PixBuf::from_pixel(40, 30, [200, 30, 30, 255]);
        let (w, h) = f.preview(&small, 0.5).unwrap().dimensions();
        assert_eq!(w, 40 + 10);
        assert!(h > 30 + 10, "the reflection under the canvas");
    }
}
