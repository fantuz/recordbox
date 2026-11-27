//! The Performance mixer's signal processing, after rekordbox 7.2.19's internal mixer
//! (`checks/re-7.2.19/mixer-fx-recording.md`): per channel TRIM, then HIGH / MID / LOW as an
//! equalizer or an isolator, then the channel fader and crossfader; on the master a peak
//! limiter. The level meters read each channel after its EQ and before its fader.
//!
//! The filter equations are rekordbox's own (traced: bilinear transform of the analog
//! prototypes below, `k = 2 F tan(pi f / F)`). Its knob and fader laws are lookup tables in the
//! original; here each is a line through a few points read off those tables, which follows
//! them to within a fraction of a dB (trim, faders) or 0.3 dB (MID gain).

/// Straight-line interpolation through `pts` (x ascending), clamped at the ends.
fn through(pts: &[(f32, f32)], x: f32) -> f32 {
    if x <= pts[0].0 {
        return pts[0].1;
    }
    for w in pts.windows(2) {
        if x <= w[1].0 {
            let t = (x - w[0].0) / (w[1].0 - w[0].0);
            return w[0].1 + t * (w[1].1 - w[0].1);
        }
    }
    pts[pts.len() - 1].1
}

fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// TRIM knob (0 = fully left, 0.5 = centre, 1 = fully right) to a gain relative to the centre
/// position: silence at the left end, 0 dB at the centre, +8.8 dB fully right. (rekordbox
/// shows the knob as -inf .. +9 dB. How its Headroom setting scales this was not traced;
/// here the centre is unity.)
pub fn trim_gain(pos: f32) -> f32 {
    let k = pos.clamp(0.0, 1.0) * 255.0;
    if k < 0.5 {
        return 0.0;
    }
    // (knob step, dB below the top of the table)
    const PTS: [(f32, f32); 12] = [(1.0, -86.8), (4.0, -78.3), (8.0, -64.3), (16.0, -40.6), (32.0, -30.3), (64.0, -17.1), (96.0, -11.9), (127.0, -8.79), (160.0, -6.36), (192.0, -4.52), (224.0, -2.01), (255.0, 0.0)];
    db(through(&PTS, k) + 8.79)
}

/// What the TRIM knob shows, in dB (-inf at the left end).
pub fn trim_db(pos: f32) -> f32 {
    let g = trim_gain(pos);
    if g <= 0.0 { f32::NEG_INFINITY } else { 20.0 * g.log10() }
}

/// Channel fader (0 = bottom, 1 = top) to gain. `curve`: Preferences > Controller > Mixer >
/// Channel Fader Curve, 0 Exponential, 1 Linear (the default), 2 Logarithmic.
pub fn channel_fader_gain(pos: f32, curve: u8) -> f32 {
    let p = pos.clamp(0.0, 1.0) * 1023.0;
    if p < 5.5 {
        return 0.0;
    }
    // (fader step, dB)
    const LINEAR: [(f32, f32); 13] = [(6.0, -100.0), (16.0, -87.9), (32.0, -82.5), (64.0, -72.5), (128.0, -55.4), (256.0, -31.2), (384.0, -17.8), (512.0, -12.0), (640.0, -8.94), (768.0, -5.88), (900.0, -2.73), (1000.0, -0.33), (1023.0, 0.0)];
    const EXPONENTIAL: [(f32, f32); 8] = [(6.0, -100.0), (128.0, -54.9), (256.0, -33.3), (512.0, -19.1), (768.0, -9.37), (900.0, -4.34), (1000.0, -0.6), (1023.0, 0.0)];
    const LOGARITHMIC: [(f32, f32); 8] = [(6.0, -100.0), (128.0, -28.2), (256.0, -16.0), (512.0, -7.13), (768.0, -2.82), (900.0, -1.22), (1000.0, -0.2), (1023.0, 0.0)];
    db(match curve {
        0 => through(&EXPONENTIAL, p),
        2 => through(&LOGARITHMIC, p),
        _ => through(&LINEAR, p),
    })
}

