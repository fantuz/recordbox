//! Stage A/B of the phrase analyser port: the first 210 seconds decimated by 8, a sixteenth-note
//! onset pattern per bar, snare candidates, and the few values stage D reads from it
//! (`Sub_Main_B` and the routines it calls, with the reference's bar-position corrections off,
//! which is how the stored analyses were made; see the trace notes).

use crate::phrase_rb_dsp::{at0, max_idx, max_val, min_val, prewarp, square_sum8, sum8, Filter};

/// What `Sub_Main_B` leaves in the analyser object for stage D.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageB {
    /// first beat (ms) and first-downbeat index, unchanged with corrections off (+512, +516)
    pub first_ms: i32,
    pub first_bar: i32,
    /// a one-bar pattern of a kick on 1 and on the "and" of 2 followed by silence was seen (+520)
    pub f520: i32,
    /// snare positions per two-beat pair and their levels (+528, +560)
    pub m528: Vec<i32>,
    pub m560: Vec<f64>,
    /// (+592, +596) counts from the snare search, (+600) beats before the music starts
    pub f592: i32,
    pub f596: i32,
    pub f600: i32,
}

/// `BeatsSamplesCalc`: 1-based sample index of each beat at rate `sr`.
pub fn beats_samples(n: usize, sr: f64, bpm: f64, first_ms: i32) -> Vec<i32> {
    let per = sr * 60.0 / bpm;
    let first = ((sr * 0.001).mul_add(first_ms as f64, 0.5)) as i32;
    let count = ((n as i32 - first) as f64 / per) as i32;
    let mut v: Vec<i32> = (0..count.max(0)).map(|i| per.mul_add(i as f64, first as f64 + 1.5) as i32).collect();
    if let Some(l) = v.last_mut() {
        if *l > n as i32 {
            *l = n as i32;
        }
    }
    v
}

/// State of `RhythmPatternAnalyze` (+608 … +920 of the analyser object).
#[derive(Clone, Debug, Default)]
pub struct RhythmState {
    pub last: f64,
    pub shift: i32,
    pub peak: f64,
    pub mode: i32,
    pub lp: [Filter; 4],
    pub env: Filter,
}

impl RhythmState {
    /// `RhythmPatternAnalyze_init(p0, p1, p2, sr)`.
    pub fn init(&mut self, last: f64, shift: i32, peak: f64, sr: f64) {
        self.last = last;
        self.shift = shift;
        self.peak = peak;
        let w = prewarp(1256.6370614359173, sr);
        self.mode = 0;
        let near = |a: f64, lo: f64, hi: f64| a > lo && a < hi;
        if near(sr, 5512.49, 5512.51) || near(sr, 5999.99, 6000.01) || near(sr, 3999.99, 4000.01) {
            self.mode = 4;
            self.lp[0].make2(0.0, 0.0, w * w, 1.0, w * 1.8477590650225735, w * w, sr);
            self.lp[0].clear();
            self.lp[2] = self.lp[0];
            self.lp[2].clear();
            self.lp[1].make2(0.0, 0.0, w * w, 1.0, w * 0.76536686473017967, w * w, sr);
            self.lp[1].clear();
            self.lp[3] = self.lp[1];
            self.lp[3].clear();
        } else if near(sr, 1378.115, 1378.135) || near(sr, 1499.99, 1500.01) || near(sr, 999.99, 1000.01) {
            self.mode = 2;
            self.lp[0].make2(0.0, 0.0, w * w, 1.0, w * 1.4142135623730951, w * w, sr);
            self.lp[0].clear();
        }
        let w2 = prewarp(62.831853071795862, sr);
        let r = if sr == 0.0 { 5512.5 } else { sr };
        self.env.make2(0.0, 0.0, w2 * w2, 1.0, w2 * 1.4142135623730951, w2 * w2, r);
        self.env.clear();
    }
}

