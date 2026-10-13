//! Capture history: every capture is kept (unless `history = false`) in
//! `%LOCALAPPDATA%\rustshot-data\history\<yyyy-mm-dd>\<hhmmss>\` (Linux:
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
//! The daemon prunes when it starts, hourly and when the limits change
//! ([`keep_pruned`]), with History on or off; entries open in an Edit
//! window stay ([`hold_open`]), and folders left without a valid
//! `meta.json` (a crash mid-write) go after an hour ([`GRACE`]).
//!
//! Writing (PNG encoding a 4K capture takes a while) runs on one worker
//! thread ([`record`], [`update`], [`flush`]) in order; the folder is
//! reserved at once, so its id is known before the files are there.
//! Every change of an entry's files is a read-modify-write under the
//! entry's lock ([`lock_entry`]: a mutex here, a lock file across
//! processes), and every file is written whole through a uniquely named,
//! flushed temporary file.

pub mod source;

use crate::export::Tm;
use crate::objects::Obj;
use crate::objects::json::{self, V};
use crate::pixbuf::PixBuf;
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use crate::chan::{Receiver, Sender};

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
    /// The upload's record (its deletehash, time, kind, 72 × 48
    /// thumbnail; [`crate::uploads`]), when it was kept.
    pub upload: Option<crate::uploads::Info>,
    /// A recording's length.
    pub duration_ms: u64,
    /// A recording's file.
    pub file: Option<PathBuf>,
    /// Edited in the Edit window since.
    pub edited: bool,
    /// Its objects' kinds, bottom-most first ("Arrow", "Number").
    pub objects: Vec<String>,
    /// A freeform capture's lasso (`original.png`'s pixels): outside it
    /// the picture is transparent (white in a JPG or BMP).
    pub mask: Option<Vec<crate::objects::Pt>>,
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
        Obj::Blur { .. } => "Blur",
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

/// The civil date `days` after 1970-01-01 (also S3 request dates).
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
    // Straight quotes: the baked UI font has no curly ones.
    format!("{n} result{} for \"{}\"", if n == 1 { "" } else { "s" }, q.trim())
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
    crate::sort::by_key(&mut hits, |r| r.start);
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

/// Captures, Screenshots, Recordings or Uploaded (the header's filter).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Filter {
    #[default]
    All,
    Shots,
    Recordings,
    /// Those with an upload link.
    Uploaded,
}

impl Filter {
    pub fn keeps(self, k: Kind) -> bool {
        match self {
            Filter::All | Filter::Uploaded => true,
            Filter::Shots => !k.is_recording(),
            Filter::Recordings => k.is_recording(),
        }
    }

