//! The performance pad host and the rules of its pad modes, after rekordbox 7.2.19
//! (`checks/re-7.2.19/pads-sampler-automix.md`, section 0 and A; addresses below are arm64
//! VAs of that build). Nothing here draws or touches a deck: `pads_ext.rs` acts on the decks,
//! `pad_audio.rs` is the audio thread's part and `ui_pads.rs` draws.

use serde::{Deserialize, Serialize};

/// rekordbox's `PadMode` (`MemoryCuePanel::getPadModeText` 0x100681e94), with its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadMode {
    HotCue,
    PadFx,
    Slicer,
    BeatJump,
    BeatLoop,
    Keyboard,
    KeyShift,
    Sampler,
    SamplerScratch,
    SeqCall,
    ActiveCensor,
    Stems,
    MemoryCue,
    /// USER 1..8 (0..7).
    User(u8),
}

/// The pull-down's entries in rekordbox's order (`popupTabMenu` 0x1006820f0, menu ids 1..11).
/// SAMPLER and SP.SCRATCH are pad modes of the controllers only; USER 1..8 are added from the
/// Pad Editor.
pub const MENU: [PadMode; 11] = [PadMode::HotCue, PadMode::PadFx, PadMode::Slicer, PadMode::BeatJump, PadMode::BeatLoop, PadMode::Keyboard, PadMode::KeyShift, PadMode::SeqCall, PadMode::ActiveCensor, PadMode::Stems, PadMode::MemoryCue];

/// `PlayerExt::pad_mode` holds a byte per deck. The six modes that existed before this file
/// keep the bytes they had (other code sets them); every other mode is 32 + rekordbox's value.
/// PAD FX is the FX track's: its byte is `PAD_FX_CODE`.
pub const PAD_FX_CODE: u8 = crate::ui_padfx::PAD_FX;
/// PAD FX has a pad view (`ui_perf::pads` draws it for `PAD_FX_CODE`). False until the FX
/// track's arm is merged: the pull-down then lists PAD FX greyed out.
pub const PAD_FX_READY: bool = true;

impl PadMode {
    /// rekordbox's enum value.
    pub fn value(self) -> u8 {
        match self {
            PadMode::HotCue => 0,
            PadMode::PadFx => 1,
            PadMode::Slicer => 2,
            PadMode::BeatJump => 3,
            PadMode::BeatLoop => 4,
            PadMode::Keyboard => 5,
            PadMode::KeyShift => 6,
            PadMode::Sampler => 7,
            PadMode::SamplerScratch => 8,
            PadMode::SeqCall => 9,
            PadMode::ActiveCensor => 10,
            PadMode::Stems => 11,
            PadMode::MemoryCue => 12,
            PadMode::User(i) => 13 + i.min(7),
        }
    }

    pub fn from_value(v: u8) -> Option<PadMode> {
        Some(match v {
            0 => PadMode::HotCue,
            1 => PadMode::PadFx,
            2 => PadMode::Slicer,
            3 => PadMode::BeatJump,
            4 => PadMode::BeatLoop,
            5 => PadMode::Keyboard,
            6 => PadMode::KeyShift,
            7 => PadMode::Sampler,
            8 => PadMode::SamplerScratch,
            9 => PadMode::SeqCall,
            10 => PadMode::ActiveCensor,
            11 => PadMode::Stems,
            12 => PadMode::MemoryCue,
            13..=20 => PadMode::User(v - 13),
            _ => return None,
        })
    }

    /// The byte kept in `PlayerExt::pad_mode`.
    pub fn code(self) -> u8 {
        match self {
            PadMode::HotCue => 0,
            PadMode::Stems => 1,
            PadMode::BeatJump => 2,
            PadMode::BeatLoop => 3,
            PadMode::Sampler => 4,
            PadMode::SeqCall => 5,
            PadMode::PadFx => PAD_FX_CODE,
            m => 32 + m.value(),
        }
    }

    pub fn from_code(c: u8) -> PadMode {
        match c {
            0 => PadMode::HotCue,
            1 => PadMode::Stems,
            2 => PadMode::BeatJump,
            3 => PadMode::BeatLoop,
            4 => PadMode::Sampler,
            5 => PadMode::SeqCall,
            PAD_FX_CODE => PadMode::PadFx,
            c if c >= 32 => PadMode::from_value(c - 32).unwrap_or(PadMode::HotCue),
            _ => PadMode::HotCue,
        }
    }

    /// The pull-down's caption.
    pub fn label(self) -> String {
        match self {
            PadMode::HotCue => "HOT CUE".into(),
            PadMode::PadFx => "PAD FX".into(),
            PadMode::Slicer => "SLICER".into(),
            PadMode::BeatJump => "BEAT JUMP".into(),
            PadMode::BeatLoop => "BEAT LOOP".into(),
            PadMode::Keyboard => "KEYBOARD".into(),
            PadMode::KeyShift => "KEY SHIFT".into(),
            PadMode::Sampler => "SAMPLER".into(),
            PadMode::SamplerScratch => "SP.SCRATCH".into(),
            PadMode::SeqCall => "SEQ. CALL".into(),
            PadMode::ActiveCensor => "ACT. CENSR".into(),
            PadMode::Stems => "STEMS".into(),
            PadMode::MemoryCue => "MEMORY CUE".into(),
            PadMode::User(i) => format!("USER {}", i + 1),
        }
    }

    /// The pad views this file's host draws itself. The others (STEMS, SAMPLER, SEQ. CALL and
    /// PAD FX) are drawn by `ui_perf::pads`, which then calls the host for the pull-down.
    pub fn own_view(self) -> bool {
        !matches!(self, PadMode::Stems | PadMode::Sampler | PadMode::SeqCall | PadMode::PadFx | PadMode::SamplerScratch)
    }
}

/// What `canSelectPadMode` (0x100683a94) looks at.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectCtx {
    /// The deck is an external input (recordbox has none; always false today).
    pub external: bool,
    /// PC MASTER OUT is on (PAD FX on an external input needs it).
    pub pc_mix: bool,
    pub loaded: bool,
    pub has_grid: bool,
    /// Playing time in seconds (negative before the start of the track).
    pub pos: f64,
}

pub const MSG_SLICER_GRID: &str = "Invalid operation. Input BPM on GRID panel.";
pub const MSG_BEFORE_START: &str = "Invalid operation. Cannot set Cue points before 00m00s.";
pub const MSG_EXTERNAL: &str = "Invalid operation. Use connected DJ mixer/controller in External Mixer mode.";
pub const MSG_KEY_RESET: &str = "KEY RESET unavailable. The key depends on the Tempo Slider position.";
pub const MSG_SLICER_ON: &str = "Invalid operation. Not available when Slicer is on.";
pub const MSG_SLICER_SLIP: &str = "Invalid operation. SLIP button is not available when Slicer is on.";
pub const MSG_SLICER_SLOTS: &str = "Invalid operation. Empty 8 slots for Slicer Capture.";
pub const MSG_CAPTURE: &str = "Invalid operation. Capture is available during Loop or Slicer performance.";

/// `canSelectPadMode`: Ok, or the status message rekordbox shows (None: refused silently).
pub fn can_select(mode: PadMode, c: SelectCtx) -> Result<(), Option<&'static str>> {
    match mode {
        PadMode::HotCue | PadMode::BeatJump | PadMode::BeatLoop | PadMode::Keyboard | PadMode::KeyShift | PadMode::ActiveCensor | PadMode::Stems | PadMode::User(_) => {
            if c.external {
                Err(None)
            } else {
                Ok(())
            }
        }
        PadMode::PadFx => {
            if c.external && !c.pc_mix {
                Err(Some(MSG_EXTERNAL))
            } else {
                Ok(())
            }
        }
        PadMode::Slicer => {
            if c.external {
                Err(None)
            } else if c.loaded && !c.has_grid {
                Err(Some(MSG_SLICER_GRID))
            } else if c.loaded && c.pos < 0.0 {
                Err(Some(MSG_BEFORE_START))
            } else {
                Ok(())
            }
        }
        PadMode::Sampler | PadMode::SeqCall => Ok(()),
        // (MEMORY CUE is a panel of its own, shown from the same menu.)
        PadMode::MemoryCue => {
            if c.external {
                Err(None)
            } else {
                Ok(())
            }
        }
        PadMode::SamplerScratch => Err(None),
    }
}

/// The menu for a deck: on an external input only SEQ. CALL, plus PAD FX with PC MASTER OUT.
pub fn menu_for(c: SelectCtx) -> Vec<PadMode> {
    if c.external {
        let mut v = Vec::new();
        if c.pc_mix {
            v.push(PadMode::PadFx);
        }
        v.push(PadMode::SeqCall);
        v
    } else {
        MENU.to_vec()
    }
}

