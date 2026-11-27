//! The bar-phase vote at the end of `BeatAnalyzer_2_0::Analyzer::analyzeFocus` (0x10116c050,
//! code from 0x10117db28 on, after `UnitBeatAdjust::analyze`), ported from rekordbox 7.2.19.
//!
//! The beat positions are final at this point; the pass only decides which beat of the bar each
//! beat is (it rotates every beat number by the same amount). It looks at
//! - `low`: the low band integrated over one beat from each beat (`calcLowInteg` at half the
//!   tempo), `all`: the full band over one beat (`calcAllInteg`), with a value for the stretch
//!   before the first beat in front;
//! - their beat-to-beat changes, normalised by the largest one (`dlow`, `dall`);
//! - where the music starts (first strong rise of `dlow` / `dall`) and ends (last strong drop),
//!   and the beat numbers there (`intro`, `intro_all`, `outro`, `outro_all`);
//! - the beat numbers of the rises that stand out above the median (`onsets`, with a second
//!   count on the normalised `low` itself), and of the 16 loudest beats (`loud`);
//! and rotates the numbers so that the winning beat becomes beat 1.
//!
//! Float sums follow the original's vectorised order (8 lanes, see [`sum8`]) and its fused
//! multiply-adds, so every threshold comes out bit for bit.

use super::uba::{BeatInfo, Uba};

const EPS: f64 = 2.22044605e-16;

/// The original's 8-lane sum of `v` in this order: lane `j % 8` collects elements `j`, `j + 8`,
/// ... of the first `len & !7` (when there are at least 8), the lanes are combined as
/// `((l0 + l2) + l4) + l6` plus `((l1 + l3) + l5) + l7`, then the rest is added one by one.
fn sum8(v: &[f64]) -> f64 {
    let n = v.len();
    let mut s = 0.0;
    let mut start = 0;
    if n >= 8 {
        let m = n & !7;
        let mut lanes = [0f64; 8];
        for g in (0..m).step_by(8) {
            for j in 0..8 {
                lanes[j] = v[g + j] + lanes[j];
            }
        }
        let r0 = ((lanes[0] + lanes[2]) + lanes[4]) + lanes[6];
        let r1 = ((lanes[1] + lanes[3]) + lanes[5]) + lanes[7];
        s = r0 + r1;
        start = m;
    }
    for x in &v[start..] {
        s = x + s;
    }
    s
}

/// Sum of squared deviations from `mean`, the same lanes as [`sum8`], each step a fused
/// multiply-add (`fmla` / `fmadd`).
fn sq8(v: &[f64], mean: f64) -> f64 {
    let n = v.len();
    let mut s = 0.0;
    let mut start = 0;
    if n >= 8 {
        let m = n & !7;
        let mut lanes = [0f64; 8];
        for g in (0..m).step_by(8) {
            for j in 0..8 {
                let d = v[g + j] - mean;
                lanes[j] = d.mul_add(d, lanes[j]);
            }
        }
        let r0 = ((lanes[0] + lanes[2]) + lanes[4]) + lanes[6];
        let r1 = ((lanes[1] + lanes[3]) + lanes[5]) + lanes[7];
        s = r0 + r1;
        start = m;
    }
    for x in &v[start..] {
        let d = x - mean;
        s = d.mul_add(d, s);
    }
    s
}

/// The next beat number in the bar (4 -> 1).
fn next(n: i32) -> i32 {
    if n < 4 { n + 1 } else { n - 3 }
}

/// The previous beat number (1 -> 4).
fn prev(n: i32) -> i32 {
    if n >= 2 { n - 1 } else { n + 3 }
}

/// How the numbers are changed, as the original writes each case.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Renumber {
    #[default]
    Keep,
    /// +1 (`n < 4 ? n + 1 : n - 3`).
    Next,
    /// +2 (`n > 2 ? n - 2 : n + 2`).
    Two,
    /// +3, i.e. one back (`n >= 2 ? n - 1 : n + 3`).
    Prev,
    /// `n + s`, plus 4 when that is below 1.
    Shift(i32),
}

