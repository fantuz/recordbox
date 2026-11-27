//! Vocal detection as rekordbox 7.2.19 does it (the PVDI section: one level 0–4 per 1024 samples
//! at 22,050 Hz). Port of `analyzer::VocalDetector` and `analyzer::Librosa` (trace:
//! `checks/re-7.2.19/vocal-detector.md`); the optional local network is selected by
//! RECORDBOX_VOCAL_GRAPH (`vocal_rb_net`).
//!
//! Signal path, per 2-second block of the track (as `CBeatAnalyzeThread::analyze` feeds it):
//! 16-bit samples, mono = (L + R) / 2 → × 2⁻¹⁵ → JUCE low-pass at 11,025 Hz → JUCE Lagrange
//! resampler to 22,050 Hz → 2048-point Hann frames every 1024 samples (the first frame centred on
//! sample 0 with mirrored padding) → power spectrum → 128-band mel spectrogram and an 11-band
//! fluctogram. At the end: dB (top 80 dB), 18 MFCCs, their deltas (width 9), 11 fluctogram values
//! per frame, a fixed per-feature scale and offset → 29 features per frame → a two-layer
//! bidirectional LSTM over 360-frame windows every 180 frames → probability of voice → level
//! (> 0.2, 0.4, 0.6, 0.8).
//!
//! The float32 stages (filter, resampler, window, FFT, power) follow the original's arithmetic
//! operation by operation, fused multiply-adds included; the double stages are ordinary double
//! arithmetic (their rounding cannot reach the float32 features the network sees).

use std::f64::consts::PI;

/// Analysis rate and hop of the stored levels.
pub const RATE: u32 = 22_050;
pub const HOP: usize = 1024;
pub const FEATURES: usize = 29;
const NFFT: usize = 2048;
const NFREQ: usize = NFFT / 2 + 1;
const NMEL: usize = 128;
const NMFCC: usize = 18;
const NBANDS: usize = 11;
const BAND: usize = 240;
const LOGBINS: usize = 540;

/// Seconds per stored level.
pub fn hop_seconds() -> f64 {
    HOP as f64 / RATE as f64
}

// ------------------------------------------------------------------------------------------
// JUCE low-pass (IIRCoefficients::makeLowPass, IIRFilter::processSamples), float state
// ------------------------------------------------------------------------------------------

struct Lowpass {
    c: [f32; 5],
    v1: f32,
    v2: f32,
}

impl Lowpass {
    fn new(sample_rate: f64, freq: f64) -> Lowpass {
        let n = 1.0 / ((freq * PI) / sample_rate).tan();
        let nsq = n * n;
        let inv_q_n = n * std::f64::consts::SQRT_2;
        let c1 = 1.0 / ((inv_q_n + 1.0) + nsq);
        let c2 = c1 + c1;
        let a1 = (-nsq).mul_add(c2, c2);
        let a2 = c1 * ((1.0 - inv_q_n) + nsq);
        Lowpass { c: [c1 as f32, c2 as f32, c1 as f32, a1 as f32, a2 as f32], v1: 0.0, v2: 0.0 }
    }

    fn process(&mut self, x: &mut [f32]) {
        let [c0, c1, c2, c3, c4] = self.c;
        let (mut v1, mut v2) = (self.v1, self.v2);
        for s in x.iter_mut() {
            let inp = *s;
            let out = inp.mul_add(c0, v1);
            *s = out;
            let t = inp.mul_add(c1, v2);
            v1 = (-out).mul_add(c3, t);
            v2 = inp.mul_add(c2, -(out * c4));
        }
        self.v1 = v1;
        self.v2 = v2;
    }
}

// ------------------------------------------------------------------------------------------
// JUCE LagrangeInterpolator (5 taps), as compiled in rekordbox
// ------------------------------------------------------------------------------------------

struct Lagrange {
    buf: [f32; 5],
    index: usize,
    pos: f64,
}

#[inline]
fn wrap5(i: usize) -> usize {
    if i + 1 == 5 {
        0
    } else {
        i + 1
    }
}

impl Lagrange {
    fn new() -> Lagrange {
        Lagrange { buf: [0.0; 5], index: 0, pos: 1.0 }
    }

    /// `LagrangeTraits::valueAtOffset(buf, x, i)`, lane by lane as the original's NEON code.
    fn value(&self, x: f32, i: usize) -> f32 {
        let b = &self.buf;
        let (a0, a1) = (x + 2.0, x + -1.0);
        let (b0, b1) = (2.0 - x, -1.0 - x);
        let v2 = (a0, b1);
        let v4 = (x * f32::from_bits(0x3EAAAAAB), x * 0.25);
        let i1 = wrap5(i);
        let s6 = x * 0.5;
        let i2 = wrap5(i1);
        let mut s16 = 1.0 - x;
        s16 = x.mul_add(s16, s16);
        s16 = s6.mul_add(s16, s16);
        s16 = (-s6).mul_add(s16, s16);
        let s7 = s16 * b[i2];
        let i3 = wrap5(i2);
        let v1 = (b0, a1);
        let i4 = wrap5(i3);
        let d3 = (f32::from_bits(0x3F2AAAAB), 0.5f32);
        let mut v16 = (d3.0 - v4.0, d3.1 - v4.1);
        let v6 = (s6, v4.0);
        let v17 = (v6.0 + -0.5, v6.1 + f32::from_bits(0xBEAAAAAB));
        v16 = (v17.0 * v16.0, v17.1 * v16.1);
        v16 = (v16.0 * b[i1], v16.1 * b[i]);
        let v2 = (v2.0 * v16.0, v2.1 * v16.1);
        let mut v3 = (v4.0 + d3.0, v4.1 + d3.1);
        let v4b = (v6.0 + 0.5, v6.1 + f32::from_bits(0x3EAAAAAB));
        v3 = (v3.0 * v4b.0, v3.1 * v4b.1);
        let mut v1 = (v1.0 * v3.0, v1.1 * v3.1);
        v1 = (v1.0 * b[i3], v1.1 * b[i4]);
        let v1 = (v1.0 + v2.0, v1.1 + v2.1);
        let s1 = v1.0 + 0.5 * v1.1;
        s1.mul_add(x, s7)
    }

    /// `VocalDetector::setData`'s resampling loop: `n_out` samples at `ratio` input samples per
    /// output; input left over at the end of the block is dropped, as in the original.
    fn run(&mut self, input: &[f32], n_out: usize, ratio: f64, out: &mut Vec<f32>) {
        let mut k = 0usize;
        let mut pos = self.pos;
        for _ in 0..n_out {
            let mut idx = self.index;
            if pos >= 1.0 {
                while pos >= 1.0 {
                    self.buf[idx] = input.get(k).copied().unwrap_or(0.0);
                    k += 1;
                    idx = wrap5(idx);
                    pos += -1.0;
                }
                self.index = idx;
            }
            out.push(self.value(pos as f32, idx));
            pos += ratio;
        }
        self.pos = pos;
    }
}

// ------------------------------------------------------------------------------------------
// BeatAnalyzer_1_0 FFT helpers (Okumura's tables, float), Hann window
// ------------------------------------------------------------------------------------------

