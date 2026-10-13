use crate::config::Config;
use crate::pixbuf::PixBuf;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use crate::chan::{self as mpsc, Receiver};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    /// Save to a specific file or directory. `None` = `auto_save_path`;
    /// with `ask`, a Save As dialog seeded with that folder and name.
    Save { path: Option<PathBuf>, ask: bool },
    /// Save to the Desktop folder ([`crate::desktop`]): the filename
    /// pattern and format, no daily folder, no dialog.
    SaveDesktop,
    Copy,
    /// Print `WxH+X+Y` of the selection to stdout.
    Geometry,
    /// Print raw PNG bytes to stdout.
    Raw,
    Upload,
    /// Recognize the text in the selection and copy it (OCR, [`crate::ocr`]):
    /// the editor runs it on a worker thread after the export.
    Text,
    /// Pin the image to the screen where it was captured (the editor opens
    /// the pin window; nothing to export).
    Pin,
    /// Open the picture with an app the user picks (the system's chooser,
    /// [`crate::open_with`]): the file this export saved, else a
    /// temporary one.
    OpenWith,
    /// Hand the picture to the system's share sheet ([`crate::share`]):
    /// the file this export saved, else a temporary one.
    Share,
}

/// Day of year (1-366) for a civil date.
fn day_of_year(year: i32, month: u16, day: u16) -> u32 {
    const CUM: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let m = month.clamp(1, 12) as usize - 1;
    CUM[m] + day as u32 + if leap && month > 2 { 1 } else { 0 }
}

/// ISO weekday (1 = Monday .. 7 = Sunday) of a civil date.
fn weekday(year: i32, month: u32, day: u32) -> u32 {
    // Sakamoto's method: 0 = Sunday.
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let m = month.clamp(1, 12) as usize;
    let y = if m < 3 { year - 1 } else { year };
    let w = (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400) + T[m - 1] + day as i32).rem_euclid(7);
    if w == 0 { 7 } else { w as u32 }
}

/// ISO 8601 week number (1-53) of a civil date.
fn iso_week(year: i32, month: u32, day: u32) -> u32 {
    // Years with 53 weeks start on a Thursday (or a Wednesday in leap years).
    let weeks = |y: i32| -> i32 {
        let p = |y: i32| (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400)).rem_euclid(7);
        if p(y) == 4 || p(y - 1) == 3 { 53 } else { 52 }
    };
    let doy = day_of_year(year, month as u16, day as u16) as i32;
    let w = (doy - weekday(year, month, day) as i32 + 10) / 7;
    if w < 1 {
        weeks(year - 1) as u32
    } else if w > weeks(year) {
        1
    } else {
        w as u32
    }
}

/// Local wall-clock time: (year, month, day, hour, minute, second).
pub type Tm = (i32, u32, u32, u32, u32, u32);

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Image formats `Save` can write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Bmp,
}

impl Format {
    /// Format for a file extension (case-insensitive, leading dot allowed).
    pub fn from_ext(ext: &str) -> Option<Format> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "png" => Some(Format::Png),
            "jpg" | "jpeg" => Some(Format::Jpeg),
            "bmp" => Some(Format::Bmp),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::Bmp => "bmp",
        }
    }

    /// The configured `save_format` (PNG if unrecognised).
    pub fn from_config(cfg: &Config) -> Format {
        Format::from_ext(cfg.save_format.trim()).unwrap_or(Format::Png)
    }
}

/// The Settings › Capture border (`border_width` logical px at `scale`,
/// at least 1 px when on) and its opaque color; `None` when off.
pub fn border(cfg: &Config, scale: f32) -> Option<(u32, [u8; 4])> {
    let w = cfg.border_width.min(crate::config::BORDER_MAX);
    if w == 0 {
        return None;
    }
    let (r, g, b, _) = crate::config::parse_color(&cfg.border_color).unwrap_or((0, 0, 0, 255));
    Some((((w as f32 * scale.max(0.25)).round() as u32).max(1), [r, g, b, 255]))
}

/// `img` with the Settings › Capture border around it ([`border`]), the
/// image grown by it on every side; `img` itself when there is none.
/// Applied where an export starts (the overlay, the Edit window, History,
/// the direct captures), never to what was exported already (the pin's
/// and the thumbnail's own Copy and Save).
pub fn with_border(img: PixBuf, cfg: &Config, scale: f32) -> PixBuf {
    let Some((b, c)) = border(cfg, scale) else { return img };
    let (w, h) = img.dimensions();
    let (ow, oh) = (w + 2 * b, h + 2 * b);
    let mut out = PixBuf::new(ow, oh);
    let d = out.as_raw_mut();
    for px in d.as_chunks_mut::<4>().0 {
        *px = c;
    }
    let (row, orow) = (w as usize * 4, ow as usize * 4);
    for (y, src) in img.as_raw().chunks_exact(row.max(1)).enumerate() {
        let at = (y + b as usize) * orow + b as usize * 4;
        d[at..at + row].copy_from_slice(src);
    }
    out
}

/// Settings › Saving "Save at": the factor the exported picture of a
/// capture at `scale` is scaled by (1: the screen's pixels; never more).
pub fn save_factor(cfg: &Config, scale: f32) -> f32 {
    match cfg.save_scale.trim() {
        "logical" if scale > 1.0 => 1.0 / scale,
        "custom" => cfg.save_scale_percent.clamp(10, 100) as f32 / 100.0,
        _ => 1.0,
    }
}

/// `img` scaled by `f` (below 1: area-averaged; else itself).
pub fn scaled(img: PixBuf, f: f32) -> PixBuf {
    if f.is_nan() || f >= 0.999 || f <= 0.0 {
        return img;
    }
    let (w, h) = img.dimensions();
    let (nw, nh) = (((w as f32 * f).round() as u32).max(1), ((h as f32 * f).round() as u32).max(1));
    if (nw, nh) == (w, h) {
        return img;
    }
    crate::imgedit::resample::resize(&img, nw, nh, crate::imgedit::Filter::Area)
}

/// `img` (a capture at `scale`) as Copy, Save and Upload hand it
/// out: at the Save at size ([`save_factor`]; `keep_size`: it was resized
/// on purpose already, the Edit window's Resize) with the border round it
/// ([`with_border`], its width at the new size). The history keeps the
/// capture as it was.
pub fn for_output(img: PixBuf, cfg: &Config, scale: f32, keep_size: bool) -> PixBuf {
    let f = if keep_size { 1.0 } else { save_factor(cfg, scale) };
    with_border(scaled(img, f), cfg, scale * f)
}

/// The border [`for_output`] puts round a capture at `scale` (px).
pub fn output_border(cfg: &Config, scale: f32) -> u32 {
    border(cfg, scale * save_factor(cfg, scale)).map_or(0, |(b, _)| b)
}

