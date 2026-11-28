//! Regression tests for the audited USB export defects (A1–A13 of the 2026-11 device pass),
//! byte-level against real rekordbox exports where one exists. Drives are folders under
//! target/export_test.

use super::anlz;
use super::pdb::{self, Database};
use super::usb::*;
use super::usb_test::demo_tracks;
use crate::model::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn oracles() -> PathBuf {
    let path = std::env::var_os("RECORDBOX_TEST_ORACLES")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .expect("set RECORDBOX_TEST_ORACLES to a rekordcrate checkout containing data/ before running ignored oracle tests");
    assert!(path.is_dir(), "oracle directory does not exist: {}", path.display());
    path
}
fn oracle_bytes(relative: &str) -> Vec<u8> {
    let path = oracles().join(relative);
    std::fs::read(&path).unwrap_or_else(|e| panic!("could not read oracle {}: {e}", path.display()))
}
fn demo_pdb() -> Vec<u8> {
    oracle_bytes("data/complete_export/demo_tracks/PIONEER/rekordbox/export.pdb")
}
fn num_rows_pdb() -> Vec<u8> {
    oracle_bytes("data/pdb/num_rows/export.pdb")
}
fn scratch(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("export_test");
    let root = base.join(format!("{name}_{}_{}", chrono::Local::now().format("%H%M%S%3f"), std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}
fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("export_test").join("src")
}
fn request(root: &Path, tracks: &HashMap<i64, ExportTrack>, nodes: Vec<ExportNode>) -> ExportRequest {
    ExportRequest { root: root.to_path_buf(), nodes, tracks: tracks.clone(), ..Default::default() }
}
fn playlist(id: i64, name: &str, tracks: Vec<i64>) -> ExportNode {
    ExportNode { id, name: name.into(), tracks, ..Default::default() }
}
fn export(req: ExportRequest) -> String {
    run_export(req, &|_| {}, &std::sync::atomic::AtomicBool::new(false)).unwrap()
}
fn picture(path: &Path, shade: u8) {
    let img = image::RgbImage::from_fn(64, 64, |x, y| image::Rgb([shade, (x * 4) as u8, (y * 4) as u8]));
    img.save(path).unwrap();
}
/// The 21 string slots of every track row, as stored bytes, by track ID.
fn track_slot_bytes(db: &Database, slot: usize) -> HashMap<u32, Vec<u8>> {
    let mut out = HashMap::new();
    for (h, o) in db.rows(0) {
        let r = db.row_bytes(h, o);
        let at = pdb::rd_u16(r, 0x5e + 2 * slot) as usize;
        let len = match r[at] {
            x if x & 1 == 1 => (x >> 1) as usize,
            _ => pdb::rd_u16(r, at + 1) as usize,
        };
        out.insert(pdb::rd_u32(r, 0x48), r[at..at + len].to_vec());
    }
    out
}

// ---- A1 ------------------------------------------------------------------------------------

#[test]
fn a1_over_long_strings_are_cut_instead_of_breaking_the_database() {
    let mut lib = DeviceLibrary::default();
    let long_comment = "é".repeat(70_000);
    let t = DevTrack { id: 1, title: "T".repeat(5_000), comment: long_comment.clone(), mix_name: "m".repeat(9_000), message: "ü".repeat(9_000), lyricist: "l".repeat(900), artist: "A".repeat(4_000), album: "ß".repeat(4_000), genre: "g".repeat(700), label: "L".repeat(700), file_path: "/Contents/A/B/x.mp3".into(), analyze_path: "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT".into(), ..Default::default() };
    lib.tracks.insert(1, t);
    lib.nodes.push(DevNode { id: 1, parent: 0, name: "p".repeat(3_000), folder: false, seq: 0, entries: vec![1] });
    let bytes = try_build_export_pdb(&lib).expect("a long comment must not fail the export");
    let back = parse_pdb(bytes).unwrap();
    let b = &back.tracks[&1];
    // Cut at the longest length seen in a real export (255 characters), never mid-character.
    assert_eq!(b.comment, "é".repeat(pdb::MAX_STRING_CHARS));
    assert_eq!(b.title.len(), 255);
    assert_eq!(b.artist.len(), 255);
    assert_eq!(b.album.chars().count(), 255);
    assert_eq!(b.file_path, "/Contents/A/B/x.mp3");
    assert_eq!(back.nodes[0].name.len(), 255);
    assert_eq!(back.nodes[0].entries, vec![1]);
    // The 16-bit length of the long forms can't wrap any more.
    let e = pdb::dstr(&"x".repeat(70_000));
    assert_eq!(u16::from_le_bytes([e[1], e[2]]) as usize, e.len());
    let e = pdb::dstr_ucs2(&"😀".repeat(40_000));
    assert_eq!(u16::from_le_bytes([e[1], e[2]]) as usize, e.len());
    assert_eq!(pdb::read_dstr(&e, 0).chars().count(), 255);
}

#[test]
#[ignore = "requires RECORDBOX_TEST_ORACLES fixtures"]
fn oracle_a1_real_export_longest_comment_is_255_characters() {
    let lib = parse_pdb(num_rows_pdb()).unwrap();
    assert_eq!(lib.tracks.values().map(|t| t.comment.chars().count()).max(), Some(255));
}

#[test]
fn a1_a_row_too_big_for_a_page_is_an_error_not_a_panic() {
    assert_eq!(pdb::MAX_ROW_BYTES, 4048);
    // 4049 and 4050 fit before padding, but their aligned 4052 bytes overlap the directory.
    for len in [4049, 4050, 4051, 4052, pdb::PAGE] {
        let rows = vec![vec![0u8; len]];
        let r = pdb::write_database(&[pdb::TableSpec { page_type: 0, rows, ..Default::default() }], 5, 3);
        assert!(r.unwrap_err().to_string().contains("more than a database page holds"), "{len}");
    }
    // The largest row that fits is written and read back.
    for len in [4045, 4046, 4047, 4048] {
        let rows = vec![vec![7u8; len]];
        let db = Database::parse(pdb::write_database(&[pdb::TableSpec { page_type: 0, rows, ..Default::default() }], 5, 3).unwrap()).unwrap();
        let rows = db.rows(0);
        assert_eq!(rows.len(), 1);
        let start = rows[0].0 + rows[0].1;
        assert_eq!(&db.bytes[start..start + len], vec![7u8; len]);
    }
}

#[test]
fn a1_aligned_rows_paginate_at_row_group_boundaries_without_overlap() {
    // Crossing 16 rows adds another row-directory group as well as alignment padding.
    for len in [117, 245, 249] {
        for count in [15, 16, 17, 32, 33] {
            let rows = (0..count).map(|i| vec![i as u8 + 1; len]).collect();
            let bytes = pdb::write_database(&[pdb::TableSpec { page_type: 0, rows, ..Default::default() }], 5, 3).unwrap();
            let db = Database::parse(bytes).unwrap();
            let rows = db.rows(0);
            assert_eq!(rows.len(), count);
            for (i, &(heap, offset)) in rows.iter().enumerate() {
                let start = heap + offset;
                assert_eq!(&db.bytes[start..start + len], vec![i as u8 + 1; len]);
            }
        }
    }
}

#[test]
fn a1_a_panic_in_the_export_thread_is_reported_as_a_failure() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = run_guarded(|| panic!("row does not fit"));
    std::panic::set_hook(hook);
    let e = r.unwrap_err();
    assert!(e.contains("internal error") && e.contains("row does not fit"), "{e}");
    assert_eq!(run_guarded(|| Ok("fine".into())).unwrap(), "fine");
    // Through the job: the window receives Finished(Err) instead of waiting for ever.
    let job = crate::jobs::spawn_export(ExportRequest { root: PathBuf::from("/nonexistent/recordbox-test-device"), ..Default::default() }, eframe::egui::Context::default());
    let mut finished = None;
    while let Ok(p) = job.rx.recv_timeout(std::time::Duration::from_secs(20)) {
        if let Progress::Finished(r) = p {
            finished = Some(r);
            break;
        }
    }
    assert!(finished.expect("the export job always reports an end").is_err());
}

