//! HTDemucs on Metal with a handful of purpose-built GPU kernels.
//!
//! Candle's stock Metal kernels for strided copies, broadcasts and im2col do 64-bit index
//! arithmetic per element and are about ten times slower than its contiguous kernels and its
//! matrix multiply. This module runs the same graph as `htdemucs::Graph::forward` using only
//! contiguous tensors: small kernels build the matrix-multiply inputs directly in the layout
//! the multiply wants (channels first, bias as an extra row of ones), and normalisation,
//! activation, layer scale and the residual add are fused into one pass.
//!
//! All activations are f32, batch 1, laid out (channels, frequency, time); the time branch
//! uses frequency = 1. Transformer tokens are (tokens, channels) in (frequency, time) order.

use candle_core::backend::BackendStorage;
use candle_core::{CpuStorage, CustomOp1, CustomOp2, DType, Layout, MetalDevice, MetalStorage, Result, Shape, Storage, Tensor};
use candle_metal_kernels::metal::{Buffer, ComputePipeline, Library};
use objc2_metal::{MTLResourceOptions, MTLSize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[path = "metal_tensor.rs"]
mod metal_tensor;

const KERNELS: &str = r#"
#include <metal_stdlib>
using namespace metal;

// Same formulas as candle's own kernels, so results match its gelu_erf and sigmoid.
static inline float erf_as(float x) {
    const float a1 = 0.254829592, a2 = -0.284496736, a3 = 1.421413741, a4 = -1.453152027, a5 = 1.061405429, p = 0.3275911;
    float s = x < 0 ? -1.0 : 1.0;
    x = fabs(x);
    float t = 1.0 / (1.0 + p * x);
    float y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * exp(-x * x);
    return s * y;
}
static inline float gelu_erf(float x) { return x * (1 + erf_as(x * M_SQRT1_2_F)) / 2; }
static inline float sigmoid(float x) { return 1.0 / (1 + exp(-x)); }

// src (C, Fin, Tin) -> dst (C*kf*kt [+1 row of ones], Fo, To), rows in weight order.
kernel void im2col(device const float *src [[buffer(0)]], device float *dst [[buffer(1)]], constant uint *p [[buffer(2)]],
                   uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0], Fin = p[1], Tin = p[2], kf = p[3], kt = p[4], sf = p[5], st = p[6], pf = p[7], pt = p[8], df = p[9], dt = p[10], Fo = p[11], To = p[12];
    uint r = gid.z;
    uint o = (r * Fo + gid.y) * To + gid.x;
    uint kk = kf * kt;
    if (r >= C * kk) { dst[o] = 1.0; return; }
    uint c = r / kk;
    uint k = r - c * kk;
    uint kfi = k / kt;
    uint kti = k - kfi * kt;
    int f = int(gid.y * sf + kfi * df) - int(pf);
    int t = int(gid.x * st + kti * dt) - int(pt);
    dst[o] = (f < 0 || f >= int(Fin) || t < 0 || t >= int(Tin)) ? 0.0 : src[(c * Fin + uint(f)) * Tin + uint(t)];
}

// Partial sums for group norm: x (C, F, T) -> out (2, F, C, nb): sum and sum of squares of
// each block of `blk` samples along T.
kernel void row_stats(device const float *x [[buffer(0)]], device float *out [[buffer(1)]], constant uint *p [[buffer(2)]],
                      uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0], F = p[1], T = p[2], blk = p[3], nb = p[4];
    uint b = gid.x, c = gid.y, f = gid.z;
    uint t0 = b * blk, t1 = min(T, t0 + blk);
    uint base = (c * F + f) * T;
    float s = 0, q = 0;
    for (uint t = t0; t < t1; t++) { float v = x[base + t]; s += v; q += v * v; }
    uint o = (f * C + c) * nb + b;
    out[o] = s;
    out[F * C * nb + o] = q;
}

// Group norm (one group; statistics per frequency row when G == F, over everything when
// G == 1, none when G == 0), then activation (1 GELU, 2 GLU over the channel halves), then
// optionally res + layer_scale * y, with an optional extra channel of ones.
// stats: [sum(G), sum of squares(G)]; cst: [weight(Cin), bias(Cin), layer scale(Cout)].
kernel void norm_act(device const float *x [[buffer(0)]], device const float *stats [[buffer(1)]], device const float *cst [[buffer(2)]],
                     device const float *res [[buffer(3)]], device float *out [[buffer(4)]], constant uint *p [[buffer(5)]],
                     uint3 gid [[thread_position_in_grid]]) {
    uint Cin = p[0], F = p[1], T = p[2], G = p[3], act = p[4], has_res = p[5];
    float n = as_type<float>(p[6]), eps = as_type<float>(p[7]);
    uint Cout = act == 2 ? Cin / 2 : Cin;
    uint c = gid.z, f = gid.y;
    uint o = (c * F + f) * T + gid.x;
    if (c >= Cout) { out[o] = 1.0; return; }
    float a = x[o];
    float m = 0, inv = 1;
    if (G > 0) {
        uint g = G == 1 ? 0 : f;
        m = stats[g] / n;
        float var = max(stats[G + g] / n - m * m, 0.0f);
        inv = 1.0 / sqrt(var + eps);
        a = (a - m) * inv * cst[c] + cst[Cin + c];
    }
    if (act == 1) {
        a = gelu_erf(a);
    } else if (act == 2) {
        float b = x[o + Cout * F * T];
        if (G > 0) { b = (b - m) * inv * cst[c + Cout] + cst[Cin + c + Cout]; }
        a = a * sigmoid(b);
    }
    if (has_res != 0) { a = res[o] + cst[2 * Cin + c] * a; }
    out[o] = a;
}

