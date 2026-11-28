//! Dynamic analysis mode: a beat grid that follows tempo changes (live drums, tempo ramps,
//! transition tracks), as rekordbox's "Dynamic" Analysis Mode does. The result is a grid of
//! tempo segments: each anchor marks a beat and the tempo that holds until the next anchor.
//!
//! Method (independently written): local tempo from windowed autocorrelation of the onset
//! envelope, smoothed by a Viterbi path over the windows; beats by dynamic programming along
//! that tempo curve; then the beat times are fitted with straight segments, and a new anchor
//! starts wherever one constant tempo no longer explains the beats within `FIT_TOL` seconds.

use super::{normalize, onsets, structure_votes, tempo_and_grid};
use crate::model::{BeatGrid, GridAnchor};

/// A constant-tempo segment must explain its beats within this many seconds.
const FIT_TOL: f64 = 0.012;
/// Local tempo windows: length and hop in seconds.
const WIN_S: f64 = 8.0;
const HOP_S: f64 = 2.0;

/// Tempo (BPM of the first segment) and a grid with as many anchors as the tempo needs.
/// A track whose tempo is steady comes back with the same single-anchor grid as Normal mode.
pub fn tempo_and_grid_dynamic(mono: &[f32], sr: f64, bpm_min: f64, bpm_max: f64) -> (f64, Option<BeatGrid>) {
    let (bpm0, grid0) = tempo_and_grid(mono, sr, bpm_min, bpm_max);
    let Some(grid0) = grid0 else { return (bpm0, None) };
    let duration = mono.len() as f64 / sr;
    let mut o = onsets(mono, sr);
    normalize(&mut o.all);
    normalize(&mut o.low);
    let env: Vec<f32> = o.all.iter().zip(&o.low).map(|(a, l)| a + 0.6 * l).collect();
    let fps = o.fps;
    if env.len() < (WIN_S * fps) as usize * 2 {
        return (bpm0, Some(grid0));
    }
    let periods = local_periods(&env, fps, bpm_min, bpm_max, 60.0 * fps / bpm0);
    let mut frames = track_beats(&env, &periods, fps);
    // Beats sit on the kick: when the low band is clearly stronger halfway between the tracked
    // beats, the tracker locked onto the off-beat, and every beat moves to the midpoint.
    let low_at = |f: f64| (-2..=2).map(|d| super::interp(&o.low, f + d as f64)).fold(0f32, f32::max) as f64;
    let (mut on, mut off) = (0.0, 0.0);
    for w in frames.windows(2) {
        on += low_at(w[0]);
        off += low_at((w[0] + w[1]) / 2.0);
    }
    if off > on * 1.15 {
        let mids: Vec<f64> = frames.windows(2).map(|w| (w[0] + w[1]) / 2.0).collect();
        frames = mids;
    }
    if frames.len() < 16 {
        return (bpm0, Some(grid0));
    }
    // Beats placed before the music starts (or after it ends) have nothing under them.
    let peak = env.iter().cloned().fold(0f32, f32::max).max(1e-9);
    let strong = |f: &f64| (-2..=2).map(|d| super::interp(&env, f + d as f64)).fold(0f32, f32::max) > 0.08 * peak;
    let lead = frames.iter().position(strong).unwrap_or(0);
    let tail = frames.iter().rposition(strong).unwrap_or(frames.len() - 1);
    let frames = &frames[lead..=tail.max(lead)];
    if frames.len() < 16 {
        return (bpm0, Some(grid0));
    }
    let mut times: Vec<f64> = frames.iter().map(|f| o.t0 + f / fps).collect();
    // The envelope peaks a little after the attack starts; Normal mode's grid is placed on
    // the attack itself. Where the constant grid agrees with the tracked beats, use it to
    // measure that lag; otherwise use the lag typical for this envelope.
    let const_beats = grid0.beats(duration);
    let mut diffs: Vec<f64> = times
        .iter()
        .filter_map(|t| crate::model::nearest_beat(&const_beats, *t).map(|b| t - b.time))
        .filter(|d| d.abs() < 0.03)
        .collect();
    let lag = if diffs.len() >= 16 && diffs.len() * 2 >= times.len() {
        diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        diffs[diffs.len() / 2]
    } else {
        0.006
    };
    for t in times.iter_mut() {
        *t -= lag;
    }
    // A steady track stays steady: when one tempo already explains four beats in five, the
    // rest are stretches with nothing to track (breakdowns, intros), not tempo changes.
    let explained = times.iter().filter(|t| crate::model::nearest_beat(&const_beats, **t).map(|b| (b.time - **t).abs() < 0.02).unwrap_or(false)).count();
    if explained * 5 >= times.len() * 4 {
        return (bpm0, Some(grid0));
    }
    let segs = bridge_drift(merge_similar(&times, merge_short(&times, segments(&times, FIT_TOL)), FIT_TOL));
    // Runs at the track's overall tempo that together hold 85 % of the beats: the short
    // remainder is the tracker wandering where there was nothing to follow.
    let at_main: usize = segs.iter().filter(|s| ((60.0 / s.2) / bpm0 - 1.0).abs() < 0.0015).map(|s| s.1 - s.0 + 1).sum();
    if at_main * 100 >= times.len() * 85 {
        return (bpm0, Some(grid0));
    }
    if segs.len() <= 1 {
        // One tempo explains every beat: Normal mode's grid has the finer phase.
        let fits = segs.first().map(|s| (60.0 / s.2 - bpm0).abs() < 0.5).unwrap_or(true);
        if fits {
            return (bpm0, Some(grid0));
        }
    }
    // Bar position: the strongest section changes vote for the beat that starts a bar.
    let votes = structure_votes(mono, sr, &times);
    let mut down = (0..4).max_by(|a, b| votes[*a].partial_cmp(&votes[*b]).unwrap()).unwrap_or(0);
    if votes.iter().all(|v| *v == 0.0) {
        // Too short to vote: keep Normal mode's bar position for the nearest beat.
        if let Some(b) = crate::model::nearest_beat(&const_beats, times[0]) {
            down = (5 - b.beat as usize) % 4;
        }
    }
    let mut anchors = vec![];
    for (start, _end, period, t0) in &segs {
        let bpm = ((60.0 / period) * 100.0).round() / 100.0;
        let beat = ((start + 4 - down) % 4) as u8 + 1;
        anchors.push(GridAnchor { time: t0.max(0.0), bpm, beat });
    }
    // The first anchor's bar position must hold when beats are extended back to time zero,
    // which `BeatGrid::beats` does by itself.
    let bpm = anchors[0].bpm;
    (bpm, Some(BeatGrid::from_anchors(anchors)))
}

