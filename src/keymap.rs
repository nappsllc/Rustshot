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
    /// A Gaussian-like blur of a rect.
    ToolBlur,
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
    /// Hand the capture to the system's share sheet (Windows, macOS).
    Share,
    Upload,
    /// Pin the selection to the screen (an always-on-top window).
    Pin,
    /// Tighten the selection to its content (its uniform margins cut;
    /// `trim`). Shift+A: A is the arrow.
    Trim,
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
    /// While typing a text or a callout (checked apart from the others:
    /// Ctrl+U is Upload otherwise): bold, italic, underline on / off.
    TextBold,
    TextItalic,
    TextUnderline,
    /// The crop frame: in the overlay over the selection, in the Edit
    /// window over the picture (non-destructive there).
    ToolCrop,
    /// Quick Redact: blur the sensitive text the selection's OCR finds.
    Redact,
    /// The freeform (lasso) selection on / off (remembered).
    SelectFreeform,
    /// The Edit window's image edits (its keys, asked there first, as the
    /// zoom's): a quarter turn left / right, mirror left-right /
    /// top-bottom, the Resize… and Canvas… dialogs, back to the picture
    /// as captured.
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    ImageResize,
    ImageCanvas,
    ImageReset,
    /// The ruler: measure a line (or, with Ctrl, a rect) in px; kept as a
    /// dimension line or discarded. U: free, and next to the other tools.
    ToolRuler,
    /// Read the QR codes in the selection (also in the Copy text
    /// long-press menu). Q for QR.
    ReadQr,
    /// The Edit window's Effects… dialog (watermark, shadow, torn edge,
    /// reflection); no default chord.
    ImageEffects,
}

impl Action {
    pub const ALL: [Action; 65] = [
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
        Action::ToolBlur,
        Action::SizeDown,
        Action::SizeUp,
        Action::Copy,
        Action::CopyText,
        Action::CopyColor,
        Action::Save,
        Action::SaveAs,
        Action::SaveDesktop,
        Action::OpenWith,
        Action::Share,
        Action::Upload,
        Action::Pin,
        Action::Trim,
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
        Action::TextBold,
        Action::TextItalic,
        Action::TextUnderline,
        Action::ToolCrop,
        Action::Redact,
        Action::SelectFreeform,
        Action::RotateLeft,
        Action::RotateRight,
        Action::FlipHorizontal,
        Action::FlipVertical,
        Action::ImageResize,
        Action::ImageCanvas,
        Action::ImageReset,
        Action::ToolRuler,
        Action::ReadQr,
        Action::ImageEffects,
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
        matches!(
            self,
            Action::ZoomIn
                | Action::ZoomOut
                | Action::ZoomFit
                | Action::ZoomActual
                | Action::RotateLeft
                | Action::RotateRight
                | Action::FlipHorizontal
                | Action::FlipVertical
                | Action::ImageResize
                | Action::ImageCanvas
                | Action::ImageReset
                | Action::ImageEffects
        )
    }

    /// An Edit window image key (rotate, flip, Resize…, Canvas…, Reset):
    /// asked there before the editor, so it must not share a chord with
    /// an overlay action (checked in [`Keymap::conflicts`]).
    pub fn image_edit(self) -> bool {
        self.edit_window() && !matches!(self, Action::ZoomIn | Action::ZoomOut | Action::ZoomFit | Action::ZoomActual)
    }

