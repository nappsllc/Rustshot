//! SPA pods for the PipeWire streams, built and read by hand (no libspa
//! headers: their builders are inline functions), and the pixel
//! conversions of what the streams deliver. Pure; tested everywhere.
//!
//! A pod is `{u32 size, u32 type}` and a body of `size` bytes, padded to 8.
//! An object's body is `{u32 object type, u32 id}` then properties `{u32
//! key, u32 flags, pod}`; a choice's body is `{u32 kind, u32 flags, child
//! pod header}` then the values back to back (the first is the default).
//! Values are in the machine's byte order.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // used by the Linux streams; tested everywhere

use crate::capture::IRect;

pub const T_ID: u32 = 3;
pub const T_INT: u32 = 4;
pub const T_RECT: u32 = 10;
pub const T_FRACTION: u32 = 11;
pub const T_OBJECT: u32 = 15;
pub const T_CHOICE: u32 = 19;

pub const CHOICE_RANGE: u32 = 1;
pub const CHOICE_ENUM: u32 = 3;
pub const CHOICE_FLAGS: u32 = 4;

pub const OBJ_FORMAT: u32 = 0x40003;
pub const OBJ_PARAM_BUFFERS: u32 = 0x40004;
pub const OBJ_PARAM_META: u32 = 0x40005;

pub const PARAM_ENUM_FORMAT: u32 = 3;
pub const PARAM_FORMAT: u32 = 4;
pub const PARAM_BUFFERS: u32 = 5;
pub const PARAM_META: u32 = 6;

pub const FORMAT_MEDIA_TYPE: u32 = 1;
pub const FORMAT_MEDIA_SUBTYPE: u32 = 2;
pub const FORMAT_VIDEO_FORMAT: u32 = 0x20001;
pub const FORMAT_VIDEO_SIZE: u32 = 0x20003;
pub const FORMAT_VIDEO_FRAMERATE: u32 = 0x20004;

pub const MEDIA_TYPE_VIDEO: u32 = 2;
pub const MEDIA_SUBTYPE_RAW: u32 = 1;

pub const BUFFERS_BUFFERS: u32 = 1;
pub const BUFFERS_DATATYPE: u32 = 6;
pub const META_TYPE: u32 = 1;
pub const META_SIZE: u32 = 2;
pub const META_HEADER: u32 = 1;
/// `sizeof(struct spa_meta_header)`.
pub const META_HEADER_SIZE: u32 = 32;

/// `spa_data_type` bits: memory we can read (no DMA-BUF).
pub const DATA_MEM_PTR: u32 = 1;
pub const DATA_MEM_FD: u32 = 2;

/// `spa_video_format` values.
pub const VF_YUY2: u32 = 4;
pub const VF_RGBX: u32 = 7;
pub const VF_BGRX: u32 = 8;
pub const VF_RGBA: u32 = 11;
pub const VF_BGRA: u32 = 12;
pub const VF_NV12: u32 = 23;

fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

/// A whole pod: header, body, padding to 8.
pub fn pod(ty: u32, body: &[u8]) -> Vec<u8> {
    let mut out = u32s(&[body.len() as u32, ty]);
    out.extend_from_slice(body);
    while !out.len().is_multiple_of(8) {
        out.push(0);
    }
    out
}

pub fn id(v: u32) -> Vec<u8> {
    pod(T_ID, &u32s(&[v]))
}

pub fn int(v: i32) -> Vec<u8> {
    pod(T_INT, &v.to_ne_bytes())
}

/// A choice of `kind` over values of `child` type, each `vals` entry one
/// value's body (all the same size).
pub fn choice(kind: u32, child: u32, vals: &[Vec<u8>]) -> Vec<u8> {
    let size = vals.first().map_or(0, Vec::len) as u32;
    let mut body = u32s(&[kind, 0, size, child]);
    for v in vals {
        body.extend_from_slice(v);
    }
    pod(T_CHOICE, &body)
}

/// An object of `ty` for param `id` with (key, pod) properties.
pub fn object(ty: u32, id: u32, props: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut body = u32s(&[ty, id]);
    for (k, p) in props {
        body.extend_from_slice(&u32s(&[*k, 0]));
        body.extend_from_slice(p);
    }
    pod(T_OBJECT, &body)
}

