//! Per-tool size and color: each drawing tool keeps its own (pencil,
//! line, arrow, rectangle, ellipse, marker, text, number: size and color;
//! pixelate, blur: size; invert: nothing), remembered across captures in
//! `config.toml` (`tool_<id>_size`, `tool_<id>_color`; the rectangle and
//! ellipse also `tool_<id>_fill`; the text and the callout their text
//! style: `tool_<id>_font`, `tool_text_style` / `tool_callout_text_style`,
//! `tool_text_align`). A tool never set starts from the `draw_*` defaults
//! (the number tag: [`COUNTER_SIZE`]).

use super::Tool;
use crate::config::{self, Config};
use crate::text::{Align, Family, TextFmt};
use crate::uifb::C4;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long after the last change the config is written (wheel scrolls
/// coalesce); an overlay that closes first writes at once.
pub const SAVE_DELAY: Duration = Duration::from_millis(300);

/// Every tool: the toolbar's, then Invert (a key only).
pub const ALL: [Tool; N] = {
    let mut v = [Tool::Invert; N];
    let mut i = 0;
    while i < super::toolbar::TOOL_ORDER.len() {
        v[i] = super::toolbar::TOOL_ORDER[i];
        i += 1;
    }
    v
};
const N: usize = super::toolbar::TOOL_ORDER.len() + 1;

/// The number tag's diameter until one is chosen.
pub const COUNTER_SIZE: f32 = 32.0;
/// The callout's text size until one is chosen (the Edit board's 17 px).
pub const CALLOUT_SIZE: f32 = 17.0;
/// The magnifier's loupe radius until one is chosen.
pub const MAGNIFY_SIZE: f32 = 60.0;
/// The blur's strength (the Gaussian's standard deviation, px) until one
/// is chosen.
pub const BLUR_SIZE: f32 = 10.0;

fn slot(t: Tool) -> usize {
    ALL.iter().position(|x| *x == t).expect("every tool has a slot")
}

/// The config id of a tool with remembered settings.
pub fn id(t: Tool) -> Option<&'static str> {
    Some(match t {
        Tool::Path => "pencil",
        Tool::Line => "line",
        Tool::Arrow => "arrow",
        Tool::Rect => "rect",
        Tool::Ellipse => "ellipse",
        Tool::Marker => "marker",
        Tool::Text => "text",
        Tool::Counter => "counter",
        Tool::Pixelate => "pixelate",
        Tool::Callout => "callout",
        Tool::Magnify => "magnify",
        Tool::Blur => "blur",
        Tool::Invert | Tool::Select | Tool::Spotlight => return None,
    })
}

/// The tool types text with a style ([`TextFmt`]: font, bold, italic,
/// underline, strikethrough; the text tool also an alignment).
pub fn has_fmt(t: Tool) -> bool {
    matches!(t, Tool::Text | Tool::Callout)
}

/// The tool has a size (all but Invert).
pub fn has_size(t: Tool) -> bool {
    !matches!(t, Tool::Invert | Tool::Select | Tool::Spotlight)
}

/// The tool draws in a color (not Pixelate, Blur or Invert).
pub fn has_color(t: Tool) -> bool {
    !matches!(t, Tool::Pixelate | Tool::Blur | Tool::Invert | Tool::Select | Tool::Spotlight)
}

/// The tool draws filled or outlined (the rectangle and the ellipse).
pub fn has_fill(t: Tool) -> bool {
    matches!(t, Tool::Rect | Tool::Ellipse | Tool::Callout)
}

/// The size range and step of a tool.
pub fn range(t: Tool) -> (f32, f32) {
    match t {
        Tool::Pixelate | Tool::Invert | Tool::Select | Tool::Spotlight => (4.0, 100.0),
        Tool::Blur => (2.0, 50.0),
        Tool::Text | Tool::Callout => (8.0, 96.0),
        Tool::Magnify => (20.0, 300.0),
        Tool::Counter => (12.0, 120.0),
        _ => (1.0, 50.0),
    }
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn hex(c: C4) -> String {
    if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    }
}

