//! Export vocals or the original mix minus its vocals, retaining the source timeline in the local collection.
use crate::app::*;
use crate::edit_render::{self as render, Encode, Mp3Rate};
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[path = "stem_export_audio.rs"]
mod audio_export;

/// Preferences > Extensions > STEMS > Stem Export.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StemExportPrefs {
    /// "Add exported stem tracks to the playlist they are exported from".
    pub to_playlist: bool,
    /// Index into `FORMAT_NAMES`: 0 Same as the original, 1 WAV, 2 FLAC, 3 AIFF, 4 M4A, 5 MP3.
    pub format: u8,
    /// Export folder; empty: `default_dir()`.
    pub dir: String,
    /// The explanation pop-up comes up at the next export.
    pub explain: bool,
}

impl Default for StemExportPrefs {
    fn default() -> Self {
        StemExportPrefs {
            to_playlist: true,
            format: 0,
            dir: String::new(),
            explain: true,
        }
    }
}

pub const FORMAT_NAMES: [&str; 6] = ["Same as the original", "WAV", "FLAC", "AIFF", "M4A (AAC, macOS)", "MP3"];

/// recordbox's own folder for exported stems, beside the Edit renders (`ui_edit::render_dir`).
/// A test run with PREPDECK_DATA keeps it inside that folder ("Stem Exports", since the data
/// folder's "stems" is the cache of separated stems and the file system ignores case).
pub fn default_dir() -> PathBuf {
    if std::env::var("PREPDECK_DATA").is_ok() {
        return crate::db::data_dir().join("Stem Exports");
    }
    dirs::audio_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("recordbox")
        .join("Stems")
}

pub fn export_dir(p: &StemExportPrefs) -> PathBuf {
    if p.dir.trim().is_empty() {
        default_dir()
    } else {
        PathBuf::from(p.dir.trim())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Vocals,
    Instrumental,
}

impl Part {
    pub fn suffix(self) -> &'static str {
        match self {
            Part::Vocals => "Vocals",
            Part::Instrumental => "Instrumental",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Part::Vocals => "vocals",
            Part::Instrumental => "instrumental",
        }
    }
}

/// "<Artist> - <Title> (Vocals)", without characters a file name cannot hold.
pub fn base_name(artist: &str, title: &str, part: Part) -> String {
    let title = if title.trim().is_empty() {
        "Untitled"
    } else {
        title.trim()
    };
    let s = if artist.trim().is_empty() {
        format!("{title} ({})", part.suffix())
    } else {
        format!("{} - {title} ({})", artist.trim(), part.suffix())
    };
    let mut name = crate::edit_project::sanitized(&s);
    // Leave room for a numbered suffix on filesystems with 255-byte name components.
    if name.len() > 200 {
        let mut cut = 200;
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        name.truncate(cut);
    }
    name
}

/// What the original file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wav,
    Aiff,
    Flac,
    Mp3,
    Aac,
    Alac,
    OtherLossless,
    OtherLossy,
}

impl Kind {
    pub fn lossy(self) -> bool {
        matches!(self, Kind::Mp3 | Kind::Aac | Kind::OtherLossy)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub kind: Kind,
    pub frames: Option<u64>,
    pub legacy_aiff_tail: u8,
    pub rate: u32,
    pub ch: u16,
    /// Bits per sample of a lossless original (0: not known).
    pub bits: u16,
    /// Bitrate in kbps (for VBR the average; 0: not known).
    pub kbps: u32,
    pub vbr: bool,
}

/// How the export is written.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub enc: Encode,
    pub ext: &'static str,
    pub name: &'static str,
    pub rate: u32,
    pub ch: u16,
    /// Set when the original's format has no encoder here and another one is used.
    pub fallback: Option<String>,
}

impl Plan {
    pub fn lossy(&self) -> bool {
        matches!(self.enc.format, 3 | 4)
    }
    /// "MP3 320 kbps", "FLAC 24-bit" ...
    pub fn describe(&self) -> String {
        match (self.enc.format, self.enc.mp3) {
            (4, Mp3Rate::Cbr(k)) => format!("MP3 {k} kbps"),
            (4, Mp3Rate::Vbr(q)) => format!("MP3 VBR V{q}"),
            (3, _) => format!("M4A (AAC) {} kbps", self.enc.aac_bps / 1000),
            _ => format!("{} {}-bit", self.name, self.enc.bits),
        }
    }
}

/// Typical average bitrate of LAME's -V0 .. -V9.
const VBR_TYPICAL: [u32; 10] = [245, 225, 190, 175, 165, 130, 115, 100, 85, 65];

/// The VBR quality whose typical bitrate is not above `kbps` (so the file is not bigger).
pub fn vbr_quality(kbps: u32) -> u8 {
    if kbps == 0 {
        return 2;
    }
    VBR_TYPICAL.iter().position(|t| *t <= kbps).unwrap_or(9) as u8
}

/// A rate MP3 and AAC can take: the same when they can, else 48 or 44.1 kHz.
pub fn lossy_rate(rate: u32) -> u32 {
    match rate {
        8000 | 11025 | 12000 | 16000 | 22050 | 24000 | 32000 | 44100 | 48000 => rate,
        r if r > 48000 && r % 48000 == 0 => 48000,
        r if r > 48000 => 44100,
        _ => 44100,
    }
}

