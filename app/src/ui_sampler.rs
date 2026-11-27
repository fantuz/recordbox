//! The sampler panel of PERFORMANCE mode, after rekordbox 7.2.19's `SamplerControlPanel`
//! (118 pt high under the mixer's bottom row). Positions are those of its `resized`
//! functions (`SamplerControlPanel::resized` 0x100b07668, `SamplerControlComp::resized`
//! 0x100aefd64, `SequencerComponent::resized` 0x1003c0380) and of reference
//! performance/08-sampler.
//!
//! 16 slots per bank, four banks. A slot is loaded by dropping a track on it. The sequencer
//! strip at the right of the bottom line works the engine's sequencer (`sequencer.rs`): its
//! icons are drawn from the skin's SVGs (`sampler_recBtn`, `sampler_playBtn`, `metronome_*`,
//! `LengthRoll_Arrow*`) and the beat counter from `p13015..17_beatcounter*.png`.

use crate::app::*;
use crate::sampler::{self, Clip, MODE_GATE, MODE_LOOP, PADS};
use crate::sequencer::{Pattern, PATTERNS};
use crate::theme::*;
use crate::ui_player::{lighten, tip};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const SAMPLER_H: f32 = 118.0;
const SLOT_H: f32 = 44.0;
const PANEL: Color32 = hex(0x1E1E1E);
const OFF: Color32 = hex(0x323232);

/// One loaded slot, as it is kept between sessions (rekordbox keeps the same values in its
/// `SamplerSettings` file: loop, gate, mt, sync, gain 0..100, offset in ms).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SlotPrefs {
    pub track: i64,
    pub path: String,
    pub name: String,
    pub looped: bool,
    pub gate: bool,
    pub mt: bool,
    pub sync: bool,
    pub gain: f32,
    pub offset_ms: f32,
    pub bpm: f32,
}

impl Default for SlotPrefs {
    fn default() -> SlotPrefs {
        SlotPrefs { track: 0, path: String::new(), name: String::new(), looped: false, gate: false, mt: true, sync: true, gain: 50.0, offset_ms: 0.0, bpm: 0.0 }
    }
}

impl SlotPrefs {
    fn mode(&self) -> u8 {
        (if self.looped { MODE_LOOP } else { 0 }) | (if self.gate { MODE_GATE } else { 0 })
    }
}

/// The sequencer, as it is kept between sessions. rekordbox keeps each saved pattern in a
/// MIDI file (`Sequences/SEQ.<n>.mid`) and these settings in its setting file; here they sit
/// with the sampler's other settings.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SeqPrefs {
    /// The eight pattern slots: what SAVE put there.
    pub patterns: Vec<Option<Pattern>>,
    pub current: u8,
    pub metronome: bool,
    /// rekordbox's MetronomeVolume: 0..2.
    pub metronome_vol: u8,
    /// rekordbox's SequencerQuantizeStatus (on) and SequencerQuantize (1..6 = 1/32 .. 1/1 note;
    /// 4 = one beat). They take effect while the sampler's Q is on.
    pub quantize_on: bool,
    pub quantize: u8,
    /// The patterns' names (empty: rekordbox's default, "PATTERN n").
    pub names: Vec<String>,
    /// rekordbox's SequenceLoadAutoPlay: "Load the sequence and automatically play."
    pub load_autoplay: bool,
}

impl Default for SeqPrefs {
    fn default() -> SeqPrefs {
        SeqPrefs { patterns: vec![None; PATTERNS], current: 0, metronome: false, metronome_vol: 1, quantize_on: true, quantize: 4, names: vec![String::new(); PATTERNS], load_autoplay: true }
    }
}

impl SeqPrefs {
    /// A pattern's name as shown.
    pub fn name(&self, slot: usize) -> String {
        match self.names.get(slot).map(|n| n.trim()).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => format!("PATTERN {}", slot + 1),
        }
    }
}

/// rekordbox's click (read where rekordbox is installed; never changed). Without it the
/// engine's own short click is used.
const CLICK_FILE: &str = "/Applications/rekordbox 7/rekordbox.app/Contents/Resources/binary/Click Sound 01 Electronic.wav";

/// rekordbox's defaults: GAIN 50, volume 100, 120 BPM, bank 1, Q and BPM SYNC off.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SamplerPrefs {
    /// 64 slots: bank * 16 + pad.
    pub slots: Vec<Option<SlotPrefs>>,
    pub gain: f32,
    pub volume: f32,
    pub bpm: f32,
    pub bank: u8,
    pub quantize: bool,
    pub sync: bool,
    pub cue: bool,
    pub seq: SeqPrefs,
    /// The colour chosen for a sample: file path -> rekordbox pad colour code (rekordbox
    /// keeps it per content in its database, `setSamplerCustomColor`).
    pub colors: std::collections::HashMap<String, u8>,
    /// rekordbox's SamplerSlotIs16 (SLOT 16 / 8), SamplerLoadLockAtLoadedSlot ("Do not
    /// overwrite the loaded Slot."), ShouldLoopCaptureIfEmptySlot, VelocityCurve (1..4, default 3),
    /// EffectorCFxOnSampler and the output the sampler deck goes to (0 = master,
    /// 1 = headphones).
    pub slots16: bool,
    pub load_lock: bool,
    pub capture_empty: bool,
    pub velocity_curve: u8,
    pub cfx_on_sampler: bool,
    pub output: u8,
    /// rekordbox's SamplerSaveDirectory (empty: recordbox's own folder), SlicerCaptureSaveBank
    /// (1..4) and the "Disable" switch of Auto Panel Mode.
    pub capture_dir: String,
    pub slicer_bank: u8,
    pub auto_panel_disable: bool,
}

impl Default for SamplerPrefs {
    fn default() -> SamplerPrefs {
        SamplerPrefs { slots: vec![None; 64], gain: 50.0, volume: 100.0, bpm: 120.0, bank: 0, quantize: false, sync: false, cue: false, seq: SeqPrefs::default(), colors: Default::default(), slots16: true, load_lock: false, capture_empty: true, velocity_curve: 3, cfx_on_sampler: false, output: 0, capture_dir: String::new(), slicer_bank: 4, auto_panel_disable: false }
    }
}

type Loaded = Result<Arc<crate::audio::Decoded>, String>;

/// What the panel keeps while the app runs.
pub struct SamplerUi {
    restored: bool,
    loading: Vec<(usize, std::sync::mpsc::Receiver<Loaded>)>,
    /// Mini waveform of each loaded slot: peak per column (150 columns).
    wave: Vec<Option<Vec<u8>>>,
    /// The slot whose EDIT panel is open.
    edit: Option<usize>,
    mute: Vec<bool>,
    pad_db: [f32; PADS],
    /// BPM + / - held: direction, when it was pressed and how many repeats have fired.
    bpm_hold: Option<(f32, f64, u32)>,
    /// The pad whose PLAY button the pointer holds.
    held: Option<usize>,
    /// The sequencer's pad mode: 0 none, 1 MUTE, 2 ERASE (only while it plays).
    pub seq_mode: u8,
    /// MASTER: the sampler deck is the sync master (not kept between sessions, as in
    /// rekordbox).
    pub master: bool,
    /// The sampler's CFX COLOR knob (0.5 = off).
    pub cfx_color: f32,
    /// The slot being dragged to another slot.
    pub drag_slot: Option<usize>,
    /// The sequencer strip is being dragged (to a deck: Sequence Load).
    pub drag_seq: bool,
    /// The BPM field while a tempo is typed into it.
    bpm_edit: Option<String>,
    /// The pattern whose name is being typed.
    name_edit: Option<(usize, String)>,
    /// Files dropped from Finder, waiting for the pointer's place (the window gets no
    /// pointer positions while a file is dragged over it), and when they came.
    dropped: Option<(Vec<std::path::PathBuf>, f64)>,
    /// Was the master deck playing last frame (BPM SYNC starts and stops the sequencer with it).
    master_was_playing: bool,
    /// Where the decks are on screen (for dropping the sequencer strip on one).
    pub deck_rects: [Rect; 4],
    /// A sequence was loaded on a deck and should play once the deck has read it:
    /// (deck, track, frames waited).
    pub autoplay: Option<(u8, i64, u32)>,
}

impl SamplerUi {
    /// The slot's mute button is on.
    pub fn is_muted(&self, idx: usize) -> bool {
        self.mute.get(idx).copied().unwrap_or(false)
    }

    pub fn toggle_mute(&mut self, idx: usize) {
        if let Some(m) = self.mute.get_mut(idx) {
            *m = !*m;
        }
    }
    pub fn take_wave(&mut self, idx: usize) -> Option<Vec<u8>> {
        self.wave.get_mut(idx).and_then(|w| w.take())
    }
    pub fn put_wave(&mut self, idx: usize, w: Vec<u8>) {
        if let Some(slot) = self.wave.get_mut(idx) {
            *slot = Some(w);
        }
    }
    pub fn bpm_open(&self) -> bool {
        self.bpm_edit.is_some()
    }
    pub fn close_edit(&mut self) {
        self.edit = None;
    }
    /// The slot's sample has been read and can play.
    pub fn ready(&self, idx: usize) -> bool {
        self.wave.get(idx).is_some_and(|w| w.is_some())
    }

