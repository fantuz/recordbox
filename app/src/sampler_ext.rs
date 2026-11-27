//! The sampler deck beyond its panel: pad colours, the pads' shared press rules (panel,
//! deck pads, keyboard), keyboard actions, moving and loading slots, loop capture and
//! Sequence Load. After rekordbox 7.2.19 (checks/re-7.2.19/pads-sampler-automix.md, B and D5).

use crate::app::*;
use crate::sampler::{self, MODE_LOOP, PADS};
use crate::sequencer::PATTERNS;
use eframe::egui::Color32;

/// rekordbox's pad colour table (index = colour code, 1..62 are colours), as its static
/// initialiser fills it (`__GLOBAL__sub_I_SamplerControlPanel.cpp` 0x100b1d124, 65
/// `juce::Colour(r, g, b)` calls into 0x105482FF0; the hot cue table at 0x105440070 holds
/// the same values).
pub const PAD_COLORS: [u32; 65] = [
    0x000000, 0x305AFF, 0x5073FF, 0x508CFF, 0x50A0FF, 0x50B4FF, 0x50B0F2, 0x50AEE8, 0x45ACDB, 0x00E0FF, 0x19DAF0, 0x32D2E6, 0x21B4B9, 0x20AAA0, 0x1FA392, 0x19A08C, 0x14A584, 0x14AA7D, 0x10B176, 0x30D26E, 0x37DE5A, 0x3CEB50,
    0x28E214, 0x7DC13D, 0x8CC832, 0x9BD723, 0xA5E116, 0xA5DC0A, 0xAAD208, 0xB4C805, 0xB4BE04, 0xBAB404, 0xC3AF04, 0xE1AA00, 0xFFA000, 0xFF9600, 0xFF8C00, 0xFF7500, 0xE0641B, 0xE0461E, 0xE0301E, 0xE02823, 0xE62828, 0xFF376F,
    0xFF2D6F, 0xFF127B, 0xF51E8C, 0xEB2DA0, 0xE637B4, 0xDE44CF, 0xDE448D, 0xE630B4, 0xE619DC, 0xE600FF, 0xDC00FF, 0xCC00FF, 0xB432FF, 0xB93CFF, 0xC542FF, 0xAA5AFF, 0xAA72FF, 0x8272FF, 0x6473FF, 0x000000, 0xFFFFFF,
];

/// `PadColor::samplerColor` (0x1037aa49c): the 16 codes the sampler's colour picker offers.
pub const PICKER: [u8; 16] = [49, 56, 60, 62, 1, 5, 9, 14, 18, 22, 26, 30, 32, 38, 42, 45];
/// The code a sample has when it was given none (rekordbox's `loadClip` falls back to 60).
pub const DEFAULT_COLOR: u8 = 60;

/// A pad colour as the sampler shows it: the table's colour at alpha 0.8 over black.
pub fn pad_color(code: u8) -> Color32 {
    let c = PAD_COLORS[if PICKER.contains(&code) { code } else { DEFAULT_COLOR } as usize];
    let k = |v: u32| ((v & 0xFF) as f32 * 0.8).round() as u8;
    Color32::from_rgb(k(c >> 16), k(c >> 8), k(c))
}

/// rekordbox's `VELOCITY_COEF_TABLE` (0x1037aa4c0): four curves, pad velocity 0..127 in,
/// 0..127 out. Curve 1 is linear.
const VELOCITY_2: [u8; 128] = [
    0, 2, 3, 5, 7, 8, 10, 12, 13, 15, 17, 18, 20, 21, 23, 25, 26, 28, 29, 31, 33, 34, 36, 37, 39, 40, 42, 43, 45, 46, 48, 49, 51, 52, 54, 55, 56, 58, 59, 61, 62, 63, 65, 66, 68, 69, 70, 72, 73, 74, 75, 77, 78, 79, 80, 82, 83, 84, 85, 86, 88, 89, 90, 91,
    92, 93, 94, 95, 97, 98, 99, 100, 101, 102, 103, 104, 104, 105, 106, 107, 108, 109, 110, 111, 111, 112, 113, 114, 115, 115, 116, 117, 117, 118, 119, 119, 120, 120, 121, 122, 122, 123, 123, 123, 124, 124, 125, 125, 125, 126, 126, 126, 126, 127, 127, 127,
    127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127,
];
const VELOCITY_3: [u8; 128] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10, 11, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16, 17, 18, 18, 19, 20, 20, 21, 22, 22, 23, 24, 25, 26, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 42, 43, 44, 45, 47, 48, 49, 51, 52, 54, 55, 57, 59, 60, 62, 64, 66, 67, 69, 71, 73, 75, 77, 79, 81, 84, 86, 88, 91, 93, 95, 98, 101, 103, 106, 109, 112, 115, 118, 121, 124, 127,
];

