use super::detect::{self, Persp};
use super::encode::{self, Segment};
use super::gf;
use super::matrix;
use super::tables::{self, Ecl};
use super::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

// ---- tables ----

#[test]
fn capacities_match_the_standard() {
    // data codewords, ISO/IEC 18004 table 7
    assert_eq!(Ecl::ALL.map(|e| tables::num_data_codewords(1, e)), [19, 16, 13, 9]);
    assert_eq!(Ecl::ALL.map(|e| tables::num_data_codewords(10, e)), [274, 216, 154, 122]);
    assert_eq!(Ecl::ALL.map(|e| tables::num_data_codewords(40, e)), [2956, 2334, 1666, 1276]);
    for v in 1..=40 {
        for e in Ecl::ALL {
            let (lens, ecc) = tables::block_layout(v, e);
            let total: usize = lens.iter().map(|l| l + ecc).sum();
            assert_eq!(total, tables::raw_data_modules(v) / 8, "v{v} {e:?}");
            assert_eq!(lens.iter().sum::<usize>(), tables::num_data_codewords(v, e));
            // the data modules of the drawn pattern match the formula
            if e == Ecl::L {
                let (_, f) = tables::function_patterns(v);
                assert_eq!(tables::data_positions(&f).len(), tables::raw_data_modules(v), "v{v}");
            }
        }
    }
}

#[test]
fn alignment_positions_match_the_standard() {
    assert_eq!(tables::alignment_positions(1), Vec::<usize>::new());
    assert_eq!(tables::alignment_positions(2), vec![6, 18]);
    assert_eq!(tables::alignment_positions(7), vec![6, 22, 38]);
    assert_eq!(tables::alignment_positions(32), vec![6, 34, 60, 86, 112, 138]);
    assert_eq!(tables::alignment_positions(40), vec![6, 30, 58, 86, 114, 142, 170]);
}

#[test]
fn format_and_version_bits_match_the_standard() {
    // ISO/IEC 18004 Annex C
    assert_eq!(tables::format_bits(Ecl::L, 0), 0b111011111000100);
    assert_eq!(tables::format_bits(Ecl::M, 0), 0b101010000010010);
    assert_eq!(tables::format_bits(Ecl::Q, 0), 0b011010101011111);
    assert_eq!(tables::format_bits(Ecl::H, 0), 0b001011010001001);
    assert_eq!(tables::format_bits(Ecl::M, 5), 0b100000011001110);
    // Annex D
    assert_eq!(tables::version_bits(7), 0b000111110010010100);
    assert_eq!(tables::version_bits(40), 0b101000110001101001);
}

// ---- Reed–Solomon ----

#[test]
fn iso_example_codewords() {
    // ISO/IEC 18004 Annex I: "01234567", 1-M
    let seg = Segment::numeric("01234567").unwrap();
    let data = encode::data_codewords(&[seg], 1, Ecl::M).unwrap();
    assert_eq!(data, [0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11]);
    assert_eq!(gf::remainder(&data, 10), [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55]);
    // "HELLO WORLD" 1-Q and 1-M
    let seg = Segment::alphanumeric("HELLO WORLD").unwrap();
    let data = encode::data_codewords(std::slice::from_ref(&seg), 1, Ecl::Q).unwrap();
    assert_eq!(data, [32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236]);
    assert_eq!(gf::remainder(&data, 13), [168, 72, 22, 82, 217, 54, 156, 0, 46, 15, 180, 122, 16]);
    let data = encode::data_codewords(&[seg], 1, Ecl::M).unwrap();
    assert_eq!(gf::remainder(&data, 10), [196, 35, 39, 119, 235, 215, 231, 226, 93, 23]);
}

