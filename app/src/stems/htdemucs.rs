//! HTDemucs (open model from facebookresearch/demucs, MIT licence) in pure Rust with Candle.
//!
//! The graph is the fixed profile validated in rekordbox-rea (analysis/open-model-harness,
//! reports/htdemucs-candle-graph.md and htdemucs-whole-track-contract.md): batch 1, stereo f32
//! at 44.1 kHz, sources drums/bass/other/vocals, 343,980-frame segments with 25 % overlap and
//! a triangular transition (power 1), no shifts. Research tracing/comparison code is removed.
//!
//! `Graph::forward` is the reference used by the CPU. The experimental Linux Vulkan
//! backend runs its transformer projections on the GPU and the remaining operations on the CPU. On
//! Metal, `separate` uses the fused kernels in `metal_fast.rs` instead (about 7× faster on
//! an M5 Pro; outputs agree with the reference to better than 100 dB, which the test
//! `metal_kernels_match_reference_graph` guards) and overlaps the transforms on the CPU
//! with the network on the GPU.

#[cfg(target_os = "macos")]
#[path = "metal_fast.rs"]
mod metal_fast;

use candle_core::{DType, Device, Module, Result, Tensor};
#[cfg(any(target_os = "macos", feature = "directml"))]
use anyhow::Context as _;
use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::HashMap;
use std::path::Path;

pub const TRAINING_FRAMES: usize = 343980;
pub const SAMPLE_RATE: u32 = 44100;
/// Output order of every HTDemucs model.
pub const SOURCES: [&str; 4] = ["drums", "bass", "other", "vocals"];
const NFFT: usize = 4096;
const HOP: usize = 1024;

fn hann(n: usize) -> Vec<f32> {
    let step = (2. * std::f64::consts::PI / n as f64) as f32;
    (0..n).map(|i| 0.5 - 0.5 * (i as f32 * step).cos()).collect()
}

fn reflect(x: &[f32], left: usize, right: usize) -> Vec<f32> {
    (0..x.len() + left + right)
        .map(|i| {
            let p = i as isize - left as isize;
            let n = x.len() as isize;
            let j = if p < 0 { -p } else if p >= n { 2 * n - 2 - p } else { p };
            x[j.clamp(0, n - 1) as usize]
        })
        .collect()
}

/// The model's short-time Fourier transform (4096 window, hop 1024, the upstream padding)
/// for one channel at a time, on the CPU.
pub(crate) struct Stft {
    win: Vec<f32>,
    fwd: std::sync::Arc<dyn rustfft::Fft<f32>>,
    inv: std::sync::Arc<dyn rustfft::Fft<f32>>,
}

impl Stft {
    pub(crate) fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        Self { win: hann(NFFT), fwd: planner.plan_fft_forward(NFFT), inv: planner.plan_fft_inverse(NFFT) }
    }

    pub(crate) fn frames(len: usize) -> usize {
        len.div_ceil(HOP)
    }

    /// x -> real and imaginary planes, each [bin][frame] with NFFT/2 bins.
    pub(crate) fn forward(&self, x: &[f32], re: &mut [f32], im: &mut [f32]) {
        let len = x.len();
        let frames = Self::frames(len);
        let freq = NFFT / 2;
        let pad = HOP / 2 * 3;
        let norm = 1. / (NFFT as f32).sqrt();
        let padded = reflect(x, pad, pad + frames * HOP - len);
        let padded = reflect(&padded, NFFT / 2, NFFT / 2);
        let mut buffer = vec![Complex::new(0f32, 0.); NFFT];
        let mut scratch = vec![Complex::new(0f32, 0.); self.fwd.get_inplace_scratch_len()];
        for frame in 0..frames {
            let start = (frame + 2) * HOP;
            for (i, b) in buffer.iter_mut().enumerate() {
                *b = Complex::new(padded[start + i] * self.win[i], 0.);
            }
            self.fwd.process_with_scratch(&mut buffer, &mut scratch);
            for f in 0..freq {
                re[f * frames + frame] = buffer[f].re * norm;
                im[f * frames + frame] = buffer[f].im * norm;
            }
        }
    }

    /// Sum of squared windows at every output position (the inverse divides by it).
    pub(crate) fn envelope(&self, frames: usize) -> Vec<f32> {
        let mut envelope = vec![0.; NFFT + (frames + 3) * HOP];
        for frame in 0..frames + 4 {
            for i in 0..NFFT {
                envelope[frame * HOP + i] += self.win[i] * self.win[i];
            }
        }
        envelope
    }

    /// Planes [bin][frame] (each value taken as v * scale + shift) -> out.len() samples.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn inverse(&self, re: &[f32], im: &[f32], frames: usize, scale: f32, shift: f32, envelope: &[f32], out: &mut [f32]) {
        let freq = NFFT / 2;
        let norm = 1. / (NFFT as f32).sqrt();
        let pad = HOP / 2 * 3;
        let mut wave = vec![0f32; NFFT + (frames + 3) * HOP];
        let mut buffer = vec![Complex::new(0f32, 0.); NFFT];
        let mut scratch = vec![Complex::new(0f32, 0.); self.inv.get_inplace_scratch_len()];
        for frame in 0..frames {
            buffer[freq] = Complex::new(0., 0.);
            for f in 0..freq {
                let v = Complex::new(re[f * frames + frame] * scale + shift, im[f * frames + frame] * scale + shift);
                buffer[f] = v;
                if f > 0 {
                    buffer[NFFT - f] = v.conj();
                }
            }
            self.inv.process_with_scratch(&mut buffer, &mut scratch);
            let w = &mut wave[(frame + 2) * HOP..(frame + 2) * HOP + NFFT];
            for i in 0..NFFT {
                w[i] += buffer[i].re * norm * self.win[i];
            }
        }
        let start = NFFT / 2 + pad;
        for (i, o) in out.iter_mut().enumerate() {
            *o = wave[start + i] / envelope[start + i];
        }
    }
}

fn spec(x: &Tensor) -> Result<Tensor> {
    let (b, c, len) = x.dims3()?;
    let frames = Stft::frames(len);
    let freq = NFFT / 2;
    let input = x.flatten_all()?.to_vec1::<f32>()?;
    let stft = Stft::new();
    let mut out = vec![0.; b * c * freq * frames * 2];
    let (mut re, mut im) = (vec![0f32; freq * frames], vec![0f32; freq * frames]);
    for bc in 0..b * c {
        stft.forward(&input[bc * len..(bc + 1) * len], &mut re, &mut im);
        for i in 0..freq * frames {
            out[(bc * freq * frames + i) * 2] = re[i];
            out[(bc * freq * frames + i) * 2 + 1] = im[i];
        }
    }
    Tensor::from_vec(out, (b, c, freq, frames, 2), x.device())
}

fn ispec(z: &Tensor, len: usize) -> Result<Tensor> {
    let (b, c, freq, frames, ri) = z.dims5()?;
    if freq != NFFT / 2 || ri != 2 {
        candle_core::bail!("wrong spectral shape")
    }
    let input = z.flatten_all()?.to_vec1::<f32>()?;
    let stft = Stft::new();
    let envelope = stft.envelope(frames);
    let mut out = vec![0f32; b * c * len];
    let (mut re, mut im) = (vec![0f32; freq * frames], vec![0f32; freq * frames]);
    for bc in 0..b * c {
        for i in 0..freq * frames {
            re[i] = input[(bc * freq * frames + i) * 2];
            im[i] = input[(bc * freq * frames + i) * 2 + 1];
        }
        stft.inverse(&re, &im, frames, 1., 0., &envelope, &mut out[bc * len..(bc + 1) * len]);
    }
    Tensor::from_vec(out, (b, c, len), z.device())
}

