//! Reading things back from a device: a track for "Import To Collection" (audio, details,
//! cues, beat grid, analysis files, artwork, My Tags) and the players' My Settings files.

use super::anlz;
use super::settings::{crc16_xmodem, UsbSettings};
use super::usb::{self, DevTrack, DeviceLibrary};
use crate::model::*;
use anyhow::{anyhow, Context, Result};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// A device track copied into the collection's folders, ready to be added to the library.
pub struct ImportedTrack {
    pub dev_id: u32,
    pub track: Track,
    pub cues: Vec<Cue>,
    pub grid: Option<BeatGrid>,
    /// Copy of the device's analysis files (base path without extension), kept so the
    /// players' own waveforms are shown and written back on the next export.
    pub anlz_base: Option<PathBuf>,
    /// My Tags as (category, tag).
    pub tags: Vec<(String, String)>,
    /// The audio file was already there (an earlier import): nothing was copied.
    pub reused_file: bool,
}

/// Where an imported device file goes: `<music_dir>/<artist>/<album>/<file name>`, the same
/// folder rule the export uses on the drive. A different file already at that name gets
/// " (2)", " (3)" …; an existing regular file is reused only when its size and
/// contents match. Hashes are read in bounded chunks, only for same-size collisions.
pub fn import_destination(
    music_dir: &Path,
    dt: &DevTrack,
    source: &Path,
) -> Result<(PathBuf, bool)> {
    let metadata = std::fs::metadata(source)
        .with_context(|| format!("could not inspect the device file {}", source.display()))?;
    if !metadata.is_file() {
        anyhow::bail!(
            "the device file is not a regular file: {}",
            source.display()
        );
    }
    let size = metadata.len();
    let mut source_hash = None;
    let dir =
        music_dir.join(usb::media_dir(&dt.artist, &dt.album).trim_start_matches("/Contents/"));
    let name = Path::new(&dt.file_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "track".into());
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.clone(), String::new()),
    };
    let mut n = 1;
    loop {
        let cand = dir.join(if n == 1 {
            name.clone()
        } else {
            format!("{stem} ({n}){ext}")
        });
        match std::fs::symlink_metadata(&cand) {
            Ok(m) if m.is_file() && m.len() == size => {
                let hash = match source_hash {
                    Some(hash) => hash,
                    None => {
                        let hash = usb::sha256_file(source)
                            .context("could not read the device file for comparison")?;
                        source_hash = Some(hash);
                        hash
                    }
                };
                if usb::sha256_file(&cand).with_context(|| {
                    format!("could not compare the existing file {}", cand.display())
                })? == hash
                {
                    return Ok((cand, true));
                }
                n += 1;
            }
            Ok(_) => n += 1,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok((cand, false)),
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("could not inspect the destination {}", cand.display())
                })
            }
        }
    }
}

/// Copy to a unique sibling temporary file, then publish its complete contents
/// without replacing a destination created after it was chosen.
fn copy_audio_no_replace(source: &Path, destination: &Path, expected_size: u64) -> Result<()> {
    let mut input = std::fs::File::open(source).context("could not open the device file")?;
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("the imported file has no parent directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .context("could not create an import temporary file")?;
    let copied = std::io::copy(&mut input, temporary.as_file_mut())
        .context("could not copy the device file")?;
    if copied != expected_size {
        anyhow::bail!("the copy is incomplete ({copied} of {expected_size} bytes)");
    }
    temporary.as_file_mut().flush()?;
    let permissions = input.metadata()?.permissions();
    temporary.as_file().set_permissions(permissions.clone())?;
    temporary.as_file().sync_all()?;
    let imported = temporary
        .persist_noclobber(destination)
        .map_err(|error| error.error)
        .with_context(|| {
            format!(
                "could not publish the imported file {}",
                destination.display()
            )
        })?;
    // Windows persistence clears temporary attributes, including the read-only bit.
    imported.set_permissions(permissions)?;
    Ok(())
}

/// Publish a complete set together. A subsequent import with fewer companions must
/// never pick up an obsolete file from the previous analysis of the same track.
fn cache_device_analysis(data_dir: &Path, files: &[(&str, Vec<u8>)]) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let mut key = Sha256::new();
    for (extension, bytes) in files {
        key.update(extension.as_bytes());
        key.update(Sha256::digest(bytes));
    }
    let cache = data_dir.join("device-anlz");
    let dir = cache.join(format!("{:x}", key.finalize()));
    if !dir.is_dir() {
        std::fs::create_dir_all(&cache)?;
        let staged = tempfile::tempdir_in(&cache)?;
        for (extension, bytes) in files {
            std::fs::write(staged.path().join(format!("ANLZ0000.{extension}")), bytes)?;
        }
        match std::fs::rename(staged.path(), &dir) {
            Ok(()) => {}
            Err(_) if dir.is_dir() => {} // A concurrent import published this same complete set.
            Err(e) => return Err(e).context("Publishing imported device analysis"),
        }
    }
    Ok(dir.join("ANLZ0000"))
}

