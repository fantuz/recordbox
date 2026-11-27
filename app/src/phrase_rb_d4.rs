//! Stage D of the phrase analyser port, part 4: block averages, the "up" (build-up) detectors
//! and the Mid-mood type/genre decision (`BlockAverage`, `UpSimpleloopAna`, `Up4_8_16Ana`,
//! `SongStructureAna_Type_Genre_Judge`).

use crate::phrase_rb_d1::{findpeaks, sum_range};
use crate::phrase_rb_dsp::sum8;

fn gi(v: &[i32], i: i64) -> i32 {
    if i < 0 {
        return 0;
    }
    v.get(i as usize).copied().unwrap_or(0)
}
fn gf(v: &[f64], i: i64) -> f64 {
    if i < 0 {
        return 0.0;
    }
    v.get(i as usize).copied().unwrap_or(0.0)
}
fn mean_link(x: &[f64], start: i64, len: i64) -> f64 {
    if len < 1 {
        return 0.0;
    }
    let w: Vec<f64> = (0..len).map(|k| gf(x, start + k)).collect();
    sum8(&w) / len as f64
}

/// `BlockAverage` → (p6: mean of `lvl` per block, p7: mean of `lowpeak` per block,
/// p8: per-bar mean of `bar_src`). `None` when the reference fails.
pub fn block_average(pos: &[i32], lvl: &[f64], lowpeak: &[f64], bar: i32, bar_src: &[f64]) -> Option<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    if lvl.is_empty() || lowpeak.is_empty() {
        return None;
    }
    let v18 = lowpeak.len().min(lvl.len()) as i32;
    let n = pos.len();
    let mut v21 = 0usize;
    for k in (0..n).rev() {
        if pos[k] > v18 {
            v21 += 1;
        } else {
            break;
        }
    }
    let v23 = n - v21;
    let mut v24: Vec<i32> = pos[..v23].to_vec();
    v24.push(v18 + 1);
    let mut p6 = vec![0f64; n];
    let mut p7 = vec![0f64; n];
    let v28 = bar - 1;
    let mut v27 = vec![0i32; v23 + 1];
    v27[0] = (v24[0] - v28).max(1);
    for i in 1..=v23 {
        v27[i] = v24[i] - v28;
    }
    if v23 >= 1 {
        for i in 0..v23 {
            let v29 = v27[i] as i64;
            let v50 = v27[i + 1] as i64 - v29;
            let v51 = ((v50 as f64).mul_add(0.044999999999999998, 0.5) as i32).min(4) as i64;
            let la = v50 - 4 * v51;
            let lb = v24[i + 1] as i64 - (4 * v51 + v24[i] as i64);
            if la < 0 || lb < 0 {
                return None;
            }
            p7[i] = mean_link(lowpeak, v24[i] as i64 - 1, lb);
            p6[i] = mean_link(lvl, v29 - 1, la);
        }
    }
    let nb = bar_src.len() / 4;
    if nb == 0 {
        return None;
    }
    let p8: Vec<f64> = (0..nb).map(|k| sum_range(bar_src, 4 * k as i32, 4 * k as i32 + 3) * 0.25).collect();
    Some((p6, p7, p8))
}

