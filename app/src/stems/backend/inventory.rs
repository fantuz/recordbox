//! Adapter metadata for startup. No call here is allowed to initialize BACKEND,
//! allocate model memory, compile shaders or dispatch a computation.
use super::{backend_hold, forced_cpu, Backend, BACKEND};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuState {
    CpuOnly,
    Unchecked,
    Detected,
    Ready,
    Unavailable,
    Paused,
}

#[derive(Clone, Copy, Debug)]
pub struct GpuCapability {
    pub package: &'static str,
    pub state: GpuState,
    pub detail: &'static str,
}

impl GpuCapability {
    pub fn candidate(self) -> bool {
        matches!(self.state, GpuState::Detected | GpuState::Ready)
    }
}

const PACKAGE: &str = if cfg!(all(windows, feature = "directml")) { "Windows DirectML" }
    else if cfg!(target_os = "macos") { "Apple Metal" }
    else if cfg!(all(target_os = "linux", feature = "vulkan")) { "experimental Linux Vulkan + CPU" }
    else if cfg!(feature = "cuda12") { "CUDA 12" }
    else { "standard CPU" };

struct Inventory { state: GpuState, identity: String, detail: String }

impl Inventory {
    #[allow(dead_code)] // Only GPU packages inventory adapters.
    fn detected(identity: String, note: &str) -> Self {
        let detail = format!("{identity} · {note}");
        Self { state: GpuState::Detected, identity, detail }
    }
    fn message(state: GpuState, detail: impl Into<String>) -> Self {
        Self { state, identity: String::new(), detail: detail.into() }
    }
}

static PASSIVE: OnceLock<Inventory> = OnceLock::new();

pub(in crate::stems) fn discover_adapters() {
    if !forced_cpu() && backend_hold().is_none() {
        PASSIVE.get_or_init(inspect);
    }
}

pub(in crate::stems) fn adapter_identity() -> &'static str {
    PASSIVE.get().map_or("", |i| &i.identity)
}

fn uses_gpu(backend: &Backend) -> bool {
    #[cfg(all(windows, feature = "directml"))]
    if backend.directml.is_some() { return true; }
    #[cfg(all(target_os = "linux", feature = "vulkan"))]
    if backend.vulkan.is_some() { return true; }
    !backend.device.is_cpu()
}

pub fn gpu_capability() -> GpuCapability {
    let (state, detail) = if forced_cpu() {
        (GpuState::CpuOnly, "CPU · selected for this session")
    } else if let Some(reason) = backend_hold() {
        (GpuState::Paused, reason)
    } else if PACKAGE == "standard CPU" {
        (GpuState::CpuOnly, "CPU · standard build")
    } else if let Some(backend) = BACKEND.get() {
        (if uses_gpu(backend) { GpuState::Ready } else { GpuState::Unavailable }, backend.label.as_str())
    } else if let Some(inventory) = PASSIVE.get() {
        (inventory.state, inventory.detail.as_str())
    } else {
        (GpuState::Unchecked, "Adapter inventory pending")
    };
    GpuCapability { package: PACKAGE, state, detail }
}

#[cfg(all(windows, feature = "directml"))]
fn inspect() -> Inventory {
    match crate::stems::directml::available() {
        Ok(name) => Inventory::detected(name, "DirectML device checked on first use"),
        Err(error) => match backend_hold() {
            Some(reason) => Inventory { state: GpuState::Paused, identity: reason.to_owned(), detail: reason.to_owned() },
            None => Inventory::message(GpuState::Unavailable, format!("CPU · {error:#}")),
        }
    }
}

#[cfg(all(target_os = "linux", feature = "vulkan"))]
fn inspect() -> Inventory {
    match crate::stems::vulkan::adapter_names() {
        Ok(names) if names.is_empty() => Inventory::message(GpuState::Unavailable, "CPU · no physical Vulkan compute adapter detected"),
        Ok(mut names) => {
            names.sort();
            Inventory::detected(names.join(", "), "experimental Vulkan + CPU · transformer linear layers checked on first use")
        }
        Err(error) => Inventory::message(GpuState::Unavailable, format!("CPU · Vulkan adapter detection failed: {error:#}")),
    }
}

#[cfg(target_os = "macos")]
fn inspect() -> Inventory {
    use objc2_metal::{MTLCopyAllDevices, MTLDevice};
    let devices = MTLCopyAllDevices();
    let mut names: Vec<_> = devices.iter().map(|d| d.name().to_string()).collect();
    names.sort();
    if names.is_empty() {
        Inventory::message(GpuState::Unavailable, "CPU · no Metal adapter detected")
    } else {
        let mut identity = names.join(", ");
        // Apple ships Metal drivers with macOS. Even a point update can change
        // the driver without changing the adapter names.
        if let Some(build) = crate::stems::hardware::sysctl_string(c"kern.osversion") {
            identity.push_str(&format!(" · macOS build {build}"));
        }
        Inventory::detected(identity, "Metal device initialized on first use")
    }
}

