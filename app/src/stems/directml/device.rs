//! Bind DirectML to an explicit D3D12 queue. The adapter driving the display
//! can differ from the Radeon doing inference, especially in remote sessions.
use anyhow::{ensure, Context, Result};
use ort::{session::builder::SessionBuilder, AsPointer};
use windows::{
    core::Interface,
    Win32::{
        Graphics::{
            Direct3D::D3D_FEATURE_LEVEL_11_0,
            Direct3D12::{
                D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device, D3D12_COMMAND_LIST_TYPE_COMPUTE,
                D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_FEATURE_ARCHITECTURE,
                D3D12_FEATURE_DATA_ARCHITECTURE,
            },
            Dxgi::{
                CreateDXGIFactory1, IDXGIAdapter1, IDXGIAdapter3, IDXGIDevice, IDXGIFactory1,
                DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
                DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
            },
        },
        AI::MachineLearning::DirectML::{DMLCreateDevice, IDMLDevice, DML_CREATE_DEVICE_FLAG_NONE},
    },
};

/// Keep the D3D12 device used for inference so a driver failure can report its own removal reason.
#[derive(Clone)]
pub(in crate::stems) struct Binding {
    adapter: IDXGIAdapter1,
    d3d: Option<ID3D12Device>,
    pub(super) name: String,
    pub(in crate::stems) shared_memory: bool,
}

fn use_shared_memory_profile(uma: Option<bool>) -> bool {
    // The larger, single-pass model requires a confirmed discrete architecture.
    uma.unwrap_or(true)
}

impl Binding {
    fn open_device(adapter: &IDXGIAdapter1) -> Result<ID3D12Device> {
        let mut d3d: Option<ID3D12Device> = None;
        unsafe { D3D12CreateDevice(adapter, D3D_FEATURE_LEVEL_11_0, &mut d3d) }
            .context("Creating D3D12 device")?;
        d3d.context("D3D12 returned no device")
    }

    fn new(adapter: IDXGIAdapter1, name: String) -> Result<Self> {
        let d3d = Self::open_device(&adapter)?;
        let mut arch = D3D12_FEATURE_DATA_ARCHITECTURE::default();
        // Query node zero of the same adapter that owns the execution queue.
        let uma = unsafe {
            d3d.CheckFeatureSupport(
                D3D12_FEATURE_ARCHITECTURE,
                std::ptr::from_mut(&mut arch).cast(),
                std::mem::size_of_val(&arch) as u32,
            )
        }
        .ok()
        .map(|()| arch.UMA.as_bool());
        Ok(Self {
            adapter,
            d3d: Some(d3d),
            name,
            shared_memory: use_shared_memory_profile(uma),
        })
    }

    /// Assessment retains the adapter identity. Only a live session owns a compute device.
    pub(in crate::stems) fn into_standby(mut self) -> Self {
        self.d3d = None;
        self
    }

    pub(super) fn for_model(&self) -> Result<Self> {
        let mut active = self.clone();
        if active.d3d.is_none() {
            active.d3d = Some(Self::open_device(&self.adapter)?);
        }
        Ok(active)
    }

    #[cfg(test)]
    pub(in crate::stems) fn warp() -> Result<Self> {
        use windows::Win32::Graphics::Dxgi::IDXGIFactory4;
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1()? };
        let mut binding = Self::new(unsafe { factory.EnumWarpAdapter()? }, "Microsoft WARP".to_owned())?;
        // Exercise the integrated Radeon profile on the headless Windows runner.
        binding.shared_memory = true;
        Ok(binding)
    }

    pub(super) fn failure_detail(&self) -> String {
        let mut lines = vec![self.name.clone()];
        if let Some(reason) = self
            .d3d
            .as_ref()
            .and_then(|d| unsafe { d.GetDeviceRemovedReason() }.err())
        {
            crate::stems::backend::hold_radeon(format!(
                "DirectML paused for this session · {} · D3D12 device removed (0x{:08X}). Manual CPU separation is available.",
                self.name, reason.code().0 as u32
            ));
            lines.push(format!(
                "D3D12 device removal reason 0x{:08X} ({reason})",
                reason.code().0 as u32
            ));
        }
        if let Ok(adapter) = self.adapter.cast::<IDXGIAdapter3>() {
            for (label, group) in [
                ("Local", DXGI_MEMORY_SEGMENT_GROUP_LOCAL),
                ("Non-local", DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL),
            ] {
                let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
                if unsafe { adapter.QueryVideoMemoryInfo(0, group, &mut info) }.is_ok() && info.Budget != 0 {
                    lines.push(format!(
                        "{label} GPU memory: {} MiB in use / {} MiB budget",
                        info.CurrentUsage >> 20,
                        info.Budget >> 20
                    ));
                }
            }
        }
        lines.join("\n")
    }

    pub(super) fn commands(&self) -> Result<Commands> {
        let d3d = self
            .d3d
            .as_ref()
            .context("DirectML requires an active D3D12 device")?;
        let mut device = None;
        unsafe { DMLCreateDevice(d3d, DML_CREATE_DEVICE_FLAG_NONE, &mut device) }
            .context("Creating DirectML device")?;
        let desc = D3D12_COMMAND_QUEUE_DESC {
            Type: if self.shared_memory {
                D3D12_COMMAND_LIST_TYPE_COMPUTE
            } else {
                D3D12_COMMAND_LIST_TYPE_DIRECT
            },
            ..Default::default()
        };
        let queue = unsafe { d3d.CreateCommandQueue(&desc) }.context("Creating DirectML command queue")?;
        Ok(Commands {
            device: device.context("DirectML returned no device")?,
            queue,
        })
    }
}

