//! Copy text from an area (OCR): the overlay's Copy text and the
//! `capture_text_hotkey` hand the selection's image to the OS's text
//! recognizer on a worker thread; the text goes to the clipboard and the
//! notice says "Copied text · N lines" (or "No text found").
//!
//! No models are bundled:
//! - Windows: `Windows.Media.Ocr` through hand-rolled WinRT vtables
//!   (`win.rs`), in the user's profile languages.
//! - macOS: Vision's `VNRecognizeTextRequest` (`macos.rs`).
//! - Linux: the `tesseract` command when it is on `PATH` (`linux.rs`),
//!   PNG on stdin, text on stdout; never installed for the user.

use crate::editor::style::ToastKind;
use crate::notice::Note;
use crate::pixbuf::PixBuf;
use std::sync::mpsc::{self, Receiver};

#[cfg(windows)]
mod win;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(windows, target_os = "macos")))]
mod linux;

#[cfg(all(test, windows))]
mod win_tests;

/// Why no text came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OcrError {
    /// Linux: no `tesseract` on `PATH`.
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    NoTesseract,
    /// Windows: no OCR language among the user's languages.
    #[cfg_attr(not(windows), allow(dead_code))]
    NoLanguage,
    /// The engine took longer than [`TIMEOUT`] (Windows, tesseract).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    TimedOut,
    /// Anything else, with the engine's reason.
    Failed(String),
}

/// How long one recognition may take (Vision has no deadline of ours).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// One recognized word and its box (image px, top-left origin).
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))] // tesseract gives plain text
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One line as the engine returned it: its words in reading order.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))] // tesseract gives plain text
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Line {
    pub words: Vec<Word>,
}

#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))] // tesseract gives plain text
impl Line {
    /// The line's top and left edge (its words' smallest y and x).
    fn top_left(&self) -> (f32, f32) {
        let top = self.words.iter().map(|w| w.y).fold(f32::INFINITY, f32::min);
        let left = self.words.iter().map(|w| w.x).fold(f32::INFINITY, f32::min);
        (top, left)
    }
}