#[test]
fn a1_export_with_a_huge_comment_completes() {
    let root = scratch("longcomment");
    let mut tracks = demo_tracks(&src_dir(), 1);
    let t = tracks.get_mut(&1).unwrap();
    t.track.comment = "Très long commentaire. ".repeat(5_000);
    t.track.duration = 12.9;
    t.track.date_added = "２０２４年１月２日 added".into();
    t.track.release_date = "2024/3/5".into();
    t.track.key_text = "7A".into();
    export(request(&root, &tracks, vec![playlist(1, "Set", vec![1])]));
    let lib = read_device(&root).unwrap();
    let d = lib.tracks.values().next().unwrap();
    assert_eq!(d.comment.chars().count(), 255);
    // A8: whole seconds cut, dates normalised, multi-byte date text no longer panics, the key
    // as the library spelled it.
    assert_eq!(d.duration, 12);
    assert_eq!(d.release_date, "2024-03-05");
    assert_eq!(d.date_added.len(), 10);
    assert_eq!(d.key_name, "7A");
    assert_eq!((d.raw_strings[2].as_str(), d.raw_strings[3].as_str(), d.raw_strings[6].as_str()), ("1", "1", "ON"));
}

// ---- A2 ------------------------------------------------------------------------------------

/// (row bytes, present) of every row of a table's data pages, in page and offset order.
fn all_rows(db: &Database, table: u32) -> Vec<(Vec<u8>, bool)> {
    let mut out = vec![];
    for pg in db.data_pages(table) {
        for (off, len, present) in pdb::page_rows(&pg) {
            out.push((pg[0x28 + off..0x28 + off + len].to_vec(), present));
        }
    }
    out
}

#[test]
#[ignore = "requires RECORDBOX_TEST_ORACLES fixtures"]
fn oracle_a2_rewrite_keeps_unmanaged_tables_and_history_rows() {
    for bytes in [demo_pdb(), num_rows_pdb()] {
        let orig = Database::parse(bytes.clone()).unwrap();
        let mut lib = parse_pdb(bytes).unwrap();
        // No real export at hand has rows in tables 9, 10, 14 or 15, so pages are planted there
        // the way a newer rekordbox might leave them: rows, a deleted row, odd flags.
        for (k, table) in KEPT_TABLES.into_iter().enumerate() {
            let mut pg = pdb::blank_page(table);
            for i in 0..(3 + k) {
                let row: Vec<u8> = (0..(20 + 4 * i)).map(|b| (b as u8) ^ (table as u8) ^ (i as u8 * 31)).collect();
                assert!(pdb::append_row(&mut pg, &row, i == 2));
            }
            pg[16..20].copy_from_slice(&(900 + table).to_le_bytes());
            lib.raw_tables.insert(table, vec![pg]);
        }
        let planted = lib.raw_tables.clone();
        let out1 = build_export_pdb(&lib);
        let db1 = Database::parse(out1.clone()).unwrap();
        for table in KEPT_TABLES {
            let got = db1.data_pages(table);
            assert_eq!(got.len(), 1, "table {table}");
            // Byte for byte apart from the page's own number and link.
            assert_eq!(got[0][..4], planted[&table][0][..4]);
            assert_eq!(got[0][16..], planted[&table][0][16..], "table {table} page content");
            assert_eq!(pdb::rd_u32(&got[0], 8), table);
            assert_eq!(all_rows(&db1, table).iter().filter(|r| r.1).count(), 1 + k_rows_present(table), "table {table}");
        }
        // Table 19: every row the drive had is still there, byte for byte and in order, now
        // flagged deleted, and one new present row follows.
        let before = all_rows(&orig, 19);
        let after = all_rows(&db1, 19);
        assert_eq!(after.len(), before.len() + 1);
        for (a, b) in before.iter().zip(&after) {
            assert_eq!(a.0, b.0);
            assert!(!b.1, "an earlier history row is still flagged present");
        }
        let (new, present) = after.last().unwrap();
        assert!(*present);
        assert_eq!(&new[..2], &[0x80, 0x02]);
        assert_eq!(pdb::rd_u32(new, 4) as usize, lib.tracks.len());
        assert_eq!(db1.rows(19).len(), 1);
        // The device label of the present row is carried into the new one.
        let lib1 = parse_pdb(out1).unwrap();
        assert_eq!(lib1.device_name, lib.device_name);
        assert_eq!(lib1.raw_tables[&14][0][16..], planted[&14][0][16..]);
        // And again: a second rewrite keeps all of those and adds one more.
        let db2 = Database::parse(build_export_pdb(&lib1)).unwrap();
        let again = all_rows(&db2, 19);
        assert_eq!(again.len(), after.len() + 1);
        assert!(again[..after.len()].iter().zip(&after).all(|(x, y)| x.0 == y.0 && !x.1));
        assert_eq!(db2.rows(19).len(), 1);
        assert_eq!(db2.rows(0).len(), orig.rows(0).len());
    }
}
fn k_rows_present(_table: u32) -> usize {
    // Planted pages: the third row retired the two before it, later rows were added beside it.
    // Rows present = total - 2, and total = 3 + k; "1 + this" is compared, so return total - 3.
    KEPT_TABLES.iter().position(|t| *t == _table).unwrap()
}

