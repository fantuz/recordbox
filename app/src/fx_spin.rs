//! RELEASE FX BACK SPIN as rekordbox runs it (`ReleaseFxBackSpin` 0x1029a8b38 on `BackSpinCore`
//! 0x1029a6ea8, `BackSpinReverseRoll` 0x1029a5544 and `SlowPlaybackFilter` 0x1028c0f5c).
//!
//! - The speed (`BackSpinCore::calculatePlaybackSpeedArray` 0x1029a7384) climbs in a straight
//!   line to 8 over SPEED UP TIME (0.16 of two beats), falls to 0.28235 over SLOW DOWN TIME (the
//!   rest of the stop time; both halves of it when that is shorter than the climb), then falls
//!   toward 0 at (speed / (-0.1861 fs)) a frame. Each piece's length is worked out at the start
//!   of each call from where the speed is. The last piece's length is (int)(-n / slope) with n
//!   the frames the piece before it took in the same call, so it ends with that call (or at once
//!   when it starts a call) and the speed is then 0 for good.
//! - The sound before the press is read backwards at that speed (`BackSpinReverseRoll`): once
//!   the next step would go further back than two beats (less one block; at least one block) a
//!   3.3333 ms gate shuts, and once shut the read goes back to the press and the gate opens.
//! - The sound is weighted by the speed (18.165 v^2 (1 - 2 v) under 0.28348, v^3 + 3 (v - v^2)
//!   under 1, then 0.98 + 0.02 v) x 0.5598, then `SlowPlaybackFilter`: two first-order low-passes
//!   whose cutoff falls with the speed (0.0041 .. 0.443 fs; none from speed 1) and, under half
//!   speed, a first-order high-pass (54.2 Hz falling to 9 Hz).

use crate::fx_lfo::Sf;

/// `SlowPlaybackFilter` (`makeCoefTable` 0x1028c11e4: 100 steps of 0.01 in speed for each
/// filter; `handleAudioProcess` 0x1028c1044).
#[derive(Clone, Copy)]
pub struct SlowPlay {
    rate: f32,
    lp: [[[f32; 2]; 2]; 2],
    hp: [[f32; 2]; 2],
    /// The last table step and its two sections' coefficients.
    step: i32,
    lc: [f32; 3],
    hc: [f32; 3],
}

impl SlowPlay {
    pub fn new(rate: f32) -> SlowPlay {
        SlowPlay { rate, lp: [[[0.0; 2]; 2]; 2], hp: [[0.0; 2]; 2], step: i32::MIN, lc: [1.0, 0.0, 0.0], hc: [1.0, 0.0, 0.0] }
    }

    /// A new rate (the steps' coefficients are worked out again).
    #[inline]
    pub fn set_rate(&mut self, rate: f32) {
        if rate != self.rate {
            self.rate = rate;
            self.step = i32::MIN;
        }
    }

    pub fn clear(&mut self) {
        self.lp = [[[0.0; 2]; 2]; 2];
        self.hp = [[0.0; 2]; 2];
    }

    /// The low-pass cutoff (Hz) of table step `i` (speed 0.01 i).
    pub fn lp_fc(i: usize, rate: f32) -> f32 {
        let s = step_speed(i);
        let k = if s > 0.92 { 0.44331 } else if s >= 0.0 { s * 0.000022676 * ((s * -36320.0 + 44156.0) * s + 11160.0) + 0.0041018 } else { 0.0034014 };
        k * rate.trunc()
    }

    /// The high-pass cutoff (Hz) of table step `i`.
    pub fn hp_fc(i: usize) -> f32 {
        let s = step_speed(i);
        if s <= 0.4 { if s >= 0.0 { s * -112.18 + 54.227 } else { 55.0 } } else { 9.0 }
    }

    #[inline]
    fn run(c: &[f32; 3], z: &mut [f32; 2], x: f32) -> f32 {
        let y = c[0] * x + c[1] * z[0] - c[2] * z[1];
        z[0] = x;
        z[1] = y;
        y
    }