/// The format of an export: `setting` is `StemExportPrefs::format`.
pub fn plan(setting: u8, s: &SourceInfo) -> Plan {
    let bits = if !s.kind.lossy() && s.bits > 16 { 24 } else { 16 };
    let ch = if s.ch == 1 { 1 } else { 2 };
    let mp3 = if s.kind == Kind::Mp3 && s.vbr {
        Mp3Rate::Vbr(vbr_quality(s.kbps))
    } else {
        Mp3Rate::Cbr(render::mp3_bitrate(
            lossy_rate(s.rate),
            if s.kind.lossy() { s.kbps } else { 0 },
        ))
    };
    let aac_bps = if s.kind.lossy() && s.kbps > 0 {
        s.kbps.clamp(64, 320) * 1000
    } else {
        256_000
    };
    let enc = |format: u8| Encode {
        format,
        bits,
        flac_level: 5,
        aac_bps,
        mp3,
    };
    let mut fallback = None;
    let mut format = match setting {
        1 => 1,
        2 => 0,
        3 => 2,
        4 => 3,
        5 => 4,
        _ => match s.kind {
            Kind::Wav => 1,
            Kind::Aiff => 2,
            Kind::Flac => 0,
            Kind::Mp3 => 4,
            Kind::Aac => 3,
            Kind::Alac => {
                fallback = Some("ALAC has no encoder in recordbox, so it was written as FLAC (lossless)".to_string());
                0
            }
            Kind::OtherLossless => {
                fallback = Some(
                    "the original's format has no encoder in recordbox, so it was written as FLAC (lossless)"
                        .to_string(),
                );
                0
            }
            Kind::OtherLossy => {
                fallback = Some(
                    "the original's format has no encoder in recordbox, so it was written as M4A (AAC)".to_string(),
                );
                3
            }
        },
    };
    if format == 3 && !Path::new("/usr/bin/afconvert").is_file() && setting != 4 {
        format = 0;
        fallback = Some("the AAC encoder is unavailable here, so the export uses FLAC".into());
    }
    let (ext, name) = match format {
        0 => ("flac", "FLAC"),
        2 => ("aiff", "AIFF"),
        3 => ("m4a", "M4A"),
        4 => ("mp3", "MP3"),
        _ => ("wav", "WAV"),
    };
    let mut e = enc(format);
    if matches!(format, 3 | 4) {
        e.bits = 16;
    }
    let rate = if matches!(format, 3 | 4) {
        lossy_rate(s.rate)
    } else {
        s.rate
    };
    Plan {
        enc: e,
        ext,
        name,
        rate,
        ch,
        fallback,
    }
}

/// Read what the original is from its container and codec (no decoding).
pub fn probe_source(path: &str, kbps: u32) -> Option<SourceInfo> {
    use symphonia::core::codecs::*;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let mut file = std::fs::File::open(path).ok()?;
    let aiff = crate::audio::aiff::frame_bounds(&mut file).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    hint.with_extension(&ext);
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    let t = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?
        .clone();
    let c = t.codec_params.codec;
    let kind = if c == CODEC_TYPE_MP3 {
        Kind::Mp3
    } else if c == CODEC_TYPE_AAC {
        Kind::Aac
    } else if c == CODEC_TYPE_ALAC {
        Kind::Alac
    } else if c == CODEC_TYPE_FLAC {
        Kind::Flac
    } else if c == CODEC_TYPE_VORBIS || c == CODEC_TYPE_OPUS || c == CODEC_TYPE_MP1 || c == CODEC_TYPE_MP2 {
        Kind::OtherLossy
    } else if matches!(ext.as_str(), "wav" | "wave") {
        Kind::Wav
    } else if aiff.is_some() || matches!(ext.as_str(), "aif" | "aiff" | "aifc") {
        Kind::Aiff
    } else {
        Kind::OtherLossless
    };
    let rate = t.codec_params.sample_rate.unwrap_or(44100);
    let ch = t.codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);
    let bits = t
        .codec_params
        .bits_per_sample
        .or(t.codec_params.bits_per_coded_sample)
        .unwrap_or(0) as u16;
    let (mut kbps, mut vbr) = (kbps, false);
    if kind == Kind::Mp3 {
        if let Some((hdr_kbps, is_vbr)) = mp3_rate_info(path) {
            vbr = is_vbr;
            if !is_vbr {
                kbps = hdr_kbps;
            }
        }
    }
    Some(SourceInfo {
        kind,
        frames: aiff.map(|b| b.frames as u64).or(t.codec_params.n_frames),
        legacy_aiff_tail: aiff.map_or(0, |b| b.legacy_tail),
        rate,
        ch,
        bits,
        kbps,
        vbr,
    })
}

/// An MP3's constant bitrate (from its first audio frame) and whether it is VBR (a Xing or
/// VBRI header; LAME writes "Info" in a constant-bitrate file).
pub fn mp3_rate_info(path: &str) -> Option<(u32, bool)> {
    use std::io::Read;
    let mut d = Vec::new();
    std::fs::File::open(path).ok()?.take(1 << 20).read_to_end(&mut d).ok()?;
    mp3_rate_info_in(&d)
}

pub fn mp3_rate_info_in(d: &[u8]) -> Option<(u32, bool)> {
    const BR1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const BR2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let mut p = 0usize;
    if d.len() >= 10 && &d[..3] == b"ID3" {
        p = 10
            + (((d[6] as usize & 0x7f) << 21)
                | ((d[7] as usize & 0x7f) << 14)
                | ((d[8] as usize & 0x7f) << 7)
                | (d[9] as usize & 0x7f));
    }
    let mut tagged_vbr = None;
    while p + 4 <= d.len() {
        let h = &d[p..p + 4];
        let ok = h[0] == 0xFF
            && h[1] & 0xE0 == 0xE0
            && (h[1] >> 1) & 3 == 1
            && h[2] >> 4 != 15
            && h[2] >> 4 != 0
            && (h[2] >> 2) & 3 != 3;
        if !ok {
            p += 1;
            continue;
        }
        let m1 = h[1] & 0x08 != 0;
        let kbps = if m1 { BR1 } else { BR2 }[(h[2] >> 4) as usize];
        let sr = if m1 {
            [44100, 48000, 32000]
        } else if h[1] & 0x10 != 0 {
            [22050, 24000, 16000]
        } else {
            [11025, 12000, 8000]
        }[((h[2] >> 2) & 3) as usize];
        let len = (if m1 { 144 } else { 72 }) * kbps as usize * 1000 / sr + ((h[2] >> 1) & 1) as usize;
        let frame = &d[p..(p + len.max(4)).min(d.len())];
        let has = |pat: &[u8]| frame.windows(4).any(|w| w == pat);
        if tagged_vbr.is_none() && (has(b"Xing") || has(b"VBRI") || has(b"Info")) {
            // The tag frame: its own bitrate says nothing; the next frame does.
            tagged_vbr = Some(has(b"Xing") || has(b"VBRI"));
            p += len.max(1);
            continue;
        }
        return Some((kbps, tagged_vbr.unwrap_or(false)));
    }
    None
}