/// Beat period in frames for each envelope frame: the best autocorrelation lag per window,
/// with a penalty for jumping between windows.
fn local_periods(env: &[f32], fps: f64, bpm_min: f64, bpm_max: f64, global: f64) -> Vec<f64> {
    let win = (WIN_S * fps) as usize;
    let hop = (HOP_S * fps) as usize;
    let lag_min = (60.0 * fps / bpm_max).floor().max(2.0) as usize;
    let lag_max = (60.0 * fps / bpm_min).ceil() as usize;
    let lags: Vec<usize> = (lag_min..=lag_max).collect();
    let mut scores: Vec<Vec<f64>> = vec![];
    let mut centres: Vec<usize> = vec![];
    let mut start = 0usize;
    while start + win <= env.len() {
        let w = &env[start..start + win];
        let mean = w.iter().map(|v| *v as f64).sum::<f64>() / win as f64;
        let e: Vec<f64> = w.iter().map(|v| *v as f64 - mean).collect();
        let ac = |lag: usize| -> f64 {
            if lag >= win {
                return 0.0;
            }
            let mut s = 0.0;
            for i in 0..win - lag {
                s += e[i] * e[i + lag];
            }
            s / (win - lag) as f64
        };
        let e0 = ac(0).max(1e-12);
        let row: Vec<f64> = lags
            .iter()
            .map(|&l| {
                let s = ac(l) + 0.5 * ac(l * 2) + 0.33 * ac(l * 4);
                // A weak pull towards the track's overall tempo keeps octave choices consistent.
                let pull = (-0.5 * ((l as f64 / global).log2() / 0.35).powi(2)).exp();
                s / e0 * (0.6 + 0.4 * pull)
            })
            .collect();
        scores.push(row);
        centres.push(start + win / 2);
        start += hop;
    }
    if scores.is_empty() {
        return vec![global; env.len()];
    }
    // Viterbi over windows: staying near the previous period is cheap, jumping is not.
    let n = lags.len();
    let mut cost = scores[0].clone();
    let mut back: Vec<Vec<usize>> = vec![];
    for row in scores.iter().skip(1) {
        let mut next = vec![f64::MIN; n];
        let mut bk = vec![0usize; n];
        for j in 0..n {
            let lo = j.saturating_sub(40);
            let hi = (j + 40).min(n - 1);
            for i in lo..=hi {
                let r = (lags[j] as f64 / lags[i] as f64).log2();
                let c = cost[i] - 30.0 * r * r;
                if c > next[j] {
                    next[j] = c;
                    bk[j] = i;
                }
            }
            next[j] += row[j];
        }
        cost = next;
        back.push(bk);
    }
    let mut j = (0..n).max_by(|a, b| cost[*a].partial_cmp(&cost[*b]).unwrap()).unwrap_or(0);
    let mut path = vec![j; scores.len()];
    for k in (0..back.len()).rev() {
        j = back[k][j];
        path[k] = j;
    }
    // Refine each window's lag between integer lags (parabola through the neighbours).
    let fine: Vec<f64> = path
        .iter()
        .enumerate()
        .map(|(k, &j)| {
            if j == 0 || j + 1 >= n {
                return lags[j] as f64;
            }
            let (a, b, c) = (scores[k][j - 1], scores[k][j], scores[k][j + 1]);
            let d = a - 2.0 * b + c;
            let off = if d.abs() > 1e-12 { (0.5 * (a - c) / d).clamp(-0.5, 0.5) } else { 0.0 };
            lags[j] as f64 + off
        })
        .collect();
    // Per frame: linear interpolation between window centres.
    let mut out = vec![0f64; env.len()];
    for (f, o) in out.iter_mut().enumerate() {
        let k = centres.partition_point(|c| *c <= f);
        *o = if k == 0 {
            fine[0]
        } else if k >= centres.len() {
            fine[fine.len() - 1]
        } else {
            let (c0, c1) = (centres[k - 1] as f64, centres[k] as f64);
            let w = (f as f64 - c0) / (c1 - c0);
            fine[k - 1] * (1.0 - w) + fine[k] * w
        };
    }
    out
}

