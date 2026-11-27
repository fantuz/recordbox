//! Port of rekordbox 7.2.19's own beat analyser, library `BeatAnalyzer_1_0` (the library its
//! Analysis Mode "Normal" and "Dynamic" use; high precision uses `BeatAnalyzer_2_0` plus a
//! neural network and is not this file). Ported stage by stage from the arm64 slice and checked
//! against the original routine run on the same samples (`checks/beat-oracle-harness/`):
//!
//! 1. `sensor`: band filters, envelopes, peak picking, raw beat candidates (`BeatSensor`).
//! 2. `info`: tempo and the coarse grid from the candidates (`BeatInfoAnalyzer`).
//! 3. `env`: the grid's position and bar phase on a 344 Hz envelope (`UnitEnvelopeMaker`).
//! 4. `adjust`: fine tempo and position (`UnitBeatAdjust`, Normal mode only).
//!
//! `Analyzer::create` 0x101136258, `setSample` 0x1011364ec, `analyze` 0x101136890,
//! `BA_GetBeatList` 0x101156380; the caller's conversion to milliseconds is
//! `AnalyzeJobBeatDetector::convertToMicrosec` 0x102328b20.

use crate::model::BeatGrid;

mod adjust;
mod env;
mod info;
mod sensor;

/// `peakss::BeatInfo` (24 bytes): one beat (candidate) of the list the stages pass on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BeatInfo {
    /// Sample position.
    pub pos: i32,
    pub f1: f32,
    /// Beat interval in ms.
    pub iv: f32,
    /// In the sensor list: the update counter; after the tempo stage: the beat number 1..4.
    pub stat: i32,
    /// In the sensor list: the band; later a segment marker.
    pub band: i32,
    pub flag: u8,
}

/// What each stage produced, for comparison with the original's dumps.
#[derive(Default, Debug, Clone)]
pub struct Stages {
    pub sensor: Vec<BeatInfo>,
    /// After `BeatInfoAnalyzer`: the coarse grid and its BPM.
    pub info: Vec<BeatInfo>,
    pub info_bpm: f32,
    /// After `UnitEnvelopeMaker`.
    pub env: Vec<BeatInfo>,
    /// Final list and BPM (after `UnitBeatAdjust` in Normal mode).
    pub fin: Vec<BeatInfo>,
    pub bpm: f32,
    /// Debug trace (`PREPDECK_RBLOG`), see `sensor::Sensor::log`.
    pub log: Option<String>,
}

/// A decoded sample (±1.0) as rekordbox's analysis thread hands it to the library: its PCM
/// reader makes 16-bit values (`PcmReaderTemplate<Float2>::getStreamAt(int, short*, short*,
/// int)` 0x102853e3c: × 32768, above 32767 → 32767, otherwise below −32768 or NaN → −32768,
/// truncated toward zero) and `CBeatAnalyzeThread::getStreamAt` turns them back into floats.
pub fn reader_sample(x: f32) -> f32 {
    let v = x * 32768.0;
    let v = if v > 32767.0 {
        32767.0
    } else if !(v >= -32768.0) {
        -32768.0
    } else {
        v
    };
    (v as i32) as f32
}

/// The analyser on interleaved stereo: `samples` through `reader_sample` when `reader` is set
/// (what rekordbox does), otherwise `samples * scale` as they are. `keep` = also return the
/// intermediate stages.
fn pipeline(samples: &[f32], scale: f32, reader: bool, sr: u32, varied: bool, keep: bool) -> Stages {
    pipeline_with_front(samples, scale, reader, sr, varied, keep, None)
}

/// Record both analysis passes' common inputs only when requested by this job.
/// No worker keeps a previously decoded track's state.
pub(super) struct Front {
    pub(super) list: Vec<BeatInfo>,
    uem: env::Uem,
    pub(super) total: i32,
}

impl Front {
    pub(super) fn adjust_envelope(&mut self, bpm: f32, list: &mut Vec<BeatInfo>, varied: bool) {
        self.uem.analyze(bpm, list, varied);
    }

    pub(super) fn from_audio(samples: &[f32], sr: u32) -> Option<Self> {
        Some(feed(samples, 32768.0, true, sr, false, None)?.0)
    }
}

fn feed(samples: &[f32], scale: f32, reader: bool, sr: u32, log: bool,
    mut uba: Option<&mut adjust::Uba>) -> Option<(Front, Option<String>)> {
    let total = (samples.len() / 2) as i32;
    if !(sr == 44100 || sr == 48000) || total < 1 {
        return None;
    }
    let block = (sr / 75) as usize;
    let mut sensor = sensor::Sensor::new(sr as i32, total);
    let mut uem = env::Uem::new(sr as i32, total);
    let mut pcm16 = vec![0i16; 2 * block];
    if log {
        sensor.log = Some(String::new());
    }
    let conv = |x: f32| -> f32 {
        if reader {
            reader_sample(x)
        } else {
            x * scale
        }
    };
    let mut mono = vec![0f32; block];
    let mut done = 0usize;
    let frames = total as usize;
    while done < frames {
        let n = block.min(frames - done);
        for i in 0..n {
            let l = conv(samples[2 * (done + i)]);
            let r = conv(samples[2 * (done + i) + 1]);
            mono[i] = (r + l) * (1.0 / 65536.0);
            pcm16[2 * i] = (l as i32) as i16;
            pcm16[2 * i + 1] = (r as i32) as i16;
        }
        sensor.put(&mono[..n]);
        uem.put(&pcm16[..2 * n]);
        if let Some(u) = uba.as_deref_mut() {
            u.put(&pcm16[..2 * n]);
        }
        done += n;
    }
    Some((Front { list: sensor.list, uem, total }, sensor.log))
}

pub(super) fn bia_analyze(sr: i32, list: &mut Vec<BeatInfo>, out: &mut Vec<BeatInfo>, total: i32,
    varied: bool, seg_list: Option<&[BeatInfo]>) -> f32 {
    info::Bia { sr }.analyze_with(list, out, total, 4, 16, 12, varied, seg_list)
}

