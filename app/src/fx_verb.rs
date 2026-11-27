//! rekordbox's reverb core (`ReverbCore`, 0x102a1b320 ..), as Beat FX REVERB (`BeatFxReverb`
//! 0x102a10d98) and the FX+ REVERB UP / REVERB DOWN (`FxPlusReverbUp` / `Down`) run it.
//!
//! - The input times its gain, then an `IirLpf1stOrder` (14 kHz) and an `IirHpf1stOrder`
//!   (80 Hz); five `ReverbCombFilter`s in parallel, whose left and right lines feed each other
//!   through an `IirLpf1stOrder`; two `ReverbAllPassFilter`s (gain 0.7) in series; the output
//!   gain 0.70795 (`ReverbCore::handleReverbProcessing` 0x102a1cd6c).
//! - Each comb's delay and gain are `SmoothFloat`s: a new value is reached in a straight line
//!   over the smoothing time (3.3333 ms when made). While a delay moves, the comb reads its line
//!   by intervals (`RingBuffer::readByIntervalSamples_mono` 0x102989a98): the read point steps
//!   1 + d(i) - d(i + 1) a frame and 0 after a call's (or piece's) last frame, so it falls one
//!   frame further behind each call until the delay settles and the read point is set again
//!   (`ReverbCombFilter::getFeedbackSignal` 0x102a19e44). A call is cut into pieces no longer
//!   than the shortest delay (`processFilter` 0x102a19a98).
//! - The percent value (REVERB's 1 .. 100; single mode: ROOM SIZE) moves one step at a time
//!   (`handleReverbProcessingCheckingSmoothingPitch` 0x102a1c8d8); each step takes the
//!   smoothing time, max(40 - 0.66667 |target - current|, 20) ms approached by 0.66667 ms a step
//!   (`BeatFxReverb::SmoothingTimeCalculator` 0x102a112dc), and the combs glide over it
//!   (`startPitchSmoothing` 0x102a1c4b0). A new target waits one smoothing time before the
//!   first step.
//! - `reset` (0x102a1ca18, on make-selected and REVERB's first turn-on after it): the filters
//!   cleared, the combs and all-passes on stand-by for 128.625 ms (`setStandByTime`; the combs
//!   give nothing, the all-passes read nothing), the percent jumps to its value.
//! - The line reads interpolate (lerp) except at 44.1 / 48 kHz (`getStaticParameter`).

use crate::fx::{reverb_in_gain, reverb_rt60_ms, reverb_size, REVERB_AP_MS, REVERB_COMB_MS};

/// Line length (frames): rekordbox's rings are 0.2 s; reads reach at most 100 ms (and a few
/// frames of the interval reads' lag) behind, so 5120 frames give the same reads up to 48 kHz.
const LEN: usize = 5120;
const EPS: f32 = 1.1920929e-7;
/// `ReverbCore`'s stand-by time (ms), set in its constructor (0x4300a000).
const STANDBY_MS: f32 = 128.625;

/// rekordbox's `SmoothFloat<float>` read the way `getSmoothValues` 0x10297aaf4 fills a block:
/// the first frame gets the current value, each next one a step more, the target after `n`.
#[derive(Clone, Copy)]
struct Sm {
    cur: f32,
    target: f32,
    step: f32,
    left: i32,
    n: i32,
}

impl Sm {
    fn new(v: f32, rate: f32) -> Sm {
        Sm { cur: v, target: v, step: 0.0, left: 0, n: ((rate * 0.0033333) as i32).max(1) }
    }

    #[inline]
    fn moving(&self) -> bool {
        self.left > 0
    }

    /// `setTargetValue` 0x10297acf8.
    fn set(&mut self, t: f32) {
        if (t - self.target).abs() >= EPS {
            self.target = t;
            self.step = (t - self.cur) / self.n as f32;
            self.left = self.n;
        }
    }

    /// `setSmoothingTime` 0x10297ad38: a moving value starts again from where it is.
    fn set_ms(&mut self, ms: f32, rate: f32) {
        let n = ((ms * 0.001) * rate) as i32;
        if ms <= 0.0 || n < 1 {
            return;
        }
        let moving = self.moving();
        self.n = n;
        if moving {
            let t = self.target;
            self.target = self.cur;
            self.left = 0;
            if (t - self.cur).abs() >= EPS {
                self.target = t;
                self.step = (t - self.cur) / n as f32;
                self.left = n;
            }
        }
    }

