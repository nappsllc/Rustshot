//! The camera bubble's background blur (`rec_camera_bg`: off, light,
//! strong). Per camera picture, on the camera thread ([`Effect::apply`]):
//!
//! 1. Now and then ([`Pacer`]) a small copy of the frame goes to the
//!    segmentation thread ([`seg::SegInput::prepare`]), which finds the
//!    person in it (the MediaPipe model, downloaded on first use, through [`seg::Net`],
//!    or macOS's Vision) and hands back the mask of the bubble's square.
//!    The camera thread never waits for it: until a new mask comes, the
//!    last one is used.
//! 2. Masks are smoothed over time ([`EMA`]: the newest counts 60 %), so
//!    the edge does not flicker.
//! 3. The low-resolution mask is brought to the bubble's size by a fast
//!    guided filter ([`refine`]): fitted to the picture's own edges at the
//!    mask's resolution, upsampled bilinearly, applied at full size.
//! 4. The background is blurred with the Blur tool's box blur
//!    ([`crate::objects::blur::blur_block_on`], on the camera thread
//!    alone; sigma 8 or 18 bubble px) with the person cut out of it
//!    (blurred weights, so the person's
//!    colors do not bleed into the blur as a halo), and the sharp person
//!    is laid over it ([`composite`]).
//!
//! Segmentation that takes longer than [`BUDGET`] runs on every other
//! picture only; the bubble keeps the camera's rate either way.
//!
//! A blur chosen but no mask to blur with (the model still downloading,
//! not to be had, not loading; the first picture before its mask): the
//! whole picture is blurred heavily ([`blur_all`]) and the bubble says
//! why ([`LOADING_BADGE`], [`UNAVAILABLE_BADGE`]). The camera's own
//! picture is never shown while a blur is chosen.

use super::seg::ops::F4;
use super::seg::{self, Segment, SegInput, SqMask};
use super::CamFrame;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

/// `rec_camera_bg`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bg {
    #[default]
    Off = 0,
    Light = 1,
    Strong = 2,
}

impl Bg {
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: [Bg; 3] = [Bg::Off, Bg::Light, Bg::Strong];

    /// "off" | "light" | "strong" (anything else: off).
    pub fn from_config(s: &str) -> Bg {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => Bg::Light,
            "strong" => Bg::Strong,
            _ => Bg::Off,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Bg::Off => "off",
            Bg::Light => "light",
            Bg::Strong => "strong",
        }
    }

    /// For an atomic between the bubble and its camera thread.
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Bg {
        match v {
            1 => Bg::Light,
            2 => Bg::Strong,
            _ => Bg::Off,
        }
    }

    /// The blur's standard deviation in bubble pixels.
    pub fn sigma(self) -> f32 {
        match self {
            Bg::Off => 0.0,
            Bg::Light => 8.0,
            Bg::Strong => 18.0,
        }
    }
}

/// The Background row's note while downloads are off and the model is
/// not here (Settings says where to put it).
pub const UNAVAILABLE: &str = "Blur unavailable — downloads are off";
/// The model file there is not this version's.
pub const DAMAGED: &str = "Blur unavailable: model file is damaged";
/// The capture panel's note after a failed download (the row retries).
pub const FAILED_SHORT: &str = "Couldn't download the model · Retry";
/// Settings' note after a failed download.
pub const FAILED: &str = "Blur unavailable — couldn't download the model";
/// The record-start question's first line.
pub const ASK_TITLE: &str = "Background blur isn't available";
/// Its answers.
pub const ASK_BLURRED: &str = "Record with the camera fully blurred";
pub const ASK_CAM_OFF: &str = "Turn the camera off";
pub const ASK_CANCEL: &str = "Cancel";

/// The bubble's badge while it is fully blurred until the model comes.
pub const LOADING_BADGE: &str = "Blur model loading\u{2026}";
/// The bubble's badge while it is fully blurred for want of the model.
pub const UNAVAILABLE_BADGE: &str = "Blur unavailable";

/// The capture panel's progress notes, by tenth.
const PROGRESS: [&str; 11] = [
    "Downloading model… 0%",
    "Downloading model… 10%",
    "Downloading model… 20%",
    "Downloading model… 30%",
    "Downloading model… 40%",
    "Downloading model… 50%",
    "Downloading model… 60%",
    "Downloading model… 70%",
    "Downloading model… 80%",
    "Downloading model… 90%",
    "Downloading model… 100%",
];

/// Whether the background can be blurred, and if not, why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blur {
    /// The model is here (or macOS Vision segments).
    Ready,
    /// Not here yet: choosing a blur downloads it.
    Fetchable,
    /// On its way (percent).
    Downloading(u8),
    /// The download failed (why); choosing a blur again retries.
    Failed(String),
    /// Downloads are off: the file to put where.
    Off { asset: &'static str, url: String, path: std::path::PathBuf },
    /// Downloads are off and the file there does not match.
    Damaged(std::path::PathBuf),
}

