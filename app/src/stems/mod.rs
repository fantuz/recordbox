//! Stem separation (vocals / instruments / bass / drums), free for everyone.
//!
//! On first use Recordbox assesses the machine, picks the most capable open model that runs
//! well on it, downloads it once (checked against its published SHA-256 prefix) and keeps it in
//! the data folder. Separated stems are cached per track so they are only computed once.
//! rekordbox's own (vendor) models are never used.

pub mod htdemucs;
mod live;
pub use live::Live;
#[cfg(test)]
mod bench;
mod backend;
#[cfg(feature = "directml")]
mod directml;
#[cfg(all(target_os = "linux", feature = "vulkan"))]
mod vulkan;
mod compare;
pub use backend::{backend_hold, backend_label, force_cpu, gpu_active, radeon_hold, status as backend_status};
pub use compare::compare_track;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// What this computer offers for running a separation model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hardware {
    pub cpu: String,
    pub cores: usize,
    pub ram_gb: f64,
    pub apple_silicon: bool,
    /// A GPU is available for STEMS. DirectML and Vulkan detection is passive until first use.
    pub gpu: bool,
}

fn sysctl(name: &str) -> Option<String> {
    let o = std::process::Command::new("/usr/sbin/sysctl").arg("-n").arg(name).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

pub fn assess() -> Hardware {
    system_hardware(backend_status().0)
}

fn system_hardware(gpu: bool) -> Hardware {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let (cpu, ram_gb) = if cfg!(target_os = "macos") {
        (sysctl("machdep.cpu.brand_string").unwrap_or_default(), sysctl("hw.memsize").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) / 1e9)
    } else if cfg!(windows) {
        (String::new(), windows_ram_gb())
    } else {
        let mem = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| s.lines().find(|l| l.starts_with("MemTotal")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())).unwrap_or(0.0);
        (String::new(), mem * 1024.0 / 1e9)
    };
    let apple_silicon = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    Hardware { cpu, cores, ram_gb, apple_silicon, gpu }
}

/// Passive support report. In particular, do not call assess()/backend_status() here:
/// a CUDA or Metal build would initialize its compute backend.
pub fn status_report() -> String {
    let hw = system_hardware(false);
    let build = if cfg!(all(windows, feature = "directml")) { "Windows DirectML" }
        else if cfg!(target_os = "macos") { "Apple Metal" }
        else if cfg!(all(target_os = "linux", feature = "vulkan")) { "experimental Linux Vulkan + CPU" }
        else if cfg!(feature = "cuda12") { "CUDA 12" }
        else { "standard CPU" };
    let ram = if hw.ram_gb > 0.0 { format!("{:.1} GB", hw.ram_gb) } else { "unknown".into() };
    let mut lines = vec![
        format!("Recordbox {} · {build} build", env!("CARGO_PKG_VERSION")),
        format!("System RAM: {ram}"),
    ];
    #[cfg(all(target_os = "linux", feature = "vulkan"))]
    lines.push(backend::vulkan_detection().1.to_owned());
    #[cfg(all(windows, feature = "directml"))]
    {
        lines.push(directml::adapter_status().unwrap_or_else(|e| format!("DXGI adapter status: {e:#}")));
        if let Ok(executable) = std::env::current_exe() {
            let missing: Vec<_> = ["onnxruntime.dll", "onnxruntime_providers_shared.dll", "directml.dll"]
                .into_iter().filter(|dll| !executable.with_file_name(dll).is_file()).collect();
            lines.push(if missing.is_empty() { "DirectML runtime files: present".into() }
                else { format!("Missing packaged runtime files: {}", missing.join(", ")) });
        }
    }
    let models = crate::db::shared_data_dir_for_inspection().join("models");
    let recommended = choose(&hw);
    for model in &CATALOG {
        let count = model.files.iter().filter(|file|
            checkpoint_ready(&models.join(file), model.checkpoint_bytes).unwrap_or(false)).count();
        let recommendation = if model.id == recommended.id { " (recommended)" } else { "" };
        lines.push(format!("{}{recommendation}: {count}/{} checkpoint files at expected size", model.name, model.files.len()));
    }
    lines.join("\n")
}

#[cfg(windows)]
fn windows_ram_gb() -> f64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // The API writes to this correctly sized structure for the duration of the call.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 { 0.0 } else { status.ullTotalPhys as f64 / 1e9 }
}

#[cfg(not(windows))]
fn windows_ram_gb() -> f64 { 0.0 }

pub use backend::device;

/// Share of each model segment that overlaps the next. Upstream's default; see
/// checks/stem-speed-study.md for what lower values cost and save.
pub const OVERLAP: f64 = 0.25;
/// Overlap when the result is only analysed (vocal levels, key), never played: 17 % fewer
/// segments; against known stems the separation quality is the same within 0.1 dB.
pub const ANALYSIS_OVERLAP: f64 = 0.1;

/// A provider failure can be retried on the CPU. Model, decoder and cancellation errors stay distinct.
#[derive(Debug)]
pub(crate) struct DirectMlFailure(pub(super) String);
impl std::fmt::Display for DirectMlFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for DirectMlFailure {}

#[cfg(all(target_os = "linux", feature = "vulkan"))]
#[derive(Debug)]
pub(crate) struct VulkanFailure(pub(super) String);
#[cfg(all(target_os = "linux", feature = "vulkan"))]
impl std::fmt::Display for VulkanFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}
#[cfg(all(target_os = "linux", feature = "vulkan"))]
impl std::error::Error for VulkanFailure {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeparationDevice { Selected, GpuOnly, Cpu }

impl SeparationDevice {
    fn check_hold(self) -> Result<()> {
        if self == Self::Cpu { return Ok(()); }
        #[cfg(all(target_os = "linux", feature = "vulkan"))]
        if let Some(reason) = backend::vulkan_hold() { return Err(VulkanFailure(reason.to_owned()).into()); }
        if let Some(reason) = radeon_hold() { return Err(DirectMlFailure(reason.to_owned()).into()); }
        Ok(())
    }

    fn device(self) -> Result<candle_core::Device> {
        if self == Self::Cpu { return Ok(candle_core::Device::Cpu); }
        let selected = device();
        self.check_hold()?;
        if self == Self::GpuOnly && !backend::gpu_active() {
            #[cfg(all(windows, feature = "directml"))]
            return Err(DirectMlFailure(format!("Checking the DirectML device\n{}", backend_label())).into());
            #[cfg(all(target_os = "linux", feature = "vulkan"))]
            return Err(VulkanFailure(format!("Checking the Vulkan device\n{}", backend_label())).into());
            #[cfg(not(any(all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan"))))]
            anyhow::bail!("GPU separation unavailable: {}", backend_label());
        }
        Ok(selected)
    }

