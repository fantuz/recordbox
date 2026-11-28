//! Exercise the two desktop sync directions against a real encrypted, disposable database.
use super::*;
use crate::{db::Db, model::parse_key, rbimport};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub(super) struct Libraries {
    _root: tempfile::TempDir,
    pub(super) local: PathBuf,
    pub(super) source: PathBuf,
    pub(super) audio: String,
}

fn native(file: &Path) -> Result<Connection> {
    let db = Connection::open(file)?;
    db.pragma_update(None, "key", rbimport::MASTER_DB_KEY)?;
    Ok(db)
}

impl Libraries {
    pub(super) fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let dir = root.path().canonicalize()?;
        let source = dir.join("desktop");
        let profile = dir.join("profile");
        let audio = dir.join("music.wav");
        std::fs::create_dir_all(source.join("share"))?;
        std::fs::create_dir(&profile)?;
        std::fs::write(&audio, b"opaque audio content")?;
        let audio = audio.to_string_lossy().into_owned();
        let local = profile.join("library.db");
        let db = native(&source.join("master.db"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE djmdContent(
                ID TEXT PRIMARY KEY, UUID TEXT, FolderPath TEXT, Title TEXT, Rating INTEGER,
                FileType INTEGER DEFAULT 1, ColorID INTEGER DEFAULT 0, DJPlayCount INTEGER DEFAULT 0,
                BPM INTEGER, KeyID TEXT, Length INTEGER DEFAULT 12, SampleRate INTEGER DEFAULT 44100,
                AnalysisDataPath TEXT DEFAULT '', AnalysisUpdated TEXT DEFAULT '1', CueUpdated INTEGER DEFAULT 0,
                ServiceID INTEGER DEFAULT 0, CloudID TEXT, rb_local_deleted INTEGER DEFAULT 0,
                rb_local_usn INTEGER DEFAULT 1, rb_local_synced INTEGER DEFAULT 1, updated_at TEXT DEFAULT 'first');
             CREATE TABLE djmdCue(ID TEXT PRIMARY KEY, UUID TEXT, ContentID TEXT, ContentUUID TEXT,
                InMsec INTEGER, OutMsec INTEGER, Kind INTEGER, Color INTEGER, ActiveLoop INTEGER, Comment TEXT,
                rb_local_synced INTEGER, rb_local_usn INTEGER, updated_at TEXT, rb_local_deleted INTEGER);
             CREATE TABLE contentCue(ID TEXT PRIMARY KEY, UUID TEXT, ContentID TEXT, Cues TEXT,
                rb_cue_count INTEGER, rb_local_deleted INTEGER, updated_at TEXT, rb_local_synced INTEGER, rb_local_usn INTEGER);
             CREATE TABLE contentFile(ID TEXT PRIMARY KEY, ContentID TEXT, Path TEXT, Hash TEXT, Size INTEGER,
                rb_local_path TEXT, rb_local_deleted INTEGER, updated_at TEXT, rb_local_synced INTEGER, rb_local_usn INTEGER,
                rb_insync_hash, rb_insync_local_usn, rb_file_hash_dirty, rb_local_file_status, rb_in_progress,
                rb_process_type, rb_temp_path, rb_file_size_dirty, rb_data_status, rb_local_data_status, usn);
             CREATE TABLE djmdKey(ID TEXT PRIMARY KEY, ScaleName TEXT, rb_local_deleted INTEGER DEFAULT 0);
             INSERT INTO djmdKey(ID,ScaleName) VALUES ('1','Am'),('2','Gm');
             CREATE TABLE agentRegistry(registry_id TEXT PRIMARY KEY, int_1 INTEGER, updated_at TEXT);
             INSERT INTO agentRegistry VALUES ('localUpdateCount',1,'first');
             CREATE TABLE djmdMyTag(ID,Name,ParentID,Seq,Attribute,rb_local_deleted);
             CREATE TABLE djmdSongMyTag(ContentID,MyTagID,rb_local_deleted);
             CREATE TABLE djmdPlaylist(ID,Seq,Name,Attribute,ParentID,SmartList,rb_local_deleted);
             CREATE TABLE djmdSongPlaylist(PlaylistID,ContentID,TrackNo,rb_local_deleted);",
        )?;
        for table in ["djmdArtist", "djmdAlbum", "djmdGenre", "djmdLabel"] {
            db.execute(
                &format!("CREATE TABLE {table}(ID,Name,rb_local_deleted)"),
                [],
            )?;
        }
        db.execute(
            "INSERT INTO djmdContent(ID,UUID,FolderPath,Title,Rating,BPM,KeyID)
             VALUES('10','f0214d3b-d66c-46a1-8927-04de39519c39',?1,'Original',2,12000,'1')",
            [&audio],
        )?;
        drop(db);
        let rb = Db::open(&local)?;
        let imported = rbimport::load_from(&source, &profile)?;
        rbimport::merge::apply(&rb, &imported)?;
        Ok(Self {
            _root: root,
            local,
            source,
            audio,
        })
    }

