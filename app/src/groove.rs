//! GROOVE CIRCUIT of PERFORMANCE mode, after rekordbox 7.2.19 (checks/re-7.2.19/
//! pads-sampler-automix.md, section D): per deck two banks of four DRUM SWAP slots, each
//! playing a drum loop in time with its deck; SINGLE / MULTI; the automatic mute of the
//! deck's DRUMS part while a swap plays in SINGLE mode; DRUM RELEASE FX on the drums; DRUM
//! CAPTURE of 4 / 8 / 16 / 32 beats into a slot; presets; the factory kit.
//!
//! The audio side. Each deck has an `Engine` with four rows (rekordbox: four clip tracks,
//! the bank being the clip slot, so slot n of bank A and of bank B share one row and only one
//! of them plays). A row reads its clip with the sampler's `ClipReader`, so a loop keeps its
//! pitch while its speed follows the deck. The deck's audio callback asks for one buffer at a
//! time (`deck_block`) and adds it to the deck's own sound before its EQ and faders.
//!
//! What follows rekordbox: quantized launch on the deck's next beat (its quantize policy for
//! these players is 1/1 beat, set in the `DrumSwapSlot` constructor 0x10084c078), the phase
//! held on the deck's beat modulo one beat (`ClipSynchronizer::doSync` 0x102955f9c compares
//! the phase modulo 1.0 and acts above 0.01 beat), all clips stopped when the deck leaves
//! PLAY or scans (`GrooveCircuit::onPlayModeChanged` 0x1000fdf78), SINGLE stopping the other
//! rows when a clip starts (`GrooveCircuitView::onClipLaunched` 0x100f1ad60), the mute rule
//! (`updateDrumsPartMuteState` 0x1000ff1a0), the gain law, the release FX list, beat table
//! and defaults, the capture range rule and the settings file's keys.
//! What is recordbox's own: the 3 ms fades, the speed nudge that holds the phase between
//! rekordbox's jumps, and the time stretch (rekordbox's stretch mode was not traced).

use crate::app::*;
use crate::audio::{ClipReader, Decoded};
use eframe::egui;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;

pub const DECKS: usize = 4;
pub const ROWS: usize = 4;
pub const BANKS: usize = 2;
/// Slots of one deck: index = row + 4 * bank (the settings file's `idx`).
pub const SLOTS: usize = ROWS * BANKS;

/// DRUM RELEASE FX: the selector's items (ids 1..3; rekordbox's fx ids 29, 30, 31).
pub const RFX_NAMES: [&str; 3] = ["VINYL BRAKE", "ECHO", "BACK SPIN"];
/// `GrooveCircuit::rfxEffectBeats` (0x1056c4ac0): the beat values the arrows step through.
pub const RFX_BEATS: [(u16, u16); 11] = [(1, 16), (1, 8), (1, 4), (1, 2), (3, 4), (1, 1), (2, 1), (4, 1), (8, 1), (16, 1), (32, 1)];
/// The beat of each effect in a new session (constant 0x1037514d0): 1/2, 1/4, 1.
pub const RFX_DEFAULT: [(u16, u16); 3] = [(1, 2), (1, 4), (1, 1)];
/// `DrumCaptureManager::setCaptureLengthNext` 0x10076a934: 4 -> 8 -> 16 -> 32 -> 4.
pub const CAPTURE_LENGTHS: [u32; 4] = [4, 8, 16, 32];
/// The kit that ships with rekordbox (`GrooveCircuitPresetImporter::bundledPresetName`), and
/// the kits it offers as a download.
pub const FACTORY_KIT: &str = "4-Floor / Breaks Kit";
pub const DOWNLOAD_KITS: [&str; 4] = ["4-Floor / Breaks Kit2", "Bass Music Kit", "Hard Groove Kit", "HIP-HOP Reggae Kit"];
/// rekordbox's factory pack, read where rekordbox is installed (never changed, never copied
/// into the app: its samples are unpacked into recordbox's data folder once, at run time).
pub const FACTORY_PACK: &str = "/Applications/rekordbox 7/rekordbox.app/Contents/Resources/GROOVE_CIRCUIT_FACTORY_SAMPLES.spp";
pub const MAX_USER_PRESETS: usize = 10;

/// Fade when a clip stops or is moved (rekordbox's fade length was not traced).
const FADE_SECS: f32 = 0.003;
/// The release FX's on/off smoothing in `fx.rs` (3.33 ms), mirrored here.
const RFX_SMOOTH_SECS: f32 = 0.0033333;

// ------------------------------------------------------------------------------- rules

/// The gain of a GAIN knob at `v` (0..100): rekordbox's trim table raised 9 dB, so 50 is
/// 0 dB and 100 about +9 dB, with a floor at -71 dB (`DrumSwapSlot::setGainOffset`
/// 0x1008561b8: 20 log10(max(T, 1e-4)) + 9). The deck's knob and the slot's knob multiply.
pub fn knob_gain(v: f32) -> f32 {
    crate::mixer::trim_gain((v / 100.0).clamp(0.0, 1.0)).max(1e-4 * 2.8184)
}

/// What a slot's GAIN knob shows: dB with one decimal, "-∞" at the left end
/// (`DecibelSlider::getTextFromValue` 0x10085847c).
pub fn knob_text(v: f32) -> String {
    if v <= 0.0 {
        "-∞".into()
    } else {
        format!("{:.1}", 20.0 * knob_gain(v).log10())
    }
}

/// A release FX beat as its label shows it: "%d" when the denominator is 1, else "%d/%d"
/// (`GrooveCircuitView::onReleaseFxBeatChanged` 0x100f1aae4).
pub fn beat_text(b: (u16, u16)) -> String {
    if b.1 <= 1 { format!("{}", b.0) } else { format!("{}/{}", b.0, b.1) }
}

/// One step down or up the beat table; it stops at both ends.
pub fn beat_step(b: (u16, u16), dir: i32) -> (u16, u16) {
    let i = RFX_BEATS.iter().position(|x| *x == b).unwrap_or(5) as i32;
    RFX_BEATS[(i + dir).clamp(0, RFX_BEATS.len() as i32 - 1) as usize]
}

/// The number of beats in a loop of `frames` at `sr` and `bpm` (at least one).
pub fn loop_beats(frames: usize, sr: u32, bpm: f32) -> u32 {
    if bpm <= 0.0 || sr == 0 {
        return 0;
    }
    ((frames as f64 * bpm as f64 / 60.0 / sr as f64).round() as u32).max(1)
}

/// The part of a track a DRUM CAPTURE takes (`DrumSwapSlot::importClippedWave` 0x100852974):
/// from the playing position `start` (not moved to a beat) for the time `len` beats of the
/// grid take, measured from the first beat after `start`; when the grid ends before that, to
/// the end of the track. Without a grid: `len` beats at 120 BPM. Returns (start, end, BPM of
/// the grid at the start, seconds from `start` to the first beat inside the range).
pub fn capture_range(beats: &[(f64, f64)], start: f64, len: usize, duration: f64) -> (f64, f64, f64, f64) {
    let i = beats.partition_point(|b| b.0 <= start);
    if beats.is_empty() || i >= beats.len() {
        let end = if beats.is_empty() { (start + len as f64 * 0.5).min(duration.max(start)) } else { duration.max(start) };
        return (start, end, beats.last().map(|b| b.1).filter(|b| *b > 0.0).unwrap_or(120.0), 0.0);
    }
    let end = if i + len < beats.len() { start + (beats[i + len].0 - beats[i].0) } else { duration.max(start) };
    let bpm = if beats[i].1 > 0.0 { beats[i].1 } else { 120.0 };
    (start, end.min(duration.max(start)), bpm, beats[i].0 - start)
}

/// Frames of a capture at sample rate `sr`.
pub fn capture_frames(start: f64, end: f64, sr: u32) -> usize {
    ((end * sr as f64).round() - (start * sr as f64).round()).max(0.0) as usize
}

/// rekordbox's automatic mute of the DRUMS part (`GrooveCircuit::updateDrumsPartMuteState`
/// 0x1000ff1a0 and `onPartMuteStateChanged` 0x1000fe2e8).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct AutoMute {
    /// The part is muted because a swap plays in SINGLE mode.
    pub on: bool,
    /// Whether it was muted before that.
    pub was_muted: bool,
}

impl AutoMute {
    /// The launch state or SINGLE / MULTI changed. `multi`: MULTI is on; `launched`: a slot
    /// of this deck plays; `muted`: the DRUMS part is muted now. Returns the mute state to
    /// set, if it has to change.
    pub fn update(&mut self, multi: bool, launched: bool, muted: bool) -> Option<bool> {
        if multi || !launched {
            if self.on {
                self.on = false;
                return Some(self.was_muted);
            }
        } else if !self.on {
            self.on = true;
            self.was_muted = muted;
            return Some(true);
        }
        None
    }
    /// The part's mute state as it is now: un-muted by hand while it was muted
    /// automatically, the automatic mute is forgotten and nothing is restored later.
    pub fn part_changed(&mut self, muted: bool) {
        if self.on && !muted {
            self.on = false;
        }
    }
}

// ------------------------------------------------------------------------------- engine

/// One loaded slot, for the audio thread.
#[derive(Clone)]
pub struct GcClip {
    pub audio: Arc<Decoded>,
    /// The loop's own tempo (0: unknown, it then plays at its own speed and is not synced).
    pub bpm: f32,
    /// The slot's GAIN knob as a gain.
    pub gain: f32,
    /// Seconds from the clip's start to its first beat.
    pub first_beat: f32,
}

impl GcClip {
    /// (loop length in frames, frames per beat or 0, the place in the loop, in beats, where
    /// a start on a beat begins).
    fn geometry(&self) -> (f64, f64, f64) {
        let frames = self.audio.frames() as f64;
        if self.bpm <= 0.0 {
            return ((frames - 1.0).max(1.0), 0.0, 0.0);
        }
        let fpb = self.audio.sr as f64 * 60.0 / self.bpm as f64;
        let beats = (frames / fpb).round().max(1.0);
        let len = (beats * fpb).min(frames).max(1.0);
        // The nearest place whose phase is zero: just after the start, or just before the end.
        let fb = (self.first_beat.max(0.0) as f64 * self.audio.sr as f64 / fpb).rem_euclid(1.0);
        let p0 = if fb <= 0.5 { fb } else { beats - 1.0 + fb };
        (len, fpb, p0)
    }
}

struct Row {
    clips: [Option<GcClip>; BANKS],
    bank: usize,
    playing: bool,
    pos: f64,
    level: f32,
    target: f32,
    /// A launch waiting for the deck's next beat: (bank, the beat; infinite until the audio
    /// thread has looked).
    pending: Option<(usize, f64)>,
    /// A launch waiting for the fade-out of what plays: (bank, the beat it belongs to).
    restart: Option<(usize, f64)>,
    /// The fade-out in progress ends in a move back onto the deck's beat, not in a stop.
    resync: bool,
    /// The deck's beat the clip started on (not finite: it is not held on the deck).
    start_beat: f64,
    /// Buffers in a row the phase was found off.
    off: u8,
    nudge: f64,
    reader: ClipReader,
    peak: f32,
}

impl Row {
    fn new() -> Row {
        Row { clips: [None, None], bank: 0, playing: false, pos: 0.0, level: 0.0, target: 0.0, pending: None, restart: None, resync: false, start_beat: f64::NAN, off: 0, nudge: 0.0, reader: ClipReader::new(), peak: 0.0 }
    }

    fn fade_out(&mut self) {
        if self.playing {
            self.target = 0.0;
            self.restart = None;
            self.resync = false;
        }
    }

    /// Start `bank` for the deck beat `at`, the deck being at beat `now`.
    fn start(&mut self, bank: usize, at: f64, now: f64) {
        let Some(c) = self.clips[bank].as_ref() else { return };
        let (len, fpb, p0) = c.geometry();
        self.bank = bank;
        self.playing = true;
        self.level = 1.0;
        self.target = 1.0;
        self.restart = None;
        self.resync = false;
        self.off = 0;
        self.nudge = 0.0;
        let held = fpb > 0.0 && at.is_finite() && now.is_finite();
        self.start_beat = if held { at } else { f64::NAN };
        self.pos = if held { ((p0 + (now - at).max(0.0)) * fpb).rem_euclid(len) } else { 0.0 };
        self.reader.reset();
    }

    /// As `start`, after the fade-out of what the row plays now.
    fn begin(&mut self, bank: usize, at: f64, now: f64) {
        if self.playing {
            self.restart = Some((bank, at));
            self.resync = false;
            self.target = 0.0;
        } else {
            self.start(bank, at, now);
        }
    }
}

/// What the audio callback knows of the deck for the buffer it is making.
pub struct DeckIn<'a> {
    /// The deck plays (and is not scanning).
    pub playing: bool,
    /// Position in source frames, the file's sample rate, and source frames per output frame.
    pub pos: f64,
    pub sr: u32,
    pub step: f64,
    /// Beat times of the track (seconds).
    pub grid: Option<&'a [f64]>,
    pub stems: Option<&'a crate::stems::Stems>,
    /// The DRUMS part as the deck has it (0 when muted).
    pub drums_gain: f32,
}