/// Sinusoidal embedding of the time-branch tokens: [t][c].
pub(crate) fn pos1(len: usize, c: usize) -> Vec<f32> {
    let half = c / 2;
    let mut out = vec![0.; len * c];
    for t in 0..len {
        for i in 0..half {
            let denom = 10000f32.powf(i as f32 / (half - 1) as f32);
            let phase = t as f32 / denom;
            out[t * c + i] = phase.cos();
            out[t * c + half + i] = phase.sin();
        }
    }
    out
}

/// 2-D sinusoidal embedding of the spectrogram-branch tokens: [time][freq][c].
pub(crate) fn pos2(f: usize, t: usize, c: usize) -> Vec<f32> {
    let half = c / 2;
    let mut out = vec![0.; f * t * c];
    let scale = -(10000f64.ln() / half as f64) as f32;
    for time in 0..t {
        for freq in 0..f {
            for i in 0..half / 2 {
                let div = ((2 * i) as f32 * scale).exp();
                let pt = time as f32 * div;
                let pf = freq as f32 * div;
                let offset = (time * f + freq) * c + 2 * i;
                out[offset] = pt.sin();
                out[offset + 1] = pt.cos();
                out[offset + half] = pf.sin();
                out[offset + half + 1] = pf.cos();
            }
        }
    }
    out
}

/// Candle's PthTensors::get reopens the ZIP directory for each weight. Reuse one
/// archive reader for the complete checkpoint, transferring each tensor immediately.
fn read_checkpoint(path: &Path, device: &Device) -> Result<HashMap<String, Tensor>> {
    use std::io::Read;
    let pth = candle_core::pickle::PthTensors::new(path, Some("state"))?;
    let infos = pth.tensor_infos();
    if infos.len() != 533 {
        candle_core::bail!("expected an HTDemucs checkpoint with 533 tensors, found {}", infos.len())
    }
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(path)?))
        .map_err(|e| candle_core::Error::Msg(e.to_string()))?;
    let mut weights = HashMap::with_capacity(infos.len());
    for (name, info) in infos {
        let mut entry = archive.by_name(&info.path).map_err(|e| candle_core::Error::Msg(e.to_string()))?;
        let layout = &info.layout;
        let fortran = layout.is_fortran_contiguous();
        if !layout.is_contiguous() && !fortran {
            candle_core::bail!("unsupported layout in tensor {name}")
        }
        let bytes = layout.dims().iter().copied().try_fold(info.dtype.size_in_bytes(), usize::checked_mul)
            .ok_or_else(|| candle_core::Error::Msg(format!("invalid size for tensor {name}")))?;
        // Candle's pickle reader records the start offset in bytes.
        let skip = layout.start_offset() as u64;
        if skip.checked_add(bytes as u64).is_none_or(|end| end > entry.size()) {
            candle_core::bail!("tensor {name} exceeds its checkpoint entry")
        }
        if skip != 0 {
            std::io::copy(&mut entry.by_ref().take(skip), &mut std::io::sink())?;
        }
        let mut buf = vec![0; bytes];
        entry.read_exact(&mut buf)?;
        let mut tensor = Tensor::from_raw_buffer(&buf, info.dtype, layout.dims(), device)?;
        let rank = layout.dims().len();
        if fortran && rank > 1 {
            tensor = tensor.reshape(layout.dims().iter().rev().copied().collect::<Vec<_>>())?
                .permute((0..rank).rev().collect::<Vec<_>>())?;
        }
        weights.insert(name.clone(), tensor);
    }
    Ok(weights)
}

/// One loaded HTDemucs network.
pub struct Graph {
    #[cfg(all(target_os = "linux", feature = "vulkan"))]
    vulkan: Option<&'static super::vulkan::Engine>,
    #[cfg(feature = "directml")]
    directml: Option<super::directml::Engine>,
    /// The checkpoint's tensors by name: the reference graph (any device).
    weights: HashMap<String, Tensor>,
    /// Weights prepared for the fused Metal kernels; when present, `separate` uses them and
    /// `weights` is empty.
    #[cfg(target_os = "macos")]
    fast: Option<metal_fast::Fast>,
}

impl Graph {
    /// Load weights from a converted `.safetensors` file or an original Demucs `.th` checkpoint.
    /// On a Metal device the fused kernels are used; elsewhere the reference graph.
    pub fn load(path: &Path, device: &Device) -> Result<Self> {
        Self::load_as(path, device, device.is_metal())
    }

    /// As `load`, but always the reference graph built from Candle's own operations (for
    /// comparing the two on a real checkpoint).
    #[allow(dead_code)]
    pub fn load_reference(path: &Path, device: &Device) -> Result<Self> {
        Self::load_as(path, device, false)
    }

    fn load_as(path: &Path, device: &Device, fast: bool) -> Result<Self> {
        let _gpu = GPU.lock();
        Self::from_weights(Self::read_weights(path, device)?, fast)
    }

    pub fn load_selected(path: &Path) -> anyhow::Result<Self> {
        #[cfg(all(target_os = "linux", feature = "vulkan"))]
        {
            let engine = super::backend::vulkan_device();
            if let Some(reason) = super::backend::vulkan_hold() {
                return Err(super::VulkanFailure(reason.to_owned()).into());
            }
            if let Some(engine) = engine {
                let mut graph = Self::load(path, &Device::Cpu)?;
                graph.vulkan = Some(engine);
                return Ok(graph);
            }
        }
        #[cfg(all(windows, feature = "directml"))]
        {
            let adapter = super::backend::directml_device();
            if let Some(reason) = super::radeon_hold() {
                return Err(super::DirectMlFailure(reason.to_owned()).into());
            }
            if let Some(adapter) = adapter { return Self::load_onnx(path, Some(adapter)); }
        }
        Ok(Self::load(path, &super::device())?)
    }

    #[cfg(feature = "directml")]
    pub(super) fn load_onnx(path: &Path, adapter: Option<&super::directml::Binding>) -> anyhow::Result<Self> {
        let _gpu = GPU.lock();
        let weights = Self::read_weights(path, &Device::Cpu)?;
        let engine = super::directml::Engine::new(weights, adapter)?;
        let mut graph = Self::from_weights(HashMap::new(), false)?;
        graph.directml = Some(engine);
        Ok(graph)
    }

    pub(super) fn read_weights(path: &Path, device: &Device) -> Result<HashMap<String, Tensor>> {
        let raw: HashMap<String, Tensor> = match path.extension().and_then(|e| e.to_str()) {
            Some("safetensors") => candle_core::safetensors::load(path, device)?,
            _ => read_checkpoint(path, device)?,
        };
        let weights = raw.into_iter().map(|(k, v)| Ok((k, v.to_dtype(DType::F32)?))).collect::<Result<HashMap<_, _>>>()?;
        if weights.len() != 533 {
            candle_core::bail!("expected an HTDemucs checkpoint with 533 tensors, found {}", weights.len())
        }
        Ok(weights)
    }

