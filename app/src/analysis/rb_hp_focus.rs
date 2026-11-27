//! `BeatAnalyzer_2_0::Analyzer::analyzeFocus` (0x10116c050) of rekordbox 7.2.19: merges the
//! network's beat list with the legacy analysis into the final grid.
//!
//! Constant tempo, 4/4 (`Analyzer` +45 clear and the first beats numbered up to 4):
//! 1. the network's beat times are rounded to whole ms, the intervals between them taken, and
//!    the first run of at least min(n, 37) intervals that change by at most 11 ms (then 20, 30)
//!    is found; its mean interval checks the legacy BPM (integer snap within 0.05 %, octave
//!    rules) and is the reference beat length;
//! 2. stretches whose intervals deviate from it by more than 20 ms (10 ms when such stretches
//!    cover half the track) are re-filled with evenly spaced beats;
//! 3. a grid at the legacy BPM is laid through the run: its phase is the mean offset of the run's
//!    beats, shifted by half a beat when the beats outside the run sit consistently off-beat;
//! 4. the grid is extended to the start and end of the track and refined by `UnitBeatAdjust`
//!    (`uba.rs`); a last pass on the low / full band integrals settles the bar phase (`vote.rs`).
//!
//! The original writes intermediate lists to CSV. Tests can compare these rows in memory.

use super::uba::{BeatInfo, Uba, UbaRefine};

/// What `BA_CalcBeatPos_Focus` gets: the legacy count and BPM (x100), the network's beat list
/// (`pos` relative to the window, beat number, bpm x100) and its median BPM (x100).
pub struct FocusInput<'a> {
    pub sr: i32,
    pub total: i32,
    /// Start of the network's window (`Analyzer` +80).
    pub offset: i32,
    /// Tempo may change (`Analyzer` +45).
    pub hendou: bool,
    pub legacy_bpm100: i32,
    pub legacy_count: i32,
    pub madmom: &'a [(i32, i32, i32)],
    pub madmom_bpm: i32,
}

pub struct FocusOutput {
    /// (position in samples, beat number, bpm x 100) as `BA_GetBeatPos_Focus` returns them.
    pub beats: Vec<(i32, i32, i32)>,
    /// `BA_GetBpm_Focus`.
    pub bpm100: i32,
    pub adaptive: bool,
    /// The `bool&` out parameter (set when the BPM was halved into range).
    pub flag: bool,
    /// Lines of rekordbox's debug files (interval.csv .. interval5.csv), for checks.
    #[cfg(test)]
    pub csv: [Vec<String>; 5],
}

/// C `%` for i32 (sign of the dividend).
#[inline]
fn cmod4(x: i64) -> i64 {
    x % 4
}

/// A run of intervals whose changes stay under a threshold: (first, last, count, sum of
/// intervals). `None` when the end is reached first (a run touching the end does not count).
fn find_run(c: &[f64], d: &[f64], cnt: usize, thr: f64, need: i64) -> Option<(i64, i64, i64, f64)> {
    let mut i = 0usize;
    let mut run = 0i64;
    let mut sum = 0f64;
    let mut start = -1i64;
    loop {
        while c[i] <= thr {
            sum = d[i] + sum;
            run += 1;
            if start < 0 {
                start = i as i64;
            }
            i += 1;
            if i == cnt {
                return None;
            }
        }
        if run >= need {
            return Some((start, i as i64 - 1, run, sum));
        }
        run = 0;
        sum = 0.0;
        start = -1;
        i += 1;
        if i == cnt {
            return None;
        }
    }
}

