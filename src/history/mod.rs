// The History and Edit windows (next) read most of this.
#![allow(dead_code)]
//! Capture history: every capture is kept (unless `history = false`) in
//! `%LOCALAPPDATA%\rustshot\history\<yyyy-mm-dd>\<hhmmss>\` (Linux:
//! `$XDG_DATA_HOME/rustshot/history`, macOS: `~/Library/Application
//! Support/rustshot/history`) as
//!
//! * `original.png`: the selection as captured, without annotations
//!   (never changed afterwards, so a capture can always be edited again);
//! * `annotations.json`: its objects ([`crate::objects::to_json`]);
//! * `thumb.png`: the annotated picture fitted into 200 × 140;
//! * `meta.json`: what is known about it ([`Meta`]): when, its size, the
//!   saved file and its size and format, the monitor, the window it came
//!   from, the uploaded link, the kind (screenshot, MP4, GIF), a
//!   recording's length and file, whether it was edited and the kinds of
//!   its objects. Written last: a folder without it is not an entry.
//!
//! Recordings keep only `thumb.png` (the area when it started) and
//! `meta.json`; the video stays where it was saved. Entries older than
//! `history_days` go, then the oldest until the history fits in
//! `history_max_mb` ([`Store::prune`]). Saved and uploaded files are never
//! touched by that (only "Clear history…" with "Also delete saved files").
//!
//! Writing (PNG encoding a 4K capture takes a while) runs on one worker
//! thread ([`record`], [`update`], [`flush`]) in order; the folder is
//! reserved at once, so its id is known before the files are there.

pub mod source;

use crate::export::Tm;
use crate::objects::Obj;
use crate::objects::json::{self, V};
use crate::pixbuf::PixBuf;
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender};

pub const ORIGINAL: &str = "original.png";
pub const ANNOTATIONS: &str = "annotations.json";
pub const THUMB: &str = "thumb.png";
pub const META: &str = "meta.json";
/// The thumbnail's box (logical px of the History grid's cards).
pub const THUMB_W: u32 = 200;
pub const THUMB_H: u32 = 140;
/// `meta.json`'s format version.
const VERSION: u32 = 1;

/// What a capture is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Kind {
    #[default]
    Shot,
    Mp4,
    Gif,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Shot => "screenshot",
            Kind::Mp4 => "mp4",
            Kind::Gif => "gif",
        }
    }

    pub fn from_id(s: &str) -> Option<Kind> {
        [Kind::Shot, Kind::Mp4, Kind::Gif].into_iter().find(|k| k.id() == s)
    }

    pub fn is_recording(self) -> bool {
        self != Kind::Shot
    }

    /// The badge / type column: "PNG" (the format), "MP4", "GIF".
    pub fn label(self) -> &'static str {
        match self {
            Kind::Shot => "PNG",
            Kind::Mp4 => "MP4",
            Kind::Gif => "GIF",
        }
    }
}

/// What is known about a capture (`meta.json`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Meta {
    /// Local wall-clock time it was taken.
    pub captured: Tm,
    pub kind: Kind,
    /// The picture's size, physical px.
    pub w: u32,
    pub h: u32,
    /// The saved file's size (else `original.png`'s; a recording's file).
    pub bytes: u64,
    /// "PNG", "JPG", "BMP", "MP4", "GIF".
    pub format: String,
    /// The monitor it was taken on ("DELL U2720Q") and its scale.
    pub monitor: String,
    pub scale: f32,
    /// The window it came from: its title and app ("Chrome").
    pub title: String,
    pub app: String,
    /// Where it was saved, the link it was uploaded to.
    pub saved: Option<PathBuf>,
    pub uploaded: Option<String>,
    /// A recording's length.
    pub duration_ms: u64,
    /// A recording's file.
    pub file: Option<PathBuf>,
    /// Edited in the Edit window since.
    pub edited: bool,
    /// Its objects' kinds, bottom-most first ("Arrow", "Number").
    pub objects: Vec<String>,
}

/// One capture in the store.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// `<yyyy-mm-dd>/<name>`.
    pub id: String,
    pub dir: PathBuf,
    pub meta: Meta,
    /// Bytes the entry's folder takes.
    pub size: u64,
}

impl Entry {
    pub fn original(&self) -> PathBuf {
        self.dir.join(ORIGINAL)
    }
    pub fn thumb(&self) -> PathBuf {
        self.dir.join(THUMB)
    }
    pub fn annotations(&self) -> PathBuf {
        self.dir.join(ANNOTATIONS)
    }
    /// The date folder's date.
    pub fn day(&self) -> (i32, u32, u32) {
        (self.meta.captured.0, self.meta.captured.1, self.meta.captured.2)
    }
}

