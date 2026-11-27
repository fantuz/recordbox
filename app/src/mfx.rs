//! MERGE FX (rekordbox 7.2.19 `MergeFX::*`, `effectGui::MFX*`; spec `checks/re-7.2.19/
//! mixer-fx-recording.md` 2.8): two units (left for decks 1 / 3, right for decks 2 / 4).
//! ON starts a BUILD FX on the deck and a BUILD SAMPLE (or oscillator) beside it, the knob
//! morphs both; OFF plays the RELEASE FX for its drop time, then the DROP SAMPLE, and
//! (optionally) the deck stops.
//!
//! From rekordbox: the component ids and names (`MFX_FX`, 1..44), the eight presets and the
//! default MERGE FX 1..4, the drop times (`MergeFX::dropTimes`), the sequence, and for every
//! BUILD FX which effects it is made of and how the knob moves their settings (decoded from
//! `MergeFX::parameters`; written here as rules). The sample sounds are read from the
//! installed rekordbox's `MERGE FX.zip` when MERGE FX is first enabled; nothing is copied.
//! The BUILD SAMPLE's repeats follow `MFXController::changeNotesPerBeat` (0x1012c787c): a
//! four-beat, 32-step note pattern with 1, 2, 4 or 8 notes a beat for |knob| below 25, 50,
//! 75 and from 75 %; a change while the deck plays restarts the grid 0.02 beats later, a
//! change on a stopped deck keeps its phase. REPEAT UP (pattern 201) sends the sample through
//! CFX PITCH, COLOR 0.5 -> 1 over the travel, PARAM 0.5 (one octave up at the end).
//! Where the sounds come from (`MFXPattern::getSamplePathFromPreset` 0x1012c3ab0): the factory
//! components play "<Music>/rekordbox/Sampler/MERGE FX/MergeFX Sample Sound NNN.wav"
//! (`SettingIF::getDefaultMFXSampleDirectory`; recordbox falls back to rekordbox's own
//! `MERGE FX.zip` when that folder is missing); REPEAT SAMPLE, REPEAT UP SAMPLE and SAMPLER
//! (`MFXPreset::isFilePathSelectable`: ids 21, 22, 43) play the file stored in the preset, one
//! path for the BUILD SAMPLE column and one for the DROP SAMPLE column, chosen in the settings
//! from the sampler's slots by bank (`PresetComponent::showSampleSelectionAt`). A path whose
//! file is gone is silent, as in rekordbox.
//! recordbox's own: the effect bodies (see `fx.rs` / `cfx.rs`), the release echoes' pitch as
//! a change of speed, the repeats' tempo (the deck's; rekordbox's sequence player starts at
//! 120 BPM and its sync was not traced). The repeats restart the sample through the clip
//! player's 1 ms fade (`Attenuator`).
//! ROLL's beat steps are the echoes' step table, as the parameter table says.

use crate::cfx::Cfx;
use crate::fx::{Kind, Ring, Slot, SlotParams};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// "-" (no component).
pub const NONE: u8 = 44;

/// A component's name (`MFX_FX`).
pub fn name(id: u8) -> &'static str {
    const N: [&str; 43] = ["ECHO", "FILTER ECHO", "JET ECHO", "SPACE UP", "SPACE DOWN", "ROLL", "ROLL UP", "ZIP DOWN", "LONG DOWN ECHO", "BRAKE DOWN ECHO", "MOBIUS UP SAW", "MOBIUS LFO UP SAW", "MOBIUS DOWN SAW", "NOISE", "REPEAT SNARE9", "REPEAT UP SNARE9", "REPEAT SNARE8", "REPEAT UP SNARE8", "REPEAT CLAP", "REPEAT CYMBAL", "REPEAT SAMPLE", "REPEAT UP SAMPLE", "-", "ECHO OUT", "UP ECHO OUT", "DOWN ECHO OUT", "REVERB OUT", "VINYL BRAKE ECHO", "CRASH CYMBAL", "SNARE", "BOOM DOWN LIFTER1", "BOOM DOWN LIFTER2", "NOISE DOWN LIFTER1", "NOISE DOWN LIFTER2", "IMPACT KICK", "IMPACT SNARE", "SHORT BOOM", "NEEDLE SCRATCH NOISE", "HORN", "REWIND", "SHORT BOOM", "LET'S GO", "SAMPLER"];
    if (1..=43).contains(&id) { N[id as usize - 1] } else { "-" }
}

/// The four columns of the settings: BUILD FX, BUILD SAMPLE, RELEASE FX, DROP SAMPLE, and
/// the ids each offers.
pub const COLUMNS: [(&str, &[u8]); 4] = [("BUILD FX", &[44, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]), ("BUILD SAMPLE", &[44, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22]), ("RELEASE FX", &[23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37]), ("DROP SAMPLE", &[44, 38, 39, 40, 41, 42, 43])];

/// rekordbox's presets: name and (BUILD FX, BUILD SAMPLE, RELEASE FX, DROP SAMPLE).
pub const PRESETS: [(&str, [u8; 4]); 8] = [("SNARE RISER", [2, 15, 36, 44]), ("ROLL RISER", [6, 11, 31, 44]), ("JET RISER", [3, 14, 28, 38]), ("SPACE RISER", [4, 12, 25, 44]), ("FILTER DROP", [2, 14, 24, 44]), ("SPACE DROP", [5, 44, 28, 44]), ("ZIP DROP", [8, 13, 35, 44]), ("ECHO DROP", [10, 44, 28, 40])];

/// The number in a component's sample file name ("MergeFX Sample Sound NNN.wav").
pub fn sample_no(id: u8) -> Option<u16> {
    match id {
        15 | 16 => Some(101),
        17 | 18 => Some(102),
        19 => Some(103),
        20 => Some(104),
        29..=37 => Some(201 + (id - 29) as u16),
        38..=42 => Some(301 + (id - 38) as u16),
        _ => None,
    }
}

/// What a component plays: a factory sound's number, or 1 / 2 for the file chosen for the
/// BUILD SAMPLE / DROP SAMPLE column (REPEAT SAMPLE, REPEAT UP SAMPLE, SAMPLER).
pub fn voice_no(id: u8) -> Option<u16> {
    match id {
        21 | 22 => Some(1),
        43 => Some(2),
        _ => sample_no(id),
    }
}

/// `MFXPreset::isFilePathSelectable`: the components that play a file of the user's choice.
pub fn path_selectable(id: u8) -> bool {
    matches!(id, 21 | 22 | 43)
}

/// `MFXController::changeNotesPerBeat`: the repeats a beat for |knob| in percent.
pub fn notes_per_beat(pct: f32) -> u32 {
    if pct < 25.0 { 1 } else if pct < 50.0 { 2 } else if pct < 75.0 { 4 } else { 8 }
}

/// The pattern it writes: 32 steps of an eighth of a beat, a note where `step % (8 / n) == 0`.
#[cfg(test)]
pub fn note_steps(n: u32) -> [bool; 32] {
    let d = 8 / n.clamp(1, 8);
    std::array::from_fn(|i| i as u32 % d == 0)
}

const FILES: [u16; 18] = [101, 102, 103, 104, 201, 202, 203, 204, 205, 206, 207, 208, 209, 301, 302, 303, 304, 305];

/// How long the RELEASE FX runs before the drop (`MergeFX::dropTimes`): the release effects
/// two beats, VINYL BRAKE ECHO a second, the release samples their own lengths.
pub fn drop_secs(id: u8, bpm: f32) -> f32 {
    match id {
        24..=27 => 2.0 * 60.0 / bpm.clamp(20.0, 1000.0),
        28 => 1.0,
        29..=37 => [1.8, 0.72, 2.0, 1.6, 2.2, 3.6, 1.5, 2.5, 0.72][id as usize - 29],
        _ => 0.0,
    }
}

/// What is kept between sessions (rekordbox's `MFXSettings.xml` and its two preferences).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MfxPrefs {
    /// "Enable MERGE FX" and "Stop playing the operating deck after MERGE FX finishes".
    pub enabled: bool,
    pub stop_deck: bool,
    /// MERGE FX 1..4: the preset each started from (0..7) and its four components.
    pub preset: [u8; 4],
    pub comps: [[u8; 4]; 4],
    /// Which of MERGE FX 1..4 each unit uses, and whether it is on decks 3 / 4.
    pub select: [u8; 2],
    pub upper: [bool; 2],
    /// Sample volume, dB (-12 .. +12).
    pub volume: f32,
    /// MERGE FX 1..4: the files chosen for the BUILD SAMPLE and the DROP SAMPLE column
    /// (rekordbox's preset keeps a path for SOUND_FX1 and SOUND_FX2; empty = none).
    pub paths: [[String; 2]; 4],
}

impl Default for MfxPrefs {
    fn default() -> Self {
        // rekordbox's defaults: SNARE RISER, SPACE RISER, SPACE DROP, ECHO DROP.
        let preset = [0u8, 3, 5, 7];
        MfxPrefs { enabled: false, stop_deck: true, preset, comps: preset.map(|p| PRESETS[p as usize].1), select: [0, 0], upper: [false; 2], volume: 0.0, paths: Default::default() }
    }
}

impl MfxPrefs {
    /// The deck (0..3) of unit `u`.
    pub fn deck(&self, u: usize) -> usize {
        u + if self.upper[u] { 2 } else { 0 }
    }
}

/// What the interface asks of a unit.
#[derive(Clone, Copy)]
pub struct Cmd {
    pub on: bool,
    /// Counted up to cancel (Shift + ON): everything stops without release or drop.
    pub cancel: u32,
    /// -1 .. 1, 0 at rest.
    pub knob: f32,
    pub comps: [u8; 4],
    pub deck: u8,
    pub bpm: f32,
    /// Sample volume as a gain.
    pub gain: f32,
    /// The deck is playing (a change of the repeat rate then restarts its grid).
    pub playing: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Idle,
    Build,
    Release,
    Drop,
}

