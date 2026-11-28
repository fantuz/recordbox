//! rekordbox's keyboard commands at work: every command of `keycmd_data.rs` that has a feature
//! in recordbox does what rekordbox's does, for the deck, mixer channel or FX unit its row
//! stands for, in the modes rekordbox lists it in.
//!
//! The key table is built from the mapping chosen in Preferences > Keyboard (a preset, a user
//! mapping or recordbox's own keys, `ui_prefs_keys.rs`). A key press runs its command, a key
//! release ends what is held (CUE, a hot cue played from a stopped deck, SLIP REVERSE, pitch
//! bend, a pad, a sampler slot, RELEASE FX). Commands that exist as MIDI functions go through
//! `midi::apply` with the registry's function, so a key and a controller button share one
//! implementation; the others call the same `App` methods the on-screen controls call.
//!
//! With recordbox's own mapping the commands that have a recordbox action keep being run by
//! the key loop in main.rs (it acts on the deck in focus); every other row is run here.

use crate::app::*;
use crate::keycmd_data::CMDS;
use crate::model::*;
use crate::prefs_rb::Mode;
use crate::ui_prefs_keys::{self as keys, Row};
use eframe::egui;
use std::sync::Arc;

/// What a command does. `unit` (the deck, channel or FX unit of the row) is passed beside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ka {
    // ---- a deck (Player A / B in EXPORT, Deck 1..4 in PERFORMANCE)
    Play,
    Cue,
    JumpDefault(i8),
    LoopIn,
    LoopOut,
    Reloop,
    BeatLoop(f64),
    LoopResize(f64),
    Track(i8),
    Quantize,
    TimeMode,
    Hot(u8),
    HotClear(u8),
    MemSet,
    MemCall(u8),
    MemDelete(u8),
    MemStep(i8),
    MemDeleteCurrent,
    Panel(u8),
    GridShift(i8, bool),
    GridCenter,
    GridInterval(i8, bool),
    GridWhole(bool),
    Tap,
    Sync,
    Master,
    MasterTempo,
    TempoReset,
    Bend(i8),
    AnalysisLock,
    TrackStart,
    TempoStep(i8, u8),
    KeyShift(i8),
    KeyReset,
    KeySync,
    BeatJump(f64),
    AutoBeatLoop,
    Metronome,
    MetronomeVolume,
    GridHalfDouble(bool),
    SetDownbeat,
    GridUndo(bool),
    MemLoopActive(u8),
    HotLoopActive(u8),
    StemPart(u8),
    PadMode(crate::pads::PadMode),
    Pad(u8),
    PadParam(u8, i8),
    /// A Performance-only command with a MIDI function of this name.
    Midi(&'static str),
    DrumBank(u8),
    DrumSlot(u8),
    // ---- all decks and the browser
    Zoom(i8),
    Volume(i8),
    /// A menu bar command (`menus::handle`).
    Menu(&'static str),
    /// A recordbox keyboard action of `prefs_ext::ACTIONS` (sampler, recordings).
    Action(&'static str),
    LoadDeck(u8),
    LoadSide(bool),
    Doubles(u8),
    DoublesSide(bool),
    Layout(u8),
    PerfLayout(u8),
    SearchList,
    SearchCollection,
    SearchTree,
    PreviewPlay,
    PreviewSkip,
    ShortcutsToggle,
    ShortcutOpen(u8),
    ShortcutAdd(u8),
    TagListView,
    TagListAdd,
    PaletteBank(u8),
    PaletteOpen(u8),
    PaletteAdd(u8),
    ShowListOfTrack,
    Related,
    UndoLoad,
    Bold,
    LocateLoaded,
    ColumnView(bool),
    AnalysisLockSel(bool),
    Quit,
    // ---- mixer (unit = channel)
    Crossfader(i8, u8),
    Trim(i8, bool),
    HeadCue,
    Fader(i8, u8),
    AutoGain(i8, bool),
    // ---- effects (unit = FX1 / FX2)
    FxAssign(u8),
    FxMode,
    FxSelect(u8),
    FxOn(u8),
    FxParam(u8, i8),
    FxReleaseSelect,
    FxReleaseOn,
    FxReleaseBeat(i8),
    FxSlotBeat(u8, i8),
    FxSingleBeat(i8),
    Cfx(u8),
    StemFx(u8),
    // ---- MIX POINT LINK and Automix
    Mpl(u8),
}

impl Ka {
    /// Held keys repeat the command (steps of a value).
    pub fn repeats(self) -> bool {
        matches!(self, Ka::TempoStep(..) | Ka::Crossfader(..) | Ka::Trim(..) | Ka::Fader(..) | Ka::AutoGain(..) | Ka::Volume(_) | Ka::Zoom(_) | Ka::GridShift(..) | Ka::GridInterval(..) | Ka::FxParam(..))
    }
    /// The command acts while its key is held, and its release matters.
    pub fn gate(self) -> bool {
        matches!(self, Ka::Cue | Ka::Hot(_) | Ka::Bend(_) | Ka::Pad(_) | Ka::FxReleaseOn) || matches!(self, Ka::Midi("SlipReverse") | Ka::Midi("PitchBendUp") | Ka::Midi("PitchBendDown")) || matches!(self, Ka::Action(a) if a.starts_with("smp_play"))
    }
}

/// Why a command does nothing in `mode` (for the Keyboard page and the report).
pub fn missing_reason_in(id: u16, mode: Mode) -> &'static str {
    match action(id) {
        Some(a) if mode != Mode::Performance && performance_only(a) => "it acts in PERFORMANCE mode, which has the mixer, effects, sampler, pads, recordings and MIX POINT LINK (rekordbox's EXPORT mode has none of them on screen).",
        _ if matches!(id, 0x2013..=0x2017 | 0xcc1b) && mode != Mode::Edit => "an EDIT mode command: it acts in EDIT.",
        _ => missing_reason(id),
    }
}

/// Why a command does nothing in recordbox (for the Keyboard page and the report).
pub fn missing_reason(id: u16) -> &'static str {
    match id {
        0x6000..=0x6006 => "PRO DJ LINK players (Link Export) are hardware recordbox does not drive.",
        0xcb00..=0xcb03 => "recordbox has no Ableton Link.",
        0x30b0..=0x30b6 | 0x30e2 => "recordbox has no DVS.",
        0xc800 => "Video is outside recordbox's scope.",
        0x2019 | 0x201a => "Lighting is outside recordbox's scope.",
        0x4015 | 0x200e | 0x201c | 0xb13a | 0xb13b | 0xb13d | 0xb13e => "recordbox has no streaming or cloud services.",
        0xa003 | 0xa007 | 0xa009 | 0xa100 | 0xa008 | 0xa002 | 0xa001 => "rekordbox's own online and registration pages.",
        0x2010 | 0x2018 => "greyed in recordbox's menu (no rekordbox 5 library, no device library backups).",
        0x3005 => "rekordbox's own name says it: \"Currently not working\".",
        0xb11e | 0xb11f => "recordbox's Playlist Palette has one bank.",
        0x30e3 => "SP.SCRATCH is not built.",
        0x3053 => "the BPM field's command was not traced (what it focuses or switches is unknown).",
        _ => "not built in recordbox yet.",
    }
}

/// The base command id (the per-unit offset taken away) and the unit.
pub fn split_id(id: u16) -> (u16, u8) {
    let base_scoped = |b: u16| CMDS.iter().find(|c| c.id == b).map(|c| c.scope != 0).unwrap_or(false);
    let u = ((id >> 8) & 0x3) as u8;
    let base = id - ((u as u16) << 8);
    if u > 0 && base_scoped(base) {
        (base, u)
    } else {
        (id, 0)
    }
}