/// Stretches where the interval error `e` stays above `thr` (two in a row to start one):
/// 128 (start, end) pairs, 1-based as the original keeps them, zero after the last.
fn outlier_segments(e: &[f64], n: usize, thr: f64, first_variant: bool) -> ([i32; 128], [i32; 128], bool) {
    let mut starts = [0i32; 128];
    let mut ends = [0i32; 128];
    let mut any = false;
    if n < 3 {
        return (starts, ends, false);
    }
    let v2316 = n as i64 - 2;
    let mut i: i64 = 0;
    let mut k = 0usize;
    loop {
        let v2407 = i + 1;
        if e[i as usize] <= thr || e[v2407 as usize] <= thr {
            i += 1;
            any = true;
            if i >= v2316 {
                break;
            }
        } else {
            starts[k] = (i + 2) as i32;
            let v2408;
            if v2407 >= v2316 {
                v2408 = i + 1;
            } else {
                let mut j = v2407;
                loop {
                    let next = j + 1;
                    let cont = e[(j + 1) as usize] > thr && next < v2316;
                    j += 1;
                    if !cont {
                        v2408 = next;
                        break;
                    }
                }
            }
            i = v2408 + 1;
            ends[k] = i as i32;
            k += 1;
            any = true;
            if i >= v2316 {
                break;
            }
        }
        if k >= 128 {
            break;
        }
    }
    let _ = first_variant;
    (starts, ends, any)
}

/// Whether the good stretches between outlier segments sit half a beat apart more often than
/// not (`v2600` / `v2666` of the original).
fn half_beat_conflict(starts: &[i32; 128], ends: &[i32; 128], count: usize, list: &[BeatInfo], period: f64) -> bool {
    if count == 0 {
        return false;
    }
    let mut mid = vec![0i32; count];
    mid[0] = starts[0] / 2;
    for k in 1..count {
        mid[k] = (starts[k] + ends[k - 1]) / 2;
    }
    let inv = 1.0 / period;
    let (mut pos, mut neg) = (0i32, 0i32);
    for j in 0..count.saturating_sub(1) {
        for k in j + 1..count {
            let a = list.get(mid[j] as usize).map(|b| b.pos).unwrap_or(0);
            let b = list.get(mid[k] as usize).map(|b| b.pos).unwrap_or(0);
            let f = ((a - b) as f64 * inv).abs();
            let frac = f - f.trunc();
            if frac >= 0.6 || frac <= 0.4 {
                neg += 1;
            } else {
                pos += 1;
            }
        }
    }
    ((pos + neg) as f64) * 0.25 < pos as f64
}

/// Why the port gives no result where rekordbox would have one.
pub const NO_INPUT: &str = "no beats to work from";
pub const OUT_OF_RANGE: &str = "rekordbox reads past its beat list here; recordbox stops instead";

/// The original's branch for conflicting stretches (`v2600 || v2666`): low band over one beat
/// at each beat of the (resized, not yet repaired) list, normalised; the first beat louder than
/// 0.6, or louder than 0.2 and 0.1 above the beat before, marks where the music starts; the
/// steady stretch around it (between two 10 ms outlier stretches, else two 20 ms ones, else from
/// the start or to the end) becomes the range the grid is fitted to.
/// Returns (last, first, reference beat, its position, its number) = v2737, v2738, v2739,
/// v2741, v2742.
#[allow(clippy::too_many_arguments)]
fn conflict_range(env: &Uba, list: &[BeatInfo], offset: i32, bpm: f32, st10: &[i32; 128], en10: &[i32; 128], count10: usize, st20: &[i32; 128], en20: &[i32; 128], count20: usize) -> (i64, i64, i64, i32, i32) {
    let len = list.len();
    let m = len.saturating_sub(1); // v2731
    let mut low = vec![0f64; len];
    for i in 0..m {
        let v = env.calc_low_integ(bpm, offset + list[i].pos);
        low[i] = if i != 0 && v < 0.0 { low[i - 1] } else { v };
    }
    let mut mx = 0f64;
    for &v in &low[..m] {
        mx = mx.max(v);
    }
    if mx.abs() > 2.22044605e-16 {
        let inv = 1.0 / mx;
        for v in low[..m].iter_mut() {
            *v *= inv;
        }
    }
    let mut d = vec![0f64; len];
    if len > 2 {
        for k in 1..m {
            d[k] = (low[k] - low[k - 1]).abs();
        }
    }
    let mut v2739 = 0usize;
    if len >= 2 {
        v2739 = (0..m).find(|&k| low[k] > 0.6 || (k != 0 && low[k] > 0.2 && d[k] > 0.1)).unwrap_or(0);
    }
    let at = v2739 as i32;
    let v2741 = list[v2739].pos;
    let v2742 = list[v2739].num;
    // the steady stretch around it: between 20 ms outlier stretches k and k + 1, then the same
    // with the 10 ms ones (which win)
    let (mut v2817, mut v2818) = (0i32, 0i32);
    if count20 >= 2 {
        if let Some(k) = (0..count20 - 1).find(|&k| en20[k] <= at && st20[k + 1] >= at) {
            v2817 = en20[k];
            v2818 = st20[k + 1];
        }
    }
    if count10 >= 2 {
        if let Some(k) = (0..count10 - 1).find(|&k| !(en10[k] > at || st10[k + 1] < at)) {
            v2818 = st10[k + 1];
            v2817 = en10[k];
        }
    }
    let v3957 = st20[0];
    let (mut v2822, mut v2823) = if v3957 >= at { (v3957, 0) } else { (v2818, v2817) };
    if v3957 < at && count20 != 0 {
        let last = en20[count20 - 1];
        if last <= at {
            v2822 = len as i32 - 1;
            v2823 = last;
        } else {
            v2822 = v2818;
            v2823 = v2817;
        }
    }
    let s10 = st10[0];
    let (mut v2825, mut v2826) = if s10 >= at { (s10, 0) } else { (v2822, v2823) };
    if s10 < at && count10 != 0 {
        let last = en10[count10 - 1];
        if last <= at {
            v2825 = len as i32 - 1;
            v2826 = last;
        } else {
            v2825 = v2822;
            v2826 = v2823;
        }
    }
    let v2830 = (at + 16).min(len as i32 - 1);
    let (v2737, v2738) = if (v2826 | v2825) != 0 { (v2825, v2826) } else { (v2830, at) };
    (v2737 as i64, v2738 as i64, v2739 as i64, v2741, v2742)
}

