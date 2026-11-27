//! rekordbox's high precision BeatGrid analysis ("Use high precision BeatGrid analysis",
//! analysis modes 2 / 3 / 4 = Auto / Normal / Dynamic), ported from rekordbox 7.2.19.
//!
//! `AnalyzeJobBeatDetector::highPrecisionBeatAnalyze` (0x102327e50):
//! 1. the legacy `BeatAnalyzer_2_0` analysis of the whole track gives a BPM, a beat count and a
//!    start offset (`Analyzer::getOffsetCalc`);
//! 2. `madmom::analyze` (0x10230a820) runs madmom's downbeat network (`net`) on a 60 s window from
//!    that offset (the whole track when the tempo may change), decodes it with a bar-pointer DBN
//!    (`dbn`) and turns the beat times into a beat list (`madmom::get_beat_list`, `get_bpm`);
//! 3. `Analyzer::analyzeFocus` merges the list with the legacy analysis and votes on the bar phase.
//! Dynamic and Auto-detected tempo changes use model7 over minute-sized windows.
//! Unresolved subdivisions, half-beat offsets or unusable timing retain the ordinary estimator.

#[path = "rb_hp_net.rs"]
pub mod net;
#[path = "rb_hp_dbn.rs"]
pub mod dbn;
#[path = "rb_hp_uba.rs"]
pub mod uba;

#[path = "rb_hp_focus.rs"]
pub mod focus;
#[path = "rb_hp_vote.rs"]
mod vote;
#[path = "rb_hp_legacy.rs"]
mod legacy;
#[path = "rb_hp_src.rs"]
mod src;
#[path = "rb_hp_tempo.rs"]
mod tempo;
#[path = "rb_hp_focus2.rs"]
mod focus2;

use crate::model::BeatGrid;
use std::sync::OnceLock;
use uba::Uba;

/// Analysis modes as rekordbox numbers them (`CBeatAnalyzeThread::analyze`, field +14544).
pub const MODE_AUTO: u8 = 2;
#[allow(dead_code)]
pub const MODE_NORMAL: u8 = 3;
pub const MODE_DYNAMIC: u8 = 4;

/// The BPM and candidate positions from `BeatAnalyzer_2_0`. The positions select
/// a network window, and the BPM guides octave and integer-tempo correction.
#[derive(Clone, Debug)]
pub struct Legacy {
    /// BPM as the legacy library stores it (float, the same value x100 rounded is `BA_GetBpm`).
    pub bpm: f32,
    /// Beat positions in samples from the first beat on.
    pub beats: Vec<i32>,
}

/// `Analyzer::getOffsetCalc` (0x101182100): where the 60 s window of the network starts.
/// Integrates the low band over half a beat at every legacy beat, normalises, and finds the
/// first loud section (thresholds 0.2 / 0.4 / 0.15 on the integrals and their differences);
/// the offset is 4 beats before it, at `sr * 60 * (index - 4) / bpm` samples.
pub fn get_offset_calc(env: &Uba, sr: i32, legacy: &Legacy) -> i32 {
    let n = legacy.beats.len();
    if n == 0 {
        return 0;
    }
    let bpm = legacy.bpm;
    let mut v19 = vec![0f64; n];
    let mut v20 = vec![0f64; n];
    let v21 = n - 1;
    let mut v22 = 0f64;
    if n >= 2 {
        for i in 0..v21 {
            let mut x = env.calc_low_integ(bpm, legacy.beats[i]);
            if i != 0 && x < 0.0 {
                x = v19[i - 1];
            }
            v19[i] = x;
        }
        for &x in &v19[..v21] {
            v22 = v22.max(x);
        }
    }
    let mut v42: i64 = -1;
    let mut v68: i64;
    'done: {
        if v22.abs() > 2.22044605e-16 {
            if n <= 1 {
                v20[0] = 0.0;
                v68 = v42;
                break 'done;
            }
            let inv = 1.0 / v22;
            for x in v19[..v21].iter_mut() {
                *x *= inv;
            }
        }
        v20[0] = 0.0;
        if n < 3 {
            v68 = -1;
            break 'done;
        }
        for k in 1..v21 {
            v20[k] = v19[k] - v19[k - 1];
        }
        let g = |v: &Vec<f64>, k: usize| v.get(k).copied().unwrap_or(0.0);
        let mut v64 = false;
        let mut v65 = 0i64;
        let mut v66 = 0i64;
        v68 = -1;
        let mut v63 = 0usize;
        loop {
            let i = v63;
            // labels of the original: 39 (quiet: reset), 40 (next), 52 (loud), 64, 65, 69
            enum Next {
                L39,
                L40,
                L52,
                L64,
                L65,
                L69,
                L76,
            }
            let mut next;
            if v63 != 0 {
                let v69 = g(&v19, i);
                if !v64 {
                    if v69 <= 0.2 {
                        v64 = false;
                        next = Next::L64;
                    } else if g(&v20, i) <= 0.15 {
                        v64 = false;
                        next = if g(&v19, i) >= 0.2 { Next::L69 } else { Next::L65 };
                    } else {
                        if v65 > 32 || v68 < 0 {
                            v42 = v63 as i64;
                        }
                        v65 = 0;
                        v68 = v63 as i64;
                        next = if v66 > 11 { Next::L76 } else { Next::L52 };
                    }
                } else if v69 <= 0.4 && (v69 <= 0.2 || g(&v19, i + 1) <= 0.2 || g(&v19, i + 2) <= 0.2) {
                    v64 = true;
                    next = if g(&v19, i) >= 0.2 { Next::L69 } else { Next::L65 };
                } else {
                    next = if v66 > 11 { Next::L76 } else { Next::L52 };
                }
            } else if g(&v19, 0) <= 0.4 {
                next = Next::L64;
            } else {
                v68 = 0;
                v42 = 0;
                next = if v66 > 11 { Next::L76 } else { Next::L52 };
            }
            loop {
                match next {
                    Next::L64 => next = if g(&v19, i) >= 0.2 { Next::L69 } else { Next::L65 },
                    Next::L52 => {
                        v66 += 1;
                        v64 = true;
                        next = if g(&v19, i) < 0.2 { Next::L65 } else { Next::L69 };
                    }
                    Next::L65 => next = if g(&v19, i + 1) < 0.2 && g(&v19, i + 2) < 0.2 { Next::L39 } else { Next::L69 },
                    Next::L69 => next = if g(&v20, i) < -0.4 && g(&v20, i + 1) < 0.2 && g(&v20, i + 2) < 0.2 { Next::L39 } else { Next::L40 },
                    Next::L39 => {
                        v66 = 0;
                        v64 = false;
                        v65 += 1;
                        next = Next::L40;
                    }
                    Next::L40 | Next::L76 => break,
                }
            }
            if matches!(next, Next::L76) {
                break 'done;
            }
            v63 += 1;
            if n - 2 == v63 {
                break;
            }
        }
        if v66 <= 12 {
            v68 = v42;
        }
    }
    let v71 = v68.max(0);
    let v72 = (sr as f64 * 60.0 * (v71 - 4) as f64 / bpm as f64) as i32;
    v72.max(0)
}

