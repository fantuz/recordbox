//! Beat FX FILTER and PHASER as rekordbox runs them, ported from the 7.2.19 arm64 binary:
//! `BeatFxFilter` (0x1029ce614 .. 0x1029cf89c) drives `LFOFilterCore` (0x1029d5f74 ..
//! 0x1029d6ec0) through two `LFOConnector`s, each the sum of a main and a second `LFO`;
//! `BeatFxPhaser` (0x102a0bf10 .. 0x102a0ccf8) drives `PhaserCore` (0x102a0eaa0 .. 0x102a0f2d8)
//! with one `LFO` a side. Both take the beat quantizer's restarts through an output gate
//! (`SignalGate`, 3.3333 ms): a restart, a new PHASE / TONE or (PHASER) a new stage count shuts
//! it, and only once it is shut do the LFOs jump, take the new period or turn over.
//!
//! Nothing here allocates: the audio thread runs it.

use crate::fx_lfo::{Lfo, Sf, Wave};

const EPS: f32 = 1.1920929e-7;

fn new_lfo(rate: f32, min: f32, max: f32) -> Lfo {
    let mut l = Lfo::new(rate, Wave::TRI);
    l.set_amplitude_range(true, min, max);
    l.set_smoothing_time(3.3333);
    l.set_period_time(10.0, true);
    l
}

fn opened_gate(rate: f32) -> Sf {
    let mut g = crate::fx_rev::new_gate(rate);
    g.reset(1.0);
    g
}

/// One value from an LFO.
#[inline]
fn next(l: &mut Lfo) -> f32 {
    let mut v = [0.0f32; 1];
    l.get_values(&mut v);
    v[0]
}

/// `BeatFxFilter::FilterParameter` (the part after the mix): resonance, the main LFOs' range
/// and heads, the second LFOs' period and half-range, PHASE (REVERSE LFO) and TONE (high-pass).
#[derive(Clone, Copy, Debug)]
pub struct FilterPar {
    pub reso: f32,
    pub max: [f32; 2],
    pub min: [f32; 2],
    pub head: [f32; 2],
    pub p2: [f32; 2],
    pub a2: [f32; 2],
    pub rev: bool,
    pub hp: bool,
}

impl FilterPar {
    /// `MultiBeatFxFilter::updateParameter` 0x1029cf9d0: the main LFOs 0 .. 1 from 0, the
    /// second ones still (100 ms, no reach).
    pub fn multi(reso: f32) -> FilterPar {
        FilterPar { reso, max: [1.0; 2], min: [0.0; 2], head: [0.0; 2], p2: [100.0; 2], a2: [0.0; 2], rev: false, hp: false }
    }

    /// `SingleBeatFxFilter::updateParameter` 0x1029cfd84 with KnobPara2Assign 0 (as the single
    /// layer makes it, `RekordboxDJSingleFxLayer::createEffectBase`): the second knob `k2` adds
    /// a faster triangle to one side, the left side's turned left, the right side's turned
    /// right. Its period is the beat divided by 1, 2, 4, 8 or 16 across the knob's reach (at
    /// least 20 ms), its half-range 0.3 (0.375 x reach + 0.075 past 0.6), and none unless the
    /// main LFO's period `main_ms` is at least twice it.
    pub fn single(reso: f32, k2: f32, bpm: f32, main_ms: f32, tone: bool, rev: bool) -> FilterPar {
        let mut p = FilterPar { reso, rev, hp: tone, ..FilterPar::multi(reso) };
        if !(0.48627..=0.51373).contains(&k2) {
            let v = crate::fx::through_value(k2);
            let beat = 60000.0 / bpm.clamp(1.0, 300.0);
            let per = (if v < 0.2 { beat } else if v < 0.4 { beat * 0.5 } else if v < 0.6 { beat * 0.25 } else if v < 0.8 { beat * 0.125 } else { beat * 0.0625 }).max(20.0);
            let amp = if v < 0.6 { 0.3 } else { v * 0.375 + 0.075 };
            let amp = if main_ms >= per + per { amp } else { 0.0 };
            p.p2 = [per; 2];
            p.a2 = if k2 < 0.48627 { [amp, 0.0] } else { [0.0, amp] };
        }
        p
    }
}

