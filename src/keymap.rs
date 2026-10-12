//! Remappable editor shortcuts: the actions, their default chords, and the
//! `[shortcuts]` config table (`action = "Ctrl+Shift+S"`, several chords
//! separated by commas, `""` unbinds). Chords use the global-hotkey syntax
//! (`hotkey::parse_hotkey`). Arrow nudges/resizes stay built in.

use crate::hotkey;
use crate::wind::Mods;
use std::collections::BTreeMap;

/// Editor actions in table order (the first wins when two share a chord).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Action {
    /// The select tool: pick, move and resize objects.
    ToolSelect,
    ToolPencil,
    ToolLine,
    ToolArrow,
    ToolRectangle,
    ToolCircle,
    ToolMarker,
    ToolText,
    /// Speech bubbles.
    ToolCallout,
    /// The numbered tags (the counter tool).
    ToolCounter,
    ToolPixelate,
    ToolInvert,
    ToolSpotlight,
    ToolMagnify,
    /// The current tool's stroke size one step down / up (also the wheel).
    SizeDown,
    SizeUp,
    Copy,
    /// Recognize the selection's text and copy it (OCR).
    CopyText,
    /// Copy the hex color under the pointer while the magnifier shows.
    CopyColor,
    Save,
    SaveAs,
    /// Save straight to the Desktop folder (no dialog, no daily folder).
    SaveDesktop,
    /// Open the capture with an app the user picks (the system's chooser).
    OpenWith,
    Upload,
    /// Pin the selection to the screen (an always-on-top window).
    Pin,
    Undo,
    Redo,
    SelectAll,
    TogglePalette,
    Accept,
    Cancel,
    /// Start a screen recording of the selection (no default chord; the
    /// global hotkey is `record_hotkey` in the config).
    /// The overlay is hidden while recording, so stopping is not an
    /// overlay action: the record hotkey toggles, the bar and the tray stop.
    Record,
    /// The overlay's UI one size bigger / smaller / back to Auto
    /// (`ui_scale`; saved like the tools' sizes).
    UiBigger,
    UiSmaller,
    UiReset,
    /// While a recording runs (global chords, registered only then; not
    /// overlay keys): pause/resume, mute the microphone, hide/show the
    /// control pill.
    RecPause,
    RecMute,
    RecHidePill,
    /// With objects selected (they win over the other actions then):
    /// remove them, duplicate them, bring them to the front / send them
    /// to the back.
    DeleteObject,
    Duplicate,
    BringFront,
    SendBack,
    /// The Edit window's zoom (its keys, checked apart from the overlay's:
    /// the UI size has the same chords there).
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ZoomActual,
}

impl Action {
    pub const ALL: [Action; 45] = [
        Action::ToolSelect,
        Action::ToolPencil,
        Action::ToolLine,
        Action::ToolArrow,
        Action::ToolRectangle,
        Action::ToolCircle,
        Action::ToolMarker,
        Action::ToolText,
        Action::ToolCallout,
        Action::ToolCounter,
        Action::ToolPixelate,
        Action::ToolInvert,
        Action::ToolSpotlight,
        Action::ToolMagnify,
        Action::SizeDown,
        Action::SizeUp,
        Action::Copy,
        Action::CopyText,
        Action::CopyColor,
        Action::Save,
        Action::SaveAs,
        Action::SaveDesktop,
        Action::OpenWith,
        Action::Upload,
        Action::Pin,
        Action::Undo,
        Action::Redo,
        Action::SelectAll,
        Action::TogglePalette,
        Action::Accept,
        Action::Cancel,
        Action::Record,
        Action::UiBigger,
        Action::UiSmaller,
        Action::UiReset,
        Action::RecPause,
        Action::RecMute,
        Action::RecHidePill,
        Action::DeleteObject,
        Action::Duplicate,
        Action::BringFront,
        Action::SendBack,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomFit,
        Action::ZoomActual,
    ];

