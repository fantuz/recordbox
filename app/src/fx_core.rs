//! Building blocks of rekordbox 7.2.19's effects, ported from its code (arm64, Hex-Rays; the
//! addresses are in `checks/re-7.2.19/track-notes/fxdsp.md`): the pitch shifter, the filter
//! sections of `IirFilter` and `Filter`, and the small helpers the effects share. Everything
//! here allocates when it is made and never in `process`.

use std::f64::consts::PI;

/// `rbxfrm::PitchShifter`: a delay-line pitch shifter. The read point moves through a short
/// line at `1 + shift` times the write speed; when it gets too close to the newest sample (or
/// too far behind, going down) it jumps by a cross-fade of 3 ms to a point on the other side
/// of the window. The jump may instead land on the last accepted rising zero crossing of the
/// input's 30..300 Hz band; that crossing is searched once per block (`begin_block`) and its
/// age only grows by whole blocks, so with 512-frame blocks most jumps are the forced ones
/// across the window. Reads are 4-point Lagrange.
///
/// Constants from `PitchShifter::PitchShifter` / `prepareToPlay`: cross-fade 3 ms, window
/// 13..23 ms, start-up fill 30 ms, detection filters a 30 Hz high-pass and a 300 Hz low-pass
/// (second order, Q 0.7071), detection mix 0.91406 L + 0.085938 R, output gain smoother 20 ms
/// (used to mute at the bottom when asked), the shift moving at most 0.05 a sample.
pub struct PitchShifter {
    buf: Vec<f32>,
    w: usize,
    filled: usize,
    /// The shift asked for (`f24`; ratio - 1) and the one in use (`f28`).
    target: f32,
    cur: f32,
    /// Read delay in frames (`f652`), the jump being faded to (`f656`) and how far into the
    /// fade it is (`d680`).
    delay: f32,
    jump: f32,
    fade_n: i32,
    fading: bool,
    starting: bool,
    /// Frames since the last rising zero crossing of the detection band (`d684`).
    since: i32,
    prev_det: f32,
    /// The block in progress: frames left and its length.
    left: usize,
    block_n: usize,
    det: [[f32; 4]; 2],
    hp: [f32; 5],
    lp: [f32; 5],
    xf: i32,
    inv_xf: f32,
    min_d: i32,
    max_d: i32,
    start_d: i32,
    gain: f32,
    gain_k: f32,
    mute_down_max: bool,
}

/// Line length: the 30 ms fill, a block and the 3 ms fade at up to 192 kHz.
const PS_CAP: usize = 8192;

impl PitchShifter {
    pub fn new(rate: f32) -> PitchShifter {
        let mut p = PitchShifter { buf: vec![0.0; PS_CAP * 2], w: 0, filled: 0, target: 0.0, cur: 0.0, delay: 0.0, jump: 0.0, fade_n: 0, fading: false, starting: true, since: 0, prev_det: 0.0, left: 0, block_n: 1, det: [[0.0; 4]; 2], hp: [0.0; 5], lp: [0.0; 5], xf: 1, inv_xf: 1.0, min_d: 1, max_d: 2, start_d: 3, gain: 1.0, gain_k: 0.0, mute_down_max: false };
        p.prepare(rate);
        p
    }

    /// An empty shifter for a place that never runs.
    pub fn empty() -> PitchShifter {
        PitchShifter { buf: Vec::new(), ..PitchShifter::new_unprepared() }
    }