/// `BeatFxFilter` with its `LFOFilterCore`: the LFOs, the output gate, the pending changes, the
/// resonance smoothers and the six first-order sections a side.
#[derive(Clone, Copy, Debug)]
pub struct BeatFilter {
    /// Main L, main R, second L, second R.
    pub lfo: [Lfo; 4],
    gate: Sf,
    /// `f680` restart asked, `f681` gate shutting for it, `f784` jump, `f785` period waiting
    /// (`f788`), `f786` PHASE waiting (`f792`), `f787` TONE waiting (`f793`).
    restart: bool,
    closing: bool,
    jump: bool,
    pend: bool,
    pend_ms: f32,
    rev_pend: bool,
    rev_val: bool,
    tone_pend: bool,
    tone_val: bool,
    /// The core's filter type (`f344`: high-pass), its resonance and its smoothers (3.3333 ms).
    pub hp: bool,
    reso_t: f32,
    reso: [Sf; 2],
    /// `processFilter`'s state a side: the input and the six sections' outputs.
    z: [[f32; 7]; 2],
}

impl BeatFilter {
    /// The constructor (0x1029ce614): triangles, the main ones 0 .. 1 and the second -0.1 .. 0.1,
    /// 3.3333 ms smoothing, a 10 ms period; the gate open; a low-pass core.
    pub fn new(rate: f32) -> BeatFilter {
        let mut lfo = [new_lfo(rate, 0.0, 1.0), new_lfo(rate, 0.0, 1.0), new_lfo(rate, -0.1, 0.1), new_lfo(rate, -0.1, 0.1)];
        lfo.iter_mut().for_each(|l| l.reset());
        BeatFilter { lfo, gate: opened_gate(rate), restart: false, closing: false, jump: false, pend: false, pend_ms: 10.0, rev_pend: false, rev_val: false, tone_pend: false, tone_val: false, hp: false, reso_t: 0.0, reso: [Sf::new(rate); 2], z: [[0.0; 7]; 2] }
    }

    /// `LFOFilterCore::reset` 0x1029d6540: the smoothers at the resonance, the sections cleared.
    fn core_reset(&mut self) {
        let t = self.reso_t;
        self.reso.iter_mut().for_each(|s| s.reset(t));
        self.z = [[0.0; 7]; 2];
    }

    /// `updateAndSetParameter` 0x1029ced2c.
    pub fn update(&mut self, p: &FilterPar) {
        self.reso_t = p.reso;
        for c in 0..2 {
            self.lfo[c].set_amplitude_range(true, p.min[c], p.max[c]);
        }
        if (self.lfo[0].head - p.head[0]).abs() >= EPS || (self.lfo[1].head - p.head[1]).abs() >= EPS {
            self.lfo[0].set_head(p.head[0]);
            self.lfo[1].set_head(p.head[1]);
        }
        for c in 0..2 {
            let l = &mut self.lfo[2 + c];
            if (p.p2[c] - l.period_ms).abs() >= EPS {
                l.set_period_time(p.p2[c], true);
            }
            l.set_amplitude_range(true, -p.a2[c], p.a2[c]);
        }
        if p.rev != self.lfo[0].reversed() {
            self.restart = true;
            self.rev_pend = true;
            self.rev_val = p.rev;
        }
        if p.hp != self.hp {
            self.restart = true;
            self.tone_pend = true;
            self.tone_val = p.hp;
        }
    }

    /// `onCalledMakeSelected` 0x1029cf7c0.
    pub fn make_selected(&mut self, p: &FilterPar) {
        self.update(p);
        if self.pend {
            self.pend = false;
            let ms = self.pend_ms;
            self.lfo[0].set_period_time(ms, false);
            self.lfo[1].set_period_time(ms, false);
        }
        self.lfo.iter_mut().for_each(|l| l.reset());
        self.restart = false;
        self.closing = false;
        self.gate.reset(1.0);
        self.core_reset();
    }