/// `UpSimpleloopAna`: kinds 30/50 whose high band rises steadily become 31 (in `kinds`).
/// → (p6: start of the rise per entry, p7: peak level per entry).
pub fn up_simpleloop(pos: &[i32], kinds: &mut [i32], all: &[f64], high: &[f64], lowblk: &[f64]) -> Option<(Vec<i32>, Vec<f64>)> {
    if kinds.is_empty() || lowblk.is_empty() {
        return None;
    }
    let n = pos.len();
    let mut v18: Vec<i32> = pos.to_vec();
    v18.push(all.len() as i32 + 1);
    let mut p6 = vec![0i32; n];
    let mut v80 = vec![0f64; n];
    let mut v81 = vec![0f64; n];
    let mut v26 = vec![0f64; n];
    let mut p7 = vec![0f64; n];
    let s8 = |a: i32| sum_range(high, a - 8, a - 1) * 0.125;
    for i in 0..n {
        let k = gi(kinds, i as i64);
        if k != 30 && k != 50 {
            continue;
        }
        let v30 = v18[i];
        let v31 = v18[i + 1];
        if v31 <= v30 + 15 {
            v80[i] = 0.0;
            v81[i] = 0.0;
            v26[i] = 0.0;
        } else {
            let mut v32 = v31 - 8;
            let mut v36 = 0.0;
            let v47;
            let v42;
            if v31 <= v30 + 31 {
                let mut m = 0.0;
                for v53 in v30 + 7..v31 {
                    let v54 = s8(v53);
                    v36 = v54.max(m);
                    if v54 > m {
                        v32 = v53 - 7;
                    }
                    m = v36;
                }
                let mut lo = v36;
                let mut l42 = v30;
                let mut l47 = v36;
                for v51 in v30 + 7..v31 {
                    let v56 = s8(v51);
                    l47 = v56.min(lo);
                    if v56 < lo {
                        l42 = v51 - 7;
                    }
                    lo = l47;
                }
                v47 = l47;
                v42 = l42;
            } else {
                let v33 = v31 - v30;
                let v35 = v33 / 2;
                let v37 = v35 + v30;
                if v37 <= v32 {
                    let mut m = 0.0;
                    for v38 in v30 + v35 + 7..v31 {
                        let v40 = s8(v38);
                        v36 = v40.max(m);
                        if v40 > m {
                            v32 = v38 - 7;
                        }
                        m = v36;
                    }
                }
                let mut l42 = v30;
                let mut l47 = v36;
                let mut lo = v36;
                let mut v41 = v30 - 1;
                loop {
                    let v44 = sum_range(high, v41, v41 + 7);
                    let v45 = v41 + 2;
                    v41 += 1;
                    let v46 = v44 * 0.125;
                    l47 = v46.min(lo);
                    if v46 < lo {
                        l42 = v41;
                    }
                    lo = l47;
                    if v45 >= v37 {
                        break;
                    }
                }
                v47 = l47;
                v42 = l42;
            }
            p7[i] = v36;
            let v57 = sum_range(all, v32 - 1, v32 + 6);
            let v58 = sum_range(all, v42 - 1, v42 + 6);
            v80[i] = (v57 - v58) * 0.125;
            v81[i] = v36 - v47;
            if v32 != v42 {
                v26[i] = (v36 - v47) / (v32 - v42) as f64;
            }
        }
        if gf(lowblk, i as i64) < 0.16 {
            let v62 = v80[i];
            let v63 = v81[i];
            let v64 = v62 >= 10.0 || v63 <= 85.0;
            let c1 = !v64 && v26[i] < 22.0 && v26[i] > 3.5;
            let c2 = v63 - v62 > 150.0 && v63 > 85.0 && v26[i] < 22.0;
            let c3 = v63 > 85.0 && v26[i] < 22.0 && v26[i] > 6.0;
            if c1 || c2 || c3 {
                kinds[i] = 31;
                p6[i] = v30;
            }
        }
    }
    Some((p6, p7))
}

const PAT4: [u8; 16] = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0];
const PAT8: [u8; 16] = [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0];

