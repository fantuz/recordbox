//! Stage D of the phrase analyser port, part 12: where the outro starts (`OutroFeatureAna`,
//! `OutroKickoutAna`).

use crate::phrase_rb_d1::sum_range;
use crate::phrase_rb_d10::beat4_ana;
use crate::phrase_rb_dsp::{max_val, sum8};

fn at(v: &[f64], i: i64) -> f64 {
    if i < 0 {
        0.0
    } else {
        v.get(i as usize).copied().unwrap_or(0.0)
    }
}
fn z(v: &mut [f64], i: i64) -> f64 {
    if i < 0 {
        return 0.0;
    }
    match v.get_mut(i as usize) {
        Some(x) => {
            if *x == 0.0 {
                *x = 0.003;
            }
            *x
        }
        None => 0.0,
    }
}

/// `OutroKickoutAna`: for each outro (60) entry, the beat after the last kick when it stops
/// within the last 12 beats, else 0.
pub fn outro_kickout_ana(pos: &[i32], kinds: &[i32], hits: &[u8]) -> Vec<i32> {
    let n = pos.len();
    let mut p3 = vec![0i32; n];
    for v14 in 0..n {
        if kinds.get(v14).copied() != Some(60) {
            continue;
        }
        let v22 = pos[v14];
        let v23 = hits.len() as i32 / 4 - v22;
        let v18 = v23 + 1;
        let mut v19 = v23 + 1;
        if v23 >= 0 {
            let mut v25 = vec![0u8; (v23 + 1) as usize];
            for (c, f) in v25.iter_mut().enumerate() {
                let s = 4 * v22 as i64 - 4 + 4 * c as i64;
                let sum: u32 = (0..4).map(|k| if s + k < 0 { 0 } else { hits.get((s + k) as usize).copied().unwrap_or(0) as u32 }).sum();
                if sum != 0 {
                    *f = 1;
                }
            }
            if let Some(c) = v25.iter().rposition(|x| *x == 1) {
                v19 = c as i32 + 1;
            }
        }
        p3[v14] = if v19 <= v23 - 12 || v19 == v18 { 0 } else { v19 + v22 };
    }
    p3
}

/// `OutroFeatureAna` → (kinds, outro list): picks the outro start beat (stored at the last
/// entry) and marks a steady 4-beat outro with +1.
pub fn outro_feature_ana(pos: &[i32], kinds: &[i32], lowpeak: &[f64], hits: &[u8], q: &[f64], bar: i32) -> (Vec<i32>, Vec<i32>) {
    let mut v108 = pos.to_vec();
    let mut v106 = kinds.to_vec();
    let mut v19 = lowpeak.to_vec();
    let v20 = v106.len();
    let ko = outro_kickout_ana(&v108, &v106, hits);
    let mut v22 = ko.get(v20.wrapping_sub(1)).copied().unwrap_or(0);
    if v22 != 0 {
        let a = z(&mut v19, v22 as i64 - 1);
        let b = z(&mut v19, v22 as i64 - 2);
        if a / b > 0.80000000000000004 || sum_range(&v19, v22 - 5, v22 - 2) * 0.25 < 0.050000000000000003 {
            v22 = 0;
        }
    }
    let mut v101 = v22;
    let v118 = v19.len() as i64;
    let v29 = *v108.last().unwrap_or(&0) as i64;
    let v30 = v29 + 9;
    let mut v28 = v118;
    let mut v26 = 0i64;
    while v118 + v26 >= v30 {
        let v31 = at(&v19, v118 - 1 + v26);
        v26 -= 1;
        if v31 > 0.00069999999999999999 {
            v28 = v118 + v26 + 2;
            break;
        }
    }
    let v100 = v28;
    let v32 = if v28 >= v30 { (v28 - v30) as usize } else { 0 };
    let mut v102 = vec![0f64; v32];
    let mut v96 = vec![0f64; v32];
    let st = (v30 - 1).max(0) as usize;
    let mut v33: Vec<f64> = if st <= v19.len() { v19[st..].to_vec() } else { vec![] };
    if v32 >= 2 {
        let b = z(&mut v33, 1);
        let a = z(&mut v33, 0);
        v102[1] = b / a;
        for i in 2..v32 {
            let x = z(&mut v33, i as i64);
            let y = z(&mut v33, i as i64 - 1);
            let r = x / y;
            v102[i] = r;
            v96[i] = r / v102[i - 1];
        }
    }
    let v43 = v32;
    if v43 >= 2 {
        let mut v44 = 1usize;
        loop {
            let v49 = v44 + 1;
            let mut to59 = false;
            if v102[v44] < 0.5 {
                let v51: Vec<f64> = (v44 + 1..v43).filter(|j| at(&v33, *j as i64) > 0.001).map(|j| v102[j]).collect();
                let high = !v51.is_empty() && sum8(&v51) / v51.len() as f64 > 0.85999999999999999;
                if !high {
                    if v49 >= v43 {
                        to59 = true;
                    } else {
                        to59 = !(v44 + 1..v43).any(|j| v102[j] > 1.1000000000000001 && at(&v33, j as i64) > 0.001);
                    }
                }
            }
            if to59 {
                let v58 = v44 as i64 + v30;
                if v101 == 0 || v58 < v101 as i64 {
                    v101 = v58 as i32;
                    if v58 < v100 - 3 {
                        let v59 = v58;
                        let v63 = (1 - bar as i64 + v58) % 4;
                        if v63 == 2 {
                            if v44 >= 2 && (v102[v44 - 1] > 1.3 || v102[v44 - 2] > 1.3) {
                                v101 = (v59 - 2) as i32;
                            }
                        } else if v63 == 3 && v44 >= 2 && at(&v102, v44 as i64 - 2) > 1.3 && at(&v102, v44 as i64 - 3) > 1.3 {
                            v101 = (v59 - 3) as i32;
                        }
                    }
                }
            }
            v44 = v49;
            if v49 >= v43 {
                break;
            }
        }
        let mut v85 = 1usize;
        'outer: while v85 < v43 {
            if at(&v33, v85 as i64) <= 0.20000000000000001 || v102[v85] <= 2.0 {
                v85 += 1;
                continue;
            }
            let mut v88 = v85 + 1;
            while v88 < v43 && at(&v33, v88 as i64) >= 0.00069999999999999999 {
                let v89 = v102[v88];
                v88 += 1;
                if v89 > 1.0 {
                    v85 += 1;
                    continue 'outer;
                }
            }
            let c = v30 + v85 as i64;
            v101 = if v101 as i64 > c || v101 == 0 { c as i32 } else { v101 };
            break;
        }
    }
    // LABEL_76
    let v113 = v32;
    let mut v79 = v101;
    if v113 >= 4 {
        let mut v73 = 1usize;
        loop {
            let v74 = v73;
            v73 += 1;
            let cand = v96[v74] > 1.3 && v96[v73] < 1.03 && at(&v96, v74 as i64 + 2) < 1.03;
            if cand {
                if v74 + 4 >= v113 {
                    break;
                }
                if !(v74 + 3..v113).any(|j| v96[j] > 0.91000000000000003) {
                    let c = v30 + v74 as i64;
                    v79 = if v101 as i64 > c || v101 == 0 { c as i32 } else { v101 };
                    break;
                }
            }
            if v73 == v113 - 2 {
                break;
            }
        }
    }
    let v70 = v100 as i32;
    let mut v80 = if v79 != 0 { v79 } else { v70 };
    if (v80 - bar + 1) & 3 == 0 && v80 < v70 {
        v80 += 1;
    }
    let mut out = vec![0i32; v108.len()];
    if v20 >= 1 {
        if let Some(x) = out.get_mut(v20 - 1) {
            *x = v80;
        }
    }
    v108.push(v80);
    if v20 >= 1 && beat4_ana(&v108, v20, &v19, q) == 1 {
        v106[v20 - 1] += 1;
    }
    (v106, out)
}

