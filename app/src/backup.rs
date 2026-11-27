//! File > Library > Backup Library / Restore Library.
//!
//! A backup is one zip archive:
//!   manifest.json     what the archive holds (format version, counts, music file map)
//!   library.db        a consistent snapshot of the library (VACUUM INTO)
//!   preferences.json  the saved preferences, as text (they also live inside library.db)
//!   artwork/<file>    the artwork images
//!   music/<n>/<file>  the music files (only when asked for)
//!
//! Restore validates the archive, keeps a copy of the present library, then copies the
//! snapshot over the open database through SQLite's online backup, so the app keeps running.
//! Music files are only ever added: a file that exists is never written over.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// Format 1 uses the original sample clock. Old readers will reject format 2,
// so a Rekordbox-clock collection cannot silently replace a legacy collection.
pub const FORMAT: u32 = 2;
const LEGACY_FORMAT: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub app: String,
    pub format: u32,
    pub created: String,
    pub schema: i64,
    pub tracks: usize,
    pub artwork_files: usize,
    /// (original path, entry in the archive).
    pub music: Vec<(String, String)>,
    /// Music files that could not be read when the backup was made.
    pub music_missing: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub tracks: usize,
    pub artwork: usize,
    pub music: usize,
    pub music_missing: usize,
    pub bytes: u64,
}

/// Progress: (done, total, what is being written).
pub type Progress<'a> = &'a dyn Fn(u64, u64, &str);

fn cancelled(c: &AtomicBool) -> Result<()> {
    if c.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    Ok(())
}

pub fn is_cancel(e: &anyhow::Error) -> bool {
    e.to_string() == "cancelled"
}

fn stored() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true)
}

fn add_file(zw: &mut zip::ZipWriter<std::fs::File>, name: &str, src: &Path, cancel: &AtomicBool) -> Result<u64> {
    let mut f = std::fs::File::open(src).with_context(|| format!("open {}", src.display()))?;
    zw.start_file(name, stored())?;
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        cancelled(cancel)?;
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        zw.write_all(&buf[..k])?;
        n += k as u64;
    }
    Ok(n)
}

