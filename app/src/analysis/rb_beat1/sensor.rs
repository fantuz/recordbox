//! Stage 1 of rekordbox's v1 beat library (`BeatAnalyzer_1_0::peakss::BeatSensor`): six band
//! filters, a rectified and decimated envelope per band, peak picking with a decaying threshold,
//! a tempo guess per band from the recent peak intervals (`PeakBuffer::calcBeat`), the choice of
//! the most consistent band (`selectBand`), and the raw beat-candidate list (`changeStat`).
//!
//! Ported from rekordbox 7.2.19 (arm64 slice): `BeatSensor::create` 0x10118f508, `reset`
//! 0x10118fc54, `put` 0x101137234, `selectBand` 0x1011378d8, `PeakBuffer::calcBeat` 0x10118ef24,
//! `calcBeat_s` 0x10118f010, `BeatListSignal::changeStat` 0x101136ba0, `IirPassFilter`
//! 0x10118b744..0x10118c074, `balib::IirFilter::operate` 0x10118b5e8. Every multiply-add that
//! the original fuses is fused here (`mul_add`), and float/double widths follow the original,
//! so the output is bit-identical on the same input.

use super::BeatInfo;

/// `BeatRangeV::s_BaseTbl` (0x1037c86c0): interval windows (ms) and the beat multiple each
/// window stands for.
const BASE_TBL: [(f32, f32, f32); 9] = [
    (500.0, 1276.0, 1.5),
    (468.0, 1071.0, 1.25),
    (923.0, 2142.0, 2.5),
    (290.0, 638.0, 0.75),
    (320.0, 860.0, 1.0),
    (290.0, 428.0, 0.5),
    (666.0, 1714.0, 2.0),
    (1500.0, 3333.0, 4.0),
    (1125.0, 2580.0, 3.0),
];
/// `BeatRangeV::s_CompTbl` (0x1037c872c): the same windows in the order used when comparing
/// earlier intervals.
const COMP_TBL: [(f32, f32, f32); 9] = [
    (1500.0, 3333.0, 4.0),
    (923.0, 2142.0, 2.5),
    (666.0, 1714.0, 2.0),
    (500.0, 1276.0, 1.5),
    (468.0, 1071.0, 1.25),
    (320.0, 860.0, 1.0),
    (290.0, 638.0, 0.75),
    (290.0, 428.0, 0.5),
    (1125.0, 2580.0, 3.0),
];

/// 2π as the original has it (0x401921fb54442d11, i.e. 2 × 3.14159265358979).
const TWO_PI: f64 = 6.28318530717958;

/// `(float)fmin((double)x * 0.04, 20.0)`: the tolerance the original uses everywhere.
#[inline]
pub(super) fn tol(x: f32) -> f32 {
    ((x as f64) * 0.04).min(20.0) as f32
}

#[repr(C)]
struct SinCos {
    sin: f64,
    cos: f64,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn __sincos_stret(x: f64) -> SinCos;
}

/// The original calls Apple's `__sincos_stret`; use the same routine where it exists.
fn sincos(x: f64) -> (f64, f64) {
    #[cfg(target_os = "macos")]
    {
        let r = unsafe { __sincos_stret(x) };
        (r.sin, r.cos)
    }
    #[cfg(not(target_os = "macos"))]
    {
        (x.sin(), x.cos())
    }
}

/// One second-order section (`balib::IirFilter`, 88 bytes): b0 b1 b2, a1 a2 and the state.
#[derive(Clone, Copy, Default)]
pub(super) struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x2: f64,
    x1: f64,
    y2: f64,
    y1: f64,
}