/// A capture's name in History: "Chrome — Stripe Dashboard" (the app
/// and its window), else the window title, the saved file's name, or
/// "Screenshot" / "Recording".
pub fn title(m: &Meta) -> String {
    let t = clean_title(&m.title, &m.app);
    match (m.app.trim(), t.trim()) {
        ("", "") => m
            .saved
            .as_ref()
            .or(m.file.as_ref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| if m.kind.is_recording() { "Recording".into() } else { "Screenshot".into() }),
        (a, "") => a.to_string(),
        ("", t) => t.to_string(),
        (a, t) => format!("{a} \u{2014} {t}"),
    }
}

/// A window title without the app's own suffix ("Stripe - Google Chrome"
/// from Chrome is "Stripe").
fn clean_title(title: &str, app: &str) -> String {
    let t = title.trim();
    let a = app.trim().to_lowercase();
    if a.is_empty() {
        return t.to_string();
    }
    for sep in [" \u{2014} ", " \u{2013} ", " - ", " | "] {
        if let Some(i) = t.rfind(sep) {
            let tail = t[i + sep.len()..].to_lowercase();
            if tail.contains(&a) || a.contains(&tail) {
                return t[..i].trim().to_string();
            }
        }
    }
    if t.to_lowercase() == a { String::new() } else { t.to_string() }
}

/// The chips under "Annotations · N objects": each kind once, in order,
/// with its count ("Number ×2").
pub fn object_chips(kinds: &[String]) -> Vec<String> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for k in kinds {
        match out.iter_mut().find(|(n, _)| n == k) {
            Some((_, c)) => *c += 1,
            None => out.push((k.clone(), 1)),
        }
    }
    out.into_iter().map(|(n, c)| if c > 1 { format!("{n} \u{d7}{c}") } else { n }).collect()
}

/// An object's kind as History names it.
pub fn kind_name(o: &Obj) -> &'static str {
    match o {
        Obj::Path { .. } => "Pencil",
        Obj::Line { .. } => "Line",
        Obj::Arrow { .. } => "Arrow",
        Obj::Rect { .. } | Obj::RectFill { .. } => "Rectangle",
        Obj::Ellipse { .. } | Obj::EllipseFill { .. } => "Ellipse",
        Obj::Marker { .. } | Obj::MarkerPath { .. } => "Marker",
        Obj::Text { .. } => "Text",
        Obj::Pixelate { .. } => "Pixelate",
        Obj::Invert { .. } => "Invert",
        Obj::Counter { .. } => "Number",
        Obj::Callout { .. } => "Callout",
        Obj::Spotlight { .. } => "Spotlight",
        Obj::Magnify { .. } => "Magnify",
    }
}

// --- dates ---------------------------------------------------------------------

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date `days` after 1970-01-01.
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
    (y, m, d)
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "Sun 11 Oct" (with the year when it is not `this_year`).
pub fn short_date(y: i32, m: u32, d: u32, this_year: i32) -> String {
    let wd = (days_from_civil(y, m, d) + 3).rem_euclid(7) as usize; // 1970-01-01 was a Thursday
    let mon = MONTHS[(m.clamp(1, 12) - 1) as usize];
    if y == this_year { format!("{} {d} {mon}", WEEKDAYS[wd]) } else { format!("{} {d} {mon} {y}", WEEKDAYS[wd]) }
}

/// A day group's header: the heading ("Today", "Yesterday", "Fri 9 Oct")
/// and the smaller part after it ("· 4 captures" today, "· Sun 11 Oct"
/// yesterday).
pub fn day_header(day: (i32, u32, u32), today: (i32, u32, u32), count: usize) -> (String, Option<String>) {
    let age = days_from_civil(today.0, today.1, today.2) - days_from_civil(day.0, day.1, day.2);
    match age {
        0 => ("Today".into(), Some(format!("\u{b7} {count} capture{}", if count == 1 { "" } else { "s" }))),
        1 => ("Yesterday".into(), Some(format!("\u{b7} {}", short_date(day.0, day.1, day.2, today.0)))),
        _ => (short_date(day.0, day.1, day.2, today.0), None),
    }
}