/// Write a backup of the library at `db_path` to `dest`. The archive appears under its final
/// name only once it is complete.
pub fn write_backup(db_path: &Path, artwork_dir: &Path, dest: &Path, with_music: bool, progress: Progress, cancel: &AtomicBool) -> Result<Summary> {
    let tmp = dest.with_extension("partial");
    let r = write_backup_to(db_path, artwork_dir, &tmp, with_music, progress, cancel);
    match r {
        Ok(mut s) => {
            std::fs::rename(&tmp, dest)?;
            s.bytes = dest.metadata().map(|m| m.len()).unwrap_or(0);
            Ok(s)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn write_backup_to(db_path: &Path, artwork_dir: &Path, tmp: &Path, with_music: bool, progress: Progress, cancel: &AtomicBool) -> Result<Summary> {
    progress(0, 1, "Preparing backup...");
    // A consistent snapshot, taken through a second connection (it includes the write-ahead log).
    let snap = tmp.with_extension("snapshot.db");
    let _ = std::fs::remove_file(&snap);
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(snap.clone());
    let (schema, tracks, prefs, format) = {
        let c = rusqlite::Connection::open(db_path)?;
        c.busy_timeout(std::time::Duration::from_secs(10))?;
        c.execute("VACUUM INTO ?1", [snap.to_string_lossy().to_string()])?;
        drop(c);
        let s = rusqlite::Connection::open(&snap)?;
        let schema = s.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let mut st = s.prepare("SELECT path FROM tracks ORDER BY id")?;
        let tracks: Vec<String> = st.query_map([], |r| r.get::<_, String>(0))?.filter_map(|x| x.ok()).collect();
        let prefs = s.query_row("SELECT value FROM settings WHERE key='prefs'", [], |r| r.get::<_, String>(0)).ok();
        let rekordbox_clock: bool = s.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key='mp3_timeline' AND value='rekordbox')", [], |r| r.get(0))?;
        (schema, tracks, prefs, if rekordbox_clock { FORMAT } else { LEGACY_FORMAT })
    };
    cancelled(cancel)?;
    let art: Vec<PathBuf> = std::fs::read_dir(artwork_dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
    let music: Vec<&String> = if with_music { tracks.iter().collect() } else { vec![] };
    let size = |p: &Path| p.metadata().map(|m| m.len()).unwrap_or(0);
    let total: u64 = size(&snap) + art.iter().map(|p| size(p)).sum::<u64>() + music.iter().map(|p| size(Path::new(p))).sum::<u64>();
    let mut done = 0u64;

    let mut zw = zip::ZipWriter::new(std::fs::File::create(tmp).with_context(|| format!("create {}", tmp.display()))?);
    progress(done, total, "Database file");
    done += add_file(&mut zw, "library.db", &snap, cancel)?;
    if let Some(p) = &prefs {
        zw.start_file("preferences.json", stored())?;
        zw.write_all(p.as_bytes())?;
    }
    let mut artwork_files = 0;
    for a in &art {
        let Some(name) = a.file_name().and_then(|n| n.to_str()) else { continue };
        progress(done, total, "Analysis and image files");
        done += add_file(&mut zw, &format!("artwork/{name}"), a, cancel)?;
        artwork_files += 1;
    }
    let mut map = vec![];
    let mut missing = vec![];
    for (i, p) in music.iter().enumerate() {
        let src = Path::new(p.as_str());
        let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("track");
        if !src.is_file() {
            missing.push(p.to_string());
            continue;
        }
        progress(done, total, name);
        let entry = format!("music/{i}/{name}");
        match add_file(&mut zw, &entry, src, cancel) {
            Ok(n) => {
                done += n;
                map.push((p.to_string(), entry));
            }
            Err(e) if is_cancel(&e) => return Err(e),
            // An unreadable music file does not stop the backup; it is listed in the manifest.
            Err(_) => {
                zw.abort_file()?;
                missing.push(p.to_string());
            }
        }
    }
    let m = Manifest { app: "recordbox".into(), format, created: chrono::Local::now().to_rfc3339(), schema, tracks: tracks.len(), artwork_files, music: map, music_missing: missing };
    zw.start_file("manifest.json", stored())?;
    zw.write_all(&serde_json::to_vec_pretty(&m)?)?;
    let f = zw.finish()?;
    f.sync_all()?;
    progress(total, total, "");
    Ok(Summary { tracks: m.tracks, artwork: m.artwork_files, music: m.music.len(), music_missing: m.music_missing.len(), bytes: 0 })
}

/// A validated backup, unpacked and ready to be put in place.
#[derive(Debug)]
pub struct Staged {
    pub manifest: Manifest,
    /// The snapshot, upgraded to the current schema and with artwork paths pointing at this
    /// computer's artwork folder.
    pub db: PathBuf,
    pub work_dir: PathBuf,
    pub music_restored: usize,
    /// Music files that could not be put back at their original place.
    pub music_failed: Vec<String>,
}

fn open_backup(archive: &Path) -> Result<zip::ZipArchive<std::fs::File>> {
    let mut za = zip::ZipArchive::new(std::fs::File::open(archive)?).map_err(|_| anyhow!("This file is not a recordbox library backup."))?;
    za.validate_header_consistency().context("The backup's ZIP headers are inconsistent or damaged.")?;
    // Sampler packs can be compressed. All versions of our library backup writer store entries.
    for i in 0..za.len() {
        if za.by_index_raw(i)?.compression() != zip::CompressionMethod::Stored {
            bail!("Library backups must contain stored ZIP entries.");
        }
    }
    Ok(za)
}

pub fn read_manifest(archive: &Path) -> Result<Manifest> {
    read_archive_manifest(&mut open_backup(archive)?)
}

fn read_archive_manifest(za: &mut zip::ZipArchive<std::fs::File>) -> Result<Manifest> {
    let mut s = String::new();
    za.by_name("manifest.json").map_err(|_| anyhow!("This file is not a recordbox library backup."))?.read_to_string(&mut s)?;
    let m: Manifest = serde_json::from_str(&s).map_err(|_| anyhow!("The backup's contents list cannot be read."))?;
    // Archives written before the name was lowercased say "Recordbox".
    if !m.app.eq_ignore_ascii_case("recordbox") {
        bail!("This file is not a recordbox library backup.");
    }
    if m.format > FORMAT {
        bail!("This backup was made by a newer version of recordbox.");
    }
    let clock = if m.format == FORMAT { crate::audio::Mp3Timeline::Rekordbox } else { crate::audio::Mp3Timeline::Legacy };
    if clock != crate::audio::mp3_timeline() {
        bail!("This backup uses the {} MP3 timeline. Restart with --mp3-timeline {} to restore it into its matching collection.", clock.name(), clock.name());
    }
    if za.by_name("library.db").is_err() {
        bail!("The backup has no database file.");
    }
    Ok(m)
}

fn extract(za: &mut zip::ZipArchive<std::fs::File>, entry: &str, dest: &Path, cancel: &AtomicBool) -> Result<u64> {
    let mut e = za.by_name(entry)?;
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = dest.with_extension("restoring");
    let mut out = std::fs::File::create(&tmp)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&tmp);
            bail!("cancelled");
        }
        let k = e.read(&mut buf)?;
        if k == 0 {
            break;
        }
        out.write_all(&buf[..k])?;
        n += k as u64;
    }
    out.sync_all()?;
    std::fs::rename(&tmp, dest)?;
    Ok(n)
}

