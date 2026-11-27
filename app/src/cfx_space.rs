//! Sound Color FX SPACE and GATE COMP, ported from rekordbox 7.2.19.
//!
//! SPACE (`ColorFxSpace` 0x102a3195c, `SpaceCore` 0x102a37150, `SpaceAllPassFilter`,
//! `SpaceCombFilter`): COLOR (an 8-bit value gliding one step per 0.66667 ms) picks a row
//! of 22 core settings (`createParameterTable` 0x102a31b64, written here as the formulas that
//! fill it). The core sums the two channels, scales them by the input gain, low-passes and
//! high-passes the sum (2nd order, Q 0.7071), runs it through four of rekordbox's "all-pass"
//! sections (v = x - 0.8 d, y = v + d), takes the input back out of the third and fourth
//! outputs for right and left, and runs each through a feedback comb with a first-order
//! low-pass in its loop. The effect's mix is dry 1, wet 2 (`calculateMixRatio`). PARAM
//! (`setParameterToCoreModule` 0x102a326ec, which rekordbox's CFX row turns on) sets the two
//! combs' feedback. Left: darker (1000 / 200 Hz), longer sections, input gain 0.5; right:
//! brighter (7000 / 3000 Hz), input gain 0.7.
//!
//! GATE COMP (`ColorFxGateComp` 0x1029ee928, `GateCore`, `CompCore`, `EnvelopeGenerator`):
//! the sound is split at 284 Hz (two 2nd-order sections each way); to the left each band goes
//! through a gate, to the right through a compressor (the low band low-passed again at 284 Hz
//! after it), and the bands are added. Thresholds are relative to each band's recent peak
//! (`EnvelopeGenerator`: peaks of 33.3 ms chunks kept for 24 chunks, falling over a second
//! after 1.2 s). `updateState` 0x1029efc84 gives the settings for COLOR and PARAM,
//! `calculateMixRatio` 0x1029ef030 the dry / wet blend near the centre.

use crate::fx_core::{bq, iir_1st, iir_hpf2, iir_lpf2};

/// Largest sample rate the delay lines are sized for when they are made (longer delays at
/// higher rates are shortened to fit).
const MAX_RATE: f32 = 96000.0;

/// rekordbox's `SmoothShiftInteger`: an 8-bit value moving one step every 0.66667 ms.
#[derive(Clone, Copy)]
struct Shift {
    v: i32,
    acc: f32,
}

impl Shift {
    /// Frames until the next step (or the whole span when it rests).
    fn span(&self, target: i32, rate: f32, left: usize) -> usize {
        if self.v == target {
            return left;
        }
        let per = 0.00066667 * rate;
        (((per - self.acc).ceil()) as usize).clamp(1, left)
    }

    fn advance(&mut self, target: i32, frames: usize, rate: f32) {
        if self.v == target {
            self.acc = 0.0;
            return;
        }
        self.acc += frames as f32;
        let per = 0.00066667 * rate;
        while self.acc >= per && self.v != target {
            self.acc -= per;
            self.v += (target - self.v).signum();
        }
    }
}

/// The 22 settings of `SpaceCore` for COLOR `c` (0..255) and PARAM `p` (0..255), as
/// `createParameterTable` and `setParameterToCoreModule` compute them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceParams {
    /// Input gain (params 0 x 1).
    pub gain: f32,
    /// Low-pass and high-pass of the sum (params 2, 3).
    pub lpf: f32,
    pub hpf: f32,
    /// The four sections' delays in ms (params 4..7); their gains are 1 and -0.8 (8..15).
    pub ap_ms: [f32; 4],
    /// The combs (left, right): delay ms (16, 17), feedback (18, 19), loop low-pass (20, 21).
    pub comb_ms: [f32; 2],
    pub fb: [f32; 2],
    pub lp: [f32; 2],
}

