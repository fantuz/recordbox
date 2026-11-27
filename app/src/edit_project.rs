//! EDIT mode projects: rekordbox's `.rbep` file (UTF-8 XML, one track, its score, the palette
//! and the quantize setting; spec: checks/re-7.2.19/edit-mode.md section 4), the project
//! status, default file naming and the recent list. Undo history, clipboard, selection, zoom
//! and loop state are not part of a project, and no audio or waveform data is stored.

use crate::edit_model::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const EXT: &str = "rbep";
pub const FILE_VERSION: f64 = 1.0;
pub const RECENT_MAX: usize = 20;

/// The collection track a project edits, as written in the file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SongRef {
    pub id: i64,
    pub uuid: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub filepath: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectData {
    pub version: f64,
    pub song: SongRef,
    pub score: ScoreSet,
    pub palettes: Vec<PaletteClip>,
    pub quantize: Quantize,
}

#[derive(Debug, PartialEq)]
pub enum LoadError {
    /// "The project file cannot be found."
    NotFound,
    /// "The project file is damaged."
    Damaged,
}

impl LoadError {
    pub fn message(&self) -> &'static str {
        match self {
            LoadError::NotFound => "The project file cannot be found.",
            LoadError::Damaged => "The project file is damaged.",
        }
    }
}

/// rekordbox's project status bits: 1 = a project is open, 2 = it has a file, 4 = modified.
pub fn status(open: bool, has_file: bool, modified: bool) -> u8 {
    if !open {
        return 0;
    }
    1 | if has_file { 2 } else { 0 } | if modified { 4 } else { 0 }
}

/// Where the save panel starts. rekordbox keeps its projects in ~/Music/PioneerDJ/Edit
/// Projects; recordbox uses a folder of its own so that it never writes among rekordbox's
/// files (a test run with PREPDECK_DATA keeps them inside that folder).
pub fn project_dir() -> PathBuf {
    if std::env::var("PREPDECK_DATA").is_ok() {
        return crate::db::data_dir().join("Edit Projects");
    }
    dirs::audio_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("recordbox").join("Edit Projects")
}

/// Characters a file name cannot hold become "_".
pub fn sanitized(title: &str) -> String {
    let s: String = title.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "Untitled".into()
    } else {
        s
    }
}

/// "<Title> (Edit).rbep", then "<Title> (Edit) (1).rbep", "(2)" ... while the name is taken.
pub fn default_file(dir: &Path, title: &str) -> PathBuf {
    let base = format!("{} (Edit)", sanitized(title));
    let mut p = dir.join(format!("{base}.{EXT}"));
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{base} ({n}).{EXT}"));
        n += 1;
    }
    p
}

/// A project file opened or saved goes to the front of the recent list (20 at most; files
/// that are gone drop out).
pub fn touch_recent(list: &mut Vec<String>, path: &Path) {
    let p = path.to_string_lossy().to_string();
    list.retain(|x| *x != p);
    list.insert(0, p);
    prune_recent(list);
}

pub fn prune_recent(list: &mut Vec<String>) {
    list.retain(|x| Path::new(x).is_file());
    list.truncate(RECENT_MAX);
}

// ---------------------------------------------------------------------------- writing

fn esc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let s: String = s.nfc().collect();
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;")
}

fn num(v: f64) -> String {
    if v <= NOT_LOOP {
        format!("{v:e}")
    } else {
        format!("{v}")
    }
}

fn write_cues(out: &mut String, ind: &str, tag: &str, v: &[CueLoop]) {
    out.push_str(&format!("{ind}<{tag}><data>\n"));
    for c in v {
        let a = &c.appended;
        out.push_str(&format!(
            "{ind}  <element cueid=\"{}\" loop=\"{}\" in=\"{}\" out=\"{}\">\n{ind}    <appended cueid=\"{}\" status=\"{}\" color=\"{}\" looplennumer=\"{}\" looplendenom=\"{}\" colortable=\"{}\">\n{ind}      <comment>{}</comment>\n{ind}    </appended>\n{ind}  </element>\n",
            c.cue_id,
            u8::from(c.is_loop()),
            num(c.pos_in),
            num(if c.is_loop() { c.pos_out } else { NOT_LOOP }),
            a.db_cue_id,
            a.status,
            a.color,
            a.loop_numer,
            a.loop_denom,
            a.color_table,
            esc(&a.comment)
        ));
    }
    out.push_str(&format!("{ind}</data></{tag}>\n"));
}

