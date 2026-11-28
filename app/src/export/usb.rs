//! USB export: keeps a device library (tracks + playlists) on the drive, merging with what is
//! already there, copies audio into /Contents, writes analysis files and rewrites
//! /PIONEER/rekordbox/export.pdb and exportExt.pdb.

use super::usb_target as target;
pub use target::{check_export_target, inspect_target, DeviceLayout};
use super::anlz;
use super::pdb::{self, dstr, dstr_ucs2, row_with_strings, Database, TableSpec};
use crate::model::*;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DevTrack {
    pub id: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub label: String,
    pub remixer: String,
    pub original_artist: String,
    pub composer: String,
    pub key_name: String,
    pub comment: String,
    pub mix_name: String,
    pub message: String,
    pub lyricist: String,
    pub year: u16,
    pub track_no: u32,
    pub disc_no: u16,
    pub duration: u16,
    pub bpm100: u32,
    pub rating: u8,
    pub color: u8,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub sample_depth: u16,
    pub file_size: u32,
    pub file_type: u16,
    pub file_path: String,
    pub analyze_path: String,
    pub date_added: String,
    pub release_date: String,
    pub analyze_date: String,
    pub artwork_path: String,
    pub play_count: u16,
    pub autoload: bool,
    /// Local source file for tracks this app exported (empty for foreign tracks).
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The row's original fixed header and 21 strings when it was read from a drive, so fields
    /// Recordbox does not manage (ISRC, unknown slots and constants) are written back unchanged.
    #[serde(skip)]
    pub raw_head: Vec<u8>,
    #[serde(skip)]
    pub raw_strings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DevNode {
    pub id: u32,
    pub parent: u32,
    pub name: String,
    pub folder: bool,
    pub seq: u32,
    pub entries: Vec<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DevHistory {
    pub id: u32,
    pub name: String,
    pub entries: Vec<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceLibrary {
    pub tracks: BTreeMap<u32, DevTrack>,
    pub nodes: Vec<DevNode>,
    pub histories: Vec<DevHistory>,
    /// My Tag categories (name, tags).
    pub tag_categories: Vec<(String, Vec<String>)>,
    /// IDs the drive's exportExt.pdb already uses, keyed by `tag_key(category, tag)` (a
    /// category itself is keyed by its name). Reused on the next export so IDs stay stable.
    #[serde(default)]
    pub tag_ids: BTreeMap<String, u32>,
    /// Browse categories and sort options the players offer (Preferences > DJ System >
    /// Category / Sort). None keeps the original's defaults.
    #[serde(default)]
    pub browse: Option<UsbBrowse>,
    /// Data pages of the tables Recordbox does not manage (export.pdb page types 9, 10, 14, 15
    /// and the synchronisation history 19), exactly as they were on the drive. Written back
    /// unchanged; table 19 gets one more row per rewrite, as rekordbox adds one per sync.
    #[serde(skip)]
    pub raw_tables: BTreeMap<u32, Vec<Vec<u8>>>,
    /// The same for exportExt.pdb (every table except My Tags 3 and 4 and the constant 7).
    #[serde(skip)]
    pub raw_ext_tables: BTreeMap<u32, Vec<Vec<u8>>>,
    /// The device label rekordbox keeps in the newest row of table 19 ("Device or backup
    /// label"; empty on most drives).
    #[serde(default)]
    pub device_name: String,
    /// Names of the eight track colours in the drive's colour table (Pink … Purple unless the
    /// user renamed them: Preferences > Customize Color Comments).
    #[serde(default)]
    pub color_names: Vec<String>,
}

/// Export.pdb tables Recordbox keeps as they are (see [`DeviceLibrary::raw_tables`]).
pub const KEPT_TABLES: [u32; 5] = [9, 10, 14, 15, 16];
/// exportExt.pdb tables kept as they are.
pub const KEPT_EXT_TABLES: [u32; 7] = [0, 1, 2, 5, 6, 7, 8];

/// Active browse categories and sort options, in order, by COLUMNS id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbBrowse {
    pub categories: Vec<u16>,
    pub sorts: Vec<u16>,
}

/// Categories that are always active (shown greyed in the original's Category page).
pub const FIXED_CATEGORIES: [u16; 5] = [4, 17, 19, 20, 24];
/// Sort options that are always active (DEFAULT, ALPHABET/TRACK NAME).
pub const FIXED_SORTS: [u16; 2] = [0x19, 0x1a];

impl Default for UsbBrowse {
    fn default() -> Self {
        let order = |rows: &[[u8; 8]], vis: usize, pos: usize| -> Vec<u16> {
            let mut v: Vec<(u8, u16)> = rows.iter().filter(|r| r[vis] != 1).map(|r| (r[pos], u16::from_le_bytes([r[0], r[1]]))).collect();
            v.sort();
            v.into_iter().map(|x| x.1).collect()
        };
        UsbBrowse { categories: order(&MENU, 5, 6), sorts: order(&TABLE18, 4, 5) }
    }
}

impl UsbBrowse {
    /// Every category the Category page can offer (id, name).
    pub fn all_categories() -> Vec<(u16, &'static str)> {
        MENU.iter().map(|r| u16::from_le_bytes([r[0], r[1]])).filter_map(|id| column_name(id).map(|n| (id, n))).collect()
    }
    pub fn all_sorts() -> Vec<(u16, &'static str)> {
        TABLE18.iter().map(|r| u16::from_le_bytes([r[0], r[1]])).filter_map(|id| column_name(id).map(|n| (id, n))).collect()
    }
    /// Fixed entries first, in the original's order, then the user's choices.
    pub fn normalized(&self) -> UsbBrowse {
        let mut c: Vec<u16> = self.categories.iter().copied().filter(|id| MENU.iter().any(|r| u16::from_le_bytes([r[0], r[1]]) == *id)).collect();
        for f in FIXED_CATEGORIES {
            if !c.contains(&f) {
                c.push(f);
            }
        }
        let mut s: Vec<u16> = FIXED_SORTS.to_vec();
        s.extend(self.sorts.iter().copied().filter(|id| !FIXED_SORTS.contains(id) && TABLE18.iter().any(|r| u16::from_le_bytes([r[0], r[1]]) == *id)));
        c.dedup();
        s.dedup();
        UsbBrowse { categories: c, sorts: s }
    }
}

pub fn column_name(id: u16) -> Option<&'static str> {
    COLUMNS.iter().find(|c| c.0 == id).map(|c| c.2)
}

/// Key for a tag inside its category (tags with the same name in two categories differ).
pub fn tag_key(category: &str, tag: &str) -> String {
    format!("{category}\u{1f}{tag}")
}

pub fn file_type_code(ext: &str) -> u16 {
    match ext.to_ascii_lowercase().as_str() {
        "mp3" => 1,
        "m4a" | "aac" | "mp4" | "alac" => 4,
        "flac" => 5,
        "wav" => 11,
        "aif" | "aiff" => 12,
        _ => 0,
    }
}

pub fn player_supported(ext: &str) -> bool {
    file_type_code(ext) != 0
}

// ------------------------------------------------------------------------------------------
// Reading an existing device library
// ------------------------------------------------------------------------------------------

/// The drive carries rekordbox's newer OneLibrary catalogue (exportLibrary.db).
pub fn has_onelibrary(root: &Path) -> bool {
    matches!(DeviceLayout::inspect(root), Ok(DeviceLayout::OneLibrary | DeviceLayout::Mixed))
}

pub fn pdb_path(root: &Path) -> PathBuf {
    target::Catalogs::inspect(root).ok().and_then(|c| c.main).unwrap_or_else(|| root.join("PIONEER/rekordbox/export.pdb"))
}

fn sidecar_path(root: &Path) -> PathBuf {
    root.join(".Recordbox").join("device.json")
}

fn cue_record_path(root: &Path) -> PathBuf {
    root.join(".Recordbox").join("cues.json")
}

/// Cue fingerprints of the tracks Recordbox last wrote to this drive, by media path.
pub fn read_cue_record(root: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(cue_record_path(root)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Update the drive's cue record (Recordbox's own folder; players never read it).
pub fn write_cue_record(root: &Path, record: &HashMap<String, String>) -> Result<()> {
    write_private(root, "cues.json", &serde_json::to_vec_pretty(&record.iter().collect::<BTreeMap<_, _>>())?)
}

pub(super) fn check_preference_target(root: &Path) -> Result<DeviceLayout> {
    let layout = check_export_target(root)?;
    if layout != DeviceLayout::Empty {
        read_device(root).context("reading the classic catalog before changing this device")?;
    }
    Ok(layout)
}

pub(super) fn write_private(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    check_preference_target(root)?;
    let path = root.join(".Recordbox").join(name);
    target::confined(root, &path)?;
    target::confined(root, &tmp_for(&path))?;
    anyhow::ensure!(target::managed_file(root, &path), "Unknown private USB record");
    std::fs::create_dir_all(root.join(".Recordbox"))?;
    write_atomic(&path, bytes)
}

/// A track whose cues on the drive differ from what Recordbox last wrote there: edited on a
/// player (or by another program).
#[derive(Clone, Debug)]
pub struct PlayerCueEdit {
    pub file_path: String,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub cues: Vec<Cue>,
    /// Fingerprint Recordbox recorded at export.
    pub exported: String,
    /// Fingerprint of the cues now on the drive.
    pub now: String,
}

/// Tracks on the drive whose cues changed since Recordbox exported them.
pub fn player_cue_edits(root: &Path, lib: &DeviceLibrary) -> Vec<PlayerCueEdit> {
    let record = read_cue_record(root);
    // An edit made on a player rewrites the analysis file, so only files newer than the record
    // (with FAT's 2-second timestamp resolution as margin) need reading.
    let since = std::fs::metadata(cue_record_path(root)).and_then(|m| m.modified()).ok().map(|t| t + std::time::Duration::from_secs(2));
    let newer = |p: &Path| match (since, std::fs::metadata(p).and_then(|m| m.modified())) {
        (Some(s), Ok(m)) => m > s,
        _ => true,
    };
    let mut out = vec![];
    for t in lib.tracks.values() {
        let Some(exported) = record.get(&t.file_path) else { continue };
        let Some(base) = anlz_base(&t.analyze_path) else { continue };
        let b = root.join(base.trim_start_matches('/'));
        if !newer(&b.with_extension("EXT")) && !newer(&b.with_extension("DAT")) {
            continue;
        }
        let ext = anlz::read_section_list(&b.with_extension("EXT"));
        let dat = anlz::read_section_list(&b.with_extension("DAT"));
        if ext.is_empty() && dat.is_empty() {
            continue;
        }
        let cues = anlz::parse_cues(&ext, &dat);
        let now = anlz::cue_fingerprint(&cues);
        if &now != exported {
            out.push(PlayerCueEdit { file_path: t.file_path.clone(), source: t.source.clone(), title: t.title.clone(), artist: t.artist.clone(), cues, exported: exported.clone(), now });
        }
    }
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

fn asset_key(path: &Path) -> String {
    // A PDB uses '/' even on Windows. Directory enumeration can return the
    // same location with native separators, including an entirely different
    // spelling of the path below the mount point.
    #[cfg(windows)]
    return path_key(&path.to_string_lossy().replace('\\', "/"));
    #[cfg(not(windows))]
    path_key(&path.to_string_lossy())
}

/// Every named asset still owned by a classic track. Use the same comparison
/// for manual cleanup and track removal on FAT and macOS volumes.
fn referenced_files(root: &Path, lib: &DeviceLibrary) -> std::collections::HashSet<String> {
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for t in lib.tracks.values() {
        if !t.file_path.is_empty() {
            used.insert(asset_key(&root.join(t.file_path.trim_start_matches('/'))));
        }
        if let Some(b) = anlz_base(&t.analyze_path) {
            let b = root.join(b.trim_start_matches('/'));
            for e in ANLZ_EXTS {
                used.insert(asset_key(&b.with_extension(e)));
            }
        }
        if !t.artwork_path.is_empty() {
            let a = root.join(t.artwork_path.trim_start_matches('/'));
            if let (Some(stem), Some(ext)) = (a.file_stem().map(|x| x.to_string_lossy().to_string()), a.extension().map(|x| x.to_string_lossy().to_string())) {
                used.insert(asset_key(&a.with_file_name(format!("{stem}_m.{ext}"))));
            }
            used.insert(asset_key(&a));
        }
    }
    used
}

/// Files in the drive's music, analysis and artwork folders that no track in its library uses
/// (left behind by deleted playlists, other programs or copies made by hand), with sizes.
/// Nothing is listed while an interrupted export's journal is still on the drive.
pub fn unused_files(root: &Path, lib: &DeviceLibrary) -> Vec<(PathBuf, u64)> {
    // OneLibrary can reference files outside export.pdb. Playlist inspection cannot establish that a file is unused.
    if journal_dir(root).join("journal.log").exists() || !matches!(check_export_target(root), Ok(DeviceLayout::Classic)) {
        return vec![];
    }
    let used = referenced_files(root, lib);
    let mut out = vec![];
    let mut stack: Vec<PathBuf> = [root.join("Contents"), root.join("PIONEER").join("USBANLZ"), root.join("PIONEER").join("Artwork")].into_iter().filter(|d| d.is_dir()).collect();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && !used.contains(&asset_key(&p)) {
                let name = e.file_name().to_string_lossy().to_string();
                // macOS bookkeeping files are not the library's concern.
                if name == ".DS_Store" || name.starts_with("._") {
                    continue;
                }
                out.push((p, e.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    out.sort();
    out
}

/// Delete the given unused files (re-checked against the drive's current library) and then
/// any folders left empty. Returns how many files were deleted.
pub fn delete_unused(root: &Path, files: &[PathBuf]) -> Result<usize> {
    anyhow::ensure!(check_export_target(root)? == DeviceLayout::Classic, "Cleanup needs a classic-only device catalog");
    let lib = read_device(root)?;
    let still: std::collections::HashSet<PathBuf> = unused_files(root, &lib).into_iter().map(|f| f.0).collect();
    let mut n = 0;
    for f in files.iter().filter(|f| still.contains(*f)) {
        target::confined(root, f)?;
        if std::fs::remove_file(f).is_ok() {
            n += 1;
            let mut dir = f.parent().map(Path::to_path_buf);
            while let Some(d) = dir {
                if d == root.join("Contents") || d == root.join("PIONEER").join("USBANLZ") || d == root.join("PIONEER").join("Artwork") || !d.starts_with(root) {
                    break;
                }
                if std::fs::remove_dir(&d).is_err() {
                    break;
                }
                dir = d.parent().map(Path::to_path_buf);
            }
        }
    }
    Ok(n)
}

/// Drives written before the rename keep Recordbox's own data in `/.PrepDeck`: it moves to
/// `/.Recordbox` (journal, backups, sidecar and cue record included) the first time the drive
/// is read or written.
pub fn migrate_app_dir(root: &Path) {
    let (old, new) = (root.join(".PrepDeck"), root.join(".Recordbox"));
    if !new.exists() && old.is_dir() && check_export_target(root).is_ok() {
        let _ = std::fs::rename(&old, &new);
        sync_dir(root);
    }
}

pub fn read_device(root: &Path) -> Result<DeviceLibrary> {
    let catalogs = target::Catalogs::inspect(root)?;
    let Some(main) = catalogs.main.as_ref() else {
        anyhow::ensure!(catalogs.layout() == DeviceLayout::Empty, "OneLibrary (Device Library Plus) is present without a readable classic catalog");
        return Ok(DeviceLibrary::default());
    };
    let mut lib = parse_pdb(std::fs::read(main)?)?;
    // Sources recorded by this app (so re-exports update instead of duplicating).
    if let Ok(s) = std::fs::read_to_string(sidecar_path(root)) {
        if let Ok(m) = serde_json::from_str::<HashMap<String, String>>(&s) {
            for t in lib.tracks.values_mut() {
                if let Some(src) = m.get(&t.file_path) {
                    t.source = src.clone();
                }
            }
        }
    }
    if let Some(ext) = catalogs.ext {
        read_ext_tags(std::fs::read(ext)?, &mut lib).context("reading the extended classic USB catalog")?;
    }
    Ok(lib)
}

pub fn parse_pdb(bytes: Vec<u8>) -> Result<DeviceLibrary> {
    let db = Database::parse(bytes)?;
    db.validate_table_set(0..20)?;
    db.validate_track_rows()?;
    let mut names: HashMap<(u32, u32), String> = HashMap::new();
    // artists (2)
    for (h, o) in db.rows(2) {
        let r = db.row_bytes(h, o);
        let sub = pdb::rd_u16(r, 0);
        let id = pdb::rd_u32(r, 4);
        let name = if sub & 4 != 0 { pdb::row_string(r, 8, 0, true) } else { pdb::row_string(r, 8, 0, false) };
        names.insert((2, id), name);
    }
    let mut album_artist: HashMap<u32, u32> = HashMap::new();
    for (h, o) in db.rows(3) {
        let r = db.row_bytes(h, o);
        let sub = pdb::rd_u16(r, 0);
        let artist = pdb::rd_u32(r, 8);
        let id = pdb::rd_u32(r, 12);
        let name = pdb::row_string(r, 20, 0, sub & 4 != 0);
        names.insert((3, id), name);
        album_artist.insert(id, artist);
    }
    for t in [1u32, 4] {
        for (h, o) in db.rows(t) {
            let r = db.row_bytes(h, o);
            names.insert((t, pdb::rd_u32(r, 0)), pdb::read_dstr(r, 4));
        }
    }
    for (h, o) in db.rows(5) {
        let r = db.row_bytes(h, o);
        names.insert((5, pdb::rd_u32(r, 0)), pdb::read_dstr(r, 8));
    }
    for (h, o) in db.rows(13) {
        let r = db.row_bytes(h, o);
        names.insert((13, pdb::rd_u32(r, 0)), pdb::read_dstr(r, 4));
    }
    let nm = |t: u32, id: u32| -> String { if id == 0 { String::new() } else { names.get(&(t, id)).cloned().unwrap_or_default() } };
    let mut lib = DeviceLibrary::default();
    for (h, o) in db.rows(0) {
        let r = db.row_bytes(h, o);
        if r.len() < 0x5e {
            continue;
        }
        let wide = pdb::rd_u16(r, 0) & 4 != 0;
        let s = |i: usize| pdb::row_string(r, 0x5c, i, wide);
        let album_id = pdb::rd_u32(r, 0x40);
        let raw_head = r[..0x5c].to_vec();
        let raw_strings: Vec<String> = (0..21).map(|i| s(i)).collect();
        let t = DevTrack {
            raw_head,
            raw_strings,
            id: pdb::rd_u32(r, 0x48),
            sample_rate: pdb::rd_u32(r, 0x08),
            composer: nm(2, pdb::rd_u32(r, 0x0c)),
            file_size: pdb::rd_u32(r, 0x10),
            artwork_path: nm(13, pdb::rd_u32(r, 0x1c)),
            key_name: nm(5, pdb::rd_u32(r, 0x20)),
            original_artist: nm(2, pdb::rd_u32(r, 0x24)),
            label: nm(4, pdb::rd_u32(r, 0x28)),
            remixer: nm(2, pdb::rd_u32(r, 0x2c)),
            bitrate: pdb::rd_u32(r, 0x30),
            track_no: pdb::rd_u32(r, 0x34),
            bpm100: pdb::rd_u32(r, 0x38),
            genre: nm(1, pdb::rd_u32(r, 0x3c)),
            album: nm(3, album_id),
            album_artist: album_artist.get(&album_id).map(|a| nm(2, *a)).unwrap_or_default(),
            artist: nm(2, pdb::rd_u32(r, 0x44)),
            disc_no: pdb::rd_u16(r, 0x4c),
            play_count: pdb::rd_u16(r, 0x4e),
            year: pdb::rd_u16(r, 0x50),
            sample_depth: pdb::rd_u16(r, 0x52),
            duration: pdb::rd_u16(r, 0x54),
            color: r[0x58],
            rating: r[0x59],
            file_type: pdb::rd_u16(r, 0x5a),
            lyricist: s(1),
            message: s(5),
            autoload: s(7) == "ON",
            date_added: s(10),
            release_date: s(11),
            mix_name: s(12),
            analyze_path: s(14),
            analyze_date: s(15),
            comment: s(16),
            title: s(17),
            file_path: s(20),
            ..Default::default()
        };
        lib.tracks.insert(t.id, t);
    }
    for (h, o) in db.rows(7) {
        let r = db.row_bytes(h, o);
        lib.nodes.push(DevNode {
            parent: pdb::rd_u32(r, 0),
            seq: pdb::rd_u32(r, 8),
            id: pdb::rd_u32(r, 12),
            folder: pdb::rd_u32(r, 16) != 0,
            name: pdb::read_dstr(r, 20),
            entries: vec![],
        });
    }
    let mut entries: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for (h, o) in db.rows(8) {
        let r = db.row_bytes(h, o);
        entries.entry(pdb::rd_u32(r, 8)).or_default().push((pdb::rd_u32(r, 0), pdb::rd_u32(r, 4)));
    }
    for n in lib.nodes.iter_mut() {
        if let Some(mut e) = entries.remove(&n.id) {
            e.sort();
            n.entries = e.into_iter().map(|x| x.1).filter(|t| lib.tracks.contains_key(t)).collect();
        }
    }
    for (h, o) in db.rows(11) {
        let r = db.row_bytes(h, o);
        lib.histories.push(DevHistory { id: pdb::rd_u32(r, 0), name: pdb::read_dstr(r, 4), entries: vec![] });
    }
    let mut hent: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for (h, o) in db.rows(12) {
        let r = db.row_bytes(h, o);
        hent.entry(pdb::rd_u32(r, 4)).or_default().push((pdb::rd_u32(r, 8), pdb::rd_u32(r, 0)));
    }
    for hh in lib.histories.iter_mut() {
        if let Some(mut e) = hent.remove(&hh.id) {
            e.sort();
            hh.entries = e.into_iter().map(|x| x.1).collect();
        }
    }
    // Colour names (id at 5, name at 8).
    let mut colors: Vec<(u8, String)> = db.rows(6).into_iter().map(|(h, o)| db.row_bytes(h, o)).filter(|r| r.len() > 8).map(|r| (r[5], pdb::read_dstr(r, 8))).collect();
    colors.sort();
    if colors.len() == 8 && colors.iter().enumerate().all(|(i, c)| c.0 as usize == i + 1) {
        lib.color_names = colors.into_iter().map(|c| c.1).collect();
    }
    // Tables Recordbox does not manage, kept page by page.
    for t in KEPT_TABLES.into_iter().chain([19]) {
        let pages = db.data_pages(t);
        if t == 16 || !pages.is_empty() {
            lib.raw_tables.insert(t, pages);
        }
    }
    // Device label: third string of the present history row (date, 0x1e19, version, label).
    for (h, o) in db.rows(19) {
        let r = db.row_bytes(h, o);
        let date_len = dstr_len(r, 12);
        let ver_at = 12 + date_len + 2;
        lib.device_name = pdb::read_dstr(r, ver_at + dstr_len(r, ver_at));
    }
    Ok(lib)
}

/// Encoded length of the DeviceSQL string at `data[pos..]`.
fn dstr_len(data: &[u8], pos: usize) -> usize {
    match data.get(pos) {
        Some(h) if h & 1 == 1 => (*h >> 1) as usize,
        Some(_) => pdb::rd_u16(data, pos + 1) as usize,
        None => 0,
    }
}

fn read_ext_tags(bytes: Vec<u8>, lib: &mut DeviceLibrary) -> Result<()> {
    let db = Database::parse(bytes)?;
    db.validate_table_set(0..9)?;
    db.validate_ext_rows()?;
    for t in KEPT_EXT_TABLES {
        let pages = db.data_pages(t);
        if t == 7 || !pages.is_empty() {
            lib.raw_ext_tables.insert(t, pages);
        }
    }
    struct T {
        parent: u32,
        pos: u32,
        id: u32,
        cat: bool,
        name: String,
    }
    let mut tags = vec![];
    for (h, o) in db.rows(3) {
        let r = db.row_bytes(h, o);
        let wide = pdb::rd_u16(r, 0) & 4 != 0;
        tags.push(T {
            parent: pdb::rd_u32(r, 12),
            pos: pdb::rd_u32(r, 16),
            id: pdb::rd_u32(r, 20),
            cat: r.get(27).copied().unwrap_or(0) == 1,
            name: pdb::row_string(r, 0x1c, 0, wide),
        });
    }
    let cat_name: HashMap<u32, String> = tags.iter().filter(|t| t.cat).map(|t| (t.id, t.name.clone())).collect();
    let by_id: HashMap<u32, String> = tags
        .iter()
        .filter(|t| !t.cat)
        .map(|t| (t.id, tag_key(cat_name.get(&t.parent).map(String::as_str).unwrap_or(""), &t.name)))
        .collect();
    for t in &tags {
        let key = if t.cat { t.name.clone() } else { by_id.get(&t.id).cloned().unwrap_or_default() };
        lib.tag_ids.insert(key, t.id);
    }
    let mut cats: Vec<&T> = tags.iter().filter(|t| t.cat).collect();
    cats.sort_by_key(|t| t.pos);
    lib.tag_categories = cats
        .iter()
        .map(|c| {
            let mut kids: Vec<&T> = tags.iter().filter(|t| !t.cat && t.parent == c.id).collect();
            kids.sort_by_key(|t| t.pos);
            (c.name.clone(), kids.iter().map(|t| t.name.clone()).collect())
        })
        .collect();
    for (h, o) in db.rows(4) {
        let r = db.row_bytes(h, o);
        let track = pdb::rd_u32(r, 4);
        let tag = pdb::rd_u32(r, 8);
        let t = lib.tracks.get_mut(&track).with_context(|| format!("Unknown track {track} in extended USB tag memberships"))?;
        let name = by_id.get(&tag).with_context(|| format!("Unknown tag {tag} in extended USB tag memberships"))?;
        t.tags.push(name.clone());
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Paths on the drive (rules recovered from the original, see MECHANICS.md)
// ------------------------------------------------------------------------------------------

/// How names are spelled on a drive's filesystem, which is how its players read them back:
/// HFS+ stores every name decomposed (and hands it back that way), FAT32 and exFAT store the
/// characters as given, for which rekordbox asks for the precomposed form
/// (rekordbox-rea reports/usb-export-paths.md).
pub fn fs_decomposes(fs: &str) -> bool {
    fs == "hfs"
}

fn sanitize(s: &str) -> String {
    sanitize_fs(s, false)
}

fn sanitize_fs(s: &str, decomposed: bool) -> String {
    use unicode_normalization::UnicodeNormalization;
    // The inhibited-character and length rules are applied to the precomposed text, so a
    // name is cut at the same place whatever the drive; only the final spelling differs.
    let s: String = s.nfc().collect();
    let out = sanitize_nfc(&s);
    if decomposed {
        // HFS+ uses Apple's variant of NFD, which leaves U+2000–2FFF, U+F900–FAFF and
        // U+2F800–2FAFF composed.
        out.chars().flat_map(|c| if matches!(c as u32, 0x2000..=0x2fff | 0xf900..=0xfaff | 0x2f800..=0x2faff) { vec![c] } else { c.to_string().nfd().collect() }).collect()
    } else {
        out
    }
}

fn sanitize_nfc(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if "/\\:?*\"<>|~".contains(c) || c.is_control() { '_' } else { c }).collect();
    out = out.trim().to_string();
    if out.ends_with('.') {
        out.pop();
        out.push('_');
    }
    out
}

fn cap(s: &str, n: usize) -> String {
    let mut v: String = s.chars().take(n).collect();
    while v.ends_with('.') || v.ends_with(' ') {
        v.pop();
    }
    v
}

pub fn media_dir(artist: &str, album: &str) -> String {
    media_dir_fs(artist, album, false)
}

/// The media folder with names spelled for the drive's filesystem. The 48-character limit is
/// counted on the precomposed text.
pub fn media_dir_fs(artist: &str, album: &str, decomposed: bool) -> String {
    let mut a = cap(&sanitize(artist), 48);
    let mut b = cap(&sanitize(album), 48);
    if a.is_empty() {
        a = "UnknownArtist".into();
    }
    if b.is_empty() {
        b = "UnknownAlbum".into();
    }
    if decomposed {
        a = sanitize_fs(&a, true);
        b = sanitize_fs(&b, true);
    }
    format!("/Contents/{a}/{b}")
}

/// Case- and normalisation-insensitive form of a device path, for comparing a path in the
/// database with one built here (FAT names ignore case; HFS+ hands back decomposed accents).
pub fn path_key(p: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    p.nfc().collect::<String>().to_lowercase()
}

fn file_candidate(stem: &str, ext: &str, suffix: Option<u32>) -> String {
    let tail = match suffix {
        Some(n) => format!("-{n}"),
        None => String::new(),
    };
    let dot_ext = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    let room = 48usize.saturating_sub(tail.chars().count() + dot_ext.chars().count()).max(1);
    let st = cap(stem, room);
    format!("{st}{tail}{dot_ext}")
}

// ------------------------------------------------------------------------------------------
// Export job
// ------------------------------------------------------------------------------------------

/// One local track ready to go to the device.
#[derive(Clone)]
pub struct ExportTrack {
    pub track: Track,
    pub cues: Vec<Cue>,
    pub grid: Option<BeatGrid>,
    pub wave: Option<Waveform>,
    pub tags: Vec<(String, String)>, // (category, tag)
    /// rekordbox's own analysis files for this audio file (base path without extension).
    pub rb_anlz: Option<PathBuf>,
    /// The track's grid is still the one rekordbox analysed (phrases/extended grid stay valid).
    pub grid_same: bool,
    /// Recordbox's vocal detection (levels 0–4 per 46.4 ms), written as PVDI when rekordbox's
    /// own is not available.
    pub vocal: Option<Vec<u8>>,
    /// Phrases the user edited (PHRASE EDIT): written instead of any analysed ones.
    pub phrases: Option<crate::phrase::Structure>,
}

impl ExportTrack {
    /// rekordbox's own phrase section can be used (its grid is unchanged and it has one).
    fn has_rb_phrases(&self) -> bool {
        self.phrases.is_none() && self.grid_same && self.rb_anlz.as_ref().is_some_and(|b| anlz::read_pssi(&b.with_extension("EXT")).is_some())
    }
}

/// A playlist (or folder) to create on the device, depth-first order.
#[derive(Clone, Default)]
pub struct ExportNode {
    /// The library's ID of this playlist or folder (0 = none). Recordbox records which device
    /// node each one became (`/.Recordbox/nodes.json`), so two playlists with the same name in
    /// one folder stay two playlists and a renamed playlist updates its device copy.
    pub id: i64,
    pub name: String,
    pub folder: bool,
    pub children: Vec<ExportNode>,
    pub tracks: Vec<i64>,
}

/// What an export did, for the result summary.
#[derive(Clone, Debug, Default)]
pub struct ExportReport {
    /// Tracks whose audio was copied to the drive by this export.
    pub copied: usize,
    /// Tracks already on the drive whose analysis, cues and details were updated.
    pub updated: usize,
    /// Playlists and folders removed from the drive (synchronisation).
    pub removed_lists: usize,
    /// Tracks removed from the drive because no playlist uses them any more.
    pub removed_tracks: usize,
    /// Tracks left out, each as "title (reason)".
    pub skipped: Vec<String>,
    pub tracks_on_device: usize,
    pub playlists_on_device: usize,
}

pub enum Progress {
    Step { done: usize, total: usize, text: String },
    /// Sent once, just before `Finished(Ok)`.
    Report(ExportReport),
    /// A track that had no analysis was analysed on the export thread; the app stores it.
    Analyzed(i64, Box<Result<crate::analysis::AnalysisResult, String>>),
    Finished(Result<String, String>),
}

#[derive(Default)]
pub struct ExportRequest {
    /// The drive's filesystem as macOS names it ("msdos" = FAT32, "exfat", "hfs", "apfs";
    /// "folder" or empty for a folder used as a device): decides how names are spelled
    /// (decomposed on HFS+) and whether FAT32's limits apply.
    pub fs: String,
    /// Names of the eight track colours for the drive's colour table (None keeps the drive's).
    pub color_names: Option<Vec<String>>,
    /// Copy a track's audio again when the local file's size differs from the drive's copy or
    /// it was modified after that copy was made (synchronisation).
    pub recopy_changed: bool,
    /// Device playlists/folders to remove in the same write (synchronisation: lists that were
    /// unticked or deleted in the library), by device node ID.
    pub remove_nodes: Vec<u32>,
    /// With `remove_nodes`: also remove their tracks that no remaining playlist uses.
    pub delete_unreferenced: bool,
    /// Tracks already on the drive keep the cues the drive has (Sync Manager without
    /// "including cue points"); new tracks always get the library's cues.
    pub keep_device_cues: bool,
    /// Text for table 19's device label (None keeps the drive's).
    pub device_name: Option<String>,
    pub root: PathBuf,
    pub nodes: Vec<ExportNode>,
    pub tracks: HashMap<i64, ExportTrack>,
    pub tag_categories: Vec<(String, Vec<String>)>,
    /// Tracks the app left out before the job started, with the reason (shown in the result).
    pub skipped: Vec<String>,
    /// Settings for analysing tracks that have no grid or waveform yet.
    pub analysis: crate::analysis::AnalysisSettings,
    /// Player settings files written to /PIONEER (MYSETTING.DAT etc.).
    pub settings: super::settings::UsbSettings,
    /// Browse categories / sort options for the players' menus.
    pub browse: UsbBrowse,
    /// Write phrase sections (PSSI) for tracks without rekordbox's own.
    pub phrases: bool,
    pub classic_phrase_estimator: bool,
}

/// Files this export created. Unless the export commits, they are removed again, so a
/// stopped or failed export leaves the drive as it was.
/// Export journal on the drive (`/.Recordbox/journal/journal.log`): every file this export
/// creates ("C" lines) and, before an existing file is overwritten, a durable copy of its old
/// bytes ("R" lines). "P" is written once both new databases are fully on the drive. An
/// export that fails is undone from it at once; one cut short by a crash, power loss or the
/// drive being pulled is undone (before "P") or finished (after "P") by [`recover_device`]
/// the next time the drive is seen.
struct Created {
    root: PathBuf,
    dir: PathBuf,
    log: Option<std::fs::File>,
    files: Vec<PathBuf>,
    /// Existing files this export overwrote, with the journal copy of their previous bytes.
    replaced: Vec<(PathBuf, PathBuf)>,
    publishing: bool,
    committed: bool,
    mixed: bool,
    /// Files to delete after publication ("D" lines).
    doomed: Vec<PathBuf>,
}

/// Delete a file and then any folders it leaves empty (never the drive's top-level ones).
fn remove_file_and_empty_dirs(root: &Path, f: &Path) {
    if !f.starts_with(root) || std::fs::remove_file(f).is_err() {
        return;
    }
    prune_empty_parents(root, f);
}

fn prune_empty_parents(root: &Path, f: &Path) {
    let keep = [root.to_path_buf(), root.join("Contents"), root.join("PIONEER"), root.join(".PIONEER"), root.join("PIONEER").join("USBANLZ"), root.join("PIONEER").join("Artwork"), root.join(".Recordbox")];
    let mut dir = f.parent().map(Path::to_path_buf);
    while let Some(d) = dir {
        if keep.contains(&d) || !d.starts_with(root) || std::fs::remove_dir(&d).is_err() {
            break;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
}

fn journal_dir(root: &Path) -> PathBuf {
    root.join(".Recordbox").join("journal")
}

#[cfg(test)]
thread_local! {
    /// Fault injection: fail at this journal step (1-based); with `CRASH`, skip the in-process
    /// rollback as if the program had died there.
    pub(crate) static FAIL_AT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static STEP: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static CRASH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn fault_point() -> Result<()> {
    #[cfg(test)]
    {
        let n = STEP.with(|s| {
            s.set(s.get() + 1);
            s.get()
        });
        if FAIL_AT.with(|f| f.get()) == n {
            return Err(anyhow!("injected fault at step {n}"));
        }
    }
    Ok(())
}

impl Created {
    fn begin(root: &Path) -> Result<Created> {
        let mixed = check_export_target(root)? == DeviceLayout::Mixed;
        let dir = journal_dir(root);
        target::confined(root, &dir)?;
        anyhow::ensure!(!dir.join("journal.log").exists(), "An earlier USB export needs recovery before writing");

        std::fs::create_dir_all(&dir)?;
        let log = std::fs::File::create_new(dir.join("journal.log"))?;
        let mut c = Created { root: root.to_path_buf(), dir, log: Some(log), files: vec![], replaced: vec![], publishing: false, committed: false, mixed, doomed: vec![] };
        c.line(if mixed { "B\tmixed" } else { "B" }, true)?;
        Ok(c)
    }
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().to_string()
    }
    fn line(&mut self, l: &str, sync: bool) -> Result<()> {
        use std::io::Write;
        fault_point()?;
        if let Some(f) = self.log.as_mut() {
            writeln!(f, "{l}")?;
            if sync {
                f.sync_all()?;
            }
        }
        fault_point()
    }
    /// A new file this export is about to write.
    fn note_created(&mut self, p: &Path) -> Result<()> {
        target::confined(&self.root, p)?;
        anyhow::ensure!(target::managed_file(&self.root, p) && !p.exists(), "Expected an unused USB filename: {}", p.display());
        self.files.push(p.to_path_buf());
        let l = format!("C\t{}", self.rel(p));
        // Durable before the file exists, so a crash can't leave an unlisted file.
        self.line(&l, true)
    }
    /// About to write `p`: keep a durable copy of what is there now, or note it as new.
    fn will_replace(&mut self, p: &Path) -> Result<()> {
        target::confined(&self.root, p)?;
        target::confined(&self.root, &tmp_for(p))?;
        anyhow::ensure!(target::managed_file(&self.root, p), "Cannot replace an unmanaged USB file: {}", p.display());
        anyhow::ensure!(!self.mixed || p.starts_with(self.root.join(".Recordbox")) || !p.exists(),
            "Existing file {} may be shared with OneLibrary. Choose a classic-only drive to replace it.", p.display());
        if !p.is_file() {
            return self.note_created(p);
        }
        let backup = self.dir.join(format!("r{}", self.replaced.len()));
        target::confined(&self.root, &backup)?;
        anyhow::ensure!(!backup.exists(), "USB journal backup already exists");
        std::fs::copy(p, &backup)?;
        std::fs::File::options().write(true).open(&backup)?.sync_all()?;
        let l = format!("R\t{}\t{}", self.rel(p), backup.file_name().unwrap_or_default().to_string_lossy());
        self.line(&l, true)?;
        self.replaced.push((p.to_path_buf(), backup));
        Ok(())
    }
    /// A file to delete once the new databases are published (a removed track's audio,
    /// analysis and artwork). Listed durably first, so the deletion is finished after a crash.
    fn will_delete(&mut self, p: &Path) -> Result<()> {
        target::confined(&self.root, p)?;
        anyhow::ensure!(!self.mixed && target::managed_file(&self.root, p), "File deletion requires a managed file on a classic-only drive");
        let l = format!("D\t{}", self.rel(p));
        self.doomed.push(p.to_path_buf());
        self.line(&l, true)
    }
    /// Both new databases are on the drive: from here on the export is finished, not undone.
    fn publishing(&mut self) -> Result<()> {
        self.line("P", true)?;
        self.publishing = true;
        Ok(())
    }
    fn finish(&mut self) {
        for f in std::mem::take(&mut self.doomed) {
            remove_file_and_empty_dirs(&self.root, &f);
        }
        self.committed = true;
        self.log = None;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for Created {
    fn drop(&mut self) {
        #[cfg(test)]
        if CRASH.with(|c| c.get()) {
            return;
        }
        if self.committed {
            return;
        }
        self.log = None;
        let _ = recover_device(&self.root);
    }
}

/// Finish or undo an export that did not complete, from the drive's journal. Returns what was
/// done, or None when there was nothing to recover.
pub fn recover_device(root: &Path) -> Option<String> {
    match recover_checked(root) {
        Ok(result) => result.map(str::to_owned),
        Err(e) => Some(format!("USB recovery paused: {e:#}")),
    }
}

fn recover_before_write(root: &Path) -> Result<()> {
    recover_checked(root).map(|_| ())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn recover_checked(root: &Path) -> Result<Option<&'static str>> {
    let layout = check_export_target(root)?;
    migrate_app_dir(root);
    let dir = journal_dir(root);
    let log = dir.join("journal.log");
    target::confined(root, &log)?;
    let metadata = match std::fs::metadata(&log) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(metadata.len() <= 32 * 1024 * 1024, "USB recovery journal is too large");
    let text = std::fs::read_to_string(log)?;
    let mut records = vec![];
    let mut published = false;
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    // An interrupted creation can leave the journal empty before any file is touched.
    anyhow::ensure!(matches!(header, "" | "B" | "B\tmixed"), "Unknown USB recovery journal format");
    for line in lines {
        let parts: Vec<_> = line.split('\t').collect();
        if parts.as_slice() == ["P"] {
            anyhow::ensure!(!published, "Duplicate USB publication marker");
            published = true;
            continue;
        }
        anyhow::ensure!(!published, "Unexpected record after USB publication");
        let (kind, raw, backup) = match parts.as_slice() {
            [kind @ ("C" | "D"), path] => (*kind, *path, None),
            ["R", path, backup] => {
                anyhow::ensure!(backup.strip_prefix('r').is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|d| d.is_ascii_digit())), "Unknown USB journal backup");
                let copy = dir.join(backup);
                target::confined(root, &copy)?;
                anyhow::ensure!(copy.is_file(), "Missing USB recovery backup {}", copy.display());
                ("R", *path, Some(copy))
            }
            _ => anyhow::bail!("Incomplete or unrecognized USB recovery record"),
        };
        let path = root.join(raw.replace('\\', "/"));
        target::confined(root, &path)?;
        target::confined(root, &tmp_for(&path))?;
        anyhow::ensure!(target::managed_file(root, &path), "Unrecognized USB recovery destination");
        anyhow::ensure!(layout != DeviceLayout::Mixed || path.starts_with(root.join(".Recordbox")) || (header == "B\tmixed" && kind == "C"),
            "The interrupted operation touched files which may now be shared with OneLibrary; its journal is retained");
        records.push((kind, path, backup));
    }
    let rb = root.join("PIONEER/rekordbox");
    let catalogs = [rb.join("export.pdb"), rb.join("exportExt.pdb")];
    for path in &catalogs {
        target::confined(root, path)?;
        target::confined(root, &tmp_for(path))?;
    }
    if published {
        for path in &catalogs {
            if tmp_for(path).exists() { publish_tmp(path)?; }
        }
        for (kind, path, _) in records {
            if kind == "D" {
                remove_if_present(&path)?;
                prune_empty_parents(root, &path);
            }
        }
    } else {
        for (kind, path, backup) in records.into_iter().rev() {
            if let Some(backup) = backup {
                write_atomic(&path, &std::fs::read(backup)?)?;
            } else if kind == "C" {
                remove_if_present(&path)?;
                prune_empty_parents(root, &path);
            }
            remove_if_present(&tmp_for(&path))?;
        }
        for path in &catalogs { remove_if_present(&tmp_for(path))?; }
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(Some(if published { "An interrupted export to this device has been completed." }
        else { "An interrupted export to this device has been undone." }))
}

pub(crate) fn sha256_file(p: &Path) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

/// rekordbox's analysis-file numbering: list the folder's files whose name starts with
/// "ANLZ" (any case), parse the rest of the stem as hex (non-hex characters ignored, 32-bit
/// wrap), take the signed maximum (start -1) and add one, formatted with at least 4 digits.
/// See rekordbox-rea reports/usb-export-paths.md.
fn parse_anlz_suffix(stem_after_anlz: &str) -> i32 {
    let mut v: u32 = 0;
    for c in stem_after_anlz.chars() {
        if let Some(d) = c.to_digit(16) {
            v = v.wrapping_mul(16).wrapping_add(d);
        }
    }
    v as i32
}

fn next_anlz_name(dir: &Path, reserved: Option<&i32>) -> String {
    let mut max: i32 = -1;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.get(..4).is_some_and(|prefix| prefix.eq_ignore_ascii_case("ANLZ")) {
                let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let v = parse_anlz_suffix(stem.get(4..).unwrap_or(""));
                max = max.max(v);
            }
        }
    }
    if let Some(r) = reserved {
        max = max.max(*r);
    }
    format!("ANLZ{:04X}", max.wrapping_add(1) as u32)
}

/// "/PIONEER/USBANLZ/P016/0000875E/ANLZ0000.DAT" -> "/PIONEER/USBANLZ/P016/0000875E/ANLZ0000".
pub fn anlz_base(analyze_path: &str) -> Option<String> {
    if !pdb::device_relative_path(analyze_path) { return None; }
    let (base, extension) = analyze_path.rsplit_once('.')?;
    let mut segments = base.strip_prefix('/')?.split('/');
    let pioneer = segments.next()?;
    let directory = segments.next()?;
    (extension.eq_ignore_ascii_case("dat")
        && (pioneer.eq_ignore_ascii_case("PIONEER") || pioneer.eq_ignore_ascii_case(".PIONEER"))
        && directory.eq_ignore_ascii_case("USBANLZ")
        && segments.next().is_some()).then(|| base.to_owned())
}

/// Copy the drive's current databases to /.Recordbox/backup before they are replaced,
/// keeping the five most recent copies.
fn backup_device_db(root: &Path) {
    let src = root.join("PIONEER").join("rekordbox");
    if !src.join("export.pdb").exists() {
        return;
    }
    let dir = root.join(".Recordbox").join("backup");
    if target::confined(root, &dir).is_err() || std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    for f in ["export.pdb", "exportExt.pdb"] {
        if src.join(f).exists() {
            let destination = dir.join(format!("{stamp}-{f}"));
            if target::confined(root, &destination).is_ok() {
                let _ = std::fs::copy(src.join(f), destination);
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut stamps: Vec<String> = rd.flatten().filter_map(|e| e.file_name().to_string_lossy().split('-').take(2).collect::<Vec<_>>().join("-").into()).collect();
        stamps.sort();
        stamps.dedup();
        while stamps.len() > 5 {
            let old = stamps.remove(0);
            for f in ["export.pdb", "exportExt.pdb"] {
                let _ = std::fs::remove_file(dir.join(format!("{old}-{f}")));
            }
        }
    }
}

fn sync_dir(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Copy a media file to the drive via a temporary name, flush it to the device and check
/// that the full length arrived before giving it its final name.
fn copy_durable(src: &Path, dest: &Path, created: &mut Created) -> Result<()> {
    target::confined(&created.root, dest)?;
    anyhow::ensure!(!created.mixed || !dest.exists(), "An existing audio file may be shared with OneLibrary. Choose a classic-only drive to replace it.");
    let tmp = dest.with_file_name(format!("{}.part", dest.file_name().unwrap_or_default().to_string_lossy()));
    created.note_created(&tmp)?;
    let n = std::fs::copy(src, &tmp)?;
    std::fs::File::options().write(true).open(&tmp)?.sync_all()?;
    let want = std::fs::metadata(src)?.len();
    if n != want || std::fs::metadata(&tmp)?.len() != want {
        return Err(anyhow!("copy of {} is incomplete ({n} of {want} bytes)", src.display()));
    }
    // Byte-for-byte check of what reached the drive.
    if sha256_file(src)? != sha256_file(&tmp)? {
        return Err(anyhow!("copy of {} does not match the original; the drive may be failing", src.display()));
    }
    // An existing file (a changed track copied again) is kept in the journal until the export
    // commits; a new one is listed so an undone export removes it.
    created.will_replace(dest)?;
    std::fs::rename(&tmp, dest)?;
    if let Some(p) = dest.parent() {
        sync_dir(p);
    }
    Ok(())
}

/// Native v7 embeddings carry track-specific values. Enumerate once per analysis
/// folder so a case-sensitive mount also sees alternative filename spellings.
fn has_device_embedding(
    root: &Path,
    base: &Path,
    folders: &mut HashMap<PathBuf, std::collections::HashSet<String>>,
) -> Result<bool> {
    let path = base.with_extension("3EX");
    target::confined(root, &path)?;
    let folder = path.parent().context("missing USB analysis folder")?;
    if !folders.contains_key(folder) {
        let mut names = std::collections::HashSet::new();
        match std::fs::read_dir(folder) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let name = entry.file_name();
                    if let Some(name) = name
                        .to_str()
                        .filter(|n| n.to_ascii_lowercase().ends_with(".3ex"))
                    {
                        names.insert(name.to_ascii_lowercase());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        folders.insert(folder.to_owned(), names);
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("invalid USB analysis filename")?;
    Ok(folders[folder].contains(&name.to_ascii_lowercase()))
}

fn embedding_grid_unchanged(
    original: &HashMap<&str, Vec<(String, Vec<u8>)>>,
    files: &anlz::AnlzFiles,
) -> bool {
    fn grid_sections(list: &[(String, Vec<u8>)]) -> impl Iterator<Item = &[u8]> {
        list.iter()
            .filter(|(tag, _)| matches!(tag.as_str(), "PQTZ" | "PQT2"))
            .map(|(_, bytes)| bytes.as_slice())
    }
    for (extension, data) in [
        ("DAT", &files.dat),
        ("EXT", &files.ext),
        ("2EX", &files.ex2),
    ] {
        let old = original
            .get(extension)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if extension == "DAT" && old.iter().filter(|(tag, _)| tag == "PQTZ").count() != 1 {
            return false;
        }
        let proposed = anlz::section_list(data);
        if !grid_sections(old).eq(grid_sections(&proposed)) {
            return false;
        }
    }
    true
}

/// Run an export step so that a panic inside it becomes an error result instead of taking
/// the export thread down silently (which left the export window waiting for ever). The
/// journal's drop handler has already undone the unfinished export by the time this returns.
pub fn run_guarded(f: impl FnOnce() -> Result<String>) -> std::result::Result<String, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(p) => {
            let what = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown error".into());
            Err(format!("The export failed because of an internal error ({what}). The device library was not changed, and files copied by this export were removed."))
        }
    }
}

/// Whether the local file differs from the drive's copy: another size, or modified more than
/// FAT's two-second timestamp step after the copy was written.
fn source_changed(src: &Path, dest: &Path) -> bool {
    let (Ok(a), Ok(b)) = (std::fs::metadata(src), std::fs::metadata(dest)) else { return false };
    if a.len() != b.len() {
        return true;
    }
    match (a.modified(), b.modified()) {
        (Ok(sa), Ok(sb)) => sa > sb + std::time::Duration::from_secs(2),
        _ => false,
    }
}

/// Directory entries a FAT32 folder already uses: each file takes one short entry plus one
/// long-name entry per 13 UTF-16 units of its name; a folder holds 65,536 entries at most.
fn fat_dir_entries(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 2 };
    2 + rd.flatten().map(|e| fat_name_entries(&e.file_name().to_string_lossy())).sum::<usize>()
}

fn fat_name_entries(name: &str) -> usize {
    1 + name.encode_utf16().count().div_ceil(13)
}

pub const FAT_DIR_MAX_ENTRIES: usize = 65_536;

/// Why FAT32 cannot take this file at `usb_path` (None when it can): the 4 GB file size
/// limit, a name or path longer than 255 UTF-16 units, or a full folder.
pub fn fat32_reason(root: &Path, usb_path: &str, size: u64, dir_entries: &mut HashMap<PathBuf, usize>) -> Option<String> {
    if size > u32::MAX as u64 {
        return Some("larger than 4 GB, too big for a FAT32 drive".into());
    }
    let name = usb_path.rsplit('/').next().unwrap_or("");
    if name.encode_utf16().count() > 255 || usb_path.split('/').any(|c| c.encode_utf16().count() > 255) {
        return Some("file or folder name longer than 255 characters, too long for a FAT32 drive".into());
    }
    if usb_path.encode_utf16().count() > 255 {
        return Some("path longer than 255 characters, too long for players reading a FAT32 drive".into());
    }
    let dir = root.join(usb_path.trim_start_matches('/')).parent().map(Path::to_path_buf).unwrap_or_default();
    let used = *dir_entries.entry(dir.clone()).or_insert_with(|| fat_dir_entries(&dir));
    let need = fat_name_entries(name);
    if used + need > FAT_DIR_MAX_ENTRIES {
        return Some("its folder on the drive is full (a FAT32 folder holds 65,536 entries)".into());
    }
    dir_entries.insert(dir, used + need);
    None
}

/// "YYYY-MM-DD" from a date written with `-`, `/` or `.` separators or with a time after it;
/// empty for anything else (a bare year, free text), as real exports hold either a full date
/// or nothing (num_rows: 640 ten-character dates, 3,246 empty).
pub fn normalize_date(s: &str) -> String {
    let s = s.trim();
    let digits: Vec<String> = s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).take(3).map(|p| p.to_string()).collect();
    if digits.len() < 3 {
        // "20240417"
        let d: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        if d.len() == 8 && s.len() == 8 {
            return normalize_date(&format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8]));
        }
        return String::new();
    }
    let (y, m, d) = (digits[0].parse::<u32>().unwrap_or(0), digits[1].parse::<u32>().unwrap_or(0), digits[2].parse::<u32>().unwrap_or(0));
    if digits[0].len() != 4 || !(1..=12).contains(&m) || !(1..=31).contains(&d) || y == 0 {
        return String::new();
    }
    format!("{y:04}-{m:02}-{d:02}")
}

/// The tempo for the track row (BPM × 100): the grid's tempo, or for a grid with several
/// anchors (Dynamic analysis) the average over the whole grid, beats ÷ time, which is what the
/// players show as the track's BPM in lists.
pub fn row_tempo(grid: Option<&BeatGrid>, duration: f64, fallback: f64) -> u32 {
    let bpm = match grid {
        // The model and imported device row have their own track-wide tempo. A
        // finite list can have long gaps that distort a duration-based average.
        Some(g) if (0.01..=655.35).contains(&fallback) && g.has_listed_beats() => fallback,
        Some(g) if g.anchors.len() > 1 => {
            let beats = g.beats(duration.max(g.anchors.last().map(|a| a.time + 1.0).unwrap_or(0.0)));
            match (beats.first(), beats.last()) {
                (Some(a), Some(b)) if beats.len() > 1 && b.time > a.time => (beats.len() - 1) as f64 * 60.0 / (b.time - a.time),
                _ => g.first_bpm(),
            }
        }
        Some(g) => g.first_bpm(),
        None => 0.0,
    };
    let bpm = if bpm > 0.0 { bpm } else { fallback };
    (bpm.max(0.0) * 100.0).round() as u32
}

/// The key text for the device's key table: the key as the library or file spelled it
/// ("7A", "Fm") when the track still has that, else the classic name of the analysed key.
/// Real exports hold the stored spelling, mixed within one drive (num_rows: "Emin", "Fmaj",
/// "2d", "7m", "12m", "Bbm" …); the players' own key display setting does the converting.
pub fn row_key_name(t: &Track) -> String {
    let text = t.key_text.trim();
    if !text.is_empty() {
        return pdb::clip_chars(text, 31).to_string();
    }
    if t.key >= 0 {
        key_name(t.key, false)
    } else {
        String::new()
    }
}

/// A row rekordbox itself wrote for this local file: no recorded source, the same size, and
/// the content path rekordbox's rules give this track (artist/album folders, file name, with
/// or without a "-N" suffix), compared without regard to case or normalisation. Several
/// candidates are narrowed by title and artist.
fn foreign_row(lib: &DeviceLibrary, root: &Path, t: &Track, src: &Path, src_len: u64, taken: &std::collections::HashSet<u32>) -> Option<u32> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    let stem = sanitize(src.file_stem().and_then(|s| s.to_str()).unwrap_or("track"));
    let dir = path_key(&media_dir(&t.artist, &t.album));
    let cands: Vec<&DevTrack> = lib
        .tracks
        .values()
        .filter(|d| d.source.is_empty() && !taken.contains(&d.id) && d.file_size as u64 == src_len.min(u32::MAX as u64))
        .filter(|d| {
            let key = path_key(&d.file_path);
            let Some((ddir, dname)) = key.rsplit_once('/') else { return false };
            ddir == dir && (std::iter::once(None).chain((1..=99).map(Some))).any(|n| path_key(&file_candidate(&stem, ext, n)) == dname)
        })
        .filter(|d| root.join(d.file_path.trim_start_matches('/')).is_file())
        .collect();
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    cands.iter().find(|d| same(&d.title, &t.display_title()) && same(&d.artist, &t.artist)).or_else(|| cands.iter().find(|d| same(&d.title, &t.display_title()))).or_else(|| (cands.len() == 1).then(|| &cands[0])).map(|d| d.id)
}

fn short_list(items: &[String], n: usize) -> String {
    let mut s = items.iter().take(n).cloned().collect::<Vec<_>>().join("; ");
    if items.len() > n {
        s.push_str(&format!("; and {} more", items.len() - n));
    }
    s
}

pub fn run_export(mut req: ExportRequest, progress: &dyn Fn(Progress), cancel: &std::sync::atomic::AtomicBool) -> Result<String> {
    use std::sync::atomic::Ordering::Relaxed;
    use sha2::Digest;
    let root = req.root.clone();
    if !root.is_dir() {
        return Err(anyhow!("The device is not available"));
    }
    let layout = check_export_target(&root)?;
    recover_before_write(&root)?;
    let mut lib = match read_device(&root) {
        Ok(l) => l,
        Err(e) => return Err(anyhow!("Device library is corrupted ({e}). Nothing was changed.")),
    };
    let mut skipped: Vec<String> = std::mem::take(&mut req.skipped);
    // Tracks whose grid is still the one rekordbox analysed can use its phrases/extended grid.
    for et in req.tracks.values_mut() {
        if let (Some(b), Some(g)) = (&et.rb_anlz, &et.grid) {
            et.grid_same = crate::export::anlz::grid_pair::matches_source(&b.with_extension("DAT"), g);
        }
    }

    // Tracks without analysis are analysed first (as the original does when exporting),
    // here on the export thread so the window stays responsive.
    // (Tracks whose file is missing are not analysed: they can only keep a copy already on the drive.)
    let needs: Vec<i64> = req.tracks.values()
        .filter(|t| (t.wave.is_none() || (req.analysis.do_grid && t.grid.is_none())) && Path::new(&t.track.path).is_file())
        .map(|t| t.track.id).collect();
    let mut preparation_failures = HashMap::new();
    let n_needs = needs.len();
    for (i, id) in needs.into_iter().enumerate() {
        if cancel.load(Relaxed) {
            return Err(anyhow!("Export was stopped. The device library was not changed."));
        }
        let Some(et) = req.tracks.get_mut(&id) else { continue };
        progress(Progress::Step { done: i, total: n_needs, text: format!("Analyzing {}", et.track.display_title()) });
        let stopped = || cancel.load(Relaxed);
        let analysis = crate::analysis::analyze_file_bounded(
            &et.track.path, &req.analysis, crate::jobs::analysis_memory_budget(), false, stopped,
        );
        if stopped() { return Err(anyhow!("Export was stopped. The device library was not changed.")); }
        let res = match analysis {
            Ok(res) => res,
            Err(error) => {
                // A partial sync must not delete anything based on an incomplete playlist.
                if !req.remove_nodes.is_empty() && req.delete_unreferenced {
                    return Err(anyhow!("Could not prepare {} ({error}). The device library was not changed.", et.track.display_title()));
                }
                preparation_failures.insert(id, error.to_string());
                continue;
            }
        };
        if et.grid.is_none() { et.grid = res.grid.clone(); }
        if et.wave.is_none() { et.wave = Some(res.wave.clone()); }
        et.track.raw_samples = res.raw_samples;
        et.track.sample_rate = res.sample_rate;
        if et.track.duration <= 0.0 { et.track.duration = res.duration; }
        if et.track.bpm <= 0.0 { et.track.bpm = res.bpm; }
        if et.vocal.is_none() { et.vocal = res.vocal.clone(); }
        progress(Progress::Analyzed(id, Box::new(Ok(res))));
    }

    // Phrases for tracks without rekordbox's own (computed on the track's grid).
    // Phrases edited by the user are written as they are.
    let mut phrases: HashMap<i64, crate::phrase::Structure> = req.tracks.values().filter_map(|t| t.phrases.clone().map(|p| (t.track.id, p))).collect();
    let mut todo = Vec::new();
    if req.phrases {
        for track in req.tracks.values().filter(|t| t.grid.is_some() && t.phrases.is_none() && !preparation_failures.contains_key(&t.track.id)) {
            if cancel.load(Relaxed) { return Err(anyhow!("Export was stopped. The device library was not changed.")); }
            if !track.has_rb_phrases() { todo.push(track.track.id); }
        }
    }
    let n_todo = todo.len();
    let mut inferred_phrases = 0;
    for (i, id) in todo.into_iter().enumerate() {
        if cancel.load(Relaxed) {
            return Err(anyhow!("Export was stopped. The device library was not changed."));
        }
        let Some(et) = req.tracks.get(&id) else { continue };
        progress(Progress::Step { done: i, total: n_todo, text: format!("Analyzing phrases of {}", et.track.display_title()) });
        let stopped = || cancel.load(Relaxed);
        let Some(grid) = &et.grid else { continue };
        // A corrupt anchor can expand to an arbitrarily large beat list. Only
        // expand a finite, ordered grid within this optional inference budget.
        if grid.anchors.len() > crate::phrase::MAX_BEATS || grid.anchors.windows(2).any(|a| a[0].time >= a[1].time)
            || grid.anchors.iter().any(|a| !(0.0..=crate::phrase::MAX_ANALYSIS_SECONDS as f64).contains(&a.time)
                || !(1.0..=1000.0).contains(&a.bpm) || !(1..=4).contains(&a.beat)) {
            continue;
        }
        if let Ok(d) = crate::audio::decode_bounded(&et.track.path, crate::phrase::MAX_ANALYSIS_SECONDS, stopped) {
            let beats = grid.beats(d.duration());
            if let Some(st) = crate::phrase::analyze_audio(&d, &beats, crate::phrase::key_input(&et.track), req.classic_phrase_estimator, &stopped) {
                phrases.insert(id, st);
                inferred_phrases += 1;
            }
        }
    }
    if cancel.load(Relaxed) {
        return Err(anyhow!("Export was stopped. The device library was not changed."));
    }

    // Space check.
    let need: u64 = req
        .tracks
        .values()
        .filter(|t| !preparation_failures.contains_key(&t.track.id) && !lib.tracks.values().any(|d| d.source == t.track.path))
        .map(|t| std::fs::metadata(&t.track.path).map(|m| m.len()).unwrap_or(0))
        .sum();
    if let Some(free) = free_space(&root) {
        if need + 50_000_000 > free {
            return Err(anyhow!("Not enough space available on the device"));
        }
    }
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let total = req.tracks.len();
    let mut created = Created::begin(&root)?;
    let mut cue_record = read_cue_record(&root);
    let mut map: HashMap<i64, u32> = HashMap::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    for t in lib.tracks.values() {
        if !t.source.is_empty() {
            sources.insert(t.file_path.clone(), t.source.clone());
        }
    }
    // Analysis numbers handed out during this export, per folder, so two new tracks that
    // share a folder get different numbers before either file exists.
    let mut reserved: HashMap<String, i32> = HashMap::new();
    let mut embedding_folders = HashMap::new();
    let mut duplicate_analysis_paths = None;
    let mut next_id = lib.tracks.keys().max().copied().unwrap_or(0) + 1;
    let fat32 = req.fs == "msdos";
    let decomposed = fs_decomposes(&req.fs);
    let mut dir_entries: HashMap<PathBuf, usize> = HashMap::new();
    let mut artwork_record = read_artwork_record(&root);
    let mut report = ExportReport::default();
    // Device rows already matched to a track of this export (so two local files can't both
    // claim one row rekordbox wrote).
    let mut taken: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut ordered: Vec<&ExportTrack> = req.tracks.values().collect();
    ordered.sort_by_key(|t| t.track.id);
    for (i, et) in ordered.iter().enumerate() {
        if cancel.load(Relaxed) {
            return Err(anyhow!("Export was stopped. The device library was not changed, and files copied by this export were removed."));
        }
        let t = &et.track;
        progress(Progress::Step { done: i, total, text: format!("Transferring {} ({} remaining)", t.display_title(), total - i) });
        let src = Path::new(&t.path);
        if !src.exists() {
            // Missing here, but the drive may still hold its copy: keep that in the playlists
            // (the drive's row, file and analysis stay as they are).
            let on_drive = lib.tracks.values().find(|d| d.source == t.path && root.join(d.file_path.trim_start_matches('/')).exists()).map(|d| d.id);
            match on_drive {
                Some(dev_id) => {
                    map.insert(t.id, dev_id);
                }
                None => skipped.push(format!("{} (file not found)", t.display_title())),
            }
            continue;
        }
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
        if !player_supported(&ext) {
            skipped.push(format!("{} (file type not supported by the players)", t.display_title()));
            continue;
        }
        let src_len = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
        // FAT32 cannot hold a file of 4 GB or more; exFAT and HFS+ can.
        if fat32 && src_len > u32::MAX as u64 {
            skipped.push(format!("{} (larger than 4 GB, too big for a FAT32 drive)", t.display_title()));
            continue;
        }
        // Already on the device?
        // A row whose file is still there wins over one whose file went missing.
        let on_drive = |d: &&DevTrack| root.join(d.file_path.trim_start_matches('/')).exists();
        let existing = lib
            .tracks
            .values()
            .filter(|d| d.source == t.path && !taken.contains(&d.id))
            .find(on_drive)
            .or_else(|| lib.tracks.values().find(|d| d.source == t.path && !taken.contains(&d.id)))
            .map(|d| (d.id, d.file_path.clone()))
            // A track rekordbox itself exported to this drive (no Recordbox record of it): the
            // row with this track's content path and file size is updated in place.
            .or_else(|| foreign_row(&lib, &root, t, src, src_len, &taken).map(|id| (id, lib.tracks[&id].file_path.clone())));
        if let Some(reason) = preparation_failures.get(&t.id) {
            // A failed refresh must not replace the device's sound, grid or cue files.
            // Keep a proven existing copy in the selected playlists.
            let kept = if let Some((id, path)) = &existing {
                let dest = root.join(path.trim_start_matches('/'));
                target::confined(&root, &dest)?;
                if dest.is_file() {
                    map.insert(t.id, *id);
                    taken.insert(*id);
                    true
                } else { false }
            } else { false };
            let prior = if kept { "; previous device copy kept" } else { "" };
            skipped.push(format!("{} (analysis failed: {reason}{prior})", t.display_title()));
            continue;
        }
        let mut protects_embedding = false;
        if let Some((id, path)) = &existing {
            let current = &lib.tracks[id];
            if let Some(base) = anlz_base(&current.analyze_path) {
                protects_embedding = has_device_embedding(
                    &root, &root.join(base.trim_start_matches('/')), &mut embedding_folders,
                )?;
            }
            if protects_embedding {
                let dest = root.join(path.trim_start_matches('/'));
                target::confined(&root, &dest)?;
                anyhow::ensure!(
                    dest.is_file() && std::fs::metadata(&dest)?.len() == src_len
                        && sha256_file(src)? == sha256_file(&dest)?,
                    "Track {id} has an existing USB .3EX embedding. Its source and device audio must match before updating it."
                );
                let duplicates = duplicate_analysis_paths.get_or_insert_with(|| {
                    let mut seen = std::collections::HashSet::new();
                    let mut duplicate = std::collections::HashSet::new();
                    for track in lib.tracks.values().filter(|t| !t.analyze_path.is_empty()) {
                        let key = path_key(&track.analyze_path);
                        if !seen.insert(key.clone()) {
                            duplicate.insert(key);
                        }
                    }
                    duplicate
                });
                anyhow::ensure!(
                    !duplicates.contains(&path_key(&current.analyze_path)),
                    "Track {id} shares its USB .3EX embedding with a second device track."
                );
            }
        }
        let (dev_id, usb_path) = match existing {
            Some((id, p)) if root.join(p.trim_start_matches('/')).exists() => {
                let dest = root.join(p.trim_start_matches('/'));
                target::confined(&root, &dest)?;
                // An embedded track already had its complete audio checked above.
                let recopy = !protects_embedding && req.recopy_changed && source_changed(src, &dest);
                // FAT timestamps are coarse. A newer local copy with identical bytes
                // can reuse the shared file even when OneLibrary is also on the drive.
                let same_shared_file = recopy && layout == DeviceLayout::Mixed
                    && std::fs::metadata(&dest)?.len() == src_len
                    && sha256_file(src)? == sha256_file(&dest)?;
                if recopy && !same_shared_file {
                    // The local file changed since it was copied: copy it again to the same place.
                    copy_durable(src, &dest, &mut created)?;
                    report.copied += 1;
                } else {
                    report.updated += 1;
                }
                (id, p)
            }
            Some((id, p)) if !p.is_empty() => {
                // The file went missing from the drive: copy it back to the same place, keeping
                // the track's ID, playlists and analysis path.
                let dest = root.join(p.trim_start_matches('/'));
                target::confined(&root, &dest)?;
                anyhow::ensure!(target::managed_file(&root, &dest), "Cannot create audio outside the existing managed USB layout");
                std::fs::create_dir_all(dest.parent().unwrap())?;
                copy_durable(src, &dest, &mut created)?;
                report.copied += 1;
                (id, p)
            }
            _ => {
                let dir = media_dir_fs(&t.artist, &t.album, decomposed);
                let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
                let stem = sanitize(stem);
                let mut suffix = None;
                let mut n = 0;
                let usb = loop {
                    let name = file_candidate(&stem, &ext, suffix);
                    let cand = format!("{dir}/{}", if decomposed { sanitize_fs(&name, true) } else { name });
                    if !root.join(cand.trim_start_matches('/')).exists() {
                        break cand;
                    }
                    n += 1;
                    suffix = Some(n);
                };
                if fat32 {
                    if let Some(why) = fat32_reason(&root, &usb, src_len, &mut dir_entries) {
                        skipped.push(format!("{} ({why})", t.display_title()));
                        continue;
                    }
                }
                let dest = root.join(usb.trim_start_matches('/'));
                target::confined(&root, &dest)?;
                anyhow::ensure!(target::managed_file(&root, &dest), "Cannot create audio outside the existing managed USB layout");
                std::fs::create_dir_all(dest.parent().unwrap())?;
                copy_durable(src, &dest, &mut created)?;
                report.copied += 1;
                let id = next_id;
                next_id += 1;
                (id, usb)
            }
        };
        taken.insert(dev_id);
        sources.insert(usb_path.clone(), t.path.clone());
        // Analysis files: a track already on the device keeps its analysis path; a new one
        // gets the next free number in its hashed folder.
        let full_len = if t.raw_samples > 0 && t.sample_rate > 0 { t.raw_samples as f64 / t.sample_rate as f64 } else { t.duration };
        let beats = et.grid.as_ref().map(|g| g.beats(full_len.max(t.duration))).unwrap_or_default();
        let wave = et.wave.clone().unwrap_or_default();
        let mut sections = std::collections::HashMap::new();
        if let Some(b) = &et.rb_anlz {
            for e in ["2EX", "EXT", "DAT"] {
                sections.extend(anlz::read_sections(&b.with_extension(e)));
            }
        }
        // The user's own phrases (PHRASE EDIT) are written, not rekordbox's for this file.
        if et.phrases.is_some() {
            sections.remove("PSSI");
        }
        let prev = lib.tracks.get(&dev_id).cloned().unwrap_or_default();
        let mut device = std::collections::HashMap::new();
        if let Some(b) = anlz_base(&prev.analyze_path) {
            let b = root.join(b.trim_start_matches('/'));
            for e in ["DAT", "EXT", "2EX"] {
                if let Some(bytes) = anlz::device_companion(&b.with_extension(e), &prev.file_path)? {
                    device.insert(e, anlz::section_list(&bytes));
                }
            }
        }
        // Without "including cue points" a track that is already on the drive keeps its cues.
        let device_cues: Option<Vec<Cue>> = (req.keep_device_cues && lib.tracks.contains_key(&dev_id)).then(|| anlz::parse_cues(device.get("EXT").map(Vec::as_slice).unwrap_or(&[]), device.get("DAT").map(Vec::as_slice).unwrap_or(&[]))).filter(|_| device.values().any(|l| !l.is_empty()));
        let cues: &[Cue] = device_cues.as_deref().unwrap_or(&et.cues);
        let ov = anlz::Overrides { sections, grid_same: et.grid_same, pssi: phrases.get(&t.id).map(crate::phrase::pssi), device, pvdi: et.vocal.as_deref().map(anlz::pvdi) };
        let files = anlz::build_with(&usb_path, &beats, &wave, cues, t.raw_samples, t.sample_rate, Some(&ov));
        let bpm100 = row_tempo(et.grid.as_ref(), full_len.max(t.duration), t.bpm);
        if protects_embedding {
            anyhow::ensure!(
                prev.bpm100 == bpm100 && embedding_grid_unchanged(&ov.device, &files),
                "Track {dev_id} has an existing USB .3EX embedding. The native BPM or beat-grid timing would change; re-analyse it in Rekordbox."
            );
        }
        let base = match anlz_base(&prev.analyze_path) {
            Some(b) => b,
            None => {
                let folder = anlz::anlz_folder(&usb_path);
                let dir = root.join(folder.trim_start_matches('/'));
                let name = next_anlz_name(&dir, reserved.get(&folder));
                reserved.insert(folder.clone(), parse_anlz_suffix(&name[4..]));
                format!("{folder}/{name}")
            }
        };
        let base_fs = root.join(base.trim_start_matches('/'));
        target::confined(&root, &base_fs)?;
        anyhow::ensure!(target::managed_file(&root, &base_fs), "Cannot create analysis outside the existing managed USB layout");
        std::fs::create_dir_all(base_fs.parent().unwrap())?;
        for (ext_name, data) in [("DAT", &files.dat), ("EXT", &files.ext), ("2EX", &files.ex2)] {
            let path = base_fs.with_extension(ext_name);
            target::confined(&root, &path)?;
            if layout == DeviceLayout::Mixed && path.is_file() && sha256_file(&path)? == <[u8;32]>::from(sha2::Sha256::digest(data)) { continue; }
            created.will_replace(&path)?;
            write_atomic(&path, data)?;
        }
        // A desktop .3EX belongs to its native track identity. No verified USB
        // writer or cross-library ID mapping exists for that embedding.
        cue_record.insert(usb_path.clone(), anlz::cue_fingerprint(&anlz::parse_cues(&anlz::section_list(&files.ext), &anlz::section_list(&files.dat))));
        // Artwork: one file per distinct picture. Tracks showing the same picture (an album)
        // share one artwork row, as real exports do (num_rows: 2,178 artwork rows for 3,886
        // tracks); the picture is recognised by the hash of the local artwork file, recorded
        // per drive in /.Recordbox/artwork.json.
        let artwork_path = if !t.artwork.is_empty() && Path::new(&t.artwork).exists() {
            let hash = sha256_file(Path::new(&t.artwork)).map(hex).unwrap_or_default();
            let on_drive = |rel: &str| root.join(rel.trim_start_matches('/')).is_file();
            let known = artwork_record.get(&hash).filter(|rel| !hash.is_empty() && on_drive(rel)).cloned();
            match known {
                Some(rel) => rel,
                None => {
                    // This track's own file is reused only when no other track shows it, else
                    // the next number is taken.
                    let shared = |p: &str| lib.tracks.values().any(|o| o.id != dev_id && o.artwork_path == p);
                    let own = layout != DeviceLayout::Mixed && !prev.artwork_path.is_empty() && prev.artwork_path.starts_with("/PIONEER/Artwork/") && !shared(&prev.artwork_path);
                    let rel = if own { prev.artwork_path.clone() } else { free_artwork_name(&root, &lib) };
                    let written = write_artwork(&root, &rel, &t.artwork, &mut created).unwrap_or_default();
                    if !written.is_empty() && !hash.is_empty() {
                        artwork_record.retain(|_, v| *v != written);
                        artwork_record.insert(hash, written.clone());
                    }
                    written
                }
            }
        } else {
            String::new()
        };
        let row_duration = if t.raw_samples > 0 && t.sample_rate > 0 && t.duration <= 0.0 { full_len } else { t.duration };
        let dt = DevTrack {
            id: dev_id,
            title: t.display_title(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_artist: t.album_artist.clone(),
            genre: t.genre.clone(),
            label: t.label.clone(),
            remixer: t.remixer.clone(),
            original_artist: t.original_artist.clone(),
            composer: t.composer.clone(),
            key_name: row_key_name(t),
            comment: t.comment.clone(),
            mix_name: t.mix_name.clone(),
            message: t.message.clone(),
            lyricist: t.lyricist.clone(),
            year: t.year.clamp(0, 9999) as u16,
            track_no: t.track_no.max(0) as u32,
            disc_no: t.disc_no.max(0) as u16,
            // Whole seconds, cut not rounded (see MECHANICS.md: the two real exports with audio
            // lengths, 172.43 s -> 172 and 128.05 s -> 128, fit both; rekordbox's length is
            // the tag library's whole-second length, which truncates).
            duration: (row_duration + 1e-6).floor().clamp(0.0, 65535.0) as u16,
            bpm100,
            rating: t.rating.min(5),
            color: t.color.min(8),
            bitrate: t.bitrate,
            sample_rate: t.sample_rate,
            sample_depth: if t.bit_depth > 0 { t.bit_depth as u16 } else { 16 },
            file_size: std::fs::metadata(root.join(usb_path.trim_start_matches('/'))).map(|m| m.len()).unwrap_or(src_len).min(u32::MAX as u64) as u32,
            file_type: file_type_code(&ext),
            file_path: usb_path.clone(),
            analyze_path: format!("{base}.DAT"),
            date_added: Some(normalize_date(&t.date_added)).filter(|d| !d.is_empty()).unwrap_or_else(|| today.clone()),
            release_date: normalize_date(&t.release_date),
            analyze_date: today.clone(),
            artwork_path: if artwork_path.is_empty() { prev.artwork_path.clone() } else { artwork_path },
            play_count: prev.play_count,
            autoload: t.autoload_hotcue,
            source: t.path.clone(),
            tags: et.tags.iter().map(|(c, x)| tag_key(c, x)).collect(),
            raw_head: prev.raw_head.clone(),
            // Slot 2 counts exports/changes of the row ("1" on a new row, higher on rows
            // exported again: num_rows holds 1 … 6+): one more for a row that was on the drive.
            raw_strings: bump_export_count(prev.raw_strings.clone()),
        };
        lib.tracks.insert(dev_id, dt);
        map.insert(t.id, dev_id);
    }

    // Playlists: merge by path of names (same name under same parent is replaced).
    progress(Progress::Step { done: total, total, text: "Write to the device".into() });
    let mut node_ids = read_node_record(&root);
    let mut claimed: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for n in &req.nodes {
        merge_node(&mut lib, 0, n, &map, &mut node_ids, &mut claimed);
    }
    // Synchronisation: lists that left the library's selection go, with their tracks when
    // asked and no remaining playlist uses them.
    let mut doomed_files: Vec<PathBuf> = vec![];
    if !req.remove_nodes.is_empty() {
        let before = lib.nodes.len();
        let orphaned = drop_nodes(&mut lib, &req.remove_nodes);
        report.removed_lists = before - lib.nodes.len();
        if req.delete_unreferenced {
            let gone: Vec<u32> = orphaned.into_iter().filter(|t| !lib.nodes.iter().any(|n| n.entries.contains(t))).collect();
            report.removed_tracks = gone.iter().filter(|t| lib.tracks.contains_key(t)).count();
            doomed_files = remove_tracks(&mut lib, &root, &gone);
        }
    }
    node_ids.retain(|_, dev| lib.nodes.iter().any(|n| n.id == *dev));
    if let Some(name) = &req.device_name {
        lib.device_name = pdb::clip_chars(name.trim(), 63).to_string();
    }
    if let Some(names) = &req.color_names {
        if names.len() == 8 {
            lib.color_names = names.iter().map(|n| pdb::clip_chars(n.trim(), 63).to_string()).collect();
        }
    }
    if lib.nodes.len() > 1000 {
        return Err(anyhow!("The number of lists and folders cannot exceed 1000 on one USB device."));
    }
    // My Tag categories: the app's current configuration.
    if !req.tag_categories.is_empty() {
        lib.tag_categories = req.tag_categories.clone();
    }
    if cancel.load(Relaxed) {
        return Err(anyhow!("Export was stopped. The device library was not changed, and files copied by this export were removed."));
    }
    lib.browse = Some(req.browse.clone());
    // Sidecar with local sources and the cue record (Recordbox's own; never read by players).
    // Written first so a finished-after-interruption export has them too; entries for files
    // an undone export removed are simply never looked up.
    std::fs::create_dir_all(root.join(".Recordbox"))?;
    // Journaled, so an undone export also puts these back.
    created.will_replace(&sidecar_path(&root))?;
    // Sorted, so the same content always gives the same bytes.
    write_atomic(&sidecar_path(&root), &serde_json::to_vec_pretty(&sources.iter().collect::<BTreeMap<_, _>>())?)?;
    cue_record.retain(|path, _| lib.tracks.values().any(|t| &t.file_path == path));
    created.will_replace(&cue_record_path(&root))?;
    write_atomic(&cue_record_path(&root), &serde_json::to_vec_pretty(&cue_record.iter().collect::<BTreeMap<_, _>>())?)?;
    artwork_record.retain(|_, rel| lib.tracks.values().any(|t| &t.artwork_path == rel));
    created.will_replace(&artwork_record_path(&root))?;
    write_atomic(&artwork_record_path(&root), &serde_json::to_vec_pretty(&artwork_record)?)?;
    created.will_replace(&node_record_path(&root))?;
    write_atomic(&node_record_path(&root), &serde_json::to_vec_pretty(&node_ids.iter().map(|(k, v)| (k.to_string(), *v)).collect::<BTreeMap<String, u32>>())?)?;
    // Player settings, as rekordbox writes them with every export: part of the journaled
    // export, and a failure to write them fails it.
    std::fs::create_dir_all(root.join("PIONEER"))?;
    if layout != DeviceLayout::Mixed {
        for (name, bytes) in req.settings.merged_onto_drive(&root.join("PIONEER")).files() {
            let p = root.join("PIONEER").join(name);
            created.will_replace(&p)?;
            write_atomic(&p, &bytes).map_err(|e| anyhow!("writing the player settings file {name}: {e}"))?;
        }
    }
    backup_device_db(&root);
    for f in &doomed_files {
        created.will_delete(f)?;
    }
    write_device_journaled(&root, &lib, &mut created)?;
    created.finish();

    report.skipped = skipped.clone();
    report.tracks_on_device = lib.tracks.len();
    report.playlists_on_device = lib.nodes.iter().filter(|n| !n.folder).count();
    progress(Progress::Report(report));
    let mut msg = format!("Export completed: {} tracks, {} playlists on the device", lib.tracks.len(), lib.nodes.iter().filter(|n| !n.folder).count());
    if inferred_phrases < n_todo {
        msg.push_str(&format!(". Generated phrases for {inferred_phrases} of {n_todo} tracks that needed them"));
    }
    if layout == DeviceLayout::Mixed {
        msg.push_str(". Note: this drive also has OneLibrary (Device Library Plus). These edits apply to its classic device catalog. Recordbox preserved existing shared files and player settings.");
    }
    if !skipped.is_empty() {
        msg.push_str(&format!(". {} track(s) were not exported: {}", skipped.len(), short_list(&skipped, 5)));
    }
    Ok(msg)
}

fn merge_node(lib: &mut DeviceLibrary, parent: u32, n: &ExportNode, map: &HashMap<i64, u32>, node_ids: &mut HashMap<i64, u32>, claimed: &mut std::collections::HashSet<u32>) {
    // The device node this library playlist became last time (same parent, same kind), else a
    // node of the same name that no other playlist of this export or of the record owns. Two
    // playlists with one name in one folder therefore stay two device playlists.
    let free = |x: &DevNode, claimed: &std::collections::HashSet<u32>| x.parent == parent && x.folder == n.folder && !claimed.contains(&x.id);
    let by_identity = (n.id != 0).then(|| node_ids.get(&n.id).copied()).flatten().filter(|d| lib.nodes.iter().any(|x| x.id == *d && free(x, claimed)));
    let owned_by_other = |x: &DevNode| n.id != 0 && node_ids.iter().any(|(k, v)| *v == x.id && *k != n.id);
    let existing = by_identity.or_else(|| lib.nodes.iter().find(|x| free(x, claimed) && x.name == n.name && !owned_by_other(x)).map(|x| x.id));
    let id = match existing {
        Some(id) => id,
        None => {
            let id = lib.nodes.iter().map(|x| x.id).max().unwrap_or(0) + 1;
            let seq = lib.nodes.iter().filter(|x| x.parent == parent).count() as u32;
            lib.nodes.push(DevNode { id, parent, name: n.name.clone(), folder: n.folder, seq, entries: vec![] });
            id
        }
    };
    claimed.insert(id);
    if n.id != 0 {
        node_ids.insert(n.id, id);
    }
    if let Some(node) = lib.nodes.iter_mut().find(|x| x.id == id) {
        // A playlist renamed in the library keeps its device copy, under the new name.
        node.name = n.name.clone();
        if !n.folder {
            node.entries = n.tracks.iter().filter_map(|t| map.get(t).copied()).collect();
        }
    }
    for c in &n.children {
        merge_node(lib, id, c, map, node_ids, claimed);
    }
}

fn node_record_path(root: &Path) -> PathBuf {
    root.join(".Recordbox").join("nodes.json")
}

fn artwork_record_path(root: &Path) -> PathBuf {
    root.join(".Recordbox").join("artwork.json")
}

/// Which device node each library playlist/folder became (library ID -> device node ID).
pub fn read_node_record(root: &Path) -> HashMap<i64, u32> {
    let m: HashMap<String, u32> = std::fs::read_to_string(node_record_path(root)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    m.into_iter().filter_map(|(k, v)| k.parse::<i64>().ok().map(|k| (k, v))).collect()
}

/// Artwork files Recordbox wrote to this drive, by SHA-256 of the local picture.
fn read_artwork_record(root: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(artwork_record_path(root)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub(crate) fn hex(b: [u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// One more in the row's export counter (string slot 2) for a row that was read from a drive.
fn bump_export_count(mut raw: Vec<String>) -> Vec<String> {
    if let Some(s) = raw.get_mut(2) {
        if let Ok(n) = s.trim().parse::<u32>() {
            *s = (n + 1).min(999_999).to_string();
        }
    }
    raw
}

/// Remove nodes and everything under them; returns the tracks their playlists held.
fn drop_nodes(lib: &mut DeviceLibrary, ids: &[u32]) -> Vec<u32> {
    let mut doomed: Vec<u32> = ids.to_vec();
    let mut i = 0;
    while i < doomed.len() {
        let p = doomed[i];
        let kids: Vec<u32> = lib.nodes.iter().filter(|n| n.parent == p && !doomed.contains(&n.id)).map(|n| n.id).collect();
        doomed.extend(kids);
        i += 1;
    }
    let mut tracks: Vec<u32> = lib.nodes.iter().filter(|n| doomed.contains(&n.id)).flat_map(|n| n.entries.clone()).collect();
    tracks.sort_unstable();
    tracks.dedup();
    lib.nodes.retain(|n| !doomed.contains(&n.id));
    // Sequence numbers of the remaining siblings close up.
    let parents: std::collections::BTreeSet<u32> = lib.nodes.iter().map(|n| n.parent).collect();
    for p in parents {
        let mut sibs: Vec<(u32, u32)> = lib.nodes.iter().filter(|n| n.parent == p).map(|n| (n.seq, n.id)).collect();
        sibs.sort();
        for (k, (_, id)) in sibs.into_iter().enumerate() {
            if let Some(n) = lib.nodes.iter_mut().find(|n| n.id == id) {
                n.seq = k as u32;
            }
        }
    }
    tracks
}

/// Take tracks out of the device library altogether: their rows, their places in playlists
/// and in play histories (rekordbox removes history memberships and renumbers the rest:
/// rekordbox-rea reports/usb-single-track-removal.md). Returns the files to delete once the
/// new databases are published: audio, analysis files, and artwork no remaining track shows.
/// Nothing is listed for a drive that also has a OneLibrary catalogue, which Recordbox can't
/// read and which may still use the files.
pub fn remove_tracks(lib: &mut DeviceLibrary, root: &Path, ids: &[u32]) -> Vec<PathBuf> {
    let mut gone: Vec<PathBuf> = vec![];
    let mut art: Vec<String> = vec![];
    for id in ids {
        let Some(dt) = lib.tracks.remove(id) else { continue };
        if !dt.file_path.is_empty() {
            gone.push(root.join(dt.file_path.trim_start_matches('/')));
        }
        if let Some(base) = anlz_base(&dt.analyze_path) {
            let b = root.join(base.trim_start_matches('/'));
            for e in ANLZ_EXTS {
                gone.push(b.with_extension(e));
            }
        }
        if !dt.artwork_path.is_empty() {
            art.push(dt.artwork_path);
        }
    }
    for n in lib.nodes.iter_mut() {
        n.entries.retain(|t| !ids.contains(t));
    }
    for h in lib.histories.iter_mut() {
        h.entries.retain(|t| !ids.contains(t));
    }
    art.sort();
    art.dedup();
    for a in art {
        if !lib.tracks.values().any(|t| t.artwork_path == a) && (a.starts_with("/PIONEER/Artwork/") || a.starts_with("/.PIONEER/Artwork/")) {
            let p = root.join(a.trim_start_matches('/'));
            if let (Some(stem), Some(ext)) = (p.file_stem().map(|x| x.to_string_lossy().to_string()), p.extension().map(|x| x.to_string_lossy().to_string())) {
                gone.push(p.with_file_name(format!("{stem}_m.{ext}")));
            }
            gone.push(p);
        }
    }
    if !matches!(DeviceLayout::inspect(root), Ok(DeviceLayout::Classic)) {
        return vec![];
    }
    let used = referenced_files(root, lib);
    gone.retain(|p| target::managed_file(root, p) && !used.contains(&asset_key(&p)));
    gone
}

/// Analysis companions belonging to a device track, including native v7 embeddings.
pub const ANLZ_EXTS: [&str; 4] = ["DAT", "EXT", "2EX", "3EX"];

/// Change the device library in one journaled step: `change` edits the library read from the
/// drive and returns the files to delete afterwards. The databases are published first and
/// the files deleted only then; the deletions are listed in the journal ("D" lines), so one
/// cut short by a crash or a pulled drive is finished the next time the drive is seen, and an
/// interruption before publication changes nothing.
pub fn change_device(root: &Path, change: impl FnOnce(&mut DeviceLibrary) -> Result<Vec<PathBuf>>) -> Result<DeviceLibrary> {
    change_device_with(root, |lib| Ok((change(lib)?, vec![])))
}

/// [`change_device`] that also writes files (player settings) inside the same journal:
/// `change` returns (files to delete afterwards, files to write as (path, bytes)). The files
/// are written before the databases are published and restored if the change is undone.
pub fn change_device_with(root: &Path, change: impl FnOnce(&mut DeviceLibrary) -> Result<(Vec<PathBuf>, Vec<(PathBuf, Vec<u8>)>)>) -> Result<DeviceLibrary> {
    if !root.is_dir() {
        return Err(anyhow!("The device is not available"));
    }
    let layout = check_export_target(root)?;
    recover_before_write(root)?;
    let mut lib = read_device(root).map_err(|e| anyhow!("Device library is corrupted ({e}). Nothing was changed."))?;
    let (gone, write) = change(&mut lib)?;
    anyhow::ensure!(layout != DeviceLayout::Mixed || (gone.is_empty() && write.is_empty()),
        "This drive has both catalogs. Classic catalog edits are supported; shared files and player settings are protected.");
    let mut created = Created::begin(root)?;
    backup_device_db(root);
    for (p, bytes) in &write {
        target::confined(root, p)?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        created.will_replace(p)?;
        write_atomic(p, bytes)?;
    }
    for f in &gone {
        created.will_delete(f)?;
    }
    write_device_journaled(root, &lib, &mut created)?;
    created.finish();
    Ok(lib)
}

/// Remove a playlist/folder from the device library (and optionally its tracks that no other
/// playlist uses, with their audio, analysis and artwork files and history entries).
pub fn remove_node(root: &Path, node_id: u32, delete_tracks: bool) -> Result<()> {
    change_device(root, |lib| {
        let removed = drop_nodes(lib, &[node_id]);
        if !delete_tracks {
            return Ok(vec![]);
        }
        let gone: Vec<u32> = removed.into_iter().filter(|t| !lib.nodes.iter().any(|n| n.entries.contains(t))).collect();
        Ok(remove_tracks(lib, root, &gone))
    })
    .map(|_| ())
}

/// Delete tracks from the device: rows, playlist and history entries, and their files.
pub fn delete_tracks(root: &Path, ids: &[u32]) -> Result<usize> {
    let mut n = 0;
    change_device(root, |lib| {
        n = ids.iter().filter(|i| lib.tracks.contains_key(i)).count();
        Ok(remove_tracks(lib, root, ids))
    })?;
    Ok(n)
}

/// Remove a play history (table 11) and its entries (table 12) from the device.
pub fn delete_history(root: &Path, history_id: u32) -> Result<()> {
    change_device(root, |lib| {
        if !lib.histories.iter().any(|h| h.id == history_id) {
            return Err(anyhow!("That history is no longer on the device."));
        }
        lib.histories.retain(|h| h.id != history_id);
        Ok(vec![])
    })
    .map(|_| ())
}

/// Set the device label rekordbox keeps in its synchronisation history row.
pub fn set_device_name(root: &Path, name: &str) -> Result<()> {
    change_device(root, |lib| {
        lib.device_name = pdb::clip_chars(name.trim(), 63).to_string();
        Ok(vec![])
    })
    .map(|_| ())
}

/// Write via a temporary file that is flushed to the device before it replaces `path`.
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    write_tmp(path, data)?;
    publish_tmp(path)
}

fn tmp_for(path: &Path) -> PathBuf {
    path.with_file_name(format!("{}.tmp_write", path.file_name().unwrap_or_default().to_string_lossy()))
}

fn write_tmp(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(tmp_for(path))?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

fn publish_tmp(path: &Path) -> Result<()> {
    std::fs::rename(tmp_for(path), path)?;
    if let Some(p) = path.parent() {
        sync_dir(p);
    }
    Ok(())
}

/// Folder of artwork number `n`: 20 numbers per folder, `n / 20 + 1` in five digits, as real
/// exports number them (num_rows: /PIONEER/Artwork/00001 holds a1–a19, 00002 a20–a39, …,
/// 00111 a2200–a2210).
pub fn artwork_rel(n: u32) -> String {
    format!("/PIONEER/Artwork/{:05}/a{n}.jpg", n / 20 + 1)
}

/// The number in an artwork path ("/PIONEER/Artwork/00002/a21.jpg" -> 21).
fn artwork_number(rel: &str) -> Option<u32> {
    let name = rel.rsplit('/').next()?;
    name.strip_prefix('a')?.strip_suffix(".jpg")?.parse().ok()
}

/// The next artwork file name: one above the highest number the library uses (numbers are
/// not reused in real exports), skipping any file already on the drive.
fn free_artwork_name(root: &Path, lib: &DeviceLibrary) -> String {
    let top = lib.tracks.values().filter_map(|t| artwork_number(&t.artwork_path)).max().unwrap_or(0);
    (top + 1..)
        .map(artwork_rel)
        .find(|rel| {
            let p = root.join(rel.trim_start_matches('/'));
            let m = p.with_file_name(format!("{}_m.jpg", p.file_stem().unwrap_or_default().to_string_lossy()));
            !p.exists() && !m.exists()
        })
        .unwrap()
}

/// Write a track's artwork (80 px and the 240 px "_m" variant) at `rel`.
fn write_artwork(root: &Path, rel: &str, src: &str, created: &mut Created) -> Result<String> {
    let img = image::open(src)?;
    let small_p = root.join(rel.trim_start_matches('/'));
    let stem = small_p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
    let big_p = small_p.with_file_name(format!("{stem}_m.jpg"));
    target::confined(root, &small_p)?;
    std::fs::create_dir_all(small_p.parent().unwrap())?;
    created.will_replace(&small_p)?;
    created.will_replace(&big_p)?;
    let small = img.resize_to_fill(80, 80, image::imageops::FilterType::Triangle).to_rgb8();
    small.save(&small_p)?;
    let big = img.resize_to_fill(240, 240, image::imageops::FilterType::Triangle).to_rgb8();
    big.save(&big_p)?;
    Ok(rel.to_string())
}

pub fn free_space(root: &Path) -> Option<u64> {
    crate::disk_space::on(root).ok().map(|s| s.available)
}

pub fn total_space(root: &Path) -> Option<u64> {
    crate::disk_space::on(root).ok().map(|s| s.capacity)
}

// ------------------------------------------------------------------------------------------
// Database writing
// ------------------------------------------------------------------------------------------

struct Interner {
    ids: HashMap<String, u32>,
    order: Vec<String>,
}
impl Interner {
    fn new() -> Self {
        Interner { ids: HashMap::new(), order: vec![] }
    }
    fn id(&mut self, s: &str) -> u32 {
        if s.trim().is_empty() {
            return 0;
        }
        if let Some(i) = self.ids.get(s) {
            return *i;
        }
        self.order.push(s.to_string());
        let id = self.order.len() as u32;
        self.ids.insert(s.to_string(), id);
        id
    }
}

/// Static rows every export carries (browse categories and menus), as observed in real exports.
const COLUMNS: [(u16, u16, &str); 27] = [
    (1, 0x80, "GENRE"),
    (2, 0x81, "ARTIST"),
    (3, 0x82, "ALBUM"),
    (4, 0x83, "TRACK"),
    (5, 0x85, "BPM"),
    (6, 0x86, "RATING"),
    (7, 0x87, "YEAR"),
    (8, 0x88, "REMIXER"),
    (9, 0x89, "LABEL"),
    (10, 0x8a, "ORIGINAL ARTIST"),
    (11, 0x8b, "KEY"),
    (12, 0x8d, "CUE"),
    (13, 0x8e, "COLOR"),
    (14, 0x92, "TIME"),
    (15, 0x93, "BITRATE"),
    (16, 0x94, "FILE NAME"),
    (17, 0x84, "PLAYLIST"),
    (18, 0x98, "HOT CUE BANK"),
    (19, 0x95, "HISTORY"),
    (20, 0x91, "SEARCH"),
    (21, 0x96, "COMMENTS"),
    (22, 0x8c, "DATE ADDED"),
    (23, 0x97, "DJ PLAY COUNT"),
    (24, 0x90, "FOLDER"),
    (25, 0xa1, "DEFAULT"),
    (26, 0xa2, "ALPHABET"),
    (27, 0xaa, "MATCHING"),
];
const MENU: [[u8; 8]; 22] = [
    [1, 0, 1, 0, 0x63, 1, 0, 0],
    [5, 0, 6, 0, 5, 1, 0, 0],
    [6, 0, 7, 0, 0x63, 1, 0, 0],
    [7, 0, 8, 0, 0x63, 1, 0, 0],
    [8, 0, 9, 0, 0x63, 1, 0, 0],
    [9, 0, 10, 0, 0x63, 1, 0, 0],
    [10, 0, 11, 0, 0x63, 1, 0, 0],
    [13, 0, 15, 0, 0x63, 1, 0, 0],
    [14, 0, 19, 0, 4, 1, 0, 0],
    [15, 0, 20, 0, 6, 1, 0, 0],
    [16, 0, 21, 0, 0x63, 1, 0, 0],
    [18, 0, 23, 0, 0x63, 1, 0, 0],
    [2, 0, 2, 0, 2, 0, 1, 0],
    [3, 0, 3, 0, 3, 0, 2, 0],
    [4, 0, 4, 0, 1, 0, 3, 0],
    [11, 0, 12, 0, 0x63, 0, 4, 0],
    [17, 0, 5, 0, 0x63, 0, 5, 0],
    [19, 0, 22, 0, 0x63, 0, 6, 0],
    [20, 0, 18, 0, 0x63, 0, 7, 0],
    [27, 0, 26, 0, 0x63, 2, 8, 0],
    [24, 0, 17, 0, 0x63, 0, 9, 0],
    [22, 0, 27, 0, 0x63, 0, 10, 0],
];
const TABLE18: [[u8; 8]; 17] = [
    [1, 0, 6, 0, 1, 0, 0, 0],
    [0x15, 0, 7, 0, 1, 0, 0, 0],
    [0x0e, 0, 8, 0, 1, 0, 0, 0],
    [8, 0, 9, 0, 1, 0, 0, 0],
    [9, 0, 10, 0, 1, 0, 0, 0],
    [10, 0, 11, 0, 1, 0, 0, 0],
    [0x0f, 0, 13, 0, 1, 0, 0, 0],
    [13, 0, 15, 0, 1, 0, 0, 0],
    [0x17, 0, 16, 0, 1, 0, 0, 0],
    [0x16, 0, 17, 0, 1, 0, 0, 0],
    [0x19, 0, 0, 0, 0, 1, 0, 0],
    [0x1a, 0, 1, 0, 0, 2, 0, 0],
    [2, 0, 2, 0, 0, 3, 0, 0],
    [3, 0, 3, 0, 0, 4, 0, 0],
    [5, 0, 4, 0, 0, 5, 0, 0],
    [6, 0, 5, 0, 0, 6, 0, 0],
    [11, 0, 12, 0, 0, 7, 0, 0],
];

fn simple_row(id: u32, name: &str) -> Vec<u8> {
    let mut r = id.to_le_bytes().to_vec();
    r.extend(dstr(name));
    r
}

/// Menu (17) and sort (18) rows: the original's rows with visibility and position taken from
/// the Category/Sort preferences. Shared with OneLibrary's category and sort tables.
pub(crate) fn browse_tables(lib: &DeviceLibrary) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    // Menu (17) and sort (18) tables: the original's rows with visibility and position taken
    // from the Category/Sort preferences (visible rows keep their observed visibility value).
    let browse = lib.browse.clone().map(|b| b.normalized());
    let menu_rows: Vec<Vec<u8>> = MENU
        .iter()
        .map(|m| {
            let mut r = m.to_vec();
            if let Some(b) = &browse {
                let id = u16::from_le_bytes([m[0], m[1]]);
                match b.categories.iter().position(|x| *x == id) {
                    Some(i) => {
                        r[5] = if m[5] == 1 { 0 } else { m[5] };
                        r[6..8].copy_from_slice(&(i as u16 + 1).to_le_bytes());
                    }
                    None => {
                        r[5] = 1;
                        r[6..8].copy_from_slice(&0u16.to_le_bytes());
                    }
                }
            }
            r
        })
        .collect();
    let t18_rows: Vec<Vec<u8>> = TABLE18
        .iter()
        .map(|m| {
            let mut r = m.to_vec();
            if let Some(b) = &browse {
                let id = u16::from_le_bytes([m[0], m[1]]);
                match b.sorts.iter().position(|x| *x == id) {
                    Some(i) => {
                        r[4] = 0;
                        r[5] = i as u8 + 1;
                    }
                    None => {
                        r[4] = 1;
                        r[5] = 0;
                    }
                }
            }
            r
        })
        .collect();
    (menu_rows, t18_rows)
}

/// The browse columns (id, kind, name) every export carries.
pub(crate) fn browse_columns() -> &'static [(u16, u16, &'static str)] {
    &COLUMNS
}

/// The synchronisation-history row rekordbox adds to table 19 at every export: subtype
/// 0x0280, the row's index shift, the number of tracks, a date, 0x1e19, the version "1000" and
/// the device label. rekordbox allots it 40 bytes (48 with an 11-character label).
fn history_row(tracks: usize, label: &str, index_in_page: usize) -> Vec<u8> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut r = vec![0x80, 0x02];
    r.extend(((index_in_page as u16).wrapping_mul(0x20)).to_le_bytes());
    r.extend((tracks as u32).to_le_bytes());
    r.extend(0u32.to_le_bytes());
    r.extend(dstr(&today));
    r.extend(0x1e19u16.to_le_bytes());
    r.extend(dstr("1000"));
    r.extend(dstr(pdb::clip_chars(label, 63)));
    let len = ((r.len() + 7) & !7).max(40);
    r.resize(len, 0);
    r
}

/// Table 19 for a rewrite: the drive's pages with every earlier row flagged deleted and one
/// new row appended to the last page (a new page once that is full), which is what real
/// drives show after several exports (rekordcrate demo_tracks: 12 retired rows and the
/// present one). A drive without history gets a single row.
fn history_table(lib: &DeviceLibrary) -> TableSpec {
    let mut pages = lib.raw_tables.get(&19).cloned().unwrap_or_default();
    if pages.is_empty() {
        return TableSpec { page_type: 19, rows: vec![history_row(lib.tracks.len(), &lib.device_name, 0)], ..Default::default() };
    }
    let count = pages.len();
    for pg in pages.iter_mut().take(count - 1) {
        pdb::retire_rows(pg);
    }
    let last = pages.last_mut().unwrap();
    let n = pdb::page_rows(last).len();
    if !pdb::append_row(last, &history_row(lib.tracks.len(), &lib.device_name, n), true) {
        pdb::retire_rows(last);
        let mut pg = pdb::blank_page(19);
        pdb::append_row(&mut pg, &history_row(lib.tracks.len(), &lib.device_name, 0), false);
        pages.push(pg);
    }
    TableSpec { page_type: 19, raw_pages: pages, ..Default::default() }
}

/// Protect player paths exactly. Shorten free-text fields when the row would exceed its page.
fn track_row(fixed: &[u8], texts: &[String]) -> Result<Vec<u8>> {
    let mut limits = [pdb::MAX_STRING_CHARS; 21];
    loop {
        let strings: Vec<Vec<u8>> = texts.iter().enumerate().map(|(i, t)| match i {
            0 => Ok(pdb::dstr_isrc(t)),
            14 | 19 | 20 => pdb::dstr_path(t),
            _ => Ok(dstr(pdb::clip_chars(t, limits[i]))),
        }).collect::<Result<_>>()?;
        let row = row_with_strings(fixed, &strings, true);
        if row.len() <= pdb::MAX_ROW_BYTES {
            return Ok(row);
        }
        // Never the paths (14 analysis, 19 file name, 20 file path): players open files by them.
        let Some(longest) = (1..21).filter(|i| ![14, 19, 20].contains(i)).max_by_key(|&i| texts[i].chars().count().min(limits[i])) else { anyhow::bail!("Track paths exceed a classic catalog page") };
        let now = texts[longest].chars().count().min(limits[longest]);
        if now == 0 {
            anyhow::bail!("Track paths exceed a classic catalog page");
        }
        limits[longest] = now / 2;
    }
}

pub fn build_export_pdb(lib: &DeviceLibrary) -> Vec<u8> {
    try_build_export_pdb(lib).expect("device database rows fit their pages")
}

pub fn try_build_export_pdb(lib: &DeviceLibrary) -> Result<Vec<u8>> {
    let mut artists = Interner::new();
    let mut genres = Interner::new();
    let mut labels = Interner::new();
    let mut keys = Interner::new();
    let mut artwork = Interner::new();
    let mut albums: HashMap<(String, u32), u32> = HashMap::new();
    let mut album_rows: Vec<(u32, u32, String)> = vec![];
    let mut track_rows = vec![];
    let db_u3: u16 = 0xfa80;
    let db_u4: u16 = 0x05e7;
    for t in lib.tracks.values() {
        let artist = artists.id(&t.artist);
        let composer = artists.id(&t.composer);
        let orig = artists.id(&t.original_artist);
        let remixer = artists.id(&t.remixer);
        let aa = if t.album_artist.trim().is_empty() { artist } else { artists.id(&t.album_artist) };
        let album = if t.album.trim().is_empty() {
            0
        } else {
            let key = (t.album.clone(), aa);
            let n = albums.len() as u32 + 1;
            *albums.entry(key).or_insert_with(|| {
                album_rows.push((n, aa, t.album.clone()));
                n
            })
        };
        let genre = genres.id(&t.genre);
        let label = labels.id(&t.label);
        let key = keys.id(&t.key_name);
        let art = artwork.id(&t.artwork_path);
        let mut f = Vec::with_capacity(0x5c);
        f.extend_from_slice(&0x24u16.to_le_bytes());
        f.extend_from_slice(&0u16.to_le_bytes());
        f.extend_from_slice(&0x000c_0700u32.to_le_bytes());
        f.extend_from_slice(&t.sample_rate.to_le_bytes());
        f.extend_from_slice(&composer.to_le_bytes());
        f.extend_from_slice(&t.file_size.to_le_bytes());
        // unknown2: observed effectively random; use a stable hash of the path.
        let mut h: u32 = 2166136261;
        for b in t.file_path.bytes() {
            h = (h ^ b as u32).wrapping_mul(16777619);
        }
        f.extend_from_slice(&h.to_le_bytes());
        f.extend_from_slice(&db_u3.to_le_bytes());
        f.extend_from_slice(&db_u4.to_le_bytes());
        for v in [art, key, orig, label, remixer, t.bitrate, t.track_no, t.bpm100, genre, album, artist, t.id] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        for v in [t.disc_no, t.play_count, t.year, t.sample_depth, t.duration, 0x29u16] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.push(t.color);
        f.push(t.rating);
        f.extend_from_slice(&t.file_type.to_le_bytes());
        debug_assert_eq!(f.len(), 0x5c);
        // A row read from the drive keeps the bytes Recordbox does not manage: the row header
        // bitmask (0x04..0x08; 0x02 is the positional index shift), the 0x14 value, the 0x18/0x1a values and the 0x56 constant.
        if t.raw_head.len() == 0x5c {
            for range in [0x04..0x08, 0x14..0x1c, 0x56..0x58] {
                f[range.clone()].copy_from_slice(&t.raw_head[range]);
            }
        }
        let fname = Path::new(&t.file_path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        // String slots Recordbox does not manage keep the drive's value (ISRC, slots 2–4, 6,
        // 8, 9, 13, 18).
        let keep = |i: usize, default: &str| -> String { t.raw_strings.get(i).cloned().unwrap_or_else(|| default.to_string()) };
        // Defaults of a new row, from the newest real exports (rekordcrate num_rows, 2024,
        // 3,886 rows; fragmede 2024): slots 2 and 3 are counters that start at "1" (3,008 and
        // 3,885 rows hold "1") and slot 6 ("publish track information") is "ON" in every row.
        // The 2022 demo export's "3"/"3"/"" was a drive exported three times by an older version.
        let (s0, s2, s3, s4, s6, s8, s9, s13, s18) = (keep(0, ""), keep(2, "1"), keep(3, "1"), keep(4, ""), keep(6, "ON"), keep(8, ""), keep(9, ""), keep(13, ""), keep(18, ""));
        // Slot 0 is the ISRC and is written in its own string form.
        let texts: Vec<String> = vec![
            s0,
            t.lyricist.clone(),
            s2,
            s3,
            s4,
            t.message.clone(),
            s6,
            (if t.autoload { "ON" } else { "" }).to_string(),
            s8,
            s9,
            t.date_added.clone(),
            t.release_date.clone(),
            t.mix_name.clone(),
            s13,
            t.analyze_path.clone(),
            t.analyze_date.clone(),
            t.comment.clone(),
            t.title.clone(),
            s18,
            fname,
            t.file_path.clone(),
        ];
        track_rows.push(track_row(&f, &texts)?);
    }
    let artist_rows: Vec<Vec<u8>> = artists
        .order
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut f = vec![0x60, 0x00, 0, 0];
            f.extend_from_slice(&(i as u32 + 1).to_le_bytes());
            row_with_strings(&f, &[dstr(n)], false)
        })
        .collect();
    let album_rows: Vec<Vec<u8>> = album_rows
        .iter()
        .map(|(id, aa, name)| {
            let mut f = vec![0x80, 0x00, 0, 0];
            f.extend_from_slice(&0u32.to_le_bytes());
            f.extend_from_slice(&aa.to_le_bytes());
            f.extend_from_slice(&id.to_le_bytes());
            f.extend_from_slice(&0u32.to_le_bytes());
            row_with_strings(&f, &[dstr(name)], false)
        })
        .collect();
    let genre_rows = genres.order.iter().enumerate().map(|(i, n)| simple_row(i as u32 + 1, n)).collect();
    let label_rows = labels.order.iter().enumerate().map(|(i, n)| simple_row(i as u32 + 1, n)).collect();
    let key_rows = keys
        .order
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut r = (i as u32 + 1).to_le_bytes().to_vec();
            r.extend((i as u32 + 1).to_le_bytes());
            r.extend(dstr(n));
            r
        })
        .collect();
    // Colour names: the drive's (or the user's, set by the export) when there are eight.
    let color_names: Vec<String> = if lib.color_names.len() == 8 { lib.color_names.clone() } else { COLOR_NAMES[1..].iter().map(|s| s.to_string()).collect() };
    let color_rows = color_names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut r = vec![0u8; 4];
            r.push(i as u8 + 1);
            r.push(i as u8 + 1);
            r.extend([0u8, 0]);
            r.extend(dstr(n));
            r
        })
        .collect();
    let artwork_rows = artwork.order.iter().enumerate().map(|(i, name)| {
        let mut row = (i as u32 + 1).to_le_bytes().to_vec();
        row.extend(pdb::dstr_path(name)?);
        Ok(row)
    }).collect::<Result<_>>()?;
    let tree_rows = lib
        .nodes
        .iter()
        .map(|n| {
            let mut r = vec![];
            r.extend(n.parent.to_le_bytes());
            r.extend(0u32.to_le_bytes());
            r.extend(n.seq.to_le_bytes());
            r.extend(n.id.to_le_bytes());
            r.extend((n.folder as u32).to_le_bytes());
            r.extend(dstr(&n.name));
            r
        })
        .collect();
    let mut entry_rows = vec![];
    for n in &lib.nodes {
        for (i, t) in n.entries.iter().enumerate() {
            let mut r = vec![];
            r.extend((i as u32 + 1).to_le_bytes());
            r.extend(t.to_le_bytes());
            r.extend(n.id.to_le_bytes());
            entry_rows.push(r);
        }
    }
    let hist_rows = lib.histories.iter().map(|h| simple_row(h.id, &h.name)).collect();
    let mut hent_rows = vec![];
    for h in &lib.histories {
        for (i, t) in h.entries.iter().enumerate() {
            let mut r = vec![];
            r.extend(t.to_le_bytes());
            r.extend(h.id.to_le_bytes());
            r.extend((i as u32 + 1).to_le_bytes());
            hent_rows.push(r);
        }
    }
    let col_rows = COLUMNS
        .iter()
        .map(|(id, u, name)| {
            let mut r = vec![];
            r.extend(id.to_le_bytes());
            r.extend(u.to_le_bytes());
            r.extend(dstr_ucs2(&format!("\u{fffa}{name}\u{fffb}")));
            r
        })
        .collect();
    let (menu_rows, t18_rows) = browse_tables(lib);

    let t = |page_type: u32, rows: Vec<Vec<u8>>| TableSpec { page_type, rows, ..Default::default() };
    // Tables Recordbox does not manage (9, 10, 14, 15): the drive's pages, unchanged.
    let kept = |page_type: u32| TableSpec { page_type, raw_pages: lib.raw_tables.get(&page_type).cloned().unwrap_or_default(), ..Default::default() };
    let tables = vec![
        TableSpec { page_type: 0, rows: track_rows, index_entries: true, index_shift: true, ..Default::default() },
        t(1, genre_rows),
        TableSpec { page_type: 2, rows: artist_rows, index_shift: true, ..Default::default() },
        TableSpec { page_type: 3, rows: album_rows, index_shift: true, ..Default::default() },
        t(4, label_rows),
        t(5, key_rows),
        t(6, color_rows),
        t(7, tree_rows),
        t(8, entry_rows),
        kept(9),
        kept(10),
        t(11, hist_rows),
        t(12, hent_rows),
        t(13, artwork_rows),
        kept(14),
        kept(15),
        if lib.raw_tables.contains_key(&16) { kept(16) } else { t(16, col_rows) },
        t(17, menu_rows),
        t(18, t18_rows),
        history_table(lib),
    ];
    pdb::write_database(&tables, 5, 20)
}

/// Longest My Tag (or category) name written: the tag row addresses its two strings with
/// 8-bit offsets, the second one after the name, so the name must end before byte 255 (32 +
/// 4 + 2 × 100 UTF-16 bytes = 236). rekordbox's own limit for tag names is not established.
pub const MAX_TAG_CHARS: usize = 100;

pub fn build_export_ext(lib: &DeviceLibrary) -> Vec<u8> {
    try_build_export_ext(lib).expect("device database rows fit their pages")
}

pub fn try_build_export_ext(lib: &DeviceLibrary) -> Result<Vec<u8>> {
    let mut tag_rows = vec![];
    let tag_row = |parent: u32, pos: u32, id: u32, cat: bool, name: &str| -> Vec<u8> {
        let name = pdb::clip_chars(name, MAX_TAG_CHARS);
        let mut f = vec![0x80, 0x06, 0, 0];
        f.extend(0u32.to_le_bytes());
        f.extend(0u32.to_le_bytes());
        f.extend(parent.to_le_bytes());
        f.extend(pos.to_le_bytes());
        f.extend(id.to_le_bytes());
        f.extend(if cat { [0u8, 0, 0, 1] } else { [0u8; 4] });
        row_with_strings(&f, &[dstr(name), dstr("")], false)
    };
    // Keep the IDs the drive already uses; new categories/tags get the next free ID.
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut next = lib.tag_ids.values().max().copied().unwrap_or(0) + 1;
    let mut id_for = |key: String| -> u32 {
        if let Some(i) = ids.get(&key) {
            return *i;
        }
        let i = match lib.tag_ids.get(&key) {
            Some(i) => *i,
            None => {
                let i = next;
                next += 1;
                i
            }
        };
        ids.insert(key, i);
        i
    };
    let mut tag_ids: HashMap<String, u32> = HashMap::new();
    for (ci, (cat, tags)) in lib.tag_categories.iter().enumerate() {
        let cid = id_for(cat.clone());
        tag_rows.push(tag_row(0, ci as u32, cid, true, cat));
        for (ti, tg) in tags.iter().enumerate() {
            let key = tag_key(cat, tg);
            let id = id_for(key.clone());
            tag_ids.insert(key, id);
            tag_rows.push(tag_row(cid, ti as u32, id, false, tg));
        }
    }
    let mut tt_rows = vec![];
    for t in lib.tracks.values() {
        for tg in &t.tags {
            // Older sidecars stored bare tag names; match those by name in any category.
            let id = tag_ids.get(tg).or_else(|| tag_ids.iter().find(|(k, _)| k.rsplit('\u{1f}').next() == Some(tg.as_str())).map(|(_, v)| v));
            if let Some(id) = id {
                let mut r = vec![];
                r.extend(0u32.to_le_bytes());
                r.extend(t.id.to_le_bytes());
                r.extend(id.to_le_bytes());
                r.extend(3u32.to_le_bytes());
                tt_rows.push(r);
            }
        }
    }
    let t7 = {
        let mut r = vec![0u8, 7, 0, 0];
        r.extend([0u8; 20]);
        r.extend([0x81, 0xfa, 0xe7, 0x05, 0x03, 0x22, 0x23, 0x24, 0x25, 0x26, 3, 3, 3, 3, 3]);
        r.resize(60, 0);
        r
    };
    let t = |page_type: u32, rows: Vec<Vec<u8>>| TableSpec { page_type, rows, ..Default::default() };
    let kept = |page_type: u32| TableSpec { page_type, raw_pages: lib.raw_ext_tables.get(&page_type).cloned().unwrap_or_default(), ..Default::default() };
    let tables = vec![
        kept(0),
        kept(1),
        kept(2),
        TableSpec { page_type: 3, rows: tag_rows, index_shift: true, ..Default::default() },
        t(4, tt_rows),
        kept(5),
        kept(6),
        if lib.raw_ext_tables.contains_key(&7) { kept(7) } else { t(7, vec![t7]) },
        kept(8),
    ];
    pdb::write_database(&tables, 5, 6)
}


/// Write both databases under the export's journal: fully written and flushed, then "P",
/// then the two renames.
fn write_device_journaled(root: &Path, lib: &DeviceLibrary, created: &mut Created) -> Result<()> {
    let dir = root.join("PIONEER").join("rekordbox");
    for name in ["export.pdb", "exportExt.pdb"] {
        target::confined(root, &dir.join(name))?;
        target::confined(root, &tmp_for(&dir.join(name)))?;
    }
    std::fs::create_dir_all(&dir)?;
    let main = try_build_export_pdb(lib)?;
    let check = parse_pdb(main.clone())?;
    if check.tracks.len() != lib.tracks.len() {
        return Err(anyhow!("internal check failed: wrote {} tracks, read back {}", lib.tracks.len(), check.tracks.len()));
    }
    anyhow::ensure!(lib.tracks.iter().all(|(id, t)| check.tracks.get(id).is_some_and(|saved| saved.file_path == t.file_path && saved.analyze_path == t.analyze_path && saved.artwork_path == t.artwork_path)),
        "USB track paths could not be represented exactly in the classic catalog");
    let ext = try_build_export_ext(lib)?;
    let mut checked = check;
    read_ext_tags(ext.clone(), &mut checked).context("invalid generated extended catalog")?;
    write_tmp(&dir.join("export.pdb"), &main)?;
    write_tmp(&dir.join("exportExt.pdb"), &ext)?;
    created.publishing()?;
    publish_tmp(&dir.join("export.pdb"))?;
    fault_point()?;
    publish_tmp(&dir.join("exportExt.pdb"))?;
    fault_point()
}

#[cfg(test)]
mod tests {
    /// Rewriting a rekordbox-made export.pdb keeps every track's unmanaged bytes and strings.
    #[test]
    fn rewrite_keeps_unmanaged_track_fields() {
        let p = dirs::home_dir().unwrap().join("Developer/rekordbox-rea/analysis/format-oracles/Holzhaus__rekordcrate/data/complete_export/demo_tracks/PIONEER/rekordbox/export.pdb");
        let Ok(bytes) = std::fs::read(&p) else { return };
        let a = parse_pdb(bytes).unwrap();
        assert!(!a.tracks.is_empty());
        let b = parse_pdb(build_export_pdb(&a)).unwrap();
        assert_eq!(a.tracks.len(), b.tracks.len());
        for (id, ta) in &a.tracks {
            let tb = &b.tracks[id];
            assert_eq!(ta.raw_strings, tb.raw_strings, "strings of track {id}");
            for range in [0x04..0x08, 0x14..0x1c, 0x56..0x58] {
                assert_eq!(ta.raw_head[range.clone()], tb.raw_head[range.clone()], "header bytes {range:?} of track {id}");
            }
            let ids = [0x0c, 0x1c, 0x20, 0x24, 0x28, 0x2c, 0x3c, 0x40, 0x44, 0x48];
            let diff: Vec<usize> = (0..0x5c).filter(|&i| i != 2 && i != 3 && !ids.iter().any(|&o| i >= o && i < o + 4) && ta.raw_head[i] != tb.raw_head[i]).collect();
            assert!(diff.is_empty(), "track {id}: header bytes {diff:x?} changed");
            assert_eq!((ta.title.clone(), ta.artist.clone(), ta.album.clone(), ta.genre.clone(), ta.bpm100, ta.key_name.clone()), (tb.title.clone(), tb.artist.clone(), tb.album.clone(), tb.genre.clone(), tb.bpm100, tb.key_name.clone()));
        }
    }

    /// The fixed tables Recordbox writes (colours, columns, menu, sort, table 19) match a real
    /// rekordbox export row for row, byte for byte (rekordcrate's demo_tracks export).
    #[test]
    fn fixed_tables_match_a_real_rekordbox_export() {
        let p = dirs::home_dir().unwrap().join("Developer/rekordbox-rea/analysis/format-oracles/Holzhaus__rekordcrate/data/complete_export/demo_tracks/PIONEER/rekordbox/export.pdb");
        let Ok(bytes) = std::fs::read(&p) else { return };
        let real = pdb::Database::parse(bytes.clone()).unwrap();
        let ours = pdb::Database::parse(build_export_pdb(&parse_pdb(bytes).unwrap())).unwrap();
        for t in [6u32, 16, 17, 18, 19] {
            let a: Vec<Vec<u8>> = real.rows(t).into_iter().map(|(h, o)| real.row_bytes(h, o).to_vec()).collect();
            let b: Vec<Vec<u8>> = ours.rows(t).into_iter().map(|(h, o)| ours.row_bytes(h, o).to_vec()).collect();
            assert_eq!(a.len(), b.len(), "table {t} row count");
            for (i, (x, y)) in a.iter().zip(&b).enumerate() {
                // Rows are compared over the shorter length (row_bytes runs to the heap's end).
                let mut n = x.len().min(y.len());
                if t == 19 {
                    // The row itself (the rest of row_bytes is page padding and footer).
                    n = n.min(32);
                }
                let (mut x, mut y) = (x[..n].to_vec(), y[..n].to_vec());
                if t == 19 {
                    // The export date (a short string at 12..23) is the day of the export, and
                    // the u16 at 2..4 varies between real exports with no rule found yet (0,
                    // 0x160, 0x20 for 0, 2 and 3,886 tracks).
                    for v in [&mut x, &mut y] {
                        v[12..23].fill(0);
                        v[2..4].fill(0);
                    }
                }
                let diff: Vec<usize> = (0..n).filter(|&k| x[k] != y[k]).collect();
                assert!(diff.is_empty(), "table {t} row {i}: bytes {diff:?} differ (rekordbox {:?}, recordbox {:?})", diff.iter().map(|&k| x[k]).collect::<Vec<_>>(), diff.iter().map(|&k| y[k]).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn default_browse_matches_original_tables() {
        let d = UsbBrowse::default();
        // Category page defaults: ARTIST, ALBUM, TRACK, KEY, PLAYLIST, HISTORY, SEARCH,
        // MATCHING, FOLDER, DATE ADDED.
        assert_eq!(d.categories, vec![2, 3, 4, 11, 17, 19, 20, 27, 24, 22]);
        // Sort page defaults: DEFAULT, ALPHABET, ARTIST, ALBUM, BPM, RATING, KEY.
        assert_eq!(d.sorts, vec![0x19, 0x1a, 2, 3, 5, 6, 11]);
        // Writing the defaults reproduces the original rows byte for byte.
        let mut lib = DeviceLibrary::default();
        let plain = build_export_pdb(&lib);
        lib.browse = Some(d);
        assert_eq!(build_export_pdb(&lib), plain);
    }

    #[test]
    fn anlz_numbering_matches_original() {
        let base = std::env::temp_dir().join(format!("recordbox_anlz_num_{}", std::process::id()));
        let case = |files: &[&str]| -> String {
            let d = base.join(files.join("_").replace('.', "-"));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            for f in files {
                std::fs::write(d.join(f), b"x").unwrap();
            }
            next_anlz_name(&d, None)
        };
        assert_eq!(case(&[]), "ANLZ0000");
        assert_eq!(case(&["ANLZ0000.DAT", "ANLZ0000.EXT"]), "ANLZ0001");
        assert_eq!(case(&["ANLZFFFFFFFF.DAT"]), "ANLZ0000");
        assert_eq!(case(&["ANLZ7FFFFFFF.DAT"]), "ANLZ80000000");
        assert_eq!(case(&["anlz0003.anything"]), "ANLZ0004");
        assert_eq!(case(&["ANLZGGGG.DAT"]), "ANLZ0001");
        // A number reserved earlier in the same export counts even before its file exists.
        let d = base.join("reserved");
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(next_anlz_name(&d, Some(&0)), "ANLZ0001");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(anlz_base("/PIONEER/USBANLZ/P016/0000875E/ANLZ0003.DAT").as_deref(), Some("/PIONEER/USBANLZ/P016/0000875E/ANLZ0003"));
        assert_eq!(anlz_base("/Contents/x.DAT"), None);
    }

    use super::*;
    #[test]
    fn media_paths_follow_rules() {
        assert_eq!(media_dir("", ""), "/Contents/UnknownArtist/UnknownAlbum");
        assert_eq!(media_dir("AC/DC", "Back: In ~ Black?"), "/Contents/AC_DC/Back_ In _ Black_");
        let f = file_candidate(&"x".repeat(80), "mp3", None);
        assert_eq!(f.chars().count(), 48);
        assert!(file_candidate("song", "mp3", Some(2)).ends_with("song-2.mp3"));
    }

    #[test]
    fn device_roundtrip() {
        let mut lib = DeviceLibrary::default();
        for i in 1..=60u32 {
            lib.tracks.insert(
                i,
                DevTrack {
                    id: i,
                    title: format!("Track {i} – ünïcode"),
                    artist: format!("Artist {}", i % 7),
                    album: format!("Album {}", i % 5),
                    genre: "House".into(),
                    key_name: "Am".into(),
                    bpm100: 12800,
                    file_path: format!("/Contents/A/B/t{i}.mp3"),
                    analyze_path: "/PIONEER/USBANLZ/P000/00000000/ANLZ0000.DAT".into(),
                    file_type: 1,
                    rating: 3,
                    tags: vec![tag_key("Genre", "Techno")],
                    ..Default::default()
                },
            );
        }
        lib.nodes.push(DevNode { id: 1, parent: 0, name: "Folder".into(), folder: true, seq: 0, entries: vec![] });
        lib.nodes.push(DevNode { id: 2, parent: 1, name: "Set".into(), folder: false, seq: 0, entries: vec![3, 1, 2] });
        lib.tag_categories = vec![("Genre".into(), vec!["Techno".into()])];
        let bytes = build_export_pdb(&lib);
        let _ = std::fs::write(std::env::temp_dir().join("recordbox_sample_export.pdb"), &bytes);
        let _ = std::fs::write(std::env::temp_dir().join("recordbox_sample_exportExt.pdb"), build_export_ext(&lib));
        let back = parse_pdb(bytes).unwrap();
        assert_eq!(back.tracks.len(), 60);
        assert_eq!(back.tracks[&5].title, "Track 5 – ünïcode");
        assert_eq!(back.tracks[&5].artist, "Artist 5");
        assert_eq!(back.tracks[&5].album, "Album 0");
        assert_eq!(back.tracks[&5].key_name, "Am");
        assert_eq!(back.nodes.len(), 2);
        assert_eq!(back.nodes.iter().find(|n| n.id == 2).unwrap().entries, vec![3, 1, 2]);
        let ext = build_export_ext(&lib);
        let mut l2 = back.clone();
        read_ext_tags(ext, &mut l2).unwrap();
        assert_eq!(l2.tag_categories[0].1, vec!["Techno".to_string()]);
        assert_eq!(l2.tracks[&1].tags, vec![tag_key("Genre", "Techno")]);
    }

    /// A cue moved on a player (the drive's analysis file rewritten) is found; an untouched one is not.
    #[test]
    fn finds_cues_changed_on_players() {
        let root = std::env::temp_dir().join(format!("recordbox_player_edits_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = "/PIONEER/USBANLZ/P000/00000001/ANLZ0000";
        let dir = root.join(base.trim_start_matches('/')).parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let w = Waveform { frames: vec![[10; 8]; 50] };
        let cue = |start: f64| Cue { id: 0, track_id: 0, kind: CueKind::Hot, slot: 0, start, end: None, color: 0, comment: String::new(), active_loop: false };
        let write = |cues: &[Cue]| {
            let f = anlz::build("/Contents/a.mp3", &[], &w, cues, 0, 44100);
            let b = root.join(base.trim_start_matches('/'));
            std::fs::write(b.with_extension("DAT"), &f.dat).unwrap();
            std::fs::write(b.with_extension("EXT"), &f.ext).unwrap();
            anlz::cue_fingerprint(&anlz::parse_cues(&anlz::section_list(&f.ext), &anlz::section_list(&f.dat)))
        };
        let fp = write(&[cue(1.0)]);
        let mut lib = DeviceLibrary::default();
        lib.tracks.insert(1, DevTrack { id: 1, title: "A".into(), file_path: "/Contents/a.mp3".into(), analyze_path: format!("{base}.DAT"), ..Default::default() });
        write_cue_record(&root, &HashMap::from([("/Contents/a.mp3".to_string(), fp.clone())])).unwrap();
        // As if the export had happened a minute ago.
        let backdate = |p: &Path| std::fs::File::options().write(true).open(p).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60)).unwrap();
        backdate(&cue_record_path(&root));
        assert!(player_cue_edits(&root, &lib).is_empty());
        assert_eq!(fp, anlz::local_cue_fingerprint(&[cue(1.0)]));
        write(&[cue(2.0)]);
        let edits = player_cue_edits(&root, &lib);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].exported, fp);
        assert_eq!(edits[0].cues[0].start, 2.0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finds_and_deletes_only_unused_files() {
        let root = std::env::temp_dir().join(format!("recordbox_unused_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
            p
        };
        mk("Contents/A/kept.mp3");
        let orphan = mk("Contents/B/old.mp3");
        mk("PIONEER/USBANLZ/P001/0000AAAA/ANLZ0000.DAT");
        mk("PIONEER/USBANLZ/P001/0000AAAA/ANLZ0000.EXT");
        let orphan_anlz = mk("PIONEER/USBANLZ/P002/0000BBBB/ANLZ0000.DAT");
        mk("PIONEER/Artwork/00001/a1.jpg");
        mk("PIONEER/Artwork/00001/a1_m.jpg");
        mk("Contents/A/.DS_Store");
        // Spelled differently on the drive (decomposed accent, other case): still used.
        mk("Contents/A/Cafe\u{301}.mp3");
        let mut lib = DeviceLibrary::default();
        lib.tracks.insert(2, DevTrack { id: 2, file_path: "/Contents/A/CAFÉ.mp3".into(), ..Default::default() });
        lib.tracks.insert(1, DevTrack { id: 1, file_path: "/Contents/A/kept.mp3".into(), analyze_path: "/PIONEER/USBANLZ/P001/0000AAAA/ANLZ0000.DAT".into(), artwork_path: "/PIONEER/Artwork/00001/a1.jpg".into(), ..Default::default() });
        assert!(unused_files(&root, &lib).is_empty(), "no cleanup before a classic catalog is present");
        std::fs::create_dir_all(root.join("PIONEER/rekordbox")).unwrap();
        std::fs::write(root.join("PIONEER/rekordbox/export.pdb"), build_export_pdb(&lib)).unwrap();
        let found: Vec<PathBuf> = unused_files(&root, &lib).into_iter().map(|f| f.0).collect();
        assert_eq!(found, { let mut v = vec![orphan.clone(), orphan_anlz.clone()]; v.sort(); v });
        // Deletion re-reads the drive catalog before removing the reviewed files.
        let n = delete_unused(&root, &found).unwrap();
        assert_eq!(n, 2);
        assert!(!root.join("Contents/B").exists() && !root.join("PIONEER/USBANLZ/P002").exists());
        assert!(root.join("Contents/A/kept.mp3").exists() && root.join("PIONEER/Artwork/00001/a1_m.jpg").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn drives_from_before_the_rename_keep_their_data() {
        let root = std::env::temp_dir().join(format!("recordbox_rename_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".PrepDeck/journal")).unwrap();
        std::fs::write(root.join(".PrepDeck/cues.json"), b"{\"/Contents/a.mp3\":\"x\"}").unwrap();
        // An unfinished export's journal must still be found after the move.
        std::fs::write(root.join(".PrepDeck/journal/journal.log"), b"B\n").unwrap();
        assert_eq!(read_cue_record(&root).len(), 0, "nothing read before the move");
        let undone = recover_device(&root);
        assert!(undone.is_some(), "the old folder's journal was recovered");
        assert!(!root.join(".PrepDeck").exists() && root.join(".Recordbox").is_dir());
        assert_eq!(read_cue_record(&root).get("/Contents/a.mp3").map(String::as_str), Some("x"));
        // A drive that already has the new folder is left alone.
        std::fs::create_dir_all(root.join(".PrepDeck")).unwrap();
        migrate_app_dir(&root);
        assert!(root.join(".PrepDeck").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn artwork_names_avoid_used_and_existing_files() {
        let root = std::env::temp_dir().join(format!("recordbox_art_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("PIONEER/Artwork/00001");
        std::fs::create_dir_all(&dir).unwrap();
        // a2_m.jpg exists on the drive (from rekordbox); a1 is used by a track in the library.
        std::fs::write(dir.join("a2_m.jpg"), b"x").unwrap();
        let mut lib = DeviceLibrary::default();
        lib.tracks.insert(9, DevTrack { id: 9, artwork_path: "/PIONEER/Artwork/00001/a1.jpg".into(), ..Default::default() });
        assert_eq!(free_artwork_name(&root, &lib), "/PIONEER/Artwork/00001/a3.jpg");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_real_rekordbox_export_if_available() {
        let p = std::env::var("PREPDECK_FIXTURE_PDB").unwrap_or_default();
        if p.is_empty() {
            return;
        }
        let lib = parse_pdb(std::fs::read(p).unwrap()).unwrap();
        assert!(!lib.tracks.is_empty());
        for t in lib.tracks.values() {
            assert!(t.file_path.starts_with("/Contents"));
        }
    }
}