    /// The beat quantizer's calls before a stretch: `setTimeUntilNextPlayHead` 0x1029cf89c (`until`
    /// ms; the main LFOs re-timed, kept within 10 ms .. 32 s), `setEffectBeatTimeWithPlayHead`
    /// 0x1029cf868 (`beat` ms: the period for the next restart, and a restart), `playHead` /
    /// `onCalledTurnOn` (`play`: a restart).
    pub fn calls(&mut self, until: f32, beat: f32, play: bool) {
        if until > 0.0 {
            let old = self.lfo[0].period_ms;
            self.lfo[0].set_remaining_period_time(until, true);
            self.lfo[1].set_remaining_period_time(until, true);
            let p = self.lfo[0].period_ms;
            if !(10.0..=32000.0).contains(&p) {
                self.lfo[0].set_period_time(old, true);
                self.lfo[1].set_period_time(old, true);
            } else {
                self.pend_ms = p;
            }
        }
        if beat > 0.0 {
            self.jump = true;
            self.pend = true;
            self.pend_ms = if beat >= 10.0 { beat.min(32000.0) } else { 10.0 };
            self.restart = true;
        }
        if play {
            self.jump = true;
            self.restart = true;
        }
    }

    /// `processEffectConcrete` 0x1029cf584 at the start of a stretch, after `update`: a restart
    /// shuts the gate (one asked for while it shuts is dropped).
    pub fn begin(&mut self) {
        if self.closing {
            self.restart = false;
        } else if self.restart {
            self.restart = false;
            self.closing = true;
            self.gate.set_target(0.0);
        }
        let t = self.reso_t;
        self.reso.iter_mut().for_each(|s| s.set_target(t));
    }

