//! The second half of `BeatAnalyzer_2_0::Analyzer::analyzeFocus` (0x10116c050, rekordbox 7.2.19):
//! what it does with the network's beat list when the tempo may change (`Analyzer` +45 set) or
//! the network did not number the first bar 1-4 (decompile lines 10774-20490 of
//! `scratch/out/d8/10116c050_analyzeFocus.c`; line numbers below are of the coverage listing).
//!
//! Two passes over the list (positions in samples):
//! 1. beat times in whole ms, their intervals, and the first steady run (11 / 20 / 30 ms) give
//!    the reference beat length (`v223`, the network's tempo when they disagree by more than
//!    25 %); each interval's ratio to it marks irregular stretches (two ratios off by more than
//!    18 % in a row);
//! 2. on the first pass the middle of the longest regular stretch becomes the reference beat
//!    (position and bar number);
//! 3. each irregular stretch, last first: a short one (under 8 intervals) that stands out from
//!    its neighbours is widened over the off intervals next to it and, when it spans a whole
//!    number of reference beats, re-filled with even beats; a long one is tested for halved,
//!    two-thirds, 1.5x and doubled tempo over the whole stretch or a part of it.
//! Then (twice) single long intervals are split, every beat gets its bar number from the
//! reference beat, the tempo the median of the per-beat tempi, and each beat the mean
//! interval of its bar (from 8 beats on each side).

use super::uba::BeatInfo;

pub const UNPORTED_CLASS: &str = "the tempo-change analysis found a stretch at half, two thirds, 1.5 or twice the tempo (a branch of rekordbox's analyzeFocus not built yet)";
pub const UNPORTED_HALFBEAT: &str = "the tempo-change analysis found beats half a beat off (a branch of rekordbox's analyzeFocus not built yet)";
pub const UNPORTED_SHORT: &str = "the tempo-change analysis got too few beats (a branch of rekordbox's analyzeFocus not built yet)";

/// Maximum beats in the supported fifteen-minute window at the public
/// 1000 BPM upper bound, with room for extrapolated boundary beats.
pub(super) const MAX_BEATS: usize = 15_002;
pub const INVALID_LIST: &str = "variable model grid is outside the supported timing bounds";

pub struct TempoFocus {
    pub list: Vec<BeatInfo>,
    pub bpm72: f32,
}

/// 1/3 (0x3fd5555555555555; the decompile prints 0.333333333).
const THIRD: f64 = 1.0 / 3.0;

#[inline]
fn blank(pos: i32) -> BeatInfo {
    BeatInfo { pos, zero: 0, interval: 0.0, num: 0, minus1: -1, flag: false }
}

/// Sum of `v` as the 4 x 2-lane loop of the original (lanes l0..l7 over blocks of 8, then
/// ((l6 + (l4 + (l2 + l0))) + (l7 + (l5 + (l3 + l1))))), then the rest in order.
fn sum8(v: &[f64]) -> f64 {
    let blocks = v.len() & !7;
    let mut l = [0f64; 8];
    for c in (0..blocks).step_by(8) {
        for j in 0..8 {
            l[j] = v[c + j] + l[j];
        }
    }
    let a = l[6] + (l[4] + (l[2] + l[0]));
    let b = l[7] + (l[5] + (l[3] + l[1]));
    let mut s = if blocks > 0 { a + b } else { 0.0 };
    for x in &v[blocks..] {
        s = x + s;
    }
    s
}

/// Sum of squared deviations as the same lane layout (fused multiply-adds).
fn sq8(v: &[f64], m: f64) -> f64 {
    let blocks = v.len() & !7;
    let mut l = [0f64; 8];
    for c in (0..blocks).step_by(8) {
        for j in 0..8 {
            let d = v[c + j] - m;
            l[j] = d.mul_add(d, l[j]);
        }
    }
    let a = l[6] + (l[4] + (l[2] + l[0]));
    let b = l[7] + (l[5] + (l[3] + l[1]));
    let mut s = if blocks > 0 { a + b } else { 0.0 };
    for x in &v[blocks..] {
        let d = x - m;
        s = d.mul_add(d, s);
    }
    s
}

/// The bar of candidate `a` re-spaced from its start at 1 / (k + 0.5) of its length.
fn respace_from_start(list: &mut [BeatInfo], a: usize, k: i32) {
    let span = (list[a + 4].pos - list[a].pos) as f64;
    let base = list[a].pos as f64 + 0.5;
    list[a].pos = base as i32;
    let tb = base.trunc();
    let inv = 1.0 / (k as f64 + 0.5);
    list[a + 1].pos = (span.mul_add(inv, 0.5) + tb) as i32;
    list[a + 2].pos = (span.mul_add(inv + inv, 0.5) + tb) as i32;
    list[a + 3].pos = ((span * 3.0).mul_add(inv, 0.5) + tb) as i32;
}

/// Beats `lo + 1 ..= hi` each moved to the middle of itself and the beat before (from the top).
fn halve_back(list: &mut [BeatInfo], hi: usize, lo: usize) {
    if hi <= lo {
        return;
    }
    let mut j = hi;
    let mut old = list[hi].pos;
    loop {
        j -= 1;
        let x = list[j].pos;
        list[j + 1].pos = ((x + old) as f64).mul_add(0.5, 0.5) as i32;
        old = x;
        if j <= lo {
            break;
        }
    }
}

