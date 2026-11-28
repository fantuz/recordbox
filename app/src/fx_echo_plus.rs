//! rekordbox's FX+ echoes (BPF ECHO, HPF ECHO, LPF ECHO, CRUSH ECHO), ported from the 7.2.19
//! arm64 binary: `FxPlusBpfEcho` (0x1029d8414 .. 0x1029d97d4), `FxPlusHpfEcho` (0x1029dd94c ..),
//! `FxPlusLpfEcho` (0x1029e02bc ..), `FxPlusCrushEcho` (0x1029daed4 ..), the `Rmx4thEcho` they
//! end in (0x1029eb7bc .. 0x1029ed758), `CrushCore` (0x1029abdcc ..) and the filters they use
//! (`IirBpf2ndOrder` on `IirFilter::handleFilterCore2nd` 0x1028bf174, `Filter::make_hpf21`
//! 0x102aaeb50 / `make_lpf21` 0x102aae9fc with `filter21_classic` 0x102aaec9c).
//!
//! A call (`processEffectConcrete`) copies its input, runs it through the input gate (3.3333 ms)
//! and then, in pieces of `(int)(rate x 0.00033333)` frames (15 at 48 kHz), through the filter
//! (CRUSH: the sample-hold, then its high-pass); the output is `input x dry + filtered x wet`, the
//! filtered sound is scaled by `wet` and, after each piece, the three knobs' 8-bit steppers
//! (`SmoothShiftInteger`, a step every 0.33333 ms) move on and the next piece's filter and gains
//! are worked out from them (with the MOD LFO in single mode). Then the filtered sound goes through
//! two 284 Hz high-passes and `Rmx4thEcho`, whose echoes are added to the output.
//!
//! Everything a call needs is set when it starts (`call`); the frames then come one at a time
//! (`next`), in the order rekordbox works them, so the numbers are the same. Nothing here
//! allocates or locks after `FxPlusEcho::new`.

use crate::fx_core::{Filter2, ShiftInt};
use crate::fx_echo::EchoCore;
use crate::fx_lfo::{Lfo, Sf, Wave};
use crate::fx_rev::{new_gate, RevFx, RevMode};

const EPS: f32 = 1.1920929e-7;

/// `FxPlus*Echo::*_DEF_STEP` (set by the static initialisers, e.g. 0x1029daeac).
pub const BPF_DEF_STEP: f32 = 1.0163086652755737;
pub const HPF_DEF_STEP: f32 = 1.0201632976531982;
pub const LPF_DEF_STEP: f32 = 0.9789531826972961;
pub const CRUSH_HPF_DEF_STEP: f32 = 1.0168726444244385;

/// Which of the four.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlusKind {
    Bpf,
    Hpf,
    Lpf,
    Crush,
}

/// One call's parameters (`*EchoParameter`): ON, the main knob (FREQ / LEVEL), ECHO MIX
/// (`ECHOSEL`), MOD, REV, FREEZE (0 .. 1 knobs; multi mode: echo 0.5, mod 0, no buttons), and
/// one beat in ms (`EffectBase::getOneBeatTimeMs`).
#[derive(Clone, Copy, Debug)]
pub struct PlusPar {
    pub on: bool,
    pub main: f32,
    pub echo: f32,
    pub modu: f32,
    pub rev: bool,
    pub freeze: bool,
    pub beat_ms: f32,
}

/// `Filter::make_hpf2(fc, dB, prewarp)` 0x102aad3bc in doubles (Q = 10^(dB / 20)).
pub fn make_hpf2_db(fc: f64, db: f64, prewarp: bool, rate: f64) -> [f64; 5] {
    let t = 1.0 / rate;
    let w = if prewarp { rate * (2.0 * (fc * 3.14159265 * t).tan()) } else { fc * 6.28318531 };
    let q = 10f64.powf(db * 0.05);
    let v10 = w * w * (t * t);
    let v11 = w * (t + t) / q;
    let n = 1.0 / (v10 + 4.0 + v11);
    [4.0 * n, -8.0 * n, 4.0 * n, (v10 + v10 - 8.0) * n, n * (v10 + 4.0 - v11)]
}

/// `Filter` as `make_hpf21` / `make_lpf21` set it and `filter21_classic` runs it: a
/// Butterworth biquad (transposed direct form II, doubles) whose input gets `res` times the state
/// of a first-order section run on the output (the opposite kind: low-pass for the high-pass).
#[derive(Clone, Copy, Debug, Default)]
pub struct F21 {
    /// +64 .. +120: b0, b1, a1, b2, a2, then the first-order b0, b1, a1; +224: res.
    pub c: [f64; 8],
    pub res: f64,
    z1: [f64; 2],
    z2: [f64; 2],
    s: [f64; 2],
}

