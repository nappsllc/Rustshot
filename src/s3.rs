//! Uploads to S3-compatible storage (Amazon S3, Cloudflare R2, MinIO,
//! Backblaze B2, …): Settings › Upload › Destination "S3-compatible".
//!
//! A capture or recording is `PUT` as one object under the bucket
//! (`Content-Type` image/png, image/jpeg, video/mp4 or image/gif) at the
//! configured key prefix (strftime tokens, `%Y/%m/%d/` by default) plus
//! its file name and a short random tail, so two captures in the same
//! minute never overwrite each other. Files over [`MULTIPART_OVER`] go up
//! as a multipart upload in equal [`PART_SIZE`] parts (R2 wants every part
//! but the last the same size), aborted when any step fails. The link is
//! the public URL base (a CDN, the bucket's `r2.dev` address) plus the
//! key; without a base, the object's own URL.
//!
//! Requests are signed with AWS Signature Version 4 ([`sigv4`], by hand:
//! no AWS SDK). Small bodies are hashed and signed; files are streamed
//! with `x-amz-content-sha256: UNSIGNED-PAYLOAD` (allowed over HTTPS,
//! which every endpoint must be). Addressing is path-style
//! (`https://<endpoint>/<bucket>/<key>`, the default: R2 and most
//! self-hosted servers) or virtual-hosted (`https://<bucket>.<endpoint>/<key>`).
//!
//! The access key id is in `config.toml`; the secret access key never is:
//! it lives in the OS credential store ([`crate::keystore`], named
//! `rustshot:s3:<bucket>`). The transport ([`SendFn`]: WinHTTP on Windows,
//! curl elsewhere, `crate::export::http_request`) is passed in, so tests
//! run against a mock and never touch the network.

pub mod sigv4;

use crate::config::Config;
use sigv4::{Canon, Creds};
use std::path::Path;

const MIB: u64 = 1024 * 1024;
/// Files larger than this go up in parts.
pub const MULTIPART_OVER: u64 = 64 * MIB;
/// The part size (S3's minimum is 5 MB; at most [`MAX_PARTS`] parts).
pub const PART_SIZE: u64 = 16 * MIB;
pub const MIN_PART: u64 = 5 * MIB;
pub const MAX_PARTS: u64 = 10_000;
/// S3 takes objects up to 5 TB (any single PUT up to 5 GB).
pub const MAX_OBJECT: u64 = 5 * 1024 * 1024 * MIB;
pub const SERVICE: &str = "s3";
/// The region when none is set (R2 takes "auto").
pub const DEFAULT_REGION: &str = "auto";
pub const DEFAULT_PREFIX: &str = "%Y/%m/%d/";

// --- the transport --------------------------------------------------------------

/// A request body.
#[derive(Clone, Copy, Debug)]
pub enum Body<'a> {
    Empty,
    Bytes(&'a [u8]),
    /// `len` bytes of `path` from `offset` (streamed, unsigned).
    File { path: &'a Path, offset: u64, len: u64 },
}

impl Body<'_> {
    pub fn len(&self) -> u64 {
        match self {
            Body::Empty => 0,
            Body::Bytes(b) => b.len() as u64,
            Body::File { len, .. } => *len,
        }
    }

    /// Hand the body to `f` in pieces of at most `chunk` bytes (a file
    /// is read as it goes; one that ends early is an error).
    pub fn for_each_chunk(&self, chunk: usize, f: &mut dyn FnMut(&[u8]) -> Result<(), String>) -> Result<(), String> {
        match *self {
            Body::Empty => Ok(()),
            Body::Bytes(b) => b.chunks(chunk.max(1)).try_for_each(f),
            Body::File { path, offset, len } => {
                use std::io::{Read, Seek, SeekFrom};
                let rd = |e: std::io::Error| format!("read {}: {e}", path.display());
                let mut file = std::fs::File::open(path).map_err(rd)?;
                file.seek(SeekFrom::Start(offset)).map_err(rd)?;
                let mut buf = vec![0u8; chunk.max(1)];
                let mut left = len;
                while left > 0 {
                    let want = (left.min(buf.len() as u64)) as usize;
                    let n = file.read(&mut buf[..want]).map_err(rd)?;
                    if n == 0 {
                        return Err(format!("{} ended early", path.display()));
                    }
                    f(&buf[..n])?;
                    left -= n as u64;
                }
                Ok(())
            }
        }
    }
}