    /// This frame's value, then on to the next.
    #[inline]
    fn take(&mut self) -> f32 {
        if self.left > 0 {
            let v = self.cur;
            self.left -= 1;
            self.cur = if self.left == 0 { self.target } else { self.cur + self.step };
            v
        } else {
            self.cur
        }
    }

    /// The value `i` frames on.
    #[inline]
    fn at(&self, i: i32) -> f32 {
        if i < self.left { self.cur + self.step * i as f32 } else { self.target }
    }

    fn advance(&mut self, frames: i32) {
        if self.left > 0 {
            if frames >= self.left {
                self.left = 0;
                self.cur = self.target;
            } else {
                self.cur += self.step * frames as f32;
                self.left -= frames;
            }
        }
    }
}

/// `IirFilter::handleFilterCore1st`: y = b0 x + b1 x1 - a1 y1 (coefficients b0, b1, a1).
#[inline]
fn first(c: &[f32; 3], z: &mut [f32; 2], x: f32) -> f32 {
    let y = c[0] * x + c[1] * z[0] - c[2] * z[1];
    z[0] = x;
    z[1] = y;
    y
}

#[inline]
fn wrap(p: f64) -> f64 {
    let n = LEN as f64;
    if p < 0.0 { p + n } else if p >= n { p - n } else { p }
}

#[inline]
fn read(line: &[f32], p: f64, lerp: bool) -> f32 {
    let i = p as usize % LEN;
    if lerp {
        let j = if i + 1 == LEN { 0 } else { i + 1 };
        line[i] + (line[j] - line[i]) * (p - i as f64) as f32
    } else {
        line[i]
    }
}

/// One `ReverbCombFilter` (two lines, left and right).
#[derive(Clone)]
struct Comb {
    line: Vec<f32>,
    w: usize,
    rp: [f64; 2],
    dly: [Sm; 2],
    gain: [Sm; 2],
    /// The delays as the current call reads them (`processFilter` fills its arrays first).
    arr: [Sm; 2],
    standby: bool,
    sb_count: i32,
    lp: [[f32; 2]; 2],
    /// This call: the delays move, the piece length (0: the call is skipped), frames left in
    /// the call and in the piece, the piece is on stand-by.
    smooth: bool,
    piece: i32,
    call_left: i32,
    piece_left: i32,
    piece_sb: bool,
}

impl Comb {
    fn new(rate: f32, alloc: bool) -> Comb {
        Comb { line: if alloc { vec![0.0; 2 * LEN] } else { Vec::new() }, w: 0, rp: [0.0; 2], dly: [Sm::new(0.0, rate); 2], gain: [Sm::new(0.0, rate); 2], arr: [Sm::new(0.0, rate); 2], standby: false, sb_count: 0, lp: [[0.0; 2]; 2], smooth: false, piece: 0, call_left: 0, piece_left: 0, piece_sb: false }
    }

    fn begin_call(&mut self, m: i32) {
        self.smooth = self.dly[0].moving() || self.dly[1].moving();
        self.arr = self.dly;
        let (a, b) = (&self.arr[0], &self.arr[1]);
        let p = [a.at(0), a.at(m - 1), b.at(0), b.at(m - 1)].iter().map(|&d| d as i32).min().unwrap_or(0);
        self.piece = p;
        self.dly.iter_mut().for_each(|d| d.advance(m));
        self.call_left = m;
        self.piece_left = 0;
    }

    #[inline]
    fn frame(&mut self, x: [f32; 2], loop_c: &[f32; 3], lerp: bool, sb_frames: i32) -> [f32; 2] {
        if self.piece < 1 {
            return [0.0; 2];
        }
        if self.piece_left == 0 {
            let len = self.piece.min(self.call_left);
            self.piece_left = len;
            self.piece_sb = self.standby;
            if self.standby {
                self.sb_count += len;
                if self.sb_count >= sb_frames {
                    self.standby = false;
                }
                // (`getFeedbackSignal` reads the delays' smoothers once more on stand-by.)
                self.dly.iter_mut().for_each(|d| d.advance(len));
            } else if !self.smooth {
                for c in 0..2 {
                    self.rp[c] = wrap(self.w as f64 - self.arr[c].at(0) as f64);
                }
            }
        }
        let last = self.piece_left == 1;
        let mut fb = [0.0f32; 2];
        for c in 0..2 {
            let d = self.arr[c].take();
            if !self.piece_sb {
                fb[c] = read(&self.line[c * LEN..(c + 1) * LEN], self.rp[c], lerp);
                let step = if !self.smooth { 1.0 } else if last { 0.0 } else { 1.0 + d as f64 - self.arr[c].at(0) as f64 };
                self.rp[c] = wrap(self.rp[c] + step);
            }
            fb[c] *= self.gain[c].take();
        }
        let f0 = first(loop_c, &mut self.lp[0], fb[0]);
        let f1 = first(loop_c, &mut self.lp[1], fb[1]);
        self.line[self.w] = x[0] + f1;
        self.line[LEN + self.w] = x[1] + f0;
        self.w = (self.w + 1) % LEN;
        self.piece_left -= 1;
        self.call_left -= 1;
        fb
    }
}