/// rekordbox's `PadModeLayoutType` (View > PAD Mode): 1 = Auto, 2 = LIST Display, 3 = PAD
/// Display. `PlayerComponent::shouldShowAsPadLayout` (0x102292314): PAD Display, or Auto with
/// a controller that has pads.
pub fn pad_layout(layout_type: u8, controller_pads: u32) -> bool {
    layout_type == 3 || (layout_type == 1 && controller_pads > 0)
}

/// Settings of the pads kept between runs (`Prefs::pads`).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PadsPrefs {
    /// `PadModeLayoutType`: 1 Auto, 2 LIST Display, 3 PAD Display.
    pub layout: u8,
    /// `BeatJumpPageDeck1..4`: 1..9, default 5.
    pub beat_jump_page: [u8; 4],
    /// `HotCueColorType`: 1 CDJ, 2 COLD1, 3 COLD2, 4 COLORFUL (default).
    pub hot_cue_color: u8,
    /// The slicer's LENGTH and ROLL selections per deck (indices into `SLICER_LENGTHS` and
    /// `SLICER_ROLLS`), as `getSlicerLength` / `getSlicerRoll` hand them out for saving.
    pub slicer_length: [u8; 4],
    pub slicer_roll: [u8; 4],
    /// KEYBOARD and KEY SHIFT pages per deck (1..5, start 2).
    pub keyboard_page: [u8; 4],
    pub key_shift_page: [u8; 4],
    /// The Pad Editor's changes: for USER 1..8, the 16 items that differ from the template
    /// shipped with rekordbox (None: the template's), as "function=...,value=..." lines.
    pub user: Vec<Vec<Option<String>>>,
    /// USER modes listed in the pull-down (the Pad Editor's `mode` flag).
    pub user_shown: [bool; 8],
    /// The Pad Editor: the mode each of a controller's eight pad mode buttons selects, for
    /// ASSIGN 1 and ASSIGN 2 (`select` of `Mode.pad.csv`; empty = rekordbox's default), the
    /// assign shown (`PadEditor Assign`) and the mode button picked.
    pub editor_modes: Vec<Vec<String>>,
    pub editor_assign: u8,
    pub editor_slot: u8,
}

impl Default for PadsPrefs {
    fn default() -> PadsPrefs {
        PadsPrefs { layout: 1, beat_jump_page: [5; 4], hot_cue_color: 4, slicer_length: [2; 4], slicer_roll: [4; 4], keyboard_page: [2; 4], key_shift_page: [2; 4], user: vec![vec![None; 16]; 8], user_shown: [false; 8], editor_modes: Vec::new(), editor_assign: 0, editor_slot: 0 }
    }
}

// ---------------------------------------------------------------------------------------------
// HOT CUE colours

/// The four colour sets of Preferences > View > Color > HOT CUE (`HotCueColorType` 1..4), as
/// palette indices for hot cues A..H (I..P repeat them): `rekordbox::hotCueColorIndexCDJ`,
/// `Cold1`, `Cold2` and `Colorfull` (0x103824327). CDJ is 22 for A..H and 26 for I..P.
pub const HOT_CUE_SETS: [(&str, [u8; 8]); 4] = [("CDJ", [22; 8]), ("COLD1", [4, 18, 58, 8, 12, 60, 14, 1]), ("COLD2", [14, 11, 8, 5, 3, 1, 61, 59]), ("COLORFUL", [43, 8, 23, 60, 19, 38, 1, 32])];

/// The colour a new hot cue in `slot` (0 = A) gets with colour set `set` (1..4).
pub fn hot_cue_set_color(set: u8, slot: i32, is_loop: bool) -> u8 {
    if is_loop {
        return crate::model::HOTCUE_LOOP_DEFAULT;
    }
    let s = (set.clamp(1, 4) - 1) as usize;
    let slot = slot.clamp(0, 15) as usize;
    if s == 0 && slot >= 8 {
        return 26;
    }
    HOT_CUE_SETS[s].1[slot % 8]
}

// ---------------------------------------------------------------------------------------------
// BEAT JUMP (`djplay::BeatJumpView`, ctor 0x101dd51e4)

/// The twelve cells (`BeatJumpView::labelTable`).
pub const JUMP_LABELS: [&str; 12] = ["FINE", "1/8", "1/4", "1/2", "1", "2", "4", "8", "16", "32", "64", "128"];
/// FINE: `BeatSearchBehavior::doBeatSearch` (0x102aecf80) moves by
/// `floor(UnitOfTime::defaultSampleRate * 5.0)` samples when the beat fraction is zero, and
/// `defaultSampleRate` is 44.1 (samples per millisecond): 220 samples, 5 ms.
pub const JUMP_FINE_SAMPLES: f64 = 220.0;
pub const JUMP_FINE_SECS: f64 = JUMP_FINE_SAMPLES / 44100.0;

/// Beats of a logical pad id 0..23 (`onPadPressed` 0x101dda6c0: id = 2 * cell + (0 back,
/// 1 forward)); None for FINE (ids 0 and 1).
pub fn jump_beats(id: u8) -> Option<f64> {
    match id {
        0 | 1 => None,
        2..=7 => Some(1.0 / 2f64.powi(((9 - id) >> 1) as i32)),
        _ => Some(2f64.powi(((id.min(23) - 8) >> 1) as i32)),
    }
}

/// Forward (true) or back.
pub fn jump_forward(id: u8) -> bool {
    id & 1 == 1
}

/// First logical id a controller's pads cover on a page: clamp(2 * page - 2, 0, 16).
pub fn jump_page_first(page: u8) -> u8 {
    (2 * page.clamp(1, 9) as i32 - 2).clamp(0, 16) as u8
}

/// The four cells PAD Display shows on a page (2 x 2): from cell `first / 2`.
pub fn jump_page_cells(page: u8) -> [usize; 4] {
    let c = (jump_page_first(page) / 2) as usize;
    [c, c + 1, c + 2, c + 3]
}

/// The page buttons (`PageSelector`, lambdas at 0x101dde15c / 0x101dde25c): one page down or
/// up; with Shift to 1 or 5 going down (from a page above 5: 5) and to 5 or 9 going up.
pub fn jump_page_step(page: u8, up: bool, shift: bool) -> u8 {
    let p = page.clamp(1, 9);
    let n = match (up, shift) {
        (true, false) => p + 1,
        (false, false) => p.saturating_sub(1),
        (true, true) => {
            if p >= 5 {
                9
            } else {
                5
            }
        }
        (false, true) => {
            if p <= 5 {
                1
            } else {
                5
            }
        }
    };
    n.clamp(1, 9)
}

// ---------------------------------------------------------------------------------------------
// BEAT LOOP (`djplay::BeatLoopView`, ctor 0x10123c16c)

/// The sixteen pads' labels (the lengths are `player_ext::LOOP_SIZES`).
pub const LOOP_LABELS: [&str; 16] = ["1/64", "1/32", "1/16", "1/8", "1/4", "1/2", "1", "2", "4", "8", "16", "32", "64", "128", "256", "512"];

/// The pad lit for a running loop of `beats` (`updateLoopState` 0x101240bb8 compares the
/// loop's length as a fraction with the labels).
pub fn loop_pad_for(beats: f64) -> Option<usize> {
    crate::player_ext::LOOP_SIZES.iter().position(|b| (beats / b - 1.0).abs() < 0.02)
}

// ---------------------------------------------------------------------------------------------
// KEYBOARD / KEY SHIFT (`djplay::KeyPositioner`, ctor 0x10184e93c, scale 0)

pub const KEY_NONE: i32 = i32::MAX;
pub const KEY_SYNC: i32 = 1000;
pub const KEY_RESET: i32 = 1001;
pub const KEY_SEMI_UP: i32 = 1002;
pub const KEY_SEMI_DOWN: i32 = 1003;
/// 1004..1007: +7, +12, -5, -12 on the last page.
const KEY_ABS: [i32; 4] = [7, 12, -5, -12];

/// Position (page * 8 + pad) to key: semitones, a function code or `KEY_NONE` (0x1037ea3d4).
pub fn key_at(pos: usize) -> i32 {
    match pos {
        0 => 12,
        4..=7 => pos as i32 + 4,
        8..=11 => pos as i32 - 4,
        12..=15 => pos as i32 - 12,
        16..=19 => pos as i32 - 20,
        20..=23 => pos as i32 - 28,
        24..=27 => pos as i32 - 36,
        32 => KEY_SYNC,
        33 => KEY_SEMI_UP,
        34 => 1004,
        35 => 1005,
        36 => KEY_RESET,
        37 => KEY_SEMI_DOWN,
        38 => 1006,
        39 => 1007,
        _ => KEY_NONE,
    }
}