impl Blur {
    /// The record-start question's two lines when a blur cannot be had
    /// ("Background blur isn't available", "(why)").
    pub fn ask_lines(&self) -> Option<(&'static str, &'static str)> {
        let why = match self {
            Blur::Ready => return None,
            Blur::Fetchable | Blur::Downloading(_) => "(the blur model is still downloading)",
            Blur::Failed(_) => "(couldn't download the blur model)",
            Blur::Off { .. } => "(downloads are off)",
            Blur::Damaged(_) => "(the blur model file is damaged)",
        };
        Some((ASK_TITLE, why))
    }

    /// The capture panel's Background row: its note when the choices are
    /// not offered (and whether a press retries).
    pub fn panel_note(&self) -> Option<&'static str> {
        match self {
            Blur::Ready | Blur::Fetchable => None,
            Blur::Downloading(p) => Some(PROGRESS[(*p as usize).min(100) / 10]),
            Blur::Failed(_) => Some(FAILED_SHORT),
            Blur::Off { .. } => Some(UNAVAILABLE),
            Blur::Damaged(_) => Some(DAMAGED),
        }
    }

    /// Settings › Recording › Background's note.
    pub fn settings_note(&self) -> String {
        match self {
            Blur::Ready => "Blurs what is behind you".into(),
            Blur::Fetchable => format!("Blurs what is behind you (first use downloads a {} model from GitHub)", crate::fetch::size_label(crate::fetch::MODEL.size)),
            Blur::Downloading(p) => format!("Downloading the blur model… {p}%"),
            Blur::Failed(why) => format!("{FAILED}: {why}"),
            Blur::Off { asset, url, path } => format!("{UNAVAILABLE}: download {asset} ({url}) and {}", save_as(path)),
            Blur::Damaged(p) => format!("{DAMAGED} ({})", p.display()),
        }
    }

    /// Settings' second line when downloads are off or the file is
    /// damaged: which file to put where.
    pub fn settings_detail(&self) -> Option<String> {
        let f = &crate::fetch::MODEL;
        let url = f.url(env!("CARGO_PKG_VERSION"));
        match self {
            Blur::Off { asset, url, path } => Some(format!("To blur, download {asset} ({url}) and {}", save_as(path))),
            Blur::Damaged(p) => Some(format!("{} is not this version's model: download {} ({url}) again and save it there", p.display(), f.asset)),
            _ => None,
        }
    }
}

#[cfg(test)]
thread_local! {
    static STATE: std::cell::RefCell<Option<Blur>> = const { std::cell::RefCell::new(None) };
}

/// "save it as <path>" (no data folder at all: where it can go instead).
fn save_as(path: &std::path::Path) -> String {
    if path.as_os_str().is_empty() {
        format!("put it in a folder named by RUSTSHOT_DATA_DIR as {}", crate::fetch::MODEL.rel)
    } else {
        format!("save it as {}", path.display())
    }
}

/// Tests: pretend the model is here (`Some(true)`), or downloads are off
/// and it is not (`Some(false)`).
#[cfg(test)]
pub fn set_available(v: Option<bool>) {
    set_state(v.map(|on| if on { Blur::Ready } else { Blur::Off { asset: crate::fetch::MODEL.asset, url: String::new(), path: "models/selfie.bin".into() } }));
}

/// Tests: pretend the blur is in this state.
#[cfg(test)]
pub fn set_state(v: Option<Blur>) {
    STATE.with(|a| *a.borrow_mut() = v);
}

/// Where the blur stands: macOS Vision (12+) always blurs; otherwise the
/// model file ([`crate::fetch::MODEL`]).
pub fn state() -> Blur {
    #[cfg(test)]
    if let Some(v) = STATE.with(|a| a.borrow().clone()) {
        return v;
    }
    #[cfg(target_os = "macos")]
    if super::seg_macos::available() {
        return Blur::Ready;
    }
    let f = &crate::fetch::MODEL;
    match crate::fetch::state(f) {
        crate::fetch::State::Ready(_) => Blur::Ready,
        crate::fetch::State::Missing => Blur::Fetchable,
        crate::fetch::State::Downloading { got, total } => Blur::Downloading((got * 100 / total.max(1)).min(100) as u8),
        crate::fetch::State::Failed(e) => Blur::Failed(e),
        crate::fetch::State::Off(path) => Blur::Off { asset: f.asset, url: f.url(env!("CARGO_PKG_VERSION")), path },
        crate::fetch::State::Damaged(p) => Blur::Damaged(p),
    }
}

/// Whether the model is on its way (windows showing its progress poll).
pub fn downloading() -> bool {
    matches!(state(), Blur::Downloading(_))
}

/// A blur was chosen: get the model if it is not here (in the background;
/// the bubble blurs once it is). A failed download is tried again.
pub fn request_model() {
    // (Tests never go to the network: they count.)
    #[cfg(test)]
    REQUESTS.with(|n| n.set(n.get() + 1));
    #[cfg(not(test))]
    request_model_now();
}