    fn new_unprepared() -> PitchShifter {
        PitchShifter { buf: Vec::new(), w: 0, filled: 0, target: 0.0, cur: 0.0, delay: 0.0, jump: 0.0, fade_n: 0, fading: false, starting: true, since: 0, prev_det: 0.0, left: 0, block_n: 1, det: [[0.0; 4]; 2], hp: [0.0; 5], lp: [0.0; 5], xf: 1, inv_xf: 1.0, min_d: 1, max_d: 2, start_d: 3, gain: 1.0, gain_k: 0.0, mute_down_max: false }
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// `prepareToPlay`: the times in frames for a rate, and a clean start.
    pub fn prepare(&mut self, rate: f32) {
        let r = rate.max(8000.0) as f64;
        self.xf = ((r * 0.003).round() as i32).max(1);
        self.inv_xf = 1.0 / self.xf as f32;
        self.min_d = (r * 0.013).round() as i32;
        self.max_d = (r * 0.023).round() as i32;
        self.start_d = ((r * 0.03).round() as i32).min(PS_CAP as i32 - 600);
        self.hp = iir_hpf2(30.0, 0.70711, r);
        self.lp = iir_lpf2(300.0, 0.70711, r);
        self.gain_k = (1.0 / (0.020 * r)) as f32;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
        self.w = 0;
        self.filled = 0;
        self.cur = self.target;
        self.delay = 0.0;
        self.jump = 0.0;
        self.fade_n = 0;
        self.fading = false;
        self.starting = true;
        self.since = 0;
        self.prev_det = 0.0;
        self.left = 0;
        self.det = [[0.0; 4]; 2];
        self.gain = 1.0;
    }

    /// `setPitchPercentage` / `setPitchParameter`: -100 % an octave down, +100 % an octave
    /// up, +200 % two octaves up.
    pub fn set_percent(&mut self, pct: f32) {
        self.target = percent_shift(pct);
    }

    /// `setPitchParameterDirect`: the shift itself (ratio - 1), -2..3.
    pub fn set_shift(&mut self, shift: f32) {
        self.target = shift.clamp(-2.0, 3.0);
    }

    /// `enableMuteWhenPitchDownMax`: fade to silence once the shift passes -0.95.
    pub fn mute_at_bottom(&mut self, on: bool) {
        self.mute_down_max = on;
    }

    #[inline]
    fn read(&self, delay: f32) -> (f32, f32) {
        let i = delay.floor();
        let x = delay - i;
        let i = i as isize;
        let cap = PS_CAP as isize;
        let at = |d: isize| {
            // (`readWithDelaySampleNum`: 1 is the newest frame.)
            let k = ((self.w as isize - d.clamp(1, cap - 1)).rem_euclid(cap)) as usize * 2;
            (self.buf[k], self.buf[k + 1])
        };
        let (am, a0, a1, a2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        // `getDelayData`: Lagrange weights for x between the points at i and i + 1.
        let x2 = x * x;
        let x3 = x2 * x;
        let wm = -(x3 - 3.0 * x2 + 2.0 * x) / 6.0;
        let w0 = (x3 - 2.0 * x2 - x + 2.0) * 0.5;
        let w1 = -(x3 - x2 - 2.0 * x) * 0.5;
        let w2 = (x3 - x) / 6.0;
        (am.0 * wm + a0.0 * w0 + a1.0 * w1 + a2.0 * w2, am.1 * wm + a0.1 * w0 + a1.1 * w1 + a2.1 * w2)
    }

    /// `checkEnableCrossFade`: move the shift toward its target and start a jump when the
    /// read point leaves the window.
    #[inline]
    fn check_jump(&mut self) {
        if self.fading || self.starting {
            return;
        }
        let t = self.target;
        let mut r = self.cur;
        if r != t {
            r = if (t - r).abs() > 0.05 { if t > r { r + 0.05 } else { r - 0.05 } } else { t };
            self.cur = r;
        }
        let d = self.delay as i32;
        let next = (d - r.round() as i32).max(2);
        let shift_xf = (r * self.xf as f32).round() as i32;
        let low = self.min_d + shift_xf;
        let to;
        if r < 0.0 {
            if next < self.max_d {
                if next < low {
                    return;
                }
                let z = self.since;
                if z <= self.min_d || z >= d {
                    return;
                }
                to = z;
            } else {
                to = low;
            }
        } else if r <= 0.0 {
            if next >= self.min_d {
                return;
            }
            to = low;
        } else {
            let high = self.max_d + shift_xf;
            if next <= self.min_d {
                to = high;
            } else if next <= low && self.since > low && self.since < high {
                to = self.since;
            } else {
                return;
            }
        }
        self.jump = (to - d) as f32;
        self.fading = true;
        self.fade_n = 0;
    }

    /// rekordbox's block length (`PitchShifter(fs, 512)`, the audio buffer it is called with).
    pub const BLOCK: usize = 512;

    /// Frames left in the block begun last (0: none).
    pub fn block_left(&self) -> usize {
        self.left
    }

    /// The start of a block (`processPitchShift`, before its frames run): the block's input
    /// goes through the 30 Hz high-pass and 300 Hz low-pass, the first rising zero crossing of
    /// 0.91406 L + 0.085938 R is found, and if the last accepted crossing is at least 13 ms old
    /// `d684` becomes minus its index. `d684` then stays put for the whole block and grows by
    /// the block's length after it. The next `n` calls of `process` are this block.
    pub fn begin_block<I: Iterator<Item = (f32, f32)>>(&mut self, frames: I) {
        let mut first: i32 = -1;
        let mut n = 0usize;
        for (i, (l, r)) in frames.enumerate() {
            let mut band = [0.0f32; 2];
            for (c, x) in [l, r].into_iter().enumerate() {
                let z = &mut self.det[c];
                let h = bq(&self.hp, &mut z[0..2], x);
                band[c] = bq(&self.lp, &mut z[2..4], h);
            }
            let det = band[0] * 0.91406 + band[1] * 0.085938;
            if first < 0 && det * self.prev_det <= 0.0 && self.prev_det < 0.0 {
                first = i as i32;
            }
            self.prev_det = det;
            n += 1;
        }
        if first >= 0 && self.since >= self.min_d {
            self.since = -first;
        }
        self.left = n.max(1);
        self.block_n = n.max(1);
    }

    /// One frame through the shifter.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.buf.is_empty() {
            return (l, r);
        }
        // Outside a block begun by the caller this frame is a block of one (rekordbox's
        // behaviour when it is called a frame at a time).
        if self.left == 0 {
            self.begin_block(std::iter::once((l, r)));
        }
        self.check_jump();
        let out;
        if self.starting {
            if self.filled as i32 >= self.start_d {
                self.delay -= self.cur;
                let s = self.read(self.delay);
                let t = self.inv_xf * (self.filled as i32 - self.start_d) as f32;
                let k = (3.0 - 2.0 * t) * t * t;
                out = (l + (s.0 - l) * k, r + (s.1 - r) * k);
                if k >= 1.0 - self.inv_xf {
                    self.starting = false;
                }
            } else {
                out = (l, r);
                self.delay += 1.0;
            }
        } else {
            self.delay -= self.cur;
            if self.fading {
                // `crossFadePitchShift`: from the old read point to the new over 3 ms.
                let a = self.read(self.delay);
                let b = self.read(self.delay + self.jump);
                let t = self.inv_xf * self.fade_n as f32;
                let k = (3.0 - 2.0 * t) * t * t;
                out = (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k);
                self.fade_n += 1;
                if t >= 1.0 - self.inv_xf {
                    self.fading = false;
                    self.fade_n = 0;
                    self.delay += self.jump;
                }
            } else {
                out = self.read(self.delay);
            }
        }
        self.delay = self.delay.clamp(0.0, (PS_CAP - 4) as f32);
        self.left -= 1;
        if self.left == 0 {
            // (`processPitchShift`: `d684 += N` after the block's frames.)
            self.since = self.since.saturating_add(self.block_n as i32);
        }
        self.buf[self.w * 2] = l;
        self.buf[self.w * 2 + 1] = r;
        self.w = (self.w + 1) % PS_CAP;
        self.filled = (self.filled + 1).min(PS_CAP);
        let want = if self.mute_down_max && self.cur < -0.95 { 0.0 } else { 1.0 };
        self.gain += (want - self.gain).clamp(-self.gain_k, self.gain_k);
        (out.0 * self.gain, out.1 * self.gain)
    }
}

/// ENIGMA JET (`BeatFxEnigmaJet::bfx_flanger_updown`): a flanger that seems to rise or fall
/// without end. The input is split at 70 Hz (a critically damped second-order pair); the lows
/// stay dry inside the wet sound, the highs feed four short lines (0.1 ms .. 2.1 ms), two per
/// side. One saw LFO turns once per beat value; the taps sit a quarter turn apart (left 0 and
/// 1/2, right 1/4 and 3/4), each faded in and out over the first and last tenth of its turn
/// (squared) and fed back, the second of each pair with the opposite sign and subtracted.
/// LEVEL/DEPTH (0..1023) sets the wet share (up to a half) and the feedback range, FEEDBACK
/// moves within that range, MOD adds a triangle wobble of the delays.
pub struct EnigmaJet {
    lines: Vec<f32>,
    len: usize,
    min: f32,
    range: f32,
    w: usize,
    filled: bool,
    phase: f32,
    wobble: f32,
    /// Wet gain and dry gain in use, moving 300 / rate a sample toward their targets.
    wet: f32,
    dry: f32,
    step: f32,
    /// The 70 Hz split: high-pass b0, b1, b2, a1, a2, and its state per side.
    hp: [f64; 5],
    z: [[f64; 2]; 2],
    was_on: bool,
}

const EJ_LINE: usize = 512;

impl EnigmaJet {
    pub fn new() -> EnigmaJet {
        let mut e = EnigmaJet { lines: vec![0.0; EJ_LINE * 4], ..EnigmaJet::empty() };
        e.prepare(48000.0);
        e
    }

