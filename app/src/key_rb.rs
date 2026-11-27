//! Key detection modelled on the reference application's analyser (see
//! `checks/key-detector-trace.md` for the trace this follows).
//!
//! Signal path: left-weighted mono mix → 8th-order low-pass and ÷8 decimation (per one-second
//! block) → percussive suppression in a 512-point STFT → reference-pitch search (434–446 Hz) →
//! 72-bin constant-Q magnitudes (A0 … G#6, Hann, hop 2048) → 12 pitch-class energies → rank
//! tables for major/minor candidates → the same analysis on every two-beat (or four/eight-beat)
//! segment of the beat grid → a vote over the segments.

use super::key_rules as rules;

/// One beat of the grid: number within the bar (1–4) and time in milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct GridBeat {
    pub num: u16,
    pub ms: u32,
}

/// Result of one analysis, with the intermediate decisions kept for comparison runs.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    /// Reference pitch of A4 in Hz.
    pub tune: f32,
    /// Best and second-best key of the whole track (A-based index, +12 for minor).
    pub whole: (i32, i32),
    /// Best and second-best key of each beat segment.
    pub segs: Vec<(i32, i32)>,
    /// Final key (A-based index, +12 for minor), or -1.
    pub key: i32,
    pub step: usize,
}

// ------------------------------------------------------------------------------------------
// Small helpers
// ------------------------------------------------------------------------------------------

/// Hann window without the zero end points: w[i] = 0.5 − 0.5·cos(2π(i+1)/(n+1)).
fn hann(n: usize) -> Vec<f32> {
    let d = (n + 1) as f64;
    (1..=n).map(|i| (0.5 - (i as f64 * 6.28318531 / d).cos() * 0.5) as f32).collect()
}

/// Dot product of `x` with the cosine and sine parts of a kernel.
#[inline]
fn dot2(x: &[f32], c: &[f32], s: &[f32]) -> (f32, f32) {
    let n = c.len().min(s.len()).min(x.len());
    let (x, c, s) = (&x[..n], &c[..n], &s[..n]);
    let mut ac = [0f32; 16];
    let mut as_ = [0f32; 16];
    let chunks = n / 16;
    for b in 0..chunks {
        let o = b * 16;
        let (xb, cb, sb) = (&x[o..o + 16], &c[o..o + 16], &s[o..o + 16]);
        for j in 0..16 {
            ac[j] += cb[j] * xb[j];
            as_[j] += sb[j] * xb[j];
        }
    }
    let fold = |a: &[f32; 16]| -> f32 {
        let mut v = [0f32; 4];
        for j in 0..4 {
            v[j] = a[12 + j] + (a[8 + j] + (a[4 + j] + a[j]));
        }
        (v[0] + v[1]) + (v[2] + v[3])
    };
    let (mut rc, mut rs) = (fold(&ac), fold(&as_));
    for i in chunks * 16..n {
        rc += c[i] * x[i];
        rs += s[i] * x[i];
    }
    (rc, rs)
}

/// Radix-2 FFT on separate real/imaginary arrays, not normalised; `inverse` flips the sign.
struct Fft {
    n: usize,
    sin: Vec<f32>,
    rev: Vec<usize>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        let sin = (0..n + n / 4).map(|i| (2.0 * std::f64::consts::PI * i as f64 / n as f64).sin() as f32).collect();
        let bits = n.trailing_zeros();
        let rev = (0..n).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) }).collect();
        Fft { n, sin, rev }
    }

    fn run(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let quarter = n / 4;
        let mut half = 1;
        while half < n {
            let span = 2 * half;
            let stride = n / span;
            for k in 0..half {
                let c = self.sin[k * stride + quarter];
                let s = self.sin[k * stride];
                let mut i = k;
                while i < n {
                    let (xr, xi) = (re[i + half], im[i + half]);
                    let (tr, ti) = if inverse { (xr * c - xi * s, xi * c + xr * s) } else { (xr * c + xi * s, xi * c - xr * s) };
                    re[i + half] = re[i] - tr;
                    re[i] += tr;
                    im[i + half] = im[i] - ti;
                    im[i] += ti;
                    i += span;
                }
            }
            half = span;
        }
    }
}

