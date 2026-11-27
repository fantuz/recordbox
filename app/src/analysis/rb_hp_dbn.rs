//! Bar-pointer DBN of rekordbox's high precision beat analysis (`madmom::downbeat_tracking`
//! 0x102300d6c, a C++ port of madmom's `DBNDownBeatTrackingProcessor`), and the two helpers that
//! turn its beats into the list `Analyzer::analyzeFocus` reads (`madmom::get_beat_list`
//! 0x102305908, `madmom::get_bpm` 0x102306b34).
//!
//! rekordbox calls it with beats per bar [4], 60 tempi, transition lambda 100, observation
//! lambda 16, peak correction on, 70-180 BPM, threshold 0.05, 100 frames per second. With these
//! values the tempo states are every whole interval from 33 to 86 frames (54 tempi, 12852
//! states). The transition table rekordbox uses is precomputed (`madmom::param_states`,
//! `param_pointers`, `param_probabilities`, copied in by `TransitionModel::make_sparse`); it is
//! madmom's `BarTransitionModel` in compressed-row form, which is rebuilt here from the same
//! definition.

/// rekordbox's parameters for `downbeat_tracking`.
pub const BEATS_PER_BAR: usize = 4;
pub const MIN_BPM: f64 = 70.0;
pub const MAX_BPM: f64 = 180.0;
pub const NUM_TEMPI: usize = 60;
pub const TRANSITION_LAMBDA: f64 = 100.0;
pub const OBSERVATION_LAMBDA: u32 = 16;
pub const THRESHOLD: f32 = 0.05;
pub const FPS: f64 = 100.0;

#[allow(dead_code)]
pub struct Dbn {
    pub intervals: Vec<u32>,
    pub num_states: usize,
    /// Bar position of every state (beat index + fraction of the beat).
    pub positions: Vec<f64>,
    /// 0 = no beat, 1 = beat, 2 = downbeat (column of the log densities).
    pub om_pointers: Vec<u8>,
    /// Compressed rows: transitions into state s are tm_from[tm_ptr[s]..tm_ptr[s + 1]].
    pub tm_ptr: Vec<u32>,
    pub tm_from: Vec<u32>,
    pub tm_prob: Vec<f64>,
}

/// madmom's `BeatStateSpace` intervals.
fn beat_intervals(min_interval: f64, max_interval: f64, num_intervals: usize) -> Vec<u32> {
    let lo = min_interval.round();
    let hi = max_interval.round();
    let mut iv: Vec<f64> = (0..).map(|k| lo + k as f64).take_while(|&x| x < hi + 1.0).collect();
    if num_intervals > 0 && num_intervals < iv.len() {
        let mut n = num_intervals;
        loop {
            let (a, b) = (min_interval.log2(), max_interval.log2());
            let step = if n > 1 { (b - a) / (n - 1) as f64 } else { 0.0 };
            let mut v: Vec<f64> = (0..n).map(|k| (a + k as f64 * step).exp2().round()).collect();
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            v.dedup();
            iv = v;
            if iv.len() >= num_intervals {
                break;
            }
            n += 1;
        }
    }
    iv.into_iter().map(|x| x as u32).collect()
}