    pub fn empty() -> EnigmaJet {
        EnigmaJet { lines: Vec::new(), len: 2, min: 1.0, range: 1.0, w: 0, filled: false, phase: 0.0, wobble: 0.0, wet: 0.0, dry: 1.0, step: 0.0, hp: [0.0; 5], z: [[0.0; 2]; 2], was_on: false }
    }

    pub fn prepare(&mut self, rate: f32) {
        let fs = rate.max(8000.0) as i32;
        let min = fs / 10000;
        let range = fs / 500;
        self.len = ((min + range + 2) as usize).min(EJ_LINE);
        self.min = min as f32;
        self.range = range as f32;
        self.step = 300.0 / fs as f32;
        let wt = 439.822972 / fs as f64;
        let (w2, w4) = (wt * wt, wt * 4.0);
        let n = 1.0 / (w4 + 4.0 + w2);
        self.hp = [4.0 * n, -8.0 * n, 4.0 * n, n * (2.0 * w2 - 8.0), n * (4.0 - w4 + w2)];
        self.reset();
    }

    pub fn reset(&mut self) {
        self.lines.iter_mut().for_each(|v| *v = 0.0);
        self.w = 0;
        self.filled = false;
        self.phase = 0.0;
        self.wobble = 0.0;
        self.z = [[0.0; 2]; 2];
        self.wet = 0.0;
        self.dry = 1.0;
    }

    /// The (feedback low, feedback high) pair and the wet target for LEVEL/DEPTH in 10-bit
    /// steps.
    pub fn level_law(v: i32) -> (f32, f32, f32) {
        if v <= 9 {
            (0.0, 0.0, 0.0)
        } else if v <= 511 {
            let n = (v - 10) as f32;
            (0.0013944224 * n, 0.0017928288 * n, n * 0.00099602)
        } else {
            let n = (v - 512) as f32;
            (0.7 + 0.000390625 * n, 0.9 + 0.00017578124 * n, 0.5)
        }
    }

    /// FEEDBACK (8-bit) inside the pair: the left half runs 0 .. low, the right low .. high.
    pub fn feedback(pair: (f32, f32), k: i32) -> f32 {
        let (a, b) = pair;
        if (k as f32 * 0.0039216) < 0.5 { k as f32 * 0.0078431 * a } else { a + (b - a) * (k as f32 * 0.0078431 - 1.0) }
    }

    /// MOD (8-bit): the wobble's step per sample (times the rate) and its depth.
    pub fn mod_law(k: i32) -> (f32, f32) {
        if k > 125 && k < 131 {
            return (0.0, 0.0);
        }
        let (n, d) = if k <= 125 { (k as f32, 4.0 - 0.031746 * k as f32) } else { ((k - 130) as f32, (k - 130) as f32 * 0.015873) };
        (n * n * 0.0060469 + 4.0, d.min(1.0) * 0.25)
    }

    /// One frame. `main`, `fbk`, `modk`: the knobs 0..1; `beat_ms` signed (the direction);
    /// `on`: the effect is on (off ramps the wet sound out).
    #[inline]
    pub fn process(&mut self, l: f32, r: f32, main: f32, fbk: f32, modk: f32, beat_ms: f32, freeze: bool, on: bool, rate: f32) -> (f32, f32) {
        if self.lines.is_empty() {
            return (l, r);
        }
        if on && !self.was_on {
            self.reset();
        }
        self.was_on = on;
        let v = (main.clamp(0.0, 1.0) * 1023.9) as i32;
        let (fa, fb, mut wet_t) = EnigmaJet::level_law(v);
        let fbg = EnigmaJet::feedback((fa, fb), (fbk.clamp(0.0, 1.0) * 255.9) as i32);
        let (ms, depth) = EnigmaJet::mod_law((modk.clamp(0.0, 1.0) * 255.9) as i32);
        let mut dry_t = 1.0 - wet_t;
        if !on || !self.filled {
            wet_t = 0.0;
            dry_t = 1.0;
        }
        let bm = (beat_ms as i32) as f32;
        let secs = if bm == 0.0 { 0.001 } else { bm * 0.001 };
        if !freeze {
            self.phase -= 1.0 / (secs * rate);
            self.phase = self.phase.rem_euclid(1.0);
        }
        let mut mdl = 1.0;
        if depth > 0.0 {
            self.wobble += ms / rate;
            if self.wobble >= 2.0 {
                self.wobble -= 2.0;
            }
            let tri = if self.wobble <= 1.0 { self.wobble } else { 2.0 - self.wobble };
            mdl = (tri - 0.5) * depth + 1.0;
        }
        // The split: the highs go to the lines, the lows (unity at DC) stay.
        let mut hi = [0.0f32; 2];
        let mut lo = [0.0f32; 2];
        let c = self.hp;
        for (ch, x) in [l, r].into_iter().enumerate() {
            let z = &mut self.z[ch];
            let wn = x as f64 - z[0] * c[3] - z[1] * c[4];
            hi[ch] = (c[0] * wn + c[1] * z[0] + c[2] * z[1]) as f32;
            lo[ch] = (wn * (1.0 + c[3] + c[4])) as f32;
            z[1] = z[0];
            z[0] = wn;
        }
        let mut taps = [0.0f32; 2];
        if self.filled {
            for k in 0..4 {
                let ph = (self.phase + [0.0, 0.5, 0.25, 0.75][k]).fract();
                let win = if ph < 0.1 { ph * 10.0 } else if ph > 0.9 { 10.0 - 10.0 * ph } else { 1.0 };
                let d = ((self.min + ph * self.range) * mdl).clamp(self.min, self.min + self.range);
                let mut pos = self.w as f32 - d;
                if pos < 0.0 {
                    pos += self.len as f32;
                }
                let i = (pos as usize).min(self.len - 1);
                let j = if i + 1 >= self.len { 0 } else { i + 1 };
                let line = &mut self.lines[k * EJ_LINE..k * EJ_LINE + self.len];
                let t = win * win * (line[i] + (pos - i as f32) * (line[j] - line[i]));
                let ch = if k == 0 || k == 1 { 0 } else { 1 };
                let sign = if k == 0 || k == 2 { 1.0 } else { -1.0 };
                line[self.w] = hi[ch] + sign * t * fbg;
                taps[ch] += sign * t;
            }
        } else {
            for k in 0..4 {
                self.lines[k * EJ_LINE + self.w] = [l, l, r, r][k];
            }
        }
        self.w += 1;
        if self.w >= self.len {
            self.w = 0;
            self.filled = true;
        }
        let ramp = |g: &mut f32, t: f32, k: f32| {
            let d = (t - *g) * k;
            *g += d;
            if (d > 0.0 && *g + 0.00032 > t) || (d < 0.0 && *g - 0.00032 < t) {
                *g = t;
            }
        };
        ramp(&mut self.wet, wet_t, self.step);
        ramp(&mut self.dry, dry_t, self.step);
        (l * self.dry + (lo[0] + taps[0]) * self.wet, r * self.dry + (lo[1] + taps[1]) * self.wet)
    }
}

