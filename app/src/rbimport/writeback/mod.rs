//! Reviewed native edits. Evidence is recorded only where a local field and a
//! specific native record agreed during a desktop import.
pub(crate) mod cues;
mod embedding;
pub(super) mod grid;
#[cfg(test)]
mod integration_tests;
mod location;
mod registration;
mod storage;

use super::RbTrack;
use crate::db::Db;
use crate::model::{key_name, parse_key, BeatGrid, Track};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, types::Value, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    folder: PathBuf,
    salt: String,
}

impl Source {
    pub(super) fn read(c: &Connection, folder: &Path) -> Result<Self> {
        // SQLCipher supplies the salt of the open handle (including a detached snapshot).
        let salt: String = c.query_row("PRAGMA cipher_salt", [], |r| r.get(0))?;
        ensure!(
            salt.len() == 32 && salt.bytes().all(|ch| ch.is_ascii_hexdigit()),
            "Encrypted library identity unavailable"
        );
        Ok(Self {
            folder: folder.canonicalize()?,
            salt,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Field {
    Rating,
    Bpm,
    Key,
    Cues,
    Grid,
}

impl Field {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Rating => "Rating",
            Self::Bpm => "BPM",
            Self::Key => "Key",
            Self::Cues => "Cues",
            Self::Grid => "Beat grid",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Edit {
    Rating(i64),
    Bpm(i64),
    Key(i32),
    Cues(cues::CueView),
    Grid(BeatGrid),
}

impl Edit {
    fn same_value(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Grid(a), Self::Grid(b)) => crate::app::grids_match(a, b),
            _ => self == other,
        }
    }

    fn field(&self) -> Field {
        match self {
            Self::Rating(_) => Field::Rating,
            Self::Bpm(_) => Field::Bpm,
            Self::Key(_) => Field::Key,
            Self::Cues(_) => Field::Cues,
            Self::Grid(_) => Field::Grid,
        }
    }

    fn display(&self) -> String {
        match self {
            Self::Rating(n) => format!("{n}/5"),
            Self::Bpm(n) => format!("{:.2}", *n as f64 / 100.0),
            Self::Key(n) if *n < 0 => "Unset".into(),
            Self::Key(n) => key_name(*n, false),
            Self::Cues(c) => c.summary(),
            Self::Grid(g) if g.has_listed_beats() => format!("{} individually timed beats", g.anchors.len()),
            Self::Grid(g) => match g.anchors.first() {
                Some(a) => format!(
                    "{:.3} s, beat {}, {:.2} BPM ({} anchors)",
                    a.time,
                    a.beat,
                    a.bpm,
                    g.anchors.len()
                ),
                None => "Empty grid".into(),
            },
        }
    }
}

pub(super) struct ImportTrack {
    uuid: String,
    rating: Option<(i64, String)>,
    bpm: Option<(i64, String)>,
    key_id: Option<String>,
    cue_ids: Option<String>,
    grid_proof: Option<String>,
}

pub(super) fn empty_cue_identity() -> String {
    format!("{:x}", Sha256::digest([]))
}

fn grid_proof(path: &str, hash: &[u8; 32]) -> String {
    format!(
        "{}:{}",
        path,
        hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

impl ImportTrack {
    pub(super) fn from_row(
        row: &HashMap<String, Value>,
        cue_ids: Option<String>,
        anlz: &str,
        dat: Option<&[u8]>,
    ) -> Option<Self> {
        let uuid = super::text(row.get("UUID")?.clone());
        // A missing/generic identifier cannot distinguish an independently re-created track.
        if !valid_uuid(&uuid) {
            return None;
        }
        let integer = |col| row.get(col).and_then(native_number);
        Some(Self {
            uuid,
            rating: integer("Rating").filter(|(v, _)| (0..=5).contains(v)),
            bpm: integer("BPM").filter(|(v, _)| (0..=65535).contains(v)),
            key_id: row.get("KeyID").cloned().map(super::text),
            cue_ids,
            grid_proof: if anlz.is_empty() {
                Some("absent".into())
            } else {
                dat.map(|b| grid_proof(anlz, &Sha256::digest(b).into()))
            },
        })
    }
}

fn valid_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, ch)| {
            if [8, 13, 18, 23].contains(&i) {
                ch == b'-'
            } else {
                ch.is_ascii_hexdigit()
            }
        })
        && id.bytes().any(|b| b != b'0' && b != b'-')
}

fn native_number(raw: &Value) -> Option<(i64, String)> {
    match raw {
        Value::Null => Some((0, "null".into())),
        Value::Integer(n) => Some((*n, n.to_string())),
        _ => None,
    }
}

fn local_edit(db: &Db, t: &Track, field: Field) -> Result<Edit> {
    Ok(match field {
        Field::Rating => Edit::Rating(t.rating.into()),
        Field::Bpm => Edit::Bpm((t.bpm * 100.0).round() as i64),
        Field::Key => Edit::Key(t.key),
        Field::Cues => Edit::Cues(cues::local(&db.cues(t.id)?)?),
        Field::Grid => Edit::Grid(db.grid_checked(t.id)?.unwrap_or_default()),
    })
}

pub(super) fn record_import(
    db: &Db,
    id: i64,
    rt: &RbTrack,
    source: &str,
    fresh: bool,
) -> Result<()> {
    let Some(native) = &rt.native else {
        return Ok(());
    };
    // A replaced native ID or a second library must never inherit the first association.
    db.conn.execute(
        "DELETE FROM native_edit_baseline WHERE track_id=?1 AND
        (source!=?2 OR native_id!=?3 OR uuid!=?4 OR path!=?5)",
        params![id, source, rt.rb_id, native.uuid, rt.xt.track.path],
    )?;
    // Fresh rows have just received the native track, cues and grid in this
    // transaction. Existing rows may have edits that the importer preserved.
    let local = if fresh {
        None
    } else {
        Some(
            db.track_at_path(&rt.xt.track.path)?
                .context("imported track is missing")?,
        )
    };
    let mut fields = Vec::with_capacity(5);
    if let Some((r, p)) = &native.rating {
        fields.push((Edit::Rating(*r), p.clone()));
    }
    if let Some((b, p)) = &native.bpm {
        fields.push((Edit::Bpm(*b), p.clone()));
    }
    if let Some(k) = &native.key_id {
        if rt.xt.track.key >= 0 || k.is_empty() || k == "0" {
            fields.push((Edit::Key(rt.xt.track.key), k.clone()));
        }
    }
    if let Some(ids) = &native.cue_ids {
        if let Ok(view) = cues::local(&rt.xt.cues) {
            fields.push((Edit::Cues(view), ids.clone()));
        }
    }
    if let Some(digest) = &native.grid_proof {
        fields.push((
            Edit::Grid(rt.xt.grid.clone().unwrap_or_default()),
            digest.clone(),
        ));
    }
    let mut save = db.conn.prepare_cached(
        "INSERT INTO native_edit_baseline
        (track_id,field,source,native_id,uuid,path,value,proof) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
        ON CONFLICT(track_id,field) DO UPDATE SET value=excluded.value,proof=excluded.proof
        WHERE value!=excluded.value OR proof!=excluded.proof",
    )?;
    for (field, proof) in fields {
        if local
            .as_ref()
            .is_none_or(|t| local_edit(db, t, field.field()).is_ok_and(|l| l.same_value(&field)))
        {
            save.execute(params![
                id,
                field.field().label(),
                source,
                rt.rb_id,
                native.uuid,
                rt.xt.track.path,
                serde_json::to_string(&field)?,
                proof
            ])?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
struct Baseline {
    local_id: i64,
    id: String,
    uuid: String,
    path: String,
    field: Edit,
    // Apply compares the reviewed SQLite text under the writer lock.
    stored_value: String,
    proof: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub track: String,
    pub local_id: i64,
    pub companion: Option<Field>,
    pub field: Field,
    pub before: String,
    pub current: String,
    pub proposed: String,
    pub reason: Option<String>,
    baseline: Baseline,
    desired: Edit,
    guard: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub rows: Vec<Row>,
    pub tracked: usize,
    source: Source,
    local_library: PathBuf,
}

#[derive(Debug, Default)]
pub(crate) struct RecoveryFiles {
    pub backup: Option<PathBuf>,
    pub remaining_staging: Option<PathBuf>,
}

pub(crate) struct Receipt {
    pub applied: usize,
    pub backup: Option<PathBuf>,
    pub remaining_staging: Option<PathBuf>,
}

fn local_db(path: &Path, write: bool) -> Result<Db> {
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let conn = Connection::open_with_flags(path, flags)?;
    conn.busy_timeout(std::time::Duration::from_millis(500))?;
    Ok(Db { conn })
}

const READ_BASELINES:&str="SELECT track_id,native_id,uuid,path,value,proof FROM native_edit_baseline WHERE source=? ORDER BY track_id,field";
fn read_baseline(r: &rusqlite::Row<'_>) -> Result<Baseline> {
    let stored_value: String = r.get(4)?;
    Ok(Baseline {
        local_id: r.get(0)?,
        id: r.get(1)?,
        uuid: r.get(2)?,
        path: r.get(3)?,
        field: serde_json::from_str(&stored_value)?,
        stored_value,
        proof: r.get(5)?,
    })
}

#[cfg(test)]
fn baselines(db: &Db, source: &Source) -> Result<Vec<Baseline>> {
    let mut st = db.conn.prepare(READ_BASELINES)?;
    let mut iter = st.query([serde_json::to_string(source)?])?;
    let mut out = vec![];
    while let Some(row) = iter.next()? {
        out.push(read_baseline(row)?);
    }
    Ok(out)
}

pub(crate) fn pending(source_dir: &Path) -> Result<bool> {
    Ok(storage::pending(source_dir))
}
pub(crate) fn backup_root(local: &Path) -> Result<PathBuf> {
    Ok(local
        .parent()
        .context("Missing Recordbox data folder")?
        .join("native-recovery"))
}
pub(crate) fn recover(local: &Path, source: &Path) -> Result<RecoveryFiles> {
    storage::recover(source, &backup_root(local)?)
}

/// The desktop reader accepts several spellings for display. A foreign-key writer
/// requires complete, recognizable key names (e.g. "Am" or "8A").
fn recognized_key(s: &str) -> Option<i32> {
    let s = s.trim();
    let idx = parse_key(s);
    if idx < 0 {
        return None;
    }
    let mut chars = s.chars();
    let first = chars.next()?;
    if first.is_ascii_digit() {
        // Open Key uses a different wheel numbering. Only Camelot A/B
        // identifiers here have a validated native mapping.
        return (s.len() <= 3 && s.ends_with(['A', 'B', 'a', 'b'])).then_some(idx);
    }
    let suffix = chars.as_str();
    let rest = suffix
        .strip_prefix(['#', '♯', 'b', '♭'])
        .unwrap_or(suffix)
        .trim()
        .to_ascii_lowercase();
    matches!(rest.as_str(), "" | "m" | "min" | "minor" | "maj" | "major").then_some(idx)
}

fn resolve_key(c: &Connection, desired: i32) -> Result<String> {
    let mut st = c.prepare_cached(
        "SELECT CAST(ID AS TEXT),ScaleName FROM djmdKey WHERE rb_local_deleted=0",
    )?;
    let mut result = None;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        if recognized_key(&r.get::<_, String>(1)?) == Some(desired) {
            ensure!(
                result.is_none(),
                "More than one native key matches this key"
            );
            result = Some(r.get(0)?);
        }
    }
    result.context("This key has no matching native key row")
}

fn registered_files(c: &Connection, id: &str) -> Result<bool> {
    let table: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='contentFile' COLLATE NOCASE)",
        [],
        |r| r.get(0),
    )?;
    Ok(table
        && c.query_row(
            "SELECT EXISTS(SELECT 1 FROM contentFile WHERE ContentID=? AND rb_local_deleted=0)",
            [id],
            |r| r.get(0),
        )?)
}

fn next_grid_revision(previous: Option<&str>) -> Result<String> {
    let mut units = previous.unwrap_or_default().encode_utf16();
    let first = units.next().unwrap_or(0);
    ensure!(
        units.next().is_none(),
        "Unrecognized native AnalysisUpdated revision"
    );
    let next = first
        .checked_add(1)
        .context("Native analysis revision overflow")?;
    String::from_utf16(&[next]).context("Unsupported native analysis revision")
}

fn read_counter(c: &Connection) -> Result<i64> {
    let mut st = c.prepare_cached(
        "SELECT int_1,updated_at FROM agentRegistry WHERE registry_id='localUpdateCount'",
    )?;
    let mut rows = st.query([])?;
    let n = rows
        .next()?
        .context("Native update counter is missing")?
        .get::<_, i64>(0)?;
    ensure!(
        n >= 0 && rows.next()?.is_none(),
        "Invalid native update counter"
    );
    Ok(n)
}

struct Current {
    value: Edit,
    proof: String,
    guard: String,
    write: Mutation,
    companion: Option<Field>,
}

enum Mutation {
    Column(&'static str, Value),
    Cues(cues::NativeCues, cues::CuePatch),
    Grid(Vec<storage::FileChange>, String),
    ObserveOnly,
}

// Build once per snapshot. Native copies do not always index FolderPath, UUID or
// AnalysisDataPath, and a count query per edited field would rescan the entire library.
struct Owners {
    duplicate_ids: HashSet<String>,
    duplicate_paths: HashSet<String>,
    shared_analysis: HashSet<String>,
}
impl Owners {
    fn read(c: &Connection) -> Result<Self> {
        fn duplicates(c: &Connection, expr: &str) -> Result<HashSet<String>> {
            let mut st = c.prepare(&format!(
                "SELECT {expr} AS identity FROM djmdContent
                WHERE rb_local_deleted=0 AND {expr} IS NOT NULL GROUP BY {expr} HAVING COUNT(*)>1"
            ))?;
            let values = st.query_map([], |r| r.get::<_, String>(0))?;
            Ok(values.collect::<rusqlite::Result<_>>()?)
        }
        Ok(Self {
            duplicate_ids: duplicates(c, "lower(UUID)")?,
            duplicate_paths: duplicates(c, "lower(replace(FolderPath,char(92),'/'))")?,
            shared_analysis: duplicates(
                c,
                "lower(ltrim(replace(AnalysisDataPath,char(92),'/'),'/'))",
            )?,
        })
    }
}

fn checked_current(
    c: &Connection,
    owners: &Owners,
    source: &Source,
    baseline: &Baseline,
    desired: &Edit,
) -> Result<Current> {
    let mut stmt =
        c.prepare_cached("SELECT * FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0")?;
    let mut rows = stmt.query([&baseline.id])?;
    let r = rows.next()?.context("Native track is missing or deleted")?;
    let uuid: String = r.get("UUID")?;
    let path: String = r.get("FolderPath")?;
    ensure!(
        uuid == baseline.uuid && path == baseline.path,
        "Native track identity or path changed"
    );
    ensure!(
        !owners.duplicate_ids.contains(&uuid.to_ascii_lowercase())
            && !owners
                .duplicate_paths
                .contains(&path.replace('\\', "/").to_ascii_lowercase()),
        "Native track path or identity is ambiguous"
    );
    for name in ["rb_local_usn", "updated_at", "rb_local_synced", "ServiceID"] {
        ensure!(
            r.as_ref().column_index(name).is_ok(),
            "Unsupported native update schema: missing {name}"
        );
    }
    // rb_file_id also exists on ordinary local media in native v6 and v7 catalogs.
    for col in ["ServiceID", "CloudID"] {
        if r.as_ref().column_index(col).is_err() {
            continue;
        }
        let value = super::text(r.get::<_, Value>(col)?);
        ensure!(
            value.is_empty() || value == "0",
            "Cloud-associated track; local write-back unavailable"
        );
    }
    let analysis = r
        .get::<_, Option<String>>("AnalysisDataPath")?
        .unwrap_or_default();
    let share = source.folder.join("share");
    if matches!(desired, Edit::Grid(_) | Edit::Bpm(_) | Edit::Key(_)) {
        if r.as_ref().column_index("AnalysisLock").is_ok() {
            ensure!(
                r.get::<_, Option<i64>>("AnalysisLock")?.unwrap_or(0) == 0,
                "Native analysis is locked"
            );
        }
        if r.as_ref().column_index("Analysed").is_ok() {
            ensure!(
                r.get::<_, Option<i64>>("Analysed")?.unwrap_or(0) & 128 == 0,
                "Native analysis is locked"
            );
        }
    }
    let guard = |value: Edit, proof: String, mutation: Mutation| Current {
        value,
        guard: proof.clone(),
        proof,
        write: mutation,
        companion: None,
    };
    Ok(match desired {
        Edit::Rating(v) => {
            ensure!((0..=5).contains(v), "Rating must be 0–5");
            let (n, raw) =
                native_number(&r.get("Rating")?).context("Invalid native rating representation")?;
            guard(
                Edit::Rating(n),
                raw,
                Mutation::Column("Rating", Value::Integer(*v)),
            )
        }
        Edit::Bpm(v) => {
            ensure!(
                (1..=65535).contains(v),
                "BPM exceeds native precision or range"
            );
            let (n, raw) =
                native_number(&r.get("BPM")?).context("Invalid native tempo representation")?;
            let mut current = guard(
                Edit::Bpm(n),
                raw,
                Mutation::Column("BPM", Value::Integer(*v)),
            );
            if n != *v {
                if analysis.is_empty() {
                    ensure!(!registered_files(c, &baseline.id)?, "Native file registrations exist without a recorded analysis path. Cannot verify BPM mirrors");
                } else {
                    registration::check_inventory(c, &baseline.id, &share, &analysis)?;
                    let snap = grid::inspect(&share, &analysis)?;
                    snap.ensure_timing_editable()?;
                    if snap.grid.anchors.len() == 1
                        && (snap.grid.first_bpm() * 100.0).round() as i64 == n
                    {
                        current.companion = Some(Field::Grid);
                    }
                }
            }
            current
        }
        Edit::Key(v) => {
            let current_id = super::text(r.get("KeyID")?);
            let current_key = if current_id.is_empty() || current_id == "0" {
                -1
            } else {
                let name: String = c.query_row(
                    "SELECT ScaleName FROM djmdKey WHERE ID=? AND rb_local_deleted=0",
                    [&current_id],
                    |r| r.get(0),
                )?;
                recognized_key(&name).context("Unrecognized native key name")?
            };
            let new_id = resolve_key(c, *v)?;
            Current {
                value: Edit::Key(current_key),
                proof: current_id.clone(),
                guard: serde_json::to_string(&(&current_id, &new_id))?,
                write: Mutation::Column("KeyID", Value::Text(new_id)),
                companion: None,
            }
        }
        Edit::Cues(cue_view) => {
            let now = cues::read(c, &baseline.id)?;
            let view = now.view();
            if view == *cue_view
                || !at_baseline(&Edit::Cues(view.clone()), now.identity(), baseline)
            {
                return Ok(guard(
                    Edit::Cues(view),
                    now.identity().to_owned(),
                    Mutation::ObserveOnly,
                ));
            }
            if analysis.is_empty() && registered_files(c, &baseline.id)? {
                bail!("Native file registrations exist without a recorded analysis path");
            }
            let patch = cues::plan(&now, cue_view)?;
            let mut registered_embedding = registration::check_inventory(c, &baseline.id, &share, &analysis)?;
            if registered_embedding.is_some() && owners.shared_analysis.contains(
                &analysis.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
            ) {
                registered_embedding = None;
            }
            cues::check_sidecars(&share, &analysis, &baseline.id, registered_embedding, &patch)?;
            Current {
                value: Edit::Cues(view),
                proof: now.identity().to_owned(),
                guard: now.guard(),
                write: Mutation::Cues(now, patch),
                companion: None,
            }
        }
        Edit::Grid(new_grid) => {
            ensure!(
                !analysis.is_empty(),
                "Track has no registered native beat grid"
            );
            ensure!(
                !owners.shared_analysis.contains(
                    &analysis
                        .replace('\\', "/")
                        .trim_start_matches('/')
                        .to_ascii_lowercase()
                ),
                "Analysis path is shared by multiple native tracks"
            );
            let snap = grid::inspect(&share, &analysis)?;
            let proof = grid_proof(&analysis, &snap.dat_sha256());
            if crate::app::grids_match(&snap.grid, new_grid)
                || !at_baseline(&Edit::Grid(snap.grid.clone()), &proof, baseline)
            {
                return Ok(guard(Edit::Grid(snap.grid), proof, Mutation::ObserveOnly));
            }
            let registered_embedding =
                registration::check_inventory(c, &baseline.id, &share, &analysis)?;
            let revision =
                next_grid_revision(r.get::<_, Option<String>>("AnalysisUpdated")?.as_deref())?;
            let mut companion = None;
            if snap.grid.anchors.len() == 1
                && new_grid.anchors.len() == 1
                && (snap.grid.first_bpm() - new_grid.first_bpm()).abs() > 1e-9
            {
                let has_cues: bool = c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM djmdCue WHERE ContentID=? AND rb_local_deleted=0)",
                    [&baseline.id],
                    |r| r.get(0),
                )?;
                ensure!(
                    !has_cues,
                    "Retiming a grid with existing native cue records needs reanalysis"
                );
                let (summary, _) =
                    native_number(&r.get("BPM")?).context("Unsupported native track BPM")?;
                if (snap.grid.first_bpm() * 100.0).round() as i64 == summary {
                    companion = Some(Field::Bpm);
                }
            }
            let files = grid::plan(
                &share,
                &analysis,
                &snap,
                new_grid,
                &baseline.id,
                registered_embedding,
            )?;
            ensure!(
                !files.is_empty(),
                "Grid edit is below native timing precision"
            );
            registration::check(c, &baseline.id, &source.folder, &files)?;
            Current {
                value: Edit::Grid(snap.grid.clone()),
                proof: grid_proof(&analysis, &snap.dat_sha256()),
                guard: serde_json::to_string(&snap)?,
                write: Mutation::Grid(files, revision),
                companion,
            }
        }
    })
}

pub(crate) fn preview(local_db_path: &Path, source_dir: &Path) -> Result<Plan> {
    location::verify(source_dir)?;
    ensure!(
        !storage::pending(source_dir),
        "A previous native write must be recovered before reviewing edits"
    );
    let local_library = local_db_path.canonicalize()?;
    let local = local_db(&local_library, false)?;
    let local_read = local.conn.unchecked_transaction()?;
    let staging = tempfile::tempdir()?;
    let snap = super::snapshot(source_dir, staging.path())?;
    let remote = super::open_snapshot(&snap.path().join("master.db"))?;
    let source = Source::read(&remote, source_dir)?;
    read_counter(&remote)?;
    let mut plan = Plan {
        rows: vec![],
        tracked: 0,
        source: source.clone(),
        local_library,
    };
    let mut st = local.conn.prepare(READ_BASELINES)?;
    let mut baselines = st.query([serde_json::to_string(&source)?])?;
    let mut track = None::<Track>;
    let mut owners = None;
    while let Some(record) = baselines.next()? {
        let base = read_baseline(record)?;
        if track.as_ref().map(|t| t.id) != Some(base.local_id) {
            track = local
                .track_at_path(&base.path)?
                .filter(|t| t.id == base.local_id);
            if track.is_some() {
                plan.tracked += 1;
            }
        }
        let Some(t) = track.as_ref() else { continue };
        let field = base.field.field();
        let (desired, problem) = match local_edit(&local, t, field) {
            Ok(edit) => (edit, None),
            Err(error) => (
                base.field.clone(),
                Some(format!("Unsupported local edit: {error:#}")),
            ),
        };
        if problem.is_none() && desired.same_value(&base.field) {
            continue;
        }
        let mut row = Row {
            track: if t.title.is_empty() {
                t.path.clone()
            } else {
                t.title.clone()
            },
            field,
            local_id: base.local_id,
            companion: None,
            before: base.field.display(),
            current: "Unavailable".into(),
            proposed: desired.display(),
            reason: None,
            guard: String::new(),
            baseline: base,
            desired,
        };
        if let Some(problem) = problem {
            row.reason = Some(problem);
            row.proposed = "Unsupported local value".into();
            plan.rows.push(row);
            continue;
        }
        if owners.is_none() {
            owners = Some(Owners::read(&remote)?);
        }
        match checked_current(
            &remote,
            owners.as_ref().unwrap(),
            &source,
            &row.baseline,
            &row.desired,
        ) {
            Ok(now) => {
                row.current = now.value.display();
                row.guard = now.guard;
                row.companion = now.companion;
                if now.value.same_value(&row.desired) {
                    row.reason = Some("Already applied".into());
                } else if !at_baseline(&now.value, &now.proof, &row.baseline) {
                    row.reason = Some("Conflict: the native field changed since import".into());
                }
            }
            Err(err) => row.reason = Some(format!("{err:#}")),
        }
        plan.rows.push(row);
    }
    let companions: HashMap<_, _> = plan
        .rows
        .iter()
        .filter(|r| r.reason.is_none())
        .map(|r| ((r.local_id, r.field), (&r.desired, r.companion)))
        .collect();
    let missing: Vec<bool> = plan
        .rows
        .iter()
        .map(|r| {
            r.companion.is_some_and(|partner| {
                let Some((edit, reverse)) = companions.get(&(r.local_id, partner)) else {
                    return true;
                };
                *reverse != Some(r.field) || !matching_tempo(&r.desired, edit)
            })
        })
        .collect();
    for (r, missing) in plan.rows.iter_mut().zip(missing) {
        if missing && r.reason.is_none() {
            r.reason=Some("The linked native BPM and beat grid require matching, eligible tempo edits together".into());
        }
    }
    local_read.commit()?;
    Ok(plan)
}

fn at_baseline(value: &Edit, proof: &str, base: &Baseline) -> bool {
    proof == base.proof && (matches!(value, Edit::Grid(_)) || *value == base.field)
}

fn matching_tempo(a: &Edit, b: &Edit) -> bool {
    match (a, b) {
        (Edit::Bpm(n), Edit::Grid(g)) | (Edit::Grid(g), Edit::Bpm(n)) => {
            g.anchors.len() == 1 && (g.first_bpm() * 100.0).round() as i64 == *n
        }
        _ => false,
    }
}

pub(crate) fn apply(
    local_db_path: &Path,
    source_dir: &Path,
    plan: &Plan,
    selected: &[usize],
) -> Result<Receipt> {
    apply_with_backup(local_db_path, source_dir, plan, selected, true)
}

pub(crate) fn apply_with_backup(
    local_db_path: &Path,
    source_dir: &Path,
    plan: &Plan,
    selected: &[usize],
    retain_backup: bool,
) -> Result<Receipt> {
    location::verify(source_dir)?;
    ensure!(!selected.is_empty(), "Select at least one edit");
    ensure!(
        plan.local_library == local_db_path.canonicalize()?
            && plan.source.folder == source_dir.canonicalize()?,
        "Library selection changed; review the new paths"
    );
    let mut unique = HashSet::new();
    let mut rows = Vec::with_capacity(selected.len());
    for &index in selected {
        let row = plan.rows.get(index).context("Unrecognized review row")?;
        ensure!(
            row.reason.is_none() && unique.insert((row.local_id, row.field)),
            "Ineligible or duplicate selection"
        );
        rows.push(row);
    }
    for row in &rows {
        if let Some(partner) = row.companion {
            ensure!(
                unique.contains(&(row.local_id, partner)),
                "Select the linked BPM and beat grid together"
            );
        }
    }
    let db = local_db(local_db_path, true)?;
    let recovery = if retain_backup {
        storage::RecoveryCopy::Keep
    } else if rows.iter().any(|r| r.field == Field::Grid) {
        storage::RecoveryCopy::TemporaryForGrids
    } else {
        storage::RecoveryCopy::DatabaseTransaction
    };
    let (proofs, recovery_files) = storage::apply_with_policy(source_dir, &backup_root(local_db_path)?, recovery, |remote| {
        location::verify(source_dir)?;
        ensure!(
            Source::read(remote, source_dir)? == plan.source,
            "Target database changed since review"
        );
        let mut usn = read_counter(remote)?;
        let first_usn = usn;
        // Validate the entire selection before updating anything. Two approved
        // fields on the same track can legitimately change its row revision.
        let owners = Owners::read(remote)?;
        let mut patch_bytes = 0usize;
        let mut validated = Vec::with_capacity(rows.len());
        for row in &rows {
            let now = checked_current(remote, &owners, &plan.source, &row.baseline, &row.desired)?;
            ensure!(
                at_baseline(&now.value, &now.proof, &row.baseline)
                    && now.guard == row.guard
                    && now.companion == row.companion,
                "Native {} changed since review for {}",
                row.field.label(),
                row.track
            );
            if let Mutation::Grid(files, _) = &now.write {
                for file in files {
                    patch_bytes = patch_bytes.saturating_add(file.contents.len());
                }
                ensure!(
                    patch_bytes <= 256 * 1024 * 1024,
                    "Selected grid edits exceed 256 MiB; apply fewer tracks at a time"
                );
            }
            validated.push(now.write);
        }
        // The native patches and any required recovery copy have been prepared.
        // Hold the local writer slot only for final validation and commit. Individual
        // lookups use (track_id,field), even in a collection with millions of records.
        db.conn.execute_batch("BEGIN IMMEDIATE")?;
        let source_json = serde_json::to_string(&plan.source)?;
        let mut check=db.conn.prepare_cached("SELECT EXISTS(SELECT 1 FROM native_edit_baseline WHERE
            track_id=?1 AND field=?2 AND source=?3 AND native_id=?4 AND uuid=?5 AND path=?6 AND value=?7 AND proof=?8)")?;
        for row in &rows {
            let base = &row.baseline;
            let t = db
                .track_at_path(&base.path)?
                .context("Local track was removed since review")?;
            let same: bool = check.query_row(
                params![
                    row.local_id,
                    row.field.label(),
                    source_json,
                    base.id,
                    base.uuid,
                    base.path,
                    base.stored_value,
                    base.proof
                ],
                |r| r.get(0),
            )?;
            ensure!(
                same && t.id == row.local_id && local_edit(&db, &t, row.field)? == row.desired,
                "Local edit or import baseline changed; review again"
            );
        }
        let stamp = chrono::Utc::now()
            .format("%Y-%m-%d %H:%M:%S%.3f +00:00")
            .to_string();
        let mut files = vec![];
        let mut proofs = vec![];
        for (row, mutation) in rows.iter().zip(validated) {
            let base = &row.baseline;
            let proof = match mutation {
                Mutation::Cues(current, patch) => {
                    cues::apply(remote, &base.id, &current, &patch, &mut usn, &stamp)?;
                    cues::read(remote, &base.id)?.identity().to_owned()
                }
                Mutation::Column(col, value) => {
                    let proof = super::text(value.clone());
                    update_column(remote, base, col, value, &mut usn, &stamp)?;
                    proof
                }
                Mutation::ObserveOnly => bail!("Field became ineligible after review"),
                Mutation::Grid(changes, revision) => {
                    let dat = changes
                        .iter()
                        .find(|f| f.relative_path.extension().is_some_and(|e| e == "DAT"))
                        .context("Grid edit produced no DAT correction")?;
                    let analysis: String = remote.query_row(
                        "SELECT AnalysisDataPath FROM djmdContent WHERE ID=?",
                        [&base.id],
                        |r| r.get(0),
                    )?;
                    let proof = grid_proof(&analysis, &Sha256::digest(&dat.contents).into());
                    registration::update(remote, &base.id, source_dir, &changes, &mut usn, &stamp)?;
                    files.extend(changes);
                    update_column(
                        remote,
                        base,
                        "AnalysisUpdated",
                        Value::Text(revision),
                        &mut usn,
                        &stamp,
                    )?;
                    proof
                }
            };
            proofs.push(proof);
        }
        let changed=remote.execute("UPDATE agentRegistry SET int_1=?1,updated_at=?2 WHERE registry_id='localUpdateCount' AND int_1=?3",params![usn,stamp,first_usn])?;
        ensure!(changed == 1, "Native registry changed during the update");
        Ok((proofs, files))
    })?;
    // Native commit precedes acknowledging new baselines. An interruption here
    // leaves old baselines, causing a conservative conflict on a subsequent edit.
    let acknowledge = || -> Result<()> {
        for (r, proof) in rows.iter().zip(proofs) {
            let updated = db.conn.execute(
                "UPDATE native_edit_baseline SET value=?1,proof=?2 WHERE track_id=?3 AND field=?4",
                params![
                    serde_json::to_string(&r.desired)?,
                    proof,
                    r.local_id,
                    r.field.label()
                ],
            )?;
            ensure!(updated == 1, "Missing local baseline");
        }
        db.conn.execute_batch("COMMIT")?;
        Ok(())
    };
    acknowledge().with_context(|| {
        let recovery = recovery_files.backup.as_ref().or(recovery_files.remaining_staging.as_ref()).map(|p| format!(" Recovery files at {}.", p.display())).unwrap_or_default();
        format!("Native changes committed.{recovery} Import again to refresh local baselines.")
    })?;
    Ok(Receipt {
        applied: rows.len(),
        backup: recovery_files.backup,
        remaining_staging: recovery_files.remaining_staging,
    })
}

fn update_column(
    c: &Connection,
    base: &Baseline,
    column: &str,
    value: Value,
    usn: &mut i64,
    stamp: &str,
) -> Result<()> {
    ensure!(
        matches!(column, "BPM" | "Rating" | "KeyID" | "AnalysisUpdated"),
        "Unsupported native column"
    );
    *usn = usn
        .checked_add(1)
        .context("Native update counter overflow")?;
    let sql = format!(
        "UPDATE djmdContent SET {column}=?1,rb_local_usn=?2,rb_local_synced=0,updated_at=?3
        WHERE ID=?4 AND UUID=?5 AND rb_local_deleted=0"
    );
    ensure!(
        c.execute(&sql, params![value, *usn, stamp, base.id, base.uuid])? == 1,
        "Native track was not updated"
    );
    Ok(())
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use crate::importer::XmlTrack;

    fn imported() -> RbTrack {
        let track = Track {
            path: "/music/edit.wav".into(),
            rating: 2,
            bpm: 120.0,
            key: parse_key("Am"),
            ..Default::default()
        };
        RbTrack {
            rb_id: "77".into(),
            fingerprint: "one".into(),
            xt: XmlTrack {
                xml_id: "77".into(),
                track,
                grid: None,
                cues: Vec::new(),
                extra: Default::default(),
            },
            tags: vec![],
            anlz_base: None,
            native: Some(ImportTrack {
                uuid: "73ee646d-b7f2-4528-95ab-9d8e51301681".into(),
                rating: Some((2, "2".into())),
                bpm: Some((12000, "12000".into())),
                key_id: Some("7".into()),
                cue_ids: Some(empty_cue_identity()),
                grid_proof: Some("absent".into()),
            }),
        }
    }

    #[test]
    fn older_divergent_fields_require_an_agreed_import_before_enrollment() -> Result<()> {
        let db = Db::open(Path::new(":memory:"))?;
        let rt = imported();
        let mut t = rt.xt.track.clone();
        t.rating = 4;
        let id = db.insert_track(&t)?;
        let source = Source {
            folder: "/fixture/source".into(),
            salt: "synthetic".into(),
        };
        record_import(&db, id, &rt, &serde_json::to_string(&source)?, false)?;
        let initial = baselines(&db, &source)?;
        assert!(!initial.iter().any(|b| b.field.field() == Field::Rating));
        assert!(initial.iter().any(|b| b.field == Edit::Bpm(12000)));
        assert!(initial.iter().any(|b| b.field.field() == Field::Cues));
        t.id = id;
        t.rating = 2;
        db.update_track(&t)?;
        record_import(&db, id, &rt, &serde_json::to_string(&source)?, false)?;
        t.rating = 5;
        db.update_track(&t)?;
        record_import(&db, id, &rt, &serde_json::to_string(&source)?, false)?;
        let agreed = baselines(&db, &source)?;
        assert!(agreed.iter().any(|b| b.field == Edit::Rating(2)));
        assert!(!agreed.iter().any(|b| b.field == Edit::Rating(5)));
        db.delete_track(id)?;
        assert!(baselines(&db, &source)?.is_empty());
        Ok(())
    }

    #[test]
    fn duplicate_native_paths_and_uuids_are_collected_across_the_snapshot() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(
            "CREATE TABLE djmdContent(UUID,FolderPath,AnalysisDataPath,rb_local_deleted);
            INSERT INTO djmdContent VALUES
            ('a','/music/one.wav','/PIONEER/USBANLZ/A/ANLZ0000.DAT',0),
            ('A','/MUSIC/ONE.wav','PIONEER/USBANLZ/a/ANLZ0000.DAT',0),
            ('b','/music/other.wav','',1);",
        )?;
        let owners = Owners::read(&db)?;
        assert!(owners.duplicate_ids.contains("a"));
        assert!(owners.duplicate_paths.contains("/music/one.wav"));
        assert!(owners
            .shared_analysis
            .contains("pioneer/usbanlz/a/anlz0000.dat"));
        assert!(!owners.duplicate_ids.contains("b"));
        Ok(())
    }