/// Unpack and check a backup without touching the present library. Artwork and (when
/// `with_music`) music files that are absent on this computer are added; nothing that exists
/// is replaced.
pub fn stage_restore(archive: &Path, work_dir: &Path, artwork_dir: &Path, with_music: bool, progress: Progress, cancel: &AtomicBool) -> Result<Staged> {
    progress(0, 1, "Preparing restore...");
    let mut za = open_backup(archive)?;
    let manifest = read_archive_manifest(&mut za)?;
    let _ = std::fs::remove_dir_all(work_dir);
    std::fs::create_dir_all(work_dir)?;
    let names: Vec<String> = za.file_names().map(|name| name.map(|s| s.into_owned())).collect::<zip::result::ZipResult<_>>()?;
    let total: u64 = (0..za.len()).filter_map(|i| za.by_index(i).ok().map(|f| f.size())).sum();
    let mut done = 0u64;
    let db = work_dir.join("library.db");
    progress(done, total, "Database file");
    done += extract(&mut za, "library.db", &db, cancel)?;
    // The snapshot must be a sound library; an older one is upgraded here, on the copy.
    {
        let c = rusqlite::Connection::open(&db)?;
        let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|_| anyhow!("The backup's database file is damaged."))?;
        if ok != "ok" {
            bail!("The backup's database file is damaged.");
        }
        let has: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE name='tracks'", [], |r| r.get(0))?;
        if has == 0 {
            bail!("The backup's database file is not a recordbox library.");
        }
    }
    {
        let d = crate::db::Db::open(&db)?;
        // A portable backup can come from another computer with the same user and paths.
        // Preserve the catalog, but only fresh imports can establish local cleanup eligibility.
        d.conn.execute("DELETE FROM direct_file_imports", [])?;
        // Artwork paths are absolute: point them at this computer's artwork folder.
        let rows: Vec<(i64, String)> = {
            let mut st = d.conn.prepare("SELECT id, artwork FROM tracks WHERE artwork != ''")?;
            let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|x| x.ok()).collect();
            v
        };
        for (id, a) in rows {
            let p = Path::new(&a);
            if p.parent().and_then(|x| x.file_name()).map(|n| n == "artwork").unwrap_or(false) {
                if let Some(name) = p.file_name() {
                    let now = artwork_dir.join(name).to_string_lossy().to_string();
                    if now != a {
                        d.conn.execute("UPDATE tracks SET artwork=?1 WHERE id=?2", rusqlite::params![now, id])?;
                    }
                }
            }
        }
        d.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
    }
    cancelled(cancel)?;
    for n in names.iter().filter(|n| n.starts_with("artwork/")) {
        let Some(file) = Path::new(n).file_name() else { continue };
        let dest = artwork_dir.join(file);
        progress(done, total, "Analysis and image files");
        if dest.exists() {
            continue;
        }
        done += extract(&mut za, n, &dest, cancel)?;
    }
    let mut music_restored = 0;
    let mut music_failed = vec![];
    if with_music {
        for (orig, entry) in &manifest.music {
            let dest = Path::new(orig);
            if dest.exists() || !dest.is_absolute() {
                continue;
            }
            progress(done, total, dest.file_name().and_then(|n| n.to_str()).unwrap_or(""));
            match extract(&mut za, entry, dest, cancel) {
                Ok(n) => {
                    done += n;
                    music_restored += 1;
                }
                Err(e) if is_cancel(&e) => return Err(e),
                Err(_) => music_failed.push(orig.clone()),
            }
        }
    }
    progress(total, total, "");
    Ok(Staged { manifest, db, work_dir: work_dir.to_path_buf(), music_restored, music_failed })
}

