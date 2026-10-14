use super::*;
use crate::rec::gif::lzw_encode;

/// Frame `k` of a test clip: a gray canvas with a colored square that
/// moves (few colors: the encoder keeps them exactly).
fn picture(k: u32, w: u32, h: u32) -> Vec<u8> {
    let mut bgra = [90u8, 90, 90, 255].repeat((w * h) as usize);
    let (x0, y0) = (4 + k * 5 % (w - 12), 3 + k * 3 % (h - 10));
    let c = [[40u8, 200, 40, 255], [200, 40, 40, 255], [40, 40, 220, 255]][(k % 3) as usize];
    for y in y0..y0 + 8 {
        for x in x0..x0 + 8 {
            let i = ((y * w + x) * 4) as usize;
            bgra[i..i + 4].copy_from_slice(&c);
        }
    }
    bgra
}

fn rgba_of(bgra: &[u8]) -> Vec<u8> {
    bgra.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect()
}

/// `n` frames at 10 fps.
fn clip(n: u32, w: u32, h: u32) -> Vec<u8> {
    let mut enc = GifEncoder::new(Cursor::new(Vec::new()), w, h, 10).unwrap();
    for k in 0..n {
        enc.push_video(&RecFrame::bgra(w, h, picture(k, w, h), Duration::from_millis(k as u64 * 100))).unwrap();
    }
    enc.into_inner().unwrap().into_inner()
}

#[test]
fn lzw_round_trips() {
    let mut r = 12345u32;
    for min in [2u8, 4, 8] {
        for len in [0usize, 1, 7, 300, 20_000] {
            let data: Vec<u8> = (0..len)
                .map(|i| {
                    r = r.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    // Runs and noise, within the code size.
                    (if i % 50 < 30 { (i / 50) as u32 } else { r >> 16 } % (1 << min)) as u8
                })
                .collect();
            let enc = lzw_encode(&data, min);
            assert_eq!(lzw_decode(&enc, min, data.len()), data, "min {min}, {len} codes");
        }
    }
}

#[test]
fn frames_decode_as_they_were_recorded() {
    let (w, h) = (40, 30);
    let gif = clip(8, w, h);
    let idx = Arc::new(Index::parse(gif).unwrap());
    assert_eq!((idx.w, idx.h), (40, 30));
    assert_eq!(idx.frames.len(), 8);
    assert_eq!(idx.length(), Duration::from_millis(800));
    assert_eq!(idx.frame_at(Duration::from_millis(250)), 2);
    let mut p = Player::new(idx);
    for k in [0u32, 3, 7, 1, 5] {
        assert_eq!(p.frame(k as usize), &rgba_of(&picture(k, w, h))[..], "frame {k}");
    }
}

#[test]
fn a_trim_keeps_the_frames_shown_and_cuts_their_delays() {
    let (w, h) = (40, 30);
    let gif = clip(10, w, h);
    let mut out = Cursor::new(Vec::new());
    trim(&mut Cursor::new(gif), &mut out, Duration::from_millis(250), Duration::from_millis(720)).unwrap();
    let idx = Arc::new(Index::parse(out.into_inner()).unwrap());
    let delays: Vec<u16> = idx.frames.iter().map(|f| f.delay_cs).collect();
    assert_eq!(delays, vec![5, 10, 10, 10, 10, 2]);
    assert_eq!(idx.length(), Duration::from_millis(470));
    let mut p = Player::new(idx);
    for (i, k) in (2u32..=7).enumerate() {
        assert_eq!(p.frame(i), &rgba_of(&picture(k, w, h))[..], "kept frame {i} is frame {k}");
    }
}

#[test]
fn a_first_frame_that_clears_itself_is_encoded_again() {
    let (w, h) = (40, 30);
    let mut gif = clip(6, w, h);
    let idx = Index::parse(gif.clone()).unwrap();
    // Frame 2 restores the background after it (disposal 2).
    let g = idx.frames[2].gce.unwrap();
    gif[g + 3] = (gif[g + 3] & !0x1C) | (2 << 2);
    let want: Vec<Vec<u8>> = {
        let mut p = Player::new(Arc::new(Index::parse(gif.clone()).unwrap()));
        (2..6).map(|i| p.frame(i).to_vec()).collect()
    };
    let mut out = Cursor::new(Vec::new());
    trim(&mut Cursor::new(gif), &mut out, Duration::from_millis(200), Duration::from_millis(600)).unwrap();
    let idx = Arc::new(Index::parse(out.into_inner()).unwrap());
    let mut p = Player::new(idx.clone());
    let opaque = |v: &[u8]| v.as_chunks::<4>().0.iter().flat_map(|q| if q[3] == 0 { [0, 0, 0, 255] } else { *q }).collect::<Vec<u8>>();
    for (i, wv) in want.iter().enumerate() {
        assert_eq!(p.frame(i), &opaque(wv)[..], "frame {i}");
    }
    assert!((idx.length().as_millis() as i64 - 400).abs() <= 20, "{:?}", idx.length());
}

#[test]
fn plans_and_errors() {
    let idx = Index::parse(clip(5, 40, 30)).unwrap();
    let ms = Duration::from_millis;
    assert_eq!(plan(&idx, ms(0), ms(500)), Some((0, 4, 10, 10)), "all of it");
    assert_eq!(plan(&idx, ms(130), ms(170)), Some((1, 1, 4, 4)), "inside one frame");
    assert_eq!(plan(&idx, ms(300), ms(300)), None, "empty");
    assert_eq!(plan(&idx, ms(600), ms(900)), None, "past the end");
    assert!(Index::parse(b"GIF89a".to_vec()).is_err(), "cut short");
    assert!(Index::parse(b"PNG not a gif at all".to_vec()).is_err());
    let mut cut = clip(3, 40, 30);
    cut.truncate(cut.len() - 20);
    assert!(Index::parse(cut).is_err(), "no trailer");
}