    /// One frame at speed `v` (the table step is (int)(v x 100); past step 99 the filters
    /// pass the sound as it is).
    #[inline]
    pub fn frame(&mut self, x: [f32; 2], v: f32) -> [f32; 2] {
        let i = (v * 100.0) as i32;
        if i != self.step {
            self.step = i;
            let ok = (0..=99).contains(&i);
            self.lc = if ok { crate::fx_core::iir_1st(Self::lp_fc(i as usize, self.rate) as f64, self.rate as f64, false) } else { [1.0, 0.0, 0.0] };
            self.hc = if ok { crate::fx_core::iir_1st(Self::hp_fc(i as usize) as f64, self.rate as f64, true) } else { [1.0, 0.0, 0.0] };
        }
        let (lp, hp) = (self.lc, self.hc);
        let mut y = x;
        for c in 0..2 {
            y[c] = Self::run(&lp, &mut self.lp[0][c], y[c]);
            y[c] = Self::run(&lp, &mut self.lp[1][c], y[c]);
        }
        if v < 0.5 {
            for c in 0..2 {
                y[c] = Self::run(&hp, &mut self.hp[c], y[c]);
            }
        }
        y
    }
}

/// The table's speeds as `makeCoefTable` steps them (0.0, then + 0.01 in float each step).
fn step_speed(i: usize) -> f32 {
    let mut s = 0.0f32;
    for _ in 0..i.min(99) {
        s += 0.01;
    }
    s
}

/// The sound's weight at speed `v` (`BackSpinCore::processEffectConcrete`), x 0.5598.
pub fn spin_gain(v: f32, peak: f32) -> f32 {
    let g = if v < 0.0 {
        0.0
    } else if v < 0.28348 {
        let q = v * v * 18.1651514;
        q - (v + v) * q
    } else if v < 1.0 {
        v * v * v + (v - v * v) * 3.0
    } else if v <= peak {
        v * 0.02 + 0.98
    } else {
        0.0
    };
    g * 0.5598
}

/// One BACK SPIN for one channel.
#[derive(Clone, Copy)]
pub struct BackSpin {
    rate: f32,
    /// Speed pieces (`BackSpinCore` 1148): 0 up, 1 down, 2 stop, 3 still; the speed (1140,
    /// unclamped), its step, frames left in the piece, frames the piece has had in this call.
    seg: u8,
    acc: f32,
    slope: f32,
    left: i32,
    piece: i32,
    up_f: f32,
    down_f: f32,
    peak: f32,
    /// Frames back from the frame before the press, the furthest it reads back, the gate.
    back: f64,
    reach: f64,
    gate: Sf,
    pub filt: SlowPlay,
}

impl BackSpin {
    pub fn new(rate: f32) -> BackSpin {
        let mut gate = crate::fx_rev::new_gate(rate);
        gate.reset(1.0);
        BackSpin { rate, seg: 3, acc: 0.0, slope: 0.0, left: 0, piece: 0, up_f: 1.0, down_f: 1.0, peak: 8.0, back: 0.0, reach: 1.0, gate, filt: SlowPlay::new(rate) }
    }

    /// The press (`onCalledTurnOn`): stop time and two beats (ms), the audio block (frames).
    pub fn start(&mut self, rate: f32, stop_ms: f32, two_beats_ms: f32, block: usize) {
        *self = BackSpin::new(rate);
        let (stop, two) = (stop_ms.clamp(10.0, 100000.0), two_beats_ms.clamp(10.0, 4000.0));
        // (`ReleaseFxBackSpin::processEffectConcrete`: SPEED UP / SLOW DOWN TIME.)
        let (up, down) = if stop - two * 0.16 >= two * 0.16 { (two * 0.16, stop - two * 0.16) } else { (stop * 0.5, stop * 0.5) };
        let fs = rate as i32 as f32;
        self.up_f = (up.max(1.0) * 0.001) * fs;
        self.down_f = (down.max(1.0) * 0.001) * fs;
        self.seg = 0;
        // (`BackSpinReverseRoll::processEffect`: the lowest read place, two beats less a block
        // back, at least a block and at most the ring less eight blocks.)
        let b = block.max(1) as f64;
        let span = (((two as f64 * 0.001) as f32) * rate) as i32 as f64;
        let ring = 2.0 * ((((4000.0f64 * 0.001) as f32) * rate) as i32) as f64;
        self.reach = (span - b).min(ring - 1.0 - 8.0 * b).max(b);
    }

