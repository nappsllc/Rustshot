//! Linux (and other Unix): the `tesseract` command, if it is on `PATH`.
//! The PNG goes in on stdin, the text comes back on stdout
//! (`tesseract - - --dpi N`); Rustshot never installs it.

use super::OcrError;
use crate::pixbuf::PixBuf;
use std::io::Write;
use std::process::{Command, Stdio};

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

pub fn recognize(img: &PixBuf) -> Result<String, OcrError> {
    let (img, k) = super::prepare(img, None);
    let png = img.to_png().map_err(|e| OcrError::Failed(format!("{e:#}")))?;
    let mut child = match Command::new(PROGRAM)
        .args(args(dpi_for(k)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(OcrError::NoTesseract),
        Err(e) => return Err(OcrError::Failed(format!("cannot run tesseract: {e}"))),
    };
    // Feed stdin from its own thread so a full stdout pipe cannot stall us.
    let mut stdin = child.stdin.take().expect("piped");
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(&png);
    });
    // Reads stdout and stderr together, then waits.
    let output = child.wait_with_output().map_err(|e| OcrError::Failed(e.to_string()))?;
    let _ = feeder.join();
    let out = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let why = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("tesseract failed").trim().to_string();
        return Err(OcrError::Failed(why));
    }
    Ok(super::tidy_text(&out))
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
}
