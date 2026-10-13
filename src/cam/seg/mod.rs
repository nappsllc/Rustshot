//! Person segmentation for the camera bubble's background blur: a small
//! hand-written inference engine for MediaPipe's Selfie Segmentation
//! model (the landscape 144x256 variant), converted to [`format`]'s
//! compact `selfie.bin` by `tools/convert-selfie`. Only the operators the
//! model uses are here ([`ops`]): convolutions (plain, depthwise,
//! transposed), add / multiply, average pooling and the mean (squeeze and
//! excite), bilinear resize, concatenation and the activations.
//!
//! [`Net::load`] checks every shape once (an operator whose output does
//! not match what its inputs give is refused), repacks the convolution
//! weights for the vector loops and allocates every activation; then
//! [`Net::run`] allocates nothing. The camera thread prepares a small
//! RGBA picture ([`SegInput::prepare`]: the frame scaled so the bubble's
//! centre square fills the model's height); the mask of that square comes
//! back as a [`SqMask`].

pub mod format;
pub(crate) mod ops;
mod team;
#[cfg(test)]
mod tests;

use super::CamFrame;
use format::{Act, FormatError, Kind, Model};
use ops::{dims4, Dim, Geo};
use std::sync::Mutex;
use team::{Rows, Team};

/// The model in this build (`selfie_model` feature: converted with
/// `tools/convert-selfie` into `assets/selfie.bin`).
#[cfg(feature = "selfie_model")]
pub static EMBEDDED: Option<&[u8]> = Some(include_bytes!("../../../assets/selfie.bin"));
#[cfg(not(feature = "selfie_model"))]
pub static EMBEDDED: Option<&[u8]> = None;

/// What a convolution keeps ready.
enum Prep {
    None,
    /// HWIO weights (outputs padded to 4) and the padded bias.
    Packed(Vec<f32>, Vec<f32>),
    /// [`ops::pack_tconv`] weights, the padded bias, a zero row.
    TConv { w: Vec<f32>, b: Vec<f32>, zero: Vec<f32> },
}

struct Step {
    kind: Kind,
    act: Act,
    inputs: Vec<usize>,
    output: usize,
    prep: Prep,
    /// A hard-swish that followed this convolution, done on its rows.
    hswish: bool,
}

/// A loaded model, ready to run.
pub struct Net {
    shapes: Vec<[usize; 4]>,
    /// Constant tensors (weights) and activation buffers, by tensor.
    consts: Vec<Option<Vec<f32>>>,
    bufs: Vec<Vec<f32>>,
    steps: Vec<Step>,
    input: usize,
    output: usize,
    /// Patches and products of the convolutions (kept between runs).
    /// Per part of a run: patches and products of the convolutions.
    scratch: Vec<Mutex<Vec<f32>>>,
    team: Team,
}

fn bad(s: impl Into<String>) -> FormatError {
    FormatError(s.into())
}

/// Threads besides the net's own for the big operators: a quarter of the
/// machine's, at most 3 (the camera, the recording and the UI need the
/// rest).
fn default_helpers() -> usize {
    let cpus = std::thread::available_parallelism().map_or(1, |p| p.get());
    (cpus / 4).clamp(1, 4) - 1
}

