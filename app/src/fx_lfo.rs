//! rekordbox's beat LFO and beat quantizer, ported from the 7.2.19 arm64 binary:
//! `SmoothFloat<float>` (0x10297aa84 ..), `SquareWave` (0x102995140 ..), `TriangleWave`
//! (0x102996638 ..), `LFO` (0x102997d90 .. 0x10299907c), `BeatFxTrans` / `TransCore`
//! (0x102a5084c .. 0x102a530d0), `BeatFxPan` (0x102a4e54c .. 0x102a4ea6c) and
//! `FxQuantizeManager` (0x102a8cb14 .. 0x102a8d868).
//!
//! Nothing here allocates: the audio thread runs it.

const EPS: f32 = 1.1920929e-7;

/// `(int)(float)((float)(ms * 0.001) * rate)`: frames in `ms` as rekordbox counts them.
#[inline]
fn frames_of(ms: f32, rate: f32) -> f32 {
    ((ms as f64 * 0.001) as f32) * rate
}

/// rekordbox's `SmoothFloat<float>`: a straight line to a new target over n frames; a frame
/// taken gives the current value and then steps (`getSmoothValues` writes `cur + i x step`).
#[derive(Clone, Copy, Debug)]
pub struct Sf {
    pub on: bool,
    pub cur: f32,
    pub target: f32,
    step: f32,
    pub left: i32,
    ms: f32,
    pub n: i32,
    inv: f32,
    rate: f32,
}

impl Sf {
    pub fn new(rate: f32) -> Sf {
        let n = ((rate as f64 * 0.0033333) as f32 as i32).max(1);
        Sf { on: false, cur: 0.0, target: 0.0, step: 0.0, left: 0, ms: 3.3333, n, inv: 1.0 / n as f32, rate }
    }

    pub fn reset(&mut self, v: f32) {
        self.on = false;
        self.cur = v;
        self.target = v;
        self.step = 0.0;
        self.left = 0;
    }

    pub fn set_target(&mut self, t: f32) {
        if (t - self.target).abs() >= EPS {
            self.target = t;
            self.step = (t - self.cur) * self.inv;
            self.on = true;
            self.left = self.n;
        }
    }

    fn restart(&mut self, n: i32) {
        let (c, t) = (self.cur, self.target);
        self.target = c;
        self.on = false;
        self.n = n;
        self.inv = 1.0 / n as f32;
        if (t - c).abs() >= EPS {
            self.target = t;
            self.step = (t - c) * self.inv;
            self.on = true;
            self.left = n;
        }
    }

    /// `setSmoothingTime` 0x10297ad38.
    pub fn set_time(&mut self, ms: f32) {
        if ms > 0.0 {
            let n = frames_of(ms, self.rate) as i32;
            if n >= 1 {
                self.ms = n as f32 * (1000.0 * (1.0 / self.rate));
                if self.on {
                    self.restart(n);
                } else {
                    self.n = n;
                    self.inv = 1.0 / n as f32;
                }
            }
        }
    }

    /// `onSamplingRateChanged` 0x10297ae08.
    pub fn set_rate(&mut self, rate: f32) {
        // (Only a change of rate re-times the line: rekordbox calls this when the rate changes.)
        if rate == self.rate {
            return;
        }
        self.rate = rate;
        let n = (frames_of(self.ms, rate) as i32).max(1);
        if self.on {
            self.restart(n);
        } else {
            self.n = n;
            self.inv = 1.0 / n as f32;
        }
    }

    #[inline]
    pub fn take(&mut self) -> f32 {
        if !self.on {
            return self.cur;
        }
        let v = self.cur;
        self.left -= 1;
        if self.left <= 0 {
            self.on = false;
            self.left = 0;
            self.cur = self.target;
        } else {
            self.cur += self.step;
        }
        v
    }
}

/// `SquareWave` / `TriangleWave`: values in -1 .. 1 over a period of whole frames.
#[derive(Clone, Copy, Debug)]
pub enum Wave {
    Square { p: i32, pt: i32, duty: f32 },
    Tri { p: i32, h: i32, step: f32 },
}

impl Wave {
    pub const SQUARE: Wave = Wave::Square { p: 44100, pt: 22050, duty: 0.5 };
    pub const TRI: Wave = Wave::Tri { p: 44100, h: 22050, step: 0.0 };

    pub fn period(&self) -> i32 {
        match *self {
            Wave::Square { p, .. } | Wave::Tri { p, .. } => p,
        }
    }

    fn set_period(&mut self, n: i32) {
        match self {
            Wave::Square { p, pt, duty } => {
                if n >= 1 && *p != n {
                    *pt = (*duty * n as f32) as i32;
                    *p = n;
                }
            }
            Wave::Tri { p, h, step } => {
                if n >= 1 {
                    *p = n;
                    *h = ((n as u32) >> 1) as i32;
                    *step = ((1.0f32 - -1.0) + (1.0f32 - -1.0)) / n as f32;
                }
            }
        }
    }

    #[inline]
    pub fn value(&self, i: i32) -> f32 {
        match *self {
            Wave::Square { p, pt, .. } => {
                if i < 0 || i >= p {
                    0.0
                } else if pt <= i {
                    -1.0
                } else {
                    1.0
                }
            }
            Wave::Tri { p, h, step } => {
                if i < 0 || p - i <= 0 {
                    0.0
                } else {
                    step * if h <= i { (p - i) as f32 } else { i as f32 } + -1.0
                }
            }
        }
    }

