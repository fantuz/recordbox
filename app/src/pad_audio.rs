//! The audio thread's part of the pad modes: the SLICER's position rules and ACT. CENSR's
//! effects. One `DeckPads` sits in each deck's `PlayState`; `audio::render_deck_cue` calls
//! `position` before it reads a frame and `sound` on the frame it read. Nothing here
//! allocates or blocks: the interface sets the plain values and (for the censor) hands over
//! buffers it allocated itself.
//!
//! After rekordbox 7.2.19: `rbxfrm::SlicerBehavior` (`onPreRender` 0x102b54fd0, `doSelectUnit`
//! 0x102b520f4) and `rbxfrm::ActiveCensorBehavior` (`doActiveCensorIfNeeded` 0x102ae1e0c). The
//! four censor effects are recordbox's own small processors: rekordbox runs its PAD FX layer
//! (REV ROLL, TRANS) and its release FX layer (ECHO, V.BRAKE) there.

use std::sync::Arc;

/// The slicer as the audio thread sees it. Positions are source frames.
#[derive(Clone, Copy, Debug, Default)]
pub struct SlicerAudio {
    pub on: bool,
    /// SLICER LOOP: playback stays inside the domain.
    pub loop_mode: bool,
    /// The domain's nine boundaries.
    pub b: [f64; 9],
    /// The roll in force: playback loops from in to out.
    pub roll: Option<(f64, f64)>,
    /// A section waiting for its moment: when the position reaches `.0` it jumps to the
    /// section's start `.1` and rolls to `.2`.
    pub pending: Option<(f64, f64, f64)>,
    /// Where the track would be without the roll (the background position).
    pub bg: Option<f64>,
    /// The pads were let go: when the background position reaches this, playback goes back
    /// to it.
    pub ret_at: Option<f64>,
    /// How many times a section was started (jump or in place), for the interface and tests.
    pub starts: u32,
}

impl SlicerAudio {
    /// Called once per output frame, before the frame is read. `adv`: source frames the
    /// position moves per output frame.
    #[inline]
    pub fn position(&mut self, pos: &mut f64, adv: f64) {
        if let Some((at, a, b)) = self.pending {
            if *pos >= at {
                *pos = a + (*pos - at).max(0.0).min((b - a - 1.0).max(0.0));
                self.roll = Some((a, b));
                self.pending = None;
                self.starts = self.starts.wrapping_add(1);
            }
        }
        if let (Some(at), Some(bg)) = (self.ret_at, self.bg) {
            if bg >= at {
                *pos = bg;
                self.bg = None;
                self.ret_at = None;
                self.roll = None;
                self.pending = None;
            }
        }
        if let Some((a, b)) = self.roll {
            if *pos >= b && b > a {
                *pos = a + (*pos - b) % (b - a);
            }
        } else if self.loop_mode && self.pending.is_none() {
            // (`onPreRender`, mode 2: a position outside the domain is folded back into it.)
            let (a, b) = (self.b[0], self.b[8]);
            let len = b - a;
            if len > 1.0 {
                if *pos >= b {
                    *pos = a + (*pos - b) % len;
                } else if *pos < a {
                    *pos = b - (a - *pos) % len;
                }
            }
        }
        if let Some(bg) = self.bg.as_mut() {
            *bg += adv;
        }
    }

    /// A roll is sounding or waiting.
    pub fn rolling(&self) -> bool {
        self.roll.is_some() || self.pending.is_some()
    }
}

/// A censored region for the audio thread: in and out in source frames, and the effect's beat
/// in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CensorSpan {
    pub a: f64,
    pub b: f64,
    pub beat_secs: f32,
}

/// Seconds of sound the censor keeps (what REV ROLL plays backwards and ECHO repeats).
const RING_SECS: usize = 6;
const RING_RATE: usize = 48000;
const RING_FRAMES: usize = RING_SECS * RING_RATE;
/// The censor's effects are rekordbox's PAD FX and RELEASE FX layers at level 0.5
/// (`ActiveCensorView::prepareFxUnit`, `onActiveCensorEffectChanged`: REV ROLL = Beat FX 11,
/// TRANS = Beat FX 4, ECHO = RELEASE FX 1, V.BRAKE = RELEASE FX 0). Their laws are the FX
/// track's (`fx.rs`); the processors here follow `fx.rs`'s, which are private to it.
const LAYER_LEVEL: f32 = 0.5;
/// RELEASE FX ECHO: each repeat at 0.7 of the one before (`fx.rs` `Channel::process`).
const ECHO_FEEDBACK: f32 = 0.7;
/// The effects' gate and edge smoothing (`fx.rs` `SMOOTH_SECS`, rekordbox's 3.33 ms).
const EDGE_SECS: f32 = 0.003_333_3;

