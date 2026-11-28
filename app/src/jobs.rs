//! Background work: analysis workers, file import, player loading and device export.

use eframe::egui;
use crate::analysis::{analyze_file_bounded, AnalysisResult, AnalysisSettings};
use crate::audio::{decode_until, Decoded};
use crate::model::Track;
use crossbeam_channel::{bounded, unbounded, Receiver, Sender, TryRecvError};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::VecDeque;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct AnalysisJob {
    pub track_id: i64,
    pub path: String,
    pub settings: AnalysisSettings,
    pub duration: f64,
    pub sample_rate: u32,
}

impl AnalysisJob {
    fn reservation(&self, budget: u64) -> u64 {
        if self.duration.is_finite() && self.duration > 0.0 {
            // Imported durations may be rounded; allow a little extra before
            // falling back to a second decode with the whole batch budget.
            self.settings.estimated_batch_bytes(self.duration * 1.05 + 2.0, self.sample_rate).min(budget)
        } else {
            budget
        }
    }
}

struct AnalysisQueue {
    jobs: VecDeque<AnalysisJob>,
    active: usize,
    reserved: u64,
    budget: u64,
    cancel: Arc<AtomicBool>,
}

type AnalysisOutcome = (i64, Result<AnalysisResult, String>);

struct Batch {
    workers: rayon::ThreadPool,
    queue: Mutex<AnalysisQueue>,
    results: Sender<AnalysisOutcome>,
    pending: Arc<AtomicUsize>,
    ctx: egui::Context,
}

impl Batch {
    fn schedule(self: &Arc<Self>) {
        // Only start work that fits. Never block a Rayon thread waiting for a
        // reservation: that thread may be needed for a running track's DSP.
        let mut q = self.queue.lock();
        while q.active < self.workers.current_num_threads() {
            let Some(job) = q.jobs.front() else { break };
            let bytes = job.reservation(q.budget);
            if bytes > q.budget - q.reserved { break; }
            let job = q.jobs.pop_front().unwrap();
            q.active += 1;
            q.reserved += bytes;
            let cancel = Arc::clone(&q.cancel);
            let batch = Arc::clone(self);
            self.workers.spawn_fifo(move || batch.run(job, bytes, cancel));
        }
    }

    fn run(self: &Arc<Self>, mut job: AnalysisJob, bytes: u64, cancel: Arc<AtomicBool>) {
        let stopped = || cancel.load(Ordering::Relaxed);
        let result = std::panic::catch_unwind(||
            analyze_file_bounded(&job.path, &job.settings, bytes, true, stopped)
        ).unwrap_or_else(|_| Err(anyhow::anyhow!("track analysis panicked")));

        if result.as_ref().is_err_and(|e| e.is::<crate::analysis::AnalysisMemoryLimit>()) {
            let mut q = self.queue.lock();
            if !stopped() && bytes < q.budget {
                // The actual track outgrew its library hint. Retry once alone;
                // the decoder already dropped its PCM on returning the error.
                job.duration = 0.0;
                q.jobs.push_front(job);
                q.reserved -= bytes;
                q.active -= 1;
                drop(q);
                self.schedule();
                return;
            }
        }

        let outcome = if stopped() { Err("stopped".into()) } else { result.map_err(|error| error.to_string()) };
        // Retain the reservation until the bounded UI channel accepts the result.
        // This also bounds completed waveforms when the UI isn't draining them.
        self.ctx.request_repaint();
        let _ = self.results.send((job.track_id, outcome));
        {
            let mut q = self.queue.lock();
            q.active -= 1;
            q.reserved -= bytes;
            self.pending.fetch_sub(1, Ordering::Release);
        }
        self.ctx.request_repaint();
        self.schedule();
    }
}

pub struct AnalysisPool {
    batch: Arc<Batch>,
    results: Receiver<AnalysisOutcome>,
    stopped: Mutex<VecDeque<i64>>,
    pub pending: Arc<AtomicUsize>,
    pub available_cpus: usize,
    pub(crate) automatic_limit: usize,
    desired_workers: usize,
}

fn analysis_threads(requested: usize, available: usize) -> usize {
    let limit = available.clamp(1, 12);
    if requested == 0 { available.saturating_sub(1).clamp(1, limit) } else { requested.min(limit) }
}

