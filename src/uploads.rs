//! Upload records: each upload's link, where it went (Imgur and its
//! deletehash, the secret that deletes it again; or an S3 bucket and the
//! object's key, [`Info::s3`]), when, whether it was an image or a video,
//! and a 72 × 48 thumbnail. "Delete upload…" ([`delete_record`]) deletes
//! from Imgur or sends the bucket a signed `DELETE` (`crate::s3`). With History on and the upload belonging to an
//! entry they live in its `meta.json` ([`crate::history::Meta::upload`]);
//! otherwise in a small `uploads.json` next to the history folder
//! (`%LOCALAPPDATA%\rustshot\uploads.json`, `$XDG_DATA_HOME/rustshot/…`),
//! the last [`KEPT`] of them. The History window copies a link and
//! deletes an upload from Imgur ([`delete_remote`]: `DELETE
//! /3/image/{deletehash}` with the app's Client-ID); the tray lists the
//! last five ([`recent`]).
//!
//! The deletehash is kept on this computer only and never logged: it is a
//! [`Secret`], whose `Debug` hides it.
//!
//! The upload code (`export::do_upload`, `imgur::upload_video`) hands the
//! reply to [`note_reply`] (the deletehash, kept in memory by link; an S3
//! upload its object to [`note_s3`]) and the
//! upload thread its thumbnail to [`note_thumb`]; whoever learns that the
//! upload finished calls [`finished`] (or [`finished_in`] with a
//! [`Store`]), which files the record where it belongs.

use crate::history::{Meta, Store};
use crate::objects::json::{self, V};
use crate::pixbuf::PixBuf;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The thumbnail's size (the History list's 72 × 48).
pub const THUMB_W: u32 = 72;
pub const THUMB_H: u32 = 48;
/// `uploads.json` keeps this many (the newest).
pub const KEPT: usize = 50;
/// The tray's "Recent uploads" lists this many.
pub const RECENT: usize = 5;
const VERSION: u32 = 1;

/// A deletehash: compared and stored, never printed.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into())
    }

    /// The value itself (for the request and the file only).
    pub fn reveal(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() { "Secret(none)" } else { "Secret(hidden)" })
    }
}

/// What was uploaded.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum UpKind {
    #[default]
    Image,
    Video,
}

impl UpKind {
    pub fn id(self) -> &'static str {
        match self {
            UpKind::Image => "image",
            UpKind::Video => "video",
        }
    }

    pub fn from_id(s: &str) -> UpKind {
        if s == "video" { UpKind::Video } else { UpKind::Image }
    }
}

/// What is kept about an upload besides its link.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Info {
    pub deletehash: Secret,
    /// Seconds since 1970 (UTC) it finished.
    pub time: u64,
    pub kind: UpKind,
    /// A 72 × 48 PNG.
    pub thumb: Option<Vec<u8>>,
    /// Uploaded to an S3-compatible bucket (destination "s3"): where the
    /// object is (its endpoint, region, bucket and key). None: Imgur.
    pub s3: Option<crate::s3::Object>,
}

impl Info {
    /// The destination's id as the records keep it.
    pub fn dest(&self) -> &'static str {
        if self.s3.is_some() { "s3" } else { "imgur" }
    }

    /// "Delete upload…" can delete it: an S3 object, or an Imgur upload
    /// whose deletehash was kept.
    pub fn deletable(&self) -> bool {
        match &self.s3 {
            Some(o) => !o.key.is_empty() && !o.target.bucket.is_empty(),
            None => valid_deletehash(self.deletehash.reveal()),
        }
    }
}

/// One upload: its link and the rest.
#[derive(Clone, PartialEq, Debug)]
pub struct Record {
    pub link: String,
    pub info: Info,
    /// The History entry it belongs to (none in `uploads.json`).
    pub entry: Option<String>,
}

// --- base64 (the thumbnails in JSON) ------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| B64.iter().position(|&b| b == c).map(|p| p as u32);
    let s = s.trim().trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in s {
        acc = acc << 6 | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits & 0xFF) as u8);
        }
    }
    Some(out)
}

// --- thumbnails ----------------------------------------------------------------

