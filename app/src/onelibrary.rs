//! Optional playlist inspection on closed USB catalog copies.
use anyhow::{bail, ensure, Context, Result};
use eframe::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

const HELPER: &str = include_str!("onelibrary_reader.py");
const SETUP: &str = "Install Python 3.12 and rbox==0.3.1 in ~/.recordbox-onelibrary, or set RECORDBOX_ONELIBRARY_PYTHON to the absolute path of its Python interpreter. See USB compatibility in the recordbox documentation.";
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REPORT: u64 = 16 * 1024 * 1024;
const USAGE: &str = "--inspect-onelibrary USB_ROOT [--playlist ID [--classic-playlist ID]]";

#[derive(Deserialize, Serialize)]
struct Playlist {
    id: i64,
    parent_id: i64,
    name: String,
    kind: i32,
}

#[derive(Deserialize, Serialize)]
struct Track {
    id: i64,
    title: String,
    path: String,
}

#[derive(Deserialize, Serialize)]
struct Report {
    db_version: i32,
    one_playlists: Vec<Playlist>,
    classic_playlists: Option<Vec<Playlist>>,
    one_tracks: Option<Vec<Track>>,
    classic_tracks: Option<Vec<Track>>,
    comparison: Option<String>,
}

fn interpreter() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("RECORDBOX_ONELIBRARY_PYTHON") {
        let path = PathBuf::from(path);
        ensure!(path.is_absolute() && path.is_file(), "{SETUP}");
        return Ok(path);
    }
    let python = if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    };
    let path = dirs::home_dir()
        .context(SETUP)?
        .join(".recordbox-onelibrary")
        .join(python);
    ensure!(path.is_file(), "{SETUP}");
    Ok(path)
}

#[derive(PartialEq, Debug)]
struct Stamp {
    size: u64,
    modified: SystemTime,
    hash: [u8; 32],
}

// Read through a single open handle and cap actual bytes, including a growing source.
fn stream(path: &Path, mut to: impl Write) -> Result<Stamp> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() > 0 && meta.len() <= MAX_BYTES,
        "Catalog must be a regular file between 1 byte and 256 MiB"
    );
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        ensure!(size <= meta.len(), "Catalog changed while reading");
        to.write_all(&buf[..n])?;
        hash.update(&buf[..n]);
    }
    let after = file.metadata()?;
    ensure!(
        size == meta.len() && after.len() == size && after.modified()? == meta.modified()?,
        "Catalog changed while reading"
    );
    Ok(Stamp {
        size,
        modified: meta.modified()?,
        hash: hash.finalize().into(),
    })
}

fn read_output(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut data)?;
    ensure!(
        data.len() as u64 <= limit,
        "OneLibrary helper output exceeds the size limit"
    );
    Ok(data)
}

