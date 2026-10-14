//! Drawing shared by the History and Edit windows (boards 10 and 11):
//! their color tokens ([`WTok`]), raised surfaces with the four-layer
//! shadow, the transparency checkerboard, pictures blitted into
//! (rounded) rects, key caps, menus and dialogs.

use crate::objects::FRect;
use crate::pixbuf::PixBuf;
use crate::theme::{rgba, Theme};
use crate::uifb::{Fb, C4};

const fn hex(v: u32) -> C4 {
    rgba(v, 1000)
}

/// The window boards' tokens for one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WTok {
    pub dark: bool,
    pub win: C4,
    pub rail: C4,
    /// The Edit window's canvas behind the picture.
    pub cv: C4,
    pub bd: C4,
    pub sep: C4,
    pub ic: C4,
    pub ich: C4,
    pub hv: C4,
    pub sf2: C4,
    pub ac: C4,
    pub acbg: C4,
    pub acbg_hover: C4,
    pub acrg: C4,
    pub acfg: C4,
    pub tx: C4,
    pub tx2: C4,
    pub tx3: C4,
    pub er: C4,
    pub erbg: C4,
    pub errg: C4,
    pub chk1: C4,
    pub chk2: C4,
    pub rim: C4,
    /// The four shadow layers (`theme::SHADOW` offsets).
    pub s: [C4; 4],
    /// A search match's highlight.
    pub mark: C4,
    /// The one solid destructive button ("Clear history").
    pub solid: C4,
    /// Over the window behind a dialog.
    pub scrim: C4,
    /// Badges and the quick-action strip over pictures (always dark).
    pub badge: C4,
    pub strip: C4,
    pub strip_fg: C4,
    pub tip_bg: C4,
    pub tip_fg: C4,
    pub key_bg: C4,
    pub key_fg: C4,
    /// A raised panel (menus, the action bar, the mini-toolbar).
    pub pop: C4,
    /// A checkbox's empty box.
    pub cb: C4,
}

impl WTok {
    pub fn of(th: &Theme) -> WTok {
        let (ac, acbg, acrg, acfg) = (th.accent, th.accent_bg, th.accent_ring, th.accent_fg);
        let common = |t: WTok| WTok { ac, acbg, acbg_hover: th.accent_bg_hover, acrg, acfg, ..t };
        if th.dark {
            common(WTok {
                dark: true,
                win: hex(0x1B1C20),
                rail: hex(0x181A1F),
                cv: hex(0x141519),
                bd: rgba(0xFFFFFF, 90),
                sep: rgba(0xFFFFFF, 80),
                ic: hex(0xA3A7B3),
                ich: hex(0xECEEF3),
                hv: rgba(0xFFFFFF, 60),
                sf2: rgba(0xFFFFFF, 50),
                ac,
                acbg,
                acbg_hover: acbg,
                acrg,
                acfg,
                tx: hex(0xECEEF3),
                tx2: hex(0x9296A3),
                tx3: hex(0x6C7080),
                er: hex(0xFF7A7A),
                erbg: rgba(0xFF6B6B, 160),
                errg: rgba(0xFF6B6B, 300),
                chk1: hex(0x262830),
                chk2: hex(0x2C2E37),
                rim: rgba(0xFFFFFF, 220),
                s: [rgba(0, 260), rgba(0, 150), rgba(0, 80), rgba(0, 40)],
                mark: rgba(0xFFC532, 280),
                solid: hex(0xE5484D),
                scrim: rgba(0x07080B, 450),
                badge: rgba(0x0E0F12, 820),
                strip: rgba(0x0E0F12, 860),
                strip_fg: hex(0xD4D7DF),
                tip_bg: rgba(0x0E0F12, 970),
                tip_fg: hex(0xF2F3F6),
                key_bg: rgba(0xFFFFFF, 120),
                key_fg: hex(0xD4D7DF),
                pop: rgba(0x1B1C20, 980),
                cb: rgba(0xFFFFFF, 300),
            })
        } else {
            common(WTok {
                dark: false,
                win: hex(0xFAFAFC),
                rail: hex(0xF2F3F6),
                cv: hex(0xE4E6EC),
                bd: rgba(0, 80),
                sep: rgba(0, 80),
                ic: hex(0x5F6473),
                ich: hex(0x15171E),
                hv: rgba(0, 60),
                sf2: rgba(0, 40),
                ac,
                acbg,
                acbg_hover: acbg,
                acrg,
                acfg,
                tx: hex(0x15171E),
                tx2: hex(0x5F6473),
                tx3: hex(0x8A8F9C),
                er: hex(0xC62828),
                erbg: rgba(0xC62828, 100),
                errg: rgba(0xC62828, 300),
                chk1: hex(0xE9EAEF),
                chk2: hex(0xF3F4F7),
                rim: rgba(0, 180),
                s: [rgba(0, 140), rgba(0, 90), rgba(0, 50), rgba(0, 25)],
                mark: rgba(0xFFC532, 450),
                solid: hex(0xE5484D),
                scrim: rgba(0x07080B, 300),
                badge: rgba(0x0E0F12, 820),
                strip: rgba(0x0E0F12, 860),
                strip_fg: hex(0xE4E6EB),
                tip_bg: rgba(0x0E0F12, 970),
                tip_fg: hex(0xF2F3F6),
                key_bg: rgba(0, 70),
                key_fg: hex(0x3B3F4B),
                pop: rgba(0xFFFFFF, 990),
                cb: rgba(0, 300),
            })
        }
    }