/// Shared across a model's ORT session builders on the thread compiling its passes.
pub(super) struct Commands {
    device: IDMLDevice,
    queue: ID3D12CommandQueue,
}

impl Commands {
    pub(super) fn attach(&self, builder: &mut SessionBuilder) -> ort::Result<()> {
        // ORT takes strong references to device and queue on success. Requesting
        // DML1 avoids the legacy provider's extra display-handle lookup.
        unsafe {
            let mut api = std::ptr::null();
            ort::Error::result_from_status((ort::api().GetExecutionProviderApi)(
                c"DML".as_ptr(),
                0,
                &mut api,
            ))?;
            let api = api
                .cast::<ort::sys::OrtDmlApi>()
                .as_ref()
                .ok_or_else(|| ort::Error::new("DirectML provider API unavailable"))?;
            ort::Error::result_from_status((api.SessionOptionsAppendExecutionProvider_DML1)(
                builder.ptr_mut(),
                self.device.as_raw(),
                self.queue.as_raw(),
            ))
        }
    }
}

// DXGI returns four 16-bit version fields, in the same order as Device Manager.
// A Vega 8 using 26.20.14048.2 hung in both HTDemucs modes after passing the probe.
fn vega_driver_hold(name: &str, version: Option<u64>) -> Option<String> {
    let name_lower = name.to_ascii_lowercase();
    if !name_lower
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair == ["vega", "8"])
    {
        return None;
    }
    const REPORTED: u64 = (26 << 48) | (20 << 32) | (14048 << 16) | 2;
    if version.is_some_and(|v| v > REPORTED) {
        return None;
    }
    let driver = driver_version(version);
    Some(format!("DirectML paused · {name} · driver {driver}. A device hang was reported on 26.20.14048.2. Use manual CPU separation or install a newer graphics driver."))
}

fn driver_version(version: Option<u64>) -> String {
    version.filter(|&v| v != 0).map_or_else(
        || "unknown version".into(),
        |v| {
            format!(
                "{}.{}.{}.{}",
                v >> 48,
                (v >> 32) & 0xffff,
                (v >> 16) & 0xffff,
                v & 0xffff
            )
        },
    )
}

struct Radeon {
    index: u32,
    adapter: IDXGIAdapter1,
    name: String,
    driver: Option<u64>,
    dedicated_bytes: usize,
    shared_bytes: usize,
}

// DXGI adapter descriptions are metadata. This path never creates a D3D12 device, queue or ORT session.
fn adapters() -> Result<Vec<Radeon>> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut adapters = Vec::new();
    for index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(a) => a,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(e.into()),
        };
        let desc = unsafe { adapter.GetDesc1()? };
        if desc.VendorId != 0x1002 || desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..end]);
        let version = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }
            .ok()
            .map(|v| v as u64);
        adapters.push(Radeon {
            index, adapter, name, driver: version,
            dedicated_bytes: desc.DedicatedVideoMemory,
            shared_bytes: desc.SharedSystemMemory,
        });
    }
    Ok(adapters)
}

pub(in crate::stems) fn adapter_status() -> Result<String> {
    let adapters = adapters()?;
    ensure!(!adapters.is_empty(), "No physical AMD Radeon adapter found");
    let mut lines = Vec::new();
    for device in adapters {
        let state = vega_driver_hold(&device.name, device.driver)
            .unwrap_or_else(|| "Eligible for the DirectML device check".to_owned());
        lines.push(format!(
            "DXGI adapter {}: {} · driver {}\n  Reported dedicated video memory: {} MiB · shared system memory limit: {} MiB\n  {}",
            device.index, device.name, driver_version(device.driver),
            device.dedicated_bytes >> 20, device.shared_bytes >> 20, state
        ));
    }
    Ok(lines.join("\n"))
}

