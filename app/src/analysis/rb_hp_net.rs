//! Network stage of rekordbox's high precision BeatGrid analysis.
//!
//! rekordbox 7.2.19 (`madmom::analyze` 0x102307f8c) feeds the mono mix `(L + R) * 0.5` of the
//! analysed window to the TensorFlow SavedModel `Resources/models/detect_beat/model8` (model7 when
//! the tempo is allowed to change) and reads `StatefulPartitionedCall:1`. The graph, decoded from
//! `saved_model.pb`, is madmom's downbeat network:
//!
//! - three STFTs (frame 1024 / 2048 / 4096, hop 441 = 100 frames per second at 44.1 kHz), each
//!   with the signal padded by half a frame of zeros at both ends, a periodic Hann window and
//!   `1 + len / 441` frames; magnitudes of the bins below Nyquist;
//! - a logarithmic filterbank per STFT (21, 45 and 91 bands; the matrices are constants in the
//!   graph and are read from the installed file), `log10(1 + x)` computed as `ln(x + 1) / ln 10`;
//! - the positive difference to the frame 1, 1 and 2 frames earlier, stacked: 314 values per frame;
//! - three bidirectional Keras `PeepholeLSTMCell` layers of 25 units (gates i, f, c, o; the input
//!   and forget gates look at the old cell state, the output gate at the new one), concatenated,
//!   and a dense softmax layer of 3 (no beat, beat, downbeat) of which columns 1 and 2 are kept.
//!
//! The weights are read at run time from the installed rekordbox (`variables/variables.index` +
//! `.data`, a TensorFlow tensor bundle); nothing of the model is part of this program.

use anyhow::{anyhow, bail, Context, Result};
use rustfft::{num_complex::Complex, FftPlanner};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const HOP: usize = 441;
pub const FRAME_SIZES: [usize; 3] = [1024, 2048, 4096];
pub const DIFF_FRAMES: [usize; 3] = [1, 1, 2];
pub const NUM_FEATURES: usize = 314;
const UNITS: usize = 25;
const GATES: usize = 4 * UNITS;

/// The model is optional local input. Bound reads even if a file changes while being read.
fn read_model_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let meta = path.metadata()?;
    if !meta.is_file() || meta.len() > limit as u64 { bail!("unexpected beat model file: {}", path.display()); }
    let file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.len() > limit as u64 { bail!("unexpected beat model file: {}", path.display()); }
    let mut buf = Vec::new();
    buf.try_reserve_exact(usize::try_from(opened.len())?)?;
    file.take(limit as u64 + 1).read_to_end(&mut buf)?;
    if buf.len() > limit { bail!("beat model file exceeds size limit"); }
    Ok(buf)
}

/// Where rekordbox keeps its beat models (`PREPDECK_RB_MODELS` overrides it).
pub fn models_dir() -> PathBuf {
    if let Ok(p) = std::env::var("PREPDECK_RB_MODELS") {
        return PathBuf::from(p);
    }
    PathBuf::from("/Applications/rekordbox 7/rekordbox.app/Contents/Resources/models/detect_beat")
}

// ------------------------------------------------------------------------------------------
// Protocol buffer and tensor bundle reading (just enough for these two files)
// ------------------------------------------------------------------------------------------

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut r = 0u64;
    let mut s = 0;
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        if s == 63 && c > 1 { return None; }
        r |= ((c & 0x7f) as u64) << s;
        s += 7;
        if c < 0x80 {
            return Some(r);
        }
        if s > 63 {
            return None;
        }
    }
}

/// Calls `f(field, payload)` for every length-delimited field and `g(field, value)` for every
/// varint field of one message.
fn fields<'a>(b: &'a [u8], mut f: impl FnMut(u32, &'a [u8]), mut g: impl FnMut(u32, u64)) -> Option<()> {
    let mut i = 0;
    while i < b.len() {
        let k = varint(b, &mut i)?;
        let (field, wire) = ((k >> 3) as u32, k & 7);
        match wire {
            0 => {
                let v = varint(b, &mut i)?;
                g(field, v);
            }
            1 => { b.get(i..i.checked_add(8)?)?; i += 8; }
            2 => {
                let l = varint(b, &mut i)? as usize;
                let end = i.checked_add(l)?;
                let s = b.get(i..end)?;
                i = end;
                f(field, s);
            }
            5 => { b.get(i..i.checked_add(4)?)?; i += 4; }
            _ => return None,
        }
    }
    Some(())
}

fn sub<'a>(b: &'a [u8], field: u32) -> Vec<&'a [u8]> {
    let mut v = vec![];
    // The supported graph has a fixed network. Avoid allocating millions of references
    // for a malformed message containing large numbers of empty repeated fields.
    let parsed = fields(b, |f, s| if f == field && v.len() <= 65_536 { v.push(s) }, |_, _| {});
    if parsed.is_none() || v.len() > 65_536 { v.clear(); }
    v
}