/// `RhythmPatternAnalyze`: per bar, which of its 16 sixteenth-note positions has an onset
/// (low-passed 200 Hz envelope rises), after aligning the grid to the strongest onsets.
/// Returns None where the reference fails.
pub fn rhythm_pattern(st: &mut RhythmState, x: &[f64], bpm: f64, first_ms: i32, p3: i32, sr: f64, p5: i32) -> Option<Vec<u8>> {
    let v15 = sr * 60.0 / bpm + 0.5;
    let v16 = v15 as i32;
    if v16 == 0 {
        return None;
    }
    let n = x.len() as i32;
    let v178 = ((first_ms as f64 * 0.001).mul_add(sr, 0.5)) as i32;
    let v176 = (p3 - 1) * v16;
    let v18 = v176 + v178;
    let v19 = (n - v18) / v16;
    let v20 = if v19 >= 0 { v19 } else { v19 + 3 };
    let v22 = if p5 < 2 { f64::from_bits(0x3F1A36E2EB1C432D) } else { st.peak };
    let v181 = st.shift;
    let mut v23 = vec![0f64; n as usize];
    match st.mode {
        2 => st.lp[0].filter2(x, &mut v23),
        4 => {
            st.lp[0].filter2(x, &mut v23);
            st.lp[1].filter2_inplace(&mut v23);
            st.lp[2].filter2_inplace(&mut v23);
            st.lp[3].filter2_inplace(&mut v23);
        }
        _ => {}
    }
    for v in v23.iter_mut() {
        *v = v.abs();
    }
    st.env.filter2_inplace(&mut v23);
    let v25 = v15.trunc();
    let v26 = v20 >> 2;
    let v172 = v25 * 4.0;
    let v27 = (v25 * 4.0) as i32;
    let v28 = (v18 + v26 * v27 > n) as i32;
    let v29 = v26 - v28;
    if v29 <= 0 {
        st.last = at0(&v23, n as i64 - 1);
        st.shift = v181;
        st.peak = v22;
        return Some(vec![0u8; 1]);
    }
    let mut out = vec![0u8; 16 * v29 as usize];
    let v171 = v25 * 0.25;
    let v173 = (v25 * 0.25 + 0.5) as i32;
    let v30 = v18 + 1;
    let mut v62 = 0i32;
    if v29 >= 2 {
        // Search the strongest onsets for the grid offset.
        let v31 = v29 - 1;
        let v32 = v31 * v27;
        let s0 = (v18 + v27) as usize;
        let mut v184: Vec<f64> = v23.get(s0..s0 + v32 as usize)?.to_vec();
        let v185 = v184.len() as i32;
        let mut idx = [0i32; 8];
        let mut v162 = 0i32;
        let mut v34 = 0i32;
        let v153 = (5 * v16) / 4;
        let mut v36 = 0.0f64;
        let mut found = false;
        let mut v40 = 0;
        let a = |i: i32| at0(&v23, i as i64);
        'outer: for v33 in 0..8usize {
            if v33 != 0 {
                let v37 = idx[v33 - 1];
                if v37 <= v185 && v37 >= 1 {
                    v184[v37 as usize - 1] = 0.0;
                }
            }
            let (mut v38, mi) = max_idx(&v184);
            idx[v33] = mi as i32 + 1;
            let v39 = if v172 == 0.0 { 0xFFFF } else { (idx[v33] as f64 * 0.25 * (1.0 / v25)) as i32 };
            v40 = v27 + v27 * v39;
            let v41 = if v40 + v18 < n { v40 + v18 } else { n };
            let v42 = v40 + v18;
            let v43 = a(v41 - 1);
            if v43 <= v38 * 0.2 || v40 >= v185 - v27 {
                continue;
            }
            let v186_start = v18 - 1 + v40;
            let v186_len = v27 + 1;
            let v45 = a(v42);
            let v46 = a(v42 + 1);
            let mut brk: Option<(i32, i32)> = None; // (v49, v50)
            if v43 <= v45 {
                if v45 > v46 || v46 > a(v42 + 2) {
                    continue;
                }
                let mut v49 = 0i32;
                while v49 < v173 {
                    let v50 = v49 + 1;
                    if a(v42 + 1 + v49) - a(v42 + v49) < 0.0 {
                        brk = Some((v49, v50));
                        break;
                    }
                    v49 += 1;
                }
            } else {
                if v45 <= v46 || v46 <= a(v42 + 2) {
                    continue;
                }
                let mut v47 = 0i32;
                if v173 >= 1 {
                    loop {
                        if a(v42 + v47 - 3) - a(v42 + v47 - 2) < 0.0 {
                            brk = Some((-v47, 1 - v47));
                            break;
                        }
                        v47 -= 1;
                        if -v173 == v47 {
                            break;
                        }
                    }
                }
            }
            if let Some((v49, v50)) = brk {
                let v51 = if v43 > v45 { !v49 } else { v50 };
                v34 = v51 + v40;
                v36 = a(v18 - 1 + v51 + v40);
            }
            if v36 > v38 * 0.595662 {
                let w = crate::phrase_rb_dsp::win(&v23, v186_start as i64, v186_len as i64);
                let v52 = min_val(w);
                if 5 * v16 >= 4 {
                    let mut v53 = false;
                    let mut v54 = 0i32;
                    let v55 = (v52 + v38) * 0.5;
                    let mut v56 = -1i32;
                    let mut v57 = v153;
                    let mut v58 = v178 + v176 - 2;
                    let mut v59 = v178 + v176 - 3;
                    let lim = (v25 * 0.0625) as i32;
                    loop {
                        let mut stop = false;
                        if !v53 {
                            let v60 = a(v34 + v58);
                            if a(v34 + v59) >= v60 && v60 < v55 {
                                v162 = v34 + v56;
                                v53 = true;
                                v38 = v60;
                            } else {
                                v53 = false;
                            }
                        } else if a(v34 + v58) <= v38 * 1.12 {
                            v54 += 1;
                            if v54 < lim {
                                v53 = true;
                            } else {
                                stop = true;
                            }
                        } else {
                            v54 = 0;
                            v53 = false;
                        }
                        if stop {
                            if v38 == 0.0 {
                                break 'outer;
                            }
                            if v36 > v38 * 2.11 {
                                found = true;
                                break 'outer;
                            }
                            continue 'outer;
                        }
                        v59 -= 1;
                        v58 -= 1;
                        v56 -= 1;
                        v57 -= 1;
                        if v57 == 0 {
                            continue 'outer;
                        }
                    }
                }
            }
        }
        if found {
            let t = (v40 as f64 + 0.5 - v162 as f64) as i32;
            v62 = if v30 >= t { t } else { v18 + 1 };
        }
    }
    let v65 = v18 + v19 * v16;
    st.shift = v62;
    let v66 = if p5 <= 1 { v62 } else { v181 };
    for i in (1..v23.len()).rev() {
        v23[i] -= v23[i - 1];
    }
    let v69 = v65 - v30;
    if !v23.is_empty() {
        v23[0] -= st.last;
    }
    let nn = v23.len() as i32;
    if v30 > v66 && v65 - v66 <= nn {
        let s = (v30 - v66 - 1) as usize;
        let len = (v69 + 1).max(0) as usize;
        v23 = v23.get(s..s + len)?.to_vec();
    } else {
        let v71 = v65 - v66 - 1;
        if !(v71 > nn || v30 >= v66) {
            let v187: Vec<f64> = v23[..v71.max(0) as usize].to_vec();
            if nn < v69 {
                v23 = vec![0.0; v69 as usize];
            }
            let z = (v66 - v18).max(0) as usize;
            for v in v23.iter_mut().take(z) {
                *v = 0.0;
            }
            for (k, v) in v187.iter().enumerate() {
                if let Some(d) = v23.get_mut(z + k) {
                    *d = *v;
                }
            }
        }
    }
    // Per bar, up to six strongest of the 16 sixteenth-note maxima become onsets.
    let mut v167 = 0.0001f64;
    let v168 = v25 * 0.75;
    let v170 = v25 * 0.5;
    let v155 = v25 + v25;
    let v156 = v25;
    let v154 = v25 * 3.0;
    let m = |p: f64| -> f64 { max_val(crate::phrase_rb_dsp::win(&v23, p as i32 as i64 - 1, v173 as i64)) };
    for v79 in 0..v29 {
        let v85 = v172.mul_add(v79 as f64, 1.0);
        let v86 = v85 + v156;
        let v87 = v85 + v155;
        let v92 = v85 + v154;
        let mut v = [
            m(v85),
            m(v85 + v171),
            m(v85 + v170),
            m(v85 + v168),
            m(v86),
            m(v86 + v171),
            m(v86 + v170),
            m(v86 + v168),
            m(v87),
            m(v87 + v171),
            m(v87 + v170),
            m(v87 + v168),
            m(v92),
            m(v92 + v171),
            m(v92 + v170),
            m(v92 + v168),
        ];
        let mut v128 = 0.0;
        for it in 0..6 {
            let mut mx = v[0];
            let mut arg = 0usize;
            for (j, x) in v.iter().enumerate().skip(1) {
                if mx < *x {
                    arg = j;
                }
                mx = mx.max(*x);
            }
            if it == 0 {
                v128 = mx;
            }
            if mx > 0.0001 && mx > v128 * 0.596 && mx > v167 * 0.178 {
                out[arg + 16 * v79 as usize] = 1;
                v[arg] = 0.0;
            }
        }
        v167 = v167.max(v128);
    }
    st.peak = v167;
    Some(out)
}