fn pipeline_with_front(samples: &[f32], scale: f32, reader: bool, sr: u32, varied: bool, keep: bool,
    saved: Option<&mut Option<Front>>) -> Stages {
    let mut st = Stages::default();
    let total = (samples.len() / 2) as i32;
    if !matches!(sr, 44_100 | 48_000) || total < 1 { return st; }
    let mut uba = (!varied).then(|| adjust::Uba::new(sr as i32, total));
    let log = keep && std::env::var_os("PREPDECK_RBLOG").is_some();
    let Some((Front { mut list, mut uem, .. }, log)) = feed(samples, scale, reader, sr, log, uba.as_mut()) else { return st };
    let bia = info::Bia { sr: sr as i32 };
    if let Some(saved) = saved {
        *saved = Some(Front { list: list.clone(), uem: uem.clone(), total });
    }
    if keep {
        st.sensor = list.clone();
        st.log = log;
    }
    let mut grid = vec![];
    st.info_bpm = bia.analyze(&mut list, &mut grid, total, 4, 16, 12, varied);
    if keep {
        st.info = grid.clone();
    }
    uem.analyze(st.info_bpm, &mut grid, varied);
    if keep {
        st.env = grid.clone();
    }
    let mut bpm = st.info_bpm;
    if let Some(u) = uba.as_mut() {
        u.analyze(&mut bpm, &mut grid);
    }
    st.fin = grid;
    st.bpm = bpm;
    st
}

/// `BA_GetBpm` (0x101156344 → `Analyzer::getBpm` 0x101136a00): BPM × 100, rounded.
fn bpm100(bpm: f32) -> i32 {
    bpm.mul_add(100.0, 0.5) as i32
}

/// `BA_GetBeatList`'s per-beat tempo × 100.
fn beat_bpm100(iv: f32) -> i32 {
    (6000000.0f32 / iv + 0.5) as i32
}

/// A beat's tempo for the grid: its own interval (60000 / `iv`), moved by at most 1e-9 BPM
/// when that would round to a different tempo × 100 than the library's f32 arithmetic lists.
fn beat_bpm(iv: f32) -> f64 {
    let listed = beat_bpm100(iv) as f64;
    let bpm = 60000.0 / iv as f64;
    if (bpm * 100.0).round() == listed {
        bpm
    } else if listed > bpm * 100.0 {
        (listed - 0.5) / 100.0 + 1e-9
    } else {
        (listed + 0.5) / 100.0 - 1e-9
    }
}

/// A beat's time as rekordbox converts it (`convertToMicrosec` 0x102328b20: whole
/// microseconds, `1000000 * pos / sr` in integers).
fn beat_time(pos: i32, sr: u32) -> f64 {
    ((1_000_000i64 * pos as i64) / sr as i64) as f64 / 1e6
}

/// The rate rekordbox analyses a file at (`CBeatAnalyzeThread::analyze` 0x1001528c4): 44.1 and
/// 48 kHz as they are; other rates go through its PCM reader's resampler, to 48 kHz when the
/// rate is a multiple of 8000 and to 44.1 kHz otherwise.
pub fn analysis_rate(sr: u32) -> u32 {
    if sr == 44100 {
        44100
    } else if sr % 8000 == 0 {
        48000
    } else {
        44100
    }
}

/// Interleaved stereo from `from` Hz to `to` Hz: windowed sinc (Blackman, 16 zero crossings,
/// cut-off at 0.95 of the lower Nyquist frequency). rekordbox's own resampler
/// (`rbxfrm::PcmReaderTemplate`) is not traced, so beats of such files are close to, not
/// identical with, rekordbox's.
fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    let frames = x.len() / 2;
    let ratio = to as f64 / from as f64;
    let out_frames = (frames as f64 * ratio).round() as usize;
    let fc = 0.95 * ratio.min(1.0);
    let half = 16.0 / fc;
    const P: usize = 512;
    let hn = (half * P as f64).ceil() as usize;
    let table: Vec<f64> = (0..=2 * hn + 1)
        .map(|k| {
            let t = (k as f64 - hn as f64) / P as f64;
            let u = (t / half).clamp(-1.0, 1.0);
            let w = 0.42 + 0.5 * (std::f64::consts::PI * u).cos() + 0.08 * (2.0 * std::f64::consts::PI * u).cos();
            let a = std::f64::consts::PI * fc * t;
            fc * (if a.abs() < 1e-12 { 1.0 } else { a.sin() / a }) * w
        })
        .collect();
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let c = i as f64 / ratio;
        let j0 = ((c - half).ceil() as i64).max(0);
        let j1 = ((c + half).floor() as i64).min(frames as i64 - 1);
        let (mut l, mut r) = (0.0f64, 0.0f64);
        for j in j0..=j1 {
            let pos = (j as f64 - c) * P as f64 + hn as f64;
            let k = pos.floor();
            let f = pos - k;
            let k = k as usize;
            let h = table[k] + (table[k + 1] - table[k]) * f;
            l += h * x[2 * j as usize] as f64;
            r += h * x[2 * j as usize + 1] as f64;
        }
        out.push(l as f32);
        out.push(r as f32);
    }
    out
}

/// The track BPM and beat grid rekordbox's Analysis Mode Normal (`dynamic` false) or Dynamic
/// produces for these samples (interleaved stereo, ±1.0), folded into `bpm_min..bpm_max` as
/// `convertToMicrosec` does. Files at rates other than 44.1 and 48 kHz are resampled first,
/// as rekordbox does (see `analysis_rate`). Unsupported inputs and unusable results allow
/// the caller to retry with its own detector. Silence has no beat grid.
pub fn tempo_and_grid(samples: &[f32], sr: u32, dynamic: bool, bpm_min: f64, bpm_max: f64) -> Option<(f64, Option<BeatGrid>)> {
    grid_with_front(samples, sr, dynamic, bpm_min, bpm_max, None)
}

