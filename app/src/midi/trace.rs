//! Opt-in tracing of mapped channel messages and controller connections.
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const FILE_LIMIT: u64 = 5 * 1024 * 1024;
const CAPACITY: usize = 1024;
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

enum Entry {
    Line(SystemTime, String),
    Finish(Sender<()>),
}

struct Trace {
    path: PathBuf,
    tx: Sender<Entry>,
    dropped: Arc<AtomicU64>,
    active: Arc<AtomicBool>,
}

static TRACE: OnceLock<Option<Trace>> = OnceLock::new();

pub(super) fn init() {
    TRACE.get_or_init(|| {
        let enabled = std::env::var_os("RECORDBOX_MIDI_LOG")
            .or_else(|| std::env::var_os("PREPDECK_MIDI_LOG"));
        if enabled.as_deref() != Some(std::ffi::OsStr::new("1")) {
            return None;
        }
        let path = crate::db::data_dir().join("midi.log");
        match start(path.clone()) {
            Ok(trace) => Some(trace),
            Err(error) => {
                eprintln!("Cannot open MIDI trace {}: {error}", path.display());
                None
            }
        }
    });
}

pub(crate) fn path() -> Option<&'static Path> {
    TRACE
        .get()?
        .as_ref()
        .filter(|trace| trace.active.load(Relaxed))
        .map(|trace| trace.path.as_path())
}

pub(super) fn record(line: impl FnOnce() -> String) {
    let Some(Some(trace)) = TRACE.get() else {
        return;
    };
    if !trace.active.load(Relaxed) {
        return;
    }
    if trace.tx.is_full()
        || trace
            .tx
            .try_send(Entry::Line(SystemTime::now(), line()))
            .is_err()
    {
        trace.dropped.fetch_add(1, Relaxed);
    }
}

/// Drain the last packets on normal exit, within a fixed time if the filesystem stalls.
pub(crate) fn finish() {
    if let Some(Some(trace)) = TRACE.get() {
        trace.finish();
    }
}

impl Trace {
    fn finish(&self) {
        if !self.active.swap(false, Relaxed) {
            return;
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        let (done, wait) = bounded(1);
        if self.tx.send_deadline(Entry::Finish(done), deadline).is_ok() {
            let _ = wait.recv_deadline(deadline);
        }
    }
}

fn start(path: PathBuf) -> io::Result<Trace> {
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let (tx, rx) = bounded(CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    let active = Arc::new(AtomicBool::new(true));
    let worker_active = active.clone();
    let (worker_path, worker_dropped) = (path.clone(), dropped.clone());
    std::thread::Builder::new()
        .name("midi-trace".into())
        .spawn(move || {
            if let Err(error) = write_session(&worker_path, file, rx, &worker_dropped, FILE_LIMIT) {
                eprintln!("MIDI trace stopped: {error}");
            }
            worker_active.store(false, Relaxed);
        })?;
    Ok(Trace {
        path,
        tx,
        dropped,
        active,
    })
}

fn write_session(
    path: &Path,
    file: File,
    rx: Receiver<Entry>,
    dropped: &AtomicU64,
    limit: u64,
) -> io::Result<()> {
    let mut size = file.metadata()?.len();
    let mut file = BufWriter::new(file);
    let mut flush_at = Instant::now() + FLUSH_INTERVAL;
    let mut entry = Entry::Line(
        SystemTime::now(),
        format!("SESSION recordbox {}", env!("CARGO_PKG_VERSION")),
    );
    loop {
        let (at, text) = match entry {
            Entry::Line(at, text) => (at, text),
            Entry::Finish(done) => {
                file.flush()?;
                let _ = done.send(());
                return Ok(());
            }
        };
        let time = chrono::DateTime::<chrono::Local>::from(at);
        let mut line = format!(
            "{} {}\n",
            time.format("%Y-%m-%d %H:%M:%S%.3f"),
            text.replace(['\n', '\r'], " ")
        );
        let missed = dropped.swap(0, Relaxed);
        if missed > 0 {
            line.push_str(&format!(
                "Dropped {missed} trace entries while the queue was full\n"
            ));
        }
        if size > 0 && size.saturating_add(line.len() as u64) > limit {
            file.flush()?;
            drop(file);
            std::fs::rename(path, path.with_file_name("midi.log.1"))?;
            file = BufWriter::new(OpenOptions::new().create(true).append(true).open(path)?);
            size = 0;
        }
        file.write_all(line.as_bytes())?;
        size += line.len() as u64;
        if Instant::now() >= flush_at {
            file.flush()?;
            flush_at = Instant::now() + FLUSH_INTERVAL;
        }
        entry = match rx.recv_deadline(flush_at) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => {
                file.flush()?;
                let Ok(event) = rx.recv() else { return Ok(()) };
                flush_at = Instant::now() + FLUSH_INTERVAL;
                event
            }
            Err(RecvTimeoutError::Disconnected) => return file.flush(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_exit_writes_queued_feedback_before_returning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("midi.log");
        let trace = start(path.clone()).unwrap();
        trace
            .tx
            .send(Entry::Line(SystemTime::now(), "OUT [test] 90 0B 00".into()))
            .unwrap();
        trace.finish();
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .ends_with("OUT [test] 90 0B 00\n"));
    }

    #[test]
    fn short_sessions_flush_and_rotation_keeps_the_previous_part() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("midi.log");
        std::fs::write(&path, vec![b'x'; 180]).unwrap();
        std::fs::write(dir.path().join("midi.log.1"), b"older").unwrap();
        let file = OpenOptions::new().append(true).open(&path).unwrap();
        let (tx, rx) = bounded(4);
        tx.send(Entry::Line(
            SystemTime::now(),
            "IN [test] 90 40 7F\ncontrol".into(),
        ))
        .unwrap();
        drop(tx);
        write_session(&path, file, rx, &AtomicU64::new(2), 180).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("midi.log.1")).unwrap(),
            vec![b'x'; 180]
        );
        let current = std::fs::read_to_string(path).unwrap();
        assert!(current.contains("SESSION recordbox"));
        assert!(current.contains("Dropped 2 trace entries"));
        assert!(current.ends_with("IN [test] 90 40 7F control\n"));
    }
}