    fn set_piece(&mut self, prev: i32) {
        let fs = self.rate as i32 as f32;
        let n = match self.seg {
            0 => {
                self.slope = self.peak / self.up_f;
                ((self.peak - self.acc) / self.slope) as i32
            }
            1 => {
                self.slope = -self.peak / self.down_f;
                ((0.28235 - self.acc) / self.slope) as i32
            }
            2 => {
                self.slope = self.acc / (fs * -0.1861);
                (-(prev as f32) / self.slope) as i32
            }
            _ => 0,
        };
        self.left = n.max(0);
    }

    /// The start of a call (`calculatePlaybackSpeedArray` works its pieces out again).
    pub fn begin_call(&mut self) {
        self.piece = 0;
        if self.seg < 3 {
            self.set_piece(0);
        }
    }

    /// The next frame's speed.
    #[inline]
    fn speed(&mut self) -> f32 {
        while self.seg < 3 && self.left <= 0 {
            let prev = self.piece;
            self.seg += 1;
            self.piece = 0;
            self.set_piece(prev);
        }
        if self.seg >= 3 {
            return 0.0;
        }
        self.acc += self.slope;
        self.left -= 1;
        self.piece += 1;
        self.acc.max(0.0)
    }

    #[cfg(test)]
    pub fn done(&self) -> bool {
        self.seg >= 3
    }

    /// One frame: `hist(k)` is the channel's sound `k` frames before the frame before the press.
    #[inline]
    pub fn frame(&mut self, hist: impl Fn(usize) -> (f32, f32)) -> (f32, f32) {
        if self.seg >= 3 {
            // (`BackSpinCore::processEffectConcrete`: silence once the speed is done.)
            return (0.0, 0.0);
        }
        let v = self.speed();
        if !self.gate.on && self.gate.cur.abs() < 1.1920929e-7 {
            // Shut: back to the press, open again.
            self.back = 0.0;
            self.gate.set_target(1.0);
        } else if !(self.gate.on && self.gate.target.abs() < 1.1920929e-7) && self.back + v as f64 > self.reach {
            self.gate.set_target(0.0);
        }
        let (l, r) = hist(self.back as usize);
        let g = self.gate.take();
        self.back += v as f64;
        let w = spin_gain(v, self.peak);
        let y = self.filt.frame([l * g * w, r * g * w], v);
        (y[0], y[1])
    }
}

/// VINYL BRAKE's weight of the sound at speed `v` (`VinylBrakeCore::processBlock`
/// 0x102a584f4, from its disassembly): none to 0.01, 12.5 v - 0.125 to 0.05, 0.5 to 0.125,
/// 0.42857 + 0.57143 v to 1, then 1.
pub fn brake_gain(v: f32) -> f32 {
    if v <= 0.01 {
        0.0
    } else if v <= 0.05 {
        12.5 * v - 0.125
    } else if v <= 0.125 {
        0.5
    } else if v <= 1.0 {
        v * 0.5714286 + 0.4285714
    } else {
        1.0
    }
}

/// rekordbox's `VinylBrakeCore` (0x102a58220 ..): the sound from the start, read at a falling
/// speed (with interpolation), weighted by the speed and through `SlowPlaybackFilter`.
#[derive(Clone, Copy)]
pub struct VinylCore {
    rate: f32,
    /// Decay time in use and wanted (ms), the speed (unclamped) and its step, the state (0
    /// falling at 1 / decay, 1 at 0.20559 / s, 2 stopped), the read place (frames from the start).
    decay_in: f32,
    pub decay: f32,
    cur: f32,
    slope: f32,
    pub state: u8,
    pos: f64,
    pub filt: SlowPlay,
}

