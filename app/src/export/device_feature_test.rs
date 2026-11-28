//! Tests of the device-side features (import from a device, synchronisation, device
//! properties, My Settings, histories, colour names, .3EX) against folder devices under
//! target/export_test.

use super::device_import::*;
use super::pdb::{self, Database};
use super::settings::{SettingFile, UsbSettings, DEV_WAVEFORM_COLOR};
use super::usb::*;
use super::usb_test::demo_tracks;
use crate::model::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("export_test");
    let root = base.join(format!("{name}_{}_{}", chrono::Local::now().format("%H%M%S%3f"), std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}
fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("export_test").join("src")
}
fn playlist(id: i64, name: &str, tracks: Vec<i64>) -> ExportNode {
    ExportNode { id, name: name.into(), tracks, ..Default::default() }
}
fn request(root: &Path, tracks: &HashMap<i64, ExportTrack>, nodes: Vec<ExportNode>) -> ExportRequest {
    ExportRequest { root: root.to_path_buf(), nodes, tracks: tracks.clone(), ..Default::default() }
}
fn export(req: ExportRequest) -> (String, ExportReport) {
    let report = std::sync::Mutex::new(ExportReport::default());
    let msg = run_export(
        req,
        &|p| {
            if let Progress::Report(r) = p {
                *report.lock().unwrap() = r;
            }
        },
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    (msg, report.into_inner().unwrap())
}
fn count_files(root: &Path, dir: &str) -> usize {
    walkdir::WalkDir::new(root.join(dir)).into_iter().flatten().filter(|e| e.file_type().is_file()).count()
}

#[test]
fn b1_a_device_track_is_imported_with_cues_grid_tags_and_analysis() {
    let root = scratch("import_dev");
    let mut tracks = demo_tracks(&src_dir(), 2);
    let art = src_dir().join("b1-cover.png");
    image::RgbImage::from_fn(48, 48, |x, y| image::Rgb([x as u8 * 5, y as u8 * 5, 90])).save(&art).unwrap();
    {
        let t = tracks.get_mut(&1).unwrap();
        t.track.title = "Night Drive".into();
        t.track.genre = "Techno".into();
        t.track.comment = "Peak time".into();
        t.track.rating = 4;
        t.track.color = 3;
        t.track.bpm = 121.5;
        t.grid = Some(crate::model::BeatGrid::from_listed(vec![
            crate::model::GridAnchor { time: 0.120501, bpm: 120.37, beat: 3 },
            crate::model::GridAnchor { time: 0.920064, bpm: 120.37, beat: 4 },
            crate::model::GridAnchor { time: 1.386999, bpm: 129.97, beat: 1 },
        ]));
        t.track.key_text = "7A".into();
        t.track.artwork = art.to_string_lossy().to_string();
        t.tags = vec![("Situation".into(), "Warm Up".into())];
        t.cues = vec![
            Cue { id: 1, track_id: 1, kind: CueKind::Hot, slot: 0, start: 1.5, end: None, color: 43, comment: "Intro".into(), active_loop: false },
            Cue { id: 2, track_id: 1, kind: CueKind::Hot, slot: 2, start: 4.0, end: Some(6.0), color: 0, comment: String::new(), active_loop: false },
            Cue { id: 3, track_id: 1, kind: CueKind::Memory, slot: -1, start: 8.25, end: None, color: 2, comment: "Break".into(), active_loop: false },
        ];
    }
    let mut req = request(&root, &tracks, vec![playlist(1, "Set", vec![1, 2])]);
    req.tag_categories = vec![("Situation".into(), vec!["Warm Up".into()])];
    export(req);
    let lib = read_device(&root).unwrap();
    let dev_id = lib.tracks.values().find(|t| t.title == "Night Drive").unwrap().id;
    let before: Vec<(PathBuf, Vec<u8>)> = walkdir::WalkDir::new(&root).into_iter().flatten().filter(|e| e.file_type().is_file()).map(|e| (e.path().to_path_buf(), std::fs::read(e.path()).unwrap())).collect();

    let home = scratch("import_home");
    let (music, data, artdir) = (home.join("Music"), home.join("data"), home.join("data/artwork"));
    let it = import_track(&root, &lib, dev_id, &music, &data, &artdir).unwrap();
    // The audio file is a byte-for-byte copy under <music>/<artist>/<album>/.
    assert!(!it.reused_file);
    let dest = PathBuf::from(&it.track.path);
    assert!(dest.starts_with(music.join("Journal Artist").join("UnknownAlbum")), "{}", dest.display());
    assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(&tracks[&1].track.path).unwrap());
    // Details come from the device library.
    let t = &it.track;
    assert_eq!((t.title.as_str(), t.artist.as_str(), t.genre.as_str(), t.comment.as_str(), t.rating, t.color, t.key_text.as_str()), ("Night Drive", "Journal Artist", "Techno", "Peak time", 4, 3, "7A"));
    assert!((t.bpm - tracks[&1].track.bpm).abs() < 0.02);
    assert_eq!(it.tags, vec![("Situation".to_string(), "Warm Up".to_string())]);
    // Cues (to the millisecond, with names and loop ends) and the beat grid.
    assert_eq!(it.cues.len(), 3);
    let hot_a = it.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == 0).unwrap();
    assert_eq!((hot_a.start, hot_a.comment.as_str(), hot_a.color), (1.5, "Intro", 43));
    let lp = it.cues.iter().find(|c| c.slot == 2).unwrap();
    assert_eq!((lp.start, lp.end), (4.0, Some(6.0)));
    let mem = it.cues.iter().find(|c| c.kind == CueKind::Memory).unwrap();
    assert_eq!((mem.start, mem.comment.as_str(), mem.color), (8.25, "Break", 2));
    let (g, want) = (it.grid.as_ref().unwrap(), tracks[&1].grid.as_ref().unwrap());
    assert_eq!(t.bpm, 121.5, "the device row keeps the published whole-track tempo");
    assert!(g.has_listed_beats());
    assert_eq!(g, want, "all measured beat numbers, tempi and microsecond times survive the import");
    // The device's analysis files are kept next to the library, valid, for display and re-export.
    let base = it.anlz_base.as_ref().unwrap();
    assert!(super::anlz::grid_pair::matches_source(&base.with_extension("DAT"), g));
    assert!(!super::anlz::grid_pair::matches_source(&base.with_extension("DAT"), &g.shifted(0.000001)));
    for e in ["DAT", "EXT", "2EX"] {
        assert!(super::anlz::validate(&std::fs::read(base.with_extension(e)).unwrap()).is_ok(), "{e}");
    }
    // The file has no embedded picture, so the device's larger artwork is taken.
    assert!(Path::new(&t.artwork).starts_with(&artdir) && image::open(&t.artwork).unwrap().width() == 240);
    // Importing again reuses the copy; a different file of the same name gets its own.
    let again = import_track(&root, &lib, dev_id, &music, &data, &artdir).unwrap();
    assert!(again.reused_file && again.track.path == it.track.path);
    std::fs::write(&dest, b"something else").unwrap();
    let other = import_track(&root, &lib, dev_id, &music, &data, &artdir).unwrap();
    assert_ne!(other.track.path, it.track.path);
    assert!(other.track.path.ends_with(" (2).wav"), "{}", other.track.path);
    // Nothing on the device changed.
    for (p, bytes) in before {
        assert_eq!(std::fs::read(&p).unwrap(), bytes, "{}", p.display());
    }
    // A track whose file is gone from the device is reported, not half-imported.
    let other_id = lib.tracks.keys().copied().find(|i| *i != dev_id).unwrap();
    std::fs::remove_file(root.join(lib.tracks[&other_id].file_path.trim_start_matches('/'))).unwrap();
    assert!(import_track(&root, &lib, other_id, &music, &data, &artdir).err().unwrap().to_string().contains("not found on the device"));
}