impl F21 {
    /// `make_hpf21` (`hp`) / `make_lpf21`: cutoff `fc`, feedback `res`, pre-warped or not.
    pub fn make(&mut self, hp: bool, fc: f64, res: f64, prewarp: bool, rate: f64) {
        let t = 1.0 / rate;
        let w = if prewarp { rate * (2.0 * (fc * 3.14159265 * t).tan()) } else { fc * 6.28318531 };
        let v10 = w * (w * (t * t));
        let v11 = t * w;
        let n = 1.0 / (v11 * 2.82842712 + 4.0 + v10);
        let a1 = n * (v10 + v10 - 8.0);
        let a2 = n * (4.0 - v11 * 2.82842712 + v10);
        let m = 1.0 / (v11 + 2.0);
        self.c = if hp {
            [4.0 * n, -8.0 * n, a1, 4.0 * n, a2, m * v11, m * v11, m * (v11 - 2.0)]
        } else {
            [n * v10, (v10 + v10) * n, a1, n * v10, a2, 2.0 * m, -2.0 * m, m * (v11 - 2.0)]
        };
        self.res = res;
    }

    pub fn clear(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
        self.s = [0.0; 2];
    }

    #[inline]
    pub fn run(&mut self, ch: usize, x: f32) -> f32 {
        let c = &self.c;
        let u = x as f64 + self.res * self.s[ch];
        let y = self.z1[ch] + u * c[0];
        self.z1[ch] = self.z2[ch] + u * c[1] - y * c[2];
        self.z2[ch] = -(y * c[4] - u * c[3]);
        self.s[ch] = -((self.s[ch] + y * c[5]) * c[7] - y * c[6]);
        y as f32
    }
}

/// `IirBpf2ndOrder`: `IirFilter::calcCoefBpf2ndOrder` (peak gain 1) in floats, run by
/// `handleFilterCore2nd` with `fb` times its last output added to each input (BPF ECHO: 0.7).
#[derive(Clone, Copy, Debug)]
pub struct IirBpf {
    rate: i32,
    /// +12: Nyquist, +16: the cutoff, +128: Q (double), +104: the feedback.
    nyq: f32,
    pub fc: f32,
    pub q: f64,
    pub fb: f32,
    /// +108 .. +124: b0, b1, a1, b2, a2.
    c: [f32; 5],
    x1: [f32; 2],
    x2: [f32; 2],
    y1: [f32; 2],
    y2: [f32; 2],
    last: [f32; 2],
}

impl IirBpf {
    /// `IirFilter::IirFilter(rate, 2)`: cutoff rate / 4 + 2.5, Q 0.70711, no feedback.
    pub fn new(rate: i32) -> IirBpf {
        let r = rate.max(10);
        let mut f = IirBpf { rate: r, nyq: r as f32 * 0.5, fc: r as f32 * 0.25 + 2.5, q: f64::from_bits(0x3FE6A09E667F3BCD), fb: 0.0, c: [0.0; 5], x1: [0.0; 2], x2: [0.0; 2], y1: [0.0; 2], y2: [0.0; 2], last: [0.0; 2] };
        f.calc();
        f
    }

    /// (`FxPlusBpfEcho::samplingRateChanged`: the rate, the cutoff held under Nyquist, the
    /// states cleared, the coefficients again.)
    pub fn set_rate(&mut self, rate: i32) {
        self.rate = rate;
        self.nyq = rate as f32 * 0.5;
        self.fc = self.nyq.min(self.fc);
        self.clear();
        self.calc();
    }

    pub fn clear(&mut self) {
        self.x1 = [0.0; 2];
        self.x2 = [0.0; 2];
        self.y1 = [0.0; 2];
        self.y2 = [0.0; 2];
        self.last = [0.0; 2];
    }

    /// `calcCoefBpf2ndOrder` 0x1028bfc64.
    pub fn calc(&mut self) {
        let q = self.q.max(0.01);
        let v6 = (self.fc as f64 * 3.14159265 / self.rate as f64).tan();
        let v7 = ((1.0 / q * (1.0 / q) + 4.0).sqrt() - 1.0 / q) * (0.0795774715 * v6);
        let v8 = 1.0 / q * 0.159154943 * v6;
        let v9 = v7 + v8;
        let v10 = v8 * 6.28318531;
        let v11 = v7 * 39.4784176 * v9;
        let n = 1.0 / (v10 + 1.0 + v11);
        let b0 = (n * v10) as f32;
        self.c = [b0, 0.0, ((v11 + v11 - 2.0) * n) as f32, -b0, (n * (1.0 - v10 + v11)) as f32];
    }

    /// Set the cutoff as `FxPlusBpfEcho::updateFilterCoef` does: under Nyquist, at least 5 Hz.
    pub fn set_fc(&mut self, fc: f32) {
        let f = self.nyq.min(fc);
        self.fc = if fc >= 5.0 { f } else { 5.0 };
        self.calc();
    }

    #[inline]
    pub fn run(&mut self, ch: usize, x: f32) -> f32 {
        let c = &self.c;
        let u = x + self.last[ch] * self.fb;
        let y = ((self.x1[ch] * c[1]) + (self.x2[ch] * c[3]) - ((self.y1[ch] * c[2]) + (self.y2[ch] * c[4]))) + u * c[0];
        self.x2[ch] = self.x1[ch];
        self.x1[ch] = u;
        self.y2[ch] = self.y1[ch];
        self.y1[ch] = y;
        self.last[ch] = y;
        y
    }
}

/// `CrushCore`: a sample-hold whose hold time (`HOLD TIME msec`, 0 .. 10) is read at the start
/// of each call; `acc` counts down the frames still to hold.
#[derive(Clone, Copy, Debug)]
pub struct Crush {
    rate: i32,
    /// The parameter as set, and as last taken (+720) with its frames (+724).
    pub param: f32,
    ms: f32,
    frames: f32,
    acc: f32,
    held: [f32; 2],
}