    fn edit(&self) -> Result<()> {
        let rb = Db::open(&self.local)?;
        let mut track = rb.track_at_path(&self.audio)?.unwrap();
        track.rating = 4;
        track.bpm = 127.25;
        track.set_key(parse_key("Gm"));
        track.key_manual = true;
        rb.update_track(&track)?;
        Ok(())
    }

    fn state(&self, file: &Path) -> Result<(i64, i64, String, String, i64)> {
        Ok(native(file)?.query_row(
            "SELECT Rating,BPM,KeyID,Title,(SELECT int_1 FROM agentRegistry WHERE registry_id='localUpdateCount')
             FROM djmdContent WHERE ID='10'", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))
        )?)
    }
}

#[test]
fn native_catalog_only_opt_out_applies_reviewed_delta_without_a_recovery_copy() -> Result<()> {
    let f = Libraries::new()?;
    f.edit()?;
    let plan = preview(&f.local, &f.source)?;
    let row = plan.rows.iter().position(|r| r.reason.is_none() && r.field == Field::Rating).unwrap();
    let receipt = apply_with_backup(&f.local, &f.source, &plan, &[row], false)?;
    assert_eq!(receipt.applied, 1);
    assert!(receipt.backup.is_none() && !backup_root(&f.local)?.exists());
    assert_eq!(f.state(&f.source.join("master.db"))?.0, 4);
    assert_eq!(std::fs::read(&f.audio)?, b"opaque audio content");
    let next = preview(&f.local, &f.source)?;
    assert!(!next.rows.iter().any(|r| r.field == Field::Rating && r.reason.is_none()));
    Ok(())
}

#[test]
fn import_preview_and_write_selected_native_fields_once() -> Result<()> {
    let f = Libraries::new()?;
    let master = f.source.join("master.db");
    let before = f.state(&master)?;
    f.edit()?;
    let plan = preview(&f.local, &f.source)?;
    assert_eq!(
        f.state(&master)?,
        before,
        "a preview must leave the desktop data untouched"
    );
    let selected: Vec<_> = plan
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| row.reason.is_none().then_some(i))
        .collect();
    assert_eq!(
        selected.len(),
        3,
        "rating, BPM and key should have independently reviewable changes"
    );
    let receipt = apply(&f.local, &f.source, &plan, &selected)?;
    assert_eq!(receipt.applied, 3);
    assert_eq!(f.state(&receipt.backup.as_ref().expect("retained by default").join("master.db"))?, before);
    let (rating, bpm, key, title, usn) = f.state(&master)?;
    assert_eq!(
        (rating, bpm, key.as_str(), title.as_str()),
        (4, 12725, "2", "Original")
    );
    assert!(usn > before.4);
    assert!(!preview(&f.local, &f.source)?
        .rows
        .iter()
        .any(|row| row.reason.is_none()));
    assert_eq!(std::fs::read(&f.audio)?, b"opaque audio content");
    Ok(())
}

