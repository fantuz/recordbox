//! SOUND COLOR FX (the CFX row of the FX panel): one effect per channel, driven by its COLOR
//! knob (centre = off) and the shared PARAM knob. In the signal path it sits after the EQ and
//! before the channel fader, as in rekordbox 7.2.19 (`checks/re-7.2.19/mixer-fx-recording.md`
//! 2.6); the master's sits on the master mix.
//!
//! Every type is ported from rekordbox's code (addresses in
//! `checks/re-7.2.19/track-notes/fx-audit.md`): FILTER (`ColorFxFilterDjm900NXS2`: the knobs'
//! 8-bit glide, double-precision `Filter::make_lpf2` / `make_hpf2` with PARAM's damping and the
//! gain that goes with it, the three-way blend), JET and DUB ECHO (`TapeEchoCore` on a cross-fading
//! delay line; DUB ECHO's two cores with their pre and post filters), CRUSH (`CrushCore`, the
//! mantissa mask, `ColorFxCrush::IfxHPF`), NOISE, PITCH (through rekordbox's pitch shifter in
//! `fx_core`), SWEEP (`ColorFxSweep`), SPACE and GATE COMP (`cfx_space`). LPF and HPF
//! (`InstFxLPF` / `InstFxHPF`, for the PAD FX pads, MERGE FX and the echoes' FILTER knob) use
//! rekordbox's cutoff tables, read from the installed binary when the effects are made.

/// The USER menu's nine types in rekordbox's order (the effect if recordbox has it).
pub const LIST: [(&str, Option<u8>); 9] = [("FILTER", Some(0)), ("JET", Some(1)), ("CRUSH", Some(2)), ("NOISE", Some(3)), ("PITCH", Some(4)), ("SPACE", Some(5)), ("DUB ECHO", Some(6)), ("SWEEP", Some(7)), ("GATE COMP", Some(8))];

/// The effects DEFAULT offers for a mixer, in the order of its buttons. Without hardware
/// rekordbox shows FILTER alone; the lists of its supported mixers were not traced, so this is
/// the one list there is.
pub const DEFAULT_LIST: [u8; 1] = [0];

pub fn name(kind: u8) -> &'static str {
    match kind {
        LPF => "LPF",
        HPF => "HPF",
        _ => LIST.iter().find(|x| x.1 == Some(kind)).map(|x| x.0).unwrap_or("FILTER"),
    }
}

/// PAD FX's LPF and HPF (rekordbox's `InstFxLPF` / `InstFxHPF`), beside the nine types.
pub const LPF: u8 = 12;
pub const HPF: u8 = 13;

/// Cutoff of rekordbox's CFX-style low-pass (12 kHz down to 93 Hz) or high-pass (61 Hz up to
/// 8.95 kHz) for how far the knob is turned (0..1): its 124-step tables, every 15th step.
pub fn inst_fc(hp: bool, x: f32) -> f32 {
    const LP: [f32; 9] = [12000.0, 6634.1, 3685.5, 1949.0, 990.7, 509.1, 249.7, 132.7, 93.0];
    const HP: [f32; 9] = [61.0, 154.0, 292.0, 523.0, 994.0, 1750.0, 3136.0, 5418.0, 8951.0];
    crate::fx::knots(if hp { &HP } else { &LP }, x)
}

/// `InstFxLPF` / `InstFxHPF::updateParameter` for the knob `a` and PARAM `p` in 8-bit steps:
/// (share of the filtered sound, cutoff, resonance).
pub fn inst_law(hp: bool, a: f32, p: f32) -> (f32, f32, f32) {
    if a < 4.0 {
        return (0.0, if hp { 61.0 } else { 12000.0 }, 1.44);
    }
    let wet = if a > 37.0 { 1.0 } else { ((a - 4.0) * 0.029412).min(1.0) };
    let u = if hp { (a - 3.0) * 0.0039683 } else { (255.0 - a) * 0.0039683 };
    let v7 = (1.0 - u) - u * (1.0 - u);
    let k = (255.0 - p) * 0.0039216;
    let v8 = 1.44 - k * k * (255.0 - p) * if hp { 0.0050974 } else { 0.0056471 };
    (wet, inst_fc(hp, (a / 2.0).floor() / 123.0), (v8 - v7.powi(8) * v8).max(0.0))
}

/// rekordbox's CFX JET for COLOR `c` and PARAM `p` in 8-bit steps (`ColorFxJetRZX::
/// updateAndSetParameter`): (dry, wet, comb delay in ms, feedback). Left the comb grows from
/// 2.67 ms to 20 ms, right it shrinks to 0.167 ms.
pub fn jet_law(c: f32, p: f32) -> (f32, f32, f32, f32) {
    let (dry, wet, delay, mut fb);
    if (124.0..=131.0).contains(&c) {
        return (1.0, 0.0, 2.6667, 0.6);
    } else if c < 124.0 {
        let n = 124.0 - c;
        delay = 2.6667 + 0.13978 * n;
        (dry, wet) = if c >= 105.0 { (1.0 - 0.015 * n, 0.035 * n) } else { (0.7, 0.7) };
        fb = if c >= 25.0 { 0.55 } else if c < 7.0 { 0.3 } else { 1.8526 - 0.013158 * n };
    } else {
        let m = c - 131.0;
        delay = if c <= 140.0 { 2.6667 } else { (2.8623 - 0.021739 * m).max(0.16667) };
        (dry, wet) = if c <= 150.0 { (1.0 - 0.015 * m, 0.035 * m) } else { (0.7, 0.7) };
        fb = if c <= 193.0 { 0.6 } else if c <= 217.0 { 1.344 - 0.012 * m } else { 0.3 };
    }
    if p <= 123.0 {
        fb *= p * 0.0080645;
    } else if p >= 132.0 {
        fb *= 0.0026882 * p + 0.64785;
    }
    (dry, wet, delay, fb.min(0.993))
}

/// rekordbox's CFX DUB ECHO for COLOR `c` and PARAM `p` in 8-bit steps (`ColorFxDubEcho::
/// onCfxKnobValueChanged`): (input level, echo time in ms, feedback). The time does not
/// follow the tempo: 55 ms near the centre, up to 189 ms fully left and 150 ms fully right.
pub fn dub_law(c: f32, p: f32) -> (f32, f32, f32) {
    if c <= 115.0 {
        let n = 116.0 - c;
        let x = n * 0.0086207;
        let fbp = if p <= 115.0 { 0.0075 * p } else if p < 140.0 { 0.87 } else { 0.87 + 0.0010345 * (p - 139.0) };
        let k = if p < 140.0 { 1.0 } else { (255.0 - p) * 0.0086207 };
        if x >= 0.2 { ((n * 0.028736).min(1.0), 55.0 + 167.5 * (x - 0.2), (fbp + 0.1 * (x - 0.2) * k).min(0.993)) } else { ((n * 0.028736).min(1.0), 55.0, 0.87) }
    } else if c > 138.0 {
        let m = c - 139.0;
        let fbp = if p <= 115.0 { 0.0075862 * p } else if p < 140.0 { 0.88 } else { 0.88 + 0.00094828 * (p - 139.0) };
        let k = if p < 140.0 { 1.0 } else { (255.0 - p) * 0.0086207 };
        if m * 0.0086207 >= 0.2 { ((m * 0.028736).min(1.0), 31.25 + 1.0237 * m, (fbp + 0.00086207 * m * k).min(0.993)) } else { ((m * 0.028736).min(1.0), 55.0, fbp.min(0.993)) }
    } else {
        (0.0, 55.0, 0.87)
    }
}

/// rekordbox's CFX SWEEP filters (`ColorFxSweep::updateFilterCoef` 0x102a48cf0) for COLOR `c`
/// and PARAM `p` (8-bit): PARAM picks a frequency f (60 Hz .. 1 kHz .. 20 kHz). Returns (high-pass
/// cutoff, low-pass cutoff, damping = 1/Q of both; `Filter::make_hpf2` / `make_lpf2`, gain 1).
/// Left of the centre the high-pass falls from 20 kHz and the low-pass rises from 60 Hz toward f
/// (20000 (f/20000)^(c/123), 60 (f/60)^(c/123), damping 1), and the two are added (a gap around
/// f); right of it a band-pass of the two in series, f / k .. f k with k = 1.4 + 0.0010078 (256 -
/// c)^2 (k = 17.4 and damping 1.0079 at 125 .. 130), damping 0.7079 + 0.009375 (162 - c) down to
/// 0.7079.
pub fn sweep_coef(c: i32, p: i32) -> (f64, f64, f64) {
    let (base, step, v) = if p < 128 { (60.0, 7.4015748, p) } else { (1000.0, 149.606299, p - 128) };
    let f = base + step * v as f64;
    if c < 125 {
        if c == 124 {
            return (20000.0, 60.0, 1.0);
        }
        let a = 10f64.powf((f * 0.00005).log10() * 0.0081300813);
        let b = 10f64.powf((f * 0.0166666667).log10() * 0.0081300813);
        return (a.powi(c) * 20000.0, b.powi(c) * 60.0, 1.0);
    }
    if c > 130 {
        let k = ((256 - c) as f64).powi(2) * 0.00100781053 + 1.4;
        let d = if c <= 161 { (162 - c) as f64 * 0.009375 + 0.707945784 } else { 0.707945784 };
        (f / k, f * k, d)
    } else {
        (f * 0.0574712644, f * 17.4, 1.00794578)
    }
}

/// SWEEP's blend (`ColorFxSweep::calculateMixRatio` 0x102a49054): (dry, wet); dry until the
/// knob is 30 steps from the centre on either side, where the filtered sound has taken over.
pub fn sweep_mix(c: i32) -> (f32, f32) {
    let v2 = if (124..132).contains(&c) { 0.5 } else if c < 124 { 1.0 + c as f32 * -0.004065 } else { 0.5 + (c - 132) as f32 * 0.004065 };
    let v8 = (v2 + v2) - 1.0;
    let v9 = if v8 >= 0.25 { 0.0 } else { (v2 * -8.0 + 4.0) + 1.0 };
    let dry = v9 * v9;
    (dry, 1.0 - dry)
}

/// CFX SWEEP as `ColorFxSweep::processEffectConcrete` 0x102a491c4: COLOR glides as an 8-bit
/// value (a step every 0.66667 ms; truncated), PARAM is taken while COLOR rests; the side is
/// decided when COLOR passes 127 (both filters cleared then); dry / wet over 3.3333 ms.
#[derive(Clone, Copy)]
pub struct Sweep {
    c: crate::fx_core::ShiftInt,
    p: i32,
    cur: i32,
    left: bool,
    lp: crate::fx_core::Filter2,
    hp: crate::fx_core::Filter2,
    g: [crate::fx_core::SmoothF; 2],
    rate: u32,
}

impl Sweep {
    pub fn new() -> Sweep {
        Sweep { c: crate::fx_core::ShiftInt::new(127, 0.66667, 48000.0), p: 0, cur: 127, left: false, lp: Default::default(), hp: Default::default(), g: [crate::fx_core::SmoothF::new(1.0, 3.3333, 48000.0), crate::fx_core::SmoothF::new(0.0, 3.3333, 48000.0)], rate: 48000 }
    }