/// The formats a stream offers: raw video in `formats` (the first
/// preferred), any size up to 16384² (`size` preferred), any rate up to
/// 1000 fps (`fps` preferred). No modifier property: shared-memory
/// buffers, never DMA-BUF.
pub fn enum_format(formats: &[u32], size: (u32, u32), fps: u32) -> Vec<u8> {
    let mut fs = vec![u32s(&[formats[0]])];
    fs.extend(formats.iter().map(|f| u32s(&[*f])));
    object(OBJ_FORMAT, PARAM_ENUM_FORMAT, &[
        (FORMAT_MEDIA_TYPE, id(MEDIA_TYPE_VIDEO)),
        (FORMAT_MEDIA_SUBTYPE, id(MEDIA_SUBTYPE_RAW)),
        (FORMAT_VIDEO_FORMAT, choice(CHOICE_ENUM, T_ID, &fs)),
        (FORMAT_VIDEO_SIZE, choice(CHOICE_RANGE, T_RECT, &[u32s(&[size.0, size.1]), u32s(&[1, 1]), u32s(&[16384, 16384])])),
        (FORMAT_VIDEO_FRAMERATE, choice(CHOICE_RANGE, T_FRACTION, &[u32s(&[fps, 1]), u32s(&[0, 1]), u32s(&[1000, 1])])),
    ])
}

/// Buffers in memory we can map (MemPtr or MemFd), 2-16 of them.
pub fn buffers_param() -> Vec<u8> {
    object(OBJ_PARAM_BUFFERS, PARAM_BUFFERS, &[
        (BUFFERS_BUFFERS, choice(CHOICE_RANGE, T_INT, &[u32s(&[4]), u32s(&[2]), u32s(&[16])])),
        (BUFFERS_DATATYPE, choice(CHOICE_FLAGS, T_INT, &[u32s(&[(1 << DATA_MEM_PTR) | (1 << DATA_MEM_FD)])])),
    ])
}

/// A header (timestamps, flags) on every buffer.
pub fn meta_header_param() -> Vec<u8> {
    object(OBJ_PARAM_META, PARAM_META, &[(META_TYPE, id(META_HEADER)), (META_SIZE, int(META_HEADER_SIZE as i32))])
}

/// A pod as 8-aligned words (what PipeWire is handed).
pub fn aligned(p: &[u8]) -> Vec<u64> {
    let mut w = vec![0u64; p.len().div_ceil(8)];
    for (d, s) in w.iter_mut().zip(p.chunks(8)) {
        let mut b = [0u8; 8];
        b[..s.len()].copy_from_slice(s);
        *d = u64::from_ne_bytes(b);
    }
    w
}

/// A negotiated video format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoFmt {
    pub format: u32,
    pub w: u32,
    pub h: u32,
}

fn rd(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The first value's body of a pod (a choice's default), and its type.
fn value(b: &[u8]) -> Option<(u32, &[u8])> {
    let (size, ty) = (rd(b, 0)? as usize, rd(b, 4)?);
    let body = b.get(8..8 + size)?;
    if ty == T_CHOICE {
        let (csize, cty) = (rd(body, 8)? as usize, rd(body, 12)?);
        return Some((cty, body.get(16..16 + csize)?));
    }
    Some((ty, body))
}

/// The format, width and height of a `Format` object pod.
pub fn parse_format(p: &[u8]) -> Option<VideoFmt> {
    let size = rd(p, 0)? as usize;
    if rd(p, 4)? != T_OBJECT {
        return None;
    }
    let body = p.get(8..8 + size)?;
    let mut at = 8; // past the object type and id
    let (mut format, mut wh) = (None, None);
    while at + 16 <= body.len() {
        let key = rd(body, at)?;
        let vp = &body[at + 8..];
        let vsize = rd(vp, 0)? as usize;
        match (key, value(vp)) {
            (FORMAT_VIDEO_FORMAT, Some((T_ID, v))) => format = rd(v, 0),
            (FORMAT_VIDEO_SIZE, Some((T_RECT, v))) => wh = Some((rd(v, 0)?, rd(v, 4)?)),
            _ => {}
        }
        at += 8 + (8 + vsize).next_multiple_of(8);
    }
    let (w, h) = wh?;
    Some(VideoFmt { format: format?, w, h })
}

