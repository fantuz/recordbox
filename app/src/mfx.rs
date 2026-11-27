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
//! recordbox's own: the effect bodies (see `fx.rs` / `cfx.rs`), the release echoes' pitch as
//! a change of speed, the repeat rates of the REPEAT samples (not in `MergeFX::parameters`;
//! their pattern codes are -1 / 201, which only drive the pitch), and REPEAT SAMPLE / SAMPLER
//! (a file path stored in the preset), which are listed but silent. ROLL's beat steps are the
//! echoes' step table, as the parameter table says.

use crate::cfx::Cfx;
use crate::fx::{Kind, Ring, Slot, SlotParams};
use serde::{Deserialize, Serialize};
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
}

impl Default for MfxPrefs {
    fn default() -> Self {
        // rekordbox's defaults: SNARE RISER, SPACE RISER, SPACE DROP, ECHO DROP.
        let preset = [0u8, 3, 5, 7];
        MfxPrefs { enabled: false, stop_deck: true, preset, comps: preset.map(|p| PRESETS[p as usize].1), select: [0, 0], upper: [false; 2], volume: 0.0 }
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

/// PCM WAV (16 or 24 bit, 1 or 2 channels) to interleaved stereo.
pub fn parse_wav(b: &[u8]) -> Option<Sample> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let (mut pos, mut fmt) = (12usize, None);
    while pos + 8 <= b.len() {
        let len = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = &b[pos + 8..(pos + 8 + len).min(b.len())];
        match &b[pos..pos + 4] {
            b"fmt " if body.len() >= 16 => fmt = Some((u16::from_le_bytes([body[2], body[3]]) as usize, u32::from_le_bytes(body[4..8].try_into().ok()?) as f32, u16::from_le_bytes([body[14], body[15]]) as usize)),
            b"data" => {
                let (ch, rate, bits) = fmt?;
                let bytes = bits / 8;
                if ch == 0 || !(bytes == 2 || bytes == 3) {
                    return None;
                }
                let one = |s: &[u8]| if bytes == 2 { i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0 } else { (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8388608.0 };
                let mut data = Vec::with_capacity(body.len() / (bytes * ch) * 2);
                for f in body.chunks_exact(bytes * ch) {
                    let l = one(&f[0..bytes]);
                    let r = if ch > 1 { one(&f[bytes..2 * bytes]) } else { l };
                    data.push(l);
                    data.push(r);
                }
                return Some(Sample { data, rate });
            }
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    None
}

/// Read rekordbox's sample sounds from its own archive (it is compressed, so the system's
/// `unzip` unpacks each file to memory). Missing rekordbox or files: those sounds are silent.
pub fn load_samples() -> Vec<(u16, Arc<Sample>)> {
    let zip = "/Applications/rekordbox 7/rekordbox.app/Contents/Resources/MERGE FX.zip";
    let mut out = Vec::new();
    if !std::path::Path::new(zip).exists() {
        return out;
    }
    for no in FILES {
        let name = format!("MergeFX Sample Sound {no}.wav");
        if let Ok(o) = std::process::Command::new("/usr/bin/unzip").args(["-p", zip, &name]).output() {
            if let Some(s) = parse_wav(&o.stdout) {
                out.push((no, Arc::new(s)));
            }
        }
    }
    out
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
}

impl UnitEng {
    fn new() -> UnitEng {
        UnitEng { slots: [Slot::new(), Slot::new(), Slot::new()], cfx: [Cfx::new(), Cfx::new()], cfx_last: [(0, 0.5); 2], ring: Ring::new(), phase: Phase::Idle, t: 0, rel_len: 0, comps: [NONE; 4], cancel: 0, knob: 0.0, rel_start: 0, gate: 0.0, brake: (1.0, 0.0), build: Voice::default(), hit: [Voice::default(); 2], osc: [0.0; 6], sweep: 0.0, seed: 0x1357_9BDF, nz: [0.0; 2], clock: 0.0, dry: 1.0 }
    }
}

/// The MERGE FX engine (about 25 MB; made when MERGE FX is first enabled).
pub struct Engine {
    pub cmd: [Cmd; 2],
    units: [UnitEng; 2],
    samples: [Option<Arc<Sample>>; 18],
    /// Counted up each time a unit's drop starts (the interface then stops the deck).
    pub dropped: [u32; 2],
    off: SlotParams,
}

impl Engine {
    pub fn new() -> Engine {
        let cmd = Cmd { on: false, cancel: 0, knob: 0.0, comps: [NONE; 4], deck: 0, bpm: 120.0, gain: 1.0 };
        Engine { cmd: [cmd, Cmd { deck: 1, ..cmd }], units: [UnitEng::new(), UnitEng::new()], samples: Default::default(), dropped: [0; 2], off: SlotParams::new(Kind::Delay) }
    }

    pub fn set_samples(&mut self, list: &[(u16, Arc<Sample>)]) {
        for (no, s) in list {
            if let Some(i) = FILES.iter().position(|f| f == no) {
                self.samples[i] = Some(s.clone());
            }
        }
    }

    pub fn phase(&self, u: usize) -> Phase {
        self.units[u].phase
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
                            // ECHO OUT (0.7 a repeat), UP / DOWN ECHO OUT (0.8, each repeat
                            // 5.8 % faster or 8.8 % slower).
                            let turn = i / len;
                            let (fb, sp) = match rel {
                                25 => (0.8f32, 1.058f32),
                                26 => (0.8, 0.912),
                                _ => (0.7, 1.0),
                            };
                            let pos = ((i % len) as f32 * sp.powi(turn as i32)) as usize % len;
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
                    e.build = Voice { no: sample_no(cmd.comps[1]).unwrap_or(0), pos: None, speed: 1.0, wait: 0 };
                }
                Phase::Build if !cmd.on => {
                    // OFF: the RELEASE FX (or release sample) for its drop time.
                    e.phase = Phase::Release;
                    e.t = 0;
                    e.rel_len = (drop_secs(e.comps[2], cmd.bpm) * r) as u64;
                    e.rel_start = e.ring.w;
                    e.brake = (1.0, 0.0);
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
                if let Some(no) = sample_no(e.comps[3]) {
                    e.hit[1] = Voice { no, pos: Some(0.0), speed: 1.0, wait: wait.min(frames) };
                }
            }
            let bpm = cmd.bpm.clamp(20.0, 1000.0);
            let pct = e.knob.abs() * 100.0;
            // BUILD SAMPLE: the repeating samples are hit again one, two, four, then eight
            // times a beat as the knob rises; the UP ones climb up to an octave with it.
            let id = e.comps[1];
            let building = e.phase == Phase::Build;
            let per_beat = [1.0, 2.0, 4.0, 8.0][((pct / 25.0) as usize).min(3)];
            let up = matches!(id, 16 | 18) as u8 as f32;
            let samples = &self.samples;
            let find = |no: u16| FILES.iter().position(|f| *f == no).and_then(|i| samples[i].as_deref());
            for n in 0..frames {
                let mut s = (0.0f32, 0.0f32);
                if building {
                    let before = e.clock;
                    e.clock += bpm as f64 / 60.0 / r as f64;
                    if (15..=20).contains(&id) && ((before * per_beat).floor() != (e.clock * per_beat).floor() || before == 0.0) {
                        e.build.pos = Some(0.0);
                        e.build.speed = 1.0 + up * pct / 100.0;
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
                for v in std::iter::once(&mut e.build).chain(e.hit.iter_mut()) {
                    if v.wait > n {
                        continue;
                    }
                    let Some(pos) = v.pos else { continue };
                    let Some(smp) = find(v.no) else {
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
                    v.pos = Some(pos + (smp.rate / r * v.speed) as f64);
                }
                buf[n * 2] += s.0 * cmd.gain;
                buf[n * 2 + 1] += s.1 * cmd.gain;
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
    loading: Option<std::sync::mpsc::Receiver<Vec<(u16, Arc<Sample>)>>>,
    loaded: bool,
    /// The bottom row of the FX panel shows MERGE FX instead of CFX.
    pub show: bool,
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

/// Once per frame (from `ui_fx::sync`, with the rack locked): make the engine when MERGE FX
/// is enabled, hand it the units' settings, and report the decks whose drop has just
/// happened and should stop.
pub fn sync(app: &mut crate::app::App, rack: &mut crate::fx::Rack, bpm: &[f32; crate::fx::CHANNELS]) -> Vec<usize> {
    let mut stop = Vec::new();
    let p = app.prefs.fx.mfx.clone();
    let st = &mut app.px.fxx.mfx;
    if !p.enabled {
        if let Some(m) = rack.merge.as_deref_mut() {
            for (c, s) in m.cmd.iter_mut().zip(st.units.iter_mut()) {
                c.on = false;
                s.on = false;
            }
        }
        return stop;
    }
    if rack.merge.is_none() {
        rack.merge = Some(Box::new(Engine::new()));
    }
    if !st.loaded && st.loading.is_none() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(load_samples());
        });
        st.loading = Some(rx);
    }
    let m = rack.merge.as_deref_mut().unwrap();
    if let Some(Ok(list)) = st.loading.as_ref().map(|rx| rx.try_recv()) {
        m.set_samples(&list);
        st.loaded = true;
        st.loading = None;
    }
    for u in 0..2 {
        let deck = p.deck(u);
        let s = &mut st.units[u];
        m.cmd[u] = Cmd { on: s.on, cancel: s.cancel, knob: s.knob, comps: p.comps[p.select[u].min(3) as usize], deck: deck as u8, bpm: bpm[deck], gain: 10f32.powf(p.volume.clamp(-12.0, 12.0) / 20.0) };
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

    #[test]
    fn wav_parsing() {
        let mut b = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0\x44\xac\0\0\0\0\0\0\x06\0\x18\0data\x0c\0\0\0".to_vec();
        b.extend([0, 0, 0x40, 0, 0, 0xC0, 0xFF, 0xFF, 0x7F, 0, 0, 0x80]);
        let s = parse_wav(&b).unwrap();
        assert_eq!((s.rate, s.data.len()), (44100.0, 4));
        assert!((s.data[0] - 0.5).abs() < 1e-6 && (s.data[1] + 0.5).abs() < 1e-6 && s.data[2] > 0.999 && s.data[3] == -1.0);
        assert!(parse_wav(b"nope").is_none());
    }
}
