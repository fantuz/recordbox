use candle_core::Device;
use std::sync::OnceLock;
mod inventory;
pub use inventory::{GpuCapability, GpuState};
pub(super) use inventory::{adapter_identity, discover_adapters};
pub use inventory::gpu_capability;

struct Backend {
    #[cfg(all(windows, feature = "directml"))]
    directml: Option<super::directml::Binding>,
    #[cfg(all(target_os = "linux", feature = "vulkan"))]
    vulkan: Option<super::vulkan::Engine>,
    #[cfg(all(feature = "cuda12", not(any(target_os = "macos", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan")))))]
    preview_limit: Option<&'static str>,
    device: Device,
    label: String,
}

impl Backend {
    fn cpu(reason: &str) -> Self {
        Self {
            #[cfg(all(windows, feature = "directml"))]
            directml: None,
            #[cfg(all(target_os = "linux", feature = "vulkan"))]
            vulkan: None,
            #[cfg(all(feature = "cuda12", not(any(target_os = "macos", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan")))))]
            preview_limit: None,
            device: Device::Cpu,
            label: format!("CPU · {reason}"),
        }
    }
}

static BACKEND: OnceLock<Backend> = OnceLock::new();
static FORCED_CPU: OnceLock<()> = OnceLock::new();

pub fn forced_cpu() -> bool {
    FORCED_CPU.get().is_some()
}

// Device callbacks can run while BACKEND is initializing. They only touch this
// separate lock, so queued jobs and the UI retain the first failure.
static BACKEND_HOLD: OnceLock<String> = OnceLock::new();

pub fn backend_hold() -> Option<&'static str> {
    BACKEND_HOLD.get().map(String::as_str)
}

pub fn radeon_hold() -> Option<&'static str> {
    #[cfg(all(windows, feature = "directml"))]
    { backend_hold() }
    #[cfg(not(all(windows, feature = "directml")))]
    { None }
}

#[cfg(all(windows, feature = "directml"))]
pub(super) fn hold_radeon(reason: String) {
    let _ = BACKEND_HOLD.set(reason);
}

#[cfg(all(target_os = "linux", feature = "vulkan"))]
pub(super) fn hold_vulkan(reason: String) {
    BACKEND_HOLD.get_or_init(|| format!("Vulkan paused for this session · {reason}"));
}

#[cfg(all(target_os = "linux", feature = "vulkan"))]
pub(super) fn vulkan_hold() -> Option<&'static str> {
    backend_hold()
}

pub fn force_cpu() {
    if BACKEND.set(Backend::cpu("selected for this session")).is_ok() {
        let _ = FORCED_CPU.set(());
    }
}

pub fn device() -> Device {
    if backend_hold().is_some() { return Device::Cpu; }
    BACKEND.get_or_init(select).device.clone()
}

#[cfg(all(windows, feature = "directml"))]
pub(super) fn directml_device() -> Option<&'static super::directml::Binding> {
    if backend_hold().is_some() { return None; }
    BACKEND.get_or_init(select).directml.as_ref()
}

#[cfg(all(target_os = "linux", feature = "vulkan"))]
pub(super) fn vulkan_device() -> Option<&'static super::vulkan::Engine> {
    if backend_hold().is_some() { return None; }
    let active = BACKEND.get_or_init(select);
    if backend_hold().is_some() { None } else { active.vulkan.as_ref() }
}

pub fn uses_directml() -> bool {
    #[cfg(all(windows, feature = "directml"))]
    { directml_device().is_some() && radeon_hold().is_none() }
    #[cfg(not(all(windows, feature = "directml")))]
    { false }
}

pub(super) fn gpu_uses_shared_memory() -> bool {
    #[cfg(all(windows, feature = "directml"))]
    { directml_device().is_some_and(|binding| binding.shared_memory) }
    #[cfg(not(all(windows, feature = "directml")))]
    { false }
}

pub fn gpu_active() -> bool {
    #[cfg(all(target_os = "linux", feature = "vulkan"))]
    { vulkan_device().is_some() }
    #[cfg(not(all(target_os = "linux", feature = "vulkan")))]
    { uses_directml() || !device().is_cpu() }
}

#[cfg(all(windows, feature = "directml"))]
fn select() -> Backend {
    if let Some(reason) = radeon_hold() { return Backend::cpu(reason); }
    match super::directml::select() {
        Ok((binding, label)) => Backend { device: Device::Cpu, label, directml: Some(binding) },
        Err(error) => Backend::cpu(&format!("{error:#}")),
    }
}

#[cfg(all(target_os = "linux", feature = "vulkan"))]
fn select() -> Backend {
    match super::vulkan::Engine::new() {
        Ok(engine) => Backend {
            device: Device::Cpu,
            label: format!("Experimental Vulkan + CPU · {} · Vulkan transformer linear layers", engine.label()),
            vulkan: Some(engine),
        },
        Err(error) => Backend::cpu(&format!("Vulkan initialization failed: {error:#}")),
    }
}