fn read_copy(dir: &Path, python: &Path, one: Option<i64>, classic: Option<i64>) -> Result<Report> {
    let stdout = dir.join("report.json");
    let stderr = dir.join("reader.log");
    let mut command = Command::new(python);
    command
        .args(["-I", "-X", "utf8", "-c", HELPER])
        .arg(dir)
        .args([one, classic].map(|id| id.map(|v| v.to_string()).unwrap_or_else(|| "-".into())))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(File::create(&stdout)?)
        .stderr(File::create(&stderr)?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW for the GUI action.
    }
    let mut child = command.spawn().context(SETUP)?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(120) {
            let _ = child.kill();
            child.wait()?;
            bail!("OneLibrary reader timed out after two minutes");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if !status.success() {
        let error = read_output(&stderr, 32 * 1024)?;
        bail!(
            "Reader exited with {status}. {}",
            String::from_utf8_lossy(&error).trim()
        );
    }
    Ok(serde_json::from_slice(&read_output(
        &stdout,
        MAX_REPORT + 2,
    )?)?)
}

fn snapshot_dir(root: &Path) -> Result<tempfile::TempDir> {
    let temp_base = std::env::temp_dir().canonicalize()?;
    ensure!(
        !temp_base.starts_with(root.canonicalize()?),
        "Choose a system temporary directory outside the selected USB root"
    );
    let mut builder = tempfile::Builder::new();
    builder.prefix("recordbox-onelibrary-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    Ok(builder.tempdir_in(temp_base)?)
}

fn inspect(root: &Path, one: Option<i64>, classic_id: Option<i64>) -> Result<Report> {
    let root = std::path::absolute(root)?;
    let files = crate::export::inspect_onelibrary_files(&root)?;
    ensure!(
        classic_id.is_none() || files.1.is_some(),
        "This device has no classic catalog"
    );
    let python = interpreter()?;
    let temp = snapshot_dir(&root)?;
    let mut sources = vec![(&files.0, "exportLibrary.db")];
    if let Some(classic) = &files.1 {
        sources.push((classic, "export.pdb"));
    }
    let mut stamps = Vec::new();
    for (source, name) in &sources {
        stamps.push(stream(source, File::create_new(temp.path().join(name))?)?);
    }
    ensure!(
        crate::export::inspect_onelibrary_files(&root)? == files,
        "USB catalog layout changed while taking the snapshot"
    );
    let report = read_copy(temp.path(), &python, one, classic_id)?;
    for ((source, _), original) in sources.iter().zip(stamps) {
        ensure!(
            stream(source, std::io::sink())? == original,
            "Source catalog changed during inspection. Close Rekordbox and retry."
        );
    }
    ensure!(
        crate::export::inspect_onelibrary_files(&root)? == files,
        "USB catalog layout changed during inspection"
    );
    Ok(report)
}

pub fn cli(mut args: impl Iterator<Item = OsString>) -> Result<()> {
    let root = PathBuf::from(args.next().context(USAGE)?);
    if root == Path::new("--help") || root == Path::new("-h") {
        println!("recordbox {USAGE}");
        return Ok(());
    }
    let mut one = None;
    let mut classic = None;
    while let Some(flag) = args.next() {
        let slot = match flag.to_str() {
            Some("--playlist") => &mut one,
            Some("--classic-playlist") => &mut classic,
            _ => bail!("{USAGE}"),
        };
        ensure!(slot.is_none(), "Duplicate option. {USAGE}");
        let id = args
            .next()
            .and_then(|v| v.to_str()?.parse::<i64>().ok())
            .filter(|id| *id > 0)
            .context(USAGE)?;
        *slot = Some(id);
    }
    ensure!(classic.is_none() || one.is_some(), "{USAGE}");
    let report = inspect(&root, one, classic)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

#[derive(Default)]
pub struct Inspector {
    visible: bool,
    root: PathBuf,
    running: Option<mpsc::Receiver<Result<Report>>>,
    report: Option<Report>,
    error: Option<String>,
    one: Option<i64>,
    classic: Option<i64>,
}

impl Inspector {
    pub fn button(&mut self, ui: &mut egui::Ui, root: &str) -> bool {
        if ui
            .add_enabled(
                self.running.is_none(),
                egui::Button::new("Inspect OneLibrary playlists…"),
            )
            .on_hover_text("Read playlist order on a closed catalog copy.")
            .clicked()
        {
            self.root = root.into();
            self.visible = true;
            self.one = None;
            self.classic = None;
            self.report = None;
            self.start(ui.ctx());
            true
        } else {
            false
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        self.running = Some(rx);
        self.error = None;
        // The last displayed order belonged to the previous selection/snapshot.
        if let Some(r) = &mut self.report {
            r.one_tracks = None;
            r.classic_tracks = None;
            r.comparison = None;
        }
        let (root, one, classic, ctx) = (self.root.clone(), self.one, self.classic, ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(inspect(&root, one, classic));
            ctx.request_repaint();
        });
    }

    pub fn window(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.running {
            match rx.try_recv() {
                Ok(Ok(report)) => {
                    self.report = Some(report);
                    self.running = None;
                }
                Ok(Err(e)) => {
                    self.error = Some(format!("{e:#}"));
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some("Inspection worker stopped.".into());
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.visible {
            return;
        }
        let mut open = true;
        egui::Window::new("OneLibrary playlist inspector")
            .open(&mut open)
            .default_width(570.0)
            .show(ctx, |ui| {
                ui.label(self.root.display().to_string());
                if self.running.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Inspecting closed catalog copies…");
                    });
                    return;
                }
                if let Some(err) = &self.error {
                    ui.label(err);
                ui.hyperlink_to("Reader setup", "https://github.com/fantuz/recordbox/blob/main/docs/USB-COMPATIBILITY.md#inspect-onelibrary-playlists");
                    if ui.button("Reload catalogs").clicked() {
                        self.report = None;
                        self.one = None;
                        self.classic = None;
                        self.start(ui.ctx());
                    }
                    return;
                }
                let Some(report) = &self.report else { return };
                let mut changed = select(
                    ui,
                    "OneLibrary playlist",
                    &report.one_playlists,
                    &mut self.one,
                );
                if let Some(nodes) = &report.classic_playlists {
                    ui.add_enabled_ui(self.one.is_some(), |ui| {
                        changed |= select(ui, "Compare classic playlist", nodes, &mut self.classic);
                    });
                }
                if self.one.is_none() {
                    self.classic = None;
                }
                if changed {
                    self.start(ui.ctx());
                    return;
                }
                if let Some(result) = &report.comparison {
                    ui.label(match result.as_str() {
                        "same_path_order" => "The resolved track paths match in order.",
                        "different_path_order" => {
                            "The resolved track paths differ in order or membership."
                        }
                        _ => "Some resolved entries have empty audio paths. Order comparison is inconclusive.",
                    });
                }
                if let Some(tracks) = &report.one_tracks {
                    ui.label(format!("OneLibrary · {} resolved entries", tracks.len()));
                    if let Some(classic) = &report.classic_tracks {
                        ui.label(format!("Classic · {} entries", classic.len()));
                    }
                    let classic = report.classic_tracks.as_deref().unwrap_or_default();
                    let n = tracks.len().max(classic.len());
                    egui::ScrollArea::vertical().max_height(300.0).show_rows(
                        ui,
                        ui.text_style_height(&egui::TextStyle::Body),
                        n,
                        |ui, range| {
                            for i in range {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{}.", i + 1));
                                    for entry in std::iter::once(tracks.get(i)).chain(
                                        report.classic_tracks.as_ref().map(|_| classic.get(i)),
                                    ) {
                                        if let Some(t) = entry {
                                            ui.add(
                                                egui::Label::new(if t.title.is_empty() {
                                                    &t.path
                                                } else {
                                                    &t.title
                                                })
                                                .truncate(),
                                            )
                                            .on_hover_text(&t.path);
                                        } else {
                                            ui.label("—");
                                        }
                                    }
                                });
                            }
                        },
                    );
                } else {
                    ui.label("Choose a OneLibrary playlist to read its track order.");
                }
                if ui.button("Refresh snapshot").clicked() {
                    self.start(ui.ctx());
                }
            });
        self.visible = open;
    }
}

fn select(ui: &mut egui::Ui, label: &str, nodes: &[Playlist], id: &mut Option<i64>) -> bool {
    let old = *id;
    let name = nodes
        .iter()
        .find(|n| Some(n.id) == *id)
        .map(|n| n.name.as_str())
        .unwrap_or("Choose…");
    egui::ComboBox::from_label(label)
        .selected_text(name)
        .show_ui(ui, |ui| {
            ui.selectable_value(id, None, "Choose…");
            let regular: Vec<_> = nodes.iter().filter(|n| n.kind == 0).collect();
            egui::ScrollArea::vertical().max_height(240.0).show_rows(
                ui,
                ui.spacing().interact_size.y,
                regular.len(),
                |ui, range| {
                    for node in &regular[range] {
                        ui.selectable_value(
                            id,
                            Some(node.id),
                            format!("{} (#{})", node.name, node.id),
                        );
                    }
                },
            );
        });
    *id != old
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn snapshot_directory_is_private_even_on_a_shared_temp_volume() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let source = tempfile::tempdir()?;
        let snapshot = snapshot_dir(source.path())?;
        assert_eq!(
            snapshot.path().metadata()?.permissions().mode() & 0o777,
            0o700
        );
        Ok(())
    }

    #[test]
    fn refuses_live_sidecars_recovery_and_ambiguous_onelibraries() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let dir = root.join("pioneer/Rekordbox");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("ExportLibrary.DB"), "demo")?;
        assert_eq!(crate::export::inspect_onelibrary_files(root)?.1, None);
        for suffix in ["-WAL", "-shm", "-journal"] {
            let marker = dir.join(format!("exportLibrary.db{suffix}"));
            fs::write(&marker, [])?;
            assert!(crate::export::inspect_onelibrary_files(root).is_err());
            fs::remove_file(marker)?;
        }
        let journal = root.join(".Recordbox/journal");
        fs::create_dir_all(&journal)?;
        let log = journal.join("journal.log");
        fs::write(&log, [])?;
        assert!(crate::export::inspect_onelibrary_files(root).is_err());
        fs::remove_file(log)?;
        let hidden = root.join(".PIONEER/rekordbox");
        fs::create_dir_all(&hidden)?;
        fs::write(hidden.join("exportLibrary.db"), "other")?;
        assert!(crate::export::inspect_onelibrary_files(root).is_err());
        Ok(())
    }

    #[test]
    fn rejects_an_unpaired_classic_and_matches_the_entire_snapshot() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let modern = temp.path().join("PIONEER/rekordbox");
        let hidden = temp.path().join(".PIONEER/rekordbox");
        fs::create_dir_all(&modern)?;
        fs::create_dir_all(&hidden)?;
        let source = modern.join("exportLibrary.db");
        fs::write(&source, b"old")?;
        fs::write(hidden.join("export.pdb"), b"classic")?;
        assert!(crate::export::inspect_onelibrary_files(temp.path()).is_err());
        let mut copy = Vec::new();
        let at_start = stream(&source, &mut copy)?;
        assert_eq!(copy, b"old");
        assert_eq!(stream(&source, std::io::sink())?, at_start);
        fs::write(&source, b"new")?;
        assert_ne!(stream(&source, std::io::sink())?.hash, at_start.hash);
        Ok(())
    }
}
