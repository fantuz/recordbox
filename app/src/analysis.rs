//! Track analysis: waveform bands, tempo + beat grid, key and loudness.
//! All algorithms are original DSP (see MECHANICS.md for the parameters).

use crate::audio::{decode_file, Decoded};
use crate::model::{BeatGrid, Waveform, WAVE_FPS};
use anyhow::Result;
use rustfft::{num_complex::Complex, FftPlanner};

#[derive(Clone, Debug)]
pub struct AnalysisSettings {
    pub bpm_min: f64,
    pub bpm_max: f64,
    pub do_grid: bool,
    pub high_precision: bool,
    /// Optional installed-model grid mode captured when this job is queued.
    pub model_grid_mode: u8,
    pub do_key: bool,
    /// Loudness the auto gain aims for (LUFS).
    pub target_lufs: f64,
    /// Analysis Mode "Dynamic": the grid follows tempo changes (several anchors).
    pub dynamic: bool,
    /// Vocal detection (rekordbox's, `vocal_rb`), Preferences > Analysis > Vocal.
    pub vocal: bool,
}

pub mod dynamic;
mod precision;
pub mod mp3_timebase;
pub mod rb_hp;

impl Default for AnalysisSettings {
    fn default() -> Self {
        AnalysisSettings { bpm_min: 70.0, bpm_max: 180.0, do_grid: true, high_precision: false,
            model_grid_mode: rb_hp::mode_from_preferences(false, 0), do_key: true, target_lufs: -10.0, dynamic: false, vocal: false }
    }
}

pub const BPM_RANGES: [(f64, f64); 10] = [
    (70.0, 180.0),
    (48.0, 95.0),
    (58.0, 115.0),
    (68.0, 135.0),
    (78.0, 155.0),
    (88.0, 175.0),
    (98.0, 195.0),
    (108.0, 215.0),
    (118.0, 235.0),
    (128.0, 255.0),
];

pub struct AnalysisResult {
    pub duration: f64,
    /// Length in sample frames of the file's full (untrimmed) decode; the original counts
    /// waveform frames and beats over this length.
    pub raw_samples: u64,
    pub sample_rate: u32,
    pub bpm: f64,
    pub grid: Option<BeatGrid>,
    pub key: i32,
    pub wave: Waveform,
    pub gain_db: f32,
    /// Linear sample peak of the measured audio (0 when unavailable).
    pub peak: f32,
    /// Vocal levels 0–4 per 1024 samples at 22,050 Hz (`vocal_rb`), when detected.
    pub vocal: Option<Vec<u8>>,
}

pub fn analyze_file(path: &str, s: &AnalysisSettings) -> Result<AnalysisResult> {
    let dec = decode_file(path)?;
    Ok(analyze_decoded(&dec, s))
}

pub fn analyze_decoded(dec: &Decoded, s: &AnalysisSettings) -> AnalysisResult {
    analyze_passes(dec, s, false)
}

/// Called inside the batch's bounded Rayon pool; nested passes share its workers.
pub(super) fn analyze_file_on_worker(path: &str, s: &AnalysisSettings) -> Result<AnalysisResult> {
    let dec = decode_file(path)?;
    Ok(analyze_passes(&dec, s, true))
}

fn analyze_grid(mono: &[f32], sr: f64, settings: &AnalysisSettings) -> (f64, Option<BeatGrid>) {
    if !settings.do_grid { return (0.0, None); }
    let detect = if settings.dynamic { dynamic::tempo_and_grid_dynamic } else { tempo_and_grid };
    let (bpm, mut grid) = detect(mono, sr, settings.bpm_min, settings.bpm_max);
    if settings.high_precision {
        if let Some(grid) = &mut grid {
            precision::refine(mono, sr, grid, settings.bpm_min, settings.bpm_max);
        }
    }
    (grid.as_ref().map_or(bpm, BeatGrid::first_bpm), grid)
}

fn analyze_passes(dec: &Decoded, s: &AnalysisSettings, parallel: bool) -> AnalysisResult {
    let mono = dec.mono();
    let sr = dec.sr as f64;
    // Key detection uses the measured tempo/grid. The waveform and loudness passes
    // are independent and only borrow the decoded samples.
    let timing = || {
        let (bpm, grid) = analyze_grid(&mono, sr, s);
        let (bpm, grid) = rb_hp::dispatch(dec, s, bpm, grid);
        let key = if s.do_key { crate::key::detect_key_stereo(&dec.samples, dec.sr, bpm, grid.as_ref()) } else { -1 };
        (bpm, grid, key)
    };
    let wave = || waveform(&mono, sr);
    let level = || loudness_and_peak(dec);
    let ((bpm, grid, key), (wave, (lufs, peak))) = if parallel {
        rayon::join(timing, || rayon::join(wave, level))
    } else {
        (timing(), (wave(), level()))
    };
    // No measurement stays unavailable (NaN), never a usable 0 dB or an infinite gain.
    let gain_db = if lufs.is_finite() { (s.target_lufs - lufs).clamp(-24.0, 24.0) as f32 } else { f32::NAN };
    let vocal = if s.vocal { crate::vocal_rb::analyse(dec) } else { None };
    AnalysisResult { duration: dec.duration(), raw_samples: dec.raw_frames as u64, sample_rate: dec.sr, bpm, grid, key, wave, gain_db, peak, vocal }
}

// ------------------------------------------------------------------------------------------
// Filters
// ------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn new(kind: &str, f: f64, q: f64, sr: f64, gain_db: f64) -> Biquad {
        let w = 2.0 * std::f64::consts::PI * f / sr;
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            "lp" => ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            "hp" => ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            "hs" => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cs + sq),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - sq),
                    (a + 1.0) - (a - 1.0) * cs + sq,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - sq,
                )
            }
            _ => (1.0, 0.0, 0.0, 1.0, 0.0, 0.0),
        };
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
    }
    #[inline]
    pub fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

// ------------------------------------------------------------------------------------------
// Waveform: 150 frames per second, peak of full / low / mid / high bands.
// ------------------------------------------------------------------------------------------