/// Encode `img` as `fmt`; `quality` (1-100) only affects JPEG.
pub fn encode(img: &PixBuf, fmt: Format, quality: u8) -> Result<Vec<u8>> {
    match fmt {
        Format::Png => img.to_png(),
        // Transparent parts (a freeform capture's outside) are white.
        Format::Jpeg => jpeg_bytes(&crate::objects::mask::flatten_white(img), quality),
        Format::Bmp => bmp_bytes(&crate::objects::mask::flatten_white(img)),
    }
}

/// Baseline JPEG; alpha dropped (capture pixels are opaque). 4:4:4 at
/// quality >= 90 keeps text sharp, 4:2:0 below that.
fn jpeg_bytes(img: &PixBuf, quality: u8) -> Result<Vec<u8>> {
    use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
    let (w, h) = img.dimensions();
    let (Ok(w16), Ok(h16)) = (u16::try_from(w), u16::try_from(h)) else {
        return Err(anyhow!("image too large for JPEG ({w}x{h}; at most 65535 per side)"));
    };
    let q = quality.clamp(1, 100);
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, q);
    enc.set_sampling_factor(if q >= 90 {
        SamplingFactor::R_4_4_4
    } else {
        SamplingFactor::R_4_2_0
    });
    enc.encode(img.as_raw(), w16, h16, ColorType::Rgba)
        .map_err(|e| anyhow!("encode jpeg: {e}"))?;
    Ok(out)
}

/// 32-bit BI_BITFIELDS BMP, rows stored top-down (negative height).
fn bmp_bytes(img: &PixBuf) -> Result<Vec<u8>> {
    const HEADER: u32 = 14 + 40 + 12; // file header + BITMAPINFOHEADER + masks
    let (w, h) = img.dimensions();
    let size = (w as u64 * h as u64 * 4 + HEADER as u64)
        .try_into()
        .ok()
        .filter(|_| w <= i32::MAX as u32 && h <= i32::MAX as u32)
        .ok_or_else(|| anyhow!("image too large for BMP ({w}x{h})"))?;
    let mut o: Vec<u8> = Vec::with_capacity(size as usize);
    let u32le = |o: &mut Vec<u8>, v: u32| o.extend_from_slice(&v.to_le_bytes());
    o.extend_from_slice(b"BM");
    u32le(&mut o, size);
    u32le(&mut o, 0); // reserved
    u32le(&mut o, HEADER); // pixel data offset
    u32le(&mut o, 40); // BITMAPINFOHEADER size
    o.extend_from_slice(&(w as i32).to_le_bytes());
    o.extend_from_slice(&(-(h as i32)).to_le_bytes()); // negative = top-down
    o.extend_from_slice(&1u16.to_le_bytes()); // planes
    o.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    u32le(&mut o, 3); // BI_BITFIELDS
    u32le(&mut o, size - HEADER); // image size
    u32le(&mut o, 2835); // 72 DPI in pixels per metre
    u32le(&mut o, 2835);
    u32le(&mut o, 0); // colors used
    u32le(&mut o, 0); // important colors
    for mask in [0x00FF_0000, 0x0000_FF00, 0x0000_00FF] {
        u32le(&mut o, mask); // R, G, B
    }
    for px in img.as_raw().as_chunks::<4>().0 {
        o.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    Ok(o)
}

/// Make one path component safe on every OS: `<>:"/\|?*` and control
/// characters become `-`, `.`/`..` components are dropped, trailing dots
/// and spaces are trimmed, and a name Windows reserves for a device
/// (`CON`, `NUL`, `COM1`, `LPT1.png`, ...) gets a `_` after its stem. May
/// return an empty string.
pub fn sanitize_component(s: &str) -> String {
    let parts: Vec<String> = s
        .split(['/', '\\'])
        .map(|p| {
            let p: String = p
                .chars()
                .map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '-' } else { c })
                .collect();
            unreserve(p.trim_end_matches(['.', ' ']))
        })
        .filter(|p| !p.is_empty())
        .collect();
    parts.join("-")
}

/// Windows' device names (any case, with any extension: `nul.png` too;
/// `COM¹`-`COM³` and `LPT¹`-`LPT³` included).
fn is_reserved(stem: &str) -> bool {
    let s = stem.trim_end_matches(' ').to_ascii_uppercase();
    if matches!(s.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let mut c = s.chars();
    let head: String = c.by_ref().take(3).collect();
    let tail: Vec<char> = c.collect();
    (head == "COM" || head == "LPT") && tail.len() == 1 && matches!(tail[0], '1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}')
}

/// `p` with a `_` after its stem when that is a reserved device name.
fn unreserve(p: &str) -> String {
    let (stem, rest) = p.split_once('.').map_or((p, None), |(a, b)| (a, Some(b)));
    if !is_reserved(stem) {
        return p.to_string();
    }
    match rest {
        Some(r) => format!("{stem}_.{r}"),
        None => format!("{stem}_"),
    }
}

/// Expanded, sanitised file stem (never empty).
fn file_stem(pattern: &str, now: Tm, epoch: u64) -> String {
    let s = sanitize_component(&expand_pattern(pattern, now, epoch));
    if s.is_empty() { "capture".into() } else { s }
}

/// Where Ctrl+S saves without asking:
/// `save_dir / [subfolder_pattern] / filename_pattern.ext`. The subfolder
/// pattern may nest with `/`; each level is sanitised and `..` dropped.
pub fn auto_save_path(cfg: &Config, now: Tm) -> PathBuf {
    auto_save_path_ext(cfg, now, Format::from_config(cfg).ext())
}

/// [`auto_save_path`] with an explicit extension (`"mp4"`, `"gif"`) in
/// place of the screenshot format's: recordings land beside captures.
pub fn auto_save_path_ext(cfg: &Config, now: Tm, ext: &str) -> PathBuf {
    let epoch = unix_now();
    let mut dir = default_save_dir(cfg);
    if cfg.save_subfolder {
        for part in expand_pattern(&cfg.subfolder_pattern, now, epoch).split(['/', '\\']) {
            let part = sanitize_component(part);
            if !part.is_empty() {
                dir.push(part);
            }
        }
    }
    dir.join(format!("{}.{ext}", file_stem(&cfg.filename_pattern, now, epoch)))
}

/// Format a Flameshot-style filename pattern (`%F`, `%H`, `%M`, ...) for
/// the current local time, sanitised for use as a file name.
pub fn format_filename(pattern: &str) -> String {
    file_stem(pattern, local_ymdhms(), unix_now())
}

/// Expand pattern tokens for `now` (`%s` = `epoch`); no sanitising.
pub(crate) fn expand_pattern(pattern: &str, now: Tm, epoch: u64) -> String {
    let (ty, tmo, td, th, tmi, ts) = now;
    let (y, mo, d) = (ty as i64, tmo as i64, td as i64);
    let (h, mi, s) = (th as i64, tmi as i64, ts as i64);
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(tok) = chars.next() else { break };
        let piece = match tok {
            'F' => format!("{y:04}-{mo:02}-{d:02}"),
            'T' => format!("{h:02}:{mi:02}:{s:02}"),
            'R' => format!("{h:02}:{mi:02}"),
            'H' => format!("{h:02}"),
            'M' => format!("{mi:02}"),
            'S' => format!("{s:02}"),
            'Y' => format!("{y:04}"),
            'y' => format!("{:02}", y.rem_euclid(100)),
            'C' => format!("{:02}", y.div_euclid(100)),
            'e' => format!("{d}"),
            'V' => format!("{:02}", iso_week(ty, tmo, td)),
            'u' => format!("{}", weekday(ty, tmo, td)),
            'm' => format!("{mo:02}"),
            'd' => format!("{d:02}"),
            'j' => format!("{:03}", day_of_year(ty, tmo as u16, td as u16)),
            'p' => (if h < 12 { "AM" } else { "PM" }).to_string(),
            'I' => {
                let h12 = h % 12;
                format!("{:02}", if h12 == 0 { 12 } else { h12 })
            }
            's' => format!("{epoch}"),
            '%' => {
                out.push('%');
                continue;
            }
            other => {
                // Unknown token: keep it literal.
                out.push('%');
                out.push(other);
                continue;
            }
        };
        out.push_str(&piece);
    }
    out
}

