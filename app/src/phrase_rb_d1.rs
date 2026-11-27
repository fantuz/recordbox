//! Stage D of the phrase analyser port, part 1: per-bar summaries of stage C's features
//! (`IntroEndFind_b`, `*_b` level functions, `UtaUmuAnalyze_a` with `findpeaks_ex`).

use crate::phrase_rb_c::Mat2;
use crate::phrase_rb_dsp::{max_val, min_val, prewarp, Filter};

#[cfg(target_os = "macos")]
extern "C" {
    fn __exp10(x: f64) -> f64;
}

/// The platform's exp10 (what the reference calls).
#[inline]
pub fn exp10(x: f64) -> f64 {
    #[cfg(target_os = "macos")]
    unsafe { __exp10(x) }
    #[cfg(not(target_os = "macos"))]
    { 10.0f64.powf(x) }
}

/// `MatArrayFloat::sum(a, b)`: inclusive range, 0 when out of range.
pub fn sum_range(x: &[f64], a: i32, b: i32) -> f64 {
    if a < 0 || a > b || b >= x.len() as i32 {
        return 0.0;
    }
    crate::phrase_rb_dsp::sum8(&x[a as usize..=b as usize])
}

/// `IntroEndFind_b` → (flags1, flags2, per-beat level). Normalises `chroma` in place.
pub fn intro_end_find_b(chroma: &mut Mat2, frames: i32, first: i32, nbeats: i32) -> (Vec<u8>, Vec<u8>, Vec<f64>) {
    let mx = max_val(&chroma.d);
    if !(mx > 0.0) {
        return (vec![0], vec![0], vec![0.0]);
    }
    let inv = 1.0 / mx;
    for v in chroma.d.iter_mut() {
        *v *= inv;
    }
    let p3 = frames.max(0) as usize;
    let cols = chroma.cols;
    let at = |r: usize, c: usize| chroma.d[r * cols + c];
    let mut cnt = vec![0i32; p3];
    let mut pk = Mat2::zeros(72, p3);
    for c in 0..p3.min(cols) {
        let mut rising = false;
        let mut inpk = false;
        let mut top = at(0, c);
        for i in 1..72 {
            let v41 = at(i, c);
            let v42 = at(i - 1, c);
            let up = v41 > v42;
            if v41 <= v42 {
                if rising {
                    if top < at(i - 1, c) {
                        top = at(i - 1, c);
                        inpk = true;
                    }
                } else if inpk {
                    if at(i, c) >= top * 0.9 {
                        inpk = true;
                    } else {
                        if at(i, c) <= 0.001 {
                            inpk = false;
                        } else {
                            let k = cnt[c] as usize;
                            cnt[c] += 1;
                            inpk = false;
                            pk.d[k * p3 + c] = top;
                        }
                        top = 0.0;
                    }
                } else {
                    inpk = false;
                }
            }
            rising = up;
        }
    }
    let v45 = cnt.iter().copied().max().unwrap_or(0);
    let v46 = if v45 != 0 { v45 } else { 1 } as usize;
    let mut v47 = vec![0i32; p3];
    for c in 0..p3 {
        for j in 0..v46.min(72) {
            if pk.d[j * p3 + c] > 0.1 {
                v47[c] += 1;
            }
        }
    }
    let v118 = nbeats;
    let v51 = (v118 / 4).max(0) as usize;
    let mut v124 = vec![0f64; v51];
    if v118 >= 4 {
        let mut v12 = first as i64;
        let mut v55 = 0i64;
        for v54 in 0..v51 {
            let v57 = ((p3 as f64 * 4.0 * (v54 + 1) as f64).mul_add(1.0 / v118 as f64, 0.5)) as i32 as i64;
            if v55 < v57 {
                let mut s = v124[v54];
                for _ in 0..(v57 - v55) {
                    s += v47.get(v12 as usize).copied().unwrap_or(0) as f64;
                    v12 += 1;
                }
                v124[v54] = s;
            }
            v55 = v12;
        }
    }
    let v77 = max_val(&v124) as i32;
    let v78 = if v77 != 0 { v77 } else { 1 } as f64;
    let inv = 1.0 / v78;
    let v127: Vec<f64> = v124.iter().map(|v| v * inv).collect();
    let mut f1 = vec![0u8; 4 * v51];
    let mut f2 = vec![0u8; 4 * v51];
    if v118 >= 4 {
        if v127[0] > 0.36 {
            f1[0] = 1;
        }
        for k in 1..v51 {
            if v127[k] > 0.36 {
                f1[k] = 1;
            }
            let set = if f1[k - 1] >= f1[k] { !(v127[k] <= 0.2 || v127[k] / v127[k - 1] <= 1.8) } else { !(v127[k - 1] * 1.2 >= v127[k]) };
            if set {
                f2[k] = 1;
            }
        }
    }
    let mut lvl = vec![0f64; 4 * v51];
    for k in 0..v51 {
        for j in 0..4 {
            lvl[4 * k + j] = v127[k];
        }
    }
    (f1, f2, lvl)
}

