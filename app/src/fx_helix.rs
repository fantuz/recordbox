//! HELIX's core, rekordbox's `HelixCore` (constructor 0x1029f7f28, `processEffect`
//! 0x1029f8630, `updateParameter` 0x1029f8e40, `turnOnEffect` 0x1029f8f88, `turnOffEffect`
//! 0x1029f91c0), on two `RingBuffer`s (`getMemory` 0x1029881c0, `readByIntervalSamplesWithLerp`
//! 0x102988db8).
//!
//! States (a1+144): 0 off (the sound passes), 1 recording, 2 looping, 3 changing speed,
//! 4 re-recording at the new length.
//! - 1: the input, through a gate that opens over 3.3333 ms from the turn-on, is written to
//!   loop A until A holds TIME (whole frames, rounded); the sound passes unchanged.
//! - 2: each frame A's frame is replaced by input x IN + loop x KEEP (both `SmoothFloat`s of
//!   3.3333 ms: KEEP made at 0, IN at 1 at the turn-on), which is also the output.
//! - 3 (TIME changed: the new length goes to loop B): A is played (linear interpolation) at a
//!   speed that every (int)(fs x 0.00033333) frames (16 at 48 kHz) is multiplied by
//!   2^(+-1 / round((V x 1.78 or 1.38 + 0.01) x fs / 16)) until it reaches old / new length
//!   (V: TRACK SPEED); above twice the speed the loop is at 1 / (speed - 1), at least 0.25.
//!   Nothing is written.
//! - 4: A plays on at that speed and input x IN + loop x KEEP is written to B; once B is full,
//!   B becomes the loop at speed 1 (state 2), unless KEEP is 1 (the loop fully held), when B
//!   is recorded again.
//!
//! The two loops are the two halves of the caller's delay line.

use crate::fx_lfo::Sf;

#[derive(Clone, Copy)]
pub struct HelixCore {
    rate: f32,
    pub state: u8,
    /// Last TIME seen (148); each loop's TIME (ms), frames and frames written (168 .. 176,
    /// 180 .. 188).
    last_ms: f32,
    a_ms: f32,
    a_len: usize,
    a_written: usize,
    b_ms: f32,
    b_len: usize,
    b_written: usize,
    /// Which half of the line is loop A; read (a double, as `RingBuffer`'s) and write places.
    a_half: usize,
    a_r: f64,
    a_w: usize,
    b_w: usize,
    /// Speed (152), its step factor (156), frames per step (160) and left of the step (164).
    pub speed: f32,
    mult: f32,
    mult_set: bool,
    step: i32,
    step_left: i32,
    /// KEEP (224), IN (240), the recording gate (256).
    keep: Sf,
    inp: Sf,
    gate: Sf,
    keep_v: f32,
    v: f32,
}

