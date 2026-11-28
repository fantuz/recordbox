//! `BeatAnalyzer_2_0::peakss::BeatInfoAnalyzer::analyze` (0x101198e4c), rekordbox 7.2.19.
//!
//! 2_0's tempo stage runs 1_0's two analyses one after the other on copies of the sensor's
//! candidate list, whatever the library was created for (the `varied` argument is not read):
//! the constant-tempo analysis, `judgeHendouDynamicPeak` (0x1011a6714) on the low-band envelope,
//! then the varied-tempo analysis, and finally its own check of the varied result's beat
//! intervals. When any of the checks says the tempo changes, the varied list and tempo are the
//! output and the verdict (`Analyzer` +45, `getBpmHendou`) is set; otherwise the constant list.
//! Created for varied tempo there is no `UnitBeatAdjust` and so no envelope, which makes the
//! judge say "changes" every time.

use super::BeatInfo;
use crate::analysis::rb_beat1::bia_analyze;

/// What `analyze` hands back to `Analyzer::analyze`.
pub struct Out {
    pub bpm: f32,
    pub list: Vec<BeatInfo>,
    /// The tempo-change verdict written through the `bool&` (+45). Left alone (false, its value
    /// from `create`) on the early exits.
    pub hendou: bool,
    /// The `bool&` of `Analyzer::analyze`: set when a tempo above 180 BPM was halved.
    pub halved: bool,
    /// The tempo-change check reached a branch that is not ported (see `post::Verdict`).
    pub unported: bool,
}

/// `judgeHendouDynamicPeak` (0x1011a6714): the low-band envelope (`low`, at sr / 4) summed over
/// half a beat (`7.5 * sr / bpm` envelope samples) at every sensor candidate in the first 15
/// minutes, until a sum can no longer be made; true ("tempo changes") when
/// `count * max * mean < 2e15` (count = all candidates). `bpm` is the constant analysis'
/// (`+40`).
pub fn judge_hendou(sr: i32, list: &[BeatInfo], bpm: f32, total: i32, low: Option<&[i16]>) -> bool {
    let n = list.len() as i32;
    let mut vals = vec![0f64; n.max(0) as usize];
    let mut m: i32;
    if n < 2 {
        return (n as f64) * 0.0 < 2.0e15;
    }
    m = n - 1;
    let limit = 900 * sr;
    let half = sr as f64 * 7.5;
    let cap = 225 * sr;
    let mut i = 0i32;
    loop {
        let p = list[i as usize].pos;
        if p >= limit {
            m = i;
            break;
        }
        let mut v = -1.0f64;
        let mut skip = false;
        if bpm.abs() > 1.1920929e-7 && p >= -3 {
            let w = (half / bpm as f64 + 0.5) as i32 as u32 as i32;
            let start = ((p as f64) * 0.25 + 0.5) as i32;
            let last = start + w - 1;
            if last <= total / 4 && last <= cap {
                if let Some(low) = low {
                    if w < 1 {
                        vals[i as usize] = 0.0;
                        skip = true;
                    } else {
                        v = 0.0;
                        for k in 0..w {
                            v += low.get((start + k) as usize).copied().unwrap_or(0) as f64;
                        }
                    }
                }
            }
        }
        if !skip {
            vals[i as usize] = v;
            if i != 0 && v < 0.0 {
                m = i;
                break;
            }
        }
        i += 1;
        if i == n - 1 {
            break;
        }
    }
    if m == 0 {
        return (n as f64) * 0.0 < 2.0e15;
    }
    let mut max = 0f64;
    let mut sum = 0f64;
    for &v in &vals[..m as usize] {
        max = max.max(v);
        sum += v;
    }
    let mean = sum / m as f64;
    (n as f64) * (max * mean) < 2.0e15
}