    fn coef(&mut self) {
        self.cur = self.c.cur;
        let (dry, wet) = sweep_mix(self.cur);
        self.g[0].set(dry);
        self.g[1].set(wet);
        let (h, l, d) = sweep_coef(self.cur, self.p);
        let r = self.rate as f64;
        self.hp.c = crate::fx_core::make_hpf2(h, d, 1.0, r);
        self.lp.c = crate::fx_core::make_lpf2(l, d, 1.0, r);
    }

    /// The effect was selected (`onCalledMakeSelected`).
    pub fn reset(&mut self, c: i32, p: i32, rate: u32) {
        self.rate = rate;
        self.c.set_rate(rate as f32);
        self.g.iter_mut().for_each(|g| g.set_rate(rate as f32));
        self.lp.clear();
        self.hp.clear();
        self.c.reset(c);
        self.left = c < 127;
        self.p = p;
        self.coef();
        let (d, w) = sweep_mix(c);
        self.g[0].reset(d);
        self.g[1].reset(w);
    }

    pub fn process(&mut self, buf: &mut [f32], c: i32, p: i32, rate: u32) {
        if rate != self.rate {
            self.reset(self.c.cur, self.p, rate);
        }
        self.c.set(c);
        if p != self.p {
            self.p = p;
            if !self.c.moving() {
                self.coef();
            }
        }
        let frames = buf.len() / 2;
        let mut n = 0;
        while n < frames {
            let moving = self.c.moving();
            let len = if moving { (self.c.count.max(1) as usize).min(frames - n) } else { frames - n };
            self.c.advance(len as i32);
            for f in buf[n * 2..(n + len) * 2].chunks_exact_mut(2) {
                let (gd, gw) = (self.g[0].next(), self.g[1].next());
                for (ch, v) in f.iter_mut().enumerate() {
                    let x = *v as f64;
                    let y = if self.left { self.lp.run(ch, x) + self.hp.run(ch, x) } else { let l = self.lp.run(ch, x); self.hp.run(ch, l) };
                    *v = *v * gd + y as f32 * gw;
                }
            }
            if moving {
                if self.cur == 127 && (self.c.cur == 126 || self.c.cur == 128) {
                    self.left = self.c.cur == 126;
                    self.lp.clear();
                    self.hp.clear();
                }
                self.coef();
            }
            n += len;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Bq {
    b: [f32; 3],
    a: [f32; 2],
    z: [[f32; 2]; 2],
}

impl Bq {
    /// Second-order low-pass (`hp` false) or high-pass at `fc` with resonance `q`.
    fn set(&mut self, rate: f32, fc: f32, q: f32, hp: bool) {
        let w = std::f32::consts::TAU * (fc / rate).clamp(0.0005, 0.45);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q.max(0.05));
        let a0 = 1.0 + alpha;
        self.b = if hp { [(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0] } else { [(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0] };
        self.a = [-2.0 * c / a0, (1.0 - alpha) / a0];
    }

    /// Band-pass (constant peak) at `fc`.
    fn band(&mut self, rate: f32, fc: f32, q: f32) {
        let w = std::f32::consts::TAU * (fc / rate).clamp(0.0005, 0.45);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q.max(0.05));
        let a0 = 1.0 + alpha;
        self.b = [alpha / a0, 0.0, -alpha / a0];
        self.a = [-2.0 * c / a0, (1.0 - alpha) / a0];
    }

    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x + z[1] - self.a[0] * y;
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// rekordbox's CFX FILTER blend (`ColorFxFilterDjm900NXS2::processEffectConcrete`
/// 0x1029d2658) for COLOR `c` (0..255): the gains of the low-pass, the dry sound and the
/// high-pass.
pub fn filter_mix(c: i32) -> (f32, f32, f32) {
    if c < 110 {
        (1.0, 0.0, 0.0)
    } else if c <= 125 {
        let d = (c - 110) as f32 * 0.0625;
        (1.0 - d, d, 0.0)
    } else if c < 131 {
        (0.0, 1.0, 0.0)
    } else if c <= 146 {
        let h = (c - 130) as f32 * 0.0625;
        (0.0, 1.0 - h, h)
    } else {
        (0.0, 0.0, 1.0)
    }
}

/// rekordbox's CFX FILTER settings (`ColorFxFilterDjm900NXS2::updateFilterCoef` 0x1029d2304)
/// for COLOR `c` and PARAM `p` (0..255): (low-pass cutoff, high-pass cutoff, damping = 1/Q,
/// gain), the same damping and gain for both filters. PARAM's damping runs from 4 (left end)
/// through 0.5 (centre) to 0.033 (right end, the most resonance); at the bottom of the
/// high-pass at least 4 (1 - 35.652 x^3)^3 + 0.03 (at most 2). Below a damping of 0.5 the gain
/// falls with it, less so with COLOR far from the quarter points (64 and 192).
pub fn filter_coef(c: i32, p: i32) -> (f32, f32, f32, f32) {
    let (lp_fc, hp_fc, reso) = if c <= 125 {
        let cf = c as f32;
        let x = cf * 0.007936508;
        (x * x * (cf * 198.4127) + 100.0, 20.0f32, 0.0f32)
    } else if c < 131 {
        (25100.0, 20.0, 0.0)
    } else {
        let x = (c - 130) as f32 * 0.007936508;
        let x3 = x * x * x;
        let fc = x3 * 8200.0 + 20.0;
        let mut r = 0.0f32;
        if fc < 249.9 {
            let t = x3 * 35.652172;
            let a = ((1.0 - t) - t * (1.0 - t)) * 4.0;
            r = ((a - t * a) + 0.03).min(2.0);
        }
        (25100.0, fc, r)
    };
    let q = if p > 127 {
        (p - 128) as f32 * -0.0036621094 + 0.5
    } else {
        let y = p as f32 * 0.0078125;
        ((1.0 - y) - y * (1.0 - y)) * 3.5 + 0.5
    };
    let w = if c > 127 { (c - 192).abs() } else { (c - 64).abs() } as f32 * 0.0078125;
    let d = q.max(reso);
    let k = if d < 0.5 { d + d } else { 1.0 };
    (lp_fc, hp_fc, d, (w + 0.5) * k + (0.5 - w))
}

/// CFX FILTER as rekordbox's `ColorFxFilterDjm900NXS2`: COLOR and PARAM glide as 8-bit values
/// (one step every 0.33333 ms), a low-pass and a high-pass (`Filter::make_lpf2` / `make_hpf2`,
/// double precision) both run all the time on the input, and the three gains (low-pass, dry,
/// high-pass) move toward COLOR's blend by 100/fs a frame (10 ms for the whole way).
#[derive(Clone, Copy)]
pub struct NxsFilter {
    c: crate::fx_core::ShiftInt,
    p: crate::fx_core::ShiftInt,
    lp: crate::fx_core::Filter2,
    hp: crate::fx_core::Filter2,
    /// Gains in use: low-pass, dry, high-pass.
    g: [f32; 3],
    rate: u32,
}

impl NxsFilter {
    pub fn new() -> NxsFilter {
        let s = crate::fx_core::ShiftInt::new(128, 0.33333, 48000.0);
        let mut f = NxsFilter { c: s, p: s, lp: Default::default(), hp: Default::default(), g: [0.0, 1.0, 0.0], rate: 0 };
        f.coef();
        f
    }

    /// The effect was selected (`onCalledMakeSelected`): the knobs and the blend are taken as
    /// they are, without a glide.
    pub fn reset(&mut self, c: i32, p: i32) {
        self.c.reset(c);
        self.p.reset(p);
        let (l, d, h) = filter_mix(c);
        self.g = [l, d, h];
        self.lp.clear();
        self.hp.clear();
        self.coef();
    }

    fn coef(&mut self) {
        let rate = if self.rate == 0 { 48000.0 } else { self.rate as f64 };
        let (lfc, hfc, d, g) = filter_coef(self.c.cur, self.p.cur);
        self.hp.c = crate::fx_core::make_hpf2(hfc as f64, d as f64, g as f64, rate);
        self.lp.c = crate::fx_core::make_lpf2(lfc as f64, d as f64, g as f64, rate);
    }

    /// The dry sound passes untouched (the knob rests in the centre and the gains are there).
    pub fn dry(&self) -> bool {
        !self.c.moving() && (126..131).contains(&self.c.cur) && self.g == [0.0, 1.0, 0.0]
    }

    pub fn process(&mut self, buf: &mut [f32], c: i32, p: i32, rate: u32) {
        if rate != self.rate {
            self.rate = rate;
            self.c.set_rate(rate as f32);
            self.p.set_rate(rate as f32);
            self.lp.clear();
            self.hp.clear();
            self.coef();
        }
        self.c.set(c);
        self.p.set(p);
        let step = 100.0 / rate as f32;
        let frames = buf.len() / 2;
        let mut n = 0;
        while n < frames {
            let mut len = frames - n;
            if self.c.moving() {
                len = len.min(self.c.count.max(1) as usize);
            }
            if self.p.moving() {
                len = len.min(self.p.count.max(1) as usize);
            }
            let (cv, pv) = (self.c.cur, self.p.cur);
            let (tl, td, th) = filter_mix(cv);
            let ramp = |g: &mut f32, t: f32| {
                if *g > t {
                    *g = if *g - step < t { t } else { *g - step };
                } else if *g < t {
                    *g = if *g + step > t { t } else { *g + step };
                }
                *g
            };
            for f in buf[n * 2..(n + len) * 2].chunks_exact_mut(2) {
                let gd = ramp(&mut self.g[1], td);
                let gl = ramp(&mut self.g[0], tl) as f64;
                let gh = ramp(&mut self.g[2], th) as f64;
                for (ch, v) in f.iter_mut().enumerate() {
                    let x = *v as f64;
                    let l = self.lp.run(ch, x) * gl;
                    let h = self.hp.run(ch, x) * gh;
                    *v = ((*v * gd) as f64 + (h + l)) as f32;
                }
            }
            self.c.advance(len as i32);
            self.p.advance(len as i32);
            if self.c.cur != cv || self.p.cur != pv {
                self.coef();
            }
            n += len;
        }
    }
}

/// rekordbox's cutoff tables of the CFX-style filters (`ColorFxFilterManipulandumCutoffFreqConversionTable`:
/// 124 steps each of the low-pass, 12 kHz down to 93 Hz, and the high-pass, 61 Hz up to
/// 8.95 kHz). The default uses interpolation. An explicit RECORDBOX_EFFECT_TABLE_SOURCE
/// enables a background lookup in a user-supplied compatible local reference.
pub struct FcTables {
    pub lpf: [f32; 124],
    pub hpf: [f32; 124],
    /// Beat FX FILTER's cutoff tables (`LFOFilterCore::LPF_MAIN_FC`, `LPF_RESO_FC`,
    /// `HPF_MAIN_FC`, `HPF_RESO_FC`: 129 doubles each, one after another), when found.
    pub lfo: Option<Box<[[f64; 129]; 4]>>,
    /// Multi-mode REVERB's comb gains (`ReverbEffectCombFilterGainTable::TABLE`: ten tables of
    /// 100 floats, one per percent step; tables 0 .. 4 the five combs' left lines, 5 .. 9 their
    /// right lines), when found.
    pub verb: Option<Box<[[f32; 100]; 10]>>,
}

static FC_TABLES: std::sync::OnceLock<Option<FcTables>> = std::sync::OnceLock::new();
static FC_WARM: std::sync::Once = std::sync::Once::new();

/// Start reading the tables on a thread of their own (called where the effects are made, never
/// on the audio thread).
pub fn warm_tables() {
    FC_WARM.call_once(|| {
        if std::env::var_os("RECORDBOX_EFFECT_TABLE_SOURCE").is_none() {
            return;
        }
        let _ = std::thread::Builder::new().name("cfx-tables".into()).spawn(|| {
            FC_TABLES.get_or_init(read_fc_tables);
        });
    });
}

/// The tables if they have been read (never waits: the audio thread asks).
pub fn fc_tables() -> Option<&'static FcTables> {
    FC_TABLES.get().and_then(|t| t.as_ref())
}

/// Read the optional reference synchronously in a component test.
#[cfg(test)]
pub fn fc_tables_now() -> Option<&'static FcTables> {
    FC_TABLES.get_or_init(read_fc_tables).as_ref()
}

/// Find the tables in the `__TEXT,__const` section of the explicitly supplied binary's arm64 slice: the
/// three 124-float tables lie one after another (CRUSH's high-pass, 20, 21, 22 ... Hz, then the
/// high-pass and the low-pass), so the start of CRUSH's is looked for and the other two are
/// checked (first and last values, monotone) before they are used.
fn read_fc_tables() -> Option<FcTables> {
    let path = std::env::var_os("RECORDBOX_EFFECT_TABLE_SOURCE")?;
    read_reference(std::path::Path::new(&path))
}

fn read_reference(path: &std::path::Path) -> Option<FcTables> {
    use std::io::{Read, Seek, SeekFrom};
    if !std::fs::metadata(path).ok()?.is_file() { return None; }
    let mut f = std::fs::File::open(path).ok()?;
    let file_len = f.metadata().ok()?.len();
    let mut head = [0u8; 8];
    f.read_exact(&mut head).ok()?;
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let (base, slice_len) = if be(&head[0..4]) == 0xCAFE_BABE {
        let n = usize::try_from(be(&head[4..8])).ok()?;
        if n > 32 { return None; }
        let mut arch = vec![0u8; 20 * n];
        f.read_exact(&mut arch).ok()?;
        let arm = arch.chunks_exact(20).find(|a| be(a) == 0x0100_000C)?;
        let base = u64::from(be(&arm[8..]));
        let len = u64::from(be(&arm[12..]));
        if base.checked_add(len)? > file_len { return None; }
        (base, len)
    } else {
        (0, file_len)
    };
    f.seek(SeekFrom::Start(base)).ok()?;
    let mut mh = [0u8; 32];
    f.read_exact(&mut mh).ok()?;
    if le(&mh) != 0xFEED_FACF || le(&mh[4..]) != 0x0100_000C {
        return None;
    }
    let ncmds = le(&mh[16..]) as usize;
    let size = le(&mh[20..]) as usize;
    if size > 1 << 20 || size as u64 + 32 > slice_len || ncmds > size / 8 { return None; }
    let mut cmds = vec![0u8; size];
    f.read_exact(&mut cmds).ok()?;
    let mut remaining = cmds.as_slice();
    let mut sect = None;
    for _ in 0..ncmds {
        let h = remaining.get(..8)?;
        let len = le(&h[4..]) as usize;
        if len < 8 { return None; }
        let command = remaining.get(..len)?;
        remaining = &remaining[len..];
        if le(h) != 0x19 || command.get(8..15) != Some(b"__TEXT\0") {
            continue;
        }
        let nsects = le(command.get(64..68)?) as usize;
        let bytes = nsects.checked_mul(80)?.checked_add(72)?;
        for section in command.get(72..bytes)?.chunks_exact(80) {
            if section.starts_with(b"__const\0") {
                let sz = u64::from_le_bytes(section[40..48].try_into().ok()?);
                sect = Some((u64::from(le(&section[48..])), sz));
            }
        }
    }
    let (soff, ssz) = sect?;
    if ssz > 64 << 20 || soff.checked_add(ssz)? > slice_len { return None; }
    let mut data = vec![0u8; ssz as usize];
    f.seek(SeekFrom::Start(base + soff)).ok()?;
    f.read_exact(&mut data).ok()?;
    let key: Vec<u8> = [20.0f32, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let fl = |at: usize| f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    let mut at = 0;
    while at + key.len() <= data.len() {
        if data[at..at + key.len()] == key[..] && at + 3 * 496 <= data.len() {
            let hpf: [f32; 124] = std::array::from_fn(|i| fl(at + 496 + 4 * i));
            let lpf: [f32; 124] = std::array::from_fn(|i| fl(at + 992 + 4 * i));
            let ok = hpf[0] == 61.0 && (hpf[123] - 8951.0).abs() < 1.0 && lpf[0] == 12000.0 && (lpf[123] - 93.0).abs() < 1.0 && hpf.windows(2).all(|w| w[0] < w[1]) && lpf.windows(2).all(|w| w[0] > w[1]);
            if ok {
                return Some(FcTables { lpf, hpf, lfo: find_lfo_tables(&data), verb: find_verb_tables(&data) });
            }
        }
        at += 4;
    }
    None
}

/// Beat FX FILTER's four tables in the same section: LPF_MAIN falling from about 23.7 kHz to
/// under 1 kHz, LPF_RESO falling, HPF_MAIN rising from about 52 Hz, HPF_RESO four times it.
fn find_lfo_tables(data: &[u8]) -> Option<Box<[[f64; 129]; 4]>> {
    let d = |at: usize| f64::from_le_bytes(data[at..at + 8].try_into().unwrap());
    let size = 129 * 8;
    let mut at = 0;
    while at + 4 * size <= data.len() {
        let v = d(at);
        if v > 23000.0 && v < 24500.0 {
            let t: [[f64; 129]; 4] = std::array::from_fn(|k| std::array::from_fn(|i| d(at + k * size + 8 * i)));
            let ok = t[0].windows(2).all(|w| w[0] > w[1]) && t[0][128] < 1000.0 && t[1].windows(2).all(|w| w[0] > w[1]) && t[1][0] > 10000.0 && t[2].windows(2).all(|w| w[0] < w[1]) && t[2][0] > 40.0 && t[2][0] < 70.0 && (0..129).all(|i| (t[3][i] / t[2][i] - 4.0).abs() < 1e-3);
            if ok {
                return Some(Box::new(t));
            }
        }
        at += 8;
    }
    None
}

/// REVERB's comb-gain tables in the same section: each entry is 10^(-3 x delay / decay time),
/// so every table rises from about 0.87 .. 0.93 toward 1 and keeps within 0.01 of
/// `fx_verb::dyn_params`' fitted gains (which pick the place).
fn find_verb_tables(data: &[u8]) -> Option<Box<[[f32; 100]; 10]>> {
    let fl = |at: usize| f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    let fit = |k: usize, p: i32| crate::fx_verb::fitted_gain(if k < 5 { 2 * k } else { 2 * (k - 5) + 1 }, p);
    let first = fit(0, 1);
    let mut at = 0;
    while at + 4000 <= data.len() {
        let v = fl(at);
        if (v - first).abs() < 0.01 {
            let t: [[f32; 100]; 10] = std::array::from_fn(|k| std::array::from_fn(|i| fl(at + 400 * k + 4 * i)));
            let ok = (0..10).all(|k| t[k].windows(2).all(|w| w[0] <= w[1]) && (0..100).all(|i| t[k][i] < 1.0 && (t[k][i] - fit(k, i as i32 + 1)).abs() < 0.01));
            if ok {
                return Some(Box::new(t));
            }
        }
        at += 4;
    }
    None
}

/// `ColorFxFilterBase::FcTableReadingHelper::getFcFromTable` 0x1029d1ba8: the table step of an
/// 8-bit knob value.
pub fn fc_index(a: i32) -> usize {
    if a < 4 || a >= 256 { 0 } else if a <= 247 { (a >> 1) as usize } else { 123 }
}

/// The cutoff of step `i` of the CFX-style low-pass (`hp` false) or high-pass: the table read
/// from rekordbox, or (without it) a line through every 15th step.
pub fn inst_step_fc(hp: bool, i: usize) -> f32 {
    let i = i.min(123);
    match fc_tables() {
        Some(t) => if hp { t.hpf[i] } else { t.lpf[i] },
        None => inst_fc(hp, i as f32 / 123.0),
    }
}

/// CRUSH's high-pass table (`ColorFxCrushFilterCutoffTable`, 124 steps, 20 Hz .. 4129 Hz): every
/// step is round(20 x (4129 / 20)^(i / 123)) (checked against all 124 entries).
pub fn crush_fc(i: usize) -> f32 {
    (20.0f64 * (4129.0f64 / 20.0).powf(i.min(123) as f64 / 123.0)).round() as f32
}

/// Which `ColorFxFilterBase` filter an `InstFilter` is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InstKind {
    /// `InstFxLPF` / `InstFxHPF` (PAD FX, MERGE FX, the echoes' FILTER knob).
    Lpf,
    Hpf,
    /// `ColorFxCrush::IfxHPF` 0x1029aab14.
    CrushHpf,
}

/// `updateParameter` of the three filters for the knob `a` and the resonance knob `p` (0..255):
/// (dry, wet, cutoff, resonance gain).
pub fn inst_params(kind: InstKind, a: i32, p: i32) -> (f32, f32, f32, f32) {
    let (a, p) = (a.clamp(0, 255), p.clamp(0, 255));
    if kind == InstKind::CrushHpf {
        let fc = crush_fc(fc_index(a));
        if a < 4 {
            return (1.0, 0.0, fc, 0.0);
        }
        let v = if a >= 130 { 0.0 } else { (a as f32 * -0.0079365 + 1.0).max(0.0) };
        let d = v * v;
        let u = (a - 3) as f32 * 0.0039683;
        let v9 = (1.0 - u) - u * (1.0 - u);
        let v2 = v9 * v9;
        let v4 = v2 * v2;
        return (d, 1.0 - d, fc, 1.2998 - v4 * v4 * 1.2998);
    }
    let hp = kind == InstKind::Hpf;
    if a < 4 {
        return (1.0, 0.0, if hp { 61.0 } else { 12000.0 }, 1.44);
    }
    let (dry, wet) = if a > 37 { (0.0, 1.0) } else { let w = ((a - 4) as f32 * 0.029412).min(1.0); (1.0 - w, w) };
    let fc = inst_step_fc(hp, fc_index(a));
    let u = if hp { (a - 3) as f32 * 0.0039683 } else { (255 - a) as f32 * 0.0039683 };
    let v7 = (1.0 - u) - u * (1.0 - u);
    let k = (255 - p) as f32 * 0.0039216;
    let v8 = k * k * ((255 - p) as f32 * if hp { -0.0050974 } else { -0.0056471 }) + 1.44;
    let v2 = v7 * v7;
    let v4 = v2 * v2;
    (dry, wet, fc, v8 - v4 * v4 * v8)
}

/// One of rekordbox's CFX-style filters whole: `ColorFxFilterBase` (both knobs glide as 8-bit
/// values, a step every 0.66667 ms, and the block is cut at each step), its `updateParameter`
/// and `FilterCore::handleFilterProcessing` 0x1029d47ac: a second-order section (Q 0.7071,
/// `IirFilter`, pre-warped) fed with the input plus the resonance gain times its own output
/// through a first-order filter of the other kind at the same cutoff. Dry and wet move over
/// 3.3333 ms (`SmoothFloat`).
#[derive(Clone, Copy)]
pub struct InstFilter {
    pub kind: InstKind,
    a: crate::fx_core::ShiftInt,
    p: crate::fx_core::ShiftInt,
    /// x1, x2, y1, y2, the resonance path's state, per channel.
    h: [[f32; 5]; 2],
    dry: crate::fx_core::SmoothF,
    wet: crate::fx_core::SmoothF,
    rate: u32,
    /// The settings the coefficients were made for, the coefficients and the resonance gain.
    made: (i32, i32, u32),
    m: [f32; 5],
    o: [f32; 3],
    rg: f32,
}

impl InstFilter {
    pub fn new(kind: InstKind) -> InstFilter {
        let s = crate::fx_core::ShiftInt::new(0, 0.66667, 48000.0);
        InstFilter { kind, a: s, p: s, h: [[0.0; 5]; 2], dry: crate::fx_core::SmoothF::new(1.0, 3.3333, 48000.0), wet: crate::fx_core::SmoothF::new(0.0, 3.3333, 48000.0), rate: 0, made: (-1, -1, 0), m: [0.0; 5], o: [0.0; 3], rg: 0.0 }
    }

    pub fn reset(&mut self, a: i32, p: i32) {
        self.a.reset(a);
        self.p.reset(p);
        self.h = [[0.0; 5]; 2];
        let (d, w, ..) = inst_params(self.kind, a, p);
        self.dry.reset(d);
        self.wet.reset(w);
        self.made = (-1, -1, 0);
    }

    /// Nothing of the filter is heard and nothing is moving.
    pub fn dry(&self) -> bool {
        !self.a.moving() && self.a.cur < 4 && self.dry.settled() && self.wet.settled()
    }

    /// Filter `buf` (stereo interleaved) in place for the knob `a` and resonance knob `p`
    /// (0..255): out = in x dry + filtered x wet.
    pub fn process(&mut self, buf: &mut [f32], a: i32, p: i32, rate: u32) {
        if rate != self.rate {
            self.rate = rate;
            for s in [&mut self.a, &mut self.p] {
                s.set_rate(rate as f32);
            }
            self.dry.set_rate(rate as f32);
            self.wet.set_rate(rate as f32);
        }
        self.a.set(a.clamp(0, 255));
        self.p.set(p.clamp(0, 255));
        let r = rate as f64;
        let frames = buf.len() / 2;
        let mut n = 0;
        while n < frames {
            let mut len = frames - n;
            if self.a.moving() {
                len = len.min(self.a.count.max(1) as usize);
            }
            if self.p.moving() {
                len = len.min(self.p.count.max(1) as usize);
            }
            if self.made != (self.a.cur, self.p.cur, rate) {
                self.made = (self.a.cur, self.p.cur, rate);
                let (d, w, fc, rg) = inst_params(self.kind, self.a.cur, self.p.cur);
                self.dry.set(d);
                self.wet.set(w);
                let lp = self.kind == InstKind::Lpf;
                self.m = if lp { crate::fx_core::iir_lpf2(fc as f64, 0.707106781, r) } else { crate::fx_core::iir_hpf2(fc as f64, 0.707106781, r) };
                self.o = crate::fx_core::iir_1st(fc as f64, r, lp);
                self.rg = rg;
            }
            let (m, o, rg) = (self.m, self.o, self.rg);
            for f in buf[n * 2..(n + len) * 2].chunks_exact_mut(2) {
                let (gd, gw) = (self.dry.next(), self.wet.next());
                for (ch, v) in f.iter_mut().enumerate() {
                    let h = &mut self.h[ch];
                    let x = *v + h[4] * rg;
                    let y = x * m[0] + h[0] * m[1] + h[1] * m[2] - (h[2] * m[3] + h[3] * m[4]);
                    h[1] = h[0];
                    h[0] = x;
                    let y1 = h[2];
                    h[3] = y1;
                    h[2] = y;
                    h[4] = y * o[0] - (h[4] * o[2] - y1 * o[1]);
                    *v = *v * gd + y * gw;
                }
            }
            self.a.advance(len as i32);
            self.p.advance(len as i32);
            n += len;
        }
    }
}

const ECHO_LEN: usize = 24000;

/// DUB ECHO's settings for COLOR `c` and PARAM `p` (8-bit), as `ColorFxDubEcho::
/// onCfxKnobValueChanged` 0x1029ad784 hands them to its two `DubEchoCore`s: (input level, delay
/// ms, feedback) for the low core (left of the centre) and the high core (right), `None` where
/// a core is not touched. In the centre both cores get no input, 55 ms and PARAM's feedback.
pub fn dub_set(c: i32, p: i32) -> [Option<(f32, f32, f32)>; 2] {
    if c <= 115 {
        let (_, ms, fb) = dub_law(c as f32, p as f32);
        let n = (116 - c) as f32;
        let inp = if n * 0.0086207 < 0.3 { n * 0.028736 } else { 1.0 };
        return [Some((inp, ms, fb)), None];
    }
    if c > 138 {
        let (_, ms, fb) = dub_law(c as f32, p as f32);
        let m = (c - 139) as f32;
        let inp = if m * 0.0086207 < 0.3 { m * 0.028736 } else { 1.0 };
        return [None, Some((inp, ms, fb))];
    }
    let (a, b, k) = if p <= 115 {
        (0.0075 * p as f32, 0.0075862 * p as f32, 1.0)
    } else if p < 140 {
        (0.87, 0.88, 1.0)
    } else {
        let q = (p - 139) as f32;
        (0.87 + 0.0010345 * q, 0.88 + 0.00094828 * q, (255 - p) as f32 * 0.00017241 + 0.98)
    };
    [Some((0.0, 55.0, k * a)), Some((0.0, 55.0, k * b))]
}

/// Frames of a DUB ECHO line: 500 ms (its longest delay) at up to 96 kHz, and a little.
const DUB_LEN: usize = 48_064;

/// rekordbox's `GainSmoother` (0x1028e0400 / setTargetGain 0x1028e065c): a straight line to the
/// target over round(ms x fs / 1000) frames.
#[derive(Clone, Copy)]
struct GainSm {
    cur: f32,
    target: f32,
    step: f32,
    left: i32,
}

impl GainSm {
    fn new(v: f32) -> GainSm {
        GainSm { cur: v, target: v, step: 0.0, left: 0 }
    }

    fn set(&mut self, t: f32, rate: f32) {
        self.set_ms(t, rate, 3.3);
    }

    fn set_ms(&mut self, t: f32, rate: f32, ms: f32) {
        if t != self.target {
            let n = ((rate as f64 * (0.001 * ms as f64)).round() as i32).max(1);
            self.left = n;
            self.step = (t - self.cur) / n as f32;
            self.target = t;
        }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.cur = if self.left == 0 { self.target } else { self.cur + self.step };
        }
        self.cur
    }
}

/// rekordbox's `TapeEchoCore` (0x1029b3ec4) on a `MultiModeDelayBuffer` in its cross-fade mode
/// (`CrossFadeDelayBuffer::getDelayDataInternal` 0x1028d1cb0): the delay in fractional frames,
/// read with a straight-line interpolation between the frames at d and d + 1 back; a new delay
/// is cross-faded in from the old one over the smoothing time (`GainSmoother`, rounded frames),
/// changes asked for meanwhile wait for the fade's end; the line takes the input plus feedback x
/// the repeats, and the output is the repeats.
struct TapeEcho {
    line: Vec<f32>,
    len: usize,
    w: usize,
    /// The delay in use (frames), the one asked for, the one being faded to, the fade.
    d: f32,
    want: f32,
    to: f32,
    xf: Option<GainSm>,
    pub fb: f32,
    /// Cross-fade time (ms).
    pub ms: f32,
}

impl TapeEcho {
    fn new(len: usize) -> TapeEcho {
        TapeEcho { line: vec![0.0; len * 2], len, w: 0, d: 0.0, want: 0.0, to: 0.0, xf: None, fb: 0.0, ms: 3.3 }
    }