/// The playlist an export started from goes into: the browser's view when it is a normal
/// playlist (not Collection, a folder, a smart playlist, a history, the Tag List, a device or
/// any other list). A playlist filtered by the search field is still that playlist.
pub fn view_playlist(view: &View, nodes: &[PlaylistNode]) -> Option<i64> {
    match view {
        View::Playlist(id) => nodes
            .iter()
            .find(|n| n.id == *id && n.kind == NodeKind::Playlist)
            .map(|n| n.id),
        _ => None,
    }
}

/// Where the new track goes in the playlist: right after the source's row (and after the
/// stem tracks of the same source already right behind it), else at the end.
pub fn insert_at(list: &[i64], src: i64, of_src: impl Fn(i64) -> bool) -> usize {
    match list.iter().position(|t| *t == src) {
        Some(mut i) => {
            i += 1;
            while i < list.len() && of_src(list[i]) {
                i += 1;
            }
            i
        }
        None => list.len(),
    }
}

// ---------------------------------------------------------------------------- the first export

/// The explanation pop-up waiting for an answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub reqs: Vec<Request>,
    /// The playlist the export was started in (id, name).
    pub playlist: Option<(i64, String)>,
    /// "Also add exported stem tracks to the playlist they are exported from".
    pub add: bool,
    /// "Don't show this again".
    pub dont_show: bool,
}

/// Whether an export started now asks first.
pub fn asks(p: &StemExportPrefs) -> bool {
    p.explain
}

/// The pop-up's answer: OK saves the choices and returns true (the export goes ahead);
/// Cancel changes nothing and starts nothing.
pub fn answer(p: &mut StemExportPrefs, a: &Ask, ok: bool) -> bool {
    if !ok {
        return false;
    }
    if a.playlist.is_some() {
        p.to_playlist = a.add;
    }
    if a.dont_show {
        p.explain = false;
    }
    true
}

// ---------------------------------------------------------------------------- the queue

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub track: i64,
    pub part: Part,
    /// The normal playlist that was the browser's view when the export was started.
    pub view_playlist: Option<i64>,
    /// Whether it goes into that playlist (the setting when the export was queued).
    pub to_playlist: bool,
}

struct Active {
    req: Request,
    path: String,
    name: String,
    model: &'static crate::stems::ModelSpec,
    cancel: Arc<AtomicBool>,
    rx: Option<crossbeam_channel::Receiver<Result<audio_export::Staged, String>>>,
    started_separation: bool,
    owned_separation: Option<Arc<AtomicBool>>,
}

#[derive(Default)]
pub struct StemExport {
    queue: VecDeque<Request>,
    active: Option<Active>,
    pub ask: Option<Ask>,
    total: usize,
    finished: usize,
    failed: usize,
    last_error: String,
    made: Option<HashSet<i64>>,
}

/// Key of the setting that links an exported stem track to its source ("<id>:<part>").
pub fn link_key(id: i64) -> String {
    format!("stem_of_{id}")
}

/// The tracks made by stem exports.
pub fn made_set(app: &mut App) -> &HashSet<i64> {
    if app.stemx.made.is_none() {
        let mut set = HashSet::new();
        if let Ok(mut st) = app
            .db
            .conn
            .prepare("SELECT key FROM settings WHERE substr(key, 1, 8) = 'stem_of_'")
        {
            if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                for k in rows.flatten() {
                    if let Some(id) = k.strip_prefix("stem_of_").and_then(|v| v.parse::<i64>().ok()) {
                        set.insert(id);
                    }
                }
            }
        }
        app.stemx.made = Some(set);
    }
    app.stemx.made.as_ref().unwrap()
}

pub fn is_stem_track(app: &mut App, id: i64) -> bool {
    made_set(app).contains(&id)
}

/// Start exporting `part` of `tracks` (the selected tracks, or the deck's track). The first
/// time, the explanation comes up first.
pub fn request(app: &mut App, tracks: &[i64], part: Part, _from_strip: bool) {
    if app.stemx.ask.is_some() {
        return;
    }
    if crate::prefs_bridge::library_protected(app) {
        app.toast_err("Stem exports add tracks to the local collection. Turn off Library Protection to export.");
        return;
    }
    let tracks: Vec<i64> = tracks
        .iter()
        .copied()
        .filter(|t| app.track(*t).is_some_and(|t| !t.missing))
        .collect();
    if tracks.is_empty() {
        app.toast_err("There is no track to export stems from.");
        return;
    }
    let pl = view_playlist(&app.view, &app.nodes);
    let reqs: Vec<Request> = tracks
        .iter()
        .map(|t| Request {
            track: *t,
            part,
            view_playlist: pl,
            to_playlist: false,
        })
        .collect();
    if asks(&app.prefs.stemx) {
        let playlist = pl.and_then(|id| app.node(id).map(|n| (id, n.name.clone())));
        app.stemx.ask = Some(Ask {
            reqs,
            playlist,
            add: app.prefs.stemx.to_playlist,
            dont_show: false,
        });
        return;
    }
    enqueue(app, reqs);
}

/// The pop-up was answered.
pub fn answered(app: &mut App, a: Ask, ok: bool) {
    if answer(&mut app.prefs.stemx, &a, ok) {
        app.save_prefs();
        enqueue(app, a.reqs);
    }
}

