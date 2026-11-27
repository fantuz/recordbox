//! Stage D of the phrase analyser port, part 7: verse/hook boundaries from bar-to-bar ratios
//! (`VerseHookJudge23`, `VerseHookJudge23Sub`).

use crate::phrase_rb_d5::Ratios;
use crate::phrase_rb_dsp::sum8;

/// A window into a longer array: direct reads may go past `n` (as the reference's linked
/// arrays do), sums stay inside it.
struct Win<'a> {
    a: &'a [f64],
    off: usize,
    n: usize,
}

impl Win<'_> {
    fn at(&self, i: i64) -> f64 {
        let j = self.off as i64 + i;
        if j < 0 {
            0.0
        } else {
            self.a.get(j as usize).copied().unwrap_or(0.0)
        }
    }
    fn sum(&self, a: i64, b: i64) -> f64 {
        if a < 0 || a > b || b >= self.n as i64 {
            return 0.0;
        }
        let v: Vec<f64> = (a..=b).map(|i| self.at(i)).collect();
        sum8(&v)
    }
}

const T3: f64 = 0.33333333333333331;

/// `VerseHookJudge23Sub`: up to 4 (+ extra) strong ratio changes in a 20-bar window.
/// Leaves `out` untouched for windows shorter than 10 bars (as the reference does).
fn sub(p0: &Win, p1: &Win, p2: &Win, p3: i32, out: &mut Vec<i32>) {
    let n = p0.n as i64;
    if n < 10 {
        return;
    }
    let v16 = n - 6;
    let v17 = p1.sum(3, n - 4) / v16 as f64;
    let v18 = if v17 == 0.0 { 0.003 } else { v17 };
    // 1-based index of the maximum of p0[3..3+v16]
    let mut v75 = 1i64;
    let mut mx = p0.at(3);
    for k in 1..v16 {
        if p0.at(3 + k) > mx {
            mx = p0.at(3 + k);
            v75 = k + 1;
        }
    }
    let mut v80: Vec<(f64, i64)> = (0..v16).map(|k| (p1.at(3 + k), k + 1)).collect();
    v80.sort_by(|a, b| if a.0 > b.0 { std::cmp::Ordering::Less } else if b.0 > a.0 { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal });
    let mut v20: Vec<i64> = v80.iter().map(|x| x.1).collect();
    let v79 = v20.len() as i64;
    let mut v21: Vec<i64> = (0..4).map(|k| v20.get(k).copied().unwrap_or(0)).collect();
    v21.sort_by(|a, b| b.cmp(a));
    let mut v22 = v21.clone();
    let f = |i: i64| p1.at(i);
    let s = |i: i64| p2.at(i);
    let ratio = |a: i64, b: i64| {
        let d = f(b + 2);
        if d == 0.0 {
            f(a + 2) * 333.33333333333331
        } else {
            f(a + 2) / d
        }
    };
    let mut v29 = 0i64;
    if v21[0] - v21[1] == 1 && v79 >= 5 {
        let v28 = ratio(v21[0], v21[1]);
        if v28 < 1.7 && s(v21[0] + 2) <= 1.8 {
            v22[0] = 0;
            v29 = 1;
        }
        if s(v21[0] + 2) > 1.8 {
            v22[1] = 0;
            v29 += 1;
        }
    }
    for (a, b) in [(1usize, 2usize), (2, 3)] {
        let r = ratio(v21[a], v21[b]);
        if v21[a] - v21[b] == 1 && v29 + 4 < v79 {
            if r < 1.7 && s(v21[a] + 2) <= 1.8 {
                v22[a] = 0;
                v29 += 1;
            }
            if s(v21[a] + 2) > 1.8 {
                v22[b] = 0;
                v29 += 1;
            }
        }
    }
    for k in 0..4 {
        if k < v20.len() {
            v20[k] = v22[k];
        }
    }
    let v41 = (v29 + 4) as usize;
    let mut p4 = vec![0i32; v41];
    let mut v72 = 0;
    let v71 = v18 * 1.0573000000000001;
    let v70 = p3 as i64 - 1;
    let d = |i: i64| p0.at(i);
    for (k, slot) in p4.iter_mut().enumerate() {
        let v43 = v20.get(k).copied().unwrap_or(0);
        if v43 == 0 {
            continue;
        }
        let v45 = p1.sum(v43, v43 + 1);
        let v46 = p1.sum(v43 - 1, v43 + 1);
        let v47 = p0.sum(v43 - 1, v43 + 1) * T3;
        let v48 = if v47 == 0.0 { 0.003 } else { v47 };
        let v49 = v43 + 3;
        let v50 = p1.sum(v43 + 3, v43 + 4) * 0.5;
        let v51 = if v50 == 0.0 { 0.003 } else { v50 };
        let v54 = p1.sum(v43 + 4, v43 + 5) * 0.5;
        let v55 = if v54 == 0.0 { 0.003 } else { v54 };
        let v56 = if d(v43 + 1) == 0.0 { 0.003 } else { d(v43 + 1) };
        let v53 = v43 + 2;
        let v57 = f(v53);
        let v58 = v57 > v18 * 1.05 || v75 == v43;
        if !v58 {
            continue;
        }
        let v59 = d(v53);
        let v60 = v59 / v48 > v18 * 1.0900000000000001 || v59 / v48 < 0.9174311926605504 / v18;
        if !(v60 || v75 == v43) {
            continue;
        }
        let v62 = v45 * 0.5 < v71 && v46 * T3 < v71;
        if v62 || (v75 == v43 && v59 / v56 > 1.1000000000000001) {
            let v63 = if v57 / v51 <= v18 { v55 < 1.2 } else { true };
            let v64 = !v63 && v57 / v55 <= v18;
            if v51 < 1.2 || !v64 || (v57 > 1.3999999999999999 && (f(v43 + 5) + f(v49)) * 0.5 < 1.2) {
                *slot = (v70 + v49) as i32;
                v72 += 1;
            }
        }
    }
    while p4.last() == Some(&0) {
        p4.pop();
    }
    if v72 == 0 {
        *out = vec![0];
        return;
    }
    p4.sort();
    for _ in 0..3 {
        if p4.first().copied().unwrap_or(0) != 0 {
            *out = p4;
            return;
        }
        if !p4.is_empty() {
            p4.remove(0);
        }
    }
    if p4.first().copied().unwrap_or(0) != 0 {
        *out = p4;
        return;
    }
    *out = vec![0];
}