    fn reset(&mut self) {
        self.line.iter_mut().for_each(|v| *v = 0.0);
        self.d = 0.0;
        self.xf = None;
    }

    /// Ask for a delay of `frames`.
    fn set_delay(&mut self, frames: f32) {
        self.want = frames.clamp(1.0, (self.len - 3) as f32);
        if self.d <= 0.0 {
            self.d = self.want;
        }
    }

    #[inline]
    fn read(&self, d: f32, ch: usize) -> f32 {
        let i = d as usize;
        let t = d - i as f32;
        let a = self.line[((self.w + self.len - i) % self.len) * 2 + ch];
        let b = self.line[((self.w + self.len - i - 1) % self.len) * 2 + ch];
        a + (b - a) * t
    }

    #[inline]
    fn run(&mut self, v: [f32; 2], rate: f32) -> [f32; 2] {
        if self.xf.is_none() && (self.want - self.d).abs() > 1.1920929e-7 {
            self.to = self.want;
            let mut f = GainSm::new(1.0);
            f.set_ms(0.0, rate, self.ms);
            self.xf = Some(f);
        }
        let mut y = [0.0f32; 2];
        if let Some(mut f) = self.xf {
            let k = f.next();
            for ch in 0..2 {
                y[ch] = self.read(self.d, ch) * k + self.read(self.to, ch) * (1.0 - k);
            }
            if f.left == 0 {
                self.d = self.to;
                self.xf = None;
            } else {
                self.xf = Some(f);
            }
        } else {
            for ch in 0..2 {
                y[ch] = self.read(self.d, ch);
            }
        }
        let fb = self.fb;
        self.line[self.w * 2] = (v[0] + y[0] * fb).clamp(-64.0, 64.0);
        self.line[self.w * 2 + 1] = (v[1] + y[1] * fb).clamp(-64.0, 64.0);
        self.w = (self.w + 1) % self.len;
        y
    }
}

/// One of DUB ECHO's two `DubEchoCore`s (`processWithOffset` 0x1029b2474): a pre filter
/// (`IirFilter`, second order, Q 0.7071), the input level (`GainSmoother`, 3.3 ms), the echo
/// (`TapeEcho`, 3.3 ms cross-fade) and a post filter.
struct DubCore {
    /// Pre and post filters: coefficients and, per channel, x1 x2 y1 y2.
    pre: Option<[f32; 5]>,
    post: Option<[f32; 5]>,
    pre_h: [[f32; 4]; 2],
    post_h: [[f32; 4]; 2],
    vol: GainSm,
    echo: TapeEcho,
}

impl DubCore {
    fn new() -> DubCore {
        DubCore { pre: None, post: None, pre_h: [[0.0; 4]; 2], post_h: [[0.0; 4]; 2], vol: GainSm::new(0.0), echo: TapeEcho::new(DUB_LEN) }
    }