/// The window `madmom::analyze` (file version, 0x10230a820) runs the network on:
/// 60 s from the offset (the last 60 s when that runs past the end), the whole track when the
/// tempo may change. Returns (start, length) in frames.
pub fn madmom_window(total: i64, sr: i64, offset: i64, hendou: bool) -> (i64, i64) {
    let v62 = total;
    let mut v63 = v62 - 1;
    let mut v64 = 60 * sr;
    let mut v65 = v62 - 1 - v64;
    let v66 = if v62 - 1 > v64 { v64 } else { v62 - 1 };
    if v62 - 1 <= v64 {
        v65 = 0;
    }
    let v67;
    if v62 - 1 - offset > v64 {
        v67 = offset;
    } else {
        v64 = v66;
        v67 = v65;
    }
    let v68;
    if hendou {
        v68 = 0;
    } else {
        v63 = v64;
        v68 = v67;
    }
    if v62 != 0 {
        (v68, v63)
    } else {
        (0, 0)
    }
}

/// Times of the network's beats at other rates than 44.1 kHz: rekordbox scales them by
/// 44100 / sr, the vectorised part as `t * (44100 / sr)`, the last 1-4 values as
/// `(t * 44100) * (1 / sr)`.
pub fn scale_times(beats: &mut [(f32, i32)], sr: i32) {
    if sr == 44100 || beats.is_empty() {
        return;
    }
    let n = beats.len();
    let v80 = sr as f32;
    let tail = if n > 4 {
        let r = n & 3;
        if r == 0 { 4 } else { r }
    } else {
        n
    };
    let k = 44100.0f32 / v80;
    let inv = 1.0f32 / v80;
    for (i, b) in beats.iter_mut().enumerate() {
        if i < n - tail {
            b.0 *= k;
        } else {
            b.0 = (b.0 * 44100.0) * inv;
        }
    }
}

fn model(dynamic: bool) -> Option<&'static net::Net> {
    static NORMAL: OnceLock<Option<net::Net>> = OnceLock::new();
    static DYNAMIC: OnceLock<Option<net::Net>> = OnceLock::new();
    let (cell, name) = if dynamic { (&DYNAMIC, "model7") } else { (&NORMAL, "model8") };
    cell.get_or_init(|| net::Net::load(&net::models_dir().join(name)).ok()).as_ref()
}

/// A cheap, process-local file check during admission. Reading weights happens
/// later on the worker, only for an enabled model inside its working-set limit.
pub(super) fn models_present(mode: u8) -> bool {
    fn present(dynamic: bool) -> bool {
        static NORMAL: OnceLock<bool> = OnceLock::new();
        static DYNAMIC: OnceLock<bool> = OnceLock::new();
        *(if dynamic { &DYNAMIC } else { &NORMAL }).get_or_init(|| {
            let dir = net::models_dir().join(if dynamic { "model7" } else { "model8" });
            ["saved_model.pb", "variables/variables.index", "variables/variables.data-00000-of-00001"]
                .iter().all(|p| dir.join(p).is_file())
        })
    }
    match mode {
        MODE_DYNAMIC => present(true),
        MODE_AUTO => present(false) || present(true),
        MODE_NORMAL => present(false),
        _ => false,
    }
}

