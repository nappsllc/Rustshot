//! Release-asset download via `curl`, shared by the Linux and macOS
//! backends (both already shell out to curl for HTTP).

use crate::update::{CANCELLED, is_allowed_download_hop, is_safe_download_url};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Download a release asset to `dest`. Only an `update::is_safe_download_url`
/// start is accepted; curl follows up to 5 https-only redirects and the final
/// URL must pass `update::is_allowed_download_hop`. Progress is the growing
/// size of `dest`, polled while curl runs (total unknown); `progress`
/// returning false kills curl (error `update::CANCELLED`). A partial file is
/// deleted on any failure.
pub fn download_to(
    url: &str,
    dest: &Path,
    progress: &dyn Fn(u64, Option<u64>) -> bool,
) -> Result<(), String> {
    if !is_safe_download_url(url) {
        return Err("refusing to download from an unexpected URL".into());
    }
    let https = ["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--max-redirs", "5"];
    let r = run_curl(url, dest, &https, progress, &is_allowed_download_hop);
    if r.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    r
}

/// Download exactly `url` (plain HTTP, no redirects, at most `max` bytes)
/// to `dest`: for Cisco's OpenH264 binaries, which Cisco serves over HTTP
/// only. The caller checks the URL is pinned and verifies the file's
/// SHA-256 before using it. A partial file is deleted on any failure.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // the Linux encoder without ffmpeg
pub fn download_http_exact(
    url: &str,
    dest: &Path,
    max: u64,
    progress: &dyn Fn(u64, Option<u64>) -> bool,
) -> Result<(), String> {
    if !url.starts_with("http://") || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("refusing to download from an unexpected URL".into());
    }
    let max = max.to_string();
    // A stalled download (under 1 KB/s for 30 s) or one past 5 minutes
    // is given up.
    let args = [
        "-fsS", "--proto", "=http", "--max-redirs", "0", "--max-filesize", &max, "--speed-limit", "1024", "--speed-time", "30", "--max-time", "300",
    ];
    let r = run_curl(url, dest, &args, progress, &|eff| eff == url);
    if r.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    r
}

fn run_curl(
    url: &str,
    dest: &Path,
    args: &[&str],
    progress: &dyn Fn(u64, Option<u64>) -> bool,
    allowed_end: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    // A stale file must not be reused (curl would truncate it in place).
    let _ = std::fs::remove_file(dest);
    let mut child = Command::new("curl")
        .arg("-q") // first argument: ignore ~/.curlrc
        .args(args)
        .args(["--connect-timeout", "15", "-w", "%{url_effective}", "-o"])
        .arg(dest)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn curl: {e}"))?;
    let size = || std::fs::metadata(dest).map_or(0, |m| m.len());
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("curl: {e}"));
            }
        }
        if !progress(size(), None) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CANCELLED.into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // stdout/stderr are tiny (-s, -w), so reading after exit cannot deadlock.
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let effective = String::from_utf8_lossy(&out.stdout);
    if !allowed_end(effective.trim()) {
        return Err("refusing a download redirected to an unexpected host".into());
    }
    let got = size();
    if !progress(got, Some(got)) {
        return Err(CANCELLED.into());
    }
    Ok(())
}