/// `LowLevelAnalyze_b` → (per-bar sums, rise/fall flags).
pub fn low_level_b(x: &[f64], bar: i32) -> (Vec<f64>, Vec<u8>) {
    let v10 = x.len() as i32 - bar;
    let v11 = v10 + 4;
    let v12 = if v11 >= 0 { v11 } else { v10 + 7 };
    let v13 = (v12 >> 2).max(0) as usize;
    let mut s = vec![0f64; v13];
    if v11 >= 12 {
        let mut v16 = bar - 1;
        let v17 = v13.max(3);
        for k in 0..v17 - 2 {
            if let Some(d) = s.get_mut(k) {
                *d = sum_range(x, v16, v16 + 3);
            }
            v16 += 4;
        }
    }
    let mut f = vec![0u8; v13];
    if v11 >= 12 {
        let v21 = v13.max(3);
        for k in 1..v21 - 1 {
            let (v28, v27) = (s.get(k - 1).copied().unwrap_or(0.0), s.get(k).copied().unwrap_or(0.0));
            let set = if v27 > v28 { v27 > v28 * 1.2 } else { !(v27 >= v28 || v28 <= v27 * 1.2) };
            if set {
                if let Some(d) = f.get_mut(k) {
                    *d = 1;
                }
            }
            let a = crate::phrase_rb_dsp::at0(x, 4 * k as i64);
            let b = crate::phrase_rb_dsp::at0(x, 4 * k as i64 - 1);
            if v27 <= 750.0 && v28 <= 750.0 && a <= 0.09 && b <= 0.09 {
                if let Some(d) = f.get_mut(k) {
                    *d = 0;
                }
            }
        }
    }
    (s, f)
}

/// `MiddleLevelAnalyze_b` → (change flags, per-bar sums).
pub fn middle_level_b(x: &[f64], bar: i32) -> (Vec<u8>, Vec<f64>) {
    let v9 = x.len() as i32 - bar;
    let v10 = v9 + 4;
    let v11 = if v10 >= 0 { v10 } else { v9 + 7 };
    let v12 = (v11 >> 2).max(0) as usize;
    let mut p3 = vec![0f64; v12];
    let v15 = v12 as i32 - 1;
    if v10 >= 8 {
        let mut v16 = bar - 1;
        for k in 0..v15.max(1) as usize {
            if let Some(d) = p3.get_mut(k) {
                *d = sum_range(x, v16, v16 + 3);
            }
            v16 += 4;
        }
    }
    let mut f = vec![0u8; v12];
    if v10 >= 12 {
        let v20 = v15.max(2) as usize;
        for k in 1..v20 {
            let mut cur = p3.get(k).copied().unwrap_or(0.0);
            let mut prev = p3.get(k - 1).copied().unwrap_or(0.0);
            if cur == 0.0 {
                cur = 0.003;
            }
            if prev == 0.0 {
                prev = 0.003;
            }
            let set = if cur > prev { cur / prev > 1.2 } else { !(cur >= prev || prev / cur <= 1.21) };
            if set {
                if let Some(d) = f.get_mut(k) {
                    *d = 1;
                }
            }
        }
    }
    (f, p3)
}