pub fn space_params(c: i32, p: i32, rate: u32) -> SpaceParams {
    let c = c.clamp(0, 255);
    let p = p.clamp(0, 255) as f32;
    let left = c < 124;
    let shape = if c <= 93 {
        1.0
    } else if c <= 124 {
        (124 - c) as f32 * 0.032258
    } else if c < 132 {
        0.0
    } else {
        ((c - 131) as f32 * 0.032258).min(1.0)
    };
    // PARAM's feedback (the effect is made with PARAM on): per side, a straight line to 116,
    // flat to 139, rising slowly to 255; scaled down toward the centre by (distance/116)^2.
    let rise = |p: f32, slope: f32, top: f32, up: f32| if p < 116.0 { p * slope } else if p <= 139.0 { top } else { top + up * (p - 139.0) };
    let (fb_l, fb_r) = if c < 116 {
        let x = c as f32 * 0.0086207;
        let k = 1.0 - x * x;
        (rise(p, 0.0057164, 0.66310, 0.00051724) * k + 0.25505, rise(p, 0.0065454, 0.75926, 0.00086207) * k + 0.11365)
    } else if c <= 139 {
        (0.25505, 0.11365)
    } else {
        let x = (255 - c) as f32 * 0.0086207;
        let k = 1.0 - x * x;
        (rise(p, 0.0058527, 0.67892, 0.00051724) * k + 0.25505, rise(p, 0.0067527, 0.78332, 0.00086207) * k + 0.11365)
    };
    // Loop low-passes: the same for both combs, one value per side and rate.
    let side = match rate {
        48000 => if left { 19000.0 } else { 18000.0 },
        96000 => if left { 15000.0 } else { 23000.0 },
        _ => if left { 19000.0 } else { 15000.0 },
    };
    SpaceParams {
        gain: shape * if left { 0.5 } else { 0.7 },
        lpf: if left { 1000.0 } else { 7000.0 },
        hpf: if left { 200.0 } else { 3000.0 },
        ap_ms: if left { [27.732430, 42.539680, 73.605438, 38.072559] } else { [24.916660, 38.229160, 66.114578, 34.1875] },
        comb_ms: [197.79160, 314.79169],
        fb: [fb_l, fb_r],
        lp: [side; 2],
    }
}

/// A mono delay line read at a whole number of frames (`DelayModule`), which fades out and
/// back in (0.66667 ms each way) when its delay changes.
struct Line {
    buf: Vec<f32>,
    w: usize,
    d: usize,
    want: usize,
    gate: f32,
}

impl Line {
    fn new(max_ms: f32) -> Line {
        Line { buf: vec![0.0; (max_ms * 0.001 * MAX_RATE) as usize + 2], w: 0, d: 1, want: 1, gate: 1.0 }
    }

    fn set_ms(&mut self, ms: f32, rate: f32) {
        self.want = ((ms * 0.001 * rate) as usize).clamp(1, self.buf.len() - 1);
    }

    #[inline]
    fn read(&mut self, step: f32) -> f32 {
        if self.want != self.d {
            self.gate -= step;
            if self.gate <= 0.0 {
                self.gate = 0.0;
                self.d = self.want;
            }
        } else if self.gate < 1.0 {
            self.gate = (self.gate + step).min(1.0);
        }
        let n = self.buf.len();
        self.buf[(self.w + n - self.d) % n] * self.gate
    }

    #[inline]
    fn write(&mut self, v: f32) {
        self.buf[self.w] = v;
        self.w = (self.w + 1) % self.buf.len();
    }

    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
        self.d = self.want;
        self.gate = 1.0;
    }
}

/// CFX SPACE.
pub struct Space {
    color: Shift,
    gain: f32,
    lpf: [f32; 5],
    hpf: [f32; 5],
    made: (f32, f32, f32, f32, u32),
    zl: [f32; 2],
    zh: [f32; 2],
    ap: [Line; 4],
    comb: [Line; 2],
    lp: [[f32; 3]; 2],
    lpz: [f32; 2],
    /// The wet sound's level lately (to know when the tail has died away).
    tail: f32,
}

impl Space {
    pub fn new() -> Space {
        Space {
            color: Shift { v: 127, acc: 0.0 },
            gain: 0.0,
            lpf: [0.0; 5],
            hpf: [0.0; 5],
            made: (0.0, 0.0, 0.0, 0.0, 0),
            zl: [0.0; 2],
            zh: [0.0; 2],
            ap: [Line::new(80.0), Line::new(80.0), Line::new(80.0), Line::new(80.0)],
            comb: [Line::new(200.0), Line::new(316.0)],
            lp: [[0.0; 3]; 2],
            lpz: [0.0; 2],
            tail: 0.0,
        }
    }