/// Rows `[n·p/parts, n·(p+1)/parts)`.
fn split(n: usize, parts: usize, p: usize) -> (usize, usize) {
    (n * p / parts, n * (p + 1) / parts)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Below this many multiply-adds an operator runs on one thread (waking
/// the team costs more).
const PAR_WORK: usize = 250_000;

/// One part's work: its output rows, those rows of the output, scratch.
type RowFn<'a> = dyn Fn((usize, usize), &mut [f32], &mut Vec<f32>) + Sync + 'a;

/// The output size of a convolution or pool along one axis, if `out` is
/// what SAME or VALID padding gives.
fn conv_len_ok(n: usize, k: usize, stride: usize, dil: usize, out: usize) -> bool {
    let eff = (k - 1) * dil + 1;
    let same = n.div_ceil(stride);
    let valid = if n >= eff { (n - eff) / stride + 1 } else { 0 };
    out > 0 && (out == same || out == valid)
}

impl Net {
    /// Load `selfie.bin` bytes; the shapes are checked, nothing is run.
    pub fn load(bytes: &[u8]) -> Result<Net, FormatError> {
        Net::from_model(Model::parse(bytes)?)
    }

    /// [`Net::load`] for a segmentation model: one output channel (the
    /// person's probability) or two (background and person scores).
    pub fn selfie(bytes: &[u8]) -> Result<Net, FormatError> {
        let net = Net::load(bytes)?;
        let so = net.shapes[net.output];
        if !(1..=2).contains(&so[3]) {
            return Err(bad(format!("the output must be 1xHxWx1 or 2 (it is {so:?})")));
        }
        Ok(net)
    }

    pub fn from_model(m: Model) -> Result<Net, FormatError> {
        Net::with_helpers(m, default_helpers())
    }

    /// [`Net::from_model`] with `helpers` threads for the big operators.
    pub fn with_helpers(m: Model, helpers: usize) -> Result<Net, FormatError> {
        let shapes: Vec<[usize; 4]> = m.tensors.iter().map(|t| dims4(&t.dims)).collect();
        let consts: Vec<Option<Vec<f32>>> = m.tensors.into_iter().map(|t| t.data).collect();
        let (input, output) = (m.input as usize, m.output as usize);
        let si = shapes[input];
        if consts[input].is_some() || si[0] != 1 || si[3] != 3 {
            return Err(bad(format!("the input must be 1xHxWx3 (it is {si:?})")));
        }
        let mut ready = vec![false; shapes.len()];
        ready[input] = true;
        for (t, c) in consts.iter().enumerate() {
            if let Some(c) = c {
                if c.len() != shapes[t].iter().product::<usize>() {
                    return Err(bad(format!("tensor {t}: data does not match its shape")));
                }
                ready[t] = true;
            }
        }
        let mut steps = Vec::with_capacity(m.ops.len());
        for (n, op) in m.ops.iter().enumerate() {
            let e = |s: &str| bad(format!("op {n} ({:?}): {s}", op.kind));
            let ins: Vec<usize> = op.inputs.iter().map(|&i| i as usize).collect();
            let out = op.output as usize;
            if ins.iter().any(|&i| !ready[i]) {
                return Err(e("an input is not computed before it"));
            }
            if ready[out] {
                return Err(e("its output is written twice or is a constant"));
            }
            let sh = |i: usize| shapes[ins[i]];
            let cst = |i: usize| consts[ins[i]].as_deref();
            let need = |k: usize| if ins.len() < k { Err(e("too few inputs")) } else { Ok(()) };
            let od = shapes[out];
            if od[0] != 1 {
                return Err(e("batch must be 1"));
            }
            let mut prep = Prep::None;
            match op.kind {
                Kind::Conv { stride, dil, .. } | Kind::Depthwise { stride, dil, .. } => {
                    need(2)?;
                    let (x, w) = (sh(0), sh(1));
                    let wd = cst(1).ok_or_else(|| e("weights must be constant"))?;
                    let bias = if ins.len() > 2 { Some(cst(2).ok_or_else(|| e("bias must be constant"))?) } else { None };
                    let dw = matches!(op.kind, Kind::Depthwise { .. });
                    let (kh, kw) = (w[1], w[2]);
                    if stride.contains(&0) || dil.contains(&0) || kh == 0 || kw == 0 {
                        return Err(e("zero stride, dilation or kernel"));
                    }
                    if !conv_len_ok(x[1], kh, stride[0] as usize, dil[0] as usize, od[1]) || !conv_len_ok(x[2], kw, stride[1] as usize, dil[1] as usize, od[2]) {
                        return Err(e("output size"));
                    }
                    let oc = od[3];
                    if bias.is_some_and(|b| b.len() != oc) {
                        return Err(e("bias size"));
                    }
                    if dw {
                        let Kind::Depthwise { mult, .. } = op.kind else { unreachable!() };
                        if w[0] != 1 || w[3] != oc || oc != x[3] * mult as usize || mult == 0 {
                            return Err(e("depthwise weights / channels"));
                        }
                        if mult == 1 {
                            let (pw, pb) = ops::pack_dw(wd, (kh, kw, oc), bias);
                            prep = Prep::Packed(pw, pb);
                        }
                    } else {
                        if w[0] != oc || w[3] != x[3] {
                            return Err(e("weights / channels"));
                        }
                        let (pw, pb) = ops::pack_hwio(wd, (oc, kh, kw, x[3]), bias);
                        prep = Prep::Packed(pw, pb);
                    }
                }
                Kind::TransposeConv { stride, .. } => {
                    need(2)?;
                    let (x, w) = (sh(0), sh(1));
                    cst(1).ok_or_else(|| e("weights must be constant"))?;
                    if ins.len() > 2 && cst(2).is_none_or(|b| b.len() != od[3]) {
                        return Err(e("bias"));
                    }
                    if stride.contains(&0) || w[0] != od[3] || w[3] != x[3] || od[1] == 0 || od[2] == 0 {
                        return Err(e("weights / channels"));
                    }
                    let oc = od[3];
                    let wp = ops::pack_tconv(cst(1).unwrap_or_default(), (oc, w[1], w[2], x[3]));
                    let b = ops::pad_bias(if ins.len() > 2 { cst(2) } else { None }, oc);
                    let zero = vec![0.0; w[1] * w[2] * ops::round4(oc)];
                    prep = Prep::TConv { w: wp, b, zero };
                }
                Kind::Add | Kind::Mul => {
                    need(2)?;
                    let (a, b) = (sh(0), sh(1));
                    for k in 0..4 {
                        let ok = (a[k] == b[k] || a[k] == 1 || b[k] == 1) && od[k] == a[k].max(b[k]);
                        if !ok {
                            return Err(e("shapes do not broadcast to the output"));
                        }
                    }
                }
                Kind::AvgPool { stride, size, .. } => {
                    need(1)?;
                    let x = sh(0);
                    if stride.contains(&0) || size.contains(&0) || x[3] != od[3] {
                        return Err(e("pool"));
                    }
                    if !conv_len_ok(x[1], size[0] as usize, stride[0] as usize, 1, od[1]) || !conv_len_ok(x[2], size[1] as usize, stride[1] as usize, 1, od[2]) {
                        return Err(e("output size"));
                    }
                }
                Kind::Mean => {
                    need(1)?;
                    if od[1] != 1 || od[2] != 1 || od[3] != sh(0)[3] {
                        return Err(e("mean must keep 1x1xC"));
                    }
                }
                Kind::Resize { .. } => {
                    need(1)?;
                    if od[3] != sh(0)[3] || od[1] == 0 || od[2] == 0 {
                        return Err(e("channels"));
                    }
                }
                Kind::Sigmoid | Kind::Relu | Kind::Relu6 | Kind::HardSwish => {
                    need(1)?;
                    if sh(0) != od {
                        return Err(e("shape"));
                    }
                }
                Kind::Concat => {
                    need(1)?;
                    let c: usize = (0..ins.len()).map(|i| sh(i)[3]).sum();
                    if (0..ins.len()).any(|i| sh(i)[..3] != od[..3]) || c != od[3] {
                        return Err(e("shapes"));
                    }
                }
                Kind::Reshape => {
                    need(1)?;
                    if sh(0).iter().product::<usize>() != od.iter().product::<usize>() {
                        return Err(e("sizes"));
                    }
                }
            }
            ready[out] = true;
            // A reshape's shape input (a constant) is not read.
            let inputs = if op.kind == Kind::Reshape { ins[..1].to_vec() } else { ins };
            steps.push(Step { kind: op.kind, act: op.act, inputs, output: out, prep, hswish: false });
        }
        if !ready[output] || output == input {
            return Err(bad("nothing computes the output"));
        }
        // A hard-swish right after a convolution whose output it alone
        // reads is done on the convolution's rows (one pass less).
        let mut uses = vec![0usize; shapes.len()];
        for s in &steps {
            for &i in &s.inputs {
                uses[i] += 1;
            }
        }
        let mut fused: Vec<Step> = Vec::with_capacity(steps.len());
        for s in steps {
            if let Some(prev) = fused.last_mut()
                && s.kind == Kind::HardSwish
                && s.act == Act::None
                && prev.output == s.inputs[0]
                && uses[prev.output] == 1
                && prev.output != output
                && prev.act == Act::None
                && !prev.hswish
                && matches!((prev.kind, &prev.prep), (Kind::Conv { .. } | Kind::Depthwise { .. }, Prep::Packed(..)))
            {
                prev.output = s.output;
                prev.hswish = true;
                continue;
            }
            fused.push(s);
        }
        let steps = fused;
        let mut written = vec![false; shapes.len()];
        written[input] = true;
        for s in &steps {
            written[s.output] = true;
        }
        let bufs = shapes.iter().zip(&written).map(|(s, &w)| if w { vec![0.0; s.iter().product()] } else { Vec::new() }).collect();
        let team = Team::new(helpers);
        let scratch = (0..team.size()).map(|_| Mutex::new(Vec::new())).collect();
        Ok(Net { shapes, consts, bufs, steps, input, output, scratch, team })
    }

    /// The input's (height, width).
    pub fn input_hw(&self) -> (usize, usize) {
        let s = self.shapes[self.input];
        (s[1], s[2])
    }

    /// The output's (height, width, channels).
    pub fn output_dim(&self) -> Dim {
        let s = self.shapes[self.output];
        (s[1], s[2], s[3])
    }

    /// The input buffer (HWC, RGB, 0..1) to fill before [`Net::run`].
    pub fn input_mut(&mut self) -> &mut [f32] {
        &mut self.bufs[self.input]
    }

    /// Run the graph on the input; the output (HWC).
    pub fn run(&mut self) -> &[f32] {
        for i in 0..self.steps.len() {
            let o = self.steps[i].output;
            let mut out = std::mem::take(&mut self.bufs[o]);
            self.step(i, &mut out);
            self.bufs[o] = out;
        }
        &self.bufs[self.output]
    }

    /// The threads working on the big operators (this one included).
    #[cfg(test)]
    pub fn threads(&self) -> usize {
        self.team.size()
    }

    /// How many parts an operator of `work` multiply-adds over `rows`
    /// output rows is split into.
    fn parts(&self, work: usize, rows: usize) -> usize {
        if work < PAR_WORK { 1 } else { self.team.size().min(rows).max(1) }
    }

    /// `f(rows, out)` for each part's rows of `out` (`row_len` values a
    /// row), on the team.
    fn by_rows(&self, out: &mut [f32], nrows: usize, row_len: usize, parts: usize, f: &RowFn) {
        assert!(out.len() >= nrows * row_len);
        let rows = Rows(out.as_mut_ptr());
        self.team.run(parts, &|p| {
            let (r0, r1) = split(nrows, parts, p);
            // SAFETY: rows r0..r1 of `out` (asserted to hold them) are this
            // part's alone: the parts' ranges are disjoint.
            let o = unsafe { std::slice::from_raw_parts_mut(rows.get().add(r0 * row_len), (r1 - r0) * row_len) };
            f((r0, r1), o, &mut lock(&self.scratch[p]));
        });
    }

    /// [`Net::run`], timing each operator kind (benchmarks): (kind, ms, count).
    #[cfg(test)]
    pub fn run_timed(&mut self) -> Vec<(String, f64, usize)> {
        let mut by: Vec<(String, f64, usize)> = Vec::new();
        for i in 0..self.steps.len() {
            let o = self.steps[i].output;
            let mut out = std::mem::take(&mut self.bufs[o]);
            let t = std::time::Instant::now();
            self.step(i, &mut out);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            self.bufs[o] = out;
            let st = &self.steps[i];
            let k = match st.kind {
                Kind::Conv { .. } if self.shapes[st.inputs[1]][1] == 1 => "conv1x1".to_string(),
                k => format!("{k:?}").split([' ', '{']).next().unwrap_or("").to_string(),
            };
            match by.iter_mut().find(|e| e.0 == k) {
                Some(e) => {
                    e.1 += ms;
                    e.2 += 1;
                }
                None => by.push((k, ms, 1)),
            }
        }
        by
    }

    fn step(&self, i: usize, out: &mut [f32]) {
        let st = &self.steps[i];
        let get = |k: usize| -> &[f32] {
            let t = st.inputs[k];
            self.consts[t].as_deref().unwrap_or(&self.bufs[t])
        };
        let d3 = |t: usize| -> Dim {
            let s = self.shapes[t];
            (s[1], s[2], s[3])
        };
        let xd = d3(st.inputs[0]);
        let od = d3(st.output);
        let p2 = |a: [u32; 2]| (a[0] as usize, a[1] as usize);
        match (st.kind, &st.prep) {
            (Kind::Conv { stride, pad, dil }, Prep::Packed(w, b)) => {
                let ws = self.shapes[st.inputs[1]];
                let g = Geo { k: (ws[1], ws[2]), stride: p2(stride), pad: p2(pad), dil: p2(dil) };
                let x = get(0);
                let parts = self.parts(od.0 * od.1 * od.2 * ws[1] * ws[2] * xd.2, od.0);
                self.by_rows(out, od.0, od.1 * od.2, parts, &|rows, o, s| {
                    ops::conv(x, xd, w, b, g, st.act, o, od, rows, s);
                    if st.hswish {
                        ops::hard_swish(o);
                    }
                });
            }
            (Kind::Depthwise { stride, pad, dil, .. }, Prep::Packed(w, b)) => {
                let ws = self.shapes[st.inputs[1]];
                let g = Geo { k: (ws[1], ws[2]), stride: p2(stride), pad: p2(pad), dil: p2(dil) };
                let x = get(0);
                let parts = self.parts(od.0 * od.1 * od.2 * ws[1] * ws[2], od.0);
                self.by_rows(out, od.0, od.1 * od.2, parts, &|rows, o, _| {
                    ops::depthwise(x, xd, w, b, g, st.act, o, od, rows);
                    if st.hswish {
                        ops::hard_swish(o);
                    }
                });
            }
            (Kind::Depthwise { stride, pad, dil, mult }, Prep::None) => {
                let ws = self.shapes[st.inputs[1]];
                let g = Geo { k: (ws[1], ws[2]), stride: p2(stride), pad: p2(pad), dil: p2(dil) };
                let b = (st.inputs.len() > 2).then(|| get(2));
                ops::depthwise_mult(get(0), xd, get(1), b, g, mult as usize, st.act, out, od);
            }
            (Kind::TransposeConv { stride, pad }, Prep::TConv { w, b, zero }) => {
                let ws = self.shapes[st.inputs[1]];
                let (k, x) = ((ws[1], ws[2]), get(0));
                // Input rows reach disjoint output rows unless the kernel
                // is taller than the stride.
                let work = xd.0 * xd.1 * xd.2 * k.0 * k.1 * ops::round4(od.2);
                let parts = if k.0 <= stride[0] as usize { self.parts(work, xd.0) } else { 1 };
                ops::tconv_bias(out, od, b);
                let at = Rows(out.as_mut_ptr());
                self.team.run(parts, &|p| {
                    // SAFETY: `out` holds the whole output; part p's input
                    // rows reach only their own output rows (k <= stride,
                    // or one part).
                    unsafe { ops::transpose_conv_rows(x, xd, w, k, zero, p2(stride), p2(pad), at.get(), od, split(xd.0, parts, p), &mut lock(&self.scratch[p])) };
                });
                ops::activate(st.act, &mut out[..od.0 * od.1 * od.2]);
            }
            (Kind::Add | Kind::Mul, _) => {
                let (a, b) = (self.shapes[st.inputs[0]], self.shapes[st.inputs[1]]);
                ops::binary(get(0), a, get(1), b, self.shapes[st.output], st.kind == Kind::Mul, st.act, out);
            }
            (Kind::AvgPool { stride, pad, size }, _) => ops::avg_pool(get(0), xd, p2(size), p2(stride), p2(pad), st.act, out, od),
            (Kind::Mean, _) => {
                ops::mean(get(0), xd, out);
                ops::activate(st.act, out);
            }
            (Kind::Resize { align, half }, _) => {
                ops::resize_bilinear(get(0), xd, (od.0, od.1), align, half, out);
                ops::activate(st.act, out);
            }
            (Kind::Sigmoid | Kind::Relu | Kind::Relu6 | Kind::HardSwish | Kind::Reshape, _) => {
                let x = get(0);
                let rl = od.1 * od.2;
                // (An exponential costs some twenty multiply-adds.)
                let parts = self.parts(out.len() * if st.kind == Kind::Sigmoid { 20 } else { 2 }, od.0);
                self.by_rows(out, od.0, rl, parts, &|(r0, r1), o, _| {
                    o.copy_from_slice(&x[r0 * rl..r1 * rl]);
                    match st.kind {
                        Kind::Sigmoid => ops::sigmoid(o),
                        Kind::Relu => ops::activate(Act::Relu, o),
                        Kind::Relu6 => ops::activate(Act::Relu6, o),
                        Kind::HardSwish => ops::hard_swish(o),
                        _ => {}
                    }
                    ops::activate(st.act, o);
                });
            }
            (Kind::Concat, _) => {
                let parts: Vec<(&[f32], usize)> = (0..st.inputs.len()).map(|k| (get(k), self.shapes[st.inputs[k]][3])).collect();
                ops::concat(&parts, od.0 * od.1, out);
                ops::activate(st.act, out);
            }
            (Kind::Conv { .. } | Kind::TransposeConv { .. }, _) | (Kind::Depthwise { .. }, Prep::TConv { .. }) => {
                unreachable!("convolutions are packed at load")
            }
        }
    }
}

/// The square of the camera picture the bubble shows, as the model sees
/// it: in [`SegInput`]'s pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Square {
    pub x: f32,
    pub y: f32,
    pub side: f32,
}

/// A small RGBA picture for the segmenter (RGBA so macOS's Vision takes
/// it as it is) and where the bubble's square is in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SegInput {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub sq: Option<Square>,
}

