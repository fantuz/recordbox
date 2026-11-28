//! BEAT FX and RELEASE FX of the Performance mixer: two FX units (FX1, FX2), each either in
//! multi mode (three effect slots) or single mode (one effect with five parameters),
//! assignable to decks 1 to 4, the sampler or the master.
//!
//! What follows rekordbox 7.2.19 (`checks/re-7.2.19/mixer-fx-recording.md`, section 2): the
//! effect lists, their order and engine ids, the single-mode parameter names, the beat tables
//! and their defaults, the insert points (a deck's effects after its channel fader and the
//! crossfader, the master's before the master limiter), a separate effect state per assigned
//! channel, the 3.33 ms smoothing of on/off, the echo family's LEVEL/DEPTH law and feedback
//! law, MT DELAY's mix law, STRETCH's speed law, TRANS as a square gate, FLANGER's sweep
//! range, PHASER's sweep range, ROLL looping the last beat from a 15 s history, the reverb's
//! structure (five damped combs into two all-passes, feedback 0.9265 to 0.9986), the RELEASE
//! FX time rules, and Beat FX quantize (start and LFO phase on the deck's beat grid).
//! Traced from rekordbox's effect classes (the `Multi*::updateParameter` functions and the
//! cores): every multi-mode effect's LEVEL/DEPTH law, the reverb (line times, decay, input
//! gain, cross-fed combs), Beat FX FILTER's structure and cutoff curves, SPIRAL's feedback
//! law, LOW CUT ECHO's band split, PING PONG's times, UP / DOWN ECHO, HELIX, ROBOT's gains,
//! PITCH's percent law, the FX+ echoes' filters, the brake and back-spin curves.
//! Traced in the second pass (`fx_core` holds the cores): rekordbox's pitch shifter (used by
//! PITCH, ROBOT, PITCH ECHO, UP / DOWN ECHO, SPIRAL's feedback, HELIX's SPEED, STRETCH), ENIGMA
//! JET and MOBIUS SAW / TRI whole, ROBOT's modulation, SPIRAL's loop limit, SPIRAL UP / DOWN's
//! and REVERB UP / DOWN's laws, MT DELAY's taps, FX+ NOISE, STRETCH's turn, PITCH ECHO's
//! feedback, pitch and mix, the echoes' single-mode FEEDBACK, DELAY's pan knob, PHASER's sweep
//! curve, VINYL BRAKE's knob tables, every single-mode read-out, RELEASE FX ECHO.
//! Third pass: SPIRAL's read point follows `SpiralBuffer` (`fx_core::SpiralMotion`); single-mode
//! ROLL's GATE knob (two square gates), HELIX's held loop after a TIME change and its TRACK
//! SPEED, REVERB's DECAY, PHASER's stage knob.
//! Fourth pass (`checks/re-7.2.19/track-notes/fx-audit.md`): the echoes on `DelayModule` take
//! whole frames and a new time by closing the read gate, jumping and opening it (no glide);
//! the lines have no soft clip (rekordbox writes them as they are); the echoes' FILTER knob is
//! `InstFxHPF` then `InstFxLPF` on the repeats (`cfx::InstFilter`); FLANGER is `FlangerCore`;
//! Beat FX FILTER reads rekordbox's 129-step coefficient tables; REVERB's filters are
//! `IirFilter` first-order sections.
//! Still recordbox's own: STRETCH's LOW / HI CUT knob, SPIRAL's loop filters (state-variable
//! sections at rekordbox's cutoffs), the reverse echoes, the ROLL / TRANS / PAN bodies, and the
//! CrushCore inside the FX+ CRUSH buttons.

use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};

/// Every Beat FX: rekordbox's 35 multi-mode effects and PITCH ECHO (single mode only, where
/// multi mode has UP ECHO and DOWN ECHO).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Delay,
    Echo,
    Spiral,
    Reverb,
    RevDelay,
    MtDelay,
    PitchEcho,
    UpEcho,
    DownEcho,
    Trans,
    Pan,
    Filter,
    Flanger,
    Phaser,
    SlipRoll,
    Roll,
    RevRoll,
    Robot,
    Pitch,
    EnigmaJet,
    MobiusSaw,
    MobiusTri,
    LowCutEcho,
    PingPong,
    Helix,
    VinylBrake,
    Stretch,
    BpfEcho,
    Noise,
    SpiralUp,
    ReverbUp,
    HpfEcho,
    LpfEcho,
    CrushEcho,
    SpiralDown,
    ReverbDown,
}

/// Beat tables of rekordbox (spec 2.4).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Table {
    B,
    C,
    D,
    /// Signed beats.
    S,
    /// Percent (REVERB).
    P1,
    /// Percent (ROBOT, PITCH).
    P2,
    /// No beat control: `<` `>` are disabled.
    None,
}

pub struct Info {
    pub kind: Kind,
    pub name: &'static str,
    /// rekordbox's engine ids (`SINGLE_FX_INDEX_LIST`, `MULTI_FX_INDEX_LIST`); -1 = not in
    /// that mode's list.
    pub single_id: i8,
    pub multi_id: i8,
    /// Single-mode labels: knob 1, knob 2, button 1, button 2 ("" = no such control).
    pub labels: [&'static str; 4],
    pub table: Table,
    /// Default index into the table.
    pub def: usize,
}

const fn info(kind: Kind, name: &'static str, single_id: i8, multi_id: i8, labels: [&'static str; 4], table: Table, def: usize) -> Info {
    Info { kind, name, single_id, multi_id, labels, table, def }
}

const CUTS: [&str; 2] = ["LOW CUT", "HI CUT"];
const RF: [&str; 2] = ["REV", "FREEZE"];
const TP: [&str; 2] = ["TONE", "PHASE"];

