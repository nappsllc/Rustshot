use super::*;
use crate::history::tests::{meta_at, TempRoot};
use std::cell::RefCell;

/// The tests that touch the tray's index: one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn base64_round_trips() {
    assert_eq!(b64_encode(b"Man"), "TWFu");
    assert_eq!(b64_encode(b"Ma"), "TWE=");
    assert_eq!(b64_encode(b"M"), "TQ==");
    assert_eq!(b64_encode(b""), "");
    for n in 0..70u8 {
        let data: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37).wrapping_add(n)).collect();
        assert_eq!(b64_decode(&b64_encode(&data)).as_deref(), Some(&data[..]), "{n} bytes");
    }
    assert_eq!(b64_decode("not base64!"), None);
}

/// The deletehash never shows in a `Debug` print (logs, test failures).
#[test]
fn the_deletehash_is_never_printed() {
    let s = Secret::new("s3cr3tHash");
    assert_eq!(format!("{s:?}"), "Secret(hidden)");
    let info = Info { deletehash: s.clone(), time: 1, kind: UpKind::Image, thumb: None, s3: None };
    let rec = Record { link: "https://i.imgur.com/a.png".into(), info, entry: None };
    assert!(!format!("{rec:?}").contains("s3cr3tHash"));
    assert_eq!(format!("{:?}", Secret::default()), "Secret(none)");
    assert_eq!(s.reveal(), "s3cr3tHash");
    // An error message quoting the reply loses it too.
    let body = r#"{"data":{"id":"a","deletehash":"s3cr3tHash","link":"x"},"success":false}"#;
    assert!(!redact(body).contains("s3cr3tHash"));
}