impl VinylCore {
    pub fn new(rate: f32) -> VinylCore {
        VinylCore { rate, decay_in: 1000.0, decay: 1000.0, cur: 1.0, slope: 0.0, state: 2, pos: 0.0, filt: SlowPlay::new(rate) }
    }

    /// `setDecayTimeMs` 0x102a5848c: 10 .. 12000 ms, else kept.
    pub fn set_decay(&mut self, ms: f32) {
        if (ms > 10.0 || (ms - 10.0).abs() < 1.1920929e-7) && (ms < 12000.0 || (ms - 12000.0).abs() < 1.1920929e-7) {
            self.decay = ms;
        }
    }

    /// `startVinylBrake` 0x102a58b44 / `ReleaseFxVinylBrake::onCalledTurnOn` 0x102a595e0.
    pub fn start(&mut self, rate: f32) {
        let d = self.decay;
        *self = VinylCore::new(rate);
        self.decay = d;
        self.decay_in = d;
        self.cur = 1.0;
        self.slope = -1.0 / ((d * 0.001) * rate);
        self.state = 0;
    }

    /// `calculatePlaybackSpeedArray` 0x102a58934 (aftertaste type 0) and the state changes
    /// `processBlock` makes after it: `out.len()` frames of speed.
    pub fn speeds(&mut self, out: &mut [f32]) {
        if (self.decay - self.decay_in).abs() >= 1.1920929e-7 {
            self.decay_in = self.decay;
            self.slope = -1.0 / ((self.decay * 0.001) * self.rate);
        }
        let low = -0.20559 / self.rate;
        let mut i = 0usize;
        let mut n = out.len() as i32;
        while n > 0 {
            let m = match self.state {
                0 => {
                    if self.cur + self.slope * n as f32 >= 0.125 {
                        n
                    } else if self.cur >= 0.125 {
                        (((0.125 - self.cur) / self.slope) as i32).max(1)
                    } else {
                        0
                    }
                }
                1 => {
                    if self.cur + self.slope * n as f32 >= 0.0 { n } else { ((self.cur / -self.slope) as i32).max(0) }
                }
                _ => {
                    out[i..].iter_mut().for_each(|v| *v = 0.0);
                    return;
                }
            };
            let m = m.min(n);
            for v in out[i..i + m as usize].iter_mut() {
                self.cur += self.slope;
                *v = self.cur;
            }
            i += m as usize;
            n -= m;
            // (Between pieces and after the call: below 1/8 the slow fall; within one step of
            // nothing, stopped.)
            if self.state == 0 && self.cur < 0.125 {
                self.state = 1;
                self.slope = low;
            } else if self.state == 1 && (self.cur as f64) < -(self.slope as f64) {
                self.state = 2;
                self.cur = 0.0;
                self.slope = 0.0;
            }
        }
    }