/// Everything in rekordbox's list order (PITCH ECHO sits where single mode has it).
pub const INFO: [Info; 36] = [
    info(Kind::Delay, "DELAY", 0, 0, ["SWING", "LR RATIO", CUTS[0], CUTS[1]], Table::B, 5),
    info(Kind::Echo, "ECHO", 1, 1, ["FEEDBACK", "FILTER", RF[0], RF[1]], Table::B, 5),
    info(Kind::Spiral, "SPIRAL", 3, 3, ["FEEDBACK", "PITCH", CUTS[0], CUTS[1]], Table::B, 5),
    info(Kind::Reverb, "REVERB", 4, 4, ["ROOM SIZE", "DECAY", CUTS[0], CUTS[1]], Table::P1, 3),
    info(Kind::RevDelay, "REV DELAY", 2, 2, ["SWING", "LR RATIO", CUTS[0], CUTS[1]], Table::B, 5),
    info(Kind::MtDelay, "MT DELAY", 13, 13, ["SWING", "TAP NUM", CUTS[0], CUTS[1]], Table::B, 5),
    info(Kind::PitchEcho, "PITCH ECHO", 14, -1, ["FEEDBACK", "PITCH", RF[0], RF[1]], Table::B, 5),
    info(Kind::UpEcho, "UP ECHO", -1, 14, ["", "", "", ""], Table::B, 5),
    info(Kind::DownEcho, "DOWN ECHO", -1, 15, ["", "", "", ""], Table::B, 5),
    info(Kind::Trans, "TRANS", 5, 5, ["DUTY", "SWING", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::Pan, "PAN", 15, 16, ["LFO AMP", "LFO", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::Filter, "FILTER", 6, 6, ["RESONANCE", "LFO AMP", TP[0], TP[1]], Table::D, 6),
    info(Kind::Flanger, "FLANGER", 7, 7, ["FEEDBACK", "LFO AMP", TP[0], TP[1]], Table::D, 6),
    info(Kind::Phaser, "PHASER", 8, 8, ["FEEDBACK", "STAGE", TP[0], TP[1]], Table::D, 6),
    info(Kind::SlipRoll, "SLIP ROLL", 9, 9, ["DUTY", "SWING", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::Roll, "ROLL", 10, 10, ["DUTY", "SWING", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::RevRoll, "REV ROLL", 11, 12, ["DUTY", "SWING", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::Robot, "ROBOT", 12, 11, ["PITCH", "MOD", CUTS[0], CUTS[1]], Table::P2, 3),
    info(Kind::Pitch, "PITCH", 16, 17, ["PITCH", "", CUTS[0], CUTS[1]], Table::P2, 0),
    info(Kind::EnigmaJet, "ENIGMA JET", 26, 27, ["FEEDBACK", "MOD", RF[0], RF[1]], Table::S, 17),
    info(Kind::MobiusSaw, "MOBIUS SAW", 27, 28, ["ECHO LEVEL", "MOD", RF[0], RF[1]], Table::S, 17),
    info(Kind::MobiusTri, "MOBIUS TRI", 28, 29, ["ECHO LEVEL", "MOD", RF[0], RF[1]], Table::S, 17),
    info(Kind::LowCutEcho, "LOW CUT ECHO", 29, 30, ["FEEDBACK", "SWING", RF[0], RF[1]], Table::B, 5),
    info(Kind::PingPong, "PING PONG", 30, 31, ["FEEDBACK", "FILTER", RF[0], RF[1]], Table::B, 5),
    info(Kind::Helix, "HELIX", 31, 32, ["TIME", "SPEED", CUTS[0], CUTS[1]], Table::C, 4),
    info(Kind::VinylBrake, "VINYL BRAKE", 32, 33, ["REPEAT", "ECHO", "ON", "BPF"], Table::C, 6),
    info(Kind::Stretch, "STRETCH", 33, 34, ["LOW / HI CUT", "", "REPITCH", "BPF"], Table::C, 5),
    info(Kind::BpfEcho, "BPF ECHO", 24, 25, ["ECHO MIX", "MOD", RF[0], RF[1]], Table::None, 0),
    info(Kind::Noise, "NOISE", 17, 18, ["VOL", "MOD", "CRUSH", "FREEZE"], Table::None, 0),
    info(Kind::SpiralUp, "SPIRAL UP", 22, 23, ["ECHO MIX", "PITCH", CUTS[0], CUTS[1]], Table::None, 0),
    info(Kind::ReverbUp, "REVERB UP", 18, 19, ["TIME", "HPF", CUTS[0], CUTS[1]], Table::None, 0),
    info(Kind::HpfEcho, "HPF ECHO", 20, 21, ["ECHO MIX", "MOD", RF[0], RF[1]], Table::None, 0),
    info(Kind::LpfEcho, "LPF ECHO", 21, 22, ["ECHO MIX", "MOD", RF[0], RF[1]], Table::None, 0),
    info(Kind::CrushEcho, "CRUSH ECHO", 25, 26, ["ECHO MIX", "MOD", RF[0], RF[1]], Table::None, 0),
    info(Kind::SpiralDown, "SPIRAL DOWN", 23, 24, ["ECHO MIX", "PITCH", CUTS[0], CUTS[1]], Table::None, 0),
    info(Kind::ReverbDown, "REVERB DOWN", 19, 20, ["TIME", "LPF", CUTS[0], CUTS[1]], Table::None, 0),
];

pub fn info_of(k: Kind) -> &'static Info {
    INFO.iter().find(|i| i.kind == k).unwrap()
}

pub fn name(k: Kind) -> &'static str {
    info_of(k).name
}

/// The pop-up list of a mode, in rekordbox's order (35 in multi mode, 34 in single mode).
pub fn list(single: bool) -> impl Iterator<Item = &'static Info> {
    INFO.iter().filter(move |i| if single { i.single_id >= 0 } else { i.multi_id >= 0 })
}

/// An effect from rekordbox's engine id (what `FxUnitSettings` stores).
pub fn from_id(id: i64, single: bool) -> Option<Kind> {
    list(single).find(|i| (if single { i.single_id } else { i.multi_id }) as i64 == id).map(|i| i.kind)
}

/// The values the `<` `>` buttons step through for an effect, and where they start:
/// (values, default index, percent instead of beats). Empty = no beat control.
pub fn beat_table(k: Kind) -> (&'static [f32], usize, bool) {
    const B: [f32; 10] = [0.0625, 0.125, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
    const C: [f32; 9] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
    const D: [f32; 11] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    const S: [f32; 22] = [-64.0, -32.0, -16.0, -8.0, -4.0, -2.0, -1.0, -0.5, -0.25, -0.125, -0.0625, 0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    const P1: [f32; 7] = [1.0, 10.0, 25.0, 50.0, 75.0, 90.0, 100.0];
    const P2: [f32; 7] = [-50.0, -37.0, -25.0, 0.0, 26.0, 50.0, 100.0];
    let i = info_of(k);
    match i.table {
        Table::B => (&B, i.def, false),
        Table::C => (&C, i.def, false),
        Table::D => (&D, i.def, false),
        Table::S => (&S, i.def, false),
        Table::P1 => (&P1, i.def, true),
        Table::P2 => (&P2, i.def, true),
        Table::None => (&[], 0, false),
    }
}

/// The value an effect starts with (0 for the effects without a beat control).
pub fn default_value(k: Kind) -> f32 {
    let (t, d, _) = beat_table(k);
    t.get(d).copied().unwrap_or(0.0)
}

/// A beat value as rekordbox prints it: whole beats as `N`, fractions as `n/d`, percent `N%`.
pub fn beat_text(v: f32, percent: bool) -> String {
    if percent {
        return format!("{v:.0}%");
    }
    let a = v.abs();
    let s = if a >= 1.0 {
        format!("{a:.0}")
    } else if (a - 0.75).abs() < 1e-6 {
        "3/4".into()
    } else {
        format!("1/{:.0}", 1.0 / a)
    };
    if v < 0.0 { format!("-{s}") } else { s }
}

/// The text of an effect's value: the signed table prints `+` too.
pub fn value_text(k: Kind, v: f32) -> String {
    let i = info_of(k);
    match i.table {
        Table::None => "-".into(),
        Table::S if v > 0.0 => format!("+{}", beat_text(v, false)),
        t => beat_text(v, matches!(t, Table::P1 | Table::P2)),
    }
}

/// RELEASE FX: (name, default index into table B). VINYL BRAKE 3/4, ECHO 1, BACK SPIN 4.
pub const RELEASE: [(&str, usize); 3] = [("VINYL BRAKE", 4), ("ECHO", 5), ("BACK SPIN", 7)];
pub const RELEASE_BEATS: [f32; 10] = [0.0625, 0.125, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];

/// What the interface sets for one slot.
#[derive(Clone, Copy)]
pub struct SlotParams {
    pub kind: Kind,
    pub on: bool,
    /// LEVEL/DEPTH, 0..1.
    pub level: f32,
    /// Beats (or percent for REVERB, ROBOT and PITCH).
    pub value: f32,
    /// Single mode's knob 1 and knob 2 (0..1, 0.5 in multi mode).
    pub knob: [f32; 2],
    /// Single mode's button 1 and button 2 (off in multi mode).
    pub btn: [bool; 2],
}

impl SlotParams {
    pub fn new(kind: Kind) -> SlotParams {
        SlotParams { kind, on: false, level: 0.5, value: default_value(kind), knob: [0.5; 2], btn: [false; 2] }
    }
}

#[derive(Clone, Copy)]
pub struct UnitParams {
    /// Decks 1 to 4, the master, the sampler.
    pub assign: [bool; CHANNELS],
    pub slots: [SlotParams; 3],
    /// Single mode: `one` is the unit's effect and `slots` are out.
    pub single: bool,
    pub one: SlotParams,
    /// RELEASE FX held: which one and its beats.
    pub release: Option<(u8, f32)>,
    /// Tempo each channel's effects run at (BPM).
    pub bpm: [f32; CHANNELS],
    /// Beat FX quantize per channel (the deck's Q): effects and their LFOs start on the grid.
    pub quantize: [bool; CHANNELS],
}

impl UnitParams {
    pub fn new() -> UnitParams {
        // rekordbox's default slots: DELAY, ECHO, SPIRAL; single mode starts on DELAY.
        UnitParams { assign: [false; CHANNELS], slots: [SlotParams::new(Kind::Delay), SlotParams::new(Kind::Echo), SlotParams::new(Kind::Spiral)], single: false, one: SlotParams::new(Kind::Delay), release: None, bpm: [120.0; CHANNELS], quantize: [true; CHANNELS] }
    }
}


// ---------------------------------------------------------------------------------------
// Laws read from rekordbox 7.2.19's effect classes (arm64, Hex-Rays). `l` is LEVEL/DEPTH.
// ---------------------------------------------------------------------------------------

/// rekordbox's feedback law for the echo family's FEEDBACK knob: the left half runs from 0
/// to the centre value, the right half from there to 1.
pub fn fb_law(k: f32, centre: f32) -> f32 {
    let k = k.clamp(0.0, 1.0);
    if k < 0.5 { 2.0 * centre * k } else { centre + (1.0 - centre) * 2.0 * (k - 0.5) }
}

/// The echo family's LEVEL/DEPTH law (`MultiBeatFxEcho::calcMixRatio`; DELAY, ECHO, REVERB,
/// LOW CUT ECHO and PING PONG use it too): (dry, wet).
pub fn echo_mix(level: f32) -> (f32, f32) {
    if level < 0.75 { (1.0, 1.3333 * level) } else { (4.0 - 4.0 * level, 1.0) }
}

/// MT DELAY's LEVEL/DEPTH law (`GroupMultiTapDelay::calculateMixRatio` 0x102a081c4, re-read
/// from its disassembly: the dry side above the centre is one fused `L x -0.6 + 1.3`): (dry, wet).
pub fn mt_mix(level: f32) -> (f32, f32) {
    if level < 0.5 { (1.0, level * 1.4) } else { (level.mul_add(-0.6, 1.3), 0.7) }
}

/// TRANS and PAN (`MultiBeatFxTrans` / `MultiBeatFxPan::updateParameter`): the effect takes
/// over in the lower half of the knob and is alone from the centre.
pub fn half_mix(l: f32) -> (f32, f32) {
    if l < 0.5 { (1.0 - 2.0 * l, 2.0 * l) } else { (0.0, 1.0) }
}

/// FLANGER and PHASER (`MultiBeatFxFlanger::updateParameter`): both at 0.75 from a quarter up.
pub fn quarter_mix(l: f32) -> (f32, f32) {
    if l >= 0.25 { (0.75, 0.75) } else { (1.0 - l, 3.0 * l) }
}

/// FLANGER's and PHASER's feedback in multi mode: none up to a quarter, then up to 0.8.
pub fn mod_fb(l: f32) -> f32 {
    if l < 0.25 { 0.0 } else { 1.0667 * l - 0.26667 }
}

/// TRANS's open share of each turn in multi mode: half up to the centre, 1/16 fully up.
pub fn trans_duty(l: f32) -> f32 {
    if l >= 0.5 { 0.9375 - 0.875 * l } else { 0.5 }
}

/// Beat FX FILTER (`MultiBeatFxFilter::updateParameter`): the filter is alone above 0.15625,
/// below that a quarter-sine cross-fade; (dry, wet).
pub fn filter_mix(l: f32) -> (f32, f32) {
    if l > 0.15625 {
        return (0.0, 1.0);
    }
    let x = l.max(0.0) * 12.8;
    let t = (1.0 - x) - x * (1.0 - x);
    let dry = ((0.5 - 0.5 * x) * ((0.071862 * t - 0.64211) * t + 1.5703) + 0.5).clamp(0.0, 1.0);
    (dry, 1.0 - dry)
}

/// Beat FX FILTER's resonance gain in multi mode (0 .. 0.96).
pub fn filter_reso(l: f32) -> f32 {
    let v = if l < 0.1523 { 1.0 } else if l >= 0.3984 { 1.0 - l } else { 1.0 - 0.000048631 * l };
    (0.96 - 0.96 * v * v).min(0.96)
}

/// The ROLL family (`MultiBeatFxRollBase::calcMixRatio`): rekordbox's 129-step table is
/// 1 - (i / 127.5)^2 exactly; (dry, wet).
pub fn roll_mix(l: f32) -> (f32, f32) {
    let i = (l.clamp(0.0, 1.0) * 256.0) as i32;
    if i > 128 {
        return (0.0, 1.0);
    }
    let t = |i: i32| (1.0 - (i as f32 / 127.5).powi(2)).max(0.0);
    (t(i), t(128 - i))
}

/// UP ECHO / DOWN ECHO (`MultiBeatFxUpEcho::updateParameter`): (wet, pitch percent of each
/// repeat). Feedback is 0.85; the dry sound stays whole.
pub fn up_echo(l: f32) -> (f32, f32) {
    if l < 0.5 { (1.4 * l, 0.0) } else { (0.7, 33.333 * l - 16.667) }
}

/// rekordbox's pitch shifter percent law (`rbxfrm::PitchShifter::setPitchPercentage`):
/// -100 % is an octave down, +100 % an octave up, +200 % two.
pub fn pitch_ratio(pct: f32) -> f32 {
    let p = pct.clamp(-200.0, 200.0);
    1.0 + if p < 0.0 { 0.005 * p } else if p < 100.0 { 0.01 * p } else { 0.02 * p - 1.0 }
}

/// SPIRAL (`BeatFxSpiral::updateParameter`): (wet, feedback, output gain). While it is on the
/// feedback passes 1 above the centre (the core limits the loop); once switched off it is
/// held under 1 so the tail dies away.
pub fn spiral_law(l: f32, on: bool) -> (f32, f32, f32) {
    let wet = if l < 0.75 { 1.333 * l } else { 1.0 };
    let fb = if l < 0.1 {
        8.0 * l
    } else if l < 0.5 {
        0.8
    } else if l < 1.0 {
        0.8 + if on { 0.8 } else { 0.386 } * (l - 0.5)
    } else if on {
        1.2
    } else {
        0.993
    };
    (wet, fb, 1.0 - 0.501 * l)
}

/// SPIRAL UP / DOWN (`RmxSpiral::updateParameter`) for the main knob `l`: (share of the input
/// that enters, delay ms, dry gain, feedback, output gain before its 2 s smoothing). UP's
/// delay shrinks from 100 ms to 5 ms and its feedback climbs to 1.5; DOWN's delay grows from
/// 10 ms to a beat, its feedback to 1.2, and the dry sound fades out above the centre. Once
/// switched off the feedback is 0.993 so the tail dies away.
pub fn rmx_spiral_law(l: f32, down: bool, on: bool, beat_ms: f32) -> (f32, f32, f32, f32, f32) {
    let v = (l.clamp(0.0, 1.0) * 255.0).round() as i32;
    let u = if v < 4 { 0.0 } else { (v - 3) as f32 * 0.0039683 };
    let inp = if u < 0.25 { u * 4.0 } else { 1.0 };
    let (ms, dry) = if !down {
        (if u >= 0.015686 { 100.0 - 95.0 * u } else { 100.0 }, 1.0)
    } else {
        (if u >= 0.015686 { 10.0 + u * (beat_ms - 10.0) } else { 10.0 }, if u >= 0.5 { 2.0 - 2.0 * u } else { 1.0 })
    };
    let ms = ms.max(5.0).min(beat_ms.max(5.0));
    let n = (u * 255.0) as i32;
    let ramp = if n >= 4 { if (n - 3) as f32 * 0.0039683 < 0.1 { (n - 3) as f32 * 0.039683 } else { 1.0 } } else { 0.0 };
    let fb = 0.8 + if !on { 0.193 } else if down { 0.4 } else { 0.7 } * ramp;
    let og = (fb * ((ms * 0.0005).min(1.0) * 0.181 - 0.501) + 1.0).min(0.3) * 1.8363;
    (inp, ms, dry, fb, og)
}

/// The FX+ reverbs' TIME knob in 8-bit steps `t` with the main knob `v`: decay seconds.
pub fn fxrev_time(t: i32, v: i32) -> f32 {
    if t <= 3 {
        return 1.0;
    }
    let x = (t - 3) as f32 * 0.0039683;
    let a = x * x * (t - 3) as f32 * 0.19444;
    if v <= 63 && a + 1.0 > 8.0 { (a * 0.015625 - 0.10938) * v as f32 + 8.0 } else { a + 1.0 }
}

/// REVERB UP (`FxPlusReverbUp::updateReverbUpParameter`) for the main knob `v` and TIME `t`
/// (8-bit): (band-pass centre Hz, its Q, its gain, room size, decay s, reverb input gain). The
/// band rises from 200 Hz to 5.2 kHz and narrows, the room shrinks from 3.3 to 0.3.
pub fn reverb_up_law(v: i32, t: i32) -> (f32, f32, f32, f32, f32, f32) {
    let (fc, q, g) = if v > 3 {
        let n = (v - 4) as f32;
        let q = if v >= 26 { n * n * 0.000031746 + 0.0101 } else { 0.0101 };
        (10f32.powf((n * 0.0039841).sqrt() * 3.7) + 200.0, q, n * 0.0031873 * (n * 0.0039841) + 1.0)
    } else {
        (200.0, 0.0101, 1.0)
    };
    let w = if v < 3 { -1.0 } else { (255 - v) as f32 * -0.0039683 };
    let size = w * w * w * w * 3.0 + 0.3;
    let gin = if v >= 64 { 0.13 } else { v as f32 * 0.0020312 };
    (fc, q, g, size, fxrev_time(t, v), gin)
}

/// REVERB DOWN (`FxPlusReverbDown::updateReverbDownParameter`): (room size, decay s, input
/// gain, dry gain). The room grows from 0.38 to 2.08; above the centre the dry sound fades.
pub fn reverb_down_law(v: i32, t: i32) -> (f32, f32, f32, f32) {
    let a = if v < 3 { 1.0331 } else { (v - 3) as f32 * 0.0090593 + 1.0331 };
    let size = a * a * 0.171 + 0.2;
    let p = v as f32 * 0.39216;
    let x = v as f32 * 0.039216;
    let extra = if p > 90.0 { ((x - 9.0) * (x - 9.0)).powi(2) * 0.2 } else if p < 17.0 { (x - 1.7) * (x - 1.7) * 0.018 } else { 0.0 };
    let base = -0.000066116 * (p - 55.0) * (p - 55.0) + 0.2 + extra;
    let short = if t > 3 { v < 64 && { let y = (t - 3) as f32 * 0.0039683; y * y * (t - 3) as f32 * 0.19444 + 1.0 > 8.0 } } else { v < 64 };
    let ramp = if short { v as f32 * 0.015625 } else { 1.0 };
    let k = t as f32 * 0.0039216;
    let gin = base * (((1.0 - k) - k * (1.0 - k)) + 0.5) * ramp;
    let dry = if v as f32 * 0.0039216 > 0.5 { 2.0 - v as f32 * 0.0078431 } else { 1.0 };
    (size, fxrev_time(t, v), gin, dry)
}

/// The FX+ reverbs' FILTER knob (8-bit, single mode): REVERB UP's high-pass (54 Hz .. 6.3 kHz)
/// or REVERB DOWN's low-pass (17.3 kHz .. 261 Hz) on the reverb, -3 dB resonance.
pub fn fxrev_filter(down: bool, f: i32) -> f32 {
    if down {
        if f < 4 { 17330.0 } else { 10f32.powf(4.2388 - 0.0072596 * (f - 4) as f32) }
    } else if f < 4 {
        54.0
    } else {
        10f32.powf((f - 4) as f32 * 0.0082376 + 1.7324)
    }
}

/// MT DELAY's eight taps (1/8 .. 8/8 of the time; `SingleMultiTapDelay::updateParameter`,
/// `MultiTapDelayCore`): multi mode has the even eighths up to 3/4 and brings the odd ones in
/// above the centre; single mode's TAP NUM (1..7) adds them in rekordbox's order. The last
/// eighth is never used.
pub fn mt_taps(level: f32, multi: bool, tap_num: f32) -> [f32; 8] {
    if multi {
        let v = if level <= 0.5 { 0.0 } else { 2.0 * level - 1.0 };
        return [v, 1.0, v, 1.0, v, 1.0, v, 0.0];
    }
    let n = ((tap_num.clamp(0.0, 1.0) * 6.0 + 1.0) as i32).clamp(1, 7);
    let mut t = [0.0; 8];
    // In the order they join: 6/8, 4/8, 2/8, 7/8, 5/8, 3/8, 1/8.
    for (k, &i) in [5usize, 3, 1, 6, 4, 2, 0].iter().enumerate() {
        if (k as i32) < n {
            t[i] = 1.0;
        }
    }
    t
}

/// MT DELAY's SWING (single mode): (left, right) stretch of the tap pattern, 1 + 2 x the
/// side's share (rekordbox sets one side's parameter from the knob's distance from the centre).
pub fn mt_swing(k: f32) -> (f32, f32) {
    if k >= 0.5 { (1.0 + 2.0 * (2.0 * k - 1.0), 1.0) } else { (1.0, 1.0 + 2.0 * (1.0 - 2.0 * k)) }
}

/// FX+ NOISE (`FxPlusNoiseEcho::updateAndSetParameter`): the noise's level for the main knob in
/// 8-bit steps; the loudest (0.8) a fifth of the way up, then falling to 0.08.
pub fn noise_gain(v: i32) -> f32 {
    if v <= 3 {
        0.0
    } else if v <= 50 {
        0.8 - 0.00036215 * ((51 - v) * (51 - v)) as f32
    } else if v <= 101 {
        0.53812 + 0.0051348 * (102 - v) as f32
    } else {
        let u = (v - 101) as f32 * 0.0019481;
        let a = (1.0 - u) * (1.0 - u);
        a * a * a * a * (v - 3) as f32 * 0.005521
    }
}

/// FX+ NOISE's band-pass centre (`updateFilterCoef`) for the knob in 8-bit steps (with MOD's
/// wobble added): 220 Hz up to 9 kHz.
pub fn noise_fc(v: i32) -> f32 {
    let v = v.clamp(0, 255);
    if v >= 4 { 2f32.powf(((v - 4) as f32 * 0.0039841).powf(0.66) * 5.3544) * 220.0 } else { 220.0 }
}

/// PITCH ECHO's FEEDBACK knob (`SingleBeatFxPitchEcho::updateAndSetParameter`): 0 .. 0.85 on
/// the left, 0.85 in the centre, up to 0.993 on the right while on (0.85 .. 1 while turning
/// off).
pub fn pitch_echo_fb(k: f32, turning_off: bool) -> f32 {
    let half = half_value(k);
    if k < 0.48627 { (half * 0.85).clamp(0.0, 1.0) } else if k <= 0.51373 { 0.85 } else { (half * if turning_off { 0.15 } else { 0.143 } + 0.85).clamp(0.0, 1.0) }
}

/// rekordbox's centre-detent knob readings (`CenterRecommendParameter`, `CenterThroughParameter`):
/// the dead zone is 0.48627 .. 0.51373. `half_value`: 0..1 across each half (left from the end
/// toward the centre, right from the centre to the end). `through_value`: the distance from
/// the centre, 0 in the dead zone, 1 at either end. `recommend_value`: 0..0.5..1 with the dead
/// zone held at 0.5.
pub fn half_value(k: f32) -> f32 {
    const S: f32 = 1.0 / 0.48627;
    if k <= 0.0 {
        0.0
    } else if k >= 1.0 {
        1.0
    } else if k < 0.48627 {
        (S * k).clamp(0.0, 1.0)
    } else if k <= 0.51373 {
        -1.0
    } else {
        (S * (k - 0.51373)).clamp(0.0, 1.0)
    }
}

/// Single-mode ROLL family's SWING knob (`SingleBeatFxRollBase::updateAndSetParameter`): the
/// zone -4 .. 4 (ninths of the travel).
pub fn roll_swing_zone(k: f32) -> f32 {
    let edges = [0.11111, 0.22222, 0.33333, 0.44444, 0.55556, 0.66667, 0.77778, 0.88889];
    -4.0 + edges.iter().filter(|&&e| k >= e).count() as f32
}

/// Single-mode ROLL / SLIP ROLL / REV ROLL's first knob (`SingleBeatFxRollBase::
/// updateAndSetParameter` 0x102a2fca0 into its two `Trans` units, square waves): (roll gate
/// duty, its start, live-sound gate duty, its start) over one turn of the roll. Centre: the
/// roll alone. Left by `v`: the roll for the first 1 - v of each turn, the live sound for the
/// rest (all live at the end). Right: the roll for the first 1 - 0.95 v, then silence.
pub fn roll_gates(k: f32) -> (f32, f32, f32, f32) {
    let v = through_value(k);
    if k < 0.48627 {
        (1.0 - v, 0.0, v, if (v - 1.0).abs() >= 1.1921e-7 { v } else { 0.0 })
    } else if k <= 0.51373 {
        (1.0, 0.0, 0.0, 0.0)
    } else {
        (1.0 - 0.95 * v, 0.0, 0.0, 0.0)
    }
}

pub fn through_value(k: f32) -> f32 {
    const S: f32 = 1.0 / 0.48627;
    if k <= 0.0 || k >= 1.0 {
        1.0
    } else if k < 0.48627 {
        (S * (0.48627 - k)).min(1.0)
    } else if k <= 0.51373 {
        0.0
    } else {
        (S * (k - 0.51373)).min(1.0)
    }
}

pub fn recommend_value(k: f32) -> f32 {
    const S: f32 = 1.0 / 0.48627;
    if k <= 0.0 {
        0.0
    } else if k >= 1.0 {
        1.0
    } else if k < 0.48627 {
        (k * 0.5 * S).max(0.0)
    } else if k <= 0.51373 {
        0.5
    } else {
        (S * (k * 0.5 - 0.25686) + 0.5).min(1.0)
    }
}

/// A single-mode knob's read-out as rekordbox prints it (`RbxSingleFxUnitPanel::
/// convertSingleParamString` and its helpers), for knob `k` (0 = knob 1) at position `v`.
pub fn single_knob_text(kind: Kind, k: usize, v: f32) -> String {
    use Kind::*;
    let c = |v: f32| if v >= 0.0 { 2.0 * v.min(1.0) - 1.0 } else { -1.0 };
    let r = |x: f32| x.round() as i32;
    let plain = |v: f32| format!("{v:.1}");
    let pct = |v: f32| format!("{}%", r(v * 100.0));
    // L:/R: (LR RATIO, SWING), H:/L: (STRETCH's EQ), signed percent (PITCH).
    let lr = |v: f32, a: &str, b: &str| {
        let x = if v >= 0.0 { v.min(1.0) * 200.0 - 100.0 } else { -100.0 };
        if x > 1.0 { format!("{b}{}%", r(x)) } else if x < -1.0 { format!("{a}{}%", r(-x)) } else { "0".into() }
    };
    let pitch = |v: f32| {
        let x = c(v);
        if x * 100.0 > 1.0 { format!("+{}%", r(x * 100.0)) } else if x * 100.0 < -1.0 { format!("{}%", r(x * 50.0)) } else { "0".into() }
    };
    let side = |v: f32, slope: f32, off: f32, top: f32, a: &str, b: &str| {
        let x = c(v);
        if x * 0.5 > 0.013725 {
            format!("{b}{}", r((x * slope + off).clamp(0.0, top)))
        } else if x * 0.5 < -0.013725 {
            format!("{a}{}", r((-x * slope + off).clamp(0.0, top)))
        } else {
            "0".into()
        }
    };
    let centre_mod = |v: f32| {
        let x = c(v);
        if x > 0.013725 { format!("{}%", r(x * 100.0)) } else if x < -0.013725 { format!("{}%", r(-x * 100.0)) } else { "0".into() }
    };
    match (kind, k) {
        (Delay | RevDelay, _) | (MtDelay, 0) | (LowCutEcho, 1) => lr(v, "L:", "R:"),
        (MtDelay, _) => format!("{}", r(v * 6.0 + 1.0)),
        (Echo, 1) => side(v, 102.82, -2.8226, 100.0, "LPF ", "HPF "),
        (Spiral | PitchEcho, 1) | (Robot | Pitch, 0) => pitch(v),
        (Pitch, _) | (Stretch, 1) => String::new(),
        (Reverb, _) | (Robot, 1) => pct(v),
        (Trans | SlipRoll | Roll | RevRoll, 0) => format!("{}%", r(v * 99.0)),
        (Trans | SlipRoll | Roll | RevRoll, _) => lr(v, "L:", "R:"),
        (Phaser, 1) => side(v, 58.609, 1.3911, 60.0, "L STAGE:", "R STAGE:"),
        (EnigmaJet | MobiusSaw | MobiusTri, 1) | (Helix, 1) => centre_mod(v),
        (Helix, _) => {
            let x = c(v);
            if x > 0.013725 { format!("+{}%", (x * 405.57 - 5.5666) as i32) } else if x < -0.013725 { format!("{}%", (x * 405.57 + 5.5666) as i32) } else { "0".into() }
        }
        (VinylBrake, 0) => vinyl_repeat_text(v),
        (VinylBrake, _) => vinyl_echo_text(v),
        (Stretch, _) => lr(v, "L:", "H:"),
        (BpfEcho | Noise | HpfEcho | LpfEcho | CrushEcho | ReverbUp | ReverbDown, 1) => pct(v),
        (SpiralUp, 1) => format!("+{}%", r(v * 100.0)),
        (SpiralDown, 1) => format!("-{}%", r(v * 50.0)),
        _ => plain(v),
    }
}

/// VINYL BRAKE's REPEAT knob in beats (`BeatFxVinylBrake::updateRepeat`, read through the
/// centre-detent law), in rekordbox's (not monotonic) order.
pub fn vinyl_repeat_beats(k: f32) -> f32 {
    let v = recommend_value(k);
    if v < 0.0005 {
        0.0625
    } else if v > 0.9995 {
        4.0
    } else {
        [(0.11, 0.125), (0.22, 0.16667), (0.33, 0.33333), (0.44, 0.25), (0.56, 0.5), (0.67, 0.75), (0.78, 1.0), (0.89, 1.5)].iter().find(|(t, _)| v < *t).map(|x| x.1).unwrap_or(2.0)
    }
}

/// VINYL BRAKE's ECHO knob in beats; `None` in the centre (no echo).
pub fn vinyl_echo_beats(k: f32) -> Option<f32> {
    let v = recommend_value(k);
    if (0.425..0.575).contains(&v) {
        return None;
    }
    Some([(0.025, 0.125), (0.125, 0.16667), (0.225, 0.33333), (0.325, 0.25), (0.425, 0.5), (0.7, 0.75), (0.825, 1.0), (0.95, 1.5)].iter().find(|(t, _)| v < *t).map(|x| x.1).unwrap_or(2.0))
}

fn frac_text(b: f32) -> String {
    let near = |x: f32| (b - x).abs() < 1e-3;
    if near(0.16667) { "1/6".into() } else if near(0.33333) { "1/3".into() } else if near(1.5) { "1.5".into() } else { beat_text(b, false) }
}

pub fn vinyl_repeat_text(k: f32) -> String {
    frac_text(vinyl_repeat_beats(k))
}

pub fn vinyl_echo_text(k: f32) -> String {
    vinyl_echo_beats(k).map(frac_text).unwrap_or_else(|| "NONE".into())
}

/// Beat FX VINYL BRAKE's brake length in beats for LEVEL/DEPTH (`BeatFxVinylBrake::
/// updateStateAndParameter`): 4 beats low on the knob, an eighth at the top.
pub fn vinyl_brake_beats(l: f32) -> f32 {
    let v = if l < 0.25 {
        4.0
    } else if l < 0.5 {
        let x = 2.0 - 4.0 * l;
        0.75 + 3.25 * x * x
    } else if l < 0.99023 {
        1.3875 - 1.2749 * l
    } else {
        0.125
    };
    v.clamp(0.125, 4.0)
}

/// The echoes' FEEDBACK knob in single mode (`SingleBeatFxEcho`, `SingleBeatFxPingPong`,
/// `SingleLowCutFxEcho`): 0 .. 0.7 on the left, 0.7 in the centre, up to 0.993 on the right
/// (1.0 while turning off).
pub fn echo_fb(k: f32, turning_off: bool) -> f32 {
    let half = half_value(k);
    if k < 0.48627 { (half * 0.7).clamp(0.0, 1.0) } else if k <= 0.51373 { 0.7 } else { (half * if turning_off { 0.3 } else { 0.293 } + 0.7).clamp(0.0, 1.0) }
}

/// TRANS's DUTY knob in single mode (`SingleBeatFxTrans::updateParameter`): the open share,
/// 1 at the left end, 1/2 in the centre, 1/16 at the right end.
pub fn trans_single_duty(k: f32) -> f32 {
    let h = half_value(k);
    if k < 0.48627 { 1.0 - 0.5 * h } else if k <= 0.51373 { 0.5 } else { 0.5 - 0.4375 * h }
}

/// Beat FX FILTER's RESONANCE in single mode (`SingleBeatFxFilter::updateParameter`): 0 .. 0.72
/// on the left, 0.72 in the centre, up to 0.96 on the right.
pub fn filter_single_reso(k: f32) -> f32 {
    let h = half_value(k);
    if k < 0.48627 { 0.72 * h } else if k <= 0.51373 { 0.72 } else { 0.72 + 0.24 * h }
}

/// FLANGER's and PHASER's FEEDBACK knob in single mode (`SingleBeatFxFlanger` / `Phaser::
/// updateParameter`): 0 .. 0.2667 .. 0.93; with the button the feedback is negative (0 .. 0.77
/// on the left, 0.8 in the centre, up to 0.95).
pub fn flanger_fb(k: f32, neg: bool) -> f32 {
    let h = half_value(k);
    if neg {
        let v = if k < 0.48627 { if h < 0.1 { h * 7.7 } else { 0.76967 + 0.0033333 * h } } else if k <= 0.51373 { 0.8 } else { 0.8 + 0.15 * h };
        -v
    } else if k < 0.48627 {
        0.26667 * h
    } else if k <= 0.51373 {
        0.26667
    } else {
        0.26667 + 0.66333 * h
    }
}

/// The level FLANGER / PHASER take back as their feedback rises (both gains of the mix).
pub fn flanger_gain(k: f32, neg: bool) -> f32 {
    if neg {
        let r = recommend_value(k);
        if r < 0.1 { 1.0 - 1.0875 * r } else if r < 0.5 { 0.89125 } else { 1.2202 - 0.65782 * r }
    } else {
        let f = flanger_fb(k, false).abs();
        if f > 0.8 { 3.6603 - 3.3254 * f } else { 1.0 }
    }
}

/// PHASER's version of that level (`SingleBeatFxPhaser::updateParameter` 0x102a0d14c): with the
/// button, 1 - 1.0875 r under 0.1, 0.89125 up to 0.5, then 1.1515 - 0.52059 r.
pub fn phaser_gain(k: f32, neg: bool) -> f32 {
    if neg {
        let r = recommend_value(k);
        if r < 0.1 { 1.0 - 1.0875 * r } else if r < 0.5 { 0.89125 } else { 1.1515 - 0.52059 * r }
    } else {
        flanger_gain(k, false)
    }
}

/// Single REVERB's ROOM SIZE (`SingleBeatFxReverb::updateParameter`): the percent value, 1 .. 50
/// across the left half, 50 in the centre, 50 .. 100 across the right.
pub fn reverb_room(k: f32) -> i32 {
    let h = half_value(k);
    let v = if k < 0.48627 { (h * 49.0 + 1.0) as i32 } else if k <= 0.51373 { 50 } else { (h * 50.0 + 50.0) as i32 };
    v.clamp(1, 100)
}

/// HELIX's TIME knob in single mode (`SingleBeatFxHelix::updateParameter`): 4 x the beat time
/// at the left end down to it at the centre, a quarter of it at the right end (10 .. 4000 ms).
pub fn helix_time(k: f32, ms: f32) -> f32 {
    let h = half_value(k);
    let t = if k < 0.48627 { (4.0 - 3.0 * h) * ms } else if k <= 0.51373 { ms } else { ms / (3.0 * h + 1.0) };
    t.clamp(10.0, 4000.0)
}

/// HELIX (`MultiBeatFxHelix::updateParameter`): (how much of the loop is kept each turn, how
/// much of the input enters). Fully up the loop is held and the input is shut out.
pub fn helix_law(l: f32) -> (f32, f32) {
    if l >= 0.5 {
        (0.7 + 0.3 * l, 0.9 - 0.9 * l)
    } else {
        let t = (1.0 - 2.0 * l) * (1.0 - 2.0 * l);
        (0.85 * (1.0 - t), 0.45 + 0.55 * t)
    }
}

/// ROBOT (`MultiFxRobot::updateParameter`): (the plain sound's gain, the robot voice's gain,
/// modulation). The knob is read in 8-bit steps.
pub fn robot_law(l: f32) -> (f32, f32, f32) {
    let v = (l.clamp(0.0, 1.0) * 255.0) as i32;
    if v > 24 { (0.75, 0.75, (v - 25) as f32 * 0.0034783) } else { (1.0 - 0.01 * v as f32, 0.03 * v as f32, 0.0) }
}

/// STRETCH's playing speed from LEVEL/DEPTH (`StretchCore::processEffectInternal`).
/// Single-mode STRETCH's knob and BPF (`SingleBeatFxStretch::updateParameter` 0x102a459c0):
/// the low band's gain into the stretch and its live gain, then the high band's. Left of the
/// centre the lows fall to 1 - the distance; right of it the highs. BPF: nothing of them is
/// stretched; they pass live, the side's band at 1 - the distance.
pub fn stretch_bands(k1: f32, bpf: bool) -> (f32, f32, f32, f32) {
    let t = through_value(k1);
    match (k1 < 0.48627, bpf) {
        (true, false) => (1.0 - t, 0.0, 1.0, 0.0),
        (true, true) => (0.0, 1.0 - t, 0.0, 1.0),
        (false, false) => (1.0, 0.0, 1.0 - t, 0.0),
        (false, true) => (0.0, 1.0, 0.0, 1.0 - t),
    }
}

pub fn stretch_speed(level: f32) -> f32 {
    if level >= 0.5 { 0.75 - 0.5 * level } else { 1.0 - level }
}

/// fx9: STRETCH's pitch shift (`StretchCore::updatePitchSpeed` 0x102a4780c, every block):
/// REPITCH 0 (the pitch falls with the speed), else 1 / speed - 1 within -2 .. 3
/// (`PitchShifter::setPitchParameterDirect` 0x1028ceb34).
pub fn stretch_shift(speed: f32, repitch: bool) -> f32 {
    if repitch { 0.0 } else { (1.0 / speed - 1.0).clamp(-2.0, 3.0) }
}

/// PHASER's all-pass stages (left, right) for single mode's second knob
/// (`SingleBeatFxPhaser::updateParameter` 0x102a0d14c, KnobPara2Assign 0 as the single-mode
/// layer makes it): 6 and 6 at the centre; turned left the left channel's count rises to 60
/// (`PhaserCore` "Stages Lch" range 2..60), turned right the right channel's, in even steps
/// (6 + trunc(v x 54), an odd count one more).
pub fn phaser_stages(k: f32) -> (usize, usize) {
    if (0.48627..=0.51373).contains(&k) {
        return (6, 6);
    }
    let mut n = (through_value(k) * 54.0) as usize + 6;
    if n % 2 == 1 {
        n += 1;
    }
    let n = n.min(60);
    if k < 0.5 { (n, 6) } else { (6, n) }
}

/// REVERB (`MultiBeatFxReverb::CoreParameterCalculator`), for the percent value `p` (1..100):
/// the size factor the comb times are multiplied by.
pub fn reverb_size(p: f32) -> f32 {
    0.2 + 0.17101 * (0.02306 * p + 1.01).powi(2)
}

/// The reverb's input gain.
pub fn reverb_in_gain(p: f32) -> f32 {
    0.2 - 6.6116e-5 * (p - 55.0).powi(2) + if p >= 91.0 { ((p - 90.0) * 0.1).powi(4) * 0.016529 } else { 0.0 }
}

/// The reverb's decay time (ms to -60 dB). rekordbox keeps ten 100-step tables of comb gains;
/// every entry is 10^(-3 * comb delay / this time), and this polynomial fits the time within
/// 4 % (0.77 s at 1 %, 9.1 s at 50 %, 250 s at 100 %).
pub fn reverb_rt60_ms(p: f32) -> f32 {
    let p = p.clamp(1.0, 100.0);
    (((((2.27830833e-9 * p - 5.30411715e-7) * p + 3.42242528e-5) * p + 6.5895372e-5) * p + 1.18306299e-2) * p + 6.6592768).exp()
}

/// Single-mode REVERB's DECAY (`SingleBeatFxReverb::getDynamicParameter` 0x102a11c74): the
/// knob's percent `d` (1..50..100, read as ROOM SIZE is) gives the time to -60 dB in seconds
/// (1.53 s at 1 %, 8.7 s at 50 %, 101 s at 100 %); ROOM SIZE alone sets the size.
pub fn reverb_decay_secs(d: f32) -> f32 {
    let d = d.clamp(1.0, 100.0);
    let d2 = d * d;
    d2 * d2 * 0.0000028728 + d2 * (d * -0.00033391) + d2 * 0.016808 + d * -0.22553 + 1.7409
}

/// Its input gain: 0.2, falling to 0.1 as DECAY turns right of the centre.
pub fn reverb_decay_in_gain(k: f32) -> f32 {
    if k > 0.51373 { 0.2 - 0.1 * half_value(k) } else { 0.2 }
}

/// The reverb's comb delays at size 1 (ms; left and right of each of the five), and the two
/// all-passes' (left, right).
pub const REVERB_COMB_MS: [f32; 10] = [22.0938, 38.6667, 39.9167, 23.2292, 23.8542, 40.8646, 41.5, 24.802, 21.5, 23.885];
pub const REVERB_AP_MS: [f32; 4] = [6.97917, 8.23958, 22.7396, 21.4792];

/// VINYL BRAKE's speed step (`VinylBrakeCore`): the speed falls in a straight line that would
/// reach zero after the decay time, and below 1/8 at a fixed 0.20559 per second.
#[inline]
pub fn brake_step(speed: f32, decay_secs: f32, rate: f32) -> f32 {
    (speed - if speed >= 0.125 { 1.0 / (decay_secs * rate) } else { 0.20559 / rate }).max(0.0)
}

/// A curve through evenly spaced points, interpolated in the logarithm (fits of rekordbox's
/// frequency tables).
pub fn knots(k: &[f32], x: f32) -> f32 {
    let f = x.clamp(0.0, 1.0) * (k.len() - 1) as f32;
    let i = (f as usize).min(k.len() - 2);
    (k[i].ln() + (k[i + 1].ln() - k[i].ln()) * (f - i as f32)).exp()
}

/// Beat FX FILTER's cutoffs over the LFO's travel (`LFOFilterCore`'s 129-step tables, every
/// eighth step): the low-pass and the lower edge of its resonance band, and the high-pass
/// (whose resonance band ends at four times its cutoff).
const LFO_LPF_MAIN: [f32; 17] = [23718.6, 16080.8, 12517.8, 9916.9, 7908.7, 6316.7, 5034.8, 3992.9, 3142.3, 2448.0, 1884.6, 1433.3, 1080.0, 814.7, 629.9, 521.0, 485.0];
const LFO_LPF_RESO: [f32; 17] = [16845.1, 6351.0, 3601.2, 2201.0, 1399.3, 914.6, 612.7, 421.3, 298.3, 218.1, 165.0, 129.2, 104.8, 88.2, 77.3, 71.2, 69.1];
const LFO_HPF_MAIN: [f32; 17] = [51.8, 112.7, 157.3, 210.3, 285.4, 390.9, 533.6, 722.2, 969.9, 1296.3, 1728.4, 2300.9, 3055.7, 4040.5, 5306.9, 6907.4, 8892.1];

/// The FX+ pack's filter cutoffs for the main knob in 8-bit steps `v` (`FxPlus*Echo::
/// updateFilterCoef`): LPF ECHO 20 kHz down to 96 Hz, HPF ECHO 60 Hz up to 8.5 kHz, BPF ECHO
/// 150 Hz up to 8.7 kHz, CRUSH ECHO's high-pass 60 Hz up to 4 kHz.
pub fn fxplus_fc(kind: Kind, v: f32) -> f32 {
    let n = (v - 4.0).max(0.0);
    match kind {
        Kind::LpfEcho => 20000.0 * 0.97895318f32.powf(251.0 * (n / 251.0).powf(0.78)),
        Kind::HpfEcho => 60.0 * 1.0201633f32.powf(n),
        Kind::BpfEcho => 150.0 * 1.0163087f32.powf(251.0 * (n / 251.0).powf(1.1)),
        _ => 60.0 * 1.0168726f32.powf(n),
    }
}

/// The FX+ echoes' share of the plain and the filtered sound for the main knob `v` (0..255).
pub fn fxplus_mix(kind: Kind, v: f32) -> (f32, f32) {
    let g = if v < 4.0 { 1.0 } else { 1.0159 - 0.0039841 * v };
    let g3 = g * g * g;
    match kind {
        Kind::LpfEcho => {
            if v <= 3.0 {
                (1.0, 0.0)
            } else if v < 32.0 {
                let x = (v - 4.0) * 0.035714;
                (1.0 - x, x)
            } else if v < 224.0 {
                (0.0, 1.0)
            } else {
                (0.0, (255.0 - v) * 0.015625 + 0.5)
            }
        }
        Kind::HpfEcho => (g3, (1.0 - g3) * if v >= 231.0 { (255.0 - v) * 0.012 + 0.7 } else { 1.0 }),
        Kind::BpfEcho => (1.0, 1.0 - g3),
        _ => (g3, 1.0 - g3),
    }
}

const MAX_RATE: usize = 48000;
/// Longest delay (4 s) and the history kept for ROLL and RELEASE FX (15 s), in frames.
const DELAY_LEN: usize = 4 * MAX_RATE;
const RING_LEN: usize = 15 * MAX_RATE;
/// The FX+ echoes share the delay line: four parts of a second each.
const PART_LEN: usize = DELAY_LEN / 4;
/// SPIRAL UP / DOWN's line: a second.
const AUX_LEN: usize = MAX_RATE;
/// On/off and gate smoothing, as rekordbox's 3.3333 ms.
const SMOOTH_SECS: f32 = 0.0033333;
const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::TAU;

/// A state-variable filter step: (low, band, high). `g` = tan(pi fc / rate), `k` = 1/Q.
#[inline]
fn svf(s: &mut [f32; 2], x: f32, g: f32, k: f32) -> (f32, f32, f32) {
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    let v3 = x - s[1];
    let v1 = a1 * s[0] + a2 * v3;
    let v2 = s[1] + a2 * s[0] + g * a2 * v3;
    s[0] = 2.0 * v1 - s[0];
    s[1] = 2.0 * v2 - s[1];
    (v2, v1, x - k * v1 - v2)
}

/// Through up to 1.5, then rounded off toward 2.5.
#[inline]
fn sat(x: f32) -> f32 {
    let a = x.abs();
    if a <= 1.5 { x } else { x.signum() * (1.5 + (a - 1.5) / (1.0 + (a - 1.5))) }
}

/// Beat FX FILTER's coefficient tables at `rate` (`LFOFilterCore::CoefficientTable`
/// 0x1029d5c84): for each of rekordbox's 129 cutoffs (read from the installed rekordbox; the
/// every-8th-step knots above without it) the cutoff is pre-warped once ((fs / pi) tan(pi
/// min(fc, 0.319 fs) / fs)) and handed to `IirFilter::calcCoef*1stOrder`, which warps it again;
/// its b0 is kept, clamped to 0 .. 1. Low-pass type: a low-pass main section on LPF_MAIN and a
/// high-pass resonance section on LPF_RESO; high-pass type: high-pass on HPF_MAIN, low-pass on
/// HPF_RESO.
pub fn lfo_filter_tables(hp: bool, rate: u32) -> [[f32; 129]; 2] {
    let r = rate as f64;
    let fcs = crate::cfx::fc_tables().and_then(|t| t.lfo.as_deref());
    let fc = |k: usize, i: usize| -> f64 {
        match fcs {
            Some(t) => t[k][i],
            None => match k {
                0 => knots(&LFO_LPF_MAIN, i as f32 / 128.0) as f64,
                1 => knots(&LFO_LPF_RESO, i as f32 / 128.0) as f64,
                2 => knots(&LFO_HPF_MAIN, i as f32 / 128.0) as f64,
                _ => 4.0 * knots(&LFO_HPF_MAIN, i as f32 / 128.0) as f64,
            },
        }
    };
    let b0 = |f: f64, lp: bool| {
        let w = ((r * 0.318309886) * (f.min(r * 0.319) * 3.14159265 * (1.0 / r)).tan()) as f32;
        crate::fx_core::iir_1st(w as f64, r, !lp)[0].clamp(0.0, 1.0)
    };
    let (mk, rk) = if hp { (2, 3) } else { (0, 1) };
    [std::array::from_fn(|i| b0(fc(mk, i), !hp)), std::array::from_fn(|i| b0(fc(rk, i), hp))]
}

/// FLANGER's smoothers at rest: feedback 0, gains 1, the sweep of one frame .. 2.6667 ms.
const FL_INIT: [crate::fx_core::SmoothF; 7] = {
    use crate::fx_core::SmoothF as S;
    [S::new_const(0.0, 3.3333), S::new_const(1.0, 3.3333), S::new_const(1.0, 3.3333), S::new_const(1.344, 50.0), S::new_const(1.344, 50.0), S::new_const(1.323, 50.0), S::new_const(1.323, 50.0)]
};

/// fx7b: `RingBuffer::readByLagrangeInterpolation`'s four-point Lagrange (frames -1, 0, 1, 2 at
/// `t`), with its constants (0.16667 for 1/6), in float.
#[inline]
pub(crate) fn lagrange4(qm1: f32, q0: f32, q1: f32, q2: f32, t: f32) -> f32 {
    let (t2, t3) = (t * t, t * t * t);
    let a = t3 - t;
    let b = t + t;
    (qm1 * t2 + q0 * ((a - (t2 + t2)) + 2.0) - q1 * (t3 - (b + t2))) * 0.5 + -(qm1 * (t3 + b) - q2 * a) * 0.16667
}

/// Keeps a delay line finite (64 = +36 dB over full scale; never reached by a track).
#[inline]
fn guard(x: f32) -> f32 {
    x.clamp(-64.0, 64.0)
}

#[inline]
fn svf_g(fc: f32, rate: f32) -> f32 {
    (PI * (fc / rate).clamp(0.0002, 0.45)).tan()
}

/// First-order sections as rekordbox's `IirFilter::calcCoef*1stOrder` (bilinear): b0, b1, a1.
#[inline]
fn one_pole(fc: f32, rate: f32, hp: bool) -> [f32; 3] {
    let k = (PI * (fc / rate).min(0.319)).tan();
    if hp { [1.0 / (1.0 + k), -1.0 / (1.0 + k), (k - 1.0) / (k + 1.0)] } else { [k / (1.0 + k), k / (1.0 + k), (k - 1.0) / (k + 1.0)] }
}

/// What the echo core does this block.
#[derive(Clone, Copy)]
struct EchoCfg {
    /// Left and right delay as multiples of the slot's time.
    ml: f32,
    mr: f32,
    fb: f32,
    rev: bool,
    freeze: bool,
    /// Pitch ratio of each repeat (1 = none).
    ratio: f32,
    /// Filter on the echo path: 0 none, 1 low-pass, 2 high-pass.
    filt: u8,
    g: f32,
    /// More than 0: that many taps, an eighth of the time apart, and no feedback (MT DELAY).
    taps: u8,
    /// The loop goes through the spiral's limiter instead of the plain one.
    limit: bool,
    /// The line is rekordbox's `DelayModule` (whole frames, and a new time is taken by
    /// closing the read gate, jumping and opening it again).
    dm: bool,
}

/// One effect's state for one channel.
pub(crate) struct Slot {
    // fx7b
    /// SPIRAL / SPIRAL UP / SPIRAL DOWN: rekordbox's `SpiralCore` (`crate::fx_spiral`), and
    /// SPIRAL UP / DOWN's input gain (`RmxSpiral` 656, 0.66667 ms).
    spc: crate::fx_spiral::SpiralCore,
    rmx_ig: crate::fx_echo::GainSm,
    /// SPIRAL UP / DOWN's four echoes (`RmxFourOrderEcho`) on four parts of the delay line.
    rmx_echo: crate::fx_spiral::FourEcho,
    /// HELIX: rekordbox's `HelixCore` (`crate::fx_helix`) on the two halves of the delay line.
    hxc: crate::fx_helix::HelixCore,
    /// FX+ NOISE: rekordbox's `FxPlusNoiseEcho` (`crate::fx_noise`; its echo made when chosen).
    nzf: crate::fx_noise::NoiseFx,
    /// STRETCH's live (thru) share of the bands: a second split whose thru output is the
    /// knob's live gains (`EffectFrequencyModule::setTargetGain` sets fx and thru apart).
    b3t: crate::fx_echo::Bands3,
    /// REVERB UP / DOWN's knobs (`crate::fx_verb::FxRevKnobs`).
    rvk: crate::fx_verb::FxRevKnobs,
    /// REVERB / REVERB UP / REVERB DOWN: rekordbox's `ReverbCore` (`crate::fx_verb`).
    verb: crate::fx_verb::VerbCore,
    /// The ROLL family's beat (ms) at its turn-on (`SingleBeatFxRollBase` a1+1260).
    roll_beat_ms: f32,
    /// Beat FX VINYL BRAKE: rekordbox's `VinylBrakeCore` (`crate::fx_spin`), where its sound
    /// starts in the history, its speeds for the call (and how far they are read), the release
    /// path's low-pass memories and its cross-fade (`BeatFxVinylBrake` 752 / 768).
    vbc: crate::fx_spin::VinylCore,
    vb_start: usize,
    vb_sp: [f32; 512],
    vb_k: (usize, usize),
    vlp: [[f32; 2]; 2],
    vxf: crate::fx_lfo::Sf,
    /// fx9: `VinylBrakeCore`'s own history (`RingBufferForbiddenOverwriting`, written once from
    /// the brake's start with what the core is given: the module's fx bands), 6 s, made with
    /// the other cores (`rev_needs` bit 64); and how many frames it holds.
    vbh: Option<Box<[f32]>>,
    vbh_n: usize,
    /// STRETCH: `StretchCore`'s `SlowPlaybackFilter` and read gain (3.3 ms), and the frames
    /// since a restart while that gain shuts (-1: none pending).
    stf: crate::fx_spin::SlowPlay,
    stg: crate::fx_lfo::Sf,
    st_cnt: f64,
    /// fx9: STRETCH's read sound for the shifter's block (`StretchCore::processEffectInternal`
    /// runs the read and `SlowPlaybackFilter` over the block before `processPitchShift`), and
    /// how far it is used.
    st_pre: [[f32; 2]; crate::fx_core::PitchShifter::BLOCK],
    st_i: usize,
    st_len: usize,
    kind: Kind,
    /// Stereo delay line (interleaved) and its write position (frames).
    delay: Vec<f32>,
    w: usize,
    /// Delay time in use (frames), moving toward the wanted time.
    time: f32,
    /// SPIRAL's read point (`SpiralBuffer`).
    spiral: crate::fx_core::SpiralMotion,
    /// Wet gain in use (0 off .. 1 on).
    gate: f32,
    lfo: f32,
    f1: [[f32; 2]; 2],
    f2: [[f32; 2]; 2],
    f3: [[f32; 2]; 2],
    lp: [f32; 2],
    ap: [[f32; 60]; 2],
    fb: [f32; 2],
    /// Beat FX FILTER: per channel, the six first-order sections' memories and the resonance
    /// path's last output.
    lf: [[f32; 13]; 2],
    /// Its coefficient tables (`LFOFilterCore::CoefficientTable`: the b0 of the main and the
    /// resonance sections at each of the 129 LFO steps) and the rate and type they are for.
    lfb: [[f32; 129]; 2],
    lfb_made: (u32, bool),
    /// ROLL / REV ROLL / SLIP ROLL: rekordbox's roll core (its buffer is `delay`).
    roll: crate::fx_roll::RollFx,
    was_on: bool,
    /// Odd turn of the LFO (SWING).
    odd: bool,
    /// rekordbox's pitch shifter, and the rate it is set up for.
    ps: crate::fx_core::PitchShifter,
    ps_rate: u32,
    ej: crate::fx_core::EnigmaJet,
    mob: crate::fx_core::Mobius,
    /// The echo core's loop limit (`EchoCfg::limit`): the input's peak; SPIRAL UP / DOWN's own
    /// line (stereo).
    pk: f32,
    aux: Vec<f32>,
    aux_w: usize,
    /// MT DELAY's eight tap gains in use.
    taps: [f32; 8],
    /// ROBOT: the line's delay and step, the feedback carried to the next frame; its two
    /// gains in use.
    rob: [f32; 4],
    ga: [f32; 2],
    /// Beat FX VINYL BRAKE: state (0 idle, 1 braking, 2 release) and highest LEVEL, the
    /// release's start LEVEL and speed, its sinking, the last LEVEL, its gain.
    vb_state: u8,
    vb: [f32; 6],
    /// The reverse echo core of REV DELAY and the REV buttons (`crate::fx_rev`; made by the
    /// interface for the slots that can run one), and whether it must start clean.
    rv: Option<Box<crate::fx_rev::RevFx>>,
    fresh: bool,
    /// `BeatFxEcho` / `BeatFxPitchEcho`'s gates (`SignalGate`, 3.3333 ms): the input (shut by
    /// FREEZE), the forward core's output (open without REV) and the reverse core's (with
    /// it); the line holds the longest delay (`f588`) and the frames counted for it (`f592`).
    bg: [crate::fx_lfo::Sf; 3],
    filled: bool,
    fill_n: i32,
    /// FLANGER (`BeatFxFlanger`): its two `LFO`s (triangles giving each side's delay in ms,
    /// smoothed over 50 ms), the output gate (`SignalGate` 3.3333 ms), a restart asked for
    /// (`f632`), the gate shutting for one (`f633`), and the period waiting for it (`f736`,
    /// `f740`).
    flo: [crate::fx_lfo::Lfo; 2],
    fgate: crate::fx_lfo::Sf,
    fplay: bool,
    fclose: bool,
    fpend: bool,
    fpend_ms: f32,
    seed: u32,
    hold: [f32; 2],
    hold_n: f32,
    /// VINYL BRAKE / STRETCH: how far behind the live sound playback is (frames), frames
    /// into the current turn, and the brake's speed.
    lag: f64,
    t: f32,
    speed: f32,
    /// MOBIUS: oscillator phases and the input's envelope. The FX+ echoes' second LFO.
    osc: [f32; 6],
    env: f32,
    lfo2: f32,
    /// `DelayModule`'s read gate (`SignalGate`, 3.3333 ms), the time waiting for it to close
    /// (frames, 0 = none), the frames written since the line was cleared, the gate this frame.
    dg: [crate::fx_core::SmoothF; 2],
    dpend: f32,
    dfill: f32,
    gv: [f32; 2],
    /// The echoes' FILTER knob (`SingleBeatFxEcho`: an `InstFxHPF` then an `InstFxLPF` on the
    /// repeats).
    ifl: [crate::cfx::InstFilter; 2],
    /// FLANGER (`FlangerCore`): the feedback, each side's output gain, each side's sweep centre
    /// and half-range (ms; the LFO's range moves over 50 ms), all `SmoothFloat`s.
    fl: [crate::fx_core::SmoothF; 7],
    /// TRANS / PAN: rekordbox's beat quantizer (`FxQuantizeManager`) and the effect it drives
    /// (`BeatFxTrans` on two square LFOs, `BeatFxPan` on two triangle ones).
    tq: crate::fx_lfo::Quant,
    trans: crate::fx_lfo::TransFx,
    pan: crate::fx_lfo::PanFx,
    /// FILTER (`BeatFxFilter` on four LFOs) and PHASER (`BeatFxPhaser` on two), with their
    /// gates and cores (`crate::fx_sweep`).
    bf: crate::fx_sweep::BeatFilter,
    bp: crate::fx_sweep::BeatPhaser,
    /// The echoes' time as the beat quantizer last gave it (`setEffectBeatTimeWithPlayHead`,
    /// ms; 0 = not yet).
    qms: f32,
    // fx7a
    /// The forward echo core of DELAY, ECHO, LOW CUT ECHO and PING PONG (`EchoCore`, two
    /// `DelayModule`s: `crate::fx_echo`), made by the interface (`ensure_rev`); a slot without
    /// it runs the line above.
    ec: Option<Box<crate::fx_echo::EchoCore>>,
    /// The band split (`EffectFrequencyModule`) of single DELAY / REV DELAY and LOW CUT ECHO.
    b3: crate::fx_echo::Bands3,
    /// UP ECHO / DOWN ECHO / PITCH ECHO's core (`PitchEchoCore`, with its own pitch shifter),
    /// made by the interface (`ensure_rev`).
    pe: Option<Box<crate::fx_echo::PitchEchoCore>>,
    /// MT DELAY's core (`MultiTapDelayCore`), made by the interface (`ensure_rev`).
    mt: Option<Box<crate::fx_echo::MtCore>>,
    /// fx8a: BPF / HPF / LPF / CRUSH ECHO's body (`crate::fx_echo_plus`, with `Rmx4thEcho`'s
    /// cores), made by the interface (`ensure_rev`) for the slot's mode.
    fxp: Option<Box<crate::fx_echo_plus::FxPlusEcho>>,
    /// fx8a: single DELAY / REV DELAY's pan (`SingleBeatFxDelayBase::BfxPan` and its shares).
    dpan: crate::fx_echo::DelayPan,
}

/// The beat quantizer's calls for the echo family over one buffer: the stretches between
/// restarts and, before each, whether the effect was restarted (`setEffectBeatTimeWithPlayHead`
/// or `playHead`, which end in the core's `playHead`) or switched on (REV DELAY:
/// `BeatFxDelay::onCalledTurnOn`, `playHead(1, 1)`).
struct EchoSync {
    n: usize,
    at: [u32; 128],
    len: [u32; 128],
    play: [bool; 128],
    both: [bool; 128],
    /// `setEffectBeatTimeWithPlayHead`'s time, `setTimeUntilNextPlayHead`'s (ms; 0 = none) and
    /// `onCalledTurnOn`, before each stretch.
    beat_ms: [f32; 128],
    until_ms: [f32; 128],
    on: [bool; 128],
    pend: bool,
    pend_both: bool,
    pend_ms: f32,
    pend_until: f32,
    pend_on: bool,
    turn_on_both: bool,
}

impl EchoSync {
    fn new(turn_on_both: bool) -> EchoSync {
        EchoSync { n: 0, at: [0; 128], len: [0; 128], play: [false; 128], both: [false; 128], beat_ms: [0.0; 128], until_ms: [0.0; 128], on: [false; 128], pend: false, pend_both: false, pend_ms: 0.0, pend_until: 0.0, pend_on: false, turn_on_both }
    }

    fn seg(&mut self, at: usize, len: usize) {
        if self.n < 128 {
            self.at[self.n] = at as u32;
            self.len[self.n] = len as u32;
            self.play[self.n] = self.pend;
            self.both[self.n] = self.pend_both;
            self.beat_ms[self.n] = self.pend_ms;
            self.until_ms[self.n] = self.pend_until;
            self.on[self.n] = self.pend_on;
            self.n += 1;
        } else {
            self.len[127] = (at + len) as u32 - self.at[127];
        }
        self.pend = false;
        self.pend_both = false;
        self.pend_ms = 0.0;
        self.pend_until = 0.0;
        self.pend_on = false;
    }
}

impl crate::fx_lfo::BeatSync for EchoSync {
    fn play_head(&mut self) {
        self.pend = true;
    }
    fn set_beat_time(&mut self, ms: f32, _offset: f32) {
        self.pend = true;
        self.pend_ms = ms.abs().max(1e-3);
    }
    fn time_until_next(&mut self, ms: f32) {
        self.pend_until = ms.max(1e-3);
    }
    fn turn_on(&mut self, _offset: f32) {
        self.pend_on = true;
        if self.turn_on_both {
            self.pend_both = true;
        }
    }
    fn turn_off(&mut self) {}
}

/// Frames of TRANS / PAN gain worked out at a time (on the stack).
const LFO_PIECE: usize = 1024;

/// fx9: VINYL BRAKE's history, stereo frames (`VinylBrakeCore`'s ring: 6 s; the core reads
/// at most 6 s from the start, and one frame past it).
const VB_HIST: usize = 6 * MAX_RATE + 2;

fn new_vb_history() -> Box<[f32]> {
    vec![0.0f32; VB_HIST * 2].into_boxed_slice()
}

/// Effect cores prepared on the interface thread, outside the shared audio rack lock.
#[derive(Default)]
pub struct NewCores {
    rv: Option<Box<crate::fx_rev::RevFx>>,
    ec: Option<Box<crate::fx_echo::EchoCore>>,
    pe: Option<Box<crate::fx_echo::PitchEchoCore>>,
    mt: Option<Box<crate::fx_echo::MtCore>>,
    fxp: Option<Box<crate::fx_echo_plus::FxPlusEcho>>,
    nz: Option<crate::fx_noise::NoiseFx>,
    vbh: Option<Box<[f32]>>,
}

impl NewCores {
    /// The cores of `bits` (`Slot::rev_needs`). This is where the memory is taken.
    pub fn make(bits: u8, single: bool) -> NewCores {
        NewCores {
            rv: (bits & 1 != 0).then(crate::fx_rev::RevFx::new),
            ec: (bits & 2 != 0).then(crate::fx_echo::EchoCore::new),
            pe: (bits & 4 != 0).then(crate::fx_echo::PitchEchoCore::new),
            mt: (bits & 8 != 0).then(crate::fx_echo::MtCore::new),
            fxp: (bits & 16 != 0).then(|| crate::fx_echo_plus::FxPlusEcho::new(single)),
            nz: (bits & 32 != 0).then(|| crate::fx_noise::NoiseFx::new(48000.0)),
            vbh: (bits & 64 != 0).then(new_vb_history),
        }
    }
}

impl Slot {
    pub(crate) fn new() -> Slot {
        Slot {
            // fx7b
            spc: crate::fx_spiral::SpiralCore::new(48000.0, 0, false),
            rmx_ig: crate::fx_echo::GainSm::new(0.0),
            rmx_echo: crate::fx_spiral::FourEcho::new(PART_LEN, 4096),
            hxc: crate::fx_helix::HelixCore::new(48000.0),
            nzf: crate::fx_noise::NoiseFx::empty(),
            b3t: crate::fx_echo::Bands3::new(),
            rvk: crate::fx_verb::FxRevKnobs::new(48000.0),
            verb: crate::fx_verb::VerbCore::new(48000.0),
            roll_beat_ms: 500.0,
            vbc: crate::fx_spin::VinylCore::new(48000.0),
            vb_start: 0,
            vb_sp: [0.0; 512],
            vb_k: (0, 0),
            vlp: [[0.0; 2]; 2],
            vxf: crate::fx_rev::new_gate(48000.0),
            vbh: None,
            vbh_n: 0,
            stf: crate::fx_spin::SlowPlay::new(48000.0),
            stg: { let mut g = crate::fx_lfo::Sf::new(48000.0); g.set_time(3.3); g.reset(1.0); g },
            st_cnt: -1.0,
            st_pre: [[0.0; 2]; crate::fx_core::PitchShifter::BLOCK],
            st_i: 0,
            st_len: 0,
            delay: vec![0.0; DELAY_LEN * 2], ps: crate::fx_core::PitchShifter::new(48000.0), ej: crate::fx_core::EnigmaJet::new(), mob: crate::fx_core::Mobius::new(), aux: vec![0.0; AUX_LEN * 2], ..Slot::empty() }
    }

    /// A slot without its lines, for a place that is never switched on.
    pub(crate) fn empty() -> Slot {
        Slot {
            // fx7b
            spc: crate::fx_spiral::SpiralCore::new(48000.0, 0, false),
            rmx_ig: crate::fx_echo::GainSm::new(0.0),
            rmx_echo: crate::fx_spiral::FourEcho::empty(),
            hxc: crate::fx_helix::HelixCore::new(48000.0),
            nzf: crate::fx_noise::NoiseFx::empty(),
            b3t: crate::fx_echo::Bands3::new(),
            rvk: crate::fx_verb::FxRevKnobs::new(48000.0),
            verb: crate::fx_verb::VerbCore::empty(),
            roll_beat_ms: 500.0,
            vbc: crate::fx_spin::VinylCore::new(48000.0),
            vb_start: 0,
            vb_sp: [0.0; 512],
            vb_k: (0, 0),
            vlp: [[0.0; 2]; 2],
            vxf: crate::fx_rev::new_gate(48000.0),
            vbh: None,
            vbh_n: 0,
            stf: crate::fx_spin::SlowPlay::new(48000.0),
            stg: { let mut g = crate::fx_lfo::Sf::new(48000.0); g.set_time(3.3); g.reset(1.0); g },
            st_cnt: -1.0,
            st_pre: [[0.0; 2]; crate::fx_core::PitchShifter::BLOCK],
            st_i: 0,
            st_len: 0,
            kind: Kind::Delay, delay: Vec::new(), w: 0, time: 0.0, spiral: crate::fx_core::SpiralMotion::new(48000.0, 0), gate: 0.0, lfo: 0.0, f1: [[0.0; 2]; 2], f2: [[0.0; 2]; 2], f3: [[0.0; 2]; 2], lp: [0.0; 2], ap: [[0.0; 60]; 2], fb: [0.0; 2], lf: [[0.0; 13]; 2], lfb: [[0.0; 129]; 2], lfb_made: (0, false), roll: crate::fx_roll::RollFx::new(crate::fx_roll::RollKind::Roll, 48000.0), was_on: false, odd: false, ps: crate::fx_core::PitchShifter::empty(), ps_rate: 48000, ej: crate::fx_core::EnigmaJet::empty(), mob: crate::fx_core::Mobius::empty(), pk: 0.0, aux: Vec::new(), aux_w: 0, taps: [0.0; 8], rob: [1.0, 0.177778, 0.0, 0.0], ga: [0.75, 0.75], vb_state: 0, vb: [0.0; 6], rv: None, fresh: false, bg: [crate::fx_rev::new_gate(48000.0); 3], filled: false, fill_n: 0, flo: [crate::fx_lfo::Lfo::new(48000.0, crate::fx_lfo::Wave::TRI); 2], fgate: crate::fx_rev::new_gate(48000.0), fplay: false, fclose: false, fpend: false, fpend_ms: 10.0, seed: 0x2468_ACE1, hold: [0.0; 2], hold_n: 0.0, lag: 0.0, t: 0.0, speed: 1.0, osc: [0.0; 6], env: 0.0, lfo2: 0.0, dg: [crate::fx_core::SmoothF::new(0.0, 3.3333, 48000.0); 2], dpend: 0.0, dfill: 0.0, gv: [1.0; 2], ifl: [crate::cfx::InstFilter::new(crate::cfx::InstKind::Hpf), crate::cfx::InstFilter::new(crate::cfx::InstKind::Lpf)], fl: FL_INIT, tq: crate::fx_lfo::Quant::new(), trans: crate::fx_lfo::TransFx::new(48000.0, crate::fx_lfo::Wave::SQUARE), pan: crate::fx_lfo::PanFx::new(48000.0), bf: crate::fx_sweep::BeatFilter::new(48000.0), bp: crate::fx_sweep::BeatPhaser::new(48000.0), qms: 0.0,
            // fx7a
            ec: None,
            b3: crate::fx_echo::Bands3::new(),
            pe: None,
            mt: None,
            fxp: None,
            dpan: crate::fx_echo::DelayPan::new(48000.0),
        }
    }

    fn clear(&mut self) {
        // fx7b
        self.spc.selected();
        self.rmx_echo.clear();
        self.hxc.turn_off();
        self.nzf.selected();
        self.rvk = crate::fx_verb::FxRevKnobs::new(self.ps_rate as f32);
        self.verb.selected();
        self.vbc.state = 2;
        self.vb_k = (0, 0);
        self.vlp = [[0.0; 2]; 2];
        self.vxf.reset(0.0);
        self.stf.clear();
        self.stg.reset(1.0);
        self.st_cnt = -1.0;
        self.delay.iter_mut().for_each(|v| *v = 0.0);
        self.ps.reset();
        self.ej.reset();
        self.mob.reset(false);
        self.aux.iter_mut().for_each(|v| *v = 0.0);
        self.pk = 0.0;
        self.taps = [0.0; 8];
        self.rob = [1.0, 0.177778, 0.0, 0.0];
        self.ga = [0.75, 0.75];
        self.vb_state = 0;
        self.vb = [0.0; 6];
        self.f1 = [[0.0; 2]; 2];
        self.f2 = [[0.0; 2]; 2];
        self.f3 = [[0.0; 2]; 2];
        self.lp = [0.0; 2];
        self.ap = [[0.0; 60]; 2];
        self.fb = [0.0; 2];
        self.lf = [[0.0; 13]; 2];
        self.time = 0.0;
        self.spiral.reset();
        self.lfo = 0.0;
        self.lfo2 = 0.0;
        self.roll = crate::fx_roll::RollFx::new(match self.kind { Kind::RevRoll => crate::fx_roll::RollKind::Rev, Kind::SlipRoll => crate::fx_roll::RollKind::Slip, _ => crate::fx_roll::RollKind::Roll }, self.ps_rate as f32);
        self.odd = false;
        self.fresh = false;
        self.filled = false;
        self.fill_n = 0;
        self.hold = [0.0; 2];
        self.hold_n = 0.0;
        self.lag = 0.0;
        self.t = 0.0;
        self.speed = 1.0;
        self.osc = [0.0; 6];
        self.env = 0.0;
        self.dg.iter_mut().for_each(|g| g.reset(0.0));
        self.dpend = 0.0;
        self.dfill = 0.0;
        self.gv = [1.0; 2];
        self.ifl[0].reset(0, 0);
        self.ifl[1].reset(0, 0);
        self.fl = FL_INIT;
        self.qms = 0.0;
        // (A new effect is made selected: its LFOs start over.)
        self.tq = crate::fx_lfo::Quant::new();
        self.tq.slip = self.kind == Kind::SlipRoll;
        self.trans = crate::fx_lfo::TransFx::new(self.ps_rate as f32, crate::fx_lfo::Wave::SQUARE);
        self.pan = crate::fx_lfo::PanFx::new(self.ps_rate as f32);
    }

    /// `BeatFxFlanger::updateAndSetParameter`: each side's range (`min` .. `max` ms) and PHASE,
    /// smoothed; the heads (0) are already in place.
    fn flanger_params(&mut self, min: [f32; 2], max: [f32; 2], rev: bool) {
        const E: f32 = 1.1920929e-7;
        for c in 0..2 {
            let l = &mut self.flo[c];
            if (min[c] - l.amin).abs() >= E || (max[c] - l.amax).abs() >= E {
                l.set_amplitude_range(true, min[c], max[c]);
            }
            if rev != l.reversed() {
                l.reverse_amplitude(rev, true);
            }
        }
    }

    /// The beat quantizer's calls before stretch `i` as `BeatFxFlanger` takes them:
    /// `setTimeUntilNextPlayHead` 0x1029fe87c (the LFOs re-timed to end the turn then, kept
    /// within 10 ms .. 32 s), `setEffectBeatTimeWithPlayHead` 0x1029fe84c (a new period for the
    /// next restart, and a restart), `playHead` and `onCalledTurnOn` (a restart).
    fn flanger_calls(&mut self, s: &EchoSync, i: usize) {
        if s.until_ms[i] > 0.0 {
            let old = self.flo[0].period_ms;
            for l in self.flo.iter_mut() {
                l.set_remaining_period_time(s.until_ms[i], true);
            }
            let p = self.flo[0].period_ms;
            if !(10.0..=32000.0).contains(&p) {
                self.flo.iter_mut().for_each(|l| l.set_period_time(old, true));
            } else {
                self.fpend_ms = p;
            }
        }
        if s.beat_ms[i] > 0.0 {
            self.fpend = true;
            self.fpend_ms = s.beat_ms[i].clamp(10.0, 32000.0);
            self.fplay = true;
        }
        if s.play[i] || s.on[i] {
            self.fplay = true;
        }
    }

    /// Make the reverse echo core if effect `k` can run one here (`single`: the single-mode slot).
    pub(crate) fn ensure_rev(&mut self, k: Kind, single: bool) {
        self.ensure_fx7b(k);
        let want = match k {
            Kind::RevDelay => true,
            Kind::Echo | Kind::LowCutEcho | Kind::PingPong | Kind::PitchEcho => single,
            _ => false,
        };
        if want && self.rv.is_none() && !self.delay.is_empty() {
            self.rv = Some(crate::fx_rev::RevFx::new());
        }
        if matches!(k, Kind::Delay | Kind::Echo | Kind::LowCutEcho | Kind::PingPong) && self.ec.is_none() && !self.delay.is_empty() {
            self.ec = Some(crate::fx_echo::EchoCore::new());
        }
        if matches!(k, Kind::PitchEcho | Kind::UpEcho | Kind::DownEcho) && self.pe.is_none() && !self.delay.is_empty() {
            self.pe = Some(crate::fx_echo::PitchEchoCore::new());
        }
        if k == Kind::MtDelay && self.mt.is_none() && !self.delay.is_empty() {
            self.mt = Some(crate::fx_echo::MtCore::new());
        }
        if matches!(k, Kind::BpfEcho | Kind::HpfEcho | Kind::LpfEcho | Kind::CrushEcho) && self.fxp.as_ref().is_none_or(|f| f.single() != single) && !self.delay.is_empty() {
            self.fxp = Some(crate::fx_echo_plus::FxPlusEcho::new(single));
        }
        if k == Kind::VinylBrake && self.vbh.is_none() && !self.delay.is_empty() {
            self.vbh = Some(new_vb_history());
        }
    }

    /// The cores `ensure_rev` would make for this kind and mode, as bits (1 reverse, 2 echo,
    /// 4 pitch echo, 8 multi tap, 16 FX+, 32 NOISE's echo, 64 VINYL BRAKE's history). Cheap: for the interface to ask under the rack's
    /// lock, so that the cores themselves (up to 25 MB) can be made without holding it: the
    /// audio thread waits only a quarter of a millisecond for that lock.
    pub(crate) fn rev_needs(&self, k: Kind, single: bool) -> u8 {
        if self.delay.is_empty() {
            return 0;
        }
        let want_rv = match k {
            Kind::RevDelay => true,
            Kind::Echo | Kind::LowCutEcho | Kind::PingPong | Kind::PitchEcho => single,
            _ => false,
        };
        let mut m = 0u8;
        if want_rv && self.rv.is_none() {
            m |= 1;
        }
        if matches!(k, Kind::Delay | Kind::Echo | Kind::LowCutEcho | Kind::PingPong) && self.ec.is_none() {
            m |= 2;
        }
        if matches!(k, Kind::PitchEcho | Kind::UpEcho | Kind::DownEcho) && self.pe.is_none() {
            m |= 4;
        }
        if k == Kind::MtDelay && self.mt.is_none() {
            m |= 8;
        }
        if matches!(k, Kind::BpfEcho | Kind::HpfEcho | Kind::LpfEcho | Kind::CrushEcho) && self.fxp.as_ref().is_none_or(|f| f.single() != single) {
            m |= 16;
        }
        // (FX+ NOISE's echo: `ensure_fx7b`.)
        if k == Kind::Noise && !self.nzf.ready() {
            m |= 32;
        }
        // (fx9: VINYL BRAKE's history.)
        if k == Kind::VinylBrake && self.vbh.is_none() {
            m |= 64;
        }
        m
    }

    /// Put cores made outside the lock in place. What they replace (or what turned out not
    /// to be needed any more) is handed back, to be dropped outside the lock too.
    pub(crate) fn rev_install(&mut self, k: Kind, single: bool, mut c: NewCores) -> NewCores {
        let m = self.rev_needs(k, single);
        if m & 1 != 0 && c.rv.is_some() {
            std::mem::swap(&mut self.rv, &mut c.rv);
        }
        if m & 2 != 0 && c.ec.is_some() {
            std::mem::swap(&mut self.ec, &mut c.ec);
        }
        if m & 4 != 0 && c.pe.is_some() {
            std::mem::swap(&mut self.pe, &mut c.pe);
        }
        if m & 8 != 0 && c.mt.is_some() {
            std::mem::swap(&mut self.mt, &mut c.mt);
        }
        if m & 16 != 0 && c.fxp.as_ref().is_some_and(|f| f.single() == single) {
            std::mem::swap(&mut self.fxp, &mut c.fxp);
        }
        if m & 32 != 0 {
            if let Some(nz) = c.nz.as_mut() {
                std::mem::swap(&mut self.nzf, nz);
            }
        }
        if m & 64 != 0 && c.vbh.is_some() {
            std::mem::swap(&mut self.vbh, &mut c.vbh);
        }
        c
    }

    /// fx8a: make the echo cores MERGE FX's slots can need (its patterns run DELAY and LOW CUT
    /// ECHO in multi mode, LOW CUT ECHO and PITCH ECHO in single mode: `EchoCore`,
    /// `PitchEchoCore` and the reverse core, about 8 MB). For the interface thread, once, where the
    /// MERGE FX engine is made (`mfx::UnitEng::new`); until then those slots run the older
    /// interleaved line.
    #[allow(dead_code)]
    pub(crate) fn ensure_merge_cores(&mut self) {
        self.ensure_rev(Kind::LowCutEcho, true);
        self.ensure_rev(Kind::PitchEcho, true);
    }

    /// fx7b: FX+ NOISE's echo, made when the effect is chosen (off the audio thread).
    fn ensure_fx7b(&mut self, k: Kind) {
        if k == Kind::Noise && !self.nzf.ready() && !self.delay.is_empty() {
            self.nzf = crate::fx_noise::NoiseFx::new(48000.0);
        }
    }

    /// fx9: STRETCH's `StretchCore::processEffectInternal` 0x102a47408 up to the pitch shifter
    /// for the `len` frames of `buf` from `n`: a restart every `total` frames takes the live
    /// place (the read gain shut over 3.3 ms, the jump, the gain opened again), the fx bands
    /// into its ring (the slot's delay line), read back by Lagrange at the speed, through the
    /// read gain and `SlowPlaybackFilter`. Into `st_pre`.
    fn stretch_pre(&mut self, buf: &[f32], n: usize, len: usize, total: f32, speed: f32) {
        for k in 0..len {
            if self.t >= total {
                self.t = 0.0;
                self.st_cnt = 0.0;
                self.stg.set_target(0.0);
            }
            if self.st_cnt >= 0.0 && !self.stg.on && self.stg.cur.abs() < 1.1920929e-7 {
                self.lag = self.st_cnt;
                self.st_cnt = -1.0;
                self.stg.set_target(1.0);
            }
            if self.st_cnt >= 0.0 {
                self.st_cnt += 1.0;
            }
            let (f, _) = self.b3.next([buf[(n + k) * 2], buf[(n + k) * 2 + 1]]);
            self.push(f[0], f[1]);
            let back = 1.0 + self.lag;
            let (wl, wr) = (self.lagrange(back, 0), self.lagrange(back, 1));
            self.lag += (1.0 - speed) as f64;
            let sg = self.stg.take();
            self.st_pre[k] = self.stf.frame([wl * sg, wr * sg], speed);
            self.t += 1.0;
        }
        self.st_i = 0;
        self.st_len = len;
    }

    /// fx7b: `RingBuffer::readByLagrangeInterpolation` (forward): four-point Lagrange through
    /// the frames before, at and two after the place `back` frames behind the write place.
    #[inline]
    fn lagrange(&self, back: f64, c: usize) -> f32 {
        let pos = (self.w as f64 - back).rem_euclid(DELAY_LEN as f64);
        let i = pos as usize % DELAY_LEN;
        let t = (pos - pos.floor()) as f32;
        let at = |k: usize| self.delay[(k % DELAY_LEN) * 2 + c];
        lagrange4(at(i + DELAY_LEN - 1), at(i), at(i + 1), at(i + 2), t)
    }

    /// Read the delay line `frames` back (fractional), channel `c`.
    #[inline]
    fn tap(&self, frames: f32, c: usize) -> f32 {
        let f = frames.clamp(1.0, (DELAY_LEN - 2) as f32);
        let i = f as usize;
        let t = f - i as f32;
        let a = self.delay[((self.w + DELAY_LEN - i) % DELAY_LEN) * 2 + c];
        let b = self.delay[((self.w + DELAY_LEN - i - 1) % DELAY_LEN) * 2 + c];
        a + (b - a) * t
    }

    #[inline]
    fn push(&mut self, l: f32, r: f32) {
        // rekordbox writes the sum as it is (`DelayModule::write`); the guard far above any
        // level a track reaches only keeps a runaway line finite.
        self.delay[self.w * 2] = guard(l);
        self.delay[self.w * 2 + 1] = guard(r);
        self.w = (self.w + 1) % DELAY_LEN;
    }

    /// One of the four parts the FX+ echoes share the line in: read `frames` back.
    #[inline]
    fn part_tap(&self, part: usize, frames: f32, c: usize) -> f32 {
        let i = (frames as usize).clamp(1, PART_LEN - 1);
        let w = self.w % PART_LEN;
        self.delay[(part * PART_LEN + (w + PART_LEN - i) % PART_LEN) * 2 + c]
    }

    #[inline]
    fn part_put(&mut self, part: usize, l: f32, r: f32) {
        let k = (part * PART_LEN + self.w % PART_LEN) * 2;
        self.delay[k] = guard(l);
        self.delay[k + 1] = guard(r);
    }

    #[inline]
    fn aux_tap(&self, frames: f32, c: usize) -> f32 {
        if self.aux.is_empty() {
            return 0.0;
        }
        let f = frames.clamp(1.0, (AUX_LEN - 2) as f32);
        let i = f as usize;
        let t = f - i as f32;
        let a = self.aux[((self.aux_w + AUX_LEN - i) % AUX_LEN) * 2 + c];
        let b = self.aux[((self.aux_w + AUX_LEN - i - 1) % AUX_LEN) * 2 + c];
        a + (b - a) * t
    }

    #[inline]
    fn aux_push(&mut self, l: f32, r: f32) {
        if self.aux.is_empty() {
            return;
        }
        self.aux[self.aux_w * 2] = l;
        self.aux[self.aux_w * 2 + 1] = r;
        self.aux_w = (self.aux_w + 1) % AUX_LEN;
    }

    /// Pitch shift by `ratio` through rekordbox's shifter (`rbxfrm::PitchShifter`, in
    /// `fx_core`).
    #[inline]
    fn shift(&mut self, l: f32, r: f32, ratio: f32) -> (f32, f32) {
        self.ps.set_shift(ratio - 1.0);
        self.ps.process(l, r)
    }

    /// The echo core (`EchoCore::processEffect`): read the repeats, colour them, write the
    /// input and the feedback. The output is the repeats alone.
    #[inline]
    fn echo(&mut self, il: f32, ir: f32, e: &EchoCfg, r: f32) -> (f32, f32) {
        let (mut el, mut er);
        if e.taps > 0 {
            el = 0.0;
            er = 0.0;
            for k in 1..=e.taps {
                let d = self.time * k as f32 / 8.0;
                el += self.tap(d * e.ml, 0);
                er += self.tap(d * e.mr, 1);
            }
            let g = 1.0 / (e.taps as f32).sqrt();
            el *= g;
            er *= g;
        } else if e.dm {
            // `DelayModule::read`: whole frames, through the read gate.
            el = self.tap((self.time * e.ml).trunc(), 0) * self.gv[0];
            er = self.tap((self.time * e.mr).max(0.010 * r).trunc(), 1) * self.gv[1];
        } else {
            el = self.tap(self.time * e.ml, 0);
            er = self.tap((self.time * e.mr).max(0.010 * r), 1);
        }
        if (e.ratio - 1.0).abs() > 1e-4 {
            (el, er) = self.shift(el, er, e.ratio);
        }
        let (il, ir) = if e.freeze { (0.0, 0.0) } else { (il, ir) };
        if e.limit {
            // SPIRAL: the loop is clipped at 0.6 of the input's peak.
            let th = (0.6 * self.pk).max(1e-4);
            self.push((il + el * e.fb).clamp(-th, th), (ir + er * e.fb).clamp(-th, th));
        } else {
            self.push(il + el * e.fb, ir + er * e.fb);
        }
        (el, er)
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// `multi`: a multi-mode slot (or a PAD FX pad), whose inner settings rekordbox derives
    /// from LEVEL/DEPTH alone; otherwise single mode, where the two knobs and two buttons set
    /// them. `beat`: where the channel's deck is on its beat grid at the start of `buf`
    /// (beats), when Beat FX quantize applies.
    /// TRANS / PAN: the next `gl.len()` frames of each side's gain (`BeatFxTrans` /
    /// `BeatFxPan` driven by `FxQuantizeManager`).
    #[allow(clippy::too_many_arguments)]
    fn lfo_piece(&mut self, kind: Kind, multi: bool, level: f32, k: (f32, f32), beat: Option<f64>, bpm: f32, rate: u32, cycle: f32, on: bool, gl: &mut [f32], gr: &mut [f32]) {
        use crate::fx_lfo::{PanPar, TransPar};
        let frames = gl.len();
        if kind == Kind::Trans {
            // (`MultiBeatFxTrans` / `SingleBeatFxTrans::updateParameter`.)
            self.trans.par = if multi { TransPar::multi(level) } else { TransPar::single(recommend_value(level), k.0, k.1) };
            let (q, fx) = (&mut self.tq, &mut self.trans);
            q.process(fx, beat, bpm, rate, frames, cycle, on, |fx, s, l| fx.gains(&mut gl[s..s + l], &mut gr[s..s + l]));
        } else {
            let last = self.pan.trans.last_period;
            self.pan.par = if multi { PanPar::multi(level) } else { PanPar::single(recommend_value(level), k.0, k.1, last) };
            let (q, fx) = (&mut self.tq, &mut self.pan);
            q.process(fx, beat, bpm, rate, frames, cycle, on, |fx, s, l| fx.gains(&mut gl[s..s + l], &mut gr[s..s + l]));
        }
    }

    /// ROLL family: the next `gl.len()` frames of the roll's sound and its place in the turn
    /// (rekordbox's roll core driven by `FxQuantizeManager`). `end`: the history's write place
    /// after these frames.
    #[allow(clippy::too_many_arguments)]
    fn roll_piece(&mut self, ring: &Ring, end: i64, beat: Option<f64>, bpm: f32, rate: u32, cycle: f32, on: bool, gl: &mut [f32], gr: &mut [f32], ph: &mut [f32]) {
        let frames = gl.len();
        let nrb = crate::fx_roll::RollFx::buffer_frames(rate as f32).min(DELAY_LEN);
        let (q, fx, rb) = (&mut self.tq, &mut self.roll, &mut self.delay[..nrb * 2]);
        let start = end - frames as i64;
        q.process(fx, beat, bpm, rate, frames, cycle, on, |fx, s, l| {
            let e = start + (s + l) as i64;
            fx.set_block(e, l as i64);
            let h = crate::fx_roll::Hist { buf: &ring.buf, len: RING_LEN, end: e, block: l as i64 };
            let p0 = fx.phase();
            fx.process(rb, &h, &mut gl[s..s + l], &mut gr[s..s + l]);
            let step = fx.phase_step();
            for (k, v) in ph[s..s + l].iter_mut().enumerate() {
                *v = (p0 + k as f32 * step).fract();
            }
        });
    }

    pub(crate) fn process(&mut self, p: &SlotParams, bpm: f32, buf: &mut [f32], rate: u32, ring: &Ring, beat: Option<f64>, multi: bool) {
        if p.kind != self.kind {
            self.kind = p.kind;
            self.clear();
            self.gate = 0.0;
            self.was_on = false;
        }
        if !p.on && self.gate <= 0.0 {
            self.was_on = false;
            return;
        }
        use Kind::*;
        let kind = self.kind;
        let r = rate as f32;
        if rate != self.ps_rate && !self.ps.is_empty() {
            self.ps_rate = rate;
            self.ps.prepare(r);
            self.ej.prepare(r);
            self.mob.prepare(r);
            self.trans.set_rate(r);
            self.pan.trans.set_rate(r);
            if matches!(self.kind, Kind::Filter | Kind::Phaser) {
                // (FILTER / PHASER are made again at the new rate.)
                self.fresh = false;
            }
        }
        let bpm = bpm.clamp(20.0, 1000.0);
        let beat_secs = 60.0 / bpm;
        let (table, _, percent) = beat_table(kind);
        // The effect's time: its beat value, or (no beat control) a beat.
        let beats = if table.is_empty() || percent { 1.0 } else { p.value.abs().max(0.0625) };
        let down = !percent && p.value < 0.0;
        let secs = (beats * beat_secs).clamp(0.010, 32.0);
        let gate_k = 1.0 / (SMOOTH_SECS * r);
        let level = p.level.clamp(0.0, 1.0);
        let (k1, k2) = (p.knob[0].clamp(0.0, 1.0), p.knob[1].clamp(0.0, 1.0));
        let (b1, b2) = (p.btn[0], p.btn[1]);
        let frames = buf.len() / 2;
        let starting = p.on && !self.was_on;
        self.was_on = p.on;
        let lfo_step = 1.0 / (secs * r);
        // Beat FX quantize: the LFO's phase is the deck's place in the beat value.
        let grid = beat.map(|b| (b / beats as f64).rem_euclid(1.0) as f32);
        if let Some(want) = grid {
            let err = (want - self.lfo + 1.5).rem_euclid(1.0) - 0.5;
            if starting || err.abs() > 0.02 {
                self.lfo = want;
                self.odd = ((beat.unwrap() / beats as f64).floor() as i64) & 1 == 1;
            } else {
                self.lfo = (self.lfo + err * 0.2).rem_euclid(1.0);
            }
        }
        if starting {
            if grid.is_none() {
                self.lfo = 0.0;
                self.odd = false;
            }
            self.lag = 0.0;
            self.t = 0.0;
            self.speed = 1.0;
        }
        // DELAY / REV DELAY's first knob in single mode: the pan on the delay's output
        // (fx8a: `crate::fx_echo::DelayPan`).
        let use_dpan = !multi && matches!(kind, Delay | RevDelay);
        // SPIRAL UP / DOWN's laws (main knob, beat).
        let rmx = rmx_spiral_law(level, kind == SpiralDown, p.on, (beat_secs * 1000.0).clamp(10.0, 3990.0));
        // The beat quantizer's calls for this buffer (`FxQuantizeManager::processQuantizeEffect`)
        // for the effects that take them here: the echoes, FLANGER, FILTER, PHASER.
        let echo_fam = matches!(kind, Delay | RevDelay | Echo | LowCutEcho | PingPong | UpEcho | DownEcho | PitchEcho);
        let mut sync = EchoSync::new(kind == RevDelay);
        if echo_fam || matches!(kind, Flanger | Filter | Phaser) {
            let mut q = self.tq;
            q.process(&mut sync, beat, bpm, rate, frames, if down { -beats } else { beats }, p.on, |s, at, len| s.seg(at, len));
            self.tq = q;
        }
        // The echoes run at the time the quantizer gives them (`setEffectBeatTimeWithPlayHead`:
        // whole frames of the beat value at the tempo of that restart), taken at each restart:
        // a new beat value or tempo reaches them at the next one. Here the time at the buffer's
        // start; a later restart in the buffer re-times them at its frame.
        if echo_fam && sync.n > 0 && sync.at[0] == 0 && sync.beat_ms[0] > 0.0 {
            self.qms = sync.beat_ms[0];
        }
        let q_secs = if echo_fam && self.qms > 0.0 { self.qms * 0.001 } else { secs };
        if echo_fam {
            if let Some(i) = (0..sync.n).rev().find(|&i| sync.beat_ms[i] > 0.0) {
                self.qms = sync.beat_ms[i];
            }
        }
        // Time the delay line runs at (rekordbox: 1 ms .. 4 s; SPIRAL 10 ms .. 3.99 s).
        let echo_secs = match kind {
            VinylBrake => vinyl_echo_beats(if multi { 0.5 } else { k2 }).unwrap_or(1.0) * beat_secs,
            // FX+ NOISE: half a beat (10 ms .. 4 s).
            Noise => (0.5 * beat_secs).clamp(0.010, 4.0),
            // SPIRAL UP / DOWN: rekordbox's delay law (in their own line).
            SpiralUp | SpiralDown => rmx.1 * 0.001,
            Helix => if multi { secs } else { helix_time(k1, secs * 1000.0) * 0.001 },
            Spiral => secs.clamp(0.010, 3.99),
            _ => q_secs,
        };
        let want_time = (echo_secs.min(4.0) * r).min((DELAY_LEN - 2) as f32).min(if matches!(kind, SpiralUp | SpiralDown) { (AUX_LEN - 4) as f32 } else { f32::MAX });
        // SPIRAL's read point moves as rekordbox's `SpiralBuffer` moves it (a new beat value
        // bends the repeats' pitch on the way); the other echoes glide in 50 ms.
        let spiral_kind = matches!(kind, Spiral | SpiralUp | SpiralDown);
        // The echoes on rekordbox's `DelayModule` (`EchoCore`, `PitchEchoCore`): whole frames;
        // a new time closes the read gate (3.3333 ms), jumps there and opens it again
        // (`DelayModule::setDelayTime` 0x10298fc28 / `read` 0x10298ff44), no glide.
        let dm_kind = matches!(kind, Delay | Echo | LowCutEcho | PingPong | UpEcho | DownEcho | PitchEcho);
        if dm_kind {
            let wi = ((echo_secs * 1000.0).clamp(1.0, 4000.0) * 0.001 * r).trunc().clamp(1.0, (DELAY_LEN - 2) as f32);
            self.dg.iter_mut().for_each(|g| g.set_rate(r));
            if self.time <= 0.0 {
                self.time = wi;
            }
            let cmp = if self.dpend > 0.0 { self.dpend } else { self.time };
            if (wi - cmp).abs() >= 0.5 {
                self.dpend = wi;
                self.dg.iter_mut().for_each(|g| g.set(0.0));
            }
        }
        if spiral_kind {
            self.spiral.setup(r, match kind { SpiralUp => 1, SpiralDown => 2, _ => 0 });
            self.spiral.beat = (beat_secs * r) as i32;
            self.spiral.set_target(want_time, self.time <= 0.0);
        }
        if self.time <= 0.0 {
            self.time = want_time;
        }
        let glide = 1.0 / (0.05 * r);
        // The echo core's settings.
        let swing = |k: f32| 1.0 + (k - 0.5) * 0.6667;
        let mut e = EchoCfg { ml: 1.0, mr: 1.0, fb: 0.0, rev: false, freeze: false, ratio: 1.0, filt: 0, g: 0.0, taps: 0, limit: false, dm: dm_kind };
        // DELAY / REV DELAY's LR RATIO zone (single mode; the thresholds are rekordbox's ninths).
        let lr_zone = [0.11111f32, 0.22222, 0.33333, 0.44444, 0.55556, 0.66667, 0.77778, 0.88889].iter().filter(|&&t| k2 >= t).count();
        // FILTER knob of the echoes (`SingleBeatFxEcho::updateAndSetParameter`): the right
        // half turns an `InstFxHPF` (2k - 1), the left half an `InstFxLPF` (1 - 2k), both on the
        // repeats, as 8-bit values (truncated).
        let mut echo_filt: Option<(i32, i32)> = None;
        let echo_filter = |f: &mut Option<(i32, i32)>, k: f32| {
            *f = Some(if k >= 0.5 { (((k + k - 1.0) * 255.0) as i32, 0) } else { (0, ((1.0 - (k + k)) * 255.0) as i32) });
        };
        // (dry, wet) of the effects that mix a wet sound with the dry one.
        let mut mix = echo_mix(level);
        let mut spiral_out = 1.0;
        // (fx7a: the repeats' pitch percent for `PitchEchoCore`.)
        let mut pe_pct = 0.0f32;
        match kind {
            Delay | RevDelay => {
                // Multi mode: one repeat, both sides at the beat value. Single mode: LR RATIO
                // in rekordbox's nine steps (1:3, 1:2, 1:1.5, 1:1.25, 1:1, ... 3:1).
                // (`SingleBeatFxDelayBase::updateAndSetParameter` 0x1029ccc24: each side is the
                // beat time times its step, at most 4 s, `BeatFxDelay::MAX_DELAY_TIME`.)
                if !multi {
                    const STEP: [(f32, f32); 9] = [(1.0, 3.0), (1.0, 2.0), (1.0, 1.5), (1.0, 1.25), (1.0, 1.0), (1.25, 1.0), (1.5, 1.0), (2.0, 1.0), (3.0, 1.0)];
                    let (a, b) = STEP[lr_zone];
                    let t = (echo_secs * 1000.0).max(1e-3);
                    e.ml = (a * t).min(4000.0) / t;
                    e.mr = (b * t).min(4000.0) / t;
                }
                e.rev = kind == RevDelay;
            }
            Echo => {
                e.fb = if multi { 0.7 } else { echo_fb(k1, !p.on) };
                if !multi {
                    echo_filter(&mut echo_filt, k2);
                    e.rev = b1;
                    e.freeze = b2;
                }
            }
            UpEcho | DownEcho => {
                // Feedback 0.85; the repeats start to climb (or fall) from the centre up.
                let (wet, pct) = up_echo(level);
                mix = (1.0, wet);
                e.fb = 0.85;
                e.ratio = pitch_ratio(if kind == UpEcho { pct } else { -pct });
                pe_pct = if kind == UpEcho { pct } else { -pct };
            }
            PitchEcho => {
                // `SingleBeatFxPitchEcho`: FEEDBACK 0 .. 0.85 .. 0.993; PITCH -50 % .. +50 % a
                // repeat; FREEZE holds the loop at feedback 1. Mix: wet 1.4 x L up to 0.7,
                // dry from 3/4 down to 0 (FREEZE: the ROLL curve, wet x 0.7).
                e.fb = pitch_echo_fb(k1, !p.on);
                let pct = through_value(k2) * if k2 < 0.5 { -50.0 } else { 50.0 };
                e.ratio = pitch_ratio(pct);
                pe_pct = pct;
                e.rev = b1;
                e.freeze = b2;
                // (`SingleBeatFxPitchEcho::calculateMixRatio` 0x1029c7bb4: FREEZE's curve and
                // the law only once the line holds the delay, or turning off; before that the dry
                // sound alone.)
                let full = p.on && self.filled && !starting;
                mix = if b2 && full { let (d, w) = roll_mix(level); (d, w * 0.7) } else if full || !p.on { (if level <= 0.75 { 1.0 } else { 4.0 - 4.0 * level }, if level < 0.5 { 1.4 * level } else { 0.7 }) } else { (1.0, 0.0) };
            }
            LowCutEcho => {
                // An echo (feedback 0.7) of the sound without its lows (under 284 Hz).
                e.fb = if multi { 0.7 } else { echo_fb(k1, !p.on) };
                if !multi {
                    // SWING (`SingleLowCutFxEcho::updateParameter` 0x1029c1dc0, a
                    // `CenterThroughParameter`): turned left the left side, turned right the
                    // right side runs at 1 - 0.25 x the distance from the centre of the time.
                    let s = 1.0 - 0.25 * through_value(k2);
                    if k2 < 0.48627 {
                        e.ml = s;
                    } else if k2 > 0.51373 {
                        e.mr = s;
                    }
                    e.rev = b1;
                    e.freeze = b2;
                }
            }
            PingPong => {
                // The right side at three quarters of the left's time (at least 10 ms).
                e.fb = if multi { 0.7 } else { echo_fb(k1, !p.on) };
                e.mr = 0.75;
                if !multi {
                    echo_filter(&mut echo_filt, k2);
                    e.rev = b1;
                    e.freeze = b2;
                }
            }
            Spiral => {
                let (wet, fb, out) = spiral_law(level, p.on);
                mix = (1.0, wet);
                spiral_out = out;
                // Single mode: FEEDBACK 0 .. 0.8 .. 1.2 and PITCH (the knob's ends an octave).
                e.fb = if multi { fb } else if k1 < 0.5 { 1.6 * k1 } else { 0.8 + 0.8 * (k1 - 0.5) };
                if !p.on {
                    e.fb = e.fb.min(0.993);
                }
                e.limit = true;
                if !multi {
                    // (An integer percent, truncated: `SingleBeatFxSpiral::updateParameter`.)
                    e.ratio = pitch_ratio(crate::fx_spiral::pitch_percent(k2 * 200.0 - 100.0) as f32);
                }
            }
            SpiralUp | SpiralDown => {
                // `RmxSpiral`: feedback and dry from the main knob; PITCH (single mode) bends
                // each pass up to +-200 % (multi mode: none).
                e.fb = rmx.3;
                mix = (rmx.2, 1.0);
                let pct = if multi { 0.0 } else { k2 * if kind == SpiralUp { 200.0 } else { -200.0 } };
                e.ratio = 1.0 + crate::fx_core::percent_shift(crate::fx_spiral::pitch_percent(pct) as f32);
            }
            MtDelay => {
                // Up to seven repeats an eighth of the beat value apart.
                mix = mt_mix(level);
                e.taps = if multi { 7 } else { (1.0 + k2 * 12.0).round().clamp(1.0, 7.0) as u8 };
                if !multi {
                    e.ml = swing(k1);
                    e.mr = swing(k1);
                }
            }
            Reverb => {}
            // (Single mode reads LEVEL/DEPTH through the centre detent.)
            Trans | Pan => mix = half_mix(if multi { level } else { recommend_value(level) }),
            Filter => mix = filter_mix(level),
            Flanger | Phaser => {
                mix = quarter_mix(if multi { level } else { recommend_value(level) });
                if !multi {
                    let g = if kind == Phaser { phaser_gain(k1, b1) } else { flanger_gain(k1, b1) };
                    mix = (mix.0 * g, mix.1 * g);
                }
            }
            // (`MultiBeatFxRollBase::calculateMixRatio`: the curve once the roll has its sound;
            // before the quantizer starts it the effect is not in.)
            Roll | RevRoll | SlipRoll => mix = if self.roll.valid || p.on { roll_mix(level) } else { (1.0, 0.0) },
            // (`SingleFxRobot::calculateMixRatio` 0x102a235c0: LEVEL/DEPTH brings the robot in
            // over the lower half; multi mode `RobotCore`'s 0 / 1.)
            Robot if !multi => mix = half_mix(level),
            Robot | Helix => mix = (0.0, 1.0),
            Pitch => mix = if level <= 0.0 || p.value == 0.0 && (multi || (k1 - 0.5).abs() <= 0.02) { (1.0, 0.0) } else { (0.0, 1.0) },
            Noise => {
                // The noise's echo: feedback 0.5, FREEZE 1 with its input shut.
                e.fb = if b2 && !multi { 1.0 } else { 0.5 };
                e.freeze = b2 && !multi;
                mix = (1.0, 1.0);
            }
            ReverbUp | ReverbDown => mix = (1.0, 1.0),
            EnigmaJet | MobiusSaw | MobiusTri => mix = (0.0, 1.0),
            // VINYL BRAKE's echo: feedback 0.7 (`updateEcho`), 14 - 14 x speed near the end of
            // the release state.
            VinylBrake => e.fb = if self.vb_state == 2 && self.speed > 0.95 { 14.0 - 14.0 * self.speed } else { 0.7 },
            _ => {}
        }
        // The `BeatFxEcho` / `BeatFxPitchEcho` / `BeatFxDelay` family. FREEZE takes hold once the
        // line holds the delay (`SingleBeatFxEcho::updateAndSetParameter` 0x1029c0248,
        // `SingleBeatFxPitchEcho` 0x1029c7808: feedback 1 and the input gate shut). The reverse
        // core (`crate::fx_rev`) runs beside the forward one and REV fades between their outputs
        // (`BeatFxEcho::processEffectConcrete` 0x1029bf630); REV DELAY is the reverse core alone.
        let gated = matches!(kind, Echo | LowCutEcho | PingPong | PitchEcho) && !multi;
        let rev_mode = match kind {
            RevDelay => Some(crate::fx_rev::RevMode::Plain),
            Echo | LowCutEcho | PingPong if !multi => Some(crate::fx_rev::RevMode::Echo),
            PitchEcho if !multi => Some(crate::fx_rev::RevMode::Pitch),
            _ => None,
        };
        let rev_on = kind == RevDelay || rev_mode.is_some() && e.rev;
        if starting {
            // (`BeatFxEcho::onCalledTurnOn`: the line is counted again.)
            self.filled = false;
            self.fill_n = 0;
        }
        let freeze_on = gated && e.freeze && p.on && self.filled;
        if gated {
            if freeze_on {
                e.fb = 1.0;
            }
            e.freeze = false;
        }
        // Multi-mode ECHO keeps the whole dry sound until the line holds the delay
        // (`MultiBeatFxEcho::updateParameter` 0x1029bf934); so do single ECHO, PING PONG and
        // both LOW CUT ECHOs (`SingleBeatFxEcho::calcMixRatioInternal` 0x1029c05a8,
        // `SingleBeatFxPingPong::calculateMixRatio` 0x1029c44d8, `SingleLowCutFxEcho::
        // calculateMixRatio` 0x1029c2064, `MultiLowCutEcho::calculateMixRatio` 0x1029c1764).
        // FREEZE once it holds: the dry sound on the ROLL curve (`MultiBeatFxRollBase::
        // calcMixRatio`); LOW CUT ECHO takes the curve's wet too.
        if matches!(kind, Echo | PingPong | LowCutEcho) {
            if freeze_on {
                let (d, w) = roll_mix(level);
                mix.0 = d;
                if kind == LowCutEcho {
                    mix.1 = w;
                }
            } else if !(p.on && self.filled) {
                mix.0 = 1.0;
            }
        }
        let t_ms = (echo_secs * 1000.0).clamp(1.0, 4000.0);
        let mut revc = if rev_mode.is_some() { self.rv.take() } else { None };
        if let (Some(f), Some(m)) = (revc.as_deref_mut(), rev_mode) {
            if !self.fresh || f.mode != m || (f.rate() - r).abs() > 0.5 {
                f.setup(m, r);
            }
        }
        if gated {
            self.bg.iter_mut().for_each(|g| g.set_rate(r));
            let t = [if freeze_on { 0.0 } else { 1.0 }, if rev_on { 0.0 } else { 1.0 }, if rev_on { 1.0 } else { 0.0 }];
            for (g, t) in self.bg.iter_mut().zip(t) {
                if self.fresh {
                    g.set_target(t);
                } else {
                    // (`onCalledMakeSelected`: the gates start where they are asked to be.)
                    g.reset(t);
                }
            }
        }
        let made = !self.fresh;
        self.fresh = true;
        // fx8a: single DELAY / REV DELAY's pan: made with the effect, its knob read every buffer,
        // restarted when the delay is switched on.
        if use_dpan {
            if made || (self.dpan.rate() - r).abs() > 0.5 {
                self.dpan.setup(r);
                self.dpan.set(k1, bpm);
                self.dpan.make_selected();
            } else {
                self.dpan.set(k1, bpm);
            }
            if starting {
                self.dpan.turn_on();
            }
        }
        let mut dpg = ([0.0f32; 64], [0.0f32; 64]);
        // fx8a: the FX+ echoes on their rekordbox body (`crate::fx_echo_plus`): one call a
        // buffer; multi mode is made without the LFO and with the half-beat echo alone.
        let use_fxp = matches!(kind, BpfEcho | HpfEcho | LpfEcho | CrushEcho) && self.fxp.as_ref().is_some_and(|f| f.single() == !multi);
        if use_fxp {
            let pk = match kind { BpfEcho => crate::fx_echo_plus::PlusKind::Bpf, HpfEcho => crate::fx_echo_plus::PlusKind::Hpf, LpfEcho => crate::fx_echo_plus::PlusKind::Lpf, _ => crate::fx_echo_plus::PlusKind::Crush };
            let par = crate::fx_echo_plus::PlusPar { on: p.on, main: level, echo: if multi { 0.5 } else { k1 }, modu: if multi { 0.0 } else { k2 }, rev: b1 && !multi, freeze: b2 && !multi, beat_ms: beat_secs * 1000.0 };
            if let Some(c) = self.fxp.as_deref_mut() {
                if made || c.kind() != pk || (c.rate() - r).abs() > 0.5 {
                    c.setup(pk, r);
                    c.make_selected(&par);
                }
                if starting {
                    c.turn_on();
                }
                c.call(frames, &par);
            }
        }
        // fx7a: the forward core (`crate::fx_echo::EchoCore`) and each side's time for it:
        // ECHO / LOW CUT ECHO 1 .. 4000 ms (`SingleBeatFxEcho::updateAndSetParameter`,
        // `SingleLowCutFxEcho::updateParameter`), PING PONG's right side 0.75 x the time held
        // to 10 .. 4000 ms (`SingleBeatFxPingPong::updateParameter` 0x1029c4620), DELAY's sides
        // at their LR RATIO steps.
        let use_ec = matches!(kind, Delay | Echo | LowCutEcho | PingPong) && self.ec.is_some();
        let fwd_ms = |t: f32| -> [f32; 2] {
            if kind == PingPong { [t.clamp(1.0, 4000.0), (0.75 * t).clamp(10.0, 4000.0)] } else { [(t * e.ml).clamp(1.0, 4000.0), (t * e.mr).clamp(1.0, 4000.0)] }
        };
        let ec_fb = [e.fb; 2];
        let mut ec_ms = fwd_ms(t_ms);
        if use_ec && made {
            // (`onCalledMakeSelected`: the core made at the rate, then `EchoCore::reset`.)
            if let Some(c) = self.ec.as_deref_mut() {
                if (c.rate() - r).abs() > 0.5 {
                    c.setup(r);
                }
                c.reset(ec_ms, ec_fb);
            }
        }
        let mut ei = 0usize;
        // MT DELAY (`MultiTapDelayCore`): base time in whole ms, each side's swing (the knob's
        // stretch 1 + 2 s), the eight tap gains; one call a buffer.
        let use_mt = kind == MtDelay && self.mt.is_some();
        let mt_base = (secs * 1000.0) as i32;
        let mt_sw = { let (a, b) = if multi { (1.0, 1.0) } else { mt_swing(k1) }; [(a - 1.0) * 0.5, (b - 1.0) * 0.5] };
        let mt_g = mt_taps(level, multi, k2);
        if use_mt {
            if let Some(m) = self.mt.as_deref_mut() {
                if made || (m.rate() - r).abs() > 0.5 {
                    m.setup(r);
                    m.reset(mt_base, mt_sw, mt_g, 1.0);
                }
                m.call(frames, mt_base, mt_sw, mt_g, 1.0);
            }
        }
        // (Single MT DELAY passes the cut bands live beside the taps, `SingleMultiTapDelay::
        // processEffectConcrete` 0x102a08c5c: the left side's thru on both sides.)
        let mut b3_thru = 0.0f32;
        let use_pe = matches!(kind, PitchEcho | UpEcho | DownEcho) && self.pe.is_some();
        let pe_ms = |t: f32| t.clamp(1.0, 4000.0);
        let mut pe_t = pe_ms(t_ms);
        if use_pe && made {
            if let Some(c) = self.pe.as_deref_mut() {
                if (c.rate() - r).abs() > 0.5 {
                    c.setup(r);
                }
                c.reset(pe_t, e.fb, pe_pct);
            }
        }
        // The band split in front of the effect: single DELAY / REV DELAY (`BeatFxDelay` made
        // with its `EffectFrequencyModule`; `SingleBeatFxDelayBase::updateAndSetParameter`:
        // LOW CUT takes band 2 out, HI CUT band 0) and LOW CUT ECHO (`setThru(2)`: band 2 out).
        // Only what the effect is given is used: the dry sound stays whole.
        let b3_on = (!multi && matches!(kind, Delay | RevDelay | MtDelay)) || kind == LowCutEcho;
        if b3_on {
            if made || (self.b3.rate() - r).abs() > 0.5 {
                self.b3.setup(r);
            }
            let (lo, hi) = if kind == LowCutEcho { (true, false) } else { (b1, b2) };
            self.b3.set_gains([if hi { 0.0 } else { 1.0 }, 1.0, if lo { 0.0 } else { 1.0 }]);
        }
        // fx7b: the same split in front of single SPIRAL (`SingleBeatFxSpiral`), SPIRAL UP / DOWN
        // (`RmxSpiral`, single), REVERB (`BeatFxReverb`), REVERB UP / DOWN (`FxPlusReverbUp` /
        // `Down`), ROBOT (`SingleFxRobot`) and PITCH (`SingleFxPitch`): always three bands,
        // LOW CUT takes band 2 out, HI CUT band 0, each gain fading over 3.3 ms.
        // VINYL BRAKE's BPF (`BeatFxVinylBrake::updateStateAndParameter` 0x102a552c4): while
        // the brake runs, bands 0 and 2 go to thru (fading over 3.3 ms) and pass live beside it.
        // (fx9: VINYL BRAKE makes its module in both modes, `BeatFxVinylBrake` 0x102a543b0.)
        let b3_mine = !multi && matches!(kind, Spiral | SpiralUp | SpiralDown | Reverb | ReverbUp | ReverbDown | Robot | Pitch) || kind == VinylBrake;
        if b3_mine {
            if made || (self.b3.rate() - r).abs() > 0.5 {
                self.b3.setup(r);
            }
            let (lo, hi) = if kind == VinylBrake { let bp = b2 && !multi && self.vb_state != 0; (bp, bp) } else { (b1, b2) };
            self.b3.set_gains([if hi { 0.0 } else { 1.0 }, 1.0, if lo { 0.0 } else { 1.0 }]);
        }
        let b3_on = b3_on || b3_mine;
        // fx9: TRANS and PAN (`BeatFxTrans` made with its `EffectFrequencyModule`, 0x102a5084c;
        // `processEffectConcrete` 0x102a512c8: the gate runs on the module's fx bands and each
        // side's thru is added after it). `SingleBeatFxTrans::updateParameter` 0x102a51ac0:
        // LOW CUT takes band 2 out, HI CUT band 0. `SingleBeatFxPan::updateParameter`
        // 0x102a4f0a8 sets the bands otherwise: button 1 takes band 1 (the middle) out, button 2
        // bands 0 and 2 (its parameters are rekordbox's "Mid Band Removal" / "High & Low Band
        // Removal").
        // The ROLL family (`SingleBeatFxRollBase::processEffectConcrete` 0x102a2ffac) and HELIX
        // (`SingleBeatFxHelix::pre/postProcessEffectHook` 0x1029f62d0 / 0x1029f62d8) run on the
        // whole sound and split twice: the live sound's cut bands (module 1 thru) plus the
        // effect output's kept bands (ROLL: module 2's thru with the buttons the other way
        // round, `updateAndSetParameter` 0x102a2fca0; HELIX: module 2's fx, `updateParameter`
        // 0x1029f6400). `b3` is module 1, `b3t` module 2.
        let b3_post = !multi && matches!(kind, Roll | RevRoll | SlipRoll | Helix);
        let b3_fx9 = !multi && matches!(kind, Trans | Pan) || b3_post;
        if b3_fx9 {
            if made || (self.b3.rate() - r).abs() > 0.5 || (self.b3t.rate() - r).abs() > 0.5 {
                self.b3.setup(r);
                self.b3t.setup(r);
            }
            let f = |b: bool| if b { 0.0 } else { 1.0 };
            self.b3.set_gains(if kind == Pan { [f(b2), f(b1), f(b2)] } else { [f(b2), 1.0, f(b1)] });
            if kind == Helix {
                self.b3t.set_gains([f(b2), 1.0, f(b1)]);
            } else if b3_post {
                self.b3t.set_gains([1.0 - f(b2), 0.0, 1.0 - f(b1)]);
            }
        }
        // STRETCH (`BeatFxStretch::processEffectConcrete` 0x102a454b4): the split is in front of
        // the stretch, which takes the bands at their fx gains; the thru gains pass them live.
        if kind == Stretch {
            if made || (self.b3.rate() - r).abs() > 0.5 || (self.b3t.rate() - r).abs() > 0.5 {
                self.b3.setup(r);
                self.b3t.setup(r);
            }
            let (fx_lo, th_lo, fx_hi, th_hi) = if !multi && (b2 || (k1 - 0.5).abs() > 0.0137) { stretch_bands(k1, b2) } else { (1.0, 0.0, 1.0, 0.0) };
            self.b3.set_gains([fx_hi, 1.0, fx_lo]);
            self.b3t.set_gains([1.0 - th_hi, 1.0, 1.0 - th_lo]);
        }
        // Each side's time for the reverse core (`BeatFxEcho::updateAndSetParameter` 0x1029bf05c:
        // at least 10 ms), its feedback, which sides run themselves (status not on; single REV
        // DELAY: also the side with the longer time), and the restarts the beat quantizer
        // gives this buffer.
        let rv_times = |ms: f32| { let t = ms.clamp(1.0, 4000.0); if kind == RevDelay { [t * e.ml, t * e.mr] } else { [(t * e.ml).max(10.0), (t * e.mr).max(10.0)] } };
        let rv_ms = rv_times(t_ms);
        let rv_fb = if kind == RevDelay { [0.0; 2] } else { [e.fb; 2] };
        let rs = !p.on;
        let rv_self = if kind == RevDelay && !multi { [rs || lr_zone >= 5, rs || lr_zone <= 3] } else { [rs; 2] };
        let rv_shift = e.ratio - 1.0;
        let mut si = 0usize;
        let mut dsi = 0usize;
        let (dry, wet) = mix;
        let echo_type = matches!(kind, Delay | RevDelay | Echo | PitchEcho | UpEcho | DownEcho | LowCutEcho | PingPong | Spiral | SpiralUp | SpiralDown | MtDelay | Reverb | ReverbUp | ReverbDown);
        // Per-effect settings that do not change inside the block.
        let dir = if down != (b1 && matches!(kind, EnigmaJet | MobiusSaw | MobiusTri)) { -1.0f32 } else { 1.0 };
        let hold_lfo = b2 && !multi && matches!(kind, EnigmaJet | MobiusSaw | MobiusTri);
        // ENIGMA JET / MOBIUS: the signed beat time (ms), turned over by REV.
        let jet_ms = (secs * 1000.0).min(32000.0) * dir;
        // ROBOT / PITCH: the percent value, or (single mode) the PITCH knob once it leaves its
        // centre. Multi-mode PITCH scales the shift with LEVEL/DEPTH (twice it at the top).
        let pct = if !multi && (k1 - 0.5).abs() > 0.02 { (k1 - 0.5) * 200.0 } else { p.value };
        let robot_ratio = pitch_ratio(pct.clamp(-100.0, 100.0));
        let pitch_r = pitch_ratio(if multi { pct * 2.0 * level } else { pct });
        // Single mode (`SingleFxRobot::updateParameter` 0x102a22e60): MODULATION sets the
        // core's PITCH GAIN by the multi law and its FLANGER GAIN straight; FLANGER RESONANCE
        // keeps its default 0.267 (`RobotCore::createParameter`).
        let (rob_a, rob_b, rob_mod) = if multi { robot_law(level) } else { (robot_law(k2).0, k2, 0.267) };
        let speed = stretch_speed(level);
        // (fx9: STRETCH's turn in frames, `BeatFxStretch::updateAndSetParameter` 0x102a4558c:
        // the beat value x LEVEL/DEPTH, at most 16 s; its shifter's shift.)
        let st_total = ((secs * level).min(16.0) * r).max(1.0);
        let st_shift = stretch_shift(speed, !multi && b1);
        let stages = if multi { (6, 6) } else { phaser_stages(k2) };
        let (helix_keep, helix_in) = helix_law(level);
        let env_k = (1.0 - (-1.0 / (0.002 * r)).exp(), 1.0 - (-1.0 / (0.12 * r)).exp());
        // The FX+ reverbs: the main knob and TIME in 8-bit steps, REVERB UP's band-pass and
        // the FILTER knob's filter on the reverb (single mode).
        let fxp_v = (level * 255.9).floor().min(255.0);
        let mut fxr_t = ((if multi { 0.5 } else { k1 }) * 255.9) as i32;
        // fx7b: REVERB UP / DOWN take their knobs through `FxRevKnobs` (stepped, the main knob
        // followed), worked out again after every piece in the frame loop.
        let fxrev = matches!(kind, ReverbUp | ReverbDown);
        let mut rv_v = fxp_v as i32;
        let mut rv_fl = (k2 * 255.9) as i32;
        if fxrev {
            if (self.rvk.rate() - r).abs() > 0.5 {
                self.rvk = crate::fx_verb::FxRevKnobs::new(r);
            }
            self.rvk.begin(level, if multi { 0.5 } else { k1 }, if multi { 0.0 } else { k2 });
            (rv_v, fxr_t, rv_fl) = self.rvk.values();
        }
        let rv_piece = crate::fx_verb::FxRevKnobs::piece(r);
        let dry = if kind == ReverbDown { self.rvk.down_dry() } else { dry };
        let (up_fc, up_q, mut up_g, ..) = reverb_up_law(rv_v, fxr_t);
        let mut up_bp = crate::fx_core::iir_bpf2(up_fc as f64, up_q as f64, r as f64);
        let fxr_fc = |down: bool, fl: i32| if multi { None } else { Some(crate::fx_core::make_pass2_db(fxrev_filter(down, fl) as f64, -3.0, true, !down, r as f64)) };
        let mut fxr_f = fxr_fc(kind == ReverbDown, rv_fl);
        // MT DELAY: the taps' gains, the time of each side.
        let mt_t = mt_taps(level, multi, k2);
        let (mt_l, mt_r) = if multi { (1.0, 1.0) } else { mt_swing(k1) };
        // REVERB: the percent value sets size, decay and input gain; single mode's ROOM SIZE
        // is the percent for the size and DECAY sets the decay time and input gain
        // (`SingleBeatFxReverb::updateParameter` / `getDynamicParameter`).
        {
            use crate::fx_verb::VerbLaw;
            let law = match kind {
                Reverb if multi => Some((p.value as i32, VerbLaw::Multi)),
                Reverb => Some((reverb_room(k1), VerbLaw::Single { decay: crate::fx_verb::single_decay_percent(k2), gin: reverb_decay_in_gain(k2) })),
                ReverbUp => {
                    let (_, _, _, size, decay_s, gin) = reverb_up_law(rv_v, fxr_t);
                    Some((1, VerbLaw::Fixed { size, decay_s, gin }))
                }
                ReverbDown => {
                    let (size, decay_s, gin, _) = reverb_down_law(rv_v, fxr_t);
                    Some((1, VerbLaw::Fixed { size, decay_s, gin }))
                }
                _ => None,
            };
            if let Some((pct, law)) = law {
                self.verb.begin(pct, law, kind != Reverb, starting, r, frames);
            }
        }
        // TRANS / PAN: each side's gain from rekordbox's LFOs, worked out a piece at a time
        // (the quantizer splits a piece where the effect restarts).
        let lfo_kind = matches!(kind, Trans | Pan | Roll | RevRoll | SlipRoll);
        // (ROLL: the roll's sound in the two gain arrays, its place in the turn here.)
        let mut rphase = [0.0f32; LFO_PIECE];
        let mut lgain = [1.0f32; LFO_PIECE];
        let mut rgain = [1.0f32; LFO_PIECE];
        let cycle = if down { -beats } else { beats };
        // FLANGER / PHASER feedback: multi mode from LEVEL/DEPTH, single mode from FEEDBACK.
        let fl_fb = if multi { mod_fb(level) } else { flanger_fb(k1, b1) };
        // FLANGER's sweep per side (`MultiBeatFxFlanger` / `SingleBeatFxFlanger::updateParameter`):
        // from one frame to 2.6667 ms; single mode's LFO AMP widens the left side's top to
        // 2.6667 + 25 x (distance from the centre) ms on the left half, the right side's on the
        // right half.
        let fl_lo = 1000.0 / r;
        let fl_top = if multi { [2.6667f32; 2] } else { let t = through_value(k2) * 25.0 + 2.6667; if k2 < 0.48627 { [t, 2.6667] } else if k2 > 0.51373 { [2.6667, t] } else { [2.6667; 2] } };
        // (`BeatFxFlanger::updateAndSetParameter` 0x1029fe1e4: the heads stay at 0 in both modes,
        // the range is one frame .. the top, PHASE turns the waves over.)
        let fl_rev = !multi && b2;
        if kind == Flanger {
            self.fl.iter_mut().for_each(|f| f.set_rate(r));
            self.fgate.set_rate(r);
            if made {
                // The constructor (0x1029fde5c: triangles, 0 .. 2.6667 ms, 50 ms smoothing, a
                // 10 ms period) and `onCalledMakeSelected` 0x1029fe7b8.
                for l in self.flo.iter_mut() {
                    *l = crate::fx_lfo::Lfo::new(r, crate::fx_lfo::Wave::TRI);
                    l.set_amplitude_range(true, 0.0, 2.6667);
                    l.set_smoothing_time(50.0);
                    l.set_period_time(10.0, true);
                    l.reset();
                }
                self.flanger_params([fl_lo; 2], fl_top, fl_rev);
                if self.fpend {
                    self.fpend = false;
                    let ms = self.fpend_ms;
                    self.flo.iter_mut().for_each(|l| l.set_period_time(ms, false));
                }
                self.flo.iter_mut().for_each(|l| l.reset());
                self.fplay = false;
                self.fclose = false;
                self.fgate.reset(1.0);
            }
        }
        let ph_fb = if multi { mod_fb(level) } else { flanger_fb(k1, b1) };
        // Beat FX FILTER: resonance gain, the LFO's reach and the type.
        let reso = if multi { filter_reso(level) } else { filter_single_reso(k1) };
        // FILTER / PHASER (`crate::fx_sweep`): made (the constructor and `onCalledMakeSelected`)
        // when the effect is new; FILTER's parameters depend on its main LFO's period, so they
        // are worked out again for each stretch.
        let flt_par = |bf: &crate::fx_sweep::BeatFilter| if multi { crate::fx_sweep::FilterPar::multi(reso) } else { crate::fx_sweep::FilterPar::single(reso, k2, bpm, bf.lfo[0].period_ms, b1, b2) };
        let ph_par = crate::fx_sweep::PhaserPar::new(ph_fb, stages, !multi && b2);
        if made && kind == Filter {
            self.bf = crate::fx_sweep::BeatFilter::new(r);
            let fp = flt_par(&self.bf);
            self.bf.make_selected(&fp);
        }
        if made && kind == Phaser {
            self.bp = crate::fx_sweep::BeatPhaser::new(r);
            self.bp.make_selected(&ph_par);
        }
        // The FX+ echoes: the main knob in 8-bit steps, ECHO MIX and MOD.
        let (fxp_echo, fxp_mod) = if multi { (0.5, 0.0) } else { (k1, k2) };
        let fxp_depth = if fxp_mod < 0.015625 { 0.0 } else if fxp_mod >= 0.15625 { 50.0 } else { 1.3943 * fxp_mod * 255.0 - 5.5556 };
        let fxp_lfo_step = 1.0 / ((0.200 - 0.00070588 * fxp_mod * 255.0).max(0.02) * r);
        let fxp_lvl: [f32; 4] = std::array::from_fn(|i| (4.0 * fxp_echo - i as f32).clamp(0.0, 1.0));
        // HELIX's SPEED (single mode) and which effects run the pitch shifter this buffer.
        let vb_echo = kind == VinylBrake && !multi && vinyl_echo_beats(k2).is_some();
        // (TRACK SPEED: how fast the held loop's speed follows, 0.5 in multi mode.)
        let helix_v = if multi { 0.5 } else { k2 };
        let ps_on = match kind {
            UpEcho | DownEcho | PitchEcho => (e.ratio - 1.0).abs() > 1e-4 && !use_pe,
            // (fx7b: single SPIRAL / SPIRAL UP / DOWN run the shifter on every block, also at 0 %.)
            Spiral | SpiralUp | SpiralDown => !multi,
            Robot | Pitch => true,
            // (fx9: `StretchCore` runs the shifter on every block: `updatePitchSpeed` 0x102a4780c.)
            Stretch => true,
            VinylBrake => self.vb_state == 2,
            _ => false,
        };
        // fx7b: SPIRAL's core for this block (`SpiralCore::processEffectConcrete`): made for the
        // effect (mode, shifter), `onCalledTurnOn` (the first after the effect was selected
        // runs `init`: line and shifter clean), the output gain and the limit worked out from
        // the block's input as the core is given it (the band split, SPIRAL UP / DOWN's input
        // gain).
        if spiral_kind {
            let mode = match kind { SpiralUp => 1, SpiralDown => 2, _ => 0 };
            self.spc.setup(r, mode, !multi);
            if starting && self.spc.turn_on() {
                if kind == Spiral {
                    self.delay.iter_mut().for_each(|v| *v = 0.0);
                } else {
                    self.aux.iter_mut().for_each(|v| *v = 0.0);
                }
                self.ps.reset();
            }
            self.spc.set_out_gain(if kind == Spiral { spiral_out } else { rmx.4 });
            if kind != Spiral {
                self.rmx_ig.set(if p.on { rmx.0 } else { 0.0 }, r, 0.66667);
                if (self.rmx_echo.rate() - r).abs() > 0.5 {
                    self.rmx_echo.setup(r);
                    self.delay.iter_mut().for_each(|v| *v = 0.0);
                }
                // (ECHO MIX is the first knob, `SUB PARAM1`.)
                self.rmx_echo.set_mix(multi, k1);
            }
            let (mut sb, mut sg, on, bb) = (self.b3, self.rmx_ig, if p.on { 1.0 } else { 0.0 }, &*buf);
            self.spc.limit_block((0..frames).map(|n| {
                let x = [bb[n * 2], bb[n * 2 + 1]];
                let f = if b3_on { sb.next(x).0 } else { x };
                let k = if kind == Spiral { on } else { sg.next() };
                (f[0] * k, f[1] * k)
            }), frames, beat_secs as f64 * 1000.0);
            if !multi {
                self.ps.set_shift(e.ratio - 1.0);
            }
        }
        // fx7b: FX+ NOISE for this block (`FxPlusNoiseEcho::updateAndSetParameter`, then the
        // echo's call and the filled count).
        // (Without its echo, made by `ensure_rev`, the noise plays alone.)
        if kind == Noise {
            if (self.nzf.rate() - r).abs() > 0.5 {
                self.nzf.setup(r);
            }
            if starting {
                self.nzf.turn_on();
            }
            self.nzf.begin(level, if multi { 0.5 } else { k1 }, if multi { 0.5 } else { k2 }, if multi { None } else { Some(b1) }, b2 && !multi, p.on, beat_secs * 1000.0);
            self.nzf.call(frames);
        }
        // fx7b: HELIX's core for this block (`BeatFxHelix::processEffectConcrete`: turned on with
        // the effect, off once faded out; `HelixCore::updateParameter` first).
        if kind == Helix {
            if (self.hxc.rate() - r).abs() > 0.5 {
                self.hxc.set_rate(r);
            }
            let ms = (echo_secs * 1000.0).clamp(10.0, 4000.0);
            if !p.on && self.gate <= 0.0 {
                self.hxc.turn_off();
            }
            if starting {
                self.hxc.turn_on(ms, helix_keep, helix_in, DELAY_LEN / 2);
            }
            self.hxc.begin(ms, helix_keep, helix_in, helix_v, DELAY_LEN / 2);
        }
        for n in 0..frames {
            if use_dpan && n % 64 == 0 {
                let len = (frames - n).min(64);
                self.dpan.gains(&mut dpg.0[..len], &mut dpg.1[..len]);
            }
            if lfo_kind && n % LFO_PIECE == 0 {
                let len = (frames - n).min(LFO_PIECE);
                let b = beat.map(|b| b + n as f64 * bpm as f64 / (60.0 * r as f64));
                if matches!(kind, Trans | Pan) {
                    self.lfo_piece(kind, multi, level, (k1, k2), b, bpm, rate, cycle, p.on, &mut lgain[..len], &mut rgain[..len]);
                } else {
                    let end = ring.w as i64 - frames as i64 + (n + len) as i64;
                    // Single mode's SWING (`SingleBeatFxRollBase::updateAndSetParameter`
                    // 0x102a2fca0): the knob in nine zones -4 .. 4, times the beat (ms) at the
                    // turn-on, within +-2000 ms.
                    if starting && n == 0 {
                        self.roll_beat_ms = 60000.0 / bpm;
                    }
                    self.roll.swing_ms = if multi { 0.0 } else { (roll_swing_zone(k2) * self.roll_beat_ms).clamp(-2000.0, 2000.0) };
                    self.roll_piece(ring, end, b, bpm, rate, cycle, p.on, &mut lgain[..len], &mut rgain[..len], &mut rphase[..len]);
                }
            }
            // (TRANS / PAN come in when the quantizer starts them, on the beat or half beat.)
            let target = if p.on && (!lfo_kind || self.tq.running()) { 1.0 } else { 0.0 };
            self.gate += (target - self.gate).clamp(-gate_k, gate_k);
            let g = self.gate;
            if g <= 0.0 && !p.on {
                // Off and faded out: the rest of the buffer is the dry sound, untouched.
                break;
            }
            if ps_on && (self.ps.block_left() == 0 || kind == Stretch && n == 0) {
                // rekordbox's shifter works a block at a time (`processPitchShift` over the
                // buffer it is given, 512 frames here at most): its zero-crossing search sees
                // the block's input before the block's frames run. The input ahead is the
                // buffer itself (PITCH, ROBOT) or what the line will give (the echoes, SPIRAL,
                // HELIX: written at least a delay time ago; STRETCH: the history).
                let len = (frames - n).min(crate::fx_core::PitchShifter::BLOCK);
                if kind == Stretch {
                    self.stf.set_rate(r);
                    self.stg.set_rate(r);
                    self.stretch_pre(buf, n, len, st_total, speed);
                }
                let mut ps = std::mem::replace(&mut self.ps, crate::fx_core::PitchShifter::empty());
                {
                    let me = &*self;
                    let t = me.time;
                    let bb = &*buf;
                    // (fx7b: single ROBOT / PITCH are given the band split's effect bands.)
                    let mut cb = me.b3;
                    ps.begin_block((0..len).map(|k| match kind {
                        // (fx9: VINYL BRAKE's shifter is given the module's fx bands too.)
                        Robot | Pitch | VinylBrake if b3_on => { let f = cb.next([bb[(n + k) * 2], bb[(n + k) * 2 + 1]]).0; (f[0], f[1]) }
                        Robot | Pitch | VinylBrake => (bb[(n + k) * 2], bb[(n + k) * 2 + 1]),
                        SpiralUp | SpiralDown => (me.aux_tap(t - k as f32, 0), me.aux_tap(t - k as f32, 1)),
                        Stretch => (me.st_pre[k][0], me.st_pre[k][1]),
                        _ => (me.tap(t - k as f32, 0), me.tap(t - k as f32, 1)),
                    }));
                }
                self.ps = ps;
            }
            let (xl, xr) = (buf[n * 2], buf[n * 2 + 1]);
            // What the effect is given (`dl`, `dr`) and the bands kept from it (`tl`, `tr`).
            let (mut dl, mut dr, mut tl, mut tr) = (xl, xr, 0.0, 0.0);
            // (fx9: the live sound's thru bands, added inside the wet sound.)
            let mut th9 = [0.0f32; 2];
            if b3_fx9 {
                let (f, t) = self.b3.next([xl, xr]);
                if !b3_post {
                    [dl, dr] = f;
                }
                th9 = t;
            } else if b3_on {
                let (f, t) = self.b3.next([xl, xr]);
                [dl, dr] = f;
                b3_thru = t[0];
                // (fx7b: VINYL BRAKE adds each side's thru live.)
                if kind == VinylBrake {
                    (tl, tr) = (t[0], t[1]);
                }
            }
            if spiral_kind {
                self.time = self.spiral.next();
            } else if dm_kind {
                // (A restart later in the buffer with a new time: `DelayModule::setDelayTime`.)
                while dsi < sync.n && sync.at[dsi] as usize <= n {
                    let ms = sync.beat_ms[dsi];
                    if ms > 0.0 && sync.at[dsi] > 0 {
                        let wi = (ms.clamp(1.0, 4000.0) * 0.001 * r).trunc().clamp(1.0, (DELAY_LEN - 2) as f32);
                        let cmp = if self.dpend > 0.0 { self.dpend } else { self.time };
                        if (wi - cmp).abs() >= 0.5 {
                            self.dpend = wi;
                            self.dg.iter_mut().for_each(|g| g.set(0.0));
                        }
                    }
                    dsi += 1;
                }
                self.gv = [self.dg[0].next(), self.dg[1].next()];
                if self.dpend > 0.0 {
                    if self.dg.iter().all(|g| g.settled() && g.cur <= 0.0) {
                        self.time = self.dpend;
                        self.dpend = 0.0;
                    }
                } else {
                    // (Each side's gate opens once its line holds its delay; the first
                    // repeat comes in over the gate's 3.3333 ms.)
                    if self.dfill >= (self.time * e.ml).trunc() {
                        self.dg[0].set(1.0);
                    }
                    if self.dfill >= (self.time * e.mr).max(0.010 * r).trunc() {
                        self.dg[1].set(1.0);
                    }
                }
                self.dfill += 1.0;
            } else if kind != Helix {
                // (HELIX's length is set once a buffer, above.)
                self.time += (want_time - self.time) * glide;
            }
            if !hold_lfo {
                self.lfo += lfo_step;
                if self.lfo >= 1.0 {
                    self.lfo -= 1.0;
                    self.odd = !self.odd;
                }
            }
            let m = level * g;
            let (il, ir) = (dl * target, dr * target);
            // The dry sound (whole for the echoes) and the wet sound, mixed by the effect's
            // law; the on/off gate moves both gains over 3.33 ms.
            let (fdl, fdr) = if echo_type || b3_mine || b3_fx9 { (xl, xr) } else { (dl, dr) };
            let mixed = |wl: f32, wr: f32| (fdl * (1.0 + (dry - 1.0) * g) + wl * wet * g, fdr * (1.0 + (dry - 1.0) * g) + wr * wet * g);
            let (ol, or) = match kind {
                Delay | RevDelay | Echo | PitchEcho | UpEcho | DownEcho | LowCutEcho | PingPong => {
                    let (mut il, mut ir) = (il, ir);
                    let (gf, gr) = if gated {
                        let gi = self.bg[0].take();
                        il *= gi;
                        ir *= gi;
                        (self.bg[1].take(), self.bg[2].take())
                    } else {
                        (1.0, 0.0)
                    };
                    let (mut el, mut er) = if kind == RevDelay {
                        (0.0, 0.0)
                    } else if let (true, Some(c)) = (use_ec, self.ec.as_deref_mut()) {
                        // Each of the quantizer's stretches is one `processEffect` call:
                        // `updateAndSetParameter` (a new time from `setEffectBeatTimeWithPlayHead`),
                        // then the call.
                        while ei < sync.n && sync.at[ei] as usize <= n {
                            if sync.beat_ms[ei] > 0.0 {
                                ec_ms = fwd_ms(sync.beat_ms[ei]);
                            }
                            c.set(ec_ms, ec_fb);
                            c.call(sync.len[ei] as usize);
                            ei += 1;
                        }
                        if sync.n == 0 && n == 0 {
                            c.set(ec_ms, ec_fb);
                            c.call(frames);
                        }
                        c.next(il, ir)
                    } else if let (true, Some(c)) = (use_pe, self.pe.as_deref_mut()) {
                        while ei < sync.n && sync.at[ei] as usize <= n {
                            if sync.beat_ms[ei] > 0.0 {
                                pe_t = pe_ms(sync.beat_ms[ei]);
                            }
                            c.set(pe_t, e.fb, pe_pct);
                            c.call(sync.len[ei] as usize);
                            ei += 1;
                        }
                        if sync.n == 0 && n == 0 {
                            c.set(pe_t, e.fb, pe_pct);
                            c.call(frames);
                        }
                        c.next(il, ir)
                    } else {
                        self.echo(il, ir, &e, r)
                    };
                    if let Some(f) = revc.as_deref_mut() {
                        while si < sync.n && sync.at[si] as usize == n {
                            if sync.both[si] {
                                f.play_head_both();
                            } else if sync.play[si] {
                                f.play_head();
                            }
                            let rv_ms = if sync.beat_ms[si] > 0.0 { rv_times(sync.beat_ms[si]) } else { rv_ms };
                            f.begin_call(sync.len[si] as usize, frames, rv_ms, rv_fb, rv_self, rv_shift);
                            si += 1;
                        }
                        let (a, b) = f.next(il, ir);
                        if kind == RevDelay {
                            (el, er) = (a, b);
                        } else {
                            el = el * gf + a * gr;
                            er = er * gf + b * gr;
                        }
                    } else {
                        // (Without a reverse core made, REV is silent.)
                        el *= gf;
                        er *= gf;
                    }
                    if let Some((ha, la)) = echo_filt {
                        let mut fr = [el, er];
                        // (Both made with their resonance knob at 0.2, 51 as 8 bits:
                        // `SingleBeatFxEcho` 0x1029bfaa4 in its single layer's filter type 1,
                        // `SingleBeatFxPingPong` 0x1029c36e0.)
                        self.ifl[0].process(&mut fr, ha, 51, rate);
                        self.ifl[1].process(&mut fr, la, 51, rate);
                        (el, er) = (fr[0], fr[1]);
                    }
                    if use_dpan {
                        // DELAY's first knob (`SingleBeatFxDelayBase::processEffectConcrete`
                        // 0x1029cd094): each side times its square-wave gain (0 .. 1 + 0.4125 x
                        // the knob's distance from the centre) on the pan share, plus the
                        // unpanned share.
                        (el, er) = self.dpan.next(el, er, dpg.0[n % 64], dpg.1[n % 64]);
                    }
                    mixed(el, er)
                }
                MtDelay if use_mt => {
                    let (a, b) = self.mt.as_deref_mut().map(|m| m.next(il, ir)).unwrap_or((0.0, 0.0));
                    let th = if multi { 0.0 } else { b3_thru * target };
                    mixed(a + th, b + th)
                }
                MtDelay => {
                    // `MultiTapDelayCore`: eight taps at whole milliseconds of k/8 of each
                    // side's time, each at its smoothed gain, summed; no feedback.
                    let ms = self.time / r * 1000.0;
                    let (mut el, mut er) = (0.0, 0.0);
                    for k in 0..8 {
                        self.taps[k] += (mt_t[k] - self.taps[k]).clamp(-gate_k, gate_k);
                        if self.taps[k] > 0.0 {
                            let f = (k + 1) as f32 * 0.125;
                            el += self.tap(((f * ms * mt_l).trunc() * 0.001 * r).min((DELAY_LEN - 2) as f32), 0) * self.taps[k];
                            er += self.tap(((f * ms * mt_r).trunc() * 0.001 * r).min((DELAY_LEN - 2) as f32), 1) * self.taps[k];
                        }
                    }
                    self.push(il, ir);
                    mixed(el, er)
                }
                Spiral => {
                    // fx7b: `SpiralCore` (crate::fx_spiral): input low-pass, the loop (clip at
                    // the limit, high- and low-pass, feedback), the repeats through the output
                    // low-pass and gain, then (single mode) the shifter and the band-pass.
                    let x = self.spc.pre([il, ir]);
                    let d = [self.tap(self.time, 0), self.tap(self.time, 1)];
                    let w = self.spc.feed(d, x, e.fb);
                    self.push(w[0], w[1]);
                    let mut o = self.spc.out(d);
                    if ps_on {
                        let (a, b) = self.ps.process(o[0], o[1]);
                        o = [a, b];
                    }
                    if self.spc.uses_bpf() {
                        o = self.spc.bpf(o);
                    }
                    mixed(o[0], o[1])
                }
                SpiralUp | SpiralDown => {
                    // `RmxSpiral`: the input at its gain (brought in over the first quarter of
                    // the knob, 0.66667 ms) through `SpiralCore` in its own line; then the FX+
                    // pack's four echoes of the core's output at ECHO MIX, added to it.
                    let ig = self.rmx_ig.next();
                    let x = self.spc.pre([dl * ig, dr * ig]);
                    let d = [self.aux_tap(self.time, 0), self.aux_tap(self.time, 1)];
                    let w = self.spc.feed(d, x, e.fb);
                    self.aux_push(w[0], w[1]);
                    let mut o = self.spc.out(d);
                    if ps_on {
                        let (a, b) = self.ps.process(o[0], o[1]);
                        o = [a, b];
                    }
                    o = self.spc.bpf(o);
                    let eo = self.rmx_echo.next(&mut self.delay, o, n);
                    mixed(o[0] + eo[0], o[1] + eo[1])
                }
                Reverb => {
                    let (rl, rr) = self.verb.frame(il, ir);
                    mixed(rl, rr)
                }
                ReverbUp | ReverbDown => {
                    // `FxPlusReverbUp` / `Down`: (UP) a band-pass rising and narrowing with
                    // the main knob at its gain, then the reverb, then (single mode) the
                    // FILTER knob's high-pass (UP) or low-pass (DOWN); added to the dry sound.
                    let (mut ul, mut ur) = (il, ir);
                    if kind == ReverbUp {
                        ul = crate::fx_core::bq(&up_bp, &mut self.f1[0], il) * up_g;
                        ur = crate::fx_core::bq(&up_bp, &mut self.f1[1], ir) * up_g;
                    }
                    let (mut rl, mut rr) = self.verb.frame(ul, ur);
                    if let Some(c) = &fxr_f {
                        rl = crate::fx_core::bq(c, &mut self.f3[0], rl);
                        rr = crate::fx_core::bq(c, &mut self.f3[1], rr);
                    }
                    // fx7b: after each piece the knobs step and the laws are worked out again
                    // (`processEffectConcrete`: `SmoothShiftInteger::processSamples`, then
                    // `updateReverbUpParameter` / `Down`).
                    if (n + 1) % rv_piece == 0 || n + 1 == frames {
                        self.rvk.piece_end(if (n + 1) % rv_piece == 0 { rv_piece } else { (n + 1) % rv_piece });
                        let (v, t, fl) = self.rvk.values();
                        let law = if kind == ReverbUp {
                            let (fc, q, g, size, decay_s, gin) = reverb_up_law(v, t);
                            up_bp = crate::fx_core::iir_bpf2(fc as f64, q as f64, r as f64);
                            up_g = g;
                            crate::fx_verb::VerbLaw::Fixed { size, decay_s, gin }
                        } else {
                            let (size, decay_s, gin, _) = reverb_down_law(v, t);
                            crate::fx_verb::VerbLaw::Fixed { size, decay_s, gin }
                        };
                        self.verb.set_law(law);
                        fxr_f = fxr_fc(kind == ReverbDown, fl);
                    }
                    mixed(rl, rr)
                }
                Trans | Pan => {
                    // `TransCore::processEffect` 0x102a530d0: each side times its LFO. TRANS: a
                    // square between 0 and 1 (open for the duty, edges a 3.3333 ms line ending on
                    // the jump). PAN: a triangle between 0 and 1.4125, the sides half a turn apart.
                    let i = n % LFO_PIECE;
                    mixed(dl * lgain[i] + th9[0], dr * rgain[i] + th9[1])
                }
                Filter => {
                    // `BeatFxFilter::processEffectConcrete` 0x1029cf584 (`crate::fx_sweep`): the
                    // quantizer's calls before this stretch, the parameters, a restart shutting
                    // the output gate; each side's place in the sweep is its main LFO (a triangle
                    // over the beat value, 0 .. 1) plus its second one (single mode's LFO AMP),
                    // through `LFOFilterCore`'s 129-step tables: three first-order sections and a
                    // resonance path of three more. TONE (high-pass) and PHASE (the LFOs turned
                    // over) wait for the gate to shut, like the jump to the head.
                    while si < sync.n && sync.at[si] as usize == n {
                        self.bf.calls(sync.until_ms[si], sync.beat_ms[si], sync.play[si] || sync.on[si]);
                        let fp = flt_par(&self.bf);
                        self.bf.update(&fp);
                        self.bf.begin();
                        si += 1;
                    }
                    if self.lfb_made != (rate, self.bf.hp) {
                        self.lfb_made = (rate, self.bf.hp);
                        self.lfb = lfo_filter_tables(self.bf.hp, rate);
                    }
                    let o = self.bf.frame([dl, dr], &self.lfb, rate);
                    mixed(o[0], o[1])
                }
                Flanger => {
                    // `FlangerCore::processEffectCore` 0x102a02a44, a frame at a time per side:
                    // the LFO (a `TriangleWave` between one frame and the side's top, 2.6667 ms;
                    // turned over by PHASE) gives the delay in ms, the line is read there
                    // (between frames), its input is the sound plus feedback x the read, and the
                    // output is the read at a gain that falls below 0.5 ms (0.3686 + 1.2628 x
                    // ms), the feedback and gain `SmoothFloat`s of 3.3333 ms.
                    // The restarts (`BeatFxFlanger::processEffectConcrete` 0x1029fe634): the beat
                    // quantizer's calls before this stretch, the parameters, then a restart shuts
                    // the output gate; once it is shut the new period is taken, both LFOs jump to
                    // their heads and the gate opens again.
                    while si < sync.n && sync.at[si] as usize == n {
                        self.flanger_calls(&sync, si);
                        self.flanger_params([fl_lo; 2], fl_top, fl_rev);
                        if self.fclose {
                            self.fplay = false;
                        } else if self.fplay {
                            self.fplay = false;
                            self.fclose = true;
                            self.fgate.set_target(0.0);
                        }
                        si += 1;
                    }
                    let (fbk, ..) = (self.fl[0].set_next(fl_fb), ());
                    let gg = self.fgate.take();
                    let mut o = [0.0f32; 2];
                    for (c, x) in [dl, dr].into_iter().enumerate() {
                        let mut v = [0.0f32; 1];
                        self.flo[c].get_values(&mut v);
                        let ms = v[0];
                        let d = (ms * 0.001 * r).max(1.0);
                        let y = self.tap(d, c);
                        o[c] = y;
                        let k = if ms <= 0.0 { 0.0 } else if ms >= 0.5 { 1.0 } else { ms * 1.2628 + 0.3686 };
                        o[c] = y * self.fl[1 + c].set_next(k) * gg;
                        self.fb[c] = x + y * fbk;
                    }
                    self.push(self.fb[0], self.fb[1]);
                    if self.fclose && !self.fgate.on && self.fgate.cur.abs() < 1.1920929e-7 {
                        self.fclose = false;
                        self.fgate.set_target(1.0);
                        if self.fpend {
                            self.fpend = false;
                            let ms = self.fpend_ms;
                            self.flo.iter_mut().for_each(|l| l.set_period_time(ms, false));
                        }
                        self.flo.iter_mut().for_each(|l| l.jump_to_head(false));
                    }
                    mixed(o[0], o[1])
                }
                Phaser => {
                    // `BeatFxPhaser::processEffectConcrete` 0x102a0c9d8 (`crate::fx_sweep`): as
                    // FILTER, with one triangle a side (the heads half way: a restart starts at
                    // the sweep's top) and a new stage count also waiting for the gate.
                    // `PhaserCore::processEffect`: 299.96 Hz .. 8058.92 Hz pre-warped, moved with
                    // the cube of the LFO; first-order all-pass stages; the output fed back
                    // through a one-pole, eased off at the bottom of the sweep.
                    while si < sync.n && sync.at[si] as usize == n {
                        self.bp.calls(sync.until_ms[si], sync.beat_ms[si], sync.play[si] || sync.on[si]);
                        self.bp.update(&ph_par);
                        self.bp.begin(sync.len[si] as usize);
                        si += 1;
                    }
                    let o = self.bp.frame([dl, dr]);
                    mixed(o[0], o[1])
                }
                Roll | RevRoll | SlipRoll => {
                    // The roll's sound (`RollCore` / `ReverseRollCore` / `SlipRollCore` read
                    // through their gate). Single mode's first knob
                    // (`SingleBeatFxRollBase::updateAndSetParameter`): two square gates over each
                    // turn (3.33 ms edges), one on the roll and one on the live sound; see
                    // `roll_gates`.
                    let i = n % LFO_PIECE;
                    let (rd, rh, dd, dh) = if multi { (1.0, 0.0, 0.0, 0.0) } else { roll_gates(k1) };
                    let ph = rphase[i];
                    let sq = |d: f32, h: f32| if d >= 1.0 || (ph + h).fract() < d { 1.0f32 } else { 0.0 };
                    self.fb[0] += (sq(rd, rh) - self.fb[0]).clamp(-gate_k, gate_k);
                    self.fb[1] += (sq(dd, dh) - self.fb[1]).clamp(-gate_k, gate_k);
                    // (fx9: single mode, the roll's kept bands beside the live cut bands.)
                    let mut rl = [lgain[i], rgain[i]];
                    if b3_post {
                        let k = self.b3t.next(rl).1;
                        rl = [k[0] + th9[0], k[1] + th9[1]];
                    }
                    let ol = rl[0] * self.fb[0] + dl * self.fb[1];
                    let or = rl[1] * self.fb[0] + dr * self.fb[1];
                    mixed(ol, or)
                }
                Robot => {
                    // `RobotCore`: the shifted sound feeds a short line whose delay a
                    // triangle sweeps between one frame and 2.67 ms (0.1778 frames a frame);
                    // the line's input is the shifted sound plus MOD x its last output, and
                    // its output is quieter below 0.5 ms (0.3686 + 0.02863 x delay at
                    // 44.1 kHz). Out: plain x A + robot voice x B.
                    let (pl, pr) = self.shift(dl, dr, robot_ratio);
                    let rk = r / 44100.0;
                    let (hi, knee) = (117.6 * rk, 22.05 * rk);
                    self.rob[2] += pl;
                    self.rob[3] += pr;
                    self.rob[0] += self.rob[1];
                    if self.rob[1] > 0.0 && self.rob[0] > hi {
                        self.rob[0] = 2.0 * hi - self.rob[0];
                        self.rob[1] = -self.rob[1];
                    } else if self.rob[1] <= 0.0 && self.rob[0] < 1.0 {
                        self.rob[0] = 2.0 - self.rob[0];
                        self.rob[1] = -self.rob[1];
                    }
                    let d = self.rob[0];
                    let (vl, vr) = (self.tap(d, 0), self.tap(d, 1));
                    self.push(self.rob[2], self.rob[3]);
                    self.rob[2] = rob_mod * vl;
                    self.rob[3] = rob_mod * vr;
                    let gk = if d <= knee { 0.0286349 / rk * d + 0.3686 } else { 1.0 };
                    self.ga[0] += (rob_a - self.ga[0]).clamp(-gate_k, gate_k);
                    self.ga[1] += (rob_b - self.ga[1]).clamp(-gate_k, gate_k);
                    // (fx7b: single mode adds the left side's thru bands to both sides,
                    // `SingleFxRobot::processEffectConcrete` 0x102a22d74.)
                    mixed(pl * self.ga[0] + vl * gk * self.ga[1] + b3_thru, pr * self.ga[0] + vr * gk * self.ga[1] + b3_thru)
                }
                Pitch => {
                    // (fx7b: as ROBOT, `SingleFxPitch::processEffectConcrete` 0x102a233a0.)
                    let (pl, pr) = self.shift(dl, dr, pitch_r);
                    mixed(pl + b3_thru, pr + b3_thru)
                }
                EnigmaJet => {
                    // rekordbox's ENIGMA JET (`fx_core::EnigmaJet`); it mixes its own dry
                    // sound.
                    let (ol, or) = self.ej.process(dl, dr, level, if multi { 0.5 } else { k1 }, if multi { 0.5 } else { k2 }, jet_ms, hold_lfo, p.on, r);
                    mixed(ol, or)
                }
                MobiusSaw | MobiusTri => {
                    // rekordbox's Shepard tone (`fx_core::Mobius`), added to the sound.
                    let (tl2, tr2) = self.mob.process(kind == MobiusSaw, level, if multi { 0.5 } else { k1 }, if multi { 0.5 } else { k2 }, jet_ms, hold_lfo, p.on, r);
                    mixed(dl + tl2, dr + tr2)
                }
                Helix => {
                    // fx7b: `HelixCore` (crate::fx_helix): records the beat value, then lays
                    // the input over the loop (IN, KEEP); a new TIME plays the loop faster or
                    // slower in steps and records the new length from it.
                    let mut y = self.hxc.frame(&mut self.delay, DELAY_LEN / 2, [dl, dr], n + 1 == frames);
                    // (fx9: single mode, the helix's kept bands beside the live cut bands.)
                    if b3_post {
                        let k = self.b3t.next(y).0;
                        y = [k[0] + th9[0], k[1] + th9[1]];
                    }
                    mixed(y[0], y[1])
                }
                VinylBrake => {
                    // `BeatFxVinylBrake::updateStateAndParameter` / `updatePitchSpeed`: turning
                    // LEVEL/DEPTH past 1/128 starts the brake (rekordbox's curve over
                    // `vinyl_brake_beats` x the beat value); turning it back by more than 1/32
                    // from its highest point hands the speed to the knob (the release state):
                    // the live sound plays pitched by log2 of a speed that runs from where the
                    // brake was back to 1 as the knob returns, and sinks by 0.1 a beat while the
                    // knob rests. REPEAT ON brakes again every REPEAT beats; ECHO adds an echo.
                    let lv = level;
                    if lv < 0.0097656 || !p.on {
                        self.vb_state = 0;
                    } else if self.vb_state == 0 {
                        self.vb_state = 1;
                        self.speed = 1.0;
                        self.lag = 0.0;
                        self.t = 0.0;
                        self.vb = [lv, 0.0, 0.0, 0.0, lv, 0.0];
                        // (`VinylBrakeCore::startVinylBrake`: its sound from here.)
                        self.vbc.set_decay((vinyl_brake_beats(lv) * beat_secs).clamp(0.010, 12.0) * 1000.0);
                        self.vbc.start(r);
                        self.vb_start = (ring.w + RING_LEN - frames + n) % RING_LEN;
                        self.vb_k = (0, 0);
                        self.vbh_n = 0;
                    }
                    let mut out = (dl, dr);
                    let mut wet_g = 0.0;
                    if self.vb_state == 1 {
                        self.vb[0] = self.vb[0].max(lv);
                        // `VinylBrakeCore::processBlock`: the speeds a call at a time (this
                        // buffer's rest, at most 512 frames), the sound read at them, weighted
                        // and through `SlowPlaybackFilter`.
                        if self.vb_k.0 >= self.vb_k.1 {
                            let m = (frames - n).min(512);
                            self.vbc.set_decay((vinyl_brake_beats(lv) * beat_secs).clamp(0.010, 12.0) * 1000.0);
                            self.vbc.speeds(&mut self.vb_sp[..m]);
                            self.vb_k = (0, m);
                        }
                        let v = self.vb_sp[self.vb_k.0];
                        self.vb_k.0 += 1;
                        self.speed = v;
                        let h0 = self.vb_start;
                        // fx9: the core's own history: this frame's fx bands are written
                        // before it reads (`processBlock` writes the block first), and it reads
                        // at most one frame past the one before.
                        let (wl, wr) = if let Some(h) = self.vbh.as_deref_mut() {
                            let k = self.vbh_n;
                            if k < VB_HIST {
                                h[k * 2] = dl;
                                h[k * 2 + 1] = dr;
                                self.vbh_n = k + 1;
                            }
                            let h = &*h;
                            self.vbc.frame(v, |k| {
                                let i = k.min(VB_HIST - 1);
                                (h[i * 2], h[i * 2 + 1])
                            })
                        } else {
                            self.vbc.frame(v, |k| {
                                let i = (h0 + k) % RING_LEN;
                                (ring.buf[i * 2], ring.buf[i * 2 + 1])
                            })
                        };
                        out = (wl, wr);
                        wet_g = 1.0;
                        self.t += 1.0;
                        if b1 && !multi && self.t >= vinyl_repeat_beats(k1) * beat_secs * r {
                            self.t = 0.0;
                            self.lag = 0.0;
                            self.speed = 1.0;
                            self.vbc.start(r);
                            self.vb_start = (ring.w + RING_LEN - frames + n + 1) % RING_LEN;
                            self.vb_k = (0, 0);
                            self.vbh_n = 0;
                        }
                        if lv < self.vb[0] - 0.03125 && !(b1 && !multi) {
                            // Into the release state from here.
                            self.vb_state = 2;
                            self.vb = [self.vb[0], lv, self.speed, 0.0, lv, 0.0];
                        }
                    }
                    if self.vb_state == 2 {
                        let (l0, v0) = (self.vb[1], self.vb[2]);
                        let s1 = (v0 - 1.0) / (l0 - 0.0097656);
                        let s2 = v0 / (l0 - 1.0);
                        let line = if lv <= l0 || s2.abs() <= s1.abs() { s1 * (lv - 0.0097656) + 1.0 } else { v0 + s2 * (lv - l0) };
                        let near = if lv < 0.125 { lv * 8.678 - 0.084746 } else { 1.0 };
                        let sp = (line - self.vb[3] * near).clamp(0.0, 1.0);
                        if sp <= 0.0 || (self.vb[4] - lv).abs() > 1e-6 {
                            if sp <= 0.0 {
                                self.vb[1] = lv;
                                self.vb[2] = 0.0;
                                self.vb[3] = 0.0;
                            }
                        } else {
                            self.vb[3] += 0.1 / (beat_secs * r);
                        }
                        self.vb[4] = lv;
                        self.speed = sp;
                        let pc = (if sp < 0.45 { sp * 0.44444 + 0.25 } else { sp }).log2() * 100.0;
                        let (pl, pr) = self.shift(dl, dr, 1.0 + crate::fx_core::percent_shift(pc.max(-200.0)));
                        // (`updateMixRatio`: the release state's gain for the speed.)
                        let g = if sp < 0.01 { 0.0 } else if sp < 0.05 { 12.5 * sp - 0.125 } else if sp < 0.125 { 0.5 } else if sp < 1.0 { 0.42857 + 0.57143 * sp } else { 1.0 };
                        self.vb[5] += (g - self.vb[5]) * (1.0 / (0.2 * r)).min(1.0);
                        // (`updateStateAndParameter`: under half speed an `IirLpf1stOrder` at
                        // 2500 + 46000 v^2 Hz is cross-faded in over 3.3333 ms; else 14 kHz, out.)
                        let fc = if sp < 0.5 { sp * sp * 46000.0 + 2500.0 } else { 14000.0 };
                        let c = crate::fx_core::iir_1st(fc.min(0.5 * r).max(5.0) as f64, r as f64, false);
                        self.vxf.set_target(if sp < 0.5 { 1.0 } else { 0.0 });
                        let x = self.vxf.take();
                        let mut f = [pl, pr];
                        for (ch, v) in f.iter_mut().enumerate() {
                            let z = &mut self.vlp[ch];
                            let y = c[0] * *v + c[1] * z[0] - c[2] * z[1];
                            z[0] = *v;
                            z[1] = y;
                            *v = *v + (y - *v) * x;
                        }
                        out = (f[0] * self.vb[5], f[1] * self.vb[5]);
                        wet_g = 1.0;
                    }
                    let (wl, wr) = out;
                    let (el, er) = if vb_echo { self.echo(wl * target, wr * target, &e, r) } else { (0.0, 0.0) };
                    self.ga[0] += (wet_g - self.ga[0]).clamp(-gate_k, gate_k);
                    let w = self.ga[0] * g;
                    // (fx9: the dry side is the whole sound, `updateMixRatio` 0x102a5568c: the
                    // EffectBase dry gain 1 only before the brake starts, the wet sound the
                    // module's bands.)
                    (xl * (1.0 - w) + (wl + el) * w, xr * (1.0 - w) + (wr + er) * w)
                }
                Stretch => {
                    // Every beat value x LEVEL/DEPTH playback starts again at the live sound and runs at
                    // rekordbox's speed for LEVEL/DEPTH (1 .. 1/2 .. 1/4), so it falls behind.
                    // REPITCH: the pitch drops with the speed; otherwise it is shifted back up.
                    // The knob cuts lows (left) or highs (right); BPF is a band-pass.
                    // (`BeatFxStretch::updateAndSetParameter`: the turn is the beat value times
                    // LEVEL/DEPTH, at most 16 s; `StretchCore` starts again at the live sound
                    // when it ends.)
                    // (`StretchCore::processEffectInternal` 0x102a47408: a restart takes the live
                    // place, shuts the read gain over 3.3 ms, jumps there and opens it again;
                    // the read sound goes through `SlowPlaybackFilter` at the speed, then the
                    // pitch shifter.)
                    // fx9: the read sound from `stretch_pre` (the block's, made before the
                    // shifter's block began), then the shifter: `updatePitchSpeed` 0x102a4780c,
                    // REPITCH (single mode) shift 0, else 1 / speed - 1 within -2 .. 3.
                    let y = if self.st_i < self.st_len { self.st_pre[self.st_i] } else { [0.0; 2] };
                    self.st_i += 1;
                    let (_, th) = self.b3t.next([xl, xr]);
                    self.ps.set_shift(st_shift);
                    let (wl, wr) = self.ps.process(y[0], y[1]);
                    let edge = 1.0;
                    // (LOW / HI CUT and BPF, `SingleBeatFxStretch::updateParameter` 0x102a459c0:
                    // the gains of the split in front, set above.)
                    let thru = th;
                    (dl * (1.0 - g) + (wl * edge + thru[0]) * g, dr * (1.0 - g) + (wr * edge + thru[1]) * g)
                }
                BpfEcho | HpfEcho | LpfEcho | CrushEcho if use_fxp => {
                    // (EffectBase mix of these: dry 0, wet 1; the on/off gate fades between.)
                    let (wl, wr) = match self.fxp.as_deref_mut() { Some(c) => c.next(dl, dr), None => (dl, dr) };
                    (dl * (1.0 - g) + wl * g, dr * (1.0 - g) + wr * g)
                }
                BpfEcho | HpfEcho | LpfEcho | CrushEcho => {
                    // rekordbox's FX+ echoes: the main knob (read in 8-bit steps) moves a
                    // filter (or the crusher and its high-pass) and the share of the filtered
                    // sound; MOD wobbles the knob with a fast LFO; ECHO MIX brings in up to
                    // four echoes of the result (a quarter, a half, three quarters of a beat
                    // and a beat; feedback 0.75).
                    self.lfo2 = (self.lfo2 + fxp_lfo_step).fract();
                    let v = (fxp_v - fxp_depth * (TAU * self.lfo2).sin().max(0.0)).clamp(0.0, 255.0);
                    let (pd, pw) = fxplus_mix(kind, v);
                    let fc = fxplus_fc(kind, v);
                    let gf = svf_g(fc, r);
                    let (mut sl, mut sr) = (dl, dr);
                    if kind == CrushEcho {
                        let hold = (1.0 + v * 0.0019608 * 0.010 * r).max(1.0);
                        self.hold_n += 1.0;
                        if self.hold_n >= hold {
                            self.hold_n -= hold;
                            self.hold = [dl, dr];
                        }
                        (sl, sr) = (self.hold[0], self.hold[1]);
                    }
                    let q = match kind {
                        LpfEcho => (1.5 * (1.0 - ((v - 3.0).max(0.0) / 252.0).powi(16))).max(0.5),
                        BpfEcho => 0.65,
                        _ => (1.4 * (1.0 - ((255.0 - v) / 252.0).powi(16))).max(0.5),
                    };
                    let a = svf(&mut self.f2[0], sl, gf, 1.0 / q);
                    let b = svf(&mut self.f2[1], sr, gf, 1.0 / q);
                    let (fl, fr) = match kind {
                        LpfEcho => (a.0, b.0),
                        BpfEcho => (a.1 / q, b.1 / q),
                        _ => (a.2, b.2),
                    };
                    let (ml, mr) = (dl * (1.0 + (pd - 1.0) * g) + fl * pw * g, dr * (1.0 + (pd - 1.0) * g) + fr * pw * g);
                    let (mut el, mut er) = (0.0, 0.0);
                    for i in 0..4 {
                        let d = ((i + 1) as f32 * 0.25 * beat_secs * r).clamp(0.010 * r, (PART_LEN - 2) as f32);
                        let (tl2, tr2) = (self.part_tap(i, d, 0), self.part_tap(i, d, 1));
                        let (fl2, fr2) = if b1 && !multi { (tr2, tl2) } else { (tl2, tr2) };
                        let (inl, inr) = if b2 && !multi { (0.0, 0.0) } else { (ml * 0.7 * target, mr * 0.7 * target) };
                        self.part_put(i, inl + fl2 * 0.75, inr + fr2 * 0.75);
                        el += tl2 * fxp_lvl[i];
                        er += tr2 * fxp_lvl[i];
                    }
                    self.w = (self.w + 1) % DELAY_LEN;
                    (ml + el * g, mr + er * g)
                }
                Noise => {
                    // fx7b: `FxPlusNoiseEcho` (crate::fx_noise): stereo noise (CRUSH holds the
                    // noise), its band-pass and gate, and its echo at 0.35 (FREEZE: whole).
                    let y = self.nzf.next();
                    mixed(y[0], y[1])
                }
            };
            buf[n * 2] = ol + tl;
            buf[n * 2 + 1] = or + tr;
        }
        // fx7b: `SpiralCore::checkOutputGain` at the block's end.
        if spiral_kind {
            self.spc.end_block(frames);
        }
        if rev_mode.is_some() {
            self.rv = revc;
        }
        if matches!(kind, Echo | LowCutEcho | PingPong | PitchEcho) {
            // `BeatFxEcho::processEffectConcrete`: the line holds the longest delay (counted up
            // to 4 s).
            // (At the end of each call: each of the quantizer's stretches.)
            let lim = ((r * 0.001) * (t_ms * e.ml).max(t_ms * e.mr)) as i32;
            for i in 0..sync.n.max(1) {
                if self.fill_n >= (r * 4.0) as i32 {
                    self.filled = true;
                } else {
                    self.fill_n += if sync.n == 0 { frames as i32 } else { sync.len[i] as i32 };
                    self.filled = self.fill_n >= lim;
                }
            }
        }
        if !p.on && self.gate <= 0.0 {
            // Off and faded out: the next ON starts clean.
            self.clear();
        }
    }
}

/// The last 15 s of a channel (interleaved stereo), for ROLL and RELEASE FX.
pub(crate) struct Ring {
    pub(crate) buf: Vec<f32>,
    pub(crate) w: usize,
}

impl Ring {
    pub(crate) fn new() -> Ring {
        Ring { buf: vec![0.0; RING_LEN * 2], w: 0 }
    }

    /// The history at a fractional frame index (wrapping).
    #[inline]
    pub(crate) fn at(&self, idx: f64) -> (f32, f32) {
        let f = idx.rem_euclid(RING_LEN as f64);
        let i = f as usize % RING_LEN;
        let j = (i + 1) % RING_LEN;
        let t = (f - f.floor()) as f32;
        (self.buf[i * 2] + (self.buf[j * 2] - self.buf[i * 2]) * t, self.buf[i * 2 + 1] + (self.buf[j * 2 + 1] - self.buf[i * 2 + 1]) * t)
    }

    pub(crate) fn write(&mut self, buf: &[f32]) {
        for f in buf.chunks_exact(2) {
            self.buf[self.w * 2] = f[0];
            self.buf[self.w * 2 + 1] = f[1];
            self.w = (self.w + 1) % RING_LEN;
        }
    }
}

/// RELEASE FX state for one channel.
struct Release {
    // fx7b
    /// BACK SPIN's core and VINYL BRAKE's (`crate::fx_spin`), the latter's speeds for the buffer.
    spin: crate::fx_spin::BackSpin,
    brake: crate::fx_spin::VinylCore,
    kind: u8,
    /// Frames since it was pressed, where in the history playback stands (frames from the
    /// press) and the brake's speed.
    t: f32,
    pos: f64,
    speed: f32,
    start: usize,
    gate: f32,
    held: bool,
}

pub(crate) struct Channel {
    /// The three multi-mode slots, then single mode's effect.
    slots: [Slot; 4],
    ring: Ring,
    release: Release,
}

impl Channel {
    fn new() -> Channel {
        Channel { slots: [Slot::new(), Slot::new(), Slot::new(), Slot::new()], ring: Ring::new(), release: Release { spin: crate::fx_spin::BackSpin::new(48000.0), brake: crate::fx_spin::VinylCore::new(48000.0), kind: 0, t: 0.0, pos: 0.0, speed: 1.0, start: 0, gate: 0.0, held: false } }
    }

    /// A channel that only ever runs its first slot (PAD FX).
    fn one_slot() -> Channel {
        Channel { slots: [Slot::new(), Slot::empty(), Slot::empty(), Slot::empty()], ..Channel::new_without_slots() }
    }

    fn new_without_slots() -> Channel {
        Channel { slots: [Slot::empty(), Slot::empty(), Slot::empty(), Slot::empty()], ring: Ring::new(), release: Release { spin: crate::fx_spin::BackSpin::new(48000.0), brake: crate::fx_spin::VinylCore::new(48000.0), kind: 0, t: 0.0, pos: 0.0, speed: 1.0, start: 0, gate: 0.0, held: false } }
    }

    fn process(&mut self, p: &UnitParams, ch: usize, buf: &mut [f32], rate: u32, beat: Option<f64>) {
        let frames = buf.len() / 2;
        let r = rate as f32;
        let holding = p.release.is_some();
        // While a RELEASE FX is held the unit's Beat FX are out (rekordbox cancels them).
        if !holding && self.release.gate <= 0.0 {
            for (i, s) in self.slots.iter_mut().enumerate() {
                // The mode that is not shown is switched off (and fades out).
                let mut sp = if i < 3 { p.slots[i] } else { p.one };
                if (i < 3) == p.single {
                    sp.on = false;
                }
                s.process(&sp, p.bpm[ch], buf, rate, &self.ring, beat, i < 3);
            }
        }
        // RELEASE FX: press and hold. The time is the beat value (rekordbox: VINYL BRAKE
        // 10 ms .. 12 s, ECHO 10 ms .. 4 s, BACK SPIN's stop time, with two beats as the
        // length it spins back over).
        if let Some((kind, beats)) = p.release {
            let beat_secs = 60.0 / p.bpm[ch].clamp(20.0, 1000.0);
            if !self.release.held {
                // (The history already has this buffer: the brake starts at its first frame.)
                let start = (self.ring.w + RING_LEN - frames % RING_LEN) % RING_LEN;
                self.release = Release { spin: self.release.spin, brake: self.release.brake, kind, t: 0.0, pos: 0.0, speed: 1.0, start, gate: self.release.gate, held: true };
                if kind == 0 {
                    // (`ReleaseFxVinylBrake`: DECAY TIME 10 ms .. 12 s, the brake from the press.)
                    self.release.brake.set_decay((beats * beat_secs).clamp(0.010, 12.0) * 1000.0);
                    self.release.brake.start(r);
                }
                if kind == 2 {
                    let secs = (beats * beat_secs).clamp(0.010, 12.0);
                    self.release.spin.start(r, secs * 1000.0, (2.0 * beat_secs).clamp(0.010, 4.0) * 1000.0, frames);
                }
            }
            let secs = (beats * beat_secs).clamp(0.010, 12.0);
            let total = secs * r;
            if self.release.kind == 2 {
                self.release.spin.begin_call();
            }
            // VINYL BRAKE: the buffer's speeds (rekordbox works them out a call at a time).
            let mut vb_speed = [0.0f32; 4096];
            if self.release.kind == 0 {
                self.release.brake.set_decay(secs * 1000.0);
                for k in vb_speed[..frames.min(4096)].chunks_mut(4096) {
                    self.release.brake.speeds(k);
                }
            }
            let gk = 1.0 / (SMOOTH_SECS * r);
            for n in 0..frames {
                let rel = &mut self.release;
                rel.gate = (rel.gate + gk).min(1.0);
                let (wl, wr) = match rel.kind {
                    1 => {
                        // ECHO (`ReleaseEcho`): for one echo time after the press the track
                        // plays on and is taken into the echo; then the track is cut and that
                        // stretch repeats at 0.7 (the output gain) x 0.7 per turn (feedback).
                        let len = (total.min(4.0 * r) as usize).clamp(1, RING_LEN - 1);
                        let i = rel.t as usize;
                        let turn = i / len;
                        if turn == 0 {
                            (buf[n * 2], buf[n * 2 + 1])
                        } else {
                            let k = (rel.start + i % len) % RING_LEN;
                            let a = 0.7f32.powi(turn as i32);
                            (self.ring.buf[k * 2] * a, self.ring.buf[k * 2 + 1] * a)
                        }
                    }
                    2 => {
                        // BACK SPIN (`crate::fx_spin`): the sound before the press read
                        // backwards at rekordbox's speed, weighted and filtered by the speed.
                        let (h0, ring) = (rel.start as i64 - 1, &self.ring);
                        rel.spin.frame(|k| {
                            let i = (h0 - k as i64).rem_euclid(RING_LEN as i64) as usize;
                            (ring.buf[i * 2], ring.buf[i * 2 + 1])
                        })
                    }
                    _ => {
                        // VINYL BRAKE (`ReleaseFxVinylBrake` on `crate::fx_spin::VinylCore`):
                        // the sound from the press at a falling speed, weighted and filtered.
                        if n > 0 && n % 4096 == 0 {
                            let m = (frames - n).min(4096);
                            rel.brake.speeds(&mut vb_speed[..m]);
                        }
                        let (h0, ring) = (rel.start, &self.ring);
                        rel.brake.frame(vb_speed[n % 4096], |k| {
                            let i = (h0 + k) % RING_LEN;
                            (ring.buf[i * 2], ring.buf[i * 2 + 1])
                        })
                    }
                };
                rel.t += 1.0;
                let g = rel.gate;
                buf[n * 2] = buf[n * 2] * (1.0 - g) + wl * g;
                buf[n * 2 + 1] = buf[n * 2 + 1] * (1.0 - g) + wr * g;
            }
        } else {
            self.release.held = false;
            if self.release.gate > 0.0 {
                // Let go: the track comes back over 3.33 ms.
                let gk = 1.0 / (SMOOTH_SECS * r);
                for n in 0..frames {
                    self.release.gate = (self.release.gate - gk).max(0.0);
                    let g = 1.0 - self.release.gate;
                    buf[n * 2] *= g;
                    buf[n * 2 + 1] *= g;
                }
            }
        }
    }

    /// Keep the channel's dry history (before the effects) for ROLL and RELEASE FX.
    fn remember(&mut self, buf: &[f32]) {
        self.ring.write(buf);
    }
}

/// The master's channel index in `UnitParams::assign`.
pub const MASTER: usize = 4;
/// The sampler deck's channel index (FX assign "S").
pub const SAMPLER: usize = 5;
pub const CHANNELS: usize = 6;

pub struct Unit {
    pub params: UnitParams,
    /// One effect state per channel, made (by the interface, not the audio thread) the first
    /// time the unit is assigned to that channel: each holds about 12 MB of delay lines.
    channels: [Option<Box<Channel>>; CHANNELS],
}

impl Unit {
    /// Make the effect states of the channels this unit is assigned to.
    pub fn prepare(&mut self, assign: &[bool; CHANNELS]) {
        for (c, a) in self.channels.iter_mut().zip(assign) {
            if *a && c.is_none() {
                *c = Some(Box::new(Channel::new()));
            }
        }
    }
}

/// What a deck's PAD FX pads ask for at the moment (set by the interface): a Beat FX or
/// Scene FX (as a multi-mode slot), a Sound Color FX (effect, COLOR, PARAM) and a RELEASE FX
/// (which one and its beats).
#[derive(Clone, Copy)]
pub struct PadSet {
    pub beat: Option<SlotParams>,
    pub color: Option<(u8, f32, f32)>,
    pub release: Option<(u8, f32)>,
}

impl PadSet {
    pub const NONE: PadSet = PadSet { beat: None, color: None, release: None };

    pub fn any(&self) -> bool {
        self.beat.is_some() || self.color.is_some() || self.release.is_some()
    }
}

/// A deck's PAD FX unit: one effect, one Sound Color FX and the release effects.
struct PadChan {
    ch: Channel,
    cfx: crate::cfx::Cfx,
    /// The last Beat FX, kept so it can fade out, and the colour effect in use.
    last: SlotParams,
    color: (u8, f32),
    /// Frames since anything was asked of it (it rests once its tails are over).
    idle: u32,
}

impl PadChan {
    fn new() -> Self {
        Self { ch: Channel::one_slot(), cfx: crate::cfx::Cfx::new(), last: SlotParams::new(Kind::Delay), color: (0, 0.5), idle: u32::MAX }
    }
}

/// Both FX units and the scratch buffers the decks are rendered into on their way through them.
pub struct Rack {
    pub units: [Unit; 2],
    pub scratch: [Vec<f32>; 4],
    /// The interface has the FX panel in use (else the audio path skips the units).
    pub enabled: bool,
    /// The entire rack, including open-ended pad and merge effects, belongs to Performance.
    pub performance: bool,
    /// The master's Sound Color FX: effect, COLOR (0.5 = off), PARAM.
    pub master_cfx: crate::cfx::Cfx,
    pub master_cfx_set: (u8, f32, f32),
    /// CFX on the sampler deck (kind, COLOR, PARAM); COLOR 0.5 = off.
    pub sampler_cfx: crate::cfx::Cfx,
    pub sampler_cfx_set: (u8, f32, f32),
    /// Where each channel's deck is on its beat grid at the start of the buffer being
    /// processed (beats; set by the audio thread, `None` without a grid or while stopped).
    pub beat: [Option<f64>; CHANNELS],
    /// PAD FX: a unit per deck (made by the interface when a deck first shows PAD FX), what
    /// its pads ask for, the deck's tempo and its Beat FX quantize.
    pads: [Option<Box<PadChan>>; 4],
    pub pad_set: [PadSet; 4],
    pub pad_bpm: [f32; 4],
    pub pad_quantize: [bool; 4],
    /// MERGE FX (made by the interface when it is first enabled).
    pub merge: Option<Box<crate::mfx::Engine>>,
    /// The master's RELEASE FX (Preferences > Controller > Effect > Unit Number 1): which one
    /// and its beats while held, the tempo, and its state (made by the interface).
    pub master_release: Option<(u8, f32)>,
    pub master_bpm: f32,
    master_rel: Option<Box<Channel>>,
}

impl Rack {
    pub fn new() -> Rack {
        let unit = || Unit { params: UnitParams::new(), channels: [None, None, None, None, None, None] };
        Rack { units: [unit(), unit()], scratch: std::array::from_fn(|_| vec![0.0; 16384]), enabled: false, performance: true, master_cfx: crate::cfx::Cfx::new(), master_cfx_set: (0, 0.5, 0.5), sampler_cfx: crate::cfx::Cfx::new(), sampler_cfx_set: (0, 0.5, 0.5), beat: [None; CHANNELS], pads: [None, None, None, None], pad_set: [PadSet::NONE; 4], pad_bpm: [120.0; 4], pad_quantize: [false; 4], merge: None, master_release: None, master_bpm: 120.0, master_rel: None }
    }

    #[cfg(test)]
    pub fn prepare_rev(&mut self) {
        for u in self.units.iter_mut() {
            let p = u.params;
            for ch in u.channels.iter_mut().flatten() {
                for (i, s) in ch.slots.iter_mut().enumerate() {
                    let k = if i < 3 { p.slots[i].kind } else { p.one.kind };
                    s.ensure_rev(k, i == 3);
                }
            }
        }
        for (pc, set) in self.pads.iter_mut().zip(self.pad_set.iter()) {
            if let (Some(pc), Some(b)) = (pc.as_deref_mut(), set.beat) {
                pc.ch.slots[0].ensure_rev(b.kind, false);
            }
        }
    }

    pub fn has_pad(&self, ch: usize) -> bool {
        ch < 4 && self.pads[ch].is_some()
    }

    fn pads_busy(&self) -> bool {
        self.pads.iter().zip(&self.pad_set).any(|(p, s)| p.as_ref().is_some_and(|p| s.any() || p.idle < 20 * 48000))
    }

    /// Something is assigned and switched on (or held, or still fading): the effects must run.
    pub fn active(&self) -> bool {
        self.performance && (self.enabled && (self.units.iter().any(|u| u.params.assign.iter().any(|a| *a)) || (self.master_cfx_set.1 - 0.5).abs() > 0.011 || (self.sampler_cfx_set.1 - 0.5).abs() > 0.011) || self.pads_busy() || self.merge.as_ref().is_some_and(|m| m.busy()) || self.master_rel.as_ref().is_some_and(|c| self.master_release.is_some() || c.release.gate > 0.0))
    }

    /// The sampler's mix has to come through here (a unit is assigned to it, or its CFX is on).
    pub fn sampler_in_use(&self) -> bool {
        self.enabled && (self.units.iter().any(|u| u.params.assign[SAMPLER]) || (self.sampler_cfx_set.1 - 0.5).abs() > 0.011)
    }

    /// Run one channel's audio (stereo interleaved) through what is assigned to it.
    /// `ch`: 0..3 = decks 1 to 4, 4 = the master, 5 = the sampler. A deck's order: MERGE FX,
    /// its PAD FX unit, then FX1 and FX2.
    pub fn process(&mut self, ch: usize, buf: &mut [f32], rate: u32) {
        // fx9: the order of the units on one place is rekordbox's `FxUnitProcessor` slots
        // (`FxUnitProcessor::processBlock` 0x102aa1388 runs slots 0 .. 6 in turn; the slot is
        // the last argument of `DjSystem::setChannelFx / setMasterFx / setAuxFx`): on the master
        // the RELEASE FX panel's unit in slot 0 (`RbxReleaseFxPanel` 0x1009396d0), the Beat FX
        // units in slot 2 (`RekordboxMulti/SingleFxUnitContainer`), CFX in slot 4
        // (`RekordboxDjColorFxUnitContainer` 0x102a668c8); on the sampler Beat FX (2) then
        // CFX (4). MERGE FX's sampler joins the mix before the master's effects (its own
        // channel, `MergeFxSamplerChannelGraph`).
        if ch == MASTER {
            if let Some(m) = self.merge.as_deref_mut() {
                m.master(buf, rate);
            }
            if let Some(c) = self.master_rel.as_deref_mut() {
                c.remember(buf);
                let mut up = UnitParams::new();
                up.slots.iter_mut().for_each(|s| s.on = false);
                up.release = self.master_release;
                up.bpm = [self.master_bpm; CHANNELS];
                c.process(&up, MASTER, buf, rate, None);
            }
        }
        if ch < 4 {
            if let Some(m) = self.merge.as_deref_mut() {
                m.deck(ch, buf, rate, self.beat[ch]);
            }
            let set = self.pad_set[ch];
            if let Some(pc) = self.pads[ch].as_deref_mut() {
                if set.any() {
                    pc.idle = 0;
                } else {
                    pc.idle = pc.idle.saturating_add((buf.len() / 2) as u32);
                }
                if pc.idle < 20 * 48000 {
                    pc.ch.remember(buf);
                    if let Some((k, c, p)) = set.color {
                        pc.color = (k, p);
                        pc.cfx.process(k, c, p, buf, rate);
                    } else {
                        // (Centre: the effect glides out and leaves the path.)
                        pc.cfx.process(pc.color.0, 0.5, pc.color.1, buf, rate);
                    }
                    let mut up = UnitParams::new();
                    match set.beat {
                        Some(sp) => {
                            pc.last = sp;
                            up.slots[0] = sp;
                        }
                        None => up.slots[0] = SlotParams { on: false, ..pc.last },
                    }
                    up.slots[1].on = false;
                    up.slots[2].on = false;
                    up.release = set.release;
                    up.bpm = [self.pad_bpm[ch]; CHANNELS];
                    let beat = if self.pad_quantize[ch] { self.beat[ch] } else { None };
                    pc.ch.process(&up, ch, buf, rate, beat);
                }
            }
        }
        if !self.enabled {
            return;
        }
        for u in self.units.iter_mut() {
            if u.params.assign[ch] {
                let p = u.params;
                if let Some(c) = u.channels[ch].as_deref_mut() {
                    c.remember(buf);
                    let beat = if p.quantize[ch] { self.beat[ch] } else { None };
                    c.process(&p, ch, buf, rate, beat);
                }
            }
        }
        // (fx9: CFX after the Beat FX, slot 4.)
        if ch == MASTER {
            let (k, c, p) = self.master_cfx_set;
            self.master_cfx.process(k, c, p, buf, rate);
        }
        if ch == SAMPLER && (self.sampler_cfx_set.1 - 0.5).abs() > 0.011 {
            let (k, c, p) = self.sampler_cfx_set;
            self.sampler_cfx.process(k, c, p, buf, rate);
        }
    }
}

/// The rack, made when Performance mode first needs it (about 30 MB of delay lines).
pub static RACK: Mutex<Option<Box<Rack>>> = Mutex::new(None);

/// Allocate a missing buffer with the callback's mutex released. If another producer
/// filled this slot meanwhile, keep the live state and free the unused buffer unlocked.
fn prepare_box<R, T>(
    guard: &mut MutexGuard<'_, R>,
    slot: impl Fn(&mut R) -> &mut Option<Box<T>>,
    make: impl FnOnce() -> T,
) {
    if slot(guard).is_some() {
        return;
    }
    let made = MutexGuard::unlocked(guard, || Box::new(make()));
    let dest = slot(guard);
    if dest.is_none() {
        *dest = Some(made);
    } else {
        MutexGuard::unlocked(guard, || drop(made));
    }
}

/// Build a selected effect's optional DSP cores before making the setting visible to audio.
fn prepare_slot(
    guard: &mut MutexGuard<'_, Option<Box<Rack>>>,
    slot: impl Fn(&mut Rack) -> &mut Slot,
    kind: Kind,
    single: bool,
) {
    let bits = slot(guard.as_deref_mut().expect("rack initialized")).rev_needs(kind, single);
    if bits != 0 {
        let made = MutexGuard::unlocked(guard, || NewCores::make(bits, single));
        let old = slot(guard.as_deref_mut().expect("rack initialized")).rev_install(kind, single, made);
        MutexGuard::unlocked(guard, || drop(old));
    }
}

/// The UI is the producer of these long-lived buffers. A normal settings update
/// acquires the rack once. Only a newly requested buffer requires an unlock.
pub fn lock_prepared(
    shared: &Mutex<Option<Box<Rack>>>,
    settings: [UnitParams; 2],
    pads: [bool; 4],
    pad_sets: [PadSet; 4],
    master_release: bool,
    merge: bool,
) -> MappedMutexGuard<'_, Rack> {
    fn rack(slot: &mut Option<Box<Rack>>) -> &mut Rack {
        slot.as_deref_mut().expect("rack initialized")
    }
    let mut guard = shared.lock();
    prepare_box(&mut guard, |slot| slot, Rack::new);
    for (u, params) in settings.iter().enumerate() {
        for (ch, &wanted) in params.assign.iter().enumerate() {
            if wanted {
                prepare_box(&mut guard, |slot| &mut rack(slot).units[u].channels[ch], Channel::new);
                let active_slots = if params.single { 3..4 } else { 0..3 };
                for i in active_slots {
                    let kind = if params.single { params.one.kind } else { params.slots[i].kind };
                    prepare_slot(&mut guard, |r| &mut r.units[u].channels[ch].as_mut().unwrap().slots[i], kind, params.single);
                }
            }
        }
    }
    for (ch, wanted) in pads.into_iter().enumerate() {
        if wanted {
            prepare_box(&mut guard, |slot| &mut rack(slot).pads[ch], PadChan::new);
            if let Some(beat) = pad_sets[ch].beat {
                prepare_slot(&mut guard, |r| &mut r.pads[ch].as_mut().unwrap().ch.slots[0], beat.kind, false);
            }
        }
    }
    if master_release {
        prepare_box(&mut guard, |slot| &mut rack(slot).master_rel, Channel::new_without_slots);
    }
    if merge {
        prepare_box(&mut guard, |slot| &mut rack(slot).merge, crate::mfx::Engine::new);
    }
    MutexGuard::map(guard, rack)
}

/// Tap tempo, as rekordbox's: gaps under 60 ms are ignored, over 1.5 s start again; the mean
/// of up to 11 intervals; a tap more than 255 ms or 16.7 % off the mean starts again.
pub struct Tap {
    last: Option<f64>,
    gaps: Vec<f64>,
}

impl Tap {
    pub fn new() -> Tap {
        Tap { last: None, gaps: Vec::new() }
    }

    /// A tap at `now` seconds; the tempo so far, if there is one.
    pub fn tap(&mut self, now: f64) -> Option<f32> {
        if let Some(l) = self.last {
            let gap = now - l;
            if gap < 0.060 {
                return self.bpm();
            }
            if gap > 1.5 {
                self.gaps.clear();
            } else {
                if let Some(mean) = self.mean() {
                    if (gap - mean).abs() > 0.255 || (gap - mean).abs() / mean > 0.167 {
                        self.gaps.clear();
                    }
                }
                self.gaps.push(gap);
                if self.gaps.len() > 11 {
                    self.gaps.remove(0);
                }
            }
        }
        self.last = Some(now);
        self.bpm()
    }

    fn mean(&self) -> Option<f64> {
        if self.gaps.is_empty() { None } else { Some(self.gaps.iter().sum::<f64>() / self.gaps.len() as f64) }
    }

    fn bpm(&self) -> Option<f32> {
        self.mean().map(|m| (60.0 / m).clamp(20.0, 1000.0) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The interface makes the cores without the rack's lock (`rev_needs`, `NewCores::make`,
    /// `rev_install`); `ensure_rev` makes them in place (tests, MERGE FX). Both must cover
    /// the same cores for every effect and mode, or an effect would run without its core
    /// in the app only.
    #[test]
    fn cores_made_outside_the_lock_are_the_ones_ensure_rev_makes() {
        use Kind::*;
        let kinds = [Delay, Echo, Spiral, Reverb, RevDelay, MtDelay, PitchEcho, UpEcho, DownEcho, Trans, Pan, Filter, Flanger, Phaser, SlipRoll, Roll, RevRoll, Robot, Pitch, EnigmaJet, MobiusSaw, MobiusTri, LowCutEcho, PingPong, Helix, VinylBrake, Stretch, BpfEcho, Noise, SpiralUp, ReverbUp, HpfEcho, LpfEcho, CrushEcho, SpiralDown, ReverbDown];
        for k in kinds {
            for single in [false, true] {
                let mut a = Slot::new();
                a.ensure_rev(k, single);
                assert_eq!(a.rev_needs(k, single), 0, "{k:?} single {single}: ensure_rev leaves something rev_needs asks for");
                let mut b = Slot::new();
                let bits = b.rev_needs(k, single);
                let _old = b.rev_install(k, single, NewCores::make(bits, single));
                assert_eq!(b.rev_needs(k, single), 0, "{k:?} single {single}: the installed cores do not cover the need");
                let has = |s: &Slot| (s.rv.is_some(), s.ec.is_some(), s.pe.is_some(), s.mt.is_some(), s.fxp.as_ref().map(|f| f.single()), s.nzf.ready(), s.vbh.is_some());
                assert_eq!(has(&a), has(&b), "{k:?} single {single}: the two ways make different cores");
            }
        }
    }


    fn tone(frames: usize, freq: f32) -> Vec<f32> {
        (0..frames).flat_map(|i| { let v = (i as f32 * freq * std::f32::consts::TAU / 48000.0).sin() * 0.5; [v, v] }).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn unit(kind: Kind, value: f32, level: f32) -> UnitParams {
        let mut p = UnitParams::new();
        p.assign = [true, false, false, false, false, false];
        p.slots[0] = SlotParams { on: true, level, value, ..SlotParams::new(kind) };
        p.bpm = [120.0; CHANNELS];
        p
    }

    #[test]
    fn first_use_allocation_releases_the_lock_and_preserves_live_state() {
        let shared = Mutex::new(None);
        let mut guard = shared.lock();
        prepare_box(&mut guard, |slot| slot, || {
            let mut concurrent = shared.try_lock().expect("audio must be able to take the lock");
            *concurrent = Some(Box::new(27));
            10
        });
        assert_eq!(guard.as_deref(), Some(&27));
        prepare_box(&mut guard, |slot| slot, || panic!("existing buffer was recreated"));
    }

    #[test]
    fn prepared_rack_allocates_only_requested_buffers_and_keeps_channel_history() {
        let shared = Mutex::new(None);
        let mut settings = [UnitParams::new(); 2];
        settings[0].assign = [false; CHANNELS];
        settings[1].assign = [false; CHANNELS];
        settings[0].assign[2] = true;
        {
            let mut r = lock_prepared(&shared, settings, [false; 4], [PadSet::NONE; 4], false, false);
            let ch = r.units[0].channels[2].as_deref_mut().unwrap();
            ch.remember(&[0.125, -0.25]);
            assert_eq!(ch.ring.w, 1);
            assert!(r.units[1].channels.iter().all(Option::is_none));
            assert!(r.pads.iter().all(Option::is_none) && r.master_rel.is_none() && r.merge.is_none());
        }
        let r = lock_prepared(&shared, settings, [true, false, false, false], [PadSet::NONE; 4], false, false);
        assert!(r.has_pad(0) && !r.has_pad(1));
        let ring = &r.units[0].channels[2].as_ref().unwrap().ring;
        assert_eq!((ring.w, ring.at(0.0)), (1, (0.125, -0.25)));
    }

    #[test]
    fn rack_preparation_builds_only_visible_slot_cores_and_preserves_other_decks() {
        let shared = Mutex::new(None);
        let mut settings = [UnitParams::new(); 2];
        for unit in &mut settings {
            unit.assign.fill(false);
        }
        settings[0].assign[0] = true;
        settings[0].single = true;
        settings[0].one.kind = Kind::BpfEcho;
        settings[0].slots[1].kind = Kind::PitchEcho;
        let pads = [false, true, false, false];
        let mut sets = [PadSet::NONE; 4];
        sets[1].beat = Some(SlotParams::new(Kind::MtDelay));
        {
            let rack = lock_prepared(&shared, settings, pads, sets, false, false);
            let slots = &rack.units[0].channels[0].as_ref().unwrap().slots;
            assert!(slots[3].fxp.as_ref().is_some_and(|core| core.single()));
            assert!(slots[1].pe.is_none());
            assert!(rack.units[0].channels[1].is_none());
            assert!(rack.pads[1].as_ref().unwrap().ch.slots[0].mt.is_some());
        }
        settings[0].single = false;
        let rack = lock_prepared(&shared, settings, pads, sets, false, false);
        let slots = &rack.units[0].channels[0].as_ref().unwrap().slots;
        assert!(slots[1].pe.is_some());
        assert!(slots[3].fxp.as_ref().is_some_and(|core| core.single()));
    }

    #[test]
    fn nothing_on_leaves_the_sound_untouched() {
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params.assign = [true, true, false, false, true, false];
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let x = tone(4800, 440.0);
        let mut y = x.clone();
        rack.process(0, &mut y, 48000);
        rack.process(MASTER, &mut y, 48000);
        assert_eq!(x, y);
    }

    #[test]
    fn delay_repeats_once_one_beat_later_and_echo_keeps_repeating() {
        // A 10 ms burst, then silence; 120 BPM, 1 beat = 0.5 s = 24,000 frames.
        let burst = |n: usize| -> Vec<f32> { (0..n).flat_map(|i| { let v = if i < 480 { 0.5 } else { 0.0 }; [v, v] }).collect() };
        for (kind, third) in [(Kind::Delay, false), (Kind::Echo, true)] {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params = unit(kind, 1.0, 0.75);
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            let mut x = burst(48000 * 2);
            rack.process(0, &mut x, 48000);
            let at = |f: usize| rms(&x[f * 2..(f + 480) * 2]);
            assert!(at(0) > 0.45, "{kind:?}: the dry burst");
            assert!(at(12000) < 0.01, "{kind:?}: silence between");
            assert!(at(24000) > 0.4, "{kind:?}: the repeat a beat later, {}", at(24000));
            if third {
                // ECHO: the second repeat at 0.7 of the first.
                assert!((at(48000) / at(24000) - 0.7).abs() < 0.05, "{}", at(48000) / at(24000));
            } else {
                assert!(at(48000) < 0.01, "DELAY repeats once");
            }
        }
    }

    #[test]
    fn rev_delay_plays_each_beat_backwards_from_the_restart() {
        // REV DELAY (multi, 1 beat at 120 BPM = 24000 frames, LEVEL/DEPTH 1: the wet sound
        // alone), no beat grid: the quantizer starts it at once and restarts it every 24000
        // frames. After the restart at 24000 the gate shuts (159 frames), the read point goes
        // to the frame written just before (23999) and the input plays backwards from there.
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::RevDelay, 1.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        rack.prepare_rev();
        let x: Vec<f32> = (0..60000).flat_map(|t| { let v = (t % 20000) as f32 * 1e-4 - 1.0; [v, v] }).collect();
        let mut y = x.clone();
        for c in y.chunks_mut(960) {
            rack.process(0, c, 48000);
        }
        let j = 24000 + 159;
        for k in [200usize, 5000, 20000] {
            assert!((y[(j + k) * 2] - x[(23999 - k) * 2]).abs() < 1e-5, "{k}: {} vs {}", y[(j + k) * 2], x[(23999 - k) * 2]);
        }
    }

    #[test]
    fn echo_rev_fades_to_the_reverse_core() {
        // Single-mode ECHO with REV held: the forward echo's output gate is shut and the reverse
        // core's open (`BeatFxEcho`); the click comes back reversed after the first restart and
        // then repeats at the feedback (0.7 at the knob's centre) every beat.
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(Kind::Echo, 1.0, 1.0);
        p.single = true;
        p.one = SlotParams { on: true, level: 1.0, value: 1.0, knob: [0.5, 0.5], btn: [true, false], ..SlotParams::new(Kind::Echo) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        rack.prepare_rev();
        let mut y: Vec<f32> = (0..96000).flat_map(|t| { let v = if t == 10000 { 1.0 } else { 0.0 }; [v, v] }).collect();
        for c in y.chunks_mut(960) {
            rack.process(0, c, 48000);
        }
        let peaks: Vec<(usize, f32)> = (0..96000).filter(|&t| y[t * 2].abs() > 0.05).map(|t| (t, y[t * 2])).collect();
        // The click (dry gone at LEVEL/DEPTH 1 once the line is full) is read back after the
        // restart at 24000: shut 159 frames, read from 23999 back to 10000 = frame 38158.
        assert!(peaks.iter().any(|&(t, _)| t == 24159 + 13999), "{peaks:?}");
        let first = peaks.iter().find(|&&(t, _)| t == 38158).unwrap().1;
        let next = peaks.iter().find(|&&(t, _)| t == 38158 + 24000).map(|p| p.1).unwrap_or(0.0);
        assert!((next / first - 0.7).abs() < 0.02, "{first} {next}");
    }

    #[test]
    fn pitch_echo_rev_runs_the_pitch_reverse_core() {
        // PITCH ECHO, REV held, PITCH at the centre (no shift): `PitchReverseEchoCore` gives the
        // click back reversed after the restart at 24000 (read at frame 38158), through its
        // shifter, then the repeats at the feedback (0.85 at the knob's centre).
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(Kind::Echo, 1.0, 1.0);
        p.single = true;
        p.one = SlotParams { on: true, level: 0.5, value: 1.0, knob: [0.5, 0.5], btn: [true, false], ..SlotParams::new(Kind::PitchEcho) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        rack.prepare_rev();
        let mut y: Vec<f32> = (0..96000).flat_map(|t| { let v = if t == 10000 { 1.0 } else { 0.0 }; [v, v] }).collect();
        for c in y.chunks_mut(960) {
            rack.process(0, c, 48000);
        }
        assert!(y.iter().all(|v| v.is_finite()));
        let e = |a: usize| y[(a - 600) * 2..(a + 600) * 2].iter().map(|v| v * v).sum::<f32>().sqrt();
        // (The shifter in the loop adds its 30 ms start-up delay, 1440 frames, to each pass.)
        let (a, b) = (e(38158 + 1440), e(38158 + 24000 + 2 * 1440));
        assert!(a > 0.2 && (b / a - 0.85).abs() < 0.05, "{a} {b}");
    }

    #[test]
    fn level_depth_law_of_the_echo_family() {
        // LEVEL/DEPTH at 0: no echo at all. Fully right: the echo only.
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Echo, 1.0, 0.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let x = tone(48000, 440.0);
        let mut y = x.clone();
        rack.process(0, &mut y, 48000);
        assert!(x.iter().zip(&y).all(|(a, b)| (a - b).abs() < 1e-6));
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Echo, 1.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut y = x.clone();
        for c in y.chunks_mut(1024) {
            rack.process(0, c, 48000);
        }
        // Until the line holds the delay (a beat, 24000 frames) the dry sound stays whole
        // (`MultiBeatFxEcho::updateParameter`); from the block after it, only the repeats.
        assert!((rms(&y[2000..40000]) - rms(&x[2000..40000])).abs() < 0.01);
        assert!(rms(&y[2 * 24600..2 * 25000]) > 0.1);
        let d: Vec<f32> = y[2 * 24600..2 * 30000].iter().zip(&x[2 * (24600 - 24000)..2 * (30000 - 24000)]).map(|(a, b)| a - b).collect();
        assert!(rms(&d) < 0.01, "only the repeat: {}", rms(&d));
    }

    #[test]
    fn trans_gates_half_of_each_beat_value() {
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Trans, 1.0, 0.5);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(48000, 440.0);
        rack.process(0, &mut x, 48000);
        // (LEVEL/DEPTH at the centre: the gate alone, open half of the turn.)
        // 1 beat at 120 BPM = 24,000 frames: open for the first half, closed for the second.
        assert!(rms(&x[2000 * 2..10000 * 2]) > 0.3);
        assert!(rms(&x[14000 * 2..22000 * 2]) < 0.01);
        assert!(rms(&x[26000 * 2..34000 * 2]) > 0.3);
        // Fully up it is open for a sixteenth of the turn (1,500 frames).
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Trans, 1.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(48000, 440.0);
        rack.process(0, &mut x, 48000);
        assert!(rms(&x[300 * 2..1200 * 2]) > 0.3 && rms(&x[2000 * 2..23000 * 2]) < 0.01);
    }

    #[test]
    fn roll_loops_the_last_beat_value() {
        // rekordbox's ROLL records from the point it starts and plays that back: the first turn
        // is the sound itself, every restart (each beat value) fades out over 3.3333 ms (159
        // frames) and plays the turn's start again.
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params.assign = [true, false, false, false, false, false];
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let ramp = |from: usize, n: usize| -> Vec<f32> { (from..from + n).flat_map(|i| { let v = i as f32 / 96000.0; [v, v] }).collect() };
        let mut hist = ramp(0, 48000);
        rack.process(0, &mut hist, 48000);
        // ROLL on, 1/4 beat = 6,000 frames at 120 BPM (no beat grid: it starts at once).
        rack.units[0].params = unit(Kind::Roll, 0.25, 1.0);
        let input = ramp(48000, 20000);
        let mut x = input.clone();
        for c in x.chunks_mut(480 * 2) {
            rack.process(0, c, 48000);
        }
        let v = |f: usize| x[f * 2];
        let src = |k: usize| input[k * 2];
        assert!((v(3000) - src(3000)).abs() < 1e-6, "{} vs {}", v(3000), src(3000));
        assert!((v(9000) - src(9000 - 6159)).abs() < 1e-6, "second turn {} vs {}", v(9000), src(9000 - 6159));
        assert!((v(15000) - src(15000 - 12159)).abs() < 1e-6, "third turn {}", v(15000));
    }

    #[test]
    fn modulation_effects_and_reverb_stay_bounded_and_change_the_sound() {
        // Every effect, in multi mode (knobs centred) and in single mode with its knobs and
        // buttons at each extreme: bounded, audible, and after OFF the dry sound exactly.
        for info in INFO.iter() {
            let kind = info.kind;
            for (single, knob, btn) in [(false, [0.5, 0.5], [false, false]), (true, [0.0, 0.0], [true, false]), (true, [1.0, 1.0], [false, true]), (true, [1.0, 0.0], [true, true])] {
                if single && info.single_id < 0 {
                    continue;
                }
                let mut rack = Rack::new();
                rack.enabled = true;
                let mut p = unit(kind, default_value(kind), 0.8);
                if single {
                    p.single = true;
                    p.one = SlotParams { knob, btn, ..p.slots[0] };
                    p.slots[0].on = false;
                }
                rack.units[0].params = p;
                rack.units[0].prepare(&p.assign);
                // (A second of history first, as a playing deck would have.)
                rack.units[0].params.slots[0].on = false;
                rack.units[0].params.one.on = false;
                let mut pre = tone(48000, 330.0);
                rack.process(0, &mut pre, 48000);
                assert_eq!(pre, tone(48000, 330.0), "{kind:?} off is the dry sound");
                rack.units[0].params = p;
                let x = tone(48000 * 3, 330.0);
                let mut y = x.clone();
                for (i, chunk) in y.chunks_mut(512 * 2).enumerate() {
                    let _ = i;
                    rack.process(0, chunk, 48000);
                }
                let peak = y.iter().fold(0.0f32, |a, b| a.max(b.abs()));
                // (ENIGMA JET's frozen comb at full FEEDBACK rings up to 22 times at its peak, as
                // rekordbox's does: feedback 0.954 of the opposite sign.)
                let most = if kind == Kind::EnigmaJet { 10.0 } else { 4.0 };
                assert!(y.iter().all(|v| v.is_finite()) && peak < most, "{kind:?} {knob:?} {btn:?} peak {peak}");
                let diff = rms(&x.iter().zip(&y).map(|(a, b)| a - b).collect::<Vec<f32>>());
                // (HELIX's low cut + high cut leave it only a band of the 330 Hz tone; the
                // echoes with FEEDBACK at zero are quiet. All must still be heard.)
                // (PAN with LFO AMP at zero does not move the sound.)
                // (NOISE with VOL at zero is silent, as rekordbox's: VOL scales the noise.)
                // (REVERB UP's FILTER fully up is a 6.3 kHz high-pass on the reverb of a 330 Hz
                // tone.)
                // (fx7b: SPIRAL with FEEDBACK at zero is silent: `SpiralCore` writes
                // (repeat + input) x feedback, so the first repeat is at the feedback too.)
                assert!(diff > 0.002 || (matches!(kind, Kind::Pan | Kind::Noise | Kind::Spiral) && knob[0] == 0.0) || (kind == Kind::ReverbUp && knob[1] == 1.0), "{kind:?} {knob:?} {btn:?} does something ({diff})");
                rack.units[0].params.slots[0].on = false;
                rack.units[0].params.one.on = false;
                let dry = tone(4800, 330.0);
                let mut z = dry.clone();
                rack.process(0, &mut z, 48000);
                assert!(z[2000..] == dry[2000..], "{kind:?} {knob:?} {btn:?}: the dry sound is back after OFF");
            }
        }
    }

    #[test]
    fn lists_ids_tables_and_laws_match_rekordbox() {
        // 35 effects in multi mode, 34 in single; engine ids are a permutation of 0..n.
        for (single, n) in [(false, 35usize), (true, 34)] {
            assert_eq!(list(single).count(), n);
            let mut ids: Vec<i8> = list(single).map(|i| if single { i.single_id } else { i.multi_id }).collect();
            ids.sort();
            assert_eq!(ids, (0..n as i8).collect::<Vec<i8>>());
            for id in 0..n as i64 {
                assert!(from_id(id, single).is_some());
            }
        }
        assert_eq!(list(false).map(|i| i.name).take(9).collect::<Vec<_>>(), ["DELAY", "ECHO", "SPIRAL", "REVERB", "REV DELAY", "MT DELAY", "UP ECHO", "DOWN ECHO", "TRANS"]);
        assert_eq!(list(true).nth(6).unwrap().name, "PITCH ECHO");
        assert_eq!(list(true).nth(24).unwrap().name, "STRETCH");
        assert_eq!(list(false).nth(26).unwrap().name, "BPF ECHO");
        assert_eq!((from_id(3, true), from_id(2, false), from_id(12, false), from_id(11, false), from_id(34, false)), (Some(Kind::Spiral), Some(Kind::RevDelay), Some(Kind::RevRoll), Some(Kind::Robot), Some(Kind::Stretch)));
        // Defaults: VINYL BRAKE 4 beats, STRETCH 2, the signed table +4, ROBOT 0 %, REVERB 50 %.
        assert_eq!(default_value(Kind::VinylBrake), 4.0);
        assert_eq!(default_value(Kind::Stretch), 2.0);
        assert_eq!((default_value(Kind::EnigmaJet), beat_table(Kind::MobiusSaw).0.len()), (4.0, 22));
        assert_eq!((default_value(Kind::Robot), default_value(Kind::Pitch), default_value(Kind::Reverb)), (0.0, -50.0, 50.0));
        assert!(beat_table(Kind::BpfEcho).0.is_empty());
        assert_eq!(value_text(Kind::EnigmaJet, 4.0), "+4");
        assert_eq!(value_text(Kind::EnigmaJet, -0.0625), "-1/16");
        assert_eq!(value_text(Kind::Robot, -37.0), "-37%");
        assert_eq!(value_text(Kind::Noise, 0.0), "-");
        assert_eq!(info_of(Kind::VinylBrake).labels, ["REPEAT", "ECHO", "ON", "BPF"]);
        // Laws: the echo's feedback (0 .. 0.7 .. 1), MT DELAY's mix, STRETCH's speed.
        assert!((fb_law(0.0, 0.7), fb_law(0.5, 0.7), fb_law(1.0, 0.7)) == (0.0, 0.7, 1.0));
        assert!((fb_law(0.25, 0.7) - 0.35).abs() < 1e-6 && (fb_law(0.75, 0.7) - 0.85).abs() < 1e-6);
        assert_eq!(mt_mix(0.0), (1.0, 0.0));
        assert!((mt_mix(0.25).1 - 0.35).abs() < 1e-6 && (mt_mix(0.5).0 - 1.0).abs() < 1e-6 && (mt_mix(1.0).0 - 0.7).abs() < 1e-6 && mt_mix(1.0).1 == 0.7);
        assert_eq!((stretch_speed(0.0), stretch_speed(0.5), stretch_speed(1.0)), (1.0, 0.5, 0.25));
        assert_eq!((echo_mix(0.75), echo_mix(1.0)), ((1.0, 1.0), (0.0, 1.0)));
    }

    /// Cycles of the strongest tone, counted from zero crossings of the left channel.
    fn crossings(x: &[f32]) -> usize {
        x.chunks_exact(2).zip(x.chunks_exact(2).skip(1)).filter(|(a, b)| a[0] <= 0.0 && b[0] > 0.0).count()
    }

    #[test]
    fn pitch_shifts_by_its_percent_value() {
        // 440 Hz at +100 % is 880 Hz (rekordbox's law: -100 % is the octave down). LEVEL/DEPTH
        // at the centre gives the value as labelled; fully up doubles it. (The shifter's spectra
        // up and down, at 44.1 and 48 kHz, are checked in `fx_core`'s tests.)
        for (pct, want) in [(100.0, 880.0), (26.0, 554.4)] {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params = unit(Kind::Pitch, pct, 0.5);
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            let mut x = tone(48000 * 2, 440.0);
            rack.process(0, &mut x, 48000);
            let hz = crossings(&x[48000 * 2..]) as f32;
            assert!((hz - want).abs() < want * 0.08, "{pct}%: {hz} Hz, wanted {want}");
        }
    }

    #[test]
    fn single_mode_runs_its_one_effect_and_its_knobs_act() {
        let burst = |n: usize| -> Vec<f32> { (0..n).flat_map(|i| { let v = if i < 480 { 0.5 } else { 0.0 }; [v, v] }).collect() };
        let run = |knob: [f32; 2], btn: [bool; 2], kind: Kind| -> Vec<f32> {
            let mut rack = Rack::new();
            rack.enabled = true;
            let mut p = unit(Kind::Trans, 1.0, 1.0);
            // (The multi slot is on too: in single mode it must not act.)
            p.single = true;
            p.one = SlotParams { on: true, level: 0.75, value: 1.0, knob, btn, ..SlotParams::new(kind) };
            rack.units[0].params = p;
            rack.units[0].prepare(&p.assign);
            let mut x = burst(48000 * 2);
            for c in x.chunks_mut(1024) {
                rack.process(0, c, 48000);
            }
            x
        };
        let at = |x: &[f32], f: usize| rms(&x[f * 2..(f + 480) * 2]);
        // ECHO, FEEDBACK at the centre: repeats at 0.7; at the left end one repeat only; at
        // the right end they do not fade.
        let x = run([0.5, 0.5], [false, false], Kind::Echo);
        assert!(at(&x, 24000) > 0.4 && (at(&x, 48000) / at(&x, 24000) - 0.7).abs() < 0.05);
        let x = run([0.0, 0.5], [false, false], Kind::Echo);
        assert!(at(&x, 24000) > 0.4 && at(&x, 48000) < 0.01);
        let x = run([1.0, 0.5], [false, false], Kind::Echo);
        assert!((at(&x, 72000) / at(&x, 24000) - 1.0).abs() < 0.05);
        // FREEZE takes hold once the line holds the delay (`SingleBeatFxEcho`: the burst is in by
        // then); from there nothing new enters and the repeats keep their level (feedback 1).
        let x = run([0.5, 0.5], [false, true], Kind::Echo);
        assert!(at(&x, 24000) > 0.4 && (at(&x, 72000) / at(&x, 24000) - 1.0).abs() < 0.02, "{} {}", at(&x, 24000), at(&x, 72000));
        // FILTER to the left: the repeat is duller (a 10 ms step keeps less of its edge).
        let dull = run([0.5, 0.0], [false, false], Kind::Echo);
        let bright = run([0.5, 0.5], [false, false], Kind::Echo);
        let edge = |x: &[f32]| (x[24001 * 2] - x[23999 * 2]).abs();
        assert!(edge(&dull) < edge(&bright) * 0.5, "{} vs {}", edge(&dull), edge(&bright));
        // DELAY, LR RATIO at the left end (1:3): the left repeat at the beat value, the right one
        // at three times it (`SingleBeatFxDelayBase::updateAndSetParameter` multiplies).
        let x = run([0.5, 0.0], [false, false], Kind::Delay);
        assert!(x[24200 * 2] > 0.3 && x[24200 * 2 + 1].abs() < 0.01 && x[72200 * 2 + 1] > 0.3);
        // At the centre (1:1) both come together.
        let x = run([0.5, 0.5], [false, false], Kind::Delay);
        assert!(x[24200 * 2] > 0.3 && x[24200 * 2 + 1] > 0.3 && x[8200 * 2].abs() < 0.01);
        // MT DELAY with TAP NUM at the centre (4 taps, rekordbox's order): 2/8, 4/8, 6/8 and
        // 7/8 of the beat (an eighth is 3,000 frames); the others are silent.
        let x = run([0.5, 0.5], [false, false], Kind::MtDelay);
        for k in [2, 4, 6, 7] {
            assert!(at(&x, k * 3000) > 0.05, "tap {k}");
        }
        for k in [1, 3, 5, 8] {
            assert!(at(&x, k * 3000 + 100) < 0.001, "tap {k} off");
        }
        // TRANS in single mode with DUTY where rekordbox's law gives a quarter (0.5 - 0.4375 x
        // the right half's reading): open for a quarter of the beat.
        assert!((trans_single_duty(0.7910) - 0.25).abs() < 0.002 && trans_single_duty(1.0) == 0.0625 && trans_single_duty(0.0) == 1.0);
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(Kind::Echo, 1.0, 1.0);
        p.slots[0].on = false;
        p.single = true;
        p.one = SlotParams { on: true, level: 1.0, value: 1.0, knob: [0.7910, 0.5], ..SlotParams::new(Kind::Trans) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(48000, 440.0);
        rack.process(0, &mut x, 48000);
        assert!(rms(&x[1000 * 2..5000 * 2]) > 0.3 && rms(&x[7000 * 2..23000 * 2]) < 0.01 && rms(&x[25000 * 2..29000 * 2]) > 0.3);
    }

    #[test]
    fn quantize_puts_the_gate_and_the_roll_on_the_beat_grid() {
        // TRANS, 1 beat, switched on 0.3 beat into a beat. With quantize rekordbox's
        // `FxQuantizeManager` waits for the half beat (the sound untouched meanwhile), then
        // starts the gate there: open half a beat, shut until the next restart at 9.5. Without
        // quantize it starts at once.
        for quantize in [true, false] {
            let mut rack = Rack::new();
            rack.enabled = true;
            let mut p = unit(Kind::Trans, 1.0, 0.5);
            p.quantize = [quantize; CHANNELS];
            rack.units[0].params = p;
            rack.units[0].prepare(&p.assign);
            let mut x = tone(36000, 440.0);
            let mut beat = 8.3f64;
            for chunk in x.chunks_mut(480 * 2) {
                rack.beat[0] = Some(beat);
                rack.process(0, chunk, 48000);
                beat += 480.0 / 24000.0;
            }
            if quantize {
                let (w, o, c) = (rms(&x[500 * 2..4500 * 2]), rms(&x[6000 * 2..16000 * 2]), rms(&x[18000 * 2..28000 * 2]));
                assert!(w > 0.3 && o > 0.3 && c < 0.01, "{w} {o} {c}");
            } else {
                let (o, c) = (rms(&x[500 * 2..11000 * 2]), rms(&x[13000 * 2..23000 * 2]));
                assert!(o > 0.3 && c < 0.01, "{o} {c}");
            }
        }
        // ROLL, 1/2 beat, switched on 0.1 beat after a beat (`FxQuantizeManager::
        // turnOnWithAdjust`: started at once, 0.1 beat late).
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params.assign = [true, false, false, false, false, false];
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut hist: Vec<f32> = (0..48000).flat_map(|i| { let v = i as f32 / 48000.0; [v, v] }).collect();
        rack.process(0, &mut hist, 48000);
        rack.units[0].params = unit(Kind::Roll, 0.5, 1.0);
        rack.beat[0] = Some(4.1);
        let mut x = vec![0.0f32; 480 * 2];
        rack.process(0, &mut x, 48000);
        // Started 0.1 beat (2,400 frames) after beat 4: the roll plays the sound from beat 4,
        // which began 2,400 frames before this buffer.
        assert!((x[400 * 2] - (48000.0 - 2400.0 + 400.0) / 48000.0).abs() < 0.001, "{}", x[400 * 2]);
    }

    #[test]
    fn stretch_plays_slower_and_vinyl_brake_stops() {
        // STRETCH at LEVEL/DEPTH 0.5 with REPITCH: half speed, so 440 Hz is heard as 220 Hz.
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(Kind::Trans, 4.0, 0.5);
        p.slots[0].on = false;
        p.single = true;
        p.one = SlotParams { on: true, level: 0.5, value: 4.0, btn: [true, false], ..SlotParams::new(Kind::Stretch) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(48000, 440.0);
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        let hz = crossings(&x[24000 * 2..]) as f32 * 2.0;
        assert!((hz - 220.0).abs() < 8.0, "{hz}");
        // Without REPITCH the pitch is put back.
        rack = Rack::new();
        rack.enabled = true;
        p.one.btn = [false, false];
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(48000, 440.0);
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        let hz = crossings(&x[24000 * 2..]) as f32 * 2.0;
        assert!((hz - 440.0).abs() < 20.0, "{hz}");
        // VINYL BRAKE, 1 beat, LEVEL/DEPTH fully up, ECHO at zero: slowing over the beat and
        // (rekordbox's tail below 1/8 speed) stopped 0.6 s after it.
        let mut rack = Rack::new();
        rack.enabled = true;
        p.one = SlotParams { on: true, level: 1.0, value: 1.0, knob: [0.5, 0.0], ..SlotParams::new(Kind::VinylBrake) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        let mut x = tone(96000, 440.0);
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        assert!(rms(&x[2000 * 2..8000 * 2]) > 0.2 && rms(&x[56000 * 2..]) < 0.01);
    }

    #[test]
    fn single_roll_gates_follow_rekordbox() {
        // Centre: the roll alone; fully left: the live sound alone; half left: roll then live.
        assert_eq!(roll_gates(0.5), (1.0, 0.0, 0.0, 0.0));
        assert_eq!(roll_gates(0.0), (0.0, 0.0, 1.0, 0.0));
        let (rd, rh, dd, dh) = roll_gates(0.48627 / 2.0);
        assert!((rd - 0.5).abs() < 1e-4 && rh == 0.0 && (dd - 0.5).abs() < 1e-4 && (dh - 0.5).abs() < 1e-4);
        // Right: the roll's share shrinks to 5 %, the rest is silent.
        let (rd, _, dd, _) = roll_gates(1.0);
        assert!((rd - 0.05).abs() < 1e-6 && dd == 0.0);
        // In a run: single ROLL 1 beat, knob half left: the second half of each turn is the
        // live sound. The roll records from its start (440 Hz for the first turn), then the
        // live tone changes to 880 Hz; in the second turn the roll part is the 440 Hz.
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(Kind::Roll, 1.0, 1.0);
        p.single = true;
        p.one = SlotParams { on: false, level: 1.0, value: 1.0, knob: [0.48627 / 2.0, 0.5], ..SlotParams::new(Kind::Roll) };
        rack.units[0].params = p;
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(48000, 440.0);
        rack.process(0, &mut x, 48000);
        rack.units[0].params.one.on = true;
        let mut first = tone(24000, 440.0);
        for c in first.chunks_mut(480) {
            rack.process(0, c, 48000);
        }
        let mut y = tone(24000, 880.0);
        let live = y.clone();
        for c in y.chunks_mut(480) {
            rack.process(0, c, 48000);
        }
        // Turn = 1 beat = 24,000 frames: frames 14,000..23,000 of it are the live sound.
        let d: f32 = y[14000 * 2..23000 * 2].iter().zip(&live[14000 * 2..23000 * 2]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        let r: f32 = y[1000 * 2..11000 * 2].iter().zip(&live[1000 * 2..11000 * 2]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(d < 0.02 && r > 0.2, "live part off by {d}, roll part {r}");
    }

    #[test]
    fn helix_holds_its_loop_and_follows_a_new_time_at_track_speed() {
        // The speed law: an octave per 0.9 s up (TRACK SPEED 0.5), per 0.7 s down.
        let mut s = 1.0f64;
        let mut n = 0;
        while s < 2.0 {
            s = crate::fx_core::helix_speed(s, 2.0, 0.5, 48000.0);
            n += 1;
        }
        assert!((n as f32 / 48000.0 - 0.9).abs() < 0.001, "{n}");
        let mut s = 1.0f64;
        let mut n = 0;
        while s > 0.5 {
            s = crate::fx_core::helix_speed(s, 0.5, 0.5, 48000.0);
            n += 1;
        }
        assert!((n as f32 / 48000.0 - 0.7).abs() < 0.001, "{n}");
        assert_eq!((crate::fx_core::helix_gain(1.5), crate::fx_core::helix_gain(3.0), crate::fx_core::helix_gain(9.0)), (1.0, 0.5, 0.25));
        // In a run: single HELIX on a 440 Hz tone, then silence and TIME from the centre to the
        // right end (a quarter of the beat): the held loop goes up two octaves, to 1760 Hz.
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(Kind::Helix, 1.0, 0.5);
        p.single = true;
        p.one = SlotParams { on: true, level: 0.5, value: 1.0, knob: [0.5, 0.0], ..SlotParams::new(Kind::Helix) };
        rack.units[0].params = p;
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(48000, 440.0);
        for c in x.chunks_mut(480) {
            rack.process(0, c, 48000);
        }
        rack.units[0].params.one.knob[0] = 1.0;
        let mut y = vec![0.0f32; 48000 * 2];
        for c in y.chunks_mut(480) {
            rack.process(0, c, 48000);
        }
        // (fx7b: `HelixCore` then records the quarter-beat loop from the sped-up one (state 4)
        // and loops it at speed 1 (state 2): 6000 frames.)
        let slot = &rack.units[0].channels[0].as_ref().unwrap().slots[3];
        assert!(slot.hxc.state == 2 && slot.hxc.speed == 1.0, "{} {}", slot.hxc.state, slot.hxc.speed);
        let _ = &y;
    }

    #[test]
    fn single_robot_modulation_sets_the_cores_gains() {
        // `SingleFxRobot::updateParameter`: PITCH GAIN 1 - 0.01 v (v = MOD x 255, 0.75 from 25),
        // FLANGER GAIN = MOD, resonance 0.267; LEVEL/DEPTH is the mix (`calculateMixRatio`).
        let ring = Ring::new();
        let tone: Vec<f32> = (0..4800).flat_map(|i| { let v = (i as f32 * 0.05).sin() * 0.5; [v, v] }).collect();
        let run = |k2: f32, level: f32| {
            let mut s = Slot::new();
            let mut p = SlotParams { on: true, level, knob: [0.5, k2], ..SlotParams::new(Kind::Robot) };
            p.value = 0.0;
            let mut y = tone.clone();
            for c in y.chunks_mut(512 * 2) {
                s.process(&p, 120.0, c, 48000, &ring, None, false);
            }
            (y, s.ga)
        };
        let (_, ga) = run(0.0, 1.0);
        assert!((ga[0] - 1.0).abs() < 1e-6 && ga[1].abs() < 1e-6, "{ga:?}");
        let (_, ga) = run(1.0, 1.0);
        assert!((ga[0] - 0.75).abs() < 1e-6 && (ga[1] - 1.0).abs() < 1e-6, "{ga:?}");
        assert_eq!(half_mix(0.25), (0.5, 0.5));
        // LEVEL at 0: the dry sound alone.
        let (y, _) = run(1.0, 0.0);
        assert!(y.iter().zip(&tone).skip(2000).all(|(a, b)| (a - b).abs() < 1e-3));
    }

    #[test]
    fn single_reverb_decay_law() {
        // rekordbox's polynomial at the ends and the centre, and the input gain.
        assert!((reverb_decay_secs(1.0) - 1.5318).abs() < 1e-3 && (reverb_decay_secs(50.0) - 8.7007).abs() < 2e-3 && (reverb_decay_secs(100.0) - 100.6).abs() < 0.1, "{} {} {}", reverb_decay_secs(1.0), reverb_decay_secs(50.0), reverb_decay_secs(100.0));
        assert_eq!((reverb_decay_in_gain(0.0), reverb_decay_in_gain(0.5)), (0.2, 0.2));
        assert!((reverb_decay_in_gain(1.0) - 0.1).abs() < 1e-6);
        // The comb gains come out as rekordbox writes them: 10^(-k x size / time), with k =
        // 0.066281 for the first left comb (3 x 22.0938 ms).
        let (_, _, g) = crate::fx_verb::dyn_params(crate::fx_verb::VerbLaw::Single { decay: 50.0, gin: 0.2 }, 50);
        let want = 10f32.powf(-0.066281 * reverb_size(50.0) / reverb_decay_secs(50.0));
        assert!((g[0] - want).abs() < 1e-4, "{} {}", g[0], want);
    }

    #[test]
    fn phaser_stage_knob_law() {
        // `SingleBeatFxPhaser::updateParameter`, KnobPara2Assign 0: centre 6 / 6; left end 60 on
        // the left channel; right end 60 on the right; even counts on the way.
        assert_eq!((phaser_stages(0.5), phaser_stages(0.0), phaser_stages(1.0)), ((6, 6), (60, 6), (6, 60)));
        let (l, r) = phaser_stages(0.3);
        assert!(r == 6 && l % 2 == 0 && l == { let n = (through_value(0.3) * 54.0) as usize + 6; n + n % 2 });
    }

    #[test]
    fn spiral_time_moves_as_spiral_buffer_moves_it() {
        // SPIRAL at 120 BPM, 1/2 beat (12,000 frames), then 1 beat: the read point does not
        // jump; it sets off slowly (rekordbox's `SpiralBuffer`, 2 s time constant).
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Spiral, 0.5, 0.8);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let time = |rack: &Rack| rack.units[0].channels[0].as_ref().unwrap().slots[0].time;
        let mut x = tone(4800, 440.0);
        rack.process(0, &mut x, 48000);
        assert!((time(&rack) - 12000.0).abs() < 1.0, "{}", time(&rack));
        rack.units[0].params.slots[0].value = 1.0;
        let mut last = time(&rack);
        let mut max_step = 0.0f32;
        for _ in 0..100 {
            let mut x = tone(480, 440.0);
            rack.process(0, &mut x, 48000);
            max_step = max_step.max((time(&rack) - last).abs());
            last = time(&rack);
        }
        // After a second it has moved, but far from all the way (0.5 x 0.087 x 48,000 at most).
        assert!(last > 12000.5 && last < 14200.0 && max_step < 480.0 * 0.1, "{last} {max_step}");
        assert!(x.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn off_fades_out_and_the_dry_signal_returns() {
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Trans, 1.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(20000, 440.0);
        rack.process(0, &mut x, 48000);
        rack.units[0].params.slots[0].on = false;
        let dry = tone(4800, 440.0);
        let mut y = dry.clone();
        rack.process(0, &mut y, 48000);
        assert!(y[2000..].iter().zip(&dry[2000..]).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn release_fx_take_over_while_held_and_let_go() {
        for kind in 0..3u8 {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params.assign = [true, false, false, false, false, false];
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            let mut hist = tone(48000, 440.0);
            rack.process(0, &mut hist, 48000);
            rack.units[0].params.release = Some((kind, 1.0));
            let mut x = tone(96000, 440.0);
            rack.process(0, &mut x, 48000);
            let peak = x.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!(peak.is_finite() && peak < 2.0);
            assert!(rms(&x[2000 * 2..6000 * 2]) > 0.05, "release {kind} is heard");
            if kind != 1 {
                // The brake (1 beat = 0.5 s, then its 0.6 s tail) and the back spin (0.67 s)
                // have stopped.
                assert!(rms(&x[56000 * 2..]) < 0.01, "release {kind} ends in silence");
            }
            rack.units[0].params.release = None;
            let dry = tone(4800, 440.0);
            let mut y = dry.clone();
            rack.process(0, &mut y, 48000);
            assert!(y[2000..].iter().zip(&dry[2000..]).all(|(a, b)| (a - b).abs() < 1e-5), "release {kind}: the track is back");
        }
    }

    /// `cargo test --release fx_cost -- --ignored --nocapture`: what the effects cost on the
    /// audio thread, as a share of real time (512-frame buffers at 48 kHz, one channel).
    #[test]
    #[ignore]
    fn fx_cost() {
        let run = |label: &str, set: &dyn Fn(&mut UnitParams)| {
            let mut rack = Rack::new();
            rack.enabled = true;
            let mut p = unit(Kind::Delay, 1.0, 0.7);
            p.slots[0].on = false;
            set(&mut p);
            rack.units[0].params = p;
            rack.units[0].prepare(&p.assign);
            rack.prepare_rev();
            let mut x = tone(48000 * 20, 330.0);
            let t = std::time::Instant::now();
            for chunk in x.chunks_mut(512 * 2) {
                rack.process(0, chunk, 48000);
            }
            let secs = t.elapsed().as_secs_f64();
            println!("fx cost: {label:<34} {:6.2} % of real time ({:.1} us per 512-frame buffer)", secs / 20.0 * 100.0, secs / (48000.0 * 20.0 / 512.0) * 1e6);
        };
        run("assigned, nothing on", &|_| {});
        run("DELAY + ECHO + SPIRAL", &|p| p.slots.iter_mut().for_each(|s| s.on = true));
        for info in INFO.iter() {
            let label = format!("{} alone", info.name);
            run(&label, &|p| p.slots[0] = SlotParams { on: true, level: 0.7, ..SlotParams::new(info.kind) });
        }
        // Single mode with REV held: both cores run (`BeatFxEcho`).
        for kind in [Kind::Echo, Kind::PingPong, Kind::LowCutEcho, Kind::PitchEcho, Kind::BpfEcho, Kind::HpfEcho, Kind::LpfEcho, Kind::CrushEcho] {
            let label = format!("{} REV (single)", info_of(kind).name);
            run(&label, &|p| {
                p.single = true;
                p.one = SlotParams { on: true, level: 0.7, knob: [0.5, 0.5], btn: [true, false], ..SlotParams::new(kind) };
            });
        }
        run("REVERB + PHASER + PITCH ECHO-like", &|p| {
            p.slots = [SlotParams { on: true, ..SlotParams::new(Kind::Reverb) }, SlotParams { on: true, ..SlotParams::new(Kind::Phaser) }, SlotParams { on: true, ..SlotParams::new(Kind::UpEcho) }];
        });
        // The Sound Color FX on one channel, COLOR at 0.85.
        for (name, kind) in crate::cfx::LIST.iter().filter_map(|(n, k)| k.map(|k| (*n, k))) {
            let mut c = crate::cfx::Cfx::new();
            let mut x = tone(48000 * 20, 330.0);
            let t = std::time::Instant::now();
            for chunk in x.chunks_mut(512 * 2) {
                c.process(kind, 0.85, 0.5, chunk, 48000);
            }
            let secs = t.elapsed().as_secs_f64();
            println!("fx cost: CFX {name:<30} {:6.2} % of real time ({:.1} us per 512-frame buffer)", secs / 20.0 * 100.0, secs / (48000.0 * 20.0 / 512.0) * 1e6);
        }
    }

    #[test]
    fn tap_tempo_and_beat_text() {
        let mut t = Tap::new();
        assert_eq!(t.tap(0.0), None);
        let mut bpm = None;
        for i in 1..6 {
            bpm = t.tap(i as f64 * 0.5);
        }
        assert!((bpm.unwrap() - 120.0).abs() < 0.01);
        // A pause of over 1.5 s starts again.
        assert_eq!(t.tap(10.0), None);
        assert!((t.tap(10.4).unwrap() - 150.0).abs() < 0.01);
        assert_eq!(beat_text(0.75, false), "3/4");
        assert_eq!(beat_text(0.0625, false), "1/16");
        assert_eq!(beat_text(4.0, false), "4");
        assert_eq!(beat_text(50.0, true), "50%");
        assert_eq!(beat_table(Kind::Filter).0[beat_table(Kind::Filter).1], 4.0);
    }

    #[test]
    fn vinyl_brake_release_state_follows_the_knob() {
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::VinylBrake, 1.0, 0.8);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let run = |rack: &mut Rack, secs: f32| {
            let mut x = tone((48000.0 * secs) as usize, 330.0);
            for c in x.chunks_mut(1024) {
                rack.process(0, c, 48000);
            }
            x
        };
        run(&mut rack, 0.3);
        let st = |rack: &Rack| { let s = &rack.units[0].channels[0].as_ref().unwrap().slots[0]; (s.vb_state, s.speed) };
        let (state, sp) = st(&rack);
        assert!(state == 1 && sp < 0.9, "braking: {state} {sp}");
        // Back by more than 1/32: the release state, speed taken from the knob's line.
        rack.units[0].params.slots[0].level = 0.4;
        run(&mut rack, 0.05);
        let (state, sp_mid) = st(&rack);
        assert_eq!(state, 2);
        rack.units[0].params.slots[0].level = 0.011;
        run(&mut rack, 0.05);
        let (_, sp_low) = st(&rack);
        assert!(sp_low > sp_mid && sp_low > 0.9, "the speed comes back as the knob returns: {sp_mid} -> {sp_low}");
        // Under 1/128: idle, the dry sound.
        rack.units[0].params.slots[0].level = 0.0;
        run(&mut rack, 0.05);
        assert_eq!(st(&rack).0, 0);
    }

    #[test]
    fn traced_laws_of_the_fxdsp_pass() {
        use crate::fx_core::{EnigmaJet, Mobius};
        // ENIGMA JET: LEVEL/DEPTH (10-bit) to the feedback pair and the wet share.
        assert_eq!(EnigmaJet::level_law(9), (0.0, 0.0, 0.0));
        let (a, b, w) = EnigmaJet::level_law(511);
        assert!((a - 0.69861).abs() < 1e-4 && (b - 0.89821).abs() < 1e-4 && (w - 0.49900).abs() < 1e-4);
        let (a, b, w) = EnigmaJet::level_law(1023);
        assert!((a - 0.89961).abs() < 1e-4 && (b - 0.98982).abs() < 1e-4 && w == 0.5);
        assert!((EnigmaJet::feedback((0.7, 0.9), 255) - 0.9).abs() < 1e-3 && EnigmaJet::feedback((0.7, 0.9), 0) == 0.0);
        assert_eq!(EnigmaJet::mod_law(128), (0.0, 0.0));
        let (r, d) = EnigmaJet::mod_law(0);
        assert!((r - 4.0).abs() < 1e-6 && (d - 0.25).abs() < 1e-6);
        // MOBIUS: the tone's level and the echo's ceiling and feedback.
        assert_eq!(Mobius::level_law(5), 0.0);
        assert!((Mobius::level_law(511) - 1.0 * 0.211 * 0.99602).abs() < 1e-3);
        assert!((Mobius::level_law(1023) - 4.37 * 0.211).abs() < 1e-4);
        assert_eq!(Mobius::echo_law(0.5), (1.0, 0.8));
        assert!((Mobius::echo_law(1.0).1 - 0.96).abs() < 1e-6);
        // SPIRAL UP / DOWN.
        let (inp, ms, dry, fb, _) = rmx_spiral_law(0.0, false, true, 500.0);
        assert_eq!((inp, ms, dry, fb), (0.0, 100.0, 1.0, 0.8));
        let (inp, ms, _, fb, og) = rmx_spiral_law(1.0, false, true, 500.0);
        assert!(inp == 1.0 && (ms - 5.0).abs() < 0.01 && (fb - 1.5).abs() < 1e-4 && (og - 0.4612).abs() < 0.01, "{ms} {fb} {og}");
        let (_, ms, dry, fb, _) = rmx_spiral_law(1.0, true, true, 500.0);
        assert!((ms - 500.0).abs() < 0.1 && dry.abs() < 0.01 && (fb - 1.2).abs() < 1e-4);
        assert!((rmx_spiral_law(1.0, false, false, 500.0).3 - 0.993).abs() < 1e-4);
        // REVERB UP / DOWN.
        let (fc, q, g, size, rt, gin) = reverb_up_law(255, 127);
        assert!((fc - 5212.0).abs() < 2.0 && (q - 2.0101).abs() < 0.001 && (g - 1.8).abs() < 0.01 && (size - 0.3).abs() < 1e-4 && gin == 0.13, "{fc} {q} {g} {size} {rt}");
        assert_eq!(reverb_up_law(0, 0), (200.0, 0.0101, 1.0, 3.3, 1.0, 0.0));
        assert!((fxrev_time(255, 255) - 50.0).abs() < 0.01);
        let (size, _, _, dry) = reverb_down_law(255, 127);
        assert!((size - 2.0800).abs() < 0.002 && dry.abs() < 1e-3);
        assert_eq!(fxrev_filter(false, 0), 54.0);
        assert!((fxrev_filter(true, 255) - 261.0).abs() < 1.0);
        // MT DELAY's taps.
        assert_eq!(mt_taps(0.5, true, 0.5), [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(mt_taps(1.0, true, 0.5), [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0]);
        assert_eq!(mt_taps(0.5, false, 0.0), [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(mt_taps(0.5, false, 1.0), [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0]);
        // NOISE.
        assert_eq!(noise_gain(3), 0.0);
        assert!((noise_gain(51) - 0.8).abs() < 1e-4 && (noise_gain(255) - 0.0801).abs() < 0.001);
        assert_eq!(noise_fc(0), 220.0);
        assert!((noise_fc(255) - 220.0 * 2f32.powf(5.3544)).abs() < 1.0);
        // The echoes' FEEDBACK and PITCH ECHO's.
        assert_eq!((echo_fb(0.0, false), echo_fb(0.5, false)), (0.0, 0.7));
        assert!((echo_fb(1.0, false) - 0.993).abs() < 1e-4);
        assert_eq!(pitch_echo_fb(0.5, false), 0.85);
        assert!((pitch_echo_fb(1.0, false) - 0.993).abs() < 1e-4);
        // VINYL BRAKE's knobs and brake length; DELAY's pan zones.
        assert_eq!((vinyl_repeat_beats(0.0), vinyl_repeat_beats(0.5), vinyl_repeat_beats(1.0)), (0.0625, 0.5, 4.0));
        assert_eq!((vinyl_echo_beats(0.5), vinyl_echo_beats(1.0)), (None, Some(2.0)));
        assert_eq!((vinyl_brake_beats(0.1), vinyl_brake_beats(1.0)), (4.0, 0.125));
        // Single-mode knobs.
        let pr = |k: f32| { let p = crate::fx_lfo::PanPar::single(1.0, k, 0.5, 250.0); (p.free, p.free_ms) };
        assert_eq!((pr(0.0), pr(0.5), pr(1.0)), (([true, false], [62.5, 0.0]), ([false, false], [0.0, 0.0]), ([false, true], [0.0, 62.5])));
        assert_eq!((filter_single_reso(0.0), filter_single_reso(0.5)), (0.0, 0.72));
        assert!((filter_single_reso(1.0) - 0.96).abs() < 1e-6);
        assert_eq!((flanger_fb(0.5, false), flanger_fb(0.5, true)), (0.26667, -0.8));
        assert!((flanger_fb(1.0, false) - 0.93).abs() < 1e-5 && (flanger_fb(1.0, true) + 0.95).abs() < 1e-5);
        assert!((flanger_gain(1.0, false) - (3.6603 - 3.3254 * 0.93)).abs() < 1e-4);
        assert_eq!((reverb_room(0.0), reverb_room(0.5), reverb_room(1.0)), (1, 50, 100));
        assert_eq!((helix_time(0.0, 500.0), helix_time(0.5, 500.0), helix_time(1.0, 500.0)), (2000.0, 500.0, 125.0));
        // The centre-detent readings.
        assert_eq!((half_value(0.5), through_value(0.5), recommend_value(0.5)), (-1.0, 0.0, 0.5));
        assert_eq!((through_value(0.0), recommend_value(1.0)), (1.0, 1.0));
    }

    #[test]
    fn traced_laws() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        // TRANS / PAN, FLANGER / PHASER, FILTER, ROLL mixes.
        assert_eq!((half_mix(0.25), half_mix(0.5), half_mix(1.0)), ((0.5, 0.5), (0.0, 1.0), (0.0, 1.0)));
        assert_eq!((quarter_mix(0.1), quarter_mix(0.25), quarter_mix(1.0)), ((0.9, 0.3), (0.75, 0.75), (0.75, 0.75)));
        assert!(near(mod_fb(0.25), 0.0) && near(mod_fb(1.0), 0.8) && mod_fb(0.1) == 0.0);
        assert!(near(trans_duty(0.3), 0.5) && near(trans_duty(1.0), 0.0625) && near(trans_duty(0.75), 0.28125));
        assert!(near(filter_mix(0.0).0, 1.0) && filter_mix(0.2) == (0.0, 1.0) && near(filter_mix(0.078125).0, 0.5));
        assert!(near(filter_reso(1.0), 0.96) && near(filter_reso(0.5), 0.72) && filter_reso(0.2) < 0.001);
        assert!(roll_mix(0.0) == (1.0, 0.0) && roll_mix(0.6) == (0.0, 1.0) && near(roll_mix(0.25).0, 0.748039) && near(roll_mix(0.25).1, 0.748039));
        assert!(near(roll_mix(0.03125).0, 0.996063) && near(roll_mix(0.46875).0, 0.1142));
        // UP ECHO: nothing bent up to the centre, 16.667 % a repeat fully up; the percent law.
        assert!(up_echo(0.25) == (0.35, 0.0) && near(up_echo(1.0).1, 16.666) && up_echo(1.0).0 == 0.7);
        assert!(near(pitch_ratio(-100.0), 0.5) && near(pitch_ratio(100.0), 2.0) && near(pitch_ratio(200.0), 4.0) && near(pitch_ratio(-50.0), 0.75) && pitch_ratio(0.0) == 1.0);
        // SPIRAL: feedback 0.8 over the middle, past 1 fully up while on, under 1 once off.
        assert!(near(spiral_law(0.05, true).1, 0.4) && near(spiral_law(0.3, true).1, 0.8) && near(spiral_law(0.75, true).1, 1.0) && near(spiral_law(1.0, true).1, 1.2));
        assert!(near(spiral_law(1.0, false).1, 0.993) && near(spiral_law(0.75, false).1, 0.8965) && near(spiral_law(1.0, true).2, 0.499) && near(spiral_law(0.6, true).0, 0.7998));
        // HELIX: through at 0, the loop held and the input shut fully up.
        assert!(helix_law(0.0) == (0.0, 1.0) && near(helix_law(0.5).0, 0.85) && near(helix_law(0.5).1, 0.45) && near(helix_law(1.0).0, 1.0) && near(helix_law(1.0).1, 0.0));
        assert!(robot_law(0.0) == (1.0, 0.0, 0.0) && robot_law(0.5).0 == 0.75 && near(robot_law(1.0).2, 0.8));
        // REVERB: the size and input gain laws, and the decay time at a few values (rekordbox's
        // tables give 765 ms at 1 %, 9.08 s at 50 %, 249.6 s at 100 %).
        assert!(near(reverb_size(50.0), 1.0) && near(reverb_size(1.0), 0.3826) && near(reverb_size(100.0), 2.0805));
        assert!(near(reverb_in_gain(55.0), 0.2) && near(reverb_in_gain(1.0), 0.0072) && near(reverb_in_gain(100.0), 0.0826));
        for (p, want) in [(1.0, 765.0), (22.0, 1329.0), (50.0, 9080.0), (78.0, 69970.0), (100.0, 249600.0)] {
            assert!((reverb_rt60_ms(p) / want - 1.0).abs() < 0.05, "{p}: {}", reverb_rt60_ms(p));
        }
        // The brake: a straight line to 1/8 speed, then 0.20559 a second.
        let mut sp = 1.0f32;
        for _ in 0..24000 {
            sp = brake_step(sp, 1.0, 48000.0);
        }
        assert!(near(sp, 0.5));
        for _ in 0..24000 {
            sp = brake_step(sp, 1.0, 48000.0);
        }
        assert!(near(sp, 0.125 - 0.20559 * 0.125), "{sp}");
        // The FX+ filters' ends and mixes.
        assert!(near(fxplus_fc(Kind::LpfEcho, 4.0), 20000.0) && (fxplus_fc(Kind::LpfEcho, 255.0) - 96.0).abs() < 2.0);
        assert!((fxplus_fc(Kind::HpfEcho, 255.0) - 9000.0).abs() < 60.0 && (fxplus_fc(Kind::BpfEcho, 255.0) - 8700.0).abs() < 60.0 && (fxplus_fc(Kind::CrushEcho, 255.0) - 4000.0).abs() < 30.0);
        assert!(fxplus_mix(Kind::LpfEcho, 0.0) == (1.0, 0.0) && fxplus_mix(Kind::LpfEcho, 100.0) == (0.0, 1.0) && near(fxplus_mix(Kind::LpfEcho, 255.0).1, 0.5));
        assert!(fxplus_mix(Kind::BpfEcho, 0.0) == (1.0, 0.0) && fxplus_mix(Kind::BpfEcho, 255.0).0 == 1.0 && near(fxplus_mix(Kind::HpfEcho, 255.0).0, 0.0));
        // The filter tables' ends.
        assert!((knots(&LFO_LPF_MAIN, 0.0) - 23718.6).abs() < 1.0 && (knots(&LFO_LPF_MAIN, 1.0) - 485.0).abs() < 0.1 && (knots(&LFO_HPF_MAIN, 0.5) - 969.9).abs() < 0.1);
    }

    fn burst(n: usize) -> Vec<f32> {
        (0..n).flat_map(|i| { let v = if i < 480 { 0.5 } else { 0.0 }; [v, v] }).collect()
    }

    #[test]
    fn robot_level_mixes_the_dry_signal_and_modulation_controls_the_single_mode_voices() {
        let input = tone(8_192, 873.0);
        let render = |single, level, modulation| {
            let mut rack = Rack::new();
            rack.enabled = true;
            let mut p = unit(Kind::Robot, 0.0, level);
            p.single = single;
            p.slots[0].knob = [0.5, modulation];
            p.one = p.slots[0];
            rack.units[0].params = p;
            rack.units[0].prepare(&p.assign);
            let mut out = input.clone();
            for block in out.chunks_mut(2 * 257) {
                rack.process(0, block, 48_000);
            }
            let slot = if single { 3 } else { 0 };
            let gains = rack.units[0].channels[0].as_ref().unwrap().slots[slot].ga;
            (out, gains)
        };

        let (dry, _) = render(true, 0.0, 1.0);
        assert!(dry == input, "zero LEVEL/DEPTH must preserve the dry signal");
        let (half, _) = render(true, 0.25, 1.0);
        let (wet, _) = render(true, 0.5, 1.0);
        assert!(half.iter().zip(&dry).zip(&wet).all(|((&a, &d), &w)| (a - (d + w) * 0.5).abs() < 1e-5),
            "25% LEVEL/DEPTH should mix equal amounts of dry and wet audio");
        let (fully_wet, gains) = render(true, 1.0, 1.0);
        assert!(wet == fully_wet, "LEVEL/DEPTH should remain fully wet from the midpoint");
        assert_eq!(gains, [0.75, 1.0]);
        let (unmodulated, gains) = render(true, 1.0, 0.0);
        assert_eq!(gains, [1.0, 0.0]);
        assert!(unmodulated.iter().zip(&wet).any(|(&a, &b)| (a - b).abs() > 0.05));

        // Multi mode gets both voice gains from LEVEL/DEPTH and has no MODULATION knob.
        let (multi, gains) = render(false, 0.05, 0.0);
        let (pitch, flanger, _) = robot_law(0.05);
        assert_eq!(gains, [pitch, flanger]);
        assert!(render(false, 0.05, 1.0).0 == multi, "multi mode should ignore the single-mode knob");
    }

    #[test]
    fn ping_pong_up_echo_and_low_cut_echo() {
        let run = |kind: Kind, level: f32, x: &mut Vec<f32>| {
            let mut rack = Rack::new();
            rack.enabled = true;
            rack.units[0].params = unit(kind, 1.0, level);
            let a = rack.units[0].params.assign;
            rack.units[0].prepare(&a);
            rack.process(0, x, 48000);
        };
        // PING PONG, 1 beat: the left repeat a beat later, the right one at three quarters.
        let mut x = burst(48000 * 2);
        run(Kind::PingPong, 0.75, &mut x);
        assert!(x[24200 * 2] > 0.3 && x[24200 * 2 + 1].abs() < 0.01 && x[18200 * 2 + 1] > 0.3 && x[18200 * 2].abs() < 0.01);
        // Its second repeats at 0.7.
        assert!((x[48200 * 2] / x[24200 * 2] - 0.7).abs() < 0.03 && (x[36200 * 2 + 1] / x[18200 * 2 + 1] - 0.7).abs() < 0.03);
        // UP ECHO at the centre: repeats at 0.85 of each other, at 0.7 of the sound, unbent.
        let mut x = burst(48000 * 2);
        run(Kind::UpEcho, 0.5, &mut x);
        assert!((x[24200 * 2] - 0.35).abs() < 0.02 && (x[48200 * 2] / x[24200 * 2] - 0.85).abs() < 0.03);
        // LOW CUT ECHO: a 100 Hz tone is not repeated, a 2 kHz one is.
        for (freq, repeated) in [(100.0, false), (2000.0, true)] {
            let mut x: Vec<f32> = tone(12000, freq);
            x.extend(std::iter::repeat(0.0).take(60000 * 2));
            run(Kind::LowCutEcho, 0.75, &mut x);
            let echo = rms(&x[26000 * 2..34000 * 2]);
            assert_eq!(echo > 0.1, repeated, "{freq} Hz: {echo}");
        }
    }

    #[test]
    fn low_cut_echo_swing_shortens_only_the_selected_stereo_side() {
        let rate = 48_000;
        let beat = 24_000; // One beat at 120 BPM.
        let click = 5000;
        for (swing, left, right, block) in [
            (0.0, 18_000, beat, 257),
            (0.243135, 21_000, beat, 192),
            (0.49, beat, beat, 128),
            (0.5, beat, beat, 960),
            (0.51, beat, beat, 192),
            (1.0, beat, 18_000, 257),
        ] {
            let mut rack = Rack::new();
            rack.enabled = true;
            let mut p = unit(Kind::LowCutEcho, 1.0, 1.0);
            p.single = true;
            p.one = SlotParams { knob: [0.5, swing], ..p.slots[0] };
            rack.units[0].params = p;
            rack.units[0].prepare(&p.assign);

            let mut audio = vec![0.0; 2 * (click + beat + 2)];
            audio[2 * click..2 * click + 2].fill(0.5);
            for chunk in audio.chunks_mut(2 * block) {
                rack.process(0, chunk, rate);
            }
            for (side, delay) in [left, right].into_iter().enumerate() {
                let repeat = audio.chunks_exact(2).enumerate().skip(click + 1)
                    .find(|(_, frame)| frame[side].abs() > 0.001)
                    .map(|(i, _)| i);
                assert_eq!(repeat, Some(click + delay), "swing {swing}, channel {side}");
            }
        }
    }

    #[test]
    fn reverb_decays_at_its_traced_rate() {
        // REVERB at 25 %: a decay time of about 1.5 s, so half a second later the tail is
        // about 20 dB down, and it has not died within the first second. (The burst comes
        // after the core's 128.625 ms stand-by from its reset at the first turn-on.)
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Reverb, 25.0, 1.0);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x: Vec<f32> = (0..48000 * 3).flat_map(|i| { let v = if (7200..9600).contains(&i) { ((i * 7919 % 1000) as f32 / 500.0 - 1.0) * 0.5 } else { 0.0 }; [v, v] }).collect();
        for chunk in x.chunks_mut(512 * 2) {
            rack.process(0, chunk, 48000);
        }
        let db = |a: usize| 20.0 * rms(&x[a * 2..(a + 9600) * 2]).log10();
        let fall = db(19200) - db(43200);
        assert!(db(19200) > -60.0, "{}", db(19200));
        assert!((fall - 20.0).abs() < 7.0, "fell {fall} dB in 0.5 s");
    }

    #[test]
    fn beat_fx_filter_sweeps_down_to_485_hz() {
        // FILTER, 1 beat, LEVEL/DEPTH at 0.3 (the filter alone, almost no resonance): at the
        // start of a turn the low-pass is wide open, half a beat in it is at 485 Hz, so a
        // 5 kHz tone is passed and then cut by about 60 dB (three first-order sections).
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params = unit(Kind::Filter, 1.0, 0.3);
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut x = tone(24000, 5000.0);
        for chunk in x.chunks_mut(64 * 2) {
            rack.process(0, chunk, 48000);
        }
        let open = rms(&x[300 * 2..900 * 2]);
        let shut = rms(&x[11700 * 2..12300 * 2]);
        assert!(open > 0.25, "{open}");
        assert!(20.0 * (shut / open).log10() < -45.0, "{}", 20.0 * (shut / open).log10());
    }

    #[test]
    fn echo_takes_its_time_from_the_quantizer_restarts() {
        // `SingleBeatFxEcho::setEffectBeatTimeWithPlayHead` stores the quantizer's time: ECHO
        // at 1 beat started on beat 8 at 120 BPM runs 24000 frames; a tempo change to 125 BPM
        // half a beat later leaves it there until the restart on beat 9, which gives
        // round(60 x 48000 / 125) = 23040 frames.
        let mut s = Slot::new();
        let ring = Ring::new();
        let p = SlotParams { on: true, level: 0.5, value: 1.0, ..SlotParams::new(Kind::Echo) };
        let mut beat = 8.0f64;
        let mut bpm = 120.0f32;
        let mut seen = Vec::new();
        for i in 0..100 {
            if i == 25 {
                bpm = 125.0;
            }
            let mut x = vec![0.0f32; 480 * 2];
            s.process(&p, bpm, &mut x, 48000, &ring, Some(beat), true);
            seen.push((beat, s.time, s.dpend));
            beat += 480.0 * bpm as f64 / (60.0 * 48000.0);
        }
        assert!(seen.iter().filter(|v| v.0 < 8.99).all(|v| v.1 == 24000.0), "{:?}", &seen[..5]);
        assert!(seen.iter().filter(|v| v.0 > 9.05).all(|v| v.1 == 23040.0 || v.2 == 23040.0), "{:?}", seen.iter().find(|v| v.0 > 9.05));
    }

    #[test]
    fn echo_time_change_closes_the_gate_and_jumps() {
        // `DelayModule`: a new beat value is not a glide. The read gate closes over 3.3333 ms
        // (159 frames at 48 kHz), the read point jumps, and the gate opens again; the repeats
        // keep their pitch (a steady tone's repeats keep its zero crossings' spacing).
        let mut s = Slot::new();
        let ring = Ring::new();
        let mut p = SlotParams { on: true, level: 1.0, value: 0.5, ..SlotParams::new(Kind::Echo) };
        let tone: Vec<f32> = (0..48000).flat_map(|i| { let v = (i as f32 * 440.0 * TAU / 48000.0).sin() * 0.5; [v, v] }).collect();
        let mut x = tone.clone();
        s.process(&p, 120.0, &mut x, 48000, &ring, None, true);
        assert_eq!(s.time, 12000.0);
        p.value = 1.0;
        let mut y = tone.clone();
        s.process(&p, 120.0, &mut y[..512 * 2], 48000, &ring, None, true);
        assert_eq!(s.time, 24000.0);
        assert!(s.dpend == 0.0 && s.dg[0].cur > 0.9, "the gate opens again within the buffer");
        // A repeat of the tone through the jump: still 440 Hz once the gate is open.
        let mut z = vec![0.0f32; 4800 * 2];
        s.process(&p, 120.0, &mut z, 48000, &ring, None, true);
        let c = z.chunks_exact(2).zip(z.chunks_exact(2).skip(1)).filter(|(a, b)| a[0] <= 0.0 && b[0] > 0.0).count();
        assert!((c as i32 - 44).abs() <= 1, "{c}");
    }

    #[test]
    fn reverb_filters_are_rekordboxs_first_order_sections() {
        // `ReverbCore`: IirLpf1stOrder 14 kHz and IirHpf1stOrder 80 Hz on the input, an
        // IirLpf1stOrder in each loop at 20.5 kHz (48 kHz) / 18 kHz (44.1 kHz; FX+ 21.5 kHz).
        let [lp, hp, lo] = crate::fx_verb::static_filters(48000.0, false);
        assert_eq!(lp, crate::fx_core::iir_1st(14000.0, 48000.0, false));
        assert_eq!(hp, crate::fx_core::iir_1st(80.0, 48000.0, true));
        assert_eq!(lo, crate::fx_core::iir_1st(20500.0, 48000.0, false));
        assert_eq!(crate::fx_verb::static_filters(44100.0, false)[2], crate::fx_core::iir_1st(18000.0, 44100.0, false));
        assert_eq!(crate::fx_verb::static_filters(44100.0, true)[2], crate::fx_core::iir_1st(21500.0, 44100.0, false));
        // A first-order high-pass at 80 Hz: -3 dB there (bilinear, pre-warped).
        let w = std::f64::consts::TAU * 80.0 / 48000.0;
        let (re, im) = (w.cos(), -w.sin());
        let (nr, ni) = (hp[0] as f64 + hp[1] as f64 * re, hp[1] as f64 * im);
        let (dr, di) = (1.0 + hp[2] as f64 * re, hp[2] as f64 * im);
        let g = ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt();
        assert!((20.0 * g.log10() + 3.0103).abs() < 0.01, "{}", 20.0 * g.log10());
    }

    #[test]
    fn flanger_sweeps_as_flanger_core() {
        // Single mode, LFO AMP at the left end: the left side sweeps up to 2.6667 + 25 ms, the
        // right side to 2.6667 ms (`SingleBeatFxFlanger::updateParameter`, KnobPara2Assign 0).
        let ring = Ring::new();
        let mut s = Slot::new();
        let mut p = SlotParams::new(Kind::Flanger);
        p.on = true;
        p.level = 0.5;
        p.knob = [0.5, 0.0];
        let mut x = vec![0.0f32; 512 * 2];
        s.process(&p, 120.0, &mut x, 48000, &ring, None, false);
        let lo = 1000.0 / 48000.0;
        // (The LFOs' ranges, `LFO::setAmplitudeRange`.)
        assert!((s.flo[0].amax - 27.6667).abs() < 1e-3 && (s.flo[0].amin - lo).abs() < 1e-6);
        assert!((s.flo[1].amax - 2.6667).abs() < 1e-4);
        // Multi mode: one frame .. 2.6667 ms on both sides.
        let mut s = Slot::new();
        let mut p = SlotParams::new(Kind::Flanger);
        p.on = true;
        p.level = 1.0;
        p.value = 16.0;
        let mut x = vec![0.0f32; 64 * 2];
        s.process(&p, 120.0, &mut x, 48000, &ring, Some(0.0), true);
        assert!((s.flo[1].amax - 2.6667).abs() < 1e-4 && (s.flo[1].amin - lo).abs() < 1e-6);
        // Switched on, the first restart is shutting the gate; until it is shut the LFOs keep the
        // constructor's 10 ms period (480 frames).
        assert!(s.fclose && s.flo[0].wave.period() == 480);
    }

    #[test]
    fn stretch_band_gains_are_rekordbox_s() {
        // (low into the stretch, low live, high into it, high live).
        assert_eq!(stretch_bands(0.5, false), (1.0, 0.0, 1.0, 0.0));
        assert_eq!(stretch_bands(0.5, true), (0.0, 1.0, 0.0, 1.0));
        assert_eq!(stretch_bands(0.0, false), (0.0, 0.0, 1.0, 0.0));
        assert_eq!(stretch_bands(0.0, true), (0.0, 0.0, 0.0, 1.0));
        assert_eq!(stretch_bands(1.0, false), (1.0, 0.0, 0.0, 0.0));
        assert_eq!(stretch_bands(1.0, true), (0.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn flanger_restarts_through_its_gate() {
        // FLANGER (multi, 1 beat at 120 BPM, no beat grid): switched on, the quantizer gives the
        // beat time (500 ms) and a restart; the output gate shuts over 159 frames, then the
        // period is taken and the LFOs jump to their heads (`BeatFxFlanger::processEffectConcrete`).
        // The next restart comes a beat later: frames 24000 .. 24158 shut the gate, and from
        // 24159 the LFOs run from 0 again.
        let ring = Ring::new();
        let mut s = Slot::new();
        let mut p = SlotParams::new(Kind::Flanger);
        p.on = true;
        p.level = 1.0;
        p.value = 1.0;
        let mut x = vec![0.0f32; 480 * 2];
        let mut done = 0;
        while done + 480 <= 24000 {
            s.process(&p, 120.0, &mut x, 48000, &ring, None, true);
            done += 480;
        }
        assert_eq!(s.flo[0].wave.period(), 24000);
        assert_eq!(s.flo[0].pos, 24000 - 159);
        let mut y = vec![0.0f32; 170 * 2];
        s.process(&p, 120.0, &mut y, 48000, &ring, None, true);
        assert!(!s.fclose && s.flo[0].pos == 11 && s.flo[1].pos == 11, "{}", s.flo[0].pos);
    }

    #[test]
    fn beat_fx_filter_coefficient_tables() {
        // The 129 cutoffs read from the installed rekordbox (skipped without it): HPF_RESO is
        // four times HPF_MAIN; the first and every 8th entry are the knots above.
        if let Some(t) = crate::cfx::fc_tables_now().and_then(|t| t.lfo.as_deref()) {
            assert!((t[0][0] - 23718.6).abs() < 0.1 && (t[0][8] - 16080.8).abs() < 0.1 && (t[2][128] - 8892.1).abs() < 0.1);
            assert!((t[3][64] / t[2][64] - 4.0).abs() < 1e-3);
        }
        // `CoefficientTable`: the cutoff warped twice, b0 kept. LPF_MAIN[0] at 48 kHz: 23718.6 Hz
        // is capped at 0.319 fs = 15312 Hz, warped to 23876 Hz, b0 = t / (1 + t), t = tan(pi x
        // 23876 / 48000).
        let [m, _] = lfo_filter_tables(false, 48000);
        let w = (48000.0f64 / std::f64::consts::PI) * (std::f64::consts::PI * 0.319).tan();
        let tt = (std::f64::consts::PI * w / 48000.0).tan();
        assert!((m[0] as f64 - tt / (1.0 + tt)).abs() < 1e-4, "{} {}", m[0], tt / (1.0 + tt));
        let [h, l] = lfo_filter_tables(true, 48000);
        assert!(h[0] > 0.99 && h[128] < h[0] && l[0] < 0.1);
    }

    #[test]
    fn pad_fx_unit_runs_what_its_pads_hold() {
        let mut rack = Rack::new();
        rack.pads[0] = Some(Box::new(PadChan::new()));
        assert!(rack.has_pad(0) && !rack.active());
        // A held ECHO pad (1/2 beat, LEVEL/DEPTH 30): the rack is in use and the echo is heard.
        rack.pad_set[0].beat = Some(SlotParams { on: true, level: 0.3, value: 0.5, ..SlotParams::new(Kind::Echo) });
        assert!(rack.active());
        let mut x = burst(48000);
        rack.process(0, &mut x, 48000);
        assert!((x[12200 * 2] - 0.5 * 1.3333 * 0.3).abs() < 0.02, "{}", x[12200 * 2]);
        // Let go: the pad's effect fades out and the sound is the dry one again.
        rack.pad_set[0] = PadSet::NONE;
        let dry = tone(9600, 440.0);
        let mut y = dry.clone();
        rack.process(0, &mut y, 48000);
        assert!(y[2000..] == dry[2000..]);
        // A release pad takes the sound over; a colour pad (LPF fully turned) dulls it.
        // (rekordbox's ECHO takes the half beat after the press into the echo, then cuts the
        // track: silence afterwards still sounds the repeats.)
        rack.pad_set[0].release = Some((1, 0.5));
        let mut y = tone(12000, 440.0);
        rack.process(0, &mut y, 48000);
        let mut y = vec![0.0f32; 12000 * 2];
        rack.process(0, &mut y, 48000);
        assert!(rms(&y[2000..]) > 0.1, "the release echo repeats what was playing");
        rack.pad_set[0] = PadSet { color: Some((crate::cfx::LPF, 1.0, 0.5)), ..PadSet::NONE };
        let mut y = tone(48000, 5000.0);
        rack.process(0, &mut y, 48000);
        assert!(rms(&y[40000 * 2..]) < 0.01);
        // Nothing asked for 20 s: the unit rests and the rack is no longer in use.
        rack.pad_set[0] = PadSet::NONE;
        let mut y = tone(48000, 440.0);
        for _ in 0..21 {
            rack.process(0, &mut y, 48000);
        }
        assert!(!rack.active());
    }

    #[test]
    fn release_vinyl_brake_starts_at_the_sound_being_played() {
        // (Found by another track: the brake used to play 15-second-old audio.) The history is
        // a ramp and the sound goes on as the same ramp while the brake is held: its first
        // output frames are the dry signal at the press, falling behind as it slows.
        let ramp = |from: usize, n: usize| -> Vec<f32> { (from..from + n).flat_map(|i| { let v = i as f32 / 100000.0; [v, v] }).collect() };
        let mut rack = Rack::new();
        rack.enabled = true;
        rack.units[0].params.assign = [true, false, false, false, false, false];
        let a = rack.units[0].params.assign;
        rack.units[0].prepare(&a);
        let mut hist = ramp(0, 48000);
        rack.process(0, &mut hist, 48000);
        rack.units[0].params.release = Some((0, 4.0));
        let mut x = ramp(48000, 4800);
        let dry = x.clone();
        rack.process(0, &mut x, 48000);
        // After the 3.33 ms fade-in the output is the dry sound within a frame or two (4 beats
        // at 120 BPM = 2 s: 300 frames in the playback is 0.5 frame behind), weighted by
        // rekordbox's speed curve (`VinylBrakeCore`: 0.42857 + 0.57143 v).
        for f in [200usize, 300, 600] {
            let w = crate::fx_spin::brake_gain(1.0 - (f + 1) as f32 / 96000.0);
            assert!((x[f * 2] / w - dry[f * 2]).abs() < 3.0 / 100000.0, "frame {f}: {} vs {}", x[f * 2] / w, dry[f * 2]);
        }
        // Later it is clearly behind the live sound (slower), never ahead, never the old history.
        let w = crate::fx_spin::brake_gain(1.0 - 4701.0 / 96000.0);
        assert!(x[4700 * 2] / w < dry[4700 * 2] && x[4700 * 2] / w > dry[4000 * 2]);
    }

    // fx7a
    /// Run single-mode `kind` (made through `prepare_rev`, so on its rekordbox cores) over
    /// `x` in 960-frame buffers at 120 BPM, beat value 1 (24000 frames).
    fn run_single(kind: Kind, level: f32, knob: [f32; 2], x: &mut [f32]) {
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = unit(kind, 1.0, level);
        p.single = true;
        p.one = SlotParams { on: true, level, value: 1.0, knob, btn: [false, false], ..SlotParams::new(kind) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        rack.prepare_rev();
        for c in x.chunks_mut(960 * 2) {
            rack.process(0, c, 48000);
        }
    }

    #[test]
    fn single_echo_runs_echo_core_and_keeps_the_dry_sound_until_the_line_is_full() {
        // ECHO at LEVEL/DEPTH 1 (law: dry 0, wet 1), FEEDBACK at its centre (0.7): a 480-frame
        // block of 1.0 at 5000. The dry sound stays whole until the line holds a beat
        // (`SingleBeatFxEcho::calcMixRatioInternal`); the repeats come whole frames of the beat
        // later through `EchoCore`, 0.7 each.
        let mut y: Vec<f32> = (0..72000).flat_map(|t| { let v = if (5000..5480).contains(&t) { 1.0 } else { 0.0 }; [v, v] }).collect();
        run_single(Kind::Echo, 1.0, [0.5, 0.5], &mut y);
        assert!((y[5200 * 2] - 1.0).abs() < 1e-4, "dry whole before the line is full: {}", y[5200 * 2]);
        let first = (20000..40000).find(|&t| y[t * 2].abs() > 1e-6).unwrap();
        assert_eq!(first, 29000, "first repeat a beat (24000 frames) on");
        assert!((y[29200 * 2] - 1.0).abs() < 1e-4, "{}", y[29200 * 2]);
        assert!((y[53200 * 2] - 0.7).abs() < 1e-4, "second repeat at the feedback: {}", y[53200 * 2]);
        assert!(y[(5000 + 24000 + 600) * 2].abs() < 1e-6, "dry gone once the line is full");
    }

    #[test]
    fn low_cut_echo_swing_shortens_one_side() {
        // SWING turned fully left: the left side at 0.75 of the beat (18000 frames), the right at
        // the beat (`SingleLowCutFxEcho::updateParameter`).
        let mut y: Vec<f32> = (0..48000).flat_map(|t| { let v = if (5000..5480).contains(&t) { (t as f32 * 0.25).sin() } else { 0.0 }; [v, v] }).collect();
        run_single(Kind::LowCutEcho, 1.0, [0.5, 0.0], &mut y);
        let l = (15000..40000).find(|&t| y[t * 2].abs() > 1e-3).unwrap();
        let r = (15000..40000).find(|&t| y[t * 2 + 1].abs() > 1e-3).unwrap();
        assert!((23000..23010).contains(&l), "left repeat at {l}");
        assert!((29000..29010).contains(&r), "right repeat at {r}");
    }

    // fx8a
    #[test]
    fn merge_slots_get_every_echo_core_they_can_run() {
        let mut s = Slot::new();
        s.ensure_merge_cores();
        assert!(s.ec.is_some() && s.pe.is_some() && s.rv.is_some());
    }

    #[test]
    fn single_fx_plus_echo_runs_its_rekordbox_body() {
        // HPF ECHO at LEVEL 0.5 (step 127): `FxPlusHpfEcho` keeps g^3 = 0.13259 of the input dry
        // and its filtered sound and echoes carry no DC, so a steady 1.0 settles at g^3.
        let mut y = vec![1.0f32; 2 * 96000];
        run_single(Kind::HpfEcho, 0.5, [0.5, 0.0], &mut y);
        let g = 1.0159f32 - 0.0039841 * 127.0;
        assert!((y[2 * 95000] - g * g * g).abs() < 1e-3, "{}", y[2 * 95000]);
        // BPF ECHO with ECHO MIX at 0.25 (the quarter-beat core alone): the first repeat of a burst
        // comes a quarter beat (6000 frames at 120 BPM) after it.
        let mut y: Vec<f32> = (0..48000).flat_map(|t| { let v = if (5000..5480).contains(&t) { (t as f32 * 0.3).sin() } else { 0.0 }; [v, v] }).collect();
        run_single(Kind::BpfEcho, 0.6, [0.25, 0.0], &mut y);
        let quiet = (5480 + 2000..11000).map(|t| y[t * 2].abs()).fold(0.0f32, f32::max);
        let loud = (11000..11480).map(|t| y[t * 2].abs()).fold(0.0f32, f32::max);
        assert!(loud > 0.05 && quiet < loud * 0.05, "repeat {loud} vs between {quiet}");
    }
}
