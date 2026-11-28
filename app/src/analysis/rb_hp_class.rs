//! The class bodies of `BeatAnalyzer_2_0::Analyzer::analyzeFocus`' tempo-change half
//! (0x10116c050, rekordbox 7.2.19): what happens to a long irregular stretch whose intervals sit
//! at a steady multiple of the reference beat (the network locked onto another pulse there).
//! Line numbers are of the coverage listing (`scratch/work/tmp/ann.c`).
//!
//! Each body: the stretch's start and end are widened backwards and forwards over "off" ratios
//! next to them (near the class but outside its window) and the beats in between removed; the
//! start and end are aligned to the beat 1, 2 or 3 reference beats away (within a quarter beat)
//! and the beats skipped over removed; then the class's own correction; then the gaps the
//! widening left at either end are filled with even beats, a beat is added before the first one
//! when there is room and beats after the last one up to the end of the audio; with the flag
//! set (an off ratio five away), one long interval is split.

use super::focus2::{INVALID_LIST, MAX_BEATS};
use super::uba::BeatInfo;

#[inline]
fn blank(pos: i32) -> BeatInfo {
    BeatInfo { pos, zero: 0, interval: 0.0, num: 0, minus1: -1, flag: false }
}

/// What the class bodies read from the pass: the distances from 1 (`q`, the original's
/// `v3941`), the rate, the audio length (Analyzer +28) and the reference interval (ms, `v223`).
pub struct Pass<'a> {
    pub q: &'a [f64],
    pub sr: i32,
    pub total: i32,
    pub v223: f64,
    pub cancelled: &'a dyn Fn() -> bool,
}

impl Pass<'_> {
    /// q[i]; past the array's end (the original would read past its vector) as a ratio of 1.
    #[inline]
    fn q(&self, i: i32) -> f64 {
        if i < 0 {
            return 0.0;
        }
        self.q.get(i as usize).copied().unwrap_or(0.0)
    }
}

/// The start of a stretch moved back over off ratios before it (the original's v484 / v519):
/// up to five; an off ratio at the fifth sets the flag.
fn back_start(p: &Pass, s0: i32, off: impl Fn(f64) -> bool, flag: &mut bool) -> i32 {
    if s0 == 0 || !off(p.q(s0 - 1)) {
        return s0;
    }
    // LABEL_559
    for k in 2..=4 {
        if s0 < k {
            return 0;
        }
        if !off(p.q(s0 - k)) {
            return s0 - k + 1;
        }
    }
    if s0 < 5 {
        return 0;
    }
    if off(p.q(s0 - 5)) {
        *flag = true;
        return s0 - 5;
    }
    s0 - 4
}

/// A stretch boundary moved forward over off ratios after it (v482 / v410 for the start,
/// v1376 / v970 for the end); `v481` = the list length the pass counts with (`v296`).
fn forward(p: &Pass, x: i32, v481: i32, off: impl Fn(f64) -> bool, flag: &mut bool) -> i32 {
    if x > v481 - 2 || !off(p.q(x + 1)) {
        return x;
    }
    if x > v481 - 3 || !off(p.q(x + 2)) {
        return x + 1;
    }
    if x > v481 - 4 || !off(p.q(x + 3)) {
        return x + 2;
    }
    if x > v481 - 5 || !off(p.q(x + 4)) {
        return x + 3;
    }
    if x <= v481 - 6 && off(p.q(x + 5)) {
        *flag = true;
        return x + 5;
    }
    x + 4
}

/// The end of a stretch moved back over off ratios before it (v1371 / v965). `first` = the
/// lower window of the off test: an off ratio there makes even the first step go deeper.
fn back_end(p: &Pass, e0: i32, off: impl Fn(f64) -> bool, first: impl Fn(f64) -> bool, flag: &mut bool) -> i32 {
    if e0 < 1 {
        return e0;
    }
    let x = p.q(e0 - 1);
    if !first(x) {
        if !off(x) {
            return e0;
        }
        if e0 < 2 {
            return 0;
        }
    } else if e0 < 2 {
        return 0;
    }
    // LABEL_1901
    if !off(p.q(e0 - 2)) {
        return e0 - 1;
    }
    if e0 < 3 {
        return 0;
    }
    if !off(p.q(e0 - 3)) {
        return e0 - 2;
    }
    if e0 < 4 {
        return 0;
    }
    if !off(p.q(e0 - 4)) {
        return e0 - 3;
    }
    if e0 < 5 {
        return 0;
    }
    if off(p.q(e0 - 5)) {
        *flag = true;
        return e0 - 5;
    }
    e0 - 4
}