fn dbn() -> &'static dbn::Dbn {
    static DBN: OnceLock<dbn::Dbn> = OnceLock::new();
    DBN.get_or_init(dbn::Dbn::rekordbox)
}

/// The network + DBN + beat list stage for one window of the decode.
pub fn madmom_stage(samples: &[f32], sr: u32, start: usize, len: usize, hendou: bool) -> Option<(Vec<dbn::ListBeat>, f64, Vec<(f32, i32)>)> {
    let pcm = samples.get(start.checked_mul(2)?..start.checked_add(len)?.checked_mul(2)?)?;
    let net = if hendou { model(true).or_else(|| model(false))? } else { model(false)? };
    let mono: Vec<f32> = pcm.chunks_exact(2).map(|lr| (lr[0] + lr[1]) * 0.5).collect();
    let act = net.run(&mono);
    let mut beats = dbn().track(&act);
    scale_times(&mut beats, sr as i32);
    let list = dbn::get_beat_list(&beats, len as i64, sr as i32, 4);
    let bpm = dbn::get_bpm(&list);
    Some((list, bpm, beats))
}

/// Everything the high precision analysis produced for one track.
#[allow(dead_code)]
pub struct HpOutcome {
    pub bpm: f64,
    pub grid: BeatGrid,
    /// Beat positions (samples), beat numbers and bpm x 100 as rekordbox's `BA_GetBeatPos_Focus`.
    pub beats: Vec<(i32, i32, i32)>,
    pub adaptive: bool,
    pub offset: i32,
    pub window: (usize, usize),
}

const DYNAMIC_FALLBACK: &str = "retain the ordinary variable-tempo grid";
const NO_GRID: &str = "the beat analysis could not establish a reliable grid";

/// The legacy envelope and the implemented model path cover at most 15 minutes.
/// Check eligibility before allocating converted audio or touching optional weights.
pub(super) fn eligible(dec: &crate::audio::Decoded, mode: u8) -> bool {
    dec.samples.len() % 2 == 0 && eligible_audio(dec.sr, dec.frames(), mode)
}

fn eligible_audio(sr: u32, frames: usize, mode: u8) -> bool {
    matches!(mode, MODE_AUTO | MODE_NORMAL | MODE_DYNAMIC) && (8000..=200_000).contains(&sr) &&
        sr % 50 == 0 && frames > 1 && frames <= 900 * sr as usize
}

fn analyze_model(dec: &crate::audio::Decoded, mode: u8, shared: Option<super::rb_beat1::Front>) -> Result<HpOutcome, &'static str> {
    if !eligible(dec, mode) || !models_present(mode) { return Err("unsupported track or missing local beat model"); }
    let sr = super::rb_beat1::analysis_rate(dec.sr);
    let converted;
    let samples = if sr == dec.sr { dec.samples.as_slice() } else {
        converted = src::new(dec.sr, sr).ok_or("unsupported model conversion rate")?.run(&dec.samples);
        &converted
    };
    let total = samples.len() / 2;
    let analysed = total.saturating_sub(1);
    let input = &samples[..2 * analysed];
    let front = shared.or_else(|| super::rb_beat1::Front::from_audio(input, sr)).ok_or(NO_GRID)?;
    let env = Uba::from_audio(sr, input);
    let dynamic = mode == MODE_DYNAMIC;
    let l = legacy::analyze(front, sr, dynamic, Some(&env.low)).ok_or(NO_GRID)?;
    let varying = dynamic || (mode == MODE_AUTO && l.hendou);
    let bpm100 = l.bpm100();
    let legacy = Legacy { bpm: l.bpm, beats: l.beats.iter().map(|b| b.pos).collect() };
    let offset = if varying { 0 } else { get_offset_calc(&env, sr as i32, &legacy) };
    let (start, len) = madmom_window(total as i64, sr as i64, offset as i64, varying);
    let (list, mbpm) = if varying {
        // Each piece is processed and discarded before starting the next one.
        let net = model(true).ok_or(DYNAMIC_FALLBACK)?;
        tempo::madmom_tempo(net, dbn(), samples, sr, total).ok_or(DYNAMIC_FALLBACK)?
    } else {
        let (list, bpm, _) = madmom_stage(samples, sr, start as usize, len as usize, false).ok_or(NO_GRID)?;
        (list, bpm as i32)
    };
    let ml: Vec<(i32, i32, i32)> = list.iter().map(|b| (b.pos as i32, b.num_bar, b.bpm)).collect();
    let input = focus::FocusInput { sr: sr as i32, total: analysed as i32, offset, hendou: varying,
        legacy_bpm100: bpm100, legacy_count: legacy.beats.len() as i32, madmom: &ml, madmom_bpm: mbpm };
    let failed = if varying { DYNAMIC_FALLBACK } else { NO_GRID };
    let out = focus::analyze_focus(&env, &input).map_err(|_| failed)?;
    let beats = out.beats;
    if beats.is_empty() || out.bpm100 <= 0 ||
        beats.iter().any(|&(pos, num, tempo)| pos < 0 || pos as usize > analysed || !(1..=4).contains(&num) || tempo <= 0) ||
        beats.windows(2).any(|w| w[0].0 >= w[1].0) { return Err(failed); }
    let bpm = f64::from(out.bpm100) / 100.0;
    let grid = grid_from_list(&beats, sr);
    if out.adaptive && !grid_replays(&grid, &beats, sr, total) { return Err(failed); }
    Ok(HpOutcome { bpm, grid, beats, adaptive: out.adaptive, offset, window: (start as usize, len as usize) })
}