/// `MiddlePeakAnalyze_b`: normalises the first nbeats−1 values of `x` in place, bar means.
pub fn middle_peak_b(x: &mut [f64], nbeats: i32, bar: i32) -> Vec<f64> {
    let mx = max_val(x);
    let v13 = if mx == 0.0 { 333.33333333333331 } else { 1.0 / mx };
    if nbeats >= 2 {
        for v in x.iter_mut().take((nbeats - 1) as usize) {
            *v *= v13;
        }
    }
    let v24 = x.len() as i32 - bar;
    let v25 = v24 + 1;
    let v26 = if v25 >= 0 { v25 } else { v24 + 4 };
    let v27 = (v26 >> 2).max(0) as usize;
    let mut out = vec![0f64; v27];
    if v25 >= 8 {
        let mut v30 = bar - 1;
        for k in 0..v27.max(2) - 1 {
            if let Some(d) = out.get_mut(k) {
                *d = sum_range(x, v30, v30 + 3) * 0.25;
            }
            v30 += 4;
        }
    }
    out
}

/// `AllLevelAnalyze_b` → (bar means of the normalised copy, normalised copy).
pub fn all_level_b(x: &[f64], bar: i32) -> (Vec<f64>, Vec<f64>) {
    let mut v11 = 1.0 / max_val(x);
    if v11 == 0.0 {
        v11 = 0.003;
    }
    let p3: Vec<f64> = x.iter().map(|v| v * v11).collect();
    let v21 = x.len() as i32 - bar;
    let v22 = v21 + 4;
    let v23 = if v22 >= 0 { v22 } else { v21 + 7 };
    let v24 = (v23 >> 2).max(0) as usize;
    let mut p2 = vec![0f64; v24];
    if v22 >= 8 {
        let mut v27 = bar - 1;
        for k in 0..v24.max(2) - 1 {
            if let Some(d) = p2.get_mut(k) {
                *d = sum_range(&p3, v27, v27 + 3) * 0.25;
            }
            v27 += 4;
        }
    }
    (p2, p3)
}

/// `HiHatAnalyze_b`: hi-hat density per bar (0/4/8/16), short deviations smoothed, change flags.
pub fn hihat_b(x: &[u8], bar: i32) -> Vec<u8> {
    let v8 = x.len() as i32 - bar;
    let v9 = if v8 + 4 < 0 { v8 + 7 } else { v8 + 4 };
    let v10 = (v9 >> 2).max(0) as usize;
    let mut out = vec![0u8; v10];
    if v8 < 0 {
        return out;
    }
    let mut sums = vec![0u8; v10];
    for i in 0..=(v8 as usize) {
        let v = x.get((bar - 1) as usize + i).copied().unwrap_or(0);
        if let Some(d) = sums.get_mut(i >> 2) {
            *d = d.wrapping_add(v);
        }
    }
    let mut lv = vec![0u8; v10.max(1)];
    for k in 0..v10.max(1) {
        let c = sums.get(k).copied().unwrap_or(0);
        lv[k] = if c <= 2 {
            0
        } else if c > 6 {
            if c > 11 {
                16
            } else {
                8
            }
        } else {
            4
        };
    }
    let g = |lv: &Vec<u8>, i: usize| lv.get(i).copied().unwrap_or(0);
    if v8 >= 8 {
        let v20 = v10.max(3) - 1;
        loop {
            let mut changed = false;
            let mut k = 1;
            while k < v20 {
                let a = g(&lv, k - 1);
                if a == g(&lv, k + 1) && g(&lv, k) != a {
                    lv[k] = a;
                    changed = true;
                }
                k += 1;
            }
            if !changed {
                break;
            }
        }
        if v8 >= 12 {
            let v27 = v10.max(4) - 2;
            loop {
                let mut changed = false;
                let mut k = 1;
                while k < v27 {
                    let a = g(&lv, k - 1);
                    if a == g(&lv, k + 2) && g(&lv, k) != a && g(&lv, k + 1) != a {
                        lv[k] = a;
                        lv[k + 1] = a;
                        changed = true;
                    }
                    k += 1;
                }
                if !changed {
                    break;
                }
            }
            if v8 >= 16 {
                let v33 = v10.max(5) - 3;
                loop {
                    let mut changed = false;
                    let mut k = 1;
                    while k < v33 {
                        let a = g(&lv, k - 1);
                        if !(a != g(&lv, k + 3) || g(&lv, k) == a || g(&lv, k + 1) == a) && g(&lv, k + 2) != a {
                            lv[k] = a;
                            lv[k + 1] = a;
                            lv[k + 2] = a;
                            changed = true;
                        }
                        k += 1;
                    }
                    if !changed {
                        break;
                    }
                }
            }
        }
    }
    if v8 < 4 {
        return out;
    }
    for k in 1..v10.max(2) {
        if g(&lv, k - 1) != g(&lv, k) {
            if let Some(d) = out.get_mut(k) {
                *d = 1;
            }
        }
    }
    out
}

