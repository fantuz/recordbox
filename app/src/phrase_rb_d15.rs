//! Stage D of the phrase analyser port, part 15: extra boundaries for the type-2 structure
//! (all Low-mood tracks and the Mid-mood tracks `SongStructureAna_Type_Genre_Judge` types 2):
//! `VerseHookAdd_genre23` with `SouonChangeDet`, `VocalInDetection` and `CandidatesAdjustment`.

use crate::phrase_rb_d1::sum_range;
use crate::phrase_rb_d8::{short_break_detection, snare_fillin_det, sort_del0};
use crate::phrase_rb_dsp::{isum, max_val};

/// `MatArrayInt::sum(a, b)`: inclusive range, 0 when out of range.
fn isum_range(x: &[i32], a: i64, b: i64) -> i32 {
    if a < 0 || a > b || b >= x.len() as i64 {
        return 0;
    }
    isum(&x[a as usize..=b as usize])
}

/// `SouonChangeDet` → bar numbers (from 2) where the level changes by a factor of 1.8 or more
/// between two bars and then stays (the next two bar-to-bar ratios average at most 1.11).
/// `lvl` is the per-beat level (`IntroEndFind_b`'s third output). The list is zero-padded to
/// `lvl.len() / 4`.
pub fn souon_change_det(lvl: &[f64]) -> Vec<i32> {
    let n = lvl.len() as i32;
    let v6 = n / 4;
    let nb = v6.max(0) as usize;
    let mut v57 = vec![0f64; nb];
    let mut v56 = vec![0f64; nb];
    let v10 = v6 - 1;
    if n >= 8 {
        for k in 0..v10.max(1) as usize {
            v57[k] = sum_range(lvl, 4 * k as i32, 4 * k as i32 + 3) * 0.25;
        }
        if n >= 12 {
            for k in 1..v10.max(2) as usize {
                if v57[k - 1] == 0.0 {
                    v57[k - 1] = 0.003;
                }
                if v57[k] == 0.0 {
                    v57[k] = 0.003;
                }
                let r = v57[k] / v57[k - 1];
                v56[k] = if r < 1.0 { 1.0 / r } else { r };
            }
        }
    }
    let mut out = vec![0i32; nb];
    if n >= 20 {
        let mut w = 0usize;
        for i in 0..(v6.max(5) - 4) as usize {
            let v51 = v56[i + 1];
            if !(v51 == 0.0 || v51 < 1.8) && sum_range(&v56, i as i32 + 3, i as i32 + 4) * 0.5 <= 1.11 {
                out[w] = i as i32 + 2;
                w += 1;
            }
        }
    }
    out
}

/// `VocalInDetection` → bar numbers where a voice comes in: per-beat vocal presence (`uta`,
/// `UtaUmuAnalyze_a`) after a gap of at least eight silent beats (and two of the next three
/// beats voiced), and — when voice is present on fewer than 60 % of the beats — after a gap of
/// three to seven beats with a loud, rising entry.
pub fn vocal_in_detection(uta: &[f64]) -> Vec<i32> {
    vocal_in_detection_with(uta, true)
}

/// `vocal_in_detection` with the second (short-gap) detector switchable, for tests.
pub fn vocal_in_detection_with(uta: &[f64], short_gaps: bool) -> Vec<i32> {
    let mut v6 = uta.to_vec();
    let n = v6.len() as i64;
    let mx = max_val(&v6);
    let f = if mx == 0.0 { 333.33333333333331 } else { 1.0 / mx };
    for x in v6.iter_mut() {
        *x *= f;
    }
    let nu = n.max(0) as usize;
    let v21 = (n / 4).max(0) as usize;
    let mut v20 = vec![0i32; nu];
    let mut v22 = vec![0f64; v21];
    let mut v23 = vec![0f64; v21];
    if n >= 8 {
        let v25 = v21 as i64 - 1;
        for k in 0..v25.max(1) as usize {
            v22[k] = sum_range(&v6, 4 * k as i32, 4 * k as i32 + 3);
        }
        if n >= 12 {
            for k in 1..v25.max(2) as usize {
                v23[k] = if v22[k - 1] != 0.0 { v22[k] / v22[k - 1] } else { 0.0 };
            }
        }
    }
    let v35 = n - 4;
    if n >= 5 {
        for i in 0..v35 as usize {
            if v6[i] > 0.0 {
                v20[i] = 1;
            }
        }
    }
    let v51 = n == 4 || (isum(&v20) as f64 / v35 as f64) < 0.6;
    let mut out = vec![0i32; nu];
    let mut w = 0usize;
    if n >= 5 {
        let (mut armed, mut cnt) = (false, 0i32);
        for i in 0..v35 {
            let x = v20[i as usize];
            let next = if x != 0 { cnt } else { cnt + 1 };
            armed |= x == 0 && cnt > 6;
            if x == 1 && armed {
                if isum_range(&v20, i, i + 2) >= 2 {
                    out[w] = i as i32 + 1;
                    w += 1;
                }
                armed = false;
                cnt = 0;
            } else if armed {
                cnt = next;
            } else {
                cnt = 0;
            }
            if x != 1 {
                cnt = next;
            }
        }
    }
    if short_gaps && v51 && n > 8 {
        let (mut armed, mut cnt) = (false, 0i32);
        for i in 0..n - 8 {
            if v20[i as usize] == 0 {
                if cnt <= 6 {
                    armed |= cnt > 1;
                } else {
                    armed = false;
                }
                cnt += 1;
            } else {
                let b = (i as u32 >> 2) as usize;
                if armed && isum_range(&v20, i, i + 2) >= 3 && v6[i as usize] >= 0.7 && (v23.get(b).copied().unwrap_or(0.0) >= 1.8 || v23.get(b + 1).copied().unwrap_or(0.0) >= 3.0) {
                    out[w] = i as i32 + 1;
                    w += 1;
                }
                armed = false;
                cnt = 0;
            }
        }
    }
    sort_del0(&mut out);
    for x in out.iter_mut() {
        let v77 = if *x >= 0 { *x } else { x.wrapping_add(3) };
        if x.wrapping_sub(v77 & !3) >= 0 {
            *x = (v77 >> 2) + 1;
        }
    }
    out
}