/// `img` cut to 3:2 around its centre and scaled to 72 × 48.
pub fn thumb_of(img: &PixBuf) -> PixBuf {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return PixBuf::new(THUMB_W, THUMB_H);
    }
    // The largest 3:2 box inside.
    let (cw, ch) = if w * THUMB_H > h * THUMB_W { ((h * THUMB_W / THUMB_H).max(1), h) } else { (w, (w * THUMB_H / THUMB_W).max(1)) };
    let (x0, y0) = ((w - cw) / 2, (h - ch) / 2);
    let src = img.as_raw();
    let mut crop = Vec::with_capacity(cw as usize * ch as usize * 4);
    for y in y0..y0 + ch {
        let i = (y as usize * w as usize + x0 as usize) * 4;
        crop.extend_from_slice(&src[i..i + cw as usize * 4]);
    }
    let crop = PixBuf::from_raw(cw, ch, crop).unwrap_or_else(|| PixBuf::new(1, 1));
    crate::pin::resample(&crop, THUMB_W, THUMB_H)
}

/// [`thumb_of`] a PNG, as a PNG.
pub fn thumb_png(png: &[u8]) -> Option<Vec<u8>> {
    thumb_of(&PixBuf::from_png(png).ok()?).to_png().ok()
}

// --- what the upload code tells us ---------------------------------------------

/// Uploads finished in this process whose records are not filed yet: the
/// link, the deletehash, the kind and the thumbnail.
struct Note {
    link: String,
    deletehash: Secret,
    kind: UpKind,
    thumb: Option<Vec<u8>>,
    s3: Option<crate::s3::Object>,
}

static NOTES: Mutex<Vec<Note>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn note_mut(link: &str, f: impl FnOnce(&mut Note)) {
    let mut n = lock(&NOTES);
    if let Some(i) = n.iter().position(|x| x.link == link) {
        f(&mut n[i]);
        return;
    }
    let mut x = Note { link: link.to_string(), deletehash: Secret::default(), kind: UpKind::Image, thumb: None, s3: None };
    f(&mut x);
    n.push(x);
    // Records nobody filed (a one-shot run that exited) do not pile up.
    if n.len() > 16 {
        n.remove(0);
    }
}

/// `"key":"value"` in an Imgur reply (`\/` unescaped).
fn json_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let i = body.find(&needle)? + needle.len();
    let rest = body[i..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].replace("\\/", "/"))
}

/// The link and deletehash of an Imgur upload reply.
pub fn parse_reply(body: &str) -> Option<(String, Secret)> {
    let link = json_field(body, "link")?;
    let dh = json_field(body, "deletehash").unwrap_or_default();
    Some((link, Secret::new(dh)))
}

/// `body` with any deletehash in it blanked (for an error message).
pub fn redact(body: &str) -> String {
    match json_field(body, "deletehash").filter(|d| !d.is_empty()) {
        Some(dh) => body.replace(&dh, "\u{2026}"),
        None => body.to_string(),
    }
}

/// An upload reply came in (`kind`): keep its deletehash for [`finished`].
pub fn note_reply(body: &str, kind: UpKind) {
    if let Some((link, dh)) = parse_reply(body) {
        note_mut(&link, |n| {
            n.deletehash = dh;
            n.kind = kind;
        });
    }
}

/// An upload to an S3-compatible bucket finished at `link`: where the
/// object is, for its record (and "Delete upload…").
pub fn note_s3(link: &str, obj: crate::s3::Object, kind: UpKind) {
    note_mut(link, |n| {
        n.s3 = Some(obj);
        n.kind = kind;
    });
}

/// The upload of `link` had this picture (a PNG): its thumbnail.
pub fn note_thumb(link: &str, thumb: Option<Vec<u8>>) {
    note_mut(link, |n| n.thumb = thumb);
}

/// The link of an upload changed (a video's, after processing): its
/// notes follow.
pub fn rekey(old: &str, new: &str) {
    if let Some(x) = lock(&NOTES).iter_mut().find(|x| x.link == old) {
        x.link = new.to_string();
    }
}