/// `HighPeakAnalyze32_b` → (normalised copy shifted to the first bar, beats after the bar).
pub fn high_peak32_b(x: &[f64], bar: i32) -> Option<(Vec<f64>, i32)> {
    let mut p2 = x.to_vec();
    let n = x.len() as i32;
    let v12 = 8 * bar - 8;
    let v13 = n - v12;
    if v13 < 0 || v12 < 0 {
        return None;
    }
    let v15 = max_val(&p2[v12 as usize..]);
    let v18 = if v15 == 0.0 { 333.33333333333331 } else { 1.0 / v15 };
    if v13 >= 25 {
        for i in 0..(v13 - 24) as usize {
            p2[i] = p2[i + v12 as usize] * v18;
        }
    }
    Some((p2, v13 / 8))
}

/// `findpeaks_ex` → (values, 1-based locations, widths at half prominence, prominences).
pub fn findpeaks(x: &[f64]) -> (Vec<f64>, Vec<i32>, Vec<f64>, Vec<f64>) {
    let n = x.len() as i64;
    let mut val = vec![];
    let mut loc: Vec<i32> = vec![];
    if n >= 2 {
        let mut st = 0;
        for i in 0..(n - 1) as usize {
            let d = x[i] - x[i + 1];
            let ns = if d < 0.0 {
                1
            } else {
                if st == 1 {
                    loc.push(i as i32 + 1);
                    val.push(x[i]);
                }
                2 * (d > 0.0) as i32
            };
            st = ns;
        }
    }
    if loc.is_empty() {
        return (vec![0.0], vec![0], vec![0.0], vec![0.0]);
    }
    let k = loc.len();
    let mut prom = vec![0f64; k];
    let (mut v26, mut v27) = (0i64, 0i64);
    let xs = |i: i64| -> f64 { crate::phrase_rb_dsp::at0(x, i) };
    for p in 0..k {
        let v29 = val[p];
        let v30 = loc[p] as i64;
        if v30 >= 1 {
            let mut v31 = v30 - 1;
            let mut edge = false;
            while v29 >= xs(v31) {
                if v31 == 0 {
                    v27 = 1;
                }
                let v32 = v31 + 1;
                v31 -= 1;
                if v32 <= 1 {
                    edge = true;
                    break;
                }
            }
            if !edge {
                v27 = v31 + 1;
            }
        }
        let v33 = min_val(crate::phrase_rb_dsp::win(x, v27 - 1, v30 - v27 + 1));
        let v34 = loc[p] as i64;
        if v34 <= n {
            let mut v35 = n - v34;
            let mut v36 = v34 - 1;
            let mut v37 = v34;
            let mut edge = false;
            while v29 >= xs(v36) {
                if v35 == 0 {
                    v26 = n;
                }
                v37 += 1;
                v35 -= 1;
                v36 += 1;
                if n + 1 == v37 {
                    edge = true;
                    break;
                }
            }
            if !edge {
                v26 = v37;
            }
        }
        let mr = min_val(crate::phrase_rb_dsp::win(x, v34 - 1, v26 - v34 + 1));
        prom[p] = val[p] - v33.max(mr);
    }
    let mut wid = vec![0f64; k];
    for p in 0..k {
        let v43 = prom[p].mul_add(-0.5, val[p]);
        let mut v44 = loc[p] as i64;
        let mut go56 = v44 <= n;
        if v44 >= 1 {
            let mut v45 = v44 - 1;
            let mut edge = false;
            while v43 <= xs(v45) {
                if v45 == 0 {
                    v27 = 1;
                }
                let v46 = v45 + 1;
                v45 -= 1;
                if v46 <= 1 {
                    edge = true;
                    break;
                }
            }
            if !edge {
                v27 = v45 + 1;
            }
            go56 = v44 <= n;
        }
        if go56 {
            let mut v47 = n - v44;
            let mut v48 = v44 - 1;
            let mut edge = false;
            while v43 <= xs(v48) {
                if v47 == 0 {
                    v26 = n;
                }
                v44 += 1;
                v47 -= 1;
                v48 += 1;
                if n + 1 == v44 {
                    edge = true;
                    break;
                }
            }
            if !edge {
                v26 = v44;
            }
        }
        wid[p] = (xs(v27 - 1) - v43) / (xs(v27) - xs(v27 - 1)) - v27 as f64 + (v26 - 1) as f64 + (v43 - xs(v26 - 2)) / (xs(v26 - 1) - xs(v26 - 2));
    }
    (val, loc, wid, prom)
}