fn write_edit(out: &mut String, ind: &str, s: &ScoreSet) {
    out.push_str(&format!("{ind}<edit>\n{ind}  <position><data>\n"));
    for p in &s.position {
        out.push_str(&format!("{ind}    <section start=\"{}\" end=\"{}\" songstart=\"{}\" songend=\"{}\"/>\n", num(p.start), num(p.end), num(p.song_start), num(p.song_end)));
    }
    out.push_str(&format!("{ind}  </data></position>\n{ind}  <volume><data>\n"));
    for m in &s.mute {
        out.push_str(&format!("{ind}    <section start=\"{}\" end=\"{}\" vol=\"{}\"/>\n", num(m.start), num(m.end), if m.muted { 0 } else { 1 }));
    }
    out.push_str(&format!("{ind}  </data></volume>\n{ind}  <bpm><data>\n"));
    for b in &s.tempo {
        out.push_str(&format!("{ind}    <section start=\"{}\" end=\"{}\" bpm=\"{}\"/>\n", num(b.start), num(b.end), num(b.bpm)));
    }
    out.push_str(&format!("{ind}  </data></bpm>\n{ind}  <prepared>\n"));
    let i2 = format!("{ind}    ");
    write_cues(out, &i2, "hotcue", &s.prepared.hot);
    write_cues(out, &i2, "memorycue", &s.prepared.memory);
    out.push_str(&format!("{i2}<activecensor><data>\n"));
    for c in &s.prepared.censor {
        let params: String = c.appended.params.iter().enumerate().map(|(i, v)| format!(" {i}=\"{v}\"")).collect();
        out.push_str(&format!(
            "{i2}  <element in=\"{}\" out=\"{}\">\n{i2}    <appended acid=\"{}\" effect=\"{}\" on=\"{}\"><effectparams{params}/></appended>\n{i2}  </element>\n",
            num(c.pos_in),
            num(c.pos_out),
            c.appended.ac_id,
            c.appended.effect,
            u8::from(c.appended.on)
        ));
    }
    out.push_str(&format!("{i2}</data></activecensor>\n{ind}  </prepared>\n{ind}</edit>\n"));
}

/// The timeline grid: one beat per whole beat of the score, at the constant tempo.
fn write_mastergrid(out: &mut String, ind: &str, s: &ScoreSet) {
    out.push_str(&format!("{ind}<mastergrid indexoffset=\"{}\">\n{ind}  <data>\n", s.bar_offset));
    let beat_ms = s.beat_ms();
    for i in 0..=(s.len().round() as usize) {
        out.push_str(&format!("{ind}    <beat index=\"{i}\" bpm=\"{}\" position=\"{}\"/>\n", num(s.bpm), num(i as f64 * beat_ms)));
    }
    out.push_str(&format!("{ind}  </data>\n{ind}</mastergrid>\n"));
}

pub fn to_xml(p: &ProjectData) -> String {
    let mut o = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n<project>\n  <info>\n    <app>rekordbox</app>\n");
    o.push_str(&format!("    <version>{}</version>\n  </info>\n  <tracks>\n    <track trackid=\"1\">\n", num(p.version)));
    let s = &p.song;
    o.push_str(&format!(
        "      <song id=\"{}\" uuid=\"{}\">\n        <title>{}</title>\n        <artist>{}</artist>\n        <album>{}</album>\n        <filepath>{}</filepath>\n      </song>\n",
        s.id,
        esc(&s.uuid),
        esc(&s.title),
        esc(&s.artist),
        esc(&s.album),
        esc(&s.filepath)
    ));
    write_edit(&mut o, "      ", &p.score);
    let g = &p.score.song_grid;
    o.push_str(&format!("      <songgrid length=\"{}\" bpm=\"{}\" indexoffset=\"{}\">\n        <orggrid indexoffset=\"{}\">\n          <data>\n", g.length_ms.round() as i64, num(g.nominal_bpm), g.bar_offset(), g.org_bar_offset));
    for (i, (bpm, ms)) in g.org_beats.iter().enumerate() {
        o.push_str(&format!("            <beat index=\"{i}\" bpm=\"{}\" position=\"{}\"/>\n", num(*bpm), num(*ms)));
    }
    o.push_str("          </data>\n        </orggrid>\n      </songgrid>\n    </track>\n  </tracks>\n");
    write_mastergrid(&mut o, "  ", &p.score);
    o.push_str("  <palettes>\n");
    for (order, pal) in p.palettes.iter().enumerate() {
        o.push_str(&format!("    <palette trackid=\"1\" paletteid=\"{}\" order=\"{order}\">\n      <name>{}</name>\n", pal.id, esc(&pal.name)));
        write_edit(&mut o, "      ", &pal.clip);
        write_mastergrid(&mut o, "      ", &pal.clip);
        o.push_str("    </palette>\n");
    }
    o.push_str(&format!("  </palettes>\n  <settings quantizebeat=\"{}\"/>\n</project>\n", p.quantize.file_value()));
    o
}