/// What rekordbox's command `base` does in recordbox (None: nothing yet; see `missing_reason`).
pub fn action(base: u16) -> Option<Ka> {
    use crate::pads::PadMode as P;
    use Ka::*;
    let hot = |i: u16| -> u8 {
        // Hot cues A, B, C, D, then E..P.
        match i {
            0x301e => 0,
            0x301f => 1,
            0x3020 => 2,
            0x3054 => 3,
            _ => (i - 0x307f + 4) as u8,
        }
    };
    let clr = |i: u16| -> u8 {
        match i {
            0x3021 => 0,
            0x3022 => 1,
            0x3023 => 2,
            0x3055 => 3,
            _ => (i - 0x308b + 4) as u8,
        }
    };
    const LOOPS: [(u16, f64); 16] = [(0x30bf, 1.0 / 64.0), (0x300d, 1.0 / 32.0), (0x300e, 1.0 / 16.0), (0x300f, 0.125), (0x3010, 0.25), (0x3011, 0.5), (0x3012, 1.0), (0x3013, 2.0), (0x3014, 4.0), (0x3015, 8.0), (0x3016, 16.0), (0x3017, 32.0), (0x30c0, 64.0), (0x30c1, 128.0), (0x30c2, 256.0), (0x30c3, 512.0)];
    if let Some(l) = LOOPS.iter().find(|l| l.0 == base) {
        return Some(BeatLoop(l.1));
    }
    const JUMPS: [f64; 7] = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
    Some(match base {
        // ---- File
        0x2000 => Menu("import_track"),
        0x2001 => Menu("import_folder"),
        0x2002 => Menu("import_playlist"),
        0x2003 => Menu("missing"),
        0x2004 => Menu("backup"),
        0x2005 => Menu("restore"),
        0x2006 => Menu("export_xml"),
        0x200a => Menu("prefs"),
        0x200c => Menu("dupes"),
        0x200d | 0x201b => Menu("pack_sample_import"),
        0x1001 => Quit,
        // ---- General
        0x3003 => Volume(1),
        0x3004 => Volume(-1),
        0x7000 => SearchCollection,
        0x7001 => PreviewPlay,
        0x7002 => PreviewSkip,
        0x7003 => SearchList,
        0x7005 => SearchTree,
        // ---- Track / Playlist
        0x400d => Menu("show_finder"),
        0x4010 => AnalysisLockSel(true),
        0x4011 => AnalysisLockSel(false),
        0x5008 => Menu("export_device"),
        0x500e => Menu("renumber"),
        0x5505 => ColumnView(false),
        0x5506 => ColumnView(true),
        // ---- Help
        0xa000 => Menu("manual"),
        0xa004 => Menu("h_about"),
        // ---- View
        0xb040 => Layout(0),
        0xb041 => Layout(3),
        0xb042 => Layout(4),
        0xb043 => Layout(1),
        0xb058 => Layout(2),
        0xb044 => PerfLayout(0),
        0xb045 => PerfLayout(2),
        0xb046 => PerfLayout(1),
        0xb047 => PerfLayout(3),
        0xb048 | 0xb049 => PerfLayout(4),
        0xb04a | 0xb04b => Menu("pad_platter"),
        0xb04c => Menu("split_screen"),
        0xb04e => Menu("fullscreen"),
        0xb053 => Menu("appearance_dark"),
        0xb054 => Menu("appearance_light"),
        // ---- Browse
        0xb105 => Menu("filter"),
        0xb106 | 0xb107 => Menu("info_window"),
        0xb108 | 0xb109 => Menu("filter"),
        0xb10a => ShortcutsToggle,
        0xb10b..=0xb112 => ShortcutOpen((base - 0xb10b) as u8),
        0xb113..=0xb11a => ShortcutAdd((base - 0xb113) as u8),
        0xb11b => TagListView,
        0xb11c => TagListAdd,
        0xb11d => PaletteBank(0),
        0xb120..=0xb123 => PaletteOpen((base - 0xb120) as u8),
        0xb124..=0xb127 => PaletteAdd((base - 0xb124) as u8),
        0xb128 => ShowListOfTrack,
        0xb129..=0xb12c => LoadDeck((base - 0xb129) as u8),
        0xb12d => LoadSide(false),
        0xb12e => LoadSide(true),
        0xb12f..=0xb132 => Doubles((base - 0xb12f) as u8),
        0xb133 => DoublesSide(false),
        0xb134 => DoublesSide(true),
        0xb135 => Related,
        0xb137 => UndoLoad,
        0xb138 => Bold,
        0xb139 => LocateLoaded,
        // ---- All Decks
        0x3000 => Zoom(1),
        0x3001 => Zoom(-1),
        0x3002 => Zoom(0),
        // ---- a deck
        0x3006 => Play,
        0x3007 => Cue,
        0x3008 => JumpDefault(1),
        0x3009 => JumpDefault(-1),
        0x300a => LoopIn,
        0x300b => LoopOut,
        0x300c => Reloop,
        0x3018 => LoopResize(0.5),
        0x3019 => LoopResize(2.0),
        0x301a => Track(1),
        0x301b => Track(-1),
        0x301c => Quantize,
        0x301d => TimeMode,
        0x301e..=0x3020 | 0x3054 | 0x307f..=0x308a => Hot(hot(base)),
        0x3021..=0x3023 | 0x3055 | 0x308b..=0x3096 => HotClear(clr(base)),
        0x3024 => MemSet,
        0x3025..=0x302e => MemCall((base - 0x3025) as u8),
        0x302f..=0x3038 => MemDelete((base - 0x302f) as u8),
        0x3039 => MemStep(1),
        0x303a => MemStep(-1),
        0x303b => MemDeleteCurrent,
        0x303f => Panel(2),
        0x3043 => GridShift(1, false),
        0x3044 => GridShift(-1, false),
        0x3045 => GridCenter,
        0x3046 => GridInterval(1, false),
        0x3047 => GridInterval(-1, false),
        0x3048 => GridWhole(true),
        0x3049 => GridWhole(false),
        0x304a => Tap,
        0x304b => Sync,
        0x304c => Master,
        0x304d => MasterTempo,
        0x304e => TempoReset,
        0x304f => Bend(1),
        0x3050 => Bend(-1),
        0x3056 => AnalysisLock,
        0x3057 => TrackStart,
        0x3058 => Midi("Slip"),
        0x3059 => Midi("TempoRange"),
        0x305a..=0x305c => TempoStep(1, (base - 0x305a) as u8),
        0x305d..=0x305f => TempoStep(-1, (base - 0x305d) as u8),
        0x3060 => Midi("Reverse"),
        0x3061 => Midi("SlipReverse"),
        0x3062 => Midi("Vinyl"),
        0x3063 => PadMode(P::HotCue),
        0x3064 => Midi("PadFx1"),
        0x3065 => Midi("PadFx2"),
        0x3066 => PadMode(P::BeatJump),
        0x3067 => PadMode(P::Sampler),
        0x3069 => Midi("Slicer"),
        0x306a => Midi("SlicerLoop"),
        0x306b..=0x307a => Pad((base - 0x306b) as u8),
        0x307b => PadParam(1, -1),
        0x307c => PadParam(1, 1),
        0x307d => PadParam(2, -1),
        0x307e => PadParam(2, 1),
        0x3097..=0x309d => BeatJump(JUMPS[(base - 0x3097) as usize]),
        0x309e..=0x30a4 => BeatJump(-JUMPS[(base - 0x309e) as usize]),
        0x30a5 => AutoBeatLoop,
        0x30a6 => Metronome,
        0x30a7 => MetronomeVolume,
        0x30a8 => GridShift(1, true),
        0x30a9 => GridShift(-1, true),
        0x30aa => GridInterval(1, true),
        0x30ab => GridInterval(-1, true),
        0x30ac => GridHalfDouble(false),
        0x30ad => GridHalfDouble(true),
        0x30ae => Panel(0),
        0x30af => Panel(1),
        0x30b7 => PadMode(P::SeqCall),
        0x30b8 => Midi("AutoCue"),
        0x30b9 => KeyShift(1),
        0x30ba => KeyShift(-1),
        0x30bb => KeyReset,
        0x30bc => KeySync,
        0x30bd => PadMode(P::Keyboard),
        0x30be => PadMode(P::BeatLoop),
        0x30c4 => SetDownbeat,
        0x30c5 => GridUndo(true),
        0x30c6 => GridUndo(false),
        0x30c7 => PadMode(P::KeyShift),
        0x30c8..=0x30d1 => MemLoopActive((base - 0x30c8) as u8),
        0x30d2..=0x30e1 => HotLoopActive((base - 0x30d2) as u8),
        0x30e4 => Midi("SyncRate"),
        // (Sources in the engine's order: 0 drums, 1 bass, 2 other, 3 vocals.)
        0x30e5 => StemPart(2),
        0x30e6 => StemPart(3),
        0x30e7 => StemPart(0),
        0x30f2 => StemPart(1),
        0x30f3 => PadMode(P::Stems),
        0x30ec => DrumBank(0),
        0x30ed => DrumBank(1),
        0x30ee..=0x30f1 => DrumSlot((base - 0x30ee) as u8),
        // ---- mixer
        0xd000..=0xd002 => Crossfader(-1, (base - 0xd000) as u8),
        0xd003..=0xd005 => Crossfader(1, (base - 0xd003) as u8),
        0xd010 => Trim(1, false),
        0xd011 => Trim(1, true),
        0xd012 => Trim(-1, false),
        0xd013 => Trim(-1, true),
        0xd014 => HeadCue,
        0xd015..=0xd017 => Fader(1, (base - 0xd015) as u8),
        0xd018..=0xd01a => Fader(-1, (base - 0xd018) as u8),
        0xd01b => AutoGain(1, false),
        0xd01c => AutoGain(1, true),
        0xd01d => AutoGain(-1, false),
        0xd01e => AutoGain(-1, true),
        // ---- sampler, sequencer, recordings: recordbox's own actions of the same names
        // (rekordbox's "Quantize" of the sampler is recordbox's "Quantize (Sampler)".)
        0xf028 => Action("smp_quantize"),
        0xf000..=0xf04f | 0xd0f0..=0xd0f4 => return keys::action_of(base, CMDS.iter().find(|c| c.id == base)?.name).map(Action),
        // ---- effects
        0xe000..=0xe005 => FxAssign((base - 0xe000) as u8),
        0xe006 => FxMode,
        0xe007..=0xe009 => FxSelect((base - 0xe007) as u8),
        0xe00a..=0xe00c => FxOn((base - 0xe00a) as u8),
        0xe00d => FxParam(0, 1),
        0xe00e => FxParam(0, -1),
        0xe00f => FxParam(1, 1),
        0xe010 => FxParam(1, -1),
        0xe011 => FxParam(2, 1),
        0xe012 => FxParam(2, -1),
        0xe015 => FxReleaseSelect,
        0xe016 => FxReleaseOn,
        0xe017 => FxReleaseBeat(1),
        0xe018 => FxReleaseBeat(-1),
        0xe019..=0xe01c => Cfx((base - 0xe019) as u8),
        0xe01d => FxSlotBeat(0, 1),
        0xe01e => FxSlotBeat(0, -1),
        0xe01f => FxSlotBeat(1, 1),
        0xe020 => FxSlotBeat(1, -1),
        0xe021 => FxSlotBeat(2, 1),
        0xe022 => FxSlotBeat(2, -1),
        0xe023 => FxSingleBeat(1),
        0xe024 => FxSingleBeat(-1),
        // (The FX part select rows of the Active Stem: 0 VOCAL, 1 INST, 2 BASS, 3 DRUMS.)
        0xe025 => StemFx(1),
        0xe026 => StemFx(0),
        0xe027 => StemFx(3),
        0xe028 => StemFx(2),
        // ---- MIX POINT LINK
        0xcd00..=0xcd0d => Mpl((base - 0xcd00) as u8),
        _ => return None,
    })
}

