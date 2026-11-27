//! Stage D of the phrase analyser port, part 3: where the intro ends (`IntroEndJudge_main`,
//! `IntroEndJudge`). Four candidates (low-band jump, vocal entry, low-level and mid-peak
//! changes) are scored against each other; the winner is the first phrase boundary.

use crate::phrase_rb_d1::sum_range;
use crate::phrase_rb_dsp::{max_val, sum8};

const T3: f64 = 0.33333333333333331;

/// A 32-bit int array with the reference's buffer behaviour (reads past `n` see what the
/// buffer held before).
#[derive(Clone, Debug, Default)]
pub struct MatI {
    pub buf: Vec<i32>,
    pub n: usize,
}

impl MatI {
    pub fn zeros(n: usize) -> MatI {
        MatI { buf: vec![0; n], n }
    }
    pub fn from(v: &[i32]) -> MatI {
        MatI { buf: v.to_vec(), n: v.len() }
    }
    pub fn copy_from(&mut self, src: &[i32]) {
        if self.buf.len() < src.len() {
            self.buf.resize(src.len(), 0);
        }
        self.buf[..src.len()].copy_from_slice(src);
        self.n = src.len();
    }
    pub fn v(&self) -> &[i32] {
        &self.buf[..self.n]
    }
    pub fn at(&self, i: usize) -> i32 {
        self.buf.get(i).copied().unwrap_or(0)
    }
    pub fn delete(&mut self, p: usize, k: usize) -> bool {
        if p + k > self.n || k < 1 {
            return false;
        }
        self.buf.copy_within(p + k..self.n, p);
        self.n -= k;
        true
    }
    pub fn resize(&mut self, n: usize) {
        if self.buf.len() < n {
            self.buf.resize(n, 0);
        }
        for i in self.n..n {
            self.buf[i] = 0;
        }
        self.n = n;
    }
    pub fn sort_ascend(&mut self) {
        let n = self.n;
        self.buf[..n].sort();
    }
}

/// Stable descending sort; returns the 1-based original positions.
pub fn sort_descend_idx(v: &mut [f64]) -> Vec<i32> {
    let mut p: Vec<(f64, i32)> = v.iter().enumerate().map(|(i, x)| (*x, i as i32 + 1)).collect();
    p.sort_by(|a, b| if a.0 > b.0 { std::cmp::Ordering::Less } else if b.0 > a.0 { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal });
    for (d, s) in v.iter_mut().zip(&p) {
        *d = s.0;
    }
    p.iter().map(|x| x.1).collect()
}

pub fn isort_descend_idx(v: &mut [i32]) -> Vec<i32> {
    let mut p: Vec<(i32, i32)> = v.iter().enumerate().map(|(i, x)| (*x, i as i32 + 1)).collect();
    p.sort_by(|a, b| b.0.cmp(&a.0));
    for (d, s) in v.iter_mut().zip(&p) {
        *d = s.0;
    }
    p.iter().map(|x| x.1).collect()
}

/// 8-lane sum of |m − x[i]| (the vectorised loops) for i in `a..b`.
fn sum_absdiff(m: f64, x: &[f64], a: usize, b: usize) -> f64 {
    let n = b.saturating_sub(a);
    let g = |i: usize| x.get(i).copied().unwrap_or(0.0);
    if n < 8 {
        let mut r = 0.0;
        for i in a..b {
            r = (m - g(i)).abs() + r;
        }
        return r;
    }
    let n8 = n & !7;
    let mut acc = [0f64; 8];
    for c in 0..n8 / 8 {
        for j in 0..8 {
            acc[j] = (m - g(a + 8 * c + j)).abs() + acc[j];
        }
    }
    let mut r = (acc[6] + (acc[4] + (acc[2] + acc[0]))) + (acc[7] + (acc[5] + (acc[3] + acc[1])));
    for i in a + n8..b {
        r = (m - g(i)).abs() + r;
    }
    r
}

/// Ratio sequence of a normalised per-bar series: (signed ratio, ratio folded to ≥ 1), 28 entries.
fn ratios(p: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut r = vec![0f64; 28];
    let mut a = vec![0f64; 28];
    let g = |i: usize| p.get(i).copied().unwrap_or(0.0);
    for k in 1..28 {
        let cur = if g(k) == 0.0 { 0.003 } else { g(k) };
        let prev = if g(k - 1) == 0.0 { 0.003 } else { g(k - 1) };
        let q = cur / prev;
        r[k] = q;
        a[k] = if q < 1.0 { 1.0 / q } else { q };
    }
    (r, a)
}

