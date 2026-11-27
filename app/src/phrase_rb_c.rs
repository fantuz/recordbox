//! Stage C of the phrase analyser port: the whole track in about 35-second pieces of whole
//! beats, three decimated streams (high band ÷2, ÷8, ÷32) and per-beat features (band levels,
//! peaks, sixteenth-note onset patterns, hi-hat counts, a 72-bin constant-Q chromagram, FFT
//! spectra per bar half and per beat). Mirrors `Sub_Main_C_init`, `Sub_Main_C` and
//! `Set_Sub_Main_C_Out`, including the reference's carry-over between pieces.

use crate::phrase_rb_dsp::{max_val, prewarp, square_sum8, sum8, Decimator, Filter};

const Q1: f64 = 1.8477590650225735;
const Q2: f64 = 0.76536686473017967;
const SQ2: f64 = 1.4142135623730951;

/// A 2-D array as the reference keeps it (rows × cols, row-major).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mat2 {
    pub rows: usize,
    pub cols: usize,
    pub d: Vec<f64>,
}

impl Mat2 {
    pub fn zeros(rows: usize, cols: usize) -> Mat2 {
        Mat2 { rows, cols, d: vec![0.0; rows * cols] }
    }
    pub fn row(&self, r: usize) -> &[f64] {
        &self.d[r * self.cols..(r + 1) * self.cols]
    }
    pub fn row_mut(&mut self, r: usize) -> &mut [f64] {
        &mut self.d[r * self.cols..(r + 1) * self.cols]
    }
}

/// The FFT the reference uses (`c_fft`): radix 2, table of sines, Hann window.
#[derive(Clone, Debug, Default)]
pub struct Fft {
    pub n: usize,
    pub win: Vec<f64>,
    sin: Vec<f64>,
    swaps: Vec<(usize, usize)>,
}

impl Fft {
    pub fn new(n: usize) -> Fft {
        let nn = n as f64;
        let step = 6.2831853071795862 / nn;
        let sin: Vec<f64> = (0..n + n / 4).map(|i| (i as f64 * step).sin()).collect();
        let mut swaps = vec![];
        let half = n / 2;
        let (mut j, mut i) = (0usize, 0usize);
        loop {
            if i < j {
                swaps.push((i, j));
            }
            i += 1;
            if i >= n {
                break;
            }
            let mut k = half;
            if j >= half {
                loop {
                    let k2 = k;
                    j -= k;
                    k = k2 / 2;
                    if j < k {
                        break;
                    }
                }
            }
            j += k;
        }
        let h = (n as i32 - 1) as f64 * 0.5;
        let c = 3.1415926535897931 / (h + 1.0);
        let win = (0..n).map(|i| ((i as f64 - h) * c).cos().mul_add(0.5, 0.5)).collect();
        Fft { n, win, sin, swaps }
    }

    /// Window then transform `re`/`im` in place.
    pub fn window_fft(&self, re: &mut [f64], im: &mut [f64]) {
        let n = self.n;
        for i in 0..n {
            re[i] *= self.win[i];
            im[i] *= self.win[i];
        }
        for &(a, b) in &self.swaps {
            re.swap(a, b);
            im.swap(a, b);
        }
        let q = n >> 2;
        let mut h = 1usize;
        let mut step = n;
        while 2 * h <= n {
            step /= 2;
            let mut t = 0usize;
            for k in 0..h {
                let c = self.sin[t + q];
                let s = self.sin[t];
                let mut i = k;
                while i < n {
                    let j = i + h;
                    let (xr, xi) = (re[j], im[j]);
                    let tr = xr.mul_add(c, xi * s);
                    let ti = xi.mul_add(c, -(xr * s));
                    let (ar, ai) = (re[i], im[i]);
                    re[j] = ar - tr;
                    im[j] = ai - ti;
                    re[i] = tr + ar;
                    im[i] = ai + ti;
                    i += 2 * h;
                }
                t += step;
            }
            h *= 2;
        }
    }

    /// Magnitudes in dB of the first n/2 bins (−200 below 1e-10) of `x[start..start+n]`.
    pub fn spectrum_db(&self, x: &[f64], start: usize, out: &mut [f64]) {
        let n = self.n;
        let mut re: Vec<f64> = (0..n).map(|i| x.get(start + i).copied().unwrap_or(0.0)).collect();
        let mut im = vec![0.0; n];
        self.window_fft(&mut re, &mut im);
        for k in 0..n / 2 {
            let m = im[k].mul_add(im[k], re[k] * re[k]).sqrt();
            out[k] = if m < 1.0e-10 { -200.0 } else { m.log10() * 20.0 };
        }
    }
}

/// `hanning_ex(n)`: Hann window without zero ends, written into `buf` (which keeps whatever
/// was beyond `n` from earlier calls, as the reference's reused buffer does).
pub fn hanning_ex(n: i32, buf: &mut Vec<f64>) {
    if buf.len() < n.max(0) as usize {
        buf.resize(n as usize, 0.0);
    }
    let c = 6.2831853071795862;
    if n & 1 != 0 {
        let v12 = (if n + 1 >= 0 { n + 1 } else { n + 2 }) >> 1;
        let v13 = (n + 1) as f64;
        if n >= 3 {
            let v15 = if v12 <= 2 { 2 } else { v12 };
            for i in 0..(v15 - 1) as usize {
                let w = (-(((i as i32 + 1) as f64 * c * (1.0 / v13)).cos())).mul_add(0.5, 0.5);
                buf[i] = w;
                buf[n as usize - 1 - i] = w;
            }
        }
        buf[(v12 - 1) as usize] = (-((v12 as f64 * c / v13).cos())).mul_add(0.5, 0.5);
    } else if n >= 2 {
        let v7 = 1.0 / (n + 1) as f64;
        for i in 0..(n as usize >> 1) {
            let w = (-(((i as i32 + 1) as f64 * c * v7).cos())).mul_add(0.5, 0.5);
            buf[i] = w;
            buf[n as usize - 1 - i] = w;
        }
    }
}

/// State of the HP-variant onset pattern (`RhythmPatternAnalyze_HP` / `2_HP`).
#[derive(Clone, Debug, Default)]
pub struct RhpState {
    pub last: f64,
    pub shift: i32,
    pub peak: f64,
    pub mode: i32,
    pub lp: [Filter; 4],
    pub env: Filter,
    /// second variant only: 110 Hz high-pass (4 sections) and its envelope filter
    pub hp: [Filter; 4],
    pub env2: Filter,
}

impl RhpState {
    pub fn init(&mut self, second: bool, sr: f64) {
        let w = prewarp(1256.6370614359173, sr);
        self.mode = 0;
        let near = |a: f64, lo: f64, hi: f64| a > lo && a < hi;
        if near(sr, 5512.49, 5512.51) || near(sr, 5999.99, 6000.01) || near(sr, 3999.99, 4000.01) {
            self.mode = 4;
            self.lp[0].make2(0.0, 0.0, w * w, 1.0, w * Q1, w * w, sr);
            self.lp[0].clear();
            self.lp[2] = self.lp[0];
            self.lp[2].clear();
            self.lp[1].make2(0.0, 0.0, w * w, 1.0, w * Q2, w * w, sr);
            self.lp[1].clear();
            self.lp[3] = self.lp[1];
            self.lp[3].clear();
        } else if near(sr, 1378.115, 1378.135) || near(sr, 1499.99, 1500.01) || near(sr, 999.99, 1000.01) {
            self.mode = 2;
            self.lp[0].make2(0.0, 0.0, w * w, 1.0, w * SQ2, w * w, sr);
            self.lp[0].clear();
        }
        let w2 = prewarp(62.831853071795862, sr);
        let r = if sr == 0.0 && !second { 5512.5 } else { sr };
        self.env.make2(0.0, 0.0, w2 * w2, 1.0, w2 * SQ2, w2 * w2, r);
        self.env.clear();
        if second {
            let w3 = prewarp(691.15038378975453, sr);
            self.hp[0].make2(1.0, 0.0, 0.0, 1.0, w3 * Q1, w3 * w3, sr);
            self.hp[0].clear();
            self.hp[2] = self.hp[0];
            self.hp[2].clear();
            self.hp[1].make2(1.0, 0.0, 0.0, 1.0, w3 * Q2, w3 * w3, sr);
            self.hp[1].clear();
            self.hp[3] = self.hp[1];
            self.hp[3].clear();
            self.env2 = self.env;
            self.env2.clear();
        }
        self.last = 0.0;
        self.shift = 0;
        self.peak = 0.0001;
    }

