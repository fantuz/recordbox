//! rekordbox's reverse echoes, ported from the 7.2.19 arm64 binary: `ReverseDelayModule`
//! (0x1029914bc .. 0x1029920dc), the mono `DelayModule` (0x10298f6c0 .. 0x1029901e4) on the
//! `RingBuffer` it reads and writes (0x102987fe4 .. 0x102989808), `ReverseEchoCore`
//! (0x1029baf6c .. 0x1029bbfd4: REV DELAY, and the REV button of ECHO, LOW CUT ECHO and
//! PING PONG) and `PitchReverseEchoCore` (0x1029b9190 .. 0x1029b9d8c: PITCH ECHO's REV).
//!
//! A reverse line reads backwards from the newest frame while the input is written forwards,
//! so it plays the sound before the read point reversed. Its read gate (`SignalGate`,
//! 3.3333 ms) opens once a delay time has been written; a restart (`playHead`, from the beat
//! quantizer, or by itself every delay time when the core "runs itself") or a new delay time
//! closes the gate, jumps back to the newest frame once it is shut and opens it again; it also
//! jumps when reading on would meet the writing (`2 x read > ring - block`).
//!
//! rekordbox runs these a block at a time. A block's output only depends on what was written
//! before it (the reverse line reads behind the write point, the echo line at least a delay
//! back), so here each block is worked out when its first frame comes and the input is
//! written a frame at a time: the same numbers, driven by `crate::fx::Slot`'s frame loop.
//! Nothing here allocates after `RevFx::new`.

use crate::fx_lfo::Sf;

const EPS: f32 = 1.1920929e-7;
/// The highest rate the lines are made for (as `crate::fx`).
const MAX_RATE: usize = 48000;
/// `ReverseEchoCore` / `PitchReverseEchoCore`'s longest delay (`AbstractEchoCore` 4000 ms).
pub const MAX_MS: f32 = 4000.0;
/// `ReverseDelayModule`'s ring (mono frames): `RingBuffer(4 x frames(max) + 10000)` keeps half.
const REV_CAP: usize = 2 * 4 * MAX_RATE + 5000;
/// `DelayModule`'s ring: `RingBuffer(2 x frames(max) + 5000)`, half of it.
const DM_CAP: usize = 4 * MAX_RATE + 2500;
/// The longest block (the audio buffers are at most this long).
const BLOCK_CAP: usize = 8192;
/// The most restarts one call can hold (a delay is at least 6.6 ms).
const CHUNKS: usize = 48;

/// `(int)(float)((float)(ms * 0.001) * rate)`.
#[inline]
fn frames_of(ms: f32, rate: f32) -> i32 {
    ((ms * 0.001) * rate) as i32
}

/// The fixed-size delay lines hold four seconds at 48 kHz. At higher output rates
/// keep the requested delay inside the ring so a long tap cannot wrap onto a newer one.
pub(crate) fn max_line_ms(rate: f32) -> f32 {
    MAX_MS * (MAX_RATE as f32 / rate).min(1.0)
}

/// `SignalGate` states (`getState` 0x102979664) on its `SmoothFloat` (0 closed, 1 open).
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Gs {
    Closed,
    Open,
    Closing,
    Opening,
}

#[inline]
pub fn gate_state(g: &Sf) -> Gs {
    if g.on {
        if g.target.abs() < EPS { Gs::Closing } else { Gs::Opening }
    } else if g.cur.abs() < EPS {
        Gs::Closed
    } else {
        Gs::Open
    }
}

/// A `SignalGate` of 3.3333 ms at `rate` (`setSmoothingTime`), shut.
pub fn new_gate(rate: f32) -> Sf {
    let mut g = Sf::new(rate);
    g.set_time(3.3333);
    g.reset(0.0);
    g
}

/// `ReverseDelayModule` on one channel.
pub struct RevLine {
    buf: Vec<f32>,
    // Writes restart at index zero. Until the first wrap, the remaining tail reads as silence.
    valid: usize,
    size: i64,
    w: i64,
    /// The backward read subscript (whole frames: it only ever starts on one).
    rd: i64,
    rate: f32,
    /// Delay time (ms, frames), the one waiting for the gate to shut, a jump waiting for it.
    pub ms: f32,
    frames: i32,
    pend: bool,
    p_ms: f32,
    p_frames: i32,
    jump: bool,
    pub gate: Sf,
    /// A delay time has been read since the clear (`f168`), the longest one (`f169`), the
    /// frames counted for those (`f172`) and read since the last jump (`f176`).
    ready: bool,
    full: bool,
    written: i32,
    count: i32,
}