    #[test]
    fn scalar_bpm_rechecks_native_registrations_with_no_analysis_path() -> Result<()> {
        let fixture = integration_tests::Libraries::new()?;
        let local = Db::open(&fixture.local)?;
        let mut track = local.track_at_path(&fixture.audio)?.unwrap();
        track.bpm = 125.;
        local.update_track(&track)?;
        drop(local);
        let p = preview(&fixture.local, &fixture.source)?;
        let row = p
            .rows
            .iter()
            .position(|r| r.field == Field::Bpm && r.reason.is_none())
            .unwrap();
        let native = Connection::open(fixture.source.join("master.db"))?;
        native.pragma_update(None, "key", super::super::MASTER_DB_KEY)?;
        native.execute("INSERT INTO contentFile(ID,ContentID,Path,rb_local_deleted) VALUES ('new','10','/PIONEER/USBANLZ/n/ANLZ0000.3EX',0)", [])?;
        drop(native);
        let fresh = preview(&fixture.local, &fixture.source)?;
        assert!(fresh.rows.iter().any(|r| r.field == Field::Bpm
            && r.reason
                .as_deref()
                .is_some_and(|reason| reason
                    .contains("registrations exist without a recorded analysis path"))));
        assert!(apply(&fixture.local, &fixture.source, &p, &[row]).is_err());
        let native = Connection::open(fixture.source.join("master.db"))?;
        native.pragma_update(None, "key", super::super::MASTER_DB_KEY)?;
        assert_eq!(
            native.query_row("SELECT BPM FROM djmdContent WHERE ID='10'", [], |r| r
                .get::<_, i64>(0))?,
            12000
        );
        Ok(())
    }

    #[test]
    fn key_names_and_native_revision_encoding_are_checked() {
        assert_eq!(recognized_key("F# minor"), Some(parse_key("F#m")));
        assert_eq!(recognized_key("Arbitrary"), None);
        assert_eq!(recognized_key("F##"), None);
        assert_eq!(recognized_key("1m"), None);
        assert_eq!(next_grid_revision(Some("9")).unwrap(), ":");
        assert!(next_grid_revision(Some("10")).is_err());
        assert_ne!(
            native_number(&Value::Null),
            native_number(&Value::Integer(0))
        );
    }
}
