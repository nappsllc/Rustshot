//! Windows OCR against a real engine: the baked UI font at 32 px.

use super::*;
use crate::uifb::{C4, Fb};

/// `text` in the baked UI font, black on white, with a margin.
fn rendered(text: &str, px: f32) -> PixBuf {
    let font = crate::fonts::ui_font();
    let tw = crate::uifb::text_width(font, px, text).ceil() as u32;
    let th = crate::uifb::text_height(font, px).ceil() as u32;
    let (w, h) = (tw + 48, th + 32);
    let mut img = PixBuf::from_pixel(w, h, [255, 255, 255, 255]);
    {
        let mut fb = Fb::new(img.as_raw_mut(), w as usize);
        fb.draw_text(font, px, text, 24.0, 16.0, C4::rgb(0, 0, 0));
    }
    img
}

#[test]
fn windows_ocr_reads_the_baked_font() {
    let img = rendered("Hello Rustshot 123", 32.0);
    match recognize(&img) {
        Err(OcrError::NoLanguage) => eprintln!("skipped: no Windows OCR language is installed"),
        Err(e) => panic!("OCR failed: {e:?}"),
        Ok(text) => assert_eq!(text, "Hello Rustshot 123"),
    }
}

#[test]
fn windows_ocr_finds_nothing_on_a_blank_area() {
    let img = PixBuf::from_pixel(120, 40, [255, 255, 255, 255]);
    match recognize(&img) {
        Err(OcrError::NoLanguage) => eprintln!("skipped: no Windows OCR language is installed"),
        r => assert_eq!(r, Ok(String::new())),
    }
}

#[test]
fn windows_ocr_keeps_lines_top_to_bottom() {
    let font = crate::fonts::ui_font();
    let (a, b) = (rendered("Hello Rustshot 123", 32.0), rendered("Second line 456", 32.0));
    let (w, h) = (a.width().max(b.width()), a.height() + b.height());
    let mut img = PixBuf::from_pixel(w, h, [255, 255, 255, 255]);
    {
        let mut fb = Fb::new(img.as_raw_mut(), w as usize);
        fb.draw_text(font, 32.0, "Hello Rustshot 123", 24.0, 16.0, C4::rgb(0, 0, 0));
        fb.draw_text(font, 32.0, "Second line 456", 24.0, 16.0 + a.height() as f32, C4::rgb(0, 0, 0));
    }
    match recognize(&img) {
        Err(OcrError::NoLanguage) => eprintln!("skipped: no Windows OCR language is installed"),
        r => assert_eq!(r, Ok("Hello Rustshot 123\nSecond line 456".to_string())),
    }
}

#[test]
fn bgra_rows_follow_the_stride() {
    let mut img = PixBuf::from_pixel(2, 2, [1, 2, 3, 0]);
    img.as_raw_mut()[4..8].copy_from_slice(&[9, 8, 7, 0]);
    let mut dst = vec![0xEEu8; 12 * 2];
    win::fill_bgra(&mut dst, 12, &img);
    assert_eq!(&dst[..8], &[3, 2, 1, 255, 7, 8, 9, 255]);
    assert_eq!(&dst[8..12], &[0xEE; 4], "padding untouched");
    assert_eq!(&dst[12..16], &[3, 2, 1, 255]);
}
