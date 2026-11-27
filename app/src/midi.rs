//! MIDI controllers in PERFORMANCE mode: the functions recordbox can be told to do by a
//! controller (the registry), the MIDI ports (CoreMIDI through `midir`), the worker of each
//! port, LED feedback, and what a mapped message does to the decks, the mixer, the FX units,
//! the sampler and the browser. After rekordbox 7.2.19 (`checks/re-7.2.19/controllers-midi.md`).
//!
//! Threads: the port's callback only queues the message; a worker per port looks it up in the
//! device's map (`midi_map.rs`). Rows with the `Fast` option (transport, pads, faders) and the
//! jog wheel act on the audio engine from the worker at once; every message also goes to the
//! interface thread (`poll`), which keeps the app's own state in step and does everything
//! else. An output thread per port sends LED messages one per `MidiOutInterval`, highest
//! priority first, and blinks.
//!
//! Function names are rekordbox's (the `Name` column of its mapping files and the names its
//! MIDI Learn window shows), so that mapping files move between the two. The list below is
//! recordbox's own: what it implements. The factory mappings and rekordbox's Learn catalogue
//! are AlphaTheta's and are not part of recordbox; factory mappings are read from the
//! installed rekordbox when a controller is connected.
//!
//! Not here on purpose: any hardware authentication handshake (controllers that need one
//! stay unsupported), HID players and PRO DJ LINK.

use crate::app::*;
use crate::audio::PlayState;
use crate::midi_csv::{Entry, Kind};
use crate::midi_map::*;
use crate::model::*;
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod ports;
mod routing;
use routing::{Platter, Routing};
pub(crate) use routing::change_mode;

// ---------------------------------------------------------------------- settings

/// The preferences that belong to controllers (a field of `Prefs`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MidiPrefs {
    /// rekordbox's TakeoverEqPartIso ("Takeover when switching to EQ/STEM ISO"), default off.
    pub takeover_eq_iso: bool,
    /// rekordbox's DeckNeedleLock (Lock by default): the needle strip does not seek while
    /// the deck plays.
    pub needle_lock: bool,
    /// JOG Rotation Speed: 45 rpm instead of 33 1/3.
    pub jog_45rpm: bool,
    /// The device last picked in the MIDI setting window.
    pub device: String,
}

impl Default for MidiPrefs {
    fn default() -> MidiPrefs {
        MidiPrefs { takeover_eq_iso: false, needle_lock: true, jog_45rpm: false, device: String::new() }
    }
}

// ---------------------------------------------------------------------- the registry

/// The MIDI setting window's tabs (rekordbox's order). VIDEO and LIGHTING are out of
/// recordbox's scope and have no functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Deck,
    Pad,
    Fx,
    Sampler,
    Mixer,
    Browse,
    Other,
    Video,
    Lighting,
    MixPointLink,
    /// Not offered by MIDI Learn (jog wheels, Shift): used by factory mappings only.
    None,
}

pub const TABS: [(Tab, &str); 10] = [(Tab::Deck, "DECK"), (Tab::Pad, "PAD"), (Tab::Fx, "FX"), (Tab::Sampler, "SAMPLER"), (Tab::Mixer, "MIXER"), (Tab::Browse, "BROWSE"), (Tab::Other, "OTHER"), (Tab::Video, "VIDEO"), (Tab::Lighting, "LIGHTING"), (Tab::MixPointLink, "MIXPOINTLINK")];

/// The Learn window's control types (rekordbox's numbers).
pub const T_BUTTON: u8 = 1;
pub const T_ROTARY: u8 = 2;
pub const T_KNOB7: u8 = 3;
pub const T_KNOB14: u8 = 4;
pub const T_INDICATOR: u8 = 5;
pub const T_PAD: u8 = 6;
pub const T_VALUE: u8 = 7;
pub const T_BUTTON_PAD: u8 = 8;

pub fn type_text(t: u8) -> &'static str {
    match t {
        T_BUTTON => "Button",
        T_ROTARY => "Rotary",
        T_KNOB7 => "Knob/Slider(0h-7Fh)",
        T_KNOB14 => "Knob/Slider(0h-3FFFh)",
        T_INDICATOR => "Indicator",
        T_PAD => "Pad",
        T_VALUE => "Value",
        T_BUTTON_PAD => "Button(for Pad)",
        _ => "",
    }
}

/// The types the Type list offers together: Button with Button(for Pad), 7-bit with 14-bit.
pub fn type_choices(t: u8) -> &'static [u8] {
    match t {
        T_BUTTON | T_BUTTON_PAD => &[T_BUTTON, T_BUTTON_PAD],
        T_KNOB7 | T_KNOB14 => &[T_KNOB7, T_KNOB14],
        T_ROTARY => &[T_ROTARY],
        T_INDICATOR => &[T_INDICATOR],
        T_PAD => &[T_PAD],
        T_VALUE => &[T_VALUE],
        _ => &[],
    }
}

/// The mapping-file type of a Learn type.
pub fn kind_of_type(t: u8) -> Kind {
    match t {
        T_ROTARY => Kind::Rotary,
        T_KNOB7 => Kind::KnobSlider,
        T_KNOB14 => Kind::KnobSliderHiRes,
        T_INDICATOR => Kind::Indicator,
        T_PAD => Kind::Pad(crate::midi_csv::PadType::ByDevice),
        T_VALUE => Kind::Value,
        _ => Kind::Button,
    }
}

/// The Learn type a mapping entry is shown as (`addMidiLearnTableData`).
pub fn type_of_entry(e: &Entry) -> u8 {
    match e.kind {
        Kind::Button => if e.opts.min == -1 && e.opts.max == -1 { T_BUTTON } else { T_BUTTON_PAD },
        Kind::KnobSlider => T_KNOB7,
        Kind::KnobSliderHiRes | Kind::Slider | Kind::Knob => T_KNOB14,
        Kind::Rotary => T_ROTARY,
        Kind::Value => T_VALUE,
        Kind::Indicator | Kind::IndicatorHiRes | Kind::JogIndicator | Kind::JogIndicatorHiRes => T_INDICATOR,
        Kind::Pad(_) => T_PAD,
        _ => T_BUTTON,
    }
}

/// What a row of a mapping-file type gives its function: a press / release, an absolute
/// position, relative steps, jog counts, or a device parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InClass {
    Press,
    Abs,
    Rel,
    Jog,
    Param,
}

/// The value class of an input row's type (None: output only).
pub fn in_class(k: Kind) -> Option<InClass> {
    Some(match k {
        Kind::Button | Kind::Toggle | Kind::Pad(_) | Kind::JogTouch => InClass::Press,
        Kind::KnobSlider | Kind::KnobSliderHiRes | Kind::Slider | Kind::Knob | Kind::Value | Kind::RawValue | Kind::RotaryAsKnob => InClass::Abs,
        Kind::Rotary => InClass::Rel,
        Kind::JogRotate | Kind::JogRotateAbsPos | Kind::Difference | Kind::JogTimestamp => InClass::Jog,
        Kind::Parameter => InClass::Param,
        Kind::Indicator | Kind::IndicatorHiRes | Kind::JogIndicator | Kind::JogIndicatorHiRes => return None,
    })
}

/// The device parameters (`Parameter` rows) recordbox uses: the output pacing and the jog
/// indicator's update interval. (`JogRatency`: rekordbox's files call it "unused", and it is
/// read but not used here either.)
pub const PARAMS: [&str; 2] = ["MidiOutInterval", "JogIndicatorInterval"];

/// Outputs that follow the engine and are sent by row name (`Hub::engine_outputs`).
pub const NAMED_OUTPUTS: [&str; 7] = ["JogPlayPauseIndicator", "JogPlayPauseIndicatorShort", "ChannelLevel", "LoadedIndicator", "GetLoopActive", "SlipStateForFlashing", "PlayingState"];

/// recordbox acts on an input row of this function and type.
pub fn input_acts(name: &str, kind: Kind) -> bool {
    if kind == Kind::JogTimestamp { return true; }
    if kind == Kind::Toggle { return false; }
    let Some(c) = in_class(kind) else { return false };
    if c == InClass::Param {
        return PARAMS.contains(&name);
    }
    registry().by_name(name).map(|f| act_accepts(f.act, c)).unwrap_or(false)
}

/// recordbox drives an output row of this function (a light, a meter, the jog ring).
pub fn output_source(name: &str) -> bool {
    NAMED_OUTPUTS.contains(&name) || registry().by_name(name).map(|f| act_has_led(f.act) || echo_led(f.act)).unwrap_or(false)
}

/// An explicitly mapped output row with a source in the running application.
pub fn output_acts(name: &str, kind: Kind) -> bool {
    // Imported maps may attach an output code to a knob or a rotary. Learn only
    // offers Lights for a subset of those types, but their mapped feedback works.
    !matches!(kind, Kind::Parameter | Kind::JogTimestamp) && output_source(name)
}

/// Which value classes an action does something with (`apply`, `Hub::fast`).
pub fn act_accepts(a: Act, c: InClass) -> bool {
    use Act as A;
    match a {
        A::SamplerVolume | A::FxUnitBeatRot(_) => matches!(c, InClass::Abs | InClass::Rel),
        A::Needle | A::Tempo | A::FxLevel(..) | A::CfxColor(_) | A::SamplerGain | A::Trim | A::Eq(_) | A::StemIsoKnob(_) | A::ChFader | A::HpMix | A::HpLevel | A::CrossFader | A::MasterLevel => c == InClass::Abs,
        A::LoopSize | A::FxReleaseType(_) | A::Browse | A::Zoom | A::LoopMoveRot => c == InClass::Rel,
        A::FxSelect(..) => matches!(c, InClass::Press | InClass::Rel),
        A::VinylAdjust(_) | A::XfAssignKnob => c == InClass::Abs,
        A::Jog(_) | A::JogSearch => c == InClass::Jog,
        A::TempoLed(_) => false,
        A::Stub(Stub::VinylBrake(_)) => c == InClass::Abs,
        _ => c == InClass::Press,
    }
}

/// Buttons whose light shows the button held (they have no state to show).
pub fn echo_led(a: Act) -> bool {
    act_accepts(a, InClass::Press) && !act_has_led(a) && !matches!(a, Act::Shift | Act::JogTouch | Act::Stub(Stub::Reverse))
}

/// The actions whose state `leds` sends to an output row.
pub fn act_has_led(a: Act) -> bool {
    use Act as A;
    use crate::sampler_ext::SmpAct;
    matches!(
        a,
        A::PlayPause | A::Cue | A::Search(_) | A::Slip | A::Quantize | A::QuantizeAll | A::MasterTempo | A::Sync | A::Master | A::KeySync | A::TempoLed(_) | A::AutoLoop | A::Loop4 | A::Reloop | A::LoopIn | A::LoopOut | A::PitchBend(_) | A::PadMode(_) | A::HotCue(_) | A::HotCueDelete(_) | A::BeatJumpPad(_) | A::BeatLoopPad(_) | A::StemPart(_) | A::StemFx(_) | A::StemSolo | A::FxOn(..) | A::FxMode(_) | A::FxAuto(_) | A::FxTap(_) | A::FxReleaseOn(_) | A::FxAssign(..) | A::SamplerPad(..) | A::SamplerPadStop(_) | A::Smp(SmpAct::Sync | SmpAct::Master | SmpAct::Quantize | SmpAct::SeqPlay | SmpAct::SeqRec) | A::SamplerCue | A::SeqCall(..) | A::HeadCue | A::XfAssign(_) | A::StemIso | A::Panel(_) | A::Related | A::Stub(Stub::Vinyl) | A::Stub(Stub::Reverse)
    )
}

/// Features other parts of recordbox are building: the registry has their commands, and
/// `stub` is the one place to connect them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stub {
    RecPanel,
    AutoMixStartStop,
    /// MIX POINT LINK: 0 panel, 1 cue select next, 2 back, 3 set, 4 exit, 5 deck select, 6 silent.
    MixPointLink(u8),
    /// PAD FX: banks 1 and 2, or the controller's combination bank (3).
    PadFxMode(u8),
    PadFx(u8, u8),
    PadFxBeat(u8, i32),
    /// Slicer: loop variant.
    SlicerMode(bool),
    SlicerPad(bool, u8),
    SlicerShift(bool, i32),
    SlicerRoll(bool, i32),
    SlicerLength(bool, i32),
    SlicerCapture,
    Reverse,
    SlipReverse,
    /// Vinyl mode on / off (the jog's own vinyl switch follows it here already).
    Vinyl,
    VinylBrake(u8),
    AutoCue,
    LoopMove(i32),
    ActiveLoop,
    SyncRate,
}

/// What a function does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    PlayPause,
    Cue,
    TrackStart,
    Track(i32),
    Search(i32),
    Needle,
    Slip,
    Quantize,
    QuantizeAll,
    MasterTempo,
    Tempo,
    TempoRange,
    /// The takeover direction LEDs: -1 TempoDown, +1 TempoUp.
    TempoLed(i32),
    TempoReset,
    Sync,
    Master,
    TapBpm,
    PitchBend(i32),
    AutoLoop,
    Loop4,
    LoopResize(f64),
    LoopSize,
    LoopIn,
    LoopOut,
    Reloop,
    ReTrigger,
    BeatLoop(f64),
    GridHalfDouble(bool),
    KeyShift(i32),
    KeySync,
    KeyReset,
    StemSolo,
    PadMode(u8),
    HotCue(u8),
    HotCueDelete(u8),
    MemoryStep(i32),
    MemorySet,
    MemoryDelete,
    BeatJumpPad(u8),
    BeatJumpPage(i32),
    BeatLoopPad(u8),
    StemPart(u8),
    StemFx(u8),
    FxLevel(u8, u8),
    FxOn(u8, u8),
    FxSelect(u8, u8, i32),
    FxBeat(u8, u8, i32),
    FxMode(u8),
    FxUnitBeat(u8, i32),
    FxUnitBeatRot(u8),
    FxAuto(u8),
    FxTap(u8),
    FxReleaseType(u8),
    FxReleaseBeat(u8, i32),
    FxReleaseOn(u8),
    FxAssign(u8, u8),
    CfxColor(u8),
    CfxSelect(i32),
    SamplerPad(u8, bool),
    SamplerPadStop(u8),
    SamplerBank(i32),
    SamplerGain,
    SamplerVolume,
    Smp(crate::sampler_ext::SmpAct),
    SamplerCue,
    SeqCall(u8, bool),
    Trim,
    Eq(u8),
    HeadCue,
    ChFader,
    HpMix,
    HpLevel,
    CrossFader,
    XfAssign(u8),
    MasterLevel,
    StemIso,
    StemIsoKnob(u8),
    Browse,
    BrowseStep(i32),
    Forward,
    Back,
    TagList,
    Related,
    Area,
    Load,
    InstDoubles,
    Sort(Col),
    Panel(u8),
    Zoom,
    /// Jog rows: true = the platter top (scratch), false = the outer ring (bend).
    Jog(bool),
    JogSearch,
    JogTouch,
    Shift,
    /// Loop Move as an encoder: one move right or left per message (`deviceLoop(int)`).
    LoopMoveRot,
    /// The top bar's panels in turn: FX, SAMPLER, MIXER, REC, none.
    PanelStep(i32),
    /// The Browse layout and back (`MainComponent::switchActiveWindow`).
    BrowseView,
    CallStep(i32),
    /// Vinyl Speed Adjust: 0 brake, 1 start, 2 both.
    VinylAdjust(u8),
    FxTapLong(u8),
    FxAssignHold(u8, u8),
    XfAssignKnob,
    CfxCenter(u8),
    /// An FX slot's effect by its place in `fx::INFO`.
    FxSelectKind(u8, u8, u8),
    Stub(Stub),
}

/// One function recordbox can be mapped to.
pub struct Func {
    /// rekordbox's `Name` of its mapping files.
    pub name: String,
    /// rekordbox's name for it in the MIDI setting window.
    pub display: String,
    pub tab: Tab,
    /// The ADD menu's submenu ("" = none).
    pub category: &'static str,
    pub ty: u8,
    pub per_deck: bool,
    pub act: Act,
}

pub struct Registry {
    pub funcs: Vec<Func>,
    by_name: HashMap<String, u32>,
}

impl Registry {
    /// The tag of a name (index + 1), 0 when recordbox has no such function.
    pub fn tag(&self, name: &str) -> u32 {
        self.by_name.get(name).copied().unwrap_or(0)
    }
    pub fn get(&self, tag: u32) -> Option<&Func> {
        self.funcs.get((tag as usize).wrapping_sub(1))
    }
    pub fn by_name(&self, name: &str) -> Option<&Func> {
        self.get(self.tag(name))
    }
    /// The name is one of the MIDI setting window's commands.
    pub fn learnable(&self, name: &str) -> bool {
        self.by_name(name).map(|f| f.tab != Tab::None).unwrap_or(false)
    }
    /// The commands of a tab in ADD-menu order.
    pub fn of_tab(&self, tab: Tab) -> impl Iterator<Item = &Func> {
        self.funcs.iter().filter(move |f| f.tab == tab)
    }
}

pub fn registry() -> &'static Registry {
    static R: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        let funcs = build_registry();
        let mut by_name = HashMap::new();
        for (i, f) in funcs.iter().enumerate() {
            by_name.entry(f.name.clone()).or_insert(i as u32 + 1);
        }
        Registry { funcs, by_name }
    })
}