/// What is known about the upload of `link` (taken: filed once).
fn take(link: &str) -> Option<Note> {
    let mut n = lock(&NOTES);
    let i = n.iter().position(|x| x.link == link)?;
    Some(n.remove(i))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The record of the finished upload of `link` (what [`note_reply`] and
/// [`note_thumb`] said; `thumb` when they had none).
fn info_for(link: &str, thumb: impl FnOnce() -> Option<Vec<u8>>) -> Info {
    let n = take(link);
    let (deletehash, kind, t, s3) = match n {
        Some(n) => (n.deletehash, n.kind, n.thumb, n.s3),
        None => (Secret::default(), if link.ends_with(".mp4") { UpKind::Video } else { UpKind::Image }, None, None),
    };
    Info { deletehash, time: unix_now(), kind, thumb: t.or_else(thumb), s3 }
}

/// The 72 × 48 PNG of an entry's `thumb.png` in `dir`.
fn entry_thumb(dir: Option<PathBuf>) -> Option<Vec<u8>> {
    let png = std::fs::read(dir?.join(crate::history::THUMB)).ok()?;
    thumb_png(&png)
}

/// The upload of `link` finished: with History entry `entry` (`root`, id)
/// its meta gets the link and the record (on the history thread, after
/// the entry's files); without one, `uploads.json` does.
pub fn finished(link: &str, entry: Option<(&Path, &str)>) {
    match entry {
        Some((root, id)) => {
            let (root, id, link) = (root.to_path_buf(), id.to_string(), link.to_string());
            let r2 = root.clone();
            let id2 = id.clone();
            crate::history::update(&root, &id, move |m| {
                let info = info_for(&link, || entry_thumb(Store::new(&r2).dir_of(&id2)));
                index_add(Record { link: link.clone(), info: info.clone(), entry: Some(id2.clone()) });
                set(m, link, info);
                changed();
            });
        }
        None => {
            let info = info_for(link, || None);
            index_add(Record { link: link.to_string(), info: info.clone(), entry: None });
            if let Some(b) = Book::default_book()
                && let Err(e) = b.add(Record { link: link.to_string(), info, entry: None })
            {
                eprintln!("rustshot: could not keep the upload in {}: {e}", b.path.display());
            }
        }
    }
    changed();
}

/// [`finished`] for an entry of `store`, written now (the History and Edit
/// windows' workers).
pub fn finished_in(store: &Store, id: &str, link: &str) -> anyhow::Result<Meta> {
    let dir = store.dir_of(id);
    let info = info_for(link, || entry_thumb(dir));
    let link = link.to_string();
    index_add(Record { link: link.clone(), info: info.clone(), entry: Some(id.to_string()) });
    let r = store.update(id, move |m| set(m, link, info));
    changed();
    r
}

fn set(m: &mut Meta, link: String, info: Info) {
    m.uploaded = Some(link);
    m.upload = Some(info);
}

// --- uploads.json ----------------------------------------------------------------

/// An S3 object as the records keep it: `s3_endpoint`, `s3_region`,
/// `s3_bucket`, `s3_key`, `s3_path_style` ("1" or "0").
pub fn s3_fields(o: &crate::s3::Object) -> [(&'static str, String); 5] {
    [
        ("s3_endpoint", o.target.endpoint.clone()),
        ("s3_region", o.target.region.clone()),
        ("s3_bucket", o.target.bucket.clone()),
        ("s3_key", o.key.clone()),
        ("s3_path_style", if o.target.path_style { "1" } else { "0" }.into()),
    ]
}

/// [`s3_fields`] read back (`get` a field's text); None without a key
/// or bucket.
pub fn s3_from_fields(get: impl Fn(&str) -> Option<String>) -> Option<crate::s3::Object> {
    let key = get("s3_key").filter(|k| !k.is_empty())?;
    let bucket = get("s3_bucket").filter(|b| !b.is_empty())?;
    Some(crate::s3::Object {
        target: crate::s3::Target {
            endpoint: get("s3_endpoint").unwrap_or_default(),
            region: get("s3_region").unwrap_or_default(),
            bucket,
            path_style: get("s3_path_style").as_deref() != Some("0"),
        },
        key,
    })
}

/// `uploads.json` (and where it is).
#[derive(Clone, Debug)]
pub struct Book {
    pub path: PathBuf,
}

/// The records as `uploads.json`.
pub fn book_to_json(recs: &[Record]) -> String {
    let mut o = format!("{{\"version\":{VERSION},\"uploads\":[");
    for (i, r) in recs.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str("{\"link\":");
        json::string(&mut o, &r.link);
        o.push_str(",\"deletehash\":");
        json::string(&mut o, r.info.deletehash.reveal());
        o.push_str(&format!(",\"time\":{},\"kind\":\"{}\"", r.info.time, r.info.kind.id()));
        if let Some(obj) = &r.info.s3 {
            o.push_str(&format!(",\"dest\":\"{}\"", r.info.dest()));
            for (k, v) in s3_fields(obj) {
                o.push(',');
                json::string(&mut o, k);
                o.push(':');
                json::string(&mut o, &v);
            }
        }
        if let Some(t) = &r.info.thumb {
            o.push_str(",\"thumb\":");
            json::string(&mut o, &b64_encode(t));
        }
        o.push('}');
    }
    o.push_str("]}\n");
    o
}

/// The records of an `uploads.json` (a bad one is empty).
pub fn book_from_json(s: &str) -> Vec<Record> {
    let Ok(V::Obj(top)) = json::parse(s) else { return Vec::new() };
    let Some((_, V::Arr(list))) = top.iter().find(|(k, _)| k == "uploads") else { return Vec::new() };
    list.iter()
        .filter_map(|v| {
            let V::Obj(f) = v else { return None };
            let s = |k: &str| match f.iter().find(|(n, _)| n == k) {
                Some((_, V::Str(s))) => Some(s.clone()),
                _ => None,
            };
            let time = match f.iter().find(|(n, _)| n == "time") {
                Some((_, V::Num(n))) if *n >= 0.0 => *n as u64,
                _ => 0,
            };
            let link = s("link").filter(|l| !l.is_empty())?;
            let info = Info {
                deletehash: Secret::new(s("deletehash").unwrap_or_default()),
                time,
                kind: UpKind::from_id(&s("kind").unwrap_or_default()),
                thumb: s("thumb").and_then(|t| b64_decode(&t)),
                s3: (s("dest").as_deref() == Some("s3")).then(|| s3_from_fields(|k| s(k))).flatten(),
            };
            Some(Record { link, info, entry: None })
        })
        .collect()
}

impl Book {
    pub fn at(path: impl Into<PathBuf>) -> Book {
        Book { path: path.into() }
    }

    /// `uploads.json` beside the history folder (none: no such folder).
    pub fn default_book() -> Option<Book> {
        if cfg!(test) {
            return None; // tests never write the user's records
        }
        crate::history::find_root().and_then(|r| r.parent().map(|p| Book::at(p.join("uploads.json"))))
    }

    /// The records, newest first.
    pub fn list(&self) -> Vec<Record> {
        std::fs::read_to_string(&self.path).map(|s| book_from_json(&s)).unwrap_or_default()
    }

    /// Change the records under the file's lock (read, `f`, written whole).
    fn edit<R>(&self, f: impl FnOnce(&mut Vec<Record>) -> R) -> anyhow::Result<R> {
        crate::config::with_config_lock(&self.path, || {
            let mut recs = self.list();
            let r = f(&mut recs);
            if let Some(d) = self.path.parent() {
                std::fs::create_dir_all(d)?;
            }
            crate::export::write_atomic(&self.path, book_to_json(&recs).as_bytes())?;
            Ok(r)
        })
    }

    /// Keep `rec` (first: newest), the last [`KEPT`] stay.
    pub fn add(&self, rec: Record) -> anyhow::Result<()> {
        self.edit(|recs| {
            recs.retain(|r| r.link != rec.link);
            recs.insert(0, rec);
            recs.truncate(KEPT);
        })
    }

    /// Forget the upload of `link` (deleted from Imgur); whether it was kept.
    pub fn remove(&self, link: &str) -> anyhow::Result<bool> {
        self.edit(|recs| {
            let n = recs.len();
            recs.retain(|r| r.link != link);
            recs.len() != n
        })
    }
}

// --- the tray's list ---------------------------------------------------------------

/// The newest [`RECENT`] uploads, from History's entries (`entries`,
/// newest first) and `uploads.json` (`book`) together.
pub fn merge_recent(entries: &[crate::history::Entry], book: Vec<Record>) -> Vec<Record> {
    let mut all: Vec<Record> = entries
        .iter()
        .filter_map(|e| {
            let link = e.meta.uploaded.clone()?;
            let mut info = e.meta.upload.clone().unwrap_or_default();
            if info.time == 0 {
                // Uploaded before records were kept: its capture's time.
                let t = e.meta.captured;
                info.time = crate::history::days_from_civil(t.0, t.1, t.2).max(0) as u64 * 86_400 + (t.3 * 3600 + t.4 * 60 + t.5) as u64;
            }
            Some(Record { link, info, entry: Some(e.id.clone()) })
        })
        .collect();
    all.extend(book);
    all.sort_by_key(|r| std::cmp::Reverse(r.info.time));
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert(r.link.clone()));
    all.truncate(RECENT);
    all
}

/// The tray's list: built once off the caller's thread ([`warm`]), then
/// kept up to date as uploads are filed and forgotten. `None`: not built.
static INDEX: Mutex<Option<Vec<Record>>> = Mutex::new(None);
static BUILDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// (Re)build the list on a worker (the daemon starts; History cleared).
pub fn warm() {
    if cfg!(test) || BUILDING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("rustshot-uploads".into()).spawn(|| {
        let entries = Store::open_default().list();
        let book = Book::default_book().map(|b| b.list()).unwrap_or_default();
        *lock(&INDEX) = Some(merge_recent(&entries, book));
        BUILDING.store(false, std::sync::atomic::Ordering::SeqCst);
        crate::tray::uploads_changed();
    });
}

/// Put `rec` first in the list (once built).
fn index_add(rec: Record) {
    if let Some(v) = lock(&INDEX).as_mut() {
        v.retain(|r| r.link != rec.link);
        v.insert(0, rec);
        v.truncate(RECENT);
    }
}

fn index_remove(link: &str) {
    if let Some(v) = lock(&INDEX).as_mut() {
        v.retain(|r| r.link != link);
    }
}

/// The tray's "Recent uploads": never lists anything on the caller's
/// thread (empty until [`warm`] is done).
pub fn recent() -> Vec<Record> {
    let v = lock(&INDEX).clone();
    v.unwrap_or_else(|| {
        warm();
        Vec::new()
    })
}

/// The listed upload whose link has menu tag `tag`.
pub fn by_tag(tag: u32) -> Option<Record> {
    lock(&INDEX).as_ref()?.iter().find(|r| crate::tray::tag_of(&r.link) == tag).cloned()
}

fn changed() {
    crate::tray::uploads_changed();
}

/// A link as a menu shows it: "i.imgur.com/aB3x.png".
pub fn short_link(link: &str) -> String {
    link.trim_start_matches("https://").trim_start_matches("http://").to_string()
}

// --- deleting from Imgur -----------------------------------------------------------

/// DELETE (host, path, headers) → (status, body).
pub type DeleteFn<'a> = &'a dyn Fn(&str, &str, &[(&str, &str)]) -> Result<(u32, String), String>;

/// A deletehash as Imgur makes them (letters and digits): anything else
/// (a hand-edited file) never reaches a URL.
pub fn valid_deletehash(s: &str) -> bool {
    (5..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The request that deletes upload `deletehash`: host, path, headers.
pub fn delete_request(deletehash: &Secret, client_id: &str) -> (&'static str, String, Vec<(&'static str, String)>) {
    (crate::imgur::HOST, format!("/3/image/{}", deletehash.reveal()), vec![("Authorization", format!("Client-ID {client_id}"))])
}

/// How a delete went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deleted {
    Now,
    /// Imgur no longer has it (404): deleted on imgur.com, or long gone.
    AlreadyGone,
}

impl Deleted {
    /// The toast.
    pub fn text(self) -> &'static str {
        match self {
            Deleted::Now => "Deleted from Imgur",
            Deleted::AlreadyGone => "Already deleted on Imgur",
        }
    }
}