impl Dbn {
    pub fn new(beats: usize, min_bpm: f64, max_bpm: f64, num_tempi: usize, transition_lambda: f64, observation_lambda: u32, fps: f64) -> Dbn {
        let min_interval = fps * 60.0 / max_bpm;
        let max_interval = fps * 60.0 / min_bpm;
        let intervals = beat_intervals(min_interval, max_interval, num_tempi);
        let per_beat: usize = intervals.iter().map(|&i| i as usize).sum();
        let num_states = per_beat * beats;
        let mut positions = Vec::with_capacity(num_states);
        let mut state_interval = Vec::with_capacity(num_states);
        // first / last state of every interval, per beat
        let mut first = vec![vec![0u32; intervals.len()]; beats];
        let mut last = vec![vec![0u32; intervals.len()]; beats];
        for b in 0..beats {
            let base = b * per_beat;
            let mut idx = 0usize;
            for (k, &i) in intervals.iter().enumerate() {
                first[b][k] = (base + idx) as u32;
                let step = 1.0 / i as f64;
                for j in 0..i {
                    positions.push(j as f64 * step + b as f64);
                    state_interval.push(i);
                }
                idx += i as usize;
                last[b][k] = (base + idx - 1) as u32;
            }
        }
        let border = 1.0 / observation_lambda as f64;
        let om_pointers = positions.iter().map(|&p| if p < border { 2 } else if p % 1.0 < border { 1 } else { 0 }).collect();
        // Tempo change probabilities between the last state of beat b-1 and the first of b.
        let n = intervals.len();
        let mut prob = vec![0f64; n * n];
        for f in 0..n {
            for t in 0..n {
                let ratio = intervals[t] as f64 / intervals[f] as f64;
                let p = ((ratio + -1.0).abs() * -transition_lambda).exp();
                prob[f * n + t] = if p <= f64::EPSILON { 0.0 } else { p };
            }
            let row = &mut prob[f * n..(f + 1) * n];
            let s = pairwise_sum(row);
            for p in row.iter_mut() {
                *p /= s;
            }
        }
        let mut is_first = vec![usize::MAX; num_states];
        for b in 0..beats {
            for k in 0..n {
                is_first[first[b][k] as usize] = b * n + k;
            }
        }
        let mut tm_ptr = Vec::with_capacity(num_states + 1);
        let mut tm_from = vec![];
        let mut tm_prob = vec![];
        for s in 0..num_states {
            tm_ptr.push(tm_from.len() as u32);
            if is_first[s] == usize::MAX {
                tm_from.push(s as u32 - 1);
                tm_prob.push(1.0);
            } else {
                let (b, t) = (is_first[s] / n, is_first[s] % n);
                let pb = (b + beats - 1) % beats;
                for f in 0..n {
                    let p = prob[f * n + t];
                    if p != 0.0 {
                        tm_from.push(last[pb][f]);
                        tm_prob.push(p);
                    }
                }
            }
        }
        tm_ptr.push(tm_from.len() as u32);
        let _ = state_interval;
        Dbn { intervals, num_states, positions, om_pointers, tm_ptr, tm_from, tm_prob }
    }

    pub fn rekordbox() -> Dbn {
        Dbn::new(BEATS_PER_BAR, MIN_BPM, MAX_BPM, NUM_TEMPI, TRANSITION_LAMBDA, OBSERVATION_LAMBDA, FPS)
    }

    /// `HiddenMarkovModel::viterbi` (0x1023035c4): the most likely state per frame.
    ///
    /// Same arithmetic as the original (`(prev + density) + log p`, first maximum kept), but
    /// only the first state of each tempo in each beat has more than one predecessor; every
    /// other state comes from the state before it with probability 1, so only the choices of
    /// those 4 x 54 states are stored per frame.
    pub fn viterbi(&self, act: &[[f32; 2]]) -> Vec<u32> {
        let ns = self.num_states;
        let nf = act.len();
        let lam = OBSERVATION_LAMBDA as f32;
        let inv = 1.0f32 / (lam + -1.0);
        let dens: Vec<[f64; 3]> = act.iter().map(|a| [((1.0 - (a[0] + a[1])) * inv).ln() as f64, a[0].ln() as f64, a[1].ln() as f64]).collect();
        let log_tp: Vec<f64> = self.tm_prob.iter().map(|p| p.ln()).collect();
        // states with several predecessors, in order, and their slot in the per-frame record
        let multi: Vec<usize> = (0..ns).filter(|&s| self.tm_ptr[s + 1] - self.tm_ptr[s] != 1 || self.tm_from[self.tm_ptr[s] as usize] as usize + 1 != s).collect();
        let mut slot = vec![u32::MAX; ns];
        for (k, &s) in multi.iter().enumerate() {
            slot[s] = k as u32;
        }
        let om: Vec<usize> = self.om_pointers.iter().map(|&p| p as usize).collect();
        let mut prev = vec![(1.0 / ns as f64).ln(); ns];
        let mut cur = vec![f64::NEG_INFINITY; ns];
        let mut bt = vec![0u32; nf * multi.len()];
        for (fr, d) in dens.iter().enumerate() {
            let row = &mut bt[fr * multi.len()..(fr + 1) * multi.len()];
            for s in 0..ns {
                let density = d[om[s]];
                let k = slot[s];
                if k == u32::MAX {
                    // single predecessor s - 1, log p = 0
                    let p = self.tm_ptr[s] as usize;
                    let v = prev[s - 1] + density + log_tp[p];
                    cur[s] = if v > f64::NEG_INFINITY { v } else { f64::NEG_INFINITY };
                    continue;
                }
                let mut best = f64::NEG_INFINITY;
                let mut arg = 0u32;
                for p in self.tm_ptr[s] as usize..self.tm_ptr[s + 1] as usize {
                    let from = self.tm_from[p];
                    let v = prev[from as usize] + density + log_tp[p];
                    if v > best {
                        best = v;
                        arg = from;
                    }
                }
                cur[s] = best;
                row[k as usize] = arg;
            }
            std::mem::swap(&mut prev, &mut cur);
        }
        // prev now holds the last frame
        let mut state = 0usize;
        for s in 1..ns {
            if prev[s] > prev[state] {
                state = s;
            }
        }
        let mut path = vec![0u32; nf];
        for fr in (0..nf).rev() {
            path[fr] = state as u32;
            let k = slot[state];
            state = if k == u32::MAX { state - 1 } else { bt[fr * multi.len() + k as usize] as usize };
        }
        path
    }