impl RevLine {
    fn new() -> RevLine {
        RevLine { buf: vec![0.0; REV_CAP], valid: 0, size: REV_CAP as i64, w: 0, rd: 0, rate: 48000.0, ms: 50.0, frames: 2400, pend: false, p_ms: 50.0, p_frames: 2400, jump: false, gate: new_gate(48000.0), ready: false, full: false, written: 0, count: 0 }
    }

    /// The constructor's state at `rate` (`ReverseDelayModule(rate, block, 4000)`), then `reset`.
    fn setup(&mut self, rate: f32) {
        self.rate = rate;
        self.size = ((2 * frames_of(MAX_MS, rate) + 5000) as i64).clamp(16, REV_CAP as i64);
        self.ms = 50.0;
        self.frames = (rate * 0.05) as i32;
        self.p_ms = self.ms;
        self.p_frames = self.frames;
        self.gate = new_gate(rate);
        self.reset();
    }

    /// `reset` 0x1029919f4 (the ring is also cleared: in recordbox one line serves several
    /// effects).
    fn reset(&mut self) {
        if self.pend {
            self.ms = self.p_ms;
            self.frames = self.p_frames;
        }
        self.valid = 0;
        self.w = 0;
        self.rd = self.size - 1;
        self.count = 0;
        self.pend = false;
        self.jump = false;
        self.gate.reset(0.0);
        self.ready = false;
        self.full = false;
        self.written = 0;
    }

    /// The gate's smoothing time as set (whole frames).
    fn gate_ms(&self) -> f32 {
        self.gate.n as f32 * (1000.0 * (1.0 / self.rate))
    }

    /// `setDelayTime(ms, smooth)` 0x102991a68: refused over 4 s or under twice the gate time.
    pub fn set_delay(&mut self, ms: f32, smooth: bool) {
        if ms <= 0.0 || (ms >= MAX_MS && (ms - MAX_MS).abs() >= EPS) {
            return;
        }
        let lo = self.gate_ms() + self.gate_ms();
        if lo >= ms {
            return;
        }
        let v = ms.max(lo).min(max_line_ms(self.rate));
        if smooth {
            let cur = if self.pend { self.p_ms } else { self.ms };
            if (v - cur).abs() >= EPS {
                self.pend = true;
                self.p_ms = v;
                self.p_frames = frames_of(v, self.rate);
                self.gate.set_target(0.0);
                self.jump = true;
            }
        } else {
            self.pend = false;
            if (v - self.ms).abs() >= EPS {
                self.ms = v;
                self.frames = frames_of(v, self.rate);
                self.p_ms = self.ms;
                self.p_frames = self.frames;
                self.to_head();
            }
        }
    }

    /// The read point back to the newest frame (`getValidSubscriptFloat(write, -1)`).
    fn to_head(&mut self) {
        self.rd = (self.w - 1).rem_euclid(self.size);
        self.count = 0;
        self.jump = false;
    }

    /// `jumpReadingPositionToHead(smooth)` 0x102992064.
    pub fn jump_to_head(&mut self, smooth: bool) {
        if smooth {
            self.gate.set_target(0.0);
            self.jump = true;
        } else {
            self.to_head();
            if self.pend {
                self.ms = self.p_ms;
                self.frames = self.p_frames;
                self.pend = false;
            }
        }
    }

    /// `getMaxAudioCallbackBlockSize` 0x102991e58.
    fn max_block(&self) -> i32 {
        if self.pend { self.frames.min(self.p_frames) } else { self.frames }
    }