/// ACT. CENSR on one deck.
pub struct CensorAudio {
    /// ON/OFF.
    pub on: bool,
    /// Effect type: 0 REV ROLL, 1 TRANS, 2 ECHO, 3 V.BRAKE.
    pub kind: u8,
    pub regions: Arc<Vec<CensorSpan>>,
    /// The region the effect is running in (index into `regions`), for the interface.
    pub active: Option<usize>,
    /// Stereo ring of the deck's sound; None until the interface hands one over (`arm`).
    ring: Option<Box<[f32]>>,
    /// Next frame to write, and frames written so far (up to the ring's length).
    w: usize,
    filled: usize,
    /// Output frames since the region was entered, and the ring position at that moment.
    k: usize,
    entry: usize,
    /// V.BRAKE: where it reads (frames after `entry`) and its speed; TRANS's gate.
    brake_pos: f64,
    brake_speed: f32,
    gate: f32,
    /// 0 = dry, 1 = effect (fades over `EDGE_SECS`).
    mix: f32,
}

impl Default for CensorAudio {
    fn default() -> CensorAudio {
        CensorAudio { on: false, kind: 0, regions: Arc::new(Vec::new()), active: None, ring: None, w: 0, filled: 0, k: 0, entry: 0, brake_pos: 0.0, brake_speed: 1.0, gate: 0.0, mix: 0.0 }
    }
}

impl CensorAudio {
    /// Give the censor its buffer (called by the interface, which allocates it).
    pub fn arm(&mut self) {
        if self.ring.is_none() {
            self.ring = Some(vec![0.0f32; RING_FRAMES * 2].into_boxed_slice());
            self.w = 0;
            self.filled = 0;
        }
    }

    pub fn armed(&self) -> bool {
        self.ring.is_some()
    }