    pub fn new() -> SamplerUi {
        SamplerUi { restored: false, loading: Vec::new(), wave: vec![None; 64], edit: None, mute: vec![false; 64], pad_db: [-120.0; PADS], bpm_hold: None, held: None, seq_mode: 0, master: false, cfx_color: 0.5, drag_slot: None, drag_seq: false, bpm_edit: None, name_edit: None, dropped: None, master_was_playing: false, deck_rects: [Rect::NOTHING; 4], autoplay: None }
    }
}

const WAVE_COLS: usize = 150;

fn mini_wave(a: &crate::audio::Decoded) -> Vec<u8> {
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

fn decode_in_background(ui: &mut SamplerUi, idx: usize, path: String) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::audio::decode_file(&path).map(Arc::new).map_err(|e| e.to_string()));
    });
    ui.loading.retain(|(i, _)| *i != idx);
    ui.loading.push((idx, rx));
}

impl App {
    /// Load a library track into a sampler slot (bank * 16 + pad). The slot keeps its mode
    /// buttons when it was loaded before, as in rekordbox.
    pub fn sampler_load(&mut self, idx: usize, id: i64) {
        let Some(t) = self.track(id).cloned() else { return };
        if t.missing {
            self.toast_err(format!("The file for \"{}\" cannot be found.", t.display_title()));
            return;
        }
        if self.sampler_locked(idx) {
            return;
        }
        self.sampler_put(idx, id, t.path.clone(), t.display_title(), t.bpm as f32);
        self.sampler_apply_track_type(idx, id);
    }

    /// Load an audio file that need not be in the collection (a file dropped from Finder, a
    /// captured loop). `bpm`: its tempo when known (0: it plays at its own speed).
    pub fn sampler_load_file(&mut self, idx: usize, path: String, name: String, bpm: f32) {
        if self.sampler_locked(idx) {
            return;
        }
        // A file of the collection brings its title and tempo.
        if let Some(t) = self.db.track_by_path(&path).and_then(|id| self.track(id)).cloned() {
            self.sampler_put(idx, t.id, t.path.clone(), t.display_title(), t.bpm as f32);
        } else {
            self.sampler_put(idx, 0, path, name, bpm);
        }
    }

    fn sampler_put(&mut self, idx: usize, id: i64, path: String, name: String, bpm: f32) {
        if idx >= 64 {
            return;
        }
        if self.prefs.sampler.slots.len() < 64 {
            self.prefs.sampler.slots.resize(64, None);
        }
        let mut s = self.prefs.sampler.slots[idx].clone().unwrap_or_default();
        s.track = id;
        s.path = path.clone();
        s.name = name;
        s.bpm = bpm;
        s.offset_ms = 0.0;
        self.prefs.sampler.slots[idx] = Some(s);
        self.sampler_decode(idx, path);
        self.save_prefs();
    }

    /// Read a slot's file in the background; the slot is silent until it is there.
    pub fn sampler_decode(&mut self, idx: usize, path: String) {
        self.px.sampler.wave[idx] = None;
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            e.set_clip(idx % PADS, idx / PADS, None);
        }
        decode_in_background(&mut self.px.sampler, idx, path);
    }

    pub fn sampler_eject(&mut self, idx: usize) {
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            e.set_clip(idx % PADS, idx / PADS, None);
        }
        if let Some(s) = self.prefs.sampler.slots.get_mut(idx) {
            *s = None;
        }
        self.px.sampler.wave[idx] = None;
        self.px.sampler.mute[idx] = false;
        self.px.sampler.loading.retain(|(i, _)| *i != idx);
        if self.px.sampler.edit == Some(idx) {
            self.px.sampler.edit = None;
        }
        self.save_prefs();
    }

    /// Once per frame in PERFORMANCE mode: bring the saved slots back the first time, take in
    /// finished decodes, and hand the deck's settings to the audio engine.
    pub fn sampler_poll(&mut self) {
        if !self.px.sampler.restored {
            self.px.sampler.restored = true;
            if self.prefs.sampler.slots.len() != 64 {
                self.prefs.sampler.slots.resize(64, None);
            }
            sampler::SAMPLER.lock().get_or_insert_with(|| Box::new(sampler::Engine::new()));
            // The saved patterns, the pattern that was chosen, and the metronome's sound.
            let sq = &mut self.prefs.sampler.seq;
            sq.patterns.resize(PATTERNS, None);
            sq.current = sq.current.min(PATTERNS as u8 - 1);
            sq.metronome_vol = sq.metronome_vol.min(2);
            sq.quantize = sq.quantize.clamp(1, 6);
            sq.names.resize(PATTERNS, String::new());
            let click = if std::path::Path::new(CLICK_FILE).exists() { crate::audio::decode_file(CLICK_FILE).ok().map(Arc::new) } else { None };
            if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                for (i, p) in sq.patterns.iter().enumerate() {
                    if let Some(p) = p {
                        e.seq.set_pattern(i, p);
                    }
                }
                e.seq.select(sq.current as usize, 0.0);
                if let Some(c) = click {
                    e.set_click(c);
                }
            }
            let saved: Vec<(usize, String)> = self.prefs.sampler.slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, s.path.clone()))).collect();
            for (i, path) in saved {
                // A file that is gone leaves its slot empty (rekordbox removes it the same way).
                if std::path::Path::new(&path).exists() {
                    decode_in_background(&mut self.px.sampler, i, path);
                } else {
                    self.prefs.sampler.slots[i] = None;
                }
            }
        }
        self.sampler_autoplay_poll();
        let mut done = Vec::new();
        self.px.sampler.loading.retain(|(i, rx)| match rx.try_recv() {
            Ok(r) => {
                done.push((*i, r));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (i, r) in done {
            let Some(s) = self.prefs.sampler.slots[i].clone() else { continue };
            match r {
                Ok(a) => {
                    self.px.sampler.wave[i] = Some(mini_wave(&a));
                    let clip = Clip { audio: a, mode: s.mode(), gain: (s.gain / 50.0).clamp(0.0, 2.0), sync: s.sync, mt: s.mt, bpm: s.bpm, offset_ms: s.offset_ms, mute: self.px.sampler.mute[i] };
                    if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                        e.set_clip(i % PADS, i / PADS, Some(clip));
                    }
                }
                Err(e) => {
                    self.toast_err(format!("\"{}\" could not be loaded into the sampler: {e}", s.name));
                    self.prefs.sampler.slots[i] = None;
                }
            }
        }
        // BPM SYNC: the sampler follows the master deck's tempo (and, in the audio thread,
        // its beats). As the MASTER it keeps its own tempo and the decks follow it.
        let master = self.px.sampler.master;
        let follows = self.prefs.sampler.sync && !master;
        let mut bpm = self.prefs.sampler.bpm;
        if follows {
            let m = self.px.master.min(self.deck_count().saturating_sub(1));
            let b = self.with_deck(m, |a| if a.deck.track.is_some() { a.effective_bpm() } else { 0.0 });
            if b > 0.0 {
                bpm = b as f32;
            }
            // The sequencer starts and stops with the sync master (rekordbox's
            // `syncStartStopStatus`).
            let playing = self.player.decks[m as usize].lock().playing;
            if playing != self.px.sampler.master_was_playing {
                if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                    if playing {
                        e.follow = true;
                        e.seq_play();
                    } else {
                        e.seq_stop();
                    }
                }
            }
            self.px.sampler.master_was_playing = playing;
        } else {
            self.px.sampler.master_was_playing = false;
        }
        crate::audio::SAMPLER_MASTER.store(master && self.performance(), std::sync::atomic::Ordering::Relaxed);
        // CFX on the sampler (Preferences > Controller > Effect), only with the FX panel shown.
        let cfx = if self.prefs.sampler.cfx_on_sampler && self.prefs.perf_fx { self.px.sampler.cfx_color } else { 0.5 };
        if let Some(r) = crate::fx::RACK.lock().as_mut() {
            let (ck, cc, cp) = crate::ui_fx::cfx_for(self, crate::fx::SAMPLER);
            r.sampler_cfx_set = (ck, if cfx == 0.5 { 0.5 } else { cc }, cp);
        }
        let sp = &self.prefs.sampler;
        let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
        let cue_gain = if sp.cue && hp_on { crate::mixer::cue_master_balance(1.0 - self.px.hp_mix) * crate::mixer::headphones_level(self.px.hp_level) } else { 0.0 };
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            // GAIN: rekordbox's trim table, unity at the centre; the fader: the channel fader's law.
            e.gain = crate::mixer::trim_gain(sp.gain / 100.0);
            e.volume = crate::mixer::channel_fader_gain(sp.volume / 100.0, self.prefs.ch_fader_curve);
            e.cue_gain = cue_gain;
            e.bpm = bpm.clamp(40.0, 499.0);
            e.quantize = sp.quantize;
            e.follow = follows;
            e.route = if sp.output == 1 && hp_on { 1 } else { 0 };
            // The record quantize follows Q (rekordbox: `SequencerQuantize` while Q is on and
            // `SequencerQuantizeStatus` is 2, else off).
            e.seq.rec_unit = if sp.quantize && sp.seq.quantize_on { crate::sequencer::quantize_unit(sp.seq.quantize) } else { 0.0 };
            e.seq.metronome = sp.seq.metronome;
            e.click_gain = sampler::METRONOME_GAINS[sp.seq.metronome_vol.min(2) as usize];
        }
    }

    /// The BPM the sampler deck shows: its own, or the master deck's while BPM SYNC is on.
    fn sampler_shown_bpm(&mut self) -> f32 {
        if self.prefs.sampler.sync && !self.px.sampler.master {
            let m = self.px.master.min(self.deck_count().saturating_sub(1));
            let b = self.with_deck(m, |a| if a.deck.track.is_some() { a.effective_bpm() } else { 0.0 });
            if b > 0.0 {
                return b as f32;
            }
        }
        self.prefs.sampler.bpm
    }

    pub fn sampler_push_slot(&mut self, idx: usize) {
        let Some(s) = self.prefs.sampler.slots[idx].clone() else { return };
        let mute = self.px.sampler.mute[idx];
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            if let Some(c) = e.clip_mut(idx % PADS, idx / PADS) {
                c.mode = s.mode();
                c.gain = (s.gain / 50.0).clamp(0.0, 2.0);
                c.sync = s.sync;
                c.mt = s.mt;
                c.bpm = s.bpm;
                c.offset_ms = s.offset_ms;
                c.mute = mute;
            }
        }
        self.save_prefs();
    }
}