/// HIGH and LOW of the equalizer: knob (0..1) to shelf gain in dB, rekordbox's formula for
/// its DJM-900NXS2 type: -26 dB fully left, flat around the centre, +6 dB fully right.
pub fn shelf_db(pos: f32) -> f32 {
    let k = (pos.clamp(0.0, 1.0) * 255.0).round();
    if k < 126.0 {
        0.45 - 20.0 * (1.15 - k / 126.0).powi(2)
    } else if k <= 129.0 {
        0.0
    } else {
        k / 21.0 - 6.142857
    }
}

/// MID of the equalizer: knob to (gain in dB, the peaking filter's Q argument).
pub fn mid_db_q(pos: f32) -> (f32, f32) {
    let k = (pos.clamp(0.0, 1.0) * 255.0).round();
    const GAIN: [(f32, f32); 13] = [(0.0, -26.38), (16.0, -17.08), (32.0, -11.06), (64.0, -4.26), (100.0, -1.35), (120.0, -0.21), (126.0, 0.0), (129.0, 0.0), (140.0, 0.44), (160.0, 1.38), (200.0, 3.12), (230.0, 4.65), (255.0, 6.0)];
    const Q: [(f32, f32); 13] = [(0.0, 2.297), (16.0, 0.788), (32.0, 0.394), (64.0, 0.18), (100.0, 0.129), (120.0, 0.113), (126.0, 0.11), (129.0, 0.11), (140.0, 0.116), (160.0, 0.129), (200.0, 0.158), (230.0, 0.188), (255.0, 0.22)];
    (through(&GAIN, k), through(&Q, k))
}

/// Headphones MIX knob (0 = CUE, 1 = MASTER) to the master's share; the cued decks get
/// `cue_master_balance(1 - mix)`. rekordbox's table: silent at the far end, rising in a
/// straight line to full at the centre and full from there on, so the centre has both.
pub fn cue_master_balance(mix: f32) -> f32 {
    let i = (mix.clamp(0.0, 1.0) * 255.0).round();
    if i <= 2.0 { 0.0 } else { ((i - 2.0) / 122.0).min(1.0) }
}

/// Headphones LEVEL knob to gain: silent at the left end, -15.7 dB at the centre, 0 dB at the
/// right end (points read off rekordbox's table).
pub fn headphones_level(pos: f32) -> f32 {
    let k = pos.clamp(0.0, 1.0) * 255.0;
    if k < 3.0 {
        return 0.0;
    }
    const PTS: [(f32, f32); 11] = [(3.0, -124.8), (8.0, -84.3), (16.0, -65.2), (32.0, -48.0), (64.0, -31.6), (96.0, -22.2), (127.0, -15.7), (160.0, -10.5), (191.0, -6.4), (224.0, -2.8), (255.0, 0.0)];
    db(through(&PTS, k))
}

/// One second-order section, in double precision as the original, transposed direct form II.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z: [[f64; 2]; 2],
}

impl Biquad {
    pub fn unity() -> Biquad {
        Biquad { b0: 1.0, ..Default::default() }
    }

    /// Coefficients from analog numerator and denominator polynomials `p0 + p1 s + p2 s^2`
    /// (bilinear transform at sample rate `rate`). The filter's history is kept.
    fn set(&mut self, rate: f64, num: [f64; 3], den: [f64; 3]) {
        let b = |p: [f64; 3]| [p[0] / (rate * rate) + 2.0 * p[1] / rate + 4.0 * p[2], 2.0 * p[0] / (rate * rate) - 8.0 * p[2], p[0] / (rate * rate) - 2.0 * p[1] / rate + 4.0 * p[2]];
        let (n, d) = (b(num), b(den));
        self.b0 = n[0] / d[0];
        self.b1 = n[1] / d[0];
        self.b2 = n[2] / d[0];
        self.a1 = d[1] / d[0];
        self.a2 = d[2] / d[0];
    }

    fn k(rate: f64, freq: f64) -> f64 {
        2.0 * rate * (std::f64::consts::PI * freq / rate).tan()
    }