#[test]
fn b3_synchronisation_updates_recopies_and_removes() {
    let root = scratch("sync");
    let mut tracks = demo_tracks(&src_dir().join("b3"), 3);
    export(request(&root, &tracks, vec![playlist(1, "Keep", vec![1, 2]), playlist(2, "Gone", vec![2, 3])]));
    let first = read_device(&root).unwrap();
    assert_eq!((first.tracks.len(), first.nodes.len()), (3, 2));
    let record = read_node_record(&root);
    let gone = record[&2];
    let id_of = |lib: &DeviceLibrary, src: &str| lib.tracks.values().find(|t| t.source == src).map(|t| t.id);
    let (p1, p3) = (tracks[&1].track.path.clone(), tracks[&3].track.path.clone());

    // A cue edited on a player (the device's analysis files rewritten with another cue).
    let dev1 = &first.tracks[&id_of(&first, &p1).unwrap()];
    let base = root.join(dev1.analyze_path.trim_start_matches('/')).with_extension("");
    let player_cue = Cue { id: 0, track_id: 0, kind: CueKind::Hot, slot: 5, start: 3.21, end: None, color: 0, comment: "From player".into(), active_loop: false };
    let f = super::anlz::build(&dev1.file_path, &[], &Waveform::default(), &[player_cue], 0, 44100);
    std::fs::write(base.with_extension("DAT"), &f.dat).unwrap();
    std::fs::write(base.with_extension("EXT"), &f.ext).unwrap();
    // The library changes: a new title, and the local file of track 1 is replaced by a longer one.
    tracks.get_mut(&1).unwrap().track.title = "Renamed".into();
    let mut bytes = std::fs::read(&p1).unwrap();
    bytes.extend_from_slice(&[0u8; 4096]);
    std::fs::write(&p1, &bytes).unwrap();

    // Synchronise with only "Keep" ticked, cues not included, unused tracks deleted.
    let keep: HashMap<i64, ExportTrack> = tracks.iter().filter(|(id, _)| **id <= 2).map(|(k, v)| (*k, v.clone())).collect();
    let (_, rep) = export(ExportRequest { recopy_changed: true, remove_nodes: vec![gone], delete_unreferenced: true, keep_device_cues: true, ..request(&root, &keep, vec![playlist(1, "Keep", vec![2, 1])]) });
    assert_eq!((rep.copied, rep.updated, rep.removed_lists, rep.removed_tracks), (1, 1, 1, 1));
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["Keep"]);
    assert_eq!(lib.tracks.len(), 2);
    assert!(id_of(&lib, &p3).is_none(), "the track only the removed playlist used is gone");
    assert_eq!((count_files(&root, "Contents"), count_files(&root, "PIONEER/USBANLZ")), (2, 6));
    let d1 = &lib.tracks[&id_of(&lib, &p1).unwrap()];
    assert_eq!(d1.title, "Renamed");
    // The changed local file was copied again, to the same place.
    assert_eq!(d1.file_path, dev1.file_path);
    assert_eq!(std::fs::read(root.join(d1.file_path.trim_start_matches('/'))).unwrap(), bytes);
    assert_eq!(d1.file_size as usize, bytes.len());
    // Without "including cue points" the player's cue is still on the device …
    let cues = |lib: &DeviceLibrary, id: u32| {
        let b = root.join(lib.tracks[&id].analyze_path.trim_start_matches('/')).with_extension("");
        super::anlz::parse_cues(&super::anlz::read_section_list(&b.with_extension("EXT")), &super::anlz::read_section_list(&b.with_extension("DAT")))
    };
    let c = cues(&lib, d1.id);
    assert_eq!((c.len(), c[0].slot, c[0].comment.as_str()), (1, 5, "From player"));
    // … and with it the library's cues are written again.
    export(ExportRequest { recopy_changed: true, ..request(&root, &keep, vec![playlist(1, "Keep", vec![2, 1])]) });
    let c = cues(&read_device(&root).unwrap(), d1.id);
    assert_eq!((c.len(), c[0].slot, c[0].comment.as_str()), (1, 0, "Intro"));
    // Nothing is left in the journal, and no file is unused.
    assert!(!root.join(".Recordbox/journal").exists());
    assert!(unused_files(&root, &read_device(&root).unwrap()).is_empty());
    // The selection is kept on the drive.
    let cfg = SyncConfig { all: false, playlists: vec![1], auto: true, cues: false, library: "abc".into() };
    write_sync(&root, &cfg).unwrap();
    assert_eq!(read_sync(&root), Some(cfg));
    assert_eq!(read_sync(&scratch("sync_none")), None);
}