/// A small text button without a box (Q, BPM SYNC, MASTER, MUTE, ERASE): grey, blue when on.
fn text_button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, on: bool, enabled: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let col = if !enabled { hex(0x424242) } else if on { BLUE } else if resp.hovered() { WHITE } else { hex(0x8C8C8C) };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, caps(9.5), col);
    resp
}

fn box_button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, on: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(r, 2.0, if on { BLUE } else if resp.hovered() { lighten(BTN) } else { BTN });
    if !label.is_empty() {
        ui.painter().text(r.center(), Align2::CENTER_CENTER, label, caps(8.0), WHITE);
    }
    resp
}

/// The panel. `r`: 118 pt high, the full width of the window.
pub fn panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    p.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 1.0)), 0.0, Color32::BLACK);
    let (x0, y0, w, h) = (r.min.x, r.min.y, r.width(), r.height());
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x0 + x, y0 + y), vec2(w, h));
    let bank = app.prefs.sampler.bank.min(3) as usize;
    let slot_w = ((w - 80.0) / 8.0).floor().max(146.0);
    let dt = ui.input(|i| i.stable_dt).min(0.1);

    // Levels and play positions of the visible bank, and the sequencer, read once.
    let mut states = [(false, 0.0f32); PADS];
    let mut sv = SeqView { playing: false, rec: false, pos: 0.0, beats: 4.0, bars: 1, cur: 0, reserved: None, dirty: false, has_events: false };
    if let Some(e) = sampler::SAMPLER.lock().as_mut() {
        let q = &e.seq;
        let cur = q.current();
        sv = SeqView { playing: q.playing, rec: q.rec, pos: q.pos(), beats: q.beats(), bars: q.pattern(cur).bars, cur, reserved: q.reserved(), dirty: q.dirty(cur), has_events: !q.pattern(cur).events.is_empty() };
        for (pad, st) in states.iter_mut().enumerate() {
            *st = e.slot_state(pad, bank);
            let pk = e.take_pad_peak(pad);
            let db = if pk > 0.0 { 20.0 * pk.log10() } else { -120.0 };
            app.px.sampler.pad_db[pad] = crate::mixer::meter_fall(app.px.sampler.pad_db[pad], db, dt);
        }
    }
    if states.iter().any(|s| s.0) || app.px.sampler.pad_db.iter().any(|d| *d > -100.0) || !app.px.sampler.loading.is_empty() || sv.playing {
        ui.ctx().request_repaint();
    }
    // MUTE and ERASE only last while the sequencer plays; EDIT is not available then.
    if !sv.playing {
        app.px.sampler.seq_mode = 0;
    } else {
        app.px.sampler.edit = None;
    }
    let seq_mode = app.px.sampler.seq_mode;
    // A pattern change that waited for its bar has happened: remember the pattern.
    if app.prefs.sampler.seq.current as usize != sv.cur {
        app.prefs.sampler.seq.current = sv.cur as u8;
        app.save_prefs();
    }

    // ---- the 16 slots: pads 1-4 and 5-8 on the left, 9-12 and 13-16 on the right
    let slot_rect = |pad: usize| {
        let (col, row) = (pad & 3, (pad & 7) >> 2);
        let x = if pad < 8 { col as f32 * slot_w + 4.0 } else { w + (col as f32 - 4.0) * slot_w - 4.0 };
        at(x, SLOT_H * row as f32 + 4.0, slot_w, SLOT_H)
    };
    let shift = ui.input(|i| i.modifiers.shift);
    let released = ui.input(|i| i.pointer.any_released());
    let pointer = ui.ctx().pointer_latest_pos();
    let pads_on = app.sampler_pads();
    let now = ui.input(|i| i.time);

    // Files dropped from Finder. The window is told no pointer position while a file is
    // dragged over it, so the files wait for the pointer's next move (it comes at once when
    // the mouse is let go): on a slot the first audio file loads there, anywhere else the
    // files are imported as the browser does it.
    let files: Vec<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if !files.is_empty() {
        ui.ctx().input_mut(|i| i.raw.dropped_files.clear());
        app.px.sampler.dropped = Some((files, now));
    }
    if let Some((files, t0)) = app.px.sampler.dropped.clone() {
        let moved = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_))));
        if moved || now - t0 > 0.6 {
            app.px.sampler.dropped = None;
            let slot = pointer.filter(|_| moved).and_then(|pp| (0..pads_on).find(|pad| slot_rect(*pad).contains(pp)));
            match slot {
                Some(pad) => match crate::importer::collect_audio(&files).into_iter().find(|f| f.is_file()) {
                    Some(f) => {
                        let name = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        app.sampler_load_file(bank * PADS + pad, f.to_string_lossy().to_string(), name, 0.0);
                    }
                    None => app.toast_err("This file cannot be loaded into the sampler."),
                },
                None if !app.ext.in_sub => app.import_paths(files, app.is_manual_playlist()),
                None => {}
            }
        } else {
            ui.ctx().request_repaint();
        }
    }
    let mut move_to: Option<(usize, usize)> = None;
    for pad in 0..PADS {
        let idx = bank * PADS + pad;
        let sr = slot_rect(pad);
        let (sx, sy, sw) = (sr.min.x, sr.min.y, sr.width());
        let s_at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(sx + x, sy + y), vec2(w, h));
        p.rect_filled(sr.shrink(4.0), 0.0, Color32::BLACK);
        if pad >= pads_on {
            // SLOT 8: pads 9 to 16 are out of use (the bank still holds their samples).
            p.rect_filled(sr.shrink(4.0), 0.0, hex(0x141414));
            continue;
        }
        let slot = app.prefs.sampler.slots.get(idx).cloned().flatten();
        if slot.is_none() {
            // An empty slot: right-click to capture a deck's loop into it.
            let bg = ui.interact(sr.shrink(4.0), ui.id().with(("smp_empty", idx)), Sense::click());
            let decks = app.deck_count();
            bg.context_menu(|ui| {
                for d in 0..decks {
                    let looping = app.player.decks[d as usize].lock().loop_range.is_some();
                    if ui.add_enabled(looping, egui::Button::new(format!("Capture the loop of DECK {}", d + 1))).on_disabled_hover_text("Invalid operation. Capture is available during Loop or Slicer performance.").clicked() {
                        app.sampler_capture_loop(d, Some(idx));
                        ui.close_menu();
                    }
                }
            });
        }
        let loading = app.px.sampler.loading.iter().any(|(i, _)| *i == idx);
        let (playing, frac) = states[pad];
        let hovered = pointer.is_some_and(|pp| sr.contains(pp));

        // Level meter (4 x 32) at the right edge.
        let mr = s_at(sw - 11.0, 6.0, 4.0, 32.0);
        p.rect_filled(mr, 0.0, OFF);
        let lv = ((app.px.sampler.pad_db[pad] + 40.0) / 40.0).clamp(0.0, 1.0);
        if lv > 0.0 {
            p.rect_filled(Rect::from_min_max(pos2(mr.min.x, mr.max.y - mr.height() * lv), mr.max), 0.0, hex(0x1EC81E));
        }

        // Mini waveform (w - 29 by 16) with the play position.
        let wr = s_at(6.0, 6.0, sw - 29.0, 16.0);
        if let Some(cols) = app.px.sampler.wave[idx].as_ref() {
            let mut bars = crate::ui_kit::Bars::new();
            let n = wr.width().max(1.0) as usize;
            for i in 0..n {
                let v = cols[(i * cols.len() / n).min(cols.len() - 1)] as f32 / 255.0;
                let hh = (v * wr.height() / 2.0).max(0.5);
                let x = wr.min.x + i as f32;
                let done = playing && (i as f32 / n as f32) < frac;
                bars.rect(Rect::from_min_max(pos2(x, wr.center().y - hh), pos2(x + 1.0, wr.center().y + hh)), if done { hex(0x5A5A5A) } else { hex(0xC8C8C8) });
            }
            bars.paint(&p);
            if playing {
                let x = wr.min.x + wr.width() * frac.clamp(0.0, 1.0);
                p.rect_filled(Rect::from_min_max(pos2(x - 0.5, wr.min.y - 1.0), pos2(x + 0.5, wr.max.y + 1.0)), 0.0, hex(0xE61E1E));
            }
        } else if loading {
            p.text(wr.left_center(), Align2::LEFT_CENTER, "Loading…", FontId::proportional(9.5), DIM);
        }

        // PLAY (26 x 14): fires on mouse-down; Shift + PLAY stops; the release stops gate modes.
        // It has the sample's colour (rekordbox's pad colour at alpha 0.8; full while it plays).
        let pr = s_at(6.0, 23.0, 26.0, 14.0);
        let loaded = slot.is_some() && app.px.sampler.wave[idx].is_some();
        // The waveform is the handle for dragging a sample to another slot: only when it is
        // loaded, not playing, and the sequencer's REC is off.
        if loaded && !playing && !sv.rec {
            let dresp = ui.interact(wr, ui.id().with(("smp_drag", idx)), Sense::drag());
            if dresp.drag_started() {
                app.px.sampler.drag_slot = Some(idx);
            }
        }
        let presp = ui.interact(pr, ui.id().with(("smp_play", idx)), Sense::click_and_drag());
        let code = app.sampler_color_code(idx);
        let col = crate::sampler_ext::pad_color(code);
        p.rect_filled(pr, 2.0, if !loaded { OFF } else if playing { hex(crate::sampler_ext::PAD_COLORS[code as usize]) } else if presp.hovered() { lighten(col) } else { col });
        if loaded {
            let c = pr.center();
            p.add(egui::Shape::convex_polygon(vec![c + vec2(-3.0, -4.0), c + vec2(4.0, 0.0), c + vec2(-3.0, 4.0)], WHITE, Stroke::NONE));
            if crate::ui_player::press_fired(ui, &presp) {
                // (With the sequencer's REC armed the engine starts the pattern on this press
                // and records it.)
                app.sampler_pad_down(pad, 1.0, shift, true);
                app.px.sampler.held = Some(pad);
            }
            // Right- or Ctrl-click: Reset and the 16 colours of rekordbox's `SamplerColorPicker`.
            presp.context_menu(|ui| {
                if ui.button("Reset").clicked() {
                    app.sampler_set_color(idx, None);
                    ui.close_menu();
                }
                let (grid, _) = ui.allocate_exact_size(vec2(4.0 * 20.0 - 2.0, 4.0 * 20.0 - 2.0), Sense::hover());
                for (k, c) in crate::sampler_ext::PICKER.iter().enumerate() {
                    let rr = Rect::from_min_size(grid.min + vec2((k % 4) as f32 * 20.0, (k / 4) as f32 * 20.0), vec2(18.0, 18.0));
                    let r = ui.interact(rr, ui.id().with(("smp_col", idx, k)), Sense::click());
                    ui.painter().rect_filled(rr, 0.0, crate::sampler_ext::pad_color(*c));
                    if *c == code || r.hovered() {
                        ui.painter().rect_stroke(rr, 0.0, Stroke::new(1.0_f32, WHITE), egui::StrokeKind::Inside);
                    }
                    if r.clicked() {
                        app.sampler_set_color(idx, Some(*c));
                        ui.close_menu();
                    }
                }
            });
        }
        if app.px.sampler.held == Some(pad) && released {
            if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                e.stop(pad, bank, false);
            }
            app.px.sampler.held = None;
        }
        let _ = tip(presp, app, "Play the sample. Shift + click stops it.");

        if let Some(s) = &slot {
            let editing = app.px.sampler.edit == Some(idx);
            let tools = hovered || editing;
            // In the sequencer's MUTE mode every slot shows its mute button, in ERASE mode
            // its erase button (where EDIT is otherwise).
            let (mute_mode, erase_mode) = (seq_mode == 1, seq_mode == 2);
            // Title (from x 38), shortened while the slot's buttons show.
            let tx = if tools { 74.0 } else if mute_mode { 54.0 } else { 38.0 };
            let tw = sw - 53.0 - (tx - 38.0) - if tools || erase_mode { 34.0 } else { 0.0 };
            let clip = p.with_clip_rect(s_at(tx, 24.0, tw.max(10.0), 14.0));
            clip.text(pos2(sx + tx, sy + 31.0), Align2::LEFT_CENTER, &s.name, FontId::proportional(10.5), WHITE);
            if tools || mute_mode {
                // Mute (16 x 14).
                let muted = app.px.sampler.mute[idx];
                let mresp = box_button(ui, s_at(34.0, 23.0, 16.0, 14.0), ("smp_mute", idx), "M", muted);
                if tip(mresp, app, "Mute").clicked() {
                    app.px.sampler.toggle_mute(idx);
                    app.sampler_push_slot(idx);
                }
            }
            if erase_mode {
                // Erase (30 x 14, `samplerSlot_btnErase`): the slot's notes leave the pattern.
                let er = s_at(sw - 47.0, 23.0, 30.0, 14.0);
                let eresp = ui.interact(er, ui.id().with(("smp_seq_erase", idx)), Sense::click());
                let down = eresp.is_pointer_button_down_on();
                p.rect_filled(er, 2.0, if down { hex(0x007DE1) } else { OFF });
                let (c, xc) = (er.center(), if down { WHITE } else { DIM });
                p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], Stroke::new(1.4_f32, xc));
                p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], Stroke::new(1.4_f32, xc));
                if tip(eresp, app, "Erase button:\nErase Sampler sequence per Sampler Slot.").clicked() {
                    if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                        e.seq.erase_track(pad);
                    }
                }
            }
            if tools {
                // Eject (16 x 14) and EDIT (30 x 14).
                let er = s_at(54.0, 23.0, 16.0, 14.0);
                let eresp = box_button(ui, er, ("smp_eject", idx), "", false);
                let c = er.center();
                p.add(egui::Shape::convex_polygon(vec![c + vec2(-4.0, 0.5), c + vec2(0.0, -4.0), c + vec2(4.0, 0.5)], WHITE, Stroke::NONE));
                p.rect_filled(Rect::from_center_size(c + vec2(0.0, 3.0), vec2(8.0, 1.5)), 0.0, WHITE);
                if tip(eresp, app, "Eject").clicked() {
                    app.sampler_eject(idx);
                    continue;
                }
                if !erase_mode {
                    let dresp = box_button(ui, s_at(sw - 47.0, 23.0, 30.0, 14.0), ("smp_edit", idx), "EDIT", editing);
                    if tip(dresp, app, "EDIT").clicked() {
                        if sv.playing {
                            app.toast_err("Invalid operation. EDIT is not available during sequencer playback/recording.");
                        } else {
                            app.px.sampler.edit = if editing { None } else { Some(idx) };
                        }
                    }
                }
            }
        }

        // A track dragged from the browser or from a deck loads here.
        if let Some(ids) = app.drag.clone() {
            if hovered {
                p.rect_stroke(sr.shrink(3.0), 0.0, Stroke::new(2.0_f32, BLUE), egui::StrokeKind::Inside);
                if released {
                    if let Some(id) = ids.first() {
                        app.sampler_load(idx, *id);
                    }
                }
            }
        }
        // Sampler list rows dragged here (sampler_lib.rs).
        crate::ui_sampler_lib::note_slot(app, idx, sr);
        if app.mixx.lib.drag.is_some() && app.drag.is_none() && hovered {
            p.rect_stroke(sr.shrink(3.0), 0.0, Stroke::new(2.0_f32, BLUE), egui::StrokeKind::Inside);
            if released {
                app.smplib_drop_on_slot(idx);
            }
        }
        // Another slot's sample dragged here moves here.
        if let Some(from) = app.px.sampler.drag_slot.filter(|f| *f != idx) {
            if hovered {
                p.rect_stroke(sr.shrink(3.0), 0.0, Stroke::new(2.0_f32, BLUE), egui::StrokeKind::Inside);
                if released {
                    move_to = Some((from, idx));
                }
            }
        }
    }
    if let Some(from) = app.px.sampler.drag_slot {
        if let (Some(pp), Some(s)) = (pointer, app.prefs.sampler.slots.get(from).cloned().flatten()) {
            let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("smp_dragl")));
            let rr = Rect::from_min_size(pp + vec2(12.0, 6.0), vec2(8.0 + s.name.chars().count().min(40) as f32 * 6.5, 18.0));
            lp.rect_filled(rr, 3.0, app.sampler_color(from));
            lp.with_clip_rect(rr).text(rr.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, &s.name, FontId::proportional(10.5), WHITE);
            ui.ctx().request_repaint();
        }
        if released || !ui.input(|i| i.pointer.any_down()) {
            app.px.sampler.drag_slot = None;
            if let Some((from, to)) = move_to {
                app.sampler_move(from, to);
            }
        }
    }

    // ---- the sampler deck's strip between the two groups of slots
    let c = ((w - 80.0) / 2.0).floor();
    let mut sp = app.prefs.sampler.clone();
    let mut changed = false;
    if let Some(v) = crate::ui_perf::knob(ui, pos2(x0 + c + 22.0, y0 + 24.0), "smp_gain", sp.gain / 100.0, 0.5, BLUE) {
        sp.gain = (v * 100.0).round();
        changed = true;
    }
    p.text(pos2(x0 + c + 22.0, y0 + 39.0), Align2::CENTER_CENTER, "GAIN", caps(8.0), hex(0x8C8C8C));
    let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
    let cr = at(c + 7.0, 50.0, 30.0, 14.0);
    let cresp = ui.interact(cr, ui.id().with("smp_cue"), Sense::click());
    p.rect_filled(cr, 2.0, if sp.cue { hex(0xFFA500) } else if cresp.hovered() { lighten(BTN) } else { BTN });
    p.text(cr.center(), Align2::CENTER_CENTER, "CUE", caps(8.0), if sp.cue { BG } else { hex(0x8C8C8C) });
    if cresp.on_hover_text(if hp_on { "Headphones CUE of the sampler" } else { "Headphones CUE of the sampler.\nNo headphones output is set: choose one in Preferences > Audio > Input/Output." }).clicked() {
        sp.cue = !sp.cue;
        changed = true;
    }
    // Volume fader (26 x 47).
    let fr = at(c + 46.0, 17.0, 26.0, 47.0);
    let (ft, fb, fx) = (fr.min.y + 3.5, fr.max.y - 2.0, fr.center().x);
    let fresp = ui.interact(fr, ui.id().with("smp_vol"), Sense::click_and_drag());
    if let Some(pp) = fresp.interact_pointer_pos().filter(|_| fresp.dragged() || fresp.is_pointer_button_down_on() || fresp.clicked()) {
        sp.volume = (((fb - pp.y) / (fb - ft)).clamp(0.0, 1.0) * 100.0).round();
        changed = true;
    }
    let hy = fb - sp.volume / 100.0 * (fb - ft);
    p.rect_filled(Rect::from_min_max(pos2(fx - 1.0, ft), pos2(fx + 1.0, fb)), 0.0, OFF);
    p.rect_filled(Rect::from_min_max(pos2(fx - 1.0, hy), pos2(fx + 1.0, fb)), 0.0, BLUE);
    p.rect_filled(Rect::from_center_size(pos2(fx, hy), vec2(24.0, 2.0)), 0.0, if fresp.hovered() || fresp.dragged() { WHITE } else { hex(0x646464) });

    // BPM (bold 18, 52 x 20): drag up or down, or the wheel; + and - under it.
    // (While it follows the master deck, + / - and typing are off; as the MASTER it is free.)
    let synced = sp.sync && !app.px.sampler.master;
    let shown = app.sampler_shown_bpm();
    let br = at(w / 2.0 - 26.0, 68.0, 52.0, 20.0);
    let typing = app.px.sampler.bpm_edit.is_some();
    let bresp = ui.interact(br, ui.id().with("smp_bpm"), if typing { Sense::hover() } else { Sense::click_and_drag() });
    if let Some(mut text) = app.px.sampler.bpm_edit.take() {
        // A tempo typed in (double-click): 40 to 499, else rekordbox's range message.
        let id = ui.id().with("smp_bpm_edit");
        let te = ui.put(br.expand2(vec2(6.0, 0.0)), egui::TextEdit::singleline(&mut text).id(id).font(bold(13.0)).horizontal_align(egui::Align::Center).margin(vec2(2.0, 2.0)));
        if !te.has_focus() && !te.lost_focus() {
            te.request_focus();
        }
        if te.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                match text.trim().replace(',', ".").parse::<f32>().ok().filter(|v| (40.0..=499.0).contains(v)) {
                    Some(v) => {
                        sp.bpm = (v * 100.0).round() / 100.0;
                        changed = true;
                    }
                    None => app.toast_err("BPM value should be between 40 and 499."),
                }
            }
        } else {
            app.px.sampler.bpm_edit = Some(text);
        }
    } else {
        let whole = format!("{}", shown.floor() as i32);
        let frac = format!(".{:02}", ((shown - shown.floor()) * 100.0).round().min(99.0) as i32);
        let g1 = p.layout_no_wrap(whole, bold(16.5), WHITE);
        let g2 = p.layout_no_wrap(frac, bold(12.5), WHITE);
        let tx = br.center().x - (g1.size().x + g2.size().x) / 2.0;
        let base = br.center().y + g1.size().y / 2.0;
        p.galley(pos2(tx, base - g1.size().y), g1.clone(), WHITE);
        p.galley(pos2(tx + g1.size().x, base - g2.size().y - 1.0), g2, WHITE);
        if bresp.double_clicked() && !synced {
            app.px.sampler.bpm_edit = Some(format!("{:.2}", sp.bpm));
        }
    }
    if !synced && !typing {
        if bresp.dragged() {
            sp.bpm = (sp.bpm - bresp.drag_delta().y * 0.2).clamp(40.0, 499.0);
            changed = true;
        }
        let wheel = if bresp.hovered() { ui.input(|i| i.raw_scroll_delta.y) } else { 0.0 };
        if wheel != 0.0 {
            sp.bpm = (sp.bpm + 0.01 * wheel.signum()).clamp(40.0, 499.0);
            changed = true;
        }
    }
    let _ = bresp.on_hover_text("Sampler BPM");
    // + / -: 0.01 on the press; held, it repeats every 100 ms after 500 ms: nine steps of
    // 0.01, twenty of 0.1, then 0.2.
    for (dir, x, label) in [(1.0f32, w / 2.0 - 22.0, "+"), (-1.0, w / 2.0 + 2.0, "-")] {
        let rr = at(x, h - 25.0, 20.0, 20.0);
        let resp = ui.interact(rr, ui.id().with(("smp_bpm_step", label)), Sense::click_and_drag());
        let col = if synced { hex(0x424242) } else if resp.hovered() { WHITE } else { hex(0xC8C8C8) };
        p.rect_filled(Rect::from_center_size(rr.center(), vec2(9.0, 1.2)), 0.0, col);
        if dir > 0.0 {
            p.rect_filled(Rect::from_center_size(rr.center(), vec2(1.2, 9.0)), 0.0, col);
        }
        if synced {
            continue;
        }
        if crate::ui_player::press_fired(ui, &resp) {
            sp.bpm = (((sp.bpm + 0.01 * dir) * 100.0).round() / 100.0).clamp(40.0, 499.0);
            changed = true;
            app.px.sampler.bpm_hold = Some((dir, now, 0));
        }
        if let Some((d, t0, ticks)) = app.px.sampler.bpm_hold.filter(|hold| hold.0 == dir) {
            if resp.is_pointer_button_down_on() {
                let due = ((now - t0 - 0.5) / 0.1).floor();
                if due >= ticks as f64 {
                    let t = ticks + 1;
                    let step = if t < 10 { 0.01 } else if t < 30 { 0.1 } else { 0.2 };
                    let snapped = if t < 10 { sp.bpm } else { (sp.bpm / step).round() * step };
                    sp.bpm = (((snapped + step * d) * 100.0).round() / 100.0).clamp(40.0, 499.0);
                    changed = true;
                    app.px.sampler.bpm_hold = Some((d, t0, t));
                }
                ui.ctx().request_repaint();
            } else {
                app.px.sampler.bpm_hold = None;
            }
        }
    }

    // ---- the bottom line: BANK, Q, BPM SYNC, MASTER
    p.text(pos2(x0 + 6.0, y0 + h - 13.0), Align2::LEFT_CENTER, "BANK", caps(9.5), hex(0xC8C8C8));
    for b in 0..4usize {
        let rr = at(43.0 + 28.0 * b as f32, h - 19.0, 22.0, 10.0);
        let resp = ui.interact(rr, ui.id().with(("smp_bank", b)), Sense::click());
        p.rect_filled(rr, 1.0, if bank == b { BLUE } else if resp.hovered() { lighten(hex(0x282828)) } else { hex(0x282828) });
        if crate::ui_player::press_fired(ui, &resp) && bank != b {
            sp.bank = b as u8;
            // Changing the bank closes an open EDIT panel.
            app.px.sampler.edit = None;
            changed = true;
        }
        let _ = resp.on_hover_text(format!("BANK {}", b + 1));
    }
    let master_r = at(4.0 * slot_w - 92.0, h - 20.0, 51.0, 10.0);
    let sync_r = Rect::from_min_size(pos2(master_r.min.x - 88.0, master_r.min.y), vec2(68.0, 10.0));
    let q_r = Rect::from_min_size(pos2(sync_r.min.x - 30.0, y0 + h - 19.0), vec2(9.0, 10.0));
    if text_button(ui, q_r.expand(3.0), "smp_q", "Q", sp.quantize, true).on_hover_text("QUANTIZE: samples start on the sampler's next beat").clicked() {
        sp.quantize = !sp.quantize;
        changed = true;
    }
    if text_button(ui, sync_r, "smp_sync", "BPM SYNC", sp.sync, true).on_hover_text("BPM SYNC: the sampler follows the master deck's BPM").clicked() {
        sp.sync = !sp.sync;
        changed = true;
    }
    if text_button(ui, master_r, "smp_master", "MASTER", app.px.sampler.master, true).on_hover_text("MASTER: the sampler deck becomes the sync master; decks with BEAT SYNC follow its BPM and beats").clicked() {
        app.sampler_toggle_master();
    }
    // The output the sampler deck goes to: only when the audio device has a second pair
    // (the headphones output), as rekordbox shows one button per output pair.
    if hp_on {
        let or = Rect::from_min_size(pos2(q_r.min.x - 96.0, sync_r.min.y - 2.0), vec2(74.0, 14.0));
        let oresp = box_button(ui, or, "smp_out", if sp.output == 1 { "HEADPHONES" } else { "MASTER OUT" }, false);
        if oresp.on_hover_text("Select audio output of the sampler deck.").clicked() {
            sp.output = 1 - sp.output.min(1);
            changed = true;
        }
    }

    // ---- the sequencer strip (590 x 30 from the centre + 40)
    sequencer_strip(app, ui, pos2(x0 + (w / 2.0).floor() + 40.0, y0 + h - 30.0), &sv, &mut sp, &mut changed);

    if changed {
        // (Only what this strip edits: slots, colours, names and patterns are written where
        // they change.)
        let d = &mut app.prefs.sampler;
        (d.gain, d.volume, d.bpm, d.bank, d.quantize, d.sync, d.cue, d.output) = (sp.gain, sp.volume, sp.bpm, sp.bank, sp.quantize, sp.sync, sp.cue, sp.output);
        (d.seq.metronome, d.seq.metronome_vol) = (sp.seq.metronome, sp.seq.metronome_vol);
        app.save_prefs();
    }

    // ---- the EDIT panel of one slot: beside it, over both rows (slot width - 2 by 84)
    if let Some(idx) = app.px.sampler.edit.filter(|i| i / PADS == bank && app.prefs.sampler.slots[*i].is_some()) {
        let pad = idx % PADS;
        let sr = slot_rect(pad);
        // To the right of the slot; for the last column of the right group, to its left.
        let ex = if (pad & !4) == 11 { sr.min.x - sr.width() + 2.0 } else { sr.max.x };
        let er = Rect::from_min_size(pos2(ex, y0 + 6.0), vec2(slot_w - 2.0, 2.0 * SLOT_H - 4.0));
        edit_panel(app, ui, er, idx, slot_w);
    }
}

