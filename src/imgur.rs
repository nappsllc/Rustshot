//! Imgur video uploads (Copy link on an MP4). The image endpoint takes a
//! raw body and rejects video, so a recording goes to `POST /3/upload` as
//! `multipart/form-data` with the file in the `video` field (anonymous,
//! `Authorization: Client-ID …`, as screenshots are). Imgur answers with
//! the post's id and link at once and transcodes afterwards
//! (`"processing": {"status": "pending"}`); the link only plays once that
//! is done, so `GET /3/image/{id}` is polled until the status is
//! `completed` (or `failed`).
//!
//! Imgur takes videos up to 200 MB (and, for anonymous uploads, 60 s);
//! a larger file is refused here before anything is sent. The transport is
//! the platform's (`export::http_post` / `http_get`: WinHTTP, curl), passed
//! in so tests never touch the network.

use std::time::Duration;

pub const HOST: &str = "api.imgur.com";
pub const UPLOAD_PATH: &str = "/3/upload";
/// Imgur's limit for a video upload.
pub const MAX_VIDEO_BYTES: u64 = 200 * 1024 * 1024;
/// How often, and how long at most, the transcode is polled.
pub const POLL_EVERY: Duration = Duration::from_secs(2);
pub const POLL_FOR: Duration = Duration::from_secs(120);

/// POST (host, path, headers, body) → (status, body).
pub type PostFn<'a> = &'a dyn Fn(&str, &str, &[(&str, &str)], &[u8]) -> Result<(u32, String), String>;
/// GET (host, path, headers) → body (Err for a non-2xx).
pub type GetFn<'a> = &'a dyn Fn(&str, &str, &[(&str, &str)]) -> Result<String, String>;

/// The transport.
pub struct Http<'a> {
    pub post: PostFn<'a>,
    pub get: GetFn<'a>,
    pub sleep: &'a dyn Fn(Duration),
}

/// A `multipart/form-data` body: the text `fields`, then the file in
/// `file_field` (named `file_name`, of type `mime`).
pub fn multipart(boundary: &str, fields: &[(&str, &str)], file_field: &str, file_name: &str, mime: &str, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 512);
    for (k, v) in fields {
        out.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    let name = file_name.replace(['"', '\r', '\n'], "_");
    out.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n")
            .as_bytes(),
    );
    out.extend_from_slice(data);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

/// A boundary that does not occur in `data` (`seed` varies it).
pub fn boundary_for(data: &[u8], seed: u64) -> String {
    let mut n = seed;
    loop {
        let b = format!("----rustshot{n:016x}");
        if !data.windows(b.len()).any(|w| w == b.as_bytes()) {
            return b;
        }
        n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }
}

/// `"key": "value"` (the first after `from`, if given), unescaped `\/`.
fn json_str(body: &str, key: &str, from: Option<&str>) -> Option<String> {
    let start = from.and_then(|f| body.find(&format!("\"{f}\""))).unwrap_or(0);
    let body = &body[start..];
    let needle = format!("\"{key}\"");
    let i = body.find(&needle)? + needle.len();
    let rest = body[i..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut esc = false;
    for c in rest.chars() {
        match (esc, c) {
            (false, '\\') => esc = true,
            (false, '"') => return Some(out),
            (true, 'n') => {
                out.push(' ');
                esc = false;
            }
            (_, c) => {
                out.push(c);
                esc = false;
            }
        }
    }
    None
}

/// What Imgur said about an upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub id: String,
    pub link: String,
    /// Still transcoding (`processing.status` pending).
    pub pending: bool,
}

/// Imgur's error text in `body` (a string `error`, or its `message`).
fn error_of(body: &str) -> Option<String> {
    json_str(body, "message", Some("error")).or_else(|| json_str(body, "error", None))
}

/// Parse an upload or image response (`status` the HTTP status).
pub fn parse_reply(status: u32, body: &str) -> Result<Reply, String> {
    let failed = body.contains("\"success\":false") || body.contains("\"success\": false") || !(200..300).contains(&status);
    if failed {
        let why = error_of(body).unwrap_or_else(|| format!("HTTP {status}"));
        return Err(why);
    }
    let link = json_str(body, "link", None).ok_or_else(|| format!("no link in response: {}", body.chars().take(200).collect::<String>()))?;
    let id = json_str(body, "id", None).unwrap_or_default();
    let state = json_str(body, "status", Some("processing"));
    if state.as_deref() == Some("failed") {
        return Err("Imgur could not process the video".into());
    }
    Ok(Reply { id, link, pending: state.as_deref() == Some("pending") })
}

