//! Stage 4 of rekordbox's v1 beat library (Analysis Mode Normal only): `UnitBeatAdjust`.
//! A 2–1 kHz band envelope at sr/4 is folded on candidate grids; the tempo whose fold has the
//! sharpest peak wins (0.002 BPM steps around the coarse tempo), then the grid moves to the
//! fold's peak.
//!
//! Ported from rekordbox 7.2.19 (arm64): `create` 0x1011330c0, `putSample` 0x101133300,
//! `analyze` 0x101133528, `bpmAdjust` 0x1011335f0, `createBeatData` 0x101134c14, `manageWave`
//! 0x101135098, `reflectBeatData` 0x101134948, `beatPositionAdjust` 0x1011344b8,
//! `offsetBeatPosition` 0x1011353dc.

use super::sensor::Biquad;
use super::BeatInfo;

pub(super) struct Uba {
    total: i32,      // +8
    sr4: i32,        // +12
    count: i32,      // +16 decimated samples taken
    ctr: i32,        // +24 candidate counter
    maxlen: i32,     // +28
    n: i32,          // +32
    acc: Vec<f64>,   // +40
    cands: Vec<Vec<i32>>, // +48
    wave: Vec<i16>,  // +72
    hp: Biquad,      // +96
    lp: Biquad,      // +104
}

impl Uba {
    pub(super) fn new(sr: i32, total: i32) -> Uba {
        let sr4 = sr >> 2;
        let maxlen = (total / 4).min(900 * (sr / 4));
        let w = ((sr4 as f64) * 0.05) as i32;
        Uba {
            total,
            sr4,
            count: 0,
            ctr: 0,
            maxlen,
            n: 0,
            acc: vec![0.0; (2 * w).max(1) as usize],
            cands: vec![],
            wave: Vec::with_capacity(maxlen.max(0) as usize),
            hp: Biquad::highpass2d(2000.0, sr4 as f64),
            lp: Biquad::lowpass2d(1000.0, sr4 as f64),
        }
    }

    /// `putSample(short*, n)`: every fourth frame of the analyser's interleaved 16-bit block.
    pub(super) fn put(&mut self, s: &[i16]) {
        let n = (s.len() / 2) as i32;
        if n < 1 {
            return;
        }
        let v17 = 4 * self.count;
        let len = self.wave.len() as i32;
        let v21 = if v17 + n > self.total { self.total - v17 } else { n };
        let q = if v21 < 0 { v21 + 3 } else { v21 } >> 2;
        let v23 = 900 * self.sr4;
        let v25 = if len + q > v23 { 4 * (v23 - len) } else { v21 };
        let mut i = 0;
        while i < v25 {
            let k = i as usize;
            let x = ((s[2 * k + 1] as f64) + (s[2 * k] as f64)) * 0.5;
            let y = self.hp.step(x).abs();
            let z = self.lp.step(y);
            let v = if z > 32767.0 {
                0x7fff
            } else if !(z >= 0.0) {
                0
            } else {
                z as i32 as i16
            };
            self.wave.push(v);
            self.count += 1;
            i += 4;
        }
    }

    fn half_window(&self) -> i32 {
        ((self.sr4 as f64) * 0.05) as i32
    }

    /// `UnitBeatAdjust::analyze(bpm&, list)` (0x101133528).
    pub(super) fn analyze(&mut self, bpm: &mut f32, v: &mut Vec<BeatInfo>) {
        if !(*bpm > 0.0) || v.is_empty() {
            return;
        }
        let n = v.len() as i32;
        self.n = n;
        let mut a = n;
        let v10 = 900 * self.sr4;
        if !(v10 > self.wave.len() as i32) {
            let span = (v[0].pos as f64).mul_add(-0.25, v10 as f64);
            a = ((((*bpm as f64) * f64::from_bits(0x3f91_1111_1111_1111)) * span) / (self.sr4 as f64)) as i32;
        }
        self.bpm_adjust(bpm, v, a);
        self.beat_position_adjust(v);
    }

    /// `createBeatData(bpm, start)` + `manageWave(a, ctr, &slot)`. `slot` is the caller's
    /// stack variable, zeroed once per search stage: `manageWave` leaves it alone when it does
    /// nothing.
    fn eval(&mut self, bpm: f64, start: i32, a: i32, slot: &mut f64) -> f64 {
        self.create_beat_data(bpm, start);
        let ctr = self.ctr;
        self.manage_wave(a, ctr, slot);
        *slot
    }