/// Scale 1 of `KeyPositioner` (0x1037ea66c), used when a DDJ-GRV6 or XDJ-AN is connected
/// (`KeyboardView::updateScale`): -12 .. +12 in a row over pages 1-4, then KEY SYNC, KEY
/// RESET, SEMITONE DOWN, SEMITONE UP, -12, -5, +7, +12. Its first page is 3.
pub fn key_at_scale1(pos: usize) -> i32 {
    match pos {
        4..=28 => pos as i32 - 16,
        32 => KEY_SYNC,
        33 => KEY_RESET,
        34 => KEY_SEMI_DOWN,
        35 => KEY_SEMI_UP,
        36 => 1007,
        37 => 1006,
        38 => 1004,
        39 => 1005,
        _ => KEY_NONE,
    }
}

/// `key_at` for scale 0 or 1.
pub fn key_at_in(pos: usize, scale1: bool) -> i32 {
    if scale1 { key_at_scale1(pos) } else { key_at(pos) }
}

/// LIST Display's place of position `pos` in scale 1 (0x1037ea864): -12 .. +12, then the
/// last page's +7, -5, -12, SEMITONE UP, SEMITONE DOWN, KEY SYNC, KEY RESET (+12 unlisted).
pub fn key_list_pos_scale1(pos: usize) -> Option<usize> {
    Some(match pos {
        4..=28 => pos - 4,
        32 => 30,
        33 => 31,
        34 => 29,
        35 => 28,
        36 => 27,
        37 => 26,
        38 => 25,
        _ => return None,
    })
}

/// The controllers that switch KEYBOARD / KEY SHIFT to scale 1.
pub fn key_scale1_device(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n.contains("DDJ-GRV6") || n.contains("XDJ-AN")
}

/// The semitones of a key item (None for the function items and blanks).
pub fn key_semis(key: i32) -> Option<i32> {
    match key {
        -12..=12 => Some(key),
        1004..=1007 => Some(KEY_ABS[(key - 1004) as usize]),
        _ => None,
    }
}

/// A pad's label; `list`: on one line (LIST Display replaces the line break with a space).
pub fn key_label(pos: usize, list: bool) -> String {
    let s = match key_at(pos) {
        KEY_NONE => String::new(),
        KEY_SYNC => "KEY\nSYNC".into(),
        KEY_RESET => "KEY\nRESET".into(),
        KEY_SEMI_UP => "SEMITONE\nUP".into(),
        KEY_SEMI_DOWN => "SEMITONE\nDOWN".into(),
        0 => "0".into(),
        k => {
            let n = key_semis(k).unwrap_or(0);
            if n > 0 {
                format!("+{n}")
            } else {
                format!("{n}")
            }
        }
    };
    if list {
        s.replace('\n', " ")
    } else {
        s
    }
}

/// LIST Display's order: the place (0..31) of position `pos` in the list, or None when the
/// position is not listed (0x1037ea5cc): +12 down to -11, then the last page's -12, -5, +12,
/// +7, SEMITONE UP, SEMITONE DOWN, KEY SYNC, KEY RESET.
pub fn key_list_pos(pos: usize) -> Option<usize> {
    Some(match pos {
        0 => 0,
        4..=7 => 8 - pos,
        8..=11 => 16 - pos,
        12..=15 => 24 - pos,
        16..=19 => 32 - pos,
        20..=23 => 40 - pos,
        25..=27 => 48 - pos,
        32 => 30,
        33 => 28,
        34 => 27,
        35 => 26,
        36 => 31,
        37 => 29,
        38 => 25,
        39 => 24,
        _ => return None,
    })
}

/// The positions in LIST Display's order.
pub fn key_list() -> Vec<usize> {
    let mut v: Vec<(usize, usize)> = (0..40).filter_map(|p| key_list_pos(p).map(|l| (l, p))).collect();
    v.sort();
    v.into_iter().map(|x| x.1).collect()
}

/// `key_list` for scale 0 or 1.
pub fn key_list_in(scale1: bool) -> Vec<usize> {
    if !scale1 {
        return key_list();
    }
    let mut v: Vec<(usize, usize)> = (0..40).filter_map(|p| key_list_pos_scale1(p).map(|l| (l, p))).collect();
    v.sort();
    v.into_iter().map(|x| x.1).collect()
}

/// `key_label` for scale 0 or 1.
pub fn key_label_in(pos: usize, list: bool, scale1: bool) -> String {
    if !scale1 {
        return key_label(pos, list);
    }
    let s = match key_at_scale1(pos) {
        KEY_NONE => String::new(),
        KEY_SYNC => "KEY\nSYNC".into(),
        KEY_RESET => "KEY\nRESET".into(),
        KEY_SEMI_UP => "SEMITONE\nUP".into(),
        KEY_SEMI_DOWN => "SEMITONE\nDOWN".into(),
        0 => "0".into(),
        k => {
            let n = key_semis(k).unwrap_or(0);
            if n > 0 { format!("+{n}") } else { format!("{n}") }
        }
    };
    if list { s.replace('\n', " ") } else { s }
}

/// The position a controller's pad `p` (0..15) plays on `page` (1..5): the page's eight, the
/// next page's on the shift pads (`onPadPressed` 0x101466d80).
pub fn key_pad_pos(page: u8, p: usize) -> usize {
    let page = page.clamp(1, 5) as usize;
    if p < 8 {
        p + 8 * (page - 1)
    } else {
        (p & 7) + 8 * page
    }
}

/// What pressing a KEY SHIFT pad does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyAct {
    /// Set the key shift to this many semitones.
    Set(i32),
    Sync,
    /// Refused, with rekordbox's status message (or none).
    Refuse(Option<&'static str>),
    Nothing,
}

/// `onKeyPressedInternal` (0x101d65478): the range is ±12 semitones; SEMITONE UP only from
/// +11 or lower and DOWN only from -11 or higher; KEY RESET needs Master Tempo.
pub fn key_press(key: i32, current: i32, master_tempo: bool) -> KeyAct {
    match key {
        KEY_NONE => KeyAct::Nothing,
        KEY_SYNC => KeyAct::Sync,
        KEY_RESET => {
            if master_tempo {
                KeyAct::Set(0)
            } else {
                KeyAct::Refuse(Some(MSG_KEY_RESET))
            }
        }
        KEY_SEMI_UP => {
            if current <= 11 {
                KeyAct::Set(current + 1)
            } else {
                KeyAct::Refuse(None)
            }
        }
        KEY_SEMI_DOWN => {
            if current >= -11 {
                KeyAct::Set(current - 1)
            } else {
                KeyAct::Refuse(None)
            }
        }
        k => match key_semis(k) {
            Some(n) if (-12..=12).contains(&n) => KeyAct::Set(n),
            _ => KeyAct::Nothing,
        },
    }
}

// ---------------------------------------------------------------------------------------------
// SLICER (`djplay::SlicerView` 0x100740b28, `rbxfrm::SlicerBehavior` 0x102b50dc0)

/// LENGTH: the domain in beats. Always eight sections.
pub const SLICER_LENGTHS: [u32; 4] = [2, 4, 8, 16];
/// ROLL: (numerator, denominator) beats (table off_1051B0BD0).
pub const SLICER_ROLLS: [(u32, u32); 8] = [(1, 32), (1, 16), (1, 8), (1, 4), (1, 2), (1, 1), (2, 1), (4, 1)];
pub const SLICER_UNITS: usize = 8;

pub fn slicer_roll_label(i: usize) -> String {
    let (n, d) = SLICER_ROLLS[i.min(7)];
    format!("{n}/{d}")
}

/// Beats of one section for LENGTH index `li`: 1/4, 1/2, 1, 2.
pub fn slicer_unit_beats(li: usize) -> f64 {
    SLICER_LENGTHS[li.min(3)] as f64 / SLICER_UNITS as f64
}

pub fn slicer_roll_beats(ri: usize) -> f64 {
    let (n, d) = SLICER_ROLLS[ri.min(7)];
    n as f64 / d as f64
}

/// Choosing LENGTH index `li` sets ROLL index `li + 2` (2 -> 1/8, 4 -> 1/4, 8 -> 1/2, 16 -> 1/1).
pub fn slicer_roll_for_length(li: usize) -> usize {
    li.min(3) + 2
}

/// A chosen ROLL index is kept inside [li, li + 3] (`onSelectionChanged` 0x100744868).
pub fn slicer_clamp_roll(li: usize, ri: usize) -> usize {
    ri.clamp(li.min(3), li.min(3) + 3)
}

/// A beat position (index into the grid, with a fraction between beats) as seconds. Outside
/// the grid the first or last beat interval carries on.
pub fn beat_time(grid: &[f64], b: f64) -> f64 {
    let n = grid.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return grid[0];
    }
    let i = (b.floor() as i64).clamp(0, n as i64 - 2) as usize;
    grid[i] + (b - i as f64) * (grid[i + 1] - grid[i])
}

