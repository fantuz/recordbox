//! The app's own private library database (SQLite in ~/Library/Application Support/Recordbox).
//! It never opens or writes rekordbox's library, and never writes to audio files.

use crate::model::*;
use crate::audio::{mp3_timeline, Mp3Timeline};
use crate::smart::SmartRule;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
mod track_ids;
pub(crate) use track_ids::visit_derived_links;

fn data_locations() -> (PathBuf, Option<PathBuf>) {
    if let Ok(d) = std::env::var("PREPDECK_DATA") {
        return (PathBuf::from(d), None);
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    (base.join("Recordbox"), Some(base.join("PrepDeck")))
}

/// Locate existing files without creating or migrating the application's data directory.
pub fn shared_data_dir_for_inspection() -> PathBuf {
    let (path, legacy) = data_locations();
    legacy.filter(|old| !path.exists() && old.is_dir()).unwrap_or(path)
}

pub fn shared_data_dir() -> PathBuf {
    if mp3_timeline() == Mp3Timeline::Rekordbox {
        // An audition must not move an existing PrepDeck library to a new location.
        return shared_data_dir_for_inspection();
    }
    let (path, legacy) = data_locations();
    // The app was called PrepDeck before: its library folder moves over once, unchanged.
    if let Some(old) = legacy.filter(|old| !path.exists() && old.is_dir()) {
        let _ = std::fs::rename(old, &path);
    }
    path
}

pub(crate) fn collection_dir(base: PathBuf, timeline: Mp3Timeline) -> PathBuf {
    match timeline {
        Mp3Timeline::Legacy => base,
        Mp3Timeline::Rekordbox => base.join("mp3-timeline-rekordbox"),
    }
}

pub fn data_dir_for_inspection() -> PathBuf {
    collection_dir(shared_data_dir_for_inspection(), mp3_timeline())
}

pub fn data_dir() -> PathBuf {
    let path = collection_dir(shared_data_dir(), mp3_timeline());
    let _ = std::fs::create_dir_all(&path);
    path
}

pub fn artwork_dir() -> PathBuf {
    let p = data_dir().join("artwork");
    let _ = std::fs::create_dir_all(&p);
    p
}

#[cfg(test)]
mod mp3_timeline_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn opt_in_collection_and_backups_are_isolated() -> Result<()> {
        // Launch the other clock in a child so the test also exercises the
        // session-wide selection without changing the running test process.
        let cancel = AtomicBool::new(false);
        if std::env::var_os("RECORDBOX_TEST_MP3_TIMELINE_CHILD").is_some() {
            Mp3Timeline::Rekordbox.initialize();
            let root = PathBuf::from(std::env::var_os("PREPDECK_DATA").unwrap());
            assert_eq!(data_dir_for_inspection(), root.join("mp3-timeline-rekordbox"));
            assert!(!data_dir_for_inspection().exists());
            let path = data_dir().join("library.db");
            let db = Db::open(&path)?;
            let tracks: i64 = db.conn.query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))?;
            assert_eq!(tracks, 0);
            assert!(Db::open(&root.join("library.db")).err().unwrap().to_string().contains("--mp3-timeline legacy"));
            assert_eq!(crate::stems::models_dir(), root.join("models"));
            assert_eq!(crate::stems::stems_dir(), data_dir().join("stems"));
            let cache = crate::stems::cache_path("same-file.flac", "htdemucs");
            let original_cache = root.join("stems").join(cache.file_name().unwrap());
            std::fs::create_dir_all(original_cache.parent().unwrap())?;
            std::fs::write(&original_cache, b"cached")?;
            assert_eq!(crate::stems::existing_cache_path(&cache), Some(original_cache.clone()));
            crate::stems::clear_cache()?;
            assert!(original_cache.exists());
            let work = data_dir().join("restore-work");
            let legacy = root.join("legacy.zip");
            assert!(crate::backup::stage_restore(&legacy, &work, &artwork_dir(), false, &|_, _, _| {}, &cancel).is_err());
            assert!(!work.exists());
            let archive = root.join("alternate.zip");
            crate::backup::write_backup(&path, &artwork_dir(), &archive, false, &|_, _, _| {}, &cancel)?;
            assert_eq!(crate::backup::read_manifest(&archive)?.format, 2);
            crate::backup::stage_restore(&archive, &work, &artwork_dir(), false, &|_, _, _| {}, &cancel)?;
            return Ok(());
        }
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("library.db");
        let legacy = Db::open(&path)?;
        legacy.conn.execute_batch(
            "INSERT INTO tracks(id,path,raw_samples) VALUES(1,'local.mp3',44100);
             INSERT INTO cues(track_id,kind,slot,start) VALUES(1,0,1,0.25);
             INSERT INTO grids(track_id,json) VALUES(1,'[]');")?;
        let archive = dir.path().join("legacy.zip");
        crate::backup::write_backup(&path, &dir.path().join("artwork"), &archive, false, &|_, _, _| {}, &cancel)?;
        assert_eq!(crate::backup::read_manifest(&archive)?.format, 1);
        let result = std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", "db::mp3_timeline_tests::opt_in_collection_and_backups_are_isolated", "--nocapture"])
            .env("PREPDECK_DATA", dir.path()).env("RECORDBOX_TEST_MP3_TIMELINE_CHILD", "1").output()?;
        assert!(result.status.success(), "{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        let original: (i64, f64, String) = legacy.conn.query_row(
            "SELECT raw_samples,start,json FROM tracks JOIN cues ON tracks.id=cues.track_id JOIN grids ON tracks.id=grids.track_id",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        assert_eq!(original, (44100, 0.25, "[]".to_owned()));
        assert!(Db::open(&dir.path().join("mp3-timeline-rekordbox/library.db")).err().unwrap().to_string().contains("--mp3-timeline rekordbox"));
        assert!(crate::backup::read_manifest(&dir.path().join("alternate.zip")).unwrap_err().to_string().contains("--mp3-timeline rekordbox"));
        Ok(())
    }
}

pub struct Db {
    pub conn: Connection,
}

const TRACK_COLS: &str = "id,path,title,artist,album,album_artist,genre,label,remixer,original_artist,composer,lyricist,mix_name,comment,grouping,message,year,track_no,disc_no,release_date,duration,sample_rate,bitrate,bit_depth,file_size,file_type,bpm,key,rating,color,date_added,date_created,play_count,analyzed,grid_locked,autoload_hotcue,gain_db,artwork,raw_samples,key_manual,key_text";