    /// `read` 0x102991e78: `out.len()` frames backwards through the gate; `block` is the
    /// audio buffer's length (`processSampleNum`).
    fn read(&mut self, out: &mut [f32], block: i32) {
        let full_frames = frames_of(max_line_ms(self.rate), self.rate);
        let mut o = 0usize;
        let mut left = out.len() as i32;
        while left > 0 {
            let mut c = left;
            if gate_state(&self.gate) == Gs::Closing {
                c = c.min(self.gate.left.max(1));
            }
            for i in 0..c as usize {
                let k = (self.rd - i as i64).rem_euclid(self.size) as usize;
                let sample = if k < self.valid { self.buf[k] } else { 0.0 };
                out[o + i] = sample * self.gate.take();
            }
            self.rd = (self.rd - c as i64).rem_euclid(self.size);
            self.count += c;
            if !self.full {
                self.written += c;
                self.ready = self.written >= self.frames;
                self.full = self.written >= full_frames;
            }
            if self.size - (block as i64) < 2 * self.count as i64 && gate_state(&self.gate) != Gs::Closed {
                self.gate.set_target(0.0);
                self.jump = true;
            }
            if gate_state(&self.gate) == Gs::Closed {
                if self.pend {
                    self.pend = false;
                    self.ms = self.p_ms;
                    self.frames = self.p_frames;
                    self.to_head();
                } else if self.jump {
                    self.to_head();
                }
            }
            left -= c;
            if self.full || self.ready {
                if gate_state(&self.gate) != Gs::Closing {
                    self.gate.set_target(1.0);
                }
            } else {
                self.gate.set_target(0.0);
            }
            o += c as usize;
        }
    }

    #[inline]
    fn write(&mut self, x: f32) {
        self.buf[self.w as usize] = x;
        self.valid = (self.valid + 1).min(self.size as usize);
        self.w = (self.w + 1) % self.size;
    }
}

/// rekordbox's `DelayModule` on one channel (whole frames; the read gate shuts for a new
/// delay time, the time is taken once it is shut, and it opens once a delay has been read).
pub struct DLine {
    buf: Vec<f32>,
    // Writes restart at index zero. Until the first wrap, the remaining tail reads as silence.
    valid: usize,
    size: i64,
    w: i64,
    rd: i64,
    rate: f32,
    pub ms: f32,
    frames: i32,
    pend: bool,
    p_ms: f32,
    p_frames: i32,
    pub gate: Sf,
    ready: bool,
    full: bool,
    written: i32,
}

impl DLine {
    pub(crate) fn new() -> DLine {
        DLine { buf: vec![0.0; DM_CAP], valid: 0, size: DM_CAP as i64, w: 0, rd: 0, rate: 48000.0, ms: 50.0, frames: 2400, pend: false, p_ms: 50.0, p_frames: 2400, gate: new_gate(48000.0), ready: false, full: false, written: 0 }
    }

    pub(crate) fn setup(&mut self, rate: f32) {
        self.rate = rate;
        self.size = ((frames_of(MAX_MS, rate) + 2500) as i64).clamp(16, DM_CAP as i64);
        self.ms = 50.0;
        self.frames = (rate * 0.05) as i32;
        self.p_ms = self.ms;
        self.p_frames = self.frames;
        self.gate = new_gate(rate);
        self.reset();
    }

    /// `reset` 0x10298fbcc.
    pub(crate) fn reset(&mut self) {
        if self.pend {
            self.ms = self.p_ms;
            self.frames = self.p_frames;
            self.pend = false;
        }
        self.valid = 0;
        self.w = 0;
        self.set_read_delay(self.frames as f32);
        self.gate.reset(0.0);
        self.ready = false;
        self.full = false;
        self.written = 0;
    }

    /// `setReadSubscriptByDelaySampleNumber` 0x1029882f4.
    fn set_read_delay(&mut self, d: f32) {
        let mut r = self.w as f64 - d as f64;
        if r < 0.0 {
            r += self.size as f64;
        }
        self.rd = (r as i64).rem_euclid(self.size);
    }

    /// `setDelayTime(ms, smooth)` 0x10298fc28.
    pub fn set_delay(&mut self, ms: f32, smooth: bool) {
        if !(ms > 0.0 && (ms < MAX_MS || (ms - MAX_MS).abs() < EPS)) {
            return;
        }
        let v = ms.max(0.0).min(max_line_ms(self.rate));
        if smooth {
            let cur = if self.pend { self.p_ms } else { self.ms };
            if (v - cur).abs() >= EPS {
                self.pend = true;
                self.p_ms = v;
                self.p_frames = frames_of(v, self.rate);
                self.gate.set_target(0.0);
            }
        } else {
            self.pend = false;
            if (v - self.ms).abs() >= EPS {
                self.ms = v;
                self.frames = frames_of(v, self.rate);
                self.p_ms = self.ms;
                self.p_frames = self.frames;
                self.set_read_delay((v * 0.001 * self.rate).trunc());
            }
        }
    }

    /// `getMaxAudioCallbackBlockSize` 0x10298ff24.
    pub(crate) fn max_block(&self) -> i32 {
        if self.pend { self.frames.min(self.p_frames) } else { self.frames }
    }

