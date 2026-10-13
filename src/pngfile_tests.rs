//! `pngfile` against the png crate (a dev-dependency kept for these
//! differential tests), plus round trips and hostile input.

use super::*;

fn rng(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// The png crate's decode, as RGBA8 (EXPAND | STRIP_16, as the app did).
fn reference(bytes: &[u8]) -> Option<Image> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Image { w: info.width, h: info.height, rgba })
}

/// A png crate encoding (default compression and filter).
fn png_crate_encode(
    w: u32,
    h: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    plte: Option<Vec<u8>>,
    trns: Option<Vec<u8>>,
) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(depth);
        if let Some(p) = plte {
            enc.set_palette(p);
        }
        if let Some(t) = trns {
            enc.set_trns(t);
        }
        enc.write_header().unwrap().write_image_data(data).unwrap();
    }
    out
}

fn random_image(s: &mut u64, w: u32, h: u32, ch: usize, smooth: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(w as usize * h as usize * ch);
    for y in 0..h {
        for x in 0..w {
            for c in 0..ch {
                v.push(if smooth { (x * 3 + y * 5 + c as u32 * 40) as u8 ^ ((x / 7 + y / 5) as u8 & 0x10) } else { rng(s) as u8 });
            }
        }
    }
    v
}

/// Pictures of more than a few MiB are compressed in bands on threads:
/// still one valid stream.
#[test]
fn big_pictures_encode_in_bands() {
    let mut s = 17u64;
    let (w, h) = (1100u32, 1500u32);
    let mut d = random_image(&mut s, w, h, 4, true);
    for v in d.iter_mut().step_by(97) {
        *v = rng(&mut s) as u8;
    }
    let f = encode(w, h, Color::Rgba, &d);
    let r = reference(&f).expect("the png crate decodes it");
    assert!(r.rgba == d);
    assert!(decode(&f).unwrap().rgba == d);
}