    fn lowpass(&mut self, x: &[f64]) -> Vec<f64> {
        let mut y = vec![0f64; x.len()];
        match self.mode {
            2 => self.lp[0].filter2(x, &mut y),
            4 => {
                self.lp[0].filter2(x, &mut y);
                self.lp[1].filter2_inplace(&mut y);
                self.lp[2].filter2_inplace(&mut y);
                self.lp[3].filter2_inplace(&mut y);
            }
            _ => {}
        }
        y
    }
}

#[inline]
fn a0(x: &[f64], i: i64) -> f64 {
    crate::phrase_rb_dsp::at0(x, i)
}

/// Max of `len` samples from 0-based `start`, None when the window would end past `n`
/// (the reference stops the bar loop there).
#[inline]
fn wmax(x: &[f64], start: i32, len: i32) -> f64 {
    max_val(crate::phrase_rb_dsp::win(x, start as i64, len as i64))
}

/// The shared tail of both HP variants: first difference, grid shift, then per bar the
/// strongest sixteenth-note positions marked (`iters` passes, threshold `rel`).
#[allow(clippy::too_many_arguments)]
fn rhp_bars(mut v: Vec<f64>, last_prev: f64, v58: i32, v177: i32, v175: i32, v21: f64, v31: i32, beats: &[i32], p3: i32, bars: i32, peak_in: f64, iters: usize, rel: f64, out: &mut [u8]) -> f64 {
    let n = v.len() as i32;
    for i in (1..v.len()).rev() {
        v[i] -= v[i - 1];
    }
    let v63 = v175 - v177;
    if !v.is_empty() {
        v[0] -= last_prev;
    }
    let v165 = v177 - 1;
    if v58 < v177 && v175 - v58 <= n {
        let s = (v177 - v58 - 1).max(0) as usize;
        let len = (v63 + 1).max(0) as usize;
        v = v.get(s..(s + len).min(v.len())).map(|w| w.to_vec()).unwrap_or_default();
    } else {
        let v65 = v175 - v58 - 1;
        if !(v65 > n || v58 <= v177) {
            let v180: Vec<f64> = v[..v65.max(0) as usize].to_vec();
            if n < v63 {
                v = vec![0.0; v63 as usize];
            }
            let z = (v58 - v165).max(0) as usize;
            for x in v.iter_mut().take(z) {
                *x = 0.0;
            }
            for (k, x) in v180.iter().enumerate() {
                if let Some(d) = v.get_mut(z + k) {
                    *d = *x;
                }
            }
        }
    }
    let n = v.len() as i32;
    let v168 = v21 * 0.25;
    let v167 = v21 * 0.5;
    let v166 = v21 * 0.75;
    let v172 = v31;
    let mut v24 = peak_in;
    let mut v176 = 0.0f64;
    for v74 in 0..bars {
        let v76 = p3 + 4 * v74;
        if v76 > beats.len() as i32 {
            break;
        }
        let mut vals = [0f64; 16];
        let mut ok = true;
        for b in 0..4i32 {
            if b > 0 && v76 + b > beats.len() as i32 {
                ok = false;
                break;
            }
            let base = (beats[(v76 - 1 + b) as usize] - v177) as f64 + 0.5;
            let len0 = if b == 0 { v31 } else { v172 };
            if (base as i32) + len0 > n {
                ok = false;
                break;
            }
            vals[4 * b as usize] = wmax(&v, base as i32, len0);
            for (q, off) in [v168, v167, v166].iter().enumerate() {
                let s = (base + off) as i32;
                if s + v172 > n {
                    ok = false;
                    break;
                }
                vals[4 * b as usize + q + 1] = wmax(&v, s, v172);
            }
            if !ok {
                break;
            }
        }
        if !ok {
            break;
        }
        let v162 = v24;
        let mut v131 = 0.0;
        for it in 0..iters {
            let mut mx = vals[0];
            let mut arg = 0usize;
            for (j, x) in vals.iter().enumerate().skip(1) {
                if mx < *x {
                    arg = j;
                }
                mx = mx.max(*x);
            }
            v131 = if it == 0 { mx } else { v176 };
            v176 = v131;
            if mx > 0.0001 && mx > v131 * rel && mx > v162 * 0.178 {
                if let Some(o) = out.get_mut(arg + 16 * v74 as usize) {
                    *o = 1;
                }
                vals[arg] = 0.0;
            }
        }
        v24 = v162.max(v131);
    }
    v24
}

/// `RhythmPatternAnalyze_HP` (first variant).
#[allow(clippy::too_many_arguments)]
pub fn rhythm_hp(st: &mut RhpState, x: &[f64], bpm: f64, ms: i32, p3: i32, sr: f64, p5: i32, beats: &[i32], p7: i32) -> Option<Vec<u8>> {
    let v18 = st.last;
    let v19 = st.shift;
    let n = x.len() as i32;
    let v21 = sr * 60.0 / bpm;
    let mut v22 = *beats.get((p7 - 1).max(0) as usize)?;
    if v21 as i32 + v22 - 1 <= n {
        v22 = v21 as i32 + v22 - 1;
    }
    let v175 = v22;
    let v23 = if p7 - p3 + 1 >= 0 { p7 - p3 + 1 } else { p7 - p3 + 4 };
    let v24 = if p5 <= 1 { 0.0001 } else { st.peak };
    let mut v25 = st.lowpass(x);
    for v in v25.iter_mut() {
        *v = v.abs();
    }
    st.env.filter2_inplace(&mut v25);
    let v173 = v19;
    let msi = ((ms as f64 * 0.001).mul_add(sr, 0.5)) as i32;
    let v177 = (v21.mul_add((p3 - 1) as f64, (msi + 1) as f64)) as i32;
    let v27 = (v21 * 4.0) as i32;
    let v28 = (v23 >> 2) - ((v177 - 1 + (v23 >> 2) * v27 > n) as i32);
    if v28 <= 0 {
        st.last = a0(&v25, n as i64 - 1);
        st.shift = v173;
        st.peak = v24;
        return Some(vec![0u8; 1]);
    }
    let mut out = vec![0u8; 16 * v28 as usize];
    let v165 = v177 - 1;
    let v31 = (v21 * 0.25) as i32;
    let mut v56 = 0i32;
    let mut v57 = v173;
    if v28 >= 2 {
        let s0 = (v177 - 1 + v27).max(0) as usize;
        let len = ((v28 - 1) * v27).max(0) as usize;
        let mut v182: Vec<f64> = (0..len).map(|i| v25.get(s0 + i).copied().unwrap_or(0.0)).collect();
        let v183 = v182.len() as i32;
        let mut idx = [0i32; 8];
        let mut v154 = 0i32;
        let mut v158 = 0i32;
        let mut v33 = 0.0f64;
        let a = |i: i32| a0(&v25, i as i64);
        let mut found = false;
        let mut v37 = 0;
        'outer: for v32 in 0..8usize {
            if v32 != 0 {
                let p = idx[v32 - 1];
                if p <= v183 && p >= 1 {
                    v182[p as usize - 1] = 0.0;
                }
            }
            let (mut v35, mi) = crate::phrase_rb_dsp::max_idx(&v182);
            idx[v32] = mi as i32 + 1;
            let v36 = if v21 * 4.0 == 0.0 { 0xFFFF } else { (idx[v32] as f64 * 0.25 * (1.0 / v21)) as i32 };
            v37 = v27 + v36 * v27;
            let v38 = v37 + v165;
            let v39 = if v37 + v165 >= n { n } else { v37 + v165 };
            let v40 = a(v39 - 1);
            if v40 <= v35 * 0.2 || v37 >= v183 - v27 {
                continue;
            }
            let v184s = v177 - 2 + v37;
            let v41 = a(v38);
            let v42 = a(v37 + v177);
            let mut hit: Option<(i32, i32)> = None; // (v43, v45)
            if v40 <= v41 {
                if v41 > v42 || v42 > a(v38 + 2) {
                    continue;
                }
                if v31 >= 1 {
                    let mut v45 = 0i32;
                    let mut v46 = v165 + v37;
                    let mut br = false;
                    while a(v46 + 1) - a(v46) >= 0.0 {
                        v45 -= 1;
                        v46 += 1;
                        if -v31 == v45 {
                            br = true;
                            break;
                        }
                    }
                    if !br {
                        hit = Some((-v45, 1 - v45));
                    }
                }
            } else {
                if v41 <= v42 || v42 <= a(v38 + 2) {
                    continue;
                }
                if v31 > 0 {
                    let mut v43 = 0i32;
                    let mut v44 = v177 - 4 + v37;
                    loop {
                        let v45 = v43 + 1;
                        if a(v44) - a(v44 + 1) < 0.0 {
                            hit = Some((v43, v45));
                            break;
                        }
                        v44 -= 1;
                        v43 += 1;
                        if v31 == v45 {
                            break;
                        }
                    }
                }
            }
            if let Some((v43, v45)) = hit {
                let v47 = if v40 > v41 { !v43 } else { v45 };
                v154 = v47 + v37;
                v33 = a(v177 - 2 + v47 + v37);
            }
            if v33 > v35 * 0.595662 {
                let v48 = crate::phrase_rb_dsp::min_val(crate::phrase_rb_dsp::win(&v25, v184s as i64, (v27 + 1) as i64));
                let cnt = (v21 * 1.25) as i32;
                if cnt >= 1 {
                    let mut v49 = false;
                    let mut v50 = 0i32;
                    let v51 = (v48 + v35) * 0.5;
                    let mut v52 = v154 - 1;
                    let mut v53 = cnt;
                    let lim = (v21 * 0.0625) as i32;
                    loop {
                        let mut post = false;
                        if !v49 {
                            let v54 = a(v177 - 2 + v52);
                            if a(v177 - 3 + v52) >= v54 && v54 < v51 {
                                v49 = true;
                                v158 = v52;
                                v35 = v54;
                            } else {
                                v49 = false;
                            }
                        } else if a(v177 - 2 + v52) <= v35 * 1.12 {
                            post = true;
                        } else {
                            v50 = 0;
                            v49 = false;
                        }
                        if post {
                            v50 += 1;
                            if v50 < lim {
                                v49 = true;
                            } else {
                                if v35 == 0.0 {
                                    break 'outer;
                                }
                                if v33 / v35 > 2.11 {
                                    found = true;
                                    break 'outer;
                                }
                                continue 'outer;
                            }
                        }
                        v52 -= 1;
                        v53 -= 1;
                        if v53 == 0 {
                            continue 'outer;
                        }
                    }
                }
            }
        }
        if found {
            v56 = (v37 as f64 + 0.5 - v158 as f64) as i32;
            if v56 >= v177 {
                v56 = v177;
            }
        }
        v57 = v173;
    }
    st.shift = v56;
    let v58 = if p5 <= 1 { v56 } else { v57 };
    st.last = a0(&v25, n as i64 - 1);
    let peak = rhp_bars(v25, v18, v58, v177, v175, v21, v31, beats, p3, v28, v24, 6, 0.596, &mut out);
    st.peak = peak;
    Some(out)
}

