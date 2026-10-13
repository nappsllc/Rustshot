//! The engine against a plain scalar evaluation of the same graph (f64,
//! straight from TensorFlow Lite's definitions, nothing shared with
//! `ops`), on synthetic models: random weights (exact in f16) of every
//! operator alone, then a small MobileNet-like encoder-decoder.

use super::format::{f16_to_f32, f32_to_f16, Act, Kind, Model, Op, Tensor};
use super::*;

/// Builds synthetic models; weights are random but exact in f16, so the
/// model survives `selfie.bin` unchanged.
pub(crate) struct B {
    pub m: Model,
    seed: u32,
}

impl B {
    pub fn new(seed: u32) -> B {
        B { m: Model { tensors: Vec::new(), ops: Vec::new(), input: 0, output: 0 }, seed: seed | 1 }
    }

    pub fn rnd(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    }

    pub fn dims(&self, t: u16) -> Vec<u32> {
        self.m.tensors[t as usize].dims.clone()
    }

    fn push(&mut self, dims: Vec<u32>, data: Option<Vec<f32>>) -> u16 {
        self.m.tensors.push(Tensor { dims, data });
        (self.m.tensors.len() - 1) as u16
    }

    pub fn input(&mut self, h: u32, w: u32) -> u16 {
        let t = self.push(vec![1, h, w, 3], None);
        self.m.input = t;
        t
    }

    pub fn act_t(&mut self, dims: Vec<u32>) -> u16 {
        self.push(dims, None)
    }

    pub fn konst(&mut self, dims: Vec<u32>, scale: f32) -> u16 {
        let n: u32 = dims.iter().product();
        let data = (0..n).map(|_| f16_to_f32(f32_to_f16(self.rnd() * scale))).collect();
        self.push(dims, Some(data))
    }

    fn op(&mut self, kind: Kind, act: Act, inputs: Vec<u16>, out_dims: Vec<u32>) -> u16 {
        let o = self.act_t(out_dims);
        self.m.ops.push(Op { kind, act, inputs, output: o });
        o
    }

