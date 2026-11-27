//! Stage D of the phrase analyser port, part 10: intro features (`IntroFeatureAna`,
//! `FillinCutAna`, `FillinSnareAnaEX`, `Beat4Ana`).

use crate::phrase_rb_d1::{findpeaks, sum_range};
use crate::phrase_rb_dsp::sum8;

fn at(v: &[f64], i: i64) -> f64 {
    if i < 0 {
        0.0
    } else {
        v.get(i as usize).copied().unwrap_or(0.0)
    }
}
fn z(v: &mut [f64], i: i64) {
    if i >= 0 {
        if let Some(x) = v.get_mut(i as usize) {
            if *x == 0.0 {
                *x = 0.003;
            }
        }
    }
}
fn sr(v: &[f64], a: i64, b: i64) -> f64 {
    sum_range(v, a as i32, b as i32)
}

/// `FillinCutAna` → (p4, p5): a fill that cuts the low band just before position `p1`.
pub fn fillin_cut_ana(pos: &[i32], p1: usize, lowpeak: &[f64], blk: &[f64]) -> (i32, i32) {
    let mut v14 = blk.to_vec();
    let (mut p4, mut p5) = (0i32, 0i32);
    let v17 = pos[p1] as i64;
    if v17 - (pos[p1 - 1] as i64) < 16 {
        return (0, 0);
    }
    let v16 = [sr(lowpeak, v17 - 17, v17 - 14) * 0.25, sr(lowpeak, v17 - 13, v17 - 10) * 0.25, sr(lowpeak, v17 - 9, v17 - 6) * 0.25, sr(lowpeak, v17 - 5, v17 - 2) * 0.25];
    let v18 = (v17 - 1) as i32;
    let go17;
    if v16[1] < v16[0] * 0.5 {
        p5 = v18;
        p4 = (v17 - 12) as i32;
        go17 = v17 == 12;
    } else if v16[2] >= v16[1] * 0.5 {
        if v16[3] >= v16[2] * 0.5 {
            go17 = p4 == 0;
        } else {
            p5 = v18;
            p4 = (v17 - 4) as i32;
            go17 = v17 == 4;
        }
    } else {
        p5 = v18;
        p4 = (v17 - 8) as i32;
        go17 = v17 == 8;
    }
    if !go17 {
        return (p4, p5);
    }
    let v22 = pos[p1] as i64;
    let x = |v: &[f64], i: i64| at(v, v22 + i);
    // the steady stretch v22-13 .. v22-6
    for k in -13i64..=-7 {
        z(&mut v14, v22 + k);
        let r = x(&v14, k + 1) / x(&v14, k);
        if !(0.69999999999999996..=1.5).contains(&r) {
            return (p4, p5);
        }
    }
    let s = |v: &[f64], a: i64, b: i64| sr(v, v22 + a, v22 + b);
    let found: Option<i64>;
    'chk: {
        // step A: -5 against -6
        let mut v39 = x(&v14, -6);
        let v42 = x(&v14, -5);
        let v44 = v42;
        if v42 < v39 * 0.66000000000000003 {
            let v43 = s(&v14, -9, -6);
            v39 = x(&v14, -6);
            if v42 < v43 * 0.16500000000000001 && x(&v14, -4) * v44 < v44 * 0.80000000000000004 * v39 {
                found = Some(v22 - 4);
                break 'chk;
            }
        }
        if v44 > v39 + v39 && v44 > s(&v14, -9, -6) * 0.5 && x(&v14, -4) * x(&v14, -5) > x(&v14, -5) * (x(&v14, -6) + x(&v14, -6)) {
            found = Some(v22 - 4);
            break 'chk;
        }
        // step B: -4 against -5
        let v46 = x(&v14, -4);
        let v47 = x(&v14, -5);
        let v49 = v46;
        if v46 < v47 * 0.66000000000000003 {
            let v48 = s(&v14, -8, -5) * 0.16500000000000001;
            if v46 < v48 && x(&v14, -3) * v49 < v49 * 0.80000000000000004 * v47 {
                found = Some(v22 - 3);
                break 'chk;
            }
        }
        if !(v49 <= v47 + v47 || v49 <= s(&v14, -8, -5) * 0.5 || x(&v14, -3) * v49 <= v49 * (x(&v14, -5) + x(&v14, -5))) {
            found = Some(v22 - 3);
            break 'chk;
        }
        // step C: -3 against -4
        let v53 = x(&v14, -3);
        let v55 = v53;
        let v49 = x(&v14, -4);
        if v53 < v49 * 0.66000000000000003 {
            let v54 = s(&v14, -7, -4);
            if v53 < v54 * 0.16500000000000001 && x(&v14, -2) * v55 < v55 * 0.80000000000000004 * v49 {
                found = Some(v22 - 2);
                break 'chk;
            }
        }
        if v55 > v49 + v49 && v55 > s(&v14, -7, -4) * 0.5 && x(&v14, -2) * x(&v14, -3) > x(&v14, -3) * (x(&v14, -4) + x(&v14, -4)) {
            found = Some(v22 - 2);
            break 'chk;
        }
        // step D: -2 against -3
        let v56 = x(&v14, -2);
        let v57 = x(&v14, -3);
        if v56 < v57 * 0.66000000000000003 {
            let v59 = s(&v14, -6, -3);
            if v56 < v59 * 0.16500000000000001 && x(&v14, -1) * v56 < v56 * 0.80000000000000004 * v57 {
                found = Some(v22 - 1);
                break 'chk;
            }
        }
        if v56 <= v57 + v57 || v56 <= s(&v14, -6, -3) * 0.5 || x(&v14, -1) * v56 <= v56 * (x(&v14, -3) + x(&v14, -3)) {
            found = None;
            break 'chk;
        }
        found = Some(v22 - 1);
    }
    if let Some(f) = found {
        p5 = v18;
        p4 = f as i32;
    }
    (p4, p5)
}