impl Crush {
    pub fn new(rate: i32) -> Crush {
        Crush { rate, param: 0.0, ms: 0.0, frames: 0.0, acc: 0.0, held: [0.0; 2] }
    }

    /// `EffectBase::setParameter(0, ms)` (`LinearParameter`: held to 0 .. 10).
    pub fn set(&mut self, ms: f32) {
        self.param = if ms >= 0.0 { ms.min(10.0) } else { 0.0 };
    }

    /// `onCalledMakeSelected` 0x1029ac040.
    pub fn make_selected(&mut self) {
        self.acc = 0.0;
        self.ms = self.param;
        self.frames = (self.ms * 0.001) * self.rate as f32;
        self.held = [0.0; 2];
    }

    /// The start of a call (`processEffectConcrete` 0x1029abf34): a new hold time is taken.
    pub fn begin(&mut self) {
        if (self.ms - self.param).abs() >= EPS {
            self.ms = self.param;
            self.frames = (self.ms * 0.001) * self.rate as f32;
        }
    }

    #[inline]
    pub fn run(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.acc >= 1.0 {
            self.acc += -1.0;
            (self.held[0], self.held[1])
        } else {
            self.held = [l, r];
            self.acc += self.frames;
            (l, r)
        }
    }
}

/// `Rmx4thEcho`: the FX+ pack's echoes. `four` (single mode): forward `EchoCore`s at a quarter,
/// a half, three quarters of a beat and a beat, and reverse ones at the same times; otherwise the
/// half-beat core alone.
pub struct Rmx4 {
    four: bool,
    rate: i32,
    /// +0 one beat (ms), +4 ECHO MIX, +8 REV, +9 FREEZE, +12 filled, +16 frames counted, +20
    /// the count's end (4 s).
    pub beat_ms: f32,
    pub mix: f32,
    pub rev: bool,
    pub freeze: bool,
    pub filled: bool,
    count: i32,
    lim: i32,
    /// Gates: input (+32), forward (+128), reverse (+224).
    g_in: Sf,
    g_fwd: Sf,
    g_rev: Sf,
    /// Forward cores (index 0 = a quarter beat .. 3 = a beat; without `four` only the half).
    fwd: Vec<Box<EchoCore>>,
    rev_c: Vec<Box<RevFx>>,
    /// This call's input weights.
    w: [f32; 4],
}

impl Rmx4 {
    pub fn new(four: bool) -> Rmx4 {
        let n = if four { 4 } else { 1 };
        let mut m = Rmx4 {
            four,
            rate: 48000,
            beat_ms: 500.0,
            mix: 0.5,
            rev: false,
            freeze: false,
            filled: false,
            count: 0,
            lim: 192000,
            g_in: new_gate(48000.0),
            g_fwd: new_gate(48000.0),
            g_rev: new_gate(48000.0),
            fwd: (0..n).map(|_| EchoCore::new()).collect(),
            rev_c: (0..if four { 4 } else { 0 }).map(|_| RevFx::new()).collect(),
            w: [0.0; 4],
        };
        m.setup(48000);
        m
    }

    /// Each core's time (`fmaxf(fminf(beat x k, 4000), 10)`).
    fn times(&self) -> [f32; 4] {
        let b = self.beat_ms;
        [(b * 0.25).min(4000.0).max(10.0), (b * 0.5).min(4000.0).max(10.0), (b * 0.75).min(4000.0).max(10.0), b.min(4000.0).max(10.0)]
    }

    /// `samplingRateChanged` 0x1029ec334 (the cores made at the rate).
    pub fn setup(&mut self, rate: i32) {
        self.rate = rate;
        self.filled = false;
        self.count = 0;
        self.lim = (rate as f32 * 4.0).ceil() as i32;
        let r = rate as f32;
        for g in [&mut self.g_in, &mut self.g_fwd, &mut self.g_rev] {
            *g = new_gate(r);
        }
        for c in self.fwd.iter_mut() {
            c.setup(r);
        }
        for c in self.rev_c.iter_mut() {
            c.setup(RevMode::Echo, r);
        }
    }

    /// `reset` 0x1029ecb04: the count, every core, the gates shut.
    pub fn reset(&mut self) {
        self.filled = false;
        self.count = 0;
        self.lim = (self.rate as f32 * 4.0).ceil() as i32;
        let t = self.times();
        if self.four {
            for (c, ms) in self.fwd.iter_mut().zip(t) {
                c.reset([ms; 2], [0.75; 2]);
            }
            for c in self.rev_c.iter_mut() {
                c.reset();
            }
        } else {
            self.fwd[0].reset([t[1]; 2], [0.75; 2]);
        }
        self.g_in.reset(0.0);
        self.g_fwd.reset(0.0);
        self.g_rev.reset(0.0);
    }

    /// `onTurnOn` 0x1029ed758: the line is counted again.
    pub fn turn_on(&mut self) {
        self.filled = false;
        self.count = 0;
    }