/// MOBIUS SAW / TRI (`BeatFxMobius::bfx_shepard_tone*`): an endless riser (or faller) added
/// to the track. Four voices two octaves apart (1, 4, 16 and 64 times a base that climbs from
/// 55 to 110 Hz once per beat value) step up an octave on every turn; a voice fades in over
/// its first octave and out over its seventh, then rests for a turn and starts again at the
/// bottom. LEVEL/DEPTH sets the tone's level, ECHO LEVEL the send into a two-line echo (80.5
/// and 12.3 ms, the second fed from the first) and its feedback, MOD wobbles the pitch.
/// Output = input + 0.1 x (tone + 0.3 x echo).
pub struct Mobius {
    l1: Vec<f32>,
    l2: Vec<f32>,
    n1: usize,
    n2: usize,
    i1: usize,
    i2: usize,
    filled: bool,
    ramp: f32,
    wobble: f32,
    oct: [i32; 4],
    mult: [f32; 4],
    ph: [f32; 4],
    amp: [f32; 4],
    up: [bool; 4],
    /// Tone on (`f716`), echo send (`f720`), level (`f776`), and their step per sample.
    on_g: f32,
    send: f32,
    level: f32,
    k: f32,
    was_on: bool,
}

impl Mobius {
    pub fn new() -> Mobius {
        let mut m = Mobius { l1: vec![0.0; 16000], l2: vec![0.0; 2500], ..Mobius::empty() };
        m.prepare(48000.0);
        m
    }

    pub fn empty() -> Mobius {
        Mobius { l1: Vec::new(), l2: Vec::new(), n1: 1, n2: 1, i1: 0, i2: 0, filled: false, ramp: 0.0, wobble: 0.0, oct: [0, 2, 4, 6], mult: [1.0, 4.0, 16.0, 64.0], ph: [0.0; 4], amp: [0.0, 1.0, 1.0, 0.0], up: [true; 4], on_g: 0.0, send: 0.0, level: 0.0, k: 0.0, was_on: false }
    }

    pub fn prepare(&mut self, rate: f32) {
        let fs = rate.max(8000.0);
        self.n1 = ((fs * 0.080544) as usize).clamp(1, self.l1.len().max(1));
        self.n2 = ((fs * 0.012336) as usize).clamp(1, self.l2.len().max(1));
        self.k = 300.0 / fs;
        self.reset(false);
    }

    /// The voices' start (on every switch-on): octaves 0, 2, 4, 6; the first voice at the
    /// ramp's level, the top one silent.
    fn start(&mut self, down: bool) {
        self.ramp = if down { 1.0 } else { 0.0 };
        self.oct = [0, 2, 4, 6];
        self.mult = [1.0, 4.0, 16.0, 64.0];
        self.ph = [0.0; 4];
        self.amp = [self.ramp, 1.0, 1.0, 0.0];
        for v in 0..4 {
            self.up[v] = if down { self.oct[v] >= 1 } else { self.oct[v] <= 5 };
        }
    }

    pub fn reset(&mut self, down: bool) {
        self.l1.iter_mut().for_each(|v| *v = 0.0);
        self.l2.iter_mut().for_each(|v| *v = 0.0);
        self.i1 = 0;
        self.i2 = 0;
        self.filled = false;
        self.wobble = 0.0;
        self.on_g = 0.0;
        self.send = 0.0;
        self.level = 0.0;
        self.start(down);
    }

    /// LEVEL/DEPTH (10-bit) to the tone's gain.
    pub fn level_law(v: i32) -> f32 {
        let g = if v < 10 {
            0.0
        } else if v <= 511 {
            (v - 10) as f32 * 0.001992
        } else if v > 1012 {
            4.37
        } else {
            ((v - 512) * (v - 512)) as f32 * 0.000013373 + 1.0
        };
        g * 0.211
    }

    /// ECHO LEVEL (0..1) to (the send's ceiling, the echo's feedback).
    pub fn echo_law(e: f32) -> (f32, f32) {
        let v7 = 2.0 * e;
        let v9 = if v7 <= 1.0 { v7 } else { 0.4 * e + 0.8 };
        let v10 = if v7 >= 0.9 { v9 } else { 0.9 };
        (v7 * v7, v10 * 0.8)
    }

