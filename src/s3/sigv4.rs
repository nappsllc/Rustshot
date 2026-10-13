//! AWS Signature Version 4, by hand (no AWS SDK): the canonical request,
//! the string to sign, the derived signing key and the `Authorization`
//! header, as the SigV4 documentation spells them out. HMAC-SHA256 and
//! SHA-256 are `crate::sha256`'s.
//!
//! The caller builds the path already encoded (S3 encodes each key byte
//! once: [`uri_encode`] with `/` kept) and hands the query unencoded
//! ([`canonical_query`] encodes and sorts it). Tested against the
//! examples AWS publishes (the SigV4 test suite's `get-vanilla` family and
//! the S3 header-signing examples), embedded as literals.

use crate::sha256::{hex, hmac};

pub const ALGORITHM: &str = "AWS4-HMAC-SHA256";
/// The payload hash of a body that is not hashed (streamed over HTTPS).
pub const UNSIGNED: &str = "UNSIGNED-PAYLOAD";
/// SHA-256 of nothing.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Access key id and secret access key.
#[derive(Clone, Copy)]
pub struct Creds<'a> {
    pub access_key: &'a str,
    pub secret: &'a str,
}

impl std::fmt::Debug for Creds<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret never reaches a log.
        write!(f, "Creds({}, secret hidden)", self.access_key)
    }
}

/// What is signed of a request.
pub struct Canon<'a> {
    pub method: &'a str,
    /// Already encoded ("/bucket/a%20b.png").
    pub path: &'a str,
    /// Unencoded (name, value) pairs, any order.
    pub query: &'a [(String, String)],
    /// Every header to sign (names in any case), `host` and `x-amz-date`
    /// among them.
    pub headers: &'a [(String, String)],
    /// Hex SHA-256 of the body, or [`UNSIGNED`].
    pub payload_hash: &'a str,
}

/// RFC 3986 percent-encoding as SigV4 wants it: unreserved characters
/// (`A-Z a-z 0-9 - _ . ~`) stay, every other byte is `%XX` (uppercase);
/// `/` stays when `keep_slash` (an object key in a path).
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') || (keep_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The canonical query string: each name and value encoded, sorted by
/// name (then value), `name=value` joined by `&` (a bare `uploads` is
/// `uploads=`).
pub fn canonical_query(q: &[(String, String)]) -> String {
    let mut pairs: Vec<(String, String)> = q.iter().map(|(k, v)| (uri_encode(k, false), uri_encode(v, false))).collect();
    crate::sort::sort(&mut pairs);
    pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

/// A header value as signed: trimmed, runs of spaces made one.
fn canon_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut space = false;
    for c in v.trim().chars() {
        if c == ' ' || c == '\t' {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

/// (canonical headers block, signed header names): names lowercased and
/// sorted, one `name:value\n` line each (repeated names joined by `,`).
pub fn canonical_headers(h: &[(String, String)]) -> (String, String) {
    let mut v: Vec<(String, String)> = h.iter().map(|(k, x)| (k.trim().to_ascii_lowercase(), canon_value(x))).collect();
    crate::sort::by(&mut v, |a, b| a.0.cmp(&b.0));
    let mut merged: Vec<(String, String)> = Vec::new();
    for (k, x) in v {
        match merged.last_mut() {
            Some(last) if last.0 == k => {
                last.1.push(',');
                last.1.push_str(&x);
            }
            _ => merged.push((k, x)),
        }
    }
    let block: String = merged.iter().map(|(k, x)| format!("{k}:{x}\n")).collect();
    let names = merged.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    (block, names)
}

/// The canonical request.
pub fn canonical_request(c: &Canon) -> String {
    let (block, names) = canonical_headers(c.headers);
    let path = if c.path.is_empty() { "/" } else { c.path };
    format!("{}\n{}\n{}\n{}\n{}\n{}", c.method, path, canonical_query(c.query), block, names, c.payload_hash)
}

/// `20150830/us-east-1/service/aws4_request` (`amz_date` the full
/// `20150830T123600Z`).
pub fn scope(amz_date: &str, region: &str, service: &str) -> String {
    format!("{}/{region}/{service}/aws4_request", &amz_date[..amz_date.len().min(8)])
}

pub fn string_to_sign(amz_date: &str, scope: &str, creq: &str) -> String {
    format!("{ALGORITHM}\n{amz_date}\n{scope}\n{}", hex(&crate::sha256::digest(creq.as_bytes())))
}

/// The signing key: `AWS4<secret>` HMAC'd through date, region, service
/// and `aws4_request`.
pub fn signing_key(secret: &str, date8: &str, region: &str, service: &str) -> [u8; 32] {
    let k = hmac(format!("AWS4{secret}").as_bytes(), date8.as_bytes());
    let k = hmac(&k, region.as_bytes());
    let k = hmac(&k, service.as_bytes());
    hmac(&k, b"aws4_request")
}

/// The hex signature of request `c`.
pub fn signature(creds: Creds, region: &str, service: &str, amz_date: &str, c: &Canon) -> String {
    let sc = scope(amz_date, region, service);
    let sts = string_to_sign(amz_date, &sc, &canonical_request(c));
    let key = signing_key(creds.secret, &amz_date[..amz_date.len().min(8)], region, service);
    hex(&hmac(&key, sts.as_bytes()))
}

/// The `Authorization` header's value.
pub fn authorization(creds: Creds, region: &str, service: &str, amz_date: &str, c: &Canon) -> String {
    let (_, names) = canonical_headers(c.headers);
    format!(
        "{ALGORITHM} Credential={}/{}, SignedHeaders={names}, Signature={}",
        creds.access_key,
        scope(amz_date, region, service),
        signature(creds, region, service, amz_date, c)
    )
}

/// `YYYYMMDDTHHMMSSZ` for seconds since 1970 (UTC).
pub fn amz_date(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let (y, m, d) = crate::history::civil_from_days(days);
    let s = unix % 86_400;
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", s / 3600, s / 60 % 60, s % 60)
}
