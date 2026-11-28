//! The tempo-change path of rekordbox's high precision BeatGrid analysis (rekordbox 7.2.19):
//! what `AnalyzeJobBeatDetector::highPrecisionBeatAnalyze` does when the tempo may change
//! (analysis mode Dynamic, or Auto when `BeatAnalyzer_2_0` judged the tempo varied).
//!
//! 1. `madmom::analyze` (PCM version, 0x102307f8c) runs the downbeat network `model7` over the
//!    whole track in pieces: whole minutes, then the last minute and the rest (split in two
//!    halves when the rest is shorter than 30 s); each piece is decoded by the DBN on its own and
//!    the pieces are joined (`network_beats`);
//! 2. the file version (0x10230a820) scales the times to 44.1 kHz and repairs the beat list at
//!    every seam between pieces and at single gaps (`seam_repair`), then `get_beat_list` /
//!    `get_bpm` as on the constant path.

use super::dbn::Dbn;
use super::net::Net;

/// `v84 = (float)sr * 0.0013605f` (60 / 44100): the start of whole minute `i` in the network's
/// time base (seconds at 44.1 kHz) is `v84 * i`.
const MINUTE_44K: f32 = f32::from_bits(0x3ab2544a);
/// `(float)start * 0.000022676f` (1 / 44100): the start of the last two pieces.
const SAMPLE_44K: f32 = f32::from_bits(0x37be37c6);

/// The pieces the PCM version cuts `n` frames into: (start, length) in frames, and whether the
/// piece is joined at the beat nearest to the previous piece's last beat (the last two pieces)
/// or appended whole (the whole minutes before them).
fn pieces(frames: usize, sr: usize) -> Vec<(usize, usize, bool)> {
    let minute = 60 * sr;
    let minutes = frames / minute;
    if minutes == 0 {
        return if frames == 0 { Vec::new() } else { vec![(0, frames, true)] };
    }
    let mut out: Vec<_> = (0..minutes - 1).map(|i| (i * minute, minute, false)).collect();
    let start = (minutes - 1) * minute;
    let tail = frames - start;
    let remainder = tail - minute;
    let split = if remainder > 0 && remainder < minute / 2 { tail / 2 } else { minute };
    out.push((start, split, true));
    if tail > split {
        out.push((start + split, tail - split, true));
    }
    out
}

/// Join the two tail windows after the beat nearest the previous window's last beat.
fn join(acc: &mut Vec<(f32, i32)>, mut beats: Vec<(f32, i32)>, offset: f32, nearest: bool) {
    for beat in &mut beats {
        beat.0 += offset;
    }
    let skip = if let Some(&(last, _)) = acc.last().filter(|_| nearest) {
        let mut best = 0;
        while best + 1 < beats.len() && (last - beats[best + 1].0).abs() < (last - beats[best].0).abs() {
            best += 1;
        }
        best + 1
    } else {
        0
    };
    acc.extend(beats.into_iter().skip(skip));
}

fn network_beats(net: &Net, dbn: &Dbn, samples: &[f32], sr: u32, len: usize, cancelled: &dyn Fn() -> bool) -> Option<Vec<(f32, i32)>> {
    let mut acc = Vec::new();
    for (start, count, nearest) in pieces(len, sr as usize) {
        if cancelled() { return None; }
        // Both the network activations and the DBN backtrace live for one piece only.
        let activations = {
            let mono: Vec<_> = samples[2 * start..2 * (start + count)].chunks_exact(2)
                .map(|lr| (lr[1] + lr[0]) * 0.5).collect();
            if cancelled() { return None; }
            net.run_cancellable(&mono, cancelled)?
        };
        if cancelled() { return None; }
        let offset = if nearest {
            start as f32 * SAMPLE_44K
        } else {
            (sr as f32 * MINUTE_44K) * (start / (60 * sr as usize)) as f32
        };
        let beats = dbn.track(&activations);
        if cancelled() { return None; }
        join(&mut acc, beats, offset, nearest);
    }
    Some(acc)
}

const EPS: f32 = f32::from_bits(0x34000000);