/// The three filterbank matrices (bins x bands, row-major) of a SavedModel: the float constants
/// of shape [512 | 1024 | 2048, bands] in its function library.
pub fn read_filterbanks(saved_model: &[u8]) -> Result<[(usize, usize, Vec<f32>); 3]> {
    let mut found: [Option<(usize, usize, Vec<f32>)>; 3] = [None, None, None];
    for mg in sub(saved_model, 2) {
        for gd in sub(mg, 2) {
            let mut nodes: Vec<&[u8]> = sub(gd, 1);
            for lib in sub(gd, 2) {
                for func in sub(lib, 1) {
                    let function_nodes = sub(func, 3);
                    if function_nodes.len() > 65_536 - nodes.len() { bail!("too many beat model nodes"); }
                    nodes.extend(function_nodes);
                }
            }
            for node in nodes {
                let mut is_const = false;
                fields(node, |f, s| if f == 2 && s == b"Const" { is_const = true }, |_, _| {});
                if !is_const {
                    continue;
                }
                for attr in sub(node, 5) {
                    let mut key: &[u8] = b"";
                    let mut val: &[u8] = b"";
                    fields(attr, |f, s| if f == 1 { key = s } else if f == 2 { val = s }, |_, _| {});
                    if key != b"value" {
                        continue;
                    }
                    for t in sub(val, 8) {
                        let mut dtype = 0;
                        let mut dims: Vec<u64> = vec![];
                        let mut content: &[u8] = b"";
                        fields(
                            t,
                            |f, s| {
                                if f == 2 {
                                    for d in sub(s, 2) {
                                        fields(d, |_, _| {}, |g, v| if g == 1 && dims.len() < 3 { dims.push(v) });
                                    }
                                } else if f == 4 {
                                    content = s;
                                }
                            },
                            |f, v| if f == 1 { dtype = v },
                        );
                        if dtype != 1 || dims.len() != 2 {
                            continue;
                        }
                        let (bins, bands) = (dims[0] as usize, dims[1] as usize);
                        let k = match (bins, bands) {
                            (512, 21) => 0,
                            (1024, 45) => 1,
                            (2048, 91) => 2,
                            _ => continue,
                        };
                        if found[k].is_some() || content.len() != bins * bands * 4 {
                            continue;
                        }
                        let m: Vec<_> = content.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                        if !m.iter().all(|weight| weight.is_finite()) { bail!("non-finite beat filterbank weights"); }
                        found[k] = Some((bins, bands, m));
                    }
                }
            }
        }
    }
    let [a, b, c] = found;
    Ok([a.ok_or_else(|| anyhow!("no 512-bin filterbank"))?, b.ok_or_else(|| anyhow!("no 1024-bin filterbank"))?, c.ok_or_else(|| anyhow!("no 2048-bin filterbank"))?])
}

/// Entries of one SSTable block: (key, value).
fn block_entries(b: &[u8]) -> Option<Vec<(Vec<u8>, &[u8])>> {
    if b.len() < 4 {
        return None;
    }
    let nres = u32::from_le_bytes(b[b.len() - 4..].try_into().ok()?) as usize;
    let end = b.len().checked_sub(nres.checked_add(1)?.checked_mul(4)?)?;
    let mut i = 0;
    let mut key: Vec<u8> = vec![];
    let mut out = vec![];
    while i < end {
        let shared = varint(b, &mut i)? as usize;
        let own = varint(b, &mut i)? as usize;
        let vlen = varint(b, &mut i)? as usize;
        if shared > key.len() || shared.checked_add(own)? > 4096 || out.len() >= 4096 { return None; }
        key.truncate(shared);
        let key_end = i.checked_add(own)?;
        key.extend_from_slice(b.get(i..key_end)?);
        i = key_end.checked_add(vlen)?;
        if i > end { return None; }
        out.push((key.clone(), b.get(key_end..i)?));
    }
    Some(out)
}

