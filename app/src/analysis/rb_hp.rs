//! rekordbox's high precision BeatGrid analysis ("Use high precision BeatGrid analysis",
//! analysis modes 2 / 3 / 4 = Auto / Normal / Dynamic), ported from rekordbox 7.2.19.
//!
//! `AnalyzeJobBeatDetector::highPrecisionBeatAnalyze` (0x102327e50):
//! 1. the legacy `BeatAnalyzer_2_0` analysis of the whole track gives a BPM, a beat count and a
//!    start offset (`Analyzer::getOffsetCalc`);
//! 2. `madmom::analyze` (0x10230a820) runs madmom's downbeat network (`net`) on a 60 s window from
//!    that offset (the whole track when the tempo may change), decodes it with a bar-pointer DBN
//!    (`dbn`) and turns the beat times into a beat list (`madmom::get_beat_list`, `get_bpm`);
//! 3. `Analyzer::analyzeFocus` merges that list with the legacy analysis into the final grid.

#[path = "rb_hp_net.rs"]
pub mod net;
#[path = "rb_hp_dbn.rs"]
pub mod dbn;
#[path = "rb_hp_uba.rs"]
pub mod uba;

#[path = "rb_hp_focus.rs"]
pub mod focus;

use crate::model::BeatGrid;
use std::sync::OnceLock;
use uba::Uba;

/// Analysis modes as rekordbox numbers them (`CBeatAnalyzeThread::analyze`, field +14544).
pub const MODE_AUTO: u8 = 2;
#[allow(dead_code)]
pub const MODE_NORMAL: u8 = 3;
pub const MODE_DYNAMIC: u8 = 4;

/// The legacy analysis rekordbox runs first (`BeatAnalyzer_2_0`): its BPM and beat positions.
/// recordbox supplies its own estimate (see `legacy_from_grid`); rekordbox only uses the BPM for
/// its octave and integer-snap rules and the beats to pick the 60 s window.
#[derive(Clone, Debug)]
pub struct Legacy {
    /// BPM as the legacy library stores it (float, the same value x100 rounded is `BA_GetBpm`).
    pub bpm: f32,
    /// Beat positions in samples from the first beat on.
    pub beats: Vec<i32>,
}