#[test]
fn encodes_what_the_png_crate_decodes() {
    let mut s = 7u64;
    for (w, h) in [(1, 1), (3, 2), (2, 3), (17, 5), (64, 48), (257, 13)] {
        for smooth in [false, true] {
            for color in [Color::Rgba, Color::Rgb] {
                let ch = if color == Color::Rgba { 4 } else { 3 };
                let d = random_image(&mut s, w, h, ch, smooth);
                let f = encode(w, h, color, &d);
                let want: Vec<u8> = if ch == 4 { d.clone() } else { d.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
                let r = reference(&f).expect("the png crate decodes it");
                assert_eq!((r.w, r.h), (w, h));
                assert!(r.rgba == want, "{w}x{h} {color:?} smooth {smooth}");
                assert_eq!(decode(&f).unwrap(), r);
            }
        }
    }
}

#[test]
fn decodes_every_color_type_and_depth_like_the_png_crate() {
    use png::{BitDepth as B, ColorType as C};
    let mut s = 11u64;
    for (w, h) in [(1, 1), (5, 3), (33, 9)] {
        // Gray at every depth, with and without a transparent value.
        for (depth, bits) in [(B::One, 1), (B::Two, 2), (B::Four, 4), (B::Eight, 8), (B::Sixteen, 16)] {
            let rb = (w as usize * bits).div_ceil(8);
            let mut d: Vec<u8> = (0..rb * h as usize).map(|_| rng(&mut s) as u8).collect();
            if bits == 16 {
                // Some samples equal the transparent key below.
                d[..2].copy_from_slice(&[0x12, 0x34]);
            }
            for trns in [None, Some(if bits == 16 { vec![0x12, 0x34] } else { vec![0, 1] })] {
                let f = png_crate_encode(w, h, C::Grayscale, depth, &d, None, trns);
                assert_eq!(decode(&f).unwrap(), reference(&f).unwrap(), "gray {bits} {w}x{h}");
            }
        }
        // Palette at every depth, with and without alphas (fewer than entries).
        for (depth, bits) in [(B::One, 1), (B::Two, 2), (B::Four, 4), (B::Eight, 8)] {
            let n = 1usize << bits;
            let rb = (w as usize * bits).div_ceil(8);
            let d: Vec<u8> = (0..rb * h as usize).map(|_| rng(&mut s) as u8).collect();
            let plte: Vec<u8> = (0..n * 3).map(|_| rng(&mut s) as u8).collect();
            for trns in [None, Some((0..n.div_ceil(2)).map(|_| rng(&mut s) as u8).collect::<Vec<u8>>())] {
                let f = png_crate_encode(w, h, C::Indexed, depth, &d, Some(plte.clone()), trns);
                assert_eq!(decode(&f).unwrap(), reference(&f).unwrap(), "palette {bits} {w}x{h}");
            }
        }
        // RGB, gray + alpha, RGBA at 8 and 16 bits; RGB with a key.
        for (color, ch) in [(C::Rgb, 3), (C::GrayscaleAlpha, 2), (C::Rgba, 4)] {
            for (depth, bytes) in [(B::Eight, 1), (B::Sixteen, 2)] {
                let mut d: Vec<u8> = (0..w as usize * h as usize * ch * bytes).map(|_| rng(&mut s) as u8).collect();
                let trns = (color == C::Rgb).then(|| {
                    let k: Vec<u8> = d[..3 * bytes].to_vec();
                    if bytes == 1 { vec![0, k[0], 0, k[1], 0, k[2]] } else { k[..6].to_vec() }
                });
                if color == C::Rgb && w * h > 1 {
                    let first = d[..3 * bytes].to_vec();
                    d[3 * bytes..6 * bytes].copy_from_slice(&first);
                }
                for t in [None, trns] {
                    let f = png_crate_encode(w, h, color, depth, &d, None, t);
                    assert_eq!(decode(&f).unwrap(), reference(&f).unwrap(), "{color:?} {bytes} {w}x{h}");
                }
            }
        }
    }
}

/// An interlaced file, written by hand (the png crate does not write
/// Adam7): every pass, rows filtered with the five filters in turn.
fn adam7_file(w: u32, h: u32, color: u8, depth: u8, samples: &[u8], plte: Option<&[u8]>) -> Vec<u8> {
    let hdr = Header { w, h, depth, color, interlaced: true };
    let bits_px = hdr.channels() * depth as usize;
    let full_rb = hdr.row_bytes(w);
    let mut raw = Vec::new();
    let mut k = 0u8;
    for &(x0, y0, dx, dy, pw, ph) in &passes(&hdr) {
        if pw == 0 {
            continue;
        }
        let rb = hdr.row_bytes(pw);
        let mut prev = vec![0u8; rb];
        for py in 0..ph {
            let y = (y0 + py * dy) as usize;
            let mut row = vec![0u8; rb];
            for px in 0..pw as usize {
                let x = (x0 as usize) + px * dx as usize;
                for bit in 0..bits_px {
                    let sb = y * full_rb * 8 + x * bits_px + bit;
                    let v = (samples[sb / 8] >> (7 - sb % 8)) & 1;
                    let db = px * bits_px + bit;
                    row[db / 8] |= v << (7 - db % 8);
                }
            }
            let f = k % 5;
            k += 1;
            let mut fr = vec![0u8; rb];
            filter_row(f, &row, &prev, hdr.bpp(), &mut fr);
            raw.push(f);
            raw.extend_from_slice(&fr);
            prev = row;
        }
    }
    let mut out = SIG.to_vec();
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&w.to_be_bytes());
    ihdr[4..8].copy_from_slice(&h.to_be_bytes());
    ihdr[8] = depth;
    ihdr[9] = color;
    ihdr[12] = 1;
    chunk(&mut out, b"IHDR", &ihdr);
    if let Some(p) = plte {
        chunk(&mut out, b"PLTE", p);
    }
    let z = deflate::zlib_compress(&raw, deflate::Effort::DEFAULT);
    // Two IDAT chunks: the stream may be cut anywhere.
    let (a, b) = z.split_at(z.len() / 3);
    chunk(&mut out, b"IDAT", a);
    chunk(&mut out, b"IDAT", b);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn decodes_interlaced_files() {
    let mut s = 5u64;
    for (w, h) in [(1, 1), (2, 2), (3, 7), (9, 9), (17, 3), (40, 23)] {
        for (color, depth, plte) in [(6u8, 8u8, false), (2, 16, false), (0, 1, false), (0, 4, false), (3, 2, true), (4, 8, false)] {
            let hdr = Header { w, h, depth, color, interlaced: true };
            let samples: Vec<u8> = (0..hdr.row_bytes(w) * h as usize).map(|_| rng(&mut s) as u8).collect();
            let pal: Vec<u8> = (0..12).map(|_| rng(&mut s) as u8).collect();
            let f = adam7_file(w, h, color, depth, &samples, plte.then_some(&pal[..]));
            let ours = decode(&f).unwrap_or_else(|e| panic!("{w}x{h} color {color} depth {depth}: {e}"));
            assert_eq!(ours, reference(&f).unwrap(), "{w}x{h} color {color} depth {depth}");
            // And it is the same picture as the plain (non-interlaced) file.
            let mut plain = SIG.to_vec();
            let mut ihdr = [0u8; 13];
            ihdr[..4].copy_from_slice(&w.to_be_bytes());
            ihdr[4..8].copy_from_slice(&h.to_be_bytes());
            ihdr[8] = depth;
            ihdr[9] = color;
            chunk(&mut plain, b"IHDR", &ihdr);
            if plte {
                chunk(&mut plain, b"PLTE", &pal);
            }
            let rb = hdr.row_bytes(w);
            let raw: Vec<u8> = samples.chunks(rb).flat_map(|r| std::iter::once(0).chain(r.iter().copied())).collect();
            chunk(&mut plain, b"IDAT", &deflate::zlib_compress(&raw, deflate::Effort::DEFAULT));
            chunk(&mut plain, b"IEND", &[]);
            assert_eq!(decode(&plain).unwrap(), ours);
        }
    }
}

/// The repo's own screenshots and boards: the same pixels as the png crate.
#[test]
fn decodes_the_repos_pngs_like_the_png_crate() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut n = 0;
    for dir in ["docs/screenshots", "packaging/icons"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap().flatten() {
            if e.path().extension().is_some_and(|x| x == "png") {
                let b = std::fs::read(e.path()).unwrap();
                assert_eq!(decode(&b).unwrap(), reference(&b).unwrap(), "{}", e.path().display());
                n += 1;
            }
        }
    }
    assert!(n >= 5);
}