    /// `madmom::downbeat_tracking`: (time in seconds, beat number 1..4) of every beat.
    pub fn track(&self, act: &[[f32; 2]]) -> Vec<(f32, i32)> {
        // threshold_activations: keep the frames from the first to the last one with an
        // activation >= threshold.
        let mut first = 0usize;
        let mut last = 0usize;
        let above: Vec<usize> = act.iter().enumerate().filter(|(_, a)| a[0] >= THRESHOLD || a[1] >= THRESHOLD).map(|(i, _)| i).collect();
        if let (Some(&a), Some(&b)) = (above.first(), above.last()) {
            first = a;
            last = (b + 1).min(act.len());
        }
        let act = &act[first..last];
        if !act.iter().any(|a| a[0] != 0.0 || a[1] != 0.0) {
            return vec![];
        }
        let path = self.viterbi(act);
        let beat_numbers: Vec<i32> = path.iter().map(|&s| self.positions[s as usize] as i32 + 1).collect();
        let beat_range: Vec<bool> = path.iter().map(|&s| self.om_pointers[s as usize] >= 1).collect();
        let mut idx: Vec<usize> = vec![];
        for i in 1..beat_range.len() {
            if beat_range[i] != beat_range[i - 1] {
                idx.push(i);
            }
        }
        if beat_range.first() == Some(&true) {
            idx.insert(0, 0);
        }
        if beat_range.last() == Some(&true) {
            idx.push(beat_range.len());
        }
        let mut out = vec![];
        for pair in idx.chunks_exact(2) {
            let (l, r) = (pair[0], pair[1]);
            // argmax over the flattened [l, r) x 2 block, first maximum, // 2
            let mut best = (f32::NEG_INFINITY, 0usize);
            for (k, a) in act[l..r].iter().enumerate() {
                for (c, &v) in a.iter().enumerate() {
                    if v > best.0 {
                        best = (v, 2 * k + c);
                    }
                }
            }
            let peak = l + best.1 / 2;
            out.push((((peak + first) as f64 * (1.0 / FPS)) as f32, beat_numbers[peak]));
        }
        out
    }
}

/// numpy's pairwise summation (the probabilities were normalised with `np.sum` in madmom).
fn pairwise_sum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        let mut s = 0.0;
        for &x in a {
            s += x;
        }
        s
    } else if n <= 128 {
        let mut r = [0f64; 8];
        r.copy_from_slice(&a[..8]);
        let mut i = 8;
        while i + 8 <= n {
            for k in 0..8 {
                r[k] += a[i + k];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        res
    } else {
        let mut n2 = n / 2;
        n2 -= n2 % 8;
        pairwise_sum(&a[..n2]) + pairwise_sum(&a[n2..])
    }
}

/// One entry of the list `madmom::get_beat_list` builds (a JSON array in rekordbox).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListBeat {
    pub index: i64,
    pub bpm: i32,
    pub pos: i64,
    pub msec: i64,
    pub num_bar: i32,
}