/// rekordbox's knob rule: `x` up to `a` percent, `y` from `b` percent, a straight line between.
pub fn lin(pct: f32, a: f32, b: f32, x: f32, y: f32) -> f32 {
    if pct <= a { x } else if pct >= b { y } else { x + (y - x) * (pct - a) / (b - a) }
}

/// A beat value from a table for the knob's travel (equal steps).
fn step(pct: f32, table: &[f32]) -> f32 {
    table[((pct / 100.0 * table.len() as f32) as usize).min(table.len() - 1)]
}

const ECHO_BEATS: [f32; 4] = [0.125, 0.25, 0.5, 0.75];
const RISE_BEATS: [f32; 8] = [1.0 / 8.0, 1.0 / 10.0, 1.0 / 12.0, 1.0 / 16.0, 1.0 / 24.0, 1.0 / 32.0, 1.0 / 48.0, 1.0 / 64.0];
/// ROLL and ROLL UP read their beat from the same step table as the echoes (table 4 in
/// `MergeFX::parameters`, rows of patterns 106 / 107, param 1: curve 2, sub-type 3).
const ROLL_BEATS: [f32; 4] = ECHO_BEATS;

/// What a BUILD FX does at a knob position: up to three effect slots (with the mode their
/// settings are read in), two colour effects (kind, COLOR, PARAM) and the dry sound's gain.
pub struct Build {
    pub slots: [Option<(SlotParams, bool)>; 3],
    pub cfx: [Option<(u8, f32, f32)>; 2],
    pub dry: f32,
}

/// The BUILD FX `id` at knob `k` (-1 .. 1), as decoded from rekordbox's parameter table.
pub fn build(id: u8, k: f32) -> Build {
    let right = k > 0.0;
    let pct = k.abs().min(1.0) * 100.0;
    let (r, l) = (if right { pct } else { 0.0 }, if right { 0.0 } else { pct });
    let multi = |kind: Kind, level: f32, beats: f32| Some((SlotParams { on: level > 0.0, level, value: beats, ..SlotParams::new(kind) }, true));
    let single = |kind: Kind, level: f32, beats: f32, knob: [f32; 2]| Some((SlotParams { on: level > 0.0, level, value: beats, knob, ..SlotParams::new(kind) }, false));
    let col = |kind: u8, amount: f32, param: f32| if amount.abs() > 0.004 { Some((kind, 0.5 + 0.5 * amount, param)) } else { None };
    // The echo most of them share: in over the first quarter of the travel, on either side.
    let echo = multi(Kind::LowCutEcho, lin(pct, 0.0, 25.0, 0.0, 0.5), step(pct, &ECHO_BEATS));
    let mut b = Build { slots: [None, None, None], cfx: [None, None], dry: 1.0 };
    match id {
        1 => {
            b.slots[0] = multi(Kind::LowCutEcho, lin(l, 0.0, 25.0, 0.0, 0.5), step(l, &ECHO_BEATS));
            b.slots[1] = single(Kind::LowCutEcho, lin(r, 0.0, 40.0, 0.0, 0.3), step(r, &RISE_BEATS), [lin(r, 0.0, 100.0, 0.7, 0.9), 0.5]);
        }
        2 => {
            b.cfx[0] = col(crate::cfx::LPF, r / 100.0, 0.5);
            b.cfx[1] = col(crate::cfx::HPF, l / 100.0, 0.5);
            b.slots[0] = echo;
        }
        3 => {
            b.cfx[0] = col(1, r / 100.0, 0.5);
            b.slots[1] = multi(Kind::EnigmaJet, lin(l, 0.0, 25.0, 0.0, 0.7), 4.0);
            b.slots[0] = echo;
        }
        4 => {
            b.cfx[0] = col(1, r / 100.0, 0.5);
            b.cfx[1] = col(crate::cfx::HPF, l / 100.0, 0.3);
            b.slots[0] = single(Kind::PitchEcho, lin(r, 0.0, 16.6, 0.0, 0.2), 0.5, [lin(r, 0.0, 66.6, 0.6, 1.0), 0.55]);
            b.slots[1] = multi(Kind::SpiralUp, l / 100.0, 0.0);
            b.dry = lin(r, 30.0, 100.0, 1.0, 0.0);
        }
        5 => {
            b.cfx[0] = col(crate::cfx::LPF, pct / 100.0, 0.3);
            b.slots[0] = multi(Kind::SpiralDown, r / 100.0, 0.0);
            b.slots[1] = multi(Kind::ReverbDown, l / 100.0, 0.0);
        }
        6 => {
            b.slots[0] = multi(Kind::Roll, if right { 1.0 } else { lin(l, 0.0, 100.0, 1.0, 0.5) }, step(pct, &ROLL_BEATS));
            b.slots[1] = multi(Kind::EnigmaJet, lin(l, 0.0, 25.0, 0.0, 0.7), 4.0);
        }
        7 => {
            b.slots[0] = multi(Kind::Roll, lin(pct, 0.0, 100.0, 0.5, 1.0), step(pct, &ROLL_BEATS));
            b.cfx[0] = col(4, l / 100.0, 0.5);
            b.slots[1] = single(Kind::PitchEcho, lin(r, 0.0, 30.0, 0.0, 0.35), 0.5, [lin(r, 0.0, 100.0, 0.75, 0.9), 0.55]);
        }
        8 => {
            b.cfx[0] = col(4, -if right { lin(r, 0.0, 50.0, 0.0, 1.0) } else { l / 100.0 }, 0.5);
            b.cfx[1] = if right { col(crate::cfx::LPF, lin(r, 50.0, 100.0, 0.0, 1.0), 0.5) } else { col(2, l / 100.0, 0.5) };
        }
        9 => {
            // (DryCutSpiral: the spiral alone.)
            b.slots[0] = multi(Kind::Spiral, 0.75, 1.0);
            b.cfx[0] = col(4, -l / 100.0, 1.0);
            b.dry = 0.0;
        }
        10 => {
            b.slots[0] = single(Kind::VinylBrake, 1.0, 4.0, [0.5, 0.0]);
            b.slots[1] = single(Kind::Spiral, 1.0, 1.0, [1.0, 0.5]);
            b.cfx[0] = col(4, -l / 100.0, 1.0);
        }
        _ => {}
    }
    b
}

/// A decoded sample sound: interleaved stereo and its rate.
pub struct Sample {
    pub data: Vec<f32>,
    pub rate: f32,
}

/// Up to 16 MiB of encoded PCM per factory sound and 128 MiB of decoded stereo
/// across the 18 known filenames. User-selected audio uses the regular decoder.
const MAX_FACTORY_WAV: u64 = 16 << 20;
const FACTORY_FLOATS: usize = (128 << 20) / size_of::<f32>();

/// PCM WAV (16 or 24 bit, mono or stereo) to interleaved stereo.
fn parse_wav(b: &[u8], remaining_floats: usize) -> Option<Sample> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let (mut pos, mut fmt) = (12usize, None);
    while let Some(header) = b.get(pos..pos.checked_add(8)?) {
        let len = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
        pos += 8;
        let end = pos.checked_add(len)?;
        let body = b.get(pos..end)?;
        match &header[..4] {
            b"fmt " if body.len() >= 16 => {
                let tag = u16::from_le_bytes(body[..2].try_into().ok()?);
                // WAVE_FORMAT_EXTENSIBLE with the PCM subformat GUID.
                let pcm_ext = tag == 0xfffe && body.get(24..40) == Some(&[1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
                let ch = u16::from_le_bytes(body[2..4].try_into().ok()?) as usize;
                let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                let bits = u16::from_le_bytes(body[14..16].try_into().ok()?);
                if (tag != 1 && !pcm_ext) || !(1..=2).contains(&ch) || !(1..=384_000).contains(&rate) || !matches!(bits, 16 | 24) {
                    return None;
                }
                fmt = Some((ch, rate, bits as usize / 8));
            }
            b"data" => {
                let (ch, rate, bytes) = fmt?;
                let frames = len / (ch * bytes);
                let count = frames.checked_mul(2)?;
                if frames < 2 || len % (ch * bytes) != 0 || count > remaining_floats {
                    return None;
                }
                let one = |s: &[u8]| if bytes == 2 { i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0 } else { (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8388608.0 };
                let mut data = Vec::new();
                data.try_reserve_exact(count).ok()?;
                for f in body.chunks_exact(bytes * ch) {
                    let l = one(f);
                    data.extend([l, if ch == 2 { one(&f[bytes..]) } else { l }]);
                }
                return Some(Sample { data, rate: rate as f32 });
            }
            _ => {}
        }
        pos = end.checked_add(len & 1)?;
    }
    None
}

fn factory_dir() -> Option<PathBuf> {
    dirs::audio_dir().or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .map(|p| p.join("rekordbox/Sampler/MERGE FX"))
}

/// fx9: the engine's sample table (one place a file of `FILES`).
pub type SampleTable = [Option<Arc<Sample>>; 18];

/// fx9: the table for `Engine::swap_samples`, made from `load_samples`' list (on the worker).
pub fn sample_table(list: &[(u16, Arc<Sample>)]) -> SampleTable {
    let mut t: SampleTable = Default::default();
    for (no, s) in list {
        if let Some(i) = FILES.iter().position(|f| f == no) {
            t[i] = Some(s.clone());
        }
    }
    t
}

/// Only read the declared number of bytes from a WAV, in memory. This also
/// bounds decompression when a ZIP member's metadata disagrees with its contents.
fn read_factory_wav(reader: impl Read, size: u64, remaining: usize) -> Option<Sample> {
    if size > MAX_FACTORY_WAV { return None; }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size as usize).ok()?;
    let read = reader.take(size + 1).read_to_end(&mut bytes).ok()?;
    (read as u64 == size).then(|| parse_wav(&bytes, remaining)).flatten()
}

fn open_factory_zip(path: &Path) -> Option<zip::ZipArchive<std::io::BufReader<std::fs::File>>> {
    let file = std::fs::File::open(path).ok()?;
    let info = file.metadata().ok()?;
    if !info.is_file() || info.len() > 64 << 20 { return None; }
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).ok()?;
    if archive.len() > 256 { return None; }
    archive.validate_header_consistency().ok()?;
    Some(archive)
}