/// One seam: the first beat at or after `seam` (seconds) and its neighbours. Beats missing at
/// the seam (one interval 1.35-3.5 times the one before, the next 0.25-0.75 times it) are
/// filled in evenly, a missing beat right after it likewise, a beat closer than 0.3 intervals
/// to the one before it is removed (decompile lines ~540-700 and 740-940 of 0x10230a820).
fn repair_at(b: &mut Vec<(f32, i32)>, seam: f32) {
    if b.is_empty() {
        return;
    }
    let count = b.len();
    let v105 = count.max(1);
    let mut v104 = 0usize;
    while b[v104].0 < seam {
        v104 += 1;
        if v105 == v104 {
            return;
        }
    }
    if v104 < 2 || count <= v104 + 1 {
        return;
    }
    let v111 = b[v104 - 1].0;
    let v112 = b[v104].0;
    let v113 = b[v104 + 1].0;
    let v114 = v111 - b[v104 - 2].0;
    let v115 = v112 - v111;
    let v116 = if v114 <= EPS { 0.0 } else { v115 / v114 };
    let v117 = if v115 <= EPS { 0.0 } else { (v113 - v112) / v115 };
    let v118 = (v116 + 0.5) as i32;
    if v118 >= 2 && v116 >= 1.35 && v116 <= 3.5 && v117 >= 0.25 && v117 <= 0.75 {
        let inv = 1.0f32 / v118 as f32;
        for j in 1..v118 {
            b.insert(v104 + (j - 1) as usize, ((v115 * j as f32).mul_add(inv, v111), 1));
        }
        return;
    }
    let v123 = (v117 + 0.5) as i32;
    if v123 >= 2 && v116 >= 0.9 && v116 <= 1.1 && v117 >= 1.35 && v117 <= 3.5 {
        let v127 = v113 - v112;
        let inv = 1.0f32 / v123 as f32;
        for k in 1..v123 {
            b.insert(v104 + k as usize, ((v127 * k as f32).mul_add(inv, v112), 1));
        }
        return;
    }
    if v116 >= 0.3 {
        if v117 >= 0.3 {
            return;
        }
        b.remove(v104 + 1);
    } else {
        b.remove(v104);
    }
}

/// Split a long interval evenly, with the same f32 rounding as the native reader.
/// Move the remainder of the beat list once per gap. Decline an implausible expansion
/// before resizing the list (almost coincident beats can produce very large ratios).
fn split_gap(beats: &mut Vec<(f32, i32)>, index: usize, steps: usize, max_beats: usize) -> Option<()> {
    let extra = steps.checked_sub(1)?;
    if beats.len().checked_add(extra)? > max_beats { return None; }
    let start = beats[index].0;
    let gap = beats[index + 1].0 - start;
    let inv = 1.0f32 / steps as f32;
    beats.splice(index + 1..index + 1, (1..steps).map(|j| ((gap * j as f32).mul_add(inv, start), 1)));
    Some(())
}

/// Repair piece boundaries, then split gaps longer than 1.8 times both neighbours
/// (only the preceding neighbour for the last gap). The native pass loops over
/// the indices the list had before insertion.
fn seam_repair(beats: &mut Vec<(f32, i32)>, len: usize, sr: u32, max_beats: usize) -> Option<()> {
    if beats.len() > max_beats { return None; }
    let minute = 60 * sr as usize;
    let minutes = len / minute;
    for i in 1..minutes {
        repair_at(beats, i as f32 * 60.0);
    }
    if let Some(&(last_start, last_len, _)) = pieces(len, sr as usize).last() {
        // On a partial final minute the native code checks the start of the last
        // piece. With an exact whole minute, or just one piece, it checks the end.
        let end = if minutes == 0 || len % minute == 0 { last_start + last_len } else { last_start };
        repair_at(beats, end as f32 / sr as f32);
    }
    if beats.len() > max_beats { return None; }
    let original = beats.len();
    for i in 1..original.saturating_sub(1) {
        let gap = beats[i + 1].0 - beats[i].0;
        let previous = beats[i].0 - beats[i - 1].0;
        if previous <= EPS { continue; }
        let ratio = gap / previous;
        if ratio <= 1.8 { continue; }
        if i + 2 < original {
            let next = beats[i + 2].0 - beats[i + 1].0;
            if next <= EPS || gap / next <= 1.8 { continue; }
        }
        let steps = (ratio + 0.5) as usize;
        if steps >= 2 { split_gap(beats, i, steps, max_beats)?; }
    }
    Some(())
}

