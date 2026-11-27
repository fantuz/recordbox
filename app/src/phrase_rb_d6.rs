//! Stage D of the phrase analyser port, part 6: irregular intros (`IrregularJudge`) and the
//! verse/hook boundary walk (`VerseHookJudge_All`, `RatioCalc_Verse`).

use crate::phrase_rb_d5::verse_hook_judge_lowpeak;

fn at(v: &[f64], i: i64) -> f64 {
    if i < 0 {
        0.0
    } else {
        v.get(i as usize).copied().unwrap_or(0.0)
    }
}
fn atu(v: &[u8], i: i64) -> u8 {
    if i < 0 {
        0
    } else {
        v.get(i as usize).copied().unwrap_or(0)
    }
}
fn z3(v: &mut [f64], i: i64) -> f64 {
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
fn fold(x: f64) -> f64 {
    if x < 1.0 {
        1.0 / x
    } else {
        x
    }
}

/// `IrregularJudge` → (count, candidates, the position list the last low-peak call left).
#[allow(clippy::too_many_arguments)]
pub fn irregular_judge(p0: i32, p1: i32, p2: &[i32], flags: &[f64], bar: i32, p5: i32, p6: i32, p7: i32, p8: i32, p9: i32, p10: i32, p11: i32, ll: &[u8], p13: i32) -> Option<(i32, Vec<i32>, Vec<i32>)> {
    if flags.is_empty() || ll.is_empty() {
        return None;
    }
    let v26: Vec<u8> = ll.iter().flat_map(|x| [*x; 4]).collect();
    let mut last: Vec<i32> = p2.to_vec();
    let mut v52 = p1;
    let mut lp = |x: i32, last: &mut Vec<i32>, v52: &mut i32| -> Option<()> {
        let (c, v) = verse_hook_judge_lowpeak(x, flags, bar, ll)?;
        *v52 = c;
        *last = v;
        Some(())
    };
    let mut v51 = 0;
    let mut v29 = p5;
    if p6 != 0 {
        lp(p6, &mut last, &mut v52)?;
        let v31 = v52;
        lp(p7, &mut last, &mut v52)?;
        let v32 = if p13 == 1 { v52 + 2 } else { v52 };
        v29 = if v31 <= v32 { p7 } else { p5 };
    }
    if p8 != 0 && v29 != p9 {
        lp(p8, &mut last, &mut v52)?;
        let a = v52;
        lp(p9, &mut last, &mut v52)?;
        if a <= v52 {
            if v29 == p5 {
                v29 = p9;
            } else {
                v51 = p9;
            }
        }
    }
    let mut p10b = v51;
    if v29 != p11 && p10 != 0 && v51 == 0 {
        lp(p10, &mut last, &mut v52)?;
        let a = v52;
        lp(p11, &mut last, &mut v52)?;
        if a <= v52 {
            if v29 == p5 {
                p10b = 0;
                v29 = p11;
            } else {
                v51 = p11;
                p10b = p11;
            }
        } else {
            p10b = 0;
        }
    }
    let d = |i: i32| at(flags, i as i64);
    let fl = |i: i32| atu(&v26, i as i64);
    let p9a = 1 - bar;
    let test = |o: i32| p9a + p5 + o >= 1 && (d(p5 + o - 1) > 0.0 || d(p5 + o - 2) > 0.0 || fl(p5 + o - bar) != 0);
    let mut early82 = false;
    if !(p5 == 0 || v29 != p5) {
        let mut p8a = 0;
        let mut go74 = false;
        let mut from51 = false;
        let mut from63 = false;
        if p5 >= 2 {
            if test(0) {
                lp(p5, &mut last, &mut v52)?;
                p8a = v52.max(0);
            }
            from51 = true;
        } else if p5 >= -2 {
            from51 = true;
        } else if (p5 as u32) >= 0xFFFFFFFA {
            from63 = true;
        } else if (p5 as u32) < 0xFFFFFFF6 {
            early82 = true;
        } else {
            v29 = p5;
            go74 = true;
        }
        if from51 {
            if test(4) {
                lp(p5 + 4, &mut last, &mut v52)?;
                v29 = if p8a >= v52 { p5 } else { p5 + 4 };
                p8a = p8a.max(v52);
            } else {
                v29 = p5;
            }
            from63 = false;
            go74 = false;
            // LABEL_64
            if test(8) {
                lp(p5 + 8, &mut last, &mut v52)?;
                if p8a < v52 {
                    v29 = p5 + 8;
                }
                p8a = p8a.max(v52);
            }
            go74 = true;
        }
        if from63 {
            v29 = p5;
            if test(8) {
                lp(p5 + 8, &mut last, &mut v52)?;
                if p8a < v52 {
                    v29 = p5 + 8;
                }
                p8a = p8a.max(v52);
            }
            go74 = true;
        }
        if go74 && !early82 && test(12) {
            lp(p5 + 12, &mut last, &mut v52)?;
            if p8a < v52 {
                v29 = p5 + 12;
            }
        }
    }
    let (mut p14, mut out, v41);
    if early82 || v29 == p5 {
        p14 = 0;
        out = vec![p0];
        v41 = p10b;
    } else {
        p14 = 1;
        out = vec![p0, v29];
        v41 = v51;
    }
    if v41 != 0 {
        p14 = 2;
        out.push(v51);
    }
    Some((p14, out, last))
}

/// `RatioCalc_Verse`: how much the bands change at `p0` (larger = stronger boundary).
fn ratio_calc_verse(p0: i32, lp: &mut [f64], ll: &mut [f64], ml: &mut [f64], lv: &mut [f64], al: &mut [f64], fl: &[f64], k: f64) -> f64 {
    let p = p0 as i64;
    z3(lp, p - 1);
    z3(lp, p - 2);
    let v17 = (p0 as f64).mul_add(0.25, 0.5) as i32 as i64;
    z3(lp, p - 3);
    z3(ll, v17 - 1);
    z3(ll, v17);
    z3(ml, v17 - 1);
    z3(ml, v17);
    z3(lv, p - 1);
    z3(lv, p - 2);
    let v22 = z3(al, v17 - 1);
    let v23 = z3(al, v17);
    let v25 = at(fl, p - 1);
    let v27;
    let v29;
    if v25 != 0.0 || at(fl, p - 2) == 0.0 {
        let r = at(lp, p - 1) / at(lp, p - 2);
        v29 = if v25 == 0.80000000000000004 || v25 == 0.40000000000000002 { r * 0.69999999999999996 } else { r };
        v27 = at(fl, p - 2);
    } else {
        v27 = at(fl, p - 2);
        let r = at(lp, p - 2) / at(lp, p - 3);
        v29 = if v27 == 0.80000000000000004 || v27 == 0.40000000000000002 { r * 0.69999999999999996 } else { r };
    }
    let v29 = fold(v29);
    let s = (at(fl, p - 4) + at(fl, p - 5)) + (at(fl, p - 3) + v27);
    let v33 = if s == 3.0 { v29 * 1.2 } else { v29 };
    let v35 = fold(at(ll, v17) / at(ll, v17 - 1));
    let v36 = fold(at(ml, v17) / at(ml, v17 - 1));
    let v37 = fold(at(lv, p - 1) / at(lv, p - 2));
    let v38 = fold(v23 / v22);
    (((v35 + v33) + (v36 + v37)) + v38) * k
}

const STEPS8: [i32; 8] = [32, 64, 16, 96, 48, 80, 128, 112];

/// `VerseHookJudge_All` → boundary beats (absolute) walking each candidate segment in 16..128
/// beat steps.
#[allow(clippy::too_many_arguments)]
pub fn verse_hook_judge_all(p0: i32, pos: &[i32], bar: i32, flags: &[f64], llu: &[u8], mlu: &[u8], hhu: &[u8], lowpeak: &[f64], ll: &[f64], ml: &[f64], lvl: &[f64], al: &[f64]) -> Option<Vec<i32>> {
    let mut p1a = lowpeak.to_vec();
    let mut p2a = ll.to_vec();
    let mut p3a = ml.to_vec();
    let mut p4a = lvl.to_vec();
    let mut p5a = al.to_vec();
    if bar - 1 > flags.len() as i32 || bar < 1 {
        return None;
    }
    let mut p6a: Vec<f64> = flags[(bar - 1) as usize..].to_vec();
    let v24 = llu.len().min(mlu.len()).min(hhu.len()).min((flags.len() as f64).mul_add(0.25, 0.5) as i32 as usize).min((lvl.len() as f64).mul_add(0.25, 0.5) as i32 as usize);
    let x4 = |v: &[u8]| -> Vec<u8> { (0..4 * v24).map(|i| v[i / 4]).collect() };
    let v28 = x4(llu);
    let v103 = x4(mlu);
    let v102 = x4(hhu);
    let n = p6a.len();
    let v116 = 4 * v24;
    p6a.resize(v116, 0.0);
    let mut v43: Vec<i32> = pos.iter().map(|x| x + 1 - bar).collect();
    v43.push(p6a.len() as i32);
    let mut v44 = vec![0i32; n.max(1)];
    let mut v45 = vec![0i32; n.max(1)];
    if p0 < 0 {
        return Some(vec![]);
    }
    let v48 = n as f64 * 0.5;
    let mut v47 = 0usize;
    let mut v49: i32 = 1;
    let mut v50;
    let mut v97 = false;
    let gi = |v: &[i32], i: usize| v.get(i).copied().unwrap_or(0);
    let mut v46 = 0usize;
    'seg: loop {
        let mut v51 = gi(&v43, v46);
        let v100 = v46;
        let v101 = v46 + 1;
        let v52 = gi(&v43, v46 + 1);
        let v53 = if v44[0] != 0 { v47 } else { 0 };
        if v53 < v44.len() {
            v44[v53] = v51;
        }
        let v98 = v47;
        let v99 = v47 + 1;
        let v54 = v52 - 4;
        let mut v55 = 1usize;
        let mut v57 = 1i32;
        let mut v58 = v49;
        let v61: usize;
        loop {
            let cand = if (1..=8).contains(&v57) { v51 + STEPS8[(v57 - 1) as usize] } else { 16 * v57 + v51 };
            v49 = cand;
            if v49 >= v52 {
                // LABEL_94
                if v58 >= v54 {
                    v49 = v58;
                }
                v61 = v55;
                break;
            }
            if v58 >= v54 {
                v61 = v55;
                v49 = v58;
                break;
            }
            let i = (v49 - 1) as i64;
            let hit = |j: i64| at(&p6a, j) == 1.0 || at(&p6a, j - 1) == 1.0 || atu(&v28, j) == 1 || atu(&v103, j) == 1 || atu(&v102, j) == 1;
            let nv61;
            if !hit(i) {
                nv61 = v55;
            } else {
                if v57 <= 2 {
                    let (j, a, b, k2) = if v57 == 2 { (v51 + 47, v51 + 48, v51 + 64, 1.02) } else { (v51 + 15, v51 + 16, v51 + 32, if v48 >= (bar - 1 + v51 + 32) as f64 { 1.25 } else { 1.23 }) };
                    if hit(j as i64) {
                        let v59 = ratio_calc_verse(a, &mut p1a, &mut p2a, &mut p3a, &mut p4a, &mut p5a, &p6a, 1.0);
                        let v60 = ratio_calc_verse(b, &mut p1a, &mut p2a, &mut p3a, &mut p4a, &mut p5a, &p6a, k2);
                        v49 = if v59 <= v60 { b } else { a };
                    } else {
                        v49 = b;
                    }
                }
                if v55 - 1 < v45.len() {
                    v45[v55 - 1] = v49;
                }
                nv61 = v55 + 1;
                v51 = v49;
            }
            if v55 == nv61 {
                v57 += 1;
            } else {
                v57 = 1;
            }
            v55 = nv61;
            v58 = v49;
            if v57 >= 12 {
                v61 = v55;
                break;
            }
        }
        // LABEL_99
        if v45[0] == 0 {
            v50 = v99;
        } else {
            if v61 >= 2 {
                for j in 0..v61 - 1 {
                    if v99 + j < v44.len() {
                        v44[v99 + j] = v45[j];
                    } else {
                        v44.push(v45[j]);
                    }
                }
            }
            v45 = vec![0i32; n.max(1)];
            v50 = v61 + v98;
        }
        v97 = v100 >= p0 as usize;
        v47 = v50;
        v46 = v101;
        if v101 == (p0 + 1) as usize {
            break 'seg;
        }
    }
    let _ = v97;
    v44.resize(v50, 0);
    Some(v44.iter().map(|x| x + bar - 1).collect())
}

