//! Stage D of the phrase analyser port, part 2: track-level measures and the mood decision
//! (`DetectComp`, `DetectFadeOut`, `CalcMaxLevelRatio`, `DetectBPM128`, `DetectTenkeiGenre1`,
//! `DetectSlowrock`, `CalcTrackLowPowerRatio`, `DetectSlowTempo`,
//! `Detect4KicksPopRockRhythm`, `GenreDevideOverall`, `Genre12FinalJudge`).
//! Sums whose association the compiler changed are written out in the emitted order.

use crate::phrase_rb_d1::sum_range;
use crate::phrase_rb_dsp::{max_idx, max_val, sum8};

/// Sixteen values added in the order the compiler emitted for the per-bar sums.
#[inline]
pub fn sum16a(x: &[f64]) -> f64 {
    ((((x[1] + x[0]) + (x[2] + x[3])) + ((x[4] + x[5]) + x[6])) + (((x[7] + x[8]) + x[9]) + x[10])) + ((((x[11] + x[12]) + x[13]) + x[14]) + x[15])
}

/// `DetectComp` → (compression ratio, scaled low level).
pub fn detect_comp(p0: &[f64], p1: &[f64], rhythm: &[u8], bar: i32) -> (f64, f64) {
    let v89 = 4 * bar - 4;
    let v17 = rhythm.len() as i32 - v89;
    let v18 = if v17 >= 0 { v17 } else { v17 + 127 };
    if v17 <= 127 {
        return (0.1, 0.3);
    }
    let n = p1.len() as i32;
    let v90 = (v18 >> 7) as usize;
    let mut v76 = vec![0f64; v90];
    let mut v74 = vec![0f64; v90];
    let v27 = n - v89;
    for w in 0..v90 {
        let mut v39 = 0i32;
        let v25 = 128 * w as i32;
        let (mut v42, mut v41) = (0.0f64, 0.0f64);
        let mut v38 = 0i32;
        for i in 0..128i32 {
            if rhythm.get((v25 + i) as usize).copied().unwrap_or(0) != 1 {
                continue;
            }
            let pos1 = v89 + 1 + v25 + i;
            if v25 + i + 1 != v39 && pos1 <= n {
                let k = (pos1 - 1) as usize;
                v42 = p1[k] + v42;
                v41 = p0.get(k).copied().unwrap_or(0.0) + v41;
            }
            v38 += 1;
            if pos1 > v27 {
                v39 = 0;
            } else {
                if pos1 < n {
                    v42 = p1[pos1 as usize] + v42;
                    v41 = p0.get(pos1 as usize).copied().unwrap_or(0.0) + v41;
                }
                v39 = v25 + i + 2;
            }
        }
        let v70 = sum_range(p1, v89 + 128 * w as i32, v89 + 128 * w as i32 + 127);
        if v38 > 0 {
            let mut v71 = 0.0;
            if v38 <= 127 {
                if v38 < 4 {
                    continue;
                }
                v71 = (v70 - v42) / (128 - v38) as f64;
            }
            let v34 = v42 / v38 as f64;
            let v35 = if v71 + v34 <= 0.0 { 0.0 } else { v71 / (v71 + v34) };
            v76[w] = v35;
            v74[w] = v41 / v38 as f64;
        }
    }
    (max_val(&v76), max_val(&v74) * 0.0075175301000000002)
}