/// `Up4_8_16Ana`: snare rolls (4 → 8 → 16 hits per bar) before a section; marks them 32/33.
/// → (new kinds, p4, p5, p6, p7).
#[allow(clippy::type_complexity)]
pub fn up4_8_16(pos: &[i32], kinds: &[i32], hits: &[u8]) -> Option<(Vec<i32>, Vec<i32>, Vec<i32>, Vec<i32>, Vec<i32>)> {
    if pos.is_empty() || kinds.is_empty() || hits.is_empty() {
        return None;
    }
    let n = pos.len();
    let hb = |i: i64| -> u8 {
        if i < 0 {
            0
        } else {
            hits.get(i as usize).copied().unwrap_or(0)
        }
    };
    let eqp = |start: i64, pat: &[u8]| pat.iter().enumerate().all(|(k, p)| hb(start + k as i64) == *p);
    let mut v20: Vec<i32> = kinds.to_vec();
    let mut p4 = vec![0i32; n];
    let mut p5 = vec![0i32; n];
    let mut p6 = vec![0i32; n];
    let mut p7 = vec![0i32; n];
    let data = |i: usize| gi(pos, i as i64);
    let v18 = |i: usize| gi(kinds, i as i64);
    let sel = |k: i32| {
        let v29 = k % 100;
        (0..=51).contains(&v29) && (1u64 << v29) & 0xC0000C0000C00 != 0
    };
    if n >= 2 {
        let mut v134 = 0i32;
        let mut v28 = 0i32;
        let mut v26 = 0usize;
        'outer: loop {
            while !sel(v18(v26)) {
                v26 += 1;
                if v26 == n - 1 {
                    break 'outer;
                }
            }
            let v136 = v26;
            let v32 = data(v26);
            let v139 = v26 + 1;
            let v33 = data(v26 + 1);
            let v140 = (v33 - v32) / 4;
            let mut v145 = vec![0i32; (v140 + 8).max(1) as usize];
            let v135 = v33 - v32 - 1;
            let mut v44 = false;
            if v135 >= 3 {
                let v148 = v140.max(1);
                let mut v36 = 4 * v32 as i64 - 4;
                for v34 in 0..v148 as usize {
                    if eqp(v36, &PAT4) {
                        v145[v34] = 4;
                    }
                    if eqp(v36, &PAT8) {
                        v145[v34] = 8;
                    } else if v34 >= 2 && v145[v34 - 1] == 8 {
                        let ok = if v145[v34 - 2] == 8 { eqp(v36 + 2, &[1, 1]) } else { eqp(v36, &[1, 1, 1, 1]) };
                        if ok {
                            v145[v34] = 16;
                        }
                    }
                    v36 += 16;
                }
                let g = |k: i32| if k < 0 { 0 } else { v145.get(k as usize).copied().unwrap_or(0) };
                let mut v48 = false;
                let mut v27: Option<i32> = None;
                for j in 0..v148 {
                    if g(j) != 8 {
                        continue;
                    }
                    if v27.is_some() {
                        break;
                    }
                    v27 = Some(j + 1);
                    let v51 = j + 5;
                    if j + 1 >= v140 {
                    } else if g(j + 1) == 8 {
                        let mut v52 = j + 3;
                        let v53 = (j + 3 < v140) | v48;
                        if j + 3 >= v140 {
                            v52 = v134;
                        }
                        v48 |= v51 < v140;
                        let v54 = if v51 >= v140 { v134 } else { v51 };
                        if g(j + 2) == 8 {
                            v52 = v54;
                        } else {
                            v48 = v53;
                        }
                        v134 = v52;
                    } else {
                        v44 |= j + 1 > 1;
                        if j + 1 > 1 {
                            v28 = j;
                        }
                        v134 = j + 2;
                        v48 = true;
                    }
                }
                if let (true, Some(v27)) = (v48, v27) {
                    if v134 <= v140 && g(v134 - 1) == 16 {
                        let v55 = if v134 - v27 <= 2 { -16 } else { -32 };
                        let v56 = if v134 - v27 == 1 { -8 } else { v55 };
                        let v57 = data(v136);
                        let v58 = v32 + 4 * v27;
                        v20[v136] = 32;
                        let v59 = v58 - 4;
                        let v60 = v58 - 8;
                        let mut v61 = v56 + v59;
                        if v61 < v57 {
                            v61 = v60;
                        }
                        p4[v136] = v61;
                        p5[v136] = v61;
                        p6[v136] = v59;
                        p7[v136] = v32 + 4 * (v134 - 1);
                    }
                }
            }
            let v45 = v136;
            if v20[v45] != 32 && v44 && v28 > 0 && v135 >= 35 && v145.get((v28 - 1) as usize).copied() == Some(4) && v28 == v140 - 2 {
                v20[v45] = 32;
                let v47 = v32 + 4 * (v28 - 1);
                p4[v45] = v47;
                p5[v45] = v47;
                p6[v45] = p4[v45] + 4;
                p7[v45] = p4[v45] + 8;
            }
            v26 = v139;
            if v26 == n - 1 {
                break;
            }
        }
    }
    // entries before a 40
    let idx: Vec<usize> = (0..kinds.len()).filter(|i| kinds[*i] == 40).collect();
    for &i in &idx {
        if i == 0 {
            continue;
        }
        let v75 = v18(i - 1) % 100;
        let v76 = v75 & !1;
        let v77 = if v76 == 10 { 0.84999999999999998 } else { 0.90000000000000002 };
        let v78 = data(i - 1);
        let v79 = data(i) - v78;
        let v80 = if v79 >= 0 { v79 } else { v79 + 3 };
        if v20[i - 1] == 32 {
            continue;
        }
        let bit = (0..=51).contains(&v75) && (1u64 << v75) & 0xC0000C0000000 != 0;
        if !bit && (v79 < 36 || v76 != 10) {
            continue;
        }
        let v83 = (v80 >> 2) as usize;
        let mut v84 = vec![0i32; v83];
        if v79 >= 4 {
            for (b, d) in v84.iter_mut().enumerate() {
                let s = 4 * v78 as i64 - 4 + 16 * b as i64;
                *d = (0..16).map(|k| hb(s + k) as i32).sum();
            }
        }
        let f = |k: usize| v84[v83 - k] as f64;
        let di = data(i);
        // (kind, p4, p5, p6, p7)
        let mut set: Option<(i32, i32, i32, i32, i32)> = None;
        if v79 & !31 == 32 {
            let v105 = (((f(8) + f(7)) + f(6)) + f(5)) * 0.25;
            let v106 = (f(4) + f(3)) * 0.5;
            let v107 = f(2);
            if v105 <= v106 * v77 && v106 <= v77 * v107 {
                set = Some((32, di - 32, di - 32, di - 16, di - 8));
            } else if v105 < (v106 + v107) * 0.5 {
                set = Some((33, di - 32, 0, 0, 0));
            }
        } else if v79 < 64 {
            if v79 < 16 {
                continue;
            }
            let v119 = (f(4) + f(3)) * 0.5;
            let v120 = f(2);
            if v119 <= v77 * v120 {
                set = Some((32, di - 16, di - 16, di - 16, di - 8));
            } else if v119 < v120 {
                set = Some((33, di - 16, 0, 0, 0));
            }
        } else {
            let v111 = (((((((f(16) + f(15)) + f(14)) + f(13)) + f(12)) + f(11)) + f(10)) + f(9)) * 0.125;
            let v112 = (((f(6) + f(5)) + f(8)) + f(7)) * 0.25;
            let v113 = ((f(4) + f(3)) + f(2)) * 0.33333000000000002;
            let v114 = v112 * v77;
            let v115 = v113 * v77;
            if v111 <= v114 && v112 <= v115 {
                set = Some((32, di - 48, di - 48, di - 32, di - 16));
            } else {
                let l = &v84[..v83 - 1];
                let mx = l.iter().copied().max().unwrap_or(0);
                let mn = l.iter().copied().min().unwrap_or(0);
                let go = if mx - mn > 4 { v111 < (v113 + v112) * 0.5 } else { v112 <= v115 };
                if go {
                    set = Some((33, di - 32, 0, 0, 0));
                }
            }
        }
        if let Some((k, a, b, c, d)) = set {
            v20[i - 1] = k;
            p4[i - 1] = a;
            p5[i - 1] = b;
            p6[i - 1] = c;
            p7[i - 1] = d;
        }
    }
    if n > 1 {
        for k in 0..n - 1 {
            if (v18(k) % 100) & !1 == 10 && (v20[k] & !1) == 32 {
                v20[k] = v18(k);
            }
        }
    }
    Some((v20, p4, p5, p6, p7))
}