/// Indices of `v` ordered by descending value; equal values keep ascending index order.
fn rank_desc(v: &[f32]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[b].partial_cmp(&v[a]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
    idx
}

// ------------------------------------------------------------------------------------------
// Stage 1: mono mix, low-pass and decimation by 8
// ------------------------------------------------------------------------------------------

const LP_A: [f32; 8] = [6.3903646, -18.000338, 29.171099, -29.731375, 19.505632, -8.0409956, 1.9036689, -0.19810002];
const LP_B: [f32; 5] = [1.7625537e-7, 1.4100430e-6, 4.9351506e-6, 9.8703013e-6, 1.2337876e-5];

/// 8th-order low-pass over one block (state starts at rest), keeping every 8th output sample.
/// The recursion is badly conditioned, so the order of operations follows the original.
fn lowpass_decimate(block: impl Iterator<Item = f32>, out: &mut [f32]) {
    let a: [f64; 8] = std::array::from_fn(|i| LP_A[i] as f64);
    let b: [f64; 5] = std::array::from_fn(|i| LP_B[i] as f64);
    let mut w = [0f64; 8]; // w[0] newest
    let mut k = 0;
    for (i, x) in block.enumerate() {
        let mut acc = w[0] * a[0];
        for j in 1..8 {
            acc = w[j].mul_add(a[j], acc);
        }
        let wn = (acc + x as f64) as f32;
        let tail = (wn * LP_B[0]) as f64;
        let mut y = w[3] * b[4];
        y = (w[4] + w[2]).mul_add(b[3], y);
        y = (w[5] + w[1]).mul_add(b[2], y);
        y = w[7].mul_add(b[0], y);
        y = (w[6] + w[0]).mul_add(b[1], y);
        y += tail;
        for j in (1..8).rev() {
            w[j] = w[j - 1];
        }
        w[0] = wn as f64;
        if i % 8 == 7 && k < out.len() {
            out[k] = y as f32;
            k += 1;
        }
    }
}

/// Low-pass and decimate by 8. Reset the filter every second. The tail the blocks do not
/// reach stays zero, including when a full second does not divide into groups of eight.
fn decimate_blocks(mut input: impl ExactSizeIterator<Item = f32>, sr: u32) -> Vec<f32> {
    let total = input.len();
    let ndec = (total / 8).min(0xA4CB80);
    let mut dec = vec![0f32; ndec];
    let block = (sr as usize).max(8);
    let (mut pos, mut done) = (0usize, 0usize);
    while done < total && pos < ndec {
        let n = (total - done).min(block);
        let dn = (n / 8).min(ndec - pos);
        lowpass_decimate(input.by_ref().take(n), &mut dec[pos..pos + dn]);
        pos += n / 8;
        done += n;
    }
    dec
}

fn decimate_pairs(frames: impl ExactSizeIterator<Item = (f32, f32)>, sr: u32) -> Vec<f32> {
    let (wl, wr) = if frames.len() < 30 * sr as usize { (0.999f32, 0.00099999f32) } else { (0.99f32, 0.01f32) };
    decimate_blocks(frames.map(|(left, right)| right.mul_add(wr, left * wl)), sr)
}

pub fn decimate(left: &[f32], right: Option<&[f32]>, sr: u32) -> Vec<f32> {
    match right {
        Some(r) => decimate_pairs(left.iter().enumerate().map(|(i, &l)| (l, r[i])), sr),
        None => decimate_blocks(left.iter().copied(), sr),
    }
}

pub(super) fn decimate_stereo(samples: &[f32], sr: u32) -> Vec<f32> {
    decimate_pairs(samples.chunks_exact(2).map(|frame| (frame[0], frame[1])), sr)
}

// ------------------------------------------------------------------------------------------
// Stage 2: percussive suppression
// ------------------------------------------------------------------------------------------

/// STFT gate: keeps only bins that stand out of their ±half-octave neighbourhood, limits the
/// lowest 20 bins to their slow envelope (kick drums), and resynthesises by overlap-add.
pub fn cut_beats(x: &[f32], rate: f32) -> Vec<f32> {
    let n = x.len();
    let nfft = 512usize;
    let half = nfft / 2;
    let hop = if rate * 30.0 <= n as f32 { 128usize } else { 64 };
    let mut out = vec![0f32; n];
    if n <= nfft {
        return out;
    }
    let frames = (n - nfft) / hop;
    let lo: Vec<usize> = (0..half).map(|k| (k as f32 * 0.70711) as usize).collect();
    let hi: Vec<usize> = (0..half)
        .map(|k| {
            let h = (k as f32 * 1.4142) as usize;
            if half <= h {
                half - 1
            } else {
                h
            }
        })
        .collect();
    let wgt: Vec<f32> = (0..half)
        .map(|k| {
            let l = (k as f32 + 1.0).log10();
            if hi[k] != lo[k] {
                l / (hi[k] as f32 - lo[k] as f32)
            } else {
                l
            }
        })
        .collect();
    let win = hann(nfft);
    let fft = Fft::new(nfft);
    let mut mag = vec![0f32; frames * half];
    let mut sn = vec![0f32; frames * half];
    let mut cs = vec![0f32; frames * half];
    let mut re = vec![0f32; nfft];
    let mut im = vec![0f32; nfft];
    let mut sum = vec![0f32; half];
    for f in 0..frames {
        let o = f * hop;
        for i in 0..nfft {
            re[i] = win[i] * x[o + i];
            im[i] = 0.0;
        }
        fft.run(&mut re, &mut im, false);
        let m = &mut mag[f * half..(f + 1) * half];
        for k in 0..half {
            m[k] = (re[k] * re[k] + im[k] * im[k]).sqrt();
            let r = re[k];
            let (mut t, mut h) = (0f32, 1f32);
            if r > 0.00000011921 || r < -0.00000011921 {
                t = im[k] / r;
                h = (t * t + 1.0).sqrt();
            }
            sn[f * half + k] = if r >= 0.0 { t } else { -t } / h;
            cs[f * half + k] = if r < 0.0 { -1.0 } else { 1.0 } / h;
        }
        for k in 4..half {
            let mut s = 0f32;
            if hi[k] >= lo[k] {
                for j in lo[k]..=hi[k] {
                    s += m[j];
                }
            }
            sum[k] = s;
        }
        for k in 4..half {
            if m[k] < wgt[k] * sum[k] {
                m[k] = 0.0;
            }
        }
    }
    // Lowest 20 bins: never above their one-pole smoothed envelope.
    let b = ((((nfft / hop) as f32 * 5.0) / rate) as f64 * -6.28318531).exp() as f32;
    let a = (1.0 - b as f64) as f32;
    for k in 0..20 {
        let mut acc = 0f32;
        for f in 0..frames {
            let y = acc + mag[f * half + k] * a;
            acc = y * b;
            if mag[f * half + k] > y {
                mag[f * half + k] = y;
            }
        }
    }
    let norm = 1.0 / (nfft as f32 * nfft as f32);
    let gain = hop as f32 * 2.6667;
    for f in 0..frames {
        for v in re.iter_mut() {
            *v = 0.0;
        }
        for v in im.iter_mut() {
            *v = 0.0;
        }
        for k in 0..half {
            let m = mag[f * half + k];
            let (r, i) = (cs[f * half + k] * m, sn[f * half + k] * m);
            re[k] = r;
            im[k] = i;
            if k > 0 {
                re[nfft - k] = r;
                im[nfft - k] = -i;
            }
        }
        fft.run(&mut re, &mut im, true);
        let o = f * hop;
        for i in 0..nfft {
            out[o + i] += ((gain * re[i]) * win[i]) * norm;
        }
    }
    out
}

// ------------------------------------------------------------------------------------------
// Stage 3: reference pitch
// ------------------------------------------------------------------------------------------

/// Complex sinusoid kernels at semitone steps from `f0`, each Hann-windowed over two periods
/// of its distance to the next semitone (constant Q ≈ 33.6).
struct Kernels {
    stride: usize,
    len: Vec<usize>,
    norm: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
}

fn kernels(freqs: &[f32], rate: f32) -> Kernels {
    let bins = freqs.len() - 1;
    let two = rate + rate;
    let l0f = (two / (freqs[1] - freqs[0])).trunc();
    let l0 = l0f as usize;
    let inv = 1.0 / rate;
    let t: Vec<f32> = (0..l0).map(|i| (i as f32 + 1.0) * inv).collect();
    let mut k = Kernels { stride: l0, len: vec![0; bins], norm: vec![0.0; bins], cos: vec![0.0; bins * l0], sin: vec![0.0; bins * l0] };
    for b in 0..bins {
        let lf = if b == 0 { l0f } else { (two / (freqs[b + 1] - freqs[b])).trunc() };
        let l = (lf as usize).min(l0);
        let w = hann(l);
        let ang = freqs[b] * 6.2832f32;
        for i in 0..l {
            let (s, c) = (ang * t[i]).sin_cos();
            k.cos[b * l0 + i] = c * w[i];
            k.sin[b * l0 + i] = s * w[i];
        }
        k.len[b] = l;
        k.norm[b] = if b == 0 { 1.0 } else { l0f / lf };
    }
    k
}

const SEMITONE: [f32; 24] = [
    1.0594631, 1.122462, 1.1892071, 1.2599211, 1.3348398, 1.4142135, 1.4983071, 1.587401, 1.6817929, 1.7817974, 1.8877486, 2.0, 2.1189263, 2.244924,
    2.3784142, 2.5198421, 2.6696796, 2.828427, 2.9966142, 3.174802, 3.3635857, 3.5635948, 3.7754972, 4.0,
];

/// Score of one reference pitch: the strongest of 24 semitone bins is added up over all frames
/// in which the same bin was also the strongest in the previous frame.
fn tuning_score(x: &[f32], rate: f32, reference: f32, hop: usize, last_best: &mut usize) -> f32 {
    let mut freqs = vec![reference; 25];
    for k in 1..25 {
        freqs[k] = SEMITONE[k - 1] * reference;
    }
    // The first kernel's length uses the rounded semitone ratio.
    let two = rate + rate;
    let l0 = (two / (reference * 0.059463)) as usize;
    let mut ks = kernels(&freqs, rate);
    if ks.stride != l0 {
        // Rebuild with the exact first length (differs by at most one sample).
        freqs[1] = reference + two / l0 as f32;
        let exact = SEMITONE[0] * reference;
        ks = kernels_with_first(&freqs, rate, l0, exact, reference);
    }
    let n = x.len();
    if n <= l0 {
        return 0.0;
    }
    let limit = n - l0;
    let mut score = 0f32;
    let mut m = 0usize;
    loop {
        let mut prev = *last_best;
        let mut best = 0usize;
        let mut top = 1.1755e-38f32;
        for b in 0..24 {
            let l = ks.len[b];
            let o = b * ks.stride;
            let (c, s) = dot2(&x[m..m + l], &ks.cos[o..o + l], &ks.sin[o..o + l]);
            let e = ks.norm[b] * (c * c + s * s).sqrt();
            if top < e {
                best = b;
                if m == 0 {
                    prev = b;
                }
            }
            top = top.max(e);
        }
        *last_best = best;
        if best == prev {
            score += top;
        }
        m += hop;
        if m >= limit {
            break;
        }
    }
    score
}

/// As `kernels`, but with the first kernel's length given and the second frequency restored.
fn kernels_with_first(freqs: &[f32], rate: f32, l0: usize, f1: f32, f0: f32) -> Kernels {
    let mut fr = freqs.to_vec();
    fr[1] = f1;
    let bins = fr.len() - 1;
    let two = rate + rate;
    let inv = 1.0 / rate;
    let t: Vec<f32> = (0..l0).map(|i| (i as f32 + 1.0) * inv).collect();
    let mut k = Kernels { stride: l0, len: vec![0; bins], norm: vec![0.0; bins], cos: vec![0.0; bins * l0], sin: vec![0.0; bins * l0] };
    for b in 0..bins {
        let l = if b == 0 { l0 } else { ((two / (fr[b + 1] - fr[b])) as usize).min(l0) };
        let w = hann(l);
        let ang = if b == 0 { f0 } else { fr[b] } * 6.2832f32;
        for i in 0..l {
            let (s, c) = (ang * t[i]).sin_cos();
            k.cos[b * l0 + i] = c * w[i];
            k.sin[b * l0 + i] = s * w[i];
        }
        k.len[b] = l;
        k.norm[b] = if b == 0 { 1.0 } else { l0 as f32 / l as f32 };
    }
    k
}

/// Reference pitch of A4 in whole Hz, 434–446.
pub fn reference_pitch(x: &[f32], rate: f32) -> f32 {
    let long = rate * 30.0 <= x.len() as f32;
    let mut scores = [0f32; 13];
    let mut last = 0usize;
    if long {
        for c in 0..13 {
            scores[c] = tuning_score(x, rate, 434.0 + c as f32, 384, &mut last);
        }
    } else {
        // Short input: every second candidate first, then the ones next to the best.
        let mut c = 2;
        while c < 11 {
            scores[c] = tuning_score(x, rate, 434.0 + c as f32, 256, &mut last);
            c += 2;
        }
        let order = rank_desc(&scores[..12]);
        let (from, to, step) = if order[0] == 2 {
            (0, 2, 1)
        } else if order[0] == 10 {
            (11, 13, 1)
        } else {
            (order[0].min(order[1]) + 1, order[0].max(order[1]), 2)
        };
        let mut c = from;
        while c < to {
            scores[c] = tuning_score(x, rate, 434.0 + c as f32, 256, &mut last);
            c += step;
        }
    }
    let mut best = 0;
    for c in 1..13 {
        if scores[best] < scores[c] {
            best = c;
        }
    }
    434.0 + best as f32
}

// ------------------------------------------------------------------------------------------
// Stage 4: constant-Q pitch classes and key candidates
// ------------------------------------------------------------------------------------------

/// Kernel bank for one reference pitch: 72 semitones from A0 (reference/16).
pub struct Bank {
    k: Kernels,
    rate: f32,
}

pub const CQ_HOP: usize = 2048;

impl Bank {
    pub fn new(rate: f32, reference: f32) -> Bank {
        let base = reference * 0.0625;
        let mut freqs: Vec<f32> = (0..72).map(|i| (i as f32 * 0.083333336f32).exp2() * base).collect();
        freqs.push(reference * 4.0);
        Bank { k: kernels(&freqs, rate), rate }
    }

    /// Constant-Q analysis of `x`: squared weighted pitch-class energies (index 0 = A) and,
    /// when `keep_raw` is set, the magnitudes before normalisation and smoothing (the later
    /// rules re-analyse them in 30-second windows). `hop` is the frame step in samples.
    /// None when `x` is too short.
    pub fn chroma(&self, x: &[f32], hop: usize, keep_raw: bool) -> Option<ConstantQ> {
        let l0 = self.k.stride;
        let n = x.len();
        if n <= l0 {
            return None;
        }
        let frames = ((n as f32 - l0 as f32) / hop as f32) as usize;
        if frames == 0 {
            return None;
        }
        let centre = (l0 as f32 * 0.5) as usize;
        let mut c = vec![0f32; 72 * frames];
        let mut top = 0f32;
        for f in 0..frames {
            let mid = f * hop + centre;
            for b in 0..72 {
                let l = self.k.len[b];
                let h = (l as f32 * 0.5) as usize;
                let o = b * l0;
                let v = if mid >= h && mid - h + l <= n {
                    let (rc, rs) = dot2(&x[mid - h..mid - h + l], &self.k.cos[o..o + l], &self.k.sin[o..o + l]);
                    (rs * rs + rc * rc).sqrt() * self.k.norm[b]
                } else {
                    0.0
                };
                c[b * frames + f] = v;
                top = top.max(v);
            }
        }
        let raw = keep_raw.then(|| c.clone());
        if top.abs() >= 0.00000011921 {
            let inv = 1.0 / top;
            for v in c.iter_mut() {
                *v *= inv;
            }
        }
        // First-order low-pass at 1 Hz along time.
        let w = (1.0f32 * 3.14159265 / (self.rate / hop as f32)).tan();
        let b0 = w / (w + 1.0);
        let a1 = (w + -1.0) / (w + 1.0);
        let mut oct = [[0f32; 12]; 6];
        for b in 0..72 {
            let mut st = 0f32;
            let mut sum = 0f32;
            for f in 0..frames {
                let t = b0 * st;
                st = c[b * frames + f] - a1 * st;
                sum += t + st * b0;
            }
            oct[b / 12][b % 12] = sum;
        }
        let chroma = rules::fold_octaves(|o, pc| oct[o][pc]);
        Some(ConstantQ { chroma, raw, frames })
    }

    /// Best and second-best key of `x` (`short` = shorter than 30 s), or None when too short.
    /// `whole_track` selects the frame step: a whole track shorter than 30 s is analysed
    /// with an eighth of the usual step, everything else with `CQ_HOP`.
    pub fn keys(&self, x: &[f32], short: bool, whole_track: bool) -> Option<(i32, i32)> {
        let hop = if whole_track && short { CQ_HOP / 8 } else { CQ_HOP };
        let cq = self.chroma(x, hop, !short)?;
        let raw = cq.raw.as_deref().map(|data| rules::RawMatrix { data, frames: cq.frames, rate: self.rate, hop });
        Some(rules::keys(&cq.chroma, short, raw))
    }
}

/// Result of the constant-Q stage for one buffer.
pub struct ConstantQ {
    /// Squared weighted pitch-class energies, index 0 = A.
    pub chroma: [f32; 12],
    /// `raw[bin * frames + frame]`: magnitudes before normalisation and smoothing.
    pub raw: Option<Vec<f32>>,
    pub frames: usize,
}

/// Best and second-best key from pitch-class energies alone (no window vote); see
/// `key_rules` for the stages.
#[allow(dead_code)]
pub fn candidates(chroma: &[f32; 12], short: bool) -> (i32, i32) {
    rules::keys(chroma, short, None)
}

// ------------------------------------------------------------------------------------------
// Whole analysis
// ------------------------------------------------------------------------------------------

/// Beats between segment boundaries, from tempo and duration.
fn segment_step(bpm: f32, dur: f32) -> usize {
    if bpm > 75.0 && bpm < 90.0 && dur > 240.0 && dur < 501.0 {
        4
    } else if bpm >= 90.0 && bpm < 95.0 && dur > 210.0 && dur < 501.0 {
        4
    } else if bpm >= 119.0 && bpm <= 125.0 && dur > 360.0 && dur < 501.0 {
        4
    } else if bpm >= 125.0 && bpm <= 127.0 && dur > 405.0 && dur < 501.0 {
        4
    } else if bpm > 135.0 && bpm <= 140.0 && dur > 360.0 && dur < 501.0 {
        4
    } else if (bpm <= 141.0 && dur < 501.0) || (bpm >= 151.0 && dur < 180.0) {
        2
    } else if dur >= 1200.0 {
        8
    } else {
        4
    }
}

pub fn analyze(left: &[f32], right: Option<&[f32]>, sr: u32, bpm: f32, beats: &[GridBeat]) -> Trace {
    let dec = decimate(left, right, sr);
    analyze_decimated(&dec, left.len(), sr, bpm, beats)
}

pub fn analyze_decimated(dec: &[f32], total: usize, sr: u32, bpm: f32, beats: &[GridBeat]) -> Trace {
    let mut tr = Trace { key: -1, ..Default::default() };
    let fsr = sr as f32;
    let rate = fsr * 0.125;
    let n = dec.len();
    if fsr * 0.42875 > n as f32 {
        return tr;
    }
    let dur = total as f32 / fsr;
    let step = segment_step(bpm, dur);
    tr.step = step;
    let mut pos: Vec<i64> = vec![];
    if let Some(b0) = beats.first() {
        let start = if (2..=4).contains(&b0.num) { 5 - b0.num as usize } else { 0 };
        let per_ms = (sr / 1000) as i64;
        let mut k = start;
        while k < beats.len() {
            pos.push(beats[k].ms as i64 * per_ms);
            k += step;
        }
    }
    let nseg = pos.len().max(2) - 1;
    let cut = cut_beats(dec, rate);
    tr.tune = reference_pitch(&cut, rate);
    let bank = Bank::new(rate, tr.tune);
    let long = rate * 30.0 <= n as f32;
    tr.whole = bank.keys(&cut, !long, true).unwrap_or((0, 0));
    let mut segs = vec![(0i32, 0i32); nseg];
    if beats.is_empty() {
        segs[0] = tr.whole;
    } else {
        let (t1, t2, t3) = (fsr * 0.5375, fsr * 0.0046875, fsr * 0.1875);
        let mut prev = 0i64;
        let mut off = 0usize;
        let mut tiled: Vec<f32> = vec![];
        for i in 0..nseg {
            if i + 1 >= pos.len() {
                break;
            }
            let len = (pos[i + 1] - prev) / 8;
            if len <= 0 || (n as i64) < 4 * len + off as i64 {
                break;
            }
            let len = len as usize;
            let seg = &cut[off..off + len];
            let lf = len as f32;
            let data: &[f32] = if t1 <= lf {
                seg
            } else {
                let reps = ((if t2 <= lf { t1 } else { t3 }) / lf).ceil() as usize;
                tiled.clear();
                for _ in 0..reps {
                    tiled.extend_from_slice(seg);
                }
                &tiled
            };
            let short = rate * 30.0 > data.len() as f32;
            if let Some(k) = bank.keys(data, short, false) {
                segs[i] = k;
            }
            off += len;
            prev = pos[i + 1];
        }
    }
    tr.key = super::key_judge::judge(tr.whole, &segs, !beats.is_empty(), bpm);
    tr.segs = segs;
    tr
}