    /// The frames just after each jump (`getDiscontinousPointPosition` >> 32).
    fn points(&self) -> ([i32; 2], usize) {
        match *self {
            Wave::Square { pt, .. } => ([pt, 0], 2),
            Wave::Tri { .. } => ([0, 0], 0),
        }
    }

    fn duty(&self) -> f32 {
        match *self {
            Wave::Square { duty, .. } => duty,
            Wave::Tri { .. } => 0.5,
        }
    }

    fn set_duty(&mut self, d: f32) {
        if let Wave::Square { p, pt, duty } = self {
            if (d > 0.0 || d.abs() < EPS) && (d < 1.0 || (d - 1.0).abs() < EPS) && (d - *duty).abs() >= EPS {
                *duty = d;
                *pt = (*p as f32 * d) as i32;
            }
        }
    }
}

/// rekordbox's `LFO`: a wave between an amplitude range, its jumps smoothed (a straight line
/// over the smoothing time, starting that long before the jump), re-timed and restarted by the
/// beat quantizer.
#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    rate: f32,
    pub period_ms: f32,
    pub pos: i32,
    pub head: f32,
    pub amin: f32,
    pub amax: f32,
    rev: bool,
    amp: Sf,
    off: Sf,
    ws: Sf,
    sleft: i32,
    lead: i32,
    pub wave: Wave,
}

impl Lfo {
    /// `LFO::LFO` at 44.1 kHz, then (as `BeatFxTrans`'s constructor) the rate, the range 0 .. 1,
    /// the 3.3333 ms smoothing and the wave.
    pub fn new(rate: f32, wave: Wave) -> Lfo {
        let mut l = Lfo { rate: 44100.0, period_ms: 1000.0, pos: 0, head: 0.0, amin: 0.0, amax: 1.0, rev: false, amp: Sf::new(44100.0), off: Sf::new(44100.0), ws: Sf::new(44100.0), sleft: -1, lead: (44100.0f64 * 0.0033333) as f32 as i32, wave };
        l.amp.reset(0.5);
        l.off.reset(0.5);
        l.ws.reset(0.0);
        l.set_rate(rate);
        l.set_amplitude_range(true, 0.0, 1.0);
        l.set_smoothing_time(3.3333);
        // (`setWaveForm`: the period in frames, the smoothers at their targets.)
        let p = (frames_of(l.period_ms, l.rate)) as i32;
        l.wave.set_period(p);
        l.amp.reset(l.amp.target);
        l.off.reset(l.off.target);
        l.sleft = -1;
        l.ws.reset(l.ws.target);
        l.set_smoothing_time(3.3333);
        l
    }