/// Delete upload `deletehash` from Imgur over `http` (no network in tests:
/// they pass a fake).
pub fn delete_remote(deletehash: &Secret, client_id: &str, http: DeleteFn) -> Result<Deleted, String> {
    if deletehash.is_empty() {
        return Err("this upload can't be deleted (it was uploaded before Rustshot kept the key)".into());
    }
    if !valid_deletehash(deletehash.reveal()) {
        return Err("this upload's key is damaged".into());
    }
    let client_id = client_id.trim();
    if client_id.is_empty() || client_id.chars().any(|c| c.is_control()) {
        return Err("no valid Imgur client id is set".into());
    }
    let (host, path, headers) = delete_request(deletehash, client_id);
    let hd: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (status, body) = http(host, &path, &hd)?;
    let ok = (200..300).contains(&status) && !body.contains("\"success\":false") && !body.contains("\"success\": false");
    if ok {
        return Ok(Deleted::Now);
    }
    if status == 404 || body.contains("\"status\":404") {
        return Ok(Deleted::AlreadyGone);
    }
    // Imgur's reason, never the request (it holds the deletehash).
    let why = json_field(&body, "error").or_else(|| json_field(&body, "message")).unwrap_or_else(|| format!("HTTP {status}"));
    Err(why.replace(deletehash.reveal(), "\u{2026}"))
}

