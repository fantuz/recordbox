//! SPIRAL's core, rekordbox's `SpiralCore` (constructor 0x102a40114, `init` 0x102a40878,
//! `processEffectConcrete` 0x102a418a0 (disassembly), `caluclateLimitterThres` 0x102a41dec,
//! `checkOutputGain` 0x102a420d4, `onCalledTurnOn` 0x102a42274). Beat FX SPIRAL
//! (`BeatFxSpiral`: mode 0, no shifter; `SingleBeatFxSpiral`: mode 0 with the shifter) and FX+
//! SPIRAL UP / DOWN (`RmxSpiral`: modes 1 / 2, the shifter in single mode) run it.
//!
//! A block goes like this. The limit is worked out from the block's input first. The input goes
//! through a 12 kHz low-pass (Q 0.707). Each frame the line gives the delayed sound `d`; `d` plus
//! the input is clipped at the limit, goes through a 150 Hz high-pass (Q 0.3) and a 12.1 kHz
//! low-pass (Q 0.7), is multiplied by the feedback and written back. The output is `d` itself
//! through a 10 kHz low-pass (Q 0.3) and the output gain (a `GainSmoother` of 2 s, made at 1).
//! With the shifter (single mode) the output is pitch shifted (the caller's
//! `rbxfrm::PitchShifter`, run on every block, also at 0 %) and goes through a band-pass at
//! 1 kHz, Q 0.02; SPIRAL UP / DOWN in multi mode take that band-pass without the shifter.
//!
//! The limit: mode 0 keeps the peak of the mono input since the turn-on and clips at 0.6 of
//! it; modes 1 and 2 follow the block's running peak through a one-pole average of
//! max(120, 0.375 x beat) frames, taken in pieces of (int)(0.000666 x fs) frames, and the limit
//! only rises. `checkOutputGain` keeps a one-second average of each block's output peak, which a
//! later turn-on starts from (mode 0: peak = 3 x it, modes 1 / 2: limit = 1.5 x it, at most 1).
//! Its decays after 100 ms of quiet output change a value the next block works out again, so
//! they do not change the sound and are kept only for the bookkeeping.

use crate::fx_core::{bq, iir_bpf2, iir_hpf2, iir_lpf2};
use crate::fx_echo::GainSm;

#[derive(Clone, Copy)]
pub struct SpiralCore {
    rate: f32,
    /// `SPIRAL_MODE` (a1+624): 0 Beat FX SPIRAL, 1 SPIRAL UP, 2 SPIRAL DOWN.
    mode: u8,
    /// Made with the pitch shifter (a1+704).
    pub pitch: bool,
    /// Input peak (588), loop limit (592), output average (596) and the frames in it (600),
    /// quiet output frames (604), the averaged peak of modes 1 / 2 (608).
    pk: f32,
    pub thr: f32,
    avg: f32,
    avg_n: i32,
    quiet: i32,
    sm: f32,
    /// This block's output peak and quiet frames, kept as the frames run.
    blk_max: f32,
    /// Made selected and not turned on since (776): the next turn-on runs `init`.
    fresh: bool,
    /// Sections: 0 input low-pass (632), 1 loop high-pass (648), 2 loop low-pass (640),
    /// 3 output low-pass (656), 4 output band-pass (664).
    c: [[f32; 5]; 5],
    z: [[[f32; 2]; 2]; 5],
    /// Output gain (688): `GainSmoother(rate, 2000 ms, 1.0)`.
    og: GainSm,
}

impl SpiralCore {
    pub fn new(rate: f32, mode: u8, pitch: bool) -> SpiralCore {
        let mut s = SpiralCore { rate: 0.0, mode, pitch, pk: 0.0, thr: 0.0, avg: 0.0, avg_n: 0, quiet: 0, sm: 0.0, blk_max: 0.0, fresh: true, c: [[0.0; 5]; 5], z: [[[0.0; 2]; 2]; 5], og: GainSm::new(1.0) };
        s.make(rate, mode, pitch);
        s
    }

