//! `BeatAnalyzer_2_0::UnitBeatAdjust` of rekordbox 7.2.19: three band envelopes of the track at a
//! quarter of the sample rate, and the BPM / beat position refinement on them that
//! `Analyzer::analyzeFocus` calls (`calcLowInteg`, `calcAllInteg`, `analyze`).
//!
//! Input is what `Analyzer::setSample` (0x10116bae8) hands over: the decoder's samples scaled to
//! 16-bit range and truncated to `i16` (wrapping, as `vmovn` does), interleaved stereo.
//! `putSample` (0x10113ad58) takes every fourth frame, mono = (R + L) / 2, and keeps
//! - high: |HP 2000 Hz| then LP 1000 Hz,
//! - low: |LP 100 Hz| then LP 25 Hz,
//! - all: |x| then LP 1000 Hz,
//! each clamped to 0..32767 and stored as `i16`, at most 900 s.
//! The filters are rekordbox's `LowPathFilter2D` / `HighPathFilter2D` (0x101143c44, 0x101143e68),
//! Q = 1/sqrt 2, evaluated with the same fused multiply-adds as the original.

/// rekordbox's `LowPathFilter2D` / `HighPathFilter2D` (direct form I biquad).
#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

const TWO_PI_RB: f64 = f64::from_bits(0x401921fb54442d11);
const SQRT_HALF_RB: f64 = f64::from_bits(0x3fe6a09e667f3bcd);