    /// One frame at speed `v` (from `speeds`): `hist(i)` the sound `i` frames after the start.
    #[inline]
    pub fn frame(&mut self, v: f32, hist: impl Fn(usize) -> (f32, f32)) -> (f32, f32) {
        if self.state == 2 && v == 0.0 {
            return (0.0, 0.0);
        }
        let i = self.pos as usize;
        let t = (self.pos - i as f64) as f32;
        let (a, b) = (hist(i), hist(i + 1));
        self.pos = (self.pos + v as f64).min(6.0 * self.rate as f64);
        let g = brake_gain(v);
        let y = self.filt.frame([(a.0 + (b.0 - a.0) * t) * g, (a.1 + (b.1 - a.1) * t) * g], v);
        (y[0], y[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_climbs_falls_and_stops_within_its_call() {
        // Stop time 2 s, two beats 1 s at 48 kHz: up 160 ms to 8, down 1.84 s to 0.28235, then
        // the stop piece ends with the call it started in.
        let mut s = BackSpin::new(48000.0);
        s.start(48000.0, 2000.0, 1000.0, 512);
        let mut v = Vec::new();
        for _ in 0..400 {
            s.begin_call();
            for _ in 0..512 {
                v.push(if s.done() { 0.0 } else { s.speed() });
            }
        }
        let up = (0.16f32 * 0.001 * 1000.0 * 48000.0) as usize;
        assert!((v[up - 1] - 8.0).abs() < 0.01 && v[up / 2] > 3.9 && v[up / 2] < 4.1, "{} {}", v[up - 1], v[up / 2]);
        // (The fall reaches 0.28235 at (8 - 0.28235) / 8 of SLOW DOWN TIME.)
        let end1 = up + (1840.0f32 * 0.001 * 48000.0 * (8.0 - 0.28235) / 8.0) as usize;
        assert!((v[end1 - 2] - 0.28235).abs() < 0.01, "{}", v[end1 - 2]);
        let call_end = (end1 / 512 + 1) * 512;
        assert!(v[call_end..].iter().all(|&x| x == 0.0), "still after the call the stop piece began in");
        assert_eq!(spin_gain(0.0, 8.0), 0.0);
        assert!((spin_gain(1.0, 8.0) - 0.5598).abs() < 1e-6 && (spin_gain(8.0, 8.0) - 1.14 * 0.5598).abs() < 1e-5);
        assert!((SlowPlay::lp_fc(50, 48000.0) - 48000.0 * (0.0041018 + 0.5 * 0.000022676 * ((0.5 * -36320.0 + 44156.0) * 0.5 + 11160.0))).abs() < 5.0);
        assert!((SlowPlay::hp_fc(0) - 54.227).abs() < 1e-3 && SlowPlay::hp_fc(60) == 9.0);
        // (STRETCH at a quarter speed: the low-passes at 0.117 fs, 5.62 kHz at 48 kHz.)
        assert!((SlowPlay::lp_fc(25, 48000.0) - 5619.8).abs() < 1.0, "{}", SlowPlay::lp_fc(25, 48000.0));
    }

    #[test]
    fn vinyl_core_falls_then_creeps_to_a_stop() {
        // Decay 1 s at 48 kHz: 1/48000 a frame down to 1/8 (42000 frames), then 0.20559 / s
        // (0.608 s more), then silence.
        let mut c = VinylCore::new(48000.0);
        c.set_decay(1000.0);
        c.start(48000.0);
        let mut v = vec![0.0f32; 48000 * 2];
        for k in v.chunks_mut(512) {
            c.speeds(k);
        }
        assert!((v[0] - (1.0 - 1.0 / 48000.0)).abs() < 1e-6);
        let knee = v.iter().position(|&x| x < 0.125).unwrap();
        assert!((knee as i32 - 42000).abs() <= 64, "{knee}"); // (f32 steps add up as in rekordbox)
        let stop = v.iter().position(|&x| x == 0.0).unwrap();
        assert!(((stop - knee) as f32 / 48000.0 - 0.125 / 0.20559).abs() < 0.02, "{}", (stop - knee) as f32 / 48000.0);
        assert!(v[stop..].iter().all(|&x| x == 0.0) && c.state == 2);
        assert_eq!((brake_gain(0.01), brake_gain(0.03), brake_gain(0.1), brake_gain(2.0)), (0.0, 0.25, 0.5, 1.0));
        assert!((brake_gain(0.5) - 0.7142857).abs() < 1e-6);
    }

    #[test]
    fn reads_back_and_loops_through_its_gate() {
        // Two beats of 100 ms, block 512: the read goes back at most 4800 - 512 frames, then
        // the gate shuts and it starts again at the press.
        let mut s = BackSpin::new(48000.0);
        s.start(48000.0, 60000.0, 100.0, 512);
        let mut far = 0.0f64;
        let mut back_to_start = false;
        for _ in 0..40 {
            s.begin_call();
            for _ in 0..512 {
                let before = s.back;
                let (l, _) = s.frame(|k| (k as f32, 0.0));
                assert!(l.is_finite());
                far = far.max(s.back);
                if before > 100.0 && s.back < 9.0 {
                    back_to_start = true;
                }
            }
        }
        assert!(back_to_start && far < 4288.0 + 8.0 + 160.0 * 8.0, "{far}");
    }
}