/// `LowPowerAnalyze16` → per-bar low-band (100 Hz) energy (+1008), from quarter-beat energies.
pub fn low_power16(lp: &mut Filter, x: &[f64], bpm: f64, first_ms: i32, sr: f64, beats: &[i32]) -> Option<Vec<f64>> {
    let _ = first_ms;
    let mut v61 = vec![0f64; x.len()];
    lp.filter2(x, &mut v61);
    let p5 = beats.len();
    let mut q = vec![0f64; 4 * p5];
    let v15 = sr * 60.0 / bpm;
    let v16 = (v15.mul_add(0.25, 0.5)) as i32;
    let v59 = (v15 + 0.5) as i32 - 1;
    let w = |s: i32, len: i32| -> Option<f64> {
        if len < 0 {
            return None;
        }
        Some(square_sum8(crate::phrase_rb_dsp::win(&v61, s as i64 - 1, len as i64)))
    };
    for k in 0..p5 {
        let v18 = beats[k];
        let v19 = if k + 2 >= p5 { v59 + v18 } else { beats[k + 1] };
        q[4 * k] = w(v18, v16)?;
        let v20 = v18 + v16;
        q[4 * k + 1] = w(v20, v16)?;
        let v21 = v20 + v16;
        q[4 * k + 2] = w(v21, v16)?;
        q[4 * k + 3] = w(v21 + v16, v19 - (v21 + v16))?;
    }
    if sr <= 5512.45 || sr >= 5512.55 {
        for v in q.iter_mut() {
            *v *= 4.0;
        }
    }
    let bars = p5 / 4;
    let mut out = vec![0f64; bars];
    for (b, o) in out.iter_mut().enumerate() {
        let mut s = 0.0;
        for j in 0..16 {
            s += q[16 * b + j];
        }
        *o = s;
    }
    Some(out)
}