// Transposed convolution, kernel 8, stride 4, cropped by 2 at the start: cols (Co*8, F, T)
// from the matrix multiply -> out (Co, Fo, To), plus bias and optional GELU. axis 0: along
// frequency, 1: along time.
kernel void tr_col2im(device const float *cols [[buffer(0)]], device const float *bias [[buffer(1)]], device float *out [[buffer(2)]],
                      constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]]) {
    uint F = p[1], T = p[2], axis = p[3], Fo = p[4], To = p[5], act = p[6];
    uint co = gid.z;
    uint pos = (axis == 0 ? gid.y : gid.x) + 2;
    uint j = pos / 4, i = pos % 4;
    uint n = axis == 0 ? F : T;
    float v = bias[co];
    if (j < n) {
        uint f = axis == 0 ? j : gid.y, t = axis == 0 ? gid.x : j;
        v += cols[((co * 8 + i) * F + f) * T + t];
    }
    if (j >= 1 && j - 1 < n) {
        uint f = axis == 0 ? j - 1 : gid.y, t = axis == 0 ? gid.x : j - 1;
        v += cols[((co * 8 + i + 4) * F + f) * T + t];
    }
    if (act == 1) { v = gelu_erf(v); }
    out[(co * Fo + gid.y) * To + gid.x] = v;
}

// a + b, same size.
kernel void add2(device const float *a [[buffer(0)]], device const float *b [[buffer(1)]], device float *out [[buffer(2)]],
                 constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]]) {
    uint o = gid.y * p[0] + gid.x;
    if (o < p[1]) { out[o] = a[o] + b[o]; }
}

// x (C, F, T) + e (C, F).
kernel void add_cf(device const float *x [[buffer(0)]], device const float *e [[buffer(1)]], device float *out [[buffer(2)]],
                   constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]]) {
    uint F = p[1], T = p[2];
    uint o = (gid.z * F + gid.y) * T + gid.x;
    out[o] = x[o] + e[gid.z * F + gid.y];
}

// Layer norm along the channels of x (L, C); cst: [weight(C), bias(C)]; optional last
// column of ones.
kernel void ln_rows(device const float *x [[buffer(0)]], device const float *cst [[buffer(1)]], device float *out [[buffer(2)]],
                    constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0], ones = p[1];
    float eps = as_type<float>(p[2]);
    uint base = gid.x * C;
    float s = 0;
    for (uint c = 0; c < C; c++) { s += x[base + c]; }
    float m = s / float(C);
    float q = 0;
    for (uint c = 0; c < C; c++) { float d = x[base + c] - m; q += d * d; }
    float inv = 1.0 / sqrt(q / float(C) + eps);
    uint ob = gid.x * (C + ones);
    for (uint c = 0; c < C; c++) { out[ob + c] = (x[base + c] - m) * inv * cst[c] + cst[C + c]; }
    if (ones != 0) { out[ob + C] = 1.0; }
}

// Attention heads: src (L, W), columns [off, off + H*D) -> dst (H, L, D).
kernel void split_heads(device const float *src [[buffer(0)]], device float *dst [[buffer(1)]], constant uint *p [[buffer(2)]],
                        uint3 gid [[thread_position_in_grid]]) {
    uint L = p[0], W = p[1], off = p[2], D = p[3];
    uint d = gid.x, l = gid.y, h = gid.z;
    dst[(h * L + l) * D + d] = src[l * W + off + h * D + d];
}

// src (H, L, D) -> dst (L, H*D + 1); the last column is ones.
kernel void merge_heads(device const float *src [[buffer(0)]], device float *dst [[buffer(1)]], constant uint *p [[buffer(2)]],
                        uint3 gid [[thread_position_in_grid]]) {
    uint L = p[0], H = p[1], D = p[2];
    uint c = gid.x, l = gid.y;
    uint W = H * D + 1;
    dst[l * W + c] = c >= H * D ? 1.0 : src[((c / D) * L + l) * D + (c % D)];
}

// x (L, C) -> GELU(x) with a last column of ones: (L, C + 1).
kernel void row_gelu(device const float *x [[buffer(0)]], device float *out [[buffer(1)]], constant uint *p [[buffer(2)]],
                     uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0];
    out[gid.y * (C + 1) + gid.x] = gid.x >= C ? 1.0 : gelu_erf(x[gid.y * C + gid.x]);
}

// Group norm (one group over the whole of x (L, C)) with per-channel weight and bias;
// stats: [sum, sum of squares]; optional last column of ones.
kernel void row_norm(device const float *x [[buffer(0)]], device const float *stats [[buffer(1)]], device const float *cst [[buffer(2)]],
                     device float *out [[buffer(3)]], constant uint *p [[buffer(4)]], uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0], ones = p[1];
    float n = as_type<float>(p[2]), eps = as_type<float>(p[3]);
    uint c = gid.x;
    uint o = gid.y * (C + ones) + c;
    if (c >= C) { out[o] = 1.0; return; }
    float m = stats[0] / n;
    float var = max(stats[1] / n - m * m, 0.0f);
    out[o] = (x[gid.y * C + c] - m) / sqrt(var + eps) * cst[c] + cst[C + c];
}
"#;