    /// The start of a `processEffect` call of `len` frames (0x1029ecbf4).
    pub fn call(&mut self, len: usize) {
        let t = self.times();
        if self.four {
            if self.rev {
                self.g_fwd.set_target(0.0);
                self.g_rev.set_target(1.0);
            } else {
                self.g_fwd.set_target(1.0);
                self.g_rev.set_target(0.0);
            }
        }
        // The line holds a beat once the frames counted since the turn-on reach it (counted up
        // to 4 s); FREEZE then shuts the input and holds the repeats at feedback 1.
        self.filled = true;
        if self.count < self.lim {
            let need = ((self.beat_ms * 0.001) * self.rate as f32).ceil() as i32;
            self.count += len as i32;
            if self.count < need {
                self.filled = false;
            }
        }
        let fb = if self.filled && self.freeze {
            self.g_in.set_target(0.0);
            1.0
        } else {
            self.g_in.set_target(1.0);
            0.75
        };
        if self.four {
            for (c, ms) in self.fwd.iter_mut().zip(t) {
                c.set([ms; 2], [fb; 2]);
                c.call(len);
            }
            for (c, ms) in self.rev_c.iter_mut().zip(t) {
                c.begin_call(len, len, [ms; 2], [fb; 2], [true; 2], 0.0);
            }
            let m = self.mix;
            let x = m * 4.0;
            self.w = if m < 0.25 {
                [x, 0.0, 0.0, 0.0]
            } else if m < 0.5 {
                [2.0 - x, x + -1.0, 0.0, 0.0]
            } else if m < 0.75 {
                [0.0, 3.0 - x, x + -2.0, 0.0]
            } else {
                [0.0, 0.0, 4.0 - x, x + -3.0]
            };
        } else {
            self.fwd[0].set([t[1]; 2], [fb; 2]);
            self.fwd[0].call(len);
        }
    }

    /// One frame: the echoes of `l`, `r` (the input itself is not passed: +10 is never set).
    #[inline]
    pub fn next(&mut self, l: f32, r: f32) -> (f32, f32) {
        let g = self.g_in.take();
        let (el, er) = ((l * 0.7) * g, (r * 0.7) * g);
        if !self.four {
            return self.fwd[0].next(el, er);
        }
        let mut y = [(0.0f32, 0.0f32); 4];
        let mut v = [(0.0f32, 0.0f32); 4];
        for i in 0..4 {
            let (a, b) = (el * self.w[i], er * self.w[i]);
            y[i] = self.fwd[i].next(a, b);
            v[i] = self.rev_c[i].next(a, b);
        }
        let (gf, gr) = (self.g_fwd.take(), self.g_rev.take());
        let fl = ((y[1].0 + y[0].0) + (y[2].0 + y[3].0)) * gf;
        let fr = ((y[1].1 + y[0].1) + (y[2].1 + y[3].1)) * gf;
        let rl = ((v[1].0 + v[0].0) + (v[2].0 + v[3].0)) * gr;
        let rr = ((v[1].1 + v[0].1) + (v[2].1 + v[3].1)) * gr;
        (fl + rl, fr + rr)
    }
}

/// The 8-bit step of a knob (`(int)(v x 255.9)`).
#[inline]
fn step_of(v: f32) -> i32 {
    (v * 255.9) as i32
}

/// The dry and filtered shares for the main knob's step `v` (`updateFilterCoef` /
/// `updateCrushCoef`: f720, f724), with the fused multiply-adds the binary uses.
pub fn plus_gains(kind: PlusKind, v: i32) -> (f32, f32) {
    let g = if v < 4 { 1.0 } else { (v as f32).mul_add(-0.0039841, 1.0159) };
    let g3 = (g * g) * g;
    match kind {
        PlusKind::Bpf => (1.0, (-(g * g)).mul_add(g, 1.0)),
        PlusKind::Hpf => {
            let mut w = 1.0 - g3;
            if v >= 231 {
                w *= ((255 - v) as f32).mul_add(0.012, 0.7);
            }
            (g3, w)
        }
        PlusKind::Lpf => {
            let (d, v7) = if v > 3 {
                if v > 31 {
                    (0.0, if v >= 224 { ((255 - v) as f32).mul_add(0.015625, 0.5) } else { 1.0 })
                } else {
                    let s = (v - 4) as f32 * 0.035714;
                    (1.0 - s, s)
                }
            } else {
                (1.0, 0.0)
            };
            (d, v7 + v7)
        }
        PlusKind::Crush => (g3, 1.0 - g3),
    }
}

/// MOD's reach in 8-bit steps for its step `m` (FREQ: 0 under 1/64 of the knob, then
/// 1.3943 m - 5.5556, 50 from 0.15625).
fn mod_depth(m: i32) -> f32 {
    let v = m as f32 * 0.0039216;
    if v >= 0.015625 {
        if v >= 0.15625 { 50.0 } else { (m as f32).mul_add(1.3943, -5.5556) }
    } else {
        0.0
    }
}

/// The resonance of HPF ECHO / CRUSH ECHO's high-pass and LPF ECHO's low-pass.
/// (`x^16` by four squarings, then `top - x^16 x top` fused.)
fn res_hp(v: i32) -> f32 {
    let mut a = (255 - v) as f32 * 0.0039683;
    for _ in 0..4 {
        a *= a;
    }
    (-a).mul_add(1.4, 1.4)
}