/// `CandidateAdjust`: drops candidates in quiet stretches (all-band level ≤ 0.32, or 0.32..0.56
/// with a weak low band) and past the end.
pub fn candidate_adjust(cands: &[i32], all: &[f64], low: &[f64]) -> Vec<i32> {
    let n = all.len();
    let mut v14 = cands.to_vec();
    let lim = 4 * n as i32 - 16;
    for k in 1..v14.len() {
        if cands[k] > lim {
            v14[k] = 0;
        }
    }
    let mut v43 = vec![0f64; 4 * n];
    let mut v44 = vec![0f64; 4 * n];
    if n >= 2 {
        for b in 0..n - 1 {
            for j in 0..4 {
                v43[4 * b + j] = all[b];
                v44[4 * b + j] = low.get(b).copied().unwrap_or(0.0);
            }
        }
    }
    let v124 = (4 * n) as i32;
    let g = |v: &[f64], i: i32| if i < 0 { 0.0 } else { v.get(i as usize).copied().unwrap_or(0.0) };
    if v14.len() > 1 {
        for k in (1..v14.len()).rev() {
            let x = v14[k];
            if x >= 1 && x <= v124 && g(&v43, x - 1) <= 0.32000000000000001 && g(&v43, x - 2) <= 0.32000000000000001 {
                v14[k] = 0;
            }
        }
        for k in (1..v14.len()).rev() {
            let x = v14[k];
            if x >= 1 && x <= v124 {
                let a = g(&v43, x - 1);
                let b = g(&v43, x - 2);
                let r = |v: f64| (0.32000000000000001..0.56000000000000005).contains(&v);
                if r(a) && r(b) && g(&v44, x - 1) <= 300.0 && g(&v44, x - 2) <= 300.0 {
                    v14[k] = 0;
                }
            }
        }
    }
    let out: Vec<i32> = v14.iter().copied().filter(|x| *x >= 1 && *x <= v124).collect();
    if out.is_empty() {
        vec![0]
    } else {
        out
    }
}

