//! rekordbox's forward echo core, as `BeatFxEcho` runs it for DELAY, ECHO, LOW CUT ECHO and
//! PING PONG: `EchoCore` (0x1029b6954 .. 0x1029b72fc). Each side is its own `DelayModule`
//! (`crate::fx_rev::DLine`: whole frames, the read gate shut for a new time), and two
//! `SmoothFloat`s of 3.3333 ms: the feedback (made at 0) and the output gain (made at 1, never
//! changed by `BeatFxEcho`). `processEffect` takes the left side's whole call, then the
//! right's, each in pieces of at most the line's delay (`getMaxAudioCallbackBlockSize`):
//! read the piece, feed `read x feedback + input` back into the line, output `read x gain`.
//!
//! A piece is never longer than the delay, so its read only needs frames written before it:
//! here the piece is read when its first frame comes and the input is written a frame at a
//! time, which gives rekordbox's numbers from `crate::fx::Slot`'s frame loop. Nothing here
//! allocates after `EchoCore::new`.

use crate::fx_lfo::Sf;
use crate::fx_rev::DLine;

const EPS: f32 = 1.1920929e-7;
/// The longest piece read at once (pieces are cut to this; the audio buffers are shorter).
const PIECE_CAP: usize = 8192;

/// `EchoCore` with its two lines.
pub struct EchoCore {
    rate: f32,
    pub line: [DLine; 2],
    /// `SmoothFloat` 840 / 888 (feedback) and 936 / 984 (output gain).
    fb: [Sf; 2],
    out: [Sf; 2],
    /// The piece read (per side), the place in it, its length, and the frames left in the call.
    piece: [Vec<f32>; 2],
    pos: [usize; 2],
    len: [usize; 2],
    left: [i32; 2],
}