/// Microseconds of a beat position as `AnalyzeJobBeatDetector::convertToMicrosec` (0x102328b20)
/// computes them: `1000000 * pos / sr` in integers; the analysis file keeps `us / 1000` (PQTZ).
pub fn beat_us(pos: i32, sr: u32) -> i64 {
    1_000_000i64 * pos as i64 / sr as i64
}

/// The grid for rekordbox's beat list (position, beat number, tempo x 100 per beat): rekordbox
/// stores each beat, while recordbox extrapolates between grid anchors. Preserve
/// each listed beat's microseconds, bar number and tempo for PQTZ and PQT2 exports.
pub fn grid_from_list(beats: &[(i32, i32, i32)], sr: u32) -> BeatGrid {
    let mut anchors: Vec<crate::model::GridAnchor> = vec![];
    // `pt`: where `BeatGrid::beats` puts this beat (it adds the period beat by beat)
    let (mut pt, mut a_bpm100, mut pn) = (0f64, 0i32, 0i32);
    for (j, &(pos, num, bpm100)) in beats.iter().enumerate() {
        let us = beat_us(pos, sr);
        let t = us as f64 / 1e6;
        let num = num.clamp(1, 4);
        if j > 0 {
            pt += 60.0 / (a_bpm100 as f64 / 100.0).max(1.0);
            pn = pn % 4 + 1;
        }
        let fits = j > 0 && bpm100 == a_bpm100 && bpm100 > 0 && num == pn && (pt * 1e6).round() as i64 == us;
        if !fits {
            let bpm = if bpm100 > 0 { bpm100 as f64 / 100.0 } else { anchors.last().map(|a| a.bpm).unwrap_or(120.0) };
            anchors.push(crate::model::GridAnchor { time: t, bpm, beat: num as u8 });
            (pt, a_bpm100, pn) = (t, if bpm100 > 0 { bpm100 } else { (bpm * 100.0).round() as i32 }, num);
        }
    }
    BeatGrid { anchors }
}

/// Some irregular interval/tempo combinations would cause the live grid to
/// insert or drop a beat. Keep the ordinary result if positions, bar numbers
/// and tempi cannot all be represented faithfully.
fn grid_replays(grid: &BeatGrid, expected: &[(i32, i32, i32)], sr: u32, total: usize) -> bool {
    let actual = grid.beats(total as f64 / f64::from(sr));
    let us = |time: f64| (time * 1e6).round() as i64;
    let first = beat_us(expected[0].0, sr);
    let last = beat_us(expected[expected.len()-1].0, sr);
    let start = actual.partition_point(|b| us(b.time) < first);
    let end = actual.partition_point(|b| us(b.time) <= last);
    let actual = &actual[start..end];
    actual.len() == expected.len() && actual.iter().zip(expected).all(|(b, &(pos, bar, bpm))|
        us(b.time) == beat_us(pos, sr) && i32::from(b.beat) == bar && (b.bpm * 100.0).round() as i32 == bpm)
}

/// Select the installed-model mode once per analysis job. Unspecialized callers use
/// the standard grid unless a valid diagnostic override explicitly requests the model.
pub fn mode_from_preferences(enabled: bool, analysis_mode: u8) -> u8 {
    match std::env::var("PREPDECK_HP_MODE").ok().and_then(|v| v.parse().ok()) {
        Some(m @ (0 | MODE_AUTO | MODE_NORMAL | MODE_DYNAMIC)) => m,
        _ if !enabled => 0,
        _ if analysis_mode == 1 => MODE_DYNAMIC,
        _ => MODE_AUTO,
    }
}

/// Try the selected model for this job. A complete result can replace the
/// standard one only within the user's requested BPM range.
pub(super) fn dispatch(dec: &crate::audio::Decoded, mono: &[f32], s: &super::AnalysisSettings,
    bpm: f64, grid: Option<BeatGrid>, shared: Option<super::rb_beat1::Front>) -> (f64, Option<BeatGrid>) {
    if grid.as_ref().is_none_or(BeatGrid::is_empty) || !eligible(dec, s.model_grid_mode) { return (bpm, grid); }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
        analyze_model(dec, s.model_grid_mode, shared)
    )).unwrap_or(Err(NO_GRID));
    match result {
        Ok(o) => {
            let mut g = o.grid;
            let mut b = o.bpm;
            let dur = dec.duration();
            while !o.adaptive && b < s.bpm_min && b > 0.0 { g = g.doubled(dur); b *= 2.0; }
            while !o.adaptive && b > s.bpm_max { g = g.halved(dur); b *= 0.5; }
            if (s.bpm_min..=s.bpm_max).contains(&b) && !g.is_empty() { (b, Some(g)) } else { (bpm, grid) }
        }
        Err(DYNAMIC_FALLBACK) if !s.dynamic && s.model_grid_mode == MODE_AUTO => {
            // An unsupported change in tempo still needs the selected adaptive
            // estimator. Explicit Dynamic already computed that grid.
            let dynamic = super::AnalysisSettings { dynamic: true, model_grid_mode: 0, ..s.clone() };
            let (b, g) = super::analyze_grid(dec, mono, &dynamic);
            if g.is_some() { (b, g) } else { (bpm, grid) }
        }
        Err(_) => (bpm, grid),
    }
}