/// One `ReverbAllPassFilter` (two lines): y = d - g x, the line takes x + g y.
#[derive(Clone)]
struct AllPass {
    line: Vec<f32>,
    w: usize,
    rp: [f64; 2],
    d: [f32; 2],
    g: f32,
    standby: bool,
    sb_count: i32,
    call_left: i32,
    piece_left: i32,
    piece_sb: bool,
}

impl AllPass {
    fn new(alloc: bool) -> AllPass {
        AllPass { line: if alloc { vec![0.0; 2 * LEN] } else { Vec::new() }, w: 0, rp: [0.0; 2], d: [1.0; 2], g: 0.7, standby: false, sb_count: 0, call_left: 0, piece_left: 0, piece_sb: false }
    }

    fn set_delay(&mut self, c: usize, frames: f32) {
        self.d[c] = frames.clamp(1.0, (LEN - 4) as f32);
        self.rp[c] = wrap(self.w as f64 - self.d[c] as f64);
    }

    #[inline]
    fn frame(&mut self, x: [f32; 2], lerp: bool, sb_frames: i32) -> [f32; 2] {
        if self.piece_left == 0 {
            let len = (self.d[0] as i32).min(self.d[1] as i32).max(1).min(self.call_left.max(1));
            self.piece_left = len;
            self.piece_sb = self.standby;
            if self.standby {
                self.sb_count += len;
                if self.sb_count >= sb_frames {
                    self.standby = false;
                }
            }
        }
        let mut y = [0.0f32; 2];
        for c in 0..2 {
            let d = if self.piece_sb { 0.0 } else { read(&self.line[c * LEN..(c + 1) * LEN], self.rp[c], lerp) };
            self.rp[c] = wrap(self.rp[c] + 1.0);
            y[c] = d - self.g * x[c];
            self.line[c * LEN + self.w] = x[c] + self.g * y[c];
        }
        self.w = (self.w + 1) % LEN;
        self.piece_left -= 1;
        self.call_left -= 1;
        y
    }
}

/// What sets the combs' delays and gains (`ReverbCore::DynamicParameter` from the core's
/// parameter calculator, read for the current percent value every call).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VerbLaw {
    /// Multi-mode REVERB (`MultiBeatFxReverb::CoreParameterCalculator::getDynamicParameter`
    /// 0x102a11354): size, input gain and decay from the percent value.
    Multi,
    /// Single-mode REVERB (`SingleBeatFxReverb::getDynamicParameter` 0x102a11c74): the
    /// percent is ROOM SIZE; DECAY's percent (a float, 1 .. 100) and its input gain are live.
    Single { decay: f32, gin: f32 },
    /// REVERB UP / DOWN (`ReverbUpCoreParamInfo` / `ReverbDownCoreParamInfo`): size, decay
    /// seconds to -60 dB, input gain.
    Fixed { size: f32, decay_s: f32, gin: f32 },
}

/// Single REVERB's DECAY knob as `getDynamicParameter` reads it: 1 .. 50 .. 100, unrounded.
pub fn single_decay_percent(k: f32) -> f32 {
    let h = crate::fx::half_value(k);
    if k < 0.48627 { h * 49.0 + 1.0 } else if k <= 0.51373 { 50.0 } else { h * 50.0 + 50.0 }
}

/// The time to -60 dB (s) for single REVERB's DECAY percent (the polynomial of
/// `SingleBeatFxReverb::getDynamicParameter`).
fn single_decay_secs(v: f32) -> f32 {
    let v2 = v * v;
    v2 * v2 * 0.0000028728 + v2 * (v * -0.00033391) + v2 * 0.016808 + v * -0.22553 + 1.7409
}