    /// The constructor's filters for `rate` (the cutoffs no caller changes), then `init`.
    fn make(&mut self, rate: f32, mode: u8, pitch: bool) {
        let r = rate as f64;
        self.rate = rate;
        self.mode = mode;
        self.pitch = pitch;
        self.c = [
            iir_lpf2(12000.0, 0.7070000171661377, r),
            iir_hpf2(150.0, 0.30000001192092896, r),
            iir_lpf2(12100.0, 0.699999988079071, r),
            iir_lpf2(10000.0, 0.30000001192092896, r),
            iir_bpf2(1000.0, 0.02, r),
        ];
        self.og = GainSm::new(1.0);
        self.fresh = true;
        self.init();
    }

    /// Made again when the rate or the effect changes.
    pub fn setup(&mut self, rate: f32, mode: u8, pitch: bool) {
        if (rate - self.rate).abs() > 0.5 || mode != self.mode || pitch != self.pitch {
            self.make(rate, mode, pitch);
        }
    }

    /// `init`: the filters' memories and the limit's values cleared (the caller clears the
    /// line and readies its shifter).
    fn init(&mut self) {
        self.z = [[[0.0; 2]; 2]; 5];
        self.pk = 0.0;
        self.thr = 0.0;
        self.avg = 0.0;
        self.avg_n = 0;
        self.quiet = 0;
        self.sm = 0.0;
    }

    /// `onCalledMakeSelected`.
    pub fn selected(&mut self) {
        self.fresh = true;
    }

    /// `onCalledTurnOn`. True when it ran `init` (the first turn-on after the effect was
    /// selected): the caller then clears the line and its shifter.
    pub fn turn_on(&mut self) -> bool {
        let fresh = self.fresh;
        if fresh {
            self.init();
            self.fresh = false;
        }
        if self.mode != 0 {
            self.thr = (self.avg * 1.5).min(1.0);
            self.quiet = 0;
        } else {
            self.pk = (self.avg * 3.0).min(1.0);
        }
        fresh
    }

    /// The output gain (parameter 6) the 2 s smoother moves to.
    pub fn set_out_gain(&mut self, g: f32) {
        self.og.set(g, self.rate, 2000.0);
    }

    /// Output band-pass in use: with the shifter, or SPIRAL UP / DOWN.
    pub fn uses_bpf(&self) -> bool {
        self.pitch || self.mode != 0
    }

    /// `caluclateLimitterThres` over the block's input (what the core is given), `n` frames,
    /// at the beat (ms).
    pub fn limit_block<I: Iterator<Item = (f32, f32)>>(&mut self, input: I, n: usize, beat_ms: f64) {
        self.blk_max = 0.0;
        if self.mode == 0 {
            let mut pk = self.pk;
            for (l, r) in input.take(n) {
                pk = pk.max(((l + r) * 0.5).abs());
            }
            self.pk = pk;
            self.thr = pk * 0.6;
            return;
        }
        let rate = self.rate as i32;
        self.pk = 0.0;
        if n == 0 {
            return;
        }
        let piece = ((rate as f32) * 0.000666) as i32;
        let w = ((beat_ms * 0.000375 * rate as f64) as i32).max(120);
        let (wm1, inv) = ((w - 1) as f32, 1.0 / w as f32);
        let mut left = n as i32;
        let mut pk = 0.0f32;
        let mut it = input;
        if piece > 0 {
            while left > piece {
                for _ in 0..piece {
                    let (l, r) = it.next().unwrap_or((0.0, 0.0));
                    pk = pk.max(((l + r) * 0.5).abs());
                    self.sm = (pk + self.sm * wm1) * inv;
                    if self.thr < self.sm {
                        self.thr = self.sm;
                    }
                }
                left -= piece;
            }
        }
        for _ in 0..left.max(0) {
            let (l, r) = it.next().unwrap_or((0.0, 0.0));
            pk = pk.max(((l + r) * 0.5).abs());
        }
        self.pk = pk;
    }

    /// The input low-pass (on what goes into the loop).
    #[inline]
    pub fn pre(&mut self, x: [f32; 2]) -> [f32; 2] {
        [bq(&self.c[0], &mut self.z[0][0], x[0]), bq(&self.c[0], &mut self.z[0][1], x[1])]
    }

