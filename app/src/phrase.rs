//! Phrase analysis (song structure) for tracks rekordbox has not analysed, and the PSSI section
//! the players read. Layout and the RB6+ XOR mask follow rekordcrate (src/anlz.rs); the
//! segmentation and labels are Recordbox's own, tuned against rekordbox's phrases for the
//! user's library (rekordbox's models are not used).

use crate::model::Beat;

/// High-mood phrase kinds (the mood rekordbox gives most dance music).
pub const INTRO: u16 = 1;
pub const UP: u16 = 2;
pub const DOWN: u16 = 3;
pub const CHORUS: u16 = 5;
pub const OUTRO: u16 = 6;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Phrase {
    /// 1-based beat number (index into the track's beats) where the phrase starts.
    pub beat: u16,
    pub kind: u16,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Structure {
    /// 1 High, 2 Mid, 3 Low.
    pub mood: u16,
    pub phrases: Vec<Phrase>,
    /// Beat number where the last phrase ends.
    pub end_beat: u16,
    // Older phrase edits contain only the three fields above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rb: Option<RbFields>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RbFields {
    version: u16,
    // All supported header and row bytes, in plain form. Only needed if the
    // canonical encoder cannot reproduce them (bank, auxiliary beats, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original: Option<Vec<u8>>,
}

impl Structure {
    fn edited(&mut self) {
        if let Some(fields) = &mut self.rb {
            fields.original = None;
            if fields.version == 0 { self.rb = None; }
        }
    }
}

pub(crate) use crate::phrase_rb::{MAX_BEATS, MAX_SECONDS as MAX_ANALYSIS_SECONDS};

/// The original analyser receives a pre-existing key, or an unknown key if key and
/// phrases are inferred together. Only the stored PSSI key flag depends on this.
pub fn key_input(track: &crate::model::Track) -> i32 {
    if track.key_manual || !track.key_text.trim().is_empty() { track.key } else { -1 }
}

/// Current-grid inference consumes the deck's stereo samples directly, including the
/// classic fallback. A cancelled native job must never start the fallback.
pub fn analyze_audio(d: &crate::audio::Decoded, beats: &[Beat], key: i32, classic: bool, cancelled: &dyn Fn() -> bool) -> Option<Structure> {
    if cancelled() || d.sr == 0 || d.frames() == 0 || d.frames() > MAX_ANALYSIS_SECONDS.saturating_mul(d.sr as usize) || !(8..=MAX_BEATS).contains(&beats.len()) {
        return None;
    }
    let duration = d.duration();
    if beats.iter().any(|b| !b.time.is_finite() || b.time < 0.0 || b.time > duration + 0.002 || !b.bpm.is_finite() || b.bpm <= 0.0 || !(1..=4).contains(&b.beat))
        || beats.windows(2).any(|pair| pair[1].time <= pair[0].time) {
        return None;
    }
    if !classic && std::env::var("PREPDECK_PHRASE").as_deref() != Ok("own") {
        let key = crate::phrase_rb::analyser_key(key);
        if let Some(result) = crate::phrase_rb::analyze_with_cancel(&d.samples, d.sr, beats, key, cancelled) {
            if cancelled() { return None; }
            if let Some(structure) = parse(&crate::phrase_rb::pssi_bytes(&result)) {
                return Some(structure);
            }
        }
    }
    let mono = |i: usize| 0.5 * (d.samples[2 * i] + d.samples[2 * i + 1]);
    analyze_classic(d.frames(), mono, d.sr as f64, beats, cancelled)
}

/// Per-bar log energies in three bands (low < 200 Hz, mid, high > 2.5 kHz).
fn bar_features(n: usize, mono: impl Fn(usize) -> f32, sr: f64, bar_times: &[f64], end_time: f64, cancelled: &dyn Fn() -> bool) -> Option<Vec<[f64; 3]>> {
    let mut lp = crate::analysis::Biquad::new("lp", 200.0, 0.707, sr, 0.0);
    let mut hp = crate::analysis::Biquad::new("hp", 2500.0, 0.707, sr, 0.0);
    let mut idx = 0usize;
    let mut out = Vec::with_capacity(bar_times.len());
    for (k, t) in bar_times.iter().enumerate() {
        let start = ((*t * sr) as usize).min(n);
        let end = ((bar_times.get(k + 1).copied().unwrap_or(end_time)) * sr) as usize;
        let end = end.min(n);
        if cancelled() { return None; }
        while idx < start {
            if idx % 16384 == 0 && cancelled() { return None; }
            let x = mono(idx) as f64;
            if !x.is_finite() { return None; }
            lp.run(x);
            hp.run(x);
            idx += 1;
        }
        let mut e = [0f64; 3];
        let mut c = 0f64;
        while idx < end {
            if idx % 16384 == 0 && cancelled() { return None; }
            let x = mono(idx) as f64;
            if !x.is_finite() { return None; }
            let l = lp.run(x);
            let h = hp.run(x);
            e[0] += l * l;
            e[1] += (x - l - h).powi(2);
            e[2] += h * h;
            c += 1.0;
            idx += 1;
        }
        let c = c.max(1.0);
        out.push([10.0 * (e[0] / c + 1e-10).log10(), 10.0 * (e[1] / c + 1e-10).log10(), 10.0 * (e[2] / c + 1e-10).log10()]);
    }
    Some(out)
}

/// Analyse the phrase structure of a track from its audio and beat grid.
#[cfg(test)]
pub fn analyze(mono: &[f32], sr: f64, beats: &[Beat]) -> Option<Structure> {
    analyze_classic(mono.len(), |i| mono[i], sr, beats, &|| false)
}

fn analyze_classic(n: usize, mono: impl Fn(usize) -> f32, sr: f64, beats: &[Beat], cancelled: &dyn Fn() -> bool) -> Option<Structure> {
    if cancelled() || beats.len() < 64 || beats.len() > u16::MAX as usize {
        return None;
    }
    let end_time = n as f64 / sr;
    // Bars start at beats numbered 1. Beats before the first bar start belong to phrase 1.
    let bar_idx: Vec<usize> = beats.iter().enumerate().filter(|(_, b)| b.beat == 1).map(|(i, _)| i).collect();
    if bar_idx.len() < 8 {
        return None;
    }
    let bar_times: Vec<f64> = bar_idx.iter().map(|i| beats[*i].time).collect();
    let f = bar_features(n, mono, sr, &bar_times, end_time, cancelled)?;
    let nb = f.len();
    // Novelty between consecutive 4-bar blocks at every bar.
    let mean = |a: usize, b: usize, band: usize| -> f64 { f[a..b].iter().map(|x| x[band]).sum::<f64>() / (b - a).max(1) as f64 };
    let nov = |i: usize| -> f64 {
        if i < 4 || i + 4 > nb {
            return 0.0;
        }
        (0..3).map(|b| (mean(i, i + 4, b) - mean(i - 4, i, b)).powi(2)).sum::<f64>().sqrt()
    };
    // Boundaries: every 8 bars from the first bar; a 4-bar boundary in between when the music
    // changes clearly there; a final short phrase is merged into the previous one.
    let threshold = params().0;
    let max_bars = max_bars();
    let mut cuts: Vec<usize> = vec![0];
    let mut last = 0usize;
    let mut i = 4;
    while i < nb {
        let since = i - last;
        if since >= max_bars || (since >= 4 && nov(i) > threshold) {
            cuts.push(i);
            last = i;
        }
        i += 4;
    }
    if nb - last < 2 && cuts.len() > 1 {
        cuts.pop();
    }
    // Label each phrase from its low-band (kick/bass) and overall energy.
    let seg: Vec<(usize, usize)> = cuts.iter().enumerate().map(|(k, a)| (*a, cuts.get(k + 1).copied().unwrap_or(nb))).collect();
    let low: Vec<f64> = seg.iter().map(|(a, b)| mean(*a, *b, 0)).collect();
    let tot: Vec<f64> = seg.iter().map(|(a, b)| (mean(*a, *b, 0) + mean(*a, *b, 1) + mean(*a, *b, 2)) / 3.0).collect();
    let low_max = low.iter().cloned().fold(f64::MIN, f64::max);
    let tot_max = tot.iter().cloned().fold(f64::MIN, f64::max);
    let (_, down_db, up_db) = params();
    let mut kinds: Vec<u16> = seg
        .iter()
        .enumerate()
        .map(|(k, _)| {
            if low[k] < low_max - down_db {
                DOWN
            } else if tot[k] < tot_max - up_db {
                UP
            } else {
                CHORUS
            }
        })
        .collect();
    // A quieter phrase right before a chorus builds up to it.
    for k in 0..kinds.len().saturating_sub(1) {
        if kinds[k] == DOWN && kinds[k + 1] == CHORUS && low[k] >= low_max - down_db * 1.5 {
            kinds[k] = UP;
        }
    }
    let n = kinds.len();
    kinds[0] = INTRO;
    if n > 2 {
        kinds[n - 1] = OUTRO;
    }
    let phrases = seg
        .iter()
        .zip(&kinds)
        .enumerate()
        .map(|(k, ((a, _), kind))| Phrase { beat: if k == 0 { 1 } else { (bar_idx[*a] + 1) as u16 }, kind: *kind })
        .collect();
    Some(Structure { mood: 1, phrases, end_beat: beats.len() as u16, rb: None })
}

fn max_bars() -> usize {
    std::env::var("PREPDECK_PHRASE_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(8)
}

fn params() -> (f64, f64, f64) {
    let g = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    // Tuned against rekordbox's phrases for 99 tracks: 74 % per-beat label agreement (all
    // "Chorus" scores 57 %), boundary F1 69.5 %.
    (g("PREPDECK_PHRASE_NOV", 8.0), g("PREPDECK_PHRASE_DOWN", 18.0), g("PREPDECK_PHRASE_UP", 2.5))
}

/// The phrase types PHRASE EDIT offers for a mood, as (kind, name): High has Intro / Up /
/// Down / Chorus / Outro; Mid and Low have Intro / Verse / Bridge / Chorus / Outro (six verses
/// in Mid; Low numbers its two verses 2 and 5, each with two variants after it).
pub fn kinds(mood: u16) -> &'static [(u16, &'static str)] {
    match mood {
        2 => &[(1, "Intro"), (2, "Verse 1"), (3, "Verse 2"), (4, "Verse 3"), (5, "Verse 4"), (6, "Verse 5"), (7, "Verse 6"), (8, "Bridge"), (9, "Chorus"), (10, "Outro")],
        3 => &[(1, "Intro"), (2, "Verse 1"), (5, "Verse 2"), (8, "Bridge"), (9, "Chorus"), (10, "Outro")],
        _ => &[(INTRO, "Intro"), (UP, "Up"), (DOWN, "Down"), (CHORUS, "Chorus"), (OUTRO, "Outro")],
    }
}