    /// Called once per output frame with the frame the deck read at `pos` (source frames).
    #[inline]
    pub fn sound(&mut self, pos: f64, l: &mut f32, r: &mut f32, rate: u32) {
        if !self.on || self.regions.is_empty() {
            self.active = None;
            self.mix = 0.0;
            return;
        }
        let Some(ring) = self.ring.as_mut() else { return };
        let n = ring.len() / 2;
        let inside = {
            let mut found = None;
            if let Some(i) = self.active {
                if let Some(x) = self.regions.get(i) {
                    if pos >= x.a && pos < x.b {
                        found = Some(i);
                    }
                }
            }
            found.or_else(|| self.regions.iter().position(|x| pos >= x.a && pos < x.b))
        };
        let edge = 1.0 / (EDGE_SECS * rate as f32).max(1.0);
        match inside {
            Some(i) => {
                if self.active != Some(i) {
                    self.active = Some(i);
                    self.k = 0;
                    self.entry = self.w;
                    self.brake_pos = 0.0;
                    self.brake_speed = 1.0;
                    self.gate = 0.0;
                }
                self.mix = (self.mix + edge).min(1.0);
            }
            None => {
                self.active = None;
                self.mix = (self.mix - edge).max(0.0);
            }
        }
        let (dl, dr) = (*l, *r);
        let Some(i) = self.active else {
            // Outside a region: remember the sound, pass it on.
            ring[self.w * 2] = dl;
            ring[self.w * 2 + 1] = dr;
            self.w = (self.w + 1) % n;
            self.filled = (self.filled + 1).min(n);
            return;
        };
        let beat = (self.regions[i].beat_secs.max(0.001) * rate as f32) as usize;
        let k = self.k;
        let at = |ring: &[f32], idx: usize| (ring[(idx % n) * 2], ring[(idx % n) * 2 + 1]);
        let (wl, wr) = match self.kind {
            // REV ROLL (ROLL family, `roll_mix` at 0.5 = the roll alone): the beat value
            // before the region played backwards and over again (32 beats by default, as much
            // as is remembered), with 3.33 ms fades at the splice.
            0 => {
                let span = beat.min(self.filled.saturating_sub(1)).max(1);
                let i = k % span;
                let back = i + 1;
                let edge = (i.min(span - 1 - i) as f32 / (EDGE_SECS * rate as f32)).min(1.0);
                let (wet_dry, wet) = crate::fx::roll_mix(LAYER_LEVEL);
                let (a, b) = at(ring, self.entry + n - back.min(n));
                (dl * wet_dry + a * edge * wet, dr * wet_dry + b * edge * wet)
            }
            // TRANS: a square gate once per beat value, open for `trans_duty` of it (never
            // under 5 ms), its edges moving at 1 / 3.33 ms (`fx.rs`).
            1 => {
                let period = beat.max(2);
                let duty = crate::fx::trans_duty(LAYER_LEVEL).clamp((0.005 * rate as f32 / period as f32).min(0.5), 1.0);
                let open = if ((k % period) as f32) < duty * period as f32 { 1.0 } else { 0.0 };
                let gk = 1.0 / (EDGE_SECS * rate as f32);
                self.gate += (open - self.gate).clamp(-gk, gk);
                (dl * self.gate, dr * self.gate)
            }
            // ECHO (RELEASE FX): the track is cut and the last beat value before the region
            // repeats, each repeat at 0.7 of the one before (at most 4 s long).
            2 => {
                let d = beat.min((4 * rate) as usize).min(self.filled.saturating_sub(1)).max(1);
                let rep = (k / d) as i32 + 1;
                let g = ECHO_FEEDBACK.powi(rep);
                let (el, er) = at(ring, self.entry + n - d.min(n) + k % d);
                (el * g, er * g)
            }
            // V.BRAKE (RELEASE FX): `brake_step` slows the sound in a straight line to a stop
            // over the beat value (under 1/8 at 0.20559 per second), the level following the
            // speed (x 8, at most 1); then silence.
            _ => {
                // (The region's own sound is kept, so that it can be read slower than it comes.)
                let wi = (self.entry + k) % n;
                ring[wi * 2] = dl;
                ring[wi * 2 + 1] = dr;
                let secs = (beat as f32 / rate as f32).clamp(0.010, 12.0);
                self.brake_speed = crate::fx::brake_step(self.brake_speed, secs, rate as f32);
                if self.brake_speed <= 0.0 || k + 2 >= n {
                    (0.0, 0.0)
                } else {
                    let p = self.brake_pos.min(k as f64);
                    let i0 = p.floor() as usize;
                    let f = (p - i0 as f64) as f32;
                    let (a0, a1) = at(ring, self.entry + i0);
                    let (b0, b1) = at(ring, self.entry + (i0 + 1).min(k));
                    self.brake_pos += self.brake_speed as f64;
                    let g = (self.brake_speed * 8.0).min(1.0);
                    ((a0 + (b0 - a0) * f) * g, (a1 + (b1 - a1) * f) * g)
                }
            }
        };
        self.k += 1;
        let m = self.mix;
        *l = dl * (1.0 - m) + wl * m;
        *r = dr * (1.0 - m) + wr * m;
    }
}

/// The pad engines of one deck (a field of `audio::PlayState`).
#[derive(Default)]
pub struct DeckPads {
    pub slicer: SlicerAudio,
    pub censor: CensorAudio,
}

impl DeckPads {
    /// Before a frame is read: the slicer's jumps.
    #[inline]
    pub fn position(&mut self, pos: &mut f64, adv: f64) {
        if self.slicer.on {
            self.slicer.position(pos, adv);
        }
    }