    /// One frame of `LFOFilterCore::processEffect` 0x1029d6688 and the gate: `x` in, the gated
    /// filter out. `tab`: the core's coefficient tables for its type and rate (main, resonance
    /// b0 over 129 steps), `rate` the rate as an integer.
    #[inline]
    pub fn frame(&mut self, x: [f32; 2], tab: &[[f32; 129]; 2], rate: u32) -> [f32; 2] {
        let mut o = [0.0f32; 2];
        let hp = self.hp;
        for c in 0..2 {
            // (`LFOConnector`: the two LFOs summed, held to 0 .. 1.)
            let a = next(&mut self.lfo[c]);
            let b = next(&mut self.lfo[2 + c]);
            let t = (a + b).max(0.0).min(1.0);
            let i0 = (t * 128.0) as usize;
            let i1 = if i0 >= 128 { 128 } else { i0 + 1 };
            let fr = t * 128.0 - (t * 128.0).trunc();
            let b0 = |tb: &[f32; 129]| {
                let v = tb[i0.min(128)] + (tb[i1] - tb[i0.min(128)]) * fr;
                let v = if v.abs() >= EPS && v <= 0.0 { 0.0 } else { v };
                if (v - 1.0).abs() >= EPS && v >= 1.0 { 1.0 } else { v }
            };
            let (mb, rb) = (b0(&tab[0]), b0(&tab[1]));
            // First-order sections from their b0: low-pass b1 = b0, a1 = 2 b0 - 1; high-pass
            // b1 = -b0, a1 = 1 - 2 b0. The resonance path is the other type.
            let (m1, ma, r1, ra) = if hp { (-mb, 1.0 - 2.0 * mb, rb, 2.0 * rb - 1.0) } else { (mb, 2.0 * mb - 1.0, -rb, 1.0 - 2.0 * rb) };
            // `calcResonanceGain` 0x1029d6b94: the smoothed resonance; the low-pass takes it
            // away at the bottom of the sweep (44.1 / 48 kHz: none under 0.1, a cubic up to 0.2;
            // 96 kHz: 1 - 0.095 (1 - 4t)^24 under 0.25).
            let mut rg = self.reso[c].take();
            if !hp {
                rg *= match rate {
                    44100 | 48000 => {
                        if t < 0.1 {
                            0.0001
                        } else if t < 0.2 {
                            let u = t * -10.0 + 2.0;
                            ((u * u) * (-0.9999 * u) + 1.0).min(1.0).max(0.0001)
                        } else {
                            1.0
                        }
                    }
                    96000 => {
                        if t < 0.25 {
                            let u = (1.0 - t * 4.0) - (t * 4.0) * (1.0 - t * 4.0);
                            let s = (u * u) * (u * u);
                            (s * s) * (-0.095 * s) + 1.0
                        } else {
                            1.0
                        }
                    }
                    _ => 1.0,
                };
            }
            // The resonance path's level: 0.8 for the high-pass, 0.4 for the low-pass at 44.1 /
            // 48 kHz, 0.8 at 96 kHz, none at other rates.
            let a7 = if hp { 0.8 } else { match rate { 44100 | 48000 => 0.4, 96000 => 0.8, _ => 0.0 } };
            // `processFilter` 0x1029d6ec0.
            let s = &mut self.z[c];
            let y0 = x[c] + rg * s[6];
            let o1 = s[0] * m1 + y0 * mb - s[1] * ma;
            s[0] = y0;
            let s1 = s[1];
            s[1] = o1;
            let o2 = s1 * m1 + o1 * mb - s[2] * ma;
            let s2 = s[2];
            s[2] = o2;
            let o3 = o2 * mb - (s[3] * ma - s2 * m1);
            let s3 = s[3];
            s[3] = o3;
            let o4 = s3 * r1 + o3 * rb - s[4] * ra;
            let s4 = s[4];
            s[4] = o4;
            let o5 = o4 * rb - (s[5] * ra - s4 * r1);
            let s5 = s[5];
            s[5] = o5;
            let o6 = o5 * rb - (s[6] * ra - s5 * r1);
            s[6] = o6;
            o[c] = o3 + rg * a7 * o6;
        }
        let g = self.gate.take();
        if self.closing && !self.gate.on && self.gate.cur.abs() < EPS {
            self.shut();
        }
        [o[0] * g, o[1] * g]
    }

    /// The gate is shut: open it again, then the waiting period, PHASE, jump and TONE.
    fn shut(&mut self) {
        self.closing = false;
        self.gate.set_target(1.0);
        if self.pend {
            self.pend = false;
            let ms = self.pend_ms;
            self.lfo[0].set_period_time(ms, false);
            self.lfo[1].set_period_time(ms, false);
        }
        if self.rev_pend {
            self.rev_pend = false;
            let r = self.rev_val;
            self.lfo.iter_mut().for_each(|l| l.reverse_amplitude(r, false));
        }
        if self.jump {
            self.jump = false;
            self.lfo.iter_mut().for_each(|l| l.jump_to_head(false));
        }
        if self.tone_pend {
            self.tone_pend = false;
            self.hp = self.tone_val;
            self.core_reset();
        }
    }
}

/// `BeatFxPhaser::PhaserParameter` (the part after the mix).
#[derive(Clone, Copy, Debug)]
pub struct PhaserPar {
    pub fb: f32,
    pub stages: [i32; 2],
    pub head: [f32; 2],
    pub rev: bool,
    pub peak: f32,
    pub bottom: f32,
}

/// PHASER's sweep in both modes (`MultiBeatFxPhaser` / `SingleBeatFxPhaser::updateParameter`,
/// KnobPara2Assign 0): 299.96 Hz .. 8058.92 Hz.
pub const PHASER_PEAK: f32 = f32::from_bits(0x45FB_D75F);
pub const PHASER_BOTTOM: f32 = f32::from_bits(0x4395_FB11);

impl PhaserPar {
    pub fn new(fb: f32, stages: (usize, usize), rev: bool) -> PhaserPar {
        PhaserPar { fb, stages: [stages.0 as i32, stages.1 as i32], head: [0.0; 2], rev, peak: PHASER_PEAK, bottom: PHASER_BOTTOM }
    }
}