/// (input gain, comb delays ms, comb gains) for percent `p`.
pub fn dyn_params(law: VerbLaw, p: i32) -> (f32, [f32; 10], [f32; 10]) {
    let pf = p.clamp(1, 100) as f32;
    let mut d = [0.0f32; 10];
    let mut g = [0.0f32; 10];
    match law {
        VerbLaw::Multi => {
            let s = reverb_size(pf);
            // rekordbox reads each gain from `ReverbEffectCombFilterGainTable` (ten 100-step
            // tables, read from the installed rekordbox: comb k's left line table k, its right
            // line table 5 + k); without them 10^(-3 delay / decay time), the time fitted.
            let tab = crate::cfx::fc_tables().and_then(|t| t.verb.as_deref());
            for i in 0..10 {
                d[i] = REVERB_COMB_MS[i] * s;
                g[i] = match tab {
                    Some(t) => t[i / 2 + 5 * (i % 2)][(p.clamp(1, 100) - 1) as usize],
                    None => fitted_gain(i, p),
                };
            }
            (reverb_in_gain(pf), d, g)
        }
        VerbLaw::Single { decay, gin } => {
            let x = pf * 0.02306 + 1.01;
            let s = x * x * 0.171 + 0.2;
            let inv = 1.0 / single_decay_secs(decay);
            for i in 0..10 {
                d[i] = REVERB_COMB_MS[i] * s;
                g[i] = 10f32.powf(-0.003 * REVERB_COMB_MS[i] * s * inv);
            }
            (gin, d, g)
        }
        VerbLaw::Fixed { size, decay_s, gin } => {
            let k = -0.003 / decay_s.max(1e-4);
            for i in 0..10 {
                d[i] = (size * REVERB_COMB_MS[i]).clamp(0.0, 100.0);
                g[i] = 10f32.powf(k * d[i]).clamp(0.0, 1.0);
            }
            (gin, d, g)
        }
    }
}

/// Multi REVERB's gain of comb line `i` (0 .. 9: comb i / 2, left then right) at percent `p`
/// without rekordbox's table: 10^(-3 x delay / decay time), the decay time a fit of the tables
/// (within 0.004 of every entry of 7.2.19's).
pub fn fitted_gain(i: usize, p: i32) -> f32 {
    let pf = p.clamp(1, 100) as f32;
    10f32.powf(-3.0 * REVERB_COMB_MS[i] * reverb_size(pf) / reverb_rt60_ms(pf))
}

/// `BeatFxReverb::SmoothingTimeCalculator::calculateSmoothingTime` 0x102a112dc: the next
/// step's smoothing time (ms) from the last one.
pub fn smoothing_ms(cur: i32, target: i32, prev: f32) -> f32 {
    let want = (40.0 - 0.66667 * (cur - target).abs() as f32).max(20.0);
    let t = if want > prev { prev + 0.66667 } else if want < prev { prev - 0.66667 } else { prev };
    t.max(20.0)
}

/// The core's first-order sections (b0, b1, a1): the input's `IirLpf1stOrder` 14 kHz and
/// `IirHpf1stOrder` 80 Hz, the loops' `IirLpf1stOrder` (18 kHz at 44.1 kHz, 21.5 kHz for the FX+
/// reverbs; 20.5 kHz at 48 kHz, 30 kHz at 96 kHz, else 20 kHz; at least 5 Hz).
pub fn static_filters(rate: f32, fxplus: bool) -> [[f32; 3]; 3] {
    let loop_fc: f32 = match rate as i32 { 44100 => if fxplus { 21500.0 } else { 18000.0 }, 48000 => 20500.0, 96000 => 30000.0, _ => 20000.0 };
    let fc = |f: f32| f.min(rate * 0.5).max(5.0) as f64;
    [crate::fx_core::iir_1st(fc(14000.0), rate as f64, false), crate::fx_core::iir_1st(fc(80.0), rate as f64, true), crate::fx_core::iir_1st(fc(loop_fc), rate as f64, false)]
}

/// rekordbox's `ReverbCore` for one slot.
pub struct VerbCore {
    rate: f32,
    combs: Vec<Comb>,
    aps: [AllPass; 2],
    in_lp: [[f32; 2]; 2],
    in_hp: [[f32; 2]; 2],
    c_lp: [f32; 3],
    c_hp: [f32; 3],
    c_loop: [f32; 3],
    lerp: bool,
    fxplus: bool,
    gin: f32,
    out: f32,
    /// The percent value in use and its target, the smoothing time (ms), its frames, frames
    /// left of the step, done (`ReverbCore` 312 .. 332).
    cur: i32,
    target: i32,
    t_ms: f32,
    left: i32,
    done: bool,
    law: VerbLaw,
    sb_frames: i32,
    ap_sb_frames: i32,
    /// The buffer's percent value, frames per core call, frames of the buffer not yet called,
    /// frames left in this call, in its piece of the percent smoothing.
    pct: i32,
    piece: i32,
    buf_left: i32,
    call_left: i32,
    sub_left: i32,
    boundary: bool,
    /// A make-selected waits for the first call; REVERB then resets again on its first turn-on.
    selected: bool,
    armed: bool,
    made: bool,
}

