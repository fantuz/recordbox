//! Signal helpers of the phrase analyser port (`phrase_rb`): the reference's IIR filter class
//! (with its fused multiply-adds), the 8th-order decimators, and the array reductions in the
//! order the reference's vectorised loops add (so results match to the last bit).

/// Second-order section in the reference's transposed form; first order uses c[0], c[1], c[3].
#[derive(Clone, Copy, Debug, Default)]
pub struct Filter {
    pub c: [f64; 5],
    pub s: [f64; 2],
}

/// Bilinear prewarp of an angular frequency `w` (rad/s) at rate `sr`.
pub fn prewarp(w: f64, sr: f64) -> f64 {
    (w / (sr + sr)).tan() * (sr + sr)
}

impl Filter {
    pub fn clear(&mut self) {
        self.s = [0.0; 2];
    }

    /// Analogue first-order prototype (b0 + b1 s) / (a0 + a1 s) → digital (bilinear, 1/sr).
    pub fn make1(&mut self, b1: f64, b0: f64, a1: f64, a0: f64, sr: f64) {
        let t = 1.0 / sr;
        let v7 = t * a0;
        let v8 = a1 + a1;
        let v9 = 1.0 / (v7 + v8);
        let v10 = t * b0;
        let v11 = b1 + b1;
        self.c[0] = v9 * (v10 + v11);
        self.c[1] = v9 * (v10 - v11);
        self.c[3] = v9 * (v7 - v8);
    }

    /// Analogue second-order prototype (b2 s² + b1 s + b0) / (a2 s² + a1 s + a0) → digital.
    #[allow(clippy::too_many_arguments)]
    pub fn make2(&mut self, b2: f64, b1: f64, b0: f64, a2: f64, a1: f64, a0: f64, sr: f64) {
        let v6 = 1.0 / sr;
        let v16 = v6 * v6;
        let v5 = v16 * a0;
        let v6b = v6 + v6;
        let v4 = v6b * a1;
        let v3 = a2 * 4.0;
        let v7 = 1.0 / ((v4 + v3) + v5);
        let v2 = v16 * b0;
        let v1 = v6b * b1;
        let v0 = b2 * 4.0;
        self.c[0] = v7 * ((v1 + v0) + v2);
        self.c[1] = (v7 + v7) * (v2 - v0);
        self.c[2] = v7 * ((v0 - v1) + v2);
        self.c[3] = (v7 + v7) * (v5 - v3);
        self.c[4] = v7 * ((v3 - v4) + v5);
    }

    /// First-order filter over `x` into `y` (may be the same buffer through `filter1_inplace`).
    pub fn filter1(&mut self, x: &[f64], y: &mut [f64]) {
        let (b0, b1, a1) = (self.c[0], self.c[1], self.c[3]);
        let mut s = self.s[0];
        for (xi, yi) in x.iter().zip(y.iter_mut()) {
            let v = xi.mul_add(b0, s);
            s = xi.mul_add(b1, -(v * a1));
            *yi = v;
        }
        self.s[0] = s;
    }

    pub fn filter1_inplace(&mut self, y: &mut [f64]) {
        let (b0, b1, a1) = (self.c[0], self.c[1], self.c[3]);
        let mut s = self.s[0];
        for yi in y.iter_mut() {
            let xi = *yi;
            let v = xi.mul_add(b0, s);
            s = xi.mul_add(b1, -(v * a1));
            *yi = v;
        }
        self.s[0] = s;
    }

    #[inline]
    pub fn step2(&mut self, x: f64) -> f64 {
        let c = &self.c;
        let v = x.mul_add(c[0], self.s[0]);
        let t = x.mul_add(c[1], self.s[1]);
        self.s[0] = (-v).mul_add(c[3], t);
        self.s[1] = x.mul_add(c[2], -(v * c[4]));
        v
    }

    /// Second-order filter over `x` into `y`.
    pub fn filter2(&mut self, x: &[f64], y: &mut [f64]) {
        for (xi, yi) in x.iter().zip(y.iter_mut()) {
            *yi = self.step2(*xi);
        }
    }

    pub fn filter2_inplace(&mut self, y: &mut [f64]) {
        for yi in y.iter_mut() {
            *yi = self.step2(*yi);
        }
    }
}

/// Four cascaded second-order sections (8th-order Butterworth) and a decimation factor:
/// `SongStructureAnalyze_resample2` (low-pass) and `resample_hpf` (high-pass).
#[derive(Clone, Debug, Default)]
pub struct Decimator {
    pub f: [Filter; 4],
    pub factor: i32,
}

const BUTTER8: [f64; 4] = [1.9615705608064609, 1.6629392246050905, 1.1111404660392046, 0.39018064403225666];

impl Decimator {
    pub fn init_lowpass(&mut self, fc: f64, sr: f64, factor: i32) -> bool {
        if sr > 0.0 && factor >= 2 {
            let w = prewarp(fc * 6.2831853071795862, sr);
            let w2 = w * w;
            for (k, q) in BUTTER8.iter().enumerate() {
                self.f[k].make2(0.0, 0.0, w2, 1.0, w * q, w2, sr);
                self.f[k].clear();
            }
            self.factor = factor;
        }
        sr > 0.0 && factor > 1
    }