    /// `onSamplingRateChanged` 0x10299802c (the lead before a jump is not recomputed there).
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
        let p = (((rate as f64 * 0.001) as f32) * self.period_ms) as i32;
        self.wave.set_period(p);
        self.amp.set_rate(rate);
        self.off.set_rate(rate);
        self.ws.set_rate(rate);
    }

    pub fn set_smoothing_time(&mut self, ms: f32) {
        if ms > 0.0 {
            self.lead = frames_of(ms, self.rate) as i32;
            self.amp.set_time(ms);
            self.off.set_time(ms);
            self.ws.set_time(ms);
        }
    }

    pub fn set_amplitude_range(&mut self, smooth: bool, min: f32, max: f32) {
        if max > min || (max - min).abs() < EPS {
            let half = (max - min) * 0.5;
            let a = if self.rev { -half } else { half };
            let c = (max + min) * 0.5;
            self.amin = min;
            self.amax = max;
            if smooth {
                self.amp.set_target(a);
                self.off.set_target(c);
            } else {
                self.amp.reset(a);
                self.off.reset(c);
            }
        }
    }

    fn wrap_lead(&self, pos: i32) -> i32 {
        let p = self.wave.period().max(1);
        let mut v = p + self.lead + pos;
        loop {
            v -= p;
            if v < p {
                break;
            }
        }
        v
    }

    fn cur_val(&self) -> f32 {
        if self.sleft > 0 || self.ws.on {
            self.ws.cur
        } else {
            self.wave.value(self.pos)
        }
    }

    /// Start a smoothed move from `from` to the wave's value one lead ahead.
    fn start_smooth(&mut self, from: f32) {
        let to = self.wave.value(self.wrap_lead(self.pos));
        self.sleft = -1;
        self.ws.reset(from);
        self.sleft = self.ws.n;
        self.ws.set_target(to);
    }

    /// `setPeriodTime` 0x102998288.
    pub fn set_period_time(&mut self, ms: f32, smooth: bool) {
        if ms > 0.0 && (self.period_ms - ms).abs() >= EPS {
            let old = self.wave.period().max(1);
            self.period_ms = ms;
            let f = frames_of(ms, self.rate);
            self.pos = ((f.trunc() * self.pos as f32) / old as f32) as i32;
            self.wave.set_period(f as i32);
            if smooth {
                let v = self.cur_val();
                self.start_smooth(v);
            }
        }
    }

    /// `setRemainingPeriodTime` 0x102998420.
    pub fn set_remaining_period_time(&mut self, ms: f32, smooth: bool) {
        if ms > 0.0 {
            let old = self.wave.period().max(1);
            let frac = self.pos as f32 / old as f32;
            let v12 = (if frac <= self.head { self.head } else { -self.head }) + frac;
            let v13 = ms / (1.0 - v12);
            let v14 = ((self.rate as f64 * 0.001) as f32) * v13;
            let v15 = v14 as i32;
            let v16 = v14.trunc();
            self.period_ms = v13;
            let v18 = (self.head * v16) as i32 + (v12 * v16) as i32;
            self.pos = v18 - if v18 <= v15 { 0 } else { v15 };
            self.wave.set_period(v15);
            if smooth {
                let v = self.cur_val();
                self.start_smooth(v);
            }
        }
    }

    pub fn reversed(&self) -> bool {
        self.rev
    }

    /// `reverseAmplitude(on, smooth)` 0x102998824: the wave turned over (its half-range made
    /// negative), over the smoothing time or at once.
    pub fn reverse_amplitude(&mut self, on: bool, smooth: bool) {
        if self.rev != on {
            let was = self.rev;
            self.rev = !was;
            let half = (self.amax - self.amin) * 0.5;
            let v = if was { half } else { -half };
            if smooth {
                let c = self.amp.cur;
                self.amp.reset(c);
                self.amp.set_target(v);
            } else {
                self.amp.reset(v);
            }
        }
    }

    pub fn set_head(&mut self, h: f32) {
        if (h > 0.0 || h.abs() < EPS) && (h < 1.0 || (h - 1.0).abs() < EPS) {
            self.head = h;
        }
    }

    pub fn duty(&self) -> f32 {
        self.wave.duty()
    }

    /// `setDutyRatio` 0x102998618.
    pub fn set_duty(&mut self, d: f32, smooth: bool) {
        if (d > 0.0 || d.abs() < EPS) && (d < 1.0 || (d - 1.0).abs() < EPS) && matches!(self.wave, Wave::Square { .. }) && (d - self.wave.duty()).abs() >= EPS {
            if smooth {
                let v = self.cur_val();
                self.wave.set_duty(d);
                self.start_smooth(v);
            } else {
                self.wave.set_duty(d);
            }
        }
    }

    /// `reset` 0x1029988b8.
    pub fn reset(&mut self) {
        let (lo, hi) = (self.amin, self.amax);
        self.amp.reset((hi - lo) * 0.5);
        self.off.reset((lo + hi) * 0.5);
        self.rev = false;
        self.sleft = -1;
        self.ws.reset(self.ws.target);
        self.pos = 0;
    }

    /// `jumpToHeadPosition` 0x10299892c.
    pub fn jump_to_head(&mut self, smooth: bool) {
        let p = self.wave.period();
        let mut v = ((self.head * p as f32) as i32).max(0);
        if v >= p {
            v = p - 1;
        }
        if !smooth {
            self.pos = v;
            self.amp.reset(self.amp.target);
            self.off.reset(self.off.target);
            self.sleft = -1;
            self.ws.reset(self.ws.target);
        } else {
            let from = self.cur_val();
            self.pos = v;
            self.start_smooth(from);
        }
    }

    /// `getCurrentPosition` 0x102999224.
    pub fn current_position(&self) -> f32 {
        let r = self.pos as f32 / self.wave.period().max(1) as f32;
        if (r.abs() >= EPS && r <= 0.0) || r >= 1.0 {
            0.0
        } else {
            r
        }
    }

    fn advance(&mut self, n: i32) {
        let p = self.wave.period().max(1);
        let mut v = p + self.pos + n;
        loop {
            v -= p;
            if v < p {
                break;
            }
        }
        self.pos = v;
    }

    /// `processWaveForm` 0x102998f58.
    fn wave_values(&mut self, out: &mut [f32]) {
        let mut o = 0;
        let mut n = out.len() as i32;
        while n > 0 {
            let mut v8 = self.sleft;
            if v8 <= 0 {
                if !self.ws.on {
                    for (k, x) in out[o..o + n as usize].iter_mut().enumerate() {
                        *x = self.wave.value(self.pos + k as i32);
                    }
                    self.advance(n);
                    return;
                }
                v8 = self.ws.left;
            }
            let take = n.min(v8).max(1);
            for x in out[o..o + take as usize].iter_mut() {
                *x = self.ws.take();
            }
            self.sleft -= take;
            if self.sleft < 1 {
                self.sleft = -1;
            }
            self.advance(take);
            o += take as usize;
            n -= take;
        }
    }

    /// `getValues` 0x102998ab0: the next `out.len()` values.
    pub fn get_values(&mut self, out: &mut [f32]) {
        if out.is_empty() {
            return;
        }
        let p = self.wave.period().max(1);
        let (pts, np) = self.wave.points();
        let mut v4 = 99_999_999;
        for &a in &pts[..np] {
            let mut d = if a >= self.pos { 0 } else { p } + a - self.pos;
            if d == 0 {
                d = p;
            }
            v4 = v4.min(d);
        }
        if v4 < self.lead && self.sleft <= 0 && !self.ws.on {
            let v = self.wave.value(self.pos);
            self.start_smooth(v);
        }
        let mut o = 0usize;
        let mut rem = out.len() as i32;
        while rem > 0 {
            let mut v27 = 99_999_999;
            for &a in &pts[..np] {
                let v30 = (a - self.lead).rem_euclid(p);
                let mut d = if v30 >= self.pos { 0 } else { p } + v30 - self.pos;
                if d == 0 {
                    d = p;
                }
                v27 = v27.min(d);
            }
            let v48 = (p - self.pos).min(v27);
            let v49 = v48.min(rem).max(1);
            self.wave_values(&mut out[o..o + v49 as usize]);
            rem -= v49;
            if v49 == v27 {
                let v = self.cur_val();
                self.start_smooth(v);
            }
            o += v49 as usize;
        }
        // `processAmplitude` 0x10299907c, then the clamp to the range.
        for x in out.iter_mut() {
            let a = self.amp.take();
            let c = self.off.take();
            *x = (*x * a + c).max(self.amin).min(self.amax);
        }
    }
}