    pub fn peaking(&mut self, rate: f64, freq: f64, gain_db: f64, q: f64) {
        let (k, a) = (Self::k(rate, freq), 10f64.powf(gain_db / 20.0));
        self.set(rate, [k * k, (k / q) * a.max(1.0), 1.0], [k * k, (k / q) / a.min(1.0), 1.0]);
    }

    pub fn high_shelf(&mut self, rate: f64, freq: f64, gain_db: f64, r: f64) {
        let (k, a) = (Self::k(rate, freq), 10f64.powf(gain_db / 20.0));
        let (kn, kd) = if a < 1.0 { (k / a.sqrt(), k) } else { (k, k * a.sqrt()) };
        self.set(rate, [a * kn * kn, a * r * kn, a], [kd * kd, r * kd, 1.0]);
    }

    pub fn low_shelf(&mut self, rate: f64, freq: f64, gain_db: f64, r: f64) {
        let (k, a) = (Self::k(rate, freq), 10f64.powf(gain_db / 20.0));
        let (kn, kd) = if a < 1.0 { (k * a.sqrt(), k) } else { (k, k / a.sqrt()) };
        self.set(rate, [kn * kn, r * kn, 1.0], [kd * kd, r * kd, 1.0]);
    }

    #[inline]
    pub fn run(&mut self, ch: usize, x: f32) -> f32 {
        let x = x as f64;
        let z = &mut self.z[ch];
        let y = self.b0 * x + z[0];
        z[0] = self.b1 * x + z[1] - self.a1 * y;
        z[1] = self.b2 * x - self.a2 * y;
        y as f32
    }
}

/// The equalizer (rekordbox's default "DJM-900NXS2" type): a high shelf at 2.2 kHz, a peak at
/// 1 kHz and a low shelf at 250 Hz in series, in that order.
pub struct Eq3 {
    bands: [Biquad; 3],
    /// Knob positions the coefficients were made for (low, mid, high) and the rate.
    made: ([f32; 3], u32),
    /// Positions in use, moving toward the knobs (so a turned knob does not step).
    cur: [f32; 3],
    /// The "DJM-900NXS" EQ type: three peaking filters at 13 kHz, 1 kHz and 70 Hz.
    pub nxs: bool,
    made_nxs: bool,
}

impl Eq3 {
    pub fn new() -> Eq3 {
        Eq3 { bands: [Biquad::unity(); 3], made: ([-1.0; 3], 0), cur: [0.5; 3], nxs: false, made_nxs: false }
    }

    /// Before a block: move toward the knob positions (low, mid, high) and remake the filters
    /// if they moved. `frames`: the block's length, for the speed of the move.
    pub fn prepare(&mut self, rate: u32, knobs: [f32; 3], frames: usize) {
        // A full sweep of a knob takes about 20 ms.
        let step = (frames as f32 / (0.02 * rate as f32)).min(1.0);
        for (c, k) in self.cur.iter_mut().zip(knobs) {
            *c += (k - *c).clamp(-step, step);
        }
        if self.made != (self.cur, rate) || self.made_nxs != self.nxs {
            self.made = (self.cur, rate);
            self.made_nxs = self.nxs;
            let r = rate as f64;
            if self.nxs {
                // rekordbox's `Equalizer`: high, mid, low in series, all three with the MID
                // knob's gain law; the Q argument is 0.0802 (0.1103 for MID) times the gain
                // or its inverse, whichever is over 1 (its Q tables follow that exactly).
                for (band, (freq, q0, knob)) in [(13000.0, 0.0802, 2usize), (1000.0, 0.1103, 1), (70.0, 0.0802, 0)].into_iter().enumerate() {
                    let g = mid_db_q(self.cur[knob]).0 as f64;
                    let a = 10f64.powf(g / 20.0);
                    self.bands[band].peaking(r, freq, g, q0 * a.max(1.0 / a));
                }
                return;
            }
            self.bands[0].high_shelf(r, 2200.0, shelf_db(self.cur[2]) as f64, 2.0);
            let (g, q) = mid_db_q(self.cur[1]);
            self.bands[1].peaking(r, 1000.0, g as f64, q as f64);
            self.bands[2].low_shelf(r, 250.0, shelf_db(self.cur[0]) as f64, 1.5);
        }
    }

