//! Local GUI setup. A release/build profile records automatic choices only. The
//! current session always rechecks device eligibility before it can dispatch work.

use crate::{analysis::rb_hp, db::Db, jobs, stems};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extensions {
    normal_grid: bool,
    dynamic_grid: bool,
    speed: bool,
    quality: bool,
}

impl Extensions {
    pub(super) fn inspect() -> Self {
        let mut files = Self {
            normal_grid: rb_hp::models_present(rb_hp::MODE_NORMAL),
            dynamic_grid: rb_hp::models_present(rb_hp::MODE_DYNAMIC),
            ..Default::default()
        };
        files.refresh_checkpoints();
        files
    }

    pub(super) fn refresh_checkpoints(&mut self) {
        let [quality, speed] = &stems::CATALOG;
        self.speed = stems::model_ready(speed);
        self.quality = stems::model_ready(quality);
    }

    fn summary(&self) -> String {
        let present = |found| if found { "present" } else { "missing" };
        format!("Local model files · Normal beat grid {} · Dynamic beat grid {} · STEMS speed {} · STEMS quality {}.",
            present(self.normal_grid), present(self.dynamic_grid), present(self.speed), present(self.quality))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Identity {
    version: String,
    package: String,
    os: String,
    arch: String,
    cpu: String,
    cores: usize,
    ram_mib: u64,
    simd: String,
    fma: bool,
    adapter: String,
}

impl Identity {
    fn for_session(hw: &stems::Hardware) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            package: stems::gpu_capability().package.into(),
            os: std::env::consts::OS.into(), arch: hw.arch.clone(),
            cpu: hw.cpu.clone(), cores: if hw.cpu_count_known { hw.cores } else { 0 },
            ram_mib: (hw.ram_gb * 1e9 / 1_048_576.0).round() as u64,
            simd: hw.simd.clone(), fma: hw.fma, adapter: hw.gpu_identity.clone(),
        }
    }

    fn auto_workers(&self) -> usize {
        // Like the macOS batch policy, plan around a quarter of installed (or
        // cgroup-limited) RAM. Per-batch admission independently samples pressure.
        jobs::automatic_workers(self.cores, self.ram_mib.saturating_mul(1_048_576) / 4)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    hardware: Identity,
    automatic_workers: usize,
}

#[derive(Default)]
pub(super) struct Setup {
    status: Option<String>,
    pub(super) extensions: Option<Extensions>,
}

impl Setup {
    fn configure(&mut self, db: &Db, identity: Identity, save: bool) -> usize {
        if identity.ram_mib == 0 || identity.cores == 0 {
            self.status = Some("Memory or CPU count unavailable. Automatic uses one worker and setup will retry at next launch.".into());
            return 1;
        }
        if !save {
            self.status = Some("Using session-only hardware settings.".into());
            return identity.auto_workers();
        }

        // Keep independent profiles when switching among packages of the same release.
        let key = format!("hardware_setup_v1_{}_{}_{}", identity.os, identity.arch, identity.package);
        let sync = || -> anyhow::Result<(usize, bool)> {
            if let Some(record) = db.setting_checked(&key)?.and_then(|s| serde_json::from_str::<Record>(&s).ok()) {
                if record.hardware == identity && record.automatic_workers == identity.auto_workers() {
                    return Ok((record.automatic_workers, false));
                }
            }
            let workers = identity.auto_workers();
            let record = Record { hardware: identity.clone(), automatic_workers: workers };
            db.set_setting_checked(&key, &serde_json::to_string(&record)?)?;
            Ok((workers, true))
        };
        match sync() {
            Ok((workers, updated)) => {
                self.status = Some(format!("Hardware setup for recordbox {} {}.", identity.version,
                    if updated { "saved" } else { "loaded" }));
                workers
            }
            Err(error) => {
                self.status = Some(format!("Hardware setup could not be saved. Using session settings and retrying at next launch. {error}"));
                identity.auto_workers()
            }
        }
    }
}

/// Keep a selected model available for cache reads even if this package cannot run
/// new separation jobs with it. An explicitly requested CPU retry stays available.
pub(super) fn separation_limit(hw: &stems::Hardware, model: &stems::ModelSpec, state: stems::GpuState,
    execution: stems::SeparationDevice) -> Option<&'static str> {
    if hw.ram_gb.is_finite() && hw.ram_gb > 0.0 && hw.ram_gb < model.min_ram_gb {
        return Some("This model needs more system memory. Previously saved stems are available.");
    }
    if model.needs_gpu && execution != stems::SeparationDevice::Cpu &&
        matches!(state, stems::GpuState::CpuOnly | stems::GpuState::Unavailable | stems::GpuState::Paused) {
        return Some("The saved quality model needs an available GPU for this job. Choose Automatic or Prioritize speed in STEMS preferences. Previously saved quality stems are available.");
    }
    None
}