/// `BeatFxTrans::TransParameter` (the part the gate uses).
#[derive(Clone, Copy, Debug)]
pub struct TransPar {
    pub dry: f32,
    pub wet: f32,
    pub duty: [f32; 2],
    /// The 5 ms rule on the open share.
    pub min5: bool,
    pub amp: [f32; 2],
    pub head: [f32; 2],
    /// A side whose LFO runs at its own period and ignores the beat (`free_ms`).
    pub free: [bool; 2],
    pub free_ms: [f32; 2],
}

impl TransPar {
    pub const fn new() -> TransPar {
        TransPar { dry: 0.0, wet: 1.0, duty: [0.5; 2], min5: false, amp: [1.0; 2], head: [0.0; 2], free: [false; 2], free_ms: [10.0; 2] }
    }

    /// `MultiBeatFxTrans::updateParameter` 0x102a51790.
    pub fn multi(l: f32) -> TransPar {
        let (dry, wet) = crate::fx::half_mix(l);
        let d = if l >= 0.5 { l * -0.875 + 0.9375 } else { 0.5 };
        TransPar { dry, wet, duty: [d; 2], min5: true, ..TransPar::new() }
    }

    /// `SingleBeatFxTrans::updateParameter` 0x102a51ac0: DUTY (1 .. 0.5 .. 0.0625) and SWING:
    /// one side's LFO restarts 1 - 0.37 x the distance from the centre into its turn (the right
    /// side on the knob's left half, the left side on its right half).
    pub fn single(l: f32, duty: f32, swing: f32) -> TransPar {
        let (dry, wet) = crate::fx::half_mix(l);
        let d = crate::fx::trans_single_duty(duty);
        let t = crate::fx::through_value(swing) * 0.37;
        let head = if swing < 0.48627 { [0.0, 1.0 - t] } else if swing > 0.51373 { [1.0 - t, 0.0] } else { [0.0, 0.0] };
        TransPar { dry, wet, duty: [d; 2], min5: true, head, ..TransPar::new() }
    }
}

/// `BeatFxTrans` and its `TransCore`: two LFOs, one per side, giving each side's gain.
#[derive(Clone, Copy, Debug)]
pub struct TransFx {
    pub lfo: [Lfo; 2],
    set_period: [Option<f32>; 2],
    jump: [bool; 2],
    pub last_period: f32,
    pub par: TransPar,
}

impl TransFx {
    pub fn new(rate: f32, wave: Wave) -> TransFx {
        TransFx { lfo: [Lfo::new(rate, wave); 2], set_period: [None; 2], jump: [false; 2], last_period: 10.0, par: TransPar::new() }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.lfo.iter_mut().for_each(|l| l.set_rate(rate));
    }

    /// `updateAndSetParameter` 0x102a51058.
    fn update(&mut self) {
        let p0 = self.lfo[0].period_ms;
        for i in 0..2 {
            let mut d = self.par.duty[i];
            if self.par.min5 && d * p0 < 5.0 {
                d = 5.0 / p0;
            }
            if (d - self.lfo[i].duty()).abs() >= EPS {
                self.lfo[i].set_duty(d, true);
            }
        }
        for i in 0..2 {
            let l = &mut self.lfo[i];
            if (l.amax - self.par.amp[i]).abs() >= EPS {
                let lo = l.amin;
                l.set_amplitude_range(true, lo, self.par.amp[i]);
            }
            if (l.head - self.par.head[i]).abs() >= EPS {
                l.set_head(self.par.head[i]);
            }
            if self.par.free[i] && (self.par.free_ms[i] - l.period_ms).abs() >= EPS {
                l.set_period_time(self.par.free_ms[i], true);
            }
        }
    }

    /// `processEffectConcrete` / `processTrans` / `TransCore::processEffect`: each side's gain
    /// for the next frames.
    pub fn gains(&mut self, gl: &mut [f32], gr: &mut [f32]) {
        self.update();
        for i in 0..2 {
            if let Some(ms) = self.set_period[i].take() {
                self.lfo[i].set_period_time(ms, true);
                self.last_period = ms;
            }
            if self.jump[i] {
                self.jump[i] = false;
                self.lfo[i].jump_to_head(true);
            }
        }
        self.lfo[0].get_values(gl);
        self.lfo[1].get_values(gr);
    }

    /// `onCalledMakeSelected` 0x102a51484.
    pub fn make_selected(&mut self) {
        self.update();
        for i in 0..2 {
            if let Some(ms) = self.set_period[i].take() {
                self.lfo[i].set_period_time(ms, false);
                self.last_period = ms;
            }
            self.lfo[i].reset();
        }
    }
}

impl BeatSync for TransFx {
    fn play_head(&mut self) {
        self.jump = [!self.par.free[0], !self.par.free[1]];
    }