struct Fft {
    sin: Vec<f32>,
    rev: Vec<usize>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        // make_sintbl: recurrence in float.
        let (n2, n4, n8) = (n / 2, n / 4, n / 8);
        let mut s = vec![0f32; n + n4];
        let t = (PI / n as f64).sin() as f32;
        s[n4] = 1.0;
        s[0] = 0.0;
        if n >= 16 {
            let t2 = t * t;
            let t4 = t2 * 4.0;
            let mut dc = t2 + t2;
            let dcd = dc as f64;
            let mut ds = ((2.0 - dcd) * dcd).sqrt() as f32;
            let (mut c, mut sv) = (1f32, 0f32);
            for i in 1..n8.max(2) {
                c -= dc;
                dc = c.mul_add(t4, dc);
                sv += ds;
                ds = (-sv).mul_add(t4, ds);
                s[i] = sv;
                s[n4 - i] = c;
            }
        }
        if n + 7 >= 15 {
            s[n8] = f32::from_bits(0x3F3504F3);
        }
        if n >= 4 {
            for i in 0..n4 {
                s[n2 - i] = s[i];
            }
        }
        for i in 0..n2 + n4 {
            s[i + n2] = -s[i];
        }
        // make_bitrev.
        let mut rev = vec![0usize; n];
        let (mut i, mut j) = (0usize, 0usize);
        loop {
            rev[i] = j;
            i += 1;
            if i >= n {
                break;
            }
            let mut k = n2;
            while k <= j {
                j -= k;
                k /= 2;
            }
            j += k;
        }
        Fft { sin: s, rev }
    }

    /// fft_less_normalize with a positive size: forward transform, not normalised.
    fn forward(&self, x: &mut [f32], y: &mut [f32]) {
        let n = x.len();
        let n4 = n / 4;
        for i in 0..n {
            let j = self.rev[i];
            if i < j {
                x.swap(i, j);
                y.swap(i, j);
            }
        }
        let mut k = 1;
        while k < n {
            let k2 = 2 * k;
            let d = n / k2;
            let mut h = 0;
            for j in 0..k {
                let c = self.sin[h + n4];
                let s = self.sin[h];
                let mut i = j;
                while i < n {
                    let ik = i + k;
                    let (xr, yi) = (x[ik], y[ik]);
                    let dx = xr.mul_add(c, yi * s);
                    let dy = yi.mul_add(c, -(xr * s));
                    let xi = x[i];
                    x[ik] = xi - dx;
                    x[i] = dx + xi;
                    let yv = y[i];
                    y[ik] = yv - dy;
                    y[i] = yv + dy;
                    i += k2;
                }
                h += d;
            }
            k = k2;
        }
    }
}

/// BeatAnalyzer_1_0::hanning: w[i-1] = 0.5 − 0.5·cos(i·2π/(n+1)), i = 1 … n.
fn hanning(n: usize) -> Vec<f32> {
    let step = 2.0 * PI / (n + 1) as f64;
    (1..=n).map(|i| (0.5f64).mul_add(-(step * i as f64).cos(), 0.5) as f32).collect()
}

// ------------------------------------------------------------------------------------------
// Librosa: mel filter bank, MFCC table, fluctogram helpers
// ------------------------------------------------------------------------------------------

/// librosa.filters.mel(sr, 2048, n_mels=128) (Slaney mel scale and area normalisation).
fn mel_filter(sr: f64) -> Vec<f64> {
    let half = sr * 0.5;
    let max_mel = if half >= 1000.0 { (sr * 0.0005).ln().mul_add(f64::from_bits(0x402D171487A833FB), 15.0) } else { sr * 0.0075 };
    let mut mel_f = [0f64; NMEL + 2];
    for (i, m) in mel_f.iter_mut().enumerate() {
        let v = max_mel * i as f64;
        *m = if v * f64::from_bits(0x3F7FC07F01FC07F0) >= f64::from_bits(0x402DFFFFFFFFFFFF) {
            v.mul_add(f64::from_bits(0x3F4176C9D7E0D82C), f64::from_bits(0xBFF0801BF4D7903E)).exp() * 1000.0
        } else {
            v * f64::from_bits(0x3FE089977BB34423)
        };
    }
    let fdiff: Vec<f64> = (0..NMEL + 1).map(|i| mel_f[i + 1] - mel_f[i]).collect();
    let inv = 1.0 / (NFREQ - 1) as f64;
    let fft_f: Vec<f64> = (0..NFREQ).map(|k| (half * k as f64) * inv).collect();
    let mut w = vec![0f64; NMEL * NFREQ];
    for i in 0..NMEL {
        let (r0, r1) = (1.0 / fdiff[i], 1.0 / fdiff[i + 1]);
        let e = 1.0 / (mel_f[i + 2] - mel_f[i]);
        let enorm = e + e;
        for k in 0..NFREQ {
            let lower = -(mel_f[i] - fft_f[k]) * r0;
            let upper = (mel_f[i + 2] - fft_f[k]) * r1;
            w[i * NFREQ + k] = lower.min(upper).max(0.0) * enorm;
        }
    }
    w
}

/// scipy.signal.windows.chebwin(240, at=200), the window rekordbox's table holds (its stored
/// values are this to 9 significant digits).
fn chebwin(m: usize, at: f64) -> Vec<f64> {
    let order = (m - 1) as f64;
    let beta = ((10f64).powf(at.abs() / 20.0).acosh() / order).cosh();
    let p: Vec<f64> = (0..m)
        .map(|k| {
            let x = beta * (PI * k as f64 / m as f64).cos();
            if x > 1.0 {
                (order * x.acosh()).cosh()
            } else if x < -1.0 {
                (2.0 * (m % 2) as f64 - 1.0) * (order * (-x).acosh()).cosh()
            } else {
                (order * x.acos()).cos()
            }
        })
        .collect();
    // Even length: w = real(FFT(p · e^{iπk/M})), then mirrored.
    let mut w = vec![0f64; m];
    for (j, wj) in w.iter_mut().enumerate() {
        let mut re = 0.0;
        for (k, pk) in p.iter().enumerate() {
            let ph = PI * k as f64 / m as f64 - 2.0 * PI * ((j * k) % m) as f64 / m as f64;
            re += pk * ph.cos();
        }
        *wj = re;
    }
    let n = m / 2 + 1;
    let mut out: Vec<f64> = (1..n).rev().map(|i| w[i]).collect();
    out.extend((1..n).map(|i| w[i]));
    let mx = out.iter().cloned().fold(f64::MIN, f64::max);
    out.iter().map(|v| v / mx).collect()
}

/// Fixed per-feature scale and offset of `Librosa::normalize_feature` (feature × scale + offset).
const NORM_SCALE: [f64; FEATURES] = [
    0.10572829740825349, 0.1875075066286435, 0.25667916996250206, 0.3555918126450177, 0.4144237895765861, 0.4808798100063106,
    0.5177738534776936, 0.577647160971935, 0.6040954360118614, 0.6378214631336069, 0.6651743303186147, 0.7100871263412366,
    0.7241019191988282, 0.7525955874101505, 0.7648052062554213, 0.7810306158221775, 0.7793004945768245, 0.7816319004613661,
    0.8481736653900437, 0.872452161594019, 0.9244892183201443, 1.0086664215472774, 1.1080837892627189, 1.2062746983990575,
    1.3054192404149232, 1.4187995077673738, 1.5780282417189575, 1.730178790620597, 1.7534915304167218,
];
const NORM_OFFSET: [f64; FEATURES] = [
    -0.012376756435658203, -0.003555513590542205, 0.0002818208946603292, -0.0022995482458490536, 0.00018162951425773039,
    -0.001456296416623111, 0.0008181396436186384, -0.0014311901589672855, 0.0008311990742261606, -0.0015107375393636298,
    0.000636778038157313, -0.0012176503033786892, 0.000496828049819892, -0.001012128175728541, 0.00040085735275465395,
    -0.0008767849693219764, 0.0004905151103014907, -0.0007594257381692587, -0.033947404564410956, -0.03562538003470876,
    -0.03144298770213009, -0.023501161035571184, -0.015168880335516244, -0.009279328431169671, -0.006071792079402699,
    -0.005981871544673414, -0.006349401774227649, -0.005701406263368334, -0.0024343021520163185,
];

