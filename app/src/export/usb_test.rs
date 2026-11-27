//! End-to-end export test: two synthetic tracks into a folder inside target/.

use super::usb::*;
use super::usb::{CRASH, FAIL_AT, STEP};
use crate::model::*;
use std::collections::HashMap;

#[test]
fn full_export_to_folder() {
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("export_test");
    let run = chrono::Local::now().format("%H%M%S%3f").to_string();
    let root = base.join(format!("usb_{run}"));
    std::fs::create_dir_all(&root).unwrap();
    let src_dir = base.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let mut tracks = HashMap::new();
    for (i, d) in crate::demo::DEMOS.iter().take(2).enumerate() {
        let p = src_dir.join(format!("{}.wav", d.name));
        if !p.exists() {
            crate::demo::write_wav(&p, 44100, &crate::demo::synth(d, 44100)).unwrap();
        }
        let mut t = crate::importer::read_track(&p);
        t.id = i as i64 + 1;
        t.artist = "Test Artist".into();
        t.album = if i == 0 { "Album: One?".into() } else { String::new() };
        t.key = 7;
        let r = crate::analysis::analyze_file(&t.path, &Default::default()).unwrap();
        t.bpm = r.bpm;
        let cues = vec![
            Cue { id: 1, track_id: t.id, kind: CueKind::Hot, slot: 0, start: 1.0, end: None, color: 43, comment: "Intro".into(), active_loop: false },
            Cue { id: 2, track_id: t.id, kind: CueKind::Memory, slot: -1, start: 8.0, end: Some(12.0), color: 3, comment: String::new(), active_loop: false },
        ];
        tracks.insert(t.id, ExportTrack { track: t, cues, grid: r.grid, wave: Some(r.wave), tags: vec![("Genre".into(), "Techno".into())], rb_anlz: None, grid_same: false, vocal: None, phrases: None });
    }
    let nodes = vec![ExportNode { name: "Sets".into(), folder: true, children: vec![ExportNode { name: "Friday".into(), folder: false, children: vec![], tracks: vec![2, 1], ..Default::default() }], tracks: vec![], ..Default::default() }];
    let req = ExportRequest { root: root.clone(), nodes: nodes.clone(), tracks: tracks.clone(), tag_categories: vec![("Genre".into(), vec!["Techno".into(), "House".into()])], skipped: vec![], analysis: Default::default(), settings: Default::default(), browse: Default::default(), phrases: true, ..Default::default() };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let msg = run_export(req, &|_| {}, &cancel).unwrap();
    assert!(msg.contains("2 tracks"), "{msg}");
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.tracks.len(), 2);
    let fri = lib.nodes.iter().find(|n| n.name == "Friday").unwrap();
    assert_eq!(fri.entries.len(), 2);
    for t in lib.tracks.values() {
        assert!(root.join(t.file_path.trim_start_matches('/')).exists(), "{}", t.file_path);
        assert!(root.join(t.analyze_path.trim_start_matches('/')).exists());
        assert_eq!(t.tags, vec![tag_key("Genre", "Techno")]);
        assert!(!t.source.is_empty());
    }
    assert!(lib.tracks.values().any(|t| t.file_path.starts_with("/Contents/Test Artist/Album_ One_/")));
    assert!(lib.tracks.values().any(|t| t.file_path.starts_with("/Contents/Test Artist/UnknownAlbum/")));
    // Exporting the same playlist again updates in place instead of duplicating.
    let req2 = ExportRequest { root: root.clone(), nodes, tracks, tag_categories: vec![], skipped: vec![], analysis: Default::default(), settings: Default::default(), browse: Default::default(), phrases: true, ..Default::default() };
    run_export(req2, &|_| {}, &cancel).unwrap();
    let lib2 = read_device(&root).unwrap();
    assert_eq!(lib2.tracks.len(), 2);
    assert_eq!(lib2.nodes.len(), 2);
    // Player settings files are written with valid checksums.
    for name in ["MYSETTING.DAT", "MYSETTING2.DAT", "DEVSETTING.DAT", "DJMMYSETTING.DAT"] {
        let b = std::fs::read(root.join("PIONEER").join(name)).unwrap();
        let n = u32::from_le_bytes(b[100..104].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(b[104 + n..108 + n].try_into().unwrap()) as u16;
        let covered = if name == "DJMMYSETTING.DAT" { &b[..104 + n] } else { &b[104..104 + n] };
        assert_eq!(crc, super::settings::crc16_xmodem(covered), "{name}");
    }
    // Re-export keeps each track's analysis path and the My Tag IDs, and backs up the old DB.
    for (id, t) in &lib.tracks {
        assert_eq!(lib2.tracks[id].analyze_path, t.analyze_path);
    }
    assert_eq!(lib2.tag_ids, lib.tag_ids);
    assert!(std::fs::read_dir(root.join(".Recordbox/backup")).unwrap().count() >= 2);
    // No temporary files are left behind.
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            assert!(!n.ends_with(".part") && !n.ends_with(".tmp_write"), "left over: {n}");
            if e.path().is_dir() {
                stack.push(e.path());
            }
        }
    }
    std::fs::write(base.join("LAST"), root.to_string_lossy().as_bytes()).unwrap();
}

