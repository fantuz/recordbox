//! rekordbox's ROLL family cores, ported from the 7.2.19 arm64 binary: `AbstractRollCore`
//! (0x102a29578 .. 0x102a29c80), `RollCore` (startEffect 0x102a2c35c, processEffectInternal
//! 0x102a2c54c), `ReverseRollCore` (0x102a2ae68, 0x102a2b058), `SlipRollCore` (0x102a2c6cc),
//! `RollBufferWriter` (0x102a2809c .. 0x102a28104), `RollBufferReader` (0x102a265d4 ..
//! 0x102a26ccc) and the `SignalGate` they fade with (0x1029791c0 ..).
//!
//! The roll keeps its own buffer of 4 s (the effect slot's delay line, made up front): the writer
//! copies the channel's sound into it from the point the roll started (forward for ROLL and
//! SLIP ROLL, backward in time for REV ROLL) and the reader plays it from that point, fading out
//! (3.3333 ms) and back in at the start on every restart the beat quantizer gives.

use crate::fx_lfo::{BeatSync, Sf};

/// The channel's history (`crate::fx::Ring`): interleaved stereo frames, `len` frames long.
pub struct Hist<'a> {
    pub buf: &'a [f32],
    pub len: usize,
    /// Where the next frame of the block after this one will be written (rekordbox's write
    /// subscript once the block is in).
    pub end: i64,
    /// The block's frames (rekordbox's `processSampleNum`).
    pub block: i64,
}