    fn set_beat_time(&mut self, ms: f32, _offset: f32) {
        let v = if ms >= 10.0 { ms.min(32000.0) } else { 10.0 };
        for i in 0..2 {
            if self.par.free[i] {
                self.jump[i] = false;
                self.set_period[i] = None;
            } else {
                self.set_period[i] = Some(v);
                self.jump[i] = true;
            }
        }
    }

    fn time_until_next(&mut self, ms: f32) {
        let p0 = self.lfo[0].period_ms;
        self.lfo[0].set_remaining_period_time(ms, true);
        self.lfo[1].set_remaining_period_time(ms, true);
        let v7 = self.lfo[0].period_ms;
        if v7 > 32000.0 || v7 < 10.0 {
            self.lfo[0].set_period_time(p0, true);
            self.lfo[1].set_period_time(p0, true);
        }
    }

    fn turn_on(&mut self, _offset: f32) {
        self.play_head();
    }

    fn turn_off(&mut self) {}
}

/// `BeatFxPan::PanParameter` (the part that reaches the gate).
#[derive(Clone, Copy, Debug)]
pub struct PanPar {
    pub dry: f32,
    pub wet: f32,
    pub amp: [f32; 2],
    pub flag: bool,
    pub offs: [f32; 2],
    pub free: [bool; 2],
    pub free_ms: [f32; 2],
}

impl PanPar {
    /// `MultiBeatFxPan::updateParameter` 0x102a4eddc.
    pub fn multi(l: f32) -> PanPar {
        let (dry, wet) = crate::fx::half_mix(l);
        PanPar { dry, wet, amp: [1.4125; 2], flag: true, offs: [0.0; 2], free: [false; 2], free_ms: [0.0; 2] }
    }

    /// `SingleBeatFxPan::updateParameter` 0x102a4f0a8 (knob 1 in its rate mode, as the single
    /// FX layer makes it): one side's LFO turns 1/8 .. 1 x as long as the other's on the knob's
    /// half for that side, on its own; SWING moves the two sides' restart points apart by up to
    /// 0.185 of a turn each.
    pub fn single(l: f32, k1: f32, k2: f32, last_period: f32) -> PanPar {
        let (dry, wet) = crate::fx::half_mix(l);
        let (mut a, mut b) = (1.0f32, 1.0f32);
        let z = k1;
        if z < 0.05 {
            a = 0.125;
        } else if z < 0.15 {
            a = 0.25;
        } else if z < 0.25 {
            a = 0.33333;
        } else if z < 0.35 {
            a = 0.5;
        } else if z < 0.45 {
            a = 0.66667;
        } else if z < 0.55 {
        } else if z < 0.65 {
            b = 0.66667;
        } else if z < 0.75 {
            b = 0.5;
        } else if z < 0.85 {
            b = 0.33333;
        } else if z < 0.95 {
            b = 0.25;
        } else {
            b = 0.125;
        }
        let two = last_period + last_period;
        let free = [(a - 1.0).abs() >= EPS, (b - 1.0).abs() >= EPS];
        let free_ms = [if free[0] { a * two } else { 0.0 }, if free[1] { b * two } else { 0.0 }];
        let t = crate::fx::through_value(k2);
        let offs = if k2 < 0.48627 { [t * -0.185, t * 0.185] } else if k2 > 0.51373 { [t * 0.185, t * -0.185] } else { [0.0, 0.0] };
        PanPar { dry, wet, amp: [1.4125; 2], flag: true, offs, free, free_ms }
    }
}

/// `BeatFxPan`: a `BeatFxTrans` on two triangle LFOs over 0 .. 1.4125 whose turn is two beat
/// values; the two sides start half a turn apart and swap their restart points every beat value.
#[derive(Clone, Copy, Debug)]
pub struct PanFx {
    pub trans: TransFx,
    base: f32,
    restart: bool,
    flip: bool,
    set_bt: bool,
    bt_ms: f32,
    pub par: PanPar,
}

impl PanFx {
    pub fn new(rate: f32) -> PanFx {
        let mut trans = TransFx::new(rate, Wave::TRI);
        trans.par = TransPar { dry: 0.0, wet: 1.0, duty: [0.5; 2], min5: false, ..TransPar::new() };
        PanFx { trans, base: 0.5, restart: true, flip: false, set_bt: false, bt_ms: 100.0, par: PanPar::multi(0.0) }
    }