/// `PhaserCore`'s pre-warped frequency: tan(pi f / fs) x fs / pi (in double, as rekordbox).
fn warp(f: f32, rate: f32) -> f32 {
    let fs = (rate as i32) as f64;
    ((f as f64 * 3.14159265 / fs).tan() * (0.318309886 * fs)) as f32
}

/// `BeatFxPhaser` with its `PhaserCore`.
#[derive(Clone, Copy, Debug)]
pub struct BeatPhaser {
    pub lfo: [Lfo; 2],
    gate: Sf,
    /// `f632` restart asked, `f633` gate shutting, `f736` jump, `f737` period waiting (`f740`),
    /// `f738` PHASE waiting (`f744`), `f739` stages waiting (`f748`, `f752`).
    restart: bool,
    closing: bool,
    jump: bool,
    pend: bool,
    pend_ms: f32,
    rev_pend: bool,
    rev_val: bool,
    st_pend: bool,
    pend_st: [i32; 2],
    /// The core: its stage parameters and the counts in use (both 2 when made), the feedback
    /// (target and smoothers, 3.3333 ms), the peak and bottom parameters and their pre-warped
    /// smoothers (50 ms; one smoother read for both sides in turn), the stages' state, each
    /// side's last output and its feedback low-pass, that low-pass's coefficient.
    st_par: [i32; 2],
    st: [usize; 2],
    fb_t: f32,
    fb: [Sf; 2],
    peak: f32,
    bottom: f32,
    pk_par: f32,
    bt_par: f32,
    pk: Sf,
    bt: Sf,
    ap: [[f32; 60]; 2],
    last: [f32; 2],
    fbs: [f32; 2],
    c: f32,
    rate: f32,
    /// This core call's smoother values a side (`processEffect` reads the same smoother for
    /// the left side and then the right), the frames left in the stretch and in the call.
    pk_lr: [Sf; 2],
    bt_lr: [Sf; 2],
    seg_left: u32,
    call_left: u32,
}

impl BeatPhaser {
    /// The constructor (0x102a0bf10): triangles 0 .. 1, a 10 ms period, 3.3333 ms smoothing,
    /// the heads half way (the sweep's top); the gate open; the core (0x102a0eaa0) at 2 stages,
    /// its range at 100 Hz.
    pub fn new(rate: f32) -> BeatPhaser {
        let mut lfo = [new_lfo(rate, 0.0, 1.0), new_lfo(rate, 0.0, 1.0)];
        lfo.iter_mut().for_each(|l| l.set_head(0.5));
        let mut pk = Sf::new(rate);
        pk.set_time(50.0);
        pk.reset(warp(100.0, rate));
        let c = (-62831.8531f64 / ((rate as i32) as f64)).exp() as f32;
        let fb = [Sf::new(rate); 2];
        BeatPhaser { lfo, gate: opened_gate(rate), restart: false, closing: false, jump: false, pend: false, pend_ms: 10.0, rev_pend: false, rev_val: false, st_pend: false, pend_st: [2, 2], st_par: [2, 2], st: [2, 2], fb_t: 0.0, fb, peak: 100.0, bottom: 100.0, pk_par: 100.0, bt_par: 100.0, pk, bt: pk, ap: [[0.0; 60]; 2], last: [0.0; 2], fbs: [0.0; 2], c, rate, pk_lr: [pk; 2], bt_lr: [pk; 2], seg_left: 0, call_left: 0 }
    }

    /// `PhaserCore::reset` 0x102a0f28c.
    fn core_reset(&mut self) {
        self.ap = [[0.0; 60]; 2];
        self.last = [0.0; 2];
        self.fbs = [0.0; 2];
    }

    fn close(&mut self) {
        if !self.closing {
            self.restart = false;
            self.closing = true;
            self.gate.set_target(0.0);
        }
    }