/// The float tensors of a TensorFlow tensor bundle (`prefix.index` + `prefix.data-00000-of-00001`):
/// name -> (shape, values).
pub fn read_bundle(prefix: &Path) -> Result<Vec<(String, Vec<usize>, Vec<f32>)>> {
    let index = read_model_file(&prefix.with_extension("index"), 1024 * 1024).context("variables.index")?;
    let data = read_model_file(&prefix.with_extension("data-00000-of-00001"), 16 * 1024 * 1024).context("variables.data")?;
    if index.len() < 48 || u64::from_le_bytes(index[index.len() - 8..].try_into().unwrap()) != 0xdb4775248b80fb57 {
        bail!("not a tensor bundle index");
    }
    let foot = &index[index.len() - 48..];
    let mut j = 0;
    let (_mo, _ms) = (varint(foot, &mut j), varint(foot, &mut j));
    let io = varint(foot, &mut j).ok_or_else(|| anyhow!("footer"))? as usize;
    let is = varint(foot, &mut j).ok_or_else(|| anyhow!("footer"))? as usize;
    let mut out = vec![];
    let mut copied = 0usize;
    for (_, handle) in block_entries(index.get(io..io.checked_add(is).context("index block length")?).ok_or_else(|| anyhow!("index block"))?).ok_or_else(|| anyhow!("index block"))? {
        let mut q = 0;
        let o = varint(handle, &mut q).ok_or_else(|| anyhow!("handle"))? as usize;
        let s = varint(handle, &mut q).ok_or_else(|| anyhow!("handle"))? as usize;
        for (key, val) in block_entries(index.get(o..o.checked_add(s).context("data block length")?).ok_or_else(|| anyhow!("data block"))?).ok_or_else(|| anyhow!("data block"))? {
            if key.is_empty() {
                continue;
            }
            let (mut dtype, mut off, mut size, mut shard) = (0u64, 0u64, 0u64, 0u64);
            let mut shape = vec![];
            fields(
                val,
                |f, s| {
                    if f == 2 {
                        for d in sub(s, 2) {
                            fields(d, |_, _| {}, |g, v| if g == 1 { shape.push(v as usize) });
                        }
                    }
                },
                |f, v| match f {
                    1 => dtype = v,
                    3 => shard = v,
                    4 => off = v,
                    5 => size = v,
                    _ => {}
                },
            );
            if dtype != 1 || shard != 0 {
                continue;
            }
            let end = off.checked_add(size).context("tensor extent")?;
            let bytes = data.get(usize::try_from(off)?..usize::try_from(end)?).context("tensor outside data file")?;
            copied = copied.checked_add(bytes.len()).context("tensor data size")?;
            if copied > data.len() || out.len() >= 4096 || bytes.len() % 4 != 0 { bail!("invalid tensor directory"); }
            let v = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            out.push((String::from_utf8_lossy(&key).into_owned(), shape, v));
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------------
// The network
// ------------------------------------------------------------------------------------------

/// One filterbank band: the first bin with a non-zero weight and the weights from there.
struct Band {
    first: usize,
    w: Vec<f32>,
}

struct Lstm {
    input: usize,
    /// input x 100, row-major (columns: i, f, c, o gates of 25 units each).
    w: Vec<f32>,
    /// 25 x 100.
    u: Vec<f32>,
    b: Vec<f32>,
    pi: Vec<f32>,
    pf: Vec<f32>,
    po: Vec<f32>,
}

pub struct Net {
    banks: [Vec<Band>; 3],
    /// [layer][0 = forward, 1 = backward]
    layers: Vec<[Lstm; 2]>,
    dense_w: Vec<f32>,
    dense_b: Vec<f32>,
}

impl Net {
    /// Reads `<dir>/saved_model.pb` and `<dir>/variables/variables.*`.
    pub fn load(dir: &Path) -> Result<Net> {
        let pb = read_model_file(&dir.join("saved_model.pb"), 32 * 1024 * 1024).with_context(|| format!("{}", dir.display()))?;
        let fbs = read_filterbanks(&pb)?;
        let banks = fbs.map(|(bins, bands, m)| {
            (0..bands)
                .map(|b| {
                    let col: Vec<f32> = (0..bins).map(|k| m[k * bands + b]).collect();
                    let first = col.iter().position(|&x| x != 0.0).unwrap_or(0);
                    let last = col.iter().rposition(|&x| x != 0.0).map(|x| x + 1).unwrap_or(first);
                    Band { first, w: col[first..last].to_vec() }
                })
                .collect::<Vec<_>>()
        });
        let mut vars: Vec<(usize, Vec<usize>, Vec<f32>)> = read_bundle(&dir.join("variables").join("variables"))?
            .into_iter()
            .filter_map(|(k, s, v)| {
                let n = k.strip_prefix("_TestLayerModel__rnn_model/variables/")?.split('/').next()?.parse().ok()?;
                Some((n, s, v))
            })
            .collect();
        vars.sort_by_key(|v| v.0);
        if vars.len() != 38 || vars.iter().enumerate().any(|(i, v)| v.0 != i) {
            bail!("unexpected variables in the beat model ({} found)", vars.len());
        }
        let mut it = vars.into_iter().map(|(_, s, v)| (s, v));
        let mut take = |shape: &[usize]| -> Result<Vec<f32>> {
            let (s, v) = it.next().ok_or_else(|| anyhow!("missing variable"))?;
            if s != shape || v.len() != shape.iter().product::<usize>() || !v.iter().all(|x| x.is_finite()) {
                bail!("invalid weights for variable of shape {shape:?}");
            }
            Ok(v)
        };
        let mut layers = vec![];
        for input in [NUM_FEATURES, 2 * UNITS, 2 * UNITS] {
            let mut dir_cell = || -> Result<Lstm> {
                Ok(Lstm {
                    input,
                    w: take(&[input, GATES])?,
                    u: take(&[UNITS, GATES])?,
                    b: take(&[GATES])?,
                    pi: take(&[UNITS])?,
                    pf: take(&[UNITS])?,
                    po: take(&[UNITS])?,
                })
            };
            let f = dir_cell()?;
            let b = dir_cell()?;
            layers.push([f, b]);
        }
        let dense_w = take(&[2 * UNITS, 3])?;
        let dense_b = take(&[3])?;
        if banks[0].len() + banks[1].len() + banks[2].len() != NUM_FEATURES / 2 {
            bail!("filterbanks do not give {NUM_FEATURES} features");
        }
        Ok(Net { banks, layers, dense_w, dense_b })
    }

    /// Number of frames the graph makes for `n` samples.
    pub fn frames(n: usize) -> usize {
        1 + n / HOP
    }

    /// The 314 input values per frame, row-major (frames x 314).
    #[cfg(test)]
    pub fn features(&self, mono: &[f32]) -> Vec<f32> {
        self.features_cancellable(mono, &|| false).expect("test feature window")
    }

    fn features_cancellable(&self, mono: &[f32], cancelled: &dyn Fn() -> bool) -> Option<Vec<f32>> {
        if cancelled() { return None; }
        let n = Net::frames(mono.len());
        let mut out = zeroed(n.checked_mul(NUM_FEATURES)?)?;
        let log: fn(f32) -> f32 = if use_tf_packets() { tf_log_packet } else { f32::ln };
        let mut planner = FftPlanner::<f32>::new();
        let mut col = 0;
        for (k, &size) in FRAME_SIZES.iter().enumerate() {
            let bank = &self.banks[k];
            let bands = bank.len();
            let win = hann_periodic(size);
            let fft = planner.plan_fft_forward(size);
            let mut buf = vec![Complex::new(0f32, 0f32); size];
            let mut scratch = vec![Complex::new(0f32, 0f32); fft.get_inplace_scratch_len()];
            let mut mag = vec![0f32; size / 2];
            let d = DIFF_FRAMES[k];
            let half = size / 2;
            for f in 0..n {
                if f % 16 == 0 && cancelled() { return None; }
                let start = (f * HOP) as isize - half as isize;
                for (i, c) in buf.iter_mut().enumerate() {
                    let p = start + i as isize;
                    let x = if p >= 0 && (p as usize) < mono.len() { mono[p as usize] } else { 0.0 };
                    *c = Complex::new(x * win[i], 0.0);
                }
                fft.process_with_scratch(&mut buf, &mut scratch);
                for (m, c) in mag.iter_mut().zip(&buf[..size / 2]) {
                    *m = c.re.hypot(c.im);
                }
                for (b, band) in bank.iter().enumerate() {
                    let mut s = 0f32;
                    for (w, m) in band.w.iter().zip(&mag[band.first..]) {
                        s += m * w;
                    }
                    // TensorFlow applies Log to the whole frames × bands tensor. Keep the
                    // result directly in the feature matrix; it also supplies the lagged value.
                    let value = tf_elementwise(f * bands + b, n * bands, s + 1.0, log, f32::ln) / std::f32::consts::LN_10;
                    let i = f * NUM_FEATURES + col + b;
                    out[i] = value;
                    out[i + bands] = if f >= d { (value - out[i - d * NUM_FEATURES]).max(0.0) } else { 0.0 };
                }
            }
            col += 2 * bands;
        }
        Some(out)
    }

    /// Beat and downbeat activations per frame for the given features (frames x 314).
    #[cfg(test)]
    pub fn activations(&self, feats: &[f32]) -> Vec<[f32; 2]> {
        self.activations_cancellable(feats.to_vec(), &|| false).expect("test activations")
    }

    fn activations_cancellable(&self, mut x: Vec<f32>, cancelled: &dyn Fn() -> bool) -> Option<Vec<[f32; 2]>> {
        let n = x.len() / NUM_FEATURES;
        for layer in &self.layers {
            if cancelled() { return None; }
            let mut y = zeroed(n.checked_mul(2 * UNITS)?)?;
            run_lstm(&layer[0], &x, n, false, &mut y, 0, cancelled)?;
            run_lstm(&layer[1], &x, n, true, &mut y, UNITS, cancelled)?;
            x = y;
        }
        let mut result = Vec::new();
        result.try_reserve_exact(n).ok()?;
        for (t, h) in x.chunks_exact(2 * UNITS).enumerate() {
            if t % 16 == 0 && cancelled() { return None; }
            let mut z = [0f32; 3];
            for (j, zj) in z.iter_mut().enumerate() {
                let mut s = 0f32;
                for (k, hk) in h.iter().enumerate() {
                    s += hk * self.dense_w[k * 3 + j];
                }
                *zj = s + self.dense_b[j];
            }
            let m = z[0].max(z[1]).max(z[2]);
            let e = z.map(|v| (v - m).exp());
            let sum = e[0] + e[1] + e[2];
            if !sum.is_finite() || sum <= 0.0 { return None; }
            result.push([e[1] / sum, e[2] / sum]);
        }
        Some(result)
    }

    /// Model7/8 operate on at most a minute per inference after the existing
    /// 44.1/48 kHz conversion. Long tracks are submitted as separate windows.
    pub fn run_cancellable(&self, mono: &[f32], cancelled: &dyn Fn() -> bool) -> Option<Vec<[f32; 2]>> {
        if mono.is_empty() || mono.len() > 60 * 48_000 { return None; }
        self.activations_cancellable(self.features_cancellable(mono, cancelled)?, cancelled)
    }

    #[cfg(test)]
    pub fn run(&self, mono: &[f32]) -> Vec<[f32; 2]> {
        self.run_cancellable(mono, &|| false).expect("test network window")
    }
}

fn zeroed(n: usize) -> Option<Vec<f32>> {
    let mut values = Vec::new();
    values.try_reserve_exact(n).ok()?;
    values.resize(n, 0.0);
    Some(values)
}

/// tf.signal.hann_window(size, periodic=True) as the graph computes it in single precision.
/// (The graph's `Cos` is TensorFlow's kernel; the sizes are multiples of four, so every value
/// takes its packet path.)
pub fn hann_periodic(size: usize) -> Vec<f32> {
    let cos: fn(f32) -> f32 = if use_tf_packets() { tf_cos_packet } else { f32::cos };
    (0..size).map(|i| 0.5f32 - 0.5f32 * tf_elementwise(i, size, 6.283_185_5f32 * i as f32 / size as f32, cos, f32::cos)).collect()
}

// TensorFlow 2.16's CPU kernels as rekordbox's bundled libtensorflow_cc runs them on arm64
// (read from its disassembly: `EvalRange<... scalar_tanh_op<float> ...>::run` 0x8d21850,
// `generic_fast_tanh_float<float>` 0x66a51c4, `EvalRange<... scalar_logistic_op<float> ...>::run`
// 0x8c8bc58). An element-wise op runs on NEON packets of four and on the elements left over one
// at a time, and the two paths round differently: in a 25-unit gate, units 0-23 take the packet
// path and unit 24 the scalar one.

/// Eigen's rational tanh on a packet (fused multiply-adds; the clamp is the non-FMA one).
#[inline]
pub(crate) fn tf_tanh_packet(a: f32) -> f32 {
    let c = f32::from_bits(0x40fcf84f);
    let x = a.min(c).max(-c);
    let x2 = x * x;
    let mut p = x2.mul_add(f32::from_bits(0xa59f25c0), f32::from_bits(0x2a61337e));
    for k in [0xaebd37ffu32, 0x335c0041, 0x3779434a, 0x3a270ded, 0x3ba059dc] {
        p = x2.mul_add(p, f32::from_bits(k));
    }
    let p = x * p;
    let mut q = x2.mul_add(f32::from_bits(0x35a0d3d8), f32::from_bits(0x38f895d6));
    q = q.mul_add(x2, f32::from_bits(0x3b14aa05));
    q = q.mul_add(x2, f32::from_bits(0x3ba059dd));
    if a.abs() < f32::from_bits(0x39d1b717) {
        x
    } else {
        p / q
    }
}

/// The same rational on a single element (separate multiplies and adds).
#[inline]
pub(crate) fn tf_tanh_scalar(a: f32) -> f32 {
    let c = f32::from_bits(0x40fcf84f);
    let x = a.min(c).max(-c);
    let x2 = x * x;
    let mut p = x2 * f32::from_bits(0xa59f25c0) + f32::from_bits(0x2a61337e);
    for k in [0xaebd37ffu32, 0x335c0041, 0x3779434a, 0x3a270ded, 0x3ba059dc] {
        p = x2 * p + f32::from_bits(k);
    }
    let mut q = f32::from_bits(0x35a0d3d8) * x2 + f32::from_bits(0x38f895d6);
    q = q * x2 + f32::from_bits(0x3b14aa05);
    let p = x * p;
    let q = x2 * q + f32::from_bits(0x3ba059dd);
    if a.abs() < f32::from_bits(0x39d1b717) {
        x
    } else {
        p / q
    }
}

/// Eigen's `pexp` on a packet: Cody-Waite reduction, a degree-5 polynomial, 2^m in three steps.
#[inline]
pub(crate) fn tf_exp_packet(x0: f32) -> f32 {
    let x = x0.min(f32::from_bits(0x42b1722d));
    let m = f32::from_bits(0x3fb8aa3b).mul_add(x, 0.5).floor();
    let r = f32::from_bits(0xbf318000).mul_add(m, x);
    let r = f32::from_bits(0x395e8083).mul_add(m, r);
    let r2 = r * r;
    let a = f32::from_bits(0x3ab42872).mul_add(r2, f32::from_bits(0x3d2aadcc));
    let b = f32::from_bits(0x3c091de6).mul_add(r2, f32::from_bits(0x3e2aaa47));
    let c = a.mul_add(r2, f32::from_bits(0x3efffffc));
    let c = b.mul_add(r, c);
    let y = c.mul_add(r2, r + 1.0);
    let mi = m.max(-278.0).min(278.0) as i32;
    let b = mi >> 2;
    let f = f32::from_bits(((b << 23) + 0x3f80_0000) as u32);
    let y = y * f * f * f;
    let g = f32::from_bits((((mi - b - 2 * b) << 23) + 0x3f80_0000) as u32);
    let e = (y * g).max(x0);
    if x0 < -104.0 {
        0.0
    } else {
        e
    }
}

/// Eigen's logistic on a packet: e / (1 + e) with e = exp(x), 1 where e overflows.
#[inline]
pub(crate) fn tf_sigmoid_packet(x: f32) -> f32 {
    let e = tf_exp_packet(x);
    if e == f32::INFINITY {
        1.0
    } else {
        e / (e + 1.0)
    }
}

/// The same on a single element, with the C library's expf.
#[inline]
pub(crate) fn tf_sigmoid_scalar(x: f32) -> f32 {
    let e = x.exp();
    if e == f32::INFINITY {
        1.0
    } else {
        e / (e + 1.0)
    }
}

/// Eigen's cosine on a packet (`psincos_float`, `EvalRange<... scalar_cos_op<float> ...>::run`
/// 0x86e5030): reduction by pi/2 in three parts, then the cosine or sine polynomial by quadrant.
/// Arguments of 71476 and more (a slower reduction in the original) do not occur here.
#[inline]
pub(crate) fn tf_cos_packet(a: f32) -> f32 {
    let x = a.abs();
    let y = x * f32::from_bits(0x3f22f983) + 12_582_912.0;
    let yr = y + -12_582_912.0;
    let r = f32::from_bits(0xbfc90fd8).mul_add(yr, x);
    let r = f32::from_bits(0xb4a8885a).mul_add(yr, r);
    let r = f32::from_bits(0xa7c234c4).mul_add(yr, r);
    let n = y.to_bits();
    let sign = (n << 30).wrapping_add(0x4000_0000) & 0x8000_0000;
    let x2 = r * r;
    let c = f32::from_bits(0x37cc730b).mul_add(x2, f32::from_bits(0xbab6036e));
    let c = x2.mul_add(c, f32::from_bits(0x3d2aaa9e));
    let c = x2.mul_add(c, -0.5);
    let c = x2.mul_add(c, 1.0);
    let sn = f32::from_bits(0xb94d70ca).mul_add(x2, f32::from_bits(0x3c0885d3));
    let sn = x2.mul_add(sn, f32::from_bits(0xbe2aaaa8));
    let sn = r.mul_add(x2 * sn, r);
    let v = if n & 1 == 0 { c } else { sn };
    f32::from_bits(v.to_bits() ^ sign)
}

/// Eigen's natural log on a packet (`plog_float`, `EvalRange<... scalar_log_op<float> ...>::run`
/// 0x89fc968): frexp (denormals scaled by 2^24), the mantissa moved to [sqrt(1/2), sqrt(2)) - 1, a
/// [2/3] rational, plus e * ln 2.
#[inline]
pub(crate) fn tf_log_packet(a: f32) -> f32 {
    // (TensorFlow runs with denormals flushed to zero: their log is -inf)
    if a != 0.0 && a.abs() < f32::MIN_POSITIVE {
        return f32::NEG_INFINITY;
    }
    let den = a.abs() < f32::from_bits(0x0080_0000);
    let xs = if den { a * f32::from_bits(0x4b80_0000) } else { a };
    let bias = if den { -150.0f32 } else { -126.0 };
    let expo = (xs.abs().to_bits() >> 23) as f32;
    let zero = a == 0.0;
    let special = expo == 255.0 || zero;
    let (m, e) = if special { (a, 0.0f32) } else { (f32::from_bits((xs.to_bits() & 0x807f_ffff) | 0x3f00_0000), bias + expo) };
    let lt = m < f32::from_bits(0x3f3504f3);
    let tmp = if lt { m } else { 0.0 };
    let x = m + -1.0;
    let e = e - if lt { 1.0 } else { 0.0 };
    let x = x + tmp;
    let p = f32::from_bits(0x3e3af1c9).mul_add(x, 1.0);
    let p = p.mul_add(x, 1.0);
    let p = x * p;
    let q = f32::from_bits(0x3d4b3a68).mul_add(x, f32::from_bits(0x3f19674d));
    let q = q.mul_add(x, 1.5);
    let q = q.mul_add(x, 1.0);
    let r = f32::from_bits(0x3f317218).mul_add(e, p / q);
    if a == f32::INFINITY {
        f32::INFINITY
    } else if !(a >= 0.0) {
        f32::NAN
    } else if zero {
        f32::NEG_INFINITY
    } else {
        r
    }
}

/// An element-wise kernel over `n` values as TensorFlow runs it on one block: packets of four,
/// the rest one at a time.
#[inline]
fn tf_elementwise(i: usize, n: usize, x: f32, packet: fn(f32) -> f32, scalar: fn(f32) -> f32) -> f32 {
    if i < (n & !3) {
        packet(x)
    } else {
        scalar(x)
    }
}

/// One row times a matrix (`MatMul` of [1 x k] by [k x 100]) as Eigen's matrix-vector product
/// does it: per output, a fused multiply-add chain over a block of columns (all of them when
/// there are fewer than 128, else 16 at a time), each block's sum added to the result.
#[inline(always)]
fn tf_gemv<const FUSED: bool>(x: &[f32], w: &[f32], out: &mut [f32; GATES]) {
    if !FUSED {
        // Older x86 machines have no native FMA. Keep the fork's original, vectorizable
        // single-precision projection there instead of one libm call per model weight.
        *out = [0.0; GATES];
        for (&sample, row) in x.iter().zip(w.chunks_exact(GATES)) {
            if sample != 0.0 {
                for (out, &weight) in out.iter_mut().zip(row) { *out += sample * weight; }
            }
        }
        return;
    }
    let k = x.len();
    let block = if k < 128 { k.max(1) } else { 16 };
    *out = [0f32; GATES];
    let mut j2 = 0;
    while j2 < k {
        let jend = (j2 + block).min(k);
        let mut acc = [0f32; GATES];
        for j in j2..jend {
            let xv = x[j];
            let wr = &w[j * GATES..(j + 1) * GATES];
            for (a, &wv) in acc.iter_mut().zip(wr) {
                *a = wv.mul_add(xv, *a);
            }
        }
        for (o, a) in out.iter_mut().zip(&acc) {
            *o += a;
        }
        j2 = jend;
    }
}

#[inline]
fn use_tf_packets() -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    { std::is_x86_feature_detected!("fma") }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    { true }
}

/// The activation used by the original portable x86 path.
#[inline]
fn sigmoid_unfused(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// The fused dot products are substantially slower in software on generic x86 releases.
/// Select the hardware path once per direction, outside the frame and gate loops.
fn run_lstm(l: &Lstm, x: &[f32], n: usize, reverse: bool, y: &mut [f32], off: usize, cancelled: &dyn Fn() -> bool) -> Option<()> {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if use_tf_packets() {
            // SAFETY: the runtime feature check covers this function's only requirement.
            return unsafe { run_lstm_fma(l, x, n, reverse, y, off, cancelled) };
        }
        run_lstm_rows::<false>(l, x, n, reverse, y, off, cancelled)
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    run_lstm_rows::<true>(l, x, n, reverse, y, off, cancelled)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "fma")]
unsafe fn run_lstm_fma(l: &Lstm, x: &[f32], n: usize, reverse: bool, y: &mut [f32], off: usize, cancelled: &dyn Fn() -> bool) -> Option<()> {
    run_lstm_rows::<true>(l, x, n, reverse, y, off, cancelled)
}

#[inline(always)]
fn run_lstm_rows<const FUSED: bool>(l: &Lstm, x: &[f32], n: usize, reverse: bool, y: &mut [f32], off: usize, cancelled: &dyn Fn() -> bool) -> Option<()> {
    // A projection is independent of other frames. Materialize only the row about
    // to be consumed, in either traversal direction, using the original dot order.
    let mut row = [0f32; GATES];
    let mut h = [0f32; UNITS];
    let mut c = [0f32; UNITS];
    let mut z = [0f32; GATES];
    let mut zh = [0f32; GATES];
    type Op = fn(f32) -> f32;
    let (sp, ss, tp, ts): (Op, Op, Op, Op) = if FUSED {
        (tf_sigmoid_packet, tf_sigmoid_scalar, tf_tanh_packet, tf_tanh_scalar)
    } else {
        (sigmoid_unfused, sigmoid_unfused, f32::tanh, f32::tanh)
    };
    for step in 0..n {
        if step % 16 == 0 && cancelled() { return None; }
        let t = if reverse { n - 1 - step } else { step };
        tf_gemv::<FUSED>(&x[t * l.input..(t + 1) * l.input], &l.w, &mut row);
        tf_gemv::<FUSED>(&h, &l.u, &mut zh);
        for j in 0..GATES {
            z[j] = (row[j] + zh[j]) + l.b[j];
        }
        for j in 0..UNITS {
            let i_g = tf_elementwise(j, UNITS, z[j] + l.pi[j] * c[j], sp, ss);
            let f_g = tf_elementwise(j, UNITS, z[UNITS + j] + l.pf[j] * c[j], sp, ss);
            let c_new = f_g * c[j] + i_g * tf_elementwise(j, UNITS, z[2 * UNITS + j], tp, ts);
            let o_g = tf_elementwise(j, UNITS, z[3 * UNITS + j] + l.po[j] * c_new, sp, ss);
            c[j] = c_new;
            h[j] = o_g * tf_elementwise(j, UNITS, c_new, tp, ts);
        }
        y[t * 2 * UNITS + off..t * 2 * UNITS + off + UNITS].copy_from_slice(&h);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fused_projection_matches_tensorflow_blocks_and_portable_keeps_the_old_sum() {
        for width in [UNITS, 127, 128, NUM_FEATURES] {
            let x: Vec<_> = (0..width).map(|i| ((i % 19) as f32 - 13.0) / 17.0).collect();
            let w: Vec<_> = (0..width * GATES).map(|i| ((i % 37) as f32 - 17.0) / 31.0).collect();
            let mut fused = [0.0; GATES];
            let mut portable = [0.0; GATES];
            tf_gemv::<true>(&x, &w, &mut fused);
            tf_gemv::<false>(&x, &w, &mut portable);
            for j in 0..GATES {
                let block = if width < 128 { width } else { 16 };
                let expected = (0..width).step_by(block).map(|start| {
                    (start..width.min(start + block)).fold(0.0, |sum, k| x[k].mul_add(w[k * GATES + j], sum))
                }).sum::<f32>();
                let original = (0..width).fold(0.0f32, |sum, k| sum + x[k] * w[k * GATES + j]);
                assert_eq!(fused[j].to_bits(), expected.to_bits());
                assert_eq!(portable[j].to_bits(), original.to_bits());
            }
        }
        // Keras' 25 gates have six four-wide packets and one scalar remainder.
        for i in 0..UNITS {
            let a = (i as f32 - 12.0) / 7.0;
            let expected = if i < 24 { tf_tanh_packet(a) } else { tf_tanh_scalar(a) };
            assert_eq!(tf_elementwise(i, UNITS, a, tf_tanh_packet, tf_tanh_scalar).to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn cancellation_discards_partial_features_and_either_lstm_direction() {
        use std::cell::Cell;
        let net = Net { banks: Default::default(), layers: vec![], dense_w: vec![], dense_b: vec![] };
        let polls = Cell::new(0);
        let stop = || { polls.set(polls.get() + 1); polls.get() == 3 };
        assert!(net.run_cancellable(&vec![0.0; 33 * HOP], &stop).is_none());
        assert_eq!(polls.get(), 3);
        let layer = Lstm {
            input: 1, w: vec![0.1; GATES], u: vec![0.1; UNITS * GATES], b: vec![0.1; GATES],
            pi: vec![0.1; UNITS], pf: vec![0.1; UNITS], po: vec![0.1; UNITS],
        };
        for reverse in [false, true] {
            polls.set(0);
            let mut output = vec![0.0; 64 * 2 * UNITS];
            assert!(run_lstm(&layer, &[1.0; 64], 64, reverse, &mut output, 0, &stop).is_none());
            let untouched = if reverse { 0 } else { 63 };
            let processed = if reverse { 63 } else { 0 };
            assert_eq!(output[untouched * 2 * UNITS], 0.0);
            assert!(output[processed * 2 * UNITS] > 0.0);
        }
    }

    #[test]
    fn incomplete_model_metadata_does_not_yield_partial_fields() {
        assert_eq!(sub(&[0x0a, 1, 42], 1), vec![&[42][..]]);
        for trailing_field in [0x09, 0x0a, 0x0d] {
            assert!(sub(&[0x0a, 1, 42, trailing_field], 1).is_empty());
        }
        assert!(block_entries(&[0, 0, 0, 1]).is_none());
        assert!(read_filterbanks(&[]).is_err());
    }

    #[test]
    fn periodic_hann_values() {
        let w = hann_periodic(4);
        // periodic Hann of length 4: 0, 0.5, 1, 0.5
        assert_eq!(w[0], 0.0);
        assert!((w[1] - 0.5).abs() < 1e-7 && (w[2] - 1.0).abs() < 1e-7 && (w[3] - 0.5).abs() < 1e-7);
    }

    /// One LSTM step by hand: x = 1 (one input), h = c = 0, all weights 0.1, biases 0, peepholes
    /// 0.5. z = 0.1 for every gate; i = f = sigmoid(0.1); c = i * tanh(0.1);
    /// o = sigmoid(0.1 + 0.5 c); h = o * tanh(c).
    #[test]
    fn lstm_cell_matches_hand_computation() {
        let l = Lstm {
            input: 1,
            w: vec![0.1; GATES],
            u: vec![0.1; UNITS * GATES],
            b: vec![0.0; GATES],
            pi: vec![0.5; UNITS],
            pf: vec![0.5; UNITS],
            po: vec![0.5; UNITS],
        };
        let mut y = vec![0f32; 2 * UNITS];
        run_lstm(&l, &[1.0], 1, false, &mut y, 0, &|| false).unwrap();
        let s = 1.0 / (1.0 + (-0.1f64).exp());
        let c = s * 0.1f64.tanh();
        let o = 1.0 / (1.0 + (-(0.1 + 0.5 * c)).exp());
        let h = o * c.tanh();
        assert!((y[0] as f64 - h).abs() < 1e-6, "{} vs {h}", y[0]);
        assert!(y[UNITS..].iter().all(|&v| v == 0.0));
        // Second step: h and c from step one feed back through U and the peepholes.
        let mut y2 = vec![0f32; 4 * UNITS];
        run_lstm(&l, &[1.0, 0.0], 2, false, &mut y2, 0, &|| false).unwrap();
        let zz = 0.1 * 25.0 * h; // x = 0, h . U column = 25 * 0.1 * h
        let i2 = 1.0 / (1.0 + (-(zz + 0.5 * c)).exp());
        let c2 = i2 * c + i2 * zz.tanh();
        let o2 = 1.0 / (1.0 + (-(zz + 0.5 * c2)).exp());
        let h2 = o2 * c2.tanh();
        assert!((y2[2 * UNITS] as f64 - h2).abs() < 1e-6, "{} vs {h2}", y2[2 * UNITS]);
    }

    /// Features of a known signal against values worked out by hand: a constant 0 input gives
    /// log10(1 + 0) = 0 everywhere; a single impulse at sample 0 shows up only in the frame
    /// centred on it (frame 0) with |X| = window value at the centre (1.0) in every bin.
    #[test]
    fn feature_front_end_by_hand() {
        let bank = |bins: usize, bands: usize| (0..bands).map(|b| Band { first: b * (bins / bands), w: vec![1.0; 2] }).collect::<Vec<_>>();
        let net = Net { banks: [bank(512, 21), bank(1024, 45), bank(2048, 91)], layers: vec![], dense_w: vec![], dense_b: vec![] };
        let zero = net.features(&vec![0.0; 2000]);
        assert_eq!(zero.len(), Net::frames(2000) * NUM_FEATURES);
        assert!(zero.iter().all(|&v| v == 0.0));
        let mut imp = vec![0.0f32; 2000];
        imp[0] = 1.0;
        let f = net.features(&imp);
        // Frame 0 is centred on sample 0: window value w[N/2] = 1, so |X[k]| = 1 for every bin
        // and every band of two bins sums to 2: log10(3).
        let l3 = 3f32.ln() / std::f32::consts::LN_10;
        assert!((f[0] - l3).abs() < 1e-6, "{}", f[0]);
        // Its difference column is 0: there is no earlier frame.
        assert_eq!(f[21], 0.0);
        // Frame 1 of the 1024 STFT covers samples -71..953: the impulse sits at offset 71 with
        // window value 0.5 - 0.5 cos(2 pi 71 / 1024).
        let w71 = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * 71.0 / 1024.0).cos();
        let v = ((2.0 * w71 + 1.0).log10()) as f32;
        assert!((f[NUM_FEATURES] - v).abs() < 1e-5, "{} vs {v}", f[NUM_FEATURES]);
    }
}