impl VerbCore {
    pub fn new(rate: f32) -> VerbCore {
        Self::make(rate, true)
    }

    /// Without lines, for a slot that is never switched on.
    pub fn empty() -> VerbCore {
        Self::make(48000.0, false)
    }

    fn make(rate: f32, alloc: bool) -> VerbCore {
        let mut v = VerbCore { rate, combs: (0..5).map(|_| Comb::new(rate, alloc)).collect(), aps: [AllPass::new(alloc), AllPass::new(alloc)], in_lp: [[0.0; 2]; 2], in_hp: [[0.0; 2]; 2], c_lp: [0.0; 3], c_hp: [0.0; 3], c_loop: [0.0; 3], lerp: false, fxplus: false, gin: 0.0, out: 0.70794576, cur: 50, target: 50, t_ms: 40.0, left: 0, done: true, law: VerbLaw::Multi, sb_frames: 0, ap_sb_frames: 0, pct: 50, piece: i32::MAX, buf_left: 0, call_left: 0, sub_left: 0, boundary: false, selected: true, armed: false, made: alloc };
        v.set_static(rate, false);
        v
    }

    /// The effect was chosen for the slot (`onCalledMakeSelected`).
    pub fn selected(&mut self) {
        self.selected = true;
    }

    /// `getStaticParameter` (REVERB and the FX+ reverbs differ only in the loop low-pass at
    /// 44.1 kHz) and `updateAndSetStaticParameter` 0x102a1bd2c.
    fn set_static(&mut self, rate: f32, fxplus: bool) {
        self.rate = rate;
        self.fxplus = fxplus;
        let ri = rate as i32;
        self.lerp = !(ri == 44100 || ri == 48000);
        [self.c_lp, self.c_hp, self.c_loop] = static_filters(rate, fxplus);
        for (k, ap) in self.aps.iter_mut().enumerate() {
            ap.g = 0.7;
            for c in 0..2 {
                ap.set_delay(c, REVERB_AP_MS[2 * k + c] * 0.001 * rate);
            }
        }
        self.out = 0.70794576;
        self.sb_frames = (STANDBY_MS * 0.001 * rate) as i32;
        self.ap_sb_frames = (STANDBY_MS * 0.001 * rate.trunc()) as i32;
    }

    /// `updateAndSetDynamicParameter`: the combs' targets for the current percent.
    fn set_dyn(&mut self) {
        let (gin, d, g) = dyn_params(self.law, self.cur);
        self.gin = gin;
        let r = self.rate;
        for (k, cb) in self.combs.iter_mut().enumerate() {
            for c in 0..2 {
                let ms = d[2 * k + c].clamp(0.0, 100.0);
                cb.dly[c].set((ms * (0.001 * r)).min((LEN - 4) as f32));
                cb.gain[c].set(g[2 * k + c].clamp(0.0, 1.0));
            }
        }
    }

    /// `startPitchSmoothing` 0x102a1c4b0.
    fn start_smoothing(&mut self) {
        self.t_ms = smoothing_ms(self.cur, self.target, self.t_ms);
        self.left = ((self.t_ms * 0.001) * self.rate) as i32;
        let (t, r) = (self.t_ms, self.rate);
        for cb in self.combs.iter_mut() {
            for s in cb.dly.iter_mut().chain(cb.gain.iter_mut()) {
                s.set_ms(t, r);
            }
        }
        self.set_dyn();
    }

    /// `ReverbCore::reset` 0x102a1ca18.
    fn reset(&mut self, p: i32) {
        self.set_dyn();
        self.in_lp = [[0.0; 2]; 2];
        self.in_hp = [[0.0; 2]; 2];
        for cb in self.combs.iter_mut() {
            cb.standby = true;
            cb.sb_count = 0;
            cb.lp = [[0.0; 2]; 2];
        }
        for ap in self.aps.iter_mut() {
            ap.standby = true;
            ap.sb_count = 0;
            for c in 0..2 {
                ap.rp[c] = wrap(ap.w as f64 - ap.d[c] as f64);
            }
        }
        self.cur = p;
        self.target = p;
        self.t_ms = 40.0;
        self.left = 0;
        self.done = true;
        self.sub_left = 0;
    }