fn enqueue(app: &mut App, reqs: Vec<Request>) {
    if app.stemx.active.is_none() && app.stemx.queue.is_empty() {
        app.stemx.total = 0;
        app.stemx.finished = 0;
        app.stemx.failed = 0;
        app.stemx.last_error.clear();
    }
    let to = app.prefs.stemx.to_playlist;
    for mut r in reqs {
        r.to_playlist = to && r.view_playlist.is_some();
        // The same export already waiting is not queued twice.
        if app.stemx.queue.iter().any(|q| q.track == r.track && q.part == r.part)
            || app
                .stemx
                .active
                .as_ref()
                .is_some_and(|a| a.req.track == r.track && a.req.part == r.part)
        {
            continue;
        }
        app.stemx.queue.push_back(r);
        app.stemx.total += 1;
    }
    poll(app);
}

impl Drop for Active {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn stop(app: &mut App) {
    app.stemx.queue.clear();
    if let Some(a) = &app.stemx.active {
        a.cancel.store(true, Ordering::Relaxed);
        // The token is unique to the worker. A pre-existing deck job or a subsequent
        // job for another track is never stopped by this queue.
        if let Some(token) = &a.owned_separation {
            token.store(true, Ordering::Relaxed);
        }
        app.ctx.request_repaint();
    }
}

pub fn busy(app: &App) -> bool {
    app.stemx.active.is_some() || !app.stemx.queue.is_empty()
}
pub fn busy_for(app: &App, track: Option<i64>) -> bool {
    track.is_some_and(|id| {
        app.stemx.active.as_ref().is_some_and(|a| a.req.track == id) || app.stemx.queue.iter().any(|r| r.track == id)
    })
}
pub fn status(app: &App) -> Option<String> {
    let a = app.stemx.active.as_ref()?;
    let stage = if a.cancel.load(Ordering::Relaxed) {
        "Stopping export".into()
    } else if a.rx.is_some() {
        "Encoding and checking audio".into()
    } else if let Some((_, p)) = app.stem.separating.filter(|s| s.0 == a.req.track) {
        format!("Separating {:.0}%", p * 100.0)
    } else {
        "Waiting for completed stems".into()
    };
    Some(format!("{stage} · {}", a.req.part.suffix()))
}
pub fn footer_text(app: &App) -> Option<String> {
    let a = app.stemx.active.as_ref()?;
    Some(format!(
        "Export {} of {} · {} · {}",
        app.stemx.finished + 1,
        app.stemx.total,
        a.name,
        status(app)?
    ))
}

pub fn poll(app: &mut App) {
    let mut a = match app.stemx.active.take() {
        Some(a) => a,
        None => {
            let Some(req) = app.stemx.queue.pop_front() else { return };
            let Some(t) = app.track(req.track) else {
                complete(app, Err("Source track left the collection.".into()));
                return;
            };
            let Some(model) = app
                .stem
                .model
                .as_deref()
                .or(app.prefs.stem_model.as_deref())
                .and_then(crate::stems::spec)
            else {
                complete(
                    app,
                    Err("Wait for the hardware and separation model check before exporting.".into()),
                );
                return;
            };
            Active {
                path: t.path.clone(),
                name: base_name(&t.artist, &t.title, req.part),
                req,
                model,
                rx: None,
                cancel: Default::default(),
                started_separation: false,
                owned_separation: None,
            }
        }
    };
    if let Some(rx) = &a.rx {
        let result = match rx.try_recv() {
            Ok(result) => Some(result),
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Some(Err("The export worker stopped unexpectedly.".into()))
            }
            Err(crossbeam_channel::TryRecvError::Empty) => None,
        };
        if let Some(result) = result {
            if a.cancel.load(Ordering::Relaxed) {
                complete(app, Err("stopped".into()));
            } else {
                complete_result(app, &a, result);
            }
            return;
        }
    } else if a.cancel.load(Ordering::Relaxed) {
        complete(app, Err("stopped".into()));
        return;
    } else if let Err(e) = start_or_wait(app, &mut a) {
        complete(app, Err(format!("{}: {e}", a.name)));
        return;
    }
    app.stemx.active = Some(a);
    app.ctx.request_repaint_after(std::time::Duration::from_millis(250));
}

fn complete_result(app: &mut App, a: &Active, result: Result<audio_export::Staged, String>) {
    let result = result
        .and_then(|s| import(app, a, s))
        .map_err(|e| format!("{}: {e}", a.name));
    complete(app, result);
}

fn complete(app: &mut App, result: Result<String, String>) {
    app.stemx.finished += 1;
    match result {
        Ok(msg) => app.toast(msg),
        Err(e) if e == "stopped" => app.toast("Stem export stopped."),
        Err(e) => {
            app.stemx.failed += 1;
            app.stemx.last_error = e.clone();
            app.toast_err(e);
        }
    }
    if app.stemx.queue.is_empty() && app.stemx.total > 1 && app.stemx.failed > 0 {
        app.toast_err(format!(
            "Stem export: {} of {} attempted tracks failed. {}",
            app.stemx.failed, app.stemx.finished, app.stemx.last_error
        ));
    }
    app.ctx.request_repaint();
}

