//! Move reviewed references in the private Recordbox database into its existing trash.
use super::{review_selected, Report};
use crate::backup;
use crate::db::{Db, SCHEMA_VERSION};
use anyhow::{ensure, Context, Result};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub const MAX_SELECTION: usize = 25;

pub(super) struct Protections {
    /// Native, externally imported, legacy, restored and subsequently edited tracks.
    pub native: HashSet<i64>,
    /// Tracks that have preparation data or participate in another library feature.
    pub prepared: HashSet<i64>,
    smart: Vec<crate::smart::SmartRule>,
    today: i64,
}

impl Protections {
    pub(super) fn is_prepared(&self, track: &crate::model::Track) -> bool {
        if self.prepared.contains(&track.id) {
            return true;
        }
        // All tagged tracks are already protected by the track_tags query.
        let no_tags = HashSet::new();
        self.smart
            .iter()
            .any(|rule| crate::smart::matches(rule, track, &no_tags, self.today))
    }
}

fn id_set(db: &Db, sql: &str) -> Result<HashSet<i64>> {
    let mut query = db.conn.prepare(sql)?;
    let matches = query.query_map([], |row| row.get(0))?;
    Ok(matches.collect::<rusqlite::Result<_>>()?)
}

// Conservative comparison used only to *protect* slots that can be resolved by path.
fn slot_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