/// The beat position (index with fraction) of time `t`.
pub fn time_beat(grid: &[f64], t: f64) -> f64 {
    let n = grid.len();
    if n < 2 {
        return 0.0;
    }
    let i = grid.partition_point(|x| *x <= t).saturating_sub(1).min(n - 2);
    i as f64 + (t - grid[i]) / (grid[i + 1] - grid[i]).max(1e-9)
}

/// The slicer's domain (`setDomain` 0x102b53d2c): nine boundaries in seconds, section k from
/// beat `start + k * unit`. The start is moved back so that the domain fits the grid, and a
/// last boundary past the track is the track's last sample.
pub fn slicer_domain(grid: &[f64], start: f64, unit: f64, duration: f64) -> (f64, [f64; 9]) {
    let last = grid.len().saturating_sub(1) as f64;
    let start = start.min(last - SLICER_UNITS as f64 * unit).max(0.0);
    let mut b = [0.0; 9];
    for (k, v) in b.iter_mut().enumerate() {
        *v = beat_time(grid, start + k as f64 * unit);
    }
    if duration > 0.0 && b[8] > duration {
        b[8] = duration;
    }
    (start, b)
}

/// SLICER (not SLICER LOOP): where the domain starts once playback at beat `beat` has left it
/// (`updateDomain` 0x102b530b8): the anchor plus whole domains.
pub fn slicer_advance(anchor: f64, beat: f64, domain_beats: f64) -> f64 {
    anchor + ((beat - anchor) / domain_beats).floor() * domain_beats
}

/// The shift buttons (`doShiftDomain` 0x102b52cec): one domain back or forward, kept inside
/// [0, beats - 2].
pub fn slicer_shift(start: f64, forward: bool, domain_beats: f64, grid_len: usize) -> f64 {
    let t = start + if forward { domain_beats } else { -domain_beats };
    t.clamp(0.0, (grid_len as f64 - 2.0).max(0.0))
}

/// The section (0..7) time `t` is in, or None outside the domain.
pub fn slicer_section(b: &[f64; 9], t: f64) -> Option<usize> {
    if t < b[0] || t >= b[8] {
        return None;
    }
    Some((0..8).rev().find(|k| t >= b[*k]).unwrap_or(0))
}

/// The pads held down, newest last (`padPressed` 0x100744204): a press selects that section;
/// letting go of the newest selects the one held before it (or none); letting go of an older
/// one only forgets it. Returns the section to select now: Some(Some(k)), Some(None) for
/// "back to the background position", or None when nothing changes.
pub fn slicer_stack(stack: &mut Vec<u8>, pad: u8, down: bool) -> Option<Option<u8>> {
    if down {
        stack.retain(|p| *p != pad);
        stack.push(pad);
        return Some(Some(pad));
    }
    let newest = stack.last().copied() == Some(pad);
    stack.retain(|p| *p != pad);
    if newest {
        Some(stack.last().copied())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// ACT. CENSR (`djplay::ActiveCensorView` 0x1010818bc, `rbxfrm::ActiveCensorBehavior` 0x102ae165c)

/// The four effects, in pad order (effect type 0..3).
pub const CENSOR_FX: [&str; 4] = ["REV ROLL", "TRANS", "ECHO", "V.BRAKE"];
/// A new region's beat per effect type (xmmword_1038FFFE0): 32/1, 1/4, 1/8, 3/4.
pub const CENSOR_DEFAULT_BEATS: [(u16, u16); 4] = [(32, 1), (1, 4), (1, 8), (3, 4)];
/// V.BRAKE's beats, longest first (`onBeatLeft` 0x101085054 steps towards the shorter).
pub const CENSOR_BRAKE_BEATS: [(u16, u16); 11] = [(32, 1), (16, 1), (8, 1), (4, 1), (2, 1), (1, 1), (3, 4), (1, 2), (1, 4), (1, 8), (1, 16)];
/// Shortest region: `sampleRate * 0.001 * DjPlayerConfig[+40]` (`addRange`, `setInPoint`),
/// the same 6.667 ms as the shortest loop.
pub const CENSOR_MIN_SECS: f64 = crate::deck::MIN_LOOP_SECS;

/// Moving the edge of region `i` while adjusting (`ActiveCensorBehavior::setInPoint` /
/// `setOutPoint`): the region keeps the minimum length, stays at or after 0 and does not
/// reach into its neighbours.
pub fn censor_edge_ok(regions: &[CensorRegion], i: usize, a: f64, b: f64) -> bool {
    a >= 0.0 && b - a >= CENSOR_MIN_SECS && !regions.iter().enumerate().any(|(j, r)| j != i && a < r.b && r.a < b)
}

/// A censored region of a track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CensorRegion {
    /// Row id in the database (0: not saved).
    pub id: i64,
    /// In and out, seconds.
    pub a: f64,
    pub b: f64,
    /// The beat of each effect type as (numerator, denominator).
    pub beats: [(u16, u16); 4],
}

impl CensorRegion {
    pub fn beat(&self, kind: usize) -> f64 {
        let (n, d) = self.beats[kind.min(3)];
        n as f64 / d.max(1) as f64
    }
}

/// `addRange` (0x102ae3a40 / 0x102ae354c): refused when out is not after in, the region is
/// shorter than the minimum or it overlaps another one (`isIncludeOtherRange` 0x102ae38b0).
pub fn censor_can_add(regions: &[CensorRegion], a: f64, b: f64) -> bool {
    b > a && b - a >= CENSOR_MIN_SECS && a >= 0.0 && !regions.iter().any(|r| a < r.b && r.a < b)
}

/// The region time `t` is in.
pub fn censor_at(regions: &[CensorRegion], t: f64) -> Option<usize> {
    regions.iter().position(|r| t >= r.a && t < r.b)
}

/// The side buttons (`callPrevInPoint` 0x102ae5470 / `callNextInPoint` 0x102ae55dc): the in
/// point before or after `t`.
pub fn censor_neighbour(regions: &[CensorRegion], t: f64, next: bool) -> Option<f64> {
    if next {
        regions.iter().map(|r| r.a).filter(|a| *a > t + 0.001).fold(None, |m: Option<f64>, a| Some(m.map_or(a, |m| m.min(a))))
    } else {
        regions.iter().map(|r| r.a).filter(|a| *a < t - 0.05).fold(None, |m: Option<f64>, a| Some(m.map_or(a, |m| m.max(a))))
    }
}

/// The beat buttons: TRANS is 1/8 (left) or 1/4 (right); V.BRAKE steps through its list (left
/// = shorter); the other two effects have no beat to change (`setEffectBeat` 0x102ae4550).
pub fn censor_beat_step(kind: usize, cur: (u16, u16), right: bool) -> (u16, u16) {
    match kind {
        1 => {
            if right {
                (1, 4)
            } else {
                (1, 8)
            }
        }
        3 => {
            let i = CENSOR_BRAKE_BEATS.iter().position(|b| *b == cur).unwrap_or(6);
            let j = if right { i.saturating_sub(1) } else { (i + 1).min(CENSOR_BRAKE_BEATS.len() - 1) };
            CENSOR_BRAKE_BEATS[j]
        }
        _ => cur,
    }
}

pub fn fraction_text(b: (u16, u16)) -> String {
    if b.1 <= 1 {
        format!("{}", b.0)
    } else {
        format!("{}/{}", b.0, b.1)
    }
}

// ---------------------------------------------------------------------------------------------
// USER 1..8 and the Pad Editor (`PadEditorData` 0x1020eea2c, `PadCsv` 0x1020f9a2c)

/// Where rekordbox keeps the pad mode templates (read when needed; nothing is copied).
pub const PAD_DIR: &str = "/Applications/rekordbox 7/rekordbox.app/Contents/Resources/pad";

/// One pad of a USER mode.
#[derive(Clone, Debug, PartialEq)]
pub struct PadItem {
    pub function: String,
    pub value: String,
    /// `key=` of a hot cue item (KEY n), if any.
    pub key: Option<i32>,
}

/// One record of a `.pad.csv` file: comma-separated key=value pairs, split at the first "=";
/// `userfunction` / `uservalue` take the place of `function` / `value` inside a USER mode.
pub fn parse_pad_line(line: &str) -> Option<PadItem> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (mut function, mut value, mut uf, mut uv, mut key) = (None, None, None, None, None);
    for part in line.split(',') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let v = v.trim().to_string();
        match k.trim() {
            "function" => function = Some(v),
            "value" => value = Some(v),
            "userfunction" => uf = Some(v),
            "uservalue" => uv = Some(v),
            "key" => key = v.parse::<i32>().ok(),
            _ => {}
        }
    }
    let function = uf.or(function)?;
    Some(PadItem { function, value: uv.or(value).unwrap_or_default(), key })
}

/// The item lines of a mode file (up to 16: pads 1..8 and the shift page).
pub fn parse_pad_csv(text: &str) -> Vec<PadItem> {
    text.lines().filter_map(parse_pad_line).filter(|i| i.function != "Version").take(16).collect()
}