/// Put a staged backup in place of the open library: first a copy of the present library is
/// written to `safety_dir`, then the snapshot replaces the database's contents in one step.
/// Returns the path of the safety copy.
pub fn apply_restore(conn: &mut rusqlite::Connection, staged: &Staged, safety_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(safety_dir)?;
    let safety = safety_dir.join(format!("library-before-restore-{}.db", chrono::Local::now().format("%Y%m%d-%H%M%S%3f")));
    conn.execute("VACUUM INTO ?1", [safety.to_string_lossy().to_string()]).context("copy of the present library")?;
    let src = rusqlite::Connection::open(&staged.db)?;
    {
        let b = rusqlite::backup::Backup::new(&src, conn)?;
        b.run_to_completion(1024, std::time::Duration::from_millis(0), None)?;
    }
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    drop(src);
    let _ = std::fs::remove_dir_all(&staged.work_dir);
    Ok(safety)
}

// ------------------------------------------------------------------ background job

pub enum Outcome {
    Backup(Result<Summary>, PathBuf),
    Restore(Result<Staged>),
}

pub struct Job {
    pub restore: bool,
    pub cancel: Arc<AtomicBool>,
    /// (done, total, what), updated by the worker.
    pub progress: Arc<parking_lot::Mutex<(u64, u64, String)>>,
    pub rx: crossbeam_channel::Receiver<Outcome>,
}

fn spawn(restore: bool, ctx: eframe::egui::Context, work: impl FnOnce(Progress, &AtomicBool) -> Outcome + Send + 'static) -> Job {
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(parking_lot::Mutex::new((0u64, 0u64, String::new())));
    let (tx, rx) = crossbeam_channel::bounded(1);
    let (c2, p2) = (cancel.clone(), progress.clone());
    std::thread::spawn(move || {
        let ctx2 = ctx.clone();
        let report = move |d: u64, t: u64, w: &str| {
            *p2.lock() = (d, t, w.to_string());
            ctx2.request_repaint();
        };
        let out = work(&report, &c2);
        let _ = tx.send(out);
        ctx.request_repaint();
    });
    Job { restore, cancel, progress, rx }
}

pub fn start_backup(ctx: eframe::egui::Context, dest: PathBuf, with_music: bool) -> Job {
    let db = crate::db::data_dir().join("library.db");
    let art = crate::db::artwork_dir();
    spawn(false, ctx, move |p, c| Outcome::Backup(write_backup(&db, &art, &dest, with_music, p, c), dest.clone()))
}

pub fn start_restore(ctx: eframe::egui::Context, archive: PathBuf) -> Job {
    let work = crate::db::data_dir().join("restore-work");
    let art = crate::db::artwork_dir();
    spawn(true, ctx, move |p, c| Outcome::Restore(stage_restore(&archive, &work, &art, true, p, c)))
}