#[cfg(all(feature = "cuda12", not(any(target_os = "macos", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan")))))]
fn inspect() -> Inventory {
    // CUDA can intentionally hide all devices. Other visibility masks can name
    // ordinals, UUIDs or partitions. The driver resolves those on first use.
    if std::env::var("CUDA_VISIBLE_DEVICES").is_ok_and(|s| matches!(s.trim(), "" | "-1")) {
        return Inventory::message(GpuState::Unavailable, "CPU · CUDA_VISIBLE_DEVICES hides all devices");
    }
    match cuda_inventory() {
        Ok(names) if !names.is_empty() => Inventory::detected(names.join(", "),
            "NVIDIA adapter detected · CUDA visibility, SM 6.1+ and runtime checked on first use"),
        // OS inventory can miss a headless/TCC or virtual compute adapter.
        _ => Inventory::message(GpuState::Unchecked, "CUDA 12 package · no NVIDIA adapter identified by the OS inventory; use --stems-device to check"),
    }
}

#[cfg(all(windows, feature = "cuda12", not(feature = "directml")))]
fn cuda_inventory() -> anyhow::Result<Vec<String>> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND};
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut names = Vec::new();
    for i in 0.. {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(i) } {
            Ok(adapter) => adapter,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(e.into()),
        };
        let desc = unsafe { adapter.GetDesc1()? };
        if desc.VendorId != 0x10de || desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 { continue; }
        let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        let mut id = String::from_utf16_lossy(&desc.Description[..end]);
        if let Ok(v) = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) } {
            let v = v as u64;
            id.push_str(&format!(" · driver {}.{}.{}.{}", v >> 48, (v >> 32) & 0xffff, (v >> 16) & 0xffff, v & 0xffff));
        }
        names.push(id);
    }
    names.sort();
    Ok(names)
}

#[cfg(all(target_os = "linux", feature = "cuda12", not(feature = "vulkan")))]
fn cuda_inventory() -> anyhow::Result<Vec<String>> {
    use std::fs::{read_dir, read_to_string};
    let mut names = Vec::new();
    if let Ok(devices) = read_dir("/proc/driver/nvidia/gpus") {
        for device in devices.flatten() {
            if let Ok(info) = read_to_string(device.path().join("information")) {
                if let Some(model) = info.lines().find_map(|l| l.strip_prefix("Model:")) {
                    names.push(model.trim().to_owned());
                }
            }
        }
    }
    if names.is_empty() {
        for device in read_dir("/sys/bus/pci/devices")?.flatten() {
            let entry = device.path();
            let attr = |s| read_to_string(entry.join(s)).ok().map(|s| s.trim().to_owned());
            if attr("vendor").as_deref() != Some("0x10de") || !attr("class").is_some_and(|c| c.starts_with("0x03")) { continue; }
            if let Some(pci) = attr("device") { names.push(format!("NVIDIA PCI {pci}")); }
        }
    }
    names.sort();
    if !names.is_empty() {
        if let Ok(version) = read_to_string("/sys/module/nvidia/version") {
            return Ok(vec![format!("{} · driver {}", names.join(", "), version.trim())]);
        }
    }
    Ok(names)
}

#[cfg(not(any(target_os = "macos", feature = "cuda12", all(windows, feature = "directml"), all(target_os = "linux", feature = "vulkan"))))]
fn inspect() -> Inventory { Inventory::message(GpuState::CpuOnly, "CPU · standard build") }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_and_preference_reads_leave_compute_unopened() {
        // Other application tests deliberately initialize/override the global
        // backend. Use fresh processes to exercise both startup orderings.
        const TEST: &str = "stems::backend::inventory::tests::setup_and_preference_reads_leave_compute_unopened";
        const CHILD: &str = "RECORDBOX_TEST_PASSIVE_STEMS_SETUP";
        let Ok(mode) = std::env::var(CHILD) else {
            for mode in ["inventory", "forced"] {
                let out = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", TEST]).env(CHILD, mode).output().unwrap();
                assert!(out.status.success(), "{mode}: {}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"), "child test did not run");
            }
            return;
        };
        assert!(BACKEND.get().is_none());
        if mode == "forced" { super::super::force_cpu(); }
        let hw = crate::stems::assess();
        let gpu = gpu_capability();
        #[cfg(target_os = "macos")]
        if gpu.state == GpuState::Detected {
            let build = crate::stems::hardware::sysctl_string(c"kern.osversion").unwrap();
            assert!(hw.gpu_identity.ends_with(&format!("macOS build {build}")));
        }
        assert_eq!(hw.gpu, gpu.candidate());
        assert!(!gpu.detail.is_empty());
        let model = crate::stems::choose(&hw);
        let _ = crate::stems::Live::preference_unavailable(&hw, model, crate::stems::SeparationDevice::Selected);
        assert!(crate::stems::status_report().contains(gpu.package));
        if mode == "forced" {
            assert!(forced_cpu());
            assert_eq!(gpu.state, GpuState::CpuOnly);
            assert!(PASSIVE.get().is_none(), "explicit CPU session enumerated the GPU");
        } else {
            assert!(BACKEND.get().is_none(), "setup, CLI status or Preferences initialized a compute device");
        }
    }
    #[test]
    fn only_detected_or_initialized_gpu_is_a_candidate() {
        for state in [GpuState::CpuOnly, GpuState::Unchecked, GpuState::Detected, GpuState::Ready, GpuState::Unavailable, GpuState::Paused] {
            let gpu = GpuCapability { state, package: "test", detail: "test" };
            assert_eq!(gpu.candidate(), matches!(state, GpuState::Detected | GpuState::Ready));
        }
    }
}