pub(super) fn grid_with_front(samples: &[f32], sr: u32, dynamic: bool, bpm_min: f64,
    bpm_max: f64, saved: Option<&mut Option<Front>>) -> Option<(f64, Option<BeatGrid>)> {
    // UnitBeatAdjust retains fifteen minutes of signal; longer sets use the existing
    // estimator. Bound both the input rate and the duration before allocating a resample.
    if !(8_000..=384_000).contains(&sr) || samples.len() % 2 != 0 ||
        samples.len() / 2 > 900 * sr as usize || !(1.0..=1000.0).contains(&bpm_min) ||
        !(bpm_min..=1000.0).contains(&bpm_max) {
        return None;
    }
    let to = analysis_rate(sr);
    if to != sr {
        let x = resample(samples, sr, to);
        return tempo_and_grid(&x, to, dynamic, bpm_min, bpm_max);
    }
    let audio = &samples[..2 * (samples.len() / 2).saturating_sub(1)];
    let st = pipeline_with_front(audio, 32768.0, true, sr, dynamic, false, saved);
    let duration = samples.len() as f64 / 2.0 / sr as f64;
    let (bpm, grid) = grid_from(&st, sr, dynamic, duration, bpm_min, bpm_max);
    if let Some(g) = &grid {
        if !(bpm_min..=bpm_max).contains(&bpm) || g.anchors.is_empty() ||
            g.anchors.iter().any(|a| !(0.0..=duration).contains(&a.time) || !a.bpm.is_finite() || a.bpm <= 0.0) ||
            g.anchors.windows(2).any(|w| w[0].time >= w[1].time) {
            return None;
        }
    }
    Some((bpm, grid))
}

/// The library's final beat list as recordbox's grid, folded into `bpm_min..bpm_max`.
fn grid_from(st: &Stages, sr: u32, dynamic: bool, duration: f64, bpm_min: f64, bpm_max: f64) -> (f64, Option<BeatGrid>) {
    let beats = &st.fin;
    if beats.is_empty() || !st.bpm.is_finite() || st.bpm <= 0.0 ||
        beats.iter().any(|b| b.pos < 0 || !b.iv.is_finite() || b.iv <= 0.0) ||
        beats.windows(2).any(|w| w[0].pos >= w[1].pos) {
        return (0.0, None);
    }
    let mut v19 = bpm100(st.bpm);
    if v19 <= 0 {
        v19 = beat_bpm100(beats[0].iv);
    }
    let beat_no = |b: &BeatInfo| b.stat.clamp(1, 4) as u8;
    let mut grid = if !dynamic {
        let t0 = beat_time(beats[0].pos, sr);
        BeatGrid::constant(t0, st.bpm as f64, beat_no(&beats[0]))
    } else {
        // rekordbox keeps every beat of the list (position, beat number, tempo × 100). Here: an
        // anchor at the first beat, wherever the listed tempo × 100 changes, and wherever a beat
        // is more than 0.5 ms (half of the millisecond rekordbox stores) from where the current
        // anchor puts it. An anchor's tempo is its beat's own (60000 / interval), so every beat
        // shows the tempo rekordbox lists for it.
        const TOL: f64 = 0.0005;
        let mut anchors: Vec<crate::model::GridAnchor> = vec![];
        let mut cur = (0usize, 0.0f64, 0.0f64); // anchor beat index, time, bpm
        let mut prev_t = 0.0;
        for (j, b) in beats.iter().enumerate() {
            let t = beat_time(b.pos, sr);
            let fits = j > 0 && {
                let (a, ta, ba) = cur;
                (t - (ta + (j - a) as f64 * 60.0 / ba)).abs() <= TOL && beat_bpm100(b.iv) == beat_bpm100(beats[a].iv)
            };
            if !fits {
                if let Some(last) = anchors.last_mut() {
                    // `BeatGrid::beats` gives a beat less than half a period before the next
                    // anchor to that anchor; keep the previous beat on the grid if it is.
                    let p = 60.0 / last.bpm;
                    if t - prev_t <= 0.5 * p {
                        if (last.time - prev_t).abs() < 1e-9 {
                            last.bpm = 60.0 / (t - prev_t);
                        } else {
                            anchors.push(crate::model::GridAnchor { time: prev_t, bpm: 60.0 / (t - prev_t), beat: beat_no(&beats[j - 1]) });
                        }
                    }
                }
                let bpm = beat_bpm(b.iv);
                anchors.push(crate::model::GridAnchor { time: t, bpm, beat: beat_no(b) });
                cur = (j, t, bpm);
            }
            prev_t = t;
        }
        BeatGrid { anchors }
    };
    let (lo, hi) = ((bpm_min * 100.0).round() as i32, (bpm_max * 100.0).round() as i32);
    if v19 < lo || v19 > hi {
        while v19 < lo && v19 > 0 {
            grid = grid.doubled(duration);
            v19 *= 2;
        }
        while v19 > hi {
            grid = grid.halved(duration);
            v19 = (if v19 >= 0 { v19 } else { v19 + 1 }) >> 1;
        }
    }
    (v19 as f64 / 100.0, Some(grid))
}

#[cfg(test)]
pub(super) mod test_signals {
    pub(crate) type Program = (f64, fn(f64) -> f64, u64, Option<(f64, f64)>);
    /// Tempo programs: constant, steps, ramps, jitter, a silent break, slow and fast tempi.
    pub(crate) const SYNTH: [Program; 10] = [
        (150.0, |_| 124.0, 1, None),
        (180.0, |t| if t < 60.0 { 120.0 } else { 126.0 }, 2, None),
        (200.0, |t| 118.0 + 14.0 * (t / 200.0), 3, None),
        (180.0, |t| if t < 90.0 { 128.0 } else { 86.0 }, 4, None),
        (160.0, |t| 100.0 * (1.0 + 0.01 * (t * 1.7).sin() + 0.006 * (t * 0.31).sin()), 5, None),
        (200.0, |_| 126.0, 6, Some((80.0, 110.0))),
        (150.0, |_| 174.0, 7, None),
        (150.0, |_| 72.0, 8, None),
        (240.0, |t| if t < 70.0 { 124.0 } else if t < 150.0 { 132.0 } else { 124.0 }, 9, None),
        (200.0, |t| 90.0 + 50.0 * (t / 200.0), 10, None),
    ];