    fn reset(&mut self) {
        self.echo.reset();
        self.pre_h = [[0.0; 4]; 2];
        self.post_h = [[0.0; 4]; 2];
        self.vol = GainSm::new(0.0);
    }

    fn set(&mut self, (inp, ms, fb): (f32, f32, f32), rate: f32) {
        self.vol.set(inp.clamp(0.0, 1.0), rate);
        self.echo.set_delay(ms.clamp(1.0, 500.0) * rate * 0.001);
        self.echo.fb = fb.clamp(0.0, 0.993);
    }

    #[inline]
    fn iir(c: &[f32; 5], h: &mut [f32; 4], x: f32) -> f32 {
        let y = ((h[0] * c[1] + h[1] * c[2]) - (h[2] * c[3] + h[3] * c[4])) + x * c[0];
        h[1] = h[0];
        h[0] = x;
        h[3] = h[2];
        h[2] = y;
        y
    }

    /// One frame: the core's output (the repeats, through the post filter).
    #[inline]
    fn run(&mut self, x: [f32; 2], rate: f32) -> [f32; 2] {
        let mut v = x;
        if let Some(c) = &self.pre {
            for ch in 0..2 {
                v[ch] = DubCore::iir(c, &mut self.pre_h[ch], v[ch]);
            }
        }
        let g = self.vol.next();
        v = [v[0] * g, v[1] * g];
        let mut y = self.echo.run(v, rate);
        if let Some(c) = &self.post {
            for ch in 0..2 {
                y[ch] = DubCore::iir(c, &mut self.post_h[ch], y[ch]);
            }
        }
        // The post gain (`GainSmoother` made at 0.7, applied after the post filter in
        // `processWithOffset` 0x1029b2474). It only moves when a filter type changes
        // (`updateParameter` 0x1029b1ccc fades it to 0, then `initFilter` and back to 1), and
        // `ColorFxDubEcho` sets the types in its constructor; `onCalledMakeSelected` syncs them
        // (`initFilter`) before the first check, so in use it stays at 0.7.
        [y[0] * DUB_POST_GAIN, y[1] * DUB_POST_GAIN]
    }
}

/// `DubEchoCore`'s post gain as it stays in use (see `DubCore::run`).
const DUB_POST_GAIN: f32 = 0.7;

/// CFX CRUSH for COLOR `c` and PARAM `p` (8-bit): (hold ms, float bit mask, high-pass Hz).
/// PARAM sets the longest hold (0.104 .. 0.5 ms left of its centre, 0.5 .. 0.75 above, never
/// over 0.5 in use); COLOR shortens it toward the centre from either side. Above 144 PARAM also
/// keeps fewer mantissa bits.
pub fn crush_law(c: i32, p: i32) -> (f32, u32, f32) {
    let top = if p < 128 { 0.10417 + 0.0030924 * p as f32 } else { 0.5 + 0.0019531 * (p - 128) as f32 };
    let hold = if c <= 127 { top - c as f32 * 0.0078431 * top } else { top * (c as f32 * 0.0078431 - 1.0) }.clamp(0.0, 0.5);
    let mask = if hold <= 0.10417 || p <= 144 { u32::MAX } else { (0xFFFF_0000u32).wrapping_shl(((8 * p - 1024) >> 7) as u32) };
    let hp = if c > 127 { 20.0 * (4129.0f32 / 20.0).powf(((c - 128) as f32 / 127.0).clamp(0.0, 1.0)) } else { 20.0 };
    (hold, mask, hp)
}

/// CFX NOISE's noise level for COLOR in 8-bit steps (`ColorFxNoise::calcColorGain`).
pub fn noise_color_gain(c: i32) -> f32 {
    if c <= 123 {
        let w = if c >= 41 { let v = (c - 40) as f32 * 0.0014; let a = (1.0 - v) * (1.0 - v); a * a } else { 1.0 };
        return w * (1.3913 - 0.01122 * c as f32);
    }
    if c < 132 {
        0.0
    } else if c <= 156 {
        0.8 - 0.00128 * ((157 - c) * (157 - c)) as f32
    } else if c <= 181 {
        0.53662 + 0.010535 * (182 - c) as f32
    } else {
        let u = (c - 180) as f32 * 0.004;
        let a = (1.0 - u) * (1.0 - u);
        a * a * a * a * (c - 131) as f32 * 0.01122
    }
}

/// CFX NOISE's band-pass (`updateFilterCoef`): (centre Hz, Q, its gain).
pub fn noise_filter(c: i32) -> (f32, f32, f32) {
    let x = |n: i32| { let t = (n as f32 * 0.0081301).powi(2); t.powi(8) * 0.1 + 0.7 };
    let (q, g, v) = if c <= 123 {
        (if c >= 117 { 2.0 * 2f32.powf(-(c - 116) as f32 * 0.090909) } else { 2.0 }, x(123 - c), c)
    } else if c >= 132 {
        (if c >= 245 { 2.0 * 2f32.powf(-(c - 244) as f32 * 0.090909) } else { 2.0 }, x(c - 132), c - 128)
    } else {
        (2.0, 0.7, c.max(128) - 128)
    };
    let a = 2f32.powf(v as f32 * 0.0569192) * 100.0;
    let fc = ((a * (-0.5 - q) / (0.5 - q) - a) * q).max(5.0);
    (fc, q, g.min(1.0))
}

/// CFX PITCH's law for COLOR `c` and PARAM `p` in 8-bit steps: the shifter's setting (ratio -
/// 1), or `None` in the centre where the sound stays dry.
pub fn pitch_law(c: i32, p: i32) -> Option<f32> {
    let semis = if p <= 126 { 2.0 + 0.07874 * p as f32 } else if p < 129 { 12.0 } else { 12.0 + 0.094488 * (p - 128) as f32 };
    if (124..=131).contains(&c) {
        // (rekordbox mixes it dry from 124 to 131; the shift is 0 up to 130.)
        return None;
    }
    if c < 124 {
        Some((c as f32 * 0.0040323 - 0.5) * (2.0 - 2.0 * 2f32.powf(-semis / 12.0)))
    } else {
        Some((2f32.powf(semis / 12.0) - 1.0) * 0.0080645 * (c - 131) as f32)
    }
}

/// One channel's Sound Color FX.
pub struct Cfx {
    kind: u8,
    /// COLOR in use, moving toward the knob.
    color: f32,
    lp: Bq,
    hp: Bq,
    band: Bq,
    seed: u32,
    hold: [f32; 2],
    hold_n: f32,
    delay: Vec<f32>,
    w: usize,
    fb_lp: [f32; 2],
    made: (f32, f32, u32),
    /// PITCH: rekordbox's pitch shifter (`fx_core`) and the rate it is set up for.
    ps: crate::fx_core::PitchShifter,
    ps_rate: u32,
    /// SPACE and GATE COMP (`cfx_space`), made with the rest so nothing is allocated later.
    space: crate::cfx_space::Space,
    gc: crate::cfx_space::GateComp,
    /// FILTER (`ColorFxFilterDjm900NXS2`).
    flt: NxsFilter,
    /// LPF / HPF (`InstFxLPF` / `InstFxHPF`) and CRUSH's high-pass (`ColorFxCrush::IfxHPF`).
    inst: InstFilter,
    /// CRUSH's COLOR and PARAM (8-bit, a step every 0.33333 ms).
    sc: crate::fx_core::ShiftInt,
    sp: crate::fx_core::ShiftInt,
    /// DUB ECHO's low and high cores, the peak of the last buffer's echoes.
    dub: [DubCore; 2],
    dub_peak: f32,
    /// JET's echo (`TapeEchoCore`) and its dry and wet gains.
    jet: TapeEcho,
    jet_g: [crate::fx_core::SmoothF; 2],
    sweep: Sweep,
}


impl Cfx {
    pub fn new() -> Cfx {
        warm_tables();
        Cfx { kind: 0, color: 0.5, lp: Bq::default(), hp: Bq::default(), band: Bq::default(), seed: 0x1234_5678, hold: [0.0; 2], hold_n: 0.0, delay: Vec::new(), w: 0, fb_lp: [0.0; 2], made: (-1.0, -1.0, 0), ps: crate::fx_core::PitchShifter::new(48000.0), ps_rate: 48000, space: crate::cfx_space::Space::new(), gc: crate::cfx_space::GateComp::new(), flt: NxsFilter::new(), inst: InstFilter::new(InstKind::Lpf), sc: crate::fx_core::ShiftInt::new(128, 0.33333, 48000.0), sp: crate::fx_core::ShiftInt::new(128, 0.33333, 48000.0), dub: [DubCore::new(), DubCore::new()], dub_peak: 0.0, jet: TapeEcho::new(4096), jet_g: [crate::fx_core::SmoothF::new(1.0, 3.3333, 48000.0), crate::fx_core::SmoothF::new(0.0, 3.3333, 48000.0)], sweep: Sweep::new() }
    }

