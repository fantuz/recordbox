//! Stage D of the phrase analyser port, part 14: the Mid/Low-mood structure — which sections
//! repeat (`ABmeloAnalyze`), from the self-similarity of the per-half-beat chroma rows.

use crate::phrase_rb_c::Mat2;

/// 8-lane accumulate as the vector loops do: lane 0 starts at `start`, the others at 0;
/// `add(x, acc)` adds one element (fused or not, as the reference does).
fn lanes8(xs: &[f64], start: f64, add: impl Fn(f64, f64) -> f64) -> f64 {
    let n = xs.len();
    if n < 8 {
        let mut r = start;
        for x in xs {
            r = add(*x, r);
        }
        return r;
    }
    let n8 = n & !7;
    let mut acc = [0f64; 8];
    acc[0] = start;
    for c in 0..n8 / 8 {
        for (j, a) in acc.iter_mut().enumerate() {
            *a = add(xs[8 * c + j], *a);
        }
    }
    let mut r = (acc[6] + (acc[4] + (acc[2] + acc[0]))) + (acc[7] + (acc[5] + (acc[3] + acc[1])));
    for x in &xs[n8..] {
        r = add(*x, r);
    }
    r
}

/// `ABmeloAnalyze` → flags[i][j] (segments i < j that repeat each other), (n−1)×(n−1) with
/// n = boundaries + 1.
pub fn abmelo(p0: &Mat2, pos: &[i32], bar: i32, nbeats: i32) -> Option<Vec<Vec<u8>>> {
    let mut v191 = pos.to_vec();
    v191.push(nbeats);
    let rows = p0.rows;
    let cols = p0.cols;
    // similarity of every pair of rows
    let mut v189 = vec![vec![0f64; rows]; rows.saturating_sub(1)];
    let mean = |r: &[f64]| if r.is_empty() { 0.0 } else { crate::phrase_rb_dsp::sum8(r) / r.len() as f64 };
    if rows >= 2 {
        for i in 0..rows - 1 {
            let a = p0.row(i);
            let mi = mean(a);
            for j in i + 1..rows {
                let b = p0.row(j);
                let mj = if cols < 1 { 0.0 } else { mean(b) };
                if cols < 1 {
                    v189[i][j] = 65535.0;
                    continue;
                }
                let d: Vec<f64> = (0..cols).map(|k| (mj + a[k]) - (mi + b[k])).collect();
                let s = lanes8(&d, 0.0, |x, acc| x.mul_add(x, acc));
                v189[i][j] = if s == 0.0 { 65535.0 } else { 1.0 / s };
            }
        }
    }
    let v192 = v191.len();
    if v192 < 1 {
        return None;
    }
    let v53 = v192 - 1;
    let mut v186 = vec![vec![0f64; v53]; v53];
    let mut out = vec![vec![0u8; v53]; v53];
    let v56: Vec<i32> = v191.iter().map(|x| (((x - bar) as f64).mul_add(0.5, 1.0) as i32).max(1)).collect();
    let v190 = rows as i32;
    let g189 = |r: i32, c: i32| -> f64 {
        if r < 0 || c < 0 {
            return 0.0;
        }
        v189.get(r as usize).and_then(|x| x.get(c as usize)).copied().unwrap_or(0.0)
    };
    if v192 >= 3 {
        for v61 in 0..v192 - 2 {
            for v63 in v61 + 1..v53 {
                let v64 = v56[v61];
                let mut v66 = v56[v61 + 1] - v64;
                let v67 = v56[v63];
                if v66 >= v56[v63 + 1] - v67 {
                    v66 = v56[v63 + 1] - v67;
                }
                let v68 = if v66 + v67 <= v190 { v66 } else { v190 - v67 };
                let mut v70 = 0.0;
                if v68 >= 1 {
                    for k in 0..v68 {
                        v70 = g189(k + v64, k + v67) + v70;
                    }
                }
                if v68 != 0 {
                    v186[v61][v63] = v70 / v68 as f64;
                }
            }
        }
    }
    // mean and spread of the positive similarities
    let mut total = 0.0;
    let mut cnt = 0usize;
    for r in &v186 {
        total = lanes8(r, total, |x, acc| x.max(-0.0) + acc);
        cnt += r.iter().filter(|x| **x > 0.0).count();
    }
    let m = if cnt >= 1 { total / cnt as f64 } else { total };
    let mut ss = 0.0;
    let mut c2 = 0usize;
    for r in &v186 {
        ss = lanes8(r, ss, |x, acc| (if x > 0.0 { (x - m) * (x - m) } else { -0.0 }) + acc);
        c2 += r.iter().filter(|x| **x > 0.0).count();
    }
    let sd = if c2 < 2 { ss } else { (ss / (c2 - 1) as f64).sqrt() };
    if v186.is_empty() {
        return Some(out);
    }
    let thr = sd + m;
    if v192 > 2 {
        for i in 0..v192 - 2 {
            for j in i + 1..v53 {
                out[i][j] = (v186[i][j] >= thr) as u8;
            }
        }
    }
    Some(out)
}