/// Local (year, month, day, hour, min, sec) wall-clock time.
#[cfg(unix)]
pub fn local_ymdhms() -> Tm {
    #[repr(C)]
    #[derive(Default)]
    struct CTm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: usize,
    }
    unsafe extern "C" {
        fn time(t: *mut i64) -> i64;
        fn localtime_r(t: *const i64, tm: *mut CTm) -> *mut CTm;
    }
    unsafe {
        let now = time(core::ptr::null_mut());
        let mut tm = CTm::default();
        if localtime_r(&now, &mut tm).is_null() {
            return (1970, 1, 1, 0, 0, 0);
        }
        (
            tm.year + 1900,
            tm.mon as u32 + 1,
            tm.mday as u32,
            tm.hour as u32,
            tm.min as u32,
            tm.sec as u32,
        )
    }
}

/// `save_path`, or `<Pictures>/rustshot` when it is empty (the Pictures
/// folder as `desktop::pictures` finds it: Windows' known folder, which
/// follows OneDrive; `XDG_PICTURES_DIR`; `~/Pictures`), falling back to
/// `<home>/rustshot`, then the current directory. Not created here.
pub fn default_save_dir(cfg: &Config) -> PathBuf {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(unix)]
    let home = std::env::var_os("HOME");
    let home = home.filter(|h| !h.is_empty()).map(PathBuf::from);
    let cwd = || std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    save_dir_with(cfg, crate::desktop::pictures, home, &|p: &Path| p.is_dir(), &cwd)
}

/// [`default_save_dir`] from what it looks up: the Pictures folder (only
/// asked when `save_path` is empty), the home, whether a folder exists,
/// the current directory.
pub fn save_dir_with(
    cfg: &Config,
    pictures: impl FnOnce() -> Option<PathBuf>,
    home: Option<PathBuf>,
    is_dir: &dyn Fn(&Path) -> bool,
    cwd: &dyn Fn() -> PathBuf,
) -> PathBuf {
    if !cfg.save_path.trim().is_empty() {
        return PathBuf::from(&cfg.save_path);
    }
    if let Some(pics) = pictures().filter(|p| !p.as_os_str().is_empty() && is_dir(p)) {
        return pics.join("rustshot");
    }
    if let Some(home) = home.filter(|h| is_dir(h)) {
        return home.join("rustshot");
    }
    cwd()
}

/// Append `_1`, `_2`, ... before the extension until the path is free.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "capture".into());
    let ext = path.extension().map(|e| e.to_string_lossy().to_string());
    for i in 1..10_000 {
        let name = match &ext {
            Some(e) => format!("{stem}_{i}.{e}"),
            None => format!("{stem}_{i}"),
        };
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// Write `img` in the format named by the path's extension, replacing
/// the file if it is there ([`write_atomic`]: a crash or a full disk
/// leaves the old file as it was).
pub fn save_image(img: &PixBuf, path: &Path, jpeg_quality: u8) -> Result<()> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let fmt = Format::from_ext(&ext).ok_or_else(|| {
        anyhow!(
            "unsupported image format {ext:?} (use .png, .jpg or .bmp): {}",
            path.display()
        )
    })?;
    let bytes = encode(img, fmt, jpeg_quality)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).context("create save directory")?;
    }
    write_atomic(path, &bytes).context("write image")?;
    Ok(())
}

/// The suffix of [`write_atomic`]'s temporary files.
pub const TMP_SUFFIX: &str = ".rustshot.tmp";

/// A temporary file's name next to `path`, never the same twice (the
/// process and a counter): `<name>.<pid>-<n>.rustshot.tmp`.
fn tmp_path(path: &Path) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!("{name}.{}-{n}{TMP_SUFFIX}", std::process::id()))
}

/// Write `bytes` to `path` all at once: into a temporary file in the
/// same folder, flushed to disk, then renamed over `path` (replacing it).
/// A reader sees the old file or the new one, never half of one; on an
/// error `path` is left as it was and the temporary file is removed.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let tmp = tmp_path(path);
    let res = (|| -> Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(bytes).with_context(|| format!("write {}", tmp.display()))?;
        f.sync_all().with_context(|| format!("flush {}", tmp.display()))?;
        drop(f);
        replace_file(&tmp, path).with_context(|| format!("replace {}", path.display()))
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Rename `from` over `to` with POSIX semantics
/// (`SetFileInformationByHandle(FileRenameInfoEx)`, `REPLACE_IF_EXISTS |
/// POSIX_SEMANTICS`, Windows 10 1809+ on NTFS): it replaces `to` even
/// while readers hold it open with FILE_SHARE_DELETE (std's `File::open`
/// does), they keep reading the old file. An error where it is not
/// supported (older Windows, FAT, network shares).
#[cfg(windows)]
fn posix_rename(from: &[u16], to: &[u16]) -> std::io::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FileRenameInfoEx, SetFileInformationByHandle, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_RENAME_INFO,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, SYNCHRONIZE,
    };
    const FILE_RENAME_FLAG_REPLACE_IF_EXISTS: u32 = 0x1;
    const FILE_RENAME_FLAG_POSIX_SEMANTICS: u32 = 0x2;
    let err = |e: windows::core::Error| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF);
    // `to` without its NUL, in bytes.
    let name = &to[..to.len().saturating_sub(1)];
    let head = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = head + std::mem::size_of_val(name);
    // u64s: the struct's alignment (a HANDLE inside).
    let mut buf = vec![0u64; size.div_ceil(8) + 1];
    // SAFETY: `from` is NUL-terminated; `buf` holds the struct and the
    // name after it (`size` bytes, 8-byte aligned); the handle is closed.
    unsafe {
        let h = CreateFileW(
            windows::core::PCWSTR(from.as_ptr()),
            (DELETE | SYNCHRONIZE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(err)?;
        let info = buf.as_mut_ptr() as *mut FILE_RENAME_INFO;
        (*info).Anonymous.Flags = FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS;
        (*info).FileNameLength = std::mem::size_of_val(name) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (buf.as_mut_ptr() as *mut u8).add(head) as *mut u16, name.len());
        let r = SetFileInformationByHandle(h, FileRenameInfoEx, info as *const _, size as u32).map_err(err);
        let _ = CloseHandle(h);
        r
    }
}