/// Lag (−5 … 5) of the best normalised cross-correlation between two windowed bands
/// (`Librosa::calc_fluctogram`), with the original's tie and NaN rules.
fn fluct_lag(prev: &[f64], cur: &[f64]) -> f64 {
    let norm = 1.0 / (0..BAND).map(|i| cur[i] * prev[i]).sum::<f64>();
    let up = |s: usize| (0..BAND - s).map(|i| prev[i] * cur[i + s]).sum::<f64>() * norm;
    let down = |s: usize| (0..BAND - s).map(|i| cur[i] * prev[i + s]).sum::<f64>() * norm;
    let c = [up(5), up(4), up(3), up(2), up(1), 1.0, down(1), down(2), down(3), down(4), down(5)];
    let mut best = if c[0] >= c[1] { 0 } else { 1 };
    let mut m = c[0].max(c[1]);
    for (k, v) in c.iter().enumerate().take(7).skip(2) {
        if m < *v {
            best = k;
        }
        m = m.max(*v);
    }
    for (k, v) in c.iter().enumerate().skip(7) {
        if !(m >= *v) {
            best = k;
        }
        m = m.max(*v);
    }
    best as f64 - 5.0
}

// ------------------------------------------------------------------------------------------
// The detector's state across blocks (VocalDetector + Librosa)
// ------------------------------------------------------------------------------------------

pub struct Frontend {
    sr: u32,
    ratio: f64,
    lp: Lowpass,
    lag: Lagrange,
    /// Samples carried to the next block (2048 slots, unused ones zero) and their count.
    hist: Vec<f32>,
    hist_len: i64,
    fft: Fft,
    window: Vec<f32>,
    mel_w: Vec<f64>,
    ff: Vec<f64>,
    targets: Vec<f64>,
    tri: Vec<f64>,
    cheb: Vec<f64>,
    /// Power mel spectrogram (frames × 128) and fluctogram rows (frames − 1 × 11).
    mel: Vec<f64>,
    fluct: Vec<f64>,
    /// Fluctogram state: last frame's windowed band values and flags.
    prev: Option<(Vec<f64>, [bool; NBANDS])>,
    /// Non-zero span of each mel band's weights.
    mel_span: Vec<(usize, usize)>,
    // Scratch.
    out: Vec<f32>,
    power: Vec<f64>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Seconds spent in: filter + resampler, frames + FFT, mel, fluctogram (for profiling).
    pub timing: [f64; 4],
}

/// Power spectrum of one windowed 2048-sample frame (`get(i)` = sample i), appended to `power`.
fn power_frame(fft: &Fft, window: &[f32], re: &mut [f32], im: &mut [f32], power: &mut Vec<f64>, get: impl Fn(usize) -> f32) {
    for (i, r) in re.iter_mut().enumerate() {
        *r = window[i] * get(i);
    }
    im.fill(0.0);
    fft.forward(re, im);
    power.extend((0..NFREQ).map(|k| im[k].mul_add(im[k], re[k] * re[k]) as f64));
}

impl Frontend {
    pub fn new(sr: u32) -> Frontend {
        let half = RATE as f64 * 0.5;
        let inv = 1.0 / (NFREQ - 1) as f64;
        let ff: Vec<f64> = (0..NFREQ).map(|k| if k < NFREQ - 1 { (k as f64 * half) * inv } else { (k as f64 * 0.5 * RATE as f64) * inv }).collect();
        let targets: Vec<f64> = (0..LOGBINS).map(|k| (k as f64 * f64::from_bits(0x3F81111111111111)).exp2() * f64::from_bits(0x406D229EC465C8A2)).collect();
        let tri: Vec<f64> = (0..BAND)
            .map(|j| (-(j as f64 + f64::from_bits(0xC05DE00000000000)).abs()).mul_add(f64::from_bits(0x3F812358E75D3033), f64::from_bits(0x3FEFFFFFFFFFFFFF)))
            .collect();
        let mel_w = mel_filter(RATE as f64);
        let mel_span = (0..NMEL)
            .map(|m| {
                let w = &mel_w[m * NFREQ..(m + 1) * NFREQ];
                let lo = w.iter().position(|v| *v != 0.0).unwrap_or(0);
                let hi = w.iter().rposition(|v| *v != 0.0).map(|i| i + 1).unwrap_or(0);
                (lo, hi.max(lo))
            })
            .collect();
        Frontend {
            sr,
            ratio: sr as f64 * f64::from_bits(0x3F07C6F8C751F177),
            lp: Lowpass::new(sr as f64, 11025.0),
            lag: Lagrange::new(),
            hist: vec![0.0; NFFT],
            hist_len: 0,
            fft: Fft::new(NFFT),
            window: hanning(NFFT),
            mel_w,
            mel_span,
            ff,
            targets,
            tri,
            cheb: chebwin(BAND, 200.0),
            mel: Vec::new(),
            fluct: Vec::new(),
            prev: None,
            out: Vec::new(),
            power: Vec::new(),
            re: vec![0.0; NFFT],
            im: vec![0.0; NFFT],
            timing: [0.0; 4],
        }
    }

    /// `VocalDetector::setData`: one block of mono samples in 16-bit scale.
    pub fn push(&mut self, mono: &[f32]) {
        let n = mono.len();
        let n_out = ((n as f64) * 22050.0 / self.sr as f64) as i64;
        let t0 = std::time::Instant::now();
        let mut x: Vec<f32> = mono.iter().map(|v| v * f32::from_bits(0x38000000)).collect();
        self.lp.process(&mut x);
        let mut out = std::mem::take(&mut self.out);
        out.clear();
        if n_out > 0 {
            self.lag.run(&x, n_out as usize, self.ratio, &mut out);
        }
        let n_out = out.len() as i64;
        let t1 = std::time::Instant::now();
        self.timing[0] += (t1 - t0).as_secs_f64();
        if self.hist_len <= 0 && n_out > 1023 {
            for i in 0..HOP {
                self.hist[i] = out[HOP - 1 - i];
            }
            self.hist_len = HOP as i64;
        }
        let hl = self.hist_len;
        let total = hl + n_out;
        self.power.clear();
        let mut p0 = 0i64;
        while p0 <= total - NFFT as i64 {
            let (hist, outr) = (&self.hist, &out);
            power_frame(&self.fft, &self.window, &mut self.re, &mut self.im, &mut self.power, |i| {
                let j = p0 + i as i64;
                if j < hl {
                    hist[j as usize]
                } else {
                    outr[(j - hl) as usize]
                }
            });
            p0 += HOP as i64;
        }
        // Carry the tail, as the original does (its short-block case keeps the wrong samples;
        // reproduced).
        let keep = if total <= 0 { -((-total) & 0x3FF) } else { total & 0x3FF } + HOP as i64;
        if keep <= n_out {
            let k = keep as usize;
            self.hist[..k].copy_from_slice(&out[(n_out - keep) as usize..]);
            for v in &mut self.hist[k..] {
                *v = 0.0;
            }
        } else {
            let moved = (keep - n_out) as usize;
            self.hist.copy_within(n_out as usize..n_out as usize + moved, 0);
            self.hist[moved..moved + n_out as usize].copy_from_slice(&out);
            for v in &mut self.hist[keep as usize..] {
                *v = 0.0;
            }
        }
        self.hist_len = keep;
        self.out = out;
        self.timing[1] += t1.elapsed().as_secs_f64();
        self.analyse_power();
    }