fn normalize(v: &mut [f64], n: usize) {
    let mx = crate::phrase_rb_dsp::max_val(v);
    let f = if mx == 0.0 { 333.33333333333331 } else { 1.0 / mx };
    for x in v.iter_mut().take(n) {
        *x *= f;
    }
}

/// `HooksoloAnalyze` → per section 10 / 40 (hook) / 50 / 0, from the middle band, the level
/// and the vocal presence per section.
pub fn hooksolo(mid: &[f64], lvl: &[f64], pos: &[i32], uta: &[f64], mood: i32) -> Option<Vec<i32>> {
    let crate_sum = |x: &[f64], a: i32, b: i32| crate::phrase_rb_d1::sum_range(x, a, b);
    let mut v135 = lvl.to_vec();
    if 4 * mid.len() > v135.len() {
        v135.resize(4 * mid.len(), 0.0);
    }
    let mut v133 = mid.to_vec();
    let n133 = v133.len();
    normalize(&mut v133, n133);
    let mut v132 = uta.to_vec();
    let n132 = v132.len();
    normalize(&mut v132, n132);
    let mut v142 = pos.to_vec();
    if !v142.is_empty() {
        let v45 = v142[0];
        v142[0] = 1;
        for x in v142.iter_mut().skip(1) {
            *x = (*x - v45) / 4 + 1;
        }
    }
    let pn = pos.len();
    let mut v47 = vec![0f64; pn];
    let mut v48 = vec![0f64; pn];
    let mut v49 = vec![0f64; pn];
    if crate::phrase_rb_dsp::sum8(&v133) == 0.0 {
        return Some(vec![0; v142.len()]);
    }
    crate::phrase_rb_d8::sort_del0(&mut v142);
    let mut v146 = v133.clone();
    v146.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let nz = v146.iter().filter(|x| **x != 0.0).count();
    v142.push(nz as i32 - 2);
    let v143 = v142.len();
    let v139 = v143 - 1;
    let seg = |src: &[f64], a: i32, b: i32, single: i32| -> f64 {
        if b - a - 1 < 2 {
            src.get(single.max(0) as usize).copied().unwrap_or(0.0)
        } else {
            0.0
        }
    };
    let _ = seg;
    if v143 >= 2 {
        for k in 0..v139 {
            let (v56, v57) = (4 * v142[k], 4 * v142[k + 1]);
            v47[k] = if v57 - v56 - 1 < 2 {
                v135.get((v57 - 5).max(0) as usize).copied().unwrap_or(0.0)
            } else if (v57 - 5) - (v56 - 4) < 0 {
                0.0
            } else {
                crate_sum(&v135, v56 - 4, v57 - 5) / ((v57 - 5) - (v56 - 4) + 1) as f64
            };
        }
        normalize(&mut v47, v139);
        for k in 0..v139 {
            let (v73, v74) = (4 * v142[k], 4 * v142[k + 1]);
            v49[k] = if v74 - v73 - 1 < 2 {
                v132.get((v74 - 5).max(0) as usize).copied().unwrap_or(0.0)
            } else if (v74 - 5) - (v73 - 4) < 0 {
                0.0
            } else {
                crate_sum(&v132, v73 - 4, v74 - 5) / ((v74 - 5) - (v73 - 4) + 1) as f64
            };
        }
        normalize(&mut v49, v139);
        for k in 0..v139 {
            let (v91, v90) = (v142[k], v142[k + 1]);
            v48[k] = if v90 - v91 - 1 < 2 {
                v133.get((v90 - 2).max(0) as usize).copied().unwrap_or(0.0)
            } else if (v90 - 2) - (v91 - 1) < 0 {
                0.0
            } else {
                crate_sum(&v133, v91 - 1, v90 - 2) / ((v90 - 2) - (v91 - 1) + 1) as f64
            };
        }
        normalize(&mut v48, v139);
    }
    let v104 = v143 - 1;
    let mut p5 = vec![0i32; v104];
    if p5.is_empty() {
        return Some(p5);
    }
    p5[0] = 10;
    let mut v148 = v49.clone();
    v148.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let k0 = v148.iter().take_while(|x| **x == 0.0).count();
    v148.drain(..k0);
    let v107 = if v148.is_empty() { 0.0 } else { crate::phrase_rb_dsp::sum8(&v148) / v148.len() as f64 };
    let v106 = if v47.is_empty() { 0.0 } else { crate::phrase_rb_dsp::sum8(&v47) / v47.len() as f64 };
    if v143 <= 1 {
        return Some(p5);
    }
    let hook = |s: f64, u: f64| !(v106 <= 0.80000000000000004 || s <= 0.92000000000000004) || (s > 0.91000000000000003 && v106 <= 0.80000000000000004) || (s > 0.80000000000000004 && u > 0.88);
    {
        let (m, u) = (v48[0], v49[0]);
        let go = if m < 0.40000000000000002 { false } else if v107 < 0.40000000000000002 { true } else { u >= 0.40000000000000002 };
        if go && hook(v47[0], v49[0]) {
            p5[0] = 40;
        }
    }
    if v104 == 1 {
        return Some(p5);
    }
    for k in 1..v104 {
        let (m, u) = (v48[k], v49[k]);
        let go = if m < 0.40000000000000002 || v107 < 0.40000000000000002 { m >= 0.40000000000000002 && v107 < 0.40000000000000002 } else { !(u < 0.40000000000000002 && v107 >= 0.40000000000000002) };
        if !go {
            continue;
        }
        if hook(v47[k], u) {
            p5[k] = 40;
            continue;
        }
        if v49[k] == 0.0 {
            v49[k] = 0.003;
        }
        if v49[k - 1] == 0.0 {
            v49[k - 1] = 0.003;
        }
        let v125 = v49[k];
        let mut v126 = v47[k];
        if v126 > 0.84999999999999998 && v125 > 0.65000000000000002 && v125 / v49[k - 1] > 1.3 {
            p5[k] = 40;
            continue;
        }
        if v126 == 0.0 {
            v47[k] = 0.003;
            v126 = 0.003;
        }
        if v47[k - 1] == 0.0 {
            v47[k - 1] = 0.003;
        }
        if (v126 > 0.84999999999999998 && v126 / v47[k - 1] > 1.5) || (p5[k - 1] == 40 && v126 / v47[k - 1] > 0.88) {
            p5[k] = 40;
        }
    }
    if v143 < 3 {
        return Some(p5);
    }
    for k in 1..v104 {
        let v129 = p5[k - 1];
        let mut v130 = if v129 == 40 { 0.62 } else { 0.54000000000000004 };
        let v131 = if v129 != 40 { 0.5 } else { 0.59999999999999998 };
        if mood == 2 {
            v130 = v131;
        }
        if v129 == 10 && v49[k] == 0.0 {
            if p5[k] != 40 {
                p5[k] = 10;
            }
        } else if p5[k] != 40 && v49[k] < v49[k - 1] * v130 {
            p5[k] = 50;
        }
    }
    Some(p5)
}