#[test]
#[ignore = "requires RECORDBOX_TEST_ORACLES fixtures"]
fn oracle_a2_device_label_of_a_real_export_is_read_and_kept() {
    let lib = parse_pdb(num_rows_pdb()).unwrap();
    assert_eq!(lib.device_name, "ZAKS BACKUP");
    let back = parse_pdb(build_export_pdb(&lib)).unwrap();
    assert_eq!(back.device_name, "ZAKS BACKUP");
    assert_eq!(back.color_names, vec!["Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple"]);
}

// ---- A3 ------------------------------------------------------------------------------------

fn files_under(root: &Path, dir: &str) -> Vec<String> {
    let mut out = vec![];
    let mut stack = vec![root.join(dir)];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                out.push(e.path().strip_prefix(root).unwrap().to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a3_deleting_a_playlist_with_its_tracks_cleans_up_everything_and_respects_onelibrary() {
    let root = scratch("remove");
    let mut tracks = demo_tracks(&src_dir(), 3);
    let art = src_dir().join("a3-cover.png");
    picture(&art, 200);
    for t in tracks.values_mut() {
        t.track.artwork = art.to_string_lossy().to_string();
    }
    export(request(&root, &tracks, vec![playlist(1, "Keep", vec![1]), playlist(2, "Drop", vec![1, 2, 3])]));
    // A play history that names the tracks, as a player would leave it.
    change_device(&root, |lib| {
        let ids: Vec<u32> = lib.tracks.keys().copied().collect();
        lib.histories.push(DevHistory { id: 1, name: "HISTORY 001".into(), entries: ids });
        Ok(vec![])
    })
    .unwrap();
    let before = read_device(&root).unwrap();
    assert_eq!(before.histories[0].entries.len(), 3);
    assert_eq!(files_under(&root, "PIONEER/Artwork").len(), 2, "one shared picture: a1.jpg and a1_m.jpg");
    let drop = before.nodes.iter().find(|n| n.name == "Drop").unwrap().id;
    let kept_id = before.nodes.iter().find(|n| n.name == "Keep").unwrap().entries[0];

    // With a OneLibrary catalogue on the drive nothing is deleted from disk.
    let guarded = scratch("remove_onelib");
    super::usb_test::copy_dir(&root, &guarded);
    std::fs::write(guarded.join("PIONEER/rekordbox/exportLibrary.db"), b"not ours").unwrap();
    let files = (files_under(&guarded, "Contents"), files_under(&guarded, "PIONEER/USBANLZ"), files_under(&guarded, "PIONEER/Artwork"));
    remove_node(&guarded, drop, true).unwrap();
    assert_eq!(read_device(&guarded).unwrap().tracks.len(), 1);
    assert_eq!(files, (files_under(&guarded, "Contents"), files_under(&guarded, "PIONEER/USBANLZ"), files_under(&guarded, "PIONEER/Artwork")));
    assert_eq!(std::fs::read(guarded.join("PIONEER/rekordbox/exportLibrary.db")).unwrap(), b"not ours");

    remove_node(&root, drop, true).unwrap();
    let after = read_device(&root).unwrap();
    assert_eq!(after.tracks.keys().copied().collect::<Vec<_>>(), vec![kept_id]);
    // History entries of the removed tracks are gone; the kept track's entry stays.
    assert_eq!(after.histories[0].entries, vec![kept_id]);
    // Files: one audio file, one analysis set, and the artwork (still shown by the kept track).
    assert_eq!(files_under(&root, "Contents").len(), 1);
    assert_eq!(files_under(&root, "PIONEER/USBANLZ").len(), 3);
    assert_eq!(files_under(&root, "PIONEER/Artwork").len(), 2);
    assert!(unused_files(&root, &after).is_empty(), "{:?}", unused_files(&root, &after));
    assert!(!root.join(".Recordbox/journal").exists());
    // Removing the last track takes the artwork with it.
    assert_eq!(delete_tracks(&root, &[kept_id]).unwrap(), 1);
    let empty = read_device(&root).unwrap();
    assert!(empty.tracks.is_empty() && empty.histories[0].entries.is_empty());
    assert!(empty.nodes.iter().all(|n| n.entries.is_empty()));
    assert!(files_under(&root, "Contents").is_empty() && files_under(&root, "PIONEER/USBANLZ").is_empty() && files_under(&root, "PIONEER/Artwork").is_empty());
}

#[test]
fn a3_deletions_listed_in_the_journal_are_finished_or_dropped_with_it() {
    for published in [true, false] {
        let root = scratch("journal_d");
        std::fs::create_dir_all(root.join("Contents/A/B")).unwrap();
        std::fs::create_dir_all(root.join(".Recordbox/journal")).unwrap();
        std::fs::write(root.join("Contents/A/B/x.wav"), b"audio").unwrap();
        std::fs::write(root.join(".Recordbox/journal/journal.log"), format!("B\nD\tContents/A/B/x.wav\n{}", if published { "P\n" } else { "" })).unwrap();
        assert!(recover_device(&root).is_some());
        // After "P" the listed deletion is carried out (and the emptied folders go); before
        // it the change never happened, so the file stays.
        assert_eq!(root.join("Contents/A/B/x.wav").exists(), !published);
        assert_eq!(root.join("Contents/A").exists(), !published);
        assert!(root.join("Contents").exists());
    }
}

// ---- A4 ------------------------------------------------------------------------------------

#[test]
fn a4_a_track_rekordbox_exported_is_updated_in_place() {
    let root = scratch("foreign");
    let tracks = demo_tracks(&src_dir(), 2);
    export(request(&root, &tracks, vec![playlist(1, "Set", vec![1, 2])]));
    let first = read_device(&root).unwrap();
    // What a rekordbox-made drive looks like to Recordbox: the same rows and files, but no
    // record of which local file each came from.
    for f in ["device.json", "nodes.json", "artwork.json", "cues.json"] {
        let _ = std::fs::remove_file(root.join(".Recordbox").join(f));
    }
    assert!(read_device(&root).unwrap().tracks.values().all(|t| t.source.is_empty()));
    let (audio, anlz) = (files_under(&root, "Contents"), files_under(&root, "PIONEER/USBANLZ"));
    export(request(&root, &tracks, vec![playlist(1, "Set", vec![2, 1])]));
    let second = read_device(&root).unwrap();
    assert_eq!(second.tracks.keys().collect::<Vec<_>>(), first.tracks.keys().collect::<Vec<_>>(), "no duplicate rows");
    assert_eq!(files_under(&root, "Contents"), audio, "no \"name-1.wav\" copy");
    assert_eq!(files_under(&root, "PIONEER/USBANLZ"), anlz, "no second analysis set");
    for (id, t) in &first.tracks {
        assert_eq!(second.tracks[id].file_path, t.file_path);
        assert_eq!(second.tracks[id].analyze_path, t.analyze_path);
        assert!(!second.tracks[id].source.is_empty());
        // The row's export counter went up by one.
        assert_eq!(second.tracks[id].raw_strings[2], "2");
    }
    // A different file that merely has the same name and folder (another size) is not taken
    // for it: it gets its own row and a "-1" name.
    let mut other = tracks.clone();
    let t = other.get_mut(&1).unwrap();
    let copy = src_dir().join("a4").join(Path::new(&t.track.path).file_name().unwrap());
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    let mut bytes = std::fs::read(&t.track.path).unwrap();
    bytes.extend_from_slice(&[0u8; 2048]);
    std::fs::write(&copy, bytes).unwrap();
    t.track.path = copy.to_string_lossy().to_string();
    t.track.id = 77;
    let one: HashMap<i64, ExportTrack> = [(77, other.remove(&1).unwrap())].into();
    export(request(&root, &one, vec![playlist(2, "Other", vec![77])]));
    assert_eq!(read_device(&root).unwrap().tracks.len(), 3);
}

// ---- A5 ------------------------------------------------------------------------------------

#[test]
fn a5_isrc_strings_round_trip_byte_for_byte() {
    assert_eq!(pdb::dstr_isrc("GBAYE6700149"), [&[0x90, 0x12, 0x00, 0x00, 0x03][..], b"GBAYE6700149", &[0x00]].concat());
    assert_eq!(pdb::read_dstr(&pdb::dstr_isrc("GBAYE6700149"), 0), "GBAYE6700149");
    assert_eq!(pdb::dstr_isrc(""), vec![0x03]);
    // Ordinary UTF-16 text is not mistaken for an ISRC.
    assert!(!pdb::is_isrc_form(&pdb::dstr_ucs2("Ünïcødé"), 0));
}

#[test]
#[ignore = "requires RECORDBOX_TEST_ORACLES fixtures"]
fn oracle_a5_real_isrc_and_string_slots_round_trip_byte_for_byte() {
    let bytes = num_rows_pdb();
    let orig = Database::parse(bytes.clone()).unwrap();
    let lib = parse_pdb(bytes).unwrap();
    let ours = Database::parse(build_export_pdb(&lib)).unwrap();
    let (a, b) = (track_slot_bytes(&orig, 0), track_slot_bytes(&ours, 0));
    let with_isrc: Vec<&u32> = a.iter().filter(|(_, v)| v.len() > 1).map(|(k, _)| k).collect();
    assert!(with_isrc.len() > 500, "the real export has {} ISRCs", with_isrc.len());
    for id in with_isrc {
        assert_eq!(a[id][0], 0x90);
        assert_eq!(a[id], b[id], "ISRC bytes of track {id}");
        assert!(lib.tracks[id].raw_strings[0].is_ascii());
    }
    assert_eq!(a.len(), b.len());
    // Every other string slot of the real rows also reads back as the same text, and the
    // long forms start on a 4-byte boundary as in the original.
    let back = parse_pdb(ours.bytes.clone()).unwrap();
    for (id, t) in &lib.tracks {
        assert_eq!(t.raw_strings, back.tracks[id].raw_strings, "track {id}");
    }
    for (h, o) in ours.rows(0) {
        let r = ours.row_bytes(h, o);
        for i in 0..21 {
            let at = pdb::rd_u16(r, 0x5e + 2 * i) as usize;
            if r[at] & 1 == 0 {
                assert_eq!(at % 4, 0);
            }
        }
    }
}

// ---- A6 ------------------------------------------------------------------------------------

fn real_anlz_bases() -> Vec<PathBuf> {
    let mut v = vec![];
    let demo = oracles().join("data/complete_export/demo_tracks/PIONEER/USBANLZ");
    for p in ["P016/0000875E/ANLZ0000", "P053/0001D21F/ANLZ0000"] {
        v.push(demo.join(p));
    }
    if let Some(pairs) = std::env::var_os("RECORDBOX_TEST_ANLZ_PAIRS") {
        let pairs = PathBuf::from(pairs);
        let text = std::fs::read_to_string(&pairs)
            .unwrap_or_else(|e| panic!("could not read oracle pairs {}: {e}", pairs.display()));
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let (_, base) = line.split_once('\t').expect("oracle pairs must contain audio<TAB>analysis-base");
            assert!(!base.is_empty(), "oracle analysis base must not be empty");
            let base = PathBuf::from(base);
            v.push(if base.is_absolute() { base } else { pairs.parent().unwrap_or(Path::new(".")).join(base) });
        }
    }
    for base in &v {
        assert!(base.with_extension("EXT").is_file() && base.with_extension("DAT").is_file(),
            "oracle DAT/EXT pair is missing: {}", base.display());
    }
    v
}

#[test]
#[ignore = "requires RECORDBOX_TEST_ORACLES fixtures"]
fn oracle_a6_real_pqt2_sections_are_regenerated_from_their_pqtz() {
    let mut checked = 0;
    for base in real_anlz_bases() {
        let dat = anlz::read_sections(&base.with_extension("DAT"));
        let ext = anlz::read_sections(&base.with_extension("EXT"));
        let (Some(pqtz), Some(real)) = (dat.get("PQTZ"), ext.get("PQT2")) else { continue };
        let entries = anlz::pqtz_entries(pqtz);
        // The whole 56-byte header follows from PQTZ alone …
        assert_eq!(anlz::pqt2(&entries, &[])[..56], real[..56], "{}", base.display());
        // … and with the per-beat microsecond fractions the section is identical.
        assert_eq!(&anlz::pqt2(&entries, &anlz::pqt2_fractions(real)), real, "{}", base.display());
        // The fractions are what makes the beats evenly spaced again.
        let us: Vec<i64> = entries.iter().zip(anlz::pqt2_fractions(real)).map(|(e, f)| e.2 as i64 * 1000 + f as i64).collect();
        let beat = 60_000_000.0 / (entries[0].1 as f64 / 100.0);
        assert!(us.windows(2).all(|w| ((w[1] - w[0]) as f64 - beat).abs() < 1000.0));
        checked += 1;
    }
    assert!(checked >= 2, "only {checked} real files checked");
    eprintln!("PQT2 regenerated byte for byte for {checked} real files");
}

#[test]
fn a6_tracks_recordbox_analysed_always_get_pqt2() {
    let grid = BeatGrid::from_anchors(vec![GridAnchor { time: 0.025_850, bpm: 127.999_25, beat: 1 }]);
    let beats = grid.beats(60.0);
    let files = anlz::build("/Contents/a/b/c.mp3", &beats, &Waveform::default(), &[], 44100 * 60, 44100);
    let tags: Vec<String> = anlz::section_list(&files.ext).into_iter().map(|s| s.0).collect();
    assert_eq!(tags, ["PPTH", "PWV3", "PCOB", "PCOB", "PCO2", "PCO2", "PQT2", "PWV5", "PWV4"], "PQT2 sits where rekordbox puts it");
    assert!(anlz::validate(&files.ext).is_ok());
    let dat = anlz::section_list(&files.dat);
    let entries = anlz::pqtz_entries(&dat.iter().find(|s| s.0 == "PQTZ").unwrap().1);
    let ext = anlz::section_list(&files.ext);
    let q = &ext.iter().find(|s| s.0 == "PQT2").unwrap().1;
    assert_eq!(entries.len(), beats.len());
    assert_eq!(u32::from_be_bytes(q[40..44].try_into().unwrap()) as usize, beats.len());
    let sum: u32 = entries.iter().map(|e| e.0 as u32 + e.1 as u32 + e.2).sum();
    assert_eq!(u32::from_be_bytes(q[44..48].try_into().unwrap()), sum);
    // PQTZ milliseconds are cut (not rounded) and PQT2 holds the rest: together the exact time.
    for ((e, f), b) in entries.iter().zip(anlz::pqt2_fractions(q)).zip(&beats) {
        assert!(f < 1000);
        assert!(((e.2 as f64 * 1000.0 + f as f64) - b.time * 1e6).abs() < 0.51, "beat at {}", b.time);
    }
    assert_eq!(entries[1].2, 494, "494.603 ms is stored as 494 + 603, as in the real file");
    // No beats, no section.
    let none = anlz::build("/Contents/a/b/c.mp3", &[], &Waveform::default(), &[], 0, 44100);
    assert!(!anlz::section_list(&none.ext).iter().any(|s| s.0 == "PQT2"));
}

// ---- A7 ------------------------------------------------------------------------------------

fn pcp2_records(ext: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![];
    for (tag, sec) in anlz::section_list(ext) {
        if tag != "PCO2" {
            continue;
        }
        let mut i = u32::from_be_bytes(sec[4..8].try_into().unwrap()) as usize;
        while i + 12 <= sec.len() {
            let total = u32::from_be_bytes(sec[i + 8..i + 12].try_into().unwrap()) as usize;
            out.push(sec[i..i + total].to_vec());
            i += total;
        }
    }
    out
}

#[test]
fn a7_pcp2_carries_loop_sizes_and_sub_millisecond_fractions() {
    let grid = BeatGrid::constant(0.1, 128.0, 1);
    let beats = grid.beats(120.0);
    let beat = 60.0 / 128.0;
    let cue = |slot: i32, start: f64, end: Option<f64>| Cue { id: 0, track_id: 1, kind: if slot >= 0 { CueKind::Hot } else { CueKind::Memory }, slot, start, end, color: 0, comment: String::new(), active_loop: false };
    let cues = vec![
        cue(0, 0.1, Some(0.1 + 4.0 * beat)),            // 4 beats
        cue(1, 0.1 + 8.0 * beat, Some(0.1 + 8.5 * beat)), // half a beat
        cue(2, 10.0, Some(11.3)),                        // not a beat-loop size
        cue(3, 287.479, None),                           // a point cue
        cue(-1, 1.234_567, Some(1.234_567 + 16.0 * beat)),
    ];
    let files = anlz::build("/Contents/a/b/c.mp3", &beats, &Waveform::default(), &cues, 0, 44100);
    let recs = pcp2_records(&files.ext);
    assert_eq!(recs.len(), 5);
    let u16at = |r: &[u8], o: usize| u16::from_be_bytes([r[o], r[o + 1]]);
    let size = |r: &[u8]| (u16at(r, 36), u16at(r, 38));
    assert_eq!(size(&recs[0]), (4, 1));
    assert_eq!(size(&recs[1]), (1, 2));
    assert_eq!(size(&recs[2]), (0, 0), "both parts zero unless both are set");
    assert_eq!(size(&recs[3]), (0, 0));
    assert_eq!(size(&recs[4]), (16, 1));
    for r in &recs {
        assert_eq!(r.len(), 88, "88 bytes without a comment");
        assert_eq!((r[17], u16at(r, 18), r[29], u16at(r, 34)), (0, 1000, 1, 0));
        assert!(u16at(r, 30) <= 999 && u16at(r, 32) <= 999);
        // Seek data: zeros (what rekordbox writes for a cue without it).
        assert!(r[48..88].iter().all(|b| *b == 0));
    }
    // 1.234567 s: 1234 ms and 567 µs; its end 8.734567 s: 8734 and 567.
    assert_eq!((u32::from_be_bytes(recs[4][20..24].try_into().unwrap()), u16at(&recs[4], 30), u16at(&recs[4], 32)), (1234, 567, 567));
    // The one real record at hand (rekordcrate's test, hot cue D at 287,479 ms, no comment):
    // the fixed 44-byte prefix is identical.
    let real: [u8; 44] = [0x50, 0x43, 0x50, 0x32, 0, 0, 0, 0x10, 0, 0, 0, 0x58, 0, 0, 0, 4, 1, 0, 3, 0xe8, 0, 4, 0x62, 0xf7, 0xff, 0xff, 0xff, 0xff, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(recs[3][..44], real);
    // Loop sizes rekordbox offers, and nothing else.
    assert_eq!(anlz::beat_loop_size(0.1, 0.1 + beat / 64.0, &beats), Some((1, 64)));
    assert_eq!(anlz::beat_loop_size(0.1, 0.1 + 512.0 * beat, &beats), Some((512, 1)));
    assert_eq!(anlz::beat_loop_size(0.1, 0.1 + 3.0 * beat, &beats), None);
    assert_eq!(anlz::beat_loop_size(0.1, 0.1 + 4.0 * beat, &[]), None);
}

// ---- A8 ------------------------------------------------------------------------------------

#[test]
fn a8_track_row_values() {
    // Average tempo of a grid with several anchors: beats ÷ time.
    let g = BeatGrid::from_anchors(vec![GridAnchor { time: 0.0, bpm: 120.0, beat: 1 }, GridAnchor { time: 60.0, bpm: 140.0, beat: 1 }]);
    let t = row_tempo(Some(&g), 120.0, 0.0);
    assert!((12_950..=13_050).contains(&t), "average of one minute at 120 and one at 140 is 130, got {t}");
    assert_eq!(row_tempo(Some(&BeatGrid::constant(0.0, 128.0, 1)), 200.0, 0.0), 12_800);
    assert_eq!(row_tempo(None, 200.0, 99.5), 9_950);
    for (given, want) in [("2024-04-17", "2024-04-17"), ("2024/4/7", "2024-04-07"), ("2024-04-17T10:22:01Z", "2024-04-17"), ("20240417", "2024-04-17"), ("2024", ""), ("", ""), ("next year", ""), ("2024-13-40", ""), ("日付２０２４", "")] {
        assert_eq!(normalize_date(given), want, "{given:?}");
    }
    let mut tr = Track { key: 4, ..Default::default() };
    let classic = key_name(4, false);
    assert_eq!(row_key_name(&tr), classic);
    tr.key_text = "7A".into();
    assert_eq!(row_key_name(&tr), "7A");
    tr.key = -1;
    tr.key_text.clear();
    assert_eq!(row_key_name(&tr), "");
    // A new row gets those defaults.
    let mut lib = DeviceLibrary::default();
    lib.tracks.insert(1, DevTrack { id: 1, title: "x".into(), file_path: "/Contents/a/b/x.mp3".into(), ..Default::default() });
    let back = parse_pdb(build_export_pdb(&lib)).unwrap();
    let s = &back.tracks[&1].raw_strings;
    assert_eq!((s[2].as_str(), s[3].as_str(), s[6].as_str()), ("1", "1", "ON"));
}

#[test]
#[ignore = "requires RECORDBOX_TEST_ORACLES fixtures"]
fn oracle_a8_real_track_key_date_and_string_values() {
    // Real exports hold the stored spellings side by side, and date strings of 10 characters
    // or none.
    let lib = parse_pdb(num_rows_pdb()).unwrap();
    let keys: std::collections::HashSet<&str> = lib.tracks.values().map(|t| t.key_name.as_str()).collect();
    for k in ["Emin", "Dm", "7m", "2d"] {
        assert!(keys.contains(k), "{k}");
    }
    assert!(lib.tracks.values().all(|t| t.release_date.is_empty() || normalize_date(&t.release_date) == t.release_date));
    assert!(lib.tracks.values().all(|t| normalize_date(&t.date_added) == t.date_added));
    // Newest fixtures: slot 3 is "1" in all but one row, slot 6 is "ON" in every row.
    assert!(lib.tracks.values().filter(|t| t.raw_strings[3] == "1").count() >= lib.tracks.len() - 1);
    assert!(lib.tracks.values().all(|t| t.raw_strings[6] == "ON"));
}

// ---- A9 ------------------------------------------------------------------------------------

#[test]
fn a9_names_are_decomposed_on_hfs_and_precomposed_elsewhere() {
    use unicode_normalization::UnicodeNormalization;
    let (artist, album) = ("Beyonce\u{301}", "Di\u{301}as");
    let nfc = media_dir_fs(artist, album, false);
    let nfd = media_dir_fs(artist, album, true);
    assert_eq!(nfc, "/Contents/Beyonc\u{e9}/D\u{ed}as");
    assert_eq!(nfd, "/Contents/Beyonce\u{301}/Di\u{301}as");
    assert_eq!(nfc, media_dir(artist, album));
    assert_eq!(path_key(&nfc), path_key(&nfd));
    assert!(fs_decomposes("hfs") && !fs_decomposes("msdos") && !fs_decomposes("exfat") && !fs_decomposes("folder"));
    // End to end: the path stored in the database is spelled for the drive's filesystem.
    for (fs, decomposed) in [("hfs", true), ("exfat", false)] {
        let root = scratch(&format!("norm_{fs}"));
        let mut tracks = demo_tracks(&src_dir(), 1);
        tracks.get_mut(&1).unwrap().track.artist = "Beyonc\u{e9}".into();
        export(ExportRequest { fs: fs.into(), ..request(&root, &tracks, vec![playlist(1, "Set", vec![1])]) });
        let lib = read_device(&root).unwrap();
        let p = &lib.tracks.values().next().unwrap().file_path;
        assert_eq!(p.contains("e\u{301}"), decomposed, "{fs}: {p:?}");
        assert_eq!(p.nfc().collect::<String>() == *p, !decomposed);
        assert!(root.join(p.trim_start_matches('/')).exists());
        // Exporting again finds the same row whatever the spelling.
        export(ExportRequest { fs: fs.into(), ..request(&root, &tracks, vec![playlist(1, "Set", vec![1])]) });
        assert_eq!(read_device(&root).unwrap().tracks.len(), 1);
    }
}

// ---- A10 -----------------------------------------------------------------------------------

#[test]
fn a10_same_named_sibling_playlists_stay_distinct() {
    let root = scratch("siblings");
    let tracks = demo_tracks(&src_dir(), 3);
    let folder = |kids: Vec<ExportNode>| ExportNode { id: 10, name: "Gigs".into(), folder: true, children: kids, ..Default::default() };
    export(request(&root, &tracks, vec![folder(vec![playlist(11, "Set", vec![1, 2]), playlist(12, "Set", vec![3])])]));
    let lib = read_device(&root).unwrap();
    let sets: Vec<&DevNode> = lib.nodes.iter().filter(|n| n.name == "Set").collect();
    assert_eq!(sets.len(), 2, "two playlists named Set under one folder");
    assert_eq!((sets[0].entries.len(), sets[1].entries.len()), (2, 1));
    assert_ne!(sets[0].seq, sets[1].seq);
    let (a, b) = (sets[0].id, sets[1].id);
    // Exported again in the other order, each library playlist updates its own device copy.
    export(request(&root, &tracks, vec![folder(vec![playlist(12, "Set", vec![3, 1]), playlist(11, "Set", vec![2])])]));
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.nodes.len(), 3);
    assert_eq!(lib.nodes.iter().find(|n| n.id == a).unwrap().entries.len(), 1);
    assert_eq!(lib.nodes.iter().find(|n| n.id == b).unwrap().entries.len(), 2);
    // A renamed playlist keeps its device copy; without Recordbox's record (a drive from
    // another computer) playlists are matched by name, one each.
    export(request(&root, &tracks, vec![folder(vec![playlist(11, "Warm-up", vec![2])])]));
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.nodes.iter().find(|n| n.id == a).unwrap().name, "Warm-up");
    assert_eq!(lib.nodes.len(), 3);
    std::fs::remove_file(root.join(".Recordbox/nodes.json")).unwrap();
    export(request(&root, &tracks, vec![folder(vec![playlist(11, "Set", vec![1]), playlist(12, "Set", vec![2])])]));
    let lib = read_device(&root).unwrap();
    assert_eq!(lib.nodes.iter().filter(|n| n.name == "Set").count(), 2);
    assert_eq!(lib.nodes.len(), 4);
}