pub fn analyze_focus(env: &Uba, inp: &FocusInput) -> Result<FocusOutput, &'static str> {
    let a3 = inp.legacy_count;
    let a4 = inp.legacy_bpm100;
    let a6 = inp.madmom.len() as i32;
    let a7 = inp.madmom_bpm;
    if a3 == 0 || a4 == 0 || a6 == 0 || a7 == 0 || inp.sr == 0 {
        return Err(NO_INPUT);
    }
    let ml = inp.madmom;
    let mut flag = false;
    #[cfg(test)]
    let mut csv: [Vec<String>; 5] = Default::default();
    // highest beat number among the first four beats
    let v81 = if a6 < 1 {
        0
    } else {
        let v80 = a6.min(4) as usize;
        let mut v = if ml[0].1 <= 0 { 0 } else { ml[0].1 };
        for b in ml.iter().take(v80).skip(1) {
            if v <= b.1 {
                v = b.1;
            }
        }
        v
    };
    let mut bpm72: f32 = (a4 as f32) * 0.01f32;
    let mut list: Vec<BeatInfo> = Vec::with_capacity(ml.len() + 64);
    let mut v87 = 0f64;
    let (mut v3978, mut v3969) = (0i32, 0i32);
    for &(pos, num, bpm) in ml {
        v3978 = pos;
        v3969 = num;
        if bpm != 0 {
            v87 = 6000000.0 / bpm as f64;
            // printed as the float it is stored as (fcvt s8 at 0x10116c310)
            #[cfg(test)]
            csv[0].push(format!("{:.3}, {}", (v87 as f32) as f64, bpm as u32));
        }
        list.push(BeatInfo { pos, zero: 0, interval: v87 as f32, num, minus1: -1, flag: false });
    }
    if inp.hendou || v81 != 4 {
        let out = super::focus2::analyze(list, inp.sr, a7, bpm72, inp.total)?;
        let bpm100 = (out.bpm72 * 100.0 + 0.5) as i32;
        let beats = out.list.iter().map(|b| (b.pos, b.num, (6_000_000.0f32 / b.interval + 0.5) as i32)).collect();
        return Ok(FocusOutput { beats, bpm100, adaptive: true, flag, #[cfg(test)] csv });
    }
    let n = a6 as usize;
    let sr = inp.sr as f64;
    let total = inp.total;
    // 1. beat times in whole ms, intervals, their changes
    let inv_sr = 1.0 / sr;
    let t: Vec<f64> = ml.iter().map(|b| ((b.0 as f64 * 1000.0) * inv_sr).round() * 0.001).collect();
    let mut d = vec![0f64; n];
    for i in 0..n - 1 {
        d[i] = (t[i + 1] - t[i]).mul_add(1000.0, 0.5).trunc();
    }
    let v2233 = n.min(37) as i64;
    let v2235: i64 = if n > 21 {
        n as i64 - 21
    } else if n > 1 {
        1
    } else {
        n as i64 - 1
    };
    if v2235 < 1 {
        return Err(NO_INPUT);
    }
    let cnt = v2235 as usize;
    let mut c = vec![0f64; n];
    for i in 0..cnt {
        c[i] = (d[i] - d[i + 1]).abs();
    }
    let run = find_run(&c, &d, cnt, 11.0, v2233).or_else(|| find_run(&c, &d, cnt, 20.0, v2233)).or_else(|| find_run(&c, &d, cnt, 30.0, v2233));
    let (start, v2308, run_len, run_sum) = match run {
        Some((s, e, r, sum)) => (s, if s < 0 { -1 } else { e }, r, sum),
        None => {
            // fall back to the first min(n, 37) intervals
            let m = v2233.max(1) as usize;
            let mut sum = 0f64;
            // pairs then the rest, as the vectorised original (whole numbers: exact)
            for x in d.iter().take(m) {
                sum = x + sum;
            }
            (0, v2233 - 1, v2233, sum)
        }
    };
    let v3908 = start.max(0);
    let v2318 = v2308.max(0);
    let v2319 = if run_len != 0 {
        run_sum / run_len as f64
    } else {
        let mut arr = vec![0f64; n];
        arr[..n - 1].copy_from_slice(&d[..n - 1]);
        if n >= 3 {
            for a in 0..(n - 2) {
                for b in a + 1..n - 1 {
                    if arr[a] < arr[b] {
                        arr.swap(a, b);
                    }
                }
            }
        }
        let m = n - 1;
        if m % 2 == 1 {
            arr[m >> 1]
        } else {
            let k = m >> 1;
            (arr[k] + arr[k.saturating_sub(1)]) * 0.5
        }
    };
    // legacy BPM: integer snap, octave rules against the run's tempo
    {
        let mut v2346 = bpm72;
        let r = bpm72.round();
        let mut v2347 = bpm72 as f64;
        let rd = r as f64;
        if rd * 0.9995 <= v2347 && rd * 1.0005 >= v2347 {
            bpm72 = r;
            v2347 = rd;
            v2346 = r;
        }
        let v2350 = 60000.0 / v2319;
        if v2350 >= v2347 * 1.98 && v2350 <= v2347 * 2.02 {
            if !(v2346 > 90.0) {
                bpm72 = v2346 + v2346;
            }
        } else if v2350 >= v2347 * 0.495 && v2350 <= v2347 * 0.505 {
            if !(v2346 < 140.0) {
                bpm72 = v2346 * 0.5;
            }
        } else if v2350 < v2347 * 0.8 || v2350 > v2347 * 1.25 {
            if v2350 < 70.0 {
                let s = v2350 as f32;
                bpm72 = s + s;
            } else if v2350 <= 180.0 {
                bpm72 = v2350 as f32;
            } else {
                bpm72 = (v2350 as f32) * 0.5;
                flag = true;
            }
        }
    }
    let v2346 = bpm72;
    // 2. interval errors and the outlier stretches
    let mut e = vec![0f64; n];
    if n >= 2 {
        for i in 0..n - 1 {
            e[i] = (d[i] - v2319).abs();
        }
        e[n - 1] = e[(n - 2).max(0)];
    }
    let tms: Vec<f64> = t.iter().map(|x| x * 1000.0).collect();
    let (st10, en10, any10) = outlier_segments(&e, n, 10.0, true);
    let mut v2412 = 0i64;
    for k in 0..128 {
        if st10[k] <= 0 {
            break;
        }
        v2412 = v2412 - st10[k] as i64 + en10[k] as i64 + 1;
    }
    let v3967 = sr * 60.0 / v2346 as f64;
    let v3905 = v2318;
    let mut fixed: Vec<f64> = Vec::with_capacity(n + 16);
    let mut st20 = [0i32; 128];
    let mut en20 = [0i32; 128];
    let mut lens20 = [0i32; 128];
    let mut counts20 = [0i32; 128];
    let use10 = !((v2412 as f64) / (n as f64) < 0.5);
    {
        let (st, en) = if !use10 {
            if any10 {
                let (s, e2, _) = outlier_segments(&e, n, 20.0, false);
                st20 = s;
                en20 = e2;
            }
            (st20, en20)
        } else {
            (st10, en10)
        };
        // re-fill the stretches with evenly spaced beats
        fixed.push(t[0]);
        let inv = 1.0 / v2319;
        let mut nseg = 0usize;
        for k in 0..128 {
            let v2425 = st[k];
            if v2425 <= 0 {
                break;
            }
            nseg = k + 1;
            let v2426 = en[k];
            if !use10 {
                lens20[k] = v2426 - v2425 + 1;
            }
            let (v2431, v2432): (i64, f64);
            let lo = if v2425 == 1 { tms[0] } else { tms[(v2425 - 2) as usize] };
            if v2426 as usize >= n {
                v2431 = ((tms[n - 1] - lo) * inv).round() as i64;
                v2432 = v2319;
            } else {
                let span = tms[v2426 as usize] - lo;
                let r = (span * inv).round() as i64;
                if r != 0 {
                    v2431 = r - 1;
                    v2432 = span / r as f64;
                } else {
                    v2431 = -1;
                    v2432 = v2319;
                }
            }
            if !use10 {
                counts20[k] = v2431 as i32;
            }
            let interpolate = |fixed: &mut Vec<f64>, from: usize| {
                for m in 1..=v2431 {
                    fixed.push(v2432.mul_add(m as f64, tms[from]).round() * 0.001);
                }
            };
            let v2358 = n as i64 - 1;
            if k != 0 {
                let prev = en[k - 1];
                if prev + 1 < v2425 - 1 {
                    for i in (prev + 1)..(v2425 - 1) {
                        fixed.push(t[i as usize]);
                    }
                }
                if v2431 > 0 {
                    if v2425 != 1 {
                        interpolate(&mut fixed, (v2425 - 2) as usize);
                    }
                    if (v2426 as i64) < v2358 {
                        fixed.push(t[v2426 as usize]);
                    }
                }
            } else if v2425 == 1 {
                if !(v2431 < 1 || v2426 as i64 >= v2358) {
                    fixed.push(t[v2426 as usize]);
                }
            } else {
                if v2425 >= 3 {
                    for i in 1..(v2425 - 1) {
                        fixed.push(t[i as usize]);
                    }
                }
                if v2431 > 0 {
                    interpolate(&mut fixed, (v2425 - 2) as usize);
                    if (v2426 as i64) < v2358 {
                        fixed.push(t[v2426 as usize]);
                    }
                }
            }
        }
        if nseg == 0 || st[0] == 0 {
            for x in t.iter().skip(1) {
                fixed.push(*x);
            }
        } else {
            let last_end = en[nseg - 1] as usize;
            if last_end < n && last_end + 1 < n {
                for x in &t[last_end + 1..n] {
                    fixed.push(*x);
                }
            }
        }
    }
    let v3957 = st20[0];
    // the focus list takes the repaired length
    while fixed.len() > list.len() {
        list.push(BeatInfo { pos: v3978, zero: 0, interval: v87 as f32, num: v3969, minus1: -1, flag: false });
    }
    // what the original's vector still holds past its end after shrinking (`+120 -= 24 * n`):
    // later reads of the run's last beat can land there on short tracks
    let mut stale: Vec<BeatInfo> = vec![];
    if fixed.len() < list.len() {
        let drop = (list.len() - fixed.len()).max(1);
        let keep = list.len().saturating_sub(drop);
        stale = list[keep..].to_vec();
        list.truncate(keep);
    }
    let len = list.len();
    let count10 = st10.iter().position(|&x| x == 0).unwrap_or(0);
    let count20 = if v3957 != 0 { (1..128).find(|&k| st20[k] == 0).unwrap_or(0) } else { 0 };
    let v2600 = half_beat_conflict(&st10, &en10, count10, &list, v3967);
    let v2666 = half_beat_conflict(&st20, &en20, count20, &list, v3967);
    let conflict = v2600 || v2666;
    if std::env::var("PREPDECK_HP_FOCUSTRACE").is_ok() {
        eprintln!("focus: len {len} run {v3908}..{v3905} v2319 {v2319} bpm72 {bpm72} conflict10 {v2600} conflict20 {v2666} st10 {:?} en10 {:?} st20 {:?} en20 {:?}", &st10[..count10], &en10[..count10], &st20[..count20.max((v3957 != 0) as usize)], &en20[..count20.max((v3957 != 0) as usize)]);
    }
    let (mut v2737, mut v2738, mut v2739): (i64, i64, i64) = (0, 0, 0);
    let (v2741, v2742);
    if conflict {
        // the steady stretches disagree by half a beat: take the stretch where the low band
        // first gets loud instead of the run (decompile from LABEL_3584 on, 0x10117...)
        let r = conflict_range(env, &list, inp.offset, bpm72, &st10, &en10, count10, &st20, &en20, count20);
        v2737 = r.0;
        v2738 = r.1;
        v2739 = r.2;
        v2741 = r.3;
        v2742 = r.4;
        if v2738 < 0 || v2737 < v2738 || v2737 as usize >= len || v2739 < 0 || v2739 as usize >= len {
            return Err(OUT_OF_RANGE);
        }
    } else {
        v2741 = list[v3908 as usize].pos;
        v2742 = list[v3908 as usize].num;
    }
    // positions from the repaired times
    {
        let k = sr * 0.001;
        let pairs = if fixed.len() >= 2 { fixed.len() & !1 } else { 0 };
        for (i, x) in fixed.iter().enumerate() {
            let p = if i < pairs { ((x * 1000.0).round() * k) as i64 as i32 } else { ((x * 1000.0).round() * 0.001 * sr) as i32 };
            list[i].pos = p;
            list[i].interval = 0.0;
        }
    }
    // beat nearest to the start of the run, and the bar phase from its number
    let mut v2846 = len as i64 - 1;
    if len <= 1 {
        v2846 = 0;
    } else {
        let mut dprev = (list[1].pos - v2741).abs() as u32;
        let d0 = (list[0].pos - v2741).abs() as u32;
        if d0 <= dprev {
            v2846 = 0;
        } else {
            let mut idx = 0i64;
            while (len as i64 - 2) != idx {
                let dj = (list[(idx + 2) as usize].pos - v2741).abs() as u32;
                idx += 1;
                let closer = dprev > dj;
                dprev = dj;
                if !closer {
                    v2846 = idx;
                    break;
                }
            }
        }
    }
    let mut v2854 = v2742 as i64 - 1 - (v2846 & 3);
    if v2854 < 0 {
        v2854 += 4;
    }
    for (i, b) in list.iter_mut().enumerate() {
        b.num = cmod4(v2854 + i as i64) as i32 + 1;
    }
    // interval2.csv
    if len >= 2 {
        for i in 0..len - 1 {
            if i + 1 >= list.len() {
                break;
            }
            let iv = ((list[i + 1].pos - list[i].pos) as f64 * 1000.0 / sr) as f32;
            list[i].interval = iv;
            #[cfg(test)]
            csv[1].push(format!("{:.3}", iv as f64));
        }
        list[len - 1].interval = list[len - 2].interval;
    }
    // 3. phase: half-beat shift when the beats outside the run sit off-beat (not when the
    //    stretches conflict: then the range above is used as it is)
    let mut v2891 = 0f64;
    if !conflict {
        let mut v2893 = 0i64;
        if v3957 >= 1 {
            for k in 0..128 {
                if v3908 >= en20[k] as i64 {
                    v2893 = counts20[k] as i64 + v2893 - lens20[k] as i64;
                }
                if k == 127 || st20[k + 1] <= 0 {
                    break;
                }
            }
        }
        v2738 = v2893 + v3908;
        v2737 = v2893 + v3905;
        // the original reads past the list here; recordbox gives up instead
        if v2738 < 0 || v2737 < v2738 || v2737 as usize >= len + stale.len() {
            return Err(OUT_OF_RANGE);
        }
        let v2896 = ((v2737 + v2738) / 2) as usize;
        let v2898 = list[v2896].num as i64;
        v2739 = if v2898 == 1 { v2896 as i64 } else { v2896 as i64 - v2898 + 1 };
        if v2739 < 0 || v2739 as usize >= len {
            return Err(OUT_OF_RANGE);
        }
        let pos = |i: i64| list.get(i as usize).or_else(|| stale.get(i as usize - len)).map(|b| b.pos).unwrap_or(0) as f64;
        let v2900 = pos(v2739);
        let (mut v2901, mut v2902) = (0f64, 0f64);
        for i in 0..v2738.max(0) {
            let v2907 = i.max(5);
            let lo = v2907 - 5;
            let hi = if i + 5 >= v2738 { v2738 - 1 } else { i + 5 };
            let v2910 = v3967 * (v2739 - i) as f64;
            let mut v2911 = v2910 - v2900 + pos(lo);
            let mut v2912 = v2911.abs();
            let v2903;
            if lo > hi {
                v2903 = v2912;
            } else {
                let mut m = v2912;
                for j in lo..=hi {
                    let v2916 = v2910 - v2900 + pos(j);
                    let v2917 = v2916.abs();
                    m = v2912.min(v2917);
                    if v2912 > v2917 {
                        v2911 = v2916;
                    }
                    v2912 = m;
                }
                v2903 = m;
            }
            let mut v2904 = v2910 + pos(i) - v2900;
            let v2905 = v2904.abs();
            let v2906 = if v2903 * 1.25 <= v2905 { v2903 } else { v2905 };
            if v2903 * 1.25 <= v2905 {
                v2904 = v2911;
            }
            v2901 = v2904 + v2901;
            v2902 = v2906 + v2902;
        }
        let v2730 = len as i64;
        let v2881 = len as i64 - 1;
        let mut i = v2737 + 1;
        while i < v2730 {
            if i as usize >= list.len() {
                break;
            }
            let mut lo = i - 5;
            if lo <= v2737 {
                lo = v2737 + 1;
            }
            let hi = if i + 5 >= v2730 { v2881 } else { i + 5 };
            let v2928 = v3967 * (v2739 - i) as f64;
            let v2929 = pos(v2739);
            let mut v2930 = v2928 - v2929 + pos(lo);
            let mut v2931 = v2930.abs();
            let mut v2922 = v2931;
            if lo <= hi {
                let mut j = lo;
                while j <= hi && (j as usize) < list.len() {
                    let v2937 = v2928 - v2929 + pos(j);
                    let v2938 = v2937.abs();
                    v2922 = v2931.min(v2938);
                    if v2931 > v2938 {
                        v2930 = v2937;
                    }
                    v2931 = v2922;
                    j += 1;
                }
                v2922 = v2931;
            }
            let mut v2923 = v2928 + pos(i) - v2929;
            let v2924 = v2923.abs();
            let v2925 = if v2922 * 1.25 <= v2924 { v2922 } else { v2924 };
            if v2922 * 1.25 <= v2924 {
                v2923 = v2930;
            }
            v2901 = v2923 + v2901;
            v2902 = v2925 + v2902;
            i += 1;
        }
        let v2939 = v2738 + len as i64 - v2737 - 1;
        let v2940 = if v2939 != 0 { v2902 / v2939 as f64 } else { 0.0 };
        if v2940 > v3967 * 0.3 && (v2939 as f64) / (len as f64) > 0.5 {
            v2891 = if v2901 > 0.0 { v3967 * 0.5 } else { v3967 * -0.5 };
        }
    }
    // the grid through the run: mean offset of the run's beats from an ideal grid
    let pos_ref = list[v2739 as usize].pos as f64;
    let mut v2992 = 0f64;
    if v2737 - v2738 >= 0 {
        let count = (v2737 - v2738 + 1) as usize;
        let mut diffs = Vec::with_capacity(count);
        for i in v2738..=v2737 {
            let ideal = v3967.mul_add((i - v2739) as f64, pos_ref);
            let p = list.get(i as usize).or_else(|| stale.get(i as usize - len)).map(|b| b.pos).unwrap_or(0);
            diffs.push(p as f64 - ideal);
        }
        // 8 lanes over all but the last 1..8 values, then the rest in order
        let mut s = 0f64;
        let split = if count >= 9 {
            let r = count & 7;
            count - if r == 0 { 8 } else { r }
        } else {
            0
        };
        if split > 0 {
            let mut lanes = [0f64; 8];
            for cch in (0..split).step_by(8) {
                for l in 0..8 {
                    lanes[l] = diffs[cch + l] + lanes[l];
                }
            }
            let a0 = lanes[2] + lanes[0];
            let a1 = lanes[3] + lanes[1];
            let a0 = lanes[4] + a0;
            let a1 = lanes[5] + a1;
            let a0 = lanes[6] + a0;
            let a1 = lanes[7] + a1;
            s = a0 + a1;
        }
        for x in &diffs[split..] {
            s = x + s;
        }
        v2992 = s / count as f64;
    }
    let mut ideal2 = vec![0f64; len];
    {
        let v2997 = v2992 + v2891 + pos_ref;
        for i in 0..len {
            let v = v3967.mul_add((i as i64 - v2739) as f64, v2997);
            ideal2[i] = v;
            list[i].pos = (v + 0.5) as i32;
            list[i].num = cmod4(v2854 + i as i64) as i32 + 1;
        }
    }
    // interval3.csv
    if len >= 2 {
        for i in 0..len - 1 {
            let iv = ((ideal2[i + 1] - ideal2[i]) * 1000.0 / sr) as f32;
            list[i].interval = iv;
            #[cfg(test)]
            csv[2].push(format!("{:.3}", iv as f64));
        }
        list[len - 1].interval = list[len - 2].interval;
    }
    for b in list.iter_mut().take(len) {
        b.pos += inp.offset;
    }
    // 4. extend to the start and the end of the track
    // A valid tempo must yield a finite period within the public analysis
    // range (1–1000 BPM). Avoid expanding a model's malformed interval.
    if !(sr * 60.0 / 1000.0..=sr * 60.0).contains(&v3967) { return Err(OUT_OF_RANGE); }
    if list[0].pos > total || list[len-1].pos < 0 { return Err(OUT_OF_RANGE); }
    let v3027 = (list[0].pos as f64 / v3967) as i32;
    let v3029 = (len - 1).min(list.len() - 1);
    let v3030 = list[v3029].pos;
    let interval = (60000.0 / bpm72 as f64) as f32;
    if v3027 >= 1 {
        let first = list[0];
        let mut leading = Vec::with_capacity(v3027 as usize);
        for j in 1..=v3027 {
            let mut p = ((-v3967).mul_add(j as f64, 0.5) + first.pos as f64) as i32;
            if p <= 0 {
                p = 0;
            }
            let r = cmod4(first.num as i64 - 1 - j as i64);
            let num = if r >= 0 { r + 1 } else { r + 5 } as i32;
            #[cfg(test)]
            csv[3].push(format!("{:.3}", interval as f64));
            leading.push(BeatInfo { pos: p, zero: 0, interval, num, minus1: -1, flag: false });
        }
        leading.reverse();
        leading.append(&mut list);
        list = leading;
    }
    let v3046 = ((total - v3030) as f64 / v3967) as i32;
    if v3046 >= 1 {
        let base = list[list.len() - 1];
        for k in 1..=v3046 {
            let mut p = (v3967.mul_add(k as f64, 0.5) + base.pos as f64) as i32;
            if total < p {
                p = total;
            }
            let num = cmod4(k as i64 + base.num as i64 - 1) as i32 + 1;
            #[cfg(test)]
            csv[4].push(format!("{:.3}", interval as f64));
            list.push(BeatInfo { pos: p, zero: 0, interval, num, minus1: -1, flag: false });
        }
    }
    let mut refine = UbaRefine::new(env);
    refine.analyze(&mut bpm72, &mut list);
    // 5. the bar-phase vote on the band integrals (beat numbers only)
    super::vote::bar_phase_vote(env, bpm72, inp.sr, &mut list);
    let bpm100 = ((bpm72 * 100.0) + 0.5) as i32;
    let beats = list.iter().map(|b| (b.pos, b.num, ((6000000.0f32 / b.interval) + 0.5) as i32)).collect();
    Ok(FocusOutput { beats, bpm100, adaptive: false, flag, #[cfg(test)] csv })
}