/// One HTTPS request: `host` (with a `:port` when not 443) and the path
/// with its query, already encoded; `headers` besides `Host` and
/// `Content-Length` (which the transport adds).
#[derive(Debug)]
pub struct HttpReq<'a> {
    pub method: &'static str,
    pub host: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Body<'a>,
}

impl HttpReq<'_> {
    /// `host` without its port, and the port (443 by default).
    pub fn host_port(&self) -> (&str, u16) {
        split_port(&self.host)
    }
}

/// `host[:port]` → (host, port).
pub fn split_port(h: &str) -> (&str, u16) {
    match h.rsplit_once(':') {
        Some((n, p)) if !n.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (n, p.parse().unwrap_or(443)),
        _ => (h, 443),
    }
}

/// What came back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HttpResp {
    pub status: u32,
    /// Names as sent (compare case-insensitively: [`HttpResp::header`]).
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpResp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub type SendFn<'a> = &'a dyn Fn(&HttpReq) -> Result<HttpResp, String>;

/// Response headers as WinHTTP's raw block or curl's `-i` print them
/// (`HTTP/1.1 200 OK\r\nETag: "x"\r\n…`): the last block's lines.
pub fn parse_header_block(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let k = k.trim();
            (!k.is_empty() && !k.contains(' ')).then(|| (k.to_string(), v.trim().to_string()))
        })
        .collect()
}

// --- where things go -----------------------------------------------------------

/// The bucket uploads go to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    /// `https://host[:port]` as typed (see [`parse_endpoint`]).
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub path_style: bool,
}

/// An uploaded object: where it is (kept with the upload's record, so
/// "Delete upload…" works after the settings changed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Object {
    pub target: Target,
    pub key: String,
}

/// The `host[:port]` of an endpoint: `https://` (or no scheme), a host
/// name or address, an optional port, nothing after but a `/`. Plain
/// `http://` is refused (the signature and the file would travel in the
/// clear).
pub fn parse_endpoint(s: &str) -> Result<String, String> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") {
        return Err("the endpoint must be https://".into());
    }
    let rest = if lower.starts_with("https://") { &s[8..] } else { s };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return Err("no endpoint is set".into());
    }
    if rest.contains(['/', '?', '#', '@']) {
        return Err("the endpoint is a host name only (no path)".into());
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    let label_ok = |l: &str| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !l.starts_with('-') && !l.ends_with('-');
    if host.len() > 253 || !host.split('.').all(label_ok) {
        return Err(format!("\"{host}\" is not a host name"));
    }
    if let Some(p) = port
        && !p.parse::<u16>().is_ok_and(|n| n > 0)
    {
        return Err(format!("\"{p}\" is not a port"));
    }
    Ok(rest.to_ascii_lowercase())
}

/// A bucket name S3 and R2 accept (3–63 of `a-z 0-9 . -`, starting and
/// ending with a letter or digit).
pub fn valid_bucket(b: &str) -> bool {
    (3..=63).contains(&b.len())
        && b.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        && b.as_bytes()[0].is_ascii_alphanumeric()
        && b.as_bytes()[b.len() - 1].is_ascii_alphanumeric()
}

impl Target {
    pub fn from_config(c: &Config) -> Target {
        Target { endpoint: c.s3_endpoint.trim().into(), region: c.s3_region.trim().into(), bucket: c.s3_bucket.trim().into(), path_style: c.s3_path_style }
    }

    /// The signing region ([`DEFAULT_REGION`] when blank).
    pub fn region(&self) -> &str {
        if self.region.trim().is_empty() { DEFAULT_REGION } else { self.region.trim() }
    }