/// Name of a phrase kind in its mood.
pub fn label(mood: u16, kind: u16) -> &'static str {
    match (mood, kind) {
        (2 | 3, 1) => "Intro",
        (2, 2..=7) => ["Verse 1", "Verse 2", "Verse 3", "Verse 4", "Verse 5", "Verse 6"][kind as usize - 2],
        (3, 2..=4) => "Verse 1",
        (3, 5..=7) => "Verse 2",
        (2 | 3, 8) => "Bridge",
        (2 | 3, 9) => "Chorus",
        (2 | 3, 10) => "Outro",
        (2 | 3, _) => "Phrase",
        _ => kind_name(kind),
    }
}

/// Colour of a phrase block in the phrase bar (0xRRGGBB).
pub fn color(mood: u16, kind: u16) -> u32 {
    match label(mood, kind) {
        "Intro" => 0x3E7BD6,
        "Up" | "Bridge" => 0xD6A23E,
        "Down" => 0x5AA05A,
        "Chorus" => 0xC8464B,
        "Outro" => 0x8A62C8,
        l if l.starts_with("Verse") => 0x3CAAB4,
        _ => 0x606060,
    }
}

/// Index of the phrase that contains 1-based beat number `beat`.
pub fn index_at(s: &Structure, beat: u16) -> Option<usize> {
    if s.phrases.is_empty() || beat > s.end_beat.max(s.phrases.last().map(|p| p.beat).unwrap_or(0)) {
        return None;
    }
    let i = s.phrases.partition_point(|p| p.beat <= beat);
    Some(i.saturating_sub(1))
}