/// `TenkaimeiAnalyze`: sections that repeat each other share a label; the rest get new verse
/// labels 61, 62, …; the last is the outro (60) unless it is a hook.
pub fn tenkaimei(rep: &[Vec<u8>], data: &mut [i32]) {
    let n = data.len();
    let line = |r: usize, c: usize| rep.get(r).and_then(|x| x.get(c)).copied().unwrap_or(0);
    let mut v7 = vec![0u8; n];
    let mut v22 = 61;
    if n >= 2 {
        for v9 in 0..n - 1 {
            for i in v9 + 1..n {
                if line(v9, i) != 1 {
                    continue;
                }
                if data[v9] == 50 {
                    if data[i] == 40 {
                        data[v9] = 0;
                        v7[v9] = 1;
                    }
                } else if data[v9] == 40 && data[i] == 50 {
                    data[i] = 0;
                    v7[i] = 1;
                }
            }
        }
        for v25 in 0..n - 1 {
            let mut v28 = false;
            for col in v25 + 1..n {
                if line(v25, col) != 1 {
                    continue;
                }
                let v34 = data[v25];
                let v35 = data[col];
                if v34 != 0 {
                    if v35 == 0 && v34 != 10 && v34 != 50 {
                        if v34 == 40 {
                            if v7[col] == 0 {
                                data[col] = 40;
                            }
                        } else {
                            data[col] = v34;
                        }
                    }
                } else if v35 > 39 {
                    if v35 == 40 {
                        if v7[v25] == 0 {
                            data[v25] = 40;
                        }
                    } else if v35 != 50 {
                        data[v25] = v35;
                    }
                } else if v35 != 0 {
                    if v35 != 10 {
                        data[v25] = v35;
                    }
                } else {
                    data[v25] = v22;
                    data[col] = v22;
                    v28 = true;
                }
            }
            v22 += v28 as i32;
        }
    }
    if n >= 1 && data[n - 1] != 40 {
        data[n - 1] = 60;
    }
    if n >= 3 {
        for x in data.iter_mut().take(n - 1).skip(1) {
            if *x == 0 {
                *x = v22;
                v22 += 1;
            }
        }
    }
}