/// The text of `lines`: top to bottom (then left to right where two lines
/// start at the same height), words joined with single spaces in the
/// engine's order, one line per row; lines without text are left out.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))] // tesseract gives plain text
pub fn join_lines(mut lines: Vec<Line>) -> String {
    lines.retain(|l| l.words.iter().any(|w| !w.text.trim().is_empty()));
    lines.sort_by(|a, b| {
        let (at, al) = a.top_left();
        let (bt, bl) = b.top_left();
        at.total_cmp(&bt).then(al.total_cmp(&bl))
    });
    lines
        .iter()
        .map(|l| l.words.iter().map(|w| w.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plain engine text (tesseract's stdout) tidied: words on a line joined
/// with single spaces, blank lines and trailing spaces dropped, a form
/// feed (page end) ignored.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
pub fn tidy_text(raw: &str) -> String {
    raw.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Regions whose longer side is at most this (px) are upscaled ×2 before
/// recognition: small text reads much better at twice the size.
pub const UPSCALE_BELOW: u32 = 2000;

/// The factor `w`×`h` is scaled by before recognition (1 or 2), given the
/// engine's largest image side (`None`: no limit).
pub fn upscale_factor(w: u32, h: u32, max_side: Option<u32>) -> u32 {
    let long = w.max(h);
    let fits = max_side.is_none_or(|m| long * 2 <= m);
    if long > 0 && long <= UPSCALE_BELOW && fits { 2 } else { 1 }
}

/// `img` at twice its size, bilinear (pixel centres kept), alpha as is.
pub fn upscale2(img: &PixBuf) -> PixBuf {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img.clone();
    }
    let src = img.as_raw();
    let (ow, oh) = (w * 2, h * 2);
    let mut out = vec![0u8; ow as usize * oh as usize * 4];
    let px = |x: u32, y: u32, c: usize| src[(y as usize * w as usize + x as usize) * 4 + c] as f32;
    for oy in 0..oh {
        // Output pixel centre in source coordinates.
        let sy = ((oy as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, fy) = (sy.floor() as u32, sy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for ox in 0..ow {
            let sx = ((ox as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, fx) = (sx.floor() as u32, sx.fract());
            let x1 = (x0 + 1).min(w - 1);
            let o = (oy as usize * ow as usize + ox as usize) * 4;
            for c in 0..4 {
                let top = px(x0, y0, c) * (1.0 - fx) + px(x1, y0, c) * fx;
                let bot = px(x0, y1, c) * (1.0 - fx) + px(x1, y1, c) * fx;
                out[o + c] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    // Sized above; never panic on the worker (panic = "abort").
    PixBuf::from_raw(ow, oh, out).unwrap_or_else(|| img.clone())
}

/// `img` as recognition wants it: ×2 when small (see [`upscale_factor`]).
pub fn prepare(img: &PixBuf, max_side: Option<u32>) -> (PixBuf, u32) {
    match upscale_factor(img.width(), img.height(), max_side) {
        2 => (upscale2(img), 2),
        _ => (img.clone(), 1),
    }
}

/// Recognize the text in `img` (blocking; call it off the UI thread).
pub fn recognize(img: &PixBuf) -> Result<String, OcrError> {
    if img.width() == 0 || img.height() == 0 {
        return Ok(String::new());
    }
    #[cfg(windows)]
    return win::recognize(img);
    #[cfg(target_os = "macos")]
    return macos::recognize(img);
    #[cfg(not(any(windows, target_os = "macos")))]
    return linux::recognize(img);
}

/// Tests: what recognition returns (no engine runs in the app's tests).
#[cfg(test)]
pub type Fake = fn(&PixBuf) -> Result<String, OcrError>;

#[cfg(test)]
thread_local! {
    pub static FAKE: std::cell::Cell<Option<Fake>> = const { std::cell::Cell::new(None) };
    /// Tests: what [`copy`] put on the "clipboard".
    pub static COPIED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Recognize on a worker thread; the result arrives on the receiver.
pub fn spawn(img: PixBuf) -> Receiver<Result<String, OcrError>> {
    let (tx, rx) = mpsc::channel();
    #[cfg(test)]
    let fake = FAKE.with(|f| f.get());
    let spawned = std::thread::Builder::new().name("rustshot-ocr".into()).spawn({
        let tx = tx.clone();
        move || {
            #[cfg(test)]
            let r = fake.map_or_else(|| Err(OcrError::Failed("no recognizer in tests".into())), |f| f(&img));
            #[cfg(not(test))]
            let r = recognize(&img);
            let _ = tx.send(r);
        }
    });
    if let Err(e) = spawned {
        let _ = tx.send(Err(OcrError::Failed(format!("cannot start a thread: {e}"))));
    }
    rx
}

/// Put recognized text on the clipboard (tests: in [`COPIED`]).
pub fn copy(text: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        COPIED.with(|c| c.borrow_mut().push(text.to_string()));
        Ok(())
    }
    #[cfg(not(test))]
    crate::export::copy_text_to_clipboard(text).map_err(|e| format!("{e:#}"))
}

/// Copy what `res` found (if anything) and say so: the note for it.
pub fn finish(res: &Result<String, OcrError>) -> Note {
    let copied = match res {
        Ok(t) if !t.trim().is_empty() => copy(t),
        _ => Ok(()),
    };
    note(res, copied)
}

/// The number of lines of `text`.
pub fn line_count(text: &str) -> usize {
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// The notice for a finished recognition: "Copied text · 3 lines" (the
/// text was copied: `copied` is the clipboard's result), "No text found",
/// or why it failed.
pub fn note(res: &Result<String, OcrError>, copied: Result<(), String>) -> Note {
    match res {
        Ok(t) if t.trim().is_empty() => Note::plain("No text found", ToastKind::Info),
        Ok(t) => match copied {
            Ok(()) => {
                let n = line_count(t);
                let lines = if n == 1 { "1 line".to_string() } else { format!("{n} lines") };
                Note::plain(format!("Copied text \u{b7} {lines}"), ToastKind::Success)
            }
            Err(e) => Note::plain(format!("Copy failed: {e}"), ToastKind::Error),
        },
        Err(OcrError::NoTesseract) => Note::plain("Install tesseract for text recognition", ToastKind::Error),
        Err(OcrError::TimedOut) => Note::plain("Text recognition timed out", ToastKind::Error),
        Err(OcrError::NoLanguage) => Note::plain("No text recognition language is installed", ToastKind::Error),
        Err(OcrError::Failed(e)) => Note::plain(format!("Text recognition failed: {e}"), ToastKind::Error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: f32, y: f32) -> Word {
        Word { text: text.into(), x, y, w: 10.0, h: 10.0 }
    }

    fn line(words: &[(&str, f32, f32)]) -> Line {
        Line { words: words.iter().map(|(t, x, y)| word(t, *x, *y)).collect() }
    }

    #[test]
    fn lines_go_top_to_bottom_with_words_joined_by_spaces() {
        let lines = vec![
            line(&[("second", 0.0, 40.0), ("row", 50.0, 41.0)]),
            line(&[("Hello", 5.0, 2.0), ("Rustshot", 60.0, 0.0), ("123", 140.0, 1.0)]),
            line(&[("  ", 0.0, 20.0)]),
            line(&[]),
            line(&[("right", 200.0, 40.0)]),
        ];
        assert_eq!(join_lines(lines), "Hello Rustshot 123\nsecond row\nright");
        assert_eq!(join_lines(Vec::new()), "");
        // The engine's word order is kept (right-to-left scripts).
        assert_eq!(join_lines(vec![line(&[("b", 50.0, 0.0), ("a", 0.0, 0.0)])]), "b a");
        // Whitespace inside a word's text is trimmed.
        assert_eq!(join_lines(vec![line(&[(" x ", 0.0, 0.0), ("y\n", 9.0, 0.0)])]), "x y");
    }

    #[test]
    fn tidy_text_drops_blank_lines_and_extra_spaces() {
        assert_eq!(tidy_text("Hello  Rustshot 123 \n\n second\trow\n\u{c}"), "Hello Rustshot 123\nsecond row");
        assert_eq!(tidy_text("\n \n\u{c}"), "");
    }

    #[test]
    fn small_regions_are_upscaled_twice() {
        assert_eq!(upscale_factor(300, 40, None), 2);
        assert_eq!(upscale_factor(2000, 100, None), 2);
        assert_eq!(upscale_factor(2001, 100, None), 1, "large enough");
        assert_eq!(upscale_factor(1200, 900, Some(2000)), 1, "would pass the engine's limit");
        assert_eq!(upscale_factor(1000, 900, Some(2000)), 2);
        assert_eq!(upscale_factor(0, 0, None), 1);
        let img = PixBuf::from_pixel(30, 10, [10, 20, 30, 255]);
        let (big, k) = prepare(&img, None);
        assert_eq!((big.dimensions(), k), ((60, 20), 2));
        let img = PixBuf::from_pixel(2500, 10, [0, 0, 0, 255]);
        assert_eq!(prepare(&img, None).0.dimensions(), (2500, 10));
    }

    #[test]
    fn upscale_is_bilinear_and_keeps_flat_colour() {
        let flat = PixBuf::from_pixel(3, 2, [10, 200, 30, 255]);
        let big = upscale2(&flat);
        assert_eq!(big.dimensions(), (6, 4));
        assert!(big.as_raw().as_chunks::<4>().0.iter().all(|p| *p == [10, 200, 30, 255]));
        // Black | white: the edges keep their colour, the middle blends.
        let mut two = PixBuf::from_pixel(2, 1, [0, 0, 0, 255]);
        two.as_raw_mut()[4..8].copy_from_slice(&[255, 255, 255, 255]);
        let big = upscale2(&two);
        let row: Vec<u8> = (0..4).map(|x| big.get_pixel(x, 0)[0]).collect();
        assert_eq!(row, [0, 64, 191, 255]);
        assert_eq!(big.get_pixel(1, 1), big.get_pixel(1, 0), "rows of a 1-high image agree");
    }

    #[test]
    fn notes_say_what_happened() {
        let ok = |t: &str| Ok::<String, OcrError>(t.into());
        assert_eq!(note(&ok("a\nb\nc"), Ok(())).text, "Copied text \u{b7} 3 lines");
        assert_eq!(note(&ok("Hello"), Ok(())).text, "Copied text \u{b7} 1 line");
        assert_eq!(note(&ok("a"), Ok(())).kind, ToastKind::Success);
        let none = note(&ok(" \n "), Ok(()));
        assert_eq!((none.text.as_str(), none.kind), ("No text found", ToastKind::Info));
        assert_eq!(note(&Err(OcrError::NoTesseract), Ok(())).text, "Install tesseract for text recognition");
        assert_eq!(note(&Err(OcrError::NoTesseract), Ok(())).kind, ToastKind::Error);
        assert_eq!(note(&Err(OcrError::TimedOut), Ok(())).text, "Text recognition timed out");
        assert!(note(&Err(OcrError::Failed("boom".into())), Ok(())).text.ends_with("boom"));
        assert!(note(&ok("a"), Err("busy".into())).text.contains("busy"));
        assert_eq!(line_count("a\n\nb\n"), 2);
    }
}