#[test]
fn reed_solomon_corrects_up_to_half_the_check_bytes() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for nsym in [7usize, 10, 17, 22, 30] {
        for _ in 0..60 {
            let k = 10 + rng.below(100) as usize;
            let data: Vec<u8> = (0..k).map(|_| rng.next() as u8).collect();
            let mut block = data.clone();
            block.extend(gf::remainder(&data, nsym));
            let good = block.clone();
            let errs = rng.below(nsym as u64 / 2 + 1) as usize;
            let mut at = Vec::new();
            while at.len() < errs {
                let i = rng.below(block.len() as u64) as usize;
                if !at.contains(&i) {
                    at.push(i);
                    block[i] ^= 1 + rng.below(255) as u8;
                }
            }
            assert_eq!(gf::correct(&mut block, nsym), Some(errs));
            assert_eq!(block, good);
        }
    }
}

#[test]
fn reed_solomon_never_miscorrects_silently_into_garbage() {
    // Beyond capacity the decoder may fail or (rarely) land on another
    // codeword, but whatever it returns is a valid codeword.
    let mut rng = Rng(77);
    for _ in 0..300 {
        let mut block: Vec<u8> = (0..40).map(|_| rng.next() as u8).collect();
        if gf::correct(&mut block, 10).is_some() {
            let (d, c) = block.split_at(30);
            assert_eq!(gf::remainder(d, 10), c);
        }
    }
}

// ---- round trips ----

fn roundtrip(code: &QrCode) -> matrix::Content {
    matrix::decode(&code.matrix).expect("decodes")
}

#[test]
fn every_version_and_level_round_trips() {
    let mut rng = Rng(42);
    for v in 1..=40 {
        for e in Ecl::ALL {
            let cap = tables::num_data_codewords(v, e);
            // fill most of this version so it is the one chosen
            let n = cap.saturating_sub(3).max(1);
            let text: String = (0..n).map(|_| (b'a' + rng.below(26) as u8) as char).collect();
            let code = encode::encode_segments(&[Segment::bytes(text.as_bytes())], e, v, None).unwrap();
            assert_eq!(code.version, v);
            let c = roundtrip(&code);
            assert_eq!(c.text, text, "v{v} {e:?}");
            assert_eq!(c.ecl, e);
        }
    }
}

#[test]
fn modes_and_masks_round_trip() {
    for (s, m) in [("0123456789012", encode::Mode::Numeric), ("HTTPS://EXAMPLE.COM/A-B", encode::Mode::Alphanumeric), ("Grüße, мир 🌍", encode::Mode::Byte)] {
        let seg = Segment::auto(s);
        assert_eq!(seg.mode, m);
        for mask in 0..8 {
            let code = encode::encode_segments(std::slice::from_ref(&seg), Ecl::Q, 1, Some(mask)).unwrap();
            assert_eq!(code.mask, mask);
            assert_eq!(roundtrip(&code).text, s);
        }
    }
    // mixed segments
    let segs = [Segment::numeric("123").unwrap(), Segment::alphanumeric("ABC").unwrap(), Segment::bytes(b"xyz")];
    let code = encode::encode_segments(&segs, Ecl::L, 1, None).unwrap();
    assert_eq!(roundtrip(&code).text, "123ABCxyz");
}

#[test]
fn eci_and_latin1_are_read() {
    // ECI 3 (ISO-8859-1) then byte "é"
    let mut bits = String::new();
    bits += "0111"; // ECI
    bits += "00000011";
    bits += "0100";
    bits += "00000001";
    bits += "11101001";
    bits += "0000";
    while !bits.len().is_multiple_of(8) {
        bits.push('0');
    }
    let data: Vec<u8> = bits.as_bytes().chunks(8).map(|c| c.iter().fold(0, |a, &b| (a << 1) | (b - b'0'))).collect();
    assert_eq!(matrix::parse(&data, 1).as_deref(), Some("é"));
    // bytes that are not UTF-8 read as ISO-8859-1
    assert_eq!(matrix::parse(&[0x40, 0x1E, 0x90], 1).as_deref(), Some("é"));
}

#[test]
fn too_long_text_does_not_encode() {
    assert!(encode_text(&"x".repeat(2331), Ecl::M).is_some());
    assert!(encode_text(&"x".repeat(2332), Ecl::M).is_none());
}