    fn load(self, path: &Path) -> Result<htdemucs::Graph> {
        match self {
            Self::Selected | Self::GpuOnly => htdemucs::Graph::load_selected(path),
            Self::Cpu => Ok(htdemucs::Graph::load(path, &candle_core::Device::Cpu)?),
        }
    }

    fn reserve_gpu(self, progress: &mut dyn FnMut(f32) -> bool) -> Result<Option<parking_lot::MutexGuard<'static, ()>>> {
        if self == Self::Cpu { return Ok(None); }
        #[cfg(all(target_os = "linux", feature = "vulkan"))]
        let serialize = backend::vulkan_device().is_some();
        #[cfg(not(all(target_os = "linux", feature = "vulkan")))]
        let serialize = backend::gpu_uses_shared_memory();
        if serialize {
            let guard = wait_for_shared_gpu(&SHARED_GPU_JOB, progress)?;
            self.check_hold()?;
            Ok(Some(guard))
        } else { Ok(None) }
    }
}

// A deck, vocal detection and key analysis can request different checkpoints at the same time.
static SHARED_GPU_JOB: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

fn wait_for_shared_gpu<'a>(slot: &'a parking_lot::Mutex<()>, progress: &mut dyn FnMut(f32) -> bool) -> Result<parking_lot::MutexGuard<'a, ()>> {
    loop {
        let available = slot.try_lock_for(std::time::Duration::from_millis(100));
        if !progress(0.0) { anyhow::bail!("stopped"); }
        if let Some(guard) = available {
            return Ok(guard);
        }
    }
}

type LoadedGraph = std::sync::Arc<htdemucs::Graph>;
static LOADED: std::sync::Mutex<Vec<(String, SeparationDevice, LoadedGraph)>> = std::sync::Mutex::new(Vec::new());

/// Shared-memory DirectML models live for one job. Other backends reuse recent checkpoints.
fn graph(file: &str, execution: SeparationDevice) -> Result<std::sync::Arc<htdemucs::Graph>> {
    // A preceding job may have lost the GPU while this job waited for its turn.
    execution.check_hold()?;
    let execution = if execution == SeparationDevice::GpuOnly { SeparationDevice::Selected } else { execution };
    let file_path = models_dir().join(file);
    let load = || execution.load(&file_path).map(std::sync::Arc::new).with_context(|| format!("Loading {file}"));
    // Let the last owner release the compiled model, command queue and driver allocations on UMA.
    if execution == SeparationDevice::Selected && backend::gpu_uses_shared_memory() {
        return load();
    }
    let mut loaded = LOADED.lock().map_err(|_| anyhow!("model cache is unusable"))?;
    if let Some(i) = loaded.iter().position(|(f, mode, _)| f == file && *mode == execution) {
        // Most recently used last.
        let entry = loaded.remove(i);
        loaded.push(entry);
    } else {
        // A compiled DirectML session also holds intermediate buffers in shared GPU memory.
        let capacity = if execution == SeparationDevice::Selected && backend::uses_directml() { 1 } else { 4 };
        while loaded.len() >= capacity {
            loaded.remove(0);
        }
        loaded.push((file.to_string(), execution, load()?));
    }
    Ok(loaded.last().unwrap().2.clone())
}

/// Stage missing graphs privately. If the driver budget drops or a later allocation
/// runs out of memory, dropping the candidates restores the original cache.
fn quality_graphs(
    files: &[&str; 4], execution: SeparationDevice, live: &Live, progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Option<[LoadedGraph; 4]>> {
    if !live.can_hold_quality_models(0) { return Ok(None); }
    let mut cache = LOADED.lock().map_err(|_| anyhow!("model cache is unusable"))?;
    stage_quality_graphs(files, &mut cache, |remaining| {
        if !progress(0.0) { anyhow::bail!("stopped"); }
        execution.check_hold()?;
        Ok(live.can_hold_quality_models(remaining))
    }, |file| execution.load(&models_dir().join(file)))
}

fn stage_quality_graphs<T>(
    files: &[&str; 4], cache: &mut Vec<(String, SeparationDevice, std::sync::Arc<T>)>,
    mut admit: impl FnMut(usize) -> Result<bool>, mut load: impl FnMut(&str) -> Result<T>,
) -> Result<Option<[std::sync::Arc<T>; 4]>> {
    let mut missing = files.iter().filter(|file|
        !cache.iter().any(|(f, mode, _)| f == **file && *mode == SeparationDevice::Selected)
    ).count();
    let mut candidates = Vec::with_capacity(files.len());
    for &file in files {
        if !admit(missing)? { return Ok(None); }
        let entry = cache.iter().find(|(f, mode, _)| f == file && *mode == SeparationDevice::Selected);
        let g = match entry {
            Some((_, _, g)) => g.clone(),
            None => match load(file) {
                Ok(g) => { missing -= 1; std::sync::Arc::new(g) }
                Err(error) if !candidates.is_empty() && allocation_pressure(&error) => return Ok(None),
                Err(error) => return Err(error).with_context(|| format!("Loading {file}")),
            },
        };
        candidates.push(g);
    }
    if !admit(0)? { return Ok(None); }
    let graphs: [std::sync::Arc<T>; 4] = candidates.try_into().map_err(|_| anyhow!("expected four STEMS specialists"))?;
    cache.clear();
    cache.extend(files.iter().zip(&graphs).map(|(f, g)| (f.to_string(), SeparationDevice::Selected, g.clone())));
    Ok(Some(graphs))
}

fn allocation_pressure(error: &anyhow::Error) -> bool {
    // The CUDA driver distinguishes out-of-memory from a lost or failed device.
    // Metal exposes generic resource errors, which we must propagate.
    #[cfg(feature = "cuda12")]
    {
        use candle_core::cuda_backend::{cudarc::driver::sys::CUresult, CudaError};
        let mut cause = error.downcast_ref::<candle_core::Error>();
        while let Some(error) = cause {
            match error {
                candle_core::Error::Cuda(inner) => return matches!(inner.downcast_ref::<CudaError>(),
                    Some(CudaError::Cuda(err)) if err.0 == CUresult::CUDA_ERROR_OUT_OF_MEMORY),
                candle_core::Error::WithPath { inner, .. } | candle_core::Error::WithBacktrace { inner, .. } |
                    candle_core::Error::Context { inner, .. } => cause = Some(inner),
                _ => break,
            }
        }
    }
    let _ = error;
    false
}

/// Planar stereo (left samples, then right) at 44.1 kHz from interleaved stereo at `sr`.
fn planar_44k(samples: &[f32], sr: u32) -> (Vec<f32>, usize) {
    let resampled;
    let inter = if sr == htdemucs::SAMPLE_RATE {
        samples
    } else {
        resampled = resample_to_44k(samples, sr);
        &resampled
    };
    let len = inter.len() / 2;
    let mut planar = vec![0f32; 2 * len];
    let (l, r) = planar.split_at_mut(len);
    for (i, f) in inter.chunks_exact(2).enumerate() {
        l[i] = f[0];
        r[i] = f[1];
    }
    (planar, len)
}

/// One downloadable separation model.
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    /// Checkpoint files under ROOT_URL; the part after '-' is the first 8 hex digits of the
    /// file's SHA-256 (Demucs's own naming).
    pub files: &'static [&'static str],
    /// true: one specialised model per source (file i provides source i).
    pub per_source: bool,
    /// Byte length of each published checkpoint in this model.
    pub checkpoint_bytes: u64,
    pub min_ram_gb: f64,
    pub needs_gpu: bool,
    pub about: &'static str,
    /// Full SHA-256 of each file when pinned (checked after download); otherwise the
    /// 8-digit prefix in the file name is checked.
    pub sha256: &'static [Option<&'static str>],
    pub licence: &'static str,
    pub source: &'static str,
}