/// Reading the published snapshot is safe on the UI thread. The setup worker
/// inventories adapters; only explicit device checks and jobs can open compute.
pub fn status() -> (bool, &'static str) {
    let gpu = gpu_capability();
    (gpu.candidate(), gpu.detail)
}

pub(super) fn preview_device_limit() -> Option<&'static str> {
    #[cfg(all(feature = "cuda12", not(any(target_os = "macos", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan")))))]
    { BACKEND.get().and_then(|b| b.preview_limit) }
    #[cfg(not(all(feature = "cuda12", not(any(target_os = "macos", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan"))))))]
    { None }
}

pub fn backend_label() -> &'static str {
    if let Some(reason) = backend_hold() { return reason; }
    let selected = BACKEND.get_or_init(select);
    backend_hold().unwrap_or(&selected.label)
}

#[cfg(target_os = "macos")]
fn select() -> Backend {
    match Device::new_metal(0) {
        Ok(device) => Backend { device, label: "Apple Metal".into() },
        Err(error) => Backend::cpu(&format!("Metal initialization failed ({error})")),
    }
}

#[cfg(not(any(target_os = "macos", feature = "cuda12", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan"))))]
fn select() -> Backend {
    Backend::cpu("standard build")
}

#[cfg(all(feature = "cuda12", not(any(target_os = "macos", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan")))))]
fn select() -> Backend {
    use candle_core::cuda_backend::cudarc::driver::CudaContext;
    let count = match CudaContext::device_count() {
        Ok(0) => return Backend::cpu("CUDA 12 found no visible NVIDIA devices"),
        Ok(count) => count,
        Err(error) => return Backend::cpu(&format!("CUDA driver initialization failed ({error})")),
    };
    let mut reasons = Vec::new();
    for ordinal in 0..count {
        match cuda_device(ordinal as usize) {
            Ok(backend) => return backend,
            Err(error) => reasons.push(format!("device {ordinal}: {error}")),
        }
    }
    Backend::cpu(&format!("CUDA 12 · {}", reasons.join(", ")))
}

#[cfg(all(feature = "cuda12", not(any(target_os = "macos", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan")))))]
fn cuda_device(ordinal: usize) -> anyhow::Result<Backend> {
    use candle_core::Tensor;
    // One explicit stream can be used by both the assessment and separation threads.
    let device = Device::new_cuda_with_stream(ordinal)?;
    let stream = device.as_cuda_device()?.cuda_stream();
    let context = stream.context();
    let name = context.name()?;
    let (major, minor) = context.compute_capability()?;
    anyhow::ensure!((major, minor) >= (6, 1), "{name} is SM {major}.{minor}, this build needs 6.1 or newer");
    let mib = context.total_mem()? / (1024 * 1024);

    // Exercise both a Candle convolution kernel and cuBLAS, then synchronize by
    // reading the result. Enumeration alone cannot detect incompatible PTX or libraries.
    let x = Tensor::new(&[[[1f32, 2., 3.]]], &device)?;
    let weights = Tensor::new(&[[[2f32, 1.]]], &device)?;
    let conv = x.conv1d(&weights, 0, 1, 1, 1)?.reshape((1, 2))?;
    let column = Tensor::new(&[[1f32], [2.]], &device)?;
    let result = conv.matmul(&column)?.flatten_all()?.to_vec1::<f32>()?;
    anyhow::ensure!(result == [18.0], "{name} failed the FP32 calculation check");
    let preview_limit = if major < 8 || mib < 8192 { Some("CUDA progressive preview needs SM 8.0 or newer and at least 8 GiB VRAM.") } else { None };
    Ok(Backend { device, preview_limit, label: format!("CUDA 12 · {name} · SM {major}.{minor} · {mib} MiB VRAM") })
}

#[cfg(all(test, windows, feature = "directml"))]
mod tests {
    #[test]
    #[ignore = "run in a fresh native test process before any inference"]
    fn idle_radeon_assessment_does_not_open_a_compute_backend() {
        assert!(super::BACKEND.get().is_none());
        let report = crate::stems::status_report();
        assert!(report.contains("Windows DirectML build"));
        assert!(report.contains("System RAM:"));
        assert!(report.contains("HTDemucs (recommended):"));
        assert!(super::BACKEND.get().is_none(), "The passive CLI opened an inference device");
        let hardware = crate::stems::assess();
        let (detected, label) = super::status();
        assert_eq!(hardware.gpu, detected);
        assert!(!label.is_empty());
        assert!(super::BACKEND.get().is_none(), "Startup or Preferences opened an inference device");
        super::force_cpu();
        assert!(!super::status().0);
        assert!(crate::stems::SeparationDevice::GpuOnly.device().is_err());
        assert!(crate::stems::SeparationDevice::Cpu.device().unwrap().is_cpu());
    }

