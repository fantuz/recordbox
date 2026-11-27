//! Stage D of the phrase analyser port, part 5: verse/hook candidate data
//! (`VerseHookAnaData23`) and the low-band candidates (`VerseHookJudge_LowPeak`).

use crate::phrase_rb_d1::sum_range;
use crate::phrase_rb_dsp::max_val;

/// A normalised series with its bar-to-bar ratios: (values, folded ratios ≥ 1, signed ratios).
#[derive(Clone, Debug, Default)]
pub struct Ratios {
    pub v: Vec<f64>,
    pub fold: Vec<f64>,
    pub sign: Vec<f64>,
}

/// Normalise the first `scale_n` values by the maximum (333.33 when it is 0) and take `cnt`
/// ratios (zeros become 0.003 in place first).
fn ratios(mut v: Vec<f64>, scale_n: usize, cnt: usize) -> Ratios {
    let n = v.len();
    let mx = max_val(&v);
    let f = if mx == 0.0 { 333.33333333333331 } else { 1.0 / mx };
    for x in v.iter_mut().take(scale_n.min(n)) {
        *x *= f;
    }
    let mut fold = vec![0f64; n];
    let mut sign = vec![0f64; n];
    for k in 0..cnt {
        if k + 1 >= n {
            break;
        }
        if v[k] == 0.0 {
            v[k] = 0.003;
        }
        if v[k + 1] == 0.0 {
            v[k + 1] = 0.003;
        }
        let r = v[k + 1] / v[k];
        sign[k + 1] = r;
        fold[k + 1] = if r < 1.0 { 1.0 / r } else { r };
    }
    Ratios { v, fold, sign }
}

/// `VerseHookAnaData23` → (low-peak bars, low level, middle peak, per-beat 3832 series).
pub fn verse_hook_ana_data23(lowpeak: &[f64], lowlevel: &[f64], s3832: &[f64], midpeak: &[f64], bar: i32) -> Option<[Ratios; 4]> {
    if lowpeak.is_empty() {
        return None;
    }
    let v31 = lowpeak.len() as i32 - bar + 1;
    let v33 = (v31 / 4).max(0) as usize;
    let mut bars = vec![0f64; v33];
    let cnt = (v33 as i32 - 1).max(1) as usize;
    if v31 >= 8 {
        let mut s = bar - 1;
        for k in 0..cnt {
            if k < bars.len() {
                bars[k] = sum_range(lowpeak, s, s + 3) * 0.25;
            }
            s += 4;
        }
    }
    let g1 = if v31 >= 8 {
        let rc = if v31 >= 12 { v33.max(3) - 2 } else { 0 };
        ratios(bars, cnt, rc)
    } else {
        Ratios { fold: vec![0.0; v33], sign: vec![0.0; v33], v: bars }
    };
    let g = |x: &[f64]| {
        let n = x.len();
        ratios(x.to_vec(), n.saturating_sub(1), n.saturating_sub(2))
    };
    Some([g1, g(lowlevel), g(midpeak), g(s3832)])
}

const STEPS: [i32; 6] = [32, 64, 16, 96, 48, 80];