    /// One frame: (left, right) to add to the input. `beat_ms` signed (the direction).
    #[inline]
    pub fn process(&mut self, saw: bool, main: f32, echo: f32, modk: f32, beat_ms: f32, freeze: bool, on: bool, rate: f32) -> (f32, f32) {
        if self.l1.is_empty() {
            return (0.0, 0.0);
        }
        let down = beat_ms < 0.0;
        if on && !self.was_on {
            self.start(down);
        }
        self.was_on = on;
        let fs = rate;
        let bm = (beat_ms as i32) as f32;
        let secs = if bm == 0.0 { 0.001 } else { bm * 0.001 };
        let slope = if freeze { 0.0 } else { 1.0 / (secs * fs) };
        let mut wrapped = false;
        self.ramp += slope;
        if slope > 0.0 && self.ramp >= 1.0 {
            self.ramp -= 1.0;
            wrapped = true;
        } else if slope < 0.0 && self.ramp < 0.0 {
            self.ramp += 1.0;
            wrapped = true;
        }
        let (rate_m, depth) = EnigmaJet::mod_law((modk.clamp(0.0, 1.0) * 255.9) as i32);
        let mut m = 1.0;
        if depth > 0.0 {
            self.wobble += rate_m / fs;
            if self.wobble >= 2.0 {
                self.wobble -= 2.0;
            }
            let tri = if self.wobble <= 1.0 { self.wobble } else { 2.0 - self.wobble };
            m = (tri - 0.5) * depth + 1.0;
        }
        let x = self.ramp;
        let f = m * (x * 55.0 * (x * 0.34315 + 0.65685) + 55.0) / fs;
        let rising = slope >= 0.0;
        let mut tone = 0.0;
        for v in 0..4 {
            if wrapped {
                if self.oct[v] == 7 {
                    // A rested voice starts again at the bottom (or the top going down).
                    self.oct[v] = if rising { 0 } else { 6 };
                    self.mult[v] = if rising { 1.0 } else { 64.0 };
                    self.ph[v] = 0.0;
                    self.up[v] = true;
                } else if rising {
                    self.mult[v] *= 2.0;
                    if self.oct[v] > 4 {
                        self.up[v] = false;
                    }
                    self.oct[v] += 1;
                } else {
                    self.mult[v] *= 0.5;
                    self.oct[v] -= 1;
                    if self.oct[v] < 0 {
                        self.amp[v] = 0.0;
                        self.oct[v] = 7;
                    } else if self.oct[v] == 0 {
                        self.up[v] = false;
                    }
                }
            }
            if self.oct[v] > 6 {
                continue;
            }
            let sl = slope.abs();
            self.amp[v] = (self.amp[v] + if self.up[v] { sl } else { -sl }).clamp(0.0, 1.0);
            let inc = f * self.mult[v];
            self.ph[v] += inc;
            let a2 = self.amp[v] * self.amp[v];
            if saw {
                let sv = if self.ph[v] >= 1.0 {
                    self.ph[v] -= 1.0;
                    1.0 - self.ph[v] / inc.max(1e-9)
                } else {
                    self.ph[v]
                };
                tone += a2 * 1.4 * (0.5 - sv);
            } else {
                if self.ph[v] >= 1.0 {
                    self.ph[v] -= 1.0;
                }
                let t = if 2.0 * self.ph[v] > 1.0 { 2.0 - 2.0 * self.ph[v] } else { 2.0 * self.ph[v] };
                tone += a2 * 2.0 * (t - 0.5);
            }
        }
        let k = self.k;
        self.on_g = (self.on_g + if on { k } else { -k }).clamp(0.0, 1.0);
        let lt = if on { Mobius::level_law((main.clamp(0.0, 1.0) * 1023.9) as i32) } else { 0.0 };
        let d = (lt - self.level) * k;
        self.level += d;
        if (d > 0.0 && self.level + 0.00032 > lt) || (d < 0.0 && self.level - 0.00032 < lt) {
            self.level = lt;
        }
        let (ceil, fb) = Mobius::echo_law(echo.clamp(0.0, 1.0));
        let dry_tone = self.on_g * tone * self.level;
        let (mut el, mut er) = (0.0, 0.0);
        if self.filled {
            self.send = (self.send + if self.send <= ceil { k } else { -k }).clamp(0.0, ceil);
            let a = self.l1[self.i1];
            let b = self.l2[self.i2];
            self.l1[self.i1] = fb * b + dry_tone * self.send;
            self.l2[self.i2] = a;
            el = a * 0.3;
            er = b * 0.3;
        } else {
            self.l1[self.i1] = dry_tone;
            self.l2[self.i2] = 0.0;
        }
        self.i1 += 1;
        if self.i1 >= self.n1 {
            self.i1 = 0;
            self.filled = true;
        }
        self.i2 = if self.i2 + 1 < self.n2 { self.i2 + 1 } else { 0 };
        ((dry_tone + el) * 0.1, (dry_tone + er) * 0.1)
    }
}

/// The pitch shifter's percent law (`setPitchPercentage`): the shift, ratio - 1.
pub fn percent_shift(pct: f32) -> f32 {
    let p = pct.clamp(-200.0, 200.0);
    if p < 0.0 { 0.005 * p } else if p < 100.0 { 0.01 * p } else { 0.02 * p - 1.0 }
}

/// A biquad in transposed direct form II: b0, b1, b2, a1, a2 (a0 = 1).
#[inline]
pub fn bq(c: &[f32; 5], z: &mut [f32], x: f32) -> f32 {
    let y = c[0] * x + z[0];
    z[0] = c[1] * x + z[1] - c[3] * y;
    z[1] = c[2] * x - c[4] * y;
    y
}

/// `IirFilter::calcCoefLpf2ndOrder`: bilinear with the cutoff pre-warped.
pub fn iir_lpf2(fc: f64, q: f64, rate: f64) -> [f32; 5] {
    let t = (fc.min(0.49 * rate) * PI / rate).tan();
    let k = t / q.max(0.01);
    let t2 = t * t;
    let n = 1.0 / (k + t2 + 1.0);
    [(n * t2) as f32, (2.0 * n * t2) as f32, (n * t2) as f32, (n * (2.0 * t2 - 2.0)) as f32, (n * (1.0 - k + t2)) as f32]
}

/// `IirFilter::calcCoefHpf2ndOrder`.
pub fn iir_hpf2(fc: f64, q: f64, rate: f64) -> [f32; 5] {
    let t = (fc.min(0.49 * rate) * PI / rate).tan();
    let k = t / q.max(0.01);
    let t2 = t * t;
    let n = 1.0 / (k + t2 + 1.0);
    [n as f32, (-2.0 * n) as f32, n as f32, (n * (2.0 * t2 - 2.0)) as f32, (n * (1.0 - k + t2)) as f32]
}

/// `IirFilter::calcCoefBpf2ndOrder`: a band-pass of peak gain 1 (b1 = 0).
pub fn iir_bpf2(fc: f64, q: f64, rate: f64) -> [f32; 5] {
    let q = q.max(0.01);
    let t = (fc.min(0.49 * rate) * PI / rate).tan();
    let v7 = ((1.0 / (q * q) + 4.0).sqrt() - 1.0 / q) * (0.0795774715 * t);
    let v8 = 1.0 / q * 0.159154943 * t;
    let v9 = v7 + v8;
    let v10 = v8 * std::f64::consts::TAU;
    let v11 = v7 * 39.4784176 * v9;
    let n = 1.0 / (v10 + 1.0 + v11);
    [(n * v10) as f32, 0.0, (-n * v10) as f32, ((2.0 * v11 - 2.0) * n) as f32, (n * (1.0 - v10 + v11)) as f32]
}

/// `IirFilter::calcCoefLpf1stOrder` / `Hpf1stOrder` as b0, b1, a1.
pub fn iir_1st(fc: f64, rate: f64, hp: bool) -> [f32; 3] {
    let t = (fc.min(0.49 * rate) * PI / rate).tan();
    let n = 1.0 / (t + 1.0);
    let a1 = -(n - t * n);
    if hp { [n as f32, -n as f32, a1 as f32] } else { [(n * t) as f32, (n * t) as f32, a1 as f32] }
}

/// `Filter::make_lpf2(fc, damping, gain)` (the three-number form the CFX filter uses): the
/// cutoff is not pre-warped, `damping` is 1/Q and `gain` scales the whole response.
pub fn make_lpf2(fc: f64, damping: f64, gain: f64, rate: f64) -> [f32; 5] {
    let tt = 1.0 / rate;
    let w = fc * std::f64::consts::TAU;
    let v5 = fc * 4.0 * PI * damping * tt;
    let v6 = w * w * tt * tt;
    let n = 1.0 / (v5 + 4.0 + v6);
    let b = v6 * gain * n;
    [b as f32, (2.0 * b) as f32, b as f32, ((2.0 * v6 - 8.0) * n) as f32, (n * (4.0 - v5 + v6)) as f32]
}

