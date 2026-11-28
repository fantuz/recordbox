//! Rekordbox 7.2.19's 29-tap sample-rate converter for the optional beat model.
//! Its 28-frame history and 14-frame group delay are part of the model's input timing.

pub const TAPS: usize = 29;

pub struct Src {
    pub l: usize,
    pub m: usize,
    pub idx: Vec<usize>,
    pub taps: Vec<[f32; TAPS]>,
}

/// (fc, β) of `SamplingFunctionSRC<Float2>::init` for a conversion.
pub fn params(inr: u32, outr: u32) -> (f64, f64) {
    let c = f64::from_bits;
    match (inr, outr) {
        (44100, 48000) => (c(0x3FE147AE147AE148), 8.0),
        (44100, 96000) | (44100, 88200) => (c(0x3FE28F5C28F5C28F), 10.0),
        (48000, 44100) => (c(0x3FE0A3D70A3D70A4), 8.0),
        (48000, 96000) => (c(0x3FDCCCCCCCCCCCCD), 10.0),
        (96000, 44100) | (88200, 44100) => (c(0x3FE199999999999A), 6.0),
        (96000, 48000) => (c(0x3FDE147AE147AE14), 10.0),
        (44100, 176400) => (c(0x3FDEB851EB851EB8), 9.0),
        (44100, 192000) => (0.48, 9.0),
        _ => (c(0x3FDCCCCCCCCCCCCD), 12.0),
    }
}

/// The converter for `inr` → `outr` Hz; None where rekordbox's refuses (rates outside
/// 8000 … 200,000 Hz or not multiples of 50).
pub fn new(inr: u32, outr: u32) -> Option<Src> {
    if !(8000..=200_000).contains(&inr) || !(8000..=200_000).contains(&outr) || inr % 50 != 0 || outr % 50 != 0 {
        return None;
    }
    let (fc, beta) = params(inr, outr);
    let l = (outr / 50) as usize;
    let m = (inr / 50) as usize;
    let fcn = fc / l.max(m) as f64;
    let n = TAPS * l;
    let half = -((if (n as i32) < 0 { n as i32 + 1 } else { n as i32 }) >> 1);
    let two_pi_fc = fcn * f64::from_bits(0x401921fb54442d18);
    let pi = f64::from_bits(0x400921fb54442d18);
    let step = {
        let r = 1.0 / (n as f64 - 1.0);
        r + r
    };
    let hb = beta * 0.5;
    // 1 / (k!)², k = 0 … 21 (exact factorials).
    let inv_f2: Vec<f64> = (0..22)
        .map(|k: u32| {
            let mut f = 1.0f64;
            for i in 2..=k {
                f *= i as f64;
            }
            1.0 / (f * f)
        })
        .collect();
    let mut ir = vec![0f64; n];
    for (i, v) in ir.iter_mut().enumerate() {
        let t = half + i as i32;
        let sinc = if t == 0 { fcn + fcn } else { (two_pi_fc * t as f64).sin() / (t as f64 * pi) };
        let u = i as f64 * step;
        let um1 = u + -1.0;
        let sq = u.mul_add(um1, -um1);
        let x = (1.0 - sq).sqrt() * hb;
        let (mut ix, mut ib) = (0f64, 0f64);
        let mut e = 0f64;
        for k in 0..22 {
            let p = e + e;
            ix = inv_f2[k].mul_add(x.powf(p), ix);
            ib = inv_f2[k].mul_add(hb.powf(p), ib);
            e += 1.0;
        }
        *v = ix * sinc / ib;
    }
    let idx: Vec<usize> = (0..l).map(|p| m * p / l).collect();
    let lf = l as f64;
    let taps = (0..l)
        .map(|p| {
            let mut h = [0f32; TAPS];
            for (j, t) in h.iter_mut().enumerate() {
                let k = l + l * (j + idx[p]) - m * p - 1;
                *t = (ir[k] * lf * 1.0) as f32;
            }
            h
        })
        .collect();
    Some(Src { l, m, idx, taps })
}

impl Src {
    /// Frames out for `n_in` frames in (`calcOutputNum` on a fresh converter): every k with
    /// ⌊kM/L⌋ < n_in.
    pub fn out_len(&self, n_in: usize) -> usize {
        // Keep the intermediate bounded even on 32-bit targets.
        (n_in / self.m) * self.l + ((n_in % self.m) * self.l).div_ceil(self.m)
    }

    /// Converts interleaved stereo (as the reader's floats) from the start of a track.
    pub fn run(&self, x: &[f32]) -> Vec<f32> {
        let n_in = x.len() / 2;
        let n_out = self.out_len(n_in);
        let mut out = vec![0f32; 2 * n_out];
        let at = |n: i64, c: usize| -> f32 {
            if n < 0 || n as usize >= n_in {
                0.0
            } else {
                x[2 * n as usize + c]
            }
        };
        for k in 0..n_out {
            let p = k % self.l;
            let base = ((k / self.l) * self.m + self.idx[p]) as i64 - 28;
            let h = &self.taps[p];
            for c in 0..2 {
                let mut acc = 0f32;
                for j in 0..14 {
                    acc = h[j].mul_add(at(base + j as i64, c), acc);
                    acc = h[28 - j].mul_add(at(base + 28 - j as i64, c), acc);
                }
                acc = h[14].mul_add(at(base + 14, c), acc);
                out[2 * k + c] = acc;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converted_stereo_impulses_follow_the_reader_delay_and_length() {
        let converter = new(96_000, 48_000).unwrap();
        let mut mono = vec![0f32; 129 * 2];
        mono[2 * 32] = 1.0;
        mono[2 * 92 + 1] = -0.5;
        let out = converter.run(&mono);
        assert_eq!(out.len(), 65 * 2);
        let peak = |ch: usize| (0..65).max_by(|&a, &b|
            out[2*a + ch].abs().total_cmp(&out[2*b + ch].abs())).unwrap();
        assert_eq!((peak(0), peak(1)), (23, 53));
        assert_eq!(out[2*53+1], -0.5 * out[2*23]);
        assert!(out[..16].iter().all(|&x| x == 0.0));
        assert!(new(11_025, 44_100).is_none());
    }
}