/// PHRASE EDIT > CUT: split the phrase containing `beat` so that a new phrase of the same
/// type starts there. False (nothing changed) when a phrase already starts on that beat or
/// the beat is outside the phrases.
pub fn cut(s: &mut Structure, beat: u16) -> bool {
    if beat < 1 || beat >= s.end_beat || s.phrases.iter().any(|p| p.beat == beat) {
        return false;
    }
    let Some(i) = index_at(s, beat) else { return false };
    if s.phrases[i].beat > beat {
        return false;
    }
    let kind = s.phrases[i].kind;
    s.phrases.insert(i + 1, Phrase { beat, kind });
    s.edited();
    true
}

/// PHRASE EDIT's type list: give the phrase containing `beat` another type. False when it
/// already has it or there is no phrase there.
pub fn set_kind(s: &mut Structure, beat: u16, kind: u16) -> bool {
    match index_at(s, beat) {
        Some(i) if s.phrases[i].kind != kind => {
            s.phrases[i].kind = kind;
            s.edited();
            true
        }
        _ => false,
    }
}

/// 1-based number of the beat at or nearest before `t` (1 before the first beat).
pub fn beat_number(beats: &[Beat], t: f64) -> u16 {
    beats.partition_point(|b| b.time <= t + 1e-4).clamp(1, u16::MAX as usize) as u16
}

const KEY: [u8; 19] = [0xCB, 0xE1, 0xEE, 0xFA, 0xE5, 0xEE, 0xAD, 0xEE, 0xE9, 0xD2, 0xE9, 0xEB, 0xE1, 0xE9, 0xF3, 0xE8, 0xE9, 0xF4, 0xE1];

/// Name of a phrase kind (high-mood labels).
pub fn kind_name(kind: u16) -> &'static str {
    match kind {
        INTRO => "Intro",
        UP => "Up",
        DOWN => "Down",
        CHORUS => "Chorus",
        OUTRO => "Outro",
        _ => "Phrase",
    }
}

