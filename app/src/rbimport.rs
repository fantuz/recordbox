//! Reading the user's existing rekordbox library (master.db).
//!
//! During desktop sync, Recordbox copies the main database and its write-ahead log into
//! a private temporary directory. A change to either source file during capture triggers
//! another attempt. SQLite reads the validated copy before Recordbox merges the result into
//! its own collection.

#[cfg(test)]
mod grid_oracle;
pub(crate) mod merge;
pub(crate) mod writeback;

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// SQLCipher passphrase of rekordbox 6/7's master.db. It is the same for every installation and
/// has been public for years (pyrekordbox and other library tools use it).
const MASTER_DB_KEY: &str = "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497";

/// Default desktop source shown in the review, including when rekordbox is absent.
pub fn default_library_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let d = dirs::home_dir()?.join("Library/Pioneer/rekordbox");
    #[cfg(target_os = "windows")]
    let d = dirs::data_dir()?.join("Pioneer/rekordbox");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let d = dirs::data_dir()?.join("Pioneer/rekordbox");
    Some(d)
}

/// Where rekordbox keeps its library on this machine, if it is installed.
pub fn library_dir() -> Option<PathBuf> {
    default_library_dir().filter(|dir| dir.join("master.db").is_file())
}

fn stamp(path: &Path) -> std::io::Result<Option<(u64, SystemTime)>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some((metadata.len(), metadata.modified()?))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Capture the main file and WAL in a unique directory kept alive by the caller.
fn snapshot(src_dir: &Path, dest_dir: &Path) -> Result<tempfile::TempDir> {
    let src = src_dir.canonicalize()?;
    let parent = std::path::absolute(dest_dir)?;
    let existing = parent.ancestors().find(|p| p.exists()).context("finding the snapshot directory")?;
    anyhow::ensure!(!existing.canonicalize()?.starts_with(&src), "the snapshot directory must be outside the source library");
    std::fs::create_dir_all(&parent)?;
    let versions = || -> Result<_> {
        anyhow::ensure!(!stamp(&src.join("master.db-journal"))?.is_some_and(|(len, _)| len > 0),
            "the source has a nonempty rollback journal; close rekordbox cleanly before syncing");
        Ok((stamp(&src.join("master.db"))?.context("source master.db is missing")?,
            stamp(&src.join("master.db-wal"))?))
    };
    for attempt in 0..5 {
        let copy = tempfile::Builder::new().prefix("sync-").tempdir_in(&parent)?;
        let before = versions()?;
        let result = (|| -> std::io::Result<()> {
            std::fs::copy(src.join("master.db"), copy.path().join("master.db"))?;
            if before.1.is_some_and(|(len, _)| len > 0) {
                std::fs::copy(src.join("master.db-wal"), copy.path().join("master.db-wal"))?;
            }
            Ok(())
        })();
        if before == versions()? {
            result.context("copying the source library into Recordbox's snapshot directory")?;
            return Ok(copy);
        }
        std::thread::sleep(std::time::Duration::from_millis(200 * (attempt + 1)));
    }
    Err(anyhow!("rekordbox kept changing its library while it was being copied; try again in a moment"))
}

/// Open a snapshot copy made by [`snapshot`]. The connection is read-only, but SQLite still
/// needs to read the copied WAL, so the copy (never the original) must be in a writable folder.
pub fn open_snapshot(db: &Path) -> Result<rusqlite::Connection> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("opening {}", db.display()))?;
    c.pragma_update(None, "key", MASTER_DB_KEY)?;
    // Fails with "file is not a database" when the key or format does not match.
    let n: i64 = c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0)).context("the rekordbox library could not be read (unsupported version?)")?;
    if n == 0 {
        return Err(anyhow!("the rekordbox library is empty or unreadable"));
    }
    // A copy taken while rekordbox was writing could be inconsistent: check it first.
    // Full integrity check (also verifies index contents, unlike quick_check).
    let check: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap_or_default();
    if check != "ok" {
        return Err(anyhow!("the copy of the rekordbox library failed its integrity check ({check}); try again with rekordbox closed"));
    }
    check_schema(&c)?;
    Ok(c)
}

/// The tables and columns the importer reads by name. History tables are optional.
const REQUIRED_SCHEMA: &[(&str, &[&str])] = &[
    ("djmdContent", &["ID", "FolderPath", "AnalysisDataPath", "FileType", "Rating", "ColorID", "DJPlayCount", "BPM", "rb_local_deleted"]),
    ("djmdCue", &["ContentID", "InMsec", "OutMsec", "Kind", "Color", "ActiveLoop", "Comment", "updated_at", "rb_local_deleted"]),
    ("djmdKey", &["ID", "ScaleName", "rb_local_deleted"]),
    ("djmdMyTag", &["ID", "Name", "ParentID", "Seq", "Attribute", "rb_local_deleted"]),
    ("djmdSongMyTag", &["ContentID", "MyTagID", "rb_local_deleted"]),
    ("djmdPlaylist", &["ID", "Seq", "Name", "Attribute", "ParentID", "SmartList", "rb_local_deleted"]),
    ("djmdSongPlaylist", &["PlaylistID", "ContentID", "TrackNo", "rb_local_deleted"]),
];