#[test]
fn damaged_matrices_still_decode() {
    let code = encode_text("https://example.com/some/path?q=1", Ecl::M).unwrap();
    let mut m = code.matrix.clone();
    let mut rng = Rng(9);
    // a few flipped data modules (well within level M)
    for _ in 0..8 {
        let x = 9 + rng.below(m.size as u64 - 18) as usize;
        let y = 9 + rng.below(m.size as u64 - 18) as usize;
        m.set(x, y, !m.get(x, y));
    }
    assert_eq!(matrix::decode(&m).unwrap().text, "https://example.com/some/path?q=1");
    // mirrored
    assert_eq!(matrix::decode(&code.matrix.transposed()).unwrap().text, "https://example.com/some/path?q=1");
}

// ---- pictures ----

/// Draw `code` into a `w×h` picture with the symbol's outer corners
/// (quiet zone excluded) at `quad`, plus `noise` (fraction of pixels).
fn picture(code: &QrCode, w: u32, h: u32, quad: [(f64, f64); 4], noise: f64, seed: u64) -> PixBuf {
    let n = code.size() as f64;
    let to_mod = Persp::quad_to_quad(quad, [(0.0, 0.0), (n, 0.0), (n, n), (0.0, n)]);
    let mut img = PixBuf::new(w, h);
    let mut rng = Rng(seed | 1);
    let d = img.as_raw_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let (mx, my) = to_mod.map(x as f64 + 0.5, y as f64 + 0.5);
            let dark = mx >= 0.0 && my >= 0.0 && mx < n && my < n && code.get(mx as usize, my as usize);
            let mut v: u8 = if dark { 30 } else { 230 };
            if noise > 0.0 && (rng.below(1_000_000) as f64) < noise * 1e6 {
                v = 255 - v;
            }
            // a soft gradient, as in a photo of a screen
            let g = (x * 40 / w as usize) as u8;
            let i = (y * w as usize + x) * 4;
            d[i..i + 4].copy_from_slice(&[v.saturating_sub(g), v.saturating_sub(g), v.saturating_sub(g / 2), 255]);
        }
    }
    img
}

fn square(cx: f64, cy: f64, half: f64, deg: f64) -> [(f64, f64); 4] {
    let (s, c) = deg.to_radians().sin_cos();
    [(-half, -half), (half, -half), (half, half), (-half, half)].map(|(x, y)| (cx + x * c - y * s, cy + x * s + y * c))
}

#[test]
fn reads_a_plain_rendered_code() {
    let code = encode_text("Hello, Rustshot!", Ecl::M).unwrap();
    let img = render(&code, 4, 4);
    assert_eq!(read(&img), vec!["Hello, Rustshot!".to_string()]);
    // one pixel per module, too
    let img = render(&code, 1, 4);
    assert_eq!(read(&img), vec!["Hello, Rustshot!".to_string()]);
}

#[test]
fn reads_rotated_codes() {
    for (i, deg) in [0.0, 17.0, 45.0, 90.0, 133.0, 180.0, 251.0, 300.0].into_iter().enumerate() {
        let text = format!("rotated {deg}° — https://example.org/{i}");
        let code = encode_text(&text, Ecl::M).unwrap();
        let img = picture(&code, 400, 400, square(200.0, 200.0, 120.0, deg), 0.0, i as u64);
        assert_eq!(read(&img), vec![text], "{deg}°");
    }
}

#[test]
fn reads_perspective_skewed_and_noisy_codes() {
    type Case<'a> = (&'a str, [(f64, f64); 4], f64);
    let cases: [Case; 4] = [
        ("skew a", [(60.0, 40.0), (330.0, 70.0), (350.0, 360.0), (40.0, 330.0)], 0.0),
        ("skew b", [(80.0, 80.0), (320.0, 50.0), (330.0, 350.0), (70.0, 310.0)], 0.0),
        ("noise", [(50.0, 50.0), (350.0, 50.0), (350.0, 350.0), (50.0, 350.0)], 0.01),
        ("skew + noise", [(70.0, 60.0), (340.0, 80.0), (320.0, 340.0), (60.0, 350.0)], 0.005),
    ];
    for (i, (name, quad, noise)) in cases.into_iter().enumerate() {
        let text = format!("{name}: The quick brown fox jumps over the lazy dog 0123456789");
        let code = encode_text(&text, Ecl::M).unwrap();
        assert!(code.version >= 4, "has an alignment pattern");
        let img = picture(&code, 400, 400, quad, noise, 1000 + i as u64);
        assert_eq!(read(&img), vec![text], "{name}");
    }
}