/// `r` clipped to a `w`×`h` picture.
fn clip(r: IRect, w: u32, h: u32) -> Option<(usize, usize, usize, usize)> {
    let x0 = r.0.max(0) as u32;
    let y0 = r.1.max(0) as u32;
    let x1 = (r.0 + r.2 as i32).clamp(0, w as i32) as u32;
    let y1 = (r.1 + r.3 as i32).clamp(0, h as i32) as u32;
    (x1 > x0 && y1 > y0).then_some((x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize))
}

/// The `crop` of a `fw`×`fh` 32-bit picture (`fmt`: BGRx/BGRA/RGBx/RGBA,
/// rows `stride` apart) as an `ow`×`oh` BGRA frame (alpha 255) into
/// `out`; what the picture does not cover (it shrank) is black. False for
/// a format that is not 32-bit RGB.
#[allow(clippy::too_many_arguments)]
pub fn convert_screen(fmt: u32, src: &[u8], stride: usize, fw: u32, fh: u32, crop: IRect, ow: u32, oh: u32, out: &mut Vec<u8>) -> bool {
    let swap = match fmt {
        VF_BGRX | VF_BGRA => false,
        VF_RGBX | VF_RGBA => true,
        _ => return false,
    };
    let (ow, oh) = (ow as usize, oh as usize);
    out.clear();
    out.resize(ow * oh * 4, 0);
    for p in out.as_chunks_mut::<4>().0 {
        p[3] = 255;
    }
    let Some((x0, y0, cw, ch)) = clip(crop, fw, fh) else { return true };
    let stride = if stride == 0 { fw as usize * 4 } else { stride };
    let (cw, ch) = (cw.min(ow), ch.min(oh));
    for y in 0..ch {
        let at = (y0 + y) * stride + x0 * 4;
        let Some(row) = src.get(at..at + cw * 4) else { break };
        let o = &mut out[y * ow * 4..][..cw * 4];
        for (d, s) in o.as_chunks_mut::<4>().0.iter_mut().zip(row.as_chunks::<4>().0) {
            *d = if swap { [s[2], s[1], s[0], 255] } else { [s[0], s[1], s[2], 255] };
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(b: &[u8]) -> Vec<u32> {
        b.as_chunks::<4>().0.iter().map(|c| u32::from_ne_bytes(*c)).collect()
    }

    #[test]
    fn primitive_pods_are_padded_to_8() {
        assert_eq!(words(&id(2)), [4, T_ID, 2, 0]);
        assert_eq!(words(&int(-1)), [4, T_INT, u32::MAX, 0]);
        assert_eq!(words(&pod(T_RECT, &u32s(&[640, 480]))), [8, T_RECT, 640, 480]);
    }

    #[test]
    fn choices_and_objects_have_the_spa_layout() {
        // Enum of two ids: kind, flags, child header {4, Id}, values.
        let c = choice(CHOICE_ENUM, T_ID, &[u32s(&[8]), u32s(&[7])]);
        assert_eq!(words(&c), [24, T_CHOICE, CHOICE_ENUM, 0, 4, T_ID, 8, 7]);
        // One value: the body is 20 bytes, padded to 24.
        let c = choice(CHOICE_FLAGS, T_INT, &[u32s(&[6])]);
        assert_eq!(words(&c), [20, T_CHOICE, CHOICE_FLAGS, 0, 4, T_INT, 6, 0]);
        // Object: type, id, then {key, flags, pod}.
        let o = object(OBJ_PARAM_META, PARAM_META, &[(META_TYPE, id(META_HEADER))]);
        assert_eq!(words(&o), [32, T_OBJECT, OBJ_PARAM_META, PARAM_META, META_TYPE, 0, 4, T_ID, META_HEADER, 0]);
    }

    #[test]
    fn the_enum_format_offers_shm_rgb() {
        let p = enum_format(&[VF_BGRX, VF_RGBX], (1920, 1080), 30);
        let w = words(&p);
        assert_eq!(&w[..4], &[w[0], T_OBJECT, OBJ_FORMAT, PARAM_ENUM_FORMAT]);
        assert_eq!(w[0] as usize + 8, p.len(), "size covers the body");
        // mediaType video, mediaSubtype raw.
        assert_eq!(&w[4..10], &[FORMAT_MEDIA_TYPE, 0, 4, T_ID, MEDIA_TYPE_VIDEO, 0]);
        assert_eq!(&w[10..16], &[FORMAT_MEDIA_SUBTYPE, 0, 4, T_ID, MEDIA_SUBTYPE_RAW, 0]);
        // Format: enum, default BGRx, then BGRx and RGBx (28 bytes, padded).
        assert_eq!(&w[16..26], &[FORMAT_VIDEO_FORMAT, 0, 28, T_CHOICE, CHOICE_ENUM, 0, 4, T_ID, VF_BGRX, VF_BGRX]);
        assert_eq!(w[26], VF_RGBX);
        // Size: range of rectangles, default 1920x1080.
        assert_eq!(&w[28..38], &[FORMAT_VIDEO_SIZE, 0, 40, T_CHOICE, CHOICE_RANGE, 0, 8, T_RECT, 1920, 1080]);
        // No modifier (DMA-BUF) is ever offered.
        assert!(!w.contains(&0x20002));
        // The negotiated format reads back as BGRx 1920x1080.
        assert_eq!(parse_format(&p), Some(VideoFmt { format: VF_BGRX, w: 1920, h: 1080 }));
    }

    #[test]
    fn a_fixed_format_parses() {
        let f = object(OBJ_FORMAT, PARAM_FORMAT, &[
            (FORMAT_MEDIA_TYPE, id(MEDIA_TYPE_VIDEO)),
            (FORMAT_VIDEO_FORMAT, id(VF_RGBX)),
            (FORMAT_VIDEO_SIZE, pod(T_RECT, &u32s(&[2560, 1440]))),
            (FORMAT_VIDEO_FRAMERATE, pod(T_FRACTION, &u32s(&[0, 1]))),
        ]);
        assert_eq!(parse_format(&f), Some(VideoFmt { format: VF_RGBX, w: 2560, h: 1440 }));
        assert_eq!(parse_format(&id(1)), None);
        assert_eq!(parse_format(&f[..20]), None, "truncated");
    }

    #[test]
    fn buffer_params_ask_for_mappable_memory() {
        let w = words(&buffers_param());
        let dt = w.iter().position(|&x| x == BUFFERS_DATATYPE).unwrap();
        assert_eq!(&w[dt..dt + 9], &[BUFFERS_DATATYPE, 0, 20, T_CHOICE, CHOICE_FLAGS, 0, 4, T_INT, 0b110]);
        let m = words(&meta_header_param());
        assert_eq!(&m[m.len() - 4..], &[4, T_INT, META_HEADER_SIZE, 0]);
        assert_eq!(aligned(&buffers_param()).len() * 8, buffers_param().len());
    }

    #[test]
    fn screens_are_cropped_and_converted() {
        // 4x2 BGRx, stride 20 (padded rows).
        let mut src = vec![0u8; 40];
        for y in 0..2 {
            for x in 0..4 {
                let i = y * 20 + x * 4;
                src[i..i + 4].copy_from_slice(&[x as u8, y as u8, 9, 0]);
            }
        }
        let mut out = Vec::new();
        assert!(convert_screen(VF_BGRX, &src, 20, 4, 2, (1, 0, 2, 2), 2, 2, &mut out));
        assert_eq!(out, [1, 0, 9, 255, 2, 0, 9, 255, 1, 1, 9, 255, 2, 1, 9, 255]);
        // RGBx swaps red and blue.
        assert!(convert_screen(VF_RGBX, &src, 20, 4, 2, (0, 0, 1, 1), 1, 1, &mut out));
        assert_eq!(out, [9, 0, 0, 255]);
        // A crop past the picture (it shrank): black beyond it.
        assert!(convert_screen(VF_BGRA, &src, 20, 4, 2, (3, 1, 2, 2), 2, 2, &mut out));
        assert_eq!(out, [3, 1, 9, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
        assert!(!convert_screen(VF_NV12, &src, 20, 4, 2, (0, 0, 1, 1), 1, 1, &mut out));
    }
}