/// `BeatInfoAnalyzer::analyze(info, out, total, 4, 16, 12, varied, low, hendou&, flag&)`.
pub fn analyze(sr: i32, sensor: &[BeatInfo], total: i32, low: Option<&[i16]>) -> Out {
    let mut lc = sensor.to_vec();
    let mut c = vec![];
    let cbpm = bia_analyze(sr, &mut lc, &mut c, total, false, None);
    let judge = judge_hendou(sr, sensor, cbpm, total, low);
    let mut lv = sensor.to_vec();
    let mut v = vec![];
    // the varied pass' segAdjust searches the list the constant pass left (it is handed the
    // caller's `info`, whose list the constant pass filtered in place)
    let vbpm = bia_analyze(sr, &mut lv, &mut v, total, true, Some(&lc));
    if cbpm.abs() <= 1.1920929e-7 || vbpm.abs() <= 1.1920929e-7 || v.is_empty() || sr == 0 {
        return Out { bpm: 0.0, list: vec![], hendou: false, halved: false, unported: false };
    }
    let p = post::check(sr, &v, cbpm, vbpm, judge);
    if p.hendou {
        Out { bpm: p.vbpm, list: v, hendou: true, halved: p.halved, unported: p.unported }
    } else {
        Out { bpm: p.cbpm, list: c, hendou: false, halved: p.halved, unported: p.unported }
    }
}

/// The check of the varied result after both analyses (0x10119fe0c to the end of `analyze`):
/// the varied list's beat intervals (whole ms), jumps in them, segments of one tempo, a
/// reference interval, how far the smoothed intervals stray from it, a regression of the beat
/// positions against a grid at the (snapped) tempo, and statistics of what is left. Any of the
/// tests can call the tempo varied ("hendou"); the regression also refines both tempi.
///
/// Sums are taken in order; where the original's vector loops add in eight lanes the results
/// can differ in the last bit for non-integer values (they only feed threshold tests).
mod post {
    use super::BeatInfo;

    pub struct Verdict {
        pub hendou: bool,
        pub cbpm: f32,
        pub vbpm: f32,
        pub halved: bool,
        /// A branch reached that is not ported (jumps in the regression residuals): the verdict
        /// there is the one without that branch.
        pub unported: bool,
    }

    const EPS: f64 = 2.22044605e-16;

    fn sum(v: &[f64]) -> f64 {
        v.iter().fold(0.0, |a, &x| x + a)
    }

    /// Mean of the longest run (at least `need` long) of intervals whose change stays within
    /// `lim` ms (0x1011a0c.. loops at 11, 20 and 30 ms).
    fn run_mean(a: &[f64], e: &[f64], n: usize, need: i32, lim: f64, best: &mut i32, best_sum: &mut f64) {
        let (mut run, mut acc) = (0i32, 0f64);
        for i in 0..n {
            if e[i] <= lim {
                acc += a[i];
                run += 1;
            } else {
                if run >= need && *best < run {
                    *best_sum = acc;
                    *best = run;
                }
                acc = 0.0;
                run = 0;
            }
        }
        if run >= need && *best < run {
            *best_sum = acc;
            *best = run;
        }
    }

    /// Stretches where two consecutive interval errors exceed `lim` ms: (start, end) beat
    /// indices as the original's 128-entry tables hold them.
    fn stretches(x: &[f64], n: i32, lim: f64) -> (Vec<i32>, Vec<i32>) {
        let (mut s, mut e) = (vec![], vec![]);
        let mut i: i32 = 0;
        loop {
            let i1 = i + 1;
            if x[i as usize].abs() <= lim || x[i1 as usize].abs() <= lim {
                i += 1;
            } else {
                s.push(i + 2);
                let mut j;
                if i1 >= n - 2 {
                    j = i + 1;
                } else {
                    let mut k = i1;
                    loop {
                        j = k + 1;
                        let big = x[(k + 1) as usize].abs() > lim;
                        k += 1;
                        if !(big && j < n - 2) {
                            break;
                        }
                    }
                }
                i = j + 1;
                e.push(i);
            }
            if !(i < n - 2 && s.len() < 128) {
                break;
            }
        }
        (s, e)
    }

    /// Expected beat count from the stretches (0x1011a1f.. "v1593"): beats outside them plus
    /// the beats the reference interval fits into each.
    fn expected(s: &[i32], e: &[i32], tm: &[f64], n: i32, inv: f64) -> i32 {
        let mut c: i32 = 1;
        for k in 0..s.len().min(128) {
            let st = s[k];
            if st <= 0 {
                break;
            }
            let en = e[k];
            let (a, b);
            if st == 1 {
                a = 0;
                b = if en >= n { ((tm[(n - 1) as usize] - tm[0]) * inv).round() as i64 } else { ((tm[en as usize] - tm[0]) * inv).round() as i64 - 1 };
            } else {
                a = if en >= n { ((tm[(n - 1) as usize] - tm[(st - 2) as usize]) * inv).round() as i64 } else { ((tm[en as usize] - tm[(st - 2) as usize]) * inv).round() as i64 - 1 };
                b = a;
            }
            if k != 0 {
                c = st + c + !e[k - 1] - 1;
            } else if st != 1 {
                c = st + c - 2;
            }
            if b >= 1 {
                c += a as i32;
                if en + 1 < n {
                    c += 1;
                }
            }
        }
        c
    }

