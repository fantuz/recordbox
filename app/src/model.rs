//! Core data types shared by the library, analysis, player and export.

use serde::{Deserialize, Serialize};

/// Camelot-ordered key index: 0..11 = 1A..12A (minor), 12..23 = 1B..12B (major).
pub const KEY_CLASSIC: [&str; 24] = [
    "Abm", "Ebm", "Bbm", "Fm", "Cm", "Gm", "Dm", "Am", "Em", "Bm", "Gbm", "Dbm", //
    "B", "Gb", "Db", "Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E",
];

pub fn key_alnum(k: i32) -> String {
    if !(0..24).contains(&k) {
        return String::new();
    }
    let n = k % 12 + 1;
    format!("{}{}", n, if k < 12 { "A" } else { "B" })
}

/// The order of a key as it is shown in the list. The Key column sorts by the label on
/// screen, which for an imported track is the spelling its library gave it ("1A", "Em"),
/// whatever notation is chosen here: codes of the number-and-letter kind go by number and
/// then letter (2A before 10A), other spellings after them by text, no key first.
pub fn key_sort_key(label: &str) -> (u8, u32, String) {
    let t = label.trim();
    if t.is_empty() {
        return (0, 0, String::new());
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    let rest = &t[digits..];
    if digits > 0 && digits <= 2 && rest.chars().count() == 1 && rest.chars().all(|c| c.is_ascii_alphabetic()) {
        return (1, t[..digits].parse().unwrap_or(0), rest.to_ascii_uppercase());
    }
    (2, 0, t.to_lowercase())
}

pub fn key_name(k: i32, alnum: bool) -> String {
    if !(0..24).contains(&k) {
        return String::new();
    }
    if alnum {
        key_alnum(k)
    } else {
        KEY_CLASSIC[k as usize].to_string()
    }
}

/// Parse a key written either way ("Am", "A#m", "8A", "08A", "C major", "Ebm").
pub fn parse_key(s: &str) -> i32 {
    let t = s.trim();
    if t.is_empty() {
        return -1;
    }
    let up = t.to_ascii_uppercase();
    // Alphanumeric / Camelot.
    let digits: String = up.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let rest = &up[digits.len()..];
        if let Ok(n) = digits.parse::<i32>() {
            if (1..=12).contains(&n) && (rest == "A" || rest == "B" || rest == "M" || rest == "D") {
                let minor = rest == "A" || rest == "M";
                return if minor { n - 1 } else { 12 + n - 1 };
            }
        }
        return -1;
    }
    // Classic: note name to pitch class.
    let mut chars = t.chars();
    let letter = chars.next().unwrap().to_ascii_uppercase();
    let base = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return -1,
    };
    let rest: String = chars.collect();
    let mut pc = base;
    let mut r = rest.as_str();
    if r.starts_with('#') || r.starts_with('♯') {
        pc += 1;
        r = &r[r.chars().next().unwrap().len_utf8()..];
    } else if r.starts_with('b') || r.starts_with('♭') {
        pc += 11;
        r = &r[r.chars().next().unwrap().len_utf8()..];
    }
    let rl = r.trim().to_ascii_lowercase();
    let minor = rl.starts_with('m') && !rl.starts_with("maj");
    key_from_pitch(pc % 12, minor)
}

/// Pitch class (C=0) + mode to Camelot-ordered index.
pub fn key_from_pitch(pc: i32, minor: bool) -> i32 {
    // Camelot number for majors: C=8B, G=9B ... each fifth up adds 1.
    // pitch*7 mod 12 gives circle-of-fifths position from C.
    let fifths = (pc * 7).rem_euclid(12);
    if minor {
        // Am = 8A; A is pitch 9 → fifths 3. 8A index 7.
        ((fifths - 3 + 7).rem_euclid(12)) as i32
    } else {
        // C = 8B index 19.
        12 + ((fifths + 7).rem_euclid(12)) as i32
    }
}