/// Suggested memory cues: the start of each phrase after the first, named after the phrase
/// it starts (provenance: phrase analysis). Times come from the grid's beats.
pub fn cue_proposals(s: &Structure, beats: &[Beat]) -> Vec<(f64, &'static str)> {
    s.phrases
        .iter()
        .filter(|p| p.beat > 1)
        .filter_map(|p| beats.get(p.beat as usize - 1).map(|b| (b.time, label(s.mood, p.kind))))
        .collect()
}

/// Supported PSSI framing. Opaque header and entry fields are left uninterpreted.
fn pssi_layout(section: &[u8]) -> Option<(usize, usize)> {
    if section.len() < 32 || &section[..4] != b"PSSI" {
        return None;
    }
    let header = u32::from_be_bytes(section[4..8].try_into().ok()?) as usize;
    let total = u32::from_be_bytes(section[8..12].try_into().ok()?) as usize;
    let version = u16::from_be_bytes([section[12], section[13]]) as usize;
    let entry_size = u16::from_be_bytes([section[14], section[15]]) as usize;
    let n = u16::from_be_bytes([section[16], section[17]]) as usize;
    if header != 32 || total != section.len() || version > 1 || entry_size != 24 || header.checked_add(n.checked_mul(entry_size)?)? != total {
        return None;
    }
    Some((version, n))
}

/// XOR the PSSI bytes after the entry count. Version 1 rotates the same mask by one
/// byte; applying this twice restores the original. Unsupported sections stay untouched.
pub fn mask_section(section: &mut [u8]) {
    let Some((version, n)) = pssi_layout(section) else { return };
    for (i, b) in section[18..].iter_mut().enumerate() {
        *b ^= KEY[(i + version) % KEY.len()].wrapping_add(n as u8);
    }
}

/// The mood field of an unmasked section is 1..=3; a masked one decodes to that range.
pub fn is_masked(section: &[u8]) -> bool {
    if pssi_layout(section).is_none() {
        return false;
    }
    let mood = u16::from_be_bytes([section[18], section[19]]);
    !(1..=3).contains(&mood)
}

/// Can this plain or masked section be interpreted on the local grid?
/// Check framing and mood without allocating the row list during export planning.
pub fn supported_section(section: &[u8]) -> bool {
    let Some((version, n)) = pssi_layout(section) else { return false };
    let mood = u16::from_be_bytes([section[18], section[19]]);
    if (1..=3).contains(&mood) { return true; }
    let decoded = u16::from_be_bytes([
        section[18] ^ KEY[version].wrapping_add(n as u8),
        section[19] ^ KEY[version + 1].wrapping_add(n as u8),
    ]);
    (1..=3).contains(&decoded)
}

/// The PSSI section (unmasked) for a structure.
pub fn pssi(s: &Structure) -> Vec<u8> {
    let version = s.rb.as_ref().map_or(0, |rb| rb.version.min(1));
    if let Some(bytes) = s.rb.as_ref().and_then(|rb| rb.original.as_ref()) {
        // A saved edit may come from an older or manually edited JSON snapshot.
        // Replay only matching, supported bytes from the same semantic structure.
        if pssi_layout(bytes) == Some((version as usize, s.phrases.len()))
            && bytes[18..20] == s.mood.to_be_bytes() && bytes[26..28] == s.end_beat.to_be_bytes()
            && bytes[32..].chunks_exact(24).zip(&s.phrases).all(|(r, p)| r[2..4] == p.beat.to_be_bytes() && r[4..6] == p.kind.to_be_bytes()) {
            return bytes.clone();
        }
    }
    canonical_pssi(s, version)
}