impl EchoCore {
    /// The constructor (`EchoCore(rate, block, 1, 4000)`), at 48 kHz until `setup`.
    pub fn new() -> Box<EchoCore> {
        let mut e = Box::new(EchoCore {
            rate: 0.0,
            line: [DLine::new(), DLine::new()],
            fb: [Sf::new(48000.0); 2],
            out: [Sf::new(48000.0); 2],
            piece: [vec![0.0; PIECE_CAP], vec![0.0; PIECE_CAP]],
            pos: [0; 2],
            len: [0; 2],
            left: [0; 2],
        });
        e.setup(48000.0);
        e
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Made at `rate`: the lines (50 ms, shut), feedback 0, output gain 1.
    pub fn setup(&mut self, rate: f32) {
        self.rate = rate;
        for c in 0..2 {
            self.line[c].setup(rate);
            self.fb[c] = Sf::new(rate);
            self.fb[c].set_time(3.3333);
            self.fb[c].reset(0.0);
            self.out[c] = Sf::new(rate);
            self.out[c].set_time(3.3333);
            self.out[c].reset(1.0);
        }
        self.pos = [0; 2];
        self.len = [0; 2];
        self.left = [0; 2];
    }

    /// `updateAndSetParameter` 0x1029b713c: each side's delay (ms; a change goes through the
    /// read gate) and feedback, taken when they differ from what is set.
    pub fn set(&mut self, ms: [f32; 2], fb: [f32; 2]) {
        for c in 0..2 {
            if (ms[c] - self.line[c].ms).abs() >= EPS {
                self.line[c].set_delay(ms[c], true);
            }
            if (fb[c] - self.fb[c].target).abs() >= EPS {
                self.fb[c].set_target(fb[c]);
            }
        }
    }

    /// `reset` 0x1029b70e0 (`onCalledMakeSelected`): the parameters, then the lines cleared at
    /// the delay asked for and the smoothers where they are going.
    pub fn reset(&mut self, ms: [f32; 2], fb: [f32; 2]) {
        self.set(ms, fb);
        for c in 0..2 {
            self.line[c].reset();
            let t = self.fb[c].target;
            self.fb[c].reset(t);
            let t = self.out[c].target;
            self.out[c].reset(t);
        }
        self.pos = [0; 2];
        self.len = [0; 2];
        self.left = [0; 2];
    }

    /// The start of a `processEffect` call of `len` frames (a piece left over from a call cut
    /// short is dropped, as rekordbox starts each call with a new read).
    pub fn call(&mut self, len: usize) {
        self.left = [len.max(1) as i32; 2];
        self.pos = self.len;
    }

    /// One frame: the core's output, and `read x feedback + input` written.
    #[inline]
    pub fn next(&mut self, l: f32, r: f32) -> (f32, f32) {
        let x = [l, r];
        let mut y = [0.0f32; 2];
        for c in 0..2 {
            if self.pos[c] >= self.len[c] {
                if self.left[c] <= 0 {
                    // (Frames past the call: a call of one piece.)
                    self.left[c] = self.line[c].max_block();
                }
                let n = (self.line[c].max_block().min(self.left[c]).max(1) as usize).min(PIECE_CAP);
                self.line[c].read(&mut self.piece[c][..n]);
                self.left[c] -= n as i32;
                self.pos[c] = 0;
                self.len[c] = n;
            }
            let d = self.piece[c][self.pos[c]];
            self.pos[c] += 1;
            let w = d * self.fb[c].take() + x[c];
            self.line[c].write(&[w]);
            y[c] = d * self.out[c].take();
        }
        (y[0], y[1])
    }
}

/// `PitchEchoCore` (0x1029b8970 .. 0x1029b8f98, UP ECHO, DOWN ECHO, PITCH ECHO): two
/// `DelayModule`s at one time, a feedback `SmoothFloat` (3.3333 ms, made at 0) and the pitch
/// shifter. `processEffect` reads both lines a piece at a time (at most the delay), shifts the
/// piece (`processPitchShift`: the piece is the shifter's block), feeds `shifted x feedback +
/// input` back and outputs the shifted sound. The pitch goes in as a whole percent
/// (`setPitchParameter((int)p)`).
pub struct PitchEchoCore {
    rate: f32,
    pub line: [DLine; 2],
    fb: Sf,
    pub ps: crate::fx_core::PitchShifter,
    piece: [Vec<f32>; 2],
    pos: usize,
    len: usize,
    left: i32,
}

impl PitchEchoCore {
    pub fn new() -> Box<PitchEchoCore> {
        let mut e = Box::new(PitchEchoCore {
            rate: 0.0,
            line: [DLine::new(), DLine::new()],
            fb: Sf::new(48000.0),
            ps: crate::fx_core::PitchShifter::new(48000.0),
            piece: [vec![0.0; PIECE_CAP], vec![0.0; PIECE_CAP]],
            pos: 0,
            len: 0,
            left: 0,
        });
        e.setup(48000.0);
        e
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Made at `rate`: lines at 50 ms (shut), feedback 0, the shifter prepared.
    pub fn setup(&mut self, rate: f32) {
        self.rate = rate;
        for l in self.line.iter_mut() {
            l.setup(rate);
        }
        self.fb = Sf::new(rate);
        self.fb.set_time(3.3333);
        self.fb.reset(0.0);
        self.ps.prepare(rate);
        self.pos = 0;
        self.len = 0;
        self.left = 0;
    }

    /// The parameters as `processEffect` takes them at its start: the delay (ms, both lines),
    /// the feedback, the pitch percent (truncated to a whole percent).
    pub fn set(&mut self, ms: f32, fb: f32, pct: f32) {
        if (ms - self.line[0].ms).abs() >= EPS {
            self.line[0].set_delay(ms, true);
            self.line[1].set_delay(ms, true);
        }
        if (fb - self.fb.target).abs() >= EPS {
            self.fb.set_target(fb);
        }
        self.ps.set_percent(pct.trunc());
    }

    /// `reset` 0x1029b8f08 (`onCalledMakeSelected`): the delay, the lines cleared, the feedback
    /// where it is asked to be, the shifter reset.
    pub fn reset(&mut self, ms: f32, fb: f32, pct: f32) {
        self.set(ms, fb, pct);
        for l in self.line.iter_mut() {
            l.reset();
        }
        self.fb.reset(fb);
        self.ps.reset();
        self.ps.set_percent(pct.trunc());
        self.pos = 0;
        self.len = 0;
        self.left = 0;
    }

    /// The start of a `processEffect` call of `len` frames.
    pub fn call(&mut self, len: usize) {
        self.left = len.max(1) as i32;
        self.pos = self.len;
    }

    /// One frame: the shifted repeat, and `shifted x feedback + input` written.
    #[inline]
    pub fn next(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.pos >= self.len {
            if self.left <= 0 {
                self.left = self.line[0].max_block();
            }
            let n = (self.line[0].max_block().min(self.line[1].max_block()).min(self.left).max(1) as usize).min(PIECE_CAP);
            let [a, b] = &mut self.piece;
            self.line[0].read(&mut a[..n]);
            self.line[1].read(&mut b[..n]);
            self.ps.begin_block((0..n).map(|k| (a[k], b[k])));
            self.left -= n as i32;
            self.pos = 0;
            self.len = n;
        }
        let (dl, dr) = (self.piece[0][self.pos], self.piece[1][self.pos]);
        self.pos += 1;
        let (sl, sr) = self.ps.process(dl, dr);
        let g = self.fb.take();
        self.line[0].write(&[sl * g + l]);
        self.line[1].write(&[sr * g + r]);
        (sl, sr)
    }
}

/// MT DELAY's core, `MultiTapDelayCore` (0x102a04fdc; `processWithOffset` 0x102a06448,
/// `updateParameter` 0x102a06198, `isFinishedSmoothing` 0x102a068d4): a 4 s mono ring a side
/// and eight taps at whole milliseconds of k/8 of each side's time (whole ms: base + (int)(2 x
/// base x swing)), each at a `GainSmoother` (3.3 ms, made at 0), summed, then the output gain
/// (`GainSmoother` 712, 3.3 ms). A new time is not glided: the output fades to 0, the times are
/// taken (held to 10 .. 4000 ms) and it fades back. The call is worked in pieces of at most an
/// eighth of the shorter side (and, while fading, the fade's frames left); the input goes into
/// the rings unscaled after each piece.
pub struct MtCore {
    rate: f32,
    ring: [Vec<f32>; 2],
    valid: usize,
    size: usize,
    w: usize,
    taps: [GainSm; 8],
    out: GainSm,
    /// Each side's time in use (ms, `f740` / `f744`) and the change under way (`f736`).
    t: [i32; 2],
    changing: bool,
    /// The parameters: base time (ms), swing per side, tap gains, output gain.
    base: i32,
    swing: [f32; 2],
    gain: [f32; 8],
    level: f32,
    /// Frames left in the piece and in the call; the piece's tap delays (frames).
    piece: i32,
    left: i32,
    d: [[usize; 8]; 2],
}

const MT_CAP: usize = 4 * 48000 + 8192;

impl MtCore {
    pub fn new() -> Box<MtCore> {
        let mut m = Box::new(MtCore {
            rate: 0.0,
            ring: [vec![0.0; MT_CAP], vec![0.0; MT_CAP]],
            valid: 0,
            size: MT_CAP,
            w: 0,
            taps: [GainSm::new(0.0); 8],
            out: GainSm::new(1.0),
            t: [500; 2],
            changing: false,
            base: 500,
            swing: [0.0; 2],
            gain: [0.0; 8],
            level: 1.0,
            piece: 0,
            left: 0,
            d: [[1; 8]; 2],
        });
        m.setup(48000.0);
        m
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    fn side_ms(&self, c: usize) -> i32 {
        (self.base + (((self.base as f32 + self.base as f32) * self.swing[c]) as i32)).clamp(10, crate::fx_rev::max_line_ms(self.rate) as i32)
    }

    /// Made at `rate` (the rings cleared, the taps at 0, the output at 1).
    pub fn setup(&mut self, rate: f32) {
        self.rate = rate;
        self.size = ((4.0 * rate) as usize + 4096).min(MT_CAP);
        self.valid = 0;
        self.w = 0;
        self.taps = [GainSm::new(0.0); 8];
        self.out = GainSm::new(1.0);
        self.changing = false;
        self.piece = 0;
        self.left = 0;
    }

    /// Made selected with these parameters: the times taken at once, the taps where asked.
    pub fn reset(&mut self, base_ms: i32, swing: [f32; 2], gain: [f32; 8], level: f32) {
        self.base = base_ms.clamp(10, 4000);
        self.swing = swing;
        self.gain = gain;
        self.level = level;
        for c in 0..2 {
            self.t[c] = self.side_ms(c);
        }
        for k in 0..8 {
            self.taps[k] = GainSm::new(gain[k]);
        }
        self.out = GainSm::new(level);
        self.changing = false;
        self.piece = 0;
    }

    /// `updateParameter` at the start of a call of `len` frames.
    pub fn call(&mut self, len: usize, base_ms: i32, swing: [f32; 2], gain: [f32; 8], level: f32) {
        self.base = base_ms.clamp(10, 4000);
        self.swing = swing;
        self.gain = gain;
        self.level = level;
        let r = self.rate;
        if !self.changing && (self.side_ms(0) != self.t[0] || self.side_ms(1) != self.t[1]) {
            self.changing = true;
            self.out.set(0.0, r, 3.3);
        }
        for k in 0..8 {
            self.taps[k].set(gain[k], r, 3.3);
        }
        if !self.changing {
            self.out.set(level, r, 3.3);
        }
        self.left = len.max(1) as i32;
        self.piece = 0;
    }

    /// `isFinishedSmoothing`, after each piece.
    fn finished_smoothing(&mut self) {
        if self.changing {
            if self.out.cur <= 0.0 {
                for c in 0..2 {
                    self.t[c] = self.side_ms(c);
                }
                self.out.set(self.level, self.rate, 3.3);
            } else if self.out.cur >= self.level {
                self.changing = false;
            }
        }
    }

    /// One frame: the taps' sum at the output gain, and the input written.
    #[inline]
    pub fn next(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.piece <= 0 {
            if self.left <= 0 {
                self.left = i32::MAX;
            }
            let k = self.rate * 0.001;
            let short = self.t[0].min(self.t[1]);
            let mut n = ((k * (short as f32 * 0.125).trunc()) as i32).min(self.left);
            if self.changing && n >= self.out.left {
                n = self.out.left;
            }
            self.piece = n.max(1);
            self.left -= self.piece;
            for c in 0..2 {
                for j in 0..8 {
                    let f = (j + 1) as f32 * 0.125;
                    self.d[c][j] = ((k * (f * self.t[c] as f32).trunc()) as usize).clamp(1, self.size - 1);
                }
            }
        }
        let mut y = [0.0f32; 2];
        let g: [f32; 8] = std::array::from_fn(|j| self.taps[j].next());
        for c in 0..2 {
            let mut s = 0.0f32;
            for j in 0..8 {
                let delay = self.d[c][j];
                if delay <= self.valid {
                    s += self.ring[c][(self.w + self.size - delay) % self.size] * g[j];
                }
            }
            y[c] = s;
        }
        let o = self.out.next();
        self.ring[0][self.w] = l;
        self.ring[1][self.w] = r;
        self.valid = (self.valid + 1).min(self.size);
        self.w = (self.w + 1) % self.size;
        self.piece -= 1;
        if self.piece == 0 {
            self.finished_smoothing();
        }
        (y[0] * o, y[1] * o)
    }
}

/// `Filter::make_hpf2_butter` 0x102aad58c / `make_lpf2_butter` 0x102aad298 as
/// `ThreeBandSeparate` makes them (no pre-warp: the bilinear transform of the analogue
/// Butterworth at 2 pi fc), in doubles: b0, b1, b2, a1, a2.
pub fn rb_butter(fc: f64, rate: f64, hp: bool) -> [f64; 5] {
    let t = 1.0 / rate;
    let w = fc * 6.28318531;
    let v8 = w * w * (t * t);
    let v9 = w * 2.82842712 * t;
    let d = 1.0 / (v8 + 4.0 + v9);
    let a1 = (v8 + v8 - 8.0) * d;
    let a2 = d * (v8 + 4.0 - v9);
    if hp { [4.0 * d, -8.0 * d, 4.0 * d, a1, a2] } else { [d * v8, (d + d) * v8, d * v8, a1, a2] }
}

/// One `Filter::filter2` section (transposed direct form II in doubles; the output is a float).
#[derive(Clone, Copy, Default)]
struct Sec {
    z: [f64; 2],
}

impl Sec {
    #[inline]
    fn run(&mut self, c: &[f64; 5], x: f32) -> f32 {
        let x = x as f64;
        let y = self.z[0] + c[0] * x;
        self.z[0] = self.z[1] + c[1] * x - y * c[3];
        self.z[1] = -(y * c[4] - c[2] * x);
        y as f32
    }
}

/// rekordbox's `GainSmoother` (0x1028e0400 / `setTargetGain` 0x1028e065c): a straight line to the
/// target over round(ms x fs / 1000) frames.
#[derive(Clone, Copy)]
pub struct GainSm {
    pub cur: f32,
    target: f32,
    step: f32,
    pub left: i32,
}

impl GainSm {
    pub fn new(v: f32) -> GainSm {
        GainSm { cur: v, target: v, step: 0.0, left: 0 }
    }