/// Size Automatic for a typical four-minute, 48 kHz analysis job. Actual jobs still
/// reserve their measured duration/rate and share the batch's live memory ceiling.
pub(crate) fn automatic_workers(cpus: usize, analysis_bytes: u64) -> usize {
    let per_track = crate::analysis::estimated_working_bytes(240.0, 48_000);
    let memory_limit = usize::try_from(analysis_bytes / per_track).unwrap_or(usize::MAX).max(1);
    analysis_threads(0, cpus).min(memory_limit)
}

fn build_workers(count: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(count).thread_name(|i| format!("rb-analysis-{i}"))
        .build().expect("start analysis workers")
}

#[cfg(windows)]
fn batch_memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut mem = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // Windows can exhaust its commit limit even with physical RAM available.
    (unsafe { GlobalMemoryStatusEx(&mut mem) } != 0).then(|| mem.ullAvailPhys.min(mem.ullAvailPageFile) / 2)
}

#[cfg(target_os = "linux")]
fn batch_memory_bytes() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let available = info.lines().find_map(|line| line.strip_prefix("MemAvailable:")?
        .split_whitespace().next()?.parse::<u64>().ok())?.saturating_mul(1024);
    let cgroup = std::path::Path::new("/sys/fs/cgroup");
    let read = |name| std::fs::read_to_string(cgroup.join(name)).ok()?.trim().parse::<u64>().ok();
    let available = match (read("memory.max"), read("memory.current")) {
        (Some(limit), Some(used)) => available.min(limit.saturating_sub(used)),
        _ => available,
    };
    Some(available / 2)
}

#[cfg(target_os = "macos")]
fn batch_memory_bytes() -> Option<u64> {
    let mut bytes = 0u64;
    let mut len = std::mem::size_of::<u64>();
    // Darwin counts reclaimable memory differently. Reserve a conservative
    // share of installed memory for batch analysis.
    let ok = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&mut bytes as *mut u64).cast(),
        &mut len, std::ptr::null_mut(), 0) };
    (ok == 0 && len == std::mem::size_of::<u64>()).then_some(bytes / 4)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn batch_memory_bytes() -> Option<u64> { None }

/// Share the same available-memory policy between library jobs and USB preparation.
pub(crate) fn analysis_memory_budget() -> u64 {
    batch_memory_bytes().unwrap_or(512 * 1024 * 1024)
}

impl AnalysisPool {
    pub fn new(ctx: egui::Context, requested: usize) -> AnalysisPool {
        let available = std::thread::available_parallelism().map(usize::from).unwrap_or(1);
        let automatic_limit = automatic_workers(available, analysis_memory_budget());
        let count = analysis_threads(if requested == 0 { automatic_limit } else { requested }, available);
        // At most two finished results wait in this channel. Lightweight IDs
        // cancelled on the UI thread travel separately so Stop never blocks.
        let (tx, results) = bounded(2);
        let pending = Arc::<AtomicUsize>::default();
        let batch = Arc::new(Batch {
            workers: build_workers(count),
            queue: Mutex::new(AnalysisQueue { jobs: VecDeque::new(), active: 0, reserved: 0,
                budget: 0, cancel: Arc::default() }),
            results: tx, pending: pending.clone(), ctx,
        });
        AnalysisPool { batch, results, stopped: Default::default(), pending, available_cpus: available, automatic_limit, desired_workers: count }
    }

    pub fn thread_count(&self) -> usize {
        self.batch.workers.current_num_threads()
    }

    pub fn set_threads(&mut self, requested: usize) {
        self.desired_workers = analysis_threads(if requested == 0 { self.automatic_limit } else { requested }, self.available_cpus);
        self.apply_thread_change();
    }

    pub(crate) fn configure_automatic(&mut self, limit: usize, requested: usize) {
        self.automatic_limit = limit.clamp(1, analysis_threads(0, self.available_cpus));
        self.set_threads(requested);
    }

