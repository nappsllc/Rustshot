//! Trim before save: cutting a finished recording to `start..end`
//! without re-encoding where the format allows it.
//!
//! - MP4 ([`mp4`]): the samples from the key frame at or before the
//!   start to the last frame shown before the end are copied, and edit
//!   lists start and end the presentation exactly where asked.
//! - GIF ([`gif`]): frames are copied as they are; the first kept frame is
//!   recomposed (it may only hold what changed) and its delay shortened.
//!
//! [`trim_file`] writes `<name>-trimmed.<ext>` next to the recording (a
//! `.part` first, renamed once finished) and returns its path; the
//! original is left for the caller to keep or remove.

pub mod decode;
#[cfg(windows)]
mod exact_win;
pub mod gif;
pub mod mp4;

use anyhow::{bail, ensure, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Which way a file is trimmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mp4,
    Gif,
}

impl Kind {
    pub fn of(path: &Path) -> Option<Kind> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "mp4" | "m4v" | "mov" => Some(Kind::Mp4),
            "gif" => Some(Kind::Gif),
            _ => None,
        }
    }
}

/// `<stem>-trimmed.<ext>` beside `path` (a free name).
pub fn trimmed_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "recording".into());
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let name = if ext.is_empty() { format!("{stem}-trimmed") } else { format!("{stem}-trimmed.{ext}") };
    crate::export::unique_path(&path.with_file_name(name))
}

/// The recording's length.
pub fn length(path: &Path) -> Result<Duration> {
    let mut f = std::io::BufReader::new(std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?);
    match Kind::of(path) {
        Some(Kind::Mp4) => mp4::duration(&mut f),
        Some(Kind::Gif) => Ok(gif::Index::read(&mut f)?.length()),
        None => bail!("not a recording: {}", path.display()),
    }
}

/// Write `path`'s `start..end` to a new file beside it; its path. The new
/// file is on disk (synced) and read back before it is returned: its index
/// parses and it lasts `end - start` (within a frame or two).
pub fn trim_file(path: &Path, start: Duration, end: Duration) -> Result<PathBuf> {
    let kind = Kind::of(path).with_context(|| format!("cannot trim {}", path.display()))?;
    let dst = trimmed_path(path);
    let mut part = dst.as_os_str().to_os_string();
    part.push(".part");
    let part = PathBuf::from(part);
    let r = write_part(kind, path, &part, start, end).and_then(|()| validate(kind, &part, end - start));
    if let Err(e) = r {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &dst).with_context(|| format!("move the trimmed recording to {}", dst.display()))?;
    Ok(dst)
}

fn write_part(kind: Kind, path: &Path, part: &Path, start: Duration, end: Duration) -> Result<()> {
    // Windows' own player ignores edit lists: the start is made exact in
    // the samples there (the edit-list trim when that cannot be done).
    #[cfg(windows)]
    if kind == Kind::Mp4 {
        match exact_win::trim(path, part, start, end).and_then(|()| sync(part)) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("rustshot: trimming with an edit list ({e:#})");
                let _ = std::fs::remove_file(part);
            }
        }
    }
    let mut src = std::io::BufReader::new(std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?);
    let mut out = std::io::BufWriter::new(std::fs::File::create(part).with_context(|| format!("create {}", part.display()))?);
    match kind {
        Kind::Mp4 => {
            mp4::trim(&mut src, &mut out, start, end)?;
        }
        Kind::Gif => {
            gif::trim(&mut src, &mut out, start, end)?;
        }
    }
    use std::io::Write;
    out.flush()?;
    out.get_ref().sync_all().context("write the trimmed recording to disk")?;
    Ok(())
}

fn sync(p: &Path) -> Result<()> {
    std::fs::OpenOptions::new().write(true).open(p)?.sync_all().context("write the trimmed recording to disk")
}

/// `p` reads back as a recording of about `want`: every track's sample
/// tables parse (MP4), every frame (GIF).
fn validate(kind: Kind, p: &Path, want: Duration) -> Result<()> {
    let mut f = std::io::BufReader::new(std::fs::File::open(p)?);
    let got = match kind {
        Kind::Mp4 => {
            let m = mp4::Movie::read(&mut f)?;
            let (_, tracks) = m.tracks()?;
            ensure!(tracks.iter().all(|t| t.s.len() > 0), "the trimmed recording has an empty track");
            mp4::duration(&mut f)?
        }
        Kind::Gif => gif::Index::read(&mut f)?.length(),
    };
    let slack = Duration::from_millis(120);
    ensure!(got + slack >= want && got <= want + slack, "the trimmed recording lasts {got:?}, not {want:?}");
    Ok(())
}

/// Put `trimmed` in place of `path` in one step (the old file stays whole
/// until then; a failed rename leaves both).
pub fn replace_with(path: &Path, trimmed: &Path) -> Result<()> {
    std::fs::rename(trimmed, path).with_context(|| format!("replace {}", path.display()))?;
    // The rename itself on disk (a directory entry).
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_names() {
        assert_eq!(Kind::of(Path::new("a/b.MP4")), Some(Kind::Mp4));
        assert_eq!(Kind::of(Path::new("b.gif")), Some(Kind::Gif));
        assert_eq!(Kind::of(Path::new("b.png")), None);
        let dir = std::env::temp_dir().join(format!("rustshot-cut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("rec.mp4");
        assert_eq!(trimmed_path(&p), dir.join("rec-trimmed.mp4"));
        std::fs::write(dir.join("rec-trimmed.mp4"), b"x").unwrap();
        assert_ne!(trimmed_path(&p), dir.join("rec-trimmed.mp4"), "a free name");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A trimmed file is read back before it replaces anything: one that
    /// does not parse, or lasts the wrong time, is refused (and the
    /// original is left as it was).
    #[test]
    fn a_bad_result_never_replaces_the_original() {
        let dir = std::env::temp_dir().join(format!("rustshot-cut-validate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut enc = crate::rec::gif::GifEncoder::new(std::io::Cursor::new(Vec::new()), 8, 8, 10).unwrap();
        for k in 0..10u64 {
            use crate::rec::VideoEncoder;
            enc.push_video(&crate::rec::Frame::bgra(8, 8, [k as u8 * 20, 0, 0, 255].repeat(64), Duration::from_millis(k * 100))).unwrap();
        }
        let gif = dir.join("a.gif");
        std::fs::write(&gif, enc.into_inner().unwrap().into_inner()).unwrap();
        assert!(validate(Kind::Gif, &gif, Duration::from_secs(1)).is_ok());
        assert!(validate(Kind::Gif, &gif, Duration::from_secs(3)).is_err(), "the wrong length");
        let bad = dir.join("b.mp4");
        std::fs::write(&bad, b"\0\0\0\x08free").unwrap();
        assert!(validate(Kind::Mp4, &bad, Duration::from_secs(1)).is_err(), "no index");
        // The trim itself: the original is untouched until the rename.
        let before = std::fs::read(&gif).unwrap();
        let t = trim_file(&gif, Duration::from_millis(200), Duration::from_millis(500)).unwrap();
        assert_eq!(std::fs::read(&gif).unwrap(), before);
        replace_with(&gif, &t).unwrap();
        assert_eq!(length(&gif).unwrap(), Duration::from_millis(300));
        assert!(!t.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