/// `CandidatesAdjustment`: the structure's boundary beats (`beats`) plus the extra boundary bars
/// (`bars`, sorted, distinct) that are not next to a boundary (from the second one on) and not
/// the first of two consecutive bars, as beats; bars after `nbars - 4` are dropped.
pub fn candidates_adjustment(beats: &[i32], bars: &[i32], bar: i32, nbars: i32) -> Vec<i32> {
    let mut v98 = bars.to_vec();
    if bars.len() >= 2 {
        for k in 0..bars.len() - 1 {
            if bars[k + 1] == bars[k].wrapping_add(1) {
                v98[k] = 0;
            }
        }
    }
    sort_del0(&mut v98);
    let mut add = v98;
    let own: Vec<i32> = beats.iter().map(|x| x.wrapping_sub(bar) / 4 + 1).collect();
    if own.len() >= 2 && !add.is_empty() {
        for &c in &own[1..] {
            for x in add.iter_mut() {
                if c == *x || c.wrapping_sub(1) == *x || c.wrapping_add(1) == *x {
                    *x = 0;
                }
            }
        }
    }
    let mut out = own;
    out.extend_from_slice(&add);
    sort_del0(&mut out);
    while let Some(&l) = out.last() {
        if l <= nbars - 4 {
            break;
        }
        out.pop();
    }
    out.iter().map(|x| (bar - 4).wrapping_add(x.wrapping_mul(4))).collect()
}

/// Number of values left after an ascending sort that drops the leading zeros
/// (`MatArrayFloat::sort_val_and_delete0(true)` on a copy).
fn count_sorted_nonzero(x: &[f64]) -> i32 {
    let mut v = x.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (v.len() - v.iter().take_while(|x| **x == 0.0).count()) as i32
}

/// The extra boundary bars `VerseHookAdd_genre23` collects (sorted, distinct, no zeros) and the
/// bar count it works with.
pub fn genre23_bars(s3960: &[f64], s3832: &[f64], hp: &[f64], uta: &[f64], lvl: &[f64]) -> (Vec<i32>, i32) {
    let v18 = count_sorted_nonzero(s3832);
    let v19 = v18 / 4;
    let mut bars = short_break_detection(s3960, s3832, v18, v19);
    bars.extend(souon_change_det(lvl));
    bars.extend(snare_fillin_det(hp, v19));
    bars.extend(vocal_in_detection(uta));
    bars.retain(|&bar| bar != 0);
    bars.sort_unstable();
    bars.dedup();
    (bars, v19)
}

/// `VerseHookAdd_genre23` → boundary beats: `VerseHookJudge23`'s boundaries (`cands`) with short
/// breaks, level changes, snare fills and vocal entries added (`CandidatesAdjustment`).
pub fn verse_hook_add_genre23(s3960: &[f64], s3832: &[f64], hp: &[f64], uta: &[f64], lvl: &[f64], cands: &[i32], bar: i32) -> Vec<i32> {
    let (bars, nbars) = genre23_bars(s3960, s3832, hp, uta, lvl);
    candidates_adjustment(cands, &bars, bar, nbars)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values the reference's own routines return for the same inputs (checked with the
    /// research harness `rbfn`).
    #[test]
    fn helpers_match_the_reference_on_small_inputs() {
        let mut lvl = vec![1.0; 8];
        lvl.extend(vec![4.0; 16]);
        assert_eq!(souon_change_det(&lvl), vec![3, 0, 0, 0, 0, 0]);
        let mut uta = vec![0.0; 10];
        uta.extend(vec![1.0; 6]);
        assert_eq!(vocal_in_detection(&uta), vec![3]);
        assert_eq!(candidates_adjustment(&[1, 33, 65], &[3, 5, 6, 12], 1, 20), vec![1, 9, 21, 33, 45]);
        // Nothing to add: boundaries after bar nbars − 4 go (beat 65 is bar 17 of 20).
        assert_eq!(candidates_adjustment(&[1, 65, 129], &[], 1, 20), vec![1]);
    }
}