/// Library schema version (SQLite user_version). Version 0 is a library from before
/// versioning. Version 4 records the origin of new direct file imports for cleanup review.
pub const SCHEMA_VERSION: i64 = 4;

impl Db {
    /// The main database attached to this connection (including a custom library directory).
    pub fn active_library_path(&self) -> &str {
        self.conn.path().filter(|path| !path.is_empty()).unwrap_or("Path unavailable")
    }

    pub fn open(path: &std::path::Path) -> Result<Db> {
        Self::open_for_timeline(path, mp3_timeline())
    }

    fn open_for_timeline(path: &std::path::Path, timeline: Mp3Timeline) -> Result<Db> {
        // Refuse a library from a newer Recordbox; back up an older one before upgrading it.
        let existed = path.metadata().map(|m| m.len() > 0).unwrap_or(false);
        if existed {
            let probe = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let v: i64 = probe.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            let has_tables: bool = probe.query_row("SELECT count(*) FROM sqlite_master WHERE name='tracks'", [], |r| r.get::<_, i64>(0)).map(|n| n > 0)?;
            if !has_tables {
                let native: bool = probe.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='djmdContent' COLLATE NOCASE)
                     OR (EXISTS(SELECT 1 FROM pragma_table_info('property') WHERE name='dbVersion' COLLATE NOCASE)
                         AND EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='content' COLLATE NOCASE))",
                    [], |r| r.get(0))?;
                anyhow::ensure!(!native, "An AlphaTheta catalog cannot be opened as a private recordbox library.");
            }
            if has_tables {
                let has_settings: bool = probe.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='settings')", [], |r| r.get(0))?;
                let marker: Option<String> = if has_settings {
                    probe.query_row("SELECT value FROM settings WHERE key='mp3_timeline'", [], |r| r.get(0)).optional()?
                } else { None };
                let clock = marker.as_deref().unwrap_or("legacy");
                anyhow::ensure!(clock == timeline.name(),
                    "This library uses the {clock} MP3 timeline. Start recordbox with --mp3-timeline {clock} to use it.");
            }
            drop(probe);
            if v > SCHEMA_VERSION {
                anyhow::bail!("This library was written by a newer version of recordbox (schema {v}); update recordbox to open it. It was not changed.");
            }
            if v < SCHEMA_VERSION && has_tables {
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                let backup = path.with_file_name(format!("library.v{v}-{stamp}.bak.db"));
                // VACUUM INTO writes a consistent copy that includes the write-ahead log.
                let c = Connection::open(path)?;
                c.execute("VACUUM INTO ?1", [backup.to_string_lossy().to_string()])?;
            }
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS tracks(
                id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, title TEXT DEFAULT '', artist TEXT DEFAULT '',
                album TEXT DEFAULT '', album_artist TEXT DEFAULT '', genre TEXT DEFAULT '', label TEXT DEFAULT '',
                remixer TEXT DEFAULT '', original_artist TEXT DEFAULT '', composer TEXT DEFAULT '', lyricist TEXT DEFAULT '',
                mix_name TEXT DEFAULT '', comment TEXT DEFAULT '', grouping TEXT DEFAULT '', message TEXT DEFAULT '',
                year INTEGER DEFAULT 0, track_no INTEGER DEFAULT 0, disc_no INTEGER DEFAULT 0, release_date TEXT DEFAULT '',
                duration REAL DEFAULT 0, sample_rate INTEGER DEFAULT 0, bitrate INTEGER DEFAULT 0, bit_depth INTEGER DEFAULT 0,
                file_size INTEGER DEFAULT 0, file_type TEXT DEFAULT '', bpm REAL DEFAULT 0, key INTEGER DEFAULT -1,
                rating INTEGER DEFAULT 0, color INTEGER DEFAULT 0, date_added TEXT DEFAULT '', date_created TEXT DEFAULT '',
                play_count INTEGER DEFAULT 0, analyzed INTEGER DEFAULT 0, grid_locked INTEGER DEFAULT 0,
                autoload_hotcue INTEGER DEFAULT 1, gain_db REAL DEFAULT 0, artwork TEXT DEFAULT '');
            CREATE TABLE IF NOT EXISTS cues(
                id INTEGER PRIMARY KEY, track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                kind INTEGER NOT NULL, slot INTEGER NOT NULL, start REAL NOT NULL, end_ REAL,
                color INTEGER DEFAULT 0, comment TEXT DEFAULT '', active_loop INTEGER DEFAULT 0);
            CREATE INDEX IF NOT EXISTS cues_track ON cues(track_id);
            CREATE TABLE IF NOT EXISTS grids(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS waves(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, data BLOB NOT NULL, preview BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS playlists(id INTEGER PRIMARY KEY, parent INTEGER NOT NULL, name TEXT NOT NULL,
                kind INTEGER NOT NULL, seq INTEGER NOT NULL, smart TEXT);
            CREATE TABLE IF NOT EXISTS playlist_tracks(playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, seq INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS pt_pl ON playlist_tracks(playlist_id, seq);
            CREATE TABLE IF NOT EXISTS mytags(id INTEGER PRIMARY KEY, parent INTEGER NOT NULL, name TEXT NOT NULL, seq INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS track_tags(track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                tag_id INTEGER NOT NULL REFERENCES mytags(id) ON DELETE CASCADE, PRIMARY KEY(track_id, tag_id));
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS match_enrichment(
                track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                path TEXT NOT NULL, artist TEXT NOT NULL, title TEXT NOT NULL, mix_name TEXT NOT NULL, evidence TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS native_edit_baseline(
                track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, field TEXT NOT NULL,
                source TEXT NOT NULL, native_id TEXT NOT NULL, uuid TEXT NOT NULL, path TEXT NOT NULL,
                value TEXT NOT NULL, proof TEXT NOT NULL, PRIMARY KEY(track_id,field));
            CREATE TRIGGER IF NOT EXISTS forget_deleted_rekordbox_mirror AFTER DELETE ON playlists
            BEGIN DELETE FROM settings WHERE key='rb_folder_id' AND value=CAST(OLD.id AS TEXT); END;
            CREATE TABLE IF NOT EXISTS peaks(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, peak REAL NOT NULL);
            CREATE TABLE IF NOT EXISTS media_stamps(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, size INTEGER NOT NULL, mtime INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS vocals(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, data BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS xml_extra(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS trash(id INTEGER PRIMARY KEY, track_id INTEGER NOT NULL, path TEXT NOT NULL,
                title TEXT NOT NULL, artist TEXT NOT NULL, removed_at INTEGER NOT NULL, json TEXT NOT NULL, wave BLOB, preview BLOB);
            CREATE TABLE IF NOT EXISTS histories(id INTEGER PRIMARY KEY, name TEXT NOT NULL, date TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS history_entries(id INTEGER PRIMARY KEY, history_id INTEGER NOT NULL REFERENCES histories(id) ON DELETE CASCADE,
                track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, played_at INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX IF NOT EXISTS he_hist ON history_entries(history_id, id);
            CREATE TABLE IF NOT EXISTS tag_list(track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, seq INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS track_matches(a INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                b INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, PRIMARY KEY(a, b));",
        )?;
        // Columns added after the first release, and the version stamp, in one transaction:
        // an interrupted upgrade leaves the library as it was (a backup was taken above).
        conn.execute_batch("BEGIN")?;
        let migrate = (|| -> Result<()> {
            // Older rows have no proof of origin. Deliberately leave them out.
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS direct_file_imports(
                    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE);
                 CREATE TRIGGER IF NOT EXISTS protect_changed_import AFTER UPDATE ON tracks
                 BEGIN DELETE FROM direct_file_imports WHERE track_id=OLD.id; END;"
            )?;
            if conn.prepare("SELECT raw_samples FROM tracks LIMIT 0").is_err() {
                conn.execute_batch("ALTER TABLE tracks ADD COLUMN raw_samples INTEGER DEFAULT 0;")?;
            }
            // Set when the user chose the key; analysis then never replaces it.
            if conn.prepare("SELECT key_manual FROM tracks LIMIT 0").is_err() {
                conn.execute_batch("ALTER TABLE tracks ADD COLUMN key_manual INTEGER DEFAULT 0;")?;
            }
            // The key's imported spelling (shown as is).
            if conn.prepare("SELECT key_text FROM tracks LIMIT 0").is_err() {
                conn.execute_batch("ALTER TABLE tracks ADD COLUMN key_text TEXT DEFAULT '';")?;
            }
            if timeline == Mp3Timeline::Rekordbox {
                conn.execute("INSERT OR IGNORE INTO settings(key,value) VALUES('mp3_timeline', ?1)", [timeline.name()])?;
            }
            track_ids::initialize(&conn)?;
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
            Ok(())
        })();
        match migrate {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        let db = Db { conn };
        db.seed_mytags()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn test_schema() {}

    fn seed_mytags(&self) -> Result<()> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM mytags", [], |r| r.get(0))?;
        if n > 0 {
            return Ok(());
        }
        let cats: [(&str, &[&str]); 4] = [
            ("Genre", &["Acid House", "Deep House", "Techno", "Nu Disco", "Electro House", "Bass Music", "Trap"]),
            ("Components", &["Synth", "Vocal", "Beat", "Sub Bass", "Percussion", "Piano", "Dark", "Upper"]),
            ("Situation", &["Main Floor", "Second Floor", "Lounge", "Mid Night", "Morning", "Build up", "Peak Time", "Build down"]),
            ("Untitled Column", &["My Comment"]),
        ];
        for (ci, (c, tags)) in cats.iter().enumerate() {
            self.conn.execute("INSERT INTO mytags(parent,name,seq) VALUES(0,?1,?2)", params![c, ci as i64])?;
            let cid = self.conn.last_insert_rowid();
            for (ti, t) in tags.iter().enumerate() {
                self.conn.execute("INSERT INTO mytags(parent,name,seq) VALUES(?1,?2,?3)", params![cid, t, ti as i64])?;
            }
        }
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.setting_checked(key).ok().flatten()
    }
    pub fn setting_checked(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()
    }
    pub fn set_setting(&self, key: &str, value: &str) {
        let _ = self.set_setting_checked(key, value);
    }
    pub fn set_setting_checked(&self, key: &str, value: &str) -> rusqlite::Result<usize> {
        self.conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)", params![key, value])
    }

    fn row_to_track(r: &rusqlite::Row) -> rusqlite::Result<Track> {
        Ok(Track {
            id: r.get(0)?,
            path: r.get(1)?,
            title: r.get(2)?,
            artist: r.get(3)?,
            album: r.get(4)?,
            album_artist: r.get(5)?,
            genre: r.get(6)?,
            label: r.get(7)?,
            remixer: r.get(8)?,
            original_artist: r.get(9)?,
            composer: r.get(10)?,
            lyricist: r.get(11)?,
            mix_name: r.get(12)?,
            comment: r.get(13)?,
            grouping: r.get(14)?,
            message: r.get(15)?,
            year: r.get(16)?,
            track_no: r.get(17)?,
            disc_no: r.get(18)?,
            release_date: r.get(19)?,
            duration: r.get(20)?,
            sample_rate: r.get(21)?,
            bitrate: r.get(22)?,
            bit_depth: r.get(23)?,
            file_size: r.get::<_, i64>(24)? as u64,
            file_type: r.get(25)?,
            bpm: r.get(26)?,
            key: r.get(27)?,
            rating: r.get(28)?,
            color: r.get(29)?,
            date_added: r.get(30)?,
            date_created: r.get(31)?,
            play_count: r.get(32)?,
            analyzed: r.get(33)?,
            grid_locked: r.get::<_, i64>(34)? != 0,
            autoload_hotcue: r.get::<_, i64>(35)? != 0,
            gain_db: r.get::<_, Option<f32>>(36)?.unwrap_or(f32::NAN),
            artwork: r.get(37)?,
            raw_samples: r.get::<_, i64>(38)? as u64,
            key_manual: r.get::<_, Option<i64>>(39)?.unwrap_or(0) != 0,
            key_text: r.get::<_, Option<String>>(40)?.unwrap_or_default(),
            missing: false,
        })
    }

    pub fn all_tracks(&self) -> Result<Vec<Track>> {
        let mut st = self.conn.prepare(&format!("SELECT {TRACK_COLS} FROM tracks ORDER BY id"))?;
        let v = st.query_map([], Self::row_to_track)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn track(&self, id: i64) -> Option<Track> {
        self.conn.query_row(&format!("SELECT {TRACK_COLS} FROM tracks WHERE id=?1"), [id], Self::row_to_track).optional().ok().flatten()
    }

    pub fn track_by_path(&self, path: &str) -> Option<i64> {
        self.conn.query_row("SELECT id FROM tracks WHERE path=?1", [path], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn track_at_path(&self, path: &str) -> rusqlite::Result<Option<Track>> {
        self.conn.query_row(&format!("SELECT {TRACK_COLS} FROM tracks WHERE path=?1"), [path], Self::row_to_track).optional()
    }

    pub fn set_track_path(&self, id: i64, path: &str) -> Result<()> {
        let changed = self.conn.execute("UPDATE tracks SET path=?1 WHERE id=?2", params![path, id])?;
        anyhow::ensure!(changed == 1, "The track is no longer in the collection");
        Ok(())
    }

    /// Insert a track; returns its new id.
    pub fn insert_track(&self, t: &Track) -> Result<i64> {
        // Allocate inside the INSERT so concurrent importers cannot choose the same ID.
        // Only a legacy ID at SQLite's exact ceiling requires checking for a free gap.
        self.conn.execute(
            "WITH RECURSIVE ceiling(n) AS (SELECT max(coalesce((SELECT max(id) FROM tracks),0),
                                                      coalesce((SELECT max(id) FROM retired_track_ids),0))),
             candidates(id) AS (SELECT CASE n WHEN 9223372036854775807 THEN 1+(random() & 0x3fffffffffffffff) ELSE n+1 END FROM ceiling
                                UNION ALL SELECT CASE id WHEN 9223372036854775807 THEN 1 ELSE id+1 END FROM candidates)
             INSERT INTO tracks(path,title,artist,album,album_artist,genre,label,remixer,original_artist,composer,lyricist,mix_name,comment,grouping,message,year,track_no,disc_no,release_date,duration,sample_rate,bitrate,bit_depth,file_size,file_type,bpm,key,rating,color,date_added,date_created,play_count,analyzed,grid_locked,autoload_hotcue,gain_db,artwork,raw_samples,key_manual,key_text,id)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37,?38,?39,?40,
                    (SELECT id FROM candidates WHERE NOT EXISTS(SELECT 1 FROM tracks t WHERE t.id=candidates.id)
                       AND NOT EXISTS(SELECT 1 FROM retired_track_ids r WHERE r.id=candidates.id) LIMIT 1))",
            params![
                t.path, t.title, t.artist, t.album, t.album_artist, t.genre, t.label, t.remixer, t.original_artist, t.composer,
                t.lyricist, t.mix_name, t.comment, t.grouping, t.message, t.year, t.track_no, t.disc_no, t.release_date,
                t.duration, t.sample_rate, t.bitrate, t.bit_depth, t.file_size as i64, t.file_type, t.bpm, t.key, t.rating,
                t.color, t.date_added, t.date_created, t.play_count, t.analyzed, t.grid_locked as i64,
                t.autoload_hotcue as i64, t.gain_db, t.artwork, t.raw_samples as i64, t.key_manual as i64, t.key_text
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The file importer actually read this track on this computer. Imported library rows,
    /// restored trash and preexisting collection records have no such evidence.
    pub fn insert_local_file(&self, t: &Track) -> Result<i64> {
        if t.file_size == 0 || t.sample_rate == 0 {
            // The file importer can keep an unparsed or inaccessible file by name alone.
            return self.insert_track(t);
        }
        let tx = self.conn.unchecked_transaction()?;
        let id = self.insert_track(t)?;
        anyhow::ensure!(self.conn.changes() == 1, "The file did not create a new track.");
        self.conn.execute("INSERT INTO direct_file_imports(track_id) VALUES(?1)", [id])?;
        tx.commit()?;
        Ok(id)
    }

    pub fn protect_imported_track(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM direct_file_imports WHERE track_id=?1", [id])?;
        Ok(())
    }

    pub fn update_track(&self, t: &Track) -> Result<()> {
        self.conn.execute(
            "UPDATE tracks SET title=?2,artist=?3,album=?4,album_artist=?5,genre=?6,label=?7,remixer=?8,original_artist=?9,composer=?10,lyricist=?11,mix_name=?12,comment=?13,grouping=?14,message=?15,year=?16,track_no=?17,disc_no=?18,release_date=?19,duration=?20,sample_rate=?21,bitrate=?22,bit_depth=?23,file_size=?24,file_type=?25,bpm=?26,key=?27,rating=?28,color=?29,date_added=?30,date_created=?31,play_count=?32,analyzed=?33,grid_locked=?34,autoload_hotcue=?35,gain_db=?36,artwork=?37,path=?38,raw_samples=?39,key_manual=?40,key_text=?41 WHERE id=?1",
            params![
                t.id, t.title, t.artist, t.album, t.album_artist, t.genre, t.label, t.remixer, t.original_artist, t.composer,
                t.lyricist, t.mix_name, t.comment, t.grouping, t.message, t.year, t.track_no, t.disc_no, t.release_date,
                t.duration, t.sample_rate, t.bitrate, t.bit_depth, t.file_size as i64, t.file_type, t.bpm, t.key, t.rating,
                t.color, t.date_added, t.date_created, t.play_count, t.analyzed, t.grid_locked as i64,
                t.autoload_hotcue as i64, t.gain_db, t.artwork, t.path, t.raw_samples as i64, t.key_manual as i64, t.key_text
            ],
        )?;
        Ok(())
    }

/// Move a track to the trash: its row, cues, grid, waveform, tags and playlist positions are
    /// kept so it can be restored with [`Db::restore_trash`].
    pub fn trash_track(&self, id: i64) -> Result<()> {
        let Some(track) = self.track(id) else { return Ok(()) };
        // Position in each playlist (rows before it), not the raw sequence number with its gaps.
        let mut st = self.conn.prepare("SELECT p.playlist_id, (SELECT COUNT(*) FROM playlist_tracks q WHERE q.playlist_id = p.playlist_id AND q.seq < p.seq) FROM playlist_tracks p WHERE p.track_id=?1")?;
        let playlists = st.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut st = self.conn.prepare("SELECT tag_id FROM track_tags WHERE track_id=?1")?;
        let tags = st.query_map([id], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let entry = TrashEntry { cues: self.cues(id)?, grid: self.grid(id), playlists, tags, xml_extra: self.xml_extra(id), vocal: self.vocal(id), peak: self.peak(id), stamp: self.media_stamps().get(&id).copied(), track: track.clone() };
        let wave: Option<(Vec<u8>, Vec<u8>)> = self.conn.query_row("SELECT data, preview FROM waves WHERE track_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.conn.execute(
            "INSERT INTO trash(track_id,path,title,artist,removed_at,json,wave,preview) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, track.path, track.display_title(), track.artist, now, serde_json::to_string(&entry)?, wave.as_ref().map(|w| &w.0), wave.as_ref().map(|w| &w.1)],
        )?;
        self.delete_track(id)
    }

    /// Trashed tracks, newest first: (trash id, title, artist, path, removed at, unix seconds).
    pub fn trash_list(&self) -> Result<Vec<(i64, String, String, String, i64)>> {
        let mut st = self.conn.prepare("SELECT id, title, artist, path, removed_at FROM trash ORDER BY removed_at DESC, id DESC")?;
        let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Put a trashed track back with everything it had (same id when still free). Returns the
    /// track id, or None when a track with the same file is in the collection again.
    pub fn restore_trash(&self, trash_id: i64) -> Result<Option<i64>> {
        let (json, wave, preview): (String, Option<Vec<u8>>, Option<Vec<u8>>) = self.conn.query_row("SELECT json, wave, preview FROM trash WHERE id=?1", [trash_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let e: TrashEntry = serde_json::from_str(&json)?;
        if self.track_by_path(&e.track.path).is_some() {
            return Ok(None);
        }
        let own = self.conn.is_autocommit();
        if own {
            self.conn.execute_batch("BEGIN")?;
        }
        let r = (|| -> Result<i64> {
            let mut id = self.insert_track(&e.track)?;
            if id != e.track.id && self.track(e.track.id).is_none() {
                self.conn.execute("UPDATE tracks SET id=?1 WHERE id=?2", params![e.track.id, id])?;
                id = e.track.id;
            }
            self.replace_cues(id, &e.cues)?;
            if let Some(g) = &e.grid {
                self.save_grid(id, g)?;
            }
            self.set_xml_extra(id, &e.xml_extra)?;
            if let Some(v) = &e.vocal {
                self.save_vocal(id, v)?;
            }
            if let Some(p) = e.peak {
                self.set_peak(id, p)?;
            }
            if let Some(st) = e.stamp {
                self.set_media_stamp(id, st)?;
            }
            if let (Some(w), Some(p)) = (wave, preview) {
                self.conn.execute("INSERT OR REPLACE INTO waves(track_id,data,preview) VALUES(?1,?2,?3)", params![id, w, p])?;
            }
            for tag in &e.tags {
                self.conn.execute("INSERT OR IGNORE INTO track_tags(track_id,tag_id) SELECT ?1, id FROM mytags WHERE id=?2", params![id, tag])?;
            }
            for (pl, seq) in &e.playlists {
                let exists: bool = self.conn.query_row("SELECT COUNT(*) FROM playlists WHERE id=?1", [pl], |r| r.get::<_, i64>(0))? > 0;
                if exists {
                    let mut list = self.playlist_tracks(*pl)?;
                    list.insert((*seq as usize).min(list.len()), id);
                    self.set_playlist_tracks(*pl, &list)?;
                }
            }
            self.conn.execute("DELETE FROM trash WHERE id=?1", [trash_id])?;
            Ok(id)
        })();
        if own {
            self.conn.execute_batch(if r.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
        }
        r.map(Some)
    }

    /// Permanently drop trash entries removed before `before` (unix seconds); all when None.
    pub fn purge_trash(&self, before: Option<i64>) -> Result<usize> {
        Ok(match before {
            Some(t) => self.conn.execute("DELETE FROM trash WHERE removed_at < ?1", [t])?,
            None => self.conn.execute("DELETE FROM trash", [])?,
        })
    }

/// Sample peak measured with the track's loudness (for the optional auto-gain peak cap).
    pub fn peak(&self, track: i64) -> Option<f32> {
        self.conn.query_row("SELECT peak FROM peaks WHERE track_id=?1", [track], |r| r.get::<_, f64>(0)).optional().ok().flatten().map(|v| v as f32)
    }
    pub fn set_peak(&self, track: i64, peak: f32) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO peaks(track_id,peak) VALUES(?1,?2)", params![track, peak as f64])?;
        Ok(())
    }

    /// Size and modification time of each audio file when Recordbox last analysed it.
    pub fn media_stamps(&self) -> std::collections::HashMap<i64, (i64, i64)> {
        let mut m = std::collections::HashMap::new();
        if let Ok(mut st) = self.conn.prepare("SELECT track_id, size, mtime FROM media_stamps") {
            if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))) {
                m.extend(rows.flatten());
            }
        }
        m
    }
    pub fn set_media_stamp(&self, track: i64, stamp: (i64, i64)) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO media_stamps(track_id,size,mtime) VALUES(?1,?2,?3)", params![track, stamp.0, stamp.1])?;
        Ok(())
    }

    /// Recordbox's vocal detection: one level 0–4 per 2048 samples at 44.1 kHz (as PVDI).
    pub fn vocal(&self, track: i64) -> Option<Vec<u8>> {
        self.conn.query_row("SELECT data FROM vocals WHERE track_id=?1", [track], |r| r.get(0)).optional().ok().flatten()
    }
    pub fn save_vocal(&self, track: i64, levels: &[u8]) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO vocals(track_id,data) VALUES(?1,?2)", params![track, levels])?;
        Ok(())
    }

    /// XML fields Recordbox doesn't model, kept from an XML import for the next XML export.
    pub fn xml_extra(&self, track: i64) -> crate::importer::XmlExtra {
        let s: Option<String> = self.conn.query_row("SELECT json FROM xml_extra WHERE track_id=?1", [track], |r| r.get(0)).optional().ok().flatten();
        s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn set_xml_extra(&self, track: i64, e: &crate::importer::XmlExtra) -> Result<()> {
        if e.is_empty() {
            self.conn.execute("DELETE FROM xml_extra WHERE track_id=?1", [track])?;
        } else {
            self.conn.execute("INSERT OR REPLACE INTO xml_extra(track_id,json) VALUES(?1,?2)", params![track, serde_json::to_string(e)?])?;
        }
        Ok(())
    }

    pub fn delete_track(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM tracks WHERE id=?1", [id])?;
        Ok(())
    }

    // ---- cues
    pub fn cues(&self, track: i64) -> Result<Vec<Cue>> {
        let mut st = self.conn.prepare("SELECT id,track_id,kind,slot,start,end_,color,comment,active_loop FROM cues WHERE track_id=?1 ORDER BY kind, slot, start")?;
        let v = st
            .query_map([track], |r| {
                Ok(Cue {
                    id: r.get(0)?,
                    track_id: r.get(1)?,
                    kind: if r.get::<_, i64>(2)? == 1 { CueKind::Hot } else { CueKind::Memory },
                    slot: r.get(3)?,
                    start: r.get(4)?,
                    end: r.get(5)?,
                    color: r.get(6)?,
                    comment: r.get(7)?,
                    active_loop: r.get::<_, i64>(8)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }
    pub fn tracks_with_cues(&self) -> Result<std::collections::HashMap<i64, (usize, usize)>> {
        let mut st = self.conn.prepare("SELECT track_id, SUM(kind=1), SUM(kind=0) FROM cues GROUP BY track_id")?;
        let mut m = std::collections::HashMap::new();
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        for r in rows.flatten() {
            m.insert(r.0, (r.1 as usize, r.2 as usize));
        }
        Ok(m)
    }
    /// Hot cue (start, slot, colour) per track, for the track list previews.
    pub fn hot_positions(&self) -> Result<std::collections::HashMap<i64, Vec<(f64, i32, u8)>>> {
        let mut st = self.conn.prepare("SELECT track_id, start, slot, color FROM cues WHERE kind=1 ORDER BY track_id, slot")?;
        let mut m: std::collections::HashMap<i64, Vec<(f64, i32, u8)>> = std::collections::HashMap::new();
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, i32>(2)?, r.get::<_, u8>(3)?)))?;
        for r in rows.flatten() {
            m.entry(r.0).or_default().push((r.1, r.2, r.3));
        }
        Ok(m)
    }
    pub fn save_cue(&self, c: &Cue) -> Result<i64> {
        let kind = if c.kind == CueKind::Hot { 1 } else { 0 };
        if c.id > 0 {
            self.conn.execute(
                "UPDATE cues SET kind=?2,slot=?3,start=?4,end_=?5,color=?6,comment=?7,active_loop=?8 WHERE id=?1",
                params![c.id, kind, c.slot, c.start, c.end, c.color, c.comment, c.active_loop as i64],
            )?;
            Ok(c.id)
        } else {
            self.conn.execute(
                "INSERT INTO cues(track_id,kind,slot,start,end_,color,comment,active_loop) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![c.track_id, kind, c.slot, c.start, c.end, c.color, c.comment, c.active_loop as i64],
            )?;
            Ok(self.conn.last_insert_rowid())
        }
    }
    pub fn delete_cue(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM cues WHERE id=?1", [id])?;
        Ok(())
    }
    /// A hot-cue slot is one pad even when an older library contains duplicate rows.
    /// Capture Undo and replace (or clear) the slot in one transaction.
    pub fn edit_hot_cue(&self, track: i64, slot: i32, replacement: Option<&Cue>) -> Result<(Vec<Cue>, Vec<Cue>)> {
        let tx = self.conn.unchecked_transaction()?;
        let before = self.cues(track)?;
        let count = before.iter().filter(|c| c.kind == CueKind::Hot && c.slot == slot).count();
        if count > 0 {
            let removed = tx.execute("DELETE FROM cues WHERE track_id=?1 AND kind=1 AND slot=?2", params![track, slot])?;
            anyhow::ensure!(removed == count, "The hot cue could not be removed");
        }
        if let Some(cue) = replacement {
            self.save_cue(cue)?;
        }
        let after = self.cues(track)?;
        anyhow::ensure!(after.iter().filter(|c| c.kind == CueKind::Hot && c.slot == slot).count() == usize::from(replacement.is_some()), "The hot cue could not be saved");
        tx.commit()?;
        Ok((before, after))
    }
    pub fn replace_cues(&self, track: i64, cues: &[Cue]) -> Result<()> {
        self.conn.execute("DELETE FROM cues WHERE track_id=?1", [track])?;
        for c in cues {
            let mut c = c.clone();
            c.id = 0;
            c.track_id = track;
            self.save_cue(&c)?;
        }
        Ok(())
    }

    /// Undo/Redo needs the inverse edit and the new row IDs from the same committed write.
    pub fn restore_cues(&self, track: i64, cues: &[Cue]) -> Result<(Vec<Cue>, Vec<Cue>)> {
        let tx = self.conn.unchecked_transaction()?;
        let previous = self.cues(track)?;
        self.replace_cues(track, cues)?;
        let restored = self.cues(track)?;
        anyhow::ensure!(restored.len() == cues.len(), "Could not restore every cue");
        tx.commit()?;
        Ok((previous, restored))
    }

    // ---- grid & waveform
    pub fn grid(&self, track: i64) -> Option<BeatGrid> {
        self.grid_checked(track).ok().flatten()
    }
    pub fn grid_checked(&self, track: i64) -> Result<Option<BeatGrid>> {
        let s: Option<String> = self.conn.query_row("SELECT json FROM grids WHERE track_id=?1", [track], |r| r.get(0)).optional()?;
        Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    /// Validate only the selected grids. A damaged or empty grid still needs analysis.
    /// Stream the same anchor type as BeatGrid without allocating its full vector.
    pub(crate) fn missing_grids(&self, ids: &[i64]) -> Result<Vec<i64>> {
        #[derive(serde::Deserialize)]
        struct GridPresence {
            #[serde(default, rename = "beat_list_signature")]
            _beat_list_signature: u64,
            #[serde(deserialize_with = "read_anchors")]
            anchors: bool,
        }
        fn read_anchors<'de, D: serde::Deserializer<'de>>(de: D) -> std::result::Result<bool, D::Error> {
            struct Anchors;
            impl<'de> serde::de::Visitor<'de> for Anchors {
                type Value = bool;
                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("an array of beat grid anchors")
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<bool, A::Error> {
                    let mut found = false;
                    while seq.next_element::<GridAnchor>()?.is_some() {
                        found = true;
                    }
                    Ok(found)
                }
            }
            de.deserialize_seq(Anchors)
        }
        let mut missing = Vec::new();
        let mut stmt = self.conn.prepare_cached(
            "SELECT selected.value, grids.json FROM json_each(?1) AS selected
             LEFT JOIN grids ON grids.track_id=selected.value",
        )?;
        for batch in ids.chunks(256) {
            let mut rows = stmt.query([serde_json::to_string(batch)?])?;
            while let Some(row) = rows.next()? {
                let complete = match row.get_ref(1)? {
                    rusqlite::types::ValueRef::Text(json) => {
                        serde_json::from_slice::<GridPresence>(json).is_ok_and(|grid| grid.anchors)
                    }
                    _ => false,
                };
                if !complete {
                    missing.push(row.get(0)?);
                }
            }
        }
        Ok(missing)
    }
    pub fn save_grid(&self, track: i64, g: &BeatGrid) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO grids(track_id,json) VALUES(?1,?2)", params![track, serde_json::to_string(g)?])?;
        Ok(())
    }
    /// Check for earlier analysis without reading a track-sized waveform blob.
    pub fn has_wave(&self, track: i64) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM waves WHERE track_id=?1)", [track], |row| row.get(0))?)
    }
    pub fn wave(&self, track: i64) -> Option<Waveform> {
        let b: Option<Vec<u8>> = self.conn.query_row("SELECT data FROM waves WHERE track_id=?1", [track], |r| r.get(0)).optional().ok().flatten();
        b.map(|b| Waveform::from_bytes(&b))
    }
    pub fn save_wave(&self, track: i64, w: &Waveform, preview: &[u8]) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO waves(track_id,data,preview) VALUES(?1,?2,?3)", params![track, w.to_bytes(), preview])?;
        Ok(())
    }
    pub fn previews(&self) -> Result<std::collections::HashMap<i64, Vec<u8>>> {
        let mut st = self.conn.prepare("SELECT track_id, preview FROM waves")?;
        let mut m = std::collections::HashMap::new();
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?.flatten() {
            m.insert(r.0, r.1);
        }
        Ok(m)
    }

    // ---- playlists
    pub fn playlists(&self) -> Result<Vec<PlaylistNode>> {
        let mut st = self.conn.prepare("SELECT id,parent,name,kind,seq,smart FROM playlists ORDER BY parent, seq, id")?;
        let v = st
            .query_map([], |r| {
                let kind: i64 = r.get(3)?;
                let smart: Option<String> = r.get(5)?;
                Ok(PlaylistNode {
                    id: r.get(0)?,
                    parent: r.get(1)?,
                    name: r.get(2)?,
                    kind: match kind {
                        1 => NodeKind::Folder,
                        2 => NodeKind::Smart,
                        _ => NodeKind::Playlist,
                    },
                    seq: r.get(4)?,
                    smart: smart.and_then(|s| serde_json::from_str::<SmartRule>(&s).ok()),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }
    pub fn create_node(&self, parent: i64, name: &str, kind: NodeKind, smart: Option<&SmartRule>) -> Result<i64> {
        let seq: i64 = self.conn.query_row("SELECT COALESCE(MAX(seq)+1,0) FROM playlists WHERE parent=?1", [parent], |r| r.get(0))?;
        let k = match kind {
            NodeKind::Playlist => 0,
            NodeKind::Folder => 1,
            NodeKind::Smart => 2,
        };
        let sj = smart.map(|s| serde_json::to_string(s).unwrap_or_default());
        self.conn.execute("INSERT INTO playlists(parent,name,kind,seq,smart) VALUES(?1,?2,?3,?4,?5)", params![parent, name, k, seq, sj])?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn rename_node(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute("UPDATE playlists SET name=?2 WHERE id=?1", params![id, name])?;
        Ok(())
    }
    pub fn move_node(&self, id: i64, parent: i64, seq: i64) -> Result<()> {
        self.conn.execute("UPDATE playlists SET seq=seq+1 WHERE parent=?1 AND seq>=?2", params![parent, seq])?;
        self.conn.execute("UPDATE playlists SET parent=?2, seq=?3 WHERE id=?1", params![id, parent, seq])?;
        Ok(())
    }
    pub fn set_smart(&self, id: i64, s: &SmartRule) -> Result<()> {
        self.conn.execute("UPDATE playlists SET smart=?2 WHERE id=?1", params![id, serde_json::to_string(s)?])?;
        Ok(())
    }
    pub fn delete_node(&self, id: i64) -> Result<()> {
        let kids: Vec<i64> = {
            let mut st = self.conn.prepare("SELECT id FROM playlists WHERE parent=?1")?;
            let v = st.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            v
        };
        for k in kids {
            self.delete_node(k)?;
        }
        self.conn.execute("DELETE FROM playlists WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn playlist_tracks(&self, id: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT track_id FROM playlist_tracks WHERE playlist_id=?1 ORDER BY seq")?;
        let v = st.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(v)
    }
    pub fn set_playlist_tracks(&self, id: i64, tracks: &[i64]) -> Result<()> {
        // Inside a caller's transaction (e.g. the rekordbox sync) write directly; otherwise
        // make the replacement atomic on its own.
        let own = self.conn.is_autocommit();
        if own {
            self.conn.execute_batch("BEGIN")?;
        }
        let r = (|| -> Result<()> {
            self.conn.execute("DELETE FROM playlist_tracks WHERE playlist_id=?1", [id])?;
            let mut st = self.conn.prepare("INSERT INTO playlist_tracks(playlist_id,track_id,seq) VALUES(?1,?2,?3)")?;
            for (i, t) in tracks.iter().enumerate() {
                st.execute(params![id, t, i as i64])?;
            }
            Ok(())
        })();
        if own {
            self.conn.execute_batch(if r.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
        }
        r
    }
    pub fn playlist_counts(&self) -> Result<std::collections::HashMap<i64, usize>> {
        let mut st = self.conn.prepare("SELECT playlist_id, COUNT(*) FROM playlist_tracks GROUP BY playlist_id")?;
        let mut m = std::collections::HashMap::new();
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.flatten() {
            m.insert(r.0, r.1 as usize);
        }
        Ok(m)
    }
    pub fn playlists_containing(&self, track: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT DISTINCT playlist_id FROM playlist_tracks WHERE track_id=?1")?;
        let v = st.query_map([track], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(v)
    }

    // ---- my tags
    pub fn mytags(&self) -> Result<Vec<MyTag>> {
        let mut st = self.conn.prepare("SELECT id,parent,name,seq FROM mytags ORDER BY parent, seq, id")?;
        let v = st
            .query_map([], |r| Ok(MyTag { id: r.get(0)?, parent: r.get(1)?, name: r.get(2)?, seq: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }
    pub fn add_mytag(&self, parent: i64, name: &str) -> Result<i64> {
        let seq: i64 = self.conn.query_row("SELECT COALESCE(MAX(seq)+1,0) FROM mytags WHERE parent=?1", [parent], |r| r.get(0))?;
        self.conn.execute("INSERT INTO mytags(parent,name,seq) VALUES(?1,?2,?3)", params![parent, name, seq])?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn rename_mytag(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute("UPDATE mytags SET name=?2 WHERE id=?1", params![id, name])?;
        Ok(())
    }
    pub fn delete_mytag(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM mytags WHERE parent=?1", [id])?;
        self.conn.execute("DELETE FROM mytags WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn track_tags(&self) -> Result<std::collections::HashMap<i64, std::collections::HashSet<i64>>> {
        let mut st = self.conn.prepare("SELECT track_id, tag_id FROM track_tags")?;
        let mut m: std::collections::HashMap<i64, std::collections::HashSet<i64>> = std::collections::HashMap::new();
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.flatten() {
            m.entry(r.0).or_default().insert(r.1);
        }
        Ok(m)
    }
    pub fn set_track_tag(&self, track: i64, tag: i64, on: bool) -> Result<()> {
        if on {
            self.conn.execute("INSERT OR IGNORE INTO track_tags(track_id,tag_id) VALUES(?1,?2)", params![track, tag])?;
        } else {
            self.conn.execute("DELETE FROM track_tags WHERE track_id=?1 AND tag_id=?2", params![track, tag])?;
        }
        Ok(())
    }
}

/// What a trashed track had in the library.
#[derive(serde::Serialize, serde::Deserialize)]
struct TrashEntry {
    track: Track,
    cues: Vec<Cue>,
    grid: Option<BeatGrid>,
    /// (playlist id, position among its tracks when removed)
    playlists: Vec<(i64, i64)>,
    tags: Vec<i64>,
    #[serde(default)]
    xml_extra: crate::importer::XmlExtra,
    #[serde(default)]
    vocal: Option<Vec<u8>>,
    #[serde(default)]
    peak: Option<f32>,
    #[serde(default)]
    stamp: Option<(i64, i64)>,
}

#[cfg(test)]
mod sqlcipher_tests;

#[cfg(test)]
mod schema_tests {
    use super::*;

    #[test]
    fn trash_and_restore_keep_everything() {
        let dir = std::env::temp_dir().join(format!("recordbox_trash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("library.db")).unwrap();
        let mk = |p: &str| Track { path: p.into(), title: p.into(), artist: "X".into(), ..Default::default() };
        let a = db.insert_track(&mk("/a.mp3")).unwrap();
        let b = db.insert_track(&mk("/b.mp3")).unwrap();
        let c = db.insert_track(&mk("/c.mp3")).unwrap();
        let pl = db.create_node(0, "Set", NodeKind::Playlist, None).unwrap();
        db.set_playlist_tracks(pl, &[a, b, c]).unwrap();
        let tag = db.add_mytag(0, "Peak").unwrap();
        db.set_track_tag(b, tag, true).unwrap();
        db.save_cue(&Cue { id: 0, track_id: b, kind: CueKind::Hot, slot: 1, start: 3.0, end: None, color: 5, comment: "in".into(), active_loop: false }).unwrap();
        db.save_grid(b, &BeatGrid::constant(0.2, 124.0, 1)).unwrap();
        db.trash_track(b).unwrap();
        assert!(db.track(b).is_none());
        assert_eq!(db.playlist_tracks(pl).unwrap(), vec![a, c]);
        let list = db.trash_list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(db.restore_trash(list[0].0).unwrap(), Some(b));
        assert_eq!(db.playlist_tracks(pl).unwrap(), vec![a, b, c]);
        assert_eq!(db.cues(b).unwrap()[0].comment, "in");
        assert!(db.grid(b).is_some());
        assert!(db.track_tags().unwrap()[&b].contains(&tag));
        assert!(db.trash_list().unwrap().is_empty());
        // Two removed from the middle come back in their places when restored newest first.
        db.trash_track(b).unwrap();
        db.trash_track(c).unwrap();
        assert_eq!(db.playlist_tracks(pl).unwrap(), vec![a]);
        let mut ids: Vec<i64> = db.trash_list().unwrap().into_iter().map(|e| e.0).collect();
        ids.sort_by(|x, y| y.cmp(x));
        for t in ids {
            db.restore_trash(t).unwrap();
        }
        assert_eq!(db.playlist_tracks(pl).unwrap(), vec![a, b, c]);
        // A track re-imported meanwhile is not duplicated.
        db.trash_track(a).unwrap();
        db.insert_track(&mk("/a.mp3")).unwrap();
        let t = db.trash_list().unwrap()[0].0;
        assert_eq!(db.restore_trash(t).unwrap(), None);
        assert_eq!(db.purge_trash(None).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unavailable_gain_stays_unavailable() {
        let dir = std::env::temp_dir().join(format!("recordbox_gain_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("library.db")).unwrap();
        let id = db.insert_track(&Track { path: "/s.wav".into(), gain_db: f32::NAN, ..Default::default() }).unwrap();
        assert!(db.track(id).unwrap().gain_db.is_nan());
        // Through the trash (JSON) and back.
        db.trash_track(id).unwrap();
        let t = db.trash_list().unwrap()[0].0;
        let back = db.restore_trash(t).unwrap().unwrap();
        assert!(db.track(back).unwrap().gain_db.is_nan());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_spelling_survives_storage() {
        let dir = std::env::temp_dir().join(format!("recordbox_keytext_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("library.db")).unwrap();
        let mut t = Track { path: "/a.mp3".into(), key: crate::model::parse_key("7A"), key_text: "7A".into(), ..Default::default() };
        t.id = db.insert_track(&t).unwrap();
        assert_eq!(db.track(t.id).unwrap().key_text, "7A");
        t.set_key(crate::model::parse_key("Fm"));
        db.update_track(&t).unwrap();
        let back = db.track(t.id).unwrap();
        assert_eq!((back.key_label(false), back.key_text.as_str()), ("Fm".to_string(), ""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upgrades_with_backup_and_refuses_newer() {
        let dir = std::env::temp_dir().join(format!("recordbox_schema_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("library.db");
        // An old, unversioned library.
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch("CREATE TABLE tracks(id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL);").unwrap();
        }
        let db = Db::open(&p).unwrap();
        let v: i64 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        drop(db);
        let backups = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".bak.db")).count();
        assert_eq!(backups, 1);
        // Reopening the current version makes no new backup.
        drop(Db::open(&p).unwrap());
        assert_eq!(std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".bak.db")).count(), 1);
        // A newer schema is refused.
        Connection::open(&p).unwrap().execute_batch("PRAGMA user_version = 99;").unwrap();
        assert!(Db::open(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
