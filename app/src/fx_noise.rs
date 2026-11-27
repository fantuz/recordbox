//! FX+ NOISE, rekordbox's `FxPlusNoiseEcho` (constructor 0x1029e2c8c (disassembly),
//! `processEffectConcrete` 0x1029e4424, `updateAndSetParameter` 0x1029e396c,
//! `updateFilterCoef` 0x1029e3618, `onCalledTurnOn` 0x1029e49cc), its `WhiteNoiseOSC`
//! (0x1028c78cc / `processSound` 0x1028c7958) and `CrushCore` (0x1029abdcc /
//! `processEffectConcrete` 0x1029abf34, disassembly).
//!
//! A block: stereo white noise (rekordbox's own generator, 2^-31 per step) at a gain moving over
//! 3.3 ms to noise_gain(main knob) x VOL x 0.501187 (halved with CRUSH); single mode's CRUSH
//! holds the noise (not the input) a sample at a time for 0.4 ms; the noise goes through a
//! band-pass (Q 0.65, its output fed back at 0.7) whose centre is set every 16 frames from the
//! main knob and MOD (both `SmoothShiftInteger`s, a step per 0.33333 ms) and a triangle `LFO`
//! (200 - 0.70588 x MOD ms); then the noise gate (3.3333 ms, open while on). An `EchoCore` takes
//! the noise through its input gate (half a beat, 10 .. 4000 ms, feedback 0.5); the output is
//! noise + 0.35 x echo. FREEZE (once the line holds the delay, while on) shuts the echo's input,
//! sets its feedback to 1 and gives the echo at full level instead of 0.35; the swaps go through
//! 3.3333 ms gates. The input sound is not in it: rekordbox mixes it at 1 / 1.

use crate::fx_core::{iir_bpf2, ShiftInt};
use crate::fx_echo::{EchoCore, GainSm};
use crate::fx_lfo::{Lfo, Sf, Wave};

/// `FxPlusNoiseEcho::GAIN_UP_COEF` (set by the file's static initialiser 0x1029e5810).
pub const GAIN_UP: f32 = f32::from_bits(0x3F00_4DCE);

pub struct NoiseFx {
    rate: f32,
    v7: u32,
    v5: u32,
    gain: GainSm,
    /// CRUSH: frames per hold (0.4 ms), the count and the held frame.
    hold: f32,
    crush: bool,
    cc: f32,
    held: [f32; 2],
    /// Band-pass: coefficients, per side x1, x2, y1, y2.
    bp: [f32; 5],
    bh: [[f32; 4]; 2],
    piece: i32,
    left: i32,
    knob: ShiftInt,
    modk: ShiftInt,
    lfo: Lfo,
    lv: [f32; 64],
    /// Gates: noise (608), echo input (768), echo at 0.35 (864), frozen echo (960).
    g_noise: Sf,
    g_in: Sf,
    g_part: Sf,
    g_frz: Sf,
    /// The line holds the delay (588) / frames counted (592) / at most (596).
    filled: bool,
    count: i32,
    count_max: i32,
    echo: Option<Box<EchoCore>>,
    pub delay_ms: f32,
}

fn gate(rate: f32, open: bool) -> Sf {
    let mut g = crate::fx_rev::new_gate(rate);
    g.reset(if open { 1.0 } else { 0.0 });
    g
}

impl NoiseFx {
    pub fn new(rate: f32) -> NoiseFx {
        let mut n = NoiseFx::empty();
        n.echo = Some(EchoCore::new());
        n.setup(rate);
        n
    }

