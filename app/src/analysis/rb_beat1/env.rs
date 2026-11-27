//! Stage 3 of rekordbox's v1 beat library: `UnitEnvelopeMaker`. A low-band envelope at
//! sr/128 decides whether the coarse grid sits on the off-beat (then it moves by half a beat)
//! and which beat of the bar is the "1".
//!
//! Ported from rekordbox 7.2.19 (arm64): `create` 0x101137f5c, `putSample` 0x101138168,
//! `analyze` 0x101138480.

use super::sensor::{Biquad, Hp1d};
use super::BeatInfo;

const BLOCK: i32 = 128;

pub(super) struct Uem {
    total: i32,
    sr: i32,
    consumed: i32,
    filled: i32,
    buf: Vec<f64>,
    t40: Vec<f64>,
    t48: Vec<f64>,
    lp1: Biquad,
    lp2: Biquad,
    lp5: Biquad,
    hp: Hp1d,
    /// +88: the envelope, one value per 128 samples.
    env: Vec<i16>,
    /// +96: peak values at their positions.
    pk: Vec<i16>,
}

/// Index of the last element `<= x` (-1 if none).
fn upper(v: &[i32], x: i32) -> i32 {
    let mut lo = 0i32;
    let mut hi = v.len() as i32 - 1;
    while hi >= lo {
        let mid = lo + (hi - lo) / 2;
        if v[mid as usize] <= x {
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    hi
}

/// Nearest grid index by midpoints (may return -1; the callers clamp to 0).
fn nearest(v: &[i32], t: i32) -> i32 {
    let n = v.len() as i32;
    let mut lo = 0i32;
    let mut hi = n - 1;
    if n < 1 {
        return hi;
    }
    while hi >= lo {
        let mid = lo + (hi - lo) / 2;
        if mid != 0 {
            if t < (v[(mid - 1) as usize] + v[mid as usize]) / 2 {
                hi = mid - 1;
            } else {
                lo = mid + 1;
            }
        } else if v[0] > t {
            hi = -1;
        } else {
            lo = mid + 1;
        }
    }
    hi
}

impl Uem {
    pub(super) fn new(sr: i32, total: i32) -> Uem {
        let n = ((total + BLOCK - 1) / BLOCK).max(0) as usize;
        let rate = (sr as f64) / (BLOCK as f64);
        Uem {
            total,
            sr,
            consumed: 0,
            filled: 0,
            buf: vec![0.0; BLOCK as usize],
            t40: vec![0.0; BLOCK as usize],
            t48: vec![0.0; BLOCK as usize],
            lp1: Biquad::lowpass2d(150.0, sr as f64),
            lp2: Biquad::lowpass2d(150.0, sr as f64),
            lp5: Biquad::lowpass2d(5.0, rate),
            hp: Hp1d::new(0.5, rate),
            env: vec![0; n],
            pk: vec![0; n],
        }
    }

    /// `putSample(short*, n)` with the analyser's interleaved 16-bit block.
    pub(super) fn put(&mut self, s: &[i16]) {
        let n = (s.len() / 2) as i32;
        let mut done = 0i32;
        while done < n {
            let avail = n - done + self.filled;
            let upto = BLOCK.min(avail);
            let cnt = upto - self.filled;
            for i in 0..cnt.max(0) {
                let k = (done + i) as usize;
                self.buf[(self.filled + i) as usize] = ((s[2 * k + 1] as f64) + (s[2 * k] as f64)) * 0.5;
            }
            self.consumed += cnt;
            self.filled = upto;
            if BLOCK <= avail || self.consumed >= self.total {
                let m = upto.max(0) as usize;
                for i in 0..m {
                    self.t40[i] = self.lp1.step(self.buf[i]);
                }
                for i in 0..(self.filled.max(0) as usize) {
                    self.t48[i] = self.lp2.step(self.t40[i]);
                }
                let x = self.t48[0].abs();
                self.t48[0] = x;
                let y = self.lp5.step(x);
                let z = self.hp.step(y);
                let at = ((self.consumed - self.filled) / BLOCK) as usize;
                if at < self.env.len() {
                    self.env[at] = (z as i32) as i16;
                }
                self.filled = 0;
            }
            done += cnt;
            if self.consumed >= self.total {
                return;
            }
        }
    }

    /// `UnitEnvelopeMaker::analyze(bpm, list, varied)` (0x101138480).
    pub(super) fn analyze(&mut self, bpm: f32, v: &mut Vec<BeatInfo>, varied: bool) {
        if !(bpm > 0.0) {
            return;
        }
        let n = v.len();
        if n < 2 {
            return;
        }
        let blk = BLOCK as f64;
        let inv = 1.0 / blk;
        let srf = self.sr as f64;
        let v15 = (v[0].pos as f64) * inv;
        let v19 = ((srf * f64::from_bits(0x3f4d_7dbf_487f_cb93)) * (v[0].iv as f64)) * inv;
        let per = |b: &BeatInfo| ((b.iv as f64) * 0.001) * srf / blk;
        let gi = |b: &BeatInfo| ((b.pos as f64) / blk + 0.5) as i32;
        let mut grid: Vec<i32> = Vec::with_capacity(n);
        let mut gper: Vec<f64> = Vec::with_capacity(n);
        let prepend = !(v15 <= v19);
        if prepend {
            grid.push(1);
            gper.push(per(&v[0]));
            for b in &v[..n - 1] {
                grid.push(gi(b));
                gper.push(per(b));
            }
        } else {
            for b in v.iter() {
                grid.push(gi(b));
                gper.push(per(b));
            }
        }
        let nenv = (self.total + BLOCK - 1) / BLOCK;
        if grid[0] < 1 {
            grid[0] = 1;
        }
        let g0 = grid[0];
        let per_at = |grid: &[i32], x: i32| -> f64 {
            if gper.is_empty() || grid.is_empty() {
                return 0.0;
            }
            let i = upper(grid, x).max(0);
            gper[i as usize]
        };
        // Peaks of the envelope.
        let mut peaks: Vec<i32> = vec![];
        let (mut n3, mut n15) = (0i32, 0i32);
        let mut l3: Vec<i32> = vec![];
        let mut l15: Vec<i32> = vec![];
        if nenv > 1 {
            let env = &self.env;
            let mut cur: i32 = 0; // v84 (16-bit)
            let mut at: i32 = 0; // v85
            let mut inpk = false;
            let mut sum: u64 = 0;
            let mut cnt: u32 = 0;
            for i in 0..(nenv - 1) {
                let nx = env[(i + 1) as usize] as i32;
                let (val, end): (i32, bool);
                if inpk {
                    let c = cur as i16 as i32;
                    let ge = c >= nx;
                    cur = if c > nx { c } else { nx };
                    if !ge {
                        at = i + 1;
                    }
                    val = cur as i16 as i32;
                    end = nx == 0 || (val as f64) / (nx as f64) > 1.5;
                } else {
                    let here = env[i as usize] as i32;
                    let c = if here > nx { nx } else { cur };
                    let c = c as i16 as i32;
                    if c >= nx {
                        cur = c;
                        inpk = false;
                        continue;
                    }
                    cur = nx;
                    at = i + 1;
                    val = cur as i16 as i32;
                    end = nx == 0 || (val as f64) / (nx as f64) > 1.5;
                }
                if end {
                    inpk = false;
                    self.pk[at as usize] = cur as i16;
                    let w = val.max(0);
                    sum += (w as u16) as u64;
                    if val > 0 {
                        cnt += 1;
                    }
                    cur = (env[(i + 1) as usize] as u16) as i32;
                } else {
                    inpk = true;
                }
            }
            let avg: i16 = if cnt == 0 { 0 } else { ((sum as i64) / (cnt as i64)) as i16 };
            for i in 1..nenv {
                if avg > self.pk[i as usize] {
                    self.pk[i as usize] = 0;
                } else {
                    peaks.push(i);
                }
            }
            // Gap patterns between consecutive peaks.
            let np = peaks.len() as i32;
            if np >= 3 {
                for k in 0..(np - 2) as usize {
                    let p0 = peaks[k];
                    let g1 = peaks[k + 1] - p0;
                    let t = per_at(&grid, p0);
                    let g2 = peaks[k + 2] - peaks[k + 1];
                    let ti = |c: f64| (t * c) as i32;
                    if g1 > ti(2.25) && g1 <= ti(2.75) {
                        let c135 = ti(f64::from_bits(0x3ff5_9999_9999_999a));
                        let c165 = ti(f64::from_bits(0x3ffa_6666_6666_6666));
                        if g2 > c135 && g2 <= c165 {
                            n3 += 1;
                            l3.push(p0);
                        } else if g2 > ti(f64::from_bits(0x3ffc_cccc_cccc_cccd)) && g2 <= ti(f64::from_bits(0x4001_9999_9999_999a)) {
                            n3 += 1;
                            l3.push(p0);
                        }
                    }
                    let c135 = ti(f64::from_bits(0x3ff5_9999_9999_999a));
                    let c165 = ti(f64::from_bits(0x3ffa_6666_6666_6666));
                    if g1 > c135 && g1 <= c165 {
                        n15 += 1;
                        l15.push(p0);
                    } else {
                        let c09 = ti(f64::from_bits(0x3fec_cccc_cccc_cccd));
                        if g1 > c09 {
                            let c11 = ti(f64::from_bits(0x3ff1_9999_9999_999a));
                            if g1 <= c11 && g2 > c09 && g2 <= c11 {
                                n15 -= 1;
                            }
                        }
                    }
                }
            }
        }
        let v157 = (srf * f64::from_bits(0x3fbc_28f5_c28f_5c29)).mul_add(inv, 0.5) as i32;
        let mut shifted = false;
        if !varied {
            let v160 = (srf * 60.0) / (blk * (bpm as f64));
            let list: &[i32] = if n3 >= 11 {
                &l3
            } else if n15 >= 81 {
                &l15
            } else {
                &peaks
            };
            let v161 = list.len() as i32;
            let mut off = 0i32;
            if v161 >= 1 && !grid.is_empty() {
                let lim = (v160 * 0.25) as i32;
                for p in list {
                    let t = p - v157;
                    let j = nearest(&grid, t).max(0);
                    let d = (grid[j as usize] - t).wrapping_abs();
                    if d > lim {
                        off += 1;
                    }
                }
            }
            if off > v161 / 2 {
                let half = (v160 * 0.5 + 0.5) as i32;
                if v160 * f64::from_bits(0x3fdc_cccc_cccc_cccd) + 0.5 >= g0 as f64 {
                    for g in grid.iter_mut() {
                        *g += half;
                    }
                } else {
                    for g in grid.iter_mut() {
                        *g -= half;
                    }
                }
                shifted = true;
                if grid[0] <= 0 {
                    grid[0] = 1;
                }
            }
        }
        // Bar phase votes.
        let mut votes = [0f64; 4];
        let p0 = peaks.first().copied().unwrap_or(0);
        if !peaks.is_empty() {
            let v211 = p0.min(v157);
            let t = per_at(&grid, p0);
            let v220 = t.mul_add(f64::from_bits(0x3fec_cccc_cccc_cccd), 0.5) as i32;
            let v221 = t.mul_add(f64::from_bits(0x3ff1_9999_9999_999a), 0.5) as i32;
            let v225 = t.mul_add(f64::from_bits(0x3ffc_cccc_cccc_cccd), 0.5) as i32;
            let first_vote = |votes: &mut [f64; 4], grid: &[i32]| {
                if grid.is_empty() {
                    return;
                }
                let j = nearest(grid, p0 - v211).max(0);
                votes[(j & 3) as usize] = (self.pk[p0 as usize] as f64) + (self.pk[p0 as usize] as f64);
            };
            let v224 = if peaks.len() >= 2 { peaks[1] - p0 } else { 0 };
            if v224 >= v220 && v224 <= v221 {
                first_vote(&mut votes, &grid);
            }
            if v224 >= v225 {
                let v22b = t.mul_add(f64::from_bits(0x4001_9999_9999_999a), 0.5) as i32;
                if v224 <= v22b {
                    first_vote(&mut votes, &grid);
                }
            }
            let v244 = if peaks.len() >= 3 { peaks[2] - p0 } else { 0 };
            if v244 >= v220 && v244 <= v221 {
                first_vote(&mut votes, &grid);
            }
            let mut mx = self.pk[p0 as usize] as f64;
            for p in &peaks[1..] {
                mx = mx.max(self.pk[*p as usize] as f64);
            }
            if peaks.len() > 2 {
                let strong = mx * f64::from_bits(0x3fe3_3333_3333_3333);
                let last = peaks.len() - 1;
                let mut k = 1usize;
                while k != last {
                    let pk = peaks[k];
                    let t = per_at(&grid, pk);
                    if !((pk as f64) - (peaks[k - 1] as f64) > t * 7.0) {
                        k += 1;
                        continue;
                    }
                    let hi = t.mul_add(f64::from_bits(0x4001_9999_9999_999a), 0.5) as i32;
                    let lo = t.mul_add(f64::from_bits(0x3ffc_cccc_cccc_cccd), 0.5) as i32;
                    k += 1;
                    let g = peaks[k] - pk;
                    let val = self.pk[pk as usize] as f64;
                    if g >= lo && g <= hi {
                        if !grid.is_empty() {
                            let j = nearest(&grid, pk - v157).max(0);
                            votes[(j & 3) as usize] += val;
                        }
                    } else {
                        if strong >= val || grid.is_empty() {
                            continue;
                        }
                        let j = nearest(&grid, pk - v157).max(0);
                        votes[(j & 3) as usize] += val;
                    }
                }
            }
        }
        let extra: Option<&[i32]> = if n3 < 11 { (n15 >= 81 && !l15.is_empty()).then_some(&l15[..]) } else { Some(&l3[..]) };
        if let Some(l) = extra {
            for p in l {
                if !grid.is_empty() {
                    let j = nearest(&grid, p - v157).max(0);
                    votes[(j & 3) as usize] += 983.04;
                }
            }
        }
        let a = votes[0].max(0.0);
        let mut best = if a < votes[1] { 1 } else { 0 };
        let a = if a < votes[1] { votes[1] } else { a };
        let lt2 = a < votes[2];
        let b = a.max(votes[2]);
        if lt2 {
            best = 2;
        }
        let c = b.max(votes[3]);
        let mut v320 = if b >= votes[3] { best } else { 3 };
        if c == 0.0 && !grid.is_empty() {
            let j = nearest(&grid, p0).max(0);
            v320 = j & 3;
        }
        // Apply: shift and beat numbers.
        let mut v327 = 4 - v320;
        let mut v328 = 0i32;
        if shifted {
            if v15 > v19 {
                v328 = BLOCK * grid[2] - v[1].pos;
                v327 = (1 - v320) & 3;
            } else {
                v328 = BLOCK * grid[1] - v[1].pos;
            }
        } else if v15 > v19 {
            v327 = (1 - v320) & 3;
        }
        for (i, b) in v.iter_mut().enumerate().take(n) {
            b.pos = b.pos.wrapping_add(v328);
            b.stat = ((v327 + i as i32) & 3) + 1;
        }
        // Ends: drop a last beat past the end or add one more; drop a first beat before 0 or
        // add one before it.
        let last = n - 1;
        let total = self.total;
        let pos = v[last].pos;
        if pos >= total {
            v.pop();
        } else {
            let np = pos + ((v[last].iv as f64).mul_add(0.001 * srf, 0.5) as i32);
            if np < total {
                let mut nb = v[last];
                let st = nb.stat;
                let m = if -st < 0 { st & 3 } else { -((-st) & 3) };
                nb.pos = np;
                nb.stat = m + 1;
                v.push(nb);
            }
        }
        if v.is_empty() {
            return;
        }
        if v[0].pos < 0 {
            v.remove(0);
        } else {
            let k = (self.sr as f64) * 0.001;
            let d = k.mul_add(v[0].iv as f64, 0.5) as i32;
            if v[0].pos >= d {
                let mut nb = v[0];
                let st = nb.stat + 2;
                let m = if -st < 0 { st & 3 } else { -((-st) & 3) };
                nb.pos = v[0].pos - d;
                nb.stat = m + 1;
                v.insert(0, nb);
            }
        }
    }
}
