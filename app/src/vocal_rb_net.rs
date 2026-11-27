//! Vocal detection with an optional local TensorFlow GraphDef. Set RECORDBOX_VOCAL_GRAPH to
//! the absolute path of a compatible unencrypted vd_param_raw.pb. The model runs on the CPU
//! without TensorFlow or a bundled checkpoint.
//!
//! Graph: InputBatch [batch, 29, time] → transpose → bidirectional dynamic RNN, each direction a
//! two-layer `BasicLSTMCell` stack (256 units, forget bias from the graph, gates i, j, f, o,
//! per-window sequence lengths from LengthBatch) → concat [time, 512] → dense 64 + ReLU →
//! dense 2 → softmax. `Discriminator::inference` cuts the track's frames into windows of 360
//! frames every 180 and averages each frame's voice probability (class 1) over the windows that
//! cover it.

use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;

// The reference graph is about 7 MiB. Bound optional input and expanded constants
// before allocation; a scalar constant can encode a much larger tensor.
const MAX_GRAPH_BYTES: usize = 32 * 1024 * 1024;
const MAX_GRAPH_VALUES: usize = MAX_GRAPH_BYTES / 4;
pub const WINDOW: usize = 360;
pub const STEP: usize = 180;

// ------------------------------------------------------------------------------------------
// Minimal protobuf / GraphDef reader
// ------------------------------------------------------------------------------------------

/// One field of a protobuf message: number, and either a varint or a byte slice.
enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
    #[allow(dead_code)]
    Fixed64(u64),
}

fn varint(b: &[u8], o: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let c = *b.get(*o).ok_or_else(|| anyhow!("truncated varint"))?;
        *o += 1;
        if shift > 63 {
            bail!("varint too long");
        }
        v |= ((c & 0x7f) as u64) << shift;
        shift += 7;
        if c < 0x80 {
            return Ok(v);
        }
    }
}

fn fields(b: &[u8]) -> Result<Vec<(u32, Field<'_>)>> {
    let mut o = 0;
    let mut out = Vec::new();
    while o < b.len() {
        let key = varint(b, &mut o)?;
        let (num, wire) = ((key >> 3) as u32, key & 7);
        let f = match wire {
            0 => Field::Varint(varint(b, &mut o)?),
            1 => {
                let s = b.get(o..o + 8).ok_or_else(|| anyhow!("truncated fixed64"))?;
                o += 8;
                Field::Fixed64(u64::from_le_bytes(s.try_into()?))
            }
            2 => {
                let n = varint(b, &mut o)? as usize;
                let s = b.get(o..o.checked_add(n).ok_or_else(|| anyhow!("length overflow"))?).ok_or_else(|| anyhow!("truncated field"))?;
                o += n;
                Field::Bytes(s)
            }
            5 => {
                let s = b.get(o..o + 4).ok_or_else(|| anyhow!("truncated fixed32"))?;
                o += 4;
                Field::Fixed32(u32::from_le_bytes(s.try_into()?))
            }
            w => bail!("unsupported wire type {w}"),
        };
        out.push((num, f));
    }
    Ok(out)
}

/// A float or int32 constant of the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    pub shape: Vec<i64>,
    pub f32s: Vec<f32>,
    pub i32s: Vec<i32>,
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub op: String,
    pub inputs: Vec<String>,
    pub value: Option<Tensor>,
    /// `shape` attribute of placeholders.
    pub shape: Option<Vec<i64>>,
}

fn shape_proto(b: &[u8]) -> Result<Vec<i64>> {
    let mut dims = Vec::new();
    for (n, f) in fields(b)? {
        if let (2, Field::Bytes(d)) = (n, f) {
            let mut size = 0i64;
            for (m, g) in fields(d)? {
                if let (1, Field::Varint(v)) = (m, g) {
                    size = v as i64;
                }
            }
            dims.push(size);
        }
    }
    Ok(dims)
}