#[test]
fn hostile_input_fails_cleanly() {
    let mut s = 3u64;
    let mut files = Vec::new();
    let d = random_image(&mut s, 23, 11, 4, true);
    files.push(encode(23, 11, Color::Rgba, &d));
    let pal: Vec<u8> = (0..48).collect();
    files.push(png_crate_encode(13, 7, png::ColorType::Indexed, png::BitDepth::Four, &[0x5a; 49], Some(pal), Some(vec![9; 3])));
    let samples: Vec<u8> = (0..4 * 9 * 9).map(|_| rng(&mut s) as u8).collect();
    files.push(adam7_file(9, 9, 6, 8, &samples, None));
    for f in &files {
        assert!(decode(f).is_ok());
        for _ in 0..400 {
            let mut c = f.clone();
            for _ in 0..1 + rng(&mut s) % 3 {
                let i = (rng(&mut s) as usize) % c.len();
                c[i] ^= 1 << (rng(&mut s) % 8);
            }
            let _ = decode(&c);
            // With every CRC fixed up, so the damage reaches inflate and the filters.
            fix_crcs(&mut c);
            let _ = decode(&c);
            let cut = (rng(&mut s) as usize) % c.len();
            let _ = decode(&c[..cut]);
        }
    }
    for _ in 0..500 {
        let n = (rng(&mut s) % 200) as usize;
        let mut junk: Vec<u8> = SIG.to_vec();
        junk.extend((0..n).map(|_| rng(&mut s) as u8));
        let _ = decode(&junk);
    }
    assert!(decode(b"").is_err());
    assert!(decode(b"not a png").is_err());
}

