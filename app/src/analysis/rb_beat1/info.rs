//! Stage 2 of rekordbox's v1 beat library: `peakss::BeatInfoAnalyzer` (tempo and the coarse
//! beat grid from the sensor's candidate list).
//!
//! Ported from rekordbox 7.2.19 (arm64): `analyze` 0x10118ff2c, `segAdjust` 0x1011944ec,
//! `findNearPos` 0x10118fd74, `gapCorrect(vector&, int)` 0x101194b00 and
//! `gapCorrect(vector&, int, int, int)` 0x101194e7c. The Hex-Rays output of `analyze` is wrong
//! in places (stale lengths, signed where the code compares unsigned); this follows the
//! machine code where they differ.

use super::sensor::tol;
use super::BeatInfo;

/// Band latency offsets in ms (`strBandOffset`, 0x1037c86a0): the same for every segment.
pub(super) const BAND_OFFSET_MS: [f64; 6] = [-30.0, -20.0, -3.0, -3.0, -20.0, -3.0];

/// `a % b` as AArch64 computes it (`sdiv` by zero gives 0, so the remainder is `a`).
#[inline]
fn rem(a: i32, b: i32) -> i32 {
    if b == 0 { a } else { a.wrapping_rem(b) }
}

#[inline]
fn bin1(c: f64, iv: f32) -> i32 {
    ((c / (iv as f64)) + 5.0) as i32
}