#[cfg(test)]
mod model_grid_tests {
    use super::*;

    #[test]
    fn duration_and_rate_gates_apply_before_model_loading_and_resampling() {
        assert!(eligible_audio(8000, 900 * 8000, MODE_AUTO));
        assert!(!eligible_audio(8000, 900 * 8000 + 1, MODE_AUTO));
        assert!(eligible_audio(96_000, 96_000, MODE_NORMAL));
        assert!(!eligible_audio(11_025, 11_025, MODE_NORMAL));
        assert!(!eligible_audio(384_000, 384_000, MODE_AUTO));
        assert!(!eligible_audio(48_000, 1, MODE_NORMAL));
        assert!(eligible_audio(48_000, 48_000, MODE_DYNAMIC));
    }

    #[test]
    fn focus_grid_preserves_submillisecond_timing_and_bar_numbers() {
        // All timing changes after beat one are below a millisecond. A single
        // constant anchor (and checking integer milliseconds only) loses them.
        let input = [(4_800, 3, 12_000), (28_801, 4, 12_000),
            (52_840, 1, 12_000), (76_820, 2, 12_000)];
        let grid = grid_from_list(&input, 48_000);
        let result: Vec<_> = grid.beats(1.7).iter().map(|b|
            ((b.time * 1e6).round() as i64, b.beat, (b.bpm * 100.0).round() as i32)).collect();
        assert_eq!(result, [(100_000, 3, 12_000), (600_020, 4, 12_000),
            (1_100_833, 1, 12_000), (1_600_416, 2, 12_000)]);
    }

    #[test]
    fn sparse_envelopes_match_saturated_pcm_including_full_scale() {
        let samples: Vec<_> = (0..4_001).flat_map(|i| {
            let x = ((i % 101) as f32 - 50.0) / 44.0;
            [x, 0.6*x]
        }).collect();
        let got = Uba::from_audio(48_000, &samples);
        let want = Uba::new(48_000, &uba::to_i16(&samples));
        assert_eq!((got.low, got.all, got.high), (want.low, want.all, want.high));
        assert_eq!(uba::to_i16(&[-1.2, -1.0, 0.0, 1.0, 1.2]), [-32_768, -32_768, 0, 32_767, 32_767]);
    }
}

#[cfg(test)]
mod oracle_tests {
    use super::net::*;

    fn read_f32(p: &str) -> Vec<f32> {
        std::fs::read(p).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    /// Network stage against rekordbox's own TensorFlow run (`checks/beat-oracle-harness/rbtf`):
    /// PREPDECK_HP_PCM = stereo f32 PCM, PREPDECK_HP_TF = rbtf output prefix, PREPDECK_HP_START /
    /// PREPDECK_HP_LEN = the window rbtf was given, PREPDECK_HP_MODEL = model directory.
    #[test]
    #[ignore]
    fn net_against_tensorflow() {
        let (Ok(pcm), Ok(tf)) = (std::env::var("PREPDECK_HP_PCM"), std::env::var("PREPDECK_HP_TF")) else { return };
        let start: usize = std::env::var("PREPDECK_HP_START").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let len: usize = std::env::var("PREPDECK_HP_LEN").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let model = std::env::var("PREPDECK_HP_MODEL").map(std::path::PathBuf::from).unwrap_or_else(|_| models_dir().join("model8"));
        let st = read_f32(&pcm);
        let n = if len == 0 { st.len() / 2 - start } else { len };
        let mono: Vec<f32> = (start..start + n).map(|i| (st[2 * i + 1] + st[2 * i]) * 0.5).collect();
        let t0 = std::time::Instant::now();
        let net = Net::load(&model).unwrap();
        let t1 = std::time::Instant::now();
        let feats = net.features(&mono);
        let t2 = std::time::Instant::now();
        let act = net.activations(&feats);
        let t3 = std::time::Instant::now();
        let want_f = read_f32(&format!("{tf}.feat.f32"));
        let want_a = read_f32(&format!("{tf}.act.f32"));
        assert_eq!(want_f.len(), feats.len(), "feature count");
        let mut worst = (0f32, 0usize);
        let mut sum = 0f64;
        for (i, (a, b)) in feats.iter().zip(&want_f).enumerate() {
            let d = (a - b).abs();
            sum += d as f64;
            if d > worst.0 {
                worst = (d, i);
            }
        }
        eprintln!(
            "features: {} frames, max |diff| {:.3e} at frame {} col {}, mean {:.3e}",
            feats.len() / NUM_FEATURES,
            worst.0,
            worst.1 / NUM_FEATURES,
            worst.1 % NUM_FEATURES,
            sum / feats.len() as f64
        );
        let mut worst_a = 0f32;
        for (a, b) in act.iter().flatten().zip(&want_a) {
            worst_a = worst_a.max((a - b).abs());
        }
        // The same network on TensorFlow's own features isolates the LSTM part.
        let act_tf = net.activations(&want_f);
        let mut worst_b = 0f32;
        for (a, b) in act_tf.iter().flatten().zip(&want_a) {
            worst_b = worst_b.max((a - b).abs());
        }
        eprintln!("activations: max |diff| {worst_a:.3e} (on TF's features {worst_b:.3e})");
        eprintln!("time: load {:?} features {:?} network {:?}", t1 - t0, t2 - t1, t3 - t2);
    }
}

#[cfg(test)]
mod pcm_dump {
    /// Writes recordbox's own decode (interleaved stereo f32) of each PREPDECK_AUDIO file
    /// ('|'-separated) to PREPDECK_HP_DUMP/<n>.f32, so the oracle harness runs on the same PCM.
    #[test]
    #[ignore]
    fn dump_pcm() {
        let Ok(dir) = std::env::var("PREPDECK_HP_DUMP") else { return };
        let list = std::env::var("PREPDECK_AUDIO").unwrap_or_default();
        for (i, p) in list.split('|').filter(|s| !s.is_empty()).enumerate() {
            let Ok(d) = crate::audio::decode_file(p) else {
                eprintln!("DUMP\t{i}\tERR\t{p}");
                continue;
            };
            let bytes: Vec<u8> = d.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(format!("{dir}/{i}.f32"), bytes).unwrap();
            eprintln!("DUMP\t{i}\t{}\t{}\t{p}", d.sr, d.frames());
        }
    }
}

#[cfg(test)]
mod stage_tests {
    use super::{dbn, net};