    /// `updateAndSetParameter` 0x102a0c7f0: a new stage count or PHASE waits for the gate (and
    /// shuts it); the heads are moved half a turn.
    pub fn update(&mut self, p: &PhaserPar) {
        self.fb_t = p.fb;
        let st = [p.stages[0].clamp(2, 60), p.stages[1].clamp(2, 60)];
        if st != self.st_par {
            self.st_pend = true;
            self.pend_st = st;
            self.close();
        }
        if p.rev != self.lfo[0].reversed() {
            self.rev_pend = true;
            self.rev_val = p.rev;
            self.close();
        }
        let half = |h: f32| {
            let v = h - 0.5;
            if v.abs() >= EPS && h + 0.5 <= 1.0 { h + 0.5 } else { v }
        };
        let (h0, h1) = (half(p.head[0]), half(p.head[1]));
        if (h0 - self.lfo[0].head).abs() >= EPS || (h1 - self.lfo[1].head).abs() >= EPS {
            self.lfo[0].set_head(h0);
            self.lfo[1].set_head(h1);
        }
        self.peak = p.peak.clamp(100.0, 15000.0);
        self.bottom = p.bottom.clamp(100.0, 15000.0);
    }

    /// `onCalledMakeSelected` 0x102a0cbf0.
    pub fn make_selected(&mut self, p: &PhaserPar) {
        self.update(p);
        if self.pend {
            self.pend = false;
            let ms = self.pend_ms;
            self.lfo.iter_mut().for_each(|l| l.set_period_time(ms, false));
        }
        self.lfo.iter_mut().for_each(|l| l.reset());
        self.restart = false;
        self.closing = false;
        self.gate.reset(1.0);
        self.jump = false;
        self.pend = false;
        self.rev_pend = false;
        self.st_pend = false;
        self.pend_ms = self.lfo[0].period_ms;
        self.core_reset();
    }

    /// The beat quantizer's calls before a stretch: `setTimeUntilNextPlayHead` 0x102a0ccf8 (the
    /// left LFO re-timed first; within 10 ms .. 32 s the right one follows, else the left one
    /// goes back), `setEffectBeatTimeWithPlayHead` 0x102a0ccac (a restart, and the period for
    /// it if it differs), `playHead` / `onCalledTurnOn` (a restart).
    pub fn calls(&mut self, until: f32, beat: f32, play: bool) {
        if until > 0.0 {
            let old = self.lfo[0].period_ms;
            self.lfo[0].set_remaining_period_time(until, true);
            let p = self.lfo[0].period_ms;
            if !(10.0..=32000.0).contains(&p) {
                self.lfo[0].set_period_time(old, true);
            } else {
                self.lfo[1].set_remaining_period_time(until, true);
                self.pend_ms = p;
            }
        }
        if beat > 0.0 {
            let ms = if beat >= 10.0 { beat.min(32000.0) } else { 10.0 };
            self.jump = true;
            self.restart = true;
            if (ms - self.lfo[0].period_ms).abs() >= EPS {
                self.pend = true;
                self.pend_ms = ms;
            }
        }
        if play {
            self.jump = true;
            self.restart = true;
        }
    }

    /// `processEffectConcrete` 0x102a0c9d8 at the start of a stretch of `len` frames, after
    /// `update`: a restart shuts the gate (one asked for while it shuts is dropped).
    pub fn begin(&mut self, len: usize) {
        if self.closing {
            self.restart = false;
        } else if self.restart {
            self.restart = false;
            self.closing = true;
            self.gate.set_target(0.0);
        }
        self.seg_left = len as u32;
        self.call_left = 0;
    }