// ---- A11 -----------------------------------------------------------------------------------

#[test]
fn a11_fat32_limits_apply_to_fat32_only() {
    let root = scratch("fat");
    let mut seen = HashMap::new();
    let ok = "/Contents/Artist/Album/track.wav";
    assert_eq!(fat32_reason(&root, ok, 1_000, &mut seen), None);
    assert!(fat32_reason(&root, ok, 5_000_000_000, &mut seen).unwrap().contains("4 GB"));
    let long = format!("/Contents/{}/{}/{}.wav", "a".repeat(120), "b".repeat(120), "c".repeat(40));
    assert!(fat32_reason(&root, &long, 1_000, &mut seen).unwrap().contains("255 characters"));
    // A folder that already uses all its directory entries.
    let dir = root.join("Contents/Full/Album");
    seen.insert(dir, FAT_DIR_MAX_ENTRIES - 2);
    assert!(fat32_reason(&root, "/Contents/Full/Album/one more track.wav", 1_000, &mut seen).unwrap().contains("full"));
    // Each accepted file is counted (name of 9 characters: one short + one long entry).
    let mut seen = HashMap::new();
    assert_eq!(fat32_reason(&root, ok, 1_000, &mut seen), None);
    assert_eq!(seen.values().next(), Some(&(2 + 2)));
    // The export names the reason per track on FAT32, and exports the same track elsewhere.
    let mut tracks = demo_tracks(&src_dir(), 1);
    tracks.get_mut(&1).unwrap().track.artist = "Full".into();
    tracks.get_mut(&1).unwrap().track.album = "Album".into();
    let fat = scratch("fat_drive");
    let dir = fat.join("Contents/Full/Album");
    std::fs::create_dir_all(&dir).unwrap();
    // 5,042 files with 255-character names use 21 entries each: more than 65,536 with one more.
    // Creating that many files would be slow; the limit itself is covered above, so here only
    // the wiring is checked with the size rule through a sparse 4 GB + 1 file.
    let big = src_dir().join("a11-big.wav");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(u32::MAX as u64 + 1).unwrap();
    let mut bigs = tracks.clone();
    bigs.get_mut(&1).unwrap().track.path = big.to_string_lossy().to_string();
    bigs.get_mut(&1).unwrap().wave = Some(Waveform::default());
    let msg = export(ExportRequest { fs: "msdos".into(), ..request(&fat, &bigs, vec![playlist(1, "Set", vec![1])]) });
    assert!(msg.contains("larger than 4 GB, too big for a FAT32 drive"), "{msg}");
    assert!(read_device(&fat).unwrap().tracks.is_empty());
    let _ = std::fs::remove_file(&big);
}