/// `Filter::make_hpf2(fc, damping, gain)`.
pub fn make_hpf2(fc: f64, damping: f64, gain: f64, rate: f64) -> [f32; 5] {
    let tt = 1.0 / rate;
    let w = fc * std::f64::consts::TAU;
    let v5 = fc * 4.0 * PI * damping * tt;
    let v6 = w * w * tt * tt;
    let n = 1.0 / (v5 + 4.0 + v6);
    let b = gain * 4.0 * n;
    [b as f32, (-2.0 * b) as f32, b as f32, ((2.0 * v6 - 8.0) * n) as f32, (n * (4.0 - v5 + v6)) as f32]
}

/// `Filter::make_lpf2(fc, resonance dB, prewarp)` / `make_hpf2`: Q = 10^(dB / 20).
pub fn make_pass2_db(fc: f64, db: f64, prewarp: bool, hp: bool, rate: f64) -> [f32; 5] {
    let tt = 1.0 / rate;
    let w = if prewarp { rate * 2.0 * (fc.min(0.49 * rate) * PI * tt).tan() } else { fc * std::f64::consts::TAU };
    let q = 10f64.powf(db * 0.05);
    let v10 = w * w * tt * tt;
    let v11 = w * 2.0 * tt / q;
    let n = 1.0 / (v10 + 4.0 + v11);
    let (b0, b1) = if hp { (4.0 * n, -8.0 * n) } else { (n * v10, 2.0 * n * v10) };
    [b0 as f32, b1 as f32, b0 as f32, (2.0 * (v10 - 4.0) * n) as f32, (n * (v10 + 4.0 - v11)) as f32]
}

/// HELIX's held loop speed (`HelixCore::processEffect` 0x1029f8630, state 3): it moves an
/// octave toward `target` (recorded length / new length) every (V x 1.78 + 0.01) s going up
/// or (V x 1.38 + 0.01) s going down, V = TRACK SPEED (0.5 in multi mode), and stops there.
pub fn helix_speed(cur: f64, target: f64, v: f32, rate: f32) -> f64 {
    if cur == target {
        return cur;
    }
    let up = cur < target;
    let secs = v.clamp(0.0, 1.0) as f64 * if up { 1.78 } else { 1.38 } + 0.01;
    let k = 2f64.powf((if up { 1.0 } else { -1.0 }) / (secs * rate as f64));
    let next = cur * k;
    if (up && next >= target) || (!up && next <= target) { target } else { next }
}

/// The held loop's level at speed `s`: 1 up to twice the speed, then 1 / (s - 1), at least 0.25.
pub fn helix_gain(s: f64) -> f32 {
    if s > 2.0 { (1.0 / (s - 1.0)).max(0.25) as f32 } else { 1.0 }
}

/// `SpiralBuffer` (0x1028d5fa0; `calcAccel` 0x1028d62f4, `getDelayDataInternal`
/// 0x1028d6860): how SPIRAL's read point follows a new delay time. It does not jump: it moves
/// at a speed (frames of delay per frame, -2 .. 0.8) that accelerates toward a limit set by the
/// distance left, re-planned every 30 frames while it moves, and stops on the target once it
/// is within a frame at a speed under 0.011. A small triangle wobble rides on the read point
/// (±1.04e-5 x rate frames, 1 s period, for Beat FX SPIRAL; ±2.08e-5 x rate, 20 s, for SPIRAL
/// UP / DOWN). Mode 0 = Beat FX SPIRAL, 1 = SPIRAL UP, 2 = SPIRAL DOWN (`RmxSpiral`).
#[derive(Clone, Copy, Debug)]
pub struct SpiralMotion {
    mode: u8,
    rate: f64,
    /// Target and current delay in frames (`+48`, `+52`).
    pub target: f32,
    pub cur: f32,
    speed: f64,
    accel: f64,
    limit: f64,
    moving: bool,
    tri: f32,
    up: bool,
    tri_step: f32,
    tri_max: f32,
    k_pos: f32,
    k_neg: f32,
    max_pos: i32,
    max_neg: i32,
    /// One beat in frames (modes 1 and 2).
    pub beat: i32,
    left: usize,
}

impl SpiralMotion {
    pub fn new(rate: f32, mode: u8) -> SpiralMotion {
        let r = rate as f64;
        let (amp, half) = if mode == 0 { (0.000010416667, 0.5) } else { (0.000020833334, 10.0) };
        let (tmax, tmin) = ((r * amp) as f32, (r * -amp) as f32);
        let (k_pos, k_neg) = if mode == 0 { ((0.5 / r) as f32, (3.33333333 / r) as f32) } else { ((1000.0 / r) as f32, (1000.0 / r) as f32) };
        SpiralMotion { mode, rate: r, target: 0.0, cur: 0.0, speed: 0.0, accel: 0.0, limit: 0.0, moving: false, tri: 0.0, up: true, tri_step: (tmax - tmin) / (r * half) as f32, tri_max: tmax, k_pos, k_neg, max_pos: (4.0 * r) as i32, max_neg: (-2.0 * r) as i32, beat: r as i32 / 2, left: 0 }
    }

    /// Made again (at rest) when the rate or the mode differs.
    pub fn setup(&mut self, rate: f32, mode: u8) {
        if self.mode != mode || self.rate != rate as f64 {
            *self = SpiralMotion::new(rate, mode);
        }
    }