/// The gain (0..1) of a pad hit of MIDI velocity `v` with rekordbox's velocity curve 1..4.
pub fn velocity_gain(curve: u8, v: u8) -> f32 {
    let v = v.min(127) as usize;
    let out = match curve.clamp(1, 4) {
        1 => v as u8,
        2 => VELOCITY_2[v],
        3 => VELOCITY_3[v],
        _ => match v {
            0 => 0,
            1..=42 => 40,
            43..=84 => 80,
            _ => 127,
        },
    };
    out as f32 / 127.0
}

/// A keyboard action of the sampler deck (rekordbox's `SamplerControlPanel` commands 0xf000..).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmpAct {
    Play(u8),
    Pause(u8),
    Bank(u8),
    Gain(f32),
    Volume(f32),
    Quantize,
    Sync,
    Master,
    Bpm(f32),
    SeqRec,
    SeqPlay,
    SeqSave,
    SeqStep(i32),
    SeqDelete,
    SeqMode(u8),
    SeqLoad(u8),
    PadMode(u8),
}

impl SmpAct {
    pub fn parse(id: &str) -> Option<SmpAct> {
        let num = |p: &str, max: usize| id.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=max).contains(n)).map(|n| n as u8 - 1);
        Some(match id {
            "smp_gain_up" => SmpAct::Gain(1.0),
            "smp_gain_down" => SmpAct::Gain(-1.0),
            "smp_vol_up" => SmpAct::Volume(1.0),
            "smp_vol_down" => SmpAct::Volume(-1.0),
            "smp_quantize" => SmpAct::Quantize,
            "smp_sync" => SmpAct::Sync,
            "smp_master" => SmpAct::Master,
            "smp_bpm_up" => SmpAct::Bpm(0.01),
            "smp_bpm_up_large" => SmpAct::Bpm(1.0),
            "smp_bpm_down" => SmpAct::Bpm(-0.01),
            "smp_bpm_down_large" => SmpAct::Bpm(-1.0),
            "seq_rec" => SmpAct::SeqRec,
            "seq_play" => SmpAct::SeqPlay,
            "seq_save" => SmpAct::SeqSave,
            "seq_prev" => SmpAct::SeqStep(-1),
            "seq_next" => SmpAct::SeqStep(1),
            "seq_delete" => SmpAct::SeqDelete,
            "seq_mute" => SmpAct::SeqMode(1),
            "seq_erase" => SmpAct::SeqMode(2),
            "padmode_sampler" => SmpAct::PadMode(crate::ui_perf::PAD_SAMPLER),
            "padmode_seqcall" => SmpAct::PadMode(crate::ui_perf::PAD_SEQ_CALL),
            _ => {
                if let Some(n) = num("smp_play", 16) {
                    SmpAct::Play(n)
                } else if let Some(n) = num("smp_pause", 16) {
                    SmpAct::Pause(n)
                } else if let Some(n) = num("smp_bank", 4) {
                    SmpAct::Bank(n)
                } else if let Some(n) = num("seq_load", 4) {
                    SmpAct::SeqLoad(n)
                } else {
                    return None;
                }
            }
        })
    }
    pub fn repeats(&self) -> bool {
        matches!(self, SmpAct::Gain(_) | SmpAct::Volume(_) | SmpAct::Bpm(_))
    }
}