    pub(crate) fn apply_thread_change(&mut self) {
        let count = self.desired_workers;
        if count != self.thread_count() && self.pending.load(Ordering::Acquire) == 0 {
            // A just-completed worker may still be exiting. Keep its pool until
            // the last task drops it; preserve results already sent to the UI.
            self.batch = Arc::new(Batch {
                workers: build_workers(count), results: self.batch.results.clone(),
                pending: self.pending.clone(), ctx: self.batch.ctx.clone(),
                queue: Mutex::new(AnalysisQueue { jobs: VecDeque::new(), active: 0, reserved: 0,
                    budget: 0, cancel: Arc::default() }),
            });
        }
    }

    pub fn submit(&self, job: AnalysisJob) {
        self.submit_with_budget(job, analysis_memory_budget);
    }

    fn submit_with_budget(&self, job: AnalysisJob, memory_budget: impl FnOnce() -> u64) {
        {
            let mut q = self.batch.queue.lock();
            if self.pending.fetch_add(1, Ordering::Relaxed) == 0 {
                q.budget = memory_budget();
            }
            q.jobs.push_back(job);
        }
        self.batch.schedule();
    }

    pub fn try_recv(&self) -> Result<AnalysisOutcome, TryRecvError> {
        self.results.try_recv().or_else(|error| {
            self.stopped.lock().pop_front().map(|id| (id, Err("stopped".into()))).ok_or(error)
        })
    }

    pub fn stop(&self) {
        let mut q = self.batch.queue.lock();
        q.cancel.store(true, Ordering::Relaxed);
        q.cancel = Arc::default();
        let waiting = std::mem::take(&mut q.jobs);
        self.pending.fetch_sub(waiting.len(), Ordering::Release);
        drop(q);
        self.stopped.lock().extend(waiting.into_iter().map(|j| j.track_id));
        self.batch.ctx.request_repaint();
    }
}

impl Drop for AnalysisPool {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn_import(paths: Vec<std::path::PathBuf>, ctx: egui::Context) -> Receiver<Option<Track>> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let files = crate::importer::collect_audio(&paths);
        for f in files {
            let t = crate::importer::read_track(&f);
            if tx.send(Some(t)).is_err() {
                return;
            }
            ctx.request_repaint();
        }
        let _ = tx.send(None);
        ctx.request_repaint();
    });
    rx
}

pub fn spawn_load(track_id: i64, path: String, ctx: egui::Context, cancel: Arc<AtomicBool>) -> Receiver<(i64, Result<Arc<Decoded>, String>)> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let r = decode_until(&path, || cancel.load(Ordering::Relaxed)).map(Arc::new).map_err(|e| e.to_string());
        let _ = tx.send((track_id, r));
        ctx.request_repaint();
    });
    rx
}

pub struct ExportJob {
    pub rx: Receiver<crate::export::usb::Progress>,
    pub cancel: Arc<AtomicBool>,
    pub last: (usize, usize, String),
}

pub fn spawn_export(req: crate::export::usb::ExportRequest, ctx: egui::Context) -> ExportJob {
    let (tx, rx) = unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        let progress = move |p: crate::export::usb::Progress| {
            let _ = tx2.send(p);
            ctx2.request_repaint();
        };
        // A panic anywhere in the export is reported as a failed export, not a hung window.
        let r = crate::export::usb::run_guarded(|| crate::export::usb::run_export(req, &progress, &c2));
        let _ = tx.send(crate::export::usb::Progress::Finished(r));
        ctx.request_repaint();
    });
    ExportJob { rx, cancel, last: (0, 0, String::new()) }
}

#[cfg(test)]
mod throughput_probe {
    /// Opt-in: PREPDECK_PATHS=<file with one audio path per line> PREPDECK_N=<workers>.
    /// Add PREPDECK_PRECISE=1 to measure high-precision grid analysis on the same files.
    /// Prints how long analysing those files takes with that many workers.
    #[test]
    #[ignore]
    fn analysis_workers() {
        let paths: Vec<String> = std::fs::read_to_string(std::env::var("PREPDECK_PATHS").unwrap()).unwrap().lines().map(|s| s.to_string()).collect();
        let n = std::env::var("PREPDECK_N").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let high_precision = std::env::var("PREPDECK_PRECISE").as_deref() == Ok("1");
        let settings = super::AnalysisSettings { high_precision, ..Default::default() };
        let pool = super::AnalysisPool::new(eframe::egui::Context::default(), n);
        let jobs: Vec<_> = paths.iter().enumerate().map(|(i, path)| {
            let track = crate::importer::read_track(std::path::Path::new(path));
            super::AnalysisJob { track_id: i as i64, path: track.path, settings: settings.clone(),
                duration: track.duration, sample_rate: track.sample_rate }
        }).collect();
        let start = std::time::Instant::now();
        for job in jobs { pool.submit(job); }
        for _ in &paths { pool.results.recv().unwrap().1.unwrap(); }
        eprintln!("{} files, {} workers: {:.3} s", paths.len(), pool.thread_count(), start.elapsed().as_secs_f64());
    }
}