/// `FillinSnareAnaEX` → p4: a snare roll (evenly spaced high-band peaks) before position `p3`.
pub fn fillin_snare_ana_ex(pos: &[i32], hp: &[f64], p2: i32, p3: usize) -> i32 {
    let v13 = p2 / 4;
    let mut v48 = vec![0i32; v13.max(0) as usize];
    if p2 >= 20 {
        let v47 = v13 - 4;
        let v16 = if v47 <= 1 { 1 } else { v47 };
        for v15 in 0..v16 as usize {
            let w: Vec<f64> = (0..16).map(|i| at(hp, (32 * v15 + 16 + i) as i64)).collect();
            let (vals, locs, _, _) = findpeaks(&w);
            let v20 = vals.len();
            let mut v34 = 0usize;
            if v20 >= 3 {
                let mut v59: Vec<f64> = vec![0.0; 8];
                let mut v53: Vec<i32> = vec![0; 8];
                let mut v52: Vec<i32> = vec![0; 8];
                let mut v26 = 0usize;
                for v25 in 0..v20 - 1 {
                    let v28 = locs[v25 + 1] - locs[v25];
                    let r = vals[v25 + 1] / vals[v25];
                    if (2..=4).contains(&v28) && r > 0.69999999999999996 && r < 1.4299999999999999 {
                        if v26 >= v59.len() {
                            v59.resize(v26 + 1, 0.0);
                            v53.resize(v26 + 1, 0);
                            v52.resize(v26 + 1, 0);
                        }
                        v52[v26] = v28;
                        v59[v26] = vals[v25];
                        v53[v26] = v25 as i32 + 1;
                        v26 += 1;
                    } else {
                        v26 = 0;
                    }
                }
                if v26 >= 2 {
                    v59.push(vals.get(v53[v26 - 1] as usize).copied().unwrap_or(0.0));
                    let v35 = v59.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    v34 = if v35 >= 0.10000000000000001 { v26 } else { 0 };
                    if v34 != 0 && v35 < 0.34999999999999998 && (1..v34).any(|k| v52[k - 1] != v52[k]) {
                        v34 = 0;
                    }
                }
            }
            if v15 < v48.len() {
                v48[v15] = v34 as i32;
            }
        }
    }
    let v41 = pos[p3];
    let v42 = v41 - pos[0];
    let v43 = v42 / 4;
    let g = |i: i32| if i < 0 { 0 } else { v48.get(i as usize).copied().unwrap_or(0) };
    if v48.len() >= 4 && v42 >= 8 && g(v43 - 1) >= 2 && g(v43 - 2) <= 1 {
        return v41 - 2;
    }
    0
}