    /// What is wrong with it, if anything.
    pub fn check(&self) -> Result<(), String> {
        parse_endpoint(&self.endpoint)?;
        if self.bucket.is_empty() {
            return Err("no bucket is set".into());
        }
        if !valid_bucket(&self.bucket) {
            return Err(format!("\"{}\" is not a bucket name", self.bucket));
        }
        Ok(())
    }

    /// The host and encoded path of `key`.
    pub fn host_path(&self, key: &str) -> Result<(String, String), String> {
        self.check()?;
        let host = parse_endpoint(&self.endpoint)?;
        let k = sigv4::uri_encode(key, true);
        Ok(if self.path_style { (host, format!("/{}/{k}", self.bucket)) } else { (format!("{}.{host}", self.bucket), format!("/{k}")) })
    }

    /// `https://…` of `key` itself (no public base).
    pub fn object_url(&self, key: &str) -> Result<String, String> {
        let (h, p) = self.host_path(key)?;
        Ok(format!("https://{h}{p}"))
    }
}

/// The link to share: `base` (a CDN, the `r2.dev` address; `https://`
/// added when it has no scheme) plus the key, else the object's URL.
pub fn public_link(base: &str, target: &Target, key: &str) -> Result<String, String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return target.object_url(key);
    }
    let lower = base.to_ascii_lowercase();
    let base = if lower.starts_with("https://") || lower.starts_with("http://") { base.to_string() } else { format!("https://{base}") };
    Ok(format!("{base}/{}", sigv4::uri_encode(key, true)))
}

/// The content type of a file by its extension.
pub fn content_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "mp4" => "video/mp4",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// The object key: `prefix` with its tokens filled in for `now` (`%Y`,
/// `%m`, `%d`, `%H`, … as in file names; empty, `.` and `..` segments
/// dropped), then `name` with `-<tail>` before its extension. Control
/// characters and backslashes never reach a key.
pub fn object_key(prefix: &str, now: crate::export::Tm, epoch: u64, name: &str, tail: &str) -> String {
    let clean = |s: &str| s.chars().filter(|c| !c.is_control() && *c != '\\').collect::<String>();
    let expanded = crate::export::expand_pattern(prefix, now, epoch);
    let mut key = String::new();
    for seg in expanded.split('/') {
        let seg = clean(seg);
        let seg = seg.trim();
        if seg.is_empty() || seg == "." || seg == ".." {
            continue;
        }
        key.push_str(seg);
        key.push('/');
    }
    let name = clean(name.rsplit(['/', '\\']).next().unwrap_or(name));
    let name = name.trim();
    let name = if name.is_empty() || name == "." || name == ".." { "capture" } else { name };
    let file = match (name.rsplit_once('.'), tail.is_empty()) {
        (_, true) => name.to_string(),
        (Some((stem, ext)), false) if !stem.is_empty() => format!("{stem}-{tail}.{ext}"),
        _ => format!("{name}-{tail}"),
    };
    key.push_str(&file);
    key
}

/// `n` random-looking base-36 characters (keys, the test object): the
/// clock and a counter through SHA-256 (nothing here needs to be secret).
pub fn random_tail(n: usize) -> String {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let c = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let seed = format!("{t}:{c}:{}", std::process::id());
    let d = crate::sha256::digest(seed.as_bytes());
    const A: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    d.iter().cycle().take(n).map(|b| A[(*b % 36) as usize] as char).collect()
}

// --- multipart planning ------------------------------------------------------------

/// The parts of a `size`-byte file: (offset, length) each, every one
/// [`PART_SIZE`] long (or more, whole MiB, so they stay under
/// [`MAX_PARTS`]) but the last. Empty: one plain PUT will do.
pub fn plan(size: u64) -> Vec<(u64, u64)> {
    if size <= MULTIPART_OVER {
        return Vec::new();
    }
    let need = size.div_ceil(MAX_PARTS).div_ceil(MIB) * MIB;
    let part = PART_SIZE.max(need).max(MIN_PART);
    (0..size.div_ceil(part)).map(|i| (i * part, part.min(size - i * part))).collect()
}