    /// What is written to the line from the delayed sound `d` and the (low-passed) input `x`.
    #[inline]
    pub fn feed(&mut self, d: [f32; 2], x: [f32; 2], fb: f32) -> [f32; 2] {
        let th = self.thr.max(0.0);
        let mut w = [0.0f32; 2];
        for c in 0..2 {
            let a = d[c] + x[c];
            let a = if a > th { th } else if a < -th { -th } else { a };
            let a = bq(&self.c[1], &mut self.z[1][c], a);
            let a = bq(&self.c[2], &mut self.z[2][c], a);
            w[c] = a * fb;
        }
        w
    }

    /// The output from the delayed sound: low-pass and output gain.
    #[inline]
    pub fn out(&mut self, d: [f32; 2]) -> [f32; 2] {
        let g = self.og.next();
        let y = [bq(&self.c[3], &mut self.z[3][0], d[0]) * g, bq(&self.c[3], &mut self.z[3][1], d[1]) * g];
        let m = ((y[0] + y[1]) * 0.5).abs();
        let quiet_at = if self.mode != 0 { self.thr * 0.001 } else { 0.001 };
        if m < quiet_at {
            self.quiet = self.quiet.saturating_add(1);
        } else {
            self.quiet = 0;
        }
        self.blk_max = self.blk_max.max(m);
        y
    }

    /// The output band-pass (after the shifter).
    #[inline]
    pub fn bpf(&mut self, y: [f32; 2]) -> [f32; 2] {
        [bq(&self.c[4], &mut self.z[4][0], y[0]), bq(&self.c[4], &mut self.z[4][1], y[1])]
    }

    /// `checkOutputGain` at the end of a block of `n` frames.
    pub fn end_block(&mut self, n: usize) {
        let n = n as i32;
        let rate = self.rate as i32;
        let lim = ((rate as f32 * 100.0) * 0.001) as i32;
        if self.quiet > lim {
            if self.mode != 0 {
                self.pk *= 0.89125;
                self.quiet -= lim;
            } else {
                self.thr *= 0.89125;
                self.quiet = 0;
            }
        }
        let full = ((1000 * rate) as f64 * 0.001) as i32;
        let (old_w, total) = if self.avg_n >= full {
            self.avg_n = full;
            (full - n, full)
        } else {
            let t = self.avg_n + n;
            let o = self.avg_n;
            self.avg_n = t;
            (o, t)
        };
        if total > 0 {
            self.avg = (self.blk_max * n as f32 + self.avg * old_w as f32) / total as f32;
        }
        self.blk_max = 0.0;
    }
}

/// The PITCH knob's percent: `SingleBeatFxSpiral::updateParameter` (k x 200 - 100) and
/// `RmxSpiral::updateParameter` (k x +-200) go through an integer `LinearParameter`
/// (`getParameter(int*)` truncates) into `PitchShifter::setPitchParameter(int)`.
pub fn pitch_percent(v: f32) -> i32 {
    (v as i32).clamp(-200, 200)
}

/// SPIRAL UP / DOWN's four echoes, rekordbox's `RmxFourOrderEcho` (0x102a3eeb8,
/// `setParameterKnob` 0x102a3e9d0) on `MultiOrderEcho` (constructor 0x10299a4dc, `addEcho`
/// 0x10299a694, `processBlock` 0x10299af04, `setSamplingRateHz` 0x10299b36c). Each echo takes
/// the input at its input gain plus what it gave in the block before (at the same place in the
/// block) x 0.75, delays that by 0.25 / 0.5 / 0.75 / 1 x 500 ms (the base time is the
/// constructor's 500 ms; nothing sets it again) and gives it at its output gain (0.70795, set by
/// `RmxSpiral`); the four are summed. So the feedback waits a block as well: the repeats after
/// the first come every delay + one audio buffer. The gains are `GainSmoother`s of 3.3 ms.
pub struct FourEcho {
    rate: f32,
    /// The lines are the caller's (four parts of `len` frames, stereo interleaved); their
    /// write place, and per echo the block before's output x feedback.
    len: usize,
    w: usize,
    fbk: Vec<f32>,
    cap: usize,
    ig: [GainSm; 4],
    og: [GainSm; 4],
    delay: [f32; 4],
}

/// The base time (`MultiOrderEcho` +20, 500 ms) and each echo's share of it.
const BASE_MS: f64 = 500.0;
const RATIOS: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

