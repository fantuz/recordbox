//! Stage D of the phrase analyser port, part 11: build-up features (`UpSimpleloopAna_b`,
//! `UpFeatureAna`, `FillinSnareAna`).

use crate::phrase_rb_d10::{fillin_cut_ana, fillin_snare_ana_ex};
use crate::phrase_rb_dsp::sum8;

fn at(v: &[f64], i: i64) -> f64 {
    if i < 0 {
        0.0
    } else {
        v.get(i as usize).copied().unwrap_or(0.0)
    }
}

/// `UpSimpleloopAna_b`: of each run of kind 31 only the bar with the highest peak keeps it;
/// the others get their kind from `before` back.
pub fn up_simpleloop_b(n: usize, kinds: &mut [i32], before: &[i32], peak: &[f64]) {
    let mut v13 = vec![0i32; n];
    if n <= 1 {
        return;
    }
    let mut v17 = 0i32;
    let mut v18 = n as i64;
    for v20 in 0..n - 1 {
        let v16 = v20 + 1;
        if kinds[v20] == 31 {
            if v16 < n {
                let mut v22 = v20;
                let mut hit_end = false;
                while kinds[v22 + 1] == 31 {
                    v22 += 1;
                    if v22 == n - 1 {
                        hit_end = true;
                        break;
                    }
                }
                if !hit_end {
                    v18 = v22 as i64 + 2;
                }
            }
            if v17 == 0 {
                v17 = (v18 - v16 as i64) as i32;
                v13[v20] = v17;
            }
            v17 -= 1;
        }
    }
    for v24 in 0..n - 1 {
        let l = v13[v24];
        if l >= 2 {
            let mut m = 1i64;
            let mut mx = at(peak, v24 as i64);
            for j in 1..l as i64 {
                if at(peak, v24 as i64 + j) > mx {
                    mx = at(peak, v24 as i64 + j);
                    m = j + 1;
                }
            }
            let keep = v24 as i64 + m;
            for j in v24..v24 + l as usize {
                if j as i64 + 1 != keep && j < kinds.len() {
                    kinds[j] = before.get(j).copied().unwrap_or(0);
                }
            }
        }
    }
}

/// `FillinSnareAna` → (fill start, fill end − 1): the low band halves and comes back.
pub fn fillin_snare_ana(pos: &[i32], p1: usize, lowpeak: &[f64]) -> (i32, i32) {
    let v13 = pos[p1];
    if v13 - pos[p1 - 1] < 16 {
        return (0, 0);
    }
    let v15 = v13 - 16;
    let m = |s: i32| {
        let w: Vec<f64> = (0..4).map(|k| at(lowpeak, (s + k) as i64)).collect();
        sum8(&w) / 4.0
    };
    let (m0, m1, m2, m3) = (m(v13 - 17), m(v15 + 3), m(v15 + 7), m(v15 + 11));
    let v27 = if m1 < m0 * 0.5 {
        if m2 > m1 * 1.25 {
            8
        } else if m3 > m2 * 1.25 {
            12
        } else {
            0
        }
    } else if m2 < m1 * 0.5 && m3 > m2 * 1.25 {
        12
    } else {
        0
    };
    if v27 == 0 {
        return (0, 0);
    }
    (v15 + v27, v13 - 1)
}

/// `UpFeatureAna`: build-ups (kinds 30..33) with a fill before the next section get +300 and
/// their fill beat in `v199`. → (p7, p8) per entry.
#[allow(clippy::too_many_arguments)]
pub fn up_feature_ana(pos: &[i32], kinds: &mut [i32], lowpeak: &[f64], blk: &[f64], v199: &mut [i32], hp: &[f64], p6: i32) -> (Vec<i32>, Vec<i32>) {
    let n = kinds.len();
    let mut v36 = vec![0i32; n.saturating_sub(1)];
    let mut v35 = vec![0i32; n.saturating_sub(1)];
    if n >= 3 {
        for v23 in 0..n - 2 {
            let k = v23 + 1;
            if !(30..=33).contains(&kinds[k]) {
                continue;
            }
            let (a, b) = fillin_snare_ana(pos, v23 + 2, lowpeak);
            v36[k] = a;
            v35[k] = b;
            if a != 0 {
                kinds[k] += 300;
                v199[k] = a;
                continue;
            }
            let (a, b) = fillin_cut_ana(pos, v23 + 2, lowpeak, blk);
            v36[k] = a;
            v35[k] = b;
            let a = if a != 0 { a } else { fillin_snare_ana_ex(pos, hp, p6, v23 + 2) };
            if a != 0 {
                kinds[k] += 300;
                v199[k] = a;
            }
        }
    }
    (v36, v35)
}

/// `HookFeatureAna`: choruses (40) with a fill before them become 440 (fill beat in `v199`),
/// steady 4-beat ones get +1. → (p8, p9) per entry.
#[allow(clippy::too_many_arguments)]
pub fn hook_feature_ana(pos: &[i32], kinds: &mut [i32], lowpeak: &[f64], q: &[f64], blk: &[f64], v199: &mut [i32], hp: &[f64], p7: i32) -> (Vec<i32>, Vec<i32>) {
    let n = kinds.len();
    let mut v41 = vec![0i32; n.saturating_sub(1)];
    let mut v40 = vec![0i32; n.saturating_sub(1)];
    if n >= 3 {
        for v24 in 1..n - 1 {
            if kinds[v24] != 40 {
                continue;
            }
            let (a, b) = fillin_cut_ana(pos, v24 + 1, lowpeak, blk);
            v41[v24] = a;
            v40[v24] = b;
            let a = if a != 0 { a } else { fillin_snare_ana_ex(pos, hp, p7, v24 + 1) };
            if a != 0 {
                kinds[v24] = 440;
                v199[v24] = a;
            }
        }
        for v27 in 1..n - 1 {
            if (kinds[v27] == 40 || kinds[v27] == 440) && crate::phrase_rb_d10::beat4_ana(pos, v27 + 1, lowpeak, q) == 1 {
                kinds[v27] += 1;
            }
        }
    }
    (v41, v40)
}