/// Beat positions in frames (with a fraction), by dynamic programming: each beat is where the
/// envelope is strong and the distance to the previous beat is close to the local period.
fn track_beats(env: &[f32], periods: &[f64], fps: f64) -> Vec<f64> {
    let n = env.len();
    let alpha = 400.0 * (200.0 / fps);
    let mut score = vec![0f64; n];
    let mut back = vec![usize::MAX; n];
    let peak = env.iter().cloned().fold(0f32, f32::max).max(1e-9) as f64;
    for t in 0..n {
        let p = periods[t];
        let lo = t as f64 - 2.0 * p;
        let hi = t as f64 - 0.5 * p;
        let mut best = 0.0;
        let mut bi = usize::MAX;
        if hi >= 0.0 {
            let a = lo.max(0.0) as usize;
            let b = hi as usize;
            for prev in a..=b.min(t.saturating_sub(1)) {
                let r = ((t - prev) as f64 / p).ln();
                let c = score[prev] - alpha * r * r * peak;
                if bi == usize::MAX || c > best {
                    best = c;
                    bi = prev;
                }
            }
        }
        score[t] = env[t] as f64 + if bi == usize::MAX { 0.0 } else { best };
        back[t] = bi;
    }
    // End on the best frame within the last period.
    let last_p = periods[n - 1].ceil() as usize;
    let from = n.saturating_sub(last_p.max(1));
    let mut t = (from..n).max_by(|a, b| score[*a].partial_cmp(&score[*b]).unwrap()).unwrap_or(n - 1);
    let mut frames = vec![];
    loop {
        // Position between frames from the envelope's shape around the chosen frame.
        let f = if t > 0 && t + 1 < n {
            let (a, b, c) = (env[t - 1] as f64, env[t] as f64, env[t + 1] as f64);
            let d = a - 2.0 * b + c;
            t as f64 + if d < -1e-9 { (0.5 * (a - c) / d).clamp(-0.5, 0.5) } else { 0.0 }
        } else {
            t as f64
        };
        frames.push(f);
        if back[t] == usize::MAX {
            break;
        }
        t = back[t];
    }
    frames.reverse();
    frames
}