/// Called every frame: finish a backup or restore whose worker is done.
pub fn poll(app: &mut crate::app::App) {
    let Some(job) = &app.dialogs.ext.backup_job else { return };
    let Ok(out) = job.rx.try_recv() else { return };
    app.dialogs.ext.backup_job = None;
    let done = match out {
        Outcome::Backup(Ok(s), dest) => {
            let mut t = format!("{} track(s), {} artwork file(s)", s.tracks, s.artwork);
            if s.music > 0 || s.music_missing > 0 {
                t.push_str(&format!(", {} music file(s)", s.music));
            }
            if s.music_missing > 0 {
                t.push_str(&format!(" ({} music file(s) could not be read and are not in the backup)", s.music_missing));
            }
            ("Library Backup".to_string(), format!("Backup is finished.\n{t}\n{:.1} MB: {}", s.bytes as f64 / 1e6, dest.display()))
        }
        Outcome::Backup(Err(e), _) if is_cancel(&e) => ("Library Backup".into(), "Backup has been canceled.".into()),
        Outcome::Backup(Err(e), _) => ("Library Backup".into(), format!("Failed to back up the collection.\n{e:#}")),
        Outcome::Restore(Err(e)) if is_cancel(&e) => ("Library Restoration".into(), "Restore is canceled.".into()),
        Outcome::Restore(Err(e)) => ("Library Restoration".into(), format!("Failed to restore the collection.\n{e:#}\nThe present library was not changed.")),
        Outcome::Restore(Ok(staged)) => {
            let safety_dir = crate::db::data_dir().join("backups");
            match apply_restore(&mut app.db.conn, &staged, &safety_dir) {
                Ok(safety) => {
                    reload_after_restore(app);
                    let made = staged.manifest.created.get(..16).unwrap_or(&staged.manifest.created).replace('T', " ");
                    let mut t = format!("Collection was restored.\n{} track(s), from the backup made {made}.", app.tracks.len());
                    if staged.music_restored > 0 {
                        t.push_str(&format!(" {} music file(s) put back.", staged.music_restored));
                    }
                    if !staged.music_failed.is_empty() {
                        t.push_str(&format!("\nOne or more tracks were not restored: {} music file(s) could not be written to their original location.", staged.music_failed.len()));
                    }
                    t.push_str(&format!("\nThe library as it was before is kept at {}", safety.display()));
                    ("Library Restoration".into(), t)
                }
                Err(e) => ("Library Restoration".into(), format!("Failed to restore the collection.\n{e:#}\nThe present library was not changed.")),
            }
        }
    };
    app.dialogs.ext.backup_done = Some(done);
}