/// `UtaUmuAnalyze_a`: per beat spectrum, the strongest clear spectral peak above 112 bins
/// (vocal presence), 0 where none.
pub fn uta_umu(spec: &Mat2) -> Vec<f64> {
    let rows = spec.rows;
    let cols = spec.cols;
    let mut f = Filter::default();
    let w = prewarp(471.23889803846896, 5512.5);
    f.make2(0.0, 0.0, w * w, 1.0, w * 1.4142135623730951, w * w, 5512.5);
    f.clear();
    let mut sm = Mat2::zeros(rows, cols);
    let mut loc = vec![vec![0i32; cols]; rows];
    let mut v58 = Mat2::zeros(rows, cols);
    let mut v57 = Mat2::zeros(rows, cols);
    let mut v56 = Mat2::zeros(rows, cols);
    for r in 0..rows {
        f.clear();
        let (src, dst) = (spec.row(r).to_vec(), sm.row_mut(r));
        f.filter2(&src, dst);
        let (val, lc, wid, prom) = findpeaks(sm.row(r));
        for i in 0..lc.len().min(cols) {
            if lc[i] < 112 {
                continue;
            }
            loc[r][i] = lc[i];
            v58.d[r * cols + i] = val[i];
            if lc[i] != 0 {
                v57.d[r * cols + i] = wid[i] / lc[i] as f64;
            }
            v56.d[r * cols + i] = prom[i];
        }
    }
    let mut lvl = vec![0f64; rows];
    for r in 0..rows {
        let row = sm.row(r);
        let mut s = 0.0;
        for j in 111..743 {
            s += exp10(row.get(j).copied().unwrap_or(0.0) * 0.05);
        }
        lvl[r] = (s * 0.0015822784810126582).log10() * 20.0;
    }
    let mut v33 = 0.0;
    for v in &lvl {
        v33 += exp10(v * 0.05);
    }
    let v34 = (v33 / rows as f64).log10();
    let v37 = v34 * 38.8;
    let mut out = vec![0f64; rows];
    for r in 0..rows {
        let mut m = vec![0f64; cols];
        for c in 0..cols {
            let a = v58.d[r * cols + c];
            if !(a <= v37 || v57.d[r * cols + c] <= 0.11 || v56.d[r * cols + c] <= 10.0) {
                m[c] = a;
            }
        }
        let _ = &loc;
        out[r] = max_val(&m);
    }
    out
}