fn res_lp(v: i32) -> f32 {
    let mut a = (v - 3) as f32 * 0.0039683;
    for _ in 0..4 {
        a *= a;
    }
    (-a).mul_add(1.5, 1.5)
}

/// The BPF ECHO / HPF ECHO / LPF ECHO cutoff for a (modulated) step.
pub fn plus_fc(kind: PlusKind, v: i32) -> f32 {
    match kind {
        PlusKind::Bpf => {
            if v >= 4 { BPF_DEF_STEP.powf(((v - 4) as f32 * 0.0039841).powf(1.1) * 251.0) * 150.0 } else { 150.0 }
        }
        PlusKind::Hpf => {
            if v >= 4 { HPF_DEF_STEP.powi(v - 4) * 60.0 } else { 60.0 }
        }
        PlusKind::Lpf => {
            if v >= 4 { LPF_DEF_STEP.powf(((v - 4) as f32 * 0.0039841).powf(0.78) * 251.0) * 20000.0 } else { 20000.0 }
        }
        PlusKind::Crush => {
            if v >= 4 { CRUSH_HPF_DEF_STEP.powi(v - 4) * 60.0 } else { 60.0 }
        }
    }
}

/// One FX+ echo with its state (made by the interface thread; about 25 MB in single mode, the
/// eight echo cores, 1.6 MB in multi mode).
pub struct FxPlusEcho {
    kind: PlusKind,
    single: bool,
    rate: i32,
    gate: Sf,
    bpf: IirBpf,
    f21: F21,
    crush: Crush,
    hp: [Filter2; 2],
    /// Main knob, ECHO MIX, MOD (`SmoothShiftInteger` 792 / 800 / 808).
    sh: [ShiftInt; 3],
    lfo: Lfo,
    piece: usize,
    /// f720 / f724 (CRUSH: f728 / f732).
    dry: f32,
    wet: f32,
    pub rmx: Rmx4,
    /// The call: frames left, place in the piece, the piece's length.
    left: usize,
    pos: usize,
    plen: usize,
}

impl FxPlusEcho {
    /// Made for single mode (`single`: the LFO and all eight cores) or multi mode.
    pub fn new(single: bool) -> Box<FxPlusEcho> {
        let mut f = Box::new(FxPlusEcho {
            kind: PlusKind::Bpf,
            single,
            rate: 48000,
            gate: new_gate(48000.0),
            bpf: IirBpf::new(48000),
            f21: F21::default(),
            crush: Crush::new(48000),
            hp: [Filter2::default(); 2],
            sh: [ShiftInt::new(127, 0.33333, 48000.0); 3],
            lfo: Lfo::new(48000.0, Wave::TRI),
            piece: 15,
            dry: 1.0,
            wet: 0.0,
            rmx: Rmx4::new(single),
            left: 0,
            pos: 0,
            plen: 0,
        });
        f.setup(PlusKind::Bpf, 48000.0);
        f
    }

    pub fn kind(&self) -> PlusKind {
        self.kind
    }

    pub fn single(&self) -> bool {
        self.single
    }

    pub fn rate(&self) -> f32 {
        self.rate as f32
    }

    /// Made as `kind` at `rate` (the constructor's state, then `samplingRateChanged`).
    pub fn setup(&mut self, kind: PlusKind, rate: f32) {
        let ri = rate as i32;
        let rd = ri as f64;
        self.kind = kind;
        self.rate = ri;
        self.piece = ((ri as f32 * 0.00033333) as i32).max(1) as usize;
        self.gate = new_gate(rate);
        self.dry = 1.0;
        self.wet = 0.0;
        self.sh = [ShiftInt::new(127, 0.33333, rate); 3];
        // (TriangleWave, -1 .. 1, smoothing 3.3333 ms, 200 ms, reset.)
        self.lfo = Lfo::new(rate, Wave::TRI);
        self.lfo.set_amplitude_range(false, -1.0, 1.0);
        self.lfo.set_smoothing_time(3.3333);
        self.lfo.set_period_time(200.0, true);
        self.lfo.reset();
        self.bpf = IirBpf::new(ri);
        self.bpf.fb = 0.7;
        self.bpf.set_rate(ri);
        self.f21 = F21::default();
        self.crush = Crush::new(ri);
        match kind {
            PlusKind::Lpf => self.f21.make(false, 20000.0, 1.5, true, rd),
            _ => self.f21.make(true, 60.0, 0.0, true, rd),
        }
        let c = make_hpf2_db(284.0, -3.0, true, rd);
        for h in self.hp.iter_mut() {
            h.c = c;
            h.clear();
        }
        self.rmx.setup(ri);
        self.left = 0;
        self.pos = 0;
        self.plen = 0;
    }

    /// `updateAndSetParameter`: the input gate, the steppers' targets, the echoes' beat and
    /// buttons (FREEZE only while on).
    fn params(&mut self, p: &PlusPar) {
        if p.on {
            self.gate.set_target(1.0);
        } else {
            self.gate.set_target(0.0);
        }
        self.sh[0].set(step_of(p.main));
        // (CRUSH ECHO's `updateAndSetParameter` 0x1029db684 takes ECHO MIX's step in double.)
        self.sh[1].set(if self.kind == PlusKind::Crush { (p.echo as f64 * 255.9) as i32 } else { step_of(p.echo) });
        self.sh[2].set(step_of(p.modu));
        self.rmx.beat_ms = p.beat_ms;
        self.rmx.freeze = p.freeze && p.on;
        self.rmx.rev = p.rev;
    }