/// Rename `from` over `to` (replacing it), on the same volume: with POSIX
/// semantics where the volume has them ([`posix_rename`]: open readers do
/// not block it), else `MoveFileExW`, waiting a little for readers.
#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let w = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (f, t) = (w(from), w(to));
    if !to.is_dir() && posix_rename(&f, &t).is_ok() {
        return Ok(());
    }
    // A file someone has open (another thread or process reading it, a
    // virus scanner, the search indexer) cannot be replaced: Access is
    // denied (5) or a sharing violation (32) until they close it, which
    // readers do quickly. Wait for that, up to about 2 s; a folder in the
    // way fails at once.
    const ACCESS_DENIED: i32 = 5;
    const SHARING_VIOLATION: i32 = 32;
    let mut pause = std::time::Duration::from_millis(5);
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        // SAFETY: both are NUL-terminated UTF-16 strings that outlive the call.
        let r = unsafe { MoveFileExW(windows::core::PCWSTR(f.as_ptr()), windows::core::PCWSTR(t.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }
            .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF));
        match r {
            Err(e)
                if matches!(e.raw_os_error(), Some(ACCESS_DENIED | SHARING_VIOLATION))
                    && !to.is_dir()
                    && std::time::Instant::now() < give_up =>
            {
                std::thread::sleep(pause);
                pause = (pause * 2).min(std::time::Duration::from_millis(100));
            }
            r => return r,
        }
    }
}

/// Rename `from` over `to` (replacing it), then flush the folder so the
/// rename itself survives a power loss.
#[cfg(unix)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)?;
    if let Some(dir) = to.parent().filter(|d| !d.as_os_str().is_empty())
        && let Ok(d) = std::fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

pub fn png_bytes(img: &PixBuf) -> Result<Vec<u8>> {
    img.to_png()
}

/// What one export task did, for the confirmation toast.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    Copied,
    Saved(PathBuf),
    /// Saved by Save to Desktop.
    SavedDesktop(PathBuf),
    /// (task, reason): "Copy", "Save" or "Upload".
    Failed(&'static str, String),
}

/// The save dialog's Cancel: the export says so, nothing failed.
pub const SAVE_CANCELLED: &str = "save dialog cancelled";

pub struct ExportResult {
    pub messages: Vec<String>,
    pub error: bool,
    /// What Copy, Save and Upload did (Upload: only when it could not start).
    pub done: Vec<Done>,
    /// Handle for a spawned upload; the caller must wait for it.
    pub upload: Option<Receiver<Result<String, String>>>,
}

/// Run the synchronous export tasks against the final (cropped) image.
/// `sel_global` is the selection rect in virtual-screen physical coords.
pub fn run_export(
    img: &PixBuf,
    sel_global: (i32, i32),
    tasks: &[Task],
    cfg: &Config,
) -> ExportResult {
    let mut messages = Vec::new();
    let mut error = false;
    let mut upload = None;
    let mut done = Vec::new();

    for task in tasks {
        match task {
            Task::Geometry => {
                println!("{}x{}+{}+{}", img.width(), img.height(), sel_global.0, sel_global.1);
            }
            Task::Raw => match png_bytes(img) {
                Ok(bytes) => {
                    use std::io::Write;
                    let out = std::io::stdout();
                    let mut lock = out.lock();
                    if let Err(e) = lock.write_all(&bytes).and_then(|_| lock.flush()) {
                        messages.push(format!("error: write png to stdout: {e}"));
                        error = true;
                    }
                }
                Err(e) => {
                    messages.push(format!("error: encode png: {e:#}"));
                    error = true;
                }
            },
            Task::Copy => match copy_to_clipboard(img) {
                Ok(()) => {
                    messages.push("copied to clipboard".to_string());
                    done.push(Done::Copied);
                }
                Err(e) => {
                    messages.push(format!("error: clipboard: {e:#}"));
                    done.push(Done::Failed(tr!("export.copy"), format!("{e:#}")));
                    error = true;
                }
            },
            Task::Save { path, ask } => match resolve_save_path(path, *ask, cfg) {
                Ok(p) => {
                    let target = unique_path(&p);
                    match save_image(img, &target, cfg.jpeg_quality) {
                        Ok(()) => {
                            messages.push(format!("saved: {}", target.display()));
                            done.push(Done::Saved(target));
                        }
                        Err(e) => {
                            messages.push(format!("error: save: {e:#}"));
                            done.push(Done::Failed(tr!("export.save"), format!("{e:#}")));
                            error = true;
                        }
                    }
                }
                Err(e) => {
                    messages.push(format!("error: save: {e:#}"));
                    if e.to_string() != SAVE_CANCELLED {
                        done.push(Done::Failed(tr!("export.save"), format!("{e:#}")));
                    }
                    error = true;
                }
            },
            Task::SaveDesktop => match desktop_path(cfg, crate::desktop::dir()) {
                Some(p) => {
                    let target = unique_path(&p);
                    match save_image(img, &target, cfg.jpeg_quality) {
                        Ok(()) => {
                            messages.push(format!("saved: {}", target.display()));
                            done.push(Done::SavedDesktop(target));
                        }
                        Err(e) => {
                            messages.push(format!("error: save: {e:#}"));
                            done.push(Done::Failed(tr!("export.save"), format!("{e:#}")));
                            error = true;
                        }
                    }
                }
                None => {
                    messages.push("error: save: no Desktop folder".to_string());
                    done.push(Done::Failed(tr!("export.save"), tr!("export.no_desktop").into()));
                    error = true;
                }
            },
            Task::Upload => {
                let bytes = match png_bytes(img) {
                    Ok(b) => b,
                    Err(e) => {
                        messages.push(format!("error: encode for upload: {e:#}"));
                        done.push(Done::Failed(tr!("export.upload"), format!("{e:#}")));
                        error = true;
                        continue;
                    }
                };
                upload = Some(spawn_upload(bytes, cfg.clone()));
            }
            Task::OpenWith => {
                let saved = done.iter().rev().find_map(|d| match d {
                    Done::Saved(p) | Done::SavedDesktop(p) => Some(p.clone()),
                    _ => None,
                });
                match crate::open_with::file_for(img, saved.as_deref(), cfg) {
                    Ok(p) => {
                        messages.push(format!("open with: {}", p.display()));
                        crate::open_with::open(&p);
                    }
                    Err(e) => {
                        messages.push(format!("error: open with: {e:#}"));
                        done.push(Done::Failed(tr!("history.open_with"), format!("{e:#}")));
                        error = true;
                    }
                }
            }
            Task::Share => {
                let saved = done.iter().rev().find_map(|d| match d {
                    Done::Saved(p) | Done::SavedDesktop(p) => Some(p.clone()),
                    _ => None,
                });
                match crate::share::file_for(img, saved.as_deref(), cfg) {
                    Ok(p) => {
                        messages.push(format!("share: {}", p.display()));
                        crate::share::share(&p);
                    }
                    Err(e) => {
                        messages.push(format!("error: share: {e:#}"));
                        done.push(Done::Failed(tr!("history.share"), format!("{e:#}")));
                        error = true;
                    }
                }
            }
            Task::Text => {} // the caller's (it runs off the UI thread)
            // The editor opens the pin (`App::run_finish`).
            Task::Pin => {}
        }
    }
    ExportResult {
        messages,
        error,
        done,
        upload,
    }
}