fn apply(list: &mut [BeatInfo], r: Renumber) {
    for b in list.iter_mut() {
        b.num = match r {
            Renumber::Keep => b.num,
            Renumber::Next => next(b.num),
            Renumber::Two => {
                if b.num > 2 { b.num - 2 } else { b.num + 2 }
            }
            Renumber::Prev => prev(b.num),
            Renumber::Shift(s) => {
                let v = b.num + s;
                if v < 1 { v + 4 } else { v }
            }
        };
    }
}

/// Values the vote is based on, for stage checks against the original.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoteTrace {
    /// Beat numbers where the music starts (low band, full band) and ends.
    pub intro: i32,
    pub intro_all: i32,
    pub outro: i32,
    pub outro_all: i32,
    /// The late rises of the envelopes' running maxima are scattered (low band, full band).
    pub scattered: (bool, bool),
    /// Median of |dlow| and the onset count of the first round (before the spacing filter).
    pub median: f64,
    pub onsets: i32,
    /// Onset beat numbers after the spacing filter, per beat number 1..4.
    pub onset_counts: [i32; 4],
    /// Beat numbers of the 16 loudest beats (low band), per beat number.
    pub loud: [i32; 4],
    /// Votes of the start / end beats per beat number, first and last winner (0-based).
    pub ends: [i32; 4],
    pub ends_first: i32,
    pub ends_last: i32,
    /// Winner of the onset rounds (1-based).
    pub winner: i32,
    /// What was done to the numbers.
    pub renumber: Renumber,
}

