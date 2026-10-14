//! The UI size setting (`ui_scale` in the config): `"auto"` draws
//! everything at each monitor's own OS scale; a percentage multiplies it
//! (90 on a 100 % laptop is 0.9×, 150 on a 300 % display 4.5×), so mixed-DPI
//! setups keep their per-monitor scaling.
//!
//! The overlay works from its own config ([`UiSize::apply`]); the windows
//! that have none (Settings, the update dialog, the post-capture notice,
//! macOS' recording bar and camera bubble) read the process-wide value
//! ([`apply`]), which the daemon sets from the config on start and on
//! every reload.

#[cfg(not(test))]
use std::sync::atomic::{AtomicU16, Ordering};

/// The sizes the setting, the Settings dropdown and Ctrl+= / Ctrl+-
/// offer, in percent.
pub const STEPS: [u16; 6] = [75, 90, 100, 125, 150, 200];

/// The Settings dropdown's choices: Auto, then every step.
pub const LABELS: [crate::i18n::Key; STEPS.len() + 1] = [
    key!("uisize.auto"),
    key!("uisize.p75"),
    key!("uisize.p90"),
    key!("uisize.p100"),
    key!("uisize.p125"),
    key!("uisize.p150"),
    key!("uisize.p200"),
];

/// The UI size: the monitor's own scale, or a percentage of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum UiSize {
    #[default]
    Auto,
    /// One of [`STEPS`].
    Pct(u16),
}

impl UiSize {
    /// The config value: `"auto"` (any case) or one of [`STEPS`], as a
    /// number or a string (`125`, `"125"`, `"125%"`).
    pub fn parse(v: &str) -> Option<UiSize> {
        let v = v.trim();
        if v.eq_ignore_ascii_case("auto") {
            return Some(UiSize::Auto);
        }
        let n: u16 = v.strip_suffix('%').unwrap_or(v).trim().parse().ok()?;
        STEPS.contains(&n).then_some(UiSize::Pct(n))
    }

    /// The raw TOML value (`"auto"` or `125`).
    pub fn to_toml(self) -> String {
        match self {
            UiSize::Auto => "\"auto\"".into(),
            UiSize::Pct(n) => n.to_string(),
        }
    }

    /// Percent of the OS scale (Auto: 100).
    pub fn pct(self) -> u16 {
        match self {
            UiSize::Auto => 100,
            UiSize::Pct(n) => n,
        }
    }

    /// The multiplier on the monitor's OS scale.
    pub fn mult(self) -> f32 {
        f32::from(self.pct()) / 100.0
    }

    /// The UI scale on a monitor whose OS scale is `os` (clamped to 1-4,
    /// as the overlay always did, then multiplied).
    pub fn apply(self, os: f32) -> f32 {
        os.clamp(1.0, 4.0) * self.mult()
    }

    /// One step bigger (Ctrl+=), from Auto's 100 %; the largest stays.
    pub fn bigger(self) -> UiSize {
        let p = self.pct();
        UiSize::Pct(STEPS.iter().copied().find(|s| *s > p).unwrap_or(STEPS[STEPS.len() - 1]))
    }

    /// One step smaller (Ctrl+-), from Auto's 100 %; the smallest stays.
    pub fn smaller(self) -> UiSize {
        let p = self.pct();
        UiSize::Pct(STEPS.iter().rev().copied().find(|s| *s < p).unwrap_or(STEPS[0]))
    }

    /// Index in [`LABELS`].
    pub fn index(self) -> usize {
        match self {
            UiSize::Auto => 0,
            UiSize::Pct(n) => STEPS.iter().position(|s| *s == n).map_or(0, |i| i + 1),
        }
    }

    pub fn from_index(i: usize) -> UiSize {
        match i {
            0 => UiSize::Auto,
            i => STEPS.get(i - 1).map_or(UiSize::Auto, |n| UiSize::Pct(*n)),
        }
    }

