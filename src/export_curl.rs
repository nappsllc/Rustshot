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
    let args = ["-fsS", "--proto", "=http", "--max-redirs", "0", "--max-filesize", &max];
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
