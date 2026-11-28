//! Reading track metadata and artwork from audio files (read-only) and importing a rekordbox
//! XML collection export (read-only).

use crate::model::*;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::{Accessor, ItemKey};
use std::path::Path;

pub fn read_track(path: &Path) -> Track {
    let mut t = Track {
        path: path.to_string_lossy().to_string(),
        key: -1,
        autoload_hotcue: true,
        file_type: path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase(),
        date_added: chrono::Local::now().format("%Y-%m-%d").to_string(),
        ..Default::default()
    };
    if let Ok(md) = std::fs::metadata(path) {
        t.file_size = md.len();
        if let Ok(c) = md.created() {
            let dt: chrono::DateTime<chrono::Local> = c.into();
            t.date_created = dt.format("%Y-%m-%d").to_string();
        }
    }
    let Ok(tagged) = lofty::read_from_path(path) else {
        t.title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        return t;
    };
    let props = tagged.properties();
    t.duration = props.duration().as_secs_f64();
    t.sample_rate = props.sample_rate().unwrap_or(0);
    t.bitrate = props.audio_bitrate().unwrap_or(0);
    t.bit_depth = props.bit_depth().unwrap_or(0) as u32;
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        t.title = tag.title().map(|s| s.to_string()).unwrap_or_default();
        t.artist = tag.artist().map(|s| s.to_string()).unwrap_or_default();
        t.album = tag.album().map(|s| s.to_string()).unwrap_or_default();
        t.genre = tag.genre().map(|s| s.to_string()).unwrap_or_default();
        t.comment = tag.comment().map(|s| s.to_string()).unwrap_or_default();
        t.year = tag.year().unwrap_or(0) as i32;
        t.track_no = tag.track().unwrap_or(0) as i32;
        t.disc_no = tag.disk().unwrap_or(0) as i32;
        let g = |k: ItemKey| tag.get_string(&k).map(|s| s.to_string()).unwrap_or_default();
        t.album_artist = g(ItemKey::AlbumArtist);
        t.label = g(ItemKey::Label);
        t.remixer = g(ItemKey::Remixer);
        t.original_artist = g(ItemKey::OriginalArtist);
        t.composer = g(ItemKey::Composer);
        t.lyricist = g(ItemKey::Lyricist);
        t.grouping = g(ItemKey::ContentGroup);
        t.mix_name = g(ItemKey::TrackSubtitle);
        let rd = g(ItemKey::RecordingDate);
        if rd.len() >= 4 {
            t.release_date = rd.clone();
            if t.year == 0 {
                t.year = rd[..4].parse().unwrap_or(0);
            }
        }
        let bpm = g(ItemKey::Bpm);
        if let Ok(b) = bpm.trim().parse::<f64>() {
            if b > 0.0 && b < 1000.0 {
                t.bpm = b;
            }
        }
        let key = g(ItemKey::InitialKey);
        if !key.is_empty() {
            t.key = parse_key(&key);
            t.key_text = key.trim().to_string();
        }
        if let Some(pic) = tag.pictures().first() {
            let data = pic.data();
            let ext = match pic.mime_type() {
                Some(lofty::picture::MimeType::Png) => "png",
                _ => "jpg",
            };
            let mut h: u64 = 1469598103934665603;
            for b in data.iter().step_by(7) {
                h = (h ^ *b as u64).wrapping_mul(1099511628211);
            }
            h ^= data.len() as u64;
            let out = crate::db::artwork_dir().join(format!("{h:016x}.{ext}"));
            if !out.exists() {
                let _ = std::fs::write(&out, data);
            }
            t.artwork = out.to_string_lossy().to_string();
        }
    }
    if t.title.trim().is_empty() {
        t.title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    }
    t
}

