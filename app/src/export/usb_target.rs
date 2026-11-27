//! Resolve the device root before treating a missing classic database as a blank drive.
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceLayout {
    Empty,
    Classic,
    OneLibrary,
    Mixed,
}

pub(super) struct Catalogs {
    pub main: Option<PathBuf>,
    pub ext: Option<PathBuf>,
    modern: Vec<PathBuf>,
    writer_spelling: bool,
}

fn is_link(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    m.file_type().is_symlink()
}

// A case-sensitive mount can show two names that represent the same entry on FAT.
// Never choose between them or mistake an inaccessible catalog for an empty disk.
fn entries<'a>(dir: &Path, wanted: &[&'a str]) -> Result<HashMap<&'a str, PathBuf>> {
    let mut found = HashMap::new();
    for entry in
        fs::read_dir(dir).with_context(|| format!("reading USB folder {}", dir.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(key) = wanted
            .iter()
            .find(|k| name.eq_ignore_ascii_case(std::ffi::OsStr::new(**k)))
        {
            ensure!(
                found.insert(*key, entry.path()).is_none(),
                "Ambiguous USB spelling for {key} in {}",
                dir.display()
            );
            ensure!(
                !is_link(&fs::symlink_metadata(entry.path())?),
                "Linked USB entry {}. Select a local, unlinked device root.",
                entry.path().display()
            );
        }
    }
    Ok(found)
}

impl Catalogs {
    pub fn inspect(root: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(root)?;
        ensure!(
            metadata.is_dir() && !is_link(&metadata),
            "Select the USB drive root or the folder containing PIONEER"
        );
        let physical = root.canonicalize()?;
        for ancestor in physical.ancestors() {
            let name = ancestor.file_name().and_then(|s| s.to_str()).unwrap_or("");
            ensure!(
                !["PIONEER", ".PIONEER", ".Recordbox", ".PrepDeck"]
                    .iter()
                    .any(|n| name.eq_ignore_ascii_case(n)),
                "Select the drive root above {}, instead of a device subfolder",
                ancestor.display()
            );
            if ancestor != physical
                && ["PIONEER/rekordbox", ".PIONEER/rekordbox"]
                    .iter()
                    .any(|dir| ancestor.join(dir).is_dir())
            {
                bail!("Select the existing device root at {}", ancestor.display());
            }
        }
        let dirs = entries(
            root,
            &[
                "PIONEER",
                ".PIONEER",
                "Contents",
                ".Recordbox",
                ".PrepDeck",
                "export.pdb",
                "exportExt.pdb",
                "exportLibrary.db",
                "master.db",
                "library.db",
                "datafile.edb",
                "ExtData.edb",
            ],
        )?;
        ensure!(!["export.pdb", "exportExt.pdb", "exportLibrary.db", "master.db", "library.db", "datafile.edb", "ExtData.edb"].iter().any(|key| dirs.contains_key(key)),
            "This folder holds a catalog or desktop library. Select the USB root containing PIONEER, or a separate export folder.");
        let mut result = Self {
            main: None,
            ext: None,
            modern: Vec::new(),
            writer_spelling: true,
        };
        for name in ["Contents", ".Recordbox", ".PrepDeck"] {
            if let Some(actual) = dirs.get(name) {
                ensure!(actual.is_dir(), "{} must be a directory", actual.display());
                result.writer_spelling &= root.join(name).is_dir();
            }
        }
        for name in ["PIONEER", ".PIONEER"] {
            let Some(pioneer) = dirs.get(name) else {
                continue;
            };
            let sub = entries(pioneer, &["rekordbox", "Master", "USBANLZ", "Artwork"])?;
            if let Some(master) = sub.get("Master") {
                ensure!(entries(master, &["master.db", "datafile.edb", "ExtData.edb"])?.is_empty(), "This device contains an external desktop library in PIONEER/Master. Choose a separate export target.");
            }
            if name == "PIONEER" {
                result.writer_spelling &= root.join("PIONEER").is_dir();
                for folder in ["USBANLZ", "Artwork", "rekordbox"] {
                    if sub.contains_key(folder) {
                        result.writer_spelling &= root.join("PIONEER").join(folder).is_dir();
                    }
                }
            }
            let Some(catalog) = sub.get("rekordbox") else {
                continue;
            };
            let mut files = entries(
                catalog,
                &[
                    "export.pdb",
                    "exportExt.pdb",
                    "exportLibrary.db",
                    "master.db",
                    "datafile.edb",
                    "ExtData.edb",
                ],
            )?;
            ensure!(
                !["master.db", "datafile.edb", "ExtData.edb"]
                    .iter()
                    .any(|key| files.contains_key(key)),
                "Desktop library detected in the selected USB catalog folder"
            );
            for path in files.values() {
                ensure!(
                    fs::metadata(path)?.is_file(),
                    "Expected a catalog file at {}",
                    path.display()
                );
            }
            if let Some(main) = files.remove("export.pdb") {
                ensure!(
                    result.main.replace(main).is_none(),
                    "Multiple classic USB catalogs found; select an unambiguous device root"
                );
                result.writer_spelling &= root.join("PIONEER/rekordbox/export.pdb").is_file();
            }
            if let Some(ext) = files.remove("exportExt.pdb") {
                ensure!(
                    result.ext.replace(ext).is_none(),
                    "Multiple extended classic USB catalogs found"
                );
                result.writer_spelling &= root.join("PIONEER/rekordbox/exportExt.pdb").is_file();
            }
            if let Some(path) = files.remove("exportLibrary.db") {
                result.modern.push(path);
            }
        }
        ensure!(
            result.ext.is_none()
                || result.main.as_ref().and_then(|p| p.parent())
                    == result.ext.as_ref().and_then(|p| p.parent()),
            "Incomplete classic device library: export.pdb and exportExt.pdb must share a folder"
        );
        Ok(result)
    }
    pub fn layout(&self) -> DeviceLayout {
        match (self.main.is_some(), !self.modern.is_empty()) {
            (false, false) => DeviceLayout::Empty,
            (true, false) => DeviceLayout::Classic,
            (false, true) => DeviceLayout::OneLibrary,
            (true, true) => DeviceLayout::Mixed,
        }
    }
}