/// What the panel reads of a row.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct RowState {
    /// The bank that plays, the bank whose launch waits for the beat, and how far through
    /// the loop playback is (0..1).
    pub playing: Option<usize>,
    pub pending: Option<usize>,
    pub progress: f32,
}

pub struct Engine {
    rows: [Row; ROWS],
    deck: usize,
    /// DRUM SWAP GAIN of the deck, as a gain.
    pub gain: f32,
    /// SINGLE: a clip that starts stops the other rows.
    pub single: bool,
    /// The DRUMS part is to stay muted while a swap plays in SINGLE mode (dropped when the
    /// part is un-muted by hand).
    pub hold_drums: bool,
    /// DRUM RELEASE FX held: which one (0 brake, 1 echo, 2 back spin) and its beats.
    pub release: Option<(u8, f32)>,
    /// The deck's tempo, for the release FX's time.
    pub bpm: f32,
    /// For the scripted run: everything works, nothing is heard.
    pub silent: bool,
    mix: Vec<f32>,
    bus: Vec<f32>,
    dry: Vec<f32>,
    /// The release FX of `fx.rs` on this deck's drums (made by the interface when the panel
    /// is first shown: about 12 MB of history and delay lines).
    rfx: Option<Box<crate::fx::Rack>>,
    wet: f32,
    /// The deck must leave its DRUMS part out of this buffer.
    pub cut: bool,
    /// The last buffer held sound of the slots or of the release FX.
    pub sounding: bool,
    /// The deck was playing in the last buffer (clips stop when it leaves PLAY).
    was_playing: bool,
}

impl Engine {
    pub fn new(deck: usize) -> Engine {
        Engine { rows: std::array::from_fn(|_| Row::new()), deck: deck.min(DECKS - 1), gain: 1.0, single: true, hold_drums: true, release: None, bpm: 120.0, silent: false, mix: vec![0.0; 16384], bus: vec![0.0; 16384], dry: vec![0.0; 16384], rfx: None, wet: 0.0, cut: false, sounding: false, was_playing: false }
    }

    /// Make the release FX unit (not on the audio thread: it allocates).
    pub fn prepare_fx(&mut self) {
        if self.rfx.is_some() {
            return;
        }
        let mut rack = Box::new(crate::fx::Rack::new());
        // (This rack is the panel's own: its unit always runs.)
        rack.enabled = true;
        let mut assign = [false; crate::fx::CHANNELS];
        assign[self.deck] = true;
        rack.units[0].params.assign = assign;
        rack.units[0].prepare(&assign);
        self.rfx = Some(rack);
    }

    /// Put a clip in a slot (or take it out); what the slot was playing stops at once.
    pub fn set_clip(&mut self, row: usize, bank: usize, clip: Option<GcClip>) {
        let r = &mut self.rows[row % ROWS];
        let bank = bank % BANKS;
        if r.bank == bank {
            r.playing = false;
            r.restart = None;
            r.resync = false;
        }
        if matches!(r.pending, Some((b, _)) if b == bank) {
            r.pending = None;
        }
        if matches!(r.restart, Some((b, _)) if b == bank) {
            r.restart = None;
        }
        r.clips[bank] = clip;
    }

    pub fn clip_mut(&mut self, row: usize, bank: usize) -> Option<&mut GcClip> {
        self.rows[row % ROWS].clips[bank % BANKS].as_mut()
    }

    /// Play pressed: the clip starts on the deck's next beat. In SINGLE mode a launch still
    /// waiting on another row is dropped (the rows that play stop when this one starts).
    pub fn launch(&mut self, row: usize, bank: usize) -> bool {
        let (row, bank) = (row % ROWS, bank % BANKS);
        if self.rows[row].clips[bank].is_none() {
            return false;
        }
        if self.single {
            for (i, r) in self.rows.iter_mut().enumerate() {
                if i != row {
                    r.pending = None;
                }
            }
        }
        self.rows[row].pending = Some((bank, f64::INFINITY));
        WANTS[self.deck].store(true, Relaxed);
        true
    }

    /// Stop a row (with a short fade); a launch still waiting is dropped.
    pub fn stop(&mut self, row: usize) {
        let r = &mut self.rows[row % ROWS];
        r.pending = None;
        r.fade_out();
    }

    pub fn stop_all(&mut self) {
        for r in self.rows.iter_mut() {
            r.pending = None;
            r.fade_out();
        }
    }

    /// A slot plays, or its launch waits for the beat (rekordbox's "launched").
    pub fn is_played(&self, row: usize, bank: usize) -> bool {
        let r = &self.rows[row % ROWS];
        let bank = bank % BANKS;
        (r.playing && r.bank == bank && r.target > 0.0) || matches!(r.restart, Some((b, _)) if b == bank) || matches!(r.pending, Some((b, _)) if b == bank)
    }

    /// A slot of the deck sounds (the DRUMS mute rule's "launched": a launch that still
    /// waits for its beat does not count, so the deck's drums play up to the swap's start).
    pub fn any_sounding(&self) -> bool {
        self.rows.iter().any(|r| r.playing && (r.target > 0.0 || r.restart.is_some()))
    }

    pub fn any_played(&self) -> bool {
        (0..ROWS).any(|r| (0..BANKS).any(|b| self.is_played(r, b)))
    }

    pub fn row_state(&self, row: usize) -> RowState {
        let r = &self.rows[row % ROWS];
        let playing = if r.playing && (r.target > 0.0 || r.resync) { Some(r.bank) } else { None };
        let progress = match (playing, r.clips[r.bank].as_ref()) {
            (Some(_), Some(c)) => (r.pos / c.geometry().0).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        };
        RowState { playing, pending: r.pending.map(|p| p.0).or(r.restart.map(|p| p.0)), progress }
    }

    pub fn take_peak(&mut self, row: usize) -> f32 {
        std::mem::take(&mut self.rows[row % ROWS].peak)
    }

    /// The release FX is heard (held, or still fading back).
    pub fn fx_active(&self) -> bool {
        self.release.is_some() || self.wet > 0.0
    }

    /// One buffer of `n` frames at `rate`: the slots' sound, and while the release FX is in
    /// use the deck's drums through it. Afterwards `out(i)` gives frame i, and `cut` says
    /// whether the deck has to leave its own drums out. Returns whether there is anything
    /// to add.
    pub fn render(&mut self, d: &DeckIn, n: usize, rate: u32) -> bool {
        if n == 0 {
            return false;
        }
        if self.mix.len() < n * 2 {
            // (Only when the device asks for a larger buffer than was foreseen.)
            self.mix.resize(n * 2, 0.0);
            self.bus.resize(n * 2, 0.0);
            self.dry.resize(n * 2, 0.0);
        }
        let rate_f = rate.max(1) as f64;
        // Where the deck is on its grid at the start and the end of the buffer.
        let (b0, bpf) = match d.grid.filter(|_| d.playing) {
            Some(g) => match (crate::audio::beat_pos(g, d.pos / d.sr.max(1) as f64), crate::audio::beat_pos(g, (d.pos + n as f64 * d.step) / d.sr.max(1) as f64)) {
                (Some(a), Some(b)) if b > a && b - a < 64.0 => (a, (b - a) / n as f64),
                _ => (f64::NAN, 0.0),
            },
            None => (f64::NAN, 0.0),
        };
        let synced = bpf > 0.0;
        // The deck left PLAY: every clip stops (`GrooveCircuit::onPlayModeChanged`). A slot
        // launched while the deck stands plays at once and unsynced, at its own tempo
        // (`launchWithQuantize` with the master not playing); only a change to a mode other
        // than PLAY stops it, so it goes on (unsynced) when the deck starts.
        if self.was_playing && !d.playing {
            self.stop_all();
        }
        self.was_playing = d.playing;
        // Launches whose beat falls inside this buffer: (frame, bank, beat).
        let mut starts: [Option<(usize, usize, f64)>; ROWS] = [None; ROWS];
        for (i, r) in self.rows.iter_mut().enumerate() {
            let Some((bank, at)) = r.pending else { continue };
            if !synced || r.clips[bank].as_ref().is_none_or(|c| c.bpm <= 0.0) {
                // Nothing to wait for (no grid on the deck or on the clip): it starts now.
                starts[i] = Some((0, bank, f64::NAN));
                r.pending = None;
                continue;
            }
            // The next beat from here (again when the deck has been moved since).
            let at = if at > b0 + 1.0 + 1e-6 || at < b0 - 1e-6 { b0.ceil() } else { at };
            if at < b0 + bpf * n as f64 {
                starts[i] = Some(((((at - b0) / bpf).max(0.0) as usize).min(n - 1), bank, at));
                r.pending = None;
            } else {
                r.pending = Some((bank, at));
            }
        }
        let single_stop = if self.single { starts.iter().enumerate().find_map(|(i, s)| s.map(|s| (i, s.0))) } else { None };
        let fade_k = 1.0 / (FADE_SECS * rate as f32);
        let hop = ((rate as usize) / 50).clamp(64, 4096);
        let deck_gain = self.gain;
        let mix = &mut self.mix[..n * 2];
        mix.iter_mut().for_each(|v| *v = 0.0);
        let mut any = false;
        for (ri, r) in self.rows.iter_mut().enumerate() {
            let start = starts[ri];
            let stop_at = match single_stop {
                Some((row, f)) if row != ri => Some(f),
                _ => None,
            };
            if !r.playing && start.is_none() {
                continue;
            }
            any = true;
            // The phase against the deck's beat, once per buffer: a small error is pulled in
            // with the speed; a large one (the deck was moved) ends in a move after a fade.
            if let (true, Some(c)) = (r.playing && synced && r.start_beat.is_finite() && r.target > 0.0, r.clips[r.bank].as_ref()) {
                let (_, fpb, p0) = c.geometry();
                if fpb > 0.0 {
                    let e = phase_error(r.pos / fpb - p0, b0 - r.start_beat);
                    if e.abs() > 0.06 {
                        r.off += 1;
                        if r.off >= 2 {
                            r.off = 0;
                            r.resync = true;
                            r.target = 0.0;
                        }
                    } else {
                        r.off = 0;
                    }
                    r.nudge = if e.abs() > 0.002 && e.abs() <= 0.06 { (-e * 0.5).clamp(-0.03, 0.03) } else { 0.0 };
                }
            }
            let mut i = 0usize;
            while i < n {
                let now = b0 + bpf * i as f64;
                if let Some((f, bank, at)) = start {
                    if i == f {
                        r.begin(bank, at, now);
                    }
                }
                if stop_at == Some(i) {
                    r.pending = None;
                    r.fade_out();
                }
                if !r.playing {
                    match start {
                        Some((f, _, _)) if f > i => {
                            i = f;
                            continue;
                        }
                        _ => break,
                    }
                }
                let Row { clips, bank, playing, pos, level, target, restart, resync, start_beat, nudge, reader, peak, .. } = r;
                let Some(c) = clips[*bank].as_ref() else {
                    *playing = false;
                    continue;
                };
                if c.audio.frames() < 2 {
                    *playing = false;
                    continue;
                }
                let (len, fpb, p0) = c.geometry();
                let ratio = c.audio.sr as f64 / rate_f;
                let held = synced && fpb > 0.0 && start_beat.is_finite();
                let step = if held { bpf * fpb * (1.0 + *nudge) } else { ratio };
                let (l, rr) = reader.frame(&c.audio, *pos, step, ratio, hop);
                let g = c.gain * deck_gain * *level;
                let (l, rr) = (l * g, rr * g);
                mix[i * 2] += l;
                mix[i * 2 + 1] += rr;
                *peak = peak.max(l.abs()).max(rr.abs());
                *pos += step;
                if *pos >= len {
                    *pos -= len;
                }
                if *level != *target {
                    *level += (*target - *level).clamp(-fade_k, fade_k);
                    if *level <= 0.0 && *target <= 0.0 {
                        if let Some((b, at)) = restart.take() {
                            *playing = false;
                            r.start(b, at, now);
                        } else if *resync && held {
                            // Back onto the deck's beat: the fraction of a beat it is off by.
                            let e = phase_error(*pos / fpb - p0, now - *start_beat);
                            *pos = (*pos - e * fpb).rem_euclid(len);
                            *resync = false;
                            *target = 1.0;
                            reader.reset();
                        } else {
                            *playing = false;
                            *resync = false;
                            *pos = 0.0;
                        }
                    }
                }
                i += 1;
            }
        }
        // ---- the drums bus and the release FX
        let playing_any = self.rows.iter().any(|r| r.playing && (r.target > 0.0 || r.restart.is_some()));
        let auto = self.single && self.hold_drums && playing_any;
        let held = self.release.filter(|_| d.playing);
        self.cut = held.is_some() || auto;
        let mut wet_out = false;
        if let (Some(rack), true) = (self.rfx.as_deref_mut(), d.playing || self.wet > 0.0) {
            let (dry, bus) = (&mut self.dry[..n * 2], &mut self.bus[..n * 2]);
            let dg = if auto { 0.0 } else { d.drums_gain };
            match d.stems.filter(|st| dg > 0.0 && st.len > 2 && d.playing) {
                Some(st) => {
                    // The deck's DRUMS part where the deck is (the part is kept at 44.1 kHz).
                    let k = crate::stems::htdemucs::SAMPLE_RATE as f64 / d.sr.max(1) as f64;
                    let (lb, rb) = (0usize, st.len);
                    for i in 0..n {
                        let sp = ((d.pos + i as f64 * d.step) * k).max(0.0);
                        let si = (sp as usize).min(st.len - 2);
                        let f = (sp - si as f64).min(1.0) as f32;
                        dry[i * 2] = (st.data[lb + si] + (st.data[lb + si + 1] - st.data[lb + si]) * f) * dg;
                        dry[i * 2 + 1] = (st.data[rb + si] + (st.data[rb + si + 1] - st.data[rb + si]) * f) * dg;
                    }
                }
                None => dry.iter_mut().for_each(|v| *v = 0.0),
            }
            for ((b, x), m) in bus.iter_mut().zip(dry.iter()).zip(mix.iter()) {
                *b = *x + *m;
            }
            let p = &mut rack.units[0].params;
            p.bpm[self.deck] = self.bpm.clamp(20.0, 1000.0);
            p.release = held;
            rack.process(self.deck, bus, rate);
            if held.is_some() || self.wet > 0.0 {
                // What the deck no longer plays itself comes from here: the effect's sound,
                // and the dry drums for as long as the deck's own are still fading.
                let k = 1.0 / (RFX_SMOOTH_SECS * rate as f32);
                for i in 0..n {
                    self.wet = if held.is_some() { (self.wet + k).min(1.0) } else { (self.wet - k).max(0.0) };
                    let dry_part = 1.0 - self.wet;
                    mix[i * 2] = bus[i * 2] - dry[i * 2] * dry_part;
                    mix[i * 2 + 1] = bus[i * 2 + 1] - dry[i * 2 + 1] * dry_part;
                }
                wet_out = true;
            }
        } else if held.is_none() {
            self.wet = 0.0;
        }
        if self.silent {
            mix.iter_mut().for_each(|v| *v = 0.0);
        }
        self.sounding = any || wet_out;
        WANTS[self.deck].store(self.sounding || self.rows.iter().any(|r| r.playing || r.pending.is_some()), Relaxed);
        self.sounding
    }