/// Inverse of key_from_pitch.
pub fn key_to_pitch(k: i32) -> (i32, bool) {
    if k < 12 {
        let fifths = (k - 7 + 3).rem_euclid(12);
        ((fifths * 7).rem_euclid(12), true)
    } else {
        let fifths = (k - 12 - 7).rem_euclid(12);
        ((fifths * 7).rem_euclid(12), false)
    }
}

/// Track colour labels (index 1..8; 0 = none).
pub const COLOR_NAMES: [&str; 9] = ["No Color", "Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple"];
pub const COLOR_RGB: [[u8; 3]; 9] = [
    [0x50, 0x50, 0x50],
    [0xF8, 0x70, 0xF8],
    [0xF8, 0x00, 0x00],
    [0xF8, 0xA0, 0x30],
    [0xF8, 0xE3, 0x30],
    [0x00, 0xE0, 0x00],
    [0x00, 0xC0, 0xF8],
    [0x00, 0x50, 0xF8],
    [0x98, 0x08, 0xF8],
];

/// Hot cue palette (index 1..62) as read from the original's hot cue swatch image.
pub const HOTCUE_PALETTE: [u32; 64] = [
    0x305AFF, 0x5073FF, 0x508CFF, 0x50A0FF, 0x50B4FF, 0x50B0F2, 0x50AEE8, 0x45ACDB, //
    0x00E0FF, 0x19DAF0, 0x32D2E6, 0x21B4B9, 0x20AAA0, 0x1FA392, 0x19A08C, 0x14A584, //
    0x14AA7D, 0x10B176, 0x30D26E, 0x37DE5A, 0x3CEB50, 0x28E214, 0x7DC13D, 0x8CC832, //
    0x9BD723, 0xA5E116, 0xA5DC0A, 0xAAD208, 0xB4C805, 0xB4BE04, 0xBAB404, 0xC3AF04, //
    0xE1AA00, 0xFFA000, 0xFF9600, 0xFF8C00, 0xFF7500, 0xE0641B, 0xE0461E, 0xE0301E, //
    0xE02823, 0xE62828, 0xFF376F, 0xFF2D6F, 0xFF127B, 0xF51E8C, 0xEB2DA0, 0xE637B4, //
    0xDE44CF, 0xDE448D, 0xE630B4, 0xE619DC, 0xE600FF, 0xDC00FF, 0xCC00FF, 0xB432FF, //
    0xB93CFF, 0xC542FF, 0xAA5AFF, 0xAA72FF, 0x8272FF, 0x6473FF, 0x000000, 0xFFFFFF,
];
/// The 16 colours offered in the hot cue colour picker (palette indices).
pub const HOTCUE_PICKER: [u8; 16] = [1, 5, 9, 14, 18, 22, 26, 30, 32, 38, 42, 45, 48, 56, 60, 62];
/// Default colour for a new hot cue A (style 4, the original's default setting).
pub const HOTCUE_DEFAULT: u8 = 43;
/// Fallback for loops without a valid colour (index 36, RGB 255,140,0 on the desktop).
pub const HOTCUE_LOOP_DEFAULT: u8 = 36;
/// Per-pad default colours for hot cues 1–8 (repeated for 9–16) in the original's default
/// colour style 4 (rekordbox-rea RF-05, CAnalyzerIF::getHotCueDefaultColor).
pub const HOTCUE_STYLE4: [u8; 8] = [43, 8, 23, 60, 19, 38, 1, 32];

/// The original's default colour for hot cue `slot` (0-based) when it has no valid colour.
pub fn hotcue_default_color(slot: i32, is_loop: bool) -> u8 {
    if is_loop {
        return HOTCUE_LOOP_DEFAULT;
    }
    if (0..16).contains(&slot) {
        HOTCUE_STYLE4[slot as usize % 8]
    } else {
        22
    }
}

