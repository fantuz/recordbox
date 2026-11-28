//! End-to-end check against a collection XML written by the original: import it, export the
//! local tracks to a folder inside target/, then read the device library and analysis files
//! back and compare them field by field with the XML. PREPDECK_XML = path of the XML.
//! The XML and the audio files are only read.

use super::anlz_compare::sections;
use super::usb::*;
use crate::model::*;
use std::collections::HashMap;

#[test]
#[ignore]
fn export_matches_collection_xml() {
    let Ok(xml_path) = std::env::var("PREPDECK_XML") else { return };
    let text = std::fs::read_to_string(&xml_path).unwrap();
    let lib = crate::importer::parse_rekordbox_xml(&text).unwrap();
    let local: Vec<&crate::importer::XmlTrack> = lib.tracks.iter().filter(|t| std::path::Path::new(&t.track.path).exists()).collect();
    eprintln!("collection tracks {}, local files {}", lib.tracks.len(), local.len());

    // Build export tracks (analysis only for waveforms; grid, key and cues come from the XML).
    let mut tracks = HashMap::new();
    let mut xml_ids: HashMap<String, i64> = HashMap::new();
    let results = std::sync::Mutex::new(vec![]);
    std::thread::scope(|s| {
        for (ci, ch) in local.chunks((local.len() / 8).max(1)).enumerate() {
            let results = &results;
            s.spawn(move || {
                for (k, xt) in ch.iter().enumerate() {
                    let r = crate::analysis::analyze_file(&xt.track.path, &Default::default()).ok();
                    results.lock().unwrap().push((ci * 10000 + k, (*xt).clone(), r));
                }
            });
        }
    });
    let rb_pairs: HashMap<String, std::path::PathBuf> = std::env::var("PREPDECK_ANLZ_PAIRS")
        .ok()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|t| t.lines().filter_map(|l| l.split_once('\t')).map(|(a, b)| (a.to_string(), std::path::PathBuf::from(b))).collect())
        .unwrap_or_default();
    for (i, (_, xt, r)) in results.into_inner().unwrap().into_iter().enumerate() {
        let id = i as i64 + 1;
        let mut t = xt.track.clone();
        t.id = id;
        if let Some(r) = &r {
            t.raw_samples = r.raw_samples;
            t.sample_rate = r.sample_rate;
            if t.duration <= 0.0 {
                t.duration = r.duration;
            }
        }
        let cues: Vec<Cue> = xt.cues.iter().map(|c| Cue { track_id: id, ..c.clone() }).collect();
        xml_ids.insert(xt.xml_id.clone(), id);
        // With PREPDECK_ANLZ_PAIRS, tracks carry rekordbox's own analysis files (as after a sync).
        let rb_anlz = rb_pairs.get(&t.path).cloned().filter(|b: &std::path::PathBuf| b.with_extension("DAT").is_file());
        let grid_same = match (&rb_anlz, &xt.grid) {
            (Some(b), Some(g)) => crate::export::anlz::grid_pair::matches_source(&b.with_extension("DAT"), g),
            _ => false,
        };
        tracks.insert(id, ExportTrack { track: t, cues, grid: xt.grid.clone(), wave: r.map(|r| r.wave), tags: vec![], rb_anlz, grid_same, vocal: None, phrases: None });
    }
    fn nodes(n: &[crate::importer::XmlNode], ids: &HashMap<String, i64>) -> Vec<ExportNode> {
        n.iter()
            .map(|x| ExportNode { name: x.name.clone(), folder: x.folder, children: nodes(&x.children, ids), tracks: x.track_keys.iter().filter_map(|k| ids.get(k).copied()).collect(), ..Default::default() })
            .collect()
    }
    let enodes = nodes(&lib.playlists, &xml_ids);
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("xml_compare");
    let root = base.join(chrono::Local::now().format("usb_%H%M%S").to_string());
    std::fs::create_dir_all(&root).unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let msg = run_export(ExportRequest { root: root.clone(), nodes: enodes, tracks: tracks.clone(), tag_categories: vec![], skipped: vec![], analysis: Default::default(), settings: Default::default(), browse: Default::default(), phrases: true, ..Default::default() }, &|_| {}, &cancel).unwrap();
    eprintln!("{msg}");
    let dev = read_device(&root).unwrap();
    // Sections passed through from rekordbox's own analysis are byte-identical to its files.
    {
        let (mut same, mut total, mut with_grid, mut pssi) = (0, 0, 0, 0);
        for et in tracks.values() {
            let Some(b) = &et.rb_anlz else { continue };
            let Some(dt) = dev.tracks.values().find(|d| d.source == et.track.path) else { continue };
            let base = root.join(dt.analyze_path.trim_start_matches('/')).with_extension("");
            for (e, tags) in [("DAT", vec!["PWAV", "PWV2", "PVBR"]), ("EXT", vec!["PWV3", "PWV4", "PWV5"]), ("2EX", vec!["PWV6", "PWV7", "PWVC", "PVDI"])] {
                let mine = super::anlz::read_sections(&base.with_extension(e));
                let theirs = super::anlz::read_sections(&b.with_extension(e));
                for t in tags {
                    if let Some(x) = theirs.get(t) {
                        total += 1;
                        if mine.get(t) == Some(x) {
                            same += 1;
                        }
                    }
                }
            }
            if et.grid_same {
                with_grid += 1;
                if super::anlz::read_sections(&base.with_extension("EXT")).contains_key("PSSI") {
                    pssi += 1;
                }
            }
        }
        eprintln!("rekordbox sections passed through byte-identical: {same} / {total}; grid unchanged on {with_grid} tracks, {pssi} with phrases (PSSI)");
        if total > 0 {
            assert_eq!(same, total);
        }
    }

    let mut checks: Vec<(&str, usize, usize)> = vec![];
    let mut bump = |name: &'static str, ok: bool, checks: &mut Vec<(&str, usize, usize)>| {
        if let Some(c) = checks.iter_mut().find(|c| c.0 == name) {
            c.1 += ok as usize;
            c.2 += 1;
        } else {
            checks.push((name, ok as usize, 1));
        }
    };
    let mut examples: Vec<String> = vec![];
    for (id, et) in &tracks {
        let src = &et.track;
        let Some(d) = dev.tracks.values().find(|d| d.source == src.path) else {
            bump("track present on device", false, &mut checks);
            continue;
        };
        bump("track present on device", true, &mut checks);
        bump("title", d.title == src.display_title(), &mut checks);
        bump("artist", d.artist == src.artist, &mut checks);
        bump("album", d.album == src.album, &mut checks);
        bump("genre", d.genre == src.genre, &mut checks);
        bump("label", d.label == src.label, &mut checks);
        let xml_bpm = et.grid.as_ref().map(|g| g.first_bpm()).unwrap_or(src.bpm);
        bump("tempo (BPM x100)", d.bpm100 == (xml_bpm * 100.0).round() as u32, &mut checks);
        bump("key", parse_key(&d.key_name) == src.key || (src.key < 0 && d.key_name.is_empty()), &mut checks);
        bump("rating", d.rating == src.rating, &mut checks);
        bump("file copied with same size", std::fs::metadata(root.join(d.file_path.trim_start_matches('/'))).map(|m| m.len()).ok() == std::fs::metadata(&src.path).map(|m| m.len()).ok(), &mut checks);
        // Analysis files
        let dat = std::fs::read(root.join(d.analyze_path.trim_start_matches('/'))).unwrap_or_default();
        let ext = std::fs::read(root.join(d.analyze_path.trim_start_matches('/').replace(".DAT", ".EXT"))).unwrap_or_default();
        let ds = sections(&dat);
        let es = sections(&ext);
        // Grid: every XML TEMPO anchor should be a beat in PQTZ (same ms, same beat-in-bar, same tempo).
        if let (Some(g), Some(q)) = (&et.grid, ds.iter().find(|s| &s.0 == b"PQTZ")) {
            let beats: Vec<(u16, u16, u32)> = q.2.chunks_exact(8).map(|c| (u16::from_be_bytes([c[0], c[1]]), u16::from_be_bytes([c[2], c[3]]), u32::from_be_bytes([c[4], c[5], c[6], c[7]]))).collect();
            for a in &g.anchors {
                let ms = (a.time * 1000.0).round() as u32;
                let hit = beats.iter().find(|b| (b.2 as i64 - ms as i64).abs() <= 1);
                let ok = hit.map(|b| b.0 == a.beat as u16 && b.1 == (a.bpm * 100.0).round() as u16).unwrap_or(false);
                bump("grid anchor present in PQTZ (±1 ms, beat, tempo)", ok, &mut checks);
                if !ok && examples.len() < 6 {
                    examples.push(format!("grid anchor {:.3}s beat {} not matched in {}", a.time, a.beat, src.display_title()));
                }
            }
            // Beat spacing between anchors follows the tempo within 1 ms.
            let mut worst = 0i64;
            for w in beats.windows(2) {
                let p = 60000.0 / (w[0].1 as f64 / 100.0);
                worst = worst.max(((w[1].2 as f64 - w[0].2 as f64) - p).abs().round() as i64);
            }
            bump("PQTZ beat spacing within 1 ms of tempo", worst <= 1, &mut checks);
        }
        // Cues: hot cues in DAT PCOB/PCPT and EXT PCO2/PCP2.
        let xml_hot: Vec<&Cue> = et.cues.iter().filter(|c| c.kind == CueKind::Hot && (0..8).contains(&c.slot)).collect();
        let pcpt: Vec<(u32, u32)> = ds
            .iter()
            .filter(|s| &s.0 == b"PCOB")
            .flat_map(|s| sections_flat(&s.2))
            .filter(|(t, _, _)| t == b"PCPT")
            .map(|(_, h, b)| (u32::from_be_bytes([h[0], h[1], h[2], h[3]]), u32::from_be_bytes([b[4], b[5], b[6], b[7]])))
            .filter(|(n, _)| *n > 0)
            .collect();
        let pcp2: Vec<(u32, u32, [u8; 3], String, u8)> = es
            .iter()
            .filter(|s| &s.0 == b"PCO2")
            .flat_map(|s| sections_flat(&s.2))
            .filter(|(t, _, _)| t == b"PCP2")
            .map(|(_, h, b)| {
                let n = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
                let time = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
                let clen = u32::from_be_bytes([b[0x18], b[0x19], b[0x1a], b[0x1b]]) as usize;
                let cm: Vec<u16> = b[0x1c..0x1c + clen].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                let comment = String::from_utf16_lossy(&cm).trim_end_matches('\0').to_string();
                let rgb = [b[0x1d + clen], b[0x1e + clen], b[0x1f + clen]];
                let idx = b[0x1c + clen];
                (n, time, rgb, comment, idx)
            })
            .filter(|x| x.0 > 0)
            .collect();
        for c in &xml_hot {
            let ms = (c.start * 1000.0).round() as u32;
            let n = c.slot as u32 + 1;
            bump("hot cue in DAT (number, ms)", pcpt.iter().any(|(a, t)| *a == n && *t == ms), &mut checks);
            let p2 = pcp2.iter().find(|x| x.0 == n);
            bump("hot cue in EXT (number, ms)", p2.map(|x| x.1 == ms).unwrap_or(false), &mut checks);
            bump("hot cue name", p2.map(|x| x.3 == c.comment).unwrap_or(false), &mut checks);
            // The XML carries the desktop RGB; the players' file carries the same palette index
            // with the device palette's RGB (rekordbox-rea RF-05).
            bump("hot cue colour (palette index)", p2.map(|x| x.4 == c.color).unwrap_or(false), &mut checks);
            bump("hot cue colour (device palette RGB)", p2.map(|x| x.2 == crate::model::device_rgb(c.color)).unwrap_or(false), &mut checks);
        }
        let _ = id;
    }
    // Playlists: names, nesting and order as in the XML.
    fn walk(x: &[crate::importer::XmlNode], parent: u32, dev: &DeviceLibrary, ids: &HashMap<String, i64>, tracks: &HashMap<i64, ExportTrack>, out: &mut Vec<(String, bool)>) {
        for n in x {
            let d = dev.nodes.iter().find(|d| d.parent == parent && d.name == n.name && d.folder == n.folder);
            out.push((format!("playlist node \"{}\" present", n.name), d.is_some()));
            if let Some(d) = d {
                if !n.folder {
                    let want: Vec<String> = n.track_keys.iter().filter_map(|k| ids.get(k)).filter_map(|i| tracks.get(i)).map(|t| t.track.path.clone()).collect();
                    let got: Vec<String> = d.entries.iter().filter_map(|e| dev.tracks.get(e)).map(|t| t.source.clone()).collect();
                    out.push((format!("playlist \"{}\" order ({} tracks)", n.name, want.len()), want == got));
                }
                walk(&n.children, d.id, dev, ids, tracks, out);
            }
        }
    }
    let mut pl = vec![];
    walk(&lib.playlists, 0, &dev, &xml_ids, &tracks, &mut pl);
    eprintln!("---- field checks (passed / total)");
    for (n, ok, tot) in &checks {
        eprintln!("{:<52} {:>4} / {:<4}{}", n, ok, tot, if ok == tot { "" } else { "  <-- differs" });
    }
    for (n, ok) in &pl {
        eprintln!("{:<52} {}", n, if *ok { "ok" } else { "DIFFERS" });
    }
    for e in examples {
        eprintln!("  e.g. {e}");
    }
    std::fs::write(base.join("LAST"), root.to_string_lossy().as_bytes()).unwrap();
}

/// Sub-sections inside a list body (PCOB/PCO2 contain PCPT/PCP2 entries).
fn sections_flat(b: &[u8]) -> Vec<([u8; 4], Vec<u8>, Vec<u8>)> {
    let mut out = vec![];
    let mut o = 0;
    while o + 12 <= b.len() {
        let tag = [b[o], b[o + 1], b[o + 2], b[o + 3]];
        let hl = u32::from_be_bytes([b[o + 4], b[o + 5], b[o + 6], b[o + 7]]) as usize;
        let tl = u32::from_be_bytes([b[o + 8], b[o + 9], b[o + 10], b[o + 11]]) as usize;
        if tl < 12 || o + tl > b.len() {
            break;
        }
        out.push((tag, b[o + 12..o + hl].to_vec(), b[o + hl..o + tl].to_vec()));
        o += tl;
    }
    out
}