/// Index of the last entry with `pos <= a`, or -1.
#[inline]
fn upper_idx(v: &[BeatInfo], a: i32) -> i32 {
    let mut lo: i32 = 0;
    let mut hi: i32 = v.len() as i32 - 1;
    while hi >= lo {
        let mid = lo + (hi - lo) / 2;
        if v[mid as usize].pos <= a {
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    hi
}

/// `findNearPos` (0x10118fd74): the candidate nearest to `a` within `b` samples, scanning from
/// the last one at or before `a` and stopping after the first one past it. `None` when the list
/// is empty (the original then leaves its out-parameter alone), else the index or -1.
pub(super) fn find_near(v: &[BeatInfo], a: i32, b: i32) -> Option<i32> {
    let n = v.len() as i32;
    if n == 0 {
        return None;
    }
    let mut lo = upper_idx(v, a);
    if lo == -1 {
        lo = 0;
    }
    let hi_raw = upper_idx(v, b.wrapping_add(a));
    let (mut i, hi) = if hi_raw == -1 {
        let i = lo.max(0);
        if i > 0 {
            return Some(-1);
        }
        (i, 0)
    } else {
        let i = lo.max(0);
        let hi = if hi_raw >= 0 { hi_raw } else { n - 1 };
        if i > hi {
            return Some(-1);
        }
        (i, hi)
    };
    let mut best = i32::MAX;
    let mut found = -1;
    loop {
        let stop = i.max(hi) + 1;
        let d;
        loop {
            let p = v[i as usize].pos;
            let dd = (a.wrapping_sub(p)).wrapping_abs();
            if dd <= b {
                d = dd;
                break;
            }
            i += 1;
            if i == stop {
                return Some(found);
            }
        }
        if d < best {
            found = i;
        }
        if v[i as usize].pos > a {
            break;
        }
        if d < best {
            best = d;
        }
        let cont = hi > i;
        i += 1;
        if !cont {
            break;
        }
    }
    Some(found)
}

pub(super) struct Bia {
    pub sr: i32,
}

impl Bia {
    /// `BeatInfoAnalyzer::analyze(info, out, total, 4, 16, 12, varied)`. Returns the BPM; the
    /// candidate `list` is consumed (the original works on a copy).
    pub(super) fn analyze(&self, list: &mut Vec<BeatInfo>, out: &mut Vec<BeatInfo>, total: i32, a5: i32, a6: i32, a7: i32, varied: bool) -> f32 {
        out.clear();
        if list.is_empty() {
            return 0.0;
        }
        let c0 = 1.0f64; // __exp10(0.0)
        let c1 = 10.0f64; // __exp10(1.0)
        let k1 = c0 * 600000.0;
        let k2 = c1 * 600000.0;
        let segs0 = if varied { self.varied_segments(list) } else { self.normal_segment(list, total, k1, k2, c0, c1) };
        let Some(mut segs) = segs0 else { return 0.0 };
        // --- P2 (LABEL_348): tempo per segment, merging neighbours of the same tempo.
        self.seg_tempo(list, &mut segs, total, k1, k2, c0, c1);
        // --- P3: per-segment band offsets are the constant table.
        // --- P4 (LABEL_424): drop candidates closer than an eighth of a beat to the previous one,
        // keeping the one whose update listed more peaks.
        let mut first_cnt: Vec<i32> = vec![];
        for e in list.iter() {
            let c = (e.stat >> 8) as usize;
            if c >= first_cnt.len() {
                first_cnt.resize(c + 1, 0);
            }
            if first_cnt[c] == 0 {
                first_cnt[c] = (e.stat as u8) as i32;
            }
        }
        if !segs.is_empty() && !list.is_empty() {
            // Same decisions as the original's in-place erases, done by compaction (the
            // original's `memmove` per erase is quadratic).
            let src = std::mem::take(list);
            let mut kept: Vec<BeatInfo> = Vec::with_capacity(src.len());
            kept.push(src[0]);
            let mut i = 1usize;
            let mut pstat = src[0].stat;
            let mut ppos = src[0].pos;
            let nseg = segs.len();
            for j in 0..nseg {
                let end = if j == nseg - 1 { total } else { segs[j + 1].pos };
                if i < src.len() {
                    let min_d = (((segs[j].iv * (self.sr as f32)) as f64) * 0.000125) as i32;
                    while i < src.len() {
                        let cur = src[i];
                        if cur.pos >= end {
                            break;
                        }
                        let d = ppos.wrapping_sub(cur.pos).wrapping_abs();
                        if d >= min_d {
                            kept.push(cur);
                            pstat = cur.stat;
                            ppos = cur.pos;
                        } else if first_cnt[(pstat >> 8) as usize] >= first_cnt[(cur.stat >> 8) as usize] {
                            // drop the current one
                        } else {
                            kept.pop();
                            kept.push(cur);
                            ppos = cur.pos;
                            pstat = cur.stat;
                        }
                        i += 1;
                    }
                }
            }
            kept.extend_from_slice(&src[i..]);
            *list = kept;
        }
        // --- P5: BPM of the longest segment.
        let nseg = segs.len();
        if nseg == 0 {
            return 0.0;
        }
        let mut bpm: f32;
        {
            let mut best = if nseg == 1 { total } else { segs[1].pos };
            if best < 1 {
                best = 0;
                bpm = 0.0;
            } else {
                bpm = 60000.0f32 / segs[0].iv;
            }
            for j in 1..nseg {
                let end = if j == nseg - 1 { total } else { segs[j + 1].pos };
                let len = end - segs[j].pos;
                if best < len {
                    best = len;
                    bpm = 60000.0f32 / segs[j].iv;
                }
            }
        }
        // --- P6: phase and tempo fine search per segment.
        self.seg_adjust(list, &mut segs, total);
        // --- P7: beat tracking per segment.
        let v335 = a6 + a5 + a7;
        let mut buf = vec![0i32; (4 * v335 + 1) as usize];
        let srf = self.sr as f64;
        let v340 = srf * 0.001;
        let v669 = 1.0 / srf;
        let v657 = 1.0 / (v335 as f64);
        let v675 = 4 * v335;
        let mut v661: i32 = 0;
        let mut v697: i32 = -1;
        let mut v700: i32 = -1;
        let mut v354: f64 = 0.0;
        let max_off = BAND_OFFSET_MS.iter().fold(f64::from_bits(0xffef_ffff_ffff_ffff), |a, b| a.max(*b));
        let off = |band: i32| -> Option<f64> { ((band as u32) <= 5).then(|| BAND_OFFSET_MS[band as usize]) };
        let sr20 = self.sr / 20;
        for j in 0..nseg {
            let end_i = if j == nseg - 1 { total } else { segs[j + 1].pos };
            let end = end_i as f64;
            let prev_last = out.last().map(|b| b.pos as f64).unwrap_or(0.0);
            let prev_last_i = prev_last as i32;
            let mut v677 = out.len() as i32;
            let seg = segs[j];
            let mut v352 = seg.pos as f64;
            let mut v353 = seg.iv;
            v354 = v340 * (v353 as f64);
            let mut v357 = v352;
            if seg.band >= 1 {
                let mut k = 1;
                loop {
                    v357 += v354;
                    v352 += v354;
                    let go = v357 < end && k < seg.band;
                    k += 1;
                    if !go {
                        break;
                    }
                }
            }
            let mut v671 = max_off;
            let v693 = seg.stat;
            buf[0] = -1;
            let v676 = (v354 * 0.125) as i32;
            let rng = sr20.max(v676);
            let v365;
            match find_near(list, v357 as i32, rng) {
                Some(idx) if idx >= 0 => {
                    let p = list[idx as usize].pos;
                    v365 = p as f64;
                    let band = list[idx as usize].band;
                    buf[0] = band;
                    v352 = BAND_OFFSET_MS[band as usize] * v340;
                    v357 = p as f64;
                }
                _ => {
                    v365 = max_off * v340;
                }
            }
            let mut v367: i32 = 0;
            let mut v368 = v352 + v365;
            if j == 0 {
                v661 = v368 as i32;
            }
            let v370 = v354 * 0.25;
            let push = |out: &mut Vec<BeatInfo>, pos: i32, iv: f32, band: i32, flag: u8| out.push(BeatInfo { pos, f1: 0.0, iv, stat: 0, band, flag });
            // Inner loop: one beat (four quarters) per pass.
            let mut seg_end = false;
            'beats: loop {
                if !(v357 < end) {
                    break 'beats;
                }
                let mut v372 = v367;
                let mut q = 0;
                loop {
                    v357 += v370;
                    v372 += 1;
                    buf[v372 as usize] = -1;
                    if rem(v372, v693) == 0 {
                        if let Some(idx) = find_near(list, v357 as i32, rng) {
                            if idx >= 0 {
                                let p = list[idx as usize].pos;
                                if p >= 0 {
                                    v357 = p as f64;
                                    buf[v372 as usize] = list[idx as usize].band;
                                }
                            }
                        }
                    }
                    q += 1;
                    if q == 4 {
                        break;
                    }
                }
                if v372 % v675 != 0 {
                    v367 = v372;
                    if v357 >= end {
                        seg_end = true;
                        break 'beats;
                    }
                    continue 'beats;
                }
                // Window of v335 beats complete: new period from the tracked drift.
                let o = off(buf[v372 as usize]).unwrap_or(v671);
                v354 = o.mul_add(v340, v357 - v368) * v657;
                v353 = ((v354 * 1000.0) * v669) as f32;
                let mut v419 = out.len() as i32;
                if !(segs[j].band + v677 < v419) {
                    if segs[j].band >= 0 {
                        let mut v422: i32 = -1;
                        let mut v423: i32 = 1;
                        let mut v425 = v677;
                        loop {
                            let p = (-v354).mul_add((segs[j].band + v423) as f64, v368) as i32;
                            let d = if p < prev_last_i {
                                if v422 >= 0 { -1 } else { 0 }
                            } else {
                                push(out, p, v353, -1, 0);
                                if v422 < 0 { 1 } else { 0 }
                            };
                            v425 += d;
                            v422 += 1;
                            v423 -= 1;
                            if !(segs[j].band > v422) {
                                break;
                            }
                        }
                        v677 = v425;
                    }
                    let b0 = buf[0];
                    push(out, v368 as i32, v353, b0, 1);
                    v419 = out.len() as i32;
                    if (b0 as u32) < 6 && v700 < 0 {
                        v700 = v419 - 1;
                    }
                }
                if v677 + a6 + segs[j].band >= v419 && a6 >= 1 {
                    let mut v455 = 0;
                    loop {
                        v368 += v354;
                        if v368 >= end {
                            break;
                        }
                        let v456 = rem(v455 + 4, v693);
                        let v457 = if v456 != 0 { -1 } else { buf[(v455 + 4) as usize] };
                        push(out, v368 as i32, v353, v457, 0);
                        if v456 == 0 && (v457 as u32) <= 5 {
                            if v700 >= 0 {
                                v697 = out.len() as i32 - 1;
                            }
                            if v700 < 0 {
                                v700 = out.len() as i32 - 1;
                            }
                        }
                        v455 += 4;
                        if v455 == 4 * a6 {
                            break;
                        }
                    }
                }
                let last = out.len() - 1;
                out[last].iv = v353;
                if a5 >= 1 {
                    let mut x = out[last].pos as f64;
                    let mut v477 = 4 * a6 + 4;
                    let mut cnt = a5;
                    loop {
                        x += v354;
                        if !(x < end) {
                            break;
                        }
                        let v478 = rem(v477, v693);
                        let v479 = if v478 != 0 { -1 } else { buf[v477 as usize] };
                        push(out, x as i32, v353, v479, 0);
                        if v478 == 0 && (v479 as u32) <= 5 {
                            if v700 >= 0 {
                                v697 = out.len() as i32 - 1;
                            }
                            if v700 < 0 {
                                v700 = out.len() as i32 - 1;
                            }
                        }
                        v477 += 4;
                        cnt -= 1;
                        if cnt == 0 {
                            break;
                        }
                    }
                }
                // (a negative index reads before the vector in the original)
                v368 = out[(out.len() as i32 - 1 - a6).max(0) as usize].pos as f64;
                v367 = v372 - 4 * a5;
                if 4 * a6 <= v367 {
                    // Slide the quarter buffer by a5 beats above the first a6 beats.
                    let count = v372 + 1 - 4 * a5 - 4 * a6;
                    for k in 0..count {
                        let d = (4 * a6 + k) as usize;
                        buf[d] = buf[d + (4 * a5) as usize];
                    }
                }
                if v357 >= end {
                    seg_end = true;
                    break 'beats;
                }
            }
            if seg_end {
                // End of segment: the last partial window.
                let mut tail = true;
                if v367 % v675 == 0 || ((v367 + 3) as u32) < 7 {
                    tail = false;
                }
                if tail {
                    let v690 = if v367 < 0 { v367 + 3 } else { v367 };
                    let mut v512 = out.len() as i32;
                    if segs[j].band + v677 >= v512 {
                        let o = off(buf[v367 as usize]).unwrap_or(v671);
                        v354 = (o.mul_add(v340, v357 - v368) * 4.0) / (v367 as f64);
                        v353 = ((v354 * 1000.0) * v669) as f32;
                        if segs[j].band >= 0 {
                            let mut v519: i32 = -1;
                            let mut v520: i32 = 1;
                            let mut v521 = v677;
                            loop {
                                let p = (-v354).mul_add((segs[j].band + v520) as f64, v368) as i32;
                                let d = if p < prev_last_i {
                                    if v519 >= 0 { -1 } else { 0 }
                                } else {
                                    push(out, p, v353, -1, 0);
                                    if v519 < 0 { 1 } else { 0 }
                                };
                                v521 += d;
                                v519 += 1;
                                v520 -= 1;
                                if !(segs[j].band > v519) {
                                    break;
                                }
                            }
                            v677 = v521;
                        }
                        let b0 = buf[0];
                        push(out, v368 as i32, v353, b0, 1);
                        v512 = out.len() as i32;
                        if (b0 as u32) < 6 && v700 < 0 {
                            v700 = v512 - 1;
                        }
                    }
                    if v677 + a6 + segs[j].band >= v512 {
                        if let Some(o) = off(buf[v367 as usize]) {
                            v671 = o;
                        }
                        v354 = (v671.mul_add(v340, v357 - v368) * 4.0) / (v367 as f64);
                        v353 = ((v354 * 1000.0) * v669) as f32;
                        if a6 >= 1 {
                            let mut v551 = 0;
                            loop {
                                v368 += v354;
                                if v368 >= end {
                                    break;
                                }
                                let v552 = rem(v551 + 4, v693);
                                let v553 = if v552 != 0 { -1 } else { buf[(v551 + 4) as usize] };
                                push(out, v368 as i32, v353, v553, 0);
                                if v552 == 0 && (v553 as u32) <= 5 {
                                    if v700 >= 0 {
                                        v697 = out.len() as i32 - 1;
                                    }
                                    if v700 < 0 {
                                        v700 = out.len() as i32 - 1;
                                    }
                                }
                                v551 += 4;
                                if v551 == 4 * a6 {
                                    break;
                                }
                            }
                        }
                    }
                    v368 = out.last().unwrap().pos as f64;
                    let v569 = (v690 >> 2) - a6;
                    if v569 >= 1 {
                        let mut v570 = 4 * a6 + 4;
                        let mut cnt = v569;
                        loop {
                            v368 += v354;
                            if !(v368 < end) {
                                break;
                            }
                            let v571 = rem(v570, v693);
                            let v572 = if v571 != 0 { -1 } else { buf[v570 as usize] };
                            push(out, v368 as i32, v353, v572, 0);
                            if v571 == 0 && (v572 as u32) <= 5 {
                                if v700 >= 0 {
                                    v697 = out.len() as i32 - 1;
                                }
                                if v700 < 0 {
                                    v700 = out.len() as i32 - 1;
                                }
                            }
                            v570 += 4;
                            cnt -= 1;
                            if cnt == 0 {
                                break;
                            }
                        }
                    }
                }
                // LABEL_874: fill to the segment end.
                if v368 < end {
                    loop {
                        v368 += v354;
                        if v368 >= end {
                            break;
                        }
                        push(out, v368 as i32, v353, -1, 0);
                    }
                }
            }
        }
        if let Some(l) = out.last_mut() {
            l.iv = (v354 / v340) as f32;
        }
        // --- P8: regular grid through the first and last tracked beats, then bar numbers.
        if out.is_empty() {
            return 0.0; // the original reads out of bounds here
        }
        if varied {
            gap_correct3(out);
        } else {
            v661 = if v700 < 0 { 0 } else { out[v700 as usize].pos };
            self.gap_correct5(out, total, v700, v697);
            bpm = 60000.0f32 / out[0].iv;
        }
        let bpb = a5.max(4);
        let n = out.len() as i32;
        if n > 0 {
            let mut v622 = upper_idx(out, v661);
            if v622 == -1 {
                v622 = 0;
            }
            let v625 = bpb + (v622 / bpb) * bpb - v622;
            for (i, b) in out.iter_mut().enumerate() {
                b.stat = (v625 + i as i32) % bpb + 1;
            }
        }
        out.retain(|b| !(b.pos < 0 || b.pos > total));
        bpm
    }

    /// The normal path's start (before LABEL_348): whole-track tempo from the candidates'
    /// intervals, removal of candidates that fit neither it nor their predecessor, and one
    /// segment from the first remaining candidate.
    fn normal_segment(&self, list: &mut Vec<BeatInfo>, total: i32, k1: f64, k2: f64, c0: f64, c1: f64) -> Option<Vec<BeatInfo>> {
        // --- P1: whole-track tempo from the candidates' intervals.
        let n = list.len() as i32;
        let mut h1 = vec![0i32; 400];
        h1[(bin1(k1, list[0].iv) / 10) as usize] += 1;
        let p0 = list[0].pos;
        for e in list.iter().skip(1) {
            if e.pos.wrapping_sub(p0) > total {
                break;
            }
            let v = bin1(k1, e.iv);
            if ((v + 9) as u32) < 0xfa9 {
                h1[(((v as i16) / 10) as u16) as usize] += 1;
            }
        }
        let _ = n;
        let mut v55 = -1i32;
        let mut m = 0;
        for (i, c) in h1.iter().enumerate() {
            if *c > m {
                m = *c;
                v55 = i as i32;
            }
        }
        if v55 < 0 {
            return None;
        }
        let mut h2 = vec![0i32; 4000];
        h2[(bin1(k2, list[0].iv) / 10) as usize] += 1;
        for e in list.iter().skip(1) {
            if e.pos.wrapping_sub(p0) > total {
                break;
            }
            let v = bin1(k2, e.iv);
            if ((v + 9) as u32) < 0x9c49 {
                h2[(v / 10) as usize] += 1;
            }
        }
        let lo = (((v55 as f64).mul_add(c1, -c1)) / c0) as i32;
        let lo = if lo <= 0 { 0 } else { lo };
        let hi = (((v55 as f64).mul_add(c1, c1)) / c0) as i32;
        let hi = if hi >= 4000 { 4000 } else { hi };
        if lo >= hi {
            return None;
        }
        let mut v68 = -1i32;
        let mut m = 0;
        for i in lo..hi {
            if h2[i as usize] > m {
                m = h2[i as usize];
                v68 = i;
            }
        }
        if v68 < 0 {
            return None;
        }
        let v70 = (600000.0f64 / (v68 as f64)) as f32;
        // Drop candidates whose interval fits neither the tempo nor the previous candidate's.
        let t = tol(v70);
        let (lo, hi) = (v70 - t, v70 + t);
        let inr = |x: f32, lo: f32, hi: f32| x > lo && x < hi;
        let mut prev = t;
        list.retain(|e| {
            let iv = e.iv;
            let p = prev;
            prev = iv;
            inr(iv, lo, hi) || inr(iv + iv, lo, hi) || inr(iv * 0.5, lo, hi) || {
                let t2 = tol(p);
                let (l2, h2) = (p - t2, p + t2);
                inr(iv, l2, h2) || inr(iv + iv, l2, h2) || inr(iv * 0.5, l2, h2)
            }
        });
        if list.is_empty() {
            return None;
        }
        Some(vec![BeatInfo { iv: v70, ..list[0] }])
    }

    /// LABEL_348: per segment, the tempo histogram of its candidates; a segment whose tempo
    /// matches the previous one (or its double or half) is merged into it.
    #[allow(clippy::too_many_arguments)]
    fn seg_tempo(&self, list: &[BeatInfo], segs: &mut Vec<BeatInfo>, total: i32, k1: f64, k2: f64, c0: f64, c1: f64) {
        let mut j: usize = 0;
        'outer: while j < segs.len() {
            let count = segs.len();
            let last = count - 1;
            let mut jj = j;
            loop {
                let (start, range) = if jj == last { (segs[last].pos, total) } else { (segs[jj].pos, segs[jj + 1].pos - segs[jj].pos) };
                let k = list.iter().position(|e| e.pos == start).map(|k| k as i32).unwrap_or(-1);
                let ku = k.max(0) as usize; // k = -1 reads before the list in the original
                let mut h1 = vec![0i32; 400];
                h1[(bin1(k1, list[ku].iv) / 10) as usize] += 1;
                let p0 = list[ku].pos;
                for e in list.iter().skip((k + 1).max(0) as usize) {
                    if e.pos.wrapping_sub(p0) > range {
                        break;
                    }
                    let v = bin1(k1, e.iv);
                    if ((v + 9) as u32) < 0xfa9 {
                        h1[(((v as i16) / 10) as u16) as usize] += 1;
                    }
                }
                let mut v266 = -1i32;
                let mut m = 0;
                for (i, c) in h1.iter().enumerate() {
                    if *c > m {
                        m = *c;
                        v266 = i as i32;
                    }
                }
                let mut merged = false;
                if v266 >= 0 {
                    let mut h2 = vec![0i32; 4000];
                    h2[(bin1(k2, list[ku].iv) / 10) as usize] += 1;
                    for e in list.iter().skip((k + 1).max(0) as usize) {
                        if e.pos.wrapping_sub(p0) > range {
                            break;
                        }
                        let v = bin1(k2, e.iv);
                        if ((v + 9) as u32) < 0x9c49 {
                            h2[(v / 10) as usize] += 1;
                        }
                    }
                    let lo = (((v266 as f64).mul_add(c1, -c1)) / c0) as i32;
                    let lo = if lo <= 0 { 0 } else { lo };
                    let hi = (((v266 as f64).mul_add(c1, c1)) / c0) as i32;
                    let hi = if hi >= 4000 { 4000 } else { hi };
                    if lo < hi {
                        let mut v279 = -1i32;
                        let mut m = 0;
                        for i in lo..hi {
                            if h2[i as usize] > m {
                                m = h2[i as usize];
                                v279 = i;
                            }
                        }
                        if v279 >= 0 {
                            let iv = (600000.0f64 / (v279 as f64)) as f32;
                            segs[jj].iv = iv;
                            segs[jj].f1 = 0.0;
                            segs[jj].stat = 4;
                            if jj >= 1 {
                                let p = segs[jj - 1].iv;
                                let t = tol(p);
                                let (l, h) = (p - t, p + t);
                                let inr = |x: f32| x > l && x < h;
                                if inr(iv) || inr(iv + iv) || inr(iv * 0.5) {
                                    segs.remove(jj);
                                    merged = true;
                                }
                            }
                        }
                    }
                }
                if merged {
                    if jj <= segs.len() {
                        j = jj - 1;
                        continue 'outer;
                    }
                    break 'outer;
                }
                jj += 1;
                if jj >= count {
                    break 'outer;
                }
            }
        }
    }

    /// `segAdjust` (0x1011944ec): per segment, tries the tempo ±1/±2 ms and beat strides of
    /// 4, 2 and 1 beats, scanning the phase in 96-sample steps for the best candidate support.
    fn seg_adjust(&self, list: &[BeatInfo], segs: &mut [BeatInfo], total: i32) {
        let n = segs.len();
        if n < 1 {
            return;
        }
        let d0 = (self.sr as f64) * 0.001;
        let d1 = total as f64;
        let w12 = self.sr / 20;
        let near_bound = |p: i32, d24: f64| -> i32 {
            // Bound from the candidate before `p` (the inlined upper-bound search).
            let d = (d24 * 0.125) as i32;
            if list.is_empty() {
                // list[-1] in the original; never reached with a non-empty sensor list.
                return p - d;
            }
            let w7 = upper_idx(list, p);
            let w7 = if w7 == -1 { 0 } else { w7 };
            if w7 - 1 < 0 {
                list[w7 as usize].pos - d
            } else {
                list[(w7 - 1) as usize].pos + d
            }
        };
        for j in 0..n {
            let iv = segs[j].iv as f64;
            let mut flag = true;
            let mut d20 = -1.0f64; // best position
            let mut d22 = 0.0f64; // best score
            let mut d21 = 0.0f64; // tempo offset in ms
            let (mut d16, mut d17, mut d18) = (0.0f64, 0.0f64, 0.0f64);
            'tempo: loop {
                let d23 = d21 + iv;
                let d24 = d0 * d23;
                let mut d25 = d1;
                if j != n - 1 {
                    let w5 = segs[j + 1].pos;
                    let w13 = near_bound(w5, d24);
                    d25 = (w13 as f64).max(segs[j].pos as f64).min(w5 as f64).max(0.0);
                }
                let d27 = if j == 0 {
                    0.0
                } else {
                    let w5 = segs[j].pos;
                    let w13 = near_bound(w5, d24);
                    (w13 as f64).max(segs[j - 1].pos as f64).min(w5 as f64).max(0.0)
                };
                let w13 = (d24 * 0.125) as i32;
                let w5 = w12.max(w13);
                let d28 = ((w5 as f64) * (0.5 * (d25 - d27))) / d24;
                let d29 = 1.0 / d24;
                let mut d26 = 4.0f64;
                loop {
                    let d30 = d26 * d24;
                    let d31 = (d30 + d27).min(d25);
                    if d27 < d31 {
                        let d8 = d26 * 4.0;
                        let mut d9 = d27;
                        loop {
                            let mut d10 = 0.0f64;
                            let upd: Option<f64>;
                            if d9 >= d25 {
                                upd = if d22 < 0.0 { Some(d9) } else { None };
                            } else {
                                let mut d11 = -1.0f64;
                                let mut d12 = d9;
                                loop {
                                    let w22 = d12 as i32;
                                    if let Some(idx) = find_near(list, w22, w5) {
                                        if idx >= 0 {
                                            let p = list[idx as usize].pos;
                                            if p >= 0 {
                                                d10 += (w5 - p.wrapping_sub(w22).wrapping_abs()) as f64;
                                                d12 = p as f64;
                                                if !(d11 >= 0.0) {
                                                    d11 = d12;
                                                }
                                            }
                                        }
                                    }
                                    d12 += d30;
                                    if d12 >= d25 {
                                        break;
                                    }
                                }
                                upd = if !(d10 > d22) {
                                    None
                                } else if d11 < 0.0 {
                                    Some(d9)
                                } else {
                                    Some(d11)
                                };
                            }
                            if let Some(x) = upd {
                                d16 = ((x - d27) * d29).trunc();
                                d20 = (-d24).mul_add(d16, x);
                                d17 = d8;
                                d18 = d23;
                                d22 = d10;
                            }
                            d9 += 96.0;
                            if d9 >= d31 {
                                break;
                            }
                        }
                    }
                    if flag && d20 >= 0.0 && (d22 * d26) > d28 {
                        break 'tempo;
                    }
                    d26 *= 0.5;
                    d22 += d22;
                    if d26 < 1.0 {
                        break;
                    }
                }
                flag = false;
                while d26 < 4.0 {
                    d22 *= 0.5;
                    d26 += d26;
                }
                if d21 != 0.0 {
                    d21 = -d21;
                }
                if !(d21 < 0.0) {
                    d21 += 1.0;
                }
                if !(d21 < 3.0) {
                    break;
                }
            }
            if d20 >= 0.0 {
                segs[j].pos = d20 as i32;
                segs[j].iv = d18 as f32;
                segs[j].stat = d17 as i32;
                segs[j].band = d16 as i32;
            }
        }
    }

    /// `gapCorrect(vector&, total, first, last)` (0x101194e7c): replaces the list by a regular
    /// grid through the beats `first` and `last`, from the start of the track to its end.
    fn gap_correct5(&self, v: &mut Vec<BeatInfo>, total: i32, b: i32, c: i32) {
        let s = b.max(0) as usize;
        let c = if c < 0 { v.len() as i32 - 1 } else { c };
        let p0 = v[s].pos as f64;
        let p1 = v[c as usize].pos as f64;
        let n = c - s as i32;
        let period = (p1 - p0) / (n as f64);
        let q = p0 / period;
        let mut k = q as i32;
        let mut x = (-period).mul_add(q.trunc(), p0);
        let band0 = v[s].band;
        let band1 = v[c as usize].band;
        v.clear();
        let iv = (period / ((self.sr as f64) * 0.001)) as f32;
        let k0 = k;
        let mk = |pos: i32| BeatInfo { pos, f1: 0.0, iv, stat: 0, band: -1, flag: 0 };
        if k >= 0 {
            loop {
                v.push(mk(x as i32));
                x += period;
                if k == 0 {
                    break;
                }
                k -= 1;
            }
            v[k0 as usize].flag = 1;
            v[k0 as usize].band = band0;
        }
        for _ in 0..n.max(0) {
            v.push(mk(x as i32));
            x += period;
        }
        let idx = n + k0;
        if idx >= 0 && (idx as usize) < v.len() {
            v[idx as usize].band = band1;
        }
        let extra = (((total as f64) - p1) / period) as i32;
        for _ in 0..extra.max(0) {
            v.push(mk(x as i32));
            x += period;
        }
    }
}