    /// One oracle dump (`checks/beat-oracle-harness/rbhp` output, RB_ALL=1).
    pub struct Dump {
        pub path: String,
        pub sr: u32,
        pub legacy_bpm: i32,
        pub beat_num: i32,
        pub offset: i64,
        pub hendou: bool,
        pub win: (usize, usize),
        pub m: Vec<(f32, i32)>,
        pub j: Vec<(i64, i32, i32)>,
        pub j_bpm: f64,
        pub f: Vec<(i64, i32, i32)>,
        pub f_bpm: i32,
    }

    pub fn read_dump(p: &std::path::Path) -> Option<Dump> {
        let text = std::fs::read_to_string(p).ok()?;
        let mut d = Dump { path: String::new(), sr: 0, legacy_bpm: 0, beat_num: 0, offset: 0, hendou: false, win: (0, 0), m: vec![], j: vec![], j_bpm: 0.0, f: vec![], f_bpm: 0 };
        for l in text.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            match (w.first().copied(), w.get(1).copied()) {
                (Some("P"), _) => d.path = l[2..].to_string(),
                (Some("SR"), Some(v)) => d.sr = v.parse().ok()?,
                (Some("L"), Some("bpm")) => {
                    d.legacy_bpm = w[2].parse().ok()?;
                    d.beat_num = w[4].parse().ok()?;
                    d.offset = w[6].parse().ok()?;
                    d.hendou = w.iter().position(|x| *x == "hendou").map(|i| w[i + 1] == "1").unwrap_or(false);
                }
                (Some("W"), _) => d.win = (w[2].parse().ok()?, w[4].parse().ok()?),
                (Some("M"), Some("beats")) => {}
                (Some("M"), Some(_)) => d.m.push((w[1].parse().ok()?, w[2].parse().ok()?)),
                (Some("J"), Some("bpm")) => d.j_bpm = w[2].parse().ok()?,
                (Some("J"), Some(_)) => d.j.push((w[1].parse().ok()?, w[2].parse().ok()?, w[3].parse().ok()?)),
                (Some("F"), Some("bpm")) => d.f_bpm = w[2].parse().ok()?,
                (Some("F"), Some(_)) => d.f.push((w[1].parse().ok()?, w[2].parse().ok()?, w[3].parse().ok()?)),
                _ => {}
            }
        }
        Some(d)
    }