fn build_registry() -> Vec<Func> {
    use crate::sampler_ext::SmpAct;
    use Act as A;
    let mut v: Vec<Func> = vec![];
    let mut add = |tab: Tab, category: &'static str, display: &str, name: &str, ty: u8, per_deck: bool, act: Act| v.push(Func { name: name.to_string(), display: display.to_string(), tab, category, ty, per_deck, act });
    let (b, rot, k7, ind, pad, val) = (T_BUTTON, T_ROTARY, T_KNOB7, T_INDICATOR, T_PAD, T_VALUE);

    // ---- DECK
    let t = Tab::Deck;
    add(t, "", "PlayPause", "PlayPause", b, true, A::PlayPause);
    add(t, "", "Cue", "Cue", b, true, A::Cue);
    add(t, "", "JumpToTrackStart", "JumpToTrackStart", b, true, A::TrackStart);
    add(t, "", "TrackSearch Next", "TrackNext", b, true, A::Track(1));
    add(t, "", "TrackSearch Back", "TrackPrev", b, true, A::Track(-1));
    add(t, "", "Search Forward", "SearchFwd", b, true, A::Search(1));
    add(t, "", "Search Reverse", "SearchRev", b, true, A::Search(-1));
    add(t, "", "NeedleSearch", "NeedleSearch", val, true, A::Needle);
    add(t, "", "Slip", "Slip", b, true, A::Slip);
    add(t, "", "Quantize", "Quantize", b, true, A::Quantize);
    add(t, "", "Reverse", "Reverse", b, true, A::Stub(Stub::Reverse));
    add(t, "", "SlipReverse", "SlipReverse", b, true, A::Stub(Stub::SlipReverse));
    add(t, "", "Vinyl", "Vinyl", b, true, A::Stub(Stub::Vinyl));
    add(t, "", "VinylBrakeSpeed LeftDeck", "VinylAdjustBrake.Left", k7, false, A::Stub(Stub::VinylBrake(0)));
    add(t, "", "VinylBrakeSpeed RightDeck", "VinylAdjustBrake.Right", k7, false, A::Stub(Stub::VinylBrake(1)));
    add(t, "", "Quantize All Decks", "Quantize4", b, false, A::QuantizeAll);
    add(t, "", "AutoCue", "AutoCue", b, true, A::Stub(Stub::AutoCue));
    add(t, "Tempo", "MasterTempo", "MasterTempo", b, true, A::MasterTempo);
    add(t, "Tempo", "Tempo", "TempoSlider", k7, true, A::Tempo);
    add(t, "Tempo", "TempoRange", "TempoRange", b, true, A::TempoRange);
    add(t, "Tempo", "TempoTakeOverLED Down", "TempoDown", ind, true, A::TempoLed(-1));
    add(t, "Tempo", "TempoTakeOverLED Up", "TempoUp", ind, true, A::TempoLed(1));
    add(t, "Tempo", "TempoReset", "TempoReset", b, true, A::TempoReset);
    add(t, "Tempo", "Sync", "Sync", b, true, A::Sync);
    add(t, "Tempo", "SyncRate", "SyncRate", b, true, A::Stub(Stub::SyncRate));
    add(t, "Tempo", "Master", "Master", b, true, A::Master);
    add(t, "Tempo", "TapBpm", "TapBpm", b, true, A::TapBpm);
    add(t, "Tempo", "PitchBend Up", "PitchBendUp", b, true, A::PitchBend(1));
    add(t, "Tempo", "PitchBend Down", "PitchBendDown", b, true, A::PitchBend(-1));
    add(t, "Loop", "AutoLoop", "AutoLoop", b, true, A::AutoLoop);
    add(t, "Loop", "4BeatLoop", "4BeatLoop", b, true, A::Loop4);
    add(t, "Loop", "ActiveLoop", "ActiveLoop", b, true, A::Stub(Stub::ActiveLoop));
    add(t, "Loop", "Loop Half", "LoopHalf", b, true, A::LoopResize(0.5));
    add(t, "Loop", "Loop Double", "LoopDouble", b, true, A::LoopResize(2.0));
    add(t, "Loop", "Loop Size", "LoopHalfDouble", rot, true, A::LoopSize);
    add(t, "Loop", "LoopMove Right", "LoopMoveRight", b, true, A::Stub(Stub::LoopMove(1)));
    add(t, "Loop", "LoopMove Left", "LoopMoveLeft", b, true, A::Stub(Stub::LoopMove(-1)));
    add(t, "Loop", "Loop Move", "LoopMove", rot, true, A::LoopMoveRot);
    add(t, "Loop", "Loop In", "LoopIn", b, true, A::LoopIn);
    add(t, "Loop", "Loop ReTrigger", "ReTrigger", b, true, A::ReTrigger);
    add(t, "Loop", "Loop Out", "LoopOut", b, true, A::LoopOut);
    add(t, "Loop", "ReloopExit", "ReloopExit", b, true, A::Reloop);
    for (i, beats) in crate::player_ext::LOOP_SIZES.iter().enumerate() {
        let label = ["1/64", "1/32", "1/16", "1/8", "1/4", "1/2", "1", "2", "4", "8", "16", "32", "64", "128", "256", "512"][i];
        add(t, "Loop", &format!("AutoBeatLoop {label}"), &format!("BeatLoop{}", label.replace('/', "_")), b, true, A::BeatLoop(*beats));
    }
    add(t, "Grid", "GridAdjust Half", "GridAdjustHalf", b, true, A::GridHalfDouble(false));
    add(t, "Grid", "GridAdjust Double", "GridAdjustDouble", b, true, A::GridHalfDouble(true));
    add(t, "Capture", "SlicerCapture", "SlicerCapture", b, true, A::Stub(Stub::SlicerCapture));
    add(t, "KeyShift", "Semitone Up", "SemitoneUp", b, true, A::KeyShift(1));
    add(t, "KeyShift", "Semitone Down", "SemitoneDown", b, true, A::KeyShift(-1));
    add(t, "KeyShift", "KeySync", "KeySync", b, true, A::KeySync);
    add(t, "KeyShift", "KeyReset", "KeyReset", b, true, A::KeyReset);
    add(t, "STEMS", "ActiveStem Mute/Solo", "ActivePartMuteSolo", b, false, A::StemSolo);

    // ---- PAD
    let t = Tab::Pad;
    add(t, "HotCue", "HotCueMode", "HotCue", b, true, A::PadMode(0));
    for n in 1..=16u8 {
        add(t, "HotCue", &format!("HotCue{n}"), &format!("PAD{n}_HotCue"), pad, true, A::HotCue(n - 1));
    }
    for n in 1..=16u8 {
        add(t, "HotCue", &format!("HotCue{n} Delete"), &format!("PAD{n}_HotCue+Shift"), pad, true, A::HotCueDelete(n - 1));
    }
    add(t, "MemoryCue", "MemoryCueCall Next", "Parameter1Right_HotCue", b, true, A::MemoryStep(1));
    add(t, "MemoryCue", "MemoryCueCall Back", "Parameter1Left_HotCue", b, true, A::MemoryStep(-1));
    add(t, "MemoryCue", "MemoryCue Set", "Memory", b, true, A::MemorySet);
    add(t, "MemoryCue", "MemoryCue Delete", "Delete", b, true, A::MemoryDelete);
    for bank in 1..=2u8 {
        add(t, "PadFX", &format!("PadFX{bank}Mode"), &format!("PadFx{bank}"), b, true, A::Stub(Stub::PadFxMode(bank)));
        for n in 1..=16u8 {
            add(t, "PadFX", &format!("PadFX{bank} Pad{n}"), &format!("PAD{n}_PadFx{bank}"), pad, true, A::Stub(Stub::PadFx(bank, n - 1)));
        }
        add(t, "PadFX", &format!("PadFX{bank}Beat Up"), &format!("Parameter1Right_PadFx{bank}"), b, true, A::Stub(Stub::PadFxBeat(bank, 1)));
        add(t, "PadFX", &format!("PadFX{bank}Beat Down"), &format!("Parameter1Left_PadFx{bank}"), b, true, A::Stub(Stub::PadFxBeat(bank, -1)));
    }
    for n in 1..=16u8 {
        add(t, "PadFX", &format!("PadFXCombination Pad{n}"), &format!("PAD{n}_PadFxCombination"), pad, true, A::Stub(Stub::PadFx(3, n - 1)));
    }
    for (lp, mode) in [(false, "Slicer"), (true, "SlicerLoop")] {
        add(t, "Slicer", &format!("{mode}Mode"), mode, b, true, A::Stub(Stub::SlicerMode(lp)));
        for n in 1..=8u8 {
            add(t, "Slicer", &format!("{mode} Pad{n}"), &format!("PAD{n}_{mode}"), pad, true, A::Stub(Stub::SlicerPad(lp, n - 1)));
        }
    }
    for (lp, mode) in [(false, "Slicer"), (true, "SlicerLoop")] {
        add(t, "Slicer", &format!("{mode}Shift Left"), &format!("PAD5_{mode}+Shift"), pad, true, A::Stub(Stub::SlicerShift(lp, -1)));
        add(t, "Slicer", &format!("{mode}Shift Right"), &format!("PAD6_{mode}+Shift"), pad, true, A::Stub(Stub::SlicerShift(lp, 1)));
    }
    for (lp, mode) in [(false, "Slicer"), (true, "SlicerLoop")] {
        add(t, "Slicer", &format!("{mode}Roll Up"), &format!("Parameter1Right_{mode}"), b, true, A::Stub(Stub::SlicerRoll(lp, 1)));
        add(t, "Slicer", &format!("{mode}Roll Down"), &format!("Parameter1Left_{mode}"), b, true, A::Stub(Stub::SlicerRoll(lp, -1)));
    }
    for (lp, mode) in [(false, "Slicer"), (true, "SlicerLoop")] {
        add(t, "Slicer", &format!("{mode}Length Up"), &format!("Parameter2Right_{mode}"), b, true, A::Stub(Stub::SlicerLength(lp, 1)));
        add(t, "Slicer", &format!("{mode}Length Down"), &format!("Parameter2Left_{mode}"), b, true, A::Stub(Stub::SlicerLength(lp, -1)));
    }
    add(t, "BeatJump", "BeatJumpMode", "BeatJump", b, true, A::PadMode(2));
    for n in 1..=16u8 {
        add(t, "BeatJump", &format!("BeatJump Pad{n}"), &format!("PAD{n}_BeatJump"), pad, true, A::BeatJumpPad(n - 1));
    }
    add(t, "BeatJump", "BeatJump Page Right", "Parameter1Right_BeatJump", b, true, A::BeatJumpPage(1));
    add(t, "BeatJump", "BeatJump Page Left", "Parameter1Left_BeatJump", b, true, A::BeatJumpPage(-1));
    add(t, "BeatLoop", "BeatLoopMode", "BeatLoop", b, true, A::PadMode(3));
    for n in 1..=16u8 {
        add(t, "BeatLoop", &format!("BeatLoop Pad{n}"), &format!("PAD{n}_BeatLoop"), pad, true, A::BeatLoopPad(n - 1));
    }
    add(t, "STEMS", "StemsMode", "Stems", b, true, A::PadMode(1));
    // (Sources in the engine's order: 0 drums, 1 bass, 2 other, 3 vocals; FX rows as on screen.)
    add(t, "STEMS", "ActiveStem Inst", "ActivePartInst", pad, true, A::StemPart(2));
    add(t, "STEMS", "ActiveStem Bass", "ActivePartBass", pad, true, A::StemPart(1));
    add(t, "STEMS", "ActiveStem Vocal", "ActivePartVocal", pad, true, A::StemPart(3));
    add(t, "STEMS", "ActiveStem Drums", "ActivePartDrums", pad, true, A::StemPart(0));
    add(t, "STEMS", "ActiveStem FX Inst", "FXPartSelectInstOn", pad, false, A::StemFx(1));
    add(t, "STEMS", "ActiveStem FX Bass", "FXPartSelectBassOn", pad, false, A::StemFx(2));
    add(t, "STEMS", "ActiveStem FX Vocal", "FXPartSelectVocalOn", pad, false, A::StemFx(0));
    add(t, "STEMS", "ActiveStem FX Drums", "FXPartSelectDrumsOn", pad, false, A::StemFx(3));

    // ---- FX
    let t = Tab::Fx;
    for u in 0..2u8 {
        let cat: &'static str = if u == 0 { "FX1" } else { "FX2" };
        for s in 0..3u8 {
            let p = format!("{cat}-{}", s + 1);
            add(t, cat, &format!("{p} LevelDepth"), &p, k7, false, A::FxLevel(u, s));
            add(t, cat, &format!("{p} On"), &format!("{p}On"), b, false, A::FxOn(u, s));
            add(t, cat, &format!("{p} Select Next"), &format!("{p}Select"), b, false, A::FxSelect(u, s, 1));
            add(t, cat, &format!("{p} Select Back"), &format!("{p}SelectBack"), b, false, A::FxSelect(u, s, -1));
            add(t, cat, &format!("{p} Beat Up"), &format!("{p}BeatUp"), b, false, A::FxBeat(u, s, 1));
            add(t, cat, &format!("{p} Beat Down"), &format!("{p}BeatDown"), b, false, A::FxBeat(u, s, -1));
        }
        add(t, cat, &format!("{cat} Multi/Single Toggle"), &format!("{cat}FXMode"), b, false, A::FxMode(u));
        add(t, cat, &format!("{cat} Beat Up"), &format!("{cat}BeatUp"), b, false, A::FxUnitBeat(u, 1));
        add(t, cat, &format!("{cat} Beat Down"), &format!("{cat}BeatDown"), b, false, A::FxUnitBeat(u, -1));
        add(t, cat, &format!("{cat} BeatSize"), &format!("{cat}BeatUpDown"), rot, false, A::FxUnitBeatRot(u));
        add(t, cat, &format!("{cat} BPM AutoMode"), &format!("{cat}AutoBeat"), b, false, A::FxAuto(u));
        add(t, cat, &format!("{cat} BPM TapMode/TapBpm"), &format!("{cat}TapBeat"), b, false, A::FxTap(u));
        add(t, cat, &format!("{cat} ReleaseFX Select"), &format!("{cat}ReleaseFXType"), rot, false, A::FxReleaseType(u));
        add(t, cat, &format!("{cat} ReleaseFX Beat Up"), &format!("{cat}ReleaseFXBeatUp"), b, false, A::FxReleaseBeat(u, 1));
        add(t, cat, &format!("{cat} ReleaseFX Beat Down"), &format!("{cat}ReleaseFXBeatDown"), b, false, A::FxReleaseBeat(u, -1));
        add(t, cat, &format!("{cat} ReleaseFX On"), &format!("{cat}ReleaseFXOn"), b, false, A::FxReleaseOn(u));
        for d in 0..4u8 {
            add(t, cat, &format!("{cat} AssignDeck{}", d + 1), &format!("{cat}Assign{}", d + 1), b, false, A::FxAssign(u, d));
        }
    }
    for (i, ch) in ["CH1", "CH2", "CH3", "CH4", "Master", "Sampler"].iter().enumerate() {
        add(t, "CFX", &format!("CFX Parameter{ch}"), &format!("CFXParameter{ch}"), k7, false, A::CfxColor(i as u8));
    }
    add(t, "CFX", "CFX Select Next", "CFXSelect", b, false, A::CfxSelect(1));
    add(t, "CFX", "CFX Select Back", "CFXSelectBack", b, false, A::CfxSelect(-1));

    // ---- SAMPLER
    let t = Tab::Sampler;
    add(t, "Sampler", "SamplerMode", "Sampler", b, true, A::PadMode(crate::ui_perf::PAD_SAMPLER));
    for n in 1..=8u8 {
        add(t, "Sampler", &format!("Sampler{n} Play"), &format!("PAD{n}_Sampler"), pad, true, A::SamplerPad(n - 1, false));
    }
    for n in 1..=8u8 {
        add(t, "Sampler", &format!("Sampler{n} Stop"), &format!("PAD{n}_Sampler+Shift"), pad, true, A::SamplerPadStop(n - 1));
    }
    add(t, "VelocitySampler", "VelocitySamplerMode", "VelocitySampler", b, true, A::PadMode(crate::ui_perf::PAD_SAMPLER));
    for n in 1..=8u8 {
        add(t, "VelocitySampler", &format!("VelocitySampler{n} Play"), &format!("PAD{n}_VelocitySampler"), pad, true, A::SamplerPad(n - 1, true));
    }
    for n in 1..=8u8 {
        add(t, "VelocitySampler", &format!("VelocitySampler{n} Stop"), &format!("PAD{n}_VelocitySampler+Shift"), pad, true, A::SamplerPadStop(n - 1));
    }
    add(t, "Bank", "SamplerBank Up/SeqEraseMode", "Parameter1Right_Sampler", b, true, A::SamplerBank(1));
    add(t, "Bank", "SamplerBank Down/SeqMuteMode", "Parameter1Left_Sampler", b, true, A::SamplerBank(-1));
    add(t, "", "SamplerGain", "SamplerGain", k7, false, A::SamplerGain);
    add(t, "", "SamplerVolume", "SamplerVolume", k7, false, A::SamplerVolume);
    add(t, "", "SamplerSync", "SamplerSync", b, false, A::Smp(SmpAct::Sync));
    add(t, "", "SamplerMaster", "SamplerMaster", b, false, A::Smp(SmpAct::Master));
    add(t, "", "SamplerCue", "SamplerCue", b, false, A::SamplerCue);
    add(t, "", "SamplerQuantize", "SamplerQuantize", b, false, A::Smp(SmpAct::Quantize));
    add(t, "Sequence", "SequencerRec", "SequencerRec", b, true, A::Smp(SmpAct::SeqRec));
    add(t, "Sequence", "SequencerStart", "SequencerStart", b, true, A::Smp(SmpAct::SeqPlay));
    add(t, "Sequence", "SequencerSave", "SaveSequence", b, true, A::Smp(SmpAct::SeqSave));
    add(t, "Sequence", "SequencerSlotSelect Next", "SelectSlot", b, true, A::Smp(SmpAct::SeqStep(1)));
    add(t, "Sequence", "SequenceLoad", "SequenceLoad", b, true, A::Smp(SmpAct::SeqLoad(255)));
    add(t, "Sequence", "SequenceCallMode", "SequenceCall", b, true, A::PadMode(crate::ui_perf::PAD_SEQ_CALL));
    for n in 1..=8u8 {
        add(t, "Sequence", &format!("SequenceCall{n} Play"), &format!("PAD{n}_SequenceCall"), pad, true, A::SeqCall(n - 1, false));
    }
    for n in 1..=8u8 {
        add(t, "Sequence", &format!("SequenceCall{n} Stop"), &format!("PAD{n}_SequenceCall+Shift"), pad, true, A::SeqCall(n - 1, true));
    }

    // ---- MIXER
    let t = Tab::Mixer;
    add(t, "", "Gain", "Gain", k7, true, A::Trim);
    add(t, "", "EQ High", "EQHigh", k7, true, A::Eq(2));
    add(t, "", "EQ Mid", "EQMid", k7, true, A::Eq(1));
    add(t, "", "EQ Low", "EQLow", k7, true, A::Eq(0));
    add(t, "", "HeadphoneCue", "HeadphoneCue", b, true, A::HeadCue);
    add(t, "", "ChannelFader", "ChannelFader", k7, true, A::ChFader);
    add(t, "", "HeadphonesMix", "HeadphonesMix", k7, false, A::HpMix);
    add(t, "", "HeadphonesVolume", "HeadphonesVolume", k7, false, A::HpLevel);
    add(t, "", "CrossFader", "CrossFader", k7, false, A::CrossFader);
    add(t, "", "CrossfaderAssignA", "CrossfaderAssign.A", b, true, A::XfAssign(1));
    add(t, "", "CrossfaderAssignB", "CrossfaderAssign.B", b, true, A::XfAssign(2));
    add(t, "", "CrossfaderAssignThru", "CrossfaderAssign.Thru", b, true, A::XfAssign(0));
    add(t, "", "MasterLevel", "MasterLevel", k7, false, A::MasterLevel);
    add(t, "STEMS", "StemISO Mode", "PartIsolator", b, true, A::StemIso);
    add(t, "STEMS", "StemISO Inst/Bass", "PartIsolatorInst", k7, true, A::StemIsoKnob(1));
    add(t, "STEMS", "StemISO Vocal", "PartIsolatorVocal", k7, true, A::StemIsoKnob(2));
    add(t, "STEMS", "StemISO Drums", "PartIsolatorDrums", k7, true, A::StemIsoKnob(0));

    // ---- BROWSE
    let t = Tab::Browse;
    add(t, "", "Browse", "Browse", rot, false, A::Browse);
    add(t, "", "Browse Up", "BrowseUp", b, false, A::BrowseStep(-1));
    add(t, "", "Browse Down", "BrowseDown", b, false, A::BrowseStep(1));
    add(t, "", "BrowseForward", "Forward", b, false, A::Forward);
    add(t, "", "BrowseBack", "Back", b, false, A::Back);
    add(t, "", "Browse Jump Up", "BrowseJumpUp", b, false, A::BrowseStep(-10));
    add(t, "", "Browse Jump Down", "BrowseJumpDown", b, false, A::BrowseStep(10));
    add(t, "", "AddToTagList", "AddToTagList", b, false, A::TagList);
    add(t, "", "RelatedTracks", "RelatedTracks", b, false, A::Related);
    add(t, "", "Area", "Area", b, false, A::Area);
    add(t, "", "Area Right", "AreaRight", b, false, A::Forward);
    add(t, "", "Area Left", "AreaLeft", b, false, A::Back);
    add(t, "", "Load", "Load", b, true, A::Load);
    add(t, "", "InstantDoubles", "InstDoubles", b, true, A::InstDoubles);
    add(t, "", "SortBPM", "SortBPM", b, false, A::Sort(Col::Bpm));
    add(t, "", "SortSONG", "SortSONG", b, false, A::Sort(Col::Title));
    add(t, "", "SortTRACK", "SortTRACK", b, false, A::Sort(Col::TrackNumber));
    add(t, "", "SortARTIST", "SortARTIST", b, false, A::Sort(Col::Artist));
    add(t, "", "SortKEY", "SortKEY", b, false, A::Sort(Col::Key));

    // ---- OTHER
    let t = Tab::Other;
    add(t, "", "FXPanel", "FxPanel", b, false, A::Panel(0));
    add(t, "", "SamplerPanel", "SamplerPanel", b, false, A::Panel(1));
    add(t, "", "RecPanel", "RecPanel", b, false, A::Stub(Stub::RecPanel));
    add(t, "", "WaveZoom", "Zoom", rot, false, A::Zoom);
    add(t, "", "AutoMix StartStop", "AutoMixStartStop", b, false, A::Stub(Stub::AutoMixStartStop));
    add(t, "", "PanelSelect Next", "PanelForward", b, false, A::PanelStep(1));
    add(t, "", "PanelSelect Back", "PanelBack", b, false, A::PanelStep(-1));
    add(t, "", "BrowseView", "SwitchActiveWindow", b, false, A::BrowseView);

    // ---- MIXPOINTLINK
    let t = Tab::MixPointLink;
    for (i, (d, n, pd)) in [("MixPointLinkPanel", "MixPointLinkPanel", false), ("MixPointLink CueSelect Next", "MixPointSelectNext", true), ("MixPointLink CueSelect Back", "MixPointSelectPrev", true), ("MixPointLink Set", "MixPointLink", true), ("MixPointLink Exit", "MixPointLinkExit", true), ("MixPointLink Deck Select", "MixPointLinkDeck", true), ("MixPointLink Silent", "MixPointLinkSilent", true)].iter().enumerate() {
        add(t, "", d, n, b, *pd, A::Stub(Stub::MixPointLink(i as u8)));
    }

    // ---- factory mappings only: MIDI Learn does not offer these
    let t = Tab::None;
    add(t, "", "", "JogScratch", b, true, A::Jog(true));
    add(t, "", "", "JogPitchBend", b, true, A::Jog(false));
    add(t, "", "", "WheelPitchBend", b, true, A::Jog(false));
    add(t, "", "", "JogSearch", b, true, A::JogSearch);
    add(t, "", "", "WheelSearch", b, true, A::JogSearch);
    add(t, "", "", "JogTouch", b, true, A::JogTouch);
    add(t, "", "", "Shift", b, true, A::Shift);
    // Hardware can report a long press of LOOP IN as its own 4-beat loop code.
    add(t, "", "", "4BeatLoopLongPress", b, true, A::Loop4);
    // Crossfader assign as a knob: A at 0x00, THRU at 0x40, B at 0x7F.
    add(t, "", "", "CrossfaderAssign", k7, true, A::XfAssignKnob);
    for ch in 0..4u8 {
        // (The CFX knob's centre click: the knob back to the middle.)
        add(t, "", "", &format!("CFXParameterCH{}Center", ch + 1), b, false, A::CfxCenter(ch));
    }
    // CDJ-style CALL buttons step through memory cues or resize the running loop.
    add(t, "", "", "CallNext", b, true, A::CallStep(1));
    add(t, "", "", "CallPrev", b, true, A::CallStep(-1));
    // (Vinyl Speed Adjust knobs of a deck: brake, start, or both. `deviceVinylAdjust`.)
    add(t, "", "", "VinylAdjustBrake", k7, true, A::VinylAdjust(0));
    add(t, "", "", "VinylAdjustStart", k7, true, A::VinylAdjust(1));
    add(t, "", "", "VinylAdjustTouchRelease", k7, true, A::VinylAdjust(2));
    for u in 0..2u8 {
        let cat = if u == 0 { "FX1" } else { "FX2" };
        // TAP pressed: a tap; held for a second: back to AUTO (`RbxBpmPanelDeviceComponent`).
        add(t, "", "", &format!("{cat}TapBeatWithLongPress"), b, false, A::FxTapLong(u));
        // A channel switch's positions: the assign follows the switch (on while its code is
        // held, off when released; `RbxFxDeviceComponent::eventOnChanged` $_1 with "set").
        for (i, ch) in ["CH1", "CH2", "CH3", "CH4"].iter().enumerate() {
            add(t, "", "", &format!("{cat}Assign.{ch}"), b, false, A::FxAssignHold(u, i as u8));
        }
        add(t, "", "", &format!("{cat}Assign.SAMPLER"), b, false, A::FxAssignHold(u, crate::fx::SAMPLER as u8));
        // A slot's effect chosen by name (`FX1-1Select.ECHO`): rekordbox's effect names in
        // capitals without spaces or brackets.
        for sl in 0..3u8 {
            for (i, info) in crate::fx::INFO.iter().enumerate() {
                let key: String = info.name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
                add(t, "", "", &format!("{cat}-{}Select.{key}", sl + 1), b, false, A::FxSelectKind(u, sl, i as u8));
            }
        }
        add(t, "", "", &format!("{cat}Assign.MASTER"), b, false, A::FxAssignHold(u, crate::fx::MASTER as u8));
    }
    v
}