pub const ROOT_URL: &str = "https://dl.fbaipublicfiles.com/demucs/hybrid_transformer/";

/// Both are open HTDemucs models (MIT licence) from facebookresearch/demucs; `choose` picks
/// the recommended one.
pub const CATALOG: [ModelSpec; 2] = [
    ModelSpec {
        id: "htdemucs_ft",
        name: "HTDemucs fine-tuned",
        files: &["f7e0c4bc-ba3fe64a.th", "d12395a8-e57c48e6.th", "92cfc3b6-ef3bcb9c.th", "04573f0d-f3cf25b2.th"],
        per_source: true,
        checkpoint_bytes: 84_141_271,
        min_ram_gb: 15.0,
        needs_gpu: true,
        about: "Four models, each fine-tuned for one stem: a little better on benchmark music, 4× the time.",
        // Hashes of the published files (their name prefixes match), pinned on 2026-11-17.
        sha256: &[
            Some("ba3fe64ae8ef66ac9a4857222ce48efbdc5eb3ad375cb79dd13debee5aaa4066"),
            Some("e57c48e6b0e38af4f7118d7bd08c49f0a0c0edf7d09143bdd902ea0d237303e6"),
            Some("ef3bcb9c8b40d14ae5d51b6db2587339cc12c6b77c0be151ce6d69002e087bf2"),
            Some("f3cf25b222c4eed7cd49dd8b2c9597d50c18bd154090f7b919cfa5f93cf22c49"),
        ],
        licence: "MIT",
        source: "facebookresearch/demucs (Hybrid Transformer Demucs, fine-tuned)",
    },
    ModelSpec {
        id: "htdemucs",
        name: "HTDemucs",
        files: &["955717e8-8726e21a.th"],
        per_source: false,
        checkpoint_bytes: 84_141_911,
        min_ram_gb: 4.0,
        needs_gpu: false,
        about: "One model for all four stems: recommended. Four times faster, and the parts add back up to the track more exactly.",
        sha256: &[Some("8726e21a993978c7ba086d3872e7608d7d5bfca646ca4aca459ffda844faa8b4")],
        licence: "MIT",
        source: "facebookresearch/demucs (Hybrid Transformer Demucs)",
    },
];

/// The recommended model: HTDemucs, on every machine. The fine-tuned set takes four times as
/// long for about +0.2 dB SDR on the MUSDB benchmark (9.0 -> 9.2, upstream's figures); on
/// the tracks tried here it was not better and its four parts add back up to the mix less
/// exactly (checks/stem-speed-study.md), so it stays a choice in Preferences rather than
/// the default.
pub fn choose(_hw: &Hardware) -> &'static ModelSpec {
    &CATALOG[1]
}

/// Whether this machine can run model `m` at all.
#[allow(dead_code)]
pub fn runs_on(m: &ModelSpec, hw: &Hardware) -> bool {
    hw.ram_gb >= m.min_ram_gb && (!m.needs_gpu || hw.gpu)
}

