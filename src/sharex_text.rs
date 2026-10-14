//! The user-visible strings of QR codes, the mouse pointer, the screen
//! ruler and the image effects, in one place (for the `tr!` catalog).

// Settings
pub const INCLUDE_POINTER: &str = "Include mouse pointer";
pub const INCLUDE_POINTER_HELP: &str = "Added as an object you can move or delete";
pub const WATERMARK_ON_EXPORT: &str = "Watermark on export";
pub const WATERMARK_HELP: &str = "This text on every export";

// QR codes
pub const READ_QR: &str = "Read QR code";
pub const COPY_TEXT_QR_ROW: &str = "Copy text \u{b7} Read QR code from area";
pub const MAKE_QR: &str = "Make QR code\u{2026}";
pub const MAKE_QR_TITLE: &str = "Make QR code";
pub const QR_NONE: &str = "No QR code found";
pub const QR_COPY: &str = "Copy";
pub const QR_OPEN: &str = "Open link";
pub const QR_COPIED: &str = "Copied";
pub const QR_TEXT_LABEL: &str = "Text or link";
pub const QR_TOO_LONG: &str = "Too long for a QR code (up to 2331 bytes)";
pub const QR_EMPTY: &str = "Type the text or link to encode";
pub const QR_COPY_IMAGE: &str = "Copy image";
pub const QR_SAVE: &str = "Save\u{2026}";
pub const QR_PIN: &str = "Pin";
pub const QR_CLOSE: &str = "Close";

/// "QR code · 2 found" style toast title.
pub fn qr_found(n: usize) -> String {
    if n == 1 { "QR code".to_string() } else { format!("{n} QR codes") }
}

// Ruler
pub const RULER: &str = "Ruler";
pub const RULER_HINT: &str = "Enter keeps it \u{b7} Esc discards";
pub const RULER_TIP: &str = "Ruler (Shift snaps, Ctrl: area)";

// Effects
pub const EFFECTS: &str = "Effects\u{2026}";
pub const EFFECTS_TITLE: &str = "Effects";
pub const FX_TABS: [&str; 4] = ["Watermark", "Drop shadow", "Torn edge", "Reflection"];
pub const FX_ON: &str = "On";
pub const FX_TEXT: &str = "Text";
pub const FX_IMAGE: &str = "PNG image";
pub const FX_POSITION: &str = "Position";
pub const FX_TILED: &str = "Tiled";
pub const FX_SIZE: &str = "Size";
pub const FX_OPACITY: &str = "Opacity";
pub const FX_MARGIN: &str = "Margin";
pub const FX_COLOR: &str = "Color";
pub const FX_OFFSET: &str = "Offset";
pub const FX_BLUR: &str = "Blur";
pub const FX_SIDES: &str = "Sides";
pub const FX_SIDE_NAMES: [&str; 4] = ["Top", "Right", "Bottom", "Left"];
pub const FX_DEPTH: &str = "Depth";
pub const FX_TEETH: &str = "Teeth";
pub const FX_NEW_PATTERN: &str = "New pattern";
pub const FX_HEIGHT: &str = "Height";
pub const FX_GAP: &str = "Gap";
pub const FX_APPLY: &str = "Apply";
pub const FX_CANCEL: &str = "Cancel";
pub const FX_REMOVE: &str = "No effects";
pub const FX_FOR_EXPORTS: &str = "Use for every export";
pub const FX_BAD: &str = "Check the fields: numbers, and colors such as #ffffff";

// The pointer object
pub const CURSOR: &str = "Mouse pointer";