// ---------------------------------------------------------------------- shared state

/// A light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Led {
    Off,
    On,
    /// Blinking in the row's `Blink=` cycle (500 ms without one).
    Blink,
    Value(u8),
}

/// What the workers need to know about a deck, refreshed by the interface every frame.
#[derive(Clone, Copy, Debug)]
pub struct DeckSnap {
    pub loaded: bool,
    pub cue_point: f64,
    /// Hot cues A to P: start and, for a loop, its end (seconds).
    pub hot: [Option<(f64, Option<f64>)>; 16],
    /// The tempo range as a fraction (0.10 = +-10 %).
    pub range: f64,
    pub follower: bool,
    /// Bend of a held pitch bend button, as the engine has it.
    pub button_bend: f64,
    pub stem_iso_on: bool,
    /// Crossfader side: 0 THRU, 1 A, 2 B.
    pub assign: u8,
    /// The on-screen position of the knob a pick-up compares with (LOW, MID, HIGH).
    pub eq: [f32; 3],
    pub stem_iso: [f32; 3],
}

impl Default for DeckSnap {
    fn default() -> DeckSnap {
        DeckSnap { loaded: false, cue_point: 0.0, hot: [None; 16], range: 0.10, follower: false, button_bend: 0.0, stem_iso_on: false, assign: 0, eq: [0.5; 3], stem_iso: [0.5; 3] }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub perf: bool,
    pub count: u8,
    pub active: u8,
    pub decks: [DeckSnap; 4],
    pub curve: u8,
    /// The sampler: the bank, how many pads it has, which are loaded, and whether pads play
    /// (the sequencer's MUTE / ERASE modes give them another meaning).
    pub bank: u8,
    pub smp_pads: u8,
    pub smp_loaded: u16,
    pub smp_plain: bool,
    pub velocity_curve: u8,
    pub takeover_eq_iso: bool,
    pub rpm45: bool,
}

impl Default for Snapshot {
    fn default() -> Snapshot {
        Snapshot { perf: true, count: 2, active: 0, decks: [DeckSnap::default(); 4], curve: 1, bank: 0, smp_pads: 16, smp_loaded: 0, smp_plain: true, velocity_curve: 3, takeover_eq_iso: false, rpm45: false }
    }
}

/// An engine value a worker set ahead of the interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Param {
    ChFader,
    Trim,
    Eq(u8),
    CrossFader,
    Tempo,
}

/// What reaches the interface thread.
pub enum UiEvent {
    Control { device: String, tag: u32, deck: u8, val: Val, fast_done: bool, seq: u64 },
    /// A message seen while LEARN is armed.
    Learn { device: std::sync::Weak<Device>, msg: [u8; 3] },
    /// A device came or went.
    Ports { removed: Vec<String>, paused: Vec<String> },
}

#[derive(Default)]
struct NamedRows {
    entries: HashMap<(&'static str, u8), Vec<Entry>>,
    last: HashMap<u16, u16>,
    jog_interval: Duration,
    last_ring: [Option<Instant>; 5],
}

impl NamedRows {
    fn due(&mut self, deck: u8, now: Instant) -> bool {
        let last = &mut self.last_ring[deck as usize];
        if last.is_some_and(|seen| now.saturating_duration_since(seen) < self.jog_interval) {
            return false;
        }
        *last = Some(now);
        true
    }
}

struct OutState {
    queue: OutQueue,
    blink: Blinker,
    /// Blinking codes: the entry to light and its priority.
    blinking: HashMap<u16, (Entry, i32)>,
    conn: Option<midir::MidiOutputConnection>,
}

impl OutState {
    fn feedback(&mut self, entry: &Entry, value: u16) {
        if let Some((first, second)) = crate::midi_map::named_messages(entry, value) {
            self.queue.push(first, entry.opts.priority);
            if let Some(low) = second {
                self.queue.push(low, entry.opts.priority);
            }
        }
    }

    fn led(&mut self, map: &MidiMap, entry: &Entry, value: u8) {
        let value = if entry.kind == Kind::IndicatorHiRes {
            u16::from(value) * 129
        } else {
            u16::from(map.led_message(entry, value)[2])
        };
        self.feedback(entry, value);
    }
}

/// One connected controller.
pub struct Device {
    pub name: String,
    input_id: String,
    pub map: Mutex<MidiMap>,
    /// The rekordbox installed here has a factory mapping for it.
    pub factory: bool,
    native_mapping: AtomicBool,
    /// rekordbox runs a hardware handshake with this model before it treats it as started.
    /// recordbox does not have one: the device is listed and its messages are read, but it
    /// is not supported and may not send anything.
    pub needs_auth: bool,
    pub has_output: bool,
    pub learning: AtomicBool,
    input_epoch: Arc<AtomicU64>,
    pub alive: AtomicBool,
    out_index: Mutex<HashMap<(u32, u8), Vec<Entry>>>,
    leds: Mutex<HashMap<(u32, u8), Led>>,
    /// Named indicator rows are indexed once when a mapping changes.
    named: Mutex<NamedRows>,
    out: Mutex<OutState>,
    out_cv: Condvar,
    interval_ms: AtomicU64,
    jog: Mutex<[Platter; 4]>,
    scan: Mutex<[Scan; 4]>,
    /// Last Load press per deck (the DDJ-400 family's double press = Instant Doubles).
    load_at: Mutex<[Option<Instant>; 4]>,
    in_conn: Mutex<Option<midir::MidiInputConnection<()>>>,
    pub received: AtomicU64,
    pub sent: AtomicU64,
}

impl Device {
    fn mapped(name: &str, input_id: String, map: MidiMap, factory: bool, conn: Option<midir::MidiOutputConnection>) -> Arc<Self> {
        let cpr = jog_counts(name);
        let dev = Arc::new(Self {
            name: name.to_string(),
            input_id,
            map: Mutex::new(map),
            factory,
            native_mapping: AtomicBool::new(false),
            needs_auth: needs_auth(name),
            has_output: conn.is_some(),
            learning: AtomicBool::new(false),
            input_epoch: Arc::default(),
            alive: AtomicBool::new(true),
            out_index: Mutex::new(HashMap::new()),
            leds: Mutex::new(HashMap::new()),
            named: Mutex::new(NamedRows::default()),
            out: Mutex::new(OutState { queue: OutQueue::default(), blink: Blinker::default(), blinking: HashMap::new(), conn }),
            out_cv: Condvar::new(),
            interval_ms: AtomicU64::new(1),
            jog: Mutex::new(std::array::from_fn(|_| Platter::new(cpr))),
            scan: Mutex::new(Default::default()),
            load_at: Mutex::new([None; 4]),
            in_conn: Mutex::new(None),
            received: AtomicU64::new(0),
            sent: AtomicU64::new(0),
        });
        dev.reindex();
        dev
    }

    /// After the map changed: mark the functions recordbox knows, index the outputs and
    /// forget what the LEDs were sent, so that they are all sent again.
    pub fn reindex(&self) {
        let reg = registry();
        let mut m = self.map.lock();
        m.tag_inputs(|n| reg.tag(n));
        let mut idx: HashMap<(u32, u8), Vec<Entry>> = HashMap::new();
        let mut named = NamedRows::default();
        if let Some(ms) = m.params.get("JogIndicatorInterval").copied().filter(|ms| (1..=1000).contains(ms)) {
            named.jog_interval = Duration::from_millis(ms as u64);
        }
        for o in &m.outputs {
            let tag = reg.tag(&o.name);
            if !output_acts(&o.name, o.kind) { continue; }
            if let Some(name) = NAMED_OUTPUTS.iter().find(|&&name| o.name == name) {
                named.entries.entry((name, o.deck)).or_default().push(o.clone());
            } else if tag != 0 {
                idx.entry((tag, o.deck)).or_default().push(o.clone());
            }
        }
        self.interval_ms.store(m.out_interval_ms(), Relaxed);
        *self.out_index.lock() = idx;
        *self.named.lock() = named;
        self.leds.lock().clear();
        let mut out = self.out.lock();
        out.blink = Blinker::default();
        out.blinking.clear();
        out.queue = OutQueue::default();
    }

    /// Whether the selected user delta came from the installed AlphaTheta application.
    pub fn from_rekordbox(&self) -> bool {
        self.native_mapping.load(Relaxed)
    }

    pub fn out_keys(&self) -> Vec<(u32, u8)> {
        self.out_index.lock().keys().copied().collect()
    }

    fn clear_button_feedback(&self) {
        if !self.has_output { return; }
        let held: Vec<_> = self.leds.lock().iter()
            .filter(|((tag, _), value)| **value == Led::On && registry().get(*tag).is_some_and(|f| echo_led(f.act)))
            .map(|(key, _)| *key)
            .collect();
        for (tag, deck) in held { self.set_led(tag, deck, Led::Off); }
    }

    /// Set the light of a function (for a deck). Sends only when it changed.
    pub fn set_led(&self, tag: u32, deck: u8, led: Led) {
        {
            let mut l = self.leds.lock();
            if l.get(&(tag, deck)) == Some(&led) {
                return;
            }
            l.insert((tag, deck), led);
        }
        let entries = self.out_index.lock().get(&(tag, deck)).cloned().unwrap_or_default();
        if entries.is_empty() {
            return;
        }
        let map = self.map.lock();
        let mut out = self.out.lock();
        for e in entries {
            out.blink.stop(e.code);
            out.blinking.remove(&e.code);
            match led {
                Led::Blink => {
                    out.blink.start(e.code, e.opts.blink, 0);
                    out.blinking.insert(e.code, (e.clone(), e.opts.priority));
                }
                Led::On => out.led(&map, &e, 127),
                Led::Off => out.led(&map, &e, 0),
                Led::Value(v) => out.led(&map, &e, v),
            }
        }
        drop(out);
        self.out_cv.notify_one();
    }

    /// Send a value to the outputs of a row by its name (rows the function registry does
    /// not have: the jog ring, the channel level meters, the loaded indicator). `value` gets
    /// the row and returns the value to send; a value equal to the last one sent is not sent.
    /// 14-bit rows (IndicatorHiRes, JogIndicatorHiRes) are sent as rekordbox's
    /// `MidiData::getLongMessage` 0x10132107c builds them: the upper seven bits on the code,
    /// the lower seven on the code + 0x10.
    fn set_named(&self, name: &'static str, deck: u8, value: impl Fn(&Entry) -> u16) {
        let mut named = self.named.lock();
        let NamedRows { entries, last, .. } = &mut *named;
        let Some(rows) = entries.get(&(name, deck)) else { return };
        let mut out = self.out.lock();
        let mut sent = false;
        for entry in rows {
            let value = value(entry);
            if last.get(&entry.code) != Some(&value) {
                out.feedback(entry, value);
                last.insert(entry.code, value);
                sent = true;
            }
        }
        if sent { self.out_cv.notify_one(); }
    }

