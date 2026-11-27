//! The sampler's library side: sampler lists, sample packs and the OSC sampler pack.
//!
//! rekordbox keeps sampler lists in `djmdSampler` (ID, Seq, Name, Attribute, ParentID) and
//! their members in `djmdSongSampler` (ID, SamplerID, ContentID, TrackNo); recordbox keeps the
//! same shape in its own database (`sampler_lists`, `sampler_list_tracks`), plus one row per
//! sample (`sampler_samples`: a sample need not be a track of the collection).
//!
//! Packs (`DatabaseMediator::importSamplerPack` 0x101a114e4): a zip with `sampler_pack.xml`,
//! unpacked into `<folder>/<zip name>` (`name(<n>)` when that exists); every TRACKS/TRACK is a
//! sample (`mode` 1 = loop, with `bpm`), every LISTS/LIST with `attr` 3 a folder under the
//! Sampler root and each LIST with `attr` 2 inside it a sampler list whose `TRACK key`s are
//! its members. The OSC pack (`BrowseBasicView::importOSCSamplerPack` 0x100317234) is the
//! installed rekordbox's own `OSC_SAMPLER.zip`, read from there and unpacked once into
//! recordbox's data folder; nothing of it is in this repository.

use crate::app::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `Attribute` of a sampler list row, as in the pack format.
pub const ATTR_LIST: i64 = 2;
pub const ATTR_FOLDER: i64 = 3;

/// The two fixed rows of the Sampler tree (`View::Sampler` ids).
pub const ALL_SAMPLES: i64 = -1;
pub const CAPTURE: i64 = -2;

/// Sample modes (`db::SamplerPlayMode`; the Track Type menu's order).
pub const MODE_ONESHOT: i64 = 0;
pub const MODE_LOOP: i64 = 1;
pub const MODE_NORMAL: i64 = 2;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct LibPrefs {
    /// rekordbox's `OSCSamplerListID`: the list the OSC pack made; -1 = not imported yet.
    pub osc_list_id: i64,
}