#[inline]
fn in_window(target: f64, t: f64, tol: f64) -> bool {
    target > t - tol && target < t + tol
}

/// Which of the eight class bodies (the ratio class of the stretch; whole or part is only
/// which range the caller passes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Intervals at half the reference (12732-13598 whole, 13599-14456 part): every other beat
    /// removed.
    Half,
    /// At two thirds (14457-15338, 15339-16203): two of every three beats removed, a beat in
    /// the middle of each interval left.
    TwoThirds,
    /// At 1.5 (16204-17116, 17117-18000): every other beat removed, two beats at the thirds of
    /// each interval left.
    ThreeHalves,
    /// At twice (18001-18576, 18577-19174): a beat in the middle of each interval; this pair
    /// has no alignment step.
    Double,
}

/// One class body on the range `lo` ..= `hi` (the stretch's `start[k]` / `end[k]` for a whole
/// stretch, the class's own range for a part; both updated as the original does). `v296` = the
/// list length the pass counts with. Returns its new value (the original's v728).
pub fn body(list: &mut Vec<BeatInfo>, p: &Pass, lo: &mut i32, hi: &mut i32, v296: i32, kind: Kind) -> Result<i32, &'static str> {
    if (p.cancelled)() || !matches!(p.sr, 44_100 | 48_000) || !(1..=900 * p.sr).contains(&p.total) ||
        !p.v223.is_finite() || p.v223 <= 0.0 || !(2..=MAX_BEATS).contains(&list.len()) ||
        v296 < 2 || v296 as usize > list.len() || *lo < 0 || *lo > *hi || *hi >= v296 ||
        list[0].pos < 0 || list.last().unwrap().pos > p.total || list.windows(2).any(|w| w[0].pos >= w[1].pos) {
        return Err(INVALID_LIST);
    }
    // the off windows: ratios near the class but outside it (q is the distance from 1, so half
    // and double share q ~ 2, two thirds and 1.5 share q ~ 1.5)
    let near2 = matches!(kind, Kind::Half | Kind::Double);
    let first = |x: f64| if near2 { x > 1.07 && x < 1.8 } else { x > 1.07 && x < 1.425 };
    let off = |x: f64| first(x) || if near2 { x > 2.2 } else { x > 1.575 && x < 1.8 };
    let align = kind != Kind::Double;
    let sr = p.sr;
    let srf = sr as f64;
    let v223 = p.v223;
    let v3920 = v223 * 0.001;
    let v294 = v223 * 0.00025;
    let v3881 = v223 * 0.002;
    let v3869 = v223 * 0.003;
    let v297 = 1.0 / v223;
    let s0 = *lo;
    let e0 = *hi;
    let mut flag = false;
    let bs = back_start(p, s0, off, &mut flag);
    let fs0 = forward(p, s0, v296, off, &mut flag);
    let be = back_end(p, e0, off, first, &mut flag);
    let fe0 = forward(p, e0, v296, off, &mut flag);
    // the beats between the end moved back and forward are removed
    let mut len = list.len() as i32;
    let v3902 = be.max(0);
    let fe = fe0.min(len - 1);
    let mut e: i32;
    if fe <= v3902 {
        e = fe;
    } else {
        list.drain((v3902 + 1) as usize..=fe as usize);
        len = list.len() as i32;
        e = v3902;
    }
    *hi = e;
    // and those between the start moved back and forward
    let s = bs.max(0);
    let fs = if len - 1 >= fs0 { fs0 } else { len - 1 };
    if fs > s {
        list.drain((s + 1) as usize..=fs as usize);
        len = list.len() as i32;
        e -= fs - s;
        *hi = e;
    }
    *lo = s;
    if e >= len || len < 2 { return Err(INVALID_LIST); }
    let mut removed_start = false;
    let mut removed_end = false;
    if align {
        // the start aligned to the beat 1, 2 or 3 reference beats later (a quarter beat either
        // way)
        if len > s {
            let cnt = if s <= len - 5 { 5 } else { len - s };
            if cnt >= 1 {
                let t0 = list[s as usize].pos as f64 / srf;
                let inv = 1.0 / srf;
                // (the whole-stretch bodies read past the list's end without a check: as no match)
                let hit = |tgt: f64, j: i32| j < len && in_window(tgt, list[j as usize].pos as f64 * inv, v294);
                let found = (s + 1..=s + cnt)
                    .find(|&j| hit(t0 + v3920, j))
                    .or_else(|| (s + 1..=s + cnt).find(|&j| hit(t0 + v3881, j)))
                    .or_else(|| (s + 1..=s + cnt).find(|&j| hit(t0 + v3869, j)));
                if let Some(j) = found {
                    if s + 1 < j {
                        list.drain((s + 1) as usize..j as usize);
                        e -= j - s - 1;
                        *hi = e;
                        removed_start = true;
                    }
                }
            }
        }
        // the end aligned to the beat 1, 2 or 3 reference beats before
        if e >= 1 {
            let te = list[e as usize].pos as f64 / srf;
            let m = e.min(5);
            let tgt1 = te - v3920;
            let mut found = (1..=m).map(|d| e - d).find(|&j| in_window(tgt1, list[j as usize].pos as f64 / srf, v294));
            if found.is_none() {
                let inv = 1.0 / srf;
                let at = |j: i32| list[j as usize].pos as f64 * inv;
                found = (1..=m).map(|d| e - d).find(|&j| in_window(te - v3881, at(j), v294)).or_else(|| (1..=m).map(|d| e - d).find(|&j| in_window(te - v3869, at(j), v294)));
            }
            let v1447 = found.unwrap_or(e);
            let v1435 = e - 1;
            removed_end = v1435 > v1447;
            if removed_end && (list.len() as i32) > v1435 {
                // (the original erases at index e - 1 each time, so after the first one it takes
                // the end beat and those after it, not the beats before; the two-thirds part
                // body stops when the list gets that short, the others do not check)
                let remove = (e - 1 - v1447).min(list.len() as i32 - v1435);
                list.drain(v1435 as usize..(v1435 + remove) as usize);
                e -= remove;
                *hi = e;
            }
            // the class's own removal, from the second beat after the start
            let v1481 = *lo + 1;
            if v1481 < e {
                // Remove every second element, or two out of three. The final group may
                // straddle the nominated end; process it exactly as the native erase loop.
                let first = v1481 + 1;
                let cut = if kind == Kind::TwoThirds { 2 } else { 1 };
                let stride = cut + 1;
                let groups = (e - first + stride) / stride;
                let end = first + stride * groups;
                let previous = list.len();
                let mut index = 0;
                list.retain(|_| {
                    let keep = index < first || index >= end || (index - first) % stride >= cut;
                    index += 1;
                    keep
                });
                e -= (previous - list.len()) as i32;
                *hi = e;
            }
        }
    }
    // beats put into each interval of the stretch, from its end
    let mut len = list.len() as i32;
    let v1492 = fe > v3902 || removed_end;
    let v1493 = v1492 || e == len - 1;
    let mut v1494 = e - v1493 as i32;
    if v1494 >= len - 2 {
        v1494 = len - 2;
    }
    let v1495 = (fs > s) as i32 | removed_start as i32;
    let v1496 = *lo;
    let per = match kind {
        Kind::Half => 0,
        Kind::TwoThirds | Kind::Double => 1,
        Kind::ThreeHalves => 2,
    };
    if per > 0 && v1494 >= v1496 + v1495 {
        let first = v1496 + v1495;
        let last = v1494;
        if first < 0 || last + 1 >= len { return Err(INVALID_LIST); }
        let added = (last - first + 1) * per;
        check_growth(list.len(), added)?;
        let intervals = &list[first as usize..=last as usize + 1];
        let mut replacement = Vec::with_capacity((last - first + 1 + added) as usize);
        for pair in intervals.windows(2) {
            if (p.cancelled)() { return Err(INVALID_LIST); }
            let (a, b) = (pair[0].pos, pair[1].pos);
            replacement.push(pair[0]);
            if per == 2 {
                let third = ((b - a) as f64 * (1.0 / 3.0)) / srf;
                replacement.push(blank(third.mul_add(srf, a as f64 + 0.5) as i32));
                replacement.push(blank(srf.mul_add(third + third, a as f64 + 0.5) as i32));
            } else {
                replacement.push(blank(((b - a) as f64).mul_add(0.5, a as f64 + 0.5) as i32));
            }
        }
        list.splice(first as usize..=last as usize, replacement);
        e += added;
        *hi = e;
        len = list.len() as i32;
    }
    // the gap the end's widening left: even beats at the reference interval
    let fill = |list: &mut Vec<BeatInfo>, at: i32| -> Result<i32, &'static str> {
        if (p.cancelled)() || at < 0 || at as usize + 1 >= list.len() { return Err(INVALID_LIST); }
        let t0 = list[at as usize].pos as f64 / srf;
        let span = list[(at + 1) as usize].pos as f64 / srf - t0;
        let n = (span * 1000.0).mul_add(v297, 0.5) as i32;
        if n < 2 {
            return Ok(0);
        }
        check_growth(list.len(), n - 1)?;
        let inv = 1.0 / n as f64;
        let index = at as usize + 1;
        list.splice(index..index, (1..n).map(|i| blank((span * i as f64).mul_add(inv, t0).mul_add(srf, 0.5) as i32)));
        Ok(n - 1)
    };
    if v1492 && e >= 0 && e + 1 < len {
        let added = fill(list, e)?;
        if added > 0 {
            e += added;
            *hi = e;
        }
    }
    // the gap at the start (with the length counted before the end's fill)
    if v1495 != 0 && v1496 + 1 < len {
        let added = fill(list, v1496)?;
        if added > 0 {
            e += added;
            *hi = e;
        }
    }
    // a beat before the first one when there is room
    if list.len() < 2 { return Err(INVALID_LIST); }
    let first_pos = list[0].pos;
    let before = first_pos - (list[1].pos - first_pos);
    if before > 0 {
        check_growth(list.len(), 1)?;
        list.insert(0, blank(before));
        *hi = e + 1;
    }
    let mut v728 = list.len() as i32;
    // beats after the last one up to the end of the audio
    if v728 >= 2 {
        let last = list[(v728 - 1) as usize].pos;
        let d = last - list[(v728 - 2) as usize].pos;
        if d <= 0 || last > p.total { return Err(INVALID_LIST); }
        let remaining = p.total - last;
        if remaining > d {
            let count = remaining / d;
            check_growth(list.len(), count)?;
            list.extend((1..=count).map(|i| blank(last + i * d)));
        }
    }
    // (the length handed back counts the beats after the last one only with the flag set)
    if flag {
        v728 = list.len() as i32;
        split_long_interval(list, v728)?;
    }
    Ok(v728)
}