/// "3 results for "stripe"".
pub fn results_header(n: usize, q: &str) -> String {
    format!("{n} result{} for \u{201c}{}\u{201d}", if n == 1 { "" } else { "s" }, q.trim())
}

/// "14:32", or with the day when the group does not say it ("Today 11:48",
/// "Thu 8 Oct 16:40").
pub fn time_text(t: &Tm, today: (i32, u32, u32), with_day: bool) -> String {
    let hm = format!("{:02}:{:02}", t.3, t.4);
    if !with_day {
        return hm;
    }
    let age = days_from_civil(today.0, today.1, today.2) - days_from_civil(t.0, t.1, t.2);
    match age {
        0 => format!("Today {hm}"),
        1 => format!("Yesterday {hm}"),
        _ => format!("{} {hm}", short_date(t.0, t.1, t.2, today.0)),
    }
}

/// The detail panel's "Captured": "Today, 09:14:07", "Yesterday, ...",
/// "Fri 9 Oct, 16:02:11".
pub fn captured_text(t: &Tm, today: (i32, u32, u32)) -> String {
    let age = days_from_civil(today.0, today.1, today.2) - days_from_civil(t.0, t.1, t.2);
    let day = match age {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        _ => short_date(t.0, t.1, t.2, today.0),
    };
    format!("{day}, {:02}:{:02}:{:02}", t.3, t.4, t.5)
}

/// "486 KB", "1.2 GB", "0 B".
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes == 0 {
        "0 B".into()
    } else if b < 1e6 {
        format!("{} KB", (b / 1e3).round().max(1.0) as u64)
    } else if b < 1e9 {
        let mb = b / 1e6;
        if mb < 10.0 { format!("{mb:.1} MB") } else { format!("{mb:.0} MB") }
    } else {
        format!("{:.1} GB", b / 1e9)
    }
}

/// A recording's length: "00:42", "1:02:03".
pub fn duration_text(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

// --- search --------------------------------------------------------------------

/// Where `q`'s words match `e` (each word somewhere in the title, the app,
/// the saved file's name or the date, case-insensitive): `Some` with the
/// byte ranges of the matches in [`title`] (to highlight), `None` when a
/// word matches nothing. An empty query matches everything.
pub fn search(e: &Meta, q: &str, today: (i32, u32, u32)) -> Option<Vec<std::ops::Range<usize>>> {
    let shown = title(e);
    let low = lower_map(&shown);
    let t = e.captured;
    let file = e.saved.as_ref().or(e.file.as_ref()).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let dates = [
        format!("{:04}-{:02}-{:02}", t.0, t.1, t.2),
        short_date(t.0, t.1, t.2, today.0).to_lowercase(),
        time_text(&t, today, true).to_lowercase(),
        MONTHS[(t.1.clamp(1, 12) - 1) as usize].to_lowercase(),
        e.kind.label().to_lowercase(),
    ];
    let hay = [e.title.to_lowercase(), e.app.to_lowercase(), file].join("\n");
    let mut hits = Vec::new();
    for w in q.split_whitespace().map(str::to_lowercase) {
        let mut found = false;
        // Every occurrence in the shown title is highlighted.
        let mut from = 0;
        while let Some(i) = low.0[from..].find(&w) {
            let a = low.1[from + i];
            let b = low.1[from + i + w.len()];
            hits.push(a..b);
            found = true;
            from += i + w.len().max(1);
        }
        found |= hay.contains(&w) || dates.iter().any(|d| d.contains(&w));
        if !found {
            return None;
        }
    }
    hits.sort_by_key(|r| r.start);
    // Merge overlaps.
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for r in hits {
        match out.last_mut() {
            Some(l) if r.start <= l.end => l.end = l.end.max(r.end),
            _ => out.push(r),
        }
    }
    Some(out)
}

/// `s` lowercased with, for each byte of the result (and its end), the
/// byte offset in `s` it came from.
fn lower_map(s: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len() + 1);
    for (i, c) in s.char_indices() {
        for l in c.to_lowercase() {
            let n = l.len_utf8();
            out.push(l);
            map.extend(std::iter::repeat_n(i, n));
        }
    }
    map.push(s.len());
    (out, map)
}

/// Captures, Screenshots or Recordings (the header's filter).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Filter {
    #[default]
    All,
    Shots,
    Recordings,
}

impl Filter {
    pub fn keeps(self, k: Kind) -> bool {
        match self {
            Filter::All => true,
            Filter::Shots => !k.is_recording(),
            Filter::Recordings => k.is_recording(),
        }
    }
}