    /// The output thread: one message per `MidiOutInterval`, and the 50 ms blink tick.
    fn run_output(self: Arc<Self>) {
        let Some(mut connection) = self.out.lock().conn.take() else { return };
        let tick = Duration::from_millis(50);
        let mut next_tick = Instant::now() + tick;
        loop {
            let mut out = self.out.lock();
            if !self.alive.load(Relaxed) { return; }
            if Instant::now() >= next_tick {
                next_tick = Instant::now() + tick;
                let changes = out.blink.tick();
                if !changes.is_empty() {
                    if let Some(map) = self.map.try_lock() {
                        for (code, on) in changes {
                            if let Some((e, _)) = out.blinking.get(&code).cloned() {
                                out.led(&map, &e, if on { 127 } else { 0 });
                            }
                        }
                    }
                }
            }
            match out.queue.pop() {
                Some(msg) => {
                    drop(out);
                    if connection.send(&msg).is_ok() {
                        self.sent.fetch_add(1, Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(self.interval_ms.load(Relaxed).clamp(1, 16)));
                }
                None if out.blinking.is_empty() => {
                    self.out_cv.wait(&mut out);
                }
                None => {
                    self.out_cv.wait_for(&mut out, next_tick.saturating_duration_since(Instant::now()));
                }
            }
        }
    }
}

/// The value a hot cue pad's light gets (`UiPlayer::deviceUpdateHotCueLed` 0x10202af74): the
/// cue's colour code when it has one, else the colour of its slot in the HOT CUE colour set
/// (`rekordbox::hotCueColorIndex*`; 36 for a loop). Pad rows then map it for their family
/// (`midi_map::pad_led`), button rows light for it. Codes above 64 are not sent (0).
pub fn hot_cue_led(color: u8, slot: i32, is_loop: bool, set: u8) -> u8 {
    let v = if color != 0 { color } else { crate::pads::hot_cue_set_color(set, slot, is_loop) };
    if v <= 64 { v } else { 0 }
}

/// `PREPDECK_MIDI_PROBE=<port name>` (with `PREPDECK_SHOT_PLAYER=midiprobe`): a real
/// controller's input port is opened for `secs` seconds and every message is logged
/// ("midiprobe:" lines on stderr). Its output port is never opened, so nothing at all is sent
/// to the device. Afterwards each message goes through MIDI Learn's filter
/// (`LearnFilter`): the code Learn would write is checked against an empty mapping (it binds
/// to a learnable command) and looked up, read-only, in the user mapping rekordbox keeps for
/// the port (`~/Library/Application Support/Pioneer/rekordbox6/MidiMappings/<port>.midi.csv`),
/// to name the function rekordbox would run.
pub fn probe(port: &str, secs: f64) {
    let say = |s: String| eprintln!("midiprobe: {s}");
    let Ok(mut input) = midir::MidiInput::new("recordbox-probe") else { return say("MIDI input client could not be opened".into()) };
    input.ignore(midir::Ignore::None);
    let names: Vec<String> = input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect();
    say(format!("input ports: {names:?}"));
    let Some(p) = input.ports().into_iter().find(|p| input.port_name(p).map(|n| n == port).unwrap_or(false)) else { return say(format!("{port}: no such input port")) };
    let log: Arc<Mutex<Vec<(u64, Vec<u8>)>>> = Arc::new(Mutex::new(vec![]));
    let l2 = log.clone();
    let t0 = Instant::now();
    let conn = input.connect(&p, "recordbox-probe-in", move |_, m, _| l2.lock().push((t0.elapsed().as_millis() as u64, m.to_vec())), ());
    let Ok(conn) = conn else { return say(format!("{port}: could not be opened")) };
    say(format!("{port}: listening for {secs:.1} s (input only; nothing is sent to the device)"));
    std::thread::sleep(Duration::from_secs_f64(secs));
    drop(conn);
    let msgs = log.lock().clone();
    for (t, m) in &msgs {
        say(format!("{t:>6} ms  {}", m.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")));
    }
    say(format!("{} messages in {secs:.1} s", msgs.len()));
    // rekordbox's own mapping for the port (read only).
    let rb_user = crate::midi_csv::native_user_files(&crate::midi_csv::native_user_dirs(), port).into_iter().find(|p| p.is_file());
    let rb_map = rb_user.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| MidiMap::load(port, None, Some(&b)).ok());
    say(format!("rekordbox's user mapping for the port: {}", match (&rb_user, &rb_map) { (Some(f), Some(m)) => format!("{} ({} output rows)", f.display(), m.outputs.len()), _ => "none".into() }));
    let factory = crate::midi_csv::factory_dirs().iter().find_map(|dir| crate::midi_csv::factory_file(dir, port));
    say(format!("factory mapping in the installed rekordbox: {}", factory.map(|f| f.display().to_string()).unwrap_or_else(|| "none".into())));
    let raw: Vec<Vec<u8>> = msgs.iter().map(|m| m.1.clone()).collect();
    let learned = probe_learn(port, &raw, rb_map.as_ref());
    for (c, ok, in_rb) in &learned {
        say(format!("Learn code {c:04X}: binds {ok} | rekordbox's mapping: {in_rb}"));
    }
    let codes = learned;
    say(format!("{} distinct codes Learn would take; {}", codes.len(), if codes.is_empty() { "nothing was touched on the controller during the run" } else { "PASS (every received control can be learned)" }));
}

/// MIDI Learn on received messages: the codes its filter passes, whether each binds to a
/// learnable command (PlayPause, deck 1) of an empty mapping, and what `rb` (rekordbox's
/// mapping for the port, when there is one) runs for it.
pub fn probe_learn(port: &str, msgs: &[Vec<u8>], rb: Option<&MidiMap>) -> Vec<(u16, bool, String)> {
    let mut filter = LearnFilter::default();
    let mut codes: Vec<u16> = vec![];
    for m in msgs {
        if let Some(c) = filter.feed(m) {
            if !codes.contains(&c) {
                codes.push(c);
            }
        }
    }
    let reg = registry();
    codes
        .into_iter()
        .map(|c| {
            let mut map = MidiMap::new(port);
            let row = LearnEntry { name: "PlayPause".into(), deck: 1, kind: Kind::Button, min: -1, max: -1, comment: String::new() };
            let ok = map.change_input(&row, None, Some(c), &|n: &str| reg.learnable(n)).is_ok() && map.input_at(c).is_some();
            let in_rb = rb.and_then(|m| m.input_at(c)).map(|e| format!("{} (deck {})", e.name, e.deck)).unwrap_or_else(|| "-".into());
            (c, ok, in_rb)
        })
        .collect()
}

/// The models whose MIDI side rekordbox authenticates (`getDeviceAuth` overrides).
pub fn needs_auth(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["DDJ-1000", "DDJ-800", "DDJ-400", "DDJ-200", "DDJ-RB", "DDJ-RR", "DDJ-RZ", "DDJ-WEGO4", "DDJ-FLX6", "DDJ-XP1", "DDJ-XP2", "DJM-250MK2", "DJM-450", "DJM-900NXS2", "DJM-TOUR1", "DJM-A9", "DJM-V10", "DJM-V5", "XDJ-RX", "XDJ-RR", "XDJ-XZ"].iter().any(|d| n.contains(d)) && !n.contains("XDJ-RX3")
}

/// Everything the port threads and the interface share.
pub struct Hub {
    playback_enabled: bool,
    routing: Mutex<Routing>,
    paused_input_names: Mutex<Vec<String>>,
    mode_epoch: Arc<AtomicU64>,
    pub devices: Mutex<Vec<Arc<Device>>>,
    pub decks: [Arc<Mutex<PlayState>>; 4],
    pub snap: Mutex<Snapshot>,
    overlay: Mutex<Vec<(u8, Param, f32, u64)>>,
    seq: AtomicU64,
    tx: crossbeam_channel::Sender<UiEvent>,
    pub ctx: Mutex<Option<eframe::egui::Context>>,
    /// Where the user's mappings are kept: `<data folder>/MidiMappings`.
    pub user_dir: PathBuf,
    factory_dirs: Vec<PathBuf>,
    native_user_dirs: Vec<PathBuf>,
    /// A physical release belongs to the audio loaded at the time of the press.
    cue_preview: [Mutex<Option<Preview>>; 4],
    hot_preview: [Mutex<Option<Preview>>; 4],
    /// Pads held, per deck (for the on-screen pads, which end a hold when no pointer is down).
    pub pads_down: [AtomicU8; 4],
    jog_bend: [AtomicU64; 4],
    takeover: Mutex<[TempoTakeover; 4]>,
    pickup: Mutex<[[Pickup; 3]; 4]>,
    /// Jog vinyl mode per deck (on by default, as in rekordbox).
    pub vinyl: [AtomicBool; 4],
    pub shift: [AtomicBool; 4],
    /// Ports recordbox opened itself, by name: never taken as controllers.
    pub ignore: Mutex<Vec<String>>,
    /// When set, only these ports are opened (tests and the hardware scripted run: a real
    /// controller on the machine is then neither opened nor sent anything).
    pub only: Mutex<Option<Vec<String>>>,
    /// When the interface last ran `poll` (ms since the hub was made): while it is not
    /// called (the window is hidden or minimised, so no frames are drawn), the hub's own
    /// thread keeps the lights that follow the engine going.
    pub ui_seen_ms: AtomicU64,
    pub born: Instant,
}

struct Preview {
    audio: std::sync::Weak<crate::audio::Decoded>,
    return_frame: Option<f64>,
}

impl Preview {
    fn belongs_to(&self, state: &PlayState) -> bool {
        state.audio.as_ref().is_some_and(|a| self.audio.ptr_eq(&Arc::downgrade(a)))
    }
}

impl Hub {
    pub fn new(decks: [Arc<Mutex<PlayState>>; 4], user_dir: PathBuf, playback_enabled: bool) -> (Arc<Hub>, crossbeam_channel::Receiver<UiEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let hub = Hub {
            playback_enabled,
            routing: Mutex::new(Routing::default()),
            paused_input_names: Default::default(),
            mode_epoch: Arc::default(),
            devices: Mutex::new(vec![]),
            decks,
            snap: Mutex::new(Snapshot::default()),
            overlay: Mutex::new(vec![]),
            seq: AtomicU64::new(1),
            tx,
            ctx: Mutex::new(None),
            user_dir,
            factory_dirs: crate::midi_csv::factory_dirs(),
            native_user_dirs: crate::midi_csv::native_user_dirs(),
            cue_preview: Default::default(),
            hot_preview: Default::default(),
            pads_down: std::array::from_fn(|_| AtomicU8::new(0)),
            jog_bend: std::array::from_fn(|_| AtomicU64::new(0f64.to_bits())),
            takeover: Mutex::new([TempoTakeover::default(); 4]),
            pickup: Mutex::new([[Pickup::default(); 3]; 4]),
            vinyl: std::array::from_fn(|_| AtomicBool::new(true)),
            shift: std::array::from_fn(|_| AtomicBool::new(false)),
            ignore: Mutex::new(vec![]),
            only: Mutex::new(None),
            ui_seen_ms: AtomicU64::new(0),
            born: Instant::now(),
        };
        (Arc::new(hub), rx)
    }

    fn wake(&self) {
        if let Some(c) = self.ctx.lock().as_ref() {
            c.request_repaint();
        }
    }

    pub fn device(&self, name: &str) -> Option<Arc<Device>> {
        self.devices.lock().iter().find(|d| d.name == name).cloned()
    }

    pub fn user_file(&self, device: &str) -> PathBuf {
        crate::midi_csv::local_file(&self.user_dir, device)
    }

    fn factory_mapping(&self, device: &str) -> Option<Vec<u8>> {
        self.factory_dirs.iter().find_map(|dir| {
            let path = crate::midi_csv::factory_file(dir, device)?;
            let bytes = crate::midi_csv::read_file(&path).ok()?;
            let csv = crate::midi_csv::CsvFile::parse(&bytes).ok()?;
            crate::midi_csv::factory_matches(device, &csv.device).then_some(bytes)
        })
    }

    /// Factory rows plus one user delta. A first connection never creates a local file.
    /// Earlier versions wrote a header before loading and thereby hid the native delta.
    pub fn load_map(&self, device: &str) -> (MidiMap, bool) {
        let (map, factory, _) = self.load_map_source(device);
        (map, factory)
    }

    fn load_map_source(&self, device: &str) -> (MidiMap, bool, bool) {
        let factory = self.factory_mapping(device);
        let local = self.user_file(device);
        let legacy = || crate::midi_csv::legacy_file(&self.user_dir, device).filter(|path| *path != local);
        let native = || crate::midi_csv::native_user_files(&self.native_user_dirs, device);
        let inputs = std::iter::once((local.clone(), true))
            .chain(std::iter::once_with(legacy).flatten().map(|p| (p, true)))
            .chain(std::iter::once_with(native).flatten().map(|p| (p, false)));
        for (path, own) in inputs {
            if let Ok(data) = crate::midi_csv::read_file(&path) {
                let Ok(csv) = crate::midi_csv::validate_import(device, &data) else { continue };
                if own && crate::midi_csv::bare_header(&csv) { continue; }
                if let Ok(map) = MidiMap::load(device, factory.as_deref(), Some(&data)) { return (map, factory.is_some(), !own); }
            }
        }
        (MidiMap::load(device, factory.as_deref(), None).unwrap_or_else(|_| MidiMap::new(device)), factory.is_some(), false)
    }

    pub fn export_file(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        crate::midi_csv::atomic_write(path, bytes, false, self.factory_dirs.iter().chain(&self.native_user_dirs))
    }

    pub fn write_override(&self, device: &str, bytes: &[u8]) -> std::io::Result<()> {
        let csv = crate::midi_csv::validate_import(device, bytes)?;
        let path = self.user_file(device);
        // Distinguish an explicit empty reset/import from an old automatic placeholder.
        let mut saved = bytes.to_vec();
        if crate::midi_csv::bare_header(&csv) {
            if !saved.ends_with(b"\n") { saved.extend_from_slice(b"\r\n"); }
            saved.extend_from_slice(b"# recordbox local override\r\n");
        }
        crate::midi_csv::atomic_write(&path, &saved, true, self.factory_dirs.iter().chain(&self.native_user_dirs))?;
        if let Some(controller) = self.device(device) {
            controller.native_mapping.store(false, Relaxed);
        }
        Ok(())
    }

    pub fn save_map(&self, dev: &Device) -> std::io::Result<()> {
        let text = dev.map.lock().save_text();
        self.write_override(&dev.name, text.as_bytes())
    }

    /// Look at the MIDI ports: connect controllers that appeared, drop those that left.
    pub fn scan(self: &Arc<Self>) {
        let Ok(mut input) = midir::MidiInput::new("recordbox") else { return };
        input.ignore(midir::Ignore::None);
        let Some(mut previous) = self.paused_input_names.try_lock() else { return };
        let ignore = self.ignore.lock().clone();
        let only = self.only.lock().clone();
        let ports: Vec<_> = input.ports().into_iter().filter_map(|port| {
            let name = input.port_name(&port).ok()?;
            if name.is_empty() || name == "<error>" || name == "Arduino Leonardo" || ignore.contains(&name)
                || only.as_ref().is_some_and(|allowed| !allowed.contains(&name)) {
                return None;
            }
            let id = port.id();
            Some((name, id, port))
        }).collect();
        let visible = ports::Inputs::inspect(ports.iter().map(|(name, id, _)| (name.as_str(), id.as_str())));
        let mut changed = *previous != visible.paused;
        let mut removed = Vec::new();
        // Retire old workers before a replacement can send its first note.
        let gone: Vec<_> = self.devices.lock().extract_if(.., |d| !visible.contains(&d.name, &d.input_id)).collect();
        for device in gone {
            {
                let _out = device.out.lock();
                device.alive.store(false, Relaxed);
                device.out_cv.notify_all();
            }
            device.in_conn.lock().take();
            self.release_device(&device);
            removed.push(device.name.clone());
            changed = true;
        }
        for (name, id, port) in ports {
            if visible.contains(&name, &id) && self.device(&name).is_none() && self.connect(&name, id, &port).is_some() {
                changed = true;
            }
        }
        if changed {
            let _ = self.tx.send(UiEvent::Ports { removed, paused: visible.paused.clone() });
            self.wake();
        }
        *previous = visible.paused;
    }

    fn connect(self: &Arc<Self>, name: &str, input_id: String, port: &midir::MidiInputPort) -> Option<Arc<Device>> {
        let (map, factory, native_mapping) = self.load_map_source(name);
        let output_enabled = std::env::var("PREPDECK_MIDI_NO_OUT").as_deref() != Ok("1");
        let conn = output_enabled.then(|| midir::MidiOutput::new("recordbox").ok()).flatten().and_then(|out| {
            let ports: Vec<_> = out.ports().into_iter().filter_map(|p| out.port_name(&p).ok().map(|n| (n, p))).collect();
            let names = ports.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
            let index = crate::midi_csv::paired_output(name, &names)?;
            out.connect(&ports[index].1, "recordbox-out").ok()
        });
        let dev = Device::mapped(name, input_id, map, factory, conn);
        dev.native_mapping.store(native_mapping, Relaxed);
        // The callback only queues; the worker does the rest.
        let (tx, rx) = crossbeam_channel::unbounded();
        let device_epoch = dev.input_epoch.clone();
        let mode_epoch = self.mode_epoch.clone();
        let mut input = midir::MidiInput::new("recordbox").ok()?;
        input.ignore(midir::Ignore::None);
        let conn = input.connect(port, "recordbox-in", move |t, msg, _| {
            // Only channel messages with two data bytes can address a mapping or be learned.
            if let &[status @ 0x80..=0xef, data1, data2] = msg {
                let _ = tx.send((t, [status, data1, data2], mode_epoch.load(Relaxed), device_epoch.load(Relaxed)));
            }
        }, ()).ok()?;
        *dev.in_conn.lock() = Some(conn);
        let (h, d) = (self.clone(), dev.clone());
        let _ = std::thread::Builder::new().name(format!("midi-in {name}")).spawn(move || h.run_input(d, rx));
        if dev.has_output {
            let d = dev.clone();
            let _ = std::thread::Builder::new().name(format!("midi-out {name}")).spawn(move || d.run_output());
        }
        eprintln!("### MIDI:open success [{name}]{}", if factory { "" } else { " (no factory mapping: MIDI Learn only)" });
        self.devices.lock().push(dev.clone());
        Some(dev)
    }

    /// The worker of a port (rekordbox's `DeviceMidi::run`).
    fn run_input(self: Arc<Self>, dev: Arc<Device>, rx: crossbeam_channel::Receiver<(u64, [u8; 3], u64, u64)>) {
        while dev.alive.load(Relaxed) {
            // An idle controller sleeps until an input or disconnect. Continuous fader
            // traffic must never starve a deadline set by a previous platter movement.
            let deadline = dev.jog.lock().iter().filter_map(Platter::deadline).min();
            let packet = match deadline {
                Some(when) if when <= Instant::now() => {
                    self.idle_jogs(&dev, Instant::now());
                    continue;
                }
                Some(when) => rx.recv_deadline(when),
                None => rx.recv().map_err(|_| crossbeam_channel::RecvTimeoutError::Disconnected),
            };
            match packet {
                Ok((t, msg, mode_epoch, device_epoch)) => {
                    self.input_message(&dev, msg, t as f64 / 1000.0, mode_epoch, device_epoch);
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => self.idle_jogs(&dev, Instant::now()),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
        self.release_device(&dev);
    }

    fn input_message(&self, dev: &Arc<Device>, msg: [u8; 3], t_ms: f64, mode_epoch: u64, device_epoch: u64) {
        let mut route = self.routing.lock();
        // A physical press queued before a mode or Learn transition belongs to the old mapping.
        if !dev.alive.load(Relaxed) || self.mode_epoch.load(Relaxed) != mode_epoch || dev.input_epoch.load(Relaxed) != device_epoch {
            return;
        }
        dev.received.fetch_add(1, Relaxed);
        if dev.learning.load(Relaxed) {
            let _ = self.tx.send(UiEvent::Learn { device: Arc::downgrade(dev), msg });
            self.wake();
            return;
        }
        let (event, peer) = {
            let mut map = dev.map.lock();
            let Some(event) = map.handle(&msg, t_ms) else { return };
            let peer = map.dual_event(&event);
            (event, peer)
        };
        self.dispatch(&mut route, dev, event);
        if let Some(peer) = peer {
            self.dispatch(&mut route, dev, peer);
        }
    }

    fn dispatch(&self, route: &mut Routing, dev: &Arc<Device>, mut ev: Event) {
        let Some(f) = registry().get(ev.tag) else { return };
        if !route.performance && !routing::browsing(f.act) { return; }
        // A stateless button echoes its press on the originating controller. Other
        // controllers may hold the same button and release it independently.
        if dev.has_output && echo_led(f.act) {
            if let Val::On(pressed, _) = ev.val {
                dev.set_led(ev.tag, ev.deck, if pressed { Led::On } else { Led::Off });
            }
        }
        if routing::momentary(f.act) {
            if !matches!(ev.val, Val::On(false, _)) && !matches!(f.act, Act::FxTapLong(_) | Act::FxAssignHold(..)) {
                let Some(n) = self.deck_ix(ev.deck) else { return };
                ev.deck = n as u8 + 1;
            }
            if !route.hold(dev, &mut ev) { return; }
        }
        self.deliver(route, dev, ev);
    }

    fn deliver(&self, route: &mut Routing, dev: &Arc<Device>, ev: Event) {
        let seq = self.seq.fetch_add(1, Relaxed);
        let done = ev.fast && registry().get(ev.tag).is_some_and(|f| self.fast(route, dev, f, ev.deck, ev.val, seq));
        let _ = self.tx.send(UiEvent::Control { device: dev.name.clone(), tag: ev.tag, deck: ev.deck, val: ev.val, fast_done: done, seq });
        self.wake();
    }

    /// The deck a mapping's deck number means: 1..4, or the active deck for a row without one.
    pub fn deck_ix(&self, deck: u8) -> Option<usize> {
        let s = self.snap.lock();
        let n = if deck == 0 { s.active as usize } else { deck as usize - 1 };
        (n < s.count as usize && n < 4).then_some(n)
    }

    // ------------------------------------------------------------------ transport

    /// PLAY/PAUSE. A cue preview that is running becomes normal play.
    pub fn play_pause(&self, n: usize) {
        if !self.playback_enabled { return; }
        let mut s = self.decks[n].lock();
        if s.audio.is_none() {
            return;
        }
        if self.cue_preview[n].lock().take().is_some_and(|p| p.belongs_to(&s)) {
            return;
        }
        s.playing = !s.playing;
    }

    /// CUE pressed or released. False: the deck stands away from the cue point, so the press
    /// sets the cue point there (the interface does that, with quantize).
    pub fn cue(&self, n: usize, down: bool) -> bool {
        let cue = self.snap.lock().decks[n].cue_point;
        let mut s = self.decks[n].lock();
        let mut preview = self.cue_preview[n].lock();
        if !down {
            if let Some(frame) = preview.take().filter(|p| p.belongs_to(&s)).and_then(|p| p.return_frame) {
                s.playing = false;
                s.pos = frame;
            }
            return true;
        }
        let Some(audio) = s.audio.as_ref() else { return true };
        let sr = audio.sr as f64;
        if s.playing {
            if !preview.as_ref().is_some_and(|p| p.belongs_to(&s)) {
                s.playing = false;
                s.pos = cue * sr;
                s.loop_range = None;
            }
            true
        } else if (s.pos / sr - cue).abs() < 0.01 {
            if !self.playback_enabled { return true; }
            *preview = Some(Preview { audio: Arc::downgrade(audio), return_frame: Some(cue * sr) });
            s.playing = true;
            true
        } else {
            false
        }
    }

    /// A hot cue pad pressed or released. False: the slot is empty, so the press stores a
    /// hot cue (the interface does that).
    pub fn hot_cue(&self, n: usize, slot: usize, down: bool) -> bool {
        let hot = self.snap.lock().decks[n].hot.get(slot).copied().flatten();
        let mut s = self.decks[n].lock();
        let mut preview = self.hot_preview[n].lock();
        if !down {
            self.pads_down[n].store(0, Relaxed);
            if let Some(held) = preview.take().filter(|p| p.belongs_to(&s)) {
                s.slip_hold = false;
                if let Some(frame) = held.return_frame {
                    s.playing = false;
                    s.loop_range = None;
                    s.pos = frame;
                }
            }
            return true;
        }
        let Some(audio) = s.audio.as_ref() else { return true };
        let Some((start, end)) = hot else { return false };
        let sr = audio.sr as f64;
        if !preview.as_ref().is_some_and(|p| p.belongs_to(&s)) {
            *preview = Some(Preview { audio: Arc::downgrade(audio), return_frame: None });
        }
        self.pads_down[n].store(1, Relaxed);
        // SLIP on a playing deck: the cue sounds while the pad is held.
        if s.slip && s.playing && s.slip_pos.is_none() {
            s.slip_pos = Some(s.pos);
        }
        s.slip_hold = s.slip && s.playing;
        s.pos = start * sr;
        if let Some(e) = end.filter(|e| *e > start) {
            s.loop_range = Some((start, e));
        }
        if self.playback_enabled && !s.playing {
            // (Gate Cue: held, it plays and returns; off, it plays on.)
            if crate::prefs_bridge::GATE_CUE.load(Relaxed) {
                if let Some(held) = preview.as_mut() { held.return_frame = Some(start * sr); }
            }
            s.playing = true;
        }
        true
    }

    /// A pad is being held on deck `n` by a controller.
    pub fn pad_held(&self, n: usize) -> bool {
        self.pads_down[n.min(3)].load(Relaxed) != 0
    }

    // ------------------------------------------------------------------ absolute controls

    /// Put a mixer or tempo value into the engine. Returns false when it was not applied (a
    /// pick-up is waiting, or the control does not exist in the layout showing).
    pub fn set_param(&self, n: usize, p: Param, pos: f32) -> bool {
        let snap = *self.snap.lock();
        if !snap.perf {
            return false;
        }
        match p {
            Param::ChFader => self.decks[n].lock().ch_fader = crate::mixer::channel_fader_gain(pos, snap.curve),
            Param::Trim => self.decks[n].lock().trim = crate::mixer::trim_gain(pos),
            Param::Eq(b) => {
                let b = (b as usize).min(2);
                let d = &snap.decks[n];
                // Pick-up after the knobs changed their meaning (EQ / STEM ISO).
                let sw = if d.stem_iso_on { d.stem_iso[b] } else { d.eq[b] };
                if !self.pickup.lock()[n][b].feed(pos, sw) {
                    return false;
                }
                let mut s = self.decks[n].lock();
                if d.stem_iso_on {
                    let g = crate::audio::stem_iso_gain(pos);
                    match b {
                        0 => s.stem_iso[0] = g,
                        1 => {
                            s.stem_iso[1] = g;
                            s.stem_iso[2] = g;
                        }
                        _ => s.stem_iso[3] = g,
                    }
                } else {
                    s.eq_pos[b] = pos;
                }
            }
            Param::CrossFader => {
                let g = crate::audio::crossfader_gains(pos);
                for i in 0..(snap.count as usize).min(4) {
                    self.decks[i].lock().fader = match snap.decks[i].assign {
                        1 => g.0,
                        2 => g.1,
                        _ => 1.0,
                    };
                }
            }
            Param::Tempo => self.decks[n].lock().tempo = 1.0 + pos as f64,
        }
        true
    }

    /// The tempo slider at `raw` (0..16383): through the soft takeover. The tempo change to
    /// set (a fraction), when the slider has the deck.
    pub fn tempo_slider(&self, n: usize, raw: i32) -> Option<f64> {
        let d = self.snap.lock().decks[n];
        let deck_pct = (self.decks[n].lock().tempo - 1.0) * 100.0;
        let slider = tempo_slider_percent(raw, d.range * 100.0);
        self.takeover.lock()[n].feed(slider, deck_pct, d.loaded, d.follower).map(|p| p / 100.0)
    }

    pub fn takeover_state(&self, n: usize) -> TempoTakeover {
        self.takeover.lock()[n.min(3)]
    }

    pub fn arm_takeover(&self, n: usize) {
        self.takeover.lock()[n.min(3)].arm();
    }

    pub fn start_pickup(&self, n: usize) {
        for p in self.pickup.lock()[n.min(3)].iter_mut() {
            p.start();
        }
    }

    // ------------------------------------------------------------------ jog

    fn jog_out(&self, n: usize, out: JogOut) {
        if !self.playback_enabled { return; }
        match out {
            JogOut::Nothing => {}
            JogOut::Bend(b) => {
                self.jog_bend[n].store(b.to_bits(), Relaxed);
                let base = self.snap.lock().decks[n].button_bend;
                self.decks[n].lock().bend = base + b;
            }
            other => jog_scratch(self, n, other),
        }
    }

    // ------------------------------------------------------------------ the fast path

    /// What a `Fast` row (or the jog) does from the worker. True when the engine side is done
    /// and the interface only has to follow.
    fn fast(&self, route: &mut Routing, dev: &Arc<Device>, f: &Func, deck: u8, val: Val, seq: u64) -> bool {
        let n = match (val, deck) {
            (Val::On(false, _), 1..=4) => deck as usize - 1,
            _ => { let Some(n) = self.deck_ix(deck) else { return false }; n }
        };
        let press = matches!(val, Val::On(true, _));
        let release = matches!(val, Val::On(false, _));
        let over = |p: Param, v: f32| self.overlay.lock().push((n as u8, p, v, seq));
        match f.act {
            Act::PlayPause => {
                if press {
                    self.play_pause(n);
                    self.transport_leds(n);
                }
                true
            }
            Act::Cue => {
                let done = if press || release { self.cue(n, press) } else { true };
                self.transport_leds(n);
                done
            }
            Act::HotCue(slot) => {
                if press || release {
                    self.hot_cue(n, slot as usize, press)
                } else {
                    true
                }
            }
            Act::SamplerPad(pad, velocity) => {
                if !self.playback_enabled { return true; }
                let snap = *self.snap.lock();
                if !snap.perf || !snap.smp_plain {
                    return false;
                }
                // Decks 2 and 4 play the second eight when the sampler has sixteen slots.
                let pad = pad as usize + if n % 2 == 1 && snap.smp_pads > 8 { 8 } else { 0 };
                if pad >= snap.smp_pads as usize || snap.smp_loaded & (1 << pad) == 0 {
                    return press;
                }
                let Val::On(on, v) = val else { return true };
                if let Some(e) = crate::sampler::SAMPLER.lock().as_mut() {
                    if on {
                        let gain = if velocity { crate::sampler_ext::velocity_gain(snap.velocity_curve, v) } else { 1.0 };
                        // (A loop sample pressed while the sequencer records is refused; the
                        // interface says so.)
                        if e.launch(pad, snap.bank as usize, gain) {
                            return false;
                        }
                    } else {
                        e.stop(pad, snap.bank as usize, false);
                    }
                }
                true
            }
            Act::ChFader | Act::Trim | Act::Eq(_) | Act::CrossFader => {
                let Some(pos) = val.unit() else { return false };
                let p = match f.act {
                    Act::ChFader => Param::ChFader,
                    Act::Trim => Param::Trim,
                    Act::Eq(b) => Param::Eq(b),
                    _ => Param::CrossFader,
                };
                if self.set_param(n, p, pos) {
                    over(p, pos);
                    true
                } else {
                    // A pick-up is waiting: the interface has nothing to do either.
                    matches!(p, Param::Eq(_))
                }
            }
            Act::Tempo => {
                let Val::Abs { .. } = val else { return false };
                let raw = (val.unit().unwrap_or(0.5) * 16383.0).round() as i32;
                let raw = if val.unit() == Some(0.5) { 8192 } else { raw };
                match self.tempo_slider(n, raw) {
                    Some(adj) => {
                        self.set_param(n, Param::Tempo, adj as f32);
                        over(Param::Tempo, adj as f32);
                        true
                    }
                    // Not picked up yet (the direction LEDs show which way to go).
                    None => true,
                }
            }
            Act::MasterLevel => {
                let Some(pos) = val.unit() else { return false };
                for d in &self.decks {
                    d.lock().volume = pos;
                }
                true
            }
            Act::JogTouch => {
                let Val::On(on, _) = val else { return true };
                let playing = self.decks[n].lock().playing;
                let snap = *self.snap.lock();
                let out = {
                    let mut j = dev.jog.lock();
                    j[n].jog.vinyl = self.vinyl[n].load(Relaxed);
                    j[n].jog.rpm = if snap.rpm45 { 45.0 } else { 100.0 / 3.0 };
                    j[n].jog.touch(on, playing)
                };
                self.routed_jog(route, dev, n, out, false);
                true
            }
            Act::Jog(scratch_row) => {
                let Val::Jog { delta, t_ms } = val else { return true };
                let out = dev.jog.lock()[n].rotate(delta, t_ms, scratch_row, Instant::now());
                self.routed_jog(route, dev, n, out, false);
                true
            }
            Act::JogSearch => {
                let Val::Jog { delta, t_ms } = val else { return true };
                if !self.playback_enabled { return true; }
                let k = dev.jog.lock()[n].jog.k();
                let Some(secs) = dev.scan.lock()[n].value(delta, t_ms, k) else { return true };
                let mut s = self.decks[n].lock();
                if let Some(a) = s.audio.clone() {
                    s.pos = (s.pos + secs * a.sr as f64).clamp(0.0, a.frames() as f64);
                }
                true
            }
            _ => false,
        }
    }

    /// The outputs that follow the engine alone, sent from the interface every frame and,
    /// while it draws no frames, from the hub's own thread: PLAY / CUE, the jog ring
    /// (`JogPlayPauseIndicator(Short)`, a `JogIndicator` row: `Min + Max * angle / 360`,
    /// `MidiJogIndicator::setLongValue` 0x101324334; the platter turns at 33 1/3 rpm), the
    /// channel level meter (`ChannelLevel`, an `Indicator` row: `max(L, R) * 100`,
    /// `ChannelLevelDeviceUnit::timerCallback` 0x1000d5ca0, here the deck's level before the
    /// faders as the engine meters it) and `LoadedIndicator` (127 loaded, 0 empty: not traced).
    pub fn engine_outputs(&self) {
        let devs: Vec<Arc<Device>> = self.devices.lock().iter().filter(|d| d.has_output).cloned().collect();
        if devs.is_empty() {
            return;
        }
        let snap = *self.snap.lock();
        let now = Instant::now();
        for n in 0..snap.count.min(4) as usize {
            self.transport_leds(n);
            let (loaded, angle, level, looping, slipping, playing) = {
                let s = self.decks[n].lock();
                let (looping, slipping, playing) = (s.loop_range.is_some(), s.slip && s.slip_pos.is_some(), s.playing);
                match s.audio.as_ref() {
                    Some(a) => {
                        let secs = s.pos / a.sr.max(1) as f64;
                        let rpm = if snap.rpm45 { 45.0 } else { 100.0 / 3.0 };
                        let turn = (secs * rpm * 6.0).rem_euclid(360.0);
                        (true, turn as i32, s.peak[0].max(s.peak[1]), looping, slipping, playing)
                    }
                    None => (false, 360, 0.0, false, false, false),
                }
            };
            let lvl = ((level.clamp(0.0, 1.0) * 100.0) as i32).clamp(0, 127) as u8;
            for d in &devs {
                for deck in [n as u8 + 1, if n as u8 == snap.active { 0 } else { 255 }] {
                    if deck == 255 {
                        continue;
                    }
                    if d.named.lock().due(deck, now) {
                        for name in ["JogPlayPauseIndicator", "JogPlayPauseIndicatorShort"] {
                            d.set_named(name, deck, |e| match e.kind {
                                Kind::JogIndicatorHiRes | Kind::IndicatorHiRes => crate::midi_map::jog_ring14(angle, &e.opts),
                                _ => u16::from(crate::midi_map::jog_ring(angle, &e.opts)),
                            });
                        }
                    }
                    let level14 = (level.clamp(0.0, 1.0) * 16383.0).round() as u16;
                    d.set_named("ChannelLevel", deck, |e| if e.kind == Kind::IndicatorHiRes { level14 } else { u16::from(lvl) });
                    for (name, on) in [("LoadedIndicator", loaded), ("GetLoopActive", looping), ("SlipStateForFlashing", slipping), ("PlayingState", playing)] {
                        d.set_named(name, deck, |e| match (on, e.kind) {
                            (false, _) => 0,
                            (true, Kind::IndicatorHiRes | Kind::JogIndicatorHiRes) => 16383,
                            _ => 127,
                        });
                    }
                }
            }
        }
    }

    /// PLAY and CUE lights straight from the engine (so they follow a press without waiting
    /// for a frame): PLAY lit while playing and blinking while a loaded deck stands; CUE lit
    /// while playing or standing on the cue point, blinking elsewhere.
    pub fn transport_leds(&self, n: usize) {
        let reg = registry();
        let (play, cue) = (reg.tag("PlayPause"), reg.tag("Cue"));
        let cue_point = self.snap.lock().decks[n].cue_point;
        let (loaded, playing, at_cue) = {
            let s = self.decks[n].lock();
            match s.audio.as_ref() {
                Some(a) => (true, s.playing, (s.pos / a.sr as f64 - cue_point).abs() < 0.01),
                None => (false, false, false),
            }
        };
        let pl = if playing { Led::On } else if loaded { Led::Blink } else { Led::Off };
        let cl = if !loaded { Led::Off } else if playing || at_cue { Led::On } else { Led::Blink };
        let active = self.snap.lock().active as usize;
        for d in self.devices.lock().iter() {
            d.set_led(play, n as u8 + 1, pl);
            d.set_led(cue, n as u8 + 1, cl);
            if n == active {
                d.set_led(play, 0, pl);
                d.set_led(cue, 0, cl);
            }
        }
    }
}

/// The platter has the record: scratch, let go, or spin on.
///
/// The deck engine's jog (deck.rs) does the work: `Scratch(speed)` holds the record at the
/// platter's speed (negative = backwards), `Release` gives it back to normal play. A platter
/// let go while spinning (`Spin`) is treated as a release: the worker gets no further speeds
/// to follow the spin down with.
pub fn jog_scratch(hub: &Hub, n: usize, out: JogOut) {
    let jog = match out {
        JogOut::Scratch(speed) => crate::deck::Jog::Speed(speed),
        _ => crate::deck::Jog::None,
    };
    hub.jog_bend[n].store(0f64.to_bits(), Relaxed);
    hub.decks[n].lock().dk.jog = jog;
}

// ---------------------------------------------------------------------- the app's side

/// MIDI state of the app (a field of `App`).
pub struct MidiState {
    pub hub: Option<Arc<Hub>>,
    rx: Option<crossbeam_channel::Receiver<UiEvent>>,
    /// The CoreMIDI client made on the main thread, kept so that port changes are noticed.
    _keeper: Option<midir::MidiInput>,
    pub win: crate::ui_midi::Win,
    /// Device names, as the MIDI setting window lists them.
    pub devices: Vec<String>,
    pub paused_inputs: Vec<String>,
    /// The beat jump pads' page per deck (1..9; 5 puts 1 beat on the first pad).
    pub jump_page: [u8; 4],
    /// Instant Doubles asked for: (deck, source deck), waiting for the track to be read.
    pending_double: Option<(u8, u8)>,
    /// What the stubs were last asked (for the features other parts are building).
    pub stub_log: Vec<String>,
    /// The tempo each deck had after the last frame, to arm the takeover when it is changed
    /// on screen.
    last_tempo: [f64; 4],
    last_iso: [bool; 4],
    last_follow: [bool; 4],
    started: bool,
    /// Messages that were mapped, for the scripted check.
    pub handled: u64,
    /// FX1 / FX2 TapBeatWithLongPress held since (a second makes it AUTO).
    tap_held: [Option<Instant>; 2],
}

impl Default for MidiState {
    fn default() -> MidiState {
        MidiState { hub: None, rx: None, _keeper: None, win: Default::default(), devices: vec![], paused_inputs: vec![], jump_page: [5; 4], pending_double: None, stub_log: vec![], last_tempo: [0.0; 4], last_iso: [false; 4], last_follow: [false; 4], started: false, handled: 0, tap_held: [None; 2] }
    }
}

impl MidiState {
    /// A controller holds a pad of deck `n` (0 = deck 1).
    pub fn pad_held(&self, n: u8) -> bool {
        self.hub.as_ref().map(|h| h.pad_held(n as usize)).unwrap_or(false)
    }
    pub fn connected(&self) -> bool {
        !self.devices.is_empty()
    }
}

fn start(app: &mut App) {
    app.midi.started = true;
    if std::env::var("PREPDECK_MIDI").map(|v| v == "off").unwrap_or(false) {
        return;
    }
    let (hub, rx) = Hub::new(app.player.decks.clone(), crate::db::data_dir().join("MidiMappings"), !app.deck_options.disable_play);
    hub.routing.lock().performance = app.performance();
    *hub.snap.lock() = snapshot(app);
    *hub.ctx.lock() = Some(app.ctx.clone());
    // The hardware probe (`probe`) has the port to itself: the hub opens nothing.
    if std::env::var("PREPDECK_MIDI_PROBE").is_ok() {
        *hub.only.lock() = Some(vec![]);
    }
    // The window's scripted runs use their own virtual controller only: a real controller on
    // the machine is not opened by them.
    if std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "miditest")).unwrap_or(false) || std::env::var("PREPDECK_SHOT_MIDI").is_ok() {
        *hub.only.lock() = Some(vec![crate::ui_midi::TEST_NAME.to_string()]);
    }
    // The first client is made here, on the main thread, whose run loop delivers CoreMIDI's
    // notices of devices coming and going.
    app.midi._keeper = midir::MidiInput::new("recordbox").ok();
    let h = Arc::downgrade(&hub);
    let _ = std::thread::Builder::new().name("midi-ports".into()).spawn(move || loop {
        let Some(hub) = h.upgrade() else { return };
        hub.scan();
        drop(hub);
        std::thread::sleep(Duration::from_millis(1000));
    });
    // While the window draws no frames (hidden, minimised: macOS sends it no redraws), the
    // lights that follow the engine are kept going from here, every 50 ms.
    let w = Arc::downgrade(&hub);
    let _ = std::thread::Builder::new().name("midi-leds".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_millis(50));
        let Some(h) = w.upgrade() else { return };
        let now = h.born.elapsed().as_millis() as u64;
        if now.saturating_sub(h.ui_seen_ms.load(Relaxed)) > 150 {
            h.engine_outputs();
        }
    });
    app.midi.hub = Some(hub);
    app.midi.rx = Some(rx);
}

fn snapshot(app: &App) -> Snapshot {
    let count = app.deck_count();
    let mut decks = [DeckSnap::default(); 4];
    for n in 0..count.min(4) {
        let d = app.deck_of(n);
        let mut hot = [None; 16];
        for c in d.cues.iter().filter(|c| c.kind == CueKind::Hot && (0..16).contains(&c.slot)) {
            hot[c.slot as usize] = Some((c.start, c.end));
        }
        let m = app.px.mix[n as usize];
        let follower = app.two_players() && d.x.beat_sync && (n != app.px.master || app.sampler_is_master());
        let range = if app.performance() { crate::player_ext::TEMPO_RANGES[(d.x.tempo_range as usize + 1) % 4] / 100.0 } else { crate::player_ext::TEMPO_RANGE };
        decks[n as usize] = DeckSnap { loaded: d.audio.is_some(), cue_point: d.cue_point, hot, range, follower, button_bend: d.x.bend as f64 * crate::player_ext::BEND_AMOUNT, stem_iso_on: m.stem_iso_on, assign: m.assign, eq: m.eq, stem_iso: m.stem_iso };
    }
    let bank = app.prefs.sampler.bank.min(3);
    let pads = app.sampler_pads().min(16);
    let mut loaded = 0u16;
    for p in 0..pads {
        if app.sampler_loaded(bank as usize * 16 + p) && app.px.sampler.ready(bank as usize * 16 + p) {
            loaded |= 1 << p;
        }
    }
    Snapshot { perf: app.performance(), count, active: app.px.active, decks, curve: app.prefs.ch_fader_curve, bank, smp_pads: pads as u8, smp_loaded: loaded, smp_plain: app.px.sampler.seq_mode == 0, velocity_curve: app.prefs.sampler.velocity_curve, takeover_eq_iso: app.prefs.midi.takeover_eq_iso, rpm45: app.prefs.midi.jog_45rpm }
}

/// The first connection to the system's MIDI server takes about half a second. It is made
/// on a helper thread first, so the main thread's own client (needed there for port-change
/// notices) is created quickly once the server is up, without holding up the first frames.
fn midi_server_ready() -> bool {
    use std::sync::atomic::AtomicU8;
    static STATE: AtomicU8 = AtomicU8::new(0);
    match STATE.load(Relaxed) {
        0 => {
            STATE.store(1, Relaxed);
            let spawned = std::thread::Builder::new().name("midi-warmup".into()).spawn(|| {
                let _ = midir::MidiInput::new("recordbox warm-up").map(|c| c.port_count());
                STATE.store(2, Relaxed);
            });
            if spawned.is_err() {
                STATE.store(2, Relaxed);
            }
            false
        }
        1 => false,
        _ => true,
    }
}

impl MidiState {
    /// A physical search button may remain held when the app loses keyboard focus.
    pub(crate) fn search_held(&self, deck: u8) -> bool {
        self.hub.as_ref().is_some_and(|hub| hub.routing.lock().search_held(deck))
    }
}

/// Deliver controller events and refresh their feedback once per frame.
pub fn poll(app: &mut App) {
    if !app.midi.started {
        if std::env::var("PREPDECK_MIDI").as_deref() == Ok("off") {
            app.midi.started = true;
            return;
        }
        if !midi_server_ready() {
            return;
        }
        start(app);
    }
    let Some(hub) = app.midi.hub.clone() else { return };
    hub.ui_seen_ms.store(hub.born.elapsed().as_millis() as u64, Relaxed);
    let have = !hub.devices.lock().is_empty();
    let pending = app.midi.rx.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
    if !have && !pending {
        if !app.midi.devices.is_empty() {
            app.midi.devices.clear();
        }
        return;
    }
    *hub.snap.lock() = snapshot(app);
    // The takeover is armed again when the tempo was changed on screen or sync changed, and
    // the EQ knobs wait for a pick-up when they switch between EQ and STEM ISO.
    for n in 0..app.deck_count().min(4) as usize {
        let d = app.deck_of(n as u8);
        let (tempo, iso, follow) = (d.x.tempo_adj, app.px.mix[n].stem_iso_on, d.x.beat_sync);
        if (tempo - app.midi.last_tempo[n]).abs() > 1e-9 || follow != app.midi.last_follow[n] {
            hub.arm_takeover(n);
        }
        if iso != app.midi.last_iso[n] && app.prefs.midi.takeover_eq_iso {
            hub.start_pickup(n);
        }
        app.midi.last_iso[n] = iso;
        app.midi.last_follow[n] = follow;
    }
    apply_pending(app, &hub);
    for n in 0..app.deck_count().min(4) as usize {
        app.midi.last_tempo[n] = app.deck_of(n as u8).x.tempo_adj;
    }
    finish_double(app);
    for (u, held) in app.midi.tap_held.iter_mut().enumerate() {
        if held.is_some_and(|t| t.elapsed() >= Duration::from_secs(1)) {
            *held = None;
            app.px.fx[u].auto = true;
        }
    }
    if have {
        *hub.snap.lock() = snapshot(app);
        leds(app, &hub);
        hub.engine_outputs();
    }
}

fn apply_pending(app: &mut App, hub: &Arc<Hub>) {
    let events: Vec<UiEvent> = app.midi.rx.as_ref().map(|r| r.try_iter().collect()).unwrap_or_default();
    for ev in events {
        match ev {
            UiEvent::Ports { removed, paused } => {
                if paused.iter().any(|name| !app.midi.paused_inputs.contains(name)) {
                    app.toast_err(format!("MIDI input paused for controllers with conflicting port names ({}). Disconnect one or give their inputs distinct names.", paused.join(", ")));
                }
                app.midi.paused_inputs = paused;
                app.midi.devices = hub.devices.lock().iter().map(|d| d.name.clone()).collect();
                crate::ui_midi::devices_changed(app, &removed);
            }
            UiEvent::Learn { device, msg } => {
                // Keep a same-name hotplug from replacing the selected device while its
                // physical input is being committed to the map.
                let _scan = hub.paused_input_names.lock();
                if let Some(d) = device.upgrade().filter(|d| d.alive.load(Relaxed) && d.learning.load(Relaxed)) {
                    crate::ui_midi::learn_message(app, &d.name, msg);
                }
            }
            UiEvent::Control { device, tag, deck, val, fast_done, seq } => {
                if let Some(f) = registry().get(tag) {
                    app.midi.handled += 1;
                    apply(app, hub, &device, f, deck, val, fast_done, seq);
                }
            }
        }
    }
}

/// Called at the end of `push_deck_controls`: the values a worker set since the interface
/// last looked are put back over what the interface just wrote, and the jog's bend is added.
pub fn after_push(app: &mut App) {
    let Some(hub) = app.midi.hub.as_ref() else { return };
    let pending: Vec<(u8, Param, f32, u64)> = hub.overlay.lock().clone();
    for (n, p, v, _) in pending {
        hub.set_param(n as usize, p, v);
    }
    for n in 0..4 {
        let b = f64::from_bits(hub.jog_bend[n].load(Relaxed));
        if b != 0.0 {
            hub.decks[n].lock().bend += b;
        }
    }
}

/// The app's deck for a mapping's deck number.
fn deck_of(app: &App, deck: u8) -> Option<u8> {
    let n = if deck == 0 { app.px.active } else { deck - 1 };
    (n < app.deck_count()).then_some(n)
}

fn browse_step(app: &mut App, n: i32) {
    if n == 0 {
        return;
    }
    if app.dialogs.ext.focus_tree {
        for _ in 0..n.abs().min(50) {
            crate::prefs_ext::tree_step(app, n.signum());
        }
        return;
    }
    if app.list_dirty {
        app.rebuild_list();
    }
    if app.list.is_empty() {
        return;
    }
    let next = match app.anchor {
        Some(cur) => (cur as i64 + n as i64).clamp(0, app.list.len() as i64 - 1) as usize,
        None => 0,
    };
    app.set_anchor(next);
    app.selected.clear();
    app.selected.insert(app.list[next]);
    app.scroll_to_selected = true;
}

/// Instant Doubles: the track of another deck comes onto this one at the same place.
fn inst_doubles(app: &mut App, n: u8) {
    if app.deck_options.disable_load { return; }
    let count = app.deck_count();
    let playing = |app: &App, k: u8| app.player.decks[k as usize].lock().playing;
    let src = (0..count).filter(|k| *k != n && app.deck_of(*k).track.is_some()).max_by_key(|k| playing(app, *k));
    let Some(src) = src else { return };
    let Some(id) = app.deck_of(src).track else { return };
    app.with_deck(n, |a| a.load_track_here(id));
    app.midi.pending_double = Some((n, src));
}

fn finish_double(app: &mut App) {
    let Some((n, src)) = app.midi.pending_double else { return };
    if app.deck_of(n).loading.is_some() {
        return;
    }
    app.midi.pending_double = None;
    if app.deck_of(n).audio.is_none() || app.deck_of(n).track != app.deck_of(src).track {
        return;
    }
    let (pos, playing) = {
        let o = app.player.decks[src as usize].lock();
        (o.pos, o.playing)
    };
    let (adj, mt) = (app.deck_of(src).x.tempo_adj, app.deck_of(src).x.master_tempo);
    let d = app.deck_of_mut(n);
    d.x.tempo_adj = adj;
    d.x.master_tempo = mt;
    {
        let mut s = app.player.decks[n as usize].lock();
        s.pos = pos;
        s.playing = playing;
    }
    app.push_deck_controls();
}

/// What a mapped message does, on the interface thread. `fast_done`: the worker already did
/// the engine side.
#[allow(clippy::too_many_arguments)]
pub fn apply(app: &mut App, hub: &Arc<Hub>, device: &str, f: &Func, deck: u8, val: Val, fast_done: bool, seq: u64) {
    let press = matches!(val, Val::On(true, _));
    let release = matches!(val, Val::On(false, _));
    let rel = if let Val::Rel(n) = val { n } else { 0 };
    let unit = val.unit();
    // Controllers act in PERFORMANCE mode; browsing works in both.
    if !app.performance() && !routing::browsing(f.act) {
        return;
    }
    let Some(n) = deck_of(app, deck) else { return };
    let ni = n as usize;
    // An absolute value a worker already put into the engine: the app's own value follows.
    let settle = |app: &mut App| {
        hub.overlay.lock().retain(|o| o.3 > seq);
        let _ = app;
    };
    match f.act {
        Act::PlayPause => {
            if press {
                if fast_done {
                    app.with_deck(n, |a| {
                        a.deck.cue_preview = false;
                        if a.playing() {
                            a.sync_align();
                        }
                    });
                } else {
                    hub.play_pause(ni);
                    app.with_deck(n, |a| if a.playing() { a.sync_align() });
                }
            }
        }
        Act::Cue => {
            if (press || release) && (fast_done || hub.cue(ni, press)) {
                return;
            }
            if press {
                // The deck stands away from the cue point: the cue point is set here.
                app.with_deck(n, |a| {
                    if a.deck.audio.is_some() && !a.playing() {
                        let q = a.quantized(a.pos());
                        a.deck.cue_point = q;
                        a.seek(q);
                    }
                });
            }
        }
        Act::HotCue(slot) => {
            if (press || release) && !fast_done && !hub.hot_cue(ni, slot as usize, press) && press {
                // An empty slot: the hot cue (or the running loop) is stored there.
                app.with_deck(n, |a| a.hot_cue_down(slot as i32));
            } else if press {
                // (The loop a hot loop started is the deck's last loop.)
                if let Some(c) = app.deck_of(n).cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32).cloned() {
                    if let Some(e) = c.end.filter(|e| *e > c.start) {
                        app.deck_of_mut(n).last_loop = Some((c.start, e));
                    }
                }
            }
        }
        Act::HotCueDelete(slot) => {
            if press {
                app.with_deck(n, |a| {
                    if let Some(id) = a.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32).map(|c| c.id) {
                        a.delete_cue(id);
                    }
                });
            }
        }
        Act::TrackStart => {
            if press {
                app.with_deck(n, |a| a.seek(0.0));
            }
        }
        Act::Track(dir) => {
            if press {
                app.with_deck(n, |a| if dir > 0 { a.load_adjacent(1) } else { a.previous_track() });
            }
        }
        Act::Search(dir) => {
            if press {
                app.with_deck(n, |a| a.search(dir));
            } else if release {
                app.with_deck(n, |a| if a.searching() == dir { a.search(0) });
            }
        }
        Act::Needle => {
            if let Some(p) = unit {
                let lock = app.prefs.midi.needle_lock;
                app.with_deck(n, |a| {
                    if lock && a.playing() {
                        a.toast_err("Needle Lock. Not available during playback.");
                    } else if a.deck.audio.is_some() {
                        let t = p as f64 * a.deck.duration;
                        a.seek(t);
                    }
                });
            }
        }
        Act::Slip => {
            if press {
                app.with_deck(n, |a| a.toggle_slip());
            }
        }
        Act::Quantize => {
            if press {
                app.with_deck(n, |a| a.prefs.quantize = !a.prefs.quantize);
                app.save_prefs();
            }
        }
        Act::QuantizeAll => {
            if press {
                let on = !app.prefs.quantize;
                app.prefs.quantize = on;
                app.px.parked_quantize = [on; 4];
                app.save_prefs();
            }
        }
        Act::MasterTempo => {
            if press {
                app.with_deck(n, |a| a.toggle_master_tempo());
            }
        }
        Act::Tempo => {
            let Val::Abs { .. } = val else { return };
            let adj = if fast_done {
                let a = hub.decks[ni].lock().tempo - 1.0;
                (hub.takeover_state(ni).state == 5 || !app.deck_of(n).audio.is_some()).then_some(a)
            } else {
                let raw = if unit == Some(0.5) { 8192 } else { (unit.unwrap_or(0.5) * 16383.0).round() as i32 };
                hub.tempo_slider(ni, raw)
            };
            if let Some(adj) = adj {
                if !app.with_deck(n, |a| a.sync_follower()) {
                    app.deck_of_mut(n).x.tempo_adj = adj;
                    app.midi.last_tempo[ni] = adj;
                }
            }
            settle(app);
        }
        Act::TempoRange => {
            if press {
                app.with_deck(n, |a| a.tempo_range_next());
            }
        }
        Act::TempoReset => {
            if press {
                app.with_deck(n, |a| a.tempo_reset());
            }
        }
        Act::TempoLed(_) => {}
        Act::Sync => {
            if press {
                app.with_deck(n, |a| a.toggle_beat_sync());
            }
        }
        Act::Master => {
            if press {
                app.with_deck(n, |a| a.set_master());
            }
        }
        Act::TapBpm => {
            if press {
                app.with_deck(n, |a| a.grid_tap());
            }
        }
        Act::PitchBend(dir) => {
            if press {
                app.with_deck(n, |a| a.pitch_bend(dir));
            } else if release {
                app.with_deck(n, |a| a.pitch_bend(0));
            }
        }
        Act::AutoLoop | Act::Loop4 => {
            if press {
                app.with_deck(n, |a| {
                    if a.loop_active().is_some() {
                        a.loop_exit();
                    } else {
                        let b = if f.act == Act::Loop4 { 4.0 } else { crate::prefs_bridge::auto_beat_loop_beats(&a.prefs) };
                        a.auto_loop(b);
                    }
                });
            }
        }
        Act::LoopResize(x) => {
            if press {
                app.with_deck(n, |a| a.resize_loop(x));
            }
        }
        Act::LoopSize => {
            if rel != 0 {
                app.with_deck(n, |a| a.resize_loop(if rel > 0 { 2.0 } else { 0.5 }));
            }
        }
        Act::LoopIn => {
            if press {
                app.with_deck(n, |a| a.loop_in_pressed());
            }
        }
        Act::LoopOut => {
            if press {
                app.with_deck(n, |a| a.loop_out_pressed());
            }
        }
        Act::Reloop => {
            if press {
                app.with_deck(n, |a| a.exit_reloop());
            }
        }
        Act::ReTrigger => {
            if press {
                app.with_deck(n, |a| match a.loop_active() {
                    Some((s, _)) => a.seek(s),
                    None => a.reloop(),
                });
            }
        }
        Act::BeatLoop(beats) => {
            if press {
                app.with_deck(n, |a| beat_loop(a, beats));
            }
        }
        Act::BeatLoopPad(i) => {
            if press {
                let beats = crate::player_ext::LOOP_SIZES[(i as usize).min(15)];
                app.with_deck(n, |a| beat_loop(a, beats));
            }
        }
        Act::GridHalfDouble(double) => {
            if press {
                app.with_deck(n, |a| a.grid_double_half(double));
            }
        }
        Act::KeyShift(dir) => {
            if press {
                app.with_deck(n, |a| a.key_shift_step(dir));
            }
        }
        Act::KeySync => {
            if press {
                app.with_deck(n, |a| a.toggle_key_sync());
            }
        }
        Act::KeyReset => {
            if press {
                app.with_deck(n, |a| a.key_reset());
            }
        }
        Act::StemSolo => {
            if press {
                app.prefs.stem_solo = !app.prefs.stem_solo;
                app.save_prefs();
            }
        }
        Act::PadMode(m) => {
            if press {
                app.px.pad_mode[ni] = m;
            }
        }
        Act::MemoryStep(dir) => {
            if press {
                app.with_deck(n, |a| a.memory_jump(dir));
            }
        }
        Act::MemorySet => {
            if press {
                app.with_deck(n, |a| a.memory_set());
            }
        }
        Act::MemoryDelete => {
            if press {
                app.with_deck(n, |a| a.memory_delete_current());
            }
        }
        Act::BeatJumpPad(i) => {
            if press {
                // rekordbox's cells: FINE, 1/8 ... 128, each back and forward; the page
                // chooses the first cell on the pads (page 5: 1 beat on pads 1 and 2).
                const CELLS: [f64; 12] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];
                let first = (2 * app.midi.jump_page[ni] as i32 - 2).clamp(0, 16) as usize;
                let id = first + i as usize;
                if let Some(b) = CELLS.get(id / 2) {
                    let beats = if id % 2 == 0 { -*b } else { *b };
                    app.with_deck(n, |a| a.beat_jump(beats));
                }
            }
        }
        Act::BeatJumpPage(dir) => {
            if press {
                app.midi.jump_page[ni] = (app.midi.jump_page[ni] as i32 + dir).clamp(1, 9) as u8;
            }
        }
        Act::StemPart(src) => {
            if press {
                // (With three parts shown, INST stands for INST and BASS, as on screen.)
                let srcs: Vec<usize> = if app.prefs.stem_parts == 3 && src == 2 { vec![2, 1] } else { vec![src as usize] };
                app.with_deck(n, |a| {
                    if a.deck.track.is_some() {
                        crate::prefs_ext::stem_part_click(a, &srcs);
                    }
                });
            }
        }
        Act::StemFx(row) => {
            if press {
                let r = (row as usize).min(3);
                app.px.stem_fx[r] = !app.px.stem_fx[r];
            }
        }
        Act::FxLevel(u, s) => {
            if let Some(p) = unit {
                let fx = &mut app.px.fx[u as usize % 2];
                if fx.params.single {
                    if s == 0 {
                        fx.params.one.level = p;
                    }
                } else {
                    fx.params.slots[s as usize % 3].level = p;
                }
            }
        }
        Act::FxOn(u, s) => {
            if press {
                let fx = &mut app.px.fx[u as usize % 2];
                if fx.params.single {
                    if s == 0 {
                        fx.params.one.on = !fx.params.one.on;
                    }
                } else {
                    let sl = &mut fx.params.slots[s as usize % 3];
                    sl.on = !sl.on;
                }
            }
        }
        Act::FxSelect(u, s, dir) => {
            // (A Rotary row steps the list by its direction, one effect per message.)
            let dir = if rel != 0 { rel.signum() } else { dir };
            if press || rel != 0 {
                let fx = &mut app.px.fx[u as usize % 2];
                let single = fx.params.single;
                if single && s != 0 {
                    return;
                }
                let slot = if single { 3 } else { s as usize % 3 };
                let cur = if single { fx.params.one.kind } else { fx.params.slots[slot].kind };
                let list: Vec<crate::fx::Kind> = crate::fx::list(single).map(|i| i.kind).collect();
                if let Some(i) = list.iter().position(|k| *k == cur) {
                    let k = list[(i as i32 + dir).rem_euclid(list.len() as i32) as usize];
                    fx.set_kind(slot, k);
                }
            }
        }
        Act::FxBeat(u, s, dir) => {
            if press {
                fx_beat(&mut app.px.fx[u as usize % 2], Some(s as usize), dir);
            }
        }
        Act::FxUnitBeat(u, dir) => {
            if press {
                fx_beat(&mut app.px.fx[u as usize % 2], None, dir);
            }
        }
        Act::FxUnitBeatRot(u) => {
            if rel != 0 {
                fx_beat(&mut app.px.fx[u as usize % 2], None, rel.signum());
            } else if let Some(p) = unit {
                // A knob (`RbxFxDeviceComponent::eventAbsoluteValueChanged` 0x1005fe09c): its
                // position across the beat values (`setEffectBeatArea(v x 0.000061039)`), the
                // unit's effect in single mode, all three slots in multi mode.
                let fx = &mut app.px.fx[u as usize % 2];
                let slots = if fx.params.single { 3..4 } else { 0..3 };
                for sl in slots {
                    let kind = if sl < 3 { fx.params.slots[sl].kind } else { fx.params.one.kind };
                    let len = crate::fx::beat_table(kind).0.len();
                    if len != 0 {
                        fx.set_index(sl, ((p as f64 * len as f64) as usize).min(len - 1));
                    }
                }
            }
        }
        Act::FxMode(u) => {
            if press {
                let p = &mut app.px.fx[u as usize % 2].params;
                p.single = !p.single;
            }
        }
        Act::FxAuto(u) => {
            if press {
                app.px.fx[u as usize % 2].auto = true;
            }
        }
        Act::FxTap(u) => {
            if press {
                let now = app.ctx.input(|i| i.time);
                let fx = &mut app.px.fx[u as usize % 2];
                fx.auto = false;
                if let Some(bpm) = fx.tap.tap(now) {
                    fx.tap_bpm = bpm;
                }
            }
        }
        Act::FxReleaseType(u) => {
            if rel != 0 {
                let fx = &mut app.px.fx[u as usize % 2];
                fx.release_kind = (fx.release_kind as i32 + rel.signum()).rem_euclid(3) as u8;
            }
        }
        Act::FxReleaseBeat(u, dir) => {
            if press {
                let fx = &mut app.px.fx[u as usize % 2];
                let k = fx.release_kind as usize % 3;
                fx.release_idx[k] = (fx.release_idx[k] as i32 + dir).clamp(0, crate::fx::RELEASE_BEATS.len() as i32 - 1) as usize;
            }
        }
        Act::FxReleaseOn(u) => {
            if press || release {
                let fx = &mut app.px.fx[u as usize % 2];
                let k = fx.release_kind as usize % 3;
                fx.params.release = press.then_some((k as u8, crate::fx::RELEASE_BEATS[fx.release_idx[k].min(crate::fx::RELEASE_BEATS.len() - 1)]));
            }
        }
        Act::FxAssign(u, ch) => {
            if press {
                let a = &mut app.px.fx[u as usize % 2].params.assign[(ch as usize).min(crate::fx::CHANNELS - 1)];
                *a = !*a;
            }
        }
        Act::CfxColor(ch) => {
            if let Some(p) = unit {
                if ch as usize == crate::fx::SAMPLER {
                    app.px.sampler.cfx_color = p;
                } else {
                    app.px.cfx_color[(ch as usize).min(4)] = p;
                }
            }
        }
        Act::CfxSelect(dir) => {
            if press {
                let list: Vec<u8> = crate::cfx::LIST.iter().filter_map(|x| x.1).collect();
                let cur = if app.px.cfx_user { app.px.cfx_kind } else { 0 };
                let i = list.iter().position(|k| *k == cur).unwrap_or(0) as i32;
                let k = list[(i + dir).rem_euclid(list.len() as i32) as usize];
                app.px.cfx_user = true;
                app.px.cfx_kind = k;
                app.px.cfx_param = app.px.fxx.cfx_params[k as usize % 9];
            }
        }
        Act::SamplerPad(pad, velocity) => {
            let Val::On(on, v) = val else { return };
            if !fast_done {
                let pad = pad as usize + if n % 2 == 1 && app.sampler_pads() > 8 { 8 } else { 0 };
                if on {
                    let gain = if velocity { crate::sampler_ext::velocity_gain(app.prefs.sampler.velocity_curve, v) } else { 1.0 };
                    app.sampler_pad_down(pad, gain, false, false);
                } else {
                    app.sampler_pad_up(pad);
                }
            }
        }
        Act::SamplerPadStop(pad) => {
            if press {
                let pad = pad as usize + if n % 2 == 1 && app.sampler_pads() > 8 { 8 } else { 0 };
                app.sampler_pad_down(pad, 1.0, true, false);
            }
        }
        Act::SamplerBank(dir) => {
            if press {
                let b = (app.prefs.sampler.bank as i32 + dir).clamp(0, 3) as u8;
                app.sampler_action(crate::sampler_ext::SmpAct::Bank(b));
            }
        }
        Act::SamplerGain => {
            if let Some(p) = unit {
                app.prefs.sampler.gain = p * 100.0;
            }
        }
        Act::SamplerVolume => {
            if let Some(p) = unit {
                app.prefs.sampler.volume = p * 100.0;
            } else if rel != 0 {
                // (An encoder, as the DDJ-ERGO has: one percent a step; not traced.)
                app.prefs.sampler.volume = (app.prefs.sampler.volume + rel as f32).clamp(0.0, 100.0);
            }
        }
        Act::SamplerCue => {
            if press {
                app.prefs.sampler.cue = !app.prefs.sampler.cue;
            }
        }
        Act::Smp(a) => {
            if press {
                let a = match a {
                    crate::sampler_ext::SmpAct::SeqLoad(_) => crate::sampler_ext::SmpAct::SeqLoad(n),
                    a => a,
                };
                app.sampler_action(a);
            }
        }
        Act::SeqCall(slot, stop) => {
            if press {
                app.sampler_seq_call(slot as usize, stop);
            }
        }
        Act::Trim => {
            if let Some(p) = unit {
                app.px.mix[ni].trim = p;
            }
            settle(app);
        }
        Act::Eq(b) => {
            if let Some(p) = unit {
                let b = (b as usize).min(2);
                // (A waiting pick-up leaves the on-screen knob where it is.)
                let applied = if fast_done { !hub.pickup.lock()[ni][b].waiting } else { hub.set_param(ni, Param::Eq(b as u8), p) };
                if applied {
                    let m = &mut app.px.mix[ni];
                    if m.stem_iso_on { m.stem_iso[b] = p } else { m.eq[b] = p }
                }
            }
            settle(app);
        }
        Act::StemIsoKnob(i) => {
            if let Some(p) = unit {
                app.px.mix[ni].stem_iso[(i as usize).min(2)] = p;
            }
        }
        Act::HeadCue => {
            if press {
                app.px.mix[ni].cue = !app.px.mix[ni].cue;
            }
        }
        Act::ChFader => {
            if let Some(p) = unit {
                app.px.mix[ni].fader = p;
            }
            settle(app);
        }
        Act::HpMix => {
            if let Some(p) = unit {
                app.px.hp_mix = p;
            }
        }
        Act::HpLevel => {
            if let Some(p) = unit {
                app.px.hp_level = p;
            }
        }
        Act::CrossFader => {
            if let Some(p) = unit {
                app.px.crossfader = p;
            }
            settle(app);
        }
        Act::XfAssign(side) => {
            if press {
                app.px.mix[ni].assign = side;
            }
        }
        Act::MasterLevel => {
            if let (Some(p), false) = (unit, fast_done) {
                app.player.state.lock().volume = p;
            }
        }
        Act::StemIso => {
            if press {
                app.px.mix[ni].stem_iso_on = !app.px.mix[ni].stem_iso_on;
            }
        }
        Act::Browse => browse_step(app, rel),
        Act::BrowseStep(d) => {
            if press {
                browse_step(app, d);
            }
        }
        Act::Forward => {
            if press {
                // Into the folder, then on to the track list.
                if app.dialogs.ext.focus_tree {
                    crate::prefs_ext::tree_expand(app, true);
                }
                app.dialogs.ext.focus_tree = false;
            }
        }
        Act::Back => {
            if press {
                if app.dialogs.ext.focus_tree {
                    crate::prefs_ext::tree_expand(app, false);
                }
                app.dialogs.ext.focus_tree = true;
            }
        }
        Act::Area => {
            if press {
                app.dialogs.ext.focus_tree = !app.dialogs.ext.focus_tree;
            }
        }
        Act::TagList => {
            if press {
                if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
                    app.tag_list_add(&[id]);
                }
            }
        }
        Act::Related => {
            if press {
                let v = if app.view == View::Related { View::Collection } else { View::Related };
                app.set_view(v);
            }
        }
        Act::Sort(col) => {
            if press {
                if app.prefs.sort_col == col {
                    app.prefs.sort_asc = !app.prefs.sort_asc;
                } else {
                    app.prefs.sort_col = col;
                    app.prefs.sort_asc = true;
                }
                app.list_dirty = true;
                app.save_prefs();
            }
        }
        Act::Load => {
            if press && !app.deck_options.disable_load {
                // The DDJ-400 and DDJ-FLX4: a second press within 300 ms is Instant Doubles.
                let double = (device.contains("DDJ-400") || device.contains("DDJ-FLX4"))
                    && hub.device(device).map(|d| {
                        let mut at = d.load_at.lock();
                        let hit = at[ni].map(|t| t.elapsed() < Duration::from_millis(300)).unwrap_or(false);
                        at[ni] = if hit { None } else { Some(Instant::now()) };
                        hit
                    }).unwrap_or(false);
                if double {
                    inst_doubles(app, n);
                } else {
                    if app.list_dirty {
                        app.rebuild_list();
                    }
                    if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
                        app.with_deck(n, |a| a.load_track_here(id));
                    }
                }
            }
        }
        Act::InstDoubles => {
            if press {
                inst_doubles(app, n);
            }
        }
        Act::Panel(p) => {
            if press {
                if p == 0 {
                    app.prefs.perf_fx = !app.prefs.perf_fx;
                } else {
                    app.prefs.perf_sampler = !app.prefs.perf_sampler;
                }
                app.save_prefs();
            }
        }
        Act::Zoom => {
            if rel != 0 {
                let z = (app.prefs.zoom as i32 - rel.signum()).clamp(0, ZOOMS.len() as i32 - 1);
                app.prefs.zoom = z as usize;
            }
        }
        Act::Shift => {
            if press || release {
                hub.shift[ni].store(press, Relaxed);
            }
        }
        // The jog is the worker's alone.
        Act::Jog(_) | Act::JogSearch | Act::JogTouch => {}
        Act::LoopMoveRot => {
            if rel != 0 {
                app.with_deck(n, |a| a.loop_move(rel.signum()));
            }
        }
        Act::PanelStep(dir) => {
            if press {
                panel_step(app, dir);
            }
        }
        Act::BrowseView => {
            if press && app.performance() {
                if app.prefs.perf_lay.browse {
                    app.prefs.perf_lay.browse = false;
                    app.poll_decks();
                    app.save_prefs();
                } else {
                    crate::ui_perf_layouts::set_row(app, 4);
                }
            }
        }
        Act::CallStep(dir) => {
            if press {
                app.with_deck(n, |a| {
                    if a.loop_active().is_some() {
                        a.resize_loop(if dir > 0 { 2.0 } else { 0.5 });
                    } else {
                        a.memory_jump(dir);
                    }
                });
            }
        }
        Act::VinylAdjust(which) => {
            if let Some(p) = unit {
                vinyl_adjust(app, ni % 2, which, (p * 16383.0).round() as i32);
            }
        }
        Act::FxTapLong(u) => {
            let u = u as usize % 2;
            if press {
                let now = app.ctx.input(|i| i.time);
                let fx = &mut app.px.fx[u];
                fx.auto = false;
                if let Some(bpm) = fx.tap.tap(now) {
                    fx.tap_bpm = bpm;
                }
                app.midi.tap_held[u] = Some(Instant::now());
                app.ctx.request_repaint_after(Duration::from_secs(1));
            } else if release {
                app.midi.tap_held[u] = None;
            }
        }
        Act::FxSelectKind(u, sl, i) => {
            if press {
                let fx = &mut app.px.fx[u as usize % 2];
                let single = fx.params.single;
                let k = crate::fx::INFO[(i as usize).min(crate::fx::INFO.len() - 1)].kind;
                // (Only an effect the mode's list has; in single mode the unit's one effect.)
                if crate::fx::list(single).any(|x| x.kind == k) && (!single || sl == 0) {
                    fx.set_kind(if single { 3 } else { sl as usize % 3 }, k);
                }
            }
        }
        Act::XfAssignKnob => {
            if let Some(p) = unit {
                app.px.mix[ni].assign = if p < 0.25 { 1 } else if p > 0.75 { 2 } else { 0 };
            }
        }
        Act::CfxCenter(ch) => {
            if press {
                app.px.cfx_color[(ch as usize).min(4)] = 0.5;
            }
        }
        Act::FxAssignHold(u, ch) => {
            if press || release {
                fx_assign_set(&mut app.px.fx[u as usize % 2].params.assign, ch as usize, press);
            }
        }
        Act::Stub(s) => stub(app, hub, s, n, val),
    }
}