fn automatic_model(hw: &stems::Hardware, files: Extensions, gpu_eligible: bool) -> &'static stems::ModelSpec {
    let quality = &stems::CATALOG[0];
    if !files.speed && files.quality && gpu_eligible && stems::runs_on(quality, hw) {
        quality
    } else {
        stems::choose(hw)
    }
}

pub(super) fn startup_model(automatic: &'static stems::ModelSpec, active: Option<&str>, session: Option<&str>, saved: Option<&str>)
    -> &'static stems::ModelSpec {
    [active, session, saved].into_iter().flatten().find_map(stems::spec).unwrap_or(automatic)
}

impl super::App {
    pub(crate) fn restored_hardware_preferences(&mut self) {
        self.pool.set_threads(self.prefs.analysis_threads);
        self.stem.model = None;
        if let (Some(hw), Some(files)) = (self.stem.hw.clone(), self.hardware_setup.extensions) {
            // A backup may have brought another machine's setup record and choices.
            // Reuse the verified local session inventory, never the backup's devices.
            self.apply_hardware_setup(&hw, files);
            let session = std::env::var("PREPDECK_STEM_MODEL").ok();
            let model = startup_model(self.automatic_stem_model(&hw), None, session.as_deref(), self.prefs.stem_model.as_deref());
            self.stem.model = Some(model.id.to_owned());
            self.stem.model_ready = stems::model_ready(model);
        }
    }

    pub(super) fn apply_hardware_setup(&mut self, hw: &stems::Hardware, files: Extensions) {
        let workers = self.hardware_setup.configure(
            &self.db, Identity::for_session(hw), self.screenshot.is_none() && !stems::forced_cpu()
        );
        self.hardware_setup.extensions = Some(files);
        self.pool.configure_automatic(workers, self.prefs.analysis_threads);
    }

    pub(crate) fn hardware_setup_summary(&self) -> String {
        let status = self.hardware_setup.status.as_deref().unwrap_or("Checking this computer…");
        format!("{status} Automatic analysis limit: {} worker(s).", self.pool.automatic_limit)
    }