fn start_or_wait(app: &mut App, a: &mut Active) -> Result<(), String> {
    let t = app
        .track(a.req.track)
        .filter(|t| t.path == a.path)
        .ok_or("The source track changed during export.")?;
    let kbps = t.bitrate;
    let cache = crate::stems::cache_path(&a.path, a.model.id);
    let deck = (0..4).find(|&n| app.deck_of(n).track == Some(a.req.track) && app.deck_of(n).audio.is_some());
    let parts = deck.and_then(|n| {
        let key = app.deck_of(n).stem_cache_key.as_ref()?;
        if key.model != a.model.id || key.path != cache {
            return None;
        }
        // Live windows are kept separately in stem_preview. Only immutable, completed stems qualify.
        app.player.decks[usize::from(n)].lock().stems.clone()
    });
    if parts.is_none() && crate::stems::existing_cache_path(&cache).is_none() {
        let Some(n) = deck else {
            return Err("Load this track into a deck and separate it with the selected model, then export.".into());
        };
        if app.stem.pending_for(a.req.track) || app.stem.separating.is_some() || app.with_deck(n, |a| a.loading_stems())
        {
            return Ok(());
        }
        if a.started_separation {
            return Err(app
                .stem
                .error_for(Some(a.req.track))
                .map(|e| e.detail.clone())
                .unwrap_or_else(|| "The deck separation did not complete.".into()));
        }
        if app.stem.model.as_deref() != Some(a.model.id) {
            return Err("The selected separation model changed. Start the export again.".into());
        }
        if app.stem.error_for(Some(a.req.track)).is_some() || crate::stems::backend_hold().is_some() {
            return Err("Review the separation error on the deck and use Retry CPU before exporting.".into());
        }
        if !crate::stems::model_ready(a.model) {
            return Err("Download the selected STEMS model in Preferences before exporting.".into());
        }
        app.with_deck(n, |app| app.separate_loaded());
        if app.stem.separating.is_some_and(|(id, _)| id == a.req.track) {
            a.owned_separation = Some(app.stem.cancel.clone());
        }
        a.started_separation = true;
        return Ok(());
    }
    let work = audio_export::Work {
        path: a.path.clone(),
        dir: export_dir(&app.prefs.stemx),
        part: a.req.part,
        setting: app.prefs.stemx.format,
        kbps,
        audio: deck.and_then(|n| app.deck_of(n).audio.clone()),
        stems: parts,
        cache,
        model: a.model.id,
    };
    let cancel = a.cancel.clone();
    let ctx = app.ctx.clone();
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("rb-stem-export".into())
        .spawn(move || {
            let _ = tx.send(audio_export::stage(work, &cancel));
            ctx.request_repaint();
        })
        .map_err(|e| format!("Cannot start the stem export worker: {e}"))?;
    a.rx = Some(rx);
    Ok(())
}

/// A race with another instance, an existing collection reference or a dangling symlink
/// simply advances the suffix. The staging file remains owned until publication succeeds.
fn publish(
    mut file: tempfile::NamedTempFile,
    dir: &Path,
    base: &str,
    ext: &str,
    reserved: impl Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    for n in 1..=100_000 {
        let suffix = if n == 1 { String::new() } else { format!(" {n}") };
        let path = dir.join(format!("{base}{suffix}.{ext}"));
        if reserved(&path) {
            continue;
        }
        match file.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => file = e.file,
            Err(e) => return Err(format!("Could not publish the stem file: {}", e.error)),
        }
    }
    Err("Cannot find an unused export filename.".into())
}

fn import(app: &mut App, a: &Active, staged: audio_export::Staged) -> Result<String, String> {
    if crate::prefs_bridge::library_protected(app) {
        return Err("Library Protection was enabled. Export cancelled.".into());
    }
    let src = app
        .track(a.req.track)
        .filter(|t| t.path == a.path)
        .cloned()
        .ok_or("The source track changed during export.")?;
    let audio_export::Staged {
        file,
        dir,
        plan,
        shift,
        source,
    } = staged;
    if source != audio_export::SourceStamp::read(&a.path)? {
        return Err("The original file changed during export.".into());
    }
    let path = publish(file, &dir, &a.name, plan.ext, |p| {
        app.db.track_by_path(&p.to_string_lossy()).is_some()
    })?;
    let playlist = a
        .req
        .view_playlist
        .filter(|_| a.req.to_playlist)
        .and_then(|id| app.node(id))
        .filter(|n| n.kind == NodeKind::Playlist)
        .map(|n| (n.id, n.name.clone()));
    let phrases = app
        .stored_phrases(src.id)
        .or_else(|| {
            (0..4).find_map(|n| {
                let deck = app.deck_of(n);
                if deck.track == Some(src.id) {
                    deck.phrases.clone()
                } else {
                    None
                }
            })
        })
        .or_else(|| rb_phrases(app, &src));
    let result = add_track(
        &app.db,
        &src,
        &path,
        a.req.part,
        shift,
        phrases.as_ref(),
        app.track_tags.get(&src.id),
        playlist.as_ref().map(|p| p.0),
    );
    let id = result.map_err(|e| format!("File saved at {}. Its library import failed: {e}", path.display()))?;
    app.stemx.made = None;
    app.edit.edited = None;
    app.reload_tracks();
    app.reload_tags();
    app.refresh_cue_counts(id);
    app.analyze_derived_audio(id);
    if playlist.is_some() {
        app.reload_playlists();
    }
    app.ext.matches = app.db.track_matches().unwrap_or_default();
    let playlist = playlist
        .map(|(_, name)| format!(" and playlist “{name}”"))
        .unwrap_or_default();
    let info = plan.fallback.as_deref().map(|n| format!(" ({n})")).unwrap_or_default();
    Ok(format!(
        "Added {} to the collection{playlist} · {}{info}",
        path.display(),
        plan.describe()
    ))
}