impl Hist<'_> {
    #[inline]
    fn at(&self, i: i64) -> (f32, f32) {
        let k = i.rem_euclid(self.len as i64) as usize;
        (self.buf[k * 2], self.buf[k * 2 + 1])
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Info {
    on: bool,
    src: i64,
    dst: i32,
}

/// `RollBufferWriter`.
#[derive(Clone, Copy, Debug, Default)]
struct Writer {
    b: Info,
    b_dst0: i32,
    f: Info,
    f_dst0: i32,
    delay_on: bool,
    delay: i32,
}

impl Writer {
    fn start(&mut self, b: Info, f: Info, delay: i32) {
        if b.on {
            self.b = b;
            self.b_dst0 = b.dst;
        }
        self.b.on = b.on;
        if f.on {
            self.f = f;
            self.f_dst0 = f.dst;
        }
        self.f.on = f.on;
        self.delay_on = delay > 0;
        self.delay = if delay <= 0 { -1 } else { delay };
    }

    /// `isWrittenPosition` 0x102a282ec.
    fn is_written(&self, a: i32) -> bool {
        if self.b.on {
            if self.f_dst0 > a {
                return self.b.dst < a;
            }
            if self.f.on {
                return self.f.dst > a;
            }
            return true;
        }
        if !self.f.on || self.f_dst0 > a {
            return true;
        }
        self.f.dst > a
    }

    /// `write` 0x102a28104: `n` more frames of the channel into the roll buffer.
    fn write(&mut self, n: i32, h: &Hist, rb: &mut [f32], len: i32) {
        if n < 1 || !(self.b.on || self.f.on) {
            return;
        }
        let mut v3 = n;
        if self.delay_on {
            if self.delay > n {
                self.delay -= n;
                return;
            }
            self.delay_on = false;
            v3 = n - self.delay;
            self.delay = -1;
            if v3 == 0 {
                return;
            }
        }
        if self.f.on {
            let v = v3.min(len - 1 - self.f.dst + 1);
            for i in 0..v.max(0) {
                let (l, r) = h.at(self.f.src + i as i64);
                let d = ((self.f.dst + i) as usize) * 2;
                rb[d] = l;
                rb[d + 1] = r;
            }
            self.f.src += v as i64;
            self.f.dst += v;
            if self.f.dst >= len {
                self.f.dst -= len;
            }
            if self.f.dst == 0 {
                self.f.on = false;
            }
        }
        if self.b.on {
            let v = v3.min(self.b.dst + 1);
            for i in 0..v.max(0) {
                let (l, r) = h.at(self.b.src - i as i64);
                let d = ((self.b.dst - i) as usize) * 2;
                rb[d] = l;
                rb[d + 1] = r;
            }
            self.b.src -= v as i64;
            self.b.dst -= v;
            if self.b.dst < 0 {
                self.b.dst += len;
            }
            if self.b.dst == len - 1 {
                self.b.on = false;
            }
        }
    }
}

/// `SignalGate` states (`getState` 0x102979664).
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Gs {
    Closed,
    Open,
    Closing,
    Opening,
}

/// `RollBufferReader`.
#[derive(Clone, Copy, Debug)]
struct Reader {
    gate: Sf,
    pos: i32,
    next: i32,
    start: i32,
    len: i32,
    fwd: bool,
    pend: bool,
    p_start: i32,
    p_len: i32,
    p_fwd: bool,
    counted: bool,
    count: i32,
}

impl Reader {
    fn new(rate: f32) -> Reader {
        let mut gate = Sf::new(rate);
        gate.reset(0.0);
        Reader { gate, pos: 0, next: 0, start: 0, len: 0, fwd: true, pend: false, p_start: 0, p_len: 0, p_fwd: true, counted: false, count: -1 }
    }

    fn state(&self) -> Gs {
        if self.gate.on {
            if self.gate.target.abs() < 1.1920929e-7 { Gs::Closing } else { Gs::Opening }
        } else if self.gate.cur.abs() < 1.1920929e-7 {
            Gs::Closed
        } else {
            Gs::Open
        }
    }

    fn close(&mut self) {
        self.gate.set_target(0.0);
    }

    fn reset(&mut self, start: i32, len: i32, fwd: bool) {
        self.gate.reset(1.0);
        self.pos = start;
        self.next = start;
        self.start = start;
        self.len = len;
        self.fwd = fwd;
        self.pend = false;
        (self.p_start, self.p_len, self.p_fwd) = (start, len, fwd);
        self.counted = false;
        self.count = -1;
    }

    /// `jumpReadingPositioin` 0x102a26b58.
    fn jump(&mut self, start: i32, off: i32, len: i32, fwd: bool) {
        self.start = start;
        self.len = len;
        self.fwd = fwd;
        self.pos = off + start;
        self.next = off + start;
        self.pend = false;
        (self.p_start, self.p_len, self.p_fwd) = (start, len, fwd);
        self.counted = false;
        self.count = -1;
    }

    /// `playHead` 0x102a26b8c: fade out, then go to the start (moved on by a late restart).
    fn play_head(&mut self, start: i32, len: i32, off: i32, fwd: bool) {
        let a = off.abs();
        let same = self.start == start && self.len == len && self.fwd == fwd;
        self.pend = !same;
        (self.p_start, self.p_len, self.p_fwd) = if same { (self.start, self.len, self.fwd) } else { (start, len, fwd) };
        let base = if self.pend { self.p_start } else { self.start };
        if off < 0 {
            self.close();
            self.counted = false;
            self.count = -1;
            self.next = base + if fwd { a } else { -a };
        } else if off != 0 {
            self.counted = true;
            self.count = off;
            self.next = base;
        } else {
            self.close();
            self.counted = false;
            self.count = -1;
            self.next = base;
        }
    }

    /// `read` 0x102a268b8: `out.len()` frames of the roll (interleaved), faded by the gate.
    fn read(&mut self, rb: &[f32], len: i32, l: &mut [f32], r: &mut [f32]) {
        let mut o = 0usize;
        let mut n = l.len() as i32;
        let end = len - 1;
        while n > 0 {
            let mut c = n;
            let st = self.state();
            if st == Gs::Closing {
                c = c.min(self.gate.left);
            }
            if self.counted && c >= self.count {
                c = self.count;
            }
            if st != Gs::Closing && st != Gs::Closed {
                let lim = if self.fwd { end - (self.gate.n + self.pos) } else { self.pos - self.gate.n };
                if c >= lim {
                    c = lim;
                }
            }
            let c = c.max(1).min(n);
            for i in 0..c as usize {
                let k = (self.pos + if self.fwd { i as i32 } else { -(i as i32) }).rem_euclid(len) as usize;
                let g = self.gate.take();
                l[o + i] = rb[k * 2] * g;
                r[o + i] = rb[k * 2 + 1] * g;
            }
            self.pos = (self.pos + if self.fwd { c } else { -c }).rem_euclid(len);
            if self.counted {
                self.count -= c;
            }
            let mut check_end = true;
            if self.state() == Gs::Closed {
                self.pos = self.next;
                if self.pend {
                    self.pend = false;
                    (self.start, self.len, self.fwd) = (self.p_start, self.p_len, self.p_fwd);
                }
                self.gate.set_target(1.0);
            }
            if self.counted && self.count == 0 {
                self.counted = false;
                self.count = -1;
                self.close();
            }
            n -= c;
            if self.state() == Gs::Closing {
                check_end = false;
            }
            if check_end && self.state() != Gs::Closed {
                let at_end = if self.fwd { self.pos == end - self.gate.n } else { self.pos == self.gate.n };
                if at_end {
                    self.close();
                    self.counted = false;
                    self.count = -1;
                    self.next = self.start;
                }
            }
            o += c as usize;
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RollKind {
    Roll,
    Rev,
    Slip,
}

/// One ROLL / REV ROLL / SLIP ROLL core and its `MultiBeatFxRollBase` start flag.
#[derive(Clone, Copy, Debug)]
pub struct RollFx {
    pub kind: RollKind,
    rate: f32,
    /// The roll buffer's frames (4 s, rekordbox's r range 10 .. 4000 ms with no head range).
    len: i32,
    /// The r parameter: the roll's length (ms).
    r_ms: f32,
    w: Writer,
    rd: Reader,
    play: bool,
    play_off: i32,
    slip_left: i32,
    /// `isValidRollSound`: the roll's sound is there (the mix law applies).
    pub valid: bool,
    start_pending: Option<f32>,
    /// Single mode's SWING: the core's parameter 1 (ms, -2000 .. 2000), where a turn starts
    /// in the roll's sound; `swing_ok` is `RollCore`'s 372 (the sound there is in).
    pub swing_ms: f32,
    swing_ok: bool,
    /// The channel history's place for this block.
    h_end: i64,
    h_block: i64,
}

#[inline]
fn frames_of(ms: f32, rate: f32) -> i32 {
    (((ms as f64 * 0.001) as f32) * rate) as i32
}

impl RollFx {
    pub fn new(kind: RollKind, rate: f32) -> RollFx {
        let len = (((rate as f64 * 0.001) as f32) * 4000.0) as i32;
        RollFx { kind, rate, len, r_ms: 10.0, w: Writer::default(), rd: Reader::new(rate), play: false, play_off: 0, slip_left: 0, valid: false, start_pending: None, swing_ms: 0.0, swing_ok: true, h_end: 0, h_block: 0 }
    }

    /// Frames the roll buffer needs at `rate` (interleaved stereo: twice this).
    pub fn buffer_frames(rate: f32) -> usize {
        ((((rate as f64 * 0.001) as f32) * 4000.0) as i32).max(1) as usize
    }

    /// Where the reader is in the roll's turn (0 .. 1), and how far a frame moves it.
    pub fn phase(&self) -> f32 {
        let rf = frames_of(self.r_ms, self.rate).max(1);
        let d = if self.rd.fwd { self.rd.pos - self.rd.start } else { self.rd.start - self.rd.pos };
        (d.rem_euclid(self.len.max(1)) as f32 / rf as f32).fract()
    }

    pub fn phase_step(&self) -> f32 {
        1.0 / frames_of(self.r_ms, self.rate).max(1) as f32
    }

    /// Where the channel history stands for the next stretch (see `Hist`).
    pub fn set_block(&mut self, end: i64, block: i64) {
        self.h_end = end;
        self.h_block = block;
    }

    /// `RollCore::startEffect` 0x102a2c35c / `ReverseRollCore::startEffect` 0x102a2ae68. SWING
    /// (`v21` frames) moves where the roll starts: earlier in the sound (the writer then also
    /// fills backward / forward from there) or later; the reader starts there. (rekordbox's
    /// ring ignores a write or read place outside the buffer, which a negative ROLL / positive
    /// REV ROLL swing gives, and keeps its last one; here the place wraps round the buffer.)
    fn start_effect(&mut self, off_ms: f32) {
        let off = (self.rate * (off_ms as f64 * 0.001) as f32) as i32;
        let n = self.len;
        let v21 = frames_of(self.swing_ms, self.rate);
        let w = |d: i32| d.rem_euclid(n.max(1));
        match self.kind {
            RollKind::Rev => {
                let mut src = self.h_end - self.h_block - 1 + off as i64;
                let mut dst = n - 1;
                let mut f = Info { on: false, ..Info::default() };
                if v21 < 0 {
                    src += v21 as i64;
                    dst += v21;
                    f = Info { on: true, src: src + 1, dst: w(dst + 1) };
                }
                self.w.start(Info { on: true, src, dst: w(dst) }, f, off.max(0));
                self.rd.reset(w(n - 1 + v21), frames_of(self.r_ms, self.rate), false);
            }
            _ => {
                let mut src = self.h_end - self.h_block + off as i64;
                let mut dst = 0;
                let mut b = Info { on: false, ..Info::default() };
                if v21 < 0 {
                    src += v21 as i64;
                    dst = v21;
                    b = Info { on: true, src: src - 1, dst: w(dst - 1) };
                }
                self.w.start(b, Info { on: true, src, dst: w(dst) }, off.max(0));
                self.rd.reset(w(v21), frames_of(self.r_ms, self.rate), true);
            }
        }
        self.swing_ok = self.swing_ms < 0.0 || self.swing_ms.abs() < 1.1920929e-7;
        self.valid = self.swing_ok;
        self.play = false;
    }

    /// The roll for the next frames (`AbstractRollCore::processEffect` with no self restart).
    pub fn process(&mut self, rb: &mut [f32], h: &Hist, l: &mut [f32], r: &mut [f32]) {
        if let Some(off) = self.start_pending.take() {
            self.start_effect(off);
        }
        let n = l.len() as i32;
        let len = self.len;
        match self.kind {
            RollKind::Roll | RollKind::Rev => {
                if self.play {
                    let fwd = self.kind == RollKind::Roll;
                    let rf = frames_of(self.r_ms, self.rate);
                    // (`processEffectInternal`: the turn starts at the SWING place once it is
                    // written, else where the reader started.)
                    let v21 = frames_of(self.swing_ms, self.rate);
                    let mut v7 = if fwd { v21 } else { len - 1 + v21 }.rem_euclid(len.max(1));
                    if self.w.is_written(v7) {
                        self.swing_ok = true;
                        self.valid = true;
                    } else {
                        v7 = self.rd.start;
                    }
                    self.rd.play_head(v7, rf, self.play_off, fwd);
                    self.play = false;
                }
                self.w.write(n, h, rb, len);
                self.rd.read(rb, len, l, r);
            }
            RollKind::Slip => self.slip(rb, h, l, r),
        }
    }

    /// `SlipRollCore::processEffectInternal` 0x102a2c6cc.
    fn slip(&mut self, rb: &mut [f32], h: &Hist, l: &mut [f32], r: &mut [f32]) {
        let len = self.len;
        let mut o = 0usize;
        let mut n = l.len() as i32;
        while self.play {
            if self.rd.state() == Gs::Closing {
                let fl = self.rd.gate.left;
                if n < fl {
                    break;
                }
                if fl >= 1 {
                    self.w.write(fl, h, rb, len);
                    self.rd.read(rb, len, &mut l[o..o + fl as usize], &mut r[o..o + fl as usize]);
                    o += fl as usize;
                    n -= fl;
                }
                let mut v26 = 0;
                let v27 = self.play_off;
                let rf = frames_of(self.r_ms, self.rate);
                if self.w.is_written(v26) {
                    self.valid = true;
                } else {
                    v26 = self.rd.start;
                }
                if (rf - self.rd.len).abs() > (self.rate * 0.0001) as i32 {
                    let src = self.h_end + v27.min(0) as i64 - self.h_block;
                    self.w.start(Info { on: v26 != 0, src: src - 1, dst: (v26 - 1).rem_euclid(len) }, Info { on: true, src, dst: v26 }, 0);
                    self.valid = false;
                }
                self.rd.jump(v26, if v27 < 0 { -v27 } else { 0 }, rf, true);
                self.play = false;
                self.play_off = 0;
                self.slip_left = 0;
            } else {
                let v32 = self.slip_left.max(0);
                if n < v32 {
                    self.w.write(n, h, rb, len);
                    self.rd.read(rb, len, &mut l[o..], &mut r[o..]);
                    self.slip_left -= n;
                    return;
                }
                if self.slip_left >= 1 {
                    self.w.write(v32, h, rb, len);
                    self.rd.read(rb, len, &mut l[o..o + v32 as usize], &mut r[o..o + v32 as usize]);
                    o += v32 as usize;
                    n -= v32;
                }
                self.rd.close();
                self.slip_left = 0;
            }
            if n <= 0 {
                return;
            }
        }
        if n > 0 {
            self.w.write(n, h, rb, len);
            self.rd.read(rb, len, &mut l[o..], &mut r[o..]);
        }
    }
}

impl BeatSync for RollFx {
    /// `AbstractRollCore::playHead` 0x102a29c50.
    fn play_head(&mut self) {
        self.play = true;
        self.play_off = 0;
        self.slip_left = 0;
    }

    /// `MultiBeatFxRollBase::setEffectBeatTimeWithPlayHead` 0x102a2df84: r = the beat value,
    /// then a restart `offset` ms late.
    fn set_beat_time(&mut self, ms: f32, offset: f32) {
        self.r_ms = ms.clamp(10.0, 4000.0);
        self.play = true;
        let f = ((offset as f64 * 0.001) as f32 * self.rate) as i32;
        self.play_off = f;
        self.slip_left = f;
    }

    fn time_until_next(&mut self, _ms: f32) {}

    /// `MultiBeatFxRollBase::onCalledTurnOn`: the core starts on the next stretch.
    fn turn_on(&mut self, offset: f32) {
        self.start_pending = Some(offset);
    }

    fn turn_off(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist(n: usize) -> Vec<f32> {
        (0..n).flat_map(|i| [i as f32, -(i as f32)]).collect()
    }

    #[test]
    fn roll_replays_from_its_start_and_fades_at_each_restart() {
        // 48 kHz, a 100 ms roll (4800 frames) started on the block's first frame: it plays the
        // sound from there, and at the restart fades out over 159 frames, then plays the start
        // again fading in.
        let rate = 48000.0;
        let hb = hist(48000);
        let mut rb = vec![0.0f32; RollFx::buffer_frames(rate) * 2];
        let mut f = RollFx::new(RollKind::Roll, rate);
        f.set_beat_time(100.0, 0.0);
        f.turn_on(0.0);
        let (mut l, mut r) = (vec![0.0f32; 9600], vec![0.0f32; 9600]);
        let mut at = 10000i64;
        for k in 0..(9600 / 480) {
            if k == 10 {
                f.set_beat_time(100.0, 0.0);
            }
            f.set_block(at + 480, 480);
            let h = Hist { buf: &hb, len: 48000, end: at + 480, block: 480 };
            f.process(&mut rb, &h, &mut l[k * 480..(k + 1) * 480], &mut r[k * 480..(k + 1) * 480]);
            at += 480;
        }
        assert_eq!(l[0], 10000.0);
        assert_eq!(l[4000], 14000.0);
        // The restart at 4800: frames 4800 .. 4958 fade out on the sound going on, then from the
        // roll's start again (fading in).
        assert_eq!(l[4800], 14800.0);
        assert!((l[4900] - 14900.0 * (1.0 - 100.0 / 159.0)).abs() < 2.0, "{}", l[4900]);
        assert_eq!(l[4959], 0.0);
        assert!((l[4959 + 100] - 10100.0 * 100.0 / 159.0).abs() < 2.0, "{}", l[5059]);
        assert_eq!(l[4959 + 200], 10200.0);
        assert_eq!(r[100], -10100.0);
    }

    #[test]
    fn swing_moves_the_rolls_start() {
        // `RollCore::startEffect`: SWING -100 ms starts the roll 4800 frames earlier in the
        // sound (the backward writer fills the frames before it); +100 ms 4800 frames later,
        // and the sound is not in until a restart finds that place written.
        let rate = 48000.0;
        let h = hist(48000 * 6);
        let len = 48000 * 6;
        let mut rb = vec![0.0f32; RollFx::buffer_frames(rate) * 2];
        for (sw, first) in [(-100.0f32, 48000 * 5 - 4800), (100.0, 48000 * 5 + 4800)] {
            let mut fx = RollFx::new(RollKind::Roll, rate);
            fx.swing_ms = sw;
            fx.r_ms = 500.0;
            fx.turn_on(0.0);
            let mut l = vec![0.0f32; 512];
            let mut r = vec![0.0f32; 512];
            fx.set_block(48000 * 5 + 512, 512);
            let hh = Hist { buf: &h, len, end: 48000 * 5 + 512, block: 512 };
            fx.process(&mut rb, &hh, &mut l, &mut r);
            if sw < 0.0 {
                // The first frame played is the sound at the start minus 100 ms.
                assert!((l[0] - h[first * 2]).abs() < 1e-6 && fx.valid, "{} {}", l[0], h[first * 2]);
            } else {
                assert!(!fx.valid, "not in before the swing place is written");
                let _ = first;
            }
        }
    }

    #[test]
    fn slip_roll_restarts_on_play_head_and_records_again_on_a_new_length() {
        let rate = 48000.0;
        let hb = hist(48000);
        let mut rb = vec![0.0f32; RollFx::buffer_frames(rate) * 2];
        let mut f = RollFx::new(RollKind::Slip, rate);
        f.set_beat_time(100.0, 0.0);
        f.turn_on(0.0);
        let (mut l, mut r) = (vec![0.0f32; 14400], vec![0.0f32; 14400]);
        let mut at = 10000i64;
        for k in 0..(14400 / 480) {
            if k == 10 {
                // (The quantizer's restart for SLIP ROLL: a play head, no new time.)
                f.play_head();
            }
            if k == 20 {
                // A new beat value: the roll records again from here.
                f.set_beat_time(50.0, 0.0);
            }
            f.set_block(at + 480, 480);
            let h = Hist { buf: &hb, len: 48000, end: at + 480, block: 480 };
            f.process(&mut rb, &h, &mut l[k * 480..(k + 1) * 480], &mut r[k * 480..(k + 1) * 480]);
            at += 480;
        }
        assert_eq!(l[4000], 14000.0);
        assert_eq!(l[4959 + 300], 10300.0);
        // Re-recorded from the block where the new length came (frame 9,600 = history 19,600).
        assert_eq!(l[9600 + 159 + 300], 19600.0 + 300.0, "{}", l[9600 + 159 + 300]);
    }

    #[test]
    fn rev_roll_plays_the_sound_before_its_start_backwards() {
        let rate = 48000.0;
        let hb = hist(48000);
        let mut rb = vec![0.0f32; RollFx::buffer_frames(rate) * 2];
        let mut f = RollFx::new(RollKind::Rev, rate);
        f.set_beat_time(100.0, 0.0);
        f.turn_on(0.0);
        let (mut l, mut r) = (vec![0.0f32; 960], vec![0.0f32; 960]);
        let at = 20000i64;
        f.set_block(at + 960, 960);
        let h = Hist { buf: &hb, len: 48000, end: at + 960, block: 960 };
        f.process(&mut rb, &h, &mut l, &mut r);
        assert_eq!(l[0], 19999.0);
        assert_eq!(l[500], 19499.0);
    }
}