/// The action is one of the sampler deck's (its keys are rekordbox's Performance preset, and
/// in PERFORMANCE mode they come before another action on the same key).
pub fn is_sampler_action(id: &str) -> bool {
    id.starts_with("smp_") || id.starts_with("seq_") || id.starts_with("padmode_")
}

/// Where captured loops and loaded sequences are written (rekordbox: `SamplerSaveDirectory`,
/// by default `Music/rekordbox/Sampler/Capture`; here under recordbox's own data folder).
pub fn capture_dir(chosen: &str) -> std::path::PathBuf {
    if chosen.trim().is_empty() { crate::db::data_dir().join("Sampler").join("Capture") } else { std::path::PathBuf::from(chosen.trim()) }
}

/// Publish a complete stereo WAV in the capture folder. If another capture already
/// owns a name, retain its bytes and try the next numbered suffix.
pub fn save_capture(dir: &std::path::Path, title: &str, suffix: &str, sr: u32, samples: &[f32]) -> std::io::Result<std::path::PathBuf> {
    use std::io::{Error, ErrorKind, Write};
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(&crate::player_ext::wav16_bytes(sr, 2, samples))?;
    let mut pending = file.into_temp_path();

    let clean: String = title.chars().take(48).map(|c| if matches!(c, '/' | ':' | '\\' | '\0') { '_' } else { c }).collect();
    let base = format!("{}{}", if clean.trim().is_empty() { "sample" } else { clean.trim() }, suffix);
    for number in 0..=u32::MAX {
        let name = if number == 0 { format!("{base}.wav") } else { format!("{base}({number}).wav") };
        let target = dir.join(name);
        match pending.persist_noclobber(&target) {
            Ok(()) => return Ok(target),
            Err(e) if e.error.kind() == ErrorKind::AlreadyExists => pending = e.path,
            Err(e) => return Err(e.error),
        }
    }
    Err(Error::new(ErrorKind::AlreadyExists, "All numbered capture names are in use"))
}

impl App {
    /// A slot's colour: the one chosen for its sample (kept per file, so every slot holding
    /// that sample shows it), else rekordbox's fallback.
    pub fn sampler_color_code(&self, idx: usize) -> u8 {
        self.prefs.sampler.slots.get(idx).and_then(|s| s.as_ref()).and_then(|s| self.prefs.sampler.colors.get(&s.path)).copied().unwrap_or(DEFAULT_COLOR)
    }
    pub fn sampler_color(&self, idx: usize) -> Color32 {
        pad_color(self.sampler_color_code(idx))
    }
    /// Choose a colour for the sample in slot `idx` (None: reset).
    pub fn sampler_set_color(&mut self, idx: usize, code: Option<u8>) {
        let Some(path) = self.prefs.sampler.slots.get(idx).and_then(|s| s.as_ref()).map(|s| s.path.clone()) else { return };
        match code.filter(|c| PICKER.contains(c)) {
            Some(c) => {
                self.prefs.sampler.colors.insert(path, c);
            }
            None => {
                self.prefs.sampler.colors.remove(&path);
            }
        }
        self.save_prefs();
    }

    /// A slot holds a sample that is ready to play.
    pub fn sampler_loaded(&self, idx: usize) -> bool {
        self.prefs.sampler.slots.get(idx).is_some_and(|s| s.is_some())
    }

    /// The pads in use: 16, or the first 8 with SLOT 8.
    pub fn sampler_pads(&self) -> usize {
        if self.prefs.sampler.slots16 { PADS } else { 8 }
    }