    pub fn dumps() -> Vec<(usize, Dump)> {
        let Ok(dir) = std::env::var("PREPDECK_HP_ORACLE") else { return vec![] };
        let mut v: Vec<(usize, Dump)> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| {
                let e = e.ok()?;
                let k: usize = e.file_name().to_str()?.strip_suffix(".txt")?.parse().ok()?;
                Some((k, read_dump(&e.path())?))
            })
            .collect();
        v.sort_by_key(|x| x.0);
        v
    }

    /// madmom stage against the oracle: the same window of recordbox's decode through the ported
    /// network + DBN (raw beats, "M") and get_beat_list / get_bpm ("J").
    /// PREPDECK_HP_ORACLE = directory of rbhp dumps.
    /// Focus stage against the oracle on the oracle's own inputs (legacy BPM, count, offset,
    /// window): the debug csv lines rekordbox wrote (PREPDECK_HP_PROP/<n>/csv/) and the final
    /// beat list. PREPDECK_HP_ORACLE = rbhp dumps, PREPDECK_HP_PROP = their csv folders,
    /// PREPDECK_HP_ONLY = comma list of track numbers.
    #[test]
    #[ignore]
    fn focus_stage_against_oracle() {
        let all = dumps();
        let Ok(prop) = std::env::var("PREPDECK_HP_PROP") else { return };
        let only: Vec<usize> = std::env::var("PREPDECK_HP_ONLY").map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
        let names = ["interval.csv", "interval2.csv", "interval3.csv", "interval4.csv", "interval5.csv"];
        let mut same = [0usize; 5];
        let (mut n, mut f_same, mut first_same) = (0, 0, 0);
        for (k, d) in &all {
            if d.hendou || d.m.is_empty() || (!only.is_empty() && !only.contains(k)) {
                continue;
            }
            let Ok(dec) = crate::audio::decode_file(&d.path) else { continue };
            let env = super::uba::Uba::new(dec.sr as i32, &super::uba::to_i16(&dec.samples));
            let Some((list, mbpm, _)) = super::madmom_stage(&dec.samples, dec.sr, d.win.0, d.win.1, false) else { continue };
            let ml: Vec<(i32, i32, i32)> = list.iter().map(|b| (b.pos as i32, b.num_bar, b.bpm)).collect();
            let inp = super::focus::FocusInput { sr: dec.sr as i32, total: dec.frames() as i32, offset: d.offset as i32, hendou: false, legacy_bpm100: d.legacy_bpm, legacy_count: d.beat_num, madmom: &ml, madmom_bpm: mbpm as i32 };
            let Ok(out) = super::focus::analyze_focus(&env, &inp) else {
                eprintln!("  #{k} focus returned nothing");
                continue;
            };
            n += 1;
            let mut line = format!("  #{k}");
            for (c, name) in names.iter().enumerate() {
                let want: Vec<String> = std::fs::read_to_string(format!("{prop}/{k}/csv/{name}")).unwrap_or_default().split("\r\n").filter(|l| !l.is_empty()).map(|l| l.trim_end_matches('\r').to_string()).collect();
                let want: Vec<String> = if c == 2 { want.iter().map(|l| l.split(',').next().unwrap_or("").to_string()).collect() } else { want };
                let ok = want == out.csv[c];
                same[c] += ok as usize;
                if !ok {
                    let first = want.iter().zip(&out.csv[c]).position(|(a, b)| a != b);
                    line += &format!(" {name}: {} vs {} lines, first diff {:?}", out.csv[c].len(), want.len(), first.map(|i| (i, out.csv[c][i].clone(), want[i].clone())));
                }
            }
            let fm: Vec<(i64, i32, i32)> = out.beats.iter().map(|b| (b.0 as i64, b.1, b.2)).collect();
            let fok = fm == d.f && out.bpm100 == d.f_bpm;
            f_same += fok as usize;
            let fs = fm.first().map(|b| b.0) == d.f.first().map(|b| b.0);
            first_same += fs as usize;
            if !fok {
                line += &format!(" | F {} beats bpm {} first {:?} vs oracle {} beats bpm {} first {:?}", fm.len(), out.bpm100, fm.first(), d.f.len(), d.f_bpm, d.f.first());
            }
            if line.len() > 8 {
                eprintln!("{line}");
            }
        }
        eprintln!("focus stage: tracks {n}; csv identical {same:?}; final list identical {f_same}, first beat identical {first_same}");
    }

    #[test]
    #[ignore]
    fn madmom_stage_against_oracle() {
        let all = dumps();
        if all.is_empty() {
            return;
        }
        let model = net::Net::load(&net::models_dir().join("model8")).unwrap();
        let hmm = dbn::Dbn::rekordbox();
        let (mut n, mut m_same, mut j_same, mut bpm_same) = (0, 0, 0, 0);
        let mut t_net = std::time::Duration::ZERO;
        let mut t_dbn = std::time::Duration::ZERO;
        for (k, d) in &all {
            if d.hendou || d.m.is_empty() {
                continue;
            }
            let Ok(dec) = crate::audio::decode_file(&d.path) else { continue };
            let (st, len) = d.win;
            let mono: Vec<f32> = (st..st + len).map(|i| (dec.samples[2 * i + 1] + dec.samples[2 * i]) * 0.5).collect();
            let t0 = std::time::Instant::now();
            let act = model.run(&mono);
            let t1 = std::time::Instant::now();
            let beats = hmm.track(&act);
            t_dbn += t1.elapsed();
            t_net += t1 - t0;
            let list = dbn::get_beat_list(&beats, len as i64, d.sr as i32, 4);
            let bpm = dbn::get_bpm(&list);
            n += 1;
            let m_ok = beats.len() == d.m.len() && beats.iter().zip(&d.m).all(|(a, b)| a.1 == b.1 && (a.0 - b.0).abs() < 1e-6);
            let j_ok = list.len() == d.j.len() && list.iter().zip(&d.j).all(|(a, b)| (a.pos, a.num_bar, a.bpm) == *b);
            m_same += m_ok as usize;
            j_same += j_ok as usize;
            bpm_same += (bpm == d.j_bpm) as usize;
            if !m_ok || !j_ok {
                let first = beats.iter().zip(&d.m).position(|(a, b)| a.1 != b.1 || (a.0 - b.0).abs() >= 1e-6);
                eprintln!(
                    "  #{k} M {} vs {} first diff {:?} | J {} vs {} bpm {bpm} vs {} :: {}",
                    beats.len(),
                    d.m.len(),
                    first.map(|i| (i, beats[i], d.m[i])),
                    list.len(),
                    d.j.len(),
                    d.j_bpm,
                    d.path.rsplit('/').next().unwrap_or("")
                );
            }
        }
        eprintln!("madmom stage: tracks {n}, raw beats identical {m_same}, beat list identical {j_same}, bpm identical {bpm_same}");
        eprintln!("time: network {:.2} s, dbn {:.2} s (total over {n} windows)", t_net.as_secs_f64(), t_dbn.as_secs_f64());
    }
}