// --- talking to the bucket ----------------------------------------------------------

/// The text of `<tag>…</tag>` in an S3 XML reply.
pub fn xml_tag(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let i = body.find(&open)? + open.len();
    let j = body[i..].find(&format!("</{tag}>"))? + i;
    Some(xml_unescape(&body[i..j]))
}

fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Why a request failed: S3's `<Message>` and `<Code>`, or the status.
pub fn failure(r: &HttpResp) -> String {
    let code = xml_tag(&r.body, "Code");
    let msg = xml_tag(&r.body, "Message").filter(|m| !m.trim().is_empty());
    match (code, msg) {
        (Some(c), Some(m)) => format!("{} ({c})", m.trim().trim_end_matches('.')),
        (Some(c), None) => c,
        (None, Some(m)) => m,
        (None, None) => format!("HTTP {}", r.status),
    }
}

/// Signs and sends requests for one bucket.
pub struct Client<'a> {
    pub target: &'a Target,
    pub creds: Creds<'a>,
    pub send: SendFn<'a>,
    /// Seconds since 1970 (each request is signed when it is sent).
    pub clock: &'a dyn Fn() -> u64,
}

impl<'a> Client<'a> {
    /// The signed request for `key`.
    pub fn request<'b>(&self, method: &'static str, key: &str, query: &[(String, String)], mut headers: Vec<(String, String)>, body: Body<'b>) -> Result<HttpReq<'b>, String> {
        if self.creds.access_key.trim().is_empty() {
            return Err("no access key ID is set".into());
        }
        if self.creds.secret.is_empty() {
            return Err("no secret access key is stored".into());
        }
        let (host, path) = self.target.host_path(key)?;
        let hash = match body {
            Body::Empty => sigv4::EMPTY_SHA256.to_string(),
            Body::Bytes(b) => crate::sha256::hex(&crate::sha256::digest(b)),
            Body::File { .. } => sigv4::UNSIGNED.to_string(),
        };
        let date = sigv4::amz_date((self.clock)());
        headers.push(("host".into(), host.clone()));
        headers.push(("x-amz-content-sha256".into(), hash.clone()));
        headers.push(("x-amz-date".into(), date.clone()));
        let creds = Creds { access_key: self.creds.access_key.trim(), secret: self.creds.secret };
        let canon = Canon { method, path: &path, query, headers: &headers, payload_hash: &hash };
        let auth = sigv4::authorization(creds, self.target.region(), SERVICE, &date, &canon);
        // The transport sends Host itself (the host it connects to).
        headers.retain(|(k, _)| k != "host");
        headers.push(("Authorization".into(), auth));
        let q = sigv4::canonical_query(query);
        let path = if q.is_empty() { path } else { format!("{path}?{q}") };
        Ok(HttpReq { method, host, path, headers, body })
    }

    fn call(&self, method: &'static str, key: &str, query: &[(String, String)], headers: Vec<(String, String)>, body: Body) -> Result<HttpResp, String> {
        let r = self.request(method, key, query, headers, body)?;
        (self.send)(&r)
    }

    pub fn put_object(&self, key: &str, ctype: &str, body: Body) -> Result<(), String> {
        let r = self.call("PUT", key, &[], vec![("content-type".into(), ctype.into())], body)?;
        if r.ok() { Ok(()) } else { Err(failure(&r)) }
    }

    /// Delete `key` (S3 answers 204 whether or not it was there).
    pub fn delete_object(&self, key: &str) -> Result<(), String> {
        let r = self.call("DELETE", key, &[], Vec::new(), Body::Empty)?;
        if r.ok() || r.status == 404 { Ok(()) } else { Err(failure(&r)) }
    }

    /// Start a multipart upload: its id.
    pub fn create_multipart(&self, key: &str, ctype: &str) -> Result<String, String> {
        let q = [("uploads".to_string(), String::new())];
        let r = self.call("POST", key, &q, vec![("content-type".into(), ctype.into())], Body::Empty)?;
        if !r.ok() {
            return Err(failure(&r));
        }
        xml_tag(&r.body, "UploadId").filter(|u| !u.is_empty()).ok_or_else(|| "no UploadId in the reply".into())
    }