#[test]
fn reads_large_versions() {
    for len in [300usize, 900, 1800] {
        let text: String = (0..len).map(|i| (b'A' + (i % 26) as u8) as char).collect();
        let code = encode_text(&text, Ecl::M).unwrap();
        assert!(code.version >= 7);
        let img = render(&code, 3, 4);
        assert_eq!(read(&img), vec![text.clone()], "v{}", code.version);
        let side = img.width() as f64;
        let img = picture(&code, img.width() + 40, img.height() + 40, square(side / 2.0 + 20.0, side / 2.0 + 20.0, side / 2.0 - 12.0, 8.0), 0.0, 5);
        assert_eq!(read(&img), vec![text], "rotated v{}", code.version);
    }
}

#[test]
fn reads_several_codes_in_one_picture() {
    let texts = ["first", "second code", "third: 12345"];
    let mut img = PixBuf::from_raw(700, 300, vec![255; 700 * 300 * 4]).unwrap();
    for (i, t) in texts.iter().enumerate() {
        let c = render(&encode_text(t, Ecl::M).unwrap(), 6, 2);
        let ox = 20 + i * 230;
        for y in 0..c.height() as usize {
            for x in 0..c.width() as usize {
                let s = (y * c.width() as usize + x) * 4;
                let d = ((y + 40) * 700 + ox + x) * 4;
                let px: [u8; 4] = c.as_raw()[s..s + 4].try_into().unwrap();
                img.as_raw_mut()[d..d + 4].copy_from_slice(&px);
            }
        }
    }
    assert_eq!(read(&img), texts.map(String::from).to_vec());
}

#[test]
fn reads_light_on_dark_codes_not_at_all_but_never_panics() {
    // Inverted codes are not in the standard; this only checks no panic.
    let code = encode_text("inverted", Ecl::M).unwrap();
    let mut img = render(&code, 4, 4);
    for p in img.as_raw_mut().as_chunks_mut::<4>().0 {
        p[0] = 255 - p[0];
        p[1] = 255 - p[1];
        p[2] = 255 - p[2];
    }
    let _ = read(&img);
}

#[test]
fn fuzz_never_panics() {
    let mut rng = Rng(0xdead_beef);
    // random pictures, stripes and checkerboards
    for i in 0..60 {
        let (w, h) = (1 + rng.below(160) as u32, 1 + rng.below(160) as u32);
        let mut d = vec![0u8; (w * h * 4) as usize];
        let mode = i % 3;
        for (k, p) in d.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (k as u32 % w, k as u32 / w);
            let v = match mode {
                0 => rng.next() as u8,
                1 => if (x / (1 + i % 5)) % 2 == 0 { 0 } else { 255 },
                _ => if ((x / 3) + (y / 3)) % 2 == 0 { 0 } else { 255 },
            };
            *p = [v, v, v, rng.next() as u8];
        }
        let _ = detect::decode_rgba(w as usize, h as usize, &d);
    }
    // finder-like fragments with garbage between them
    let code = encode_text("fuzz", Ecl::L).unwrap();
    for i in 0..40 {
        let mut img = render(&code, 3, 3);
        let n = img.as_raw().len();
        for _ in 0..(i * 200) {
            let k = rng.below(n as u64 / 4) as usize * 4;
            let v = rng.next() as u8;
            img.as_raw_mut()[k..k + 3].copy_from_slice(&[v, v, v]);
        }
        let _ = read(&img);
    }
    // random module matrices and random codeword streams
    for _ in 0..400 {
        let v = 1 + rng.below(40) as usize;
        let size = tables::size_of(v);
        let mut m = tables::Matrix::new(size);
        for c in m.cells.iter_mut() {
            *c = rng.next() & 1 == 1;
        }
        let _ = matrix::decode(&m);
        let data: Vec<u8> = (0..rng.below(300)).map(|_| rng.next() as u8).collect();
        let _ = matrix::parse(&data, v);
    }
    // real codes with format bits destroyed and random bytes after a valid header
    for e in Ecl::ALL {
        let code = encode_text("format", e).unwrap();
        let mut m = code.matrix.clone();
        for i in 0..15 {
            for (x, y) in tables::format_positions(m.size, i) {
                m.set(x, y, rng.next() & 1 == 1);
            }
        }
        let _ = matrix::decode(&m);
    }
    // a too-short or empty buffer
    assert!(detect::decode_rgba(10, 10, &[0; 12]).is_empty());
    assert!(detect::decode_rgba(0, 0, &[]).is_empty());
}