    /// A while-recording chord: global (Meta allowed), not an overlay key.
    pub fn while_recording(self) -> bool {
        matches!(self, Action::RecPause | Action::RecMute | Action::RecHidePill)
    }

    /// An object action: active only while objects are selected, and
    /// then before every other action (its chords may be another's, as
    /// Ctrl+D is Save to Desktop's).
    pub fn on_objects(self) -> bool {
        matches!(self, Action::DeleteObject | Action::Duplicate | Action::BringFront | Action::SendBack)
    }

    /// An Edit window key (its zoom): asked there before the editor's.
    pub fn edit_window(self) -> bool {
        matches!(self, Action::ZoomIn | Action::ZoomOut | Action::ZoomFit | Action::ZoomActual)
    }

    /// Config key in the `[shortcuts]` table.
    pub fn id(self) -> &'static str {
        self.info().0
    }

    /// Description for the Settings shortcut table.
    pub fn label(self) -> &'static str {
        self.info().1
    }

    fn defaults(self) -> &'static [&'static str] {
        self.info().2
    }

    fn info(self) -> (&'static str, &'static str, &'static [&'static str]) {
        match self {
            Action::ToolSelect => ("tool_select", "Select objects", &["V"]),
            Action::ToolPencil => ("tool_pencil", "Pencil", &["P"]),
            Action::ToolLine => ("tool_line", "Line", &["L"]),
            Action::ToolArrow => ("tool_arrow", "Arrow", &["A"]),
            Action::ToolRectangle => ("tool_rectangle", "Rectangle", &["R"]),
            Action::ToolCircle => ("tool_circle", "Ellipse", &["E"]),
            Action::ToolMarker => ("tool_marker", "Marker", &["M"]),
            Action::ToolText => ("tool_text", "Text", &["T"]),
            Action::ToolCallout => ("tool_callout", "Callout", &["K"]),
            Action::ToolCounter => ("tool_counter", "Number", &["N"]),
            // X as on the design boards; B (blur) as before.
            Action::ToolPixelate => ("tool_pixelate", "Pixelate", &["X", "B"]),
            Action::ToolInvert => ("tool_invert", "Invert", &["I"]),
            Action::ToolSpotlight => ("tool_spotlight", "Spotlight", &["S"]),
            Action::ToolMagnify => ("tool_magnify", "Magnify", &["Z"]),
            Action::SizeDown => ("size_down", "Stroke size -", &["["]),
            Action::SizeUp => ("size_up", "Stroke size +", &["]"]),
            Action::Copy => ("copy", "Copy", &["Ctrl+C"]),
            Action::CopyText => ("copy_text", "Copy text", &["O", "Ctrl+Shift+C"]),
            Action::CopyColor => ("copy_color", "Copy color under pointer", &["H"]),
            Action::Save => ("save", "Save", &["Ctrl+S"]),
            Action::SaveAs => ("save_as", "Save As", &["Ctrl+Shift+S"]),
            Action::SaveDesktop => ("save_desktop", "Save to Desktop", &["Ctrl+D"]),
            Action::OpenWith => ("open_with", "Open with\u{2026}", &["Ctrl+O"]),
            Action::Upload => ("upload", "Upload", &["Ctrl+U"]),
            Action::Pin => ("pin", "Pin to screen", &["Ctrl+P"]),
            Action::Undo => ("undo", "Undo", &["Ctrl+Z"]),
            Action::Redo => ("redo", "Redo", &["Ctrl+Shift+Z", "Ctrl+Y"]),
            Action::SelectAll => ("select_all", "Select whole screen", &["Ctrl+A"]),
            Action::TogglePalette => ("toggle_palette", "Toggle palette", &["Space"]),
            Action::Accept => ("accept", "Accept (default action)", &["Enter"]),
            Action::Cancel => ("cancel", "Cancel", &["Esc"]),
            Action::Record => ("record", "Record screen", &[]),
            // Ctrl+Shift+= is Ctrl+Plus on US keyboards.
            Action::UiBigger => ("ui_bigger", "Bigger UI", &["Ctrl+=", "Ctrl+Shift+=", "Ctrl+NumpadAdd"]),
            Action::UiSmaller => ("ui_smaller", "Smaller UI", &["Ctrl+-", "Ctrl+NumpadSubtract"]),
            Action::UiReset => ("ui_reset", "UI size: Auto", &["Ctrl+0", "Ctrl+Numpad0"]),
            Action::RecPause => ("rec_pause", "Pause / resume", &["Ctrl+Alt+Shift+P"]),
            Action::RecMute => ("rec_mute", "Mute microphone", &["Ctrl+Alt+Shift+M"]),
            Action::RecHidePill => ("rec_hide_pill", "Hide / show controls", &["Ctrl+Alt+Shift+H"]),
            Action::DeleteObject => ("delete_object", "Delete selected objects", &["Del", "Backspace"]),
            Action::Duplicate => ("duplicate", "Duplicate selected objects", &["Ctrl+D"]),
            Action::BringFront => ("bring_front", "Bring objects to front", &["Ctrl+]"]),
            Action::SendBack => ("send_back", "Send objects to back", &["Ctrl+["]),
            Action::ZoomIn => ("zoom_in", "Zoom in", &["Ctrl+=", "Ctrl+Shift+=", "Ctrl+NumpadAdd"]),
            Action::ZoomOut => ("zoom_out", "Zoom out", &["Ctrl+-", "Ctrl+NumpadSubtract"]),
            Action::ZoomFit => ("zoom_fit", "Zoom to fit", &["Ctrl+0", "Ctrl+Numpad0"]),
            Action::ZoomActual => ("zoom_actual", "Actual size", &["Ctrl+1"]),
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.id() == id)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// What the Meta modifier is called here: the Windows key, Super on
/// Linux, ⌘ (Cmd) on macOS.
#[cfg(windows)]
pub const META_NAME: &str = "Win";
#[cfg(target_os = "macos")]
pub const META_NAME: &str = "Cmd";
#[cfg(not(any(windows, target_os = "macos")))]
pub const META_NAME: &str = "Super";

/// One key plus modifiers. `vk` uses the Win32 numbering of `wind::key`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub vk: u32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Chord {
    /// `"Ctrl+Shift+S"`, `"Del"`, `"F5"`, `"Meta+X"` (case-insensitive).
    pub fn parse(s: &str) -> Option<Chord> {
        // Win32 modifier flags from parse_hotkey: ALT=1, CONTROL=2, SHIFT=4, WIN=8.
        let (m, vk) = hotkey::parse_hotkey(s)?;
        hotkey::key_name(vk)?; // only keys we can also display
        Some(Chord { vk, ctrl: m & 2 != 0, shift: m & 4 != 0, alt: m & 1 != 0, meta: m & 8 != 0 })
    }

    /// Canonical text form, e.g. `"Ctrl+Shift+S"`.
    pub fn display(&self) -> String {
        let mut out = String::new();
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.meta, "Meta")] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&hotkey::key_name(self.vk).unwrap_or_else(|| format!("0x{:X}", self.vk)));
        out
    }

    /// [`display`](Chord::display) with this OS's name for Meta
    /// ([`META_NAME`]), for messages: `"Shift+Win+X"`.
    pub fn label(&self) -> String {
        let s = self.display();
        if self.meta { s.replacen("Meta", META_NAME, 1) } else { s }
    }

    /// The key name alone ("S", "Space"), for key caps.
    pub fn key(&self) -> String {
        hotkey::key_name(self.vk).unwrap_or_default()
    }

    fn matches(&self, vk: u32, mods: Mods) -> bool {
        // `Mods` carries no meta state (macOS folds ⌘ into ctrl), so a
        // Meta chord never fires inside the editor.
        self.vk == vk && self.ctrl == mods.ctrl && self.shift == mods.shift && self.alt == mods.alt && !self.meta
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    chords: Vec<Vec<Chord>>,
}