static LIBRARIES: Mutex<Vec<(u64, Arc<Library>)>> = Mutex::new(Vec::new());
static PIPELINES: Mutex<Vec<(u64, &'static str, Arc<ComputePipeline>)>> = Mutex::new(Vec::new());

fn msg<E: std::fmt::Display>(e: E) -> candle_core::Error {
    candle_core::Error::Msg(e.to_string())
}

fn pipeline(dev: &MetalDevice, name: &'static str) -> Result<Arc<ComputePipeline>> {
    let id = dev.metal_device().registry_id();
    let mut pipes = PIPELINES.lock().map_err(msg)?;
    if let Some(p) = pipes.iter().find(|p| p.0 == id && p.1 == name) {
        return Ok(p.2.clone());
    }
    let mut libs = LIBRARIES.lock().map_err(msg)?;
    let lib = match libs.iter().find(|l| l.0 == id) {
        Some(l) => l.1.clone(),
        None => {
            let lib = Arc::new(dev.metal_device().new_library_with_source(KERNELS, None).map_err(msg)?);
            libs.push((id, lib.clone()));
            lib
        }
    };
    let func = lib.get_function(name, None).map_err(msg)?;
    let pipe = Arc::new(dev.metal_device().new_compute_pipeline_state_with_function(&func).map_err(msg)?);
    pipes.push((id, name, pipe.clone()));
    Ok(pipe)
}

/// Output buffers of this module's operations. Candle's own pool releases every unused
/// buffer each time it commits a command buffer (every 50 operations), so the large
/// intermediates of a segment would be freed and allocated again several times per segment
/// (measured: about 15 % of a segment's time). They are kept here until `release_buffers`.
/// Buffers are only reused on the device (command queue) that made them, since that is
/// what keeps the order of use.
static POOL: Mutex<Vec<(candle_core::metal_backend::DeviceId, Arc<Buffer>)>> = Mutex::new(Vec::new());

fn alloc(dev: &MetalDevice, elements: usize) -> Result<Arc<Buffer>> {
    let bytes = elements.max(1) * 4;
    let mut pool = POOL.lock().map_err(msg)?;
    // A buffer nobody else holds is free: commands already encoded against it run before
    // anything encoded later.
    let id = dev.id();
    let free = pool.iter().filter(|(d, b)| *d == id && Arc::strong_count(b) == 1 && b.length() >= bytes && b.length() <= bytes + bytes / 4 + 4096).min_by_key(|(_, b)| b.length());
    if let Some((_, b)) = free {
        return Ok(b.clone());
    }
    let buffer = Arc::new(dev.metal_device().new_buffer(bytes, MTLResourceOptions::StorageModeShared).map_err(msg)?);
    pool.push((id, buffer.clone()));
    Ok(buffer)
}

/// Give back the memory held for intermediates (call when a separation ends).
pub fn release_buffers() {
    if let Ok(mut pool) = POOL.lock() {
        // Buffers still referenced by live tensors are freed with those tensors.
        pool.clear();
    }
}

/// The values of a contiguous f32 tensor, copied straight out of its (shared-memory) buffer
/// once the GPU has finished. `Tensor::to_vec1` copies the whole underlying buffer first,
/// which for one source of four is four times too much.
pub fn read(t: &Tensor) -> Result<Vec<f32>> {
    t.device().synchronize()?;
    let (storage, layout) = t.storage_and_layout();
    let Storage::Metal(s) = &*storage else { candle_core::bail!("read: tensor is not on Metal") };
    let count = layout.shape().elem_count();
    if !layout.is_contiguous() || s.dtype() != DType::F32 || (layout.start_offset() + count) * 4 > s.buffer().length() {
        candle_core::bail!("read: tensor must be contiguous f32")
    }
    // SAFETY: the buffer is CPU-visible shared memory of at least `start + count` f32s (checked
    // above), it stays alive through `storage`, and every command writing it has completed.
    Ok(unsafe { std::slice::from_raw_parts((s.buffer().contents() as *const f32).add(layout.start_offset()), count) }.to_vec())
}

/// Candle's matrix multiply kernel, writing into a pooled buffer. Operands (.., m, k) and
/// (.., k, n) with the same leading dimensions; either may be a transposed view.
struct Gemm;

impl CustomOp2 for Gemm {
    fn name(&self) -> &'static str {
        "gemm"
    }
    fn cpu_fwd(&self, _: &CpuStorage, _: &Layout, _: &CpuStorage, _: &Layout) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("gemm runs on Metal only")
    }
    fn metal_fwd(&self, lhs: &MetalStorage, ll: &Layout, rhs: &MetalStorage, rl: &Layout) -> Result<(MetalStorage, Shape)> {
        let (ld, rd) = (ll.dims(), rl.dims());
        let rank = ld.len();
        if rank < 2 || rd.len() != rank || ld[..rank - 2] != rd[..rank - 2] || ld[rank - 1] != rd[rank - 2] {
            candle_core::bail!("gemm: shapes {ld:?} and {rd:?} do not multiply")
        }
        let (b, m, k, n) = (ld[..rank - 2].iter().product::<usize>(), ld[rank - 2], ld[rank - 1], rd[rank - 1]);
        let dev = lhs.device().clone();
        let out = alloc(&dev, b * m * n)?;
        let enc = dev.command_encoder()?;
        enc.set_label("gemm");
        candle_metal_kernels::call_mlx_gemm(dev.metal_device(), &enc, dev.kernels(), candle_metal_kernels::GemmDType::F32, (b, m, n, k), ll.stride(), ll.start_offset() * 4, lhs.buffer(), rl.stride(), rl.start_offset() * 4, rhs.buffer(), &out)
            .map_err(msg)?;
        drop(enc);
        let mut shape = ld[..rank - 2].to_vec();
        shape.extend([m, n]);
        Ok((MetalStorage::new(out, dev, b * m * n, DType::F32), Shape::from(shape)))
    }
}