    fn create_beat_data(&mut self, d: f64, a: i32) {
        if !(d > 0.0) || a < 0 || !(self.maxlen > a) || self.n < 1 {
            return;
        }
        let per = ((self.sr4 as f64) * 60.0) / d;
        let lim = (self.total / 4) as f64;
        let mut out: Vec<i32> = Vec::with_capacity(self.n as usize);
        let mut x = 0.0f64;
        for k in 0..self.n {
            x = per.mul_add(k as f64, a as f64);
            if x < lim {
                out.push((x + 0.5) as i32);
            }
        }
        let mut y = x + per;
        while y < lim {
            out.push((y + 0.5) as i32);
            y += per;
        }
        self.cands.push(out);
    }

    fn manage_wave(&mut self, a: i32, b: i32, d: &mut f64) {
        if a < 1 || b < 0 || (b as usize) >= self.cands.len() {
            return;
        }
        let w = self.half_window();
        let beats = &self.cands[b as usize];
        let m = (beats.len() as i32).min(a);
        let w2 = 2 * w;
        if w >= 1 {
            for x in self.acc.iter_mut().take(w2.max(1) as usize) {
                *x = 0.0;
            }
        }
        if m >= 3 {
            for k in 1..(m - 1) as usize {
                let p = beats[k];
                let s = p - w;
                if s >= 0 {
                    let e = p + w;
                    if e > s && self.maxlen >= e && (e - s) <= w2 {
                        let len = e - s;
                        if len >= 1 && self.wave.len() as i32 >= len {
                            for i in 0..len as usize {
                                self.acc[i] += self.wave[s as usize + i] as f64;
                            }
                        }
                    }
                }
            }
        }
        if w >= 1 {
            let mut mx = 0.0f64;
            for x in self.acc.iter().take(w2 as usize) {
                mx = x.max(mx);
            }
            *d = mx;
        }
    }

