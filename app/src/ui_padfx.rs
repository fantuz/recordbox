//! PAD FX, the pad mode (rekordbox 7.2.19 `djplay::PadFxView`; spec `checks/re-7.2.19/
//! pads-sampler-automix.md` A2): two banks of 16 pads per deck, each pad an effect with its
//! own parameters, on while the pad is held.
//!
//! From rekordbox: the effect list and its ids (0..40 in five groups), which parameters each
//! group has and the values they step through, the three default preset tables (standard,
//! beginner, combination), the press rules (momentary; RELEASE FX pads latch with HOLD ON;
//! one release effect at a time; a held pad of the same effect takes over when the running
//! one is let go; no PAD FX while MERGE FX is on), SLIP LOOP as a deck function, edit mode,
//! the pad's look in the list layout (colour square, "R" box on release pads, name, value at
//! the right) and the default colour code of every effect. A deck's PAD FX unit sits before
//! FX1 / FX2 in its path and runs the multi-mode effect classes, as rekordbox's does.
//! Beat shift on held pads (`PadFxView::onBeatShift` 0x101e19dec): each held pad whose
//! parameters start with BEAT (or have BEAT second, or have one or none; rekordbox's test)
//! steps one entry along 1/16 .. 32 (Beat Up / Beat Down of a controller); the running effect
//! or SLIP LOOP follows, and the pad's own beat comes back when it is let go
//! (`stopFx` / `stopReleaseFx`). The combination pads (`PadFxCombination`, a controller's
//! "PAD1..16_PadFxCombination"): sixteen more pads per deck that always play rekordbox's
//! combination preset (`restorePadFxInfoCombinationPreset`; nothing of them is stored) and
//! have no view of their own, as in rekordbox.
//! recordbox's own: where the bank, edit and page buttons sit under the pads (rekordbox's
//! controller strip is 18 pt high under the pads; its exact place on screen has no capture),
//! a second press ending a latched RELEASE FX pad, and the menu the edit mode opens.

use crate::app::*;
use crate::fx::{self, Kind, PadSet, SlotParams};
use crate::theme::*;
use crate::ui_player::{lighten, press_fired};
use eframe::egui;
use egui::{pos2, vec2, Align2, FontId, Rect, Sense, Stroke, Ui};
use serde::{Deserialize, Serialize};

const PANEL: egui::Color32 = hex(0x1E1E1E);

/// `PlayerExt::pad_mode` value of PAD FX (100 + rekordbox's PadMode 1).
pub const PAD_FX: u8 = 101;

/// rekordbox's `PfxFxTypeName` (0..40). CHORUS and NOISE LFO exist in the enum only.
pub const NAMES: [&str; 41] = ["DELAY", "ECHO", "SPIRAL", "REVERB", "TRANS", "FILTER LFO", "FLANGER", "PHASER", "ROBOT", "SLIP ROLL", "ROLL", "REV ROLL", "MT DELAY", "UP ECHO", "DOWN ECHO", "REV DELAY", "PAN", "CHORUS", "NOISE LFO", "CRUSH", "SPACE", "DUB ECHO", "PITCH", "HPF", "LPF", "SWEEP", "NOISE", "GATE COMP", "SLIP LOOP", "V.BRAKE", "ECHO", "BACKSPIN", "BPF ECHO", "NOISE", "SPIRAL UP", "REVERB UP", "HPF ECHO", "LPF ECHO", "CRUSH ECHO", "SPIRAL DOWN", "REVERB DOWN"];

/// Controller pad colour code of each effect (rekordbox's defaults for its "PAD FX color").
const COLOR_CODE: [u8; 41] = [5, 5, 22, 22, 9, 14, 14, 14, 56, 9, 9, 9, 5, 5, 5, 5, 56, 22, 14, 56, 22, 5, 56, 14, 14, 14, 56, 9, 9, 1, 1, 1, 5, 56, 22, 22, 5, 5, 5, 22, 22];

pub const SLIP_LOOP: u8 = 28;

/// The key Preferences > View > PAD FX color stores an effect's colour under (its name there,
/// with "(RFX)" for the release effects and "(SFX)" for the Scene FX).
pub fn color_key(fx: u8) -> String {
    let name = match fx {
        29 => "VINYL BRAKE",
        31 => "BACK SPIN",
        _ => NAMES[fx.min(40) as usize],
    };
    match group(fx) {
        3 => format!("{name}(RFX)"),
        4 => format!("{name}(SFX)"),
        _ => name.to_string(),
    }
}

/// A pad's colour: the one chosen in Preferences (PAD FX color, 1..9: Pink, Red, Orange,
/// Yellow, Green, Aqua, Blue, Purple, Teal, the page's dot colours), else rekordbox's default
/// controller colour code for the effect.
pub fn pad_fx_color(chosen: &std::collections::BTreeMap<String, u8>, fx: u8) -> egui::Color32 {
    const DOTS: [u32; 9] = [0xFF5AA5, 0xF03C3C, 0xFF8C19, 0xF8E330, 0x24E01F, 0x00DFFE, 0x56B2FB, 0xB133FF, 0x28A092];
    match chosen.get(&color_key(fx)) {
        Some(&c) => hex(DOTS[(c.clamp(1, 9) - 1) as usize]),
        None => crate::sampler_ext::pad_color(COLOR_CODE[fx.min(40) as usize]),
    }
}

/// rekordbox's fxGroup: 0 Beat FX, 1 Sound Color FX, 2 SLIP LOOP, 3 Release FX, 4 Scene FX.
pub fn group(fx: u8) -> u8 {
    match fx {
        0..=18 => 0,
        19..=27 => 1,
        28 => 2,
        29..=31 => 3,
        _ => 4,
    }
}

/// The menu's sections, in rekordbox's order (its Beat FX list leaves out CHORUS and NOISE
/// LFO and ends with SLIP LOOP).
pub const SECTIONS: [(&str, &[u8]); 4] = [("BEAT FX", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 28]), ("SOUND COLOR FX", &[19, 20, 21, 22, 23, 24, 25, 26, 27]), ("SCENE FX", &[32, 33, 34, 35, 36, 37, 38, 39, 40]), ("RELEASE FX", &[29, 30, 31])];

/// The parameters a pad can have (rekordbox's editable types).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Param {
    Beat,
    RoomSize,
    PitchShift,
    Color,
    LevelDepth,
    Hold,
}