/// PanelSelect Next / Back (`MainComponent::eventOnChanged`, PanelForward / PanelBack): the
/// panels in rekordbox's order FX, SAMPLER, MIXER, REC (VIDEO is not in recordbox), then
/// none; the one after (or before) the last one showing is shown alone. Panels the layout
/// does not have are left as they are and passed over.
fn panel_step(app: &mut App, dir: i32) {
    use crate::ui_perf_layouts::{mixer_shown, panel_enabled};
    // (Tile numbers of `panel_enabled`: 0 FX, 2 Sampler, 3 Mixer, 4 Rec.)
    let tiles = [0usize, 2, 3, 4];
    let on = [app.prefs.perf_fx, app.prefs.perf_sampler, mixer_shown(app), app.prefs.rec.panel];
    let enabled: [bool; 4] = std::array::from_fn(|i| panel_enabled(app, tiles[i]));
    let pick = panel_next(on, enabled, dir);
    let want = |i: usize| pick == Some(i);
    if enabled[0] {
        app.prefs.perf_fx = want(0);
    }
    if enabled[1] {
        app.prefs.perf_sampler = want(1);
    }
    if enabled[2] && mixer_shown(app) != want(2) {
        crate::ui_perf_layouts::toggle_mixer(app);
    }
    if enabled[3] {
        app.prefs.rec.panel = want(3);
    }
    app.save_prefs();
}