/// `RhythmPatternAnalyze2_HP`: search on the 200 Hz envelope, onsets from the 110–200 Hz band.
#[allow(clippy::too_many_arguments)]
pub fn rhythm2_hp(st: &mut RhpState, x: &[f64], bpm: f64, ms: i32, p3: i32, sr: f64, p5: i32, beats: &[i32], p7: i32) -> Option<Vec<u8>> {
    let v18 = st.last;
    let v174 = st.shift;
    let n = x.len() as i32;
    let v20 = sr * 60.0 / bpm;
    let mut v21 = *beats.get((p7 - 1).max(0) as usize)?;
    if v20 as i32 + v21 - 1 <= n {
        v21 = v20 as i32 + v21 - 1;
    }
    let v171 = v21;
    let v22 = if p7 - p3 + 1 >= 0 { p7 - p3 + 1 } else { p7 - p3 + 4 };
    let v23 = if p5 <= 1 { 0.0001 } else { st.peak };
    st.shift = 0;
    let mut v24 = st.lowpass(x);
    let mut v26: Vec<f64> = v24.iter().map(|v| v.abs()).collect();
    st.env.filter2_inplace(&mut v26);
    let msi = ((ms as f64 * 0.001).mul_add(sr, 0.5)) as i32;
    let v173 = (v20.mul_add((p3 - 1) as f64, (msi + 1) as f64)) as i32;
    let v27 = (v20 * 4.0) as i32;
    let v168 = v173 - 1;
    let v28 = (v22 >> 2) - ((v173 - 1 + (v22 >> 2) * v27 > n) as i32);
    if v28 <= 0 {
        st.last = a0(&v26, n as i64 - 1);
        st.peak = v23;
        st.shift = v174;
        return Some(vec![0u8; 1]);
    }
    let mut out = vec![0u8; 16 * v28 as usize];
    let v170 = (v20 * 0.25) as i32;
    let mut v33 = 0i32;
    if v28 >= 2 {
        let s0 = (v173 - 1 + v27).max(0) as usize;
        let len = ((v28 - 1) * v27).max(0) as usize;
        let mut v181: Vec<f64> = (0..len).map(|i| v26.get(s0 + i).copied().unwrap_or(0.0)).collect();
        let v182 = v181.len() as i32;
        let mut idx = [0i32; 8];
        let mut v154 = 0i32;
        let mut v157 = 0i32;
        let mut v152 = 0.0f64;
        let a = |i: i32| a0(&v26, i as i64);
        let mut found = false;
        let mut v37 = 0;
        'outer: for v31 in 0..8usize {
            if v31 != 0 {
                let p = idx[v31 - 1];
                if p <= v182 && p >= 1 {
                    v181[p as usize - 1] = 0.0;
                }
            }
            let (mut v35, mi) = crate::phrase_rb_dsp::max_idx(&v181);
            idx[v31] = mi as i32 + 1;
            let v36 = if v20 * 4.0 == 0.0 { 0xFFFF } else { (idx[v31] as f64 * 0.25 * (1.0 / v20)) as i32 };
            v37 = v27 + v36 * v27;
            let v38 = v37 + v168;
            let v39 = if v37 + v168 >= n { n } else { v37 + v168 };
            let v40 = a(v39 - 1);
            if v40 <= v35 * 0.2 || v37 >= v182 - v27 {
                continue;
            }
            let v180s = v168 - 1 + v37;
            let v41 = a(v38);
            if v40 > v41 {
                continue;
            }
            let v42 = a(v37 + v173);
            if v41 > v42 || v42 > a(v38 + 2) {
                continue;
            }
            let mut v44;
            if v170 < 1 {
                v44 = v154;
            } else {
                v44 = v37 + 1;
                let mut v43 = v170;
                let mut broke = false;
                loop {
                    let v45 = v173 + v44 - 2;
                    if a(v173 - 1 + v44) - a(v45) < 0.0 {
                        v152 = a(v45);
                        broke = true;
                        break;
                    }
                    v44 += 1;
                    v43 -= 1;
                    if v43 == 0 {
                        break;
                    }
                }
                if !broke {
                    v44 = v154;
                }
            }
            if v152 > v35 * 0.595662 {
                let v46 = crate::phrase_rb_dsp::min_val(crate::phrase_rb_dsp::win(&v26, v180s as i64, (v27 + 1) as i64));
                let mut v47 = (v20 * 1.25) as i32;
                if v47 < 1 {
                    v154 = v44;
                    continue;
                }
                let mut v48 = false;
                let mut v49 = 0i32;
                let v50 = (v46 + v35) * 0.5;
                let mut v51 = v44 - 1;
                let lim = (v20 * 0.0625) as i32;
                loop {
                    let mut post = false;
                    if !v48 {
                        let v52 = a(v173 - 2 + v51);
                        if a(v173 - 3 + v51) >= v52 && v52 < v50 {
                            v48 = true;
                            v157 = v51;
                            v35 = v52;
                        } else {
                            v48 = false;
                        }
                    } else if a(v173 - 2 + v51) <= v35 * 1.12 {
                        post = true;
                    } else {
                        v49 = 0;
                        v48 = false;
                    }
                    if post {
                        v49 += 1;
                        if v49 < lim {
                            v48 = true;
                        } else {
                            if v35 == 0.0 {
                                break 'outer;
                            }
                            if v152 / v35 > 2.11 {
                                found = true;
                                break 'outer;
                            }
                            v154 = v44;
                            continue 'outer;
                        }
                    }
                    v51 -= 1;
                    v47 -= 1;
                    if v47 == 0 {
                        v154 = v44;
                        continue 'outer;
                    }
                }
            }
            v154 = v44;
        }
        if found {
            v33 = (v37 as f64 + 0.5 - v157 as f64) as i32;
            if v33 >= v173 {
                v33 = v173;
            }
        }
    }
    st.shift = v33;
    let v54 = if p5 <= 1 { v33 } else { v174 };
    for k in 0..4 {
        st.hp[k].filter2_inplace(&mut v24);
    }
    let mut v55: Vec<f64> = v24.iter().map(|v| v.abs()).collect();
    st.env2.filter2_inplace(&mut v55);
    st.last = a0(&v55, n as i64 - 1);
    let peak = rhp_bars(v55, v18, v54, v173, v171, v20, v170, beats, p3, v28, v23, 12, 0.5, &mut out);
    st.peak = peak;
    Some(out)
}