/// HTTPS DELETE via `curl`; (status, body). The URL and headers go on
/// curl's stdin (`-K -`), not its command line: the path holds an
/// upload's deletehash, which other users' process lists must not show.
#[cfg_attr(test, allow(dead_code))] // tests never delete
pub fn http_delete(host: &str, path: &str, headers: &[(&str, &str)]) -> Result<(u32, String), String> {
    use std::io::Write;
    let conf = curl_config(host, path, headers);
    let mut child = Command::new("curl")
        .args(["-sS", "--max-time", "30", "-X", "DELETE", "-w", "\n%{http_code}", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn curl: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or_else(|| "curl stdin".to_string())?;
        stdin.write_all(conf.as_bytes()).map_err(|e| format!("write curl stdin: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    Ok((code.trim().parse().unwrap_or(0), body.to_string()))
}

/// One HTTPS request for S3 (`crate::s3`) via `curl`. The URL and headers
/// go in a config file only we can read (`-K`), never on the command line
/// (the `Authorization` header is a live signature other users' process
/// lists must not show); the body goes on stdin, fed from a thread as
/// curl reads it (`--data-binary @-`); `-i` prints the response headers
/// (a part's `ETag`) before the body. No redirects are followed.
#[cfg_attr(test, allow(dead_code))] // tests use a mock transport
pub fn http_request(r: &crate::s3::HttpReq) -> Result<crate::s3::HttpResp, String> {
    use std::io::Write;
    let headers: Vec<(&str, &str)> = r
        .headers
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("host") && !k.eq_ignore_ascii_case("content-length"))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut conf = curl_config(&r.host, &r.path, &headers);
    conf.push_str(&format!("request = \"{}\"\n", r.method));
    let send_body = matches!(r.method, "PUT" | "POST");
    if send_body && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
        // curl would add a form content type of its own.
        conf.push_str("header = \"Content-Type:\"\n");
    }
    let conf_path = private_conf(&conf)?;
    let mut cmd = Command::new("curl");
    cmd.args(["-q", "-sS", "-i", "--proto", "=https", "--max-redirs", "0", "--connect-timeout", "15", "--max-time", "1800", "-w", "\n%{http_code}", "-K"]).arg(&conf_path);
    if send_body {
        cmd.args(["--data-binary", "@-"]);
    }
    let spawned = cmd.stdin(if send_body { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&conf_path);
            return Err(format!("spawn curl: {e}"));
        }
    };
    let stdin = child.stdin.take();
    let out = std::thread::scope(|s| {
        if let Some(mut stdin) = stdin {
            // Fed on a thread: curl may answer (an error) before it read it all.
            s.spawn(move || {
                let _ = r.body.for_each_chunk(1 << 20, &mut |c| stdin.write_all(c).map_err(|e| e.to_string()));
            });
        }
        child.wait_with_output()
    });
    // curl has read its config long before it exits.
    let _ = std::fs::remove_file(&conf_path);
    let out = out.map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(parse_included(&String::from_utf8_lossy(&out.stdout)))
}

/// curl's `-i` output with `-w "\n%{http_code}"` after it: the last header
/// block (after any `100 Continue`), the body, the status.
pub fn parse_included(text: &str) -> crate::s3::HttpResp {
    let (mut rest, code) = text.rsplit_once('\n').unwrap_or(("", text));
    let mut headers = Vec::new();
    while rest.starts_with("HTTP/") {
        let (block, after) = match rest.find("\r\n\r\n") {
            Some(i) => (&rest[..i], &rest[i + 4..]),
            None => match rest.find("\n\n") {
                Some(i) => (&rest[..i], &rest[i + 2..]),
                None => (rest, ""),
            },
        };
        headers = crate::s3::parse_header_block(block);
        rest = after;
    }
    crate::s3::HttpResp { status: code.trim().parse().unwrap_or(0), headers, body: rest.to_string() }
}

/// Write `conf` to a new file only this user can read (in
/// `$XDG_RUNTIME_DIR`, else the temp folder); its path.
fn private_conf(conf: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).filter(|d| d.is_dir()).unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("rustshot-curl-{}-{}.conf", std::process::id(), crate::s3::random_tail(10)));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|e| format!("write {}: {e}", path.display()))?;
    if let Err(e) = f.write_all(conf.as_bytes()) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("write {}: {e}", path.display()));
    }
    Ok(path)
}

/// The `-K -` config for a request: its URL and headers, quoted (a line
/// break would end the line and start another option: dropped).
fn curl_config(host: &str, path: &str, headers: &[(&str, &str)]) -> String {
    let q = |s: &str| s.replace(['\r', '\n'], "").replace('\\', r"\\").replace('"', r#"\""#);
    let mut conf = format!("url = \"https://{}{}\"\n", q(host), q(path));
    for (k, v) in headers {
        conf.push_str(&format!("header = \"{}: {}\"\n", q(k), q(v)));
    }
    conf
}

#[cfg(test)]
mod tests {
    #[test]
    fn curl_config_is_quoted_line_by_line() {
        let c = super::curl_config("api.imgur.com", "/3/image/abc12", &[("Authorization", "Client-ID x\"y\nurl = \"https://evil\"")]);
        assert_eq!(c.lines().count(), 2, "{c}");
        assert!(c.starts_with("url = \"https://api.imgur.com/3/image/abc12\"\n"));
        assert!(c.contains(r#"Client-ID x\"yurl = \"https://evil\""#), "{c}");
    }

    /// `-i` output: a 100 Continue block, then the real one, the body and
    /// the `-w` status line.
    #[test]
    fn included_headers_are_split_from_the_body() {
        let t = "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nETag: \"abc\"\r\nContent-Length: 2\r\n\r\nok\n200";
        let r = super::parse_included(t);
        assert_eq!((r.status, r.header("etag"), r.body.as_str()), (200, Some("\"abc\""), "ok"));
        let r = super::parse_included("HTTP/2 204 \r\nx-amz-request-id: 1\r\n\r\n\n204");
        assert_eq!((r.status, r.body.as_str(), r.header("X-Amz-Request-Id")), (204, "", Some("1")));
    }
}