    /// Frame `i` of the last buffer.
    #[inline]
    pub fn out(&self, i: usize) -> (f32, f32) {
        match self.mix.get(i * 2..i * 2 + 2) {
            Some(s) => (s[0], s[1]),
            None => (0.0, 0.0),
        }
    }
}

/// How far a clip is from where the deck's beat wants it, in beats, modulo one beat
/// (-0.5..0.5; positive: the clip is ahead). `have`: beats the clip has played since a
/// beat-aligned start; `want`: beats the deck has moved since then.
pub fn phase_error(have: f64, want: f64) -> f64 {
    (have - want + 0.5).rem_euclid(1.0) - 0.5
}

/// The decks' engines (made by the interface when the panel is first shown on a deck).
pub static ENGINES: [Mutex<Option<Box<Engine>>>; DECKS] = [Mutex::new(None), Mutex::new(None), Mutex::new(None), Mutex::new(None)];
/// An engine exists and has something to do (else the audio callback does not even lock).
pub static ACTIVE: [AtomicBool; DECKS] = [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)];

/// A slot of the deck plays or waits to start: the deck has to render while it stands
/// (set by the engine after each buffer and by a launch).
pub static WANTS: [AtomicBool; DECKS] = [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)];

/// The audio callback asks whether a standing deck still has Groove Circuit sound to make.
#[inline]
pub fn wants_render(s: &crate::audio::PlayState) -> bool {
    let d = (s.deck_no as usize).min(DECKS - 1);
    ACTIVE[d].load(Relaxed) && WANTS[d].load(Relaxed)
}

/// One buffer of a deck's Groove Circuit, held by the audio callback while it mixes it in.
pub struct Block {
    g: parking_lot::MutexGuard<'static, Option<Box<Engine>>>,
}

impl Block {
    #[inline]
    pub fn at(&self, i: usize) -> (f32, f32) {
        match self.g.as_deref() {
            Some(e) => e.out(i),
            None => (0.0, 0.0),
        }
    }
}

/// The audio callback's side: make the deck's Groove Circuit buffer for the `n` frames it is
/// about to render at `speed`, and take the DRUMS part out of `stem_target` (the gains of the
/// original and the four parts the deck is heading for) while a swap or the release FX
/// replaces it. `None`: nothing to add.
pub fn deck_block(s: &crate::audio::PlayState, audio: &Decoded, speed: f64, stem_target: &mut [f32; 5], n: usize, rate: u32) -> Option<Block> {
    let deck = (s.deck_no as usize).min(DECKS - 1);
    if !ACTIVE[deck].load(Relaxed) {
        return None;
    }
    // (The interface holds an engine only to read or set a few values: waited for briefly,
    // as the decks' own state is.)
    let mut g = ENGINES[deck].try_lock().or_else(|| {
        let t = std::time::Instant::now();
        loop {
            for _ in 0..64 {
                std::hint::spin_loop();
            }
            if let Some(g) = ENGINES[deck].try_lock() {
                return Some(g);
            }
            if t.elapsed() > std::time::Duration::from_micros(250) {
                return None;
            }
        }
    })?;
    let e = g.as_deref_mut()?;
    let d = DeckIn {
        playing: s.playing && s.search == 0.0,
        pos: s.pos,
        sr: audio.sr,
        step: audio.sr as f64 / rate.max(1) as f64 * speed,
        grid: s.grid.as_deref().map(|v| v.as_slice()),
        stems: s.stems.as_deref(),
        drums_gain: if s.stem_on[0] { s.stem_iso[0].clamp(0.0, 10.0) } else { 0.0 },
    };
    let some = e.render(&d, n, rate);
    if e.cut && s.stems.is_some() {
        // The other parts as the deck has them, without the drums.
        stem_target[0] = 0.0;
        stem_target[1] = 0.0;
        for i in 1..4 {
            stem_target[1 + i] = if s.stem_on[i] { s.stem_iso[i].clamp(0.0, 10.0) } else { 0.0 };
        }
    }
    if some { Some(Block { g }) } else { None }
}

// ------------------------------------------------------------------------------- settings

/// Preferences > Extensions > STEMS, the Groove Circuit rows (rekordbox's setting keys
/// `GrooveCircuitEnable`, `ShowPanel_GROOVE_CIRCUIT`, `DrumSwapSlotLoadLock`,
/// `ShowTitleToDrumSwapSlot`, `DrumCaptureStem`, `DrumCaptureSaveDirectory`,
/// `DrumAutomaticallyImport`, `HasBeenLoadedGrooveCircuitBundledSamples`). rekordbox has the
/// function off until a plan or a controller unlocks it; here it is simply available, and
/// the panel stays hidden until its button in the top bar is pressed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct GroovePrefs {
    pub enable: bool,
    pub panel: bool,
    pub load_lock: bool,
    pub show_title: bool,
    /// 1 = DRUMS Stem, 2 = Original track.
    pub capture_stem: u8,
    /// Empty: the sampler's capture folder in recordbox's data folder.
    pub capture_dir: String,
    pub auto_import: bool,
    pub bundled_loaded: bool,
}

impl Default for GroovePrefs {
    fn default() -> GroovePrefs {
        GroovePrefs { enable: true, panel: false, load_lock: true, show_title: true, capture_stem: 1, capture_dir: String::new(), auto_import: true, bundled_loaded: false }
    }
}

/// One slot as `GrooveCircuitSettings.xml` keeps it: a `Track` element with `idx id name url
/// length samplingrate gain` (`GrooveCircuitPropertiesFile::setTrack` 0x100f70898).
/// recordbox adds `bpm` and `firstbeat`, because a slot's file need not be in its collection
/// (rekordbox reads both from its database).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SlotInfo {
    /// The collection's track id (0: a file that is not in the collection).
    pub id: i64,
    pub name: String,
    pub url: String,
    /// Length in frames and sample rate (filled in when the file has been read).
    pub length: i64,
    pub samplingrate: u32,
    /// The slot's GAIN knob, 0..100.
    pub gain: f32,
    pub bpm: f32,
    pub first_beat: f32,
}