// ---- A12 -----------------------------------------------------------------------------------

#[test]
fn a12_long_my_tag_names_are_cut_to_fit_their_row() {
    let mut lib = DeviceLibrary::default();
    lib.tracks.insert(1, DevTrack { id: 1, file_path: "/Contents/a/b/x.mp3".into(), ..Default::default() });
    let long_tag = "ж".repeat(300);
    let long_cat = "C".repeat(400);
    lib.tag_categories = vec![(long_cat.clone(), vec![long_tag.clone(), "Short".into()])];
    lib.tracks.get_mut(&1).unwrap().tags = vec![tag_key(&long_cat, &long_tag), tag_key(&long_cat, "Short")];
    let ext = try_build_export_ext(&lib).unwrap();
    let db = Database::parse(ext.clone()).unwrap();
    for (h, o) in db.rows(3) {
        let r = db.row_bytes(h, o);
        // Both 8-bit offsets point inside the row, the second after the first string.
        let (a, b) = (r[0x1d] as usize, r[0x1e] as usize);
        assert!(a >= 0x1f && b > a && b <= 255, "offsets {a} {b}");
        assert!(pdb::read_dstr(r, a).chars().count() <= MAX_TAG_CHARS);
        assert_eq!(pdb::read_dstr(r, b), "");
    }
    // Read back the way a drive is read: the names are the cut ones and the track keeps both tags.
    let root = scratch("tags");
    std::fs::create_dir_all(root.join("PIONEER/rekordbox")).unwrap();
    std::fs::write(root.join("PIONEER/rekordbox/export.pdb"), build_export_pdb(&lib)).unwrap();
    std::fs::write(root.join("PIONEER/rekordbox/exportExt.pdb"), ext).unwrap();
    let back = read_device(&root).unwrap();
    assert_eq!(back.tag_categories[0].0, "C".repeat(MAX_TAG_CHARS));
    assert_eq!(back.tag_categories[0].1, vec!["ж".repeat(MAX_TAG_CHARS), "Short".to_string()]);
    assert_eq!(back.tracks[&1].tags.len(), 2);
}