/// `DetectFadeOut` on the last 132 beats of the quarter-beat low and full powers.
pub fn detect_fade_out(p0: &[f64], p1: &[f64]) -> i32 {
    let n = p0.len() as i32;
    if n <= 1551 {
        return 0;
    }
    let mut a = [0f64; 32];
    let mut b = [0f64; 32];
    for k in 0..32 {
        let base = ((n & !15) - 528 + 16 * k as i32) as usize;
        let s = |x: &[f64]| -> f64 {
            let d = |j: usize| x.get(base + j).copied().unwrap_or(0.0);
            let mut acc = 0.0 + d(0);
            acc += d(1) + d(2);
            acc += (d(3) + d(4)) + d(5);
            acc += ((d(6) + d(7)) + d(8)) + d(9);
            acc += (((d(10) + d(11)) + d(12)) + d(13)) + d(14);
            acc + d(15)
        };
        a[k] = s(p0);
        b[k] = s(p1);
    }
    let sb: Vec<f64> = (0..16).map(|m| if m == 0 { b[0] + b[1] } else { (0.0 + b[2 * m]) + b[2 * m + 1] }).collect();
    let sa: Vec<f64> = (0..16).map(|m| if m == 0 { a[0] + a[1] } else { (0.0 + a[2 * m]) + a[2 * m + 1] }).collect();
    let any_small = sb.iter().any(|v| *v < 0.0000001);
    let r: Vec<f64> = (0..15).map(|m| if any_small { 0.0 } else { sa[m] / sb[m] }).collect();
    let mut mxb = sb[0];
    for v in &sb[1..] {
        mxb = mxb.max(*v);
    }
    if sb[15] >= mxb * 0.001 || !(sb[14] > sb[15]) {
        return 0;
    }
    let mut lo = r[0];
    let mut hi = r[0];
    for v in &r[1..12] {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    let inr = |x: f64| lo <= x && x <= hi;
    (inr(r[14]) && sb[13] > sb[14] && inr(r[13]) && sb[12] > sb[13] && inr(r[12])) as i32
}

/// `CalcMaxLevelRatio`: share of 4-beat blocks whose peak reaches 0.89125.
pub fn calc_max_level_ratio(x: &[f64]) -> f64 {
    let n = x.len() as i32;
    if n < 16 {
        return 0.0;
    }
    let blocks = (n >> 4) as usize;
    let hits = (0..blocks).filter(|k| max_val(&x[16 * k..16 * k + 16]) >= 0.89124999999999999).count();
    hits as f64 / blocks as f64
}

/// `DetectBPM128` → (tempo near 128 or 133, tempo in the wider house range).
pub fn detect_bpm128(bpm: f64) -> (i32, i32) {
    if (127.04000000000001..=128.96000000000001).contains(&bpm) || (132.33002500000001..=134.32997500000002).contains(&bpm) {
        return (1, 0);
    }
    if (122.88..=133.12).contains(&bpm) {
        return (0, 1);
    }
    if bpm >= 127.99680000000001 && bpm <= 138.66320000000002 {
        return (0, 1);
    }
    (0, 0)
}

pub fn detect_tenkei_genre1(bpm: f64, f520: i32) -> i32 {
    (bpm >= 96.099999999999994 && bpm <= 105.01000000000001 && f520 == 1) as i32
}

/// `DetectSlowrock`: kick on 1 and 3 (both detectors), snare-only between, twice in a row.
pub fn detect_slowrock(a: &[u8], b: &[u8]) -> i32 {
    let n = a.len() as i64;
    if n < 33 {
        return 0;
    }
    let ga = |i: i64| a.get(i as usize).copied().unwrap_or(0);
    let gb = |i: i64| b.get(i as usize).copied().unwrap_or(0);
    let both0 = |i: i64| ga(i) == 0 && gb(i) == 0;
    let (mut v8, mut v9) = (0i64, 0i32);
    for t in 0..=(n - 33) {
        if !(ga(t) == 1 && gb(t) == 1) {
            continue;
        }
        let mut ok = true;
        for j in 2..=6 {
            if !(t + j + 1 <= n && both0(t + j)) {
                ok = false;
                break;
            }
        }
        if !ok || !(t + 8 <= n) {
            continue;
        }
        let mut v13 = (ga(t + 7) == 0 && gb(t + 7) == 1) as i32;
        if t + 9 <= n {
            if ga(t + 8) == 0 && gb(t + 8) == 1 {
                v13 += 1;
            }
            if t + 10 <= n && ga(t + 9) == 0 && gb(t + 9) == 1 {
                v13 += 1;
            }
        }
        if v13 == 0 {
            continue;
        }
        let mut ok = true;
        for j in 10..=14 {
            if !(t + j + 1 <= n && both0(t + j)) {
                ok = false;
                break;
            }
        }
        if !ok || !(t + 16 <= n) {
            continue;
        }
        let mut v14 = (ga(t + 15) == 1 && gb(t + 15) == 1) as i32;
        if t + 17 <= n {
            if ga(t + 16) == 1 && gb(t + 16) == 1 {
                v14 += 1;
            }
            if t + 18 <= n && ga(t + 17) == 1 && gb(t + 17) == 1 {
                v14 += 1;
            }
        }
        if v14 != 0 {
            let v16 = t == v8 || t + 1 == v8 || t + 2 == v8;
            if v16 {
                v9 += 1;
            }
            v8 = if v16 { 0 } else { t + 17 };
        }
    }
    (v9 > 1) as i32
}

/// `CalcTrackLowPowerRatio` → (√(Σlow/Σall), per-bar ratio, per-bar low power, peak ratio, peak power).
pub fn calc_track_low_power_ratio(p0: &[f64], p1: &[f64], nbeats: i32) -> (f64, Vec<f64>, Vec<f64>, f64, f64) {
    let v18 = sum8(p0);
    let v19 = sum8(p1);
    let p3 = if v19 > 0.0 { (v18 / v19).sqrt() } else { 1.0 };
    let bars = (nbeats / 4).max(0) as usize;
    if nbeats < 4 {
        return (p3, vec![0.0], vec![0.0], 0.0, 0.0);
    }
    let mut ratio = vec![0f64; bars];
    let mut pow = vec![0f64; bars];
    let z = [0f64; 16];
    for k in 0..bars {
        let a = p0.get(16 * k..16 * k + 16).unwrap_or(&z);
        let b = p1.get(16 * k..16 * k + 16).unwrap_or(&z);
        let v38 = sum16a(a);
        let v39 = sum16a(b);
        pow[k] = v38;
        ratio[k] = if v39 <= 0.0 { 0.0 } else { v38 / v39 };
    }
    let (p6, p7);
    if nbeats > 127 {
        let w = ((nbeats as u32 >> 6) - 1) as usize;
        let mut r = vec![0f64; w];
        let mut q = vec![0f64; w];
        for k in 0..w {
            r[k] = sum16a(ratio.get(16 * k..16 * k + 16).unwrap_or(&z)) * 0.0625;
            q[k] = sum16a(pow.get(16 * k..16 * k + 16).unwrap_or(&z)) * 0.0625;
        }
        p6 = max_val(&r);
        p7 = max_val(&q);
    } else {
        p6 = sum8(&ratio) / bars as f64;
        p7 = sum8(&pow) / bars as f64;
    }
    (p3, ratio, pow, p6, p7)
}

/// `DetectSlowTempo` → (slow, half-time feel).
#[allow(clippy::too_many_arguments)]
pub fn detect_slow_tempo(sr8: f64, bar_pow: &[f64], bpm: f64, snare: &[i32], ms_in: i32, ms_b: i32, beats: &[i32], nbeats: i32, f592: i32, f596: i32) -> (i32, i32) {
    let mut v24: Vec<f64> = snare.iter().map(|x| *x as f64).collect();
    let (v30, v31) = (f592, f596);
    let mut p12 = 0;
    let v32 = v31 - 1;
    let v33 = v31 + v30;
    if v30 >= 1 {
        let v34 = if v31 > v33 - 1 { v31 } else { v33 - 1 };
        let v35 = sr8 * 0.001 * (ms_in - ms_b) as f64;
        for i in v32..v34 {
            if let Some(v) = v24.get_mut(i as usize) {
                *v += v35;
            }
        }
    }
    let v48 = if bpm == 0.0 { sr8 * 0.5 } else { sr8 * 60.0 / bpm };
    let v50 = sum8(bar_pow);
    let n = bar_pow.len() as i32;
    let at = |i: i32| v24.get(i as usize).copied().unwrap_or(0.0);
    if n != 0 && n >= 4 && v33 - 2 >= v31 {
        let mut arr = vec![];
        let v54 = v48 * 0.125;
        let v55 = v50 / n as f64;
        let v56 = n - 3;
        let v57 = (v48 * 0.25 + 0.5).trunc();
        let l = (v48 + 0.5) as i32;
        let mut v52 = 0;
        loop {
            if bar_pow[v52 as usize] > v55 {
                let v58 = if 4 * v52 + 32 >= nbeats { nbeats - 1 } else { 4 * v52 + 31 };
                let v59 = beats.get((4 * v52) as usize).copied().unwrap_or(0) as f64;
                let v60 = beats.get(v58 as usize).copied().unwrap_or(0) as f64;
                let mut v61 = v32;
                loop {
                    let v63 = at(v61);
                    if v63 >= v59 && v63 <= v60 && v61 + 2 < v33 {
                        let q = (v63 - v59) * (1.0 / l as f64);
                        let v65 = q - ((q as i32).wrapping_mul(l)) as f64;
                        let v66 = v65 >= v57 && v65 <= v57 * 3.0;
                        let v67 = at(v61 + 1) - v63;
                        if v67 > v54 && v67 > v48 * 1.875 && v67 < v48 * 2.125 && !v66 && arr.len() <= 63 {
                            arr.push(0);
                        }
                        if v67 > v54 && v67 > v48 * 0.875 && v67 < v48 * 1.125 && v66 && arr.len() <= 63 {
                            arr.push(1);
                        }
                    }
                    v61 += 1;
                    if !(v63 <= v60 && v61 < v33 - 2) {
                        break;
                    }
                }
                if arr.len() > 63 {
                    break;
                }
            }
            v52 += 1;
            if v52 == v56 {
                break;
            }
        }
        if !arr.is_empty() && arr.iter().sum::<i32>() as f64 / arr.len() as f64 > 0.5 {
            p12 = 1;
        }
    }
    let p11 = (p12 == 0 && bpm < 83.900000000000006) as i32;
    (p11, p12)
}

/// `Detect4KicksPopRockRhythm` → (four-on-the-floor, 0-6-8 kick, 0-8 kick, rock 0-8-10).
pub fn four_kicks_full(rhythm: &[u8], bpm: f64, bar_power: &[f64]) -> (i32, i32, i32, i32) {
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
    let v27 = (v20 > 63) as i32;
    if v20 < 1 || v21 <= 0 {
        return (0, 0, 0, 0);
    }
    let pat = |bar: i32, ones: &[usize]| -> bool {
        (0..16).all(|j| {
            let b = rhythm.get((bar * 16) as usize + j).copied().unwrap_or(0);
            if ones.contains(&j) {
                b == 1
            } else {
                b == 0
            }
        })
    };
    let (mut c80, mut c78, mut c76, mut c82) = (0, 0, 0, 0);
    for g in 0..v21 {
        let (mut v34, mut v35, mut v36, mut v37) = (0, 0, 0, 0);
        for k in 0..8i32 {
            if (k + 1) & 0x7FFFFFFA == 2 {
                let bar = (v17 - 1) + 8 * g + k;
                if bar >= 1 {
                    v35 += pat(bar, &[0, 8]) as i32;
                    v36 += pat(bar, &[0, 6, 8]) as i32;
                    v37 += (pat(bar, &[2, 6, 10, 14]) || pat(bar, &[0, 4, 8, 12])) as i32;
                    v34 += pat(bar, &[0, 8, 10]) as i32;
                }
            }
        }
        c80 += (v37 > 0) as i32;
        c78 += (v36 >= 1) as i32;
        c76 += (v35 >= 1) as i32;
        c82 += (v34 >= 1) as i32;
    }
    if c80 > 1 {
        return (1, 0, 0, 0);
    }
    if c82 > v27 {
        return (0, 0, 0, 1);
    }
    if c78 > v27 && bpm < 165.0 {
        return (0, 1, 0, 0);
    }
    if c76 > v27 {
        return (0, 0, 1, 0);
    }
    (0, 0, 0, 0)
}

/// `GenreDevideOverall`: 1 High, 2 Mid, 3 Low.
#[allow(clippy::too_many_arguments)]
pub fn genre_devide_overall(p0: i32, p1: i32, p2: f64, p3: i32, p4: i32, p5: i32, p6: i32, p7: i32, p8: i32, p11: i32, p12: i32, p13: i32, p14: i32, p17: f64) -> i32 {
    let v21 = p17 < 0.61280000000000001 && p1 == 1;
    if !(p2 >= 0.39789999999999998 && p0 != 1 && !v21) {
        return 3;
    }
    if p7 == 1 || p8 == 1 {
        return 2;
    }
    if p11 == 1 {
        return p11;
    }
    let inner = (p4 as f64) * -0.050000000000000003;
    let s = ((p12 as f64) - (p5 as f64)).mul_add(0.29999999999999999, inner);
    let s = ((p13 as f64) + (p14 as f64)).mul_add(0.10000000000000001, s);
    let s = (((p6 as f64) - (p3 as f64)) + p17).mul_add(0.59999999999999998, s);
    if s > 1.0 {
        1
    } else {
        2
    }
}

/// `Genre12FinalJudge`: a Mid track whose low band swings between bars becomes High.
pub fn genre12_final_judge(mood: &mut i32, nbeats: i32, lowpeak: &[f64], lowpeak4: &[f64]) {
    if *mood != 2 {
        return;
    }
    let (v12, v11_1) = if nbeats <= 95 { (0, 0) } else { (8, nbeats - 64) };
    let (v13, v14, v15) = if nbeats <= 95 { (-1, -2, -1) } else { (7, nbeats - 66, nbeats - 73) };
    let v16 = if v14 - v13 < 0 { 0.0 } else { sum_range(lowpeak, v13, v14) / (v15 + 1) as f64 };
    let v18 = 4 * v12;
    let v19 = !(4 * v12) + 4 * v11_1;
    let v20;
    if nbeats == 2 || v19 < 49 {
        let v21 = if v19 >= 0 { v19 } else { v19 + 3 };
        if v19 < 4 {
            return;
        }
        v20 = v21 >> 2;
    } else {
        v20 = ((v19 - 32) as u32 >> 2) as i32;
    }
    let n20 = v20.max(0) as usize;
    let mut v60 = vec![0i32; n20];
    let mut v58 = vec![0f64; n20];
    let mut v56 = vec![0f64; n20];
    if v20 > 1 {
        for k in 1..n20 {
            let s = v18 as usize + 4 * (k - 1);
            let w = lowpeak4.get(s..s + 4).unwrap_or(&[0.0; 4]);
            let (v31, i) = max_idx(w);
            let v52 = i as i32 + 1;
            let v33 = if v31 != 0.0 { v31 } else { 0.003 };
            let v34 = v60[k - 1];
            let v35 = (v52 - v34).abs();
            v60[k] = v52;
            v56[k] = v31;
            let mut v36 = Some(1.0);
            if v52 != v34 {
                v36 = if v35 & 0x7FFFFFFD != 1 { None } else { Some(lowpeak4.get((v18 - 1 + 4 * (k as i32 - 1) + v34) as usize).copied().unwrap_or(0.0) / v33) };
            }
            if let Some(v) = v36 {
                v58[k - 1] = v;
            }
        }
    }
    let v39 = n20 >= 1 && sum8(&v58) / n20 as f64 > 0.91000000000000003;
    let n54 = (v20 - 4).max(0) as usize;
    let mut v54 = vec![0f64; n54];
    if v20 >= 7 {
        for j in 2..n54 {
            if v56[j] == 0.0 {
                v56[j] = 0.003;
            }
            if v56[j - 1] == 0.0 {
                v56[j - 1] = 0.003;
            }
            let (c, p) = (v56[j], v56[j - 1]);
            v54[j] = if c < p { p / c } else { c / p };
        }
    }
    if n54 >= 3 && sum_range(&v54, 2, n54 as i32 - 1) / (n54 as i32 - 2) as f64 >= 1.5 && v16 >= 0.2 && v39 {
        *mood = 1;
    }
}

/// `FindSecurePoint`: bars (1-based, +1) where the low band rises clearly and stays.
pub fn find_secure_point(lowpeak: &[f64], bar: i32) -> Option<Vec<i32>> {
    let n = lowpeak.len() as i32;
    let v8 = (n / 4).max(0) as usize;
    let mut v61 = vec![0f64; v8];
    let v11 = v8 as i32 - 1;
    if n >= 8 {
        let mut s = bar - 1;
        for k in 0..v11.max(1) as usize {
            if let Some(d) = v61.get_mut(k) {
                *d = crate::phrase_rb_d1::sum_range(lowpeak, s, s + 3) * 0.25;
            }
            s += 4;
        }
    }
    let mut v60 = vec![0f64; v8];
    if n >= 8 {
        for k in 0..v11.max(1) as usize {
            if k + 1 < v8 {
                v60[k] = v61[k + 1] - v61[k];
            }
        }
    }
    let v22 = max_val(&v60);
    let mut v58 = vec![0i32; v8];
    if n >= 12 {
        let mut i = 0usize;
        let v27 = v8.max(3);
        for v25 in 0..v27 - 2 {
            let x = v60.get(v25).copied().unwrap_or(0.0);
            if x > v22 * 0.6 && x > 0.14999999999999999 {
                v58[i] = v25 as i32 + 1;
                i += 1;
            }
        }
        let g = |k: i32| v61.get(k as usize).copied().unwrap_or(0.0);
        for j in 0..i {
            let v32 = v58[j];
            let v33 = g(v32);
            let (hi, lo) = (v33 * 1.3, v33 * 0.69999999999999996);
            let a = g(v32 + 1);
            let b = g(v32 + 2);
            if a > hi || a < lo || b > hi || b < lo {
                v58[j] = 0;
            }
        }
    }
    if v58.is_empty() {
        return None;
    }
    v58.sort_by(|a, b| b.cmp(a));
    let mut k = v58.len();
    while k > 0 {
        k -= 1;
        let v = v58[k];
        if v != 0 {
            v58[k] = v + 1;
        } else if k != 0 {
            v58.remove(k);
        }
    }
    v58.sort();
    Some(v58)
}
