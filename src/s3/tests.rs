use super::sigv4::*;
use super::*;
use std::cell::RefCell;

fn hs(h: &[(&str, &str)]) -> Vec<(String, String)> {
    h.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn qs(q: &[(&str, &str)]) -> Vec<(String, String)> {
    hs(q)
}

// --- the SigV4 test suite (AKIDEXAMPLE, us-east-1, service "service") ---------------

const SUITE: Creds = Creds { access_key: "AKIDEXAMPLE", secret: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY" };
const SUITE_DATE: &str = "20150830T123600Z";

fn suite(method: &str, path: &str, query: &[(&str, &str)]) -> (String, String, String) {
    let h = hs(&[("Host", "example.amazonaws.com"), ("X-Amz-Date", SUITE_DATE)]);
    let q = qs(query);
    let c = Canon { method, path, query: &q, headers: &h, payload_hash: EMPTY_SHA256 };
    let creq = canonical_request(&c);
    let sts = string_to_sign(SUITE_DATE, &scope(SUITE_DATE, "us-east-1", "service"), &creq);
    (creq, sts, authorization(SUITE, "us-east-1", "service", SUITE_DATE, &c))
}

/// `get-vanilla`: the canonical request, string to sign and header exactly.
#[test]
fn suite_get_vanilla() {
    let (creq, sts, auth) = suite("GET", "/", &[]);
    assert_eq!(
        creq,
        "GET\n/\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sts,
        "AWS4-HMAC-SHA256\n20150830T123600Z\n20150830/us-east-1/service/aws4_request\nbb579772317eb040ac9ed261061d46c1f17a8133879d6129b6e1c25292927e63"
    );
    assert_eq!(
        auth,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
    );
}

/// `post-vanilla` and `get-vanilla-query-order-key-case` (the query is
/// sorted by name).
#[test]
fn suite_post_vanilla_and_query_order() {
    let (_, _, auth) = suite("POST", "/", &[]);
    assert!(auth.ends_with("Signature=5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b"), "{auth}");
    let (creq, _, auth) = suite("GET", "/", &[("Param2", "value2"), ("Param1", "value1")]);
    assert!(creq.starts_with("GET\n/\nParam1=value1&Param2=value2\n"), "{creq}");
    assert!(auth.ends_with("Signature=b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"), "{auth}");
}

/// The derived key from the SigV4 documentation's example (IAM,
/// 2012-02-15).
#[test]
fn signing_key_example() {
    let k = signing_key("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20120215", "us-east-1", "iam");
    assert_eq!(crate::sha256::hex(&k), "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d");
}

// --- the S3 header-signing examples (examplebucket, 2013-05-24) ------------------------

const S3EX: Creds = Creds { access_key: "AKIAIOSFODNN7EXAMPLE", secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY" };
const S3_DATE: &str = "20130524T000000Z";

fn s3_sig(method: &str, path: &str, query: &[(&str, &str)], extra: &[(&str, &str)], hash: &str) -> (String, String) {
    let mut h = hs(&[("Host", "examplebucket.s3.amazonaws.com"), ("x-amz-date", S3_DATE), ("x-amz-content-sha256", hash)]);
    h.extend(hs(extra));
    let q = qs(query);
    let c = Canon { method, path, query: &q, headers: &h, payload_hash: hash };
    (canonical_request(&c), signature(S3EX, "us-east-1", "s3", S3_DATE, &c))
}

#[test]
fn s3_example_get_object_with_range() {
    let (creq, sig) = s3_sig("GET", "/test.txt", &[], &[("Range", "bytes=0-9")], EMPTY_SHA256);
    assert_eq!(
        creq,
        "GET\n/test.txt\n\nhost:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\nx-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nx-amz-date:20130524T000000Z\n\nhost;range;x-amz-content-sha256;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(sig, "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41");
}

/// PUT `test$file.text`: the `$` is encoded in the path, the body hashed.
#[test]
fn s3_example_put_object() {
    let body = b"Welcome to Amazon S3.";
    let hash = crate::sha256::hex(&crate::sha256::digest(body));
    assert_eq!(hash, "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072");
    let path = format!("/{}", uri_encode("test$file.text", true));
    assert_eq!(path, "/test%24file.text");
    let (_, sig) = s3_sig("PUT", &path, &[], &[("Date", "Fri, 24 May 2013 00:00:00 GMT"), ("x-amz-storage-class", "REDUCED_REDUNDANCY")], &hash);
    assert_eq!(sig, "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd");
}

/// GET `?lifecycle` (a bare name signs as `lifecycle=`) and
/// `?max-keys=2&prefix=J`.
#[test]
fn s3_example_bucket_queries() {
    let (creq, sig) = s3_sig("GET", "/", &[("lifecycle", "")], &[], EMPTY_SHA256);
    assert!(creq.starts_with("GET\n/\nlifecycle=\n"), "{creq}");
    assert_eq!(sig, "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543");
    let (_, sig) = s3_sig("GET", "/", &[("prefix", "J"), ("max-keys", "2")], &[], EMPTY_SHA256);
    assert_eq!(sig, "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7");
}

// --- canonical forms ----------------------------------------------------------------------

#[test]
fn uri_encoding_edge_cases() {
    assert_eq!(uri_encode("AZaz09-_.~", false), "AZaz09-_.~");
    assert_eq!(uri_encode("a b+c=d&e/f", false), "a%20b%2Bc%3Dd%26e%2Ff");
    assert_eq!(uri_encode("2026/10/12/a b.png", true), "2026/10/12/a%20b.png");
    // UTF-8, byte by byte, uppercase hex.
    assert_eq!(uri_encode("ሴ/é", true), "%E1%88%B4/%C3%A9");
    assert_eq!(uri_encode("*'()!", false), "%2A%27%28%29%21");
    assert_eq!(uri_encode("%", false), "%25");
}

#[test]
fn queries_and_headers_canonicalise() {
    assert_eq!(canonical_query(&qs(&[("uploadId", "a/b=c"), ("partNumber", "2")])), "partNumber=2&uploadId=a%2Fb%3Dc");
    assert_eq!(canonical_query(&qs(&[("b", "2"), ("a", "1"), ("a", "0")])), "a=0&a=1&b=2");
    assert_eq!(canonical_query(&[]), "");
    let (block, names) = canonical_headers(&hs(&[("X-Amz-Meta-B", "  two   words "), ("Host", "h"), ("x-amz-meta-b", "again"), ("Content-Type", "image/png")]));
    assert_eq!(block, "content-type:image/png\nhost:h\nx-amz-meta-b:two words,again\n");
    assert_eq!(names, "content-type;host;x-amz-meta-b");
}

#[test]
fn amz_dates_are_utc() {
    assert_eq!(amz_date(0), "19700101T000000Z");
    assert_eq!(amz_date(1_440_938_160), "20150830T123600Z");
    assert_eq!(amz_date(1_369_353_600), "20130524T000000Z");
}

#[test]
fn creds_never_print_the_secret() {
    let d = format!("{S3EX:?}");
    assert!(d.contains("AKIAIOSFODNN7EXAMPLE") && !d.contains("wJalr"), "{d}");
}

// --- endpoints, keys, links --------------------------------------------------------------------

#[test]
fn endpoints_parse_to_a_host() {
    assert_eq!(parse_endpoint("https://abc123.r2.cloudflarestorage.com/"), Ok("abc123.r2.cloudflarestorage.com".into()));
    assert_eq!(parse_endpoint(" s3.eu-west-1.amazonaws.com "), Ok("s3.eu-west-1.amazonaws.com".into()));
    assert_eq!(parse_endpoint("HTTPS://MinIO.lan:9000"), Ok("minio.lan:9000".into()));
    // The default port is never in the signed Host (the transports leave it out).
    assert_eq!(parse_endpoint("https://s3.example.com:443/"), Ok("s3.example.com".into()));
    assert!(parse_endpoint("http://s3.amazonaws.com").unwrap_err().contains("https"));
    assert!(parse_endpoint("https://host/bucket").is_err());
    assert!(parse_endpoint("https://user@host").is_err());
    assert!(parse_endpoint("https://ho st").is_err());
    assert!(parse_endpoint("https://host:99999").is_err());
    assert!(parse_endpoint("").is_err());
    assert_eq!(split_port("minio.lan:9000"), ("minio.lan", 9000));
    assert_eq!(split_port("s3.amazonaws.com"), ("s3.amazonaws.com", 443));
}

#[test]
fn buckets_and_styles() {
    assert!(valid_bucket("my-shots.2026") && !valid_bucket("ab") && !valid_bucket("Caps") && !valid_bucket("-x-") && !valid_bucket("a_b"));
    let mut t = Target { endpoint: "https://acc.r2.cloudflarestorage.com".into(), region: String::new(), bucket: "shots".into(), path_style: true };
    assert_eq!(t.host_path("2026/a b.png"), Ok(("acc.r2.cloudflarestorage.com".into(), "/shots/2026/a%20b.png".into())));
    assert_eq!(t.region(), "auto");
    t.path_style = false;
    assert_eq!(t.host_path("k.png"), Ok(("shots.acc.r2.cloudflarestorage.com".into(), "/k.png".into())));
    assert_eq!(t.object_url("k.png").unwrap(), "https://shots.acc.r2.cloudflarestorage.com/k.png");
    // A dotted bucket would not match the endpoint's certificate as a
    // subdomain: always path-style.
    t.bucket = "shots.example.com".into();
    assert!(t.path_style());
    assert_eq!(t.host_path("k.png"), Ok(("acc.r2.cloudflarestorage.com".into(), "/shots.example.com/k.png".into())));
    t.bucket = String::new();
    assert_eq!(t.check(), Err("no bucket is set".into()));
}

#[test]
fn links_start_with_the_public_base() {
    let t = Target { endpoint: "https://acc.r2.cloudflarestorage.com".into(), region: "auto".into(), bucket: "shots".into(), path_style: true };
    assert_eq!(public_link("https://cdn.example.com/", &t, "2026/10/12/a b.png").unwrap(), "https://cdn.example.com/2026/10/12/a%20b.png");
    assert_eq!(public_link("pub-xyz.r2.dev", &t, "k.png").unwrap(), "https://pub-xyz.r2.dev/k.png");
    assert_eq!(public_link("", &t, "k.png").unwrap(), "https://acc.r2.cloudflarestorage.com/shots/k.png");
}

#[test]
fn keys_fill_in_the_prefix_tokens() {
    let now = (2026, 10, 12, 9, 5, 7);
    assert_eq!(object_key("%Y/%m/%d/", now, 0, "rustshot_2026-10-12.png", "a1b2c3"), "2026/10/12/rustshot_2026-10-12-a1b2c3.png");
    assert_eq!(object_key("shots/%F_%H/", now, 0, "x.mp4", "t"), "shots/2026-10-12_09/x-t.mp4");
    // Empty, `.` and `..` segments go; so do leading slashes and backslashes.
    assert_eq!(object_key("/../a//./b\\c/", now, 0, "x.png", ""), "a/bc/x.png");
    assert_eq!(object_key("", now, 0, "noext", "t"), "noext-t");
    assert_eq!(object_key("", now, 0, "../..", "t"), "capture-t");
    assert_eq!(object_key("%%Y/", now, 0, "x.gif", ""), "%Y/x.gif");
    assert_eq!(object_key("", now, 0, "/home/u/rec.mp4", ""), "rec.mp4");
}

#[test]
fn random_tails_are_128_bits_of_base32() {
    let a = random_tail();
    assert_eq!(a.len(), 26);
    assert!(a.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)));
    assert_ne!(random_tail(), random_tail());
    // RFC 4648 vectors (lowercase, no padding).
    assert_eq!(base32(b""), "");
    assert_eq!(base32(b"f"), "my");
    assert_eq!(base32(b"foobar"), "mzxw6ytboi");
    assert_eq!(base32(&[0xff; 16]).len(), 26);
}

#[test]
fn content_types() {
    assert_eq!(content_type("a.PNG"), "image/png");
    assert_eq!(content_type("a.jpeg"), "image/jpeg");
    assert_eq!(content_type("a.jpg"), "image/jpeg");
    assert_eq!(content_type("r.mp4"), "video/mp4");
    assert_eq!(content_type("r.gif"), "image/gif");
    assert_eq!(content_type("x"), "application/octet-stream");
}

// --- multipart planning ----------------------------------------------------------------------

#[test]
fn multipart_plans_equal_parts() {
    assert!(plan(0).is_empty());
    assert!(plan(MULTIPART_OVER).is_empty(), "64 MB exactly: one PUT");
    let p = plan(MULTIPART_OVER + 1);
    assert_eq!(p.len(), 5);
    assert!(p[..4].iter().all(|&(_, l)| l == PART_SIZE));
    assert_eq!(p[4], (4 * PART_SIZE, 1));
    // Contiguous and complete.
    let size = 1_000_000_007u64;
    let p = plan(size);
    let mut at = 0;
    for &(o, l) in &p {
        assert_eq!(o, at);
        assert!(l >= MIN_PART || o + l == size);
        at += l;
    }
    assert_eq!(at, size);
    // Huge: bigger parts, never more than 10,000, all but the last equal.
    let big = 400 * 1024 * 1024 * 1024u64;
    let p = plan(big);
    assert!(p.len() as u64 <= MAX_PARTS);
    assert!(p[0].1 > PART_SIZE && p[0].1.is_multiple_of(1024 * 1024));
    assert!(p[..p.len() - 1].iter().all(|x| x.1 == p[0].1));
    assert_eq!(p.iter().map(|x| x.1).sum::<u64>(), big);
}

// --- requests over a mock transport -------------------------------------------------------------

/// A recorded request.
#[derive(Clone, Debug)]
struct Sent {
    method: String,
    host: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Sent {
    fn header(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

/// A mock S3: records every request (reading the body as a transport
/// would) and answers with `answer`.
struct Mock {
    sent: RefCell<Vec<Sent>>,
    answer: Box<dyn Fn(&Sent) -> HttpResp>,
}

impl Mock {
    fn new(answer: impl Fn(&Sent) -> HttpResp + 'static) -> Mock {
        Mock { sent: RefCell::new(Vec::new()), answer: Box::new(answer) }
    }

    fn send(&self, r: &HttpReq) -> Result<HttpResp, String> {
        let mut body = Vec::new();
        r.body.for_each_chunk(1 << 16, &mut |c| {
            body.extend_from_slice(c);
            Ok(())
        })?;
        let s = Sent { method: r.method.into(), host: r.host.clone(), path: r.path.clone(), headers: r.headers.clone(), body };
        let a = (self.answer)(&s);
        self.sent.borrow_mut().push(s);
        Ok(a)
    }

    fn methods(&self) -> Vec<String> {
        self.sent.borrow().iter().map(|s| format!("{} {}", s.method, s.path)).collect()
    }
}

fn ok(status: u32) -> HttpResp {
    HttpResp { status, ..Default::default() }
}

fn r2() -> Target {
    Target { endpoint: "https://acc.r2.cloudflarestorage.com".into(), region: "auto".into(), bucket: "shots".into(), path_style: true }
}

const CLOCK: &dyn Fn() -> u64 = &|| 1_791_000_000;
const NOSLEEP: &dyn Fn(std::time::Duration) = &|_| {};

/// A small PUT is signed with its body's hash and the content type; the
/// signature verifies (recomputed from what was sent).
#[test]
fn a_picture_is_put_signed_with_its_hash() {
    let m = Mock::new(|_| ok(200));
    let t = r2();
    let send = |r: &HttpReq| m.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    c.put_object("2026/10/12/a.png", "image/png", Body::Bytes(b"PNGDATA")).unwrap();
    let s = m.sent.borrow()[0].clone();
    assert_eq!((s.method.as_str(), s.host.as_str(), s.path.as_str()), ("PUT", "acc.r2.cloudflarestorage.com", "/shots/2026/10/12/a.png"));
    assert_eq!(s.body, b"PNGDATA");
    assert_eq!(s.header("content-type"), Some("image/png"));
    let hash = crate::sha256::hex(&crate::sha256::digest(b"PNGDATA"));
    assert_eq!(s.header("x-amz-content-sha256"), Some(hash.as_str()));
    assert_eq!(s.header("x-amz-date"), Some(amz_date(1_791_000_000).as_str()));
    assert!(s.header("host").is_none(), "the transport sends Host");
    let auth = s.header("authorization").unwrap();
    assert!(auth.starts_with(&format!("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/{}/auto/s3/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, Signature=", &amz_date(1_791_000_000)[..8])), "{auth}");
    // Recompute it from the wire.
    let mut h: Vec<(String, String)> = s.headers.iter().filter(|(k, _)| k != "Authorization").cloned().collect();
    h.push(("host".into(), s.host.clone()));
    let canon = Canon { method: "PUT", path: &s.path, query: &[], headers: &h, payload_hash: &hash };
    assert!(auth.ends_with(&signature(S3EX, "auto", "s3", &amz_date(1_791_000_000), &canon)));
    assert!(!format!("{:?}", m.sent.borrow()).contains(S3EX.secret), "the secret is never sent");
}

/// A file is streamed unsigned; S3's error XML becomes the message.
#[test]
fn files_stream_unsigned_and_errors_read_well() {
    let dir = std::env::temp_dir().join(format!("rustshot-s3-test-{}", random_tail()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("rec.mp4");
    std::fs::write(&f, b"0123456789abcdef").unwrap();
    let m = Mock::new(|_| HttpResp { status: 403, body: "<?xml version=\"1.0\"?><Error><Code>SignatureDoesNotMatch</Code><Message>The request signature we calculated does not match.</Message></Error>".into(), ..Default::default() });
    let t = Target { path_style: false, ..r2() };
    let send = |r: &HttpReq| m.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    let e = c.upload_file(&f, 16, "rec.mp4", "video/mp4", &mut None).unwrap_err();
    assert_eq!(e, "The request signature we calculated does not match (SignatureDoesNotMatch)");
    assert_eq!(m.methods().len(), 1, "a refusal is not retried");
    let s = m.sent.borrow()[0].clone();
    assert_eq!((s.host.as_str(), s.path.as_str()), ("shots.acc.r2.cloudflarestorage.com", "/rec.mp4"));
    assert_eq!(s.header("x-amz-content-sha256"), Some(UNSIGNED));
    assert_eq!(s.body, b"0123456789abcdef");
    assert_eq!(failure(&ok(500)), "HTTP 500");
    let _ = std::fs::remove_dir_all(&dir);
}

fn big_file(tag: &str, size: u64) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("rustshot-s3-test-{tag}-{}", random_tail()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("big.mp4");
    std::fs::File::create(&f).unwrap().set_len(size).unwrap();
    (dir, f)
}

/// The multipart flow on a file just over the threshold: create, each
/// part with its range, complete with the ETags in order.
#[test]
fn a_large_file_goes_up_in_parts() {
    let size = MULTIPART_OVER + 3;
    let (dir, f) = big_file("parts", size);
    let m = Mock::new(|s| {
        if s.method == "POST" && s.path.ends_with("?uploads=") {
            HttpResp { status: 200, body: "<InitiateMultipartUploadResult><Bucket>shots</Bucket><Key>big.mp4</Key><UploadId>up/1&amp;2</UploadId></InitiateMultipartUploadResult>".into(), ..Default::default() }
        } else if s.method == "PUT" {
            let n = s.path.split("partNumber=").nth(1).unwrap().split('&').next().unwrap().to_string();
            HttpResp { status: 200, headers: vec![("ETag".into(), format!("\"etag{n}\""))], ..Default::default() }
        } else {
            HttpResp { status: 200, body: "<CompleteMultipartUploadResult><ETag>\"x-5\"</ETag></CompleteMultipartUploadResult>".into(), ..Default::default() }
        }
    });
    let t = r2();
    let send = |r: &HttpReq| m.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    let mut partial = None;
    c.upload_file(&f, size, "big.mp4", "video/mp4", &mut partial).unwrap();
    assert_eq!(partial, None);
    let methods = m.methods();
    assert_eq!(methods.len(), 1 + 5 + 1, "{methods:?}");
    assert_eq!(methods[0], "POST /shots/big.mp4?uploads=");
    assert_eq!(methods[1], "PUT /shots/big.mp4?partNumber=1&uploadId=up%2F1%262");
    assert_eq!(methods[6], "POST /shots/big.mp4?uploadId=up%2F1%262");
    let sent = m.sent.borrow();
    assert_eq!(sent[0].header("content-type"), Some("video/mp4"));
    assert_eq!(sent[1].body.len() as u64, PART_SIZE);
    assert_eq!(sent[5].body.len(), 3, "the last part is the rest");
    assert!(sent[1..6].iter().all(|s| s.header("x-amz-content-sha256") == Some(UNSIGNED)));
    let xml = String::from_utf8(sent[6].body.clone()).unwrap();
    assert_eq!(xml, complete_xml(&(1..=5).map(|n| format!("\"etag{n}\"")).collect::<Vec<_>>()));
    assert!(xml.starts_with("<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"etag1\"</ETag></Part>"));
    let hash = crate::sha256::hex(&crate::sha256::digest(xml.as_bytes()));
    assert_eq!(sent[6].header("x-amz-content-sha256"), Some(hash.as_str()), "the small XML is signed");
    drop(sent);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Transport errors, 5xx and SlowDown are tried again (signed afresh,
/// waiting longer each time); after the last try the error stands. A
/// refusal is not retried.
#[test]
fn transient_failures_are_retried_with_backoff() {
    let n = RefCell::new(0u32);
    let clock_n = RefCell::new(1_791_000_000u64);
    let flaky = |r: &HttpReq| {
        *n.borrow_mut() += 1;
        let k = *n.borrow();
        let _ = r;
        match k {
            1 => Err("connection reset".to_string()),
            2 => Ok(HttpResp { status: 503, body: "<Error><Code>SlowDown</Code><Message>Please reduce your request rate.</Message></Error>".into(), ..Default::default() }),
            _ => Ok(ok(200)),
        }
    };
    let waits = RefCell::new(Vec::new());
    let sleep = |d: std::time::Duration| waits.borrow_mut().push(d);
    let clock = || {
        *clock_n.borrow_mut() += 60;
        *clock_n.borrow()
    };
    let t = r2();
    let c = Client { target: &t, creds: S3EX, send: &flaky, clock: &clock, sleep: &sleep };
    c.put_object("k.png", "image/png", Body::Bytes(b"x")).unwrap();
    assert_eq!(*n.borrow(), 3);
    let w = waits.borrow().clone();
    assert_eq!(w.len(), 2);
    assert!(w[0] >= BACKOFF && w[0] < BACKOFF * 2, "{w:?}");
    assert!(w[1] >= BACKOFF * 2 && w[1] < BACKOFF * 4, "{w:?}");

    // Always failing: ATTEMPTS tries, then the error.
    let tries = RefCell::new(0u32);
    let down = |_: &HttpReq| {
        *tries.borrow_mut() += 1;
        Ok(HttpResp { status: 500, body: "<Error><Code>InternalError</Code><Message>We encountered an internal error.</Message></Error>".into(), ..Default::default() })
    };
    let c = Client { target: &t, creds: S3EX, send: &down, clock: CLOCK, sleep: NOSLEEP };
    assert_eq!(c.delete_object("k"), Err("We encountered an internal error (InternalError)".into()));
    assert_eq!(*tries.borrow(), ATTEMPTS);

    // Each try is signed when it is sent (a new date, a new signature).
    let seen = RefCell::new(Vec::new());
    let k = RefCell::new(0);
    let record = |r: &HttpReq| {
        seen.borrow_mut().push(r.headers.iter().find(|(h, _)| h == "Authorization").unwrap().1.clone());
        *k.borrow_mut() += 1;
        Ok(if *k.borrow() == 1 { ok(502) } else { ok(200) })
    };
    let c = Client { target: &t, creds: S3EX, send: &record, clock: &clock, sleep: NOSLEEP };
    c.put_object("k", "text/plain", Body::Bytes(b"y")).unwrap();
    let s = seen.borrow();
    assert_eq!(s.len(), 2);
    assert_ne!(s[0], s[1], "re-signed");
    assert!(retryable(&Ok(ok(429))) && !retryable(&Ok(ok(403))) && !retryable(&Ok(ok(200))));
}

/// A part that keeps failing with a server error leaves the upload for
/// the next try (not aborted); the next try lists the parts there and
/// sends only the missing ones. A refused part aborts.
#[test]
fn a_failed_multipart_upload_resumes_or_aborts() {
    let size = MULTIPART_OVER * 2; // 8 parts
    let (dir, f) = big_file("resume", size);
    let fail3 = std::rc::Rc::new(std::cell::Cell::new(true));
    let m = Mock::new({
        let fail3 = fail3.clone();
        move |s| {
            if s.path.ends_with("?uploads=") {
                HttpResp { status: 200, body: "<UploadId>U</UploadId>".into(), ..Default::default() }
            } else if s.method == "GET" {
                // Parts 1 and 2 are there (part 2 with a wrong size: sent again).
                let p = |n: u64, size: u64| format!("<Part><PartNumber>{n}</PartNumber><ETag>\"have{n}\"</ETag><Size>{size}</Size></Part>");
                HttpResp { status: 200, body: format!("<ListPartsResult><IsTruncated>false</IsTruncated>{}{}</ListPartsResult>", p(1, PART_SIZE), p(2, 5)), ..Default::default() }
            } else if s.method == "PUT" && s.path.contains("partNumber=3&") && fail3.get() {
                HttpResp { status: 500, body: "<Error><Code>InternalError</Code><Message>We encountered an internal error.</Message></Error>".into(), ..Default::default() }
            } else if s.method == "PUT" {
                HttpResp { status: 200, headers: vec![("etag".into(), "\"e\"".into())], ..Default::default() }
            } else if s.method == "POST" {
                ok(200)
            } else {
                ok(204)
            }
        }
    });
    let t = r2();
    let send = |r: &HttpReq| m.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    let mut partial = None;
    let e = c.upload_file(&f, size, "big.mp4", "video/mp4", &mut partial).unwrap_err();
    assert!(e.starts_with("part 3 of 8: We encountered an internal error"), "{e}");
    let methods = m.methods();
    assert_eq!(methods.len(), 1 + 2 + ATTEMPTS as usize, "part 3 tried {ATTEMPTS} times: {methods:?}");
    assert!(!methods.iter().any(|x| x.starts_with("DELETE")), "kept for the next try");
    assert_eq!(partial, Some(Partial { key: "big.mp4".into(), upload_id: "U".into(), size }));

    // The next try: ListParts, then parts 2..8 (2 had the wrong size), then complete.
    fail3.set(false);
    m.sent.borrow_mut().clear();
    c.upload_file(&f, size, "big.mp4", "video/mp4", &mut partial).unwrap();
    let methods = m.methods();
    assert_eq!(methods[0], "GET /shots/big.mp4?uploadId=U");
    assert!(!methods.iter().any(|x| x.ends_with("?uploads=")), "no new upload");
    assert!(!methods.iter().any(|x| x.contains("partNumber=1&")), "part 1 reused");
    assert_eq!(methods.iter().filter(|x| x.starts_with("PUT")).count(), 7);
    let xml = String::from_utf8(m.sent.borrow().last().unwrap().body.clone()).unwrap();
    assert!(xml.starts_with("<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"have1\"</ETag>"), "{xml}");
    assert_eq!(partial, None);

    // A refused part: aborted, nothing kept.
    let refused = Mock::new(|s| {
        if s.path.ends_with("?uploads=") {
            HttpResp { status: 200, body: "<UploadId>V</UploadId>".into(), ..Default::default() }
        } else if s.method == "PUT" {
            HttpResp { status: 403, body: "<Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>".into(), ..Default::default() }
        } else {
            ok(204)
        }
    });
    let send = |r: &HttpReq| refused.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    let mut partial = None;
    assert!(c.upload_file(&f, size, "big.mp4", "video/mp4", &mut partial).unwrap_err().contains("AccessDenied"));
    assert_eq!(refused.methods().last().unwrap(), "DELETE /shots/big.mp4?uploadId=V");
    assert_eq!(partial, None);

    // An upload id that is gone (completed, aborted, expired): start over.
    let gone = Mock::new(|s| {
        if s.method == "GET" {
            HttpResp { status: 404, body: "<Error><Code>NoSuchUpload</Code></Error>".into(), ..Default::default() }
        } else if s.path.ends_with("?uploads=") {
            HttpResp { status: 200, body: "<UploadId>W</UploadId>".into(), ..Default::default() }
        } else if s.method == "PUT" {
            HttpResp { status: 200, headers: vec![("etag".into(), "\"e\"".into())], ..Default::default() }
        } else {
            ok(200)
        }
    });
    let send = |r: &HttpReq| gone.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    let mut partial = Some(Partial { key: "big.mp4".into(), upload_id: "OLD".into(), size });
    c.upload_file(&f, size, "big.mp4", "video/mp4", &mut partial).unwrap();
    let ms = gone.methods();
    assert_eq!((ms[0].as_str(), ms[1].as_str()), ("GET /shots/big.mp4?uploadId=OLD", "POST /shots/big.mp4?uploads="));
    assert_eq!(ms.iter().filter(|x| x.starts_with("PUT")).count(), 8);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An error that comes with a 200 on complete (and stays) fails the
/// upload after its retries.
#[test]
fn an_error_with_200_on_complete_fails() {
    let size = MULTIPART_OVER + 1;
    let (dir, f) = big_file("complete", size);
    let m = Mock::new(|s| {
        if s.path.ends_with("?uploads=") {
            HttpResp { status: 200, body: "<UploadId>U</UploadId>".into(), ..Default::default() }
        } else if s.method == "PUT" {
            HttpResp { status: 200, headers: vec![("etag".into(), "\"e\"".into())], ..Default::default() }
        } else if s.method == "POST" {
            HttpResp { status: 200, body: "<Error><Code>InternalError</Code><Message>Try again</Message></Error>".into(), ..Default::default() }
        } else {
            ok(204)
        }
    });
    let t = r2();
    let send = |r: &HttpReq| m.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    let mut partial = None;
    assert_eq!(c.upload_file(&f, size, "big.mp4", "video/mp4", &mut partial), Err("Try again (InternalError)".into()));
    assert_eq!(m.methods().iter().filter(|x| x.starts_with("POST /shots/big.mp4?uploadId=")).count(), ATTEMPTS as usize);
    assert!(partial.is_some(), "kept: the parts are there");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_parts_pages_and_parses() {
    let body = "<ListPartsResult><Part><PartNumber>2</PartNumber><ETag>&quot;b&quot;</ETag><Size>10</Size></Part><Part><PartNumber>1</PartNumber><ETag>\"a\"</ETag><Size>16</Size></Part></ListPartsResult>";
    assert_eq!(parse_parts(body), vec![(2, "\"b\"".to_string(), 10), (1, "\"a\"".to_string(), 16)]);
    let page = RefCell::new(0);
    let m = Mock::new(move |s| {
        *page.borrow_mut() += 1;
        if s.path.contains("part-number-marker=1") {
            HttpResp { status: 200, body: "<IsTruncated>false</IsTruncated><Part><PartNumber>2</PartNumber><ETag>\"b\"</ETag><Size>5</Size></Part>".into(), ..Default::default() }
        } else {
            HttpResp { status: 200, body: "<IsTruncated>true</IsTruncated><NextPartNumberMarker>1</NextPartNumberMarker><Part><PartNumber>1</PartNumber><ETag>\"a\"</ETag><Size>5</Size></Part>".into(), ..Default::default() }
        }
    });
    let t = r2();
    let send = |r: &HttpReq| m.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    assert_eq!(c.list_parts("k", "U").unwrap().unwrap().len(), 2);
}

/// Test connection: PUT then DELETE `rustshot-test-<tail>.txt`; delete
/// accepts 204 (and a 404: already gone).
#[test]
fn test_connection_puts_then_deletes() {
    let m = Mock::new(|s| ok(if s.method == "DELETE" { 204 } else { 200 }));
    let t = r2();
    let send = |r: &HttpReq| m.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    c.test_connection("k3y").unwrap();
    assert_eq!(m.methods(), ["PUT /shots/rustshot-test-k3y.txt", "DELETE /shots/rustshot-test-k3y.txt"]);
    assert_eq!(m.sent.borrow()[0].header("content-type"), Some("text/plain"));

    let denied = Mock::new(|_| HttpResp { status: 403, body: "<Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>".into(), ..Default::default() });
    let send = |r: &HttpReq| denied.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    assert_eq!(c.test_connection("x"), Err("upload failed: Access Denied (AccessDenied)".into()));
    assert_eq!(denied.methods().len(), 1, "no delete after a refused upload");

    let gone = Mock::new(|_| ok(404));
    let send = |r: &HttpReq| gone.send(r);
    let c = Client { target: &t, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    assert_eq!(c.delete_object("k"), Ok(()));
}

/// Nothing is sent without a key, a secret or a valid target.
#[test]
fn incomplete_settings_send_nothing() {
    let m = Mock::new(|_| panic!("sent"));
    let send = |r: &HttpReq| m.send(r);
    let t = r2();
    let c = Client { target: &t, creds: Creds { access_key: "AK", secret: "" }, send: &send, clock: CLOCK, sleep: NOSLEEP };
    assert!(c.put_object("k", "text/plain", Body::Empty).unwrap_err().contains("secret"));
    let c = Client { target: &t, creds: Creds { access_key: " ", secret: "s" }, send: &send, clock: CLOCK, sleep: NOSLEEP };
    assert!(c.put_object("k", "text/plain", Body::Empty).unwrap_err().contains("access key"));
    let bad = Target { endpoint: "http://insecure".into(), ..r2() };
    let c = Client { target: &bad, creds: S3EX, send: &send, clock: CLOCK, sleep: NOSLEEP };
    assert!(c.delete_object("k").unwrap_err().contains("https"));
}

/// The whole upload with the config's values: the key under the prefix,
/// the link under the public base, the object (and the access key id)
/// for the record.
#[test]
fn uploads_use_the_config() {
    let cfg = Config {
        upload_destination: "s3".into(),
        s3_endpoint: "https://acc.r2.cloudflarestorage.com".into(),
        s3_bucket: "shots".into(),
        s3_access_key_id: " AKID ".into(),
        s3_public_url: "https://cdn.example.com".into(),
        s3_key_prefix: "caps/%Y/".into(),
        ..Config::default()
    };
    assert_eq!(Dest::of(&cfg), Dest::S3);
    assert_eq!(Dest::of(&Config::default()), Dest::Imgur);
    let m = Mock::new(|_| ok(200));
    let send = |r: &HttpReq| m.send(r);
    let io = Io { send: &send, clock: CLOCK, sleep: NOSLEEP };
    let (link, obj) = upload(&cfg, Source::Bytes(b"PNG", "shot.png"), "secret", &io).unwrap();
    let year = crate::export::local_ymdhms().0;
    assert!(obj.key.starts_with(&format!("caps/{year}/shot-")) && obj.key.ends_with(".png"), "{}", obj.key);
    assert_eq!(obj.key.len(), format!("caps/{year}/shot-").len() + 26 + 4, "128 random bits");
    assert_eq!(link, format!("https://cdn.example.com/{}", obj.key));
    assert_eq!(obj.target, Target::from_config(&cfg));
    assert_eq!(obj.access_key, "AKID");
    let s = m.sent.borrow()[0].clone();
    assert_eq!(s.path, format!("/shots/{}", obj.key));
    assert!(s.header("authorization").unwrap().contains("Credential=AKID/"));
}

/// Delete upload… signs with the access key id the record kept and the
/// secret stored for it; a record without one falls back to the current
/// id and says so when that fails.
#[test]
fn deletes_use_the_records_credentials() {
    let t = Target { endpoint: "https://del.example.com".into(), region: "auto".into(), bucket: "rs-del".into(), path_style: true };
    crate::keystore::os().set("rustshot:s3:rs-del@del.example.com/AKOLD", "", "old-secret").unwrap();
    let m = Mock::new(|_| ok(204));
    let send = |r: &HttpReq| m.send(r);
    let io = Io { send: &send, clock: CLOCK, sleep: NOSLEEP };
    let obj = Object { target: t.clone(), key: "k.png".into(), access_key: "AKOLD".into() };
    delete_now(&obj, "AKNEW", &io).unwrap();
    assert!(m.sent.borrow()[0].header("authorization").unwrap().contains("Credential=AKOLD/"));
    let legacy = Object { access_key: String::new(), ..obj };
    let e = delete_now(&legacy, "AKNEW", &io).unwrap_err();
    assert!(e.contains("AKNEW") && e.contains("the current one was used"), "{e}");
}

/// A recording whose multipart upload failed goes on under the same key
/// when it is uploaded again (the next Copy link).
#[test]
fn a_recording_upload_resumes_under_the_same_key() {
    let size = MULTIPART_OVER + 1;
    let (dir, f) = big_file("app-resume", size);
    let cfg = Config {
        upload_destination: "s3".into(),
        s3_endpoint: "https://acc.r2.cloudflarestorage.com".into(),
        s3_bucket: "resume-shots".into(),
        s3_access_key_id: "AKID".into(),
        ..Config::default()
    };
    let up = std::rc::Rc::new(std::cell::Cell::new(false));
    let m = Mock::new({
        let up = up.clone();
        move |s| {
            if s.path.ends_with("?uploads=") {
                HttpResp { status: 200, body: "<UploadId>R</UploadId>".into(), ..Default::default() }
            } else if s.method == "GET" {
                HttpResp { status: 200, body: "<IsTruncated>false</IsTruncated>".into(), ..Default::default() }
            } else if s.method == "PUT" && !up.get() {
                HttpResp { status: 503, ..Default::default() }
            } else if s.method == "PUT" {
                HttpResp { status: 200, headers: vec![("etag".into(), "\"e\"".into())], ..Default::default() }
            } else {
                ok(200)
            }
        }
    });
    let send = |r: &HttpReq| m.send(r);
    let io = Io { send: &send, clock: CLOCK, sleep: NOSLEEP };
    assert!(upload(&cfg, Source::File(&f), "secret", &io).is_err());
    let first_key = m.sent.borrow()[0].path.clone();
    up.set(true);
    m.sent.borrow_mut().clear();
    let (link, obj) = upload(&cfg, Source::File(&f), "secret", &io).unwrap();
    let ms = m.methods();
    assert!(ms[0].starts_with("GET ") && ms[0].ends_with("?uploadId=R"), "{ms:?}");
    assert_eq!(first_key.split('?').next(), Some(format!("/resume-shots/{}", obj.key).as_str()));
    assert!(link.ends_with(&obj.key));
    let _ = std::fs::remove_dir_all(&dir);
}

// Body chunks: bytes split, a file range read, a short file an error.
#[test]
fn bodies_come_in_chunks() {
    let mut got = Vec::new();
    Body::Bytes(b"abcdefg").for_each_chunk(3, &mut |c| {
        got.push(c.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(got, [b"abc".to_vec(), b"def".to_vec(), b"g".to_vec()]);
    let dir = std::env::temp_dir().join(format!("rustshot-s3-test-{}", random_tail()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("f.bin");
    std::fs::write(&f, b"0123456789").unwrap();
    let mut all = Vec::new();
    Body::File { path: &f, offset: 3, len: 4 }
        .for_each_chunk(3, &mut |c| {
            all.extend_from_slice(c);
            Ok(())
        })
        .unwrap();
    assert_eq!(all, b"3456");
    assert!(Body::File { path: &f, offset: 8, len: 4 }.for_each_chunk(3, &mut |_| Ok(())).unwrap_err().contains("ended early"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn header_blocks_parse() {
    let h = parse_header_block("HTTP/1.1 200 OK\r\nETag: \"abc\"\r\nx-amz-id-2: a:b\r\n\r\n");
    assert_eq!(h, vec![("ETag".to_string(), "\"abc\"".to_string()), ("x-amz-id-2".to_string(), "a:b".to_string())]);
}