/// `VerseHookJudge_LowPeak` → (count, positions): candidate boundaries every 16..96 beats from
/// the intro end where the low band or the low-level flags mark a change.
pub fn verse_hook_judge_lowpeak(p0: i32, flags: &[f64], bar: i32, lowlevel_flags: &[u8]) -> Option<(i32, Vec<i32>)> {
    if lowlevel_flags.is_empty() {
        return None;
    }
    let n = lowlevel_flags.len();
    let v16: Vec<u8> = lowlevel_flags.iter().flat_map(|x| [*x; 4]).collect();
    let start = (bar - 1).max(0) as usize;
    if bar - 1 > flags.len() as i32 {
        return None;
    }
    let mut v26: Vec<f64> = flags[start.min(flags.len())..].to_vec();
    v26.resize(4 * n, 0.0);
    let v28 = v26.len() as i32;
    let cap = ((v28 as f64).mul_add(0.0625, 0.5) as i32).max(0) as usize;
    let mut v27 = vec![0i32; cap.max(1)];
    v27[0] = p0;
    let v29 = v28 - 4;
    let mut v30 = 2i32;
    let mut v31 = 1i32;
    let mut v33 = 0i32;
    let mut cur = p0;
    let fl = |i: i32| if i < 0 { 0.0 } else { v26.get(i as usize).copied().unwrap_or(0.0) };
    let hb = |i: i32| if i < 0 { 0 } else { v16.get(i as usize).copied().unwrap_or(0) };
    'a: loop {
        // B: do { ... } while v32 < v29
        'b: loop {
            let v32;
            // C
            loop {
                // D: next in-range candidate, or out of the B loop (to L25)
                let cand;
                loop {
                    let c = if ((v31 - 1) as u32) < 6 { STEPS[(v31 - 1) as usize] + cur } else { 16 * v31 + cur };
                    if c < v28 {
                        cand = c;
                        break;
                    }
                    v33 = v30;
                    v31 += 1;
                    if c >= v29 {
                        break 'b;
                    }
                }
                let v34 = cand - bar;
                if fl(v34) == 1.0 || fl(v34 - 1) == 1.0 {
                    v32 = cand;
                    break;
                }
                v33 = v30;
                if hb(v34) == 1 {
                    v32 = cand;
                    break;
                }
                v31 += 1;
                if cand >= v29 {
                    break 'b;
                }
            }
            if let Some(d) = v27.get_mut((v30 - 1) as usize) {
                *d = v32;
            } else {
                v27.push(v32);
            }
            v33 = v30 + 1;
            cur = v32;
            v31 = 1;
            v30 = v33;
            if v32 >= v29 {
                break 'b;
            }
        }
        v30 = v33;
        if v31 >= 7 {
            break 'a;
        }
    }
    if v33 >= 2 {
        let mut k = v33 - 2;
        while k >= 0 {
            if v27.get(k as usize).copied() == Some(0) {
                v27.remove(k as usize);
                v33 -= 1;
            }
            k -= 1;
        }
    }
    Some((v33 - 1, v27))
}

fn x4(v: &[f64], n: usize) -> Vec<f64> {
    (0..4 * n).map(|i| v.get(i / 4).copied().unwrap_or(0.0)).collect()
}
fn z3(v: &mut [f64], i: i64, z: f64) -> f64 {
    if i < 0 {
        return 0.0;
    }
    match v.get_mut(i as usize) {
        Some(x) => {
            if *x == 0.0 {
                *x = z;
            }
            *x
        }
        None => 0.0,
    }
}
fn at(v: &[f64], i: i64) -> f64 {
    if i < 0 {
        0.0
    } else {
        v.get(i as usize).copied().unwrap_or(0.0)
    }
}

/// `BreakFind` → (break start, break end, deep-break flag): a candidate where all three bands
/// drop and come back within 4/8/12 beats.
pub fn break_find(cnt: i32, pos: &[i32], bar: i32, low: &[f64], mid: &[f64], all: &[f64]) -> (i32, i32, i32) {
    let v19 = low.len().min(mid.len()).min(all.len());
    let mut a = x4(low, v19);
    let mut b = x4(mid, v19);
    let mut c = x4(all, v19);
    let (mut p6, mut p7, mut p8) = (0, 0, 0);
    if cnt < 3 {
        return (p6, p7, p8);
    }
    let mut v35 = 0;
    for k in 1..(cnt - 1) as usize {
        let v39 = pos.get(k).copied().unwrap_or(0);
        let v40 = (v39 - bar) as i64;
        let a0 = z3(&mut a, v40 - 1, 0.003);
        let b0 = z3(&mut b, v40 - 1, 0.003);
        let c0 = z3(&mut c, v40 - 1, 0.003);
        let v49 = at(&a, v40) / a0;
        let v50 = at(&b, v40) / b0;
        let v51 = at(&c, v40) / c0;
        if v49 >= 0.34999999999999998 || v50 >= 0.65000000000000002 || v51 >= 0.80000000000000004 {
            continue;
        }
        if v50 < 0.29999999999999999 {
            v35 = 1;
            p8 = 1;
        }
        let mut hit = None;
        for off in [3i64, 7, 11] {
            let i = v40 + off;
            let a1 = z3(&mut a, i, 0.003);
            let b1 = z3(&mut b, i, 0.003);
            let c1 = z3(&mut c, i, 0.003);
            if at(&a, i + 1) / a1 > 2.7000000000000002 {
                let v56 = at(&b, i + 1) / b1;
                if v56 > 1.3 && at(&c, i + 1) / c1 > 1.2 {
                    hit = Some((off as i32 + 1, v56));
                    break;
                }
            }
        }
        if let Some((v61, v56)) = hit {
            if v35 == 1 && v56 < 2.5 {
                p8 = 0;
            }
            p6 = v39;
            p7 = v39 + v61;
            return (p6, p7, p8);
        }
    }
    (p6, p7, p8)
}