/// Per-beat windows [b_i, b_{i+1}) (1-based), reduced with `f`; 0 outside the buffer.
fn per_beat(x: &[f64], ext: &[i32], p5: usize, f: impl Fn(&[f64]) -> f64) -> Vec<f64> {
    let n = x.len() as i32;
    (0..p5)
        .map(|k| {
            let s = ext[k];
            if s >= 1 && s <= n {
                let e = ext[k + 1];
                if e - 1 <= n {
                    return f(crate::phrase_rb_dsp::win(x, s as i64 - 1, (e - s) as i64));
                }
            }
            0.0
        })
        .collect()
}

/// Quarter-beat square sums (the LowPower16 family).
fn quarter_power(y: &[f64], beats: &[i32], p5: usize, v16: i32, v59: i32, last_rule: usize) -> Option<Vec<f64>> {
    let mut q = vec![0f64; 4 * p5];
    let w = |s: i32, len: i32| -> Option<f64> {
        if len < 0 {
            return None;
        }
        Some(square_sum8(crate::phrase_rb_dsp::win(y, s as i64 - 1, len as i64)))
    };
    for k in 0..p5 {
        let v18 = beats[k];
        let v19 = if k + last_rule >= p5 { v59 + v18 } else { beats[k + 1] };
        q[4 * k] = w(v18, v16)?;
        let v20 = v18 + v16;
        q[4 * k + 1] = w(v20, v16)?;
        let v21 = v20 + v16;
        q[4 * k + 2] = w(v21, v16)?;
        q[4 * k + 3] = w(v21 + v16, v19 - (v21 + v16))?;
    }
    Some(q)
}

/// Stage C state: the analyser object's fields from +920 to +7536 that this stage uses.
#[derive(Clone, Debug, Default)]
pub struct StageC {
    pub sr: f64,
    pub sr2: f64,
    pub sr8: f64,
    pub sr32: f64,
    pub bpm: f64,
    pub total: i32,
    beat_s: f64,
    dec2: Decimator,
    dec8: Decimator,
    dec32: Decimator,
    b2: Vec<f64>,
    b8: Vec<f64>,
    b32: Vec<f64>,
    t2: Vec<f64>,
    t8: Vec<f64>,
    t32: Vec<f64>,
    pub state: i32,
    n2: i32,
    n8: i32,
    n32: i32,
    f731: i32,
    consumed: i32,
    pub chunk: i32,
    ms: i32,
    bar0: i32,
    bar: i32,
    pub done_beats: i32,
    pub f888: i32,
    pub f889: i32,
    pub zero_flag: i32,
    f361: f64,
    f362: f64,
    f363: f64,
    prev_peak: f64,
    pub nbeats: i32,
    pub bt2: Vec<i32>,
    pub bt8: Vec<i32>,
    pub bt32: Vec<i32>,
    // per-beat outputs (reference offsets in the names)
    pub g2968: Vec<f64>,
    pub g3000: Vec<f64>,
    pub g3032: Vec<f64>,
    pub g3064: Vec<f64>,
    pub g3096: Vec<f64>,
    pub g3128: Vec<f64>,
    pub g3160: Vec<f64>,
    pub g3224: Vec<f64>,
    pub g3256: Vec<f64>,
    pub g3288: Vec<f64>,
    pub g3320: Vec<f64>,
    pub g3352: Vec<f64>,
    pub g3384: Vec<f64>,
    pub g3416: Vec<f64>,
    pub g3448: Vec<u8>,
    pub n3448: usize,
    pub g3480: Vec<u8>,
    pub n3480: usize,
    pub g3512: Vec<u8>,
    pub g3568: Mat2,
    pub g3592: Mat2,
    pub g3616: Mat2,
    pub g3640: Mat2,
    // filters and analyser states
    f920: Filter,
    f1128: Filter,
    f1184: Filter,
    f4480: Filter,
    f4536: Filter,
    f4688: [Filter; 4],
    f4976: Filter,
    f5032: Filter,
    f5120: [Filter; 4],
    f5344: Filter,
    rhp: RhpState,
    rhp2: RhpState,
    ie_carry: Vec<f64>,
    ie_ncarry: i32,
    ie_len: Vec<i32>,
    ie_ratio: Vec<f64>,
    ie_kern: Vec<Vec<(f64, f64)>>,
    ie_first: i32,
    ie_frames: i32,
    fft4096: Fft,
    fft2048: Fft,
    f6944: Filter,
    f7000: Filter,
    f7056: Filter,
    f7112: Filter,
    f7232: [Filter; 4],
    f7456: Filter,
}

fn lp2(f: &mut Filter, w: f64, q: f64, sr: f64) {
    f.make2(0.0, 0.0, w * w, 1.0, w * q, w * w, sr);
    f.clear();
}
fn hp2(f: &mut Filter, w: f64, q: f64, sr: f64) {
    f.make2(1.0, 0.0, 0.0, 1.0, w * q, w * w, sr);
    f.clear();
}
fn lp1(f: &mut Filter, w: f64, sr: f64) {
    f.make1(0.0, w, 1.0, w, sr);
    f.clear();
}