    /// `onCalledMakeSelected`: everything quiet, COLOR at the knob.
    pub fn reset(&mut self, color: i32) {
        self.color = Shift { v: color.clamp(0, 255), acc: 0.0 };
        self.gain = 0.0;
        self.zl = [0.0; 2];
        self.zh = [0.0; 2];
        self.ap.iter_mut().for_each(Line::clear);
        self.comb.iter_mut().for_each(Line::clear);
        self.lpz = [0.0; 2];
        self.tail = 0.0;
    }

    /// COLOR as it has glided so far.
    pub fn color(&self) -> i32 {
        self.color.v
    }

    /// The tail is still heard (the effect stays in the path at the centre until it is gone).
    pub fn ringing(&self) -> bool {
        self.tail > 1e-6
    }

    /// `buf` (stereo interleaved) in place: dry + 2 x the core's output.
    pub fn process(&mut self, buf: &mut [f32], color: i32, param: i32, rate: u32) {
        let r = rate as f32;
        let frames = buf.len() / 2;
        let gstep = 1.0 / (0.00066667 * r);
        let mut n = 0;
        while n < frames {
            let len = self.color.span(color, r, frames - n);
            let sp = space_params(self.color.v, param, rate);
            if self.made != (sp.lpf, sp.hpf, sp.lp[0], sp.lp[1], rate) {
                if (self.made.2, self.made.3) != (sp.lp[0], sp.lp[1]) {
                    // (`setLPFCutoffFrequency(.., true)` clears the loop filters.)
                    self.lpz = [0.0; 2];
                }
                self.made = (sp.lpf, sp.hpf, sp.lp[0], sp.lp[1], rate);
                self.lpf = iir_lpf2(sp.lpf as f64, 0.70710678, r as f64);
                self.hpf = iir_hpf2(sp.hpf as f64, 0.70710678, r as f64);
                for k in 0..2 {
                    self.lp[k] = iir_1st(sp.lp[k].min(0.5 * r) as f64, r as f64, false);
                }
            }
            for (k, a) in self.ap.iter_mut().enumerate() {
                a.set_ms(sp.ap_ms[k], r);
            }
            for (k, cb) in self.comb.iter_mut().enumerate() {
                cb.set_ms(sp.comb_ms[k], r);
            }
            let mut peak = 0.0f32;
            for f in buf[n * 2..(n + len) * 2].chunks_exact_mut(2) {
                // The input gain glides (SmoothFloat, 0.66667 ms).
                self.gain += (sp.gain - self.gain).clamp(-gstep, gstep);
                let mut m = (f[0] + f[1]) * self.gain;
                m = bq(&self.lpf, &mut self.zl, m);
                m = bq(&self.hpf, &mut self.zh, m);
                let mut y = m;
                let mut outs = [0.0f32; 2];
                for (k, a) in self.ap.iter_mut().enumerate() {
                    let d = a.read(gstep);
                    let v = y - 0.8 * d;
                    a.write(v);
                    y = v + d;
                    if k == 2 {
                        outs[1] = y - m;
                    }
                }
                outs[0] = y - m;
                for k in 0..2 {
                    let d = self.comb[k].read(gstep) * sp.fb[k];
                    let x = outs[k] + d;
                    let c = &self.lp[k];
                    let o = c[0] * x + self.lpz[k];
                    self.lpz[k] = c[1] * x - c[2] * o;
                    let o = o.clamp(-8.0, 8.0);
                    self.comb[k].write(o);
                    outs[k] = o;
                }
                peak = peak.max(outs[0].abs()).max(outs[1].abs());
                f[0] += 2.0 * outs[0];
                f[1] += 2.0 * outs[1];
            }
            self.tail = peak.max(self.gain);
            self.color.advance(color, len, r);
            n += len;
        }
    }
}

/// `GainSmoother`: a gain moving in a straight line to its target over a set time.
#[derive(Clone, Copy)]
struct Smoother {
    cur: f32,
    target: f32,
    step: f32,
    left: usize,
    n: usize,
}

impl Smoother {
    fn new(ms: f32, init: f32, rate: f32) -> Smoother {
        Smoother { cur: init, target: init, step: 0.0, left: 0, n: ((ms * 0.001 * rate) as usize).max(1) }
    }