/// Recompute each chunk's CRC (as far as the chunk lengths make sense).
fn fix_crcs(f: &mut [u8]) {
    let mut pos = 8;
    while pos + 12 <= f.len() {
        let len = be32(&f[pos..]) as usize;
        if len > f.len() - pos - 12 {
            break;
        }
        let c = crc32(&[&f[pos + 4..pos + 8 + len]]);
        f[pos + 8 + len..pos + 12 + len].copy_from_slice(&c.to_be_bytes());
        pos += 12 + len;
    }
}

fn file_with_header(w: u32, h: u32, depth: u8, color: u8, idat: &[u8]) -> Vec<u8> {
    let mut out = SIG.to_vec();
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&w.to_be_bytes());
    ihdr[4..8].copy_from_slice(&h.to_be_bytes());
    ihdr[8] = depth;
    ihdr[9] = color;
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn sizes_are_capped_and_headers_checked() {
    // A zlib bomb's header: refused before anything is allocated.
    let bomb = deflate::zlib_compress(&vec![0u8; 1 << 20], deflate::Effort::DEFAULT);
    assert_eq!(decode(&file_with_header(60000, 60000, 8, 6, &bomb)), Err(PngError("picture too big")));
    // The cap is the image editor's: 200 MP.
    assert_eq!(MAX_PIXELS, 200_000_000);
    assert_eq!(decode(&file_with_header(20_000, 10_001, 8, 6, &bomb)), Err(PngError("picture too big")));
    // Just under it, with far too little data: an error, and nothing near
    // the claimed 800 MB is allocated up front.
    assert_eq!(decode(&file_with_header(14_142, 14_142, 8, 6, &bomb)), Err(PngError("not enough image data")));
    assert_eq!(decode(&file_with_header(1 << 17, 1, 8, 6, &bomb)), Err(PngError("bad size")));
    assert_eq!(decode(&file_with_header(0, 5, 8, 6, &bomb)), Err(PngError("bad size")));
    // More data than the picture needs: an error, not more memory.
    assert!(decode(&file_with_header(16, 16, 8, 6, &bomb)).is_err());
    // Bad depth / color combinations.
    for (d, c) in [(16, 3), (4, 2), (2, 6), (7, 0), (8, 5), (1, 4)] {
        assert_eq!(decode(&file_with_header(4, 4, d, c, &bomb)), Err(PngError("unsupported IHDR")), "depth {d} color {c}");
    }
    // A palette picture without PLTE, a palette index past its end.
    let raw = deflate::zlib_compress(&[0, 5], deflate::Effort::DEFAULT);
    assert_eq!(decode(&file_with_header(1, 1, 8, 3, &raw)), Err(PngError("no palette")));
    let mut f = SIG.to_vec();
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&1u32.to_be_bytes());
    ihdr[4..8].copy_from_slice(&1u32.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 3;
    chunk(&mut f, b"IHDR", &ihdr);
    chunk(&mut f, b"PLTE", &[1, 2, 3]);
    chunk(&mut f, b"IDAT", &raw);
    chunk(&mut f, b"IEND", &[]);
    assert_eq!(decode(&f), Err(PngError("palette index out of range")));
    // A bad filter type.
    let raw = deflate::zlib_compress(&[9, 1, 2, 3, 4], deflate::Effort::DEFAULT);
    assert_eq!(decode(&file_with_header(1, 1, 8, 6, &raw)), Err(PngError("bad filter type")));
}