fn mm(a: &Tensor, b: &Tensor) -> Result<Tensor> {
    metal_tensor::multiply(a, b).unwrap_or_else(|| a.apply_op2_no_bwd(b, &Gemm))
}

/// a + b for contiguous tensors of the same shape.
fn add(a: &Tensor, b: &Tensor) -> Result<Tensor> {
    if a.dims() != b.dims() {
        candle_core::bail!("add: shapes differ")
    }
    let n = a.elem_count();
    let width = n.min(1 << 14);
    run("add2", a, &[b], vec![width as u32, n as u32], a.dims(), (width, n.div_ceil(width), 1))
}

/// One kernel call: buffers are the input, then `extra`, then the output, then `params`.
struct Kernel {
    name: &'static str,
    extra: Vec<Tensor>,
    params: Vec<u32>,
    out: Vec<usize>,
    /// Threads along x, y, z (innermost dimension first).
    grid: (usize, usize, usize),
}

impl CustomOp1 for Kernel {
    fn name(&self) -> &'static str {
        self.name
    }
    fn cpu_fwd(&self, _: &CpuStorage, _: &Layout) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("{} runs on Metal only", self.name)
    }
    fn metal_fwd(&self, storage: &MetalStorage, layout: &Layout) -> Result<(MetalStorage, Shape)> {
        if !layout.is_contiguous() || storage.dtype() != DType::F32 {
            candle_core::bail!("{}: input must be contiguous f32", self.name)
        }
        let dev = storage.device().clone();
        let count: usize = self.out.iter().product();
        let out = alloc(&dev, count)?;
        let pipe = pipeline(&dev, self.name)?;
        // Hold the extra tensors' storage for the duration of the encoding.
        let extra: Vec<_> = self.extra.iter().map(|t| t.storage_and_layout()).collect();
        let enc = dev.command_encoder()?;
        let encoder = enc.as_ref();
        encoder.set_label(self.name);
        encoder.set_compute_pipeline_state(&pipe);
        // Candle tracks buffer reads and writes to order dispatches using
        // untracked buffers. Keep the command guard alive through dispatch.
        encoder.set_input_buffer(0, Some(storage.buffer()), layout.start_offset() * 4);
        for (i, (s, l)) in extra.iter().enumerate() {
            let Storage::Metal(s) = &**s else { candle_core::bail!("{}: inputs must be on Metal", self.name) };
            if !l.is_contiguous() || s.dtype() != DType::F32 {
                candle_core::bail!("{}: input {} must be contiguous f32", self.name, i + 1)
            }
            encoder.set_input_buffer(i + 1, Some(s.buffer()), l.start_offset() * 4);
        }
        let k = extra.len() + 1;
        encoder.set_output_buffer(k, Some(&out), 0);
        encoder.set_bytes_directly(k + 1, self.params.len() * 4, self.params.as_ptr() as *const std::ffi::c_void);
        // Thread group: powers of two per axis, within the pipeline's limit.
        let limit = pipe.max_total_threads_per_threadgroup().max(1);
        let mut group = candle_metal_kernels::utils::get_block_dims(self.grid.0, self.grid.1, self.grid.2);
        while group.width * group.height * group.depth > limit {
            if group.depth > 1 {
                group.depth /= 2
            } else if group.height > 1 {
                group.height /= 2
            } else {
                group.width /= 2
            }
        }
        encoder.dispatch_threads(MTLSize { width: self.grid.0, height: self.grid.1, depth: self.grid.2 }, group);
        drop(enc);
        Ok((MetalStorage::new(out, dev, count, DType::F32), Shape::from(self.out.clone())))
    }
}

fn run(name: &'static str, x: &Tensor, extra: &[&Tensor], params: Vec<u32>, out: &[usize], grid: (usize, usize, usize)) -> Result<Tensor> {
    x.apply_op1_no_bwd(&Kernel { name, extra: extra.iter().map(|t| (*t).clone()).collect(), params, out: out.to_vec(), grid })
}

/// Convolution geometry along (frequency, time).
#[derive(Clone, Copy)]
struct Geo {
    k: (usize, usize),
    s: (usize, usize),
    p: (usize, usize),
    d: (usize, usize),
}

/// The matrix-multiply input of a convolution over x (C, F, T), with a last row of ones for
/// the bias: (C*kf*kt + 1, Fo*To). Returns it with (Fo, To).
fn im2col(x: &Tensor, g: Geo, out: (usize, usize)) -> Result<Tensor> {
    let (c, f, t) = x.dims3()?;
    let rows = c * g.k.0 * g.k.1 + 1;
    let params = [c, f, t, g.k.0, g.k.1, g.s.0, g.s.1, g.p.0, g.p.1, g.d.0, g.d.1, out.0, out.1].map(|v| v as u32).to_vec();
    run("im2col", x, &[], params, &[rows, out.0 * out.1], (out.1, out.0, rows))
}