/// rekordbox's template for USER `n` (0..7), read from its installation; empty without it.
pub fn user_template(n: usize) -> Vec<PadItem> {
    std::fs::read_to_string(format!("{PAD_DIR}/User{}.pad.csv", n + 1)).map(|t| parse_pad_csv(&t)).unwrap_or_default()
}

/// The Pad Editor's functions for one pad and the values each offers (`PadMenu`'s tables).
pub fn pad_functions() -> Vec<(&'static str, &'static str, Vec<String>)> {
    let hot: Vec<String> = (0..16).map(|i| ((b'A' + i) as char).to_string()).collect();
    let jump: Vec<String> = ["-", "+"].iter().flat_map(|s| JUMP_LABELS.iter().map(move |l| format!("{s}{l}"))).collect();
    let mut key: Vec<String> = vec!["SEMITONE UP".into(), "SEMITONE DOWN".into(), "KEY SYNC".into(), "KEY RESET".into()];
    key.extend((-12..=12).map(|n: i32| if n > 0 { format!("+{n}") } else { format!("{n}") }));
    vec![
        ("HotCue", "HOT CUE/LOOP", hot),
        ("UserBeatJump", "BEAT JUMP", jump),
        ("BeatLoop", "BEAT LOOP", LOOP_LABELS.iter().map(|s| s.to_string()).collect()),
        ("UserKeyShift", "KEY SHIFT", key),
        ("Sampler", "SAMPLER", (1..=16).map(|n| format!("{n}")).collect()),
        ("SequenceCall", "SEQ. CALL", (1..=8).map(|n| format!("{n}")).collect()),
        ("PadFx1", "PAD FX1", (1..=16).map(|n| format!("{n}")).collect()),
        ("PadFx2", "PAD FX2", (1..=16).map(|n| format!("{n}")).collect()),
        ("Transport", "TRANSPORT", TRANSPORT_VALUES.iter().map(|s| s.to_string()).collect()),
        ("SampleScratch", "SP.SCRATCH", (1..=16).map(|n| format!("{n}")).collect()),
    ]
}

/// The transport items in `PadMenu::transportMenu2` order, with the BPM steps of
/// `transportMenu3`.
pub const TRANSPORT_VALUES: [&str; 17] = ["PLAY/PAUSE", "CUE", "PITCH BEND -", "PITCH BEND +", "BPM - SMALL", "BPM - MEDIUM", "BPM - LARGE", "BPM + SMALL", "BPM + MEDIUM", "BPM + LARGE", "CUE/LOOP CALL <", "CUE/LOOP CALL >", "TEMPO RESET", "SLIP", "PREVIOUS TRACK", "NEXT TRACK", "ACTIVE LOOP"];

/// One row of rekordbox's `Mode.pad.csv` (read from the installed app at run time).
#[derive(Clone, Debug, PartialEq)]
pub struct ModeEntry {
    pub function: String,
    /// separator / preset / user / combination.
    pub kind: String,
    /// Listed as a pad mode (`mode`), as a template (`template`), its items as pad items (`pad`).
    pub mode: bool,
    pub template: bool,
    pub pad: bool,
    pub text: String,
    /// The controller's pad mode button it is assigned to (1..8).
    pub select: Option<u8>,
}

/// `Mode.pad.csv` (`PadEditorData::read`): booleans are on / yes / true.
pub fn read_mode_csv() -> Vec<ModeEntry> {
    let Ok(t) = std::fs::read_to_string(format!("{PAD_DIR}/Mode.pad.csv")) else { return Vec::new() };
    let yes = |v: &str| matches!(v.trim().to_ascii_lowercase().as_str(), "on" | "yes" | "true");
    t.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| {
            let mut e = ModeEntry { function: String::new(), kind: String::new(), mode: false, template: false, pad: false, text: String::new(), select: None };
            for part in l.trim().split(',') {
                let Some((k, v)) = part.split_once('=') else { continue };
                match k.trim() {
                    "function" => e.function = v.trim().into(),
                    "type" => e.kind = v.trim().into(),
                    "mode" => e.mode = yes(v),
                    "template" => e.template = yes(v),
                    "pad" => e.pad = yes(v),
                    "text" => e.text = v.trim().into(),
                    "select" => e.select = v.trim().parse().ok(),
                    _ => {}
                }
            }
            e
        })
        .collect()
}

/// The sixteen items of a preset or combination mode (`<function>.pad.csv`).
pub fn mode_items(function: &str) -> Vec<PadItem> {
    std::fs::read_to_string(format!("{PAD_DIR}/{function}.pad.csv")).map(|t| parse_pad_csv(&t)).unwrap_or_default()
}

/// The Pad Editor's window (`PadEditorComponent::show` / `resized` / `resizedPadItem`): 640 x 578;
/// a 30 pt black tab strip (ASSIGN 1, ASSIGN 2) on #323232; "PERFORMANCE MODE" at (25, 54);
/// eight mode buttons 140 x 25 at x 25 + 150 i, y 86 / 121; "PERFORMANCE PADS" at (25, 171);
/// TEMPLATES 140 x 30 at (25, 203); sixteen pads 140 x 70 at (25 + 150 c, 243 + 80 r).
pub mod editor_geo {
    pub const W: f32 = 640.0;
    pub const H: f32 = 578.0;
    pub const TAB_H: f32 = 30.0;
    pub const MODE_LABEL_Y: f32 = 54.0;
    pub const PADS_LABEL_Y: f32 = 171.0;
    pub const LABEL_H: f32 = 15.0;
    pub fn mode_button(i: usize) -> (f32, f32, f32, f32) {
        (25.0 + 150.0 * (i % 4) as f32, 86.0 + 35.0 * (i / 4) as f32, 140.0, 25.0)
    }
    pub const TEMPLATES: (f32, f32, f32, f32) = (25.0, 203.0, 140.0, 30.0);
    pub fn pad(i: usize) -> (f32, f32, f32, f32) {
        (25.0 + 150.0 * (i % 4) as f32, 243.0 + 80.0 * (i / 4) as f32, 140.0, 70.0)
    }
    /// Colours (`__GLOBAL__sub_I_PadEditorComponent`, `createComponent`): background, tab
    /// strip, button fill, outline (normal / on), text (normal / on).
    pub const BG: u32 = 0x323232;
    pub const STRIP: u32 = 0x000000;
    pub const FILL: u32 = 0x191919;
    pub const OUTLINE: u32 = 0x969696;
    pub const TEXT: u32 = 0xC8C8C8;
    pub const ON: u32 = 0x1473EB;
}

/// What a USER pad does, from its item.
#[derive(Clone, Debug, PartialEq)]
pub enum UserAct {
    HotCue(i32),
    /// Beats (None: FINE) and forward.
    BeatJump(Option<f64>, bool),
    BeatLoop(f64),
    Key(i32),
    Sampler(usize),
    SeqCall(usize),
    /// PAD FX: bank 0 = PAD FX1, 1 = PAD FX2, 2 = the combination bank; pad 0..15
    /// (devices `PAD%d_PadFx1`, `PAD%d_PadFx2`, `PAD%d_PadFxCombination`).
    PadFx(u8, u8),
    /// A transport item (`function=Transport`), by rekordbox's device name.
    Transport(Transport),
    /// SP.SCRATCH slot 0..15 (`@SampleScratchSlot%d`).
    SampleScratch(usize),
    /// A function recordbox does not have on a pad (lighting).
    Other,
}

/// The Pad Editor's transport items (`PadMenu::transportMenu2/3`) and the devices they press
/// (the `function=Transport,value=...,name=...` table in the binary).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transport {
    PlayPause,
    Cue,
    PitchBend(i8),
    /// BPM - / + by SMALL, MEDIUM or LARGE: `TempoDec*` / `TempoInc*` move the tempo slider
    /// by 10, 50 or 200 hundredths of a percent (`WidgetPlayerDJ::operate` switch ids 92 /
    /// 93: `slider +- n * 0.01 / range`).
    Bpm(i8, u16),
    CueLoopCall(i8),
    TempoReset,
    Slip,
    Track(i8),
    ActiveLoop,
}

/// The transport item of a value text.
pub fn transport_of(v: &str) -> Option<Transport> {
    Some(match v {
        "PLAY/PAUSE" => Transport::PlayPause,
        "CUE" => Transport::Cue,
        "PITCH BEND -" => Transport::PitchBend(-1),
        "PITCH BEND +" => Transport::PitchBend(1),
        "BPM - SMALL" => Transport::Bpm(-1, 10),
        "BPM - MEDIUM" => Transport::Bpm(-1, 50),
        "BPM - LARGE" => Transport::Bpm(-1, 200),
        "BPM + SMALL" => Transport::Bpm(1, 10),
        "BPM + MEDIUM" => Transport::Bpm(1, 50),
        "BPM + LARGE" => Transport::Bpm(1, 200),
        "CUE/LOOP CALL <" => Transport::CueLoopCall(-1),
        "CUE/LOOP CALL >" => Transport::CueLoopCall(1),
        "TEMPO RESET" => Transport::TempoReset,
        "SLIP" => Transport::Slip,
        "PREVIOUS TRACK" => Transport::Track(-1),
        "NEXT TRACK" => Transport::Track(1),
        "ACTIVE LOOP" => Transport::ActiveLoop,
        _ => return None,
    })
}