    /// The start of a call of `frames` (`ReverbCore::processEffect` 0x102a1c43c): `percent`
    /// the core's parameter (1 .. 100), `fxplus` for REVERB UP / DOWN (reset only when made
    /// selected), `starting` the slot turning on.
    pub fn begin(&mut self, percent: i32, law: VerbLaw, fxplus: bool, starting: bool, rate: f32, frames: usize) {
        if !self.made {
            return;
        }
        let p = percent.clamp(1, 100);
        self.law = law;
        if rate != self.rate || fxplus != self.fxplus {
            self.set_static(rate, fxplus);
            for cb in self.combs.iter_mut() {
                for s in cb.dly.iter_mut().chain(cb.gain.iter_mut()).chain(cb.arr.iter_mut()) {
                    *s = Sm::new(s.target, rate);
                }
            }
            self.selected = true;
        }
        if self.selected {
            self.selected = false;
            self.reset(p);
            self.armed = !fxplus;
        } else if starting && self.armed {
            // (`BeatFxReverb::onCalledTurnOn` 0x102a112a4: the first turn-on after it was made
            // selected resets the core again.)
            self.armed = false;
            self.reset(p);
        }
        self.pct = p;
        // (`FxPlusReverbUp::processEffectConcrete` 0x1029e9908 / `Down` call the core every
        // (int)(fs x 0.00066667) frames, 32 at 48 kHz; REVERB once a buffer.)
        self.piece = if fxplus { ((rate * 0.00066667) as i32).max(1) } else { i32::MAX };
        self.buf_left = frames as i32;
        self.call_left = 0;
    }

    /// A new law for the calls still to come in this buffer (the FX+ reverbs' parameters
    /// worked out after each piece).
    pub fn set_law(&mut self, law: VerbLaw) {
        self.law = law;
    }

    /// `ReverbCore::processEffect` for the next call of the buffer.
    fn call(&mut self) {
        let p = self.pct;
        if p != self.target {
            self.target = p;
            self.done = false;
            self.start_smoothing();
        }
        self.set_dyn();
        self.call_left = self.piece.min(self.buf_left);
        self.buf_left -= self.call_left;
        self.sub_left = 0;
    }

    /// One frame of the buffer.
    #[inline]
    pub fn frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.made {
            return (0.0, 0.0);
        }
        if self.call_left <= 0 {
            if self.buf_left <= 0 {
                return (0.0, 0.0);
            }
            self.call();
        }
        if self.sub_left == 0 {
            let m = if self.done {
                self.boundary = false;
                self.call_left
            } else if self.left > self.call_left {
                self.left -= self.call_left;
                self.boundary = false;
                self.call_left
            } else {
                self.boundary = true;
                self.left.max(1)
            };
            self.sub_left = m;
            for cb in self.combs.iter_mut() {
                cb.begin_call(m);
            }
            for ap in self.aps.iter_mut() {
                ap.call_left = m;
                ap.piece_left = 0;
            }
        }
        let mut x = [l * self.gin, r * self.gin];
        for c in 0..2 {
            x[c] = first(&self.c_lp, &mut self.in_lp[c], x[c]);
            x[c] = first(&self.c_hp, &mut self.in_hp[c], x[c]);
        }
        let mut acc = [0.0f32; 2];
        let (lc, lerp, sb) = (self.c_loop, self.lerp, self.sb_frames);
        for cb in self.combs.iter_mut() {
            let o = cb.frame(x, &lc, lerp, sb);
            acc[0] += o[0];
            acc[1] += o[1];
        }
        let asb = self.ap_sb_frames;
        let y = self.aps[0].frame(acc, lerp, asb);
        let y = self.aps[1].frame(y, lerp, asb);
        self.sub_left -= 1;
        self.call_left -= 1;
        if self.sub_left == 0 && self.boundary {
            if self.cur == self.target {
                self.done = true;
            } else {
                self.cur += if self.cur < self.target { 1 } else { -1 };
                self.start_smoothing();
            }
        }
        (y[0] * self.out, y[1] * self.out)
    }

    /// The percent value in use (for tests).
    #[cfg(test)]
    pub fn current(&self) -> i32 {
        self.cur
    }
}