impl Param {
    /// The caption of its row in edit mode.
    pub fn caption(self) -> &'static str {
        match self {
            Param::Beat => "BEAT:",
            Param::RoomSize => "ROOM SIZE:",
            Param::PitchShift => "PITCH SHIFT:",
            Param::Color => "COLOR:",
            Param::LevelDepth => "LEVEL/DEPTH:",
            Param::Hold => "HOLD ON/OFF:",
        }
    }
}

/// The (up to two) parameters of an effect (`PadFxPadBase::updateEditableType`).
pub fn params_of(fx: u8) -> [Option<Param>; 2] {
    match (group(fx), fx) {
        (0, 3) => [Some(Param::RoomSize), Some(Param::LevelDepth)],
        (0, 8) => [Some(Param::PitchShift), Some(Param::LevelDepth)],
        (0, _) => [Some(Param::Beat), Some(Param::LevelDepth)],
        (1, 26) => [Some(Param::Color), Some(Param::LevelDepth)],
        (1, _) => [Some(Param::Color), None],
        (2, _) => [Some(Param::Beat), None],
        (3, _) => [Some(Param::Beat), Some(Param::Hold)],
        _ => [Some(Param::LevelDepth), None],
    }
}

/// The beats a pad steps through: 1/16 .. 32 (numerator, denominator).
pub const BEATS: [(u8, u8); 11] = [(1, 16), (1, 8), (1, 4), (1, 2), (3, 4), (1, 1), (2, 1), (4, 1), (8, 1), (16, 1), (32, 1)];

/// The values a parameter steps through.
pub fn values(p: Param) -> Vec<i16> {
    match p {
        Param::Beat => (0..BEATS.len() as i16).collect(),
        Param::RoomSize => std::iter::once(1).chain((1..=10).map(|k| k * 10)).collect(),
        Param::PitchShift | Param::Color => (-5..=5).map(|k| k * 20).collect(),
        Param::LevelDepth => (0..=10).map(|k| k * 10).collect(),
        Param::Hold => vec![0, 1],
    }
}

/// One pad's settings (rekordbox's `PfxFXInfo`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pad {
    pub fx: u8,
    pub num: u8,
    pub den: u8,
    pub room: i16,
    pub pitch: i16,
    pub color: i16,
    pub ld: i16,
    /// 0 = HOLD ON (a release pad latches), 1 = HOLD OFF.
    pub hold: u8,
}

impl Default for Pad {
    fn default() -> Self {
        Pad::of(1)
    }
}

impl Pad {
    /// A newly chosen effect with rekordbox's factory settings (`padFxDefaultList`): Beat FX
    /// 1/1 at 50 (FILTER LFO, FLANGER and PHASER 4/1), REVERB room 50, ROBOT pitch 0, colour
    /// effects 50, SLIP LOOP 1/1, V.BRAKE 3/4, ECHO 1/2, BACKSPIN 1/1, Scene FX 50.
    pub fn of(fx: u8) -> Pad {
        let (num, den) = match fx {
            5..=7 => (4, 1),
            29 => (3, 4),
            30 => (1, 2),
            _ => (1, 1),
        };
        Pad { fx: fx.min(40), num, den, room: 50, pitch: 0, color: 50, ld: 50, hold: 1 }
    }

    fn from_preset(t: (u8, u8, u8, i16, i16)) -> Pad {
        let mut p = Pad::of(t.0);
        if t.1 > 0 {
            (p.num, p.den) = (t.1, t.2);
        }
        match params_of(t.0)[0] {
            Some(Param::RoomSize) => p.room = t.3,
            Some(Param::PitchShift) => p.pitch = t.3,
            Some(Param::Color) => p.color = t.3,
            _ => {}
        }
        if t.4 >= 0 {
            p.ld = t.4;
        }
        p
    }

    pub fn beats(&self) -> f32 {
        self.num.max(1) as f32 / self.den.max(1) as f32
    }

    pub fn get(&self, p: Param) -> i16 {
        match p {
            Param::Beat => BEATS.iter().position(|b| *b == (self.num, self.den)).unwrap_or(5) as i16,
            Param::RoomSize => self.room,
            Param::PitchShift => self.pitch,
            Param::Color => self.color,
            Param::LevelDepth => self.ld,
            Param::Hold => self.hold as i16,
        }
    }

    pub fn set(&mut self, p: Param, v: i16) {
        match p {
            Param::Beat => (self.num, self.den) = BEATS[(v.max(0) as usize).min(BEATS.len() - 1)],
            Param::RoomSize => self.room = v.clamp(1, 100),
            Param::PitchShift => self.pitch = v.clamp(-100, 100),
            Param::Color => self.color = v.clamp(-100, 100),
            Param::LevelDepth => self.ld = v.clamp(0, 100),
            Param::Hold => self.hold = v.clamp(0, 1) as u8,
        }
    }

    /// A parameter's value as the pad shows it (rekordbox's `toString(PfxFXInfo, int)`).
    pub fn text(&self, p: Param) -> String {
        match p {
            Param::Beat => format!("{}/{}", self.num, self.den),
            Param::Hold => if self.hold == 0 { "ON" } else { "OFF" }.into(),
            other => format!("{}", self.get(other)),
        }
    }