/// One slot of a preset: rekordbox keeps `idx id`; recordbox also the file and its tempo.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PresetTrack {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub bpm: f32,
    pub first_beat: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Preset {
    /// `Time::currentTimeMillis` when it was made.
    pub id: i64,
    pub name: String,
    pub tracks: [Option<PresetTrack>; SLOTS],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeckSet {
    pub slots: [Option<SlotInfo>; SLOTS],
    /// `visibleBank<n>`: 0 = A, 1 = B.
    pub bank: u8,
    /// `singleMulti<n>`: false = SINGLE.
    pub multi: bool,
    /// `fxType<n>`: 0 vinyl brake, 1 echo, 2 back spin; `FxSet<n>`: each one's beat.
    pub fx_type: u8,
    pub fx_beats: [(u16, u16); 3],
}

impl Default for DeckSet {
    fn default() -> DeckSet {
        DeckSet { slots: Default::default(), bank: 0, multi: false, fx_type: 0, fx_beats: RFX_DEFAULT }
    }
}

/// `GrooveCircuitSettings.xml` (rekordbox: a JUCE PropertiesFile in its own settings folder;
/// recordbox: the same shape in recordbox's data folder).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Settings {
    pub decks: [DeckSet; DECKS],
    pub presets: Vec<Preset>,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

impl Settings {
    pub fn path() -> std::path::PathBuf {
        crate::db::data_dir().join("GrooveCircuitSettings.xml")
    }

    pub fn to_xml(&self) -> String {
        let mut o = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n<PROPERTIES>\n");
        for (i, d) in self.decks.iter().enumerate() {
            let n = i + 1;
            o += &format!("  <VALUE name=\"visibleBank{n}\" val=\"{}\"/>\n", d.bank.min(1));
            o += &format!("  <VALUE name=\"singleMulti{n}\" val=\"{}\"/>\n", d.multi as u8);
            o += &format!("  <VALUE name=\"fxType{n}\" val=\"{}\"/>\n", d.fx_type.min(2));
            o += &format!("  <VALUE name=\"FxSet{n}\">\n    <FxSet{n}>\n");
            for (k, b) in d.fx_beats.iter().enumerate() {
                o += &format!("      <Beat idx=\"{k}\" numerator=\"{}\" denominator=\"{}\"/>\n", b.0, b.1);
            }
            o += &format!("    </FxSet{n}>\n  </VALUE>\n");
            o += &format!("  <VALUE name=\"SamplerSet{n}\">\n    <SamplerSet{n}>\n");
            for (idx, s) in d.slots.iter().enumerate() {
                if let Some(s) = s {
                    o += &format!(
                        "      <Track idx=\"{idx}\" id=\"{}\" name=\"{}\" url=\"{}\" length=\"{}\" samplingrate=\"{}\" gain=\"{}\" bpm=\"{}\" firstbeat=\"{}\"/>\n",
                        s.id,
                        esc(&s.name),
                        esc(&s.url),
                        s.length,
                        s.samplingrate,
                        s.gain,
                        s.bpm,
                        s.first_beat
                    );
                }
            }
            o += &format!("    </SamplerSet{n}>\n  </VALUE>\n");
        }
        o += "  <VALUE name=\"Presets\">\n    <Presets>\n";
        for p in &self.presets {
            o += &format!("      <Preset id=\"{}\" name=\"{}\">\n", p.id, esc(&p.name));
            for (idx, t) in p.tracks.iter().enumerate() {
                match t {
                    Some(t) => o += &format!("        <Track idx=\"{idx}\" id=\"{}\" name=\"{}\" url=\"{}\" bpm=\"{}\" firstbeat=\"{}\"/>\n", t.id, esc(&t.name), esc(&t.url), t.bpm, t.first_beat),
                    // (rekordbox writes an empty slot as id 0.)
                    None => o += &format!("        <Track idx=\"{idx}\" id=\"0\"/>\n"),
                }
            }
            o += "      </Preset>\n";
        }
        o += "    </Presets>\n  </VALUE>\n</PROPERTIES>\n";
        o
    }

    pub fn from_xml(xml: &str) -> Settings {
        use quick_xml::events::Event;
        let mut out = Settings::default();
        let mut rd = quick_xml::Reader::from_str(xml);
        // The VALUE being read: its name; and the preset being read.
        let mut value = String::new();
        let mut preset: Option<Preset> = None;
        let deck_of = |name: &str, key: &str| name.strip_prefix(key).and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=DECKS).contains(n)).map(|n| n - 1);
        loop {
            let ev = match rd.read_event() {
                Ok(Event::Eof) | Err(_) => break,
                Ok(e) => e,
            };
            let (e, empty) = match &ev {
                Event::Start(e) => (e, false),
                Event::Empty(e) => (e, true),
                Event::End(e) => {
                    match e.name().as_ref() {
                        b"VALUE" => value.clear(),
                        b"Preset" => {
                            if let Some(p) = preset.take() {
                                out.presets.push(p);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                _ => continue,
            };
            let attr = |key: &str| -> Option<String> { e.attributes().flatten().find(|a| a.key.as_ref() == key.as_bytes()).and_then(|a| a.unescape_value().ok().map(|v| v.to_string())) };
            let num = |key: &str| attr(key).and_then(|v| v.parse::<f64>().ok());
            match e.name().as_ref() {
                b"VALUE" => {
                    let name = attr("name").unwrap_or_default();
                    if let Some(v) = attr("val") {
                        let iv = v.parse::<i64>().unwrap_or(0);
                        if let Some(d) = deck_of(&name, "visibleBank") {
                            out.decks[d].bank = iv.clamp(0, 1) as u8;
                        } else if let Some(d) = deck_of(&name, "singleMulti") {
                            out.decks[d].multi = iv != 0;
                        } else if let Some(d) = deck_of(&name, "fxType") {
                            out.decks[d].fx_type = iv.clamp(0, 2) as u8;
                        }
                    }
                    if !empty {
                        value = name;
                    }
                }
                b"Beat" => {
                    if let (Some(d), Some(i), Some(nu), Some(de)) = (deck_of(&value, "FxSet"), num("idx"), num("numerator"), num("denominator")) {
                        let b = (nu as u16, (de as u16).max(1));
                        if (0.0..3.0).contains(&i) && RFX_BEATS.contains(&b) {
                            out.decks[d].fx_beats[i as usize] = b;
                        }
                    }
                }
                b"Preset" => {
                    let p = Preset { id: num("id").unwrap_or(0.0) as i64, name: attr("name").unwrap_or_default(), tracks: Default::default() };
                    if empty {
                        out.presets.push(p);
                    } else {
                        preset = Some(p);
                    }
                }
                b"Track" => {
                    let Some(idx) = num("idx").map(|i| i as usize).filter(|i| *i < SLOTS) else { continue };
                    let id = num("id").unwrap_or(0.0) as i64;
                    let url = attr("url").unwrap_or_default();
                    if let Some(p) = preset.as_mut() {
                        if id != 0 || !url.is_empty() {
                            p.tracks[idx] = Some(PresetTrack { id, name: attr("name").unwrap_or_default(), url, bpm: num("bpm").unwrap_or(0.0) as f32, first_beat: num("firstbeat").unwrap_or(0.0) as f32 });
                        }
                    } else if let Some(d) = deck_of(&value, "SamplerSet") {
                        if id != 0 || !url.is_empty() {
                            out.decks[d].slots[idx] = Some(SlotInfo {
                                id,
                                name: attr("name").unwrap_or_default(),
                                url,
                                length: num("length").unwrap_or(0.0) as i64,
                                samplingrate: num("samplingrate").unwrap_or(44100.0) as u32,
                                gain: num("gain").unwrap_or(50.0).clamp(0.0, 100.0) as f32,
                                bpm: num("bpm").unwrap_or(0.0) as f32,
                                first_beat: num("firstbeat").unwrap_or(0.0) as f32,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(Settings::path()).map(|s| Settings::from_xml(&s)).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = Settings::path();
        let tmp = p.with_extension("xml.tmp");
        if std::fs::write(&tmp, self.to_xml()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

// ------------------------------------------------------------------------------- factory kit

/// One sample of rekordbox's factory pack: (name, where it was unpacked, its tempo).
pub type FactorySample = (String, std::path::PathBuf, f32);

/// Where the factory kit is unpacked (rekordbox: `Music/rekordbox/Sampler/GROOVE CIRCUIT/
/// PRESET`; here the same folders under recordbox's data folder).
pub fn factory_dir() -> std::path::PathBuf {
    crate::db::data_dir().join("Sampler").join("GROOVE CIRCUIT").join("PRESET")
}

/// The `TRACK` rows of the pack's `sampler_pack.xml`, in order: (location, BPM).
pub fn parse_pack_xml(xml: &str) -> Vec<(String, f32)> {
    use quick_xml::events::Event;
    let mut rd = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match rd.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"TRACK" => {
                let attr = |key: &str| e.attributes().flatten().find(|a| a.key.as_ref() == key.as_bytes()).and_then(|a| a.unescape_value().ok().map(|v| v.to_string()));
                if let Some(loc) = attr("location") {
                    out.push((loc, attr("bpm").and_then(|b| b.parse().ok()).unwrap_or(0.0)));
                }
            }
            _ => {}
        }
    }
    out
}

/// Unpack rekordbox's factory pack (a plain zip archive) from the installed rekordbox into
/// `dir`, unless it is there already, and list its samples in the kit's order. The archive's
/// entries are deflated, which the app's own zip reader is not built to read, so macOS's
/// `unzip` does it.
pub fn import_factory(pack: &std::path::Path, dir: &std::path::Path) -> Result<Vec<FactorySample>, String> {
    if !pack.is_file() {
        return Err("rekordbox's GROOVE CIRCUIT factory samples were not found (rekordbox 7 is not installed in Applications).".into());
    }
    let run = |args: &[&std::ffi::OsStr]| std::process::Command::new("/usr/bin/unzip").args(args).output().map_err(|e| format!("unzip could not be started: {e}"));
    let os = std::ffi::OsStr::new;
    let listing = run(&[os("-p"), pack.as_os_str(), os("sampler_pack.xml")])?;
    let rows = parse_pack_xml(&String::from_utf8_lossy(&listing.stdout));
    if rows.is_empty() {
        return Err("The factory pack holds no sample list.".into());
    }
    // (Paths that would leave the folder are not taken.)
    let safe = |loc: &str| !loc.starts_with('/') && !loc.split('/').any(|p| p == "..") && loc.to_ascii_lowercase().ends_with(".wav");
    let missing = rows.iter().any(|(loc, _)| safe(loc) && !dir.join(loc).is_file());
    if missing {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let out = run(&[os("-o"), os("-q"), pack.as_os_str(), os("*.wav"), os("-d"), dir.as_os_str()])?;
        if !out.status.success() {
            return Err(format!("The factory pack could not be unpacked: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    Ok(rows
        .into_iter()
        .filter(|(loc, _)| safe(loc) && dir.join(loc).is_file())
        .map(|(loc, bpm)| {
            let p = dir.join(&loc);
            (p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), p, bpm)
        })
        .collect())
}

// ------------------------------------------------------------------------------- the interface's state

type Loaded = Result<Arc<Decoded>, String>;
const WAVE_COLS: usize = 120;

fn mini_wave(a: &Decoded) -> Vec<u8> {
    let frames = a.frames().max(1);
    (0..WAVE_COLS)
        .map(|c| {
            let (s, e) = (c * frames / WAVE_COLS, ((c + 1) * frames / WAVE_COLS).max(c * frames / WAVE_COLS + 1).min(frames));
            let stride = ((e - s) / 256).max(1);
            let peak = (s..e).step_by(stride).fold(0.0f32, |m, i| m.max(a.samples[i * 2].abs()).max(a.samples[i * 2 + 1].abs()));
            (peak.min(1.0) * 255.0) as u8
        })
        .collect()
}

/// What Groove Circuit keeps while the app runs.
pub struct GrooveUi {
    pub set: Settings,
    restored: bool,
    /// DRUM SWAP GAIN per deck, 0..100 (not kept between sessions, as in rekordbox).
    pub gain: [f32; DECKS],
    /// Mini waveform of each loaded slot (peak per column), once its file has been read.
    pub wave: [[Option<Vec<u8>>; SLOTS]; DECKS],
    loading: Vec<(usize, usize, std::sync::mpsc::Receiver<Loaded>)>,
    pub auto: [AutoMute; DECKS],
    last: [(bool, bool); DECKS],
    /// DRUM CAPTURE: the deck it is on (one at most), and its length in beats.
    pub capture_on: Option<u8>,
    pub capture_len: u32,
    /// The page of the column at the left of the waveforms: 0 zoom, 2 DRUM CAPTURE.
    pub page: u8,
    /// Mouse hold on the DRUM RELEASE FX button of each deck.
    pub rfx_held: [bool; DECKS],
    /// Slot level meters (dB).
    pub meter: [[f32; ROWS]; DECKS],
    factory: Option<std::sync::mpsc::Receiver<Result<Vec<FactorySample>, String>>>,
    /// What to do with the factory kit once it is unpacked: load it on these decks.
    factory_for: Vec<usize>,
    /// The name window of the presets: (preset to rename or None for a new one, deck, text).
    pub name_dialog: Option<(Option<i64>, usize, String)>,
    pub confirm_delete: Option<i64>,
    /// Files dropped from Finder, waiting for the pointer's place.
    pub dropped: Option<(Vec<std::path::PathBuf>, f64)>,
    /// Where each deck's slots were last drawn (for drops, and for the scripted run).
    pub slot_rects: [[egui::Rect; ROWS]; DECKS],
    pub panel_rects: [egui::Rect; DECKS],
    frames: u32,
    visible_decks: usize,
    /// The panel is hidden and something is still fading out or being read.
    winding: bool,
    pub textures: std::collections::HashMap<&'static str, Option<egui::TextureHandle>>,
    /// The DRUM CAPTURE area is being dragged from this deck's waveform onto a slot
    /// (`DrumSwapSlot::itemDroppedFromWave` 0x1008508c8: the slot captures that range).
    pub drag_capture: Option<u8>,
}

impl Default for GrooveUi {
    fn default() -> GrooveUi {
        GrooveUi {
            set: Settings::default(),
            restored: false,
            gain: [50.0; DECKS],
            wave: Default::default(),
            loading: Vec::new(),
            auto: [AutoMute::default(); DECKS],
            last: [(false, false); DECKS],
            capture_on: None,
            capture_len: 4,
            page: 0,
            rfx_held: [false; DECKS],
            meter: [[-120.0; ROWS]; DECKS],
            factory: None,
            factory_for: Vec::new(),
            name_dialog: None,
            confirm_delete: None,
            dropped: None,
            slot_rects: [[egui::Rect::NOTHING; ROWS]; DECKS],
            drag_capture: None,
            panel_rects: [egui::Rect::NOTHING; DECKS],
            frames: 0,
            visible_decks: 0,
            winding: false,
            textures: Default::default(),
        }
    }
}

fn with_engine<R>(deck: usize, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    ENGINES[deck.min(DECKS - 1)].lock().as_deref_mut().map(f)
}

impl App {
    /// The panel is shown: STEMS is enabled, PERFORMANCE mode, the function is enabled and
    /// its button in the top bar is on (`PlayerPanelL::setGrooveCircuitVisible` 0x102292c7c;
    /// rekordbox's plan check is always passed here).
    pub fn groove_visible(&self) -> bool {
        self.performance() && self.prefs.show_stems && self.prefs.groove.enable && self.prefs.groove.panel
    }

    /// The top bar's button can be used (STEMS and the function are enabled).
    pub fn groove_available(&self) -> bool {
        self.prefs.show_stems && self.prefs.groove.enable
    }

    /// Height the panel adds to each row of decks (the panel is 52 pt with 1 pt above it:
    /// `PlayerPanelL::getPlayerHeight` 0x10228a2d0 adds 53, or 106 with two rows of decks).
    pub fn groove_row_height(&self) -> f32 {
        if self.groove_visible() { 53.0 } else { 0.0 }
    }

    /// Height the panels add to the Performance player area.
    pub fn groove_extra_height(&self) -> f32 {
        self.groove_row_height() * if self.prefs.perf_four { 2.0 } else { 1.0 }
    }

    pub fn groove_save(&self) {
        // (Nothing is written in the scripted self-test's throwaway run either way: it works
        // on a copy of the data folder.)
        self.gc.set.save();
    }

    fn groove_slot_mut(&mut self, deck: usize, idx: usize) -> &mut Option<SlotInfo> {
        &mut self.gc.set.decks[deck.min(DECKS - 1)].slots[idx.min(SLOTS - 1)]
    }

    pub fn groove_slot(&self, deck: usize, idx: usize) -> Option<&SlotInfo> {
        self.gc.set.decks[deck.min(DECKS - 1)].slots[idx.min(SLOTS - 1)].as_ref()
    }

    /// The slot's sample has been read and can play.
    pub fn groove_ready(&self, deck: usize, idx: usize) -> bool {
        self.gc.wave[deck.min(DECKS - 1)][idx.min(SLOTS - 1)].is_some()
    }

    pub fn groove_is_played(&self, deck: usize, idx: usize) -> bool {
        ENGINES[deck.min(DECKS - 1)].lock().as_deref().is_some_and(|e| e.is_played(idx % ROWS, idx / ROWS))
    }

    /// Eject/Load Lock ("Lock"): a playing slot refuses a load, a capture and an eject.
    fn groove_locked(&mut self, deck: usize, idx: usize, eject: bool) -> bool {
        if self.prefs.groove.load_lock && self.groove_is_played(deck, idx) {
            self.toast_err(if eject { "Eject Lock. Not available during playback." } else { "Load Lock. Not available during playback." });
            return true;
        }
        false
    }

    fn groove_decode(&mut self, deck: usize, idx: usize, path: String) {
        self.gc.wave[deck][idx] = None;
        with_engine(deck, |e| e.set_clip(idx % ROWS, idx / ROWS, None));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::audio::decode_file(&path).map(Arc::new).map_err(|e| e.to_string()));
        });
        self.gc.loading.retain(|(d, i, _)| !(*d == deck && *i == idx));
        self.gc.loading.push((deck, idx, rx));
    }

    fn groove_put(&mut self, deck: usize, idx: usize, info: SlotInfo) {
        let (deck, idx) = (deck.min(DECKS - 1), idx.min(SLOTS - 1));
        let path = info.url.clone();
        *self.groove_slot_mut(deck, idx) = Some(info);
        self.groove_decode(deck, idx, path);
        self.groove_save();
    }

    /// The first beat of a collection track, seconds from its start (0 without a grid).
    fn groove_first_beat(&self, id: i64) -> f32 {
        self.db.grid(id).and_then(|g| g.beats(8.0).first().map(|b| b.time as f32)).unwrap_or(0.0)
    }

    /// Load a track of the collection into a slot (a drag from the browser or from a deck).
    pub fn groove_load_track(&mut self, deck: usize, idx: usize, id: i64) -> bool {
        let Some(t) = self.track(id).cloned() else { return false };
        if t.missing {
            self.toast_err(format!("The file for \"{}\" cannot be found.", t.display_title()));
            return false;
        }
        if self.groove_locked(deck, idx, false) {
            return false;
        }
        let first_beat = self.groove_first_beat(id);
        self.groove_put(deck, idx, SlotInfo { id, name: t.display_title(), url: t.path.clone(), length: 0, samplingrate: 44100, gain: 50.0, bpm: t.bpm as f32, first_beat });
        true
    }

    /// Load an audio file that need not be in the collection (dropped from Finder, a factory
    /// sample, a preset's file). One that is in the collection brings its title and tempo.
    pub fn groove_load_file(&mut self, deck: usize, idx: usize, path: String, name: String, bpm: f32, first_beat: f32) -> bool {
        if self.groove_locked(deck, idx, false) {
            return false;
        }
        if let Some(id) = self.db.track_by_path(&path).filter(|id| self.track(*id).is_some()) {
            return self.groove_load_track(deck, idx, id);
        }
        if !std::path::Path::new(&path).is_file() {
            self.toast_err(format!("The file for \"{name}\" cannot be found."));
            return false;
        }
        self.groove_put(deck, idx, SlotInfo { id: 0, name, url: path, length: 0, samplingrate: 44100, gain: 50.0, bpm, first_beat });
        true
    }

    /// EJECT: the slot is emptied (`force`: without the lock, as a preset load does).
    pub fn groove_eject(&mut self, deck: usize, idx: usize, force: bool) -> bool {
        let (deck, idx) = (deck.min(DECKS - 1), idx.min(SLOTS - 1));
        if !force && self.groove_locked(deck, idx, true) {
            return false;
        }
        with_engine(deck, |e| {
            if e.is_played(idx % ROWS, idx / ROWS) {
                e.stop(idx % ROWS);
            }
            e.set_clip(idx % ROWS, idx / ROWS, None)
        });
        *self.groove_slot_mut(deck, idx) = None;
        self.gc.wave[deck][idx] = None;
        self.gc.loading.retain(|(d, i, _)| !(*d == deck && *i == idx));
        self.groove_save();
        true
    }

    /// A slot's Play button (or its key): stop when it plays, else start on the deck's next
    /// beat. A deck that is not playing starts nothing: its clips stop when it leaves PLAY.
    pub fn groove_slot_play(&mut self, deck: usize, idx: usize) {
        let (deck, idx) = (deck.min(DECKS - 1), idx.min(SLOTS - 1));
        if self.deck_options.disable_play || !self.groove_visible() || !self.groove_ready(deck, idx) {
            return;
        }
        // (A deck that stands plays the slot at once and unsynced: `launchWithQuantize` with
        // the master not playing; see `Engine::render`.)
        with_engine(deck, |e| {
            if e.is_played(idx % ROWS, idx / ROWS) {
                e.stop(idx % ROWS);
            } else {
                e.launch(idx % ROWS, idx / ROWS);
            }
        });
    }

    /// DRUM LOOP BANK A / B: which bank's slots the panel shows (what plays keeps playing).
    pub fn groove_set_bank(&mut self, deck: usize, bank: u8) {
        let d = &mut self.gc.set.decks[deck.min(DECKS - 1)];
        if d.bank != bank.min(1) {
            d.bank = bank.min(1);
            self.groove_save();
        }
    }

    /// SINGLE / MULTI. A change stops all eight clips (`execSingleMulti` 0x100f18e5c).
    pub fn groove_set_multi(&mut self, deck: usize, multi: bool) {
        let deck = deck.min(DECKS - 1);
        if self.gc.set.decks[deck].multi != multi {
            self.gc.set.decks[deck].multi = multi;
            with_engine(deck, |e| {
                e.single = !multi;
                e.stop_all();
            });
            self.groove_save();
        }
    }

    /// Overall volume of this deck's four Drum Swap rows in either bank.
    pub fn groove_set_deck_gain(&mut self, deck: usize, percent: f32) {
        let Some(value) = self.gc.gain.get_mut(deck) else { return };
        if percent.is_finite() {
            *value = percent.clamp(0.0, 100.0);
        }
    }

    /// A slot's GAIN knob (0..100).
    pub fn groove_set_slot_gain(&mut self, deck: usize, idx: usize, v: f32) {
        let v = v.clamp(0.0, 100.0).round();
        if let Some(s) = self.groove_slot_mut(deck, idx).as_mut() {
            s.gain = v;
        }
        with_engine(deck, |e| {
            if let Some(c) = e.clip_mut(idx % ROWS, idx / ROWS) {
                c.gain = knob_gain(v);
            }
        });
    }

    /// The release FX selector's choice, and the beat arrows.
    pub fn groove_select_fx(&mut self, deck: usize, kind: u8) {
        self.gc.set.decks[deck.min(DECKS - 1)].fx_type = kind.min(2);
        self.groove_save();
    }
    pub fn groove_fx_beat_step(&mut self, deck: usize, dir: i32) {
        let d = &mut self.gc.set.decks[deck.min(DECKS - 1)];
        let k = d.fx_type.min(2) as usize;
        let next = beat_step(d.fx_beats[k], dir);
        if next != d.fx_beats[k] {
            d.fx_beats[k] = next;
            self.groove_save();
        }
    }

    /// DRUM RELEASE FX pressed or let go: on from the press to the release, as the FX
    /// panel's RELEASE FX button.
    pub fn groove_release_fx(&mut self, deck: usize, on: bool) {
        let deck = deck.min(DECKS - 1);
        self.gc.rfx_held[deck] = on && self.groove_visible();
    }

    pub fn groove_release_held(&self, deck: usize) -> bool {
        !self.deck_options.disable_play && (self.gc.rfx_held[deck] || self.midi.drum_release[deck])
    }

    // -------------------------------------------------------------------------- capture
    /// DRUM CAPTURE On/Off for a deck: one deck at most (`setCaptureOnOff` 0x10076a87c).
    pub fn groove_capture_toggle(&mut self, deck: u8) {
        if self.gc.capture_on == Some(deck) {
            self.gc.capture_on = None;
        } else if self.groove_visible() && deck < self.deck_count() {
            self.gc.capture_on = Some(deck);
        }
    }

    /// DRUM CAPTURE Length: 4, 8, 16, 32 and round again.
    pub fn groove_capture_length_next(&mut self) {
        if !self.groove_visible() {
            return;
        }
        let i = CAPTURE_LENGTHS.iter().position(|l| *l == self.gc.capture_len).unwrap_or(0);
        self.gc.capture_len = CAPTURE_LENGTHS[(i + 1) % CAPTURE_LENGTHS.len()];
    }

    /// The seconds a capture on `deck` would take from where it plays now: (start, end).
    pub fn groove_capture_span(&self, deck: u8) -> Option<(f64, f64)> {
        let d = self.deck_of(deck);
        d.track?;
        let beats: Vec<(f64, f64)> = d.beats.iter().map(|b| (b.time, b.bpm)).collect();
        let (pos, sr) = {
            let s = self.player.decks[(deck as usize).min(3)].lock();
            (s.pos, s.audio.as_ref().map(|a| a.sr)?)
        };
        let (a, b, _, _) = capture_range(&beats, pos / sr.max(1) as f64, self.gc.capture_len as usize, d.duration);
        Some((a, b))
    }

    /// A slot pressed while DRUM CAPTURE is on: the deck it is on gives the next 4 / 8 / 16 /
    /// 32 beats of its DRUMS part (or of the whole track, as Preferences says) to this slot
    /// (`DrumSwapSlot::doCapture` 0x100854300). The file is a 16-bit stereo WAV.
    pub fn groove_capture(&mut self, view_deck: usize, idx: usize) -> bool {
        let (view_deck, idx) = (view_deck.min(DECKS - 1), idx.min(SLOTS - 1));
        let Some(src) = self.gc.capture_on else { return false };
        let (track, beats, duration) = {
            let d = self.deck_of(src);
            (d.track.and_then(|id| self.track(id)).cloned(), d.beats.iter().map(|b| (b.time, b.bpm)).collect::<Vec<(f64, f64)>>(), d.duration)
        };
        let Some(track) = track else { return false };
        let (pos, audio, stems) = {
            let s = self.player.decks[(src as usize).min(3)].lock();
            (s.pos, s.audio.clone(), s.stems.clone())
        };
        let Some(audio) = audio else { return false };
        if self.groove_locked(view_deck, idx, false) {
            return false;
        }
        let from_stem = self.prefs.groove.capture_stem != 2;
        if from_stem && stems.is_none() {
            self.toast_err("Invalid operation. The STEMS function analysis is not complete.");
            return false;
        }
        let start = pos / audio.sr.max(1) as f64;
        let (a, b, bpm, first) = capture_range(&beats, start, self.gc.capture_len as usize, duration.min(audio.duration()));
        // The DRUMS part is kept at 44.1 kHz; the whole track at its own rate.
        let (sr, samples): (u32, Vec<f32>) = match stems.filter(|_| from_stem) {
            Some(st) => {
                let sr = crate::stems::htdemucs::SAMPLE_RATE;
                let f0 = ((a * sr as f64).round() as usize).min(st.len);
                let f1 = (f0 + capture_frames(a, b, sr)).min(st.len);
                (sr, (f0..f1).flat_map(|i| [st.data[i], st.data[st.len + i]]).collect())
            }
            None => {
                let frames = audio.frames();
                let f0 = ((a * audio.sr as f64).round() as usize).min(frames);
                let f1 = (f0 + capture_frames(a, b, audio.sr)).min(frames);
                (audio.sr, audio.samples[f0 * 2..f1 * 2].to_vec())
            }
        };
        if samples.len() < 4 {
            return false;
        }
        let dir = crate::sampler_ext::capture_dir(&self.prefs.groove.capture_dir);
        let suffix = if from_stem { "_drum_capture" } else { "_capture" };
        let Ok(path) = crate::sampler_ext::save_capture(&dir, &track.display_title(), suffix, sr, &samples) else {
            self.toast_err("Error while saving");
            return false;
        };
        // Loaded from what was written (16 bit), without reading the file back.
        let q: Vec<f32> = samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() / 32768.0).collect();
        let frames = q.len() / 2;
        let decoded = Arc::new(Decoded { sr, samples: q, raw_frames: frames, damaged: 0, truncated: false });
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let info = SlotInfo { id: 0, name, url: path.to_string_lossy().to_string(), length: frames as i64, samplingrate: sr, gain: 50.0, bpm: bpm as f32, first_beat: first.max(0.0) as f32 };
        self.gc.loading.retain(|(d, i, _)| !(*d == view_deck && *i == idx));
        self.groove_set_decoded(view_deck, idx, &info, decoded);
        *self.groove_slot_mut(view_deck, idx) = Some(info);
        self.groove_save();
        // (rekordbox switches the flag of the panel's own deck off after a capture.)
        if self.gc.capture_on == Some(view_deck as u8) {
            self.gc.capture_on = None;
        }
        true
    }

    fn groove_set_decoded(&mut self, deck: usize, idx: usize, info: &SlotInfo, a: Arc<Decoded>) {
        self.gc.wave[deck][idx] = Some(mini_wave(&a));
        let clip = GcClip { audio: a, bpm: info.bpm, gain: knob_gain(info.gain), first_beat: info.first_beat };
        with_engine(deck, |e| e.set_clip(idx % ROWS, idx / ROWS, Some(clip)));
    }

    // -------------------------------------------------------------------------- presets
    /// The slots of a deck as a preset keeps them.
    fn groove_preset_tracks(&self, deck: usize) -> [Option<PresetTrack>; SLOTS] {
        std::array::from_fn(|i| self.groove_slot(deck, i).map(|s| PresetTrack { id: s.id, name: s.name.clone(), url: s.url.clone(), bpm: s.bpm, first_beat: s.first_beat }))
    }

    /// "Save as New Preset" (at most ten user presets).
    pub fn groove_preset_save_new(&mut self, deck: usize, name: &str) -> Option<i64> {
        if self.gc.set.presets.len() >= MAX_USER_PRESETS {
            return None;
        }
        let mut id = chrono::Local::now().timestamp_millis();
        while self.gc.set.presets.iter().any(|p| p.id == id) {
            id += 1;
        }
        let name = if name.trim().is_empty() { "Untitled Preset".to_string() } else { name.trim().to_string() };
        self.gc.set.presets.push(Preset { id, name, tracks: self.groove_preset_tracks(deck) });
        self.groove_save();
        Some(id)
    }

    pub fn groove_preset_overwrite(&mut self, deck: usize, id: i64) {
        let tracks = self.groove_preset_tracks(deck);
        if let Some(p) = self.gc.set.presets.iter_mut().find(|p| p.id == id) {
            p.tracks = tracks;
            self.groove_save();
        }
    }

    pub fn groove_preset_rename(&mut self, id: i64, name: &str) {
        if let Some(p) = self.gc.set.presets.iter_mut().find(|p| p.id == id) {
            if !name.trim().is_empty() {
                p.name = name.trim().to_string();
            }
        }
        self.groove_save();
    }

    pub fn groove_preset_delete(&mut self, id: i64) {
        self.gc.set.presets.retain(|p| p.id != id);
        self.groove_save();
    }

    /// Load eight slots (A1..A4, B1..B4); an empty one ejects its slot
    /// (`GrooveCircuitView::loadSamples` 0x100f19084). Refused while a slot plays and the
    /// lock is on.
    pub fn groove_load_slots(&mut self, deck: usize, tracks: &[Option<PresetTrack>; SLOTS]) -> bool {
        let deck = deck.min(DECKS - 1);
        if self.prefs.groove.load_lock && (0..SLOTS).any(|i| self.groove_is_played(deck, i)) {
            self.toast_err("Load Lock. Not available during playback.");
            return false;
        }
        for (i, t) in tracks.iter().enumerate() {
            match t {
                Some(t) if t.id != 0 && self.track(t.id).is_some() => {
                    self.groove_load_track(deck, i, t.id);
                }
                Some(t) if !t.url.is_empty() => {
                    self.groove_load_file(deck, i, t.url.clone(), t.name.clone(), t.bpm, t.first_beat);
                }
                _ => {
                    self.groove_eject(deck, i, true);
                }
            }
        }
        true
    }

    pub fn groove_preset_load(&mut self, deck: usize, id: i64) {
        if let Some(p) = self.gc.set.presets.iter().find(|p| p.id == id).cloned() {
            self.groove_load_slots(deck, &p.tracks);
        }
    }

    /// The factory kit onto these decks' slots, once it has been unpacked (in the background).
    pub fn groove_load_factory(&mut self, decks: &[usize]) {
        for d in decks {
            if !self.gc.factory_for.contains(d) {
                self.gc.factory_for.push(*d);
            }
        }
        if self.gc.factory.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            let dir = factory_dir();
            std::thread::spawn(move || {
                let _ = tx.send(import_factory(std::path::Path::new(FACTORY_PACK), &dir));
            });
            self.gc.factory = Some(rx);
        }
    }

    /// File > Additional contents > GROOVE CIRCUIT FACTORY SAMPLES PACK > Import (command
    /// 8219; `app_hooks::pack_import` id `pack_gc_import`): a pack file chosen in a file
    /// panel (the installed rekordbox's `GROOVE_CIRCUIT_FACTORY_SAMPLES.spp` when `path` is
    /// None and the panel is cancelled nothing happens) is unpacked into the factory folder
    /// and its kit goes onto the decks' slots, as the first showing of the panel does.
    pub fn groove_import_pack(&mut self, path: Option<std::path::PathBuf>) {
        let pack = match path {
            Some(p) => p,
            None => {
                let start = std::path::Path::new(FACTORY_PACK).parent().map(|p| p.to_path_buf()).unwrap_or_default();
                let mut d = rfd::FileDialog::new().set_title("GROOVE CIRCUIT FACTORY SAMPLES PACK").add_filter("GROOVE CIRCUIT pack", &["spp", "zip"]);
                if start.is_dir() {
                    d = d.set_directory(&start);
                }
                match d.pick_file() {
                    Some(p) => p,
                    None => return,
                }
            }
        };
        let decks: Vec<usize> = (0..self.deck_count() as usize).collect();
        for d in &decks {
            if !self.gc.factory_for.contains(d) {
                self.gc.factory_for.push(*d);
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = factory_dir();
        std::thread::spawn(move || {
            let _ = tx.send(import_factory(&pack, &dir));
        });
        self.gc.factory = Some(rx);
    }

    // -------------------------------------------------------------------------- per frame
    /// Once per frame, in every mode: bring the saved slots back the first time the panel is
    /// shown, take in files that have been read, keep the engines' settings current, apply
    /// the DRUMS mute rule, and stop everything when the panel goes away.
    pub fn groove_poll(&mut self) {
        let visible = self.groove_visible();
        let count = usize::from(self.deck_count()).min(DECKS);
        if !visible {
            self.midi.drum_release.fill(false);
            if self.gc.visible_decks == 0 && !self.gc.winding {
                return;
            }
            if self.gc.visible_decks > 0 {
                // (`onGrooveCircuitDisabled` 0x100f1af0c: capture off, all clips stopped.)
                self.gc.visible_decks = 0;
                self.gc.capture_on = None;
                self.gc.page = 0;
                self.gc.rfx_held = [false; DECKS];
                for d in 0..DECKS {
                    with_engine(d, |e| {
                        e.stop_all();
                        e.release = None;
                    });
                }
            }
            // The engines stay with the audio callback until they have faded out, and the
            // DRUMS part gets its state back.
            let mut winding = !self.gc.loading.is_empty();
            for d in 0..DECKS {
                let busy = ENGINES[d].lock().as_deref().is_some_and(|e| e.sounding || e.fx_active() || e.any_played());
                if busy {
                    winding = true;
                } else {
                    ACTIVE[d].store(false, Relaxed);
                }
                self.groove_mute_rule(d, false);
            }
            self.gc.winding = winding;
            self.groove_take_loaded();
            return;
        }
        // A layout change can hide a deck while its lever is down. Its engine
        // still exists, but the visible-deck loop below will no longer update it.
        if self.gc.capture_on.is_some_and(|deck| usize::from(deck) >= count) {
            self.gc.capture_on = None;
        }
        for d in count..self.gc.visible_decks {
            self.gc.rfx_held[d] = false;
            self.midi.drum_release[d] = false;
            with_engine(d, |e| e.release = None);
        }
        self.gc.visible_decks = count;
        self.gc.frames = self.gc.frames.wrapping_add(1);
        if !self.gc.restored {
            self.gc.restored = true;
            self.gc.set = Settings::load();
            for d in 0..DECKS {
                for i in 0..SLOTS {
                    let Some(s) = self.groove_slot(d, i).cloned() else { continue };
                    // A file that is gone leaves its slot empty.
                    if std::path::Path::new(&s.url).is_file() {
                        self.groove_decode(d, i, s.url);
                    } else {
                        *self.groove_slot_mut(d, i) = None;
                    }
                }
            }
            // The first time the panel is shown, the factory kit goes onto all four decks
            // ("Drum Loop Audio" > "Automatically import audio files").
            if !self.prefs.groove.bundled_loaded && self.prefs.groove.auto_import {
                self.prefs.groove.bundled_loaded = true;
                self.save_prefs();
                let empty: Vec<usize> = (0..DECKS).filter(|d| self.gc.set.decks[*d].slots.iter().all(|s| s.is_none())).collect();
                self.groove_load_factory(&empty);
            }
        }
        // The engines of the decks in the layout (the first one of a deck also makes its
        // release FX unit).
        for d in 0..count {
            let mut g = ENGINES[d].lock();
            if g.is_none() {
                let mut e = Box::new(Engine::new(d));
                e.prepare_fx();
                *g = Some(e);
            }
            drop(g);
            ACTIVE[d].store(true, Relaxed);
        }
        self.groove_take_loaded();
        // The factory kit, once unpacked.
        if let Some(r) = self.gc.factory.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.gc.factory = None;
            let decks = std::mem::take(&mut self.gc.factory_for);
            match r {
                Ok(samples) => {
                    let tracks: [Option<PresetTrack>; SLOTS] = std::array::from_fn(|i| samples.get(i).map(|(name, p, bpm)| PresetTrack { id: 0, name: name.clone(), url: p.to_string_lossy().to_string(), bpm: *bpm, first_beat: 0.0 }));
                    for d in decks {
                        self.groove_load_slots(d, &tracks);
                    }
                }
                Err(e) => self.toast_err(e),
            }
        }
        // A slot whose track had no tempo when it was loaded takes it once it is analysed.
        if self.gc.frames % 60 == 0 {
            for d in 0..DECKS {
                for i in 0..SLOTS {
                    let Some((id, bpm)) = self.groove_slot(d, i).map(|s| (s.id, s.bpm)) else { continue };
                    if id != 0 && bpm <= 0.0 {
                        let now = self.track(id).map(|t| t.bpm as f32).unwrap_or(0.0);
                        if now > 0.0 {
                            let fb = self.groove_first_beat(id);
                            if let Some(s) = self.groove_slot_mut(d, i).as_mut() {
                                s.bpm = now;
                                s.first_beat = fb;
                            }
                            with_engine(d, |e| {
                                if let Some(c) = e.clip_mut(i % ROWS, i / ROWS) {
                                    c.bpm = now;
                                    c.first_beat = fb;
                                }
                            });
                        }
                    }
                }
            }
        }
        // Settings to the engines, the mute rule, the meters.
        for d in 0..count {
            let set = &self.gc.set.decks[d];
            let (single, kind, beat) = (!set.multi, set.fx_type.min(2), set.fx_beats[set.fx_type.min(2) as usize]);
            let bpm = self.groove_deck_bpm(d);
            let held = self.groove_release_held(d);
            let gain = knob_gain(self.gc.gain[d]);
            let mut peaks = [0.0f32; ROWS];
            let launched = with_engine(d, |e| {
                e.gain = gain;
                e.single = single;
                e.release = if held { Some((kind, beat.0 as f32 / beat.1.max(1) as f32)) } else { None };
                if bpm > 0.0 {
                    e.bpm = bpm;
                }
                for (r, p) in peaks.iter_mut().enumerate() {
                    *p = e.take_peak(r);
                }
                e.any_sounding()
            })
            .unwrap_or(false);
            for (r, p) in peaks.iter().enumerate() {
                let db = if *p > 1e-6 { 20.0 * p.log10() } else { -120.0 };
                let m = &mut self.gc.meter[d][r];
                *m = if db > *m { db } else { (*m - 1.5).max(db) };
            }
            self.groove_mute_rule(d, launched);
        }
    }

    /// The tempo heard on deck `d`: the grid's BPM where it plays times its speed (0: no track).
    fn groove_deck_bpm(&self, d: usize) -> f32 {
        let dk = self.deck_of(d as u8);
        if dk.track.is_none() {
            return 0.0;
        }
        let (pos, tempo) = {
            let s = self.player.decks[d.min(3)].lock();
            (s.pos / s.audio.as_ref().map(|a| a.sr).unwrap_or(44100).max(1) as f64, s.tempo)
        };
        let i = dk.beats.partition_point(|b| b.time <= pos);
        let bpm = if i > 0 { dk.beats[i - 1].bpm } else { dk.beats.first().map(|b| b.bpm).or(dk.track.and_then(|id| self.track(id)).map(|t| t.bpm)).unwrap_or(0.0) };
        (bpm * tempo) as f32
    }

    /// The DRUMS mute rule for one deck (see `AutoMute`).
    fn groove_mute_rule(&mut self, deck: usize, launched: bool) {
        let multi = self.gc.set.decks[deck].multi;
        let muted = !self.player.decks[deck].lock().stem_on[0];
        self.gc.auto[deck].part_changed(muted);
        if self.gc.last[deck] != (multi, launched) {
            self.gc.last[deck] = (multi, launched);
            if let Some(m) = self.gc.auto[deck].update(multi, launched, muted) {
                self.player.decks[deck].lock().stem_on[0] = !m;
            }
        }
        // While the automatic mute stands the engine keeps the deck's drums out from the
        // swap's first frame; once it is dropped (un-muted by hand) the drums are back.
        let hold = self.gc.auto[deck].on || !launched;
        with_engine(deck, |e| e.hold_drums = hold);
    }

    fn groove_take_loaded(&mut self) {
        let mut done = Vec::new();
        self.gc.loading.retain(|(d, i, rx)| match rx.try_recv() {
            Ok(r) => {
                done.push((*d, *i, r));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (d, i, r) in done {
            let Some(mut s) = self.groove_slot(d, i).cloned() else { continue };
            match r {
                Ok(a) => {
                    s.length = a.frames() as i64;
                    s.samplingrate = a.sr;
                    self.groove_set_decoded(d, i, &s, a);
                    *self.groove_slot_mut(d, i) = Some(s);
                }
                Err(e) => {
                    self.toast_err(format!("\"{}\" could not be loaded into the DRUM SWAP SLOT: {e}", s.name));
                    *self.groove_slot_mut(d, i) = None;
                }
            }
            self.groove_save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loop of `beats` beats at `bpm`: a click of 1.0 for 20 frames on each beat, 0.25 between.
    fn drum_loop(bpm: f32, beats: usize, sr: u32) -> GcClip {
        let fpb = sr as f64 * 60.0 / bpm as f64;
        let frames = (beats as f64 * fpb).round() as usize;
        let mut samples = vec![0.25f32; frames * 2];
        for b in 0..beats {
            let f0 = (b as f64 * fpb).round() as usize;
            for f in f0..(f0 + 20).min(frames) {
                samples[f * 2] = 1.0;
                samples[f * 2 + 1] = 1.0;
            }
        }
        GcClip { audio: Arc::new(Decoded { sr, samples, raw_frames: frames, damaged: 0, truncated: false }), bpm, gain: 1.0, first_beat: 0.0 }
    }

    /// A loop with a kick on every beat (a decaying 150 Hz burst of about 80 ms): something a
    /// time stretcher can carry, unlike `drum_loop`'s 20-frame clicks, which fall between grains.
    fn kick_loop(bpm: f32, beats: usize, sr: u32) -> GcClip {
        let fpb = sr as f64 * 60.0 / bpm as f64;
        let frames = (beats as f64 * fpb).round() as usize;
        let mut samples = vec![0.0f32; frames * 2];
        for b in 0..beats {
            let f0 = (b as f64 * fpb).round() as usize;
            for k in 0..(sr as usize * 80 / 1000).min(frames - f0) {
                let v = (k as f32 * 150.0 * std::f32::consts::TAU / sr as f32).cos() * (-(k as f32) / (sr as f32 * 0.02)).exp();
                samples[(f0 + k) * 2] = v;
                samples[(f0 + k) * 2 + 1] = v;
            }
        }
        GcClip { audio: Arc::new(Decoded { sr, samples, raw_frames: frames, damaged: 0, truncated: false }), bpm, gain: 1.0, first_beat: 0.0 }
    }

    fn grid(bpm: f64, secs: f64) -> Vec<f64> {
        (0..(secs * bpm / 60.0) as usize).map(|i| i as f64 * 60.0 / bpm).collect()
    }

    /// Run the engine as a deck playing at `speed` would call it; the deck's position moves on.
    fn run(e: &mut Engine, g: &[f64], pos: &mut f64, speed: f64, frames: usize, playing: bool) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * 2);
        let mut done = 0;
        while done < frames {
            let n = 512.min(frames - done);
            let d = DeckIn { playing, pos: *pos, sr: 44100, step: speed, grid: Some(g), stems: None, drums_gain: 1.0 };
            let some = e.render(&d, n, 44100);
            for i in 0..n {
                let (l, r) = if some { e.out(i) } else { (0.0, 0.0) };
                out.push(l);
                out.push(r);
            }
            if playing {
                *pos += n as f64 * speed;
            }
            done += n;
        }
        out
    }

    #[test]
    fn a_slot_starts_on_the_decks_next_beat_and_stays_on_its_beats() {
        // Deck at 120 BPM (22050 frames per beat), the loop is 120 BPM too.
        let g = grid(120.0, 60.0);
        let mut e = Engine::new(0);
        e.set_clip(0, 0, Some(drum_loop(120.0, 4, 44100)));
        let mut pos = 5000.0;
        assert!(e.launch(0, 0));
        assert!(e.is_played(0, 0) && !e.any_sounding(), "launched, waiting for the beat");
        let out = run(&mut e, &g, &mut pos, 1.0, 22050 * 6, true);
        let first = (0..out.len() / 2).find(|i| out[i * 2] != 0.0).unwrap();
        assert_eq!(first, 22050 - 5000, "the first frame falls on the deck's beat");
        // Every click of the loop is on a beat of the deck from then on.
        let clicks: Vec<usize> = (1..out.len() / 2).filter(|i| out[i * 2] > 0.9 && out[i * 2 - 2] < 0.9).collect();
        assert!(clicks.len() >= 5, "{clicks:?}");
        for c in &clicks {
            let off = (*c as f64 + 5000.0) % 22050.0;
            assert!(off.min(22050.0 - off) < 3.0, "click at {c}: {off} frames off the beat");
        }
        assert_eq!(e.row_state(0).playing, Some(0));
    }

    #[test]
    fn a_slot_follows_the_decks_tempo_whatever_its_own() {
        // A 100 BPM loop on a 125 BPM track played 4 % faster: beats every 20352 frames.
        for (deck_bpm, speed, loop_bpm) in [(125.0f64, 1.04f64, 100.0f32), (90.0, 1.0, 140.0), (128.0, 0.94, 123.0)] {
            let g = grid(deck_bpm, 120.0);
            let mut e = Engine::new(0);
            e.set_clip(1, 0, Some(kick_loop(loop_bpm, 4, 44100)));
            let mut pos = 0.0;
            e.launch(1, 0);
            let out = run(&mut e, &g, &mut pos, speed, 44100 * 6, true);
            let beat = 44100.0 * 60.0 / (deck_bpm * speed);
            // (The loop is time-stretched, so a click's edge can move by a grain's search
            // range: its place is taken as the loudest frame around each beat.)
            let peaks: Vec<f64> = (1..8)
                .map(|k| {
                    let c = (k as f64 * beat) as usize;
                    let (lo, hi) = (c.saturating_sub(900), (c + 900).min(out.len() / 2 - 1));
                    let p = (lo..hi).max_by(|a, b| out[a * 2].abs().total_cmp(&out[b * 2].abs())).unwrap();
                    p as f64 - k as f64 * beat
                })
                .collect();
            for p in &peaks {
                assert!(p.abs() < 700.0, "deck {deck_bpm} x {speed}, loop {loop_bpm}: a beat of the loop is {p} frames off ({peaks:?})");
            }
            // The loop's position has moved by the deck's beats, not by its own tempo.
            let st = e.row_state(1);
            let want = ((44100.0 * 6.0 / beat) % 4.0) / 4.0;
            let d = (st.progress as f64 - want + 0.5).rem_euclid(1.0) - 0.5;
            assert!(d.abs() < 0.01, "progress {} want {want}", st.progress);
        }
    }

    #[test]
    fn the_phase_is_held_modulo_one_beat_when_the_deck_is_moved() {
        let g = grid(120.0, 120.0);
        let mut e = Engine::new(0);
        e.set_clip(0, 0, Some(drum_loop(120.0, 4, 44100)));
        let mut pos = 0.0;
        e.launch(0, 0);
        run(&mut e, &g, &mut pos, 1.0, 22050 * 3, true);
        let before = e.row_state(0).progress;
        // A jump of exactly two beats: the loop carries on where it was.
        pos += 44100.0;
        run(&mut e, &g, &mut pos, 1.0, 2048, true);
        let after = e.row_state(0).progress;
        assert!((after - before - 2048.0 / 88200.0).abs() < 0.002, "{before} {after}");
        // A jump of a third of a beat: it is moved back onto the beat (after two buffers).
        pos += 7350.0;
        run(&mut e, &g, &mut pos, 1.0, 22050 * 2, true);
        let have = e.row_state(0).progress as f64 * 4.0;
        let want = pos / 22050.0;
        assert!(phase_error(have, want).abs() < 0.01, "{have} {want}");
        assert_eq!(e.row_state(0).playing, Some(0));
    }

    #[test]
    fn single_plays_one_row_and_multi_plays_several() {
        let g = grid(120.0, 60.0);
        // SINGLE: starting row 1 stops row 0 on the beat row 1 starts on.
        let mut e = Engine::new(0);
        e.single = true;
        for r in 0..3 {
            e.set_clip(r, 0, Some(drum_loop(120.0, 4, 44100)));
        }
        let mut pos = 0.0;
        e.launch(0, 0);
        run(&mut e, &g, &mut pos, 1.0, 22050 * 2, true);
        assert_eq!(e.row_state(0).playing, Some(0));
        e.launch(1, 0);
        // Until the beat both are "launched", and only row 0 sounds.
        assert!(e.is_played(0, 0) && e.is_played(1, 0));
        run(&mut e, &g, &mut pos, 1.0, 22050 * 2, true);
        assert_eq!((e.row_state(0).playing, e.row_state(1).playing), (None, Some(0)));
        // Two launches in the same beat: the later one wins.
        e.launch(0, 0);
        e.launch(2, 0);
        run(&mut e, &g, &mut pos, 1.0, 22050 * 2, true);
        assert_eq!((e.row_state(0).playing, e.row_state(1).playing, e.row_state(2).playing), (None, None, Some(0)));
        // MULTI: they add up.
        let mut m = Engine::new(0);
        m.single = false;
        for r in 0..3 {
            m.set_clip(r, 0, Some(drum_loop(120.0, 4, 44100)));
        }
        let mut pos = 0.0;
        m.launch(0, 0);
        m.launch(1, 0);
        m.launch(2, 0);
        let out = run(&mut m, &g, &mut pos, 1.0, 22050 * 2, true);
        assert!((0..3).all(|r| m.row_state(r).playing == Some(0)));
        assert!((out[2 * 30000] - 0.75).abs() < 1e-3, "three loops at 0.25: {}", out[2 * 30000]);
        // A slot of bank B on a row that plays bank A takes the row over.
        m.set_clip(0, 1, Some(drum_loop(120.0, 4, 44100)));
        m.launch(0, 1);
        run(&mut m, &g, &mut pos, 1.0, 22050 * 2, true);
        assert_eq!(m.row_state(0).playing, Some(1));
        assert!(m.is_played(0, 1) && !m.is_played(0, 0));
    }

    #[test]
    fn clips_stop_when_the_deck_leaves_play_and_a_stopped_deck_plays_them_unsynced() {
        let g = grid(120.0, 60.0);
        let mut e = Engine::new(0);
        e.set_clip(0, 0, Some(drum_loop(120.0, 4, 44100)));
        let mut pos = 0.0;
        e.launch(0, 0);
        run(&mut e, &g, &mut pos, 1.0, 22050, true);
        assert!(e.any_sounding());
        let out = run(&mut e, &g, &mut pos, 1.0, 2048, false);
        assert!(!e.any_played() && e.row_state(0).playing.is_none());
        assert_eq!(out[2 * 2000], 0.0, "silent after the 3 ms fade");
        // rekordbox (`launchWithQuantize` with the deck not playing): a launch on a stopped
        // deck starts at once and plays at the clip's own speed, from its first frame.
        let clip = drum_loop(120.0, 4, 44100);
        e.launch(0, 0);
        let out = run(&mut e, &g, &mut pos, 1.0, 4096, false);
        assert!(e.any_sounding() && e.row_state(0).playing == Some(0), "plays while the deck stands");
        for i in [10usize, 500, 3000] {
            assert!((out[i * 2] - clip.audio.samples[i * 2]).abs() < 0.02, "frame {i}: {} vs {}", out[i * 2], clip.audio.samples[i * 2]);
        }
        // The deck starts: the slot goes on; it stops only when the deck leaves PLAY again.
        run(&mut e, &g, &mut pos, 1.0, 4096, true);
        assert!(e.any_sounding(), "still playing after the deck started");
        run(&mut e, &g, &mut pos, 1.0, 2048, false);
        assert!(!e.any_played(), "stopped when the deck left PLAY");
    }

    #[test]
    fn the_drums_mute_rule() {
        // SINGLE, a slot starts: the part is muted and its state remembered; it stops: back.
        let mut a = AutoMute::default();
        assert_eq!(a.update(false, true, false), Some(true));
        assert!(a.on && !a.was_muted);
        assert_eq!(a.update(false, true, true), None, "already muted automatically");
        assert_eq!(a.update(false, false, true), Some(false));
        assert!(!a.on);
        // It was muted before: it stays muted afterwards.
        let mut a = AutoMute::default();
        assert_eq!(a.update(false, true, true), Some(true));
        assert_eq!(a.update(false, false, true), Some(true));
        // MULTI never mutes; switching to MULTI while a swap plays restores.
        let mut a = AutoMute::default();
        assert_eq!(a.update(true, true, false), None);
        assert_eq!(a.update(false, true, false), Some(true));
        assert_eq!(a.update(true, true, true), Some(false));
        // Un-muted by hand while muted automatically: forgotten, nothing restored later.
        let mut a = AutoMute::default();
        assert_eq!(a.update(false, true, false), Some(true));
        a.part_changed(false);
        assert!(!a.on);
        assert_eq!(a.update(false, false, false), None);
    }

    #[test]
    fn single_takes_the_decks_drums_out_from_the_swaps_first_frame() {
        let g = grid(120.0, 60.0);
        let mut e = Engine::new(0);
        e.single = true;
        e.set_clip(0, 0, Some(drum_loop(120.0, 4, 44100)));
        let mut pos = 100.0;
        e.launch(0, 0);
        let d = DeckIn { playing: true, pos, sr: 44100, step: 1.0, grid: Some(&g), stems: None, drums_gain: 1.0 };
        e.render(&d, 512, 44100);
        assert!(!e.cut, "the launch waits for the beat: the deck's drums still play");
        run(&mut e, &g, &mut pos, 1.0, 22050, true);
        assert!(e.cut);
        // MULTI, or the part un-muted by hand: the deck keeps its drums.
        e.hold_drums = false;
        run(&mut e, &g, &mut pos, 1.0, 512, true);
        assert!(!e.cut);
        e.hold_drums = true;
        e.single = false;
        run(&mut e, &g, &mut pos, 1.0, 512, true);
        assert!(!e.cut);
    }

    #[test]
    fn the_release_fx_runs_on_the_drums_and_the_swap_only() {
        // The DRUMS part is 0.5. Give the other stems a different signal so a
        // release effect applied to the full mix would fail this check.
        let g = grid(120.0, 60.0);
        let len = 44100 * 5;
        let mut data = vec![-0.25f32; len * 8];
        data[..len * 2].iter_mut().for_each(|v| *v = 0.5);
        let st = crate::stems::Stems { len, data };
        let mut e = Engine::new(0);
        e.prepare_fx();
        e.bpm = 120.0;
        let mut pos = 0.0;
        let mut step = |e: &mut Engine, pos: &mut f64, frames: usize| -> (Vec<f32>, bool) {
            let mut out = Vec::new();
            let mut cut = false;
            let mut done = 0;
            while done < frames {
                let n = 512.min(frames - done);
                let d = DeckIn { playing: true, pos: *pos, sr: 44100, step: 1.0, grid: Some(&g), stems: Some(&st), drums_gain: 1.0 };
                let some = e.render(&d, n, 44100);
                cut = e.cut;
                for i in 0..n {
                    out.push(if some { e.out(i).0 } else { 0.0 });
                }
                *pos += n as f64;
                done += n;
            }
            (out, cut)
        };
        // Not held: nothing is added and the deck keeps its drums.
        let (out, cut) = step(&mut e, &mut pos, 44100);
        assert!(!cut && out.iter().all(|v| *v == 0.0));
        // BACK SPIN over one beat (0.5 s): the deck's drums go out, the spin's sound comes in
        // their place and has stopped after the beat.
        e.release = Some((2, 1.0));
        let (out, cut) = step(&mut e, &mut pos, 44100);
        assert!(cut);
        // The 120 BPM backspin accelerates from 0 to 8x in 160 ms. Above 1x,
        // the gain follows 0.5598 * (0.98 + 0.02 * speed).
        for frame in [1000, 2000, 3700] {
            let speed = 8.0 * frame as f32 / (0.160 * 44100.0);
            let expected = 0.5 * 0.5598 * (0.98 + 0.02 * speed);
            assert!((out[frame] - expected).abs() < 0.002, "backspin frame {frame}: {} vs {expected}", out[frame]);
        }
        assert!(out[40000].abs() < 0.01, "stopped: {}", out[40000]);
        // The press itself does not step: the deck's own drums fade as the effect comes in.
        assert!((out[0] - 0.0).abs() < 0.01 && out[..300].windows(2).all(|w| (w[1] - w[0]).abs() < 0.02), "{:?}", &out[..8]);
        e.release = None;
        step(&mut e, &mut pos, 44100);
        // ECHO (rekordbox's `ReleaseEcho`): the beat after the press plays on and is taken in,
        // then it repeats at 0.7, 0.49, ...
        e.release = Some((1, 1.0));
        let (out, _) = step(&mut e, &mut pos, 44100);
        assert!((out[2000] - 0.5).abs() < 0.01 && (out[22050 + 2000] - 0.35).abs() < 0.01, "{} {}", out[2000], out[24050]);
        // Let go: the deck's drums are back, and after the fade nothing is added.
        e.release = None;
        let (out, cut) = step(&mut e, &mut pos, 4096);
        assert!(!cut && out[2000..].iter().all(|v| v.abs() < 1e-6));
        assert!(!e.fx_active());
    }

    #[test]
    fn capture_length_in_frames_for_each_beat_count() {
        for bpm in [87.5f64, 120.0, 128.0, 140.03, 174.0] {
            let beats: Vec<(f64, f64)> = (0..2000).map(|i| (0.123 + i as f64 * 60.0 / bpm, bpm)).collect();
            for sr in [44100u32, 48000] {
                for len in CAPTURE_LENGTHS {
                    // From a place between two beats: the start is not moved onto a beat.
                    let start = 30.0 + 0.2;
                    let (a, b, got_bpm, first) = capture_range(&beats, start, len as usize, 600.0);
                    assert_eq!(a, start);
                    assert_eq!(got_bpm, bpm);
                    let frames = capture_frames(a, b, sr);
                    let want = len as f64 * 60.0 / bpm * sr as f64;
                    assert!((frames as f64 - want).abs() <= 1.0, "{bpm} BPM, {len} beats at {sr}: {frames} frames, want {want}");
                    assert_eq!(loop_beats(frames, sr, bpm as f32), len);
                    assert!(first > 0.0 && first < 60.0 / bpm);
                }
            }
        }
        // The grid ends before the range does: to the end of the track.
        let beats: Vec<(f64, f64)> = (0..40).map(|i| (i as f64 * 0.5, 120.0)).collect();
        let (a, b, _, _) = capture_range(&beats, 15.0, 16, 21.0);
        assert_eq!((a, b), (15.0, 21.0));
        // No grid: 120 BPM.
        let (a, b, bpm, _) = capture_range(&[], 2.0, 4, 100.0);
        assert_eq!((a, b, bpm), (2.0, 4.0, 120.0));
    }

    #[test]
    fn settings_survive_a_round_trip_in_rekordboxs_shape() {
        let mut s = Settings::default();
        s.decks[0].bank = 1;
        s.decks[0].multi = true;
        s.decks[0].fx_type = 2;
        s.decks[0].fx_beats = [(3, 4), (1, 16), (32, 1)];
        s.decks[0].slots[0] = Some(SlotInfo { id: 42, name: "Kick & <Snare> \"1\"".into(), url: "/Music/a b/loop.wav".into(), length: 88200, samplingrate: 44100, gain: 72.0, bpm: 123.0, first_beat: 0.0 });
        s.decks[0].slots[5] = Some(SlotInfo { id: 0, name: "Title_drum_capture".into(), url: "/tmp/x_drum_capture.wav".into(), length: 400, samplingrate: 48000, gain: 0.0, bpm: 128.5, first_beat: 0.125 });
        s.decks[3].slots[7] = Some(SlotInfo { id: 7, name: "B4".into(), url: "/b4.wav".into(), length: 1, samplingrate: 44100, gain: 100.0, bpm: 0.0, first_beat: 0.0 });
        let mut p = Preset { id: 1_790_000_000_123, name: "Untitled Preset".into(), tracks: Default::default() };
        p.tracks[0] = Some(PresetTrack { id: 42, name: "Kick".into(), url: "/Music/a b/loop.wav".into(), bpm: 123.0, first_beat: 0.0 });
        p.tracks[7] = Some(PresetTrack { id: 0, name: "House1".into(), url: "/data/House1.wav".into(), bpm: 123.0, first_beat: 0.0 });
        s.presets.push(p);
        s.presets.push(Preset { id: 5, name: "Empty".into(), tracks: Default::default() });
        let xml = s.to_xml();
        assert_eq!(Settings::from_xml(&xml), s);
        // rekordbox's keys and elements.
        for key in ["<PROPERTIES>", "<VALUE name=\"visibleBank1\" val=\"1\"/>", "<VALUE name=\"singleMulti1\" val=\"1\"/>", "<VALUE name=\"fxType1\" val=\"2\"/>", "<VALUE name=\"SamplerSet1\">", "<SamplerSet1>", "<VALUE name=\"FxSet4\">", "<Beat idx=\"0\" numerator=\"3\" denominator=\"4\"/>", "<VALUE name=\"Presets\">", "<Preset id=\"1790000000123\" name=\"Untitled Preset\">", "<Track idx=\"5\" id=\"0\" name=\"Title_drum_capture\""] {
            assert!(xml.contains(key), "{key} not in\n{xml}");
        }
        // A new file: SINGLE, bank A, VINYL BRAKE, beats 1/2, 1/4 and 1.
        let d = Settings::from_xml("");
        assert_eq!(d, Settings::default());
        assert_eq!((d.decks[1].bank, d.decks[1].multi, d.decks[1].fx_type, d.decks[1].fx_beats), (0, false, 0, [(1, 2), (1, 4), (1, 1)]));
        // A file as rekordbox writes it (no bpm or firstbeat attributes) still loads.
        let rb = "<PROPERTIES><VALUE name=\"SamplerSet2\"><SamplerSet2><Track idx=\"4\" id=\"9\" name=\"n\" url=\"/u.wav\" length=\"10\" samplingrate=\"44100\" gain=\"50.0\"/></SamplerSet2></VALUE><VALUE name=\"singleMulti2\" val=\"1\"/></PROPERTIES>";
        let d = Settings::from_xml(rb);
        assert_eq!(d.decks[1].slots[4].as_ref().map(|s| (s.id, s.gain, s.bpm)), Some((9, 50.0, 0.0)));
        assert!(d.decks[1].multi);
    }

    #[test]
    fn deck_gain_scales_playing_and_new_drum_loops_without_changing_slot_gains() {
        let mut engine = Engine::new(0);
        engine.single = false;
        let mut first = drum_loop(120.0, 1, 44100);
        first.gain = 0.25;
        engine.set_clip(0, 0, Some(first));
        assert!(engine.launch(0, 0));

        // Away from the click each channel of each loop contains 0.25.
        let mut pos = 0.0;
        let mut volume = |e: &mut Engine| *run(e, &[], &mut pos, 1.0, 1024, false).last().unwrap();
        assert!((volume(&mut engine) - 0.25 * 0.25).abs() < 1e-5);
        engine.gain = knob_gain(75.0);
        assert!((volume(&mut engine) - 0.25 * 0.25 * knob_gain(75.0)).abs() < 1e-5);

        let mut later = drum_loop(120.0, 1, 44100);
        later.gain = 0.75;
        engine.set_clip(1, 1, Some(later));
        assert!(engine.launch(1, 1));
        assert!((volume(&mut engine) - 0.25 * knob_gain(75.0)).abs() < 1e-5);
        engine.gain = knob_gain(50.0);
        let center = volume(&mut engine);
        assert!((center - 0.25 * knob_gain(50.0)).abs() < 1e-5);
        // One 14-bit MIDI step above center must produce a distinct output.
        let next_step = 50.0 + 50.0 / 8191.0;
        engine.gain = knob_gain(next_step);
        let next = volume(&mut engine);
        assert!(next > center);
        assert!((next - 0.25 * knob_gain(next_step)).abs() < 1e-6);
        assert_eq!(engine.clip_mut(0, 0).unwrap().gain, 0.25);
        assert_eq!(engine.clip_mut(1, 1).unwrap().gain, 0.75);
    }

    #[test]
    fn gain_beats_and_labels_follow_rekordbox() {
        // 50 is 0 dB, 100 about +9 dB, the left end is as good as silent.
        assert!((20.0 * knob_gain(50.0).log10()).abs() < 0.1);
        assert!((20.0 * knob_gain(100.0).log10() - 9.0).abs() < 0.3);
        assert!(knob_gain(0.0) < 0.0004);
        assert_eq!(knob_text(0.0), "-∞");
        assert_eq!(knob_text(50.0), "0.0");
        // The beat table: 1/16 .. 32, stopping at both ends.
        assert_eq!(beat_step((1, 16), -1), (1, 16));
        assert_eq!(beat_step((1, 2), 1), (3, 4));
        assert_eq!(beat_step((3, 4), 1), (1, 1));
        assert_eq!(beat_step((32, 1), 1), (32, 1));
        assert_eq!([beat_text((3, 4)), beat_text((1, 1)), beat_text((16, 1)), beat_text((1, 16))], ["3/4", "1", "16", "1/16"]);
        assert_eq!(loop_beats(87258, 44100, 123.0), 4);
        assert_eq!(loop_beats(100, 44100, 0.0), 0);
        // The start of a clip whose first beat is late: just after the start, or one beat
        // before the end, whichever is nearer a beat.
        let mut c = drum_loop(120.0, 4, 44100);
        c.first_beat = 0.1;
        assert!((c.geometry().2 - 0.2).abs() < 1e-6);
        c.first_beat = 0.4;
        assert!((c.geometry().2 - 3.8).abs() < 1e-6);
    }

    #[test]
    fn the_factory_pack_list_is_read_in_the_kits_order() {
        let xml = "<?xml version=\"1.0\"?><GROOVE_CIRCUIT_FACTORY_SAMPLES Version=\"1.0.0\"><TRACKS num=\"2\"><TRACK id=\"1\" location=\"Kit/One.wav\" mode=\"1\" bpm=\"123.00\" /><TRACK id=\"2\" location=\"Kit/Two.wav\" mode=\"1\" bpm=\"125.00\" /></TRACKS><LISTS><LIST id=\"1\" name=\"x\"><TRACK key=\"1\" /></LIST></LISTS></GROOVE_CIRCUIT_FACTORY_SAMPLES>";
        assert_eq!(parse_pack_xml(xml), vec![("Kit/One.wav".to_string(), 123.0), ("Kit/Two.wav".to_string(), 125.0)]);
    }

    /// `cargo test --release factory_pack -- --ignored --nocapture`: unpack the installed
    /// rekordbox's factory pack into a temporary folder (needs rekordbox 7 in Applications).
    #[test]
    #[ignore]
    fn factory_pack_unpacks_from_the_installed_rekordbox() {
        let dir = std::env::temp_dir().join(format!("recordbox-gcpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let got = import_factory(std::path::Path::new(FACTORY_PACK), &dir).unwrap();
        eprintln!("{got:?}");
        assert_eq!(got.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), ["House1", "House2", "Techno1", "Breaks1", "House3", "House4", "Breaks2", "Breaks3"]);
        assert_eq!(got.iter().map(|g| g.2).collect::<Vec<_>>(), [123.0, 125.0, 136.0, 130.0, 140.0, 150.0, 140.0, 121.0]);
        for (name, p, bpm) in &got {
            let a = crate::audio::decode_file(&p.to_string_lossy()).unwrap();
            eprintln!("{name}: {} frames at {} Hz, {} beats at {bpm}", a.frames(), a.sr, loop_beats(a.frames(), a.sr, *bpm));
            assert!([4, 16, 32].contains(&loop_beats(a.frames(), a.sr, *bpm)));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