fn smooth(v: &mut [f64], cnt: usize) {
    let mut prev = v[0];
    for k in 1..=cnt {
        if k >= v.len() {
            break;
        }
        let mut cur = v[k];
        let d = (prev - cur).abs();
        if d < prev * 0.10000000000000001 || d < 0.050000000000000003 {
            v[k] = prev;
            cur = prev;
        }
        prev = cur;
    }
}

/// `SongStructureAna_Type_Genre_Judge` → (mood, structure type 1/2).
#[allow(clippy::too_many_arguments)]
pub fn type_genre_judge(lowpeak: &[f64], bar: i32, dur: f64, kinds_up: &[i32], kinds_roll: &[i32], mood_in: i32) -> Option<(i32, i32)> {
    let n = lowpeak.len() as i32;
    let (v19, v20, v21, v22) = if dur <= 600.0 { (1.5, 1, 3.0, 2) } else { (dur * 0.0041700000000000001, (dur * 0.0033300000000000001) as i32, dur * 0.0083300000000000006, (dur * 0.0066699999999999997) as i32) };
    let v272 = (n / 4).max(0) as usize;
    let v270 = v272 as i32 - 1;
    let cnt = v270.max(1) as usize;
    let mut v23 = vec![0f64; v272.max(cnt + 1)];
    if n >= 8 {
        let mut s = bar - 1;
        for k in 0..cnt {
            v23[k] = sum_range(lowpeak, s, s + 3) * 0.25;
            s += 4;
        }
        smooth(&mut v23, cnt);
    }
    v23.truncate(v272);
    let mut v266 = vec![0f64; v272];
    if n >= 4 {
        for k in 0..v272 {
            v266[k] = 1.0 - v23[k];
        }
    }
    let mut v265 = v266.clone();
    let (mut v286, mut v275, mut v285, mut v284) = findpeaks(&v266);
    let data = &v266;
    let nn = data.len() as i64;
    let d = |i: i64| gf(data, i);
    let mut v54: i64 = 0;
    let mut v55: i64 = 0;
    let fill = |v265: &mut Vec<f64>, from: i64, cnt: i64, src: i64| {
        let val = gf(v265, src);
        for k in 0..cnt.max(0) {
            if let Some(x) = v265.get_mut((from + k) as usize) {
                *x = val;
            }
        }
    };
    if v286.len() >= 3 {
        let mut v56 = v286.len() as i64 - 2;
        loop {
            if v285[v56 as usize] >= 4.0 {
                v286.remove(v56 as usize);
                v275.remove(v56 as usize);
                v285.remove(v56 as usize);
                v284.remove(v56 as usize);
            } else {
                let v58 = v275[v56 as usize] as i64;
                let mut c = v58 - 1;
                while c >= 1 {
                    if d(c) <= d(c - 1) {
                        v54 = c;
                        break;
                    }
                    c -= 1;
                }
                let v62 = nn - 1;
                let mut v63 = v58;
                let mut j = v58 - 1;
                while j < v62 {
                    if d(j) > d(j + 1) {
                        v63 = j;
                        break;
                    }
                    j += 1;
                }
                let v65 = if v63 <= v62 { v62 } else { v63 };
                let mut v67 = v55;
                let mut j = v63;
                while j < v65 {
                    if d(j) <= d(j + 1) {
                        v67 = j;
                        break;
                    }
                    j += 1;
                }
                let v74 = v54 >= v67 || v54 + 1 == v67;
                if !v74 {
                    fill(&mut v265, v54 + 1, v67 - v54 - 1, v54);
                }
                v55 = v67;
            }
            if v56 <= 1 {
                break;
            }
            v56 -= 1;
        }
    }
    let v53 = v286.len();
    if v53 >= 1 {
        let v81 = v275[0] as i64;
        if v81 >= 1 {
            let mut v84 = 0i64;
            let mut c = v81 - 1;
            while c >= 1 {
                if d(c) <= d(c - 1) {
                    v84 = c;
                    break;
                }
                c -= 1;
            }
            let v88 = nn - 1;
            let mut v89 = v81 - 1;
            let mut v92 = v81 - 1;
            let mut j = v81 - 1;
            while j < v88 {
                if d(j) > d(j + 1) {
                    v89 = j;
                    v92 = j;
                    break;
                }
                j += 1;
            }
            if v92 <= v88 {
                v92 = v88;
            }
            let mut v96 = v55;
            let mut j = v89;
            while j < v92 {
                if d(j) <= d(j + 1) {
                    v96 = j;
                    break;
                }
                j += 1;
            }
            if d(v84) <= d(v96) {
                if v84 + 1 < v96 {
                    fill(&mut v265, v84 + 1, v96 - v84 - 1, v96);
                }
            } else if v84 < v96 {
                fill(&mut v265, v84, v96 - v84, v84);
            }
            if v53 >= 2 {
                let v108 = v275[v275.len() - 1] as i64;
                let mut c = v108 - 1;
                while c >= 1 {
                    if d(c) <= d(c - 1) {
                        v84 = c;
                        break;
                    }
                    c -= 1;
                }
                let mut v115 = v108 - 1;
                let mut v117 = v108 - 1;
                let mut j = v108 - 1;
                while j < v88 {
                    if d(j) > d(j + 1) {
                        v115 = j;
                        v117 = j;
                        break;
                    }
                    j += 1;
                }
                if v117 <= v88 {
                    v117 = v88;
                }
                let mut v90 = v88;
                let mut v122 = v88;
                let mut j = v115;
                while j < v117 {
                    if d(j) <= d(j + 1) {
                        v90 = j;
                        v122 = j;
                        break;
                    }
                    j += 1;
                }
                if d(v84) <= d(v90) {
                    if v84 + 1 < v122 {
                        fill(&mut v265, v84 + 1, v122 - v84 - 1, v90);
                    }
                } else if v84 < v122 {
                    fill(&mut v265, v84, v90 - v84, v84);
                }
            }
        }
    }
    if n >= 4 {
        for x in v265.iter_mut() {
            *x = 1.0 - *x;
        }
    }
    if n >= 8 {
        let c = cnt.min(v265.len().saturating_sub(1));
        smooth(&mut v265, c);
    }
    let mut v264 = vec![0f64; v272];
    if n >= 8 {
        for k in 0..cnt {
            v264[k] = gf(&v265, k as i64 + 1) - gf(&v265, k as i64);
        }
    }
    if v264.len() >= 35 {
        let keep = v272 - 17;
        v264.truncate(keep);
        v264.drain(..16);
    }
    v264.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let v161 = v264.len();
    let v163 = v161 / 2;
    let v164 = (v161 & !1) as i64 - 1;
    let mut v290 = vec![0f64; v161];
    let mut v288 = vec![0f64; v161];
    let v166 = v163 as i64 - 1;
    if (v163 as i64) < v164 {
        for i in 0..v166 as usize {
            v290[v163 + i] = v264[i];
            v288[v163 + i] = -v264[v161 - 1 - i];
        }
    }
    if v161 >= 2 {
        for i in 0..v163 {
            v290[v166 as usize - i] = v264[i];
            v288[v166 as usize - i] = -v264[v161 - 1 - i];
        }
    }
    let (v283, _, v282, _) = findpeaks(&v290);
    let (v280, _, v279, _) = findpeaks(&v288);
    let v219 = kinds_up.iter().filter(|k| **k == 31).count() as i32;
    let mut v252 = 0;
    for k in 1..kinds_roll.len() {
        if kinds_roll[k] == 32 || (kinds_roll[k] == 33 && kinds_roll[k - 1] != 10) {
            v252 += 1;
        }
    }
    let v250 = gf(&v279, 0).mul_add(gf(&v280, 0), gf(&v282, 0) * gf(&v283, 0));
    let mut mood = mood_in;
    let ty;
    if v250 < v21 && v219 < v22 && v252 < v20 {
        ty = if v250 >= v19 { 1 } else { 2 };
    } else {
        ty = 1;
        mood = 1;
    }
    Some((mood, ty))
}