    /// For a toast: "UI size 125 %", "UI size Auto".
    pub fn describe(self) -> String {
        match self {
            UiSize::Auto => tr!("uisize.toast_auto").into(),
            UiSize::Pct(n) => tr!("uisize.toast", n),
        }
    }
}

/// The UI scales anything is drawn at: 75 % of 100 % up to 200 % of 400 %.
pub const MIN_SCALE: f32 = 0.75;
pub const MAX_SCALE: f32 = 8.0;

/// `s` within [`MIN_SCALE`]..=[`MAX_SCALE`] (a bad value never makes a
/// zero-sized or huge drawing).
pub fn clamp(s: f32) -> f32 {
    if s.is_finite() { s.clamp(MIN_SCALE, MAX_SCALE) } else { 1.0 }
}

/// "Never set in this process" (otherwise 0 = Auto, else the percent).
const UNSET: u16 = u16::MAX;

// The process-wide setting. Tests run in parallel and must not see each
// other's values, so under `cfg(test)` it is per thread (a window test
// sets it on the thread that runs the window).
#[cfg(not(test))]
static CURRENT: AtomicU16 = AtomicU16::new(UNSET);
#[cfg(test)]
thread_local! {
    static CURRENT: std::cell::Cell<u16> = const { std::cell::Cell::new(UNSET) };
}

fn load() -> u16 {
    #[cfg(not(test))]
    return CURRENT.load(Ordering::Relaxed);
    #[cfg(test)]
    return CURRENT.with(std::cell::Cell::get);
}

fn store(v: u16) {
    #[cfg(not(test))]
    CURRENT.store(v, Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.set(v));
}

/// Make `s` the process-wide UI size (the overlay changed it, or Settings
/// applied it).
pub fn set(s: UiSize) {
    store(if s == UiSize::Auto { 0 } else { s.pct() });
}

/// Start-up value from the config file: only when nothing in this process
/// has set the size yet (an overlay's change may still wait for its
/// debounced write, and the file is then older than the process).
pub fn init(s: UiSize) {
    if load() == UNSET {
        set(s);
    }
}

pub fn current() -> UiSize {
    match load() {
        UNSET | 0 => UiSize::Auto,
        n => UiSize::Pct(n),
    }
}

/// The process-wide multiplier.
pub fn mult() -> f32 {
    current().mult()
}