    fn set_target(&mut self, t: f32) {
        if t != self.target {
            self.target = t;
            self.left = self.n;
            self.step = (t - self.cur) / self.n as f32;
        }
    }

    fn set_current(&mut self, v: f32) {
        self.cur = v;
        self.target = v;
        self.left = 0;
    }

    fn advance(&mut self, k: usize) {
        if self.left == 0 {
            return;
        }
        if k >= self.left {
            self.cur = self.target;
            self.left = 0;
        } else {
            self.cur += self.step * k as f32;
            self.left -= k;
        }
    }
}

/// `EnvelopeGenerator`'s level: the peak of the last 24 chunks of 33.3 ms, held at least
/// 1.2 s before it falls (over a second) toward a lower one.
#[derive(Clone, Copy)]
struct Level {
    chunk: usize,
    hold: usize,
    ring: [f32; 24],
    recent_peak: f32,
    ri: usize,
    cmax: f32,
    pos: usize,
    held: usize,
    sm: Smoother,
    /// What the gate's threshold follows (`+372`); the compressor follows `sm.cur`.
    level: f32,
}

impl Level {
    fn new(rate: f32) -> Level {
        Level { chunk: ((0.0333 * rate).round() as usize).max(1), hold: (1.2 * rate).round() as usize, ring: [0.0; 24], recent_peak: 0.0, ri: 0, cmax: 0.0, pos: 0, held: 0, sm: Smoother::new(1000.0, 0.05, rate), level: 0.0 }
    }

    fn decide(&mut self, v6: usize) {
        let cur = self.sm.cur;
        let m = self.recent_peak;
        if m >= cur {
            self.sm.set_current(m);
            self.level = m;
            self.held = 0;
        } else if cur * 0.75 <= m {
            self.held = 0;
            self.level = cur;
        } else {
            if self.held >= self.hold {
                self.sm.set_target(m);
                self.held = 0;
            } else {
                self.held += v6;
            }
            self.level = m;
        }
    }

    /// `createEnvelope` for a block whose per-frame envelope is `env`.
    fn feed(&mut self, env: &[f32]) {
        let mut v6 = env.len();
        let mut at = 0;
        while self.pos + v6 > self.chunk {
            let take = self.chunk - self.pos;
            self.cmax = env[at..at + take].iter().fold(self.cmax, |a, b| a.max(*b));
            self.ring[self.ri] = self.cmax;
            self.recent_peak = self.ring.iter().copied().fold(0.0f32, f32::max);
            self.ri = (self.ri + 1) % 24;
            self.cmax = 0.0;
            self.sm.advance(take);
            self.decide(v6);
            v6 -= take;
            at += take;
            self.pos = 0;
            if v6 == 0 {
                return;
            }
        }
        self.cmax = env[at..at + v6].iter().fold(self.cmax, |a, b| a.max(*b));
        self.sm.advance(v6);
        self.decide(v6);
        self.pos += v6;
    }
}

/// One band's gate (`GateCore`) or compressor (`CompCore`) and the level it follows.
#[derive(Clone, Copy)]
struct Dyn {
    level: Level,
    thr: Smoother,
    f656: f32,
    f660: f32,
}

/// The five settings of a core: threshold share, attack, release, open rate (gate) or
/// make-up (compressor), close rate (gate) or slope (compressor).
pub type Core = [f32; 5];

impl Dyn {
    fn new(rate: f32, ms: f32) -> Dyn {
        Dyn { level: Level::new(rate), thr: Smoother::new(ms, 0.05, rate), f656: 0.0, f660: 0.0 }
    }

    fn gate(&mut self, buf: &mut [f32], env: &[f32], p: &Core) {
        self.level.feed(env);
        self.thr.set_target(self.level.level);
        for (f, &e) in buf.chunks_exact_mut(2).zip(env) {
            let t = p[0] * self.thr.cur;
            self.thr.advance(1);
            self.f656 = if self.f656 >= e { self.f656 * p[2] } else { self.f656 + (e - self.f656) * p[1] };
            self.f660 = if self.f656 <= t { self.f660 * p[4] } else { p[3] - self.f660 * p[3] + self.f660 };
            f[0] *= self.f660;
            f[1] *= self.f660;
        }
        if self.f656 < 1e-10 {
            self.f656 = 0.0;
        }
        if self.f660 < 1e-10 {
            self.f660 = 0.0;
        }
    }

