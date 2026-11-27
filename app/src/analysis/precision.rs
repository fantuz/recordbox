//! Refine an existing beat lattice against short attacks in the decoded audio.
//! Only a consistent, well-distributed set of attacks can move a tempo segment.

use crate::model::{BeatGrid, GridAnchor};

const MAX_OBSERVATIONS: usize = 128;
const SEARCH_SECONDS: f64 = 0.040;
const INLIER_SECONDS: f64 = 0.004;

fn median(values: &mut [f64]) -> f64 {
    values.sort_unstable_by(f64::total_cmp);
    let middle = values.len() / 2;
    (values[(values.len() - 1) / 2] + values[middle]) * 0.5
}

// Compare the energy ahead of each half-millisecond block with the energy
// behind it. Backtrack from the steep rise to the start of the audible attack.
fn attack(mono: &[f32], sr: f64, predicted: f64) -> Option<f64> {
    let block = (sr / 2000.0).round().max(1.0) as usize;
    let start = ((predicted - SEARCH_SECONDS).max(0.0) * sr).round() as usize;
    let end = (((predicted + SEARCH_SECONDS) * sr).round() as usize).min(mono.len());
    let samples = mono.get(start..end)?;
    let mut energy = [0.0_f64; 192];
    let mut sum = [0.0_f64; 193];
    let n = (samples.len() / block).min(energy.len());
    for (i, chunk) in samples.chunks_exact(block).take(n).enumerate() {
        energy[i] = chunk.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / block as f64;
        sum[i + 1] = sum[i] + energy[i];
    }
    const W: usize = 8;
    let (mut best, mut score, mut floor) = (0, 0.0, 0.0);
    for i in W..n.saturating_sub(W) {
        let before = (sum[i] - sum[i - W]) / W as f64;
        let after = (sum[i + W] - sum[i]) / W as f64;
        if after > 4.0 * before + 1e-9 && after - before > score {
            best = i;
            score = after - before;
            floor = before + 0.02 * score;
        }
    }
    if score == 0.0 { return None; }
    let mut onset = best;
    while onset > best - W && energy[onset - 1] > floor { onset -= 1; }
    while onset < best + W && energy[onset] <= floor { onset += 1; }
    Some((start + onset * block) as f64 / sr)
}

fn fit_segment(mono: &[f32], sr: f64, anchor: GridAnchor, end: f64, bpm_range: (f64, f64)) -> Option<GridAnchor> {
    let period = 60.0 / anchor.bpm;
    let beats = ((end - anchor.time) / period).ceil() as usize;
    let count = beats.min(MAX_OBSERVATIONS);
    if count < 8 { return None; }
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let beat = (i * (beats - 1) / (count - 1)) as f64;
        let predicted = anchor.time + beat * period;
        if let Some(time) = attack(mono, sr, predicted) {
            points.push((beat, time - predicted));
        }
    }
    if points.len() < 8 || points.len() * 2 < count { return None; }

    // A median slope between distant beats tolerates stray hats, missed kicks
    // and breakdowns. Do not extrapolate a correction fitted to a short intro.
    let span = (beats - 1) as f64;
    let mut slopes = Vec::new();
    for (i, &(x, y)) in points.iter().enumerate() {
        for &(other_x, other_y) in &points[i + 1..] {
            if other_x - x >= span * 0.5 { slopes.push((other_y - y) / (other_x - x)); }
        }
    }
    if slopes.is_empty() { return None; }
    let slope = median(&mut slopes);
    let mut offsets: Vec<_> = points.iter().map(|&(x, y)| y - slope * x).collect();
    let offset = median(&mut offsets);
    let n = points.len();
    points.retain(|&(x, y)| (y - offset - slope * x).abs() <= INLIER_SECONDS);
    if points.len() < 8 || points.len() * 3 < n * 2 || points.last()?.0 - points.first()?.0 < span * 0.5 { return None; }
    let bpm = 60.0 / (period + slope);
    if !(bpm_range.0..=bpm_range.1).contains(&bpm) || offset.abs().max((offset + slope * span).abs()) > SEARCH_SECONDS {
        return None;
    }
    Some(GridAnchor { time: (anchor.time + offset).max(0.0), bpm, beat: anchor.beat })
}

pub(super) fn refine(mono: &[f32], sr: f64, grid: &mut BeatGrid, bpm_min: f64, bpm_max: f64) {
    let duration = mono.len() as f64 / sr;
    let mut fitted = grid.anchors.clone();
    for (i, &anchor) in grid.anchors.iter().enumerate() {
        let end = grid.anchors.get(i + 1).map_or(duration, |next| next.time);
        if let Some(new) = fit_segment(mono, sr, anchor, end, (bpm_min, bpm_max)) {
            fitted[i] = new;
        }
    }
    // BeatGrid::beats gives a beat within half a period of the next anchor
    // to that anchor. Preserve the number of beats and bar phase at each join.
    let interval_beats = |w: &[GridAnchor]| ((w[1].time - w[0].time) * w[0].bpm / 60.0 - 0.5).ceil();
    if fitted.windows(2).zip(grid.anchors.windows(2)).all(|(new, old)| new[1].time > new[0].time && interval_beats(new) == interval_beats(old)) {
        grid.anchors = fitted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_or_isolated_attacks_do_not_move_an_established_grid() {
        let sr = 8000.0;
        let mut audio = vec![0.0; (sr * 20.0) as usize];
        let original = BeatGrid { anchors: vec![
            GridAnchor { time: 0.031, bpm: 120.0, beat: 3 },
            GridAnchor { time: 10.031, bpm: 128.0, beat: 3 }
        ] };
        for moment in [2.04, 5.04, 12.39] {
            let i = (moment * sr) as usize;
            audio[i..i + 12].fill(0.5);
        }
        let mut grid = original.clone();
        refine(&audio, sr, &mut grid, 70.0, 180.0);
        assert_eq!(grid, original);
        let silence = vec![0.0; audio.len()];
        let settings = super::super::AnalysisSettings { use_recordbox_estimator: true, high_precision: true, ..Default::default() };
        assert!(super::super::grid_from_mono(&silence, sr, &settings).1.is_none());
    }
}