impl StageC {
    /// `Sub_Main_C_init(first_ms, bar)`.
    pub fn init(sr: i32, total: i32, bpm: f64, ms: i32, bar: i32) -> Option<StageC> {
        let srf = sr as f64;
        let mut c = StageC { sr: srf, sr2: srf * 0.5, sr8: srf * 0.125, sr32: srf * 0.03125, bpm, total, ..Default::default() };
        let (v7, v6, v8, v9) = (c.sr, c.sr2, c.sr8, c.sr32);
        if !(c.dec2.init_highpass(v6 * 0.55, v7, 2) && c.dec8.init_lowpass(v8 * 0.4, v7, 8) && c.dec32.init_lowpass(v9 * 0.4, v7, 32)) {
            return None;
        }
        c.beat_s = 60.0 / bpm;
        c.f361 = ms as f64 * 0.001;
        c.chunk = 1;
        // BeatsSamplesCalcFX
        let v14 = total as f64;
        let (p0, p2, p4) = (v14 / 2.0, v14 / 8.0, v14 / 32.0);
        let b = 60.0 / bpm;
        let (v13, v15, v16) = (b * v6, b * v8, b * v9);
        let v14m = ms as f64 * 0.001;
        let v17 = v14m.mul_add(v6, 0.5).trunc();
        let v19 = v14m.mul_add(v8, 0.5).trunc();
        let v20 = v14m.mul_add(v9, 0.5).trunc();
        let mut v18 = ((p0 - v17) / v13) as i32;
        if (((p2 - v19) / v15) as i32) < v18 {
            v18 = ((p2 - v19) / (60.0 / bpm * v8)) as i32;
        }
        if v18 >= ((p4 - v20) / (60.0 / bpm * v9)) as i32 {
            v18 = ((p4 - v20) / (60.0 / bpm * v9)) as i32;
        }
        if v18 < 1 {
            return None;
        }
        c.nbeats = v18;
        let m = (v18 + 1) as usize;
        c.bt2 = (0..m).map(|i| v13.mul_add(i as f64, v17 + 1.5) as i32).collect();
        c.bt8 = (0..m).map(|i| v15.mul_add(i as f64, v19 + 1.5) as i32).collect();
        c.bt32 = (0..m).map(|i| v16.mul_add(i as f64, v20 + 1.5) as i32).collect();
        let l = (v18 - 1) as usize;
        c.bt2[l] = c.bt2[l].min(p0 as i32);
        c.bt8[l] = c.bt8[l].min(p2 as i32);
        c.bt32[l] = c.bt32[l].min(p4 as i32);
        let v15b = v18;
        c.ms = ms;
        c.bar = bar;
        c.bar0 = bar;
        let v16b = if v15b - bar + 1 >= 0 { v15b - bar + 1 } else { v15b - bar + 4 };
        let v17b = (v16b >> 2).max(0) as usize;
        let nb = v15b as usize;
        c.g2968 = vec![0.0; 4 * nb];
        c.g3000 = vec![0.0; 4 * nb];
        c.g3032 = vec![0.0; 4 * nb];
        c.g3064 = vec![0.0; nb];
        c.g3616 = Mat2::zeros(4 * v17b, 1024);
        c.g3160 = vec![0.0; nb];
        c.g3096 = vec![0.0; nb];
        c.g3512 = vec![0; nb];
        c.g3448 = vec![0; 16 * v17b];
        c.n3448 = 16 * v17b;
        c.g3224 = vec![0.0; nb];
        c.g3480 = vec![0; 16 * v17b];
        c.n3480 = 16 * v17b;
        c.g3128 = vec![0.0; 4 * nb];
        c.g3256 = vec![0.0; nb];
        c.g3288 = vec![0.0; nb];
        c.g3320 = vec![0.0; nb];
        c.g3640 = Mat2::zeros(1, 1);
        c.g3592 = Mat2::zeros(2 * v17b, 2048);
        c.g3352 = vec![0.0; 4 * nb];
        let cols = (-((v8 * 1.2230919765166333).trunc() - v14 * 0.125) * 0.00390625) as i32;
        c.g3568 = Mat2::zeros(72, cols.max(0) as usize);
        c.g3384 = vec![0.0; nb];
        c.g3416 = vec![0.0; 8 * nb];
        // feature initialisations
        let w = prewarp(628.31853071795865, v9);
        lp2(&mut c.f4480, w, SQ2, v9);
        lp1(&mut c.f4536, prewarp(31.415926535897931, v9), v9);
        lp2(&mut c.f1128, prewarp(942.47779607693792, v9), SQ2, v9);
        lp1(&mut c.f1184, prewarp(31.415926535897931, v9), v9);
        lp2(&mut c.f920, prewarp(628.31853071795865, v9), SQ2, v9);
        lp2(&mut c.f4688[0], prewarp(251.32741228718345, v9), SQ2, v9);
        for k in 1..4 {
            c.f4688[k] = c.f4688[0];
        }
        hp2(&mut c.f4976, prewarp(3141.5926535897929, v8), SQ2, v8);
        lp1(&mut c.f5032, prewarp(31.415926535897931, v8), v8);
        let w4 = prewarp(2513.2741228718346, v8);
        let w1k = prewarp(6283.1853071795858, v8);
        hp2(&mut c.f5120[0], w4, Q1, v8);
        lp2(&mut c.f5120[2], w1k, Q1, v8);
        hp2(&mut c.f5120[1], w4, Q2, v8);
        lp2(&mut c.f5120[3], w1k, Q2, v8);
        lp1(&mut c.f5344, prewarp(62.831853071795862, v8), v8);
        c.rhp.init(false, v8);
        c.rhp2.init(true, v8);
        c.intro_init(v8)?;
        c.fft4096 = Fft::new(4096);
        c.fft2048 = Fft::new(2048);
        lp2(&mut c.f6944, prewarp(3141.5926535897929, v9), SQ2, v9);
        hp2(&mut c.f7000, prewarp(3141.5926535897929, v8), SQ2, v8);
        lp1(&mut c.f7056, prewarp(31.415926535897931, v9), v9);
        lp1(&mut c.f7112, prewarp(31.415926535897931, v8), v8);
        let w5k = prewarp(31415.926535897932, v6);
        hp2(&mut c.f7232[0], w5k, Q1, v6);
        hp2(&mut c.f7232[1], w5k, Q2, v6);
        let w7k = prewarp(43982.297150257102, v6);
        lp2(&mut c.f7232[2], w7k, Q1, v6);
        lp2(&mut c.f7232[3], w7k, Q2, v6);
        lp1(&mut c.f7456, prewarp(62.831853071795862, v6), v6);
        Some(c)
    }

    /// True when stage C/D runs (the reference's +192 flag: more than 30 beats after the bar).
    pub fn go(&self) -> bool {
        self.nbeats - self.bar0 > 30
    }

    fn intro_init(&mut self, sr: f64) -> Option<()> {
        self.ie_carry = vec![0.0; (sr * 3.0) as i32 as usize];
        self.ie_ncarry = 0;
        let c12 = f64::from_bits(0x3FB5555555555555);
        let mut f = vec![27.5f64; 73];
        for (k, v) in f.iter_mut().enumerate().skip(1) {
            *v = (k as f64 * c12).exp2() * 27.5;
        }
        let v11 = sr + sr;
        let len0 = (v11 / (f[1] - f[0])) as i32;
        let t: Vec<f64> = (0..len0.max(0)).map(|i| (i + 1) as f64 * (1.0 / sr)).collect();
        let l0 = len0.max(0) as usize;
        let mut kern: Vec<Vec<(f64, f64)>> = vec![vec![(0.0, 0.0); l0]; 72];
        for k in 0..12 {
            let w = f[k] * 6.2831853071795862;
            for i in 0..l0 {
                let a = w * t[i];
                kern[k][i] = (a.cos(), a.sin());
            }
        }
        let pairs = l0 & !1;
        for k in 12..72 {
            for i in 0..l0 {
                let (re, im) = kern[k - 12][i];
                let r = if i < pairs { (-im).mul_add(im, re * re) } else { re.mul_add(re, -(im * im)) };
                kern[k][i] = (r, re * (im + im));
            }
        }
        let mut lens = vec![0i32; 72];
        let mut ratio = vec![0f64; 72];
        lens[0] = len0;
        let mut win = Vec::new();
        hanning_ex(len0, &mut win);
        for i in 0..l0 {
            let w = win[i];
            kern[0][i].0 *= w;
            kern[0][i].1 *= w;
        }
        ratio[0] = 1.0;
        for k in 1..72 {
            lens[k] = (v11 / (f[k + 1] - f[k])) as i32;
            hanning_ex(lens[k], &mut win);
            for i in 0..l0 {
                let w = win.get(i).copied().unwrap_or(0.0);
                kern[k][i].0 *= w;
                kern[k][i].1 *= w;
            }
            ratio[k] = lens[0] as f64 / lens[k] as f64;
        }
        self.ie_len = lens;
        self.ie_ratio = ratio;
        self.ie_kern = kern;
        Some(())
    }

    /// `IntroEndFind_a`: constant-Q magnitudes (bins 23–71) every 256 samples.
    fn intro_end_find(&mut self, x: &[f64], bpm: f64, p2: i32, p3: i32, sr: f64, p5: i32) -> Option<Mat2> {
        let inner = (p2 as f64).mul_add(0.001, (p3 - 1) as f64 * 60.0 / bpm);
        self.ie_first = (sr * 0.00390625 * inner).ceil() as i32;
        let mut v78: Vec<f64> = Vec::with_capacity(self.ie_ncarry.max(0) as usize + x.len());
        if self.ie_ncarry != 0 {
            if self.ie_ncarry >= 1 {
                v78.extend_from_slice(&self.ie_carry[..self.ie_ncarry as usize]);
                v78.extend_from_slice(x);
            }
        } else {
            v78.extend_from_slice(x);
        }
        let len0 = self.ie_len[0];
        let v20 = ((v78.len() as i32 - len0) as f64 * 0.00390625) as i32;
        let mut out = Mat2::zeros(72, v20.max(0) as usize);
        let v76 = (len0 as f64 * 0.5) as i32;
        for fr in 0..v20.max(0) as usize {
            for bin in 23..72 {
                let l = self.ie_len[bin];
                let (mut re, mut im) = (0.0f64, 0.0f64);
                if l >= 1 {
                    let v32 = (l as f64 * 0.5) as i32;
                    let s = (256 * fr) as i64 + (v76 - v32) as i64;
                    let k = &self.ie_kern[bin];
                    let lu = l as usize;
                    let xs = |i: usize| a0(&v78, s + i as i64);
                    let n8 = if lu >= 8 { lu & !7 } else { 0 };
                    if n8 > 0 {
                        let mut ar = [0f64; 8];
                        let mut ai = [0f64; 8];
                        for c in 0..n8 / 8 {
                            for j in 0..8 {
                                let i = 8 * c + j;
                                let xv = xs(i);
                                ar[j] = k[i].0.mul_add(xv, ar[j]);
                                ai[j] = k[i].1.mul_add(xv, ai[j]);
                            }
                        }
                        re = (ar[6] + (ar[4] + (ar[2] + ar[0]))) + (ar[7] + (ar[5] + (ar[3] + ar[1])));
                        im = (ai[6] + (ai[4] + (ai[2] + ai[0]))) + (ai[7] + (ai[5] + (ai[3] + ai[1])));
                    }
                    for i in n8..lu {
                        let xv = xs(i);
                        re = k[i].0.mul_add(xv, re);
                        im = k[i].1.mul_add(xv, im);
                    }
                }
                out.d[bin * out.cols + fr] = self.ie_ratio[bin] * im.mul_add(im, re * re).sqrt();
            }
        }
        if p5 <= 1 {
            let v56 = (v20.max(0) as usize) << 8;
            let v57 = v78.len() as i32 - v56 as i32;
            if (self.ie_carry.len() as i32) < v57 {
                self.ie_carry.resize(v57 as usize, 0.0);
            }
            if v57 >= 1 {
                self.ie_carry[..v57 as usize].copy_from_slice(&v78[v56..v56 + v57 as usize]);
            }
            self.ie_ncarry = v57;
        }
        self.ie_frames = v20;
        Some(out)
    }

