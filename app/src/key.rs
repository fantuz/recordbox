//! Key detection.
//!
//! The default detector (`key_rb.rs`, `key_rules.rs`, `key_judge.rs`) follows the reference
//! application's analyser stage by stage; `checks/key-detector-trace.md` documents the trace
//! and the measurements. The older template-matching detector is kept below as
//! `legacy_detect_key`: it is the fallback for audio the traced detector cannot handle
//! (shorter than about 3.5 s) and the comparison baseline in the benchmark.

#[path = "key_judge.rs"]
mod key_judge;
#[path = "key_rb.rs"]
pub mod key_rb;
#[path = "key_rules.rs"]
mod key_rules;

use crate::model::BeatGrid;
use rustfft::{num_complex::Complex, FftPlanner};

#[derive(Clone, Copy, Debug)]
pub struct KeyParams {
    /// Estimate and correct global tuning (deviation from A=440).
    pub tuning: bool,
    /// Use only spectral peaks (local maxima) instead of every bin.
    pub peaks: bool,
    /// Weight of the bass-register profile added to the full profile.
    pub bass_weight: f64,
    /// Template set (see PROFILES).
    pub profile: usize,
    /// Exponent applied to magnitudes before summing (compression).
    pub power: f64,
    /// Score bonus for minor keys (dance music is mostly minor).
    pub minor_bias: f64,
}

/// Harmonic/percussive separation before the pitch profile: keep energy that is steady in
/// time (notes, chords) and suppress energy that is spread across frequency (drums).
fn hpss_enabled() -> bool {
    std::env::var("PREPDECK_KEY_HPSS").map(|v| v != "0").unwrap_or(HPSS_DEFAULT)
}
const HPSS_DEFAULT: bool = false;

fn median(v: &mut [f32]) -> f32 {
    let mid = v.len() / 2;
    *v.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).1
}

/// Soft-mask each frame with H² / (H² + P²), H = median over ±k frames, P = median over ±k bins.
fn hpss(frames: &mut [Vec<f32>], k: usize) {
    let nf = frames.len();
    if nf == 0 {
        return;
    }
    let nb = frames[0].len();
    let mut h = vec![vec![0f32; nb]; nf];
    let mut buf = Vec::with_capacity(2 * k + 1);
    for b in 0..nb {
        for f in 0..nf {
            buf.clear();
            for g in f.saturating_sub(k)..(f + k + 1).min(nf) {
                buf.push(frames[g][b]);
            }
            h[f][b] = median(&mut buf);
        }
    }
    for f in 0..nf {
        let row = frames[f].clone();
        for b in 0..nb {
            buf.clear();
            buf.extend_from_slice(&row[b.saturating_sub(k)..(b + k + 1).min(nb)]);
            let p = median(&mut buf);
            let hh = h[f][b] * h[f][b];
            let mask = hh / (hh + p * p + 1e-12);
            frames[f][b] = row[b] * mask;
        }
    }
}

pub const DEFAULT: KeyParams = KeyParams { tuning: false, peaks: true, bass_weight: 0.45, profile: 2, power: 1.0, minor_bias: 0.15 };

/// (name, major, minor) templates, index 0 = tonic.
pub const PROFILES: [(&str, [f64; 12], [f64; 12]); 5] = [
    (
        "krumhansl",
        [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88],
        [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17],
    ),
    (
        "temperley",
        [5.0, 2.0, 3.5, 2.0, 4.5, 4.0, 2.0, 4.5, 2.0, 3.5, 1.5, 4.0],
        [5.0, 2.0, 3.5, 4.5, 2.0, 4.0, 2.0, 4.5, 3.5, 2.0, 1.5, 4.0],
    ),
    (
        "diatonic-triad",
        [3.0, 0.0, 1.0, 0.0, 2.0, 1.0, 0.0, 2.0, 0.0, 1.0, 0.0, 1.0],
        [3.0, 0.0, 1.0, 2.0, 0.0, 1.0, 0.0, 2.0, 1.0, 0.0, 1.0, 0.0],
    ),
    (
        // Natural-minor biased template suited to dance music (minor keys with flat 6/7).
        "dance",
        [6.0, 1.0, 3.5, 1.0, 4.5, 4.0, 1.0, 5.0, 1.0, 3.5, 1.5, 2.5],
        [6.0, 1.0, 3.0, 5.0, 1.0, 4.0, 1.0, 5.0, 4.0, 1.0, 3.5, 1.5],
    ),
    (
        "shaath-like",
        [6.6, 2.0, 3.5, 2.3, 4.6, 4.0, 2.5, 5.2, 2.4, 3.7, 2.3, 3.4],
        [6.5, 2.8, 3.5, 5.4, 2.7, 3.5, 2.5, 5.2, 4.0, 2.7, 4.3, 3.2],
    ),
];

