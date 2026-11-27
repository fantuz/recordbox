//! Export the collection in the common DJ collection XML format (tracks, grids, cues,
//! playlists) so prepared data can move to other software.

use crate::app::App;
use crate::model::*;

fn esc(s: &str) -> String {
    // Newlines and tabs as character references: XML parsers turn literal ones in attribute
    // values into spaces.
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;")
}

fn location(path: &str) -> String {
    let mut out = String::from("file://localhost");
    for b in path.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "/-_.~".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Marks an attribute the imported XML did not have (written back as absent).
pub const ABSENT: &str = "\u{0}absent";

/// TRACK attributes as Recordbox writes them, unescaped (TrackID aside). Colour is empty when
/// the track has none (then not written).
pub fn track_attrs(t: &Track) -> Vec<(&'static str, String)> {
    let rating = [0, 51, 102, 153, 204, 255][t.rating.min(5) as usize];
    let colour = match t.color {
        1 => "0xFF007F",
        2 => "0xFF0000",
        3 => "0xFFA500",
        4 => "0xFFFF00",
        5 => "0x00FF00",
        6 => "0x25FDE9",
        7 => "0x0000FF",
        8 => "0x660099",
        _ => "",
    };
    vec![
        ("Name", t.display_title()),
        ("Artist", t.artist.clone()),
        ("Composer", t.composer.clone()),
        ("Album", t.album.clone()),
        ("Grouping", t.grouping.clone()),
        ("Genre", t.genre.clone()),
        ("Kind", format!("{} File", t.file_type.to_ascii_uppercase())),
        ("Size", t.file_size.to_string()),
        ("TotalTime", (t.duration.round() as u64).to_string()),
        ("DiscNumber", t.disc_no.to_string()),
        ("TrackNumber", t.track_no.to_string()),
        ("Year", t.year.to_string()),
        ("AverageBpm", format!("{:.2}", t.bpm)),
        ("DateAdded", t.date_added.clone()),
        ("BitRate", t.bitrate.to_string()),
        ("SampleRate", t.sample_rate.to_string()),
        ("Comments", t.comment.clone()),
        ("PlayCount", t.play_count.to_string()),
        ("Rating", rating.to_string()),
        ("Location", location(&t.path)),
        ("Remixer", t.remixer.clone()),
        ("Tonality", t.stored_key_label(false)),
        ("Label", t.label.clone()),
        ("Mix", t.mix_name.clone()),
        // Track colour, as rekordbox writes it (0xRRGGBB of its eight track colours).
        ("Colour", colour.to_string()),
    ]
}

/// The opening TRACK element. A field whose value is unchanged since an XML import is written
/// with the imported text (precision, key spelling, URL encoding, absent attributes); edited
/// fields are written from Recordbox's value; unknown attributes follow.
pub fn track_line(t: &Track, extra: &crate::importer::XmlExtra) -> String {
    track_line_id(t, extra, &t.id.to_string())
}

/// TrackIDs for export: each imported track keeps the ID its XML gave it when no other track
/// already has it; the rest use Recordbox's own ID, or the next free number on a clash.
pub fn export_ids(tracks: &[(i64, String)]) -> std::collections::HashMap<i64, String> {
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = std::collections::HashMap::new();
    for (id, xml) in tracks {
        if !xml.is_empty() && used.insert(xml.clone()) {
            out.insert(*id, xml.clone());
        }
    }
    let mut next = used.iter().filter_map(|v| v.parse::<i64>().ok()).max().unwrap_or(0).max(tracks.iter().map(|t| t.0).max().unwrap_or(0)) + 1;
    for (id, _) in tracks {
        if out.contains_key(id) {
            continue;
        }
        let own = id.to_string();
        let v = if used.insert(own.clone()) {
            own
        } else {
            while used.contains(&next.to_string()) {
                next += 1;
            }
            used.insert(next.to_string());
            next.to_string()
        };
        out.insert(*id, v);
    }
    out
}

pub fn track_line_id(t: &Track, extra: &crate::importer::XmlExtra, id: &str) -> String {
    let mut line = format!("    <TRACK TrackID=\"{}\"", esc(id));
    for (k, v) in track_attrs(t) {
        let v = extra.orig.iter().find(|o| o.0 == k && o.2 == v).map(|o| o.1.clone()).unwrap_or(v);
        if v == ABSENT || (k == "Colour" && v.is_empty()) {
            continue;
        }
        line.push_str(&format!(" {k}=\"{}\"", esc(&v)));
    }
    line.push_str(&extra_xml(extra, "").0);
    line.push_str(">\n");
    line
}

pub fn build(app: &App) -> String {
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n");
    x.push_str("  <PRODUCT Name=\"recordbox\" Version=\"1.0\" Company=\"\"/>\n");
    x.push_str(&format!("  <COLLECTION Entries=\"{}\">\n", app.tracks.len()));
    let extras: std::collections::HashMap<i64, crate::importer::XmlExtra> = app.tracks.iter().map(|t| (t.id, app.db.xml_extra(t.id))).collect();
    let ids = export_ids(&app.tracks.iter().map(|t| (t.id, extras[&t.id].xml_id.clone())).collect::<Vec<_>>());
    for t in &app.tracks {
        // Fields Recordbox doesn't model, and original text of unchanged fields, as imported.
        let extra = &extras[&t.id];
        x.push_str(&track_line_id(t, extra, &ids[&t.id]));
        let (_, extra_nodes) = extra_xml(extra, "      ");
        x.push_str(&track_children(t, extra, app.db.grid(t.id), app.db.cues(t.id).unwrap_or_default()));
        x.push_str(&extra_nodes);
        x.push_str("    </TRACK>\n");
    }
    x.push_str("  </COLLECTION>\n  <PLAYLISTS>\n");
    fn node(app: &App, ids: &std::collections::HashMap<i64, String>, parent: i64, depth: usize, x: &mut String) {
        for n in app.nodes.iter().filter(|n| n.parent == parent) {
            let pad = "  ".repeat(depth + 3);
            match n.kind {
                NodeKind::Folder => {
                    let count = app.nodes.iter().filter(|c| c.parent == n.id).count();
                    x.push_str(&format!("{pad}<NODE Type=\"0\" Name=\"{}\" Count=\"{}\">\n", esc(&n.name), count));
                    node(app, ids, n.id, depth + 1, x);
                    x.push_str(&format!("{pad}</NODE>\n"));
                }
                _ => {
                    let tracks = if n.kind == NodeKind::Smart { n.smart.as_ref().map(|r| app.smart_tracks(r)).unwrap_or_default() } else { app.db.playlist_tracks(n.id).unwrap_or_default() };
                    x.push_str(&format!("{pad}<NODE Name=\"{}\" Type=\"1\" KeyType=\"0\" Entries=\"{}\">\n", esc(&n.name), tracks.len()));
                    for t in tracks {
                        let key = ids.get(&t).cloned().unwrap_or_else(|| t.to_string());
                        x.push_str(&format!("{pad}  <TRACK Key=\"{}\"/>\n", esc(&key)));
                    }
                    x.push_str(&format!("{pad}</NODE>\n"));
                }
            }
        }
    }
    let top = app.nodes.iter().filter(|n| n.parent == 0).count();
    x.push_str(&format!("    <NODE Type=\"0\" Name=\"ROOT\" Count=\"{top}\">\n"));
    node(app, &ids, 0, 0, &mut x);
    x.push_str("    </NODE>\n  </PLAYLISTS>\n</DJ_PLAYLISTS>\n");
    x
}

/// Which beat records become TEMPO nodes, as rekordbox's XML export chooses them with its
/// default `bridgeExportBeatGrid` = 1 (traced in rekordbox-rea reports/native-xml-trace.md):
/// a beat is kept when its BPM changes or its time (ms) matches neither the running
/// prediction nor the prediction from the previous beat, each truncated with and without
/// +0.5. Records are (beat in bar, BPM × 100, time in ms), as in PQTZ.
/// rekordbox's `bridgeExportBeatGrid` setting; its default is 1.
pub const BRIDGE_EXPORT_BEAT_GRID: i64 = 1;

/// TEMPO nodes for a mode of `bridgeExportBeatGrid` as traced: below 1 none, 1 the filtered
/// beats ([`tempo_keep`]), 2 every beat, above 2 none (nodes are built and discarded).
pub fn tempo_select(records: &[(u16, u16, u32)], mode: i64) -> Vec<usize> {
    if records.is_empty() {
        return vec![];
    }
    match mode {
        1 => tempo_keep(records),
        2 => (0..records.len()).collect(),
        _ => vec![],
    }
}

pub fn tempo_keep(records: &[(u16, u16, u32)]) -> Vec<usize> {
    let trunc = |x: f64| x as u32;
    let (mut prev_bpm, mut expected, mut from_prev) = (0u16, 0.0f64, 0.0f64);
    let mut out = vec![];
    for (i, &(_, bpm, ms)) in records.iter().enumerate() {
        let hit_expected = ms == trunc(expected) || ms == trunc(expected + 0.5);
        let hit_prev = ms == trunc(from_prev) || ms == trunc(from_prev + 0.5);
        if bpm != prev_bpm || !(hit_expected || hit_prev) {
            out.push(i);
            if !hit_expected {
                expected = ms as f64;
            }
            prev_bpm = bpm;
        }
        let interval = if bpm > 0 { 6_000_000.0 / bpm as f64 } else { 0.0 };
        expected += interval;
        from_prev = ms as f64 + interval;
    }
    out
}

#[cfg(test)]
mod id_tests {
    #[test]
    fn imported_ids_kept_without_clashes() {
        // Track 5 was imported as 1; native track 1 must then take another number.
        let ids = super::export_ids(&[(1, String::new()), (5, "1".into()), (6, "77".into()), (7, "77".into())]);
        assert_eq!(ids[&5], "1");
        assert_eq!(ids[&6], "77");
        assert_ne!(ids[&1], "1");
        assert_ne!(ids[&7], "77");
        let all: std::collections::HashSet<&String> = ids.values().collect();
        assert_eq!(all.len(), 4);
    }
}

#[cfg(test)]
mod tempo_tests {
    use super::tempo_keep;

    #[test]
    fn constant_grid_keeps_first_beat_only() {
        // 128 BPM from 0.120 s, times rounded to ms as in PQTZ.
        let recs: Vec<(u16, u16, u32)> = (0..64).map(|k| ((k % 4 + 1) as u16, 12800, (120.0 + k as f64 * 468.75).round() as u32)).collect();
        assert_eq!(tempo_keep(&recs), vec![0]);
        // A BPM change and a timing jump each add a node.
        let mut v = recs.clone();
        for r in v.iter_mut().skip(32) {
            r.2 += 7;
        }
        assert_eq!(tempo_keep(&v), vec![0, 32]);
        // Modes: 2 keeps every beat; below 1 and above 2 keep none.
        assert_eq!(super::tempo_select(&recs, 2).len(), 64);
        assert!(super::tempo_select(&recs, 0).is_empty() && super::tempo_select(&recs, 3).is_empty());
        assert_eq!(super::tempo_select(&recs, 1), vec![0]);
    }

    /// rekordbox's own XML export against its own PQTZ for the same tracks. Run with
    /// PREPDECK_XML and PREPDECK_ANLZ_PAIRS set, --ignored.
    #[test]
    #[ignore]
    fn matches_rekordbox_xml_tempo_nodes() {
        let (Ok(xml), Ok(pairs)) = (std::env::var("PREPDECK_XML"), std::env::var("PREPDECK_ANLZ_PAIRS")) else { return };
        let text = std::fs::read_to_string(xml).unwrap();
        // Location -> TEMPO nodes as written.
        use quick_xml::events::Event;
        let mut r = quick_xml::Reader::from_str(&text);
        let mut by_loc: std::collections::HashMap<String, Vec<(String, String, String)>> = Default::default();
        let mut cur: Option<String> = None;
        loop {
            match r.read_event().unwrap() {
                Event::Start(e) | Event::Empty(e) => {
                    let a: std::collections::HashMap<String, String> = e.attributes().flatten().map(|a| (String::from_utf8_lossy(a.key.as_ref()).to_string(), a.unescape_value().unwrap().to_string())).collect();
                    match e.name().as_ref() {
                        b"TRACK" if a.contains_key("Location") => {
                            let loc = crate::importer::decode_location(&a["Location"]);
                            by_loc.entry(loc.clone()).or_default();
                            cur = Some(loc);
                        }
                        b"TEMPO" => {
                            if let Some(c) = &cur {
                                by_loc.get_mut(c).unwrap().push((a["Inizio"].clone(), a["Bpm"].clone(), a["Battito"].clone()));
                            }
                        }
                        _ => {}
                    }
                }
                Event::End(e) if e.name().as_ref() == b"TRACK" => cur = None,
                Event::Eof => break,
                _ => {}
            }
        }
        let (mut same, mut total, mut nodes) = (0, 0, 0);
        for line in std::fs::read_to_string(pairs).unwrap().lines() {
            let Some((path, base)) = line.split_once('\t') else { continue };
            let Some(want) = by_loc.get(path) else { continue };
            let b = std::fs::read(format!("{base}.DAT")).unwrap();
            let be32 = |o: usize| u32::from_be_bytes(b[o..o + 4].try_into().unwrap());
            let mut i = be32(4) as usize;
            let mut recs = vec![];
            while i + 12 <= b.len() {
                if &b[i..i + 4] == b"PQTZ" {
                    for k in 0..be32(i + 20) as usize {
                        let o = i + 24 + k * 8;
                        recs.push((u16::from_be_bytes([b[o], b[o + 1]]), u16::from_be_bytes([b[o + 2], b[o + 3]]), be32(o + 4)));
                    }
                }
                i += be32(i + 8) as usize;
            }
            let got: Vec<(String, String, String)> = tempo_keep(&recs).into_iter().map(|k| (format!("{:.3}", recs[k].2 as f64 * 0.001), format!("{:.2}", recs[k].1 as f64 * 0.01), recs[k].0.to_string())).collect();
            total += 1;
            nodes += want.len();
            if &got == want {
                same += 1;
            } else {
                eprintln!("differs: {path}\n  rekordbox {want:?}\n  ours      {got:?}");
            }
        }
        eprintln!("TEMPO nodes identical on {same} / {total} tracks ({nodes} nodes in rekordbox's XML)");
        assert_eq!(same, total);
    }
}

/// Unknown TRACK attributes (" Key=\"value\"…") and child elements (one line each, with
/// `pad`), written exactly as imported.
pub fn extra_xml(e: &crate::importer::XmlExtra, pad: &str) -> (String, String) {
    let attrs: String = e.attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect();
    let nodes: String = e
        .nodes
        .iter()
        .map(|(tag, a)| format!("{pad}<{tag}{}/>\n", a.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect::<String>()))
        .collect();
    // Elements with children or text go back exactly as they were written.
    let nodes = nodes + &e.raw.iter().map(|r| format!("{pad}{r}\n")).collect::<String>();
    (attrs, nodes)
}

/// A TRACK's TEMPO and cue POSITION_MARK elements: the imported originals while the grid /
/// cues are unchanged since an XML import, else from Recordbox's data as rekordbox writes them.
pub fn track_children(t: &Track, extra: &crate::importer::XmlExtra, grid: Option<BeatGrid>, cues: Vec<Cue>) -> String {
    let mut x = String::new();
        let raw = |attrs: &Vec<(String, String)>, tag: &str| format!("      <{tag}{}/>\n", attrs.iter().map(|(k, v)| format!(" {k}=\"{}\"", esc(v))).collect::<String>());
        if !extra.tempo.is_empty() && crate::importer::grid_fingerprint(&grid) == extra.grid_fp {
            // Grid unchanged since the XML import: the original TEMPO elements, as written.
            for a in &extra.tempo {
                x.push_str(&raw(a, "TEMPO"));
            }
        } else if let Some(g) = grid {
            // The per-beat records Recordbox writes to PQTZ, filtered as rekordbox does.
            let full = if t.raw_samples > 0 && t.sample_rate > 0 { t.raw_samples as f64 / t.sample_rate as f64 } else { t.duration };
            let recs: Vec<(u16, u16, u32)> = g.beats(full.max(t.duration)).iter().map(|b| (b.beat as u16, (b.bpm * 100.0).round() as u16, (b.time * 1000.0).round() as u32)).collect();
            for i in tempo_select(&recs, crate::prefs_bridge::xml_beatgrid_mode()) {
                let (beat, bpm, ms) = recs[i];
                x.push_str(&format!("      <TEMPO Inizio=\"{:.3}\" Bpm=\"{:.2}\" Metro=\"4/4\" Battito=\"{}\"/>\n", ms as f64 * 0.001, bpm as f64 * 0.01, beat));
            }
        }
        let marks_unchanged = !extra.marks.is_empty() && crate::importer::cues_fingerprint(&cues) == extra.cues_fp;
        if marks_unchanged {
            // Cues unchanged since the XML import: the original elements, as written.
            for a in &extra.marks {
                x.push_str(&raw(a, "POSITION_MARK"));
            }
        }
        for c in cues.into_iter().filter(|_| !marks_unchanged) {
            // rekordbox's XML writer emits numbered cues for slots A–H only (rekordbox-rea XML-003).
            if c.kind == CueKind::Hot && !(0..8).contains(&c.slot) {
                continue;
            }
            let num = if c.kind == CueKind::Hot { c.slot } else { -1 };
            // As rekordbox's writer: Type 4 and End whenever End is non-zero (rekordbox-rea XML-003;
            // the colour helper alone uses end > start).
            let end_ms = c.end.map(|e| (e * 1000.0).round() as i64).unwrap_or(0);
            let ty = if end_ms != 0 { 4 } else { 0 };
            let end = if end_ms != 0 { format!(" End=\"{:.3}\"", c.end.unwrap()) } else { String::new() };
            let rgb = if c.kind == CueKind::Hot {
                // An uncoloured hot cue gets its slot's default (loops their own), as rekordbox's
                // writer asks its default-colour helper (rekordbox-rea native-xml-trace §1).
                let idx = if (1..=64).contains(&c.color) { c.color } else { crate::model::hotcue_default_color(c.slot, c.is_loop()) };
                let [r, g, b] = hotcue_rgb(idx);
                format!(" Red=\"{r}\" Green=\"{g}\" Blue=\"{b}\"")
            } else {
                String::new()
            };
            x.push_str(&format!("      <POSITION_MARK Name=\"{}\" Type=\"{}\" Start=\"{:.3}\"{} Num=\"{}\"{}/>\n", esc(&c.comment), ty, c.start, end, num, rgb));
        }
    x
}
