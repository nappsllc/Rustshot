use super::*;
use crate::objects::Pt;
use crate::uifb::C4;

/// A fresh folder under the temp dir, removed on drop.
pub(crate) struct TempRoot(pub PathBuf);

impl TempRoot {
    pub fn new(tag: &str) -> TempRoot {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("rustshot-history-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        TempRoot(p)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn img(w: u32, h: u32, v: u8) -> PixBuf {
    PixBuf::from_pixel(w, h, [v, v / 2, 255 - v, 255])
}

fn arrow() -> Obj {
    Obj::Arrow { a: Pt::new(2.0, 2.0), b: Pt::new(30.0, 20.0), color: C4::rgb(240, 68, 56), width: 3.0 }
}

pub(crate) fn meta_at(t: Tm) -> Meta {
    Meta {
        captured: t,
        title: "#design-review - Slack".into(),
        app: "Slack".into(),
        monitor: "Built-in Display".into(),
        scale: 2.0,
        ..Meta::default()
    }
}

#[test]
fn meta_round_trips() {
    let m = Meta {
        captured: (2026, 10, 12, 9, 14, 7),
        kind: Kind::Mp4,
        w: 1180,
        h: 760,
        bytes: 486_000,
        format: "MP4".into(),
        monitor: "DELL U2720Q \"main\"".into(),
        scale: 1.5,
        title: "Stripe \u{2014} Dashboard\n".into(),
        app: "Chrome".into(),
        saved: Some(PathBuf::from(r"C:\Users\u\Pictures\rustshot\2026-10-12_09-14.png")),
        uploaded: Some("https://i.imgur.com/a8Kq2.png".into()),
        duration_ms: 42_000,
        file: Some(PathBuf::from("/home/u/Videos/rec.mp4")),
        edited: true,
        objects: vec!["Arrow".into(), "Number".into(), "Number".into()],
    };
    let back = meta_from_json(&meta_to_json(&m)).expect("parses");
    assert_eq!(back, m);
    // The defaults (no saved file, no link) round-trip too.
    let d = Meta { captured: (2025, 1, 31, 23, 59, 59), ..Meta::default() };
    assert_eq!(meta_from_json(&meta_to_json(&d)).unwrap(), d);
    // Unknown keys are ignored; a newer version or a bad date is an error.
    let extra = meta_to_json(&d).replacen('{', "{\"future\":[1,{\"x\":2}],", 1);
    assert_eq!(meta_from_json(&extra).unwrap(), d);
    assert!(meta_from_json(&meta_to_json(&d).replace("\"version\":1", "\"version\":2")).is_err());
    assert!(meta_from_json(&meta_to_json(&d).replace("2025-01-31", "2025-13-31")).is_err());
    assert!(meta_from_json(&meta_to_json(&d).replace("screenshot", "hologram")).is_err());
    assert!(meta_from_json("[]").is_err());
    assert!(meta_from_json("{").is_err());
}

#[test]
fn shot_is_written_listed_and_read_back() {
    let r = TempRoot::new("write");
    let st = Store::new(&r.0);
    let t = (2026, 10, 12, 14, 32, 7);
    let (id, dir) = st.reserve(t).unwrap();
    assert_eq!(id, "2026-10-12/143207");
    // The same second again gets its own folder.
    let (id2, _) = st.reserve(t).unwrap();
    assert_eq!(id2, "2026-10-12/143207-2");
    // Not an entry until meta.json is there.
    assert!(st.get(&id).is_none() && st.list().is_empty());
    let orig = img(400, 300, 40);
    let m = st.write_shot(&dir, &orig, &[arrow()], None, &meta_at(t)).unwrap();
    assert_eq!((m.w, m.h, m.format.as_str()), (400, 300, "PNG"));
    assert_eq!(m.objects, ["Arrow"]);
    for f in [ORIGINAL, ANNOTATIONS, THUMB, META] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    let e = st.get(&id).expect("listed");
    assert_eq!(st.list(), vec![e.clone()]);
    assert_eq!(st.original(&e).unwrap(), orig, "original.png is unannotated");
    assert_eq!(st.objects(&e).unwrap(), vec![arrow()]);
    let th = PixBuf::from_png(&std::fs::read(e.thumb()).unwrap()).unwrap();
    assert_eq!(th.dimensions(), (187, 140), "fitted into 200 x 140");
    // The thumbnail shows the annotation: some pixel differs from the plain fit.
    assert_ne!(th, crate::scale::fit(&orig, THUMB_W, THUMB_H));
    assert!(e.size > 0 && st.usage() >= e.size);
    // Ids that are not ours read nothing.
    for bad in ["../x", "2026-10-12", "2026-10-12/../../etc", "x/143207", "2026-10-12/a b"] {
        assert!(st.get(bad).is_none(), "{bad}");
    }
}

#[test]
fn edits_keep_the_original() {
    let r = TempRoot::new("edit");
    let st = Store::new(&r.0);
    let t = (2026, 10, 12, 9, 0, 0);
    let (id, dir) = st.reserve(t).unwrap();
    let orig = img(64, 48, 90);
    st.write_shot(&dir, &orig, &[], None, &meta_at(t)).unwrap();
    let before = std::fs::read(dir.join(ORIGINAL)).unwrap();
    let saved = r.0.join("out.png");
    std::fs::write(&saved, b"12345").unwrap();
    let m = st.save_edit(&id, &[arrow(), arrow()], Some((saved.clone(), 5))).unwrap();
    assert!(m.edited);
    assert_eq!(m.saved.as_deref(), Some(saved.as_path()));
    assert_eq!((m.bytes, m.objects.len()), (5, 2));
    assert_eq!(std::fs::read(dir.join(ORIGINAL)).unwrap(), before, "original.png is never modified");
    let e = st.get(&id).unwrap();
    assert_eq!(st.objects(&e).unwrap().len(), 2);
    assert!(e.meta.edited);
    let m = st.update(&id, |m| m.uploaded = Some("https://x/y".into())).unwrap();
    assert_eq!(m.uploaded.as_deref(), Some("https://x/y"));
    assert!(st.get(&id).unwrap().meta.edited, "update keeps the rest");
}

fn add(st: &Store, t: Tm, w: u32) -> String {
    let (id, dir) = st.reserve(t).unwrap();
    st.write_shot(&dir, &img(w, w, (t.2 * 7) as u8), &[], None, &meta_at(t)).unwrap();
    id
}

#[test]
fn prune_by_age() {
    let r = TempRoot::new("age");
    let st = Store::new(&r.0);
    let old = add(&st, (2026, 9, 1, 10, 0, 0), 8);
    let edge = add(&st, (2026, 9, 12, 10, 0, 0), 8); // 30 days before 12 Oct
    let kept = add(&st, (2026, 9, 13, 10, 0, 0), 8); // 29 days
    let new = add(&st, (2026, 10, 12, 10, 0, 0), 8);
    let gone = st.prune((2026, 10, 12), 30, u64::MAX, &[]);
    assert_eq!(gone.len(), 2);
    assert!(gone.contains(&old) && gone.contains(&edge));
    let left: Vec<String> = st.list().into_iter().map(|e| e.id).collect();
    assert_eq!(left, vec![new, kept], "newest first");
    // The emptied day folders go too.
    assert!(!r.0.join("2026-09-01").exists());
    // An entry in use is kept.
    let busy = add(&st, (2026, 1, 1, 0, 0, 0), 8);
    assert!(st.prune((2026, 10, 12), 30, u64::MAX, std::slice::from_ref(&busy)).is_empty());
}

#[test]
fn prune_by_size_drops_the_oldest_first() {
    let r = TempRoot::new("size");
    let st = Store::new(&r.0);
    let ids: Vec<String> = (1..=4).map(|d| add(&st, (2026, 10, d, 12, 0, 0), 64)).collect();
    let each = st.get(&ids[0]).unwrap().size;
    assert!(each > 0);
    let all: u64 = st.list().iter().map(|e| e.size).sum();
    // Room for a bit more than two entries.
    let gone = st.prune((2026, 10, 4), 365, all - each - each / 2, &[]);
    assert_eq!(gone, vec![ids[0].clone(), ids[1].clone()], "oldest first");
    assert_eq!(st.list().len(), 2);
    // Within the cap nothing goes.
    assert!(st.prune((2026, 10, 4), 365, u64::MAX, &[]).is_empty());
}

#[test]
fn delete_and_clear() {
    let r = TempRoot::new("clear");
    let st = Store::new(&r.0);
    let a = add(&st, (2026, 10, 1, 1, 0, 0), 8);
    let saved = r.0.join("saved.png");
    std::fs::write(&saved, b"png").unwrap();
    st.update(&a, |m| {
        m.saved = Some(saved.clone());
        m.bytes = 3;
    })
    .unwrap();
    let b = add(&st, (2026, 10, 2, 1, 0, 0), 8);
    let (n, bytes, dir) = saved_files(&st.list());
    assert_eq!((n, bytes, dir.as_deref()), (1, 3, Some(r.0.as_path())));
    st.delete(&b, false).unwrap();
    assert!(st.get(&b).is_none());
    assert_eq!(st.clear(false), 1);
    assert!(st.list().is_empty());
    assert!(saved.exists(), "saved files stay unless asked");
    let c = add(&st, (2026, 10, 3, 1, 0, 0), 8);
    st.update(&c, |m| {
        m.saved = Some(saved.clone());
        m.bytes = 3;
    })
    .unwrap();
    // A half-written folder (no meta.json) goes with a clear too.
    st.reserve((2026, 10, 3, 2, 0, 0)).unwrap();
    assert_eq!(st.clear(true), 1);
    assert!(!saved.exists(), "Also delete saved files");
    assert_eq!(std::fs::read_dir(&r.0).unwrap().count(), 0);
}

#[test]
fn recordings_keep_their_file_and_poster() {
    let r = TempRoot::new("rec");
    let st = Store::new(&r.0);
    let file = r.0.join("rec.mp4");
    std::fs::create_dir_all(&r.0).unwrap();
    std::fs::write(&file, vec![0u8; 1234]).unwrap();
    let (id, dir) = st.reserve((2026, 10, 12, 13, 5, 0)).unwrap();
    let meta = Meta { kind: Kind::Mp4, file: Some(file.clone()), duration_ms: 42_000, w: 1920, h: 1080, ..meta_at((2026, 10, 12, 13, 5, 0)) };
    st.write_recording(&dir, Some(&img(1920, 1080, 10)), &meta).unwrap();
    let e = st.get(&id).unwrap();
    assert_eq!((e.meta.bytes, e.meta.format.as_str(), e.meta.kind), (1234, "MP4", Kind::Mp4));
    assert!(e.thumb().is_file() && !e.original().exists());
    st.delete(&id, false).unwrap();
    assert!(file.exists(), "pruning never removes the video");
}

#[test]
fn the_writer_thread_writes_in_order() {
    let r = TempRoot::new("worker");
    let cfg = crate::config::Config::default();
    let t = (2026, 10, 12, 8, 1, 2);
    let id = record(&cfg, &r.0, ShotRec { original: img(32, 32, 1), objects: vec![arrow()], meta: meta_at(t) }).expect("on by default");
    update(&r.0, &id, |m| m.uploaded = Some("https://u".into()));
    flush(std::time::Duration::from_secs(20));
    let e = Store::new(&r.0).get(&id).expect("written");
    assert_eq!(e.meta.uploaded.as_deref(), Some("https://u"));
    assert_eq!(today_count(&r.0, (2026, 10, 12)), 1);
    assert_eq!(today_count(&r.0, (2026, 10, 13)), 0);
    // History off: nothing is kept.
    let off = crate::config::Config { history: false, ..cfg };
    assert!(record(&off, &r.0, ShotRec { original: img(4, 4, 1), objects: vec![], meta: meta_at(t) }).is_none());
}

/// The daemon prunes without new captures and with History off (the
/// captures kept before still expire).
#[test]
fn pruning_runs_with_history_off() {
    let r = TempRoot::new("off");
    let st = Store::new(&r.0);
    // Its own id: what is being written or edited is kept by id in this
    // process, and pruning_leaves_open_entries_alone holds
    // 2020-01-01/100000 open at the same time (in another folder).
    let old = add(&st, (2020, 1, 3, 10, 0, 0), 8);
    let cfg = crate::config::Config { history: false, ..crate::config::Config::default() };
    keep_pruned(&r.0, &cfg);
    flush(std::time::Duration::from_secs(20));
    assert!(st.get(&old).is_none(), "expired");
}

/// A saved screenshot's file and a recording's video survive pruning
/// by age and by size.
#[test]
fn pruning_keeps_saved_files() {
    let r = TempRoot::new("keepsaved");
    let st = Store::new(&r.0);
    let saved = r.0.join("shot.png");
    let a = add(&st, (2026, 9, 1, 10, 0, 0), 8);
    std::fs::write(&saved, b"png!").unwrap();
    st.update(&a, |m| {
        m.saved = Some(saved.clone());
        m.bytes = 4;
    })
    .unwrap();
    let b = add(&st, (2026, 10, 11, 10, 0, 0), 64);
    let saved_b = r.0.join("shot-b.png");
    std::fs::write(&saved_b, b"png?").unwrap();
    st.update(&b, |m| {
        m.saved = Some(saved_b.clone());
        m.bytes = 4;
    })
    .unwrap();
    let _c = add(&st, (2026, 10, 12, 10, 0, 0), 64);
    assert_eq!(st.prune((2026, 10, 12), 30, u64::MAX, &[]), vec![a]);
    assert_eq!(st.prune((2026, 10, 12), 30, 1, &[]).first(), Some(&b));
    assert!(saved.is_file() && saved_b.is_file(), "saved files stay");
}

/// Folders without a valid meta.json (a crash mid-write, a corrupt or
/// newer meta.json) and stray temporary files count toward the cap and
/// go once they are an hour old; younger ones stay.
#[test]
fn pruning_sweeps_orphans_and_temporary_files() {
    let r = TempRoot::new("orphans");
    let st = Store::new(&r.0);
    let good = add(&st, (2026, 10, 12, 9, 0, 0), 8);
    let (half, hdir) = st.reserve((2026, 10, 12, 9, 1, 0)).unwrap();
    std::fs::write(hdir.join(ORIGINAL), vec![7u8; 50_000]).unwrap();
    let (bad, bdir) = st.reserve((2026, 10, 12, 9, 2, 0)).unwrap();
    std::fs::write(bdir.join(META), "{\"version\":1,\"kind\":\"screen").unwrap();
    let gdir = st.get(&good).unwrap().dir;
    let tmp = gdir.join("meta.json.1-2.rustshot.tmp");
    std::fs::write(&tmp, b"half").unwrap();
    assert_eq!(st.list().len(), 1, "orphans are not entries");
    let now = std::time::SystemTime::now();
    // Young: kept, but their bytes count (the entry goes for the cap).
    let gone = st.prune_at((2026, 10, 12), 30, 40_000, &[], now);
    assert_eq!(gone, vec![good.clone()], "50 KB of orphans fill the 40 KB cap");
    assert!(hdir.is_dir() && bdir.is_dir());
    // An hour on: they go (and in use, one stays).
    let later = now + GRACE + std::time::Duration::from_secs(60);
    let gone = st.prune_at((2026, 10, 12), 30, u64::MAX, std::slice::from_ref(&bad), later);
    assert_eq!(gone, vec![half]);
    assert!(!hdir.exists() && bdir.is_dir());
    let gone = st.prune_at((2026, 10, 12), 30, u64::MAX, &[], later);
    assert_eq!(gone, vec![bad]);
    assert!(std::fs::read_dir(&r.0).unwrap().count() == 0, "the day folder goes too");
    // A temporary file in a live entry goes after the hour; the entry stays.
    let good = add(&st, (2026, 10, 12, 9, 0, 0), 8);
    let gdir = st.get(&good).unwrap().dir;
    let tmp = gdir.join("annotations.json.1-3.rustshot.tmp");
    std::fs::write(&tmp, b"half").unwrap();
    assert!(st.prune_at((2026, 10, 12), 30, u64::MAX, &[], now).is_empty());
    assert!(tmp.exists(), "young");
    assert!(st.prune_at((2026, 10, 12), 30, u64::MAX, &[], later).is_empty());
    assert!(!tmp.exists() && st.get(&good).is_some());
}

/// An entry open in an Edit window is never pruned (in this process
/// through the keep list, in any through its held `.open` file).
#[test]
fn pruning_leaves_open_entries_alone() {
    let r = TempRoot::new("open");
    let st = Store::new(&r.0);
    let id = add(&st, (2020, 1, 1, 10, 0, 0), 8);
    let dir = st.get(&id).unwrap().dir;
    let held = hold_open(&id, &dir);
    assert!(keep_now().contains(&id));
    assert!(st.prune((2026, 10, 12), 30, 1, &[]).is_empty(), "held open (the file lock)");
    assert!(st.get(&id).is_some());
    drop(held);
    assert!(!keep_now().contains(&id));
    assert_eq!(st.prune((2026, 10, 12), 30, u64::MAX, &[]), vec![id]);
}

/// "Also delete saved files" deletes only regular files of a capture's
/// format that are as big as recorded, by absolute path.
#[test]
fn deleting_saved_files_checks_them_first() {
    let r = TempRoot::new("deletable");
    let st = Store::new(&r.0);
    std::fs::create_dir_all(&r.0).unwrap();
    let file = |name: &str, bytes: &[u8]| {
        let p = r.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    };
    let ok = file("ok.png", b"1234");
    let resized = file("resized.png", b"123456");
    let text = file("notes.txt", b"1234");
    let folder = r.0.join("folder.png");
    std::fs::create_dir_all(&folder).unwrap();
    let mut ids = Vec::new();
    for (i, p) in [ok.clone(), resized.clone(), text.clone(), folder.clone(), PathBuf::from("ok.png")].into_iter().enumerate() {
        let id = add(&st, (2026, 10, 1, 1, i as u32, 0), 8);
        st.update(&id, |m| {
            m.saved = Some(p);
            m.bytes = 4;
        })
        .unwrap();
        ids.push(id);
    }
    let (n, bytes, _) = saved_files(&st.list());
    assert_eq!((n, bytes), (1, 4), "only the one that would be deleted");
    for id in &ids {
        st.delete(id, true).unwrap();
    }
    assert!(!ok.exists(), "deleted");
    assert!(resized.exists() && text.exists() && folder.is_dir(), "changed since, not a capture, a folder");
}

/// A symbolic link in the history is never followed: its target is not
/// listed, pruned or cleared.
#[test]
fn links_are_never_followed() {
    let r = TempRoot::new("links");
    let out = TempRoot::new("links-out");
    let other = Store::new(&out.0);
    let victim = add(&other, (2020, 1, 1, 10, 0, 0), 8);
    let vdir = other.get(&victim).unwrap().dir;
    std::fs::create_dir_all(r.0.join("2020-01-02")).unwrap();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(out.0.join("2020-01-01"), r.0.join("2020-01-01")).is_ok()
        && std::os::windows::fs::symlink_dir(&vdir, r.0.join("2020-01-02").join("100000")).is_ok();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(out.0.join("2020-01-01"), r.0.join("2020-01-01")).is_ok()
        && std::os::unix::fs::symlink(&vdir, r.0.join("2020-01-02").join("100000")).is_ok();
    if !made {
        eprintln!("no symbolic links here (Windows without Developer Mode): skipped");
        return;
    }
    let st = Store::new(&r.0);
    assert!(st.list().is_empty());
    assert!(st.prune((2026, 10, 12), 1, 0, &[]).is_empty());
    st.clear(true);
    assert!(other.get(&victim).is_some(), "the target is untouched");
}

/// Many threads changing one entry at once (the upload link, Edited, the
/// objects) lose nothing, and meta.json stays valid throughout.
#[test]
fn concurrent_updates_all_land() {
    let r = TempRoot::new("concurrent");
    let st = Store::new(&r.0);
    let id = add(&st, (2026, 10, 12, 10, 0, 0), 8);
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let (st, id) = (st.clone(), id.clone());
            std::thread::spawn(move || {
                for i in 0..15 {
                    st.update(&id, |m| {
                        m.objects.push(format!("t{t}-{i}"));
                        if t % 2 == 0 {
                            m.uploaded = Some(format!("https://u/{t}"));
                        } else {
                            m.edited = true;
                        }
                    })
                    .unwrap();
                    // A reader never sees half a file.
                    assert!(st.get(&id).is_some());
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let e = st.get(&id).unwrap();
    assert_eq!(e.meta.objects.len(), 120, "no update lost");
    assert!(e.meta.edited && e.meta.uploaded.is_some(), "both kinds of change survive");
    let leftovers = std::fs::read_dir(&e.dir).unwrap().flatten().filter(|f| f.file_name().to_string_lossy().ends_with(".tmp")).count();
    assert_eq!(leftovers, 0);
}

#[test]
fn titles_name_the_app_and_window() {
    let m = |title: &str, app: &str| Meta { title: title.into(), app: app.into(), ..Meta::default() };
    assert_eq!(title(&m("#design-review - Slack", "Slack")), "Slack \u{2014} #design-review");
    assert_eq!(title(&m("Stripe Dashboard - Google Chrome", "Chrome")), "Chrome \u{2014} Stripe Dashboard");
    assert_eq!(title(&m("overlay.rs - rustshot - Visual Studio Code", "Code")), "Code \u{2014} overlay.rs - rustshot");
    assert_eq!(title(&m("Landing page", "Figma")), "Figma \u{2014} Landing page");
    assert_eq!(title(&m("Figma", "Figma")), "Figma");
    assert_eq!(title(&m("Untitled", "")), "Untitled");
    assert_eq!(title(&Meta::default()), "Screenshot");
    assert_eq!(title(&Meta { kind: Kind::Gif, ..Meta::default() }), "Recording");
    let saved = Meta { saved: Some(PathBuf::from("dir/2026-10-12_09-14.png")), ..Meta::default() };
    assert_eq!(title(&saved), "2026-10-12_09-14.png");
}

#[test]
fn search_matches_title_app_file_and_date() {
    let today = (2026, 10, 12);
    let e = Meta {
        captured: (2026, 10, 8, 16, 40, 0),
        title: "Stripe Dashboard - Google Chrome".into(),
        app: "Chrome".into(),
        saved: Some(PathBuf::from("/p/2026-10-08_16-40.png")),
        ..Meta::default()
    };
    // "Chrome — Stripe Dashboard": the match is highlighted.
    let shown = title(&e);
    let hits = search(&e, "stripe", today).expect("matches");
    assert_eq!(hits.len(), 1);
    assert_eq!(&shown[hits[0].clone()], "Stripe");
    assert_eq!(search(&e, "STRIPE dash", today).unwrap().len(), 2, "every word, any case");
    assert!(search(&e, "stripe slack", today).is_none(), "all words must match");
    assert_eq!(search(&e, "", today), Some(vec![]));
    // The app, the file name and the date match without a highlight.
    assert_eq!(search(&e, "google", today), Some(vec![]));
    assert!(search(&e, "16-40.png", today).is_some());
    assert!(search(&e, "2026-10-08", today).is_some());
    assert!(search(&e, "thu", today).is_some(), "Thu 8 Oct");
    assert!(search(&e, "oct", today).is_some());
    assert!(search(&e, "8 oct", today).is_some());
    assert!(search(&e, "png", today).is_some(), "the type");
    assert!(search(&e, "yesterday", today).is_none());
    // Non-ASCII titles: ranges stay on char boundaries.
    let u = Meta { title: "Grüße — ÄÖÜ".into(), ..Meta::default() };
    let shown = title(&u);
    for r in search(&u, "äö", today).unwrap() {
        assert_eq!(&shown[r], "ÄÖ");
    }
    // Overlapping words merge into one highlight.
    let hits = search(&e, "strip ripe", today).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(&title(&e)[hits[0].clone()], "Stripe");
    assert!(Filter::Recordings.keeps(Kind::Gif) && !Filter::Recordings.keeps(Kind::Shot));
    assert!(Filter::Shots.keeps(Kind::Shot) && !Filter::Shots.keeps(Kind::Mp4) && Filter::All.keeps(Kind::Mp4));
}

#[test]
fn group_labels() {
    let today = (2026, 10, 12); // a Monday
    assert_eq!(day_header(today, today, 4), ("Today".into(), Some("\u{b7} 4 captures".into())));
    assert_eq!(day_header(today, today, 1).1.as_deref(), Some("\u{b7} 1 capture"));
    assert_eq!(day_header((2026, 10, 11), today, 9), ("Yesterday".into(), Some("\u{b7} Sun 11 Oct".into())));
    assert_eq!(day_header((2026, 10, 9), today, 2), ("Fri 9 Oct".into(), None));
    assert_eq!(day_header((2025, 12, 31), today, 2), ("Wed 31 Dec 2025".into(), None));
    // Across a month and a year.
    assert_eq!(day_header((2026, 9, 30), (2026, 10, 1), 1).0, "Yesterday");
    assert_eq!(day_header((2025, 12, 31), (2026, 1, 1), 1).0, "Yesterday");
    assert_eq!(results_header(3, " stripe "), "3 results for \"stripe\"");
    assert_eq!(results_header(1, "x"), "1 result for \"x\"");
    let t = (2026, 10, 8, 16, 40, 5);
    assert_eq!(time_text(&t, today, false), "16:40");
    assert_eq!(time_text(&t, today, true), "Thu 8 Oct 16:40");
    assert_eq!(time_text(&(2026, 10, 12, 11, 48, 0), today, true), "Today 11:48");
    assert_eq!(captured_text(&(2026, 10, 12, 9, 14, 7), today), "Today, 09:14:07");
    assert_eq!(captured_text(&t, today), "Thu 8 Oct, 16:40:05");
    assert_eq!(size_text(0), "0 B");
    assert_eq!(size_text(486_000), "486 KB");
    assert_eq!(size_text(1_200_000_000), "1.2 GB");
    assert_eq!(size_text(2_000_000_000), "2.0 GB");
    assert_eq!(size_text(3_400_000), "3.4 MB");
    assert_eq!(duration_text(42_000), "00:42");
    assert_eq!(duration_text(3_723_000), "1:02:03");
    assert_eq!(object_chips(&["Arrow".into(), "Callout".into(), "Number".into(), "Number".into()]), ["Arrow", "Callout", "Number \u{d7}2"]);
}

#[test]
fn civil_dates_round_trip() {
    for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 10, 12), (1969, 12, 31), (2100, 3, 1)] {
        assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
    }
    assert_eq!(days_from_civil(1970, 1, 2), 1);
    assert_eq!(short_date(2026, 10, 12, 2026), "Mon 12 Oct");
}


#[test]
fn the_root_is_outside_the_program_folder() {
    let base = Path::new(if cfg!(windows) { r"C:\Users\u\AppData\Local" } else { "/home/u/.local/share" });
    let root = root_in(base);
    assert!(root.ends_with("history"));
    if cfg!(windows) {
        // %LOCALAPPDATA%\rustshot is where the installer puts rustshot.exe.
        assert_eq!(root, base.join("rustshot-data").join("history"));
        assert!(!root.starts_with(base.join("rustshot")));
    } else {
        assert_eq!(root, base.join("rustshot").join("history"));
    }
}

#[test]
fn list_lines_name_the_capture_and_its_file() {
    let shot = Entry {
        id: "2026-10-12/091407".into(),
        dir: PathBuf::from("h").join("2026-10-12").join("091407"),
        meta: Meta {
            captured: (2026, 10, 12, 9, 14, 7),
            w: 400,
            h: 300,
            bytes: 12_345,
            saved: Some(PathBuf::from("shot.png")),
            ..Meta::default()
        },
        size: 0,
    };
    assert_eq!(list_line(&shot), "2026-10-12 09:14:07\tscreenshot\t400x300\t12345\tshot.png");
    // Never saved: its original.png; a recording: its file.
    let plain = Entry { meta: Meta { saved: None, ..shot.meta.clone() }, ..shot.clone() };
    assert!(list_line(&plain).ends_with(&format!("\t{}", shot.dir.join("original.png").display())));
    let rec = Entry { meta: Meta { kind: Kind::Mp4, saved: None, file: Some(PathBuf::from("rec.mp4")), ..shot.meta.clone() }, ..shot.clone() };
    assert!(list_line(&rec).contains("\tmp4\t") && list_line(&rec).ends_with("\trec.mp4"));
}