    /// The OS title bar in this theme: the window surface (`win`, what
    /// every window draws along its top edge) with the board's 1 px
    /// border (`bd` over `win`), so the caption reads as part of the
    /// design rather than a white strip over a dark window.
    pub fn frame(&self) -> crate::wind::FrameColors {
        let over = |c: C4, o: C4| {
            let a = u32::from(o.a);
            let mix = |b: u8, f: u8| ((u32::from(b) * (255 - a) + u32::from(f) * a + 127) / 255) as u8;
            [mix(c.r, o.r), mix(c.g, o.g), mix(c.b, o.b)]
        };
        let w = self.win;
        crate::wind::FrameColors { dark: self.dark, caption: [w.r, w.g, w.b], border: over(w, self.bd) }
    }

    /// The Edit window's checkerboard (darker in light mode: board 11).
    #[allow(dead_code)] // the Edit window (next)
    pub fn edit_checks(&self) -> (C4, C4) {
        if self.dark { (self.chk1, self.chk2) } else { (hex(0xD4D7DF), hex(0xDEE1E8)) }
    }
}

/// Logical rect → physical px (rounded).
pub fn px(r: FRect, k: f32) -> FRect {
    FRect { x: (r.x * k).round(), y: (r.y * k).round(), w: (r.w * k).round(), h: (r.h * k).round() }
}

pub fn fill(f: &mut Fb, r: FRect, radius: f32, c: C4) {
    // (Tests: an opaque fill hides the text under it.)
    #[cfg(test)]
    if c.a == 255 {
        crate::ui::preview::note_cover(Some(r));
    }
    f.fill_rounded(r.x as i32, r.y as i32, r.w as i32, r.h as i32, radius, c);
}

/// A 1 px (at scale `k`) ring inside `r`.
pub fn ring(f: &mut Fb, r: FRect, radius: f32, k: f32, c: C4) {
    let lw = k.round().max(1.0);
    f.stroke_rounded(FRect { x: r.x + lw / 2.0, y: r.y + lw / 2.0, w: r.w - lw, h: r.h - lw }, (radius - lw / 2.0).max(0.0), lw, c);
}

/// A `w`-thick ring just outside `r` (the selection ring of a card).
pub fn outer_ring(f: &mut Fb, r: FRect, radius: f32, gap: f32, w: f32, c: C4) {
    let o = FRect { x: r.x - gap - w / 2.0, y: r.y - gap - w / 2.0, w: r.w + 2.0 * gap + w, h: r.h + 2.0 * gap + w };
    f.stroke_rounded(o, radius + gap + w / 2.0, w, c);
}

/// The boards' raised shadow under `r` (physical px): `layers` of
/// `theme::SHADOW` (offset y, spread) in the token colors.
pub fn shadow(f: &mut Fb, t: &WTok, r: FRect, radius: f32, k: f32, layers: &[usize]) {
    #[cfg(test)]
    crate::ui::preview::note_cover(Some(r));
    for &i in layers.iter().rev() {
        let (dy, sp) = (crate::theme::SHADOW[i].0 * k, crate::theme::SHADOW[i].1 * k);
        let s = FRect { x: (r.x - sp).round(), y: (r.y + dy - sp).round(), w: (r.w + 2.0 * sp).round(), h: (r.h + 2.0 * sp).round() };
        fill(f, s, radius + sp, t.s[i]);
    }
}