    /// Mel spectrogram and fluctogram of the frames in `self.power`.
    fn analyse_power(&mut self) {
        let nf = self.power.len() / NFREQ;
        if nf == 0 {
            return;
        }
        let t0 = std::time::Instant::now();
        for f in 0..nf {
            let p = &self.power[f * NFREQ..(f + 1) * NFREQ];
            for m in 0..NMEL {
                // Weights outside the span are exactly zero.
                let (lo, hi) = self.mel_span[m];
                let w = &self.mel_w[m * NFREQ + lo..m * NFREQ + hi];
                self.mel.push(p[lo..hi].iter().zip(w).map(|(a, b)| a * b).sum());
            }
        }
        let t1 = std::time::Instant::now();
        self.fluctogram(nf);
        self.timing[2] += (t1 - t0).as_secs_f64();
        self.timing[3] += t1.elapsed().as_secs_f64();
    }

    /// `Librosa::get_fluctogram` for `nf` new frames.
    fn fluctogram(&mut self, nf: usize) {
        // Magnitudes interpolated onto 540 log-frequency bins (10 cents from 233.08 Hz).
        let mut idx = vec![0usize; LOGBINS];
        let mut i = 0usize;
        for (j, f) in self.targets.iter().enumerate() {
            while i < NFREQ - 1 && *f > self.ff[i + 1] {
                i += 1;
            }
            idx[j] = i;
        }
        let mut logspec = vec![0f64; nf * LOGBINS];
        for fr in 0..nf {
            let p = &self.power[fr * NFREQ..(fr + 1) * NFREQ];
            for j in 0..LOGBINS {
                let k = idx[j];
                let (m0, m1) = (p[k].sqrt(), p[k + 1].sqrt());
                let f0 = self.ff[k];
                logspec[fr * LOGBINS + j] = (self.targets[j] - f0) * (m1 - m0) / (self.ff[k + 1] - f0) + m0;
            }
        }
        let had_state = self.prev.is_some();
        let rows = if had_state { nf } else { nf - 1 };
        let base = self.fluct.len();
        self.fluct.resize(base + rows * NBANDS, 0.0);
        let off = had_state as usize;
        let eps = f64::EPSILON;
        let mut last = vec![0f64; NBANDS * BAND];
        let mut last_flags = [false; NBANDS];
        for b in 0..NBANDS {
            let mut win = vec![0f64; nf * BAND];
            let mut flags = vec![false; nf];
            for fr in 0..nf {
                let x = &logspec[fr * LOGBINS + 30 * b..fr * LOGBINS + 30 * b + BAND];
                let num: f64 = x.iter().zip(&self.cheb).map(|(a, c)| a * c).sum();
                let den: f64 = x.iter().sum::<f64>() + eps;
                let ratio = num / den;
                flags[fr] = if ratio <= 0.1 {
                    false
                } else {
                    let lg: f64 = x.iter().map(|v| (v + eps).ln()).sum();
                    let ar: f64 = x.iter().sum();
                    (lg * f64::from_bits(0x3F71111111111111)).exp() / (ar * f64::from_bits(0x3F71111111111111) + eps) < 0.6
                };
                for j in 0..BAND {
                    win[fr * BAND + j] = x[j] * self.tri[j];
                }
            }
            if let Some((pw, pf)) = &self.prev {
                if pf[b] {
                    self.fluct[base + b] = fluct_lag(&pw[b * BAND..(b + 1) * BAND], &win[..BAND]);
                }
            }
            for fr in 0..nf.saturating_sub(1) {
                if flags[fr] {
                    self.fluct[base + (fr + off) * NBANDS + b] = fluct_lag(&win[fr * BAND..(fr + 1) * BAND], &win[(fr + 1) * BAND..(fr + 2) * BAND]);
                }
            }
            last[b * BAND..(b + 1) * BAND].copy_from_slice(&win[(nf - 1) * BAND..nf * BAND]);
            last_flags[b] = flags[nf - 1];
        }
        self.prev = Some((last, last_flags));
    }

    /// `VocalDetector::applyEndProcess` up to the network: the last frame(s) with mirrored
    /// padding, then the 29 normalised features per frame (frames × 29).
    pub fn finish(mut self) -> Option<(usize, Vec<f64>)> {
        let hl = self.hist_len;
        self.power.clear();
        let mut p0 = 0i64;
        while p0 < hl - HOP as i64 {
            let hist = &self.hist;
            power_frame(&self.fft, &self.window, &mut self.re, &mut self.im, &mut self.power, |i| {
                let j = p0 + i as i64;
                if j < hl {
                    hist[j as usize]
                } else {
                    hist[(2 * hl - 1 - j) as usize]
                }
            });
            p0 += HOP as i64;
        }
        self.analyse_power();
        let n = self.mel.len() / NMEL;
        if n < 9 {
            return None;
        }
        // power_to_db: reference = the track's maximum, floor 1e-10, top 80 dB.
        let mx = self.mel.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let r = mx.max(1e-10).log10();
        let mut top = f64::MIN;
        for v in self.mel.iter_mut() {
            *v = (v.max(1e-10).log10() - r) * 10.0;
            top = top.max(*v);
        }
        let floor = top + -80.0;
        for v in self.mel.iter_mut() {
            if *v < floor {
                *v = floor;
            }
        }
        // MFCC: DCT-II, orthonormal, coefficients 0 … 17.
        let cos: Vec<f64> = (1..NMFCC)
            .flat_map(|k| {
                let a = k as f64 * PI / 256.0;
                (0..NMEL).map(move |m| ((2 * m + 1) as f64 * a).cos())
            })
            .collect();
        let mut mfcc = vec![0f64; n * NMFCC];
        for f in 0..n {
            let x = &self.mel[f * NMEL..(f + 1) * NMEL];
            mfcc[f * NMFCC] = x.iter().sum::<f64>() * f64::from_bits(0x3FB6A09E667F3BCD);
            for k in 1..NMFCC {
                let c = &cos[(k - 1) * NMEL..k * NMEL];
                mfcc[f * NMFCC + k] = x.iter().zip(c).map(|(a, b)| a * b).sum::<f64>() * 0.125;
            }
        }
        // Deltas, width 9; the first and last four frames copy the nearest computed one.
        let mut delta = vec![0f64; n * NMFCC];
        let at = |t: usize, k: usize| mfcc[t * NMFCC + k];
        for t in 4..n - 4 {
            for k in 0..NMFCC {
                let d1 = at(t + 1, k) - at(t - 1, k);
                let d2 = at(t + 2, k) - at(t - 2, k);
                let d3 = at(t + 3, k) - at(t - 3, k);
                let d4 = at(t + 4, k) - at(t - 4, k);
                delta[t * NMFCC + k] = d4.mul_add(4.0, d3.mul_add(3.0, d1 + (d2 + d2))) * f64::from_bits(0x3F91111111111111);
            }
        }
        for t in (0..4).chain(n - 4..n) {
            let src = if t < 4 { 4 } else { n - 5 };
            for k in 0..NMFCC {
                delta[t * NMFCC + k] = delta[src * NMFCC + k];
            }
        }
        let mut feat = vec![0f64; n * FEATURES];
        for t in 0..n {
            feat[t * FEATURES..t * FEATURES + NMFCC].copy_from_slice(&delta[t * NMFCC..(t + 1) * NMFCC]);
            if t + 1 < n {
                feat[t * FEATURES + NMFCC..(t + 1) * FEATURES].copy_from_slice(&self.fluct[t * NBANDS..(t + 1) * NBANDS]);
            }
        }
        for t in 0..n {
            for k in 0..FEATURES {
                let v = &mut feat[t * FEATURES + k];
                *v = v.mul_add(NORM_SCALE[k], NORM_OFFSET[k]);
            }
        }
        Some((n, feat))
    }
}

