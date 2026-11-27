//! SOUND COLOR FX (the CFX row of the FX panel): one effect per channel, driven by its COLOR
//! knob (centre = off) and the shared PARAM knob. In the signal path it sits after the EQ and
//! before the channel fader, as in rekordbox 7.2.19 (`checks/re-7.2.19/mixer-fx-recording.md`
//! 2.6); the master's sits on the master mix.
//!
//! FILTER follows rekordbox's traced rules exactly in its blend zones, cutoff curves and
//! resonance law (the filters themselves are ordinary second-order sections; rekordbox's own
//! filter shape was not traced). JET (delay, feedback and mix for every knob position), DUB
//! ECHO (its two short tape echoes: time, feedback and input level for every knob position)
//! and SWEEP (the notch and band-pass edges) follow rekordbox's parameter code; the comb,
//! echo and filter bodies are plain ones. PITCH (`ColorFxPitch`: range and COLOR law, through
//! rekordbox's pitch shifter in `fx_core`) and NOISE (`ColorFxNoise`: level, band-pass centre,
//! Q and gain) are traced. CRUSH's hold, bit mask and mix are traced (`ColorFxCrush`); its high-pass cutoffs are
//! a log line through the ends of rekordbox's table. SPACE and GATE COMP are ported from rekordbox's code in `cfx_space`. LPF and HPF (`InstFxLPF` / `InstFxHPF`) exist
//! for the PAD FX pads and MERGE FX.

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
    (dry, wet, delay, fb.min(0.97))
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