    fn reset(&mut self) {
        self.lp.z = [[0.0; 2]; 2];
        self.hp.z = [[0.0; 2]; 2];
        self.band.z = [[0.0; 2]; 2];
        self.delay.iter_mut().for_each(|v| *v = 0.0);
        self.fb_lp = [0.0; 2];
        self.hold = [0.0; 2];
        self.made = (-1.0, -1.0, 0);
        self.ps.reset();
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// CRUSH (`ColorFxCrush::processEffectConcrete` 0x1029aa418): COLOR and PARAM glide as
    /// 8-bit values (a step every 0.33333 ms, the block cut at each step). The sound alone, its
    /// floats' mantissas cut once PARAM is above 144 and the hold over 0.104 ms (the mask from
    /// PARAM's new value and the hold in use, once a buffer), its samples held (`CrushCore`)
    /// for PARAM's top hold (0.104 .. 0.5 ms left of its centre, 0.5 .. 0.75 above, never over
    /// 0.5) shortened toward COLOR's centre, then right of the centre `ColorFxCrush::IfxHPF`.
    fn crush(&mut self, buf: &mut [f32], c: i32, p: i32, rate: u32) {
        let centred = (124..=131).contains(&c);
        if centred && !self.sc.moving() && (124..=131).contains(&self.sc.cur) && self.inst.dry() {
            return;
        }
        let r = rate as f32;
        self.sc.set_rate(r);
        self.sp.set_rate(r);
        self.sc.set(c);
        self.sp.set(p);
        let (held, _, _) = crush_law(self.sc.cur, self.sp.cur);
        let mask = if held <= 0.10417 || p <= 144 { u32::MAX } else { (0xFFFF_0000u32).wrapping_shl(((8 * p - 1024) >> 7) as u32) };
        let frames = buf.len() / 2;
        let mut n = 0;
        while n < frames {
            let mut len = frames - n;
            if self.sc.moving() {
                len = len.min(self.sc.count.max(1) as usize);
            }
            if self.sp.moving() {
                len = len.min(self.sp.count.max(1) as usize);
            }
            let (hold_ms, _, _) = crush_law(self.sc.cur, self.sp.cur);
            let hold = (hold_ms * 0.001 * r).max(1.0);
            for f in buf[n * 2..(n + len) * 2].chunks_exact_mut(2) {
                let x = [f32::from_bits(f[0].to_bits() & mask), f32::from_bits(f[1].to_bits() & mask)];
                self.hold_n += 1.0;
                if self.hold_n >= hold {
                    self.hold_n -= hold;
                    self.hold = x;
                }
                f[0] = self.hold[0];
                f[1] = self.hold[1];
            }
            let a = if self.sc.cur > 127 { ((self.sc.cur as f32 * 0.0078431 - 1.0) * 255.0) as i32 } else { 0 };
            self.inst.process(&mut buf[n * 2..(n + len) * 2], a, 0, rate);
            self.sc.advance(len as i32);
            self.sp.advance(len as i32);
            n += len;
        }
    }

    /// JET (`ColorFxJetRZX::processEffectConcrete` 0x102a0095c): COLOR glides as an 8-bit value
    /// (a step every 0.33333 ms, the block cut at each step); for each piece
    /// `updateAndSetParameter` 0x102a00a68 gives a `TapeEchoCore` its delay (2.67 ms growing to
    /// 20 ms to the left, shrinking to 0.167 ms to the right), feedback (scaled by PARAM) and
    /// cross-fade time (2.9167 ms left of the centre, 0.29167 ms right), and the dry / wet blend
    /// (3.3333 ms). The echo runs all the time; in the centre the sound is dry exactly.
    fn jet(&mut self, buf: &mut [f32], c: i32, p: i32, rate: u32) {
        let r = rate as f32;
        self.sc.set_rate(r);
        self.sc.set(c);
        self.jet_g.iter_mut().for_each(|g| g.set_rate(r));
        let frames = buf.len() / 2;
        let mut n = 0;
        while n < frames {
            let mut len = frames - n;
            if self.sc.moving() {
                len = len.min(self.sc.count.max(1) as usize);
            }
            let cv = self.sc.cur;
            let (d, w, ms, fb) = jet_law(cv as f32, p as f32);
            if cv < 124 {
                self.jet.ms = 2.9167;
            } else if cv > 131 {
                self.jet.ms = 0.29167;
            }
            self.jet.set_delay(ms * 0.001 * r);
            self.jet.fb = fb.clamp(0.0, 0.993);
            self.jet_g[0].set(d);
            self.jet_g[1].set(w);
            for f in buf[n * 2..(n + len) * 2].chunks_exact_mut(2) {
                let y = self.jet.run([f[0], f[1]], r);
                let (gd, gw) = (self.jet_g[0].next(), self.jet_g[1].next());
                f[0] = f[0] * gd + y[0] * gw;
                f[1] = f[1] * gd + y[1] * gw;
            }
            self.sc.advance(len as i32);
            n += len;
        }
    }

    /// DUB ECHO (`ColorFxDubEcho::processEffectConcrete` 0x1029add0c): COLOR and PARAM glide as
    /// 8-bit values (a step every 0.66667 ms; the block is cut at each step and the cores get
    /// their settings after it), both cores run on the sound, and the output is the sound plus
    /// the high core plus 0.7 x the low core. The tails ring on in the centre; once they are
    /// silent the sound passes untouched.
    fn dub_echo(&mut self, buf: &mut [f32], color: f32, param: f32, rate: u32) {
        let (c, p) = ((color.clamp(0.0, 1.0) * 255.0) as i32, (param.clamp(0.0, 1.0) * 255.0) as i32);
        let r = rate as f32;
        let centred = (116..=138).contains(&c);
        if centred && !self.sc.moving() && (116..=138).contains(&self.sc.cur) && self.dub_peak < 1e-7 && self.dub.iter().all(|d| d.vol.cur == 0.0 && d.vol.left == 0) {
            return;
        }
        self.sc.set_rate(r);
        self.sp.set_rate(r);
        self.sc.set(c);
        self.sp.set(p);
        let frames = buf.len() / 2;
        let mut peak = 0.0f32;
        let mut n = 0;
        while n < frames {
            let mut len = frames - n;
            if self.sc.moving() {
                len = len.min(self.sc.count.max(1) as usize);
            }
            if self.sp.moving() {
                len = len.min(self.sp.count.max(1) as usize);
            }
            for f in buf[n * 2..(n + len) * 2].chunks_exact_mut(2) {
                let x = [f[0], f[1]];
                let a = self.dub[0].run(x, r);
                let b = self.dub[1].run(x, r);
                let (wl, wr) = (b[0] + a[0] * 0.7, b[1] + a[1] * 0.7);
                peak = peak.max(wl.abs()).max(wr.abs());
                f[0] = x[0] + wl;
                f[1] = x[1] + wr;
            }
            self.sc.advance(len as i32);
            self.sp.advance(len as i32);
            for (k, set) in dub_set(self.sc.cur, self.sp.cur).into_iter().enumerate() {
                if let Some(v) = set {
                    self.dub[k].set(v, r);
                }
            }
            n += len;
        }
        self.dub_peak = peak;
    }

    /// Process `buf` (stereo interleaved) in place. `color` and `param`: the knobs, 0..1.
    pub fn process(&mut self, kind: u8, color: f32, param: f32, buf: &mut [f32], rate: u32) {
        if rate == 0 || buf.len() < 2 { return; }
        let ci = (color.clamp(0.0, 1.0) * 255.0).round() as i32;
        let pi = (param.clamp(0.0, 1.0) * 255.0).round() as i32;
        if kind != self.kind {
            self.kind = kind;
            self.reset();
            if kind == 5 { self.space.reset(ci); }
            if kind == 8 { self.gc.reset(ci); }
            self.flt.reset(ci, pi);
            self.inst = InstFilter::new(match kind { HPF => InstKind::Hpf, 2 => InstKind::CrushHpf, _ => InstKind::Lpf });
            // (CRUSH's knobs step every 0.33333 ms, DUB ECHO's every 0.66667 ms, both
            // truncated by DUB ECHO.)
            let ms = if kind == 6 { 0.66667 } else { 0.33333 };
            self.sc = crate::fx_core::ShiftInt::new(ci, ms, rate as f32);
            self.sp = crate::fx_core::ShiftInt::new(pi, ms, rate as f32);
            if kind == 7 {
                self.sweep.reset((color.clamp(0.0, 1.0) * 255.0) as i32, (param.clamp(0.0, 1.0) * 255.0) as i32, rate);
            }
            if kind == 1 {
                self.jet.reset();
                self.jet.ms = 0.29167;
                let (d, w, ms, fb) = jet_law(ci as f32, pi as f32);
                self.jet_g[0].reset(d);
                self.jet_g[1].reset(w);
                self.jet.set_delay(ms * 0.001 * rate as f32);
                self.jet.fb = fb;
            }
            if kind == 6 {
                let (c8, p8) = ((color.clamp(0.0, 1.0) * 255.0) as i32, (param.clamp(0.0, 1.0) * 255.0) as i32);
                self.sc.reset(c8);
                self.sp.reset(p8);
                for d in self.dub.iter_mut() {
                    d.reset();
                }
                let r = rate as f32;
                // Low core: band-pass at 1375 Hz before the echo, nothing after; high core:
                // high-pass at 2500 Hz before, low-pass at 12 kHz after (`ColorFxDubEcho` ctor).
                let fc = |f: f32| f.min(r * 0.5).max(5.0) as f64;
                self.dub[0].pre = Some(crate::fx_core::iir_bpf2(fc(1375.0), 0.707106781, r as f64));
                self.dub[0].post = None;
                self.dub[1].pre = Some(crate::fx_core::iir_hpf2(fc(2500.0), 0.707106781, r as f64));
                self.dub[1].post = Some(crate::fx_core::iir_lpf2(fc(12000.0), 0.707106781, r as f64));
                for (k, set) in dub_set(c8, p8).into_iter().enumerate() {
                    if let Some(v) = set {
                        self.dub[k].set(v, r);
                    }
                }
                self.dub_peak = 0.0;
            }
        }
        if kind == 7 {
            self.sweep.process(buf, (color.clamp(0.0, 1.0) * 255.0) as i32, (param.clamp(0.0, 1.0) * 255.0) as i32, rate);
            self.color = color;
            return;
        }
        if kind == 1 {
            self.jet(buf, ci, pi, rate);
            self.color = color;
            return;
        }
        if kind == 6 {
            self.dub_echo(buf, color, param, rate);
            self.color = color;
            return;
        }
        if kind == 2 {
            self.crush(buf, ci, pi, rate);
            self.color = color;
            return;
        }
        if kind == LPF || kind == HPF {
            // PAD FX's LPF / HPF: the knob's distance from the centre is how far the filter
            // is turned (8-bit, truncated as rekordbox's `EightBitsParameter`).
            let a = (((color.clamp(0.0, 1.0) - 0.5).abs() * 2.0).min(1.0) * 255.0) as i32;
            if a < 4 && self.inst.dry() {
                return;
            }
            self.inst.process(buf, a, pi, rate);
            return;
        }
        if kind == 0 {
            // FILTER runs all the time (its filters keep their state); in the centre with
            // the blend settled it gives the dry sound exactly.
            self.color = color;
            self.flt.process(buf, ci, pi, rate);
            return;
        }
        if kind == 5 || kind == 8 {
            // SPACE / GATE COMP glide COLOR themselves (0.66667 ms a step). At the centre with
            // nothing left ringing the sound passes untouched.
            let now = if kind == 5 { self.space.color() } else { self.gc.color() };
            if (124..=131).contains(&ci) && now == ci && (if kind == 5 { !self.space.ringing() } else { self.gc.dry() }) {
                self.color = color;
                return;
            }
            self.color = color;
            if kind == 5 {
                self.space.process(buf, ci, pi, rate);
            } else {
                self.gc.process(buf, ci, pi, rate);
            }
            return;
        }
        // The effect is out of the path while the knob rests in the centre and nothing rings.
        let centred = (color - 0.5).abs() < 0.012;
        if centred && (self.color - 0.5).abs() < 0.012 {
            self.color = 0.5;
            return;
        }
        let r = rate as f32;
        let frames = buf.len() / 2;
        // The knob glides (rekordbox: 0.3333 ms per 8-bit step, 85 ms for the whole travel).
        let step = 1.0 / (0.085 * r);
        let mut n = 0;
        while n < frames {
            let end = (n + 32).min(frames);
            let target = if centred { 0.5 } else { color.clamp(0.0, 1.0) };
            let span = (end - n) as f32;
            self.color += (target - self.color).clamp(-step * span, step * span);
            let c = self.color;
            // How far from the centre, 0..1, and which side.
            let depth = ((c - 0.5).abs() * 2.0).min(1.0);
            let right = c > 0.5;
            let p = param.clamp(0.0, 1.0);
            match self.kind {
                3 => {
                    // NOISE (`ColorFxNoise`): white noise at COLOR's gain x PARAM (x 1.0593,
                    // `GAIN_5dB_UP_COEF`, set by its static initialiser) through a band-pass (Q 2) whose centre COLOR moves (left: 133 Hz
                    // at the end up to 13 kHz near the centre; right: 155 Hz up to 20 kHz),
                    // added to the sound.
                    let (ci, pi) = ((c * 255.0) as i32, (p * 255.0) as i32);
                    let (fc, q, fg) = noise_filter(ci);
                    let level = noise_color_gain(ci) * pi as f32 * 0.0039216 * 1.0592537 * fg;
                    let co = crate::fx_core::iir_bpf2(fc.min(0.45 * r) as f64, q as f64, r as f64);
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        let x = self.noise();
                        let y = crate::fx_core::bq(&co, &mut self.band.z[0], x) * level;
                        f[0] += y;
                        f[1] += y;
                    }
                }
                4 => {
                    // PITCH (`ColorFxPitch::processEffectConcrete`): the whole sound through
                    // rekordbox's shifter. PARAM sets the range (2 .. 12 semitones in its left
                    // half, 12 .. 24 in its right), COLOR moves the pitch down (left) or up
                    // (right) within it; the centre (124 .. 131) leaves the sound dry.
                    if rate != self.ps_rate {
                        self.ps_rate = rate;
                        self.ps.prepare(r);
                    }
                    let shift = pitch_law((c * 255.0) as i32, (p * 255.0) as i32);
                    if let Some(sh) = shift {
                        self.ps.set_shift(sh);
                    }
                    let wet = shift.is_some();
                    for i in n..end {
                        if self.ps.block_left() == 0 {
                            // (A block of up to 512 frames, as rekordbox's buffer: the
                            // shifter's crossing search sees it first.)
                            let len = (frames - i).min(crate::fx_core::PitchShifter::BLOCK);
                            let bb = &*buf;
                            self.ps.begin_block((i..i + len).map(|k| (bb[k * 2], bb[k * 2 + 1])));
                        }
                        let (l, rr) = self.ps.process(buf[i * 2], buf[i * 2 + 1]);
                        if wet {
                            buf[i * 2] = l;
                            buf[i * 2 + 1] = rr;
                        }
                    }
                }
                _ => {}
            }
            n = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(kind: u8, color: f32, param: f32, freq: f32) -> f32 {
        let mut fx = Cfx::new();
        let mut x: Vec<f32> = (0..48000).flat_map(|i| { let v = (i as f32 * freq * std::f32::consts::TAU / 48000.0).sin() * 0.5; [v, v] }).collect();
        fx.process(kind, color, param, &mut x, 48000);
        let tail = &x[48000..];
        20.0 * ((tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt() / (0.5 * std::f32::consts::FRAC_1_SQRT_2)).log10()
    }

    #[test]
    fn optional_effect_reference_rejects_incomplete_headers_and_oversize_sections() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_reference(dir.path()).is_none());
        let path = dir.path().join("reference");
        std::fs::write(&path, b"\xCA\xFE\xBA\xBE\xff\xff\xff\xff").unwrap();
        assert!(read_reference(&path).is_none());
        let mut thin = [0u8; 32 + 72 + 80];
        thin[..4].copy_from_slice(&0xFEED_FACFu32.to_le_bytes());
        thin[4..8].copy_from_slice(&0x0100_000Cu32.to_le_bytes());
        thin[16..20].copy_from_slice(&1u32.to_le_bytes());
        thin[20..24].copy_from_slice(&152u32.to_le_bytes());
        thin[32..36].copy_from_slice(&0x19u32.to_le_bytes());
        thin[36..40].copy_from_slice(&152u32.to_le_bytes());
        thin[40..47].copy_from_slice(b"__TEXT\0");
        thin[96..100].copy_from_slice(&1u32.to_le_bytes());
        thin[104..112].copy_from_slice(b"__const\0");
        thin[144..152].copy_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(&path, thin).unwrap();
        assert!(read_reference(&path).is_none());
    }

    #[test]
    fn traced_pitch_and_noise_laws() {
        // PITCH: PARAM's range (12 semitones at the centre), COLOR's ends an octave down / up.
        assert_eq!(pitch_law(127, 127), None);
        assert!((pitch_law(0, 127).unwrap() + 0.5).abs() < 1e-3);
        assert!((pitch_law(255, 127).unwrap() - 1.0).abs() < 1e-3);
        assert!((pitch_law(255, 255).unwrap() - 3.0).abs() < 0.01);
        assert!((pitch_law(0, 0).unwrap() + (1.0 - 2f32.powf(-2.0 / 12.0))).abs() < 1e-3);
        // CRUSH: the hold (at most 0.5 ms) and the bit mask.
        let (h, m, _) = crush_law(0, 0);
        assert!((h - 0.10417).abs() < 1e-5 && m == u32::MAX);
        let (h, m, hp) = crush_law(255, 255);
        assert!((h - 0.5).abs() < 1e-5 && m == 0xFF80_0000 && (hp - 4129.0).abs() < 1.0);
        // NOISE: its level and band.
        assert!((noise_color_gain(0) - 1.3913).abs() < 1e-4 && noise_color_gain(128) == 0.0);
        assert!((noise_color_gain(157) - 0.8).abs() < 1e-4);
        let (fc, q, _) = noise_filter(0);
        assert!((fc - 133.33).abs() < 0.1 && q == 2.0);
    }

    #[test]
    fn filter_law_matches_rekordbox() {
        // Blend zones (`processEffectConcrete`).
        assert_eq!(filter_mix(60), (1.0, 0.0, 0.0));
        assert_eq!(filter_mix(118), (0.5, 0.5, 0.0));
        assert_eq!(filter_mix(128), (0.0, 1.0, 0.0));
        assert_eq!(filter_mix(138), (0.0, 0.5, 0.5));
        assert_eq!(filter_mix(200), (0.0, 0.0, 1.0));
        // `updateFilterCoef`: low-pass 100 Hz at the left end, about 24.5 kHz at 125 and 25.1 kHz
        // in the centre; high-pass 20 Hz up to about 8 kHz at the right end.
        let near = |a: f32, b: f32, e: f32| (a - b).abs() < e;
        let (l, h, d, g) = filter_coef(0, 128);
        assert!(near(l, 100.0, 1e-3) && h == 20.0 && d == 0.5 && near(g, 1.0, 1e-6));
        assert!(near(filter_coef(125, 128).0, 24509.0, 2.0) && filter_coef(128, 128).0 == 25100.0);
        assert!(near(filter_coef(255, 128).1, 8026.3, 0.1));
        // PARAM: damping 4 at the left end, 0.5 at the centre, 0.0349 at the right end, where the
        // gain falls with it (COLOR 64: 0.5 + damping).
        assert!(near(filter_coef(60, 0).2, 4.0, 1e-6));
        let (_, _, d, g) = filter_coef(64, 255);
        assert!(near(d, 0.034912, 1e-5) && near(g, 0.534912, 1e-5));
        // The bottom of the high-pass: damping up to 2.
        assert!(near(filter_coef(131, 128).2, 2.0, 1e-6) && near(filter_coef(170, 128).2, 0.5, 1e-6) && filter_coef(160, 128).2 > 0.58);
        // The knobs glide one step per 0.33333 ms (15 frames at 48 kHz): 128 -> 0 takes 128 steps.
        let mut f = NxsFilter::new();
        let mut x = vec![0.0f32; 2 * 15 * 64];
        f.process(&mut x, 0, 128, 48000);
        assert_eq!(f.c.cur, 64);
    }

    #[test]
    fn filter_is_out_of_the_path_at_the_centre_and_cuts_at_the_ends() {
        assert!(level(0, 0.5, 0.5, 1000.0).abs() < 0.001);
        // Fully left: a 100 Hz low-pass, so 5 kHz is gone and 50 Hz stays.
        assert!(level(0, 0.0, 0.5, 5000.0) < -50.0);
        assert!(level(0, 0.0, 0.5, 50.0).abs() < 3.0);
        // Fully right: an 8 kHz high-pass, so 200 Hz is gone and 15 kHz stays.
        assert!(level(0, 1.0, 0.5, 200.0) < -50.0);
        assert!(level(0, 1.0, 0.5, 15000.0).abs() < 3.0);
    }

    #[test]
    fn every_effect_stays_bounded_and_does_something() {
        for (name, kind) in LIST {
            let Some(kind) = kind else { continue };
            for color in [0.05, 0.95] {
                let mut fx = Cfx::new();
                // (GATE COMP's gate is tried with a quiet sound: a loud one passes it untouched.)
                let amp = if kind == 8 && color < 0.5 { 0.1 } else { 0.5 };
                let x: Vec<f32> = (0..48000).flat_map(|i| { let v = (i as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * amp; [v, v] }).collect();
                let mut y = x.clone();
                fx.process(kind, color, 0.5, &mut y, 48000);
                let peak = y.iter().fold(0.0f32, |a, b| a.max(b.abs()));
                assert!(peak.is_finite() && peak < 4.0, "{name} at {color}: peak {peak}");
                let diff = (x.iter().zip(&y).map(|(a, b)| (a - b) * (a - b)).sum::<f32>() / x.len() as f32).sqrt();
                assert!(diff > 0.003, "{name} at {color} changes the sound ({diff})");
            }
        }
    }

    #[test]
    fn pitch_and_gate_comp() {
        let tone = |amp: f32| -> Vec<f32> { (0..96000).flat_map(|i| { let v = (i as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * amp; [v, v] }).collect() };
        let hz = |x: &[f32]| x.chunks_exact(2).zip(x.chunks_exact(2).skip(1)).filter(|(a, b)| a[0] <= 0.0 && b[0] > 0.0).count() as f32;
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        // PITCH: an octave up at the right end, an octave down at the left.
        for (color, want) in [(1.0, 880.0), (0.0, 220.0)] {
            let mut fx = Cfx::new();
            let mut x = tone(0.5);
            fx.process(4, color, 0.5, &mut x, 48000);
            let got = hz(&x[96000..]);
            assert!((got - want).abs() < want * 0.06, "{color}: {got}");
        }
        // GATE COMP (rekordbox's thresholds follow each band's recent peak): a steady tone
        // keeps the gate open at the left end, and the compressor at the right end turns any
        // steady tone down by the same share whatever its level.
        let mut fx = Cfx::new();
        let mut x = tone(0.1);
        fx.process(8, 0.0, 0.5, &mut x, 48000);
        assert!((rms(&x[96000..]) / (0.1 * std::f32::consts::FRAC_1_SQRT_2) - 1.0).abs() < 0.1, "{}", rms(&x[96000..]));
        let level = |amp: f32| {
            let mut fx = Cfx::new();
            let mut x = tone(amp);
            fx.process(8, 1.0, 0.5, &mut x, 48000);
            rms(&x[96000..]) / (amp * std::f32::consts::FRAC_1_SQRT_2)
        };
        let (loud, quiet) = (level(0.8), level(0.08));
        assert!(loud > 0.3 && loud < 0.6 && (loud - quiet).abs() < 0.02, "{loud} {quiet}");
    }

    #[test]
    fn traced_laws_of_jet_dub_echo_sweep_and_the_filters() {
        let near = |a: f32, b: f32| (a - b).abs() < 2e-3;
        // JET: out of the path at the centre; left the comb grows to 20 ms, right it shrinks
        // to 0.167 ms; feedback 0.55 / 0.6 over the middle of each side, 0.3 at the ends.
        assert_eq!(jet_law(128.0, 128.0), (1.0, 0.0, 2.6667, 0.6));
        let (d, w, ms, fb) = jet_law(0.0, 128.0);
        assert!(d == 0.7 && w == 0.7 && near(ms, 20.0) && near(fb, 0.3));
        let (d, w, ms, fb) = jet_law(114.0, 128.0);
        assert!(near(d, 0.85) && near(w, 0.35) && near(ms, 4.0645) && near(fb, 0.55));
        let (_, _, ms, fb) = jet_law(255.0, 128.0);
        assert!(near(ms, 0.16667) && near(fb, 0.3));
        assert!(near(jet_law(180.0, 128.0).3, 0.6) && near(jet_law(180.0, 62.0).3, 0.3) && near(jet_law(180.0, 255.0).3, 0.8));
        // DUB ECHO: 55 ms and 0.87 near the centre, 189 ms fully left, 150 ms fully right;
        // the input comes in over the first 30 % of the travel.
        assert_eq!(dub_law(128.0, 128.0), (0.0, 55.0, 0.87));
        let (inp, ms, fb) = dub_law(0.0, 128.0);
        assert!(inp == 1.0 && near(ms, 189.0) && near(fb, 0.95));
        let (inp, ms, fb) = dub_law(255.0, 128.0);
        assert!(inp == 1.0 && near(ms, 150.0) && near(fb, 0.98));
        assert!(near(dub_law(110.0, 128.0).0, 0.1724) && dub_law(110.0, 128.0).1 == 55.0);
        assert!(near(dub_law(0.0, 60.0).2, 0.45 + 0.08) && near(dub_law(0.0, 255.0).2, 0.99));
        // SWEEP (`updateFilterCoef`): PARAM picks 60 Hz .. 1 kHz .. 20 kHz; left the gap's edges
        // run from 20 kHz / 60 Hz to f, right the band narrows to f / 1.4 .. 1.4 f.
        let (h, l, d) = sweep_coef(0, 128);
        assert!((h - 20000.0).abs() < 1e-6 && (l - 60.0).abs() < 1e-6 && d == 1.0);
        let (h, l, _) = sweep_coef(123, 128);
        assert!((h - 1000.0).abs() < 0.01 && (l - 1000.0).abs() < 0.01);
        let (h, l, d) = sweep_coef(255, 128);
        assert!((h - 1000.0 / 1.40100781).abs() < 1e-3 && (l - 1401.00781).abs() < 1e-3 && (d - 0.707945784).abs() < 1e-9);
        assert!((sweep_coef(140, 128).2 - (22.0 * 0.009375 + 0.707945784)).abs() < 1e-9 && (sweep_coef(128, 0).0 - 60.0 / 17.4).abs() < 1e-6);
        // Its blend: dry at the centre, wet from 30 steps out.
        assert_eq!(sweep_mix(128), (1.0, 0.0));
        assert_eq!(sweep_mix(0), (0.0, 1.0));
        assert_eq!(sweep_mix(255), (0.0, 1.0));
        let (d, w) = sweep_mix(110);
        assert!((d - (5.0f32 - 8.0 * (1.0 - 0.004065 * 110.0)).powi(2)).abs() < 1e-5 && (d + w - 1.0).abs() < 1e-6);
        // LPF / HPF: the cutoffs' ends, the blend over the first steps, the resonance law.
        assert!(near(inst_fc(false, 0.0), 12000.0) && near(inst_fc(false, 1.0), 93.0) && near(inst_fc(true, 0.0), 61.0) && near(inst_fc(true, 1.0), 8951.0));
        assert_eq!(inst_law(false, 0.0, 128.0).0, 0.0);
        assert!(near(inst_law(false, 21.0, 128.0).0, 0.5) && inst_law(false, 100.0, 128.0).0 == 1.0);
        assert!(inst_law(false, 255.0, 128.0).2 < 1e-4 && near(inst_law(false, 10.0, 255.0).2, 1.44) && near(inst_law(true, 255.0, 255.0).2, 1.44));
    }

    #[test]
    fn inst_filters_and_their_tables() {
        // CRUSH's table is round(20 x (4129/20)^(i/123)): its ends and a middle step.
        assert_eq!((crush_fc(0), crush_fc(60), crush_fc(100), crush_fc(123)), (20.0, 269.0, 1524.0, 4129.0));
        assert_eq!((fc_index(3), fc_index(4), fc_index(100), fc_index(247), fc_index(250)), (0, 2, 50, 123, 123));
        // The LPF / HPF tables, read from the installed rekordbox (skipped without it): ends,
        // and every 15th step as listed in the fx3 notes.
        if let Some(t) = fc_tables_now() {
            assert_eq!((t.lpf[0], t.hpf[0], t.hpf[123]), (12000.0, 61.0, 8951.0));
            assert!((t.lpf[60] - 1080.91).abs() < 0.01 && (t.lpf[123] - 93.0).abs() < 1e-3 && t.hpf[60] == 915.0);
        }
        // `InstFxLPF::updateParameter`: dry below 4, the blend over 4..37, the resonance law.
        assert_eq!(inst_params(InstKind::Lpf, 3, 128), (1.0, 0.0, 12000.0, 1.44));
        let (d, w, _, q) = inst_params(InstKind::Lpf, 21, 0);
        assert!((w - 0.5).abs() < 1e-3 && (d - 0.5).abs() < 1e-3 && q.abs() < 1e-3);
        assert!((inst_params(InstKind::Lpf, 10, 255).3 - 1.44).abs() < 1e-4);
        // `ColorFxCrush::IfxHPF`: dry (1 - a/126)^2, gone from 130; resonance up to 1.2998.
        let (d, w, fc, q) = inst_params(InstKind::CrushHpf, 63, 0);
        assert!((d - 0.25).abs() < 1e-3 && (w - 0.75).abs() < 1e-3 && fc == crush_fc(31) && q > 0.0 && q < 1.2998);
        assert_eq!(inst_params(InstKind::CrushHpf, 200, 0).0, 0.0);
        // The filter: fully turned the LPF (93 Hz) takes 5 kHz away; at 0 it is dry exactly.
        let tone = |f: f32| -> Vec<f32> { (0..48000).flat_map(|i| { let v = (i as f32 * f * std::f32::consts::TAU / 48000.0).sin() * 0.5; [v, v] }).collect() };
        let mut x = tone(5000.0);
        let mut fl = InstFilter::new(InstKind::Lpf);
        fl.process(&mut x, 255, 0, 48000);
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        assert!(20.0 * (rms(&x[24000..]) / 0.35355).log10() < -60.0);
        let dry = tone(440.0);
        let mut y = dry.clone();
        let mut fl = InstFilter::new(InstKind::Hpf);
        fl.process(&mut y, 0, 128, 48000);
        assert_eq!(dry, y);
    }

    #[test]
    fn filter_centre_is_dry_and_glides_to_a_side() {
        let dry: Vec<f32> = (0..9600).flat_map(|i| { let v = (i as f32 * 0.05).sin() * 0.5; [v, v] }).collect();
        let mut y = dry.clone();
        let mut f = Cfx::new();
        f.process(0, 0.5, 0.5, &mut y, 48000);
        assert_eq!(dry, y);
        // Turned to the left end it takes 128 steps of 15 frames to get there.
        let mut f = NxsFilter::new();
        let mut z = dry.clone();
        f.process(&mut z, 0, 128, 48000);
        assert_eq!(f.c.cur, 0);
        assert_eq!(f.g, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn dub_echo_cores_follow_rekordbox() {
        // `onCfxKnobValueChanged`: the centre feeds neither core and sets 55 ms and PARAM's
        // feedback (low 0.87, high 0.88 at PARAM's centre; x (0.98 + ...) above 139).
        assert_eq!(dub_set(128, 128), [Some((0.0, 55.0, 0.87)), Some((0.0, 55.0, 0.88))]);
        let [a, b] = dub_set(128, 255);
        assert!((a.unwrap().2 - 0.98 * (0.87 + 0.0010345 * 116.0)).abs() < 1e-5 && (b.unwrap().2 - 0.98 * (0.88 + 0.00094828 * 116.0)).abs() < 1e-5);
        let [a, b] = dub_set(0, 128);
        assert!(b.is_none() && a.unwrap().0 == 1.0 && (a.unwrap().1 - 189.0).abs() < 0.01);
        let [a, b] = dub_set(255, 128);
        assert!(a.is_none() && (b.unwrap().1 - 150.0).abs() < 0.01);
        // The body: a click, fully left, repeats 189 ms later through the low core (band-pass
        // 1375 Hz, x 0.7); fully right 150 ms later through the high core.
        let first = |color: f32| -> (usize, f32) {
            let mut fx = Cfx::new();
            let mut x = vec![0.0f32; 48000 * 2];
            // (200 frames first: the input level comes up over 3.3 ms.)
            fx.process(6, color, 0.5, &mut x[..400], 48000);
            x[400] = 1.0;
            x[401] = 1.0;
            fx.process(6, color, 0.5, &mut x[400..], 48000);
            let tail = &x[800..];
            let (i, v) = tail.chunks_exact(2).enumerate().fold((0, 0.0f32), |m, (i, f)| if f[0].abs() > m.1 { (i, f[0].abs()) } else { m });
            (i + 400 - 200, v)
        };
        let (i, _) = first(0.0);
        assert!((i as i32 - 9072).abs() < 12, "{i}");
        let (i, _) = first(1.0);
        assert!((i as i32 - 7200).abs() < 12, "{i}");
        // Each core's output is at `DubEchoCore`'s post gain, made at 0.7 and kept there
        // (the filter types are synced when the effect is selected, so its fade never runs).
        let mut c = DubCore::new();
        let y = c.run([0.0, 0.0], 48000.0);
        assert!(DUB_POST_GAIN == 0.7 && y == [0.0, 0.0]);
        // Back at the centre with the tails gone the sound is untouched.
        let mut fx = Cfx::new();
        let dry: Vec<f32> = (0..4800).flat_map(|i| { let v = (i as f32 * 0.03).sin() * 0.5; [v, v] }).collect();
        let mut y = dry.clone();
        fx.process(6, 0.5, 0.5, &mut y, 48000);
        assert_eq!(dry, y);
    }

    #[test]
    fn jet_is_rekordboxs_tape_echo() {
        // Fully left: dry 0.7, a 20 ms echo (959.98 frames, read between frames) at 0.7 whose
        // repeats come back at feedback 0.3.
        let mut fx = Cfx::new();
        let mut x = vec![0.0f32; 4800 * 2];
        fx.process(1, 0.0, 0.5, &mut x[..2], 48000);
        x[2] = 1.0;
        x[3] = 1.0;
        fx.process(1, 0.0, 0.5, &mut x[2..], 48000);
        assert!((x[2] - 0.7).abs() < 1e-6, "{}", x[2]);
        let at = |k: usize| x[(1 + k) * 2];
        assert!((at(960) - 0.7 * 0.98).abs() < 0.01 && (at(959) - 0.7 * 0.02).abs() < 0.01, "{} {}", at(960), at(959));
        assert!(((at(1920) + at(1919) + at(1921)) / (0.7 * 0.3) - 1.0).abs() < 0.02);
        // In the centre the sound is dry exactly.
        let mut fx = Cfx::new();
        let dry: Vec<f32> = (0..4800).flat_map(|i| { let v = (i as f32 * 0.03).sin() * 0.5; [v, v] }).collect();
        let mut y = dry.clone();
        fx.process(1, 0.5, 0.5, &mut y, 48000);
        assert_eq!(dry, y);
    }

    #[test]
    fn lpf_and_hpf_for_the_pads() {
        // Fully turned: the LPF (93 Hz) removes 5 kHz, the HPF (8.95 kHz) removes 200 Hz.
        assert!(level(LPF, 1.0, 0.5, 5000.0) < -50.0);
        assert!(level(LPF, 1.0, 0.5, 40.0).abs() < 6.0);
        assert!(level(HPF, 1.0, 0.5, 200.0) < -50.0);
        assert_eq!((name(LPF), name(HPF)), ("LPF", "HPF"));
    }
}