/// `FillAdjustAna`: drops fills that sit in a silent stretch (kinds lose their +100/+300/+400
/// and the fill beat is cleared). → v193 (1 where the fill is kept as a break).
pub fn fill_adjust_ana(pos: &[i32], kinds: &mut [i32], fills: &mut [i32], cnt: usize, lowpeak: &[f64], s3696: &[f64]) -> Vec<i32> {
    let n = lowpeak.len();
    let mut v87 = vec![0f64; n];
    let g = |k: usize| s3696.get(k).copied().unwrap_or(0.0);
    let pairs = if n != 1 { n & !1 } else { 0 };
    for (k, d) in v87.iter_mut().enumerate() {
        let (a, b, c, e) = (g(4 * k), g(4 * k + 1), g(4 * k + 2), g(4 * k + 3));
        *d = if k < pairs { c + ((a + b) + e) } else { ((a + b) + c) + e };
    }
    let m4 = max_val(lowpeak);
    let m7 = max_val(&v87);
    let f4 = if m4 == 0.0 { 333.33333333333331 } else { 1.0 / m4 };
    let f7 = if m7 == 0.0 { 333.33333333333331 } else { 1.0 / m7 };
    let v86: Vec<f64> = lowpeak.iter().map(|x| x * f4).collect();
    let v85: Vec<f64> = v87.iter().map(|x| x * f7).collect();
    let mut v84 = vec![0i32; cnt];
    let mut v193 = vec![0i32; cnt];
    if cnt >= 2 {
        for v47 in 0..cnt - 1 {
            let v56 = fills.get(v47).copied().unwrap_or(0);
            if v56 == 0 {
                continue;
            }
            let v57 = pos.get(v47 + 1).copied().unwrap_or(0);
            if v57 - v56 < 3 {
                continue;
            }
            let a = |i: i32| at(&v86, i as i64);
            let b = |i: i32| at(&v85, i as i64);
            let v60 = [2, 3, 4, 5].iter().filter(|d| a(v57 - **d) < 0.14999999999999999).count();
            if v60 >= 3 && b(v57 - 3) < 0.040000000000000001 && b(v57 - 4) < 0.040000000000000001 && (b(v57 - 2) < 0.040000000000000001 || b(v57 - 5) < 0.040000000000000001) {
                v84[v47] = 1;
                if v56 >= 9 && (2..=9).filter(|d| a(v56 - d) > 0.40000000000000002).count() >= 6 {
                    v84[v47] = 2;
                    v193[v47] = 1;
                }
            }
        }
        if v84[..cnt - 1].iter().any(|x| *x == 1 || *x == 2) {
            for v79 in 0..cnt - 1 {
                if v84[v79] == 1 {
                    if let Some(f) = fills.get_mut(v79) {
                        *f = 0;
                    }
                    let k = kinds[v79];
                    kinds[v79] = if k > 400 { k - 400 } else if k > 300 { k - 300 } else if k >= 101 { k - 100 } else { k };
                }
            }
        }
    }
    v193
}