fn factory_samples(dir: Option<&Path>, archive: Option<&Path>) -> Vec<(u16, Arc<Sample>)> {
    let mut archive = archive.and_then(open_factory_zip);
    let mut remaining = FACTORY_FLOATS;
    let mut out = Vec::new();
    for no in FILES {
        let name = format!("MergeFX Sample Sound {no}.wav");
        let sample = dir.and_then(|dir| {
            let path = dir.join(&name);
            if !path.metadata().ok()?.is_file() { return None; }
            let file = std::fs::File::open(path).ok()?;
            let size = file.metadata().ok()?.len();
            read_factory_wav(file, size, remaining)
        }).or_else(|| {
            let entry = archive.as_mut()?.by_name(&name).ok()?;
            if !entry.is_file() || entry.is_symlink() { return None; }
            let size = entry.size();
            read_factory_wav(entry, size, remaining)
        });
        if let Some(sample) = sample {
            remaining -= sample.data.len();
            out.push((no, Arc::new(sample)));
        }
    }
    out
}

/// Prefer loose sounds in the platform Music folder. All platforms also support
/// a user-provided MERGE FX.zip here or via RECORDBOX_MERGE_FX_ARCHIVE.
pub fn load_samples() -> Vec<(u16, Arc<Sample>)> {
    let dir = factory_dir();
    let archive = std::env::var_os("RECORDBOX_MERGE_FX_ARCHIVE").map(PathBuf::from)
        .or_else(|| dir.as_ref().map(|p| p.join("MERGE FX.zip")).filter(|p| p.is_file()));
    #[cfg(target_os = "macos")]
    let archive = archive.or_else(|| Some(PathBuf::from("/Applications/rekordbox 7/rekordbox.app/Contents/Resources/MERGE FX.zip")));
    factory_samples(dir.as_deref(), archive.as_deref())
}

/// Decode the first two minutes of the selected sampler file on a worker.
pub fn load_user(path: &str) -> Option<Arc<Sample>> {
    if path.is_empty() || !Path::new(path).is_file() { return None; }
    let d = crate::audio::decode_prefix(path, 120).ok()?;
    (d.samples.len() >= 4).then(|| Arc::new(Sample { data: d.samples, rate: d.sr as f32 }))
}

/// A sample being played: its place (frames of the file) or `None`.
#[derive(Clone, Copy, Default)]
struct Voice {
    no: u16,
    pos: Option<f64>,
    speed: f32,
    /// Frames of this buffer to wait before it starts.
    wait: usize,
}

struct UnitEng {
    slots: [Slot; 3],
    cfx: [Cfx; 2],
    cfx_last: [(u8, f32); 2],
    ring: Ring,
    phase: Phase,
    /// Frames into the phase, and the release's length.
    t: u64,
    rel_len: u64,
    comps: [u8; 4],
    cancel: u32,
    knob: f32,
    /// The release on the deck: where the caught beat starts in the history, the gate.
    rel_start: usize,
    gate: f32,
    brake: (f32, f64),
    /// Sounds beside the deck: the repeating build sample, the release sample and the drop
    /// sample; the oscillator's phases, the noise's memory and the repeat clock (beats).
    build: Voice,
    hit: [Voice; 2],
    osc: [f32; 6],
    sweep: f32,
    seed: u32,
    nz: [f32; 2],
    clock: f64,
    dry: f32,
    /// The repeat pattern in use (notes a beat, 0 before the first) and the beat its grid
    /// starts from.
    npb: u32,
    grid0: f64,
    /// fx9: the build sample's clip player fade (`crate::mfx_att`, rekordbox's `Attenuator`)
    /// and the rate it was made at.
    att: crate::mfx_att::Attenuator,
    att_rate: f32,
    /// REPEAT UP: CFX PITCH on the build sample, and the buffer it is made in.
    pitch: Cfx,
    sbuf: Vec<f32>,
    /// UP / DOWN ECHO OUT (`ReleasePitchEcho`, a `PitchEchoCore`): its line (stereo), write
    /// place and pitch shifter.
    pe_line: Vec<f32>,
    pe_w: usize,
    pe_ps: crate::fx_core::PitchShifter,
}

impl UnitEng {
    fn new() -> UnitEng {
        let mut e = UnitEng { slots: [Slot::new(), Slot::new(), Slot::new()], cfx: [Cfx::new(), Cfx::new()], cfx_last: [(0, 0.5); 2], ring: Ring::new(), phase: Phase::Idle, t: 0, rel_len: 0, comps: [NONE; 4], cancel: 0, knob: 0.0, rel_start: 0, gate: 0.0, brake: (1.0, 0.0), build: Voice::default(), hit: [Voice::default(); 2], osc: [0.0; 6], sweep: 0.0, seed: 0x1357_9BDF, nz: [0.0; 2], clock: 0.0, dry: 1.0, npb: 0, grid0: 0.0, att: crate::mfx_att::Attenuator::new(48000.0, 1.0), att_rate: 48000.0, pitch: Cfx::new(), sbuf: vec![0.0; SBUF * 2], pe_line: vec![0.0; PE_LEN * 2], pe_w: 0, pe_ps: crate::fx_core::PitchShifter::new(48000.0) };
        // (Made by the interface: the echo cores of the slots, so that MERGE FX runs rekordbox's
        // echo and not the old shared line.)
        for s in e.slots.iter_mut() {
            s.ensure_merge_cores();
        }
        e
    }
}

/// Frames the build sample is made in at a time (longer buffers go in pieces).
const SBUF: usize = 2048;
/// UP / DOWN ECHO OUT's line: 4 s at 48 kHz and a block.
const PE_LEN: usize = 4 * 48000 + 1024;

/// The MERGE FX engine (about 25 MB; made when MERGE FX is first enabled).
pub struct Engine {
    pub cmd: [Cmd; 2],
    units: [UnitEng; 2],
    samples: SampleTable,
    /// Per unit, the files chosen for the BUILD SAMPLE and DROP SAMPLE columns (decoded on
    /// the interface's side and handed over).
    user: [[Option<Arc<Sample>>; 2]; 2],
    /// Counted up each time a unit's drop starts (the interface then stops the deck).
    pub dropped: [u32; 2],
    off: SlotParams,
}

impl Engine {
    pub fn new() -> Engine {
        let cmd = Cmd { on: false, cancel: 0, knob: 0.0, comps: [NONE; 4], deck: 0, bpm: 120.0, gain: 1.0, playing: false };
        Engine { cmd: [cmd, Cmd { deck: 1, ..cmd }], units: [UnitEng::new(), UnitEng::new()], samples: Default::default(), user: Default::default(), dropped: [0; 2], off: SlotParams::new(Kind::Delay) }
    }

    /// Hand over (or take back) the file of a unit's column (0 BUILD SAMPLE, 1 DROP SAMPLE).
    /// The interface receives the old sample and releases it after unlocking the rack.
    pub fn set_user(&mut self, u: usize, col: usize, s: Option<Arc<Sample>>) -> Option<Arc<Sample>> {
        std::mem::replace(&mut self.user[u][col], s)
    }

    /// (Tests: the samples of `list` in their places.)
    #[cfg(test)]
    pub fn set_samples(&mut self, list: &[(u16, Arc<Sample>)]) {
        for (no, s) in list {
            if let Some(i) = FILES.iter().position(|f| f == no) {
                self.samples[i] = Some(s.clone());
            }
        }
    }

    /// fx9: put a table made by `sample_table` in place (the old one comes back in `t`). Only
    /// pointers move: for the interface thread with the rack locked.
    pub fn swap_samples(&mut self, t: &mut SampleTable) {
        std::mem::swap(&mut self.samples, t);
    }

    pub fn phase(&self, u: usize) -> Phase {
        self.units[u].phase
    }

    /// Turning off MERGE FX or leaving Performance cancels the two units without
    /// playing a drop or queuing a deck stop on return.
    pub fn stop(&mut self, state: &mut MfxState) {
        for u in 0..2 {
            let ui = &mut state.units[u];
            let cmd = &mut self.cmd[u];
            let unit = &mut self.units[u];
            ui.on = false;
            ui.phase_on = false;
            ui.knob = 0.0;
            ui.seen = self.dropped[u];
            cmd.on = false;
            cmd.knob = 0.0;
            cmd.cancel = ui.cancel;
            unit.cancel = ui.cancel;
            unit.phase = Phase::Idle;
            unit.gate = 0.0;
            unit.knob = 0.0;
            unit.dry = 1.0;
            unit.build.pos = None;
            unit.hit = [Voice::default(); 2];
        }
    }

    pub fn busy(&self) -> bool {
        self.cmd.iter().any(|c| c.on) || self.units.iter().any(|u| u.phase != Phase::Idle || u.gate > 0.0 || u.hit.iter().any(|h| h.pos.is_some()))
    }