    /// What the deck's PAD FX unit is asked for while this pad runs.
    pub fn engine(&self) -> PadSet {
        let ld = self.ld.clamp(0, 100) as f32 / 100.0;
        let slot = |kind: Kind, value: f32| PadSet { beat: Some(SlotParams { on: true, level: ld, value, ..SlotParams::new(kind) }), ..PadSet::NONE };
        let color = |kind: u8, c: f32, p: f32| PadSet { color: Some((kind, c, p)), ..PadSet::NONE };
        let c = 0.5 + self.color.clamp(-100, 100) as f32 / 200.0;
        let b = self.beats();
        match self.fx {
            0 => slot(Kind::Delay, b),
            1 => slot(Kind::Echo, b),
            2 => slot(Kind::Spiral, b),
            3 => slot(Kind::Reverb, self.room as f32),
            4 => slot(Kind::Trans, b),
            5 => slot(Kind::Filter, b),
            6 => slot(Kind::Flanger, b),
            7 => slot(Kind::Phaser, b),
            8 => slot(Kind::Robot, self.pitch as f32),
            9 => slot(Kind::SlipRoll, b),
            10 => slot(Kind::Roll, b),
            11 => slot(Kind::RevRoll, b),
            12 => slot(Kind::MtDelay, b),
            13 => slot(Kind::UpEcho, b),
            14 => slot(Kind::DownEcho, b),
            15 => slot(Kind::RevDelay, b),
            16 => slot(Kind::Pan, b),
            19 => color(2, c, 0.5),
            20 => color(5, c, 0.5),
            21 => color(6, c, 0.5),
            22 => color(4, c, 0.5),
            // (HPF and LPF only know how far they are turned.)
            23 => color(crate::cfx::HPF, 0.5 + (c - 0.5).abs(), 0.5),
            24 => color(crate::cfx::LPF, 0.5 + (c - 0.5).abs(), 0.5),
            25 => color(7, c, 0.5),
            26 => color(3, c, ld),
            27 => color(8, c, 0.5),
            29 => PadSet { release: Some((0, b)), ..PadSet::NONE },
            30 => PadSet { release: Some((1, b)), ..PadSet::NONE },
            31 => PadSet { release: Some((2, b)), ..PadSet::NONE },
            32 => slot(Kind::BpfEcho, 0.0),
            33 => slot(Kind::Noise, 0.0),
            34 => slot(Kind::SpiralUp, 0.0),
            35 => slot(Kind::ReverbUp, 0.0),
            36 => slot(Kind::HpfEcho, 0.0),
            37 => slot(Kind::LpfEcho, 0.0),
            38 => slot(Kind::CrushEcho, 0.0),
            39 => slot(Kind::SpiralDown, 0.0),
            40 => slot(Kind::ReverbDown, 0.0),
            _ => PadSet::NONE,
        }
    }
}

/// rekordbox's default presets as (effect, beat numerator, denominator, room size / pitch /
/// colour, level/depth or -1): the standard one (PAD FX1 then PAD FX2), the beginner one
/// (rekordbox uses it with a DDJ-WeGO4) and the combination one (16 pads).
const STANDARD: [(u8, u8, u8, i16, i16); 32] = [(10, 1, 2, 0, 50), (25, 0, 0, 80, -1), (6, 16, 1, 0, 70), (29, 3, 4, 0, -1), (1, 1, 4, 0, 30), (1, 1, 2, 0, 30), (3, 0, 0, 50, 30), (30, 1, 2, 0, -1), (4, 1, 2, 0, 50), (19, 0, 0, 40, -1), (5, 4, 1, 0, 100), (31, 4, 1, 0, -1), (12, 1, 8, 0, 30), (21, 0, 0, 30, -1), (20, 0, 0, 30, -1), (30, 1, 1, 0, -1), (28, 1, 16, 0, -1), (28, 1, 8, 0, -1), (28, 1, 4, 0, -1), (28, 1, 2, 0, -1), (11, 1, 16, 0, 50), (11, 1, 8, 0, 50), (11, 1, 4, 0, 50), (11, 1, 2, 0, 50), (4, 1, 16, 0, 50), (4, 1, 8, 0, 50), (4, 1, 4, 0, 50), (4, 1, 2, 0, 50), (12, 1, 16, 0, 50), (12, 1, 8, 0, 50), (12, 1, 4, 0, 50), (12, 1, 2, 0, 50)];
const BEGINNER: [(u8, u8, u8, i16, i16); 32] = [(28, 1, 8, 0, -1), (28, 1, 4, 0, -1), (28, 1, 2, 0, -1), (28, 1, 1, 0, -1), (3, 0, 0, 50, 50), (1, 3, 4, 0, 40), (6, 16, 1, 0, 70), (29, 3, 4, 0, -1), (4, 1, 2, 0, 50), (19, 0, 0, 40, -1), (5, 4, 1, 0, 100), (31, 4, 1, 0, -1), (12, 1, 8, 0, 30), (21, 0, 0, 30, -1), (20, 0, 0, 30, -1), (30, 1, 1, 0, -1), (4, 1, 8, 0, 50), (4, 1, 4, 0, 50), (4, 1, 2, 0, 50), (4, 1, 1, 0, 50), (2, 1, 1, 0, 50), (12, 1, 4, 0, 25), (15, 1, 1, 0, 100), (31, 1, 1, 0, -1), (4, 1, 16, 0, 50), (4, 1, 8, 0, 50), (4, 1, 4, 0, 50), (4, 1, 2, 0, 50), (12, 1, 16, 0, 50), (12, 1, 8, 0, 50), (12, 1, 4, 0, 50), (12, 1, 2, 0, 50)];
const COMBINATION: [(u8, u8, u8, i16, i16); 16] = [(29, 3, 4, 0, -1), (31, 4, 1, 0, -1), (30, 1, 2, 0, -1), (30, 1, 1, 0, -1), (21, 0, 0, 30, -1), (3, 0, 0, 50, 30), (25, 0, 0, 80, -1), (1, 1, 4, 0, 30), (10, 1, 2, 0, 50), (6, 1, 16, 0, 70), (1, 1, 2, 0, 30), (1, 1, 2, 0, 30), (1, 1, 2, 0, 30), (1, 1, 2, 0, 30), (1, 1, 2, 0, 30), (1, 1, 2, 0, 30)];

/// A preset's pads: 0 standard, 1 beginner, 2 combination.
pub fn preset(which: u8) -> Vec<Pad> {
    match which {
        1 => BEGINNER.iter().map(|t| Pad::from_preset(*t)).collect(),
        2 => COMBINATION.iter().map(|t| Pad::from_preset(*t)).collect(),
        _ => STANDARD.iter().map(|t| Pad::from_preset(*t)).collect(),
    }
}

/// Pads 0..31 are a deck's two banks, 32..47 the combination pads.
pub const PADS: usize = 48;

/// Whether `onBeatShift` moves this pad's beat: its first parameter is BEAT, it has one or
/// no parameter, or its second is BEAT.
pub fn beat_shiftable(fx: u8) -> bool {
    match params_of(fx) {
        [None, _] | [Some(Param::Beat), _] | [Some(_), None] | [_, Some(Param::Beat)] => true,
        _ => false,
    }
}