fn add_track(
    db: &crate::db::Db,
    src: &Track,
    path: &Path,
    part: Part,
    shift: f64,
    phrases: Option<&crate::phrase::Structure>,
    tags: Option<&HashSet<i64>>,
    playlist: Option<i64>,
) -> anyhow::Result<i64> {
    let tx = db.conn.unchecked_transaction()?;
    anyhow::ensure!(
        db.track(src.id).is_some_and(|current| current.path == src.path),
        "The source library reference changed during export."
    );
    let mut track = crate::importer::read_track(path);
    track.title = format!("{} ({})", src.display_title(), part.suffix());
    track.artist = src.artist.clone();
    track.bpm = src.bpm;
    track.album = src.album.clone();
    track.album_artist = src.album_artist.clone();
    track.genre = src.genre.clone();
    track.label = src.label.clone();
    track.remixer = src.remixer.clone();
    track.original_artist = src.original_artist.clone();
    track.composer = src.composer.clone();
    track.lyricist = src.lyricist.clone();
    track.mix_name = src.mix_name.clone();
    track.comment = src.comment.clone();
    track.grouping = src.grouping.clone();
    track.message = src.message.clone();
    track.year = src.year;
    track.track_no = src.track_no;
    track.disc_no = src.disc_no;
    track.release_date = src.release_date.clone();
    track.key = src.key;
    track.key_text = src.key_text.clone();
    track.key_manual = src.key_manual;
    track.rating = src.rating;
    track.color = src.color;
    track.artwork = src.artwork.clone();
    let id = db.insert_track(&track)?;
    if let Some(mut grid) = db.grid_checked(src.id)? {
        for anchor in &mut grid.anchors {
            anchor.time += shift;
        }
        db.save_grid(id, &grid)?;
    }
    for c in db.cues(src.id)? {
        db.save_cue(&Cue {
            id: 0,
            track_id: id,
            start: c.start + shift,
            end: c.end.map(|v| v + shift),
            ..c
        })?;
    }
    if let Some(st) = &phrases {
        db.set_setting_checked(&format!("phrase_edit:{id}"), &serde_json::to_string(st)?)?;
    }
    db.set_setting_checked(&link_key(id), &format!("{}:{}", src.id, part.key()))?;
    if let Some(tags) = tags {
        for &tag in tags {
            db.set_track_tag(id, tag, true)?;
        }
    }
    if let Some(pl) = playlist {
        let mut tracks = db.playlist_tracks(pl)?;
        let prefix = format!("{}:", src.id);
        let at = insert_at(&tracks, src.id, |t| {
            db.setting(&link_key(t)).is_some_and(|s| s.starts_with(&prefix))
        });
        tracks.insert(at, id);
        db.set_playlist_tracks(pl, &tracks)?;
    }
    db.set_match(src.id, id, true)?;
    tx.commit()?;
    Ok(id)
}