#[test]
fn only_http_links_are_openable() {
    assert_eq!(openable_link(" https://example.com/x "), Some("https://example.com/x"));
    assert_eq!(openable_link("HTTP://EXAMPLE.COM"), Some("HTTP://EXAMPLE.COM"));
    for bad in ["javascript:alert(1)", "file:///C:/x", "https://", "http:///x", "ftp://x", "https://a b", "https://a\u{7}", "mailto:a@b", "www.example.com", "ms-settings:"] {
        assert_eq!(openable_link(bad), None, "{bad}");
    }
    // user info hides the real host: refused; ports are shown
    for bad in ["https://www.paypal.com-login@evil.example/", "https://a@b", "https://x:99999/", "https://x:/", "https://ex%61mple.com/", "https://a..b/", "https://a\u{202e}b/"] {
        assert_eq!(link_parts(bad), None, "{bad}");
    }
    let p = link_parts("https://Example.COM:8443/a/b?c#d").unwrap();
    assert_eq!((p.head.as_str(), p.unicode, p.rest), ("https://example.com:8443", None, "/a/b?c#d"));
    let p = link_parts("http://[::1]:80/").unwrap();
    assert_eq!(p.head, "http://[::1]:80");
    // an IDN host: punycode first, the Unicode form beside it
    assert_eq!(punycode("bücher").as_deref(), Some("bcher-kva"));
    assert_eq!(punycode("münchen").as_deref(), Some("mnchen-3ya"));
    assert_eq!(punycode("例え").as_deref(), Some("r8jz45g"));
    let p = link_parts("https://\u{430}pple.com/login").unwrap();
    assert_eq!(p.head, "https://xn--pple-43d.com");
    assert_eq!(p.unicode.as_deref(), Some("https://\u{430}pple.com"));
}

#[test]
fn decode_is_fast_on_4k() {
    let code = encode_text("speed", Ecl::M).unwrap();
    let small = render(&code, 8, 4);
    let (w, h) = (3840u32, 2160u32);
    let mut d = vec![255u8; (w * h * 4) as usize];
    for y in 0..small.height() as usize {
        let s = y * small.width() as usize * 4;
        let o = ((y + 900) * w as usize + 1700) * 4;
        d[o..o + small.width() as usize * 4].copy_from_slice(&small.as_raw()[s..s + small.width() as usize * 4]);
    }
    let img = PixBuf::from_raw(w, h, d).unwrap();
    let t = std::time::Instant::now();
    assert_eq!(read(&img), vec!["speed".to_string()]);
    let ms = t.elapsed().as_millis();
    eprintln!("4K read: {ms} ms");
    if !cfg!(debug_assertions) {
        assert!(ms < 1000, "{ms} ms");
    }
}

// ---- bounded time on adversarial pictures ----

fn gray(w: u32, h: u32, v: u8) -> PixBuf {
    PixBuf::from_raw(w, h, vec![v; (w * h * 4) as usize].chunks(4).flat_map(|_| [v, v, v, 255]).collect()).unwrap()
}