/// One step along rekordbox's beat list (`getBeatList`: 1/16 .. 32): up, or down; a beat not
/// in the list goes to its first entry.
pub fn shift_beat(cur: (u8, u8), down: bool) -> (u8, u8) {
    let n = BEATS.len() as i32;
    let i = BEATS.iter().position(|b| *b == cur).map(|i| i as i32).unwrap_or(-1);
    let to = if down { i.max(1) - 1 } else { (i + 1).min(n - 1) };
    BEATS[to as usize]
}

/// What is kept between sessions: every deck's 32 pads (bank * 16 + pad) and the bank shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PadFxPrefs {
    pub decks: [Vec<Pad>; 4],
    pub bank: [u8; 4],
}

impl Default for PadFxPrefs {
    fn default() -> Self {
        PadFxPrefs { decks: std::array::from_fn(|_| preset(0)), bank: [0; 4] }
    }
}

impl PadFxPrefs {
    pub fn pad(&self, deck: usize, idx: usize) -> Pad {
        if idx >= 32 {
            return COMBINATION.get(idx - 32).map(|t| Pad::from_preset(*t)).unwrap_or_default();
        }
        self.decks[deck.min(3)].get(idx).copied().unwrap_or_default()
    }

    /// A deck's 48 pads as they play now: the two banks, the combination pads, and the
    /// beats of held pads shifted (`shift`: pad, numerator, denominator).
    pub fn all(&self, deck: usize, shift: &[(u8, u8, u8)]) -> [Pad; PADS] {
        let mut out: [Pad; PADS] = std::array::from_fn(|i| self.pad(deck, i));
        for &(i, num, den) in shift {
            if let Some(p) = out.get_mut(i as usize) {
                (p.num, p.den) = (num, den);
            }
        }
        out
    }

    fn pad_mut(&mut self, deck: usize, idx: usize) -> &mut Pad {
        let d = &mut self.decks[deck.min(3)];
        if d.len() < 32 {
            let std = preset(0);
            let from = d.len();
            d.extend_from_slice(&std[from..]);
        }
        &mut d[idx.min(31)]
    }
}

/// What a press or release asks the deck to do (SLIP LOOP is the deck's, not the unit's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeckAsk {
    SlipLoop(f32),
    EndSlipLoop,
}

/// A deck's pads as they are being played.
#[derive(Clone, Default)]
pub struct DeckPads {
    /// Held pads (bank * 16 + pad) in the order they were pressed.
    pub held: Vec<u8>,
    /// The pad whose Beat FX or Scene FX runs, the pad whose colour effect runs, the pad
    /// whose RELEASE FX runs (held or latched) and the pad whose SLIP LOOP runs.
    pub beat: Option<u8>,
    pub color: Option<u8>,
    pub release: Option<u8>,
    pub slip: Option<u8>,
    pub edit: bool,
    /// Held pads whose beat has been shifted, and to what (until they are let go).
    pub shift: Vec<(u8, u8, u8)>,
}

impl DeckPads {
    /// Pad `idx` goes down in either editable bank or the combination bank.
    pub fn press(&mut self, pads: &[Pad], idx: u8) -> Option<DeckAsk> {
        let pad = *pads.get(idx as usize)?;
        if !self.held.contains(&idx) {
            self.held.push(idx);
        }
        match group(pad.fx) {
            3 => {
                // One release effect at a time; a latched pad pressed again lets go.
                if self.release == Some(idx) && pad.hold == 0 {
                    self.release = None;
                    self.held.retain(|h| *h != idx);
                } else {
                    self.release = Some(idx);
                }
                None
            }
            2 => {
                self.slip = Some(idx);
                Some(DeckAsk::SlipLoop(pad.beats()))
            }
            1 => {
                self.color = Some(idx);
                None
            }
            _ => {
                self.beat = Some(idx);
                None
            }
        }
    }

    /// Pad `idx` comes up.
    pub fn release(&mut self, pads: &[Pad], idx: u8) -> Option<DeckAsk> {
        let fx_of = |i: u8| pads.get(i as usize).map(|p| p.fx).unwrap_or(255);
        let pad = pads.get(idx as usize).copied().unwrap_or_default();
        self.held.retain(|h| *h != idx);
        // (Its own beat comes back.)
        self.shift.retain(|s| s.0 != idx);
        // Another held pad of the same effect takes over with its own parameters.
        let heir = self.held.iter().rev().copied().find(|h| fx_of(*h) == pad.fx);
        if self.beat == Some(idx) {
            self.beat = heir;
        }
        if self.color == Some(idx) {
            self.color = heir;
        }
        if self.release == Some(idx) && pad.hold != 0 {
            self.release = heir;
        }
        if self.slip == Some(idx) {
            self.slip = heir;
            return Some(match heir {
                Some(h) => DeckAsk::SlipLoop(pads[h as usize].beats()),
                None => DeckAsk::EndSlipLoop,
            });
        }
        None
    }

    /// Everything off (bank change, edit mode, another pad mode).
    pub fn stop(&mut self) -> Option<DeckAsk> {
        let slip = self.slip.take();
        self.held.clear();
        self.shift.clear();
        self.beat = None;
        self.color = None;
        self.release = None;
        slip.map(|_| DeckAsk::EndSlipLoop)
    }

    /// What the deck's unit is asked for now.
    pub fn engine(&self, pads: &[Pad]) -> PadSet {
        let of = |i: Option<u8>| i.and_then(|i| pads.get(i as usize)).map(|p| p.engine()).unwrap_or(PadSet::NONE);
        PadSet { beat: of(self.beat).beat, color: of(self.color).color, release: of(self.release).release }
    }

    /// `onBeatShift`: every held pad that has a beat steps it once (`pads`: as they play now).
    /// SLIP LOOP on a shifted pad changes its loop.
    pub fn beat_shift(&mut self, pads: &[Pad], down: bool) -> Option<DeckAsk> {
        let mut ask = None;
        for &idx in &self.held {
            let Some(p) = pads.get(idx as usize) else { continue };
            if !beat_shiftable(p.fx) {
                continue;
            }
            let (num, den) = shift_beat((p.num, p.den), down);
            match self.shift.iter_mut().find(|s| s.0 == idx) {
                Some(s) => (s.1, s.2) = (num, den),
                None => self.shift.push((idx, num, den)),
            }
            if self.slip == Some(idx) {
                ask = Some(DeckAsk::SlipLoop(num.max(1) as f32 / den.max(1) as f32));
            }
        }
        ask
    }