pub(super) fn demo_tracks(src_dir: &std::path::Path, n: usize) -> HashMap<i64, ExportTrack> {
    std::fs::create_dir_all(src_dir).unwrap();
    let mut tracks = HashMap::new();
    for (i, d) in crate::demo::DEMOS.iter().take(n).enumerate() {
        let p = src_dir.join(format!("{}.wav", d.name));
        if !p.exists() {
            crate::demo::write_wav(&p, 44100, &crate::demo::synth(d, 44100)).unwrap();
        }
        let mut t = crate::importer::read_track(&p);
        t.id = i as i64 + 1;
        t.artist = "Journal Artist".into();
        let r = crate::analysis::analyze_file(&t.path, &Default::default()).unwrap();
        t.bpm = r.bpm;
        let cues = vec![Cue { id: 1, track_id: t.id, kind: CueKind::Hot, slot: 0, start: 1.0, end: None, color: 43, comment: "Intro".into(), active_loop: false }];
        tracks.insert(t.id, ExportTrack { track: t, cues, grid: r.grid, wave: Some(r.wave), tags: vec![], rb_anlz: None, grid_same: false, vocal: None, phrases: None });
    }
    tracks
}

pub(super) fn copy_dir(a: &std::path::Path, b: &std::path::Path) {
    std::fs::create_dir_all(b).unwrap();
    for e in std::fs::read_dir(a).unwrap().flatten() {
        let to = b.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), to).unwrap();
        }
    }
}

/// Every file on the drive outside Recordbox's own folder, with its contents.
fn snapshot(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().to_string();
            // Recordbox's own folder: only the files a rollback must restore are compared.
            if rel.starts_with(".Recordbox") && rel != ".Recordbox/device.json" && rel != ".Recordbox/cues.json" && !rel.starts_with(".Recordbox/journal") && e.path().is_file() {
                continue;
            }
            if rel.starts_with(".Recordbox/backup") || rel.starts_with(".Recordbox/journal") {
                continue;
            }
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                out.insert(rel, std::fs::read(e.path()).unwrap());
            }
        }
    }
    out
}