/// Commands that only make sense in PERFORMANCE mode (the mixer, the effects, pads ...).
fn performance_only(a: Ka) -> bool {
    use Ka::*;
    matches!(a, Midi(_) | PadMode(_) | Pad(_) | PadParam(..) | DrumBank(_) | DrumSlot(_) | Crossfader(..) | Trim(..) | HeadCue | Fader(..) | AutoGain(..) | FxAssign(_) | FxMode | FxSelect(_) | FxOn(_) | FxParam(..) | FxReleaseSelect | FxReleaseOn | FxReleaseBeat(_) | FxSlotBeat(..) | FxSingleBeat(_) | Cfx(_) | StemFx(_) | Mpl(_) | PerfLayout(_) | StemPart(_) | BeatJump(_) | MemLoopActive(_) | HotLoopActive(_))
        || matches!(a, Action(x) if x.starts_with("smp_") || x.starts_with("seq_") || x.starts_with("rec_") || x.starts_with("padmode_"))
}

/// Whether the command `id` (with its unit offset) acts in recordbox in `mode`.
pub fn acts(id: u16, mode: Mode) -> bool {
    if mode == Mode::Edit {
        return edit_cmd(id).is_some();
    }
    let (base, _) = split_id(id);
    match action(base) {
        None => false,
        Some(a) => mode == Mode::Performance || !performance_only(a),
    }
}

// ------------------------------------------------------------------ the key table

/// One key of the table: the shortcut and the row it runs.
#[derive(Clone, Debug)]
struct Bind {
    sc: Shortcut,
    id: u16,
    ka: Ka,
    unit: u8,
}

#[derive(Default)]
struct State {
    /// The table and what it was built from (mode, mapping, the mapping's keys).
    sig: Option<(Mode, String, String)>,
    table: Vec<Bind>,
    /// Commands whose key is down: (key, command id, action, unit).
    held: Vec<(egui::Key, u16, Ka, u8)>,
    hub: Option<Arc<crate::midi::Hub>>,
    /// What ran, for the scripted run: (command id, pressed).
    pub log: Vec<(u16, bool)>,
}

static STATE: parking_lot::Mutex<Option<State>> = parking_lot::Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock();
    f(g.get_or_insert_with(State::default))
}

/// The keys of the mapping in use for `mode`, as (shortcut, row). With recordbox's own
/// mapping the rows run by the key loop of main.rs are left out.
fn build_table(prefs: &Prefs, mode: Mode) -> Vec<Bind> {
    let cur = keys::current(prefs, mode);
    let own = cur == keys::OWN;
    let mut v = vec![];
    for row in keys::commands(mode) {
        if own && row.unit == 0 && keys::action_of(row.id, &row.name).is_some() {
            continue;
        }
        let k = keys::key_of(prefs, mode, &cur, &row);
        if k.is_empty() {
            continue;
        }
        let Some(sc) = keys::juce_to_stored(&k).and_then(|s| Shortcut::parse(&s)) else { continue };
        let (base, unit) = split_id(row.id);
        let Some(ka) = action(base) else { continue };
        if mode != Mode::Performance && performance_only(ka) {
            continue;
        }
        v.push(Bind { sc, id: row.id, ka, unit: row_unit(&row).max(unit) });
    }
    v
}

fn row_unit(r: &Row) -> u8 {
    split_id(r.id).1
}

fn signature(prefs: &Prefs, mode: Mode) -> (Mode, String, String) {
    let cur = keys::current(prefs, mode);
    let m = prefs.rb.key_mappings.get(&cur).map(|m| format!("{m:?}")).unwrap_or_default();
    let own = if cur == keys::OWN { format!("{:?}", prefs.keymap) } else { String::new() };
    (mode, cur, m + &own)
}

/// Whether the key loop of main.rs still runs recordbox's action `action`: always with
/// recordbox's own mapping; with one of rekordbox's only for the actions rekordbox has no
/// command for (moving in the list, loading with Return, Delete, Change Focus, searching).
pub fn old_loop_runs(prefs: &Prefs, action: &str) -> bool {
    let mode = Mode::of(prefs.app_mode);
    if keys::current(prefs, mode) == keys::OWN {
        return true;
    }
    static WITH_ROWS: std::sync::OnceLock<std::collections::HashSet<&'static str>> = std::sync::OnceLock::new();
    let set = WITH_ROWS.get_or_init(|| keys::commands(Mode::Performance).iter().chain(keys::commands(Mode::Export).iter()).filter_map(|r| keys::action_of(r.id, &r.name)).collect());
    !set.contains(action)
}

/// F11 normally controls the native windows. An explicitly assigned command in the
/// active keyboard mapping gets first use, including deck-specific and EDIT rows.
pub(crate) fn custom_f11_binding(prefs: &Prefs) -> bool {
    let mode = Mode::of(prefs.app_mode);
    let mapping = keys::current(prefs, mode);
    if keys::is_preset(&mapping, mode) {
        return false;
    }
    if mode != Mode::Edit && prefs.keymap.iter().any(|(action, key)| {
        action != "fullscreen" && key == "F11" && old_loop_runs(prefs, action)
            && (mode == Mode::Performance || !crate::sampler_ext::is_sampler_action(action))
    }) {
        return true;
    }
    let Some(rows) = prefs.rb.key_mappings.get(&mapping) else { return false };
    if !rows.values().any(|key| key.trim().eq_ignore_ascii_case("F11")) {
        return false;
    }
    keys::commands(mode).iter().any(|row| {
        row.id != 0xb04e && acts(row.id, mode) && keys::key_of(prefs, mode, &mapping, row).trim().eq_ignore_ascii_case("F11")
    })
}

/// Once a frame (from main.rs, before recordbox's own key loop): the keys of the mapping in
/// use. Returns nothing; presses it ran are taken out of the frame's events.
pub fn frame(app: &mut App, ctx: &egui::Context) {
    let mode = Mode::of(app.prefs.app_mode);
    if mode == Mode::Edit {
        release_all(app);
        return;
    }
    let events: Vec<egui::Event> = ctx.input(|i| i.events.iter().filter(|e| matches!(e, egui::Event::Key { .. })).cloned().collect());
    let focused = ctx.input(|i| i.focused);
    let held_any = with_state(|s| !s.held.is_empty());
    if events.is_empty() && !held_any {
        return;
    }
    // Key-up events are not delivered once the window has lost the focus: let go of all.
    if !focused && app.screenshot.is_none() {
        release_all(app);
        return;
    }
    let sig = signature(&app.prefs, mode);
    let rebuild = with_state(|s| s.sig.as_ref() != Some(&sig));
    if rebuild {
        let t = build_table(&app.prefs, mode);
        with_state(|s| {
            s.table = t;
            s.sig = Some(sig);
        });
    }
    let typing = ctx.wants_keyboard_input() || prefs_waits_for_key(ctx);
    let mut ran: Vec<(egui::Key, egui::Modifiers)> = vec![];
    for e in events {
        let egui::Event::Key { key, pressed, repeat, modifiers, .. } = e else { continue };
        if !pressed {
            let rel: Vec<(egui::Key, u16, Ka, u8)> = with_state(|s| {
                let (r, keep): (Vec<_>, Vec<_>) = s.held.drain(..).partition(|h| h.0 == key);
                s.held = keep;
                r
            });
            for (_, id, ka, unit) in rel {
                with_state(|s| s.log.push((id, false)));
                run(app, ka, unit, false);
            }
            continue;
        }
        if typing {
            continue;
        }
        // The tree has the keyboard focus: its own keys come first, whatever the mapping says
        // (rekordbox: the focused `browse::CustomTreeView::keyPressed` 0x101ff6cec takes plain
        // Up / Down, Left = `moveOutOfSelectedItem`, Right = `moveIntoSelectedItem` and Return
        // = `toggleOpenSelectedItem` before the key mapping sees them).
        if app.dialogs.ext.focus_tree && modifiers.is_none() && tree_key(app, key) {
            ran.push((key, modifiers));
            continue;
        }
        let hits: Vec<Bind> = with_state(|s| s.table.iter().filter(|b| b.sc.key == key && b.sc.mods_match(&modifiers, false)).cloned().collect());
        for b in hits {
            if repeat && !b.ka.repeats() {
                ran.push((key, modifiers));
                continue;
            }
            if b.ka.gate() && !repeat {
                with_state(|s| s.held.push((key, b.id, b.ka, b.unit)));
            }
            with_state(|s| s.log.push((b.id, true)));
            run(app, b.ka, b.unit, true);
            ran.push((key, modifiers));
        }
    }
    if !ran.is_empty() {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. } if ran.contains(&(*key, *modifiers)))));
    }
}