fn candidates() -> Result<Vec<Radeon>> {
    let mut adapters = adapters()?;
    let mut held = None;
    adapters.retain(|device| match vega_driver_hold(&device.name, device.driver) {
        Some(reason) => { held = Some(reason); false }
        None => true,
    });
    if adapters.is_empty() {
        if let Some(reason) = held {
            crate::stems::backend::hold_radeon(reason.clone());
            anyhow::bail!("{reason}");
        }
    }
    Ok(adapters)
}

pub(in crate::stems) fn available() -> Result<String> {
    candidates()?
        .into_iter()
        .next()
        .map(|d| format!("{} · driver {}", d.name, driver_version(d.driver)))
        .context("no hardware AMD Radeon adapter")
}

pub(in crate::stems) fn select() -> Result<(Binding, String)> {
    let eligible = candidates()?;
    let dll = std::env::current_exe()?.with_file_name("onnxruntime.dll");
    ensure!(
        dll.is_file(),
        "Extract the complete DirectML ZIP with onnxruntime.dll beside recordbox.exe"
    );
    ort::init_from(&dll)
        .context("Loading the packaged ONNX Runtime")?
        .commit();
    let mut failures = Vec::new();
    for Radeon { index, name, adapter, .. } in eligible {
        match Binding::new(adapter, name.clone()).and_then(|binding| {
            super::probe(Some(&binding))?;
            Ok(binding)
        }) {
            Ok(binding) => {
                return Ok((
                    binding.into_standby(),
                    format!("DirectML · {name} · DXGI adapter {index}"),
                ));
            }
            Err(error) => failures.push(format!("{name}: {error:#}")),
        }
    }
    ensure!(
        !failures.is_empty(),
        "DirectML found no hardware AMD Radeon adapter"
    );
    anyhow::bail!("DirectML device check failed: {}", failures.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_uma_uses_the_bounded_inference_profile() {
        assert!(use_shared_memory_profile(None));
        assert!(use_shared_memory_profile(Some(true)));
        assert!(!use_shared_memory_profile(Some(false)));
    }

    #[test]
    #[ignore = "run in a separate process with the software-only D3D12 WARP device"]
    fn device_removal_stops_queued_gpu_jobs_and_preserves_manual_cpu() -> Result<()> {
        use crate::stems::{backend, SeparationDevice};
        use windows::Win32::Graphics::Direct3D12::ID3D12Device5;
        let binding = Binding::warp()?;
        let d3d: ID3D12Device5 = binding.d3d.as_ref().unwrap().cast()?;
        unsafe { d3d.RemoveDevice() };
        let detail = binding.failure_detail();
        assert!(detail.contains("D3D12 device removal reason"));
        assert!(!backend::status().0);
        let preferences = crate::app::Prefs {
            show_stems: true,
            stems_auto_separate: true,
            stems_auto_cpu: true,
            ..Default::default()
        };
        assert!(!preferences.auto_stems_on_load(true));
        assert!(!preferences.auto_stems_on_load(false));
        for execution in [SeparationDevice::Selected, SeparationDevice::GpuOnly] {
            assert!(execution
                .device()
                .unwrap_err()
                .is::<crate::stems::DirectMlFailure>());
            let error = crate::stems::graph("queued-before-removal.th", execution)
                .err()
                .unwrap();
            assert!(error.is::<crate::stems::DirectMlFailure>());
        }
        assert!(SeparationDevice::Cpu.device()?.is_cpu());
        // The deck must be able to finish an actual manual job after the driver stops.
        let frames = 8192;
        let samples: Vec<_> = (0..frames).flat_map(|i| {
            let sample = (i as f32 * 0.03).sin() * 0.5;
            [sample, sample]
        }).collect();
        let model = &crate::stems::CATALOG[1];
        let stems = crate::stems::separate_with_device(model, &samples, 44_100, &mut |_| true, SeparationDevice::Cpu)?;
        assert_eq!((stems.len, stems.data.len()), (frames, frames * 8));
        assert!(stems.data.iter().all(|sample| sample.is_finite()));
        assert!(stems.data.iter().any(|sample| sample.abs() > 1e-5));
        assert!(crate::stems::radeon_hold().is_some());
        Ok(())
    }

    #[test]
    fn vega8_driver_guard_is_specific_and_handles_unknown_versions() {
        let version = (26 << 48) | (20 << 32) | (14048 << 16) | 2;
        let vega = "AMD Radeon(TM) Vega 8 Graphics";
        assert!(vega_driver_hold(vega, Some(version))
            .unwrap()
            .contains("26.20.14048.2"));
        for unknown_or_older in [None, Some(0), Some(version - 1)] {
            assert!(vega_driver_hold(vega, unknown_or_older).is_some());
        }
        assert!(vega_driver_hold(vega, Some(version + 1)).is_none());
        for other in ["Radeon RX Vega 56", "Radeon Vega 80", "AMD Radeon RX 7800 XT"] {
            assert!(vega_driver_hold(other, Some(version)).is_none());
        }
    }
}