/// The trained DBN spans 70–180 BPM. Allow up to eight beat observations a
/// second for seam repairs, while bounding any extrapolated lead-in or tail.
fn plausible_beats(beats: &[(f32, i32)], len: usize, sr: u32, max_beats: usize) -> bool {
    if beats.len() < 2 { return false; }
    let duration = len as f64 / sr as f64;
    if beats.iter().any(|&(time, bar)| !time.is_finite() || time < 0.0 || time as f64 > duration ||
        !(1..=4).contains(&bar)) { return false; }
    // get_beat_list rounds to milliseconds before extending the tail. Coincident
    // rounded beats would give a zero step. Only the end intervals are used
    // for extrapolation, so short interior beats need no artificial BPM limit.
    let millis = |t: f32| (t * 1000.0).round() as i64;
    if beats.windows(2).any(|pair| millis(pair[0].0) >= millis(pair[1].0)) { return false; }
    let first = beats[0].0;
    let step = beats[1].0 - first;
    if first >= step && step < 0.001 { return false; }
    let leading = (first / step).floor() as usize;
    let sample = |t| ((millis(t) * i64::from(sr)) as f32 * 0.001) as usize;
    let last = sample(beats[beats.len() - 1].0);
    let previous = sample(beats[beats.len() - 2].0);
    if last <= previous { return false; }
    let trailing = len.saturating_sub(last).saturating_sub(1) / (last - previous);
    leading.saturating_add(beats.len()).saturating_add(trailing) <= max_beats
}

fn admissible_frames(sr: u32, total: usize, stereo_samples: usize) -> bool {
    matches!(sr, 44100 | 48000) && total >= 2 && total <= 900 * sr as usize &&
        total.checked_mul(2) == Some(stereo_samples)
}

/// Model7 runs sequentially in pieces of at most 60 seconds. The PCM is
/// interleaved stereo at the analysis rate, and the native reader omits the last frame.
pub fn madmom_tempo(net: &Net, dbn: &Dbn, samples: &[f32], sr: u32, total: usize, cancelled: &dyn Fn() -> bool) -> Option<(Vec<super::dbn::ListBeat>, i32)> {
    if cancelled() || !admissible_frames(sr, total, samples.len()) { return None; }
    let len = total - 1;
    let beats = network_beats(net, dbn, samples, sr, len, cancelled)?;
    finish_beats(beats, sr, len, cancelled)
}