/// Group-norm statistics of x (C, F, T): [sums, sums of squares] per group (per frequency
/// row when `per_f`, else one group).
fn stats(x: &Tensor, per_f: bool) -> Result<Tensor> {
    let (c, f, t) = x.dims3()?;
    let blk = if t <= 512 { t } else { 256 };
    let nb = t.div_ceil(blk);
    let s = run("row_stats", x, &[], [c, f, t, blk, nb].map(|v| v as u32).to_vec(), &[2, f, c, nb], (nb, c, f))?;
    let groups = if per_f { f } else { 1 };
    s.reshape((2 * groups, (f / groups) * c * nb))?.sum_keepdim(1)?.flatten_all()
}

struct Norm<'a> {
    /// [weight, bias] of the group norm (+ layer scale when there is a residual).
    cst: &'a Tensor,
    per_f: bool,
}

/// norm (optional) -> activation (0 none, 1 GELU, 2 GLU) -> optional residual with layer
/// scale; `ones` appends a channel of ones for the next layer's bias.
fn norm_act(x: &Tensor, norm: Option<Norm>, act: u32, res: Option<&Tensor>, ones: bool) -> Result<Tensor> {
    let (c, f, t) = x.dims3()?;
    let cout = if act == 2 { c / 2 } else { c };
    let (st, cst, groups, n) = match &norm {
        Some(nm) => {
            let groups = if nm.per_f { f } else { 1 };
            (stats(x, nm.per_f)?, nm.cst.clone(), groups, (c * f * t / groups) as f32)
        }
        None => (x.clone(), x.clone(), 0, 1.0),
    };
    let params = vec![c as u32, f as u32, t as u32, groups as u32, act, res.is_some() as u32, n.to_bits(), 1e-5f32.to_bits()];
    let ch = cout + ones as usize;
    run("norm_act", x, &[&st, &cst, res.unwrap_or(x)], params, &[ch, f, t], (t, f, ch))
}

struct DconvLayer {
    conv: Tensor,
    norm1: Tensor,
    expand: Tensor,
    norm2: Tensor,
    dilation: usize,
}

struct Dconv(Vec<DconvLayer>);

impl Dconv {
    /// x (C, F, T) -> (C [+1 ones], F, T). In the frequency branch the block runs along time
    /// separately for every frequency row.
    fn forward(&self, x: &Tensor, per_f: bool, ones: bool) -> Result<Tensor> {
        let (_, f, t) = x.dims3()?;
        let mut x = x.clone();
        for (i, l) in self.0.iter().enumerate() {
            let geo = Geo { k: (1, 3), s: (1, 1), p: (0, l.dilation), d: (1, l.dilation) };
            let y = mm(&l.conv, &im2col(&x, geo, (f, t))?)?;
            let h = y.dim(0)?;
            let y = norm_act(&y.reshape((h, f, t))?, Some(Norm { cst: &l.norm1, per_f }), 1, None, true)?;
            let z = mm(&l.expand, &y.reshape((h + 1, f * t))?)?;
            let c2 = z.dim(0)?;
            x = norm_act(&z.reshape((c2, f, t))?, Some(Norm { cst: &l.norm2, per_f }), 2, Some(&x), ones && i + 1 == self.0.len())?;
        }
        Ok(x)
    }
}

struct Encoder {
    conv: Tensor,
    dconv: Dconv,
    rewrite: Tensor,
}

struct Decoder {
    rewrite: Tensor,
    dconv: Dconv,
    /// Transposed convolution as a matrix (Co*8, C) and its bias.
    tr: Tensor,
    tr_bias: Tensor,
}

const HEADS: usize = 8;

fn ln_rows(x: &Tensor, cst: &Tensor, ones: bool) -> Result<Tensor> {
    let (l, c) = x.dims2()?;
    run("ln_rows", x, &[cst], vec![c as u32, ones as u32, 1e-5f32.to_bits()], &[l, c + ones as usize], (l, 1, 1))
}

/// Columns [off, off + c) of x (L, W) as attention heads (1, heads, L, c / heads).
fn split_heads(x: &Tensor, off: usize, c: usize) -> Result<Tensor> {
    let (l, w) = x.dims2()?;
    let d = c / HEADS;
    run("split_heads", x, &[], [l, w, off, d].map(|v| v as u32).to_vec(), &[1, HEADS, l, d], (d, l, HEADS))
}

/// One transformer layer (self- or cross-attention) with its weights as transposed matrices
/// carrying the bias as a last row; layer scales are folded into `out` and `lin2`, the
/// attention scale into the query projection.
struct Layer {
    norm1: Tensor,
    norm_k: Option<Tensor>,
    in_q: Tensor,
    in_kv: Option<Tensor>,
    out: Tensor,
    norm_ff: Tensor,
    lin1: Tensor,
    lin2: Tensor,
    norm_out: Tensor,
}

/// (Co, Ci) weight and bias -> (Ci + 1, Co), rows of the weight scaled by `scale` (Co).
fn linear_t(w: &Tensor, b: &Tensor, scale: Option<&Tensor>) -> Result<Tensor> {
    let co = w.dim(0)?;
    let m = Tensor::cat(&[w.clone(), b.reshape((co, 1))?], 1)?;
    let m = match scale {
        Some(s) => m.broadcast_mul(&s.reshape((co, 1))?)?,
        None => m,
    };
    m.t()?.contiguous()
}