    /// Upload part `n` (1-based): its ETag.
    pub fn upload_part(&self, key: &str, upload_id: &str, n: usize, body: Body) -> Result<String, String> {
        let q = [("partNumber".to_string(), n.to_string()), ("uploadId".to_string(), upload_id.to_string())];
        let r = self.call("PUT", key, &q, Vec::new(), body)?;
        if !r.ok() {
            return Err(failure(&r));
        }
        r.header("etag").map(str::to_string).filter(|e| !e.is_empty()).ok_or_else(|| format!("no ETag for part {n}"))
    }

    pub fn complete_multipart(&self, key: &str, upload_id: &str, etags: &[String]) -> Result<(), String> {
        let q = [("uploadId".to_string(), upload_id.to_string())];
        let xml = complete_xml(etags);
        let r = self.call("POST", key, &q, vec![("content-type".into(), "application/xml".into())], Body::Bytes(xml.as_bytes()))?;
        // An error can come with a 200 (S3 answers early, then fails).
        if r.ok() && !r.body.contains("<Error>") { Ok(()) } else { Err(failure(&r)) }
    }

    pub fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), String> {
        let q = [("uploadId".to_string(), upload_id.to_string())];
        let r = self.call("DELETE", key, &q, Vec::new(), Body::Empty)?;
        if r.ok() || r.status == 404 { Ok(()) } else { Err(failure(&r)) }
    }

    /// Upload the file at `path` (`size` bytes) to `key`: one PUT, or in
    /// parts when it is large (aborted on any failure).
    pub fn upload_file(&self, path: &Path, size: u64, key: &str, ctype: &str) -> Result<(), String> {
        if size > MAX_OBJECT {
            return Err("the file is larger than S3 takes (5 TB)".into());
        }
        let parts = plan(size);
        if parts.is_empty() {
            return self.put_object(key, ctype, Body::File { path, offset: 0, len: size });
        }
        let id = self.create_multipart(key, ctype)?;
        let mut etags = Vec::with_capacity(parts.len());
        let mut run = || -> Result<(), String> {
            for (i, (offset, len)) in parts.iter().enumerate() {
                let e = self.upload_part(key, &id, i + 1, Body::File { path, offset: *offset, len: *len }).map_err(|e| format!("part {} of {}: {e}", i + 1, parts.len()))?;
                etags.push(e);
            }
            self.complete_multipart(key, &id, &etags)
        };
        let r = run();
        if r.is_err() {
            // Parts left behind would be stored (and billed) until aborted.
            let _ = self.abort_multipart(key, &id);
        }
        r
    }

    /// Settings' "Test connection": PUT a tiny `rustshot-test-<tail>.txt`,
    /// then DELETE it.
    pub fn test_connection(&self, tail: &str) -> Result<(), String> {
        let key = format!("rustshot-test-{tail}.txt");
        self.put_object(&key, "text/plain", Body::Bytes(b"Rustshot connection test\n")).map_err(|e| format!("upload failed: {e}"))?;
        self.delete_object(&key).map_err(|e| format!("uploaded, but the delete failed: {e}"))
    }
}

/// The `CompleteMultipartUpload` document.
pub fn complete_xml(etags: &[String]) -> String {
    let mut x = String::from("<CompleteMultipartUpload>");
    for (i, e) in etags.iter().enumerate() {
        x.push_str(&format!("<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>", i + 1, xml_escape(e)));
    }
    x.push_str("</CompleteMultipartUpload>");
    x
}

// --- uploads from the app ---------------------------------------------------------------

/// The upload destination setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    Imgur,
    S3,
}

impl Dest {
    pub fn of(c: &Config) -> Dest {
        if c.upload_destination.trim().eq_ignore_ascii_case("s3") { Dest::S3 } else { Dest::Imgur }
    }
}