/// rekordbox's CFX SWEEP (`ColorFxSweep::updateFilterCoef`) for COLOR `c` and PARAM `p` in
/// 8-bit steps: PARAM picks a frequency (60 Hz .. 1 kHz .. 20 kHz). Left of the centre the
/// sound loses a band around it that widens with the knob (what is kept: below the first
/// frequency and above the second); right of it only a band around it is kept, narrowing
/// with the knob (high-pass at the first, low-pass at the second, with the resonance).
pub fn sweep_law(c: f32, p: f32) -> (f32, f32, f32) {
    let f = if p < 128.0 { 60.0 + 7.4015748 * p } else { 1000.0 + 149.606299 * (p - 128.0) };
    if c < 125.0 {
        let t = c.min(123.0) / 123.0;
        (60.0 * (f / 60.0).powf(t), 20000.0 * (f / 20000.0).powf(t), 1.0)
    } else {
        let k = if c > 130.0 { (256.0 - c) * (256.0 - c) * 0.00100781053 + 1.4 } else { 17.4 };
        let q = if c > 130.0 && c <= 161.0 { (162.0 - c) * 0.009375 + 0.707945784 } else if c > 161.0 { 0.707945784 } else { 1.00794578 };
        (f / k, (f * k).min(20000.0), q)
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

/// rekordbox's CFX FILTER as numbers: for a COLOR position `c` (0..255) and PARAM `p`
/// (0..255), the share of low-pass, dry and high-pass, the two cutoffs and the resonance.
pub fn filter_law(c: f32, p: f32) -> (f32, f32, f32, f32, f32, f32) {
    let (lp, dry, hp) = if c < 110.0 {
        (1.0, 0.0, 0.0)
    } else if c <= 125.0 {
        let d = (c - 110.0) / 16.0;
        (1.0 - d, d, 0.0)
    } else if c <= 130.0 {
        (0.0, 1.0, 0.0)
    } else if c <= 146.0 {
        let h = (c - 130.0) / 16.0;
        (0.0, 1.0 - h, h)
    } else {
        (0.0, 0.0, 1.0)
    };
    let cl = c.min(125.0);
    let lp_fc = 100.0 + 198.41 * cl * (cl / 126.0).powi(2);
    let ch = ((c.max(131.0) - 130.0) / 126.0).powi(3);
    let hp_fc = 20.0 + 8200.0 * ch;
    let mut q = if p <= 127.0 { 0.5 + 3.5 * (1.0 - p / 128.0).powi(2) } else { 0.5 - 0.0036621 * (p - 128.0) };
    // Extra resonance at the low end of the high-pass.
    if c >= 131.0 && hp_fc < 249.9 {
        let t = 35.652 * ch;
        let r = (4.0 * (1.0 - t).powi(3) + 0.03).min(2.0);
        q = q.max(r);
    }
    (lp, dry, hp, lp_fc, hp_fc, q.max(0.05))
}

const ECHO_LEN: usize = 24000;

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
}


impl Cfx {
    pub fn new() -> Cfx {
        Cfx { kind: 0, color: 0.5, lp: Bq::default(), hp: Bq::default(), band: Bq::default(), seed: 0x1234_5678, hold: [0.0; 2], hold_n: 0.0, delay: vec![0.0; ECHO_LEN * 2], w: 0, fb_lp: [0.0; 2], made: (-1.0, -1.0, 0), ps: crate::fx_core::PitchShifter::new(48000.0), ps_rate: 48000, space: crate::cfx_space::Space::new(), gc: crate::cfx_space::GateComp::new() }
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
                0 => {
                    let (lp, dry, hp, lfc, hfc, q) = filter_law(c * 255.0, p * 255.0);
                    if self.made != (c, p, rate) {
                        self.made = (c, p, rate);
                        self.lp.set(r, lfc.min(0.45 * r), q, false);
                        self.hp.set(r, hfc, q, true);
                    }
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let x = *v;
                            let l = self.lp.run(ch, x);
                            let h = self.hp.run(ch, x);
                            *v = l * lp + x * dry + h * hp;
                        }
                    }
                }
                1 => {
                    // JET: a feedback comb at rekordbox's delay, feedback and mix for the
                    // knob position; PARAM scales the feedback.
                    let (dry, wet, ms, fbk) = jet_law((c * 255.0).round(), (p * 255.0).round());
                    let d = (ms * 0.001 * r).max(1.0);
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let i = d as usize;
                            let t = d - i as f32;
                            let a = self.delay[((self.w + ECHO_LEN - i) % ECHO_LEN) * 2 + ch];
                            let b = self.delay[((self.w + ECHO_LEN - i - 1) % ECHO_LEN) * 2 + ch];
                            let e = a + (b - a) * t;
                            self.delay[self.w * 2 + ch] = (*v + e * fbk).clamp(-4.0, 4.0);
                            *v = *v * dry + e * wet;
                        }
                        self.w = (self.w + 1) % ECHO_LEN;
                    }
                }
                2 => {
                    // CRUSH (`ColorFxCrush::processEffectConcrete`): the sound alone, its
                    // samples held (`CrushCore`) for at most 0.5 ms and, with PARAM above 144
                    // and a hold over 0.104 ms, its floats' mantissas cut (fewer bits as PARAM
                    // rises); to the right a high-pass rises from 20 Hz to 4.1 kHz.
                    let (ci, pi) = ((c * 255.0).round() as i32, (p * 255.0).round() as i32);
                    let (hold_ms, mask, hp) = crush_law(ci, pi);
                    let hold = (hold_ms * 0.001 * r).max(1.0);
                    if right && self.made != (c, p, rate) {
                        self.made = (c, p, rate);
                        self.hp.set(r, hp, 0.7071, true);
                    }
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        let x = [f32::from_bits(f[0].to_bits() & mask), f32::from_bits(f[1].to_bits() & mask)];
                        self.hold_n += 1.0;
                        if self.hold_n >= hold {
                            self.hold_n -= hold;
                            self.hold = x;
                        }
                        for (ch, v) in f.iter_mut().enumerate() {
                            let y = self.hold[ch];
                            *v = if right { self.hp.run(ch, y) } else { y };
                        }
                    }
                }
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
                6 => {
                    // DUB ECHO: a short echo at rekordbox's time, feedback and input level
                    // for the knob position; left low-passes the repeats, right high-passes
                    // them (around its pre-filter's 1375 Hz).
                    let (inp, ms, fbk) = dub_law((c * 255.0).round(), (p * 255.0).round());
                    let d = ((ms * 0.001 * r) as usize).clamp(1, ECHO_LEN - 1);
                    let k = 1.0 - (-std::f32::consts::TAU * 1375.0 / r).exp();
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let e = self.delay[((self.w + ECHO_LEN - d) % ECHO_LEN) * 2 + ch];
                            self.fb_lp[ch] += (e - self.fb_lp[ch]) * k;
                            let coloured = if right { e - self.fb_lp[ch] } else { self.fb_lp[ch] };
                            self.delay[self.w * 2 + ch] = (*v * inp + coloured * fbk).clamp(-4.0, 4.0);
                            *v += coloured;
                        }
                        self.w = (self.w + 1) % ECHO_LEN;
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
                LPF | HPF => {
                    // rekordbox's CFX-style low-pass / high-pass: the knob's distance from
                    // the centre is how far the filter is turned.
                    let hp = self.kind == HPF;
                    let (wet, fc, reso) = inst_law(hp, (depth * 255.0).round(), (p * 255.0).round());
                    if self.made != (c, p, rate) {
                        self.made = (c, p, rate);
                        self.lp.set(r, fc.min(0.45 * r), 0.5 + reso, hp);
                    }
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            let y = self.lp.run(ch, *v);
                            *v += (y - *v) * wet;
                        }
                    }
                }
                _ => {
                    // SWEEP: left a widening gap around PARAM's frequency, right a narrowing
                    // band around it (rekordbox's edges).
                    let (f1, f2, q) = sweep_law((c * 255.0).round(), (p * 255.0).round());
                    if self.made != (c, p, rate) {
                        self.made = (c, p, rate);
                        if right {
                            self.hp.set(r, f1.max(20.0), q, true);
                            self.lp.set(r, f2.min(0.45 * r), q, false);
                        } else {
                            self.lp.set(r, f1.max(20.0), 0.7071, false);
                            self.hp.set(r, f2.min(0.45 * r), 1.0, true);
                        }
                    }
                    for f in buf[n * 2..end * 2].chunks_exact_mut(2) {
                        for (ch, v) in f.iter_mut().enumerate() {
                            *v = if right { let h = self.hp.run(ch, *v); self.lp.run(ch, h) } else { self.lp.run(ch, *v) + self.hp.run(ch, *v) };
                        }
                    }
                }
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
        // Blend zones.
        assert_eq!(filter_law(60.0, 128.0).0, 1.0);
        let (lp, dry, hp, ..) = filter_law(118.0, 128.0);
        assert!((lp - 0.5).abs() < 1e-6 && (dry - 0.5).abs() < 1e-6 && hp == 0.0);
        assert_eq!(filter_law(128.0, 128.0).1, 1.0);
        assert!((filter_law(138.0, 128.0).2 - 0.5).abs() < 1e-6);
        assert_eq!(filter_law(200.0, 128.0).2, 1.0);
        // Cutoffs: low-pass 100 Hz at the left end and about 24.5 kHz at 125; high-pass about
        // 8 kHz at the right end.
        assert!((filter_law(0.0, 128.0).3 - 100.0).abs() < 0.01);
        assert!((filter_law(125.0, 128.0).3 - 24509.0).abs() < 60.0, "{}", filter_law(125.0, 128.0).3);
        assert!((filter_law(255.0, 128.0).4 - 8026.0).abs() < 30.0, "{}", filter_law(255.0, 128.0).4);
        // Resonance: 4.0 with PARAM at the left end, 0.5 at the centre, falling to the right.
        assert!((filter_law(60.0, 0.0).5 - 4.0).abs() < 1e-5);
        assert!((filter_law(60.0, 128.0).5 - 0.5).abs() < 1e-5);
        assert!(filter_law(60.0, 255.0).5 < 0.1);
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
        // SWEEP: PARAM picks 60 Hz .. 1 kHz .. 20 kHz; left the gap around it widens to
        // 60 Hz .. 20 kHz, right the band narrows to within a factor 1.4 of it.
        assert!(near(sweep_law(0.0, 0.0).0, 60.0) && near(sweep_law(123.0, 128.0).0, 1000.0) && near(sweep_law(123.0, 128.0).1, 1000.0));
        let (lo, hi, _) = sweep_law(0.0, 128.0);
        assert!(near(lo, 60.0) && near(hi, 20000.0));
        let (hp, lp, q) = sweep_law(255.0, 128.0);
        assert!((hp - 1000.0 / 1.401).abs() < 1.0 && (lp - 1401.0).abs() < 1.5 && near(q, 0.7079));
        assert!(sweep_law(131.0, 128.0).0 < 60.0 && near(sweep_law(140.0, 128.0).2, 0.9142));
        // LPF / HPF: the cutoffs' ends, the blend over the first steps, the resonance law.
        assert!(near(inst_fc(false, 0.0), 12000.0) && near(inst_fc(false, 1.0), 93.0) && near(inst_fc(true, 0.0), 61.0) && near(inst_fc(true, 1.0), 8951.0));
        assert_eq!(inst_law(false, 0.0, 128.0).0, 0.0);
        assert!(near(inst_law(false, 21.0, 128.0).0, 0.5) && inst_law(false, 100.0, 128.0).0 == 1.0);
        assert!(inst_law(false, 255.0, 128.0).2 < 1e-4 && near(inst_law(false, 10.0, 255.0).2, 1.44) && near(inst_law(true, 255.0, 255.0).2, 1.44));
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