/// Levels from voice probabilities (`applyEndProcess`, float comparisons).
pub fn levels(prob: &[f32]) -> Vec<u8> {
    prob.iter()
        .map(|p| {
            let mut l = (*p > 0.2f32) as u8;
            if *p > 0.4f32 {
                l = 2;
            }
            if *p > 0.6f32 {
                l = 3;
            }
            if *p > 0.8f32 {
                l = 4;
            }
            l
        })
        .collect()
}

/// rekordbox's reader hands the analysers 16-bit samples: float × 32768, clamped, truncated
/// (`PcmReaderTemplate<Float2>::getStreamAt`). Measured on the local tracks, truncating our
/// decoder's floats also matches rekordbox's MP3 path better than rounding (its bundled mpg123
/// gives the same 16-bit values as truncating its float output); `round` is for comparisons.
pub fn to_s16(x: f32, round: bool) -> f32 {
    let v = (x * 32768.0).clamp(-32768.0, 32767.0);
    if round {
        v.round()
    } else {
        v.trunc()
    }
}

/// Features of a decoded track (interleaved stereo at `sr`), fed in 2-second blocks as
/// rekordbox does. None for tracks rekordbox would not analyse (≤ 0.418 s or > 2 h).
pub fn features(samples: &[f32], sr: u32, round: bool) -> Option<(usize, Vec<f64>)> {
    let frames = samples.len() / 2;
    let dur = frames as f64 / sr as f64;
    if !(dur > 0.417959184 && dur <= 7200.0) {
        return None;
    }
    let mut fe = Frontend::new(sr);
    let chunk = 2 * sr as usize;
    let mut mono = Vec::with_capacity(chunk);
    for block in samples.chunks(2 * chunk) {
        mono.clear();
        mono.extend(block.chunks_exact(2).map(|c| (to_s16(c[1], round) + to_s16(c[0], round)) * 0.5));
        fe.push(&mono);
    }
    fe.finish()
}

/// The stem-based detector (`stems::vocal_levels`) is used instead of this one: with
/// PREPDECK_VOCAL=stems, or when no compatible local graph has been configured.
pub fn use_stems() -> bool {
    std::env::var("PREPDECK_VOCAL").map(|v| v == "stems").unwrap_or(false) || crate::vocal_rb_net::configured().is_none()
}

/// rekordbox's vocal detection of a decoded track: one level 0–4 per 1024 samples at
/// 22,050 Hz (the PVDI frame; the same 46.4 ms as `stems::VOCAL_HOP` at 44.1 kHz). None when
/// the stem-based detector is in use, or for tracks rekordbox does not analyse (≤ 0.418 s, > 2 h).
pub fn analyse(dec: &crate::audio::Decoded) -> Option<Vec<u8>> {
    if use_stems() {
        return None;
    }
    let model = crate::vocal_rb_net::configured()?;
    let (_, feat) = features(&dec.samples, dec.sr, ROUND_S16)?;
    Some(levels(&model.run(&feat)?))
}

/// Our decoder's floats are made 16-bit the way rekordbox's reader does it for float sources
/// (truncation); see `vocal_benchmark` for the measured effect of rounding MP3s instead.
pub const ROUND_S16: bool = false;

/// Features of 16-bit interleaved stereo, in rekordbox's 2-second blocks (None for tracks
/// rekordbox would not analyse).
pub fn features_s16(pcm: &[i16], sr: u32) -> Option<(usize, Vec<f64>)> {
    features_s16_timed(pcm, sr).map(|(f, _)| f)
}

/// The same, with the time spent per stage (filter + resampler, frames + FFT, mel,
/// fluctogram, end: seconds).
pub fn features_s16_timed(pcm: &[i16], sr: u32) -> Option<((usize, Vec<f64>), [f64; 5])> {
    let dur = (pcm.len() / 2) as f64 / sr as f64;
    if !(dur > 0.417959184 && dur <= 7200.0) {
        return None;
    }
    let mut fe = Frontend::new(sr);
    let chunk = 2 * sr as usize;
    let mut mono = Vec::with_capacity(chunk);
    for block in pcm.chunks(2 * chunk) {
        mono.clear();
        mono.extend(block.chunks_exact(2).map(|c| (c[1] as f32 + c[0] as f32) * 0.5));
        fe.push(&mono);
    }
    let tm = fe.timing;
    let t1 = std::time::Instant::now();
    let r = fe.finish()?;
    Some((r, [tm[0], tm[1], tm[2], tm[3], t1.elapsed().as_secs_f64()]))
}

/// Analyse the same PCM used by the deck so its cue and vocal timelines agree.
pub fn analyse_file(path: &str, dec: Option<&crate::audio::Decoded>) -> Option<Vec<u8>> {
    if use_stems() {
        return None;
    }
    match dec {
        Some(dec) => analyse(dec),
        None => analyse(&crate::audio::decode_file(path).ok()?),
    }
}