/// The panel PanelForward (`dir` 1) or PanelBack (-1) shows next, of FX, SAMPLER, MIXER,
/// REC (None: none). The current one is the last one lit.
pub fn panel_next(on: [bool; 4], enabled: [bool; 4], dir: i32) -> Option<usize> {
    let cur = (0..4).rev().find(|i| on[*i] && enabled[*i]);
    if dir == 0 { return cur; }
    let mut next: i32 = match (dir > 0, cur) {
        (true, Some(c)) => c as i32 + 1,
        (true, None) => 0,
        (false, Some(c)) => c as i32 - 1,
        (false, None) => 3,
    };
    while (0..4).contains(&next) && !enabled[next as usize] {
        next += dir.signum();
    }
    (0..4).contains(&next).then_some(next as usize)
}

/// The speed table index a Vinyl Speed Adjust knob position (0..16383) gives.
pub fn vinyl_index(v: i32) -> u8 {
    (((v.max(0) as f64) * 0.0155649149) as i32).clamp(0, 255) as u8
}

/// A Vinyl Speed Adjust knob (`UiPlayer::deviceVinylAdjust` 0x102029430): the knob's 14-bit
/// position times 0.0155649149 (0..255) is the speed table index; 0 sets Touch/Brake, 1
/// Release/Start, 2 both. Kept per side, as Preferences keeps them.
fn vinyl_adjust(app: &mut App, side: usize, which: u8, v: i32) {
    let idx = vinyl_index(v);
    let side = side.min(1);
    if which != 1 {
        app.prefs.rb.brake[side] = idx;
        app.prefs.deck.vinyl_brake[side] = idx;
    }
    if which != 0 {
        app.prefs.rb.start[side] = idx;
        app.prefs.deck.vinyl_start[side] = idx;
    }
    app.push_deck_controls();
}