/// `RhythmPatternReject` → (kept candidates, mean similarity, similarity per 32-beat candidate,
/// flags): drops 32-beat boundaries whose rhythm pattern does not change.
pub fn rhythm_pattern_reject(cands: &[i32], r: &[u8]) -> (Vec<i32>, f64, Vec<f64>, Vec<u8>) {
    let n = cands.len();
    let mut v89 = cands.to_vec();
    let mut p5 = vec![];
    let mut v88 = vec![];
    let mut p6 = vec![0u8; n];
    let rb = |i: i64| -> u8 {
        if i < 0 {
            0
        } else {
            r.get(i as usize).copied().unwrap_or(0)
        }
    };
    let eqc = |a: i64, b: i64, k: i64| (0..k).filter(|i| rb(a + i) == rb(b + i)).count() as i32;
    let v85 = r.len() as f64;
    let mut v20 = 0usize;
    if n >= 2 {
        for v22 in 1..n {
            let v23 = cands[v22];
            if v85.mul_add(0.25, -(v23 as f64)) < 32.0 {
                break;
            }
            if v23 - cands[v22 - 1] != 32 {
                continue;
            }
            v88.push(v23);
            let v24 = 4 * v23 as i64;
            let v27 = eqc(v24 - 121, v24 + 7, 40) + eqc(v24 - 57, v24 + 71, 40);
            p5.push(v27 as f64 * 0.012500000000000001);
            if v22 != 1 && v22 != n - 1 {
                let v52 = eqc(v24 - 49, v24 - 17, 16);
                let v56 = eqc(v24 - 113, v24 - 49, 16);
                if v52 < v56 {
                    p6[v20] = 1;
                }
            }
            v20 += 1;
        }
    }
    let v59 = if p5.is_empty() { 0.0 } else { crate::phrase_rb_dsp::sum8(&p5) / p5.len() as f64 };
    if v20 == 0 {
        return (v89, v59, p5, p6);
    }
    let gi = |v: &[i32], i: i64| if i < 0 { 0 } else { v.get(i as usize).copied().unwrap_or(0) };
    for v69 in 0..v20 {
        let idx: Vec<i64> = (0..v89.len()).filter(|k| v89[*k] == v88[v69]).map(|k| k as i64 + 1).collect();
        if idx.is_empty() {
            continue;
        }
        if !idx.iter().all(|i| gi(&v89, i - 1) - gi(&v89, i - 2) == 32) {
            continue;
        }
        let del = if v69 == 0 { p6[0] == 0 && v59 < p5[0] } else { (if p6[v69] == 0 { 1.02 } else { 1.0600000000000001 }) * v59 < p5[v69] };
        if del {
            for i in &idx {
                let k = (i - 1) as usize;
                if k < v89.len() {
                    v89.remove(k);
                }
            }
        }
    }
    (v89, v59, p5, p6)
}