#[derive(Debug, PartialEq)]
pub enum SaveError {
    /// "Cannot save. You don't have the write-permission in the folder."
    Permission,
    /// "Cannot save. The file is open or the save directory could not be found."
    Other,
}

impl SaveError {
    pub fn message(&self) -> &'static str {
        match self {
            SaveError::Permission => "Cannot save. You don't have the write-permission in the folder.",
            SaveError::Other => "Cannot save. The file is open or the save directory could not be found.",
        }
    }
}

/// Write the project: to a temporary file next to the target first, then over the target.
pub fn save(path: &Path, p: &ProjectData) -> Result<(), SaveError> {
    // rekordbox's own check before writing: a song id, and an absolute file path.
    if p.song.id == 0 || p.song.id != p.score.track_id || !Path::new(&p.song.filepath).is_absolute() || !p.score.is_valid() {
        return Err(SaveError::Other);
    }
    if !path.extension().and_then(|s| s.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case(EXT)) {
        return Err(SaveError::Other);
    }
    let map = |e: std::io::Error| if e.kind() == std::io::ErrorKind::PermissionDenied { SaveError::Permission } else { SaveError::Other };
    use std::io::Write;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(map)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(map)?;
    tmp.write_all(to_xml(p).as_bytes()).map_err(map)?;
    tmp.as_file().sync_all().map_err(map)?;
    tmp.persist(path).map_err(|e| map(e.error))?;
    Ok(())
}

// ---------------------------------------------------------------------------- reading

/// A small XML tree: what a project file needs (elements, attributes, text; declarations
/// and comments are skipped). Attribute names may start with a digit, as rekordbox's
/// `<effectparams 0="…"/>` does, which stricter parsers refuse.
#[derive(Debug, Default)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == name).map(|a| a.1.as_str())
    }
    fn f(&self, name: &str, default: f64) -> f64 {
        self.attr(name).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(default)
    }
    fn i(&self, name: &str, default: i64) -> i64 {
        self.attr(name).and_then(|v| v.trim().parse::<f64>().ok()).map(|v| v as i64).unwrap_or(default)
    }
    fn text_of(&self, name: &str) -> String {
        self.child(name).map(|c| c.text.trim().to_string()).unwrap_or_default()
    }
    fn path(&self, names: &[&str]) -> Option<&Node> {
        let mut n = self;
        for name in names {
            n = n.child(name)?;
        }
        Some(n)
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn parse_xml(src: &str) -> Option<Node> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<Node> = vec![Node::default()];
    while i < b.len() {
        if b[i] != b'<' {
            let end = src[i..].find('<').map(|k| i + k).unwrap_or(b.len());
            stack.last_mut()?.text.push_str(&unescape(&src[i..end]));
            i = end;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->")? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>")?;
            stack.last_mut()?.text.push_str(&rest[9..end]);
            i += end + 3;
        } else if rest.starts_with("<?") {
            i += rest.find("?>")? + 2;
        } else if rest.starts_with("<!") {
            i += rest.find('>')? + 1;
        } else if rest.starts_with("</") {
            let end = rest.find('>')?;
            let done = stack.pop()?;
            if done.name != rest[2..end].trim() {
                return None;
            }
            stack.last_mut()?.children.push(done);
            i += end + 1;
        } else {
            // An opening tag: name, then attributes, then ">" or "/>".
            let mut k = 1;
            let bytes = rest.as_bytes();
            while k < bytes.len() && !bytes[k].is_ascii_whitespace() && bytes[k] != b'>' && bytes[k] != b'/' {
                k += 1;
            }
            let mut node = Node { name: rest[1..k].to_string(), ..Default::default() };
            let mut closed = false;
            loop {
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                match bytes.get(k)? {
                    b'>' => {
                        k += 1;
                        break;
                    }
                    b'/' => {
                        if bytes.get(k + 1) != Some(&b'>') {
                            return None;
                        }
                        k += 2;
                        closed = true;
                        break;
                    }
                    _ => {
                        let eq = k + rest[k..].find('=')?;
                        let key = rest[k..eq].trim().to_string();
                        let mut q = eq + 1;
                        while bytes.get(q)?.is_ascii_whitespace() {
                            q += 1;
                        }
                        let quote = *bytes.get(q)?;
                        if quote != b'"' && quote != b'\'' {
                            return None;
                        }
                        let close = q + 1 + rest[q + 1..].find(quote as char)?;
                        node.attrs.push((key, unescape(&rest[q + 1..close])));
                        k = close + 1;
                    }
                }
            }
            i += k;
            if closed {
                stack.last_mut()?.children.push(node);
            } else {
                stack.push(node);
            }
        }
    }
    if stack.len() != 1 {
        return None;
    }
    stack.pop()?.children.into_iter().next()
}