/// `YokuyoAnalyze` → (flag, score): whether the level swings between sections (PSSI byte 28 on
/// the Mid/Low path).
pub fn yokuyo(lvl: &[f64], pos: &[i32]) -> (i32, f64) {
    let n = pos.len();
    if n <= 2 {
        return (0, 0.0);
    }
    let mut v10 = pos.to_vec();
    if (lvl.len() as i32) < v10[n - 1] {
        v10[n - 1] = lvl.len() as i32;
    }
    let sr = |x: &[f64], a: i32, b: i32| crate::phrase_rb_d1::sum_range(x, a, b);
    let mut v12 = vec![0f64; n - 1];
    for k in 0..n - 1 {
        let (a, b) = (v10[k] - 1, v10[k + 1] - 1);
        v12[k] = if b - a < 0 { 0.0 } else { sr(lvl, a, b) / (b - a + 1) as f64 };
    }
    let v22 = (n - 1) as i32;
    let mut v23 = if v22 > 2 { sr(&v12, 1, v22 - 2) / (v22 - 2) as f64 } else { 0.0 };
    let v25 = v12[1];
    let v26 = v23 * 0.5;
    if v25 < v26 {
        v23 = if n >= 4 { sr(&v12, 2, n as i32 - 2) / (n as f64 - 3.0) } else { 0.0 };
    }
    let v60 = v23;
    let low = v25 < v26 && n > 3;
    let v30 = if low { 3 } else { 2 };
    let v31 = if low { 0.074999999999999997 } else { 0.089999999999999997 };
    if v30 >= v22 {
        return (0, 0.0);
    }
    let v35 = (v22 - 1).max(v30);
    let v36 = (v30 - 1) as usize;
    let v37 = (v35 - v30 + 1) as usize;
    let v32: Vec<f64> = (0..v37).map(|j| (v60 - v12[v36 + j]).abs()).collect();
    let mut v33 = vec![];
    let mut v47 = v10[v36];
    for j in 0..v37 {
        let v50 = v10[v36 + 1 + j];
        if v50 - v47 > 16 || v12[v36 + j] > v60 * 0.69999999999999996 {
            v33.push(v32[j]);
        }
        v47 = v50;
    }
    if v33.is_empty() {
        return (0, 0.0);
    }
    let v52 = sr(&v33, 0, v33.len() as i32 - 1) / v33.len() as f64;
    ((v52 >= v31) as i32, v52)
}