    /// `read` 0x10298ff44: `out.len()` frames through the gate.
    pub(crate) fn read(&mut self, out: &mut [f32]) {
        let a3 = out.len() as i32;
        let full_frames = frames_of(max_line_ms(self.rate), self.rate);
        let mut left = a3;
        let mut o = 0usize;
        while left > 0 {
            let mut lim = self.frames;
            if gate_state(&self.gate) == Gs::Closing && lim >= self.gate.left {
                lim = self.gate.left;
            }
            let c = lim.min(left).max(1);
            for i in 0..c as usize {
                let k = (self.rd + i as i64).rem_euclid(self.size) as usize;
                let sample = if k < self.valid { self.buf[k] } else { 0.0 };
                out[o + i] = sample * self.gate.take();
            }
            self.rd = (self.rd + c as i64).rem_euclid(self.size);
            if !self.full {
                // (rekordbox counts the whole call's length here, once a piece.)
                self.written += a3;
                self.ready = self.written >= self.frames;
                if self.written >= full_frames {
                    self.full = true;
                }
            }
            left -= c;
            if self.pend && gate_state(&self.gate) == Gs::Closed {
                self.ms = self.p_ms;
                self.frames = self.p_frames;
                self.set_read_delay((left - a3 + self.p_frames) as f32);
                self.pend = false;
            }
            if self.full || self.ready {
                if gate_state(&self.gate) != Gs::Closing {
                    self.gate.set_target(1.0);
                }
            } else {
                self.gate.set_target(0.0);
            }
            o += c as usize;
        }
    }

    pub(crate) fn write(&mut self, xs: &[f32]) {
        self.valid = (self.valid + xs.len()).min(self.size as usize);
        for &x in xs {
            self.buf[self.w as usize] = x;
            self.w = (self.w + 1) % self.size;
        }
    }
}

/// One stretch of a call between restarts: start, length, restart at its start.
#[derive(Clone, Copy, Default)]
struct Chunk {
    at: u32,
    len: u32,
    jump: bool,
}

/// Which core the lines are run as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RevMode {
    /// `ReverseEchoCore` without its echo lines (REV DELAY: the reverse sound alone).
    Plain,
    /// `ReverseEchoCore` with them (ECHO, LOW CUT ECHO, PING PONG): reverse + echo, the sum
    /// fed back into the echo line at the feedback.
    Echo,
    /// `PitchReverseEchoCore` (PITCH ECHO): one time for both sides, the sum through the
    /// pitch shifter before it is fed back.
    Pitch,
}

/// The lines and state of a reverse echo core for an effect slot (about 4.7 MB, made by the
/// interface thread for the slots that can run one).
pub struct RevFx {
    pub mode: RevMode,
    rate: f32,
    pub rev: [RevLine; 2],
    pub echo: [DLine; 2],
    /// The feedback smoothers (`SmoothFloat`, 3.3333 ms; PITCH: one for both sides).
    fb: [Sf; 2],
    ps: crate::fx_core::PitchShifter,
    /// Restart asked for (`f952` / `f953`; PITCH `f528`), runs itself (`f956` / `f964`;
    /// `f532`) and frames into its own turn (`f960` / `f968`; `f536`).
    play: [bool; 2],
    run_self: [bool; 2],
    count: [i32; 2],
    /// The call being played: its stretches per side, the next one, the frames left of the
    /// current one.
    plan: [[Chunk; CHUNKS]; 2],
    nplan: [usize; 2],
    next_chunk: [usize; 2],
    chunk_left: [u32; 2],
    block: i32,
    /// The block worked out (per side), where playing is in it and its length.
    out: [Vec<f32>; 2],
    tmp: [Vec<f32>; 2],
    pos: [usize; 2],
    len: [usize; 2],
}

impl RevFx {
    pub fn new() -> Box<RevFx> {
        let mut f = Box::new(RevFx {
            mode: RevMode::Plain,
            rate: 48000.0,
            rev: [RevLine::new(), RevLine::new()],
            echo: [DLine::new(), DLine::new()],
            fb: [Sf::new(48000.0); 2],
            ps: crate::fx_core::PitchShifter::new(48000.0),
            play: [false; 2],
            run_self: [false; 2],
            count: [0; 2],
            plan: [[Chunk::default(); CHUNKS]; 2],
            nplan: [0; 2],
            next_chunk: [0; 2],
            chunk_left: [0; 2],
            block: 512,
            out: [vec![0.0; BLOCK_CAP], vec![0.0; BLOCK_CAP]],
            tmp: [vec![0.0; BLOCK_CAP], vec![0.0; BLOCK_CAP]],
            pos: [0; 2],
            len: [0; 2],
        });
        f.setup(RevMode::Plain, 48000.0);
        f
    }