/// A raised panel: shadow, fill, 1 px border.
#[allow(dead_code)] // the Edit window (next)
pub fn panel(f: &mut Fb, t: &WTok, r: FRect, radius: f32, k: f32, bg: C4) {
    shadow(f, t, r, radius, k, &[0, 1, 2, 3]);
    fill(f, r, radius, bg);
    ring(f, r, radius, k, t.bd);
}

/// The transparency checkerboard (`cell` physical px) in `r`.
pub fn checker(f: &mut Fb, r: FRect, cell: f32, a: C4, b: C4) {
    let cell = cell.max(1.0) as i32;
    let (x0, y0, x1, y1) = (r.x as i32, r.y as i32, r.x1() as i32, r.y1() as i32);
    f.fill_rect(x0, y0, x1 - x0, y1 - y0, b);
    let mut y = y0;
    while y < y1 {
        let row = (y - y0) / cell;
        let mut x = x0 + if row % 2 == 0 { cell } else { 0 };
        while x < x1 {
            f.fill_rect(x, y, cell.min(x1 - x), cell.min(y1 - y), a);
            x += 2 * cell;
        }
        y += cell;
    }
}

/// Coverage of pixel (x, y) by the rounded rect `r` (radius `rad`), 0..1.
fn rounded_cov(r: FRect, rad: f32, x: i32, y: i32) -> f32 {
    if rad <= 0.0 {
        return 1.0;
    }
    let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
    let (ix0, ix1, iy0, iy1) = (r.x + rad, r.x1() - rad, r.y + rad, r.y1() - rad);
    let qx = if cx < ix0 { ix0 - cx } else if cx > ix1 { cx - ix1 } else { return 1.0 };
    let qy = if cy < iy0 { iy0 - cy } else if cy > iy1 { cy - iy1 } else { return 1.0 };
    (rad + 0.5 - qx.hypot(qy)).clamp(0.0, 1.0)
}

/// `img` drawn into `dst` (physical px, any size: bilinear when scaled),
/// clipped to the rounded rect `clip` (radius `rad`).
pub fn blit(f: &mut Fb, img: &PixBuf, dst: FRect, clip: FRect, rad: f32) {
    let (iw, ih) = (img.width() as usize, img.height() as usize);
    if iw == 0 || ih == 0 || dst.w < 1.0 || dst.h < 1.0 {
        return;
    }
    let (bx0, by0) = (f.ox, f.oy);
    let (bx1, by1) = (f.ox + f.stride as i32, f.oy + f.height());
    let x0 = (dst.x.max(clip.x).floor() as i32).max(bx0);
    let y0 = (dst.y.max(clip.y).floor() as i32).max(by0);
    let x1 = (dst.x1().min(clip.x1()).ceil() as i32).min(bx1);
    let y1 = (dst.y1().min(clip.y1()).ceil() as i32).min(by1);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let src = img.as_raw();
    let (sx, sy) = (iw as f32 / dst.w, ih as f32 / dst.h);
    let same = (dst.w.round() as usize == iw) && (dst.h.round() as usize == ih);
    let bgra = f.order == crate::raster::Order::Bgra;
    // A mirrored frame: the picture pre-flipped in its box (it reads
    // normally after the frame's flip).
    let flip = (dst.x + dst.x1()).round() as i32 - 1;
    let mirror = f.mirror;
    for y in y0..y1 {
        for x in x0..x1 {
            let x_dst = x;
            let x = if mirror { flip - x } else { x };
            let p = if same {
                let (u, v) = (((x as f32 - dst.x).round() as usize).min(iw - 1), ((y as f32 - dst.y).round() as usize).min(ih - 1));
                let i = (v * iw + u) * 4;
                [src[i], src[i + 1], src[i + 2], src[i + 3]]
            } else {
                // Bilinear at the pixel centre.
                let u = ((x as f32 + 0.5 - dst.x) * sx - 0.5).clamp(0.0, (iw - 1) as f32);
                let v = ((y as f32 + 0.5 - dst.y) * sy - 0.5).clamp(0.0, (ih - 1) as f32);
                let (u0, v0) = (u.floor() as usize, v.floor() as usize);
                let (u1, v1) = ((u0 + 1).min(iw - 1), (v0 + 1).min(ih - 1));
                let (fu, fv) = (u - u0 as f32, v - v0 as f32);
                let at = |uu: usize, vv: usize, c: usize| src[(vv * iw + uu) * 4 + c] as f32;
                let mut o = [0u8; 4];
                for (c, oc) in o.iter_mut().enumerate() {
                    let top = at(u0, v0, c) * (1.0 - fu) + at(u1, v0, c) * fu;
                    let bot = at(u0, v1, c) * (1.0 - fu) + at(u1, v1, c) * fu;
                    *oc = (top * (1.0 - fv) + bot * fv).round() as u8;
                }
                o
            };
            let x = x_dst;
            let cov = rounded_cov(clip, rad, x, y);
            let a = (p[3] as f32 * cov).round() as u8;
            if a == 0 {
                continue;
            }
            let (r, g, b) = if bgra { (p[2], p[1], p[0]) } else { (p[0], p[1], p[2]) };
            if a == 255 {
                let i = ((y - by0) as usize * f.stride + (x - bx0) as usize) * 4;
                f.d[i..i + 4].copy_from_slice(&[r, g, b, 255]);
            } else {
                // `blend_px` takes RGBA and swaps itself.
                f.blend_px(x, y, C4::new(p[0], p[1], p[2], a));
            }
        }
    }
}