fn parse_list(v: &str) -> Vec<Option<Chord>> {
    v.split(',').map(str::trim).filter(|p| !p.is_empty()).map(Chord::parse).collect()
}

impl Keymap {
    pub fn defaults() -> Keymap {
        let chords = Action::ALL
            .iter()
            .map(|a| a.defaults().iter().map(|s| Chord::parse(s).expect("default chord")).collect())
            .collect();
        Keymap { chords }
    }

    /// Defaults with the `[shortcuts]` overrides applied, plus warnings for
    /// unknown actions and unparsable chords (those entries are ignored).
    pub fn from_config(m: &BTreeMap<String, String>) -> (Keymap, Vec<String>) {
        let mut km = Keymap::defaults();
        let mut warn = Vec::new();
        for (k, v) in m {
            let Some(a) = Action::from_id(k.trim()) else {
                warn.push(format!("[shortcuts] unknown action '{k}' (ignored)"));
                continue;
            };
            let parsed = parse_list(v);
            let good: Vec<Chord> = parsed.iter().flatten().copied().collect();
            if good.len() < parsed.len() {
                warn.push(format!("[shortcuts] {k}: invalid key in {v:?}"));
                if good.is_empty() {
                    continue; // keep the default
                }
            }
            if !a.while_recording() && good.iter().any(|c| c.meta) {
                warn.push(format!("[shortcuts] {k}: Meta chords do not reach the editor"));
            }
            km.chords[a.index()] = good;
        }
        (km, warn)
    }