#[cfg(test)]
mod oracle {
    /// Compare this port with rekordbox's own detector run by the oracle harness
    /// (`checks/vocal-oracle-harness`) on the same 16-bit PCM.
    /// PREPDECK_VOCAL_PCM (s16le stereo), PREPDECK_VOCAL_SR, PREPDECK_VOCAL_ORACLE (harness
    /// output folder); run with --ignored.
    #[test]
    #[ignore]
    fn compare_with_oracle() {
        let pcm: Vec<i16> = std::fs::read(std::env::var("PREPDECK_VOCAL_PCM").unwrap()).unwrap().chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        let sr: u32 = std::env::var("PREPDECK_VOCAL_SR").ok().and_then(|v| v.parse().ok()).unwrap_or(44100);
        let dir = std::path::PathBuf::from(std::env::var("PREPDECK_VOCAL_ORACLE").unwrap());
        let t0 = std::time::Instant::now();
        let ((n, feat), tm) = super::features_s16_timed(&pcm, sr).unwrap();
        let t1 = t0.elapsed();
        eprintln!("timing: resample {:.3}s, frames+fft {:.3}s, mel {:.3}s, fluctogram {:.3}s, end {:.3}s", tm[0], tm[1], tm[2], tm[3], tm[4]);
        let rd = |f: &str| std::fs::read(dir.join(f)).unwrap_or_default();
        let ofeat: Vec<f64> = rd("feat.f64").chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();
        eprintln!("frames {n} (oracle {})", ofeat.len() / 29);
        let mut worst = vec![0f64; 29];
        let mut f32_same = 0usize;
        for (i, (a, b)) in feat.iter().zip(&ofeat).enumerate() {
            let d = (a - b).abs();
            let k = i % 29;
            worst[k] = worst[k].max(d);
            f32_same += (*a as f32 == *b as f32) as usize;
        }
        eprintln!("features: float32-identical {f32_same}/{}; max abs diff per feature {:?}", ofeat.len().min(feat.len()), worst.iter().map(|v| format!("{v:.2e}")).collect::<Vec<_>>());
        // Fluctogram columns: count differing values.
        let fl_diff = feat.iter().zip(&ofeat).enumerate().filter(|(i, (a, b))| i % 29 >= 18 && a != b).count();
        eprintln!("fluctogram values differing: {fl_diff}");
        let Some(model) = crate::vocal_rb_net::configured() else { return };
        let t2 = std::time::Instant::now();
        let prob = model.run(&feat).unwrap();
        let t3 = t2.elapsed();
        let oprob: Vec<f32> = rd("prob.f32").chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let pd = prob.iter().zip(&oprob).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        let lv = super::levels(&prob);
        let ol = rd("levels.u8");
        let same = lv.iter().zip(&ol).filter(|(a, b)| a == b).count();
        eprintln!("probabilities: max abs diff {pd:.3e}; levels identical {same}/{} (oracle {}); features {:.2}s, network {:.2}s", lv.len(), ol.len(), t1.as_secs_f32(), t3.as_secs_f32());
        // The same network on rekordbox's own features isolates the network port.
        let p2 = model.run(&ofeat).unwrap();
        let pd2 = p2.iter().zip(&oprob).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        let same2 = super::levels(&p2).iter().zip(&ol).filter(|(a, b)| a == b).count();
        eprintln!("network on the oracle's features: max abs diff {pd2:.3e}; levels identical {same2}/{}", ol.len());
    }
}

#[cfg(test)]
mod bench {
    /// Agreement of two level sequences: (frames compared, exact, vocal/no-vocal agree, tp, fp, fn).
    pub fn agree(got: &[u8], want: &[u8]) -> (usize, usize, usize, usize, usize, usize) {
        let n = got.len().min(want.len());
        let mut r = (n, 0, 0, 0, 0, 0);
        for i in 0..n {
            let (a, b) = (got[i] > 0, want[i] > 0);
            r.1 += (got[i] == want[i]) as usize;
            r.2 += (a == b) as usize;
            r.3 += (a && b) as usize;
            r.4 += (a && !b) as usize;
            r.5 += (!a && b) as usize;
        }
        r
    }
    fn f1(tp: usize, fp: usize, fn_: usize) -> f64 {
        if tp + fp + fn_ == 0 {
            1.0
        } else {
            2.0 * tp as f64 / (2 * tp + fp + fn_) as f64
        }
    }