/// Check, before anything is imported, that the library has the layout the importer was
/// written for (LIB-015). A rekordbox update that renames or drops something then gives one
/// clear message naming what is missing, not a half-imported library or an SQL error.
pub fn check_schema(c: &rusqlite::Connection) -> Result<()> {
    let mut missing = vec![];
    for (table, cols) in REQUIRED_SCHEMA {
        let have: Vec<String> = c
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .and_then(|mut st| st.query_map([], |r| r.get::<_, String>(0)).map(|it| it.flatten().collect()))
            .unwrap_or_default();
        if have.is_empty() {
            missing.push(table.to_string());
            continue;
        }
        for col in cols.iter() {
            if !have.iter().any(|h| h.eq_ignore_ascii_case(col)) {
                missing.push(format!("{table}.{col}"));
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("this rekordbox library has a layout recordbox does not know (missing: {}); nothing was imported", missing.join(", ")))
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn desktop_sync_reads_committed_encrypted_wal_without_changing_source_files() -> Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        std::fs::create_dir(&source)?;
        let db = rusqlite::Connection::open(source.join("master.db"))?;
        db.pragma_update(None, "key", MASTER_DB_KEY)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")?;
        for (table, cols) in REQUIRED_SCHEMA {
            db.execute(&format!("CREATE TABLE {table} ({})", cols.join(", ")), [])?;
        }
        for table in ["djmdArtist", "djmdAlbum", "djmdGenre", "djmdLabel"] {
            db.execute(&format!("CREATE TABLE {table} (ID, Name, rb_local_deleted)"), [])?;
        }
        db.execute_batch("ALTER TABLE djmdContent ADD COLUMN Title; PRAGMA wal_checkpoint(TRUNCATE);")?;
        let song = root.path().join("fixture.wav");
        std::fs::write(&song, b"fixture")?;
        db.execute("INSERT INTO djmdContent(ID,FolderPath,Title,rb_local_deleted) VALUES('1',?1,'In the WAL',0)", [song.to_string_lossy().as_ref()])?;
        let bytes = || ["master.db", "master.db-wal", "master.db-shm"].map(|name| std::fs::read(source.join(name)).unwrap());
        let before = bytes();
        assert!(!before[1].is_empty());
        let snapshot_root = root.path().join("copies");
        let first = snapshot(&source, &snapshot_root)?;
        let second = snapshot(&source, &snapshot_root)?;
        assert_ne!(first.path(), second.path());
        let copy = open_snapshot(&first.path().join("master.db"))?;
        let lib = read_library(&copy, &source.join("share"))?;
        assert_eq!(lib.tracks.len(), 1);
        assert_eq!(lib.tracks[0].xt.track.title, "In the WAL");
        assert_eq!(before, bytes());
        drop(copy);
        drop((first, second));
        assert_eq!(std::fs::read_dir(snapshot_root)?.count(), 0);
        Ok(())
    }

    #[test]
    fn desktop_snapshot_rejects_source_dest_overlap_and_nonempty_rollback_journal() -> Result<()> {
        let root = tempfile::tempdir()?;
        let src = root.path().join("source");
        std::fs::create_dir(&src)?;
        std::fs::write(src.join("master.db"), b"source bytes")?;
        assert!(snapshot(&src, &src).is_err());
        assert!(snapshot(&src, &src.join("copy")).is_err());
        assert!(!src.join("copy").exists());
        #[cfg(unix)]
        {
            let alias = root.path().join("alias");
            std::os::unix::fs::symlink(&src, &alias)?;
            assert!(snapshot(&src, &alias).is_err());
        }
        std::fs::write(src.join("master.db-journal"), b"pending rollback")?;
        assert!(snapshot(&src, &root.path().join("copy")).unwrap_err().to_string().contains("rollback journal"));
        assert_eq!(std::fs::read(src.join("master.db"))?, b"source bytes");
        Ok(())
    }
}

#[cfg(test)]
mod schema_tests {
    #[test]
    fn optional_histories_distinguish_absent_from_unreadable() -> anyhow::Result<()> {
        let db = rusqlite::Connection::open_in_memory()?;
        for (table, columns) in super::REQUIRED_SCHEMA {
            db.execute(&format!("CREATE TABLE {table} ({})", columns.join(", ")), [])?;
        }
        for table in ["djmdArtist", "djmdAlbum", "djmdGenre", "djmdLabel"] {
            db.execute(&format!("CREATE TABLE {table}(ID, Name, rb_local_deleted)"), [])?;
        }
        let share = std::path::Path::new(".");
        assert!(super::read_library(&db, share)?.histories.is_empty());
        db.execute_batch("CREATE TABLE djmdHistory(ID, Seq, Name, Attribute, ParentID, rb_local_deleted);
                          INSERT INTO djmdHistory VALUES('night', 1, 'Club', 0, 'root', 0);")?;
        let partial = super::read_library(&db, share).err().expect("history membership table is missing");
        assert!(format!("{partial:#}").contains("djmdSongHistory"));
        db.execute_batch("CREATE TABLE djmdSongHistory(HistoryID, ContentID, TrackNo, rb_local_deleted);
                          INSERT INTO djmdSongHistory VALUES('night', 'b', 2, 0), ('night', 'a', 1, 0);")?;
        let library = super::read_library(&db, share)?;
        assert_eq!(library.histories.len(), 1);
        assert_eq!(library.histories[0].track_keys, ["a", "b"]);
        db.execute("ALTER TABLE djmdSongHistory RENAME COLUMN TrackNo TO Other", [])?;
        assert!(super::read_library(&db, share).is_err());
        Ok(())
    }

    #[test]
    fn schema_check_names_what_is_missing() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        for (t, cols) in super::REQUIRED_SCHEMA {
            c.execute(&format!("CREATE TABLE {t} ({})", cols.join(", ")), []).unwrap();
        }
        assert!(super::check_schema(&c).is_ok());
        c.execute("ALTER TABLE djmdCue RENAME COLUMN InMsec TO InMs", []).unwrap();
        c.execute("DROP TABLE djmdKey", []).unwrap();
        let e = super::check_schema(&c).unwrap_err().to_string();
        assert!(e.contains("djmdCue.InMsec") && e.contains("djmdKey") && !e.contains("djmdPlaylist"), "{e}");
    }
}


use crate::importer::{XmlNode, XmlTrack};
use crate::model::*;
use rusqlite::types::Value;
use std::collections::HashMap;

/// One rekordbox track as Recordbox needs it, plus what is used to notice later changes.
pub struct RbTrack {
    pub rb_id: String,
    native: Option<writeback::ImportTrack>,
    /// Changes whenever rekordbox changes the track, its cues or its My Tags.
    pub fingerprint: String,
    pub xt: XmlTrack,
    /// (category, tag) My Tag assignments.
    pub tags: Vec<(String, String)>,
    /// rekordbox's own analysis files for this track, without extension (".../ANLZ0000").
    pub anlz_base: Option<std::path::PathBuf>,
}

pub struct RbLibrary {
    source: Option<writeback::Source>,
    pub tracks: Vec<RbTrack>,
    /// Playlist tree; `track_keys` are rekordbox content IDs in playlist order.
    pub playlists: Vec<XmlNode>,
    /// My Tag categories in rekordbox's order, each with its tags.
    pub tag_categories: Vec<(String, Vec<String>)>,
    /// Tracks without a local file (streaming services or moved files).
    pub not_local: usize,
    /// Intelligent playlists that use a condition Recordbox has no equivalent for (not copied).
    pub smart_lists: usize,
    /// Play histories (rekordbox's Histories tree); `track_keys` are content IDs.
    pub histories: Vec<XmlNode>,
}

/// The tag placeholder a translated My Tag condition carries until the sync knows Recordbox's
/// tag ids.
pub fn tag_placeholder(cat: &str, tag: &str) -> String {
    format!("\u{1}{cat}\u{1f}{tag}")
}

/// Translate a rekordbox intelligent playlist (djmdPlaylist.SmartList XML: a NODE with
/// LogicalOperator 1 = all / 2 = any and CONDITION children). None when a condition has no
/// Recordbox equivalent, since dropping it would change what the list matches.
pub fn translate_smartlist(xml: &str, tag_of: &HashMap<String, (String, String)>, keys: &HashMap<String, String>) -> Option<crate::smart::SmartRule> {
    use crate::smart::{Condition, Field, Op, SmartRule};
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut rule = SmartRule { match_all: true, conditions: vec![], auto_update: true };
    let mut saw_node = false;
    loop {
        let ev = reader.read_event().ok()?;
        let e = match &ev {
            Event::Start(e) | Event::Empty(e) => e.clone(),
            Event::Eof => break,
            _ => continue,
        };
        let mut a: HashMap<String, String> = HashMap::new();
        for at in e.attributes().flatten() {
            a.insert(String::from_utf8_lossy(at.key.as_ref()).to_string(), at.unescape_value().map(|v| v.to_string()).unwrap_or_default());
        }
        let get = |k: &str| a.get(k).cloned().unwrap_or_default();
        match e.name().as_ref() {
            b"NODE" => {
                saw_node = true;
                rule.match_all = get("LogicalOperator") != "2";
                rule.auto_update = get("AutomaticUpdate") != "0";
            }
            b"CONDITION" => {
                let field = match get("PropertyName").as_str() {
                    "artist" => Field::Artist,
                    "album" => Field::Album,
                    "albumArtist" => Field::AlbumArtist,
                    "originalArtist" => Field::OriginalArtist,
                    "bpm" => Field::Bpm,
                    // rekordbox's "grouping" property is the track colour (ColorID).
                    "grouping" => Field::Color,
                    "comments" => Field::Comments,
                    "producer" => Field::Composer,
                    "stockDate" => Field::DateAdded,
                    "counter" => Field::PlayCount,
                    "fileName" => Field::FileName,
                    "genre" => Field::Genre,
                    "key" => Field::Key,
                    "label" => Field::Label,
                    "mixName" => Field::MixName,
                    "myTag" => Field::MyTag,
                    "rating" => Field::Rating,
                    "dateReleased" => Field::ReleaseDate,
                    "remixedBy" => Field::Remixer,
                    "duration" => Field::Time,
                    "name" => Field::Title,
                    "year" => Field::Year,
                    _ => return None,
                };
                let op = match get("Operator").trim().parse::<u8>().ok()? {
                    1 => Op::Is,
                    2 => Op::IsNot,
                    3 => Op::Greater,
                    4 => Op::Less,
                    5 => Op::InRange,
                    6 => Op::InLast,
                    7 => Op::NotInLast,
                    8 => Op::Contains,
                    9 => Op::NotContains,
                    10 => Op::StartsWith,
                    11 => Op::EndsWith,
                    _ => return None,
                };
                if !Op::for_type(field.ftype()).contains(&op) {
                    return None;
                }
                let (mut value, mut value2) = (get("ValueLeft"), get("ValueRight"));
                match field {
                    Field::MyTag => {
                        // Tag IDs above 2^31 are stored sign-shifted.
                        let id = value.trim().parse::<i64>().ok()?;
                        let id = if id < 0 { id + (1i64 << 32) } else { id };
                        let (c, t) = tag_of.get(&id.to_string())?;
                        value = tag_placeholder(c, t);
                    }
                    Field::Key => {
                        if let Some(name) = keys.get(value.trim()) {
                            value = name.clone();
                        }
                        if crate::model::parse_key(&value) < 0 {
                            return None;
                        }
                    }
                    Field::Bpm => {
                        // Stored as BPM, or ×100 like djmdContent.
                        for v in [&mut value, &mut value2] {
                            if let Ok(x) = v.trim().parse::<f64>() {
                                if x > 1000.0 {
                                    *v = format!("{}", x / 100.0);
                                }
                            }
                        }
                    }
                    _ => {}
                }
                rule.conditions.push(Condition { field, op, value, value2, months: get("ValueUnit") == "month" });
            }
            _ => {}
        }
    }
    (saw_node && !rule.conditions.is_empty()).then_some(rule)
}

fn text(v: Value) -> String {
    match v {
        Value::Text(s) => s,
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        _ => String::new(),
    }
}
fn int(v: &Value) -> i64 {
    match v {
        Value::Integer(i) => *i,
        Value::Real(f) => *f as i64,
        Value::Text(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Run `sql` and return each row as a name -> value map.
fn rows(c: &rusqlite::Connection, sql: &str) -> Result<Vec<HashMap<String, Value>>> {
    let mut st = c.prepare(sql)?;
    let names: Vec<String> = st.column_names().map(|s| s.to_string()).collect();
    let out = st
        .query_map([], |r| {
            let mut m = HashMap::new();
            for (i, n) in names.iter().enumerate() {
                m.insert(n.clone(), r.get::<_, Value>(i)?);
            }
            Ok(m)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

fn names(c: &rusqlite::Connection, table: &str) -> Result<HashMap<String, String>> {
    Ok(rows(c, &format!("SELECT ID, Name FROM {table} WHERE rb_local_deleted = 0"))?
        .into_iter()
        .map(|mut r| (text(r.remove("ID").unwrap_or(Value::Null)), text(r.remove("Name").unwrap_or(Value::Null))))
        .collect())
}

/// Beat grid from rekordbox's own analysis file (the PQTZ section of ANLZxxxx.DAT): one
/// anchor at the first beat and one wherever the tempo changes.
pub fn read_pqtz_grid(dat: &Path) -> Option<BeatGrid> {
    let b = std::fs::read(dat).ok()?;
    parse_pqtz_grid(&b)
}

pub(crate) fn parse_pqtz_grid(b: &[u8]) -> Option<BeatGrid> {
    let mut anchors = vec![];
    let mut last_tempo = None;
    for beat in pqtz_beats(b)? {
        let beat = beat?;
        if Some(beat.bpm) != last_tempo {
            anchors.push(GridAnchor { time: beat.time, bpm: beat.bpm, beat: beat.beat });
            last_tempo = Some(beat.bpm);
        }
    }
    (!anchors.is_empty()).then_some(BeatGrid { anchors })
}

/// Native phrase beat numbers are positions in PQTZ, so reuse them only when the
/// audible grid has the same source records at the same positions.
pub fn pqtz_beat_indices_match(dat: &Path, beats: &[Beat]) -> bool {
    std::fs::read(dat).ok().map(|b| pqtz_indices_match(&b, beats)).unwrap_or(false)
}

fn pqtz_indices_match(b: &[u8], beats: &[Beat]) -> bool {
    pqtz_beats(b).and_then(|source| matching_pqtz_prefix(source, beats)).is_some()
}

/// Edited/inferred phrases can refer to any local beat, including tail beats that
/// a shorter native grid never contained. Check the exact section USB will write.
pub(crate) fn pqtz_matches_all_beats(section: &[u8], beats: &[Beat]) -> bool {
    pqtz_section_beats(section).and_then(|source| matching_pqtz_prefix(source, beats)) == Some(beats.len())
}

fn matching_pqtz_prefix(source: impl Iterator<Item = Option<Beat>>, beats: &[Beat]) -> Option<usize> {
    let mut count = 0;
    for entry in source {
        let source = entry?;
        let local = beats.get(source.index)?;
        if local.index != source.index || local.beat != source.beat || (local.time - source.time).abs() > 0.0011 || (local.bpm - source.bpm).abs() >= 0.01
            || !local.time.is_finite() || !local.bpm.is_finite() {
            return None;
        }
        count += 1;
    }
    (count > 0).then_some(count)
}

fn pqtz_beats(b: &[u8]) -> Option<impl Iterator<Item = Option<Beat>> + '_> {
    if b.get(0..4)? != b"PMAI" {
        return None;
    }
    let be32 = |o: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?)) };
    let mut i = be32(4)? as usize;
    if i < 12 || i > b.len() || be32(8)? as usize != b.len() {
        return None;
    }
    while i < b.len() {
        let header = be32(i.checked_add(4)?)? as usize;
        let total = be32(i + 8)? as usize;
        if header < 12 || total < header {
            return None;
        }
        let section = b.get(i..i.checked_add(total)?)?;
        if section.get(..4)? == b"PQTZ" {
            return pqtz_section_beats(section);
        }
        i += total;
    }
    None
}

fn pqtz_section_beats(section: &[u8]) -> Option<impl Iterator<Item = Option<Beat>> + '_> {
    if section.get(..4)? != b"PQTZ" || section.get(4..8)? != 24u32.to_be_bytes() { return None; }
    let total = u32::from_be_bytes(section.get(8..12)?.try_into().ok()?) as usize;
    let n = u32::from_be_bytes(section.get(20..24)?.try_into().ok()?) as usize;
    if total != section.len() || n.checked_mul(8)?.checked_add(24)? != total { return None; }
    // Deck grid edits keep the original entries and store a signed millisecond
    // adjustment in the header. Negative effective times are unsupported: the
    // grid model drops preroll beats, which would change native phrase indices.
    let shift = i16::from_be_bytes([section[18], section[19]]) as i64;
    let mut previous = None;
    Some(section[24..].chunks_exact(8).enumerate().map(move |(index, entry)| {
        let beat = u16::from_be_bytes([entry[0], entry[1]]);
        let tempo = u16::from_be_bytes([entry[2], entry[3]]);
        let ms = i64::from(u32::from_be_bytes(entry[4..8].try_into().ok()?)) + shift;
        if ms < 0 || tempo == 0 || !(1..=4).contains(&beat) || previous.map(|last| ms <= last).unwrap_or(false) {
            return None;
        }
        previous = Some(ms);
        Some(Beat { time: ms as f64 / 1000.0, bpm: tempo as f64 / 100.0, beat: beat as u8, index })
    }))
}

#[cfg(test)]
mod pqtz_tests {
    use super::*;

    fn fixture(shift: i16, entries: &[(u16, u16, u32)]) -> Vec<u8> {
        let total = 28 + 24 + entries.len() * 8;
        let mut b = b"PMAI".to_vec();
        b.extend_from_slice(&28u32.to_be_bytes());
        b.extend_from_slice(&(total as u32).to_be_bytes());
        b.resize(28, 0);
        b.extend_from_slice(b"PQTZ");
        b.extend_from_slice(&24u32.to_be_bytes());
        b.extend_from_slice(&((24 + entries.len() * 8) as u32).to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&8u16.to_be_bytes());
        b.extend_from_slice(&shift.to_be_bytes());
        b.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (beat, tempo, ms) in entries {
            b.extend_from_slice(&beat.to_be_bytes());
            b.extend_from_slice(&tempo.to_be_bytes());
            b.extend_from_slice(&ms.to_be_bytes());
        }
        b
    }

    #[test]
    fn pqtz_import_applies_signed_shift_to_each_tempo_anchor() {
        for (shift, raw, first, next) in [
            (0, 29, 0.029, 1.029),
            (414, 29, 0.443, 1.443),
            (-166, 178, 0.012, 1.012),
            (-230, 236, 0.006, 1.006),
            (i16::MIN, 32768, 0.0, 1.0),
            (i16::MAX, 1, 32.768, 33.768),
        ] {
            let b = fixture(shift, &[(1, 12000, raw), (2, 12000, raw + 500), (3, 12500, raw + 1000)]);
            let g = parse_pqtz_grid(&b).unwrap();
            assert_eq!(g.anchors.len(), 2);
            assert_eq!(g.anchors[0].time, first);
            assert_eq!(g.anchors[1].time, next);
            assert_eq!((g.anchors[0].bpm, g.anchors[1].bpm), (120.0, 125.0));
            assert_eq!((g.anchors[0].beat, g.anchors[1].beat), (1, 3));
            assert_eq!(parse_pqtz_grid(&b).unwrap().anchors[0].time, g.anchors[0].time);
        }
    }

    #[test]
    fn pqtz_rejects_negative_preroll_without_reindexing_native_phrases() {
        // Dropping source beat 1 would make a native phrase at beat 2 refer to beat 3.
        let b = fixture(-200, &[(1, 12000, 100), (2, 12000, 600)]);
        assert!(parse_pqtz_grid(&b).is_none());
        assert!(!pqtz_indices_match(&b, &BeatGrid::constant(0.4, 120.0, 2).beats(1.0)));
        // Check every record, including one that would not create a new tempo anchor.
        assert!(parse_pqtz_grid(&fixture(-200, &[(1, 12000, 400), (2, 12000, 100)])).is_none());
        assert_eq!(parse_pqtz_grid(&fixture(-100, &[(1, 12000, 100)])).unwrap().anchors[0].time, 0.0);
    }

    #[test]
    fn pqtz_phrase_indices_match_only_the_same_audible_records() {
        let b = fixture(-166, &[(1, 12000, 178), (2, 12000, 678)]);
        let beats = parse_pqtz_grid(&b).unwrap().beats(1.1);
        assert!(pqtz_indices_match(&b, &beats));
        let phrase = crate::phrase::Structure { mood: 1, phrases: vec![crate::phrase::Phrase { beat: 2, kind: crate::phrase::UP }], end_beat: 2, rb: None };
        assert_eq!(crate::phrase::cue_proposals(&phrase, &beats), vec![(0.512, "Up")]);
        assert!(!pqtz_indices_match(&b, &beats[..1]), "missing source records cannot be reused");
        for field in 0..4 {
            let mut changed = beats.clone();
            match field {
                0 => changed[1].time += 0.002,
                1 => changed[1].bpm += 1.0,
                2 => changed[1].beat = 3,
                _ => changed[1].index = 0,
            }
            assert!(!pqtz_indices_match(&b, &changed));
        }
        let rounded = fixture(-166, &[(1, 12000, 178), (2, 12000, 677)]);
        assert!(pqtz_indices_match(&rounded, &beats), "whole-millisecond quantization is allowed");
    }

    #[test]
    fn pqtz_positive_shift_imports_without_reusing_reindexed_phrases() {
        // The documented deck edit moves 29 ms to 443 ms at 137 BPM. That is more
        // than one period, so the model extrapolates an earlier audible beat.
        let b = fixture(414, &[(1, 13700, 29), (2, 13700, 467)]);
        let g = parse_pqtz_grid(&b).unwrap();
        assert_eq!(g.anchors[0].time, 0.443);
        let beats = g.beats(2.0);
        assert!(beats[0].time < 0.006);
        assert!((beats[1].time - 0.443).abs() < 1e-9);
        assert!(!pqtz_indices_match(&b, &beats), "native phrases must be analysed on this audible grid");
    }

    #[test]
    fn pqtz_rejects_invalid_beat_records() {
        for entries in [vec![(0, 12000, 100)], vec![(5, 12000, 100)], vec![(1, 0, 100)], vec![(1, 12000, 100), (2, 12000, 100)], vec![(1, 12000, 100), (2, 12000, 50)]] {
            let b = fixture(0, &entries);
            assert!(parse_pqtz_grid(&b).is_none());
            assert!(!pqtz_indices_match(&b, &BeatGrid::constant(0.1, 120.0, 1).beats(1.0)));
        }
    }

    #[test]
    fn refined_grid_replaces_native_usb_timing_even_below_a_millisecond() {
        use crate::export::anlz;
        let original = fixture(0, &[(1, 12000, 200), (2, 12000, 700)]);
        let native = parse_pqtz_grid(&original).unwrap();
        let stored: BeatGrid = serde_json::from_str(&serde_json::to_string(&native).unwrap()).unwrap();
        assert!(crate::app::grids_match(&native, &stored));
        let different_bpm = BeatGrid::constant(0.2, 120.003, 1);
        assert!(!crate::app::grids_match(&native, &different_bpm));

        let refined = BeatGrid::constant(0.200475, 120.0, 1);
        let overrides = anlz::Overrides {
            sections: HashMap::from([
                ("PQTZ".into(), original[28..].to_vec()),
                ("PQT2".into(), anlz::pqt2(&[(1, 12000, 200), (2, 12000, 700)], &[0, 0])),
            ]),
            grid_same: crate::app::grids_match(&native, &refined),
            pssi: None, device: Default::default(), pvdi: None,
        };
        let files = anlz::build_with("/Contents/test.wav", &refined.beats(1.0), &Default::default(), &[], 44100, 44100, Some(&overrides));
        let ext = anlz::section_list(&files.ext);
        let section = &ext.iter().find(|(tag, _)| tag == "PQT2").unwrap().1;
        assert_eq!(anlz::pqt2_fractions(section), [475, 475]);
    }

    #[test]
    fn pqtz_materialized_export_applies_shift_once() {
        let original = fixture(200, &[(1, 12000, 100)]);
        let g = parse_pqtz_grid(&original).unwrap();
        let wave = crate::model::Waveform { frames: vec![[0; 8]] };
        let files = crate::export::anlz::build("/Contents/test.wav", &g.beats(1.0), &wave, &[], 0, 0);
        let imported = parse_pqtz_grid(&files.dat).unwrap();
        assert_eq!(imported.anchors.len(), 1);
        assert_eq!(imported.anchors[0].time, 0.3);
        assert_eq!(imported.anchors[0].beat, 1);
        let emitted = crate::export::anlz::section_list(&files.dat).into_iter().find(|(tag, _)| tag == "PQTZ").unwrap().1;
        assert_eq!(&emitted[18..20], &[0, 0]);

        // An unchanged source grid keeps its original section instead of folding the
        // already-applied adjustment into the raw entries a second time.
        let raw_pqt2 = crate::export::anlz::pqt2(&[(1, 12000, 100)], &[345]);
        let overrides = crate::export::anlz::Overrides {
            sections: HashMap::from([("PQTZ".into(), original[28..].to_vec()), ("PQT2".into(), raw_pqt2.clone())]),
            grid_same: true,
            pssi: None,
            device: Default::default(),
            pvdi: None,
        };
        let preserved = crate::export::anlz::build_with("/Contents/test.wav", &g.beats(1.0), &wave, &[], 0, 0, Some(&overrides));
        let emitted = crate::export::anlz::section_list(&preserved.dat).into_iter().find(|(tag, _)| tag == "PQTZ").unwrap().1;
        assert_eq!(emitted, original[28..]);
        assert_eq!(parse_pqtz_grid(&preserved.dat).unwrap().anchors[0].time, 0.3);
        let emitted = crate::export::anlz::section_list(&preserved.ext).into_iter().find(|(tag, _)| tag == "PQT2").unwrap().1;
        assert_eq!(emitted, raw_pqt2, "PQT2 anchors and fractions stay in their original section");
    }

    #[test]
    fn pqtz_rejects_incomplete_framing_and_counts() {
        let b = fixture(0, &[(1, 12000, 100)]);
        for end in 0..b.len() {
            assert!(parse_pqtz_grid(&b[..end]).is_none(), "prefix {end}");
        }
        for count in [0, 2, u32::MAX] {
            let mut bad = b.clone();
            bad[48..52].copy_from_slice(&count.to_be_bytes());
            assert!(parse_pqtz_grid(&bad).is_none());
        }
        // A following opaque section must not supply an entry missing from PQTZ.
        let mut bad = b;
        bad[48..52].copy_from_slice(&2u32.to_be_bytes());
        bad.extend_from_slice(b"PZZZ\0\0\0\x0c\0\0\0\x0c");
        let len = bad.len() as u32;
        bad[8..12].copy_from_slice(&len.to_be_bytes());
        assert!(parse_pqtz_grid(&bad).is_none());
    }
}

/// Cue kind in master.db -> (hot cue slot 0-based, or None for a memory cue). Kinds 1–3 are hot
/// cues A–C, kind 5 is D and 6–17 are E–P (rekordbox-rea RF-07); 0 and 4 are memory cues.
fn cue_slot(kind: i64) -> Option<i32> {
    match kind {
        1..=3 => Some(kind as i32 - 1),
        5 => Some(3),
        6..=17 => Some(kind as i32 - 2),
        _ => None,
    }
}

/// Modern hot cues store a pad palette index separately. A memory cue's native
/// colors 0..7 map to the eight nonzero color rows used by Recordbox.
fn desktop_cue_color(slot: Option<i32>, looped: bool, color: Option<i64>, palette: Option<i64>, modern: bool) -> u8 {
    if let Some(slot) = slot {
        let index = if modern { palette } else { color };
        return index.filter(|n| (1..=64).contains(n)).map_or_else(|| hotcue_default_color(slot, looped), |n| n as u8);
    }
    if modern {
        color.filter(|n| (0..=7).contains(n)).map_or(0, |n| (n + 1) as u8)
    } else {
        color.filter(|n| (1..=8).contains(n)).unwrap_or(0) as u8
    }
}

/// Read everything Recordbox imports from a snapshot opened with [`open_snapshot`].
/// `share_dir` is rekordbox's "share" folder, where its analysis files and artwork live.
pub fn read_library(c: &rusqlite::Connection, share_dir: &Path) -> Result<RbLibrary> {
    let artists = names(c, "djmdArtist")?;
    let albums = names(c, "djmdAlbum")?;
    let genres = names(c, "djmdGenre")?;
    let labels = names(c, "djmdLabel")?;
    let keys: HashMap<String, String> = rows(c, "SELECT ID, ScaleName FROM djmdKey WHERE rb_local_deleted = 0")?
        .into_iter()
        .map(|mut r| (text(r.remove("ID").unwrap_or(Value::Null)), text(r.remove("ScaleName").unwrap_or(Value::Null))))
        .collect();
    let lookup = |m: &HashMap<String, String>, v: Option<&Value>| v.map(|v| text(v.clone())).and_then(|id| m.get(&id).cloned()).unwrap_or_default();

    // Cues per track, with their newest change time for the fingerprint.
    let mut cues: HashMap<String, Vec<Cue>> = HashMap::new();
    let mut cue_stamp: HashMap<String, String> = HashMap::new();
    let modern_colors: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('djmdCue') WHERE name = 'ColorTableIndex')", [], |r| r.get(0))?;
    let palette = if modern_colors { "ColorTableIndex" } else { "NULL" };
    for r in rows(c, &format!("SELECT ContentID, InMsec, OutMsec, Kind, Color, {palette} AS palette, ActiveLoop, Comment, updated_at FROM djmdCue WHERE rb_local_deleted = 0 ORDER BY InMsec"))? {
        let cid = text(r["ContentID"].clone());
        let kind = int(&r["Kind"]);
        let start = int(&r["InMsec"]).max(0) as f64 / 1000.0;
        let out = int(&r["OutMsec"]);
        let end = (out > 0 && out as f64 / 1000.0 > start).then_some(out as f64 / 1000.0);
        let hot_slot = cue_slot(kind);
        let (ck, slot) = hot_slot.map_or((CueKind::Memory, -1), |slot| (CueKind::Hot, slot));
        let optional_int = |v: &Value| (!matches!(v, Value::Null)).then(|| int(v));
        let color = desktop_cue_color(hot_slot, end.is_some(), optional_int(&r["Color"]), optional_int(&r["palette"]), modern_colors);
        cues.entry(cid.clone()).or_default().push(Cue { id: 0, track_id: 0, kind: ck, slot, start, end, color, comment: text(r["Comment"].clone()), active_loop: int(&r["ActiveLoop"]) != 0 });
        let u = text(r["updated_at"].clone());
        let e = cue_stamp.entry(cid).or_default();
        if u > *e {
            *e = u;
        }
    }

    // My Tag: categories are rows whose parent is "root".
    let tag_rows = rows(c, "SELECT ID, Name, ParentID, Seq, Attribute FROM djmdMyTag WHERE rb_local_deleted = 0 ORDER BY Seq")?;
    let mut tag_categories: Vec<(String, String, Vec<String>)> = vec![];
    let mut tag_of: HashMap<String, (String, String)> = HashMap::new();
    for r in tag_rows.iter().filter(|r| text(r["ParentID"].clone()) == "root") {
        tag_categories.push((text(r["ID"].clone()), text(r["Name"].clone()), vec![]));
    }
    for r in tag_rows.iter().filter(|r| text(r["ParentID"].clone()) != "root") {
        let parent = text(r["ParentID"].clone());
        if let Some(cat) = tag_categories.iter_mut().find(|c| c.0 == parent) {
            let name = text(r["Name"].clone());
            cat.2.push(name.clone());
            tag_of.insert(text(r["ID"].clone()), (cat.1.clone(), name));
        }
    }
    let mut song_tags: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for r in rows(c, "SELECT ContentID, MyTagID FROM djmdSongMyTag WHERE rb_local_deleted = 0")? {
        if let Some(t) = tag_of.get(&text(r["MyTagID"].clone())) {
            song_tags.entry(text(r["ContentID"].clone())).or_default().push(t.clone());
        }
    }

    // Optional native identity columns only affect reverse sync on older schemas.
    let mut cue_identities = writeback::cues::identities(c).ok();
    let mut tracks = vec![];
    let mut not_local = 0;
    for r in rows(c, "SELECT * FROM djmdContent WHERE rb_local_deleted = 0")? {
        let rb_id = text(r["ID"].clone());
        let path = text(r["FolderPath"].clone());
        if !Path::new(&path).is_file() {
            not_local += 1;
            continue;
        }
        let g = |k: &str| r.get(k).cloned().map(text).unwrap_or_default();
        let n = |k: &str| r.get(k).map(int).unwrap_or(0);
        let ext = Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let image = g("ImagePath");
        let artwork = if image.is_empty() { String::new() } else { share_dir.join(image.trim_start_matches('/')).to_string_lossy().to_string() };
        let t = Track {
            path: path.clone(),
            title: g("Title"),
            artist: lookup(&artists, r.get("ArtistID")),
            album: lookup(&albums, r.get("AlbumID")),
            genre: lookup(&genres, r.get("GenreID")),
            label: lookup(&labels, r.get("LabelID")),
            remixer: lookup(&artists, r.get("RemixerID")),
            original_artist: lookup(&artists, r.get("OrgArtistID")),
            composer: lookup(&artists, r.get("ComposerID")),
            lyricist: g("Lyricist"),
            comment: g("Commnt"),
            message: String::new(),
            year: n("ReleaseYear") as i32,
            track_no: n("TrackNo") as i32,
            disc_no: n("DiscNo") as i32,
            release_date: g("ReleaseDate"),
            duration: n("Length") as f64,
            sample_rate: n("SampleRate") as u32,
            bitrate: n("BitRate") as u32,
            bit_depth: n("BitDepth") as u32,
            file_size: n("FileSize") as u64,
            file_type: ext,
            bpm: n("BPM") as f64 / 100.0,
            key: parse_key(&lookup(&keys, r.get("KeyID"))),
            key_text: lookup(&keys, r.get("KeyID")).trim().to_string(),
            rating: n("Rating").clamp(0, 5) as u8,
            color: n("ColorID").clamp(0, 8) as u8,
            date_added: g("StockDate"),
            date_created: g("DateCreated"),
            play_count: n("DJPlayCount").max(0) as u32,
            autoload_hotcue: g("HotCueAutoLoad") != "off",
            artwork: if Path::new(&artwork).is_file() { artwork } else { String::new() },
            analyzed: 0,
            ..Default::default()
        };
        let anlz = g("AnalysisDataPath");
        let grid_bytes = (!anlz.is_empty()).then(|| std::fs::read(share_dir.join(anlz.trim_start_matches('/'))).ok()).flatten();
        let grid = grid_bytes.as_deref().and_then(parse_pqtz_grid);
        let cue_identity = cue_identities.as_mut().map(|ids| ids.remove(&rb_id)
            .unwrap_or_else(writeback::empty_cue_identity));
        let native = writeback::ImportTrack::from_row(&r, cue_identity, &anlz, grid_bytes.as_deref().filter(|_| grid.is_some()));
        let anlz_base = (!anlz.is_empty()).then(|| share_dir.join(anlz.trim_start_matches('/')).with_extension("")).filter(|p| p.with_extension("DAT").is_file());
        let tags = song_tags.remove(&rb_id).unwrap_or_default();
        let mut tag_names: Vec<String> = tags.iter().map(|(c, t)| format!("{c}/{t}")).collect();
        tag_names.sort();
        let fingerprint = format!("{}|{}|{}|{}", g("updated_at"), cue_stamp.get(&rb_id).cloned().unwrap_or_default(), g("AnalysisUpdated"), tag_names.join(","));
        let xt = XmlTrack { xml_id: rb_id.clone(), track: t, grid, cues: cues.remove(&rb_id).unwrap_or_default(), extra: Default::default() };
        tracks.push(RbTrack { rb_id, native, fingerprint, xt, tags, anlz_base });
    }

    // Playlist tree.
    let pl = rows(c, "SELECT ID, Seq, Name, Attribute, ParentID, SmartList FROM djmdPlaylist WHERE rb_local_deleted = 0 ORDER BY Seq")?;
    let mut members: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for r in rows(c, "SELECT PlaylistID, ContentID, TrackNo FROM djmdSongPlaylist WHERE rb_local_deleted = 0")? {
        members.entry(text(r["PlaylistID"].clone())).or_default().push((int(&r["TrackNo"]), text(r["ContentID"].clone())));
    }
    let mut smart_lists = 0;
    struct Ctx<'a> {
        tag_of: &'a HashMap<String, (String, String)>,
        keys: &'a HashMap<String, String>,
        smart: usize,
    }
    fn build(parent: &str, pl: &[HashMap<String, Value>], members: &mut HashMap<String, Vec<(i64, String)>>, cx: &mut Ctx) -> Vec<XmlNode> {
        let mut out = vec![];
        for r in pl.iter().filter(|r| r.get("ParentID").map(|v| text(v.clone())).unwrap_or_default() == parent) {
            let id = text(r["ID"].clone());
            let folder = int(&r["Attribute"]) == 1;
            let smart_xml = r.get("SmartList").map(|v| text(v.clone())).unwrap_or_default();
            let smart = if smart_xml.is_empty() {
                None
            } else {
                match translate_smartlist(&smart_xml, cx.tag_of, cx.keys) {
                    Some(rule) => Some(rule),
                    None => {
                        cx.smart += 1;
                        continue;
                    }
                }
            };
            let mut keys = members.remove(&id).unwrap_or_default();
            keys.sort_by_key(|k| k.0);
            out.push(XmlNode { name: text(r["Name"].clone()), folder, children: if folder { build(&id, pl, members, cx) } else { vec![] }, track_keys: keys.into_iter().map(|k| k.1).collect(), by_location: false, smart });
        }
        out
    }
    let mut cx = Ctx { tag_of: &tag_of, keys: &keys, smart: 0 };
    let playlists = build("root", &pl, &mut members, &mut cx);
    // Histories: the same tree shape (year and month folders, one list per day).
    let has_history: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name COLLATE NOCASE IN ('djmdHistory', 'djmdSongHistory'))",
        [],
        |row| row.get(0),
    )?;
    let histories = if has_history {
        // A failed query must not turn an existing history mirror into an empty tree.
        let hist = rows(c, "SELECT ID, Seq, Name, Attribute, ParentID FROM djmdHistory WHERE rb_local_deleted = 0 ORDER BY Seq")
            .context("reading rekordbox history folders")?;
        let mut hmembers: HashMap<String, Vec<(i64, String)>> = HashMap::new();
        for r in rows(c, "SELECT HistoryID, ContentID, TrackNo FROM djmdSongHistory WHERE rb_local_deleted = 0")
            .context("reading rekordbox history tracks")? {
            hmembers.entry(text(r["HistoryID"].clone())).or_default().push((int(&r["TrackNo"]), text(r["ContentID"].clone())));
        }
        build("root", &hist, &mut hmembers, &mut cx)
    } else {
        Vec::new()
    };
    smart_lists += cx.smart;
    Ok(RbLibrary {
        source: None,
        tracks,
        playlists,
        tag_categories: tag_categories.into_iter().map(|(_, n, t)| (n, t)).collect(),
        not_local,
        smart_lists,
        histories,
    })
}