/// `img` contained (aspect kept, centred, never enlarged past `max_up`)
/// in `r`: the rect it takes.
pub fn contain(img: (u32, u32), r: FRect, max_up: f32) -> FRect {
    let (iw, ih) = (img.0.max(1) as f32, img.1.max(1) as f32);
    let k = (r.w / iw).min(r.h / ih).min(max_up);
    let (w, h) = ((iw * k).round(), (ih * k).round());
    FRect { x: (r.x + (r.w - w) / 2.0).round(), y: (r.y + (r.h - h) / 2.0).round(), w, h }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blit_copies_and_scales() {
        let mut d = vec![0u8; 8 * 8 * 4];
        let img = PixBuf::from_pixel(4, 4, [200, 100, 50, 255]);
        let mut f = Fb::new(&mut d, 8);
        let r = FRect { x: 2.0, y: 2.0, w: 4.0, h: 4.0 };
        blit(&mut f, &img, r, r, 0.0);
        assert_eq!(&d[(3 * 8 + 3) * 4..(3 * 8 + 3) * 4 + 4], &[200, 100, 50, 255]);
        assert_eq!(&d[..4], &[0, 0, 0, 0], "outside the rect untouched");
        // Scaled to 8x8 the color stays.
        let mut d = vec![0u8; 8 * 8 * 4];
        let mut f = Fb::new(&mut d, 8);
        let all = FRect { x: 0.0, y: 0.0, w: 8.0, h: 8.0 };
        blit(&mut f, &img, all, all, 0.0);
        assert!(d.as_chunks::<4>().0.iter().all(|p| *p == [200, 100, 50, 255]));
        // Rounded corners leave the corner pixel alone.
        let mut d = vec![0u8; 8 * 8 * 4];
        let mut f = Fb::new(&mut d, 8);
        blit(&mut f, &img, all, all, 3.0);
        assert_eq!(&d[..4], &[0, 0, 0, 0]);
        assert_eq!(contain((200, 100), FRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, 1.0), FRect { x: 0.0, y: 25.0, w: 100.0, h: 50.0 });
        assert_eq!(contain((20, 10), FRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, 1.0).w, 20.0, "never enlarged");
    }

    #[test]
    fn checker_alternates() {
        let mut d = vec![0u8; 4 * 2 * 4];
        let mut f = Fb::new(&mut d, 4);
        let (a, b) = (C4::rgb(1, 1, 1), C4::rgb(9, 9, 9));
        checker(&mut f, FRect { x: 0.0, y: 0.0, w: 4.0, h: 2.0 }, 1.0, a, b);
        let px = |i: usize| d[i * 4];
        assert_eq!([px(0), px(1), px(2), px(3)], [9, 1, 9, 1]);
        assert_eq!([px(4), px(5)], [1, 9]);
    }
}