    fn comp(&mut self, buf: &mut [f32], env: &[f32], p: &Core) {
        self.level.feed(env);
        self.thr.set_target(self.level.sm.cur);
        for (f, &e) in buf.chunks_exact_mut(2).zip(env) {
            let t = (p[0] * self.thr.cur).max(1.1921e-7);
            self.thr.advance(1);
            self.f656 = if self.f656 >= e { self.f656 * p[2] } else { self.f656 + (e - self.f656) * p[1] };
            self.f660 = if self.f656 > t { p[3] / ((1.0 - p[4]) + self.f656 * p[4] / t).max(1.1921e-7) } else { p[3] };
            f[0] *= self.f660;
            f[1] *= self.f660;
        }
        if self.f656 < 1.1921e-7 {
            self.f656 = 0.0;
        }
    }
}

/// What `updateState` sets for COLOR `c` and PARAM `p`: 0 nothing, 1 gate, 2 compressor, and
/// the (high band, low band) cores' settings.
pub fn gate_comp_state(c: i32, p: i32) -> (u8, Core, Core) {
    let pf = p as f32;
    if c <= 123 {
        // Gate.
        if c > 107 {
            return (1, [0.33, 1.0, 0.99983, 1.0, 0.99992], [0.24, 1.0, 0.99983, 0.0012215, 0.99992]);
        }
        let v5 = (c as f32 * -0.0093458 + 1.0).max(0.0);
        let a = pf * 0.0056471 + 0.28;
        let v6 = a - ((1.0 - v5) - v5 * (1.0 - v5)) * a;
        let (lo, hi) = if p <= 127 { (pf * 0.0056693 + 0.28, pf * 0.0056693 + 0.28) } else { ((pf - 128.0) * 0.011339 + 0.28, (pf - 128.0) * 0.023622 + 1.0) };
        let high = [v6 * 0.2325 + 0.33, 1.0, 0.99983, 1.0, v5 * v5 * (-0.00032808 * hi) + 0.99992];
        let low = [v6 * 0.36 + 0.24, 1.0, 0.99983, 0.0012215, v5 * v5 * (-0.0013565 * lo) + 0.99992];
        (1, high, low)
    } else if c <= 131 {
        (0, [0.0; 5], [0.0; 5])
    } else if c < 148 {
        (2, [0.4334, 1.0, 0.99932, 1.0, 0.15], [0.54653, 1.0, 0.99919, 1.0, 0.15])
    } else {
        let v15 = ((c - 148) as f32 * 0.0093458).min(1.0);
        let u = 1.0 - v15;
        let v16 = u * u * u * (pf * 0.0078431);
        let slope = v15 * 0.45 + 0.15 + pf * 0.0015686;
        let makeup = v15 * 2.0 + 1.0 + pf * 0.0021495;
        (2, [v16 * 0.32625 + 0.10715, 1.0, 0.99932, makeup, slope], [v16 * 0.41142 + 0.13512, 1.0, 0.99919, makeup, slope])
    }
}

/// `calculateMixRatio`: (dry, wet) for COLOR `c`. The effect comes in over the 30 steps
/// next to the centre on either side.
pub fn gate_comp_mix(c: i32) -> (f32, f32) {
    let v2 = if c < 124 { 1.0 - c as f32 * 0.004065 } else if c < 132 { 0.5 } else { 0.5 + (c - 132) as f32 * 0.004065 };
    let v9 = if 2.0 * v2 - 1.0 >= 0.25 { 0.0 } else { 5.0 - 8.0 * v2 };
    (v9 * v9, 1.0 - v9 * v9)
}

/// CFX GATE COMP.
pub struct GateComp {
    color: Shift,
    rate: u32,
    /// LR crossover at 284 Hz: high (two high-passes), low (two low-passes), and the low
    /// band's extra low-pass in compressor mode.
    hp: [[f32; 5]; 2],
    lp: [[f32; 5]; 2],
    zh: [[[f32; 2]; 2]; 2],
    zl: [[[f32; 2]; 2]; 2],
    zx: [[f32; 2]; 2],
    /// High band and low band: gate and compressor.
    gate: [Dyn; 2],
    comp: [Dyn; 2],
    hi: Vec<f32>,
    lo: Vec<f32>,
    env: Vec<f32>,
    mix: (f32, f32),
    // Decks can call process once per stereo frame. Update the control laws only
    // when the effective 8-bit COLOR or PARAM changes.
    knobs: (i32, i32),
    state: (u8, Core, Core),
    target_mix: (f32, f32),
}