    pub fn set(&mut self, t: f32, rate: f32, ms: f32) {
        if t != self.target {
            let n = ((rate as f64 * (0.001 * ms as f64)).round() as i32).max(1);
            self.left = n;
            self.step = (t - self.cur) / n as f32;
            self.target = t;
        }
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.cur = if self.left == 0 { self.target } else { self.cur + self.step };
        }
        self.cur
    }
}

/// `EffectFrequencyModule` (0x1029851c4): `ThreeBandSeparate::doSeparate` 0x1028d0604 splits each
/// side into band 0 (highs: two high-passes at 4649 Hz), band 1 (two high-passes at 284 Hz,
/// then two low-passes at 4649 Hz) and band 2 (lows: two low-passes at 284 Hz); `updateBuffer`
/// gives the effect the bands at their gains and the "thru" sum at the other gains, each a
/// `GainSmoother` of 3.3 ms (made at 1 for the effect, 0 for thru).
#[derive(Clone, Copy)]
pub struct Bands3 {
    rate: f32,
    c: [[f64; 5]; 4],
    s: [[Sec; 8]; 2],
    fx: [GainSm; 3],
    thru: [GainSm; 3],
}

impl Bands3 {
    pub fn new() -> Bands3 {
        Bands3 { rate: 0.0, c: [[0.0; 5]; 4], s: [[Sec::default(); 8]; 2], fx: [GainSm::new(1.0); 3], thru: [GainSm::new(0.0); 3] }
    }