pub fn spec(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn models_dir() -> PathBuf {
    let d = crate::db::shared_data_dir().join("models");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn stems_dir() -> PathBuf {
    let d = crate::db::data_dir().join("stems");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn model_ready(m: &ModelSpec) -> bool {
    let dir = models_dir();
    m.files.iter().all(|f| checkpoint_ready(&dir.join(f), m.checkpoint_bytes).unwrap_or(false))
}

fn checkpoint_ready(path: &Path, expected_bytes: u64) -> Result<bool> {
    match path.metadata() {
        Ok(info) => {
            anyhow::ensure!(info.is_file(), "Expected a model file at {}", path.display());
            Ok(info.len() == expected_bytes)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("Checking {}", path.display())),
    }
}

fn sha_prefix_ok(path: &Path, file_name: &str) -> Result<bool> {
    let want = file_name.split('-').nth(1).and_then(|s| s.split('.').next()).unwrap_or("");
    let got = sha_hex(path)?;
    Ok(!want.is_empty() && got.starts_with(want))
}

fn sha_hex(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// Download any missing checkpoint of `m` into the models folder. Each file is written to a
/// temporary name, checked against its SHA-256 prefix and only then renamed into place.
pub fn download(m: &ModelSpec, progress: &mut dyn FnMut(u64, u64, &str), cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    download_to(m, &models_dir(), progress, cancel, |file| {
        let url = format!("{ROOT_URL}{file}");
        ureq::get(&url).call().with_context(|| format!("downloading {url}"))
    })
}

fn download_to(m: &ModelSpec, dir: &Path, progress: &mut dyn FnMut(u64, u64, &str),
               cancel: &std::sync::atomic::AtomicBool, fetch: impl Fn(&str) -> Result<ureq::Response>) -> Result<()> {
    for (k, f) in m.files.iter().enumerate() {
        let dest = dir.join(f);
        if checkpoint_ready(&dest, m.checkpoint_bytes)? {
            continue;
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) { anyhow::bail!("stopped"); }
        let resp = fetch(f)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        let mut reader = resp.into_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(anyhow!("stopped"));
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            done += n as u64;
            anyhow::ensure!(done <= m.checkpoint_bytes, "{f} exceeds its expected model size");
            tmp.write_all(&buf[..n])?;
            progress(done, m.checkpoint_bytes, f);
        }
        anyhow::ensure!(done == m.checkpoint_bytes, "Incomplete model {f}: received {done} of {} bytes", m.checkpoint_bytes);
        tmp.as_file().sync_all()?;
        let ok = match m.sha256.get(k).copied().flatten() {
            Some(full) => sha_hex(tmp.path())? == full,
            None => sha_prefix_ok(tmp.path(), f)?,
        };
        if !ok {
            return Err(anyhow!("{f} did not match its published checksum; it was not used"));
        }
        tmp.persist(&dest).with_context(|| format!("Installing model {f}"))?;
    }
    Ok(())
}

/// Separated stems of one track: planar f32, [source][channel][frame], 44.1 kHz.
pub struct Stems {
    pub len: usize,
    pub data: Vec<f32>,
}

impl Stems {
    /// Interleaved stereo samples of one source (0 drums, 1 bass, 2 other, 3 vocals).
    pub fn interleaved(&self, source: usize) -> Vec<f32> {
        let (l, r) = (&self.data[(source * 2) * self.len..(source * 2 + 1) * self.len], &self.data[(source * 2 + 1) * self.len..(source * 2 + 2) * self.len]);
        l.iter().zip(r).flat_map(|(a, b)| [*a, *b]).collect()
    }
}

/// Linear resampling of interleaved stereo (used only when a file is not 44.1 kHz).
fn resample_to_44k(samples: &[f32], sr: u32) -> Vec<f32> {
    if sr == htdemucs::SAMPLE_RATE {
        return samples.to_vec();
    }
    let frames = samples.len() / 2;
    let out_frames = (frames as f64 * htdemucs::SAMPLE_RATE as f64 / sr as f64).round() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let p = i as f64 * sr as f64 / htdemucs::SAMPLE_RATE as f64;
        let a = (p.floor() as usize).min(frames.saturating_sub(1));
        let b = (a + 1).min(frames.saturating_sub(1));
        let t = (p - a as f64) as f32;
        for ch in 0..2 {
            out.push(samples[a * 2 + ch] * (1.0 - t) + samples[b * 2 + ch] * t);
        }
    }
    out
}

/// Separate a decoded track with model `m` (which must be downloaded).
pub fn separate(m: &ModelSpec, samples: &[f32], sr: u32, progress: &mut dyn FnMut(f32) -> bool) -> Result<Stems> {
    separate_with_device(m, samples, sr, progress, SeparationDevice::Selected)
}

pub fn separate_with_device(m: &ModelSpec, samples: &[f32], sr: u32, progress: &mut dyn FnMut(f32) -> bool, execution: SeparationDevice) -> Result<Stems> {
    separate_with_live(m, samples, sr, progress, execution, None)
}

/// A preview uses the same weighted segment results as the final cached output.
/// All callers without a preview keep the completed-output path.
pub fn separate_with_live(m: &ModelSpec, samples: &[f32], sr: u32, progress: &mut dyn FnMut(f32) -> bool, execution: SeparationDevice, live: Option<&Live>) -> Result<Stems> {
    if !progress(0.0) { anyhow::bail!("stopped"); }
    anyhow::ensure!(sr > 0 && !samples.is_empty() && samples.len() % 2 == 0, "expected nonempty interleaved stereo and a positive sample rate");
    let device = execution.device()?;
    let _job = execution.reserve_gpu(progress)?;
    let (planar, len) = planar_44k(samples, sr);
    let live = live.filter(|live| live.len() == len && execution != SeparationDevice::Cpu && !device.is_cpu());
    let n_models = m.files.len();
    let run = |progress: &mut dyn FnMut(f32) -> bool| -> Result<Vec<f32>> {
        if !m.per_source {
            if !progress(0.0) { anyhow::bail!("stopped"); }
            let g = graph(m.files[0], execution)?;
            let mut cb = |done: usize, total: usize| progress(done as f32 / total as f32);
            let publish = live.map(|live| live::Publication { live, previous: &[] });
            return htdemucs::separate_publishing(&g, &device, &planar, len, &htdemucs::Options { overlap: OVERLAP, source: None }, &mut cb, publish);
        }
        if let (Some(live), Ok(files)) = (live, m.files.try_into()) {
            if let Some(graphs) = quality_graphs(files, execution, live, progress)? {
                let parts = std::array::from_fn(|i| graphs[i].as_ref());
                let mut cb = |done: usize, total: usize| progress(done as f32 / total as f32);
                let publish = live::Publication { live, previous: &[] };
                return htdemucs::separate_quality_windows(parts, &device, &planar, len, OVERLAP, &mut cb, publish);
            }
        }
        // Model k is the specialist for source k (upstream's weights matrix is the identity),
        // so only that source is taken from it.
        let mut out = Vec::with_capacity(8 * len);
        for (k, f) in m.files.iter().enumerate() {
            if !progress(k as f32 / n_models as f32) { anyhow::bail!("stopped"); }
            let g = graph(f, execution)?;
            let mut cb = |done: usize, total: usize| progress((k as f32 + done as f32 / total as f32) / n_models as f32);
            let publish = live.filter(|_| k == 3).map(|live| live::Publication { live, previous: &out });
            let channel_pair = htdemucs::separate_publishing(&g, &device, &planar, len, &htdemucs::Options { overlap: OVERLAP, source: Some(k) }, &mut cb, publish)?;
            out.extend(channel_pair);
        }
        Ok(out)
    };
    let data = run(progress);
    htdemucs::release_scratch();
    Ok(Stems { len, data: data? })
}

/// The vocal stem alone (mono, 44.1 kHz): the vocals specialist of HTDemucs fine-tuned when
/// it is downloaded, else HTDemucs. Used for vocal detection.
pub fn separate_vocals(samples: &[f32], sr: u32, progress: &mut dyn FnMut(f32) -> bool) -> Result<Vec<f32>> {
    separate_one(3, samples, sr, progress)
}

/// One source (0 drums, 1 bass, 2 other, 3 vocals) as mono 44.1 kHz, running a single model:
/// that source's specialist of HTDemucs fine-tuned when downloaded, else HTDemucs.
pub fn separate_one(source: usize, samples: &[f32], sr: u32, progress: &mut dyn FnMut(f32) -> bool) -> Result<Vec<f32>> {
    if !progress(0.0) { anyhow::bail!("stopped"); }
    let ft = &CATALOG[0];
    let file = if model_ready(ft) { ft.files[source] } else { CATALOG[1].files[0] };
    let device = SeparationDevice::Selected.device()?;
    let _job = SeparationDevice::Selected.reserve_gpu(progress)?;
    let (planar, len) = planar_44k(samples, sr);
    let g = graph(file, SeparationDevice::Selected)?;
    let mut cb = |done: usize, total: usize| progress(done as f32 / total as f32);
    let y = htdemucs::separate(&g, &device, &planar, len, &htdemucs::Options { overlap: ANALYSIS_OVERLAP, source: Some(source) }, &mut cb);
    htdemucs::release_scratch();
    let y = y?;
    Ok(y[..len].iter().zip(&y[len..]).map(|(a, b)| 0.5 * (a + b)).collect())
}

/// Samples per vocal level (rekordbox's PVDI uses 1024 at 22,050 Hz: the same 46.4 ms).
pub const VOCAL_HOP: usize = 2048;
/// Feature: per 46.4 ms frame, the mean of the vocal stem's level (dB) and its level relative
/// to the mix (smoothed over 21 frames), then a centred moving mean over VOCAL_SMOOTH frames;
/// levels from four thresholds. Fitted against rekordbox's own vocal detection (PVDI) on
/// local tracks with leave-one-track-out checks; see PARITY-GAPS.md.
pub const VOCAL_SMOOTH: usize = 15;
pub const VOCAL_THRESHOLDS: [f32; 4] = [-25.41, -21.05, -17.95, -15.47];

fn frame_db(x: &[f32]) -> Vec<f32> {
    x.chunks(VOCAL_HOP).map(|c| 10.0 * (c.iter().map(|v| v * v).sum::<f32>() / c.len().max(1) as f32 + 1e-10).log10()).collect()
}

/// Centred moving mean with the edges held (as numpy's edge padding).
fn moving_mean(x: &[f32], w: usize) -> Vec<f32> {
    let n = x.len();
    if w <= 1 || n == 0 {
        return x.to_vec();
    }
    let pad = w / 2;
    let at = |i: isize| x[i.clamp(0, n as isize - 1) as usize];
    (0..n as isize).map(|i| (0..w as isize).map(|k| at(i - pad as isize + k)).sum::<f32>() / w as f32).collect()
}

/// Mono 44.1 kHz from interleaved stereo at any rate.
pub fn mono_44k(samples: &[f32], sr: u32) -> Vec<f32> {
    resample_to_44k(samples, sr).chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect()
}

/// Vocal levels 0–4 per 2048 samples from mono 44.1 kHz vocal stem and mix.
pub fn vocal_levels(vocal: &[f32], mix: &[f32]) -> Vec<u8> {
    let v = frame_db(vocal);
    let m = moving_mean(&frame_db(mix), 21);
    let f: Vec<f32> = v.iter().enumerate().map(|(i, x)| 0.5 * x + 0.5 * (x - m.get(i).copied().unwrap_or(*x))).collect();
    moving_mean(&f, VOCAL_SMOOTH).iter().map(|x| VOCAL_THRESHOLDS.iter().filter(|t| x >= *t).count() as u8).collect()
}

#[cfg(test)]
mod catalog_tests {
    #[test]
    fn pinned_hashes_match_file_names() {
        for m in super::CATALOG {
            assert_eq!(m.sha256.len(), m.files.len(), "{}", m.id);
            for (f, h) in m.files.iter().zip(m.sha256) {
                if let Some(h) = h {
                    let prefix = f.split('-').nth(1).and_then(|s| s.split('.').next()).unwrap();
                    assert!(h.starts_with(prefix) && h.len() == 64, "{f}");
                }
            }
        }
    }
}

#[cfg(test)]
mod key_bench {
    /// Key from the drum-free mix (mix minus the drums stem) against rekordbox's keys.
    /// PREPDECK_KEYGT (path<TAB>key), PREPDECK_KEY_N; run with --ignored.
    #[test]
    #[ignore]
    fn key_without_drums() {
        let rows: Vec<(String, i32)> = std::fs::read_to_string(std::env::var("PREPDECK_KEYGT").unwrap()).unwrap().lines().filter_map(|l| l.split_once('\t').map(|(p, k)| (p.to_string(), crate::model::parse_key(k)))).collect();
        let n: usize = std::env::var("PREPDECK_KEY_N").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
        let step = (rows.len() / n).max(1);
        let (mut full_ok, mut harm_ok, mut total) = (0, 0, 0);
        for (p, want) in rows.iter().step_by(step).take(n) {
            let Ok(d) = crate::audio::decode_file(p) else { continue };
            let mix44 = super::resample_to_44k(&d.samples, d.sr);
            let mono: Vec<f32> = mix44.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect();
            let drums = super::separate_one(0, &d.samples, d.sr, &mut |_| true).unwrap();
            let harm: Vec<f32> = mono.iter().zip(&drums).map(|(m, dr)| m - dr).collect();
            let k_full = crate::key::detect_key(&mono, 44100.0);
            let k_harm = crate::key::detect_key(&harm, 44100.0);
            total += 1;
            full_ok += (k_full == *want) as usize;
            harm_ok += (k_harm == *want) as usize;
            eprintln!("{} want {} full {} harm {}", p.rsplit('/').next().unwrap_or(""), crate::model::key_name(*want, false), crate::model::key_name(k_full, false), crate::model::key_name(k_harm, false));
        }
        eprintln!("key: full mix {full_ok}/{total}, without drums {harm_ok}/{total}");
    }
}

#[cfg(test)]
mod vocal_bench {
    /// Dump per-frame vocal-stem features next to rekordbox's PVDI levels, for fitting.
    /// PREPDECK_ANLZ_PAIRS, PREPDECK_VOCAL_N, PREPDECK_VOCAL_OUT; run with --ignored.
    #[test]
    #[ignore]
    fn dump_vocal_features() {
        use std::io::Write;
        let pairs = std::fs::read_to_string(std::env::var("PREPDECK_ANLZ_PAIRS").unwrap()).unwrap();
        let n: usize = std::env::var("PREPDECK_VOCAL_N").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
        let mut out = std::fs::File::create(std::env::var("PREPDECK_VOCAL_OUT").unwrap()).unwrap();
        writeln!(out, "track\tframe\tvocal_db\tmix_db\tlevel\tvocal_hp_db").unwrap();
        // Spread over the list (every k-th pair).
        let lines: Vec<&str> = pairs.lines().collect();
        let step = (lines.len() / n).max(1);
        for (ti, line) in lines.iter().step_by(step).take(n).enumerate() {
            let Some((path, base)) = line.split_once('\t') else { continue };
            let sec = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.2EX")));
            let Some(pv) = sec.get("PVDI").and_then(|s| crate::app::parse_pvdi(s)) else { continue };
            let Ok(dec) = crate::audio::decode_file(path) else { continue };
            let t0 = std::time::Instant::now();
            let voc = super::separate_vocals(&dec.samples, dec.sr, &mut |_| true).unwrap();
            let mix = super::resample_to_44k(&dec.samples, dec.sr);
            // Vocal stem high-passed at ~200 Hz (one-pole, twice) to drop bass/kick bleed.
            let a = (1.0 / (1.0 + 2.0 * std::f32::consts::PI * 200.0 / 44100.0)) as f32;
            let hp = |x: &[f32]| {
                let mut y = vec![0f32; x.len()];
                let (mut px, mut py) = (0f32, 0f32);
                for (i, v) in x.iter().enumerate() {
                    py = a * (py + v - px);
                    px = *v;
                    y[i] = py;
                }
                y
            };
            let voc_hp = hp(&hp(&voc));
            let hop = (pv.0 * 44100.0).round() as usize;
            for (f, lvl) in pv.1.iter().enumerate() {
                let a = f * hop;
                if a + hop > voc.len() {
                    break;
                }
                let e_v: f32 = voc[a..a + hop].iter().map(|x| x * x).sum::<f32>() / hop as f32;
                let e_m: f32 = (a..a + hop).map(|i| 0.25 * (mix[2 * i] + mix[2 * i + 1]).powi(2)).sum::<f32>() / hop as f32;
                let e_h: f32 = voc_hp[a..a + hop].iter().map(|x| x * x).sum::<f32>() / hop as f32;
                writeln!(out, "{ti}\t{f}\t{:.2}\t{:.2}\t{lvl}\t{:.2}", 10.0 * (e_v + 1e-10).log10(), 10.0 * (e_m + 1e-10).log10(), 10.0 * (e_h + 1e-10).log10()).unwrap();
            }
            eprintln!("{ti}: {} in {:.1}s", path.rsplit('/').next().unwrap_or(""), t0.elapsed().as_secs_f32());
        }
    }
}

// ------------------------------------------------------------------ cache

/// Cache file for a track's stems: keyed by the audio file's path, size and modification time
/// and the model, so a replaced file or another model is separated again.
pub fn cache_path(track_path: &str, model: &str) -> PathBuf {
    stems_dir().join(cache_file_name(track_path, model, crate::audio::mp3_timeline()))
}

/// Same-clock MP3 and all non-MP3 cache keys are compatible. A read can reuse
/// that key across collections; writes and quota enforcement stay in the
/// active collection even if the read fails.
pub fn existing_cache_path(destination: &Path) -> Option<PathBuf> {
    if destination.is_file() { return Some(destination.to_owned()); }
    use crate::audio::Mp3Timeline::{Legacy, Rekordbox};
    let other = match crate::audio::mp3_timeline() { Legacy => Rekordbox, Rekordbox => Legacy };
    let base = crate::db::shared_data_dir_for_inspection();
    let candidate = crate::db::collection_dir(base, other).join("stems").join(destination.file_name()?);
    candidate.is_file().then_some(candidate)
}

fn cache_file_name(track_path: &str, model: &str, timeline: crate::audio::Mp3Timeline) -> String {
    let meta = std::fs::metadata(track_path).ok();
    let mut h = Sha256::new();
    h.update(track_path.as_bytes());
    h.update(meta.as_ref().map(|m| m.len()).unwrap_or(0).to_le_bytes());
    if let Some(t) = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
        h.update(t.as_secs().to_le_bytes());
    }
    if timeline == crate::audio::Mp3Timeline::Rekordbox && crate::audio::is_mp3(track_path) {
        h.update(b"\0mp3-rekordbox-timeline-v1");
    }
    let key = format!("{:x}", h.finalize());
    format!("{}-{model}.stems", &key[..24])
}

#[cfg(test)]
#[test]
fn mp3_timeline_only_changes_opt_in_mp3_cache_names() {
    use crate::audio::Mp3Timeline::{Legacy, Rekordbox};
    let dir = tempfile::tempdir().unwrap();
    let mp3 = dir.path().join("track.MP3");
    let mp3 = mp3.to_str().unwrap();
    let flac = dir.path().join("track.flac");
    let flac = flac.to_str().unwrap();
    let mut original = Sha256::new();
    original.update(mp3.as_bytes());
    original.update(0u64.to_le_bytes()); // missing file, as in the historical cache key
    let digest = format!("{:x}", original.finalize());
    assert_eq!(cache_file_name(mp3, "model", Legacy), format!("{}-model.stems", &digest[..24]));
    assert_ne!(cache_file_name(mp3, "model", Legacy), cache_file_name(mp3, "model", Rekordbox));
    assert_eq!(cache_file_name(flac, "model", Legacy), cache_file_name(flac, "model", Rekordbox));
}

const MAGIC: &[u8; 8] = b"PDSTEMS1";

/// Stored as 16-bit PCM (planar) to keep the cache small.
pub fn save(path: &Path, s: &Stems) -> Result<()> {
    anyhow::ensure!(s.len.checked_mul(8) == Some(s.data.len()), "incomplete separated stems");
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().context("missing STEMS cache directory")?)?;
    tmp.write_all(MAGIC)?;
    tmp.write_all(&(s.len as u64).to_le_bytes())?;
    // Converted a block at a time: one write per sample made this the slowest step after the
    // model itself.
    let mut block = Vec::with_capacity(2 << 16);
    for chunk in s.data.chunks(1 << 16) {
        block.clear();
        for v in chunk {
            block.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
        tmp.write_all(&block)?;
    }
    // On disk before it gets its final name, so a crash can't leave a short cache file that
    // looks complete.
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Stems> {
    let mut file = std::fs::File::open(path)?;
    let mut header = [0; 16];
    file.read_exact(&mut header).context("reading the STEMS cache header")?;
    anyhow::ensure!(&header[..8] == MAGIC, "not a stems cache file");
    let frames = u64::from_le_bytes(header[8..].try_into()?);
    let bytes = file.metadata()?.len().checked_sub(16).context("stems cache file is incomplete")?;
    anyhow::ensure!(frames.checked_mul(16) == Some(bytes), "stems cache file is incomplete");
    let len = usize::try_from(frames)?;
    let samples = usize::try_from(bytes / 2)?;
    let mut data = Vec::new();
    data.try_reserve_exact(samples).context("allocating memory for saved stems")?;
    // Decode directly into the playback allocation. A four-minute track has about 169 MB of PCM.
    let mut block = vec![0; 128 * 1024];
    while data.len() < samples {
        let count = (samples - data.len()).min(block.len() / 2);
        let part = &mut block[..count * 2];
        file.read_exact(part)?;
        data.extend(part.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32767.0));
    }
    // Mounted volumes can suppress automatic access times. Restoring audio still works on
    // read-only media even when the cache cannot record this use.
    let accessed = std::fs::FileTimes::new().set_accessed(std::time::SystemTime::now());
    #[cfg(not(windows))]
    let _ = file.set_times(accessed);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Windows requires FILE_WRITE_ATTRIBUTES even when only updating the access time.
        if let Ok(attrs) = std::fs::OpenOptions::new().access_mode(0x100).open(path) {
            let _ = attrs.set_times(accessed);
        }
    }
    Ok(Stems { len, data })
}