/// The UI scale on a monitor whose OS scale is `os`, for windows without a
/// config of their own: the same formula as the overlay's ([`UiSize::apply`]).
pub fn apply(os: f32) -> f32 {
    current().apply(os)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_auto_and_the_steps_only() {
        assert_eq!(UiSize::parse("auto"), Some(UiSize::Auto));
        assert_eq!(UiSize::parse(" AUTO "), Some(UiSize::Auto));
        for n in STEPS {
            assert_eq!(UiSize::parse(&n.to_string()), Some(UiSize::Pct(n)));
            assert_eq!(UiSize::parse(&format!("{n}%")), Some(UiSize::Pct(n)));
            assert_eq!(UiSize::parse(&format!("{n} %")), Some(UiSize::Pct(n)));
        }
        for bad in ["", "big", "0", "80", "110", "300", "-100", "1.5", "100.0", "%"] {
            assert_eq!(UiSize::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn toml_round_trip() {
        assert_eq!(UiSize::Auto.to_toml(), "\"auto\"");
        assert_eq!(UiSize::Pct(125).to_toml(), "125");
        for s in std::iter::once(UiSize::Auto).chain(STEPS.map(UiSize::Pct)) {
            let t = s.to_toml();
            assert_eq!(UiSize::parse(t.trim_matches('"')), Some(s));
        }
    }

    /// The UI scale is the monitor's OS scale times the setting, per
    /// monitor: mixed DPI keeps its per-monitor ratio.
    #[test]
    fn effective_scale_per_monitor() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(near(UiSize::Auto.apply(1.0), 1.0));
        assert!(near(UiSize::Auto.apply(1.5), 1.5));
        assert!(near(UiSize::Pct(90).apply(1.0), 0.9), "90 on a 100 % laptop shrinks 10 %");
        assert!(near(UiSize::Pct(150).apply(3.0), 4.5), "150 on an 8K at 300 %");
        assert!(near(UiSize::Pct(200).apply(4.0), 8.0));
        assert!(near(UiSize::Pct(75).apply(1.0), 0.75));
        // The OS scale keeps its old clamp (below 100 % and above 400 %).
        assert!(near(UiSize::Pct(100).apply(0.5), 1.0));
        assert!(near(UiSize::Pct(100).apply(6.0), 4.0));
        // Mixed DPI: a 100 % and a 200 % monitor at 125 %.
        let s = UiSize::Pct(125);
        let (a, b) = (s.apply(1.0), s.apply(2.0));
        assert!(near(a, 1.25) && near(b, 2.5) && near(b / a, 2.0));
        assert!(near(UiSize::Pct(100).apply(1.75), UiSize::Auto.apply(1.75)), "100 % is Auto's size");
        // Every product stays within the drawing range.
        for os in [1.0, 1.25, 2.0, 3.0, 4.0] {
            for n in STEPS {
                let s = UiSize::Pct(n).apply(os);
                assert_eq!(clamp(s), s, "{n} % at {os}");
            }
        }
        assert_eq!(clamp(f32::NAN), 1.0);
        assert_eq!(clamp(0.1), MIN_SCALE);
    }

    #[test]
    fn stepping_goes_through_the_list() {
        use UiSize::*;
        assert_eq!(Auto.bigger(), Pct(125), "from Auto's 100 %");
        assert_eq!(Auto.smaller(), Pct(90));
        assert_eq!(Pct(75).smaller(), Pct(75), "the smallest stays");
        assert_eq!(Pct(200).bigger(), Pct(200), "the largest stays");
        let mut s = Pct(75);
        let mut seen = vec![s.pct()];
        while s.bigger() != s {
            s = s.bigger();
            seen.push(s.pct());
        }
        assert_eq!(seen, STEPS);
        while s.smaller() != s {
            s = s.smaller();
        }
        assert_eq!(s, Pct(75));
    }

    #[test]
    fn dropdown_indices() {
        assert_eq!(LABELS[0], "Auto");
        for (i, n) in STEPS.iter().enumerate() {
            assert_eq!(LABELS[i + 1], format!("{n} %"));
        }
        for i in 0..LABELS.len() {
            assert_eq!(UiSize::from_index(i).index(), i);
        }
        assert_eq!(UiSize::from_index(99), UiSize::Auto);
        assert_eq!(UiSize::Pct(100).index(), 3);
    }

    /// The process-wide scale clamps the OS scale like the overlay does.
    #[test]
    fn process_wide_apply_clamps_like_the_overlay() {
        set(UiSize::Pct(125));
        for os in [0.5, 1.0, 1.75, 4.0, 6.0] {
            assert_eq!(apply(os), UiSize::Pct(125).apply(os), "{os}");
        }
        assert_eq!(apply(6.0), 5.0);
        assert_eq!(apply(0.5), 1.25);
    }

    /// The file's value only fills in a size nothing has set.
    #[test]
    fn init_does_not_overwrite_a_set_size() {
        assert_eq!(current(), UiSize::Auto);
        init(UiSize::Pct(90));
        assert_eq!(current(), UiSize::Pct(90), "nothing set it yet");
        init(UiSize::Pct(150));
        assert_eq!(current(), UiSize::Pct(90), "already set");
        set(UiSize::Auto);
        init(UiSize::Pct(150));
        assert_eq!(current(), UiSize::Auto, "an explicit Auto is a value too");
    }

    #[test]
    fn describes_itself() {
        assert_eq!(UiSize::Auto.describe(), "UI size Auto");
        assert_eq!(UiSize::Pct(150).describe(), "UI size 150 %");
    }
}