fn data<'a>(n: &'a Node, tag: &str) -> Vec<&'a Node> {
    n.path(&[tag, "data"]).map(|d| d.children.iter().collect()).unwrap_or_default()
}

fn read_cues(prepared: &Node, tag: &str) -> Vec<CueLoop> {
    data(prepared, tag)
        .into_iter()
        .filter(|e| e.name == "element")
        .map(|e| {
            let ap = e.child("appended");
            let g = |k: &str| ap.map(|a| a.i(k, 0)).unwrap_or(0);
            CueLoop {
                cue_id: e.i("cueid", 0) as i32,
                pos_in: e.f("in", NOT_LOOP),
                // (rekordbox reads `out` only when `loop` is set.)
                pos_out: if e.i("loop", 0) != 0 { e.f("out", NOT_LOOP) } else { NOT_LOOP },
                appended: CueAppended { db_cue_id: g("cueid"), status: g("status") as i32, color: g("color") as u8, comment: ap.map(|a| a.text_of("comment")).unwrap_or_default(), loop_numer: g("looplennumer") as i32, loop_denom: g("looplendenom") as i32, color_table: g("colortable") as u8 },
            }
        })
        .collect()
}

fn read_edit(edit: &Node, track_id: i64, bar_offset: i32, grid: Arc<SongGrid>) -> Option<ScoreSet> {
    let position: Vec<PosSec> = data(edit, "position")
        .into_iter()
        .map(|s| {
            let (a, ss) = (s.f("start", NOT_LOOP), s.f("songstart", NOT_LOOP));
            PosSec { start: a, end: s.f("end", NOT_LOOP).max(a), song_start: ss, song_end: s.f("songend", NOT_LOOP).max(ss) }
        })
        .collect();
    let mute: Vec<MuteSec> = data(edit, "volume")
        .into_iter()
        .map(|s| {
            let a = s.f("start", NOT_LOOP);
            MuteSec { start: a, end: s.f("end", NOT_LOOP).max(a), muted: s.f("vol", 1.0) < 1.0 }
        })
        .collect();
    let tempo: Vec<BpmSec> = data(edit, "bpm")
        .into_iter()
        .map(|s| {
            let a = s.f("start", NOT_LOOP);
            BpmSec { start: a, end: s.f("end", NOT_LOOP).max(a), bpm: s.f("bpm", -1.0) }
        })
        .collect();
    let bpm = tempo.first()?.bpm;
    let prepared = match edit.child("prepared") {
        Some(p) => Prepared {
            hot: read_cues(p, "hotcue"),
            memory: read_cues(p, "memorycue"),
            censor: data(p, "activecensor")
                .into_iter()
                .filter(|e| e.name == "element")
                .map(|e| {
                    let ap = e.child("appended");
                    let mut params: Vec<(usize, i32)> = ap.and_then(|a| a.child("effectparams")).map(|ep| ep.attrs.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.trim().parse().ok()?))).collect()).unwrap_or_default();
                    params.sort();
                    Censor { pos_in: e.f("in", NOT_LOOP), pos_out: e.f("out", NOT_LOOP), appended: CensorAppended { ac_id: ap.map(|a| a.i("acid", 0)).unwrap_or(0) as i32, effect: ap.map(|a| a.i("effect", 0)).unwrap_or(0) as u8, on: ap.map(|a| a.i("on", 0)).unwrap_or(0) != 0, params: params.into_iter().map(|p| p.1).collect() } }
                })
                .collect(),
        },
        None => Prepared::default(),
    };
    let s = ScoreSet { track_id, bpm, bar_offset, position, mute, tempo, prepared, song_grid: grid };
    s.is_valid().then_some(s)
}