    /// `bpmAdjust(bpm&, list, a)` (0x1011335f0): 17 tempi ±0.016 BPM around the coarse tempo,
    /// extended by 9 (and then 8) more on the side where the best one sits at the edge, then
    /// ±0.001 around the best.
    fn bpm_adjust(&mut self, bpm: &mut f32, v: &mut Vec<BeatInfo>, a: i32) {
        if a < 1 {
            return;
        }
        let b = *bpm;
        if !(b > f32::from_bits(0x3d4c_cccd)) || v.is_empty() {
            return;
        }
        let pos0 = v[0].pos;
        let start = (if pos0 < 0 { pos0 + 3 } else { pos0 }) >> 2;
        let clamp = |x: f64| x.max(70.0).min(180.0);
        let step = f64::from_bits(0x3f60_624d_d2f1_a9fc); // 0.002
        let mut best = 0.0f64; // d10
        let mut best_bpm = 0.0f64; // d9
        let mut idx: i32 = 0; // w24
        let mut slot: f64;
        let ev = |this: &mut Uba, cand: f64, best: &mut f64, best_bpm: &mut f64, idx: &mut i32, slot: &mut f64| {
            let s = this.eval(cand, start, a, slot);
            if !(*best >= s) {
                *idx = this.ctr;
                *best_bpm = cand;
            }
            this.ctr += 1;
            *best = best.max(s);
        };
        let base = (b as f64) + f64::from_bits(0xbf90_624d_d2f1_a9fc); // -0.016
        let mut low_ext = false;
        if base > 0.0 && pos0 >= -3 && self.maxlen > start {
            slot = 0.0;
            for k in 0..17 {
                let cand = clamp((k as f64).mul_add(step, base));
                ev(self, cand, &mut best, &mut best_bpm, &mut idx, &mut slot);
            }
            if (idx as u32) <= 2 {
                low_ext = pos0 >= -3;
            } else if ((idx - 15) as u32) <= 2 {
                // High side.
                let base2 = ((*bpm) as f64) + f64::from_bits(0x3f92_6e97_8d4f_df3c); // +0.018
                if base2 > 0.0 && self.maxlen > start {
                    slot = 0.0;
                    for k in 0..9 {
                        let cand = clamp((k as f64).mul_add(step, base2));
                        ev(self, cand, &mut best, &mut best_bpm, &mut idx, &mut slot);
                    }
                }
                if ((idx - 24) as u32) <= 2 {
                    let bb = (*bpm) as f64;
                    let offs: [u64; 8] = [
                        0x3fa2_6e97_8d4f_df3c,
                        0x3fa3_74bc_6a7e_f9dc,
                        0x3fa4_7ae1_47ae_147c,
                        0x3fa5_8106_24dd_2f1b,
                        0x3fa6_872b_020c_49bb,
                        0x3fa7_8d4f_df3b_645b,
                        0x3fa8_9374_bc6a_7efa,
                        0x3fa9_9999_9999_999a,
                    ];
                    if bb + f64::from_bits(offs[0]) > 0.0 && self.maxlen > start {
                        slot = 0.0;
                        for o in offs {
                            let cand = clamp(bb + f64::from_bits(o));
                            ev(self, cand, &mut best, &mut best_bpm, &mut idx, &mut slot);
                        }
                    }
                }
            }
        } else {
            low_ext = pos0 >= -3;
        }
        if low_ext {
            let base2 = ((*bpm) as f64) + f64::from_bits(0xbfa1_6872_b020_c49c); // -0.034
            if base2 > 0.0 && self.maxlen > start {
                slot = 0.0;
                for k in 0..9 {
                    let cand = clamp((k as f64).mul_add(step, base2));
                    ev(self, cand, &mut best, &mut best_bpm, &mut idx, &mut slot);
                }
            }
        }
        if low_ext && ((idx - 17) as u32) <= 2 && pos0 >= -3 {
            let bb = (*bpm) as f64;
            let offs: [u64; 8] = [
                0xbfa9_9999_9999_999a,
                0xbfa8_9374_bc6a_7efa,
                0xbfa7_8d4f_df3b_645a,
                0xbfa6_872b_020c_49bb,
                0xbfa5_8106_24dd_2f1b,
                0xbfa4_7ae1_47ae_147b,
                0xbfa3_74bc_6a7e_f9dc,
                0xbfa2_6e97_8d4f_df3c,
            ];
            if bb + f64::from_bits(offs[0]) > 0.0 && self.maxlen > start {
                slot = 0.0;
                for o in offs {
                    let cand = clamp(bb + f64::from_bits(o));
                    ev(self, cand, &mut best, &mut best_bpm, &mut idx, &mut slot);
                }
            }
        }
        // ±0.001 around the best.
        let w26 = (best_bpm * 1000.0) as i32;
        let w27 = ((*bpm as f64) * 1000.0) as i32;
        let mut fin = best_bpm;
        let mut try_up = true;
        if w27 - 50 < w26 && pos0 >= -3 {
            let c = best_bpm + f64::from_bits(0xbf50_624d_d2f1_a9fc);
            if c > 0.0 && self.maxlen > start {
                let cand = clamp(c);
                slot = 0.0;
                let s = self.eval(cand, start, a, &mut slot);
                if !(best >= s) {
                    idx = self.ctr;
                    fin = cand;
                } else {
                    fin = best_bpm;
                }
                self.ctr += 1;
                best = best.max(s);
            }
        }
        if !(w27 + 50 > w26 && pos0 >= -3) {
            try_up = false;
        }
        if try_up {
            let c = best_bpm + f64::from_bits(0x3f50_624d_d2f1_a9fc);
            if c > 0.0 && self.maxlen > start {
                let cand = clamp(c);
                slot = 0.0;
                let s = self.eval(cand, start, a, &mut slot);
                if !(best >= s) {
                    idx = self.ctr;
                    fin = cand;
                }
                self.ctr += 1;
            }
        }
        self.reflect_beat_data(fin, bpm, v, idx);
    }