    /// Made as `mode` at `rate` (the constructor), then `reset` (`onCalledMakeSelected`).
    pub fn setup(&mut self, mode: RevMode, rate: f32) {
        self.mode = mode;
        if (self.rate - rate).abs() > 0.5 {
            self.ps.prepare(rate);
        }
        self.rate = rate;
        for c in 0..2 {
            self.rev[c].setup(rate);
            self.echo[c].setup(rate);
            self.fb[c] = Sf::new(rate);
            self.fb[c].set_time(3.3333);
            self.fb[c].reset(0.0);
        }
        self.play = [false; 2];
        self.run_self = [false; 2];
        self.count = [0; 2];
        self.reset();
    }

    /// `ReverseEchoCore::reset` 0x1029bb8cc / `PitchReverseEchoCore::reset` 0x1029b9cfc.
    pub fn reset(&mut self) {
        for c in 0..2 {
            self.echo[c].reset();
            self.rev[c].jump_to_head(true);
            self.rev[c].reset();
            let t = self.fb[c].target;
            self.fb[c].reset(t);
        }
        if self.mode == RevMode::Pitch {
            self.ps.reset();
        }
        self.play = [false; 2];
        self.count = [0; 2];
        self.nplan = [0; 2];
        self.next_chunk = [0; 2];
        self.chunk_left = [0; 2];
        self.pos = [0; 2];
        self.len = [0; 2];
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// `BeatFxDelay::onCalledTurnOn` 0x1029bd6f0: `playHead(1, 1)`, both sides.
    pub fn play_head_both(&mut self) {
        self.play = [true; 2];
    }

    /// `playHead` from the effect (`BeatFxEcho::playHead` / `BeatFxDelay::playHead`: each side
    /// that does not run itself; `BeatFxPitchEcho::playHead`: unless it runs itself).
    pub fn play_head(&mut self) {
        if self.mode == RevMode::Pitch {
            if !self.run_self[0] {
                self.play[0] = true;
            }
        } else {
            self.play = [!self.run_self[0], !self.run_self[1]];
        }
    }

    /// The start of a call (`processEffect` over `len` frames of an audio buffer of `block`):
    /// `setRunningSelf`, `updateAndSetParameter` with each side's delay (ms) and feedback, and
    /// (PITCH) the shift (ratio - 1); then the call's restarts are laid out as rekordbox's
    /// order gives them (the left side's whole call first, then the right's).
    pub fn begin_call(&mut self, len: usize, block: usize, ms: [f32; 2], fb: [f32; 2], run_self: [bool; 2], shift: f32) {
        self.block = block as i32;
        let len = len.min(BLOCK_CAP * 64) as i32;
        // (A call cut short leaves nothing behind.)
        self.pos = self.len;
        self.chunk_left = [0; 2];
        self.next_chunk = [0; 2];
        match self.mode {
            RevMode::Pitch => {
                // `setRunningSelf(bool)` 0x1029b9d8c, then `updateAndSetParameter` 0x1029b9938.
                if self.run_self[0] != run_self[0] {
                    self.run_self = [run_self[0]; 2];
                    self.count[0] = 0;
                }
                if (ms[0] - self.echo[0].ms).abs() >= EPS {
                    for c in 0..2 {
                        self.echo[c].set_delay(ms[0], true);
                        self.rev[c].set_delay(ms[0], true);
                    }
                }
                self.fb[0].set_target(fb[0]);
                self.ps.set_shift(shift);
                // `processEffect` 0x1029b9a04: running itself, a restart every delay time
                // (the echo line's).
                let (mut at, mut left, mut n) = (0i32, len, 0usize);
                if self.run_self[0] {
                    while left > 0 {
                        let per = frames_of(self.echo[0].ms, self.rate);
                        let c = left.min(per - self.count[0]);
                        if c >= 1 {
                            self.push(0, &mut n, at, c);
                            at += c;
                            left -= c;
                            self.count[0] += c;
                            if self.count[0] != per {
                                continue;
                            }
                        }
                        self.count[0] = 0;
                        self.play[0] = true;
                    }
                } else if left > 0 {
                    self.push(0, &mut n, 0, left);
                }
                self.nplan[0] = n;
            }
            _ => {
                // `setRunningSelf(bool, bool)` 0x1029bbfd4.
                for c in 0..2 {
                    if self.run_self[c] != run_self[c] {
                        self.run_self[c] = run_self[c];
                        self.count[c] = 0;
                    }
                }
                // `updateAndSetParameter` 0x1029bb954: with echo lines a time equal to the
                // echo line's current one is not passed on.
                for c in 0..2 {
                    if self.mode == RevMode::Echo {
                        if (ms[c] - self.echo[c].ms).abs() >= EPS {
                            self.echo[c].set_delay(ms[c], true);
                            self.rev[c].set_delay(ms[c], true);
                        }
                    } else {
                        self.rev[c].set_delay(ms[c], true);
                    }
                    self.fb[c].set_target(fb[c]);
                }
                // `processEffect` 0x1029bba90: each side running itself restarts every one of
                // its delay times (`playHead(1, 0)` for the left, `(0, 1)` for the right).
                for c in 0..2 {
                    let (mut at, mut left, mut n) = (0i32, len, 0usize);
                    if self.run_self[c] {
                        while left > 0 {
                            let per = frames_of(self.rev[c].ms, self.rate);
                            let k = left.min(per - self.count[c]);
                            if k >= 1 {
                                self.push(c, &mut n, at, k);
                                at += k;
                                left -= k;
                                self.count[c] += k;
                                if self.count[c] != per {
                                    continue;
                                }
                            }
                            self.count[c] = 0;
                            self.play[c] = true;
                            self.play[1 - c] = false;
                        }
                    } else if left > 0 {
                        self.push(c, &mut n, 0, left);
                    }
                    self.nplan[c] = n;
                }
            }
        }
    }

    /// One `processEffectInternal` call in the plan: it takes the restart asked for so far.
    fn push(&mut self, c: usize, n: &mut usize, at: i32, len: i32) {
        let jump = std::mem::replace(&mut self.play[c], false);
        if *n < CHUNKS {
            self.plan[c][*n] = Chunk { at: at as u32, len: len as u32, jump };
            *n += 1;
        } else {
            // (More restarts than a call can hold: the last stretch runs on.)
            let p = &mut self.plan[c][CHUNKS - 1];
            p.len = (at as u32 + len as u32).saturating_sub(p.at);
            p.jump |= jump;
        }
    }

    /// The next block of side `c` (both for PITCH).
    fn next_block(&mut self, c: usize) {
        if self.chunk_left[c] == 0 {
            if self.next_chunk[c] >= self.nplan[c] {
                // (Past the planned call: a block of one frame at a time.)
                self.chunk_left[c] = 1;
            } else {
                let ch = self.plan[c][self.next_chunk[c]];
                self.next_chunk[c] += 1;
                self.chunk_left[c] = ch.len.max(1);
                if ch.jump {
                    if self.mode == RevMode::Pitch {
                        self.rev[0].jump_to_head(true);
                        self.rev[1].jump_to_head(true);
                    } else {
                        self.rev[c].jump_to_head(true);
                    }
                }
            }
        }
        let mut n = (self.chunk_left[c] as i32).min(self.rev[c].max_block());
        if self.mode != RevMode::Plain {
            n = n.min(self.echo[c].max_block());
        }
        let n = (n.max(1) as usize).min(BLOCK_CAP);
        let block = self.block;
        match self.mode {
            RevMode::Plain => {
                self.rev[c].read(&mut self.out[c][..n], block);
            }
            RevMode::Echo => {
                // `ReverseEchoCore::processEffectInternal` 0x1029bbe24.
                self.rev[c].read(&mut self.out[c][..n], block);
                self.echo[c].read(&mut self.tmp[c][..n]);
                for i in 0..n {
                    let y = self.out[c][i] + self.tmp[c][i];
                    self.out[c][i] = y;
                    self.tmp[c][i] = y * self.fb[c].take();
                }
                self.echo[c].write(&self.tmp[c][..n]);
            }
            RevMode::Pitch => {
                // `PitchReverseEchoCore::processEffectInternal` 0x1029b9b20 (the left lines'
                // block size for both).
                let (o0, o1) = self.out.split_at_mut(1);
                self.rev[0].read(&mut o0[0][..n], block);
                self.rev[1].read(&mut o1[0][..n], block);
                let (t0, t1) = self.tmp.split_at_mut(1);
                self.echo[0].read(&mut t0[0][..n]);
                self.echo[1].read(&mut t1[0][..n]);
                for i in 0..n {
                    self.out[0][i] += self.tmp[0][i];
                    self.out[1][i] += self.tmp[1][i];
                }
                // `processPitchShift` over the block (in the shifter's pieces of 512).
                let mut s = 0;
                while s < n {
                    let e = (s + crate::fx_core::PitchShifter::BLOCK).min(n);
                    let (a, b) = (&self.out[0], &self.out[1]);
                    self.ps.begin_block((s..e).map(|i| (a[i], b[i])));
                    for i in s..e {
                        let (l, r) = self.ps.process(self.out[0][i], self.out[1][i]);
                        self.out[0][i] = l;
                        self.out[1][i] = r;
                    }
                    s = e;
                }
                for i in 0..n {
                    let g = self.fb[0].take();
                    self.tmp[0][i] = self.out[0][i] * g;
                    self.tmp[1][i] = self.out[1][i] * g;
                }
                let (t0, t1) = self.tmp.split_at_mut(1);
                self.echo[0].write(&t0[0][..n]);
                self.echo[1].write(&t1[0][..n]);
                self.pos[1] = 0;
                self.len[1] = n;
            }
        }
        self.chunk_left[c] -= n as u32;
        if self.mode == RevMode::Pitch {
            self.chunk_left[1] = self.chunk_left[0];
        }
        self.pos[c] = 0;
        self.len[c] = n;
    }

    /// One frame: the core's output for it, and its input written.
    #[inline]
    pub fn next(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut y = [0.0f32; 2];
        if self.mode == RevMode::Pitch {
            if self.pos[0] >= self.len[0] {
                self.next_block(0);
            }
            y = [self.out[0][self.pos[0]], self.out[1][self.pos[0]]];
            self.pos[0] += 1;
            self.pos[1] = self.pos[0];
        } else {
            for (c, yc) in y.iter_mut().enumerate() {
                if self.pos[c] >= self.len[c] {
                    self.next_block(c);
                }
                *yc = self.out[c][self.pos[c]];
                self.pos[c] += 1;
            }
        }
        self.rev[0].write(l);
        self.rev[1].write(r);
        (y[0], y[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f32 = 48000.0;

    /// Runs `frames` of a ramp input (1, 2, 3, ...) through calls of `call` frames.
    fn run(f: &mut RevFx, frames: usize, call: usize, ms: f32, play_every: Option<usize>) -> Vec<f32> {
        let mut out = Vec::new();
        let mut t = 0usize;
        while t < frames {
            let n = call.min(frames - t);
            if let Some(p) = play_every {
                if t > 0 && t % p == 0 {
                    f.play_head();
                }
            }
            f.begin_call(n, call, [ms; 2], [0.0; 2], [false; 2], 0.0);
            for i in 0..n {
                let x = (t + i + 1) as f32;
                out.push(f.next(x, x).0);
            }
            t += n;
        }
        out
    }

    #[test]
    fn resetting_echo_lines_retires_the_old_turn_without_touching_its_storage() {
        let mut forward = DLine::new();
        let mut reverse = RevLine::new();
        forward.size = 16;
        reverse.size = 16;
        forward.write(&[3.0; 20]);
        for _ in 0..20 {
            reverse.write(3.0);
        }
        forward.reset();
        reverse.reset();
        assert_eq!((forward.valid, reverse.valid), (0, 0));
        assert_eq!((forward.buf[15], reverse.buf[15]), (3.0, 3.0));
        forward.write(&[5.0, 7.0]);
        reverse.write(5.0);
        reverse.write(7.0);
        forward.rd = 15;
        reverse.rd = 1;
        forward.gate.reset(1.0);
        reverse.gate.reset(1.0);
        let mut out = [0.0; 3];
        forward.read(&mut out);
        assert_eq!(out, [0.0, 5.0, 7.0]);
        reverse.read(&mut out, 3);
        assert_eq!(out, [7.0, 5.0, 0.0]);
        forward.reset();
        reverse.reset();
        assert_eq!((forward.valid, reverse.valid), (0, 0));
    }

    #[test]
    fn long_delays_at_high_output_rates_saturate_inside_the_allocated_ring() {
        let mut forward = DLine::new();
        let mut reverse = RevLine::new();
        for rate in [48000.0, 96000.0, 192000.0] {
            forward.setup(rate);
            reverse.setup(rate);
            forward.set_delay(4000.0, false);
            reverse.set_delay(4000.0, false);
            assert_eq!(forward.frames, 192000);
            assert_eq!(reverse.frames, 192000);
            assert!(i64::from(forward.frames) < forward.size);
            assert!(2 * i64::from(reverse.frames) < reverse.size);
        }
    }

    #[test]
    fn rev_lines_are_rekordbox_sized() {
        // `RingBuffer(4 x (int)(4 s x 48000) + 10000)` keeps half: 389000 frames; the echo
        // line `RingBuffer(2 x 192000 + 5000)`: 194500.
        let f = RevFx::new();
        assert_eq!(f.rev[0].size, 389000);
        assert_eq!(f.echo[0].size, 194500);
        let mut g = RevFx::new();
        g.setup(RevMode::Plain, 44100.0);
        assert_eq!(g.rev[0].size, 2 * 176400 + 5000);
    }

    #[test]
    fn rev_delay_plays_the_last_turn_backwards_after_each_restart() {
        // REV DELAY at 100 ms (4800 frames), a restart every 4800 frames, calls of 480: the
        // line opens once 100 ms is read; from a restart at frame t0 the gate shuts over 159
        // frames (3.3333 ms at 48 kHz), the read point goes back to the newest frame, then
        // the input before that point plays backwards, the gate opening over 159 frames.
        let mut f = RevFx::new();
        let out = run(&mut f, 48000, 480, 100.0, Some(4800));
        let gn = f.rev[0].gate.n as usize;
        assert_eq!(gn, 159);
        // The restart at 4800 x 4: the gate is shut at 19200 + 159, and the read point goes
        // to the newest frame written before the block (19199, which holds 19200); from there
        // the input plays backwards, the gate opening over 159 frames.
        let j = 19200 + gn;
        for k in [gn + 5, 1000, 3000] {
            let i = j + k;
            assert!((out[i] - (19200 - k) as f32).abs() < 1e-3, "frame {i}: {} vs {}", out[i], 19200 - k);
        }
        // Over the shut: the gate's line down to 0 at the jump.
        assert!(out[j - 1].abs() < (j as f32) * 0.01);
    }

    #[test]
    fn rev_line_jumps_before_it_reads_into_the_writing() {
        // No restarts, 50 ms, blocks of 512: the line is cleared and its read point starts at
        // the ring's end (389000 frames at 48 kHz), so it reads silence backwards while the
        // input is written forwards. The check comes after each block: once 2 x read > 389000 -
        // 512 (after 380 blocks, 194560 frames) the gate shuts over the next 159 frames, read on
        // from where it was (by now inside what was written: 194439, then down), and the read
        // point goes to the newest frame (194559).
        let mut f = RevFx::new();
        let out = run(&mut f, 200000, 512, 50.0, None);
        let blk = 512 * 380;
        assert!(out[..blk].iter().all(|v| *v == 0.0));
        assert_eq!(out[blk], 194440.0);
        let j = blk + 159;
        assert_eq!(out[j + 200], (blk - 200) as f32);
    }

    #[test]
    fn rev_echo_feeds_the_sum_back() {
        // ECHO's reverse core at 100 ms with a restart every 100 ms, feedback 0.5: a click at
        // frame 3000 comes out of the reverse line after the restart at 4800 (gate shut at
        // 4959, read from 4799 back: frame 6758), then the echo line repeats it every 4800
        // frames at 0.5, 0.25.
        let mut f = RevFx::new();
        f.setup(RevMode::Echo, R);
        let mut out = Vec::new();
        for t in (0..36 * 480).step_by(480) {
            if t > 0 && t % 4800 == 0 {
                f.play_head();
            }
            f.begin_call(480, 480, [100.0; 2], [0.5; 2], [false; 2], 0.0);
            for i in 0..480 {
                let x = if t + i == 3000 { 1.0 } else { 0.0 };
                out.push(f.next(x, x).0);
            }
        }
        let peaks: Vec<(usize, f32)> = out.iter().enumerate().filter(|(_, v)| v.abs() > 0.01).map(|(i, v)| (i, *v)).collect();
        assert_eq!(peaks.len(), 3, "{peaks:?}");
        assert_eq!(peaks[0], (6758, 1.0));
        assert_eq!(peaks[1].0, 6758 + 4800);
        assert!((peaks[1].1 - 0.5).abs() < 1e-6 && (peaks[2].1 - 0.25).abs() < 1e-6, "{peaks:?}");
    }
}