impl Default for LibPrefs {
    fn default() -> Self {
        LibPrefs { osc_list_id: -1 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SamplerList {
    pub id: i64,
    pub seq: i64,
    pub name: String,
    pub attribute: i64,
    pub parent: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub mode: i64,
    /// BPM times 100 (0 = not known), as rekordbox stores it for loop samples.
    pub bpm100: i64,
    pub duration: f64,
    /// The collection track this sample was made from (0 = none).
    pub track_id: i64,
    /// The row in the list being shown (`sampler_list_tracks.id`), 0 outside a list.
    pub member: i64,
}

/// What the tree and the list keep while the app runs.
#[derive(Default)]
pub struct LibState {
    pub ready: bool,
    pub lists: Vec<SamplerList>,
    pub counts: std::collections::HashMap<i64, usize>,
    pub open: std::collections::HashSet<i64>,
    pub root_closed: bool,
    pub rename: Option<(i64, String)>,
    pub ask_delete: Option<i64>,
    /// "There are duplicates being added to the sampler list.": (list, samples) waiting.
    pub ask_dup: Option<(i64, Vec<i64>)>,
    pub selected: std::collections::BTreeSet<usize>,
    pub rows: Vec<Sample>,
    pub rows_for: Option<i64>,
    pub rows_dirty: bool,
    /// The OSC pack being unpacked in the background.
    pub osc_job: Option<crossbeam_channel::Receiver<Result<(PathBuf, Pack), String>>>,
    pub osc_tried: bool,
    /// Samples whose length and loop BPM are being read in the background (pack samples come
    /// without them): (sample id, length s, BPM).
    pub measure_job: Option<crossbeam_channel::Receiver<(i64, f64, f64)>>,
    pub measured_all: bool,
    /// Where the sampler slots were drawn (the scripted run's drop targets).
    pub slots_ui: Vec<(usize, eframe::egui::Rect)>,
    /// Sampler list rows being dragged (to a sampler slot, a deck or a playlist).
    pub drag: Option<Vec<Sample>>,
    /// A dragged sample that is not in the collection: imported first for a deck or playlist.
    pub drag_import: Option<String>,
    /// Where the tree's rows were drawn (for the scripted run).
    pub ui: Vec<(String, eframe::egui::Rect)>,
}

// ---------------------------------------------------------------- track types

/// The track menu's Track Type (`cue_tools.rs`: 0 Normal, 1 Oneshot, 2 Loop) as a sample mode.
pub fn mode_of_type(t: u8) -> i64 {
    match t {
        1 => MODE_ONESHOT,
        2 => MODE_LOOP,
        _ => MODE_NORMAL,
    }
}

pub fn type_of_mode(m: i64) -> u8 {
    match m {
        MODE_ONESHOT => 1,
        MODE_LOOP => 2,
        _ => 0,
    }
}

/// The mode of the sample row made from a collection track, if there is one.
pub fn track_mode(conn: &Connection, track_id: i64) -> Option<i64> {
    conn.query_row("SELECT mode FROM sampler_samples WHERE track_id=?1 ORDER BY id LIMIT 1", [track_id], |r| r.get(0)).ok()
}

/// Set the mode of the sample rows made from these collection tracks.
pub fn set_track_mode(conn: &Connection, track_ids: &[i64], mode: i64) {
    for t in track_ids {
        let _ = conn.execute("UPDATE sampler_samples SET mode=?1 WHERE track_id=?2", params![mode, t]);
    }
}

/// A loop's BPM: the analysed tempo when the loop is long enough to have one, else the BPM
/// that makes its length a whole number of beats (1, 2, 4 .. 64) nearest 120 within 70..180.
pub fn loop_bpm(secs: f64, detected: f64) -> f64 {
    if detected > 0.0 {
        return detected;
    }
    if secs <= 0.0 {
        return 0.0;
    }
    let mut best = 0.0f64;
    for n in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0] {
        let b = 60.0 * n / secs;
        if (70.0..=180.0).contains(&b) && (best == 0.0 || (b - 120.0).abs() < (best - 120.0).abs()) {
            best = b;
        }
    }
    (best * 100.0).round() / 100.0
}

// ---------------------------------------------------------------- the database

pub fn ensure(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sampler_lists(id INTEGER PRIMARY KEY, seq INTEGER NOT NULL, name TEXT NOT NULL,
            attribute INTEGER NOT NULL, parent_id INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS sampler_samples(id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, name TEXT NOT NULL,
            mode INTEGER NOT NULL DEFAULT 0, bpm100 INTEGER NOT NULL DEFAULT 0, duration REAL NOT NULL DEFAULT 0,
            track_id INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS sampler_list_tracks(id INTEGER PRIMARY KEY, sampler_id INTEGER NOT NULL,
            content_id INTEGER NOT NULL, track_no INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS slt_list ON sampler_list_tracks(sampler_id, track_no);",
    )
}

pub fn lists(conn: &Connection) -> rusqlite::Result<Vec<SamplerList>> {
    let mut st = conn.prepare("SELECT id, seq, name, attribute, parent_id FROM sampler_lists ORDER BY parent_id, seq, id")?;
    let rows = st.query_map([], |r| Ok(SamplerList { id: r.get(0)?, seq: r.get(1)?, name: r.get(2)?, attribute: r.get(3)?, parent: r.get(4)? }))?;
    rows.collect()
}

/// A name no sibling has: `name`, then `name(1)`, `name(2)`... (the pack import's rule).
pub fn free_name(conn: &Connection, parent: i64, name: &str) -> String {
    let taken = |n: &str| conn.query_row("SELECT count(*) FROM sampler_lists WHERE parent_id=?1 AND name=?2", params![parent, n], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if !taken(name) {
        return name.to_string();
    }
    (1..).map(|k| format!("{name}({k})")).find(|n| !taken(n)).unwrap()
}

pub fn create_list(conn: &Connection, parent: i64, name: &str, attribute: i64) -> rusqlite::Result<i64> {
    let seq: i64 = conn.query_row("SELECT coalesce(max(seq), -1) + 1 FROM sampler_lists WHERE parent_id=?1", [parent], |r| r.get(0))?;
    conn.execute("INSERT INTO sampler_lists(seq, name, attribute, parent_id) VALUES(?1, ?2, ?3, ?4)", params![seq, name, attribute, parent])?;
    Ok(conn.last_insert_rowid())
}

pub fn rename_list(conn: &Connection, id: i64, name: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE sampler_lists SET name=?1 WHERE id=?2", params![name, id]).map(|_| ())
}

/// Delete a list or a folder with everything in it. The samples themselves stay (they are
/// still under All Samples), as tracks stay in the collection when a playlist goes.
pub fn delete_list(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    let kids: Vec<i64> = {
        let mut st = conn.prepare("SELECT id FROM sampler_lists WHERE parent_id=?1")?;
        let v = st.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        v
    };
    for k in kids {
        delete_list(conn, k)?;
    }
    conn.execute("DELETE FROM sampler_list_tracks WHERE sampler_id=?1", [id])?;
    conn.execute("DELETE FROM sampler_lists WHERE id=?1", [id])?;
    Ok(())
}

/// The sample row for a file, made when it is not there yet.
pub fn sample_for(conn: &Connection, path: &str, name: &str, mode: i64, bpm100: i64, duration: f64, track_id: i64) -> rusqlite::Result<i64> {
    if let Ok(id) = conn.query_row("SELECT id FROM sampler_samples WHERE path=?1", [path], |r| r.get::<_, i64>(0)) {
        return Ok(id);
    }
    conn.execute("INSERT INTO sampler_samples(path, name, mode, bpm100, duration, track_id) VALUES(?1, ?2, ?3, ?4, ?5, ?6)", params![path, name, mode, bpm100, duration, track_id])?;
    Ok(conn.last_insert_rowid())
}

/// Which of `samples` the list already has.
pub fn duplicates(conn: &Connection, list: i64, samples: &[i64]) -> Vec<i64> {
    samples.iter().copied().filter(|s| conn.query_row("SELECT count(*) FROM sampler_list_tracks WHERE sampler_id=?1 AND content_id=?2", params![list, s], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0).collect()
}

/// Append samples to a list (`addTrackToSamplerListInternal`), numbering on from its last row.
pub fn add_to_list(conn: &Connection, list: i64, samples: &[i64]) -> rusqlite::Result<usize> {
    let mut no: i64 = conn.query_row("SELECT coalesce(max(track_no), 0) FROM sampler_list_tracks WHERE sampler_id=?1", [list], |r| r.get(0))?;
    for s in samples {
        no += 1;
        conn.execute("INSERT INTO sampler_list_tracks(sampler_id, content_id, track_no) VALUES(?1, ?2, ?3)", params![list, s, no])?;
    }
    Ok(samples.len())
}

/// Take rows out of a list and close the gaps in its numbering.
pub fn remove_members(conn: &Connection, list: i64, members: &[i64]) -> rusqlite::Result<()> {
    for m in members {
        conn.execute("DELETE FROM sampler_list_tracks WHERE id=?1 AND sampler_id=?2", params![m, list])?;
    }
    let ids: Vec<i64> = {
        let mut st = conn.prepare("SELECT id FROM sampler_list_tracks WHERE sampler_id=?1 ORDER BY track_no, id")?;
        let v = st.query_map([list], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        v
    };
    for (k, id) in ids.iter().enumerate() {
        conn.execute("UPDATE sampler_list_tracks SET track_no=?1 WHERE id=?2", params![k as i64 + 1, id])?;
    }
    Ok(())
}

fn sample_row(r: &rusqlite::Row) -> rusqlite::Result<Sample> {
    Ok(Sample { id: r.get(0)?, path: r.get(1)?, name: r.get(2)?, mode: r.get(3)?, bpm100: r.get(4)?, duration: r.get(5)?, track_id: r.get(6)?, member: r.get(7)? })
}

/// A list's samples in its order.
pub fn list_samples(conn: &Connection, list: i64) -> rusqlite::Result<Vec<Sample>> {
    let mut st = conn.prepare(
        "SELECT s.id, s.path, s.name, s.mode, s.bpm100, s.duration, s.track_id, m.id FROM sampler_list_tracks m
         JOIN sampler_samples s ON s.id = m.content_id WHERE m.sampler_id=?1 ORDER BY m.track_no, m.id",
    )?;
    let v = st.query_map([list], sample_row)?.collect();
    v
}

pub fn all_samples(conn: &Connection) -> rusqlite::Result<Vec<Sample>> {
    let mut st = conn.prepare("SELECT id, path, name, mode, bpm100, duration, track_id, 0 FROM sampler_samples ORDER BY name COLLATE NOCASE, id")?;
    let v = st.query_map([], sample_row)?.collect();
    v
}

pub fn set_mode(conn: &Connection, samples: &[i64], mode: i64) -> rusqlite::Result<()> {
    for s in samples {
        conn.execute("UPDATE sampler_samples SET mode=?1 WHERE id=?2", params![mode, s])?;
    }
    Ok(())
}

pub fn counts(conn: &Connection) -> std::collections::HashMap<i64, usize> {
    let mut out = std::collections::HashMap::new();
    if let Ok(mut st) = conn.prepare("SELECT sampler_id, count(*) FROM sampler_list_tracks GROUP BY sampler_id") {
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))) {
            for (id, n) in rows.flatten() {
                out.insert(id, n as usize);
            }
        }
    }
    if let Ok(n) = conn.query_row("SELECT count(*) FROM sampler_samples", [], |r| r.get::<_, i64>(0)) {
        out.insert(ALL_SAMPLES, n as usize);
    }
    out
}

// ---------------------------------------------------------------- packs

#[derive(Clone, Debug, PartialEq)]
pub struct PackTrack {
    pub id: i64,
    pub location: String,
    pub mode: i64,
    pub bpm: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PackList {
    pub name: String,
    pub attr: i64,
    pub seq: i64,
    /// Index of the LIST this one is inside.
    pub parent: Option<usize>,
    pub keys: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pack {
    pub tracks: Vec<PackTrack>,
    pub lists: Vec<PackList>,
}

/// The value of attribute `name` in the text of one element (`NAME a="1" b="2"`).
pub fn attr(el: &str, name: &str) -> Option<String> {
    let mut rest = el;
    while let Some(k) = rest.find(name) {
        let before_ok = k == 0 || rest[..k].ends_with(|c: char| c.is_whitespace());
        let after = rest[k + name.len()..].trim_start();
        if before_ok && after.starts_with('=') {
            let v = after[1..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let end = v[1..].find(q)?;
                return Some(v[1..1 + end].replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'"));
            }
        }
        rest = &rest[k + name.len()..];
    }
    None
}

/// Read `sampler_pack.xml`. (rekordbox's own OSC pack closes its root element under another
/// name than it opens it with, so this reads element by element and does not ask for a
/// well-formed document.)
pub fn parse_pack_xml(xml: &str) -> Pack {
    let mut pack = Pack::default();
    let mut stack: Vec<usize> = vec![];
    let mut in_tracks = false;
    for el in xml.split('<').skip(1) {
        let Some(end) = el.find('>') else { continue };
        let body = el[..end].trim();
        let self_closed = body.ends_with('/');
        let body = body.trim_end_matches('/').trim();
        let name = body.split(|c: char| c.is_whitespace()).next().unwrap_or("");
        match name {
            "TRACKS" => in_tracks = !self_closed,
            "/TRACKS" => in_tracks = false,
            "TRACK" if in_tracks => {
                if let (Some(id), Some(location)) = (attr(body, "id").and_then(|v| v.parse().ok()), attr(body, "location")) {
                    pack.tracks.push(PackTrack { id, location, mode: attr(body, "mode").and_then(|v| v.parse().ok()).unwrap_or(0), bpm: attr(body, "bpm").and_then(|v| v.parse().ok()) });
                }
            }
            "TRACK" => {
                if let (Some(l), Some(key)) = (stack.last(), attr(body, "key").and_then(|v| v.parse().ok())) {
                    pack.lists[*l].keys.push(key);
                }
            }
            "LIST" => {
                pack.lists.push(PackList { name: attr(body, "name").unwrap_or_default(), attr: attr(body, "attr").and_then(|v| v.parse().ok()).unwrap_or(ATTR_LIST), seq: attr(body, "seq").and_then(|v| v.parse().ok()).unwrap_or(0), parent: stack.last().copied(), keys: vec![] });
                if !self_closed {
                    stack.push(pack.lists.len() - 1);
                }
            }
            "/LIST" => {
                stack.pop();
            }
            _ => {}
        }
    }
    pack
}

/// Both ZIP names and XML locations must be portable, relative paths owned by this pack.
fn relative_sample_path(name: &str) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for part in name.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." || part.ends_with([' ', '.'])
            || part.contains([':', '*', '?', '"', '<', '>', '|']) || part.chars().any(char::is_control) {
            return None;
        }
        let stem = part.split('.').next()?.to_ascii_uppercase();
        if matches!(stem.trim_end(), "NUL" | "AUX" | "CON" | "PRN")
            || ["COM", "LPT"].iter().any(|p| stem.strip_prefix(p).is_some_and(|d| matches!(d, "1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"|"¹"|"²"|"³"))) {
            return None;
        }
        result.push(part);
    }
    Some(result)
}

/// Reserve a fresh destination. Two simultaneous imports cannot share a cleanup path.
fn new_pack_dir(parent: &Path, name: &str) -> std::io::Result<PathBuf> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    for n in 0usize.. {
        let path = if n == 0 { parent.join(name) } else { parent.join(format!("{name}({n})")) };
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

pub fn unpack(zip_path: &Path, dest: &Path) -> Result<(PathBuf, Pack), String> {
    use std::io::Read;
    const MAX_XML: u64 = 8 << 20;
    const MAX_SAMPLE: u64 = 512 << 20;
    const MAX_PACK: u64 = 4 << 30;
    let f = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(f).map_err(|e| format!("Cannot read the sample pack: {e}"))?;
    if archive.len() > 16_384 {
        return Err("The sample pack contains too many files.".into());
    }
    archive.validate_header_consistency().map_err(|e| e.to_string())?;
    let mut manifest = archive.by_name("sampler_pack.xml").map_err(|e| format!("Cannot read sampler_pack.xml: {e}"))?;
    if manifest.size() > MAX_XML || !manifest.is_file() || manifest.is_symlink() {
        return Err("Invalid sample pack manifest.".into());
    }
    let mut xml = String::new();
    let size = manifest.size();
    let read = (&mut manifest).take(size + 1).read_to_string(&mut xml).map_err(|e| e.to_string())?;
    if read as u64 != size {
        return Err("The sample pack manifest size is inconsistent.".into());
    }
    drop(manifest);
    let pack = parse_pack_xml(&xml);
    let samples: std::collections::HashSet<_> = pack.tracks.iter()
        .filter_map(|t| relative_sample_path(&t.location))
        .filter(|p| crate::audio::has_audio_extension(p)).collect();
    let stem = zip_path.file_stem().and_then(|s| s.to_str())
        .filter(|s| relative_sample_path(s).is_some_and(|p| p.components().count() == 1)).unwrap_or("pack");
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let dir = new_pack_dir(dest, stem).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let mut total = 0u64;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            let Some(path) = relative_sample_path(&entry.name().map_err(|e| e.to_string())?) else { continue };
            if !samples.contains(&path) || !entry.is_file() || entry.is_symlink() {
                continue;
            }
            let size = entry.size();
            if size > MAX_SAMPLE || size > MAX_PACK - total {
                return Err("The sample pack exceeds the audio import size limit.".into());
            }
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let mut out = std::fs::File::create_new(&path).map_err(|e| e.to_string())?;
            let read = std::io::copy(&mut (&mut entry).take(size + 1), &mut out).map_err(|e| e.to_string())?;
            if read != size {
                return Err("The sample size is inconsistent.".into());
            }
            total += size;
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok((dir, pack))
}

/// Put an unpacked pack into the database. Returns the id of its first top-level row (the
/// folder the pack made), or 0.
pub fn register_pack(conn: &Connection, dir: &Path, pack: &Pack) -> rusqlite::Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let conn = &tx;
    ensure(conn)?;
    let mut by_key = std::collections::HashMap::new();
    for t in &pack.tracks {
        let Some(relative) = relative_sample_path(&t.location) else { continue };
        let path = dir.join(relative);
        if !crate::audio::has_audio_extension(&path) || !path.is_file() {
            continue;
        }
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        // A loop sample's BPM is kept as int(bpm * 100).
        let bpm100 = if t.mode == MODE_LOOP { t.bpm.map(|b| (b * 100.0) as i64).unwrap_or(0) } else { 0 };
        let id = sample_for(conn, &path.to_string_lossy(), &name, if t.mode == MODE_LOOP { MODE_LOOP } else { MODE_ONESHOT }, bpm100, 0.0, 0)?;
        by_key.insert(t.id, id);
    }
    let mut ids: Vec<i64> = vec![0; pack.lists.len()];
    let mut first = 0;
    for (k, l) in pack.lists.iter().enumerate() {
        let parent = l.parent.map(|p| ids[p]).unwrap_or(0);
        // Folders sit under the Sampler root, lists inside a folder (or the root).
        let attribute = if l.attr == ATTR_FOLDER { ATTR_FOLDER } else { ATTR_LIST };
        let name = free_name(conn, parent, &l.name);
        let id = create_list(conn, parent, &name, attribute)?;
        ids[k] = id;
        if first == 0 && parent == 0 {
            first = id;
        }
        let members: Vec<i64> = l.keys.iter().filter_map(|k| by_key.get(k).copied()).collect();
        add_to_list(conn, id, &members)?;
    }
    tx.commit()?;
    Ok(first)
}

/// The installed rekordbox's OSC pack, and where recordbox unpacks packs.
pub fn osc_zip() -> PathBuf {
    PathBuf::from("/Applications/rekordbox 7/rekordbox.app/Contents/Resources/OSC_SAMPLER.zip")
}

pub fn packs_dir() -> PathBuf {
    crate::db::data_dir().join("Sampler")
}

/// The rows of the Capture list: the audio files of the capture folder, newest first.
pub fn capture_files(dir: &Path) -> Vec<Sample> {
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| crate::audio::has_audio_extension(p)).map(|p| (p.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p)).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().enumerate().map(|(k, (_, p))| Sample { id: -(k as i64) - 1, name: p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), path: p.to_string_lossy().to_string(), mode: MODE_ONESHOT, bpm100: 0, duration: 0.0, track_id: 0, member: 0 }).collect()
}

impl App {
    /// Make the tables once and read the lists.
    pub fn smplib_ready(&mut self) {
        if !self.mixx.lib.ready {
            self.mixx.lib.ready = true;
            let _ = ensure(&self.db.conn);
            self.smplib_reload();
        }
    }

    pub fn smplib_reload(&mut self) {
        self.mixx.lib.lists = lists(&self.db.conn).unwrap_or_default();
        self.mixx.lib.counts = counts(&self.db.conn);
        self.mixx.lib.rows_dirty = true;
    }

    /// "Create New Sampler List" (under the root or a folder): "Untitled Sampler List".
    pub fn smplib_create(&mut self, parent: i64) -> i64 {
        self.smplib_ready();
        let name = free_name(&self.db.conn, parent, "Untitled Sampler List");
        let id = create_list(&self.db.conn, parent, &name, ATTR_LIST).unwrap_or(0);
        if parent > 0 {
            self.mixx.lib.open.insert(parent);
        }
        self.smplib_reload();
        if id > 0 {
            self.mixx.lib.rename = Some((id, name));
        }
        id
    }

    pub fn smplib_delete(&mut self, id: i64) {
        let _ = delete_list(&self.db.conn, id);
        if self.prefs.mixx.lib.osc_list_id == id {
            // (The pack is not imported a second time: the id stays "imported".)
            self.prefs.mixx.lib.osc_list_id = 0;
            self.save_prefs();
        }
        if self.view == View::Sampler(id) {
            self.set_view(View::Sampler(ALL_SAMPLES));
        }
        self.smplib_reload();
    }

    /// Sample rows for collection tracks (made when a track is first used as a sample).
    pub fn smplib_samples_for_tracks(&mut self, ids: &[i64]) -> Vec<i64> {
        self.smplib_ready();
        let mut out = vec![];
        for id in ids {
            if let Some(t) = self.track(*id).cloned() {
                let (path, title, bpm, dur, tid) = (t.path.clone(), t.display_title(), t.bpm, t.duration, t.id);
                let ty = self.cue_tools_type_only(tid);
                let mode = if ty == 0 { MODE_ONESHOT } else { mode_of_type(ty) };
                if let Ok(s) = sample_for(&self.db.conn, &path, &title, mode, (bpm * 100.0) as i64, dur, tid) {
                    out.push(s);
                }
            }
        }
        out
    }

    /// "Add To Sampler List": rekordbox asks before adding what the list already has.
    pub fn smplib_add(&mut self, list: i64, samples: Vec<i64>, force: bool) {
        self.smplib_ready();
        if samples.is_empty() || !self.mixx.lib.lists.iter().any(|l| l.id == list && l.attribute == ATTR_LIST) {
            return;
        }
        if !force && !duplicates(&self.db.conn, list, &samples).is_empty() {
            self.mixx.lib.ask_dup = Some((list, samples));
            return;
        }
        let _ = add_to_list(&self.db.conn, list, &samples);
        self.smplib_reload();
    }

    /// The samples of a tree row: All Samples, Capture, a list, or a folder's lists together.
    pub fn smplib_rows(&mut self, id: i64) -> Vec<Sample> {
        self.smplib_ready();
        match id {
            ALL_SAMPLES => all_samples(&self.db.conn).unwrap_or_default(),
            CAPTURE => capture_files(&crate::sampler_ext::capture_dir(&self.prefs.sampler.capture_dir)),
            _ => {
                let folder = self.mixx.lib.lists.iter().any(|l| l.id == id && l.attribute == ATTR_FOLDER);
                if folder {
                    let kids: Vec<i64> = self.mixx.lib.lists.iter().filter(|l| l.parent == id).map(|l| l.id).collect();
                    kids.into_iter().flat_map(|k| self.smplib_rows(k)).collect()
                } else {
                    list_samples(&self.db.conn, id).unwrap_or_default()
                }
            }
        }
    }

    /// "Load the top 16 tracks into the current sampler bank"
    /// (`treeMessageLoadToSamplerBank` 0x1015794e4).
    pub fn smplib_load_top16(&mut self, id: i64) -> usize {
        let rows = self.smplib_rows(id);
        let bank = self.prefs.sampler.bank.min(3) as usize;
        let mut n = 0;
        for (k, s) in rows.iter().take(16).enumerate() {
            if !Path::new(&s.path).is_file() {
                continue;
            }
            self.smplib_load_slot(bank * 16 + k, s);
            n += 1;
        }
        n
    }

    /// Pack samples (and samples made from files) come without their length and, for loops,
    /// without a BPM: they are read in the background once (rekordbox analyses the tracks a
    /// pack brings in). Called by `smplib_poll`.
    fn smplib_measure(&mut self) {
        if let Some(rx) = &self.mixx.lib.measure_job {
            let got: Vec<(i64, f64, f64)> = rx.try_iter().collect();
            let done = rx.is_empty() && got.iter().any(|g| g.0 == -1);
            for (id, secs, bpm) in got.into_iter().filter(|g| g.0 > 0) {
                let _ = self.db.conn.execute("UPDATE sampler_samples SET duration=?1, bpm100=CASE WHEN mode=?2 AND bpm100=0 THEN ?3 ELSE bpm100 END WHERE id=?4", params![secs, MODE_LOOP, (bpm * 100.0).round() as i64, id]);
                self.mixx.lib.rows_dirty = true;
            }
            if done {
                self.mixx.lib.measure_job = None;
            }
            return;
        }
        if self.mixx.lib.measured_all || !self.mixx.lib.ready {
            return;
        }
        self.mixx.lib.measured_all = true;
        let todo: Vec<(i64, String, i64)> = all_samples(&self.db.conn).unwrap_or_default().into_iter().filter(|s| s.duration <= 0.0 || (s.mode == MODE_LOOP && s.bpm100 == 0)).map(|s| (s.id, s.path, s.mode)).collect();
        if todo.is_empty() {
            return;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            for (id, path, mode) in todo {
                if let Ok(d) = crate::audio::decode_file(&path) {
                    let secs = d.frames() as f64 / d.sr.max(1) as f64;
                    let bpm = if mode == MODE_LOOP {
                        let detected = if secs >= 4.0 { crate::analysis::tempo_and_grid(&d.mono(), d.sr as f64, 70.0, 180.0).0 } else { 0.0 };
                        loop_bpm(secs, detected)
                    } else {
                        0.0
                    };
                    let _ = tx.send((id, secs, bpm));
                }
            }
            let _ = tx.send((-1, 0.0, 0.0));
            ctx.request_repaint();
        });
        self.mixx.lib.measure_job = Some(rx);
    }

    /// A collection track's Track Type (one setting for the track menu and the sampler list).
    pub fn smplib_track_type(&mut self, track_id: i64) -> Option<u8> {
        self.smplib_ready();
        track_mode(&self.db.conn, track_id).map(type_of_mode)
    }

    pub fn smplib_set_track_type(&mut self, track_ids: &[i64], t: u8) {
        self.smplib_ready();
        set_track_mode(&self.db.conn, track_ids, mode_of_type(t));
        self.mixx.lib.rows_dirty = true;
    }

    /// A collection track went into a sampler slot: its Track Type decides whether the slot
    /// loops (Loop) or plays once (Oneshot); Normal leaves the slot's mode as it was.
    pub fn sampler_apply_track_type(&mut self, idx: usize, track_id: i64) {
        let t = self.track_type(track_id);
        if t == 0 {
            return;
        }
        if let Some(Some(slot)) = self.prefs.sampler.slots.get_mut(idx) {
            slot.looped = t == 2;
            self.sampler_push_slot(idx);
            self.save_prefs();
        }
    }

    /// Sampler list rows dropped on a slot: the first one loads there, in its mode.
    pub fn smplib_drop_on_slot(&mut self, idx: usize) {
        if let Some(rows) = self.mixx.lib.drag.take() {
            if let Some(s) = rows.first() {
                self.smplib_load_slot(idx, s);
            }
        }
        self.mixx.lib.drag_import = None;
    }

    /// Load one sample into a slot, in the play mode its Track Type says.
    pub fn smplib_load_slot(&mut self, idx: usize, s: &Sample) {
        self.sampler_load_file(idx, s.path.clone(), s.name.clone(), s.bpm100 as f32 / 100.0);
        if let Some(Some(slot)) = self.prefs.sampler.slots.get_mut(idx) {
            if slot.path == s.path && s.mode != MODE_NORMAL {
                slot.looped = s.mode == MODE_LOOP;
                self.sampler_push_slot(idx);
                self.save_prefs();
            }
        }
    }

    /// Import a sample pack the user chose ("Import Sampler Plus Pack").
    pub fn smplib_import_pack(&mut self, zip_path: &Path) {
        self.smplib_ready();
        match unpack(zip_path, &packs_dir()) {
            Ok((dir, pack)) => match register_pack(&self.db.conn, &dir, &pack) {
                Ok(id) => {
                    if id > 0 {
                        self.mixx.lib.open.insert(id);
                    }
                    self.mixx.lib.measured_all = false;
                    self.smplib_reload();
                    self.toast(format!("Imported {} samples.", pack.tracks.len()));
                }
                Err(e) => self.toast_err(format!("The sampler pack could not be imported: {e}")),
            },
            Err(e) => self.toast_err(format!("The sampler pack could not be imported: {e}")),
        }
    }

    /// Once per frame in PERFORMANCE mode: the OSC pack is imported the first time (when the
    /// installed rekordbox has it), unpacked in the background.
    pub fn smplib_poll(&mut self) {
        if !self.performance() {
            return;
        }
        self.smplib_measure();
        if self.prefs.mixx.lib.osc_list_id == -1 && !self.mixx.lib.osc_tried {
            self.mixx.lib.osc_tried = true;
            let zip = osc_zip();
            if zip.is_file() {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let ctx = self.ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(unpack(&zip, &packs_dir()));
                    ctx.request_repaint();
                });
                self.mixx.lib.osc_job = Some(rx);
            }
        }
        let Some(rx) = &self.mixx.lib.osc_job else { return };
        if let Ok(r) = rx.try_recv() {
            self.mixx.lib.osc_job = None;
            self.smplib_ready();
            match r.and_then(|(dir, pack)| register_pack(&self.db.conn, &dir, &pack).map_err(|e| e.to_string())) {
                Ok(id) => {
                    self.prefs.mixx.lib.osc_list_id = id;
                    self.save_prefs();
                    self.mixx.lib.measured_all = false;
                    self.smplib_reload();
                }
                Err(e) => eprintln!("OSC sampler pack: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        ensure(&c).unwrap();
        c
    }

    #[test]
    fn sampler_lists_round_trip() {
        let c = conn();
        ensure(&c).unwrap();
        let folder = create_list(&c, 0, "Drums", ATTR_FOLDER).unwrap();
        let a = create_list(&c, folder, "Kicks", ATTR_LIST).unwrap();
        let b = create_list(&c, 0, "Untitled Sampler List", ATTR_LIST).unwrap();
        assert_eq!(free_name(&c, 0, "Untitled Sampler List"), "Untitled Sampler List(1)");
        assert_eq!(free_name(&c, folder, "Untitled Sampler List"), "Untitled Sampler List");
        let s1 = sample_for(&c, "/s/kick.wav", "kick", MODE_ONESHOT, 0, 0.4, 0).unwrap();
        let s2 = sample_for(&c, "/s/loop.wav", "loop", MODE_LOOP, 12800, 3.75, 77).unwrap();
        assert_eq!(sample_for(&c, "/s/kick.wav", "other name", MODE_LOOP, 1, 1.0, 0).unwrap(), s1, "one row per file");
        assert_eq!(add_to_list(&c, a, &[s1, s2]).unwrap(), 2);
        assert_eq!(duplicates(&c, a, &[s2, 999]), vec![s2]);
        add_to_list(&c, a, &[s2]).unwrap();
        add_to_list(&c, b, &[s2]).unwrap();
        let ls = lists(&c).unwrap();
        assert_eq!(ls.iter().map(|l| (l.name.as_str(), l.attribute, l.parent, l.seq)).collect::<Vec<_>>(), vec![("Drums", ATTR_FOLDER, 0, 0), ("Untitled Sampler List", ATTR_LIST, 0, 1), ("Kicks", ATTR_LIST, folder, 0)]);
        let rows = list_samples(&c, a).unwrap();
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["kick", "loop", "loop"]);
        assert_eq!((rows[1].mode, rows[1].bpm100, rows[1].track_id), (MODE_LOOP, 12800, 77));
        let cn = counts(&c);
        assert_eq!((cn[&a], cn[&b], cn[&ALL_SAMPLES]), (3, 1, 2));
        // Removing the middle row closes the numbering.
        remove_members(&c, a, &[rows[1].member]).unwrap();
        let nos: Vec<i64> = c.prepare("SELECT track_no FROM sampler_list_tracks WHERE sampler_id=?1 ORDER BY track_no").unwrap().query_map([a], |r| r.get(0)).unwrap().flatten().collect();
        assert_eq!(nos, vec![1, 2]);
        set_mode(&c, &[s1], MODE_LOOP).unwrap();
        assert_eq!(all_samples(&c).unwrap().iter().map(|s| s.mode).collect::<Vec<_>>(), vec![MODE_LOOP, MODE_LOOP]);
        rename_list(&c, b, "Vocals").unwrap();
        assert!(lists(&c).unwrap().iter().any(|l| l.name == "Vocals"));
        // A folder goes with its lists; the samples stay.
        delete_list(&c, folder).unwrap();
        assert_eq!(lists(&c).unwrap().len(), 1);
        assert_eq!(c.query_row("SELECT count(*) FROM sampler_list_tracks", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(all_samples(&c).unwrap().len(), 2);
    }

    #[test]
    fn pack_xml_is_read_even_when_its_root_does_not_close() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<SAMPLER_PACK Version=\"1.0.0\">\n <TRACKS num=\"3\">\n  <TRACK id=\"1\" location=\"A/one.wav\" mode=\"0\"/>\n  <TRACK id=\"2\" location=\"A/two &amp; a half.wav\" mode=\"1\" bpm=\"127.5\"/>\n  <TRACK id=\"3\" location=\"B/three.wav\" mode=\"1\"/>\n </TRACKS>\n <LISTS>\n  <LIST id=\"1\" name=\"PACK\" attr=\"3\" seq=\"0\">\n   <LIST id=\"2\" name=\"A\" attr=\"2\" seq=\"0\">\n    <TRACK key=\"1\"/>\n    <TRACK key=\"2\"/>\n   </LIST>\n   <LIST id=\"3\" name=\"B\" attr=\"2\" seq=\"1\"><TRACK key=\"3\"/></LIST>\n  </LIST>\n </LISTS>\n</SAMPLE_PACK>\n";
        let p = parse_pack_xml(xml);
        assert_eq!(p.tracks.len(), 3);
        assert_eq!(p.tracks[1], PackTrack { id: 2, location: "A/two & a half.wav".into(), mode: 1, bpm: Some(127.5) });
        assert_eq!(p.lists.iter().map(|l| (l.name.as_str(), l.attr, l.parent, l.keys.clone())).collect::<Vec<_>>(), vec![("PACK", 3, None, vec![]), ("A", 2, Some(0), vec![1, 2]), ("B", 2, Some(0), vec![3])]);
        assert_eq!(attr("LIST id=\"1\" name='x y' attr = \"3\"", "attr").as_deref(), Some("3"));
        assert_eq!(attr("LIST id=\"1\" name='x y'", "name").as_deref(), Some("x y"));
        assert_eq!(attr("TRACK contents_id=\"5\"", "id"), None, "a longer name's tail is not the attribute");
    }

    #[test]
    fn pack_import_makes_the_folder_lists_and_samples() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join(format!("recordbox-pack-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let zip_path = tmp.join("MY PACK.zip");
        {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("sampler_pack.xml", o).unwrap();
            zw.write_all(b"<SAMPLER_PACK Version=\"1.0.0\"><TRACKS num=\"2\"><TRACK id=\"1\" location=\"ONE/hit.wav\" mode=\"0\"/><TRACK id=\"2\" location=\"ONE/groove.wav\" mode=\"1\" bpm=\"126\"/><TRACK id=\"3\" location=\"ONE/missing.wav\" mode=\"0\"/></TRACKS><LISTS><LIST id=\"1\" name=\"MY PACK\" attr=\"3\" seq=\"0\"><LIST id=\"2\" name=\"ONE\" attr=\"2\" seq=\"0\"><TRACK key=\"2\"/><TRACK key=\"1\"/><TRACK key=\"3\"/></LIST></LIST></LISTS></SAMPLE_PACK>").unwrap();
            for n in ["ONE/hit.wav", "ONE/groove.wav", "Anlz/00001.DAT", "../escape.wav"] {
                zw.start_file(n, o).unwrap();
                zw.write_all(b"RIFF0000WAVE").unwrap();
            }
            zw.finish().unwrap();
        }
        let dest = tmp.join("Sampler");
        let (dir, pack) = unpack(&zip_path, &dest).unwrap();
        assert_eq!(dir, dest.join("MY PACK"));
        assert!(dir.join("ONE/hit.wav").is_file() && dir.join("ONE/groove.wav").is_file());
        assert!(!dir.join("sampler_pack.xml").exists() && !dir.join("Anlz").exists(), "the XML and Anlz are not kept");
        assert!(!dest.join("escape.wav").exists() && !tmp.join("escape.wav").exists());
        let c = conn();
        let top = register_pack(&c, &dir, &pack).unwrap();
        let ls = lists(&c).unwrap();
        assert_eq!(ls.iter().map(|l| (l.name.as_str(), l.attribute, l.parent)).collect::<Vec<_>>(), vec![("MY PACK", ATTR_FOLDER, 0), ("ONE", ATTR_LIST, top)]);
        let rows = list_samples(&c, ls[1].id).unwrap();
        assert_eq!(rows.iter().map(|r| (r.name.as_str(), r.mode, r.bpm100)).collect::<Vec<_>>(), vec![("groove", MODE_LOOP, 12600), ("hit", MODE_ONESHOT, 0)], "list order, loop BPM x 100, the missing file left out");
        // The same pack again: a second folder beside the first, on disk and in the tree.
        let (dir2, pack2) = unpack(&zip_path, &dest).unwrap();
        assert_eq!(dir2, dest.join("MY PACK(1)"));
        register_pack(&c, &dir2, &pack2).unwrap();
        assert!(lists(&c).unwrap().iter().any(|l| l.name == "MY PACK(1)" && l.parent == 0));
        // Not a pack.
        let other = tmp.join("other.zip");
        {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&other).unwrap());
            zw.start_file("readme.txt", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            zw.write_all(b"x").unwrap();
            zw.finish().unwrap();
        }
        assert!(unpack(&other, &dest).is_err());
        assert!(!dest.join("other").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The installed rekordbox's OSC pack (deflated), when it is there: four one-shot WAVs
    /// in one list inside one folder.
    #[test]
    fn osc_pack_of_the_installed_rekordbox_imports() {
        let zip = osc_zip();
        if !zip.is_file() {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("recordbox-osc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (dir, pack) = unpack(&zip, &tmp).unwrap();
        assert_eq!(dir, tmp.join("OSC_SAMPLER"));
        assert_eq!(pack.tracks.len(), 4);
        for t in &pack.tracks {
            let b = std::fs::read(dir.join(&t.location)).unwrap();
            assert_eq!(&b[..4], b"RIFF", "{}", t.location);
            assert_eq!(&b[8..12], b"WAVE");
        }
        let c = conn();
        let top = register_pack(&c, &dir, &pack).unwrap();
        let ls = lists(&c).unwrap();
        assert_eq!(ls.len(), 2);
        assert_eq!((ls[0].id, ls[0].attribute, ls[1].parent, ls[1].attribute), (top, ATTR_FOLDER, top, ATTR_LIST));
        assert_eq!(list_samples(&c, ls[1].id).unwrap().len(), 4);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