/// Removes every candidate that came from update `c` (the sensor's `changeStat` call index,
/// kept in the upper bits of `stat`).
fn erase_call(list: &mut Vec<BeatInfo>, c: i32) {
    list.retain(|e| (e.stat >> 8) != c);
}

/// `x` within `(lo, hi)`, as the original's `fcmp/fccmp gt/lt` pairs test it.
#[inline]
fn inside(x: f32, lo: f32, hi: f32) -> bool {
    x > lo && x < hi
}

/// `x`, `2x` or `x/2` within the tolerance band around `t`.
#[inline]
fn octave_match(x: f32, t: f32) -> bool {
    let w = tol(t);
    let (lo, hi) = (t - w, t + w);
    inside(x, lo, hi) || inside(x + x, lo, hi) || inside(x * 0.5, lo, hi)
}

impl Bia {
    /// The Dynamic-mode start of `analyze` (0x1011900d0 .. 0x101191584): drop updates whose
    /// tempo does not hold for more than one update, then runs of fewer than eight candidates
    /// that agree with neither neighbour, then cut segments where the 8-second tempo mode
    /// changes.
    fn varied_segments(&self, list: &mut Vec<BeatInfo>) -> Option<Vec<BeatInfo>> {
        // V1: tempo per update.
        let mut tv: Vec<f32> = vec![];
        for e in list.iter() {
            let c = e.stat >> 8;
            if c >= tv.len() as i32 {
                tv.resize((c + 1) as usize, 0.0);
                tv[c as usize] = e.iv;
                continue;
            }
            if c < 0 || tv[c as usize] != 0.0 {
                continue;
            }
            let len = tv.len() as i32;
            let mut k = c;
            let mut next = None;
            while k != len - 1 {
                k += 1;
                let v = tv[k as usize];
                if v != 0.0 {
                    next = Some(v);
                    break;
                }
            }
            if let Some(v) = next {
                if octave_match(e.iv, v) {
                    tv[c as usize] = e.iv;
                }
            }
        }
        if list.is_empty() {
            return None;
        }
        // V2: runs of updates with one tempo.
        let len = tv.len() as i32;
        if len >= 1 {
            let mut w20: i32 = 0;
            while w20 < len && tv[w20 as usize] == 0.0 {
                erase_call(list, w20);
                w20 += 1;
            }
            let mut w25 = w20 + 1;
            if w25 < len {
                let mut w24: i32 = 0;
                let mut w21: i32 = 0;
                let mut s12 = tv[w20 as usize];
                let mut s10 = 0f32;
                'outer: loop {
                    let mut x23 = w25;
                    let mut s0;
                    loop {
                        s0 = tv[x23 as usize];
                        if s0 != 0.0 {
                            break;
                        }
                        erase_call(list, x23);
                        x23 += 1;
                        if x23 >= len {
                            w25 = x23;
                            break 'outer;
                        }
                    }
                    let erase_run = |list: &mut Vec<BeatInfo>, w20: i32, x23: i32| -> i32 {
                        if w20 >= x23 {
                            return w20;
                        }
                        if !list.is_empty() {
                            for c in w20..x23 {
                                erase_call(list, c);
                            }
                        }
                        x23
                    };
                    if octave_match(s0, s12) {
                        w25 = x23 + 1;
                        w24 += 1;
                    } else if w24 > 1 {
                        let new_seg = (w24 as u32) > 7 || !octave_match(s0, s10);
                        if new_seg {
                            w25 = x23 + 1;
                            w20 = x23;
                            s10 = s12;
                            w21 = w24;
                            w24 = 0;
                        } else {
                            let x0 = erase_run(list, w20, x23);
                            if w21 != 0 {
                                w25 = x23;
                                w20 = x0;
                                s0 = s10;
                                w24 = w21;
                            } else {
                                w24 = 0;
                                s0 = tv[x23 as usize];
                                w25 = x23;
                                w20 = x0;
                            }
                        }
                    } else {
                        let x0 = erase_run(list, w20, x23);
                        if w21 == 0 {
                            w24 = 0;
                            s0 = tv[x23 as usize];
                            w25 = x23;
                            w20 = x0;
                        } else {
                            w25 = x23;
                            w20 = x0;
                            s0 = s10;
                            w24 = w21;
                        }
                    }
                    s12 = s0;
                    if !(w25 < len) {
                        break;
                    }
                }
                if w21 != 0 && !(w24 > 0) && w20 < w25 {
                    for c in w20..w25 {
                        erase_call(list, c);
                    }
                }
            }
        }
        // V3: runs of consecutive candidates with one tempo.
        if list.is_empty() {
            return None;
        }
        if list.len() > 1 {
            let mut i19 = 0usize;
            let mut i28 = 1usize;
            let mut w24: i32 = 0;
            let mut w23: i32 = 0;
            let mut s12 = list[0].iv;
            let mut s10 = 0f32;
            loop {
                let s0 = list[i28].iv;
                let s0n;
                if octave_match(s0, s12) {
                    i28 += 1;
                    w24 += 1;
                    s0n = s0;
                } else {
                    let new_run = w24 > 7 && ((w24 as u32) > 31 || !octave_match(s0, s10));
                    if new_run {
                        i19 = i28;
                        i28 += 1;
                        s10 = s12;
                        w23 = w24;
                        w24 = 0;
                        s0n = s0;
                    } else {
                        if i19 != i28 {
                            list.drain(i19..i28);
                        }
                        if w23 == 0 {
                            w24 = 0;
                            s0n = list.get(i19).map(|b| b.iv).unwrap_or(0.0);
                            i28 = i19;
                        } else {
                            i28 = i19;
                            s0n = s10;
                            w24 = w23;
                        }
                    }
                }
                s12 = s0n;
                if i28 >= list.len() {
                    break;
                }
            }
            if w23 != 0 && !(w24 > 7) && i19 != i28 {
                list.truncate(i19);
            }
        }
        // V4: segments where the 8-second tempo mode changes.
        let k1 = 600000.0f64;
        let n = list.len() as i32;
        let win = 8 * self.sr;
        let bin = |e: &BeatInfo| bin1(k1, e.iv) / 10;
        let mut segs: Vec<BeatInfo> = vec![];
        let mut hist = vec![0i32; 400];
        let mut s13 = 0f32;
        let mut w8: i32 = 0;
        'l259: loop {
            let s14 = s13;
            let x23 = w8.max(n);
            let mut x11 = w8;
            loop {
                if x11 == x23 {
                    break 'l259;
                }
                let k = x11;
                let len = list.len() as i32;
                for h in hist.iter_mut() {
                    *h = 0;
                }
                let b0 = bin(&list[k as usize]);
                hist[b0 as usize] += 1;
                x11 = k + 1;
                if k + 1 < len {
                    let p0 = list[k as usize].pos;
                    for e in &list[(k + 1) as usize..] {
                        if e.pos.wrapping_sub(p0) > win {
                            break;
                        }
                        let v = bin1(k1, e.iv);
                        if ((v + 9) as u32) < 0xfa9 {
                            hist[(((v as i16) / 10) as u16) as usize] += 1;
                        }
                    }
                }
                let mut w13 = -1i32;
                let mut m = 0;
                for (i, c) in hist.iter().enumerate() {
                    if *c > m {
                        m = *c;
                        w13 = i as i32;
                    }
                }
                if w13 == -1 {
                    continue;
                }
                s13 = 60000.0f32 / (w13 as f32);
                if octave_match(s13, s14) {
                    w8 = k + 1;
                    continue 'l259;
                }
                // Tempo change: the segment starts at the earliest candidate of the new tempo.
                let len = list.len() as i32;
                let find = |b: i32, from: i32| -> i32 { (from..len).find(|&j| bin(&list[j as usize]) == b).unwrap_or(-1) };
                let v194 = if k >= len {
                    -1
                } else if b0 == w13 {
                    k
                } else {
                    find(w13, k + 1)
                };
                let t = tol(s13);
                let v200 = t + 0.5;
                let v201 = 0.5 - t;
                let mut v198 = v194;
                for center in [s13 * 0.5, s13, s13 + s13] {
                    let lo_b = (60000.0f32 / (v200 + center)) as i32;
                    let hi_b = (60000.0f32 / (v201 + center)) as i32;
                    if lo_b < hi_b && k < len {
                        for b in lo_b..hi_b {
                            let j = find(b, k);
                            if v198 > j && j != -1 {
                                v198 = j;
                            }
                        }
                    }
                }
                let a = list[v198.max(0) as usize];
                let ivs = list[v194.max(0) as usize].iv;
                segs.push(BeatInfo { iv: ivs, ..a });
                w8 = v198 + 1;
                continue 'l259;
            }
        }
        Some(segs)
    }
}