    /// Whether the capture `m` is shown.
    pub fn keeps_meta(self, m: &Meta) -> bool {
        match self {
            Filter::Uploaded => m.uploaded.is_some(),
            f => f.keeps(m.kind),
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
    if let Some(i) = &m.upload {
        // Kept on this computer only (PRIVACY.md); never logged.
        s("upload_deletehash", i.deletehash.reveal());
        s("upload_kind", i.kind.id());
        if let Some(obj) = &i.s3 {
            s("upload_dest", i.dest());
            for (k, v) in crate::uploads::s3_fields(obj) {
                s(&format!("upload_{k}"), &v);
            }
        }
        if let Some(t) = &i.thumb {
            s("upload_thumb", &crate::uploads::b64_encode(t));
        }
    }
    if let Some(p) = &m.file {
        s("file", &p.to_string_lossy());
    }
    let scale = if m.scale.is_finite() { m.scale } else { 1.0 };
    o.push_str(&format!(
        ",\"w\":{},\"h\":{},\"bytes\":{},\"scale\":{},\"duration_ms\":{},\"edited\":{}",
        m.w, m.h, m.bytes, scale, m.duration_ms, m.edited
    ));
    if let Some(i) = &m.upload {
        o.push_str(&format!(",\"upload_time\":{}", i.time));
    }
    // An unreadable lasso stays (empty: still unreadable, fail closed).
    if let Some(pts) = m.mask.as_ref() {
        o.push_str(",\"mask\":[");
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let f = |v: f32| if v.is_finite() { v } else { 0.0 };
            o.push_str(&format!("[{},{}]", f(p.x), f(p.y)));
        }
        o.push(']');
    }
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

/// `meta.json`'s `mask`: points `[x, y]`. A mask that is there but cannot
/// be read is kept as an empty one (fail closed: nothing of the picture
/// shows, [`mask_unreadable`]).
fn mask_of(v: Option<&V>) -> Option<Vec<crate::objects::Pt>> {
    let v = v?;
    let V::Arr(a) = v else { return Some(Vec::new()) };
    let pts: Option<Vec<crate::objects::Pt>> = a
        .iter()
        .map(|p| match p {
            V::Arr(xy) if xy.len() == 2 => match (&xy[0], &xy[1]) {
                (V::Num(x), V::Num(y)) if x.is_finite() && y.is_finite() => Some(crate::objects::Pt::new(*x as f32, *y as f32)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    Some(pts.filter(|p| p.len() <= json::MAX_POINTS && crate::objects::mask::is_shape(p)).unwrap_or_default())
}

/// The entry has a lasso that cannot be read: its picture is not shown.
pub fn mask_unreadable(m: &Meta) -> bool {
    m.mask.as_ref().is_some_and(|p| !crate::objects::mask::is_shape(p))
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
        upload: opt("uploaded").and(opt("upload_kind")).map(|k| crate::uploads::Info {
            deletehash: crate::uploads::Secret::new(opt("upload_deletehash").unwrap_or_default()),
            time: num("upload_time") as u64,
            kind: crate::uploads::UpKind::from_id(&k),
            thumb: opt("upload_thumb").and_then(|t| crate::uploads::b64_decode(&t)),
            s3: (opt("upload_dest").as_deref() == Some("s3")).then(|| crate::uploads::s3_from_fields(|k| opt(&format!("upload_{k}")))).flatten(),
        }),
        uploaded: opt("uploaded"),
        duration_ms: num("duration_ms") as u64,
        file: opt("file").map(PathBuf::from),
        edited: matches!(get("edited"), Some(V::Bool(true))),
        objects,
        mask: mask_of(get("mask")),
    })
}

// --- the store -----------------------------------------------------------------

/// The history folder: always a local (never a roaming, synced) one;
/// None when there is no such folder (then nothing is kept).
pub fn find_root() -> Option<PathBuf> {
    let get = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute());
    #[cfg(windows)]
    let base = get("LOCALAPPDATA").or_else(|| get("USERPROFILE").map(|h| h.join("AppData").join("Local")));
    #[cfg(target_os = "macos")]
    let base = get("HOME").map(|h| h.join("Library").join("Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = get("XDG_DATA_HOME").or_else(|| get("HOME").map(|h| h.join(".local").join("share")));
    base.map(|b| root_in(&b))
}

/// The history folder under the local data folder `base`. On Windows it
/// is in `rustshot-data`, not `rustshot`: `%LOCALAPPDATA%\rustshot` is the
/// installer's program folder.
pub fn root_in(base: &Path) -> PathBuf {
    let app = if cfg!(windows) { "rustshot-data" } else { "rustshot" };
    base.join(app).join("history")
}

/// The history folder the windows show ([`find_root`]; without one, an
/// empty folder in the temporary directory, where nothing is recorded).
pub fn default_root() -> PathBuf {
    find_root().unwrap_or_else(|| std::env::temp_dir().join("rustshot").join("history"))
}

/// The history folder.
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}

/// Bytes under `p` (symbolic links are not followed).
fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => dir_size(&e.path()),
                    Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
                    _ => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

/// An entry folder's name ("143207", "143207-2").
fn entry_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Folders without a valid `meta.json` and stray temporary files are
/// left alone this long (they may be being written), then pruned.
pub const GRACE: std::time::Duration = std::time::Duration::from_secs(3600);

/// The file each entry's writers lock ([`lock_entry`]).
const LOCK: &str = ".lock";
/// The file an Edit window holds (shared-locked) while it has the entry
/// open ([`hold_open`]): pruning in any process leaves the entry alone.
const OPEN: &str = ".open";

/// Entry folders being written by a thread of this process.
static BUSY: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static BUSY_FREE: std::sync::Condvar = std::sync::Condvar::new();

/// One writer of an entry at a time: the folder is taken in this process
/// (other threads wait) and its lock file locked (other processes wait).
pub struct EntryLock {
    dir: PathBuf,
    file: Option<std::fs::File>,
}

impl Drop for EntryLock {
    fn drop(&mut self) {
        drop(self.file.take()); // unlocks
        let mut b = lock(&BUSY);
        b.retain(|d| *d != self.dir);
        BUSY_FREE.notify_all();
    }
}

/// Lock entry folder `dir` for a read-modify-write (it must exist).
pub fn lock_entry(dir: &Path) -> Result<EntryLock> {
    {
        let mut b = lock(&BUSY);
        while b.iter().any(|d| d == dir) {
            b = BUSY_FREE.wait(b).unwrap_or_else(|e| e.into_inner());
        }
        b.push(dir.to_path_buf());
    }
    // From here the guard releases the folder, whatever happens.
    let mut g = EntryLock { dir: dir.to_path_buf(), file: None };
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK))
        .with_context(|| format!("lock {}", dir.display()))?;
    f.lock().with_context(|| format!("lock {}", dir.display()))?;
    g.file = Some(f);
    Ok(g)
}

/// An entry open in an Edit window: never pruned while this lives (in
/// this process: [`keep_now`]; in others: the shared lock on `.open`).
pub struct OpenGuard {
    id: String,
    _file: Option<std::fs::File>,
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        let mut e = lock(&EDITING);
        if let Some(i) = e.iter().position(|x| *x == self.id) {
            e.remove(i);
        }
    }
}

/// Entries open in Edit windows of this process.
static EDITING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Mark entry `id` (folder `dir`) as open for editing.
#[cfg_attr(target_os = "macos", allow(dead_code))] // no Edit window there yet
pub fn hold_open(id: &str, dir: &Path) -> OpenGuard {
    lock(&EDITING).push(id.to_string());
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(OPEN)).ok().filter(|f| f.lock_shared().is_ok());
    OpenGuard { id: id.to_string(), _file: file }
}