/// `Detect4KicksPopRockRhythm`, the one output stage B uses: true when two or more groups of
/// eight bars contain a bar of four-on-the-floor kicks (or a pure off-beat pattern).
pub fn four_kicks(rhythm: &[u8], bar_power: &[f64]) -> bool {
    let n = bar_power.len() as i32;
    let mut v17 = 1i32;
    if n != 0 {
        let mean = sum8(bar_power) / n as f64;
        v17 = if n < 17 { 1 } else { 17 };
        if v17 <= n - 3 {
            let mut v19 = v17;
            loop {
                if bar_power[(v19 - 1) as usize] > mean {
                    v17 = v19;
                    break;
                }
                v19 += 1;
                if v19 == n - 2 {
                    break;
                }
            }
        }
    }
    let v20 = (rhythm.len() as f64 * 0.063) as i32 - v17;
    let v21 = if v20 >= 64 { 8 } else { v20 / 8 };
    if v20 < 1 || v21 <= 0 {
        return false;
    }
    let pat = |bar: usize, ones: &[usize]| -> bool {
        (0..16).all(|j| {
            let b = rhythm.get(bar * 16 + j).copied().unwrap_or(0);
            if ones.contains(&j) {
                b == 1
            } else {
                b == 0
            }
        })
    };
    let mut groups = 0;
    for g in 0..v21 {
        let mut hit = false;
        for k in 0..8i32 {
            let x = k + 1;
            if x & 0x7FFFFFFA == 2 {
                let bar = (v17 - 1) + 8 * g + k;
                if bar >= 1 && (pat(bar as usize, &[2, 6, 10, 14]) || pat(bar as usize, &[0, 4, 8, 12])) {
                    hit = true;
                }
            }
        }
        groups += hit as i32;
    }
    groups > 1
}