/// A key of the tree with the focus (see `frame`). True: it was the tree's.
pub fn tree_key(app: &mut App, key: egui::Key) -> bool {
    use egui::Key as K;
    match key {
        K::ArrowUp => crate::prefs_ext::tree_step(app, -1),
        K::ArrowDown => crate::prefs_ext::tree_step(app, 1),
        K::ArrowLeft | K::ArrowRight | K::Enter => {
            let View::Playlist(id) = app.view else { return true };
            let Some(node) = app.node(id).cloned() else { return true };
            let folder = node.kind == NodeKind::Folder;
            let open = app.expanded.contains(&id);
            match key {
                // Out: an open folder closes, else its parent folder is selected.
                K::ArrowLeft => {
                    if folder && open {
                        app.expanded.remove(&id);
                    } else if node.parent != 0 {
                        app.side = SideMode::Playlists;
                        app.set_view(View::Playlist(node.parent));
                    }
                }
                // In: a closed folder opens, an open one selects its first child.
                K::ArrowRight => {
                    if folder && !open {
                        app.expanded.insert(id);
                    } else if folder {
                        let first = crate::prefs_ext::visible_nodes(&app.nodes, &app.expanded).into_iter().find(|n| app.node(*n).map(|x| x.parent == id).unwrap_or(false));
                        if let Some(c) = first {
                            app.set_view(View::Playlist(c));
                        }
                    }
                }
                _ => {
                    if folder {
                        if open {
                            app.expanded.remove(&id);
                        } else {
                            app.expanded.insert(id);
                        }
                    }
                }
            }
        }
        _ => return false,
    }
    true
}

/// The Preferences > Keyboard page is waiting for the key of a command.
fn prefs_waits_for_key(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Option<u16>>(egui::Id::new("prefs_key_wait"))).flatten().is_some()
}

/// A key holds a hot cue or a pad of deck `n` (the on-screen pads leave its preview alone).
pub fn holding(n: u8) -> bool {
    with_state(|s| s.held.iter().any(|h| h.3 == n && matches!(h.2, Ka::Hot(_) | Ka::Pad(_) | Ka::Cue)))
}

/// Let go of every held command (focus lost, mode changed).
pub fn release_all(app: &mut App) {
    let held: Vec<(egui::Key, u16, Ka, u8)> = with_state(|s| s.held.drain(..).collect());
    for (_, _, ka, unit) in held {
        run(app, ka, unit, false);
    }
}

/// The commands that ran (for the scripted run), emptied.
pub fn take_log() -> Vec<(u16, bool)> {
    with_state(|s| std::mem::take(&mut s.log))
}

/// The hub `midi::apply` needs: the controllers' one, or one of the keyboard's own when no
/// MIDI service runs (it shares the decks and has no devices).
fn hub(app: &App) -> Arc<crate::midi::Hub> {
    if let Some(h) = app.midi.hub.clone() {
        return h;
    }
    with_state(|s| s.hub.get_or_insert_with(|| crate::midi::Hub::new(app.player.decks.clone(), crate::db::data_dir().join("MidiMappings"), !app.deck_options.disable_play).0).clone())
}

/// A registry function run as a press or release of a button on deck `unit`.
fn midi(app: &mut App, name: &str, unit: u8, press: bool) {
    let reg = crate::midi::registry();
    let Some(f) = reg.by_name(name) else { return };
    let h = hub(app);
    let deck = if f.per_deck { unit + 1 } else { 0 };
    crate::midi::apply(app, &h, "keyboard", f, deck, crate::midi_map::Val::On(press, if press { 127 } else { 0 }), false, 0);
}

fn selected(app: &App) -> Vec<i64> {
    let mut v: Vec<i64> = app.list.iter().copied().filter(|id| app.selected.contains(id)).collect();
    if v.is_empty() {
        if let Some(a) = app.anchor_id {
            v.push(a);
        }
    }
    v
}

fn anchor(app: &mut App) -> Option<i64> {
    if app.list_dirty {
        app.rebuild_list();
    }
    app.anchor_id.filter(|id| app.list.contains(id)).or_else(|| app.selected.iter().next().copied())
}

/// Steps of the mixer's keys as parts of the control's range: small, medium, large
/// (`MixerControlPanel::perform` 0x1000e4db0: 0.03, 0.1 and 0.5; trim and auto gain have
/// small 0.03 and large 0.1).
const STEPS: [f32; 3] = [0.03, 0.1, 0.5];
/// Increase / Decrease Parameter of an FX unit: 0.01 of the knob's range
/// (`RbxMultiFxUnitPanel::performCommand` 0x101835d90, `RbxSingleFxUnitPanel::performCommand`
/// 0x101d572d0).
const FX_STEP: f32 = 0.01;