fn canonical_pssi(s: &Structure, version: u16) -> Vec<u8> {
    let n = s.phrases.len().min(u16::MAX as usize) as u16;
    let mut body = vec![];
    body.extend_from_slice(&s.mood.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&s.end_beat.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.push(0); // bank: default
    body.push(0);
    for (i, p) in s.phrases.iter().take(n as usize).enumerate() {
        let mut e = [0u8; 24];
        e[0..2].copy_from_slice(&(i as u16 + 1).to_be_bytes());
        e[2..4].copy_from_slice(&p.beat.to_be_bytes());
        e[4..6].copy_from_slice(&p.kind.to_be_bytes());
        // k1 = 1 for intro, chorus and outro, as rekordbox writes them most often.
        e[7] = matches!(p.kind, INTRO | CHORUS | OUTRO) as u8;
        body.extend_from_slice(&e);
    }
    let mut v = vec![];
    v.extend_from_slice(b"PSSI");
    v.extend_from_slice(&32u32.to_be_bytes());
    v.extend_from_slice(&((18 + body.len()) as u32).to_be_bytes());
    v.extend_from_slice(&version.to_be_bytes());
    v.extend_from_slice(&24u16.to_be_bytes());
    v.extend_from_slice(&n.to_be_bytes());
    v.extend_from_slice(&body);
    v
}

/// Parse an (unmasked or masked) PSSI section.
pub fn parse(section: &[u8]) -> Option<Structure> {
    let (version, n) = pssi_layout(section)?;
    let version = version as u16;
    let mut s = section.to_vec();
    if is_masked(&s) {
        mask_section(&mut s);
    }
    let mood = u16::from_be_bytes([s[18], s[19]]);
    if !(1..=3).contains(&mood) {
        return None;
    }
    let end_beat = u16::from_be_bytes([s[26], s[27]]);
    let mut phrases = vec![];
    for k in 0..n {
        let e = s.get(32 + k * 24..32 + (k + 1) * 24)?;
        phrases.push(Phrase { beat: u16::from_be_bytes([e[2], e[3]]), kind: u16::from_be_bytes([e[4], e[5]]) });
    }
    let mut result = Structure { mood, phrases, end_beat, rb: None };
    let original = (s != canonical_pssi(&result, version)).then_some(s);
    if version != 0 || original.is_some() {
        result.rb = Some(RbFields { version, original });
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_classic_analysis_does_not_publish_a_partial_structure() {
        let beats: Vec<_> = (0..80).map(|i| Beat { time: i as f64 * 0.5, bpm: 120.0, beat: (i % 4) as u8 + 1, index: i }).collect();
        let calls = std::cell::Cell::new(0);
        let stopped = || { calls.set(calls.get() + 1); calls.get() >= 5 };
        assert!(analyze_classic(160_000, |_| 0.0, 4000.0, &beats, &stopped).is_none());
        assert_eq!(calls.get(), 5);
    }

    #[test]
    fn inferred_keys_and_imported_keys_give_the_reference_header_flags() {
        let mut track = crate::model::Track { key: 4, ..Default::default() };
        assert_eq!(key_input(&track), -1);
        track.key_text = "5A".into();
        assert_eq!(key_input(&track), 4);
        track.key_text.clear();
        track.key_manual = true;
        assert_eq!(key_input(&track), 4);
    }

    #[test]
    fn proposals_use_the_phrase_label_for_each_mood() {
        let beats: Vec<Beat> = (0..8).map(|i| Beat { index:i, time:i as f64 * 0.5, bpm:120.0, beat:1 + (i % 4) as u8 }).collect();
        let mut structure = Structure { mood:3, end_beat:8, phrases:vec![Phrase { beat:5, kind:5 }], rb:None };
        assert_eq!(cue_proposals(&structure, &beats), [(2.0, "Verse 2")]);
        structure.mood = 2;
        assert_eq!(cue_proposals(&structure, &beats), [(2.0, "Verse 4")]);
    }

    #[test]
    fn phrase_edits() {
        let mut s = Structure { mood: 1, phrases: vec![Phrase { beat: 1, kind: INTRO }, Phrase { beat: 33, kind: UP }, Phrase { beat: 65, kind: CHORUS }], end_beat: 129, rb: None };
        // CUT inside the Up phrase: a second Up phrase starts at the cut.
        assert!(cut(&mut s, 49));
        assert_eq!(s.phrases.iter().map(|p| (p.beat, p.kind)).collect::<Vec<_>>(), vec![(1, INTRO), (33, UP), (49, UP), (65, CHORUS)]);
        // On an existing boundary, before the start or at the end: nothing happens.
        assert!(!cut(&mut s, 49));
        assert!(!cut(&mut s, 33));
        assert!(!cut(&mut s, 0));
        assert!(!cut(&mut s, 129));
        assert!(!cut(&mut s, 400));
        assert_eq!(s.phrases.len(), 4);
        // The type list changes the phrase under the playhead only.
        assert!(set_kind(&mut s, 50, DOWN));
        assert!(!set_kind(&mut s, 64, DOWN));
        assert_eq!(s.phrases[2], Phrase { beat: 49, kind: DOWN });
        assert_eq!(s.phrases[1].kind, UP);
        assert_eq!(index_at(&s, 1), Some(0));
        assert_eq!(index_at(&s, 128), Some(3));
        // The edited structure is what the PSSI section carries.
        assert_eq!(parse(&pssi(&s)), Some(s.clone()));
        // It survives being stored as JSON (how edits are kept per track).
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Structure>(&j).unwrap(), s);
        // Type lists per mood.
        assert_eq!(kinds(1).iter().map(|k| k.1).collect::<Vec<_>>(), vec!["Intro", "Up", "Down", "Chorus", "Outro"]);
        assert_eq!(kinds(2).len(), 10);
        assert_eq!(kinds(3).iter().map(|k| k.1).collect::<Vec<_>>(), vec!["Intro", "Verse 1", "Verse 2", "Bridge", "Chorus", "Outro"]);
        assert_eq!(label(1, 5), "Chorus");
        assert_eq!(label(2, 5), "Verse 4");
        assert_eq!(label(3, 6), "Verse 2");
        assert_eq!(label(2, 8), "Bridge");
        // Beat numbers from times.
        let beats: Vec<Beat> = (0..16).map(|i| Beat { time: 1.0 + i as f64 * 0.5, bpm: 120.0, beat: (i % 4) as u8 + 1, index: i }).collect();
        assert_eq!(beat_number(&beats, 0.0), 1);
        assert_eq!(beat_number(&beats, 1.0), 1);
        assert_eq!(beat_number(&beats, 1.6), 2);
        assert_eq!(beat_number(&beats, 99.0), 16);
    }

    #[test]
    fn pssi_roundtrip_and_mask() {
        let s = Structure { mood: 1, phrases: vec![Phrase { beat: 1, kind: INTRO }, Phrase { beat: 33, kind: UP }, Phrase { beat: 65, kind: CHORUS }, Phrase { beat: 129, kind: OUTRO }], end_beat: 160, rb: None };
        let mut b = pssi(&s);
        assert_eq!(b.len(), 32 + 4 * 24);
        assert_eq!(parse(&b), Some(s.clone()));
        mask_section(&mut b);
        assert!(is_masked(&b));
        assert_eq!(parse(&b), Some(s));
    }

    fn pssi_fixture(version: u16, mood: u16, masked: bool) -> Vec<u8> {
        let mut b = b"PSSI".to_vec();
        b.extend_from_slice(&32u32.to_be_bytes());
        b.extend_from_slice(&80u32.to_be_bytes());
        b.extend_from_slice(&version.to_be_bytes());
        b.extend_from_slice(&24u16.to_be_bytes());
        b.extend_from_slice(&2u16.to_be_bytes());
        b.extend_from_slice(&mood.to_be_bytes());
        b.extend_from_slice(&[11, 22, 33, 44, 55, 66]);
        b.extend_from_slice(&128u16.to_be_bytes());
        b.extend_from_slice(&[77, 88, 254, 99]); // Unnamed bank and opaque header fields.
        for (index, beat, kind) in [(1u16, 16u16, 2u16), (2, 64, 5)] {
            b.extend_from_slice(&index.to_be_bytes());
            b.extend_from_slice(&beat.to_be_bytes());
            b.extend_from_slice(&kind.to_be_bytes());
            b.extend_from_slice(&[0x5a; 18]);
        }
        if masked {
            // Independent fixture encoder from the documented ASCII seed, rather than
            // production KEY or mask_section. Version selects the starting seed byte.
            for (byte, seed) in b[18..].iter_mut().zip(b"Kanzen-niRikaishita".iter().cycle().skip(version as usize)) {
                *byte ^= seed.wrapping_add(0x80).wrapping_add(2);
            }
        }
        b
    }

    #[test]
    fn pssi_accepts_masked_and_plain_versions_zero_and_one() {
        let expected = Structure { mood: 3, phrases: vec![Phrase { beat: 16, kind: 2 }, Phrase { beat: 64, kind: 5 }], end_beat: 128, rb: None };
        for version in [0, 1] {
            let plain = pssi_fixture(version, 3, false);
            let masked = pssi_fixture(version, 3, true);
            for (bytes, is_ciphertext) in [(&plain, false), (&masked, true)] {
                let before = bytes.clone();
                assert_eq!(is_masked(bytes), is_ciphertext);
                assert!(supported_section(bytes));
                let parsed = parse(bytes).unwrap();
                assert_eq!((parsed.mood, &parsed.phrases, parsed.end_beat), (expected.mood, &expected.phrases, expected.end_beat));
                let mut stored: Structure = serde_json::from_str(&serde_json::to_string(&parsed).unwrap()).unwrap();
                assert!(!set_kind(&mut stored, 64, 5));
                assert_eq!(pssi(&stored), plain, "unedited rows and header survive persistence");
                let mut direct_edit = stored.clone();
                direct_edit.phrases[0].beat += 1;
                assert_eq!(parse(&pssi(&direct_edit)).unwrap().phrases, direct_edit.phrases);
                assert!(cut(&mut stored, 32));
                assert!(set_kind(&mut stored, 32, 8));
                let mut edited = pssi(&stored);
                assert_eq!(pssi_layout(&edited), Some((version as usize, 3)));
                assert_eq!(&edited[20..26], &[0; 6]);
                mask_section(&mut edited);
                assert_eq!(parse(&edited).unwrap(), stored);
                assert_eq!(*bytes, before, "parsing must not change source bytes");
            }
            let mut transformed = plain.clone();
            mask_section(&mut transformed);
            assert_eq!(transformed, masked);
            mask_section(&mut transformed);
            assert_eq!(transformed, plain, "opaque fields survive mask roundtrip");
        }
        assert_eq!(&pssi_fixture(1, 3, true)[18..20], &[0xe3, 0xf3]);
    }

    #[test]
    fn pssi_rejects_unsupported_or_inconsistent_framing() {
        let plain = pssi_fixture(1, 3, false);
        for end in 0..plain.len() {
            assert!(parse(&plain[..end]).is_none(), "prefix {end}");
        }
        for (offset, value) in [(12, 2u16), (14, 23), (16, 1), (16, 3), (16, u16::MAX)] {
            let mut bad = plain.clone();
            bad[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
            let before = bad.clone();
            assert!(parse(&bad).is_none());
            mask_section(&mut bad);
            assert_eq!(bad, before, "unsupported framing must stay untouched");
        }
        for (offset, value) in [(4, 30u32), (8, 79), (8, 81)] {
            let mut bad = plain.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
            assert!(parse(&bad).is_none());
        }
        let mut trailing = plain;
        trailing.push(0);
        assert!(parse(&trailing).is_none());
    }

    #[test]
    fn pssi_rejects_invalid_moods_after_decoding() {
        for version in [0, 1] {
            for mood in [0, 4, u16::MAX] {
                for masked in [false, true] {
                    let bytes = pssi_fixture(version, mood, masked);
                    assert!(!supported_section(&bytes));
                    assert!(parse(&bytes).is_none());
                }
            }
        }
    }

    /// Compare Recordbox's phrases with rekordbox's for the user's library, on rekordbox's own
    /// grids. PREPDECK_ANLZ_PAIRS = tsv "audio path\trekordbox ANLZ base". Run with --ignored.
    #[test]
    #[ignore]
    fn cue_proposal_benchmark() {
        // How well phrase-start proposals match the cues the user set in rekordbox (hot and
        // memory, read from rekordbox's own files): recall = user cues within one beat of a
        // proposal, precision = proposals within one beat of a user cue. rekordbox's own phrase
        // starts are scored the same way for reference.
        // The user's cues come from a snapshot of the rekordbox library (the original is not
        // touched); rekordbox keeps cues in master.db, not in its analysis files.
        let dest = std::env::temp_dir().join(format!("recordbox_cuebench_{}", std::process::id()));
        let Ok(Some(lib)) = crate::rbimport::load_installed(&dest) else { return };
        let rows: Vec<(String, String, Vec<f64>)> = lib
            .tracks
            .iter()
            .filter_map(|t| Some((t.xt.track.path.clone(), t.anlz_base.as_ref()?.to_string_lossy().to_string(), t.xt.cues.iter().map(|c| c.start).collect::<Vec<f64>>())))
            .filter(|r| !r.2.is_empty())
            .collect();
        let res = std::sync::Mutex::new(vec![]);
        std::thread::scope(|sc| {
            for ch in rows.chunks((rows.len() / 8).max(1)) {
                let res = &res;
                sc.spawn(move || {
                    for (audio, base, user) in ch {
                        let dat = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
                        let Some(q) = dat.get("PQTZ") else { continue };
                        let n = u32::from_be_bytes(q[20..24].try_into().unwrap()) as usize;
                        let beats: Vec<Beat> = (0..n).map(|k| {
                            let o = 24 + k * 8;
                            Beat { time: u32::from_be_bytes(q[o + 4..o + 8].try_into().unwrap()) as f64 / 1000.0, bpm: u16::from_be_bytes([q[o + 2], q[o + 3]]) as f64 / 100.0, beat: u16::from_be_bytes([q[o], q[o + 1]]) as u8, index: k }
                        }).collect();
                        let beat_len = 60.0 / beats.first().map(|b| b.bpm).unwrap_or(120.0).max(1.0);
                        if std::env::var("PREPDECK_CUE_DUMP2").is_ok() {
                            let down = beats.iter().find(|b| b.beat == 1).map(|b| b.time).unwrap_or(-1.0);
                            for u in user.iter() {
                                eprintln!("cue2 at_first_downbeat={} at_first_beat={}", (u - down).abs() < 0.002, (u - beats[0].time).abs() < 0.002);
                            }
                        }
                        if std::env::var("PREPDECK_CUE_DUMP").is_ok() {
                            let first = beats.first().map(|b| b.time).unwrap_or(0.0);
                            eprintln!("cues {:?} first beat {:.3}", user.iter().map(|u| format!("{:.2}s (beat {:.1})", u, (u - first) / beat_len)).collect::<Vec<_>>(), first);
                        }
                        let Ok(d) = crate::audio::decode_file(audio) else { continue };
                        let Some(mine) = analyze(&d.mono(), d.sr as f64, &beats) else { continue };
                        let ours: Vec<f64> = cue_proposals(&mine, &beats).iter().map(|p| p.0).collect();
                        let rb_ext = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.EXT")));
                        let theirs: Vec<f64> = rb_ext.get("PSSI").and_then(|s| parse(s)).map(|s| cue_proposals(&s, &beats).iter().map(|p| p.0).collect()).unwrap_or_default();
                        let near = |a: &[f64], t: f64| a.iter().any(|x| (x - t).abs() <= beat_len * 1.01);
                        let score = |props: &[f64]| (user.iter().filter(|u| near(props, **u)).count(), props.iter().filter(|p| near(&user, **p)).count(), props.len());
                        res.lock().unwrap().push((user.len(), score(&ours), score(&theirs)));
                    }
                });
            }
        });
        let r = res.into_inner().unwrap();
        let users: usize = r.iter().map(|x| x.0).sum();
        let sum = |f: &dyn Fn(&(usize, (usize, usize, usize), (usize, usize, usize))) -> (usize, usize, usize)| r.iter().map(f).fold((0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
        let o = sum(&|x| x.1);
        let t = sum(&|x| x.2);
        eprintln!("{} tracks, {users} user cues", r.len());
        eprintln!("recordbox phrase starts: recall {:.1}%, precision {:.1}% ({} proposals)", 100.0 * o.0 as f64 / users as f64, 100.0 * o.1 as f64 / o.2.max(1) as f64, o.2);
        eprintln!("rekordbox phrase starts: recall {:.1}%, precision {:.1}% ({} proposals)", 100.0 * t.0 as f64 / users as f64, 100.0 * t.1 as f64 / t.2.max(1) as f64, t.2);
    }

    #[test]
    #[ignore]
    fn phrase_benchmark() {
        let Ok(f) = std::env::var("PREPDECK_ANLZ_PAIRS") else { return };
        let rows: Vec<(String, String)> = std::fs::read_to_string(f).unwrap().lines().filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string()))).collect();
        let res = std::sync::Mutex::new(vec![]);
        std::thread::scope(|sc| {
            for ch in rows.chunks((rows.len() / 8).max(1)) {
                let res = &res;
                sc.spawn(move || {
                    for (audio, base) in ch {
                        let ext = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.EXT")));
                        let Some(rb) = ext.get("PSSI").and_then(|s| parse(s)) else { continue };
                        if rb.mood != 1 {
                            continue;
                        }
                        let dat = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
                        let Some(q) = dat.get("PQTZ") else { continue };
                        let n = u32::from_be_bytes(q[20..24].try_into().unwrap()) as usize;
                        let beats: Vec<Beat> = (0..n).map(|k| {
                            let o = 24 + k * 8;
                            Beat { time: u32::from_be_bytes(q[o + 4..o + 8].try_into().unwrap()) as f64 / 1000.0, bpm: u16::from_be_bytes([q[o + 2], q[o + 3]]) as f64 / 100.0, beat: u16::from_be_bytes([q[o], q[o + 1]]) as u8, index: k }
                        }).collect();
                        let Ok(d) = crate::audio::decode_file(audio) else { continue };
                        let Some(mine) = analyze(&d.mono(), d.sr as f64, &beats) else { continue };
                        // Per-beat label agreement and boundary F1 (exact beat).
                        let label_at = |s: &Structure, b: u16| s.phrases.iter().rev().find(|p| p.beat <= b).map(|p| p.kind).unwrap_or(0);
                        let total = rb.end_beat.max(1);
                        let same = (1..=total).filter(|b| label_at(&rb, *b) == label_at(&mine, *b)).count();
                        let rb_b: std::collections::HashSet<u16> = rb.phrases.iter().map(|p| p.beat).collect();
                        let my_b: std::collections::HashSet<u16> = mine.phrases.iter().map(|p| p.beat).collect();
                        let hit = rb_b.intersection(&my_b).count();
                        res.lock().unwrap().push((same as f64 / total as f64, hit, rb_b.len(), my_b.len()));
                    }
                });
            }
        });
        let r = res.into_inner().unwrap();
        let n = r.len() as f64;
        let label = r.iter().map(|x| x.0).sum::<f64>() / n;
        let (hit, rbn, myn): (usize, usize, usize) = r.iter().fold((0, 0, 0), |a, x| (a.0 + x.1, a.1 + x.2, a.2 + x.3));
        let p = hit as f64 / myn.max(1) as f64;
        let rc = hit as f64 / rbn.max(1) as f64;
        eprintln!("tracks {n}  label agreement {:.1}%  boundary precision {:.1}% recall {:.1}% F1 {:.1}%", label * 100.0, p * 100.0, rc * 100.0, 2.0 * p * rc / (p + rc).max(1e-9) * 100.0);
    }
}