fn del0_sorted(v: &mut Vec<i32>) {
    v.sort();
    let k = v.iter().take_while(|x| **x == 0).count();
    v.drain(..k);
}

/// `VerseHookJudge23` → boundary beats.
#[allow(clippy::too_many_arguments)]
pub fn verse_hook_judge23(g: &[Ratios; 4], cands: &[i32], intro: i32, bar: i32, ty: i32) -> Option<Vec<i32>> {
    let v26 = g[0].v.len().min(g[1].v.len()).min(g[2].v.len());
    let cp = |x: &Vec<f64>| x[..v26.min(x.len())].to_vec();
    let (bv, bf, bs) = (cp(&g[0].v), cp(&g[0].fold), cp(&g[0].sign));
    let (lv, lf, ls) = (cp(&g[1].v), cp(&g[1].fold), cp(&g[1].sign));
    let (mv, mf, ms) = (cp(&g[2].v), cp(&g[2].fold), cp(&g[2].sign));
    let q = |x: i32| ((x - bar) as f64).mul_add(0.25, 1.5) as i32;
    let mut v133: Vec<i32> = cands.iter().map(|x| {
        let b = q(*x);
        if (v26 as i32) < b {
            0
        } else {
            b
        }
    }).collect();
    v133.sort();
    let mut v137 = v133.clone();
    for k in 0..v133.len().saturating_sub(1) {
        if v133[k] == v133[k + 1] {
            v137[k] = 0;
        }
    }
    v137.sort();
    let mut v136 = v137;
    let mut v40 = 4;
    let mut leading = 0;
    while leading < 4 && v136.first().copied().unwrap_or(0) < 1 {
        if !v136.is_empty() {
            v136.remove(0);
        }
        leading += 1;
        if leading < 4 && v136.first().copied().unwrap_or(0) > 0 {
            break;
        }
    }
    if leading > 0 {
        v40 = 4 - leading;
    }
    if v40 == 0 || v136.iter().sum::<i32>() == 0 {
        v136 = vec![0];
    }
    let v41 = q(intro);
    if v40 >= 2 && v41 - v136[0] == 1 {
        v136.remove(0);
    }
    let mut p13 = v136;
    let v43 = p13.last().copied().unwrap_or(0).max(3) as i64;
    let v26i = v26 as i64;
    let mut v44 = (v43 + 19).min(v26i);
    let mut v45 = if v43 - 2 <= v44 { v43 - 2 } else { v26i };
    let mut v132 = vec![0i32];
    let mut v131 = vec![0i32];
    let mut v130 = vec![0i32];
    let mut v119: Vec<i32> = vec![];
    if v26i - v43 - 1 >= -15 {
        let mut off = v45 - 1;
        let mut p10a = 1 - v45;
        let mut p9a = (v26i - v43 - 1) / 16 + 1;
        loop {
            let len = v44 + p10a;
            if len < 0 || off < 0 {
                return None;
            }
            fn w(a: &[f64], off: i64, len: i64) -> Win<'_> {
                Win { a, off: off as usize, n: len as usize }
            }
            sub(&w(&bv, off, len), &w(&bf, off, len), &w(&bs, off, len), v45 as i32, &mut v119);
            v132.extend_from_slice(&v119);
            if ty == 1 {
                v132 = vec![0];
            }
            sub(&w(&lv, off, len), &w(&lf, off, len), &w(&ls, off, len), v45 as i32, &mut v119);
            v131.extend_from_slice(&v119);
            sub(&w(&mv, off, len), &w(&mf, off, len), &w(&ms, off, len), v45 as i32, &mut v119);
            v130.extend_from_slice(&v119);
            if ty == 1 {
                v130 = vec![0];
            }
            if v26i - v44 >= 10 {
                v44 = if v44 + 16 >= v26i { v26i - 1 } else { v44 + 16 };
                off += 16;
                v45 += 16;
                p10a -= 16;
                p9a -= 1;
                if p9a != 0 {
                    continue;
                }
            }
            break;
        }
    }
    let mut v128 = v132;
    v128.extend_from_slice(&v131);
    v128.extend_from_slice(&v130);
    v128.sort();
    let mut v127 = v128.clone();
    for k in 0..v128.len().saturating_sub(1) {
        if v128[k + 1] == v128[k] {
            v127[k + 1] = 0;
        }
    }
    let mut v128 = v127;
    del0_sorted(&mut v128);
    p13.extend_from_slice(&v128);
    let mut v57 = p13.clone();
    for i in 0..p13.len().saturating_sub(1) {
        if p13[i] != 0 && p13[i] == p13[i + 1] - 1 {
            v57[i] = 0;
        }
    }
    del0_sorted(&mut v57);
    p13 = v57;
    if p13.first().copied() != Some(1) {
        p13.insert(0, 1);
    }
    let n = p13.len();
    let mut v70 = vec![0f64; n];
    let gf = |v: &Vec<f64>, i: i32| if i < 0 { 0.0 } else { v.get(i as usize).copied().unwrap_or(0.0) };
    for i in 0..n {
        let v77 = p13[i];
        let v78 = (gf(&lf, v77 - 1) + gf(&bf, v77 - 1)).mul_add(1.05, gf(&mf, v77 - 1) * 0.90000000000000002);
        if v77 <= 2 {
            v70[i] = if v77 == 1 { 0.0 } else { v78 * T3 };
        } else {
            v70[i] = (gf(&lf, v77 - 2) + v78) * 0.25;
        }
    }
    let mut v71 = p13.clone();
    let v41i = v41;
    if n > 2 {
        for m in 1..n - 1 {
            if v70[m] == 0.0 {
                v70[m] = 0.003;
            }
            if v70[m + 1] == 0.0 {
                v70[m + 1] = 0.003;
            }
            let v81 = v70[m + 1];
            let v88 = v70[m];
            let r = if v81 / v88 < 1.0 { v88 / v81 } else { v81 / v88 };
            if r >= 1.0700000000000001 && p13[m + 1] - p13[m] <= 3 {
                if v81 <= v88 {
                    let v91 = v71[m + 1];
                    if v91 != 5 && v91 != 9 && v91 != v41i && v81 < 1.6000000000000001 {
                        v71[m + 1] = 0;
                    }
                } else {
                    let v90 = v71[m];
                    if v90 != 5 && v90 != 9 && v90 != v41i && v88 < 1.6000000000000001 {
                        v71[m] = 0;
                    }
                }
            }
        }
    }
    let mut out = vec![];
    for (k, x) in p13.iter().enumerate() {
        if k == 0 || v71[k] != 0 {
            out.push(*x);
        }
    }
    Some(out.iter().map(|x| bar - 4 + 4 * x).collect())
}