    /// The overlay action bound to `vk` with exactly these modifiers
    /// (never a while-recording chord).
    pub fn resolve(&self, vk: u32, mods: Mods) -> Option<Action> {
        Action::ALL
            .into_iter()
            .filter(|a| !a.while_recording() && !a.on_objects() && !a.edit_window())
            .find(|a| self.chords(*a).iter().any(|c| c.matches(vk, mods)))
    }

    /// The object action bound to `vk` with these modifiers (asked first
    /// while objects are selected).
    pub fn resolve_objects(&self, vk: u32, mods: Mods) -> Option<Action> {
        Action::ALL.into_iter().filter(|a| a.on_objects()).find(|a| self.chords(*a).iter().any(|c| c.matches(vk, mods)))
    }

    /// The Edit window key bound to `vk` with these modifiers (its zoom).
    pub fn resolve_edit_window(&self, vk: u32, mods: Mods) -> Option<Action> {
        Action::ALL.into_iter().filter(|a| a.edit_window()).find(|a| self.chords(*a).iter().any(|c| c.matches(vk, mods)))
    }

    pub fn chords(&self, a: Action) -> &[Chord] {
        &self.chords[a.index()]
    }

    /// Chords bound to more than one action (actions in table order). The
    /// overlay keys, the while-recording chords (the overlay is never open
    /// during a recording) and the object actions (which win while objects
    /// are selected) are apart: each group is checked within itself.
    pub fn conflicts(&self) -> Vec<(Chord, Vec<Action>)> {
        let mut out = self.conflicts_in(0);
        out.extend(self.conflicts_in(1));
        out.extend(self.conflicts_in(2));
        out.extend(self.conflicts_in(3));
        out
    }

    fn conflicts_in(&self, group: u8) -> Vec<(Chord, Vec<Action>)> {
        let mut by: BTreeMap<Chord, Vec<Action>> = BTreeMap::new();
        let of = |a: &Action| if a.while_recording() { 1 } else if a.on_objects() { 2 } else if a.edit_window() { 3 } else { 0 };
        for a in Action::ALL.into_iter().filter(|a| of(a) == group) {
            for c in self.chords(a) {
                let v = by.entry(*c).or_default();
                if !v.contains(&a) {
                    v.push(a);
                }
            }
        }
        by.into_iter().filter(|(_, v)| v.len() > 1).collect()
    }