/// Forget the upload of `link` wherever it is kept: entry `entry` of
/// `store` (its link and record cleared; an entry gone meanwhile is
/// fine) or `uploads.json`.
pub fn forget(store: &Store, entry: Option<&str>, link: &str) -> anyhow::Result<()> {
    let r = match entry {
        Some(id) if store.get(id).is_some() => store
            .update(id, |m| {
                m.uploaded = None;
                m.upload = None;
            })
            .map(|_| ()),
        Some(_) => Ok(()),
        None => Book::default_book().map_or(Ok(()), |b| b.remove(link).map(|_| ())),
    };
    index_remove(link);
    changed();
    r
}

/// "Clear history" with its checkbox: `uploads.json` goes too (the list
/// is built again).
pub fn forget_book() {
    if let Some(b) = Book::default_book() {
        let _ = std::fs::remove_file(&b.path);
    }
    *lock(&INDEX) = None;
    warm();
}

/// Deletes an S3 object (a signed `DELETE`; a fake in tests).
pub type S3DeleteFn<'a> = &'a dyn Fn(&crate::s3::Object) -> Result<(), String>;

/// Delete `rec` where it was uploaded (Imgur over `http`, an S3 bucket
/// through `s3`) and forget it; the toast's text and kind.
pub fn delete_record(store: &Store, rec: &Record, client_id: &str, http: DeleteFn, s3: S3DeleteFn) -> (String, crate::editor::style::ToastKind) {
    use crate::editor::style::ToastKind;
    let done = match &rec.info.s3 {
        Some(obj) => s3(obj).map(|()| "Deleted from S3").map_err(|e| format!("Couldn't delete the upload: {e}")),
        None => delete_remote(&rec.info.deletehash, client_id, http).map(Deleted::text).map_err(|e| format!("Couldn't delete from Imgur: {e}")),
    };
    match done {
        Ok(text) => match forget(store, rec.entry.as_deref(), &rec.link) {
            Ok(()) => (text.into(), ToastKind::Success),
            Err(e) => (format!("{text}, but the link is still listed: {e:#}"), ToastKind::Error),
        },
        Err(e) => (e, ToastKind::Error),
    }
}