impl FourEcho {
    /// For lines of `len` frames each and audio buffers up to `cap` frames (made off the audio
    /// thread).
    pub fn new(len: usize, cap: usize) -> FourEcho {
        let mut e = FourEcho { rate: 0.0, len, w: 0, fbk: vec![0.0; cap * 2 * 4], cap, ig: [GainSm::new(1.0); 4], og: [GainSm::new(0.70795); 4], delay: [0.0; 4] };
        e.setup(48000.0);
        e
    }

    pub fn empty() -> FourEcho {
        FourEcho { rate: 48000.0, len: 2, w: 0, fbk: Vec::new(), cap: 1, ig: [GainSm::new(1.0); 4], og: [GainSm::new(0.70795); 4], delay: [0.0; 4] }
    }

    /// `setSamplingRateHz`: the delays for `rate`, the lines cleared.
    pub fn setup(&mut self, rate: f32) {
        self.rate = rate;
        let base = (rate as f64 * 0.00100000005 * BASE_MS) as f32;
        // (Whole frames: how `NormalDelayBuffer` reads a fraction is not traced.)
        self.delay = RATIOS.map(|k| (base * k).trunc().min((self.len - 2) as f32));
        self.clear();
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// `MultiOrderEcho::init` (the caller clears its lines).
    pub fn clear(&mut self) {
        self.fbk.iter_mut().for_each(|v| *v = 0.0);
        self.w = 0;
    }

    /// `RmxSpiral::updateParameter`: single mode sets the input gains from ECHO MIX
    /// (`setParameterKnob`); multi mode the second echo alone at 0.7.
    pub fn set_mix(&mut self, multi: bool, k: f32) {
        let g = if multi {
            [0.0, 0.7, 0.0, 0.0]
        } else if k < 0.25 {
            [k * 2.8, 0.0, 0.0, 0.0]
        } else if k < 0.5 {
            [1.4 - k * 2.8, k * 2.8 - 0.7, 0.0, 0.0]
        } else if k < 0.75 {
            [0.0, 2.1 - k * 2.8, k * 2.8 - 1.4, 0.0]
        } else {
            [0.0, 0.0, 2.8 - k * 2.8, k * 2.8 - 2.1]
        };
        for i in 0..4 {
            self.ig[i].set(g[i], self.rate, 3.3);
        }
    }

    /// Frame `j` of the block: the four echoes of `x`, summed (`line`: the four lines).
    #[inline]
    pub fn next(&mut self, line: &mut [f32], x: [f32; 2], j: usize) -> [f32; 2] {
        if self.fbk.is_empty() || line.len() < self.len * 8 {
            return [0.0; 2];
        }
        let j = j % self.cap;
        let mut out = [0.0f32; 2];
        for i in 0..4 {
            let g = self.ig[i].next();
            let o = self.og[i].next();
            let base = i * self.len;
            let fb = (i * self.cap + j) * 2;
            // Read the delayed frame first (the line holds block + delay), then write.
            let d = self.delay[i];
            let k = d as usize;
            let t = d - k as f32;
            let at = |n: usize| (base + (self.w + self.len - n) % self.len) * 2;
            let (a, b) = (at(k), at(k + 1));
            for c in 0..2 {
                let y = (line[a + c] + (line[b + c] - line[a + c]) * t) * o;
                let wv = x[c] * g + self.fbk[fb + c];
                line[(base + self.w) * 2 + c] = wv.clamp(-64.0, 64.0);
                self.fbk[fb + c] = y * 0.75;
                out[c] += y;
            }
        }
        self.w = (self.w + 1) % self.len;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mode 0 clips the loop at 0.6 of the input's peak; the loop is low- and high-passed
    /// and multiplied by the feedback; the output gain moves over 2 s.
    #[test]
    fn spiral_core_limit_and_loop_are_rekordbox_s() {
        let rate = 48000.0;
        let mut s = SpiralCore::new(rate, 0, false);
        assert!(s.turn_on(), "first turn-on runs init");
        assert!(!s.turn_on(), "the second does not");
        let block: Vec<(f32, f32)> = (0..512).map(|i| if i == 100 { (0.5, 0.3) } else { (0.0, 0.0) }).collect();
        s.limit_block(block.iter().copied(), 512, 500.0);
        assert!((s.pk - 0.4).abs() < 1e-6 && (s.thr - 0.24).abs() < 1e-6, "peak of the mono input, limit 0.6 x it: {} {}", s.pk, s.thr);
        // A big sum is clipped at the limit before the filters: a DC step settles under it.
        let mut w = [0.0f32; 2];
        for _ in 0..48000 {
            w = s.feed([2.0, 2.0], [0.0, 0.0], 1.0);
        }
        assert!(w[0].abs() < 0.01, "the 150 Hz high-pass takes the DC out: {}", w[0]);
        let mut s2 = SpiralCore::new(rate, 0, false);
        s2.thr = 0.24;
        let first = s2.feed([2.0, 2.0], [0.0, 0.0], 0.5)[0];
        let h = iir_hpf2(150.0, 0.30000001192092896, 48000.0);
        let l = iir_lpf2(12100.0, 0.699999988079071, 48000.0);
        assert!((first - 0.24 * h[0] * l[0] * 0.5).abs() < 1e-7, "clip, high-pass, low-pass, feedback: {first}");
        // Output gain: 2 s to the new value.
        s.set_out_gain(0.5);
        for _ in 0..47999 {
            s.og.next();
        }
        assert!(s.og.cur > 0.75 && s.og.cur < 0.752, "halfway after 1 s: {}", s.og.cur);
    }

    /// Modes 1 / 2: the limit follows the averaged running peak and only rises; the one-second
    /// output average sets the next turn-on's limit (x 1.5).
    #[test]
    fn spiral_up_down_limit_follows_the_averaged_peak() {
        let rate = 48000.0;
        let mut s = SpiralCore::new(rate, 1, true);
        s.turn_on();
        let n = 512;
        s.limit_block(std::iter::repeat((0.5, 0.5)).take(n), n, 500.0);
        // 500 ms beat: w = 0.375 x 500 x 48 = 9000 frames; 16 pieces of 31 frames averaged.
        let w = 9000.0f32;
        let mut sm = 0.0f32;
        for _ in 0..(16 * 31) {
            sm = (0.5 + sm * (w - 1.0)) / w;
        }
        assert!((s.thr - sm).abs() < 1e-6, "limit {} want {}", s.thr, sm);
        let th = s.thr;
        s.limit_block(std::iter::repeat((0.0, 0.0)).take(n), n, 500.0);
        assert!(s.thr >= th, "the limit does not fall");
        for _ in 0..n {
            s.out([0.2, 0.2]);
        }
        s.end_block(n);
        assert!(s.avg > 0.0 && s.avg < 0.2);
        let a = s.avg;
        s.turn_on();
        assert!((s.thr - (1.5 * a).min(1.0)).abs() < 1e-7);
        assert_eq!((pitch_percent(0.509 * 200.0 - 100.0), pitch_percent(-0.9), pitch_percent(250.0)), (1, 0, 200));
    }

    /// `RmxFourOrderEcho`: echoes at 125 / 250 / 375 / 500 ms whatever the tempo, at 0.70795,
    /// the feedback (0.75) a block late; ECHO MIX crossfades the input between neighbours.
    #[test]
    fn four_echo_times_gains_and_late_feedback() {
        let rate = 48000.0;
        let mut e = FourEcho::new(48000, 512);
        let mut line = vec![0.0f32; 48000 * 8];
        e.setup(rate);
        e.set_mix(false, 0.0);
        // (The input gains start at 1 and reach their targets in 3.3 ms; then knob 1.0: the
        // fourth echo alone at 0.7.)
        e.set_mix(false, 1.0);
        let block = 512;
        let mut out = Vec::new();
        for j in 0..(48000 * 2) {
            let x = if j == 1000 { 1.0 } else { 0.0 };
            out.push(e.next(&mut line, [x, x], j % block)[0]);
        }
        // Only the 500 ms echo has input gain 0.7 by frame 1000.
        let at = 1000 + 24000;
        assert!((out[at] - 0.7 * 0.70795).abs() < 1e-5, "first repeat {}", out[at]);
        // The repeat is fed back at the same place one block later: 24000 + 512 frames on.
        let again = at + 24000 + 512;
        assert!((out[again] - 0.7 * 0.70795 * 0.75 * 0.70795).abs() < 1e-5, "second repeat {}", out[again]);
        assert!(out[at + 24000].abs() < 1e-9, "nothing a plain delay later");
    }
}