/// Another process (or window) has entry folder `dir` open for editing.
fn held_elsewhere(dir: &Path) -> bool {
    let p = dir.join(OPEN);
    if !p.is_file() {
        return false;
    }
    match std::fs::OpenOptions::new().write(true).open(&p) {
        Ok(f) => matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        Err(_) => false,
    }
}

/// The entries pruning leaves alone now: being written, or open in an
/// Edit window.
pub fn keep_now() -> Vec<String> {
    let mut k = lock(&WRITING).clone();
    k.extend(lock(&EDITING).iter().cloned());
    k
}

/// `t` is at least [`GRACE`] before `now`.
fn stale(t: Option<std::time::SystemTime>, now: std::time::SystemTime) -> bool {
    t.is_some_and(|t| now.duration_since(t).is_ok_and(|d| d >= GRACE))
}

/// Remove the temporary files (`*.tmp`, from writes that never finished)
/// left in entry folder `dir` for [`GRACE`].
fn sweep_tmp(dir: &Path, now: std::time::SystemTime) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for f in rd.flatten() {
        let is_tmp = f.file_name().to_string_lossy().ends_with(".tmp");
        if is_tmp && f.file_type().is_ok_and(|t| t.is_file()) && stale(f.metadata().and_then(|m| m.modified()).ok(), now) {
            let _ = std::fs::remove_file(f.path());
        }
    }
}

/// A saved file or recording `meta` points at that may be deleted with
/// the entry: an absolute path to a regular file (not a link or a
/// folder) of a capture's format, as big as when it was recorded.
fn deletable(f: &Path, meta: &Meta) -> bool {
    const EXTS: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "mp4", "gif"];
    let ext_ok = f.extension().is_some_and(|e| EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()));
    f.is_absolute() && ext_ok && std::fs::symlink_metadata(f).is_ok_and(|m| m.file_type().is_file() && (meta.bytes == 0 || m.len() == meta.bytes))
}

/// What [`Store::scan`] finds: the entries, and the entry-shaped folders
/// without a valid `meta.json` (id, folder, bytes, last change).
struct Scan {
    entries: Vec<Entry>,
    orphans: Vec<(String, PathBuf, u64, Option<std::time::SystemTime>)>,
}