/// `gapCorrect(vector&, int)` (0x101194b00): walks four beats at a time and repairs a gap or
/// a squeeze between the middle two (Dynamic mode).
fn gap_correct3(v: &mut Vec<BeatInfo>) {
    let c11 = f64::from_bits(0x3ff1_9999_9999_999a);
    let c09 = f64::from_bits(0x3fec_cccc_cccc_cccd);
    let c33 = f64::from_bits(0x400a_6666_6666_6666);
    let c27 = f64::from_bits(0x4005_9999_9999_999a);
    let t1 = f64::from_bits(0x3fd5_5555_5555_5555);
    let t2 = f64::from_bits(0x3fe5_5555_5555_5555);
    let mut b: usize = 0;
    loop {
        let e = v.len();
        if b >= e {
            return;
        }
        let i1 = b + 1;
        let i2 = i1 + (i1 != e) as usize;
        let i3 = i2 + (i2 != e) as usize;
        if !(i1 < e && i2 < e && i3 < e) {
            return;
        }
        let p0 = v[b].pos;
        let p1 = v[i1].pos;
        let p2 = v[i2].pos;
        let p3 = v[i3].pos;
        let d0 = p1.wrapping_sub(p0) as f64;
        let d2 = p2.wrapping_sub(p1) as f64;
        let d4 = p3.wrapping_sub(p2) as f64;
        let d3 = d0.max(d4);
        let d1 = d4.min(d0);
        let mut next_b = b + 1;
        if !(d2 / d3 <= c11) {
            // The middle interval is long.
            if d3 == d0 && !((d2 - d3) / (d3 - d1) >= 4.0) {
                v[i2].pos = p1 + (d3 as i32);
            } else if d1 == d0 && d3.mul_add(3.0, d2) > d1 * 5.0 {
                let w12 = d1 as i32;
                let w10 = (d1 + d1) as i32;
                let w11 = p1 + w12;
                v[i2].pos = w11;
                if w11 + w10 < v[i3].pos {
                    let mut tmp = v[i2];
                    tmp.pos = v[i1].pos + w10;
                    v.insert(i2, tmp);
                }
            } else {
                let r = ((d2 + d0) + d4) / d3;
                let span = p3.wrapping_sub(p0) as f64;
                if !(r >= c33) {
                    v[i1].pos = p0 + ((span * t1) as i32);
                    v[i2].pos = v[b].pos + ((span * t2) as i32);
                } else {
                    v[i1].pos = p0 + ((span * 0.25) as i32);
                    v[i2].pos = v[b].pos + ((span * 0.5) as i32);
                    let mut tmp = v[i2];
                    tmp.pos = v[b].pos + ((span * 0.75) as i32);
                    v.insert(i2, tmp);
                }
            }
        } else if !(d2 / d1 >= c09) {
            // The middle interval is short.
            if d1 == d0 && !((d1 - d2) / (d3 - d1) >= 4.0) {
                v[i2].pos = p1 + (d1 as i32);
            } else if d3 == d0 && d1.mul_add(4.0, d2) < d3 * 4.0 {
                let w8 = p1 + (d3 as i32);
                if w8 < p3 {
                    v[i2].pos = w8;
                } else {
                    v.remove(i2);
                    next_b = back_two(i2) + 1;
                }
            } else {
                let r = ((d2 + d0) + d4) / d1;
                if !(r > c27) {
                    v[i1].pos = (p3 + p0) / 2;
                    v.remove(i2);
                    next_b = back_two(i2) + 1;
                } else {
                    let span = p3.wrapping_sub(p0) as f64;
                    v[i1].pos = p0 + ((span * t1) as i32);
                    v[i2].pos = v[b].pos + ((span * t2) as i32);
                }
            }
        }
        b = next_b;
    }
}

/// After erasing at `i`, the original steps its pointer back twice (not before the start).
fn back_two(i: usize) -> usize {
    i.saturating_sub(2)
}