    #[inline]
    pub fn run(&mut self, ch: usize, x: f32) -> f32 {
        let x = self.bands[0].run(ch, x);
        let x = self.bands[1].run(ch, x);
        self.bands[2].run(ch, x)
    }
}

/// The master peak limiter, as rekordbox's `Limitter`: stereo-linked, sample by sample, no
/// look-ahead. Gain falls with a 3 ms time constant while the output is over full scale, holds
/// for 50 ms, and recovers with a 600 ms time constant once the output is 3 dB under.
pub struct Limiter {
    gain: f32,
    hold: u32,
    rate: u32,
    attack: f32,
    release: f32,
    hold_len: u32,
}

impl Limiter {
    pub const fn new() -> Limiter {
        Limiter { gain: 1.0, hold: 0, rate: 0, attack: 1.0, release: 1.0, hold_len: 0 }
    }

    pub fn process(&mut self, out: &mut [f32], channels: usize, rate: u32, enabled: bool) {
        if rate != self.rate {
            self.rate = rate;
            self.attack = (-1000.0 / (rate as f32 * 3.0)).exp();
            self.release = (1000.0 / (rate as f32 * 600.0)).exp();
            self.hold_len = (0.05 * rate as f32).round() as u32;
        }
        if !enabled && self.gain >= 1.0 {
            return;
        }
        for f in out.chunks_mut(channels.max(1)) {
            let mut p = 0.0f32;
            for v in f.iter_mut() {
                *v *= self.gain;
                p = p.max(v.abs());
            }
            if enabled && p > 1.0 {
                self.gain *= self.attack;
                self.hold = self.hold_len;
            } else if p <= 0.70795 || !enabled {
                if self.hold < 1 || !enabled {
                    self.gain = (self.gain * self.release).min(1.0);
                } else {
                    self.hold -= 1;
                }
            } else {
                self.hold = self.hold_len;
            }
        }
    }
}