/// Collect audio files from files and folders.
pub fn collect_audio(paths: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for p in paths {
        if p.is_dir() {
            for e in walkdir::WalkDir::new(p).follow_links(true).into_iter().flatten() {
                let ep = e.path();
                let hidden = ep.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with('.')).unwrap_or(false);
                if e.file_type().is_file() && !hidden && crate::audio::has_audio_extension(ep) {
                    out.push(ep.to_path_buf());
                }
            }
        } else if crate::audio::has_audio_extension(p) {
            out.push(p.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

// ------------------------------------------------------------------------------------------
// rekordbox XML (File > Export Collection in xml format) import
// ------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct XmlTrack {
    pub xml_id: String,
    pub track: Track,
    pub grid: Option<BeatGrid>,
    pub cues: Vec<Cue>,
    /// What Recordbox doesn't model, kept to be written back on XML export: unknown TRACK
    /// attributes and unknown child elements (e.g. fade-in/out and load marks).
    pub extra: XmlExtra,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct XmlExtra {
    /// Unknown TRACK attributes in document order.
    pub attrs: Vec<(String, String)>,
    /// Unknown child elements, as (tag, attributes in order).
    pub nodes: Vec<(String, Vec<(String, String)>)>,
    /// Known attributes whose imported text differs from how Recordbox writes the imported
    /// value: (attribute, original text or ABSENT, Recordbox's text at import). Written back
    /// while the field is unedited.
    #[serde(default)]
    pub orig: Vec<(String, String, String)>,
    /// The TEMPO and cue POSITION_MARK elements as imported (all attributes, in order), with
    /// fingerprints of the grid and cues they became: while those are unchanged, export writes
    /// the original elements (meter, precision and unknown attributes kept).
    #[serde(default)]
    pub tempo: Vec<Vec<(String, String)>>,
    #[serde(default)]
    pub marks: Vec<Vec<(String, String)>>,
    #[serde(default)]
    pub grid_fp: String,
    #[serde(default)]
    pub cues_fp: String,
    /// The TrackID the XML gave this track (written back when free, so playlists made
    /// elsewhere keep referring to the same IDs).
    #[serde(default)]
    pub xml_id: String,
    /// Unknown elements with children or text, exactly as written (including nested ones).
    #[serde(default)]
    pub raw: Vec<String>,
}

/// Fingerprint of a beat grid for "unchanged since import".
pub fn grid_fingerprint(g: &Option<crate::model::BeatGrid>) -> String {
    serde_json::to_string(g).unwrap_or_default()
}

/// Fingerprint of a track's cues (ids aside) for "unchanged since import".
pub fn cues_fingerprint(cues: &[Cue]) -> String {
    let mut v: Vec<String> = cues.iter().map(|c| format!("{:?}|{}|{:.6}|{:?}|{}|{}", c.kind, c.slot, c.start, c.end.map(|e| (e * 1e6).round() as i64), c.color, c.comment)).collect();
    v.sort();
    v.join("\n")
}

impl XmlExtra {
    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.nodes.is_empty() && self.orig.is_empty() && self.tempo.is_empty() && self.marks.is_empty() && self.xml_id.is_empty() && self.raw.is_empty()
    }
}

/// TRACK attributes Recordbox reads (and writes back from its own data).
pub const XML_TRACK_ATTRS: [&str; 26] = [
    "TrackID", "Name", "Artist", "Composer", "Album", "Grouping", "Genre", "Kind", "Size", "TotalTime", "DiscNumber", "TrackNumber", "Year", "AverageBpm", "DateAdded", "BitRate", "SampleRate", "Comments", "PlayCount", "Rating", "Location", "Remixer", "Tonality", "Label", "Mix", "Colour",
];

pub struct XmlNode {
    pub name: String,
    pub folder: bool,
    pub children: Vec<XmlNode>,
    /// Track references: TrackIDs, or (KeyType="1") decoded file locations.
    pub track_keys: Vec<String>,
    pub by_location: bool,
    /// Intelligent playlist rule (from a rekordbox library; My Tag values are still
    /// "\u{1}category\u{1f}tag" names, resolved when the playlist is created).
    pub smart: Option<crate::smart::SmartRule>,
}

pub struct XmlLibrary {
    pub tracks: Vec<XmlTrack>,
    pub playlists: Vec<XmlNode>,
    /// Problems found while reading (bad escapes, count mismatches), for the import report.
    pub warnings: Vec<String>,
}

pub fn decode_location(loc: &str) -> String {
    let s = loc.strip_prefix("file://localhost").or_else(|| loc.strip_prefix("file://")).unwrap_or(loc);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn parse_rekordbox_xml(text: &str) -> anyhow::Result<XmlLibrary> {
    use quick_xml::events::Event;
    // quick-xml skips a byte-order mark without counting it in its positions; drop it first so
    // positions index `text` (raw element capture slices by them).
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = quick_xml::Reader::from_str(text);
    let mut tracks: Vec<XmlTrack> = vec![];
    let mut stack: Vec<XmlNode> = vec![];
    let mut roots: Vec<XmlNode> = vec![];
    let mut in_collection = false;
    let mut cur: Option<XmlTrack> = None;
    let mut tempos: Vec<(f64, f64, u8)> = vec![];
    let mut bad_attrs = 0usize;
    let mut raw_capture: Option<(usize, usize)> = None;
    let mut declared: Option<usize> = None;
    loop {
        let before = reader.buffer_position() as usize;
        let ev = reader.read_event()?;
        // Inside an unknown element of a track: copy it whole (children and text included).
        if let Some((depth, start)) = raw_capture.as_mut() {
            match &ev {
                Event::Start(_) => *depth += 1,
                Event::End(_) => {
                    *depth -= 1;
                    if *depth == 0 {
                        let end = reader.buffer_position() as usize;
                        let raw = text.get(*start..end).unwrap_or("").to_string();
                        if let Some(t) = cur.as_mut() {
                            t.extra.raw.push(raw);
                        }
                        raw_capture = None;
                    }
                }
                Event::Eof => anyhow::bail!("the file ends inside an element (truncated?)"),
                _ => {}
            }
            continue;
        }
        if let Event::Start(e) = &ev {
            let n = e.name();
            if cur.is_some() && in_collection && !matches!(n.as_ref(), b"TEMPO" | b"POSITION_MARK" | b"TRACK") {
                raw_capture = Some((1usize, before));
                continue;
            }
        }
        let (e, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            Event::End(e) => {
                let name = e.name();
                match name.as_ref() {
                    b"COLLECTION" => in_collection = false,
                    b"TRACK" if in_collection => {
                        if let Some(mut t) = cur.take() {
                            finish_grid(&mut t, &tempos);
                            tracks.push(t);
                        }
                        tempos.clear();
                    }
                    b"NODE" => {
                        if let Some(n) = stack.pop() {
                            if let Some(p) = stack.last_mut() {
                                p.children.push(n);
                            } else {
                                roots.push(n);
                            }
                        }
                    }
                    _ => {}
                }
                (None, false)
            }
            Event::Eof => {
                // A document cut short (unclosed collection, track or playlist) is refused
                // rather than imported in part.
                if in_collection || cur.is_some() || !stack.is_empty() {
                    anyhow::bail!("the file ends before its collection or playlists are complete (truncated?)");
                }
                break;
            }
            _ => (None, false),
        };
        let Some(e) = e else { continue };
        let mut attrs = std::collections::HashMap::new();
        let mut ordered: Vec<(String, String)> = vec![];
        for a in e.attributes() {
            let Ok(a) = a else {
                bad_attrs += 1;
                continue;
            };
            let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
            let v = match a.unescape_value() {
                Ok(v) => v.to_string(),
                Err(_) => {
                    // Kept as written rather than emptied, and reported.
                    bad_attrs += 1;
                    String::from_utf8_lossy(&a.value).to_string()
                }
            };
            ordered.push((k.clone(), v.clone()));
            attrs.insert(k, v);
        }
        let get = |k: &str| attrs.get(k).cloned().unwrap_or_default();
        let num = |k: &str| attrs.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        match e.name().as_ref() {
            b"COLLECTION" => {
                in_collection = true;
                declared = attrs.get("Entries").and_then(|v| v.parse::<usize>().ok());
            }
            b"TRACK" if in_collection => {
                let loc = decode_location(&get("Location"));
                let t = Track {
                    path: loc,
                    title: get("Name"),
                    artist: get("Artist"),
                    composer: get("Composer"),
                    album: get("Album"),
                    grouping: get("Grouping"),
                    genre: get("Genre"),
                    file_type: get("Kind").split_whitespace().next().unwrap_or("").to_ascii_lowercase(),
                    file_size: num("Size") as u64,
                    duration: num("TotalTime"),
                    disc_no: num("DiscNumber") as i32,
                    track_no: num("TrackNumber") as i32,
                    year: num("Year") as i32,
                    bpm: num("AverageBpm"),
                    date_added: get("DateAdded"),
                    bitrate: num("BitRate") as u32,
                    sample_rate: num("SampleRate") as u32,
                    comment: get("Comments"),
                    play_count: num("PlayCount") as u32,
                    rating: match num("Rating") as u32 {
                        0 => 0,
                        r if r <= 51 => 1,
                        r if r <= 102 => 2,
                        r if r <= 153 => 3,
                        r if r <= 204 => 4,
                        _ => 5,
                    },
                    remixer: get("Remixer"),
                    key: parse_key(&get("Tonality")),
                    key_text: get("Tonality").trim().to_string(),
                    label: get("Label"),
                    mix_name: get("Mix"),
                    autoload_hotcue: true,
                    analyzed: 0,
                    ..Default::default()
                };
                let colour = get("Colour").to_ascii_uppercase();
                let color = match colour.trim_start_matches("0X") {
                    "FF007F" => 1,
                    "FF0000" => 2,
                    "FFA500" => 3,
                    "FFFF00" => 4,
                    "00FF00" => 5,
                    "25FDE9" => 6,
                    "0000FF" => 7,
                    "660099" => 8,
                    _ => 0,
                };
                let extra = XmlExtra { attrs: ordered.iter().filter(|(k, _)| !XML_TRACK_ATTRS.contains(&k.as_str())).cloned().collect(), nodes: vec![], orig: vec![], xml_id: get("TrackID"), ..Default::default() };
                let mut extra = extra;
                let track = Track { color, ..t };
                for (k, v) in crate::xmlexport::track_attrs(&track) {
                    let original = attrs.get(k).cloned().unwrap_or_else(|| if k == "Colour" { String::new() } else { crate::xmlexport::ABSENT.to_string() });
                    if original != v {
                        extra.orig.push((k.to_string(), original, v));
                    }
                }
                let xt = XmlTrack { xml_id: get("TrackID"), track, grid: None, cues: vec![], extra };
                if empty {
                    tracks.push(xt);
                } else {
                    cur = Some(xt);
                }
            }
            b"TEMPO" => {
                tempos.push((num("Inizio"), num("Bpm"), get("Battito").parse::<u8>().unwrap_or(1)));
                if let Some(t) = cur.as_mut() {
                    t.extra.tempo.push(ordered.clone());
                }
            }
            b"POSITION_MARK" if cur.is_some() && !matches!(get("Type").trim(), "0" | "4" | "") => {
                // Fade-in (1), fade-out (2), load (3) and unknown types: kept as written.
                if let Some(t) = cur.as_mut() {
                    t.extra.nodes.push(("POSITION_MARK".into(), ordered.clone()));
                }
            }
            b"POSITION_MARK" => {
                if let Some(t) = cur.as_mut() {
                    let start = num("Start");
                    let ty = num("Type") as i32;
                    let end = if ty == 4 { attrs.get("End").and_then(|v| v.parse::<f64>().ok()) } else { None };
                    let n = attrs.get("Num").and_then(|v| v.parse::<i32>().ok()).unwrap_or(-1);
                    let (r, g, b) = (num("Red") as u8, num("Green") as u8, num("Blue") as u8);
                    let kind = if n >= 0 { CueKind::Hot } else { CueKind::Memory };
                    let color = if kind == CueKind::Hot && (r, g, b) != (0, 0, 0) { nearest_hot_color([r, g, b]) } else if kind == CueKind::Hot { HOTCUE_DEFAULT } else { 0 };
                    t.cues.push(Cue { id: 0, track_id: 0, kind, slot: n, start, end, color, comment: get("Name"), active_loop: false });
                    t.extra.marks.push(ordered.clone());
                }
            }
            tag if cur.is_some() && in_collection && !matches!(tag, b"TEMPO" | b"POSITION_MARK" | b"TRACK") => {
                // Unknown element inside a collection track: kept (its attributes) as written.
                let name = String::from_utf8_lossy(tag).to_string();
                if let Some(t) = cur.as_mut() {
                    t.extra.nodes.push((name, ordered.clone()));
                }
            }
            b"NODE" => {
                let ty = get("Type");
                let name = get("Name");
                let node = XmlNode { name, folder: ty == "0", children: vec![], track_keys: vec![], by_location: get("KeyType") == "1", smart: None };
                if empty {
                    if let Some(p) = stack.last_mut() {
                        p.children.push(node);
                    } else {
                        roots.push(node);
                    }
                } else {
                    // Bounded nesting (a real collection is a few levels deep).
                    if stack.len() >= 256 {
                        anyhow::bail!("playlists are nested more than 256 levels deep");
                    }
                    stack.push(node);
                }
            }
            b"TRACK" if !in_collection => {
                if let Some(n) = stack.last_mut() {
                    let k = get("Key");
                    n.track_keys.push(if n.by_location { decode_location(&k) } else { k });
                }
            }
            _ => {}
        }
    }
    // The XML wraps everything in a ROOT folder node.
    let playlists = if roots.len() == 1 && roots[0].name == "ROOT" { roots.pop().unwrap().children } else { roots };
    let mut warnings = vec![];
    if bad_attrs > 0 {
        warnings.push(format!("{bad_attrs} attribute value(s) had invalid escapes and were kept as written"));
    }
    if let Some(n) = declared {
        if n != tracks.len() {
            warnings.push(format!("the collection declares {n} tracks but contains {}", tracks.len()));
        }
    }
    Ok(XmlLibrary { tracks, playlists, warnings })
}

fn finish_grid(t: &mut XmlTrack, tempos: &[(f64, f64, u8)]) {
    if tempos.is_empty() {
        return;
    }
    // Every TEMPO entry is kept: the original also writes same-tempo entries where its
    // millisecond beat positions drift from the rounded tempo, so they carry timing.
    let mut anchors: Vec<GridAnchor> = vec![];
    for (time, bpm, beat) in tempos {
        if anchors.last().map(|a| *time <= a.time).unwrap_or(false) {
            continue;
        }
        anchors.push(GridAnchor { time: *time, bpm: *bpm, beat: (*beat).clamp(1, 4) });
    }
    t.grid = Some(BeatGrid::from_anchors(anchors));
}

#[cfg(test)]
mod keytype_tests {
    #[test]
    fn location_keyed_playlist() {
        let x = r#"<?xml version="1.0" encoding="UTF-8"?><DJ_PLAYLISTS Version="1.0.0"><COLLECTION Entries="1">
<TRACK TrackID="7" Name="A" Location="file://localhost/Users/x/My%20Song.mp3"/></COLLECTION>
<PLAYLISTS><NODE Type="0" Name="ROOT" Count="1"><NODE Name="ByLoc" Type="1" KeyType="1" Entries="1">
<TRACK Key="file://localhost/Users/x/My%20Song.mp3"/></NODE></NODE></PLAYLISTS></DJ_PLAYLISTS>"#;
        let lib = super::parse_rekordbox_xml(x).unwrap();
        let n = &lib.playlists[0];
        assert!(n.by_location);
        assert_eq!(n.track_keys, vec!["/Users/x/My Song.mp3".to_string()]);
        assert_eq!(lib.tracks[0].track.path, "/Users/x/My Song.mp3");
    }
}

pub fn nearest_hot_color(rgb: [u8; 3]) -> u8 {
    let mut best = (u32::MAX, HOTCUE_DEFAULT);
    for (i, v) in HOTCUE_PALETTE.iter().enumerate() {
        let c = [(v >> 16) as u8, (v >> 8) as u8, *v as u8];
        let d: u32 = (0..3).map(|k| (c[k] as i32 - rgb[k] as i32).pow(2) as u32).sum();
        if d < best.0 {
            best = (d, i as u8 + 1);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_xml() {
        let x = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0"><PRODUCT Name="x" Version="7"/>
<COLLECTION Entries="1">
<TRACK TrackID="7" Name="Song" Artist="Me" Kind="MP3 File" TotalTime="200" AverageBpm="128.00" Tonality="Am" Rating="153" Location="file://localhost/Users/a/Music/My%20Song.mp3" Colour="0xFF0000">
<TEMPO Inizio="0.025" Bpm="128.00" Metro="4/4" Battito="1"/>
<POSITION_MARK Name="Drop" Type="0" Start="32.025" Num="0" Red="230" Green="40" Blue="40"/>
<POSITION_MARK Name="" Type="4" Start="1.0" End="2.875" Num="-1"/>
</TRACK></COLLECTION>
<PLAYLISTS><NODE Type="0" Name="ROOT" Count="1"><NODE Name="Set" Type="1" KeyType="0" Entries="1"><TRACK Key="7"/></NODE></NODE></PLAYLISTS>
</DJ_PLAYLISTS>"#;
        let lib = parse_rekordbox_xml(x).unwrap();
        assert_eq!(lib.tracks.len(), 1);
        let t = &lib.tracks[0];
        assert_eq!(t.track.path, "/Users/a/Music/My Song.mp3");
        assert_eq!(t.track.rating, 3);
        assert_eq!(t.track.color, 2);
        assert_eq!(t.cues.len(), 2);
        assert_eq!(t.cues[0].kind, CueKind::Hot);
        assert!(t.cues[1].is_loop());
        assert_eq!(t.grid.as_ref().unwrap().anchors.len(), 1);
        assert_eq!(lib.playlists.len(), 1);
        assert_eq!(lib.playlists[0].track_keys, vec!["7"]);
    }
}

#[cfg(test)]
mod xml_children_tests {
    #[test]
    fn unchanged_tempo_and_cues_come_back_verbatim() {
        let x = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0"><COLLECTION Entries="1">
<TRACK TrackID="1" Name="A" Location="file://localhost/x/a.mp3" AverageBpm="128.00">
<TEMPO Inizio="0.1204" Bpm="128.00" Metro="3/4" Battito="1"/>
<POSITION_MARK Name="Drop" Type="0" Start="12.34567" Num="0" Red="40" Green="226" Blue="20" Future="x"/>
</TRACK></COLLECTION><PLAYLISTS><NODE Type="0" Name="ROOT" Count="0"></NODE></PLAYLISTS></DJ_PLAYLISTS>"#;
        let lib = super::parse_rekordbox_xml(x).unwrap();
        let xt = &lib.tracks[0];
        let mut extra = xt.extra.clone();
        extra.grid_fp = super::grid_fingerprint(&xt.grid);
        extra.cues_fp = super::cues_fingerprint(&xt.cues);
        let out = crate::xmlexport::track_children(&xt.track, &extra, xt.grid.clone(), xt.cues.clone());
        assert!(out.contains(r#"<TEMPO Inizio="0.1204" Bpm="128.00" Metro="3/4" Battito="1"/>"#), "{out}");
        assert!(out.contains(r#"Start="12.34567" Num="0" Red="40" Green="226" Blue="20" Future="x"/>"#), "{out}");
        // A moved cue is written from Recordbox's data instead (the grid still verbatim).
        let mut moved = xt.cues.clone();
        moved[0].start = 13.0;
        let out2 = crate::xmlexport::track_children(&xt.track, &extra, xt.grid.clone(), moved);
        assert!(out2.contains(r#"Start="13.000""#) && !out2.contains("Future") && out2.contains("Metro=\"3/4\""), "{out2}");
    }
}

#[cfg(test)]
mod xml_extra_tests {
    #[test]
    fn keeps_unknown_track_fields_and_marks() {
        let x = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0"><COLLECTION Entries="1">
<TRACK TrackID="7" Name="A &amp; B" Location="file://localhost/x/a.mp3" Future="1.23456789" AverageBpm="128.0040" Tonality="1d" Rating="60">
<TEMPO Inizio="0.120" Bpm="128.00" Metro="4/4" Battito="1"/>
<POSITION_MARK Name="" Type="0" Start="1.000" Num="0" Red="40" Green="226" Blue="20"/>
<POSITION_MARK Name="fade" Type="1" Start="2.5000004" Num="-1"/>
<NEWTHING A="b"/>
<GROUPED Kind="x"><ITEM N="1">text &amp; more</ITEM><ITEM N="2"/></GROUPED>
</TRACK></COLLECTION><PLAYLISTS><NODE Type="0" Name="ROOT" Count="0"></NODE></PLAYLISTS></DJ_PLAYLISTS>"#;
        let lib = super::parse_rekordbox_xml(x).unwrap();
        // The same document with a byte-order mark captures the same raw elements.
        let with_bom = super::parse_rekordbox_xml(&format!("\u{feff}{x}")).unwrap();
        assert_eq!(with_bom.tracks[0].extra.raw, lib.tracks[0].extra.raw);
        let t = &lib.tracks[0];
        assert_eq!(t.cues.len(), 1);
        assert_eq!(t.extra.attrs, vec![("Future".to_string(), "1.23456789".to_string())]);
        assert_eq!(t.extra.nodes.len(), 2);
        assert_eq!(t.extra.nodes[0].0, "POSITION_MARK");
        assert!(t.extra.nodes[0].1.contains(&("Start".to_string(), "2.5000004".to_string())));
        assert_eq!(t.extra.nodes[1], ("NEWTHING".to_string(), vec![("A".to_string(), "b".to_string())]));
        // Unchanged fields come back with their original text; edited ones from Recordbox.
        let line = crate::xmlexport::track_line(&super::Track { id: 7, ..t.track.clone() }, &t.extra);
        assert!(line.contains(" Name=\"A &amp; B\""), "{line}");
        assert!(line.contains(" Location=\"file://localhost/x/a.mp3\""));
        assert!(line.contains(" Tonality=\"1d\""), "{line}");
        assert!(line.contains(" Rating=\"60\""), "{line}");
        assert!(line.contains(" AverageBpm=\"128.0040\""), "{line}");
        assert!(!line.contains("Grouping="), "absent attributes stay absent: {line}");
        assert!(line.contains(" Future=\"1.23456789\""));
        let mut edited = t.track.clone();
        edited.id = 7;
        edited.rating = 4;
        let line2 = crate::xmlexport::track_line(&edited, &t.extra);
        assert!(line2.contains(" Rating=\"204\"") && line2.contains(" Tonality=\"1d\""), "{line2}");
        // A truncated document is refused.
        let cut = &x[..x.find("</TRACK>").unwrap()];
        assert!(super::parse_rekordbox_xml(cut).is_err());
        // Written back unchanged.
        let (a, n) = crate::xmlexport::extra_xml(&t.extra, "      ");
        assert_eq!(a, " Future=\"1.23456789\"");
        assert!(n.contains("<POSITION_MARK Name=\"fade\" Type=\"1\" Start=\"2.5000004\" Num=\"-1\"/>"));
        assert!(n.contains("<NEWTHING A=\"b\"/>"));
        assert!(n.contains(r#"<GROUPED Kind="x"><ITEM N="1">text &amp; more</ITEM><ITEM N="2"/></GROUPED>"#), "{n}");
    }
}