#[cfg(test)]
thread_local! {
    static REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// [`request_model`] calls on this thread (tests).
#[cfg(test)]
pub fn requests() -> usize {
    REQUESTS.with(|n| n.get())
}

#[cfg(not(test))]
fn request_model_now() {
    #[cfg(target_os = "macos")]
    if super::seg_macos::available() {
        return;
    }
    let f = &crate::fetch::MODEL;
    crate::fetch::ensure(f.asset, f.rel, |r| {
        if let Err(e) = r {
            eprintln!("rustshot: camera blur: {e}");
        }
    });
}

/// Whether the segmenter can be made now (the model is here), and if not
/// whether it may still come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Ready,
    Loading,
    Unavailable,
}

fn engine_ready() -> Engine {
    match state() {
        Blur::Ready => Engine::Ready,
        Blur::Fetchable | Blur::Downloading(_) => Engine::Loading,
        Blur::Failed(_) | Blur::Off { .. } | Blur::Damaged(_) => Engine::Unavailable,
    }
}

/// The segmenter of this build (`None`: the model is not here or does not
/// load: said on stderr).
pub fn default_engine() -> Option<Box<dyn Segment>> {
    #[cfg(target_os = "macos")]
    if let Some(v) = super::seg_macos::Vision::new() {
        return Some(Box::new(v));
    }
    // Bytes that do not load although the file looked right: looked at
    // again (it shows as damaged or missing, not ready).
    let Some(bytes) = seg::model_bytes() else {
        crate::fetch::MODEL.forget();
        return None;
    };
    match seg::Net::selfie(&bytes) {
        Ok(n) => Some(Box::new(n)),
        Err(e) => {
            eprintln!("rustshot: camera blur: {e}");
            crate::fetch::MODEL.forget();
            None
        }
    }
}

/// Segmentation slower than this (on average) runs on every other picture.
pub const BUDGET_MS: f32 = 6.0;
/// The newest mask's weight in the temporal smoothing.
pub const EMA: f32 = 0.6;

/// When to segment: every picture while segmentation keeps within
/// [`BUDGET_MS`], every other one while it does not, never while the
/// last one is still running (its mask is used meanwhile).
#[derive(Clone, Debug, Default)]
pub struct Pacer {
    /// Mean segmentation time (ms), smoothed.
    avg: Option<f32>,
    /// Pictures seen.
    n: u64,
}

impl Pacer {
    /// Picture `n + 1`: segment it?
    pub fn want(&mut self, busy: bool) -> bool {
        self.n += 1;
        !busy && (self.every() == 1 || self.n.is_multiple_of(2))
    }

    /// A segmentation took `ms`.
    pub fn took(&mut self, ms: f32) {
        self.avg = Some(match self.avg {
            Some(a) => a * 0.8 + ms * 0.2,
            None => ms,
        });
    }

    /// Segment every `every()`-th picture.
    pub fn every(&self) -> u32 {
        if self.avg.is_some_and(|a| a > BUDGET_MS) { 2 } else { 1 }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn avg_ms(&self) -> Option<f32> {
        self.avg
    }
}

/// The temporally smoothed low-resolution mask.
#[derive(Clone, Debug, Default)]
pub struct Smooth {
    pub m: SqMask,
}

impl Smooth {
    /// Fold in a new mask (a different size starts over). Values are
    /// taken as 0..1 (anything else, a NaN included, as 0), so one bad
    /// mask cannot stay in the average.
    pub fn push(&mut self, new: &SqMask) {
        let ok = |q: f32| if q.is_finite() { q.clamp(0.0, 1.0) } else { 0.0 };
        if self.m.n != new.n || self.m.v.len() != new.v.len() {
            self.m.n = new.n;
            self.m.v.clear();
            self.m.v.extend(new.v.iter().map(|&q| ok(q)));
            return;
        }
        for (p, &q) in self.m.v.iter_mut().zip(&new.v) {
            *p = EMA * ok(q) + (1.0 - EMA) * *p;
        }
    }

    pub fn has(&self) -> bool {
        self.m.n > 0
    }