/// A config color (`#rgb`, `#rrggbb`, `#rrggbbaa`).
pub fn parse(s: &str) -> Option<C4> {
    config::parse_color(s).map(|(r, g, b, a)| C4::new(r, g, b, a))
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolMem {
    size: [f32; N],
    color: [C4; N],
    fill: [bool; N],
    fmt: [TextFmt; N],
}

/// The style `cfg` remembers for tool id `id` (`tool_<id>_font`, the
/// style words, the text's `tool_text_align`).
pub fn fmt_of(cfg: &Config, id: &str) -> TextFmt {
    let mut f = TextFmt::default();
    if let Some(fam) = cfg.tool_fonts.get(id).and_then(|s| Family::from_id(s)) {
        f.family = fam;
    }
    if let Some(s) = cfg.tool_text_styles.get(id) {
        f = f.with_style(s);
    }
    if id == "text"
        && let Some(a) = cfg.tool_aligns.get(id).and_then(|s| Align::from_id(s))
    {
        f.align = a;
    }
    f
}

impl ToolMem {
    /// The tools as `cfg` remembers them, the defaults for the rest.
    pub fn from_cfg(cfg: &Config) -> ToolMem {
        let base = parse(&cfg.draw_color).map(|c| c.with_alpha(255)).unwrap_or(C4::rgb(255, 0, 0));
        let mut m = ToolMem { size: [0.0; N], color: [base; N], fill: [false; N], fmt: [TextFmt::default(); N] };
        for t in ALL {
            let (lo, hi) = range(t);
            let default = match t {
                Tool::Marker => cfg.draw_marker_size,
                Tool::Pixelate | Tool::Invert | Tool::Select | Tool::Spotlight => cfg.draw_pixelate_size,
                Tool::Blur => BLUR_SIZE,
                Tool::Text => cfg.draw_font_size,
                Tool::Counter => COUNTER_SIZE,
                Tool::Callout => CALLOUT_SIZE,
                Tool::Magnify => MAGNIFY_SIZE,
                _ => cfg.draw_thickness,
            };
            if has_fmt(t)
                && let Some(i) = id(t)
            {
                m.fmt[slot(t)] = fmt_of(cfg, i);
            }
            let id = id(t);
            let size = id.and_then(|i| cfg.tool_sizes.get(i).copied()).filter(|v| v.is_finite()).unwrap_or(default);
            m.size[slot(t)] = size.clamp(lo, hi);
            if let Some(c) = id.filter(|_| has_color(t)).and_then(|i| cfg.tool_colors.get(i)).and_then(|s| parse(s)) {
                m.color[slot(t)] = c;
            }
            // Callouts start filled (the boards' red bubble).
            m.fill[slot(t)] = has_fill(t) && id.and_then(|i| cfg.tool_fills.get(i)).copied().unwrap_or(t == Tool::Callout);
        }
        m
    }

    /// `t` draws filled (only a shape with [`has_fill`]).
    pub fn fill(&self, t: Tool) -> bool {
        self.fill[slot(t)]
    }

    /// Flip `t`'s fill; false for a tool without one.
    pub fn toggle_fill(&mut self, t: Tool) -> bool {
        if !has_fill(t) {
            return false;
        }
        let v = &mut self.fill[slot(t)];
        *v = !*v;
        true
    }

    pub fn size(&self, t: Tool) -> f32 {
        self.size[slot(t)]
    }

    /// `t`'s text style (plain for a tool without text).
    pub fn fmt(&self, t: Tool) -> TextFmt {
        self.fmt[slot(t)]
    }

    /// Set `t`'s text style; true when it changed (false for a tool
    /// without text). A callout keeps no alignment.
    pub fn set_fmt(&mut self, t: Tool, f: TextFmt) -> bool {
        if !has_fmt(t) {
            return false;
        }
        let f = if t == Tool::Callout { TextFmt { align: Align::Left, ..f } } else { f };
        let v = &mut self.fmt[slot(t)];
        let changed = *v != f;
        *v = f;
        changed
    }

    pub fn color(&self, t: Tool) -> C4 {
        self.color[slot(t)]
    }

    /// Set `t`'s size (clamped to its range).
    pub fn set_size(&mut self, t: Tool, v: f32) {
        let (lo, hi) = range(t);
        self.size[slot(t)] = v.clamp(lo, hi);
    }

    /// Set `t`'s fill; false for a tool without one.
    pub fn set_fill(&mut self, t: Tool, on: bool) -> bool {
        if !has_fill(t) {
            return false;
        }
        self.fill[slot(t)] = on;
        true
    }

    /// Step `t`'s size by `dir` within its range; the new size when it
    /// changed.
    pub fn step(&mut self, t: Tool, dir: i32) -> Option<f32> {
        if !has_size(t) {
            return None;
        }
        let (lo, hi) = range(t);
        let v = &mut self.size[slot(t)];
        let n = (*v + dir as f32).clamp(lo, hi);
        (n != *v).then(|| {
            *v = n;
            n
        })
    }

    /// Set `t`'s color; true when it changed.
    pub fn set_color(&mut self, t: Tool, c: C4) -> bool {
        let v = &mut self.color[slot(t)];
        let changed = *v != c;
        *v = c;
        changed
    }
}

/// Config edits waiting to be written, coalesced per key.
#[derive(Default)]
pub struct Pending {
    edits: BTreeMap<String, Option<String>>,
    /// When they are due.
    due: Option<Instant>,
    /// Every key set this capture (written or not).
    touched: std::collections::BTreeSet<String>,
}

impl Pending {
    /// Set `key` to the raw TOML `value` (`None` removes it), due
    /// [`SAVE_DELAY`] after `now`.
    pub fn set(&mut self, key: String, value: Option<String>, now: Instant) {
        self.touched.insert(key.clone());
        self.edits.insert(key, value);
        self.due = Some(now + SAVE_DELAY);
    }

    pub fn is_due(&self, now: Instant) -> bool {
        self.due.is_some_and(|d| now >= d)
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// The edits, in key order (emptied).
    pub fn take(&mut self) -> Vec<(String, Option<String>)> {
        self.due = None;
        std::mem::take(&mut self.edits).into_iter().collect()
    }

    /// The keys set this capture.
    pub fn touched(&self) -> impl Iterator<Item = &str> {
        self.touched.iter().map(String::as_str)
    }

    /// The value waiting for `key` (tests).
    #[cfg(test)]
    pub fn get(&self, key: &str) -> Option<&Option<String>> {
        self.edits.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_then_remembered() {
        let cfg = Config::default();
        let m = ToolMem::from_cfg(&cfg);
        assert_eq!(m.size(Tool::Line), 3.0);
        assert_eq!(m.size(Tool::Marker), 15.0);
        assert_eq!(m.size(Tool::Pixelate), 12.0);
        assert_eq!(m.size(Tool::Text), 16.0);
        assert_eq!(m.color(Tool::Arrow), C4::rgb(0xf0, 0x44, 0x38));
        let mut cfg = Config::default();
        cfg.tool_sizes.insert("arrow".into(), 7.0);
        cfg.tool_sizes.insert("text".into(), 500.0);
        cfg.tool_colors.insert("arrow".into(), "#00ff0080".into());
        cfg.tool_colors.insert("line".into(), "nonsense".into());
        let m = ToolMem::from_cfg(&cfg);
        assert_eq!(m.size(Tool::Arrow), 7.0);
        assert_eq!(m.size(Tool::Line), 3.0, "the others keep the default");
        assert_eq!(m.size(Tool::Text), 96.0, "clamped");
        assert_eq!(m.color(Tool::Arrow), C4::new(0, 255, 0, 128));
        assert_eq!(m.color(Tool::Line), C4::rgb(0xf0, 0x44, 0x38), "unreadable: the default");
    }

    #[test]
    fn steps_stay_in_range_and_invert_has_no_size() {
        let mut m = ToolMem::from_cfg(&Config::default());
        assert_eq!(m.step(Tool::Line, 1), Some(4.0));
        assert_eq!(m.size(Tool::Path), 3.0, "the pencil is separate");
        for _ in 0..200 {
            m.step(Tool::Pixelate, -1);
        }
        assert_eq!(m.step(Tool::Pixelate, -1), None, "at the floor");
        assert_eq!(m.size(Tool::Pixelate), 4.0);
        assert_eq!(m.step(Tool::Invert, 1), None);
        assert!(m.set_color(Tool::Rect, C4::rgb(1, 2, 3)));
        assert!(!m.set_color(Tool::Rect, C4::rgb(1, 2, 3)));
    }

    #[test]
    fn counter_and_fill_memory() {
        let m = ToolMem::from_cfg(&Config::default());
        assert_eq!(m.size(Tool::Counter), COUNTER_SIZE);
        assert_eq!(id(Tool::Counter), Some("counter"));
        assert!(has_color(Tool::Counter) && has_size(Tool::Counter) && !has_fill(Tool::Counter));
        assert!(!m.fill(Tool::Rect) && !m.fill(Tool::Ellipse));
        let mut cfg = Config::default();
        cfg.tool_sizes.insert("counter".into(), 500.0);
        cfg.tool_colors.insert("counter".into(), "#112233".into());
        cfg.tool_fills.insert("rect".into(), true);
        cfg.tool_fills.insert("line".into(), true);
        let mut m = ToolMem::from_cfg(&cfg);
        assert_eq!(m.size(Tool::Counter), 120.0, "clamped");
        assert_eq!(m.color(Tool::Counter), C4::rgb(0x11, 0x22, 0x33));
        assert!(m.fill(Tool::Rect) && !m.fill(Tool::Ellipse) && !m.fill(Tool::Line));
        assert!(m.toggle_fill(Tool::Ellipse) && m.fill(Tool::Ellipse));
        assert!(!m.toggle_fill(Tool::Line) && !m.fill(Tool::Line));
        for t in ALL {
            assert!(id(t).is_none_or(|i| config::TOOL_IDS.contains(&i)), "{t:?}");
            assert_eq!(has_fill(t), id(t).is_some_and(|i| config::FILL_IDS.contains(&i)), "{t:?}");
        }
    }

    #[test]
    fn color_text_round_trips() {
        for c in [C4::rgb(240, 68, 56), C4::new(1, 2, 3, 4), C4::new(255, 255, 255, 254)] {
            assert_eq!(parse(&hex(c)), Some(c), "{c:?}");
        }
        assert_eq!(hex(C4::rgb(0, 128, 255)), "#0080ff");
    }

    #[test]
    fn pending_coalesces_per_key() {
        let mut p = Pending::default();
        let t0 = Instant::now();
        p.set("tool_line_size".into(), Some("4".into()), t0);
        p.set("tool_line_size".into(), Some("5".into()), t0 + Duration::from_millis(100));
        assert!(!p.is_due(t0 + Duration::from_millis(350)), "the last change restarts the delay");
        assert!(p.is_due(t0 + Duration::from_millis(400)));
        assert_eq!(p.take(), [("tool_line_size".to_string(), Some("5".to_string()))]);
        assert!(p.is_empty() && !p.is_due(t0 + Duration::from_secs(9)));
    }
}
