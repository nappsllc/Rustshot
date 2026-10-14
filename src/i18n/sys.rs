//! The system's preferred UI languages, best first, as BCP 47-ish tags
//! (`de-DE`, `pt_BR.UTF-8`; [`super::best_match`] normalizes them).

/// Windows: the user's preferred UI languages (MUI).
#[cfg(windows)]
pub fn preferred() -> Vec<String> {
    use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
    let (mut n, mut len) = (0u32, 0u32);
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut n, None, &mut len).is_err() || len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut n, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len).is_err() {
            return Vec::new();
        }
        buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
    }
}

/// macOS: `CFLocaleCopyPreferredLanguages` (the order in System Settings).
#[cfg(target_os = "macos")]
pub fn preferred() -> Vec<String> {
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> Ref;
        fn CFArrayGetCount(a: Ref) -> isize;
        fn CFArrayGetValueAtIndex(a: Ref, i: isize) -> Ref;
        fn CFStringGetCString(s: Ref, buf: *mut u8, len: isize, enc: u32) -> u8;
        fn CFRelease(cf: Ref);
    }
    const UTF8: u32 = 0x0800_0100;
    let mut out = Vec::new();
    unsafe {
        let a = CFLocaleCopyPreferredLanguages();
        if a.is_null() {
            return out;
        }
        for i in 0..CFArrayGetCount(a) {
            let s = CFArrayGetValueAtIndex(a, i);
            let mut buf = [0u8; 64];
            if !s.is_null() && CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0 {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(0);
                out.push(String::from_utf8_lossy(&buf[..end]).into_owned());
            }
        }
        CFRelease(a);
    }
    out
}

/// Linux: `LANGUAGE` (a colon list), then `LC_ALL`, `LC_MESSAGES`, `LANG`.
#[cfg(target_os = "linux")]
pub fn preferred() -> Vec<String> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut out: Vec<String> = Vec::new();
    // LANGUAGE only counts with a locale set (gettext's rule).
    let locale = get("LC_ALL").or_else(|| get("LC_MESSAGES")).or_else(|| get("LANG"));
    if let (Some(l), Some(_)) = (get("LANGUAGE"), &locale) {
        out.extend(l.split(':').filter(|s| !s.is_empty()).map(str::to_string));
    }
    out.extend(locale);
    out
}