/// `madmom::get_beat_list(beats, n_samples, sr, 4)`.
pub fn get_beat_list(beats: &[(f32, i32)], n_samples: i64, sr: i32, beats_per_bar: i32) -> Vec<ListBeat> {
    let mut v: Vec<(f32, i32)> = beats.to_vec();
    if v.len() >= 2 {
        let d = v[0].0 - v[1].0;
        let mut t = v[0].0 + d;
        let mut num = v[0].1;
        while t >= 0.0 {
            num = if num >= 2 { num - 1 } else { beats_per_bar };
            v.insert(0, (t, num));
            t += d;
        }
    }
    let n = v.len();
    let mut out: Vec<ListBeat> = Vec::with_capacity(n);
    for i in 0..n {
        let num = v[i].1;
        let msec = (v[i].0 * 1000.0f32).round() as i64;
        let mut b = 0f32;
        if i != 0 {
            let prev = out[i - 1].msec;
            b = 60000.0f32 / (msec - prev) as f32;
            out[i - 1].bpm = (b * 100.0f32) as i32;
            if i == n - 1 {
                b = out[i - 1].bpm as f32;
            }
        }
        let pos = ((((msec * sr as i64) as f32) * 0.001f32) as i32) as i64;
        out.push(ListBeat { index: i as i64, bpm: b as i32, pos, msec, num_bar: num });
    }
    if out.len() > 1 {
        let lastb = out[out.len() - 1];
        let step = lastb.pos - out[out.len() - 2].pos;
        let mut p = lastb.pos + step;
        let mut index = lastb.index + 1;
        let mut num = lastb.num_bar;
        while p < n_samples {
            out.push(ListBeat {
                index,
                bpm: lastb.bpm,
                pos: p,
                msec: (p as f64 * 100.0 * (1.0 / sr as f64)).round() as i64,
                num_bar: num % beats_per_bar + 1,
            });
            index += 1;
            num += 1;
            p += step;
        }
    }
    out
}

/// `madmom::get_bpm(list, true)`: median ("linear" percentile 50) of the bpm values.
pub fn get_bpm(list: &[ListBeat]) -> f64 {
    if list.is_empty() {
        return 0.0;
    }
    let mut v: Vec<i32> = list.iter().map(|b| b.bpm).collect();
    v.sort();
    let pos = 0.5 * (v.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - lo as f64;
    v[lo] as f64 + (v[hi] as f64 - v[lo] as f64) * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_space_size() {
        let d = Dbn::rekordbox();
        assert_eq!(d.intervals.first(), Some(&33));
        assert_eq!(d.intervals.last(), Some(&86));
        assert_eq!(d.intervals.len(), 54);
        assert_eq!(d.num_states, 12852);
        assert_eq!(d.tm_ptr.len(), 12853);
    }

    /// A clean 120 BPM pulse (every 50 frames, downbeat every 4th) comes back on the same frames.
    #[test]
    fn tracks_a_synthetic_pulse() {
        let d = Dbn::rekordbox();
        let n = 1000;
        let act: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                if i % 50 == 10 {
                    if (i / 50) % 4 == 0 { [0.05, 0.9] } else { [0.9, 0.05] }
                } else {
                    [0.01, 0.005]
                }
            })
            .collect();
        let beats = d.track(&act);
        assert!(beats.len() >= 18, "{beats:?}");
        for (t, num) in &beats {
            let f = (t * 100.0).round() as i32;
            assert_eq!(f % 50, 10, "{beats:?}");
            assert_eq!(*num, ((f / 50) % 4 + 1) as i32, "{beats:?}");
        }
    }

    #[test]
    fn beat_list_extends_both_ends() {
        let beats = [(0.6f32, 2), (1.1, 3), (1.6, 4)];
        let l = get_beat_list(&beats, 44100 * 3, 44100, 4);
        assert_eq!(l[0].msec, 100);
        assert_eq!(l[0].num_bar, 1);
        assert_eq!(l[1].msec, 600);
        // 60000 / 500 ms = 120 BPM
        assert_eq!(l[1].bpm, 12000);
        let last = l.last().unwrap();
        assert!(last.pos < 44100 * 3 && last.pos + 22050 >= 44100 * 3);
        assert_eq!(get_bpm(&l), 12000.0);
    }
}