    /// Made at `rate` (the constructor): sections cleared, gains at 1 and 0.
    pub fn setup(&mut self, rate: f32) {
        let r = rate as f64;
        *self = Bands3::new();
        self.rate = rate;
        self.c = [rb_butter(4649.0, r, true), rb_butter(284.0, r, true), rb_butter(4649.0, r, false), rb_butter(284.0, r, false)];
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// `setTargetGain(band, fx, 1 - fx)` for the three bands (`setThru(band, 1)` is fx 0).
    pub fn set_gains(&mut self, g: [f32; 3]) {
        for b in 0..3 {
            let v = g[b].clamp(0.0, 1.0);
            self.fx[b].set(v, self.rate, 3.3);
            self.thru[b].set(1.0 - v, self.rate, 3.3);
        }
    }

    /// One frame: (what the effect is given, the thru sum), per side.
    #[inline]
    pub fn next(&mut self, x: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let mut bands = [[0.0f32; 3]; 2];
        for c in 0..2 {
            let (k, s) = (&self.c, &mut self.s[c]);
            let h = s[0].run(&k[0], x[c]);
            bands[c][0] = s[1].run(&k[0], h);
            let m = s[2].run(&k[1], x[c]);
            let m = s[3].run(&k[1], m);
            let m = s[4].run(&k[2], m);
            bands[c][1] = s[5].run(&k[2], m);
            let l = s[6].run(&k[3], x[c]);
            bands[c][2] = s[7].run(&k[3], l);
        }
        let (mut fx, mut th) = ([0.0f32; 2], [0.0f32; 2]);
        for b in 0..3 {
            let (gf, gt) = (self.fx[b].next(), self.thru[b].next());
            for c in 0..2 {
                fx[c] += bands[c][b] * gf;
                th[c] += bands[c][b] * gt;
            }
        }
        (fx, th)
    }

    /// The LOW CUT / HI CUT buttons as every Beat FX with them sets the module
    /// (`setThru`: LOW CUT takes band 2 out of what the effect gets, HI CUT band 0; a band taken
    /// out goes to the "thru" sum instead). For another effect: keep a `Bands3`, `setup(rate)`
    /// once made (or at a new rate), `set_cuts` every buffer, and per frame
    /// `let (fx, thru) = b.next([l, r])`: run the effect on `fx` and add `thru` to its output
    /// (the wet side), as `crate::fx::Slot` does for single DELAY / REV DELAY / MT DELAY.
    pub fn set_cuts(&mut self, low_cut: bool, hi_cut: bool) {
        self.set_gains([if hi_cut { 0.0 } else { 1.0 }, 1.0, if low_cut { 0.0 } else { 1.0 }]);
    }
}

/// Single DELAY / REV DELAY's first knob (`SingleBeatFxDelayBase`): the delay's output goes
/// through `BfxPan` (a `BeatFxPan` made on two `SquareWave`s with its LFOs smoothed over 30 ms,
/// sides starting at turn positions 0 and 0.5) on the share of `SmoothFloat` 1272, and passes
/// unpanned on the share of `SmoothFloat` 1320 (3.3333 ms each; the centre zone of the knob is
/// all unpanned). The pan restarts when its period or side changes (`updateAndSetParameter`
/// 0x1029ccc24) and when the delay is switched on (`onCalledTurnOn` 0x1029cd240); the beat
/// quantizer does not restart it (`playHead` 0x1029cd28c reaches only the delay).
#[derive(Clone, Copy, Debug)]
pub struct DelayPan {
    pub trans: crate::fx_lfo::TransFx,
    rate: f32,
    /// `BeatFxPan` +600 restart asked, +601 flip, +602 a new beat time, +604 its period (ms).
    restart: bool,
    flip: bool,
    set_bt: bool,
    bt_ms: f32,
    /// `BfxPan` +668 (the period it was given, made at 10 ms), +680 the side's level, +688 the
    /// side it starts on (left half of the knob: the left).
    period: f32,
    amp: f32,
    flag: bool,
    pan: Sf,
    thru: Sf,
}

impl DelayPan {
    pub fn new(rate: f32) -> DelayPan {
        let mut d = DelayPan { trans: crate::fx_lfo::TransFx::new(rate, crate::fx_lfo::Wave::SQUARE), rate, restart: false, flip: false, set_bt: false, bt_ms: 100.0, period: 10.0, amp: 0.0, flag: true, pan: Sf::new(rate), thru: Sf::new(rate) };
        d.setup(rate);
        d
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Made at `rate` (the constructors' state).
    pub fn setup(&mut self, rate: f32) {
        let mut t = crate::fx_lfo::TransFx::new(rate, crate::fx_lfo::Wave::SQUARE);
        for l in t.lfo.iter_mut() {
            l.set_smoothing_time(30.0);
        }
        t.par = crate::fx_lfo::TransPar { dry: 0.0, wet: 1.0, duty: [0.5; 2], min5: false, amp: [0.0; 2], head: [0.0; 2], free: [false; 2], free_ms: [0.0; 2] };
        self.trans = t;
        self.rate = rate;
        self.restart = false;
        self.flip = false;
        self.set_bt = false;
        self.bt_ms = 100.0;
        self.period = 10.0;
        self.amp = 0.0;
        self.flag = true;
        for s in [&mut self.pan, &mut self.thru] {
            *s = Sf::new(rate);
            s.set_time(3.3333);
        }
    }

    /// The pan part of `updateAndSetParameter` for the knob `k` at `bpm` (`60000 / BPM` times the
    /// zone's step; the left half starts on the left).
    pub fn set(&mut self, k: f32, bpm: f32) {
        let beat = 60000.0 / bpm;
        let (step, left) = if k < 0.090909 {
            (0.0625, true)
        } else if k < 0.18182 {
            (0.125, true)
        } else if k < 0.27273 {
            (0.25, true)
        } else if k < 0.36364 {
            (0.5, true)
        } else if k < 0.45455 {
            (1.0, true)
        } else if k < 0.54545 {
            self.pan.set_target(0.0);
            self.thru.set_target(1.0);
            return;
        } else if k < 0.63636 {
            (1.0, false)
        } else if k < 0.72727 {
            (0.5, false)
        } else if k < 0.81818 {
            (0.25, false)
        } else if k < 0.90909 {
            (0.125, false)
        } else {
            (0.0625, false)
        };
        let ms = if step == 1.0 { beat } else { beat * step };
        self.pan.set_target(1.0);
        self.thru.set_target(0.0);
        if (ms - self.period).abs() >= EPS || left != self.flag {
            let c = crate::fx::through_value(k);
            self.amp = 1.0 - (c - 1.4125 * c);
            self.flag = left;
            // `BfxPan::setEffectBeatTimeWithPlayHead(ms)`, then `turnOn`.
            self.period = ms;
            if !self.restart {
                self.restart = true;
                self.flip = !self.flip;
            }
            self.set_bt = true;
            self.bt_ms = ms + ms;
            self.turn_on();
        }
    }

    /// `BeatFxPan::onCalledTurnOn` 0x102a4e9e4 (the delay's turn-on passes it on).
    pub fn turn_on(&mut self) {
        self.restart = true;
        self.flip = true;
    }

    /// `onCalledMakeSelected` 0x1029cd1d8: the shares where they are going, the LFOs reset.
    pub fn make_selected(&mut self) {
        let t = self.pan.target;
        self.pan.reset(t);
        let t = self.thru.target;
        self.thru.reset(t);
        self.trans.par.amp = [self.amp; 2];
        self.trans.par.head = [0.5, 0.0];
        self.trans.make_selected();
    }

    /// `BeatFxPan::processEffectConcrete` 0x102a4e768: each side's gain for the next frames
    /// (heads 0 and 0.5, swapped by the side; a new period or a restart from the heads).
    pub fn gains(&mut self, gl: &mut [f32], gr: &mut [f32]) {
        use crate::fx_lfo::BeatSync;
        self.trans.par.amp = [self.amp; 2];
        if self.restart {
            let (base, other) = (0.0f32, 0.5f32);
            let one = self.flip != self.flag;
            self.trans.par.head = if one { [other, base] } else { [base, other] };
            if self.set_bt {
                self.trans.set_beat_time(self.bt_ms, 0.0);
            } else {
                self.trans.play_head();
            }
            self.restart = false;
            self.set_bt = false;
        }
        self.trans.gains(gl, gr);
    }

    /// One frame of the delay's output with this frame's pan gains.
    #[inline]
    pub fn next(&mut self, l: f32, r: f32, gl: f32, gr: f32) -> (f32, f32) {
        let t = self.thru.take();
        let p = self.pan.take();
        ((l * p) * gl + l * t, (r * p) * gr + r * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EchoCore`: whole frames of the delay; the read gate opens over 3.3333 ms once the line
    /// holds the delay; the feedback rises from 0 over 3.3333 ms (160 frames at 48 kHz).
    #[test]
    fn echo_core_repeats_at_whole_frames_with_rekordbox_s_smoothing() {
        let rate = 48000.0;
        let mut e = EchoCore::new();
        e.setup(rate);
        // 10.01 ms = 480.48 frames: rekordbox's line runs at 480.
        e.reset([10.01, 10.01], [0.5, 0.5]);
        let n = 4000;
        let mut out = vec![0.0f32; n];
        e.call(n);
        for i in 0..n {
            // (A block of 1.0, then silence.)
            let x = if i < 480 { 1.0 } else { 0.0 };
            out[i] = e.next(x, 0.0).0;
        }
        assert!(out[..481].iter().all(|v| v.abs() < 1e-9), "nothing before 480 frames (and the gate's first step is 0)");
        assert!(out[481] > 0.0 && out[481] < 0.02, "the gate opens from frame 480: {}", out[481]);
        assert!((out[700] - 1.0).abs() < 1e-5, "first repeat whole once the gate is open: {}", out[700]);
        assert!((out[1180] - 0.5).abs() < 1e-5, "second repeat at the feedback: {}", out[1180]);
        // Feedback made at 0: a core made and reset at once is already at its target.
        let mut f = EchoCore::new();
        f.setup(rate);
        f.set([10.0, 10.0], [0.5, 0.5]);
        assert!((f.fb[0].target - 0.5).abs() < 1e-6 && f.fb[0].cur.abs() < 1e-6, "feedback glides from 0");
        let steps = (0..400).take_while(|_| { f.fb[0].take(); f.fb[0].on }).count();
        assert!((150..=170).contains(&steps), "feedback glide {steps} frames (3.3333 ms = 160)");
    }

    /// `ThreeBandSeparate`'s sections are not pre-warped: the high-pass "4649 Hz" is -3 dB at
    /// 4513 Hz at 48 kHz (2 atan(pi x 4649 / 48000) / 2 pi x 48000); the three bands at gain 1
    /// sum to an all-pass (flat level).
    #[test]
    fn three_band_split_is_rekordbox_s_unwarped_butterworth() {
        let rate = 48000.0f64;
        let mag = |c: &[f64; 5], f: f64| {
            let w = std::f64::consts::TAU * f / rate;
            let z1 = (w.cos(), -w.sin());
            let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
            let num = (c[0] + c[1] * z1.0 + c[2] * z2.0, c[1] * z1.1 + c[2] * z2.1);
            let den = (1.0 + c[3] * z1.0 + c[4] * z2.0, c[3] * z1.1 + c[4] * z2.1);
            ((num.0 * num.0 + num.1 * num.1) / (den.0 * den.0 + den.1 * den.1)).sqrt()
        };
        let hp = rb_butter(4649.0, rate, true);
        let f3 = 2.0 * (std::f64::consts::PI * 4649.0 / 48000.0).atan() / std::f64::consts::TAU * 48000.0;
        assert!((f3 - 4513.0).abs() < 1.0, "{f3}");
        assert!((mag(&hp, f3) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        let mut b = Bands3::new();
        b.setup(48000.0);
        let (mut e_in, mut e_out) = (0.0f64, 0.0f64);
        for i in 0..48000 {
            let x = (i as f32 * std::f32::consts::TAU * 1000.0 / 48000.0).sin();
            let (fx, _) = b.next([x, x]);
            if i > 4800 {
                e_in += (x * x) as f64;
                e_out += (fx[0] * fx[0]) as f64;
            }
        }
        assert!((e_out / e_in - 1.0).abs() < 0.01, "bands sum flat at 1 kHz: {}", e_out / e_in);
        // LOW CUT: band 2 out, its gain falling over 3.3 ms (158 frames at 48 kHz).
        b.set_gains([1.0, 1.0, 0.0]);
        let n = (0..400).take_while(|_| { b.next([0.0, 0.0]); b.fx[2].cur > 0.0 }).count();
        assert_eq!(n + 1, 158);
    }

    /// `PitchEchoCore`: the repeat comes a whole-frame delay later through the shifter; the
    /// pitch is a whole percent (`setPitchParameter((int)p)`: 3.9 % plays as 3 %).
    #[test]
    fn pitch_echo_core_takes_a_whole_percent() {
        let run = |pct: f32| {
            let mut e = PitchEchoCore::new();
            e.setup(48000.0);
            e.reset(100.0, 0.85, pct);
            let mut out = vec![0.0f32; 24000];
            for (i, o) in out.iter_mut().enumerate() {
                if i % 960 == 0 {
                    e.set(100.0, 0.85, pct);
                    e.call(960);
                }
                let x = if i < 4800 { (i as f32 * 0.05).sin() } else { 0.0 };
                *o = e.next(x, x).0;
            }
            out
        };
        let (a, b, c, z) = (run(3.0), run(3.9), run(4.0), run(0.0));
        assert!(z[..4801].iter().all(|v| v.abs() < 1e-9), "nothing before 4800 frames (100 ms) and the gate's first step");
        let first = z.iter().position(|v| v.abs() > 1e-3).unwrap();
        // (The shifter's own start: it reads 30 ms behind until its first splice.)
        assert!((4801..12000).contains(&first), "the repeat after 100 ms: {first}");
        assert_eq!(a, b, "3.9 % plays as 3 %");
        assert!(a.iter().zip(&c).any(|(x, y)| (x - y).abs() > 1e-4), "4 % differs");
    }

    /// `MultiTapDelayCore`: taps at whole ms of k/8 of the time; a new time fades the output
    /// to 0 over 3.3 ms (158 frames at 48 kHz), takes the new taps and fades back in.
    #[test]
    fn mt_delay_core_fades_through_a_time_change() {
        let mut m = MtCore::new();
        m.setup(48000.0);
        let g = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        m.reset(500, [0.0, 0.0], g, 1.0);
        let mut out = Vec::new();
        for b in 0..60 {
            m.call(960, if b < 40 { 500 } else { 400 }, [0.0, 0.0], g, 1.0);
            for i in 0..960 {
                let t = b * 960 + i;
                let x = if t == 100 { 1.0 } else if t >= 20000 { 1.0 } else { 0.0 };
                out.push(m.next(x, x).0);
            }
        }
        // The click at 100 comes back at 2/8, 4/8, 6/8 of 500 ms: 6000, 12000, 18000 frames on.
        let hits: Vec<usize> = (0..20000).filter(|&t| out[t].abs() > 0.5).collect();
        assert_eq!(hits, vec![6100, 12100, 18100]);
        // A steady 1.0 from 20000: three taps, 3.0 once all are in.
        assert!((out[38390] - 3.0).abs() < 1e-5, "{}", out[38390]);
        // The new time at 38400: down to 0, then back to 3.0, never more.
        let after = &out[38400..40000];
        let low = after.iter().cloned().fold(f32::MAX, f32::min);
        assert!(low.abs() < 1e-6, "faded to 0: {low}");
        let zero_at = after.iter().position(|v| v.abs() < 1e-6).unwrap();
        assert_eq!(zero_at, 157, "158-frame fade (the last step lands on 0)");
        assert!((after[1000] - 3.0).abs() < 1e-5 && after.iter().all(|&v| v <= 3.0 + 1e-5));
    }

    #[test]
    fn unchanged_clamped_swing_keeps_multitap_output_open() {
        let mut m = MtCore::new();
        let swing = [-0.5, 0.5];
        m.reset(4000, swing, [1.0; 8], 1.0);
        assert_eq!(m.t, [10, 4000]);
        for _ in 0..4 {
            m.call(512, 4000, swing, [1.0; 8], 1.0);
            for _ in 0..512 {
                m.next(1.0, 1.0);
            }
            assert!(!m.changing);
            assert_eq!(m.out.cur, 1.0);
        }
        m.setup(48000.0);
        assert_eq!(m.valid, 0);
        assert_eq!(m.ring[0][1000], 1.0);
    }

    /// DELAY's first knob at 0.2 (the 1/4-beat zone, left half): at 120 BPM the pan's turn is
    /// 2 x 125 ms; the left side starts on at 1 + 0.4125 x 0.5887, the right off, and they swap
    /// half a turn later (`SingleBeatFxDelayBase::updateAndSetParameter`, `BeatFxPan`). In the
    /// centre zone the delay passes unpanned.
    #[test]
    fn delay_pan_swaps_sides_on_square_waves() {
        let mut d = DelayPan::new(48000.0);
        d.set(0.2, 120.0);
        d.make_selected();
        d.turn_on();
        let amp = 1.0 + 0.4125 * crate::fx::through_value(0.2);
        let mut out = vec![(0.0f32, 0.0f32); 24000];
        for c in 0..(24000 / 480) {
            d.set(0.2, 120.0);
            let (mut gl, mut gr) = ([0.0f32; 480], [0.0f32; 480]);
            d.gains(&mut gl, &mut gr);
            for k in 0..480 {
                out[c * 480 + k] = d.next(1.0, 1.0, gl[k], gr[k]);
            }
        }
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(near(out[3000].0, amp) && near(out[3000].1, 0.0), "{:?}", out[3000]);
        assert!(near(out[9000].0, 0.0) && near(out[9000].1, amp), "{:?}", out[9000]);
        assert!(near(out[15000].0, amp) && near(out[15000].1, 0.0), "{:?}", out[15000]);
        // (The edges are smoothed over 30 ms ending on the jump.)
        assert!(out[5800].0 < amp && out[5800].0 > 0.0, "{:?}", out[5800]);
        let mut d = DelayPan::new(48000.0);
        d.set(0.5, 120.0);
        d.make_selected();
        let (mut gl, mut gr) = ([0.0f32; 64], [0.0f32; 64]);
        d.gains(&mut gl, &mut gr);
        assert_eq!(d.next(0.3, -0.2, gl[10], gr[10]), (0.3, -0.2));
    }
}