    pub fn clear(&mut self) {
        self.m.n = 0;
        self.m.v.clear();
    }
}

/// Mean over a `(2r + 1)²` window (clipped at the edges) of an `n`x`n`
/// field, separable running sums.
fn box_mean(src: &[f32], n: usize, r: usize, out: &mut Vec<f32>, tmp: &mut Vec<f32>) {
    tmp.clear();
    tmp.resize(n * n, 0.0);
    out.clear();
    out.resize(n * n, 0.0);
    for y in 0..n {
        let row = &src[y * n..][..n];
        let mut s: f32 = row[..r.min(n - 1) + 1].iter().sum();
        for x in 0..n {
            let (lo, hi) = (x.saturating_sub(r), (x + r).min(n - 1));
            tmp[y * n + x] = s / (hi - lo + 1) as f32;
            if x + r + 1 < n {
                s += row[x + r + 1];
            }
            if x >= r {
                s -= row[x - r];
            }
        }
    }
    let mut s: Vec<f32> = vec![0.0; n];
    for y in 0..=r.min(n - 1) {
        for (a, v) in s.iter_mut().zip(&tmp[y * n..][..n]) {
            *a += v;
        }
    }
    for y in 0..n {
        let (lo, hi) = (y.saturating_sub(r), (y + r).min(n - 1));
        let k = 1.0 / (hi - lo + 1) as f32;
        for (o, a) in out[y * n..][..n].iter_mut().zip(&s) {
            *o = a * k;
        }
        if y + r + 1 < n {
            for (a, v) in s.iter_mut().zip(&tmp[(y + r + 1) * n..][..n]) {
                *a += v;
            }
        }
        if y >= r {
            for (a, v) in s.iter_mut().zip(&tmp[(y - r) * n..][..n]) {
                *a -= v;
            }
        }
    }
}

/// Luma (0..=255) of a BGRA pixel (BT.601 weights in 1/256).
#[inline(always)]
fn luma8(p: &[u8; 4]) -> u32 {
    (29 * p[0] as u32 + 150 * p[1] as u32 + 77 * p[2] as u32 + 128) >> 8
}

/// The guided filter's window radius at the mask's resolution, and its
/// regularization (larger: smoother, less edge-following).
const GF_R: usize = 2;
const GF_EPS: f32 = 2e-3;
/// The refined mask's contrast curve: below the first, background; above
/// the second, person.
const EDGE: (f32, f32) = (0.2, 0.8);

/// Scratch kept between pictures.
#[derive(Default)]
pub struct Scratch {
    f: [Vec<f32>; 12],
    blur: Vec<u8>,
    /// The half-size picture (plain means) and the bilinear taps.
    plain: Vec<u8>,
    taps: Vec<(usize, usize, f32)>,
    tmp: Vec<u8>,
    acc: Vec<u16>,
}

/// The guided fit's resolution for an `n` px mask: halved (the fast
/// guided filter's subsampling) while over 96 px.
fn fit_n(n: usize) -> usize {
    if n > 96 { n / 2 } else { n }
}

/// `m` (the smoothed mask of the square) at the picture's size `d`x`d`,
/// 0..=255 into `out`: a fast guided filter with the picture as guide.
/// The linear fit of mask to luma is made at a low resolution
/// ([`fit_n`]), its two coefficients upsampled bilinearly (along the
/// rows, then between rows four lanes at a time) and applied to the
/// full-size luma.
pub fn refine(m: &SqMask, pic: &CamFrame, out: &mut Vec<u8>, s: &mut Scratch) {

    let (n0, d) = (m.n, pic.w as usize);
    out.clear();
    if n0 == 0 || d == 0 || pic.h as usize != d || pic.bgra.len() < d * d * 4 || m.v.len() < n0 * n0 {
        return;
    }
    let n = fit_n(n0);
    let [gi, mi, mp, ip, ii, a, b, t, u, v, ah, bh] = &mut s.f;
    // The mask at the fit's resolution (means of 2x2 when halved).
    let p: &[f32] = if n == n0 {
        &m.v[..n * n]
    } else {
        t.clear();
        for j in 0..n {
            for i in 0..n {
                let (r0, r1) = (&m.v[2 * j * n0..][..n0], &m.v[(2 * j + 1) * n0..][..n0]);
                t.push((r0[2 * i] + r0[2 * i + 1] + r1[2 * i] + r1[2 * i + 1]) * 0.25);
            }
        }
        std::mem::swap(t, v);
        &v[..]
    };
    // The guide there: the mean luma over each low pixel's footprint.
    gi.clear();
    gi.resize(n * n, 0.0);
    let span = |i: usize| (i * d / n, ((i + 1) * d / n).max(i * d / n + 1).min(d));
    for j in 0..n {
        let (y0, y1) = span(j);
        for i in 0..n {
            let (x0, x1) = span(i);
            let mut acc = 0u32;
            for y in y0..y1 {
                for q in pic.bgra[(y * d + x0) * 4..(y * d + x1) * 4].as_chunks::<4>().0 {
                    acc += luma8(q);
                }
            }
            gi[j * n + i] = acc as f32 / (255 * (y1 - y0) * (x1 - x0)) as f32;
        }
    }
    box_mean(gi, n, GF_R, mi, t);
    box_mean(p, n, GF_R, mp, t);
    u.clear();
    u.extend(gi.iter().zip(p).map(|(i, q)| i * q));
    box_mean(u, n, GF_R, ip, t);
    u.clear();
    u.extend(gi.iter().map(|i| i * i));
    box_mean(u, n, GF_R, ii, t);
    // a = cov(I, p) / (var(I) + eps), b = mean(p) - a mean(I); then their
    // means. (`v` may hold `p`: b goes to `u` first.)
    let (fa, fb) = (ah, bh);
    fa.clear();
    fb.clear();
    for k in 0..n * n {
        let var = ii[k] - mi[k] * mi[k];
        let cov = ip[k] - mi[k] * mp[k];
        let ak = cov / (var + GF_EPS);
        fa.push(ak);
        fb.push(mp[k] - ak * mi[k]);
    }
    box_mean(fa, n, GF_R, a, t);
    box_mean(fb, n, GF_R, b, t);
    // Along the rows: n rows of d (pixel centres aligned).
    let k = n as f32 / d as f32;
    let lerp = |i: usize| {
        let f = ((i as f32 + 0.5) * k - 0.5).clamp(0.0, (n - 1) as f32);
        let i0 = f as usize;
        (i0, (i0 + 1).min(n - 1), f - i0 as f32)
    };
    let xs: Vec<(usize, usize, f32)> = (0..d).map(lerp).collect();
    let (ah, bh) = (fa, fb);
    ah.clear();
    bh.clear();
    for j in 0..n {
        let (ra, rb) = (&a[j * n..][..n], &b[j * n..][..n]);
        for &(x0, x1, fx) in &xs {
            ah.push(ra[x0] + (ra[x1] - ra[x0]) * fx);
            bh.push(rb[x0] + (rb[x1] - rb[x0]) * fx);
        }
    }
    // The fit shrinks the mask's contrast near an edge (a window's mean
    // mask on either side): a smoothstep from EDGE.0 to EDGE.1 takes it
    // back to 0 and 1, keeping the edge (0.5) in place. As a table, from
    // q already scaled to 0..255 over that range: q' = q · ek - e0 · ek.
    let lut: [u8; 256] = std::array::from_fn(|i| {
        let t = i as f32 / 255.0;
        (t * t * (3.0 - 2.0 * t) * 255.0 + 0.5) as u8
    });
    let ek = 255.0 / (EDGE.1 - EDGE.0);
    let (k4, e4, l4) = (F4::splat(ek), F4::splat(-EDGE.0 * ek), F4::splat(ek / 255.0));
    let (zero, top) = (F4::splat(0.0), F4::splat(255.0));
    out.resize(d * d, 0);
    let d4 = d / 4 * 4;
    let lum = u;
    lum.resize(d, 0.0);
    let q = t;
    q.resize(d, 0.0);
    for y in 0..d {
        let (y0, y1, fy) = lerp(y);
        let row = pic.bgra[y * d * 4..][..d * 4].as_chunks::<4>().0;
        for (l, px) in lum.iter_mut().zip(row) {
            *l = luma8(px) as f32;
        }
        let (a0, a1, b0, b1) = (&ah[y0 * d..][..d], &ah[y1 * d..][..d], &bh[y0 * d..][..d], &bh[y1 * d..][..d]);
        let fy4 = F4::splat(fy);
        // SAFETY: every slice holds d values; lanes stay below d4 <= d.
        unsafe {
            let mut x = 0;
            while x < d4 {
                let (pa0, pb0) = (F4::ld(a0.as_ptr().add(x)), F4::ld(b0.as_ptr().add(x)));
                let av = pa0.mul_add(F4::ld(a1.as_ptr().add(x)).mul_add(pa0, F4::splat(-1.0)), fy4);
                let bv = pb0.mul_add(F4::ld(b1.as_ptr().add(x)).mul_add(pb0, F4::splat(-1.0)), fy4);
                // q · ek - e0 · ek with q = av · luma / 255 + bv.
                let r = av.mul(F4::ld(lum.as_ptr().add(x))).mul(l4).add(bv.mul(k4)).add(e4);
                r.max(zero).min(top).st(q.as_mut_ptr().add(x));
                x += 4;
            }
        }
        for x in d4..d {
            let av = a0[x] + (a1[x] - a0[x]) * fy;
            let bv = b0[x] + (b1[x] - b0[x]) * fy;
            q[x] = ((av * lum[x] / 255.0 + bv - EDGE.0) * ek).clamp(0.0, 255.0);
        }
        for (o, &v) in out[y * d..][..d].iter_mut().zip(q.iter()) {
            *o = lut[v as usize];
        }
    }
}


/// The background of `pic` (BGRA `d`x`d`, alpha 255) blurred at `sigma`
/// under the sharp person (`mask`, 255 = person): the blur takes the
/// background's colors only (each weighted by how much background it
/// is; the weights blurred alike and divided out), so no halo of the
/// person's colors spreads into it.
pub fn composite(pic: &mut CamFrame, mask: &[u8], sigma: f32, s: &mut Scratch) {
    let d = pic.w as usize;
    if d == 0 || pic.h as usize != d || mask.len() < d * d || pic.bgra.len() < d * d * 4 || sigma <= 0.0 {
        return;
    }
    // (Pixels are four f32 lanes, B G R A; the picture's alpha is 255, so
    // a pixel times its background weight w / 255 has w as its alpha.)
    let bl = &mut s.blur;
    bl.clear();
    let px = pic.bgra.as_chunks::<4>().0;
    if d < HALF_FROM {
        bl.resize(d * d * 4, 0);
        for ((o, p), &m) in bl.as_chunks_mut::<4>().0.iter_mut().zip(px).zip(mask) {
            *o = F4::from_px(*p).mul(F4::splat((255 - m) as f32 * (1.0 / 255.0))).to_px();
        }
        crate::objects::blur::blur_block_on(bl, d, d, sigma, &mut s.tmp, &mut s.acc, 1);
        for ((p, b), &m) in pic.bgra.as_chunks_mut::<4>().0.iter_mut().zip(bl.as_chunks::<4>().0).zip(mask) {
            if m != 255 {
                mix(p, F4::from_px(*b), m);
            }
        }
        return;
    }
    // Half size: the weighted 2x2 means (and the plain ones), blurred at
    // half the sigma; the background's color is worked out there (the
    // division, a quarter as often), then read back bilinearly (each full
    // pixel sits a quarter of a half pixel off a half pixel's centre:
    // weights 3:1) and laid under the person.
    let h = d.div_ceil(2);
    bl.resize(h * h * 4, 0);
    let plain = &mut s.plain;
    plain.clear();
    plain.resize(h * h * 4, 0);
    for j in 0..h {
        let (y0, y1) = (2 * j, (2 * j + 1).min(d - 1));
        for i in 0..h {
            let (x0, x1) = (2 * i, (2 * i + 1).min(d - 1));
            let (mut acc, mut sum) = (F4::splat(0.0), F4::splat(0.0));
            for at in [y0 * d + x0, y0 * d + x1, y1 * d + x0, y1 * d + x1] {
                let v = F4::from_px(px[at]);
                acc = acc.mul_add(v, F4::splat((255 - mask[at]) as f32 * (0.25 / 255.0)));
                sum = sum.mul_add(v, F4::splat(0.25));
            }
            bl[(j * h + i) * 4..][..4].copy_from_slice(&acc.to_px());
            plain[(j * h + i) * 4..][..4].copy_from_slice(&sum.to_px());
        }
    }
    crate::objects::blur::blur_block_on(bl, h, h, sigma / 2.0, &mut s.tmp, &mut s.acc, 1);
    for (b, q) in bl.as_chunks_mut::<4>().0.iter_mut().zip(plain.as_chunks::<4>().0) {
        *b = background(F4::from_px(*b), F4::from_px(*q)).to_px();
    }
    // Per full index: the two half pixels and the first one's weight.
    let taps = &mut s.taps;
    taps.clear();
    taps.extend((0..d).map(|x| {
        let i = x / 2;
        if x % 2 == 0 { (i.saturating_sub(1), i, if i == 0 { 1.0 } else { 0.25 }) } else { (i, (i + 1).min(h - 1), 0.75) }
    }));
    let bp = s.blur.as_chunks::<4>().0;
    for (y, &(y0, y1, wy)) in taps.iter().enumerate() {
        let (r0, r1) = (&bp[y0 * h..][..h], &bp[y1 * h..][..h]);
        let row = pic.bgra[y * d * 4..][..d * 4].as_chunks_mut::<4>().0;
        let (wy0, wy1) = (F4::splat(wy), F4::splat(1.0 - wy));
        for ((p, &(x0, x1, wx)), &m) in row.iter_mut().zip(taps.iter()).zip(&mask[y * d..][..d]) {
            if m == 255 {
                continue;
            }
            let (wx0, wx1) = (F4::splat(wx), F4::splat(1.0 - wx));
            let top = F4::from_px(r0[x0]).mul(wx0).mul_add(F4::from_px(r0[x1]), wx1);
            let bot = F4::from_px(r1[x0]).mul(wx0).mul_add(F4::from_px(r1[x1]), wx1);
            let k = m as f32 * (1.0 / 255.0);
            *p = F4::from_px(*p).mul(F4::splat(k)).mul_add(top.mul(wy0).mul_add(bot, wy1), F4::splat(1.0 - k)).to_px();
        }
    }
}

/// The background's color from its blurred weighted colors `b` (the
/// weight, of 255, in the fourth lane): the weight divided out. Where
/// little background is near (a weight under [`SURE`], e.g. a narrow gap
/// by the person) that division magnifies the 8-bit rounding, so it fades
/// into `plain` (the picture's own color there) instead of switching.
#[inline(always)]
fn background(b: F4, plain: F4) -> F4 {
    let a = b.lane3();
    if a < 1.0 {
        return plain;
    }
    let t = ((a - 1.0) * (1.0 / (SURE - 1.0))).min(1.0);
    b.mul(F4::splat(255.0 / a)).mul(F4::splat(t)).mul_add(plain, F4::splat(1.0 - t))
}

/// Bubbles this large blur their background at half size (it is a blur:
/// nothing shows the difference, and it costs a quarter).
const HALF_FROM: usize = 256;

/// The picture's pixel `p` over the blurred background `b` (weighted
/// colors and, in the fourth lane, their weight; see [`background`]) by
/// the mask `m`.
#[inline(always)]
fn mix(p: &mut [u8; 4], b: F4, m: u8) {
    let pv = F4::from_px(*p);
    let k = m as f32 * (1.0 / 255.0);
    *p = pv.mul(F4::splat(k)).mul_add(background(b, pv), F4::splat(1.0 - k)).to_px();
}

/// The whole of `pic` blurred so heavily that no one and nothing in it
/// can be made out (sigma a sixth of its side, twice: what a blur chosen
/// shows while there is no mask).
pub fn blur_all(pic: &mut CamFrame, s: &mut Scratch) {
    let (w, h) = (pic.w as usize, pic.h as usize);
    if w == 0 || h == 0 || pic.bgra.len() < w * h * 4 {
        return;
    }
    let sigma = (w.min(h) as f32 / 6.0).max(4.0);
    for _ in 0..2 {
        crate::objects::blur::blur_block_on(&mut pic.bgra[..w * h * 4], w, h, sigma, &mut s.tmp, &mut s.acc, 1);
    }
}

/// The blurred background weight (of 255) from which its color is
/// trusted fully.
const SURE: f32 = 16.0;

/// What the camera thread and the segmentation thread share.
#[derive(Default)]
struct Slot {
    /// A picture to segment.
    job: Option<SegInput>,
    /// The last mask and how long it took (ms).
    done: Option<(SqMask, f32)>,
    /// Buffers handed back for reuse.
    spare_in: Option<SegInput>,
    spare_mask: Option<SqMask>,
    /// A job is out (from submit until its mask is taken).
    busy: bool,
    stop: bool,
    /// The picture size the segmenter wants, once it is made; `failed`:
    /// there is none.
    size: Option<(usize, usize)>,
    failed: bool,
}

struct Shared {
    slot: Mutex<Slot>,
    cv: Condvar,
}

fn lock(m: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The segmentation thread (stopped and joined on drop). It makes the
/// segmenter itself (loading the model and starting its helpers takes a
/// few ms, which the camera thread does not wait for).
struct Worker {
    sh: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn start(make: Maker) -> Option<Worker> {
        let sh = Arc::new(Shared { slot: Mutex::new(Slot::default()), cv: Condvar::new() });
        let s2 = sh.clone();
        let thread = std::thread::Builder::new()
            .name("camera-segment".into())
            .spawn(move || {
                let sh = s2;
                let mut make = make;
                let Some(mut eng) = make() else {
                    lock(&sh.slot).failed = true;
                    return;
                };
                lock(&sh.slot).size = Some(eng.input_size());
                loop {
                    let job = {
                        let mut st = lock(&sh.slot);
                        loop {
                            if st.stop {
                                return;
                            }
                            if let Some(j) = st.job.take() {
                                break j;
                            }
                            st = sh.cv.wait(st).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    let mut mask = lock(&sh.slot).spare_mask.take().unwrap_or_default();
                    let t = Instant::now();
                    let ok = eng.segment(&job, &mut mask);
                    let ms = t.elapsed().as_secs_f32() * 1000.0;
                    let mut st = lock(&sh.slot);
                    st.spare_in = Some(job);
                    if ok {
                        st.done = Some((mask, ms));
                    } else {
                        st.busy = false;
                        st.spare_mask = Some(mask);
                    }
                }
            })
            .ok()?;
        Some(Worker { sh, thread: Some(thread) })
    }

    /// The segmenter's picture size (`None` while it is being made).
    fn size(&self) -> Option<(usize, usize)> {
        lock(&self.sh.slot).size
    }

    fn failed(&self) -> bool {
        lock(&self.sh.slot).failed
    }

    /// The finished mask, if one is waiting (the job is over then).
    fn take(&self) -> Option<(SqMask, f32)> {
        let mut st = lock(&self.sh.slot);
        let d = st.done.take()?;
        st.busy = false;
        Some(d)
    }

    fn give_back(&self, m: SqMask) {
        lock(&self.sh.slot).spare_mask = Some(m);
    }

    fn busy(&self) -> bool {
        lock(&self.sh.slot).busy
    }

    fn spare(&self) -> SegInput {
        lock(&self.sh.slot).spare_in.take().unwrap_or_default()
    }

    fn submit(&self, inp: SegInput) {
        let mut st = lock(&self.sh.slot);
        st.job = Some(inp);
        st.busy = true;
        self.sh.cv.notify_all();
    }

    /// Wait (tests) until the segmenter is made (or not) and the job out
    /// is done.
    #[cfg(test)]
    fn wait_idle(&self) {
        let end = Instant::now() + std::time::Duration::from_secs(10);
        let out = |st: &Slot| (st.size.is_none() && !st.failed) || st.job.is_some() || (st.busy && st.done.is_none());
        while out(&lock(&self.sh.slot)) {
            assert!(Instant::now() < end, "the segmentation thread is stuck");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        lock(&self.sh.slot).stop = true;
        self.sh.cv.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Makes the segmenter, once, on the segmentation thread.
pub type Maker = Box<dyn FnMut() -> Option<Box<dyn Segment>> + Send>;

/// The background effect of one camera thread.
pub struct Effect {
    /// Taken by the worker when it starts.
    make: Option<Maker>,
    /// `Some(None)`: tried, no segmenter here.
    worker: Option<Option<Worker>>,
    pub pacer: Pacer,
    smooth: Smooth,
    /// A job sent before the blur was turned off: its mask is dropped.
    stale: bool,
    mask: Vec<u8>,
    scratch: Scratch,
    /// Whether the segmenter can be made yet (the model may still be on
    /// its way: the picture is fully blurred meanwhile).
    ready: Option<fn() -> Engine>,
    /// The model was asked for (once per bubble; a retry is the user's).
    asked: bool,
}

impl Default for Effect {
    fn default() -> Effect {
        Effect { ready: Some(engine_ready), ..Effect::with_maker(Box::new(default_engine)) }
    }
}

impl Effect {
    /// Tests: an effect whose segmenter can be made once `ready` says so.
    #[cfg(test)]
    pub fn with_ready(make: Maker, ready: fn() -> Engine) -> Effect {
        Effect { ready: Some(ready), ..Effect::with_maker(make) }
    }

    pub fn with_maker(make: Maker) -> Effect {
        Effect { make: Some(make), worker: None, pacer: Pacer::default(), smooth: Smooth::default(), stale: false, mask: Vec::new(), scratch: Scratch::default(), ready: None, asked: false }
    }

    /// `pic` (the bubble's square of `raw`) with its background blurred
    /// as `bg` says. Off: left alone. A blur with no mask to blur with
    /// (the model on its way or not to be had, the segmenter not made or
    /// failed, the first mask not back yet): the whole picture blurred,
    /// with its badge ([`CamFrame::note`]); never the camera as it is.
    pub fn apply(&mut self, bg: Bg, raw: &CamFrame, pic: &mut CamFrame) {
        pic.note = None;
        if bg == Bg::Off {
            // The thread stays (turning it on again is instant); the old
            // mask, and one still being made, would show the wrong place.
            self.smooth.clear();
            if let Some(Some(w)) = &self.worker {
                if let Some((m, _)) = w.take() {
                    w.give_back(m);
                }
                self.stale |= w.busy();
            }
            return;
        }
        if self.worker.is_none() {
            let now = self.ready.map_or(Engine::Ready, |r| r());
            if now != Engine::Ready {
                if !std::mem::replace(&mut self.asked, true) {
                    request_model();
                }
                return self.fail_safe(pic, if now == Engine::Loading { LOADING_BADGE } else { UNAVAILABLE_BADGE });
            }
            self.worker = Some(self.make.take().and_then(Worker::start));
        }
        if matches!(&self.worker, Some(Some(w)) if w.failed()) {
            self.worker = Some(None);
        }
        let Some(Some(w)) = &self.worker else { return self.fail_safe(pic, UNAVAILABLE_BADGE) };
        if let Some((m, ms)) = w.take() {
            self.pacer.took(ms);
            if !std::mem::take(&mut self.stale) {
                self.smooth.push(&m);
            }
            w.give_back(m);
        }
        if let Some((sw, sh)) = w.size()
            && self.pacer.want(w.busy())
        {
            let mut inp = w.spare();
            inp.prepare(raw, sw, sh);
            if inp.sq.is_some() {
                w.submit(inp);
            }
        }
        if !self.smooth.has() {
            // The segmenter starting, the first mask on its way: blurred
            // whole meanwhile (no badge: a moment).
            blur_all(pic, &mut self.scratch);
            return;
        }
        refine(&self.smooth.m, pic, &mut self.mask, &mut self.scratch);
        composite(pic, &self.mask, bg.sigma(), &mut self.scratch);
    }

    /// No mask to be had: `pic` blurred whole, saying `badge`.
    fn fail_safe(&mut self, pic: &mut CamFrame, badge: &'static str) {
        blur_all(pic, &mut self.scratch);
        pic.note = Some(badge);
    }

    /// The smoothed mask (tests).
    #[cfg(test)]
    pub fn mask(&self) -> &SqMask {
        &self.smooth.m
    }

    /// Wait for the segmenter and the segmentation out (tests).
    #[cfg(test)]
    pub fn settle(&self) {
        if let Some(Some(w)) = &self.worker {
            w.wait_idle();
        }
    }

    /// Whether a segmenter runs (tests; after `settle`).
    #[cfg(test)]
    pub fn running(&self) -> Option<bool> {
        self.worker.as_ref().map(|w| w.as_ref().is_some_and(|w| !w.failed()))
    }
}

#[cfg(test)]
mod tests;