/// The FX+ reverbs' knobs (`FxPlusReverbUp` / `FxPlusReverbDown`): the main knob, TIME and
/// FILTER are `SmoothShiftInteger`s (0 .. 255, a step per 0.66667 ms, made at 127; set once a
/// buffer from knob x 255.9), advanced a piece at a time ((int)(fs x 0.00066667) frames, 32
/// at 48 kHz); after each piece the main knob is followed by a float (made at 127) that moves
/// 1 / k toward it, k moving one a piece toward 15 - the distance (at most 5)
/// (`updateReverbUpParameter` 0x1029e9ba8 / `updateReverbDownParameter` 0x1029e6f7c, +792 / +776
/// and +720 / +722); the laws then take its whole part.
#[derive(Clone, Copy)]
pub struct FxRevKnobs {
    rate: f32,
    main: crate::fx_core::ShiftInt,
    time: crate::fx_core::ShiftInt,
    filt: crate::fx_core::ShiftInt,
    pub f: f32,
    k: i16,
}

impl FxRevKnobs {
    pub fn new(rate: f32) -> FxRevKnobs {
        let si = || crate::fx_core::ShiftInt::new(127, 0.66667, rate);
        FxRevKnobs { rate, main: si(), time: si(), filt: si(), f: 127.0, k: 0 }
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Frames of a piece (`(int)(fs x 0x3A2EC33F)`).
    pub fn piece(rate: f32) -> usize {
        ((rate * f32::from_bits(0x3A2E_C33F)) as i32).max(1) as usize
    }

    /// `updateAndSetParameter`: the three knobs (0 .. 1).
    pub fn begin(&mut self, main: f32, time: f32, filt: f32) {
        self.main.set(((main * 255.9) as i32).clamp(0, 255));
        self.time.set(((time * 255.9) as i32).clamp(0, 255));
        self.filt.set(((filt * 255.9) as i32).clamp(0, 255));
    }

    /// The end of a piece of `n` frames: the knobs step, then the follower.
    pub fn piece_end(&mut self, n: usize) {
        self.main.advance(n as i32);
        self.time.advance(n as i32);
        self.filt.advance(n as i32);
        let v2 = self.main.cur;
        let fi = self.f as i32;
        let (dir, d) = if v2 > fi { (1.0f32, v2 - fi) } else if v2 < fi { (-1.0, fi - v2) } else { (0.0, 0) };
        if d >= 1 {
            let want = (15 - d.min(5)) as i16;
            let k = self.k;
            let v14 = if want <= k { k } else { k + 1 };
            let v16 = v14 - (want < v14) as i16;
            if want > k || want < v14 {
                self.k = v16;
            }
            self.f = (dir / v16 as f32 + self.f).max(0.0).min(255.0);
        }
    }

    /// REVERB DOWN's dry gain (`FxPlusReverbDown::calculateMixRatio` 0x1029e72cc): from the
    /// main knob's stepped value, 2 - v x 0.0078431 above the centre.
    pub fn down_dry(&self) -> f32 {
        let v = self.main.cur as f32;
        if v * 0.0039216 > 0.5 { v * -0.0078431 + 2.0 } else { 1.0 }
    }

    /// (main knob as the laws take it, TIME, FILTER).
    pub fn values(&self) -> (i32, i32, i32) {
        (self.f as i32, self.time.cur, self.filt.cur)
    }
}

#[cfg(test)]
mod tests {
    /// The FX+ reverbs' main-knob follower: a jump from 127 to 255 is followed 1, 1/2, 1/3 ...
    /// a piece (the step count rising toward 10), never all at once.
    #[test]
    fn fx_plus_reverb_knob_follower_is_rekordbox_s() {
        assert_eq!(FxRevKnobs::piece(48000.0), 32);
        let mut k = FxRevKnobs::new(48000.0);
        k.begin(1.0, 0.5, 0.0);
        // The SmoothShiftInteger: a step per 32 frames (0.66667 ms), so 32 frames a step.
        k.piece_end(32);
        assert_eq!(k.values().0, 128, "{}", k.f);
        k.piece_end(32);
        assert!((k.f - 128.5).abs() < 1e-5, "{}", k.f);
        for _ in 0..20000 {
            k.piece_end(32);
        }
        assert_eq!(k.values(), (255, 127, 0));
    }

    use super::*;

    fn run(v: &mut VerbCore, p: i32, law: VerbLaw, frames: usize, x: impl Fn(usize) -> f32) -> Vec<f32> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < frames {
            let n = 512.min(frames - i);
            v.begin(p, law, false, false, 48000.0, n);
            for _ in 0..n {
                let s = x(i);
                out.push(v.frame(s, s).0);
                i += 1;
            }
        }
        out
    }