    /// Bind `a` to exactly `chords` (empty = unbound).
    pub fn set(&mut self, a: Action, chords: Vec<Chord>) {
        self.chords[a.index()] = chords;
    }

    /// The `[shortcuts]` entries that differ from the defaults.
    pub fn to_config(&self) -> BTreeMap<String, String> {
        let def = Keymap::defaults();
        Action::ALL
            .into_iter()
            .filter(|a| self.chords(*a) != def.chords(*a))
            .map(|a| {
                let v: Vec<String> = self.chords(a).iter().map(Chord::display).collect();
                (a.id().to_string(), v.join(", "))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wind::key;

    fn m(ctrl: bool, shift: bool, alt: bool) -> Mods {
        Mods { ctrl, shift, alt }
    }
    const NONE: Mods = Mods { ctrl: false, shift: false, alt: false };

    fn shown(km: &Keymap, a: Action) -> Vec<String> {
        km.chords(a).iter().map(Chord::display).collect()
    }

    #[test]
    fn defaults_match_spec_table() {
        let km = Keymap::defaults();
        let want: [(Action, &[&str]); 45] = [
            (Action::ToolSelect, &["V"]),
            (Action::ToolPencil, &["P"]),
            (Action::ToolLine, &["L"]),
            (Action::ToolArrow, &["A"]),
            (Action::ToolRectangle, &["R"]),
            (Action::ToolCircle, &["E"]),
            (Action::ToolMarker, &["M"]),
            (Action::ToolText, &["T"]),
            (Action::ToolCallout, &["K"]),
            (Action::ToolCounter, &["N"]),
            (Action::ToolPixelate, &["X", "B"]),
            (Action::ToolInvert, &["I"]),
            (Action::ToolSpotlight, &["S"]),
            (Action::ToolMagnify, &["Z"]),
            (Action::SizeDown, &["["]),
            (Action::SizeUp, &["]"]),
            (Action::Copy, &["Ctrl+C"]),
            (Action::CopyText, &["O", "Ctrl+Shift+C"]),
            (Action::CopyColor, &["H"]),
            (Action::Save, &["Ctrl+S"]),
            (Action::SaveAs, &["Ctrl+Shift+S"]),
            (Action::SaveDesktop, &["Ctrl+D"]),
            (Action::Upload, &["Ctrl+U"]),
            (Action::Pin, &["Ctrl+P"]),
            (Action::Undo, &["Ctrl+Z"]),
            (Action::Redo, &["Ctrl+Shift+Z", "Ctrl+Y"]),
            (Action::SelectAll, &["Ctrl+A"]),
            (Action::TogglePalette, &["Space"]),
            (Action::Accept, &["Enter"]),
            (Action::Cancel, &["Esc"]),
            (Action::Record, &[]),
            (Action::UiBigger, &["Ctrl+=", "Ctrl+Shift+=", "Ctrl+NumpadAdd"]),
            (Action::UiSmaller, &["Ctrl+-", "Ctrl+NumpadSubtract"]),
            (Action::UiReset, &["Ctrl+0", "Ctrl+Numpad0"]),
            (Action::RecPause, &["Ctrl+Alt+Shift+P"]),
            (Action::RecMute, &["Ctrl+Alt+Shift+M"]),
            (Action::RecHidePill, &["Ctrl+Alt+Shift+H"]),
            (Action::DeleteObject, &["Del", "Backspace"]),
            (Action::Duplicate, &["Ctrl+D"]),
            (Action::BringFront, &["Ctrl+]"]),
            (Action::SendBack, &["Ctrl+["]),
            (Action::ZoomIn, &["Ctrl+=", "Ctrl+Shift+=", "Ctrl+NumpadAdd"]),
            (Action::ZoomOut, &["Ctrl+-", "Ctrl+NumpadSubtract"]),
            (Action::ZoomFit, &["Ctrl+0", "Ctrl+Numpad0"]),
            (Action::ZoomActual, &["Ctrl+1"]),
        ];
        for (a, keys) in want {
            assert_eq!(shown(&km, a), keys, "{a:?}");
        }
        assert!(km.conflicts().is_empty());
        assert!(km.to_config().is_empty());
    }

    #[test]
    fn ids_are_unique_and_round_trip() {
        for a in Action::ALL {
            assert_eq!(Action::from_id(a.id()), Some(a));
            assert!(!a.label().is_empty());
        }
        assert_eq!(Action::SaveAs.id(), "save_as");
        assert_eq!(Action::ToolPencil.id(), "tool_pencil");
        assert_eq!(Action::Record.id(), "record");
        assert_eq!(Action::from_id("stop_recording"), None, "the record hotkey toggles; no stop action");
        // Record can be bound like any other.
        let (km, w) = Keymap::from_config(&cfg(&[("record", "Ctrl+R")]));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(km.resolve('R' as u32, m(true, false, false)), Some(Action::Record));
    }

    #[test]
    fn parse_display_round_trip() {
        for s in ["Ctrl+Shift+S", "[", "]", "Ctrl+[", "Meta+X", "Del", "Space", "Q", "F5", "F12", "Ctrl+Alt+Shift+Meta+7", "Esc", "Enter", "Ctrl+=", "Ctrl+-", "Ctrl+NumpadAdd", "Ctrl+NumpadSubtract", "Ctrl+Numpad0", "Numpad9"] {
            let c = Chord::parse(s).unwrap_or_else(|| panic!("{s}"));
            assert_eq!(c.display(), s);
            assert_eq!(Chord::parse(&c.display()), Some(c));
        }
        assert_eq!(Chord::parse("shift+ctrl+s").unwrap().display(), "Ctrl+Shift+S");
        assert_eq!(Chord::parse("Win+delete").unwrap().display(), "Meta+Del");
        assert_eq!(Chord::parse("ctrl+plus").unwrap().display(), "Ctrl+=");
        assert_eq!(Chord::parse("Ctrl+Minus").unwrap().display(), "Ctrl+-");
        assert_eq!(Chord::parse("Meta+Shift+X").unwrap().label(), format!("Shift+{META_NAME}+X"));
        assert_eq!(Chord::parse("Ctrl+S").unwrap().label(), "Ctrl+S");
        // The default capture hotkey is written in the display order.
        let def = crate::config::Config::default().capture_hotkey;
        assert_eq!(Chord::parse(&def).unwrap().display(), def);
        let c = Chord::parse("Meta+X").unwrap();
        assert!(c.meta && !c.ctrl && c.vk == 'X' as u32);
        assert_eq!(Chord::parse("Del").unwrap().vk, key::DELETE);
        for bad in ["", "Ctrl", "Ctrl+", "Foo", "A+B", "F13", "Numpad10", "Numpad"] {
            assert_eq!(Chord::parse(bad), None, "{bad}");
        }
    }

    fn cfg(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn override_replaces_default() {
        let (km, w) = Keymap::from_config(&cfg(&[("tool_pencil", "Shift+K"), ("redo", "Ctrl+R, F4")]));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(shown(&km, Action::ToolPencil), ["Shift+K"]);
        assert_eq!(shown(&km, Action::Redo), ["Ctrl+R", "F4"]);
        assert_eq!(km.resolve('P' as u32, NONE), None);
        assert_eq!(km.resolve('K' as u32, m(false, true, false)), Some(Action::ToolPencil));
        let out = km.to_config();
        assert_eq!(out.len(), 2);
        assert_eq!(out["redo"], "Ctrl+R, F4");
        assert_eq!(Keymap::from_config(&out).0, km);
    }

    #[test]
    fn empty_string_unbinds() {
        let (km, w) = Keymap::from_config(&cfg(&[("save", "")]));
        assert!(w.is_empty());
        assert!(km.chords(Action::Save).is_empty());
        assert_eq!(km.resolve('S' as u32, m(true, false, false)), None);
        assert_eq!(km.to_config()["save"], "");
    }

    #[test]
    fn unknown_action_and_bad_chord_warn() {
        let (km, w) = Keymap::from_config(&cfg(&[("tool_stamp", "N"), ("copy", "Ctrl+Nope")]));
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w.iter().any(|s| s.contains("tool_stamp")));
        assert_eq!(shown(&km, Action::Copy), ["Ctrl+C"], "bad value keeps the default");
    }

    #[test]
    fn conflicts_are_reported_and_first_in_table_wins() {
        let (km, _) = Keymap::from_config(&cfg(&[("cancel", "P"), ("undo", "Ctrl+Y")]));
        let c = km.conflicts();
        assert_eq!(c.len(), 2, "{c:?}");
        let p = Chord::parse("P").unwrap();
        assert!(c.contains(&(p, vec![Action::ToolPencil, Action::Cancel])));
        let y = Chord::parse("Ctrl+Y").unwrap();
        assert!(c.contains(&(y, vec![Action::Undo, Action::Redo])));
        assert_eq!(km.resolve('P' as u32, NONE), Some(Action::ToolPencil));
        assert_eq!(km.resolve('Y' as u32, m(true, false, false)), Some(Action::Undo));
    }

    #[test]
    fn resolve_matches_modifiers_exactly() {
        let km = Keymap::defaults();
        assert_eq!(km.resolve('P' as u32, NONE), Some(Action::ToolPencil));
        assert_eq!(km.resolve('P' as u32, m(false, true, false)), None);
        assert_eq!(km.resolve('P' as u32, m(true, false, false)), Some(Action::Pin), "Ctrl+P pins, P is the pencil");
        assert_eq!(km.resolve('P' as u32, m(true, true, false)), None);
        assert_eq!(km.resolve('P' as u32, m(false, false, true)), None);
        assert_eq!(km.resolve('S' as u32, m(true, false, false)), Some(Action::Save));
        assert_eq!(km.resolve('S' as u32, m(true, true, false)), Some(Action::SaveAs));
        assert_eq!(km.resolve('S' as u32, m(true, true, true)), None);
        assert_eq!(km.resolve('D' as u32, m(true, false, false)), Some(Action::SaveDesktop));
        assert_eq!(km.resolve('D' as u32, NONE), None, "D was Line before the redesign");
        assert_eq!(km.resolve('L' as u32, NONE), Some(Action::ToolLine));
        assert_eq!(km.resolve('E' as u32, NONE), Some(Action::ToolCircle));
        assert_eq!(km.resolve('X' as u32, NONE), Some(Action::ToolPixelate));
        assert_eq!(km.resolve('B' as u32, NONE), Some(Action::ToolPixelate));
        assert_eq!(km.resolve(key::LBRACKET, NONE), Some(Action::SizeDown));
        assert_eq!(km.resolve(key::RBRACKET, NONE), Some(Action::SizeUp));
        assert_eq!(km.resolve('C' as u32, NONE), None);
        assert_eq!(km.resolve('O' as u32, NONE), Some(Action::CopyText));
        assert_eq!(km.resolve('H' as u32, NONE), Some(Action::CopyColor));
        assert_eq!(km.resolve('H' as u32, m(true, false, false)), None);
        assert_eq!(km.resolve('C' as u32, m(true, true, false)), Some(Action::CopyText));
        assert_eq!(km.resolve('C' as u32, m(true, false, false)), Some(Action::Copy));
        assert_eq!(km.resolve('N' as u32, NONE), Some(Action::ToolCounter));
        assert_eq!(km.resolve('Z' as u32, m(true, false, false)), Some(Action::Undo));
        assert_eq!(km.resolve('Z' as u32, m(true, true, false)), Some(Action::Redo));
        assert_eq!(km.resolve('A' as u32, m(true, false, false)), Some(Action::SelectAll));
        assert_eq!(km.resolve('A' as u32, NONE), Some(Action::ToolArrow));
        assert_eq!(km.resolve(key::SPACE, NONE), Some(Action::TogglePalette));
        assert_eq!(km.resolve(key::RETURN, NONE), Some(Action::Accept));
        assert_eq!(km.resolve(key::ESCAPE, NONE), Some(Action::Cancel));
        assert_eq!(km.resolve(key::DELETE, NONE), None, "Del is an object action");
        assert_eq!(km.resolve_objects(key::DELETE, NONE), Some(Action::DeleteObject));
        assert_eq!(km.resolve_objects('D' as u32, m(true, false, false)), Some(Action::Duplicate), "over Save to Desktop");
        assert_eq!(km.resolve_objects('D' as u32, NONE), None);
        assert_eq!(km.resolve_objects(key::RBRACKET, m(true, false, false)), Some(Action::BringFront));
        assert_eq!(km.resolve_objects(key::LBRACKET, m(true, false, false)), Some(Action::SendBack));
        assert_eq!(km.resolve('V' as u32, NONE), Some(Action::ToolSelect));
        assert_eq!(km.resolve('K' as u32, NONE), Some(Action::ToolCallout));
        assert_eq!(km.resolve('S' as u32, NONE), Some(Action::ToolSpotlight));
        assert_eq!(km.resolve('Z' as u32, NONE), Some(Action::ToolMagnify));
        let ctrl = m(true, false, false);
        assert_eq!(km.resolve(key::EQUALS, ctrl), Some(Action::UiBigger));
        assert_eq!(km.resolve(key::EQUALS, m(true, true, false)), Some(Action::UiBigger), "Ctrl+Plus");
        assert_eq!(km.resolve(key::ADD, ctrl), Some(Action::UiBigger));
        assert_eq!(km.resolve(key::MINUS, ctrl), Some(Action::UiSmaller));
        assert_eq!(km.resolve(key::SUBTRACT, ctrl), Some(Action::UiSmaller));
        assert_eq!(km.resolve('0' as u32, ctrl), Some(Action::UiReset));
        assert_eq!(km.resolve(key::NUMPAD0, ctrl), Some(Action::UiReset));
        assert_eq!(km.resolve(key::EQUALS, NONE), None, "a bare = is text");
        // Meta chords never fire (no meta state in `Mods`).
        let (km, _) = Keymap::from_config(&cfg(&[("copy", "Meta+C")]));
        assert_eq!(km.resolve('C' as u32, m(true, false, false)), None);
    }

    /// The while-recording chords are remappable like the overlay keys,
    /// Meta is fine for them (no warning), they never resolve in the
    /// overlay, and they clash only with each other.
    #[test]
    fn while_recording_chords() {
        let (km, w) = Keymap::from_config(&cfg(&[("rec_pause", "Ctrl+Alt+P"), ("rec_mute", "Meta+Shift+K"), ("rec_hide_pill", "")]));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(shown(&km, Action::RecPause), ["Ctrl+Alt+P"]);
        assert_eq!(shown(&km, Action::RecMute), ["Shift+Meta+K"]);
        assert!(km.chords(Action::RecHidePill).is_empty());
        assert_eq!(km.resolve('P' as u32, m(true, false, true)), None, "not an overlay key");
        assert!(Action::RecPause.while_recording() && !Action::Record.while_recording());
        // The same chord as an overlay key is no clash; two recording ones are.
        let (km, _) = Keymap::from_config(&cfg(&[("rec_pause", "P")]));
        assert!(km.conflicts().is_empty());
        let (km, _) = Keymap::from_config(&cfg(&[("rec_pause", "Ctrl+Alt+Shift+M")]));
        assert_eq!(km.conflicts(), [(Chord::parse("Ctrl+Alt+Shift+M").unwrap(), vec![Action::RecPause, Action::RecMute])]);
        assert_eq!(Keymap::from_config(&km.to_config()).0, km);
    }
}
