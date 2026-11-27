//! MERGE FX's clip player fade at its restarts: rekordbox's `rbxfrm::f2dsp::Attenuator`
//! (`Float2_Attenuator.cpp`), as `AudioPlayerCore` runs it each block: the block's
//! discontinuous points (where the playing place jumps) go through
//! `letSmoothDiscontinuousPoints` 0x1028885f0, which plans fade-outs into each point and
//! fade-ins from it, then `apply` 0x102887d70 gives every frame its gain from the 1 ms
//! raised-sine curve (`createCurve` 0x102887b84). No allocation: the curve and the plan are
//! fixed arrays (rekordbox grows its plan with `realloc`; a block here has far fewer points than
//! it holds).

/// Longest curve (1 ms at 192 kHz is 192 frames).
const MAX_LEN: usize = 256;
/// Most planned fades in a block (two a point).
const MAX_EV: usize = 64;

/// One planned change (`letSmoothDiscontinuousPoints`' 12-byte entries): at frame `at` of the
/// block start fading in (`fade_in`) or out, from curve place `pos` (`None`: where it is).
#[derive(Clone, Copy, Default)]
struct Ev {
    at: i32,
    fade_in: bool,
    pos: Option<i32>,
}

#[derive(Clone, Copy)]
pub struct Attenuator {
    /// +24: the curve's length; +32: the curve.
    len: i32,
    curve: [f32; MAX_LEN],
    /// +12: the place on the curve (0 silent, len - 1 whole); +8: 0 steady, 1 fading out,
    /// 2 fading in, 3 fading out from the plan.
    pos: i32,
    state: u8,
    /// +48 / +60: the plan.
    ev: [Ev; MAX_EV],
    nev: usize,
}

impl Attenuator {
    /// `Attenuator(rate, ms)` 0x102887cdc: steady at place 0 (silent).
    pub fn new(rate: f32, ms: f32) -> Attenuator {
        // (`createCurve`: (ms x 0.001) x rate in float, rounded to the nearest frame.)
        let len = (((ms * 0.001) * rate) as f64).round_ties_even().clamp(2.0, MAX_LEN as f64) as i32;
        let mut curve = [0.0f32; MAX_LEN];
        for (i, c) in curve.iter_mut().take(len as usize).enumerate() {
            *c = ((i as f64 * 3.14159265 / (len - 1) as f64 + -1.57079633).sin() * 0.5 + 0.5) as f32;
        }
        Attenuator { len, curve, pos: 0, state: 0, ev: [Ev::default(); MAX_EV], nev: 0 }
    }

    pub fn len(&self) -> i32 {
        self.len
    }

    /// The curve's value at place `i`.
    #[cfg(test)]
    pub fn curve(&self, i: i32) -> f32 {
        self.curve[i.clamp(0, self.len - 1) as usize]
    }

    fn push(&mut self, at: i32, fade_in: bool, pos: Option<i32>) {
        if self.nev < MAX_EV {
            self.ev[self.nev] = Ev { at, fade_in, pos };
            self.nev += 1;
        }
    }

    /// `letSmoothDiscontinuousPoints(points, n)`: `points` are the block's jumps (frames from its
    /// start, rising), `n` its length.
    pub fn smooth(&mut self, points: &[i32], n: i32) {
        let len = self.len;
        // (Only the points before twice the block count.)
        let mut cnt = points.len();
        loop {
            if cnt < 1 {
                return;
            }
            cnt -= 1;
            if points[cnt] < 2 * n {
                break;
            }
        }
        let cnt = cnt + 1;
        if cnt >= 2 && points[..cnt].windows(2).any(|w| w[1] - w[0] < 2 * len) {
            // Two jumps closer than two fades: silent, fading in from the block's start.
            self.state = 2;
            self.pos = 0;
            return;
        }
        let mut st = self.state;
        if st & 1 != 0 {
            if st != 3 {
                return;
            }
            st = 0;
        }
        if len + n > points[0] {
            let (mut prev, mut cur) = (0, self.pos);
            let mut i = 0;
            loop {
                let p = points[i];
                let d = p - prev;
                if d >= 0 {
                    if d < cur {
                        // (Too near to fade out from here: from the place that reaches 0 there.)
                        self.push(0, false, Some(d));
                    } else if st != 0 {
                        // (Fading in: down from half way, or a whole curve before the point.)
                        if d + cur <= 2 * len {
                            self.push((p + cur) / 2, false, None);
                        } else {
                            self.push(p - len, false, None);
                        }
                    } else {
                        self.push(p - cur, false, None);
                    }
                    self.push(p, true, None);
                    cur = 0;
                    st = 2;
                    prev = p;
                }
                i += 1;
                if i == cnt || len + n <= points[i] {
                    break;
                }
            }
        }
        if self.nev != 0 {
            if self.ev[0].at != 0 {
                return;
            }
            self.state = if self.ev[0].fade_in { 2 } else { 3 };
        } else {
            self.state = 2;
        }
    }