impl Biquad {
    pub fn low_pass(f: f64, sr: f64) -> Biquad {
        let w = f * TWO_PI_RB / sr;
        let (s, c) = w.sin_cos();
        let sk = s * SQRT_HALF_RB;
        let half = (-c).mul_add(0.5, 0.5); // fmsub: 0.5 - c * 0.5
        let inv = 1.0 / (sk + 1.0);
        let b0 = half * inv;
        let b1 = (-c).mul_add(inv, inv); // inv - c * inv
        let a1 = (c * -2.0) * inv;
        let a2 = (-sk).mul_add(inv, inv); // inv - sk * inv
        Biquad { b0, b1, b2: b0, a1, a2, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    pub fn high_pass(f: f64, sr: f64) -> Biquad {
        let w = f * TWO_PI_RB / sr;
        let (s, c) = w.sin_cos();
        let sk = s * SQRT_HALF_RB;
        let half = c.mul_add(0.5, 0.5); // c * 0.5 + 0.5
        let inv = 1.0 / (sk + 1.0);
        let b0 = half * inv;
        let b1 = -(c + 1.0) * inv;
        let a1 = (c * -2.0) * inv;
        let a2 = (-sk).mul_add(inv, inv);
        Biquad { b0, b1, b2: b0, a1, a2, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    /// `operate(x, y, 1)`: y = x b0 + x1 b1 + x2 b2 - (y1 a1 + y2 a2), with rekordbox's fmadds.
    #[inline]
    pub fn run(&mut self, x: f64) -> f64 {
        let fb = self.y2.mul_add(self.a2, self.y1 * self.a1);
        let ff = self.x2.mul_add(self.b2, self.x1.mul_add(self.b1, x * self.b0));
        let y = ff - fb;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

fn clamp16(v: f64) -> i16 {
    if v < 0.0 {
        0
    } else if v > 32767.0 {
        32767
    } else {
        v as i32 as i16
    }
}

#[allow(dead_code)]
pub struct Uba {
    /// Total length of the track in frames.
    pub total: i32,
    /// Sample rate / 4.
    pub sr4: i32,
    /// Envelope length rekordbox allows (`min(total / 4, 900 s)`).
    pub cap: i32,
    pub high: Vec<i16>,
    pub low: Vec<i16>,
    pub all: Vec<i16>,
    /// `high` as f64, for the folding loops.
    pub high_f: Vec<f64>,
}

impl Uba {
    /// `create(sr, total)` + `putSample` over the whole track. `pcm16` = interleaved stereo as
    /// rekordbox's `setSample` truncates it (see [`to_i16`]); blocks of sr/75 frames as there.
    pub fn new(sr: i32, pcm16: &[i16]) -> Uba {
        Self::from_frames(sr, pcm16.len() / 2, |f| f64::from(pcm16[2*f]) + f64::from(pcm16[2*f+1]))
    }

    /// Only every fourth stereo frame feeds these envelopes. Quantize those
    /// frames directly rather than constructing another full-track PCM buffer.
    pub(super) fn from_audio(sr: u32, samples: &[f32]) -> Uba {
        Self::from_frames(sr as i32, samples.len() / 2, |f|
            f64::from(super::super::rb_beat1::reader_sample(samples[2*f])) +
            f64::from(super::super::rb_beat1::reader_sample(samples[2*f+1])))
    }

    fn from_frames(sr: i32, frames: usize, stereo_sum: impl Fn(usize) -> f64) -> Uba {
        let total = frames as i32;
        let sr4 = sr >> 2;
        let cap = (total / 4).min(900 * (sr / 4));
        let fs = sr4 as f64;
        let (mut hp, mut lp1, mut lp2, mut lp3, mut lp4) =
            (Biquad::high_pass(2000.0, fs), Biquad::low_pass(1000.0, fs), Biquad::low_pass(100.0, fs), Biquad::low_pass(25.0, fs), Biquad::low_pass(1000.0, fs));
        let n = cap.max(0) as usize;
        let mut high = Vec::with_capacity(n + 1);
        let mut low = Vec::with_capacity(n + 1);
        let mut all = Vec::with_capacity(n + 1);
        // Analyzer::setSample hands putSample blocks of sr/75 frames; each block is processed
        // from its first frame in steps of 4, the last (partial) one rounded down to 4.
        let block = (sr / 75).max(1) as usize;
        let frames = total as usize;
        let mut start = 0usize;
        let mut count = 0i32;
        while start < frames {
            let len = block.min(frames - start);
            let remaining = total - 4 * count;
            let rounded = if remaining >= 0 { remaining & !3 } else { remaining };
            let v43 = if (remaining as i64) < len as i64 { rounded } else { len as i32 };
            let v44 = 4 * (900 * sr4 - count);
            let v45 = v43.min(v44);
            let mut i = 0;
            while i < v45 {
                let f = start + i as usize;
                let x = stereo_sum(f) * 0.5;
                let h = lp1.run(hp.run(x).abs());
                let l = lp3.run(lp2.run(x).abs());
                let a = lp4.run(x.abs());
                high.push(clamp16(h));
                low.push(clamp16(l));
                all.push(clamp16(a));
                count += 1;
                i += 4;
            }
            start += len;
        }
        let high_f = high.iter().map(|&v| v as f64).collect();
        Uba { total, sr4, cap, high, low, all, high_f }
    }

    fn integ(&self, buf: &[i16], beat_frac: f64, bpm: f32, pos: i32) -> f64 {
        if !(bpm.abs() > 1.1921e-7 && pos >= -3) {
            return -1.0;
        }
        let n = ((self.sr4 as f64 * beat_frac / bpm as f64) + 0.5) as i32;
        let start = (pos as f64 * 0.25 + 0.5) as i32;
        if start + n - 1 > self.cap {
            return -1.0;
        }
        if n < 1 {
            return 0.0;
        }
        let mut s = 0f64;
        for k in start..start + n {
            s += buf.get(k as usize).copied().unwrap_or(0) as f64;
        }
        s
    }

    /// `calcLowInteg(bpm, pos)` (0x10113d760): low envelope summed over half a beat from `pos`.
    pub fn calc_low_integ(&self, bpm: f32, pos: i32) -> f64 {
        self.integ(&self.low, 30.0, bpm, pos)
    }

    /// `calcAllInteg(bpm, pos)` (0x10113d9ac): full-band envelope summed over one beat.
    pub fn calc_all_integ(&self, bpm: f32, pos: i32) -> f64 {
        self.integ(&self.all, 60.0, bpm, pos)
    }
}

/// `BeatAnalyzer_2_0::peakss::BeatInfo` (24 bytes in rekordbox): position in samples, beat
/// length in ms, beat number in the bar (1..4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatInfo {
    pub pos: i32,
    pub zero: i32,
    pub interval: f32,
    pub num: i32,
    pub minus1: i32,
    pub flag: bool,
}

impl BeatInfo {
    pub fn new(pos: i32, interval: f32, num: i32) -> BeatInfo {
        BeatInfo { pos, zero: 0, interval, num, minus1: -1, flag: false }
    }
}

/// `findpeaks_x` (0x1011685c4): local maxima of `x` with their prominence and width at half
/// prominence, MATLAB style. Returns (1-based index, value, prominence, width), each `n` long,
/// zero past the number of peaks.
pub fn findpeaks_x(n: usize, x: &[f64]) -> (Vec<i32>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut o_idx = vec![0i32; n];
    let mut o_val = vec![0f64; n];
    let mut o_prom = vec![0f64; n];
    let mut o_wid = vec![0f64; n];
    if n < 2 {
        return (o_idx, o_val, o_prom, o_wid);
    }
    let a2 = n as i64;
    let mut idx: Vec<i64> = vec![];
    let mut val: Vec<f64> = vec![];
    let mut prev = 0;
    for i in 0..n - 1 {
        let d = x[i] - x[i + 1];
        let cur;
        if d < 0.0 {
            cur = 1;
        } else {
            cur = if d > 0.0 { 2 } else { 0 };
            if prev == 1 {
                idx.push(i as i64 + 1);
                val.push(x[i]);
            }
        }
        prev = cur;
    }
    let m = idx.len();
    if m == 0 {
        return (o_idx, o_val, o_prom, o_wid);
    }
    let xa = |k: i64| x[k as usize];
    let len = x.len() as i64;
    let mut v64: i64 = 0;
    let mut v65: i64 = 0;
    let mut prom = vec![0f64; m];
    for p in 0..m {
        let v70 = val[p];
        let v71 = idx[p];
        let v72 = v71 - 1;
        let mut v75;
        // left side
        let mut reached_start = false;
        let mut v73 = v71 - 1;
        loop {
            if !(v70 >= xa(v73)) {
                break;
            }
            if v73 == 0 {
                v65 = 1;
            }
            let v74 = v73 + 1;
            v73 -= 1;
            if v74 <= 1 {
                reached_start = true;
                break;
            }
        }
        if reached_start {
            v75 = xa(v65 - 1);
        } else {
            v65 = v73 + 1;
            v75 = xa(v73);
        }
        if v71 >= v65 {
            for k in v65..=v71 {
                v75 = v75.min(xa(k));
            }
        }
        // right side
        let v96;
        if v71 <= a2 {
            let mut v93 = a2 - v71;
            let mut v94 = v71 - 1;
            let mut v95 = v71;
            let mut to_end = false;
            loop {
                if !(v70 >= xa(v94)) {
                    break;
                }
                if v93 == 0 {
                    v64 = a2;
                }
                v95 += 1;
                v93 -= 1;
                v94 += 1;
                if a2 + 1 == v95 {
                    to_end = true;
                    break;
                }
            }
            if !to_end {
                v64 = v95;
            }
            let mut m96 = xa(v72);
            if v71 <= v64 {
                let mut v97 = if len < v71 { 0 } else { len - v71 };
                let mut k = v71;
                let mut v99 = v64 - v71 + 1;
                loop {
                    if v97 == 0 {
                        break;
                    }
                    m96 = m96.min(xa(k));
                    k += 1;
                    v97 -= 1;
                    v99 -= 1;
                    if v99 == 0 {
                        break;
                    }
                }
            }
            v96 = m96;
        } else {
            v96 = xa(v72);
        }
        prom[p] = v70 - v75.max(v96);
    }
    let mut wid = vec![0f64; m];
    for p in 0..m {
        let v108 = prom[p].mul_add(-0.5, val[p]);
        let mut v109 = idx[p];
        let v115;
        let mut v110 = v109 - 1;
        let mut reached_start = false;
        loop {
            if !(v108 <= xa(v110)) {
                break;
            }
            if v110 == 0 {
                v65 = 1;
            }
            let v111 = v110 + 1;
            v110 -= 1;
            if v111 <= 1 {
                reached_start = true;
                break;
            }
        }
        let (v113, v114) = if reached_start {
            (xa(v65 - 1), xa(v65) - xa(v65 - 1))
        } else {
            v65 = v110 + 1;
            (xa(v110), xa(v110 + 1) - xa(v110))
        };
        v115 = if v114 != 0.0 { (v108 - v113) / v114 } else { 0.0 };
        if v109 <= a2 {
            let mut v117 = a2 - v109;
            let mut v118 = v109 - 1;
            let mut to_end = false;
            loop {
                if !(v108 <= xa(v118)) {
                    break;
                }
                if v117 == 0 {
                    v64 = a2;
                }
                v109 += 1;
                v117 -= 1;
                v118 += 1;
                if a2 + 1 == v109 {
                    to_end = true;
                    break;
                }
            }
            if !to_end {
                v64 = v109;
            }
        }
        let v104 = v64 - 1;
        let v105 = xa(v64 - 2);
        let v106 = xa(v104) - v105;
        let v107 = if v106 == 0.0 { 0.0 } else { (v108 - v105) / v106 };
        wid[p] = (v104 as i32) as f64 - (v115 + v65 as f64) + v107;
    }
    for p in 0..m {
        o_idx[p] = idx[p] as i32;
        o_val[p] = val[p];
        o_prom[p] = prom[p];
        o_wid[p] = wid[p];
    }
    (o_idx, o_val, o_prom, o_wid)
}

/// Sum of squared deviations as the vectorised loop of `findBeatPositionForBeatSubEach`
/// computes it: 8 fused-multiply-add lanes, combined pairwise, then the tail.
fn sq_dev_sum(v: &[f64], mean: f64) -> f64 {
    let n8 = if v.len() >= 8 { v.len() & !7 } else { 0 };
    let mut acc = [0f64; 8];
    let mut c = 0;
    while c < n8 {
        for l in 0..8 {
            let d = v[c + l] - mean;
            acc[l] = d.mul_add(d, acc[l]);
        }
        c += 8;
    }
    let mut s = if n8 > 0 {
        let a0 = acc[2] + acc[0];
        let a1 = acc[3] + acc[1];
        let a0 = acc[4] + a0;
        let a1 = acc[5] + a1;
        let a0 = acc[6] + a0;
        let a1 = acc[7] + a1;
        a0 + a1
    } else {
        0.0
    };
    for &x in &v[n8..] {
        let d = x - mean;
        s = d.mul_add(d, s);
    }
    s
}

/// Plain sum as a vectorised `vaddq` loop with 8 lanes does it (lanes combined 0+2+4+6,
/// 1+3+5+7, then the two halves), then the tail in order.
fn lane_sum(v: &[f64]) -> f64 {
    let n8 = if v.len() >= 8 { v.len() & !7 } else { 0 };
    let mut acc = [0f64; 8];
    let mut c = 0;
    while c < n8 {
        for l in 0..8 {
            acc[l] = v[c + l] + acc[l];
        }
        c += 8;
    }
    let mut s = if n8 > 0 {
        let a0 = acc[2] + acc[0];
        let a1 = acc[3] + acc[1];
        let a0 = acc[4] + a0;
        let a1 = acc[5] + a1;
        let a0 = acc[6] + a0;
        let a1 = acc[7] + a1;
        a0 + a1
    } else {
        0.0
    };
    for &x in &v[n8..] {
        s = x + s;
    }
    s
}

/// State of the BPM / position refinement (`UnitBeatAdjust` fields +24, +32, +40, +48..+80, +88).
pub struct UbaRefine<'a> {
    pub env: &'a Uba,
    /// Number of candidate beat lists made so far (+24).
    counter: i32,
    /// Beats in the list being refined (+32).
    count: i32,
    /// Largest folded value of the last `findBeatPositionForBeatSubAll` (+40).
    peak40: f64,
    /// Candidate beat lists (+88), kept only for indices that can still be chosen.
    beat_data: std::collections::HashMap<i32, Vec<i32>>,
    /// Folded profiles: all beats (+48) and beats 1, 2, 3, 4 of the bar (+56, +64, +72, +80).
    fold: [Vec<f64>; 5],
    /// Candidate lists actually stored (+88 size); once a candidate is skipped the original's
    /// indices no longer match its vector and every later lookup fails.
    pushed: i32,
}

impl<'a> UbaRefine<'a> {
    pub fn new(env: &'a Uba) -> UbaRefine<'a> {
        let w = (2 * ((env.sr4 as f64 * 0.05) as i32)).max(1) as usize;
        UbaRefine { env, counter: 0, count: 0, peak40: 0.0, pushed: 0, beat_data: Default::default(), fold: [vec![0.0; w], vec![0.0; w], vec![0.0; w], vec![0.0; w], vec![0.0; w]] }
    }

    fn half_window(&self) -> i32 {
        (self.env.sr4 as f64 * 0.05) as i32
    }

    /// `UnitBeatAdjust::analyze(bpm, list)` (0x10113af38).
    pub fn analyze(&mut self, bpm: &mut f32, list: &mut Vec<BeatInfo>) {
        if !(*bpm > 0.0) || list.is_empty() {
            return;
        }
        let n = list.len() as i32;
        self.count = n;
        let mut beats = n;
        let sr4 = self.env.sr4;
        let lim = 900 * sr4;
        if self.env.cap >= lim {
            let d1 = (list[0].pos as f64).mul_add(-0.25, lim as f64);
            beats = (((*bpm as f64) * f64::from_bits(0x3f91111111111111)) * d1 / sr4 as f64) as i32;
        }
        self.bpm_adjust(bpm, list, beats);
        self.beat_position_adjust(list);
    }

    /// `createBeatData(bpm, start)` (0x10113c9d8): beat positions (quarter rate) from `start`.
    fn create_beat_data(&self, bpm: f64, start: i32) -> Option<Vec<i32>> {
        if !(bpm > 0.0) || start < 0 || self.env.cap <= start {
            return None;
        }
        let n = self.count;
        if n < 1 {
            return None;
        }
        let step = self.env.sr4 as f64 * 60.0 / bpm;
        let limit = (self.env.total as f64 * 0.25).floor();
        let st = start as f64;
        let mut v = Vec::with_capacity(n as usize + 8);
        let mut p = 0.0;
        for i in 0..n {
            p = step.mul_add(i as f64, st);
            if p < limit {
                v.push((p + 0.5) as i32);
            }
        }
        let mut q = p + step;
        while q < limit {
            v.push((q + 0.5) as i32);
            q += step;
        }
        Some(v)
    }

    /// `manageWave(beats, idx, peak)` (0x10113ce60): fold the high envelope around the beats of
    /// one candidate list; returns the largest folded value (None when rekordbox leaves it).
    fn manage_wave(&mut self, beats: i32, v: &[i32]) -> Option<f64> {
        if beats < 1 {
            return None;
        }
        let v7 = self.half_window();
        let v11 = (v.len() as i32).min(beats);
        let v12 = 2 * v7;
        let buf = &mut self.fold[0];
        if v7 >= 1 {
            for b in buf.iter_mut().take(v12.max(1) as usize) {
                *b = 0.0;
            }
        }
        if v11 >= 3 {
            let cap = self.env.cap;
            for k in 1..(v11 - 1) as usize {
                let c = v[k];
                let lo = c - v7;
                let hi = c + v7;
                if lo >= 0 && hi > lo && cap >= hi && 2 * v7 >= 1 && 2 * v7 <= v12 && 2 * v7 <= cap {
                    let a = (lo as usize).min(self.env.high_f.len());
                    let b = (lo as usize + v12 as usize).min(self.env.high_f.len());
                    for (x, e) in buf[..b - a].iter_mut().zip(&self.env.high_f[a..b]) {
                        *x += e;
                    }
                }
            }
        }
        if v7 >= 1 {
            let mut mx = 0f64;
            for &b in buf.iter().take(v12 as usize) {
                mx = b.max(mx);
            }
            return Some(mx);
        }
        None
    }

    /// `manageWave` without the shared scratch buffer (same sums), for the parallel scan.
    fn fold_peak(&self, beats: i32, v: &[i32], buf: &mut Vec<f64>) -> Option<f64> {
        if beats < 1 {
            return None;
        }
        let v7 = self.half_window();
        let v11 = (v.len() as i32).min(beats);
        let v12 = 2 * v7;
        if v7 < 1 {
            return None;
        }
        buf.clear();
        buf.resize(v12.max(1) as usize, 0.0);
        if v11 >= 3 {
            let cap = self.env.cap;
            for &c in &v[1..(v11 - 1) as usize] {
                let lo = c - v7;
                let hi = c + v7;
                if lo >= 0 && hi > lo && cap >= hi && 2 * v7 <= cap {
                    let a = (lo as usize).min(self.env.high_f.len());
                    let b = (lo as usize + v12 as usize).min(self.env.high_f.len());
                    for (x, e) in buf[..b - a].iter_mut().zip(&self.env.high_f[a..b]) {
                        *x += e;
                    }
                }
            }
        }
        let mut mx = 0f64;
        for &b in buf.iter().take(v12 as usize) {
            mx = b.max(mx);
        }
        Some(mx)
    }

    /// `getPeakBpm(bpm, steps, start, beats, &bpm_out, &peak_out, &idx_out, &more)` (0x10113c028).
    #[allow(clippy::too_many_arguments)]
    fn get_peak_bpm(&mut self, bpm0: f64, a2: i32, start: i32, beats: i32, out_bpm: &mut f64, out_peak: &mut f64, out_idx: &mut i32, more: &mut bool) {
        if start < 0 || bpm0 <= 0.0 || !(1..=51).contains(&a2) || beats < 1 || self.env.cap <= start {
            return;
        }
        let mut local = 0f64;
        let n = (32 * a2) as usize;
        let mut bpms = vec![0f64; n];
        let mut peaks = vec![0f64; n];
        let cand: Vec<f64> = (0..n).map(|i| ((i as f64).mul_add(0.0000625, bpm0)).max(70.0).min(180.0)).collect();
        // every candidate's folded peak, computed in parallel (independent of each other)
        let threads = std::thread::available_parallelism().map(|v| v.get()).unwrap_or(4).clamp(1, 8);
        let mut folded: Vec<(bool, Option<f64>)> = vec![(false, None); n];
        {
            let this = &*self;
            let chunk = n.div_ceil(threads).max(1);
            std::thread::scope(|sc| {
                for (part, out) in cand.chunks(chunk).zip(folded.chunks_mut(chunk)) {
                    sc.spawn(move || {
                        let mut buf = Vec::new();
                        for (b, o) in part.iter().zip(out.iter_mut()) {
                            match this.create_beat_data(*b, start) {
                                Some(d) => *o = (true, this.fold_peak(beats, &d, &mut buf)),
                                None => *o = (false, None),
                            }
                        }
                    });
                }
            });
        }
        let mut best: Option<(i32, f64)> = None;
        for i in 0..n {
            let b = cand[i];
            let (stored, peak) = folded[i];
            if stored {
                self.pushed += 1;
            }
            // manageWave reads vector number `counter`; it only exists while every candidate so
            // far was stored
            if stored && self.pushed == self.counter + 1 {
                if let Some(p) = peak {
                    local = p;
                }
            }
            bpms[i] = b;
            peaks[i] = local;
            if *out_peak < local {
                *out_peak = local;
                *out_bpm = b;
                *out_idx = self.counter;
                if stored {
                    best = Some((self.counter, b));
                }
            }
            self.counter += 1;
        }
        if let Some((k, b)) = best {
            if let Some(d) = self.create_beat_data(b, start) {
                self.beat_data.insert(k, d);
            }
        }
        let (idx, pk, prom, _w) = findpeaks_x(n, &peaks);
        let mut found = 0usize;
        let mut all_nonzero = true;
        for &v in idx.iter().take(n.max(1)) {
            if v == 0 {
                all_nonzero = false;
                break;
            }
            found += 1;
        }
        if all_nonzero {
            *more = true;
        } else {
            let v53 = found;
            *more = true;
            if v53 >= 21 {
                let v54 = *out_peak * 0.075;
                let mut none_big = true;
                for &p in &prom[10..v53 - 10] {
                    if p > v54 {
                        none_big = false;
                        *more = false;
                    }
                }
                if !none_big {
                    let mut v58 = 0usize;
                    let mut v61 = 0f64;
                    for j in 0..v53.max(1) {
                        if v61 < prom[j] {
                            v58 = (idx[j] - 1) as usize;
                            v61 = prom[j];
                        }
                    }
                    let v63 = bpms[v58];
                    if *out_bpm != v63 {
                        *out_bpm = v63;
                    }
                    let v64 = *out_peak * 0.75;
                    let v66 = pk.iter().take(v53.max(1)).filter(|&&p| p > v64).count();
                    if v66 as f64 / v53 as f64 > 0.5 {
                        *more = true;
                    }
                }
            }
        }
        let v84 = (bpm0 + 0.05).round();
        if v84 >= bpm0 && v84 <= bpm0 + 0.1 {
            *more = false;
        }
        if *out_bpm <= 70.007 || *out_bpm >= 179.982 {
            *more = false;
        }
    }

    /// One scan of `bpmAdjust`: repeated `getPeakBpm` from `from` in steps of `step` while
    /// it asks for more. Returns (best bpm, best idx, best peak, bpm of the last call).
    fn scan(&mut self, from: f64, step: f64, start: i32, beats: i32, pre_step: bool, best_bpm: &mut f64, best_idx: &mut i32, best_peak: &mut f64) -> f64 {
        let mut b = from;
        let mut out_bpm = 0f64;
        let mut out_idx = 0i32;
        let mut more = true;
        let mut last = 0f64;
        loop {
            if pre_step {
                b += step;
            }
            let mut peak = 0f64;
            self.get_peak_bpm(b, 51, start, beats, &mut out_bpm, &mut peak, &mut out_idx, &mut more);
            if !pre_step {
                b += step;
            }
            last = out_bpm;
            if *best_peak < peak {
                *best_bpm = out_bpm;
                *best_idx = out_idx;
            }
            *best_peak = best_peak.max(peak);
            if !more {
                break;
            }
        }
        last
    }

    /// `bpmAdjust(bpm, list, beats)` (0x10113aff8).
    fn bpm_adjust(&mut self, bpm: &mut f32, list: &mut Vec<BeatInfo>, beats: i32) {
        if beats < 1 || !(*bpm > 0.05f32) || list.is_empty() {
            return;
        }
        let n = list.len();
        let first_pos = |list: &Vec<BeatInfo>, add: i32| -> i32 {
            let mut v = 0;
            for (k, b) in list.iter().enumerate() {
                v = b.pos + add;
                if !(v <= -4 && k + 1 < n) {
                    break;
                }
            }
            v / 4
        };
        let v44 = first_pos(list, 0);
        let v33 = *bpm;
        let frac = *bpm - bpm.floor();
        let mut v47 = 0i32;
        let mut v48 = 0f64;
        let mut v57 = 0f64;
        let v50;
        let start = v33 as f64 + -0.05;
        if frac == 0.5 {
            let mut b1 = 0f64;
            let mut i1 = 0;
            let mut p1 = 0f64;
            // the original keeps the bpm of the last call of the first (upward) scan
            v50 = self.scan(start, 0.05, v44, beats, false, &mut b1, &mut i1, &mut p1);
            let mut b2 = 0f64;
            let mut i2 = 0;
            let mut p2 = 0f64;
            let _ = self.scan(*bpm as f64, -0.05, v44, beats, true, &mut b2, &mut i2, &mut p2);
            if p1 >= p2 {
                v57 = b1;
                v47 = i1;
            } else {
                v57 = b2;
                v47 = i2;
            }
            v48 = p1.max(p2);
        } else if frac <= 0.5 {
            v50 = self.scan(start, -0.05, v44, beats, false, &mut v57, &mut v47, &mut v48);
        } else {
            v50 = self.scan(start, 0.05, v44, beats, false, &mut v57, &mut v47, &mut v48);
        }
        let v58 = v50.round();
        if v50 != v58 && v50 >= v58 * 0.9995 && v50 <= v58 * 1.0005 {
            let (mut ob, mut oi, mut pk, mut more) = (0f64, 0i32, 0f64, true);
            self.get_peak_bpm(v58 + -0.05, 51, v44, beats, &mut ob, &mut pk, &mut oi, &mut more);
            if v48 < pk {
                v57 = ob;
                v47 = oi;
                v48 = pk;
            }
        }
        // The same search half a beat later (off-beat phase).
        let mut v61 = 0i32;
        if v57.abs() > 2.22044605e-16 {
            v61 = (self.env.sr4 as f64 * 30.0 / v57 + 0.5) as i32;
        }
        let v67 = first_pos(list, v61);
        let frac2 = v57 - v57.floor();
        let mut v70 = 0i32;
        let mut v77 = 0f64;
        let mut v80 = 0f64;
        let v75;
        let start2 = v57 + -0.05;
        if frac2 == 0.5 {
            let mut b1 = 0f64;
            let mut i1 = 0;
            let mut p1 = 0f64;
            v75 = self.scan(start2, 0.05, v67, beats, false, &mut b1, &mut i1, &mut p1);
            let mut b2 = 0f64;
            let mut i2 = 0;
            let mut p2 = 0f64;
            let _ = self.scan(start2, -0.05, v67, beats, false, &mut b2, &mut i2, &mut p2);
            v80 = p1.max(p2);
            if p1 >= p2 {
                v70 = i1;
                v77 = b1;
            } else {
                v70 = i2;
                v77 = b2;
            }
        } else if frac2 <= 0.5 {
            v75 = self.scan(start2, -0.05, v67, beats, false, &mut v77, &mut v70, &mut v80);
        } else {
            v75 = self.scan(start2, 0.05, v67, beats, false, &mut v77, &mut v70, &mut v80);
        }
        let v83 = v75.round();
        if v75 != v83 && v75 >= v83 * 0.9995 && v75 <= v83 * 1.0005 {
            let (mut ob, mut oi, mut pk, mut more) = (0f64, 0i32, 0f64, true);
            self.get_peak_bpm(v83 + -0.05, 51, v67, beats, &mut ob, &mut pk, &mut oi, &mut more);
            if v80 < pk {
                v77 = ob;
                v70 = oi;
                v80 = pk;
            }
        }
        if v48 >= v80 * 0.96 {
            self.reflect_beat_data(bpm, list, v47, v57);
            return;
        }
        for b in list.iter_mut() {
            b.pos += v61;
        }
        self.reflect_beat_data(bpm, list, v70, v77);
    }

    /// `reflectBeatData(bpm_val, bpm, list, idx)` (0x10113c670): the chosen candidate's beats
    /// become the list.
    fn reflect_beat_data(&mut self, bpm: &mut f32, list: &mut Vec<BeatInfo>, idx: i32, a5: f64) {
        if !(a5 > 0.0) || idx < 0 || idx >= self.pushed || !(*bpm > 0.0) {
            return;
        }
        let Some(v12) = self.beat_data.get(&idx).cloned() else { return };
        if list.is_empty() || (*bpm as f64 * 1000.0) as i32 == (a5 * 1000.0) as i32 {
            return;
        }
        let v13 = v12.len() as i32;
        let mut v14 = self.count;
        while v14 != v13 {
            while v14 > v13 {
                list.pop();
                v14 -= 1;
                self.count = v14;
            }
            if v14 == v13 {
                break;
            }
            let last = list[(v14 - 1) as usize];
            list.push(last);
            let e = &mut list[v14 as usize];
            e.num = if e.num == 4 { 1 } else { e.num + 1 };
            e.flag = false;
            v14 += 1;
            self.count = v14;
        }
        let mut v31 = list[0].num;
        let mut v32 = v31 - 1;
        let v33 = v12[0];
        if v13 > 1 {
            let mut v36 = (-4 * v33 + list[0].pos).abs();
            for k in 1..v13 as usize {
                if v31 <= list[k].num {
                    v31 = list[k].num;
                }
                let v38 = (list[k].pos - 4 * v33).abs();
                let closer = v36 > v38;
                if v36 >= v38 {
                    v36 = v38;
                }
                if closer {
                    v32 = list[k].num - 1;
                }
            }
        }
        if v13 >= 1 {
            let v40 = (60000.0 / a5) as f32;
            list[0].pos = 4 * v33;
            list[0].interval = v40;
            list[0].num = v32 % v31 + 1;
            for k in 1..v13 as usize {
                list[k].pos = 4 * v12[k];
                list[k].interval = v40;
                list[k].num = (v32 + k as i32) % v31 + 1;
            }
        }
        *bpm = a5 as f32;
    }

    /// `beatPositionAdjust(list)` (0x10113b7d4).
    fn beat_position_adjust(&mut self, list: &mut Vec<BeatInfo>) {
        if list.is_empty() {
            return;
        }
        let n = list.len() as i32;
        let v17 = self.half_window();
        let v18 = 2 * v17;
        if v17 >= 1 {
            for f in self.fold.iter_mut() {
                for x in f.iter_mut().take(v18.max(1) as usize) {
                    *x = 0.0;
                }
            }
        }
        if n >= 3 {
            let cap = self.env.cap;
            for i in 1..(n - 1) as usize {
                let c = list[i].pos / 4;
                let lo = c - v17;
                let hi = c + v17;
                let valid = lo >= 0 && hi > lo && cap >= hi && (hi - lo) >= 1 && (hi - lo) <= v18 && (hi - lo) <= cap;
                if !valid {
                    continue;
                }
                // all beats, and the beat's slot in the bar: i % 4 == 1, 2, 3, 0 -> +56, +64, +72, +80
                let slot = match i & 3 {
                    1 => 1,
                    2 => 2,
                    3 => 3,
                    _ => 4,
                };
                let a = (lo as usize).min(self.env.high_f.len());
                let b = (lo as usize + v18 as usize).min(self.env.high_f.len());
                let src = &self.env.high_f[a..b];
                for (x, e) in self.fold[0].iter_mut().zip(src) {
                    *x += e;
                }
                for (x, e) in self.fold[slot].iter_mut().zip(src) {
                    *x += e;
                }
            }
        }
        let mut d = 0f64;
        let mut off = 0i32;
        self.find_beat_position_for_beat(v18, &mut d, &mut off);
        self.offset_beat_position(v17, off, list);
    }

    /// `findBeatPositionForBeat(n, &val, &pos)` (0x10113d18c).
    fn find_beat_position_for_beat(&mut self, a2: i32, a3: &mut f64, a4: &mut i32) {
        if a2 < 1 || a2 > 2 * self.half_window() {
            return;
        }
        self.sub_all(a2, a3, a4);
        let mut r = [(0f64, 0i32, false); 4];
        for (k, out) in r.iter_mut().enumerate() {
            let buf = self.fold[k + 1].clone();
            self.sub_each(&buf, a2, &mut out.0, &mut out.1, &mut out.2);
        }
        let mut v31 = r[0].1;
        if r[0].2 || r[0].1 <= 0 {
            v31 = 0;
        }
        for k in 1..4 {
            if !r[k].2 && v31 < r[k].1 {
                v31 = r[k].1;
            }
        }
        if *a4 <= v31 - 35 {
            *a4 = v31;
        }
    }

    /// 32-tap moving average of a folded profile (`v30` in the original), with its maximum
    /// and the index of the first maximum.
    fn moving_average(buf: &[f64], a2: i32) -> (Vec<f64>, f64, usize) {
        let v28 = (a2 - 32) as usize;
        let mut ma = vec![0f64; v28];
        let mut mx = 0f64;
        let mut arg = 0usize;
        for i in 0..v28 {
            let mut s = 0f64;
            for k in 0..32 {
                s += buf[i + k];
            }
            let v = s * 0.03125;
            if mx < v {
                arg = i;
            }
            ma[i] = v;
            mx = mx.max(v);
        }
        (ma, mx, arg)
    }

    /// `findBeatPositionForBeatSubAll(n, &val, &pos)` (0x10113dbfc).
    fn sub_all(&mut self, a2: i32, a3: &mut f64, a4: &mut i32) {
        if a2 < 34 || a2 > 2 * self.half_window() {
            return;
        }
        let buf = self.fold[0].clone();
        let v28 = (a2 - 32) as usize;
        let v29 = (a2 - 33) as usize;
        let (v30, v35, v33) = Self::moving_average(&buf, a2);
        let mut v31 = vec![0f64; v29];
        let mut v71 = 0f64;
        for j in 0..v29 {
            let d = v30[j + 1] - v30[j];
            v31[j] = d;
            v71 = v71.max(d);
        }
        let v77 = v71 * 0.25;
        let (idx, pk, prom, _w) = findpeaks_x(v28, &v30);
        let mut v86 = 0usize;
        let mut v87 = 0i32;
        let mut cnt = 0usize;
        let mut all = true;
        for j in 0..v28 {
            if idx[j] == 0 {
                all = false;
                break;
            }
            cnt += 1;
        }
        if all {
            v86 = 0;
            v87 = 0;
        } else if cnt != 0 {
            let mut v91 = 0f64;
            let mut v90 = 0f64;
            for j in 0..cnt {
                v91 = v91.max(pk[j]);
                v90 = v90.max(prom[j]);
            }
            let v117 = v90 * 0.25;
            let mut j = 0usize;
            let mut found = false;
            while j < cnt {
                if pk[j] > v91 * 0.65 && prom[j] > v117 {
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                v86 = 0;
                v87 = 0;
            } else {
                v86 = idx[j] as usize;
                v87 = 0;
                if j != 0 {
                    let v118 = v91 * 0.25;
                    let mut k = j as i64 - 1;
                    loop {
                        if pk[k as usize] > v118 && prom[k as usize] > v117 {
                            v87 = idx[k as usize];
                        }
                        let v120 = k + 1;
                        k -= 1;
                        if v120 <= 1 {
                            break;
                        }
                    }
                }
            }
        }
        // the lowest point before the maximum
        let mut v122 = 0usize;
        if v33 > 1 {
            let mut v123 = false;
            let mut v124 = v30[0];
            let mut v126 = v30[0];
            for v125 in 1..v33 {
                if v123 {
                    v124 = v126;
                }
                v126 = v30[v125];
                v123 = v124 > v126;
                if v124 > v126 {
                    v122 = v125;
                }
            }
        }
        *a3 = 0.0;
        *a4 = 0;
        if (v87 as i64) < v86 as i64 {
            let mut run = 0;
            let mut k = v87 as usize;
            while k != v86 {
                if v31[k] <= v77 || v30[k] <= v35 * 0.4 || k <= v122 {
                    run = 0;
                    k += 1;
                    continue;
                }
                if run >= 7 {
                    let p = k + 8;
                    *a3 = buf[p];
                    *a4 = p as i32;
                    break;
                }
                run += 1;
                k += 1;
            }
        }
        let mut v150 = 0usize;
        let mut v151 = 0f64;
        for (k, &b) in buf.iter().enumerate().take(a2 as usize) {
            let better = v151 < b;
            v151 = v151.max(b);
            if better {
                v150 = k;
            }
        }
        self.peak40 = v151;
        let v155 = if buf[0] >= v151 * 0.7 {
            let mut v156 = 1usize;
            let mut res = None;
            while v156 != a2 as usize {
                let v157 = buf[v156];
                v156 += 1;
                if v157 < v151 * 0.7 {
                    res = Some(((v156 - 1) as i32) < a2);
                    break;
                }
            }
            res.unwrap_or((v156 as i32) < a2)
        } else {
            true
        };
        if (*a4 == 0 && *a3 == 0.0) || !v155 {
            *a3 = v151;
            *a4 = v150 as i32;
        }
    }

    /// `findBeatPositionForBeatSubEach(buf, n, &val, &pos, &bad)` (0x10113e538).
    fn sub_each(&mut self, a2: &[f64], a3: i32, out_val: &mut f64, out_pos: &mut i32, bad: &mut bool) {
        if a3 < 34 || a3 > 2 * self.half_window() {
            return;
        }
        let v31 = (a3 - 32) as usize;
        let v32 = (a3 - 33) as usize;
        let (v33, v38, v36) = Self::moving_average(a2, a3);
        let mut v34 = vec![0f64; v32];
        let mut v45 = 0f64;
        for j in 0..v32 {
            let d = v33[j + 1] - v33[j];
            v34[j] = d;
            v45 = v45.max(d);
        }
        let v43 = lane_sum(&v34);
        let v50 = v43 / v32 as f64;
        let v52 = sq_dev_sum(&v34, v50);
        let v71 = self.peak40;
        let (mut v72, mut v73) = (0f64, 0f64);
        if v71.abs() > 2.22044605e-16 {
            v72 = v45 / v71;
            v73 = v52 / (v71 * v32 as f64);
        }
        let (idx, pk, prom, _w) = findpeaks_x(v31, &v33);
        let mut cnt = 0usize;
        let mut all = true;
        for j in 0..v31 {
            if idx[j] == 0 {
                all = false;
                break;
            }
            cnt += 1;
        }
        let v82: i64 = if all {
            0
        } else if cnt != 0 {
            let mut v86 = 0f64;
            let mut v85 = 0f64;
            for j in 0..cnt {
                v86 = v86.max(pk[j]);
                v85 = v85.max(prom[j]);
            }
            let v112 = v86 * 0.75;
            let v113 = v85 * 0.25;
            let mut r = 0i64;
            for j in 0..cnt {
                if pk[j] > v112 && prom[j] > v113 {
                    r = idx[j] as i64;
                    break;
                }
            }
            r
        } else {
            0
        };
        // lowest point before the maximum
        let mut v115 = v33[0];
        let mut v116 = 0usize;
        if v36 > 1 {
            let mut v118 = v33[0];
            for v117 in 1..v36 {
                let v119 = v33[v117];
                v115 = v118.min(v119);
                if v118 > v119 {
                    v116 = v117;
                }
                v118 = v115;
            }
        }
        let mut v120 = v115 + (v38 - v115) * 0.4;
        if v38 * 0.4 >= v115 {
            v120 = v38 * 0.4;
        }
        *out_val = 0.0;
        *out_pos = 0;
        let v121 = v116 + 1;
        let (v129, v130, v131): (i64, f64, i64);
        if (v121 as i64) >= v82 {
            v129 = 0;
            v131 = 0;
            v130 = 0.0;
        } else {
            let v127 = (v82 - 1) as usize;
            let mut v128 = v116;
            let mut run = 0;
            let mut k = v121;
            let mut got = None;
            loop {
                if v34[k] <= v45 * 0.25 || v33[k] <= v120 {
                    run = 0;
                    v128 += 1;
                    k += 1;
                    if v127 == v128 {
                        break;
                    }
                    continue;
                }
                if run >= 4 {
                    got = Some(v128);
                    break;
                }
                run += 1;
                v128 += 1;
                k += 1;
                if v127 == v128 {
                    break;
                }
            }
            if let Some(m) = got {
                v129 = m as i64 + 12;
                v130 = a2[v129 as usize];
                *out_val = v130;
                *out_pos = v129 as i32;
            } else {
                v129 = 0;
                v130 = 0.0;
            }
            // the end of the rise: 10 frames in a row above the threshold
            let mut run2 = 0;
            let mut j = v116;
            let mut kk = v121;
            let mut end = None;
            loop {
                if v33[kk] <= v120 {
                    run2 = 0;
                    j += 1;
                    kk += 1;
                    if v127 == j {
                        break;
                    }
                    continue;
                }
                if run2 >= 9 {
                    end = Some(j);
                    break;
                }
                run2 += 1;
                j += 1;
                kk += 1;
                if v127 == j {
                    break;
                }
            }
            v131 = end.map(|m| m as i64 + 7).unwrap_or(0);
        }
        let v133 = a3 as usize;
        let v132 = a2[0];
        let mut v134 = 0usize;
        let mut v136 = v132;
        let mut v137 = v132;
        let mut v139 = v132;
        for v135 in 1..v133 {
            let v138 = a2[v135];
            v139 = v137.max(v138);
            if v137 < v138 {
                v134 = v135;
            }
            v136 = v136.min(v138);
            v137 = v139;
        }
        let (mut v165, mut v166) = (0f64, 0f64);
        if v139.abs() > 2.22044605e-16 {
            v165 = v136 / v139;
            v166 = (v139 - v136) / v139;
        }
        let v167 = a2[v134];
        let mut v169 = v167;
        for &x in &a2[v134 + 1..v133] {
            v169 = v169.min(x);
        }
        let v186 = v139 - v169;
        let (mut v190, mut v189): (u32, f64);
        if v134 >= v133 {
            v190 = 0;
            v189 = 0.0;
        } else {
            let v188 = v186 * 0.25;
            v189 = v167 - v169;
            v190 = (v189 > v188) as u32;
            if v189 <= v188 {
                v189 = -0.0;
            }
            // the original sums the rest with 8 lanes when there are at least 8 values
            let rest = &a2[v134 + 1..v133];
            let n8 = if rest.len() >= 8 { rest.len() & !7 } else { 0 };
            let mut lanes = [0f64; 8];
            lanes[0] = v189;
            for c in (0..n8).step_by(8) {
                for l in 0..8 {
                    let d = rest[c + l] - v169;
                    if d > v188 {
                        v190 += 1;
                        lanes[l] += d;
                    } else {
                        lanes[l] += -0.0;
                    }
                }
            }
            if n8 > 0 {
                let a0 = lanes[2] + lanes[0];
                let a1 = lanes[3] + lanes[1];
                let a0 = lanes[4] + a0;
                let a1 = lanes[5] + a1;
                let a0 = lanes[6] + a0;
                let a1 = lanes[7] + a1;
                v189 = a0 + a1;
            }
            for &x in &rest[n8..] {
                let mut d = x - v169;
                if d <= v188 {
                    d = -0.0;
                } else {
                    v190 += 1;
                }
                v189 = d + v189;
            }
        }
        let v223 = if v186.abs() > 2.22044605e-16 { v186 } else { 1.0 };
        let v224 = v139 * 0.7;
        let v225 = if v132 >= v224 {
            let mut v226 = 1usize;
            let mut res = None;
            while v133 != v226 {
                let v227 = a2[v226];
                v226 += 1;
                if v227 < v224 {
                    res = Some((v226 - 1) < a3 as usize);
                    break;
                }
            }
            res.unwrap_or(v226 < a3 as usize)
        } else {
            true
        };
        let v231 = (v129 == 0 && v130 == 0.0) || !v225 || v131 < v129 - 20;
        let v232 = v72 < 0.0015 && v73 < 0.1;
        if v231 || (v190 <= 0x13 && v189 / v223 < 10.0) || v232 || (v165 > 0.35 && v166 < 0.65) {
            *bad = true;
        }
    }

    /// `offsetBeatPosition(half_window, offset, list)` (0x10113d304).
    fn offset_beat_position(&mut self, a2: i32, a3: i32, list: &mut Vec<BeatInfo>) {
        if a2 < 0 {
            return;
        }
        let v16 = self.env.sr4;
        let v17 = 2 * ((v16 as f64 * 0.05) as i32);
        if v17 < a3 || a3 < 0 || v17 < a2 || list.is_empty() {
            return;
        }
        let v22 = ((v16 as f32) * ((0.004 * list[0].interval as f64) as f32)) as i32;
        let v23 = 4 * (a3 - a2);
        let total = self.env.total;
        let mut neg = 0usize;
        let mut past = 0usize;
        for b in list.iter_mut() {
            b.pos += v23;
            if b.pos < 0 {
                neg += 1;
            }
            if b.pos.wrapping_add(v23) > total {
                past += 1;
            }
        }
        if neg != 0 {
            list.drain(0..neg.min(list.len()));
        }
        if past != 0 {
            let keep = list.len().saturating_sub(past);
            list.truncate(keep);
        }
        if list.is_empty() {
            self.count = 0;
            return;
        }
        if list[0].pos >= v22 {
            loop {
                let first = list[0];
                list.insert(0, first);
                let num = if list[0].num == 1 { 4 } else { list[0].num - 1 };
                let p = list[0].pos - v22;
                list[0].pos = p;
                list[0].num = num;
                list[0].flag = false;
                if p < v22 {
                    break;
                }
            }
        }
        let last = *list.last().unwrap();
        if last.pos + v22 <= total {
            loop {
                let l = *list.last().unwrap();
                list.push(l);
                let e = list.last_mut().unwrap();
                e.pos += v22;
                let next = e.pos + v22;
                e.num = if e.num == 4 { 1 } else { e.num + 1 };
                e.flag = false;
                if next > total {
                    break;
                }
            }
        }
        self.count = list.len() as i32;
    }
}

/// The samples `Analyzer::setSample` gives the envelope units: float scaled to 16-bit range,
/// truncated to an integer and narrowed without saturation.
pub fn to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|&v| super::super::rb_beat1::reader_sample(v) as i16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biquad_dc_gain_and_rejection() {
        let mut lp = Biquad::low_pass(100.0, 11025.0);
        let mut y = 0.0;
        for _ in 0..20000 {
            y = lp.run(1.0);
        }
        assert!((y - 1.0).abs() < 1e-9, "{y}");
        let mut hp = Biquad::high_pass(2000.0, 11025.0);
        for _ in 0..20000 {
            y = hp.run(1.0);
        }
        assert!(y.abs() < 1e-9, "{y}");
    }

    /// Envelopes and integrals against rekordbox's own (`rbhp` with RB_DUMPENV):
    /// PREPDECK_HP_ENV = dump, PREPDECK_HP_PCM = the stereo f32 PCM it was made from.
    #[test]
    #[ignore]
    fn envelopes_against_oracle() {
        let (Ok(envp), Ok(pcmp)) = (std::env::var("PREPDECK_HP_ENV"), std::env::var("PREPDECK_HP_PCM")) else { return };
        let e = std::fs::read(envp).unwrap();
        let cnt = i32::from_le_bytes(e[0..4].try_into().unwrap()) as usize;
        let want: Vec<i16> = e[8..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        let pcm: Vec<f32> = std::fs::read(pcmp).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let sr: i32 = std::env::var("PREPDECK_HP_SR").ok().and_then(|v| v.parse().ok()).unwrap_or(44100);
        let u = Uba::new(sr, &to_i16(&pcm));
        eprintln!("count mine {} oracle {cnt}", u.high.len());
        for (b, (name, mine)) in [("high", &u.high), ("low", &u.low), ("all", &u.all)].iter().enumerate() {
            let w = &want[b * cnt..(b + 1) * cnt];
            let diff = mine.iter().zip(w).filter(|(a, b)| a != b).count();
            let first = mine.iter().zip(w).position(|(a, b)| a != b);
            eprintln!("{name}: {diff} of {cnt} differ, first {first:?}");
        }
    }
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    /// `UnitBeatAdjust::analyze` against rekordbox's own run on the same list (rbhp with
    /// RB_UBATEST): PREPDECK_HP_PCM, PREPDECK_HP_LEG = "L pos num bpm" lines (input),
    /// PREPDECK_HP_UBA = rekordbox's result.
    #[test]
    #[ignore]
    fn uba_analyze_against_oracle() {
        let (Ok(pcmp), Ok(legp), Ok(ubap)) = (std::env::var("PREPDECK_HP_PCM"), std::env::var("PREPDECK_HP_LEG"), std::env::var("PREPDECK_HP_UBA")) else { return };
        let pcm: Vec<f32> = std::fs::read(pcmp).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let sr: i32 = std::env::var("PREPDECK_HP_SR").ok().and_then(|v| v.parse().ok()).unwrap_or(44100);
        let leg = std::fs::read_to_string(legp).unwrap();
        let mut bpm100 = 0;
        let mut list = vec![];
        for l in leg.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            if w.len() >= 4 && w[0] == "L" && w[1] == "bpm" {
                bpm100 = w[2].parse().unwrap();
            } else if w.len() == 4 && w[0] == "L" {
                list.push((w[1].parse::<i32>().unwrap(), w[2].parse::<i32>().unwrap()));
            }
        }
        let n: usize = std::env::var("PREPDECK_HP_UBAN").ok().and_then(|v| v.parse().ok()).unwrap_or(list.len()).min(list.len());
        let mut bpm = bpm100 as f32 / 100.0;
        let mut v: Vec<BeatInfo> = list[..n].iter().map(|&(p, num)| BeatInfo::new(p, 60000.0f32 / bpm, num)).collect();
        let env = Uba::new(sr, &to_i16(&pcm));
        let t0 = std::time::Instant::now();
        let mut r = UbaRefine::new(&env);
        r.analyze(&mut bpm, &mut v);
        eprintln!("mine: bpm {bpm} n {} first {:?} time {:?}", v.len(), v.first(), t0.elapsed());
        let want = std::fs::read_to_string(ubap).unwrap();
        let mut w = vec![];
        let mut wbpm = 0f32;
        for l in want.lines() {
            let x: Vec<&str> = l.split_whitespace().collect();
            if x.len() >= 3 && x[1] == "bpm" {
                wbpm = x[2].parse().unwrap();
            } else if x.len() == 5 {
                w.push((x[1].parse::<i32>().unwrap(), x[2].parse::<f32>().unwrap(), x[3].parse::<i32>().unwrap()));
            }
        }
        let same = v.len() == w.len() && v.iter().zip(&w).all(|(a, b)| (a.pos, a.interval, a.num) == *b);
        let first = v.iter().zip(&w).position(|(a, b)| (a.pos, a.interval, a.num) != *b);
        eprintln!("oracle: bpm {wbpm} n {} first {:?}; identical {same}, first difference {:?}", w.len(), w.first(), first.map(|i| (i, v[i], w[i])));
    }
}