/// Upload `data` (an MP4 named `name`) and wait for Imgur to process it;
/// the link to share.
pub fn upload_video(data: &[u8], name: &str, client_id: &str, seed: u64, http: &Http) -> Result<String, String> {
    if data.len() as u64 > MAX_VIDEO_BYTES {
        return Err(format!("Imgur takes videos up to {} MB", MAX_VIDEO_BYTES / (1024 * 1024)));
    }
    let b = boundary_for(data, seed);
    let body = multipart(&b, &[("type", "file"), ("title", "rustshot"), ("description", "rustshot recording")], "video", name, "video/mp4", data);
    let auth = format!("Client-ID {client_id}");
    let ctype = format!("multipart/form-data; boundary={b}");
    let (status, text) = (http.post)(HOST, UPLOAD_PATH, &[("Authorization", &auth), ("Content-Type", &ctype)], &body)?;
    let mut r = parse_reply(status, &text)?;
    // Its deletehash, for the upload's record.
    crate::uploads::note_reply(&text, crate::uploads::UpKind::Video);
    let mut waited = Duration::ZERO;
    while r.pending && !r.id.is_empty() {
        if waited >= POLL_FOR {
            // The link works once Imgur is done; hand it over anyway.
            break;
        }
        (http.sleep)(POLL_EVERY);
        waited += POLL_EVERY;
        let text = (http.get)(HOST, &format!("/3/image/{}", r.id), &[("Authorization", &auth)])?;
        r = parse_reply(200, &text)?;
    }
    Ok(r.link)
}