pub const LOW_CUT: f64 = 200.0;
pub const HIGH_CUT: f64 = 2500.0;

pub fn waveform(mono: &[f32], sr: f64) -> Waveform {
    let hop = sr / WAVE_FPS;
    let n = (mono.len() as f64 / hop).ceil() as usize;
    let mut lp1 = Biquad::new("lp", LOW_CUT, 0.707, sr, 0.0);
    let mut lp2 = Biquad::new("lp", LOW_CUT, 0.707, sr, 0.0);
    let mut hp1 = Biquad::new("hp", HIGH_CUT, 0.707, sr, 0.0);
    let mut hp2 = Biquad::new("hp", HIGH_CUT, 0.707, sr, 0.0);
    let mut mh = Biquad::new("hp", LOW_CUT, 0.707, sr, 0.0);
    let mut ml = Biquad::new("lp", HIGH_CUT, 0.707, sr, 0.0);
    let mut peak = vec![[0f32; 4]; n];
    let mut sq = vec![[0f64; 4]; n];
    let mut cnt = vec![0u32; n];
    for (i, &x) in mono.iter().enumerate() {
        let x = x as f64;
        let lo = lp2.run(lp1.run(x));
        let hi = hp2.run(hp1.run(x));
        let mi = ml.run(mh.run(x));
        let fi = ((i as f64) / hop) as usize;
        if fi >= n {
            break;
        }
        for (k, v) in [x, lo, mi, hi].iter().enumerate() {
            peak[fi][k] = peak[fi][k].max(v.abs() as f32);
            sq[fi][k] += v * v;
        }
        cnt[fi] += 1;
    }
    let frames = (0..n)
        .map(|i| {
            let mut o = [0u8; 8];
            for k in 0..4 {
                o[k] = (peak[i][k].min(1.0) * 255.0).round() as u8;
                let rms = (sq[i][k] / cnt[i].max(1) as f64).sqrt();
                o[4 + k] = (rms * 510.0).round().min(255.0) as u8;
            }
            o
        })
        .collect();
    Waveform { frames }
}

// ------------------------------------------------------------------------------------------
// Tempo and beat grid
// ------------------------------------------------------------------------------------------

const ONSET_FPS: f64 = 200.0;

struct Onsets {
    all: Vec<f32>,
    low: Vec<f32>,
    fps: f64,
    /// Time of frame 0 (seconds)
    t0: f64,
}