    /// A typing key (the text style): active only while a text or a
    /// callout is typed in, and then before every other action.
    pub fn while_typing(self) -> bool {
        matches!(self, Action::TextBold | Action::TextItalic | Action::TextUnderline)
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
            Action::ToolSelect => ("tool_select", tr!("action.tool_select"), &["V"]),
            Action::ToolPencil => ("tool_pencil", tr!("action.tool_pencil"), &["P"]),
            Action::ToolLine => ("tool_line", tr!("action.tool_line"), &["L"]),
            Action::ToolArrow => ("tool_arrow", tr!("action.tool_arrow"), &["A"]),
            Action::ToolRectangle => ("tool_rectangle", tr!("action.tool_rectangle"), &["R"]),
            Action::ToolCircle => ("tool_circle", tr!("action.tool_circle"), &["E"]),
            Action::ToolMarker => ("tool_marker", tr!("action.tool_marker"), &["M"]),
            Action::ToolText => ("tool_text", tr!("action.tool_text"), &["T"]),
            Action::ToolCallout => ("tool_callout", tr!("action.tool_callout"), &["K"]),
            Action::ToolCounter => ("tool_counter", tr!("action.tool_counter"), &["N"]),
            // X as on the design boards (B is the blur now).
            Action::ToolPixelate => ("tool_pixelate", tr!("action.tool_pixelate"), &["X"]),
            Action::ToolInvert => ("tool_invert", tr!("action.tool_invert"), &["I"]),
            Action::ToolSpotlight => ("tool_spotlight", tr!("action.tool_spotlight"), &["S"]),
            Action::ToolMagnify => ("tool_magnify", tr!("action.tool_magnify"), &["Z"]),
            Action::ToolBlur => ("tool_blur", tr!("action.tool_blur"), &["B"]),
            Action::SizeDown => ("size_down", tr!("action.size_down"), &["["]),
            Action::SizeUp => ("size_up", tr!("action.size_up"), &["]"]),
            Action::Copy => ("copy", tr!("action.copy"), &["Ctrl+C"]),
            Action::CopyText => ("copy_text", tr!("action.copy_text"), &["O", "Ctrl+Shift+C"]),
            Action::CopyColor => ("copy_color", tr!("action.copy_color"), &["H"]),
            Action::Save => ("save", tr!("action.save"), &["Ctrl+S"]),
            Action::SaveAs => ("save_as", tr!("action.save_as"), &["Ctrl+Shift+S"]),
            Action::SaveDesktop => ("save_desktop", tr!("action.save_desktop"), &["Ctrl+D"]),
            Action::OpenWith => ("open_with", tr!("action.open_with"), &["Ctrl+O"]),
            Action::Share => ("share", tr!("action.share"), &["Ctrl+Shift+E"]),
            Action::Upload => ("upload", tr!("action.upload"), &["Ctrl+U"]),
            Action::Pin => ("pin", tr!("action.pin"), &["Ctrl+P"]),
            Action::Trim => ("trim", tr!("action.trim"), &["Shift+A"]),
            Action::Undo => ("undo", tr!("action.undo"), &["Ctrl+Z"]),
            Action::Redo => ("redo", tr!("action.redo"), &["Ctrl+Shift+Z", "Ctrl+Y"]),
            Action::SelectAll => ("select_all", tr!("action.select_all"), &["Ctrl+A"]),
            Action::TogglePalette => ("toggle_palette", tr!("action.toggle_palette"), &["Space"]),
            Action::Accept => ("accept", tr!("action.accept"), &["Enter"]),
            Action::Cancel => ("cancel", tr!("action.cancel"), &["Esc"]),
            Action::Record => ("record", tr!("action.record"), &[]),
            // Ctrl+Shift+= is Ctrl+Plus on US keyboards.
            Action::UiBigger => ("ui_bigger", tr!("action.ui_bigger"), &["Ctrl+=", "Ctrl+Shift+=", "Ctrl+NumpadAdd"]),
            Action::UiSmaller => ("ui_smaller", tr!("action.ui_smaller"), &["Ctrl+-", "Ctrl+NumpadSubtract"]),
            Action::UiReset => ("ui_reset", tr!("action.ui_reset"), &["Ctrl+0", "Ctrl+Numpad0"]),
            Action::RecPause => ("rec_pause", tr!("action.rec_pause"), &["Ctrl+Alt+Shift+P"]),
            Action::RecMute => ("rec_mute", tr!("action.rec_mute"), &["Ctrl+Alt+Shift+M"]),
            Action::RecHidePill => ("rec_hide_pill", tr!("action.rec_hide_pill"), &["Ctrl+Alt+Shift+H"]),
            Action::DeleteObject => ("delete_object", tr!("action.delete_object"), &["Del", "Backspace"]),
            Action::Duplicate => ("duplicate", tr!("action.duplicate"), &["Ctrl+D"]),
            Action::BringFront => ("bring_front", tr!("action.bring_front"), &["Ctrl+]"]),
            Action::SendBack => ("send_back", tr!("action.send_back"), &["Ctrl+["]),
            Action::ZoomIn => ("zoom_in", tr!("action.zoom_in"), &["Ctrl+=", "Ctrl+Shift+=", "Ctrl+NumpadAdd"]),
            Action::ZoomOut => ("zoom_out", tr!("action.zoom_out"), &["Ctrl+-", "Ctrl+NumpadSubtract"]),
            Action::ZoomFit => ("zoom_fit", tr!("action.zoom_fit"), &["Ctrl+0", "Ctrl+Numpad0"]),
            Action::ZoomActual => ("zoom_actual", tr!("action.zoom_actual"), &["Ctrl+1"]),
            Action::TextBold => ("text_bold", tr!("action.text_bold"), &["Ctrl+B"]),
            Action::TextItalic => ("text_italic", tr!("action.text_italic"), &["Ctrl+I"]),
            Action::TextUnderline => ("text_underline", tr!("action.text_underline"), &["Ctrl+U"]),
            // C (Ctrl+C is Copy), Shift+R (R is the rectangle), Shift+F
            // (F is video mode's Screen source).
            Action::ToolCrop => ("tool_crop", tr!("action.tool_crop"), &["C"]),
            Action::Redact => ("redact", tr!("action.redact"), &["Shift+R"]),
            Action::SelectFreeform => ("select_freeform", tr!("action.select_freeform"), &["Shift+F"]),
            // Ctrl+L / Ctrl+R as in photo viewers; the Ctrl+Shift ones are
            // free in the overlay and the Edit window alike.
            Action::RotateLeft => ("rotate_left", tr!("action.rotate_left"), &["Ctrl+L"]),
            Action::RotateRight => ("rotate_right", tr!("action.rotate_right"), &["Ctrl+R"]),
            Action::FlipHorizontal => ("flip_horizontal", tr!("action.flip_horizontal"), &["Ctrl+Shift+H"]),
            Action::FlipVertical => ("flip_vertical", tr!("action.flip_vertical"), &["Ctrl+Shift+V"]),
            Action::ImageResize => ("image_resize", tr!("action.image_resize"), &["Ctrl+Shift+R"]),
            Action::ImageCanvas => ("image_canvas", tr!("action.image_canvas"), &["Ctrl+Shift+K"]),
            Action::ImageReset => ("image_reset", tr!("action.image_reset"), &[]),
            Action::ToolRuler => ("tool_ruler", tr!("action.tool_ruler"), &["U"]),
            Action::ReadQr => ("read_qr", tr!("action.read_qr"), &["Q"]),
            Action::ImageEffects => ("image_effects", tr!("action.image_effects"), &[]),
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.id() == id)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A chord a typing key may have: with Ctrl or Alt (else it would type).
pub fn typing_ok(c: &Chord) -> bool {
    c.ctrl || c.alt
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
            // A typing key without Ctrl or Alt would eat what is typed.
            let (good, plain): (Vec<Chord>, Vec<Chord>) = good.into_iter().partition(|c| !a.while_typing() || typing_ok(c));
            if !plain.is_empty() {
                warn.push(format!("[shortcuts] {k}: a key used while typing needs Ctrl or Alt (ignored)"));
                if good.is_empty() {
                    continue; // keep the default
                }
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
            .filter(|a| !a.while_recording() && !a.on_objects() && !a.edit_window() && !a.while_typing())
            .find(|a| self.chords(*a).iter().any(|c| c.matches(vk, mods)))
    }

    /// The typing key bound to `vk` with these modifiers (asked while a
    /// text or a callout is typed in).
    pub fn resolve_typing(&self, vk: u32, mods: Mods) -> Option<Action> {
        Action::ALL.into_iter().filter(|a| a.while_typing()).find(|a| self.chords(*a).iter().any(|c| c.matches(vk, mods)))
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
        out.extend(self.conflicts_in(4));
        // The Edit window asks its image keys before the editor's: one on
        // an overlay action's chord would take it there (the zoom keys
        // share the UI size's on purpose).
        for a in Action::ALL.into_iter().filter(|a| a.image_edit()) {
            for c in self.chords(a) {
                if let Some(o) = Action::ALL.into_iter().find(|o| !o.while_recording() && !o.on_objects() && !o.edit_window() && !o.while_typing() && self.chords(*o).contains(c)) {
                    out.push((*c, vec![a, o]));
                }
            }
        }
        // The Edit window asks its zoom keys first, typing or not.
        for a in Action::ALL.into_iter().filter(|a| a.while_typing()) {
            for c in self.chords(a) {
                if let Some(z) = Action::ALL.into_iter().find(|z| z.edit_window() && self.chords(*z).contains(c)) {
                    out.push((*c, vec![z, a]));
                }
            }
        }
        out
    }

    /// Typing keys bound without Ctrl or Alt (they would eat what is
    /// typed; [`Keymap::from_config`] drops them, Settings shows them).
    pub fn plain_typing(&self) -> Vec<Action> {
        Action::ALL.into_iter().filter(|a| a.while_typing() && self.chords(*a).iter().any(|c| !typing_ok(c))).collect()
    }

    fn conflicts_in(&self, group: u8) -> Vec<(Chord, Vec<Action>)> {
        let mut by: BTreeMap<Chord, Vec<Action>> = BTreeMap::new();
        let of = |a: &Action| {
            if a.while_recording() {
                1
            } else if a.on_objects() {
                2
            } else if a.edit_window() {
                3
            } else if a.while_typing() {
                4
            } else {
                0
            }
        };
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

    /// Typing keys need Ctrl or Alt (a plain key, Shift+key or F-key is
    /// dropped with a warning, the default kept), and one shared with the
    /// Edit window's zoom is a conflict (the zoom wins there).
    #[test]
    fn typing_keys_need_a_modifier_and_dodge_the_zoom() {
        let cfg = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        for bad in ["B", "Shift+B", "F5"] {
            let (km, warn) = Keymap::from_config(&cfg(&[("text_bold", bad)]));
            assert_eq!(shown(&km, Action::TextBold), ["Ctrl+B"], "{bad}: the default stays");
            assert!(warn.iter().any(|w| w.contains("needs Ctrl or Alt")), "{bad}: {warn:?}");
        }
        let (km, warn) = Keymap::from_config(&cfg(&[("text_bold", "Alt+B, F6")]));
        assert_eq!((shown(&km, Action::TextBold), warn.len()), (vec!["Alt+B".to_string()], 1));
        let (km, _) = Keymap::from_config(&cfg(&[("text_italic", "Ctrl+0")]));
        assert!(km.conflicts().iter().any(|(_, v)| v[..] == [Action::ZoomFit, Action::TextItalic]), "{:?}", km.conflicts());
        let mut km = Keymap::defaults();
        km.set(Action::TextUnderline, vec![Chord::parse("U").unwrap()]);
        assert_eq!(km.plain_typing(), [Action::TextUnderline]);
        assert!(Keymap::defaults().plain_typing().is_empty() && Keymap::defaults().conflicts().is_empty());
    }

    #[test]
    fn defaults_match_spec_table() {
        let km = Keymap::defaults();
        let want: [(Action, &[&str]); 62] = [
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
            (Action::ToolPixelate, &["X"]),
            (Action::ToolInvert, &["I"]),
            (Action::ToolSpotlight, &["S"]),
            (Action::ToolMagnify, &["Z"]),
            (Action::ToolBlur, &["B"]),
            (Action::SizeDown, &["["]),
            (Action::SizeUp, &["]"]),
            (Action::Copy, &["Ctrl+C"]),
            (Action::CopyText, &["O", "Ctrl+Shift+C"]),
            (Action::CopyColor, &["H"]),
            (Action::Save, &["Ctrl+S"]),
            (Action::SaveAs, &["Ctrl+Shift+S"]),
            (Action::SaveDesktop, &["Ctrl+D"]),
            (Action::OpenWith, &["Ctrl+O"]),
            (Action::Share, &["Ctrl+Shift+E"]),
            (Action::Upload, &["Ctrl+U"]),
            (Action::Pin, &["Ctrl+P"]),
            (Action::Trim, &["Shift+A"]),
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
            (Action::TextBold, &["Ctrl+B"]),
            (Action::TextItalic, &["Ctrl+I"]),
            (Action::TextUnderline, &["Ctrl+U"]),
            (Action::RotateLeft, &["Ctrl+L"]),
            (Action::RotateRight, &["Ctrl+R"]),
            (Action::FlipHorizontal, &["Ctrl+Shift+H"]),
            (Action::FlipVertical, &["Ctrl+Shift+V"]),
            (Action::ImageResize, &["Ctrl+Shift+R"]),
            (Action::ImageCanvas, &["Ctrl+Shift+K"]),
            (Action::ImageReset, &[]),
            (Action::ToolRuler, &["U"]),
            (Action::ReadQr, &["Q"]),
            (Action::ImageEffects, &[]),
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
        // Ctrl+R is Rotate right's in the Edit window (asked first there):
        // a conflict; the zoom keys and the UI size share theirs on purpose.
        let ctrl_r = Chord::parse("Ctrl+R").unwrap();
        assert!(km.conflicts().contains(&(ctrl_r, vec![Action::RotateRight, Action::Redo])), "{:?}", km.conflicts());
        assert!(Keymap::defaults().conflicts().is_empty());
        assert!(Action::RotateRight.image_edit() && !Action::ZoomIn.image_edit());
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
        assert_eq!(km.resolve('A' as u32, m(false, true, false)), Some(Action::Trim), "Shift+A trims, A is the arrow");
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
        assert_eq!(km.resolve('B' as u32, NONE), Some(Action::ToolBlur));
        assert_eq!(km.resolve(key::LBRACKET, NONE), Some(Action::SizeDown));
        assert_eq!(km.resolve(key::RBRACKET, NONE), Some(Action::SizeUp));
        assert_eq!(km.resolve('C' as u32, NONE), Some(Action::ToolCrop), "C crops (Ctrl+C copies)");
        assert_eq!(km.resolve('R' as u32, m(false, true, false)), Some(Action::Redact));
        assert_eq!(km.resolve('F' as u32, m(false, true, false)), Some(Action::SelectFreeform));
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