/// Carry out a command for `unit` (deck, channel or FX unit). `press`: key down, else up.
pub fn run(app: &mut App, ka: Ka, unit: u8, press: bool) {
    use Ka::*;
    let perf = app.performance();
    if !press {
        // Releases: only the held commands react.
        match ka {
            Cue => on_deck(app, unit, |a| a.cue_up()),
            Hot(_) => on_deck(app, unit, |a| a.hot_cue_up()),
            Bend(_) => on_deck(app, unit, |a| a.pitch_bend(0)),
            Pad(i) => pad(app, unit, i, false),
            FxReleaseOn => app.px.fx[unit as usize % 2].params.release = None,
            Midi(n) => midi(app, n, unit, false),
            Action(a) => {
                if let Some(n) = a.strip_prefix("smp_play").and_then(|n| n.parse::<usize>().ok()) {
                    app.sampler_pad_up(n - 1);
                }
            }
            _ => {}
        }
        return;
    }
    match ka {
        // ---- a deck
        Play => on_deck(app, unit, |a| if a.performance() { a.play_pause_synced() } else { a.play_pause() }),
        Cue => on_deck(app, unit, |a| a.cue_down()),
        JumpDefault(d) => on_deck(app, unit, |a| {
            let b = a.prefs.beat_jump;
            a.beat_jump(d as f64 * b)
        }),
        LoopIn => on_deck(app, unit, |a| a.loop_in_pressed()),
        LoopOut => on_deck(app, unit, |a| a.loop_out_pressed()),
        Reloop => on_deck(app, unit, |a| a.exit_reloop()),
        BeatLoop(b) => on_deck(app, unit, |a| beat_loop(a, b)),
        LoopResize(f) => on_deck(app, unit, |a| a.resize_loop(f)),
        Track(d) => on_deck(app, unit, |a| if d > 0 { a.load_adjacent(1) } else { a.previous_track() }),
        Quantize => on_deck(app, unit, |a| if a.performance() { a.toggle_quantize() } else { crate::prefs_ext::run_action(a, crate::prefs_ext::Act::Quantize) }),
        TimeMode => crate::prefs_ext::run_action(app, crate::prefs_ext::Act::TimeMode),
        Hot(s) => on_deck(app, unit, |a| a.hot_cue_down(s as i32)),
        HotClear(s) => on_deck(app, unit, |a| crate::prefs_ext::run_action(a, crate::prefs_ext::Act::ClearHot(s as i32))),
        MemSet => on_deck(app, unit, |a| a.memory_set()),
        MemCall(n) => on_deck(app, unit, |a| crate::prefs_ext::run_action(a, crate::prefs_ext::Act::MemCall(n as usize))),
        MemDelete(n) => on_deck(app, unit, |a| crate::prefs_ext::run_action(a, crate::prefs_ext::Act::MemDelete(n as usize))),
        MemStep(d) => on_deck(app, unit, |a| a.memory_jump(d as i32)),
        MemDeleteCurrent => on_deck(app, unit, |a| a.memory_delete_current()),
        Panel(p) => on_deck(app, unit, |a| {
            // 0 PAD (the CUE/LOOP panel), 1 GRID, 2 the memory cue list.
            a.deck.ctrl_tab = match p {
                1 if a.deck.ctrl_tab != 1 => 1,
                2 if a.deck.ctrl_tab != 2 => 2,
                _ => 0,
            };
        }),
        GridShift(d, large) => on_deck(app, unit, |a| a.grid_shift(d as f64 * if large { 0.005 } else { 0.001 })),
        GridCenter => on_deck(app, unit, |a| a.grid_center()),
        GridInterval(d, large) => on_deck(app, unit, |a| {
            // (Expand = the beats further apart = a lower BPM; 0.01 BPM a step, 0.1 large.)
            let step = if large { 0.1 } else { 0.01 };
            a.grid_scale_bpm(|v| ((v - d as f64 * step) * 100.0).round() / 100.0)
        }),
        GridWhole(whole) => {
            app.prefs.grid_from_current = !whole;
            app.save_prefs();
        }
        Tap => on_deck(app, unit, |a| a.grid_tap()),
        Sync => on_deck(app, unit, |a| a.toggle_beat_sync()),
        Master => on_deck(app, unit, |a| a.set_master()),
        MasterTempo => on_deck(app, unit, |a| a.toggle_master_tempo()),
        TempoReset => on_deck(app, unit, |a| a.tempo_reset()),
        Bend(d) => on_deck(app, unit, |a| a.pitch_bend(d as i32)),
        AnalysisLock => on_deck(app, unit, |a| {
            if let Some(id) = a.deck.track {
                if let Some(t) = a.track_mut(id) {
                    t.grid_locked = !t.grid_locked;
                }
                a.save_track(id);
            }
        }),
        TrackStart => on_deck(app, unit, |a| {
            a.seek(0.0);
            if !a.playing() {
                a.deck.cue_point = 0.0;
            }
        }),
        TempoStep(d, size) => on_deck(app, unit, |a| {
            if a.deck.audio.is_none() || a.sync_follower() {
                return;
            }
            // Small, medium and large: 0.02 %, 0.1 %, 1 % of tempo (recordbox's choice).
            let pct = [0.0002, 0.001, 0.01][size.min(2) as usize];
            let range = a.tempo_range();
            a.deck.x.tempo_adj = (a.deck.x.tempo_adj + d as f64 * pct).clamp(-range, range);
            a.push_deck_controls();
        }),
        KeyShift(d) => on_deck(app, unit, |a| a.key_shift_step(d as i32)),
        KeyReset => on_deck(app, unit, |a| a.key_reset()),
        KeySync => on_deck(app, unit, |a| a.toggle_key_sync()),
        BeatJump(b) => on_deck(app, unit, |a| {
            let id = beat_jump_id(b);
            a.pad_beat_jump(id)
        }),
        AutoBeatLoop => on_deck(app, unit, |a| {
            if a.loop_active().is_some() {
                a.loop_exit();
            } else {
                let b = crate::prefs_bridge::auto_beat_loop_beats(&a.prefs);
                a.auto_loop(b);
            }
        }),
        Metronome => on_deck(app, unit, |a| {
            a.deck.metronome = !a.deck.metronome;
            a.update_metronome();
        }),
        MetronomeVolume => {
            let v = app.prefs.metronome_vol.clamp(1, 3);
            app.set_metronome_volume(v % 3 + 1);
        }
        GridHalfDouble(d) => on_deck(app, unit, |a| a.grid_double_half(d)),
        SetDownbeat => on_deck(app, unit, |a| a.grid_set_downbeat()),
        GridUndo(undo) => on_deck(app, unit, |a| if undo { a.grid_undo() } else { a.grid_redo() }),
        MemLoopActive(n) => on_deck(app, unit, |a| toggle_active_loop(a, CueKind::Memory, n as i32)),
        HotLoopActive(n) => on_deck(app, unit, |a| toggle_active_loop(a, CueKind::Hot, n as i32)),
        StemPart(src) => {
            let srcs: Vec<usize> = if app.prefs.stem_parts == 3 && src == 2 { vec![2, 1] } else { vec![src as usize] };
            on_deck(app, unit, |a| {
                if a.deck.track.is_some() {
                    crate::prefs_ext::stem_part_click(a, &srcs)
                }
            })
        }
        PadMode(m) => {
            if perf {
                on_deck(app, unit, |a| {
                    a.set_pad_mode(m);
                });
            }
        }
        Pad(i) => pad(app, unit, i, true),
        PadParam(p, d) => pad_param(app, unit, p, d),
        Midi(n) => {
            if perf {
                midi(app, n, unit, true)
            }
        }
        DrumBank(b) => app.groove_set_bank(unit as usize, b),
        DrumSlot(s) => {
            let bank = app.gc.set.decks[(unit as usize).min(3)].bank as usize;
            app.groove_slot_play(unit as usize, bank * 4 + s as usize)
        }
        // ---- all decks, the browser, the menus
        Zoom(d) => crate::prefs_ext::run_action(app, crate::prefs_ext::Act::Zoom(-(d as i32))),
        Volume(d) => crate::prefs_ext::run_action(app, crate::prefs_ext::Act::Volume(d as f32 * 0.05)),
        Menu(id) => crate::menus::handle(app, id),
        Action(id) => {
            if let Some(n) = id.strip_prefix("smp_play").and_then(|n| n.parse::<usize>().ok()) {
                // (A slot's key held is a gate; the release stops gate samples.)
                app.sampler_pad_down(n - 1, 1.0, false, false);
            } else if let Some(a) = crate::prefs_ext::Act::parse(id) {
                crate::prefs_ext::run_action(app, a);
            }
        }
        LoadDeck(n) => load_to(app, n),
        LoadSide(right) => {
            let n = side_deck(app, right);
            load_to(app, n)
        }
        Doubles(n) => doubles(app, n),
        DoublesSide(right) => {
            let n = side_deck(app, right);
            doubles(app, n)
        }
        Layout(k) => {
            if !perf {
                app.set_player_layout(k)
            }
        }
        PerfLayout(k) => crate::app_hooks::set_perf_layout(app, k),
        SearchList => app.ctx.data_mut(|d| d.insert_temp(egui::Id::new("kc_search_focus"), true)),
        SearchCollection => {
            app.side = SideMode::Collection;
            app.set_view(View::Collection);
            app.ctx.data_mut(|d| d.insert_temp(egui::Id::new("kc_search_focus"), true));
        }
        SearchTree => {
            app.ext.tree_search_open = true;
            app.ctx.data_mut(|d| d.insert_temp(egui::Id::new("tree_search_focus"), true));
        }
        PreviewPlay => {
            if app.preview_status().is_some() {
                app.preview_stop();
            } else if let Some(id) = anchor(app) {
                app.preview_play(id, 0.0);
            }
        }
        PreviewSkip => {
            // Skip: 15 s further into the track (recordbox's choice; not traced).
            if let Some((id, Some(frac))) = app.preview_status() {
                let dur = app.track(id).map(|t| t.duration).unwrap_or(0.0).max(1.0);
                app.preview_play(id, (frac + 15.0 / dur).min(0.99));
            }
        }
        ShortcutsToggle => {
            app.prefs.shortcuts_open = !app.prefs.shortcuts_open;
            app.save_prefs();
        }
        ShortcutOpen(n) => {
            if let Some(node) = app.prefs.shortcuts.get(n as usize).copied() {
                app.jump_to_playlist(node);
            }
        }
        ShortcutAdd(n) => {
            let ids = selected(app);
            if let Some(node) = app.prefs.shortcuts.get(n as usize).copied().filter(|nd| app.node(*nd).map(|x| x.kind == NodeKind::Playlist).unwrap_or(false)) {
                if !ids.is_empty() {
                    app.add_to_playlist(node, &ids);
                }
            }
        }
        TagListView => app.set_view(View::TagList),
        TagListAdd => {
            let ids = selected(app);
            if !ids.is_empty() {
                app.tag_list_add(&ids);
            }
        }
        PaletteBank(_) => {
            app.prefs.palette_open = true;
            app.save_prefs();
        }
        PaletteOpen(n) => {
            if let Some(pl) = app.prefs.palette.get(n as usize).copied().filter(|p| *p != 0) {
                app.jump_to_playlist(pl);
            }
        }
        PaletteAdd(n) => {
            let ids = selected(app);
            if let Some(pl) = app.prefs.palette.get(n as usize).copied().filter(|p| *p != 0) {
                if !ids.is_empty() {
                    app.add_to_playlist(pl, &ids);
                }
            }
        }
        ShowListOfTrack => {
            // The list the selected track was loaded from is not kept: the collection with
            // the track selected.
            if let Some(id) = anchor(app) {
                app.side = SideMode::Collection;
                app.set_view(View::Collection);
                app.selected.clear();
                app.selected.insert(id);
                app.rebuild_list();
                if let Some(i) = app.list.iter().position(|x| *x == id) {
                    app.set_anchor(i);
                }
                app.scroll_to_selected = true;
            }
        }
        Related => {
            let v = if app.view == View::Related { View::Collection } else { View::Related };
            app.set_view(v);
        }
        UndoLoad => {
            // The deck's previous track, at its place (rekordbox's "Undo track load").
            if let Some((old, pos, gain)) = app.deck_backup.take() {
                let mut s = app.player.state.lock();
                s.audio = old.audio.clone();
                s.pos = pos;
                s.gain = gain;
                s.playing = false;
                drop(s);
                app.deck = old;
            }
        }
        Bold => {
            app.prefs.browse_bold = !app.prefs.browse_bold;
            app.save_prefs();
        }
        LocateLoaded => {
            if let Some(id) = app.deck.track {
                app.rebuild_list();
                if !app.list.contains(&id) {
                    app.side = SideMode::Collection;
                    app.set_view(View::Collection);
                    app.rebuild_list();
                }
                if let Some(i) = app.list.iter().position(|x| *x == id) {
                    app.set_anchor(i);
                    app.selected.clear();
                    app.selected.insert(id);
                    app.scroll_to_selected = true;
                }
            }
        }
        ColumnView(on) => {
            app.prefs.bx.column_view = on;
            app.save_prefs();
        }
        AnalysisLockSel(on) => crate::menus::handle(app, if on { "lock_on" } else { "lock_off" }),
        Quit => app.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        // ---- mixer
        Crossfader(d, size) => app.px.crossfader = (app.px.crossfader + d as f32 * STEPS[size.min(2) as usize]).clamp(0.0, 1.0),
        Trim(d, large) => {
            let m = &mut app.px.mix[unit as usize % 4];
            m.trim = (m.trim + d as f32 * STEPS[large as usize]).clamp(0.0, 1.0);
        }
        HeadCue => {
            let m = &mut app.px.mix[unit as usize % 4];
            m.cue = !m.cue;
        }
        Fader(d, size) => {
            let m = &mut app.px.mix[unit as usize % 4];
            m.fader = (m.fader + d as f32 * STEPS[size.min(2) as usize]).clamp(0.0, 1.0);
        }
        AutoGain(d, large) => {
            // The deck's A-GAIN knob: 0.03 or 0.1 of its range a step, as the TRIM keys.
            let step = d as f32 * STEPS[large as usize];
            on_deck(app, unit, |a| {
                let pos = (crate::deck::again_pos(a.gain_db()) + step).clamp(0.0, 1.0);
                a.set_gain_db(crate::deck::again_db(pos), true);
            });
        }
        // ---- effects
        FxAssign(ch) => {
            let ch = match ch {
                4 => crate::fx::SAMPLER,
                5 => crate::fx::MASTER,
                c => c as usize,
            };
            // (As the panel's buttons: M lit clears the others, another lit clears M.)
            let ch = ch.min(crate::fx::CHANNELS - 1);
            let a = &mut app.px.fx[unit as usize % 2].params.assign;
            a[ch] = !a[ch];
            if a[ch] {
                for (i, d) in a.iter_mut().enumerate() {
                    if (ch == crate::fx::MASTER) != (i == crate::fx::MASTER) {
                        *d = false;
                    }
                }
            }
        }
        FxMode => midi(app, if unit == 0 { "FX1FXMode" } else { "FX2FXMode" }, 0, true),
        FxSelect(s) => midi(app, &format!("FX{}-{}Select", unit % 2 + 1, s + 1), 0, true),
        FxOn(s) => midi(app, &format!("FX{}-{}On", unit % 2 + 1, s + 1), 0, true),
        FxParam(s, d) => {
            let fx = &mut app.px.fx[unit as usize % 2];
            let step = d as f32 * FX_STEP;
            if fx.params.single {
                // Single mode: LEVEL/DEPTH, then the two knobs.
                match s {
                    0 => fx.params.one.level = (fx.params.one.level + step).clamp(0.0, 1.0),
                    k => fx.params.one.knob[(k - 1) as usize % 2] = (fx.params.one.knob[(k - 1) as usize % 2] + step).clamp(0.0, 1.0),
                }
            } else {
                let sl = &mut fx.params.slots[s as usize % 3];
                sl.level = (sl.level + step).clamp(0.0, 1.0);
            }
        }
        FxReleaseSelect => {
            let fx = &mut app.px.fx[unit as usize % 2];
            fx.release_kind = (fx.release_kind + 1) % 3;
        }
        FxReleaseOn => midi(app, if unit == 0 { "FX1ReleaseFXOn" } else { "FX2ReleaseFXOn" }, 0, true),
        FxReleaseBeat(d) => midi(app, &format!("FX{}ReleaseFXBeat{}", unit % 2 + 1, if d > 0 { "Up" } else { "Down" }), 0, true),
        FxSlotBeat(s, d) => midi(app, &format!("FX{}-{}Beat{}", unit % 2 + 1, s + 1, if d > 0 { "Up" } else { "Down" }), 0, true),
        FxSingleBeat(d) => midi(app, &format!("FX{}Beat{}", unit % 2 + 1, if d > 0 { "Up" } else { "Down" }), 0, true),
        Cfx(i) => {
            let list: Vec<u8> = crate::cfx::LIST.iter().filter_map(|x| x.1).collect();
            if let Some(k) = list.get(i as usize).copied() {
                app.px.cfx_user = true;
                app.px.cfx_kind = k;
                app.px.cfx_param = app.px.fxx.cfx_params[k as usize % 9];
            }
        }
        StemFx(r) => {
            let r = (r as usize).min(3);
            app.px.stem_fx[r] = !app.px.stem_fx[r];
        }
        // ---- MIX POINT LINK
        Mpl(c) => match c {
            0 => {
                // Toggle Mix In and Mix Out: the mode of the mix point setting.
                let m = if app.mixx.mpl.mode == 0 { 1 } else { 0 };
                app.mpl_set_mode(m);
            }
            1 => app.mpl_select_cue(-1),
            2 => app.mpl_select_cue(1),
            3 => app.mpl_toggle_silent(),
            4 => app.mpl_toggle_silent_edit(),
            5 => app.mpl_request_link(),
            6 | 7 => {
                // Slide the OverView: the lanes 4 s to the left or right (recordbox's step;
                // rekordbox's was not traced).
                let d = if c == 6 { -4.0 } else { 4.0 };
                for v in app.mixx.mpl.ui.view.iter_mut() {
                    *v = (*v + d).max(0.0);
                }
            }
            12 | 13 => {
                for d in 0..4 {
                    app.mpl_cancel(d, if c == 12 { 1 } else { 2 });
                }
            }
            _ => {}
        },
    }
}