    fn from_weights(weights: HashMap<String, Tensor>, fast: bool) -> Result<Self> {
        #[cfg(target_os = "macos")]
        if fast {
            return Ok(Self { #[cfg(feature = "directml")] directml: None, fast: Some(metal_fast::Fast::new(&weights)?), weights: HashMap::new() });
        }
        let _ = fast;
        Ok(Self {
            #[cfg(all(target_os = "linux", feature = "vulkan"))]
            vulkan: None,
            #[cfg(feature = "directml")]
            directml: None,
            weights,
            #[cfg(target_os = "macos")]
            fast: None,
        })
    }

    fn w(&self, name: &str) -> Result<Tensor> {
        self.weights.get(name).cloned().ok_or_else(|| candle_core::Error::Msg(format!("missing weight {name}")))
    }
    fn conv1(&self, x: &Tensor, name: &str, pad: usize, stride: usize, dilation: usize) -> Result<Tensor> {
        let w = self.w(&format!("{name}.weight"))?;
        let co = w.dim(0)?;
        x.conv1d(&w, pad, stride, dilation, 1)?.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1))?)
    }
    fn conv2(&self, x: &Tensor, name: &str, pad: usize) -> Result<Tensor> {
        let w = self.w(&format!("{name}.weight"))?;
        let co = w.dim(0)?;
        x.conv2d(&w, pad, 1, 1, 1)?.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1, 1))?)
    }
    fn freq_conv(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        let (b, c, f, t) = x.dims4()?;
        let w = self.w(&format!("{name}.weight"))?.squeeze(3)?;
        let co = w.dim(0)?;
        let y = x.permute((0, 3, 1, 2))?.contiguous()?.reshape((b * t, c, f))?.conv1d(&w, 2, 4, 1, 1)?;
        let fo = y.dim(2)?;
        y.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1))?)?.reshape((b, t, co, fo))?.permute((0, 2, 3, 1))?.contiguous()
    }
    fn transpose1(&self, x: &Tensor, w: &Tensor) -> Result<Tensor> {
        if matches!(x.device(), Device::Cpu) {
            x.unsqueeze(3)?.conv_transpose2d(&w.unsqueeze(3)?, 0, 0, 4, 1)?.squeeze(3)
        } else {
            x.conv_transpose1d(w, 0, 0, 4, 1, 1)
        }
    }
    fn transposed(&self, x: &Tensor, name: &str, freq: bool, length: usize) -> Result<Tensor> {
        let mut w = self.w(&format!("{name}.weight"))?;
        let co = w.dim(1)?;
        if freq {
            let (b, c, f, t) = x.dims4()?;
            w = w.squeeze(3)?;
            let y = self.transpose1(&x.permute((0, 3, 1, 2))?.contiguous()?.reshape((b * t, c, f))?, &w)?;
            let fo = y.dim(2)?;
            y.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1))?)?.narrow(2, 2, fo - 4)?.reshape((b, t, co, fo - 4))?.permute((0, 2, 3, 1))?.contiguous()
        } else {
            self.transpose1(x, &w)?.broadcast_add(&self.w(&format!("{name}.bias"))?.reshape((1, co, 1))?)?.narrow(2, 2, length)?.contiguous()
        }
    }
    fn gn(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        // CPU normalises in f64 (large near-constant groups amplify f32 reduction error).
        let internal = if matches!(x.device(), Device::Cpu) { DType::F64 } else { DType::F32 };
        candle_nn::GroupNorm::new(self.w(&format!("{name}.weight"))?.to_dtype(internal)?, self.w(&format!("{name}.bias"))?.to_dtype(internal)?, x.dim(1)?, 1, 1e-5)?
            .forward(&x.to_dtype(internal)?)?
            .to_dtype(DType::F32)
    }
    fn ln(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        candle_nn::LayerNorm::new(self.w(&format!("{name}.weight"))?, self.w(&format!("{name}.bias"))?, 1e-5).forward(x)
    }
    fn glu(&self, x: &Tensor) -> Result<Tensor> {
        let half = x.dim(1)? / 2;
        x.narrow(1, 0, half)?.mul(&candle_nn::ops::sigmoid(&x.narrow(1, half, half)?)?)
    }
    fn dconv(&self, x: &Tensor, name: &str) -> Result<Tensor> {
        let mut x = x.clone();
        for i in 0..2 {
            let p = format!("{name}.layers.{i}");
            let dilation = 1 << i;
            let y = self.conv1(&x, &format!("{p}.0"), dilation, 1, dilation)?;
            let y = self.gn(&y, &format!("{p}.1"))?.gelu_erf()?;
            let y = self.conv1(&y, &format!("{p}.3"), 0, 1, 1)?;
            let y = self.glu(&self.gn(&y, &format!("{p}.4"))?)?;
            let scale = self.w(&format!("{p}.6.scale"))?.reshape((1, x.dim(1)?, 1))?;
            x = (x + y.broadcast_mul(&scale)?)?;
        }
        Ok(x)
    }
    fn spatial_dconv(&self, x: &Tensor, name: &str, freq: bool) -> Result<Tensor> {
        if !freq {
            return self.dconv(x, name);
        }
        let (b, c, f, t) = x.dims4()?;
        self.dconv(&x.permute((0, 2, 1, 3))?.contiguous()?.reshape((b * f, c, t))?, name)?.reshape((b, f, c, t))?.permute((0, 2, 1, 3))?.contiguous()
    }
    fn encode(&self, x: &Tensor, name: &str, freq: bool) -> Result<Tensor> {
        let y = if freq {
            self.freq_conv(x, &format!("{name}.conv"))?
        } else {
            let len = x.dim(2)?;
            let x = x.pad_with_zeros(2, 0, (4 - len % 4) % 4)?;
            self.conv1(&x, &format!("{name}.conv"), 2, 4, 1)?
        }
        .gelu_erf()?;
        let y = self.spatial_dconv(&y, &format!("{name}.dconv"), freq)?;
        let y = if freq { self.conv2(&y, &format!("{name}.rewrite"), 0)? } else { self.conv1(&y, &format!("{name}.rewrite"), 0, 1, 1)? };
        self.glu(&y)
    }
    fn decode(&self, x: &Tensor, skip: &Tensor, name: &str, freq: bool, length: usize, last: bool) -> Result<Tensor> {
        let y = (x + skip)?;
        let y = if freq { self.conv2(&y, &format!("{name}.rewrite"), 1)? } else { self.conv1(&y, &format!("{name}.rewrite"), 1, 1, 1)? };
        let pre = self.spatial_dconv(&self.glu(&y)?, &format!("{name}.dconv"), freq)?;
        let z = self.transposed(&pre, &format!("{name}.conv_tr"), freq, length)?;
        if last {
            Ok(z)
        } else {
            z.gelu_erf()
        }
    }
    fn linear_weight(&self, x: &Tensor, w: &Tensor, bias: &Tensor, checkpoint: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<Tensor> {
        checkpoint()?;
        let (b, t, c) = x.dims3()?;
        let (co, inner) = w.dims2()?;
        anyhow::ensure!(inner == c, "Transformer projection expects {inner} input channels, found {c}");
        #[cfg(all(target_os = "linux", feature = "vulkan"))]
        if let Some(engine) = self.vulkan {
            let product = vulkan_linear(engine, x, w, b * t, c, co, checkpoint)?;
            return Ok(Tensor::from_vec(product, (b, t, co), &Device::Cpu)?.broadcast_add(bias)?);
        }
        Ok(x.reshape((b * t, c))?.matmul(&w.t()?.contiguous()?)?.broadcast_add(bias)?.reshape((b, t, co))?)
    }
    fn linear(&self, x: &Tensor, name: &str, checkpoint: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<Tensor> {
        self.linear_weight(x, &self.w(&format!("{name}.weight"))?, &self.w(&format!("{name}.bias"))?, checkpoint)
    }
    fn attention(&self, q: &Tensor, k: &Tensor, name: &str, checkpoint: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<Tensor> {
        let (b, l, c) = q.dims3()?;
        let s = k.dim(1)?;
        let h = 8;
        let head = c / h;
        let w = self.w(&format!("{name}.in_proj_weight"))?;
        let bias = self.w(&format!("{name}.in_proj_bias"))?;
        let q = self.linear_weight(q, &w.narrow(0, 0, c)?.contiguous()?, &bias.narrow(0, 0, c)?, checkpoint)?.reshape((b, l, h, head))?.permute((0, 2, 1, 3))?.contiguous()?;
        let kp = self.linear_weight(k, &w.narrow(0, c, c)?.contiguous()?, &bias.narrow(0, c, c)?, checkpoint)?.reshape((b, s, h, head))?.permute((0, 2, 1, 3))?.contiguous()?;
        let vp = self.linear_weight(k, &w.narrow(0, 2 * c, c)?.contiguous()?, &bias.narrow(0, 2 * c, c)?, checkpoint)?.reshape((b, s, h, head))?.permute((0, 2, 1, 3))?.contiguous()?;
        // Limit temporary attention scores on CUDA and in the host portion of Vulkan inference.
        // Each query still attends to every key, including across tile boundaries.
        let rows = if q.device().is_cuda() { 256 } else { l };
        #[cfg(all(target_os = "linux", feature = "vulkan"))]
        let rows = if self.vulkan.is_some() { 256 } else { rows };
        let attended = attention_rows(&q, &kp.transpose(2, 3)?.contiguous()?, &vp, rows, checkpoint)?;
        let y = attended.permute((0, 2, 1, 3))?.contiguous()?.reshape((b, l, c))?;
        self.linear(&y, &format!("{name}.out_proj"), checkpoint)
    }
    fn transformer_layer(&self, q: &Tensor, k: Option<&Tensor>, name: &str, checkpoint: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<Tensor> {
        let nq = self.ln(q, &format!("{name}.norm1"))?;
        let attn = if let Some(k) = k { self.attention(&nq, &self.ln(k, &format!("{name}.norm2"))?, &format!("{name}.cross_attn"), checkpoint)? } else { self.attention(&nq, &nq, &format!("{name}.self_attn"), checkpoint)? };
        let x = (q + attn.broadcast_mul(&self.w(&format!("{name}.gamma_1.scale"))?)?)?;
        let norm_name = if k.is_some() { "norm3" } else { "norm2" };
        let y = self.ln(&x, &format!("{name}.{norm_name}"))?;
        let y = self.linear(&y, &format!("{name}.linear1"), checkpoint)?.gelu_erf()?;
        let y = self.linear(&y, &format!("{name}.linear2"), checkpoint)?;
        let x = (x + y.broadcast_mul(&self.w(&format!("{name}.gamma_2.scale"))?)?)?;
        Ok(self.gn(&x.transpose(1, 2)?.contiguous()?, &format!("{name}.norm_out"))?.transpose(1, 2)?.contiguous()?)
    }
    fn transformer(&self, x: &Tensor, xt: &Tensor, checkpoint: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<(Tensor, Tensor)> {
        let (b, c, f, t) = x.dims4()?;
        let tt = xt.dim(2)?;
        let x = x.permute((0, 3, 2, 1))?.contiguous()?.reshape((b, t * f, c))?;
        let mut x = self.ln(&x, "crosstransformer.norm_in")?.broadcast_add(&Tensor::from_vec(pos2(f, t, c), (1, t * f, c), x.device())?)?;
        let mut xt = self.ln(&xt.transpose(1, 2)?.contiguous()?, "crosstransformer.norm_in_t")?.broadcast_add(&Tensor::from_vec(pos1(tt, c), (1, tt, c), xt.device())?)?;
        for i in 0..5 {
            checkpoint()?;
            let prefix = format!("crosstransformer.layers.{i}");
            let tp = format!("crosstransformer.layers_t.{i}");
            if i % 2 == 0 {
                x = self.transformer_layer(&x, None, &prefix, checkpoint)?;
                xt = self.transformer_layer(&xt, None, &tp, checkpoint)?;
            } else {
                let old_x = x;
                x = self.transformer_layer(&old_x, Some(&xt), &prefix, checkpoint)?;
                xt = self.transformer_layer(&xt, Some(&old_x), &tp, checkpoint)?;
            }
        }
        Ok((x.reshape((b, t, f, c))?.permute((0, 3, 2, 1))?.contiguous()?, xt.transpose(1, 2)?.contiguous()?))
    }
    fn normalize(&self, x: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let internal = if matches!(x.device(), Device::Cpu) { DType::F64 } else { DType::F32 };
        let flat = x.flatten_from(1)?.to_dtype(internal)?;
        let mean = flat.mean_keepdim(1)?.to_dtype(DType::F32)?;
        let std = flat.var_keepdim(1)?.sqrt()?.to_dtype(DType::F32)?;
        let mut dims = vec![1; x.rank()];
        dims[0] = x.dim(0)?;
        let mean = mean.reshape(dims.clone())?;
        let std = std.reshape(dims)?;
        let y = x.broadcast_sub(&mean)?.broadcast_div(&(&std + 1e-5)?)?;
        Ok((y, mean, std))
    }

    /// One model pass: [1, 2, L] (L <= 343980) -> [1, 4, 2, L].
    fn forward(&self, input: &Tensor, checkpoint: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<Tensor> {
        checkpoint()?;
        let (b, c, original_len) = input.dims3()?;
        if b != 1 || c != 2 || original_len == 0 || original_len > TRAINING_FRAMES {
            anyhow::bail!("HTDemucs input must be [1, 2, L] with 1 <= L <= {TRAINING_FRAMES}")
        }
        let mix = input.pad_with_zeros(2, 0, TRAINING_FRAMES - original_len)?;
        let spectral = spec(&mix)?;
        let frames = spectral.dim(3)?;
        let freq = spectral.dim(2)?;
        let magnitude = spectral.permute((0, 1, 4, 2, 3))?.contiguous()?.reshape((b, c * 2, freq, frames))?;
        let (mut x, mean, std) = self.normalize(&magnitude)?;
        let (mut xt, meant, stdt) = self.normalize(&mix)?;
        let mut saved = vec![];
        let mut saved_t = vec![];
        let mut lengths_t = vec![];
        for i in 0..4 {
            checkpoint()?;
            lengths_t.push(xt.dim(2)?);
            xt = self.encode(&xt, &format!("tencoder.{i}"), false)?;
            x = self.encode(&x, &format!("encoder.{i}"), true)?;
            if i == 0 {
                let embedding = (self.w("freq_emb.embedding.weight")?.t()?.unsqueeze(0)?.unsqueeze(3)? * 10.)?;
                x = x.broadcast_add(&(embedding * 0.2)?)?;
            }
            saved.push(x.clone());
            saved_t.push(xt.clone());
        }
        let (_, co, f, t) = x.dims4()?;
        x = self.conv1(&x.reshape((b, co, f * t))?, "channel_upsampler", 0, 1, 1)?.reshape((b, 512, f, t))?;
        xt = self.conv1(&xt, "channel_upsampler_t", 0, 1, 1)?;
        (x, xt) = self.transformer(&x, &xt, checkpoint)?;
        x = self.conv1(&x.reshape((b, 512, f * t))?, "channel_downsampler", 0, 1, 1)?.reshape((b, co, f, t))?;
        xt = self.conv1(&xt, "channel_downsampler_t", 0, 1, 1)?;
        for i in 0..4 {
            checkpoint()?;
            x = self.decode(&x, &saved.pop().unwrap(), &format!("decoder.{i}"), true, t, i == 3)?;
            xt = self.decode(&xt, &saved_t.pop().unwrap(), &format!("tdecoder.{i}"), false, lengths_t.pop().unwrap(), i == 3)?;
        }
        let x = x.reshape((b, 4, 4, freq, frames))?.broadcast_mul(&std.unsqueeze(1)?)?.broadcast_add(&mean.unsqueeze(1)?)?;
        let complex = x.reshape((b * 4, 2, 2, freq, frames))?.permute((0, 1, 3, 4, 2))?.contiguous()?;
        let spectral_wave = ispec(&complex, TRAINING_FRAMES)?.reshape((b, 4, 2, TRAINING_FRAMES))?;
        let temporal_wave = xt.reshape((b, 4, 2, TRAINING_FRAMES))?.broadcast_mul(&stdt.unsqueeze(1)?)?.broadcast_add(&meant.unsqueeze(1)?)?;
        Ok((spectral_wave + temporal_wave)?.narrow(3, 0, original_len)?.contiguous()?)
    }
}

#[cfg(all(target_os = "linux", feature = "vulkan"))]
fn vulkan_linear(engine: &super::vulkan::Engine, x: &Tensor, weights: &Tensor, rows: usize, inner: usize,
    cols: usize, checkpoint: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<Vec<f32>> {
    if let Some(reason) = super::backend::vulkan_hold() {
        return Err(super::VulkanFailure(reason.to_owned()).into());
    }
    // Q, K and V are contiguous views into different regions of the same checkpoint tensor.
    // Borrow only the view's elements, including its starting offset.
    fn elements<'a>(storage: &'a candle_core::Storage, layout: &candle_core::Layout) -> anyhow::Result<&'a [f32]> {
        let candle_core::Storage::Cpu(cpu) = storage else { anyhow::bail!("Vulkan projections require host tensors") };
        let (start, end) = layout.contiguous_offsets().ok_or_else(|| anyhow::anyhow!("Non-contiguous Vulkan input"))?;
        cpu.as_slice::<f32>()?.get(start..end).ok_or_else(|| anyhow::anyhow!("Vulkan tensor view exceeds its storage"))
    }
    let (x, weights) = (x.contiguous()?, weights.contiguous()?);
    let (a, a_layout) = x.storage_and_layout();
    let (b, b_layout) = weights.storage_and_layout();
    let mut stopped = false;
    engine.linear(elements(&a, a_layout)?, elements(&b, b_layout)?, rows, inner, cols, &mut || {
        let result = checkpoint();
        stopped = result.is_err();
        result
    }).map_err(|err| {
        if stopped { err } else {
            let message = format!("Vulkan transformer projection on {}: {err:#}", engine.label());
            err.context(super::VulkanFailure(message))
        }
    })
}

#[cfg(all(test, target_os = "linux", feature = "vulkan"))]
#[test]
#[ignore = "requires a software Vulkan ICD"]
fn vulkan_transformer_projections_match_cpu() -> anyhow::Result<()> {
    static ENGINE: std::sync::OnceLock<super::vulkan::Engine> = std::sync::OnceLock::new();
    let engine = ENGINE.get_or_init(|| super::vulkan::Engine::new_for_test().expect("software Vulkan ICD"));
    let (b, t, c, co) = (2, 19, 23, 17);
    let samples = |n| (0..n).map(|i| ((i as f32 + 3.) * 0.1).sin()).collect();
    let x = Tensor::from_vec(samples(b * c * t), (b, c, t), &Device::Cpu)?.transpose(1, 2)?;
    // Model Q/K/V rows and their biases occupy different offsets within the fused weights.
    let w = Tensor::from_vec(samples(3 * co * c), (3 * co, c), &Device::Cpu)?.narrow(0, 2 * co, co)?;
    let bias = Tensor::from_vec(samples(3 * co), 3 * co, &Device::Cpu)?.narrow(0, co, co)?;
    let mut graph = Graph::from_weights(HashMap::new(), false)?;
    let project = |graph: &Graph| -> anyhow::Result<Vec<f32>> {
        let y = graph.linear_weight(&x, &w, &bias, &mut || Ok(()))?;
        assert_eq!(y.dims(), [b, t, co]);
        Ok(y.flatten_all()?.to_vec1::<f32>()?)
    };
    let before = engine.completed_matmuls();
    let cpu = project(&graph)?;
    assert_eq!(engine.completed_matmuls(), before);
    graph.vulkan = Some(engine);
    let gpu = project(&graph)?;
    assert_eq!(engine.completed_matmuls(), before + 1);
    assert!(cpu.iter().zip(gpu).all(|(a, b)| (a - b).abs() < 1e-4));

    let mut checks = 0;
    let stopped = graph.linear_weight(&x, &w, &bias, &mut || {
        checks += 1;
        anyhow::ensure!(checks < 3, "stopped");
        Ok(())
    }).unwrap_err();
    assert_eq!(stopped.to_string(), "stopped");
    assert!(stopped.downcast_ref::<super::VulkanFailure>().is_none());
    assert_eq!(engine.completed_matmuls(), before + 1);
    Ok(())
}

#[cfg(all(test, target_os = "linux", feature = "vulkan"))]
#[test]
#[ignore = "requires RECORDBOX_TEST_CHECKPOINT and a software Vulkan ICD"]
fn vulkan_checkpoint_matches_cpu() -> anyhow::Result<()> {
    static ENGINE: std::sync::OnceLock<super::vulkan::Engine> = std::sync::OnceLock::new();
    let engine = ENGINE.get_or_init(|| super::vulkan::Engine::new_for_test().expect("software Vulkan ICD"));
    let path = std::env::var_os("RECORDBOX_TEST_CHECKPOINT").expect("checkpoint path");
    let len = SAMPLE_RATE as usize;
    let audio = tests::signal(len);
    let mut graph = Graph::load(Path::new(&path), &Device::Cpu)?;
    let run = |graph: &Graph| separate(graph, &Device::Cpu, &audio, len, &Default::default(), &mut |_, _| true);
    let before = engine.completed_matmuls();
    let expected = run(&graph)?;
    assert_eq!(engine.completed_matmuls(), before);
    graph.vulkan = Some(engine);
    let actual = run(&graph)?;
    // Both transformer branches run six projections in each of the model's five layers.
    assert_eq!(engine.completed_matmuls() - before, 60);
    assert_eq!((expected.len(), actual.len()), (8 * len, 8 * len));
    for ((name, reference), actual) in SOURCES.iter().zip(expected.chunks_exact(2 * len)).zip(actual.chunks_exact(2 * len)) {
        let error = super::compare::relative_error(reference, actual)?;
        println!("{name}: {:.4}% relative RMS difference", 100. * error);
        assert!(error <= 0.03);
    }
    Ok(())
}

fn attention_rows(q: &Tensor, keys_t: &Tensor, values: &Tensor, rows: usize, checkpoint: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<Tensor> {
    let (_, _, len, width) = q.dims4()?;
    let attend = |q: &Tensor| {
        let scores = (q.matmul(keys_t)? / (width as f64).sqrt())?;
        candle_nn::ops::softmax_last_dim(&scores)?.matmul(values)
    };
    if len <= rows {
        return Ok(attend(q)?);
    }
    let mut parts = Vec::new();
    for start in (0..len).step_by(rows) {
        checkpoint()?;
        let block = q.narrow(2, start, rows.min(len - start))?.contiguous()?;
        parts.push(attend(&block)?);
    }
    Ok(Tensor::cat(&parts, 2)?)
}

#[test]
fn attention_tiles_include_all_keys_and_the_last_partial_tile() {
    let q = Tensor::from_vec((0..60).map(|i| i as f32 / 30.).collect(), (1, 3, 5, 4), &Device::Cpu).unwrap();
    let values = q.narrow(2, 0, 3).unwrap().contiguous().unwrap();
    let keys_t = values.transpose(2, 3).unwrap().contiguous().unwrap();
    let run = |rows| attention_rows(&q, &keys_t, &values, rows, &mut || Ok(())).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
    let (full, tiled) = (run(5), run(2));
    assert_eq!(full.len(), tiled.len());
    assert!(full.iter().zip(tiled).all(|(a, b)| (a - b).abs() < 1e-5));
}

/// How a track is cut up and what is returned.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Share of each 7.8 s segment that overlaps the next (upstream default 0.25).
    pub overlap: f64,
    /// Return only this source (0 drums, 1 bass, 2 other, 3 vocals) instead of all four.
    pub source: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Self { overlap: 0.25, source: None }
    }
}

/// Segment start offsets and the triangular weight of upstream `apply_model`.
fn plan(len: usize, overlap: f64) -> (Vec<usize>, Vec<f32>) {
    let segment = TRAINING_FRAMES;
    let stride = (((1.0 - overlap.clamp(0.0, 0.9)) * segment as f64) as usize).max(1);
    let half = segment / 2;
    let maximum = (segment - half) as f32;
    let weight = (0..segment).map(|i| (if i < half { i + 1 } else { segment - i }) as f32 / maximum).collect();
    ((0..len).step_by(stride).collect(), weight)
}

/// The segment for the chunk at `offset`, padded to the segment length from the surrounding
/// audio (centred), planar stereo.
fn segment_at(audio: &[f32], len: usize, offset: usize) -> Vec<f32> {
    let segment = TRAINING_FRAMES;
    let chunk_len = (len - offset).min(segment);
    let start = offset as isize - ((segment - chunk_len) / 2) as isize;
    let mut input = vec![0f32; 2 * segment];
    for ch in 0..2 {
        let (a, b) = (start.max(0) as usize, ((start + segment as isize) as usize).min(len));
        let at = (a as isize - start) as usize;
        input[ch * segment + at..ch * segment + at + (b - a)].copy_from_slice(&audio[ch * len + a..ch * len + b]);
    }
    input
}

/// Free the GPU memory kept for intermediates between segments (call when a job ends).
pub fn release_scratch() {
    #[cfg(target_os = "macos")]
    metal_fast::release_buffers();
}

/// Serialises GPU work between separation jobs (full stems, vocal detection, key analysis).
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run a model over a whole track the way upstream `apply_model` does (segment 343,980,
/// overlapping segments with triangular weights, chunks padded from the surrounding audio).
/// `audio` is planar stereo: left samples then right samples, `len` frames each.
/// Returns planar [source][channel][frame] (4 × 2 × len), or [channel][frame] of the one
/// source asked for. The callback reports completed chunks and is also polled between model layers.
pub fn separate(graph: &Graph, device: &Device, audio: &[f32], len: usize, opts: &Options, progress: &mut dyn FnMut(usize, usize) -> bool) -> anyhow::Result<Vec<f32>> {
    if len == 0 || audio.len() != 2 * len {
        anyhow::bail!("empty or malformed input")
    }
    if opts.source.is_some_and(|s| s >= 4) {
        anyhow::bail!("no such source")
    }
    if !progress(0, 1) { anyhow::bail!("stopped"); }
    #[cfg(target_os = "macos")]
    if let Some(fast) = &graph.fast {
        return separate_accelerated(audio, len, opts, 2, progress, |spec, wave, source| {
            let (freq, frames, segment) = (NFFT/2, Stft::frames(TRAINING_FRAMES), TRAINING_FRAMES);
            let x = Tensor::from_vec(spec, (4, freq, frames), device)?;
            let xt = Tensor::from_vec(wave, (2, 1, segment), device)?;
            let (mut y, mut yt) = fast.forward(&x, &xt)?;
            if let Some(s) = source {
                y = y.narrow(0, s * 4, 4)?;
                yt = yt.narrow(0, s * 2, 2)?;
            }
            Ok((metal_fast::read(&y)?, metal_fast::read(&yt)?))
        });
    }
    #[cfg(feature = "directml")]
    if let Some(engine) = &graph.directml {
        return separate_accelerated(audio, len, opts, engine.buffered_segments(), progress, |spec, wave, source| {
            engine.forward(spec, wave, source)
        });
    }
    let segment = TRAINING_FRAMES;
    let (offsets, weight) = plan(len, opts.overlap);
    let sources: Vec<usize> = opts.source.map(|s| vec![s]).unwrap_or_else(|| (0..4).collect());
    let mut out = vec![0f32; sources.len() * 2 * len];
    let mut sums = vec![0f32; len];
    for (k, &offset) in offsets.iter().enumerate() {
        let chunk_len = (len - offset).min(segment);
        let x = Tensor::from_vec(segment_at(audio, len, offset), (1, 2, segment), device)?;
        let y = {
            let _gpu = GPU.lock();
            graph.forward(&x, &mut || {
                if !progress(k, offsets.len()) { anyhow::bail!("stopped"); }
                Ok(())
            })?.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?
        };
        let crop = (segment - chunk_len) / 2;
        for (n, &s) in sources.iter().enumerate() {
            for ch in 0..2 {
                let (dst, src) = (&mut out[(n * 2 + ch) * len + offset..][..chunk_len], &y[(s * 2 + ch) * segment + crop..][..chunk_len]);
                for i in 0..chunk_len {
                    dst[i] += weight[i] * src[i];
                }
            }
        }
        for i in 0..chunk_len {
            sums[offset + i] += weight[i];
        }
        if !progress(k + 1, offsets.len()) {
            anyhow::bail!("stopped")
        }
    }
    for ch in out.chunks_mut(len) {
        for (v, s) in ch.iter_mut().zip(&sums) {
            *v /= s.max(1e-9);
        }
    }
    Ok(out)
}

/// Mean and (unbiased) standard deviation, accumulated in f64.
fn mean_std(x: &[f32]) -> (f32, f32) {
    let n = x.len() as f64;
    let mean = x.iter().map(|v| *v as f64).sum::<f64>() / n;
    let var = x.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (mean as f32, var.sqrt() as f32)
}

/// One segment ready for the GPU: normalised spectrogram and waveform with their statistics.
#[cfg(any(target_os = "macos", feature = "directml"))]
struct Prepared {
    offset: usize,
    spec: Vec<f32>,
    wave: Vec<f32>,
    /// (mean, std) of the spectrogram and of the waveform.
    stats: [(f32, f32); 2],
}

/// The network's two outputs for one segment, still normalised.
#[cfg(any(target_os = "macos", feature = "directml"))]
struct Raw {
    offset: usize,
    spec: Vec<f32>,
    wave: Vec<f32>,
    stats: [(f32, f32); 2],
}

/// The accelerated path: three stages run side by side so the GPU never waits for the CPU. A
/// thread prepares segments (normalisation and the forward transform), this thread runs the
/// network, and a third does the inverse transform and the overlap-add.
#[cfg(any(target_os = "macos", feature = "directml"))]
fn separate_accelerated(audio: &[f32], len: usize, opts: &Options, buffered_segments: usize, progress: &mut dyn FnMut(usize, usize) -> bool,
    mut forward: impl FnMut(Vec<f32>, Vec<f32>, Option<usize>) -> anyhow::Result<(Vec<f32>, Vec<f32>)>) -> anyhow::Result<Vec<f32>> {
    let segment = TRAINING_FRAMES;
    let freq = NFFT / 2;
    let frames = Stft::frames(segment);
    let plane = freq * frames;
    let (offsets, weight) = plan(len, opts.overlap);
    let sources: Vec<usize> = opts.source.map(|s| vec![s]).unwrap_or_else(|| (0..4).collect());
    let n_src = sources.len();
    let stft = Stft::new();
    let envelope = stft.envelope(frames);
    let mut out = vec![0f32; n_src * 2 * len];
    let (stft, envelope, weight, offsets) = (&stft, &envelope, &weight, &offsets);

    let result = std::thread::scope(|scope| -> anyhow::Result<()> {
        let (tx_in, rx_in) = std::sync::mpsc::sync_channel::<Prepared>(buffered_segments);
        let (tx_out, rx_out) = std::sync::mpsc::sync_channel::<Raw>(buffered_segments);
        scope.spawn(move || {
            for &offset in offsets {
                let input = segment_at(audio, len, offset);
                let mut spec = vec![0f32; 4 * plane];
                std::thread::scope(|s| {
                    for (ch, planes) in spec.chunks_mut(2 * plane).enumerate() {
                        let x = &input[ch * segment..(ch + 1) * segment];
                        s.spawn(move || {
                            let (re, im) = planes.split_at_mut(plane);
                            stft.forward(x, re, im);
                        });
                    }
                });
                let stats = [mean_std(&spec), mean_std(&input)];
                let mut wave = input;
                for (v, (mean, std)) in [(&mut spec, stats[0]), (&mut wave, stats[1])] {
                    let scale = 1.0 / (std + 1e-5);
                    v.iter_mut().for_each(|x| *x = (*x - mean) * scale);
                }
                if tx_in.send(Prepared { offset, spec, wave, stats }).is_err() {
                    return;
                }
            }
        });
        let out = &mut out;
        let post = scope.spawn(move || {
            for raw in rx_out {
                let chunk_len = (len - raw.offset).min(segment);
                let crop = (segment - chunk_len) / 2;
                let raw = &raw;
                std::thread::scope(|s| {
                    for (n, dst) in out.chunks_mut(len).enumerate() {
                        s.spawn(move || {
                            let (re, im) = raw.spec[n * 2 * plane..(n + 1) * 2 * plane].split_at(plane);
                            let mut wave = vec![0f32; segment];
                            stft.inverse(re, im, frames, raw.stats[0].1, raw.stats[0].0, envelope, &mut wave);
                            let time = &raw.wave[n * segment..(n + 1) * segment];
                            let (mean, std) = raw.stats[1];
                            let dst = &mut dst[raw.offset..raw.offset + chunk_len];
                            for i in 0..chunk_len {
                                dst[i] += weight[i] * (wave[crop + i] + time[crop + i] * std + mean);
                            }
                        });
                    }
                });
            }
        });
        let mut run = || -> anyhow::Result<()> {
            for (k, p) in rx_in.iter().enumerate() {
                if !progress(k, offsets.len()) { anyhow::bail!("stopped"); }
                let (spec, wave) = {
                    let _gpu = GPU.lock();
                    forward(p.spec, p.wave, opts.source).with_context(|| format!("Separation segment {} of {}", k + 1, offsets.len()))?
                };
                if tx_out.send(Raw { offset: p.offset, spec, wave, stats: p.stats }).is_err() {
                    anyhow::bail!("stem post-processing stopped")
                }
                if !progress(k + 1, offsets.len()) {
                    anyhow::bail!("stopped")
                }
            }
            Ok(())
        };
        let r = run();
        // Closing both channels lets the other two stages finish.
        drop(rx_in);
        drop(tx_out);
        if post.join().is_err() {
            anyhow::bail!("stem post-processing failed")
        }
        r
    });
    result?;
    let mut sums = vec![0f32; len];
    for &offset in offsets {
        let chunk_len = (len - offset).min(segment);
        for i in 0..chunk_len {
            sums[offset + i] += weight[i];
        }
    }
    for ch in out.chunks_mut(len) {
        for (v, s) in ch.iter_mut().zip(&sums) {
            *v /= s.max(1e-9);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Check the fast ZIP reader against Candle on an optional local Demucs checkpoint.
    #[test]
    #[ignore = "set PREPDECK_STEM_TH to a downloaded HTDemucs checkpoint"]
    fn checkpoint_reader_matches_candle() {
        let path = std::env::var_os("PREPDECK_STEM_TH").expect("provide a local .th checkpoint");
        let path = Path::new(&path);
        let actual = read_checkpoint(path, &Device::Cpu).unwrap();
        let reference = candle_core::pickle::PthTensors::new(path, Some("state")).unwrap();
        assert_eq!(actual.len(), reference.tensor_infos().len());
        for (name, tensor) in actual {
            let expected = reference.get(&name).unwrap().unwrap();
            assert_eq!(tensor.dtype(), expected.dtype(), "{name}");
            assert_eq!(tensor.dims(), expected.dims(), "{name}");
            let values = |t: Tensor| t.to_dtype(DType::F32).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
            assert!(values(tensor).iter().zip(values(expected)).all(|(a, b)| a.to_bits() == b.to_bits()), "{name}");
        }
    }

    #[cfg(feature = "directml")]
    #[test]
    fn driver_failure_on_second_segment_keeps_the_cause_and_returns_no_partial_stems() {
        let len = TRAINING_FRAMES + 4096;
        let audio = vec![0.; 2 * len];
        let mut calls = 0;
        let result = separate_accelerated(&audio, len, &Options { source: Some(3), ..Default::default() },
            0, &mut |_, _| true, |spec, wave, _| {
                calls += 1;
                if calls == 2 { return Err(crate::stems::DirectMlFailure("simulated provider error".into()).into()); }
                Ok((spec, wave))
            });
        let error = result.unwrap_err();
        assert!(error.is::<crate::stems::DirectMlFailure>());
        assert!(format!("{error:#}").contains("Separation segment 2 of 2"));
        assert_eq!(calls, 2);

        let stopped = separate_accelerated(&audio, len, &Default::default(), 0,
            &mut |_, _| false, |_, _, _| panic!("Cancelled work called the provider")).unwrap_err();
        assert!(!stopped.is::<crate::stems::DirectMlFailure>());
    }

    /// Deterministic values in [-1, 1).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        }
    }

    /// A network with HTDemucs's exact tensor names and shapes and random values, so the two
    /// implementations can be compared without a downloaded checkpoint.
    fn random_weights(device: &Device) -> HashMap<String, Tensor> {
        let mut rng = Lcg(7);
        let mut w = HashMap::new();
        // kind 0: weight scaled by fan-in; 1: norm weight near 1; 2: bias; 3: layer scale.
        let mut put = |name: String, dims: &[usize], kind: u8| {
            let n: usize = dims.iter().product();
            let fan = (n / dims[0]).max(1) as f32;
            let (mean, amp) = match kind {
                0 => (0.0, (3.0 / fan).sqrt()),
                1 => (1.0, 0.2),
                2 => (0.0, 0.1),
                _ => (0.3, 0.1),
            };
            let v: Vec<f32> = (0..n).map(|_| mean + amp * rng.next()).collect();
            w.insert(name, Tensor::from_vec(v, dims, device).unwrap());
        };
        let mut dconv = |put: &mut dyn FnMut(String, &[usize], u8), name: String, c: usize| {
            for l in 0..2 {
                let (p, h) = (format!("{name}.layers.{l}"), c / 8);
                put(format!("{p}.0.weight"), &[h, c, 3], 0);
                put(format!("{p}.0.bias"), &[h], 2);
                put(format!("{p}.1.weight"), &[h], 1);
                put(format!("{p}.1.bias"), &[h], 2);
                put(format!("{p}.3.weight"), &[2 * c, h, 1], 0);
                put(format!("{p}.3.bias"), &[2 * c], 2);
                put(format!("{p}.4.weight"), &[2 * c], 1);
                put(format!("{p}.4.bias"), &[2 * c], 2);
                put(format!("{p}.6.scale"), &[c], 3);
            }
        };
        for (prefix, first, freq) in [("encoder", 4, true), ("tencoder", 2, false)] {
            for i in 0..4 {
                let c = 48 << i;
                let cin = if i == 0 { first } else { c / 2 };
                let name = format!("{prefix}.{i}");
                put(format!("{name}.conv.weight"), &[c, cin, 8, 1][..if freq { 4 } else { 3 }], 0);
                put(format!("{name}.conv.bias"), &[c], 2);
                dconv(&mut put, format!("{name}.dconv"), c);
                put(format!("{name}.rewrite.weight"), &[2 * c, c, 1, 1][..if freq { 4 } else { 3 }], 0);
                put(format!("{name}.rewrite.bias"), &[2 * c], 2);
            }
        }
        for (prefix, last, freq) in [("decoder", 16, true), ("tdecoder", 8, false)] {
            for j in 0..4 {
                let c = 384 >> j;
                let cout = if j == 3 { last } else { c / 2 };
                let name = format!("{prefix}.{j}");
                put(format!("{name}.conv_tr.weight"), &[c, cout, 8, 1][..if freq { 4 } else { 3 }], 0);
                put(format!("{name}.conv_tr.bias"), &[cout], 2);
                dconv(&mut put, format!("{name}.dconv"), c);
                put(format!("{name}.rewrite.weight"), &[2 * c, c, 3, 3][..if freq { 4 } else { 3 }], 0);
                put(format!("{name}.rewrite.bias"), &[2 * c], 2);
            }
        }
        put("freq_emb.embedding.weight".into(), &[512, 48], 2);
        for t in ["", "_t"] {
            put(format!("channel_upsampler{t}.weight"), &[512, 384, 1], 0);
            put(format!("channel_upsampler{t}.bias"), &[512], 2);
            put(format!("channel_downsampler{t}.weight"), &[384, 512, 1], 0);
            put(format!("channel_downsampler{t}.bias"), &[384], 2);
            put(format!("crosstransformer.norm_in{t}.weight"), &[512], 1);
            put(format!("crosstransformer.norm_in{t}.bias"), &[512], 2);
            for i in 0..5 {
                let name = format!("crosstransformer.layers{t}.{i}");
                let cross = i % 2 == 1;
                let attn = format!("{name}.{}", if cross { "cross_attn" } else { "self_attn" });
                put(format!("{attn}.in_proj_weight"), &[1536, 512], 0);
                put(format!("{attn}.in_proj_bias"), &[1536], 2);
                put(format!("{attn}.out_proj.weight"), &[512, 512], 0);
                put(format!("{attn}.out_proj.bias"), &[512], 2);
                put(format!("{name}.linear1.weight"), &[2048, 512], 0);
                put(format!("{name}.linear1.bias"), &[2048], 2);
                put(format!("{name}.linear2.weight"), &[512, 2048], 0);
                put(format!("{name}.linear2.bias"), &[512], 2);
                put(format!("{name}.gamma_1.scale"), &[512], 3);
                put(format!("{name}.gamma_2.scale"), &[512], 3);
                for n in ["norm1", "norm2", "norm_out"].into_iter().chain(cross.then_some("norm3")) {
                    put(format!("{name}.{n}.weight"), &[512], 1);
                    put(format!("{name}.{n}.bias"), &[512], 2);
                }
            }
        }
        w
    }

    /// Planar stereo test signal: tones, a sweep and noise, different in the two channels.
    pub(super) fn signal(len: usize) -> Vec<f32> {
        let mut rng = Lcg(3);
        (0..2 * len)
            .map(|n| {
                let (ch, i) = (n / len, (n % len) as f32);
                let t = i / SAMPLE_RATE as f32;
                0.3 * (2.0 * std::f32::consts::PI * (110.0 + 40.0 * ch as f32) * t).sin() + 0.2 * (2.0 * std::f32::consts::PI * (300.0 + 900.0 * t) * t).sin() + 0.1 * rng.next()
            })
            .collect()
    }

    fn snr_db(reference: &[f32], other: &[f32]) -> f64 {
        let (mut s, mut e) = (0f64, 0f64);
        for (a, b) in reference.iter().zip(other) {
            s += (*a as f64).powi(2);
            e += ((*a - *b) as f64).powi(2);
        }
        10.0 * (s / e.max(1e-30)).log10()
    }

    #[test]
    fn transform_round_trip() {
        let len = 20 * HOP + 300;
        let x: Vec<f32> = signal(len)[..len].to_vec();
        let stft = Stft::new();
        let frames = Stft::frames(len);
        let (mut re, mut im) = (vec![0f32; NFFT / 2 * frames], vec![0f32; NFFT / 2 * frames]);
        stft.forward(&x, &mut re, &mut im);
        let mut y = vec![0f32; len];
        stft.inverse(&re, &im, frames, 1.0, 0.0, &stft.envelope(frames), &mut y);
        // As upstream, the frames that hang over the ends are dropped, so only the interior
        // comes back (and without the Nyquist bin).
        let snr = snr_db(&x[NFFT..len - NFFT], &y[NFFT..len - NFFT]);
        assert!(snr > 40.0, "round trip {snr:.1} dB");
    }

    #[test]
    fn segments_cover_the_track() {
        let len = 3 * TRAINING_FRAMES + 1000;
        for overlap in [0.25, 0.1] {
            let (offsets, weight) = plan(len, overlap);
            assert_eq!(weight.len(), TRAINING_FRAMES);
            assert_eq!(offsets[0], 0);
            assert!(offsets.windows(2).all(|w| w[1] - w[0] <= TRAINING_FRAMES));
            assert!(offsets.last().unwrap() + TRAINING_FRAMES >= len);
        }
        // A short last chunk is padded from the audio around it, centred.
        let audio: Vec<f32> = (0..2 * len).map(|i| (i % len) as f32).collect();
        let offset = 3 * TRAINING_FRAMES - 500;
        let seg = segment_at(&audio, len, offset);
        let crop = (TRAINING_FRAMES - (len - offset)) / 2;
        assert_eq!(seg[crop], offset as f32);
        assert_eq!(seg[TRAINING_FRAMES + crop], offset as f32);
        assert_eq!(seg[0], (offset - crop) as f32);
    }

    /// The fused Metal kernels against the reference graph built from Candle's operations,
    /// on the same (random) weights: outputs must agree far below audibility. Skipped where
    /// there is no Metal device.
    #[test]
    fn metal_kernels_match_reference_graph() {
        let Ok(device) = Device::new_metal(0) else { return };
        let weights = random_weights(&device);
        let reference = Graph::from_weights(weights.clone(), false).unwrap();
        let fast = Graph::from_weights(weights, true).unwrap();
        // One full segment and a short, padded one.
        let len = TRAINING_FRAMES + SAMPLE_RATE as usize;
        let audio = signal(len);
        let opts = Options::default();
        let a = separate(&reference, &device, &audio, len, &opts, &mut |_, _| true).unwrap();
        let b = separate(&fast, &device, &audio, len, &opts, &mut |_, _| true).unwrap();
        assert_eq!(a.len(), 8 * len);
        for s in 0..4 {
            let snr = snr_db(&a[s * 2 * len..(s + 1) * 2 * len], &b[s * 2 * len..(s + 1) * 2 * len]);
            eprintln!("{}: fused kernels vs reference {snr:.1} dB", SOURCES[s]);
            assert!(snr > 60.0, "{}: fused kernels differ from the reference ({snr:.1} dB)", SOURCES[s]);
        }
        // One source alone is the same signal as that source of the full result.
        let one = separate(&fast, &device, &audio, len, &Options { source: Some(3), ..opts }, &mut |_, _| true).unwrap();
        assert!(snr_db(&b[6 * len..], &one) > 100.0);
        // Returning false from the progress callback stops the job.
        let mut calls = 0;
        let stopped = separate(&fast, &device, &audio, len, &opts, &mut |_, _| {
            calls += 1;
            false
        });
        assert!(stopped.is_err_and(|e| e.to_string().contains("stopped")) && calls == 1);
        release_scratch();
    }
}