    pub(crate) fn automatic_stem_model(&self, hw: &stems::Hardware) -> &'static stems::ModelSpec {
        automatic_model(hw, self.hardware_setup.extensions.unwrap_or_default(), stems::gpu_capability().candidate())
    }

    pub(crate) fn local_grid_available(&self, dynamic: bool) -> Option<bool> {
        self.hardware_setup.extensions.map(|files| if dynamic { files.dynamic_grid } else { files.normal_grid })
    }

    pub(crate) fn stem_weights_present(&self, id: &str) -> Option<bool> {
        let files = self.hardware_setup.extensions?;
        match id {
            "htdemucs" => Some(files.speed),
            "htdemucs_ft" => Some(files.quality),
            _ => None,
        }
    }

    pub(crate) fn hardware_extensions_summary(&self) -> String {
        self.hardware_setup.extensions.map(|e| e.summary()).unwrap_or_else(|| "Checking local model files…".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Db, Identity) {
        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        let machine = Identity {
            version: "0.9.2".into(), package: "Vulkan".into(), os: "Linux".into(), arch: "x86_64".into(),
            cpu: "fixture".into(), cores: 24, ram_mib: 8192, simd: "AVX2".into(), fma: true, adapter: "GPU / driver 1".into(),
        };
        (db, machine)
    }

    #[test]
    fn release_and_device_changes_refresh_only_automatic_choices() {
        let (db, mut machine) = fixture();
        let prefs = r#"{"analysis_threads":2,"stem_model":"htdemucs_ft","stems_auto_separate":false}"#;
        db.set_setting_checked("prefs", prefs).unwrap();
        let mut setup = Setup::default();
        let key = "hardware_setup_v1_Linux_x86_64_Vulkan";
        assert_eq!(setup.configure(&db, machine.clone(), true), 3);
        let original = db.setting(key).unwrap();
        // A read-only connection proves an identical launch needs no write.
        db.conn.execute_batch("PRAGMA query_only=ON").unwrap();
        assert_eq!(setup.configure(&db, machine.clone(), true), 3);
        assert!(setup.status.as_ref().unwrap().ends_with("loaded."));
        db.conn.execute_batch("PRAGMA query_only=OFF").unwrap();
        machine.version = "0.10.0".into();
        setup.configure(&db, machine.clone(), true);
        assert_ne!(db.setting(key).unwrap(), original);
        machine.adapter.push('2');
        setup.configure(&db, machine.clone(), true);
        assert_eq!(serde_json::from_str::<Record>(&db.setting(key).unwrap()).unwrap().hardware, machine);
        machine.package = "CPU".into();
        machine.ram_mib = 2048;
        assert_eq!(setup.configure(&db, machine, true), 1);
        assert!(db.setting("hardware_setup_v1_Linux_x86_64_CPU").is_some());
        assert_eq!(db.setting("prefs").as_deref(), Some(prefs));
    }

    #[test]
    fn old_manual_model_and_session_overrides_retain_their_scope() {
        let hw = stems::Hardware { ram_gb: 20.0, ..Default::default() };
        let manual: super::super::Prefs = serde_json::from_str(r#"{"stem_model":"htdemucs_ft"}"#).unwrap();
        let saved = manual.stem_model.as_deref();
        let speed = stems::choose(&hw);
        assert_eq!(startup_model(speed, None, None, saved).id, "htdemucs_ft");
        assert_eq!(startup_model(speed, None, Some("htdemucs"), saved).id, "htdemucs");
        assert_eq!(startup_model(speed, None, Some("invalid"), saved).id, "htdemucs_ft");
        assert_eq!(startup_model(speed, Some("htdemucs_ft"), Some("htdemucs"), saved).id, "htdemucs_ft");
        let automatic: super::super::Prefs = serde_json::from_str("{}").unwrap();
        assert!(automatic.stem_model.is_none());
        assert_eq!(startup_model(speed, None, None, automatic.stem_model.as_deref()).id, "htdemucs");

        let quality = stems::spec("htdemucs_ft").unwrap();
        let selected = stems::SeparationDevice::Selected;
        assert!(separation_limit(&hw, quality, stems::GpuState::CpuOnly, selected).is_some());
        assert!(separation_limit(&hw, quality, stems::GpuState::Unchecked, selected).is_none());
        assert!(separation_limit(&hw, quality, stems::GpuState::Paused, stems::SeparationDevice::Cpu).is_none());
        let low = stems::Hardware { ram_gb: 8.0, ..hw };
        assert!(separation_limit(&low, quality, stems::GpuState::Ready, selected).is_some());
    }

    #[test]
    fn automatic_uses_eligible_installed_quality_only_when_speed_is_absent() {
        let hw = stems::Hardware { gpu: true, ram_gb: 16.0, ..Default::default() };
        let files = Extensions { quality: true, ..Default::default() };
        assert_eq!(automatic_model(&hw, files, true).id, "htdemucs_ft");
        assert_eq!(automatic_model(&hw, Extensions { speed: true, ..files }, true).id, "htdemucs");
        assert_eq!(automatic_model(&hw, Extensions::default(), true).id, "htdemucs");
        for candidate in [
            stems::Hardware { gpu: false, ..hw.clone() },
            stems::Hardware { ram_gb: 8.0, ..hw.clone() },
            stems::Hardware { ram_gb: 0.0, ..hw.clone() }
        ] {
            assert_eq!(automatic_model(&candidate, files, true).id, "htdemucs");
        }
        assert_eq!(automatic_model(&hw, files, false).id, "htdemucs");
    }

    #[test]
    fn incomplete_session_only_and_failed_writes_retry_without_replacing_a_good_profile() {
        let (db, machine) = fixture();
        let mut setup = Setup::default();
        let key = "hardware_setup_v1_Linux_x86_64_Vulkan";
        setup.configure(&db, machine.clone(), false);
        assert!(db.setting(key).is_none());
        setup.configure(&db, machine.clone(), true);
        let original = db.setting(key).unwrap();
        let mut unknown = machine.clone();
        unknown.ram_mib = 0;
        assert_eq!(setup.configure(&db, unknown, true), 1);
        assert_eq!(db.setting(key).unwrap(), original);
        let mut next = machine.clone();
        next.version.push('1');
        db.conn.execute_batch("PRAGMA query_only=ON").unwrap();
        assert_eq!(setup.configure(&db, next.clone(), true), 3);
        assert!(setup.status.as_ref().unwrap().contains("could not be saved"));
        assert_eq!(db.setting(key).unwrap(), original);
        db.conn.execute_batch("PRAGMA query_only=OFF").unwrap();
        db.set_setting_checked(key, "{truncated").unwrap();
        setup.configure(&db, next.clone(), true);
        assert_eq!(serde_json::from_str::<Record>(&db.setting(key).unwrap()).unwrap().hardware, next);
    }
}