/// `f` with deck `unit` in place, when that deck exists in the layout showing.
fn on_deck(app: &mut App, unit: u8, f: impl FnOnce(&mut App)) {
    if unit < app.deck_count() {
        app.with_deck(unit, f);
    }
}

fn beat_loop(a: &mut App, beats: f64) {
    let cur = a.loop_active().map(|(s, e)| (e - s) / a.beat_len().max(1e-9));
    if cur.map(|c| (c / beats - 1.0).abs() < 0.02).unwrap_or(false) {
        a.loop_exit();
    } else {
        a.prefs.loop_beats = beats;
        a.auto_loop(beats);
    }
}

/// The BEAT JUMP pad id (`pads::jump_beats`) of a jump of `b` beats (negative: back).
pub fn beat_jump_id(b: f64) -> u8 {
    let fwd = b > 0.0;
    let id = (2..24u8).step_by(2).find(|i| crate::pads::jump_beats(*i).is_some_and(|x| (x - b.abs()).abs() < 1e-9)).unwrap_or(8);
    id + fwd as u8
}

/// A memory or hot loop's ACTIVE flag on / off ("Memory Loop n Active On/Off").
fn toggle_active_loop(a: &mut App, kind: CueKind, n: i32) {
    let c = match kind {
        CueKind::Memory => a.memory_cues().into_iter().filter(|c| c.end.is_some()).nth(n as usize),
        _ => a.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == n && c.end.is_some()).cloned(),
    };
    if let Some(mut c) = c {
        c.active_loop = !c.active_loop;
        a.update_cue(c);
    }
}

fn side_deck(app: &App, right: bool) -> u8 {
    // The left decks are 1 and 3, the right 2 and 4: the one of the pair in focus.
    let a = app.px.active;
    let left = if a == 2 { 2 } else { 0 };
    let r = if a == 3 { 3 } else { 1 };
    if right { r } else { left }
}

fn load_to(app: &mut App, n: u8) {
    if n >= app.deck_count() {
        return;
    }
    if let Some(id) = anchor(app) {
        app.with_deck(n, |a| a.load_track_here(id));
    }
}