/// Check the whole expansion before allocating or shifting any suffix.
fn check_growth(len: usize, additional: i32) -> Result<(), &'static str> {
    if len > MAX_BEATS || additional < 0 || additional as usize > MAX_BEATS - len { return Err(INVALID_LIST); }
    Ok(())
}

/// With the flag set: split a long interval by the spacing of the following interval.
fn split_long_interval(list: &mut Vec<BeatInfo>, len: i32) -> Result<(), &'static str> {
    if len < 2 || len as usize > list.len() || list.len() > MAX_BEATS { return Err(INVALID_LIST); }
    let stop = if len - 2 <= 1 { 1 } else { len - 2 };
    let mut i = 1;
    let (at, d, next) = loop {
        if i == stop {
            return Ok(());
        }
        let a = list[i as usize].pos;
        let p = list[(i - 1) as usize].pos;
        let b = list[(i + 1) as usize].pos;
        let c = list[(i + 2) as usize].pos;
        let d = b - a;
        if !(d <= 5 * (a - p) && d <= 5 * (c - b)) {
            break (i, d, c - b);
        }
        i += 1;
    };
    if next <= 0 || d <= 0 { return Err(INVALID_LIST); }
    let n = d / next;
    if n < 2 {
        return Ok(());
    }
    check_growth(list.len(), n - 1)?;
    let step = d / n;
    let base = list[at as usize].pos;
    let index = at as usize + 1;
    list.splice(index..index, (1..n).map(|m| blank(base + step * m)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_tails_and_gap_splits_are_rejected_before_bulk_growth() {
        // 15 minutes of "remaining" audio and a one-sample final spacing.
        let total = 900 * 44_100;
        let mut beats: Vec<_> = (0..32).map(blank).collect();
        let q = [1.0; 32];
        let pass = Pass { q: &q, sr: 44_100, total, v223: 500.0, cancelled: &|| false };
        let (mut lo, mut hi) = (2, 14);
        assert_eq!(body(&mut beats, &pass, &mut lo, &mut hi, 32, Kind::Half), Err(INVALID_LIST));
        assert!(beats.capacity() < 100);

        let mut sparse = vec![blank(0), blank(1), blank(total - 1), blank(total)];
        let before = sparse.clone();
        assert_eq!(split_long_interval(&mut sparse, 4), Err(INVALID_LIST));
        assert_eq!(sparse, before);
    }
}