    fn reflect_beat_data(&mut self, d: f64, bpm: &mut f32, v: &mut Vec<BeatInfo>, a: i32) {
        if !(d > 0.0) || a < 0 || (a as usize) >= self.cands.len() || !(*bpm > 0.0) || v.is_empty() {
            return;
        }
        if (((*bpm) as f64) * 1000.0) as i32 == (d * 1000.0) as i32 {
            return;
        }
        let cand = &self.cands[a as usize];
        let m = cand.len() as i32;
        while self.n != m {
            if self.n > m {
                v.pop();
                self.n -= 1;
                continue;
            }
            let mut nb = v[(self.n - 1) as usize];
            nb.stat = if nb.stat == 4 { 1 } else { nb.stat + 1 };
            nb.flag = 0;
            v.push(nb);
            self.n += 1;
        }
        let iv = (60000.0 / d) as f32;
        for (b, p) in v.iter_mut().zip(cand.iter()) {
            b.pos = 4 * p;
            b.iv = iv;
        }
        *bpm = d as f32;
    }

    fn beat_position_adjust(&mut self, v: &mut Vec<BeatInfo>) {
        if v.is_empty() {
            return;
        }
        let n = v.len() as i32;
        let w = self.half_window();
        let w2 = 2 * w;
        if w >= 1 {
            for x in self.acc.iter_mut().take(w2.max(1) as usize) {
                *x = 0.0;
            }
        }
        if n >= 3 {
            for k in 1..(n - 1) as usize {
                let p = v[k].pos / 4;
                let s = p - w;
                if s >= 0 {
                    let e = p + w;
                    if e > s && self.maxlen >= e && (e - s) <= w2 {
                        let len = e - s;
                        if len >= 1 && self.wave.len() as i32 >= len {
                            for i in 0..len as usize {
                                self.acc[i] += self.wave[s as usize + i] as f64;
                            }
                        }
                    }
                }
            }
        }
        let mut at = 0i32;
        if w >= 1 {
            let mut mx = 0.0f64;
            for (i, x) in self.acc.iter().take(w2 as usize).enumerate() {
                if *x > mx {
                    at = i as i32;
                }
                mx = x.max(mx);
            }
        }
        self.offset_beat_position(w, at, v);
    }

    fn offset_beat_position(&mut self, a: i32, b: i32, v: &mut Vec<BeatInfo>) {
        if a < 0 {
            return;
        }
        let w2 = 2 * (((self.sr4 as f64) * 0.05) as i32);
        if w2 < b || b < 0 || w2 < a || v.is_empty() {
            return;
        }
        let mut n = v.len() as i32;
        let v10 = ((self.sr4 as f64) * -0.001) as i32;
        let v11 = ((self.sr4 as f32) * (f32::from_bits(0x3b83_126f) * v[0].iv)) as i32;
        let v12 = b - a + v10;
        let mut v13 = 4 * (b - a);
        let v14 = v13 + 20 * v10;
        if v[0].pos + v13 < v11 || v14 >= 1 {
            v13 = 4 * v12;
        }
        if n >= 1 {
            let total = self.total;
            let mut neg = 0u32;
            let mut over = 0i32;
            for bi in v.iter_mut() {
                let p = bi.pos.wrapping_add(v13);
                bi.pos = p;
                neg = neg.wrapping_add((p as u32) >> 31);
                if p.wrapping_add(v13) > total {
                    over += 1;
                }
            }
            if neg != 0 {
                let k = (neg as usize).min(v.len());
                v.drain(..k);
                n -= neg as i32;
            }
            if over != 0 {
                n -= over;
                let keep = v.len().saturating_sub(over as usize);
                v.truncate(keep);
            }
        }
        if v.is_empty() {
            self.n = 0;
            return;
        }
        if v[0].pos >= v11 {
            loop {
                let mut nb = v[0];
                nb.stat = if nb.stat == 1 { 4 } else { nb.stat - 1 };
                nb.pos -= v11;
                nb.flag = 0;
                let np = nb.pos;
                v.insert(0, nb);
                n += 1;
                if np < v11 {
                    break;
                }
            }
        }
        let total = self.total;
        let last = (n - 1) as usize;
        if v[last].pos + v11 <= total {
            loop {
                let mut nb = *v.last().unwrap();
                nb.pos += v11;
                nb.stat = if nb.stat == 4 { 1 } else { nb.stat + 1 };
                nb.flag = 0;
                let np = nb.pos;
                v.push(nb);
                n += 1;
                if !(np + v11 <= total) {
                    break;
                }
            }
        }
        self.n = n;
    }
}