/// Instant Doubles onto deck `n`: the track of the deck that plays (else the first loaded
/// one), at its place.
fn doubles(app: &mut App, n: u8) {
    if !app.performance() || n >= app.deck_count() {
        return;
    }
    let count = app.deck_count();
    let playing = |app: &App, k: u8| app.player.decks[k as usize].lock().playing;
    let Some(src) = (0..count).filter(|k| *k != n && app.deck_of(*k).track.is_some()).max_by_key(|k| playing(app, *k)) else { return };
    app.with_deck(n, |a| a.instant_double_from(src));
}

/// A deck pad (A..P) in the pad mode the deck shows.
fn pad(app: &mut App, unit: u8, i: u8, down: bool) {
    use crate::pads::PadMode as P;
    if !app.performance() || unit >= app.deck_count() {
        return;
    }
    let n = unit;
    let mode = app.with_deck(n, |a| a.pad_mode());
    let idx = i as usize;
    match mode {
        P::HotCue => app.with_deck(n, |a| if down { a.hot_cue_down(i as i32) } else { a.hot_cue_up() }),
        P::BeatJump => {
            if down {
                let page = app.prefs.pads.beat_jump_page[(n as usize).min(3)];
                let id = crate::pads::jump_page_first(page) + i;
                if id < 24 {
                    app.with_deck(n, |a| a.pad_beat_jump(id));
                }
            }
        }
        P::BeatLoop => {
            if down {
                app.with_deck(n, |a| a.pad_beat_loop(idx));
            }
        }
        P::KeyShift => {
            if down {
                let page = app.prefs.pads.key_shift_page[(n as usize).min(3)];
                app.with_deck(n, |a| a.pad_key(crate::pads::key_pad_pos(page, idx)));
            }
        }
        P::Keyboard => {
            let page = app.prefs.pads.keyboard_page[(n as usize).min(3)];
            app.with_deck(n, |a| if down { a.pad_keyboard_down(crate::pads::key_pad_pos(page, idx)) } else { a.pad_keyboard_up() });
        }
        P::Slicer => {
            if i < 8 {
                app.with_deck(n, |a| a.slicer_pad(i, down));
            }
        }
        P::Sampler => {
            let p = idx + if n % 2 == 1 && app.sampler_pads() > 8 { 8 } else { 0 };
            if down {
                app.sampler_deck_pad_down(n, p, false);
            } else {
                app.sampler_pad_up(p);
            }
        }
        P::SeqCall => {
            if down && i < 8 {
                app.sampler_seq_call(idx, false);
            }
        }
        P::PadFx => {
            if down {
                crate::ui_padfx::pad_down(app, n as usize, i);
            } else {
                crate::ui_padfx::pad_up(app, n as usize, i);
            }
        }
        P::MemoryCue => {
            if down {
                app.with_deck(n, |a| crate::prefs_ext::run_action(a, crate::prefs_ext::Act::MemCall(idx)));
            }
        }
        P::User(u) => app.with_deck(n, |a| {
            if down {
                if let Some(item) = a.user_items(u as usize).get(idx).cloned() {
                    a.user_pad_down(&item, false);
                }
            } else {
                a.user_pad_up();
            }
        }),
        P::Stems => {
            // (Pads 1..4: the parts VOCAL, INST, BASS, DRUMS, as the STEMS pad mode of the
            // controllers orders them.)
            if down && i < 4 {
                let src = [3usize, 2, 1, 0][idx];
                let srcs: Vec<usize> = if app.prefs.stem_parts == 3 && src == 2 { vec![2, 1] } else { vec![src] };
                app.with_deck(n, |a| crate::prefs_ext::stem_part_click(a, &srcs));
            }
        }
        P::ActiveCensor | P::SamplerScratch => {}
    }
}

/// Parameter 1 / 2 <- and -> of the pad mode the deck shows (as the controllers' buttons).
fn pad_param(app: &mut App, unit: u8, p: u8, dir: i8) {
    use crate::pads::PadMode as P;
    if !app.performance() || unit >= app.deck_count() {
        return;
    }
    let n = unit;
    let ni = (n as usize).min(3);
    let mode = app.with_deck(n, |a| a.pad_mode());
    match (mode, p) {
        (P::HotCue, 1) => app.with_deck(n, |a| a.memory_jump(dir as i32)),
        (P::BeatJump, 1) => {
            let pg = app.prefs.pads.beat_jump_page[ni];
            app.prefs.pads.beat_jump_page[ni] = crate::pads::jump_page_step(pg, dir > 0, false);
            app.save_prefs();
        }
        (P::KeyShift, 1) => app.prefs.pads.key_shift_page[ni] = (app.prefs.pads.key_shift_page[ni] as i8 + dir).clamp(1, 5) as u8,
        (P::Keyboard, 1) => app.prefs.pads.keyboard_page[ni] = (app.prefs.pads.keyboard_page[ni] as i8 + dir).clamp(1, 5) as u8,
        (P::Slicer, 1) => {
            let ri = (app.prefs.pads.slicer_roll[ni] as i32 + dir as i32).max(0) as usize;
            app.with_deck(n, |a| a.slicer_set_roll(ri));
        }
        (P::Slicer, 2) => {
            let li = (app.prefs.pads.slicer_length[ni] as i32 + dir as i32).clamp(0, 3) as usize;
            app.with_deck(n, |a| a.slicer_set_length(li));
        }
        (P::Sampler, 1) => {
            let b = (app.prefs.sampler.bank as i32 + dir as i32).clamp(0, 3) as u8;
            app.sampler_action(crate::sampler_ext::SmpAct::Bank(b));
        }
        (P::MemoryCue, 1) => app.with_deck(n, |a| a.memory_jump(dir as i32)),
        _ => {}
    }
}

// ------------------------------------------------------------------ EDIT mode

/// The editor's command for a keyboard row of EDIT mode.
pub fn edit_cmd(id: u16) -> Option<crate::ui_edit::Cmd> {
    use crate::edit_model::{EditCmd as E, SelectCmd as S};
    use crate::ui_edit::Cmd as C;
    Some(match id {
        0x2013 => C::NewProject,
        0x2014 => C::OpenProject,
        0x2015 => C::Save,
        0x2016 => C::SaveAs,
        0x2017 => C::Render,
        0xcc00 => C::Edit(E::Clone),
        0xcc01 => C::Edit(E::Copy),
        0xcc02 => C::Edit(E::Cut),
        0xcc03 => C::Edit(E::Paste),
        0xcc04 => C::Edit(E::Insert),
        0xcc05 => C::Edit(E::Delete),
        0xcc06 => C::Edit(E::Clear),
        0xcc07 => C::Edit(E::Undo),
        0xcc08 => C::Edit(E::Redo),
        0xcc09 => C::Select(S::Half),
        0xcc0a => C::Select(S::Double),
        0xcc0b => C::Select(S::Cancel),
        0xcc0c..=0xcc13 => C::Select(S::ByBeat(1 << (id - 0xcc0c))),
        0xcc14 => C::AddToPalette,
        0xcc15 => C::PlayPause,
        0xcc16 => C::Head,
        0xcc17 => C::Loop,
        0xcc18 => C::ZoomIn,
        0xcc19 => C::ZoomOut,
        0xcc1a => C::ZoomReset,
        0xcc1b => C::Browser,
        _ => return None,
    })
}