    /// A sampler pad pressed (the slot's PLAY button, a deck pad in SAMPLER mode, a key).
    /// In the sequencer's MUTE mode the press toggles the slot's mute and in ERASE mode it
    /// takes the pad's notes out of the pattern; `stop` (Shift) stops the sample; otherwise it
    /// plays.
    pub fn sampler_pad_down(&mut self, pad: usize, velocity: f32, stop: bool, from_panel: bool) {
        if pad >= self.sampler_pads() {
            return;
        }
        let bank = self.prefs.sampler.bank.min(3) as usize;
        let idx = bank * PADS + pad;
        if !self.sampler_loaded(idx) {
            return;
        }
        // (The panel's own PLAY button keeps playing in MUTE / ERASE mode: in rekordbox only
        // pads and keys change their meaning.)
        let mode = if from_panel { 0 } else { self.px.sampler.seq_mode };
        match mode {
            1 => {
                self.px.sampler.toggle_mute(idx);
                self.sampler_push_slot(idx);
            }
            2 => {
                if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                    e.seq.erase_track(pad);
                }
            }
            _ => {
                let mut loop_refused = false;
                if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                    if stop {
                        e.stop(pad, bank, true);
                    } else if !self.deck_options.disable_play {
                        loop_refused = e.launch(pad, bank, velocity);
                    }
                }
                if loop_refused {
                    self.toast_err("Loop record error. Loop sample can not be recorded as a sequence.");
                }
            }
        }
    }

    /// The pad let go: gate samples stop.
    pub fn sampler_pad_up(&mut self, pad: usize) {
        let bank = self.prefs.sampler.bank.min(3) as usize;
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            e.stop(pad, bank, false);
        }
    }

    /// "Do not overwrite the loaded Slot." is ticked and the slot is loaded: say so.
    pub fn sampler_locked(&mut self, idx: usize) -> bool {
        if self.prefs.sampler.load_lock && self.sampler_loaded(idx) {
            self.toast_err("Invalid operation. Load the sample to a vacant slot, or untick \"Do not overwrite the loaded Slot.\" in [Preferences] > [Controller] > [Sampler].");
            return true;
        }
        false
    }

    /// Move a sample to another slot (rekordbox loads it there, then ejects the source). The
    /// slot's buttons, gain and start shift go with it.
    pub fn sampler_move(&mut self, from: usize, to: usize) {
        if from == to || from >= 64 || to >= 64 || self.sampler_locked(to) {
            return;
        }
        let Some(s) = self.prefs.sampler.slots.get(from).cloned().flatten() else { return };
        let clip = sampler::SAMPLER.lock().as_ref().and_then(|e| e.clip(from % PADS, from / PADS).cloned());
        let wave = self.px.sampler.take_wave(from);
        self.sampler_eject(from);
        self.sampler_eject(to);
        self.prefs.sampler.slots[to] = Some(s.clone());
        match (clip, wave) {
            (Some(mut c), Some(w)) => {
                c.mute = false;
                if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                    e.set_clip(to % PADS, to / PADS, Some(c));
                }
                self.px.sampler.put_wave(to, w);
            }
            // (Still being read when it was moved: read it again for the new slot.)
            _ => self.sampler_decode(to, s.path),
        }
        self.save_prefs();
    }

    /// Capture the active loop of `deck` into a slot: `target` (bank * 16 + pad), or the first
    /// empty slot of the bank showing. The loop is written as a WAV file (2 channels, 16 bit,
    /// the track's sample rate) and loaded; the slot keeps its buttons.
    pub fn sampler_capture_loop(&mut self, deck: u8, target: Option<usize>) -> bool {
        let bank = self.prefs.sampler.bank.min(3) as usize;
        let pads = self.sampler_pads();
        let Some(idx) = target.or_else(|| (0..pads).map(|p| bank * PADS + p).find(|i| !self.sampler_loaded(*i))) else {
            self.toast_err("Invalid operation. Load the sample to a vacant slot, or untick \"Do not overwrite the loaded Slot.\" in [Preferences] > [Controller] > [Sampler].");
            return false;
        };
        let (range, audio) = {
            let s = self.player.decks[(deck as usize).min(3)].lock();
            (s.loop_range, s.audio.clone())
        };
        let (Some((a, b)), Some(audio)) = (range, audio) else {
            self.toast_err("Invalid operation. Capture is available during Loop or Slicer performance.");
            return false;
        };
        if self.sampler_locked(idx) {
            return false;
        }
        let (title, bpm, beats) = {
            let d = self.deck_of(deck);
            let t = d.track.and_then(|id| self.track(id));
            (t.map(|t| t.display_title()).unwrap_or_default(), t.map(|t| t.bpm).unwrap_or(0.0), d.beats.iter().map(|x| x.time).collect::<Vec<f64>>())
        };
        // As rekordbox: with the deck's quantize on, an edge closer than 3 ms to a beat of
        // the grid is put on it.
        let snap = |t: f64| beats.iter().copied().find(|x| (x - t).abs() < 0.003).unwrap_or(t);
        let (a, b) = if self.prefs.quantize { (snap(a), snap(b)) } else { (a, b) };
        let sr = audio.sr as f64;
        let (f0, f1) = (((a * sr).round().max(0.0) as usize).min(audio.frames()), ((b * sr).round().max(0.0) as usize).min(audio.frames()));
        if f1 <= f0 + 1 {
            return false;
        }
        let dir = capture_dir(&self.prefs.sampler.capture_dir);
        let Ok(path) = save_capture(&dir, &title, "_capture", audio.sr, &audio.samples[f0 * 2..f1 * 2]) else {
            self.toast_err("The captured loop could not be written.");
            return false;
        };
        // The loop's own tempo: its beats over its length when it sits on the grid.
        let n = beats.iter().filter(|x| **x >= a - 0.0015 && **x < b - 0.0015).count();
        let bpm = if n >= 1 && b > a { (n as f64 * 60.0 / (b - a)) as f32 } else { bpm as f32 };
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        self.sampler_load_file(idx, path.to_string_lossy().to_string(), name, bpm);
        true
    }

    /// Sequence Load: one pass of the pattern, as audio, onto a deck (1..4 = 0..3).
    pub fn sampler_sequence_load(&mut self, deck: u8) {
        if self.deck_options.disable_load { return self.toast_err("Sequence Load is disabled for this session (--disable-load)."); }
        if deck >= self.deck_count() {
            return;
        }
        let rate = 44100u32;
        let (audio, bpm, cur) = match sampler::SAMPLER.lock().as_ref() {
            Some(e) => (e.render_pattern(rate), e.bpm, e.seq.current()),
            None => (None, 120.0, 0),
        };
        let Some(audio) = audio else {
            self.toast_err("SEQUENCE LOAD is invalid. No loaded sequence.");
            return;
        };
        // (rekordbox throws away a result of fewer than 4 beats or 0.1 s.)
        let secs = audio.len() as f64 / 2.0 / rate as f64;
        if (secs * bpm as f64 / 60.0 + 0.5).floor() < 4.0 || secs <= 0.1 {
            return;
        }
        let dir = capture_dir(&self.prefs.sampler.capture_dir);
        let name = self.prefs.sampler.seq.name(cur);
        let Ok(path) = save_capture(&dir, &name, "", rate, &audio) else {
            self.toast_err("The sequence could not be written.");
            return;
        };
        // Into the collection with the sampler's tempo and a grid from its first frame, then
        // onto the deck.
        let mut t = crate::importer::read_track(&path);
        t.bpm = bpm as f64;
        let id = match self.db.track_by_path(&t.path) {
            Some(id) => id,
            None => match self.db.insert_track(&t) {
                Ok(id) => id,
                Err(_) => {
                    self.toast_err("The sequence could not be added to the collection.");
                    return;
                }
            },
        };
        // (The pattern starts on its first frame at the sampler's tempo: that is its grid.)
        let _ = self.db.save_grid(id, &crate::model::BeatGrid::constant(0.0, bpm as f64, 1));
        self.reload_tracks();
        self.with_deck(deck, |a| a.load_track_here(id));
        // "Load the sequence and automatically play.": once the deck has read the file.
        if self.prefs.sampler.seq.load_autoplay && !self.deck_options.disable_play {
            self.px.sampler.autoplay = Some((deck, id, 0));
        }
    }

    /// Once per frame: start the deck a sequence was loaded on, when its audio is there.
    pub fn sampler_autoplay_poll(&mut self) {
        let Some((deck, id, frames)) = self.px.sampler.autoplay else { return };
        let d = (deck as usize).min(3);
        let (ready, playing) = {
            let s = self.player.decks[d].lock();
            (s.audio.is_some(), s.playing)
        };
        if self.deck_of(deck).track != Some(id) || frames > 1800 {
            self.px.sampler.autoplay = None;
        } else if ready {
            self.px.sampler.autoplay = None;
            if !playing {
                self.with_deck(deck, |a| a.play_pause());
            }
        } else {
            self.px.sampler.autoplay = Some((deck, id, frames + 1));
        }
    }

    /// Carry out a keyboard action of the sampler deck.
    pub fn sampler_action(&mut self, act: SmpAct) {
        if !self.performance() || (self.deck_options.disable_play && matches!(act, SmpAct::SeqPlay)) {
            return;
        }
        // Auto Panel Mode: a sampler or sequencer key shows the sampler panel first.
        if !self.prefs.sampler.auto_panel_disable && !self.prefs.perf_sampler && !matches!(act, SmpAct::PadMode(_)) {
            self.prefs.perf_sampler = true;
        }
        let with_engine = |f: &mut dyn FnMut(&mut sampler::Engine)| {
            if let Some(e) = sampler::SAMPLER.lock().as_mut() {
                f(e);
            }
        };
        let (playing, cur, reserved) = sampler::SAMPLER.lock().as_ref().map(|e| (e.seq.playing, e.seq.current(), e.seq.reserved())).unwrap_or((false, 0, None));
        let sp = &mut self.prefs.sampler;
        let synced = sp.sync && !self.px.sampler.master;
        match act {
            SmpAct::Play(n) => self.sampler_pad_down(n as usize, 1.0, false, false),
            SmpAct::Pause(n) => self.sampler_pad_down(n as usize, 1.0, true, false),
            SmpAct::Bank(b) => {
                sp.bank = b.min(3);
                self.px.sampler.close_edit();
            }
            SmpAct::Gain(d) => sp.gain = (sp.gain + d).clamp(0.0, 100.0),
            SmpAct::Volume(d) => sp.volume = (sp.volume + d).clamp(0.0, 100.0),
            SmpAct::Quantize => sp.quantize = !sp.quantize,
            SmpAct::Sync => sp.sync = !sp.sync,
            SmpAct::Master => self.sampler_toggle_master(),
            SmpAct::Bpm(d) => {
                if !synced {
                    sp.bpm = (((sp.bpm + d) * 100.0).round() / 100.0).clamp(40.0, 499.0);
                }
            }
            SmpAct::SeqRec => with_engine(&mut |e| {
                let on = !e.seq.rec;
                e.seq.set_rec(on)
            }),
            SmpAct::SeqPlay => with_engine(&mut |e| if e.seq.playing { e.seq_stop() } else { e.seq_play() }),
            SmpAct::SeqSave => self.sampler_seq_save(),
            SmpAct::SeqStep(dir) => {
                let to = (reserved.unwrap_or(cur) as i32 + dir).rem_euclid(PATTERNS as i32) as usize;
                with_engine(&mut |e| e.seq.select(to, 4.0));
            }
            SmpAct::SeqDelete => self.sampler_seq_delete(cur),
            SmpAct::SeqMode(m) => {
                if playing {
                    self.px.sampler.seq_mode = if self.px.sampler.seq_mode == m { 0 } else { m };
                } else {
                    self.toast_err("Invalid operation. Available during sequencer playback/recording.");
                }
            }
            SmpAct::SeqLoad(d) => self.sampler_sequence_load(d),
            SmpAct::PadMode(m) => {
                let n = self.px.active.min(3) as usize;
                self.px.pad_mode[n] = m;
            }
        }
        self.save_prefs();
    }

    /// SAVE: the pattern as it is goes into its slot (a paid function in rekordbox; free here).
    pub fn sampler_seq_save(&mut self) {
        let mut saved = None;
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            let cur = e.seq.current();
            if e.seq.dirty(cur) {
                saved = Some((cur, e.seq.pattern(cur).clone()));
                e.seq.mark_saved(cur);
            }
        }
        if let Some((cur, mut pat)) = saved {
            pat.events.iter_mut().for_each(|ev| ev.fresh = false);
            pat.events.shrink_to_fit();
            self.prefs.sampler.seq.patterns.resize(PATTERNS, None);
            self.prefs.sampler.seq.patterns[cur] = Some(pat);
            self.save_prefs();
        }
    }

    /// DELETE: the pattern slot is cleared (its name goes back to "PATTERN n").
    pub fn sampler_seq_delete(&mut self, slot: usize) {
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            e.seq.clear(slot);
        }
        let sq = &mut self.prefs.sampler.seq;
        sq.patterns.resize(PATTERNS, None);
        sq.patterns[slot % PATTERNS] = None;
        if let Some(n) = sq.names.get_mut(slot) {
            n.clear();
        }
        self.save_prefs();
    }

    /// SEQ. CALL: start pattern `slot`. Stopped: at once, from its start. Playing: the same
    /// pattern again on the next beat, another one on the next bar (rekordbox's quantize
    /// arguments 0, 1 and 4 beats).
    pub fn sampler_seq_call(&mut self, slot: usize, stop: bool) {
        if self.deck_options.disable_play && !stop {
            return;
        }
        if let Some(e) = sampler::SAMPLER.lock().as_mut() {
            if stop {
                if e.seq.playing && e.seq.current() == slot {
                    e.seq_stop();
                }
            } else if !e.seq.playing {
                e.seq.select(slot, 0.0);
                e.seq_play();
            } else if e.seq.current() == slot {
                e.seq.restart_on(1.0);
            } else {
                e.seq.select(slot, 4.0);
            }
        }
    }

    /// MASTER: the sampler deck becomes the sync master (again: the deck that was master
    /// before is master again).
    pub fn sampler_toggle_master(&mut self) {
        self.px.sampler.master = !self.px.sampler.master;
        self.push_deck_controls();
    }

    /// A deck pad in SAMPLER mode. An empty pad pressed while the deck loops captures the
    /// loop into it ("Capture the loop in an empty slot").
    pub fn sampler_deck_pad_down(&mut self, deck: u8, pad: usize, shift: bool) {
        let idx = self.prefs.sampler.bank.min(3) as usize * PADS + pad;
        if pad < self.sampler_pads() && !self.sampler_loaded(idx) && self.prefs.sampler.capture_empty && self.player.decks[(deck as usize).min(3)].lock().loop_range.is_some() {
            self.sampler_capture_loop(deck, Some(idx));
            return;
        }
        self.sampler_pad_down(pad, 1.0, shift, false);
    }

    /// A slot's play mode for labels: is it a loop sample.
    pub fn sampler_is_loop(&self, idx: usize) -> bool {
        sampler::SAMPLER.lock().as_ref().and_then(|e| e.clip(idx % PADS, idx / PADS).map(|c| c.mode & MODE_LOOP != 0)).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_colours_are_rekordboxs_table_at_80_percent() {
        // Code 60 (#AA72FF), the fallback, and code 45 (#FF127B).
        assert_eq!(pad_color(60), Color32::from_rgb(136, 91, 204));
        assert_eq!(pad_color(45), Color32::from_rgb(204, 14, 98));
        // A code the picker does not offer falls back to 60.
        assert_eq!(pad_color(2), pad_color(60));
        assert_eq!(PAD_COLORS[1], 0x305AFF);
        assert_eq!(PAD_COLORS[62], 0x6473FF);
        assert!(PICKER.iter().all(|c| (1..=62).contains(c)));
    }

    #[test]
    fn velocity_curves_match_rekordboxs_table() {
        assert_eq!(velocity_gain(1, 64), 64.0 / 127.0);
        assert_eq!(velocity_gain(2, 32), 51.0 / 127.0);
        assert_eq!(velocity_gain(2, 64), 92.0 / 127.0);
        assert_eq!(velocity_gain(2, 96), 120.0 / 127.0);
        assert_eq!(velocity_gain(3, 8), 1.0 / 127.0);
        assert_eq!(velocity_gain(3, 64), 21.0 / 127.0);
        assert_eq!(velocity_gain(3, 126), 124.0 / 127.0);
        assert_eq!(velocity_gain(4, 32), 40.0 / 127.0);
        assert_eq!(velocity_gain(4, 64), 80.0 / 127.0);
        assert_eq!(velocity_gain(4, 96), 1.0);
        for c in 1..=4 {
            assert_eq!(velocity_gain(c, 0), 0.0);
            assert_eq!(velocity_gain(c, 127), 1.0);
        }
    }

    #[test]
    fn sampler_actions_parse() {
        assert_eq!(SmpAct::parse("smp_play1"), Some(SmpAct::Play(0)));
        assert_eq!(SmpAct::parse("smp_play16"), Some(SmpAct::Play(15)));
        assert_eq!(SmpAct::parse("smp_play17"), None);
        assert_eq!(SmpAct::parse("smp_pause9"), Some(SmpAct::Pause(8)));
        assert_eq!(SmpAct::parse("smp_bank4"), Some(SmpAct::Bank(3)));
        assert_eq!(SmpAct::parse("seq_load2"), Some(SmpAct::SeqLoad(1)));
        assert_eq!(SmpAct::parse("seq_next"), Some(SmpAct::SeqStep(1)));
        assert!(SmpAct::parse("smp_bpm_up").unwrap().repeats() && !SmpAct::parse("seq_play").unwrap().repeats());
        // rekordbox's Performance preset: X C V B play slots 1 to 4, M , . / slots 9 to 12,
        // with Shift they stop; Cmd+Y / J / K play, previous and next sequence.
        let def = |id: &str| crate::prefs_ext::ACTIONS.iter().find(|a| a.0 == id).map(|a| a.2).unwrap();
        assert_eq!([def("smp_play1"), def("smp_play4"), def("smp_play9"), def("smp_play12")], ["X", "B", "M", "Slash"]);
        assert_eq!([def("smp_pause2"), def("smp_pause10")], ["Shift+C", "Shift+Comma"]);
        assert_eq!([def("seq_play"), def("seq_prev"), def("seq_next"), def("seq_rec"), def("smp_play5")], ["Cmd+Y", "Cmd+J", "Cmd+K", "", ""]);
        for a in crate::prefs_ext::ACTIONS.iter().filter(|a| is_sampler_action(a.0)) {
            assert!(SmpAct::parse(a.0).is_some(), "{}", a.0);
            assert_eq!(a.3, "Sampler");
        }
    }

    #[test]
    fn capture_names_follow_rekordboxs_rule() {
        let dir = tempfile::tempdir().unwrap();
        let long = "A".repeat(60);
        let save = |title: &str| save_capture(dir.path(), title, "_capture", 44100, &[0.0, 0.0]).unwrap();
        assert_eq!(save(&long).file_name().unwrap().to_string_lossy(), format!("{}_capture.wav", "A".repeat(48)));
        assert_eq!(save(&long).file_name().unwrap().to_string_lossy(), format!("{}_capture(1).wav", "A".repeat(48)));
        assert_eq!(save("a/b:c").file_name().unwrap(), "a_b_c_capture.wav");
    }

    #[test]
    fn concurrent_captures_preserve_all_three_files() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("mix_capture.wav");
        std::fs::write(&existing, b"existing audio").unwrap();
        let start = std::sync::Barrier::new(2);
        let save = |sample| {
            start.wait();
            let stereo = [sample, sample];
            let path = save_capture(dir.path(), "mix", "_capture", 44100, &stereo).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), crate::player_ext::wav16_bytes(44100, 2, &stereo));
            path
        };
        let mut paths = std::thread::scope(|scope| {
            let first = scope.spawn(|| save(0.25));
            let second = scope.spawn(|| save(-0.25));
            [first.join().unwrap(), second.join().unwrap()]
        });
        paths.sort();
        assert_eq!(paths, [dir.path().join("mix_capture(1).wav"), dir.path().join("mix_capture(2).wav")]);
        assert_eq!(std::fs::read(existing).unwrap(), b"existing audio");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }
}