fn finish_beats(mut beats: Vec<(f32, i32)>, sr: u32, len: usize, cancelled: &dyn Fn() -> bool) -> Option<(Vec<super::dbn::ListBeat>, i32)> {
    if cancelled() { return None; }
    super::scale_times(&mut beats, sr as i32);
    // Raw pieces can produce close neighbours at a seam. Check chronology only
    // here so the repair can remove those beats before extrapolating either end.
    if beats.iter().any(|b| !b.0.is_finite()) || beats.windows(2).any(|b| b[0].0 > b[1].0) { return None; }
    let limit = 8 * len.div_ceil(sr as usize) + 2;
    seam_repair(&mut beats, len, sr, limit)?;
    if cancelled() || !plausible_beats(&beats, len, sr, limit) { return None; }
    let list = super::dbn::get_beat_list(&beats, len as i64, sr as i32, 4);
    let bpm = super::dbn::get_bpm(&list);
    (!cancelled() && list.len() <= limit && list.windows(2).all(|b| b[1].pos > b[0].pos) &&
        bpm.is_finite() && bpm > 0.0).then_some((list, bpm as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_follow_the_minute_layout() {
        let sr = 44100usize;
        let m = 60 * sr;
        // under a minute: one piece
        assert_eq!(pieces(30 * sr, sr), vec![(0, 30 * sr, true)]);
        // 150 s: two whole minutes... the second is the last minute, the 30 s rest its own piece
        assert_eq!(pieces(150 * sr, sr), vec![(0, m, false), (m, m, true), (2 * m, 30 * sr, true)]);
        // 200 s: the last minute and the 20 s rest are split in two halves of 40 s
        assert_eq!(pieces(200 * sr, sr), vec![(0, m, false), (m, m, false), (2 * m, 40 * sr, true), (2 * m + 40 * sr, 40 * sr, true)]);
        // exactly three minutes: no rest piece
        assert_eq!(pieces(180 * sr, sr), vec![(0, m, false), (m, m, false), (2 * m, m, true)]);
    }

    #[test]
    fn seam_repair_fills_a_missing_beat_at_a_minute() {
        // 0.5 s beats with the one at 60.0 s missing (the next is at 60.5 s)
        let mut b: Vec<(f32, i32)> = (0..130).map(|k| k as f32 * 0.5).filter(|t| *t != 60.0).map(|t| (t, 1)).collect();
        let n = 66 * 44100;
        seam_repair(&mut b, n, 44100, 528).unwrap();
        assert!(b.iter().any(|x| (x.0 - 60.0).abs() < 1e-4), "beat at the seam not filled");
        assert!(b.windows(2).all(|w| (w[1].0 - w[0].0 - 0.5).abs() < 1e-3));
    }

    #[test]
    fn chunks_cover_both_rates_and_the_join_keeps_native_tie_order() {
        for sr in [44100usize, 48000] {
            let minute = 60 * sr;
            for len in [1, minute - 1, minute, minute + 1, minute + minute / 2 - 1, minute + minute / 2, 15 * minute] {
                let mut consumed = 0;
                for (start, count, _) in pieces(len, sr) {
                    assert_eq!(start, consumed);
                    assert!((1..=minute).contains(&count));
                    consumed += count;
                }
                assert_eq!(consumed, len);
            }
        }
        let mut beats = vec![(3.0, 1)];
        join(&mut beats, vec![], 2.0, true);
        join(&mut beats, vec![(0.5, 2), (1.5, 3), (2.0, 4)], 2.0, true);
        assert_eq!(beats, [(3.0, 1), (3.5, 3), (4.0, 4)]);
    }

    #[test]
    fn cancellation_discards_a_finished_window_list_before_publication() {
        let beats = vec![(0.0, 1), (0.5, 2), (1.0, 3), (1.5, 4)];
        assert!(finish_beats(beats.clone(), 44100, 88200, &|| false).is_some());
        let checkpoints = std::cell::Cell::new(0);
        let cancel_at_publication = || {
            checkpoints.set(checkpoints.get() + 1);
            checkpoints.get() == 3
        };
        assert!(finish_beats(beats, 44100, 88200, &cancel_at_publication).is_none());
        assert_eq!(checkpoints.get(), 3);
    }

    #[test]
    fn invalid_buffers_and_collapsed_or_excessively_dense_beats_are_declined() {
        assert!(!admissible_frames(0, 1, 2));
        assert!(!admissible_frames(44100, 2, 3));
        assert!(!admissible_frames(48000, 48000 * 901, 2 * 48000 * 901));
        assert!(admissible_frames(48000, 48000 * 900, 2 * 48000 * 900));
        assert!(!plausible_beats(&[(0.0,1), (0.0001,2)], 48000, 48000, 10));
        assert!(!plausible_beats(&[(0.0,1), (f32::NAN,2)], 48000, 48000, 10));
        assert!(!plausible_beats(&[(0.5,1), (0.501,2)], 48000, 48000, 10));
        assert!(plausible_beats(&[(0.0,1), (0.5,2), (0.55,3), (1.0,4)], 48000, 48000, 10));
        let mut pathological = vec![(0.0, 1), (0.0001, 2), (10.0, 3)];
        assert!(seam_repair(&mut pathological, 480000, 48000, 82).is_none());
        assert_eq!(pathological.len(), 3);
    }
}