/// `DetectSnarePositionsFilter` (+1040): 300 Hz high-pass (8th order), rectified, 5 Hz
/// envelope, first difference.
pub fn snare_filter(x: &[f64], sr: f64) -> Vec<f64> {
    let sr = if sr == 0.0 { 5512.5 } else { sr };
    let w = prewarp(1884.9555921538758, sr);
    let mut f = [Filter::default(); 4];
    f[0].make2(1.0, 0.0, 0.0, 1.0, w * 1.8477590650225735, w * w, sr);
    f[0].clear();
    f[2] = f[0];
    f[1].make2(1.0, 0.0, 0.0, 1.0, w * 0.76536686473017967, w * w, sr);
    f[1].clear();
    f[3] = f[1];
    let w1 = prewarp(31.415926535897931, sr);
    let mut e = Filter::default();
    e.make1(0.0, w1, 1.0, w1, sr);
    e.clear();
    let mut y = vec![0f64; x.len()];
    f[0].filter2(x, &mut y);
    f[1].filter2_inplace(&mut y);
    f[2].filter2_inplace(&mut y);
    f[3].filter2_inplace(&mut y);
    for v in y.iter_mut() {
        *v = v.abs();
    }
    e.filter1_inplace(&mut y);
    for i in (1..y.len()).rev() {
        y[i] -= y[i - 1];
    }
    if let Some(v) = y.first_mut() {
        *v = 0.0;
    }
    y
}

/// One bar of the snare envelope sorted by level (descending, stable) with the 1-based sample
/// offsets in the bar (what `DetectSnarePositions` leaves in its per-bar arrays).
pub struct SortedBar {
    pub val: Vec<f64>,
    pub idx: Vec<i32>,
}

pub fn sorted_bars(env: &[f64], beats: &[i32]) -> Option<Vec<SortedBar>> {
    let nb = beats.len() / 8;
    let mut out = Vec::with_capacity(nb);
    for k in 0..nb {
        let s = beats[4 * k];
        let e = *beats.get(4 * k + 4)?;
        let len = e - s;
        if len <= 0 {
            return None;
        }
        let w = crate::phrase_rb_dsp::win(env, s as i64 - 1, len as i64);
        let mut p: Vec<(f64, i32)> = w.iter().enumerate().map(|(i, v)| (*v, i as i32 + 1)).collect();
        p.sort_by(|a, b| if a.0 > b.0 { std::cmp::Ordering::Less } else if b.0 > a.0 { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal });
        out.push(SortedBar { val: p.iter().map(|x| x.0).collect(), idx: p.iter().map(|x| x.1).collect() });
    }
    Some(out)
}

/// Stable ascending sort of an int array, returning the 1-based original positions.
pub fn sort_ascend_idx(v: &mut [i32]) -> Vec<i32> {
    let mut p: Vec<(i32, i32)> = v.iter().enumerate().map(|(i, x)| (*x, i as i32 + 1)).collect();
    p.sort_by(|a, b| a.0.cmp(&b.0));
    for (d, s) in v.iter_mut().zip(&p) {
        *d = s.0;
    }
    p.iter().map(|x| x.1).collect()
}