/// [`upload_video`] over the platform's HTTPS (never in tests).
#[cfg(not(test))]
pub fn upload_video_file(path: &std::path::Path, client_id: &str) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let name = path.file_name().map_or_else(|| "recording.mp4".into(), |n| n.to_string_lossy().into_owned());
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let post = |h: &str, p: &str, hd: &[(&str, &str)], b: &[u8]| crate::export::http_post(h, p, hd, b);
    let get = |h: &str, p: &str, hd: &[(&str, &str)]| crate::export::http_get(h, p, hd);
    let sleep = |d: Duration| std::thread::sleep(d);
    upload_video(&data, &name, client_id, seed, &Http { post: &post, get: &get, sleep: &sleep })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn multipart_body_has_the_fields_then_the_video() {
        let b = multipart("XYZ", &[("type", "file")], "video", "a\"b.mp4", "video/mp4", b"\x00\x01mp4");
        let want = "--XYZ\r\nContent-Disposition: form-data; name=\"type\"\r\n\r\nfile\r\n\
                    --XYZ\r\nContent-Disposition: form-data; name=\"video\"; filename=\"a_b.mp4\"\r\nContent-Type: video/mp4\r\n\r\n\
                    \x00\x01mp4\r\n--XYZ--\r\n";
        assert_eq!(String::from_utf8_lossy(&b), want);
    }

    #[test]
    fn the_boundary_is_not_in_the_data() {
        let first = boundary_for(b"", 7);
        let data = format!("junk{first}junk");
        let b = boundary_for(data.as_bytes(), 7);
        assert_ne!(b, first);
        assert!(!data.contains(&b));
    }

    #[test]
    fn replies_success_pending_and_errors() {
        let ok = r#"{"data":{"id":"aB3x","link":"https:\/\/i.imgur.com\/aB3x.mp4","processing":{"status":"pending"}},"success":true,"status":200}"#;
        assert_eq!(parse_reply(200, ok), Ok(Reply { id: "aB3x".into(), link: "https://i.imgur.com/aB3x.mp4".into(), pending: true }));
        let done = ok.replace("pending", "completed");
        assert!(!parse_reply(200, &done).unwrap().pending);
        let no_proc = r#"{"data":{"id":"q","link":"https://i.imgur.com/q.mp4"},"success":true}"#;
        assert!(!parse_reply(200, no_proc).unwrap().pending);
        let bad = r#"{"data":{"error":"File type invalid (1)","request":"\/3\/image","method":"POST"},"success":false,"status":400}"#;
        assert_eq!(parse_reply(400, bad), Err("File type invalid (1)".into()));
        let nested = r#"{"data":{"error":{"code":1003,"message":"File is over the size limit","type":"ImgurException"}},"success":false,"status":400}"#;
        assert_eq!(parse_reply(400, nested), Err("File is over the size limit".into()));
        assert_eq!(parse_reply(429, "Too Many Requests"), Err("HTTP 429".into()));
        assert_eq!(parse_reply(200, &ok.replace("pending", "failed")), Err("Imgur could not process the video".into()));
    }

    /// The video goes to /3/upload as multipart (the `video` field) with
    /// the client id; a pending transcode is polled at /3/image/{id}.
    #[test]
    fn a_video_is_uploaded_then_polled_until_processed() {
        let posted = RefCell::new(Vec::new());
        let polls = RefCell::new(0);
        let post = |h: &str, p: &str, hd: &[(&str, &str)], b: &[u8]| {
            posted.borrow_mut().push((h.to_string(), p.to_string(), hd.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>(), b.to_vec()));
            Ok((200, r#"{"data":{"id":"v1","link":"https://i.imgur.com/v1.mp4","processing":{"status":"pending"}},"success":true}"#.to_string()))
        };
        let get = |h: &str, p: &str, hd: &[(&str, &str)]| {
            assert_eq!((h, p), (HOST, "/3/image/v1"));
            assert_eq!(hd, &[("Authorization", "Client-ID cid")]);
            *polls.borrow_mut() += 1;
            let st = if *polls.borrow() < 3 { "pending" } else { "completed" };
            Ok(format!(r#"{{"data":{{"id":"v1","link":"https://i.imgur.com/v1.mp4","processing":{{"status":"{st}"}}}},"success":true}}"#))
        };
        let slept = RefCell::new(Duration::ZERO);
        let sleep = |d: Duration| *slept.borrow_mut() += d;
        let http = Http { post: &post, get: &get, sleep: &sleep };
        assert_eq!(upload_video(b"MP4DATA", "clip.mp4", "cid", 1, &http), Ok("https://i.imgur.com/v1.mp4".into()));
        assert_eq!(*polls.borrow(), 3);
        assert_eq!(*slept.borrow(), POLL_EVERY * 3);
        let p = posted.borrow();
        let (h, path, hd, body) = &p[0];
        assert_eq!((h.as_str(), path.as_str()), (HOST, UPLOAD_PATH));
        assert_eq!(hd[0], "Authorization: Client-ID cid");
        let b = hd[1].strip_prefix("Content-Type: multipart/form-data; boundary=").expect("multipart");
        let text = String::from_utf8_lossy(body);
        assert!(text.contains("name=\"video\"; filename=\"clip.mp4\"\r\nContent-Type: video/mp4\r\n\r\nMP4DATA\r\n"), "{text}");
        assert!(text.ends_with(&format!("--{b}--\r\n")));
    }

    #[test]
    fn too_big_or_refused_videos_fail_with_imgurs_reason() {
        let post = |_: &str, _: &str, _: &[(&str, &str)], _: &[u8]| -> Result<(u32, String), String> {
            Ok((400, r#"{"data":{"error":"Video is too long"},"success":false,"status":400}"#.into()))
        };
        let get = |_: &str, _: &str, _: &[(&str, &str)]| -> Result<String, String> { panic!("no poll after an error") };
        let sleep = |_: Duration| {};
        let http = Http { post: &post, get: &get, sleep: &sleep };
        assert_eq!(upload_video(b"x", "a.mp4", "c", 0, &http), Err("Video is too long".into()));
        // Over the limit: nothing is sent.
        let never = |_: &str, _: &str, _: &[(&str, &str)], _: &[u8]| -> Result<(u32, String), String> { panic!("sent") };
        let http = Http { post: &never, get: &get, sleep: &sleep };
        let big = vec![0u8; MAX_VIDEO_BYTES as usize + 1];
        assert!(upload_video(&big, "a.mp4", "c", 0, &http).unwrap_err().contains("200 MB"));
    }

    /// A transcode that never ends: the link after `POLL_FOR`.
    #[test]
    fn polling_gives_up_with_the_link() {
        let post = |_: &str, _: &str, _: &[(&str, &str)], _: &[u8]| {
            Ok((200, r#"{"data":{"id":"s","link":"https://i.imgur.com/s.mp4","processing":{"status":"pending"}},"success":true}"#.to_string()))
        };
        let get = |_: &str, _: &str, _: &[(&str, &str)]| {
            Ok(r#"{"data":{"id":"s","link":"https://i.imgur.com/s.mp4","processing":{"status":"pending"}},"success":true}"#.to_string())
        };
        let n = RefCell::new(0u32);
        let sleep = |_: Duration| *n.borrow_mut() += 1;
        let http = Http { post: &post, get: &get, sleep: &sleep };
        assert_eq!(upload_video(b"x", "a.mp4", "c", 0, &http), Ok("https://i.imgur.com/s.mp4".into()));
        assert_eq!(*n.borrow(), (POLL_FOR.as_secs() / POLL_EVERY.as_secs()) as u32);
    }
}