/// The half-beat correction (decompile ~19505-19915): for each pair of candidates the beats
/// from the first bar to the second are moved back half a beat and one beat is added after
/// the second; a last single candidate moves every beat after it and adds one at the end.
/// Only the 4.5-beat bars are built (a 3.5-beat bar: not seen on the corpus).
fn half_beat_fix(list: &mut Vec<BeatInfo>, cand: &mut Vec<(i32, i32)>, total: i32) -> Result<(), &'static str> {
    let cnt = cand.len();
    for i in 0..cnt / 2 {
        let (a, k1) = cand[2 * i];
        let (b, k2) = cand[2 * i + 1];
        let (a, b) = (a as usize, b as usize);
        respace_from_start(list, a, k1);
        let span2 = (list[b + 4].pos - list[b].pos) as f64;
        let e = list[b + 4].pos as f64 + 0.5;
        list[b + 4].pos = e as i32;
        let te = e.trunc();
        let inv2 = 1.0 / (k2 as f64 + 0.5);
        list[b + 3].pos = ((-span2).mul_add(inv2, 0.5) + te) as i32;
        list[b + 2].pos = ((span2 * -2.0).mul_add(inv2, 0.5) + te) as i32;
        list[b + 1].pos = ((span2 * -3.0).mul_add(inv2, 0.5) + te) as i32;
        if k1 == 3 {
            return Err(UNPORTED_HALFBEAT);
        }
        if k1 == 4 {
            if k2 == 3 {
                return Err(UNPORTED_HALFBEAT);
            }
            if k2 == 4 {
                halve_back(list, b, a + 4);
                let d = (list[a + 4].pos - list[a + 3].pos) as f64;
                list[a + 4].pos -= d.mul_add(THIRD, 0.5) as i32;
                let p = ((list[b].pos + list[b + 1].pos) as f64).mul_add(0.5, 0.5) as i32;
                if list.len() >= MAX_BEATS { return Err(INVALID_LIST); }
                list.insert(b + 1, blank(p));
                if cnt as i64 - (2 * i) as i64 >= 3 && 2 * i + 2 < cnt {
                    for c in cand.iter_mut().skip(2 * i + 2) {
                        c.0 += 1;
                    }
                }
            }
        }
    }
    if cnt & 1 == 1 {
        let (a, k) = cand[cnt - 1];
        let a = a as usize;
        respace_from_start(list, a, k);
        let n = list.len();
        if k == 4 {
            if n - 1 > a + 4 {
                halve_back(list, n - 1, a + 4);
            }
            let d = (list[a + 4].pos - list[a + 3].pos) as f64;
            list[a + 4].pos -= d.mul_add(THIRD, 0.5) as i32;
            if n > 1 {
                let p = list[n - 1].pos - list[n - 2].pos + list[n - 1].pos;
                if p <= total {
                    if list.len() >= MAX_BEATS { return Err(INVALID_LIST); }
                    list.push(blank(p));
                }
            }
        } else if k == 3 {
            return Err(UNPORTED_HALFBEAT);
        }
    }
    Ok(())
}

/// The doubled-tempo test's body (whole stretch at about 2x the reference interval: the network
/// missed every other beat; coverage listing 18001-18576): the stretch is widened over ratios
/// that are not 1 or 2, a beat is put in the middle of each interval, a beat added before the
/// first one when there is room. Returns the list's new length (the original's `v296` from here
/// on). Branches never seen on the corpus (removals, re-fills, the end extension) are not built.
fn class2_whole(list: &mut Vec<BeatInfo>, q: &[f64], start: &mut [i32; 128], end: &mut [i32; 128], k: usize, v416: i32, total: i32) -> Result<i32, &'static str> {
    let fits = |x: f64| (x >= 1.8 || x <= 1.07) && x <= 2.2;
    let off = |x: f64| !fits(x);
    let qi = |i: i32| q[i as usize];
    let mut v301 = start[k];
    let mut v300 = end[k];
    let mut v530 = false;
    // the start side (LABEL_2308 / 2309 / 2310)
    let v527: i32 = 'st: {
        if v301 == 0 {
            break 'st 0;
        }
        if fits(qi(v301 - 1)) {
            break 'st v301;
        }
        if v301 < 2 {
            break 'st 0;
        }
        if fits(qi(v301 - 2)) {
            break 'st v301 - 1;
        }
        if v301 < 3 {
            break 'st 0;
        }
        if fits(qi(v301 - 3)) {
            break 'st v301 - 2;
        }
        if v301 < 4 {
            break 'st 0;
        }
        if off(qi(v301 - 4)) {
            if v301 >= 5 {
                v530 = off(qi(v301 - 5));
                break 'st if v530 { v301 - 5 } else { v301 - 4 };
            }
            break 'st 0;
        }
        v301 - 3
    };
    // LABEL_2310: the start moves forward over off ratios after it
    if v301 <= v416 - 2 && off(qi(v301 + 1)) {
        if v301 > v416 - 3 || fits(qi(v301 + 2)) {
            v301 += 1;
        } else if v301 > v416 - 4 || fits(qi(v301 + 3)) {
            v301 += 2;
        } else if v301 > v416 - 5 || fits(qi(v301 + 4)) {
            v301 += 3;
        } else if v301 <= v416 - 6 {
            let o = off(qi(v301 + 5));
            v301 += if o { 5 } else { 4 };
            v530 |= o;
        } else {
            v301 += 4;
        }
    }
    // the end side
    let v1749: i32 = if v300 < 1 {
        end[k]
    } else {
        let x = qi(v300 - 1);
        let mut deep = false;
        let mut r = 0;
        if x <= 1.07 || x >= 1.8 {
            r = if x > 2.2 { 0 } else { end[k] };
            if x > 2.2 && v300 >= 2 {
                deep = true;
            }
        } else if v300 < 2 {
            r = 0;
        } else {
            deep = true;
        }
        if deep {
            // LABEL_2336
            r = v300 - 2;
            if fits(qi(v300 - 2)) {
                r = v300 - 1;
            } else if v300 < 3 {
                r = 0;
            } else if off(qi(v300 - 3)) {
                if v300 < 4 {
                    r = 0;
                } else if fits(qi(v300 - 4)) {
                    r = v300 - 3;
                } else if v300 < 5 {
                    r = 0;
                } else {
                    r = v300 - 4;
                    let o = off(qi(v300 - 5));
                    if o {
                        r = v300 - 5;
                    }
                    v530 |= o;
                }
            }
        }
        r
    };
    // LABEL_2362: the end moves forward over off ratios after it
    if v300 <= v416 - 2 && off(qi(v300 + 1)) {
        if v300 > v416 - 3 || fits(qi(v300 + 2)) {
            v300 += 1;
        } else if v300 > v416 - 4 || fits(qi(v300 + 3)) {
            v300 += 2;
        } else if v300 > v416 - 5 || fits(qi(v300 + 4)) {
            v300 += 3;
        } else if v300 > v416 - 6 {
            v300 += 4;
        } else {
            let o = off(qi(v300 + 5));
            v300 += if o { 5 } else { 4 };
            v530 |= o;
        }
    }
    let v1762 = v1749.max(0);
    let len = list.len() as i32;
    let v1768 = v300.min(len - 1);
    if v1768 > v1762 {
        // the original removes the beats between (never seen)
        return Err(UNPORTED_CLASS);
    }
    let mut v1771 = v1768;
    end[k] = v1771;
    let v1772 = v527.max(0);
    let v1773 = if len - 1 >= v301 { v301 } else { len - 1 };
    if v1773 > v1772 {
        return Err(UNPORTED_CLASS);
    }
    let v1783 = v1768 > v1762 || v1771 == len - 1;
    let mut v1784 = v1771 - v1783 as i32;
    if v1784 >= len - 2 {
        v1784 = len - 2;
    }
    start[k] = v1772;
    let v1785 = if v1773 <= v1772 { v1772 } else { v1772 + 1 };
    if v1784 >= v1785 {
        // a beat in the middle of each interval, from the end
        let mut j = v1784;
        loop {
            let a = list[j as usize].pos;
            let b = list[(j + 1) as usize].pos;
            let p = ((b - a) as f64).mul_add(0.5, a as f64 + 0.5) as i32;
            if list.len() >= MAX_BEATS { return Err(INVALID_LIST); }
            list.insert((j + 1) as usize, blank(p));
            end[k] += 1;
            v1771 = end[k];
            if j <= v1785 {
                break;
            }
            j -= 1;
        }
    }
    // a beat before the first one when it fits
    let v1812 = list[0].pos;
    let v1814 = v1812 - (list[1].pos - v1812);
    if v1814 > 0 {
        if list.len() >= MAX_BEATS { return Err(INVALID_LIST); }
        list.insert(0, blank(v1814));
        end[k] = v1771 + 1;
    }
    let n = list.len();
    if n >= 2 {
        let last = list[n - 1].pos;
        let prev = list[n - 2].pos;
        if last != prev && ((total - last) as f32) / ((last - prev) as f32) > 1.0 {
            return Err(UNPORTED_CLASS);
        }
    }
    if v530 {
        return Err(UNPORTED_CLASS);
    }
    Ok(n as i32)
}