#[test]
fn a_desktop_change_after_review_aborts_the_selected_batch() -> Result<()> {
    let f = Libraries::new()?;
    let master = f.source.join("master.db");
    f.edit()?;
    let plan = preview(&f.local, &f.source)?;
    let selected: Vec<_> = plan
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| row.reason.is_none().then_some(i))
        .collect();
    assert_eq!(selected.len(), 3);
    native(&master)?.execute(
        "UPDATE djmdContent SET Rating=?1 WHERE ID=?2",
        params![5, "10"],
    )?;
    let now = f.state(&master)?;
    assert!(apply(&f.local, &f.source, &plan, &selected).is_err());
    assert_eq!(f.state(&master)?, now);
    Ok(())
}

#[test]
fn registered_grid_nudge_rechecks_ownership_and_backs_up_both_tables() -> Result<()> {
    use crate::{export::anlz, model::Beat};
    use md5::{Digest, Md5};

    let f = Libraries::new()?;
    let analysis = "/PIONEER/USBANLZ/P001/ANLZ0000.DAT";
    let share = f.source.join("share");
    let dat = share.join(analysis.trim_start_matches('/'));
    std::fs::create_dir_all(dat.parent().unwrap())?;
    let master = f.source.join("master.db");
    let db = native(&master)?;
    db.execute(
        "UPDATE djmdContent SET AnalysisDataPath=? WHERE ID='10'",
        [analysis],
    )?;
    let beats: Vec<_> = (0..4)
        .map(|index| Beat {
            index,
            beat: index as u8 + 1,
            time: 31.946 + 0.5 * index as f64,
            bpm: if index < 2 { 120.00 } else { 120.01 },
        })
        .collect();
    let mut files = anlz::build(&f.audio, &beats, &Default::default(), &[], 0, 44100);
    let rev = files.ext.windows(4).position(|tag| tag == b"PQT2").unwrap() + 16;
    files.ext[rev..rev + 4].copy_from_slice(&0x0200_0002u32.to_be_bytes());
    let siblings = [
        ("DAT", files.dat),
        ("EXT", files.ext),
        ("2EX", files.ex2),
        ("3EX", embedding::test_embedding(b"0")),
    ];
    for (ext, bytes) in &siblings {
        std::fs::write(dat.with_extension(ext), bytes)?;
        db.execute(
            "INSERT INTO contentFile(ID,ContentID,Path,Hash,Size,rb_local_deleted,rb_local_synced)
            VALUES(?1,'10',?2,?3,?4,0,0)",
            params![
                ext,
                format!("/PIONEER/USBANLZ/P001/ANLZ0000.{ext}"),
                format!("{:x}", Md5::digest(bytes)),
                i64::try_from(bytes.len())?
            ],
        )?;
    }
    drop(db);
    let rb = Db::open(&f.local)?;
    rbimport::merge::apply(
        &rb,
        &rbimport::load_from(&f.source, f.local.parent().unwrap())?,
    )?;
    let id = rb.track_at_path(&f.audio)?.unwrap().id;
    let mut grid = rb.grid_checked(id)?.context("expected an imported grid")?;
    assert_eq!(grid.anchors.len(), 2);
    for anchor in &mut grid.anchors {
        anchor.time += 0.001;
    }
    rb.save_grid(id, &grid)?;
    // A valid persisted JSON representation can differ from its reserialization.
    rb.conn.execute("UPDATE native_edit_baseline SET value=' '||value WHERE track_id=? AND field='Beat grid'", [id])?;
    drop(rb);
    let plan = preview(&f.local, &f.source)?;
    let i = plan
        .rows
        .iter()
        .position(|r| r.field == Field::Grid)
        .unwrap();
    assert_eq!(plan.rows[i].reason, None);

    // Another owner claiming the unassigned embedding between review and apply
    // invalidates the selected changes, even when its bytes still match.
    native(&master)?.execute(
        "INSERT INTO contentFile(ID,ContentID,Path,rb_local_deleted)
        SELECT 'second','20',Path,0 FROM contentFile WHERE ID='3EX'",
        [],
    )?;
    assert!(apply(&f.local, &f.source, &plan, &[i]).is_err());
    for (ext, original) in &siblings {
        assert_eq!(&std::fs::read(dat.with_extension(ext))?, original);
    }
    native(&master)?.execute("DELETE FROM contentFile WHERE ID='second'", [])?;
    let receipt = apply(&f.local, &f.source, &plan, &[i])?;
    assert_eq!(receipt.applied, 1);
    let backups = [
        std::fs::read(receipt.backup.as_ref().expect("retained by default").join("0.original"))?,
        std::fs::read(receipt.backup.as_ref().expect("retained by default").join("1.original"))?,
    ];
    let current = native(&master)?;
    let old = native(&receipt.backup.as_ref().expect("retained by default").join("master.db"))?;
    for (ext, original) in &siblings {
        let actual = std::fs::read(dat.with_extension(ext))?;
        let registered = |c: &Connection| {
            c.query_row(
                "SELECT Hash,Size FROM contentFile WHERE ID=?",
                [ext],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
        };
        assert_eq!(
            registered(&current)?,
            (format!("{:x}", Md5::digest(&actual)), actual.len() as i64)
        );
        assert_eq!(
            registered(&old)?,
            (
                format!("{:x}", Md5::digest(original)),
                original.len() as i64
            )
        );
        if matches!(*ext, "DAT" | "EXT") {
            assert_ne!(&actual, original);
            assert!(backups.contains(original));
        } else {
            assert_eq!(&actual, original);
        }
    }
    assert!(crate::app::grids_match(
        &grid::inspect(&share, analysis)?.grid,
        &grid
    ));
    assert!(preview(&f.local, &f.source)?.rows.is_empty());

    // Simulate the native change being saved before Recordbox acknowledges it.
    // The floating-point sum 31.946 + 0.001 differs from native 31947/1000.
    let base = &plan.rows[i].baseline;
    Db::open(&f.local)?.conn.execute(
        "UPDATE native_edit_baseline SET value=?1, proof=?2 WHERE track_id=?3 AND field='Beat grid'",
        params![base.stored_value, base.proof, id],
    )?;
    let repeated = preview(&f.local, &f.source)?;
    assert_eq!(repeated.rows.len(), 1);
    assert_eq!(repeated.rows[0].reason.as_deref(), Some("Already applied"));
    Ok(())
}

/// Optional round trip using pyrekordbox's public v6 catalog and matching analysis bytes.
/// All path changes, media placeholders and writes are confined to a new temporary directory.
#[test]
#[ignore = "set RECORDBOX_TEST_PYREKORDBOX to a checkout containing its public rekordbox 6 fixture"]
fn published_v6_catalog_keeps_native_anlz_registrations_in_sync() -> Result<()> {
    use md5::{Digest, Md5};
    let checkout = PathBuf::from(
        std::env::var_os("RECORDBOX_TEST_PYREKORDBOX").context("fixture checkout missing")?,
    );
    let original = checkout.join(".testdata/rekordbox 6");
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let source = root.join("desktop");
    let profile = root.join("profile");
    std::fs::create_dir(&source)?;
    std::fs::create_dir(&profile)?;
    let active = source.join("master.db");
    std::fs::copy(original.join("master_locked.db"), &active)?;
    let db = native(&active)?;
    // The public backup has different folder IDs from its example database.
    // Match each sidecar by its original, registered digest.
    let mut by_md5 = HashMap::new();
    for entry in walkdir::WalkDir::new(original.join("backup/share")) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let bytes = std::fs::read(entry.path())?;
            by_md5.insert(format!("{:x}", Md5::digest(&bytes)), entry.into_path());
        }
    }
    let records: Vec<(String,String,String,String)> = db.prepare(
        "SELECT ID,ContentID,Path,Hash FROM contentFile WHERE rb_local_deleted=0 ORDER BY ContentID,Path"
    )?.query_map([],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?
      .collect::<rusqlite::Result<_>>()?;
    let owner = records
        .iter()
        .map(|r| r.1.as_str())
        .find(|id| {
            let group = records.iter().filter(|r| r.1 == *id).collect::<Vec<_>>();
            group.len() == 3
                && group.iter().all(|r| by_md5.contains_key(&r.3))
                && group.iter().any(|r| {
                    r.2.ends_with(".DAT")
                        && rbimport::read_pqtz_grid(&by_md5[&r.3]).is_some_and(|g| !g.is_empty())
                })
        })
        .context("fixture lacks a complete registered analysis set")?
        .to_owned();
    for (id, _, path, hash) in records.iter().filter(|r| r.1 == owner) {
        let target = source.join("share").join(path.trim_start_matches('/'));
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::copy(&by_md5[hash], &target)?;
        db.execute(
            "UPDATE contentFile SET rb_local_path=?1 WHERE ID=?2",
            params![target.to_string_lossy().as_ref(), id],
        )?;
    }
    let media = root.join("fixture.mp3");
    std::fs::copy(checkout.join(".testdata/empty.mp3"), &media)?;
    let path = media.to_str().unwrap();
    db.execute(
        "UPDATE djmdContent SET FolderPath=?1 WHERE ID=?2",
        params![path, owner],
    )?;
    let dat: String = db.query_row(
        "SELECT AnalysisDataPath FROM djmdContent WHERE ID=?",
        [&owner],
        |r| r.get(0),
    )?;
    drop(db);

    let dest = profile.join("library.db");
    let rb = Db::open(&dest)?;
    rbimport::merge::apply(&rb, &rbimport::load_from(&source, &profile)?)?;
    let mut t = rb
        .track_at_path(path)?
        .context("fixture track was not imported")?;
    t.rating = 4;
    rb.update_track(&t)?;
    let mut desired = rb.grid_checked(t.id)?.context("missing source grid")?;
    for a in &mut desired.anchors {
        a.time += 0.010;
        a.beat = (a.beat % 4) + 1;
    }
    rb.save_grid(t.id, &desired)?;
    let reviewed = preview(&dest, &source)?;
    let chosen: Vec<_> = reviewed
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            (r.reason.is_none() && matches!(r.field, Field::Grid | Field::Rating)).then_some(i)
        })
        .collect();
    assert_eq!(chosen.len(), 2, "{:#?}", reviewed.rows);
    let receipt = apply(&dest, &source, &reviewed, &chosen)?;
    assert_eq!(receipt.applied, 2);
    assert!(crate::app::grids_match(
        &grid::inspect(&source.join("share"), &dat)?.grid,
        &desired
    ));
    let db = native(&active)?;
    let old = native(&receipt.backup.as_ref().expect("retained by default").join("master.db"))?;
    assert_eq!(
        db.query_row("SELECT Rating FROM djmdContent WHERE ID=?", [&owner], |r| r
            .get::<_, i64>(0))?,
        4
    );
    assert_eq!(
        old.query_row("SELECT Rating FROM djmdContent WHERE ID=?", [&owner], |r| r
            .get::<_, i64>(0))?,
        0
    );
    let mut changed = 0;
    for (id, _, path, old_hash) in records.iter().filter(|r| r.1 == owner) {
        let (hash, size): (String, i64) =
            db.query_row("SELECT Hash,Size FROM contentFile WHERE ID=?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        let bytes = std::fs::read(source.join("share").join(path.trim_start_matches('/')))?;
        assert_eq!(hash, format!("{:x}", Md5::digest(&bytes)));
        assert_eq!(size, i64::try_from(bytes.len())?);
        changed += usize::from(hash != *old_hash);
    }
    assert_eq!(
        changed, 2,
        "Both DAT and EXT registrations should reflect the rotated beat numbering"
    );
    Ok(())
}