/// Snapshot the installed rekordbox library into `dest_dir` and read it.
pub fn load_installed(dest_dir: &Path) -> Result<Option<RbLibrary>> {
    let Some(src) = library_dir() else { return Ok(None) };
    Ok(Some(load_from(&src, dest_dir)?))
}

pub(super) fn load_from(src: &Path, dest_dir: &Path) -> Result<RbLibrary> {
    anyhow::ensure!(!writeback::pending(src)?, "Recover the interrupted native write before importing this library");
    let db = snapshot(src, dest_dir)?;
    let c = open_snapshot(&db.path().join("master.db"))?;
    let mut library = read_library(&c, &src.join("share"))?;
    anyhow::ensure!(!writeback::pending(src)?, "Native recovery became pending during import; try again after recovery");
    library.source = writeback::Source::read(&c, src).ok();
    Ok(library)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real library on this machine, if present (copy only). Run with --ignored.
    #[test]
    #[ignore]
    fn reads_local_library_snapshot() {
        let Some(src) = library_dir() else { return };
        let dest = std::env::temp_dir().join(format!("recordbox_rb_snapshot_{}", std::process::id()));
        let before = stamp(&src.join("master.db")).unwrap();
        let db = snapshot(&src, &dest).unwrap();
        let c = open_snapshot(&db.path().join("master.db")).unwrap();
        for t in ["djmdContent", "djmdPlaylist", "djmdSongPlaylist", "djmdCue", "djmdMyTag"] {
            let n: i64 = c.query_row(&format!("SELECT count(*) FROM {t} WHERE rb_local_deleted = 0"), [], |r| r.get(0)).unwrap();
            println!("{t}: {n}");
        }
        let lib = read_library(&c, &src.join("share")).unwrap();
        for t in lib.tracks.iter().take(6) {
            println!("key {:?} -> {} :: {}", t.xt.track.key_text, t.xt.track.key, t.xt.track.title);
        }
        let with_grid = lib.tracks.iter().filter(|t| t.xt.grid.is_some()).count();
        let hot = lib.tracks.iter().map(|t| t.xt.cues.iter().filter(|c| c.kind == CueKind::Hot).count()).sum::<usize>();
        println!("local tracks {}, not local {}, with grid {}, hot cues {}, playlists {}, smart {}, tag categories {}", lib.tracks.len(), lib.not_local, with_grid, hot, lib.playlists.len(), lib.smart_lists, lib.tag_categories.len());
        fn walk(n: &[XmlNode], d: usize) {
            for x in n {
                println!("{}history node {:?} folder={} tracks={}", "  ".repeat(d), x.name, x.folder, x.track_keys.len());
                walk(&x.children, d + 1);
            }
        }
        walk(&lib.histories, 0);
        assert_eq!(before, stamp(&src.join("master.db")).unwrap(), "the original must not change");
        let mut st = c.prepare("SELECT Name, SmartList FROM djmdPlaylist WHERE rb_local_deleted=0 AND SmartList IS NOT NULL AND SmartList != ''").unwrap();
        for r in st.query_map([], |r| Ok(format!("{} => {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap().flatten() {
            println!("smartlist: {r}");
        }
        for t in ["djmdHistory", "djmdSongHistory"] {
            if let Ok(mut st) = c.prepare(&format!("PRAGMA table_info({t})")) {
                let cols: Vec<String> = st.query_map([], |r| r.get::<_, String>(1)).unwrap().flatten().collect();
                let n: i64 = c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)).unwrap_or(-1);
                println!("{t} ({n} rows): {}", cols.join(","));
            }
        }
        if let Ok(mut st) = c.prepare("SELECT ID, Seq, Name, Attribute, ParentID, DateCreated FROM djmdHistory WHERE rb_local_deleted=0 LIMIT 10") {
            for r in st.query_map([], |r| Ok((0..6).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
                println!("history: {r}");
            }
        }
        let _ = std::fs::remove_dir_all(&dest);
    }
}

#[cfg(test)]
mod smartlist_tests {
    use super::*;
    use crate::smart::{Field, Op};

    #[test]
    fn translates_rekordbox_smartlists() {
        let tags = HashMap::from([("3937219461".to_string(), ("Mood".to_string(), "Dark".to_string()))]);
        let keys = HashMap::from([("5".to_string(), "Am".to_string())]);
        let xml = r#"<NODE Id="-12345" LogicalOperator="2" AutomaticUpdate="1"><CONDITION PropertyName="genre" Operator="8" ValueUnit="" ValueLeft="Techno &amp; House" ValueRight=""/><CONDITION PropertyName="bpm" Operator="5" ValueUnit="" ValueLeft="12000" ValueRight="128"/><CONDITION PropertyName="myTag" Operator="8" ValueUnit="" ValueLeft="-357747835" ValueRight=""/><CONDITION PropertyName="stockDate" Operator="6" ValueUnit="month" ValueLeft="3" ValueRight=""/><CONDITION PropertyName="key" Operator="1" ValueUnit="" ValueLeft="5" ValueRight=""/><CONDITION PropertyName="grouping" Operator="1" ValueUnit="" ValueLeft="2" ValueRight=""/></NODE>"#;
        let r = translate_smartlist(xml, &tags, &keys).unwrap();
        assert!(!r.match_all);
        assert_eq!(r.conditions.len(), 6);
        assert_eq!((r.conditions[0].field, r.conditions[0].op, r.conditions[0].value.as_str()), (Field::Genre, Op::Contains, "Techno & House"));
        assert_eq!((r.conditions[1].value.as_str(), r.conditions[1].value2.as_str()), ("120", "128"));
        assert_eq!(r.conditions[2].value, tag_placeholder("Mood", "Dark"));
        assert!(r.conditions[3].months && r.conditions[3].op == Op::InLast);
        assert_eq!(r.conditions[4].value, "Am");
        assert_eq!(r.conditions[5].field, Field::Color);
        // A condition with no equivalent keeps the whole list out.
        let odd = r#"<NODE Id="1" LogicalOperator="1" AutomaticUpdate="1"><CONDITION PropertyName="dateCreated" Operator="6" ValueUnit="day" ValueLeft="7" ValueRight=""/></NODE>"#;
        assert!(translate_smartlist(odd, &tags, &keys).is_none());
    }
}

#[cfg(test)]
mod schema_dump {
    #[test]
    #[ignore]
    fn dump_columns() {
        let Some(src) = super::library_dir() else { return };
        let dest = std::env::temp_dir().join(format!("recordbox_rb_schema_{}", std::process::id()));
        let snapshot = super::snapshot(&src, &dest).unwrap();
        let c = super::open_snapshot(&snapshot.path().join("master.db")).unwrap();
        for t in ["djmdContent", "djmdCue", "djmdPlaylist", "djmdSongPlaylist", "djmdKey", "djmdArtist", "djmdAlbum", "djmdGenre", "djmdLabel", "djmdColor", "djmdMyTag", "djmdSongMyTag"] {
            let mut st = c.prepare(&format!("PRAGMA table_info({t})")).unwrap();
            let cols: Vec<String> = st.query_map([], |r| r.get::<_, String>(1)).unwrap().flatten().collect();
            println!("{t}: {}", cols.join(","));
        }
        let mut st = c.prepare("SELECT Kind, count(*), min(InMsec), max(OutMsec), min(Color), max(Color), min(ColorTableIndex), max(ColorTableIndex), sum(ActiveLoop) FROM djmdCue WHERE rb_local_deleted=0 GROUP BY Kind").unwrap();
        for r in st.query_map([], |r| Ok((0..9).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" "))).unwrap().flatten() {
            println!("cue kind: {r}");
        }
        let mut st = c.prepare("SELECT ScaleName FROM djmdKey LIMIT 30").unwrap();
        println!("keys: {:?}", st.query_map([], |r| r.get::<_, String>(0)).unwrap().flatten().collect::<Vec<_>>());
        let mut st = c.prepare("SELECT AnalysisDataPath, FolderPath, FileType, Rating, ColorID, DJPlayCount, BPM FROM djmdContent LIMIT 2").unwrap();
        for r in st.query_map([], |r| Ok((0..7).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
            println!("content sample: {r}");
        }
        let mut st = c.prepare("SELECT ID, Seq, Name, Attribute, ParentID, SmartList IS NOT NULL AND SmartList != '' FROM djmdPlaylist WHERE rb_local_deleted=0").unwrap();
        for r in st.query_map([], |r| Ok((0..6).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
            println!("playlist: {r}");
        }
        let mut st = c.prepare("SELECT Name, SmartList FROM djmdPlaylist WHERE rb_local_deleted=0 AND SmartList IS NOT NULL AND SmartList != ''").unwrap();
        for r in st.query_map([], |r| Ok(format!("{} => {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap().flatten() {
            println!("smartlist: {r}");
        }
        for t in ["djmdHistory", "djmdSongHistory"] {
            if let Ok(mut st) = c.prepare(&format!("PRAGMA table_info({t})")) {
                let cols: Vec<String> = st.query_map([], |r| r.get::<_, String>(1)).unwrap().flatten().collect();
                let n: i64 = c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)).unwrap_or(-1);
                println!("{t} ({n} rows): {}", cols.join(","));
            }
        }
        if let Ok(mut st) = c.prepare("SELECT ID, Seq, Name, Attribute, ParentID, DateCreated FROM djmdHistory WHERE rb_local_deleted=0 LIMIT 10") {
            for r in st.query_map([], |r| Ok((0..6).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
                println!("history: {r}");
            }
        }
        let _ = std::fs::remove_dir_all(&dest);
    }
}

#[cfg(test)]
mod onelibrary_probe {
    /// Does the master.db key open a OneLibrary catalog? Run with --ignored.
    #[test]
    #[ignore]
    fn probe_key() {
        let p = std::env::var("PREPDECK_ONELIB").unwrap();
        let c = rusqlite::Connection::open_with_flags(&p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let key = std::env::var("PREPDECK_ONELIB_KEY_FILE").ok().map(|f| std::fs::read_to_string(f).unwrap()).unwrap_or_else(|| super::MASTER_DB_KEY.to_string());
        c.pragma_update(None, "key", key.trim()).unwrap();
        match c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)) {
            Ok(n) => {
                println!("opened: {n} objects");
                let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type='table'").unwrap();
                println!("{:?}", st.query_map([], |r| r.get::<_, String>(0)).unwrap().flatten().collect::<Vec<_>>());
                if let Ok(out) = std::env::var("PREPDECK_ONELIB_DDL") { let out = out.as_str();
                    let mut st = c.prepare("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY rowid").unwrap();
                    let ddl: Vec<String> = st.query_map([], |r| Ok(format!("-- {} {}\n{};", r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).unwrap().flatten().collect();
                    std::fs::write(out, ddl.join("\n\n")).unwrap();
                    let mut rows = String::new();
                    for t in ["menuItem", "category", "sort", "property", "color", "key", "myTag", "image"] {
                        let mut st = c.prepare(&format!("SELECT * FROM {t}")).unwrap();
                        let n = st.column_count();
                        for r in st.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap())).collect::<Vec<_>>().join(", "))).unwrap().flatten() {
                            rows.push_str(&format!("{t}: {r}\n"));
                        }
                    }
                    std::fs::write(format!("{out}.rows"), rows).unwrap();
                    for pr in ["journal_mode", "page_size", "cipher_version", "kdf_iter", "cipher_page_size", "cipher_hmac_algorithm", "cipher_kdf_algorithm", "user_version", "encoding"] {
                        println!("{pr}: {:?}", c.query_row(&format!("PRAGMA {pr}"), [], |r| r.get::<_, rusqlite::types::Value>(0)));
                    }
                }
                println!("property: {:?}", c.query_row("SELECT deviceName, dbVersion, numberOfContents, createdDate FROM property", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))));
            }
            Err(e) => println!("not opened: {e}"),
        }
    }
}