    /// `apply`: the gains of the block's `g.len()` frames; the plan is used up.
    pub fn apply(&mut self, g: &mut [f32]) {
        let n = g.len() as i32;
        let lm = self.len - 1;
        self.pos = self.pos.clamp(0, lm);
        if self.state == 0 && self.nev == 0 {
            let v = self.curve[self.pos as usize];
            g.iter_mut().for_each(|x| *x = v);
            return;
        }
        let mut i = 0i32;
        let nev = self.nev;
        for k in 0..=nev {
            // (After the plan: the block's end, a fade-in place kept.)
            let e = if k < nev { self.ev[k] } else { Ev { at: n, fade_in: true, pos: None } };
            if i < e.at {
                let end = e.at.min(n);
                let fade_in = self.state & 2 != 0 && !(self.state & 1 != 0 && nev != 0);
                if fade_in {
                    let m = end.min(self.len + i - self.pos);
                    while i < m {
                        g[i as usize] = self.curve[self.pos as usize];
                        self.pos += 1;
                        i += 1;
                    }
                } else if self.state & 1 != 0 {
                    let m = end.min(self.pos + i);
                    while i < m {
                        g[i as usize] = self.curve[self.pos as usize];
                        self.pos -= 1;
                        i += 1;
                    }
                }
                // (The rest up to the next change: held where the fade ended, steady.)
                if end > i {
                    self.pos = self.pos.clamp(0, lm);
                    let v = self.curve[self.pos as usize];
                    g[i as usize..end as usize].iter_mut().for_each(|x| *x = v);
                    self.state = 0;
                    i = end;
                }
            }
            if i == e.at {
                if let Some(p) = e.pos.filter(|&p| p < self.len) {
                    self.pos = p;
                }
                if e.at < n {
                    self.state = if e.fade_in { 2 } else { 3 };
                }
            }
            if i >= n {
                break;
            }
        }
        self.nev = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(a: &mut Attenuator, points: &[i32], n: usize) -> Vec<f32> {
        let mut g = vec![0.0f32; n];
        a.smooth(points, n as i32);
        a.apply(&mut g);
        g
    }

    /// The curve (`createCurve`: 48 frames at 48 kHz, 0 .. 1), and a jump at frame 200 of a
    /// 480-frame block while steady and whole: a fade-out over the 47 frames before it (from
    /// the place it is at: frame 199 at curve place 1), a fade-in from it (frame 200 silent,
    /// 247 whole). Fading in when the plan starts, the fade-out starts a whole curve before
    /// the jump instead.
    #[test]
    fn a_jump_fades_out_into_it_and_in_from_it() {
        let mut a = Attenuator::new(48000.0, 1.0);
        assert_eq!(a.len(), 48);
        assert!(a.curve(0) == 0.0 && (a.curve(47) - 1.0).abs() < 1e-6 && (a.curve(24) - 0.5).abs() < 0.04);
        // Steady at the start (silent), then a first point: fades in from it.
        let g = run(&mut a, &[10], 480);
        assert!(g[..11].iter().all(|&v| v == 0.0) && g[11] == a.curve(1) && g[57] == a.curve(47) && g[479] == a.curve(47));
        let g = run(&mut a, &[200], 480);
        assert!(g[..154].iter().all(|&v| v == a.curve(47)));
        assert_eq!((g[154], g[198], g[199], g[200], g[201], g[247]), (a.curve(46), a.curve(2), a.curve(1), 0.0, a.curve(1), a.curve(47)));
        // A jump 10 frames into the next block, seen ahead: its fade-out starts in this one.
        let g = run(&mut a, &[490, 970], 480);
        assert_eq!((g[442], g[443], g[444], g[479]), (a.curve(47), a.curve(47), a.curve(46), a.curve(11)));
        let g = run(&mut a, &[10, 490], 480);
        assert_eq!((g[0], g[8], g[9], g[10], g[11], g[57]), (a.curve(10), a.curve(2), a.curve(1), 0.0, a.curve(1), a.curve(47)));
        assert_eq!((g[441], g[442], g[479]), (a.curve(47), a.curve(47), a.curve(10)));
        let g = run(&mut a, &[10], 480);
        assert_eq!((g[0], g[8], g[9], g[10], g[11]), (a.curve(9), a.curve(1), 0.0, 0.0, a.curve(1)));
        // Two jumps closer than two fades: silent from the block's start, fading in.
        let g = run(&mut a, &[100, 150], 480);
        assert_eq!((g[0], g[1], g[47]), (0.0, a.curve(1), a.curve(47)));
    }
}