fn parse_fraction(s: &str) -> Option<f64> {
    match s.split_once('/') {
        Some((n, d)) => Some(n.trim().parse::<f64>().ok()? / d.trim().parse::<f64>().ok()?),
        None => s.trim().parse::<f64>().ok(),
    }
}

pub fn user_act(item: &PadItem) -> UserAct {
    let v = item.value.trim();
    match item.function.as_str() {
        "HotCue" => match v.chars().next() {
            Some(c) if c.is_ascii_uppercase() && (c as u8) < b'A' + 16 => UserAct::HotCue((c as u8 - b'A') as i32),
            _ => UserAct::Other,
        },
        "UserBeatJump" | "BeatJump" => {
            let forward = !v.starts_with('-');
            let body = v.trim_start_matches(['-', '+']);
            if body.eq_ignore_ascii_case("FINE") {
                UserAct::BeatJump(None, forward)
            } else {
                parse_fraction(body).map(|b| UserAct::BeatJump(Some(b), forward)).unwrap_or(UserAct::Other)
            }
        }
        "BeatLoop" => parse_fraction(v).map(UserAct::BeatLoop).unwrap_or(UserAct::Other),
        "UserKeyShift" | "KeyShift" | "Keyboard" => match v {
            "KEY SYNC" | "KeySync" => UserAct::Key(KEY_SYNC),
            "KEY RESET" | "KeyReset" => UserAct::Key(KEY_RESET),
            "SEMITONE UP" | "SemitoneUp" => UserAct::Key(KEY_SEMI_UP),
            "SEMITONE DOWN" | "SemitoneDown" => UserAct::Key(KEY_SEMI_DOWN),
            _ => v.trim_start_matches('+').parse::<i32>().ok().filter(|n| (-12..=12).contains(n)).map(UserAct::Key).unwrap_or(UserAct::Other),
        },
        "Sampler" => v.parse::<usize>().ok().filter(|n| (1..=16).contains(n)).map(|n| UserAct::Sampler(n - 1)).unwrap_or(UserAct::Other),
        "SequenceCall" => v.parse::<usize>().ok().filter(|n| (1..=8).contains(n)).map(|n| UserAct::SeqCall(n - 1)).unwrap_or(UserAct::Other),
        "PadFx1" | "PadFx2" | "PadFxCombination" => {
            let bank = match item.function.as_str() {
                "PadFx1" => 0,
                "PadFx2" => 1,
                _ => 2,
            };
            v.parse::<u8>().ok().filter(|n| (1..=16).contains(n)).map(|n| UserAct::PadFx(bank, n - 1)).unwrap_or(UserAct::Other)
        }
        "Transport" => transport_of(v).map(UserAct::Transport).unwrap_or(UserAct::Other),
        "SampleScratch" => v.parse::<usize>().ok().filter(|n| (1..=16).contains(n)).map(|n| UserAct::SampleScratch(n - 1)).unwrap_or(UserAct::Other),
        _ => UserAct::Other,
    }
}