/// What to upload.
pub enum Source<'a> {
    /// An encoded picture named `name`.
    Bytes(&'a [u8], &'a str),
    File(&'a Path),
}

/// Upload `src` with `cfg`'s S3 settings (the secret from `secret`):
/// the link and where the object is.
pub fn upload(cfg: &Config, src: Source, secret: &str, send: SendFn, clock: &dyn Fn() -> u64) -> Result<(String, Object), String> {
    let target = Target::from_config(cfg);
    target.check()?;
    let name = match &src {
        Source::Bytes(_, n) => n.to_string(),
        Source::File(p) => p.file_name().map_or_else(|| "recording".into(), |n| n.to_string_lossy().into_owned()),
    };
    let now = crate::export::local_ymdhms();
    let key = object_key(&cfg.s3_key_prefix, now, clock(), &name, &random_tail(6));
    let ctype = content_type(&name);
    let client = Client { target: &target, creds: Creds { access_key: &cfg.s3_access_key_id, secret }, send, clock };
    match src {
        Source::Bytes(b, _) => client.put_object(&key, ctype, Body::Bytes(b))?,
        Source::File(p) => {
            let size = std::fs::metadata(p).map_err(|e| format!("read {}: {e}", p.display()))?.len();
            client.upload_file(p, size, &key, ctype)?;
        }
    }
    let link = public_link(&cfg.s3_public_url, &target, &key)?;
    Ok((link, Object { target, key }))
}

/// The secret for `bucket` from the OS credential store.
pub fn stored_secret(bucket: &str) -> Result<String, String> {
    match crate::keystore::os().get(&crate::keystore::s3_name(bucket)) {
        Ok(Some(s)) if !s.is_empty() => Ok(s),
        Ok(_) => Err(format!("no secret access key is stored for \"{bucket}\" (Settings \u{203a} Upload)")),
        Err(e) => Err(format!("could not read the secret access key: {e}")),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The platform's HTTPS (never in tests: they fail instead).
fn platform_send(r: &HttpReq) -> Result<HttpResp, String> {
    #[cfg(test)]
    {
        let _ = r;
        Err("no network in tests".into())
    }
    #[cfg(not(test))]
    {
        crate::export::http_request(r)
    }
}

/// [`upload`] with the stored secret over the platform's HTTPS; the
/// object is noted for the upload's record (`crate::uploads::note_s3`).
pub fn upload_now(cfg: &Config, src: Source) -> Result<String, String> {
    let secret = stored_secret(cfg.s3_bucket.trim())?;
    let video = matches!(&src, Source::File(p) if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4")));
    let (link, obj) = upload(cfg, src, &secret, &platform_send, &unix_now)?;
    crate::uploads::note_s3(&link, obj, if video { crate::uploads::UpKind::Video } else { crate::uploads::UpKind::Image });
    Ok(link)
}

/// Delete an uploaded object (the record's) with its bucket's stored secret.
pub fn delete_now(obj: &Object, access_key: &str, send: SendFn) -> Result<(), String> {
    let secret = stored_secret(&obj.target.bucket)?;
    let client = Client { target: &obj.target, creds: Creds { access_key, secret: &secret }, send, clock: &unix_now };
    client.delete_object(&obj.key)
}

/// [`delete_now`] over the platform's HTTPS.
pub fn delete_platform(obj: &Object, access_key: &str) -> Result<(), String> {
    delete_now(obj, access_key, &platform_send)
}

/// Settings' Test connection with these values (`secret`: typed, or the
/// stored one when blank).
pub fn test_with(target: &Target, access_key: &str, secret: &str) -> Result<(), String> {
    target.check()?;
    let stored;
    let secret = if secret.is_empty() {
        stored = stored_secret(&target.bucket)?;
        stored.as_str()
    } else {
        secret
    };
    let client = Client { target, creds: Creds { access_key, secret }, send: &platform_send, clock: &unix_now };
    client.test_connection(&random_tail(8))
}

#[cfg(test)]
mod tests;