impl SegInput {
    /// `f` scaled so its centre square (the bubble's picture) fills the
    /// height of a `w`x`h` picture, centred: a 16:9 frame fills it, a
    /// narrower one leaves black at the sides, a wider one is cut. Each
    /// pixel is the mean of four samples (enough against aliasing at
    /// 1/5 scale, and cheap on the camera thread).
    pub fn prepare(&mut self, f: &CamFrame, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.rgba.clear();
        self.rgba.resize(w * h * 4, 0);
        let (fw, fh) = (f.w as usize, f.h as usize);
        if fw == 0 || fh == 0 || f.bgra.len() < fw * fh * 4 || w == 0 || h == 0 {
            self.sq = None;
            return;
        }
        let s = fw.min(fh) as f32;
        let side = (h.min(w)) as f32;
        let k = s / side; // frame px per input px
        let (cx, cy) = (fw as f32 / 2.0, fh as f32 / 2.0);
        let (ox, oy) = (w as f32 / 2.0, h as f32 / 2.0);
        let q = k * 0.25;
        // The two sample columns (byte offsets) of each input column; a
        // column outside the frame stays black (and so do rows).
        let inside = |v: f32, n: usize| v >= 0.0 && v < n as f32;
        let xs: Vec<Option<[usize; 2]>> = (0..w)
            .map(|u| {
                let fx = (u as f32 + 0.5 - ox) * k + cx;
                (inside(fx - q, fw) && inside(fx + q, fw)).then(|| [(fx - q) as usize * 4, (fx + q) as usize * 4])
            })
            .collect();
        let row_b = fw * 4;
        for v in 0..h {
            let fy = (v as f32 + 0.5 - oy) * k + cy;
            let row = &mut self.rgba[v * w * 4..][..w * 4];
            for o in row.as_chunks_mut::<4>().0 {
                o[3] = 255;
            }
            if !(inside(fy - q, fh) && inside(fy + q, fh)) {
                continue;
            }
            let (r0, r1) = (&f.bgra[(fy - q) as usize * row_b..][..row_b], &f.bgra[(fy + q) as usize * row_b..][..row_b]);
            for (o, xx) in row.as_chunks_mut::<4>().0.iter_mut().zip(&xs) {
                let Some([a, b]) = *xx else { continue };
                let (p, q, s, t) = (&r0[a..a + 3], &r0[b..b + 3], &r1[a..a + 3], &r1[b..b + 3]);
                for c in 0..3 {
                    o[2 - c] = ((p[c] as u32 + q[c] as u32 + s[c] as u32 + t[c] as u32 + 2) >> 2) as u8;
                }
            }
        }
        self.sq = Some(Square { x: ox - side / 2.0, y: oy - side / 2.0, side });
    }
}