fn onsets(mono: &[f32], sr: f64) -> Onsets {
    let nfft = 1024usize;
    let hop = (sr / ONSET_FPS).round().max(1.0) as usize;
    let fps = sr / hop as f64;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let win: Vec<f32> = (0..nfft).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / nfft as f32).cos()).collect();
    let nb = nfft / 2;
    let max_bin = ((8000.0 / sr) * nfft as f64) as usize;
    let low_bin = ((160.0 / sr) * nfft as f64).ceil() as usize;
    let mut prev = vec![0f32; nb];
    let mut all = vec![];
    let mut low = vec![];
    let mut buf = vec![Complex::new(0f32, 0f32); nfft];
    let mut start = 0usize;
    while start + nfft <= mono.len() {
        for i in 0..nfft {
            buf[i] = Complex::new(mono[start + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        let mut fa = 0f32;
        let mut fl = 0f32;
        for b in 1..max_bin.min(nb) {
            let m = (1.0 + 100.0 * buf[b].norm()).ln();
            let d = m - prev[b];
            if d > 0.0 {
                fa += d;
                if b <= low_bin {
                    fl += d;
                }
            }
            prev[b] = m;
        }
        all.push(fa);
        low.push(fl);
        start += hop;
    }
    Onsets { all, low, fps, t0: nfft as f64 / 2.0 / sr }
}

fn normalize(v: &mut [f32]) {
    if v.is_empty() {
        return;
    }
    // Subtract a moving average so that only local peaks remain.
    let w = 16usize;
    let mut out = vec![0f32; v.len()];
    let mut acc = 0f64;
    let mut q = std::collections::VecDeque::new();
    for i in 0..v.len() + w {
        if i < v.len() {
            acc += v[i] as f64;
            q.push_back(v[i]);
        }
        if q.len() > 2 * w + 1 {
            acc -= q.pop_front().unwrap() as f64;
        }
        if i >= w {
            let c = i - w;
            let mean = acc / q.len() as f64;
            out[c] = (v[c] as f64 - mean).max(0.0) as f32;
        }
    }
    let mx = out.iter().cloned().fold(0f32, f32::max).max(1e-9);
    for (a, b) in v.iter_mut().zip(out) {
        *a = b / mx;
    }
}

fn interp(v: &[f32], x: f64) -> f32 {
    if x < 0.0 {
        return 0.0;
    }
    let i = x as usize;
    if i + 1 >= v.len() {
        return 0.0;
    }
    let f = (x - i as f64) as f32;
    v[i] * (1.0 - f) + v[i + 1] * f
}

/// Score of a beat comb with period `p` frames: best phase and its score.
fn comb_score(env: &[f32], p: f64) -> (f64, f64) {
    let steps = (p * 2.0).ceil() as usize;
    let mut best = (0.0, -1.0);
    for s in 0..steps {
        let ph = s as f64 * 0.5;
        let mut x = ph;
        let mut sc = 0f64;
        let mut n = 0;
        while x < env.len() as f64 {
            sc += interp(env, x) as f64;
            n += 1;
            x += p;
        }
        let sc = if n > 0 { sc / n as f64 } else { 0.0 };
        if sc > best.1 {
            best = (ph, sc);
        }
    }
    best
}

pub fn tempo_and_grid(mono: &[f32], sr: f64, bpm_min: f64, bpm_max: f64) -> (f64, Option<BeatGrid>) {
    if mono.len() < (sr * 4.0) as usize {
        return (0.0, None);
    }
    let mut o = onsets(mono, sr);
    normalize(&mut o.all);
    normalize(&mut o.low);
    let env: Vec<f32> = o.all.iter().zip(&o.low).map(|(a, l)| a + 0.6 * l).collect();
    if !env.iter().any(|&strength| strength > 0.0) { return (0.0, None); }
    let fps = o.fps;

    // 1) Autocorrelation over the allowed range, with harmonics.
    let max_lag = (60.0 * fps / bpm_min * 4.0) as usize + 2;
    let n = env.len();
    let mean = env.iter().map(|v| *v as f64).sum::<f64>() / n as f64;
    let e: Vec<f64> = env.iter().map(|v| *v as f64 - mean).collect();
    let mut ac = vec![0f64; max_lag + 1];
    for lag in 1..=max_lag.min(n - 1) {
        let mut s = 0.0;
        for i in 0..n - lag {
            s += e[i] * e[i + lag];
        }
        ac[lag] = s / (n - lag) as f64;
    }
    let ac_at = |l: f64| -> f64 {
        let i = l as usize;
        if i + 1 >= ac.len() {
            return 0.0;
        }
        let f = l - i as f64;
        ac[i] * (1.0 - f) + ac[i + 1] * f
    };
    let mut cands: Vec<(f64, f64)> = vec![];
    let mut bpm = bpm_min;
    while bpm <= bpm_max {
        let lag = 60.0 * fps / bpm;
        let sc = ac_at(lag) + 0.5 * ac_at(lag * 2.0) + 0.33 * ac_at(lag * 4.0) + 0.25 * ac_at(lag * 0.5);
        // Mild preference for the 110..140 region (estimated), common for dance music.
        let w = (-0.5 * ((bpm / 125.0).log2() / 0.9).powi(2)).exp();
        cands.push((bpm, sc * (0.7 + 0.3 * w)));
        bpm += 0.25;
    }
    cands.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let coarse = cands.first().map(|c| c.0).unwrap_or(120.0);

    // 2) Refine with a phase-aligned comb on a 0.01 BPM grid.
    let mut best = (coarse, 0.0, -1.0);
    let mut b = coarse - 0.5;
    while b <= coarse + 0.5 {
        let p = 60.0 * fps / b;
        let (ph, sc) = comb_score(&env, p);
        if sc > best.2 {
            best = (b, ph, sc);
        }
        b += 0.01;
    }
    let mut bpm = (best.0 * 100.0).round() / 100.0;
    // Prefer a whole BPM when it explains the beats nearly as well.
    let whole = bpm.round();
    if (whole - bpm).abs() > 1e-6 && (whole - bpm).abs() < 0.35 {
        let (_, sc) = comb_score(&env, 60.0 * fps / whole);
        if sc >= best.2 * 0.97 {
            bpm = whole;
        }
    }
    let period = 60.0 * fps / bpm;
    let (mut phase, _) = comb_score(&env, period);
    // Beats sit on the kick: compare low-band attack on this phase and the half-beat offset.
    let low_at = |ph: f64| -> f64 {
        let mut x = ph;
        let mut s = 0.0;
        while x < o.low.len() as f64 {
            s += interp(&o.low, x) as f64;
            x += period;
        }
        s
    };
    if low_at(phase + period / 2.0) > low_at(phase) * 1.15 {
        phase += period / 2.0;
    }
    // The first kick of a track is almost always on a beat: if it sits on the half-beat
    // of the chosen phase, move the grid by half a beat.
    let lpeak = o.low.iter().cloned().fold(0f32, f32::max).max(1e-6);
    if let Some(f0) = o.low.iter().position(|v| *v > 0.35 * lpeak) {
        // refine to the local maximum just after crossing
        let mut fm = f0;
        for k in f0..(f0 + 6).min(o.low.len()) {
            if o.low[k] > o.low[fm] {
                fm = k;
            }
        }
        let rel = ((fm as f64 - phase) / period).rem_euclid(1.0);
        if (rel - 0.5).abs() < 0.2 {
            phase += period / 2.0;
        }
    }

    // 3) Sub-frame refinement against low-band transients in the time domain.
    let period_s = 60.0 / bpm;
    let mut first = o.t0 + phase / fps;
    first = refine_phase(mono, sr, first, period_s) - phase_lead() / 1000.0;
    // Bring the first beat to the earliest position >= 0.
    first -= (first / period_s).floor() * period_s;

    // 4) Downbeat: which of the 4 beat positions carries the most low-frequency attack.
    let mut sums = [0f64; 4];
    let mut t = first;
    let mut k = 0usize;
    while t < mono.len() as f64 / sr {
        let fi = (t - o.t0) * fps;
        let mut v = 0f64;
        for d in -4..=4 {
            let x = fi + d as f64;
            v = v.max(interp(&o.low, x) as f64 + 0.5 * interp(&o.all, x) as f64);
        }
        // Bar starts often coincide with changes (crashes, new parts): weight 16-beat phrase starts.
        sums[k % 4] += v;
        t += period_s;
        k += 1;
    }
    // Harmony tends to change at the start of a bar: add the chroma change at each beat.
    let times: Vec<f64> = (0..k).map(|i| first + i as f64 * period_s).collect();
    let nov = chroma_novelty(mono, sr, &times);
    let mut hsum = [0f64; 4];
    for (i, v) in nov.iter().enumerate() {
        hsum[i % 4] += v;
    }
    let z = |a: [f64; 4]| -> [f64; 4] {
        let m = a.iter().sum::<f64>() / 4.0;
        let sd = (a.iter().map(|x| (x - m).powi(2)).sum::<f64>() / 4.0).sqrt().max(1e-9);
        [(a[0] - m) / sd, (a[1] - m) / sd, (a[2] - m) / sd, (a[3] - m) / sd]
    };
    let (zl, zh) = (z(sums), z(hsum));
    // Section changes (drops, breakdowns, new parts) land on a bar's first beat.
    let zs = z(structure_votes(mono, sr, &times));
    let ws = structure_weight();
    // DJ edits usually start on a bar: favour the beat where the music starts.
    let mut score: Vec<f64> = (0..4).map(|j| zl[j] * 0.6 + zh[j] + zs[j] * ws).collect();
    let peak = o.low.iter().cloned().fold(0f32, f32::max).max(1e-6);
    if let Some(k0) = times.iter().position(|t| {
        let fi = (t - o.t0) * fps;
        (-4..=4).map(|d| interp(&o.low, fi + d as f64)).fold(0f32, f32::max) > 0.25 * peak
    }) {
        score[k0 % 4] += downbeat_start_bonus();
    }
    let down = (0..4).max_by(|a, b| score[*a].partial_cmp(&score[*b]).unwrap()).unwrap_or(0);
    // Beat number of the first beat so that beat index `down` is a "1".
    let first_beat_no = ((4 - down) % 4) as u8 + 1;
    (bpm, Some(BeatGrid::constant(first, bpm, first_beat_no)))
}

fn structure_weight() -> f64 {
    std::env::var("PREPDECK_STRUCT_W").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0)
}

/// Votes per beat position (mod 4) from the strongest section changes: per beat, log energy in
/// three bands; novelty = distance between the means of the 8 beats after and the 8 before;
/// the top peaks (at least 4 beats apart) vote for their position, weighted by strength.
fn structure_votes(mono: &[f32], sr: f64, times: &[f64]) -> [f64; 4] {
    let mut votes = [0f64; 4];
    if times.len() < 40 {
        return votes;
    }
    let mut lp = Biquad::new("lp", 200.0, 0.707, sr, 0.0);
    let mut hp = Biquad::new("hp", 2500.0, 0.707, sr, 0.0);
    let mut feats: Vec<[f64; 3]> = Vec::with_capacity(times.len());
    let mut idx = 0usize;
    let n = mono.len();
    for (k, t) in times.iter().enumerate() {
        let end = times.get(k + 1).map(|x| (x * sr) as usize).unwrap_or(n).min(n);
        let start = ((*t * sr) as usize).min(n);
        let mut e = [0f64; 3];
        // keep the filters running continuously
        while idx < start {
            lp.run(mono[idx] as f64);
            hp.run(mono[idx] as f64);
            idx += 1;
        }
        let mut cnt = 0f64;
        while idx < end {
            let x = mono[idx] as f64;
            let l = lp.run(x);
            let h = hp.run(x);
            e[0] += l * l;
            e[1] += (x - l - h).powi(2);
            e[2] += h * h;
            cnt += 1.0;
            idx += 1;
        }
        let c = cnt.max(1.0);
        feats.push([(e[0] / c + 1e-9).ln(), (e[1] / c + 1e-9).ln(), (e[2] / c + 1e-9).ln()]);
    }
    let w: usize = std::env::var("PREPDECK_STRUCT_WIN").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let max_peaks: usize = std::env::var("PREPDECK_STRUCT_PEAKS").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let mut nov = vec![0f64; feats.len()];
    for i in w..feats.len().saturating_sub(w) {
        let mut d = 0f64;
        for b in 0..3 {
            let after: f64 = feats[i..i + w].iter().map(|f| f[b]).sum::<f64>() / w as f64;
            let before: f64 = feats[i - w..i].iter().map(|f| f[b]).sum::<f64>() / w as f64;
            d += (after - before).powi(2);
        }
        nov[i] = d.sqrt();
    }
    // Strongest peaks, at least 4 beats apart.
    let mut order: Vec<usize> = (0..nov.len()).collect();
    order.sort_by(|a, b| nov[*b].partial_cmp(&nov[*a]).unwrap());
    let mut picked: Vec<usize> = vec![];
    for i in order {
        if nov[i] <= 0.0 || picked.len() >= max_peaks {
            break;
        }
        if picked.iter().all(|p| p.abs_diff(i) >= 4) {
            picked.push(i);
        }
    }
    for p in picked {
        votes[p % 4] += nov[p];
    }
    votes
}

/// Find the offset that best aligns predicted beats with the steepest rise of the
/// low-passed amplitude envelope (median over the track).
fn downbeat_start_bonus() -> f64 {
    std::env::var("PREPDECK_DB_BONUS").ok().and_then(|v| v.parse().ok()).unwrap_or(3.0)
}

/// Fraction of the steepest rise that marks the start of a kick's attack.
const ATTACK_FRACTION: f32 = 0.85;
fn attack_fraction() -> f32 {
    std::env::var("PREPDECK_ATTACK").ok().and_then(|v| v.parse().ok()).unwrap_or(ATTACK_FRACTION)
}

/// Low-pass of the envelope the phase is refined on; 0 = the full band. rekordbox marks the
/// broadband onset of the kick (measured: full band 1.3 ms median against its grids, 150 Hz 5.8 ms
/// and about 24 ms late).
fn refine_hz() -> f64 {
    std::env::var("PREPDECK_REFINE_HZ").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0)
}
fn phase_lead() -> f64 {
    std::env::var("PREPDECK_PHASE_LEAD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0)
}
fn refine_back() -> isize {
    std::env::var("PREPDECK_REFINE_BACK").ok().and_then(|v| v.parse().ok()).unwrap_or(12)
}

fn refine_phase(mono: &[f32], sr: f64, first: f64, period: f64) -> f64 {
    let hz = refine_hz();
    let lookback = refine_back().clamp(0, 80);
    let fraction = attack_fraction();
    let mut lp1 = Biquad::new("lp", hz.max(1.0), 0.707, sr, 0.0);
    let mut lp2 = Biquad::new("lp", hz.max(1.0), 0.707, sr, 0.0);
    let ms = (sr / 1000.0).max(1.0) as usize;
    let mut env = Vec::with_capacity(mono.len() / ms + 1);
    let mut acc = 0f32;
    for (i, &x) in mono.iter().enumerate() {
        let y = if hz > 0.0 { lp2.run(lp1.run(x as f64)).abs() as f32 } else { x.abs() };
        acc = acc.max(y);
        if (i + 1) % ms == 0 {
            env.push(acc);
            acc = 0.0;
        }
    }
    let mut offsets = vec![];
    let mut t = first;
    let dur = mono.len() as f64 / sr;
    while t < dur - 0.1 {
        let c = (t * sr / ms as f64) as isize;
        let mut best = (0isize, 0f32);
        for d in -40isize..=40 {
            let i = c + d;
            if i < 2 || i as usize + 1 >= env.len() {
                continue;
            }
            let rise = env[i as usize + 1] - env[i as usize - 2];
            if rise > best.1 {
                best = (d, rise);
            }
        }
        if best.1 > 0.02 {
            // Use where the attack starts, not where it is steepest.
            let rise_at = |d: isize| -> f32 {
                let i = c + d;
                if i < 2 || i as usize + 1 >= env.len() {
                    return 0.0;
                }
                env[i as usize + 1] - env[i as usize - 2]
            };
            let mut start = best.0;
            for d in (best.0 - lookback).max(-40)..best.0 {
                if rise_at(d) >= fraction * best.1 {
                    start = d;
                    break;
                }
            }
            offsets.push(start);
        }
        t += period;
    }
    if offsets.len() < 8 {
        return first;
    }
    offsets.sort();
    let med = offsets[offsets.len() / 2];
    first + med as f64 * ms as f64 / sr
}

/// Cosine distance between the pitch-class profile of each beat and the previous beat.
fn chroma_novelty(mono: &[f32], sr: f64, times: &[f64]) -> Vec<f64> {
    let nfft = 4096usize;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let win: Vec<f32> = (0..nfft).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / nfft as f32).cos()).collect();
    let mut buf = vec![Complex::new(0f32, 0f32); nfft];
    let mut prev: Option<[f64; 12]> = None;
    let mut out = Vec::with_capacity(times.len());
    for &t in times {
        let start = ((t + 0.03) * sr) as usize;
        if start + nfft > mono.len() {
            out.push(0.0);
            continue;
        }
        for i in 0..nfft {
            buf[i] = Complex::new(mono[start + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        let mut c = [0f64; 12];
        for b in 1..nfft / 2 {
            let f = b as f64 * sr / nfft as f64;
            if !(90.0..=2000.0).contains(&f) {
                continue;
            }
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            let pc = (midi.round() as i64).rem_euclid(12) as usize;
            c[pc] += buf[b].norm() as f64;
        }
        let n = c.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-9);
        for x in c.iter_mut() {
            *x /= n;
        }
        let v = match prev {
            Some(p) => 1.0 - (0..12).map(|i| p[i] * c[i]).sum::<f64>(),
            None => 0.0,
        };
        out.push(v);
        prev = Some(c);
    }
    out
}

/// Recompute a grid's BPM by tapping intervals etc. is UI work; this helper fits a constant
/// tempo for "Analyze from current position" style edits.
pub fn shift_grid(g: &BeatGrid, seconds: f64) -> BeatGrid {
    BeatGrid { anchors: g.anchors.iter().map(|a| crate::model::GridAnchor { time: a.time + seconds, ..*a }).collect() }
}

// ------------------------------------------------------------------------------------------
// Key detection: chroma profile correlated with major/minor key templates.
// ------------------------------------------------------------------------------------------

const MAJOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

pub fn detect_key(mono: &[f32], sr: f64) -> i32 {
    // Downsample by an integer factor to ~11 kHz with a simple low-pass.
    let factor = ((sr / 11025.0).round() as usize).max(1);
    let mut lp1 = Biquad::new("lp", 4000.0, 0.707, sr, 0.0);
    let mut lp2 = Biquad::new("lp", 4000.0, 0.707, sr, 0.0);
    let mut ds = Vec::with_capacity(mono.len() / factor + 1);
    for (i, &x) in mono.iter().enumerate() {
        let y = lp2.run(lp1.run(x as f64));
        if i % factor == 0 {
            ds.push(y as f32);
        }
    }
    let dsr = sr / factor as f64;
    let nfft = 8192usize;
    let hop = 4096usize;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let win: Vec<f32> = (0..nfft).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / nfft as f32).cos()).collect();
    // Bin -> pitch class weights (55 Hz .. 1760 Hz).
    let mut map: Vec<(usize, usize, f32)> = vec![];
    for b in 1..nfft / 2 {
        let f = b as f64 * dsr / nfft as f64;
        if !(55.0..=1760.0).contains(&f) {
            continue;
        }
        let midi = 69.0 + 12.0 * (f / 440.0).log2();
        let nearest = midi.round();
        let dev = (midi - nearest).abs();
        if dev > 0.35 {
            continue;
        }
        let pc = (nearest as i64).rem_euclid(12) as usize;
        let w = (1.0 - dev / 0.35) as f32;
        // Emphasise bass/mid register where harmony lives.
        let reg = if f < 220.0 { 1.4 } else if f < 900.0 { 1.0 } else { 0.6 };
        map.push((b, pc, w * reg));
    }
    let mut chroma = [0f64; 12];
    let mut buf = vec![Complex::new(0f32, 0f32); nfft];
    let mut start = 0;
    while start + nfft <= ds.len() {
        for i in 0..nfft {
            buf[i] = Complex::new(ds[start + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        let mut frame = [0f64; 12];
        let mut tot = 0.0;
        for &(b, pc, w) in &map {
            let m = buf[b].norm() as f64;
            frame[pc] += (m * w as f64).sqrt();
            tot += m;
        }
        if tot > 1e-3 {
            let s: f64 = frame.iter().sum();
            for k in 0..12 {
                chroma[k] += frame[k] / s.max(1e-9);
            }
        }
        start += hop;
    }
    if chroma.iter().sum::<f64>() <= 0.0 {
        return -1;
    }
    let corr = |prof: &[f64; 12], root: usize| -> f64 {
        let xm = chroma.iter().sum::<f64>() / 12.0;
        let ym = prof.iter().sum::<f64>() / 12.0;
        let (mut num, mut dx, mut dy) = (0.0, 0.0, 0.0);
        for i in 0..12 {
            let x = chroma[(i + root) % 12] - xm;
            let y = prof[i] - ym;
            num += x * y;
            dx += x * x;
            dy += y * y;
        }
        num / (dx * dy).sqrt().max(1e-12)
    };
    let mut best = (-2.0, 0usize, false);
    for root in 0..12 {
        let a = corr(&MAJOR, root);
        if a > best.0 {
            best = (a, root, false);
        }
        let b = corr(&MINOR, root);
        if b > best.0 {
            best = (b, root, true);
        }
    }
    crate::model::key_from_pitch(best.1 as i32, best.2)
}

// ------------------------------------------------------------------------------------------
// Loudness (ITU-R BS.1770 style: K-weighting, 400 ms blocks, absolute + relative gating)
// ------------------------------------------------------------------------------------------

/// Integrated loudness (LUFS) as rekordbox's library analysis measures it (rekordbox-rea
/// reports/loudness-and-auto-gain.md and loudness-analysis-feed.md): the file is read in
/// two-second blocks and only every F-th block is measured, F = min(16, floor(D / 30) + 1)
/// for D whole seconds; samples are truncated to 16 bits; the selected blocks run through one
/// continuous EBU R128 measurement (K-weighting, 400 ms windows every 100 ms, absolute gate
/// -70 LUFS, relative gate -10 LU, both inclusive). Negative infinity when there is too
/// little audio.
pub fn loudness(dec: &Decoded) -> f64 {
    loudness_and_peak(dec).0
}

/// Loudness as [`loudness`] plus the linear sample peak of the same measured (16-bit
/// truncated, block-sampled) samples, as rekordbox measures both together.
pub fn loudness_and_peak(dec: &Decoded) -> (f64, f32) {
    let rate = dec.sr as usize;
    let frames = dec.frames();
    if rate == 0 || frames == 0 {
        return (f64::NEG_INFINITY, 0.0);
    }
    let e = frames - 1;
    let d = e / rate;
    let f = (d / 30 + 1).min(16);
    let block = 2 * rate;
    let q = |x: f32| ((x * 32768.0).clamp(-32768.0, 32767.0).trunc()) / 32768.0;
    let mut fed: Vec<f32> = Vec::with_capacity(2 * frames / f + 2 * block);
    let mut start = 0;
    let mut ordinal = 0usize;
    while start < frames {
        let n = block.min(frames - start);
        if ordinal % f == 0 {
            fed.extend(dec.samples[start * 2..(start + n) * 2].iter().map(|x| q(*x)));
        }
        start += n;
        ordinal += 1;
    }
    let peak = fed.iter().fold(0f32, |m, v| m.max(v.abs()));
    (r128_integrated(&fed, dec.sr as f64), peak)
}

/// EBU R128 integrated loudness of interleaved stereo (libebur128's non-histogram mode).
pub fn r128_integrated(samples: &[f32], sr: f64) -> f64 {
    let mut ch: Vec<(Biquad, Biquad)> = (0..2)
        .map(|_| (Biquad::new("hs", 1681.97, 0.7072, sr, 4.0), Biquad::new("hp", 38.13, 0.5003, sr, 0.0)))
        .collect();
    let step = ((sr as usize) + 5) / 10;
    let window = 4 * step;
    let frames = samples.len() / 2;
    let mut pre = vec![0f64; frames + 1];
    for i in 0..frames {
        let mut s = 0.0;
        for c in 0..2 {
            let x = samples[i * 2 + c] as f64;
            let f = &mut ch[c];
            let y0 = f.0.run(x);
            let y = f.1.run(y0);
            s += y * y;
        }
        pre[i + 1] = pre[i] + s;
    }
    let mut blocks = vec![];
    let mut s = 0;
    while s + window <= frames {
        blocks.push((pre[s + window] - pre[s]) / window as f64);
        s += step;
    }
    let lk = |z: f64| -0.691 + 10.0 * z.max(1e-300).log10();
    let abs: Vec<f64> = blocks.into_iter().filter(|z| lk(*z) >= -70.0).collect();
    if abs.is_empty() {
        return f64::NEG_INFINITY;
    }
    let mean = abs.iter().sum::<f64>() / abs.len() as f64;
    let gated: Vec<f64> = abs.into_iter().filter(|z| *z >= 0.1 * mean).collect();
    if gated.is_empty() {
        return lk(mean);
    }
    lk(gated.iter().sum::<f64>() / gated.len() as f64)
}

#[cfg(test)]
mod loudness_tests {
    use super::*;

    fn tone(secs: f64, sr: u32, amp: f32) -> Decoded {
        let n = (secs * sr as f64) as usize;
        let samples: Vec<f32> = (0..n).flat_map(|i| {
            let v = amp * (2.0 * std::f32::consts::PI * 997.0 * i as f32 / sr as f32).sin();
            [v, v]
        }).collect();
        Decoded { sr, samples, raw_frames: n, damaged: 0, truncated: false }
    }

    #[test]
    fn steady_tone_and_block_sampling() {
        // A steady tone measures the same whether every block or every 16th is fed.
        let short = loudness(&tone(20.0, 44100, 0.1));
        let long = loudness(&tone(500.0, 44100, 0.1));
        assert!((short - long).abs() < 0.05, "{short} vs {long}");
        // 997 Hz at amplitude 0.1 in both channels: mean square 0.005 per channel, summed
        // -20 dB, minus 0.691, plus K-weighting's small gain near 1 kHz: about -20.3 LUFS.
        assert!(short > -20.8 && short < -19.8, "{short}");
        // The sample peak comes with it (16-bit truncated, so just under 0.1).
        let (_, pk) = loudness_and_peak(&tone(20.0, 44100, 0.1));
        assert!(pk > 0.0995 && pk <= 0.1, "{pk}");
        // Silence and too-short audio have no loudness.
        assert!(loudness(&tone(10.0, 44100, 0.0)).is_infinite());
        assert!(loudness(&tone(0.2, 48000, 0.1)).is_infinite());
        // rekordbox's gain for it: -10 - L.
        assert!(((-10.0 - short) - (-10.0 - long)).abs() < 0.05);
    }

    #[test]
    fn only_selected_blocks_count() {
        // 100 s: F = 4, so blocks 0, 4, 8, ... (2 s each) are measured. A loud section placed
        // only in unselected blocks does not change the result.
        let mut d = tone(100.0, 44100, 0.05);
        let sr = 44100usize;
        for blk in [1usize, 2, 3] {
            for i in blk * 2 * sr..(blk + 1) * 2 * sr {
                d.samples[2 * i] *= 10.0;
                d.samples[2 * i + 1] *= 10.0;
            }
        }
        let base = loudness(&tone(100.0, 44100, 0.05));
        assert!((loudness(&d) - base).abs() < 0.01);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_track(bpm: f64, secs: f64, sr: f64, offset: f64) -> Vec<f32> {
        let n = (secs * sr) as usize;
        let mut v = vec![0f32; n];
        let p = 60.0 / bpm;
        let mut t = offset;
        let mut k = 0;
        while t < secs {
            let start = (t * sr) as usize;
            let amp = if k % 4 == 0 { 1.0 } else { 0.6 };
            for i in 0..(0.08 * sr) as usize {
                if start + i < n {
                    let env = (-(i as f64) / (0.015 * sr)).exp();
                    v[start + i] += (amp * env * (2.0 * std::f64::consts::PI * 60.0 * i as f64 / sr).sin()) as f32;
                }
            }
            t += p;
            k += 1;
        }
        // hats on offbeats
        let mut t = offset + p / 2.0;
        while t < secs {
            let start = (t * sr) as usize;
            for i in 0..(0.01 * sr) as usize {
                if start + i < n {
                    let noise = ((i * 7919 % 97) as f32 / 97.0 - 0.5) * 0.2;
                    v[start + i] += noise;
                }
            }
            t += p;
        }
        v
    }

    #[test]
    fn detects_tempo_and_phase() {
        for &(bpm, off) in &[(128.0, 0.31), (122.0, 0.05), (140.0, 0.2), (174.0, 0.12)] {
            let sr = 44100.0;
            let x = click_track(bpm, 40.0, sr, off);
            let (got, grid) = tempo_and_grid(&x, sr, 70.0, 180.0);
            assert!((got - bpm).abs() < 0.02, "bpm {bpm} got {got}");
            let g = grid.unwrap();
            let first = g.anchors[0].time;
            let p = 60.0 / bpm;
            let err = ((first - off) / p - ((first - off) / p).round()).abs() * p;
            assert!(err < 0.012, "phase err {err} for {bpm}");
            // the loud beat is the downbeat
            let beats = g.beats(40.0);
            let b1 = beats.iter().find(|b| (b.time - off).abs() < 0.02).unwrap();
            assert_eq!(b1.beat, 1, "downbeat for {bpm}");
        }
    }

    #[test]
    fn high_precision_tracks_fractional_tempo_and_attack_times() {
        for (sr, bpm, offset) in [(44_100.0, 127.137, 0.2137), (48_000.0, 121.043, 0.0573)] {
            let seconds = 64.0;
            let music = click_track(bpm, seconds, sr, offset);
            let mut settings = AnalysisSettings::default();
            let (_, standard) = analyze_grid(&music, sr, &settings);
            settings.high_precision = true;
            let (found, precise) = analyze_grid(&music, sr, &settings);
            let precise = precise.unwrap();
            let (base, beats) = (standard.unwrap().beats(seconds), precise.beats(seconds));
            let error = |grid: &[crate::model::Beat]| {
                let mut errors: Vec<_> = (0..).map(|i| offset + f64::from(i) * 60.0 / bpm)
                    .take_while(|&t| t < seconds - 1.0)
                    .map(|t| (crate::model::nearest_beat(grid, t).unwrap().time - t).abs() * 1000.0).collect();
                errors.sort_unstable_by(f64::total_cmp);
                errors[errors.len() * 95 / 100]
            };
            let (before, after) = (error(&base), error(&beats));
            eprintln!("precision grid {sr} Hz, {bpm} BPM: 95th percentile {before:.3} -> {after:.3} ms, measured tempo {found:.4}");
            assert_eq!(precise.anchors.len(), 1);
            assert!((found - bpm).abs() < 0.004 && after < 2.0 && after < before, "{bpm}: {before} -> {after}, BPM {found}");
            assert_eq!(crate::model::nearest_beat(&beats, offset).unwrap().beat, 1);
        }
    }

    #[test]
    fn detects_key_of_chord() {
        let sr = 22050.0;
        let n = (sr * 10.0) as usize;
        // A minor triad: A3 C4 E4 with octaves
        let freqs = [220.0, 261.63, 329.63, 110.0, 440.0];
        let x: Vec<f32> = (0..n)
            .map(|i| freqs.iter().map(|f| (2.0 * std::f64::consts::PI * f * i as f64 / sr).sin() as f32 * 0.15).sum())
            .collect();
        let k = detect_key(&x, sr);
        assert_eq!(crate::model::key_name(k, false), "Am");
    }
}

#[cfg(test)]
mod real_files {
    #[test]
    #[ignore]
    fn analyze_env_files() {
        let list = std::env::var("PREPDECK_AUDIO").unwrap_or_default();
        for p in list.split('|').filter(|s| !s.is_empty()) {
            let t0 = std::time::Instant::now();
            match super::analyze_file(p, &Default::default()) {
                Ok(r) => {
                    let g = r.grid.unwrap();
                    eprintln!("{:.1}s bpm {:.2} first {:.3} beat {} key {} gain {:.1} dur {:.1} :: {}", t0.elapsed().as_secs_f64(), r.bpm, g.anchors[0].time, g.anchors[0].beat, crate::model::key_name(r.key, false) + "/" + &crate::model::key_alnum(r.key), r.gain_db, r.duration, p.rsplit('/').next().unwrap());
                }
                Err(e) => eprintln!("ERR {e} :: {p}"),
            }
        }
    }
}

#[cfg(test)]
mod decode_probe {
    /// Writes the first 30 s of each decoded file (left channel, f32 LE) to PREPDECK_DUMP_PCM so
    /// the start offset can be compared with another decoder. PREPDECK_AUDIO = paths joined by '|'.
    #[test]
    #[ignore]
    fn dump_decoded_head() {
        let Ok(dir) = std::env::var("PREPDECK_DUMP_PCM") else { return };
        let list = std::env::var("PREPDECK_AUDIO").unwrap_or_default();
        for (i, p) in list.split('|').filter(|s| !s.is_empty()).enumerate() {
            let Ok(d) = crate::audio::decode_file(p) else { continue };
            let n = (d.sr as usize * 30).min(d.frames());
            let bytes: Vec<u8> = d.samples.chunks_exact(2).take(n).flat_map(|c| c[0].to_le_bytes()).collect();
            std::fs::write(format!("{dir}/{i}.f32"), bytes).unwrap();
            eprintln!("{i}\t{}\t{}\t{:?}\t{p}", d.frames(), d.raw_frames, crate::audio::mp3_timing_class(p));
        }
    }
}

#[cfg(test)]
mod ground_truth {
    /// Compare analysis with reference grids: PREPDECK_GT = tsv of "path\tbeat,bpm100,ms;..."
    #[test]
    #[ignore]
    fn compare_reference_grids() {
        let Ok(f) = std::env::var("PREPDECK_GT") else { return };
        let text = std::fs::read_to_string(f).unwrap();
        let rows: Vec<(String, Vec<(u8, f64, f64)>)> = text
            .lines()
            .filter_map(|l| {
                let (p, b) = l.split_once('\t')?;
                let beats = b.split(';').filter_map(|x| {
                    let v: Vec<f64> = x.split(',').filter_map(|n| n.parse().ok()).collect();
                    (v.len() == 3).then(|| (v[0] as u8, v[1] / 100.0, v[2] / 1000.0))
                }).collect();
                Some((p.to_string(), beats))
            })
            .collect();
        let results = std::sync::Mutex::new(vec![]);
        let chunks: Vec<_> = rows.chunks((rows.len() / 8).max(1)).collect();
        std::thread::scope(|s| {
            for ch in chunks {
                let results = &results;
                s.spawn(move || {
                    for (p, beats) in ch {
                        let Ok(r) = super::analyze_file(p, &Default::default()) else { continue };
                        let Some(g) = r.grid else { continue };
                        let (rb, rbpm, rt) = beats[0];
                        let period = 60.0 / rbpm;
                        let mine = g.anchors[0].time;
                        let mut d = (mine - rt) / period;
                        d -= d.round();
                        let phase_ms = d * period * 1000.0;
                        // downbeat: reference time of a "1"
                        let ref_one = rt + ((5 - rb as i32) % 4) as f64 * period;
                        let mine_beats = g.beats(30.0);
                        let near = crate::model::nearest_beat(&mine_beats, ref_one).map(|b| b.beat).unwrap_or(0);
                        results.lock().unwrap().push((p.rsplit('/').next().unwrap().to_string(), rbpm, r.bpm, phase_ms, near == 1));
                    }
                });
            }
        });
        let res = results.into_inner().unwrap();
        let n = res.len() as f64;
        let bpm_ok = res.iter().filter(|r| (r.1 - r.2).abs() < 0.02).count();
        let bpm_oct = res.iter().filter(|r| (r.1 - r.2).abs() >= 0.02).collect::<Vec<_>>();
        let ph: Vec<f64> = res.iter().filter(|r| (r.1 - r.2).abs() < 0.02).map(|r| r.3).collect();
        let mut abs: Vec<f64> = ph.iter().map(|x| x.abs()).collect();
        abs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = ph.iter().sum::<f64>() / ph.len().max(1) as f64;
        let down = res.iter().filter(|r| r.4).count();
        eprintln!("tracks {n}  bpm exact {bpm_ok}  downbeat {down}");
        let mut sg = ph.clone();
        sg.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!("phase median signed {:.1} ms", sg.get(sg.len() / 2).unwrap_or(&0.0));
        eprintln!("phase mean {mean:.1} ms, median |err| {:.1} ms, 90% {:.1} ms, within 5ms {}", abs.get(abs.len() / 2).unwrap_or(&0.0), abs.get(abs.len() * 9 / 10).unwrap_or(&0.0), abs.iter().filter(|x| **x <= 5.0).count());
        for r in bpm_oct {
            eprintln!("  BPM ref {:.2} mine {:.2} :: {}", r.1, r.2, r.0);
        }
        for r in res.iter().filter(|r| (r.1 - r.2).abs() < 0.02 && r.3.abs() > 0.0) {
            eprintln!("  phase {:.1} ms :: {}", r.3, r.0);
        }
    }
}

#[cfg(test)]
mod edge_input_tests {
    use super::*;

    fn dec(samples: Vec<f32>) -> Decoded {
        let n = samples.len() / 2;
        Decoded { sr: 44100, samples, raw_frames: n, damaged: 0, truncated: false }
    }

    /// Silence, a single frame, an impulse and antiphase stereo analyse without panicking and
    /// without inventing tempo, key or loudness.
    #[test]
    fn degenerate_inputs() {
        let s = AnalysisSettings::default();
        let silent = analyze_decoded(&dec(vec![0.0; 44100 * 2 * 10]), &s);
        assert!(silent.gain_db.is_nan(), "silence has no loudness");
        assert!(silent.key < 0, "silence has no key");
        let one = analyze_decoded(&dec(vec![0.1, 0.1]), &s);
        assert!(one.gain_db.is_nan() && one.duration < 0.001);
        let mut imp = vec![0.0f32; 44100 * 2 * 5];
        imp[44100] = 1.0;
        imp[44101] = 1.0;
        let r = analyze_decoded(&dec(imp), &s);
        assert!(r.duration > 4.9);
        // Antiphase: the mono mix cancels, but loudness (per channel) does not.
        let anti: Vec<f32> = (0..44100 * 10).flat_map(|i| { let v = 0.1 * (i as f32 * 0.142).sin(); [v, -v] }).collect();
        let a = analyze_decoded(&dec(anti), &s);
        assert!(a.gain_db.is_finite());
    }
}