/// Device (CDJ/XDJ) palette used for hot cue RGB in exported analysis files; it differs from
/// the desktop palette above (rekordbox-rea RF-05, device slot +0x170).
pub const DEVICE_PALETTE: [u32; 64] = [0x0000FF, 0x001CFF, 0x0038FF, 0x0054FF, 0x0070FF, 0x008CFF, 0x00A8FF, 0x00C4FF, 0x00E0FF, 0x00FFFF, 0x00FFE8, 0x00FFD1, 0x00FFBA, 0x00FFA3, 0x00FF8C, 0x00FF75, 0x00FF5E, 0x00FF47, 0x00FF30, 0x00FF1A, 0x00FF00, 0x1AFF00, 0x33FF00, 0x4DFF00, 0x66FF00, 0x80FF00, 0x99FF00, 0xB3FF00, 0xCCFF00, 0xE6FF00, 0xFFFF00, 0xFFE800, 0xFFD100, 0xFFBA00, 0xFFA300, 0xFF8C00, 0xFF7500, 0xFF5E00, 0xFF4700, 0xFF3000, 0xFF1A00, 0xFF0000, 0xFF0017, 0xFF002E, 0xFF0045, 0xFF005C, 0xFF0073, 0xFF008A, 0xFF00A1, 0xFF00B8, 0xFF00CF, 0xFF00E6, 0xFF00FF, 0xE600FF, 0xCC00FF, 0xB300FF, 0x9900FF, 0x8000FF, 0x6600FF, 0x4D00FF, 0x3300FF, 0x1A00FF, 0x000000, 0xFFFFFF];