/// An FX assign set to on or off as the on-screen button does it when it is clicked: the
/// master alone, or channels without the master.
pub fn fx_assign_set(a: &mut [bool; crate::fx::CHANNELS], c: usize, on: bool) {
    let c = c.min(crate::fx::CHANNELS - 1);
    if a[c] == on {
        return;
    }
    a[c] = on;
    if on {
        if c == crate::fx::MASTER {
            for (i, d) in a.iter_mut().enumerate() {
                if i != crate::fx::MASTER {
                    *d = false;
                }
            }
        } else {
            a[crate::fx::MASTER] = false;
        }
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

/// Beat value one step up or down: of a slot, or of the unit (single mode: its effect; multi
/// mode: all three slots).
fn fx_beat(fx: &mut crate::ui_fx::FxUi, slot: Option<usize>, dir: i32) {
    let step = |fx: &mut crate::ui_fx::FxUi, s: usize| {
        let cur = if s < 3 { fx.idx[s] } else { fx.one_idx } as i32;
        fx.set_index(s, (cur + dir).max(0) as usize);
    };
    match (fx.params.single, slot) {
        (true, Some(0)) | (true, None) => step(fx, 3),
        (true, _) => {}
        (false, Some(s)) => step(fx, s % 3),
        (false, None) => {
            for s in 0..3 {
                step(fx, s);
            }
        }
    }
}

/// The one place where the commands of features built elsewhere arrive.
/// A short per-session log records the most recent control changes.
pub fn stub(app: &mut App, hub: &Arc<Hub>, s: Stub, deck: u8, val: Val) {
    let press = matches!(val, Val::On(true, _));
    let release = matches!(val, Val::On(false, _));
    if !(press || release || val.unit().is_some() || matches!(val, Val::Rel(_))) {
        return;
    }
    let note = |app: &mut App, what: String| {
        app.midi.stub_log.push(what);
        let n = app.midi.stub_log.len();
        if n > 64 {
            app.midi.stub_log.drain(..n - 64);
        }
    };
    match s {
        // Recording: show / hide the REC panel.
        Stub::RecPanel => {
            if press {
                app.rec_toggle_panel();
                note(app, "RecPanel".into());
            }
        }
        // Automix: start / stop.
        Stub::AutoMixStartStop => {
            if press {
                app.automix_start_stop();
                note(app, "AutoMixStartStop".into());
            }
        }
        // MIX POINT LINK: 0 panel, 1 cue select next, 2 back, 3 set, 4 exit, 5 deck select, 6 silent.
        Stub::MixPointLink(cmd) => {
            if press {
                match cmd {
                    0 => app.mpl_toggle_panel(),
                    1 => app.mpl_select_cue(1),
                    2 => app.mpl_select_cue(-1),
                    3 => app.mpl_request_link(),
                    4 => {
                        for d in 0..4 {
                            app.mpl_cancel(d, 3);
                        }
                    }
                    5 => {
                        // Deck select: the deck's playhead becomes the point of the current mode.
                        let t = app.with_deck(deck, |a| a.pos());
                        app.mpl_wave_click(deck as usize, t, false);
                    }
                    _ => app.mpl_toggle_silent(),
                }
                note(app, format!("MixPointLink {cmd} deck {}", deck + 1));
            }
        }
        // PAD FX: pad mode of bank 1 / 2; a pad held; beat value up / down.
        Stub::PadFxMode(bank) => {
            if press {
                app.with_deck(deck, |a| a.set_pad_mode(crate::pads::PadMode::PadFx));
                note(app, format!("PadFxMode {bank} deck {}", deck + 1));
            }
        }
        Stub::PadFx(bank, pad) => {
            // (Pads 1..16 of a bank; the second bank's pads follow the first's.)
            let idx = ((bank.max(1) - 1) as u8) * 16 + pad as u8;
            if press {
                crate::ui_padfx::pad_down(app, deck as usize, idx);
            } else if release {
                crate::ui_padfx::pad_up(app, deck as usize, idx);
            }
            if press || release {
                note(app, format!("PadFx {bank} pad {} {} deck {}", pad + 1, if press { "on" } else { "off" }, deck + 1));
            }
        }
        Stub::PadFxBeat(bank, dir) => {
            if press {
                crate::ui_padfx::beat_shift(app, deck as usize, dir < 0);
                note(app, format!("PadFxBeat {bank} {dir:+} deck {}", deck + 1));
            }
        }
        // Slicer / Slicer Loop: pad mode, a section pad, shift, roll, length, capture.
        Stub::SlicerMode(lp) => {
            if press {
                app.with_deck(deck, |a| {
                    if a.set_pad_mode(crate::pads::PadMode::Slicer) && a.slicer_now().1 != lp {
                        a.slicer_toggle_loop();
                    }
                });
                note(app, format!("SlicerMode loop={lp} deck {}", deck + 1));
            }
        }
        Stub::SlicerPad(lp, pad) => {
            if press || release {
                app.with_deck(deck, |a| a.slicer_pad(pad as u8, press));
                note(app, format!("SlicerPad loop={lp} pad {} {} deck {}", pad + 1, if press { "on" } else { "off" }, deck + 1));
            }
        }
        Stub::SlicerShift(lp, dir) | Stub::SlicerRoll(lp, dir) | Stub::SlicerLength(lp, dir) => {
            if press {
                let n = (deck as usize).min(3);
                match s {
                    Stub::SlicerShift(..) => app.with_deck(deck, |a| a.slicer_shift(dir > 0)),
                    Stub::SlicerRoll(..) => {
                        let ri = (app.prefs.pads.slicer_roll[n] as i32 + dir.signum() as i32).max(0) as usize;
                        app.with_deck(deck, |a| a.slicer_set_roll(ri));
                    }
                    _ => {
                        let li = (app.prefs.pads.slicer_length[n] as i32 + dir.signum() as i32).clamp(0, 3) as usize;
                        app.with_deck(deck, |a| a.slicer_set_length(li));
                    }
                }
                note(app, format!("{s:?} loop={lp} {dir:+} deck {}", deck + 1));
            }
        }
        Stub::SlicerCapture => {
            if press {
                app.with_deck(deck, |a| {
                    a.slicer_capture();
                });
                note(app, format!("SlicerCapture deck {}", deck + 1));
            }
        }
        // Reverse (toggle) and Slip Reverse (while held).
        Stub::Reverse => {
            if press {
                app.with_deck(deck, |a| a.reverse_toggle(false));
                note(app, format!("Reverse deck {}", deck + 1));
            }
        }
        Stub::SlipReverse => {
            if press {
                app.with_deck(deck, |a| a.reverse_toggle(true));
            } else if release {
                app.with_deck(deck, |a| a.reverse_off());
            }
            if press || release {
                note(app, format!("SlipReverse {} deck {}", if press { "on" } else { "off" }, deck + 1));
            }
        }
        // Vinyl mode: the jog's switch follows already; the deck's VINYL button should too.
        Stub::Vinyl => {
            if press {
                let v = &hub.vinyl[(deck as usize).min(3)];
                v.store(!v.load(Relaxed), Relaxed);
                note(app, format!("Vinyl {} deck {}", if v.load(Relaxed) { "on" } else { "off" }, deck + 1));
            }
        }
        // Vinyl Speed Adjust (Touch/Brake) of the left / right decks: 0..1.
        Stub::VinylBrake(side) => {
            if let Some(p) = val.unit() {
                vinyl_adjust(app, side as usize, 0, (p * 16383.0).round() as i32);
                note(app, format!("VinylBrake side {side} {p:.3}"));
            }
        }
        Stub::AutoCue => {
            if press {
                crate::prefs_bridge::toggle_auto_cue(app, deck);
                note(app, format!("AutoCue deck {}", deck + 1));
            }
        }
        // Move the running loop one beat.
        Stub::LoopMove(dir) => {
            if press {
                let moved = app.with_deck(deck, |a| {
                    let had = a.loop_active().is_some();
                    a.loop_move(dir as i32);
                    had
                });
                note(app, format!("LoopMove {dir:+} deck {} ({})", deck + 1, if moved { "moved" } else { "no loop" }));
            }
        }
        // The memory loop at the cue point is made an active loop or not (`eventActiveLoop`).
        Stub::ActiveLoop => {
            if press {
                app.with_deck(deck, |a| a.toggle_cue_point_active_loop());
                note(app, format!("ActiveLoop deck {}", deck + 1));
            }
        }
        Stub::SyncRate => {
            if press {
                app.with_deck(deck, |a| a.sync_rate_toggle());
                note(app, format!("SyncRate deck {}", deck + 1));
            }
        }
    }
}

/// The lights of every connected controller, from the app's state.
fn leds(app: &mut App, hub: &Arc<Hub>) {
    let devs: Vec<Arc<Device>> = hub.devices.lock().iter().filter(|d| d.has_output).cloned().collect();
    if devs.is_empty() {
        return;
    }
    let reg = registry();
    let count = app.deck_count();
    // Each deck once.
    struct D {
        loaded: bool,
        playing: bool,
        at_cue: bool,
        looping: bool,
        slip: bool,
        searching: i32,
        reverse: bool,
    }
    let ds: Vec<D> = (0..4u8)
        .map(|n| {
            if n >= count {
                return D { loaded: false, playing: false, at_cue: false, looping: false, slip: false, searching: 0, reverse: false };
            }
            let cue = app.deck_of(n).cue_point;
            let s = app.player.decks[n as usize].lock();
            let pos = s.audio.as_ref().map(|a| s.pos / a.sr as f64);
            D { loaded: pos.is_some(), playing: s.playing, at_cue: pos.map(|p| (p - cue).abs() < 0.01).unwrap_or(false), looping: s.loop_range.is_some(), slip: s.slip, searching: if s.search > 0.0 { 1 } else if s.search < 0.0 { -1 } else { 0 }, reverse: s.dk.reverse }
        })
        .collect();
    let on = |b: bool| if b { Led::On } else { Led::Off };
    let (seq_playing, seq_rec, seq_cur) = crate::sampler::SAMPLER.try_lock().and_then(|g| g.as_ref().map(|e| (e.seq.playing, e.seq.rec, e.seq.current()))).unwrap_or((false, false, 0));
    for dev in devs {
        for (tag, deck) in dev.out_keys() {
            let Some(f) = reg.get(tag) else { continue };
            let Some(n) = deck_of(app, deck) else {
                dev.set_led(tag, deck, Led::Off);
                continue;
            };
            let ni = n as usize;
            let d = &ds[ni];
            let dk = app.deck_of(n);
            let m = app.px.mix[ni];
            let pad_mode = app.px.pad_mode[ni];
            let led = match f.act {
                Act::PlayPause => if d.playing { Led::On } else if d.loaded { Led::Blink } else { Led::Off },
                Act::Cue => if !d.loaded { Led::Off } else if d.playing || d.at_cue { Led::On } else { Led::Blink },
                Act::Search(dir) => on(d.searching == dir),
                Act::Slip => on(d.slip),
                Act::Quantize => on(if n == app.px.active { app.prefs.quantize } else { app.px.parked_quantize[ni] }),
                Act::QuantizeAll => on(app.prefs.quantize),
                Act::MasterTempo => on(dk.x.master_tempo),
                Act::Sync => on(dk.x.beat_sync),
                Act::Master => on(app.px.master == n && !app.sampler_is_master()),
                Act::KeySync => on(dk.x.key_sync),
                Act::TempoLed(dir) => {
                    let (down, up) = hub.takeover_state(ni).leds();
                    on(if dir < 0 { down } else { up })
                }
                Act::AutoLoop | Act::Loop4 | Act::Reloop | Act::LoopIn | Act::LoopOut => on(d.looping),
                Act::PitchBend(dir) => on(dk.x.bend == dir),
                Act::PadMode(mode) => on(pad_mode == mode),
                Act::HotCue(slot) | Act::HotCueDelete(slot) => match dk.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32) {
                    Some(c) => Led::Value(hot_cue_led(c.color, slot as i32, c.end.is_some(), app.prefs.pads.hot_cue_color)),
                    None => Led::Off,
                },
                Act::BeatJumpPad(_) | Act::BeatLoopPad(_) => on(d.loaded),
                Act::StemPart(src) => on(app.player.decks[ni].try_lock().map(|s| s.stem_on[(src as usize).min(3)]).unwrap_or(true)),
                Act::StemFx(row) => on(app.px.stem_fx[(row as usize).min(3)]),
                Act::StemSolo => on(app.prefs.stem_solo),
                Act::FxOn(u, s) => {
                    let p = &app.px.fx[u as usize % 2].params;
                    on(if p.single { s == 0 && p.one.on } else { p.slots[s as usize % 3].on })
                }
                Act::FxMode(u) => on(app.px.fx[u as usize % 2].params.single),
                Act::FxAuto(u) => on(app.px.fx[u as usize % 2].auto),
                Act::FxTap(u) => on(!app.px.fx[u as usize % 2].auto),
                Act::FxReleaseOn(u) => on(app.px.fx[u as usize % 2].params.release.is_some()),
                Act::FxAssign(u, ch) => on(app.px.fx[u as usize % 2].params.assign[(ch as usize).min(crate::fx::CHANNELS - 1)]),
                Act::SamplerPad(pad, _) | Act::SamplerPadStop(pad) => {
                    let p = pad as usize + if n % 2 == 1 && app.sampler_pads() > 8 { 8 } else { 0 };
                    on(app.sampler_loaded(app.prefs.sampler.bank.min(3) as usize * 16 + p))
                }
                Act::Smp(crate::sampler_ext::SmpAct::Sync) => on(app.prefs.sampler.sync),
                Act::Smp(crate::sampler_ext::SmpAct::Master) => on(app.px.sampler.master),
                Act::Smp(crate::sampler_ext::SmpAct::Quantize) => on(app.prefs.sampler.quantize),
                Act::Smp(crate::sampler_ext::SmpAct::SeqPlay) => on(seq_playing),
                Act::Smp(crate::sampler_ext::SmpAct::SeqRec) => if seq_rec && seq_playing { Led::On } else if seq_rec { Led::Blink } else { Led::Off },
                Act::SamplerCue => on(app.prefs.sampler.cue),
                Act::SeqCall(slot, _) => on(seq_playing && seq_cur == slot as usize),
                Act::HeadCue => on(m.cue),
                Act::XfAssign(side) => on(m.assign == side),
                Act::StemIso => on(m.stem_iso_on),
                Act::Panel(p) => on(if p == 0 { app.prefs.perf_fx } else { app.prefs.perf_sampler }),
                Act::Related => on(app.view == View::Related),
                Act::Stub(Stub::Vinyl) => on(hub.vinyl[ni].load(Relaxed)),
                // (Blinking while the deck plays backwards: `deviceUpdateLed` 0x10202a7f4.)
                Act::Stub(Stub::Reverse) => if d.reverse { Led::Blink } else { Led::Off },
                _ => continue,
            };
            dev.set_led(tag, deck, led);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learn_binds_what_a_controller_sends_and_names_rekordboxs_function() {
        // A button pressed and let go (Note On, then velocity 0), and a 14-bit fader (MSB, LSB).
        let msgs = vec![vec![0x90, 0x0B, 0x7F], vec![0x90, 0x0B, 0x00], vec![0xB0, 0x13, 0x40], vec![0xB0, 0x33, 0x00]];
        let rb = MidiMap::load("TEST", None, Some(b"@file,1,TEST\r\nPlayPause,,Button,,900B,910B,,,,,,,,,Play\r\n")).ok();
        let got = probe_learn("TEST", &msgs, rb.as_ref());
        // The release and the LSB are passed over, as rekordbox's Learn does.
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), vec![0x900B, 0xB013]);
        assert!(got.iter().all(|g| g.1), "both bind");
        assert_eq!(got[0].2, "PlayPause (deck 1)");
        assert_eq!(got[1].2, "-");
    }

    #[test]
    fn hot_cue_pad_lights_take_the_cue_colour_or_the_colour_set() {
        // A cue with a colour: that code.
        assert_eq!(hot_cue_led(42, 0, false, 4), 42);
        // Without one: the slot's colour in the set (COLORFUL A = 43, C = 23; CDJ I = 26).
        assert_eq!(hot_cue_led(0, 0, false, 4), 43);
        assert_eq!(hot_cue_led(0, 2, false, 4), 23);
        assert_eq!(hot_cue_led(0, 8, false, 1), 26);
        // A loop without a colour: rekordbox's loop entry (index 16 of the tables, 36).
        assert_eq!(hot_cue_led(0, 3, true, 2), crate::model::HOTCUE_LOOP_DEFAULT);
        // Codes above 64 are not sent.
        assert_eq!(hot_cue_led(70, 0, false, 4), 0);
    }

    #[test]
    fn registry_names_are_unique_per_tab_and_typed() {
        let r = registry();
        assert!(r.funcs.len() > 300, "{}", r.funcs.len());
        let mut seen = std::collections::HashSet::new();
        for f in &r.funcs {
            assert!(seen.insert(f.name.clone()), "{} twice", f.name);
            if f.tab != Tab::None {
                assert!(!f.display.is_empty(), "{}", f.name);
                assert!((1..=8).contains(&f.ty));
                // Pad rows are pads; pads of the catalogue are named PAD<n>_ or are stem parts.
                if is_pad_name(&f.name) {
                    assert_eq!(f.ty, T_PAD, "{}", f.name);
                }
            }
        }
        // rekordbox's names for a few of them.
        assert_eq!(r.by_name("TrackNext").unwrap().display, "TrackSearch Next");
        assert_eq!(r.by_name("PAD1_HotCue+Shift").unwrap().display, "HotCue1 Delete");
        assert_eq!(r.by_name("BeatLoop1_64").unwrap().display, "AutoBeatLoop 1/64");
        assert_eq!(r.by_name("FX2-3SelectBack").unwrap().display, "FX2-3 Select Back");
        assert_eq!(r.by_name("CrossfaderAssign.Thru").unwrap().act, Act::XfAssign(0));
        assert!(r.by_name("TempoSlider").unwrap().per_deck && !r.by_name("CrossFader").unwrap().per_deck);
        // Jog wheels and Shift are not offered by MIDI Learn.
        assert!(!r.learnable("JogScratch") && !r.learnable("Shift") && !r.learnable("Nonsense"));
        assert!(r.learnable("PlayPause") && r.learnable("MixPointLink"));
        // The tabs that have commands.
        for (tab, name) in TABS {
            let n = r.of_tab(tab).count();
            assert_eq!(n == 0, matches!(tab, Tab::Video | Tab::Lighting), "{name}: {n}");
        }
        // Types and their families.
        assert_eq!(type_choices(T_BUTTON), &[T_BUTTON, T_BUTTON_PAD]);
        assert_eq!(type_choices(T_KNOB14), &[T_KNOB7, T_KNOB14]);
        assert_eq!(type_choices(T_PAD), &[T_PAD]);
        assert_eq!(type_text(T_KNOB14), "Knob/Slider(0h-3FFFh)");
        assert_eq!(kind_of_type(T_KNOB14), Kind::KnobSliderHiRes);
    }

    #[test]
    fn devices_that_need_the_hardware_handshake_are_known() {
        for d in ["DDJ-400", "DDJ-1000", "DDJ-FLX6", "DJM-A9", "XDJ-XZ (1)", "Pioneer DJ XDJ-RX2", "PIONEER XDJ-RX"] {
            assert!(needs_auth(d), "{d}");
        }
        for d in ["DDJ-FLX4", "DDJ-FLX2", "DDJ-GRV6", "DDJ-REV5", "DDJ-FLX10", "XDJ-RX3", "PIONEER DDJ-SX2", "Launchpad Mini"] {
            assert!(!needs_auth(d), "{d}");
        }
    }

    #[test]
    fn browsing_and_closing_midi_learn_preserves_native_updates_and_empty_devices() {
        let temp = tempfile::tempdir().unwrap();
        let native = temp.path().join("native");
        let local = temp.path().join("recordbox");
        std::fs::create_dir(&native).unwrap();
        let contents = |code| format!("@file,1,Deck\r\nPlayPause,PlayPause,Button,{code:04X},,,,,,,,,,,\r\n");
        let native_file = native.join("Deck.midi.csv");
        std::fs::write(&native_file, contents(0x9010)).unwrap();
        let decks = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let (mut hub, _) = Hub::new(decks, local.clone(), true);
        let h = Arc::get_mut(&mut hub).unwrap();
        h.native_user_dirs = vec![native];
        h.factory_dirs.clear();
        for device in ["Deck", "Empty"] {
            let (map, factory) = h.load_map(device);
            h.devices.lock().push(Device::mapped(device, device.into(), map, factory, None));
        }
        let mut midi = MidiState { hub: Some(hub.clone()), ..Default::default() };
        midi.win.open = true;
        crate::ui_midi::select_device(&mut midi, "Deck");
        crate::ui_midi::select_device(&mut midi, "Empty");
        crate::ui_midi::close(&mut midi);
        assert!(!local.exists(), "read-only navigation must never create a delta");
        std::fs::write(native_file, contents(0x9011)).unwrap();
        let updated = hub.load_map("Deck").0;
        assert_eq!(updated.live_inputs().map(|i| i.e.code).collect::<Vec<_>>(), [0x9011]);
    }

    #[test]
    fn first_connection_reads_native_mappings_without_writing_and_local_edits_take_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let native = temp.path().join("native");
        let factory = temp.path().join("factory");
        let local = temp.path().join("recordbox");
        std::fs::create_dir(&native).unwrap();
        std::fs::create_dir(&factory).unwrap();
        let decks = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let (mut hub, _) = Hub::new(decks, local.clone(), true);
        let h = Arc::get_mut(&mut hub).unwrap();
        h.native_user_dirs = vec![native.clone()];
        h.factory_dirs = vec![factory.clone()];
        let port = "Deck";
        let name = "Deck.midi.csv";
        let data = |code| format!("@file,1,Deck\r\nPlayPause,PlayPause,Button,{code:04X},,,,,,,,,,,\r\n");
        let original = data(0x9011);
        std::fs::write(native.join(name), &original).unwrap();
        std::fs::write(factory.join(name), data(0x9001)).unwrap();
        let inputs = || h.load_map(port).0.live_inputs().map(|e| e.e.code).collect::<Vec<_>>();
        assert_eq!(inputs(), [0x9001, 0x9011]);
        assert!(!local.exists(), "opening a device must not create a local mapping");
        std::fs::create_dir(&local).unwrap();
        std::fs::write(local.join(name), crate::midi_csv::empty_user_file(port)).unwrap();
        assert_eq!(inputs(), [0x9001, 0x9011], "legacy empty artifact must not hide native edits");
        let edited = data(0x9022);
        h.write_override(port, edited.as_bytes()).unwrap();
        assert_eq!(inputs(), [0x9001, 0x9022]);
        assert!(h.write_override(port, b"invalid").is_err());
        assert!(h.write_override(port, b"@file,1,Wrong").is_err());
        assert!(h.write_override(port, b"@file,1,Deck\nbroken row").is_err());
        assert_eq!(std::fs::read(local.join(name)).unwrap(), edited.as_bytes());
        assert!(h.export_file(&native.join(name), b"bad").is_err());
        assert!(h.export_file(&factory.join(name), b"bad").is_err());
        let exported = temp.path().join("export.csv");
        h.export_file(&exported, edited.as_bytes()).unwrap();
        assert_eq!(std::fs::read(exported).unwrap(), edited.as_bytes());
        h.write_override(port, crate::midi_csv::empty_user_file(port).as_bytes()).unwrap();
        assert_eq!(inputs(), [0x9001], "explicit reset keeps only factory bindings across reconnects");
        assert_eq!(std::fs::read(native.join(name)).unwrap(), original.as_bytes());
    }

    #[test]
    fn alsa_reconnect_reads_previous_assignment_and_only_saves_to_stable_file() {
        let temp = tempfile::tempdir().unwrap();
        let decks = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let (mut hub, _) = Hub::new(decks, temp.path().to_owned(), true);
        let h = Arc::get_mut(&mut hub).unwrap();
        h.factory_dirs.clear();
        h.native_user_dirs.clear();
        let original = "Deck:Deck MIDI 1 24:0";
        let reconnected = "Deck:Deck MIDI 1 42:0";
        let csv = |port, code| format!("@file,1,{port}\r\nPlayPause,PlayPause,Button,{code:04X},,,,,,,,,,,\r\n");
        let old_path = temp.path().join(format!("{}.midi.csv", crate::midi_csv::legacy_stem(original)));
        let old_mapping = csv(original, 0x9012);
        std::fs::write(&old_path, &old_mapping).unwrap();
        let codes = |port| h.load_map(port).0.live_inputs().map(|i| i.e.code).collect::<Vec<_>>();
        assert_eq!(codes(reconnected), [0x9012]);
        assert!(codes("Deck:Deck MIDI 1 42:1").is_empty());
        assert!(!h.user_file(reconnected).exists());
        h.write_override(reconnected, csv(reconnected, 0x9013).as_bytes()).unwrap();
        assert_eq!(codes("Deck:Deck MIDI 1 51:0"), [0x9013]);
        assert_eq!(std::fs::read(old_path).unwrap(), old_mapping.as_bytes());
    }

    #[test]
    fn hires_feedback_keeps_controller_and_low_byte_order_and_resends_after_remap() {
        let csv = b"@file,1,Test\nJogPlayPauseIndicator,Jog,JogIndicatorHiRes,,,,,,,BB00,BB01,,,RO;Max=16383;,\nPlayPause,Play,IndicatorHiRes,,,,,,,B209,B309,,,,\n";
        let map = MidiMap::load("Test", None, Some(csv)).unwrap();
        let dev = Device::mapped("Test", "port".into(), map, false, None);
        let angle = |e: &Entry| crate::midi_map::jog_ring14(180, &e.opts);
        dev.set_named("JogPlayPauseIndicator", 1, angle);
        dev.set_named("JogPlayPauseIndicator", 1, angle);
        let mut out = dev.out.lock();
        assert_eq!(out.queue.pop(), Some([0xBB, 0, 63]));
        assert_eq!(out.queue.pop(), Some([0xBB, 0x10, 127]));
        assert_eq!(out.queue.pop(), None);
        drop(out);
        dev.reindex();
        dev.set_named("JogPlayPauseIndicator", 1, angle);
        assert_eq!(dev.out.lock().queue.len(), 2);
        let tag = registry().tag("PlayPause");
        dev.set_led(tag, 1, Led::On);
        let mut out = dev.out.lock();
        let packets: Vec<_> = std::iter::from_fn(|| out.queue.pop()).collect();
        assert_eq!(&packets[2..], &[[0xB2, 9, 127], [0xB2, 0x19, 127]]);
    }

    #[test]
    fn indicator_cadence_is_per_destination_deck_and_preserves_other_outputs() {
        let mut rows = NamedRows { jog_interval: Duration::from_millis(80), ..Default::default() };
        let now = Instant::now();
        assert!(rows.due(1, now));
        assert!(!rows.due(1, now + Duration::from_millis(79)));
        assert!(rows.due(2, now));
        assert!(rows.due(1, now + Duration::from_millis(80)));
    }

    #[test]
    fn the_new_functions_follow_rekordboxs_rules() {
        // Vinyl Speed Adjust: knob position x 0.0155649149.
        assert_eq!((vinyl_index(0), vinyl_index(8192), vinyl_index(16383)), (0, 127, 255));
        // FX assign following a switch: master alone.
        let mut a = [true, false, false, false, false, false];
        fx_assign_set(&mut a, crate::fx::MASTER, true);
        assert_eq!(a, [false, false, false, false, true, false]);
        fx_assign_set(&mut a, 1, true);
        assert_eq!(a, [false, true, false, false, false, false]);
        fx_assign_set(&mut a, 1, false);
        assert_eq!(a, [false; 6]);
        // PanelSelect: FX, SAMPLER, MIXER, REC, none, in turn; back the other way.
        let all = [true; 4];
        assert_eq!(panel_next([false; 4], all, 1), Some(0));
        assert_eq!(panel_next([true, false, false, false], all, 1), Some(1));
        assert_eq!(panel_next([true, true, true, false], all, 1), Some(3));
        assert_eq!(panel_next([false, false, false, true], all, 1), None);
        assert_eq!(panel_next([false; 4], all, -1), Some(3));
        assert_eq!(panel_next([true, false, false, false], all, -1), None);
        // A panel the layout does not have is passed over.
        assert_eq!(panel_next([true, false, false, false], [true, false, true, true], 1), Some(2));
        // Lights: a button with no state of its own is lit while held; REVERSE blinks.
        assert!(echo_led(Act::Load) && echo_led(Act::CallStep(1)) && echo_led(Act::Stub(Stub::SlipReverse)));
        assert!(!echo_led(Act::Sync) && act_has_led(Act::Stub(Stub::Reverse)));
    }

    fn tone(secs: f64) -> Arc<crate::audio::Decoded> {
        let n = (44100.0 * secs) as usize;
        Arc::new(crate::audio::Decoded { sr: 44100, samples: vec![0.0; n * 2], raw_frames: n, damaged: 0, truncated: false })
    }

    #[test]
    fn disabled_play_still_allows_seeking_to_a_hot_cue() {
        let decks = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let (hub, _) = Hub::new(decks, PathBuf::new(), false);
        let audio = tone(1.0);
        let sr = audio.sr as f64;
        hub.decks[0].lock().audio = Some(audio);
        hub.snap.lock().decks[0].hot[0] = Some((0.25, None));
        hub.play_pause(0);
        assert!(!hub.decks[0].lock().playing);
        assert!(hub.cue(0, true));
        assert!(!hub.decks[0].lock().playing);
        assert!(hub.hot_cue(0, 0, true));
        let deck = hub.decks[0].lock();
        assert_eq!(deck.pos, sr * 0.25);
        assert!(!deck.playing);
    }

    #[cfg(target_os = "macos")]
    fn wait(what: &str, mut ok: impl FnMut() -> bool) {
        let t0 = Instant::now();
        while !ok() {
            assert!(t0.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(super) fn virtual_controller_mapping(name: &str) -> String {
        // The controller's mapping, as a user file in rekordbox's format.
        format!(
            "@file,1,{name}\r\n\
             MidiOutInterval,MidiOutInterval,Parameter,FFF3,,,,,,,,,,Value=2;,\r\n\
             PlayPause,PlayPause,Button,,900B,910B,,,,900B,910B,,,Fast;Blink=1000;Priority=50;,\r\n\
             Cue,Cue,Button,,900C,910C,,,,900C,910C,,,Fast;Blink=500;Priority=50;,\r\n\
             PAD1_HotCue,PAD1_HotCue,Pad,,9700,9900,,,,9700,9900,,,Fast;,\r\n\
             ChannelFader,ChannelFader,KnobSliderHiRes,,B013,B113,,,,,,,,Fast;,\r\n\
             CrossFader,CrossFader,KnobSlider,B61F,,,,,,,,,,Fast;,\r\n\
             EQLow,EQLow,KnobSlider,,B00F,B10F,,,,,,,,Fast;,\r\n\
             TempoSlider,TempoSlider,KnobSliderHiRes,,B000,B100,,,,,,,,Fast;,\r\n\
             JogPitchBend,JogPitchBend,JogRotate,,B023,B123,,,,,,,,RO;,\r\n\
             Sync,Sync,Button,,9058,9158,,,,9058,9158,,,,\r\n\
             JogPlayPauseIndicatorShort,JogPlayPauseIndicatorShort,JogIndicator,,,,,,,BB00,BB01,,,RO;Min=1;Max=72;Priority=100;,\r\n\
             ChannelLevel,ChannelLevel,Indicator,,,,,,B002,0,1,,,RO;Priority=50;,\r\n"
        )
    }

    /// End to end over CoreMIDI: a virtual controller (a source that sends, a destination
    /// that receives the LEDs) is found by name, its user mapping is loaded, notes and CCs
    /// change the decks and the mixer in the engine without any interface frame, and the
    /// LED messages come back out.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_virtual_controller_plays_a_deck_moves_the_mixer_and_gets_its_leds() {
        use midir::os::unix::{VirtualInput, VirtualOutput};
        let name = format!("recordbox test ctl {}", std::process::id());
        let dir = std::env::temp_dir().join(format!("recordbox-midi-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.midi.csv")), virtual_controller_mapping(&name)).unwrap();

        // The virtual controller: its source (what it sends) and its destination (its LEDs).
        let mut ctl_out = match midir::MidiOutput::new("test-controller")
            .map_err(|e| e.to_string())
            .and_then(|o| o.create_virtual(&name).map_err(|e| e.to_string()))
        {
            Ok(port) => port,
            Err(error) => {
                assert!(std::env::var("RECORDBOX_REQUIRE_MIDI_LOOPBACK").as_deref() != Ok("1"),
                    "the required virtual MIDI output could not be opened: {error}");
                eprintln!("virtual MIDI exchange skipped: {error}");
                return;
            }
        };
        let leds: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(vec![]));
        let l2 = leds.clone();
        let _ctl_in = midir::MidiInput::new("test-controller").unwrap().create_virtual(&name, move |_, m, _| l2.lock().push(m.to_vec()), ()).unwrap();

        let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        for d in &decks[..2] {
            d.lock().audio = Some(tone(60.0));
        }
        let (hub, rx) = Hub::new(decks.clone(), dir.clone(), true);
        // Only this test's own port: a real controller on the machine (a DDJ-S1 was) is not
        // opened, so nothing is sent to it.
        hub.routing.lock().performance = true;
        hub.snap.lock().perf = true;
        *hub.only.lock() = Some(vec![name.clone()]);
        {
            let mut s = hub.snap.lock();
            s.decks[0].loaded = true;
            s.decks[1].loaded = true;
            s.decks[0].cue_point = 1.0;
            s.decks[0].hot[0] = Some((12.0, None));
            s.decks[0].assign = 1;
            s.decks[1].assign = 2;
        }
        hub.scan();
        let dev = hub.device(&name).expect("the virtual controller is found by its port name");
        assert!(dev.has_output && !dev.factory && !dev.needs_auth);
        assert_eq!(dev.interval_ms.load(Relaxed), 2);
        assert!(matches!(rx.try_recv(), Ok(UiEvent::Ports { .. })));
        // A second scan does not connect it twice.
        hub.scan();
        assert_eq!(hub.devices.lock().iter().filter(|d| d.name == name).count(), 1);
        assert_eq!(hub.devices.lock().len(), 1, "no other port was opened");

        // The outputs that follow the engine: deck 1 half a turn in (0.9 s at 33 1/3 rpm =
        // 180 degrees: ring step 1 + 72 * 180 / 360 = 37), its level at 0.5 (50).
        {
            let mut d = decks[0].lock();
            d.pos = 0.9 * 44100.0;
            d.peak = [0.5, 0.4];
        }
        hub.engine_outputs();
        wait("the jog ring and the level meter", || {
            let l = leds.lock();
            l.iter().any(|m| m == &[0xBB, 0x00, 37]) && l.iter().any(|m| m == &[0xB0, 0x02, 50])
        });
        {
            let mut d = decks[0].lock();
            d.pos = 0.0;
            d.peak = [0.0, 0.0];
        }
        // PLAY on deck 1: the engine plays, and the PLAY LED goes on (it was not lit).
        ctl_out.send(&[0x90, 0x0B, 0x7F]).unwrap();
        wait("deck 1 playing", || decks[0].lock().playing);
        ctl_out.send(&[0x90, 0x0B, 0x00]).unwrap();
        wait("the PLAY LED", || leds.lock().iter().any(|m| m == &[0x90, 0x0B, 0x7F]));
        // The interface is told, and that the engine side is done.
        let ev = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        match ev {
            UiEvent::Control { tag, deck, val, fast_done, .. } => {
                assert_eq!((registry().get(tag).unwrap().name.as_str(), deck, val, fast_done), ("PlayPause", 1, Val::On(true, 127), true));
            }
            _ => panic!("not a control event"),
        }
        // CUE while playing: back to the cue point, stopped; its LED lights (on the cue point).
        ctl_out.send(&[0x90, 0x0C, 0x7F]).unwrap();
        wait("deck 1 back on its cue point", || {
            let s = decks[0].lock();
            !s.playing && (s.pos - 44100.0).abs() < 1.0
        });
        wait("the CUE LED", || leds.lock().iter().any(|m| m == &[0x90, 0x0C, 0x7F]));
        // The stopped deck's PLAY LED blinks in its 1000 ms cycle: off and on arrive.
        leds.lock().clear();
        wait("PLAY blinking", || {
            let l = leds.lock();
            l.iter().any(|m| m == &[0x90, 0x0B, 0x00]) && l.iter().any(|m| m == &[0x90, 0x0B, 0x7F])
        });
        ctl_out.send(&[0x90, 0x0C, 0x00]).unwrap();
        // Hot cue A held on the stopped deck with Gate Cue on (Preferences > Controller > Deck
        // > HOT CUE): plays from 12 s; released: back there, stopped.
        crate::prefs_bridge::GATE_CUE.store(true, std::sync::atomic::Ordering::Relaxed);
        ctl_out.send(&[0x97, 0x00, 0x7F]).unwrap();
        wait("hot cue A playing", || {
            let s = decks[0].lock();
            s.playing && s.pos >= 12.0 * 44100.0
        });
        assert!(hub.pad_held(0));
        ctl_out.send(&[0x97, 0x00, 0x00]).unwrap();
        wait("hot cue A released", || {
            let s = decks[0].lock();
            !s.playing && (s.pos - 12.0 * 44100.0).abs() < 1.0
        });
        assert!(!hub.pad_held(0));
        // Deck 2's channel fader, 14-bit: half way.
        ctl_out.send(&[0xB1, 0x13, 0x40]).unwrap();
        ctl_out.send(&[0xB1, 0x33, 0x00]).unwrap();
        let half = crate::mixer::channel_fader_gain(0.5, 1);
        wait("deck 2's fader", || (decks[1].lock().ch_fader - half).abs() < 1e-6);
        // The crossfader hard to B: deck 1 (side A) is silent, deck 2 at full level.
        ctl_out.send(&[0xB6, 0x1F, 0x7F]).unwrap();
        wait("the crossfader", || decks[0].lock().fader.abs() < 1e-6 && (decks[1].lock().fader - 1.0).abs() < 1e-6);
        // EQ LOW of deck 1 fully down.
        ctl_out.send(&[0xB0, 0x0F, 0x00]).unwrap();
        wait("EQ LOW", || decks[0].lock().eq_pos[0] == 0.0);
        // The tempo slider: armed and 6 % off the deck's tempo, it waits; once brought to
        // the deck's tempo it has the deck.
        ctl_out.send(&[0xB0, 0x00, 0x66]).unwrap();
        ctl_out.send(&[0xB0, 0x20, 0x00]).unwrap();
        wait("the takeover to notice", || hub.takeover_state(0).state == 3);
        assert_eq!(decks[0].lock().tempo, 1.0);
        ctl_out.send(&[0xB0, 0x00, 0x40]).unwrap();
        ctl_out.send(&[0xB0, 0x20, 0x00]).unwrap();
        wait("the takeover to match", || hub.takeover_state(0).state == 5);
        ctl_out.send(&[0xB0, 0x00, 0x60]).unwrap();
        ctl_out.send(&[0xB0, 0x20, 0x00]).unwrap();
        wait("the tempo", || (decks[0].lock().tempo - (1.0 + 0.10 * 4096.0 / 8191.0)).abs() < 1e-6);
        // One +63-count movement crosses this platter's deadband. Observe the
        // bend while sending, even if the runner pauses between MIDI callbacks.
        assert_eq!(dev.map.lock().input_at(0xB023).map(|e| (e.name.as_str(), e.deck, e.kind)),
                   Some(("JogPitchBend", 1, crate::midi_csv::Kind::JogRotate)));
        let jog_tag = registry().tag("JogPitchBend");
        let until = Instant::now() + Duration::from_secs(5);
        let mut jog_events = 0;
        let received_before = dev.received.load(Relaxed);
        loop {
            ctl_out.send(&[0xB0, 0x23, 0x7F]).unwrap();
            // The worker posts the event after applying the fast-path action.
            if let Ok(UiEvent::Control { tag, deck: 1, val: Val::Jog { delta: 63, .. }, fast_done: true, .. })
                = rx.recv_timeout(Duration::from_millis(10))
            {
                if tag == jog_tag {
                    jog_events += 1;
                    if decks[0].lock().bend > 0.0 { break; }
                }
            }
            assert!(Instant::now() < until, "no positive jog bend: {jog_events} mapped events, {} MIDI packets received",
                dev.received.load(Relaxed) - received_before);
        }
        wait("the bend to end", || decks[0].lock().bend == 0.0);
        // A row without Fast (SYNC) is left to the interface.
        while rx.try_recv().is_ok() {}
        ctl_out.send(&[0x90, 0x58, 0x7F]).unwrap();
        loop {
            match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                UiEvent::Control { tag, fast_done, .. } if registry().get(tag).map(|f| f.name == "Sync").unwrap_or(false) => {
                    assert!(!fast_done);
                    break;
                }
                _ => {}
            }
        }
        // LEDs set from the app's side arrive in priority order, the newest value per code.
        leds.lock().clear();
        let sync = registry().tag("Sync");
        dev.set_led(sync, 1, Led::On);
        dev.set_led(sync, 2, Led::On);
        dev.set_led(sync, 2, Led::Off);
        wait("the SYNC LEDs", || {
            let l = leds.lock();
            l.iter().any(|m| m == &[0x90, 0x58, 0x7F]) && l.iter().any(|m| m == &[0x91, 0x58, 0x00])
        });
        assert!(dev.received.load(Relaxed) >= 18 && dev.sent.load(Relaxed) >= 6);

        // LEARN captures the message without running its previous mapping.
        hub.set_learning(&dev, true);
        while rx.try_recv().is_ok() {}
        ctl_out.send(&[0x90, 0x0B, 0x7F]).unwrap();
        let mut learned = false;
        for _ in 0..4 {
            if let Ok(UiEvent::Learn { msg, .. }) = rx.recv_timeout(Duration::from_secs(2)) {
                assert_eq!(msg, [0x90, 0x0B, 0x7F]);
                learned = true;
                break;
            }
        }
        assert!(learned);
        assert!(!decks[0].lock().playing, "LEARN must not play the deck");

        // Unplugged: the device goes at the next scan.
        drop(ctl_out);
        drop(_ctl_in);
        wait("the device to go", || {
            hub.scan();
            hub.devices.lock().is_empty()
        });
        assert!(!dev.alive.load(Relaxed));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