/// Read a project. `Err(Damaged)` for anything that is not a complete, valid project. A
/// `version` above `FILE_VERSION` still reads (the caller asks before using it).
pub fn from_xml(text: &str) -> Result<ProjectData, LoadError> {
    let bad = LoadError::Damaged;
    let root = parse_xml(text).filter(|r| r.name == "project").ok_or(LoadError::Damaged)?;
    let version: f64 = root.path(&["info", "version"]).and_then(|v| v.text.trim().parse().ok()).ok_or(LoadError::Damaged)?;
    let track = root.path(&["tracks", "track"]).ok_or(LoadError::Damaged)?;
    let sn = track.child("song").ok_or(LoadError::Damaged)?;
    let song = SongRef { id: sn.i("id", 0), uuid: sn.attr("uuid").unwrap_or_default().to_string(), title: sn.text_of("title"), artist: sn.text_of("artist"), album: sn.text_of("album"), filepath: sn.text_of("filepath") };
    if song.id == 0 {
        return Err(bad);
    }
    let sg = track.child("songgrid").ok_or(LoadError::Damaged)?;
    let org = sg.child("orggrid").ok_or(LoadError::Damaged)?;
    let org_beats: Vec<(f64, f64)> = data(sg, "orggrid").into_iter().filter(|b| b.name == "beat").map(|b| (b.f("bpm", -1.0), b.f("position", NOT_LOOP))).collect();
    if org_beats.is_empty() || org_beats.iter().any(|b| b.1 <= NOT_LOOP) || !(sg.f("bpm", -1.0) > 0.0) {
        return Err(bad);
    }
    let grid = Arc::new(SongGrid::new(org_beats, org.i("indexoffset", 0) as i32, sg.f("length", 0.0), sg.f("bpm", -1.0)));
    let master_off = root.child("mastergrid").map(|m| m.i("indexoffset", 0)).unwrap_or(0) as i32;
    let score = read_edit(track.child("edit").ok_or(LoadError::Damaged)?, song.id, master_off, grid.clone()).ok_or(LoadError::Damaged)?;
    let mut pals: Vec<(i64, PaletteClip)> = vec![];
    for p in root.child("palettes").map(|p| p.children.iter().filter(|c| c.name == "palette").collect::<Vec<_>>()).unwrap_or_default() {
        let order = p.i("order", -1);
        // (rekordbox rejects a file with an entry that has no order.)
        if order < 0 {
            return Err(bad);
        }
        let off = p.child("mastergrid").map(|m| m.i("indexoffset", 0)).unwrap_or(0) as i32;
        let clip = read_edit(p.child("edit").ok_or(LoadError::Damaged)?, song.id, off, grid.clone()).ok_or(LoadError::Damaged)?;
        pals.push((order, PaletteClip { id: p.i("paletteid", 0).max(0) as u32, name: p.text_of("name"), clip: Arc::new(clip) }));
    }
    pals.sort_by_key(|p| p.0);
    let quantize = Quantize::from_file(root.child("settings").map(|s| s.i("quantizebeat", 0)).unwrap_or(0) as i32);
    Ok(ProjectData { version, song, score, palettes: pals.into_iter().map(|p| p.1).collect(), quantize })
}

