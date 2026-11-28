//! CPU and usable system memory for passive setup. All memory quantities in
//! Hardware are decimal GB, matching the existing model's RAM thresholds.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Hardware {
    pub cpu: String,
    /// Logical CPUs available to this process, accounting for affinity/OS limits.
    pub cores: usize,
    pub cpu_count_known: bool,
    pub arch: String,
    pub simd: String,
    pub fma: bool,
    pub ram_gb: f64,
    pub apple_silicon: bool,
    /// An eligible adapter was detected or its compute device was initialized.
    pub gpu: bool,
    /// Stable passive inventory, independent of whether a job has initialized the backend.
    pub gpu_identity: String,
}

pub(super) fn system_hardware() -> Hardware {
    let (simd, fma) = cpu_features();
    let threads = std::thread::available_parallelism().ok();
    Hardware {
        cpu: cpu_name().unwrap_or_else(|| std::env::consts::ARCH.to_owned()),
        cores: threads.map(usize::from).unwrap_or(1),
        cpu_count_known: threads.is_some(),
        arch: std::env::consts::ARCH.to_owned(),
        simd, fma,
        ram_gb: memory_bytes().unwrap_or(0) as f64 / 1e9,
        apple_silicon: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        gpu: super::gpu_capability().candidate(),
        gpu_identity: super::backend::adapter_identity().to_owned(),
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn cpu_features() -> (String, bool) {
    // Rust checks OS register-save support as well as the x86 CPUID bits.
    let names = [
        ("SSE2", std::is_x86_feature_detected!("sse2")),
        ("SSE4.1", std::is_x86_feature_detected!("sse4.1")),
        ("AVX", std::is_x86_feature_detected!("avx")),
        ("AVX2", std::is_x86_feature_detected!("avx2")),
        ("AVX-512F", std::is_x86_feature_detected!("avx512f")),
    ];
    let features: Vec<_> = names.into_iter().filter_map(|(name, available)| available.then_some(name)).collect();
    (if features.is_empty() { "scalar".into() } else { features.join(", ") }, std::is_x86_feature_detected!("fma"))
}

#[cfg(target_arch = "aarch64")]
fn cpu_features() -> (String, bool) {
    let neon = std::arch::is_aarch64_feature_detected!("neon");
    (if neon { "NEON" } else { "scalar" }.into(), neon)
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
fn cpu_features() -> (String, bool) { ("portable".into(), false) }

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn cpu_name() -> Option<String> {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::__cpuid;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::__cpuid;
    if __cpuid(0x80000000).eax < 0x80000004 { return None; }
    let mut name = Vec::with_capacity(48);
    for leaf in 0x80000002..=0x80000004 {
        let registers = __cpuid(leaf);
        for word in [registers.eax, registers.ebx, registers.ecx, registers.edx] {
            name.extend(word.to_le_bytes());
        }
    }
    let name = std::str::from_utf8(&name).ok()?.trim_matches([' ', '\0']);
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(all(target_os = "macos", not(any(target_arch = "x86", target_arch = "x86_64"))))]
fn cpu_name() -> Option<String> {
    let mut bytes = [0u8; 256];
    let n = sysctl(c"machdep.cpu.brand_string", &mut bytes)?;
    let name = std::str::from_utf8(bytes.get(..n)?).ok()?.trim_matches([' ', '\0']);
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(all(not(target_os = "macos"), not(any(target_arch = "x86", target_arch = "x86_64"))))]
fn cpu_name() -> Option<String> { None }

#[cfg(target_os = "linux")]
fn memory_bytes() -> Option<u64> {
    let mem = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut bytes = mem.lines().find_map(|line| line.strip_prefix("MemTotal:")?
        .split_whitespace().next()?.parse::<u64>().ok())?.checked_mul(1024)?;
    // With cgroup v2 the root can be unrestricted while the process's child
    // group (or its parent slice) is capped. Traverse only normal components.
    let root = std::path::Path::new("/sys/fs/cgroup");
    let mut dir = root.to_owned();
    let groups = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if let Some(relative) = groups.lines().find_map(|s| s.strip_prefix("0::/")) {
        let path = std::path::Path::new(relative);
        if path.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
            dir.push(path);
        }
    }
    for parent in dir.ancestors().take_while(|p| p.starts_with(root)) {
        if let Some(limit) = std::fs::read_to_string(parent.join("memory.max")).ok()
            .and_then(|s| s.trim().parse::<u64>().ok()) {
            bytes = bytes.min(limit);
        }
    }
    Some(bytes)
}

#[cfg(windows)]
fn memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut mem = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    (unsafe { GlobalMemoryStatusEx(&mut mem) } != 0).then_some(mem.ullTotalPhys)
}

#[cfg(target_os = "macos")]
fn sysctl(key: &std::ffi::CStr, bytes: &mut [u8]) -> Option<usize> {
    let mut len = bytes.len();
    // The OS can write at most the supplied buffer length.
    let result = unsafe { libc::sysctlbyname(key.as_ptr(), bytes.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
    (result == 0 && len <= bytes.len()).then_some(len)
}

#[cfg(target_os = "macos")]
fn memory_bytes() -> Option<u64> {
    let mut bytes = [0u8; 8];
    (sysctl(c"hw.memsize", &mut bytes)? == bytes.len()).then(|| u64::from_ne_bytes(bytes))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn memory_bytes() -> Option<u64> { None }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_report_tracks_available_parallelism_and_runtime_fma() {
        let hw = system_hardware();
        assert!(!hw.cpu.is_empty() && !hw.simd.is_empty());
        assert_eq!(hw.arch, std::env::consts::ARCH);
        assert_eq!(hw.cores, std::thread::available_parallelism().map(usize::from).unwrap_or(1));
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        assert_eq!(hw.fma, std::is_x86_feature_detected!("fma"));
    }
}