/// The sequencer as the panel shows it.
struct SeqView {
    playing: bool,
    rec: bool,
    pos: f64,
    beats: f64,
    bars: u8,
    cur: usize,
    reserved: Option<usize>,
    dirty: bool,
    has_events: bool,
}

/// The box of REC and PLAY (20 x 20 in the reference, half a point in from the left). Gives
/// back where the 19 x 18 icon of `sampler_recBtn_*.svg` / `sampler_playBtn_*.svg` starts:
/// the reference shows it at its own size in the middle of the box.
fn seq_box(p: &egui::Painter, r: Rect, fill: Color32) -> Pos2 {
    let b = Rect::from_min_size(r.min + vec2(0.5, 0.0), vec2(20.0, 20.0));
    p.rect_filled(b, 2.0, fill);
    r.min + vec2(0.5, 1.0)
}

/// The sequencer strip: REC, PLAY, SAVE, the pattern with its arrows, the length, the beat
/// counter, MUTE, ERASE, the metronome and its volume. `so`: its top left corner.
fn sequencer_strip(app: &mut App, ui: &mut Ui, so: Pos2, sv: &SeqView, sp: &mut SamplerPrefs, changed: &mut bool) {
    let p = ui.painter().clone();
    let q_at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(so + vec2(x, y), vec2(w, h));
    let with_engine = |f: &mut dyn FnMut(&mut sampler::Engine)| {
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            f(e);
        }
    };
    const RED: Color32 = hex(0xFF0000);

    // REC: arms recording; it runs while the pattern plays (a pad starts it).
    let rr = q_at(0.0, 5.0, 21.0, 20.0);
    let resp = ui.interact(rr, ui.id().with("seq_rec"), Sense::click());
    let o = seq_box(&p, rr, if sv.rec { RED } else { BTN });
    let ic = if sv.rec { WHITE } else { RED };
    p.circle_filled(o + vec2(9.5, 9.0), 3.0, ic);
    p.circle_stroke(o + vec2(9.5, 9.0), 4.5, Stroke::new(1.0_f32, ic));
    if tip(resp, app, "Sequence Rec:\nRec Sampler Sequence").clicked() {
        with_engine(&mut |e| e.seq.set_rec(!sv.rec));
    }

    // PLAY: from the start of the pattern; again: stop (REC goes off, every pad stops).
    let pr = q_at(25.0, 5.0, 21.0, 20.0);
    let resp = ui.interact(pr, ui.id().with("seq_play"), Sense::click());
    let o = seq_box(&p, pr, if sv.playing { BLUE } else { BTN });
    p.add(egui::Shape::convex_polygon(vec![o + vec2(6.2, 5.7), o + vec2(12.2, 9.0), o + vec2(6.2, 12.3)], WHITE, Stroke::new(0.5_f32, WHITE)));
    if tip(resp, app, "Sequence Play:\nPlay Sampler sequence").clicked() {
        app.sampler_action(crate::sampler_ext::SmpAct::SeqPlay);
    }

    // SAVE: the pattern as it is goes into its slot (a paid function in rekordbox; free here).
    let save = q_at(50.0, 5.0, 46.0, 20.0);
    let resp = ui.interact(save, ui.id().with("seq_save"), Sense::click());
    p.rect_filled(save, 2.0, hex(0x0A0A0A));
    p.text(save.center(), Align2::CENTER_CENTER, "SAVE", caps(10.5), if !sv.dirty { hex(0x303030) } else if resp.hovered() { WHITE } else { hex(0x8C8C8C) });
    if tip(resp, app, "Sequence Save:\nSave Sampler sequence").clicked() && sv.dirty {
        app.sampler_seq_save();
    }

    // The pattern between its arrows (`LengthRoll_ArrowL/R.svg`, 5 x 8, rgb 150).
    let arrow = hex(0x969696);
    for (dir, x, id) in [(-1i32, 105.0f32, "seq_prev"), (1, 200.0, "seq_next")] {
        let ar = q_at(x, 11.0, 5.0, 8.0);
        let resp = ui.interact(ar.expand2(vec2(4.0, 6.0)), ui.id().with(id), Sense::click());
        let col = if resp.hovered() { WHITE } else { arrow };
        let (tipx, backx) = if dir < 0 { (ar.min.x, ar.max.x) } else { (ar.max.x, ar.min.x) };
        let s = if dir < 0 { 1.0 } else { -1.0 };
        // The chevron of the SVG: tip at (0, 4.07), arms to the far edge, 1.8 thick.
        for arm in [-1.0f32, 1.0] {
            p.add(egui::Shape::convex_polygon(vec![pos2(tipx, ar.center().y), pos2(backx, ar.center().y + 4.0 * arm), pos2(backx, ar.center().y + 2.2 * arm), pos2(tipx + 2.3 * s, ar.center().y)], col, Stroke::NONE));
        }
        if tip(resp, app, "Select sequence to Rec/Play").clicked() {
            let from = sv.reserved.unwrap_or(sv.cur) as i32;
            let to = (from + dir).rem_euclid(PATTERNS as i32) as usize;
            // Stopped: at once. Playing: on the next bar.
            with_engine(&mut |e| e.seq.select(to, 4.0));
        }
    }
    let slot_r = q_at(112.0, 5.0, 86.0, 20.0);
    let resp = ui.interact(slot_r, ui.id().with("seq_slot"), Sense::click_and_drag());
    let slot_hover = resp.hovered();
    let saved_slot = app.prefs.sampler.seq.patterns.get(sv.cur).is_some_and(|x| x.is_some());
    // Dragged onto a deck: Sequence Load. Right-click: the name, and Sequence Load by deck.
    if resp.drag_started() {
        app.px.sampler.drag_seq = true;
    }
    if app.px.sampler.drag_seq {
        let ptr = ui.ctx().pointer_latest_pos();
        let over = ptr.and_then(|pp| (0..app.deck_count()).find(|d| app.px.sampler.deck_rects[*d as usize].contains(pp)));
        if let Some(pp) = ptr {
            let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("seq_dragl")));
            let rr = Rect::from_min_size(pp + vec2(12.0, 6.0), vec2(110.0, 18.0));
            lp.rect_filled(rr, 3.0, BLUE);
            lp.text(rr.center(), Align2::CENTER_CENTER, "Sequence Load", FontId::proportional(10.5), WHITE);
            if let Some(d) = over {
                lp.rect_stroke(app.px.sampler.deck_rects[d as usize].shrink(1.0), 0.0, Stroke::new(2.0_f32, BLUE), egui::StrokeKind::Inside);
            }
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.pointer.any_released()) || !ui.input(|i| i.pointer.any_down()) {
            app.px.sampler.drag_seq = false;
            if let Some(d) = over {
                app.sampler_sequence_load(d);
            }
        }
    }
    let decks = app.deck_count();
    resp.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            app.px.sampler.name_edit = Some((sv.cur, app.prefs.sampler.seq.name(sv.cur)));
            ui.close_menu();
        }
        ui.separator();
        for d in 0..decks {
            if ui.add_enabled(!app.deck_options.disable_load, egui::Button::new(format!("Sequence Load (Deck{})", d + 1))).clicked() {
                app.sampler_sequence_load(d);
                ui.close_menu();
            }
        }
    });
    // White: saved. Grey: not saved. Grey with a red dot: recorded, not saved yet.
    let temp = sv.dirty && sv.has_events;
    let blink = (ui.input(|i| i.time) / 0.3) as i64 % 2 == 0;
    let (name_no, name_col) = match sv.reserved {
        // A change waiting for its bar: the pattern to come blinks.
        Some(n) => (n, if blink { WHITE } else { hex(0x505050) }),
        None => (sv.cur, if saved_slot && !sv.dirty { WHITE } else { hex(0x505050) }),
    };
    if let Some((slot, mut text)) = app.px.sampler.name_edit.take() {
        // The pattern's name being typed (Enter keeps it; empty: back to "PATTERN n").
        let te = ui.put(slot_r, egui::TextEdit::singleline(&mut text).id(ui.id().with("seq_name_edit")).font(FontId::proportional(10.5)).char_limit(32));
        if !te.has_focus() && !te.lost_focus() {
            te.request_focus();
        }
        if te.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let sq = &mut app.prefs.sampler.seq;
                sq.names.resize(PATTERNS, String::new());
                let t = text.trim();
                sq.names[slot % PATTERNS] = if t == format!("PATTERN {}", slot + 1) { String::new() } else { t.to_string() };
                app.save_prefs();
            }
        } else {
            app.px.sampler.name_edit = Some((slot, text));
        }
    } else {
        p.with_clip_rect(q_at(112.0, 4.5, 86.0, 20.0)).text(q_at(100.0, 4.5, 110.0, 20.0).center(), Align2::CENTER_CENTER, app.prefs.sampler.seq.name(name_no), caps(9.0), name_col);
    }
    if temp && sv.reserved.is_none() {
        p.circle_filled(so + vec2(120.0, 15.0), 4.0, RED);
    }
    let _ = tip(resp, app, "Sequence Slot:\nIndicates how sequence is saved:\n- Sequence name in white indicates the sequence is saved.\n- Gray out sequence name indicates the sequence is not saved.\n- Gray out sequence name with red circle indicates the sequence is temporarily recorded.\nUp to eight sequences can be saved.");
    // DELETE (`p13008_delete_1nrm.png`, 12 x 12) shows while the pointer is over the pattern.
    if slot_hover && (saved_slot || sv.has_events || sv.dirty) && sv.reserved.is_none() {
        let dr = q_at(185.0, 9.0, 12.0, 12.0);
        let dresp = ui.interact(dr, ui.id().with("seq_delete"), Sense::click());
        let col = if dresp.hovered() { WHITE } else { hex(0xA0A0A0) };
        let c = dr.center();
        p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], Stroke::new(1.3_f32, col));
        p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], Stroke::new(1.3_f32, col));
        if tip(dresp, app, "Delete selected sequence").clicked() {
            app.sampler_seq_delete(sv.cur);
        }
    }

    // Length: 1Bar / 2Bars / 4Bars.
    let len = q_at(213.0, 6.0, 48.0, 18.0);
    let resp = ui.interact(len, ui.id().with("seq_len"), Sense::click());
    let bars_label = |b: u8| if b == 1 { "1Bar".to_string() } else { format!("{b}Bars") };
    p.text(len.left_center() + vec2(5.5, -0.5), Align2::LEFT_CENTER, bars_label(sv.bars), FontId::proportional(11.0), WHITE);
    crate::icons_player::chevron_down(&p, len.right_center() + vec2(-4.0, 0.5), 1.0, WHITE);
    let mut new_bars = None;
    crate::ui_browser::dropdown(ui, &resp, "seq_len_pop", |ui| {
        ui.set_min_width(70.0);
        for b in [1u8, 2, 4] {
            if ui.selectable_label(sv.bars == b, bars_label(b)).clicked() {
                new_bars = Some(b);
                ui.memory_mut(|m| m.close_popup());
            }
        }
    });
    let _ = tip(resp, app, "Change sequence length");
    if let Some(b) = new_bars {
        with_engine(&mut |e| e.seq.set_bars(b));
    }

    // Beat counter (163 x 12): a black bar with a mark per beat, and the play position.
    let beat = q_at(270.0, 10.0, 163.0, 12.0);
    let resp = ui.interact(beat, ui.id().with("seq_beat"), Sense::hover());
    p.rect_filled(Rect::from_min_size(beat.min + vec2(0.0, 3.0), vec2(163.0, 6.0)), 0.0, Color32::BLACK);
    let marks = sv.bars.max(1) as usize * 4;
    for k in 0..=marks {
        // Every bar line (1 Bar: every second beat) is the brighter mark.
        let strong = if sv.bars == 1 { k % 2 == 0 } else { k % 4 == 0 };
        let x = beat.min.x + 1.0 + 160.0 * k as f32 / marks as f32;
        p.rect_filled(Rect::from_min_size(pos2(x, beat.min.y + 4.0), vec2(1.0, 4.0)), 0.0, if strong { hex(0x808080) } else { hex(0x404040) });
    }
    // The marker (`p13019_plyposition`, drawn at twice its size: 8 x 12): white, red while
    // recording.
    let frac = (sv.pos / sv.beats.max(1.0)).clamp(0.0, 1.0) as f32;
    let mk = Rect::from_min_size(pos2(beat.min.x + frac * (163.0 - 8.0), beat.min.y), vec2(8.0, 12.0));
    p.rect_filled(mk, 1.5, Color32::BLACK);
    p.rect_filled(mk.shrink2(vec2(2.0, 1.5)), 0.0, if sv.rec && sv.playing { hex(0xFE0000) } else { WHITE });
    let _ = tip(resp, app, "Beat Counter:\nIndicates sequence length and beats.\nDuring recording or playback of a sequence, a marker that indicates playback position is moving.");

    // MUTE and ERASE: pad modes of the sequencer, one at a time, only while it plays.
    for (mode, x, label, id, hint) in [
        (1u8, 445.0f32, "MUTE", "seq_mute", "Sequence Mute mode:\nMute Sampler sequence per Sampler Slot"),
        (2, 488.0, "ERASE", "seq_erase", "Sequence Erase mode:\nErase Sampler sequence per Sampler Slot"),
    ] {
        let on = app.px.sampler.seq_mode == mode;
        // (Arial 12 in rekordbox: a little larger than Q and BPM SYNC.)
        let mr = q_at(x, 5.0, 46.0, 20.0);
        let resp = ui.interact(mr, ui.id().with(id), Sense::click());
        let col = if !sv.playing { hex(0x484848) } else if on { BLUE } else if resp.hovered() { WHITE } else { hex(0x8C8C8C) };
        p.text(mr.center(), Align2::CENTER_CENTER, label, caps(10.5), col);
        if tip(resp, app, hint).clicked() {
            if sv.playing {
                app.px.sampler.seq_mode = if on { 0 } else { mode };
            } else {
                app.toast_err("Invalid operation. Available during sequencer playback/recording.");
            }
        }
    }

    // Metronome (`metronome_off/on.svg`, 19 x 18) and its volume (`metronome_vol1..3.svg`).
    let mr = q_at(545.0, 4.0, 19.0, 18.0);
    let resp = ui.interact(mr, ui.id().with("seq_metro"), Sense::click());
    p.rect_filled(Rect::from_min_size(mr.min + vec2(0.5, 0.0), vec2(18.0, 18.0)), 2.0, if sp.seq.metronome { BLUE } else { BTN });
    let m = |x: f32, y: f32| mr.min + vec2(x, y);
    let poly = |pts: Vec<Pos2>| {
        p.add(egui::Shape::convex_polygon(pts, WHITE, Stroke::NONE));
    };
    poly(vec![m(5.95, 11.0), m(13.05, 11.0), m(13.5, 14.0), m(5.5, 14.0)]);
    poly(vec![m(7.0, 4.0), m(12.0, 4.0), m(11.14, 5.0), m(7.86, 5.0)]);
    poly(vec![m(12.0, 4.0), m(13.05, 11.0), m(12.0, 11.0), m(11.14, 5.0)]);
    poly(vec![m(6.35, 8.4), m(7.17, 9.8), m(7.0, 11.0), m(5.95, 11.0)]);
    poly(vec![m(5.3, 4.5), m(6.5, 4.5), m(10.3, 11.0), m(9.0, 11.0)]);
    p.rect_filled(Rect::from_min_size(m(8.5, 6.0), vec2(2.0, 1.0)), 0.0, WHITE);
    p.rect_filled(Rect::from_min_size(m(8.5, 8.0), vec2(2.0, 1.0)), 0.0, WHITE);
    if tip(resp, app, "Metronome On/Off").clicked() {
        sp.seq.metronome = !sp.seq.metronome;
        *changed = true;
    }
    let vr = q_at(570.0, 4.0, 19.0, 18.0);
    let resp = ui.interact(vr, ui.id().with("seq_metro_vol"), Sense::click());
    let vol = sp.seq.metronome_vol.min(2);
    for (k, (x, y0, y1)) in [(3.0f32, 5.0f32, 13.0f32), (8.0, 3.0, 15.0), (13.0, 1.0, 17.0)].into_iter().enumerate() {
        p.rect_filled(Rect::from_min_max(vr.min + vec2(x, y0), vr.min + vec2(x + 3.0, y1)), 0.0, if k as u8 <= vol { BLUE } else { OFF });
    }
    if tip(resp, app, "Metronome Volume").clicked() {
        sp.seq.metronome_vol = (vol + 1) % 3;
        *changed = true;
    }
}