// --- meta.json ---------------------------------------------------------------

fn tm_text(t: &Tm) -> String {
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", t.0, t.1, t.2, t.3, t.4, t.5)
}

fn parse_tm(s: &str) -> Option<Tm> {
    let (d, t) = s.split_once('T')?;
    let mut dp = d.split('-');
    let y = dp.next()?.parse().ok()?;
    let mo = dp.next()?.parse().ok().filter(|m| (1..=12).contains(m))?;
    let da = dp.next()?.parse().ok().filter(|d| (1..=31).contains(d))?;
    let mut tp = t.split(':');
    let h = tp.next()?.parse().ok().filter(|h| *h < 24)?;
    let mi = tp.next()?.parse().ok().filter(|m| *m < 60)?;
    let se = tp.next()?.parse().ok().filter(|s| *s < 61)?;
    Some((y, mo, da, h, mi, se))
}

/// `m` as `meta.json`.
pub fn meta_to_json(m: &Meta) -> String {
    let mut o = String::new();
    o.push_str(&format!("{{\"version\":{VERSION}"));
    let mut s = |k: &str, v: &str| {
        o.push(',');
        json::string(&mut o, k);
        o.push(':');
        json::string(&mut o, v);
    };
    s("kind", m.kind.id());
    s("captured", &tm_text(&m.captured));
    s("format", &m.format);
    s("monitor", &m.monitor);
    s("title", &m.title);
    s("app", &m.app);
    if let Some(p) = &m.saved {
        s("saved", &p.to_string_lossy());
    }
    if let Some(u) = &m.uploaded {
        s("uploaded", u);
    }
    if let Some(p) = &m.file {
        s("file", &p.to_string_lossy());
    }
    let scale = if m.scale.is_finite() { m.scale } else { 1.0 };
    o.push_str(&format!(
        ",\"w\":{},\"h\":{},\"bytes\":{},\"scale\":{},\"duration_ms\":{},\"edited\":{}",
        m.w, m.h, m.bytes, scale, m.duration_ms, m.edited
    ));
    o.push_str(",\"objects\":[");
    for (i, k) in m.objects.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        json::string(&mut o, k);
    }
    o.push_str("]}\n");
    o
}