#[cfg(test)]
mod reference_benchmark {
    /// End to end against rekordbox's stored high precision grids, exact in rekordbox's own units:
    /// BPM x100, first beat in whole ms (`convertToMicrosec`), beat number of the first beat.
    /// PREPDECK_GT = oracle/rbgrids.tsv; PREPDECK_HP_MODE = 2 | 3 (default 2).
    /// Also times the standard analysis and the high precision one per track.
    #[test]
    #[ignore]
    fn hp_reference_grids() {
        let Ok(f) = std::env::var("PREPDECK_GT") else { return };
        let mode: u8 = std::env::var("PREPDECK_HP_MODE").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let text = std::fs::read_to_string(f).unwrap();
        let rows: Vec<(String, (i64, i64, i64))> = text
            .lines()
            .filter_map(|l| {
                let (p, b) = l.split_once('\t')?;
                let v: Vec<i64> = b.split(';').next()?.split(',').filter_map(|n| n.parse().ok()).collect();
                (v.len() == 3).then(|| (p.to_string(), (v[0], v[1], v[2])))
            })
            .collect();
        let results = std::sync::Mutex::new(vec![]);
        let threads: usize = std::env::var("PREPDECK_HP_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
        let chunks: Vec<_> = rows.chunks(rows.len().div_ceil(threads).max(1)).collect();
        std::thread::scope(|s| {
            for ch in chunks {
                let results = &results;
                s.spawn(move || {
                    for (p, (rb, rbpm, rms)) in ch {
                        let Ok(dec) = crate::audio::decode_file(p) else { continue };
                        let mono = dec.mono();
                        let t0 = std::time::Instant::now();
                        let (bpm, grid) = super::super::tempo_and_grid(&mono, dec.sr as f64, 70.0, 180.0);
                        let t_std = t0.elapsed().as_secs_f64();
                        let Some(g) = grid else { continue };
                        let t1 = std::time::Instant::now();
                        let out = super::analyze_model(&dec, mode, None).ok();
                        let t_hp = t1.elapsed().as_secs_f64();
                        let row = match out {
                            Some(o) => {
                                let (pos, num, _) = o.beats[0];
                                let ms = (1_000_000i64 * pos as i64 / dec.sr as i64) / 1000;
                                (p.clone(), *rb, *rbpm, *rms, Some(((o.bpm * 100.0).round() as i64, ms, num as i64)), t_std, t_hp)
                            }
                            None => (p.clone(), *rb, *rbpm, *rms, None, t_std, t_hp),
                        };
                        results.lock().unwrap().push(row);
                    }
                });
            }
        });
        let mut res = results.into_inner().unwrap();
        res.sort_by(|a, b| a.0.cmp(&b.0));
        let (mut n, mut bpm_ok, mut ms_ok, mut num_ok, mut all_ok, mut none) = (0, 0, 0, 0, 0, 0);
        let (mut ts, mut th) = (0f64, 0f64);
        for (p, rb, rbpm, rms, got, t_std, t_hp) in &res {
            n += 1;
            ts += t_std;
            th += t_hp;
            let name = p.rsplit('/').next().unwrap_or("");
            match got {
                None => {
                    none += 1;
                    eprintln!("  no high precision result :: {name}");
                }
                Some((b, ms, num)) => {
                    let (a, m, u) = (b == rbpm, ms == rms, num == rb);
                    bpm_ok += a as usize;
                    ms_ok += m as usize;
                    num_ok += u as usize;
                    all_ok += (a && m && u) as usize;
                    if !(a && m && u) {
                        eprintln!("  mine bpm {b} first {ms} ms beat {num} | rekordbox bpm {rbpm} first {rms} ms beat {rb} :: {name}");
                    }
                }
            }
        }
        eprintln!("high precision mode {mode}: tracks {n}, no result {none}, bpm identical {bpm_ok}, first beat ms identical {ms_ok}, beat number identical {num_ok}, all three {all_ok}");
        eprintln!("time per track: standard {:.2} s, high precision extra {:.2} s ({:+.0} %)", ts / n.max(1) as f64, th / n.max(1) as f64, 100.0 * th / ts.max(1e-9));
    }
}