    pub fn init_highpass(&mut self, fc: f64, sr: f64, factor: i32) -> bool {
        if sr > 0.0 && factor >= 2 {
            let w = prewarp(fc * 6.2831853071795862, sr);
            for (k, q) in BUTTER8.iter().enumerate() {
                self.f[k].make2(1.0, 0.0, 0.0, 1.0, w * q, w * w, sr);
                self.f[k].clear();
            }
            self.factor = factor;
        }
        sr > 0.0 && factor > 1
    }

    /// `filter_8th_ds::downsample`: filter the block, keep every factor-th sample (the last of
    /// each group).
    pub fn downsample(&mut self, x: &[f64], out: &mut Vec<f64>) -> bool {
        let d = self.factor;
        if d < 2 {
            return false;
        }
        let n = x.len();
        out.clear();
        out.resize(n, 0.0);
        self.f[0].filter2(x, out);
        for k in 1..4 {
            self.f[k].filter2_inplace(out);
        }
        let m = n / d as usize;
        for i in 0..m {
            out[i] = out[(i + 1) * d as usize - 1];
        }
        out.truncate(m);
        true
    }
}

/// `MatArrayFloat::sum`: eight running sums for arrays of eight or more, then the rest.
pub fn sum8(x: &[f64]) -> f64 {
    let n = x.len();
    if n < 8 {
        let mut r = 0.0;
        for v in x {
            r += *v;
        }
        return r;
    }
    let n8 = n & !7;
    let mut a = [0f64; 8];
    for c in x[..n8].chunks_exact(8) {
        for j in 0..8 {
            a[j] += c[j];
        }
    }
    let l0 = a[6] + (a[4] + (a[2] + a[0]));
    let l1 = a[7] + (a[5] + (a[3] + a[1]));
    let mut r = l0 + l1;
    for v in &x[n8..] {
        r += *v;
    }
    r
}

/// `MatArrayFloat::square_sum` (fused multiply-adds, eight running sums).
pub fn square_sum8(x: &[f64]) -> f64 {
    let n = x.len();
    if n < 8 {
        let mut r = 0.0f64;
        for v in x {
            r = v.mul_add(*v, r);
        }
        return r;
    }
    let n8 = n & !7;
    let mut a = [0f64; 8];
    for c in x[..n8].chunks_exact(8) {
        for j in 0..8 {
            a[j] = c[j].mul_add(c[j], a[j]);
        }
    }
    let l0 = a[6] + (a[4] + (a[2] + a[0]));
    let l1 = a[7] + (a[5] + (a[3] + a[1]));
    let mut r = l0 + l1;
    for v in &x[n8..] {
        r = v.mul_add(*v, r);
    }
    r
}

/// `MatArrayInt::sum`: plain integer sum (wrapping like the 32-bit original).
pub fn isum(x: &[i32]) -> i32 {
    x.iter().fold(0i32, |a, b| a.wrapping_add(*b))
}

/// Value and 0-based index of the first maximum (0.0, 0 for an empty slice).
pub fn max_idx(x: &[f64]) -> (f64, usize) {
    if x.is_empty() {
        return (0.0, 0);
    }
    let mut best = x[0];
    let mut bi = 0;
    for (i, v) in x.iter().enumerate().skip(1) {
        if best < *v {
            bi = i;
        }
        best = best.max(*v);
    }
    (x[bi], bi)
}

pub fn max_val(x: &[f64]) -> f64 {
    max_idx(x).0
}

/// Value and index of the first minimum.
pub fn min_idx(x: &[f64]) -> (f64, usize) {
    if x.is_empty() {
        return (0.0, 0);
    }
    let mut best = x[0];
    let mut bi = 0;
    for (i, v) in x.iter().enumerate().skip(1) {
        if *v < best {
            bi = i;
        }
        best = v.min(best);
    }
    (x[bi], bi)
}

pub fn min_val(x: &[f64]) -> f64 {
    min_idx(x).0
}

pub fn imax_idx(x: &[i32]) -> (i32, usize) {
    if x.is_empty() {
        return (0, 0);
    }
    let mut bi = 0;
    for (i, v) in x.iter().enumerate().skip(1) {
        if *v > x[bi] {
            bi = i;
        }
    }
    (x[bi], bi)
}

pub fn imin_idx(x: &[i32]) -> (i32, usize) {
    if x.is_empty() {
        return (0, 0);
    }
    let mut bi = 0;
    for (i, v) in x.iter().enumerate().skip(1) {
        if *v < x[bi] {
            bi = i;
        }
    }
    (x[bi], bi)
}

/// C's `(int)x` for doubles in range (truncation toward zero; saturating outside like arm64).
#[inline]
pub fn ti(x: f64) -> i32 {
    x as i32
}

/// Element of a slice with the reference's 1-based indexing, 0.0 outside (the original reads
/// whatever memory is there; these reads do not happen on real input).
#[inline]
pub fn at1(x: &[f64], i: i64) -> f64 {
    if i >= 1 && (i as usize) <= x.len() {
        x[i as usize - 1]
    } else {
        0.0
    }
}

#[inline]
pub fn at0(x: &[f64], i: i64) -> f64 {
    if i >= 0 && (i as usize) < x.len() {
        x[i as usize]
    } else {
        0.0
    }
}

/// A window of `len` elements from 0-based `start`, clipped to the slice.
#[inline]
pub fn win(x: &[f64], start: i64, len: i64) -> &[f64] {
    let a = start.clamp(0, x.len() as i64) as usize;
    let b = (start + len).clamp(a as i64, x.len() as i64) as usize;
    &x[a..b]
}