    pub fn check(sr: i32, v: &[BeatInfo], cbpm0: f32, vbpm0: f32, judge: bool) -> Verdict {
        let mut cbpm = cbpm0;
        let mut vbpm = vbpm0;
        let mut halved = false;
        let mut unported = false;
        let n = v.len() as i32;
        let nu = n as usize;
        // beat times in seconds, rounded to whole ms (vector part: pos * (1000 / sr), the last
        // one or two: pos * 1000 * (1 / sr))
        let srf = sr as f64;
        let k_v = 1000.0 / srf;
        let inv_sr = 1.0 / srf;
        let simd = if n > 2 { n - if n & 1 == 1 { 1 } else { 2 } } else { 0 };
        let t: Vec<f64> = v.iter().enumerate().map(|(i, b)| if (i as i32) < simd { (b.pos as f64 * k_v).round() * 0.001 } else { ((b.pos as f64 * 1000.0) * inv_sr).round() * 0.001 }).collect();
        let v1132: i32 = if n <= 21 { if n <= 1 { 0 } else { n - 2 } } else { 20 };
        let v1135: i32 = if n <= 52 { n.max(16) - 17 } else { 36 };
        let mut a = vec![0f64; nu];
        let mut b = vec![0f64; nu];
        let mut cc = vec![0f64; nu];
        let mut d = vec![0f64; nu];
        let mut e = vec![0f64; nu];
        if n >= 2 {
            for i in 0..nu - 1 {
                let x = (t[i + 1] - t[i]).abs().mul_add(1000.0, 0.5);
                a[i] = x;
                b[i] = x;
            }
        }
        let v2426 = n - v1132;
        let m = n - v1132 - 1; // v1147
        let mut seg_hendou = false; // v1163
        if v2426 > 1 {
            for i in 0..m as usize {
                e[i] = (a[i + 1] - a[i]).abs();
            }
            // runs of interval jumps above 11 ms
            let mut st = [0i32; 32];
            let mut en = [0i32; 32];
            let mut mean = [0f64; 34];
            let mut k: i32 = 0;
            let mut prev = false;
            let mut count = -1i32;
            for i in 0..m {
                let x = e[i as usize];
                if x <= 11.0 {
                    if prev {
                        let old = k;
                        k += 1;
                        if old > 30 {
                            count = k;
                            break;
                        }
                    }
                } else {
                    if !prev {
                        st[k as usize] = i;
                    }
                    en[k as usize] = i;
                }
                prev = x > 11.0;
            }
            if count < 0 {
                count = k;
            }
            if count >= 1 {
                seg_hendou = segments(&mut a, &t, &st, &en, &mut mean, count, m);
            }
        }
        let vb = vbpm as f64; // v1434
        // smoothed intervals: means over 16
        let k16 = n - v1135 - 1; // v1435
        for i in 0..k16.max(0) {
            let w = if i + 16 >= n { n - i - 1 } else { 16 };
            if w >= 1 {
                let mut acc = cc[i as usize];
                for t2 in 0..w {
                    acc += a[(i + t2) as usize];
                }
                cc[i as usize] = acc / w as f64;
            }
        }
        let need = n.min(37);
        let span = (m.max(1)) as usize; // v1449
        let beat_s = 60.0 / vb; // v1450
        // the reference interval (v1465): the longest steady run, else the median
        let mut best = 0i32;
        let mut best_sum = 0f64;
        let refv: f64;
        if v2426 <= 1 {
            refv = beat_s * 1000.0;
        } else {
            run_mean(&a, &e, span, need, 11.0, &mut best, &mut best_sum);
            if best == 0 {
                run_mean(&a, &e, span, need, 20.0, &mut best, &mut best_sum);
            }
            if best == 0 {
                run_mean(&a, &e, span, need, 30.0, &mut best, &mut best_sum);
            }
            if best != 0 {
                refv = best_sum / best as f64;
            } else {
                let mut q: Vec<f64> = a[..span].to_vec();
                q.resize(m.max(0) as usize, 0.0);
                // The reference sorts in descending order to take the median.
                // These are finite, rounded beat intervals; a full quadratic
                // selection sort has no bearing on either centre value.
                q.sort_unstable_by(|a, b| b.total_cmp(a));
                refv = if m & (0x8000_0001u32 as i32) == 1 {
                    q[(m as u32 >> 1) as usize]
                } else {
                    let h = if m >= 0 { m } else { m + 1 } >> 1;
                    (q[h as usize] + q[(h - 1) as usize]) * 0.5
                };
            }
        }
        // how far the smoothed intervals stray from it
        let mut dev = 0f64; // v1493
        if k16 >= 1 {
            let mut s1 = 0f64;
            for i in 0..k16 as usize {
                let x = (cc[i] - refv).abs();
                d[i] = x;
                s1 += x;
            }
            let den = n as f64 - v1135 as f64 + -1.0;
            dev = s1 / den;
            let (mut cnt, mut s2) = (0i32, 0f64);
            let inv = 1.0 / refv;
            for i in 0..k16 as usize {
                let c = cc[i];
                let close = if c.abs() <= EPS {
                    true
                } else {
                    let r = refv / c;
                    let r2 = if r >= 1.0 { r } else { c * inv };
                    r2 < 1.05
                };
                if close {
                    s2 += d[i];
                    cnt += 1;
                }
            }
            let pct = cnt as f64 * 100.0 / den;
            if cnt != 0 && pct > 70.0 {
                dev = s2 / cnt as f64;
            }
        }
        // beats in steady runs longer than 23
        let mut steady = 0i32;
        if v2426 > 1 {
            let mut run = 0i32;
            for i in 0..span {
                let x = e[i];
                let nxt = if x <= 11.0 { run + 1 } else { 0 };
                if run <= 23 || x <= 11.0 {
                    run = 0;
                }
                steady += run;
                run = nxt;
            }
        }
        let dev_hendou = (dev >= 2.25 && steady as f64 / n as f64 <= 0.19) || (dev > 1.56 && (steady as f64 / n as f64) < 0.057); // v1509
        // the tempo the regression uses (v1512)
        let rv = vbpm.round();
        let mut tb: f32 = if rv as f64 * 1.0005 >= vb && rv as f64 * 0.9995 <= vb { rv } else { vbpm };
        let rbpm = 60000.0 / refv; // v1513
        if rbpm >= tb as f64 * 1.98 && rbpm <= tb as f64 * 2.02 {
            if tb <= 90.0 {
                tb += tb;
            }
        } else if rbpm < tb as f64 * 0.495 || rbpm > tb as f64 * 0.505 {
            let inside = rbpm >= tb as f64 * 0.8 && rbpm <= tb as f64 * 1.25;
            if !inside {
                if rbpm >= 70.0 {
                    tb = rbpm as f32;
                    if rbpm > 180.0 {
                        tb *= 0.5;
                        halved = true;
                    }
                } else {
                    let x = rbpm as f32;
                    tb = x + x;
                }
            }
        } else if tb >= 140.0 {
            tb *= 0.5;
        }
        if dev_hendou || judge || seg_hendou {
            return Verdict { hendou: true, cbpm, vbpm, halved, unported };
        }
        // --- the regression part (LABEL_2027 onwards)
        let mut tm = vec![0f64; nu];
        let mut x = vec![0f64; nu];
        tm[0] = t[0] * 1000.0;
        let mut ran = false;
        let (mut s10, mut e10) = (vec![], vec![]);
        if n >= 2 {
            for i in 0..nu - 1 {
                tm[i + 1] = t[i + 1] * 1000.0;
                x[i] = b[i] - refv;
            }
            if n >= 3 {
                let r = stretches(&x, n, 10.0);
                s10 = r.0;
                e10 = r.1;
                ran = true;
            }
        }
        let mut iv = srf * 60.0 / tb as f64; // v1585
        let in10: i32 = s10.iter().zip(&e10).map(|(s, e)| e - s + 1).sum();
        let inv = 1.0 / refv;
        // the positions the regression works on: the varied list, made as long as the count
        // expected from the stretches (copies of the last beat appended, or cut)
        let tables_10 = in10 as f64 / n as f64 >= 0.5;
        let (ss, ee) = if tables_10 { (s10.clone(), e10.clone()) } else if ran { stretches(&x, n, 20.0) } else { (vec![], vec![]) };
        let exp_count = expected(&ss, &ee, &tm, n, inv);
        let total_exp = if let Some(&last) = ee.last() { (if last >= n { 0 } else { !last + n }) + exp_count } else { exp_count + (n - 1) };
        let nf = n as f64;
        let lo = (nf * 0.05 + 0.5) as i32;
        let hi = (nf * 0.8 + 0.5) as i32;
        let verdict = |h: bool, cbpm: f32, vbpm: f32, unported: bool| Verdict { hendou: h, cbpm, vbpm, halved, unported };
        if hi - lo < 65 {
            return verdict(true, cbpm, vbpm, unported);
        }
        // windows between the 10 ms stretches, inside [5 %, 80 %] of the list
        let mut mm = (0..128).find(|&k| { let s = s10.get(k).copied().unwrap_or(0); s == 0 || s > hi }).map(|k| k as i32).unwrap_or(0);
        let mut q1 = vec![0i32; (mm + 1) as usize];
        let mut q2 = vec![0i32; (mm + 1) as usize];
        q1[0] = lo;
        for k in 0..mm as usize {
            q2[k] = s10[k] - 2;
            q1[k + 1] = e10[k] + 2;
        }
        q2[mm as usize] = hi;
        let mut k: i32 = 0;
        'rm: loop {
            loop {
                if q1[k as usize] > q2[k as usize] {
                    break;
                }
                k += 1;
                if k > mm {
                    break 'rm;
                }
            }
            for j in k..mm {
                q1[j as usize] = q1[(j + 1) as usize];
                q2[j as usize] = q2[(j + 1) as usize];
            }
            q1[mm as usize] = 0;
            q2[mm as usize] = 0;
            mm -= 1;
            if !(k <= mm) {
                break;
            }
        }
        if mm < 0 {
            return verdict(true, cbpm, vbpm, unported);
        }
        let q3: Vec<i32> = (0..=mm as usize).map(|k| q2[k] - q1[k] + 1).collect();
        let total: i32 = q3.iter().sum();
        if total <= 0 {
            return verdict(true, cbpm, vbpm, unported);
        }
        // Read the logical truncated or last-beat-padded list in place.
        let p = |i: i32| if (0..total_exp).contains(&i) { v[i.min(n - 1) as usize].pos } else { 0 };
        // deviations of the beats from a grid at `iv`, window by window
        let mut r = vec![0i32; total as usize];
        if q3[0] >= 1 {
            let b0 = q1[0];
            let p0 = p(b0);
            r[0] = 0;
            let mut w: i32;
            if q3[0] < 2 {
                w = 1;
            } else {
                let off = (iv * b0 as f64 + 0.5) as i32;
                w = 1;
                while q3[0] > w {
                    r[w as usize] = p(w + b0) + off - (p0 + (iv * (w + b0) as f64 + 0.5) as i32);
                    w += 1;
                }
            }
            if w != 0 && mm >= 1 {
                for g in 1..=mm as usize {
                    if q3[g] >= 1 {
                        let bg = q1[g];
                        let pg = p(bg);
                        let base_w = w;
                        let prev = r[(base_w - 1) as usize];
                        r[base_w as usize] = prev;
                        if q3[g] < 2 {
                            w = base_w + 1;
                        } else {
                            let base = prev - pg + (iv * bg as f64 + 0.5) as i32;
                            let mut t2 = 1;
                            while t2 < q3[g] {
                                r[(base_w + t2) as usize] = base + p(t2 + bg) - (iv * (t2 + bg) as f64 + 0.5) as i32;
                                t2 += 1;
                            }
                            w = base_w + t2;
                        }
                    }
                }
            }
        }
        // linear regression of the deviations
        let (mut sx, mut sy, mut sxx, mut sxy) = (0f64, 0f64, 0f64, 0f64);
        for (i, &y) in r.iter().enumerate() {
            let xi = i as f64;
            let yf = y as f64;
            sx += xi;
            sy += yf;
            sxx += xi * xi;
            sxy += yf * xi;
        }
        let tf = total as f64;
        let xm = sx / tf;
        let nx = xm * tf;
        if (sxx - nx * xm).abs() > EPS {
            let ym = sy / tf;
            let slope = (-nx).mul_add(ym, sxy) / (sxx - nx * xm);
            for (i, y) in r.iter_mut().enumerate() {
                *y -= ((i as f64 - xm).mul_add(slope, ym)).round() as i32;
            }
            if (iv - slope).abs() > EPS {
                // refine both tempi by the slope (0x1011a2d20 ..)
                let iv2 = iv - slope;
                let srd = srf;
                let s60 = srd * 60.0;
                let ds = slope / srd;
                let vcorr: f32 = if (beat_s - ds).abs() <= EPS { 0.0 } else { (60.0 / (beat_s - ds)) as f32 };
                let b_iv = s60 / iv;
                let c_period = 60.0 / cbpm as f64;
                let b_iv2 = s60 / iv2;
                let ccorr: f32 = if (c_period - ds).abs() > EPS { (60.0 / (c_period - ds)) as f32 } else { 0.0 };
                let (r1, r2) = (b_iv.round(), b_iv2.round());
                if r1 == r2 && (b_iv - r1).abs() > (b_iv2 - r2).abs() {
                    iv = iv2;
                }
                let rc = vcorr.round();
                if rv == rc && (vbpm - rv).abs() > (vcorr - rc).abs() {
                    vbpm = vcorr;
                }
                let rcb = cbpm.round();
                let rcc = ccorr.round();
                if rcb == rcc && (cbpm - rcb).abs() > (ccorr - rcc).abs() {
                    cbpm = ccorr;
                }
                let s600 = srd * 600.0;
                let q = s600 / iv;
                let qr = q.round();
                if !(q > qr * 1.001 || q < qr * 0.999) {
                    iv = s600 / qr;
                }
                let snap = |bpm: f32| -> f32 {
                    let f = (bpm * 10.0).round();
                    if bpm > f * 0.1001 || bpm < f * 0.0999 {
                        bpm
                    } else {
                        f * 0.1
                    }
                };
                vbpm = snap(vbpm);
                cbpm = snap(cbpm);
            }
        }
        if total < 2 {
            return verdict(true, cbpm, vbpm, unported);
        }
        // jumps in the detrended deviations
        let nd = (total - 1) as usize;
        let mut dd: Vec<i32> = (0..nd).map(|i| (r[i + 1] - r[i]).abs()).collect();
        let mut mn = dd[0];
        if total >= 3 {
            for &y in &dd[1..] {
                if mn >= y {
                    mn = y;
                }
            }
        }
        dd[0] -= mn;
        for y in dd.iter_mut().skip(1) {
            *y -= mn;
        }
        let jumps: Vec<i32> = (0..nd as i32).filter(|&i| iv * 0.0625 < dd[i as usize] as f64).collect();
        if !jumps.is_empty() {
            // phase jumps: the original drops short stretches between them (more than one jump)
            // and measures the deviations per stretch (any jump; 0x1011a3.. "v1843 != 0"), not
            // ported; never reached on the benchmark tracks or the synthetic programs
            unported = true;
        }
        // deviations relative to their minimum (all but the last), then to the grid
        let r0 = r[0];
        let mut mn2 = r0;
        if total >= 3 {
            for &y in &r[1..(total - 1) as usize] {
                if mn2 >= y {
                    mn2 = y;
                }
            }
        }
        let cnt = (total - 1) as usize;
        r[0] = r0 - mn2;
        for y in r.iter_mut().skip(1) {
            *y -= mn2;
        }
        let s_r: f64 = r[..cnt].iter().map(|&y| y as f64).sum();
        let qv = s_r / (iv * cnt as f64);
        let tq = qv.trunc();
        let fr = qv - tq;
        let off = if fr < 0.1 && fr > -0.1 {
            Some(iv * tq)
        } else if fr > 0.9 {
            Some(iv + tq * iv)
        } else if fr < -0.9 {
            Some(-(iv - tq * iv))
        } else if fr > 0.4 && fr < 0.6 {
            Some((tq + 0.5) * iv)
        } else if fr > -0.6 && fr < -0.4 {
            Some((tq + -0.5) * iv)
        } else if fr > 0.15 && fr < 0.35 {
            Some((tq + 0.25) * iv)
        } else if fr > -0.35 && fr < -0.15 {
            Some((tq + -0.25) * iv)
        } else if fr > 0.65 && fr < 0.85 {
            Some((tq + 0.75) * iv)
        } else if fr > -0.85 && fr < -0.65 {
            Some((tq + -0.75) * iv)
        } else {
            None
        };
        let (u, w): (Vec<i32>, Vec<i32>) = r[..cnt]
            .iter()
            .map(|&y| match off {
                Some(o) => ((0.5 - o + y as f64) as i32, ((y as f64 - o).abs() + 0.5) as i32),
                None => (y, y),
            })
            .unzip();
        if (cnt as i32) < 64 {
            return verdict(true, cbpm, vbpm, unported);
        }
        let n2 = cnt - 31;
        let mut mu = vec![0f64; n2];
        let mut mw = vec![0f64; n2];
        for i in 0..n2 {
            let (mut su, mut sw) = (0f64, 0f64);
            for t2 in 0..32 {
                su += u[i + t2] as f64;
                sw += w[i + t2] as f64;
            }
            mu[i] = su * 0.03125;
            mw[i] = sw * 0.03125;
        }
        let mnw = mw.iter().copied().fold(mw[0], f64::min);
        mw[0] -= mnw;
        let mut mx = mw[0].max(0.0);
        for y in mw.iter_mut().skip(1) {
            *y -= mnw;
            mx = mx.max(*y);
        }
        let n3 = cnt - 47;
        let mut du = vec![0f64; n3];
        let mut dw = vec![0f64; n3];
        let (mut s_du, mut s_dw) = (0f64, 0f64);
        for i in 0..n3 {
            du[i] = mu[i + 16] - mu[i];
            dw[i] = mw[i + 16] - mw[i];
            s_du += du[i];
            s_dw += (mw[i + 16] - mw[i]).abs();
        }
        let n3f = n3 as f64;
        let mean_du = s_du / n3f;
        let var_du: f64 = du.iter().map(|&y| (y - mean_du) * (y - mean_du)).sum();
        let n4 = cnt - 63;
        let s_d2: f64 = (0..n4).map(|i| (dw[i + 16] - dw[i]).abs()).sum();
        let h = mx * nf > iv * 0.13 * n2 as f64 && s_dw * nf / n3f > n3f * 380.0 && s_d2 * nf / n4 as f64 > n4 as f64 * 130.0 && var_du * nf / n3f > iv * 6.4 * n3f;
        verdict(h, cbpm, vbpm, unported)
    }

    /// Segments of one tempo between interval jumps (0x1011a07.. to 0x1011a14..): each
    /// segment's intervals normalised by its mean and scaled back by the reference segment's
    /// mean when the two are related by 1/2, 2/3, 3/2 or 2; true when at least two long
    /// segments have near-whole but different tempi and none dominates.
    fn segments(a: &mut [f64], t: &[f64], st: &[i32; 32], en: &[i32; 32], mean: &mut [f64; 34], count: i32, m: i32) -> bool {
        let c = count as usize;
        let s0 = st[0];
        if s0 < 1 {
            mean[0] = a[0];
        } else {
            let mu = sum(&a[..s0 as usize]) / s0 as f64;
            mean[0] = mu;
            if mu.abs() > EPS {
                let r = 1.0 / mu;
                for y in a[..s0 as usize].iter_mut() {
                    *y *= r;
                }
            }
        }
        if en[0] >= s0 {
            for y in a[s0 as usize..=en[0] as usize].iter_mut() {
                *y = 1.0;
            }
        }
        for j in 1..c {
            let s = st[j];
            let pe = en[j - 1];
            if s <= pe + 1 {
                mean[j] = a[(pe + 1) as usize];
            } else {
                let mu = (mean[j] + sum(&a[(pe + 1) as usize..s as usize])) / (s - pe - 1) as f64;
                mean[j] = mu;
                if mu.abs() > EPS {
                    let r = 1.0 / mu;
                    for y in a[(pe + 1) as usize..s as usize].iter_mut() {
                        *y *= r;
                    }
                }
            }
            let e = en[j];
            if e >= s {
                for y in a[s as usize..=e as usize].iter_mut() {
                    *y = 1.0;
                }
            }
        }
        let last = en[c - 1];
        let stt = last + 1;
        let tail_mean;
        if m <= stt {
            tail_mean = a[stt as usize];
            mean[c] = tail_mean;
        } else {
            let mu = (mean[c] + sum(&a[stt as usize..m as usize])) / (m - last - 1) as f64;
            mean[c] = mu;
            tail_mean = mu;
            if mu.abs() > EPS {
                let r = 1.0 / mu;
                for y in a[stt as usize..m as usize].iter_mut() {
                    *y *= r;
                }
            }
        }
        let mut f = vec![0f32; c + 1];
        let mut g = vec![0i32; c + 1];
        f[0] = (t[s0 as usize] - t[1]) as f32;
        g[0] = s0 - 1;
        let mut gmax = s0 - 1;
        let mut arg = 0usize;
        for j in 0..c.saturating_sub(1) {
            g[j + 1] = st[j + 1] - en[j] - 1;
            f[j + 1] = (t[st[j + 1] as usize] - t[(en[j] + 1) as usize]) as f32;
            if gmax < g[j + 1] {
                arg = j + 1;
            }
            if gmax <= g[j + 1] {
                gmax = g[j + 1];
            }
        }
        g[c] = m - last - 1;
        f[c] = (t[m as usize] - t[(last + 1) as usize]) as f32;
        let rj = if gmax >= g[c] { arg } else { c };
        let refm = mean[rj];
        let test = |x: f64| (x > refm * 0.485 && x < refm * 0.515) || (x > refm * 0.646666667 && x < refm * 0.686666667) || (x > refm * 1.455 && x < refm * 1.545) || (x > refm * 1.94 && x < refm * 2.06);
        let m0 = mean[0];
        let e0 = en[0];
        if e0 >= 0 {
            let k = if test(m0) { refm } else { m0 };
            for y in a[..=e0 as usize].iter_mut() {
                *y *= k;
            }
        }
        let mut pe = e0;
        for j in 1..c {
            let xm = mean[j];
            let e = en[j];
            if pe < e {
                let k = if test(xm) { refm } else { xm };
                for y in a[(pe + 1) as usize..=e as usize].iter_mut() {
                    *y *= k;
                }
            }
            pe = e;
        }
        if m > stt {
            let k = if test(tail_mean) { refm } else { tail_mean };
            for y in a[stt as usize..m as usize].iter_mut() {
                *y *= k;
            }
        }
        // per segment: tempo, its distance to a whole BPM, its share of the duration
        let tm = t[m as usize];
        let inv_t = 1.0 / tm;
        let mut bpm = vec![0f32; c + 1];
        let mut ibpm = vec![0i64; c + 1];
        let mut dv = vec![0f32; c + 1];
        let mut fr = vec![0f32; c + 1];
        for j in 0..=c {
            let mf = mean[j] as f32;
            let (b, ib) = if mf.abs() <= 1.1920929e-7 { (120.0f32, 120i64) } else { let b = 60000.0f32 / mf; (b, b.round() as i64) };
            bpm[j] = b;
            ibpm[j] = ib;
            dv[j] = (b - ib as f32).abs();
            fr[j] = if tm.abs() > EPS { (f[j] as f64 * inv_t) as f32 } else { 0.5 };
        }
        let maxdv = dv.iter().copied().fold(dv[0], f32::max);
        let maxfr = fr.iter().copied().fold(fr[0], f32::max);
        let mut minfr = 1.0f32;
        for j in 0..=c {
            if g[j] >= 17 && minfr > fr[j] {
                minfr = fr[j];
            }
        }
        let mut distinct = false;
        'j: for j in 0..c {
            if g[j] >= 17 && fr[j] > 0.075 {
                let mut k = j;
                loop {
                    if fr[k] > 0.075 && ibpm[j] != ibpm[k] {
                        distinct = true;
                        break 'j;
                    }
                    let mut found = false;
                    while k != c {
                        k += 1;
                        if g[k] >= 17 {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        break;
                    }
                }
            }
        }
        maxdv < 0.2 && maxfr < 0.65 && minfr > 0.075 && (maxfr - minfr) < 0.55 && distinct
    }
}