#[test]
fn ancillary_and_split_chunks() {
    let d = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let f = encode(2, 1, Color::Rgba, &d);
    // A text chunk before IEND, IEND missing: still the picture.
    let iend = f.len() - 12;
    let mut g = f[..iend].to_vec();
    chunk(&mut g, b"tEXt", b"Comment\0hi");
    assert_eq!(decode(&g).unwrap().rgba, d);
    let mut no_end = f[..iend].to_vec();
    assert_eq!(decode(&no_end).unwrap().rgba, d);
    // An unknown critical chunk is refused; IDAT after another chunk too.
    chunk(&mut no_end, b"QUUX", b"");
    assert!(decode(&no_end).is_err());
    let mut late = f[..iend].to_vec();
    chunk(&mut late, b"tEXt", b"a\0b");
    chunk(&mut late, b"IDAT", b"");
    assert_eq!(decode(&late), Err(PngError("IDAT chunks apart")));
}

/// The repo's screenshots tiled into a 4K picture: the size of our file
/// against the png crate's default, and the times.
/// `cargo test --release png_codec_bench -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn png_codec_bench() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut pics = Vec::new();
    for dir in ["docs/screenshots", "docs/design/rustshot-ui"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap().flatten() {
            if e.path().extension().is_some_and(|x| x == "png") {
                pics.push((e.file_name().to_string_lossy().into_owned(), decode(&std::fs::read(e.path()).unwrap()).unwrap()));
            }
        }
    }
    pics.sort_by(|a, b| a.0.cmp(&b.0));
    let crate_png = |im: &Image| png_crate_encode(im.w, im.h, png::ColorType::Rgba, png::BitDepth::Eight, &im.rgba, None, None);
    let time = |f: &dyn Fn() -> usize| {
        let mut best = f64::MAX;
        let mut n = 0;
        for _ in 0..3 {
            let t = std::time::Instant::now();
            n = f();
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
        }
        (n, best)
    };
    let (mut tot_c, mut tot_o) = (0usize, 0usize);
    for (name, im) in &pics {
        let (c, tc) = time(&|| crate_png(im).len());
        let (o, to) = time(&|| encode(im.w, im.h, Color::Rgba, &im.rgba).len());
        eprintln!("BENCH {name} {}x{}: png crate {c} B {tc:.1} ms, ours {o} B {to:.1} ms ({:+.1} %)", im.w, im.h, (o as f64 / c as f64 - 1.0) * 100.0);
        tot_c += c;
        tot_o += o;
    }
    eprintln!("BENCH corpus: png crate {tot_c} B, ours {tot_o} B ({:+.1} %)", (tot_o as f64 / tot_c as f64 - 1.0) * 100.0);
    // A 4K screenshot: the pictures tiled.
    let (w, h) = (3840u32, 2160u32);
    let mut big = vec![0u8; (w * h * 4) as usize];
    let (mut x, mut y, mut row_h) = (0u32, 0u32, 0u32);
    for (_, im) in pics.iter().cycle() {
        if y >= h {
            break;
        }
        if x >= w {
            x = 0;
            y += row_h;
            row_h = 0;
            continue;
        }
        let cw = im.w.min(w - x);
        let chh = im.h.min(h.saturating_sub(y));
        for r in 0..chh {
            let s = (r * im.w * 4) as usize;
            let d = (((y + r) * w + x) * 4) as usize;
            big[d..d + (cw * 4) as usize].copy_from_slice(&im.rgba[s..s + (cw * 4) as usize]);
        }
        x += cw;
        row_h = row_h.max(chh);
    }
    let big = Image { w, h, rgba: big };
    let (c, tc) = time(&|| crate_png(&big).len());
    let (o, to) = time(&|| encode(w, h, Color::Rgba, &big.rgba).len());
    eprintln!("BENCH 4K encode: png crate {c} B {tc:.1} ms, ours {o} B {to:.1} ms ({:+.1} % size, x{:.2} time)", (o as f64 / c as f64 - 1.0) * 100.0, to / tc);
    let f = encode(w, h, Color::Rgba, &big.rgba);
    let (_, dc) = time(&|| reference(&f).unwrap().rgba.len());
    let (_, dt) = time(&|| decode(&f).unwrap().rgba.len());
    eprintln!("BENCH 4K decode: png crate {dc:.1} ms, ours {dt:.1} ms (x{:.2})", dt / dc);
}