impl Layer {
    fn new(w: &Weights, name: &str, cross: bool) -> Result<Self> {
        let norm = |n: &str| Tensor::cat(&[get(w, &format!("{name}.{n}.weight"))?, get(w, &format!("{name}.{n}.bias"))?], 0);
        let attn = format!("{name}.{}", if cross { "cross_attn" } else { "self_attn" });
        let (iw, ib) = (get(w, &format!("{attn}.in_proj_weight"))?, get(w, &format!("{attn}.in_proj_bias"))?);
        let c = iw.dim(1)?;
        let dev = iw.device();
        let q_scale = Tensor::full((1.0 / ((c / HEADS) as f64).sqrt()) as f32, c, dev)?;
        let (in_q, in_kv) = if cross {
            (linear_t(&iw.narrow(0, 0, c)?, &ib.narrow(0, 0, c)?, Some(&q_scale))?, Some(linear_t(&iw.narrow(0, c, 2 * c)?, &ib.narrow(0, c, 2 * c)?, None)?))
        } else {
            let scale = Tensor::cat(&[q_scale, Tensor::ones(2 * c, DType::F32, dev)?], 0)?;
            (linear_t(&iw, &ib, Some(&scale))?, None)
        };
        Ok(Self {
            norm1: norm("norm1")?,
            norm_k: if cross { Some(norm("norm2")?) } else { None },
            in_q,
            in_kv,
            out: linear_t(&get(w, &format!("{attn}.out_proj.weight"))?, &get(w, &format!("{attn}.out_proj.bias"))?, Some(&get(w, &format!("{name}.gamma_1.scale"))?))?,
            norm_ff: norm(if cross { "norm3" } else { "norm2" })?,
            lin1: linear_t(&get(w, &format!("{name}.linear1.weight"))?, &get(w, &format!("{name}.linear1.bias"))?, None)?,
            lin2: linear_t(&get(w, &format!("{name}.linear2.weight"))?, &get(w, &format!("{name}.linear2.bias"))?, Some(&get(w, &format!("{name}.gamma_2.scale"))?))?,
            norm_out: norm("norm_out")?,
        })
    }

    /// q (L, C), k (S, C) for cross-attention -> (L, C [+1 ones]).
    fn forward(&self, q: &Tensor, k: Option<&Tensor>, ones: bool) -> Result<Tensor> {
        let (l, c) = q.dims2()?;
        let nq = ln_rows(q, &self.norm1, true)?;
        let (qh, kh, vh) = match (k, &self.norm_k, &self.in_kv) {
            (Some(k), Some(norm_k), Some(in_kv)) => {
                let qp = mm(&nq, &self.in_q)?;
                let kv = mm(&ln_rows(k, norm_k, true)?, in_kv)?;
                (split_heads(&qp, 0, c)?, split_heads(&kv, 0, c)?, split_heads(&kv, c, c)?)
            }
            _ => {
                let p = mm(&nq, &self.in_q)?;
                (split_heads(&p, 0, c)?, split_heads(&p, c, c)?, split_heads(&p, 2 * c, c)?)
            }
        };
        // Candle's fused attention kernel (scores, softmax and the weighted sum in one pass,
        // without storing the L × S score matrix); the 1/sqrt(d) scale is already in `in_q`.
        let attn = candle_nn::ops::sdpa(&qh, &kh, &vh, None, false, 1.0, 1.0)?;
        let d = c / HEADS;
        let merged = run("merge_heads", &attn, &[], [l, HEADS, d].map(|v| v as u32).to_vec(), &[l, c + 1], (c + 1, l, 1))?;
        let x = add(q, &mm(&merged, &self.out)?)?;
        let y = mm(&ln_rows(&x, &self.norm_ff, true)?, &self.lin1)?;
        let h = y.dim(1)?;
        let y = mm(&run("row_gelu", &y, &[], vec![h as u32], &[l, h + 1], (h + 1, l, 1))?, &self.lin2)?;
        let x = add(&x, &y)?;
        // norm_out: group norm with one group over all tokens and channels.
        let st = run("row_stats", &x, &[], [l, 1, c, c, 1].map(|v| v as u32).to_vec(), &[2, l], (1, l, 1))?.sum_keepdim(1)?.flatten_all()?;
        let params = vec![c as u32, ones as u32, ((l * c) as f32).to_bits(), 1e-5f32.to_bits()];
        run("row_norm", &x, &[&st, &self.norm_out], params, &[l, c + ones as usize], (c + ones as usize, l, 1))
    }
}

/// Prepared weights of one network.
pub struct Fast {
    enc: Vec<Encoder>,
    tenc: Vec<Encoder>,
    dec: Vec<Decoder>,
    tdec: Vec<Decoder>,
    /// Frequency embedding, already scaled: (48, 512).
    freq_emb: Tensor,
    /// Upsamplers to the transformer width as (385, 512); downsamplers as (384, 513).
    up: Tensor,
    up_t: Tensor,
    down: Tensor,
    down_t: Tensor,
    norm_in: Tensor,
    norm_in_t: Tensor,
    layers: Vec<Layer>,
    layers_t: Vec<Layer>,
    /// Positional embeddings for the two token sequences, made on first use.
    pos: std::sync::OnceLock<Tensor>,
    pos_t: std::sync::OnceLock<Tensor>,
}

type Weights = HashMap<String, Tensor>;

fn get(w: &Weights, name: &str) -> Result<Tensor> {
    w.get(name).cloned().ok_or_else(|| candle_core::Error::Msg(format!("missing weight {name}")))
}

/// Convolution weight (Co, C, k..) and bias as one matrix (Co, C*k + 1).
fn conv_matrix(w: &Weights, name: &str) -> Result<Tensor> {
    let weight = get(w, &format!("{name}.weight"))?;
    let co = weight.dim(0)?;
    Tensor::cat(&[weight.reshape((co, ()))?, get(w, &format!("{name}.bias"))?.reshape((co, 1))?], 1)?.contiguous()
}