    /// Without its echo line, for a slot that never runs.
    pub fn empty() -> NoiseFx {
        let rate = 48000.0;
        NoiseFx { rate: 0.0, v7: 0x6745_2301, v5: 0xEFCD_AB89, gain: GainSm::new(0.0), hold: 0.0, crush: false, cc: 0.0, held: [0.0; 2], bp: [0.0; 5], bh: [[0.0; 4]; 2], piece: 16, left: 0, knob: ShiftInt::new(127, 0.33333, rate), modk: ShiftInt::new(127, 0.33333, rate), lfo: Lfo::new(rate, Wave::TRI), lv: [0.0; 64], g_noise: gate(rate, false), g_in: gate(rate, false), g_part: gate(rate, false), g_frz: gate(rate, false), filled: false, count: 0, count_max: 1, echo: None, delay_ms: 250.0 }
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    pub fn ready(&self) -> bool {
        self.echo.is_some()
    }

    /// The constructor's state for `rate`.
    pub fn setup(&mut self, rate: f32) {
        self.rate = rate;
        self.v7 = 0x6745_2301;
        self.v5 = 0xEFCD_AB89;
        self.gain = GainSm::new(0.0);
        self.hold = 0.001f32 * 0.4 * rate;
        self.cc = 0.0;
        self.held = [0.0; 2];
        self.bh = [[0.0; 4]; 2];
        self.piece = ((rate * f32::from_bits(0x39AE_C33F)) as i32).clamp(1, 64);
        self.left = 0;
        self.knob = ShiftInt::new(127, 0.33333, rate);
        self.modk = ShiftInt::new(127, 0.33333, rate);
        self.lfo = Lfo::new(rate, Wave::TRI);
        self.lfo.set_amplitude_range(false, -1.0, 1.0);
        self.lfo.set_smoothing_time(3.3333);
        self.lfo.set_period_time(200.0, false);
        self.lfo.reset();
        self.g_noise = gate(rate, false);
        self.g_in = gate(rate, false);
        self.g_part = gate(rate, false);
        self.g_frz = gate(rate, false);
        self.count_max = (rate * 4.0).ceil() as i32;
        self.filled = false;
        self.count = 0;
        self.delay_ms = 250.0;
        if let Some(e) = self.echo.as_deref_mut() {
            e.setup(rate);
            e.reset([250.0; 2], [0.5; 2]);
        }
        self.coef(1);
    }

    /// `onCalledMakeSelected`: the filter's memories, the LFO and the echo cleared.
    pub fn selected(&mut self) {
        self.bh = [[0.0; 4]; 2];
        self.lfo.reset();
        if let Some(e) = self.echo.as_deref_mut() {
            e.reset([self.delay_ms; 2], [0.5; 2]);
        }
    }

    /// `onCalledTurnOn`.
    pub fn turn_on(&mut self) {
        self.filled = false;
        self.count = 0;
    }

    /// `updateFilterCoef` for a piece of `n` frames.
    fn coef(&mut self, n: i32) {
        let v4 = self.knob.cur;
        let v5 = self.modk.cur as f32;
        self.lfo.set_period_time(v5 * -0.70588 + 200.0, true);
        let m = v5 * 0.0039216;
        let depth = if m >= 0.015625 { if m < 0.15625 { v5 * 0.027887 - 0.11111 } else { 1.0 } } else { 0.0 };
        let mut add = 0;
        if n >= 1 {
            let k = if v4 <= 230 { 1.0 } else { (255 - v4) as f32 * 0.04 };
            let len = (n as usize).min(self.lv.len());
            let mut lv = [0.0f32; 64];
            self.lfo.get_values(&mut lv[..len]);
            self.lv = lv;
            add = (depth * k * (65.0 * lv[0])) as i32 as u16 as i16 as i32;
        }
        let v = (((add + v4) as u16 as i16) as i32).clamp(0, 255);
        let fc = crate::fx::noise_fc(v).max(5.0).min(0.49 * self.rate);
        self.bp = iir_bpf2(fc as f64, 0.6499999761581421, self.rate as f64);
    }

    /// `updateAndSetParameter` at a block's start: main knob (0 .. 1), VOL, MOD, CRUSH (single
    /// mode only), FREEZE, on, the beat (ms).
    pub fn begin(&mut self, level: f32, vol: f32, modk: f32, crush: Option<bool>, freeze: bool, on: bool, beat_ms: f32) {
        self.knob.set(((level * 255.9) as i32).clamp(0, 255));
        self.modk.set(((modk * 255.9) as i32).clamp(0, 255));
        let mut g = crate::fx::noise_gain(self.knob.cur) * vol * GAIN_UP;
        if crush == Some(true) {
            g *= 0.5;
        }
        self.gain.set(g, self.rate, 3.3);
        let half = beat_ms * 0.5;
        let ms = if half >= 10.0 { if half <= 4000.0 { half } else { 4000.0 } } else { 10.0 };
        let frozen = on && freeze && self.filled;
        self.delay_ms = ms;
        if let Some(e) = self.echo.as_deref_mut() {
            e.set([ms; 2], [if frozen { 1.0 } else { 0.5 }; 2]);
        }
        self.g_noise.set_target(if on { 1.0 } else { 0.0 });
        self.g_in.set_target(if frozen { 0.0 } else { 1.0 });
        self.g_part.set_target(if frozen { 0.0 } else { 1.0 });
        self.g_frz.set_target(if frozen { 1.0 } else { 0.0 });
        self.crush = crush == Some(true);
    }

    /// The echo's call for a block of `n` frames, and the filled count (`processEffectConcrete`
    /// counts the block before it is run).
    pub fn call(&mut self, n: usize) {
        if let Some(e) = self.echo.as_deref_mut() {
            e.call(n);
        }
        let was = self.count;
        self.filled = true;
        if was < self.count_max {
            let need = ((self.delay_ms * 0.001) * self.rate).ceil() as i32;
            self.count = was + n as i32;
            if self.count < need {
                self.filled = false;
            }
        }
    }

    /// One frame of the effect's output (the noise and its echo).
    #[inline]
    pub fn next(&mut self) -> [f32; 2] {
        if self.rate <= 0.0 {
            return [0.0; 2];
        }
        // `WhiteNoiseOSC::processSound`.
        let v8 = self.v7 ^ self.v5;
        let l = (self.v5 as i32 as f64 * 4.65661288e-10) as f32;
        let v10 = v8.wrapping_add(self.v5);
        self.v7 = v10 ^ v8;
        let r = (v10 as i32 as f64 * 4.65661288e-10) as f32;
        self.v5 = self.v7.wrapping_add(v10);
        let g = self.gain.next();
        let mut x = [l * g, r * g];
        if self.crush {
            // `CrushCore`: a new sample once the count is under 1, then held.
            if self.cc >= 1.0 {
                x = self.held;
                self.cc -= 1.0;
            } else {
                self.held = x;
                self.cc += self.hold;
            }
        }
        if self.left <= 0 {
            self.left = self.piece;
        }
        let mut y = [0.0f32; 2];
        for c in 0..2 {
            let h = &mut self.bh[c];
            let xin = x[c] + h[2] * 0.7;
            let o = xin * self.bp[0] + h[0] * self.bp[1] + h[1] * self.bp[2] - (h[2] * self.bp[3] + h[3] * self.bp[4]);
            h[1] = h[0];
            h[0] = xin;
            h[3] = h[2];
            h[2] = o;
            y[c] = o;
        }
        self.left -= 1;
        if self.left == 0 {
            self.knob.advance(self.piece);
            self.modk.advance(self.piece);
            self.coef(self.piece);
        }
        let gn = self.g_noise.take();
        let n = [y[0] * gn, y[1] * gn];
        let gi = self.g_in.take();
        let (el, er) = match self.echo.as_deref_mut() {
            Some(e) => e.next(n[0] * gi, n[1] * gi),
            None => (0.0, 0.0),
        };
        let (gp, gf) = (self.g_part.take(), self.g_frz.take());
        [n[0] + el * 0.35 * gp + el * gf, n[1] + er * 0.35 * gp + er * gf]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rekordbox's generator from its seeds, the 0.4 ms hold of CRUSH on the noise, the
    /// band-pass centre from the main knob.
    #[test]
    fn noise_generator_crush_and_filter_are_rekordbox_s() {
        let mut n = NoiseFx::new(48000.0);
        // First frame: L = 0xEFCDAB89 as i32 x 2^-31; R = (v8 + v5) as i32 x 2^-31.
        let (v7, v5) = (0x6745_2301u32, 0xEFCD_AB89u32);
        let v8 = v7 ^ v5;
        let r0 = (v8.wrapping_add(v5) as i32 as f64 * 4.65661288e-10) as f32;
        let l0 = (v5 as i32 as f64 * 4.65661288e-10) as f32;
        assert!(l0 < 0.0 && l0 > -0.2 && r0.abs() <= 1.0);
        assert!((GAIN_UP - 0.501187).abs() < 1e-5, "{}", GAIN_UP);
        assert_eq!(n.piece, 16, "(int)(48000 x 0x39AEC33F), in float");
        // Hold: 0.4 ms = 19.2 frames: a new sample, then 19 or 20 frames held.
        n.crush = true;
        n.gain = GainSm::new(1.0);
        let mut takes = Vec::new();
        for i in 0..200 {
            let before = n.cc;
            n.next();
            if before < 1.0 {
                takes.push(i);
            }
        }
        let gaps: Vec<i32> = takes.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|&g| g == 20 || g == 21), "{gaps:?}");
        // Main knob at 255 -> fc noise_fc(255) (no MOD).
        n.knob.reset(255);
        n.modk.reset(0);
        n.coef(16);
        let want = iir_bpf2(crate::fx::noise_fc(255).min(0.49 * 48000.0) as f64, 0.6499999761581421, 48000.0);
        assert_eq!(n.bp, want);
    }
}