impl DeviceLayout {
    pub fn inspect(root: &Path) -> Result<Self> {
        Ok(Catalogs::inspect(root)?.layout())
    }
}

/// Files for a single closed OneLibrary snapshot, with its classic sibling if present.
pub(crate) fn inspect_onelibrary_files(root: &Path) -> Result<(PathBuf, Option<PathBuf>)> {
    let catalogs = Catalogs::inspect(root)?;
    let [modern] = catalogs.modern.as_slice() else {
        bail!("Select a drive containing exactly one OneLibrary exportLibrary.db");
    };
    if let Some(classic) = &catalogs.main {
        ensure!(classic.parent() == modern.parent(), "The catalogs are in different export folders");
    }
    let filename = modern.file_name().context("Invalid OneLibrary filename")?.to_string_lossy();
    let sidecars = ["-wal", "-shm", "-journal"].map(|ext| format!("{filename}{ext}"));
    let folder = modern.parent().context("Invalid OneLibrary folder")?;
    for entry in fs::read_dir(folder)? {
        let name = entry?.file_name();
        ensure!(!sidecars.iter().any(|s| name.eq_ignore_ascii_case(std::ffi::OsStr::new(s))),
            "OneLibrary has a SQLite sidecar. Close Rekordbox and use a cleanly closed export. Leave the sidecars in place.");
    }
    if let Some(recordbox) = entries(root, &[".Recordbox"])?.get(".Recordbox") {
        if let Some(journal) = entries(recordbox, &["journal"])?.get("journal") {
            ensure!(entries(journal, &["journal.log"])?.is_empty(), "A recordbox USB export is active or needs recovery");
        }
    }
    Ok((modern.to_owned(), catalogs.main))
}