/// Straight-line fit of beat times `ts[a..=b]` against their index: (period, time of beat a,
/// largest deviation).
fn fit(ts: &[f64], a: usize, b: usize) -> (f64, f64, f64) {
    let n = (b - a + 1) as f64;
    let mx = (n - 1.0) / 2.0;
    let my = ts[a..=b].iter().sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (k, t) in ts[a..=b].iter().enumerate() {
        let dx = k as f64 - mx;
        sxy += dx * (t - my);
        sxx += dx * dx;
    }
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    let t0 = my - slope * mx;
    let dev = ts[a..=b].iter().enumerate().map(|(k, t)| (t - (t0 + slope * k as f64)).abs()).fold(0.0, f64::max);
    (slope, t0, dev)
}

/// Splits beat times into constant-tempo runs: (first beat index, last beat index, period in
/// seconds, fitted time of the first beat). A run grows while one tempo explains all its
/// beats within `tol`; runs are at least 4 beats long.
pub fn segments(ts: &[f64], tol: f64) -> Vec<(usize, usize, f64, f64)> {
    let mut out = vec![];
    let mut a = 0usize;
    while a < ts.len() {
        let mut b = (a + 3).min(ts.len() - 1);
        let mut best = fit(ts, a, b);
        // Grow in doubling steps, then settle the exact end by bisection.
        let mut step = 4usize;
        loop {
            let nb = (b + step).min(ts.len() - 1);
            if nb == b {
                break;
            }
            let f = fit(ts, a, nb);
            if f.2 <= tol {
                b = nb;
                best = f;
                step *= 2;
            } else if step > 1 {
                step /= 2;
            } else {
                break;
            }
        }
        if b - a < 3 && !out.is_empty() && ts.len() - a < 4 {
            // A short tail joins the previous run.
            let (pa, _, _, _): (usize, usize, f64, f64) = out.pop().unwrap();
            let f = fit(ts, pa, ts.len() - 1);
            out.push((pa, ts.len() - 1, f.0, f.1));
            break;
        }
        if best.0 <= 0.0 {
            break;
        }
        out.push((a, b, best.0, best.1));
        a = b + 1;
    }
    out
}

/// Runs shorter than 8 beats are tracking noise more often than music: each joins the
/// neighbour whose tempo is closer, and the joined run is fitted again.
fn merge_short(ts: &[f64], mut segs: Vec<(usize, usize, f64, f64)>) -> Vec<(usize, usize, f64, f64)> {
    loop {
        if segs.len() < 2 {
            return segs;
        }
        let Some(i) = (0..segs.len()).find(|&i| segs[i].1 - segs[i].0 + 1 < 8) else { return segs };
        let near = |j: usize| (segs[j].2 / segs[i].2).ln().abs();
        let j = if i == 0 {
            1
        } else if i + 1 == segs.len() {
            i - 1
        } else if near(i - 1) <= near(i + 1) {
            i - 1
        } else {
            i + 1
        };
        let (a, b) = (i.min(j), i.max(j));
        // The longer run's tempo is kept; the short one's beats only extend its range.
        let keep = if segs[a].1 - segs[a].0 >= segs[b].1 - segs[b].0 { a } else { b };
        let (start, end) = (segs[a].0, segs[b].1);
        let period = segs[keep].2;
        let t0 = segs[keep].3 - (segs[keep].0 - start) as f64 * period;
        segs[a] = (start, end, period, t0);
        segs.remove(b);
    }
}