#[test]
fn b2_device_property_is_written_in_one_journaled_step() {
    let root = scratch("property");
    let tracks = demo_tracks(&src_dir(), 1);
    export(request(&root, &tracks, vec![playlist(1, "Set", vec![1])]));
    let mut settings = UsbSettings::default();
    settings.set(SettingFile::Dev, DEV_WAVEFORM_COLOR, 3);
    settings.set(SettingFile::My, 10, 0x80);
    let files: Vec<(PathBuf, Vec<u8>)> = settings.files().into_iter().map(|(n, b)| (root.join("PIONEER").join(n), b)).collect();
    let browse = UsbBrowse { categories: vec![4, 2, 11], sorts: vec![5] }.normalized();
    let b2 = browse.clone();
    change_device_with(&root, |lib| {
        lib.device_name = "ADAM USB 1".into();
        lib.color_names = ["Warm up", "Peak", "Orange", "Yellow", "Green", "Aqua", "Closing", "Purple"].iter().map(|s| s.to_string()).collect();
        lib.browse = Some(b2);
        Ok((vec![], files))
    })
    .unwrap();
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.device_name, "ADAM USB 1");
    assert_eq!(lib.color_names[0], "Warm up");
    assert_eq!(lib.color_names[6], "Closing");
    assert_eq!(lib.tracks.len(), 1);
    // The label is the third string of the present history row; the colour rows keep their layout.
    let db = Database::parse(std::fs::read(pdb_path(&root)).unwrap()).unwrap();
    let rows = db.rows(19);
    assert_eq!(rows.len(), 1);
    let r = db.row_bytes(rows[0].0, rows[0].1);
    assert_eq!(&r[12 + 11 + 2 + 5..12 + 11 + 2 + 5 + 11], &[[0x17u8].as_slice(), b"ADAM USB 1"].concat()[..]);
    let c = db.rows(6);
    let first = db.row_bytes(c[0].0, c[0].1);
    assert_eq!(&first[..8], &[0, 0, 0, 0, 1, 1, 0, 0]);
    assert_eq!(pdb::read_dstr(first, 8), "Warm up");
    // Category and sort tables hold the choice.
    assert_eq!(crate::device_ext::device_browse(&root), Some(browse));
    // The settings files are on the drive, valid, and read back by "Import My Settings".
    let (got, found) = read_my_settings(&root, &UsbSettings::default());
    assert_eq!(found.len(), 4);
    assert_eq!(got.get(SettingFile::Dev, DEV_WAVEFORM_COLOR), 3);
    assert_eq!(got.get(SettingFile::My, 10), 0x80);
    assert_eq!(got, settings);
    // A later export keeps the label and colour names (the request brings none of its own).
    export(request(&root, &tracks, vec![playlist(1, "Set", vec![1])]));
    let lib = read_device(&root).unwrap();
    assert_eq!((lib.device_name.as_str(), lib.color_names[1].as_str()), ("ADAM USB 1", "Peak"));
    // B8: an export with the user's colour names writes them to the colour table.
    let names: Vec<String> = ["A", "B", "C", "D", "E", "F", "G", "H"].iter().map(|s| s.to_string()).collect();
    export(ExportRequest { color_names: Some(names.clone()), device_name: Some("Renamed".into()), ..request(&root, &tracks, vec![playlist(1, "Set", vec![1])]) });
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.color_names, names);
    assert_eq!(lib.device_name, "Renamed");
    // Recordbox's own properties and "Delete My Settings".
    write_props(&root, &DeviceProps { bg_color: 5 }).unwrap();
    assert_eq!(read_props(&root).bg_color, 5);
    assert!(has_my_settings(&root));
    assert_eq!(delete_my_settings(&root).unwrap(), 3);
    assert!(!has_my_settings(&root) && root.join("PIONEER/DEVSETTING.DAT").is_file());
    assert_eq!(read_my_settings(&root, &UsbSettings::default()).1, ["DEVSETTING.DAT"]);
    // A damaged settings file is not imported.
    let mut bad = std::fs::read(root.join("PIONEER/DEVSETTING.DAT")).unwrap();
    bad[110] ^= 0xff;
    std::fs::write(root.join("PIONEER/DEVSETTING.DAT"), bad).unwrap();
    assert!(read_my_settings(&root, &UsbSettings::default()).1.is_empty());
}