const GC_BLOCK: usize = 512;

impl GateComp {
    pub fn new() -> GateComp {
        let r = 48000.0;
        GateComp { color: Shift { v: 127, acc: 0.0 }, rate: 0, hp: [[0.0; 5]; 2], lp: [[0.0; 5]; 2], zh: Default::default(), zl: Default::default(), zx: Default::default(), gate: [Dyn::new(r, 3.3); 2], comp: [Dyn::new(r, 0.66667); 2], hi: vec![0.0; GC_BLOCK * 2], lo: vec![0.0; GC_BLOCK * 2], env: vec![0.0; GC_BLOCK], mix: (1.0, 0.0), knobs: (-1, -1), state: (0, [0.0; 5], [0.0; 5]), target_mix: (1.0, 0.0) }
    }

    /// `onCalledMakeSelected`: filters and followers cleared, COLOR at the knob.
    pub fn reset(&mut self, color: i32) {
        self.color = Shift { v: color.clamp(0, 255), acc: 0.0 };
        self.zh = Default::default();
        self.zl = Default::default();
        self.zx = Default::default();
        self.rate = 0;
        self.mix = gate_comp_mix(self.color.v);
    }

    /// COLOR as it has glided so far.
    pub fn color(&self) -> i32 {
        self.color.v
    }

    /// The blend has reached the dry sound alone.
    pub fn dry(&self) -> bool {
        self.mix == (1.0, 0.0)
    }

