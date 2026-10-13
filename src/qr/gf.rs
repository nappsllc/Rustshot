//! GF(256) arithmetic and Reed–Solomon coding as QR codes use them:
//! the field's primitive polynomial is x⁸+x⁴+x³+x²+1 (0x11D), α = 2, and
//! the generator polynomial's roots are α⁰ … αⁿ⁻¹. A block's byte 0 is the
//! coefficient of the highest power.

struct Tables {
    exp: [u8; 512],
    log: [u8; 256],
}

const fn build() -> Tables {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11D;
        }
        i += 1;
    }
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    Tables { exp, log }
}

static T: Tables = build();

pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        T.exp[T.log[a as usize] as usize + T.log[b as usize] as usize]
    }
}

/// `a / b`; `b` must not be zero (a zero divisor gives zero, never a panic).
pub fn div(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        T.exp[T.log[a as usize] as usize + 255 - T.log[b as usize] as usize]
    }
}

/// αⁱ.
pub fn alpha(i: usize) -> u8 {
    T.exp[i % 255]
}

/// The generator polynomial of degree `n`, highest power first (`g[0] == 1`).
pub fn generator(n: usize) -> Vec<u8> {
    let mut g = vec![1u8];
    for i in 0..n {
        let a = alpha(i);
        let mut next = vec![0u8; g.len() + 1];
        for (j, &c) in g.iter().enumerate() {
            next[j] ^= c;
            next[j + 1] ^= mul(c, a);
        }
        g = next;
    }
    g
}

/// The `n` error-correction bytes for `data`.
pub fn remainder(data: &[u8], n: usize) -> Vec<u8> {
    let g = generator(n);
    let mut rem = vec![0u8; n];
    if n == 0 {
        return rem;
    }
    for &b in data {
        let factor = b ^ rem[0];
        rem.copy_within(1.., 0);
        rem[n - 1] = 0;
        for j in 0..n {
            rem[j] ^= mul(g[j + 1], factor);
        }
    }
    rem
}

/// Evaluate a polynomial given highest power first at `x`.
fn eval_hi(p: &[u8], x: u8) -> u8 {
    p.iter().fold(0, |s, &c| mul(s, x) ^ c)
}

/// Evaluate a polynomial given lowest power first at `x`.
fn eval_lo(p: &[u8], x: u8) -> u8 {
    p.iter().rev().fold(0, |s, &c| mul(s, x) ^ c)
}

/// Correct `block` (data then `nsym` check bytes) in place. Returns the
/// number of corrected bytes, or `None` when there are more errors than the
/// code can fix (the block is then left as it was).
pub fn correct(block: &mut [u8], nsym: usize) -> Option<usize> {
    let n = block.len();
    if nsym == 0 || nsym >= n || n > 255 {
        return if nsym == 0 { Some(0) } else { None };
    }
    let synd: Vec<u8> = (0..nsym).map(|j| eval_hi(block, alpha(j))).collect();
    if synd.iter().all(|&s| s == 0) {
        return Some(0);
    }
    // Berlekamp–Massey: the error locator Λ (lowest power first).
    let mut c = vec![0u8; nsym + 1];
    let mut b = vec![0u8; nsym + 1];
    c[0] = 1;
    b[0] = 1;
    let mut l = 0usize;
    let mut m = 1usize;
    let mut bb = 1u8;
    for k in 0..nsym {
        let mut d = synd[k];
        for i in 1..=l {
            d ^= mul(c[i], synd[k - i]);
        }
        if d == 0 {
            m += 1;
            continue;
        }
        let coef = div(d, bb);
        let t = c.clone();
        for i in 0..=nsym {
            if i >= m {
                c[i] ^= mul(coef, b[i - m]);
            }
        }
        if 2 * l <= k {
            l = k + 1 - l;
            b = t;
            bb = d;
            m = 1;
        } else {
            m += 1;
        }
    }
    if l == 0 || 2 * l > nsym {
        return None;
    }
    let lambda = &c[..=l];
    // Chien search over the block's positions.
    let mut pos = Vec::with_capacity(l);
    for i in 0..n {
        let p = n - 1 - i; // the power of x at byte i
        let xinv = alpha(255 - p % 255);
        if eval_lo(lambda, xinv) == 0 {
            pos.push(i);
        }
    }
    if pos.len() != l {
        return None;
    }
    // Forney: Ω = S·Λ mod x^nsym; e = X·Ω(X⁻¹) / Λ'(X⁻¹).
    let mut omega = vec![0u8; nsym];
    for i in 0..nsym {
        for j in 0..=l.min(i) {
            omega[i] ^= mul(synd[i - j], lambda[j]);
        }
    }
    let deriv: Vec<u8> = (1..=l).map(|i| if i % 2 == 1 { lambda[i] } else { 0 }).collect();
    let orig = block.to_vec();
    for &i in &pos {
        let p = n - 1 - i;
        let x = alpha(p);
        let xinv = alpha(255 - p % 255);
        let den = eval_lo(&deriv, xinv);
        if den == 0 {
            block.copy_from_slice(&orig);
            return None;
        }
        block[i] ^= mul(x, div(eval_lo(&omega, xinv), den));
    }
    if (0..nsym).any(|j| eval_hi(block, alpha(j)) != 0) {
        block.copy_from_slice(&orig);
        return None;
    }
    Some(pos.len())
}