#[cfg(test)]
mod cue_origin_probe {
    /// When were the user's cues created relative to the track's analysis? Run with --ignored.
    #[test]
    #[ignore]
    fn cue_creation_times() {
        let Some(src) = super::library_dir() else { return };
        let dest = std::env::temp_dir().join(format!("recordbox_cue_origin_{}", std::process::id()));
        let snapshot = super::snapshot(&src, &dest).unwrap();
        let c = super::open_snapshot(&snapshot.path().join("master.db")).unwrap();
        let mut st = c.prepare("SELECT k.Kind, k.Comment, k.ColorTableIndex, k.created_at, c.created_at, c.updated_at, c.AnalysisUpdated FROM djmdCue k JOIN djmdContent c ON c.ID = k.ContentID WHERE k.rb_local_deleted = 0 LIMIT 12").unwrap();
        for r in st.query_map([], |r| Ok((0..7).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| format!("{v:?}")).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
            println!("cue: {r}");
        }
        for q in ["SELECT Analysed, count(*) FROM djmdContent WHERE rb_local_deleted = 0 GROUP BY 1", "SELECT group_concat(name, ' ') FROM pragma_table_info('djmdContent')", "SELECT Analysed, AnalysisDataPath IS NOT NULL, count(*) FROM djmdContent WHERE rb_local_deleted = 0 GROUP BY 1,2", "SELECT DBID FROM djmdProperty", "SELECT ID, MasterDBID, MasterSongID, ContentLink, ModifiedByRBM, SearchStr, DateCreated, StockDate, CueUpdated, AnalysisUpdated, TrackInfoUpdated, DeliveryControl, HotCueAutoLoad, Analysed FROM djmdContent WHERE rb_local_deleted = 0 LIMIT 6"] {
            let mut st = c.prepare(q).unwrap();
            let n = st.column_count();
            for r in st.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap())).collect::<Vec<_>>().join(" | "))).unwrap().flatten() {
                println!("{q}: {r}");
            }
        }
        let n: i64 = c.query_row("SELECT count(DISTINCT substr(created_at,1,16)) FROM djmdCue WHERE rb_local_deleted = 0", [], |r| r.get(0)).unwrap();
        println!("distinct creation minutes: {n}");
        let _ = std::fs::remove_dir_all(&dest);
    }
}