    /// `FFT_calc_2` (`big`: 4096 points at bar starts and halves) or `FFT_calc_1` (2048 points
    /// at every beat).
    fn fft_calc(&self, big: bool, x: &[f64], sr: f64, bpm: f64, p4: i32, beats: &[i32], p6: i32) -> Option<Mat2> {
        let v17 = p6 - p4 + 1;
        let v18 = if v17 >= 0 { v17 } else { p6 - p4 + 4 };
        let v19 = sr * 60.0 / bpm;
        let v20 = if v19 - v19.trunc() <= 0.0 { v19 as i32 } else { v19 as i32 + 1 };
        let (fft, n, per_bar) = if big { (&self.fft4096, 4096usize, 2usize) } else { (&self.fft2048, 2048usize, 4usize) };
        let size = (4 * v20 + n as i32 + 1).max(1) as usize;
        let bars = (v18 >> 2).max(0) as usize;
        let mut out = Mat2::zeros(per_bar * bars, n / 2);
        if v17 < 4 {
            return Some(out);
        }
        let mut buf = vec![0f64; size];
        for bar in 1..=bars {
            for v in buf.iter_mut().take(size) {
                *v = 0.0;
            }
            let v32 = p4 + 4 * (bar as i32 - 1);
            let v33 = *beats.get((v32 - 1) as usize)?;
            let bufn = buf.len() as i32;
            let v37 = if bar >= bars {
                (bufn + v33 - 1).min(x.len() as i32)
            } else {
                let nb = *beats.get((4 * bar as i32 + p4 - 1) as usize)?;
                (nb - 1).min(x.len() as i32)
            };
            let v38 = v37 - v33;
            if bufn <= v38 {
                buf.resize((v38 + 1) as usize, 0.0);
            }
            if v38 >= 0 {
                for i in 0..=(v38 as usize) {
                    buf[i] = a0(x, (v33 - 1) as i64 + i as i64);
                }
            }
            let offs: &[i32] = if big { &[0, 2] } else { &[0, 1, 2, 3] };
            for (r, o) in offs.iter().enumerate() {
                let b = *beats.get((v32 - 1 + o) as usize)?;
                let start = (1 - v33 + b - 1) as usize;
                let row = per_bar * (bar - 1) + r;
                fft.spectrum_db(&buf, start, out.row_mut(row));
            }
        }
        Some(out)
    }