/// [`delete_record`]'s S3 delete over the platform's HTTPS with the
/// configured access key (tests: never the network).
pub fn s3_delete_with(cfg: &crate::config::Config) -> impl Fn(&crate::s3::Object) -> Result<(), String> + use<> {
    let key = cfg.s3_access_key_id.clone();
    move |obj: &crate::s3::Object| crate::s3::delete_platform(obj, &key)
}

// --- the tray's Delete upload… ------------------------------------------------------

/// The upload the tray asked to delete, until its question is answered.
static ASKED: Mutex<Option<Record>> = Mutex::new(None);

/// The question for the tray's "Delete upload…": a note with Delete
/// and Cancel (None: that upload is no longer listed).
pub fn ask_delete(tag: u32) -> Option<crate::notice::Note> {
    let rec = by_tag(tag)?;
    let mut n = crate::notice::Note::plain(format!("Delete upload {}?", short_link(&rec.link)), crate::editor::style::ToastKind::Info);
    n.actions = vec![crate::notice::NoteAct::DeleteUpload, crate::notice::NoteAct::KeepUpload];
    n.ttl = std::time::Duration::from_secs(10);
    *lock(&ASKED) = Some(rec);
    Some(n)
}

/// The question's answer: Delete runs on a worker (its result goes to
/// `done`); the note while it runs.
pub fn answer(delete: bool, done: impl FnOnce(crate::notice::Note) + Send + 'static) -> Option<crate::notice::Note> {
    let rec = lock(&ASKED).take()?;
    if !delete {
        return None;
    }
    let cfg = crate::config::load();
    std::thread::spawn(move || {
        #[cfg(test)]
        let del = |_: &str, _: &str, _: &[(&str, &str)]| -> Result<(u32, String), String> { Err("no network in tests".into()) };
        #[cfg(not(test))]
        let del = |h: &str, p: &str, hd: &[(&str, &str)]| crate::export::http_delete(h, p, hd);
        let s3 = s3_delete_with(&cfg);
        let (text, kind) = delete_record(&Store::open_default(), &rec, &cfg.upload_client_id, &del, &s3);
        done(crate::notice::Note::plain(text, kind));
    });
    Some(crate::notice::Note::plain("Deleting upload\u{2026}", crate::editor::style::ToastKind::Info))
}

#[cfg(test)]
mod tests;