    /// The start of a `PhaserCore::processEffect` call of `n` frames: the parameters taken in,
    /// a changed stage count clears the stages between the old and the new count.
    fn core_call(&mut self, n: u32) {
        let r = self.rate;
        if (self.pk_par - self.peak).abs() >= EPS {
            self.pk_par = self.peak;
            self.pk.set_target(warp(self.peak, r));
        }
        if (self.bt_par - self.bottom).abs() >= EPS {
            self.bt_par = self.bottom;
            self.bt.set_target(warp(self.bottom, r));
        }
        let t = self.fb_t;
        self.fb.iter_mut().for_each(|s| s.set_target(t));
        for c in 0..2 {
            let want = self.st_par[c] as usize;
            if self.st[c] != want {
                let (lo, hi) = (want.min(self.st[c]), want.max(self.st[c]));
                self.ap[c][lo..hi].iter_mut().for_each(|v| *v = 0.0);
                self.st[c] = want;
            }
        }
        // (The left side reads the next n values of each range smoother, the right side the n
        // after them.)
        self.pk_lr[0] = self.pk;
        self.bt_lr[0] = self.bt;
        for _ in 0..n {
            self.pk.take();
            self.bt.take();
        }
        self.pk_lr[1] = self.pk;
        self.bt_lr[1] = self.bt;
        for _ in 0..n {
            self.pk.take();
            self.bt.take();
        }
        self.call_left = n;
    }

    /// One frame of `PhaserCore::processEffect` 0x102a0f2d8 and the gate: `x` in, the gated
    /// all-pass chain out.
    #[inline]
    pub fn frame(&mut self, x: [f32; 2]) -> [f32; 2] {
        if self.call_left == 0 {
            // (`processEffectConcrete`: while the gate shuts, a call runs to the gate's end.)
            let left = self.seg_left.max(1);
            let n = if self.closing { left.min((self.gate.left.max(1)) as u32) } else { left };
            self.core_call(n);
        }
        let k = 6.2831855f32 / ((self.rate as i32) as f32);
        let mut o = [0.0f32; 2];
        for c in 0..2 {
            let v = next(&mut self.lfo[c]);
            let (pk, bt) = (self.pk_lr[c].take(), self.bt_lr[c].take());
            // The all-pass coefficient: the pre-warped frequency moves from the bottom to the
            // peak with the cube of the LFO.
            let w = (bt + v * v * v * (pk - bt)) * k;
            let a = (2.0 - w) / (w + 2.0);
            // The feedback, eased off at the bottom of the sweep: fb (1 - 0.1 (1 - v)^8).
            let g = (1.0 - v) - v * (1.0 - v);
            let g = g * g;
            let fbv = self.fb[c].take();
            let fb = fbv + (g * g) * -0.1 * fbv;
            // The output fed back through a one-pole (exp(-2 pi 10 kHz / fs) on the new value).
            let f = self.c * self.last[c] + self.fbs[c] * (1.0 - self.c);
            self.fbs[c] = f;
            let mut y = x[c] + fb * f;
            for z in self.ap[c][..self.st[c]].iter_mut() {
                let s = y + *z * a;
                y = *z - s * a;
                *z = s;
            }
            self.last[c] = y;
            o[c] = y;
        }
        let g = self.gate.take();
        self.seg_left = self.seg_left.saturating_sub(1);
        self.call_left = self.call_left.saturating_sub(1);
        if self.closing && !self.gate.on && self.gate.cur.abs() < EPS {
            self.shut();
            self.call_left = 0;
        }
        [o[0] * g, o[1] * g]
    }