fn tensor_proto(b: &[u8], remaining_values: &mut usize) -> Result<Tensor> {
    let mut dtype = 0u64;
    let mut shape = Vec::new();
    let mut content: &[u8] = &[];
    let mut f32s = Vec::new();
    let mut i32s = Vec::new();
    for (n, f) in fields(b)? {
        match (n, f) {
            (1, Field::Varint(v)) => dtype = v,
            (2, Field::Bytes(s)) => shape = shape_proto(s)?,
            (4, Field::Bytes(s)) => content = s,
            (5, Field::Bytes(s)) => f32s.extend(s.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))),
            (5, Field::Fixed32(v)) => f32s.push(f32::from_bits(v)),
            (7, Field::Bytes(s)) => {
                let mut o = 0;
                while o < s.len() {
                    i32s.push(varint(s, &mut o)? as i32);
                }
            }
            (7, Field::Varint(v)) => i32s.push(v as i32),
            _ => {}
        }
    }
    match dtype {
        1 if !content.is_empty() => f32s = content.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        3 if !content.is_empty() => i32s = content.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        1 | 3 => {}
        _ => {}
    }
    // A scalar or splat stored as one value fills the declared shape.
    let count = shape.iter().try_fold(1usize, |size, &dim| {
        size.checked_mul(usize::try_from(dim).ok()?)
    }).ok_or_else(|| anyhow!("invalid tensor dimensions"))?;
    let represented = match dtype {
        1 => if f32s.len() == 1 { count.max(1) } else { f32s.len() },
        3 => if i32s.len() == 1 { count.max(1) } else { i32s.len() },
        _ => 0,
    };
    *remaining_values = remaining_values.checked_sub(represented)
        .ok_or_else(|| anyhow!("vocal graph constants exceed supported size"))?;
    if dtype == 1 && f32s.len() == 1 && count > 1 {
        f32s = vec![f32s[0]; count];
    }
    if dtype == 3 && i32s.len() == 1 && count > 1 {
        i32s = vec![i32s[0]; count];
    }
    Ok(Tensor { shape, f32s, i32s })
}

