//! The app icon for the Windows resource (build.rs includes this file,
//! `#[path]`): the .ico with its BMP images stored as PNG. Tested in the
//! crate.

/// An .ico with its 32-bit BMP images re-stored as PNG (the pixels as
/// they are; alpha from the AND mask when the BMP has none). `None` if the
/// file is not an icon this understands.
pub fn repack(b: &[u8]) -> Option<Vec<u8>> {
    let u16_at = |i: usize| Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?));
    let u32_at = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?));
    if u16_at(0)? != 0 || u16_at(2)? != 1 {
        return None;
    }
    let n = u16_at(4)? as usize;
    let mut images = Vec::new();
    for k in 0..n {
        let e = 6 + 16 * k;
        let (size, off) = (u32_at(e + 8)? as usize, u32_at(e + 12)? as usize);
        let img = b.get(off..off + size)?;
        let mut entry = b.get(e..e + 16)?.to_vec();
        let bmp = u32_at(off)? == 40 && u16_at(off + 14)? == 32 && u32_at(off + 16)? == 0;
        let data = if bmp {
            let w = u32_at(off + 4)? as usize;
            let h = (u32_at(off + 8)? / 2) as usize;
            let px = img.get(40..40 + w * h * 4)?;
            let mask_row = w.div_ceil(8).div_ceil(4) * 4;
            let mask = img.get(40 + w * h * 4..40 + w * h * 4 + mask_row * h)?;
            let no_alpha = px.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
            let mut rgba = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                let sy = h - 1 - y; // bottom-up
                for x in 0..w {
                    let p = &px[(sy * w + x) * 4..][..4];
                    let a = if !no_alpha {
                        p[3]
                    } else if mask[sy * mask_row + x / 8] >> (7 - x % 8) & 1 == 1 {
                        0
                    } else {
                        255
                    };
                    rgba.extend_from_slice(&[p[2], p[1], p[0], a]);
                }
            }
            crate::pngfile::encode(w as u32, h as u32, crate::pngfile::Color::Rgba, &rgba)
        } else {
            img.to_vec()
        };
        entry[8..12].copy_from_slice(&(data.len() as u32).to_le_bytes());
        images.push((entry, data));
    }
    let mut out = b[..6].to_vec();
    let mut off = 6 + 16 * n;
    for (entry, data) in &images {
        out.extend_from_slice(&entry[..12]);
        out.extend_from_slice(&(off as u32).to_le_bytes());
        off += data.len();
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    /// Every image of the repacked icon has the pixels of the original's.
    #[test]
    fn repacked_icon_keeps_the_pixels() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/icons/rustshot.ico");
        let b = std::fs::read(path).unwrap();
        let r = super::repack(&b).expect("repacks");
        assert!(r.len() < b.len() / 2, "{} -> {} bytes", b.len(), r.len());
        let n = u16::from_le_bytes([b[4], b[5]]) as usize;
        assert_eq!(&r[..6], &b[..6]);
        let at = |v: &[u8], i: usize| u32::from_le_bytes(v[i..i + 4].try_into().unwrap()) as usize;
        for k in 0..n {
            let e = 6 + 16 * k;
            assert_eq!(&r[e..e + 8], &b[e..e + 8], "entry {k}");
            let (old, new) = (&b[at(&b, e + 12)..][..at(&b, e + 8)], &r[at(&r, e + 12)..][..at(&r, e + 8)]);
            if at(old, 0) != 40 {
                assert_eq!(old, new, "PNG entries stay");
                continue;
            }
            let im = crate::pngfile::decode(new).expect("a PNG");
            let w = at(old, 4);
            assert_eq!((im.w as usize, im.h as usize), (w, w));
            for y in 0..w {
                for x in 0..w {
                    let p = &old[40 + ((w - 1 - y) * w + x) * 4..][..4];
                    assert_eq!(&im.rgba[(y * w + x) * 4..][..4], &[p[2], p[1], p[0], p[3]], "{w}px at {x},{y}");
                }
            }
        }
    }
}