/// Copy one device track into `music_dir` and read everything the device knows about it.
/// Nothing on the device is changed. `data_dir` receives the copy of its analysis files and
/// `artwork_dir` its artwork when the audio file has none embedded.
pub fn import_track(root: &Path, lib: &DeviceLibrary, id: u32, music_dir: &Path, data_dir: &Path, artwork_dir: &Path) -> Result<ImportedTrack> {
    let dt = lib.tracks.get(&id).ok_or_else(|| anyhow!("the track is no longer on the device"))?;
    let src = root.join(dt.file_path.trim_start_matches('/'));
    super::usb_target::confined(root, &src)?;
    let size = match std::fs::metadata(&src) {
        Ok(metadata) => metadata.len(),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(anyhow!("file not found on the device"))
        }
        Err(e) => return Err(e).context("could not inspect the device file"),
    };
    let mut analysis = Vec::new();
    if !dt.analyze_path.is_empty() {
        let base = usb::anlz_base(&dt.analyze_path).context("Invalid device analysis path in the USB catalog")?;
        let base = root.join(base.trim_start_matches('/'));
        for extension in usb::ANLZ_EXTS {
            let path = base.with_extension(extension);
            super::usb_target::confined(root, &path)?;
            if let Some(bytes) = anlz::device_companion(&path, &dt.file_path)? {
                analysis.push((extension, bytes));
            }
        }
    }
    let (dest, reused_file) = import_destination(music_dir, dt, &src)?;
    if !reused_file {
        std::fs::create_dir_all(dest.parent().unwrap())?;
        copy_audio_no_replace(&src, &dest, size)?;
    }
    // The file's own tags first, then what the device library says (which is what the DJ saw
    // and edited in rekordbox), field by field where the device has a value.
    let mut t = crate::importer::read_track(&dest);
    let set = |to: &mut String, from: &str| {
        if !from.trim().is_empty() {
            *to = from.to_string();
        }
    };
    set(&mut t.title, &dt.title);
    set(&mut t.artist, &dt.artist);
    set(&mut t.album, &dt.album);
    set(&mut t.album_artist, &dt.album_artist);
    set(&mut t.genre, &dt.genre);
    set(&mut t.label, &dt.label);
    set(&mut t.remixer, &dt.remixer);
    set(&mut t.original_artist, &dt.original_artist);
    set(&mut t.composer, &dt.composer);
    set(&mut t.lyricist, &dt.lyricist);
    set(&mut t.comment, &dt.comment);
    set(&mut t.message, &dt.message);
    set(&mut t.release_date, &dt.release_date);
    // The device's title is the library's display title: a mix name is already part of it.
    if !dt.mix_name.is_empty() && !t.title.contains(&dt.mix_name) {
        t.mix_name = dt.mix_name.clone();
    } else {
        t.mix_name.clear();
    }
    if dt.year > 0 {
        t.year = dt.year as i32;
    }
    if dt.track_no > 0 {
        t.track_no = dt.track_no as i32;
    }
    if dt.disc_no > 0 {
        t.disc_no = dt.disc_no as i32;
    }
    if dt.bpm100 > 0 {
        t.bpm = dt.bpm100 as f64 / 100.0;
    }
    if !dt.key_name.is_empty() {
        t.key = parse_key(&dt.key_name);
        t.key_text = dt.key_name.clone();
    }
    t.rating = dt.rating.min(5);
    t.color = dt.color.min(8);
    t.play_count = dt.play_count as u32;
    t.autoload_hotcue = dt.autoload;
    if dt.date_added.len() == 10 {
        t.date_added = dt.date_added.clone();
    }
    let companion = |wanted| analysis.iter().find(|(ext, _)| *ext == wanted).map(|(_, bytes)| bytes.as_slice());
    let dat = companion("DAT");
    let dat_sections = dat.map(anlz::section_list).unwrap_or_default();
    let ext_sections = companion("EXT").map(anlz::section_list).unwrap_or_default();
    let cues = anlz::parse_cues(&ext_sections, &dat_sections);
    let grid = dat.and_then(|bytes| {
        companion("EXT").and_then(|ext| anlz::grid_pair::read(std::io::Cursor::new(bytes), std::io::Cursor::new(ext)))
            .or_else(|| crate::rbimport::parse_pqtz_grid(bytes))
    });
    let anlz_base = if dat.is_some() { Some(cache_device_analysis(data_dir, &analysis)?) } else { None };
    // Artwork: the file's embedded picture when it has one, else the device's (the larger
    // "_m" version when it exists).
    if t.artwork.is_empty() && !dt.artwork_path.is_empty() {
        let small = root.join(dt.artwork_path.trim_start_matches('/'));
        let big = small.with_file_name(format!("{}_m.jpg", small.file_stem().unwrap_or_default().to_string_lossy()));
        super::usb_target::confined(root, &small)?;
        super::usb_target::confined(root, &big)?;
        if let Some(from) = [big, small].into_iter().find(|p| p.is_file()) {
            if let Ok(hash) = usb::sha256_file(&from) {
                let _ = std::fs::create_dir_all(artwork_dir);
                let to = artwork_dir.join(format!("device-{}.jpg", &usb::hex(hash)[..16]));
                if to.is_file() || std::fs::copy(&from, &to).is_ok() {
                    t.artwork = to.to_string_lossy().to_string();
                }
            }
        }
    }
    let tags = dt
        .tags
        .iter()
        .map(|k| match k.split_once('\u{1f}') {
            Some((c, n)) => (c.to_string(), n.to_string()),
            None => (String::new(), k.clone()),
        })
        .collect();
    Ok(ImportedTrack { dev_id: id, track: t, cues, grid, anlz_base, tags, reused_file })
}