    /// `processEffectConcrete` 0x102a4e768: each side's gain for the next frames.
    pub fn gains(&mut self, gl: &mut [f32], gr: &mut [f32]) {
        let t = &mut self.trans.par;
        t.amp = self.par.amp;
        t.free = self.par.free;
        t.free_ms = [self.par.free_ms[0] * 0.5, self.par.free_ms[1] * 0.5];
        if self.restart {
            let base = self.base;
            let other = base + if base > 0.5 { -0.5 } else { 0.5 };
            let one = self.flip != self.par.flag;
            let (hl, hr) = if one { (other, base) } else { (base, other) };
            let wrap = |mut x: f32| {
                if x < 0.0 {
                    x += 1.0;
                }
                if (x - 1.0).abs() < EPS || x > 1.0 {
                    x -= 1.0;
                }
                x
            };
            t.head = [wrap(hl + self.par.offs[0]), wrap(hr + self.par.offs[1])];
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
}

impl BeatSync for PanFx {
    fn play_head(&mut self) {
        if !self.restart {
            self.restart = true;
            self.flip = !self.flip;
        }
    }

    fn set_beat_time(&mut self, ms: f32, _offset: f32) {
        self.play_head();
        self.set_bt = true;
        self.bt_ms = ms + ms;
    }

    /// `setTimeUntilNextPlayHead` 0x102a4ea6c.
    fn time_until_next(&mut self, ms: f32) {
        let in01 = |v: f32| (v.abs() < EPS || v > 0.0) && v < 1.0;
        let target = |base: f32, use_base: bool| -> f32 {
            if use_base {
                return base;
            }
            let mut v9 = if in01(base) { base + 0.5 } else { 0.5 };
            while v9 < 0.0 {
                v9 += 1.0;
            }
            if v9 > 1.0 || (v9 - 1.0).abs() < EPS {
                loop {
                    let v6 = v9 - 1.0;
                    let more = v6 > 1.0 || (v9 - 2.0).abs() < EPS;
                    v9 = v6;
                    if !more {
                        break;
                    }
                }
            }
            v9
        };
        let flag = self.par.flag;
        let v4 = if flag { !self.flip } else { self.flip };
        let v5 = if flag { self.flip } else { !self.flip };
        for (i, use_base) in [(0usize, v4), (1, v5)] {
            let h = target(self.base, use_base);
            let cp = self.trans.lfo[i].current_position();
            let h = if in01(h) { h } else { 0.0 };
            let mut d = h - cp;
            if (h - cp).abs() >= EPS && h <= cp {
                d += 1.0;
            }
            if d > 1e-6 {
                self.trans.lfo[i].set_period_time(ms / d, true);
            }
        }
    }

    fn turn_on(&mut self, _offset: f32) {
        self.restart = true;
        self.flip = true;
    }

    fn turn_off(&mut self) {}
}

/// What the beat quantizer asks of an effect (`EffectBase`'s virtual calls).
pub trait BeatSync {
    fn play_head(&mut self);
    fn set_beat_time(&mut self, ms: f32, offset: f32);
    fn time_until_next(&mut self, ms: f32);
    fn turn_on(&mut self, offset: f32);
    fn turn_off(&mut self);
}

/// rekordbox's `FxQuantizeManager`: restarts the effect at every turn of its beat value counted
/// from the beat (or half beat) it was switched on at, splitting the audio block there.
#[derive(Clone, Copy, Debug)]
pub struct Quant {
    rate: f64,
    cycle: f64,
    new_cycle: f64,
    prev: f64,
    on: bool,
    want: bool,
    next: f64,
    origin: f64,
    defer: bool,
    pend: bool,
    pub slip: bool,
    adj: bool,
    applied: Option<f32>,
    rev: bool,
    /// The beat position counted here when no beat grid is given (rekordbox's BPM mode).
    clock: f64,
}

#[inline]
fn round_even(x: f64) -> i64 {
    x.round_ties_even() as i64
}

impl Quant {
    pub const fn new() -> Quant {
        Quant { rate: 48000.0, cycle: 1.0, new_cycle: 1.0, prev: 0.0, on: false, want: false, next: 1.0, origin: 1.0, defer: false, pend: false, slip: false, adj: false, applied: None, rev: false, clock: 0.0 }
    }

    fn frames(&self, beats: f64, bpm: f64) -> i64 {
        round_even(beats * (60.0 * self.rate) / bpm)
    }

    fn ms_of(&self, beats: f64, bpm: f64) -> f32 {
        ((1000 * self.frames(beats, bpm)) as f64 / self.rate) as f32
    }

    /// `onCycleBeatChanged` 0x102a8d330.
    fn cycle_changed(&mut self, b: f32, now: f64) {
        let mut r = b as f64;
        if self.on {
            if b >= 0.0 {
                if self.rev {
                    self.rev = false;
                    self.new_cycle = r;
                    self.pend = true;
                    return;
                }
                let v = self.origin + r;
                if v > now {
                    self.next = v;
                }
            } else {
                if !self.rev {
                    self.rev = true;
                    self.new_cycle = -r;
                    self.pend = true;
                    return;
                }
                let v = self.origin - r;
                if v > now {
                    self.next = v;
                }
                r = -r;
            }
        } else {
            self.rev = b < 0.0;
            r = r.abs();
            self.cycle = r;
        }
        self.new_cycle = r;
    }

    /// `doRetrigger` 0x102a8cfc8.
    fn retrigger<E: BeatSync>(&mut self, fx: &mut E, now: f64, bpm: f64) {
        if self.slip && !self.adj {
            fx.play_head();
        } else {
            let ms = self.ms_of(self.new_cycle, bpm);
            let off = if self.slip { (((-1000 * self.frames(now - now.trunc(), bpm)) as i32) as f64 / self.rate) as f32 } else { 0.0 };
            fx.set_beat_time(if self.rev { -ms } else { ms }, off);
        }
        self.defer = false;
        self.pend = false;
        self.adj = false;
    }

    /// `adjustPlayAHeadTiming` 0x102a8d868.
    fn adjust<E: BeatSync>(&mut self, fx: &mut E, now: f64, bpm: f64, half: bool) {
        let (v6, v5) = (self.cycle, self.new_cycle);
        if (v6 - v5).abs() >= 2.220446e-16 || self.pend {
            let mut retrig = true;
            if v6 <= v5 {
                if !self.slip {
                    if !self.defer && (v6 <= 1.0 || now - self.origin < 1.0) {
                        let ms = self.ms_of(v5 - now + self.origin, bpm);
                        fx.time_until_next(ms);
                        self.adj = true;
                        retrig = false;
                    }
                } else {
                    self.adj = true;
                }
            } else if !self.slip {
                if !self.defer && now - self.origin <= v6 {
                    retrig = false;
                }
            } else {
                self.adj = true;
            }
            if retrig {
                self.retrigger(fx, now, bpm);
            }
            self.cycle = self.new_cycle;
        }
        let v8 = if half { 0.5 } else { 0.0 } + now.trunc();
        self.next = self.new_cycle + v8;
        self.origin = v8;
    }

    /// `playAHeadWithAdjust` 0x102a8d55c.
    fn play_ahead<E: BeatSync>(&mut self, fx: &mut E, now: f64, bpm: f64, grid: bool) {
        if !grid {
            self.cycle = self.new_cycle;
            self.adj = true;
            self.retrigger(fx, now, bpm);
            self.origin = now;
            self.next = self.new_cycle + now;
            self.pend = false;
        } else {
            let f = now - now.trunc();
            if f < 0.25 {
                self.adjust(fx, now, bpm, false);
            } else if (0.5..0.75).contains(&f) {
                self.adjust(fx, now, bpm, true);
            } else if (self.cycle - self.new_cycle).abs() >= 2.220446e-16 {
                self.defer = true;
            }
        }
    }

    /// `turnOnWithAdjust` 0x102a8d650.
    fn turn_on<E: BeatSync>(&mut self, fx: &mut E, now: f64, bpm: f64, grid: bool) {
        let mut v3 = 0.0f32;
        if !grid {
            self.on = self.want;
            self.next = self.cycle + now;
            self.origin = now;
        } else {
            let v4 = now.trunc();
            let f = now - v4;
            if f < 0.25 {
                self.on = self.want;
                v3 = -(round_even(f * 60.0 * self.rate / bpm) as i32 as f32);
                self.next = self.cycle + v4;
                self.origin = v4;
            } else if (0.5..0.75).contains(&f) {
                self.on = self.want;
                v3 = -(round_even(self.rate * (f * 60.0 - 30.0) / bpm) as i32 as f32);
                self.next = self.cycle + v4 + 0.5;
                self.origin = v4 + 0.5;
            }
        }
        if self.on {
            let off = (v3 as f64 * 1000.0 / self.rate) as f32;
            let mut ms = ((self.frames(self.next - self.origin, bpm) as i32) as f64 * 1000.0 / self.rate) as f32;
            if self.rev {
                ms = -ms;
            }
            fx.set_beat_time(ms, off);
            fx.turn_on(off);
        }
    }

    /// Whether the effect is running (switched on and started on the grid).
    pub fn running(&self) -> bool {
        self.on
    }

    /// `updateParameter` + `processQuantizeEffect` 0x102a8ce50 for one block of `frames`:
    /// `run(fx, start, len)` processes each stretch between restarts. `beat` is the deck's
    /// place on its beat grid at the block's start (None: no grid, a beat clock of its own).
    pub fn process<E: BeatSync>(&mut self, fx: &mut E, beat: Option<f64>, bpm: f32, rate: u32, frames: usize, cycle_beats: f32, on: bool, mut run: impl FnMut(&mut E, usize, usize)) {
        self.rate = rate as f64;
        let bpm = (bpm as f64).max(1.0);
        let grid = beat.is_some();
        let now = beat.unwrap_or(self.clock);
        self.want = on;
        if self.applied != Some(cycle_beats) && cycle_beats.abs() >= EPS {
            self.applied = Some(cycle_beats);
            self.cycle_changed(cycle_beats, now);
        }
        if (self.cycle - self.new_cycle).abs() >= 2.220446e-16 || self.pend {
            self.play_ahead(fx, now, bpm, grid);
        }
        if self.on != self.want {
            if self.want {
                self.turn_on(fx, now, bpm, grid);
            } else {
                self.on = false;
                fx.turn_off();
            }
        }
        if (self.prev as i64) > (now as i64) {
            self.next -= 0.5;
        }
        let mut v9 = now;
        let mut at = 0usize;
        let mut n = frames;
        while n > 0 {
            let v12 = round_even((self.next - v9) * (60.0 * self.rate) / bpm);
            let v13 = v12.max(0) as usize;
            if n <= v13 {
                run(fx, at, n);
                break;
            }
            if v12 >= 1 {
                run(fx, at, v13);
            }
            let v10 = self.next;
            self.next = self.cycle + v10;
            self.origin = v10;
            self.retrigger(fx, now, bpm);
            v9 += bpm * v13 as f64 / (self.rate * 60.0);
            at += v13;
            n -= v13;
        }
        self.prev = now;
        if !grid {
            self.clock += frames as f64 * bpm / (60.0 * self.rate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_lfo_edges_ramp_over_the_smoothing_time_before_the_jump() {
        // A 200 ms square at 48 kHz, open half the turn: 4800 frames at 1, 4800 at 0, each jump
        // a straight line over 159 frames (3.3333 ms) that ends on the jump.
        let mut l = Lfo::new(48000.0, Wave::SQUARE);
        l.set_period_time(200.0, false);
        l.reset();
        let mut v = vec![0.0f32; 9600];
        for c in v.chunks_mut(512) {
            l.get_values(c);
        }
        assert_eq!(l.lead, 159);
        assert_eq!(v[0], 1.0);
        assert_eq!(v[4800 - 160], 1.0);
        assert!((v[4800 - 159] - 1.0).abs() < 1e-6, "{}", v[4800 - 159]);
        assert!((v[4800 - 80] - 0.5 - 0.5 / 159.0).abs() < 1e-3, "{}", v[4800 - 80]);
        assert_eq!(v[4800], 0.0);
        assert_eq!(v[9600 - 160], 0.0);
        assert!(v[9599] > 0.99);
    }

    #[test]
    fn trans_swing_starts_one_side_later() {
        // SWING fully right: the left LFO restarts at 0.63 of its turn, so it opens 0.37 of a
        // turn after the right one.
        let mut t = TransFx::new(48000.0, Wave::SQUARE);
        t.par = TransPar::single(1.0, 0.5, 1.0);
        assert!((t.par.head[0] - 0.63).abs() < 1e-6 && t.par.head[1] == 0.0);
        t.set_beat_time(500.0, 0.0);
        let (mut gl, mut gr) = (vec![0.0f32; 24000], vec![0.0f32; 24000]);
        for (a, b) in gl.chunks_mut(512).zip(gr.chunks_mut(512)) {
            t.gains(a, b);
        }
        // Right: open 0 .. 12000. Left: 0.63 x 24000 = 15120 into its turn: shut until 24000 -
        // 15120 = 8880, then open.
        assert!(gr[1000] > 0.99 && gr[13000] < 0.01);
        assert!(gl[4000] < 0.01 && gl[9200] > 0.99, "{} {}", gl[4000], gl[9200]);
    }

    #[test]
    fn pan_moves_linearly_between_the_sides_each_beat_value() {
        // Multi PAN, a 500 ms beat value at 48 kHz: the left side starts at 1.4125 and falls in
        // a straight line to 0 over the beat value (24,000 frames) while the right one rises;
        // the next beat value (restart) takes them back.
        let mut p = PanFx::new(48000.0);
        p.par = PanPar::multi(1.0);
        p.set_beat_time(500.0, 0.0);
        p.turn_on(0.0);
        let (mut gl, mut gr) = (vec![0.0f32; 48000], vec![0.0f32; 48000]);
        for k in 0..2 {
            let (a, b) = (&mut gl[k * 24000..(k + 1) * 24000], &mut gr[k * 24000..(k + 1) * 24000]);
            if k == 1 {
                p.set_beat_time(500.0, 0.0);
            }
            for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
                p.gains(x, y);
            }
        }
        let near = |a: f32, b: f32| (a - b).abs() < 2e-3;
        // (The first restart moves from where the LFOs were over 3.3333 ms.)
        assert!(near(gl[200], 1.4125 * (1.0 - 200.0 / 24000.0)) && near(gr[200], 1.4125 * 200.0 / 24000.0), "{} {}", gl[200], gr[200]);
        assert!(near(gl[12000], 0.70625) && near(gr[12000], 0.70625), "{} {}", gl[12000], gr[12000]);
        assert!(near(gl[23999], 0.0) && near(gr[23999], 1.4125));
        assert!(near(gl[36000], 0.70625) && near(gl[47999], 1.4125), "{} {}", gl[36000], gl[47999]);
    }

    #[test]
    fn quantizer_starts_on_the_beat_and_restarts_each_turn() {
        struct Log(Vec<(usize, &'static str, f32)>, usize);
        impl BeatSync for Log {
            fn play_head(&mut self) { self.0.push((self.1, "head", 0.0)); }
            fn set_beat_time(&mut self, ms: f32, o: f32) { self.0.push((self.1, "beat", ms + o * 0.0)); }
            fn time_until_next(&mut self, ms: f32) { self.0.push((self.1, "until", ms)); }
            fn turn_on(&mut self, o: f32) { self.0.push((self.1, "on", o)); }
            fn turn_off(&mut self) {}
        }
        let mut q = Quant::new();
        let mut log = Log(Vec::new(), 0);
        // 120 BPM, 48 kHz: a beat is 24000 frames. Switched on at beat 3.1 (on the beat's first
        // quarter): started at once, 0.1 beat late (-50 ms), half-beat turns from beat 3.
        let mut beat = 3.1;
        for b in 0..60 {
            log.1 = b * 512;
            q.process(&mut log, Some(beat), 120.0, 48000, 512, 0.5, true, |_, _, _| {});
            beat += 512.0 / 24000.0;
        }
        assert_eq!(log.0[0], (0, "beat", 250.0));
        assert_eq!(log.0[1], (0, "on", -50.0));
        // The next restart at beat 3.5: 0.4 beat = 9600 frames in.
        let r: Vec<_> = log.0.iter().filter(|e| e.1 == "beat").skip(1).map(|e| e.0).collect();
        assert_eq!(r[0], 9600 / 512 * 512);
        // Switched on at beat 3.3 (second quarter): it waits for the half beat.
        let mut q = Quant::new();
        let mut log = Log(Vec::new(), 0);
        q.process(&mut log, Some(3.3), 120.0, 48000, 512, 0.5, true, |_, _, _| {});
        assert!(!q.running() && !log.0.iter().any(|e| e.1 == "on"));
    }
}