/// A day folder's name ("2026-10-12") as a date.
fn parse_day(name: &str) -> Option<(i32, u32, u32)> {
    let mut p = name.split('-');
    let y = p.next()?.parse().ok()?;
    let m = p.next()?.parse().ok().filter(|m| (1..=12).contains(m))?;
    let d = p.next()?.parse().ok().filter(|d| (1..=31).contains(d))?;
    p.next().is_none().then_some((y, m, d))
}

/// Write `bytes` to `path` through a uniquely named temporary file,
/// flushed before the rename (a reader never sees half a file).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::export::write_atomic(path, bytes)
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
        entry_name(name).then(|| self.root.join(day).join(name))
    }

    /// Entry `id`'s folder (None: not an entry id).
    pub fn dir_of(&self, id: &str) -> Option<PathBuf> {
        self.entry_dir(id)
    }

    /// A new entry's folder for a capture taken at `now`:
    /// `<yyyy-mm-dd>/<hhmmss>` (`-2`, `-3`... when taken). Returns its id.
    pub fn reserve(&self, now: Tm) -> Result<(String, PathBuf)> {
        let day = format!("{:04}-{:02}-{:02}", now.0, now.1, now.2);
        let base = format!("{:02}{:02}{:02}", now.3, now.4, now.5);
        let dd = self.root.join(&day);
        std::fs::create_dir_all(&dd).with_context(|| format!("create {}", dd.display()))?;
        let mut retries = 3;
        let mut n = 1;
        while n < 1000 {
            let name = if n == 1 { base.clone() } else { format!("{base}-{n}") };
            let dir = dd.join(&name);
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok((format!("{day}/{name}"), dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                // The day folder was just removed as empty (a prune or a
                // delete meanwhile): again.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && retries > 0 => {
                    retries -= 1;
                    std::fs::create_dir_all(&dd).with_context(|| format!("create {}", dd.display()))?;
                }
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
                made = thumbnail_doc(original, objects, None, meta.mask.as_deref(), &Default::default());
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
        let mut out = self.scan(false).entries;
        crate::sort::by(&mut out, |a, b| b.meta.captured.cmp(&a.meta.captured).then_with(|| b.id.cmp(&a.id)));
        out
    }

    /// The day folders' entry folders (real folders only: a symbolic
    /// link is never followed out of the root); with `orphans`, also
    /// those without a valid `meta.json`.
    fn scan(&self, orphans: bool) -> Scan {
        let mut s = Scan { entries: Vec::new(), orphans: Vec::new() };
        let Ok(days) = std::fs::read_dir(&self.root) else { return s };
        for d in days.flatten() {
            let name = d.file_name().to_string_lossy().into_owned();
            if parse_day(&name).is_none() || !d.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let Ok(items) = std::fs::read_dir(d.path()) else { continue };
            for it in items.flatten() {
                let item = it.file_name().to_string_lossy().into_owned();
                if !entry_name(&item) || !it.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let id = format!("{name}/{item}");
                match self.get(&id) {
                    Some(e) => s.entries.push(e),
                    None if orphans => {
                        let dir = it.path();
                        let changed = std::fs::symlink_metadata(&dir).and_then(|m| m.modified()).ok();
                        s.orphans.push((id, dir.clone(), dir_size(&dir), changed));
                    }
                    None => {}
                }
            }
        }
        s
    }

    /// The bytes the history takes.
    #[cfg(test)]
    pub fn usage(&self) -> u64 {
        dir_size(&self.root)
    }

    /// Change `id`'s `meta.json` (Edited, the saved file, the link): read,
    /// changed and written under the entry's lock, so writers in other
    /// threads and processes never lose each other's changes.
    pub fn update(&self, id: &str, f: impl FnOnce(&mut Meta)) -> Result<Meta> {
        let dir = self.entry_dir(id).ok_or_else(|| anyhow!("bad history id {id:?}"))?;
        let _l = self.lock_existing(id, &dir)?;
        self.update_locked(id, &dir, f)
    }

    fn update_locked(&self, id: &str, dir: &Path, f: impl FnOnce(&mut Meta)) -> Result<Meta> {
        let mut m = meta_from_json(&std::fs::read_to_string(dir.join(META)).with_context(|| format!("read {id}"))?)?;
        f(&mut m);
        write_atomic(&dir.join(META), meta_to_json(&m).as_bytes())?;
        Ok(m)
    }

    /// The lock of entry `id`, which must exist (an entry deleted
    /// meanwhile is not written back).
    fn lock_existing(&self, id: &str, dir: &Path) -> Result<EntryLock> {
        if !dir.join(META).is_file() {
            bail!("history entry {id} is gone");
        }
        let l = lock_entry(dir)?;
        if !dir.join(META).is_file() {
            bail!("history entry {id} is gone");
        }
        Ok(l)
    }

    /// The Edit window saved `id`: its objects, a new thumbnail (made
    /// from the original and them), "Edited", and the file it was saved
    /// to. `original.png` stays as it was.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn save_edit(&self, id: &str, objects: &[Obj], saved: Option<(PathBuf, u64)>) -> Result<Meta> {
        self.save_edit_doc(id, objects, None, &Default::default(), saved)
    }

    /// [`Store::save_edit`] with the Edit window's crop (written with its
    /// stand-ins for older builds, [`crate::objects::to_json_doc`]).
    pub fn save_edit_doc(&self, id: &str, objects: &[Obj], crop: Option<crate::objects::FRect>, edits: &crate::imgedit::Edits, saved: Option<(PathBuf, u64)>) -> Result<Meta> {
        let dir = self.entry_dir(id).ok_or_else(|| anyhow!("bad history id {id:?}"))?;
        let original = PixBuf::from_png(&std::fs::read(dir.join(ORIGINAL)).unwrap_or_default()).ok();
        let mask = std::fs::read_to_string(dir.join(META)).ok().and_then(|s| meta_from_json(&s).ok()).and_then(|m| m.mask);
        let thumb = original.as_ref().map(|o| thumbnail_doc(o, objects, crop, mask.as_deref(), edits));
        let size = original.as_ref().map_or((0, 0), PixBuf::dimensions);
        let _l = self.lock_existing(id, &dir)?;
        write_atomic(&dir.join(ANNOTATIONS), crate::objects::to_json_full(objects, crop, edits, size).as_bytes())?;
        if let Some(t) = thumb {
            write_atomic(&dir.join(THUMB), &t.to_png()?)?;
        }
        self.update_locked(id, &dir, |m| {
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn objects(&self, e: &Entry) -> Result<Vec<Obj>> {
        self.doc(e).map(|d| d.objects)
    }

    /// The objects and the crop of `e`.
    pub fn doc(&self, e: &Entry) -> Result<crate::objects::Doc> {
        match std::fs::read_to_string(e.annotations()) {
            Ok(t) => crate::objects::from_json_doc(&t),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Remove entry `id`; with `files` also the file it was saved to (or
    /// the recording).
    pub fn delete(&self, id: &str, files: bool) -> Result<()> {
        let dir = self.entry_dir(id).ok_or_else(|| anyhow!("bad history id {id:?}"))?;
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_dir()) {
            bail!("no history entry {id}");
        }
        // `meta.json` goes first, under the lock: a writer waiting for it
        // finds the entry gone, and a delete that stops half way leaves a
        // folder pruning recognises as left over.
        let meta = {
            let _l = lock_entry(&dir)?;
            let meta = self.get(id).map(|e| e.meta);
            match std::fs::remove_file(dir.join(META)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e).with_context(|| format!("remove {}", dir.display())),
                _ => {}
            }
            meta
        };
        if files && let Some(m) = &meta {
            for f in m.saved.iter().chain(m.file.iter()).filter(|f| deletable(f, m)) {
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
    /// files too. Returns how many went. Entries being written stay.
    pub fn clear(&self, files: bool) -> usize {
        let writing = lock(&WRITING).clone();
        let s = self.scan(true);
        let n = s.entries.iter().filter(|e| !writing.contains(&e.id) && self.delete(&e.id, files).is_ok()).count();
        // Folders without meta.json (half written, from a crash) go too.
        for (id, dir, _, _) in &s.orphans {
            if !writing.contains(id) {
                let _ = std::fs::remove_dir_all(dir);
                if let Some(day) = dir.parent() {
                    let _ = std::fs::remove_dir(day);
                }
            }
        }
        n
    }

    /// Keep the history within `days` (entries from days before that go)
    /// and `max_bytes` (then the oldest go until it fits); returns the ids
    /// removed. `keep`: entries being written or edited now stay (so do
    /// those open in another process's Edit window). Folders without a
    /// valid `meta.json` and stray temporary files go once they are
    /// [`GRACE`] old (until then their bytes count); saved files and
    /// recordings outside the history are never touched.
    pub fn prune(&self, today: (i32, u32, u32), days: u32, max_bytes: u64, keep: &[String]) -> Vec<String> {
        self.prune_at(today, days, max_bytes, keep, std::time::SystemTime::now())
    }

    pub(crate) fn prune_at(&self, today: (i32, u32, u32), days: u32, max_bytes: u64, keep: &[String], now: std::time::SystemTime) -> Vec<String> {
        let t = days_from_civil(today.0, today.1, today.2);
        let Scan { entries: mut all, orphans } = self.scan(true);
        crate::sort::by(&mut all, |a, b| b.meta.captured.cmp(&a.meta.captured).then_with(|| b.id.cmp(&a.id)));
        let kept = |id: &String, dir: &Path| keep.contains(id) || held_elsewhere(dir);
        let mut gone = Vec::new();
        let mut young: u64 = 0;
        for (id, dir, size, changed) in orphans {
            if !kept(&id, &dir) && stale(changed, now) && std::fs::remove_dir_all(&dir).is_ok() {
                if let Some(day) = dir.parent() {
                    let _ = std::fs::remove_dir(day);
                }
                gone.push(id);
            } else {
                young += size;
            }
        }
        for e in &all {
            sweep_tmp(&e.dir, now);
        }
        all.retain(|e| {
            let (y, m, d) = e.day();
            let old = t - days_from_civil(y, m, d) >= days as i64;
            if old && !kept(&e.id, &e.dir) && self.delete(&e.id, false).is_ok() {
                gone.push(e.id.clone());
                return false;
            }
            true
        });
        let mut total: u64 = young + all.iter().map(|e| e.size).sum::<u64>();
        // Oldest first.
        for e in all.iter().rev() {
            if total <= max_bytes {
                break;
            }
            if kept(&e.id, &e.dir) {
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

/// The thumbnail of a screenshot as it exports ([`render`]).
pub fn thumbnail_doc(original: &PixBuf, objects: &[Obj], crop: Option<crate::objects::FRect>, mask: Option<&[crate::objects::Pt]>, edits: &crate::imgedit::Edits) -> PixBuf {
    crate::imgedit::render_thumb(original, objects, crop, mask, edits, THUMB_W, THUMB_H)
}

/// The picture as it exports: `original` with `objects`, transparent
/// outside the lasso `mask`, cut to `crop`.
pub fn render(original: &PixBuf, objects: &[Obj], crop: Option<crate::objects::FRect>, mask: Option<&[crate::objects::Pt]>) -> PixBuf {
    // What the lasso and the crop leave out is blanked first: a magnifier
    // (or a blur, an invert) over the rest never reads it.
    let mut plain = original.clone();
    crate::objects::mask::blank_outside(&mut plain, crop, mask);
    let mut img = if objects.is_empty() {
        plain
    } else {
        let font = crate::text::AnnotFont::load();
        crate::editor::bake_objects(&plain, objects, font.as_ref())
    };
    if let Some(m) = mask {
        crate::objects::mask::apply(&mut img, m, crate::objects::Pt::new(0.0, 0.0));
    }
    match crop {
        Some(c) => crate::objects::mask::crop(&img, c),
        None => img,
    }
}

/// The saved files of `entries` that are still there: how many, their
/// bytes, and the folder most of them are in (for "Also delete saved
/// files (1.2 GB in ~/Pictures/rustshot)").
pub fn saved_files(entries: &[Entry]) -> (usize, u64, Option<PathBuf>) {
    let mut n = 0;
    let mut bytes = 0;
    let mut dirs: Vec<(PathBuf, usize)> = Vec::new();
    for (f, m) in entries.iter().flat_map(|e| e.meta.saved.iter().chain(e.meta.file.iter()).map(move |f| (f, &e.meta))) {
        // Only the files "Also delete saved files" would delete.
        if !deletable(f, m) {
            continue;
        }
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
    crate::sort::by_key(&mut dirs, |d| std::cmp::Reverse(d.1));
    (n, bytes, dirs.into_iter().next().map(|d| d.0))
}

/// One line of `rustshot history --list`, tab-separated: when it was
/// captured, the kind (screenshot, mp4, gif), the size in px, the bytes
/// and its file (where it was saved, a recording's file, else its
/// `original.png`).
pub fn list_line(e: &Entry) -> String {
    let m = &e.meta;
    let t = m.captured;
    let file = m.saved.clone().or_else(|| m.file.clone()).unwrap_or_else(|| e.original());
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}\t{}\t{}x{}\t{}\t{}",
        t.0,
        t.1,
        t.2,
        t.3,
        t.4,
        t.5,
        m.kind.id(),
        m.w,
        m.h,
        m.bytes,
        file.display()
    )
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
    Recording { root: PathBuf, id: String, dir: PathBuf, poster: Option<PixBuf>, meta: Box<Meta>, days: u32, max: u64 },
    Update { root: PathBuf, id: String, f: Box<dyn FnOnce(&mut Meta) + Send> },
    Prune { root: PathBuf, days: u32, max: u64 },
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
        let (tx, rx) = crate::chan::channel();
        std::thread::Builder::new().name("rustshot-history".into()).spawn(move || run(rx)).expect("history thread");
        tx
    });
    // The thread never ends while we hold its sender (a panic aborts).
    let _ = tx.send(job);
}

fn today(t: &Tm) -> (i32, u32, u32) {
    (t.0, t.1, t.2)
}

/// Prune with the keep list as it is now (being written, open for editing).
fn prune_now(st: &Store, today: (i32, u32, u32), days: u32, max: u64) {
    let keep = keep_now();
    if !st.prune(today, days, max, &keep).is_empty() {
        changed();
    }
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
                prune_now(&st, today(&shot.meta.captured), days, max);
                changed();
            }
            Job::Recording { root, id, dir, poster, meta, days, max } => {
                let st = Store::new(root);
                if let Err(e) = st.write_recording(&dir, poster.as_ref(), &meta) {
                    eprintln!("rustshot: history: {e:#}");
                    let _ = std::fs::remove_dir_all(&dir);
                }
                lock(&WRITING).retain(|w| *w != id);
                prune_now(&st, today(&meta.captured), days, max);
                changed();
            }
            Job::Update { root, id, f } => {
                if let Err(e) = Store::new(root).update(&id, f) {
                    eprintln!("rustshot: history: {e:#}");
                }
                changed();
            }
            Job::Prune { root, days, max } => {
                let t = crate::export::local_ymdhms();
                prune_now(&Store::new(root), today(&t), days, max);
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
    lock(&WRITING).push(id.clone());
    let (days, max) = limits(cfg);
    send(Job::Recording { root: root.to_path_buf(), id: id.clone(), dir, poster, meta: Box::new(meta), days, max });
    Some(id)
}

/// The limits the daemon prunes with (the history folder, days, bytes).
static LIMITS: Mutex<Option<(PathBuf, u32, u64)>> = Mutex::new(None);
static PRUNER: std::sync::Once = std::sync::Once::new();
/// How often the daemon prunes (captures age out without new ones).
pub const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// The daemon keeps the history within `cfg`'s limits, whether it is on
/// or off and whether captures come or not: a prune now when the limits
/// are new (the daemon started, or Settings changed them), then hourly.
pub fn keep_pruned(root: &Path, cfg: &crate::config::Config) {
    let (days, max) = limits(cfg);
    let new = Some((root.to_path_buf(), days, max));
    let changed = {
        let mut l = lock(&LIMITS);
        let c = *l != new;
        *l = new;
        c
    };
    if changed {
        send(Job::Prune { root: root.to_path_buf(), days, max });
    }
    PRUNER.call_once(|| {
        let _ = std::thread::Builder::new().name("rustshot-history-prune".into()).spawn(|| loop {
            std::thread::sleep(PRUNE_EVERY);
            if let Some((root, days, max)) = lock(&LIMITS).clone() {
                send(Job::Prune { root, days, max });
            }
        });
    });
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
    let (tx, rx) = crate::chan::channel();
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