fn dconv(w: &Weights, name: &str) -> Result<Dconv> {
    let mut layers = vec![];
    for i in 0..2 {
        let p = format!("{name}.layers.{i}");
        layers.push(DconvLayer {
            conv: conv_matrix(w, &format!("{p}.0"))?,
            norm1: Tensor::cat(&[get(w, &format!("{p}.1.weight"))?, get(w, &format!("{p}.1.bias"))?], 0)?,
            expand: conv_matrix(w, &format!("{p}.3"))?,
            norm2: Tensor::cat(&[get(w, &format!("{p}.4.weight"))?, get(w, &format!("{p}.4.bias"))?, get(w, &format!("{p}.6.scale"))?], 0)?,
            dilation: 1 << i,
        });
    }
    Ok(Dconv(layers))
}

impl Fast {
    pub fn new(w: &Weights) -> Result<Self> {
        let encoder = |name: String| -> Result<Encoder> {
            Ok(Encoder { conv: conv_matrix(w, &format!("{name}.conv"))?, dconv: dconv(w, &format!("{name}.dconv"))?, rewrite: conv_matrix(w, &format!("{name}.rewrite"))? })
        };
        let decoder = |name: String| -> Result<Decoder> {
            // (C, Co, 8[, 1]) -> (Co*8, C)
            let tr = get(w, &format!("{name}.conv_tr.weight"))?;
            let (c, co) = (tr.dim(0)?, tr.dim(1)?);
            let tr = tr.reshape((c, co * 8))?.t()?.contiguous()?;
            Ok(Decoder { rewrite: conv_matrix(w, &format!("{name}.rewrite"))?, dconv: dconv(w, &format!("{name}.dconv"))?, tr, tr_bias: get(w, &format!("{name}.conv_tr.bias"))? })
        };
        Ok(Self {
            enc: (0..4).map(|i| encoder(format!("encoder.{i}"))).collect::<Result<_>>()?,
            tenc: (0..4).map(|i| encoder(format!("tencoder.{i}"))).collect::<Result<_>>()?,
            dec: (0..4).map(|i| decoder(format!("decoder.{i}"))).collect::<Result<_>>()?,
            tdec: (0..4).map(|i| decoder(format!("tdecoder.{i}"))).collect::<Result<_>>()?,
            freq_emb: (get(w, "freq_emb.embedding.weight")?.t()? * 2.0)?.contiguous()?,
            up: conv_matrix(w, "channel_upsampler")?.t()?.contiguous()?,
            up_t: conv_matrix(w, "channel_upsampler_t")?.t()?.contiguous()?,
            down: conv_matrix(w, "channel_downsampler")?,
            down_t: conv_matrix(w, "channel_downsampler_t")?,
            norm_in: Tensor::cat(&[get(w, "crosstransformer.norm_in.weight")?, get(w, "crosstransformer.norm_in.bias")?], 0)?,
            norm_in_t: Tensor::cat(&[get(w, "crosstransformer.norm_in_t.weight")?, get(w, "crosstransformer.norm_in_t.bias")?], 0)?,
            layers: (0..5).map(|i| Layer::new(w, &format!("crosstransformer.layers.{i}"), i % 2 == 1)).collect::<Result<_>>()?,
            layers_t: (0..5).map(|i| Layer::new(w, &format!("crosstransformer.layers_t.{i}"), i % 2 == 1)).collect::<Result<_>>()?,
            pos: std::sync::OnceLock::new(),
            pos_t: std::sync::OnceLock::new(),
        })
    }

    /// Encoder layer over x (C, F, T); `freq` picks the axis of the stride-4 convolution.
    /// The result carries a channel of ones when `ones`.
    fn encode(&self, e: &Encoder, x: &Tensor, freq: bool, ones: bool) -> Result<Tensor> {
        let (_, f, t) = x.dims3()?;
        let (geo, out) = if freq { (Geo { k: (8, 1), s: (4, 1), p: (2, 0), d: (1, 1) }, (f / 4, t)) } else { (Geo { k: (1, 8), s: (1, 4), p: (0, 2), d: (1, 1) }, (1, t.div_ceil(4))) };
        let y = mm(&e.conv, &im2col(x, geo, out)?)?;
        let c = y.dim(0)?;
        let y = norm_act(&y.reshape((c, out.0, out.1))?, None, 1, None, false)?;
        let y = e.dconv.forward(&y, freq, true)?;
        let z = mm(&e.rewrite, &y.reshape((c + 1, out.0 * out.1))?)?;
        norm_act(&z.reshape((2 * c, out.0, out.1))?, None, 2, None, ones)
    }

    /// Decoder layer; `length` is the output size along the stride axis.
    fn decode(&self, d: &Decoder, x: &Tensor, skip: &Tensor, freq: bool, length: usize, last: bool) -> Result<Tensor> {
        let (c, f, t) = x.dims3()?;
        let y = add(x, skip)?;
        let geo = if freq { Geo { k: (3, 3), s: (1, 1), p: (1, 1), d: (1, 1) } } else { Geo { k: (1, 3), s: (1, 1), p: (0, 1), d: (1, 1) } };
        let z = mm(&d.rewrite, &im2col(&y, geo, (f, t))?)?;
        let z = norm_act(&z.reshape((2 * c, f, t))?, None, 2, None, false)?;
        let z = d.dconv.forward(&z, freq, false)?;
        let cols = mm(&d.tr, &z.reshape((c, f * t))?)?;
        let co = cols.dim(0)? / 8;
        let (fo, to) = if freq { (length, t) } else { (1, length) };
        let params = [co, f, t, if freq { 0 } else { 1 }, fo, to, if last { 0 } else { 1 }].map(|v| v as u32).to_vec();
        run("tr_col2im", &cols, &[&d.tr_bias], params, &[co, fo, to], (to, fo, co))
    }