/// A run that leaves a tempo and comes back to it exactly in phase was the tracker drifting
/// through a passage without beats: when a later run has the same tempo as an earlier one and
/// starts a whole number of that tempo's beats after it, everything between joins the earlier run.
fn bridge_drift(mut segs: Vec<(usize, usize, f64, f64)>) -> Vec<(usize, usize, f64, f64)> {
    let mut i = 0;
    while i < segs.len() {
        let (_, _, p, t0) = segs[i];
        let mut joined = false;
        for k in (i + 1..segs.len()).rev() {
            let (_, kend, kp, kt0) = segs[k];
            if (kp / p - 1.0).abs() > 0.0015 {
                continue;
            }
            let n = (kt0 - t0) / p;
            if (n - n.round()).abs() * p <= 0.015 {
                segs[i].1 = kend;
                segs.drain(i + 1..=k);
                joined = true;
                break;
            }
        }
        if !joined {
            i += 1;
        }
    }
    segs
}

/// Neighbouring runs that one tempo explains nearly as well (within 1.5 x the tolerance)
/// become one run: tracking jitter must not look like a tempo change.
fn merge_similar(ts: &[f64], mut segs: Vec<(usize, usize, f64, f64)>, tol: f64) -> Vec<(usize, usize, f64, f64)> {
    let mut i = 0;
    while i + 1 < segs.len() {
        let f = fit(ts, segs[i].0, segs[i + 1].1);
        if f.2 <= tol * 1.5 {
            segs[i] = (segs[i].0, segs[i + 1].1, f.0, f.1);
            segs.remove(i + 1);
            i = i.saturating_sub(1);
        } else {
            i += 1;
        }
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kicks at the given times with hats between them.
    fn clicks(times: &[f64], secs: f64, sr: f64) -> Vec<f32> {
        let n = (secs * sr) as usize;
        let mut v = vec![0f32; n];
        for (k, t) in times.iter().enumerate() {
            let start = (t * sr) as usize;
            let amp = if k % 4 == 0 { 1.0 } else { 0.6 };
            for i in 0..(0.08 * sr) as usize {
                if start + i < n {
                    let env = (-(i as f64) / (0.015 * sr)).exp();
                    v[start + i] += (amp * env * (2.0 * std::f64::consts::PI * 60.0 * i as f64 / sr).sin()) as f32;
                }
            }
            if let Some(next) = times.get(k + 1) {
                let hs = ((t + next) / 2.0 * sr) as usize;
                for i in 0..(0.01 * sr) as usize {
                    if hs + i < n {
                        v[hs + i] += ((i * 7919 % 97) as f32 / 97.0 - 0.5) * 0.2;
                    }
                }
            }
        }
        v
    }

    fn worst_error(grid: &BeatGrid, truth: &[f64], secs: f64, skip: f64) -> f64 {
        let beats = grid.beats(secs);
        truth
            .iter()
            .filter(|t| **t > skip && **t < secs - skip)
            .map(|t| crate::model::nearest_beat(&beats, *t).map(|b| (b.time - t).abs()).unwrap_or(9.0))
            .fold(0.0, f64::max)
    }

    #[test]
    fn steady_track_keeps_one_anchor() {
        let sr = 22050.0;
        let p = 60.0 / 126.0;
        let times: Vec<f64> = (0..).map(|k| 0.2 + k as f64 * p).take_while(|t| *t < 60.0).collect();
        let x = clicks(&times, 60.0, sr);
        let (bpm, g) = tempo_and_grid_dynamic(&x, sr, 70.0, 180.0);
        let g = g.unwrap();
        assert_eq!(g.anchors.len(), 1, "anchors {:?}", g.anchors);
        assert!((bpm - 126.0).abs() < 0.05);
    }

    #[test]
    fn tempo_step_gets_a_second_anchor() {
        let sr = 22050.0;
        let mut times = vec![];
        let mut t = 0.25;
        while t < 40.0 {
            times.push(t);
            t += 60.0 / 120.0;
        }
        while t < 80.0 {
            times.push(t);
            t += 60.0 / 132.0;
        }
        let x = clicks(&times, 80.0, sr);
        let (_, g) = tempo_and_grid_dynamic(&x, sr, 70.0, 180.0);
        let g = g.unwrap();
        assert!(g.anchors.len() >= 2, "anchors {:?}", g.anchors);
        assert!((g.anchors[0].bpm - 120.0).abs() < 0.3, "{:?}", g.anchors);
        assert!((g.anchors.last().unwrap().bpm - 132.0).abs() < 0.3, "{:?}", g.anchors);
        let err = worst_error(&g, &times, 80.0, 3.0);
        assert!(err < 0.03, "worst beat error {err}");
    }

    #[test]
    fn precision_keeps_a_tempo_step_and_bar_positions() {
        let sr = 22_050.0;
        let mut t = 0.198;
        let times: Vec<_> = (0..148).map(|i| {
            let beat = t;
            t += 60.0 / if i < 76 { 120.0 } else { 132.0 };
            beat
        }).collect();
        let seconds = t + 0.1;
        let x = clicks(&times, seconds, sr);
        let mut opts = super::super::AnalysisSettings { use_recordbox_estimator: true, dynamic: true, ..Default::default() };
        let normal = super::super::grid_from_mono(&x, sr, &opts).1.unwrap();
        opts.high_precision = true;
        let refined = super::super::grid_from_mono(&x, sr, &opts).1.unwrap();
        assert!(refined.anchors.len() >= 2);
        assert_eq!(normal.anchors.iter().map(|a| a.beat).collect::<Vec<_>>(), refined.anchors.iter().map(|a| a.beat).collect::<Vec<_>>());
        assert!((refined.first_bpm() - 120.0).abs() < 0.3);
        assert!((refined.anchors.last().unwrap().bpm - 132.0).abs() < 0.3);
        let before = worst_error(&normal, &times, seconds, 3.0);
        let after = worst_error(&refined, &times, seconds, 3.0);
        eprintln!("precision tempo step: worst beat error {before:.6} -> {after:.6} seconds");
        assert!(after < 0.002 && after < before, "{before} -> {after}");
    }

    #[test]
    fn tempo_ramp_is_followed() {
        let sr = 22050.0;
        let mut times = vec![];
        let mut t = 0.3;
        while t < 90.0 {
            times.push(t);
            let bpm = 110.0 + 20.0 * (t / 90.0);
            t += 60.0 / bpm;
        }
        let x = clicks(&times, 90.0, sr);
        let (_, g) = tempo_and_grid_dynamic(&x, sr, 70.0, 180.0);
        let g = g.unwrap();
        assert!(g.anchors.len() >= 4, "anchors {}", g.anchors.len());
        let err = worst_error(&g, &times, 90.0, 3.0);
        assert!(err < 0.03, "worst beat error {err}");
    }

    #[test]
    fn segments_split_where_the_tempo_changes() {
        let mut ts: Vec<f64> = (0..32).map(|k| k as f64 * 0.5).collect();
        let last = *ts.last().unwrap();
        ts.extend((1..=32).map(|k| last + k as f64 * 0.4));
        let s = segments(&ts, 0.005);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!((s[0].2 - 0.5).abs() < 1e-6 && (s[1].2 - 0.4).abs() < 1e-6);
        assert_eq!(s[1].0, s[0].1 + 1);
    }
}

#[cfg(test)]
mod real_files {
    /// PREPDECK_GT = tsv whose first column is an audio path: how many anchors does Dynamic
    /// mode give each (steady dance tracks should keep one)?
    #[test]
    #[ignore]
    fn dynamic_anchor_census() {
        let Ok(f) = std::env::var("PREPDECK_GT") else { return };
        let n: usize = std::env::var("PREPDECK_N").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
        let text = std::fs::read_to_string(f).unwrap();
        let (mut one, mut many) = (0, 0);
        for p in text.lines().filter_map(|l| l.split('\t').next()).step_by(5).take(n) {
            let Ok(dec) = crate::audio::decode_file(p) else { continue };
            let mono = dec.mono();
            let (bpm, g) = super::tempo_and_grid_dynamic(&mono, dec.sr as f64, 70.0, 180.0);
            let a = g.map(|g| g.anchors).unwrap_or_default();
            if a.len() <= 1 {
                one += 1;
            } else {
                many += 1;
                eprintln!("{} anchors, first bpm {bpm:.2}: {:?} :: {p}", a.len(), a.iter().map(|x| (x.time as i64, x.bpm)).take(8).collect::<Vec<_>>());
            }
        }
        eprintln!("one tempo: {one}, several: {many}");
    }
}