// Match only files produced by cache_path. Other files in a shared directory and
// temporary files from an active separation must survive both trimming and clearing.
fn saved_entries(dir: &Path) -> std::io::Result<Vec<std::fs::DirEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some((key, model)) = name.to_str().and_then(|n| n.strip_suffix(".stems")).and_then(|n| n.split_once('-')) else { continue };
        if key.len() == 24 && key.bytes().all(|b| b.is_ascii_hexdigit())
            && spec(model).is_some() && entry.file_type()?.is_file() {
            entries.push(entry);
        }
    }
    Ok(entries)
}

pub fn cache_size() -> std::io::Result<u64> {
    let mut bytes = 0;
    for entry in saved_entries(&stems_dir())? {
        match entry.metadata() {
            Ok(metadata) => bytes += metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(bytes)
}

/// Apply the current retention policy after a successful save. Always keep that result,
/// including when a single track exceeds the whole budget.
pub fn trim_cache(keep: &Path, limit: impl Fn() -> Option<u64>) {
    if limit().is_none() { return; }
    let Some(dir) = keep.parent() else { return };
    let Ok(entries) = saved_entries(dir) else { return };
    let mut files: Vec<_> = entries.into_iter()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.accessed().or_else(|_| m.modified()).ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        let Some(limit) = limit() else { break };
        if total <= limit {
            break;
        }
        if path != keep && std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

pub fn clear_cache() -> Result<()> {
    clear_saved_entries(&stems_dir())
}

fn clear_saved_entries(dir: &Path) -> Result<()> {
    for entry in saved_entries(dir)? {
        let path = entry.path();
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error).with_context(|| format!("Removing {}", path.display()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_uses_the_last_disk_load_and_keeps_other_files() -> Result<()> {
        use std::fs::{File, FileTimes};
        use std::time::{Duration, UNIX_EPOCH};

        let folder = tempfile::tempdir()?;
        let speed = folder.path().join("000000000000000000000001-htdemucs.stems");
        let quality = folder.path().join("000000000000000000000001-htdemucs_ft.stems");
        let newest = folder.path().join("000000000000000000000002-htdemucs.stems");
        let parts = Stems { len: 1, data: vec![0.0; 8] };
        for (index, path) in [&quality, &speed, &newest].into_iter().enumerate() {
            save(path, &parts)?;
            let then = UNIX_EPOCH + Duration::from_secs(1_000_000_000 + index as u64 * 300);
            File::options().write(true).open(path)?.set_times(FileTimes::new().set_accessed(then).set_modified(then))?;
        }
        let foreign = folder.path().join("my-recording.stems");
        let temp = folder.path().join(".in-progress");
        std::fs::write(&foreign, b"recording")?;
        std::fs::write(&temp, b"in progress")?;
        load(&quality)?;
        let result_bytes = std::fs::metadata(&quality)?.len();
        trim_cache(&newest, || Some(result_bytes * 2));
        assert!(!speed.exists());
        assert!(quality.exists() && newest.exists());
        trim_cache(&newest, || Some(0));
        assert!(!quality.exists() && newest.exists());
        clear_saved_entries(folder.path())?;
        assert!(!newest.exists());
        assert_eq!(std::fs::read(foreign)?, b"recording");
        assert_eq!(std::fs::read(temp)?, b"in progress");
        Ok(())
    }

    #[test]
    fn incomplete_checkpoint_is_replaced_only_after_a_verified_download() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let m = ModelSpec {
            files: &["test-ba7816bf.th"], sha256: &[None], checkpoint_bytes: 3,
            ..CATALOG[1]
        };
        let dest = dir.path().join(m.files[0]);
        std::fs::write(&dest, b"a")?;
        let cancel = Default::default();
        for (bytes, error) in [("a", "Incomplete model"), ("xyz", "checksum"), ("abcd", "exceeds")] {
            let err = download_to(&m, dir.path(), &mut |_, _, _| {}, &cancel,
                |_| Ok(ureq::Response::new(200, "OK", bytes)?)).unwrap_err();
            assert!(err.to_string().contains(error), "{err}");
            assert_eq!(std::fs::read(&dest)?, b"a");
            assert_eq!(dir.path().read_dir()?.count(), 1);
        }
        download_to(&m, dir.path(), &mut |_, _, _| {}, &cancel,
            |_| Ok(ureq::Response::new(200, "OK", "abc")?))?;
        assert!(checkpoint_ready(&dest, m.checkpoint_bytes)?);
        assert_eq!(std::fs::read(&dest)?, b"abc");
        download_to(&m, dir.path(), &mut |_, _, _| {}, &cancel, |_| panic!("already complete"))?;
        Ok(())
    }

    #[test]
    fn waiting_for_a_shared_gpu_can_be_cancelled_and_the_next_job_can_run() {
        let slot = parking_lot::Mutex::new(());
        let first_job = slot.lock();
        std::thread::scope(|s| {
            let waiting = s.spawn(|| wait_for_shared_gpu(&slot, &mut |_| false).unwrap_err().to_string());
            assert_eq!(waiting.join().unwrap(), "stopped");
        });
        drop(first_job);
        assert_eq!(wait_for_shared_gpu(&slot, &mut |_| false).unwrap_err().to_string(), "stopped");
        assert!(wait_for_shared_gpu(&slot, &mut |_| true).is_ok());
    }

    #[test]
    fn quality_staging_rolls_back_on_pressure_and_reuses_warm_graphs() {
        use std::sync::Arc;
        let mode = SeparationDevice::Selected;
        let warm = Arc::new(1);
        let mut cache = vec![("drums".to_owned(), mode, warm.clone())];
        let files = ["drums", "bass", "other", "vocals"];
        let loads = std::cell::Cell::new(0);
        let load = |_: &str| { loads.set(loads.get() + 1); Ok(2) };
        // Keep one old graph and release both privately staged models on the last load.
        assert!(stage_quality_graphs(&files, &mut cache, |missing| Ok(missing > 1), load).unwrap().is_none());
        assert_eq!(loads.get(), 2);
        assert_eq!(cache.len(), 1);
        assert_eq!(Arc::strong_count(&warm), 2);
        let error = stage_quality_graphs(&files, &mut cache, |_| Ok(true), |_| Err(anyhow!("device lost"))).unwrap_err();
        assert!(format!("{error:#}").contains("device lost"));
        assert_eq!(cache.len(), 1);
        let result = stage_quality_graphs(&files, &mut cache, |_| Ok(true), load).unwrap().unwrap();
        assert_eq!(loads.get(), 5);
        assert!(Arc::ptr_eq(&result[0], &warm));
        assert_eq!(cache.len(), 4);
    }

    #[test]
    fn generic_device_errors_are_never_retried_as_memory_pressure() {
        assert!(!allocation_pressure(&anyhow!("device lost")));
        assert!(!allocation_pressure(&anyhow!("out of memory")));
        #[cfg(feature = "cuda12")]
        {
            use candle_core::cuda_backend::{cudarc::driver::{result::DriverError, sys::CUresult}, CudaError};
            let error = |code| anyhow::Error::from(candle_core::Error::from(CudaError::Cuda(DriverError(code))));
            assert!(allocation_pressure(&error(CUresult::CUDA_ERROR_OUT_OF_MEMORY)));
            assert!(!allocation_pressure(&error(CUresult::CUDA_ERROR_LAUNCH_FAILED)));
        }
    }

    #[test]
    fn cancelled_separation_needs_no_weights_or_decoded_audio() {
        let err = super::separate(&super::CATALOG[1], &[], 44_100, &mut |_| false).err().unwrap();
        assert_eq!(err.to_string(), "stopped");
    }

    #[test]
    fn chooses_best_model_for_hardware() {
        let big = Hardware { cpu: String::new(), cores: 10, ram_gb: 32.0, apple_silicon: true, gpu: true };
        let small = Hardware { cpu: String::new(), cores: 4, ram_gb: 8.0, apple_silicon: false, gpu: false };
        let no_gpu = Hardware { gpu: false, ..big.clone() };
        // The single model is recommended everywhere; the fine-tuned set is an option where
        // it can run.
        assert_eq!(choose(&big).id, "htdemucs");
        assert_eq!(choose(&small).id, "htdemucs");
        assert_eq!(choose(&no_gpu).id, "htdemucs");
        let ft = spec("htdemucs_ft").unwrap();
        assert!(runs_on(ft, &big) && !runs_on(ft, &small) && !runs_on(ft, &no_gpu));
    }

    #[test]
    fn incomplete_stems_do_not_replace_a_saved_track() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let saved = dir.path().join("track.stems");
        save(&saved, &Stems { len: 1, data: vec![0.; 8] })?;
        assert!(save(&saved, &Stems { len: 2, data: vec![0.; 8] }).is_err());
        assert_eq!(load(&saved)?.len, 1);
        let mut header = MAGIC.to_vec();
        header.extend_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(&saved, header)?;
        assert!(load(&saved).is_err());
        Ok(())
    }

    #[test]
    fn cache_file_round_trip() {
        let s = Stems { len: 70_000, data: (0..8 * 70_000).map(|i| ((i % 2001) as f32 / 1000.0 - 1.0) * 1.2).collect() };
        let path = std::env::temp_dir().join(format!("recordbox-stems-test-{}.stems", std::process::id()));
        save(&path, &s).unwrap();
        let back = load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.len, s.len);
        // 16-bit, clipped to full scale.
        assert!(s.data.iter().zip(&back.data).all(|(a, b)| (a.clamp(-1.0, 1.0) - b).abs() < 1.0 / 32000.0));
    }

    /// End-to-end timing on a real track, per stage, as the app runs it. Set
    /// PREPDECK_STEM_TEST_AUDIO to an audio file and optionally PREPDECK_STEM_MODEL
    /// (htdemucs / htdemucs_ft; the model must be in the models folder). Nothing is written
    /// to the stems cache. Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn bench_full_track() {
        let audio = std::env::var("PREPDECK_STEM_TEST_AUDIO").unwrap_or_default();
        if audio.is_empty() {
            return;
        }
        let m = std::env::var("PREPDECK_STEM_MODEL").ok().and_then(|id| spec(&id)).unwrap_or_else(|| choose(&assess()));
        if !model_ready(m) {
            println!("{} is not downloaded", m.id);
            return;
        }
        let t = std::time::Instant::now();
        let dec = crate::audio::decode_file(&audio).unwrap();
        let secs = dec.samples.len() as f64 / 2.0 / dec.sr as f64;
        println!("{}: {secs:.1} s of audio, device {:?}", m.id, device());
        println!("decode            {:6.2} s", t.elapsed().as_secs_f64());
        for pass in ["first (loads the model)", "second (model resident)"] {
            let t = std::time::Instant::now();
            let st = separate(m, &dec.samples, dec.sr, &mut |_| true).unwrap();
            let d = t.elapsed().as_secs_f64();
            println!("separate, {pass:<24} {d:6.2} s = {:.1}x real time", secs / d);
            if pass.starts_with("second") {
                let path = std::env::temp_dir().join(format!("recordbox-stems-bench-{}.stems", std::process::id()));
                let t = std::time::Instant::now();
                save(&path, &st).unwrap();
                println!("cache write       {:6.2} s", t.elapsed().as_secs_f64());
                let t = std::time::Instant::now();
                let back = load(&path).unwrap();
                println!("cache read        {:6.2} s", t.elapsed().as_secs_f64());
                let _ = std::fs::remove_file(&path);
                assert_eq!(back.len, st.len);
            }
        }
        let t = std::time::Instant::now();
        let v = separate_vocals(&dec.samples, dec.sr, &mut |_| true).unwrap();
        println!("vocal stem only   {:6.2} s ({} samples)", t.elapsed().as_secs_f64(), v.len());
    }

    /// Separates 20 s of a real track with the local HTDemucs checkpoint (from the research
    /// folder) and checks that the four stems add back up to the mix. Run with --ignored.
    #[test]
    #[ignore]
    fn separates_and_stems_sum_to_mix() {
        let th = dirs::home_dir().unwrap().join("Developer/rekordbox-rea/analysis/open-model-harness/open-reference/955717e8-8726e21a.th");
        let audio = std::env::var("PREPDECK_STEM_TEST_AUDIO").unwrap_or_default();
        if !th.exists() || audio.is_empty() {
            return;
        }
        let device = device();
        println!("device: {:?}", device);
        let t0 = std::time::Instant::now();
        let graph = htdemucs::Graph::load(&th, &device).unwrap();
        let opts = htdemucs::Options::default();
        println!("loaded .th in {:.1}s", t0.elapsed().as_secs_f64());
        let dec = crate::audio::decode_file(&audio).unwrap();
        assert_eq!(dec.sr, 44100);
        let start = 60 * 44100;
        let len = 20 * 44100;
        let seg = &dec.samples[start * 2..(start + len) * 2];
        let mut planar = vec![0f32; 2 * len];
        for i in 0..len {
            planar[i] = seg[2 * i];
            planar[len + i] = seg[2 * i + 1];
        }
        let t1 = std::time::Instant::now();
        let y = htdemucs::separate(&graph, &device, &planar, len, &opts, &mut |d, t| {
            println!("chunk {d}/{t} at {:.1}s", t1.elapsed().as_secs_f64());
            true
        })
        .unwrap();
        println!("separated 20 s in {:.1}s", t1.elapsed().as_secs_f64());
        let mut err = 0f64;
        let mut energy = 0f64;
        let mut src_energy = [0f64; 4];
        for ch in 0..2 {
            for i in 0..len {
                let sum: f32 = (0..4).map(|s| y[(s * 2 + ch) * len + i]).sum();
                let x = planar[ch * len + i];
                err += ((sum - x) as f64).powi(2);
                energy += (x as f64).powi(2);
                for s in 0..4 {
                    src_energy[s] += (y[(s * 2 + ch) * len + i] as f64).powi(2);
                }
            }
        }
        let snr = 10.0 * (energy / err.max(1e-12)).log10();
        println!("mix reconstruction SNR {snr:.1} dB; stem energy share {:?}", src_energy.map(|e| (e / energy * 100.0).round()));
        assert!(snr > 20.0, "stems should add back up to the mix");
    }
}