#[test]
fn b5_an_imported_history_is_deleted_from_the_device() {
    let root = scratch("history");
    let tracks = demo_tracks(&src_dir(), 2);
    export(request(&root, &tracks, vec![playlist(1, "Set", vec![1, 2])]));
    change_device(&root, |lib| {
        let ids: Vec<u32> = lib.tracks.keys().copied().collect();
        lib.histories.push(DevHistory { id: 1, name: "HISTORY 001".into(), entries: ids.clone() });
        lib.histories.push(DevHistory { id: 2, name: "HISTORY 002".into(), entries: vec![ids[0]] });
        Ok(vec![])
    })
    .unwrap();
    let db = Database::parse(std::fs::read(pdb_path(&root)).unwrap()).unwrap();
    assert_eq!((db.rows(11).len(), db.rows(12).len()), (2, 3));
    delete_history(&root, 1).unwrap();
    let db = Database::parse(std::fs::read(pdb_path(&root)).unwrap()).unwrap();
    assert_eq!((db.rows(11).len(), db.rows(12).len()), (1, 1));
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.histories.iter().map(|h| (h.id, h.name.as_str(), h.entries.len())).collect::<Vec<_>>(), [(2, "HISTORY 002", 1)]);
    // Tracks, playlists and files are untouched.
    assert_eq!((lib.tracks.len(), lib.nodes.len(), count_files(&root, "Contents")), (2, 1, 2));
    assert!(delete_history(&root, 1).unwrap_err().to_string().contains("no longer on the device"));
    assert!(!root.join(".Recordbox/journal").exists());
}