// ---- A13 -----------------------------------------------------------------------------------

#[test]
fn a13_identical_artwork_is_shared_and_folders_roll_over() {
    assert_eq!(artwork_rel(1), "/PIONEER/Artwork/00001/a1.jpg");
    assert_eq!(artwork_rel(19), "/PIONEER/Artwork/00001/a19.jpg");
    assert_eq!(artwork_rel(20), "/PIONEER/Artwork/00002/a20.jpg");
    assert_eq!(artwork_rel(2210), "/PIONEER/Artwork/00111/a2210.jpg");
    let root = scratch("artwork");
    let mut tracks = demo_tracks(&src_dir(), 3);
    let (one, two) = (src_dir().join("a13-one.png"), src_dir().join("a13-two.png"));
    picture(&one, 10);
    picture(&two, 250);
    // The same picture stored as two files (as an album's tracks usually have it).
    let one_copy = src_dir().join("a13-one-copy.png");
    std::fs::copy(&one, &one_copy).unwrap();
    for (id, art) in [(1, &one), (2, &one_copy), (3, &two)] {
        tracks.get_mut(&id).unwrap().track.artwork = art.to_string_lossy().to_string();
    }
    export(request(&root, &tracks, vec![playlist(1, "Set", vec![1, 2, 3])]));
    let lib = read_device(&root).unwrap();
    let art: Vec<&str> = lib.tracks.values().map(|t| t.artwork_path.as_str()).collect();
    assert_eq!(art[0], art[1], "identical pictures share one artwork file");
    assert_ne!(art[0], art[2]);
    assert_eq!(files_under(&root, "PIONEER/Artwork"), ["PIONEER/Artwork/00001/a1.jpg", "PIONEER/Artwork/00001/a1_m.jpg", "PIONEER/Artwork/00001/a2.jpg", "PIONEER/Artwork/00001/a2_m.jpg"]);
    // The artwork table has one row per picture.
    let db = Database::parse(std::fs::read(pdb_path(&root)).unwrap()).unwrap();
    assert_eq!(db.rows(13).len(), 2);
    // Exporting again writes no further pictures.
    export(request(&root, &tracks, vec![playlist(1, "Set", vec![1, 2, 3])]));
    assert_eq!(files_under(&root, "PIONEER/Artwork").len(), 4);
    assert_eq!(read_device(&root).unwrap().tracks.values().map(|t| t.artwork_path.clone()).collect::<Vec<_>>(), art);
}

#[test]
#[ignore = "requires RECORDBOX_TEST_ORACLES fixtures"]
fn oracle_a13_real_export_artwork_paths_follow_folder_rule() {
    // Every artwork path of the real 2,178-picture export follows the rule.
    let db = Database::parse(num_rows_pdb()).unwrap();
    let rows = db.rows(13);
    assert!(rows.len() > 2000);
    for (h, o) in rows {
        let r = db.row_bytes(h, o);
        assert_eq!(pdb::read_dstr(r, 4), artwork_rel(pdb::rd_u32(r, 0)));
    }
}