    pub fn lit(&self, idx: u8) -> bool {
        self.beat == Some(idx) || self.color == Some(idx) || self.release == Some(idx) || self.slip == Some(idx)
    }
}

/// The PAD FX state of the four decks (not kept between sessions).
pub struct PadFxState {
    pub decks: [DeckPads; 4],
    /// The slip switch as it was before a SLIP LOOP pad turned it on.
    slip_was: [Option<bool>; 4],
    /// Decks whose PAD FX unit has been made.
    made: [bool; 4],
    /// Where each deck's eight pads and its bank 1, bank 2, edit and page buttons were last
    /// drawn (for the scripted run).
    pub rects: [[Rect; 12]; 4],
}

impl Default for PadFxState {
    fn default() -> Self {
        PadFxState { decks: Default::default(), slip_was: [None; 4], made: [false; 4], rects: [[Rect::NOTHING; 12]; 4] }
    }
}

impl PadFxState {
    fn press(&mut self, n: usize, pads: &[Pad], idx: u8) -> Option<DeckAsk> {
        let deck = &mut self.decks[n];
        let ask = deck.press(pads, idx);
        // A MIDI or keyboard pad can sound before this deck's pad grid is drawn.
        // Keep the unit alive after release so its delay/reverb tails can finish.
        self.made[n] |= deck.beat.is_some() || deck.color.is_some() || deck.release.is_some();
        ask
    }

    /// What each deck's unit is asked for (for the audio thread).
    pub fn sets(&self, prefs: &PadFxPrefs) -> [PadSet; 4] {
        std::array::from_fn(|d| {
            let deck = &self.decks[d];
            if deck.beat.is_none() && deck.color.is_none() && deck.release.is_none() { return PadSet::NONE; }
            deck.engine(&prefs.all(d, &deck.shift))
        })
    }

    /// Decks whose PAD FX grid was shown or whose effect was activated.
    pub fn wanted(&self) -> [bool; 4] {
        self.made
    }
}

/// Carry out what a press or release asked of deck `n` (`app.deck` is that deck).
fn deck_ask(app: &mut App, n: usize, ask: Option<DeckAsk>) {
    match ask {
        Some(DeckAsk::SlipLoop(beats)) => {
            if app.deck.track.is_none() {
                return;
            }
            if app.px.fxx.pad.slip_was[n].is_none() {
                app.px.fxx.pad.slip_was[n] = Some(app.deck.x.slip);
            }
            if !app.deck.x.slip {
                app.toggle_slip();
            }
            if app.loop_active().is_some() {
                app.loop_exit();
            }
            app.auto_loop(beats as f64);
        }
        Some(DeckAsk::EndSlipLoop) => {
            if app.loop_active().is_some() {
                app.loop_exit();
            }
            if let Some(was) = app.px.fxx.pad.slip_was[n].take() {
                if app.deck.x.slip != was {
                    app.toggle_slip();
                }
            }
        }
        None => {}
    }
}

/// Press pad `idx` of deck `n` (also what a key or controller pad would call).
pub fn pad_down(app: &mut App, n: usize, idx: u8) {
    if crate::mfx::any_on(app) {
        app.toast_err("Invalid operation. PAD FX cannot be operated when MERGE FX is ON.");
        return;
    }
    let n = n.min(3);
    let pads = app.prefs.fx.padfx.all(n, &app.px.fxx.pad.decks[n].shift);
    let ask = app.px.fxx.pad.press(n, &pads, idx);
    deck_ask(app, n, ask);
}

/// Beat Up / Beat Down on deck `n` (a controller's Parameter1Right / Left of PAD FX): the held
/// pads' beats step.
pub fn beat_shift(app: &mut App, n: usize, down: bool) {
    let pads = app.prefs.fx.padfx.all(n.min(3), &app.px.fxx.pad.decks[n.min(3)].shift);
    let ask = app.px.fxx.pad.decks[n.min(3)].beat_shift(&pads, down);
    deck_ask(app, n, ask);
}

pub fn pad_up(app: &mut App, n: usize, idx: u8) {
    let pads = app.prefs.fx.padfx.all(n.min(3), &app.px.fxx.pad.decks[n.min(3)].shift);
    let ask = app.px.fxx.pad.decks[n.min(3)].release(&pads, idx);
    deck_ask(app, n, ask);
}

/// Deck `n` leaves PAD FX (or changes bank, or enters edit mode): everything of it stops.
pub fn stop_deck(app: &mut App, n: usize) {
    let ask = app.px.fxx.pad.decks[n.min(3)].stop();
    deck_ask(app, n, ask);
}