#[cfg(test)]
mod analysis_pool_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn worker_budget_reserves_a_processor_and_caps_live_decodes() {
        for (cpus, automatic) in [(1, 1), (2, 1), (4, 3), (8, 7), (13, 12), (64, 12)] {
            assert_eq!(analysis_threads(0, cpus), automatic);
            assert_eq!(analysis_threads(1, cpus), 1);
            assert_eq!(analysis_threads(99, cpus), cpus.min(12));
        }
    }

    #[test]
    fn stopping_queued_work_does_not_cancel_the_next_submission() {
        let ctx = egui::Context::default();
        let pool = AnalysisPool::new(ctx, 1);
        let (entered, wait_entered) = crossbeam_channel::bounded(0);
        let (release, wait_release) = crossbeam_channel::bounded(0);
        pool.batch.workers.spawn(move || { entered.send(()).unwrap(); wait_release.recv().unwrap(); });
        wait_entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let job = |track_id| AnalysisJob { track_id, path: "/missing/synthetic.wav".into(), settings: Default::default(), duration: 1.0, sample_rate: 44100 };
        for id in 1..=3 { pool.submit(job(id)); }
        pool.stop();
        pool.submit(job(4));
        release.send(()).unwrap();
        let mut finished = Vec::new();
        for _ in 0..4 {
            let (id, result) = pool.try_recv().or_else(|_| pool.results.recv_timeout(Duration::from_secs(10))).unwrap();
            assert_eq!(result.err().unwrap() == "stopped", id < 4);
            finished.push(id);
        }
        finished.sort_unstable();
        assert_eq!(finished, [1, 2, 3, 4]);
    }

    #[test]
    fn batch_budget_bounds_active_tracks_independently_of_dsp_threads() {
        let mut pool = AnalysisPool::new(egui::Context::default(), 1);
        pool.available_cpus = 3;
        pool.set_threads(3);
        let (entered, wait_entered) = bounded(3);
        let (release, wait_release) = bounded(3);
        for _ in 0..3 {
            let entered = entered.clone();
            let wait = wait_release.clone();
            pool.batch.workers.spawn(move || { entered.send(()).unwrap(); wait.recv().unwrap(); });
        }
        for _ in 0..3 { wait_entered.recv_timeout(Duration::from_secs(10)).unwrap(); }
        let job = |id, duration| AnalysisJob { track_id: id, path: "/missing/fixture.wav".into(), duration,
            sample_rate: 44100, settings: Default::default() };
        let bytes = job(0, 60.0).reservation(u64::MAX);
        let budget = 2 * bytes;
        for id in 1..=3 { pool.submit_with_budget(job(id, 60.0), || budget); }
        pool.submit_with_budget(job(4, 0.0), || budget);
        {
            let q = pool.batch.queue.lock();
            assert_eq!((pool.thread_count(), q.active, q.reserved, q.jobs.len()), (3, 2, budget, 2));
            assert_eq!(q.jobs.back().unwrap().reservation(budget), budget);
        }
        for _ in 0..3 { release.send(()).unwrap(); }
        let mut ids = Vec::new();
        for _ in 0..4 { ids.push(pool.results.recv_timeout(Duration::from_secs(10)).unwrap().0); }
        ids.sort_unstable();
        assert_eq!(ids, [1, 2, 3, 4]);
    }

    #[test]
    fn dropping_a_pool_with_a_full_result_channel_retires_pending_jobs() {
        let pool = AnalysisPool::new(egui::Context::default(), 1);
        let pending = pool.pending.clone();
        for id in 0..10 {
            pool.submit_with_budget(AnalysisJob { track_id: id, path: "/missing/synthetic.wav".into(),
                settings: Default::default(), duration: 1.0, sample_rate: 44100 }, || 0);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !pool.results.is_full() && std::time::Instant::now() < deadline { std::thread::sleep(Duration::from_millis(1)); }
        assert!(pool.results.is_full());
        // Even when the operating system reported zero available memory, the
        // next job gets an actionable error, without opening its missing path.
        assert!(pool.results.recv().unwrap().1.err().unwrap().contains("analysis memory budget"));
        while !pool.results.is_full() && std::time::Instant::now() < deadline { std::thread::sleep(Duration::from_millis(1)); }
        assert!(pool.results.is_full());
        let (done, waited) = bounded(1);
        std::thread::spawn(move || { drop(pool); done.send(()).unwrap(); });
        waited.recv_timeout(Duration::from_secs(10)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while pending.load(Ordering::Acquire) != 0 && std::time::Instant::now() < deadline { std::thread::sleep(Duration::from_millis(1)); }
        assert_eq!(pending.load(Ordering::Acquire), 0);
    }

    #[test]
    fn underestimated_duration_retries_once_and_oversize_tracks_report_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("four_seconds.wav");
        let rate = 8000;
        crate::demo::write_wav(&path, rate, &vec![0.125; 4 * rate as usize * 2]).unwrap();
        let job = |id| AnalysisJob { track_id: id, path: path.to_string_lossy().into_owned(),
            duration: 0.1, sample_rate: rate,
            settings: AnalysisSettings { do_key: false, do_grid: false, ..Default::default() } };
        let pool = AnalysisPool::new(egui::Context::default(), 1);
        let budget = crate::analysis::estimated_working_bytes(4.0, rate);
        assert!(job(1).reservation(budget) < budget);
        pool.submit_with_budget(job(1), || budget);
        let (id, result) = pool.results.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!((id, result.unwrap().raw_samples), (1, 32_000));
        // A separate batch with less than four seconds of room must report a
        // failed whole track; no waveform, BPM or key is derived from a prefix.
        let pool = AnalysisPool::new(egui::Context::default(), 1);
        pool.submit_with_budget(job(2), || budget - 1);
        assert!(pool.results.recv_timeout(Duration::from_secs(10)).unwrap().1.err().unwrap().contains("analysis memory budget"));
    }

    #[test]
    fn parallel_analysis_matches_serial_for_normal_dynamic_and_disabled_passes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pulse.wav");
        let sr = 22050;
        let samples: Vec<f32> = (0..sr * 10).flat_map(|i| {
            let beat = if i % (sr / 2) < 200 { 0.55 } else { 0.03 };
            let v = beat * (std::f32::consts::TAU * 220.0 * i as f32 / sr as f32).sin();
            [v, v * 0.8]
        }).collect();
        crate::demo::write_wav(&path, sr, &samples).unwrap();
        let path = path.to_str().unwrap();
        let workers = build_workers(3);
        for (dynamic, high_precision, do_grid, do_key) in [
            (false, false, true, true), (true, false, true, true),
            (false, true, true, true), (true, true, true, true), (false, true, false, false),
        ] {
            let settings = AnalysisSettings { dynamic, high_precision, do_grid, do_key, ..Default::default() };
            let a = crate::analysis::analyze_file(path, &settings).unwrap();
            let b = workers.install(|| analyze_file_bounded(path, &settings, 512 << 20, true, || false)).unwrap();
            assert_eq!((a.duration.to_bits(), a.raw_samples, a.sample_rate, a.bpm.to_bits(), a.key),
                       (b.duration.to_bits(), b.raw_samples, b.sample_rate, b.bpm.to_bits(), b.key));
            assert_eq!(serde_json::to_value(a.grid).unwrap(), serde_json::to_value(b.grid).unwrap());
            assert_eq!(a.wave.frames, b.wave.frames);
            assert_eq!((a.gain_db.to_bits(), a.peak.to_bits()), (b.gain_db.to_bits(), b.peak.to_bits()));
        }
    }
}