/// The My Settings files on a drive: (file name, payload length, checksum over whole file).
const SETTING_FILES: [(&str, usize, bool); 4] = [("MYSETTING.DAT", 40, false), ("MYSETTING2.DAT", 40, false), ("DEVSETTING.DAT", 32, false), ("DJMMYSETTING.DAT", 52, true)];
/// The files "Delete My Settings" removes (the players' and mixers' settings; DEVSETTING.DAT
/// holds the device's display settings and stays).
pub const MY_SETTING_FILES: [&str; 3] = ["MYSETTING.DAT", "MYSETTING2.DAT", "DJMMYSETTING.DAT"];

/// Payload of a settings file when its lengths and CRC-16/XMODEM checksum are right.
fn valid_payload(bytes: &[u8], len: usize, whole: bool) -> Option<Vec<u8>> {
    if bytes.len() != 108 + len || u32::from_le_bytes(bytes.get(100..104)?.try_into().ok()?) as usize != len {
        return None;
    }
    let crc = u32::from_le_bytes(bytes[104 + len..108 + len].try_into().ok()?) as u16;
    let covered = if whole { &bytes[..104 + len] } else { &bytes[104..104 + len] };
    (crc == crc16_xmodem(covered)).then(|| bytes[104..104 + len].to_vec())
}

/// "Import My Settings": the drive's settings files laid over `current`. Only files that are
/// present and valid are taken; returns the result and the names of the files read (empty
/// when the drive has none: "My Settings file is not available.").
pub fn read_my_settings(root: &Path, current: &UsbSettings) -> (UsbSettings, Vec<&'static str>) {
    let mut out = current.clone().normalized();
    let mut found = vec![];
    for dir in ["PIONEER", ".PIONEER"] {
        for (name, len, whole) in SETTING_FILES {
            if found.contains(&name) {
                continue;
            }
            let Ok(bytes) = std::fs::read(root.join(dir).join(name)) else { continue };
            let Some(p) = valid_payload(&bytes, len, whole) else { continue };
            match name {
                "MYSETTING.DAT" => out.my = p,
                "MYSETTING2.DAT" => out.my2 = p,
                "DEVSETTING.DAT" => out.dev = p,
                _ => out.djm = p,
            }
            found.push(name);
        }
    }
    (out, found)
}