    /// `onCalledMakeSelected`: the parameters, the gate shut, the filters, LFO and echoes
    /// cleared, the steppers at the knobs.
    pub fn make_selected(&mut self, p: &PlusPar) {
        self.params(p);
        self.gate.reset(0.0);
        self.bpf.clear();
        self.f21.clear();
        if self.kind == PlusKind::Crush {
            self.crush.make_selected();
        }
        if self.single {
            self.lfo.reset();
        }
        for h in self.hp.iter_mut() {
            h.clear();
        }
        self.rmx.reset();
        self.sh[0].reset(step_of(p.main));
        self.sh[1].reset(step_of(p.echo));
        self.sh[2].reset(step_of(p.modu));
    }

    /// `onCalledTurnOn` (`Rmx4thEcho::onTurnOn`).
    pub fn turn_on(&mut self) {
        self.rmx.turn_on();
    }

    /// `updateFilterCoef` / `updateCrushCoef` after a piece of `n` frames.
    fn update(&mut self, n: usize) {
        let v3 = self.sh[0].cur;
        let v4 = self.sh[1].cur;
        let v5 = self.sh[2].cur;
        (self.dry, self.wet) = plus_gains(self.kind, v3);
        let mut first = 0.0f32;
        let mut have = false;
        if self.single {
            self.lfo.set_period_time((v5 as f32).mul_add(-0.70588, 200.0), true);
            if n >= 1 {
                let mut buf = [0.0f32; 64];
                let k = n.min(64);
                self.lfo.get_values(&mut buf[..k]);
                if n > 64 {
                    // (Pieces are 15 frames at 48 kHz; longer ones only at rates above 192 kHz.)
                    let mut rest = n - 64;
                    while rest > 0 {
                        let k2 = rest.min(64);
                        let mut b2 = [0.0f32; 64];
                        self.lfo.get_values(&mut b2[..k2]);
                        rest -= k2;
                    }
                }
                first = buf[0];
                have = true;
            }
        }
        let rd = self.rate as f64;
        match self.kind {
            PlusKind::Crush => {
                let mut off = 0.0;
                if have {
                    let m = v5 as f32 * 0.0039216;
                    let reach = if m >= 0.015625 { if m < 0.15625 { (v5 as f32).mul_add(0.0058097, -0.023148) } else { 0.20833 } } else { 0.0 };
                    off = reach * first;
                }
                self.crush.set((v3 as f32).mul_add(0.0019608, off));
                let v = v3.clamp(0, 255);
                if v >= 4 {
                    self.f21.make(true, plus_fc(PlusKind::Crush, v) as f64, res_hp(v) as f64, false, rd);
                } else {
                    self.f21.make(true, 60.0, 0.0, false, rd);
                }
            }
            k => {
                let off = if have { (mod_depth(v5) * first) as i32 } else { 0 };
                let s = if k == PlusKind::Lpf { v3 - off } else { v3 + off };
                let v = ((s as i16) as i32).clamp(0, 255);
                match k {
                    PlusKind::Bpf => {
                        self.bpf.q = 0.65f32 as f64;
                        self.bpf.set_fc(plus_fc(k, v));
                        self.bpf.fb = 0.7;
                    }
                    PlusKind::Hpf => {
                        if v >= 4 {
                            self.f21.make(true, plus_fc(k, v) as f64, res_hp(v) as f64, false, rd);
                        } else {
                            self.f21.make(true, 60.0, 0.0, false, rd);
                        }
                    }
                    _ => {
                        if v >= 4 {
                            self.f21.make(false, plus_fc(k, v) as f64, res_lp(v) as f64, false, rd);
                        } else {
                            self.f21.make(false, 20000.0, 1.5, false, rd);
                        }
                    }
                }
            }
        }
        if self.single {
            self.rmx.mix = v4 as f32 * 0.0039216;
        }
    }

    /// The start of a call of `len` frames with `p`.
    pub fn call(&mut self, len: usize, p: &PlusPar) {
        self.params(p);
        self.left = len;
        self.pos = 0;
        self.plen = self.piece.min(len).max(1);
        if self.single {
            // (The echoes run after the pieces, on ECHO MIX as the last piece left it.)
            let mut s = self.sh[1];
            s.advance(len as i32);
            self.rmx.mix = s.cur as f32 * 0.0039216;
        }
        self.rmx.call(len);
    }