    /// A deck's audio on its way to the mixer: the unit on this deck applies its BUILD FX or
    /// RELEASE FX. (The phases themselves move in `master`, once per buffer.)
    pub fn deck(&mut self, ch: usize, buf: &mut [f32], rate: u32, beat: Option<f64>) {
        let r = rate as f32;
        for u in 0..2 {
            let cmd = self.cmd[u];
            if cmd.deck as usize != ch {
                continue;
            }
            let e = &mut self.units[u];
            let frames = buf.len() / 2;
            e.ring.write(buf);
            // BUILD FX: the knob glides (30 ms for the whole travel) and sets the effects.
            let building = e.phase == Phase::Build;
            let target = if building { cmd.knob.clamp(-1.0, 1.0) } else { 0.0 };
            let stepk = frames as f32 / (0.030 * r);
            e.knob += (target - e.knob).clamp(-stepk, stepk);
            let b = build(if building { e.comps[0] } else { NONE }, e.knob);
            let dry_step = frames as f32 / (0.0033333 * r);
            let dry_to = e.dry + (b.dry - e.dry).clamp(-dry_step, dry_step);
            for i in 0..2 {
                let (k, c, p) = b.cfx[i].unwrap_or((e.cfx_last[i].0, 0.5, e.cfx_last[i].1));
                e.cfx_last[i] = (k, p);
                e.cfx[i].process(k, c, p, buf, rate);
            }
            for i in 0..3 {
                match b.slots[i] {
                    Some((sp, multi)) => e.slots[i].process(&sp, cmd.bpm, buf, rate, &e.ring, beat, multi),
                    None => e.slots[i].process(&self.off, cmd.bpm, buf, rate, &e.ring, beat, true),
                }
            }
            if (e.dry - 1.0).abs() > 1e-6 || (dry_to - 1.0).abs() > 1e-6 {
                // (SPACE UP and LONG DOWN ECHO take the plain sound away: here the whole
                // result is turned down by that share while their echoes ring.)
                for (n, f) in buf.chunks_exact_mut(2).enumerate() {
                    let g = e.dry + (dry_to - e.dry) * n as f32 / frames as f32;
                    let keep = 0.35 + 0.65 * g;
                    f[0] *= keep;
                    f[1] *= keep;
                }
            }
            e.dry = dry_to;
            // RELEASE FX on the deck (the release samples leave the deck alone).
            let rel = e.comps[2];
            let releasing = e.phase == Phase::Release && (24..=28).contains(&rel);
            let gk = 1.0 / (0.0033333 * r);
            if releasing || e.gate > 0.0 {
                let beat_frames = (60.0 / cmd.bpm.clamp(20.0, 1000.0) * r) as usize;
                let len = beat_frames.clamp(480, 4 * 48000);
                for n in 0..frames {
                    e.gate = (e.gate + if releasing { gk } else { -gk }).clamp(0.0, 1.0);
                    let i = (e.t as usize).saturating_add(n);
                    if rel == 25 || rel == 26 {
                        // UP / DOWN ECHO OUT (`ReleasePitchEcho::processEffectConcrete`
                        // 0x1029cafd8): for one echo time (a beat) the track plays on and goes
                        // into a `PitchEchoCore`; then the input is shut and only the core is
                        // heard, at 0.7. Its repeats come back through rekordbox's pitch
                        // shifter at the preset's pitch (0.529 / 0.456 of -100 .. +100 %: +5.8 %
                        // / -8.8 %, by the shifter's percent law), feedback 0.8.
                        let pct = if rel == 25 { 0.529f32 * 200.0 - 100.0 } else { 0.456f32 * 200.0 - 100.0 };
                        e.pe_ps.set_percent(pct);
                        let d = len.min(PE_LEN - 1);
                        if e.pe_ps.block_left() == 0 {
                            let (line, w) = (&e.pe_line, e.pe_w);
                            let k = (frames - n).min(crate::fx_core::PitchShifter::BLOCK);
                            e.pe_ps.begin_block((0..k).map(|j| {
                                let at = ((w + PE_LEN + j - d) % PE_LEN) * 2;
                                (line[at], line[at + 1])
                            }));
                        }
                        let at = ((e.pe_w + PE_LEN - d) % PE_LEN) * 2;
                        let (sl, sr) = e.pe_ps.process(e.pe_line[at], e.pe_line[at + 1]);
                        let rec = i < len;
                        let (xl, xr) = if rec { (buf[n * 2], buf[n * 2 + 1]) } else { (0.0, 0.0) };
                        let k = e.pe_w * 2;
                        e.pe_line[k] = (xl + sl * 0.8).clamp(-64.0, 64.0);
                        e.pe_line[k + 1] = (xr + sr * 0.8).clamp(-64.0, 64.0);
                        e.pe_w = (e.pe_w + 1) % PE_LEN;
                        let g = e.gate;
                        let (wl, wr) = if rec { (buf[n * 2] + sl * 0.7, buf[n * 2 + 1] + sr * 0.7) } else { (sl * 0.7, sr * 0.7) };
                        buf[n * 2] = buf[n * 2] * (1.0 - g) + wl * g;
                        buf[n * 2 + 1] = buf[n * 2 + 1] * (1.0 - g) + wr * g;
                        continue;
                    }
                    let (wl, wr) = match rel {
                        28 => {
                            // VINYL BRAKE ECHO: the sound slows to a stop over the second.
                            e.brake.0 = crate::fx::brake_step(e.brake.0, 0.6, r);
                            e.brake.1 += e.brake.0 as f64;
                            let (a, b2) = e.ring.at(e.rel_start as f64 + e.brake.1);
                            let g = (e.brake.0 * 8.0).min(1.0);
                            (a * g, b2 * g)
                        }
                        27 => {
                            // REVERB OUT: the input is shut and the last beat rings out, fading.
                            let a = 0.5f32.powf(i as f32 / len as f32);
                            let (x, y) = e.ring.at((e.rel_start + len * 4 - len + i % len) as f64);
                            (x * a * 0.6, y * a * 0.6)
                        }
                        _ => {
                            // ECHO OUT (`ReleaseEcho`: 0.7 out, 0.7 a repeat).
                            let turn = i / len;
                            let fb = 0.7f32;
                            let pos = i % len;
                            let (x, y) = e.ring.at((e.rel_start + len * 4 - len + pos) as f64);
                            let a = fb.powi(turn as i32 + 1);
                            (x * a, y * a)
                        }
                    };
                    let g = e.gate;
                    buf[n * 2] = buf[n * 2] * (1.0 - g) + wl * g;
                    buf[n * 2 + 1] = buf[n * 2 + 1] * (1.0 - g) + wr * g;
                }
            }
        }
    }