    /// The frame that was read at `pos`: the censor's effect.
    #[inline]
    pub fn sound(&mut self, pos: f64, l: &mut f32, r: &mut f32, rate: u32) {
        if self.censor.on {
            self.censor.sound(pos, l, r, rate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{render_deck, Decoded, PlayState};

    /// A deck whose sample value is its own frame number / 100000 (so a frame heard tells
    /// where it came from).
    fn deck(frames: usize) -> PlayState {
        let x: Vec<f32> = (0..frames).flat_map(|i| [i as f32 / 100000.0, i as f32 / 100000.0]).collect();
        let mut s = PlayState::default();
        s.audio = Some(Arc::new(Decoded { sr: 44100, samples: x, raw_frames: frames, damaged: 0, truncated: false }));
        s.playing = true;
        s.volume = 1.0;
        s.gain = 1.0;
        s
    }

    fn run(s: &mut PlayState, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        render_deck(s, &mut out, 2, 44100);
        out
    }

    fn domain(start: f64, unit: f64) -> [f64; 9] {
        std::array::from_fn(|k| start + k as f64 * unit)
    }

    #[test]
    fn slicer_roll_in_the_render_path() {
        // Sections of 1000 frames from 4000; playback is at 5500 (section 1).
        let mut s = deck(60000);
        s.pads.slicer = SlicerAudio { on: true, b: domain(4000.0, 1000.0), ..Default::default() };
        run(&mut s, 5500);
        assert!((s.pos - 5500.0).abs() < 1e-6);
        // Section 5 is pressed with a quantize boundary at 5600: nothing moves before it.
        s.pads.slicer.bg = Some(s.pos);
        s.pads.slicer.pending = Some((5600.0, 9000.0, 9250.0));
        let out = run(&mut s, 100);
        assert!((out[198] - 0.05599).abs() < 1e-5, "{}", out[198]);
        assert!(s.pads.slicer.roll.is_none());
        // At the boundary playback is at the section's start and rolls over 250 frames.
        let out = run(&mut s, 600);
        assert!((out[0] - 0.09).abs() < 1e-5, "{}", out[0]);
        assert!((out[2 * 249] - 0.09249).abs() < 1e-5);
        assert!((out[2 * 250] - 0.09).abs() < 1e-5, "{}", out[2 * 250]);
        assert_eq!(s.pads.slicer.roll, Some((9000.0, 9250.0)));
        assert_eq!(s.pads.slicer.starts, 1);
        // The background position ran on as if nothing had happened.
        assert!((s.pads.slicer.bg.unwrap() - 6200.0).abs() < 1e-6);
        // Let go: back to the background position when it reaches the next boundary (7000).
        s.pads.slicer.ret_at = Some(7000.0);
        run(&mut s, 799);
        assert!(s.pads.slicer.roll.is_some());
        let out = run(&mut s, 10);
        assert!(s.pads.slicer.roll.is_none() && s.pads.slicer.bg.is_none());
        assert!((out[2] - 0.07).abs() < 1e-5, "{}", out[2]);
        assert!((s.pos - 7009.0).abs() < 1e-6, "{}", s.pos);
    }

    #[test]
    fn slicer_unquantized_press_jumps_at_once_and_loop_mode_wraps() {
        let mut s = deck(60000);
        s.pads.slicer = SlicerAudio { on: true, b: domain(4000.0, 1000.0), ..Default::default() };
        run(&mut s, 4100);
        // Without quantize the boundary is the position itself.
        s.pads.slicer.bg = Some(s.pos);
        s.pads.slicer.pending = Some((s.pos, 6000.0, 6500.0));
        let out = run(&mut s, 10);
        assert!((out[0] - 0.06).abs() < 1e-5);
        // And the return is at once too.
        s.pads.slicer.ret_at = Some(0.0);
        let out = run(&mut s, 10);
        assert!((out[0] - 0.0411).abs() < 1e-5, "{}", out[0]);
        // SLICER LOOP: the end of the domain leads back to its start.
        s.pads.slicer.loop_mode = true;
        run(&mut s, 12000 - 4120);
        assert!((s.pos - 12000.0).abs() < 1e-6);
        let out = run(&mut s, 5);
        assert!((out[0] - 0.04).abs() < 1e-5, "{}", out[0]);
        assert!((s.pos - 4005.0).abs() < 1e-6);
        // A position before the domain is folded into it as well.
        s.pos = 3900.0;
        run(&mut s, 1);
        assert!((s.pos - 11901.0).abs() < 1e-6, "{}", s.pos);
        // SLICER (not loop) leaves the position alone.
        s.pads.slicer.loop_mode = false;
        s.pos = 11990.0;
        run(&mut s, 20);
        assert!((s.pos - 12010.0).abs() < 1e-6);
        // Off: nothing at all.
        let mut s = deck(60000);
        s.pads.slicer.pending = Some((0.0, 6000.0, 6500.0));
        run(&mut s, 10);
        assert!((s.pos - 10.0).abs() < 1e-6);
    }

    fn censor_deck(kind: u8, a: f64, b: f64, beat_secs: f32) -> PlayState {
        let mut s = deck(60000);
        s.pads.censor.arm();
        s.pads.censor.on = true;
        s.pads.censor.kind = kind;
        s.pads.censor.regions = Arc::new(vec![CensorSpan { a, b, beat_secs }]);
        s
    }

    #[test]
    fn censor_applies_itself_inside_regions_only() {
        // REV ROLL from 10000 to 12000: before it the track is untouched.
        let mut s = censor_deck(0, 10000.0, 12000.0, 16.0);
        let out = run(&mut s, 10000);
        assert!((out[2 * 9999] - 0.09999).abs() < 1e-5);
        assert!(s.pads.censor.active.is_none());
        // Inside, what was heard before the region comes backwards (after the 2 ms fade in).
        let out = run(&mut s, 2000);
        assert_eq!(s.pads.censor.active, Some(0));
        assert!((out[2 * 500] - 0.09499).abs() < 2e-5, "{}", out[2 * 500]);
        assert!((out[2 * 1500] - 0.08499).abs() < 2e-5, "{}", out[2 * 1500]);
        // After it the track is back (the position never left its course).
        let out = run(&mut s, 1000);
        assert!(s.pads.censor.active.is_none());
        assert!((out[2 * 999] - 0.12999).abs() < 1e-5, "{}", out[2 * 999]);
        assert!((s.pos - 13000.0).abs() < 1e-6);
        // Switched off, or without its buffer, a region does nothing.
        let mut s = censor_deck(0, 10000.0, 12000.0, 16.0);
        s.pads.censor.on = false;
        let out = run(&mut s, 11000);
        assert!((out[2 * 10500] - 0.105).abs() < 1e-5);
    }

    #[test]
    fn censor_trans_echo_and_brake() {
        // TRANS at layer level 0.5 (`trans_duty(0.5)` = half), a cycle of 0.01 s (441
        // frames): open for 220.5 frames, its gate moving at 1 / 3.33 ms (147 frames).
        let mut s = censor_deck(1, 10000.0, 14410.0, 0.01);
        run(&mut s, 10000);
        let out = run(&mut s, 4410);
        assert!((out[2 * (441 + 200)] - 0.10641).abs() < 2e-5, "{}", out[2 * 641]);
        assert!((out[2 * (441 + 73)] - 0.10514 * 74.0 / 147.0).abs() < 2e-3, "{}", out[2 * 514]);
        assert_eq!(out[2 * (441 + 400)], 0.0);
        assert_eq!(out[2 * (882 + 400)], 0.0);
        // ECHO (RELEASE FX) with a beat of 0.01 s: the 441 frames before the region repeat,
        // each time at 0.7 of the last, and the track's own sound is gone.
        let mut s = censor_deck(2, 10000.0, 14410.0, 0.01);
        run(&mut s, 10000);
        let out = run(&mut s, 4410);
        let first = out[2 * 300];
        let second = out[2 * (441 + 300)];
        assert!((first - 0.09859 * 0.7).abs() < 2e-4, "{first}");
        assert!((second - 0.09859 * 0.49).abs() < 2e-4, "{second}");
        // V.BRAKE over 0.02 s (882 frames): rekordbox's `brake_step` takes the speed down in
        // a straight line to 1/8 (frame 772), then at 0.20559 per second; the level is the
        // speed x 8 (at most 1).
        let mut s = censor_deck(3, 10000.0, 14410.0, 0.02);
        run(&mut s, 10000);
        let out = run(&mut s, 4410);
        // (By frame 200 it has read 200 - 200^2 / 1764 = 177 frames: 0.10177; the track 0.102.)
        assert!((out[2 * 200] - 0.10177).abs() < 2e-5, "{}", out[2 * 200]);
        // (Slower than the track: by frame 800 it has read less than 800 frames of it.)
        assert!(out[2 * 800] < 0.105, "{}", out[2 * 800]);
        let sp = s.pads.censor.brake_speed;
        assert!((sp - (0.125 - (4410.0 - 772.0) * 0.20559 / 44100.0)).abs() < 2e-3, "{sp}");
        // The position is never touched by any of them.
        assert!((s.pos - 14410.0).abs() < 1e-6);
    }
}