/// Where Save to Desktop writes in `desktop`: the filename pattern with
/// the configured format (collisions get a suffix when saving).
pub fn desktop_path(cfg: &Config, desktop: Option<PathBuf>) -> Option<PathBuf> {
    let ext = Format::from_config(cfg).ext();
    Some(desktop?.join(format!("{}.{ext}", format_filename(&cfg.filename_pattern))))
}

/// Figure out where `Save` should write, possibly asking the user.
fn resolve_save_path(path: &Option<PathBuf>, ask: bool, cfg: &Config) -> Result<PathBuf> {
    let ext = Format::from_config(cfg).ext();
    match path {
        // A file path with an extension is used as-is; anything else is a
        // directory (even if it does not exist yet).
        Some(p) if !p.is_dir() && p.extension().is_some() => Ok(p.clone()),
        Some(p) => Ok(p.join(format!("{}.{ext}", format_filename(&cfg.filename_pattern)))),
        None if !ask => Ok(auto_save_path(cfg, local_ymdhms())),
        None => {
            let auto = auto_save_path(cfg, local_ymdhms());
            let dir = auto.parent().unwrap_or(Path::new("."));
            let name = auto
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            // Seed the dialog with the dated folder; undo its creation if
            // the user cancels or picks another folder.
            let created = create_dirs(dir);
            let chosen = save_dialog(dir, &name);
            if chosen.as_deref().and_then(Path::parent) != Some(dir) {
                for d in &created {
                    let _ = std::fs::remove_dir(d); // only succeeds if empty
                }
            }
            let mut p = chosen.ok_or_else(|| anyhow!(SAVE_CANCELLED))?;
            if p.extension().is_none() {
                p.set_extension(ext);
            }
            Ok(p)
        }
    }
}

/// Create `dir` and its missing parents; returns the ones created,
/// deepest first.
fn create_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if d.as_os_str().is_empty() || d.exists() {
            break;
        }
        missing.push(d.to_path_buf());
        cur = d.parent();
    }
    if std::fs::create_dir_all(dir).is_err() {
        return Vec::new();
    }
    missing
}

/// Blocking wait used after the UI has exited (one-shot processes).
pub fn wait_upload(rx: Receiver<Result<String, String>>, copy_url: bool) -> Option<String> {
    match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(Ok(url)) => {
            println!("uploaded: {url}");
            if copy_url
                && let Err(e) = copy_text_to_clipboard(&url) {
                    eprintln!("warning: could not copy URL: {e:#}");
                }
            Some(url)
        }
        Ok(Err(e)) => {
            eprintln!("error: upload failed: {e}");
            None
        }
        Err(_) => {
            eprintln!("error: upload timed out");
            None
        }
    }
}

/// Tests: what an upload returns (no network ever: unset, it fails).
#[cfg(test)]
pub type FakeUpload = fn(&[u8]) -> Result<String, String>;

#[cfg(test)]
thread_local! {
    pub static FAKE_UPLOAD: std::cell::Cell<Option<FakeUpload>> = const { std::cell::Cell::new(None) };
}

/// Upload `png` on a worker to the configured destination: Imgur, or
/// the S3 bucket (named by the file name pattern, `.png`).
fn spawn_upload(png: Vec<u8>, cfg: Config) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    #[cfg(test)]
    let fake = FAKE_UPLOAD.with(|f| f.get());
    crate::busy::spawn(move || {
        #[cfg(test)]
        let r = {
            let _ = &cfg;
            fake.map_or_else(|| Err("no uploader in tests".to_string()), |f| f(&png))
        };
        #[cfg(not(test))]
        let r = match crate::s3::Dest::of(&cfg) {
            crate::s3::Dest::S3 => {
                let name = format!("{}.png", format_filename(&cfg.filename_pattern));
                crate::s3::upload_now(&cfg, crate::s3::Source::Bytes(&png, &name))
            }
            crate::s3::Dest::Imgur => do_upload(&png, &cfg.upload_client_id),
        };
        // The record's thumbnail (the deletehash came with the reply).
        if let Ok(link) = &r {
            crate::uploads::note_thumb(link, crate::uploads::thumb_png(&png));
        }
        let _ = tx.send(r);
    });
    rx
}

use imp::save_dialog;

#[cfg(windows)]
#[path = "export_win.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "export_linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "export_macos.rs"]
mod imp;

pub use imp::*;