    pub fn process(&mut self, buf: &mut [f32], color: i32, param: i32, rate: u32) {
        let r = rate as f32;
        if rate != self.rate {
            self.rate = rate;
            let c = iir_lpf2(284.0, 0.70710678, r as f64);
            let h = iir_hpf2(284.0, 0.70710678, r as f64);
            self.lp = [c, c];
            self.hp = [h, h];
            self.gate = [Dyn::new(r, 3.3); 2];
            self.comp = [Dyn::new(r, 0.66667); 2];
        }
        let frames = buf.len() / 2;
        let mstep = 1.0 / (0.0033333 * r);
        let mut n = 0;
        while n < frames {
            let len = self.color.span(color, r, (frames - n).min(GC_BLOCK));
            let knobs = (self.color.v, param);
            if self.knobs != knobs {
                self.state = gate_comp_state(knobs.0, knobs.1);
                self.target_mix = gate_comp_mix(knobs.0);
                self.knobs = knobs;
            }
            let (state, high, low) = self.state;
            let (dry, wet) = self.target_mix;
            let x = &mut buf[n * 2..(n + len) * 2];
            // Band split.
            for (i, f) in x.chunks_exact(2).enumerate() {
                for ch in 0..2 {
                    let h = bq(&self.hp[0], &mut self.zh[0][ch], f[ch]);
                    self.hi[i * 2 + ch] = bq(&self.hp[1], &mut self.zh[1][ch], h);
                    let l = bq(&self.lp[0], &mut self.zl[0][ch], f[ch]);
                    self.lo[i * 2 + ch] = bq(&self.lp[1], &mut self.zl[1][ch], l);
                }
            }
            let (hi, lo, env) = (&mut self.hi[..len * 2], &mut self.lo[..len * 2], &mut self.env[..len]);
            let envelope = |b: &[f32], e: &mut [f32]| {
                for (v, f) in e.iter_mut().zip(b.chunks_exact(2)) {
                    *v = f[0].abs().max(f[1].abs());
                }
            };
            match state {
                1 => {
                    envelope(hi, env);
                    self.gate[0].gate(hi, env, &high);
                    envelope(lo, env);
                    self.gate[1].gate(lo, env, &low);
                }
                2 => {
                    envelope(hi, env);
                    self.comp[0].comp(hi, env, &high);
                    envelope(lo, env);
                    self.comp[1].comp(lo, env, &low);
                    for f in lo.chunks_exact_mut(2) {
                        for ch in 0..2 {
                            f[ch] = bq(&self.lp[0], &mut self.zx[ch], f[ch]);
                        }
                    }
                }
                _ => {}
            }
            for (i, f) in x.chunks_exact_mut(2).enumerate() {
                // (EffectBase's dry / wet glide, 3.33 ms.)
                self.mix.0 += (dry - self.mix.0).clamp(-mstep, mstep);
                self.mix.1 += (wet - self.mix.1).clamp(-mstep, mstep);
                for ch in 0..2 {
                    f[ch] = f[ch] * self.mix.0 + (hi[i * 2 + ch] + lo[i * 2 + ch]) * self.mix.1;
                }
            }
            self.color.advance(color, len, r);
            n += len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_gate_laws_match_recalculation_on_deck_and_rack() {
        let mut cached = GateComp::new();
        let mut fresh = GateComp::new();
        for frames in [1, 33, 512] {
            for (c, p, rate) in [(43, 162, 48000), (212, 162, 48000), (212, 13, 48000),
                                  (43, 255, 44100), (212, 153, 96000), (127, 153, 96000)] {
                for repeat in 0..9 {
                    let mut a: Vec<_> = (0..frames * 2).map(|i| ((i + repeat) as f32 * 0.117).sin() * 0.8).collect();
                    let mut b = a.clone();
                    fresh.knobs = (-1, -1);
                    cached.process(&mut a, c, p, rate);
                    fresh.process(&mut b, c, p, rate);
                    assert_eq!(a, b);
                }
            }
        }
    }

    #[test]
    fn space_table_matches_rekordboxs_rows() {
        // Rows read back from rekordbox's `createParameterTable` at 44.1 kHz (PARAM 127.5 is
        // not a row: the knob sends 0..255; here PARAM 0 and 255 at a few COLOR steps).
        let a = space_params(0, 0, 44100);
        assert_eq!((a.gain, a.lpf, a.hpf, a.lp), (0.5, 1000.0, 200.0, [19000.0, 19000.0]));
        let b = space_params(255, 0, 44100);
        assert!((b.gain - 0.7).abs() < 1e-6 && b.lpf == 7000.0 && b.hpf == 3000.0 && b.lp == [15000.0, 15000.0]);
        assert_eq!(space_params(255, 0, 48000).lp, [18000.0, 18000.0]);
        // Input gain: full to 93, falling to 0 at 124, 0 to 131, back to full at 162.
        let g = |c| space_params(c, 0, 44100).gain;
        assert!((g(93) - 0.5).abs() < 1e-6 && (g(109) - 0.5 * 0.48387).abs() < 1e-4 && g(124) == 0.0 && g(131) == 0.0);
        assert!((g(140) - 0.7 * 0.29032).abs() < 1e-4 && (g(162) - 0.7).abs() < 1e-5);
        // Comb feedback with PARAM on: at the ends PARAM's line + 0.25505 / 0.11365; the
        // centre band keeps the base.
        let f = |c, p| space_params(c, p, 44100).fb;
        assert!((f(0, 255)[0] - (0.66310 + 0.00051724 * 116.0 + 0.25505)).abs() < 1e-4);
        assert!((f(255, 100)[1] - (0.0067527 * 100.0 + 0.11365)).abs() < 1e-4);
        assert_eq!(f(128, 200), [0.25505, 0.11365]);
        let mid = f(58, 128);
        assert!((mid[0] - (0.66310 * (1.0 - (58.0f32 / 116.0).powi(2)) + 0.25505)).abs() < 1e-4, "{mid:?}");
    }

    #[test]
    fn space_rings_and_centre_is_dry() {
        let mut s = Space::new();
        let tone: Vec<f32> = (0..24000).flat_map(|i| { let v = (i as f32 * 3000.0 * std::f32::consts::TAU / 48000.0).sin() * 0.3; [v, v] }).collect();
        // Fully right: the reverb adds a lot and stays bounded.
        let mut x = tone.clone();
        for c in x.chunks_mut(1024) {
            s.process(c, 255, 128, 48000);
        }
        let rms = |v: &[f32]| (v.iter().map(|a| a * a).sum::<f32>() / v.len() as f32).sqrt();
        assert!(rms(&x[24000..]) > rms(&tone[24000..]) * 1.2 && x.iter().all(|v| v.abs() < 8.0), "{}", rms(&x[24000..]));
        // Back at the centre with silence in: the tail goes on (rekordbox keeps the wet path).
        let mut quiet = vec![0.0f32; 9600];
        s.process(&mut quiet, 127, 128, 48000);
        assert!(quiet.iter().any(|v| v.abs() > 1e-3) && s.ringing());
        // A fresh one at the centre returns the input exactly.
        let mut s = Space::new();
        let mut y = tone.clone();
        s.process(&mut y, 127, 128, 48000);
        assert_eq!(y, tone);
    }

    #[test]
    fn gate_comp_laws() {
        // Mix: dry at the centre, wet from 30 steps away.
        assert_eq!(gate_comp_mix(127), (1.0, 0.0));
        assert_eq!(gate_comp_mix(0), (0.0, 1.0));
        assert_eq!(gate_comp_mix(255), (0.0, 1.0));
        let (d, w) = gate_comp_mix(110);
        assert!(d > 0.0 && d < 1.0 && (d + w - 1.0).abs() < 1e-6);
        // Gate at the left end, PARAM centre: v6 = 0.28 + 0.0056471 x 128.
        let (st, hi, lo) = gate_comp_state(0, 128);
        let a = 0.28 + 0.0056471 * 128.0;
        assert!(st == 1 && (lo[0] - (a * 0.36 + 0.24)).abs() < 1e-5 && (hi[0] - (a * 0.2325 + 0.33)).abs() < 1e-5);
        assert!((lo[3] - 0.0012215).abs() < 1e-9 && hi[3] == 1.0 && (lo[4] - (0.99992 - 0.0013565 * 0.28)).abs() < 1e-6);
        // Near the centre the gate is fixed; COLOR 132..147 a fixed compressor.
        assert_eq!(gate_comp_state(115, 0).1[0], 0.33);
        assert_eq!(gate_comp_state(140, 0).2[0], 0.54653);
        assert_eq!(gate_comp_state(128, 0).0, 0);
        // Compressor at the right end: threshold share at its floor, make-up 3 + PARAM's.
        let (st, hi, lo) = gate_comp_state(255, 100);
        assert!(st == 2 && (hi[0] - 0.10715).abs() < 1e-6 && (lo[0] - 0.13512).abs() < 1e-6 && (hi[3] - (3.0 + 0.21495)).abs() < 1e-5);
    }

    #[test]
    fn gate_cuts_quiet_parts_and_comp_lifts_them() {
        // Hits: 50 ms at 0.5, then 200 ms at 0.05, over and over. The gate (left end) mutes
        // what falls under its share of the recent peak; the compressor (right end) lifts it.
        let sig: Vec<f32> = (0..96000).flat_map(|i| { let a = if i % 12000 < 2400 { 0.5 } else { 0.05 }; let v = (i as f32 * 1000.0 * std::f32::consts::TAU / 48000.0).sin() * a; [v, v] }).collect();
        let quiet = |x: &[f32]| { let v: Vec<f32> = x.chunks_exact(2).enumerate().filter(|(i, _)| *i >= 48000 && i % 12000 > 7200).map(|(_, f)| f[0]).collect(); (v.iter().map(|a| a * a).sum::<f32>() / v.len() as f32).sqrt() };
        let run = |c: i32| {
            let mut g = GateComp::new();
            g.reset(c);
            let mut x = sig.clone();
            for b in x.chunks_mut(1024) {
                // (PARAM fully right: the gate closes fastest.)
                g.process(b, c, 255, 48000);
            }
            x
        };
        let dry = quiet(&sig);
        let gated = run(0);
        let comped = run(255);
        assert!(quiet(&gated) < dry * 0.1, "gate {} vs {}", quiet(&gated), dry);
        assert!(quiet(&comped) > dry * 1.5, "comp {} vs {}", quiet(&comped), dry);
        assert!(gated.iter().chain(comped.iter()).all(|v| v.is_finite() && v.abs() < 4.0));
        // At the centre: the input exactly.
        let mut g = GateComp::new();
        let mut x = sig[..4096].to_vec();
        g.process(&mut x, 127, 128, 48000);
        assert!(x.iter().zip(&sig[..4096]).all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