/// `DetectSnarePositionsForGenre` → (+528, +560, +592, +596).
#[allow(clippy::too_many_arguments)]
pub fn snare_for_genre(sr: f64, bpm: f64, first_ms: i32, beats: &[i32], p6: i32, p7: bool, rhythm: &[u8], bars: &[SortedBar]) -> Option<(Vec<i32>, Vec<f64>, i32, i32)> {
    let v25 = if p6 >= 0 { p6 } else { p6 + 7 };
    let v26 = v25 & !7;
    let nn = v26.max(0) as usize;
    let mut loc = vec![0i32; nn];
    let mut v28 = vec![0f64; nn];
    let v29 = if bpm != 0.0 { bpm } else { 120.0 };
    let p1 = if sr == 0.0 { 5512.5 } else { sr };
    let v30 = p1 * 60.0 / v29;
    let v31 = v30.mul_add(0.25, 0.5) as i32;
    if p6 >= 8 {
        let mut v33 = 0usize;
        let mut v34 = 0usize;
        let v36 = v31 - 20;
        for v32 in 0..(v25 >> 3) as usize {
            let b = bars.get(v32)?;
            let v40 = b.val.len();
            if v40 >= 2 {
                let v42 = beats[4 * v32];
                let mut v41 = 0usize;
                loop {
                    let mut check = v33 < 1;
                    if !check {
                        let v43 = b.idx[v41] + v42;
                        check = !loc[..v33].iter().any(|l| !(v43 < l - v36 || v43 > l + v36));
                    }
                    if check && v34 < nn {
                        let v49 = b.val[v41];
                        let v48 = b.val[v41 + 1];
                        if v49 * 0.9886 > v48 {
                            loc[v34] = v42 - 1 + b.idx[v41];
                            v28[v34] = v49;
                            v34 += 1;
                            v33 += 1;
                        } else if v41 + 3 > v40 || v48 * 0.9886 <= b.val[v41 + 2] {
                            break;
                        }
                    }
                    v41 += 1;
                    if v41 == v40 - 1 {
                        break;
                    }
                }
            }
        }
    }
    let perm = sort_ascend_idx(&mut loc);
    let mut v54: Vec<f64> = vec![0.0; nn];
    for (k, p) in perm.iter().enumerate() {
        v54[k] = v28[*p as usize - 1];
    }
    let mut p10 = loc.clone();
    let mut p11 = v54.clone();
    let ms = ((p1 * 0.001).mul_add(first_ms as f64, 0.5)) as i32;
    if p6 >= 8 {
        let v58 = (v30 + v30 + 0.5) as i32;
        let v59 = (v30 * 4.0 + 0.5) as i32;
        let v60 = (v30 * 0.125 + 0.5) as i32;
        let v61 = ms;
        let v62 = (v26.max(1)) as usize;
        let lc = |i: usize, loc: &Vec<i32>| loc.get(i).copied().unwrap_or(0);
        for v57 in 0..v62 {
            let v65 = lc(v57, &loc);
            if v65 == 0 {
                continue;
            }
            let (v67, v68) = (v65 - v58, v65 + v58);
            let mut v69 = true;
            for v66 in 0..v62 {
                let v70 = lc(v66, &loc);
                if v70 != 0 && ((v67 >= v70 - v60 && v67 <= v70 + v60) || (v68 >= v70 - v60 && v68 <= v70 + v60)) {
                    v69 = true;
                    break;
                }
                v69 = ((v66 + 1) as i32) < v26;
            }
            let (v76, v77) = (v65 - v59, v65 + v59);
            let mut v78 = true;
            for v75 in 0..v62 {
                let v79 = lc(v75, &loc);
                if v79 != 0 && ((v76 >= v79 - v31 && v76 <= v79 + v60) || (v77 >= v79 - v31 && v77 <= v79 + v60)) {
                    v78 = true;
                    break;
                }
                v78 = ((v75 + 1) as i32) < v26;
            }
            if v69 {
                let v84 = (v65 - v61) / v31;
                let r = |i: i32| rhythm.get(i as usize).copied().unwrap_or(0);
                if v84 >= 6 && v84 + 5 < rhythm.len() as i32 {
                    let v85 = r(v84 - 4);
                    if (v85 != 1 || r(v84 + 2) != 1) && (r(v84 - 6) != 1 || r(v84 + 5) != 1) && (v85 != 1 || r(v84 + 4) != 1) {
                        loc[v57] = 0;
                        v54[v57] = 0.0;
                    } else if r(v84) == 1 {
                        loc[v57] = 0;
                        v54[v57] = 0.0;
                    }
                }
            } else {
                loc[v57] = 0;
                v54[v57] = 0.0;
                if !v78 {
                    p10[v57] = 0;
                    p11[v57] = 0.0;
                }
            }
        }
        if !p7 {
            let n = rhythm.len() as i32;
            for k in 0..v62.min(p10.len()) {
                if p10[k] != 0 {
                    let v107 = (p10[k] - v61 - 1) / v31;
                    if v107 >= 0 && v107 + 1 < n && rhythm[v107 as usize] == 1 {
                        p10[k] = 0;
                    }
                }
            }
        }
    }
    let _ = sort_ascend_idx(&mut loc);
    let perm2 = sort_ascend_idx(&mut p10);
    let mut v139 = vec![0f64; nn];
    for (k, p) in perm2.iter().enumerate() {
        v139[k] = p11[*p as usize - 1];
    }
    let p11 = v139;
    let (v98, v99);
    if p6 <= 7 {
        v98 = 0usize;
        v99 = 0usize;
    } else {
        let mx = (v26.max(1)) as usize;
        v98 = loc.iter().position(|x| *x != 0).unwrap_or(mx);
        v99 = p10.iter().position(|x| *x != 0).unwrap_or(mx);
    }
    let f592 = v26 - v99 as i32;
    let v108 = v99 as i32 + 1;
    let _ = v98;
    if v26 >= v108 {
        for k in (v108 - 1) as usize..nn {
            p10[k] -= ms;
        }
    }
    Some((p10, p11, f592, v108))
}