fn rb_phrases(app: &App, src: &Track) -> Option<crate::phrase::Structure> {
    // Ask SQLite for this source only. A full library's map can contain hundreds of thousands of paths.
    let path: String = app.db.conn.query_row(
        "SELECT j.value FROM settings,json_each(settings.value) AS j WHERE settings.key='rb_anlz' AND j.key=?1 LIMIT 1",
        [&src.path], |r| r.get(0)).ok()?;
    let path = PathBuf::from(path).with_extension("EXT");
    if std::fs::metadata(&path).ok()?.len() > 32 * 1024 * 1024 {
        return None;
    }
    crate::export::anlz::read_sections(&path)
        .get("PSSI")
        .and_then(|s| crate::phrase::parse(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn source(dir: &Path, frames: usize, ch: u16, bits: u16) -> (String, Arc<crate::stems::Stems>) {
        let sr = 44100;
        let path = dir.join("Source.wav");
        // A quiet but nonzero mix with an independently audible vocal transient near frame zero.
        let mut file = File::create_new(&path).unwrap();
        render::write_pcm(&mut file, 1, frames, sr, ch, bits, 0, |dst| {
            dst.fill(1);
            Ok(())
        })
        .unwrap();
        let mut data = vec![0.0; 8 * frames];
        for i in 0..frames {
            let amplitude = if i < 1500 { 0.8 } else { 0.14 };
            let v = amplitude * (2.0 * std::f32::consts::PI * i as f32 * 880.0 / 44100.0).sin();
            data[6 * frames + i] = v;
            data[7 * frames + i] = v;
        }
        (
            path.to_string_lossy().into_owned(),
            Arc::new(crate::stems::Stems { len: frames, data }),
        )
    }

    use std::fs::File;

    fn work(path: &str, stems: Arc<crate::stems::Stems>, dir: &Path, setting: u8, part: Part) -> audio_export::Work {
        let model = crate::stems::CATALOG[1].id;
        audio_export::Work {
            path: path.into(),
            dir: dir.into(),
            part,
            setting,
            kbps: 320,
            audio: None,
            stems: Some(stems),
            cache: crate::stems::cache_path(path, model),
            model,
        }
    }

    #[test]
    fn mp3_bitrates_match_the_sample_rate_and_encoder_modes() {
        assert_eq!(render::mp3_bitrate(11025, 320), 64);
        assert_eq!(render::mp3_bitrate(24000, 320), 160);
        assert_eq!(render::mp3_bitrate(44100, 144), 128);
        assert_eq!(render::mp3_bitrate(44100, 56), 48);
        assert_eq!(render::mp3_bitrate(8000, 16), 16);
        assert_eq!(render::mp3_bitrate(48000, 0), 320);
    }

    #[test]
    fn aiff_validation_rejects_inconsistent_comm_ssnd_sizes_and_padding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("odd.aiff");
        let mut bytes = Vec::new();
        render::write_pcm(&mut bytes, 2, 5, 44100, 1, 24, 0, |dst| {
            dst.fill(1);
            Ok(())
        })
        .unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(audio_export::written_aiff_frames(&path).unwrap(), 5);
        for offset in [7usize, 25, 45, 49, 53, bytes.len() - 1] {
            let mut broken = bytes.clone();
            broken[offset] ^= 1;
            std::fs::write(&path, broken).unwrap();
            assert!(
                audio_export::written_aiff_frames(&path).is_err(),
                "accepted corrupt byte {offset}"
            );
        }
        std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(audio_export::written_aiff_frames(&path).is_err());
    }

    #[test]
    fn lossless_outputs_keep_mono_24_bit_and_odd_frame_counts() {
        let dir = tempfile::tempdir().unwrap();
        let (path, stems) = source(dir.path(), 9001, 1, 24);
        let original = std::fs::read(&path).unwrap();
        for setting in [1, 2, 3] {
            let staged = audio_export::stage(
                work(&path, stems.clone(), dir.path(), setting, Part::Instrumental),
                &AtomicBool::new(false),
            )
            .unwrap();
            let name = staged.file.path().to_string_lossy();
            let info = probe_source(&name, 0).unwrap();
            assert_eq!((info.rate, info.ch, info.bits), (44100, 1, 24));
            let back = crate::audio::decode_file(&name).unwrap();
            assert_eq!((back.frames(), staged.shift), (9001, 0.0));
            for (i, frame) in back.samples.chunks_exact(2).enumerate() {
                let expected = (1.0f32 / 8388608.0 - stems.data[6 * 9001 + i]) * 8388608.0;
                assert_eq!((frame[0] * 8388608.0).round(), expected.round());
                assert_eq!(frame[0], frame[1]);
            }
        }
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn legacy_aiff_cache_with_metadata_tail_exports_only_real_audio() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("Metadata.aiff");
        let frames = 9001;
        let mut bytes = Vec::new();
        render::write_pcm(&mut bytes, 2, frames, 44100, 1, 24, 0, |dst| {
            dst.fill(0);
            Ok(())
        })
        .unwrap();
        bytes.extend(b"ANNO\x00\x00\x00\x04abcd");
        let size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&size.to_be_bytes());
        std::fs::write(&path, bytes).unwrap();
        let path = path.to_str().unwrap();
        let make = |tail| {
            let len = frames + tail;
            let mut data = vec![0.0; len * 8];
            for c in [6, 7] {
                let channel = &mut data[c * len..(c + 1) * len];
                channel[frames - 1] = 0.25;
                channel[frames..].fill(1.0);
            }
            Arc::new(crate::stems::Stems { len, data })
        };
        let staged =
            audio_export::stage(work(path, make(2), d.path(), 1, Part::Vocals), &AtomicBool::new(false)).unwrap();
        let decoded = crate::audio::decode_file(&staged.file.path().to_string_lossy()).unwrap();
        assert_eq!(decoded.frames(), frames);
        assert_eq!(decoded.samples[2 * (frames - 1)], 0.25);
        assert!(audio_export::stage(work(path, make(3), d.path(), 1, Part::Vocals), &AtomicBool::new(false)).is_err());
    }

    fn check_lossy_timeline(setting: u8) {
        let dir = tempfile::tempdir().unwrap();
        let (path, stems) = source(dir.path(), 44100, 2, 16);
        let staged = audio_export::stage(
            work(&path, stems, dir.path(), setting, Part::Vocals),
            &AtomicBool::new(false),
        )
        .unwrap();
        let back = crate::audio::decode_file(&staged.file.path().to_string_lossy()).unwrap();
        let lead = (staged.shift * 44100.0).round() as usize;
        assert!(lead <= 6000 && back.frames().abs_diff(44100 + lead) < 2304);
        // The source's first 1500 frames are quiet in the mix but audible in the vocal.
        // Checking the level change on the shifted timeline catches dropping the encoder lead.
        let rms = |a: usize, b: usize| {
            (back.samples[2 * (a + lead)..2 * (b + lead)]
                .iter()
                .map(|v| v * v)
                .sum::<f32>()
                / (2 * (b - a)) as f32)
                .sqrt()
        };
        assert!(rms(400, 1000) > 0.4, "the quiet-mix intro must retain the vocal");
        assert!(rms(1550, 1750) < 0.20);
    }

    #[test]
    fn mp3_round_trip_keeps_the_quiet_intro_and_uses_decoder_clock() {
        const TEST: &str = "stem_export::tests::mp3_round_trip_keeps_the_quiet_intro_and_uses_decoder_clock";
        let alternate = std::env::var_os("RECORDBOX_TEST_STEM_EXPORT_MP3_CLOCK").is_some();
        if alternate {
            crate::audio::Mp3Timeline::Rekordbox.initialize();
        }
        check_lossy_timeline(5);
        if !alternate {
            let profile = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env("PREPDECK_DATA", profile.path())
                .env("RECORDBOX_TEST_STEM_EXPORT_MP3_CLOCK", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_aac_keeps_the_quiet_intro_and_uses_decoder_clock() {
        check_lossy_timeline(4);
    }

    #[test]
    fn offdeck_export_reads_the_model_cache_and_rejects_incomplete_parts() {
        if std::env::var_os("RECORDBOX_TEST_STEM_EXPORT_CACHE").is_none() {
            let d = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "stem_export::tests::offdeck_export_reads_the_model_cache_and_rejects_incomplete_parts",
                ])
                .env("PREPDECK_DATA", d.path())
                .env("RECORDBOX_TEST_STEM_EXPORT_CACHE", "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let (path, parts) = source(d.path(), 16387, 2, 16);
        let mut w = work(&path, parts.clone(), d.path(), 1, Part::Vocals);
        crate::stems::save(&w.cache, &parts).unwrap();
        let cache = w.cache.clone();
        w.stems = None;
        let staged = audio_export::stage(w, &AtomicBool::new(false)).unwrap();
        let back = crate::audio::decode_file(&staged.file.path().to_string_lossy()).unwrap();
        assert_eq!(back.frames(), parts.len);
        let saved = crate::stems::load(&cache).unwrap();
        for (i, pair) in back.samples.chunks_exact(2).enumerate() {
            for c in 0..2 {
                let expected = (saved.data[(6 + c) * parts.len + i] * 32768.0)
                    .round()
                    .clamp(-32768.0, 32767.0);
                assert_eq!(pair[c] * 32768.0, expected);
            }
        }
        drop(staged);
        let mut w = work(&path, parts, d.path(), 1, Part::Vocals);
        w.stems = None;
        let file = std::fs::OpenOptions::new().write(true).open(cache).unwrap();
        file.set_len(16).unwrap();
        assert!(audio_export::stage(w, &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_publication_avoids_a_competing_file_and_a_reserved_library_path() {
        let d = tempfile::tempdir().unwrap();
        let mut f = tempfile::NamedTempFile::new_in(d.path()).unwrap();
        f.write_all(b"stem").unwrap();
        let existing = d.path().join("Part.wav");
        std::fs::write(&existing, b"other").unwrap();
        let dest = publish(f, d.path(), "Part", "wav", |p| p.ends_with("Part 2.wav")).unwrap();
        assert!(dest.ends_with("Part 3.wav"));
        assert_eq!(std::fs::read(existing).unwrap(), b"other");
        assert_eq!(std::fs::read(dest).unwrap(), b"stem");
    }

    #[test]
    fn stopping_leaves_another_process_partial_and_original_alone() {
        let d = tempfile::tempdir().unwrap();
        let (path, parts) = source(d.path(), 1000, 2, 16);
        let other = d.path().join(".recordbox-stem-1-2.partial.wav");
        std::fs::write(&other, "still writing").unwrap();
        assert!(audio_export::stage(work(&path, parts, d.path(), 1, Part::Vocals), &AtomicBool::new(true)).is_err());
        assert_eq!(std::fs::read_to_string(other).unwrap(), "still writing");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 2);
    }

    #[test]
    fn offdeck_queue_keeps_cues_key_and_grid_through_waveform_analysis() -> anyhow::Result<()> {
        const TEST: &str = "stem_export::tests::offdeck_queue_keeps_cues_key_and_grid_through_waveform_analysis";
        if std::env::var_os("RECORDBOX_TEST_STEM_EXPORT_QUEUE").is_none() {
            let profile = tempfile::tempdir()?;
            let output = std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", TEST, "--nocapture"])
                .env("PREPDECK_DATA", profile.path())
                .env("RECORDBOX_TEST_STEM_EXPORT_QUEUE", "1")
                .output()?;
            anyhow::ensure!(
                output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Ok(());
        }
        let d = tempfile::tempdir()?;
        let (path, parts) = source(d.path(), 44100 * 3, 2, 16);
        let mut app = App::open_library(
            &Default::default(),
            None,
            crate::DeckOptions {
                disable_play: true,
                disable_load: false,
            },
        );
        app.prefs.stemx = StemExportPrefs {
            format: 1,
            dir: d.path().to_string_lossy().into_owned(),
            explain: false,
            ..Default::default()
        };
        let m = &crate::stems::CATALOG[1];
        app.stem.model = Some(m.id.into());
        crate::stems::save(&crate::stems::cache_path(&path, m.id), &parts)?;
        let mut src = crate::importer::read_track(Path::new(&path));
        src.title = "Source".into();
        src.key = 11;
        src.rating = 4;
        src.bpm = 137.5;
        let id = app.db.insert_track(&src)?;
        let grid = BeatGrid::constant(0.137, 137.5, 1);
        app.db.save_grid(id, &grid)?;
        app.db.save_cue(&Cue {
            id: 0,
            track_id: id,
            kind: CueKind::Hot,
            slot: 4,
            start: 0.75,
            end: Some(1.75),
            color: 3,
            comment: "my loop".into(),
            active_loop: false,
        })?;
        app.reload_tracks();
        app.prefs.rb.library_protection = true;
        request(&mut app, &[id], Part::Vocals, false);
        assert!(!busy(&app));
        app.prefs.rb.library_protection = false;
        request(&mut app, &[id], Part::Vocals, false);
        assert!(busy(&app));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while busy(&app) || app.analysis_total != 0 {
            poll(&mut app);
            app.poll_analysis();
            anyhow::ensure!(
                std::time::Instant::now() < until,
                "stem export or waveform worker did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.deck.track.is_none() && app.stem.separating.is_none());
        let added = app
            .db
            .track_by_path(&d.path().join("Source (Vocals).wav").to_string_lossy())
            .expect("exported track");
        let track = app.db.track(added).unwrap();
        assert_eq!((track.key, track.rating, track.bpm, track.analyzed), (11, 4, 137.5, 1));
        assert!(app.db.has_wave(added)?);
        assert_eq!(app.db.grid(added).unwrap(), grid);
        let cues = app.db.cues(added)?;
        assert_eq!(cues.len(), 1);
        assert_eq!(
            (cues[0].slot, cues[0].start, cues[0].end, cues[0].comment.as_str()),
            (4, 0.75, Some(1.75), "my loop")
        );
        Ok(())
    }

    #[test]
    fn failed_playlist_write_rolls_back_track_cues_grid_and_source_link() -> anyhow::Result<()> {
        let d = tempfile::tempdir()?;
        let (original, _) = source(d.path(), 8000, 2, 16);
        let db = crate::db::Db::open(&d.path().join("library.db"))?;
        let mut src = crate::importer::read_track(Path::new(&original));
        src.id = db.insert_track(&src)?;
        db.save_grid(src.id, &BeatGrid::constant(0.2, 128.0, 1))?;
        db.save_cue(&Cue {
            id: 0,
            track_id: src.id,
            kind: CueKind::Hot,
            slot: 0,
            start: 0.4,
            end: Some(0.9),
            color: 2,
            comment: "first".into(),
            active_loop: false,
        })?;
        let pl = db.create_node(0, "Show", NodeKind::Playlist, None)?;
        db.set_playlist_tracks(pl, &[src.id])?;
        let exported = d.path().join("Vocal.wav");
        std::fs::copy(&original, &exported)?;
        db.conn.execute_batch(
            "CREATE TRIGGER reject_link BEFORE INSERT ON playlist_tracks BEGIN SELECT RAISE(ABORT,'busy'); END",
        )?;
        assert!(add_track(&db, &src, &exported, Part::Vocals, 0.05, None, None, Some(pl)).is_err());
        assert!(db.track_by_path(&exported.to_string_lossy()).is_none());
        assert_eq!(db.playlist_tracks(pl)?, vec![src.id]);
        assert_eq!(db.cues(src.id)?.len(), 1);
        assert!(exported.is_file());
        db.conn.execute_batch("DROP TRIGGER reject_link")?;
        let id = add_track(&db, &src, &exported, Part::Vocals, 0.05, None, None, Some(pl))?;
        assert_eq!(db.cues(id)?[0].start, 0.4 + 0.05);
        assert_eq!(db.grid(id).unwrap().anchors[0].time, 0.2 + 0.05);
        assert_eq!(db.setting(&link_key(id)), Some(format!("{}:vocals", src.id)));
        assert_eq!(db.playlist_tracks(pl)?, vec![src.id, id]);
        Ok(())
    }
}