/// Whether the drive has any of the My Settings files.
pub fn has_my_settings(root: &Path) -> bool {
    ["PIONEER", ".PIONEER"].iter().any(|d| MY_SETTING_FILES.iter().any(|f| root.join(d).join(f).is_file()))
}

/// "Delete My Settings": remove the players' and mixers' settings files from the drive.
/// Returns how many files were deleted.
pub fn delete_my_settings(root: &Path) -> Result<usize> {
    anyhow::ensure!(matches!(usb::check_preference_target(root)?, usb::DeviceLayout::Classic | usb::DeviceLayout::Empty), "Shared player settings on a mixed-format drive are protected");
    let mut n = 0;
    for dir in ["PIONEER", ".PIONEER"] {
        for f in MY_SETTING_FILES {
            let path = root.join(dir).join(f);
            super::usb_target::confined(root, &path)?;
            match std::fs::remove_file(path) {
                Ok(()) => n += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(n)
}

/// Recordbox's own per-device properties (`/.Recordbox/props.json`).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DeviceProps {
    /// Background Color: 0 Default Color, 1–8 Pink … Purple. The standard device library
    /// (export.pdb, DEVSETTING.DAT) has no field for it; rekordbox keeps it in OneLibrary's
    /// `property.backGroundColorType`, which Recordbox does not write. Kept here so the
    /// choice travels with the drive and colours its row in Recordbox.
    pub bg_color: u8,
}

pub fn read_props(root: &Path) -> DeviceProps {
    std::fs::read_to_string(root.join(".Recordbox").join("props.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write_props(root: &Path, p: &DeviceProps) -> Result<()> {
    usb::write_private(root, "props.json", &serde_json::to_vec_pretty(p)?)
}

/// Which library playlists a device is kept in sync with (`/.Recordbox/sync.json`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// "All playlists" (true) or "Selected playlists".
    pub all: bool,
    /// Library IDs of the ticked playlists and folders.
    pub playlists: Vec<i64>,
    /// "Automatic synchronization": offered each time the device is connected.
    pub auto: bool,
    /// "including cue points": tracks already on the device get the library's cues again.
    pub cues: bool,
    /// The library (computer) this device is synchronised with.
    pub library: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig { all: false, playlists: vec![], auto: false, cues: true, library: String::new() }
    }
}

pub fn read_sync(root: &Path) -> Option<SyncConfig> {
    serde_json::from_str(&std::fs::read_to_string(root.join(".Recordbox").join("sync.json")).ok()?).ok()
}

pub fn write_sync(root: &Path, c: &SyncConfig) -> Result<()> {
    usb::write_private(root, "sync.json", &serde_json::to_vec_pretty(c)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "recordbox-device-import-{}-{time}-{id}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn device_track() -> DevTrack {
        DevTrack {
            id: 1,
            artist: "Artist".into(),
            album: "Album".into(),
            file_path: "/song.mp3".into(),
            ..Default::default()
        }
    }

    #[test]
    fn unknown_or_inconsistent_usb_mirrors_keep_the_dat_grid_and_original_bytes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let device = dir.path().join("usb");
        let mut track = device_track();
        track.analyze_path = "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".into();
        let dat = device.join(track.analyze_path.trim_start_matches('/'));
        std::fs::create_dir_all(dat.parent().unwrap())?;
        std::fs::write(device.join("song.mp3"), b"fixture")?;
        let beats: Vec<_> = [0.100501, 0.900866, 1.400011].iter().enumerate()
            .map(|(i,&time)| Beat { time, index: i, beat: i as u8 + 1, bpm: 120.0 }).collect();
        let source = anlz::build(&track.file_path, &beats, &Waveform::default(), &[], 0, 44100);
        std::fs::write(&dat, &source.dat)?;
        let section = source.ext.windows(4).position(|tag| tag == b"PQT2").context("missing extended grid")?;
        let mut lib = DeviceLibrary::default();
        lib.tracks.insert(1, track);
        let data = dir.path().join("data");
        for (field, value) in [(16, 2u8), (16, 9), (47, source.ext[section + 47] ^ 1), (56, 0xff)] {
            let mut modified = source.ext.clone();
            modified[section + field] = value;
            std::fs::write(dat.with_extension("EXT"), &modified)?;
            let import = import_track(&device, &lib, 1, &dir.path().join("music"), &data, &dir.path().join("art"))?;
            let grid = import.grid.context("DAT tempo segments should remain readable")?;
            assert!(!grid.has_listed_beats());
            assert_eq!(grid.anchors[0].time, 0.1);
            let cached = import.anlz_base.context("USB analysis files remain available")?;
            assert_eq!(std::fs::read(cached.with_extension("EXT"))?, modified);
            assert_eq!(std::fs::read(dat.with_extension("EXT"))?, modified);
        }
        Ok(())
    }

    #[test]
    fn usb_analysis_cannot_attach_another_tracks_cues_or_leave_stale_companions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let device = dir.path().join("usb");
        let music = dir.path().join("music");
        let data = dir.path().join("data");
        let art = dir.path().join("art");
        let mut track = device_track();
        track.analyze_path = "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT".into();
        let audio = device.join(track.file_path.trim_start_matches('/'));
        let dat = device.join(track.analyze_path.trim_start_matches('/'));
        std::fs::create_dir_all(audio.parent().unwrap())?;
        std::fs::create_dir_all(dat.parent().unwrap())?;
        std::fs::write(&audio, b"disposable test audio")?;
        let generate = |path| anlz::build(path, &[], &Waveform::default(), &[], 0, 0);
        let native = generate(&track.file_path);
        std::fs::write(&dat, &native.dat)?;
        std::fs::write(dat.with_extension("EXT"), generate("/Contents/other.mp3").ext)?;
        let mut lib = DeviceLibrary::default();
        lib.tracks.insert(1, track);
        let import = || import_track(&device, &lib, 1, &music, &data, &art);
        let error = import().err().context("A mismatched companion was imported")?;
        assert!(error.to_string().contains("conflicting track path"));
        assert!(!music.exists() && !data.exists());

        std::fs::write(dat.with_extension("EXT"), native.ext)?;
        let first = import()?.anlz_base.context("Missing device analysis")?;
        assert!(first.with_extension("EXT").is_file());
        std::fs::remove_file(dat.with_extension("EXT"))?;
        let second = import()?.anlz_base.context("Missing updated device analysis")?;
        assert_ne!(first, second);
        assert!(!second.with_extension("EXT").exists());
        assert_eq!(std::fs::read(second.with_extension("DAT"))?, native.dat);
        Ok(())
    }

    #[test]
    fn import_keeps_different_same_size_audio_and_reuses_only_matching_content() {
        let dir = TestDirectory::new();
        let device = dir.0.join("device");
        std::fs::create_dir(&device).unwrap();
        // A difference beyond the hash reader's first chunk must still be detected.
        let original = vec![37; (2 << 20) + 17];
        std::fs::write(device.join("song.mp3"), &original).unwrap();
        let track = device_track();
        let mut library = DeviceLibrary::default();
        library.tracks.insert(track.id, track);
        let music = dir.0.join("Music");
        let data = dir.0.join("data");
        let artwork = dir.0.join("artwork");

        let first = import_track(&device, &library, 1, &music, &data, &artwork).unwrap();
        assert!(!first.reused_file);
        let first_path = Path::new(&first.track.path);
        let mut different = original.clone();
        *different.last_mut().unwrap() ^= 1;
        std::fs::write(first_path, &different).unwrap();

        let second = import_track(&device, &library, 1, &music, &data, &artwork).unwrap();
        assert!(!second.reused_file);
        assert!(second.track.path.ends_with("song (2).mp3"));
        assert_eq!(std::fs::read(&second.track.path).unwrap(), original);
        assert_eq!(std::fs::read(first_path).unwrap(), different);

        let again = import_track(&device, &library, 1, &music, &data, &artwork).unwrap();
        assert!(again.reused_file);
        assert_eq!(again.track.path, second.track.path);
        assert_eq!(std::fs::read(device.join("song.mp3")).unwrap(), original);
        assert!(!music.join("Artist/Album/song (3).mp3").exists());
    }

    #[test]
    fn destination_directories_are_not_reused_or_replaced() {
        let dir = TestDirectory::new();
        let source = dir.0.join("source");
        std::fs::write(&source, b"audio").unwrap();
        let music = dir.0.join("Music");
        let occupied = music.join("Artist/Album/song.mp3");
        std::fs::create_dir_all(&occupied).unwrap();
        let (destination, reused) = import_destination(&music, &device_track(), &source).unwrap();
        assert!(!reused);
        assert_eq!(destination, music.join("Artist/Album/song (2).mp3"));
        assert!(occupied.is_dir());
    }

    #[test]
    fn missing_or_non_file_device_sources_are_reported() {
        let dir = TestDirectory::new();
        assert!(import_destination(
            &dir.0.join("Music"),
            &device_track(),
            &dir.0.join("missing")
        )
        .is_err());
        assert!(import_destination(&dir.0.join("Music"), &device_track(), &dir.0).is_err());
        assert!(!dir.0.join("Music").exists());
    }

    #[cfg(unix)]
    #[test]
    fn destination_inspection_errors_are_not_treated_as_available_paths() {
        let dir = TestDirectory::new();
        let source = dir.0.join("source");
        let music = dir.0.join("not-a-directory");
        std::fs::write(&source, b"audio").unwrap();
        std::fs::write(&music, b"keep me").unwrap();
        let error = import_destination(&music, &device_track(), &source).unwrap_err();
        assert!(error
            .to_string()
            .contains("could not inspect the destination"));
        assert_eq!(std::fs::read(&music).unwrap(), b"keep me");
    }

    #[cfg(unix)]
    #[test]
    fn destination_symlinks_are_kept_and_not_reused() {
        let dir = TestDirectory::new();
        let source = dir.0.join("source");
        std::fs::write(&source, b"audio").unwrap();
        let music = dir.0.join("Music");
        let occupied = music.join("Artist/Album/song.mp3");
        std::fs::create_dir_all(occupied.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&source, &occupied).unwrap();
        let (destination, reused) = import_destination(&music, &device_track(), &source).unwrap();
        assert!(!reused);
        assert_eq!(destination, music.join("Artist/Album/song (2).mp3"));
        assert_eq!(std::fs::read_link(&occupied).unwrap(), source);
    }

    #[test]
    fn copy_refuses_a_file_created_after_the_destination_was_selected() {
        let dir = TestDirectory::new();
        let source = dir.0.join("source");
        std::fs::write(&source, b"new audio").unwrap();
        let (destination, reused) =
            import_destination(&dir.0.join("Music"), &device_track(), &source).unwrap();
        assert!(!reused);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&destination, b"another import").unwrap();
        assert!(copy_audio_no_replace(&source, &destination, 9).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"another import");
        assert_eq!(std::fs::read(&source).unwrap(), b"new audio");
        assert_eq!(
            std::fs::read_dir(destination.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn incomplete_copy_removes_only_its_own_partial_destination() {
        let dir = TestDirectory::new();
        let source = dir.0.join("source");
        let destination = dir.0.join("destination");
        let unrelated = dir.0.join("destination.part");
        std::fs::write(&source, b"audio").unwrap();
        std::fs::write(&unrelated, b"keep me").unwrap();
        let error = copy_audio_no_replace(&source, &destination, 99).unwrap_err();
        assert!(error.to_string().contains("copy is incomplete"));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep me");
        assert_eq!(std::fs::read(&source).unwrap(), b"audio");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
    }

    #[test]
    fn complete_copy_preserves_read_only_source_permissions() {
        let dir = TestDirectory::new();
        let source = dir.0.join("source");
        let destination = dir.0.join("destination");
        std::fs::write(&source, b"audio").unwrap();
        let writable = std::fs::metadata(&source).unwrap().permissions();
        let mut readonly = writable.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&source, readonly).unwrap();
        let result = copy_audio_no_replace(&source, &destination, 5);
        let copied_readonly = std::fs::metadata(&destination).map(|m| m.permissions().readonly());
        // Restore original permissions before assertions so Windows can remove the fixture.
        std::fs::set_permissions(&source, writable.clone()).unwrap();
        if destination.exists() {
            std::fs::set_permissions(&destination, writable).unwrap();
        }
        result.unwrap();
        assert!(copied_readonly.unwrap());
        assert_eq!(std::fs::read(&destination).unwrap(), b"audio");
    }

    #[test]
    fn missing_copy_source_does_not_create_a_destination() {
        let dir = TestDirectory::new();
        let destination = dir.0.join("destination");
        assert!(copy_audio_no_replace(&dir.0.join("missing"), &destination, 5).is_err());
        assert!(!destination.exists());
    }
}