#[test]
fn replies_give_the_link_and_deletehash() {
    let body = r#"{"data":{"id":"aB3x","deletehash":"Kq2dH","link":"https:\/\/i.imgur.com\/aB3x.png"},"success":true,"status":200}"#;
    let (link, dh) = parse_reply(body).unwrap();
    assert_eq!(link, "https://i.imgur.com/aB3x.png");
    assert_eq!(dh, Secret::new("Kq2dH"));
    assert_eq!(parse_reply(r#"{"data":{}}"#), None);
    // No deletehash (an odd reply): an empty one, the link still kept.
    assert_eq!(parse_reply(r#"{"data":{"link":"https://i.imgur.com/q.png"}}"#).unwrap().1, Secret::default());
}

/// What the upload code noted (deletehash, kind, thumbnail) becomes the
/// record once the upload is filed, once.
#[test]
fn notes_become_the_record() {
    let link = "https://i.imgur.com/notes1.mp4";
    note_reply(&format!(r#"{{"data":{{"deletehash":"dhV","link":"{link}"}}}}"#), UpKind::Video);
    note_thumb(link, Some(vec![1, 2, 3]));
    let info = info_for(link, || panic!("the noted thumbnail is used"));
    assert_eq!((info.deletehash.reveal(), info.kind, info.thumb.as_deref()), ("dhV", UpKind::Video, Some(&[1u8, 2, 3][..])));
    assert!(info.time > 1_700_000_000);
    // Taken: a second filing knows nothing (a guess from the link).
    let again = info_for(link, || None);
    assert!(again.deletehash.is_empty());
    assert_eq!(again.kind, UpKind::Video);
    // (A link no other test uploads: the notes are the process's.)
    assert_eq!(info_for("https://i.imgur.com/notes-unique-9.png", || Some(vec![9])).thumb, Some(vec![9]));
}

#[test]
fn thumbnails_are_72_by_48() {
    let wide = PixBuf::from_pixel(400, 100, [10, 20, 30, 255]);
    let t = thumb_of(&wide);
    assert_eq!(t.dimensions(), (THUMB_W, THUMB_H));
    assert_eq!(&t.as_raw()[..4], &[10, 20, 30, 255]);
    let tall = PixBuf::from_pixel(50, 900, [200, 0, 0, 255]);
    assert_eq!(thumb_of(&tall).dimensions(), (72, 48));
    let png = thumb_png(&wide.to_png().unwrap()).unwrap();
    assert_eq!(PixBuf::from_png(&png).unwrap().dimensions(), (72, 48));
    assert_eq!(thumb_png(b"junk"), None);
}

fn rec(link: &str, dh: &str, time: u64) -> Record {
    Record { link: link.into(), info: Info { deletehash: Secret::new(dh), time, kind: UpKind::Image, thumb: Some(vec![7, 8]), s3: None }, entry: None }
}

/// uploads.json: records round-trip (thumbnail included), the newest
/// first, at most KEPT, a link once; a removed one goes.
#[test]
fn the_book_keeps_records() {
    let a = rec("https://i.imgur.com/a.png", "dA", 10);
    let b = Record { info: Info { kind: UpKind::Video, thumb: None, ..rec("", "dB", 20).info }, ..rec("https://i.imgur.com/b.mp4", "", 0) };
    let text = book_to_json(&[a.clone(), b.clone()]);
    assert_eq!(book_from_json(&text), vec![a.clone(), b.clone()]);
    assert_eq!(book_from_json("{oops"), Vec::new());
    assert_eq!(book_from_json(r#"{"version":1,"uploads":[{"deletehash":"x"},{"link":"https://i.imgur.com/c.png"}]}"#).len(), 1, "a record needs its link");

    let root = TempRoot::new("uploads-book");
    let book = Book::at(root.0.join("uploads.json"));
    assert!(book.list().is_empty());
    book.add(a.clone()).unwrap();
    book.add(b.clone()).unwrap();
    assert_eq!(book.list(), vec![b.clone(), a.clone()], "newest first");
    book.add(a.clone()).unwrap();
    assert_eq!(book.list(), vec![a.clone(), b.clone()], "a link once");
    for i in 0..KEPT + 5 {
        book.add(rec(&format!("https://i.imgur.com/{i}.png"), "d", i as u64)).unwrap();
    }
    assert_eq!(book.list().len(), KEPT);
    assert_eq!(book.list()[0].link, format!("https://i.imgur.com/{}.png", KEPT + 4));
    let top = book.list()[0].link.clone();
    assert!(book.remove(&top).unwrap());
    assert!(!book.remove(&top).unwrap());
    assert_eq!(book.list().len(), KEPT - 1);
    // The file itself holds the deletehash (kept on this computer only).
    assert!(std::fs::read_to_string(&book.path).unwrap().contains("\"deletehash\":\"d\""));
}

#[test]
fn recent_uploads_merge_history_and_the_book() {
    let entry = |id: &str, link: Option<&str>, time: u64| crate::history::Entry {
        id: id.into(),
        dir: PathBuf::from(id),
        meta: Meta {
            uploaded: link.map(String::from),
            upload: link.map(|_| Info { deletehash: Secret::new("h"), time, kind: UpKind::Image, thumb: None, s3: None }),
            ..meta_at((2026, 10, 12, 9, 0, 0))
        },
        size: 0,
    };
    let entries = vec![entry("2026-10-12/090000", Some("https://i.imgur.com/e1.png"), 300), entry("2026-10-12/090001", None, 0), entry("2026-10-11/090000", Some("https://i.imgur.com/e2.png"), 100)];
    let book = vec![rec("https://i.imgur.com/b1.png", "x", 200), rec("https://i.imgur.com/e1.png", "dup", 50), rec("https://i.imgur.com/b0.png", "x", 1)];
    let r = merge_recent(&entries, book);
    let links: Vec<&str> = r.iter().map(|r| r.link.as_str()).collect();
    assert_eq!(links, ["https://i.imgur.com/e1.png", "https://i.imgur.com/b1.png", "https://i.imgur.com/e2.png", "https://i.imgur.com/b0.png"]);
    assert_eq!(r[0].entry.as_deref(), Some("2026-10-12/090000"));
    assert_eq!(r[1].entry, None);
    let many: Vec<Record> = (0..9).map(|i| rec(&format!("https://i.imgur.com/m{i}.png"), "x", i)).collect();
    assert_eq!(merge_recent(&[], many).len(), RECENT);
    assert_eq!(short_link("https://i.imgur.com/aB3x.png"), "i.imgur.com/aB3x.png");
}

/// The delete request: DELETE /3/image/{deletehash} at api.imgur.com with
/// the app's Client-ID (a fake transport: nothing is sent).
#[test]
fn delete_requests_are_built_and_answered() {
    let dh = Secret::new("Kq2dH");
    let (host, path, headers) = delete_request(&dh, "cid42");
    assert_eq!((host, path.as_str()), ("api.imgur.com", "/3/image/Kq2dH"));
    assert_eq!(headers, vec![("Authorization", "Client-ID cid42".to_string())]);

    let sent = RefCell::new(Vec::new());
    let ok = |h: &str, p: &str, hd: &[(&str, &str)]| {
        sent.borrow_mut().push((h.to_string(), p.to_string(), hd.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>()));
        Ok((200, r#"{"data":true,"success":true,"status":200}"#.to_string()))
    };
    assert_eq!(delete_remote(&dh, " cid42 ", &ok), Ok(Deleted::Now));
    assert_eq!(sent.borrow()[0], ("api.imgur.com".to_string(), "/3/image/Kq2dH".to_string(), vec!["Authorization: Client-ID cid42".to_string()]));

    // 404: Imgur no longer has it (deleted there already): gone, not a failure.
    let gone = |_: &str, _: &str, _: &[(&str, &str)]| {
        Ok((404, r#"{"data":{"error":"Unable to find an image with the hash Kq2dH","request":"\/3\/image\/Kq2dH","method":"DELETE"},"success":false,"status":404}"#.to_string()))
    };
    assert_eq!(delete_remote(&dh, "cid", &gone), Ok(Deleted::AlreadyGone));
    assert_eq!(Deleted::AlreadyGone.text(), "Already deleted on Imgur");
    // Imgur's reason, without the deletehash it may quote.
    let denied = |_: &str, _: &str, _: &[(&str, &str)]| Ok((403, r#"{"data":{"error":"Not allowed for Kq2dH"},"success":false,"status":403}"#.to_string()));
    let e = delete_remote(&dh, "cid", &denied).unwrap_err();
    assert!(e.starts_with("Not allowed") && !e.contains("Kq2dH"), "{e}");
    let limited = |_: &str, _: &str, _: &[(&str, &str)]| Ok((429, "Too Many Requests".to_string()));
    assert_eq!(delete_remote(&dh, "cid", &limited), Err("HTTP 429".into()));
    let offline = |_: &str, _: &str, _: &[(&str, &str)]| -> Result<(u32, String), String> { Err("WinHttpSendRequest: no network".into()) };
    assert_eq!(delete_remote(&dh, "cid", &offline), Err("WinHttpSendRequest: no network".into()));
    // Nothing to send without a deletehash or a client id.
    let never = |_: &str, _: &str, _: &[(&str, &str)]| -> Result<(u32, String), String> { panic!("sent") };
    assert!(delete_remote(&Secret::default(), "cid", &never).is_err());
    assert!(delete_remote(&dh, "  ", &never).is_err());
    assert!(delete_remote(&dh, "cid\nX-Evil: 1", &never).is_err());
    let long = "a".repeat(33);
    for bad in ["abc", "Kq2dH/../x", "Kq2dH?x=1", "Kq 2dH", long.as_str()] {
        assert!(!valid_deletehash(bad), "{bad}");
        assert!(delete_remote(&Secret::new(bad), "cid", &never).is_err(), "{bad}");
    }
    assert!(valid_deletehash("Kq2dH") && valid_deletehash("aB3xYz9QwErTyUi"));
}

/// The tray's list comes from the index (never a listing on the caller's
/// thread); filed and forgotten uploads update it; Delete upload…
/// is asked, then answered.
#[test]
fn the_index_follows_uploads_and_the_tray_asks() {
    let _l = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *lock(&INDEX) = Some(vec![rec("https://i.imgur.com/old1.png", "dOld1", 1)]);
    index_add(rec("https://i.imgur.com/idx1.png", "dIdx1", 5));
    let links: Vec<String> = recent().into_iter().map(|r| r.link).collect();
    assert_eq!(links, ["https://i.imgur.com/idx1.png", "https://i.imgur.com/old1.png"]);
    let t = crate::tray::tag_of("https://i.imgur.com/idx1.png");
    assert_eq!(by_tag(t).unwrap().info.deletehash.reveal(), "dIdx1");
    assert_eq!(by_tag(12345), None);
    // The question, then Cancel: nothing happens.
    let n = ask_delete(t).unwrap();
    assert_eq!(n.text, "Delete upload i.imgur.com/idx1.png?");
    assert_eq!(n.actions, vec![crate::notice::NoteAct::DeleteUpload, crate::notice::NoteAct::KeepUpload]);
    assert!(answer(false, |_| panic!("no delete")).is_none());
    assert!(answer(true, |_| panic!("asked once")).is_none(), "answered already");
    index_remove("https://i.imgur.com/idx1.png");
    assert_eq!(recent().len(), 1);
    assert!(ask_delete(t).is_none());
    for i in 0..9 {
        index_add(rec(&format!("https://i.imgur.com/many{i}.png"), "dMany1", i));
    }
    assert_eq!(recent().len(), RECENT);
}

/// An upload of a History entry files its record in the entry's meta (a
/// thumbnail from the entry's own when the upload had none); forgetting
/// it clears the link and the record.
#[test]
fn entries_keep_their_upload() {
    let _l = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = TempRoot::new("uploads-entry");
    let st = Store::new(&root.0);
    let (id, dir) = st.reserve((2026, 10, 12, 9, 14, 7)).unwrap();
    let pic = PixBuf::from_pixel(300, 200, [40, 80, 120, 255]);
    st.write_shot(&dir, &pic, &[], None, &meta_at((2026, 10, 12, 9, 14, 7))).unwrap();
    let link = "https://i.imgur.com/entry1.png";
    note_reply(&format!(r#"{{"data":{{"deletehash":"dhE","link":"{link}"}}}}"#), UpKind::Image);
    let m = finished_in(&st, &id, link).unwrap();
    assert_eq!(m.uploaded.as_deref(), Some(link));
    let info = m.upload.clone().unwrap();
    assert_eq!(info.deletehash.reveal(), "dhE");
    let t = PixBuf::from_png(info.thumb.as_deref().unwrap()).unwrap();
    assert_eq!(t.dimensions(), (THUMB_W, THUMB_H));
    // On disk, read back the same.
    assert_eq!(st.get(&id).unwrap().meta.upload, Some(info));
    forget(&st, Some(&id), link).unwrap();
    let m = st.get(&id).unwrap().meta;
    assert_eq!((m.uploaded, m.upload), (None, None));
}

fn s3_obj(key: &str) -> crate::s3::Object {
    crate::s3::Object {
        target: crate::s3::Target { endpoint: "https://acc.r2.cloudflarestorage.com".into(), region: "auto".into(), bucket: "shots".into(), path_style: false },
        key: key.into(),
    }
}

/// An S3 upload's record keeps where the object is (destination "s3"),
/// in uploads.json and in a History entry's meta.json alike.
#[test]
fn s3_records_keep_the_object() {
    let r = Record { info: Info { s3: Some(s3_obj("2026/10/12/a b-x1.png")), ..rec("", "", 30).info }, ..rec("https://cdn.example.com/2026/10/12/a%20b-x1.png", "", 0) };
    assert_eq!((r.info.dest(), r.info.deletable()), ("s3", true));
    let text = book_to_json(std::slice::from_ref(&r));
    assert!(text.contains("\"dest\":\"s3\"") && text.contains("\"s3_key\":\"2026/10/12/a b-x1.png\"") && text.contains("\"s3_path_style\":\"0\""), "{text}");
    assert_eq!(book_from_json(&text), vec![r.clone()]);
    assert_eq!(rec("l", "dA1b2", 1).info.dest(), "imgur");
    assert!(!rec("l", "", 1).info.deletable(), "an Imgur upload without its key");

    let m = Meta { uploaded: Some(r.link.clone()), upload: Some(r.info.clone()), ..meta_at((2026, 10, 12, 9, 0, 0)) };
    let json = crate::history::meta_to_json(&m);
    assert!(json.contains("\"upload_dest\":\"s3\"") && json.contains("\"upload_s3_bucket\":\"shots\""), "{json}");
    assert_eq!(crate::history::meta_from_json(&json).unwrap().upload, Some(r.info));
}

/// Delete upload… on an S3 record sends the signed DELETE (`s3`), never
/// Imgur's; a failure says why and keeps the record.
#[test]
fn deleting_an_s3_upload_uses_the_bucket() {
    let _l = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = TempRoot::new("uploads-s3-del");
    let st = Store::new(&root.0);
    let (id, dir) = st.reserve((2026, 10, 12, 9, 14, 8)).unwrap();
    let pic = PixBuf::from_pixel(30, 20, [40, 80, 120, 255]);
    st.write_shot(&dir, &pic, &[], None, &meta_at((2026, 10, 12, 9, 14, 8))).unwrap();
    let link = "https://cdn.example.com/k-1.png";
    note_s3(link, s3_obj("k-1.png"), UpKind::Image);
    let m = finished_in(&st, &id, link).unwrap();
    let rec = Record { link: link.into(), info: m.upload.clone().unwrap(), entry: Some(id.clone()) };
    assert_eq!(rec.info.s3.as_ref().map(|o| o.key.as_str()), Some("k-1.png"));
    let imgur = |_: &str, _: &str, _: &[(&str, &str)]| -> Result<(u32, String), String> { panic!("not Imgur") };
    let refused = |_: &crate::s3::Object| Err("Access Denied (AccessDenied)".to_string());
    let (text, kind) = delete_record(&st, &rec, "cid", &imgur, &refused);
    assert_eq!((text.as_str(), kind), ("Couldn't delete the upload: Access Denied (AccessDenied)", crate::editor::style::ToastKind::Error));
    assert!(st.get(&id).unwrap().meta.uploaded.is_some());
    let asked = RefCell::new(None);
    let ok = |o: &crate::s3::Object| {
        *asked.borrow_mut() = Some(o.clone());
        Ok(())
    };
    let (text, _) = delete_record(&st, &rec, "cid", &imgur, &ok);
    assert_eq!(text, "Deleted from S3");
    assert_eq!(asked.borrow().as_ref(), Some(&s3_obj("k-1.png")));
    assert_eq!(st.get(&id).unwrap().meta.uploaded, None);
}