/// A USER pad's caption.
pub fn user_label(item: &PadItem) -> String {
    match user_act(item) {
        UserAct::HotCue(s) => format!("HOT CUE {}", (b'A' + s as u8) as char),
        UserAct::BeatJump(b, f) => format!("{} {}", if f { "+" } else { "-" }, b.map(crate::player_ext::beats_text).unwrap_or_else(|| "FINE".into())),
        UserAct::BeatLoop(b) => format!("LOOP {}", crate::player_ext::beats_text(b)),
        UserAct::Key(k) => match k {
            KEY_SYNC => "KEY SYNC".into(),
            KEY_RESET => "KEY RESET".into(),
            KEY_SEMI_UP => "SEMITONE UP".into(),
            KEY_SEMI_DOWN => "SEMITONE DOWN".into(),
            n if n > 0 => format!("KEY +{n}"),
            n => format!("KEY {n}"),
        },
        UserAct::Sampler(n) => format!("SAMPLER {}", n + 1),
        UserAct::SeqCall(n) => format!("PATTERN {}", n + 1),
        // (`PadMenu`'s texts: "PAD FX1: %d", "PAD FX2: %d"; the transport value as it is.)
        UserAct::PadFx(0, n) => format!("PAD FX1: {}", n + 1),
        UserAct::PadFx(1, n) => format!("PAD FX2: {}", n + 1),
        UserAct::PadFx(_, n) => format!("PAD FX {}", n + 1),
        UserAct::Transport(_) => item.value.trim().to_string(),
        UserAct::SampleScratch(n) => format!("SP.SCR {}", n + 1),
        UserAct::Other => match item.function.as_str() {
            "PadFxCombination" => format!("PAD FX {}", item.value),
            "PadFx1" => format!("PAD FX1: {}", item.value),
            "PadFx2" => format!("PAD FX2: {}", item.value),
            f => format!("{f} {}", item.value).trim().to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_mode_values_and_bytes_round_trip() {
        // rekordbox's values 0..20.
        for v in 0..=20u8 {
            let m = PadMode::from_value(v).unwrap();
            assert_eq!(m.value(), v);
            assert_eq!(PadMode::from_code(m.code()), m, "{m:?}");
        }
        assert!(PadMode::from_value(21).is_none());
        // The bytes other code already uses keep their meaning.
        assert_eq!(PadMode::HotCue.code(), 0);
        assert_eq!(PadMode::Stems.code(), 1);
        assert_eq!(PadMode::BeatJump.code(), 2);
        assert_eq!(PadMode::BeatLoop.code(), 3);
        assert_eq!(PadMode::Sampler.code(), crate::ui_perf::PAD_SAMPLER);
        assert_eq!(PadMode::SeqCall.code(), crate::ui_perf::PAD_SEQ_CALL);
        // The menu: eleven entries in rekordbox's order.
        let labels: Vec<String> = MENU.iter().map(|m| m.label()).collect();
        assert_eq!(labels, ["HOT CUE", "PAD FX", "SLICER", "BEAT JUMP", "BEAT LOOP", "KEYBOARD", "KEY SHIFT", "SEQ. CALL", "ACT. CENSR", "STEMS", "MEMORY CUE"]);
        assert_eq!(PadMode::User(2).label(), "USER 3");
    }

    #[test]
    fn scale1_keys_transport_and_pad_fx_items() {
        let keys: Vec<i32> = (0..40).map(key_at_scale1).collect();
        assert_eq!(&keys[4..12], &[-12, -11, -10, -9, -8, -7, -6, -5]);
        assert_eq!(keys[20], 4);
        assert_eq!(&keys[24..29], &[8, 9, 10, 11, 12]);
        assert_eq!(&keys[32..40], &[KEY_SYNC, KEY_RESET, KEY_SEMI_DOWN, KEY_SEMI_UP, 1007, 1006, 1004, 1005]);
        assert_eq!(key_semis(keys[38]), Some(7));
        assert_eq!(key_list_pos_scale1(16), Some(12));
        assert_eq!(key_list_pos_scale1(39), None);
        assert!(key_scale1_device("DDJ-GRV6") && !key_scale1_device("DDJ-FLX4"));
        let item = |f: &str, v: &str| parse_pad_line(&format!("function={f},value={v}")).unwrap();
        assert_eq!(user_act(&item("Transport", "BPM - MEDIUM")), UserAct::Transport(Transport::Bpm(-1, 50)));
        assert_eq!(user_act(&item("Transport", "NEXT TRACK")), UserAct::Transport(Transport::Track(1)));
        assert_eq!(user_act(&item("PadFx2", "3")), UserAct::PadFx(1, 2));
        assert_eq!(user_act(&item("PadFxCombination", "16")), UserAct::PadFx(2, 15));
        assert_eq!(user_label(&item("PadFx1", "4")), "PAD FX1: 4");
        assert_eq!(user_act(&item("SampleScratch", "2")), UserAct::SampleScratch(1));
    }

    #[test]
    fn can_select_rules() {
        let ok = SelectCtx { loaded: true, has_grid: true, pos: 3.0, ..Default::default() };
        for m in MENU {
            assert!(can_select(m, ok).is_ok(), "{m:?}");
        }
        // SLICER on a loaded track needs a grid and a position at or after the start.
        assert_eq!(can_select(PadMode::Slicer, SelectCtx { has_grid: false, ..ok }), Err(Some(MSG_SLICER_GRID)));
        assert_eq!(can_select(PadMode::Slicer, SelectCtx { pos: -0.5, ..ok }), Err(Some(MSG_BEFORE_START)));
        assert!(can_select(PadMode::Slicer, SelectCtx::default()).is_ok());
        // An external input: only SEQ. CALL (and SAMPLER), PAD FX with PC MASTER OUT.
        let ext = SelectCtx { external: true, ..ok };
        assert!(can_select(PadMode::HotCue, ext).is_err());
        assert!(can_select(PadMode::SeqCall, ext).is_ok());
        assert!(can_select(PadMode::Sampler, ext).is_ok());
        assert_eq!(can_select(PadMode::PadFx, ext), Err(Some(MSG_EXTERNAL)));
        assert!(can_select(PadMode::PadFx, SelectCtx { pc_mix: true, ..ext }).is_ok());
        assert_eq!(menu_for(ext), vec![PadMode::SeqCall]);
        assert_eq!(menu_for(SelectCtx { pc_mix: true, ..ext }), vec![PadMode::PadFx, PadMode::SeqCall]);
        assert_eq!(menu_for(ok).len(), 11);
        // Layout: Auto is the list without a controller.
        assert!(!pad_layout(1, 0));
        assert!(pad_layout(1, 8));
        assert!(!pad_layout(2, 8));
        assert!(pad_layout(3, 0));
    }

    #[test]
    fn beat_jump_amounts_and_pages() {
        // Ids 0/1 are FINE; 2..7 are 1/8, 1/4, 1/2; 8..23 are 1..128, back then forward.
        assert_eq!(jump_beats(0), None);
        assert_eq!(jump_beats(1), None);
        let beats: Vec<f64> = (2..24).step_by(2).map(|i| jump_beats(i).unwrap()).collect();
        assert_eq!(beats, [0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0]);
        for id in 0..24u8 {
            assert_eq!(jump_beats(id), jump_beats(id ^ 1));
            assert_eq!(jump_forward(id), id % 2 == 1);
            // The cell's label is the amount.
            if let Some(b) = jump_beats(id) {
                assert_eq!(crate::player_ext::beats_text(b), JUMP_LABELS[(id / 2) as usize]);
            }
        }
        // FINE is 220 samples at 44.1 kHz: 5 ms.
        assert!((JUMP_FINE_SECS - 0.004989).abs() < 1e-5);
        // Pages 1..9: a controller's first pad is logical id clamp(2 * page - 2, 0, 16).
        let firsts: Vec<u8> = (1..=9).map(jump_page_first).collect();
        assert_eq!(firsts, [0, 2, 4, 6, 8, 10, 12, 14, 16]);
        // Page 5 (the default): 1, 2, 4, 8 beats, as BeatJump.pad.csv.
        assert_eq!(jump_page_cells(5).map(|c| JUMP_LABELS[c]), ["1", "2", "4", "8"]);
        assert_eq!(jump_page_cells(1).map(|c| JUMP_LABELS[c]), ["FINE", "1/8", "1/4", "1/2"]);
        assert_eq!(jump_page_cells(9).map(|c| JUMP_LABELS[c]), ["16", "32", "64", "128"]);
        // Page buttons, and with Shift.
        assert_eq!(jump_page_step(5, true, false), 6);
        assert_eq!(jump_page_step(9, true, false), 9);
        assert_eq!(jump_page_step(1, false, false), 1);
        assert_eq!(jump_page_step(3, true, true), 5);
        assert_eq!(jump_page_step(5, true, true), 9);
        assert_eq!(jump_page_step(7, false, true), 5);
        assert_eq!(jump_page_step(5, false, true), 1);
        assert_eq!(PadsPrefs::default().beat_jump_page, [5; 4]);
    }

    #[test]
    fn beat_loop_pads() {
        assert_eq!(LOOP_LABELS.len(), crate::player_ext::LOOP_SIZES.len());
        for (i, b) in crate::player_ext::LOOP_SIZES.iter().enumerate() {
            assert_eq!(crate::player_ext::beats_text(*b), LOOP_LABELS[i]);
            assert_eq!(loop_pad_for(*b), Some(i));
        }
        assert_eq!(loop_pad_for(3.0), None);
    }

    #[test]
    fn key_table_and_limits() {
        // The five pages of eight (scale 0).
        let page = |p: usize| -> Vec<String> { (0..8).map(|i| key_label(p * 8 + i, true)).collect() };
        assert_eq!(page(0), ["+12", "", "", "", "+8", "+9", "+10", "+11"]);
        assert_eq!(page(1), ["+4", "+5", "+6", "+7", "0", "+1", "+2", "+3"]);
        assert_eq!(page(2), ["-4", "-3", "-2", "-1", "-8", "-7", "-6", "-5"]);
        assert_eq!(page(3), ["-12", "-11", "-10", "-9", "", "", "", ""]);
        assert_eq!(page(4), ["KEY SYNC", "SEMITONE UP", "+7", "+12", "KEY RESET", "SEMITONE DOWN", "-5", "-12"]);
        assert_eq!(key_label(32, false), "KEY\nSYNC");
        // Pages 2 and 3 are Keyboard.pad.csv / KeyShift.pad.csv: pads 1..8 and the shift pads.
        let pads: Vec<i32> = (0..16).map(|p| key_at(key_pad_pos(2, p))).collect();
        assert_eq!(pads, [4, 5, 6, 7, 0, 1, 2, 3, -4, -3, -2, -1, -8, -7, -6, -5]);
        // The list: 32 entries from +12 down.
        let list: Vec<String> = key_list().iter().map(|p| key_label(*p, true)).collect();
        assert_eq!(list.len(), 32);
        assert_eq!(&list[..5], ["+12", "+11", "+10", "+9", "+8"]);
        assert_eq!(list[12], "0");
        assert_eq!(&list[23..], ["-11", "-12", "-5", "+12", "+7", "SEMITONE UP", "SEMITONE DOWN", "KEY SYNC", "KEY RESET"]);
        // Limits.
        assert_eq!(key_press(key_at(8), 0, false), KeyAct::Set(4));
        assert_eq!(key_press(KEY_SEMI_UP, 11, false), KeyAct::Set(12));
        assert_eq!(key_press(KEY_SEMI_UP, 12, false), KeyAct::Refuse(None));
        assert_eq!(key_press(KEY_SEMI_DOWN, -11, false), KeyAct::Set(-12));
        assert_eq!(key_press(KEY_SEMI_DOWN, -12, false), KeyAct::Refuse(None));
        assert_eq!(key_press(KEY_RESET, 5, true), KeyAct::Set(0));
        assert_eq!(key_press(KEY_RESET, 5, false), KeyAct::Refuse(Some(MSG_KEY_RESET)));
        assert_eq!(key_press(KEY_SYNC, 5, false), KeyAct::Sync);
        assert_eq!(key_press(1006, 3, false), KeyAct::Set(-5));
        assert_eq!(key_press(KEY_NONE, 3, false), KeyAct::Nothing);
        for p in 0..40 {
            if let KeyAct::Set(n) = key_press(key_at(p), 0, true) {
                assert!((-12..=12).contains(&n));
            }
        }
    }

    #[test]
    fn hot_cue_colour_sets() {
        assert_eq!(hot_cue_set_color(4, 0, false), 43);
        assert_eq!(hot_cue_set_color(4, 9, false), 8);
        assert_eq!(hot_cue_set_color(1, 3, false), 22);
        assert_eq!(hot_cue_set_color(1, 8, false), 26);
        assert_eq!(hot_cue_set_color(2, 2, false), 58);
        assert_eq!(hot_cue_set_color(3, 7, false), 59);
        assert_eq!(hot_cue_set_color(3, 7, true), crate::model::HOTCUE_LOOP_DEFAULT);
        // COLORFUL is what recordbox used before the setting existed.
        for s in 0..16 {
            assert_eq!(hot_cue_set_color(4, s, false), crate::model::hotcue_default_color(s, false));
        }
    }

    fn grid(n: usize, bpm: f64) -> Vec<f64> {
        (0..n).map(|i| 0.1 + i as f64 * 60.0 / bpm).collect()
    }

    #[test]
    fn slicer_sections_and_roll() {
        // LENGTH 2 / 4 / 8 / 16 beats over eight sections.
        let units: Vec<f64> = (0..4).map(slicer_unit_beats).collect();
        assert_eq!(units, [0.25, 0.5, 1.0, 2.0]);
        // LENGTH sets ROLL; a chosen ROLL stays inside its window of four.
        let rolls: Vec<String> = (0..4).map(|l| slicer_roll_label(slicer_roll_for_length(l))).collect();
        assert_eq!(rolls, ["1/8", "1/4", "1/2", "1/1"]);
        assert_eq!(slicer_clamp_roll(2, 0), 2);
        assert_eq!(slicer_clamp_roll(2, 7), 5);
        assert_eq!(slicer_clamp_roll(0, 7), 3);
        assert_eq!(slicer_clamp_roll(3, 4), 4);
        assert_eq!(slicer_roll_beats(4), 0.5);
        // The domain: nine boundaries on the grid.
        let g = grid(64, 120.0);
        let (start, b) = slicer_domain(&g, 16.0, 1.0, 40.0);
        assert_eq!(start, 16.0);
        for k in 0..9 {
            assert!((b[k] - g[16 + k]).abs() < 1e-9);
        }
        // Half-beat sections.
        let (_, b) = slicer_domain(&g, 16.0, 0.5, 40.0);
        assert!((b[1] - (g[16] + 0.25)).abs() < 1e-9);
        assert!((b[8] - g[20]).abs() < 1e-9);
        // The start is pulled back so that the domain fits the grid.
        let (start, b) = slicer_domain(&g, 60.0, 1.0, 40.0);
        assert_eq!(start, 55.0);
        assert!((b[8] - g[63]).abs() < 1e-9);
        // Sections.
        let (_, b) = slicer_domain(&g, 16.0, 1.0, 40.0);
        assert_eq!(slicer_section(&b, g[16]), Some(0));
        assert_eq!(slicer_section(&b, g[19] + 0.2), Some(3));
        assert_eq!(slicer_section(&b, g[24] - 0.001), Some(7));
        assert_eq!(slicer_section(&b, g[24]), None);
        assert_eq!(slicer_section(&b, g[15]), None);
        // SLICER: the domain moves on in whole domains from the anchor.
        assert_eq!(slicer_advance(16.0, 24.2, 8.0), 24.0);
        assert_eq!(slicer_advance(16.0, 41.0, 8.0), 40.0);
        assert_eq!(slicer_advance(16.0, 9.0, 8.0), 8.0);
        assert_eq!(slicer_advance(3.0, 12.5, 4.0), 11.0);
        // Shift buttons, kept inside [0, beats - 2].
        assert_eq!(slicer_shift(16.0, true, 8.0, 64), 24.0);
        assert_eq!(slicer_shift(16.0, false, 8.0, 64), 8.0);
        assert_eq!(slicer_shift(4.0, false, 8.0, 64), 0.0);
        assert_eq!(slicer_shift(60.0, true, 8.0, 64), 62.0);
        // Beat positions and times are each other's inverse.
        for t in [0.1, 0.35, 7.77, 20.0] {
            assert!((beat_time(&g, time_beat(&g, t)) - t).abs() < 1e-9);
        }
    }

    #[test]
    fn slicer_held_pads() {
        let mut st = Vec::new();
        assert_eq!(slicer_stack(&mut st, 2, true), Some(Some(2)));
        assert_eq!(slicer_stack(&mut st, 5, true), Some(Some(5)));
        // Letting go of the older pad changes nothing.
        assert_eq!(slicer_stack(&mut st, 2, false), None);
        assert_eq!(slicer_stack(&mut st, 2, true), Some(Some(2)));
        // Letting go of the newest goes back to the one held before.
        assert_eq!(slicer_stack(&mut st, 2, false), Some(Some(5)));
        assert_eq!(slicer_stack(&mut st, 5, false), Some(None));
        assert!(st.is_empty());
    }

    #[test]
    fn censor_regions() {
        let r = |a: f64, b: f64| CensorRegion { id: 0, a, b, beats: CENSOR_DEFAULT_BEATS };
        let regions = vec![r(10.0, 12.0), r(20.0, 21.0)];
        assert!(censor_can_add(&regions, 13.0, 14.0));
        assert!(censor_can_add(&regions, 12.0, 20.0));
        // Out not after in, too short, overlapping.
        assert!(!censor_can_add(&regions, 14.0, 14.0));
        assert!(!censor_can_add(&regions, 14.0, 13.0));
        assert!(!censor_can_add(&regions, 14.0, 14.001));
        assert!(!censor_can_add(&regions, 11.0, 13.0));
        assert!(!censor_can_add(&regions, 9.0, 30.0));
        assert!(!censor_can_add(&regions, 20.5, 20.8));
        assert_eq!(censor_at(&regions, 10.0), Some(0));
        assert_eq!(censor_at(&regions, 12.0), None);
        assert_eq!(censor_at(&regions, 20.99), Some(1));
        // Adjusting region 0 (10..11): its out may not reach region 1, nor shrink under
        // 6.667 ms; its in may not go before 0.
        assert!(censor_edge_ok(&regions, 0, 10.0, 19.9));
        assert!(!censor_edge_ok(&regions, 0, 10.0, 20.5));
        assert!(!censor_edge_ok(&regions, 0, 10.0, 10.005));
        assert!(censor_edge_ok(&regions, 0, 10.0, 10.007));
        assert!(!censor_edge_ok(&regions, 0, -0.1, 11.0));
        assert!(!censor_can_add(&regions, 30.0, 30.006));
        assert_eq!(censor_neighbour(&regions, 11.0, true), Some(20.0));
        assert_eq!(censor_neighbour(&regions, 11.0, false), Some(10.0));
        assert_eq!(censor_neighbour(&regions, 25.0, true), None);
        assert_eq!(censor_neighbour(&regions, 5.0, false), None);
        // Defaults and the beat buttons.
        assert_eq!(regions[0].beat(0), 32.0);
        assert_eq!(regions[0].beat(1), 0.25);
        assert_eq!(regions[0].beat(2), 0.125);
        assert_eq!(regions[0].beat(3), 0.75);
        assert_eq!(censor_beat_step(1, (1, 4), false), (1, 8));
        assert_eq!(censor_beat_step(1, (1, 8), true), (1, 4));
        assert_eq!(censor_beat_step(3, (3, 4), false), (1, 2));
        assert_eq!(censor_beat_step(3, (3, 4), true), (1, 1));
        assert_eq!(censor_beat_step(3, (1, 16), false), (1, 16));
        assert_eq!(censor_beat_step(3, (32, 1), true), (32, 1));
        assert_eq!(censor_beat_step(0, (32, 1), true), (32, 1));
        assert_eq!(fraction_text((3, 4)), "3/4");
        assert_eq!(fraction_text((32, 1)), "32");
    }

    #[test]
    fn pad_csv_items() {
        let it = parse_pad_line("function=HotCue,value=C").unwrap();
        assert_eq!(user_act(&it), UserAct::HotCue(2));
        // Inside a USER mode the user function and value take over.
        let it = parse_pad_line("function=BeatJump,value=3,userfunction=UserBeatJump,uservalue=-1/2").unwrap();
        assert_eq!(user_act(&it), UserAct::BeatJump(Some(0.5), false));
        assert_eq!(user_label(&it), "- 1/2");
        assert_eq!(user_act(&parse_pad_line("function=UserBeatJump,value=+16").unwrap()), UserAct::BeatJump(Some(16.0), true));
        assert_eq!(user_act(&parse_pad_line("function=UserBeatJump,value=+FINE").unwrap()), UserAct::BeatJump(None, true));
        assert_eq!(user_act(&parse_pad_line("function=BeatLoop,value=1/4").unwrap()), UserAct::BeatLoop(0.25));
        assert_eq!(user_act(&parse_pad_line("function=UserKeyShift,value=KEY SYNC").unwrap()), UserAct::Key(KEY_SYNC));
        assert_eq!(user_act(&parse_pad_line("function=UserKeyShift,value=-3").unwrap()), UserAct::Key(-3));
        assert_eq!(user_act(&parse_pad_line("function=Sampler,value=4").unwrap()), UserAct::Sampler(3));
        assert_eq!(user_act(&parse_pad_line("function=PadFxCombination,value=2").unwrap()), UserAct::PadFx(2, 1));
        assert!(parse_pad_line("# a comment").is_none());
        assert!(parse_pad_line("").is_none());
        assert_eq!(parse_pad_line("function=HotCue,value=A,key=-2").unwrap().key, Some(-2));
        // rekordbox's own templates, when it is installed: USER 1 is hot cues A-D and sampler
        // slots 1-4, with beat loops and PAD FX on the shift page.
        let t = user_template(0);
        if !t.is_empty() {
            assert_eq!(t.len(), 16);
            assert_eq!(user_act(&t[0]), UserAct::HotCue(0));
            assert_eq!(user_act(&t[4]), UserAct::Sampler(0));
            assert_eq!(user_act(&t[8]), UserAct::BeatLoop(1.0));
        }
        // Every function the editor offers gives items that act.
        for (f, _, values) in pad_functions() {
            for v in values {
                let it = PadItem { function: f.to_string(), value: v.clone(), key: None };
                assert_ne!(user_act(&it), UserAct::Other, "{f} {v}");
            }
        }
    }
}
