//! Linux (and other Unix): the `tesseract` command, if it is on `PATH`.
//! The PNG goes in on stdin, the text comes back on stdout
//! (`tesseract - - --dpi N`); Rustshot never installs it. A run that
//! takes longer than [`super::TIMEOUT`] is killed.

use super::OcrError;
use crate::pixbuf::PixBuf;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The program looked up on `PATH`.
pub const PROGRAM: &str = "tesseract";

/// The arguments: stdin to stdout, at `dpi` (a PNG carries none, and
/// tesseract would assume 70 and read small text worse).
pub fn args(dpi: u32) -> Vec<String> {
    vec!["-".into(), "-".into(), "--dpi".into(), dpi.clamp(70, 2400).to_string()]
}

/// The resolution of an image upscaled `k` times (screen px at 96 dpi).
pub fn dpi_for(k: u32) -> u32 {
    96 * k.max(1)
}

/// The words with their boxes: tesseract's `tsv` output.
pub fn recognize_lines(img: &PixBuf) -> Result<Vec<super::Line>, OcrError> {
    let (img, k) = super::prepare(img, None);
    let png = img.to_png().map_err(|e| OcrError::Failed(format!("{e:#}")))?;
    let mut a = args(dpi_for(k));
    a.push("tsv".into());
    let out = run(PROGRAM, &a, png, super::TIMEOUT)?;
    Ok(super::tsv::parse_tsv(&out, k))
}

pub fn recognize(img: &PixBuf) -> Result<String, OcrError> {
    let (img, k) = super::prepare(img, None);
    let png = img.to_png().map_err(|e| OcrError::Failed(format!("{e:#}")))?;
    let out = run(PROGRAM, &args(dpi_for(k)), png, super::TIMEOUT)?;
    Ok(super::tidy_text(&out))
}

/// Run `program args` with `input` on stdin and return its stdout,
/// killing it after `timeout`. A missing program is
/// [`OcrError::NoTesseract`].
pub fn run(program: &str, args: &[String], input: Vec<u8>, timeout: Duration) -> Result<String, OcrError> {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(OcrError::NoTesseract),
        Err(e) => return Err(OcrError::Failed(format!("cannot run {program}: {e}"))),
    };
    // stdin, stdout and stderr each on their own thread, so no full pipe
    // can stall the child or us; a killed child closes them all.
    let feeder = child.stdin.take().map(|mut stdin| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let drain = |p: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            if let Some(mut p) = p {
                let _ = p.read_to_end(&mut v);
            }
            v
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Ok(st),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(OcrError::TimedOut);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(OcrError::Failed(e.to_string()));
            }
        }
    };
    if let Some(f) = feeder {
        let _ = f.join();
    }
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    let status = status?;
    if !status.success() {
        let err = String::from_utf8_lossy(&err);
        let why = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("tesseract failed").trim().to_string();
        return Err(OcrError::Failed(why));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tesseract_reads_stdin_and_writes_stdout() {
        assert_eq!(args(96), ["-", "-", "--dpi", "96"]);
        assert_eq!(args(dpi_for(2)), ["-", "-", "--dpi", "192"]);
        assert_eq!(args(1), ["-", "-", "--dpi", "70"], "tesseract's floor");
        assert_eq!(dpi_for(0), 96);
        assert_eq!(PROGRAM, "tesseract");
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["-c".into(), script.into()]
    }

    #[test]
    fn the_command_gets_stdin_and_its_stdout_comes_back() {
        let r = run("sh", &sh("cat; echo done"), b"Hello Rustshot 123\n".to_vec(), Duration::from_secs(10));
        assert_eq!(r, Ok("Hello Rustshot 123\ndone\n".to_string()));
        let r = run("sh", &sh("echo 'Error: no tessdata' >&2; exit 1"), Vec::new(), Duration::from_secs(10));
        assert_eq!(r, Err(OcrError::Failed("Error: no tessdata".into())));
        let r = run("rustshot-no-such-ocr-program", &[], Vec::new(), Duration::from_secs(10));
        assert_eq!(r, Err(OcrError::NoTesseract));
    }

    /// A hung command is killed at the deadline (and does not take its
    /// own time): "Text recognition timed out".
    #[test]
    fn a_hung_command_is_killed_at_the_deadline() {
        let t0 = Instant::now();
        // It never reads stdin: a big input must not block the kill either.
        let r = run("sh", &sh("exec sleep 30"), vec![0u8; 1 << 20], Duration::from_millis(300));
        assert_eq!(r, Err(OcrError::TimedOut));
        assert!(t0.elapsed() < Duration::from_secs(10), "{:?}", t0.elapsed());
        assert_eq!(super::super::note(&r, Ok(())).text, "Text recognition timed out");
    }
}