/// EDIT mode's keys from the mapping in use (ui_edit.rs): the commands of this frame's key
/// presses. "*" also arrives as text on layouts where it has no key of its own.
pub fn edit_keys(app: &App, ctx: &egui::Context) -> Vec<crate::ui_edit::Cmd> {
    let mode = Mode::Edit;
    let cur = keys::current(&app.prefs, mode);
    let rows = keys::commands(mode);
    let binds: Vec<(Shortcut, Option<String>, u16)> = rows
        .iter()
        .filter_map(|r| {
            let k = keys::key_of(&app.prefs, mode, &cur, r);
            if k.is_empty() {
                return None;
            }
            let last = k.rsplit(" + ").next().unwrap_or("").to_string();
            let sc = keys::juce_to_stored(&k).and_then(|s| Shortcut::parse(&s));
            match sc {
                Some(sc) => Some((sc, None, r.id)),
                // (A key egui has no name for, such as "*": matched by its text.)
                None if last.chars().count() == 1 => Some((Shortcut { key: egui::Key::Space, ctrl: false, alt: false, shift: false, cmd: false }, Some(last), r.id)),
                None => None,
            }
        })
        .collect();
    let mut out = vec![];
    ctx.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
                    for (sc, text, id) in &binds {
                        // (Delete answers to both of the Mac's delete keys, as in rekordbox.)
                        let k = if *key == egui::Key::Delete { egui::Key::Backspace } else { *key };
                        let want = if sc.key == egui::Key::Delete { egui::Key::Backspace } else { sc.key };
                        if text.is_none() && k == want && sc.mods_match(modifiers, false) {
                            if let Some(c) = edit_cmd(*id) {
                                out.push(c);
                            }
                        }
                    }
                }
                egui::Event::Text(t) => {
                    for (_, text, id) in &binds {
                        if text.as_deref() == Some(t.as_str()) {
                            if let Some(c) = edit_cmd(*id) {
                                out.push(c);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    });
    out
}

// ------------------------------------------------------------------ the report's table

/// Per category of a mode: (category, commands, acting, the ids that do not act).
pub fn coverage(mode: Mode) -> Vec<(String, usize, usize, Vec<u16>)> {
    let mut v: Vec<(String, usize, usize, Vec<u16>)> = vec![];
    for r in keys::commands(mode) {
        let a = acts(r.id, mode);
        match v.iter_mut().find(|x| x.0 == r.cat) {
            Some(x) => {
                x.1 += 1;
                if a {
                    x.2 += 1;
                } else {
                    x.3.push(r.id);
                }
            }
            None => v.push((r.cat.clone(), 1, a as usize, if a { vec![] } else { vec![r.id] })),
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_split_into_base_and_unit() {
        assert_eq!(split_id(0x3006), (0x3006, 0));
        assert_eq!(split_id(0x3106), (0x3006, 1));
        assert_eq!(split_id(0x3306), (0x3006, 3));
        assert_eq!(split_id(0xd110), (0xd010, 1));
        assert_eq!(split_id(0xe10a), (0xe00a, 1));
        // A command without units keeps its id (0xb129 is not deck 2 of 0xb029).
        assert_eq!(split_id(0xb129), (0xb129, 0));
        assert_eq!(split_id(0xf010), (0xf010, 0));
    }

    #[test]
    fn every_deck_command_of_each_mode_has_a_reason_or_an_action() {
        for m in [Mode::Export, Mode::Performance] {
            for r in keys::commands(m) {
                let (base, _) = split_id(r.id);
                if action(base).is_none() {
                    assert!(!missing_reason_in(base, m).is_empty(), "{:x} {} has neither", r.id, r.name);
                }
            }
        }
        // The deck rows of every deck act, not only deck 1's.
        assert!(acts(0x3306, Mode::Performance) && acts(0x3107, Mode::Export));
        assert!(!acts(0xd010, Mode::Export), "no mixer in EXPORT");
        assert!(acts(0xd010, Mode::Performance));
        // Registry functions named by the table exist.
        let reg = crate::midi::registry();
        for base in 0x3000u16..0x3100 {
            if let Some(Ka::Midi(n)) = action(base) {
                assert!(reg.by_name(n).is_some(), "{n}");
            }
        }
        for u in 1..=2 {
            for s in 1..=3 {
                for n in [format!("FX{u}-{s}Select"), format!("FX{u}-{s}On"), format!("FX{u}-{s}BeatUp"), format!("FX{u}-{s}BeatDown")] {
                    assert!(reg.by_name(&n).is_some(), "{n}");
                }
            }
            for n in [format!("FX{u}FXMode"), format!("FX{u}ReleaseFXOn"), format!("FX{u}ReleaseFXBeatUp"), format!("FX{u}BeatDown")] {
                assert!(reg.by_name(&n).is_some(), "{n}");
            }
        }
    }

    #[test]
    fn edit_rows_all_act_and_the_browser_key_is_b() {
        let rows = keys::commands(Mode::Edit);
        assert!(rows.iter().all(|r| edit_cmd(r.id).is_some()), "every EDIT row has a command");
        let b = rows.iter().find(|r| r.id == 0xcc1b).expect("the Browser row is listed in EDIT");
        assert_eq!(keys::preset_key(b, keys::EDIT_PRESET.0), "B");
        assert_eq!(edit_cmd(0xcc0e), Some(crate::ui_edit::Cmd::Select(crate::edit_model::SelectCmd::ByBeat(4))));
        // EDIT's keys follow a mapping of one's own: a copy of the preset, with a key added.
        let mut p = Prefs::default();
        let m = Mode::Edit;
        keys::copy(&mut p, m, keys::EDIT_PRESET.0, "mine (edit)");
        keys::select(&mut p, m, "mine (edit)");
        assert_eq!(keys::current(&p, m), "mine (edit)");
        let sel4 = rows.iter().find(|r| r.id == 0xcc0e).unwrap();
        keys::assign(&mut p, m, "mine (edit)", sel4, "F1");
        assert_eq!(keys::key_of(&p, m, "mine (edit)", sel4), "F1");
        assert_eq!(keys::key_of(&p, m, "mine (edit)", b), "B", "the copy keeps the preset's keys");
        // It is listed in EDIT, not in the other modes; the PERFORMANCE mapping is untouched.
        assert!(keys::mappings(&p, m).iter().any(|x| x.0 == "mine (edit)"));
        assert!(!keys::mappings(&p, Mode::Performance).iter().any(|x| x.0 == "mine (edit)"));
        assert_eq!(keys::current(&p, Mode::Performance), keys::OWN);
        keys::select(&mut p, m, keys::EDIT_PRESET.0);
        assert_eq!(keys::current(&p, m), keys::EDIT_PRESET.0);
    }

    #[test]
    fn preset_tables_bind_every_deck() {
        let mut p = Prefs::default();
        keys::select(&mut p, Mode::Performance, "0000000000000");
        let t = build_table(&p, Mode::Performance);
        let find = |id: u16| t.iter().find(|b| b.id == id).map(|b| (b.sc.name(), b.unit, b.ka));
        assert_eq!(find(0x3006), Some(("Z".into(), 0, Ka::Play)));
        assert_eq!(find(0x3106), Some(("N".into(), 1, Ka::Play)));
        assert_eq!(find(0x3306), Some(("N".into(), 3, Ka::Play)), "deck 4 has deck 2's keys in Performance 1");
        assert_eq!(find(0x3061).map(|x| x.2), Some(Ka::Midi("SlipReverse")));
        assert_eq!(find(0xb129).map(|x| x.0), Shortcut::parse("Shift+ArrowLeft").map(|s| s.name()));
        // The old loop leaves the commands to this table under a preset, but keeps the list keys.
        assert!(!old_loop_runs(&p, "play") && old_loop_runs(&p, "list_up") && old_loop_runs(&p, "load"));
        // With recordbox's own mapping the old loop keeps deck 1's actions and this table has
        // the rest only.
        keys::select(&mut p, Mode::Performance, keys::OWN);
        assert!(old_loop_runs(&p, "play"));
        let t = build_table(&p, Mode::Performance);
        assert!(t.iter().all(|b| !(b.unit == 0 && keys::action_of(b.id, "x").is_some() && b.id < 0xf000)));
        // EXPORT: Player B is Player A with Shift.
        keys::select(&mut p, Mode::Export, "0000000000030");
        p.app_mode = 0;
        let t = build_table(&p, Mode::Export);
        let b = t.iter().find(|b| b.id == 0x3106).unwrap();
        assert_eq!((b.sc.name().as_str(), b.unit), ("Shift+Space", 1));
        assert!(t.iter().all(|b| !performance_only(b.ka)));
    }

    /// The report's table: `cargo test --release keycmd::tests::coverage_table -- --nocapture`.
    #[test]
    fn coverage_table() {
        let mut total = (0, 0);
        for m in [Mode::Export, Mode::Performance, Mode::Edit] {
            eprintln!("== {}", m.name());
            for (cat, n, a, miss) in coverage(m) {
                total.0 += n;
                total.1 += a;
                let names: Vec<String> = miss
                    .iter()
                    .map(|id| {
                        let (b, _) = split_id(*id);
                        let name = CMDS.iter().find(|c| c.id == b).map(|c| c.name).unwrap_or("?");
                        format!("{id:x} {name} ({})", missing_reason_in(b, m))
                    })
                    .collect();
                eprintln!("{cat}\t{n}\t{a}\t{}", names.join("; "));
            }
        }
        eprintln!("rows {} acting {}", total.0, total.1);
        // Unique commands (by base id) of the 523, in any mode.
        let mut ids = std::collections::BTreeSet::new();
        let mut acting = std::collections::BTreeSet::new();
        for m in [Mode::Export, Mode::Performance, Mode::Edit] {
            for r in keys::commands(m) {
                let b = split_id(r.id).0;
                ids.insert(b);
                if acts(r.id, m) {
                    acting.insert(b);
                }
            }
        }
        eprintln!("commands listed in some mode {} of {}, acting in some mode {}", ids.len(), CMDS.len(), acting.len());
        assert!(acting.len() * 10 > ids.len() * 8, "most commands act");
    }

    #[test]
    fn beat_jump_ids() {
        assert_eq!(beat_jump_id(1.0), 9);
        assert_eq!(beat_jump_id(-1.0), 8);
        assert_eq!(beat_jump_id(0.5), 7);
        assert_eq!(beat_jump_id(-32.0), 18);
        for b in [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0] {
            assert_eq!(crate::pads::jump_beats(beat_jump_id(b)), Some(b));
            assert!(crate::pads::jump_forward(beat_jump_id(b)) && !crate::pads::jump_forward(beat_jump_id(-b)));
        }
    }
}