    #[test]
    fn multi_gains_from_rekordboxs_table() {
        // 7.2.19's TABLE at p = 1 and 100 for the first comb's left line (table 0) and its
        // right line (table 5), when the installed rekordbox has them.
        let Some(t) = crate::cfx::fc_tables_now().and_then(|t| t.verb.as_deref()) else { return };
        assert!((t[0][0] - 0.92651).abs() < 1e-5 && (t[0][99] - 0.99873).abs() < 1e-5 && (t[5][0] - 0.87497).abs() < 1e-5, "{} {} {}", t[0][0], t[0][99], t[5][0]);
        let (_, _, g) = dyn_params(VerbLaw::Multi, 50);
        assert_eq!((g[0], g[1]), (t[0][49], t[5][49]));
        assert!((0..10).all(|i| (0..100).all(|p| (fitted_gain(i, p + 1) - t[i / 2 + 5 * (i % 2)][p as usize]).abs() < 0.004)));
    }

    #[test]
    fn smoothing_time_law() {
        // max(40 - 0.66667 |d|, 20) approached by 0.66667 ms a step, never under 20 ms.
        assert!((smoothing_ms(10, 60, 40.0) - 39.33333).abs() < 1e-4);
        assert_eq!(smoothing_ms(50, 50, 40.0), 40.0);
        assert!((smoothing_ms(59, 60, 30.0) - 30.66667).abs() < 1e-4);
        assert_eq!(smoothing_ms(10, 90, 20.3), 20.0);
    }

    #[test]
    fn stands_by_after_a_reset() {
        let sb = (128.625f32 * 0.001 * 48000.0) as usize;
        assert_eq!(sb, 6174);
        // An impulse on stand-by: the combs give nothing and feed nothing back (the all-passes
        // only hear the combs): nothing comes out, then or later.
        let mut v = VerbCore::new(48000.0);
        let out = run(&mut v, 50, VerbLaw::Multi, 24000, |i| if i == 0 { 1.0 } else { 0.0 });
        assert!(out.iter().all(|&s| s.abs() < 1e-9), "nothing rings from the stand-by");
        // An impulse after the stand-by rings.
        let mut v = VerbCore::new(48000.0);
        let out = run(&mut v, 50, VerbLaw::Multi, 48000, |i| if i == 7000 { 1.0 } else { 0.0 });
        let tail: f32 = out[8000..].iter().map(|s| s * s).sum();
        assert!(tail > 1e-4, "{tail}");
        assert!(out.iter().all(|s| s.is_finite() && s.abs() < 4.0));
    }

    #[test]
    fn percent_steps_one_at_a_time() {
        let mut v = VerbCore::new(48000.0);
        run(&mut v, 50, VerbLaw::Multi, 4800, |_| 0.0);
        assert_eq!(v.current(), 50);
        // 50 -> 60: the first step after one smoothing time (39.333 ms), then one each
        // smoothing time (38.667, 38.0, ... ms).
        run(&mut v, 60, VerbLaw::Multi, 1880, |_| 0.0);
        assert_eq!(v.current(), 50);
        run(&mut v, 60, VerbLaw::Multi, 20, |_| 0.0);
        assert_eq!(v.current(), 51);
        run(&mut v, 60, VerbLaw::Multi, 48000, |_| 0.0);
        assert_eq!(v.current(), 60);
    }

    #[test]
    fn dynamic_laws_as_traced() {
        // Single: size 0.2 + 0.171 (0.02306 p + 1.01)^2, gains 10^(-0.003 ms size / T(decay)).
        let (gin, d, g) = dyn_params(VerbLaw::Single { decay: 50.0, gin: 0.2 }, 50);
        let s = 0.2 + 0.171 * (0.02306f32 * 50.0 + 1.01).powi(2);
        assert!((d[0] - 22.09375 * s).abs() < 1e-3 && gin == 0.2);
        assert!((g[0] - 10f32.powf(-0.066281 * s / 8.7007)).abs() < 2e-4, "{}", g[0]);
        // FX+: delays at most 100 ms, gains 10^(-0.003 d / decay).
        let (_, d, g) = dyn_params(VerbLaw::Fixed { size: 3.0, decay_s: 2.0, gin: 0.1 }, 1);
        assert_eq!(d[6], 100.0);
        assert!((g[6] - 10f32.powf(-0.003 * 100.0 / 2.0)).abs() < 1e-6);
        assert!((single_decay_percent(0.0) - 1.0).abs() < 1e-6 && single_decay_percent(0.5) == 50.0 && (single_decay_percent(1.0) - 100.0).abs() < 1e-4);
    }
}