pub struct KeyFeatures {
    pub full: [f64; 12],
    pub bass: [f64; 12],
}

/// Pitch-class energy of a whole track.
pub fn features(mono: &[f32], sr: f64, p: &KeyParams) -> KeyFeatures {
    // Downsample to ~11 kHz.
    let factor = ((sr / 11025.0).round() as usize).max(1);
    let mut lp = [0f64; 4];
    let a = (-2.0 * std::f64::consts::PI * 3500.0 / sr).exp();
    let mut ds = Vec::with_capacity(mono.len() / factor + 1);
    for (i, &x) in mono.iter().enumerate() {
        // 4 one-pole stages
        let mut y = x as f64;
        for s in lp.iter_mut() {
            *s = a * *s + (1.0 - a) * y;
            y = *s;
        }
        if i % factor == 0 {
            ds.push(y as f32);
        }
    }
    let dsr = sr / factor as f64;
    let nfft = 8192usize;
    let hop = 2048usize;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let win: Vec<f32> = (0..nfft).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / nfft as f32).cos()).collect();
    let bin_hz = dsr / nfft as f64;
    let lo_bin = (50.0 / bin_hz) as usize;
    let hi_bin = ((2000.0 / bin_hz) as usize).min(nfft / 2 - 2);
    // Collect per-frame magnitudes (only the needed bins) once.
    let mut frames: Vec<Vec<f32>> = vec![];
    let mut buf = vec![Complex::new(0f32, 0f32); nfft];
    let mut start = 0;
    while start + nfft <= ds.len() {
        for i in 0..nfft {
            buf[i] = Complex::new(ds[start + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        frames.push((lo_bin..=hi_bin + 1).map(|b| buf[b].norm()).collect());
        start += hop;
    }
    if hpss_enabled() {
        hpss(&mut frames, 8);
    }
    // Peak list per frame: (fractional bin, magnitude)
    let peaks_of = |m: &Vec<f32>| -> Vec<(f64, f64)> {
        let mut out = vec![];
        for i in 1..m.len() - 1 {
            if p.peaks {
                if m[i] > m[i - 1] && m[i] >= m[i + 1] {
                    // parabolic interpolation
                    let (l, c, r) = (m[i - 1] as f64, m[i] as f64, m[i + 1] as f64);
                    let d = 0.5 * (l - r) / (l - 2.0 * c + r).min(-1e-12);
                    out.push(((lo_bin + i) as f64 + d, c));
                }
            } else {
                out.push(((lo_bin + i) as f64, m[i] as f64));
            }
        }
        out
    };
    // Tuning: histogram of peak deviations (cents) weighted by magnitude.
    let mut tune_cents = 0.0;
    if p.tuning {
        let mut hist = [0f64; 20];
        for (fi, m) in frames.iter().enumerate() {
            if fi % 4 != 0 {
                continue;
            }
            for (b, mag) in peaks_of(m) {
                let f = b * bin_hz;
                if f < 100.0 {
                    continue;
                }
                let midi = 69.0 + 12.0 * (f / 440.0).log2();
                let dev = midi - midi.round(); // -0.5..0.5
                let k = (((dev + 0.5) * 20.0) as usize).min(19);
                hist[k] += mag;
            }
        }
        // circular smoothing and argmax
        let mut best = (0usize, -1.0);
        for k in 0..20 {
            let s = hist[(k + 19) % 20] + 2.0 * hist[k] + hist[(k + 1) % 20];
            if s > best.1 {
                best = (k, s);
            }
        }
        tune_cents = ((best.0 as f64 + 0.5) / 20.0 - 0.5) * 100.0;
    }
    let mut full = [0f64; 12];
    let mut bass = [0f64; 12];
    for m in &frames {
        let mut fr = [0f64; 12];
        let mut bs = [0f64; 12];
        for (b, mag) in peaks_of(m) {
            let f = b * bin_hz;
            let midi = 69.0 + 12.0 * (f / 440.0).log2() - tune_cents / 100.0;
            let near = midi.round();
            let dev = (midi - near).abs();
            if dev > 0.4 {
                continue;
            }
            let w = 1.0 - dev / 0.4;
            let pc = (near as i64).rem_euclid(12) as usize;
            let v = mag.powf(p.power) * w;
            if f < 220.0 {
                bs[pc] += v;
            }
            if f >= 80.0 {
                fr[pc] += v;
            }
        }
        let s: f64 = fr.iter().sum();
        if s > 1e-9 {
            for k in 0..12 {
                full[k] += fr[k] / s;
            }
        }
        let s: f64 = bs.iter().sum();
        if s > 1e-9 {
            for k in 0..12 {
                bass[k] += bs[k] / s;
            }
        }
    }
    KeyFeatures { full, bass }
}

fn corr(x: &[f64; 12], prof: &[f64; 12], root: usize) -> f64 {
    let xm = x.iter().sum::<f64>() / 12.0;
    let ym = prof.iter().sum::<f64>() / 12.0;
    let (mut num, mut dx, mut dy) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let a = x[(i + root) % 12] - xm;
        let b = prof[i] - ym;
        num += a * b;
        dx += a * a;
        dy += b * b;
    }
    num / (dx * dy).sqrt().max(1e-12)
}

/// Camelot-ordered key index (see model::key_from_pitch), or -1.
pub fn key_from_features(f: &KeyFeatures, p: &KeyParams) -> i32 {
    if f.full.iter().sum::<f64>() <= 0.0 {
        return -1;
    }
    let (_, maj, min) = PROFILES[p.profile.min(PROFILES.len() - 1)];
    let mut best = (f64::MIN, 0usize, false);
    for root in 0..12 {
        for (minor, prof) in [(false, &maj), (true, &min)] {
            let mut s = corr(&f.full, prof, root);
            if p.bass_weight > 0.0 && f.bass.iter().sum::<f64>() > 0.0 {
                s += p.bass_weight * corr(&f.bass, prof, root);
            }
            if minor {
                s += p.minor_bias;
            }
            if s > best.0 {
                best = (s, root, minor);
            }
        }
    }
    crate::model::key_from_pitch(best.1 as i32, best.2)
}

/// Converts the traced detector's key (0..11 major on A, Bb, …; 12..23 the minors) to the
/// Camelot-ordered index used everywhere else, or -1.
fn from_traced(k: i32) -> i32 {
    if (0..24).contains(&k) {
        crate::model::key_from_pitch((k % 12 + 9) % 12, k >= 12)
    } else {
        -1
    }
}

/// Brings left/right to 44.1 or 48 kHz, the two rates the analyser is defined for: integer
/// multiples are averaged down, anything else is linearly interpolated to 44.1 kHz.
fn to_base_rate(ch: Vec<f32>, sr: u32) -> (Vec<f32>, u32) {
    if sr == 44100 || sr == 48000 || sr == 0 {
        return (ch, sr);
    }
    for base in [44100u32, 48000] {
        if sr % base == 0 {
            let f = (sr / base) as usize;
            let out = ch.chunks_exact(f).map(|c| c.iter().sum::<f32>() / f as f32).collect();
            return (out, base);
        }
    }
    let ratio = sr as f64 / 44100.0;
    let n = (ch.len() as f64 / ratio) as usize;
    let out = (0..n)
        .map(|i| {
            let p = i as f64 * ratio;
            let j = p as usize;
            let t = (p - j as f64) as f32;
            let a = ch[j.min(ch.len() - 1)];
            let b = ch[(j + 1).min(ch.len() - 1)];
            a + (b - a) * t
        })
        .collect();
    (out, 44100)
}

/// The traced analysis with its intermediate decisions. `left`/`right` at `sr` Hz; `bpm` and
/// `grid` as analysed (no grid = one whole-track decision, without the vote over segments).
pub fn trace_key(left: Vec<f32>, right: Option<Vec<f32>>, sr: u32, bpm: f64, grid: Option<&BeatGrid>) -> key_rb::Trace {
    let peak = left.iter().chain(right.iter().flatten()).fold(0f32, |m, v| m.max(v.abs()));
    trace_with_grid(left.len(), sr, peak, bpm, grid, |bpm, beats| {
        let (left, base) = to_base_rate(left, sr);
        let right = right.map(|r| to_base_rate(r, sr).0);
        key_rb::analyze(&left, right.as_deref(), base, bpm, beats)
    })
}

fn trace_with_grid(
    frames: usize, sr: u32, peak: f32, bpm: f64, grid: Option<&BeatGrid>,
    analyze: impl FnOnce(f32, &[key_rb::GridBeat]) -> key_rb::Trace,
) -> key_rb::Trace {
    // Nothing audible (digital silence, or noise far below any music): no key.
    if frames == 0 || sr == 0 || !(peak > 1e-5) {
        return key_rb::Trace { key: -1, ..Default::default() };
    }
    let duration = frames as f64 / sr as f64;
    let beats: Vec<key_rb::GridBeat> = match grid {
        Some(g) if !g.is_empty() => g
            .beats(duration)
            .iter()
            .filter(|b| b.time >= 0.0)
            .map(|b| key_rb::GridBeat { num: b.beat as u16, ms: (b.time * 1000.0).round() as u32 })
            .collect(),
        _ => vec![],
    };
    let bpm = if bpm > 0.0 { bpm } else { grid.map(|g| g.first_bpm()).unwrap_or(0.0) };
    analyze(bpm as f32, &beats)
}

/// Key of interleaved stereo audio with its analysed tempo and beat grid (estimated here when
/// none is given)
/// (Camelot-ordered index, see model::key_from_pitch, or -1).
pub fn detect_key_stereo(samples: &[f32], sr: u32, bpm: f64, grid: Option<&BeatGrid>) -> i32 {
    detect_key_stereo_with_mono(samples, None, sr, bpm, grid)
}

/// `mono` may borrow the same track's existing arithmetic stereo downmix for tempo and fallback.
pub(crate) fn detect_key_stereo_with_mono(samples: &[f32], mono: Option<&[f32]>, sr: u32, bpm: f64, grid: Option<&BeatGrid>) -> i32 {
    use std::borrow::Cow;
    let mono = || match mono {
        Some(m) => Cow::Borrowed(m),
        None => Cow::Owned(samples.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect::<Vec<f32>>()),
    };
    // Without a grid (grid analysis switched off) one is estimated just for the segment vote.
    let estimated = match grid {
        Some(g) if !g.is_empty() => None,
        _ => Some(crate::analysis::tempo_and_grid(&mono(), sr as f64, 70.0, 180.0)),
    };
    let (bpm, grid) = match &estimated {
        Some((b, g)) => (*b, g.as_ref()),
        None => (bpm, grid),
    };
    let trace = if matches!(sr, 44100 | 48000) {
        let frames = samples.len() / 2;
        let peak = samples[..2 * frames].iter().fold(0f32, |m, v| m.max(v.abs()));
        trace_with_grid(frames, sr, peak, bpm, grid, |bpm, beats| {
            let decimated = key_rb::decimate_stereo(samples, sr);
            key_rb::analyze_decimated(&decimated, frames, sr, bpm, beats)
        })
    } else {
        let (left, right) = samples.chunks_exact(2).map(|c| (c[0], c[1])).unzip();
        trace_key(left, Some(right), sr, bpm, grid)
    };
    let k = from_traced(trace.key);
    if k >= 0 {
        return k;
    }
    legacy_detect_key(&mono(), sr as f64)
}

/// Key of mono audio; tempo and beat grid are estimated here for the vote over segments.
pub fn detect_key(mono: &[f32], sr: f64) -> i32 {
    let (bpm, grid) = crate::analysis::tempo_and_grid(mono, sr, 70.0, 180.0);
    let k = from_traced(trace_key(mono.to_vec(), None, sr.round() as u32, bpm, grid.as_ref()).key);
    if k >= 0 {
        k
    } else {
        legacy_detect_key(mono, sr)
    }
}

/// The earlier detector: pitch-class profile matched against key templates.
pub fn legacy_detect_key(mono: &[f32], sr: f64) -> i32 {
    let f = features(mono, sr, &DEFAULT);
    key_from_features(&f, &DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two bars of a chord progression per key, as plain harmonic tones with a click on every
    /// beat, at 44.1 kHz.
    fn progression(root_pc: i32, minor: bool, seconds: f64) -> Vec<f32> {
        let sr = 44100.0;
        let n = (seconds * sr) as usize;
        let beat = 0.5; // 120 BPM
        // i–iv–v–i (minor) or I–IV–V–I (major), one bar each.
        let third = if minor { 3 } else { 4 };
        let chords = [[0, third, 7], [5, 5 + third, 12], [7, 7 + third, 14], [0, third, 7]];
        let mut out = vec![0f32; n];
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f64 / sr;
            let bar = ((t / (4.0 * beat)) as usize) % 4;
            let mut v = 0.0;
            for (j, st) in chords[bar].iter().enumerate() {
                for oct in [0, 12] {
                    let midi = 48 + root_pc + st + oct;
                    let f = 440.0 * 2f64.powf((midi as f64 - 69.0) / 12.0);
                    for h in 1..=3 {
                        v += (2.0 * std::f64::consts::PI * f * h as f64 * t).sin() / (h * h) as f64 * if j == 0 { 1.0 } else { 0.7 };
                    }
                }
            }
            let phase = (t / beat).fract() * beat;
            let click = if phase < 0.01 { (1.0 - phase / 0.01) * (2.0 * std::f64::consts::PI * 60.0 * t).sin() } else { 0.0 };
            *o = (0.08 * v + 0.5 * click) as f32;
        }
        out
    }

    #[test]
    fn traced_detector_names_simple_progressions() {
        for (pc, minor) in [(9, true), (0, false), (5, true), (7, false)] {
            let mono = progression(pc, minor, 48.0);
            let grid = BeatGrid::constant(0.0, 120.0, 1);
            let tr = trace_key(mono.clone(), Some(mono), 44100, 120.0, Some(&grid));
            assert_eq!(from_traced(tr.key), crate::model::key_from_pitch(pc, minor), "pc {pc} minor {minor}: {:?} {:?}", tr.whole, tr.key);
        }
    }

    #[test]
    fn traced_key_indices_map_to_camelot_order() {
        // A minor = 8A (index 7), C major = 8B (index 19), F minor = 4A (index 3).
        assert_eq!(from_traced(12), 7);
        assert_eq!(from_traced(3), 19);
        assert_eq!(from_traced(20), 3);
        assert_eq!(from_traced(24), -1);
        assert_eq!(from_traced(-1), -1);
    }

    #[test]
    fn native_stereo_decimation_matches_separate_channels_bit_for_bit() {
        // Cover a fractional and a whole group of eight per second, both stereo mix
        // weights, and an incomplete final stereo frame.
        for (sr, seconds) in [(44100, 2), (48000, 30)] {
            let frames = sr as usize * seconds + 13;
            let stereo: Vec<f32> = (0..frames)
                .flat_map(|i| [((i % 251) as f32 - 125.0) / 127.0, ((i % 101) as f32 - 50.0) / 61.0])
                .chain([0.95])
                .collect();
            let (left, right): (Vec<_>, Vec<_>) = stereo.chunks_exact(2).map(|pair| (pair[0], pair[1])).unzip();
            let expected = key_rb::decimate(&left, Some(&right), sr);
            let actual = key_rb::decimate_stereo(&stereo, sr);
            assert_eq!(actual.len(), frames / 8);
            assert!(actual.iter().map(|x| x.to_bits()).eq(expected.iter().map(|x| x.to_bits())), "{sr} Hz");
        }
    }

    #[test]
    fn stereo_key_uses_existing_downmix_for_the_short_track_fallback() {
        let mono = progression(9, true, 2.0);
        let stereo: Vec<_> = mono.iter().flat_map(|&v| [v, v]).collect();
        let supplied = detect_key_stereo_with_mono(&stereo, Some(&mono), 44100, 0.0, None);
        assert_eq!(supplied, legacy_detect_key(&mono, 44100.0));
        assert_eq!(supplied, detect_key_stereo(&stereo, 44100, 0.0, None));
    }

    #[test]
    fn other_sample_rates_are_brought_to_a_base_rate() {
        assert_eq!(to_base_rate(vec![0.0; 96000], 96000).1, 48000);
        assert_eq!(to_base_rate(vec![0.0; 96000], 96000).0.len(), 48000);
        assert_eq!(to_base_rate(vec![0.0; 88200], 88200).1, 44100);
        let (v, sr) = to_base_rate(vec![1.0; 22050], 22050);
        assert_eq!((v.len(), sr), (44100, 44100));
        assert!(v.iter().all(|x| (*x - 1.0).abs() < 1e-6));
    }

    #[test]
    fn very_short_audio_falls_back_without_panicking() {
        let mono = progression(9, true, 2.0);
        let stereo: Vec<f32> = mono.iter().flat_map(|v| [*v, *v]).collect();
        let _ = detect_key_stereo(&stereo, 44100, 0.0, None);
        assert_eq!(detect_key_stereo(&vec![0.0; 2 * 441000], 44100, 0.0, None), -1, "silence has no key");
        assert_eq!(detect_key(&vec![0.0; 441000], 44100.0), -1, "silence has no key");
        assert_eq!(detect_key_stereo(&[], 44100, 0.0, None), legacy_detect_key(&[], 44100.0));
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// Relation of an estimated key to a reference key (both Camelot-ordered indices).
    fn relation(reference: i32, got: i32) -> &'static str {
        if reference < 0 || got < 0 {
            return "none";
        }
        let (rn, rmaj) = (reference % 12, reference >= 12);
        let (gn, gmaj) = (got % 12, got >= 12);
        if reference == got {
            "same"
        } else if rmaj == gmaj && ((rn - gn).rem_euclid(12) == 1 || (gn - rn).rem_euclid(12) == 1) {
            "fifth"
        } else if rn == gn {
            "relative"
        } else if (!rmaj && gmaj && (gn - rn).rem_euclid(12) == 3) || (rmaj && !gmaj && (rn - gn).rem_euclid(12) == 3) {
            "parallel"
        } else {
            "other"
        }
    }

    #[derive(Default, Clone)]
    struct Tally {
        n: usize,
        same: usize,
        fifth: usize,
        relative: usize,
        parallel: usize,
        other: usize,
    }

    impl Tally {
        fn add(&mut self, reference: i32, got: i32) {
            self.n += 1;
            match relation(reference, got) {
                "same" => self.same += 1,
                "fifth" => self.fifth += 1,
                "relative" => self.relative += 1,
                "parallel" => self.parallel += 1,
                _ => self.other += 1,
            }
        }
        /// MIREX-style weighted score (fifth in either direction 0.5, relative 0.3, parallel 0.2).
        fn line(&self) -> String {
            let w = self.same as f64 + 0.5 * self.fifth as f64 + 0.3 * self.relative as f64 + 0.2 * self.parallel as f64;
            format!(
                "exact {}/{} ({:.1} %), weighted {:.1} %; fifth {} relative {} parallel {} other {}",
                self.same,
                self.n,
                100.0 * self.same as f64 / self.n.max(1) as f64,
                100.0 * w / self.n.max(1) as f64,
                self.fifth,
                self.relative,
                self.parallel,
                self.other
            )
        }
    }

    struct Row {
        path: String,
        key: i32,
        conf: f64,
        legacy: i32,
        traced: i32,
        /// The reference routine itself on the same samples and grid (PREPDECK_RBKEY), or -2.
        oracle: i32,
        secs: f64,
    }

    /// Runs the native research harness (directory in PREPDECK_RBKEY, see the trace notes) on
    /// the same decoded audio and beat grid; returns the Camelot-ordered key or -2.
    fn oracle_key(dir: &str, slice: &str, d: &crate::audio::Decoded, bpm: f64, grid: Option<&BeatGrid>, tag: usize) -> i32 {
        let tmp = std::env::temp_dir().join(format!("prepdeck-keyoracle-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let pcm = tmp.join("pcm.f32");
        let beats = tmp.join("beats.txt");
        let bytes: Vec<u8> = d.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut text = format!("{}\n", bpm as f32);
        if let Some(g) = grid {
            for b in g.beats(d.duration()) {
                if b.time >= 0.0 {
                    text.push_str(&format!("{} {}\n", b.beat, (b.time * 1000.0).round() as u32));
                }
            }
        }
        let mut key = -2;
        if std::fs::write(&pcm, bytes).is_ok() && std::fs::write(&beats, text).is_ok() {
            let out = std::process::Command::new(format!("{dir}/rbkey"))
                .current_dir(dir)
                .args([slice, "fixups.txt", pcm.to_str().unwrap(), &d.sr.to_string(), beats.to_str().unwrap(), "2"])
                .output();
            if let Ok(o) = out {
                let s = String::from_utf8_lossy(&o.stdout);
                if let Some(v) = s.lines().find_map(|l| l.strip_prefix("final ")).and_then(|v| v.trim().parse::<i32>().ok()) {
                    key = from_traced(v);
                }
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
        key
    }

    /// PREPDECK_KEYGT = tsv "path<TAB>key[<TAB>energy<TAB>confidence…]" (a header line is
    /// skipped). Prints agreement of the traced and the legacy detector with those keys, using
    /// this application's own decode, tempo and beat grid. PREPDECK_KEYOUT = tsv to write per
    /// track results to. PREPDECK_RBKEY + PREPDECK_RBSLICE additionally run the reference
    /// routine on identical input.
    #[test]
    #[ignore]
    fn key_benchmark() {
        let Ok(f) = std::env::var("PREPDECK_KEYGT") else { return };
        let rows: Vec<(String, i32, f64)> = std::fs::read_to_string(f)
            .unwrap()
            .lines()
            .filter_map(|l| {
                let mut p = l.split('\t');
                let path = p.next()?.to_string();
                let key = crate::model::parse_key(p.next()?);
                let conf = p.nth(1).and_then(|c| c.trim().parse::<f64>().ok()).unwrap_or(1.0);
                (key >= 0).then_some((path, key, conf))
            })
            .collect();
        let oracle = std::env::var("PREPDECK_RBKEY").ok().zip(std::env::var("PREPDECK_RBSLICE").ok());
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results = std::sync::Mutex::new(vec![]);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((path, key, conf)) = rows.get(i) else { break };
                    let Ok(d) = crate::audio::decode_file(path) else { continue };
                    let mono = d.mono();
                    let (bpm, grid) = crate::analysis::tempo_and_grid(&mono, d.sr as f64, 70.0, 180.0);
                    let t0 = std::time::Instant::now();
                    let traced = detect_key_stereo(&d.samples, d.sr, bpm, grid.as_ref());
                    let secs = t0.elapsed().as_secs_f64();
                    let legacy = legacy_detect_key(&mono, d.sr as f64);
                    let ok = match &oracle {
                        Some((dir, slice)) => oracle_key(dir, slice, &d, bpm, grid.as_ref(), i),
                        None => -2,
                    };
                    results.lock().unwrap().push(Row { path: path.clone(), key: *key, conf: *conf, legacy, traced, oracle: ok, secs });
                });
            }
        });
        let res = results.into_inner().unwrap();
        let name = crate::model::key_alnum;
        if let Ok(out) = std::env::var("PREPDECK_KEYOUT") {
            let mut text = String::from("path\tlabel\tconfidence\tlegacy\ttraced\toracle\n");
            for r in &res {
                text.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\n", r.path, name(r.key), r.conf, name(r.legacy), name(r.traced), if r.oracle >= -1 { name(r.oracle) } else { "".into() }));
            }
            let _ = std::fs::write(out, text);
        }
        eprintln!("tracks {} (slowest key analysis {:.2} s, mean {:.2} s)", res.len(), res.iter().map(|r| r.secs).fold(0.0, f64::max), res.iter().map(|r| r.secs).sum::<f64>() / res.len().max(1) as f64);
        for (label, min_conf) in [("all", 0.0), ("confidence >= 0.9", 0.9)] {
            let (mut tl, mut tt, mut to, mut same) = (Tally::default(), Tally::default(), Tally::default(), 0usize);
            let mut with_oracle = 0usize;
            for r in res.iter().filter(|r| r.conf >= min_conf) {
                tl.add(r.key, r.legacy);
                tt.add(r.key, r.traced);
                if r.oracle >= -1 {
                    with_oracle += 1;
                    to.add(r.key, r.oracle);
                    same += (r.oracle == r.traced) as usize;
                }
            }
            eprintln!("[{label}] legacy : {}", tl.line());
            eprintln!("[{label}] traced : {}", tt.line());
            if with_oracle > 0 {
                eprintln!("[{label}] reference routine on the same input: {}", to.line());
                eprintln!("[{label}] traced == reference routine: {same}/{with_oracle}");
            }
        }
        for r in &res {
            if r.traced != r.key {
                eprintln!("  ref {:>4} traced {:>4} legacy {:>4} ({}) :: {}", name(r.key), name(r.traced), name(r.legacy), relation(r.key, r.traced), r.path.rsplit('/').next().unwrap());
            }
        }
    }
}