    /// Synthetic drum loops with a tempo program (beats per minute as a function of time), for
    /// checking the multi-segment (Dynamic) paths: kick on every beat, snare on 2 and 4, hats
    /// on the off-beats, a little noise. Returns interleaved stereo in [-1, 1] at 44.1 kHz.
    pub(crate) fn synth(secs: f64, tempo: &dyn Fn(f64) -> f64, seed: u64, gap: Option<(f64, f64)>) -> Vec<f32> {
        let sr = 44100.0;
        let n = (secs * sr) as usize;
        let mut v = vec![0f32; n];
        let mut rng = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut rnd = move || {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((rng >> 33) as f64 / (1u64 << 31) as f64) - 0.5
        };
        let mut t = 0.37;
        let mut k = 0usize;
        while t < secs {
            let silent = gap.map(|(a, b)| t >= a && t < b).unwrap_or(false);
            let p = 60.0 / tempo(t);
            if !silent {
                let start = (t * sr) as usize;
                for i in 0..(0.12 * sr) as usize {
                    if start + i >= n {
                        break;
                    }
                    let x = i as f64 / sr;
                    let f = 50.0 + 90.0 * (-x / 0.03).exp();
                    v[start + i] += (0.8 * (-x / 0.06).exp() * (2.0 * std::f64::consts::PI * f * x).sin()) as f32;
                    if k % 2 == 1 && i < (0.08 * sr) as usize {
                        v[start + i] += (0.35 * (-x / 0.04).exp() * rnd() * 2.0) as f32;
                    }
                }
                let h = ((t + p / 2.0) * sr) as usize;
                for i in 0..(0.03 * sr) as usize {
                    if h + i < n {
                        v[h + i] += (0.15 * (-(i as f64) / (0.008 * sr)).exp() * rnd() * 2.0) as f32;
                    }
                }
            }
            t += p;
            k += 1;
        }
        let mut out = Vec::with_capacity(2 * n);
        for x in v {
            let y = (x + (rnd() * 0.002) as f32).clamp(-1.0, 1.0);
            out.push(y);
            out.push(y);
        }
        out
    }

}

#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
mod oracle {
    //! `PREPDECK_RBBEAT=<dir with rbbeat and fixups.txt> PREPDECK_RBSLICE=<arm64 slice>
    //! PREPDECK_GT=<tsv: path TAB ...> cargo test --release -- --ignored rb_beat1_oracle --nocapture`
    use super::*;
    use super::test_signals::{synth, SYNTH};