    /// `DelayBufferBase::setDelaySampleNum`: the first time (or with `jump`) the read point
    /// is put there at once and its motion cleared (`onDelayTimeChanged(true)`).
    pub fn set_target(&mut self, frames: f32, jump: bool) {
        self.target = frames;
        if self.cur <= 0.0 || jump {
            self.cur = frames;
            self.moving = false;
            self.speed = 0.0;
            self.accel = 0.0;
            self.limit = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.cur = 0.0;
        self.target = 0.0;
        self.speed = 0.0;
        self.accel = 0.0;
        self.limit = 0.0;
        self.moving = false;
        self.tri = 0.0;
        self.up = true;
        self.left = 0;
    }

    /// `calcAccel` for the distance `a2` (target - current).
    pub fn calc_accel(&mut self, mut a2: f32) {
        let sp = self.speed;
        let ms = (self.rate * 0.001) as f32;
        let beat = self.beat.max(1) as f32;
        if a2 > 1.0 {
            match self.mode {
                0 => {
                    a2 = a2.min(self.max_pos as f32);
                    if sp <= 0.01 || ms <= a2 {
                        let v = (a2 / self.max_pos as f32) as f64;
                        self.limit = 0.21 - 0.2 * (1.0 - v).powi(3);
                        self.accel = (self.limit - sp) * self.k_pos as f64;
                    } else {
                        self.limit = 0.01;
                        self.accel = (0.01 - sp) / a2 as f64;
                    }
                }
                1 => {
                    a2 = a2.max(beat);
                    self.limit = 0.11;
                    self.accel = (0.11 - sp) * self.k_pos as f64;
                }
                _ => {
                    a2 = a2.max(beat);
                    if sp > 0.01 && a2 < ms {
                        self.limit = 0.01;
                        a2 = a2.max((self.rate * 0.00066667) as f32);
                        self.accel = (0.01 - sp) / a2 as f64;
                    } else {
                        let v = (a2 / beat) as f64;
                        self.limit = 0.180010448 - 0.170010448 * (1.0 - v).powi(3);
                        self.accel = (self.limit - sp) * self.k_pos as f64;
                    }
                }
            }
        } else if a2 < -1.0 {
            match self.mode {
                0 => {
                    a2 = a2.max(self.max_neg as f32);
                    if sp < -0.01 && -ms < a2 {
                        self.limit = -0.01;
                        self.accel = (sp + 0.01) / a2 as f64;
                    } else {
                        let v = (a2 / self.max_neg as f32) as f64;
                        self.limit = 0.401568621 * (1.0 - v).powi(3) - 0.411568621;
                        self.accel = (self.limit - sp) * self.k_neg as f64;
                    }
                }
                m => {
                    a2 = a2.max(-beat);
                    if sp < -0.01 && -ms < a2 {
                        self.limit = -0.01;
                        self.accel = (sp + 0.01) / a2 as f64;
                    } else {
                        let v = (a2 / beat) as f64;
                        let (k, c) = if m == 1 { (0.701917112, -0.711917112) } else { (0.170010448, -0.180010448) };
                        self.limit = k * (1.0 + v).powi(3) + c;
                        self.accel = (self.limit - sp) * self.k_neg as f64;
                    }
                }
            }
        }
        if sp > -0.011 && sp < 0.011 && a2 > -1.0 && a2 < 1.0 {
            // Arrived: the read point sits on the target and stops.
            self.cur = self.target;
            self.moving = false;
            self.speed = 0.0;
            self.accel = 0.0;
            self.limit = 0.0;
        }
    }

    /// The delay (frames) to read the next frame at.
    pub fn next(&mut self) -> f32 {
        if self.left == 0 {
            // A new piece: re-planned when the target is elsewhere (30 frames while moving).
            let diff = self.target - self.cur;
            if diff.abs() > 1.1921e-7 {
                self.moving = true;
                self.calc_accel(diff);
            }
            self.left = 30;
        }
        self.left -= 1;
        if self.up {
            self.tri += self.tri_step;
            if self.tri > self.tri_max {
                self.up = false;
                self.tri = 2.0 * self.tri_max - self.tri;
            }
        } else {
            self.tri -= self.tri_step;
            if self.tri < -self.tri_max {
                self.up = true;
                self.tri = -2.0 * self.tri_max - self.tri;
            }
        }
        self.cur = ((self.speed + self.cur as f64).max(44.0999985)) as f32;
        self.speed = (self.accel + self.speed).clamp(-2.0, 0.8);
        let d = self.cur + self.tri;
        if (self.accel <= 0.0 && self.speed <= self.limit) || (self.accel > 0.0 && self.speed >= self.limit) {
            self.accel = 0.0;
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, f: f32) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * f * std::f32::consts::TAU / 48000.0).sin() * 0.5).collect()
    }


    #[test]
    fn spiral_motion_follows_rekordboxs_speed_law() {
        // Beat FX SPIRAL at 48 kHz: half a second to a second. The read point starts slowly
        // (the limit for 24,000 frames of 192,000 is 0.21 - 0.2 x 0.875^3 = 0.076 a frame,
        // approached with a 2 s time constant), never jumps, and arrives.
        let mut m = SpiralMotion::new(48000.0, 0);
        m.set_target(24000.0, false);
        assert_eq!(m.cur, 24000.0);
        m.set_target(48000.0, false);
        let mut last = m.next();
        let mut max_step = 0.0f32;
        let mut t = 0;
        while (m.cur - 48000.0).abs() > 0.5 || m.moving {
            let d = m.next();
            if m.moving {
                max_step = max_step.max((d - last).abs());
            }
            last = d;
            t += 1;
            assert!(t < 48000 * 60, "never arrives");
        }
        // (Under a tenth of a frame per frame while it moves; the last step onto the target
        // is under a frame, as rekordbox's snap.)
        assert!(max_step < 0.1, "{max_step}");
        assert!(m.limit == 0.0 && m.speed == 0.0);
        // It takes seconds, as a tape would: 12.8 s here.
        assert!((t as f32 / 48000.0 - 12.83).abs() < 0.1, "{t}");
        // The first plan's numbers.
        let mut m = SpiralMotion::new(48000.0, 0);
        m.set_target(24000.0, false);
        m.set_target(48000.0, false);
        m.calc_accel(24000.0);
        assert!((m.limit - (0.21 - 0.2 * 0.875f64.powi(3))).abs() < 1e-9 && (m.accel - m.limit * 0.5 / 48000.0).abs() < 1e-12);
        // Shorter: toward a quarter of the time the limit is negative and faster.
        m.speed = 0.0;
        m.calc_accel(-24000.0);
        assert!((m.limit - (0.401568621 * 0.75f64.powi(3) - 0.411568621)).abs() < 1e-9);
        // SPIRAL UP (mode 1): a fixed 0.11 going up; DOWN (2) by the beat.
        let mut u = SpiralMotion::new(48000.0, 1);
        u.beat = 24000;
        u.calc_accel(1000.0);
        assert!((u.limit - 0.11).abs() < 1e-12);
        u.speed = 0.0;
        u.calc_accel(-12000.0);
        assert!((u.limit - (0.701917112 * 0.125 - 0.711917112)).abs() < 1e-9);
        // The wobble: ±0.5 frame at 48 kHz for SPIRAL, a 1 s period.
        let mut w = SpiralMotion::new(48000.0, 0);
        w.set_target(4800.0, false);
        let v: Vec<f32> = (0..48000).map(|_| w.next() - 4800.0).collect();
        let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |a, b| (a.0.min(*b), a.1.max(*b)));
        assert!((hi - 0.5).abs() < 0.01 && (lo + 0.5).abs() < 0.01, "{lo} {hi}");
    }

    /// The strongest frequency (0.5 Hz steps between 50 Hz and 1 kHz) of `x` at `rate`.
    pub(crate) fn peak_at(x: &[f32], rate: f32) -> f32 {
        let mut best = (0.0, 0.0f64);
        for f2 in 100..2000 {
            let f = f2 as f64 * 0.5;
            let w = f * std::f64::consts::TAU / rate as f64;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, &v) in x.iter().enumerate() {
                re += v as f64 * (w * i as f64).cos();
                im += v as f64 * (w * i as f64).sin();
            }
            if re * re + im * im > best.1 {
                best = (f as f32, re * re + im * im);
            }
        }
        best.0
    }

    /// Run `x` (mono) through a shifter at `pct` in rekordbox's 512-frame blocks.
    fn shifted(x: &[f32], rate: f32, pct: f32) -> Vec<f32> {
        let mut p = PitchShifter::new(rate);
        p.set_percent(pct);
        let mut y = Vec::with_capacity(x.len());
        for b in x.chunks(PitchShifter::BLOCK) {
            p.begin_block(b.iter().map(|&v| (v, v)));
            for &v in b {
                y.push(p.process(v, v).0);
            }
        }
        y
    }

    #[test]
    fn pitch_shifter_constants_and_ratios() {
        let p = PitchShifter::new(48000.0);
        assert_eq!((p.xf, p.min_d, p.max_d, p.start_d), (144, 624, 1104, 1440));
        assert_eq!(percent_shift(-100.0), -0.5);
        assert_eq!(percent_shift(100.0), 1.0);
        assert_eq!(percent_shift(200.0), 3.0);
        assert_eq!(percent_shift(50.0), 0.5);
        // A 220 Hz tone in 512-frame blocks: the spectrum's peak lands on the shifted pitch.
        for rate in [44100.0f32, 48000.0] {
            let p0 = PitchShifter::new(rate);
            let x: Vec<f32> = (0..(rate as usize * 2)).map(|i| (i as f32 * 220.0 * std::f32::consts::TAU / rate).sin() * 0.5).collect();
            for (pct, want) in [(-25.0, 192.5), (-100.0, 110.0), (100.0, 440.0), (-50.0, 165.0), (50.0, 330.0), (26.0, 277.2), (0.0, 220.0)] {
                let y = shifted(&x, rate, pct);
                let n = rate as usize;
                let got = peak_at(&y[n / 2..n / 2 + n], rate);
                // With rekordbox's blocks no zero-crossing jump fires on this tone (the last
                // accepted crossing is never more than 13 ms old inside a block), so the read
                // point makes only its forced jumps across the window: grains of G frames, each
                // jumping J frames of input. The line of the shifted tone moves by the jump's
                // phase per grain, frac(220 J / fs) / G x fs (to the nearer side).
                let r = percent_shift(pct);
                let pred = if r == 0.0 {
                    220.0
                } else {
                    let (min, max, xf) = (p0.min_d as f32, p0.max_d as f32, p0.xf as f32);
                    let (j, g, sign) = if r < 0.0 { let low = min + (r * xf).round(); (max - low, (max - low) / -r, 1.0) } else { let high = max + (r * xf).round(); (high - min, (high - min) / r, -1.0) };
                    let c = sign * 220.0 * j / rate;
                    220.0 * (1.0 + r) + (c - c.round()) * rate / g
                };
                eprintln!("PS {rate} Hz {pct}%: peak {got} Hz; ratio alone {want} Hz; with the forced jumps' phase {pred:.1} Hz");
                assert!((got - pred).abs() <= 1.0, "{rate} {pct}%: {got} vs {pred}");
                assert!(y.iter().all(|v| v.abs() < 0.8));
            }
        }
    }

    #[test]
    fn pitch_shifter_stays_bounded_on_kick_and_bass() {
        // A decaying 55 Hz kick every half second over a 110 Hz bass and a 2 kHz tone.
        let rate = 44100.0f32;
        let x: Vec<f32> = (0..(rate as usize * 4))
            .map(|i| {
                let t = i as f32 / rate;
                let k = t % 0.5;
                (55.0 * std::f32::consts::TAU * k).sin() * (-k * 12.0).exp() * 0.8 + (110.0 * std::f32::consts::TAU * t).sin() * 0.3 + (2000.0 * std::f32::consts::TAU * t).sin() * 0.1
            })
            .collect();
        for pct in [-100.0, -50.0, -25.0, 26.0, 50.0, 100.0, 200.0] {
            let y = shifted(&x, rate, pct);
            let peak = y.iter().fold(0.0f32, |a, v| a.max(v.abs()));
            assert!(y.iter().all(|v| v.is_finite()) && peak < 1.3, "{pct}%: {peak}");
        }
    }

    #[test]
    fn pitch_shifter_at_zero_is_a_delay_and_mutes_at_the_bottom() {
        let mut p = PitchShifter::new(48000.0);
        let x = tone(9600, 440.0);
        let y: Vec<f32> = x.iter().map(|&v| p.process(v, v).0).collect();
        // Start-up: dry for the 30 ms fill, then the line at its fill delay.
        assert_eq!(y[100], x[100]);
        let d = 1440;
        assert!((y[6000] - x[6000 - d]).abs() < 0.02, "{} {}", y[6000], x[6000 - d]);
        let mut p = PitchShifter::new(48000.0);
        p.mute_at_bottom(true);
        p.set_shift(-1.0);
        let y: Vec<f32> = x.iter().map(|&v| p.process(v, v).0).collect();
        assert!(y[4000..].iter().all(|v| v.abs() < 1e-3));
    }

    #[test]
    fn filter_shapes() {
        // make_lpf2 / make_hpf2 at DC and Nyquist.
        let dc = |c: &[f32; 5]| (c[0] + c[1] + c[2]) / (1.0 + c[3] + c[4]);
        let ny = |c: &[f32; 5]| (c[0] - c[1] + c[2]) / (1.0 - c[3] + c[4]);
        let l = make_lpf2(1000.0, 0.5, 1.0, 48000.0);
        assert!((dc(&l) - 1.0).abs() < 1e-4 && ny(&l).abs() < 1e-4);
        let h = make_hpf2(1000.0, 0.5, 0.8, 48000.0);
        assert!(dc(&h).abs() < 1e-4 && (ny(&h) - 0.8).abs() < 0.01);
        let b = iir_bpf2(1000.0, 1.0, 48000.0);
        assert!(dc(&b).abs() < 1e-5 && ny(&b).abs() < 1e-5);
        let l = iir_lpf2(1000.0, 0.7071, 48000.0);
        assert!((dc(&l) - 1.0).abs() < 1e-4);
    }
}