impl HelixCore {
    pub fn new(rate: f32) -> HelixCore {
        let mut h = HelixCore { rate, state: 0, last_ms: 10.0, a_ms: 10.0, a_len: 1, a_written: 0, b_ms: 10.0, b_len: 1, b_written: 0, a_half: 0, a_r: 0.0, a_w: 0, b_w: 0, speed: 1.0, mult: 1.0, mult_set: false, step: 16, step_left: 0, keep: Sf::new(rate), inp: Sf::new(rate), gate: crate::fx_rev::new_gate(rate), keep_v: 0.0, v: 0.5 };
        h.set_rate(rate);
        h
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
        self.step = (((rate.trunc() * 0.00033333) as f64).round() as i32).max(1);
        for s in [&mut self.keep, &mut self.inp] {
            s.set_rate(rate);
            s.set_time(3.3333);
        }
        self.keep.reset(0.0);
        self.inp.reset(1.0);
        self.gate = crate::fx_rev::new_gate(rate);
        self.state = 0;
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Frames for a TIME (`(ms x 0.001) x trunc(fs)`, rounded), within a half of the line.
    fn frames(&self, ms: f32, half: usize) -> usize {
        (((ms * 0.001) * self.rate.trunc()) as f64).round().clamp(1.0, (half - 2) as f64) as usize
    }

    /// `turnOnEffect`: recording from the start of A, both loops at TIME.
    pub fn turn_on(&mut self, ms: f32, keep: f32, inp: f32, half: usize) {
        if self.state != 0 {
            return;
        }
        self.state = 1;
        self.last_ms = ms;
        self.a_ms = ms;
        self.a_len = self.frames(ms, half);
        self.b_ms = ms;
        self.b_len = self.a_len;
        self.a_r = 0.0;
        self.a_w = 0;
        self.b_w = 0;
        self.a_written = 0;
        self.b_written = 0;
        self.gate.reset(0.0);
        self.gate.set_target(1.0);
        self.keep.reset(0.0);
        self.keep.set_target(keep);
        self.inp.reset(1.0);
        self.inp.set_target(inp);
        self.step_left = 0;
        self.speed = 1.0;
    }

    /// `turnOffEffect`.
    pub fn turn_off(&mut self) {
        self.state = 0;
    }

    /// `updateParameter` at a block's start: TIME (ms), KEEP, IN and TRACK SPEED.
    pub fn begin(&mut self, ms: f32, keep: f32, inp: f32, v: f32, half: usize) {
        self.v = v;
        self.mult_set = false;
        if ms != self.last_ms {
            match self.state {
                2..=4 => {
                    // (`getMemory` on B: its places back to 0.)
                    self.b_ms = ms;
                    self.b_len = self.frames(ms, half);
                    self.b_w = 0;
                }
                1 => {
                    self.a_ms = ms;
                    self.a_len = self.frames(ms, half);
                    // (`getMemory` sets A's places to 0; the write place is put back when it is
                    // inside the new length.)
                    let w = self.a_w;
                    self.a_r = 0.0;
                    self.a_w = if w < self.a_len { w } else { 0 };
                    if self.a_len < self.a_written {
                        self.state = 2;
                    }
                }
                _ => {}
            }
            self.last_ms = ms;
        }
        self.keep_v = keep;
        self.keep.set_target(keep);
        self.inp.set_target(inp);
    }

    #[inline]
    fn read_lerp(&self, line: &[f32], half: usize, c: usize) -> f32 {
        let i = self.a_r as usize;
        let t = (self.a_r - i as f64) as f32;
        let j = if i + 1 >= self.a_len { 0 } else { i + 1 };
        let base = self.a_half * half;
        let a = line[(base + i.min(self.a_len - 1)) * 2 + c];
        let b = line[(base + j) * 2 + c];
        a - t * a + b * t
    }

    #[inline]
    fn speed_gain(&self) -> f32 {
        if self.speed > 2.0 { (1.0 / (self.speed - 1.0)).max(0.25) } else { 1.0 }
    }

    /// One frame of input `x`; `last` marks the block's last frame (rekordbox checks the
    /// speed at the end of each piece it reads, also one the block cuts short). `line` is the
    /// delay line (two halves of `half` frames, stereo interleaved).
    #[inline]
    pub fn frame(&mut self, line: &mut [f32], half: usize, x: [f32; 2], last: bool) -> [f32; 2] {
        if half < 4 || line.len() < half * 4 {
            return x;
        }
        if self.state == 2 && self.a_ms != self.b_ms {
            self.state = 3;
        } else if self.state == 4 && self.speed != self.a_ms / self.b_ms {
            self.state = 3;
        }
        match self.state {
            1 => {
                let g = self.gate.take();
                let k = (self.a_half * half + self.a_w) * 2;
                line[k] = x[0] * g;
                line[k + 1] = x[1] * g;
                self.a_w = (self.a_w + 1) % self.a_len;
                self.a_written += 1;
                if self.a_written >= self.a_len {
                    self.state = 2;
                }
                x
            }
            2 => {
                let (kg, ig) = (self.keep.take(), self.inp.take());
                let i = (self.a_r as usize).min(self.a_len - 1);
                let base = self.a_half * half;
                let mut y = [0.0f32; 2];
                for c in 0..2 {
                    y[c] = line[(base + i) * 2 + c] * kg + x[c] * ig;
                }
                let k = (base + self.a_w) * 2;
                line[k] = y[0];
                line[k + 1] = y[1];
                self.a_r = ((i + 1) % self.a_len) as f64;
                self.a_w = (self.a_w + 1) % self.a_len;
                y
            }
            3 => {
                let target = self.a_ms / self.b_ms;
                if !self.mult_set {
                    self.mult_set = true;
                    if self.speed == target {
                        self.to_record_b();
                        return self.frame(line, half, x, last);
                    }
                    let (dir, k) = if self.speed < target { (1.0f32, 1.78f32) } else { (-1.0, 1.38) };
                    self.mult = (dir / (((self.v * k + 0.01) * self.rate.trunc()) / self.step as f32).round()).exp2();
                }
                if self.step_left <= 0 {
                    self.speed *= self.mult;
                    self.step_left = self.step;
                }
                self.step_left -= 1;
                let g = self.speed_gain();
                let (kg, ig) = (self.keep.take(), self.inp.take());
                let s = [self.read_lerp(line, half, 0), self.read_lerp(line, half, 1)];
                self.advance_read();
                let y = [x[0] * ig + s[0] * g * kg, x[1] * ig + s[1] * g * kg];
                if self.step_left == 0 || last {
                    let up = self.mult > 1.0;
                    if (up && target <= self.speed) || (!up && self.speed <= target) {
                        self.speed = target;
                        self.to_record_b();
                    }
                }
                y
            }
            4 => {
                let g = self.speed_gain();
                let (kg, ig) = (self.keep.take(), self.inp.take());
                let s = [self.read_lerp(line, half, 0), self.read_lerp(line, half, 1)];
                self.advance_read();
                let y = [x[0] * ig + s[0] * g * kg, x[1] * ig + s[1] * g * kg];
                let k = ((1 - self.a_half) * half + self.b_w) * 2;
                line[k] = y[0];
                line[k + 1] = y[1];
                self.b_w = (self.b_w + 1) % self.b_len;
                self.b_written += 1;
                if self.b_written >= self.b_len {
                    if self.keep_v == 1.0 {
                        self.b_written = 0;
                    } else {
                        // B becomes the loop, at speed 1, from its start.
                        self.a_half = 1 - self.a_half;
                        self.a_ms = self.b_ms;
                        self.a_len = self.b_len;
                        self.a_written = self.b_written;
                        self.b_written = 0;
                        self.a_r = 0.0;
                        self.a_w = 0;
                        self.b_w = 0;
                        self.speed = 1.0;
                        self.state = 2;
                    }
                }
                y
            }
            _ => x,
        }
    }

    fn to_record_b(&mut self) {
        self.b_w = 0;
        self.b_written = 0;
        self.step_left = 0;
        self.state = 4;
    }

    #[inline]
    fn advance_read(&mut self) {
        self.a_r += self.speed as f64;
        if self.a_r >= self.a_len as f64 {
            self.a_r -= self.a_len as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recording passes the sound, the loop then mixes KEEP x loop + IN x input and writes it
    /// back; a new TIME plays the loop faster in 16-frame steps and records the new length.
    #[test]
    fn helix_core_records_loops_and_changes_speed_as_rekordbox() {
        let rate = 48000.0;
        let half = 48000 * 4;
        let mut line = vec![0.0f32; half * 4];
        let mut h = HelixCore::new(rate);
        assert_eq!(h.step, 16);
        // 10 ms = 480 frames.
        h.turn_on(10.0, 0.5, 0.5, half);
        let mut out = Vec::new();
        for i in 0..2000 {
            if i % 512 == 0 {
                h.begin(10.0, 0.5, 0.5, 0.5, half);
            }
            out.push(h.frame(&mut line, half, [1.0, 1.0], i % 512 == 511)[0]);
            if i == 100 {
                assert!(line[2] > 0.0 && line[2] < 0.01 && (line[200] - 0.625).abs() < 0.01, "the gate opens from 0 over 3.3333 ms: {} {}", line[2], line[200]);
            }
        }
        assert_eq!(out[100], 1.0, "recording passes the sound");
        // First frame of the loop: KEEP from 0, IN from 1 (`take` gives the value, then steps).
        assert!((out[480] - (line[0] * 0.0 + 1.0)).abs() < 0.01 || out[480] > 0.9, "{}", out[480]);
        assert_eq!(h.state, 2);
        // TIME 5 ms: speed up to 2 in steps; V 0.5: 2^(1 / round(0.9 x 48000 / 16)) a step.
        h.begin(5.0, 0.5, 0.5, 0.5, half);
        let m = (1.0f32 / ((0.5f32 * 1.78 + 0.01) * 48000.0 / 16.0).round()).exp2();
        h.frame(&mut line, half, [0.0, 0.0], false);
        assert_eq!(h.state, 3);
        assert!((h.speed - m).abs() < 1e-7, "first step at once: {} {}", h.speed, m);
        let mut n = 1;
        while h.state == 3 && n < 200000 {
            if n % 512 == 0 {
                h.begin(5.0, 0.5, 0.5, 0.5, half);
            }
            h.frame(&mut line, half, [0.0, 0.0], n % 512 == 511);
            n += 1;
        }
        assert_eq!((h.state, h.speed), (4, 2.0));
        // An octave in (0.5 x 1.78 + 0.01) s = 43200 frames, in 16-frame steps.
        assert!((n as i32 - 43200).abs() <= 32, "{n}");
        for i in 0..240 {
            h.frame(&mut line, half, [0.0, 0.0], i == 239);
        }
        assert_eq!((h.state, h.speed, h.a_len), (2, 1.0, 240), "B is the loop at speed 1");
    }
}