fn edit_panel(app: &mut App, ui: &mut Ui, er: Rect, idx: usize, slot_w: f32) {
    let p = ui.painter().clone();
    // Swallow clicks so the slots underneath do not react.
    let _ = ui.interact(er, ui.id().with(("smp_editbg", idx)), Sense::click());
    p.rect_filled(er, 2.0, hex(0x141414));
    p.rect_stroke(er, 2.0, Stroke::new(1.0_f32, hex(0x3C3C3C)), egui::StrokeKind::Inside);
    let Some(mut s) = app.prefs.sampler.slots[idx].clone() else { return };
    let o = er.min;
    let v = 2.0 + ((slot_w - 148.0) / 2.0).max(0.0).floor();
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(o.x + v + x, o.y + y), vec2(w, h));
    let mut changed = false;

    // One-shot / loop (20 and 21 x 14) and gate (18 x 14).
    let one = box_button(ui, at(4.0, 8.0, 20.0, 14.0), ("smp_one", idx), "", !s.looped);
    let c = one.rect.center();
    p.line_segment([c + vec2(-5.0, 0.0), c + vec2(3.0, 0.0)], Stroke::new(1.2_f32, WHITE));
    p.line_segment([c + vec2(5.0, -4.0), c + vec2(5.0, 4.0)], Stroke::new(1.2_f32, WHITE));
    p.add(egui::Shape::convex_polygon(vec![c + vec2(0.5, -3.0), c + vec2(4.0, 0.0), c + vec2(0.5, 3.0)], WHITE, Stroke::NONE));
    if tip(one, app, "One-shot: the sample plays once").clicked() && s.looped {
        s.looped = false;
        changed = true;
    }
    let lp = box_button(ui, at(25.0, 8.0, 21.0, 14.0), ("smp_loop", idx), "", s.looped);
    let c = lp.rect.center();
    let ring: Vec<Pos2> = (0..=10).map(|i| { let a = (40.0 + 28.0 * i as f32).to_radians(); c + vec2(a.cos() * 5.5, a.sin() * 3.5) }).collect();
    let tipp = *ring.last().unwrap();
    p.add(egui::Shape::line(ring, Stroke::new(1.2_f32, WHITE)));
    p.add(egui::Shape::convex_polygon(vec![tipp + vec2(-3.0, -1.0), tipp + vec2(1.5, -2.5), tipp + vec2(0.5, 2.5)], WHITE, Stroke::NONE));
    if tip(lp, app, "Loop: the sample repeats; only loop samples follow the sampler's BPM").clicked() && !s.looped {
        s.looped = true;
        changed = true;
    }
    let gate = box_button(ui, at(50.0, 8.0, 18.0, 14.0), ("smp_gate", idx), "", s.gate);
    let c = gate.rect.center();
    p.add(egui::Shape::line(vec![c + vec2(-6.0, 3.0), c + vec2(-3.0, 3.0), c + vec2(-3.0, -3.0), c + vec2(3.0, -3.0), c + vec2(3.0, 3.0), c + vec2(6.0, 3.0)], Stroke::new(1.2_f32, WHITE)));
    if tip(gate, app, "Gate: the sample sounds only while its PLAY button is held").clicked() {
        s.gate = !s.gate;
        changed = true;
    }
    // MT and SYNC (30 x 14): for loop samples.
    for (x, label, id) in [(74.0, "MT", "smp_mt"), (108.0, "SYNC", "smp_sync1")] {
        let on = if label == "MT" { s.mt } else { s.sync };
        let rr = at(x, 8.0, 30.0, 14.0);
        let resp = ui.interact(rr, ui.id().with((id, idx)), Sense::click());
        p.rect_filled(rr, 2.0, if on && s.looped { BLUE } else if resp.hovered() && s.looped { lighten(BTN) } else { BTN });
        p.text(rr.center(), Align2::CENTER_CENTER, label, caps(8.0), if s.looped { WHITE } else { hex(0x424242) });
        let hint = if label == "MT" { "MASTER TEMPO: the sample keeps its pitch when its tempo changes" } else { "SYNC: the loop sample follows the sampler's BPM" };
        if tip(resp, app, hint).clicked() && s.looped {
            if label == "MT" {
                s.mt = !s.mt;
            } else {
                s.sync = !s.sync;
            }
            changed = true;
        }
    }
    // GAIN knob (24 x 24) with its label, and the sample's BPM (64 x 20).
    if let Some(g) = crate::ui_perf::knob(ui, pos2(o.x + v + 37.0, o.y + 41.0), ("smp_slot_gain", idx), s.gain / 100.0, 0.5, WHITE) {
        s.gain = (g * 100.0).round();
        changed = true;
    }
    p.text(pos2(o.x + v + 38.0, o.y + 55.5), Align2::CENTER_CENTER, "GAIN", caps(8.0), hex(0x8C8C8C));
    let bpm_r = at(75.0, 34.0, 64.0, 20.0);
    p.text(bpm_r.center(), Align2::CENTER_CENTER, if s.bpm > 0.0 { format!("{:.2}", s.bpm) } else { "---.--".to_string() }, bold(13.0), if s.bpm > 0.0 { WHITE } else { hex(0x646464) });
    // Shift the start 1 ms earlier / later; halve / double the sample's BPM (40..499).
    let sl = box_button(ui, at(4.0, 64.0, 30.0, 14.0), ("smp_shl", idx), "", false);
    crate::icons_player::chevron_left(&p, sl.rect.center(), 0.9, WHITE);
    if tip(sl, app, "Move the start point 1 ms earlier").clicked() {
        s.offset_ms = (s.offset_ms - 1.0).max(0.0);
        changed = true;
    }
    let sr = box_button(ui, at(38.0, 64.0, 30.0, 14.0), ("smp_shr", idx), "", false);
    crate::icons_player::chevron_right(&p, sr.rect.center(), 0.9, WHITE);
    if tip(sr, app, "Move the start point 1 ms later").clicked() {
        s.offset_ms += 1.0;
        changed = true;
    }
    for (x, label, k) in [(74.0, "/2", 0.5f32), (108.0, "x2", 2.0)] {
        let resp = box_button(ui, at(x, 64.0, 30.0, 14.0), ("smp_bpmk", idx, label), label, false);
        if tip(resp, app, if k < 1.0 { "Halve the sample's BPM" } else { "Double the sample's BPM" }).clicked() && s.bpm > 0.0 {
            let nb = s.bpm * k;
            if (40.0..=499.0).contains(&nb) {
                s.bpm = nb;
                changed = true;
            } else {
                app.toast_err("The BPM must be between 40 and 499.");
            }
        }
    }
    if changed {
        app.prefs.sampler.slots[idx] = Some(s);
        app.sampler_push_slot(idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (Where rekordbox is not installed there is nothing to check: the engine's own click
    /// is used.)
    #[test]
    fn the_metronome_reads_rekordboxs_click_when_it_is_installed() {
        if !std::path::Path::new(CLICK_FILE).exists() {
            return;
        }
        let d = crate::audio::decode_file(CLICK_FILE).expect("the click file decodes");
        assert_eq!(d.sr, 44100);
        assert!(d.frames() > 1000 && d.frames() < 44100, "{}", d.frames());
        assert!(d.samples.iter().any(|v| v.abs() > 0.1));
    }

    #[test]
    fn sequencer_settings_survive_a_round_trip_and_old_settings_still_load() {
        let mut sp = SamplerPrefs::default();
        sp.seq.patterns[2] = Some(Pattern { bars: 4, events: vec![crate::sequencer::Event { t: 1.5, pad: 3, bank: 1, on: true, vel: 0.5, fresh: true }] });
        sp.seq.current = 2;
        sp.seq.metronome = true;
        sp.seq.metronome_vol = 2;
        let back: SamplerPrefs = serde_json::from_str(&serde_json::to_string(&sp).unwrap()).unwrap();
        let p = back.seq.patterns[2].as_ref().unwrap();
        assert_eq!((p.bars, p.events.len(), p.events[0].t, p.events[0].pad, p.events[0].bank, p.events[0].fresh), (4, 1, 1.5, 3, 1, false));
        assert_eq!((back.seq.current, back.seq.metronome, back.seq.metronome_vol), (2, true, 2));
        // Settings written before the sequencer existed: rekordbox's defaults.
        let old: SamplerPrefs = serde_json::from_str(r#"{"gain":50.0,"bpm":128.0}"#).unwrap();
        assert_eq!((old.seq.patterns.len(), old.seq.metronome, old.seq.metronome_vol, old.seq.quantize_on, old.seq.quantize, old.bpm), (PATTERNS, false, 1, true, 4, 128.0));
    }
}