fn fill(img: &mut PixBuf, x0: i64, y0: i64, s: i64, v: u8) {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let d = img.as_raw_mut();
    for y in y0.max(0)..(y0 + s).min(h) {
        for x in x0.max(0)..(x0 + s).min(w) {
            let i = ((y * w + x) * 4) as usize;
            d[i..i + 3].copy_from_slice(&[v, v, v]);
        }
    }
}

/// A finder pattern of module `m` with its top-left at (x, y), with a
/// light separator ring round it.
fn finder_at(img: &mut PixBuf, x: i64, y: i64, m: i64) {
    fill(img, x - m, y - m, 9 * m, 255);
    fill(img, x, y, 7 * m, 0);
    fill(img, x + m, y + m, 5 * m, 255);
    fill(img, x + 2 * m, y + 2 * m, 3 * m, 0);
}

/// The review's adversarial pictures (4K, and two monitors wide) are read
/// within the budget; a legitimate full-screen code still reads.
fn adversarial(limit_ms: u128) {
    let mut cases: Vec<(&str, PixBuf)> = Vec::new();
    // noise
    let mut rng = Rng(5);
    let mut noise = PixBuf::new(3840, 2160);
    for p in noise.as_raw_mut().as_chunks_mut::<4>().0 {
        let v = if rng.next() & 1 == 0 { 0 } else { 255 };
        *p = [v, v, v, 255];
    }
    cases.push(("noise", noise));
    // a grid of tiny finder patterns
    let mut grid = gray(3840, 2160, 255);
    for gy in 0..2160 / 10 {
        for gx in 0..3840 / 10 {
            finder_at(&mut grid, gx * 10 + 1, gy * 10 + 1, 1);
        }
    }
    cases.push(("tiny finders", grid));
    // three huge finders at a right angle over a dark field, no alignment
    let mut dark = gray(3840, 2160, 0);
    for (x, y) in [(100, 100), (1900, 100), (100, 1500)] {
        finder_at(&mut dark, x, y, 90);
    }
    cases.push(("huge finders on dark", dark.clone()));
    // the same twice, two monitors wide
    let mut wide = gray(7680, 2160, 0);
    for ox in [0, 3840] {
        for (x, y) in [(100, 100), (1900, 100), (100, 1500)] {
            finder_at(&mut wide, ox + x, y, 90);
        }
    }
    cases.push(("two monitors", wide));
    for (name, img) in &cases {
        let t = std::time::Instant::now();
        let got = read(img);
        let ms = t.elapsed().as_millis();
        eprintln!("{name}: {ms} ms, {} found", got.len());
        assert!(ms < limit_ms, "{name}: {ms} ms (limit {limit_ms})");
    }
    // a legitimate full-screen v3 code at 54 px a module
    let code = encode_text("https://example.com/full-screen", Ecl::M).unwrap();
    let big = render(&code, 54, 4);
    let mut screen = gray(3840, 2160, 255);
    let (bw, bh) = (big.width() as usize, big.height() as usize);
    for y in 0..bh.min(2160) {
        let s = y * bw * 4;
        let o = (y * 3840 + 600) * 4;
        let n = bw.min(3840 - 600) * 4;
        screen.as_raw_mut()[o..o + n].copy_from_slice(&big.as_raw()[s..s + n]);
    }
    let t = std::time::Instant::now();
    assert_eq!(read(&screen), vec!["https://example.com/full-screen".to_string()]);
    eprintln!("full-screen v{}: {} ms", code.version, t.elapsed().as_millis());
}

#[test]
fn adversarial_pictures_are_bounded() {
    // the budget (4 s in debug, 300 ms in release) and some slack
    adversarial(if cfg!(debug_assertions) { 9000 } else { 900 });
}

/// The release numbers: `cargo test --release qr_adversarial_bench --
/// --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "benchmark; run in release, alone"]
fn qr_adversarial_bench() {
    adversarial(450);
}