impl Biquad {
    fn lowpass(f: f32, sr: f32, q: f32) -> Biquad {
        let (s, c) = sincos((f as f64) * TWO_PI / (sr as f64));
        let alpha = s / ((q as f64) + (q as f64));
        let n = 1.0 / (alpha + 1.0);
        let b0 = (-c).mul_add(0.5, 0.5) * n;
        Biquad { b0, b1: (-c).mul_add(n, n), b2: b0, a1: (c * -2.0) * n, a2: (-alpha).mul_add(n, n), ..Default::default() }
    }
    fn highpass(f: f32, sr: f32, q: f32) -> Biquad {
        let (s, c) = sincos((f as f64) * TWO_PI / (sr as f64));
        let alpha = s / ((q as f64) + (q as f64));
        let n = 1.0 / (alpha + 1.0);
        let b0 = c.mul_add(0.5, 0.5) * n;
        Biquad { b0, b1: -(c + 1.0) * n, b2: b0, a1: (c * -2.0) * n, a2: (-alpha).mul_add(n, n), ..Default::default() }
    }
    /// `createBandPathFilterW`: band-pass with the bandwidth in octaves.
    fn bandpass_w(f: f32, sr: f32, bw: f32) -> Biquad {
        let w = (f as f64) * TWO_PI / (sr as f64);
        let (s, c) = sincos(w);
        let sh = ((bw as f64) * 0.34657359027997264 * w / s).sinh();
        let alpha = sh * s;
        let neg = -(sh * s);
        let n = 1.0 / (alpha + 1.0);
        Biquad { b0: alpha * n, b1: 0.0, b2: neg * n, a1: (c * -2.0) * n, a2: (-alpha).mul_add(n, n), ..Default::default() }
    }
    /// `LowPathFilter2D::create(f, rate)` (0x10118c0a8): Q = 1/sqrt(2) as a double.
    pub(super) fn lowpass2d(f: f64, rate: f64) -> Biquad {
        let (s, c) = sincos(f * TWO_PI / rate);
        let alpha = s * f64::from_bits(0x3fe6_a09e_667f_3bcd);
        let n = 1.0 / (alpha + 1.0);
        let b0 = (-c).mul_add(0.5, 0.5) * n;
        Biquad { b0, b1: (-c).mul_add(n, n), b2: b0, a1: (c * -2.0) * n, a2: (-alpha).mul_add(n, n), ..Default::default() }
    }
    /// `HighPathFilter2D::create(f, rate)` (0x10118c2cc).
    pub(super) fn highpass2d(f: f64, rate: f64) -> Biquad {
        let (s, c) = sincos(f * TWO_PI / rate);
        let alpha = s * f64::from_bits(0x3fe6_a09e_667f_3bcd);
        let n = 1.0 / (alpha + 1.0);
        let b0 = c.mul_add(0.5, 0.5) * n;
        Biquad { b0, b1: -(c + 1.0) * n, b2: b0, a1: (c * -2.0) * n, a2: (-alpha).mul_add(n, n), ..Default::default() }
    }
    #[inline]
    pub(super) fn step(&mut self, x: f64) -> f64 {
        let y = self.x2.mul_add(self.b2, self.x1.mul_add(self.b1, x * self.b0)) - self.y2.mul_add(self.a2, self.y1 * self.a1);
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// `HighPathFilter1D` (0x10118c4f4 / 0x10118c540): first-order high-pass.
#[derive(Clone, Copy, Default)]
pub(super) struct Hp1d {
    b0: f64,
    b1: f64,
    a1: f64,
    x1: f64,
    y1: f64,
}

impl Hp1d {
    pub(super) fn new(f: f64, rate: f64) -> Hp1d {
        let w = f * TWO_PI / rate;
        let d2 = w + 2.0;
        let b0 = 2.0 / d2;
        Hp1d { b0, b1: -b0, a1: (w + -2.0) / d2, x1: 0.0, y1: 0.0 }
    }
    #[inline]
    pub(super) fn step(&mut self, x: f64) -> f64 {
        let y = (-self.y1).mul_add(self.a1, self.x1.mul_add(self.b1, x * self.b0));
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Moving-average removal used on two derived bands (`+132` flag of the band state).
struct MovAvg {
    buf: Vec<f32>,
    idx: usize,
    sum: f32,
}

/// Envelope follower and peak picker of one band (160-byte block at BeatSensor+32; the
/// original's field offsets are given in the comments).
#[derive(Default)]
struct Env {
    state: i32,     // +0
    min_gap: u64,   // +8   sr * 0.29
    thr: f32,       // +16
    decay_per: f32, // +20
    decay: f32,     // +24
    thr_floor: f32, // +28  0.001
    phase: i32,     // +32
    hold: i32,      // +36
    k_peak: f32,    // +40  0.75
    k_dec1: f32,    // +44  0.5
    k_dec2: f32,    // +48  0.5
    hold0: u64,     // +56  sr * 1.7
    len1: u64,      // +64  sr * 1.3
    len2: u64,      // +72  sr * 1.3
    t_ref: u64,     // +80
    pos: u64,       // +88
    step: i32,      // +96
    peak_val: f32,  // +100
    peak_pos: u64,  // +104
    peak_pos2: u64, // +112
    sr: u64,        // +120
}

/// `PeakInfo` (12 bytes) in the band's ring of recent peaks.
#[derive(Clone, Copy, Default)]
struct PeakInfo {
    pos: u32,
    /// Stored as the original stores it; nothing reads it back.
    #[allow(dead_code)]
    val: f32,
    iv: f32,
}

/// `PeakBuffer` (80 bytes at BeatSensor+40).
#[derive(Default)]
pub(super) struct PeakBuf {
    band: i32,        // +0
    iv: f32,          // +4   interval (ms) of the band's tempo
    ring: Vec<PeakInfo>, // +8
    head: i32,        // +16
    count: i32,       // +20
    cap: i32,         // +24
    last_pos: u64,    // +32
    valid: bool,      // +57
    dev: f32,         // +60
    cnt: i32,         // +64
    val: f32,         // +68
    vpos: u64,        // +72
}

impl PeakBuf {
    /// `PeakBuffer::calcBeat` (0x10118ef24): tries every interval window and keeps the one
    /// that explains the most earlier peaks (at most 180 BPM).
    fn calc_beat(&mut self, f: f32) -> bool {
        let mut avg = 0f32;
        let mut dev;
        let mut cnt;
        let mut best = 0i32;
        let mut found = false;
        dev = f32::MAX;
        cnt = 0;
        let mut r = self.calc_beat_s(f, 4.0, 0, &mut avg, &mut dev, &mut cnt);
        while r >= 0 {
            if avg >= 333.0 && best < cnt {
                self.iv = avg;
                self.dev = dev;
                self.cnt = cnt;
                found = true;
                best = cnt;
            }
            cnt = 0;
            dev = f32::MAX;
            r = self.calc_beat_s(f, 4.0, r, &mut avg, &mut dev, &mut cnt);
        }
        found
    }

    #[inline]
    fn at(&self, i: i32) -> &PeakInfo {
        &self.ring[i as usize]
    }

    /// `PeakBuffer::calcBeat_s` (0x10118f010). Returns the next window to try, or -1.
    fn calc_beat_s(&self, f: f32, a8: f32, start: i32, out_avg: &mut f32, out_dev: &mut f32, out_cnt: &mut i32) -> i32 {
        if start > 8 {
            return -1;
        }
        let base_row = |from: i32| -> Option<i32> { (from..9).find(|&j| BASE_TBL[j as usize].0 < f && BASE_TBL[j as usize].1 > f) };
        let Some(mut j) = base_row(start) else { return -1 };
        let s3 = a8 + a8;
        let d4 = (a8 as f64) * 4.0;
        let cap = self.cap;
        let w14 = cap + self.head;
        let w15 = w14 - 2;
        loop {
            let mut ratio = BASE_TBL[j as usize].2;
            let base = f / ratio;
            let t = tol(base);
            let mut target = if ratio < a8 { a8 } else { s3 };
            let result = j + 1;
            let mut sum = f;
            let mut w6: i32 = 0;
            'l15: loop {
                let mut w2 = w15 - w6;
                'l16: loop {
                    let iv = self.at((w14 + !w6) % cap).iv;
                    let mut k: i32 = 0;
                    loop {
                        if k > 8 {
                            break 'l15;
                        }
                        let Some(row) = (k..9).find(|&r| COMP_TBL[r as usize].0 < iv && COMP_TBL[r as usize].1 > iv) else { break 'l15 };
                        k = row + 1;
                        let cr = COMP_TBL[row as usize].2;
                        if (iv / cr - base).abs() >= t {
                            continue;
                        }
                        sum = iv + sum;
                        ratio = cr + ratio;
                        if ratio == target {
                            // L_found
                            let a = sum / target;
                            *out_avg = a;
                            *out_dev = (a - base).abs();
                            let mut w11 = w6 + 1;
                            let t2 = tol(a);
                            let inv = 1.0 / a;
                            loop {
                                let x = self.at(w2 % cap).iv;
                                let mut q = (x * 4.0).mul_add(inv, 0.5).trunc() * 0.25;
                                let qr = (q + 0.5).trunc();
                                if q > 2.5 {
                                    q = qr;
                                }
                                q = q.min(4.0);
                                if (x / q - a).abs() >= t2 {
                                    *out_cnt = w11;
                                    return result;
                                }
                                let w13 = w11 + 1;
                                if w11 > 28 {
                                    *out_cnt = w13;
                                    return result;
                                }
                                let w14b = w11 + 2;
                                w2 -= 1;
                                w11 = w13;
                                if w14b >= self.count {
                                    *out_cnt = w13;
                                    return result;
                                }
                            }
                        }
                        if ratio > target {
                            if !(target > a8) || !(d4 > target as f64) || !(d4 > ratio as f64) {
                                break 'l15;
                            }
                            if w6 + 2 >= self.count {
                                break 'l15;
                            }
                            target = target + target;
                            let lt = w6 < 29;
                            w6 += 1;
                            if lt {
                                continue 'l15;
                            }
                            break 'l15;
                        }
                        if ratio >= target {
                            break 'l15;
                        }
                        if w6 > 28 {
                            break 'l15;
                        }
                        let w7 = w6 + 2;
                        w6 += 1;
                        w2 -= 1;
                        if w7 < self.count {
                            continue 'l16;
                        }
                        break 'l15;
                    }
                }
            }
            // L_35: next base window.
            if j >= 8 {
                return -1;
            }
            let _ = result;
            match base_row(j + 1) {
                Some(r) => j = r,
                None => return -1,
            }
        }
    }
}

/// One of the six bands: its filter chain (or the band it copies from), the moving-average
/// removal, the decimation accumulator, the envelope follower and the peak buffer.
struct Band {
    filt: Vec<Biquad>,
    src: i32,
    ma: Option<MovAvg>,
    buf: Vec<f32>,
    acc: f32,
    env: Env,
    pk: PeakBuf,
}

pub(super) struct Sensor {
    bands: Vec<Band>,
    decim: i32,   // +56
    dcount: i32,  // +60
    count: i32,   // +104
    start: i32,   // +108
    total: i32,   // +112
    hist: [i32; 256], // +116
    hmin: i32,    // +1140
    hmax: i32,    // +1144
    block: usize, // +100: sr / 75
    stat: i32,    // BeatListMaker+28
    pub(super) list: Vec<BeatInfo>,
    /// Debug trace of every `calcBeat` call (same format as the oracle harness's
    /// `RB_LOG_CALCBEAT`).
    pub(super) log: Option<String>,
}

impl Sensor {
    /// `BeatSensor::create(sr, 0, total, 8, 6)` followed by `reset`.
    pub(super) fn new(sr: i32, total: i32) -> Sensor {
        let srf = sr as f32;
        let block = (sr / 75) as usize;
        let mk_env = |step: i32| Env {
            min_gap: (((sr as u64) as f32) * f32::from_bits(0x3e94_7ae2)) as u64,
            thr: 0.001,
            thr_floor: 0.001,
            k_peak: 0.75,
            k_dec1: 0.5,
            k_dec2: 0.5,
            hold0: (srf * 1.7f32) as u64,
            len1: (srf * 1.3f32) as u64,
            len2: (srf * 1.3f32) as u64,
            step,
            sr: sr as u64,
            ..Default::default()
        };
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let filters: [Vec<Biquad>; 6] = [
            vec![Biquad::lowpass(283.0, srf, q); 2],
            vec![Biquad::bandpass_w(150.0, srf, 1.5); 2],
            vec![Biquad::highpass(283.0, srf, q), Biquad::lowpass(7900.0, srf, q), Biquad::highpass(283.0, srf, q), Biquad::lowpass(7900.0, srf, q)],
            vec![Biquad::highpass(7900.0, srf, q); 2],
            vec![],
            vec![],
        ];
        let ma_len = |c: f32| (((sr as u64) as f32).mul_add(c, 0.5) as i32).max(2) as usize;
        let mut bands = vec![];
        for (k, filt) in filters.into_iter().enumerate() {
            let (src, ma) = match k {
                4 => (0, Some(ma_len(f32::from_bits(0x3a83_126f)))),
                5 => (3, Some(ma_len(f32::from_bits(0x3951_b718)))),
                _ => (-1, None),
            };
            bands.push(Band {
                filt,
                src,
                ma: ma.map(|n| MovAvg { buf: vec![0.0; n], idx: 0, sum: 0.0 }),
                buf: vec![0.0; block],
                acc: 0.0,
                env: mk_env(8),
                pk: PeakBuf { band: k as i32, ring: vec![PeakInfo::default(); 32], cap: 32, dev: f32::MAX, ..Default::default() },
            });
        }
        Sensor {
            bands,
            decim: 8,
            dcount: 0,
            count: 0,
            start: 0,
            total,
            hist: [0; 256],
            hmin: 200,
            hmax: 40,
            block,
            stat: 0,
            list: vec![],
            log: None,
        }
    }

    /// `BeatSensor::put(buf, n, nullptr)` (0x101137234).
    pub(super) fn put(&mut self, mut input: &[f32]) {
        let mut left = input.len() as i32;
        while left > 0 {
            if self.count >= self.total {
                return;
            }
            let n = (self.block as i32).min(left) as usize;
            self.filter_block(&input[..n]);
            for k in 0..self.bands.len() {
                let src = self.bands[k].src;
                if src >= 0 {
                    let (a, b) = self.bands.split_at_mut(k);
                    b[0].buf[..n].copy_from_slice(&a[src as usize].buf[..n]);
                }
                let band = &mut self.bands[k];
                if let Some(ma) = band.ma.as_mut() {
                    let len = ma.buf.len();
                    let inv = 1.0f32 / (len as i32) as f32;
                    for x in band.buf[..n].iter_mut() {
                        let v = *x;
                        let s = ma.sum + (v - ma.buf[ma.idx]);
                        ma.sum = s;
                        ma.buf[ma.idx] = v;
                        ma.idx += 1;
                        if ma.idx == len {
                            ma.idx = 0;
                        }
                        *x = (-s).mul_add(inv, v).max(0.0);
                    }
                }
            }
            left -= n as i32;
            for i in 0..n {
                self.count += 1;
                if self.count < self.start {
                    continue;
                }
                if self.count >= self.total {
                    break;
                }
                for b in self.bands.iter_mut() {
                    b.acc = b.acc + b.buf[i];
                }
                self.dcount += 1;
                if self.dcount >= self.decim {
                    self.dcount = 0;
                    self.decimated_step();
                }
            }
            input = &input[n..];
        }
    }

    /// Bands 0-3: the input through each band's filter chain, |y| as f32. The original runs
    /// each section over the whole block in turn; here all ten sections advance one sample at a
    /// time so their recurrences overlap. Every section does the same arithmetic in the same
    /// order (`Biquad::step`), so the output is identical.
    #[inline(never)]
    fn filter_block(&mut self, input: &[f32]) {
        let n = input.len();
        let mut s = [Biquad::default(); 10];
        let mut j = 0;
        for b in &self.bands[..4] {
            for st in &b.filt {
                s[j] = *st;
                j += 1;
            }
        }
        let [b0, b1, b2, b3, ..] = &mut self.bands[..] else { unreachable!() };
        debug_assert!(j == 10 && b0.filt.len() == 2 && b1.filt.len() == 2 && b2.filt.len() == 4);
        let (o0, o1, o2, o3) = (&mut b0.buf[..n], &mut b1.buf[..n], &mut b2.buf[..n], &mut b3.buf[..n]);
        for i in 0..n {
            let x = input[i] as f64;
            let a = s[0].step(x);
            let a = s[1].step(a);
            let b = s[2].step(x);
            let b = s[3].step(b);
            let c = s[4].step(x);
            let c = s[5].step(c);
            let c = s[6].step(c);
            let c = s[7].step(c);
            let d = s[8].step(x);
            let d = s[9].step(d);
            o0[i] = (a as f32).abs();
            o1[i] = (b as f32).abs();
            o2[i] = (c as f32).abs();
            o3[i] = (d as f32).abs();
        }
        let mut j = 0;
        for b in [b0, b1, b2, b3] {
            for st in b.filt.iter_mut() {
                *st = s[j];
                j += 1;
            }
        }
    }

    fn decimated_step(&mut self) {
        let decim = self.decim as f32;
        let log = &mut self.log;
        for b in self.bands.iter_mut() {
            let v = b.acc / decim;
            b.acc = 0.0;
            b.pk.valid = false;
            let e = &mut b.env;
            let pos = e.pos.wrapping_add(e.step as i64 as u64);
            e.pos = pos;
            let mut thr = e.thr - e.decay;
            e.thr = thr;
            match e.state {
                0 => {
                    if !(thr >= e.thr_floor) {
                        e.thr = e.thr_floor;
                        thr = e.thr_floor;
                    }
                    if e.hold < (pos as i32).wrapping_sub(e.t_ref as i32) {
                        let mut go = true;
                        let (len, target) = if e.phase == 1 {
                            e.hold = e.len2 as i32;
                            (e.len2, e.k_dec2 * thr)
                        } else if e.phase == 0 {
                            e.phase = 1;
                            e.hold = e.len1 as i32;
                            (e.len1, e.k_dec1 * e.peak_val)
                        } else {
                            go = false;
                            (0, 0.0)
                        };
                        if go {
                            let d = (thr - target) / (len as f32);
                            e.decay_per = d;
                            e.decay = d * (e.step as f32);
                            e.t_ref = pos;
                        }
                    }
                    if !(thr > v) {
                        e.peak_pos = pos;
                        e.peak_pos2 = pos;
                        e.peak_val = v;
                        e.thr = e.k_peak * v;
                        e.hold = e.hold0 as i32;
                        e.phase = 0;
                        e.decay_per = 0.0;
                        e.decay = 0.0;
                        e.t_ref = pos;
                        e.state = 1;
                    }
                }
                1 => {
                    let mut pv = e.peak_val;
                    let confirm;
                    if pv >= v {
                        confirm = pos.wrapping_sub(e.peak_pos2) >= e.min_gap;
                    } else {
                        e.peak_pos = pos;
                        e.peak_pos2 = pos;
                        e.peak_val = v;
                        e.thr = e.k_peak * v;
                        e.phase = 0;
                        e.hold = e.hold0 as i32;
                        e.decay_per = 0.0;
                        e.decay = 0.0;
                        e.t_ref = pos;
                        pv = v;
                        confirm = e.min_gap == 0;
                    }
                    if confirm {
                        e.state = 0;
                        let pk = &mut b.pk;
                        let prev = pk.last_pos;
                        let ppos = e.peak_pos;
                        pk.last_pos = ppos;
                        let (ivbits, val) = if prev != 0 {
                            let iv = ((ppos.wrapping_sub(prev)) as f32 * 1000.0) / (e.sr as f32);
                            pk.iv = iv;
                            pk.valid = pk.calc_beat(iv);
                            if let Some(l) = log.as_mut() {
                                use std::fmt::Write;
                                let _ = writeln!(l, "{} {} {:.9e} {} {:.9e} {:.9e} {}", pk.band, pk.last_pos, iv, pk.valid as i32, pk.iv, pk.dev, pk.cnt);
                            }
                            (iv, e.peak_val)
                        } else {
                            (0.0, pv)
                        };
                        if pk.count < pk.cap {
                            pk.count += 1;
                        }
                        let h = pk.head as usize;
                        pk.ring[h] = PeakInfo { pos: e.peak_pos as u32, val, iv: ivbits };
                        pk.head = (pk.head + 1) % pk.cap;
                        if pk.valid {
                            pk.val = e.peak_val;
                            pk.vpos = e.peak_pos;
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(sel) = self.select_band() {
            let _ = sel;
            for k in 0..self.bands.len() {
                if self.bands[k].pk.valid {
                    let iv = self.bands[k].pk.iv;
                    self.change_stat(iv, k);
                }
            }
        }
    }

    /// `BeatSensor::selectBand` (0x1011378d8): returns the band whose tempo guess is the most
    /// consistent, after checking it against the tempo histogram of the whole track so far.
    fn select_band(&mut self) -> Option<usize> {
        let mut best_dev = f32::MAX;
        let mut res: Option<usize> = None;
        let mut sel_bpm: i32 = 0;
        let mut a2 = 0f32;
        for (k, b) in self.bands.iter().enumerate() {
            if !b.pk.valid {
                continue;
            }
            let iv = b.pk.iv;
            let bpm = (60000.0f32 / iv + 0.5) as i32;
            let dev = b.pk.dev;
            let w = ((4.0f32 / (dev + 1.0)) as i32).max(1);
            self.hist[bpm as usize] += w;
            if self.hmin > bpm {
                self.hmin = bpm;
            }
            if self.hmax < bpm {
                self.hmax = bpm;
            }
            if !(dev >= best_dev) {
                best_dev = dev;
                sel_bpm = bpm;
                a2 = iv;
                res = Some(k);
            }
        }
        let argmax = |h: &[i32; 256], lo: i32, hi: i32| -> i32 {
            let mut m = 0;
            let mut at = -1;
            if lo <= hi {
                for i in lo..=hi {
                    if h[i as usize] > m {
                        m = h[i as usize];
                        at = i;
                    }
                }
            }
            at
        };
        // With no band updated the original still scans the histogram but discards the
        // result (no side effects), so the scan is skipped here.
        let Some(res) = res else {
            return None;
        };
        let mut v14 = argmax(&self.hist, self.hmin, self.hmax);
        let mut v20 = 60000.0f32 / (v14 as f32);
        if v14 < 1 {
            if v14 < 0 {
                return None;
            }
            v14 = 0;
        } else {
            let t = tol(a2);
            if (v20 - a2).abs() > t {
                loop {
                    if !((v20 + v20 - a2).abs() > t) || !((v20 * 0.5 - a2).abs() > t) {
                        break;
                    }
                    if self.hist[sel_bpm as usize] < 3 {
                        return None;
                    }
                    let (lo, hi) = (self.hmin, self.hmax);
                    self.hmin = 200;
                    self.hmax = 40;
                    if lo > hi {
                        return None;
                    }
                    for i in lo..=hi {
                        let x = 60000.0f32 / (i as f32);
                        if (x - a2).abs() > t && (x + x - a2).abs() > t && (x * 0.5 - a2).abs() > t {
                            self.hist[i as usize] = 0;
                        } else if self.hist[i as usize] != 0 {
                            if i < self.hmin {
                                self.hmin = i;
                            }
                            if i > self.hmax {
                                self.hmax = i;
                            }
                        }
                    }
                    if self.hmin > self.hmax {
                        return None;
                    }
                    v14 = argmax(&self.hist, self.hmin, self.hmax);
                    v20 = 60000.0f32 / (v14 as f32);
                    if v14 <= 0 {
                        if v14 < 0 {
                            return None;
                        }
                        v14 = 0;
                        break;
                    }
                    if !((v20 - a2).abs() > t) {
                        break;
                    }
                }
            }
        }
        if self.hist[v14 as usize] < 3 {
            return None;
        }
        for b in self.bands.iter_mut() {
            if b.pk.valid {
                let x = b.pk.iv;
                let t = tol(x);
                if (v20 - x).abs() > t && (v20 + v20 - x).abs() > t && (v20 * 0.5 - x).abs() > t {
                    b.pk.valid = false;
                }
            }
        }
        Some(res)
    }

    /// `BeatListSignal::changeStat(interval, pos, value, pb)` (0x101136ba0) as called through
    /// `BeatListMaker`: merges the band's last `cnt + 2` peaks into the candidate list.
    fn change_stat(&mut self, a2: f32, k: usize) {
        let cnt = self.bands[k].pk.cnt;
        let band = self.bands[k].pk.band;
        self.stat = self.stat.wrapping_add(cnt + 2);
        if cnt >= -1 {
            let v23 = (600000.0f32 / a2 + 0.5) as i32;
            let v25 = 2 * v23;
            let t = tol(a2);
            let inv = 1.0f32 / a2;
            let mut v24 = cnt + 1;
            loop {
                let v29 = v24;
                let pk = &self.bands[k].pk;
                let idx = (pk.head + !v24 + pk.cap) % pk.cap;
                v24 -= 1;
                let e = pk.ring[idx as usize];
                let pos = e.pos as i32;
                let q = (e.iv * 4.0).mul_add(inv, 0.5).trunc();
                let f1 = q * 0.25;
                let n = self.list.len() as i32;
                // Insertion point: after the last entry whose position is <= pos.
                let mut i = n;
                while i >= 1 && self.list[(i - 1) as usize].pos > pos {
                    i -= 1;
                }
                let mut at;
                let mut insert = true;
                if i >= 1 {
                    at = i;
                    let mut j = i - 1;
                    loop {
                        let ex = self.list[j as usize];
                        if ex.pos != pos {
                            at = i;
                            break;
                        }
                        let f = ex.iv;
                        let v48 = (600000.0f32 / f + 0.5) as i32;
                        let same_int = v48 == v23 || 2 * v48 == v23 || v25 == v48;
                        let none_close = (f - a2).abs() > t && (f + f - a2).abs() > t && (f * 0.5 - a2).abs() > t;
                        if same_int || none_close {
                            if (self.stat as u8) > (ex.stat as u8) {
                                self.list.remove(j as usize);
                                at = i - 1;
                            } else {
                                insert = false;
                            }
                            break;
                        }
                        if j == 0 {
                            at = i;
                            break;
                        }
                        j -= 1;
                    }
                } else {
                    at = 0;
                }
                if insert {
                    let item = BeatInfo { pos, f1, iv: a2, stat: self.stat, band, flag: 0 };
                    self.list.insert(at as usize, item);
                }
                if v29 <= 0 {
                    break;
                }
            }
        }
        self.stat = (self.stat.wrapping_add(256)) & !0xff;
    }
}