    /// One block of the track (`Sub_Main_C`). Returns None where the reference fails.
    pub fn block(&mut self, blk: &[f64]) -> Option<()> {
        let (v8, v7, v9, v10, v11) = (self.sr, self.sr2, self.sr8, self.sr32, self.bpm);
        let v12 = self.total;
        let v13 = self.beat_s;
        let v123 = self.ms;
        let v14 = self.bar;
        let v15 = self.nbeats;
        let n = blk.len() as i32;
        let mut s2 = Vec::new();
        let mut s8 = Vec::new();
        let mut s32 = Vec::new();
        self.dec2.downsample(blk, &mut s2);
        self.dec8.downsample(blk, &mut s8);
        self.dec32.downsample(blk, &mut s32);
        self.b2.extend_from_slice(&s2);
        self.b8.extend_from_slice(&s8);
        self.b32.extend_from_slice(&s32);
        let v122 = v14;
        let v121 = v15;
        self.consumed += n;
        let mut v18 = self.consumed;
        let mut v19 = v123;
        let v20 = v123 as f64 * 0.001;
        let v21 = self.bar - 1;
        let v22 = ((35.11 - self.beat_s.mul_add(v21 as f64, v20)) / (self.beat_s * 4.0)) as i32;
        self.f731 = v22;
        let mut v23 = v21 + 4 * v22;
        self.n2 += s2.len() as i32;
        let v24 = self.n2;
        self.n8 += s8.len() as i32;
        let mut v25 = self.n8;
        self.n32 += s32.len() as i32;
        let dt = n as f64 / v8;
        self.f362 += dt;
        self.f363 += dt;
        let v26 = self.f362;
        let v27 = if self.chunk == 1 { v20 } else { 0.0 };
        let (mut v28, mut v29);
        let v30 = v121;
        if v26 >= v13.mul_add(v23 as f64, v20) && v121 >= self.done_beats + v23 && v23 >= 0 {
            self.state = 1;
            let i = v23 as usize;
            let (v32, v33, v34) = (*self.bt2.get(i)?, *self.bt8.get(i)?, *self.bt32.get(i)?);
            if v24 >= v32 {
                let v35 = (v32 - 1).max(0) as usize;
                self.t2 = self.b2.get(v35..v24 as usize)?.to_vec();
                self.b2.truncate(v35);
                v25 = self.n8;
                v29 = v35 as i32;
            } else {
                v29 = v24;
            }
            if v25 >= v33 {
                let v36 = self.b8.len() as i32;
                if v36 >= v33 && v36 >= v25 {
                    let v39 = (v33 - 1).max(0) as usize;
                    self.t8 = self.b8.get(v39..v25 as usize)?.to_vec();
                    self.b8.truncate(v39);
                    v25 = v39 as i32;
                }
            }
            v28 = self.n32;
            if v28 >= v34 {
                let v40 = self.b32.len() as i32;
                if v40 >= v34 && v40 >= v28 {
                    let v43 = (v34 - 1).max(0) as usize;
                    self.t32 = self.b32.get(v43..v28 as usize)?.to_vec();
                    self.b32.truncate(v43);
                    v28 = v43 as i32;
                }
            }
            let a = (v122 - 1) as f64 + self.f731 as f64 * 4.0;
            let v44 = self.f362 - a.mul_add(v13, v20);
            self.f361 = v44;
            self.f362 = v44;
            v18 = self.consumed;
            v19 = v123;
        } else {
            v28 = 0;
            v25 = 0;
            v29 = 0;
        }
        let mut v45 = self.state;
        if v18 >= v12 {
            if v45 == 0 {
                let (v53, v54, v119) = (self.n8, self.n32, self.n2);
                self.b2.resize(v119.max(0) as usize, 0.0);
                self.b8.resize(v53.max(0) as usize, 0.0);
                self.b32.resize(v54.max(0) as usize, 0.0);
                v23 = ((v26 - v27) / v13) as i32;
                let v55 = (v13 * -4.0).mul_add(self.f731 as f64, self.f363 - v123 as f64);
                self.f361 = v55;
                self.f362 = v55;
                v29 = v119;
                v25 = v53;
                v28 = v54;
            }
            v45 = 2;
            self.state = 2;
        }
        if v45 < 1 || v23 < 1 || v30 <= self.done_beats {
            return Some(());
        }
        let (v111, v116, v118) = (v25, v28, v29);
        if self.zero_flag == 0 {
            self.zero_flag = self.b32.iter().any(|v| *v != 0.0) as i32;
        }
        let v47 = self.done_beats;
        let v48 = v30 - v47;
        let mut v49 = v47 + v23;
        if v30 < v49 {
            v49 = v30;
            v23 = v48;
        }
        self.done_beats = v49;
        let nb = v23 as usize;
        let done = self.done_beats as usize;
        // LowPeakAnalyze
        {
            let mut y = vec![0f64; self.b32.len()];
            self.f4480.filter2(&self.b32, &mut y);
            for v in y.iter_mut() {
                *v = v.abs();
            }
            self.f4536.filter1_inplace(&mut y);
            let mut ext: Vec<i32> = self.bt32[..nb].to_vec();
            ext.push(self.bt32[nb - 1] + (v10 * 60.0 / v11) as i32);
            let ny = y.len() as i32;
            let v59 = per_beat(&y, &ext, nb, max_val);
            let mut v61 = vec![0f64; 4 * nb];
            for k in 0..nb {
                let (v26b, v25b) = (ext[k], ext[k + 1]);
                let v27b = ((v25b - v26b) as f64 * 0.25 + 0.5) as i32;
                let v28b = v26b + v27b;
                let w = |s: i32, len: i32, e1: i32| -> f64 {
                    if s >= 1 && s <= ny && e1 - 1 <= ny {
                        max_val(crate::phrase_rb_dsp::win(&y, s as i64 - 1, len as i64))
                    } else {
                        0.0
                    }
                };
                v61[4 * k] = w(v26b, v27b, v28b);
                let v32b = v26b + 2 * v27b;
                v61[4 * k + 1] = w(v28b, v32b - v28b, v32b);
                let v35b = 3 * v27b + v26b;
                v61[4 * k + 2] = w(v32b, v35b - v32b, v35b);
                v61[4 * k + 3] = w(v35b, v25b - v35b, v25b);
            }
            let mut v41 = vec![0f64; nb];
            let p6 = self.prev_peak;
            if p6 != 0.0 {
                let v42 = v59[0];
                if v42 > p6 {
                    if v42 > p6 * 1.77 {
                        v41[0] = 1.0;
                    }
                } else if !(v42 >= p6 || v42 + v42 >= p6) {
                    v41[0] = 0.5;
                }
                if v42 <= 0.09 && p6 <= 0.09 {
                    v41[0] *= 0.8;
                }
            }
            for k in 1..nb {
                let (v48b, v47b) = (v59[k - 1], v59[k]);
                if v47b > v48b {
                    if v47b > v48b * 1.77 {
                        v41[k] = 1.0;
                    }
                } else if !(v47b >= v48b || v48b <= v47b + v47b) {
                    v41[k] = 0.5;
                }
                if v47b <= 0.09 && v48b <= 0.09 {
                    v41[k] *= 0.8;
                }
            }
            put(&mut self.g3064, done, &v59);
            put(&mut self.g3128, 4 * done, &v61);
            put(&mut self.g3096, done, &v41);
            self.prev_peak = self.g3064[done - 1];
        }
        // LowLevelAnalyze_a
        {
            let mut y = vec![0f64; self.b32.len()];
            self.f1128.filter2(&self.b32, &mut y);
            for v in y.iter_mut() {
                *v = v.abs();
            }
            self.f1184.filter1_inplace(&mut y);
            let l = (v10 * 60.0 / v11) as i32;
            let last = self.bt32[nb - 1];
            let v16 = if last + l - 1 <= y.len() as i32 { last + l } else { y.len() as i32 };
            let mut ext: Vec<i32> = self.bt32[..nb].to_vec();
            ext.push(v16);
            let mut r = per_beat(&y, &ext, nb, sum8);
            for v in r.iter_mut() {
                *v *= 4.0;
            }
            put(&mut self.g3224, done, &r);
        }
        // LowPowerAnalyze16 and _40Hz
        {
            let mut y = vec![0f64; self.b32.len()];
            self.f920.filter2(&self.b32, &mut y);
            let v15f = v10 * 60.0 / v11;
            let q16 = (v15f.mul_add(0.25, 0.5)) as i32;
            let v59 = (v15f + 0.5) as i32 - 1;
            let mut q = quarter_power(&y, &self.bt32, nb, q16, v59, 2)?;
            if v10 <= 5512.45 || v10 >= 5512.55 {
                for v in q.iter_mut() {
                    *v *= 4.0;
                }
            }
            put(&mut self.g2968, 4 * done, &q);
            let mut y = vec![0f64; self.b32.len()];
            self.f4688[0].filter2(&self.b32, &mut y);
            for k in 1..4 {
                self.f4688[k].filter2_inplace(&mut y);
            }
            let mut q = quarter_power(&y, &self.bt32, nb, q16, v59, 1)?;
            if v10 <= 5512.45 || v10 >= 5512.55 {
                for v in q.iter_mut() {
                    *v *= 4.0;
                }
            }
            put(&mut self.g3352, 4 * done, &q);
        }
        // MiddleLevelAnalyze_a, MiddlePeakAnalyze, AllLevelAnalyze_a (whole beat array + end)
        {
            let l = (v9 * 60.0 / v11) as i32;
            let n8 = self.b8.len() as i32;
            let mut e = self.bt8[nb - 1] + l;
            if e - 1 > n8 {
                e = n8;
            }
            let mut ext = self.bt8.clone();
            ext.push(e);
            let mut y = vec![0f64; self.b8.len()];
            self.f4976.filter2(&self.b8, &mut y);
            for v in y.iter_mut() {
                *v = v.abs();
            }
            self.f5032.filter1_inplace(&mut y);
            let r = per_beat(&y, &ext, nb, sum8);
            put(&mut self.g3256, done, &r);
            let mut y = vec![0f64; self.b8.len()];
            self.f5120[0].filter2(&self.b8, &mut y);
            for k in 1..4 {
                self.f5120[k].filter2_inplace(&mut y);
            }
            for v in y.iter_mut() {
                *v = v.abs();
            }
            let mut y2 = vec![0f64; y.len()];
            self.f5344.filter1(&y, &mut y2);
            let r = per_beat(&y2, &ext, nb, max_val);
            put(&mut self.g3160, done, &r);
            let ya: Vec<f64> = self.b8.iter().map(|v| v.abs()).collect();
            let r = per_beat(&ya, &ext, nb, max_val);
            put(&mut self.g3384, done, &r);
        }
        // AllPowerAnalyze16
        {
            let v18f = v9 * 60.0 / v11;
            let v19q = (v18f.mul_add(0.25, 0.5)) as i32;
            let v28q = (v18f + 0.5) as i32 - 1;
            let mut a = vec![0f64; 4 * nb];
            let mut m = vec![0f64; 4 * nb];
            let x = &self.b8;
            for k in 0..nb {
                let v22 = self.bt8[k];
                let v23e = if k >= nb - 1 { v28q + v22 } else { self.bt8[k + 1] };
                let starts = [v22, v22 + v19q, v22 + 2 * v19q, v22 + 3 * v19q];
                for (j, s) in starts.iter().enumerate() {
                    let len = if j < 3 { v19q } else { v23e - s };
                    if len < 0 {
                        return None;
                    }
                    let mut w: Vec<f64> = (0..len as usize).map(|i| a0(x, (*s - 1) as i64 + i as i64)).collect();
                    a[4 * k + j] = square_sum8(&w);
                    for v in w.iter_mut() {
                        *v = v.abs();
                    }
                    m[4 * k + j] = max_val(&w);
                }
            }
            put(&mut self.g3000, 4 * done, &a);
            put(&mut self.g3032, 4 * done, &m);
        }
        // sixteenth-note onset patterns
        {
            let bar = self.bar;
            let chunk = self.chunk;
            let r = rhythm_hp(&mut self.rhp, &self.b8, v11, v123, bar, v9, chunk, &self.bt8, v23)?;
            let v50 = ((self.done_beats - self.bar0 + 1) as f64 * 0.25) as i32;
            let v52 = 16 * v50 - r.len() as i32;
            if v52 < 0 || v50 < 1 {
                self.n3448 = 1;
                if let Some(z) = self.g3448.get_mut(0) {
                    *z = 0;
                }
            } else {
                putb(&mut self.g3448, v52 as usize, &r);
            }
            let r = rhythm2_hp(&mut self.rhp2, &self.b8, v11, v123, bar, v9, chunk, &self.bt8, v23)?;
            let v56 = ((self.done_beats - self.bar0 + 1) as f64 * 0.25) as i32;
            let v58 = 16 * v56 - r.len() as i32;
            if v58 < 0 || v56 < 1 {
                self.n3480 = 1;
                if let Some(z) = self.g3480.get_mut(0) {
                    *z = 0;
                }
            } else {
                putb(&mut self.g3480, v58 as usize, &r);
            }
        }
        // HiHatAnalyze_a
        {
            let x: Vec<f64> = self.b2.iter().map(|v| v.abs()).collect();
            let inv = 1.0 / v7;
            let (mut s0, mut s1) = (0.0f64, 0.0f64);
            let mut on = vec![];
            for (i, xv) in x.iter().enumerate() {
                let k0 = if s0 > *xv { 20.0 } else { 2000.0 };
                let k1 = if s1 > *xv { f64::from_bits(0x402aaaaaaaaaaaab) } else { 1000.0 };
                s0 = (k0 * (xv - s0)).mul_add(inv, s0);
                s1 = (k1 * (xv - s1)).mul_add(inv, s1);
                if s0 > s1 && s0 > 0.06 {
                    on.push(i as i32 + 1);
                }
            }
            let gap = v7 * 13.333333333333332 / v11;
            let mut kept = vec![];
            let mut prev = 0;
            for o in on {
                if gap < (o - prev) as f64 {
                    kept.push(o);
                    prev = o;
                }
            }
            let mut cnt = vec![0u8; nb];
            let bt = &self.bt2;
            for o in &kept {
                for k in 0..nb.saturating_sub(1) {
                    if *o >= bt[k] && *o < bt[k + 1] {
                        cnt[k] = cnt[k].wrapping_add(1);
                    }
                }
                if *o >= bt[nb - 1] {
                    cnt[nb - 1] = cnt[nb - 1].wrapping_add(1);
                }
            }
            putb(&mut self.g3512, done - nb, &cnt);
        }
        // IntroEndFind_a → chromagram columns
        {
            let b8 = std::mem::take(&mut self.b8);
            let st = self.state;
            let r = self.intro_end_find(&b8, v11, 0, v122, v9, st);
            self.b8 = b8;
            let m = r?;
            if self.chunk == 1 {
                self.f889 = self.ie_first;
            }
            self.f888 += self.ie_frames;
            let c0 = (self.f888 - m.cols as i32).max(0) as usize;
            for row in 0..72 {
                let dst = self.g3568.cols;
                for (j, v) in m.row(row).iter().enumerate() {
                    if c0 + j < dst {
                        self.g3568.d[row * dst + c0 + j] = *v;
                    }
                }
            }
        }
        // FFT spectra
        {
            let m = self.fft_calc(true, &self.b8, v9, v11, v122, &self.bt8, v23)?;
            let v103 = 2 * ((self.done_beats - self.bar0 + 1) as f64 * 0.25) as i32 - m.rows as i32;
            for r in 0..m.rows {
                let dr = v103 + r as i32;
                if dr >= 0 && (dr as usize) < self.g3592.rows {
                    self.g3592.row_mut(dr as usize).copy_from_slice(m.row(r));
                }
            }
            let m = self.fft_calc(false, &self.b8, v9, v11, v122, &self.bt8, v23)?;
            let v65 = self.done_beats - self.bar0;
            let v66 = if v65 + 1 >= 0 { v65 + 1 } else { v65 + 4 };
            let v104 = (v66 & !3) - m.rows as i32;
            for r in 0..m.rows {
                let dr = v104 + r as i32;
                if dr >= 0 && (dr as usize) < self.g3616.rows {
                    self.g3616.row_mut(dr as usize).copy_from_slice(m.row(r));
                }
            }
        }
        // DividedBandAnalyze
        {
            let mut y32 = vec![0f64; self.b32.len()];
            self.f6944.filter2(&self.b32, &mut y32);
            for v in y32.iter_mut() {
                *v = v.abs();
            }
            let mut y8 = vec![0f64; self.b8.len()];
            self.f7000.filter2(&self.b8, &mut y8);
            for v in y8.iter_mut() {
                *v = v.abs();
            }
            self.f7056.filter1_inplace(&mut y32);
            self.f7112.filter1_inplace(&mut y8);
            let rs = |y: &[f64], a: i32, b: i32| -> f64 {
                if a < 0 || a > b || b >= y.len() as i32 {
                    0.0
                } else {
                    sum8(&y[a as usize..=b as usize])
                }
            };
            let mut r32 = vec![0f64; nb];
            let mut r8 = vec![0f64; nb];
            for k in 0..nb.saturating_sub(1) {
                r32[k] = rs(&y32, self.bt32[k] - 1, self.bt32[k + 1] - 2);
                r8[k] = rs(&y8, self.bt8[k] - 1, self.bt8[k + 1] - 2);
            }
            r32[nb - 1] = rs(&y32, self.bt32[nb - 1] - 1, y32.len() as i32 - 1);
            r8[nb - 1] = rs(&y8, self.bt8[nb - 1] - 1, y8.len() as i32 - 1);
            put(&mut self.g3288, done, &r32);
            put(&mut self.g3320, done, &r8);
        }
        // HighPeakAnalyze32
        {
            let mut y = vec![0f64; self.b2.len()];
            self.f7232[0].filter2(&self.b2, &mut y);
            for k in 1..4 {
                self.f7232[k].filter2_inplace(&mut y);
            }
            for v in y.iter_mut() {
                *v = v.abs();
            }
            self.f7456.filter1_inplace(&mut y);
            let v14f = v7 * 60.0 / v11;
            let ny = y.len() as i32;
            let last = self.bt2[nb - 1];
            let v15e = if last + v14f as i32 - 1 <= ny { last + v14f as i32 } else { ny };
            let mut b: Vec<i32> = self.bt2.clone();
            b.truncate(nb + 1);
            b.resize(nb + 1, 0);
            b[nb] = v15e;
            let e = ((v14f + 0.5).trunc().mul_add(0.125, 0.5)) as i32;
            let mut r = vec![0f64; 8 * nb];
            for k in 0..nb {
                let s = b[k];
                let st = [s, s + e + 1, s + 2 * e + 1, s + 3 * e + 1, s + 4 * e + 1, s + 5 * e + 1, s + 6 * e + 1, s + 7 * e + 1];
                let ln = [e + 1, e, e, e, e, e, e, b[k + 1] - (7 * e + 1 + s)];
                for j in 0..8 {
                    if ln[j] < 0 {
                        return None;
                    }
                    r[8 * k + j] = max_val(crate::phrase_rb_dsp::win(&y, st[j] as i64 - 1, ln[j] as i64));
                }
            }
            put(&mut self.g3416, 8 * done, &r);
        }
        let v69 = self.state;
        if v69 == 2 || v69 != 1 {
            return Some(());
        }
        // rebase on the carried tails
        self.b2 = if self.n2 <= v118 { vec![] } else { self.t2.clone() };
        self.b8 = if self.n8 <= v111 { vec![] } else { self.t8.clone() };
        self.b32 = if self.n32 <= v116 { vec![] } else { self.t32.clone() };
        self.n2 -= v118;
        self.n8 -= v111;
        self.n32 -= v116;
        self.state = 0;
        self.ms = 0;
        self.bar = 1;
        let v70 = (v121 + 1) as usize;
        let i = v23 as usize;
        let (v72, v73, v74) = (self.bt2[i], self.bt8[i], self.bt32[i]);
        for k in 0..v70 {
            self.bt2[k] -= v72;
            self.bt8[k] -= v73;
            self.bt32[k] -= v74;
        }
        let v95 = v70 as i32 - v23;
        for k in 0..v95.max(0) as usize {
            self.bt2[k] = self.bt2[k + i] + 1;
            self.bt8[k] = self.bt8[k + i] + 1;
            self.bt32[k] = self.bt32[k + i] + 1;
        }
        self.chunk += 1;
        Some(())
    }
}

/// memcpy of a per-chunk result into a global array so that it ends at `end`.
fn put(dst: &mut [f64], end: usize, src: &[f64]) {
    let s = end as i64 - src.len() as i64;
    for (k, v) in src.iter().enumerate() {
        let i = s + k as i64;
        if i >= 0 && (i as usize) < dst.len() {
            dst[i as usize] = *v;
        }
    }
}

fn putb(dst: &mut [u8], start: usize, src: &[u8]) {
    for (k, v) in src.iter().enumerate() {
        if let Some(d) = dst.get_mut(start + k) {
            *d = *v;
        }
    }
}