/// The person mask of the bubble's square: `n`x`n`, 0 (background) to 1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SqMask {
    pub n: usize,
    pub v: Vec<f32>,
}

/// The square `sq` (in a `iw`x`ih` input) of a `mw`x`mh` mask `m`,
/// resampled bilinearly to its own resolution in the mask.
pub fn crop_square(m: &[f32], (mw, mh): (usize, usize), (iw, ih): (usize, usize), sq: Square, out: &mut SqMask) {
    let (kx, ky) = (mw as f32 / iw as f32, mh as f32 / ih as f32);
    let n = ((sq.side * kx.max(ky)).round() as usize).max(1);
    out.n = n;
    out.v.clear();
    out.v.reserve(n * n);
    let at = |x: usize, y: usize| m[y.min(mh - 1) * mw + x.min(mw - 1)];
    for j in 0..n {
        let fy = ((sq.y + (j as f32 + 0.5) * sq.side / n as f32) * ky - 0.5).clamp(0.0, (mh - 1) as f32);
        let (y0, wy) = (fy.floor() as usize, fy - fy.floor());
        for i in 0..n {
            let fx = ((sq.x + (i as f32 + 0.5) * sq.side / n as f32) * kx - 0.5).clamp(0.0, (mw - 1) as f32);
            let (x0, wx) = (fx.floor() as usize, fx - fx.floor());
            let top = at(x0, y0) * (1.0 - wx) + at(x0 + 1, y0) * wx;
            let bot = at(x0, y0 + 1) * (1.0 - wx) + at(x0 + 1, y0 + 1) * wx;
            out.v.push(top * (1.0 - wy) + bot * wy);
        }
    }
}