/// The pads of deck `n`: eight of the bank's sixteen (the second page is rekordbox's Shift
/// page), drawn as rekordbox's list layout draws a PAD FX pad. `pad(column, row)` gives a
/// pad's rectangle; `strip` is the row under the pads beside the pad mode pull-down.
pub fn pads(app: &mut App, ui: &mut Ui, n: u8, pad: &dyn Fn(usize, usize) -> Rect, strip: Rect) {
    let p = ui.painter().clone();
    let d = (n as usize).min(3);
    app.px.fxx.pad.made[d] = true;
    let bank = app.prefs.fx.padfx.bank[d].min(1) as usize;
    let page = app.deck.x.pad_page.min(1) as usize;
    let edit = app.px.fxx.pad.decks[d].edit;
    let released = ui.input(|i| i.pointer.any_released());
    let held_key = egui::Id::new(("padfx_held", n));
    for i in 0..8usize {
        let idx = (bank * 16 + page * 8 + i) as u8;
        let pr = pad(i / 4, i % 4);
        app.px.fxx.pad.rects[d][i] = pr;
        let mut info = app.prefs.fx.padfx.pad(d, idx as usize);
        if let Some(s) = app.px.fxx.pad.decks[d].shift.iter().find(|s| s.0 == idx) {
            // (A shifted beat shows while the pad is held.)
            (info.num, info.den) = (s.1, s.2);
        }
        let lit = app.px.fxx.pad.decks[d].lit(idx);
        let resp = ui.interact(pr, ui.id().with(("padfx", n, i)), Sense::click_and_drag());
        p.rect_filled(pr, 0.0, if lit { gy(0x2C2C2C) } else if resp.hovered() { gy(0x1E1E1E) } else { gy(0x161616) });
        if lit {
            p.rect_stroke(pr, 0.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
        }
        // The effect's colour: an 8 x 8 square, 6 in from the left.
        let col = pad_fx_color(&app.prefs.rb.pad_fx_colors, info.fx);
        p.rect_filled(Rect::from_min_size(pos2(pr.min.x + 6.0, (pr.center().y - 4.0).round()), vec2(8.0, 8.0)), 0.0, if lit { lighten(col) } else { col });
        let text = gy(0xF4F4F4);
        let mut x = pr.min.x + 20.0;
        if group(info.fx) == 3 {
            // RELEASE FX pads: an "R" in a box.
            let b = Rect::from_min_size(pos2(x, (pr.center().y - 5.5).round()), vec2(11.0, 11.0));
            p.rect_filled(b, 0.0, text);
            p.text(b.center(), Align2::CENTER_CENTER, "R", bold(8.5), gy(0x161616));
            x += 14.0;
        }
        let [p1, p2] = params_of(info.fx);
        let value = match (p1, p2) {
            (Some(Param::Beat), _) | (Some(Param::RoomSize), _) | (Some(Param::PitchShift), _) | (Some(Param::Color), _) => info.text(p1.unwrap()),
            (Some(other), _) => info.text(other),
            _ => String::new(),
        };
        p.with_clip_rect(pr).text(pos2(x, pr.center().y), Align2::LEFT_CENTER, NAMES[info.fx.min(40) as usize], FontId::proportional(10.5), text);
        p.text(pos2(pr.max.x - 6.0, pr.center().y), Align2::RIGHT_CENTER, value, FontId::proportional(10.5), if lit { hex(0x3C8CFF) } else { text });
        if edit {
            // Edit mode: a click opens the pad's settings.
            crate::ui_browser::dropdown(ui, &resp, &format!("padfxedit{n}{i}"), |ui| edit_menu(app, ui, d, idx));
            continue;
        }
        if press_fired(ui, &resp) {
            pad_down(app, d, idx);
            ui.ctx().data_mut(|m| m.insert_temp(held_key, idx));
        }
        let _ = resp.on_hover_text(pad_tip(&info));
    }
    if released {
        if let Some(h) = ui.ctx().data_mut(|m| m.remove_temp::<u8>(held_key)) {
            pad_up(app, d, h);
        }
    }
    // The strip: bank "1" / "2", the edit button, and the page (pads 9 to 16 are the
    // controller's Shift page in rekordbox).
    let left = n % 2 == 0;
    let mut x = if left { strip.min.x } else { strip.max.x };
    let mut cell = |w: f32| {
        let r = if left { Rect::from_min_size(pos2(x, strip.min.y), vec2(w, strip.height())) } else { Rect::from_min_size(pos2(x - w, strip.min.y), vec2(w, strip.height())) };
        x += if left { w + 2.0 } else { -(w + 2.0) };
        r
    };
    for b in 0..2usize {
        let r = cell(30.0);
        app.px.fxx.pad.rects[d][8 + b] = r;
        let resp = ui.interact(r, ui.id().with(("padfxbank", n, b)), Sense::click());
        p.rect_filled(r, 2.0, if bank == b { BLUE.c() } else if resp.hovered() { lighten(BTN.c()) } else { PANEL });
        p.text(r.center(), Align2::CENTER_CENTER, format!("{}", b + 1), bold(9.5), WHITE.c());
        if resp.on_hover_text(format!("PAD FX{}", b + 1)).clicked() && bank != b {
            stop_deck(app, d);
            app.prefs.fx.padfx.bank[d] = b as u8;
            app.save_prefs();
        }
    }
    let r = cell(20.0);
    app.px.fxx.pad.rects[d][10] = r;
    let resp = ui.interact(r, ui.id().with(("padfxedit", n)), Sense::click());
    p.rect_filled(r, 2.0, if edit { BLUE.c() } else if resp.hovered() { lighten(BTN.c()) } else { PANEL });
    // (A small gear: a ring with six teeth.)
    let c = r.center();
    p.circle_stroke(c, 3.2, Stroke::new(1.6_f32, WHITE));
    for k in 0..6 {
        let a = k as f32 * std::f32::consts::TAU / 6.0;
        p.line_segment([c + vec2(a.cos(), a.sin()) * 4.0, c + vec2(a.cos(), a.sin()) * 5.6], Stroke::new(1.6_f32, WHITE));
    }
    if resp.on_hover_text("Edit the PAD FX pads").clicked() {
        stop_deck(app, d);
        app.px.fxx.pad.decks[d].edit = !edit;
    }
    let r = cell(40.0);
    app.px.fxx.pad.rects[d][11] = r;
    let resp = ui.interact(r, ui.id().with(("padfxpage", n)), Sense::click());
    p.rect_filled(r, 2.0, if resp.hovered() { lighten(BTN.c()) } else { PANEL });
    p.text(r.center(), Align2::CENTER_CENTER, if page == 0 { "1 - 8" } else { "9 - 16" }, FontId::proportional(9.0), DIM2.c());
    if resp.on_hover_text("Page").clicked() {
        app.deck.x.pad_page = 1 - page as u8;
    }
}

fn pad_tip(pad: &Pad) -> String {
    let mut s = NAMES[pad.fx.min(40) as usize].to_string();
    for q in params_of(pad.fx).into_iter().flatten() {
        s.push_str(&format!("\n{} {}", q.caption(), pad.text(q)));
    }
    s
}

/// Edit mode's menu for one pad: the effect (rekordbox's four sections) and its parameters.
fn edit_menu(app: &mut App, ui: &mut Ui, d: usize, idx: u8) {
    let cur = app.prefs.fx.padfx.pad(d, idx as usize);
    let mut next = cur;
    for (title, list) in SECTIONS {
        ui.menu_button(title, |ui| {
            for fx in list.iter() {
                if ui.selectable_label(cur.fx == *fx, NAMES[*fx as usize]).clicked() {
                    next = Pad::of(*fx);
                    ui.close_menu();
                }
            }
        });
    }
    ui.separator();
    for q in params_of(cur.fx).into_iter().flatten() {
        ui.horizontal(|ui| {
            ui.label(q.caption());
            let mut t = cur;
            egui::ComboBox::from_id_salt(("padfxparam", d, idx, q.caption())).width(64.0).selected_text(cur.text(q)).show_ui(ui, |ui| {
                for v in values(q) {
                    t.set(q, v);
                    if ui.selectable_label(cur.get(q) == v, t.text(q)).clicked() {
                        next.set(q, v);
                    }
                }
            });
        });
    }
    if next != cur {
        *app.prefs.fx.padfx.pad_mut(d, idx as usize) = next;
        app.save_prefs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_groups_and_parameters() {
        let std = preset(0);
        assert_eq!((std.len(), preset(1).len(), preset(2).len()), (32, 32, 16));
        // Standard PAD FX1: ROLL 1/2 at 50, SWEEP colour 80, FLANGER 16/1 at 70, V.BRAKE 3/4 ...
        assert_eq!((NAMES[std[0].fx as usize], std[0].text(Param::Beat), std[0].ld), ("ROLL", "1/2".to_string(), 50));
        assert_eq!((NAMES[std[1].fx as usize], std[1].color), ("SWEEP", 80));
        assert_eq!((NAMES[std[2].fx as usize], std[2].text(Param::Beat), std[2].ld), ("FLANGER", "16/1".to_string(), 70));
        assert_eq!((NAMES[std[3].fx as usize], std[3].text(Param::Beat), std[3].hold), ("V.BRAKE", "3/4".to_string(), 1));
        assert_eq!((NAMES[std[6].fx as usize], std[6].room, std[6].ld), ("REVERB", 50, 30));
        assert_eq!((NAMES[std[15].fx as usize], std[15].text(Param::Beat)), ("ECHO", "1/1".to_string()));
        // PAD FX2 starts with four SLIP LOOPs (1/16 .. 1/2) and ends with MT DELAY 1/2.
        assert!(std[16..20].iter().all(|p| p.fx == SLIP_LOOP) && std[16].text(Param::Beat) == "1/16");
        assert_eq!((NAMES[std[31].fx as usize], std[31].text(Param::Beat)), ("MT DELAY", "1/2".to_string()));
        // Beginner starts with SLIP LOOP 1/8; combination with V.BRAKE 3/4, BACKSPIN 4/1.
        assert_eq!((preset(1)[0].fx, preset(1)[0].text(Param::Beat)), (SLIP_LOOP, "1/8".to_string()));
        assert_eq!((NAMES[preset(2)[1].fx as usize], preset(2)[1].text(Param::Beat)), ("BACKSPIN", "4/1".to_string()));
        // Groups and parameters.
        assert_eq!((group(16), group(19), group(27), group(28), group(31), group(32), group(40)), (0, 1, 1, 2, 3, 4, 4));
        assert_eq!(params_of(1), [Some(Param::Beat), Some(Param::LevelDepth)]);
        assert_eq!(params_of(3), [Some(Param::RoomSize), Some(Param::LevelDepth)]);
        assert_eq!(params_of(8), [Some(Param::PitchShift), Some(Param::LevelDepth)]);
        assert_eq!(params_of(25), [Some(Param::Color), None]);
        assert_eq!(params_of(26), [Some(Param::Color), Some(Param::LevelDepth)]);
        assert_eq!(params_of(30), [Some(Param::Beat), Some(Param::Hold)]);
        assert_eq!(params_of(37), [Some(Param::LevelDepth), None]);
        assert_eq!(values(Param::RoomSize), vec![1, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
        assert_eq!(values(Param::Color), vec![-100, -80, -60, -40, -20, 0, 20, 40, 60, 80, 100]);
        assert_eq!((values(Param::LevelDepth).len(), values(Param::Beat).len()), (11, 11));
        // Factory settings of a newly chosen effect.
        assert_eq!((Pad::of(6).text(Param::Beat), Pad::of(29).text(Param::Beat), Pad::of(30).text(Param::Beat), Pad::of(31).text(Param::Beat)), ("4/1".into(), "3/4".into(), "1/2".into(), "1/1".into()));
        // Every offered effect reaches the engine (SLIP LOOP is the deck's).
        for (_, list) in SECTIONS {
            for fx in list.iter() {
                assert_eq!(Pad::of(*fx).engine().any(), *fx != SLIP_LOOP, "{}", NAMES[*fx as usize]);
            }
        }
        let e = Pad { ld: 30, ..Pad::of(1) }.engine().beat.unwrap();
        assert!(e.on && e.kind == Kind::Echo && (e.level - 0.3).abs() < 1e-6 && e.value == 1.0);
        assert_eq!(Pad { color: 80, ..Pad::of(25) }.engine().color, Some((7, 0.9, 0.5)));
        assert_eq!(Pad::of(31).engine().release, Some((2, 1.0)));
    }

    #[test]
    fn an_unrendered_controller_pad_requests_only_its_own_effect_unit_and_keeps_its_tail() {
        let pads = [Pad::of(SLIP_LOOP), Pad::of(1), Pad::of(25), Pad { hold: 0, ..Pad::of(30) }];
        for (i, fx) in pads.iter().enumerate() {
            let mut state = PadFxState::default();
            assert_eq!(state.wanted(), [false; 4]);
            state.press(2, &pads, i as u8);
            let expected = [false, false, fx.fx != SLIP_LOOP, false];
            assert_eq!(state.wanted(), expected);
            assert_eq!(state.decks[2].engine(&pads).any(), expected[2]);
            state.decks[2].release(&pads, i as u8);
            assert_eq!(state.wanted(), expected);
        }
    }

    #[test]
    fn hold_stack_and_latch_rules() {
        // Pads: 0 ECHO 1/4, 1 ECHO 1/2, 2 TRANS, 3 V.BRAKE (HOLD OFF), 4 ECHO release with
        // HOLD ON, 5 SLIP LOOP, 6 SWEEP.
        let mut pads = vec![Pad { num: 1, den: 4, ..Pad::of(1) }, Pad { num: 1, den: 2, ..Pad::of(1) }, Pad::of(4), Pad::of(29), Pad { hold: 0, ..Pad::of(30) }, Pad::of(SLIP_LOOP), Pad::of(25)];
        pads.resize(32, Pad::of(0));
        let mut d = DeckPads::default();
        // Momentary: on while held.
        assert_eq!(d.press(&pads, 0), None);
        assert_eq!(d.engine(&pads).beat.unwrap().value, 0.25);
        d.release(&pads, 0);
        assert!(!d.engine(&pads).any());
        // Stacking: with ECHO 1/4 and ECHO 1/2 held, the last pressed runs; letting it go
        // hands the effect to the other with its parameters; letting that go ends it.
        d.press(&pads, 0);
        d.press(&pads, 1);
        assert_eq!(d.engine(&pads).beat.unwrap().value, 0.5);
        d.release(&pads, 1);
        assert_eq!((d.beat, d.engine(&pads).beat.unwrap().value), (Some(0), 0.25));
        d.release(&pads, 0);
        assert!(d.beat.is_none());
        // A held pad of another effect does not take over.
        d.press(&pads, 2);
        d.press(&pads, 0);
        d.release(&pads, 0);
        assert!(d.beat.is_none() && d.held == vec![2]);
        d.release(&pads, 2);
        // A colour pad runs beside a Beat FX pad.
        d.press(&pads, 0);
        d.press(&pads, 6);
        let e = d.engine(&pads);
        assert!(e.beat.is_some() && e.color.is_some());
        d.stop();
        // Release pads: HOLD OFF is momentary; HOLD ON stays after the pad comes up, until
        // it is pressed again or another release pad takes its place.
        d.press(&pads, 3);
        assert_eq!(d.engine(&pads).release, Some((0, 0.75)));
        d.release(&pads, 3);
        assert!(d.release.is_none());
        d.press(&pads, 4);
        d.release(&pads, 4);
        assert_eq!(d.engine(&pads).release, Some((1, 0.5)));
        d.press(&pads, 3);
        assert_eq!(d.engine(&pads).release, Some((0, 0.75)), "one release effect at a time");
        d.release(&pads, 3);
        d.press(&pads, 4);
        d.release(&pads, 4);
        d.press(&pads, 4);
        assert!(d.release.is_none(), "a second press lets a latched pad go");
        d.release(&pads, 4);
        // SLIP LOOP asks the deck, and ends when the pad comes up.
        assert_eq!(d.press(&pads, 5), Some(DeckAsk::SlipLoop(1.0)));
        assert!(d.lit(5) && !d.engine(&pads).any());
        assert_eq!(d.release(&pads, 5), Some(DeckAsk::EndSlipLoop));
        // Stopping everything (bank change, edit mode) ends a running slip loop too.
        d.press(&pads, 5);
        d.press(&pads, 0);
        assert_eq!(d.stop(), Some(DeckAsk::EndSlipLoop));
        assert!(d.held.is_empty() && !d.engine(&pads).any());
    }

    #[test]
    fn beat_shift_on_held_pads_and_combination_pads() {
        // rekordbox's list and steps: up to 32 at most, down to 1/16 at least.
        assert_eq!((shift_beat((1, 2), false), shift_beat((1, 2), true), shift_beat((32, 1), false), shift_beat((1, 16), true), shift_beat((5, 7), false)), ((3, 4), (1, 4), (32, 1), (1, 16), (1, 16)));
        // Which pads move: BEAT first or second, or one parameter or none (REVERB and ROBOT
        // do not, NOISE (colour + level) does not).
        assert!(beat_shiftable(1) && beat_shiftable(28) && beat_shiftable(29) && beat_shiftable(19) && beat_shiftable(34));
        assert!(!beat_shiftable(3) && !beat_shiftable(8) && !beat_shiftable(26));
        let prefs = PadFxPrefs::default();
        let mut d = DeckPads::default();
        // Standard bank 1: pad 5 ECHO 1/2, pad 6 REVERB, pad 16 SLIP LOOP 1/16.
        let pads = prefs.all(0, &d.shift);
        assert_eq!((pads[5].fx, pads[5].beats(), pads[6].fx, pads[16].fx), (1, 0.5, 3, SLIP_LOOP));
        d.press(&pads, 6);
        d.press(&pads, 5);
        assert_eq!(d.beat_shift(&pads, false), None);
        let pads = prefs.all(0, &d.shift);
        assert_eq!((pads[5].num, pads[5].den, pads[6].room), (3, 4, 50));
        // The running effect follows; letting go brings the pad's own beat back.
        d.release(&pads, 6);
        assert_eq!(d.engine(&prefs.all(0, &d.shift)).beat.unwrap().value, 0.75);
        d.release(&prefs.all(0, &d.shift), 5);
        assert!(d.shift.is_empty() && prefs.all(0, &d.shift)[5].beats() == 0.5);
        // SLIP LOOP held: Beat Down halves its loop.
        let pads = prefs.all(0, &d.shift);
        assert_eq!(d.press(&pads, 17), Some(DeckAsk::SlipLoop(0.125)));
        assert_eq!(d.beat_shift(&pads, true), Some(DeckAsk::SlipLoop(0.0625)));
        // Combination pads (33..48): rekordbox's combination preset, whatever the banks hold.
        let mut p2 = PadFxPrefs::default();
        p2.decks[0][0] = Pad::of(7);
        let all = p2.all(0, &[]);
        assert_eq!((all[32].fx, all[32].num, all[32].den, all[33].fx, all[37].fx, all[37].room), (29, 3, 4, 31, 3, 50));
        let mut c = DeckPads::default();
        c.press(&all, 37);
        assert_eq!(c.engine(&all).beat.unwrap().kind, Kind::Reverb);
    }

    #[test]
    fn pads_are_kept_between_sessions() {
        let mut p = PadFxPrefs::default();
        assert_eq!(p.decks[0], preset(0));
        *p.pad_mut(1, 17) = Pad { ld: 80, ..Pad::of(37) };
        p.pad_mut(1, 0).set(Param::Beat, 10);
        p.bank[1] = 1;
        let json = serde_json::to_string(&p).unwrap();
        let back: PadFxPrefs = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!((back.pad(1, 17).fx, back.pad(1, 17).ld, back.pad(1, 0).text(Param::Beat)), (37, 80, "32/1".to_string()));
        // An older file without the part, or with a short list, still gives 32 pads.
        let old: PadFxPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(old, PadFxPrefs::default());
        let mut short: PadFxPrefs = serde_json::from_str(r#"{"decks":[[{"fx":4}],[],[],[]]}"#).unwrap();
        assert_eq!(short.pad(0, 0).fx, 4);
        assert_eq!(short.pad_mut(0, 20).fx, preset(0)[20].fx);
    }
}
