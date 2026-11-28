//! Preserve the identity of tracks referenced by edits, stem exports and Removed Tracks.
//! Old links store only row IDs. An ID already reassigned in an older release is ambiguous.
use anyhow::Result;
use rusqlite::Connection;

fn positive_id(text: &str) -> Option<i64> {
    if text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

pub(crate) fn visit_derived_links(
    db: &Connection,
    mut visit: impl FnMut(Option<i64>, Option<i64>) -> Result<()>,
) -> Result<()> {
    let mut statement = db.prepare(
        "SELECT key,value FROM settings WHERE key GLOB 'edit_of_*' OR key GLOB 'stem_of_*'",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let key = row.get_ref(0)?.as_str().ok();
        let value = row.get_ref(1)?.as_str().ok();
        let (child, source) = if let Some(child) = key.and_then(|key| key.strip_prefix("edit_of_"))
        {
            (Some(child), value)
        } else {
            let child = key.and_then(|key| key.strip_prefix("stem_of_"));
            let source = value
                .and_then(|value| value.split_once(':'))
                .and_then(|(id, part)| {
                    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase())).then_some(id)
                });
            (child, source)
        };
        visit(child.and_then(positive_id), source.and_then(positive_id))?;
    }
    Ok(())
}

/// Called inside the private schema transaction. Existing markers and track rows stay intact.
pub(super) fn initialize(db: &Connection) -> Result<()> {
    let initialized: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='retired_track_ids')",
        [], |r| r.get(0),
    )?;
    if !initialized {
        db.execute_batch(
            "CREATE TABLE retired_track_ids(id INTEGER PRIMARY KEY CHECK(id>0));
             INSERT OR IGNORE INTO retired_track_ids SELECT track_id FROM trash
             WHERE typeof(track_id)='integer' AND track_id>0;",
        )?;
        let mut reserve = db.prepare("INSERT OR IGNORE INTO retired_track_ids VALUES(?1)")?;
        visit_derived_links(db, |child, source| {
            for id in [child, source].into_iter().flatten() {
                reserve.execute([id])?;
            }
            Ok(())
        })?;
    }
    db.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS reserve_deleted_track_id AFTER DELETE ON tracks WHEN OLD.id>0
         BEGIN INSERT OR IGNORE INTO retired_track_ids VALUES(OLD.id); END;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Db, model::Track};

    fn track(path: &str) -> Track {
        Track {
            path: path.into(),
            ..Default::default()
        }
    }

    #[test]
    fn derived_ids_survive_trash_new_imports_and_restore() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("library.db");
        let db = Db::open(&path)?;
        let source = db.insert_track(&track("Original.wav"))?;
        let child = db.insert_track(&track("Vocals.wav"))?;
        let key = crate::stem_export::link_key(child);
        db.set_setting_checked(&key, &format!("{source}:vocals"))?;
        db.trash_track(child)?;
        db.trash_track(source)?;
        let second = Db::open(&path)?;
        let replacement = second.insert_track(&track("Unrelated.wav"))?;
        assert!(replacement > child);
        assert!(second
            .setting(&crate::stem_export::link_key(replacement))
            .is_none());
        for (trash, _, _, path, _) in db.trash_list()? {
            let original = if path == "Original.wav" {
                source
            } else {
                child
            };
            assert_eq!(db.restore_trash(trash)?, Some(original));
        }
        assert_eq!(db.setting(&key), Some(format!("{source}:vocals")));
        assert_eq!(db.track(source).unwrap().path, "Original.wav");
        assert_eq!(db.track(child).unwrap().path, "Vocals.wav");
        assert_eq!(db.track(replacement).unwrap().path, "Unrelated.wav");
        Ok(())
    }

    #[test]
    fn upgrade_reserves_legacy_links_and_trash_without_rewriting_them() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("library.db");
        let db = Db::open(&path)?;
        let id = db.insert_track(&track("Recover.wav"))?;
        db.trash_track(id)?;
        db.conn.execute_batch(
            "INSERT INTO trash(track_id,path,title,artist,removed_at,json)
             VALUES('invalid','corrupt','','',0,'{}');",
        )?;
        db.conn.execute_batch(
            "DROP TRIGGER reserve_deleted_track_id; DROP TABLE retired_track_ids;",
        )?;
        db.set_setting_checked("edit_of_8", "9")?;
        db.set_setting_checked("stem_of_10", "15:instrumental")?;
        db.set_setting_checked("stem_of_7", "9223372036854775807bad:vocals")?;
        drop(db);
        let db = Db::open(&path)?;
        let seeded = db
            .conn
            .prepare("SELECT id FROM retired_track_ids ORDER BY id")?
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        assert_eq!(seeded, [id, 7, 8, 9, 10, 15]);
        assert_eq!(db.insert_track(&track("Next.wav"))?, 16);
        assert_eq!(
            db.setting("stem_of_7").as_deref(),
            Some("9223372036854775807bad:vocals")
        );
        // An actual SQLite maximum may exist. Imports still avoid occupied and retired IDs.
        db.conn
            .execute("UPDATE tracks SET id=?1 WHERE id=16", [i64::MAX])?;
        db.delete_track(i64::MAX)?;
        let next = db.insert_track(&track("After ceiling.wav"))?;
        assert!(next > 0 && next != i64::MAX && !seeded.contains(&next));
        Ok(())
    }

    #[test]
    fn detected_alphatheta_catalogs_are_refused_without_schema_writes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for (name, schema) in [
            (
                "master.db",
                "CREATE TABLE djmdContent(ID INTEGER PRIMARY KEY);",
            ),
            (
                "exportLibrary.db",
                "CREATE TABLE property(dbVersion INTEGER); CREATE TABLE content(ID INTEGER);",
            ),
        ] {
            let path = dir.path().join(name);
            Connection::open(&path)?.execute_batch(schema)?;
            let before = std::fs::read(&path)?;
            assert!(Db::open(&path)
                .err()
                .unwrap()
                .to_string()
                .contains("AlphaTheta"));
            assert_eq!(std::fs::read(path)?, before);
        }
        let private = dir.path().join("library.db");
        Connection::open(&private)?.execute_batch("CREATE TABLE user_extension(value TEXT);")?;
        let db = Db::open(&private)?;
        assert!(db.insert_track(&track("Private.wav"))? > 0);
        assert!(db.conn.prepare("SELECT value FROM user_extension").is_ok());
        Ok(())
    }
}