/// The tempo-change half of analyzeFocus on `list` (built by the caller as the constant half
/// does). `sr` = the Analyzer's rate (+20), `a7` = the network's bpm x 100 (`(int)get_bpm`),
/// `bpm72` = +72 on entry.
pub fn analyze(mut list: Vec<BeatInfo>, sr: i32, a7: i32, bpm72: f32, total: i32) -> Result<TempoFocus, &'static str> {
    if list.len() < 4 { return Err(UNPORTED_SHORT); }
    if !matches!(sr, 44_100 | 48_000) || total <= 0 || total > 900 * sr ||
        !(1..=100_000).contains(&a7) || !(1.0..=1000.0).contains(&bpm72) ||
        list.len() > MAX_BEATS || list.iter().any(|b| !(0..=total).contains(&b.pos)) ||
        list.windows(2).any(|b| b[0].pos >= b[1].pos) { return Err(INVALID_LIST); }
    let mut bpm72 = bpm72;
    let v110 = 60.0 / a7 as f64;
    let v3872 = a7 as f64 * 0.0125;
    let v3873 = a7 as f64 * 0.008;
    let v3871 = (a7 as f32) * 0.01f32;
    let v3870 = v110 * 100000.0;
    let mut v3918: i32 = -1;
    let mut v3880: i32 = 0;
    let mut first_pass = true;
    loop {
        let v3910 = first_pass;
        let n = list.len();
        if n == 0 {
            break;
        }
        let ni = n as i32;
        // LABEL_68: times in whole ms (pairs as pos * (1000 / sr), the rest (pos * 1000) / sr)
        let v120 = sr as f64;
        let mut t = vec![0f64; n];
        let v121 = if n > 2 { n - if n & 1 == 0 { 2 } else { 1 } } else { 0 };
        let k = 1000.0 / v120;
        let inv = 1.0 / v120;
        for i in 0..n {
            let p = list[i].pos as f64;
            t[i] = if i < v121 { (p * k).round() * 0.001 } else { (p * 1000.0 * inv).round() * 0.001 };
        }
        // LABEL_76: intervals (ms) and their changes
        let v133: i32 = if ni <= 21 { if ni <= 1 { 0 } else { ni - 2 } } else { 20 };
        let mut d = vec![0f64; n];
        let v136 = ni - 1;
        for i in 0..(n - 1) {
            d[i] = (t[i + 1] - t[i]).mul_add(1000.0, 0.5).trunc();
        }
        let v152_0: i64 = if ni >= 37 { 37 } else { ni as i64 };
        let v153 = ni - v133;
        let v154 = ni - v133 - 1;
        let mut c = vec![0f64; n];
        if v153 <= 1 {
            return Err(UNPORTED_SHORT);
        }
        for i in 0..v154 as usize {
            c[i] = (d[i + 1] - d[i]).abs();
        }
        // the first steady run (11, 20, 30 ms), else the first min(n, 37) intervals
        let cnt = v154 as usize;
        let find = |thr: f64| -> Option<(i64, f64)> {
            let (mut i, mut run, mut sum) = (0usize, 0i64, 0f64);
            loop {
                while c[i] <= thr {
                    sum = d[i] + sum;
                    run += 1;
                    i += 1;
                    if i == cnt {
                        return None;
                    }
                }
                if run >= v152_0 {
                    return Some((run, sum));
                }
                run = 0;
                sum = 0.0;
                i += 1;
                if i == cnt {
                    return None;
                }
            }
        };
        let v189 = match find(11.0).or_else(|| find(20.0)).or_else(|| find(30.0)) {
            Some((run, sum)) => sum / run as f64,
            None => {
                // LABEL_144: the first min(n, 37) intervals (whole numbers: any order is exact)
                let m = v152_0.max(1) as usize;
                let mut s = 0f64;
                for x in d.iter().take(m) {
                    s = x + s;
                }
                s / v152_0 as f64
            }
        };
        // LABEL_185: the reference interval, the network's tempo when they disagree
        let q60 = 60000.0 / v189;
        let v221 = q60 < v3873 || q60 > v3872;
        bpm72 = if v221 { v3871 } else { q60 as f32 };
        let v223 = if v221 { v3870 } else { v189 };
        // ratios to the reference and their distance from 1
        let mut r = vec![0f64; n];
        let mut q = vec![0f64; n];
        let ir = 1.0 / v223;
        for i in 0..v136 as usize {
            let x = d[i] * ir;
            r[i] = x;
            q[i] = if x.abs() > 2.22044605e-16 {
                if x >= 1.0 {
                    x
                } else {
                    1.0 / x
                }
            } else {
                0.0
            };
        }
        if n >= 2 {
            r[n - 1] = r[n - 2];
            q[n - 1] = q[n - 2];
        } else {
            return Err(UNPORTED_SHORT);
        }
        // irregular stretches: two ratios off by more than 18 % in a row, up to 128
        let mut start = [0i32; 128];
        let mut end = [0i32; 128];
        {
            let v243 = n as i64;
            let (mut v239, mut v240) = (0i64, 0usize);
            loop {
                if q[v239 as usize] > 1.18 {
                    let mut v244 = v239 + 1;
                    if q[v244 as usize] > 1.18 {
                        start[v240] = v239 as i32;
                        if v243 > v244 && (ni as i64) > v244 {
                            let mut v245 = v239 + 2;
                            loop {
                                v244 = v245;
                                if v243 <= v245 {
                                    break;
                                }
                                let v246 = q[v245 as usize];
                                v245 += 1;
                                if !(v246 > 1.18 && v244 < ni as i64) {
                                    break;
                                }
                            }
                        }
                        end[v240] = (v244 - 1) as i32;
                        v240 += 1;
                        v239 = v244;
                    }
                }
                v239 += 1;
                if !(v239 < v136 as i64 && v240 < 128) {
                    break;
                }
            }
        }
        // LABEL_221
        let mut v248 = 0usize;
        while end[v248] > 0 {
            v248 += 1;
            if v248 == 128 {
                break;
            }
        }
        if v3918 < 0 {
            // the middle of the longest regular stretch becomes the reference beat
            let (mut span_end, v271): (i32, i32);
            if v248 == 0 {
                span_end = v136;
                v271 = 0;
            } else {
                let v250 = if v248 == 128 { 127 } else { v248 - 1 };
                let mut v251 = if start[0] <= 0 { 0 } else { start[0] };
                let mut v252 = if start[0] <= 0 { v136 } else { start[0] - 1 };
                let mut v253 = 0i32;
                if v248 >= 2 {
                    for j in 0..v250 {
                        let v257 = start[j + 1];
                        let v259 = end[j];
                        let v262 = v257 - v259;
                        let v263 = v251 < v262;
                        if v251 <= v262 {
                            v251 = v262;
                        }
                        if v263 {
                            v252 = v257 - 1;
                            v253 = v259 + 1;
                        }
                    }
                }
                let v270 = end[v250];
                if v136 > v270 && v251 < v136 - v270 {
                    span_end = v136;
                    v271 = v270 + 1;
                } else {
                    span_end = v252;
                    v271 = v253;
                }
            }
            // (the original indexes half the span's length, not its middle)
            let idx = ((span_end - v271) / 2) as usize;
            v3918 = list[idx].pos;
            v3880 = list[idx].num;
            span_end = 0;
            let _ = span_end;
        }
        // LABEL_254: per stretch, where its ratios sit near 1/2, 2/3, 3/2 and 2 (first, last)
        let mut h_lo = [-1i32; 128];
        let mut h_hi = [-1i32; 128];
        let mut t_lo = [-1i32; 128];
        let mut t_hi = [-1i32; 128];
        let mut p_lo = [-1i32; 128];
        let mut p_hi = [-1i32; 128];
        let mut w_lo = [-1i32; 128];
        let mut w_hi = [-1i32; 128];
        for k in 0..v248 {
            let mut v275 = start[k];
            let v276 = end[k];
            if v275 > v276 {
                continue;
            }
            let mut v277 = h_lo[k];
            let mut v278 = v275 - 1;
            loop {
                let cur = r[(v278 + 1) as usize];
                // the ratio before the stretch's first one may be read at index -1: the
                // original then reads the double before the array (never 0.4425..0.5575 here)
                let prev = if v278 >= 0 { r[v278 as usize] } else { 0.0 };
                if cur >= 0.5575 || cur <= 0.4425 {
                    let prev_half = prev < 0.5575 && prev > 0.4425;
                    if v277 < 0 || h_hi[k] != -1 || !prev_half {
                        let mut v283 = t_lo[k];
                        if cur >= 0.7433333333333333 || cur <= 0.59 {
                            let prev_t = prev < 0.7433333333333333 && prev > 0.59;
                            if v283 < 0 || t_hi[k] != -1 || !prev_t {
                                let mut v286 = p_lo[k];
                                if cur >= 1.6724999999999999 || cur <= 1.3275000000000001 {
                                    if v286 < 0 || p_hi[k] != -1 || prev >= 1.6724999999999999 || prev <= 1.3275000000000001 {
                                        let mut v288 = w_lo[k];
                                        if cur >= 2.23 || cur <= 1.77 {
                                            if v288 >= 0 && w_hi[k] == -1 && prev < 2.23 && prev > 1.77 {
                                                w_hi[k] = v278;
                                            }
                                        } else {
                                            if v288 == -1 {
                                                v288 = v278 + 1;
                                            }
                                            w_lo[k] = v288;
                                            if v275 >= v276 {
                                                w_hi[k] = if w_hi[k] != -1 { w_hi[k] } else { v278 + 1 };
                                            }
                                        }
                                    } else {
                                        p_hi[k] = v278;
                                    }
                                } else {
                                    if v286 == -1 {
                                        v286 = v278 + 1;
                                    }
                                    p_lo[k] = v286;
                                    if v275 >= v276 {
                                        p_hi[k] = if p_hi[k] != -1 { p_hi[k] } else { v278 + 1 };
                                    }
                                }
                            } else {
                                t_hi[k] = v278;
                            }
                        } else {
                            if v283 == -1 {
                                v283 = v278 + 1;
                            }
                            t_lo[k] = v283;
                            if v275 >= v276 {
                                t_hi[k] = if t_hi[k] != -1 { t_hi[k] } else { v278 + 1 };
                            }
                        }
                    } else {
                        h_hi[k] = v278;
                    }
                } else {
                    if v277 == -1 {
                        v277 = v278 + 1;
                    }
                    if v275 >= v276 {
                        h_hi[k] = if h_hi[k] != -1 { h_hi[k] } else { v278 + 1 };
                    }
                }
                v275 += 1;
                v278 += 1;
                if v276 == v278 {
                    break;
                }
            }
            h_lo[k] = v277;
        }
        // the stretches, last first
        if v248 >= 1 {
            let v297 = 1.0 / v223;
            let mut v296 = ni;
            let mut v298 = v248;
            loop {
                let v299 = v298;
                v298 -= 1;
                let k = v298;
                let v300 = end[k];
                let v301 = start[k];
                let v302 = v300 - v301;
                'seg: {
                    if v302 < 1 {
                        break 'seg;
                    }
                    if v302 < 8 {
                        // mean distance from 1 inside, and of up to 8 ratios on each side
                        let mut v304 = 0f64;
                        if v300 >= v301 {
                            let sl = &q[v301 as usize..=v300 as usize];
                            v304 = if v302 < 7 {
                                let mut s = 0f64;
                                for x in sl {
                                    s = x + s;
                                }
                                s
                            } else {
                                sum8(sl)
                            };
                        }
                        let v504 = if v301 >= 8 { 8 } else { start[k] };
                        let mut v303 = 0f64;
                        if v301 >= 1 {
                            let v505 = v504.max(1);
                            v303 = q[(v301 - 1) as usize];
                            for j in 2..=v505 {
                                v303 = q[(v301 - j) as usize] + v303;
                            }
                        }
                        let v506 = v296 - 1 - v300;
                        let v507 = v506.min(8);
                        if v506 >= 1 {
                            let v508 = v507.max(1);
                            v303 = q[(v300 + 1) as usize] + v303;
                            if v507 >= 2 {
                                for j in 2..=v508 {
                                    v303 = q[(v300 + j) as usize] + v303;
                                }
                            }
                        }
                        let v509 = v304 / (v302 + 1) as f64;
                        if v504 + v507 != 0 {
                            v303 /= (v504 + v507) as f64;
                        }
                        if v509 - v303 <= 0.16 {
                            break 'seg;
                        }
                        // widen over up to 3 off intervals before and after
                        let mut v511 = start[k];
                        if v301 >= 1 && q[(v301 - 1) as usize] > 1.07 {
                            start[k] = v301 - 1;
                            if v301 < 2 {
                                v511 = 0;
                            } else {
                                v511 = v301 - 1;
                                if q[(v301 - 2) as usize] > 1.07 {
                                    start[k] = v301 - 2;
                                    v511 = v301 - 2;
                                    if v301 - 1 >= 2 && q[(v301 - 3) as usize] > 1.07 {
                                        start[k] = v301 - 3;
                                        v511 = v301 - 3;
                                    }
                                }
                            }
                        } else if v301 < 1 {
                            v511 = start[k];
                        }
                        let n2 = v296 - 2;
                        let v525: i32 = if v300 >= n2 {
                            // LABEL_659
                            if v300 <= n2 {
                                v300
                            } else {
                                end[k] = n2;
                                n2
                            }
                        } else {
                            // LABEL_690
                            let mut e = v300;
                            if q[(v300 + 1) as usize] > 1.07 {
                                end[k] = v300 + 1;
                                e = v300 + 1;
                                if v300 + 1 < n2 && q[(v300 + 2) as usize] > 1.07 {
                                    end[k] = v300 + 2;
                                    e = v300 + 2;
                                    if v300 + 2 < n2 && q[(v300 + 3) as usize] > 1.07 {
                                        // LABEL_660
                                        end[k] = v300 + 3;
                                        e = v300 + 3;
                                    }
                                }
                            }
                            e
                        };
                        // LABEL_699: re-fill when the stretch spans a whole number of beats
                        let v537 = t[v511 as usize];
                        let v538 = t[(v525 + 1) as usize] - v537;
                        let v539 = v538 * 1000.0 * v297;
                        let v540 = (v539 + 0.5) as i32;
                        let v541 = (v539 + 0.5).trunc();
                        let v542 = (v539 - v541).abs();
                        if v540 >= 2 && v542 < 0.25 {
                            if v511 != v525 {
                                list.drain((v511 + 1) as usize..=(v525) as usize);
                            }
                            let v552 = 1.0 / v541;
                            let mut at = (v511 + 1) as usize;
                            for j in 1..v540 {
                                let p = (v538 * j as f64).mul_add(v552, v537).mul_add(sr as f64, 0.5) as i32;
                                if list.len() >= MAX_BEATS { return Err(INVALID_LIST); }
                                list.insert(at, blank(p));
                                at += 1;
                            }
                            v3918 = list[0].pos;
                            v3880 = list[0].num;
                        }
                        break 'seg;
                    }
                    // a long stretch: mean ratio and its variance
                    let seg = &r[v301 as usize..=v300 as usize];
                    let v307 = (v302 + 1) as f64;
                    let v308 = sum8(seg) / v307;
                    let v309 = sq8(seg, v308);
                    let at = |i: i32| r[i as usize];
                    let v346 = v301 + 1;
                    let mut v347 = 0f64;
                    let mut v348 = 0f64;
                    if v301 + 1 < v296 && v300 > 0 {
                        v348 = (at(v300 - 1) - at(v346)).abs() / (v302 - 2) as f64;
                    }
                    let v349 = v302 - 3;
                    let mut v350 = 0f64;
                    if v301 + 2 < v296 && v300 >= 2 {
                        v350 = (at(v300 - 2) - at(v346)).abs() / v349 as f64;
                        v347 = (at(v300 - 1) - at(v301 + 2)).abs() / v349 as f64;
                    }
                    let v436 = v309 / v307;
                    let v437 = (v347 + v348 + v350) * THIRD;
                    // the four ratio classes inside the stretch: mean, variance, end differences
                    let class = |lo: i32, hi: i32, guard127: bool| -> (f64, f64, f64, f64) {
                        // (variance, d1, d2a, d2b) as v354/v353/v370/v369 for the half class
                        let len = r.len() as i64;
                        let span = hi - lo;
                        let mut var = 0f64;
                        if hi >= lo {
                            let cnt = (hi + 1 - lo) as f64;
                            let ok = |i: i32| if guard127 { ((i - 1) as u32) <= 0x7e } else { i >= 0 && len > i as i64 };
                            let mut s = 0f64;
                            for i in lo..=hi {
                                if ok(i) {
                                    s = r[i as usize] + s;
                                }
                            }
                            let m = s / cnt;
                            let mut v = 0f64;
                            for i in lo..=hi {
                                if ok(i) {
                                    let e = r[i as usize] - m;
                                    v = e.mul_add(e, v);
                                }
                            }
                            var = v / cnt;
                        }
                        let mut d1 = 0f64;
                        if span >= 3 && lo + 1 < v296 && hi >= 1 {
                            d1 = (r[(hi - 1) as usize] - r[(lo + 1) as usize]).abs() / (span - 2) as f64;
                        }
                        let (mut d2a, mut d2b) = (0f64, 0f64);
                        if span > 3 && lo + 2 < v296 && hi >= 2 {
                            d2a = (r[(hi - 2) as usize] - r[(lo + 1) as usize]).abs() / (span - 3) as f64;
                            d2b = (r[(hi - 1) as usize] - r[(lo + 2) as usize]).abs() / (span - 3) as f64;
                        }
                        (var, d1, d2a, d2b)
                    };
                    let (v354, v353, v370, v369) = class(h_lo[k], h_hi[k], true);
                    let (v368, v386, v388, v385) = class(t_lo[k], t_hi[k], false);
                    let (v393, v392, v414, v413) = class(p_lo[k], p_hi[k], false);
                    let (v412, v432, v431, v434) = class(w_lo[k], w_hi[k], false);
                    // the 1.5 class sums with a stop at the array's end (`v398`): the same values
                    // here as the guard above (indices are inside the array)
                    let v355 = h_hi[k] - h_lo[k];
                    let v103 = t_hi[k] - t_lo[k];
                    let v394 = p_hi[k] - p_lo[k];
                    let v419 = w_hi[k] - w_lo[k];
                    // the eight tests, in order (each body is a branch not ported yet)
                    if v308 < 0.515 && v308 > 0.485 && v437 < 0.008 && v301 >= 0 && v436 < 0.015 {
                        return Err(UNPORTED_CLASS);
                    }
                    if (v369 + v353 + v370) * THIRD < 0.008 && h_lo[k] >= 0 && v355 >= 8 && v354 < 0.015 {
                        return Err(UNPORTED_CLASS);
                    }
                    if v308 < 0.6866666666666666 && v308 > 0.6466666666666666 && v437 < 0.008 && v301 >= 0 && v436 < 0.015 {
                        return Err(UNPORTED_CLASS);
                    }
                    if (v385 + v386 + v388) * THIRD < 0.008 && t_lo[k] >= 0 && v103 >= 8 && v368 < 0.015 {
                        return Err(UNPORTED_CLASS);
                    }
                    if v308 < 1.545 && v308 > 1.455 && v437 < 0.008 && v301 >= 0 && v436 < 0.015 {
                        return Err(UNPORTED_CLASS);
                    }
                    if (v413 + v392 + v414) * THIRD < 0.008 && p_lo[k] >= 0 && v394 >= 8 && v393 < 0.015 {
                        return Err(UNPORTED_CLASS);
                    }
                    if v308 < 2.06 && v308 > 1.94 && v437 < 0.008 && v301 >= 0 && v436 < 0.015 {
                        // LABEL_2670: the list's new length counts for the stretches before
                        v296 = class2_whole(&mut list, &q, &mut start, &mut end, k, v296, total)?;
                        break 'seg;
                    }
                    if !((v431 + v432 + v434) * THIRD >= 0.008 || w_lo[k] < 0 || v419 < 8) && v412 < 0.015 {
                        return Err(UNPORTED_CLASS);
                    }
                    let _ = v412;
                }
                // LABEL_331
                if v299 <= 1 {
                    break;
                }
            }
        }
        first_pass = false;
        if !v3910 {
            break;
        }
    }
    // the final stage, twice: single long intervals split, the half-beat check
    let mut again = true;
    loop {
        let v1986 = again;
        let n = list.len() as i32;
        if n <= 2 {
            return Err(UNPORTED_SHORT);
        }
        let v1988 = n - 1;
        let v1989 = n - 2;
        let mut v1991 = 1i32;
        loop {
            let i = v1991 as usize;
            let v1994 = list[i - 1].pos;
            let v1995 = list[i].pos;
            let v1996 = v1991 + 1;
            if v1991 >= v1989 {
                if v1995 != v1994 {
                    let v2011 = (list[i + 1].pos - v1995) as f64;
                    let v2012 = v2011 / (v1995 - v1994) as f64;
                    let v2013 = (v2012 + 0.5) as i32;
                    if v2013 >= 1 && v2012 > 1.8 && v2013 >= 2 {
                        let step = (v2011 / v2013 as f64 + 0.5) as i32;
                        for m in 1..v2013 {
                            let p = list[i].pos + step * m;
                            if list.len() >= MAX_BEATS { return Err(INVALID_LIST); }
                            list.insert(i + m as usize, blank(p));
                        }
                    }
                }
            } else {
                let v1997 = list[i + 1].pos;
                let mut v1998 = 0f64;
                let mut v1999 = 0f64;
                let v2000 = v1995 - v1994;
                if v2000 != 0 {
                    v1999 = (v1997 - v1995) as f64 / v2000 as f64;
                }
                let v2001 = list[i + 2].pos - v1997;
                if v2001 != 0 {
                    v1998 = (v1997 - v1995) as f64 / v2001 as f64;
                }
                let v2002 = (v1999 + 0.5) as i32;
                if v2002 >= 1 && (v1998 + 0.5) as i32 >= 1 && v1999 > 1.8 && v1998 > 1.8 && v2002 >= 2 {
                    let step = ((v1997 - v1995) as f64 / v2002 as f64 + 0.5) as i32;
                    for m in 1..v2002 {
                        let p = list[i].pos + step * m;
                        if list.len() >= MAX_BEATS { return Err(INVALID_LIST); }
                        list.insert(i + m as usize, blank(p));
                    }
                }
            }
            v1991 = v1996;
            if v1996 == v1988 {
                break;
            }
        }
        // bars half a beat too long (more than 64 beats): candidates (index m, 4 x bar / the
        // bar before), handled in pairs (where it starts, where it ends) and a last single one
        let n = list.len() as i32;
        if n > 64 {
            let (mut v2024, mut v2025) = (false, 0i32);
            let mut cand: Vec<(i32, i32)> = vec![];
            let mut m = 32i32;
            loop {
                // bars: the four beats from m against the four before and after
                let b = (m + 4) as usize;
                let v2029 = list[m as usize].pos;
                let v2030 = list[b].pos - v2029;
                let mut v2031 = 0f64;
                let mut v2032 = 0f64;
                let v2033 = v2029 - list[(m - 4) as usize].pos;
                if v2033 != 0 {
                    v2032 = v2030 as f64 * 4.0 / v2033 as f64;
                }
                let v2034 = list[(m + 8) as usize].pos - list[b].pos;
                if v2034 != 0 {
                    v2031 = v2030 as f64 * 4.0 / v2034 as f64;
                }
                let v2035 = v2032 - v2032.trunc();
                let v2036 = v2031 - v2031.trunc();
                if v2035 > 0.4 && v2035 < 0.6 && v2036 > 0.4 && v2036 < 0.6 {
                    let v2041 = (v2032 as i32) != (v2031 as i32) || v2025 < 5;
                    if v2041 || v2024 {
                        v2024 = !v2041;
                    } else {
                        if cand.len() > 126 {
                            return Err(UNPORTED_HALFBEAT);
                        }
                        cand.push((m, v2032 as i32));
                        v2025 = 0;
                        v2024 = true;
                    }
                } else {
                    v2024 = false;
                }
                v2025 += 1;
                m += 1;
                if n - 32 == m {
                    break;
                }
            }
            if !cand.is_empty() {
                half_beat_fix(&mut list, &mut cand, total)?;
            }
        }
        again = false;
        if !v1986 {
            break;
        }
    }
    // the tempo: median of the per-beat tempi (the last interval counted twice)
    let n = list.len();
    let srf = sr as f32;
    let mut tempi: Vec<f32> = Vec::with_capacity(n);
    if n >= 2 {
        for i in 0..n - 1 {
            let dd = list[i + 1].pos - list[i].pos;
            if dd != 0 {
                tempi.push((srf * 60.0f32) / dd as f32);
            }
        }
        let dd = list[n - 1].pos - list[n - 2].pos;
        if dd != 0 {
            tempi.push((srf * 60.0f32) / dd as f32);
        }
        if tempi.len() < n {
            return Err(UNPORTED_SHORT);
        }
        tempi.sort_unstable_by(f32::total_cmp);
        bpm72 = if n & 1 == 1 { tempi[n >> 1] } else { (tempi[n >> 1] + tempi[(n >> 1) - 1]) * 0.5f32 };
    }
    // the beat nearest the reference, bar numbers from its number
    let ni = n as i32;
    let mut v2208: i32 = ni - 1;
    if ni <= 1 {
        v2208 = 0;
    } else {
        let mut dprev = (list[1].pos - v3918).unsigned_abs();
        let d0 = (list[0].pos - v3918).unsigned_abs();
        if d0 <= dprev {
            v2208 = 0;
        } else {
            let mut kk = 0i32;
            while ni - 2 != kk {
                let dj = (list[(kk + 2) as usize].pos - v3918).unsigned_abs();
                kk += 1;
                let closer = dprev > dj;
                dprev = dj;
                if !closer {
                    v2208 = kk;
                    break;
                }
            }
        }
        if ni >= 5 {
            // a bar numbering that breaks where a minute starts sends the reference to beat 0
            let mut v2272 = list[2].pos;
            let mut v2274 = 2i32;
            loop {
                let j = v2274 as usize;
                let v2275 = v2272 / sr / 60;
                v2272 = list[j + 1].pos;
                if v2275 != v2272 / sr / 60 {
                    let v2276 = list[j - 1].num;
                    let v2277 = v2276 - list[j - 2].num;
                    let v2278 = list[j].num;
                    let v2279 = list[j + 1].num;
                    let ok = |x: i32| x == 1 || x == -3;
                    if !ok(v2277) || !ok(v2278 - v2276) || !ok(v2279 - v2278) || !ok(list[j + 2].num - v2279) {
                        v2208 = 0;
                        v3880 = list[0].num;
                    }
                }
                v2274 += 1;
                if ni - 2 == v2274 {
                    break;
                }
            }
        }
    }
    for (i, b) in list.iter_mut().enumerate() {
        let x = v3880 - v2208 + i as i32 - 1;
        let m = x % 4;
        b.num = m + if m < 0 { 4 } else { 0 } + 1;
    }
    // each beat's interval from 8 beats on each side (ms)
    for i in 0..ni {
        let lo = i.max(8) - 8;
        let hi = if i + 8 >= ni { ni - 1 } else { i + 8 };
        if hi != lo {
            list[i as usize].interval = ((list[hi as usize].pos - list[lo as usize].pos) as f32 * 1000.0f32) / (srf * (hi - lo) as f32);
        }
    }
    // the first downbeat, then the mean interval per bar
    let (v2305, v2306): (i32, bool) = if list[0].num == 1 {
        (0, true)
    } else if list[1].num == 1 {
        (1, false)
    } else if list[2].num == 1 {
        (2, false)
    } else {
        let one = list[3].num == 1;
        (if one { 3 } else { 0 }, !one)
    };
    let mut v2774 = 0i32;
    if v2305 < ni - 4 {
        v2774 = v2305;
        loop {
            let v2775 = v2774;
            v2774 += 4;
            let v2776 = if v2774 <= ni { 4 } else { ni - v2775 };
            if v2776 >= 1 {
                let mut s = 0f32;
                for j in 0..v2776 {
                    s = list[(v2775 + j) as usize].interval + s;
                }
                let m = s / v2776 as f32;
                for j in 0..v2776 {
                    list[(v2775 + j) as usize].interval = m;
                }
            }
            if v2774 >= ni - 4 {
                break;
            }
        }
    }
    if !v2306 {
        let v = list[v2305 as usize].interval;
        for b in list.iter_mut().take(v2305 as usize) {
            b.interval = v;
        }
    }
    if v2774 >= 1 && v2774 < ni {
        let v = list[(v2774 - 1) as usize].interval;
        for b in list.iter_mut().skip(v2774 as usize) {
            b.interval = v;
        }
    }
    Ok(TempoFocus { list, bpm72 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network_list(ramp: bool) -> (Vec<BeatInfo>, i32) {
        let mut seconds = 0.1;
        let mut list = Vec::new();
        for i in 0..240 {
            let bpm = if ramp { 118.0 + 8.0 * f64::from(i) / 239.0 } else { 128.0 };
            list.push(BeatInfo { pos: (seconds * 44100.0_f64).round() as i32, zero: 0,
                num: i % 4 + 1, interval: (60_000.0 / bpm) as f32, minus1: -1, flag: false });
            seconds += 60.0 / bpm;
        }
        (list, (seconds * 44100.0) as i32)
    }

    #[test]
    fn dynamic_focus_preserves_a_gradual_tempo_change() {
        let (list, total) = network_list(true);
        let positions: Vec<_> = list.iter().map(|b| b.pos).collect();
        let out = analyze(list, 44100, 12200, 128.0, total).unwrap();
        assert_eq!(out.list.iter().map(|b| b.pos).collect::<Vec<_>>(), positions);
        assert!(out.list.iter().enumerate().all(|(i, b)| b.num == (i % 4) as i32 + 1));
        assert!((out.bpm72 - 122.00014).abs() < 0.001);
        assert!((out.list[0].interval - 507.8630).abs() < 0.01);
        assert!((out.list[239].interval - 477.1097).abs() < 0.01);
    }

    #[test]
    fn dynamic_focus_repairs_one_skipped_pulse_and_rejects_invalid_input() {
        let (mut list, total) = network_list(false);
        let gap = list.remove(96);
        let out = analyze(list, 44100, 12800, 128.0, total).unwrap();
        assert_eq!(out.list.len(), 240);
        assert!((out.list[96].pos - gap.pos).abs() <= 1);
        assert_eq!(out.list[96].num, gap.num);
        assert_eq!(analyze(out.list[..3].to_vec(), 44100, 12800, 128.0, total).err(), Some(UNPORTED_SHORT));
        let mut repeated = out.list;
        repeated[2].pos = repeated[1].pos;
        assert_eq!(analyze(repeated, 44100, 12800, 128.0, total).err(), Some(INVALID_LIST));
    }
}