    /// TensorFlow Lite's SAME (or VALID) output and top/left padding.
    fn pads(n: u32, k: u32, s: u32, d: u32, same: bool) -> (u32, u32) {
        let eff = (k - 1) * d + 1;
        if same {
            let out = n.div_ceil(s);
            let total = ((out - 1) * s + eff).saturating_sub(n);
            (out, total / 2)
        } else {
            ((n - eff) / s + 1, 0)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn conv_full(&mut self, x: u16, oc: u32, k: (u32, u32), s: (u32, u32), d: (u32, u32), same: bool, bias: bool, act: Act) -> u16 {
        let xd = self.dims(x);
        let (oh, pt) = Self::pads(xd[1], k.0, s.0, d.0, same);
        let (ow, pl) = Self::pads(xd[2], k.1, s.1, d.1, same);
        let fan = (k.0 * k.1 * xd[3]) as f32;
        let w = self.konst(vec![oc, k.0, k.1, xd[3]], 1.5 / fan.sqrt());
        let mut ins = vec![x, w];
        if bias {
            ins.push(self.konst(vec![oc], 0.2));
        }
        self.op(Kind::Conv { stride: [s.0, s.1], pad: [pt, pl], dil: [d.0, d.1] }, act, ins, vec![1, oh, ow, oc])
    }

    pub fn conv(&mut self, x: u16, oc: u32, k: u32, s: u32, act: Act) -> u16 {
        self.conv_full(x, oc, (k, k), (s, s), (1, 1), true, true, act)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dw_full(&mut self, x: u16, mult: u32, k: (u32, u32), s: (u32, u32), d: (u32, u32), same: bool, bias: bool, act: Act) -> u16 {
        let xd = self.dims(x);
        let oc = xd[3] * mult;
        let (oh, pt) = Self::pads(xd[1], k.0, s.0, d.0, same);
        let (ow, pl) = Self::pads(xd[2], k.1, s.1, d.1, same);
        let w = self.konst(vec![1, k.0, k.1, oc], 1.5 / ((k.0 * k.1) as f32).sqrt());
        let mut ins = vec![x, w];
        if bias {
            ins.push(self.konst(vec![oc], 0.2));
        }
        self.op(Kind::Depthwise { stride: [s.0, s.1], pad: [pt, pl], dil: [d.0, d.1], mult }, act, ins, vec![1, oh, ow, oc])
    }

    pub fn dw(&mut self, x: u16, k: u32, s: u32, act: Act) -> u16 {
        self.dw_full(x, 1, (k, k), (s, s), (1, 1), true, true, act)
    }

    pub fn bin(&mut self, a: u16, b: u16, mul: bool, act: Act) -> u16 {
        let (ad, bd) = (dims4(&self.dims(a)), dims4(&self.dims(b)));
        let od: Vec<u32> = (0..4).map(|k| ad[k].max(bd[k]) as u32).collect();
        self.op(if mul { Kind::Mul } else { Kind::Add }, act, vec![a, b], od)
    }

    pub fn unary(&mut self, x: u16, kind: Kind) -> u16 {
        let d = self.dims(x);
        self.op(kind, Act::None, vec![x], d)
    }

    pub fn mean(&mut self, x: u16) -> u16 {
        let d = self.dims(x);
        self.op(Kind::Mean, Act::None, vec![x], vec![1, 1, 1, d[3]])
    }

    pub fn pool(&mut self, x: u16, size: u32, s: u32, same: bool) -> u16 {
        let d = self.dims(x);
        let (oh, pt) = Self::pads(d[1], size, s, 1, same);
        let (ow, pl) = Self::pads(d[2], size, s, 1, same);
        self.op(Kind::AvgPool { stride: [s, s], pad: [pt, pl], size: [size, size] }, Act::None, vec![x], vec![1, oh, ow, d[3]])
    }

    pub fn resize(&mut self, x: u16, h: u32, w: u32, align: bool, half: bool) -> u16 {
        let d = self.dims(x);
        self.op(Kind::Resize { align, half }, Act::None, vec![x], vec![1, h, w, d[3]])
    }

    pub fn tconv(&mut self, x: u16, oc: u32, k: u32, s: u32, same: bool, act: Act) -> u16 {
        let d = self.dims(x);
        let (oh, ow) = if same { (d[1] * s, d[2] * s) } else { ((d[1] - 1) * s + k, (d[2] - 1) * s + k) };
        let pt = ((d[1] - 1) * s + k).saturating_sub(oh) / 2;
        let pl = ((d[2] - 1) * s + k).saturating_sub(ow) / 2;
        let w = self.konst(vec![oc, k, k, d[3]], 1.0 / ((k * k * d[3]) as f32).sqrt());
        let b = self.konst(vec![oc], 0.2);
        self.op(Kind::TransposeConv { stride: [s, s], pad: [pt, pl] }, act, vec![x, w, b], vec![1, oh, ow, oc])
    }

    /// Zeros around (rows above, below, columns left, right), then a
    /// VALID convolution: what TensorFlow exports for an asymmetric SAME.
    pub fn pad(&mut self, x: u16, pad: [u32; 4]) -> u16 {
        let d = self.dims(x);
        self.op(Kind::Pad { pad }, Act::None, vec![x], vec![1, d[1] + pad[0] + pad[1], d[2] + pad[2] + pad[3], d[3]])
    }

    pub fn concat(&mut self, parts: &[u16]) -> u16 {
        let d = self.dims(parts[0]);
        let c: u32 = parts.iter().map(|&p| self.dims(p)[3]).sum();
        self.op(Kind::Concat, Act::None, parts.to_vec(), vec![1, d[1], d[2], c])
    }

    /// Squeeze and excite: the mean, two 1x1 convolutions, a hard
    /// sigmoid (as MediaPipe exports it: +3, ReLU6, ×1/6) and the product.
    pub fn se(&mut self, x: u16) -> u16 {
        let c = self.dims(x)[3];
        let g = self.mean(x);
        let r = self.conv(g, (c / 4).max(1), 1, 1, Act::Relu);
        let e = self.conv(r, c, 1, 1, Act::None);
        let three = self.push(vec![1], Some(vec![3.0]));
        let sixth = self.push(vec![1], Some(vec![f16_to_f32(f32_to_f16(1.0 / 6.0))]));
        let t = self.bin(e, three, false, Act::Relu6);
        let h = self.bin(t, sixth, true, Act::None);
        self.bin(x, h, true, Act::None)
    }

    pub fn finish(mut self, out: u16) -> Model {
        self.m.output = out;
        self.m
    }
}

/// The graph evaluated the slow way, in f64.
pub(crate) fn reference(m: &Model, input: &[f32]) -> Vec<f32> {
    let mut v: Vec<Option<Vec<f64>>> = m.tensors.iter().map(|t| t.data.as_ref().map(|d| d.iter().map(|&x| x as f64).collect())).collect();
    v[m.input as usize] = Some(input.iter().map(|&x| x as f64).collect());
    let shape = |t: u16| dims4(&m.tensors[t as usize].dims);
    let act = |a: Act, x: f64| match a {
        Act::None => x,
        Act::Relu => x.max(0.0),
        Act::Relu6 => x.clamp(0.0, 6.0),
        Act::ReluN1To1 => x.clamp(-1.0, 1.0),
        Act::Tanh => x.tanh(),
    };
    for op in &m.ops {
        let od = shape(op.output);
        let n: usize = od.iter().product();
        let get = |k: usize| v[op.inputs[k] as usize].clone().expect("computed before");
        let xd = shape(op.inputs[0]);
        let at = |d: [usize; 4], y: usize, x: usize, c: usize| (y * d[2] + x) * d[3] + c;
        let mut out = vec![0f64; n];
        match op.kind {
            Kind::Conv { stride, pad, dil } | Kind::Depthwise { stride, pad, dil, .. } => {
                let (x, w) = (get(0), get(1));
                let wd = shape(op.inputs[1]);
                let b = (op.inputs.len() > 2).then(|| get(2));
                let dw = matches!(op.kind, Kind::Depthwise { .. });
                let mult = if let Kind::Depthwise { mult, .. } = op.kind { mult as usize } else { 1 };
                for oy in 0..od[1] {
                    for ox in 0..od[2] {
                        for o in 0..od[3] {
                            let mut s = b.as_ref().map_or(0.0, |b| b[o]);
                            for ky in 0..wd[1] {
                                for kx in 0..wd[2] {
                                    let iy = (oy * stride[0] as usize + ky * dil[0] as usize) as isize - pad[0] as isize;
                                    let ix = (ox * stride[1] as usize + kx * dil[1] as usize) as isize - pad[1] as isize;
                                    if iy < 0 || ix < 0 || iy >= xd[1] as isize || ix >= xd[2] as isize {
                                        continue;
                                    }
                                    let (iy, ix) = (iy as usize, ix as usize);
                                    if dw {
                                        s += x[at(xd, iy, ix, o / mult)] * w[(ky * wd[2] + kx) * wd[3] + o];
                                    } else {
                                        for i in 0..xd[3] {
                                            s += x[at(xd, iy, ix, i)] * w[((o * wd[1] + ky) * wd[2] + kx) * wd[3] + i];
                                        }
                                    }
                                }
                            }
                            out[at(od, oy, ox, o)] = act(op.act, s);
                        }
                    }
                }
            }
            Kind::TransposeConv { stride, pad } => {
                let (x, w) = (get(0), get(1));
                let wd = shape(op.inputs[1]);
                let b = (op.inputs.len() > 2).then(|| get(2));
                for iy in 0..xd[1] {
                    for ix in 0..xd[2] {
                        for ky in 0..wd[1] {
                            for kx in 0..wd[2] {
                                let oy = (iy * stride[0] as usize + ky) as isize - pad[0] as isize;
                                let ox = (ix * stride[1] as usize + kx) as isize - pad[1] as isize;
                                if oy < 0 || ox < 0 || oy >= od[1] as isize || ox >= od[2] as isize {
                                    continue;
                                }
                                for o in 0..od[3] {
                                    for i in 0..xd[3] {
                                        out[at(od, oy as usize, ox as usize, o)] += x[at(xd, iy, ix, i)] * w[((o * wd[1] + ky) * wd[2] + kx) * wd[3] + i];
                                    }
                                }
                            }
                        }
                    }
                }
                for (k, o) in out.iter_mut().enumerate() {
                    *o = act(op.act, *o + b.as_ref().map_or(0.0, |b| b[k % od[3]]));
                }
            }
            Kind::Add | Kind::Mul => {
                let (a, b) = (get(0), get(1));
                let (ad, bd) = (shape(op.inputs[0]), shape(op.inputs[1]));
                let ix = |d: [usize; 4], p: [usize; 4]| {
                    let q: Vec<usize> = (0..4).map(|k| if d[k] == 1 { 0 } else { p[k] }).collect();
                    ((q[0] * d[1] + q[1]) * d[2] + q[2]) * d[3] + q[3]
                };
                let mut k = 0;
                for n0 in 0..od[0] {
                    for y in 0..od[1] {
                        for x in 0..od[2] {
                            for c in 0..od[3] {
                                let p = [n0, y, x, c];
                                let (p, q) = (a[ix(ad, p)], b[ix(bd, p)]);
                                out[k] = act(op.act, if op.kind == Kind::Mul { p * q } else { p + q });
                                k += 1;
                            }
                        }
                    }
                }
            }
            Kind::AvgPool { stride, pad, size } => {
                let x = get(0);
                for oy in 0..od[1] {
                    for ox in 0..od[2] {
                        for c in 0..od[3] {
                            let (mut s, mut cnt) = (0.0, 0);
                            for ky in 0..size[0] as usize {
                                for kx in 0..size[1] as usize {
                                    let iy = (oy * stride[0] as usize + ky) as isize - pad[0] as isize;
                                    let ix = (ox * stride[1] as usize + kx) as isize - pad[1] as isize;
                                    if iy >= 0 && ix >= 0 && iy < xd[1] as isize && ix < xd[2] as isize {
                                        s += x[at(xd, iy as usize, ix as usize, c)];
                                        cnt += 1;
                                    }
                                }
                            }
                            out[at(od, oy, ox, c)] = act(op.act, s / cnt.max(1) as f64);
                        }
                    }
                }
            }
            Kind::Mean => {
                let x = get(0);
                for c in 0..xd[3] {
                    let s: f64 = (0..xd[1] * xd[2]).map(|p| x[p * xd[3] + c]).sum();
                    out[c] = act(op.act, s / (xd[1] * xd[2]) as f64);
                }
            }
            Kind::Resize { align, half } => {
                let x = get(0);
                // reference_ops::ResizeBilinear.
                let scale = |i: usize, o: usize| if align && o > 1 { (i as f64 - 1.0) / (o as f64 - 1.0) } else { i as f64 / o as f64 };
                let (sy, sx) = (scale(xd[1], od[1]), scale(xd[2], od[2]));
                let src = |v: usize, s: f64| if half { (v as f64 + 0.5) * s - 0.5 } else { v as f64 * s };
                for oy in 0..od[1] {
                    let fy = src(oy, sy);
                    let (y0, y1) = ((fy.floor() as isize).max(0) as usize, (fy.ceil() as usize).min(xd[1] - 1));
                    for ox in 0..od[2] {
                        let fx = src(ox, sx);
                        let (x0, x1) = ((fx.floor() as isize).max(0) as usize, (fx.ceil() as usize).min(xd[2] - 1));
                        let (dy, dx) = (fy - y0 as f64, fx - x0 as f64);
                        for c in 0..od[3] {
                            let v = x[at(xd, y0, x0, c)] * (1.0 - dy) * (1.0 - dx)
                                + x[at(xd, y1, x0, c)] * dy * (1.0 - dx)
                                + x[at(xd, y0, x1, c)] * (1.0 - dy) * dx
                                + x[at(xd, y1, x1, c)] * dy * dx;
                            out[at(od, oy, ox, c)] = act(op.act, v);
                        }
                    }
                }
            }
            Kind::Pad { pad } => {
                let x = get(0);
                for y in 0..xd[1] {
                    for xx in 0..xd[2] {
                        for c in 0..xd[3] {
                            out[at(od, y + pad[0] as usize, xx + pad[2] as usize, c)] = act(op.act, x[at(xd, y, xx, c)]);
                        }
                    }
                }
            }
            Kind::Sigmoid | Kind::Relu | Kind::Relu6 | Kind::HardSwish | Kind::Reshape => {
                let x = get(0);
                for k in 0..n {
                    let t = x[k];
                    out[k] = act(
                        op.act,
                        match op.kind {
                            Kind::Sigmoid => 1.0 / (1.0 + (-t).exp()),
                            Kind::Relu => t.max(0.0),
                            Kind::Relu6 => t.clamp(0.0, 6.0),
                            Kind::HardSwish => t * (t + 3.0).clamp(0.0, 6.0) / 6.0,
                            _ => t,
                        },
                    );
                }
            }
            Kind::Concat => {
                let mut k = 0;
                for p in 0..od[1] * od[2] {
                    for &t in &op.inputs {
                        let c = shape(t)[3];
                        let x = v[t as usize].as_ref().unwrap();
                        for j in 0..c {
                            out[k] = act(op.act, x[p * c + j]);
                            k += 1;
                        }
                    }
                }
            }
        }
        v[op.output as usize] = Some(out);
    }
    v[m.output as usize].as_ref().unwrap().iter().map(|&x| x as f32).collect()
}

fn random_input(b: &mut B, n: usize) -> Vec<f32> {
    (0..n).map(|_| b.rnd() * 0.5 + 0.5).collect()
}

/// Net output vs the reference: the largest difference, relative to the
/// output's range.
fn compare(m: Model, seed: u32) -> f32 {
    let bytes = m.to_bytes();
    let parsed = Model::parse(&bytes).expect("parses");
    assert_eq!(parsed, m, "selfie.bin keeps the model");
    let mut net = Net::load(&bytes).expect("loads");
    let mut b = B::new(seed);
    let n = net.input_mut().len();
    let x = random_input(&mut b, n);
    net.input_mut().copy_from_slice(&x);
    let got = net.run().to_vec();
    let want = reference(&m, &x);
    assert_eq!(got.len(), want.len());
    let range = want.iter().fold(0f32, |a, v| a.max(v.abs())).max(1e-3);
    got.iter().zip(&want).fold(0f32, |a, (g, w)| a.max((g - w).abs())) / range
}

#[test]
fn half_floats_round_trip_and_round_to_even() {
    for h in 0..=u16::MAX {
        let f = f16_to_f32(h);
        if f.is_nan() {
            assert!(f32_to_f16(f) & 0x7c00 == 0x7c00 && f32_to_f16(f) & 0x3ff != 0);
            continue;
        }
        assert_eq!(f32_to_f16(f), h, "{h:#06x} = {f}");
    }
    for (x, h) in [(1.0f32, 0x3c00u16), (-2.0, 0xc000), (65504.0, 0x7bff), (65520.0, 0x7c00), (1e9, 0x7c00), (0.1, 0x2e66), (5.960_464_5e-8, 1), (2.9e-8, 0), (1e-10, 0), (-0.0, 0x8000)] {
        assert_eq!(f32_to_f16(x), h, "{x}");
    }
    // Halfway cases go to the even neighbour (1 + 2^-11 lies between 1 and 1 + 2^-10).
    assert_eq!(f32_to_f16(1.0 + 1.0 / 2048.0), 0x3c00);
    assert_eq!(f32_to_f16(1.0 + 3.0 / 2048.0), 0x3c02);
}

#[test]
fn every_operator_matches_the_reference() {
    type Make = Box<dyn Fn(&mut B, u16) -> u16>;
    let cases: Vec<(&str, (u32, u32), Make)> = vec![
        ("conv 3x3 s2 same", (13, 17), Box::new(|b, x| b.conv(x, 13, 3, 2, Act::Relu6))),
        ("conv 3x3 s1 valid", (9, 11), Box::new(|b, x| b.conv_full(x, 5, (3, 3), (1, 1), (1, 1), false, true, Act::None))),
        ("conv 5x3 dilated", (12, 14), Box::new(|b, x| b.conv_full(x, 8, (5, 3), (1, 2), (2, 1), true, false, Act::Relu))),
        ("conv tanh", (6, 7), Box::new(|b, x| b.conv(x, 6, 3, 1, Act::Tanh))),
        (
            "pad then a valid stride-2 conv",
            (10, 13),
            Box::new(|b, x| {
                let p = b.pad(x, [0, 1, 1, 2]);
                b.conv_full(p, 7, (3, 3), (2, 2), (1, 1), false, true, Act::Relu)
            }),
        ),
        (
            "pointwise 4x8 blocks and tails",
            (7, 9),
            Box::new(|b, x| {
                let y = b.conv(x, 19, 1, 1, Act::None);
                let z = b.conv(y, 16, 1, 1, Act::Relu);
                let t = b.conv(z, 3, 1, 1, Act::Tanh);
                b.conv(t, 24, 1, 1, Act::ReluN1To1)
            }),
        ),
        (
            "depthwise",
            (11, 10),
            Box::new(|b, x| {
                let y = b.conv(x, 10, 1, 1, Act::None);
                let a = b.dw(y, 3, 1, Act::Relu);
                let c = b.dw(a, 5, 2, Act::Relu6);
                b.dw_full(c, 1, (3, 3), (1, 1), (2, 2), false, false, Act::None)
            }),
        ),
        ("depthwise multiplier 2", (8, 9), Box::new(|b, x| b.dw_full(x, 2, (3, 3), (2, 1), (1, 1), true, true, Act::Relu))),
        ("transposed conv same", (5, 6), Box::new(|b, x| b.tconv(x, 6, 2, 2, true, Act::None))),
        ("transposed conv 3x3 s2", (5, 4), Box::new(|b, x| b.tconv(x, 5, 3, 2, true, Act::Relu))),
        ("transposed conv valid", (4, 5), Box::new(|b, x| b.tconv(x, 2, 3, 2, false, Act::None))),
        (
            "add and mul with broadcasting",
            (6, 5),
            Box::new(|b, x| {
                let y = b.conv(x, 8, 1, 1, Act::None);
                let ch = b.konst(vec![8], 1.0);
                let one = b.konst(vec![1], 1.0);
                let col = b.konst(vec![1, 6, 1, 8], 1.0);
                let s = b.bin(y, ch, false, Act::None);
                let t = b.bin(one, s, true, Act::Relu);
                let u = b.bin(t, col, false, Act::None);
                let g = b.mean(u);
                let v = b.bin(g, u, true, Act::None);
                b.bin(v, y, false, Act::Relu6)
            }),
        ),
        (
            "pools",
            (9, 12),
            Box::new(|b, x| {
                let p = b.pool(x, 3, 2, true);
                let q = b.pool(p, 2, 1, false);
                let g = b.pool(q, 4, 4, false);
                let r = b.resize(g, 4, 5, false, false);
                b.concat(&[r, q])
            }),
        ),
        (
            "resize",
            (5, 7),
            Box::new(|b, x| {
                let a = b.resize(x, 11, 16, false, false);
                let c = b.resize(a, 9, 9, true, false);
                let d = b.resize(c, 20, 13, false, true);
                let e = b.resize(d, 6, 4, false, true);
                let f = b.resize(e, 6, 4, false, false);
                b.resize(f, 1, 1, true, false)
            }),
        ),
        (
            "activations and concat",
            (4, 6),
            Box::new(|b, x| {
                let y = b.conv(x, 5, 3, 1, Act::None);
                let h = b.unary(y, Kind::HardSwish);
                let s = b.unary(y, Kind::Sigmoid);
                let r = b.unary(y, Kind::Relu);
                let r6 = b.unary(h, Kind::Relu6);
                let c = b.concat(&[h, s, r, r6, x]);
                b.unary(c, Kind::Reshape)
            }),
        ),
    ];
    for (i, (name, (h, w), make)) in cases.iter().enumerate() {
        let mut b = B::new(100 + i as u32);
        let x = b.input(*h, *w);
        let y = make(&mut b, x);
        let err = compare(b.finish(y), 7 + i as u32);
        assert!(err < 2e-5, "{name}: relative error {err}");
    }
}

/// A MobileNet-like encoder-decoder (hard-swish, inverted residuals with
/// squeeze and excite, skips added after bilinear upsampling, a
/// transposed convolution to full size, a sigmoid): the whole graph as
/// the reference evaluates it. `w`x`h` input; `c` scales the widths.
pub(crate) fn mobile_model(h: u32, w: u32, c: u32, seed: u32) -> Model {
    let mut b = B::new(seed);
    let x = b.input(h, w);
    let s0 = b.conv(x, 2 * c, 3, 2, Act::None);
    let s0 = b.unary(s0, Kind::HardSwish);
    // Inverted residual: expand, depthwise, squeeze-excite, project.
    let block = |b: &mut B, x: u16, e: u32, o: u32, k: u32, s: u32| {
        let xd = b.dims(x);
        let y = b.conv(x, e, 1, 1, Act::None);
        let y = b.unary(y, Kind::HardSwish);
        let y = b.dw(y, k, s, Act::None);
        let y = b.unary(y, Kind::HardSwish);
        let y = b.se(y);
        let y = b.conv(y, o, 1, 1, Act::None);
        if s == 1 && xd[3] == o { b.bin(x, y, false, Act::None) } else { y }
    };
    let s1 = block(&mut b, s0, 2 * c, 2 * c, 3, 2);
    let s2 = block(&mut b, s1, 6 * c, 3 * c, 3, 2);
    let s2 = block(&mut b, s2, 9 * c, 3 * c, 3, 1);
    let s3 = block(&mut b, s2, 12 * c, 5 * c, 5, 2);
    let s3 = block(&mut b, s3, 15 * c, 5 * c, 5, 1);
    let s4 = block(&mut b, s3, 15 * c, 6 * c, 5, 2);
    let s4 = block(&mut b, s4, 18 * c, 6 * c, 5, 1);
    // Decoder: up, a 1x1 to the skip's width, add, refine.
    let up = |b: &mut B, x: u16, skip: u16| {
        let sd = b.dims(skip);
        let u = b.resize(x, sd[1], sd[2], false, true);
        let u = b.conv(u, sd[3], 1, 1, Act::Relu);
        let a = b.bin(u, skip, false, Act::None);
        let a = b.dw(a, 3, 1, Act::Relu);
        b.conv(a, sd[3], 1, 1, Act::Relu)
    };
    let d3 = up(&mut b, s4, s3);
    let d2 = up(&mut b, d3, s2);
    let d1 = up(&mut b, d2, s1);
    let d0 = up(&mut b, d1, s0);
    let t = b.tconv(d0, 1, 2, 2, true, Act::None);
    let out = b.unary(t, Kind::Sigmoid);
    b.finish(out)
}

#[test]
fn a_mobile_graph_matches_the_reference() {
    let m = mobile_model(36, 64, 4, 3);
    assert_eq!(dims4(&m.tensors[m.output as usize].dims), [1, 36, 64, 1]);
    let err = compare(m, 5);
    assert!(err < 2e-5, "relative error {err}");
}

#[test]
fn bad_models_are_refused() {
    let ok = {
        let mut b = B::new(1);
        let x = b.input(8, 8);
        let y = b.conv(x, 4, 3, 1, Act::None);
        let z = b.conv(y, 1, 1, 1, Act::None);
        b.finish(z)
    };
    assert!(Net::from_model(ok.clone()).is_ok());
    let bytes = ok.to_bytes();
    // Every truncation, and random byte flips, fail cleanly (no panic).
    for n in 0..bytes.len() {
        assert!(Model::parse(&bytes[..n]).is_err(), "truncated at {n}");
    }
    let mut b = B::new(9);
    for _ in 0..3000 {
        let mut c = bytes.clone();
        let i = ((b.rnd() * 0.5 + 0.5) * c.len() as f32) as usize % c.len();
        c[i] ^= 1 << (((b.rnd() * 0.5 + 0.5) * 8.0) as u32 % 8);
        // What loads runs within its checked shapes.
        if let Ok(m) = Model::parse(&c)
            && let Ok(mut n) = Net::with_helpers(m, 1)
        {
            n.run();
        }
    }
    // The same for a graph with every kind of operator in it.
    let mobile = mobile_model(18, 32, 2, 4).to_bytes();
    for _ in 0..600 {
        let mut c = mobile.clone();
        let i = ((b.rnd() * 0.5 + 0.5) * c.len() as f32) as usize % c.len();
        c[i] ^= 1 << (((b.rnd() * 0.5 + 0.5) * 8.0) as u32 % 8);
        if let Ok(m) = Model::parse(&c)
            && let Ok(mut n) = Net::with_helpers(m, 1)
        {
            n.run();
        }
    }
    let with = |f: &dyn Fn(&mut Model)| {
        let mut m = ok.clone();
        f(&mut m);
        Net::from_model(m).err().map(|e| e.0).unwrap_or_default()
    };
    // Weights for 2 input channels on a 3-channel input.
    assert!(
        with(&|m| {
            m.tensors[1].dims[3] = 2;
            m.tensors[1].data.as_mut().unwrap().truncate(4 * 3 * 3 * 2);
        })
        .contains("channels"),
        "channel mismatch"
    );
    assert!(!with(&|m| m.tensors[1].data = None).is_empty(), "weights must be constant");
    assert!(with(&|m| m.tensors[3].dims[1] = 5).contains("output size"));
    assert!(with(&|m| m.ops.swap(0, 1)).contains("not computed"));
    assert!(with(&|m| m.output = m.input).contains("output"));
    assert!(with(&|m| m.tensors[0].dims[3] = 4).contains("input"));
    assert!(with(&|m| m.ops[1].output = 3).contains("twice"));
    // A segmentation model has a 1- or 2-channel output.
    let wide = {
        let mut b = B::new(2);
        let x = b.input(8, 8);
        let y = b.conv(x, 3, 3, 1, Act::None);
        b.finish(y)
    };
    assert!(Net::selfie(&wide.to_bytes()).err().unwrap().0.contains("1xHxWx1 or 2"));
    assert!(Net::selfie(&bytes).is_ok());
}

#[test]
fn input_is_the_bubble_square_filling_the_height() {
    // A 16:9 frame: left half red, right half blue, a green band at the top.
    let (fw, fh) = (320u32, 180u32);
    let mut f = CamFrame { w: fw, h: fh, bgra: vec![0; (fw * fh * 4) as usize] };
    for y in 0..fh {
        for x in 0..fw {
            let p = &mut f.bgra[((y * fw + x) * 4) as usize..][..4];
            let c = if y < 18 { [0, 255, 0] } else if x < fw / 2 { [0, 0, 255] } else { [255, 0, 0] };
            p.copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let mut inp = SegInput::default();
    inp.prepare(&f, 256, 144);
    assert_eq!(inp.sq, Some(Square { x: 56.0, y: 0.0, side: 144.0 }));
    let px = |i: &SegInput, x: usize, y: usize| <[u8; 4]>::try_from(&i.rgba[(y * i.w + x) * 4..][..4]).unwrap();
    assert_eq!(px(&inp, 10, 100), [255, 0, 0, 255], "red (RGB order)");
    assert_eq!(px(&inp, 250, 100), [0, 0, 255, 255]);
    assert_eq!(px(&inp, 128, 5), [0, 255, 0, 255]);
    // 4:3: the square fills the height, black at the sides.
    let f43 = CamFrame { w: 240, h: 180, bgra: [200u8, 100, 50, 255].repeat(240 * 180) };
    inp.prepare(&f43, 256, 144);
    assert_eq!(px(&inp, 5, 70), [0, 0, 0, 255], "letterboxed");
    assert_eq!(px(&inp, 128, 70), [50, 100, 200, 255]);
    assert_eq!(px(&inp, 33, 70), [50, 100, 200, 255], "the frame is 192 wide in the input");
    assert_eq!(px(&inp, 31, 70), [0, 0, 0, 255]);
    // Nothing to read: no square.
    inp.prepare(&CamFrame::default(), 256, 144);
    assert_eq!(inp.sq, None);
}

#[test]
fn the_square_is_cut_from_the_mask() {
    // A 16x9 mask whose value is its x; the square (3.5..12.5 in a 32x18
    // input = 1.75..6.25 of 16x9 rows) comes back 9x9 (its own size there).
    let (mw, mh) = (16, 9);
    let m: Vec<f32> = (0..mw * mh).map(|i| (i % mw) as f32).collect();
    let mut out = SqMask::default();
    crop_square(&m, (mw, mh), (32, 18), Square { x: 7.0, y: 0.0, side: 18.0 }, &mut out);
    assert_eq!(out.n, 9);
    // Column centres: (7 + (i + 0.5) * 2) / 2 - 0.5 = 3.5 + i.
    for i in 0..9 {
        assert!((out.v[4 * 9 + i] - (3.5 + i as f32)).abs() < 1e-5, "{i}: {}", out.v[4 * 9 + i]);
    }
}

#[test]
fn net_segments_the_square() {
    let m = mobile_model(144, 256, 2, 11);
    let mut net = Net::from_model(m).unwrap();
    assert_eq!(net.input_size(), (256, 144));
    let f = CamFrame { w: 1280, h: 720, bgra: (0..1280 * 720 * 4).map(|i| (i * 31 % 251) as u8).collect() };
    let mut inp = SegInput::default();
    inp.prepare(&f, 256, 144);
    let mut sq = SqMask::default();
    assert!(net.segment(&inp, &mut sq));
    assert_eq!((sq.n, sq.v.len()), (144, 144 * 144));
    assert!(sq.v.iter().all(|v| (0.0..=1.0).contains(v)));
    // The wrong input size is refused.
    inp.prepare(&f, 128, 72);
    assert!(!net.segment(&inp, &mut sq));
}

/// The engine's cost on a model of about the real one's size (~0.1 M
/// weights, 144x256). `cargo test --release seg_bench -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn seg_bench() {
    let m = mobile_model(144, 256, 8, 3);
    let weights: usize = m.tensors.iter().filter_map(|t| t.data.as_ref()).map(Vec::len).sum();
    let bytes = m.to_bytes();
    let mut net = Net::load(&bytes).unwrap();
    let f = CamFrame { w: 1280, h: 720, bgra: (0..1280 * 720 * 4).map(|i| (i * 31 % 251) as u8).collect() };
    let mut inp = SegInput::default();
    let t = std::time::Instant::now();
    for _ in 0..20 {
        inp.prepare(&f, 256, 144);
    }
    println!("BENCH prepare 720p -> 256x144 {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
    let mut sq = SqMask::default();
    net.segment(&inp, &mut sq);
    let t = std::time::Instant::now();
    for _ in 0..20 {
        net.segment(&inp, &mut sq);
    }
    println!(
        "BENCH net ({weights} weights, {} ops, {} bytes, {} threads) {:.3} ms",
        m.ops.len(),
        bytes.len(),
        net.threads(),
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );
    let mut tot: Vec<(String, f64, usize)> = Vec::new();
    for _ in 0..10 {
        for (k, ms, n) in net.run_timed() {
            match tot.iter_mut().find(|e| e.0 == k) {
                Some(e) => e.1 += ms / 10.0,
                None => tot.push((k, ms / 10.0, n)),
            }
        }
    }
    for (k, ms, n) in tot {
        println!("BENCH   {k:<14} x{n:<3} {ms:.3} ms");
    }
}

/// The converted MediaPipe model, when `assets/selfie.bin` is there (read
/// at run time, so this runs with or without the `selfie_model` feature).
fn real_model() -> Option<Vec<u8>> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/selfie.bin")).ok()
}

/// Synthetic 256x144 RGB inputs (0..1): a gradient, a bright head and
/// shoulders (ellipses) on a dark background, the same skin-toned on a
/// green one (which the model takes for a person: the dark one it does
/// not), noise.
fn real_inputs() -> Vec<(&'static str, Vec<f32>)> {
    let (w, h) = (256usize, 144usize);
    let px = |f: &dyn Fn(f32, f32) -> [f32; 3]| -> Vec<f32> {
        (0..w * h).flat_map(|i| f((i % w) as f32 / w as f32, (i / w) as f32 / h as f32)).collect()
    };
    let mut s = 12345u32;
    let noise: Vec<f32> = (0..w * h * 3)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as f32 / (1 << 24) as f32
        })
        .collect();
    vec![
        ("gradient", px(&|u, v| [u, v, 1.0 - u])),
        ("bright ellipse on dark", px(&|u, v| if person(u, v) { [0.85, 0.7, 0.6] } else { [0.08, 0.1, 0.12] })),
        ("head and shoulders", px(&|u, v| if person(u, v) { [0.8, 0.6, 0.5] } else { [0.2, 0.7, 0.2] })),
        ("noise", noise),
    ]
}

/// The synthetic person: a head (ellipse) over shoulders (half ellipse),
/// in the middle of a 16:9 picture (u, v in 0..1).
fn person(u: f32, v: f32) -> bool {
    let x = (u - 0.5) * 16.0 / 9.0;
    let head = (x / 0.16).powi(2) + ((v - 0.38) / 0.22).powi(2) < 1.0;
    let body = v > 0.62 && (x / 0.42).powi(2) + ((v - 1.05) / 0.45).powi(2) < 1.0;
    head || body
}

/// The real graph, the engine against the scalar f64 reference (which
/// shares nothing with `ops`), on the synthetic inputs; and the mask is
/// plausible on the head and shoulders.
#[test]
fn real_model_matches_the_reference() {
    let Some(bytes) = real_model() else {
        eprintln!("SKIP: no assets/selfie.bin");
        return;
    };
    let model = Model::parse(&bytes).unwrap();
    let mut net = Net::selfie(&bytes).unwrap();
    assert_eq!((net.input_hw(), net.output_dim()), ((144, 256), (144, 256, 1)));
    for (name, x) in real_inputs() {
        net.input_mut().copy_from_slice(&x);
        let got = net.run().to_vec();
        let want = reference(&model, &x);
        let worst = got.iter().zip(&want).fold(0f32, |a, (g, w)| a.max((g - w).abs()));
        eprintln!("{name}: max |engine - reference| = {worst:e}");
        assert!(worst < 1e-3, "{name}: {worst}");
        assert!(got.iter().all(|v| (0.0..=1.0).contains(v)));
        if name == "head and shoulders" {
            let (w, h) = (256usize, 144usize);
            let (mut inside, mut n_in) = (0, 0);
            for (i, &m) in got.iter().enumerate() {
                if person((i % w) as f32 / w as f32, (i / w) as f32 / h as f32) {
                    n_in += 1;
                    inside += usize::from(m > 0.5);
                }
            }
            let corners = [got[0], got[w - 1], got[(h - 1) * w], got[h * w - 1]];
            eprintln!("person pixels > 0.5: {inside} of {n_in}; corners {corners:?}");
            assert!(inside * 10 >= n_in * 8, "most of the person: {inside} of {n_in}");
            assert!(corners.iter().all(|&c| c < 0.5), "{corners:?}");
        }
    }
}

/// A stand-in `selfie.bin` (random weights, the landscape model's input
/// and output, about its size) at `RUSTSHOT_SYNTH_SELFIE`: to build and
/// try the `selfie_model` feature without the real model.
#[test]
#[ignore = "writes a synthetic model; set RUSTSHOT_SYNTH_SELFIE"]
fn write_synthetic_selfie_bin() {
    let Some(path) = std::env::var_os("RUSTSHOT_SYNTH_SELFIE") else { return };
    let bytes = mobile_model(144, 256, 8, 3).to_bytes();
    std::fs::write(&path, &bytes).unwrap();
    println!("wrote {} bytes to {}", bytes.len(), std::path::Path::new(&path).display());
}

/// The model this build embeds (`selfie_model`): it loads, takes a
/// 144x256 picture (the landscape model) and gives a probability per
/// pixel. `cargo test --features selfie_model embedded_model`
#[cfg(feature = "selfie_model")]
#[test]
fn embedded_model_loads_and_runs() {
    let bytes = EMBEDDED.expect("the feature embeds the model");
    assert!(bytes.len() < 300 * 1024, "{} bytes (the binary may grow less than 300 KB)", bytes.len());
    let mut net = Net::selfie(bytes).expect("assets/selfie.bin loads");
    assert_eq!(net.input_size(), (256, 144));
    let f = CamFrame { w: 1280, h: 720, bgra: (0..1280 * 720 * 4).map(|i| (i * 31 % 251) as u8).collect() };
    let mut inp = SegInput::default();
    inp.prepare(&f, 256, 144);
    let mut sq = SqMask::default();
    assert!(net.segment(&inp, &mut sq));
    assert!(sq.n > 0 && sq.v.iter().all(|v| (0.0..=1.0).contains(v)));
}