/// The part of `BeatposCorrection2016` stage D reads (+520): whether some position of the
/// onset pattern starts a kick on 1 and on the "and" of 2 followed by nine silent sixteenths.
pub fn beatpos_flag(rhythm: &[u8], nbeats: i32) -> i32 {
    let v29 = if nbeats >= 0 { nbeats } else { nbeats + 7 };
    if nbeats < 16 || (rhythm.len() as f64 * 0.03125) as i32 <= 1 {
        return 0;
    }
    let v30 = v29 >> 3;
    let v38 = (16 * v30).max(1) as usize;
    const REF1: [u8; 7] = [1, 0, 0, 0, 0, 0, 1];
    for v34 in 0..v38 {
        let a = rhythm.get(v34..v34 + 7);
        let b = rhythm.get(v34 + 7..v34 + 16);
        if a == Some(&REF1[..]) && b.map(|b| b.iter().all(|x| *x == 0)).unwrap_or(false) {
            return 1;
        }
    }
    0
}

/// `BarPositionJudge` (+600): beats from the first beat to the first audible sample, + 1.
pub fn bar_position(x: &[f64], bpm: f64, first_ms: i32, sr: f64) -> i32 {
    let v16 = if bpm == 0.0 { 0.5 } else { 60.0 / bpm };
    let v17 = v16.mul_add(sr, 0.5) as i32;
    let n = x.len() as i64;
    let mut v18 = 0.0;
    if n >= 2 {
        let v20 = ((first_ms as f64 * 0.001).mul_add(sr, 0.5)) as i32 as i64;
        let mut v19: i64 = 0;
        loop {
            let i = v20 + v19;
            if !(i as u32 as i64 >= n || i < 0 || x[i as usize].abs() <= 0.0005) {
                v18 = (v19 + 1) as f64;
                break;
            }
            v19 += 1;
            if v19 == n - 1 {
                break;
            }
        }
    }
    let v22 = if v17 == 0 { v18 * 0.00036281179138321996 } else { v18 / v17 as f64 };
    (v22 + 0.5) as i32 + 1
}

/// `Sub_Main_B` with corrections off. `dec` is stage A's decimated audio (at most 210 s).
#[cfg(test)]
#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
pub fn sub_main_b(dec: &[f64], sr2: f64, bpm: f64, first_ms: i32, first_bar: i32) -> Option<StageB> {
    sub_main_b_cancellable(dec, sr2, bpm, first_ms, first_bar, &|| false)
}

pub fn sub_main_b_cancellable(dec: &[f64], sr2: f64, bpm: f64, first_ms: i32, first_bar: i32, cancelled: &dyn Fn() -> bool) -> Option<StageB> {
    if cancelled() { return None; }
    let n210 = dec.len().min((sr2 * 210.0) as i32 as usize);
    let p0a = &dec[..n210];
    let beats = beats_samples(p0a.len(), sr2, bpm, first_ms);
    let p6 = beats.len() as i32;
    let mut rs = RhythmState::default();
    rs.init(0.0, 0, 0.0001, sr2);
    let rhythm = rhythm_pattern(&mut rs, p0a, bpm, first_ms, 1, sr2, 1)?;
    if cancelled() { return None; }
    let mut lp = Filter::default();
    let w = prewarp(628.31853071795865, sr2);
    lp.make2(0.0, 0.0, w * w, 1.0, w * 1.4142135623730951, w * w, sr2);
    lp.clear();
    let bar_power = low_power16(&mut lp, dec, bpm, first_ms, sr2, &beats)?;
    let p3 = four_kicks(&rhythm, &bar_power);
    if cancelled() { return None; }
    let env = snare_filter(p0a, sr2);
    let bars = sorted_bars(&env, &beats)?;
    if beats.len() / 8 == 0 {
        return None;
    }
    if cancelled() { return None; }
    let (m528, m560, f592, f596) = snare_for_genre(sr2, bpm, first_ms, &beats, p6, p3, &rhythm, &bars)?;
    let f520 = beatpos_flag(&rhythm, p6);
    let n30 = dec.len().min((sr2 * 30.0) as i32 as usize);
    let f600 = bar_position(&dec[..n30], bpm, first_ms, sr2);
    Some(StageB { first_ms, first_bar, f520, m528, m560, f592, f596, f600 })
}