/// Everything the app holds in memory is read again from the restored database.
fn reload_after_restore(app: &mut crate::app::App) {
    // Both decks are emptied: their tracks may not exist in the restored library.
    for d in 0..2u8 {
        app.with_deck(d, |a| a.unload_track());
    }
    app.undo.clear();
    app.redo.clear();
    if let Some(p) = app.db.setting("prefs").and_then(|s| serde_json::from_str::<crate::app::Prefs>(&s).ok()) {
        // This computer's audio device and extra device folders stay as they are.
        let (dev, extra) = (app.prefs.audio_device.clone(), app.prefs.devices_extra.clone());
        app.prefs = p;
        app.prefs.audio_device = dev;
        app.prefs.devices_extra = extra;
        app.save_prefs();
    }
    app.textures.clear();
    app.selected.clear();
    app.anchor = None;
    app.anchor_id = None;
    app.side = crate::app::SideMode::Collection;
    app.view = crate::app::View::Collection;
    app.sub_view = crate::app::View::Collection;
    app.reload_all();
    app.list_dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("recordbox_backup_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn titles(c: &rusqlite::Connection) -> Vec<String> {
        let mut st = c.prepare("SELECT title FROM tracks ORDER BY title").unwrap();
        let v = st.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|x| x.unwrap()).collect();
        v
    }

    #[test]
    fn sampler_deflate_support_does_not_change_library_backup_format() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("compressed.zip");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        archive.start_file("manifest.json", options).unwrap();
        archive.write_all(b"{}").unwrap();
        archive.finish().unwrap();
        assert!(read_manifest(&path).unwrap_err().to_string().contains("stored ZIP entries"));
    }

    #[test]
    fn backup_restore_round_trip() {
        let d = dir("roundtrip");
        let (art, music) = (d.join("artwork"), d.join("music"));
        std::fs::create_dir_all(&art).unwrap();
        std::fs::create_dir_all(&music).unwrap();
        std::fs::write(art.join("aa.jpg"), b"jpeg bytes").unwrap();
        let song = music.join("one.mp3");
        std::fs::write(&song, vec![7u8; 3000]).unwrap();
        let db_path = d.join("library.db");
        let mut db = crate::db::Db::open(&db_path).unwrap();
        let mut t = crate::importer::read_track(&song);
        t.title = "One".into();
        t.artwork = "/some/other/computer/artwork/aa.jpg".into();
        db.insert_track(&t).unwrap();
        let mut gone = t.clone();
        gone.path = music.join("gone.mp3").to_string_lossy().to_string();
        gone.title = "Gone".into();
        db.insert_track(&gone).unwrap();
        db.set_setting("prefs", r#"{"key_alnum":true}"#);
        let pl = db.create_node(0, "Set", crate::model::NodeKind::Playlist, None).unwrap();

        let archive = d.join("backup.zip");
        let never = AtomicBool::new(false);
        let seen = std::cell::Cell::new(0u64);
        let s = write_backup(&db_path, &art, &archive, true, &|done, _, _| seen.set(seen.get().max(done)), &never).unwrap();
        assert_eq!((s.tracks, s.artwork, s.music, s.music_missing), (2, 1, 1, 1));
        assert!(seen.get() >= 3000, "progress counts bytes");
        assert!(!archive.with_extension("partial").exists());
        let m = read_manifest(&archive).unwrap();
        assert_eq!(m.music.len(), 1);
        assert_eq!(m.music_missing, vec![gone.path.clone()]);

        // The library changes after the backup; the music file and artwork disappear.
        db.conn.execute("DELETE FROM tracks WHERE title='One'", []).unwrap();
        db.conn.execute("UPDATE tracks SET title='Changed'", []).unwrap();
        db.delete_node(pl).unwrap();
        db.set_setting("prefs", r#"{"key_alnum":false}"#);
        std::fs::remove_file(&song).unwrap();
        std::fs::remove_file(art.join("aa.jpg")).unwrap();
        assert_eq!(titles(&db.conn), vec!["Changed"]);

        let staged = stage_restore(&archive, &d.join("work"), &art, true, &|_, _, _| {}, &never).unwrap();
        assert_eq!(staged.music_restored, 1);
        assert!(staged.music_failed.is_empty());
        // Staging leaves the open library alone.
        assert_eq!(titles(&db.conn), vec!["Changed"]);
        let safety = apply_restore(&mut db.conn, &staged, &d.join("backups")).unwrap();
        assert_eq!(titles(&db.conn), vec!["Gone", "One"]);
        assert_eq!(db.setting("prefs").as_deref(), Some(r#"{"key_alnum":true}"#));
        assert_eq!(db.playlists().unwrap().len(), 1);
        assert_eq!(std::fs::read(&song).unwrap(), vec![7u8; 3000]);
        assert_eq!(std::fs::read(art.join("aa.jpg")).unwrap(), b"jpeg bytes");
        // Artwork now points into this computer's artwork folder.
        let a: String = db.conn.query_row("SELECT artwork FROM tracks WHERE title='One'", [], |r| r.get(0)).unwrap();
        assert_eq!(a, art.join("aa.jpg").to_string_lossy());
        // The library as it was before the restore is kept.
        let before = rusqlite::Connection::open(&safety).unwrap();
        assert_eq!(titles(&before), vec!["Changed"]);
        assert!(!d.join("work").exists());
        // The restored database still takes writes through the same connection.
        db.set_setting("after", "1");
        assert_eq!(db.setting("after").as_deref(), Some("1"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mismatched_zip_headers_are_refused_before_restore_writes() {
        let d = dir("mismatched_headers");
        let art = d.join("artwork");
        std::fs::create_dir_all(&art).unwrap();
        let image = art.join("aa.jpg");
        std::fs::write(&image, b"jpeg bytes").unwrap();
        let song = d.join("one.mp3");
        std::fs::write(&song, b"music bytes").unwrap();
        let db_path = d.join("library.db");
        let db = crate::db::Db::open(&db_path).unwrap();
        let mut t = crate::importer::read_track(&song);
        t.title = "Original".into();
        t.artwork = image.to_string_lossy().to_string();
        db.insert_track(&t).unwrap();
        let archive = d.join("valid.zip");
        let never = AtomicBool::new(false);
        write_backup(&db_path, &art, &archive, true, &|_, _, _| {}, &never).unwrap();
        assert!(read_manifest(&archive).is_ok());
        let valid = std::fs::read(&archive).unwrap();

        db.conn.execute("UPDATE tracks SET title='Changed'", []).unwrap();
        std::fs::remove_file(&song).unwrap();
        std::fs::remove_file(&image).unwrap();
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let sentinel = work.join("keep.txt");
        std::fs::write(&sentinel, b"existing work").unwrap();

        // Keep the central directory and payload intact; change only the corresponding
        // local header name. Also check an entry not needed to read the manifest.
        for (i, entry) in ["manifest.json", "library.db", "artwork/aa.jpg"].iter().enumerate() {
            let mut za = zip::ZipArchive::new(std::io::Cursor::new(&valid)).unwrap();
            let offset = za.by_name(entry).unwrap().header_start() as usize;
            let mut malformed = valid.clone();
            assert_eq!(&malformed[offset..offset + 4], b"PK\x03\x04");
            let name_len = u16::from_le_bytes([malformed[offset + 26], malformed[offset + 27]]) as usize;
            let name_start = offset + 30;
            assert_eq!(&malformed[name_start..name_start + name_len], entry.as_bytes());
            malformed[name_start + name_len - 1] ^= 1;
            let bad = d.join(format!("mismatched-{i}.zip"));
            std::fs::write(&bad, &malformed).unwrap();

            assert!(read_manifest(&bad).unwrap_err().to_string().contains("ZIP headers"));
            assert!(stage_restore(&bad, &work, &art, true, &|_, _, _| {}, &never)
                .unwrap_err().to_string().contains("ZIP headers"));
            assert_eq!(titles(&db.conn), vec!["Changed"]);
            assert_eq!(std::fs::read(&sentinel).unwrap(), b"existing work");
            assert!(!work.join("library.db").exists());
            assert!(!song.exists());
            assert!(!image.exists());
            assert_eq!(std::fs::read(&bad).unwrap(), malformed);
        }
        assert_eq!(std::fs::read(&archive).unwrap(), valid);
        drop(db);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn existing_music_is_never_replaced_and_bad_archives_are_refused() {
        let d = dir("refuse");
        let art = d.join("artwork");
        std::fs::create_dir_all(&art).unwrap();
        let song = d.join("keep.mp3");
        std::fs::write(&song, b"original").unwrap();
        let db_path = d.join("library.db");
        let mut db = crate::db::Db::open(&db_path).unwrap();
        let mut t = crate::importer::read_track(&song);
        t.title = "Keep".into();
        db.insert_track(&t).unwrap();
        let archive = d.join("b.zip");
        let never = AtomicBool::new(false);
        write_backup(&db_path, &art, &archive, true, &|_, _, _| {}, &never).unwrap();
        // The file is edited after the backup: a restore leaves it as it is now.
        std::fs::write(&song, b"edited since").unwrap();
        let staged = stage_restore(&archive, &d.join("work"), &art, true, &|_, _, _| {}, &never).unwrap();
        assert_eq!(staged.music_restored, 0);
        assert_eq!(std::fs::read(&song).unwrap(), b"edited since");

        // Not a zip, a zip without a manifest, and a damaged database are all refused, and
        // the open library is untouched.
        let junk = d.join("junk.zip");
        std::fs::write(&junk, b"not a zip at all").unwrap();
        assert!(stage_restore(&junk, &d.join("w2"), &art, true, &|_, _, _| {}, &never).is_err());
        let other = d.join("other.zip");
        {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&other).unwrap());
            zw.start_file("hello.txt", stored()).unwrap();
            zw.write_all(b"hi").unwrap();
            zw.finish().unwrap();
        }
        assert!(read_manifest(&other).is_err());
        let bad = d.join("bad.zip");
        {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
            zw.start_file("library.db", stored()).unwrap();
            zw.write_all(&vec![0x55u8; 4096]).unwrap();
            zw.start_file("manifest.json", stored()).unwrap();
            zw.write_all(&serde_json::to_vec(&Manifest { app: "recordbox".into(), format: LEGACY_FORMAT, ..Default::default() }).unwrap()).unwrap();
            zw.finish().unwrap();
        }
        assert!(stage_restore(&bad, &d.join("w3"), &art, true, &|_, _, _| {}, &never).is_err());
        let newer = d.join("newer.zip");
        {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&newer).unwrap());
            zw.start_file("library.db", stored()).unwrap();
            zw.start_file("manifest.json", stored()).unwrap();
            zw.write_all(&serde_json::to_vec(&Manifest { app: "recordbox".into(), format: FORMAT + 1, ..Default::default() }).unwrap()).unwrap();
            zw.finish().unwrap();
        }
        assert!(read_manifest(&newer).unwrap_err().to_string().contains("newer"));
        assert_eq!(titles(&db.conn), vec!["Keep"]);

        // Cancelling leaves no archive behind.
        let cancelled = AtomicBool::new(true);
        let out = d.join("c.zip");
        let e = write_backup(&db_path, &art, &out, true, &|_, _, _| {}, &cancelled).unwrap_err();
        assert!(is_cancel(&e));
        assert!(!out.exists() && !out.with_extension("partial").exists());
        let _ = db.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        let _ = std::fs::remove_dir_all(&d);
    }
}