pub fn inspect_target(root: &Path) -> Result<(DeviceLayout, Option<String>)> {
    let paths = Catalogs::inspect(root)?;
    let layout = paths.layout();
    let reason = if layout == DeviceLayout::OneLibrary {
        Some("This drive contains only OneLibrary (Device Library Plus). Use AlphaTheta Rekordbox to update it, or select a separate classic export target.")
    } else if !paths.writer_spelling {
        Some("The existing USB layout can be read, but writing here would create a second tree with different spelling. Use the canonical PIONEER/rekordbox layout or a separate target.")
    } else {
        None
    };
    Ok((layout, reason.map(str::to_owned)))
}

pub fn check_export_target(root: &Path) -> Result<DeviceLayout> {
    let (layout, blocker) = inspect_target(root)?;
    if let Some(reason) = blocker {
        bail!("{reason}");
    }
    Ok(layout)
}

/// Check every existing component on the requested drive; a symlink or Windows
/// junction can otherwise redirect a write outside an otherwise valid catalog.
pub(super) fn confined(root: &Path, path: &Path) -> Result<()> {
    let rel = path
        .strip_prefix(root)
        .context("destination is outside the selected USB root")?;
    let mut part = root.to_path_buf();
    ensure!(
        rel.components().next().is_some(),
        "expected a path inside the selected drive"
    );
    for c in rel.components() {
        let Component::Normal(name) = c else {
            bail!("invalid path inside the selected USB drive");
        };
        let s = name.to_string_lossy();
        ensure!(
            !s.contains([':', '\\', '<', '>', '"', '|', '?', '*'])
                && !s.ends_with(['.', ' '])
                && !s.bytes().any(|b| b < 32),
            "invalid USB path component"
        );
        #[cfg(windows)]
        {
            // Win32 treats these components as devices even with a filename extension.
            let stem = s
                .split('.')
                .next()
                .unwrap_or("")
                .trim_end()
                .to_ascii_uppercase();
            let serial = stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"));
            ensure!(
                !matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                ) && !serial.is_some_and(|n| matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )),
                "reserved Windows device name in USB path"
            );
        }
        part.push(name);
        match fs::symlink_metadata(&part) {
            Ok(m) => ensure!(!is_link(&m), "linked USB path: {}", part.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Only the classic export assets and private Recordbox records may be journaled.
/// Catalogs themselves have a separate fixed, two-file publication sequence.
pub(super) fn managed_file(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let fields: Vec<_> = rel.iter().map(|s| s.to_str().unwrap_or("")).collect();
    // A differently-cased spelling from a native catalog can name the same
    // directory on FAT. On a case-sensitive mount it must already exist;
    // otherwise the writer would create a second tree.
    let directory = |parent: &Path, name: &str, canonical: &str| {
        name == canonical || (name.eq_ignore_ascii_case(canonical) && parent.join(name).is_dir())
    };
    match fields.as_slice() {
        [folder, _, ..] if directory(root, folder, "Contents") => true,
        [folder, sub, _, ..]
            if directory(root, folder, "PIONEER") || directory(root, folder, ".PIONEER") =>
        {
            let base = root.join(folder);
            directory(&base, sub, "USBANLZ") || directory(&base, sub, "Artwork")
        }
        ["PIONEER" | ".PIONEER", "MYSETTING.DAT" | "MYSETTING2.DAT" | "DEVSETTING.DAT" | "DJMMYSETTING.DAT"] => {
            true
        }
        [".Recordbox", "device.json" | "cues.json" | "nodes.json" | "artwork.json" | "props.json"
        | "sync.json"] => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{device_import, usb};

    fn write(root: &Path, rel: &str, data: &[u8]) -> Result<PathBuf> {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, data)?;
        Ok(path)
    }

    #[test]
    fn one_library_only_is_never_initialized_as_an_empty_classic_drive() -> Result<()> {
        for name in [
            "PIONEER/rekordbox/exportLibrary.db",
            ".PIONEER/REKORDBOX/EXPORTLIBRARY.DB",
        ] {
            let dir = tempfile::tempdir()?;
            let root = dir.path();
            let modern = write(root, name, b"opaque modern catalog")?;
            let settings = write(root, "PIONEER/MYSETTING.DAT", b"shared player setting")?;
            assert_eq!(inspect_target(root)?.0, DeviceLayout::OneLibrary);
            assert!(check_export_target(root).is_err());
            assert!(usb::read_device(root).is_err());
            assert!(usb::set_device_name(root, "new label").is_err());
            assert!(device_import::write_props(root, &Default::default()).is_err());
            assert!(device_import::delete_my_settings(root).is_err());
            assert_eq!(fs::read(modern)?, b"opaque modern catalog");
            assert_eq!(fs::read(settings)?, b"shared player setting");
            assert!(!root.join("PIONEER/rekordbox/export.pdb").exists());
            assert!(!root.join(".Recordbox").exists());
        }
        Ok(())
    }

    #[test]
    fn external_desktop_and_nested_usb_roots_are_refused_before_writing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        assert_eq!(check_export_target(root)?, DeviceLayout::Empty);
        let desktop = write(root, "PIONEER/Master/master.db", b"desktop")?;
        for path in [
            root.to_path_buf(),
            root.join("PIONEER"),
            root.join("PIONEER/Master"),
        ] {
            assert!(usb::set_device_name(&path, "label").is_err());
            assert!(!path.join(".Recordbox").exists());
        }
        assert_eq!(fs::read(desktop)?, b"desktop");
        let legacy = tempfile::tempdir()?;
        let db = write(legacy.path(), "datafile.edb", b"legacy")?;
        assert!(check_export_target(legacy.path()).is_err());
        assert_eq!(fs::read(db)?, b"legacy");
        Ok(())
    }

    #[test]
    fn existing_catalog_with_another_spelling_cannot_fork_or_disappear() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        let main = usb::build_export_pdb(&Default::default());
        write(root, "PIONEER/rekordbox/EXPORT.PDB", &main)?;
        assert_eq!(DeviceLayout::inspect(root)?, DeviceLayout::Classic);
        assert!(usb::read_device(root)?.tracks.is_empty());
        if !root.join("PIONEER/rekordbox/export.pdb").exists() {
            assert!(check_export_target(root).is_err());
            assert!(usb::set_device_name(root, "label").is_err());
            write(root, "PIONEER/rekordbox/export.pdb", &main)?;
            assert!(usb::read_device(root).is_err());
        }
        Ok(())
    }

    #[test]
    fn damaged_extension_is_preserved_and_catalog_changes_are_refused() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        let main = usb::build_export_pdb(&Default::default());
        let main_path = write(root, "PIONEER/rekordbox/export.pdb", &main)?;
        let extension = write(
            root,
            "PIONEER/rekordbox/exportExt.pdb",
            b"unknown catalog format",
        )?;
        assert!(usb::set_device_name(root, "label").is_err());
        assert_eq!(fs::read(main_path)?, main);
        assert_eq!(fs::read(extension)?, b"unknown catalog format");
        assert!(!root.join(".Recordbox").exists());
        Ok(())
    }

    #[test]
    fn mixed_export_refuses_changed_shared_analysis_and_keeps_player_settings() -> Result<()> {
        use std::sync::atomic::AtomicBool;
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("volume");
        fs::create_dir(&root)?;
        let src = dir.path().join("music");
        let mut tracks = crate::export::usb_test::demo_tracks(&src, 1);
        let request = |tracks| usb::ExportRequest {
            root: root.clone(),
            tracks,
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        usb::run_export(request(tracks.clone()), &|_| (), &cancel)?;
        let lib = usb::read_device(&root)?;
        let track = lib.tracks.values().next().unwrap();
        let analysis = root.join(track.analyze_path.trim_start_matches('/'));
        let audio = root.join(track.file_path.trim_start_matches('/'));
        let before_analysis = usb::sha256_file(&analysis)?;
        let before_audio = usb::sha256_file(&audio)?;
        let modern = write(
            &root,
            "PIONEER/rekordbox/exportLibrary.db",
            b"modern catalog",
        )?;
        let setting = write(&root, "PIONEER/MYSETTING.DAT", b"shared settings")?;
        assert_eq!(check_export_target(&root)?, DeviceLayout::Mixed);
        // A catalog edit is still valid on this drive.
        usb::set_device_name(&root, "Classic system")?;
        let main = root.join("PIONEER/rekordbox/export.pdb");
        let before_main = usb::sha256_file(&main)?;
        tracks.get_mut(&1).unwrap().cues[0].start += 1.0;
        let error = usb::run_export(request(tracks), &|_| (), &cancel).unwrap_err();
        assert!(
            error.to_string().contains("shared with OneLibrary"),
            "{error:#}"
        );
        assert!(device_import::delete_my_settings(&root).is_err());
        assert!(usb::delete_unused(&root, &[audio.clone()]).is_err());
        assert_eq!(usb::sha256_file(&main)?, before_main);
        assert_eq!(usb::sha256_file(&analysis)?, before_analysis);
        assert_eq!(usb::sha256_file(&audio)?, before_audio);
        assert_eq!(fs::read(modern)?, b"modern catalog");
        assert_eq!(fs::read(setting)?, b"shared settings");
        assert!(!root.join(".Recordbox/journal/journal.log").exists());
        Ok(())
    }

    #[test]
    fn alternate_analysis_spelling_retains_its_native_device_path() {
        assert_eq!(
            usb::anlz_base("/.Pioneer/USBanlz/P002/ABCD/ANLZ0000.Dat").as_deref(),
            Some("/.Pioneer/USBanlz/P002/ABCD/ANLZ0000"),
        );
        assert!(usb::anlz_base("/PIONEER/USBANLZ/../master.db.DAT").is_none());
        assert!(usb::anlz_base("/Contents/album/track.DAT").is_none());
    }

    #[test]
    fn nested_paths_longer_than_a_metadata_field_round_trip_exactly() -> Result<()> {
        let mut lib = usb::DeviceLibrary::default();
        let path = format!("/Contents/{}/{}/歌曲.mp3", "a".repeat(145), "b".repeat(145));
        lib.tracks.insert(
            1,
            usb::DevTrack {
                id: 1,
                file_path: path.clone(),
                ..Default::default()
            },
        );
        let saved = usb::parse_pdb(usb::try_build_export_pdb(&lib)?)?;
        assert_eq!(saved.tracks[&1].file_path, path);
        Ok(())
    }

    #[test]
    fn removing_one_classic_row_keeps_all_assets_referenced_by_another() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        let mut lib = usb::DeviceLibrary::default();
        let mut track = usb::DevTrack {
            id: 1,
            file_path: "/Contents/Café.mp3".into(),
            analyze_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT".into(),
            artwork_path: "/PIONEER/Artwork/00001/a1.jpg".into(),
            ..Default::default()
        };
        lib.tracks.insert(1, track.clone());
        track.id = 2;
        track.file_path = "/Contents/Cafe\u{301}.MP3".into();
        track.analyze_path = "/PIONEER/USBANLZ/P000/00000001/anlz0000.dat".into();
        track.artwork_path = "/PIONEER/Artwork/00001/A1.jpg".into();
        lib.tracks.insert(2, track);
        write(
            root,
            "PIONEER/rekordbox/export.pdb",
            &usb::build_export_pdb(&lib),
        )?;
        assert!(usb::remove_tracks(&mut lib, root, &[1]).is_empty());
        assert_eq!(lib.tracks.len(), 1);
        Ok(())
    }

    #[test]
    fn recovery_keeps_a_shared_file_and_old_classic_journal_on_a_mixed_drive() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        write(
            root,
            "PIONEER/rekordbox/export.pdb",
            &usb::build_export_pdb(&Default::default()),
        )?;
        write(root, "PIONEER/rekordbox/exportLibrary.db", b"modern")?;
        let audio = write(root, "Contents/audio.mp3", b"shared")?;
        let journal = write(
            root,
            ".Recordbox/journal/journal.log",
            b"B\nD\tContents/audio.mp3\nP\n",
        )?;
        assert!(usb::recover_checked(root).is_err());
        assert!(usb::set_device_name(root, "label").is_err());
        assert_eq!(fs::read(audio)?, b"shared");
        assert!(journal.is_file());
        Ok(())
    }
}