    /// Vocal levels against rekordbox's stored PVDI for every listed track (audio path<TAB>ANLZ
    /// base path, read only), with this application's decoder. Prints per track and overall:
    /// exact-level agreement, vocal/no-vocal agreement, F1, frame counts, time.
    /// PREPDECK_VOCAL_PAIRS; optional PREPDECK_VOCAL_ROUND (auto (default): MP3s through
    /// rekordbox's mpg123 when available, else truncation; 0: this application's decoder,
    /// truncated; 1: this application's decoder, rounded),
    /// PREPDECK_VOCAL_HARNESS + PREPDECK_RBSLICE + PREPDECK_VOCAL_FIXUPS + PREPDECK_VOCAL_TMP
    /// (also run rekordbox's own detector on the same PCM), PREPDECK_VOCAL_OLD=1 (also the
    /// stem-based detector, from cached stems only; =sep separates when not cached, GPU),
    /// PREPDECK_VOCAL_N. Run with --ignored.
    #[test]
    #[ignore]
    fn vocal_benchmark() {
        let pairs = std::fs::read_to_string(std::env::var("PREPDECK_VOCAL_PAIRS").unwrap()).unwrap();
        let limit: usize = std::env::var("PREPDECK_VOCAL_N").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
        let round_mode = std::env::var("PREPDECK_VOCAL_ROUND").unwrap_or_else(|_| "auto".into());
        let harness = std::env::var("PREPDECK_VOCAL_HARNESS").ok();
        let old_mode = std::env::var("PREPDECK_VOCAL_OLD").unwrap_or_default();
        let old = old_mode == "1" || old_mode == "sep";
        let model = crate::vocal_rb_net::configured().expect("rekordbox's network");
        let mut tot = [0usize; 6];
        let mut tot_h = [0usize; 6];
        let mut tot_hp = [0usize; 6];
        let mut tot_old = [0usize; 6];
        let (mut tracks, mut perfect, mut len_ok, mut secs, mut audio_secs, mut bytes_same) = (0, 0, 0, 0f64, 0f64, 0);
        let mut rows = Vec::new();
        for line in pairs.lines().take(limit) {
            let Some((path, base)) = line.split_once('\t') else { continue };
            let sec = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.2EX")));
            let Some(pv) = sec.get("PVDI").and_then(|s| crate::app::parse_pvdi(s)) else { continue };
            let Ok(dec) = crate::audio::decode_file(path) else {
                eprintln!("SKIP (decode) {path}");
                continue;
            };
            let mp3 = path.to_ascii_lowercase().ends_with(".mp3");
            let round = match round_mode.as_str() {
                "1" => true,
                _ => super::ROUND_S16,
            };
            let t0 = std::time::Instant::now();
            // MP3s through rekordbox's own mpg123 when available (as the app does), unless a
            // rounding mode is asked for.
            #[cfg(target_os = "macos")]
            let rb_pcm = if mp3 && round_mode == "auto" { crate::vocal_rb_mp3::decode(path) } else { None };
            #[cfg(not(target_os = "macos"))]
            let rb_pcm: Option<(Vec<i16>, u32)> = None;
            let feat = match &rb_pcm {
                Some((pcm, sr)) => super::features_s16(pcm, *sr).map(|f| f.1),
                None => super::features(&dec.samples, dec.sr, round).map(|f| f.1),
            };
            let Some(feat) = feat else {
                eprintln!("SKIP (short) {path}");
                continue;
            };
            let prob = model.run(&feat).unwrap();
            let lv = super::levels(&prob);
            let dt = t0.elapsed().as_secs_f64();
            if let Ok(d) = std::env::var("PREPDECK_VOCAL_DUMP") {
                // <index>.lv (our levels), <index>.rb (stored), <index>.p (probabilities).
                let _ = std::fs::create_dir_all(&d);
                let i = tracks;
                let _ = std::fs::write(format!("{d}/{i}.lv"), &lv);
                let _ = std::fs::write(format!("{d}/{i}.rb"), &pv.1);
                let _ = std::fs::write(format!("{d}/{i}.p"), prob.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
            }
            secs += dt;
            audio_secs += dec.duration();
            let want = &pv.1;
            // The section the USB export would write, against rekordbox's stored one.
            bytes_same += (sec.get("PVDI").map(|s| s.as_slice()) == Some(crate::export::anlz::pvdi(&lv).as_slice())) as usize;
            let a = agree(&lv, want);
            for k in 0..6 {
                tot[k] += [a.0, a.1, a.2, a.3, a.4, a.5][k];
            }
            tracks += 1;
            perfect += (a.1 == a.0 && lv.len() == want.len()) as usize;
            len_ok += (lv.len() == want.len()) as usize;
            let name = path.rsplit('/').next().unwrap_or("").to_string();
            let mut extra = String::new();
            if let Some(h) = &harness {
                let tmp = std::path::PathBuf::from(std::env::var("PREPDECK_VOCAL_TMP").unwrap());
                let _ = std::fs::create_dir_all(&tmp);
                let pcm = tmp.join("in.s16");
                let bytes: Vec<u8> = match &rb_pcm {
                    Some((pcm, _)) => pcm.iter().flat_map(|v| v.to_le_bytes()).collect(),
                    None => dec.samples.iter().flat_map(|v| (super::to_s16(*v, round) as i16).to_le_bytes()).collect(),
                };
                std::fs::write(&pcm, bytes).unwrap();
                let _ = std::fs::remove_file(tmp.join("levels.u8"));
                let st = std::process::Command::new(h)
                    .args([&std::env::var("PREPDECK_RBSLICE").unwrap(), &std::env::var("PREPDECK_VOCAL_FIXUPS").unwrap(), pcm.to_str().unwrap(), &dec.sr.to_string(), tmp.to_str().unwrap()])
                    .stderr(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .status();
                let _ = std::fs::remove_file(&pcm);
                let hl = std::fs::read(tmp.join("levels.u8")).unwrap_or_default();
                let b = agree(&hl, want);
                let c = agree(&lv, &hl);
                for k in 0..6 {
                    tot_h[k] += [b.0, b.1, b.2, b.3, b.4, b.5][k];
                    tot_hp[k] += [c.0, c.1, c.2, c.3, c.4, c.5][k];
                }
                extra = format!(" | rekordbox's own on this PCM: exact {:.2}% (status {:?}) | port vs it: {}/{}", 100.0 * b.1 as f64 / b.0.max(1) as f64, st.map(|s| s.success()).unwrap_or(false), c.1, c.0);
            }
            if old {
                let cp = crate::stems::cache_path(path, crate::stems::CATALOG[0].id);
                let ol = if let Ok(st) = crate::stems::load(&cp) {
                    let (l, r) = (&st.data[6 * st.len..7 * st.len], &st.data[7 * st.len..8 * st.len]);
                    let mono: Vec<f32> = l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect();
                    let mix: Vec<f32> = (0..st.len).map(|i| (0..8).map(|c| st.data[c * st.len + i]).sum::<f32>() * 0.5).collect();
                    Some(crate::stems::vocal_levels(&mono, &mix))
                } else if old_mode == "sep" {
                    // As the deck does without cached stems: the vocal stem alone (GPU model).
                    crate::stems::separate_vocals(&dec.samples, dec.sr, &mut |_| true).ok().map(|v| crate::stems::vocal_levels(&v, &crate::stems::mono_44k(&dec.samples, dec.sr)))
                } else {
                    None
                };
                if let Some(ol) = ol {
                    let o = agree(&ol, want);
                    for k in 0..6 {
                        tot_old[k] += [o.0, o.1, o.2, o.3, o.4, o.5][k];
                    }
                    extra += &format!(" | stem detector: exact {:.2}%, vocal {:.2}%, F1 {:.3}", 100.0 * o.1 as f64 / o.0.max(1) as f64, 100.0 * o.2 as f64 / o.0.max(1) as f64, f1(o.3, o.4, o.5));
                }
            }
            let line = format!(
                "{:>6.2}% exact, {:>6.2}% vocal, F1 {:.3}, frames {} vs {} stored, {:.2}s{}  {}",
                100.0 * a.1 as f64 / a.0.max(1) as f64,
                100.0 * a.2 as f64 / a.0.max(1) as f64,
                f1(a.3, a.4, a.5),
                lv.len(),
                want.len(),
                dt,
                extra,
                name
            );
            eprintln!("{line}");
            rows.push(line);
        }
        eprintln!(
            "VOCAL tracks {tracks}: exact {}/{} = {:.3}%, vocal/no-vocal {:.3}%, F1 {:.4}; {perfect} tracks identical ({bytes_same} PVDI sections byte-identical), {len_ok} same length; {:.2} s per track ({:.0}x real time)",
            tot[1],
            tot[0],
            100.0 * tot[1] as f64 / tot[0].max(1) as f64,
            100.0 * tot[2] as f64 / tot[0].max(1) as f64,
            f1(tot[3], tot[4], tot[5]),
            secs / tracks.max(1) as f64,
            audio_secs / secs.max(1e-9)
        );
        if harness.is_some() {
            eprintln!(
                "REKORDBOX-ON-OUR-PCM vs stored: exact {}/{} = {:.3}%, vocal {:.3}%, F1 {:.4}; port vs rekordbox-on-our-PCM: exact {}/{} = {:.4}%",
                tot_h[1],
                tot_h[0],
                100.0 * tot_h[1] as f64 / tot_h[0].max(1) as f64,
                100.0 * tot_h[2] as f64 / tot_h[0].max(1) as f64,
                f1(tot_h[3], tot_h[4], tot_h[5]),
                tot_hp[1],
                tot_hp[0],
                100.0 * tot_hp[1] as f64 / tot_hp[0].max(1) as f64
            );
        }
        if old {
            eprintln!(
                "STEM DETECTOR frames {}: exact {:.3}%, vocal {:.3}%, F1 {:.4}",
                tot_old[0],
                100.0 * tot_old[1] as f64 / tot_old[0].max(1) as f64,
                100.0 * tot_old[2] as f64 / tot_old[0].max(1) as f64,
                f1(tot_old[3], tot_old[4], tot_old[5])
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_matches_a_direct_dft() {
        let fft = Fft::new(64);
        let x0: Vec<f32> = (0..64).map(|i| ((i * 7 % 13) as f32 - 6.0) * 0.1).collect();
        let (mut re, mut im) = (x0.clone(), vec![0f32; 64]);
        fft.forward(&mut re, &mut im);
        for k in 0..64 {
            let (mut a, mut b) = (0f64, 0f64);
            for (n, v) in x0.iter().enumerate() {
                let ph = -2.0 * PI * (k * n) as f64 / 64.0;
                a += *v as f64 * ph.cos();
                b += *v as f64 * ph.sin();
            }
            assert!((re[k] as f64 - a).abs() < 1e-4 && (im[k] as f64 - b).abs() < 1e-4, "bin {k}");
        }
    }

    #[test]
    fn mel_bank_rows_have_slaney_area() {
        // Slaney normalisation: each triangle's area over frequency is 1 (within the grid error).
        let w = mel_filter(22050.0);
        let df = 22050.0 / 2048.0;
        for m in [5usize, 40, 100] {
            let area: f64 = w[m * NFREQ..(m + 1) * NFREQ].iter().sum::<f64>() * df;
            assert!((area - 1.0).abs() < 0.1, "band {m}: {area}");
        }
    }

    #[test]
    fn chebwin_matches_scipy() {
        let w = chebwin(240, 200.0);
        assert_eq!(w.len(), 240);
        assert!((w[119] - 1.0).abs() < 1e-12 && (w[120] - 1.0).abs() < 1e-12);
        // scipy.signal.windows.chebwin(240, 200)[0] = 1.0139202632e-08
        assert!((w[0] / 1.0139202632399871e-08 - 1.0).abs() < 1e-6, "{}", w[0]);
    }

    #[test]
    fn fluct_lag_finds_the_shift() {
        let base: Vec<f64> = (0..BAND).map(|i| (-((i as f64 - 100.0) / 6.0).powi(2)).exp()).collect();
        let shifted: Vec<f64> = (0..BAND).map(|i| (-((i as f64 - 103.0) / 6.0).powi(2)).exp()).collect();
        // The current band moved up 3 bins: lag −3 (index 2 of the list).
        assert_eq!(fluct_lag(&base, &shifted), -3.0);
        assert_eq!(fluct_lag(&base, &base), 0.0);
        assert_eq!(fluct_lag(&shifted, &base), 3.0);
    }

    #[test]
    fn levels_use_the_four_thresholds() {
        assert_eq!(levels(&[0.0, 0.2, 0.21, 0.41, 0.61, 0.81, 1.0]), vec![0, 0, 1, 2, 3, 4, 4]);
    }

    /// 20 s of a synthetic stereo signal (tones with vibrato, a slow swell, bass, noise),
    /// 16-bit, the same as the oracle run that produced the expected values below.
    fn synthetic() -> Vec<i16> {
        let sr = 44100.0f64;
        let mut s: u64 = 12345;
        let mut out = Vec::new();
        for i in 0..20 * 44100 {
            let t = i as f64 / sr;
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let noise = ((s >> 40) as f64 / (1u64 << 24) as f64 - 0.5) * 0.05;
            let f0 = 220.0 * (1.0 + 0.02 * (2.0 * PI * 5.0 * t).sin());
            let env = 0.5 + 0.5 * (2.0 * PI * 0.25 * t).sin();
            let v = 0.3 * env * ((2.0 * PI * f0 * t).sin() + 0.5 * (2.0 * PI * 2.0 * f0 * t).sin() + 0.25 * (2.0 * PI * 3.0 * f0 * t).sin()) + 0.2 * (2.0 * PI * 55.0 * t).sin() + noise;
            let q = |x: f64| (x * 16000.0).round_ties_even().clamp(-32768.0, 32767.0) as i16;
            out.push(q(v));
            out.push(q(v * 0.9 + noise));
        }
        out
    }

    #[test]
    fn features_match_rekordbox_on_a_synthetic_signal() {
        // Expected values: rekordbox's own VocalDetector on this PCM (checks/vocal-oracle-harness),
        // float32 bits of the features the network sees.
        let (n, feat) = features_s16(&synthetic(), 44100).unwrap();
        assert_eq!(n, 431);
        let bits = |fr: usize, k: usize| (feat[fr * FEATURES + k] as f32).to_bits();
        let want = [
            (3, [0x3e4b3f4d, 0xbf9a9e9d, 0xbe2db84e]),
            (100, [0x3f21afa0, 0xbec8d949, 0x3e456cd3]),
            (250, [0x3ed3aad0, 0x3ee6f68c, 0xbd47244e]),
            (400, [0xbf8d9d58, 0xbd43e6d9, 0xbdb8549a]),
        ];
        for (fr, w) in want {
            for (j, k) in [0usize, 5, 17].iter().enumerate() {
                assert_eq!(bits(fr, *k), w[j], "frame {fr} feature {k}");
            }
        }
        // Fluctogram at frame 100: bands 8–10 at lags +5, +5, −5, the rest 0.
        let fl: Vec<f64> = (NMFCC..FEATURES).map(|k| (feat[100 * FEATURES + k] - NORM_OFFSET[k]) / NORM_SCALE[k]).collect();
        let lags: Vec<i32> = fl.iter().map(|v| v.round() as i32).collect();
        assert_eq!(lags, vec![0, 0, 0, 0, 0, 0, 0, 0, 5, 5, -5]);
        let nonzero = (0..n).flat_map(|fr| (NMFCC..FEATURES).map(move |k| (fr, k))).filter(|(fr, k)| (feat[fr * FEATURES + k] - NORM_OFFSET[*k]).abs() > 1e-9).count();
        // Non-zero fluctogram values over the whole signal (oracle: 436).
        assert_eq!(nonzero, 436);
    }

    #[test]
    fn resampler_keeps_the_band_and_drops_the_top() {
        // 1 kHz passes the second-order low-pass (11,025 Hz) and the 2:1 resampler at full
        // level; 15 kHz is cut to about −11 dB, 20 kHz to below −30 dB (folded down by the 2:1).
        for (f, lo, hi) in [(1000.0, 0.95, 1.05), (15000.0, 0.2, 0.35), (20000.0, 0.0, 0.03)] {
            let mut lp = Lowpass::new(44100.0, 11025.0);
            let mut x: Vec<f32> = (0..44100).map(|i| (2.0 * PI * f * i as f64 / 44100.0).sin() as f32).collect();
            lp.process(&mut x);
            let mut lag = Lagrange::new();
            let mut y = Vec::new();
            lag.run(&x, 22050, 2.0, &mut y);
            let rms = (y[2000..].iter().map(|v| (v * v) as f64).sum::<f64>() / (y.len() - 2000) as f64).sqrt() * 2f64.sqrt();
            assert!(rms > lo && rms < hi, "{f} Hz: {rms}");
        }
    }

    #[test]
    fn analysis_gives_vocal_levels() {
        // A configured local graph produces one level per 1024 samples at 22,050 Hz.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("demo.wav");
        crate::demo::write_wav(&p, 44100, &crate::demo::synth(&crate::demo::DEMOS[0], 44100)).unwrap();
        let r = crate::analysis::analyze_file(p.to_str().unwrap(), &Default::default()).unwrap();
        // Preferences > Analysis > Vocal off: none.
        let off = crate::analysis::analyze_file(p.to_str().unwrap(), &crate::analysis::AnalysisSettings { vocal: false, ..Default::default() }).unwrap();
        assert!(off.vocal.is_none());
        if use_stems() {
            assert!(r.vocal.is_none());
            return;
        }
        let v = r.vocal.expect("vocal levels");
        let frames = (r.duration * 44100.0).round() as usize;
        // 2-second blocks of 88,200 samples give 44,100 each; the last block half its length.
        let resampled = (frames / 88200) * 44100 + (frames % 88200) / 2;
        assert_eq!(v.len(), resampled.div_ceil(HOP));
        assert!(v.iter().all(|l| *l <= 4));
    }

    #[test]
    fn frontend_frames_and_features() {
        // 10 s of a tone with a slow vibrato: ceil(n_out / 1024) frames, finite features.
        let sr = 44100u32;
        let n = 10 * sr as usize;
        let mut st = Vec::with_capacity(2 * n);
        for i in 0..n {
            let t = i as f64 / sr as f64;
            let v = (0.3 * (2.0 * PI * (440.0 * t + 3.0 * (2.0 * PI * 5.0 * t).sin()))).sin() as f32 * 0.5;
            st.push(v);
            st.push(v);
        }
        let (frames, feat) = features(&st, sr, false).unwrap();
        // 5 blocks of 2 s → 5 × 44,100 resampled samples.
        assert_eq!(frames, (5 * 44100usize).div_ceil(HOP));
        assert_eq!(feat.len(), frames * FEATURES);
        assert!(feat.iter().all(|v| v.is_finite()));
    }
}