    /// Once per buffer, on the master mix before the master's effects: move each unit through
    /// its phases and add its samples and oscillator.
    pub fn master(&mut self, buf: &mut [f32], rate: u32) {
        let r = rate as f32;
        let frames = buf.len() / 2;
        for u in 0..2 {
            let cmd = self.cmd[u];
            let e = &mut self.units[u];
            if cmd.cancel != e.cancel {
                // Shift + ON: everything of this unit stops, no release, no drop.
                e.cancel = cmd.cancel;
                e.phase = Phase::Idle;
                e.build.pos = None;
                e.hit = [Voice::default(); 2];
                continue;
            }
            match e.phase {
                Phase::Idle | Phase::Drop if cmd.on => {
                    e.phase = Phase::Build;
                    e.comps = cmd.comps;
                    e.t = 0;
                    e.clock = 0.0;
                    e.npb = 0;
                    e.grid0 = 0.0;
                    e.build = Voice { no: voice_no(cmd.comps[1]).unwrap_or(0), pos: None, speed: 1.0, wait: 0 };
                }
                Phase::Build if !cmd.on => {
                    // OFF: the RELEASE FX (or release sample) for its drop time.
                    e.phase = Phase::Release;
                    e.t = 0;
                    e.rel_len = (drop_secs(e.comps[2], cmd.bpm) * r) as u64;
                    e.rel_start = e.ring.w;
                    e.brake = (1.0, 0.0);
                    e.pe_line.iter_mut().for_each(|v| *v = 0.0);
                    e.pe_w = 0;
                    e.pe_ps.prepare(r);
                    e.build.pos = None;
                    if let Some(no) = sample_no(e.comps[2]) {
                        e.hit[0] = Voice { no, pos: Some(0.0), speed: 1.0, wait: 0 };
                    }
                }
                Phase::Drop if e.hit.iter().all(|h| h.pos.is_none()) && e.gate <= 0.0 => e.phase = Phase::Idle,
                _ => {}
            }
            if e.phase == Phase::Release && e.t + frames as u64 >= e.rel_len {
                // The drop: the DROP SAMPLE starts on the frame the release time ends.
                let wait = e.rel_len.saturating_sub(e.t) as usize;
                e.phase = Phase::Drop;
                self.dropped[u] = self.dropped[u].wrapping_add(1);
                if let Some(no) = voice_no(e.comps[3]) {
                    e.hit[1] = Voice { no, pos: Some(0.0), speed: 1.0, wait: wait.min(frames) };
                }
            }
            let bpm = cmd.bpm.clamp(20.0, 1000.0);
            let pct = e.knob.abs() * 100.0;
            // BUILD SAMPLE: the repeating samples are hit 1, 2, 4 or 8 times a beat as the knob
            // turns (`changeNotesPerBeat`); the UP ones go through CFX PITCH, up to an octave.
            let id = e.comps[1];
            let building = e.phase == Phase::Build;
            let repeat = (15..=22).contains(&id);
            if building && repeat {
                let n = notes_per_beat(pct);
                if n != e.npb {
                    if e.npb > 0 && cmd.playing {
                        // (rekordbox moves its four-beat sequence to 3.98: the new grid
                        // starts with a note 0.02 beats later.)
                        e.grid0 = e.clock + 0.02;
                    }
                    e.npb = n;
                }
            }
            let up = matches!(id, 16 | 18 | 22);
            let samples = &self.samples;
            let user = &self.user[u];
            let find = |no: u16| match no {
                1 | 2 => user[no as usize - 1].as_deref(),
                _ => FILES.iter().position(|f| *f == no).and_then(|i| samples[i].as_deref()),
            };
            let rate_ratio = |smp: &Sample| (smp.rate / r) as f64;
            let build_sample = find(e.build.no);
            let hits = e.hit.map(|v| find(v.no));
            let mut done = 0;
            while done < frames {
                let len = (frames - done).min(SBUF);
                let sb = &mut e.sbuf[..len * 2];
                sb.iter_mut().for_each(|v| *v = 0.0);
                // fx9: the clip player's fade for this block (`AudioPlayerCore`: the block's
                // discontinuous points through `Attenuator::letSmoothDiscontinuousPoints`, then
                // `apply`): the restarts the grid test below makes, found ahead over the block
                // and one fade more.
                let use_att = building && repeat && e.npb > 0;
                let mut att_g = [1.0f32; SBUF];
                if use_att {
                    if (e.att_rate - r).abs() > 0.5 {
                        e.att = crate::mfx_att::Attenuator::new(r, 1.0);
                        e.att_rate = r;
                    }
                    let mut pts = [0i32; 64];
                    let mut np = 0;
                    let mut c = e.clock;
                    for k in 0..len as i32 + e.att.len() {
                        let before = c;
                        c += bpm as f64 / 60.0 / r as f64;
                        let (a, b) = ((before - e.grid0) * e.npb as f64, (c - e.grid0) * e.npb as f64);
                        if b >= 0.0 && (a < 0.0 || a.floor() != b.floor() || before == e.grid0) && np < pts.len() {
                            pts[np] = k;
                            np += 1;
                        }
                    }
                    e.att.smooth(&pts[..np], len as i32);
                    e.att.apply(&mut att_g[..len]);
                }
                for k in 0..len {
                    let n = done + k;
                    let mut s = (0.0f32, 0.0f32);
                    if building {
                        let before = e.clock;
                        e.clock += bpm as f64 / 60.0 / r as f64;
                        if repeat && e.npb > 0 {
                            let (a, b) = ((before - e.grid0) * e.npb as f64, (e.clock - e.grid0) * e.npb as f64);
                            if b >= 0.0 && (a < 0.0 || a.floor() != b.floor() || before == e.grid0) {
                                e.build.pos = Some(0.0);
                            }
                        }
                        match id {
                            11..=13 => {
                                // The MOBIUS oscillators: six saws an octave apart climbing (or
                                // falling) an octave every 16 beats (LFO: every 4), loudest in the
                                // middle of the range; the level follows the knob.
                                let turn = if id == 12 { 4.0 } else { 16.0 };
                                let dir = if id == 13 { -1.0 } else { 1.0 };
                                e.sweep = (e.sweep + dir * bpm / 60.0 / r / turn).rem_euclid(1.0);
                                let lvl = if id == 13 { lin(pct, 50.0, 100.0, 0.8, 0.0) } else { lin(pct, 0.0, 100.0, 0.8, 1.0) };
                                let mut v = 0.0;
                                for k in 0..6 {
                                    let x = k as f32 + e.sweep;
                                    e.osc[k] = (e.osc[k] + 55.0 * 2f32.powf(x) / r).fract();
                                    v += (0.5 - 0.5 * (std::f32::consts::TAU * x / 6.0).cos()) * (2.0 * e.osc[k] - 1.0);
                                }
                                s = (v * 0.12 * lvl, v * 0.12 * lvl);
                            }
                            14 => {
                                // NOISE: white noise through a low-pass that opens with the knob.
                                e.seed ^= e.seed << 13;
                                e.seed ^= e.seed >> 17;
                                e.seed ^= e.seed << 5;
                                let x = (e.seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                                let fc = 300.0 * (12000.0f32 / 300.0).powf(pct / 100.0);
                                let k = 1.0 - (-std::f32::consts::TAU * fc / r).exp();
                                e.nz[0] += (x - e.nz[0]) * k;
                                e.nz[1] += (e.nz[0] - e.nz[1]) * k;
                                let lvl = 0.25 * lin(pct, 70.0, 100.0, 0.9, 1.0);
                                s = (e.nz[1] * lvl, e.nz[1] * lvl);
                            }
                            _ => {}
                        }
                    }
                    // The build sample, into its own buffer (it may be pitched). A repeat restarts
                    // it through the clip player's `Attenuator` (1 ms raised-sine curve,
                    // `rbxfrm::f2dsp::Attenuator::createCurve` 0x102887b84, made by
                    // `AudioPlayerCore`'s constructor 0x1028f3668): a sample still playing fades out
                    // into the restart point and the restart fades in (`AudioClipPlayer::
                    // launchAudioClip` 0x10294c620, `doMovePlayingPosWithDelay`; fx9: the gains
                    // worked out for the block above, `crate::mfx_att`).
                    let fade_g = att_g[k];
                    if let Some(pos) = e.build.pos {
                        match build_sample {
                            Some(smp) if (pos as usize + 1) * 2 + 1 < smp.data.len() => {
                                let i = pos as usize;
                                let t = (pos - i as f64) as f32;
                                sb[k * 2] += (smp.data[i * 2] + (smp.data[i * 2 + 2] - smp.data[i * 2]) * t) * fade_g;
                                sb[k * 2 + 1] += (smp.data[i * 2 + 1] + (smp.data[i * 2 + 3] - smp.data[i * 2 + 1]) * t) * fade_g;
                                e.build.pos = Some(pos + rate_ratio(smp));
                            }
                            _ => e.build.pos = None,
                        }
                    }
                    for (v, sample) in e.hit.iter_mut().zip(hits) {
                        if v.wait > n {
                            continue;
                        }
                        let Some(pos) = v.pos else { continue };
                        let Some(smp) = sample else {
                            v.pos = None;
                            continue;
                        };
                        let i = pos as usize;
                        if (i + 1) * 2 + 1 >= smp.data.len() {
                            v.pos = None;
                            continue;
                        }
                        let t = (pos - i as f64) as f32;
                        s.0 += smp.data[i * 2] + (smp.data[i * 2 + 2] - smp.data[i * 2]) * t;
                        s.1 += smp.data[i * 2 + 1] + (smp.data[i * 2 + 3] - smp.data[i * 2 + 1]) * t;
                        v.pos = Some(pos + rate_ratio(smp) * v.speed as f64);
                    }
                    buf[n * 2] += s.0 * cmd.gain;
                    buf[n * 2 + 1] += s.1 * cmd.gain;
                }
                // REPEAT UP: CFX PITCH, COLOR 0.5 -> 1 with the knob (both sides), PARAM 0.5.
                let color = if building && up { 0.5 + 0.5 * pct / 100.0 } else { 0.5 };
                e.pitch.process(4, color, 0.5, sb, rate);
                for (o, v) in buf[done * 2..(done + len) * 2].iter_mut().zip(sb.iter()) {
                    *o += v * cmd.gain;
                }
                done += len;
            }
            for v in e.hit.iter_mut() {
                v.wait = v.wait.saturating_sub(frames);
            }
            e.t += frames as u64;
        }
    }
}

/// The interface's side of a unit.
#[derive(Clone, Copy, Default)]
pub struct UnitUi {
    pub on: bool,
    pub knob: f32,
    pub cancel: u32,
    /// The engine's drop counter last seen, and the phase last read (for the panel).
    seen: u32,
    pub phase_on: bool,
}

#[derive(Default)]
pub struct MfxState {
    pub units: [UnitUi; 2],
    /// The samples are being (or have been) read.
    loading: Option<std::sync::mpsc::Receiver<SampleTable>>,
    loaded: bool,
    /// The bottom row of the FX panel shows MERGE FX instead of CFX.
    pub show: bool,
    /// Each deck is playing (read before the rack is locked).
    pub playing: [bool; 4],
    /// Per unit and column (BUILD SAMPLE, DROP SAMPLE): the chosen file being decoded or
    /// handed to the engine.
    user: [[UserFile; 2]; 2],
}

#[derive(Default)]
struct UserFile {
    /// The path asked for, and the one the engine has.
    want: String,
    have: String,
    rx: Option<std::sync::mpsc::Receiver<(String, Option<Arc<Sample>>)>>,
}

/// The file a unit's column should play: the preset's path when its component takes one.
pub fn wanted_path(p: &MfxPrefs, sel: usize, col: usize) -> &str {
    let id = p.comps[sel][if col == 0 { 1 } else { 3 }];
    if path_selectable(id) { &p.paths[sel][col] } else { "" }
}

/// The sample menu (`PresetComponent::showSampleSelectionAt`): the file chosen now (if any),
/// then BANK 1..4 with the sampler's loaded slots (up to 64, 16 a bank) by file name.
pub fn sample_menu(slots: &[Option<crate::ui_sampler::SlotPrefs>]) -> [Vec<(String, String)>; 4] {
    let mut banks: [Vec<(String, String)>; 4] = Default::default();
    for (i, s) in slots.iter().enumerate().take(64) {
        if let Some(s) = s {
            if !s.path.is_empty() {
                let stem = std::path::Path::new(&s.path).file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                banks[i / 16].push((stem, s.path.clone()));
            }
        }
    }
    banks
}

/// A unit is ON (PAD FX is refused meanwhile).
pub fn any_on(app: &crate::app::App) -> bool {
    app.prefs.fx.mfx.enabled && app.px.fxx.mfx.units.iter().any(|u| u.on)
}

/// Switch a unit ON or OFF (`cancel`: Shift + ON, which stops it without release and drop).
pub fn toggle(app: &mut crate::app::App, u: usize, cancel: bool) {
    let s = &mut app.px.fxx.mfx.units[u];
    if cancel {
        s.on = false;
        s.cancel = s.cancel.wrapping_add(1);
    } else {
        s.on = !s.on;
    }
    if !s.on {
        // (rekordbox: the knob springs back to the centre when the unit goes off.)
        s.knob = 0.0;
    }
}

/// Collected before locking the audio rack. After installation these slots retain
/// replaced samples until the interface releases the rack.
#[derive(Default)]
pub struct SampleUpdates {
    factory: Option<SampleTable>,
    user: [[Option<Option<Arc<Sample>>>; 2]; 2],
}

pub fn prepare_samples(app: &mut crate::app::App) -> SampleUpdates {
    let mut updates = SampleUpdates::default();
    let p = &app.prefs.fx.mfx;
    let st = &mut app.px.fxx.mfx;
    if !st.loaded && st.loading.is_none() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // (fx9: the engine's table is made here, off the interface thread and its lock.)
            let _ = tx.send(sample_table(&load_samples()));
        });
        st.loading = Some(rx);
    }
    match st.loading.as_ref().map(|rx| rx.try_recv()) {
        Some(Ok(table)) => {
            updates.factory = Some(table);
            st.loaded = true;
            st.loading = None;
        }
        Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => st.loading = None,
        _ => {}
    }
    for u in 0..2 {
        let sel = p.select[u].min(3) as usize;
        // The chosen files: decoded on a worker thread, handed over when ready.
        for col in 0..2 {
            let f = &mut st.user[u][col];
            let want = wanted_path(p, sel, col);
            if want != f.want {
                f.want = want.to_string();
                f.have.clear();
                updates.user[u][col] = Some(None);
            }
            match f.rx.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok((path, sample))) => {
                    f.rx = None;
                    if path == f.want {
                        f.have = path;
                        updates.user[u][col] = Some(sample);
                    }
                }
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => f.rx = None,
                _ => {}
            }
            // At most one decode per unit/column. If the selection changed while it ran,
            // discard the stale result above, then request only the newest file.
            if f.rx.is_none() && !want.is_empty() && want != f.have {
                let (tx, rx) = std::sync::mpsc::channel();
                let path = want.to_string();
                f.have.clone_from(&path);
                std::thread::spawn(move || {
                    let sample = load_user(&path);
                    let _ = tx.send((path, sample));
                });
                f.rx = Some(rx);
            }
        }
    }
    updates
}