/// Something that finds the person in a [`SegInput`].
pub trait Segment: Send {
    /// The mask of the input's square into `out`; false when it could not.
    fn segment(&mut self, input: &SegInput, out: &mut SqMask) -> bool;
    /// The picture size it wants.
    fn input_size(&self) -> (usize, usize);
}

impl Segment for Net {
    fn input_size(&self) -> (usize, usize) {
        let (h, w) = self.input_hw();
        (w, h)
    }

    fn segment(&mut self, input: &SegInput, out: &mut SqMask) -> bool {
        let (h, w) = self.input_hw();
        let Some(sq) = input.sq.filter(|_| (input.w, input.h) == (w, h) && input.rgba.len() >= w * h * 4) else { return false };
        for (d, s) in self.input_mut().as_chunks_mut::<3>().0.iter_mut().zip(input.rgba.as_chunks::<4>().0) {
            for c in 0..3 {
                d[c] = s[c] as f32 * (1.0 / 255.0);
            }
        }
        let (oh, ow, oc) = self.output_dim();
        let y = self.run();
        // Two channels: background and person scores (a softmax's second).
        let m: std::borrow::Cow<[f32]> = if oc == 1 {
            std::borrow::Cow::Borrowed(y)
        } else {
            y.as_chunks::<2>().0.iter().map(|p| 1.0 / (1.0 + (p[0] - p[1]).exp())).collect::<Vec<_>>().into()
        };
        crop_square(&m, (ow, oh), (w, h), sq, out);
        true
    }
}