/// The vote on the refined list (`list` = `Analyzer` +112 after `UnitBeatAdjust`, `bpm` its
/// tempo, `sr` the sample rate). Changes only the beat numbers.
pub fn bar_phase_vote(env: &Uba, bpm: f32, sr: i32, list: &mut [BeatInfo]) -> VoteTrace {
    let mut tr = VoteTrace::default();
    let n = list.len();
    if n < 2 {
        return tr;
    }
    let ni = n as i32;
    let num = |list: &[BeatInfo], i: usize| list[i].num;
    // --- integrals at every beat; index 0 = the stretch before the first beat
    let pos0 = list[0].pos;
    let v3088 = sr as f64 * 60.0;
    let v3089 = bpm as f64 * pos0 as f64 / v3088;
    let mut low = vec![0f64; n + 2];
    let mut all = vec![0f64; n + 2];
    if v3089 >= 0.5 {
        let v3090 = if pos0 != 0 { v3088 / pos0 as f64 } else { 120.0 };
        let s8 = v3090 as f32;
        low[0] = env.calc_low_integ(s8 * 0.5, 0) * v3090 / bpm as f64;
        all[0] = env.calc_all_integ(s8, 0) * v3090 / bpm as f64;
    }
    for i in 0..n {
        let mut v = env.calc_low_integ(bpm * 0.5, list[i].pos);
        if v < 0.0 && i != 0 {
            v = 0.0;
        }
        low[i + 1] = v;
    }
    for i in 0..n {
        let mut v = env.calc_all_integ(bpm, list[i].pos);
        if v < 0.0 && i != 0 {
            v = 0.0;
        }
        all[i + 1] = v;
    }
    // --- beat-to-beat changes, normalised by the largest rise (the last one is left as it is)
    let diff_norm = |x: &[f64]| -> Vec<f64> {
        let mut d = vec![0f64; n + 2];
        d[0] = x[0];
        for k in 1..n + 2 {
            d[k] = x[k] - x[k - 1];
        }
        let mut mx = x[0].max(0.0);
        for &v in &d[1..=n] {
            mx = mx.max(v);
        }
        if mx.abs() > EPS {
            d[0] = x[0] / mx;
            let inv = 1.0 / mx;
            for v in d[1..=n].iter_mut() {
                *v *= inv;
            }
        }
        d
    };
    let dlow = diff_norm(&low);
    let dall = diff_norm(&all);
    // --- running maxima from the end; where they last exceed 0.7; scattered late rises
    let suffix = |d: &[f64]| -> Vec<f64> {
        let mut e = vec![0f64; n + 2];
        e[n + 1] = d[n + 1].abs();
        for k in (0..=n).rev() {
            e[k] = e[k + 1].max(d[k].abs());
        }
        e
    };
    let elow = suffix(&dlow);
    let eall = suffix(&dall);
    let last_above = |e: &[f64]| -> usize {
        let mut idx = n + 2;
        while e[idx - 1] <= 0.7 {
            idx -= 1;
            if idx == 0 {
                break;
            }
        }
        idx
    };
    let scattered = |e: &[f64], from: usize| -> bool {
        // drops of the running maximum after `from`, the first four positions kept
        let mut cnt = 0i32;
        let mut p = [0i32; 4];
        if from < n {
            for j in from..n {
                if e[j] - e[j + 1] > 0.005 {
                    if cnt <= 3 {
                        p[cnt as usize] = j as i32;
                    }
                    cnt += 1;
                }
            }
        }
        if cnt > 4 {
            return true;
        }
        let mut m = (cnt - 1).max(0);
        let mut i = 0;
        loop {
            let before = m;
            if m == 0 {
                return before != 0;
            }
            let d = p[i + 1] - p[i];
            m -= 1;
            i += 1;
            if d != 1 {
                return before != 0;
            }
        }
    };
    let scat_low = scattered(&elow, last_above(&elow));
    let scat_all = scattered(&eall, last_above(&eall));
    tr.scattered = (scat_low, scat_all);
    // --- where the music starts
    let v3234 = ni.min(48) as usize;
    let first_number = |k: usize, list: &[BeatInfo]| -> i32 { if k != 0 { num(list, k - 1) } else { prev(num(list, 0)) } };
    let (intro, intro_all);
    if v3089 >= 0.5 {
        intro = (0..=n).find(|&k| dlow[k] > 0.35).map(|k| first_number(k, list)).unwrap_or(0);
        let vals: Vec<f64> = dall[0..=v3234].iter().map(|x| x.abs()).collect();
        let cnt = (v3234 + 1) as f64;
        let mean = sum8(&vals) / cnt;
        let var = sq8(&vals, mean);
        let thr = (var / cnt).sqrt().mul_add(1.5, mean);
        intro_all = (0..=n).find(|&k| dall[k] > thr).map(|k| first_number(k, list)).unwrap_or(0);
    } else {
        intro = (1..=n).find(|&k| dlow[k] > 0.35).map(|k| num(list, k - 1)).unwrap_or(0);
        let (mean, var) = if v3234 == 0 {
            (0.0, 0.0)
        } else {
            let vals: Vec<f64> = dall[1..=v3234].iter().map(|x| x.abs()).collect();
            let mean = sum8(&vals) / v3234 as f64;
            (mean, sq8(&vals, mean))
        };
        let thr = (var / v3234 as f64).sqrt().mul_add(1.5, mean);
        intro_all = (1..=n).find(|&k| dall[k] > thr).map(|k| num(list, k - 1)).unwrap_or(0);
    }
    // --- where it ends: the last drop below -(mean + sd / 2) of the last 48 changes
    let outro_of = |d: &[f64]| -> i32 {
        let bottom = (n + 1) as i64 - v3234 as i64; // v3346
        let (mean, var) = if bottom <= ni as i64 {
            let vals: Vec<f64> = ((bottom + 1) as usize..=n + 1).rev().map(|k| d[k].abs()).collect();
            let mean = sum8(&vals) / v3234 as f64;
            (mean, sq8(&vals, mean))
        } else {
            (0.0, 0.0)
        };
        let sd = (var / v3234 as f64).sqrt();
        let thr = sd.mul_add(-0.5, -mean);
        let mut k = n + 1;
        loop {
            if d[k] < thr {
                return if k <= n { num(list, k - 1) } else { next(num(list, k - 2)) };
            }
            if k <= 2 {
                return 0;
            }
            k -= 1;
        }
    };
    let outro = outro_of(&dlow);
    let outro_all = outro_of(&dall);
    tr.intro = intro;
    tr.intro_all = intro_all;
    tr.outro = outro;
    tr.outro_all = outro_all;
    // --- median of |dlow|
    let median_desc = |v: &[f64]| -> f64 {
        let mut a: Vec<f64> = v.iter().map(|x| x.abs()).collect();
        let len = a.len();
        for i in 0..len {
            for j in i + 1..len {
                if a[i] < a[j] {
                    a.swap(i, j);
                }
            }
        }
        if len % 2 == 1 { a[len >> 1] } else { (a[len >> 1] + a[(len >> 1) - 1]) * 0.5 }
    };
    // onsets: changes above median + 0.2 (+ 0.1 when that finds at most 4), the beat number of the
    // beat the change leads into
    let onsets = |d: &[f64], med: f64| -> (Vec<i32>, Vec<i32>) {
        let pick = |thr: f64| -> (Vec<i32>, Vec<i32>) {
            let (mut ks, mut nums) = (vec![], vec![]);
            for k in 1..=(n + 1) {
                if d[k] > thr {
                    ks.push(k as i32);
                    nums.push(if k <= n { num(list, k - 1) } else { next(num(list, k - 2)) });
                }
            }
            (ks, nums)
        };
        let r = pick(med + 0.2);
        if r.0.len() > 4 { r } else { pick(med + 0.1) }
    };
    // keep an onset when it is at least 5 beats after each of the (up to) four before it
    let spaced = |ks: &[i32], nums: &[i32]| -> Vec<i32> {
        let c = ks.len();
        if c == 0 {
            return vec![];
        }
        let mut out = vec![nums[0]];
        for i in 1..c {
            let ok = (1..=4.min(i)).all(|b| ks[i] - ks[i - b] >= 5);
            if ok {
                out.push(nums[i]);
            }
        }
        out
    };
    let count = |v: &[i32]| -> [i32; 4] {
        let mut c = [0i32; 4];
        if v.len() > 1 {
            for &x in v {
                if (1..=4).contains(&x) {
                    c[(x - 1) as usize] += 1;
                }
            }
        }
        c
    };
    let med = median_desc(&dlow);
    let (ks1, nums1) = onsets(&dlow, med);
    tr.median = med;
    tr.onsets = ks1.len() as i32;
    let kept1 = spaced(&ks1, &nums1);
    let c = count(&kept1);
    tr.onset_counts = c;
    // --- the 16 loudest beats (low band), each with the number of the beat after it
    let mut order: Vec<usize> = (0..n + 2).collect();
    let mut lv = low.clone();
    let mut knum: Vec<i32> = (0..n).map(|i| num(list, i)).collect();
    knum.push(next(num(list, n - 1)));
    let l = num(list, n - 1);
    knum.push(l + if l > 2 { -2 } else { 2 });
    for i in 0..n + 1 {
        for j in i + 1..n + 2 {
            if lv[i] < lv[j] {
                order.swap(i, j);
                knum.swap(i, j);
                lv.swap(i, j);
            }
        }
    }
    let mut loud = [0i32; 4];
    for &x in knum.iter().take(16) {
        if (1..=4).contains(&x) {
            loud[(x - 1) as usize] += 1;
        }
    }
    tr.loud = loud;
    let (n1, n2, n3, n4) = (loud[0], loud[1], loud[2], loud[3]);
    // --- votes of the start and the end
    let mut e = [0i32; 4];
    let mut vote = |x: i32, e: &mut [i32; 4]| {
        if (1..=4).contains(&x) {
            e[(x - 1) as usize] += 1;
        }
    };
    vote(intro, &mut e);
    vote(intro_all, &mut e);
    if !scat_low {
        vote(outro, &mut e);
    }
    if !scat_all {
        vote(outro_all, &mut e);
    }
    let (v3504, v3505, v3506, v3507) = (e[0], e[1], e[2], e[3]);
    let v3511 = v3505.max(v3504);
    let mut v3512 = (v3504 < v3505) as i32;
    if v3511 < v3506 {
        v3512 = 2;
    }
    let v3513 = v3511.max(v3506);
    let v3514 = if v3507 <= v3513 { v3512 } else { 3 };
    let v3515 = v3513.max(v3507);
    let mut v3516;
    let mut skip = false;
    if v3514 != 0 {
        v3516 = if v3515 == v3504 { 0 } else { v3514 };
        if v3514 != 1 {
            if v3515 == v3505 {
                v3516 = 1;
            }
            if v3514 == 2 {
                skip = true;
            }
        }
    } else {
        v3516 = (v3515 == v3505) as i32;
    }
    if !skip && v3515 == v3506 {
        v3516 = 2;
    }
    if v3507 == v3513 {
        v3516 = 3;
    }
    let (v3928, v3933) = (v3514, v3516);
    tr.ends = e;
    tr.ends_first = v3928;
    tr.ends_last = v3933;
    // --- winner of the onset counts; a second round on the normalised low band when the first
    //     is close
    let (c1, c2, c3, c4) = (c[0], c[1], c[2], c[3]);
    let mut v3522 = c1.max(0);
    let mut v3523 = c2.max(v3522);
    let mut v3524 = (v3522 < c2) as i32;
    if v3523 < c3 {
        v3524 = 2;
    }
    let mut v3525 = c3.max(v3523);
    if v3525 < c4 {
        v3524 = 3;
    }
    if v3524 != 2 {
        if v3524 != 1 {
            let v3526 = c2.max(0);
            v3522 = if v3524 != 0 { v3523 } else { v3526 };
        }
        v3523 = c3.max(v3522);
    }
    let mut v3527 = c4.max(v3523);
    if v3525 < c4 {
        v3527 = v3523;
    }
    if v3525 <= c4 {
        v3525 = c4;
    }
    let v3528 = (v3525 - v3527).abs();
    let v3530 = if v3528 > 10 || v3527 <= 4 {
        v3524 + 1
    } else {
        let mut a = low.clone();
        let mut mx = 0f64;
        for &v in &a[0..=n] {
            mx = mx.max(v);
        }
        if mx.abs() > EPS {
            let inv = 1.0 / mx;
            for v in a[0..=n].iter_mut() {
                *v *= inv;
            }
        }
        let med2 = median_desc(&a);
        let (ks2, nums2) = onsets(&a, med2);
        let c2v = count(&spaced(&ks2, &nums2));
        let (d1, d2, d3, d4) = (c2v[0], c2v[1], c2v[2], c2v[3]);
        let mut v3625 = d1.max(0);
        let v3626 = (d1 > 0) as i32;
        let v3627 = v3625 < d2;
        v3625 = v3625.max(d2);
        let v3628 = if v3627 { 2 } else { v3626 };
        let v3629 = v3625 < d3;
        v3625 = v3625.max(d3);
        let v3630 = if v3629 { 3 } else { v3628 };
        if v3625 >= d4 { v3630 } else { 4 }
    };
    tr.winner = v3530;
    // --- the decision
    let v3631 = if outro >= 2 { -1 } else { 3 };
    let v3632 = if outro_all >= 2 { -1 } else { 3 };
    let only = |a: i32, b: i32, cc: i32, d: i32| -> bool { a != 0 && b == 0 && cc == 0 && d == 0 };
    let mut rot = Renumber::Keep;
    if only(c1, c2, c3, c4) {
        // already on beat 1
    } else if only(c2, c1, c3, c4) {
        rot = Renumber::Prev;
    } else if only(c3, c1, c2, c4) {
        rot = Renumber::Two;
    } else if only(c4, c1, c2, c3) {
        rot = Renumber::Next;
    } else {
        let v3652 = v3933 == 0 || v3928 == 0;
        if !v3652 || c1 == 0 || c2 > 1 || c3 > 1 || c4 >= 2 {
            let v3653 = v3933 == 1 || v3928 == 1;
            let v3722 = v3933 == 2 || v3928 == 2;
            let v3725 = v3933 == 3 || v3507 > v3513;
            let (i0, i1) = (intro, intro_all);
            let po = v3631 + outro;
            let pa = v3632 + outro_all;
            let cond_x = i0 != 1
                && i0 == i1
                && ((i0 == outro && i0 == outro_all) || (i0 == po && i0 == pa) || (i0 == po && i0 == outro_all) || (i0 == outro && i0 == pa));
            if v3653 && c1 <= 1 && c2 != 0 && c3 <= 1 && c4 <= 1 {
                rot = Renumber::Prev;
            } else if v3722 && c1 <= 1 && c2 <= 1 && c3 != 0 && c4 <= 1 {
                rot = Renumber::Two;
            } else if v3725 && c1 <= 1 && c2 <= 1 && c3 <= 10 && c4 != 0 {
                rot = Renumber::Next;
            } else if cond_x {
                rot = Renumber::Shift(1 - i0);
            } else if v3928 != 0 || v3933 != 0 || v3530 != 1 {
                if (v3928 != 0 && v3928 + 1 == v3530) || (v3933 >= 1 && v3933 + 1 == v3530) {
                    rot = Renumber::Shift(1 - v3530);
                } else if only(v3504, 0, 0, 0) && only(n1, n2, n3, n4) {
                    // the start and the loudest beats agree on beat 1
                } else if only(v3505, 0, 0, 0) && only(n2, n1, n3, n4) {
                    rot = Renumber::Prev;
                } else if only(v3506, 0, 0, 0) && only(n3, n1, n2, n4) {
                    rot = Renumber::Two;
                } else if only(v3507, 0, 0, 0) && only(n4, n1, n2, n3) {
                    rot = Renumber::Next;
                } else if v3928 != 0 && v3928 == v3933 {
                    rot = Renumber::Shift(-v3928);
                }
            }
        }
    }
    tr.renumber = rot;
    apply(list, rot);
    tr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum8_uses_the_original_lane_order() {
        // 11 values: lanes 0..7 take the first 8, the last 3 are added one by one
        let v: Vec<f64> = (0..11).map(|i| 1.0 + i as f64 * 1e-17 + if i == 3 { 1e16 } else { 0.0 }).collect();
        let r0 = ((v[0] + v[2]) + v[4]) + v[6];
        let r1 = ((v[1] + v[3]) + v[5]) + v[7];
        let want = v[10] + (v[9] + (v[8] + (r0 + r1)));
        assert_eq!(sum8(&v).to_bits(), want.to_bits());
        let short = [1.0, 2.0, 3.0];
        assert_eq!(sum8(&short), 6.0);
    }

    #[test]
    fn renumbering_cases_wrap_as_the_original() {
        let mut l: Vec<BeatInfo> = (1..=4).map(|n| BeatInfo::new(0, 0.0, n)).collect();
        apply(&mut l, Renumber::Next);
        assert_eq!(l.iter().map(|b| b.num).collect::<Vec<_>>(), [2, 3, 4, 1]);
        apply(&mut l, Renumber::Two);
        assert_eq!(l.iter().map(|b| b.num).collect::<Vec<_>>(), [4, 1, 2, 3]);
        apply(&mut l, Renumber::Prev);
        assert_eq!(l.iter().map(|b| b.num).collect::<Vec<_>>(), [3, 4, 1, 2]);
        apply(&mut l, Renumber::Shift(-2));
        assert_eq!(l.iter().map(|b| b.num).collect::<Vec<_>>(), [1, 2, 3, 4]);
        // an unfound intro (0) shifts by one without wrapping 4, as the original does
        apply(&mut l, Renumber::Shift(1));
        assert_eq!(l.iter().map(|b| b.num).collect::<Vec<_>>(), [2, 3, 4, 5]);
    }
}