/// `BigChangeFind` → (p8, p9, p10, p11): the first and second bar where every band jumps.
#[allow(clippy::too_many_arguments)]
pub fn big_change_find(cnt: i32, pos: &[i32], bar: i32, low: &[f64], mid: &[f64], all: &[f64], lvl: &[f64], lowpeak: &[f64]) -> Option<(i32, i32, i32, i32)> {
    if pos.is_empty() {
        return None;
    }
    let mut v21 = lvl.to_vec();
    if bar - 1 > lowpeak.len() as i32 || bar < 1 {
        return None;
    }
    let mut v82 = lowpeak[(bar - 1) as usize..].to_vec();
    let v88 = v82.len();
    let v90 = v21.len();
    let v24 = low.len().min(mid.len()).min(all.len()).min((v88 as f64).mul_add(0.25, 0.5) as i32 as usize).min((v90 as f64).mul_add(0.25, 0.5) as i32 as usize);
    let mut a = x4(low, v24);
    let mut b = x4(mid, v24);
    let mut c = x4(all, v24);
    let (mut p8, mut p9, mut p10, mut p11) = (0, 0, 0, 0);
    let v41 = pos[0];
    let v42 = v41 - (bar - 1);
    let v43 = v24 as i32 - (v42 as f64).mul_add(0.25, 0.5) as i32;
    if v43 - 8 >= 1 {
        let mut v44 = 0i64;
        let mut v45 = 0;
        let mut v55: i64 = 0;
        let mut exited = false;
        for v51 in 1..(v43 - 7) as i64 {
            let v54 = 4 * v51;
            v55 = v54 + v42 as i64;
            let i2 = v55 - 2;
            let v61 = z3(&mut a, i2, 0.003);
            let v63 = z3(&mut b, i2, 0.003);
            let v65 = z3(&mut c, i2, 0.003);
            let v56 = z3(&mut v82, i2, 0.003);
            let v57 = v55 - 1;
            let v58 = at(&a, v57);
            let v62 = at(&b, v57);
            let v64 = at(&c, v57);
            let v66 = at(&v82, v57);
            let v67 = z3(&mut v21, v57, 0.017999999999999999);
            let v68 = z3(&mut v21, i2, 0.017999999999999999);
            let v69 = v58 / v61;
            let v70 = v62 / v63;
            let v71 = v64 / v65;
            let v72 = v66 / v56;
            let v73 = v67 / v68;
            let hitc = (v69 > 20.0 && v70 > 1.3999999999999999 && v71 > 1.3)
                || (v72 > 1.2 && v69 > 2.5 && v70 > 2.5 && v71 > 1.7 && v73 > 3.0)
                || (v72 > 6.5 && v69 > 8.5 && v70 < 0.94999999999999996 && v73 > 1.0)
                || (v70 > 2.7000000000000002 && v71 > 2.5 && v73 > 1.5)
                || (v72 < 0.10000000000000001 && v69 < 0.10000000000000001 && v70 > 1.0 && v71 < 0.80000000000000004 && v73 < 0.80000000000000004)
                || (v72 > 4.0 && v69 > 2.5 && v70 > 1.3 && v71 < 1.0 && v73 < 1.0);
            if !hitc {
                continue;
            }
            let q = v54 as f64 * 0.0625;
            let to64 = if cnt < 2 {
                true
            } else if q != (q + 0.5).trunc() || v45 != 0 {
                if v45 != 0 {
                    exited = true;
                    break;
                }
                let v74 = (v54 + v41 as i64) as i32;
                !pos.iter().take(cnt.max(1) as usize).skip(1).any(|x| *x == v74)
            } else {
                false
            };
            if to64 {
                if v45 != 0 {
                    exited = true;
                    break;
                }
                v45 = 1;
                v44 = v55;
            }
        }
        if !exited {
            v55 = 0;
        }
        let p94 = (bar - 1) as i64;
        if v44 != 0 {
            p8 = ((v44 + p94 - v41 as i64) / 16 * 16 + v41 as i64) as i32;
            p9 = (v44 + p94) as i32;
        }
        if v55 != 0 {
            p10 = ((v55 + p94 - v41 as i64) / 16 * 16 + v41 as i64) as i32;
            p11 = (v55 + p94) as i32;
        }
    }
    Some((p8, p9, p10, p11))
}