pub fn legacy_from_grid(bpm: f64, first_beat: f64, sr: u32, total: usize) -> Option<Legacy> {
    let bpm100 = (bpm * 100.0).round();
    // The legacy ABI represents beats as integer sample positions and BPM x 100
    // as an integer. A sub-sample period cannot produce a distinct beat.
    if !(1.0..=6000.0 * sr as f64).contains(&bpm100) || !first_beat.is_finite() || sr == 0 {
        return None;
    }
    let b = (bpm100 / 100.0) as f32;
    let period = 60.0 / bpm100 * 100.0;
    let mut t = first_beat.rem_euclid(period);
    let mut beats = vec![];
    while ((t * sr as f64) as usize) < total {
        beats.push((t * sr as f64) as i32);
        t += period;
    }
    if beats.is_empty() {
        return None;
    }
    Some(Legacy { bpm: b, beats })
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

/// True when rekordbox's beat models are installed and readable.
pub fn available() -> bool {
    model(false).is_some()
}

/// The network + DBN + beat list stage for one window of the decode.
pub fn madmom_stage(samples: &[f32], sr: u32, start: usize, len: usize, hendou: bool) -> Option<(Vec<dbn::ListBeat>, f64, Vec<(f32, i32)>)> {
    let pcm = samples.get(start.checked_mul(2)?..start.checked_add(len)?.checked_mul(2)?)?;
    let net = if hendou { model(true).or_else(|| model(false))? } else { model(false)? };
    let mono: Vec<f32> = pcm.chunks_exact(2).map(|lr| (lr[0] + lr[1]) * 0.5).collect();
    let act = net.run(&mono);
    static DBN: OnceLock<dbn::Dbn> = OnceLock::new();
    let mut beats = DBN.get_or_init(dbn::Dbn::rekordbox).track(&act);
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
    pub offset: i32,
    pub window: (usize, usize),
}

/// rekordbox's high precision analysis of decoded audio. `legacy` is the standard analysis'
/// tempo and first beat (recordbox's own, standing in for `BeatAnalyzer_2_0`). Returns None
/// when the models are not installed or the audio is not 44.1 / 48 kHz.
pub fn analyze(dec: &crate::audio::Decoded, mode: u8, legacy: (f64, f64, u8)) -> Option<HpOutcome> {
    let sr = dec.sr;
    // UBA stores at most fifteen minutes of envelope data. Keep the existing grid
    // for longer sets or for the still-unimplemented tempo-change focus pass.
    if !matches!(sr, 44_100 | 48_000) || dec.frames() > 900 * sr as usize || mode == MODE_DYNAMIC {
        return None;
    }
    if !available() {
        return None;
    }
    let total = dec.frames();
    let (std_bpm, std_t0, std_beat) = legacy;
    let legacy = legacy_from_grid(legacy.0, legacy.1, sr, total)?;
    let pcm16 = uba::to_i16(&dec.samples);
    let env = Uba::new(sr as i32, &pcm16);
    let offset = get_offset_calc(&env, sr as i32, &legacy);
    let hendou = false;
    let (start, len) = madmom_window(total as i64, sr as i64, offset as i64, hendou);
    let (list, mbpm, _) = madmom_stage(&dec.samples, sr, start as usize, len as usize, hendou)?;
    let bpm100 = (legacy.bpm as f64 * 100.0).round() as i32;
    let ml: Vec<(i32, i32, i32)> = list.iter().map(|b| (b.pos as i32, b.num_bar, b.bpm)).collect();
    let input = focus::FocusInput { sr: sr as i32, total: total as i32, offset, hendou, legacy_bpm100: bpm100, legacy_count: legacy.beats.len() as i32, madmom: &ml, madmom_bpm: mbpm as i32 };
    // a reading past the end of a list in a branch not yet seen must not take the analysis down
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| focus::analyze_focus(&env, &input))).ok().flatten()?;
    if out.beats.is_empty() || out.bpm100 <= 0 {
        return None;
    }
    let mut beats = out.beats;
    // The bar-phase pass that follows UnitBeatAdjust in analyzeFocus (0x10116c050; decompiled
    // lines 2250-5222 of the constant path: band integrals at every beat, intro / outro onsets,
    // ranking and a vote over beat numbers) is not ported. The bar phase is taken from
    // recordbox's own estimate instead: the number of its beat nearest to the first beat.
    // PREPDECK_HP_BARPHASE=focus keeps the numbering the port has before that pass.
    if std::env::var("PREPDECK_HP_BARPHASE").as_deref() != Ok("focus") && std_bpm > 0.0 {
        let t = beats[0].0 as f64 / sr as f64;
        let k = ((t - std_t0) / (60.0 / std_bpm)).round() as i64;
        let n0 = (k + std_beat as i64 - 1).rem_euclid(4);
        for (i, b) in beats.iter_mut().enumerate() {
            b.1 = ((n0 + i as i64) % 4 + 1) as i32;
        }
    }
    let first = beats[0];
    let bpm = out.bpm100 as f64 / 100.0;
    let t0 = first.0 as f64 / sr as f64;
    let grid = BeatGrid::constant(t0, bpm, first.1.clamp(1, 4) as u8);
    Some(HpOutcome { bpm, grid, beats, offset, window: (start as usize, len as usize) })
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

/// Hook of `analysis::analyze_decoded`: with high precision on, the standard estimate becomes the
/// legacy input of rekordbox's high precision analysis and its grid replaces the standard one.
/// Falls back to the standard grid (and says so on stderr) when rekordbox's models are missing
/// or a part of the pipeline that is not ported is needed.
pub fn dispatch(dec: &crate::audio::Decoded, s: &super::AnalysisSettings, bpm: f64, grid: Option<BeatGrid>) -> (f64, Option<BeatGrid>) {
    let mode = s.model_grid_mode;
    // The model's dynamic focus pass is incomplete. Keep the fork's adaptive grid
    // without loading weights or allocating the full-track legacy analysis.
    if mode == 0 || mode == MODE_DYNAMIC || s.dynamic || !s.do_grid ||
        !(s.bpm_min.is_finite() && s.bpm_max.is_finite() && s.bpm_min > 0.0 && s.bpm_min <= s.bpm_max) {
        return (bpm, grid);
    }
    let Some((first, beat)) = grid.as_ref().and_then(|g| g.anchors.first()).map(|a| (a.time, a.beat)) else { return (bpm, grid) };
    match analyze(dec, mode, (bpm, first, beat)) {
        Some(o) => {
            let mut g = o.grid;
            let mut b = o.bpm;
            // convertToMicrosec: halve or double into the BPM range (constant tempo only)
            let dur = dec.duration();
            while b < s.bpm_min && b > 0.0 {
                g = g.doubled(dur);
                b *= 2.0;
            }
            while b > s.bpm_max {
                g = g.halved(dur);
                b *= 0.5;
            }
            (b, Some(g))
        }
        None => {
            eprintln!("high precision BeatGrid: not available for this track (mode {mode}); standard analysis used");
            (bpm, grid)
        }
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
            let Some(out) = super::focus::analyze_focus(&env, &inp) else {
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
                        let out = super::analyze(&dec, mode, (bpm, g.anchors[0].time, g.anchors[0].beat));
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