#[test]
fn native_embeddings_stay_on_their_device_track() {
    let root = scratch("threeex");
    let rb = scratch("threeex_source");
    let mut tracks = demo_tracks(&rb, 1);
    let native = [
        b"\x81\xa9embedding".as_slice(),
        &[0x81, 0xa2, b'd', b'2', 7],
    ]
    .concat();
    let source_base = rb.join("ANLZ0000");
    std::fs::write(source_base.with_extension("3EX"), &native).unwrap();
    tracks.get_mut(&1).unwrap().rb_anlz = Some(source_base);
    let set = || vec![playlist(1, "Set", vec![1])];
    export(request(&root, &tracks, set()));

    let original = read_device(&root).unwrap();
    let track = original.tracks.values().next().unwrap();
    let base = root.join(track.analyze_path.trim_start_matches('/'));
    let fourth = base.with_extension("3EX");
    assert!(
        !fourth.exists(),
        "the desktop embedding has a different identity"
    );
    let owned = b"opaque existing device embedding";
    std::fs::write(&fourth, owned).unwrap();
    tracks.get_mut(&1).unwrap().track.rating = 4;
    export(request(&root, &tracks, set()));
    let updated = read_device(&root).unwrap();
    assert_eq!(updated.tracks[&track.id].rating, 4);
    assert_eq!(std::fs::read(&fourth).unwrap(), owned);

    let catalogs = [
        pdb_path(&root),
        base.clone(),
        base.with_extension("EXT"),
        base.with_extension("2EX"),
        fourth.clone(),
    ];
    let reject = |items: &HashMap<i64, ExportTrack>| {
        let before: Vec<_> = catalogs.iter().map(|p| std::fs::read(p).unwrap()).collect();
        let req = request(&root, items, set());
        let err = run_export(req, &|_| {}, &std::sync::atomic::AtomicBool::new(false)).unwrap_err();
        assert!(err.to_string().contains(".3EX"), "{err:#}");
        for (path, expected) in catalogs.iter().zip(&before) {
            assert_eq!(
                std::fs::read(path).unwrap(),
                *expected,
                "{}",
                path.display()
            );
        }
    };
    let mut shifted = tracks.clone();
    shifted.get_mut(&1).unwrap().grid.as_mut().unwrap().anchors[0].time += 0.025;
    reject(&shifted);

    // Native list BPM and the extended grid can each diverge without a
    // change to the DAT beat grid. Neither may bypass the embedding check.
    let catalog = pdb_path(&root);
    let original_bytes = std::fs::read(&catalog).unwrap();
    let mut other_bpm = updated.clone();
    other_bpm.tracks.get_mut(&track.id).unwrap().bpm100 += 1;
    std::fs::write(&catalog, build_export_pdb(&other_bpm)).unwrap();
    reject(&tracks);
    std::fs::write(&catalog, original_bytes).unwrap();
    let ext = base.with_extension("EXT");
    let original_bytes = std::fs::read(&ext).unwrap();
    let mut other_grid = original_bytes.clone();
    let start = other_grid.windows(4).position(|tag| tag == b"PQT2").unwrap();
    let size = u32::from_be_bytes(other_grid[start + 8..start + 12].try_into().unwrap()) as usize;
    other_grid[start + size - 1] ^= 1;
    std::fs::write(&ext, other_grid).unwrap();
    reject(&tracks);
    std::fs::write(&ext, original_bytes).unwrap();

    // A same-size source replacement invalidates the old audio features too.
    let source = Path::new(&tracks[&1].track.path);
    let mut audio = std::fs::read(source).unwrap();
    *audio.last_mut().unwrap() ^= 1;
    std::fs::write(source, &audio).unwrap();
    reject(&tracks);
    assert_ne!(
        std::fs::read(root.join(track.file_path.trim_start_matches('/'))).unwrap(),
        audio
    );
    assert!(unused_files(&root, &updated).is_empty());
    delete_tracks(&root, &[track.id]).unwrap();
    assert!(!fourth.exists());
}