/// Normalise all but the last value by the maximum (`inv0` for a zero maximum is 0.003 or 333.33).
fn norm_but_last(p: &mut [f64], zero_inv: Option<f64>) {
    let mx = max_val(p);
    let inv = match zero_inv {
        Some(z) => {
            if mx == 0.0 {
                z
            } else {
                1.0 / mx
            }
        }
        None => {
            let v = 1.0 / mx;
            if v == 0.0 {
                0.003
            } else {
                v
            }
        }
    };
    let n = p.len();
    if n >= 2 {
        for v in p.iter_mut().take(n - 1) {
            *v *= inv;
        }
    }
}

/// The three strongest changes among the first 17 bars that are not weak early changes,
/// with neighbours merged (shared by both level passes).
fn top3(v795: &[f64], cap: usize) -> MatI {
    let mut v769: Vec<f64> = v795[..17].to_vec();
    let idx = sort_descend_idx(&mut v769);
    let mut v789 = MatI::from(&idx);
    let mut selfa = MatI::zeros(cap.max(17));
    selfa.copy_from(&v789.buf[..17]);
    let mut kept = 0usize;
    for k in 0..16 {
        let c = idx[k];
        if c > 3 || v795[(c - 1) as usize] >= 1.35 {
            kept += 1;
        } else {
            selfa.delete(kept, 1);
        }
    }
    v789.copy_from(selfa.v());
    let mut v792 = MatI::zeros(cap.max(5));
    let five: Vec<i32> = (0..5).map(|i| v789.at(i)).collect();
    v792.copy_from(&five);
    let (a, b) = (v792.at(0), v792.at(1));
    if b - a == 1 {
        v792.delete(1, 1);
    } else if a - b == 1 {
        v792.delete(0, 1);
    }
    let b = v792.at(1);
    if v792.at(2) - b == 1 {
        v792.delete(2, 1);
    } else if b - v792.at(0) == 1 {
        v792.delete(1, 1);
    }
    v792.resize(3);
    v792.sort_ascend();
    v792
}

/// The common candidate test of both level passes. Returns Some(candidate) when one passes.
#[allow(clippy::too_many_arguments)]
fn level_pass(cands: &[i32], v725: &mut [f64], v794: &[f64], v795: &mut [f64], second: bool) -> i32 {
    let v330 = v794[1];
    let n764 = 27i32;
    let g = |x: &[f64], i: i32| x.get(i as usize).copied().unwrap_or(0.0);
    let mut chosen: Option<i32> = None;
    for &c in cands.iter().take(3) {
        let mut refine = false;
        if c == 2 {
            if v330 > 1.0 && (g(v795, 3) + g(v795, 2)) * 0.5 < 1.4 && g(v795, 1) > 1.5 {
                chosen = Some(c);
                break;
            }
            continue;
        }
        if c < 3 {
            continue;
        }
        let v336 = c - 1;
        let (v337, v339) = if v330 >= 0.85 { (sum_range(v725, 0, c - 2) / v336 as f64, 1) } else { (sum_range(v725, 1, c - 2) / (c - 2) as f64, 2) };
        let hi = v339.max(v336);
        let v342 = sum_absdiff(v337, v725, (v339 - 1) as usize, hi as usize);
        let mut v357 = v342 / (c - v339) as f64;
        if !second && v357 == 0.0 {
            v357 = 0.003;
        }
        let v358 = g(v725, c - 1);
        let mut to757 = false;
        let mut v368 = false;
        if v357 >= 0.005 {
            let v369 = v358 / (v357 + v337);
            let v370 = v358 / (v337 - v357);
            if v369 > 1.15 || v370 < 0.86956521739130443 {
                v368 = c > 3;
            } else {
                to757 = true;
            }
        } else if (v337 - v358).abs() / v357 <= 15.0 {
            to757 = true;
        } else {
            v368 = c > 3;
        }
        let mut to778 = false;
        if to757 {
            if c < 4 {
                continue;
            }
            if g(v795, c - 1) <= 1.3 {
                to778 = true;
            } else {
                let v359 = if c <= 9 { 2 } else { 4 };
                let v360 = c - v359 - 1;
                let v362 = if c - 2 - v360 >= 0 { sum_range(v725, v360, c - 2) / (c - 2 - v360 + 1) as f64 } else { 0.0 };
                let mut v361 = 0.0;
                if v360 < c - 1 {
                    for j in 0..v359 {
                        v361 = (v362 - g(v725, v360 + j)).abs() + v361;
                    }
                }
                let v366 = v361 / v359 as f64;
                let v367 = g(v725, c - 1);
                if v367 / (v366 + v362) <= 1.2 && v367 / (v362 - v366) >= 0.83333333333333337 {
                    to778 = true;
                } else {
                    v368 = true;
                }
            }
        }
        if !to778 {
            if n764 < c + 3 {
                if !v368 {
                    continue;
                }
                to778 = true;
            } else {
                let i2 = (c + 2) as usize;
                if v725[i2] == 0.0 {
                    v725[i2] = 0.003;
                }
                let v376 = v725[i2];
                let v377 = g(v795, c + 1);
                let v378 = (v377 + g(v795, c)) * 0.5;
                let mut v379 = g(v725, c) / v376;
                if v379 < 1.0 {
                    v379 = 1.0 / v379;
                }
                let mut next = false;
                if v378 >= 1.4 {
                    let v380 = (g(v795, c + 2) + v377) * 0.5;
                    if v380 >= 1.4 {
                        let v381 = g(v795, c - 1) * 0.6;
                        if v378 >= v381 && v380 >= v381 && v379 >= 1.1 {
                            next = true;
                        }
                    }
                }
                if next {
                    continue;
                }
                refine = true;
            }
        }
        if to778 {
            let mut v372 = if second { sum_range(v725, c - 3, c - 2) * 0.5 } else { sum_range(v725, c - 4, c - 2) * T3 };
            if v372 == 0.0 {
                v372 = 0.003;
            }
            let v373 = g(v725, c - 1) / v372;
            if v373 > 1.4 || v373 < 0.7 {
                refine = true;
            } else {
                continue;
            }
        }
        if refine {
            chosen = Some(c);
            break;
        }
    }
    let Some(mut c) = chosen else { return 0 };
    // refinement
    let i1 = (c + 1) as usize;
    if v725.get(i1).copied() == Some(0.0) {
        v725[i1] = 0.003;
    }
    let v383 = g(v725, c + 1);
    if c < 3 && g(v794, c - 1) <= 1.6 {
        return c;
    }
    let v388 = g(v794, c - 1);
    let v389 = g(v794, c);
    if v388 <= 1.0 {
        if v389 * v388 > 0.83 {
            c += 1;
        }
    } else if v389 > 1.15 || v389 < 0.6 {
        let q = g(v725, c - 1) / v383;
        c += (q > 1.05 || q < 0.95) as i32;
    }
    c
}