/// `Beat4Ana` → 1 when the section before position `p1` keeps a steady 4-beat pattern.
pub fn beat4_ana(pos: &[i32], p1: usize, lowpeak: &[f64], q: &[f64]) -> i32 {
    let mut p4 = 0;
    let v14 = pos[p1 - 1];
    let v15 = pos[p1];
    let v16 = if v15 - 2 - (v14 - 1) < 0 { 0.0 } else { sum_range(lowpeak, v14 - 1, v15 - 2) / (v15 - 2 - (v14 - 1) + 1) as f64 };
    let v17 = 4 * v14;
    let v18 = 4 * v15 - 4 * v14;
    let v19 = v18 - 1;
    let v22 = if v19 < 49 || pos.len() - 1 == p1 { v19 } else { v18 - 33 };
    let v23 = (v22 / 4).max(0) as usize;
    if v22 < 4 {
        return 0;
    }
    let mut v24 = vec![0i32; v23.max(2)];
    let mut v25 = vec![0f64; v23];
    let mut v27 = vec![0f64; v23.max(2)];
    if v22 >= 8 {
        let v28 = v23.max(2);
        for c in 0..v28 - 1 {
            let base = (v17 + 4 * c as i32) as i64;
            let mut v57 = 1i32;
            let mut v33 = at(q, base);
            for j in 1..4 {
                if at(q, base + j) > v33 {
                    v33 = at(q, base + j);
                    v57 = j as i32 + 1;
                }
            }
            let v35 = if v33 != 0.0 { v33 } else { 0.003 };
            let v36 = v24[c];
            let v37 = (v57 - v36).abs();
            if c + 1 < v24.len() {
                v24[c + 1] = v57;
            }
            if c + 1 < v27.len() {
                v27[c + 1] = v33;
            }
            if v57 == v36 {
                if c < v25.len() {
                    v25[c] = 1.0;
                }
            } else if v37 & 0x7FFFFFFD == 1 {
                if c < v25.len() {
                    v25[c] = at(q, base - 1 + v36 as i64) / v35;
                }
            }
        }
    }
    if !v25.is_empty() && sum8(&v25) / v25.len() as f64 > 0.80000000000000004 {
        p4 = 1;
    } else {
        let v40 = v23 as i32 - 4;
        let v41 = if v22 >= 16 { v40.max(0) as usize } else { 0 };
        let mut v42 = vec![0f64; v41.max(3)];
        if v22 >= 28 {
            let v43 = v40.max(3) as usize;
            for k in 2..v43 {
                if k >= v27.len() {
                    break;
                }
                if v27[k] == 0.0 {
                    v27[k] = 0.003;
                }
                if v27[k - 1] == 0.0 {
                    v27[k - 1] = 0.003;
                }
                let (a, b) = (v27[k], v27[k - 1]);
                if k < v42.len() {
                    v42[k] = if a < b { b / a } else { a / b };
                }
            }
        }
        v42.truncate(v41.max(0));
        if v41 >= 3 && sum_range(&v42, 2, v41 as i32 - 1) / (v41 as f64 - 2.0) < 1.1200000000000001 {
            p4 = 1;
        }
    }
    if v16 < 0.22 {
        p4 = 0;
    }
    p4
}

/// `IntroFeatureAna`: marks the intro 110 (fill) and +1 (steady 4-beat) → (kinds, v199[0], p8, p9).
#[allow(clippy::too_many_arguments)]
pub fn intro_feature_ana(pos: &[i32], kinds: &mut [i32], lowpeak: &[f64], q: &[f64], blk: &[f64], v199: &mut [i32], hp: &[f64], p7: i32) -> (i32, i32) {
    let n = kinds.len();
    let (mut p8, mut p9) = (0, 0);
    if n <= 1 {
        return (p8, p9);
    }
    let v16 = n - 1;
    let (mut v21, mut v22) = (0, 0);
    for v20 in 0..v16 {
        if kinds[v20] == 10 {
            let (a, b) = fillin_cut_ana(pos, 1, lowpeak, blk);
            p8 = a;
            p9 = b;
            v22 = a;
            v21 = b;
            if p8 == 0 {
                p8 = fillin_snare_ana_ex(pos, hp, p7, v20 + 1);
                p9 = 0;
                v22 = 0;
            }
            if p8 != 0 {
                kinds[0] = 110;
                if let Some(x) = v199.get_mut(0) {
                    *x = p8;
                }
            }
        }
    }
    p8 = v22;
    p9 = v21;
    for v26 in 0..v16 {
        if kinds[v26] == 10 || kinds[v26] == 110 {
            if beat4_ana(pos, v26 + 1, lowpeak, q) == 1 {
                kinds[v26] += 1;
            }
        }
    }
    (p8, p9)
}