/// What a level meter shows from one reading to the next (rekordbox: read every 36 ms, a
/// higher peak shows at once, otherwise the level falls 2 dB per reading).
pub fn meter_fall(shown_db: f32, peak: f32, seconds: f32) -> f32 {
    let new = if peak > 0.0 { 20.0 * peak.min(1.0).log10() } else { -120.0 };
    new.max(shown_db - 2.0 * seconds / 0.036).max(-120.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(f: &mut dyn FnMut(f32) -> f32, freq: f32) -> f32 {
        let n = 48000;
        let mut acc = 0.0f64;
        for i in 0..n {
            let y = f((i as f32 * freq * std::f32::consts::TAU / 48000.0).sin());
            if i >= n / 2 {
                acc += (y * y) as f64;
            }
        }
        10.0 * ((acc / (n / 2) as f64) as f32 * 2.0).log10()
    }

    #[test]
    fn knob_and_fader_laws() {
        assert_eq!(trim_gain(0.0), 0.0);
        assert!((trim_db(0.5)).abs() < 0.1);
        assert!((trim_db(1.0) - 8.79).abs() < 0.01);
        assert!((shelf_db(0.0) + 26.0).abs() < 0.01 && shelf_db(0.5) == 0.0 && (shelf_db(1.0) - 6.0).abs() < 0.01);
        assert!((mid_db_q(0.0).0 + 26.38).abs() < 0.01 && mid_db_q(0.5).0 == 0.0 && (mid_db_q(1.0).0 - 6.0).abs() < 0.01);
        for curve in 0..3 {
            assert_eq!(channel_fader_gain(0.0, curve), 0.0);
            assert!((channel_fader_gain(1.0, curve) - 1.0).abs() < 1e-6);
            let mut prev = 0.0;
            for i in 0..=100 {
                let g = channel_fader_gain(i as f32 / 100.0, curve);
                assert!(g >= prev);
                prev = g;
            }
        }
        // Linear curve: -12 dB at half way; the exponential curve is lower there, the logarithmic higher.
        assert!((20.0 * channel_fader_gain(0.5, 1).log10() + 12.0).abs() < 0.2);
        assert!(channel_fader_gain(0.5, 0) < channel_fader_gain(0.5, 1) && channel_fader_gain(0.5, 2) > channel_fader_gain(0.5, 1));
    }

    #[test]
    fn equalizer_is_flat_at_the_centre_and_reaches_its_gains() {
        let mut eq = Eq3::new();
        eq.prepare(48000, [0.5; 3], 48000);
        for f in [40.0, 250.0, 1000.0, 2200.0, 12000.0] {
            assert!(level(&mut |x| eq.run(0, x), f).abs() < 0.01, "{f} Hz");
        }
        let mut cut = |knobs: [f32; 3], f: f32| {
            let mut eq = Eq3::new();
            eq.prepare(48000, knobs, 48000);
            eq.prepare(48000, knobs, 48000);
            level(&mut |x| eq.run(0, x), f)
        };
        // LOW fully left: -26 dB in the bass, the top untouched. Fully right: +6 dB.
        assert!((cut([0.0, 0.5, 0.5], 30.0) + 26.0).abs() < 1.0);
        assert!(cut([0.0, 0.5, 0.5], 8000.0).abs() < 0.3);
        assert!((cut([1.0, 0.5, 0.5], 30.0) - 6.0).abs() < 0.3);
        // HIGH likewise in the treble.
        // (The shelf is gentle: 1 dB short of its full cut at 16 kHz, as the formula gives.)
        assert!((cut([0.5, 0.5, 0.0], 16000.0) + 26.0).abs() < 1.5);
        assert!(cut([0.5, 0.5, 0.0], 60.0).abs() < 0.3);
        assert!((cut([0.5, 0.5, 1.0], 16000.0) - 6.0).abs() < 0.3);
        // MID at 1 kHz.
        assert!((cut([0.5, 0.0, 0.5], 1000.0) + 26.38).abs() < 0.2);
        assert!((cut([0.5, 1.0, 0.5], 1000.0) - 6.0).abs() < 0.1);
    }

    #[test]
    fn limiter_holds_the_output_at_full_scale() {
        let mut lim = Limiter::new();
        let mut buf: Vec<f32> = (0..48000 * 2).map(|i| ((i / 2) as f32 * 0.05).sin() * 2.0).collect();
        lim.process(&mut buf, 2, 48000, true);
        // After the first few milliseconds the 6 dB overshoot is gone.
        let peak = buf[4800..].iter().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(peak < 1.02, "{peak}");
        // A signal under full scale passes unchanged once the limiter has recovered.
        let mut lim = Limiter::new();
        let mut quiet: Vec<f32> = (0..9600).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let want = quiet.clone();
        lim.process(&mut quiet, 2, 48000, true);
        assert_eq!(quiet, want);
    }

    #[test]
    fn headphones_mix_and_level() {
        // MIX at the CUE end: no master. Centre: both full. MASTER end: no cue.
        assert_eq!(cue_master_balance(0.0), 0.0);
        assert_eq!(cue_master_balance(0.5), 1.0);
        assert_eq!(cue_master_balance(1.0), 1.0);
        assert!((cue_master_balance(0.25) - 0.508).abs() < 0.01);
        assert_eq!(headphones_level(0.0), 0.0);
        assert!((20.0 * headphones_level(0.5).log10() + 15.7).abs() < 0.3);
        assert!((headphones_level(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn meter_rises_at_once_and_falls_two_db_per_reading() {
        assert!((meter_fall(-40.0, 1.0, 0.036)).abs() < 1e-6);
        assert!((meter_fall(0.0, 0.0, 0.036) + 2.0).abs() < 1e-6);
        assert!((meter_fall(0.0, 0.0, 0.072) + 4.0).abs() < 1e-6);
    }
}