/// `IntroEndJudge` → (candidates as beats, chosen intro end beat).
pub fn intro_end_judge(p0: &[f64], p1: &mut [f64], p2: &mut [f64], p3: i32, p4: &[f64]) -> Option<(Vec<i32>, i32)> {
    let cap = p0.len();
    let v19 = p3 - 1;
    let v20 = p0.len() as i32 - (p3 - 1);
    let v728 = v20 / 4;
    let mut v22 = vec![0f64; v728.max(0) as usize];
    let p1a = v728 - 1;
    if v20 >= 8 {
        let mut s = v19;
        for k in 0..p1a.max(1) as usize {
            if let Some(d) = v22.get_mut(k) {
                *d = sum_range(p0, s, s + 3) * 0.25;
            }
            s += 4;
        }
    }
    let mut v25 = 1.0 / max_val(&v22);
    if v25 == 0.0 {
        v25 = 0.003;
    }
    if v20 >= 8 {
        for v in v22.iter_mut().take(p1a.max(1) as usize) {
            *v *= v25;
        }
    }
    if v22.len() < 28 {
        v22.resize(28, 0.0);
    }
    let mut v36 = vec![0f64; 25];
    for k in 1..25 {
        if v22[k - 1] == 0.0 {
            v22[k - 1] = 0.003;
        }
        v36[k] = v22[k] / v22[k - 1];
    }
    let mut v61: Vec<i32> = vec![];
    for k in 0..25 {
        if v36[k] >= 2.0 && v22[k] > 0.095 {
            v61.push(k as i32 + 1);
        }
    }
    let v716 = v61.len();
    let g22 = |i: i32| v22.get(i as usize).copied().unwrap_or(0.0);
    let mut v65: Vec<i32> = vec![];
    for &c in &v61 {
        let mut v73 = g22(c - 1);
        if v73 == 0.0 {
            v73 = 0.003;
        }
        let a3 = sum_range(&v22[..v728.max(0) as usize], c, c + 2) * 0.3333333333333333;
        let a2 = sum_range(&v22[..v728.max(0) as usize], c, c + 1) * 0.5;
        let r = |x: f64| -> f64 {
            let x = if x == 0.0 { 0.003 } else { x };
            let q = x / v73;
            if 1.0 > q {
                v73 / x
            } else {
                q
            }
        };
        if !(r(a2) > 1.2) && !(r(a3) > 1.3) {
            v65.push(c);
        }
    }
    let v736;
    let vv = &v22[..v728.max(0) as usize];
    if v716 == 0 {
        v736 = 0;
    } else {
        let mut v84 = [0f64; 3];
        let mut v745 = [0f64; 7];
        let mut v85 = vec![0f64; v716.max(1)];
        let mut v86 = vec![0f64; v716.max(1)];
        let mut v741 = vec![0f64; v716.max(1)];
        let mut p0b = vec![0f64; v716.max(1)];
        let mut cnt = v65.len();
        if cnt < v716 {
            for (i, &c) in v61.iter().enumerate() {
                let v92 = sum_range(vv, c, c + 2);
                let v94 = if v728 >= c + 7 {
                    sum_range(vv, c, c + 6) * 0.14285714285714285
                } else if p1a - c < 0 {
                    0.0
                } else {
                    sum_range(vv, c, v728 - 1) / (v728 - c) as f64
                };
                let v95 = v92 * T3;
                for j in 0..7 {
                    if c + j + 1 < v728 {
                        v745[j as usize] = (g22(c + j) - v94).abs();
                        if j < 3 {
                            v84[j as usize] = (g22(c + j) - v95).abs();
                        }
                    }
                }
                v85[i] = sum8(&v84) / 3.0;
                v86[i] = sum8(&v745) / 7.0;
                if !(v85[i] > 0.1 || v86[i] > 0.13 || (cnt != 0 && v65[cnt - 1] == c)) {
                    v65.push(c);
                    cnt += 1;
                }
            }
        }
        if cnt < v716 {
            let mut v111 = 0i32;
            for (i, &c) in v61.iter().enumerate() {
                let v114 = if (c - 2) as u32 >= 3 { v111 } else { c - 1 };
                let v115 = if c <= 4 { v114 } else { 4 };
                v111 = v115;
                let s = (c - 1 - v115).max(0) as usize;
                let mut w: Vec<f64> = (0..v115.max(0) as usize).map(|k| v22.get(s + k).copied().unwrap_or(0.0)).collect();
                let mut v119 = if w.is_empty() { 0.0 } else { sum8(&w) / w.len() as f64 };
                if v119 == 0.0 {
                    v119 = 0.003;
                }
                for x in w.iter_mut() {
                    *x = (*x - v119).abs();
                }
                v741[i] = if w.is_empty() { 0.0 } else { sum8(&w) / w.len() as f64 };
                let v134 = g22(c - 1) / v119;
                p0b[i] = v134;
                if !(v85[i] > 0.14 || v86[i] > 0.16 || v741[i] > 0.04 || v134 < 3.0) && !(cnt != 0 && v65[cnt - 1] == c) {
                    v65.push(c);
                    cnt += 1;
                }
            }
        }
        if cnt < v716 {
            for (i, &c) in v61.iter().enumerate() {
                if v85[i] <= 0.18 && v86[i] <= 0.3 && v741[i] <= 0.01 && p0b[i] >= 3.0 && !(cnt != 0 && v65[cnt - 1] == c) {
                    v65.push(c);
                    cnt += 1;
                }
            }
        }
        if cnt >= 1 {
            let mut s = v65.clone();
            s.truncate(cnt);
            s.sort();
            let mut m = s[0];
            let mut mi = 0usize;
            for (i, x) in s.iter().enumerate() {
                if *x < m {
                    m = *x;
                    mi = i;
                }
            }
            let next = mi + 1;
            if cnt > next && s[next] - m == 1 {
                m = s[next];
            }
            v736 = m;
        } else {
            v736 = 0;
        }
    }
    // vocal entry
    let mut v142 = vec![0f64; 112];
    let mx = max_val(p4);
    let v145 = if mx == 0.0 { 333.33333333333331 } else { 1.0 / mx };
    for i in 0..112 {
        v142[i] = p4.get(i).copied().unwrap_or(0.0) * v145;
    }
    let mut v825: Vec<i32> = vec![];
    for k in 0..96usize {
        let a = v142[k + 2];
        let b = v142[k + 3];
        let c = v142[k + 1];
        let mut v153 = (a + c) + b;
        if k & 3 == 0 {
            v153 = 1.0;
        }
        if v142[k] >= 0.5 && v153 != 0.0 {
            let cnt = (1..8).filter(|j| v142[k + j] > 0.0).count() + 1;
            let s = sum_range(&v142, k as i32, k as i32 + 7) / cnt as f64;
            let ok = if cnt < 5 { cnt == 4 && s >= 0.5 } else { s >= 0.4 };
            if ok {
                v825.push(k as i32 + 1);
            }
        }
    }
    let v825_0 = v825.first().copied().unwrap_or(0);
    let v825_1 = v825.get(1).copied().unwrap_or(0);
    let mut v731 = 0i32;
    let mut found = false;
    for k in 0..92i32 {
        let v159 = sum_range(&v142, 0, k + 4);
        if k + 5 >= 35 && v159 != 0.0 {
            break;
        }
        let b = |i: i32| v142[i as usize];
        if b(k + 5) > 0.0 {
            let v162 = (k + 5..=k + 20).filter(|i| b(*i) > 0.0).count();
            let v163 = sum_range(&v142, k + 5, k + 20) * 0.0625;
            if (v162 >= 8 && v163 > 0.1) || (v162 >= 6 && v163 > 0.2) {
                let v170 = sum_range(&v142, 4, k + 4) / (k + 1) as f64;
                let v171 = b(k + 7) * b(k + 5) * b(k + 8) <= 0.0 && b(k + 6) * b(k + 5) <= 0.0;
                if !v171 && v170 < 0.05 {
                    v731 = k + 6;
                    found = true;
                    break;
                }
            }
        }
    }
    if !found {
        let mut done = false;
        let mut v199 = 8usize;
        loop {
            let v200 = v199;
            let v201 = sum_range(&v142, 0, v199 as i32 - 1);
            let mut v174 = vec![0f64; 112];
            for i in 0..v199 {
                if v142[i] > 0.0 {
                    v174[i] = 1.0;
                }
            }
            let v207 = (v199 >> 2) as usize;
            let mut v175 = [0f64; 8];
            for j in 0..v207.min(8) {
                v175[j] = sum_range(&v174, 4 * j as i32, 4 * j as i32 + 3);
            }
            let mut brk = false;
            for j in 1..v207.min(8) {
                if v175[j] >= 3.0 && v175[j - 1] >= 3.0 {
                    brk = true;
                    break;
                }
            }
            if brk {
                break;
            }
            v199 = v200 + 1;
            if v142[v200] > 0.0 {
                let v212 = sum_range(&v142, v200 as i32, v200 as i32 + 11);
                let v214 = 1 + (v142[v200 + 1] > 0.0) as i32 + (v142[v200 + 2] > 0.0) as i32 + (v142[v200 + 3] > 0.0) as i32;
                if v214 >= 3 {
                    let c1 = (4..8).filter(|j| v142[v200 + j] > 0.0).count();
                    let c2 = (8..12).filter(|j| v142[v200 + j] > 0.0).count();
                    let v219 = v212 * 0.083333333333333329;
                    let v220 = v212 * 0.041666666666666664;
                    if c1 >= 3 && c2 >= 3 && v219 > 0.4 && v201 / (v200 as f64) < v220 {
                        v731 = v200 as i32 + 1;
                        done = true;
                        break;
                    }
                }
            }
            if v200 == 33 {
                break;
            }
        }
        if !done {
            let mut hit = None;
            for v226 in 14..73i32 {
                let v227 = sum_range(&v142, 0, v226 - 8);
                let b = |i: i32| v142[i as usize];
                if b(v226 - 7) > 0.0 {
                    let v230 = (v226 - 7..=v226).filter(|i| b(*i) > 0.0).count() as i32;
                    let v231 = sum_range(&v142, v226 - 7, v226 + 16);
                    let v232 = sum_range(&v142, v226 - 7, v226);
                    let v233 = v230 < 3 || v231 * 0.041666666666666664 < 0.1;
                    let v234 = !v233 && v227 / ((v226 - 7) as f64) < 0.04;
                    if v234 && (v232 / v230 as f64 >= 0.65 || (b(v226 - 7) > 0.7 && b(v226 - 6) > 0.3 && b(v226 - 5) > 0.3)) {
                        hit = Some(v226);
                        break;
                    }
                }
            }
            if let Some(v226) = hit {
                v731 = v226 - 6;
            }
        }
    }
    let bar_of = |x: i32| -> i32 {
        let f = (x - 1) as f64 * 0.25 + 1.0;
        (f + if f < 0.0 { -0.5 } else { 0.5 }) as i32
    };
    let (b0, b1) = (bar_of(v731), bar_of(v825_0));
    let mut v176 = v825_0;
    let mut v185 = true;
    let mut v190;
    let mut jump397 = false;
    v190 = 0;
    if b1 != 1 && !(b0 == 5 && b1 == 4) && !(b0 == 9 && b1 == 8) {
        v185 = !(b1 <= b0 || b0 == 1);
        if b0 == 8 && b1 == 9 {
            v190 = 9;
            jump397 = true;
        }
    }
    if !jump397 {
        let sel = !(b0 == 4 && b1 == 5) && v185;
        if sel {
            v190 = b0;
            v176 = v731;
        } else {
            v190 = b1;
        }
    }
    if jump397 || v190 >= 2 {
        let v192 = if ((v176 - 14) as u32) >= 6 { v190 } else { 5 };
        v190 = if ((v176 - 30) as u32) >= 6 { v192 } else { 9 };
        let inr = (v176 as u32) <= 35 && ((1u64 << v176) & 0xFC00FC000) != 0;
        if !inr {
            if v190 == 8 {
                v190 = if bar_of(v825_1) == 9 { 9 } else { 8 };
            } else if v190 == 4 {
                v190 = if bar_of(v825_1) == 5 { 5 } else { 4 };
            }
        }
    }
    // level passes
    norm_but_last(p1, None);
    let (v810, v242) = ratios(p1);
    norm_but_last(p2, Some(333.33333333333331));
    let (v809, v269) = ratios(p2);
    let mut v725: Vec<f64> = (0..27).map(|i| p1.get(i).copied().unwrap_or(0.0)).collect();
    let v794a: Vec<f64> = v810[..27].to_vec();
    let mut v795: Vec<f64> = v242.clone();
    for k in 1..=16 {
        if v725[k - 1] < 0.02 && v725[k] < 0.02 && v795[k] < 3.0 {
            v795[k] = 1.0;
        }
    }
    let c3 = top3(&v795, cap);
    let v724 = level_pass(c3.v(), &mut v725, &v794a, &mut v795, false);
    let mut v725: Vec<f64> = (0..27).map(|i| p2.get(i).copied().unwrap_or(0.0)).collect();
    let v794b: Vec<f64> = v809[..27].to_vec();
    let mut v795: Vec<f64> = v269[..27].to_vec();
    v795.push(0.0);
    let mut c3 = top3(&v795, cap);
    {
        let v412: Vec<i32> = c3.v().to_vec();
        let (a, b, c) = (v412[0], v412[1], v412[2]);
        let g = |v: &Vec<f64>, i: i32| v.get(i as usize).copied().unwrap_or(0.0);
        if v795[(a - 1) as usize] == 0.0 {
            v795[(a - 1) as usize] = 0.003;
        }
        if v795[(b - 1) as usize] == 0.0 {
            v795[(b - 1) as usize] = 0.003;
        }
        let v417 = v795[(b - 1) as usize];
        let mut v418 = true;
        if a >= 3 && b - a == 2 {
            v418 = v417 / g(&v795, a - 1) <= 2.0 && g(&v795, b) + g(&v795, b - 2) + g(&v795, b + 1) >= g(&v795, a) + g(&v795, a - 2) + g(&v795, a + 1);
        }
        let mut order: Option<[i32; 3]> = None;
        let mut v422 = true;
        if c - b == 2 {
            if g(&v795, c - 1) / v417 > 2.0 {
                order = Some(if !v418 { [c, b, a] } else { [a, c, b] });
            } else {
                v422 = g(&v795, c) + g(&v795, c - 2) + g(&v795, c + 1) >= g(&v795, b) + g(&v795, b - 2) + g(&v795, b + 1);
                if !v418 && !v422 {
                    order = Some([c, b, a]);
                }
            }
        }
        if order.is_none() {
            if !v418 {
                order = Some([b, a, c]);
            } else if !v422 {
                order = Some([a, c, b]);
            }
        }
        if let Some(o) = order {
            c3.buf[..3].copy_from_slice(&o);
        }
    }
    let v413 = level_pass(c3.v(), &mut v725, &v794b, &mut v795, true);
    // scoring
    let mut v787 = [v736, v190, v724, v413];
    let mut v720 = v787;
    for x in v787.iter_mut() {
        if *x == 1 {
            *x = 0;
        }
    }
    let eq = |i: usize, j: usize| (v787[i] == v787[j]) as i32;
    let v479: Vec<i32> = (0..4).map(|i| eq(0, i)).collect();
    let v480: Vec<i32> = (0..4).map(|i| eq(1, i)).collect();
    let v756: Vec<i32> = (0..4).map(|i| eq(2, i)).collect();
    let s786: i32 = v479.iter().sum();
    let s785: i32 = v480.iter().sum();
    let s784: i32 = v756.iter().sum();
    let mut v478 = [0f64; 4];
    let mut v505;
    let v506;
    if s786 >= 3 && v787[0] != 0 {
        v505 = v787[0];
        v478 = [3.0; 4];
        v506 = true;
    } else if s785 >= 3 && v787[1] != 0 {
        v505 = v787[1];
        v478 = [3.0; 4];
        v506 = true;
    } else {
        v506 = false;
        if s786 == 2 && v787[0] != 0 {
            for i in 0..4 {
                v478[i] = v479[i] as f64;
            }
        }
        if s785 == 2 && v787[1] != 0 && v787[0] != v787[1] {
            for i in 0..4 {
                v478[i] += v480[i] as f64;
            }
        }
        if s784 == 2 && v787[2] != 0 && v787[0] != v787[2] && v787[1] != v787[2] {
            for i in 0..4 {
                v478[i] += v756[i] as f64;
            }
        }
        let mut u: Vec<i32> = v787.to_vec();
        u.sort();
        u.dedup();
        if u[0] == 0 {
            u.remove(0);
        }
        if u.len() == 1 {
            for s in 0..4 {
                if v787[s] == u[0] && u[0] != 0 {
                    v478[s] += 0.4;
                }
            }
        } else if u.len() > 1 {
            for s in 0..4 {
                let x = v787[s];
                for (ix, uu) in u.iter().enumerate() {
                    if x != *uu || *uu == 0 {
                        continue;
                    }
                    let old = v478[s];
                    if ix == 0 {
                        v478[s] = old + 0.4;
                        if u[1] - u[0] >= 4 {
                            v478[s] = old + 0.7;
                        }
                    } else if ix == 1 {
                        v478[s] = if u[1] - u[0] <= 3 { old + 0.2 } else { old };
                    } else {
                        v478[s] = old + -0.34999999999999998;
                    }
                }
            }
        }
        let mut srt = v787;
        let ix = isort_descend_idx(&mut srt);
        for t in 0..3 {
            if srt[t] - srt[t + 1] == 1 {
                let a = ix[t];
                v478[(a - 1) as usize] += 0.75;
                let b = ix[t + 1];
                if b == 1 {
                    v478[0] += 0.85999999999999999;
                } else if a == 3 && b == 4 {
                    v478[2] += 0.3;
                }
            }
        }
        for s in 0..4 {
            let x = v787[s];
            if x != 0 {
                let old = v478[s];
                v478[s] = old + 1.0;
                let add = match x {
                    2 => Some(1.2),
                    3 => Some(1.3),
                    5 | 17 => Some(1.8),
                    9 => Some(1.8500000000000001),
                    _ if x >= 18 => Some(0.44999999999999996),
                    _ => None,
                };
                if let Some(a) = add {
                    v478[s] = old + a;
                }
            } else {
                v478[s] = 0.0;
            }
        }
        v505 = 0;
    }
    // LABEL_1063
    let n1 = p1.len();
    let mut v507 = vec![0f64; n1];
    for k in 1..n1 {
        v507[k] = (p1[k] - p1[k - 1]).abs();
    }
    let v517 = v787[2] - 1;
    if v517 >= 0 && v242.get(v517 as usize).copied().unwrap_or(0.0) < 1.3 && v507.get(v517 as usize).copied().unwrap_or(0.0) < 0.09 {
        v478[2] = 0.0;
        v787[2] = 0;
        v720[2] = 0;
    }
    if !v506 {
        let mut uq: Vec<f64> = v478.to_vec();
        uq.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        uq.dedup();
        uq.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let mut srt = v478.to_vec();
        let ix = sort_descend_idx(&mut srt);
        let v539 = (ix[0] - 1) as usize;
        v505 = v787[v539];
        let v541 = (0..4).filter(|i| v478[*i] == srt[*i]).count();
        if uq.len() >= 2 && v541 == 1 {
            let v691 = v478[2];
            if uq[1] == v691 {
                let v692 = (v505 - v787[2]).abs();
                if v692 <= 4 && v478[v539] - v691 <= 1.1 && ix[0] != 1 {
                    let v694 = v810.get((v724 - 1).max(0) as usize).copied().unwrap_or(0.0);
                    if v694 < 0.32 || v694 > 1.5 {
                        v505 = v787[2];
                    }
                }
            }
        }
    }
    let _ = v720;
    let v520 = p3 - 4;
    let out: Vec<i32> = v787.iter().map(|x| if *x != 0 { v520 + 4 * x } else { 0 }).collect();
    let chosen = if v505 == 0 { p3 } else { v520 + 4 * v505 };
    Some((out, chosen))
}