/// With the already prepared rack locked, hand the engine its settings and report
/// decks whose drop has just happened and should stop.
pub fn sync(app: &mut crate::app::App, rack: &mut crate::fx::Rack, bpm: &[f32; crate::fx::CHANNELS], updates: &mut SampleUpdates) -> Vec<usize> {
    let mut stop = Vec::new();
    let p = &app.prefs.fx.mfx;
    let st = &mut app.px.fxx.mfx;
    if !p.enabled {
        if let Some(m) = rack.merge.as_deref_mut() {
            m.stop(st);
        }
        return stop;
    }
    let m = rack.merge.as_deref_mut().expect("MERGE FX engine prepared before settings update");
    if let Some(table) = updates.factory.as_mut() {
        m.swap_samples(table);
    }
    for u in 0..2 {
        let deck = p.deck(u);
        let sel = p.select[u].min(3) as usize;
        let s = st.units[u];
        m.cmd[u] = Cmd { on: s.on, cancel: s.cancel, knob: s.knob, comps: p.comps[sel], deck: deck as u8, bpm: bpm[deck], gain: 10f32.powf(p.volume.clamp(-12.0, 12.0) / 20.0), playing: st.playing[deck] };
        for (col, change) in updates.user[u].iter_mut().enumerate() {
            if let Some(sample) = change {
                *sample = m.set_user(u, col, sample.take());
            }
        }
        let s = &mut st.units[u];
        s.phase_on = m.phase(u) != Phase::Idle;
        if m.dropped[u] != s.seen {
            s.seen = m.dropped[u];
            if p.stop_deck {
                stop.push(deck);
            }
        }
    }
    stop
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaving_performance_cancels_merge_without_a_delayed_deck_stop() {
        let mut engine = Engine::new();
        let mut state = MfxState::default();
        engine.cmd[0].on = true;
        engine.units[0].phase = Phase::Release;
        engine.units[0].gate = 1.0;
        engine.units[0].hit[0].pos = Some(42.0);
        state.units[0].on = true;
        engine.dropped[0] = 7;
        engine.stop(&mut state);
        assert!(!engine.busy());
        assert!(!state.units[0].on);
        assert_eq!(state.units[0].seen, 7);
        let mut next = [0.0; 512];
        engine.master(&mut next, 48000);
        assert_eq!(engine.dropped[0], 7);
        assert_eq!(next, [0.0; 512]);
    }

    #[test]
    fn ids_presets_and_drop_times() {
        assert_eq!((name(1), name(10), name(11), name(22), name(23), name(24), name(28), name(29), name(37), name(38), name(43), name(44)), ("ECHO", "BRAKE DOWN ECHO", "MOBIUS UP SAW", "REPEAT UP SAMPLE", "-", "ECHO OUT", "VINYL BRAKE ECHO", "CRASH CYMBAL", "SHORT BOOM", "NEEDLE SCRATCH NOISE", "SAMPLER", "-"));
        // SNARE RISER = FILTER ECHO + REPEAT SNARE9 + IMPACT SNARE; ECHO DROP = BRAKE DOWN ECHO
        // + VINYL BRAKE ECHO + REWIND.
        assert_eq!(PRESETS[0].1.map(name), ["FILTER ECHO", "REPEAT SNARE9", "IMPACT SNARE", "-"]);
        assert_eq!(PRESETS[7].1.map(name), ["BRAKE DOWN ECHO", "-", "VINYL BRAKE ECHO", "REWIND"]);
        assert_eq!(PRESETS[3].1.map(name), ["SPACE UP", "MOBIUS LFO UP SAW", "UP ECHO OUT", "-"]);
        let d = MfxPrefs::default();
        assert_eq!(d.preset.map(|p| PRESETS[p as usize].0), ["SNARE RISER", "SPACE RISER", "SPACE DROP", "ECHO DROP"]);
        assert!(!d.enabled && d.stop_deck && d.deck(0) == 0 && d.deck(1) == 1);
        assert_eq!((sample_no(15), sample_no(18), sample_no(29), sample_no(37), sample_no(38), sample_no(42), sample_no(43), sample_no(24)), (Some(101), Some(102), Some(201), Some(209), Some(301), Some(305), None, None));
        // Drop times: release effects two beats, VINYL BRAKE ECHO 1 s, samples their lengths.
        assert_eq!((drop_secs(24, 120.0), drop_secs(27, 150.0), drop_secs(28, 120.0), drop_secs(29, 120.0), drop_secs(34, 99.0), drop_secs(37, 120.0), drop_secs(23, 120.0)), (1.0, 0.8, 1.0, 1.8, 3.6, 0.72, 0.0));
        // Every id of a column has a name; settings survive a round trip.
        assert!(COLUMNS.iter().all(|(_, ids)| ids.iter().all(|i| *i == 44 || *i == 23 || name(*i) != "-")));
        let mut p = MfxPrefs::default();
        p.comps[2] = [9, 14, 27, 42];
        p.upper[1] = true;
        p.volume = -6.0;
        let back: MfxPrefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!((back.clone(), back.deck(1)), (p, 3));
        assert_eq!(serde_json::from_str::<MfxPrefs>("{}").unwrap(), MfxPrefs::default());
    }

    #[test]
    fn build_fx_follow_the_knob() {
        assert_eq!((lin(10.0, 0.0, 25.0, 0.0, 0.5), lin(50.0, 0.0, 25.0, 0.0, 0.5), lin(60.0, 30.0, 100.0, 1.0, 0.0) > 0.5), (0.2, 0.5, true));
        // At rest nothing is on.
        for id in 1..=8 {
            let b = build(id, 0.0);
            assert!(b.slots.iter().flatten().all(|s| !s.0.on || id == 6 || id == 7) && b.cfx.iter().all(|c| c.is_none()), "{id}");
        }
        // FILTER ECHO: right the low-pass closes, left the high-pass; the echo is in by a quarter.
        let b = build(2, 0.5);
        assert_eq!(b.cfx[0], Some((crate::cfx::LPF, 0.75, 0.5)));
        assert!(b.cfx[1].is_none() && b.slots[0].unwrap().0.level == 0.5);
        let b = build(2, -1.0);
        assert_eq!(b.cfx[1], Some((crate::cfx::HPF, 1.0, 0.5)));
        // ECHO: left a low-cut echo stepping 1/8 .. 3/4; right a faster and faster one (to 1/64).
        assert_eq!((build(1, -0.1).slots[0].unwrap().0.value, build(1, -0.9).slots[0].unwrap().0.value), (0.125, 0.75));
        let s = build(1, 1.0).slots[1].unwrap();
        assert!((s.0.value - 1.0 / 64.0).abs() < 1e-6 && s.0.level == 0.3 && s.0.knob[0] == 0.9 && !s.1);
        // SPACE UP takes the plain sound away from 30 % up; ZIP DOWN drops the pitch.
        assert!(build(4, 0.2).dry == 1.0 && build(4, 1.0).dry == 0.0);
        assert_eq!(build(8, 0.5).cfx[0], Some((4, 0.0, 0.5)));
    }

    fn engine(comps: [u8; 4]) -> Engine {
        let mut e = Engine::new();
        e.cmd[0].comps = comps;
        // A sample for every file: a second of 0.5.
        let s = Arc::new(Sample { data: vec![0.5; 44100 * 2], rate: 44100.0 });
        e.set_samples(&FILES.map(|f| (f, s.clone())));
        e
    }

    #[test]
    fn sequence_build_release_drop_on_time() {
        // ECHO OUT as the release: two beats (1 s at 120 BPM), then the drop sample.
        let mut e = engine([2, 44, 24, 38]);
        let mut run = |e: &mut Engine, bufs: usize| -> Vec<f32> {
            let mut out = Vec::new();
            for _ in 0..bufs {
                let mut deck = vec![0.1f32; 480 * 2];
                e.deck(0, &mut deck, 48000, None);
                let mut m = vec![0.0f32; 480 * 2];
                e.master(&mut m, 48000);
                out.extend(m);
            }
            out
        };
        assert!(!e.busy());
        e.cmd[0].on = true;
        e.cmd[0].knob = 1.0;
        run(&mut e, 50);
        assert!(e.phase(0) == Phase::Build && e.busy());
        e.cmd[0].on = false;
        let x = run(&mut e, 150);
        assert_eq!(e.dropped[0], 1);
        // The drop sample starts 48,000 frames after OFF (one buffer of delay at most).
        let first = x.chunks_exact(2).position(|f| f[0] > 0.1).unwrap();
        assert!((first as i64 - 48000).abs() <= 480, "{first}");
        // It ends, and the unit comes to rest.
        run(&mut e, 200);
        assert!(e.phase(0) == Phase::Idle && !e.busy());
        // A release sample (IMPACT SNARE, 2.5 s) plays at OFF and the drop comes at its end.
        let mut e = engine([2, 44, 36, 38]);
        e.cmd[0].on = true;
        run(&mut e, 10);
        e.cmd[0].on = false;
        let x = run(&mut e, 10);
        assert!(x[200] > 0.1 && e.phase(0) == Phase::Release);
        run(&mut e, 238);
        assert_eq!(e.dropped[0], 0);
        run(&mut e, 4);
        assert_eq!(e.dropped[0], 1);
        // Cancel: no release, no drop.
        let mut e = engine([2, 15, 24, 38]);
        e.cmd[0].on = true;
        let x = run(&mut e, 100);
        assert!(x.iter().any(|v| v.abs() > 0.1), "the repeating snare is heard while building");
        e.cmd[0].on = false;
        e.cmd[0].cancel = 1;
        let x = run(&mut e, 200);
        assert!(e.dropped[0] == 0 && e.phase(0) == Phase::Idle && x[960..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn build_fx_change_the_deck_and_rest_leaves_it_alone() {
        let tone: Vec<f32> = (0..48000).flat_map(|i| { let v = (i as f32 * 2000.0 * std::f32::consts::TAU / 48000.0).sin() * 0.5; [v, v] }).collect();
        let mut e = engine([2, 44, 24, 44]);
        let mut x = tone.clone();
        e.deck(0, &mut x, 48000, None);
        assert_eq!(x, tone, "idle: the deck is untouched");
        let mut y = tone.clone();
        e.deck(1, &mut y, 48000, None);
        assert_eq!(y, tone, "another deck is never touched");
        // FILTER ECHO fully right: the low-pass at 93 Hz takes a 2 kHz tone away.
        e.cmd[0].on = true;
        e.cmd[0].knob = 1.0;
        let mut m = vec![0.0f32; 960];
        e.master(&mut m, 48000);
        let mut x = tone.clone();
        for c in x.chunks_mut(960) {
            e.deck(0, c, 48000, None);
        }
        let rms = |v: &[f32]| (v.iter().map(|a| a * a).sum::<f32>() / v.len() as f32).sqrt();
        assert!(rms(&x[80000..]) < 0.02, "{}", rms(&x[80000..]));
        assert!(x.iter().all(|v| v.is_finite()));
        // Every BUILD FX at both ends stays bounded.
        for id in 1..=10u8 {
            for k in [-1.0f32, 1.0] {
                let mut e = engine([id, 44, 24, 44]);
                e.cmd[0].on = true;
                e.cmd[0].knob = k;
                let mut x = tone.clone();
                for c in x.chunks_mut(960) {
                    let mut m = vec![0.0f32; 960];
                    e.master(&mut m, 48000);
                    e.deck(0, c, 48000, None);
                }
                let peak = x.iter().fold(0.0f32, |a, b| a.max(b.abs()));
                assert!(peak.is_finite() && peak < 6.0, "{} at {k}: {peak}", name(id));
            }
        }
    }

    /// Frames where a run's left channel jumps up from silence (sample starts).
    fn onsets(x: &[f32]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut prev = 0.0f32;
        for (i, f) in x.chunks_exact(2).enumerate() {
            if f[0] > 0.05 && prev <= 0.05 {
                out.push(i);
            }
            prev = f[0];
        }
        out
    }

    #[test]
    fn repeat_rates_are_rekordboxs_notes_per_beat() {
        // `changeNotesPerBeat`: 1 / 2 / 4 / 8 a beat below 25 / 50 / 75 / from 75 % of |knob|.
        assert_eq!([0.0, 24.9, 25.0, 49.9, 50.0, 74.9, 75.0, 100.0].map(notes_per_beat), [1, 1, 2, 2, 4, 4, 8, 8]);
        // Its 32-step pattern (eighths of a beat over four beats).
        assert_eq!(note_steps(1).iter().filter(|b| **b).count(), 4);
        assert_eq!(note_steps(8), [true; 32]);
        let two = note_steps(2);
        assert!(two[0] && !two[1] && !two[3] && two[4] && two[28]);
        assert_eq!((voice_no(21), voice_no(22), voice_no(43), voice_no(15), voice_no(11)), (Some(1), Some(1), Some(2), Some(101), None));
        assert!(path_selectable(21) && path_selectable(22) && path_selectable(43) && !path_selectable(15) && !path_selectable(38));
        // In the engine: at 120 BPM (24,000 frames a beat at 48 kHz) a short click sample is
        // hit every 24,000 frames at rest, every 3,000 at 80 % (either side).
        for (knob, gap) in [(0.0f32, 24000usize), (0.3, 12000), (-0.6, 6000), (0.8, 3000)] {
            let mut e = Engine::new();
            e.cmd[0].comps = [44, 15, 23, 44];
            let mut click = vec![0.0f32; 200 * 2];
            // (fx7b: 150 frames, longer than the restart's 1 ms fade-in.)
            click[..300].iter_mut().for_each(|v| *v = 0.5);
            e.set_samples(&[(101, Arc::new(Sample { data: click, rate: 48000.0 }))]);
            e.cmd[0].on = true;
            e.cmd[0].knob = knob;
            let mut out = Vec::new();
            for _ in 0..200 {
                let mut d = vec![0.0f32; 480 * 2];
                e.deck(0, &mut d, 48000, None);
                let mut m = vec![0.0f32; 480 * 2];
                e.master(&mut m, 48000);
                out.extend(m);
            }
            let o = onsets(&out);
            // (The knob glides there in the first 30 ms; count the steady part.)
            let steady: Vec<usize> = o.windows(2).filter(|w| w[0] > 2400).map(|w| w[1] - w[0]).collect();
            assert!(!steady.is_empty() && steady.iter().all(|g| g.abs_diff(gap) <= 1), "{knob}: {steady:?}");
        }
    }

    /// fx7b: a repeat restarts the sample through the clip player's `Attenuator` (1 ms
    /// raised-sine): a held tone fades out into each restart and in after it.
    #[test]
    fn repeat_restarts_fade_through_the_clip_players_attenuator() {
        let mut e = Engine::new();
        e.cmd[0].comps = [44, 15, 23, 44];
        // A long constant "sample": the repeats restart it while it still plays. (fx9: handed
        // over as the app does, a table made beforehand and swapped in.)
        let mut t = sample_table(&[(101, Arc::new(Sample { data: vec![0.5f32; 48000 * 2 * 2], rate: 48000.0 }))]);
        e.swap_samples(&mut t);
        assert!(t.iter().all(|s| s.is_none()) && e.samples[0].is_some());
        e.cmd[0].on = true;
        e.cmd[0].knob = 0.0;
        let mut out = Vec::new();
        for _ in 0..200 {
            let mut d = vec![0.0f32; 480 * 2];
            e.deck(0, &mut d, 48000, None);
            let mut m = vec![0.0f32; 480 * 2];
            e.master(&mut m, 48000);
            out.extend(m);
        }
        let l: Vec<f32> = out.chunks_exact(2).map(|c| c[0]).collect();
        // Restarts every 24,000 frames: the level dips to 0 at each and is whole between.
        let dips: Vec<usize> = (1..l.len()).filter(|&i| l[i].abs() < 1e-6 && l[i - 1].abs() > 1e-6).collect();
        assert!(dips.len() >= 2, "{dips:?}");
        let mid = dips[0] + 12000;
        assert!(l[mid].abs() > 0.1 && (l[dips[0] + 24] / l[mid] - 0.5).abs() < 0.1, "{} {}", l[dips[0] + 24], l[mid]);
        assert!((dips[1] - dips[0]).abs_diff(24000) <= 1, "{dips:?}");
        // fx9: steady before it, the fade-out runs from the curve's top to its place 1 on the
        // frame before the restart (`Attenuator::letSmoothDiscontinuousPoints`: from `p - pos`).
        let c = crate::mfx_att::Attenuator::new(48000.0, 1.0);
        let k = l[dips[1] - 1] / l[mid];
        assert!((k - c.curve(1)).abs() < 2e-4 && (l[dips[1] - 2] / l[mid] - c.curve(2)).abs() < 2e-4, "{k} vs {}", c.curve(1));
    }

    #[test]
    fn repeat_grid_restarts_on_a_playing_deck_and_keeps_phase_on_a_stopped_one() {
        for playing in [false, true] {
            let mut e = Engine::new();
            e.cmd[0].comps = [44, 15, 23, 44];
            e.cmd[0].playing = playing;
            let mut click = vec![0.0f32; 200 * 2];
            click[..300].iter_mut().for_each(|v| *v = 0.5);
            e.set_samples(&[(101, Arc::new(Sample { data: click, rate: 48000.0 }))]);
            e.cmd[0].on = true;
            let mut out = Vec::new();
            for b in 0..150 {
                // 1 a beat, then from frame 30,000 (1.25 beats) 2 a beat.
                if b == 62 {
                    e.cmd[0].knob = 0.3;
                }
                let mut d = vec![0.0f32; 480 * 2];
                e.deck(0, &mut d, 48000, None);
                let mut m = vec![0.0f32; 480 * 2];
                e.master(&mut m, 48000);
                out.extend(m);
            }
            let o = onsets(&out);
            // (fx7b: every onset is the fade-in's few frames late: `z`.)
            let z = o[0];
            assert!(z <= 48 && o[1].abs_diff(24000 + z) <= 1, "{o:?}");
            // The knob reaches 25 % about 9 ms after it is turned (frame 29,760 + 432).
            if playing {
                // A note 0.02 beats (480 frames) after the change, then every half beat.
                assert!(o[2].abs_diff(29760 + 432 + 480) <= 480 && (o[3] - o[2]).abs_diff(12000) <= 1, "{o:?}");
            } else {
                // The grid goes on from the start: the next half beat is 36,000.
                assert!(o[2].abs_diff(36000 + z) <= 1 && o[3].abs_diff(48000 + z) <= 1, "{o:?}");
            }
        }
    }

    #[test]
    fn repeat_up_goes_an_octave_up_through_cfx_pitch() {
        // A 440 Hz sample, a second long; REPEAT UP SNARE9 with the knob at the end: CFX PITCH
        // at COLOR 1, PARAM 0.5 doubles it (and the sample's length stays: no speed-up).
        let sine: Vec<f32> = (0..48000).flat_map(|i| { let v = (i as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.4; [v, v] }).collect();
        let level = |x: &[f32], f: f32| {
            let w = f * std::f32::consts::TAU / 48000.0;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, v) in x.chunks_exact(2).enumerate() {
                re += (v[0] * (w * i as f32).cos()) as f64;
                im += (v[0] * (w * i as f32).sin()) as f64;
            }
            (re * re + im * im).sqrt()
        };
        let mut res = Vec::new();
        for id in [15u8, 16] {
            let mut e = Engine::new();
            e.cmd[0].comps = [44, id, 23, 44];
            e.set_samples(&[(101, Arc::new(Sample { data: sine.clone(), rate: 48000.0 }))]);
            e.cmd[0].on = true;
            e.cmd[0].knob = 0.1;
            let mut out = Vec::new();
            for b in 0..40 {
                if b == 1 {
                    e.cmd[0].knob = 0.2;
                }
                let mut d = vec![0.0f32; 480 * 2];
                e.deck(0, &mut d, 48000, None);
                let mut m = vec![0.0f32; 480 * 2];
                e.master(&mut m, 48000);
                out.extend(m);
            }
            // COLOR at 20 %: 0.6 -> shift (2 - 1) * 0.0080645 * (153 - 131) = 0.177.
            let tail = &out[9600 * 2..19200 * 2];
            res.push((level(tail, 440.0), level(tail, 440.0 * 1.177)));
        }
        assert!(res[0].0 > 10.0 * res[0].1, "plain REPEAT stays at 440: {res:?}");
        assert!(res[1].1 > 2.0 * res[1].0 && res[1].0 < 0.1 * res[0].0, "REPEAT UP moves to 518 Hz: {res:?}");
        assert!((crate::cfx::pitch_law(255, 127).unwrap() - 1.0).abs() < 0.01, "an octave at the end");
    }

    #[test]
    fn chosen_files_play_and_missing_ones_are_silent() {
        let mut p = MfxPrefs::default();
        p.comps[0] = [2, 21, 24, 43];
        p.paths[0] = ["/a.wav".into(), "/b.wav".into()];
        assert_eq!((wanted_path(&p, 0, 0), wanted_path(&p, 0, 1)), ("/a.wav", "/b.wav"));
        p.comps[0][1] = 15;
        assert_eq!(wanted_path(&p, 0, 0), "", "a factory component ignores the path");
        assert!(load_user("/no/such/file.wav").is_none() && load_user("").is_none());
        // The engine plays the file handed over for the column.
        let mut e = Engine::new();
        e.cmd[0].comps = [44, 21, 23, 44];
        assert!(e.set_user(0, 0, Some(Arc::new(Sample { data: vec![0.5; 4800 * 2], rate: 48000.0 }))).is_none());
        e.cmd[0].on = true;
        let mut m = vec![0.0f32; 480 * 2];
        e.deck(0, &mut vec![0.0f32; 960], 48000, None);
        e.master(&mut m, 48000);
        assert!(m[100] > 0.4, "{}", m[100]);
        // Without one it is silent.
        let mut e = Engine::new();
        e.cmd[0].comps = [44, 21, 23, 44];
        e.cmd[0].on = true;
        let mut m = vec![0.0f32; 480 * 2];
        e.master(&mut m, 48000);
        assert!(m.iter().all(|v| *v == 0.0));
        // The sample menu: slots by bank, the file's name without extension.
        let mut slots: Vec<Option<crate::ui_sampler::SlotPrefs>> = vec![None; 64];
        let mk = |p: &str| { let mut s: crate::ui_sampler::SlotPrefs = serde_json::from_str("{}").unwrap(); s.path = p.into(); Some(s) };
        slots[3] = mk("/x/Kick.wav");
        slots[17] = mk("/x/Clap 2.aiff");
        slots[63] = mk("/x/Last.mp3");
        let b = sample_menu(&slots);
        assert_eq!((b[0][0].0.as_str(), b[1][0].0.as_str(), b[3][0].1.as_str(), b[2].len()), ("Kick", "Clap 2", "/x/Last.mp3", 0));
    }

    #[test]
    fn factory_sounds_come_from_rekordboxs_folder() {
        if let Some(dir) = factory_dir() { assert!(dir.ends_with("rekordbox/Sampler/MERGE FX")); }
    }

    #[test]
    fn wav_parsing() {
        let mut b = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0\x44\xac\0\0\0\0\0\0\x06\0\x18\0data\x0c\0\0\0".to_vec();
        b.extend([0, 0, 0x40, 0, 0, 0xC0, 0xFF, 0xFF, 0x7F, 0, 0, 0x80]);
        let s = parse_wav(&b, FACTORY_FLOATS).unwrap();
        assert_eq!((s.rate, s.data.len()), (44100.0, 4));
        assert!((s.data[0] - 0.5).abs() < 1e-6 && (s.data[1] + 0.5).abs() < 1e-6 && s.data[2] > 0.999 && s.data[3] == -1.0);
        assert!(parse_wav(b"nope", FACTORY_FLOATS).is_none());
        assert!(parse_wav(&b, 3).is_none());
        assert!(parse_wav(&b[..b.len() - 1], FACTORY_FLOATS).is_none());
    }

    #[test]
    fn local_factory_wav_precedes_zip_and_only_named_sounds_are_read() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let fixture = |value: i16| {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x44\xac\0\0\x88\x58\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            wav.extend(value.to_le_bytes());
            wav.extend(value.to_le_bytes());
            wav
        };
        let zip = temp.path().join("MERGE FX.zip");
        let mut pack = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        for name in ["MergeFX Sample Sound 101.wav", "MergeFX Sample Sound 201.wav", "Unused.wav"] {
            pack.start_file(name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            pack.write_all(&fixture(8192)).unwrap();
        }
        pack.finish().unwrap();
        std::fs::write(temp.path().join("MergeFX Sample Sound 101.wav"), fixture(16384)).unwrap();
        let samples = factory_samples(Some(temp.path()), Some(&zip));
        assert_eq!(samples.len(), 2);
        assert_eq!((samples[0].0, samples[0].1.data[0]), (101, 0.5));
        assert_eq!((samples[1].0, samples[1].1.data[0]), (201, 0.25));
        let wav = fixture(8192);
        assert!(read_factory_wav(&wav[..], MAX_FACTORY_WAV + 1, FACTORY_FLOATS).is_none());
        assert!(read_factory_wav(&wav[..], wav.len() as u64 - 1, FACTORY_FLOATS).is_none());
    }

    #[test]
    fn release_pitch_echoes_shift_by_the_shifters_percent_law() {
        // UP / DOWN ECHO OUT: the preset's pitch 0.529 / 0.456 of -100 .. +100 % through the
        // shifter's percent law (down: 0.005 x pct): ratios 1.058 and 0.956 per repeat.
        let up = 1.0 + crate::fx_core::percent_shift(0.529 * 200.0 - 100.0);
        let down = 1.0 + crate::fx_core::percent_shift(0.456 * 200.0 - 100.0);
        assert!((up - 1.058).abs() < 1e-4 && (down - 0.956).abs() < 1e-4, "{up} {down}");
    }
}