/// Node list of a GraphDef (field 1 of the message), with Const values and placeholder shapes.
pub fn parse_graph(b: &[u8]) -> Result<Vec<Node>> {
    let mut remaining_values = MAX_GRAPH_VALUES;
    let mut nodes = Vec::new();
    for (n, f) in fields(b)? {
        let (1, Field::Bytes(nb)) = (n, f) else { continue };
        let mut node = Node::default();
        for (m, g) in fields(nb)? {
            match (m, g) {
                (1, Field::Bytes(s)) => node.name = String::from_utf8_lossy(s).into_owned(),
                (2, Field::Bytes(s)) => node.op = String::from_utf8_lossy(s).into_owned(),
                (3, Field::Bytes(s)) => node.inputs.push(String::from_utf8_lossy(s).into_owned()),
                (5, Field::Bytes(entry)) => {
                    let mut key = String::new();
                    let mut val: &[u8] = &[];
                    for (k, h) in fields(entry)? {
                        match (k, h) {
                            (1, Field::Bytes(s)) => key = String::from_utf8_lossy(s).into_owned(),
                            (2, Field::Bytes(s)) => val = s,
                            _ => {}
                        }
                    }
                    for (k, h) in fields(val)? {
                        match (key.as_str(), k, h) {
                            ("value", 8, Field::Bytes(t)) => node.value = Some(tensor_proto(t, &mut remaining_values)?),
                            ("shape", 7, Field::Bytes(s)) => node.shape = Some(shape_proto(s)?),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        nodes.push(node);
    }
    Ok(nodes)
}

// ------------------------------------------------------------------------------------------
// The network
// ------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Lstm {
    /// [input + hidden, 4·hidden], rows for the input first.
    pub kernel: Vec<f32>,
    pub bias: Vec<f32>,
    pub input: usize,
    pub hidden: usize,
    pub forget_bias: f32,
}

#[derive(Clone)]
pub struct Model {
    /// fw and bw: [layer 0, layer 1].
    pub fw: [Lstm; 2],
    pub bw: [Lstm; 2],
    pub fc0: Vec<f32>,
    pub fc0_b: Vec<f32>,
    pub fc1: Vec<f32>,
    pub fc1_b: Vec<f32>,
    pub features: usize,
}

impl Model {
    pub fn from_nodes(nodes: &[Node]) -> Result<Model> {
        let by: HashMap<&str, &Node> = nodes.iter().map(|n| (n.name.as_str(), n)).collect();
        let konst = |name: &str| -> Result<&Tensor> { by.get(name).filter(|n| n.op == "Const").and_then(|n| n.value.as_ref()).ok_or_else(|| anyhow!("graph has no constant {name}")) };
        // The structure this implementation assumes.
        for (name, op) in [
            ("InputBatch", "Placeholder"),
            ("LengthBatch", "PlaceholderWithDefault"),
            ("transpose", "Transpose"),
            ("bidirectional_rnn/bw/ReverseSequence", "ReverseSequence"),
            ("concat", "ConcatV2"),
            ("Relu", "Relu"),
            ("Softmax", "Softmax"),
        ] {
            if by.get(name).map(|n| n.op.as_str()) != Some(op) {
                bail!("graph node {name} is not a {op}");
            }
        }
        let features = by["InputBatch"].shape.as_ref().and_then(|s| s.get(1).copied()).and_then(|n| usize::try_from(n).ok()).filter(|n| (1..=29).contains(n))
            .ok_or_else(|| anyhow!("unsupported InputBatch feature count"))?;
        let perm = &konst("transpose/perm")?.i32s;
        if perm != &[0, 2, 1] {
            bail!("unexpected input transpose {perm:?}");
        }
        let lstm = |dir: &str, cell: usize, input: usize| -> Result<Lstm> {
            let p = format!("bidirectional_rnn/{dir}/multi_rnn_cell/cell_{cell}/basic_lstm_cell");
            let k = konst(&format!("{p}/kernel"))?;
            let b = konst(&format!("{p}/bias"))?;
            let fb = konst(&format!("bidirectional_rnn/{dir}/{dir}/while/{dir}/multi_rnn_cell/cell_{cell}/basic_lstm_cell/Const_2"))?;
            let hidden = b.f32s.len() / 4;
            if hidden == 0 || hidden > 256 || k.shape != [(input + hidden) as i64, (4 * hidden) as i64] || k.f32s.len() != (input + hidden) * 4 * hidden || b.f32s.len() != 4 * hidden {
                bail!("{p}: kernel {:?} does not fit input {input}, bias {}", k.shape, b.f32s.len());
            }
            Ok(Lstm { kernel: k.f32s.clone(), bias: b.f32s.clone(), input, hidden, forget_bias: *fb.f32s.first().ok_or_else(|| anyhow!("{p}: no forget bias"))? })
        };
        let fw0 = lstm("fw", 0, features)?;
        let h = fw0.hidden;
        let fw = [fw0, lstm("fw", 1, h)?];
        let bw = [lstm("bw", 0, features)?, lstm("bw", 1, h)?];
        let fc0 = konst("bilstm/fc_out/layer0/W")?;
        let fc0_b = konst("bilstm/fc_out/layer0/Variable")?;
        let fc1 = konst("bilstm/fc_out/layer1/W")?;
        let fc1_b = konst("bilstm/fc_out/layer1/Variable")?;
        let mid = fc0_b.f32s.len();
        if fw[1].hidden != h || bw[0].hidden != h || bw[1].hidden != h || mid == 0 || mid > 64 || fc0.f32s.len() != 2 * h * mid || fc1.f32s.len() != 2 * mid || fc1_b.f32s.len() != 2 {
            bail!("dense layers do not fit: {:?} {:?}", fc0.shape, fc1.shape);
        }
        Ok(Model { fw, bw, fc0: fc0.f32s.clone(), fc0_b: fc0_b.f32s.clone(), fc1: fc1.f32s.clone(), fc1_b: fc1_b.f32s.clone(), features })
    }

    pub fn load(path: &str) -> Result<Model> {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        let mut b = Vec::new();
        file.take(MAX_GRAPH_BYTES as u64 + 1).read_to_end(&mut b)?;
        anyhow::ensure!(b.len() <= MAX_GRAPH_BYTES, "vocal graph exceeds supported size");
        Model::from_nodes(&parse_graph(&b)?)
    }

    /// `Discriminator::inference`: voice probability per frame for `feat` (frames × features,
    /// already normalised). None when there are no frames.
    pub fn run(&self, feat: &[f64]) -> Option<Vec<f32>> {
        let nf = self.features;
        let n = feat.len() / nf;
        if n == 0 {
            return None;
        }
        let x: Vec<f32> = feat.iter().map(|v| *v as f32).collect();
        let nb = ((n as i32 as f64) * f64::from_bits(0x3F76C16C16C16C17)).ceil() as i64;
        let wins: Vec<(usize, usize)> = (0..nb)
            .filter_map(|b| {
                let s = STEP as i64 * b;
                let len = if s + WINDOW as i64 <= n as i64 { WINDOW as i64 } else { n as i64 - s };
                (len >= 1).then_some((s as usize, len as usize))
            })
            .collect();
        let mut sum = vec![0f32; n];
        let mut count = vec![0u8; n];
        for group in wins.chunks(64) {
            let probs = self.windows(&x, group);
            for (w, (s, len)) in group.iter().enumerate() {
                for t in 0..*len {
                    sum[s + t] += probs[w * WINDOW + t];
                    count[s + t] += 1;
                }
            }
        }
        Some(sum.iter().zip(&count).map(|(s, c)| s / *c as f32).collect())
    }

    /// Class-1 probabilities of a group of windows (window × 360, unused steps zero).
    fn windows(&self, x: &[f32], wins: &[(usize, usize)]) -> Vec<f32> {
        let h = self.fw[1].hidden;
        let (hf, hb) = std::thread::scope(|s| {
            let a = s.spawn(|| self.direction(&self.fw, x, wins, false));
            let b = self.direction(&self.bw, x, wins, true);
            (a.join().unwrap(), b)
        });
        let rows = wins.len() * WINDOW;
        let mut cat = vec![0f32; rows * 2 * h];
        for r in 0..rows {
            cat[r * 2 * h..r * 2 * h + h].copy_from_slice(&hf[r * h..(r + 1) * h]);
            cat[r * 2 * h + h..(r + 1) * 2 * h].copy_from_slice(&hb[r * h..(r + 1) * h]);
        }
        let mid = self.fc0_b.len();
        let mut z = vec![0f32; rows * mid];
        for r in 0..rows {
            z[r * mid..(r + 1) * mid].copy_from_slice(&self.fc0_b);
        }
        sgemm(rows, mid, 2 * h, &cat, &self.fc0, &mut z, 1.0);
        for v in z.iter_mut() {
            *v = v.max(0.0);
        }
        let mut out = vec![0f32; rows];
        for r in 0..rows {
            let zr = &z[r * mid..(r + 1) * mid];
            let (mut a, mut b) = (0f32, 0f32);
            for (k, v) in zr.iter().enumerate() {
                a += v * self.fc1[2 * k];
                b += v * self.fc1[2 * k + 1];
            }
            let (a, b) = (a + self.fc1_b[0], b + self.fc1_b[1]);
            let m = a.max(b);
            let (ea, eb) = ((a - m).exp(), (b - m).exp());
            out[r] = eb / (ea + eb);
        }
        out
    }

    /// Top-layer outputs of one direction for each window position (window × 360 × hidden).
    fn direction(&self, cells: &[Lstm; 2], x: &[f32], wins: &[(usize, usize)], reverse: bool) -> Vec<f32> {
        let nf = self.features;
        let h = cells[0].hidden;
        let g = 4 * h;
        let nw = wins.len();
        let mut out = vec![0f32; nw * WINDOW * h];
        let mut st = [(vec![0f32; nw * h], vec![0f32; nw * h]), (vec![0f32; nw * h], vec![0f32; nw * h])];
        let mut inp0 = vec![0f32; nw * (nf + h)];
        let mut inp1 = vec![0f32; nw * 2 * h];
        let mut gates = vec![0f32; nw * g];
        let maxlen = wins.iter().map(|w| w.1).max().unwrap_or(0);
        for t in 0..maxlen {
            // Windows are in order of non-increasing length, so the active ones are a prefix.
            let k = wins.iter().take_while(|w| w.1 > t).count();
            for (w, (s, len)) in wins.iter().take(k).enumerate() {
                let pos = if reverse { len - 1 - t } else { t };
                let row = &mut inp0[w * (nf + h)..(w + 1) * (nf + h)];
                row[..nf].copy_from_slice(&x[(s + pos) * nf..(s + pos + 1) * nf]);
                row[nf..].copy_from_slice(&st[0].1[w * h..(w + 1) * h]);
            }
            cell_step(&cells[0], &inp0[..k * (nf + h)], k, &mut gates, &mut st[0].0, &mut st[0].1);
            for w in 0..k {
                let row = &mut inp1[w * 2 * h..(w + 1) * 2 * h];
                row[..h].copy_from_slice(&st[0].1[w * h..(w + 1) * h]);
                row[h..].copy_from_slice(&st[1].1[w * h..(w + 1) * h]);
            }
            cell_step(&cells[1], &inp1[..k * 2 * h], k, &mut gates, &mut st[1].0, &mut st[1].1);
            for (w, (_, len)) in wins.iter().take(k).enumerate() {
                let pos = if reverse { len - 1 - t } else { t };
                out[(w * WINDOW + pos) * h..(w * WINDOW + pos + 1) * h].copy_from_slice(&st[1].1[w * h..(w + 1) * h]);
            }
        }
        out
    }
}

#[inline]
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// One BasicLSTMCell step for the first `k` rows: gates = [x, h] · kernel + bias.
fn cell_step(c: &Lstm, inp: &[f32], k: usize, gates: &mut [f32], cs: &mut [f32], hs: &mut [f32]) {
    let h = c.hidden;
    let g = 4 * h;
    for r in 0..k {
        gates[r * g..(r + 1) * g].copy_from_slice(&c.bias);
    }
    sgemm(k, g, c.input + h, inp, &c.kernel, &mut gates[..k * g], 1.0);
    for r in 0..k {
        let gr = &gates[r * g..(r + 1) * g];
        let (cr, hr) = (&mut cs[r * h..(r + 1) * h], &mut hs[r * h..(r + 1) * h]);
        for u in 0..h {
            let (i, j, f, o) = (gr[u], gr[h + u], gr[2 * h + u], gr[3 * h + u]);
            let nc = cr[u] * sigmoid(f + c.forget_bias) + sigmoid(i) * j.tanh();
            cr[u] = nc;
            hr[u] = nc.tanh() * sigmoid(o);
        }
    }
}

// ------------------------------------------------------------------------------------------
// Matrix product: C[m×n] = A[m×k] · B[k×n] + beta·C (row-major)
// ------------------------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[link(name = "Accelerate", kind = "framework")]
extern "C" {
    fn cblas_sgemm(order: i32, ta: i32, tb: i32, m: i32, n: i32, k: i32, alpha: f32, a: *const f32, lda: i32, b: *const f32, ldb: i32, beta: f32, c: *mut f32, ldc: i32);
}

pub fn sgemm(m: usize, n: usize, k: usize, a: &[f32], b: &[f32], c: &mut [f32], beta: f32) {
    assert!(a.len() >= m * k && b.len() >= k * n && c.len() >= m * n);
    if m == 0 {
        return;
    }
    #[cfg(target_os = "macos")]
    unsafe {
        // 101 = row major, 111 = no transpose.
        cblas_sgemm(101, 111, 111, m as i32, n as i32, k as i32, 1.0, a.as_ptr(), k as i32, b.as_ptr(), n as i32, beta, c.as_mut_ptr(), n as i32);
    }
    #[cfg(not(target_os = "macos"))]
    sgemm_plain(m, n, k, a, b, c, beta);
}

#[allow(dead_code)]
fn sgemm_plain(m: usize, n: usize, k: usize, a: &[f32], b: &[f32], c: &mut [f32], beta: f32) {
    for i in 0..m {
        let cr = &mut c[i * n..(i + 1) * n];
        for v in cr.iter_mut() {
            *v *= beta;
        }
        for p in 0..k {
            let av = a[i * k + p];
            let br = &b[p * n..(p + 1) * n];
            for (cv, bv) in cr.iter_mut().zip(br) {
                *cv += av * bv;
            }
        }
    }
}

static M: std::sync::OnceLock<Option<Model>> = std::sync::OnceLock::new();

/// Use a compatible local graph only when the user has supplied its absolute path.
/// The default STEMS detector runs independently of any AlphaTheta installation.
pub fn configured() -> Option<&'static Model> {
    M.get_or_init(|| {
        let path = std::env::var("RECORDBOX_VOCAL_GRAPH").ok()?;
        if !std::path::Path::new(&path).is_absolute() {
            eprintln!("vocal detection: RECORDBOX_VOCAL_GRAPH needs an absolute path");
            return None;
        }
        match Model::load(&path) {
            Ok(m) if m.features == crate::vocal_rb::FEATURES => Some(m),
            Ok(_) => {
                eprintln!("vocal detection: incompatible feature count; using STEMS");
                None
            }
            Err(e) => {
                eprintln!("vocal detection: local graph unavailable ({e}); using STEMS");
                None
            }
        }
    }).as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tiny protobuf writer for building test graphs.
    fn put_varint(o: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                o.push(b);
                return;
            }
            o.push(b | 0x80);
        }
    }
    fn put_bytes(o: &mut Vec<u8>, field: u32, b: &[u8]) {
        put_varint(o, ((field as u64) << 3) | 2);
        put_varint(o, b.len() as u64);
        o.extend_from_slice(b);
    }
    fn put_int(o: &mut Vec<u8>, field: u32, v: u64) {
        put_varint(o, (field as u64) << 3);
        put_varint(o, v);
    }
    fn shape(dims: &[i64]) -> Vec<u8> {
        let mut s = Vec::new();
        for d in dims {
            let mut dim = Vec::new();
            put_int(&mut dim, 1, *d as u64);
            put_bytes(&mut s, 2, &dim);
        }
        s
    }
    fn const_f32(name: &str, dims: &[i64], vals: &[f32], packed_content: bool) -> Vec<u8> {
        let mut t = Vec::new();
        put_int(&mut t, 1, 1);
        put_bytes(&mut t, 2, &shape(dims));
        let raw: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
        if packed_content {
            put_bytes(&mut t, 4, &raw);
        } else {
            put_bytes(&mut t, 5, &raw);
        }
        node(name, "Const", &[], Some(("value", 8, t)))
    }
    fn const_i32(name: &str, dims: &[i64], vals: &[i32]) -> Vec<u8> {
        let mut t = Vec::new();
        put_int(&mut t, 1, 3);
        put_bytes(&mut t, 2, &shape(dims));
        let mut p = Vec::new();
        for v in vals {
            put_varint(&mut p, *v as u32 as u64);
        }
        put_bytes(&mut t, 7, &p);
        node(name, "Const", &[], Some(("value", 8, t)))
    }
    fn node(name: &str, op: &str, inputs: &[&str], attr: Option<(&str, u32, Vec<u8>)>) -> Vec<u8> {
        let mut n = Vec::new();
        put_bytes(&mut n, 1, name.as_bytes());
        put_bytes(&mut n, 2, op.as_bytes());
        for i in inputs {
            put_bytes(&mut n, 3, i.as_bytes());
        }
        if let Some((k, f, v)) = attr {
            let mut av = Vec::new();
            put_bytes(&mut av, f, &v);
            let mut e = Vec::new();
            put_bytes(&mut e, 1, k.as_bytes());
            put_bytes(&mut e, 2, &av);
            put_bytes(&mut n, 5, &e);
        }
        let mut g = Vec::new();
        put_bytes(&mut g, 1, &n);
        g
    }
    fn rnd(seed: &mut u64, n: usize, scale: f32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((*seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * scale
            })
            .collect()
    }
    /// A complete small graph of the expected structure (features 3, hidden 4, dense 5).
    #[test]
    fn scalar_constants_cannot_exceed_the_expansion_budget() {
        let small = const_f32("small", &[3], &[1.0], false);
        let large = const_f32("large", &[MAX_GRAPH_VALUES as i64], &[0.0], false);
        assert!(parse_graph(&[small, large].concat()).is_err());
    }

    fn tiny_graph(seed: u64) -> Vec<u8> {
        let (nf, h, mid) = (3usize, 4usize, 5usize);
        let mut s = seed;
        let mut g = Vec::new();
        g.extend(node("InputBatch", "Placeholder", &[], Some(("shape", 7, shape(&[-1, nf as i64, -1])))));
        g.extend(node("LengthBatch", "PlaceholderWithDefault", &["mul"], None));
        g.extend(const_i32("transpose/perm", &[3], &[0, 2, 1]));
        for name in ["transpose", "bidirectional_rnn/bw/ReverseSequence", "concat", "Relu", "Softmax"] {
            let op = match name {
                "transpose" => "Transpose",
                "concat" => "ConcatV2",
                "Relu" => "Relu",
                "Softmax" => "Softmax",
                _ => "ReverseSequence",
            };
            g.extend(node(name, op, &[], None));
        }
        for dir in ["fw", "bw"] {
            for (cell, inp) in [(0usize, nf), (1, h)] {
                let p = format!("bidirectional_rnn/{dir}/multi_rnn_cell/cell_{cell}/basic_lstm_cell");
                g.extend(const_f32(&format!("{p}/kernel"), &[(inp + h) as i64, (4 * h) as i64], &rnd(&mut s, (inp + h) * 4 * h, 1.6), true));
                g.extend(const_f32(&format!("{p}/bias"), &[(4 * h) as i64], &rnd(&mut s, 4 * h, 0.4), true));
                g.extend(const_f32(&format!("bidirectional_rnn/{dir}/{dir}/while/{dir}/multi_rnn_cell/cell_{cell}/basic_lstm_cell/Const_2"), &[], &[1.0], false));
            }
        }
        g.extend(const_f32("bilstm/fc_out/layer0/W", &[(2 * h) as i64, mid as i64], &rnd(&mut s, 2 * h * mid, 1.5), true));
        g.extend(const_f32("bilstm/fc_out/layer0/Variable", &[1, mid as i64], &rnd(&mut s, mid, 0.3), true));
        g.extend(const_f32("bilstm/fc_out/layer1/W", &[mid as i64, 2], &rnd(&mut s, 2 * mid, 1.5), true));
        g.extend(const_f32("bilstm/fc_out/layer1/Variable", &[1, 2], &[0.1, -0.1], false));
        g
    }

    #[test]
    fn reads_graph_constants() {
        let b = tiny_graph(7);
        let nodes = parse_graph(&b).unwrap();
        let k = nodes.iter().find(|n| n.name == "bidirectional_rnn/fw/multi_rnn_cell/cell_0/basic_lstm_cell/kernel").unwrap();
        let t = k.value.as_ref().unwrap();
        assert_eq!(t.shape, vec![7, 16]);
        assert_eq!(t.f32s.len(), 112);
        let p = nodes.iter().find(|n| n.name == "transpose/perm").unwrap();
        assert_eq!(p.value.as_ref().unwrap().i32s, vec![0, 2, 1]);
        let fb = nodes.iter().find(|n| n.name.ends_with("cell_1/basic_lstm_cell/Const_2")).unwrap();
        assert_eq!(fb.value.as_ref().unwrap().f32s, vec![1.0]);
        assert_eq!(nodes.iter().find(|n| n.name == "InputBatch").unwrap().shape, Some(vec![-1, 3, -1]));
        let m = Model::from_nodes(&nodes).unwrap();
        assert_eq!((m.features, m.fw[0].hidden, m.fw[1].input, m.fc0_b.len()), (3, 4, 4, 5));
        // A truncated file is refused, not misread.
        assert!(parse_graph(&b[..b.len() - 3]).is_err());
        // A graph without the expected structure is refused.
        let bad: Vec<Node> = nodes.into_iter().filter(|n| n.name != "Softmax").collect();
        assert!(Model::from_nodes(&bad).is_err());
    }

    /// Straightforward per-window reference of the same network.
    fn reference(m: &Model, x: &[f32], s: usize, len: usize) -> Vec<f32> {
        let nf = m.features;
        let run = |cells: &[Lstm; 2], order: Vec<usize>| -> Vec<Vec<f32>> {
            let h = cells[0].hidden;
            let mut state = vec![(vec![0f32; h], vec![0f32; h]); 2];
            let mut out = vec![vec![0f32; h]; len];
            for pos in order {
                let mut inp: Vec<f32> = x[(s + pos) * nf..(s + pos + 1) * nf].to_vec();
                for (l, c) in cells.iter().enumerate() {
                    inp.extend_from_slice(&state[l].1);
                    let g: Vec<f32> = (0..4 * h).map(|j| c.bias[j] + inp.iter().enumerate().map(|(r, v)| v * c.kernel[r * 4 * h + j]).sum::<f32>()).collect();
                    for u in 0..h {
                        let nc = state[l].0[u] * sigmoid(g[2 * h + u] + c.forget_bias) + sigmoid(g[u]) * g[h + u].tanh();
                        state[l].0[u] = nc;
                        state[l].1[u] = nc.tanh() * sigmoid(g[3 * h + u]);
                    }
                    inp = state[l].1.clone();
                }
                out[pos] = inp;
            }
            out
        };
        let f = run(&m.fw, (0..len).collect());
        let b = run(&m.bw, (0..len).rev().collect());
        (0..len)
            .map(|t| {
                let cat: Vec<f32> = f[t].iter().chain(&b[t]).copied().collect();
                let mid = m.fc0_b.len();
                let z: Vec<f32> = (0..mid).map(|j| (m.fc0_b[j] + cat.iter().enumerate().map(|(r, v)| v * m.fc0[r * mid + j]).sum::<f32>()).max(0.0)).collect();
                let a = m.fc1_b[0] + z.iter().enumerate().map(|(k, v)| v * m.fc1[2 * k]).sum::<f32>();
                let bb = m.fc1_b[1] + z.iter().enumerate().map(|(k, v)| v * m.fc1[2 * k + 1]).sum::<f32>();
                1.0 / (1.0 + (a - bb).exp())
            })
            .collect()
    }

    #[test]
    fn batched_forward_matches_reference() {
        let m = Model::from_nodes(&parse_graph(&tiny_graph(11)).unwrap()).unwrap();
        // 500 frames: windows (0,360) (180,320) (360,140); frames covered once or twice.
        let n = 500;
        let mut s = 3u64;
        let feat: Vec<f64> = rnd(&mut s, n * 3, 4.0).into_iter().map(|v| v as f64).collect();
        let got = m.run(&feat).unwrap();
        let x: Vec<f32> = feat.iter().map(|v| *v as f32).collect();
        let w: Vec<Vec<f32>> = [(0, 360), (180, 320), (360, 140)].iter().map(|(s, l)| reference(&m, &x, *s, *l)).collect();
        for t in 0..n {
            let mut acc = Vec::new();
            for (k, (s, l)) in [(0usize, 360usize), (180, 320), (360, 140)].iter().enumerate() {
                if t >= *s && t < s + l {
                    acc.push(w[k][t - s]);
                }
            }
            let want = acc.iter().sum::<f32>() / acc.len() as f32;
            assert!((got[t] - want).abs() < 2e-5, "frame {t}: {} vs {want}", got[t]);
        }
    }

    #[test]
    fn window_count_follows_the_original() {
        // ceil(n · (1/180 rounded up)): an exact multiple of 180 gets one extra, empty window.
        let nb = |n: usize| ((n as i32 as f64) * f64::from_bits(0x3F76C16C16C16C17)).ceil() as i64;
        assert_eq!(nb(179), 1);
        assert_eq!(nb(181), 2);
        assert_eq!(nb(500), 3);
    }
}