    #[test]
    #[ignore = "requires an isolated model directory, the Windows DirectML runtime and WARP"]
    fn deck_separates_with_both_radeon_models_and_releases_the_gpu_graph() -> anyhow::Result<()> {
        use crate::stems::{self, compare::relative_error, directml::Binding, SeparationDevice};
        use std::sync::Arc;

        let fixture = std::path::PathBuf::from(std::env::var_os("RECORDBOX_TEST_CHECKPOINT").expect("checkpoint"));
        assert_eq!(stems::models_dir().join(stems::CATALOG[1].files[0]), fixture);
        let binding = Binding::warp()?.into_standby();
        assert!(super::BACKEND.set(super::Backend {
            directml: Some(binding),
            device: candle_core::Device::Cpu,
            label: "DirectML WARP with shared GPU memory".into(),
        }).is_ok());
        assert!(super::gpu_uses_shared_memory());

        let frames = 32_768;
        let audio: Vec<f32> = (0..frames).flat_map(|i| {
            let t = i as f32 / 44_100.;
            let kick = if i % 4096 < 32 { 0.25 } else { 0. };
            [(t * 440.).sin() * 0.21 + kick, (t * 1293.).sin() * 0.1 + kick]
        }).collect();
        for model in stems::CATALOG.iter().rev() {
            stems::download(model, &mut |_, _, _| {}, &Default::default())?;
            for (file, hash) in model.files.iter().zip(model.sha256) {
                if let Some(hash) = hash {
                    assert_eq!(stems::sha_hex(&stems::models_dir().join(file))?, *hash);
                }
            }
            let run = |mode| stems::separate_with_device(model, &audio, 44_100, &mut |_| true, mode);
            let actual = run(SeparationDevice::GpuOnly)?;
            let expected = run(SeparationDevice::Cpu)?;
            assert_eq!((actual.len, actual.data.len()), (frames, 8 * frames));
            assert_eq!(actual.data.len(), expected.data.len());
            for ((name, reference), result) in stems::htdemucs::SOURCES.iter()
                .zip(expected.data.chunks_exact(2 * frames)).zip(actual.data.chunks_exact(2 * frames)) {
                let error = relative_error(reference, result)?;
                println!("{} deck {name}: {:.4}% relative RMS difference", model.name, 100. * error);
                assert!(error <= 0.03);
            }
        }
        let file = stems::CATALOG[1].files[0];
        let gpu = stems::graph(file, SeparationDevice::GpuOnly)?;
        let weak = Arc::downgrade(&gpu);
        drop(gpu);
        assert!(weak.upgrade().is_none(), "Idle shared GPU still owns a compiled model");
        let cpu = stems::graph(file, SeparationDevice::Cpu)?;
        assert!(Arc::ptr_eq(&cpu, &stems::graph(file, SeparationDevice::Cpu)?));
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux", feature = "vulkan"))]
mod vulkan_tests {
    #[test]
    #[ignore = "run in a fresh process before any compute backend is selected"]
    fn passive_vulkan_detection_and_cpu_override() {
        assert!(super::BACKEND.get().is_none());
        assert!(crate::stems::status_report().contains("Linux Vulkan"));
        assert!(!super::status().1.is_empty());
        assert!(super::BACKEND.get().is_none());
        super::force_cpu();
        assert!(super::vulkan_device().is_none());
        assert!(!super::status().0);
        assert_eq!(super::backend_label(), "CPU · selected for this session");
        assert!(crate::stems::SeparationDevice::GpuOnly.device().is_err());
    }

    #[test]
    #[ignore = "run in a fresh process to simulate a driver loss"]
    fn vulkan_loss_stops_gpu_requests_and_keeps_explicit_cpu_available() {
        super::hold_vulkan("GPU work timed out".into());
        super::hold_vulkan("second error".into());
        assert!(super::backend_label().contains("timed out"));
        assert_eq!(super::status(), (false, super::backend_label()));
        assert!(!super::gpu_active());
        assert!(super::vulkan_device().is_none());
        for mode in [crate::stems::SeparationDevice::GpuOnly, crate::stems::SeparationDevice::Selected] {
            let error = mode.device().unwrap_err();
            assert!(error.to_string().contains("timed out"));
            assert!(error.is::<crate::stems::VulkanFailure>());
        }
        assert!(crate::stems::SeparationDevice::Cpu.device().unwrap().is_cpu());
        assert!(super::BACKEND.get().is_none(), "a request reopened the GPU after device loss");
    }

    #[test]
    #[ignore = "requires a fresh process, a Vulkan ICD, the HTDemucs checkpoint and RECORDBOX_TEST_AUDIO"]
    fn compare_vulkan_checkpoint_with_cpu() -> anyhow::Result<()> {
        // A software ICD may run the same Vulkan shader in CI. The released
        // application selects physical GPUs only.
        let path = std::env::var("RECORDBOX_TEST_AUDIO")?;
        let engine = crate::stems::vulkan::Engine::new_for_test()?;
        let label = format!("Vulkan test adapter · {}", engine.label());
        assert!(super::BACKEND.set(super::Backend {
            device: candle_core::Device::Cpu,
            label,
            vulkan: Some(engine),
        }).is_ok());
        crate::stems::compare_track(&path)
    }
}