/// `IntroEndJudge_main` → (candidates, intro end beat, found).
#[allow(clippy::too_many_arguments)]
pub fn intro_end_judge_main(lowpeak: &[f64], lowlevel_b: &[f64], midpeak_b: &[f64], bar: i32, utaumu: &[f64], secure: &[i32]) -> Option<(Vec<i32>, i32, i32)> {
    let mut a = lowpeak.to_vec();
    let mut b = lowlevel_b.to_vec();
    let mut c = midpeak_b.to_vec();
    let mut d = utaumu.to_vec();
    let v23 = a.len() as i32 / 4;
    let v25 = v23.min(b.len() as i32).min(c.len() as i32).min(d.len() as i32 / 4);
    let mut p6: Vec<i32>;
    let mut p6a = 0i32;
    let mut p8;
    let v26;
    if v25 < 2 {
        p8 = 0;
        p6 = vec![0];
        v26 = true;
    } else {
        let n = v25 as usize;
        if v25 <= 27 {
            let (a0, b0, c0, d0) = (a[..4 * n].to_vec(), b[..n].to_vec(), c[..n].to_vec(), d[..4 * n].to_vec());
            a = a0.clone();
            b = b0.clone();
            c = c0.clone();
            d = d0.clone();
            let target = ((v25 + 27) as u8 / v25 as u8) as i32;
            for _ in 0..=target {
                a.extend_from_slice(&a0);
                b.extend_from_slice(&b0);
                c.extend_from_slice(&c0);
                d.extend_from_slice(&d0);
            }
        }
        let (cands, chosen) = intro_end_judge(&a, &mut b, &mut c, bar, &d)?;
        p6 = cands;
        p6a = chosen;
        p8 = 1;
        if v25 <= 7 {
            p8 = 0;
            p6 = vec![0];
            p6a = 0;
            v26 = true;
        } else {
            v26 = false;
        }
    }
    let mut v70 = p6.clone();
    let sum70: i32 = v70.iter().sum();
    if !v26 && sum70 == 0 {
        if v25 >= 28 {
            p8 = 1;
            p6 = vec![bar];
            p6a = bar;
        } else {
            p8 = 0;
        }
    }
    let v30 = p6.len();
    p6.sort_by(|x, y| y.cmp(x));
    if !v26 && sum70 != 0 && v25 <= 27 {
        p8 = 1;
        for k in 0..v30.min(v70.len()) {
            if v70[k] <= v25 {
                p6a = v70[k];
            } else {
                v70[k] = 0;
            }
        }
        if v70.iter().sum::<i32>() == 0 {
            p6 = vec![bar];
            p6a = bar;
        }
    }
    let mut p7 = p6a;
    let v68: Vec<i32> = p6.iter().map(|x| {
        let v34 = x - bar;
        if v34 <= 3 {
            v34 / 4
        } else {
            v34 / 4 + 1
        }
    }).collect();
    if secure.is_empty() || secure[0] == 0 {
        return Some((p6, p7, p8));
    }
    let v66: Vec<i32> = secure.iter().map(|x| (x - bar) / 4 + 1).collect();
    let v41 = p6.len();
    if !secure.is_empty() {
        if v41 < 1 {
            return Some((p6, p7, p8));
        }
        let mut hit = false;
        for i in (0..secure.len()).rev() {
            for j in 0..v41 {
                if secure[i] == p6[j] {
                    p7 = secure[i];
                    hit = true;
                    p8 = 1;
                }
            }
        }
        if hit {
            return Some((p6, p7, p8));
        }
    }
    let v48 = v66[0] % 4;
    if v41 < 1 {
        return Some((p6, p7, p8));
    }
    let v83: Vec<i32> = v68.iter().map(|x| x % 4).collect();
    if v41 >= 2 {
        let mut v61 = 0i32;
        for k in 0..v41 - 1 {
            if v83[k] == v48 {
                let v64 = if v61 >= p6[k] { p6[k] } else { v61 };
                v61 = if v61 != 0 { v64 } else { p6[k] };
            }
        }
        if v61 != 0 {
            p7 = v61;
            p8 = 1;
        }
    }
    Some((p6, p7, p8))
}