pub fn device_rgb(idx: u8) -> [u8; 3] {
    let v = if (1..=64).contains(&idx) { DEVICE_PALETTE[idx as usize - 1] } else { DEVICE_PALETTE[21] };
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

pub fn hotcue_rgb(idx: u8) -> [u8; 3] {
    if idx == 0 || idx as usize > HOTCUE_PALETTE.len() {
        return [0x28, 0xE2, 0x14];
    }
    let v = HOTCUE_PALETTE[idx as usize - 1];
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub label: String,
    pub remixer: String,
    pub original_artist: String,
    pub composer: String,
    pub lyricist: String,
    pub mix_name: String,
    pub comment: String,
    pub grouping: String,
    pub message: String,
    pub year: i32,
    pub track_no: i32,
    pub disc_no: i32,
    pub release_date: String,
    pub duration: f64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub bit_depth: u32,
    pub file_size: u64,
    pub file_type: String,
    pub bpm: f64,
    pub key: i32,
    pub rating: u8,
    pub color: u8,
    pub date_added: String,
    pub date_created: String,
    pub play_count: u32,
    /// 0 = not analysed, 1 = analysed, 2 = failed, 3 = queued/running
    pub analyzed: u8,
    pub grid_locked: bool,
    pub autoload_hotcue: bool,
    /// Auto gain in dB; NaN when loudness could not be measured (silence, too short), which
    /// plays without auto gain (stored as NULL).
    #[serde(default, deserialize_with = "f32_or_nan")]
    pub gain_db: f32,
    pub artwork: String,
    pub missing: bool,
    #[serde(default)]
    pub raw_samples: u64,
    /// The user chose this key: analysis never replaces it (Analyze Key does, when asked).
    #[serde(default)]
    pub key_manual: bool,
    /// The key as the imported library or file spelled it ("7A", "Fm"), shown as is, as the
    /// original does, until analysis or the user sets the key.
    #[serde(default)]
    pub key_text: String,
}

impl Track {
    /// The key as shown: the imported spelling when there is one, else in the chosen notation.
    pub fn key_label(&self, alnum: bool) -> String {
        // (View > Display Type > "Display key information on the database": off, every key
        // is shown in the chosen notation.)
        if crate::prefs_bridge::SHOW_ORIGINAL_KEY.load(std::sync::atomic::Ordering::Relaxed) {
            self.stored_key_label(alnum)
        } else {
            key_name(self.key, alnum)
        }
    }

    /// Key spelling for interchange, independent of the current display preference.
    pub fn stored_key_label(&self, alnum: bool) -> String {
        if self.key_text.trim().is_empty() {
            key_name(self.key, alnum)
        } else {
            self.key_text.clone()
        }
    }
    /// Set the key from analysis or the user (drops the imported spelling).
    pub fn set_key(&mut self, k: i32) {
        self.key = k;
        self.key_text.clear();
    }

    fn recorded_file_name(&self) -> Option<&str> {
        // A library can retain Windows paths even when opened on macOS or Linux.
        let tail = if matches!(self.path.as_bytes(), [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic()) {
            &self.path[3..]
        } else if let Some(unc) = self.path.strip_prefix(r"\\") {
            unc.splitn(3, ['\\', '/']).nth(2)?
        } else {
            return std::path::Path::new(&self.path).file_name()?.to_str();
        };
        tail.rsplit(['\\', '/'])
            .next()
            .filter(|name| !matches!(*name, "" | "." | ".."))
    }

    pub fn file_name(&self) -> String {
        self.recorded_file_name().unwrap_or_default().to_owned()
    }
    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() {
            self.recorded_file_name()
                .and_then(|name| std::path::Path::new(name).file_stem()?.to_str())
                .unwrap_or_default()
                .to_owned()
        } else {
            self.title.clone()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CueKind {
    Memory,
    Hot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cue {
    pub id: i64,
    pub track_id: i64,
    pub kind: CueKind,
    /// Hot cue slot 0..7 (A..H); -1 for memory cues.
    pub slot: i32,
    pub start: f64,
    pub end: Option<f64>,
    /// Memory cues: track colour index 0..8. Hot cues: palette index 1..62 (0 = none).
    pub color: u8,
    pub comment: String,
    pub active_loop: bool,
}

impl Cue {
    pub fn is_loop(&self) -> bool {
        self.end.map(|e| e > self.start).unwrap_or(false)
    }
}

/// A beat grid made of tempo segments. Each anchor marks a beat at `time` with tempo `bpm`;
/// beats continue at that tempo until the next anchor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BeatGrid {
    pub anchors: Vec<GridAnchor>,
    /// When present, the anchors are the finite, measured beats themselves (including
    /// the last beat), not tempo segments. A grid edit invalidates the signature.
    #[serde(default, skip_serializing_if = "no_beat_list")]
    beat_list_signature: u64,
}

fn no_beat_list(signature: &u64) -> bool {
    *signature == 0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridAnchor {
    pub time: f64,
    pub bpm: f64,
    /// Beat number within the bar (1..4) of the beat at `time`.
    pub beat: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Beat {
    pub time: f64,
    pub bpm: f64,
    pub beat: u8,
    pub index: usize,
}

impl BeatGrid {
    pub(crate) fn from_anchors(anchors: Vec<GridAnchor>) -> Self {
        Self { anchors, beat_list_signature: 0 }
    }

    pub(crate) fn from_listed(anchors: Vec<GridAnchor>) -> Self {
        let mut grid = Self::from_anchors(anchors);
        if !grid.anchors.is_empty() {
            grid.beat_list_signature = grid.list_signature();
        }
        grid
    }

    fn list_signature(&self) -> u64 {
        // Compare at the precision of the analysis files. JSON parsers may round the
        // last bit of a floating-point value even on an otherwise untouched round trip.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for a in &self.anchors {
            let time_us = (a.time * 1e6).round() as i64 as u64;
            let tempo = (a.bpm * 1e4).round() as u64;
            for word in [time_us, tempo, u64::from(a.beat)] {
                hash = (hash ^ word).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash.max(1)
    }

    pub(crate) fn has_listed_beats(&self) -> bool {
        self.beat_list_signature != 0 && self.beat_list_signature == self.list_signature()
    }

    pub(crate) fn shifted(&self, seconds: f64) -> Self {
        let shifted = self.anchors.iter().map(|a| GridAnchor { time: a.time + seconds, ..*a }).collect();
        if self.has_listed_beats() { Self::from_listed(shifted) } else { Self::from_anchors(shifted) }
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }
    pub fn constant(first_beat: f64, bpm: f64, beat: u8) -> BeatGrid {
        BeatGrid::from_anchors(vec![GridAnchor { time: first_beat, bpm, beat }])
    }
    pub fn first_bpm(&self) -> f64 {
        self.anchors.first().map(|a| a.bpm).unwrap_or(0.0)
    }
    /// Every beat from the start of the track to `duration`.
    pub fn beats(&self, duration: f64) -> Vec<Beat> {
        if self.has_listed_beats() {
            return self.anchors.iter().take_while(|a| a.time <= duration).filter(|a| a.time >= 0.0)
                .enumerate().map(|(index, a)| Beat { time: a.time, bpm: a.bpm, beat: a.beat, index }).collect();
        }
        let mut out = vec![];
        if self.anchors.is_empty() {
            return out;
        }
        let a0 = self.anchors[0];
        if a0.bpm <= 0.0 {
            return out;
        }
        // Extrapolate backwards from the first anchor to time 0.
        let period = 60.0 / a0.bpm;
        let back = (a0.time / period).floor() as i64;
        let mut beat_no = a0.beat as i64;
        let mut t = a0.time - back as f64 * period;
        beat_no = (beat_no - 1 - back).rem_euclid(4) + 1;
        let mut idx = 0usize;
        for (ai, a) in self.anchors.iter().enumerate() {
            let end = self.anchors.get(ai + 1).map(|n| n.time).unwrap_or(duration + 1e-9);
            if ai > 0 {
                t = a.time;
                beat_no = a.beat as i64;
            }
            let p = 60.0 / a.bpm.max(1.0);
            // A beat closer than half a period to the next anchor belongs to that anchor.
            let stop = if ai + 1 < self.anchors.len() { end - p * 0.5 } else { end - 1e-6 };
            while t < stop && t <= duration {
                if t >= -1e-6 {
                    out.push(Beat { time: t.max(0.0), bpm: a.bpm, beat: beat_no as u8, index: idx });
                    idx += 1;
                }
                t += p;
                beat_no = beat_no % 4 + 1;
            }
        }
        out
    }
    /// Snap a time to the nearest beat (for quantize).
    pub fn snap(&self, t: f64, duration: f64) -> f64 {
        let beats = self.beats(duration);
        nearest_beat(&beats, t).map(|b| b.time).unwrap_or(t)
    }
    /// The grid at double tempo, following rekordbox's traced `bpmToDouble`: every beat stays
    /// and one is added halfway to the next; beat numbers map 1→1, 2→3, 3→1, 4→3. Where the
    /// interval across a tempo change is not a whole beat, the added beat sits at its midpoint.
    pub fn doubled(&self, duration: f64) -> BeatGrid {
        let beats = self.beats(duration);
        let map = |n: u8| -> u8 {
            match n {
                2 | 4 => 3,
                _ => 1,
            }
        };
        if self.has_listed_beats() {
            let mut out = Vec::with_capacity(beats.len().saturating_mul(2));
            for (i, b) in beats.iter().enumerate() {
                let bpm = b.bpm * 2.0;
                let beat = map(b.beat);
                out.push(GridAnchor { time: b.time, bpm, beat });
                if let Some(next) = beats.get(i + 1) {
                    out.push(GridAnchor { time: (b.time + next.time) * 0.5, bpm, beat: beat % 4 + 1 });
                }
            }
            return Self::from_listed(out);
        }
        let mut out = vec![];
        for (i, a) in self.anchors.iter().enumerate() {
            out.push(GridAnchor { time: a.time, bpm: a.bpm * 2.0, beat: map(a.beat) });
            let Some(n) = self.anchors.get(i + 1) else { continue };
            let lo = if i == 0 { f64::MIN } else { a.time - 1e-9 };
            let Some(last) = beats.iter().rev().find(|b| b.time < n.time - 1e-9 && b.time >= lo) else { continue };
            let gap = n.time - last.time;
            if (gap - 60.0 / a.bpm.max(1.0)).abs() > 1e-6 {
                out.push(GridAnchor { time: last.time + gap / 2.0, bpm: a.bpm * 2.0, beat: map(last.beat) % 4 + 1 });
            }
        }
        BeatGrid::from_anchors(out)
    }
    /// The grid at half tempo, following rekordbox's traced `bpmToHalf`: beats 0, 2, 4… counted
    /// from the first beat of the track stay where they are, and the first beat's number picks
    /// the new start of the 1–4 cycle (1→1, 2→4, 3→4, 4→1).
    pub fn halved(&self, duration: f64) -> BeatGrid {
        let beats = self.beats(duration);
        let Some(first) = beats.first() else { return self.clone() };
        let start: usize = match first.beat {
            2 | 3 => 4,
            _ => 1,
        };
        if self.has_listed_beats() {
            return Self::from_listed(beats.iter().step_by(2).enumerate().map(|(i, b)| GridAnchor {
                time: b.time, bpm: b.bpm * 0.5, beat: ((start - 1 + i) % 4 + 1) as u8,
            }).collect());
        }
        let mut out = vec![];
        for (i, a) in self.anchors.iter().enumerate() {
            let lo = if i == 0 { f64::MIN } else { a.time - 1e-9 };
            let hi = self.anchors.get(i + 1).map(|n| n.time - 1e-9).unwrap_or(f64::MAX);
            if let Some(b) = beats.iter().find(|b| b.index % 2 == 0 && b.time >= lo && b.time < hi) {
                out.push(GridAnchor { time: b.time, bpm: a.bpm / 2.0, beat: ((start - 1 + b.index / 2) % 4 + 1) as u8 });
            }
        }
        BeatGrid::from_anchors(out)
    }
}

pub fn nearest_beat(beats: &[Beat], t: f64) -> Option<Beat> {
    if beats.is_empty() {
        return None;
    }
    let i = beats.partition_point(|b| b.time < t);
    let mut best = None;
    for j in [i.saturating_sub(1), i.min(beats.len() - 1)] {
        let b = beats[j];
        match best {
            None => best = Some(b),
            Some(bb) if (b.time - t).abs() < (bb.time - t).abs() => best = Some(b),
            _ => {}
        }
    }
    best
}

/// Waveform data per 1/150 s frame, linear levels of the full signal and three bands:
/// bytes 0..4 = peak × 255 (full, low, mid, high), bytes 4..8 = RMS × 510.
#[derive(Clone, Debug, Default)]
pub struct Waveform {
    pub frames: Vec<[u8; 8]>,
}
pub const WAVE_FPS: f64 = 150.0;

/// Display gains per band applied before the display curve (estimated for legibility).
const DISP_GAIN: [f32; 4] = [1.0, 1.0, 1.5, 2.6];

impl Waveform {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = vec![2u8];
        v.extend(self.frames.iter().flat_map(|f| f.iter().copied()));
        v
    }
    pub fn from_bytes(b: &[u8]) -> Waveform {
        if b.len() % 8 == 1 && b.first() == Some(&2) {
            return Waveform { frames: b[1..].chunks_exact(8).map(|c| [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]).collect() };
        }
        // Older format (display levels only): approximate the linear values.
        let frames = b
            .chunks_exact(4)
            .map(|c| {
                let mut f = [0u8; 8];
                for k in 0..4 {
                    let lin = ((c[k] as f32 / 255.0).powf(1.0 / 0.85) / DISP_GAIN[k]).min(1.0);
                    f[k] = (lin * 255.0) as u8;
                    f[4 + k] = (lin * 0.5 * 510.0).min(255.0) as u8;
                }
                f
            })
            .collect();
        Waveform { frames }
    }
    /// Downsample to `n` columns (max of peaks, mean of RMS per bucket).
    pub fn columns(&self, n: usize) -> Vec<[u8; 8]> {
        let len = self.frames.len();
        if len == 0 || n == 0 {
            return vec![[0; 8]; n];
        }
        (0..n)
            .map(|i| {
                let a = i * len / n;
                let b = ((i + 1) * len / n).max(a + 1).min(len);
                let mut m = [0u8; 8];
                let mut acc = [0u32; 4];
                for f in &self.frames[a..b] {
                    for k in 0..4 {
                        m[k] = m[k].max(f[k]);
                        acc[k] += f[4 + k] as u32;
                    }
                }
                for k in 0..4 {
                    m[4 + k] = (acc[k] / (b - a) as u32) as u8;
                }
                m
            })
            .collect()
    }
    /// Display levels (0..255) of a frame or column: full, low, mid, high.
    pub fn disp(v: &[u8; 8]) -> [u8; 4] {
        let mut o = [0u8; 4];
        for k in 0..4 {
            let x = (v[k] as f32 / 255.0 * DISP_GAIN[k]).min(1.0).powf(0.85);
            o[k] = (x * 255.0).round() as u8;
        }
        o
    }
    /// Display-level preview with `n` columns, 4 bytes each (stored for the track list).
    pub fn preview_bytes(&self, n: usize) -> Vec<u8> {
        self.columns(n).iter().flat_map(|c| Self::disp(c)).collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NodeKind {
    Playlist,
    Folder,
    Smart,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaylistNode {
    pub id: i64,
    pub parent: i64,
    pub name: String,
    pub kind: NodeKind,
    pub seq: i64,
    pub smart: Option<crate::smart::SmartRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MyTag {
    pub id: i64,
    /// 0 for a category row; otherwise the category id.
    pub parent: i64,
    pub name: String,
    pub seq: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_filenames_survive_a_change_of_computer() {
        for path in [
            r"C:\Users\old\Dropbox\contents_42\Café Mix.wav",
            "D:/DJ/contents_42/Café Mix.wav",
            r"\\nas\sets\Café Mix.wav",
            "/Volumes/DJ/contents_42/Café Mix.wav",
        ] {
            let track = Track { path: path.into(), ..Default::default() };
            assert_eq!(track.file_name(), "Café Mix.wav");
            assert_eq!(track.display_title(), "Café Mix");
            assert_eq!(track.path, path);
        }
        for root in [r"C:\", r"\\nas\sets"] {
            assert_eq!(Track { path: root.into(), ..Default::default() }.file_name(), "");
        }
        #[cfg(unix)]
        assert_eq!(Track { path: r"/music/Intro\Outro.wav".into(), ..Default::default() }.file_name(), r"Intro\Outro.wav");
    }

    fn times(g: &BeatGrid, d: f64) -> Vec<(i64, u8)> {
        g.beats(d).iter().map(|b| ((b.time * 1e6).round() as i64, b.beat)).collect()
    }
    #[test]
    fn loops_follow_the_grid_through_a_tempo_change() {
        // 120 BPM until 2.0 s, then 100 BPM.
        let g = BeatGrid::from_anchors(vec![GridAnchor { time: 0.0, bpm: 120.0, beat: 1 }, GridAnchor { time: 2.0, bpm: 100.0, beat: 1 }]);
        let b = g.beats(10.0);
        let end = |s: f64, n: f64| (crate::app::loop_end_on_grid(&b, s, n).unwrap() * 1e6).round() as i64;
        // Four beats from 1.0 s: two at 0.5 s and two at 0.6 s, not 4 x 0.5.
        assert_eq!(end(1.0, 4.0), 3_200_000);
        assert_eq!(end(0.0, 1.0), 500_000);
        // Half a beat uses the interval the loop starts in.
        assert_eq!(end(2.0, 0.5), 2_300_000);
        assert_eq!(end(1.5, 1.5), 2_300_000);
        assert!(crate::app::loop_end_on_grid(&[], 1.0, 4.0).is_none());
    }
    #[test]
    fn double_and_half_grid_follow_rekordbox() {
        // Constant grid: first beat at 0.1 s numbered 2.
        let g = BeatGrid::constant(0.1, 120.0, 2);
        let d = g.doubled(4.0);
        let db = times(&d, 4.0);
        assert_eq!(&db[..4], &[(100_000, 3), (350_000, 4), (600_000, 1), (850_000, 2)]);
        // Half: beats 0, 2, 4… stay; a first beat numbered 2 starts the cycle at 4.
        let h = g.halved(4.0);
        assert_eq!(&times(&h, 4.0)[..3], &[(100_000, 4), (1_100_000, 1), (2_100_000, 2)]);
        // Doubling then halving gives the original beat times back.
        let back: Vec<i64> = times(&d.halved(4.0), 4.0).iter().map(|x| x.0).collect();
        let orig: Vec<i64> = times(&g, 4.0).iter().map(|x| x.0).collect();
        assert_eq!(back, orig);
        // Two tempos with an uneven interval (0.7 s, not 0.5) across the change: every original
        // beat is kept and the added one sits at the midpoint of that interval.
        let g2 = BeatGrid::from_anchors(vec![GridAnchor { time: 0.0, bpm: 120.0, beat: 1 }, GridAnchor { time: 2.2, bpm: 100.0, beat: 1 }]);
        let d2: Vec<i64> = times(&g2.doubled(5.0), 5.0).iter().map(|x| x.0).collect();
        for t in times(&g2, 5.0) {
            assert!(d2.contains(&t.0), "beat at {} lost", t.0);
        }
        assert!(d2.contains(&1_850_000) && !d2.contains(&1_750_000));
        assert_eq!(d2.len(), times(&g2, 5.0).len() * 2);
        // Halving keeps both tempos instead of flattening the grid to one.
        let h2 = g2.halved(5.0);
        assert_eq!(h2.anchors.len(), 2);
        assert_eq!((h2.anchors[0].bpm, h2.anchors[1].bpm), (60.0, 50.0));
    }
    #[test]
    fn imported_key_spelling_is_shown_until_the_key_changes() {
        let mut t = Track { key: parse_key("7A"), key_text: "7A".into(), ..Default::default() };
        // Shown as imported whatever the notation preference (as rekordbox lists "7A" beside "Fm").
        assert_eq!(t.key_label(false), "7A");
        assert_eq!(t.key_label(true), "7A");
        t.set_key(parse_key("Fm"));
        assert_eq!((t.key_label(false), t.key_label(true)), ("Fm".to_string(), "4A".to_string()));
    }

    #[test]
    fn keys() {
        assert_eq!(parse_key("Am"), 7);
        assert_eq!(key_alnum(parse_key("Am")), "8A");
        assert_eq!(key_alnum(parse_key("C")), "8B");
        assert_eq!(key_alnum(parse_key("F#m")), "11A");
        assert_eq!(key_alnum(parse_key("Abm")), "1A");
        assert_eq!(key_alnum(parse_key("E")), "12B");
        assert_eq!(parse_key("8A"), 7);
        for k in 0..24 {
            let (pc, m) = key_to_pitch(k);
            assert_eq!(key_from_pitch(pc, m), k);
            assert_eq!(parse_key(KEY_CLASSIC[k as usize]), k);
        }
    }
    #[test]
    fn grid() {
        let g = BeatGrid::constant(0.25, 120.0, 1);
        let b = g.beats(10.0);
        assert_eq!(b[0].time, 0.25);
        assert_eq!(b[1].beat, 2);
        assert_eq!(b.len(), 20);
        let g2 = BeatGrid::constant(1.25, 120.0, 3);
        let b2 = g2.beats(4.0);
        assert!((b2[0].time - 0.25).abs() < 1e-9);
        assert_eq!(b2[0].beat, 1);
    }
}

/// An f32 that may have been written as null (serde_json writes NaN that way).
fn f32_or_nan<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    Ok(<Option<f32> as serde::Deserialize>::deserialize(d)?.unwrap_or(f32::NAN))
}

#[cfg(test)]
mod key_sort_tests {
    use super::*;

    #[test]
    fn keys_sort_as_they_are_shown() {
        let mut v = vec!["10A", "1A", "Em", "8A", "", "3A", "1B", "12A", "2A", "Abm", "9A", "01A"];
        v.sort_by_key(|k| key_sort_key(k));
        assert_eq!(v, ["", "1A", "01A", "1B", "2A", "3A", "8A", "9A", "10A", "12A", "Abm", "Em"]);
    }
}