#[cfg(not(windows))]
#[path = "export_curl.rs"]
mod curl;
#[cfg(target_os = "linux")]
pub use curl::download_http_exact;
#[cfg(not(windows))]
pub use curl::download_to;
#[cfg(all(not(windows), not(test)))]
pub use curl::http_request;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_tokens() {
        let s = format_filename("%F_%H-%M");
        assert!(s.contains('_'), "{s}");
        assert!(!s.contains('%'), "{s}");
        assert_eq!(format_filename("shot"), "shot");
        assert_eq!(format_filename("a%Qb"), "a%Qb");
    }

    const NOW: Tm = (2026, 3, 7, 9, 5, 2);

    #[test]
    fn settings_tokens() {
        // 2026-03-07 is a Saturday in ISO week 10.
        let e = |p: &str, t: Tm| expand_pattern(p, t, 0);
        assert_eq!(e("%C|%e|%V|%u", NOW), "20|7|10|6");
        assert_eq!(e("%e", (2026, 3, 17, 0, 0, 0)), "17");
        assert_eq!(e("%C", (1999, 1, 1, 0, 0, 0)), "19");
        // Week-year boundaries (ISO 8601).
        for (t, w, u) in [
            ((2021, 1, 1), 53, 5),  // Friday, week 53 of 2020
            ((2021, 1, 4), 1, 1),   // Monday
            ((2024, 12, 30), 1, 1), // Monday, week 1 of 2025
            ((2026, 12, 31), 53, 4),
            ((2027, 1, 3), 53, 7),  // Sunday, still week 53 of 2026
            ((2023, 1, 1), 52, 7),
            ((2020, 12, 31), 53, 4),
        ] {
            assert_eq!(iso_week(t.0, t.1, t.2), w, "{t:?}");
            assert_eq!(weekday(t.0, t.1, t.2), u, "{t:?}");
        }
        assert_eq!(e("%V-%u", (2026, 10, 10, 0, 0, 0)), "41-6");
    }

    #[test]
    fn auto_save_paths() {
        let base = std::env::temp_dir().join("rustshot_auto");
        let cfg = Config {
            save_path: base.to_string_lossy().into(),
            ..Config::default()
        };
        assert_eq!(
            auto_save_path(&cfg, NOW),
            base.join("2026-03-07").join("2026-03-07_09-05.png")
        );
        let flat = Config { save_subfolder: false, save_format: "jpg".into(), ..cfg.clone() };
        assert_eq!(auto_save_path(&flat, NOW), base.join("2026-03-07_09-05.jpg"));
        let custom = Config {
            subfolder_pattern: "%Y/%m/../x:y".into(),
            filename_pattern: "shot %T".into(),
            save_format: "bmp".into(),
            ..cfg.clone()
        };
        assert_eq!(
            auto_save_path(&custom, NOW),
            base.join("2026").join("03").join("x-y").join("shot 09-05-02.bmp")
        );
        // Recordings: same folder and stem, their own extension.
        assert_eq!(
            auto_save_path_ext(&cfg, NOW, "mp4"),
            base.join("2026-03-07").join("2026-03-07_09-05.mp4")
        );
        assert_eq!(auto_save_path_ext(&flat, NOW, "gif"), base.join("2026-03-07_09-05.gif"));
        let empty = Config { filename_pattern: "".into(), subfolder_pattern: "..".into(), ..cfg };
        assert_eq!(auto_save_path(&empty, NOW), base.join("capture.png"));
    }

    #[test]
    fn default_dir_is_pictures_rustshot() {
        let d = default_save_dir(&Config::default());
        assert_eq!(d.file_name().unwrap(), "rustshot", "{}", d.display());
        if let Some(pics) = crate::desktop::pictures().filter(|p| p.is_dir()) {
            assert_eq!(d, pics.join("rustshot"));
        }
        let set = Config { save_path: "X:/shots".into(), ..Config::default() };
        assert_eq!(default_save_dir(&set), PathBuf::from("X:/shots"));
    }

    /// The Pictures folder as found (a OneDrive one, a localised XDG
    /// one), else the home, else the current directory; a set folder wins
    /// without a lookup.
    #[test]
    fn the_save_dir_takes_the_pictures_folder_found() {
        let dirs = ["/home/me", "/home/me/Изображения", r"C:\Users\me\OneDrive\Pictures"];
        let is_dir = |p: &Path| dirs.iter().any(|d| Path::new(d) == p);
        let cwd = || PathBuf::from("/cwd");
        let home = || Some(PathBuf::from("/home/me"));
        let d = Config::default();
        let od = PathBuf::from(r"C:\Users\me\OneDrive\Pictures");
        assert_eq!(save_dir_with(&d, || Some(od.clone()), home(), &is_dir, &cwd), od.join("rustshot"));
        let xdg = PathBuf::from("/home/me/Изображения");
        assert_eq!(save_dir_with(&d, || Some(xdg.clone()), home(), &is_dir, &cwd), xdg.join("rustshot"));
        let gone = || Some(PathBuf::from("/home/me/Pictures")); // not made yet
        assert_eq!(save_dir_with(&d, gone, home(), &is_dir, &cwd), PathBuf::from("/home/me/rustshot"));
        assert_eq!(save_dir_with(&d, || None, None, &is_dir, &cwd), PathBuf::from("/cwd"));
        let set = Config { save_path: "D:/shots".into(), ..Config::default() };
        assert_eq!(save_dir_with(&set, || panic!("not looked up"), home(), &is_dir, &cwd), PathBuf::from("D:/shots"));
    }

    #[test]
    fn sanitizes_components() {
        assert_eq!(sanitize_component("a<b>c:d\"e|f?g*h"), "a-b-c-d-e-f-g-h");
        assert_eq!(sanitize_component("x\ty\u{1}z"), "x-y-z");
        assert_eq!(sanitize_component("a/b\\c"), "a-b-c");
        assert_eq!(sanitize_component("a/../b"), "a-b");
        assert_eq!(sanitize_component("../.."), "");
        assert_eq!(sanitize_component("."), "");
        assert_eq!(sanitize_component("name. . "), "name");
        assert_eq!(sanitize_component("v1.2 final"), "v1.2 final");
    }

    /// Windows cannot make a file or folder named after a device.
    #[test]
    fn windows_device_names_get_an_underscore() {
        for (from, to) in [
            ("CON", "CON_"),
            ("nul", "nul_"),
            ("Aux.png", "Aux_.png"),
            ("com1", "com1_"),
            ("LPT9.tar.gz", "LPT9_.tar.gz"),
            ("COM\u{b9}", "COM\u{b9}_"),
            ("conout$", "conout$_"),
            ("prn.", "prn_"),
            ("a/con/b", "a-con_-b"),
        ] {
            assert_eq!(sanitize_component(from), to, "{from}");
        }
        for keep in ["CONSOLE", "com10", "COM0", "lpt", "nul-2026", "my con", "com1x"] {
            assert_eq!(sanitize_component(keep), keep);
        }
        let cfg = Config { save_path: "/s".into(), filename_pattern: "nul".into(), save_subfolder: false, ..Config::default() };
        assert_eq!(auto_save_path(&cfg, NOW).file_name().unwrap(), "nul_.png");
    }

    #[test]
    fn formats_from_ext() {
        assert_eq!(Format::from_ext("png"), Some(Format::Png));
        assert_eq!(Format::from_ext("PNG"), Some(Format::Png));
        assert_eq!(Format::from_ext("jpg"), Some(Format::Jpeg));
        assert_eq!(Format::from_ext("jpeg"), Some(Format::Jpeg));
        assert_eq!(Format::from_ext(".bmp"), Some(Format::Bmp));
        assert_eq!(Format::from_ext("gif"), None);
        assert_eq!(Format::from_ext(""), None);
        assert_eq!(Format::Jpeg.ext(), "jpg");
        let cfg = Config { save_format: "jpeg".into(), ..Config::default() };
        assert_eq!(Format::from_config(&cfg), Format::Jpeg);
        assert_eq!(Format::from_config(&Config::default()), Format::Png);
    }

    #[test]
    fn bmp_layout() {
        let mut img = PixBuf::new(2, 2);
        img.as_raw_mut().copy_from_slice(&[
            1, 2, 3, 255, 4, 5, 6, 255, //
            7, 8, 9, 255, 10, 11, 12, 255,
        ]);
        let b = encode(&img, Format::Bmp, 90).unwrap();
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let i32_at = |i: usize| i32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        assert_eq!(&b[0..2], b"BM");
        assert_eq!(u32_at(2), 66 + 16);
        assert_eq!(b.len(), 66 + 16);
        assert_eq!(u32_at(10), 66, "pixel offset");
        assert_eq!(u32_at(14), 40, "info header size");
        assert_eq!(i32_at(18), 2);
        assert_eq!(i32_at(22), -2, "negative height = top-down");
        assert_eq!(u16::from_le_bytes([b[26], b[27]]), 1);
        assert_eq!(u16::from_le_bytes([b[28], b[29]]), 32);
        assert_eq!(u32_at(30), 3, "BI_BITFIELDS");
        assert_eq!(u32_at(34), 16);
        assert_eq!(
            [u32_at(54), u32_at(58), u32_at(62)],
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF]
        );
        assert_eq!(
            &b[66..],
            &[3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]
        );
    }

    /// Width/height from the first SOF0..SOF2 marker.
    fn jpeg_dims(b: &[u8]) -> (u16, u16) {
        let mut i = 2;
        while i + 9 < b.len() {
            assert_eq!(b[i], 0xFF, "marker at {i}");
            let m = b[i + 1];
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if (0xC0..=0xC2).contains(&m) {
                let h = u16::from_be_bytes([b[i + 5], b[i + 6]]);
                let w = u16::from_be_bytes([b[i + 7], b[i + 8]]);
                return (w, h);
            }
            i += 2 + len;
        }
        panic!("no SOF marker");
    }

    #[test]
    fn jpeg_output() {
        // Noisy gradient so quality matters.
        let (w, h) = (37u32, 21u32);
        let mut img = PixBuf::new(w, h);
        for (i, px) in img.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v = (i as u32).wrapping_mul(2_654_435_761) >> 24;
            *px = [v as u8, (i * 7) as u8, (i % w as usize * 6) as u8, 255];
        }
        let hi = encode(&img, Format::Jpeg, 95).unwrap();
        let lo = encode(&img, Format::Jpeg, 30).unwrap();
        for b in [&hi, &lo] {
            assert_eq!(&b[..2], &[0xFF, 0xD8]);
            assert_eq!(&b[b.len() - 2..], &[0xFF, 0xD9]);
            assert_eq!(jpeg_dims(b), (w as u16, h as u16));
        }
        assert!(lo.len() < hi.len(), "q30 {} >= q95 {}", lo.len(), hi.len());
        assert!(encode(&PixBuf::new(70_000, 1), Format::Jpeg, 90).is_err());
    }

    #[test]
    fn saves_by_extension() {
        let dir = std::env::temp_dir().join("rustshot_test_formats");
        let _ = std::fs::remove_dir_all(&dir);
        let img = PixBuf::from_pixel(3, 2, [10, 20, 30, 255]);
        for (name, magic) in [("a.png", &[0x89, b'P'][..]), ("b.JPG", &[0xFF, 0xD8]), ("c.bmp", b"BM")] {
            let p = dir.join("sub").join(name);
            save_image(&img, &p, 80).unwrap();
            assert!(std::fs::read(&p).unwrap().starts_with(magic), "{name}");
        }
        assert!(save_image(&img, &dir.join("d.gif"), 80).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explicit_paths() {
        let cfg = Config { save_format: "bmp".into(), ..Config::default() };
        let f = PathBuf::from("out/x.jpg");
        assert_eq!(resolve_save_path(&Some(f.clone()), false, &cfg).unwrap(), f);
        let d = resolve_save_path(&Some(PathBuf::from("no_such_dir_rs")), false, &cfg).unwrap();
        assert_eq!(d.parent().unwrap(), Path::new("no_such_dir_rs"));
        assert_eq!(d.extension().unwrap(), "bmp");
    }

    #[test]
    fn unique_paths() {
        let dir = std::env::temp_dir().join("rustshot_test_unique");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("x.png");
        let _ = std::fs::remove_file(&f);
        assert_eq!(unique_path(&f), f);
        std::fs::write(&f, b"hi").unwrap();
        let u = unique_path(&f);
        assert_ne!(u, f);
        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A save over an existing file replaces it whole under its own name
    /// (no temporary file left); a save that fails leaves it as it was.
    #[test]
    fn saves_replace_files_atomically() {
        let dir = std::env::temp_dir().join(format!("rustshot_test_atomic_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("shot.png");
        std::fs::write(&f, b"old bytes").unwrap();
        let img = PixBuf::from_pixel(3, 2, [9, 8, 7, 255]);
        save_image(&img, &f, 80).unwrap();
        assert_eq!(PixBuf::from_png(&std::fs::read(&f).unwrap()).unwrap().dimensions(), (3, 2));
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, [std::ffi::OsString::from("shot.png")]);
        // Writing into a folder that is not there fails; nothing changes.
        let gone = dir.join("missing").join("x.png");
        assert!(write_atomic(&gone, b"x").is_err());
        // The target is a folder: the rename fails, the temporary file goes.
        let sub = dir.join("sub.png");
        std::fs::create_dir_all(&sub).unwrap();
        assert!(write_atomic(&sub, b"x").is_err());
        assert!(sub.is_dir());
        let left = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(TMP_SUFFIX)).count();
        assert_eq!(left, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows (NTFS): a reader holding the file open the way std opens
    /// it (FILE_SHARE_DELETE) does not hold a save up at all: the POSIX
    /// rename replaces it at once, the reader goes on with the old bytes,
    /// a new open sees the new ones.
    #[cfg(windows)]
    #[test]
    fn a_save_replaces_a_file_held_open_at_once() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("rustshot_test_atomic_posix_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("meta.json");
        std::fs::write(&f, b"old").unwrap();
        let mut held = std::fs::File::open(&f).unwrap();
        let t0 = std::time::Instant::now();
        write_atomic(&f, b"new bytes").unwrap();
        assert!(t0.elapsed() < std::time::Duration::from_millis(500), "no waiting: {:?}", t0.elapsed());
        let mut old = String::new();
        held.read_to_string(&mut old).unwrap();
        assert_eq!(old, "old");
        assert_eq!(std::fs::read(&f).unwrap(), b"new bytes");
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A reader that has the file open (the History window listing,
    /// another process, a virus scanner) does not make a save fail: the
    /// replace waits for it (it failed with "replace ...: Access is denied"
    /// in `concurrent_updates_all_land`).
    #[test]
    fn a_save_waits_for_a_reader_of_the_old_file() {
        let dir = std::env::temp_dir().join(format!("rustshot_test_atomic_open_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("meta.json");
        std::fs::write(&f, b"old").unwrap();
        let (tx, rx) = crate::chan::channel();
        let g = f.clone();
        let reader = std::thread::spawn(move || {
            let open = std::fs::File::open(&g).unwrap();
            tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(150));
            drop(open);
        });
        rx.recv().unwrap();
        write_atomic(&f, b"new").expect("replaced once the reader is done");
        reader.join().unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "live network upload to imgur"]
    fn live_imgur_upload() {
        let img = PixBuf::from_pixel(1, 1, [1, 2, 3, 255]);
        let png = png_bytes(&img).unwrap();
        match do_upload(&png, "313baf0c7b4d3ff") {
            Ok(link) => assert!(link.starts_with("https://i.imgur.com/"), "{link}"),
            // Transport verified either way; 429 means imgur rate-limits the
            // shared anonymous client-id, not that the request failed.
            Err(e) if e.contains("\"code\":\"429\"") => {}
            Err(e) => panic!("upload failed: {e}"),
        }
    }
}

#[cfg(test)]
mod border_tests {
    use super::*;

    fn cfg(w: u32, c: &str) -> Config {
        Config { border_width: w, border_color: c.into(), ..Config::default() }
    }

    #[test]
    fn the_border_surrounds_the_image_in_physical_px() {
        let img = PixBuf::from_pixel(3, 2, [10, 20, 30, 255]);
        // Off: the image as it is.
        assert_eq!(with_border(img.clone(), &Config::default(), 1.0), img);
        let out = with_border(img.clone(), &cfg(2, "#ff8000"), 1.0);
        assert_eq!(out.dimensions(), (7, 6));
        for y in 0..6 {
            for x in 0..7 {
                let inside = (2..5).contains(&x) && (2..4).contains(&y);
                let want = if inside { [10, 20, 30, 255] } else { [255, 128, 0, 255] };
                assert_eq!(out.get_pixel(x, y), want, "({x}, {y})");
            }
        }
        // Logical px: 3 at 150 % is 5 (rounded) on each side; 1 at 50 % is 1.
        assert_eq!(with_border(img.clone(), &cfg(3, "#000"), 1.5).dimensions(), (13, 12));
        assert_eq!(border(&cfg(1, "#000"), 0.5), Some((1, [0, 0, 0, 255])));
        assert_eq!(border(&cfg(8, "#123456"), 2.0), Some((16, [0x12, 0x34, 0x56, 255])));
        // A width past the limit is the limit; a bad color is black.
        assert_eq!(border(&cfg(20, "nope"), 1.0), Some((8, [0, 0, 0, 255])));
    }

    /// What is saved and what is encoded carry the border.
    #[test]
    fn saved_files_carry_the_border() {
        let dir = std::env::temp_dir().join(format!("rustshot-border-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.png");
        let img = with_border(PixBuf::from_pixel(4, 4, [255, 255, 255, 255]), &cfg(1, "#00ff00"), 1.0);
        let c = Config { save_path: dir.display().to_string(), ..cfg(1, "#00ff00") };
        let res = run_export(&img, (0, 0), &[Task::Save { path: Some(path.clone()), ask: false }], &c);
        assert!(!res.error, "{:?}", res.messages);
        let back = PixBuf::from_png(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(back.dimensions(), (6, 6));
        assert_eq!(back.get_pixel(0, 0), [0, 255, 0, 255]);
        assert_eq!(back.get_pixel(1, 1), [255, 255, 255, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Settings › Saving "Save at": the screen's pixels (as captured), 1×
    /// (a 200 % capture halved, area-averaged) or a percent; the border
    /// keeps its logical width at the new size; a Resize wins.
    #[test]
    fn save_at_scales_what_goes_out() {
        let at = |mode: &str, pct: u32| Config { save_scale: mode.into(), save_scale_percent: pct, ..Config::default() };
        // A 2x checkerboard of 2x2 blocks: halved it is flat per block.
        let mut img = PixBuf::new(40, 20);
        for (i, p) in img.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % 40, i / 40);
            *p = if (x / 2 + y / 2) % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
        }
        assert_eq!(for_output(img.clone(), &at("screen", 50), 2.0, false), img);
        let one = for_output(img.clone(), &at("logical", 50), 2.0, false);
        assert_eq!(one.dimensions(), (20, 10));
        assert_eq!(one.get_pixel(0, 0), [255, 0, 0, 255], "area average: a 2x2 block is one pixel");
        assert_eq!(one.get_pixel(1, 0), [0, 0, 255, 255]);
        assert_eq!(for_output(img.clone(), &at("logical", 50), 1.0, false), img, "a 100 % capture is 1× already");
        assert_eq!(for_output(img.clone(), &at("logical", 50), 1.5, false).dimensions(), (27, 13));
        assert_eq!(for_output(img.clone(), &at("custom", 25), 1.0, false).dimensions(), (10, 5));
        assert_eq!(for_output(img.clone(), &at("custom", 100), 2.0, false), img);
        assert_eq!(for_output(img.clone(), &at("custom", 25), 1.0, true), img, "resized on purpose: kept");
        // The border: 2 logical px; at 200 % saved at 1×: 2 px.
        let b = Config { border_width: 2, ..at("logical", 50) };
        assert_eq!(for_output(img.clone(), &b, 2.0, false).dimensions(), (24, 14));
        assert_eq!(output_border(&b, 2.0), 2);
        assert_eq!(output_border(&Config { border_width: 2, ..Config::default() }, 2.0), 4, "screen pixels: 4");
        assert_eq!((save_factor(&at("logical", 50), 2.0), save_factor(&at("custom", 5), 1.0)), (0.5, 0.1));
    }
}