    /// The whole network between the transforms: x is the normalised spectrogram
    /// (4, 2048, frames), xt the normalised mix (2, 1, L). Returns the spectrogram branch
    /// (16, 2048, frames) and the time branch (8, 1, L), both still normalised.
    pub fn forward(&self, x: &Tensor, xt: &Tensor) -> Result<(Tensor, Tensor)> {
        let (x, xt, saved, saved_t, lengths) = self.encoders(x, xt)?;
        let (f, t) = (saved[3].dim(1)?, saved[3].dim(2)?);
        let (x, xt) = self.transformer(&x, &xt, f, t)?;
        // Tokens (L, 513) back to channels first through the downsamplers.
        let x = mm(&self.down, &x.t()?)?.reshape((384, f, t))?;
        let xt = mm(&self.down_t, &xt.t()?)?.reshape((384, 1, ()))?;
        self.decoders(&x, &xt, saved, saved_t, lengths)
    }

    /// x (L, 512) tokens in (frequency, time) order, xt (T', 512). The results carry a last
    /// column of ones for the downsamplers' bias.
    fn transformer(&self, x: &Tensor, xt: &Tensor, f: usize, t: usize) -> Result<(Tensor, Tensor)> {
        let dev = x.device();
        let c = x.dim(1)?;
        let tt = xt.dim(0)?;
        let pos = match self.pos.get() {
            Some(p) if p.dim(0)? == f * t => p.clone(),
            _ => {
                let p = Tensor::from_vec(super::pos2(f, t, c), (t, f, c), dev)?.transpose(0, 1)?.contiguous()?.reshape((f * t, c))?;
                let _ = self.pos.set(p.clone());
                p
            }
        };
        let pos_t = match self.pos_t.get() {
            Some(p) if p.dim(0)? == tt => p.clone(),
            _ => {
                let p = Tensor::from_vec(super::pos1(tt, c), (tt, c), dev)?;
                let _ = self.pos_t.set(p.clone());
                p
            }
        };
        let mut x = add(&ln_rows(x, &self.norm_in, false)?, &pos)?;
        let mut xt = add(&ln_rows(xt, &self.norm_in_t, false)?, &pos_t)?;
        for i in 0..5 {
            let last = i == 4;
            if i % 2 == 0 {
                x = self.layers[i].forward(&x, None, last)?;
                xt = self.layers_t[i].forward(&xt, None, last)?;
            } else {
                let old = x;
                x = self.layers[i].forward(&old, Some(&xt), last)?;
                xt = self.layers_t[i].forward(&xt, Some(&old), last)?;
            }
        }
        Ok((x, xt))
    }

    /// Both encoders. Returns the transformer inputs as tokens (F*T, 512) and (T', 512), the
    /// skips and the time-branch lengths.
    #[allow(clippy::type_complexity)]
    fn encoders(&self, x: &Tensor, xt: &Tensor) -> Result<(Tensor, Tensor, Vec<Tensor>, Vec<Tensor>, Vec<usize>)> {
        let (mut x, mut xt) = (x.clone(), xt.clone());
        let (mut saved, mut saved_t, mut lengths) = (vec![], vec![], vec![]);
        for i in 0..4 {
            lengths.push(xt.dim(2)?);
            xt = self.encode(&self.tenc[i], &xt, false, i == 3)?;
            x = self.encode(&self.enc[i], &x, true, i == 3)?;
            if i == 0 {
                let (c, f, t) = x.dims3()?;
                x = run("add_cf", &x, &[&self.freq_emb], [c, f, t].map(|v| v as u32).to_vec(), &[c, f, t], (t, f, c))?;
            }
            // The deepest layer carries a channel of ones for the upsampler's bias.
            let c = x.dim(0)? - (i == 3) as usize;
            saved.push(x.narrow(0, 0, c)?);
            saved_t.push(xt.narrow(0, 0, c)?);
        }
        let (c1, f, t) = x.dims3()?;
        let x = mm(&x.reshape((c1, f * t))?.t()?, &self.up)?;
        let xt = mm(&xt.reshape((c1, ()))?.t()?, &self.up_t)?;
        Ok((x, xt, saved, saved_t, lengths))
    }

    /// Both decoders from the downsampled transformer outputs x (384, 8, 336), xt (384, 1, T').
    /// Returns the spectrogram branch (16, 2048, 336) and the time branch (8, 1, L).
    fn decoders(&self, x: &Tensor, xt: &Tensor, mut saved: Vec<Tensor>, mut saved_t: Vec<Tensor>, mut lengths: Vec<usize>) -> Result<(Tensor, Tensor)> {
        let (mut x, mut xt) = (x.clone(), xt.clone());
        for i in 0..4 {
            let f = x.dim(1)?;
            x = self.decode(&self.dec[i], &x, &saved.pop().unwrap(), true, f * 4, i == 3)?;
            xt = self.decode(&self.tdec[i], &xt, &saved_t.pop().unwrap(), false, lengths.pop().unwrap(), i == 3)?;
        }
        Ok((x, xt))
    }
}