    fn parse_list(text: &str) -> Vec<BeatInfo> {
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let v: Vec<&str> = l.split_whitespace().collect();
                if v.len() < 6 {
                    return None;
                }
                let f5: f32 = v[5].parse().ok()?;
                Some(BeatInfo { pos: v[0].parse().ok()?, f1: v[1].parse().ok()?, iv: v[2].parse().ok()?, stat: v[3].parse().ok()?, band: v[4].parse().ok()?, flag: f5.to_bits() as u8 })
            })
            .collect()
    }

    struct OracleOut {
        sensor: Vec<BeatInfo>,
        info: Vec<BeatInfo>,
        info_bpm: f32,
        env: Vec<BeatInfo>,
        fin: Vec<BeatInfo>,
        fin_bpm: f32,
        /// What the library hands its caller: `BA_GetBpm` and `BA_GetBeatList` (position,
        /// beat number, tempo × 100 per beat).
        api_bpm: i32,
        api: Vec<(i32, i32, i32)>,
        secs: f64,
    }

    fn parse_bpm(text: &str) -> f32 {
        text.lines().find_map(|l| l.strip_prefix("# bpm ")).and_then(|v| v.trim().parse().ok()).unwrap_or(f32::NAN)
    }

    /// A scratch folder that is removed when dropped, so a track's PCM copy (about 100 MB) goes
    /// as soon as the track is compared, also when the comparison stops part-way.
    struct TmpDir(std::path::PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl std::ops::Deref for TmpDir {
        type Target = std::path::Path;
        fn deref(&self) -> &std::path::Path {
            &self.0
        }
    }

    fn tmp_root() -> std::path::PathBuf {
        std::env::var("PREPDECK_RBTMP").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir())
    }

    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    /// Removes `rbbeat1-<pid>-*` folders left by runs that were killed (their process is gone).
    fn sweep_stale() {
        let Ok(rd) = std::fs::read_dir(tmp_root()) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(rest) = name.strip_prefix("rbbeat1-") else { continue };
            let pid: i32 = rest.split('-').next().and_then(|p| p.parse().ok()).unwrap_or(0);
            if pid > 0 && unsafe { kill(pid, 0) } != 0 {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    fn run_oracle(dir: &str, slice: &str, d: &crate::audio::Decoded, tag: usize, varied: bool) -> Option<OracleOut> {
        let tmp = TmpDir(tmp_root().join(format!("rbbeat1-{}-{tag}", std::process::id())));
        let _ = std::fs::create_dir_all(&*tmp);
        let pcm = tmp.join("pcm.f32");
        let bytes: Vec<u8> = d.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&pcm, bytes).ok()?;
        let sens = tmp.join("sensor.txt");
        let infof = tmp.join("info.txt");
        let t0 = std::time::Instant::now();
        let mut cmd = std::process::Command::new(format!("{dir}/rbbeat"));
        if let Ok(dd) = std::env::var("PREPDECK_RBDIFF") {
            if std::env::var("PREPDECK_RBLOG").is_ok() {
                cmd.env("RB_LOG_CALCBEAT", format!("{dd}/{tag}.oracle.log"));
            }
        }
        let out = cmd
            .current_dir(dir)
            .env("RB_SCALE", "32768")
            .env("RB_ALL", "1")
            .env("RB_DUMP_SENSOR", &sens)
            .env("RB_DUMP_LIST", &infof)
            .env("RB_SKIP_UEM", "1")
            .env("RB_SKIP_UBA", "1")
            .args([slice, "fixups.txt", pcm.to_str().unwrap(), &d.sr.to_string(), "1", if varied { "1" } else { "0" }])
            .output()
            .ok()?;
        let secs = t0.elapsed().as_secs_f64();
        let _ = out;
        let sensor = parse_list(&std::fs::read_to_string(&sens).ok()?);
        let it = std::fs::read_to_string(&infof).ok()?;
        let info = parse_list(&it);
        let info_bpm = parse_bpm(&it);
        let envf = tmp.join("env.txt");
        std::process::Command::new(format!("{dir}/rbbeat"))
            .current_dir(dir)
            .env("RB_SCALE", "32768")
            .env("RB_ALL", "1")
            .env("RB_DUMP_LIST", &envf)
            .env("RB_SKIP_UBA", "1")
            .args([slice, "fixups.txt", pcm.to_str().unwrap(), &d.sr.to_string(), "1", if varied { "1" } else { "0" }])
            .output()
            .ok()?;
        let env = parse_list(&std::fs::read_to_string(&envf).ok()?);
        let finf = tmp.join("fin.txt");
        let out = std::process::Command::new(format!("{dir}/rbbeat"))
            .current_dir(dir)
            .env("RB_SCALE", "32768")
            .env("RB_ALL", "1")
            .env("RB_DUMP_LIST", &finf)
            .args([slice, "fixups.txt", pcm.to_str().unwrap(), &d.sr.to_string(), "1", if varied { "1" } else { "0" }])
            .output()
            .ok()?;
        let ft = std::fs::read_to_string(&finf).ok()?;
        let fin = parse_list(&ft);
        let fin_bpm = parse_bpm(&ft);
        // stdout: the library's own debug prints, then "bpm B beats N ..." and (with RB_ALL) one
        // "pos beat tempo ms" line per beat.
        let so = String::from_utf8_lossy(&out.stdout);
        let mut lines = so.lines().skip_while(|l| !l.starts_with("bpm "));
        let api_bpm = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
        let api = lines
            .filter_map(|l| {
                let w: Vec<i32> = l.split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
                (w.len() == 3).then(|| (w[0], w[1], w[2]))
            })
            .collect();
        drop(tmp);
        Some(OracleOut { sensor, info, info_bpm, env, fin, fin_bpm, api_bpm, api, secs })
    }

    fn first_diff_full(a: &[BeatInfo], b: &[BeatInfo]) -> Option<usize> {
        let key = |x: &BeatInfo| (x.pos, x.f1.to_bits(), x.iv.to_bits(), x.stat, x.band, x.flag);
        (0..a.len().max(b.len())).find(|&i| a.get(i).map(key) != b.get(i).map(key))
    }

    fn first_diff(a: &[BeatInfo], b: &[BeatInfo]) -> Option<usize> {
        (0..a.len().max(b.len())).find(|&i| a.get(i).map(|x| (x.pos, x.f1.to_bits(), x.iv.to_bits(), x.stat, x.band)) != b.get(i).map(|x| (x.pos, x.f1.to_bits(), x.iv.to_bits(), x.stat, x.band)))
    }

    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }
    extern "C" {
        fn clock_gettime(id: u32, tp: *mut Timespec) -> i32;
    }
    /// CPU time of the calling thread (CLOCK_THREAD_CPUTIME_ID), so a loaded machine changes
    /// the comparison less than wall time does.
    fn thread_cpu() -> f64 {
        let mut t = Timespec { sec: 0, nsec: 0 };
        unsafe { clock_gettime(16, &mut t) };
        t.sec as f64 + 1e-9 * t.nsec as f64
    }

    /// Speed, one track at a time, in thread CPU seconds: the port (Normal and Dynamic) against
    /// the original routine on the same PCM (the harness times Create + setSample, which
    /// includes the analysis, and leaves out its own start-up), plus recordbox's own estimator.
    /// `PREPDECK_RBBEAT`, `PREPDECK_RBSLICE`, `PREPDECK_GT`, `PREPDECK_LIMIT` as above.
    #[test]
    #[ignore]
    fn rb_beat1_speed() {
        let (Ok(dir), Ok(slice), Ok(gt)) = (std::env::var("PREPDECK_RBBEAT"), std::env::var("PREPDECK_RBSLICE"), std::env::var("PREPDECK_GT")) else { return };
        let limit: usize = std::env::var("PREPDECK_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
        let paths: Vec<String> = std::fs::read_to_string(gt).unwrap().lines().filter_map(|l| l.split('\t').next()).map(|s| s.to_string()).filter(|p| std::path::Path::new(p).exists()).take(limit).collect();
        sweep_stale();
        let tmp = TmpDir(tmp_root().join(format!("rbbeat1-{}-speed", std::process::id())));
        let _ = std::fs::create_dir_all(&*tmp);
        // [port normal, port dynamic, original normal, original dynamic, own estimator] as (cpu, wall)
        let mut t = [(0.0f64, 0.0f64); 5];
        let (mut n, mut audio) = (0, 0.0);
        let oracle = |pcm: &std::path::Path, sr: u32, varied: bool| -> Option<(f64, f64)> {
            let out = std::process::Command::new(format!("{dir}/rbbeat")).current_dir(&dir).env("RB_SCALE", "32768").env("RB_TIME", "1").args([slice.as_str(), "fixups.txt", pcm.to_str().unwrap(), &sr.to_string(), "1", if varied { "1" } else { "0" }]).output().ok()?;
            let e = String::from_utf8_lossy(&out.stderr);
            let l = e.lines().find(|l| l.starts_with("rbtime "))?;
            let w: Vec<f64> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            Some((w[0], w[1]))
        };
        for p in &paths {
            let Ok(d) = crate::audio::decode_file(p) else { continue };
            let pcm = tmp.join("pcm.f32");
            let bytes: Vec<u8> = d.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(&pcm, bytes).unwrap();
            let (Some(on), Some(od)) = (oracle(&pcm, d.sr, false), oracle(&pcm, d.sr, true)) else { continue };
            let mut run = |k: usize, f: &mut dyn FnMut()| {
                let (c, w) = (thread_cpu(), std::time::Instant::now());
                f();
                t[k].0 += thread_cpu() - c;
                t[k].1 += w.elapsed().as_secs_f64();
            };
            run(0, &mut || drop(tempo_and_grid(&d.samples, d.sr, false, 70.0, 180.0)));
            run(1, &mut || drop(tempo_and_grid(&d.samples, d.sr, true, 70.0, 180.0)));
            let mono = d.mono();
            run(4, &mut || drop(crate::analysis::tempo_and_grid(&mono, d.sr as f64, 70.0, 180.0)));
            t[2].0 += on.0;
            t[2].1 += on.1;
            t[3].0 += od.0;
            t[3].1 += od.1;
            audio += d.duration();
            n += 1;
        }
        drop(tmp);
        let k = n.max(1) as f64;
        let f = |i: usize| format!("{:.3} s cpu ({:.3} s wall)", t[i].0 / k, t[i].1 / k);
        eprintln!("speed: {n} tracks, {:.0} s of audio, per track:", audio);
        eprintln!("speed Normal: port {}, original {}", f(0), f(2));
        eprintln!("speed Dynamic: port {}, original {}", f(1), f(3));
        eprintln!("speed own estimator (PREPDECK_BEAT=own): {}", f(4));
    }

    #[test]
    #[ignore]
    fn rb_beat1_oracle() {
        let (Ok(dir), Ok(slice)) = (std::env::var("PREPDECK_RBBEAT"), std::env::var("PREPDECK_RBSLICE")) else { return };
        let gt = std::env::var("PREPDECK_GT").unwrap_or_default();
        let limit: usize = std::env::var("PREPDECK_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
        // PREPDECK_RBVARIED=1: Analysis Mode Dynamic (the library's "varied" analyser).
        let varied = std::env::var("PREPDECK_RBVARIED").map(|v| v == "1").unwrap_or(false);
        let mut paths: Vec<String> = if gt.is_empty() { vec![] } else { std::fs::read_to_string(gt).unwrap().lines().filter_map(|l| l.split('\t').next()).map(|s| s.to_string()).filter(|p| std::path::Path::new(p).exists()).take(limit).collect() };
        // PREPDECK_AUDIO: more files (joined by '|'); PREPDECK_RBSYNTH=1: synthetic tempo programs.
        for p in std::env::var("PREPDECK_AUDIO").unwrap_or_default().split('|').filter(|p| !p.is_empty()) {
            paths.push(p.to_string());
        }
        if std::env::var("PREPDECK_RBSYNTH").is_ok() {
            for k in 0..SYNTH.len() {
                paths.push(format!("synth:{k}"));
            }
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let rows = std::sync::Mutex::new(vec![]);
        // Each thread holds at most one scratch folder at a time, so this also caps how many
        // PCM copies exist at once.
        sweep_stale();
        let threads: usize = std::env::var("PREPDECK_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(4).clamp(1, 6);
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(p) = paths.get(i) else { break };
                    let d = if let Some(k) = p.strip_prefix("synth:") {
                        let (secs, f, seed, gap) = SYNTH[k.parse::<usize>().unwrap()];
                        let samples = synth(secs, &f, seed, gap);
                        let frames = samples.len() / 2;
                        crate::audio::Decoded { sr: 44100, samples, raw_frames: frames, damaged: 0, truncated: false }
                    } else {
                        let Ok(d) = crate::audio::decode_file(p) else { continue };
                        d
                    };
                    // By default both sides see what rekordbox's reader delivers: the port takes
                    // the decoded floats through `reader_sample` (the path the app uses), the
                    // original gets those 16-bit values. PREPDECK_RBRAW=1: unquantized floats.
                    let raw = std::env::var("PREPDECK_RBRAW").is_ok();
                    let duration = d.samples.len() as f64 / 2.0 / d.sr as f64;
                    let sr = d.sr;
                    let t0 = std::time::Instant::now();
                    let mine = pipeline(&d.samples, 32768.0, !raw, d.sr, varied, true);
                    let my_secs = t0.elapsed().as_secs_f64();
                    let fed = if raw { d } else { crate::audio::Decoded { samples: d.samples.iter().map(|&x| reader_sample(x) / 32768.0).collect(), ..d } };
                    let Some(o) = run_oracle(&dir, &slice, &fed, i, varied) else { continue };
                    let diff = first_diff(&mine.sensor, &o.sensor);
                    if diff.is_some() {
                        if let Ok(dd) = std::env::var("PREPDECK_RBDIFF") {
                            let fmt = |v: &[BeatInfo]| v.iter().map(|b| format!("{} {:.9} {:.9} {} {} {}\n", b.pos, b.f1, b.iv, b.stat, b.band, b.flag)).collect::<String>();
                            let _ = std::fs::write(format!("{dd}/{i}.mine.sensor"), fmt(&mine.sensor));
                            let _ = std::fs::write(format!("{dd}/{i}.oracle.sensor"), fmt(&o.sensor));
                            if let Some(l) = &mine.log {
                                let _ = std::fs::write(format!("{dd}/{i}.mine.log"), l);
                            }
                        }
                    }
                    let mut line = match diff {
                        None => format!("sensor equal ({} entries)", o.sensor.len()),
                        Some(k) => format!("sensor differs at {k} of {}/{}: mine {:?} oracle {:?}", mine.sensor.len(), o.sensor.len(), mine.sensor.get(k), o.sensor.get(k)),
                    };
                    let d2 = first_diff_full(&mine.info, &o.info);
                    let info_ok = d2.is_none() && mine.info_bpm.to_bits() == o.info_bpm.to_bits();
                    line += &match d2 {
                        None => format!(" | info equal ({} beats, bpm {} / {})", o.info.len(), mine.info_bpm, o.info_bpm),
                        Some(k) => format!(" | info differs at {k} of {}/{}: mine {:?} oracle {:?} (bpm {} / {})", mine.info.len(), o.info.len(), mine.info.get(k), o.info.get(k), mine.info_bpm, o.info_bpm),
                    };
                    let d3 = first_diff_full(&mine.env, &o.env);
                    let env_ok = d3.is_none();
                    line += &match d3 {
                        None => format!(" | env equal ({} beats)", o.env.len()),
                        Some(k) => format!(" | env differs at {k} of {}/{}: mine {:?} oracle {:?}", mine.env.len(), o.env.len(), mine.env.get(k), o.env.get(k)),
                    };
                    let d4 = first_diff_full(&mine.fin, &o.fin);
                    let fin_ok = d4.is_none() && mine.bpm.to_bits() == o.fin_bpm.to_bits();
                    line += &match d4 {
                        None => format!(" | final equal ({} beats, bpm {} / {})", o.fin.len(), mine.bpm, o.fin_bpm),
                        Some(k) => format!(" | final differs at {k} of {}/{}: mine {:?} oracle {:?} (bpm {} / {})", mine.fin.len(), o.fin.len(), mine.fin.get(k), o.fin.get(k), mine.bpm, o.fin_bpm),
                    };
                    // The library's output as its caller reads it.
                    let my_api: Vec<(i32, i32, i32)> = mine.fin.iter().map(|b| (b.pos, b.stat, beat_bpm100(b.iv))).collect();
                    let api_ok = my_api == o.api && bpm100(mine.bpm) == o.api_bpm;
                    if !api_ok {
                        let k = (0..my_api.len().max(o.api.len())).find(|&i| my_api.get(i) != o.api.get(i));
                        line += &format!(" | API differs: bpm {} / {}, first beat difference at {k:?}", bpm100(mine.bpm), o.api_bpm);
                    }
                    // recordbox's grid (no BPM folding) against every beat the library returns:
                    // the largest time difference to the nearest grid beat, and whether that
                    // grid beat has the library's beat number.
                    let (_, g) = grid_from(&mine, sr, varied, duration, 0.0, 1e9);
                    let g_anchors = g.as_ref().map_or(0, |g| g.anchors.len());
                    let gb = g.map(|g| g.beats(duration)).unwrap_or_default();
                    let (mut worst, mut nums, mut tempos) = (0.0f64, 0usize, 0usize);
                    for &(pos, no, tempo) in &o.api {
                        let t = beat_time(pos, sr);
                        let k = gb.partition_point(|b| b.time < t);
                        let near = [k.checked_sub(1), Some(k)].into_iter().flatten().filter_map(|j| gb.get(j)).min_by(|x, y| (x.time - t).abs().total_cmp(&(y.time - t).abs()));
                        match near {
                            Some(b) => {
                                worst = worst.max((b.time - t).abs());
                                nums += (b.beat as i32 == no.clamp(1, 4)) as usize;
                                tempos += ((b.bpm * 100.0).round() as i32 == tempo) as usize;
                            }
                            None => worst = f64::INFINITY,
                        }
                    }
                    let anchors = g_anchors;
                    line += &format!(" | grid max {:.3} ms, beat numbers {nums}/{}, tempo x100 {tempos}/{}, {anchors} anchors", worst * 1e3, o.api.len(), o.api.len());
                    let grid = (worst, nums == o.api.len(), tempos == o.api.len());
                    rows.lock().unwrap().push((p.rsplit('/').next().unwrap().to_string(), diff.is_none(), line, my_secs, o.secs, info_ok, env_ok, fin_ok && api_ok, grid));
                });
            }
        });
        let rows = rows.into_inner().unwrap();
        for r in &rows {
            eprintln!("{} :: {}", r.2, r.0);
        }
        let n = rows.len();
        let eq = rows.iter().filter(|r| r.1).count();
        let ieq = rows.iter().filter(|r| r.5).count();
        let eeq = rows.iter().filter(|r| r.6).count();
        let feq = rows.iter().filter(|r| r.7).count();
        eprintln!("tracks {n}  sensor equal {eq}  info equal {ieq}  env equal {eeq}  final equal {feq} (list, bpm, BA_GetBpm and BA_GetBeatList)");
        let gmax = rows.iter().map(|r| r.8 .0).fold(0.0, f64::max);
        let gnum = rows.iter().filter(|r| r.8 .1).count();
        let gtempo = rows.iter().filter(|r| r.8 .2).count();
        eprintln!("grid vs the library's beats: largest difference {:.3} ms; every beat number equal on {gnum} of {n} tracks, every tempo x100 on {gtempo} of {n}", gmax * 1e3);
        eprintln!("wall time per track ({threads} at once; rb_beat1_speed measures speed): port {:.3} s, oracle {:.3} s", rows.iter().map(|r| r.3).sum::<f64>() / n.max(1) as f64, rows.iter().map(|r| r.4).sum::<f64>() / n.max(1) as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kick on every beat (louder on 1), hats on the off-beats, at 44.1 kHz stereo.
    fn clicks(bpm: f64, secs: f64, offset: f64) -> Vec<f32> {
        clicks_at(bpm, secs, offset, 44100.0)
    }

    fn clicks_at(bpm: f64, secs: f64, offset: f64, sr: f64) -> Vec<f32> {
        let n = (secs * sr) as usize;
        let mut v = vec![0f32; n];
        let p = 60.0 / bpm;
        let mut t = offset;
        let mut k = 0;
        while t < secs {
            let s0 = (t * sr) as usize;
            let amp = if k % 4 == 0 { 0.9 } else { 0.6 };
            for i in 0..(0.1 * sr) as usize {
                if s0 + i < n {
                    let x = i as f64 / sr;
                    v[s0 + i] += (amp * (-x / 0.04).exp() * (2.0 * std::f64::consts::PI * (55.0 + 80.0 * (-x / 0.02).exp()) * x).sin()) as f32;
                }
            }
            let h = ((t + p / 2.0) * sr) as usize;
            for i in 0..(0.02 * sr) as usize {
                if h + i < n {
                    let i44 = (i as f64 * 44100.0 / sr) as usize;
                    v[h + i] += (((i44 * 7919 % 97) as f32 / 97.0) - 0.5) * 0.2 * (-(i44 as f32) / 300.0).exp();
                }
            }
            t += p;
            k += 1;
        }
        v.iter().flat_map(|x| [*x, *x]).collect()
    }

    #[test]
    fn shared_sensor_data_belongs_to_the_current_analysis() {
        let mut pcm = clicks(120.0, 12.0, 0.1);
        let mut shared = None;
        grid_with_front(&pcm, 44_100, false, 70.0, 180.0, Some(&mut shared));
        let shared = shared.unwrap();
        let positions = |front: &Front| front.list.iter().map(|b| b.pos).collect::<Vec<_>>();
        let separate = Front::from_audio(&pcm[..pcm.len()-2], 44_100).unwrap();
        assert!(!shared.list.is_empty());
        assert_eq!(positions(&shared), positions(&separate));
        // Reuse the same allocation for another track of identical duration.
        // The fresh sensor pass must see silence, even while the first Front exists.
        pcm.fill(0.0);
        assert!(Front::from_audio(&pcm[..pcm.len()-2], 44_100).unwrap().list.is_empty());
    }

    #[test]
    fn steady_tempo_and_phase() {
        for &(bpm, off) in &[(128.0, 0.31), (122.0, 0.05), (140.0, 0.2), (100.0, 0.12)] {
            let x = clicks(bpm, 45.0, off);
            let (got, grid) = tempo_and_grid(&x, 44100, false, 70.0, 180.0).unwrap();
            assert!((got - bpm).abs() < 0.06, "bpm {bpm}: got {got}");
            let g = grid.unwrap();
            let p = 60.0 / bpm;
            let first = g.anchors[0].time;
            let d = ((first - off) / p - ((first - off) / p).round()).abs() * p;
            assert!(d < 0.015, "phase {d} s at {bpm}");
            assert!((g.anchors[0].bpm - bpm).abs() < 0.06);
        }
    }

    #[test]
    fn dynamic_follows_a_tempo_step() {
        let x = test_signals::synth(150.0, &|t| if t < 70.0 { 120.0 } else { 128.0 }, 3, None);
        let (_, grid) = tempo_and_grid(&x, 44100, true, 70.0, 180.0).unwrap();
        let g = grid.unwrap();
        let near = |b: f64| g.anchors.iter().any(|a| (a.bpm - b).abs() < 0.5);
        assert!(near(120.0) && near(128.0), "{:?}", g.anchors.iter().map(|a| (a.time, a.bpm)).collect::<Vec<_>>());
        // Normal mode keeps one tempo.
        let (_, gn) = tempo_and_grid(&x, 44100, false, 70.0, 180.0).unwrap();
        assert_eq!(gn.unwrap().anchors.len(), 1);
    }

    #[test]
    fn other_rates_are_resampled_first() {
        assert_eq!((analysis_rate(44100), analysis_rate(48000)), (44100, 48000));
        assert_eq!((analysis_rate(96000), analysis_rate(32000), analysis_rate(16000)), (48000, 48000, 48000));
        assert_eq!((analysis_rate(88200), analysis_rate(22050), analysis_rate(11025)), (44100, 44100, 44100));
        // A 1 kHz tone keeps its level and frequency through 96 → 48 kHz and 22.05 → 44.1 kHz.
        for &(from, to) in &[(96000u32, 48000u32), (22050, 44100)] {
            let x: Vec<f32> = (0..from as usize).flat_map(|i| {
                let v = (0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / from as f64).sin()) as f32;
                [v, v]
            }).collect();
            let y = resample(&x, from, to);
            assert_eq!(y.len(), 2 * to as usize);
            let err = (100..to as usize - 100).map(|i| (y[2 * i] as f64 - 0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / to as f64).sin()).abs()).fold(0.0, f64::max);
            assert!(err < 2e-3, "{from} -> {to}: {err}");
        }
        for &sr in &[96000.0, 88200.0] {
            let x = clicks_at(126.0, 45.0, 0.2, sr);
            let (got, grid) = tempo_and_grid(&x, sr as u32, false, 70.0, 180.0).unwrap();
            assert!((got - 126.0).abs() < 0.06, "{sr}: {got}");
            let first = grid.unwrap().anchors[0].time;
            let p = 60.0 / 126.0;
            let d = ((first - 0.2) / p - ((first - 0.2) / p).round()).abs() * p;
            assert!(d < 0.015, "{sr}: phase {d}");
        }
    }

    #[test]
    fn dynamic_grid_keeps_every_listed_beat_and_tempo() {
        // A step, a ramp and a step back: every beat the library lists lands on the grid within
        // 0.5 ms, with its beat number and its tempo × 100.
        for k in [1usize, 3, 8, 9] {
            let (secs, f, seed, gap) = test_signals::SYNTH[k];
            let x = test_signals::synth(secs, &f, seed, gap);
            let st = pipeline(&x, 32768.0, true, 44100, true, false);
            let duration = x.len() as f64 / 2.0 / 44100.0;
            let (_, g) = grid_from(&st, 44100, true, duration, 0.0, 1e9);
            let gb = g.unwrap().beats(duration);
            assert!(st.fin.len() > 100);
            for b in &st.fin {
                let t = beat_time(b.pos, 44100);
                let n = gb.iter().min_by(|x, y| (x.time - t).abs().total_cmp(&(y.time - t).abs())).unwrap();
                assert!((n.time - t).abs() <= 0.0005 + 1e-9, "program {k}: beat at {t} is {} ms off", (n.time - t) * 1e3);
                assert_eq!(n.beat as i32, b.stat.clamp(1, 4), "program {k} at {t}");
                assert_eq!((n.bpm * 100.0).round() as i32, beat_bpm100(b.iv), "program {k} at {t}");
            }
        }
    }

    #[test]
    fn rates_silence_and_range() {
        assert!(tempo_and_grid(&vec![0.0; 44100 * 2], 0, false, 70.0, 180.0).is_none());
        let (b, g) = tempo_and_grid(&vec![0.0; 44100 * 20], 44100, false, 70.0, 180.0).unwrap();
        assert!(b == 0.0 && g.is_none());
        // A 128 BPM track in the 48–95 range comes back halved.
        let x = clicks(128.0, 45.0, 0.2);
        let (got, _) = tempo_and_grid(&x, 44100, false, 48.0, 95.0).unwrap();
        assert!((got - 64.0).abs() < 0.05, "{got}");
    }

    #[test]
    fn find_near_matches_the_original_contract() {
        let mk = |p: &[i32]| p.iter().map(|&pos| BeatInfo { pos, ..Default::default() }).collect::<Vec<_>>();
        let v = mk(&[100, 200, 300, 400]);
        assert_eq!(info::find_near(&v, 210, 50), Some(1));
        assert_eq!(info::find_near(&v, 250, 49), Some(-1));
        assert_eq!(info::find_near(&v, 250, 50), Some(1));
        assert_eq!(info::find_near(&[], 10, 10), None);
    }
}