    /// One frame of the call: the effect's output (the dry share, the filtered sound, echoes).
    #[inline]
    pub fn next(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.pos == 0 && self.kind == PlusKind::Crush {
            self.crush.begin();
        }
        let g = self.gate.take();
        let (mut wl, mut wr) = (l * g, r * g);
        match self.kind {
            PlusKind::Bpf => {
                wl = self.bpf.run(0, wl);
                wr = self.bpf.run(1, wr);
            }
            PlusKind::Hpf | PlusKind::Lpf => {
                wl = self.f21.run(0, wl);
                wr = self.f21.run(1, wr);
            }
            PlusKind::Crush => {
                (wl, wr) = self.crush.run(wl, wr);
                wl = self.f21.run(0, wl);
                wr = self.f21.run(1, wr);
            }
        }
        let ol = (l * self.dry) + (wl * self.wet);
        let or = (r * self.dry) + (wr * self.wet);
        wl *= self.wet;
        wr *= self.wet;
        self.pos += 1;
        if self.pos >= self.plen {
            let n = self.plen;
            for s in self.sh.iter_mut() {
                s.advance(n as i32);
            }
            self.update(n);
            self.left = self.left.saturating_sub(n);
            self.pos = 0;
            self.plen = if self.left == 0 { self.piece } else { self.piece.min(self.left) };
        }
        let (al, ar) = (self.hp[0].run(0, wl as f64) as f32, self.hp[0].run(1, wr as f64) as f32);
        let hl = self.hp[1].run(0, al as f64) as f32;
        let hr = self.hp[1].run(1, ar as f64) as f32;
        let (el, er) = self.rmx.next(hl, hr);
        (ol + el, or + er)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f32 = 48000.0;

    fn par(main: f32, echo: f32, modu: f32) -> PlusPar {
        PlusPar { on: true, main, echo, modu, rev: false, freeze: false, beat_ms: 500.0 }
    }

    /// Run `f` over `x` (stereo interleaved) in 480-frame calls.
    fn run(f: &mut FxPlusEcho, p: &PlusPar, x: &mut [f32]) {
        for c in x.chunks_mut(960) {
            f.call(c.len() / 2, p);
            for k in 0..c.len() / 2 {
                let (a, b) = f.next(c[k * 2], c[k * 2 + 1]);
                c[k * 2] = a;
                c[k * 2 + 1] = b;
            }
        }
    }

    #[test]
    fn plus_laws_follow_the_traced_numbers() {
        // `updateFilterCoef`: BPF dry 1, wet 1 - g^3; HPF g^3 / (1 - g^3) x (0.012 (255 - v) + 0.7)
        // from 231; LPF dry 1 - 0.035714 (v - 4), wet twice that share; CRUSH g^3 / 1 - g^3.
        let g = 127.0f32.mul_add(-0.0039841, 1.0159);
        assert!((plus_gains(PlusKind::Bpf, 127).1 - (1.0 - g * g * g)).abs() < 1e-6);
        assert_eq!(plus_gains(PlusKind::Bpf, 2), (1.0, 0.0));
        let (d, w) = plus_gains(PlusKind::Hpf, 240);
        let g = 240.0f32.mul_add(-0.0039841, 1.0159);
        assert!((d - g * g * g).abs() < 1e-6 && (w - (1.0 - g * g * g) * (15.0 * 0.012 + 0.7)).abs() < 1e-6);
        assert_eq!(plus_gains(PlusKind::Lpf, 3), (1.0, 0.0));
        let (d, w) = plus_gains(PlusKind::Lpf, 18);
        assert!((d - (1.0 - 14.0 * 0.035714)).abs() < 1e-6 && (w - 28.0 * 0.035714).abs() < 1e-6);
        assert_eq!(plus_gains(PlusKind::Lpf, 100), (0.0, 2.0));
        assert!((plus_gains(PlusKind::Lpf, 255).1 - 1.0).abs() < 1e-6);
        // Cutoffs: BPF 150 Hz .. 8.7 kHz, HPF 60 Hz .. 9 kHz, LPF 20 kHz .. 96 Hz, CRUSH 60 Hz .. 4 kHz.
        assert!((plus_fc(PlusKind::Bpf, 255) - 8700.0).abs() < 30.0 && plus_fc(PlusKind::Bpf, 3) == 150.0);
        assert!((plus_fc(PlusKind::Hpf, 255) - 9000.0).abs() < 30.0);
        assert!((plus_fc(PlusKind::Lpf, 255) - 96.0).abs() < 1.0);
        assert!((plus_fc(PlusKind::Crush, 255) - 4000.0).abs() < 20.0);
        assert!((res_hp(255) - 1.4).abs() < 1e-6 && res_hp(4) < 0.1 && (res_lp(3) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn bpf_coefficients_match_the_iir_port() {
        // `calcCoefBpf2ndOrder` is also `crate::fx_core::iir_bpf2` (NOISE): the same numbers.
        let mut f = IirBpf::new(48000);
        f.q = 0.65f32 as f64;
        f.set_fc(1000.0);
        let c = crate::fx_core::iir_bpf2(1000.0, 0.65f32 as f64, 48000.0);
        assert_eq!([f.c[0], f.c[2], f.c[3], f.c[4]], [c[0], c[3], c[2], c[4]]);
    }

    #[test]
    fn knob_at_zero_gives_the_dry_sound() {
        // Main knob 0 (step 0): BPF / HPF / CRUSH g = 1, LPF v <= 3: dry 1, wet 0. What goes
        // into the echoes is the filtered sound x 0, so the output is the input exactly.
        for k in [PlusKind::Bpf, PlusKind::Hpf, PlusKind::Lpf, PlusKind::Crush] {
            for single in [false, true] {
                let mut f = FxPlusEcho::new(single);
                f.setup(k, R);
                let p = par(0.0, 0.5, 0.0);
                f.make_selected(&p);
                f.turn_on();
                let mut x: Vec<f32> = (0..48000).map(|t| ((t as f32) * 0.013).sin() * 0.5).collect();
                let y0 = x.clone();
                run(&mut f, &p, &mut x);
                assert!(x.iter().zip(&y0).all(|(a, b)| a == b), "{k:?} single {single}");
            }
        }
    }

    #[test]
    fn rmx_weights_times_and_feedback() {
        // Single mode, ECHO MIX 0.3: the quarter-beat core gets 2 - 1.2 = 0.8 of the input, the
        // half-beat one 0.2; the input goes in at 0.7, the repeats come back at 0.75. Beat 500 ms
        // = 24000 frames at 48 kHz.
        let mut m = Rmx4::new(true);
        m.setup(48000);
        m.beat_ms = 500.0;
        m.mix = 0.3;
        m.reset();
        m.turn_on();
        let n = 72000;
        let mut y = vec![0.0f32; n];
        for (k, c) in (0..n).collect::<Vec<_>>().chunks(480).enumerate() {
            m.call(c.len());
            for (i, &t) in c.iter().enumerate() {
                let x = if t == 1000 { 1.0 } else { 0.0 };
                y[k * 480 + i] = m.next(x, x).0;
            }
        }
        let at = |t: usize| y[t];
        assert!((at(1000 + 6000) - 0.7 * 0.8).abs() < 1e-5, "{}", at(7000));
        assert!((at(1000 + 12000) - (0.7 * 0.8 * 0.75 + 0.7 * 0.2)).abs() < 1e-5, "{}", at(13000));
        assert!((at(1000 + 18000) - 0.7 * 0.8 * 0.75 * 0.75).abs() < 1e-5, "{}", at(19000));
        assert!(at(1000 + 3000).abs() < 1e-9);
    }

    #[test]
    fn freeze_holds_the_repeats_once_a_beat_is_held() {
        // Multi-like half-beat core: FREEZE after the line holds a beat (24000 frames): input gate
        // shut, feedback 1, so a repeat keeps its level.
        let mut m = Rmx4::new(false);
        m.setup(48000);
        m.beat_ms = 500.0;
        m.reset();
        m.turn_on();
        m.freeze = true;
        let n = 96000;
        let mut y = vec![0.0f32; n];
        for (k, c) in (0..n).collect::<Vec<_>>().chunks(480).enumerate() {
            m.call(c.len());
            assert_eq!(m.filled, (k + 1) * 480 >= 24000);
            for (i, &t) in c.iter().enumerate() {
                let x = if t == 1000 || t == 30000 { 1.0 } else { 0.0 };
                y[k * 480 + i] = m.next(x, x).0;
            }
        }
        assert!((y[13000] - 0.7).abs() < 1e-5);
        assert!((y[25000] - 0.7 * 0.75).abs() < 1e-5, "before it holds: {}", y[25000]);
        assert!((y[37000] - 0.7 * 0.75).abs() < 1e-5, "held at feedback 1: {}", y[37000]);
        assert!((y[85000] - 0.7 * 0.75).abs() < 1e-5);
        assert!(y[42000].abs() < 1e-5, "new input shut out: {}", y[42000]);
    }

    #[test]
    fn crush_holds_whole_runs() {
        // CRUSH at step 255: hold 255 x 0.0019608 = 0.5 ms = 24 frames: a sample is kept for 25.
        let mut c = Crush::new(48000);
        c.set(255.0 * 0.0019608);
        c.make_selected();
        let out: Vec<f32> = (0..100).map(|t| c.run(t as f32, 0.0).0).collect();
        assert_eq!(out[0], 0.0);
        assert_eq!(out[24], 0.0);
        assert_eq!(out[25], 25.0);
        assert_eq!(out[49], 25.0);
        assert_eq!(out[50], 50.0);
    }

    #[test]
    fn hpf_echo_keeps_g3_of_a_steady_level() {
        // HPF ECHO at the knob's centre (step 127): the dry share g^3 = 0.13259; the filtered
        // sound and its echoes (two 284 Hz high-passes) carry no DC, so a steady 1.0 settles there.
        let mut f = FxPlusEcho::new(false);
        f.setup(PlusKind::Hpf, R);
        let p = par(0.5, 0.5, 0.0);
        f.make_selected(&p);
        f.turn_on();
        let mut x = vec![1.0f32; 2 * 96000];
        run(&mut f, &p, &mut x);
        let g = 127.0f32.mul_add(-0.0039841, 1.0159);
        assert!((x[2 * 95000] - g * g * g).abs() < 1e-3, "{}", x[2 * 95000]);
    }

    /// The cost of making the effect selected on the audio thread (single mode: the eight
    /// cores' lines are cleared). `cargo test --release fx_plus_select_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn fx_plus_select_cost() {
        let mut f = FxPlusEcho::new(true);
        let p = par(0.5, 0.5, 0.5);
        let t = std::time::Instant::now();
        for _ in 0..20 {
            f.setup(PlusKind::Bpf, R);
            f.make_selected(&p);
        }
        println!("fx plus select: {:.0} us each (setup + make_selected, single mode)", t.elapsed().as_secs_f64() / 20.0 * 1e6);
    }
}