/// A `meta.json` document. Unknown keys are ignored; a newer version, a
/// bad date or kind is an error.
pub fn meta_from_json(s: &str) -> Result<Meta> {
    let V::Obj(top) = json::parse(s).map_err(|e| anyhow!("meta.json: {}", e.to_string().trim_start_matches("annotations.json: ")))? else {
        bail!("meta.json: not an object")
    };
    let get = |k: &str| top.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    match get("version") {
        Some(V::Num(v)) if *v >= 1.0 && *v <= VERSION as f64 => {}
        _ => bail!("meta.json: unsupported version"),
    }
    let st = |k: &str| match get(k) {
        Some(V::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let num = |k: &str| match get(k) {
        Some(V::Num(n)) if n.is_finite() && *n >= 0.0 => *n,
        _ => 0.0,
    };
    let opt = |k: &str| match get(k) {
        Some(V::Str(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let kind = Kind::from_id(&st("kind")).ok_or_else(|| anyhow!("meta.json: unknown kind"))?;
    let captured = parse_tm(&st("captured")).ok_or_else(|| anyhow!("meta.json: bad date"))?;
    let objects = match get("objects") {
        Some(V::Arr(a)) => a.iter().filter_map(|v| if let V::Str(s) = v { Some(s.clone()) } else { None }).collect(),
        _ => Vec::new(),
    };
    Ok(Meta {
        captured,
        kind,
        w: num("w").min(u32::MAX as f64) as u32,
        h: num("h").min(u32::MAX as f64) as u32,
        bytes: num("bytes") as u64,
        format: st("format"),
        monitor: st("monitor"),
        scale: (num("scale") as f32).max(0.0),
        title: st("title"),
        app: st("app"),
        saved: opt("saved").map(PathBuf::from),
        uploaded: opt("uploaded"),
        duration_ms: num("duration_ms") as u64,
        file: opt("file").map(PathBuf::from),
        edited: matches!(get("edited"), Some(V::Bool(true))),
        objects,
    })
}

// --- the store -----------------------------------------------------------------

/// The default history folder.
pub fn default_root() -> PathBuf {
    let get = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute());
    #[cfg(windows)]
    let base = get("LOCALAPPDATA");
    #[cfg(target_os = "macos")]
    let base = get("HOME").map(|h| h.join("Library").join("Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = get("XDG_DATA_HOME").or_else(|| get("HOME").map(|h| h.join(".local").join("share")));
    match base {
        Some(b) => b.join("rustshot").join("history"),
        None => crate::config::config_dir().join("history"),
    }
}

/// The history folder.
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().map(|e| e.metadata().map_or(0, |m| if m.is_dir() { dir_size(&e.path()) } else { m.len() })).sum())
        .unwrap_or(0)
}

/// A day folder's name ("2026-10-12") as a date.
fn parse_day(name: &str) -> Option<(i32, u32, u32)> {
    let mut p = name.split('-');
    let y = p.next()?.parse().ok()?;
    let m = p.next()?.parse().ok().filter(|m| (1..=12).contains(m))?;
    let d = p.next()?.parse().ok().filter(|d| (1..=31).contains(d))?;
    p.next().is_none().then_some((y, m, d))
}

/// Write `bytes` to `path` through a temporary file (a reader never sees
/// half a file).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename to {}", path.display()))?;
    Ok(())
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    pub fn open_default() -> Store {
        Store::new(default_root())
    }

    fn entry_dir(&self, id: &str) -> Option<PathBuf> {
        let (day, name) = id.split_once('/')?;
        parse_day(day)?;
        let ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        ok.then(|| self.root.join(day).join(name))
    }

    /// A new entry's folder for a capture taken at `now`:
    /// `<yyyy-mm-dd>/<hhmmss>` (`-2`, `-3`... when taken). Returns its id.
    pub fn reserve(&self, now: Tm) -> Result<(String, PathBuf)> {
        let day = format!("{:04}-{:02}-{:02}", now.0, now.1, now.2);
        let base = format!("{:02}{:02}{:02}", now.3, now.4, now.5);
        let dd = self.root.join(&day);
        std::fs::create_dir_all(&dd).with_context(|| format!("create {}", dd.display()))?;
        for n in 1..1000 {
            let name = if n == 1 { base.clone() } else { format!("{base}-{n}") };
            let dir = dd.join(&name);
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok((format!("{day}/{name}"), dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).with_context(|| format!("create {}", dir.display())),
            }
        }
        bail!("no free history folder for {day} {base}")
    }

    /// Write a screenshot into its reserved folder: the original, its
    /// objects, the thumbnail (`thumb`, already fitted, or made from
    /// them), then `meta.json`.
    pub fn write_shot(&self, dir: &Path, original: &PixBuf, objects: &[Obj], thumb: Option<&PixBuf>, meta: &Meta) -> Result<Meta> {
        let png = original.to_png().context("encode original.png")?;
        write_atomic(&dir.join(ORIGINAL), &png)?;
        write_atomic(&dir.join(ANNOTATIONS), crate::objects::to_json(objects).as_bytes())?;
        let made;
        let thumb = match thumb {
            Some(t) => t,
            None => {
                made = thumbnail(original, objects);
                &made
            }
        };
        write_atomic(&dir.join(THUMB), &thumb.to_png()?)?;
        let mut m = meta.clone();
        m.w = original.width();
        m.h = original.height();
        if m.saved.is_none() || m.bytes == 0 {
            m.bytes = m.saved.as_ref().and_then(|p| std::fs::metadata(p).ok()).map_or(png.len() as u64, |md| md.len());
        }
        if m.format.is_empty() {
            m.format = m.saved.as_ref().and_then(|p| p.extension()).map_or("PNG".into(), |e| e.to_string_lossy().to_uppercase());
        }
        m.objects = objects.iter().map(|o| kind_name(o).to_string()).collect();
        write_atomic(&dir.join(META), meta_to_json(&m).as_bytes())?;
        Ok(m)
    }

    /// Write a recording into its reserved folder: its poster (the area
    /// when it started) as the thumbnail, then `meta.json`.
    pub fn write_recording(&self, dir: &Path, poster: Option<&PixBuf>, meta: &Meta) -> Result<()> {
        if let Some(p) = poster {
            write_atomic(&dir.join(THUMB), &crate::scale::fit(p, THUMB_W, THUMB_H).to_png()?)?;
        }
        let mut m = meta.clone();
        if let Some(f) = &m.file {
            m.bytes = std::fs::metadata(f).map_or(m.bytes, |md| md.len());
            if m.format.is_empty() {
                m.format = f.extension().map_or(String::new(), |e| e.to_string_lossy().to_uppercase());
            }
        }
        write_atomic(&dir.join(META), meta_to_json(&m).as_bytes())
    }

    /// The entry `id` (None: gone, or not written yet).
    pub fn get(&self, id: &str) -> Option<Entry> {
        let dir = self.entry_dir(id)?;
        let meta = meta_from_json(&std::fs::read_to_string(dir.join(META)).ok()?).ok()?;
        Some(Entry { id: id.to_string(), size: dir_size(&dir), dir, meta })
    }

    /// Every entry, newest first.
    pub fn list(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        let Ok(days) = std::fs::read_dir(&self.root) else { return out };
        for d in days.flatten() {
            let name = d.file_name().to_string_lossy().into_owned();
            if parse_day(&name).is_none() || !d.path().is_dir() {
                continue;
            }
            let Ok(items) = std::fs::read_dir(d.path()) else { continue };
            for it in items.flatten() {
                let id = format!("{name}/{}", it.file_name().to_string_lossy());
                if let Some(e) = self.get(&id) {
                    out.push(e);
                }
            }
        }
        out.sort_by(|a, b| b.meta.captured.cmp(&a.meta.captured).then_with(|| b.id.cmp(&a.id)));
        out
    }

    /// The bytes the history takes.
    pub fn usage(&self) -> u64 {
        dir_size(&self.root)
    }

    /// Change `id`'s `meta.json` (Edited, the saved file, the link).
    pub fn update(&self, id: &str, f: impl FnOnce(&mut Meta)) -> Result<Meta> {
        let dir = self.entry_dir(id).ok_or_else(|| anyhow!("bad history id {id:?}"))?;
        let mut m = meta_from_json(&std::fs::read_to_string(dir.join(META)).with_context(|| format!("read {id}"))?)?;
        f(&mut m);
        write_atomic(&dir.join(META), meta_to_json(&m).as_bytes())?;
        Ok(m)
    }

    /// The Edit window saved `id`: its objects, a new thumbnail (made
    /// from the original and them), "Edited", and the file it was saved
    /// to. `original.png` stays as it was.
    pub fn save_edit(&self, id: &str, objects: &[Obj], saved: Option<(PathBuf, u64)>) -> Result<Meta> {
        let dir = self.entry_dir(id).ok_or_else(|| anyhow!("bad history id {id:?}"))?;
        write_atomic(&dir.join(ANNOTATIONS), crate::objects::to_json(objects).as_bytes())?;
        if let Ok(orig) = PixBuf::from_png(&std::fs::read(dir.join(ORIGINAL)).unwrap_or_default()) {
            write_atomic(&dir.join(THUMB), &thumbnail(&orig, objects).to_png()?)?;
        }
        self.update(id, |m| {
            m.edited = true;
            m.objects = objects.iter().map(|o| kind_name(o).to_string()).collect();
            if let Some((p, bytes)) = saved {
                m.format = p.extension().map_or(m.format.clone(), |e| e.to_string_lossy().to_uppercase());
                m.saved = Some(p);
                m.bytes = bytes;
            }
        })
    }

    /// The original picture of `e`.
    pub fn original(&self, e: &Entry) -> Result<PixBuf> {
        PixBuf::from_png(&std::fs::read(e.original()).with_context(|| format!("read {}", e.original().display()))?)
    }

    /// The objects of `e` (none when it has no annotations.json).
    pub fn objects(&self, e: &Entry) -> Result<Vec<Obj>> {
        match std::fs::read_to_string(e.annotations()) {
            Ok(t) => crate::objects::from_json(&t),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }

    /// Remove entry `id`; with `files` also the file it was saved to (or
    /// the recording).
    pub fn delete(&self, id: &str, files: bool) -> Result<()> {
        let dir = self.entry_dir(id).ok_or_else(|| anyhow!("bad history id {id:?}"))?;
        if files && let Some(e) = self.get(id) {
            for f in e.meta.saved.iter().chain(e.meta.file.iter()) {
                let _ = std::fs::remove_file(f);
            }
        }
        std::fs::remove_dir_all(&dir).with_context(|| format!("remove {}", dir.display()))?;
        if let Some(day) = dir.parent() {
            let _ = std::fs::remove_dir(day); // only when it is empty now
        }
        Ok(())
    }

    /// Remove every entry ("Clear history…"); with `files` their saved
    /// files too. Returns how many went.
    pub fn clear(&self, files: bool) -> usize {
        let all = self.list();
        let n = all.iter().filter(|e| self.delete(&e.id, files).is_ok()).count();
        // Folders without meta.json (half written, from a crash) go too.
        if let Ok(days) = std::fs::read_dir(&self.root) {
            for d in days.flatten().filter(|d| parse_day(&d.file_name().to_string_lossy()).is_some()) {
                let _ = std::fs::remove_dir_all(d.path());
            }
        }
        n
    }

    /// Keep the history within `days` (entries from days before that go)
    /// and `max_bytes` (then the oldest go until it fits); returns the ids
    /// removed. `keep`: an entry being written or edited now stays.
    pub fn prune(&self, today: (i32, u32, u32), days: u32, max_bytes: u64, keep: &[String]) -> Vec<String> {
        let t = days_from_civil(today.0, today.1, today.2);
        let mut all = self.list();
        let mut gone = Vec::new();
        all.retain(|e| {
            let (y, m, d) = e.day();
            let old = t - days_from_civil(y, m, d) >= days as i64;
            if old && !keep.contains(&e.id) && self.delete(&e.id, false).is_ok() {
                gone.push(e.id.clone());
                return false;
            }
            true
        });
        let mut total: u64 = all.iter().map(|e| e.size).sum();
        // Oldest first.
        for e in all.iter().rev() {
            if total <= max_bytes {
                break;
            }
            if keep.contains(&e.id) {
                continue;
            }
            if self.delete(&e.id, false).is_ok() {
                total = total.saturating_sub(e.size);
                gone.push(e.id.clone());
            }
        }
        gone
    }
}

/// The thumbnail of a screenshot: the original with its objects, fitted
/// into 200 × 140.
pub fn thumbnail(original: &PixBuf, objects: &[Obj]) -> PixBuf {
    if objects.is_empty() {
        return crate::scale::fit(original, THUMB_W, THUMB_H);
    }
    let font = crate::text::AnnotFont::load();
    crate::scale::fit(&crate::editor::bake_objects(original, objects, font.as_ref()), THUMB_W, THUMB_H)
}

/// The saved files of `entries` that are still there: how many, their
/// bytes, and the folder most of them are in (for "Also delete saved
/// files (1.2 GB in ~/Pictures/rustshot)").
pub fn saved_files(entries: &[Entry]) -> (usize, u64, Option<PathBuf>) {
    let mut n = 0;
    let mut bytes = 0;
    let mut dirs: Vec<(PathBuf, usize)> = Vec::new();
    for f in entries.iter().flat_map(|e| e.meta.saved.iter().chain(e.meta.file.iter())) {
        let Ok(md) = std::fs::metadata(f) else { continue };
        n += 1;
        bytes += md.len();
        // Daily subfolders count as their parent.
        let mut d = f.parent().map(Path::to_path_buf).unwrap_or_default();
        if d.file_name().and_then(|n| parse_day(&n.to_string_lossy())).is_some() {
            d = d.parent().map(Path::to_path_buf).unwrap_or(d);
        }
        match dirs.iter_mut().find(|(p, _)| *p == d) {
            Some((_, c)) => *c += 1,
            None => dirs.push((d, 1)),
        }
    }
    dirs.sort_by_key(|d| std::cmp::Reverse(d.1));
    (n, bytes, dirs.into_iter().next().map(|d| d.0))
}

/// `p` with the home folder as `~` ("~/Pictures/rustshot").
pub fn tilde(p: &Path) -> String {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(unix)]
    let home = std::env::var_os("HOME");
    if let Some(h) = home.filter(|h| !h.is_empty())
        && let Ok(rest) = p.strip_prefix(PathBuf::from(h))
    {
        let r = rest.to_string_lossy().replace('\\', "/");
        return if r.is_empty() { "~".into() } else { format!("~/{r}") };
    }
    p.display().to_string()
}

// --- the writer ------------------------------------------------------------------

/// A screenshot for the history: the unannotated selection, its objects
/// (in its coordinates) and what is known about it.
pub struct ShotRec {
    pub original: PixBuf,
    pub objects: Vec<Obj>,
    pub meta: Meta,
}

enum Job {
    Shot { root: PathBuf, id: String, dir: PathBuf, shot: Box<ShotRec>, days: u32, max: u64 },
    Recording { root: PathBuf, dir: PathBuf, poster: Option<PixBuf>, meta: Box<Meta>, days: u32, max: u64 },
    Update { root: PathBuf, id: String, f: Box<dyn FnOnce(&mut Meta) + Send> },
    Flush(Sender<()>),
}

static WORKER: Mutex<Option<Sender<Job>>> = Mutex::new(None);
/// Entries being written: never pruned meanwhile.
static WRITING: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn send(job: Job) {
    let mut w = lock(&WORKER);
    let tx = w.get_or_insert_with(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("rustshot-history".into()).spawn(move || run(rx)).expect("history thread");
        tx
    });
    if let Err(std::sync::mpsc::SendError(job)) = tx.send(job) {
        // The thread is gone (it panicked): a new one.
        *w = None;
        drop(w);
        send(job);
    }
}

fn today(t: &Tm) -> (i32, u32, u32) {
    (t.0, t.1, t.2)
}

fn run(rx: Receiver<Job>) {
    for job in rx {
        match job {
            Job::Shot { root, id, dir, shot, days, max } => {
                let st = Store::new(root);
                if let Err(e) = st.write_shot(&dir, &shot.original, &shot.objects, None, &shot.meta) {
                    eprintln!("rustshot: history: {e:#}");
                    let _ = std::fs::remove_dir_all(&dir);
                }
                lock(&WRITING).retain(|w| *w != id);
                let keep = lock(&WRITING).clone();
                st.prune(today(&shot.meta.captured), days, max, &keep);
                changed();
            }
            Job::Recording { root, dir, poster, meta, days, max } => {
                let st = Store::new(root);
                if let Err(e) = st.write_recording(&dir, poster.as_ref(), &meta) {
                    eprintln!("rustshot: history: {e:#}");
                    let _ = std::fs::remove_dir_all(&dir);
                }
                let keep = lock(&WRITING).clone();
                st.prune(today(&meta.captured), days, max, &keep);
                changed();
            }
            Job::Update { root, id, f } => {
                if let Err(e) = Store::new(root).update(&id, f) {
                    eprintln!("rustshot: history: {e:#}");
                }
                changed();
            }
            Job::Flush(tx) => {
                let _ = tx.send(());
            }
        }
    }
}

/// Bumped whenever the history changed (an open History window lists
/// it again).
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn changed() {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

fn limits(cfg: &crate::config::Config) -> (u32, u64) {
    (cfg.history_days.max(1), cfg.history_max_mb.max(1) as u64 * 1_000_000)
}

/// Keep screenshot `shot` (when `cfg.history` is on): its folder now, the
/// files on the history thread. Returns the entry's id.
pub fn record(cfg: &crate::config::Config, root: &Path, shot: ShotRec) -> Option<String> {
    if !cfg.history {
        return None;
    }
    let st = Store::new(root);
    let (id, dir) = st.reserve(shot.meta.captured).map_err(|e| eprintln!("rustshot: history: {e:#}")).ok()?;
    lock(&WRITING).push(id.clone());
    let (days, max) = limits(cfg);
    send(Job::Shot { root: root.to_path_buf(), id: id.clone(), dir, shot: Box::new(shot), days, max });
    Some(id)
}

/// Keep a recording (its file in `meta.file`, `poster`: the area when it
/// started).
pub fn record_recording(cfg: &crate::config::Config, root: &Path, poster: Option<PixBuf>, meta: Meta) -> Option<String> {
    if !cfg.history {
        return None;
    }
    let st = Store::new(root);
    let (id, dir) = st.reserve(meta.captured).map_err(|e| eprintln!("rustshot: history: {e:#}")).ok()?;
    let (days, max) = limits(cfg);
    send(Job::Recording { root: root.to_path_buf(), dir, poster, meta: Box::new(meta), days, max });
    Some(id)
}

/// Change `id`'s meta on the history thread (after its files are written).
pub fn update(root: &Path, id: &str, f: impl FnOnce(&mut Meta) + Send + 'static) {
    send(Job::Update { root: root.to_path_buf(), id: id.to_string(), f: Box::new(f) });
}

/// Wait (up to `limit`) until everything sent so far is written.
pub fn flush(limit: std::time::Duration) {
    if lock(&WORKER).is_none() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    send(Job::Flush(tx));
    let _ = rx.recv_timeout(limit);
}

/// How many captures were kept today (the tray's "History… 4 today").
pub fn today_count(root: &Path, today: (i32, u32, u32)) -> usize {
    let day = root.join(format!("{:04}-{:02}-{:02}", today.0, today.1, today.2));
    std::fs::read_dir(day).map(|rd| rd.flatten().filter(|e| e.path().join(META).is_file()).count()).unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod tests;