pub(super) fn protections(db: &Db) -> Result<Protections> {
    let mut native = id_set(db, "SELECT id FROM tracks WHERE id NOT IN
        (SELECT track_id FROM direct_file_imports) UNION ALL SELECT track_id FROM native_edit_baseline")?;
    let mut prepared = id_set(db, "SELECT id FROM tracks WHERE
        rating!=0 OR play_count!=0 OR color!=0 OR analyzed!=0 OR grid_locked!=0 OR key_manual!=0
        OR gain_db!=0 OR comment!='' OR message!='' OR grouping!='' OR artwork!=''
        UNION ALL SELECT track_id FROM playlist_tracks UNION ALL SELECT track_id FROM history_entries
        UNION ALL SELECT track_id FROM tag_list UNION ALL SELECT track_id FROM track_tags
        UNION ALL SELECT a FROM track_matches UNION ALL SELECT b FROM track_matches
        UNION ALL SELECT track_id FROM grids UNION ALL SELECT track_id FROM waves
        UNION ALL SELECT track_id FROM cues UNION ALL SELECT track_id FROM vocals
        UNION ALL SELECT track_id FROM peaks UNION ALL SELECT track_id FROM xml_extra
        UNION ALL SELECT track_id FROM media_stamps
        UNION ALL SELECT id FROM tracks WHERE EXISTS (
            SELECT 1 FROM settings WHERE key='phrase_edit:'||tracks.id AND value!='')")?;
    // DJs can remove a Matching Tracks pair while the edit or stem still links to its source.
    crate::db::visit_derived_links(&db.conn, |child, source| {
        prepared.insert(child.context("Cannot verify a derived track ID")?);
        prepared.insert(source.context("Cannot verify a derived track's source ID")?);
        Ok(())
    })?;
    let mut paths = HashSet::new();
    if let Some(json) = db.setting_checked("hot_cue_banks")? {
        let banks: crate::export::hotcue_bank::Banks =
            serde_json::from_str(&json).context("Cannot verify the saved Hot Cue Bank links")?;
        for slot in banks.0.iter().flat_map(|bank| bank.slots.iter().flatten()) {
            prepared.insert(slot.track_id);
            paths.insert(slot_path(&slot.path));
        }
    }
    if let Some(json) = db.setting_checked("prefs")? {
        #[derive(serde::Deserialize)]
        struct SamplerSettings {
            #[serde(default)]
            sampler: crate::ui_sampler::SamplerPrefs,
        }
        let settings: SamplerSettings =
            serde_json::from_str(&json).context("Cannot verify the saved sampler links")?;
        for slot in settings.sampler.slots.iter().flatten() {
            prepared.insert(slot.track);
            paths.insert(slot_path(&slot.path));
        }
        paths.extend(settings.sampler.colors.keys().map(|p| slot_path(p)));
    }
    // A user can re-import the audio after removing a native track. The ANLZ map
    // and the sync exclusion list may still hold its path across the row-ID change.
    let mut native_paths = HashSet::new();
    if let Some(json) = db.setting_checked("rb_anlz")? {
        let anlz: std::collections::HashMap<String, String> =
            serde_json::from_str(&json).context("Cannot verify the saved native analysis paths")?;
        native_paths.extend(anlz.keys().map(|p| slot_path(p)));
    }
    if let Some(json) = db.setting_checked("rb_removed_paths")? {
        let removed: Vec<String> =
            serde_json::from_str(&json).context("Cannot verify the native sync exclusions")?;
        native_paths.extend(removed.iter().map(|p| slot_path(p)));
    }
    if !paths.is_empty() || !native_paths.is_empty() {
        let mut statement = db
            .conn
            .prepare("SELECT id,path FROM tracks JOIN direct_file_imports ON track_id=id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let path = slot_path(row.get_ref(1)?.as_str()?);
            if paths.contains(&path) {
                prepared.insert(row.get(0)?);
            }
            if native_paths.contains(&path) {
                native.insert(row.get(0)?);
            }
        }
    }
    let mut smart = Vec::new();
    let mut stmt = db
        .conn
        .prepare("SELECT smart FROM playlists WHERE kind=2")?;
    let rules = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for json in rules {
        smart.push(serde_json::from_str(&json?).context("Cannot verify an intelligent playlist")?);
    }
    Ok(Protections {
        native,
        prepared,
        smart,
        today: crate::smart::today_days(),
    })
}

fn check_confirmation(count: usize, local_music_confirmed: bool, confirmation: &str) -> Result<()> {
    ensure!(
        (1..=MAX_SELECTION).contains(&count),
        "Select between 1 and {MAX_SELECTION} specific references."
    );
    ensure!(
        local_music_confirmed,
        "Confirm that the selected Music folder is local and independent of cloud sync."
    );
    ensure!(
        confirmation == format!("MOVE {count}"),
        "Type MOVE {count} to confirm this exact selection."
    );
    Ok(())
}

pub fn apply(
    library: &Path,
    report: &Report,
    selected: &BTreeSet<i64>,
    local_music_confirmed: bool,
    confirmation: &str,
) -> Result<PathBuf> {
    ensure!(
        cfg!(any(target_os = "windows", target_os = "macos")),
        "Cleanup on this operating system is review only."
    );
    check_confirmation(selected.len(), local_music_confirmed, confirmation)?;
    let active = crate::db::data_dir_for_inspection().join("library.db");
    ensure!(
        fs::symlink_metadata(&active)?.is_file(),
        "Select the regular private Recordbox library file."
    );
    let path = fs::canonicalize(library)?;
    ensure!(
        path == fs::canonicalize(&report.library)? && path == fs::canonicalize(active)?,
        "The active Recordbox library changed. Review again."
    );
    let db = Db {
        conn: Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?,
    };
    db.conn
        .execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    let ids: Vec<_> = selected.iter().copied().collect();
    move_reviewed(&db, &path, &ids, || {
        let current = review_selected(&db, report, &ids)?;
        report.verify_selection(&current, &ids)
    })
}

// Separate connection for the backup sees all committed WAL pages. The IMMEDIATE transaction
// prevents another writer from changing the collection between validation, snapshot and commit.
fn move_reviewed(
    db: &Db,
    path: &Path,
    ids: &[i64],
    verify: impl Fn() -> Result<()>,
) -> Result<PathBuf> {
    let tx = Transaction::new_unchecked(&db.conn, TransactionBehavior::Immediate)?;
    ensure!(
        db.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?
            == SCHEMA_VERSION,
        "The library schema changed. Reopen Recordbox."
    );
    verify()?;
    let parent = path.parent().context("Library folder is unavailable")?;
    let root = parent.join("cleanup-backups");
    match fs::create_dir(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    ensure!(
        fs::symlink_metadata(&root)?.file_type().is_dir() && root.canonicalize()? == root,
        "The cleanup backup folder must be a local directory beside the library."
    );
    let dir = tempfile::Builder::new()
        .prefix("review-")
        .tempdir_in(&root)?;
    let archive = dir.path().join("recordbox-before-cleanup.zip");
    let expected: i64 = db
        .conn
        .query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))?;
    backup::write_backup(
        path,
        &parent.join("artwork"),
        &archive,
        false,
        &|_, _, _| {},
        &AtomicBool::new(false),
    )?;
    verify_backup(&archive, dir.path(), expected)?;
    // File access can change while the archive is being written.
    verify()?;
    for id in ids {
        let _: i64 = db
            .conn
            .query_row("SELECT id FROM tracks WHERE id=?1", [id], |r| r.get(0))?;
        db.trash_track(*id)?;
        ensure!(
            db.conn.changes() == 1
                && db.conn.query_row(
                    "SELECT NOT EXISTS(SELECT 1 FROM tracks WHERE id=?1)
                    AND EXISTS(SELECT 1 FROM trash WHERE track_id=?1 AND id=last_insert_rowid())",
                    [id],
                    |row| row.get::<_, bool>(0)
                )?,
            "Track {id} could not be moved to Removed Tracks."
        );
    }
    let _kept = dir.keep();
    tx.commit()?;
    Ok(archive)
}

fn verify_backup(path: &Path, scratch: &Path, expected_tracks: i64) -> Result<()> {
    let manifest = backup::read_manifest(path)?;
    ensure!(
        manifest.schema == SCHEMA_VERSION
            && manifest.tracks as i64 == expected_tracks
            && manifest.music.is_empty(),
        "Unexpected cleanup backup manifest."
    );
    let temp = tempfile::Builder::new()
        .suffix(".db")
        .tempfile_in(scratch)?
        .into_temp_path();
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.name()? == "library.db" {
            let mut db_copy = fs::File::create(&temp)?;
            io::copy(&mut entry, &mut db_copy)?;
        } else {
            io::copy(&mut entry, &mut io::sink())?;
        }
    }
    let check = Connection::open_with_flags(&temp, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    ensure!(
        check.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? == "ok"
            && !check.prepare("PRAGMA foreign_key_check")?.exists([])?
            && check.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?
                == SCHEMA_VERSION
            && check.query_row("SELECT count(*) FROM tracks", [], |r| r.get::<_, i64>(0))?
                == expected_tracks,
        "The cleanup recovery copy failed verification."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeKind, Track};
    use std::cell::Cell;

    fn track(path: &str) -> Track {
        Track {
            path: path.to_owned(),
            title: path.to_owned(),
            sample_rate: 44100,
            file_size: 2048,
            ..Default::default()
        }
    }

    fn collection() -> (tempfile::TempDir, PathBuf, Db) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("library.db");
        let db = Db::open(&path).unwrap();
        (dir, path, db)
    }

    #[test]
    fn direct_provenance_requires_a_new_parsed_file_and_is_revoked_after_any_edit() {
        let (_dir, _path, db) = collection();
        let source = track("same.mp3");
        let foreign = db.insert_track(&source).unwrap();
        assert!(db.insert_local_file(&source).is_err());
        let unreadable = db
            .insert_local_file(&Track {
                sample_rate: 0,
                ..track("unparsed.mp3")
            })
            .unwrap();
        let raw = db.insert_local_file(&track("fresh.mp3")).unwrap();
        let linked = db.insert_local_file(&track("prior-db-id.mp3")).unwrap();
        db.set_setting_checked("rb_anlz", r#"{"PRIOR-DB-ID.MP3":"ANLZ0000"}"#)
            .unwrap();
        let edit = db.insert_local_file(&track("renamed.mp3")).unwrap();
        let matched = db.insert_local_file(&track("native.mp3")).unwrap();
        db.protect_imported_track(matched).unwrap();
        db.conn
            .execute("UPDATE tracks SET title='Renamed' WHERE id=?1", [edit])
            .unwrap();
        db.conn.execute_batch("CREATE TRIGGER test_ignore BEFORE INSERT ON tracks WHEN NEW.path='ignored.mp3' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(db.insert_local_file(&track("ignored.mp3")).is_err());
        let p = protections(&db).unwrap();
        assert!(p
            .native
            .is_superset(&[foreign, unreadable, edit, matched, linked].into()));
        assert!(!p.native.contains(&raw));
    }

    #[test]
    fn migration_leaves_older_tracks_review_only() {
        let (_dir, path, db) = collection();
        let old = db.insert_track(&track("old.mp3")).unwrap();
        db.conn.execute_batch("DROP TRIGGER protect_changed_import; DROP TABLE direct_file_imports; PRAGMA user_version=3;").unwrap();
        drop(db);
        let upgraded = Db::open(&path).unwrap();
        assert!(protections(&upgraded).unwrap().native.contains(&old));
    }

    #[test]
    fn edit_and_stem_sources_stay_protected_after_unmatching() {
        let (_dir, _path, db) = collection();
        let unrelated = db.insert_local_file(&track("unrelated.wav")).unwrap();
        for (source_name, child_name, stem) in [
            ("song-a.wav", "song-a-edit.wav", false),
            ("song-b.wav", "song-b-vocals.wav", true),
        ] {
            let source = db.insert_local_file(&track(source_name)).unwrap();
            let child = db.insert_track(&track(child_name)).unwrap();
            assert!(!protections(&db).unwrap().prepared.contains(&source));
            let (key, value) = if stem {
                (
                    crate::stem_export::link_key(child),
                    format!("{source}:vocals"),
                )
            } else {
                (crate::ui_edit::edited_key(child), source.to_string())
            };
            db.set_setting_checked(&key, &value).unwrap();
            db.set_match(source, child, true).unwrap();
            db.set_match(source, child, false).unwrap();
            let checked = protections(&db).unwrap();
            assert!(checked.prepared.contains(&source) && checked.prepared.contains(&child));
            assert!(!checked.prepared.contains(&unrelated));
        }
    }

    #[test]
    fn relations_smart_playlists_and_path_backed_settings_are_protected() {
        let (_dir, _path, db) = collection();
        let original = db.insert_local_file(&track("backed.mp3")).unwrap();
        let phrase = db.insert_local_file(&track("phrase.mp3")).unwrap();
        let history = db.insert_local_file(&track("played.mp3")).unwrap();
        let smart = db
            .insert_local_file(&Track {
                genre: "house".into(),
                ..track("smart.mp3")
            })
            .unwrap();
        let unused = db.insert_local_file(&track("unused.mp3")).unwrap();
        db.set_setting_checked(
            "prefs",
            r#"{"sampler":{"slots":[{"track":9999,"path":"BACKED.MP3"}]}}"#,
        )
        .unwrap();
        let mut rule = crate::smart::SmartRule::default();
        rule.conditions[0].value = "house".into();
        db.create_node(0, "house", NodeKind::Smart, Some(&rule))
            .unwrap();
        db.set_setting_checked(&format!("phrase_edit:{phrase}"), "[]")
            .unwrap();
        db.conn
            .execute("INSERT INTO histories VALUES (1,'Set','2026-11-22')", [])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO history_entries(history_id,track_id) VALUES (1,?1)",
                [history],
            )
            .unwrap();
        let p = protections(&db).unwrap();
        for id in [original, phrase, history, smart] {
            assert!(p.is_prepared(&db.track(id).unwrap()), "Track {id}");
        }
        assert!(!p.is_prepared(&db.track(unused).unwrap()));
        db.set_setting_checked("hot_cue_banks", "?").unwrap();
        assert!(protections(&db).is_err());
    }

    #[test]
    fn moved_references_have_a_pre_write_backup_and_can_be_restored_from_trash() {
        let (dir, path, db) = collection();
        let audio = dir.path().join("A.mp3");
        fs::write(&audio, b"unchanged audio").unwrap();
        let metadata = track(audio.to_str().unwrap());
        let id = db.insert_local_file(&metadata).unwrap();
        let other = db.insert_local_file(&track("B.mp3")).unwrap();
        let reviews = Cell::new(0);
        let archive = move_reviewed(&db, &path, &[id], || {
            reviews.set(reviews.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(reviews.get(), 2);
        assert_eq!(fs::read(&audio).unwrap(), b"unchanged audio");
        assert_eq!(backup::read_manifest(&archive).unwrap().tracks, 2);
        verify_backup(&archive, dir.path(), 2).unwrap();
        let restored = backup::stage_restore(
            &archive,
            &dir.path().join("staging"),
            &dir.path().join("artwork"),
            true,
            &|_, _, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let recovery = Db {
            conn: Connection::open_with_flags(restored.db, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap(),
        };
        assert_eq!(recovery.track(id).unwrap().path, metadata.path);
        assert!(recovery.trash_list().unwrap().is_empty());
        assert!(protections(&recovery).unwrap().native.contains(&id));
        assert!(db.track(id).is_none());
        assert!(db.track(other).is_some());
        let trash = db.trash_list().unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(db.restore_trash(trash[0].0).unwrap(), Some(id));
        assert!(protections(&db).unwrap().native.contains(&id));
    }

    #[test]
    fn stale_evidence_or_partial_failure_roll_back_the_entire_batch() {
        let (_dir, path, db) = collection();
        let a = db.insert_local_file(&track("A.mp3")).unwrap();
        let b = db.insert_local_file(&track("B.mp3")).unwrap();
        let calls = Cell::new(0);
        let stale = move_reviewed(&db, &path, &[a, b], || {
            calls.set(calls.get() + 1);
            ensure!(calls.get() == 1, "A file appeared during backup");
            Ok(())
        });
        assert!(stale.is_err());
        assert_eq!(calls.get(), 2);
        db.conn.execute_batch("CREATE TRIGGER test_refuse BEFORE DELETE ON tracks WHEN OLD.path='B.mp3' BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
        let failed = move_reviewed(&db, &path, &[a, b], || Ok(()));
        assert!(failed.unwrap_err().to_string().contains("test"));
        assert!(db.track(a).is_some() && db.track(b).is_some());
        assert!(db.trash_list().unwrap().is_empty());
    }

    #[test]
    fn backup_failure_and_bulk_or_unconfirmed_requests_are_refused() {
        let (dir, path, db) = collection();
        let a = db.insert_local_file(&track("A.mp3")).unwrap();
        fs::write(dir.path().join("cleanup-backups"), b"reserved").unwrap();
        assert!(move_reviewed(&db, &path, &[a], || Ok(())).is_err());
        assert!(db.track(a).is_some());
        assert!(check_confirmation(0, true, "MOVE 0").is_err());
        assert!(check_confirmation(
            MAX_SELECTION + 1,
            true,
            &format!("MOVE {}", MAX_SELECTION + 1)
        )
        .is_err());
        assert!(check_confirmation(1, false, "MOVE 1").is_err());
        assert!(check_confirmation(2, true, "MOVE 1").is_err());
        assert!(check_confirmation(2, true, "MOVE 2").is_ok());
    }
}