pub fn load(path: &Path) -> Result<ProjectData, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { LoadError::NotFound } else { LoadError::Damaged })?;
    from_xml(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit_model::tests::song;

    fn project() -> ProjectData {
        let mut e = EditSession::new(song(64));
        e.select_range(8.0, 16.0, 1);
        assert!(e.edit(EditCmd::Clone));
        e.select_range(32.0, 36.0, 1);
        assert!(e.edit(EditCmd::Clear));
        e.select_range(40.0, 44.0, 1);
        assert!(e.edit(EditCmd::Delete));
        e.select_range(16.0, 24.0, 1);
        e.add_to_palette("Drop & <break> \"A\"").unwrap();
        e.select_range(0.0, 4.0, 1);
        e.add_to_palette("Intro é").unwrap();
        ProjectData {
            version: FILE_VERSION,
            song: SongRef { id: 1, uuid: "abc-123".into(), title: "Tïtle <1> & more".into(), artist: "Artist".into(), album: "Album".into(), filepath: std::env::temp_dir().join("a b").join("track.mp3").to_string_lossy().into_owned() },
            score: (*e.cur).clone(),
            palettes: e.palettes.clone(),
            quantize: Quantize::Four,
        }
    }

    #[test]
    fn a_project_reads_back_as_it_was_written() {
        let p = project();
        let xml = to_xml(&p);
        let back = from_xml(&xml).unwrap();
        assert_eq!(back, p);
        // And again: the second file is the first one byte for byte.
        assert_eq!(to_xml(&back), xml);
        // The pieces rekordbox's reader looks for are there.
        assert!(xml.contains("<app>rekordbox</app>") && xml.contains("<version>1</version>"));
        assert!(xml.contains("<track trackid=\"1\">") && xml.contains("<settings quantizebeat=\"4\"/>"));
        assert!(xml.contains("vol=\"0\"") && xml.contains("out=\"-1.7976931348623157e308\""));
        assert!(xml.contains("<effectparams 0=\"1\" 1=\"2\"/>"));
        assert!(xml.contains("<palette trackid=\"1\" paletteid=\"1\" order=\"0\">"));
        // 120 BPM: the timeline grid's second beat is at 500 ms.
        assert!(xml.contains("<beat index=\"1\" bpm=\"120\" position=\"500\"/>"));
    }

    #[test]
    fn saving_is_atomic_and_loading_reports_what_is_wrong() {
        let dir = std::env::temp_dir().join(format!("recordbox_rbep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = project();
        let f = dir.join("sub").join("a.rbep");
        let original = dir.join("original.wav");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&original, b"audio").unwrap();
        assert!(save(&original, &p).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"audio");
        save(&f, &p).unwrap();
        assert_eq!(load(&f).unwrap(), p);
        assert_eq!(std::fs::read_dir(f.parent().unwrap()).unwrap().count(), 1, "no temporary file is left");
        assert_eq!(load(&dir.join("nope.rbep")), Err(LoadError::NotFound));
        std::fs::write(dir.join("bad.rbep"), "<project><info></info>").unwrap();
        assert_eq!(load(&dir.join("bad.rbep")), Err(LoadError::Damaged));
        // A relative file path or a score for another track is refused before writing.
        let mut q = p.clone();
        q.song.filepath = "track.mp3".into();
        assert_eq!(save(&dir.join("b.rbep"), &q), Err(SaveError::Other));
        assert!(!dir.join("b.rbep").exists());
        // Default names: "<Title> (Edit).rbep", then (1), (2).
        let d = dir.join("names");
        std::fs::create_dir_all(&d).unwrap();
        let n0 = default_file(&d, "My: Track?");
        assert_eq!(n0.file_name().unwrap().to_string_lossy(), "My_ Track_ (Edit).rbep");
        std::fs::write(&n0, "x").unwrap();
        let n1 = default_file(&d, "My: Track?");
        assert_eq!(n1.file_name().unwrap().to_string_lossy(), "My_ Track_ (Edit) (1).rbep");
        std::fs::write(&n1, "x").unwrap();
        assert_eq!(default_file(&d, "My: Track?").file_name().unwrap().to_string_lossy(), "My_ Track_ (Edit) (2).rbep");
        // Recent list: newest first, no doubles, gone files dropped, 20 at most.
        let mut recent = vec![];
        touch_recent(&mut recent, &n0);
        touch_recent(&mut recent, &n1);
        touch_recent(&mut recent, &n0);
        assert_eq!(recent, vec![n0.to_string_lossy().to_string(), n1.to_string_lossy().to_string()]);
        std::fs::remove_file(&n1).unwrap();
        prune_recent(&mut recent);
        assert_eq!(recent.len(), 1);
        for i in 0..25 {
            let f = d.join(format!("r{i}.rbep"));
            std::fs::write(&f, "x").unwrap();
            touch_recent(&mut recent, &f);
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert!(recent[0].ends_with("r24.rbep"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reader_follows_rekordboxs_rules() {
        // A file as rekordbox lays it out: wrapped lines, defaults left out, a newer version.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <info><app>rekordbox</app><version>2</version></info>
  <tracks><track trackid="1">
    <song id="77" uuid="u"><title>T &amp; U</title><artist>A</artist><album></album>
      <filepath>/x/y.mp3</filepath></song>
    <edit>
      <position><data><section start="0" end="8" songstart="0" songend="8"/>
        <section start="8" end="12" songstart="2" songend="6"/></data></position>
      <volume><data><section start="0" end="4"/><section start="4" end="12" vol="0.5"/></data></volume>
      <bpm><data><section start="0" end="12" bpm="100"/></data></bpm>
      <prepared>
        <hotcue><data>
          <element cueid="2" loop="0" in="3" out="9"><appended cueid="5" color="12"><comment>x &lt; y</comment></appended></element>
          <element cueid="3" loop="1" in="4" out="8"/>
        </data></hotcue>
        <memorycue><data/></memorycue>
        <activecensor><data><element in="1" out="2"><appended acid="4" effect="1" on="1"><effectparams 1="20" 0="10"/></appended></element></data></activecensor>
      </prepared>
    </edit>
    <songgrid length="5400" bpm="100" indexoffset="1">
      <orggrid indexoffset="0"><data>
        <beat index="0" bpm="100" position="300"/><beat index="1" bpm="100" position="900"/>
        <beat index="2" bpm="100" position="1500"/><beat index="3" bpm="100" position="2100"/>
        <beat index="4" bpm="100" position="2700"/><beat index="5" bpm="100" position="3300"/>
        <beat index="6" bpm="100" position="3900"/><beat index="7" bpm="100" position="4500"/>
      </data></orggrid>
    </songgrid>
  </track></tracks>
  <mastergrid indexoffset="1"><data/></mastergrid>
  <palettes/>
  <settings quantizebeat="7"/>
</project>"#;
        let p = from_xml(xml).unwrap();
        assert_eq!(p.version, 2.0);
        assert_eq!((p.song.id, p.song.title.as_str(), p.song.filepath.as_str()), (77, "T & U", "/x/y.mp3"));
        assert_eq!(p.score.len(), 12.0);
        assert_eq!(p.score.bpm, 100.0);
        assert_eq!(p.score.bar_offset, 1);
        // No vol = audible; vol below 1 = muted.
        assert_eq!(p.score.mute.iter().map(|m| m.muted).collect::<Vec<_>>(), vec![false, true]);
        // `out` only counts with loop set.
        assert!(!p.score.prepared.hot[0].is_loop() && p.score.prepared.hot[1].is_loop());
        assert_eq!((p.score.prepared.hot[0].appended.db_cue_id, p.score.prepared.hot[0].appended.color, p.score.prepared.hot[0].appended.comment.as_str()), (5, 12, "x < y"));
        assert_eq!(p.score.prepared.censor[0].appended.params, vec![10, 20]);
        assert_eq!(p.quantize, Quantize::Auto, "an unknown quantize value reads as AUTO");
        // The song grid is rebuilt from the original beats: one beat added in front.
        assert_eq!((p.score.song_grid.pre, p.score.song_grid.bar_offset()), (1, 1));
        // No version: damaged. A palette entry without an order: damaged. Sections that do
        // not line up: damaged.
        assert_eq!(from_xml(&xml.replace("<version>2</version>", "")), Err(LoadError::Damaged));
        assert_eq!(from_xml(&xml.replace("<palettes/>", "<palettes><palette paletteid=\"1\"><name>n</name></palette></palettes>")), Err(LoadError::Damaged));
        assert_eq!(from_xml(&xml.replace("start=\"8\" end=\"12\" songstart", "start=\"9\" end=\"12\" songstart")), Err(LoadError::Damaged));
        assert_eq!(from_xml("not xml"), Err(LoadError::Damaged));
        assert_eq!(status(false, false, false), 0);
        assert_eq!((status(true, false, false), status(true, false, true), status(true, true, false), status(true, true, true)), (1, 5, 3, 7));
    }
}