    /// The gate is shut: open it, then jump, the waiting period, PHASE and stages, each
    /// clearing the core.
    fn shut(&mut self) {
        self.closing = false;
        self.gate.set_target(1.0);
        if self.jump {
            self.jump = false;
            self.lfo.iter_mut().for_each(|l| l.jump_to_head(false));
            self.core_reset();
        }
        if self.pend {
            self.pend = false;
            let ms = self.pend_ms;
            self.lfo.iter_mut().for_each(|l| l.set_period_time(ms, false));
            self.core_reset();
        }
        if self.rev_pend {
            let r = self.rev_val;
            self.lfo.iter_mut().for_each(|l| l.reverse_amplitude(r, false));
            self.rev_pend = false;
            self.core_reset();
        }
        if self.st_pend {
            self.st_par = self.pend_st;
            self.st_pend = false;
            self.core_reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_single_lfo_amp_adds_a_second_triangle_to_one_side() {
        // `SingleBeatFxFilter::updateParameter`, KnobPara2Assign 0, 120 BPM (beat 500 ms):
        // the left end gives the left side a 31.25 ms triangle of half-range 0.45; a little
        // right of the centre the right side a 500 ms one of 0.3, but only while the main LFO's
        // period is at least twice it; the centre none.
        let p = FilterPar::single(0.72, 0.0, 120.0, 2000.0, false, false);
        assert_eq!((p.p2, p.a2), ([31.25; 2], [0.45, 0.0]));
        let p = FilterPar::single(0.72, 0.6, 120.0, 1000.0, false, false);
        assert_eq!((p.p2, p.a2), ([500.0; 2], [0.0, 0.3]));
        let p = FilterPar::single(0.72, 0.6, 120.0, 999.0, false, false);
        assert_eq!(p.a2, [0.0, 0.0]);
        let p = FilterPar::single(0.72, 0.5, 120.0, 2000.0, true, true);
        assert_eq!((p.p2, p.a2, p.max, p.min, p.hp, p.rev), ([100.0; 2], [0.0; 2], [1.0; 2], [0.0; 2], true, true));
    }

    #[test]
    fn filter_restart_and_tone_wait_for_the_gate() {
        // A restart shuts the gate over 3.3333 ms (159 frames at 48 kHz); TONE asked for in the
        // meantime is taken only once it is shut, and then the LFOs are at their heads (0).
        let tab = [[0.5f32; 129]; 2];
        let mut f = BeatFilter::new(48000.0);
        let p = FilterPar::multi(0.0);
        f.make_selected(&p);
        f.calls(0.0, 500.0, true);
        f.update(&p);
        f.begin();
        for _ in 0..100 {
            f.frame([0.0; 2], &tab, 48000);
        }
        let hp = FilterPar { hp: true, ..p };
        f.update(&hp);
        f.begin();
        assert!(!f.hp && f.closing);
        for _ in 0..59 {
            f.frame([0.0; 2], &tab, 48000);
        }
        assert!(f.hp && !f.closing, "TONE once the gate is shut");
        assert_eq!((f.lfo[0].pos, f.lfo[0].period_ms), (0, 500.0));
    }

    #[test]
    fn phaser_restarts_at_the_top_of_its_sweep() {
        // `BeatFxPhaser`: the heads are half way (the triangle's top, the LFO at 1 = 8058.92 Hz)
        // and the jump waits for the gate; the stage count (2 when made) becomes 6 then too.
        let mut ph = BeatPhaser::new(48000.0);
        let p = PhaserPar::new(0.0, (6, 6), false);
        ph.make_selected(&p);
        assert_eq!(ph.st_par, [2, 2]);
        ph.calls(0.0, 500.0, true);
        ph.update(&p);
        ph.begin(4800);
        for _ in 0..159 {
            ph.frame([0.0; 2]);
        }
        assert_eq!(ph.st_par, [6, 6]);
        let mut v = [0.0f32; 1];
        let mut l = ph.lfo[0];
        l.get_values(&mut v);
        assert!((v[0] - 1.0).abs() < 1e-3, "{}", v[0]);
        assert_eq!(l.period_ms, 500.0);
        assert!((PHASER_PEAK - 8058.92).abs() < 0.01 && (PHASER_BOTTOM - 299.96).abs() < 0.01);
        // The range glides from 100 Hz over 50 ms (the smoother read twice a call).
        assert!(ph.pk.cur > warp(100.0, 48000.0));
    }

    #[test]
    fn phaser_feedback_is_low_passed_and_eased_at_the_bottom() {
        // `PhaserCore::onSamplingRateChanged`: the feedback one-pole's coefficient on the last
        // output is exp(-62831.85 / fs) (0.2702 at 48 kHz).
        let ph = BeatPhaser::new(48000.0);
        assert!((ph.c - (-62831.8531f64 / 48000.0).exp() as f32).abs() < 1e-7 && (ph.c - 0.27018).abs() < 1e-4);
    }
}