/// USB-005: an export cut short at any journal step (as if the program died there) is either
/// undone or finished by recovery: the drive is byte-for-byte the "before" or the "after".
#[test]
fn interrupted_export_recovers_to_before_or_after() {
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("export_test");
    let run = format!("journal_{}", chrono::Local::now().format("%H%M%S%3f"));
    let work = base.join(&run);
    let tracks2 = demo_tracks(&base.join("src"), 3);
    let tracks1: HashMap<i64, ExportTrack> = tracks2.iter().filter(|(id, _)| **id <= 2).map(|(k, v)| (*k, v.clone())).collect();
    let req = |root: &std::path::Path, tracks: &HashMap<i64, ExportTrack>, ids: Vec<i64>| ExportRequest {
        root: root.to_path_buf(),
        nodes: vec![ExportNode { name: "Set".into(), folder: false, children: vec![], tracks: ids, ..Default::default() }],
        tracks: tracks.clone(),
        tag_categories: vec![],
        skipped: vec![],
        analysis: Default::default(),
        settings: Default::default(),
        browse: Default::default(),
        phrases: true,
        // This test replays every journal failure on the same songs. Keep real PSSI
        // generation here; full_export_to_folder exercises the default estimator.
        classic_phrase_estimator: true,
        ..Default::default()
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let initial = work.join("initial");
    std::fs::create_dir_all(&initial).unwrap();
    run_export(req(&initial, &tracks1, vec![1, 2]), &|_| {}, &cancel).unwrap();
    // The second export moves a cue (rewrites existing analysis files) and adds a track.
    let mut tracks2 = tracks2;
    tracks2.get_mut(&1).unwrap().cues[0].start = 2.5;
    let after_dir = work.join("after");
    copy_dir(&initial, &after_dir);
    run_export(req(&after_dir, &tracks2, vec![1, 2, 3]), &|_| {}, &cancel).unwrap();
    let (before, after) = (snapshot(&initial), snapshot(&after_dir));
    assert_ne!(before, after);
    let (mut undone, mut finished) = (0, 0);
    for n in 1.. {
        let d = work.join(format!("crash_{n}"));
        copy_dir(&initial, &d);
        STEP.with(|s| s.set(0));
        FAIL_AT.with(|f| f.set(n));
        CRASH.with(|c| c.set(true));
        let r = run_export(req(&d, &tracks2, vec![1, 2, 3]), &|_| {}, &cancel);
        CRASH.with(|c| c.set(false));
        FAIL_AT.with(|f| f.set(0));
        if r.is_ok() {
            assert_eq!(snapshot(&d), after);
            break;
        }
        assert!(recover_device(&d).is_some(), "step {n}: no journal to recover from");
        let snap = snapshot(&d);
        if snap == before {
            undone += 1;
        } else if snap == after {
            finished += 1;
        } else {
            let diff: Vec<&String> = before.keys().chain(after.keys()).filter(|k| snap.get(*k) != before.get(*k) && snap.get(*k) != after.get(*k)).collect();
            panic!("step {n}: drive is neither before nor after; differing: {diff:?}");
        }
        assert!(!journal_exists(&d));
        let _ = std::fs::remove_dir_all(&d);
        assert!(n < 500);
    }
    assert!(undone > 0 && finished > 0, "undone {undone}, finished {finished}");
    eprintln!("journal test: {undone} crash points undone, {finished} finished");
    // The in-process path: a plain failure (no crash) also leaves the drive as before.
    let d = work.join("fail");
    copy_dir(&initial, &d);
    STEP.with(|s| s.set(0));
    FAIL_AT.with(|f| f.set(5));
    assert!(run_export(req(&d, &tracks2, vec![1, 2, 3]), &|_| {}, &cancel).is_err());
    FAIL_AT.with(|f| f.set(0));
    assert_eq!(snapshot(&d), before);
    let _ = std::fs::remove_dir_all(&work);
}

fn journal_exists(root: &std::path::Path) -> bool {
    root.join(".Recordbox/journal").exists()
}

/// Run journal recovery on a drive given in PREPDECK_RECOVER (manual check). --ignored.
#[test]
#[ignore]
fn recover_given_drive() {
    let Ok(p) = std::env::var("PREPDECK_RECOVER") else { return };
    println!("{:?}", recover_device(std::path::Path::new(&p)));
    let lib = read_device(std::path::Path::new(&p)).unwrap();
    println!("tracks {}, playlists {:?}", lib.tracks.len(), lib.nodes.iter().map(|n| (n.name.clone(), n.entries.len())).collect::<Vec<_>>());
}

/// A track whose file is missing on this computer keeps its entry in the drive's playlist on
/// re-export when the drive still has its copy (it is not dropped from the playlist).
#[test]
fn missing_local_file_keeps_drive_copy_in_playlists() {
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("export_test");
    let work = base.join(format!("missing_{}", chrono::Local::now().format("%H%M%S%3f")));
    let src = work.join("src");
    std::fs::create_dir_all(&src).unwrap();
    // Own copies of two demo files, so one can be removed.
    let mut tracks = demo_tracks(&base.join("src"), 2);
    for et in tracks.values_mut() {
        let to = src.join(std::path::Path::new(&et.track.path).file_name().unwrap());
        std::fs::copy(&et.track.path, &to).unwrap();
        et.track.path = to.to_string_lossy().to_string();
    }
    let root = work.join("usb");
    std::fs::create_dir_all(&root).unwrap();
    let req = |tracks: &HashMap<i64, ExportTrack>| ExportRequest {
        root: root.clone(),
        nodes: vec![ExportNode { name: "Set".into(), folder: false, children: vec![], tracks: vec![1, 2], ..Default::default() }],
        tracks: tracks.clone(),
        tag_categories: vec![],
        skipped: vec![],
        analysis: Default::default(),
        settings: Default::default(),
        browse: Default::default(),
        phrases: false,
        ..Default::default()
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    run_export(req(&tracks), &|_| {}, &cancel).unwrap();
    std::fs::remove_file(&tracks[&1].track.path).unwrap();
    let msg = run_export(req(&tracks), &|_| {}, &cancel).unwrap();
    assert!(!msg.contains("not exported"), "{msg}");
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.nodes.iter().find(|n| n.name == "Set").unwrap().entries.len(), 2);
    let _ = std::fs::remove_dir_all(&work);
}
