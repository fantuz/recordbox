//! The Performance deck's engine rules, after rekordbox 7.2.19 (`checks/re-7.2.19/
//! performance-decks.md` and the functions traced for this file, named where they are used):
//! reverse and slip reverse, scan, vinyl start / stop ramps, jog (touch, scratch, inertia, bend
//! and bend slope), BEAT / BPM SYNC with its half and double rates, loops along the beat grid
//! (halve, double, move, edge adjust, the 4-beat loop), the per-action quantize rules, needle
//! search, the hot cue countdown, the crossfader's curve family, Headroom and A-GAIN.
//!
//! What the audio callback needs per deck is `Engine` (a field of `PlayState`); what the
//! interface keeps per deck is `DeckUi` (a field of `DeckExt`); the settings are `DeckPrefs`
//! (a field of `Prefs`). The functions a controller would call are the `pub fn`s of `impl App`
//! below; each acts on the active deck (`App::with_deck` for another).

use crate::app::*;
use crate::model::*;
use eframe::egui;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------- constants (traced)

/// Length of rekordbox's deck fade (`f2dsp::Attenuator(44100, 1.0 ms)`): 44 steps of a raised
/// cosine. A jump fades out over it, moves, and fades in over it again.
pub const FADE_N: usize = 44;
/// Shortest loop: 6.667 ms (`DjPlayerConfig` +40, used by `CueBehavior::getMinimumLoopLength`).
pub const MIN_LOOP_SECS: f64 = 0.006_666_666_666_666_667;
/// A loop's wrap is smoothed only when the loop is longer than 2 ms (88 frames at 44.1 kHz;
/// `DjPlayerCore::shouldSmoothDiscontinuousPoints`).
pub const LOOP_SMOOTH_SECS: f64 = 88.0 / 44100.0;
/// How far a bend slope can take the bend (`StretchBehavior`'s last constructor argument).
pub const BEND_LIMIT: f64 = 0.42;
/// FF / REW in Performance mode: rekordbox's `SCAN_SPEED_x20`, 133 ms grains.
pub const SCAN_SPEED: f64 = 20.0;
pub const SCAN_GRAIN_SECS: f64 = 0.133;
/// Sync (`DjPlayerConfig::createSlaveStateReqirements`): no sync under 40 BPM, at most double
/// speed, and the half / double thresholds.
pub const SYNC_MIN_BPM: f64 = 40.0;
pub const SYNC_MAX_SPEED: f64 = 2.0;
pub const SYNC_HALF_BELOW: f64 = 2.0 / 3.0;
pub const SYNC_DOUBLE_ABOVE: f64 = 1.5;
/// The platter turns at 33 1/3 or 45 rpm (Preferences > Controller > Deck > JOG).
pub const RPM_33: f64 = 100.0 / 3.0;

/// The fade curve at step `i` (0 = silent .. FADE_N - 1 = full): `Attenuator::createCurve`.
#[inline]
pub fn fade_curve(i: usize) -> f32 {
    let i = i.min(FADE_N - 1) as f32;
    0.5 + 0.5 * (std::f32::consts::PI * i / (FADE_N - 1) as f32 - std::f32::consts::FRAC_PI_2).sin()
}

/// Vinyl Speed Adjust: the time in ms a full speed's worth of start or stop takes at knob
/// step `idx` (0..255). rekordbox's table is 1 ms for the first twelve steps and then rises
/// in 10 ms steps to 5.54 s; this is a line through 18 points of it (five steps differ from
/// the original by one 10 ms step: 44, 98, 100, 120 and 160).
pub fn vinyl_ms(idx: u8) -> f32 {
    const K: [(f32, f32); 18] = [(12.0, 13.0), (26.0, 20.0), (44.0, 39.0), (58.0, 61.0), (63.0, 67.0), (79.0, 110.0), (97.0, 161.0), (106.0, 214.0), (129.0, 340.0), (130.0, 368.0), (141.0, 480.0), (160.0, 686.0), (162.0, 742.0), (191.0, 1372.0), (193.0, 1459.0), (223.0, 2764.0), (225.0, 2938.0), (255.0, 5548.0)];
    let i = idx as f32;
    if idx < 12 {
        return 1.0;
    }
    for w in K.windows(2) {
        if i <= w[1].0 {
            let v = w[0].1 + (w[1].1 - w[0].1) * (i - w[0].0) / (w[1].0 - w[0].0);
            return ((v / 10.0 + 1e-6).floor() * 10.0).max(10.0);
        }
    }
    5540.0
}

// ---------------------------------------------------------------------- engine state

/// What the hand does to the platter.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Jog {
    #[default]
    None,
    /// Touched and held still: the platter brakes to a stop.
    Touch,
    /// Moved at this speed (1.0 = the record's own speed, negative backwards).
    Speed(f64),
}

/// The deck engine's state in `PlayState`, read by the audio callback.
#[derive(Clone, Copy, Debug)]
pub struct Engine {
    pub reverse: bool,
    /// Reverse was started as slip reverse: the track runs on underneath and playback
    /// returns to it when reverse ends (also without the SLIP button).
    pub slip_reverse: bool,
    pub(crate) slip_return: bool,
    pub jog: Jog,
    /// Vinyl Speed Adjust in ms: Release/Start and Touch/Brake.
    pub start_ms: f32,
    pub stop_ms: f32,
    /// Bend slope (change of the bend per ms) and what it has added up to.
    pub bend_slope: f64,
    pub bend_acc: f64,
    /// A quantized jump that waits for its place: at source frame `.0` playback continues
    /// from `.1` (once).
    pub warp: Option<(f64, f64)>,
    /// Length of a search grain in seconds (0 = the Export player's 60 ms).
    pub scan_grain: f64,
    /// BEAT SYNC's rate against the master: 0.5, 1 or 2.
    pub sync_rate: f64,
    /// Crossfader side (0 THRU, 1 A, 2 B; 255 = the plain `fader` gain is used) and the
    /// gains of the two sides.
    pub route: u8,
    pub xf: [f32; 2],
    /// The speed in use (1.0 = the track's own speed), for the interface.
    pub vel: f64,
    /// A loop that starts with the quantized `warp` above (an auto beat loop that lies
    /// behind the playhead is entered on its own lattice, `QuantizedCueBehavior::
    /// doShortAutoBeatLoop`): it becomes `loop_range` when the warp is taken.
    pub arm_loop: Option<(f64, f64)>,
    /// The channel's EQ / ISOLATOR switch as it sounds (rekordbox's `EqIso`).
    pub eqsw: EqSwitch,
    /// DJM-900NXS EQ: the filter steps in use per band (rekordbox's `Equalizer`).
    pub nxs: NxsSteps,
}

impl Default for Engine {
    fn default() -> Self {
        Engine { reverse: false, slip_reverse: false, slip_return: false, jog: Jog::None, start_ms: 1.0, stop_ms: 1.0, bend_slope: 0.0, bend_acc: 0.0, warp: None, scan_grain: 0.0, sync_rate: 1.0, route: 255, xf: [1.0; 2], vel: 0.0, arm_loop: None, eqsw: EqSwitch::default(), nxs: NxsSteps::default() }
    }
}

// ---------------------------------------------------------------------- EQ type switch

/// rekordbox's `EqIso::processBlock`: when the channel's EQ type changes (EQ / ISOLATOR, or
/// the EQ model), the old one fades out over a 3.3 ms `GainSmoother`, the new one is set up
/// from scratch (`initialize`) and fades in over another 3.3 ms.
#[derive(Clone, Copy, Debug)]
pub struct EqSwitch {
    /// The strip that sounds (0 none, 1 NXS2 EQ, 2 isolator, 3 NXS EQ; 255 = not yet set).
    pub mode: u8,
    /// 0 steady, 1 fading out, 2 fading in.
    pub phase: u8,
    pub gain: Ramp,
}

impl Default for EqSwitch {
    fn default() -> Self {
        EqSwitch { mode: 255, phase: 0, gain: Ramp::at(1.0) }
    }
}

/// Length of the EQ type switch's fades (`GainSmoother(rate, 3.3 ms, 1.0)` in `EqIso`).
pub const EQ_SWITCH_SECS: f32 = 0.0033;

impl EqSwitch {
    /// Before a block: the strip to start it with. A first call takes `want` at once.
    pub fn begin(&mut self, want: u8, rate: u32) -> u8 {
        if self.mode == 255 || (self.mode == 0) != (want == 0) {
            // (A strip switched on or off with the mode is not a type change.)
            self.mode = want;
            self.phase = 0;
            self.gain = Ramp::at(1.0);
        } else if self.phase == 0 && want != self.mode {
            self.phase = 1;
            self.gain.set(0.0, EQ_SWITCH_SECS * rate as f32);
        }
        self.mode
    }

    /// One frame's gain; `Some(new mode)` on the frame where the old type has faded out and
    /// the new one takes over (the caller resets that one's filters).
    #[inline]
    pub fn next(&mut self, want: u8, rate: u32) -> (f32, Option<u8>) {
        if self.phase == 0 {
            return (1.0, None);
        }
        let g = self.gain.next();
        if self.gain.idle() {
            if self.phase == 1 {
                self.phase = 2;
                self.mode = want;
                self.gain.set(1.0, EQ_SWITCH_SECS * rate as f32);
                return (g, Some(want));
            }
            self.phase = 0;
        }
        (g, None)
    }
}

// ---------------------------------------------------------------------- DJM-900NXS EQ steps

/// rekordbox's DJM-900NXS `Equalizer` does not follow its knob directly: the knob (rounded to
/// 256 positions) picks one of 488 filter steps through a table (`mp_s_vr_eqtbl`, with a flat
/// zone around the centre), and every 1/3 ms (`sampleRate * 0.000333` frames) the filter moves
/// one step toward it. This keeps the step in use per band (low, mid, high).
#[derive(Clone, Copy, Debug)]
pub struct NxsSteps {
    pub cur: [f32; 3],
    pub ready: bool,
}

impl Default for NxsSteps {
    fn default() -> Self {
        NxsSteps { cur: [0.0; 3], ready: false }
    }
}

/// The last filter step.
pub const NXS_STEPS: u16 = 487;

static NXS_TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();

/// The knob-to-step table, read once from the installed rekordbox 7.2.19 (it is not copied
/// into recordbox); a straight line when that is not there.
pub fn nxs_table() -> &'static [u16; 256] {
    NXS_TABLE.get_or_init(|| read_nxs_table().unwrap_or_else(|| std::array::from_fn(|k| ((k as f32 * NXS_STEPS as f32 / 255.0).round()) as u16)))
}

/// Read `Equalizer::mp_s_vr_eqtbl` from the arm64 slice of the installed binary (7.2.19:
/// slice offset 0x38E17F0, 256 little-endian u16, rising from 0 to 487).
fn read_nxs_table() -> Option<[u16; 256]> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open("/Applications/rekordbox 7/rekordbox.app/Contents/MacOS/rekordbox").ok()?;
    let mut head = [0u8; 8];
    f.read_exact(&mut head).ok()?;
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    // A universal binary: find the arm64 slice (cputype 0x0100000C).
    let base = if be(&head[0..4]) == 0xCAFE_BABE {
        let n = be(&head[4..8]) as usize;
        let mut arch = vec![0u8; 20 * n.min(8)];
        f.read_exact(&mut arch).ok()?;
        (0..n.min(8)).find(|i| be(&arch[i * 20..]) == 0x0100_000C).map(|i| be(&arch[i * 20 + 8..]) as u64)?
    } else {
        0
    };
    f.seek(SeekFrom::Start(base + 0x38E_17F0)).ok()?;
    let mut raw = [0u8; 512];
    f.read_exact(&mut raw).ok()?;
    let t: [u16; 256] = std::array::from_fn(|i| u16::from_le_bytes([raw[2 * i], raw[2 * i + 1]]));
    (t[0] == 0 && t[255] == NXS_STEPS && t.windows(2).all(|w| w[0] <= w[1] && w[1] - w[0] <= 4)).then_some(t)
}

/// The filter step a knob position (0..1) asks for.
pub fn nxs_step_of(knob: f32) -> u16 {
    nxs_table()[(knob.clamp(0.0, 1.0) * 255.0).round() as usize]
}

/// The knob position (0..1) whose gain law gives filter step `step`: the middle of the knob
/// positions that map to it, or a point between its neighbours.
pub fn nxs_knob_of(step: f32) -> f32 {
    let t = nxs_table();
    let s = step.clamp(0.0, NXS_STEPS as f32);
    let i = t.partition_point(|v| (*v as f32) < s);
    if i < 256 && t[i] as f32 == s {
        let j = t.partition_point(|v| (*v as f32) <= s);
        return (i + j - 1) as f32 / 2.0 / 255.0;
    }
    let (a, b) = (i.saturating_sub(1), i.min(255));
    let (va, vb) = (t[a] as f32, t[b] as f32);
    let k = if vb > va { a as f32 + (s - va) / (vb - va) } else { a as f32 };
    k / 255.0
}

impl NxsSteps {
    /// Move the steps toward the knobs for a block of `frames` and give the knob positions
    /// that the equalizer's gain law is to use for it.
    pub fn advance(&mut self, knobs: [f32; 3], frames: usize, rate: u32) -> [f32; 3] {
        let target = knobs.map(|k| nxs_step_of(k) as f32);
        if !self.ready {
            self.cur = target;
            self.ready = true;
        } else {
            let n = (frames as f32 / (rate as f32 * 0.000_333_333_3)).max(0.0);
            for (c, t) in self.cur.iter_mut().zip(target) {
                *c += (t - *c).clamp(-n, n);
                if (t - *c).abs() < 1e-3 {
                    *c = t;
                }
            }
        }
        self.cur.map(nxs_knob_of)
    }
}

/// A gain that reaches its target in a straight line over a fixed number of frames, however
/// far it has to go (rekordbox's `GainSmoother`).
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    pub cur: f32,
    target: f32,
    inc: f32,
    left: u32,
}

impl Ramp {
    pub fn at(v: f32) -> Ramp {
        Ramp { cur: v, target: v, inc: 0.0, left: 0 }
    }
    pub fn idle(&self) -> bool {
        self.left == 0
    }
    /// Head for `target` over `frames` (nothing when it is the target already).
    pub fn set(&mut self, target: f32, frames: f32) {
        if target == self.target {
            return;
        }
        self.target = target;
        self.left = frames.max(1.0) as u32;
        self.inc = (target - self.cur) / self.left as f32;
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.cur = if self.left == 0 { self.target } else { self.cur + self.inc };
        }
        self.cur
    }
}

// ---------------------------------------------------------------------- sync rules

/// The rate a follower takes against the master with multi-BPM sync on
/// (`SlaveDjPlayerStateReqirements::getSyncRate`): half when its own BPM is under 2/3 of the
/// master's, double when it is over 1.5 times, else 1.
pub fn sync_rate_auto(master_bpm: f64, own_bpm: f64) -> f64 {
    if SYNC_HALF_BELOW * master_bpm > own_bpm {
        0.5
    } else if SYNC_DOUBLE_ABOVE * master_bpm < own_bpm {
        2.0
    } else {
        1.0
    }
}

/// The x1/2, x1, x2 button (`getToggleSyncRate`): from x1 to x2 when the deck can play that
/// fast, else to x1/2; from x2 to x1/2; from x1/2 back to x1 when x1 is in range.
pub fn sync_rate_toggle(current: f64, master_bpm: f64, own_bpm: f64) -> f64 {
    if own_bpm <= 0.0 {
        return 1.0;
    }
    if current == 1.0 {
        if 2.0 * master_bpm / own_bpm <= SYNC_MAX_SPEED { 2.0 } else { 0.5 }
    } else if current == 2.0 {
        0.5
    } else if master_bpm / own_bpm <= SYNC_MAX_SPEED {
        1.0
    } else {
        0.5
    }
}

/// The speed a follower plays at, or None when it cannot follow: a BPM under 40
/// (unsyncable) or a speed over 2.0 (out of range, `isBpmWithinRange`).
pub fn sync_speed(master_bpm: f64, own_bpm: f64, rate: f64) -> Option<f64> {
    if master_bpm < SYNC_MIN_BPM || own_bpm < SYNC_MIN_BPM {
        return None;
    }
    let v = master_bpm * rate / own_bpm;
    (v <= SYNC_MAX_SPEED).then_some(v)
}

/// Limited sync rate (`SlaveDjPlayerStateReqirements` +32 / +40, switched by +69): a follower
/// may then play only between half and one and a half times its own speed.
pub const SYNC_LIMITED_MIN: f64 = 0.5;
pub const SYNC_LIMITED_MAX: f64 = 1.5;

/// `sync_speed` with rekordbox's limited sync rate (`isBpmWithinRange`): with `limited` the
/// speed must lie in 0.5 ..= 1.5, otherwise up to 2.0.
pub fn sync_speed_limited(master_bpm: f64, own_bpm: f64, rate: f64, limited: bool) -> Option<f64> {
    if !limited {
        return sync_speed(master_bpm, own_bpm, rate);
    }
    if master_bpm < SYNC_MIN_BPM || own_bpm < SYNC_MIN_BPM {
        return None;
    }
    let v = master_bpm * rate / own_bpm;
    (SYNC_LIMITED_MIN..=SYNC_LIMITED_MAX).contains(&v).then_some(v)
}

/// The x1/2, x1, x2 button with the limited sync rate (`getToggleSyncRate`, the branch on
/// +69): x1 goes to x2 if that is at most 1.5 times the speed, else to x1/2 if that is at
/// least half, else stays; x2 goes to x1/2 if that is in range, else to x1 if that is, else
/// stays; x1/2 goes to x1, unless x1 is out of range, when it is x2 if that fits, else x1/2.
pub fn sync_rate_toggle_limited(current: f64, master_bpm: f64, own_bpm: f64, limited: bool) -> f64 {
    if !limited {
        return sync_rate_toggle(current, master_bpm, own_bpm);
    }
    if own_bpm <= 0.0 {
        return 1.0;
    }
    let r = master_bpm / own_bpm;
    let (lo, hi) = (SYNC_LIMITED_MIN, SYNC_LIMITED_MAX);
    if current == 1.0 {
        if 2.0 * r <= hi {
            2.0
        } else if 0.5 * r >= lo {
            0.5
        } else {
            1.0
        }
    } else if current == 2.0 {
        if 0.5 * r >= lo {
            0.5
        } else if r >= lo && r <= hi {
            1.0
        } else {
            2.0
        }
    } else if r > hi || r < lo {
        if 2.0 * r <= hi { 2.0 } else { 0.5 }
    } else {
        1.0
    }
}

/// Controllers with a motorised platter switch every deck to the limited sync rate while they
/// are connected (`MainComponent::initializeDevice` / `finalizeDevice`, which call
/// `setLimitedSyncRate(true / false)` on each deck when `DeviceDefs::isMotorizedJogDevice`).
/// rekordbox 7.2.19 lists one such device.
pub fn motorized_jog_device(name: &str) -> bool {
    name.to_ascii_uppercase().contains("DDJ-REV7")
}

/// `PlayHeadInfo::isJumpThresholdDeviation`: whether a phase error of `beats` at `bpm` is a
/// deviation that asks the follower to jump. The threshold is in ms; a negative threshold
/// means every error counts. A deck master's play head keeps its constructor's -1.0
/// (`DjSyncMasterBehavior` never sets it), so for decks only the 0.01 beat rule of
/// `shouldJumpForSyncBeat` remains.
pub fn jump_threshold_deviation(beats: f64, bpm: f64, threshold_ms: f64) -> bool {
    threshold_ms < 0.0 || beats.abs() * 60000.0 / bpm.max(1e-9) > threshold_ms
}

/// The jump threshold of a deck master's play head (ms; negative: none).
pub const DECK_MASTER_JUMP_THRESHOLD_MS: f64 = -1.0;

/// How far the follower's beats are from the master's, -0.5 .. 0.5 (positive: ahead), with
/// both positions in beats of their own grids (`getMinJumpSamplesToSyncBeat`). The unit is
/// the follower's beat at rate 1 and 2, the master's at 1/2; the jump that corrects it is
/// `-e * min(rate, 1)` of the follower's beats.
pub fn sync_phase_error(follower_beats: f64, master_beats: f64, rate: f64) -> f64 {
    let d = if rate >= 1.0 { follower_beats - rate * master_beats } else { follower_beats / rate - master_beats };
    (d + 0.5).rem_euclid(1.0) - 0.5
}

/// `StretchBehavior::isScratchableByInertia`: a platter that turns by itself still scratches
/// while it goes against the playing direction or faster than the deck plays.
pub fn scratchable_by_inertia(jog_speed: f64, playing_speed: f64) -> bool {
    if (jog_speed < 0.0) == (playing_speed >= 0.0) {
        return true;
    }
    if playing_speed < 0.0 { playing_speed > jog_speed } else { playing_speed < jog_speed }
}

// ---------------------------------------------------------------------- the beat grid

/// Where `t` is on the grid in beats (whole part: the beat's index, fraction: the way to the
/// next). Before the first and after the last beat the nearest interval is continued.
pub fn grid_pos(beats: &[Beat], t: f64) -> Option<f64> {
    if beats.len() < 2 {
        return None;
    }
    let n = beats.len();
    let i = beats.partition_point(|b| b.time <= t).clamp(1, n - 1);
    let (a, b) = (beats[i - 1].time, beats[i].time);
    Some((i - 1) as f64 + (t - a) / (b - a).max(1e-9))
}

/// The time of grid position `pos` (the inverse of `grid_pos`).
pub fn grid_time(beats: &[Beat], pos: f64) -> Option<f64> {
    if beats.len() < 2 {
        return None;
    }
    let n = beats.len();
    let i = (pos.floor() as i64).clamp(0, n as i64 - 2) as usize;
    let (a, b) = (beats[i].time, beats[i + 1].time);
    Some(a + (pos - i as f64) * (b - a))
}

/// A time as a whole source frame, in seconds (loops are kept on whole frames so that every
/// pass plays the same samples).
pub fn on_frame(t: f64, sr: u32) -> f64 {
    (t * sr as f64).round() / sr.max(1) as f64
}

/// The loop `l` with its length multiplied by `factor` (0.5 halves, 2 doubles), measured in
/// beats along the grid so that it stays on the grid through a tempo change; without a grid,
/// in time. None when the result would be shorter than the minimum loop length, or when a
/// longer loop would not end before the last sample (`CueBehavior::isValidLoopCutRatio`).
pub fn loop_scaled(beats: &[Beat], l: (f64, f64), factor: f64, sr: u32, duration: f64) -> Option<(f64, f64)> {
    let (a, b) = l;
    let end = match (grid_pos(beats, a), grid_pos(beats, b)) {
        (Some(pa), Some(pb)) => grid_time(beats, pa + (pb - pa) * factor)?,
        _ => a + (b - a) * factor,
    };
    let end = on_frame(end, sr);
    if factor < 1.0 && end - a <= MIN_LOOP_SECS {
        return None;
    }
    if factor > 1.0 && (b >= duration - 1.0 / sr.max(1) as f64 || end > duration) {
        return None;
    }
    Some((a, end))
}

/// The loop moved by `n` times its own length, kept inside the track
/// (`CueBehaveDispatcher::doLoopMove`): along the grid when there is one.
pub fn loop_moved(beats: &[Beat], l: (f64, f64), n: i32, sr: u32, duration: f64) -> (f64, f64) {
    let (a, b) = l;
    let (mut na, mut nb) = match (grid_pos(beats, a), grid_pos(beats, b)) {
        (Some(pa), Some(pb)) => {
            let len = pb - pa;
            (grid_time(beats, pa + len * n as f64).unwrap_or(a), grid_time(beats, pb + len * n as f64).unwrap_or(b))
        }
        _ => (a + (b - a) * n as f64, b + (b - a) * n as f64),
    };
    let len = nb - na;
    let last = (duration - 2.0 / sr.max(1) as f64).max(len);
    if na < 0.0 {
        na = 0.0;
        nb = len;
    }
    if nb > last {
        nb = last;
        na = last - len;
    }
    (on_frame(na, sr), on_frame(nb, sr))
}

/// A loop edge moved by `delta` seconds (`CueBehavior::doAdjustLoopPoint`): the in point stays
/// at least the minimum loop length before the out point, the out point inside the track.
pub fn loop_edge(l: (f64, f64), is_in: bool, delta: f64, sr: u32, duration: f64) -> (f64, f64) {
    let (a, b) = l;
    if is_in {
        (on_frame((a + delta).min(b - MIN_LOOP_SECS).max(0.0), sr), b)
    } else {
        (a, on_frame((b + delta).max(a + MIN_LOOP_SECS).min(duration.max(a + MIN_LOOP_SECS)), sr))
    }
}

// ---------------------------------------------------------------------- quantize rules

/// rekordbox's four quantize types (`ControllerQuantizeComponent`): what is snapped with
/// which beat value. Defaults: SNAP 1, HOT CUE 1/4, LOOP 1, REVERSE 1 beat.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum QType {
    Snap = 0,
    HotCue = 1,
    Loop = 2,
    Reverse = 3,
}

/// The beat value of a stored unit (rekordbox's enum: 1 = 1/32, 2 = 1/16 .. 6 = 1 beat).
pub fn unit_beats(unit: u8) -> f64 {
    match unit {
        1 => 1.0 / 32.0,
        2 => 1.0 / 16.0,
        3 => 0.125,
        4 => 0.25,
        5 => 0.5,
        _ => 1.0,
    }
}

/// `DjPlayerQuantizePolicy::thresholdForQuantizeUnit`: the part of a unit `u` (in beats) that
/// still counts as "just passed", from the type's raw threshold `p` (0.5 for SNAP and
/// REVERSE, 1/32 for HOT CUE and LOOP).
pub fn quantize_threshold(u: f64, p: f64) -> f64 {
    if p >= 0.125 {
        u * p
    } else if u > p {
        p
    } else {
        u / 2.0
    }
}

fn raw_threshold(kind: QType) -> f64 {
    match kind {
        QType::Snap | QType::Reverse => 0.5,
        QType::HotCue | QType::Loop => 0.03125,
    }
}

/// The grid point an action at `t` belongs to (`FormattedBeatGrid::goodPlaceSample`): the
/// first point of the `unit` lattice that is strictly later than `t` less the threshold. For
/// SNAP that is the nearest point (a tie goes to the later one); for HOT CUE and LOOP it is
/// the next point, unless one was passed by less than 1/32 beat. Reversed, the threshold is
/// `unit - threshold`.
pub fn quantize_point(beats: &[Beat], t: f64, kind: QType, unit: f64, reversed: bool) -> f64 {
    let Some(p) = grid_pos(beats, t) else { return t };
    let u = unit.max(1.0 / 64.0);
    let mut th = quantize_threshold(u, raw_threshold(kind));
    if reversed {
        th = u - th;
    }
    let k = ((p - th) / u + 1e-9).floor() + 1.0;
    let mut q = k * u;
    while grid_time(beats, q).map(|x| x < 0.0).unwrap_or(false) {
        q += u;
    }
    grid_time(beats, q).unwrap_or(t)
}

// ---------------------------------------------------------------------- mixer laws

/// Where a crossfader curve leaves full level (step 0..1023) for CROSSFADER Curve setting
/// `bank` (0 = smoothest, 16 = the default, 32 = the sharpest cut). From rekordbox's family of
/// 33 curves (`CROSSFADER_CURVE_OF_2CH_MIXER`): each is full level up to this knee and then a
/// straight line down to silence at the far end.
pub fn crossfader_knee(bank: u8) -> f32 {
    // (Ties round to the even step, as rekordbox's table shows at 736.)
    let even = |x: f32| {
        let r = x.round();
        if (x - x.floor() - 0.5).abs() < 1e-4 && r % 2.0 != 0.0 { r - 1.0 } else { r }
    };
    match bank.min(32) {
        16 => 450.63,
        b if b < 16 => even(28.12 * b as f32),
        b => even(450.0 + 35.8125 * (b - 16) as f32),
    }
}

/// Crossfader position (0 = side A's end .. 1 = side B's) to the gains of sides A and B.
pub fn crossfader_curve(pos: f32, bank: u8) -> (f32, f32) {
    let i = (pos.clamp(0.0, 1.0) * 1023.0).round();
    let knee = crossfader_knee(bank);
    let g = |i: f32| if i >= 1023.0 { 0.0 } else { ((1023.0 - i) / (1023.0 - knee).max(1e-6)).min(1.0) };
    (g(i), g(1023.0 - i))
}

/// Headroom labels of the internal mixer, by stored value 1..8.
pub const HEADROOM_LABELS: [&str; 8] = ["NONE", "-3dB", "-6dB", "-9dB", "-12dB", "-15dB", "-18dB", "-21dB"];

/// The gain the Headroom setting (1..8, default 4 = -9 dB) puts on every channel's trim.
/// rekordbox hands the mixer `9 - 3 * (value - 1)` dB (`MainComponent::changeListenerCallback`
/// to `DjMixerUnit::adjustGain`) on top of a trim table whose centre is -8.79 dB, so NONE is
/// 0.21 dB over unity at the centre and each step takes 3 dB off.
pub fn headroom_gain(level: u8) -> f32 {
    let l = if (1..=8).contains(&level) { level } else { 4 };
    10f32.powf((9.0 - 3.0 * (l - 1) as f32 - 8.79) / 20.0)
}

/// What the A-GAIN knob shows for a position (rekordbox: `20 log10(trim table) + 9` dB).
pub fn again_db(pos: f32) -> f32 {
    crate::mixer::trim_db(pos) + 0.21
}

/// The A-GAIN knob's position for a gain in dB (the inverse of `again_db`, by bisection).
pub fn again_pos(db: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if again_db(mid) < db { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi)
}

// ---------------------------------------------------------------------- settings and state

/// The deck settings of Preferences > Controller (rekordbox's names and defaults).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct DeckPrefs {
    /// Needle Lock: the overview does not take clicks while the deck plays.
    pub needle_lock: bool,
    /// Vinyl Speed Adjust, knob steps 0..255: Touch/Brake and Release/Start, each for decks
    /// 1, 3 and for decks 2, 4.
    pub vinyl_brake: [u8; 2],
    pub vinyl_start: [u8; 2],
    /// JOG Rotation Speed: 45 rpm instead of 33.
    pub jog_45: bool,
    /// Sync Type: BEAT SYNC (tempo and beat position) or BPM SYNC (tempo only).
    pub beat_sync: bool,
    /// Follow at half or double the master's tempo when that is nearer.
    pub multi_bpm_sync: bool,
    /// Show the x1/2, x1, x2 button.
    pub show_sync_rate: bool,
    /// Quantize: the Q button sets all decks (and the sampler) at once.
    pub quantize_one_go: bool,
    /// Beat values of SNAP, HOT CUE, LOOP and REVERSE (2 = 1/16 .. 6 = 1 beat).
    pub quantize_units: [u8; 4],
    /// CROSSFADER Curve 0..32.
    pub crossfader_curve: u8,
    /// Headroom 1..8 (NONE, -3 dB .. -21 dB).
    pub headroom: u8,
    /// EQ Type DJM-900NXS instead of DJM-900NXS2.
    pub eq_nxs: bool,
    /// Hot cue countdown on the platter.
    pub hot_cue_countdown: bool,
    /// Limited sync rate (0.5 .. 1.5 times the own speed) without a motorised controller.
    /// rekordbox has no setting for it (only such a controller switches it on); recordbox
    /// keeps the switch here, off by default, so the rule can be used and tested.
    pub limited_sync: bool,
    /// Per deck, as rekordbox keeps them (`Player%d_Quantize`, `_Sync`, `_TimeMode`,
    /// `_DvsMode`).
    pub player_quantize: [bool; 4],
    pub player_sync: [bool; 4],
    pub player_time_mode: [u8; 4],
    pub player_dvs_mode: [u32; 4],
}

impl Default for DeckPrefs {
    fn default() -> Self {
        DeckPrefs { needle_lock: true, vinyl_brake: [0; 2], vinyl_start: [0; 2], jog_45: false, beat_sync: true, multi_bpm_sync: true, show_sync_rate: true, quantize_one_go: false, quantize_units: [6, 4, 6, 6], crossfader_curve: 16, headroom: 4, eq_nxs: false, hot_cue_countdown: true, limited_sync: false, player_quantize: [true; 4], player_sync: [false; 4], player_time_mode: [0; 4], player_dvs_mode: [1003; 4] }
    }
}

/// Per-deck state of the interface (a field of `DeckExt`).
#[derive(Default)]
pub struct DeckUi {
    pub reverse: bool,
    pub slip_reverse: bool,
    /// The sync rate picked with the x1/2, x1, x2 button (None: rekordbox's own choice).
    pub sync_rate: Option<f64>,
    /// The rate in use, and whether the follower cannot reach the master's tempo.
    pub rate_now: f64,
    pub out_of_range: bool,
    /// BEAT SYNC was bent or scratched off the beat: only the tempo follows until SYNC is
    /// switched off and on again.
    pub beat_lost: bool,
    /// Jog state as set by the platter or a controller.
    pub jog: Jog,
    /// The platter is held with the pointer: the angle and time of the last reading, and the
    /// speed in use.
    pub platter: Option<(f32, f64, f64)>,
    /// Bend from the jog ring (added to the bend buttons').
    pub jog_bend: f64,
    /// Loop edge adjust with the jog: what is left over under a whole millisecond.
    pub edge_frac: f64,
    /// The TRIM knob of this deck's channel shows A-GAIN.
    pub again: bool,
    /// BPM drag on the platter: pointer y and BPM when it began.
    pub bpm_drag: Option<(f32, f64)>,
    /// The track being loaded was dragged over from another deck's platter: it becomes an
    /// instant double whatever the Instant Doubles setting says.
    pub double_pending: bool,
    /// The deck's tab column shows the GRID panel instead of the pads
    /// (`MemoryCuePanel::setPanelType(2)`).
    pub grid_tab: bool,
}

/// `kPlayerOperateErrorBeforeStart` (status bar text in 7.2.19).
pub const BEFORE_START: &str = "Invalid operation. Cannot set Cue points before 00m00s.";

/// The platter's turn as rekordbox draws it (`VirtualDeckComp::drawCurrentCuePosition`): the
/// fraction of a turn at `secs` and `rpm` (33 1/3 or 45) in degrees, 0 = 3 o'clock, clockwise
/// on screen. The position mark (a 15 degree gap in the outer ring, the panel colour) is
/// centred 44.041 degrees before it, and so is the cue mark (8 degrees, `#EB870F`).
pub fn platter_angle_deg(secs: f64, rpm: f64) -> f32 {
    let turns = (secs * 1000.0) as f32 * (0.000_016_667 * rpm as f32);
    turns.fract() * 360.0 - 44.041
}

/// BPM drag on the platter's BPM (`BpmLabel`: a hidden `juce::Slider` in IncDecButtons style,
/// absolute drag, 250 pixels for the whole range, up is faster): the BPM for a drag of `dy`
/// pixels (positive = down) from `start`, the range being the tempo range around the track's
/// own BPM (`setRangeByTempo(range, 0.02)`), in steps of 0.02.
pub fn bpm_drag_value(start: f64, own: f64, range: f64, dy: f32) -> f64 {
    let (lo, hi) = (own * (1.0 - range), own * (1.0 + range));
    if hi <= lo {
        return start;
    }
    let prop = ((start - lo) / (hi - lo) - dy as f64 / 250.0).clamp(0.0, 1.0);
    let v = lo + prop * (hi - lo);
    (lo + ((v - lo) / 0.02).round() * 0.02).clamp(lo, hi)
}

/// UI play status as rekordbox's `ePlayStatus`.
pub const PAUSED: u8 = 1;
pub const PLAYING: u8 = 2;
pub const CUED: u8 = 3;
pub const CUE_HELD: u8 = 4;

impl App {
    // ------------------------------------------------------------------ transport
    /// 1 paused, 2 playing, 3 standing at the cue point, 4 CUE held (cue sampler).
    pub fn play_status(&self) -> u8 {
        if self.deck.cue_preview {
            CUE_HELD
        } else if self.playing() {
            PLAYING
        } else if self.deck.audio.is_some() && (self.pos() - self.deck.cue_point).abs() < 0.01 {
            CUED
        } else {
            PAUSED
        }
    }

    /// CUP (cue and play): CUE down, CUE up, then PLAY (`UiPlayer::eventCupOn`). A playing
    /// deck starts again from its cue point; a paused one sets the cue where it stands and
    /// plays; one standing at the cue plays from it.
    pub fn cup(&mut self) {
        self.cue_down();
        self.cue_up();
        self.play_pause_synced();
    }

    /// Reverse on / off (`UiPlayer::eventReverse`). `slip`: as slip reverse, where the track
    /// runs on underneath and playback returns to it when reverse ends. With Q on and a
    /// grid the turn waits for the next REVERSE beat value.
    pub fn reverse_toggle(&mut self, slip: bool) {
        if self.deck.audio.is_none() || crate::prefs_bridge::reverse_quantized(self, slip) {
            return;
        }
        let on = !self.deck.x.dk.reverse;
        self.set_reverse(on, on && slip);
    }

    /// Reverse off (the button's release in hold mode).
    pub fn reverse_off(&mut self) {
        self.set_reverse(false, false);
    }

    pub fn reversed(&self) -> bool {
        self.deck.x.dk.reverse
    }

    fn set_reverse(&mut self, on: bool, slip: bool) {
        if self.deck.x.dk.reverse == on {
            return;
        }
        self.deck.x.dk.reverse = on;
        self.deck.x.dk.slip_reverse = slip;
        self.sync_lost();
        let mut s = self.player.state.lock();
        s.dk.reverse = on;
        s.dk.slip_reverse = slip;
    }

    /// FF / REW held (dir +1 / -1) or released (0): rekordbox's scan at 20 times the speed in
    /// 133 ms grains, also on a paused deck (`UiPlayer::eventFFOn`).
    pub fn scan(&mut self, dir: i32) {
        let mut s = self.player.state.lock();
        s.dk.scan_grain = SCAN_GRAIN_SECS;
        s.search = if s.audio.is_some() { dir.signum() as f64 * SCAN_SPEED } else { 0.0 };
    }

    // ------------------------------------------------------------------ jog
    /// The platter is touched (vinyl mode: it brakes to a stop over the Touch/Brake time and
    /// is silent) or let go (the deck starts again over the Release/Start time, or goes back
    /// to the cue point when CUE is held). `DjEngineIF::touchJog`.
    pub fn jog_touch(&mut self, on: bool) {
        if self.deck.audio.is_none() {
            return;
        }
        self.deck.x.dk.jog = if on { Jog::Touch } else { Jog::None };
        if on {
            self.sync_lost();
        }
        self.player.state.lock().dk.jog = self.deck.x.dk.jog;
        // (Controller > Deck > JOG: "After turning on the cue button and spinning the jog, the
        // track returns to the cue point and pause." Off, the deck plays on from the jog.)
        if !on && self.deck.cue_preview && self.prefs.rb.jog_sampler_mode {
            self.cue_up();
        }
    }

    /// The platter moves at `speed` (1.0 = the record's own speed, negative backwards).
    /// `touched`: the hand is on it. A platter that was let go and is slowing down
    /// (`touched` false) stops being a scratch as soon as its speed no longer exceeds the
    /// playing speed in the same direction: the deck then plays on (rekordbox's inertia rule,
    /// `StretchBehavior::isScratchableByInertia`). Returns whether the deck is scratching.
    pub fn jog_speed(&mut self, speed: f64, touched: bool) -> bool {
        if self.deck.audio.is_none() {
            return false;
        }
        let playing = self.playing();
        if !touched && playing {
            let normal = self.deck_speed() * if self.deck.x.dk.reverse { -1.0 } else { 1.0 };
            if !scratchable_by_inertia(speed, normal) {
                self.jog_touch(false);
                return false;
            }
        }
        self.sync_lost();
        self.deck.x.dk.jog = Jog::Speed(speed);
        self.player.state.lock().dk.jog = Jog::Speed(speed);
        true
    }

    /// The jog ring (or the platter with vinyl mode off): a bend of `ratio` on top of the
    /// tempo, as long as it is sent (0.0 ends it). `DjEngineIF::setJogRingSpeed`.
    pub fn jog_bend(&mut self, ratio: f64) {
        let range = 1.0;
        self.deck.x.dk.jog_bend = ratio.clamp(-range, range);
        if ratio != 0.0 {
            self.sync_lost();
        }
        self.push_deck_controls();
    }

    /// Bend slope: the bend changes by `percent_per_ms` every millisecond until it is set
    /// to 0 (`StretchBehavior::doSetBendSlope`). `reset`: start from no bend.
    pub fn bend_slope(&mut self, percent_per_ms: f64, reset: bool) {
        let v = percent_per_ms * 0.01;
        if v != 0.0 {
            self.sync_lost();
        }
        let mut s = self.player.state.lock();
        if reset {
            s.dk.bend_acc = 0.0;
        }
        s.dk.bend_slope = if v.abs() < 1.19e-7 { 0.0 } else { v };
    }

    /// A follower that is bent, scratched or reversed keeps the master's tempo but no longer
    /// its beat position (rekordbox's message says how to get it back).
    fn sync_lost(&mut self) {
        if self.performance() && self.sync_follower() && self.prefs.deck.beat_sync && !self.deck.x.dk.beat_lost && self.playing() {
            self.deck.x.dk.beat_lost = true;
            self.toast("BPM Sync on. Turn Sync off -> on to Beat Sync again.");
            self.push_deck_controls();
        }
    }

    /// Platter revolutions per second.
    pub fn platter_rps(&self) -> f64 {
        if self.prefs.deck.jog_45 { 0.75 } else { RPM_33 / 60.0 }
    }

    // ------------------------------------------------------------------ loops
    /// The loop size box's `<` and `>` (`PlayerComponent::loopDoubleHalvePressed`): one step
    /// in the 16 sizes. A running loop is halved or doubled on the beat grid (refused under
    /// the minimum length or past the end of the track); otherwise only the size changes.
    pub fn loop_step(&mut self, dir: i32) {
        let sizes = crate::player_ext::LOOP_SIZES;
        let cur = sizes.iter().position(|s| (*s - self.prefs.loop_beats).abs() < 1e-9).unwrap_or(8) as i32;
        let next = (cur + dir.signum()).clamp(0, 15);
        if next == cur {
            return;
        }
        if let Some(l) = self.loop_active() {
            let sr = self.deck.audio.as_ref().map(|a| a.sr).unwrap_or(44100);
            let factor = if dir > 0 { 2.0 } else { 0.5 };
            let Some(n) = loop_scaled(&self.deck.beats, l, factor, sr, self.deck.duration) else { return };
            self.player.state.lock().loop_range = Some(n);
            self.deck.last_loop = Some(n);
        }
        self.prefs.loop_beats = sizes[next as usize];
        self.save_prefs();
    }

    /// The 4-beat loop shortcut (`UiPlayer::event4BeatLoop`): only when no loop runs and
    /// the deck is not at its end.
    pub fn loop_4beat(&mut self) {
        if self.deck.audio.is_none() || self.loop_active().is_some() || self.pos() >= self.deck.duration - 0.05 {
            return;
        }
        self.deck.x.loop_adjust = 0;
        self.auto_loop(4.0);
    }

    /// Move the running loop by its own length, back (-1) or on (+1); the playhead keeps its
    /// place in the loop (`UiPlayer::eventLoopMoveLeft` / `Right`).
    pub fn loop_move(&mut self, dir: i32) {
        let Some(l) = self.loop_active() else { return };
        let sr = self.deck.audio.as_ref().map(|a| a.sr).unwrap_or(44100);
        let n = loop_moved(&self.deck.beats, l, dir.signum(), sr, self.deck.duration);
        if n == l {
            return;
        }
        let p = self.pos();
        let frac = ((p - l.0) / (l.1 - l.0).max(1e-9)).clamp(0.0, 1.0);
        self.player.state.lock().loop_range = Some(n);
        self.deck.last_loop = Some(n);
        self.seek(n.0 + frac * (n.1 - n.0));
    }

    /// The jog in loop adjust (IN or OUT lit while a loop runs): the edge moves with the
    /// platter's surface, `revolutions` of it (`UiPlayer::deviceJogValue`: whole milliseconds
    /// of `60000 / rpm` per revolution, the rest is kept for the next reading).
    pub fn loop_edge_jog(&mut self, revolutions: f64) -> bool {
        let edge = self.deck.x.loop_adjust;
        let Some(l) = self.loop_active() else { return false };
        if edge == 0 {
            return false;
        }
        let ms = revolutions / self.platter_rps() * 1000.0 + self.deck.x.dk.edge_frac;
        let whole = ms.trunc();
        self.deck.x.dk.edge_frac = ms - whole;
        if whole != 0.0 {
            let sr = self.deck.audio.as_ref().map(|a| a.sr).unwrap_or(44100);
            let n = loop_edge(l, edge == 1, whole / 1000.0, sr, self.deck.duration);
            self.player.state.lock().loop_range = Some(n);
            self.deck.last_loop = Some(n);
        }
        true
    }

    // ------------------------------------------------------------------ loops with quantize
    /// The deck stands before the start of the track (`getPlayingTimeFloatMs < 0`): CUE,
    /// LOOP IN and SLICER refuse with rekordbox's `kPlayerOperateErrorBeforeStart`. (recordbox
    /// keeps positions at 0 or later, so this is a guard rather than a common case.)
    pub fn before_start(&mut self) -> bool {
        if self.deck.audio.is_some() && self.pos() < 0.0 {
            self.toast_err(BEFORE_START);
            return true;
        }
        false
    }

    /// LOOP IN in Performance (`UiPlayer::eventLoopIn` → `setCurrentCue`): the in point lands
    /// on the SNAP value with Q on. Returns false outside Performance (the caller's rule).
    pub fn perf_loop_in(&mut self) -> bool {
        if !self.performance() || self.deck.audio.is_none() {
            return false;
        }
        if self.before_start() {
            return true;
        }
        let t = self.quantize_action(self.pos(), QType::Snap);
        let sr = self.deck.audio.as_ref().map(|a| a.sr).unwrap_or(44100);
        self.deck.loop_in = Some(on_frame(t, sr));
        true
    }

    /// LOOP OUT in Performance (`setLoopOutPoint`, then `QuantizedCueBehavior::doLoop`): with
    /// Q on the jump back to the in point waits for the next point of the LOOP value's
    /// lattice, so the loop ends there; one passed by less than 1/32 beat counts as now (the
    /// playhead then lands as far past the in point).
    pub fn perf_loop_out(&mut self) -> bool {
        if !self.performance() || self.deck.audio.is_none() {
            return false;
        }
        let Some(a) = self.deck.loop_in else { return true };
        let sr = self.deck.audio.as_ref().map(|x| x.sr).unwrap_or(44100);
        let p = self.pos();
        let b = if self.prefs.quantize && self.deck.beats.len() >= 2 { quantize_point(&self.deck.beats, p, QType::Loop, self.quantize_unit(QType::Loop), self.deck.x.dk.reverse) } else { p };
        let (a, b) = (on_frame(a, sr), on_frame(b, sr));
        if b <= a + MIN_LOOP_SECS {
            return true;
        }
        self.player.state.lock().loop_range = Some((a, b));
        self.deck.last_loop = Some((a, b));
        if b <= p {
            self.seek(a + (p - b));
        }
        true
    }

    /// An auto beat loop in Performance with Q on (`QuantizedCueBehavior::doAutoBeatLoop` and
    /// `updateAutoBeatLoopState`): the in point is the SNAP point nearest the playhead and the
    /// out point that many beats on along the grid. When the loop lies wholly behind a playing
    /// deck (a short loop started late in its beat), the deck enters it on the loop's own
    /// lattice: at the next multiple of its length after the in point, where it warps to the
    /// in point sample exact (`doShortAutoBeatLoop`). Returns false when the caller's rule
    /// applies (outside Performance, Q off or no grid).
    pub fn perf_auto_loop(&mut self, beats: f64) -> bool {
        if !self.performance() || self.deck.audio.is_none() || !self.prefs.quantize || self.deck.beats.len() < 2 {
            return false;
        }
        let sr = self.deck.audio.as_ref().map(|x| x.sr).unwrap_or(44100);
        let p = self.pos();
        let a = on_frame(quantize_point(&self.deck.beats, p, QType::Snap, self.quantize_unit(QType::Snap), false), sr);
        let Some(pa) = grid_pos(&self.deck.beats, a) else { return false };
        let b = on_frame(grid_time(&self.deck.beats, pa + beats).unwrap_or(a + beats * self.beat_len()).max(a + MIN_LOOP_SECS), sr);
        self.deck.last_loop = Some((a, b));
        let playing = self.playing();
        let mut s = self.player.state.lock();
        s.dk.arm_loop = None;
        if !playing || p < b {
            s.loop_range = Some((a, b));
            return true;
        }
        // Behind the playhead: the next lattice point (the loop's own length, LOOP threshold).
        let pp = grid_pos(&self.deck.beats, p).unwrap_or(pa);
        let th = quantize_threshold(beats, raw_threshold(QType::Loop));
        let k = ((pp - pa - th) / beats + 1e-9).floor() + 1.0;
        let point = grid_time(&self.deck.beats, pa + k * beats).unwrap_or(p);
        if point <= p {
            s.loop_range = Some((a, b));
            drop(s);
            self.seek(a + (p - point));
        } else {
            s.loop_range = None;
            s.dk.warp = Some(((point * sr as f64).round(), (a * sr as f64).round()));
            s.dk.arm_loop = Some((a, b));
        }
        true
    }

    // ------------------------------------------------------------------ per-deck settings
    /// Per-deck Q and BEAT SYNC as rekordbox keeps them (`Player%d_Quantize`, `Player%d_Sync`),
    /// put back on the decks once at start.
    pub fn restore_player_settings(&mut self) {
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let q = self.prefs.deck.player_quantize;
        let sync = self.prefs.deck.player_sync;
        if self.prefs.deck.quantize_one_go {
            let any = q.iter().any(|v| *v);
            self.px.parked_quantize = [any; 4];
            self.prefs.quantize = any;
        } else {
            self.px.parked_quantize = q;
            self.prefs.quantize = q[(self.px.active as usize).min(3)];
        }
        for n in 0..4u8 {
            self.deck_of_mut(n).x.beat_sync = sync[n as usize];
        }
        self.push_deck_controls();
    }

    /// The time display's mode of the active deck (`TimeView::setTimeMode(-1)`, stored as
    /// `Player%d_TimeMode`, 0 or 1). In Performance both times are always shown; the mode only
    /// decides whether a time before the start carries its minus sign.
    pub fn time_mode_toggle(&mut self) {
        let n = (self.px.active as usize).min(3);
        self.prefs.deck.player_time_mode[n] = if self.prefs.deck.player_time_mode[n] == 0 { 1 } else { 0 };
        self.save_prefs();
    }

    // ------------------------------------------------------------------ quantize
    /// The beat value of a quantize type, in beats.
    pub fn quantize_unit(&self, kind: QType) -> f64 {
        unit_beats(self.prefs.deck.quantize_units[kind as usize])
    }

    /// Where an action of `kind` at `t` lands with Q on (`t` itself with Q off or no grid).
    pub fn quantize_action(&self, t: f64, kind: QType) -> f64 {
        // (Preferences > Controller > Others: the type's QUANTIZE switch and mode.)
        let Some(k) = crate::prefs_bridge::quantize_kind(self, kind) else { return t };
        if self.prefs.quantize && self.deck.beats.len() >= 2 {
            quantize_point(&self.deck.beats, t, k, self.quantize_unit(kind), self.deck.x.dk.reverse)
        } else {
            t
        }
    }

    /// A hot cue called on a playing deck with Q on (Performance): the jump is made on the
    /// HOT CUE beat value, so that the beat is kept. It happens at once when a lattice point
    /// was passed by less than 1/32 beat, else at the next one, sample exact in the audio
    /// callback. Returns false when the caller should jump plainly.
    pub fn hot_cue_quantized(&mut self, start: f64) -> bool {
        if !(self.performance() && self.prefs.quantize && self.playing()) || self.deck.beats.len() < 2 || self.deck.x.dk.reverse {
            return false;
        }
        let Some(a) = self.deck.audio.clone() else { return false };
        let p = self.pos();
        let exit = self.quantize_action(p, QType::HotCue);
        if exit <= p {
            // Just past the point: land the same distance past the cue.
            self.seek(start + (p - exit));
        } else {
            let mut s = self.player.state.lock();
            s.dk.warp = Some(((exit * a.sr as f64).round(), (start * a.sr as f64).round()));
            s.dk.arm_loop = None;
        }
        true
    }

    /// The Q button: this deck, or with "All Decks" every deck and the sampler
    /// (`MainComponent::setQuantizeInOneGo`).
    pub fn toggle_quantize(&mut self) {
        let on = !self.prefs.quantize;
        self.prefs.quantize = on;
        if self.performance() {
            if self.prefs.deck.quantize_one_go {
                self.px.parked_quantize = [on; 4];
                self.prefs.deck.player_quantize = [on; 4];
                self.prefs.sampler.quantize = on;
            } else {
                self.prefs.deck.player_quantize[(self.px.active as usize).min(3)] = on;
            }
        }
        self.save_prefs();
    }

    // ------------------------------------------------------------------ sync
    /// The x1/2, x1, x2 button of a follower (`DjEngineIF::toggleSyncRate`).
    pub fn sync_rate_toggle(&mut self) {
        if !self.sync_follower() {
            return;
        }
        let (master_bpm, own) = self.sync_bpms();
        let cur = self.deck.x.dk.rate_now.max(0.5);
        let limited = self.limited_sync_rate();
        self.deck.x.dk.sync_rate = Some(sync_rate_toggle_limited(if cur == 0.5 || cur == 2.0 { cur } else { 1.0 }, master_bpm, own, limited));
        self.push_deck_controls();
        self.sync_align();
    }

    /// The label of the sync rate button: "x1/2", "x1" or "x2".
    pub fn sync_rate_label(&self) -> &'static str {
        let r = self.deck.x.dk.rate_now;
        if r == 0.5 { "x1/2" } else if r == 2.0 { "x2" } else { "x1" }
    }

    /// The sync button's caption: BEAT SYNC, or BPM SYNC with that sync type.
    pub fn sync_label(&self) -> &'static str {
        if self.prefs.deck.beat_sync { "BEAT SYNC" } else { "BPM SYNC" }
    }

    /// (the master's tempo as it plays, the active deck's own BPM where it stands)
    fn sync_bpms(&self) -> (f64, f64) {
        let m = self.px.master.min(3);
        let own = self.current_bpm();
        let master = if self.sampler_is_master() {
            self.prefs.sampler.bpm as f64
        } else {
            let d = self.deck_of(m);
            let pos = {
                let s = self.player.decks[m as usize].lock();
                s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0)
            };
            let bpm = if d.beats.is_empty() { d.track.and_then(|id| self.track(id)).map(|t| t.bpm).unwrap_or(0.0) } else { crate::player_ext::bpm_at(&d.beats, pos) };
            bpm * (1.0 + d.x.tempo_adj)
        };
        (master, own)
    }

    /// MASTER moves on by itself when the master deck stops playing and another deck plays
    /// with a grid (rekordbox's master priorities: a deck that plays normally outranks a
    /// master that does not). Called once per frame.
    pub fn master_handover(&mut self) {
        if !self.performance() || self.sampler_is_master() {
            return;
        }
        let count = self.deck_count();
        let plays = |a: &App, n: u8| {
            let s = a.player.decks[n as usize].lock();
            s.playing && s.search == 0.0 && s.dk.jog == Jog::None && !s.dk.reverse && a.deck_of(n).beats.len() >= 2
        };
        if plays(self, self.px.master.min(count - 1)) {
            return;
        }
        if let Some(n) = (0..count).find(|n| plays(self, *n)) {
            self.px.master = n;
        }
    }

    /// The limited sync rate is in force: a motorised-platter controller is connected
    /// (rekordbox's only switch for it), or recordbox's own setting is on.
    pub fn limited_sync_rate(&self) -> bool {
        self.prefs.deck.limited_sync || self.midi.devices.iter().any(|d| motorized_jog_device(d))
    }

    // ------------------------------------------------------------------ tempo
    /// The tempo slider's position, -1 (top, faster... no: slower) .. 1: rekordbox's slider
    /// is `-tempo % / range`, so the top of its travel is slower, as on a player.
    pub fn tempo_slider(&self) -> f64 {
        (-self.deck.x.tempo_adj / self.tempo_range()).clamp(-1.0, 1.0)
    }

    /// Set the tempo from the slider (`VirtualDeckPanel::applyTempoSliderValue`).
    pub fn set_tempo_slider(&mut self, s: f64) {
        if self.sync_follower() {
            self.toast_err("Invalid operation. In Sync mode, Tempo Slider is only available for Master Deck.");
            return;
        }
        self.deck.x.tempo_adj = -s.clamp(-1.0, 1.0) * self.tempo_range();
        self.push_deck_controls();
    }

    /// Drag on the BPM value: `steps` of 0.02 BPM, inside the tempo range (rekordbox's
    /// `BpmLabel::setRangeByTempo(range, 0.02)`).
    pub fn bpm_drag_to(&mut self, bpm: f64) {
        let own = self.current_bpm();
        if own <= 0.0 || self.sync_follower() {
            return;
        }
        let range = self.tempo_range();
        let v = ((bpm / 0.02).round() * 0.02).clamp(own * (1.0 - range), own * (1.0 + range));
        self.deck.x.tempo_adj = v / own - 1.0;
        self.push_deck_controls();
    }

    // ------------------------------------------------------------------ load, needle
    /// Needle search from the overview (`WidgetLoudView::mouseDown` and
    /// `UiPlayer::eventLoudView`): the click is read in 400 steps across the waveform; with
    /// Needle Lock it is refused while the deck plays; a loop the target is outside of ends.
    /// Returns false outside Performance mode (the caller then seeks as before).
    pub fn needle_search(&mut self, frac: f64) -> bool {
        if !self.performance() {
            return false;
        }
        if self.deck.audio.is_none() {
            return true;
        }
        if self.prefs.deck.needle_lock && self.playing() && self.deck.x.dk.jog == Jog::None {
            if !self.toasts.iter().any(|t| t.text.starts_with("Needle Lock")) {
                self.toast_err("Needle Lock. Not available during playback.");
            }
            return true;
        }
        let step = (frac.clamp(0.0, 1.0) * 400.0).floor();
        let t = step * 0.0025 * self.deck.duration;
        if let Some((a, b)) = self.loop_active() {
            if t < a || t > b {
                self.loop_exit();
            }
        }
        self.seek(t);
        true
    }

    /// Instant doubles from deck `src` onto the active deck (the platter dragged from one
    /// deck to another, `DjEngineIF::instantDouble`): the same track at the same position,
    /// tempo and play state.
    pub fn instant_double_from(&mut self, src: u8) {
        let dst = self.px.active;
        if src == dst || src >= self.deck_count() {
            return;
        }
        let Some(id) = self.deck_of(src).track else { return };
        if self.deck.track != Some(id) {
            // The track comes over first; the double is taken when it has been read.
            self.load_track_here(id);
            if self.deck.track == Some(id) {
                self.deck.x.dk.double_pending = true;
            }
            return;
        }
        self.deck.x.dk.double_pending = true;
        self.instant_double();
        self.deck.x.dk.double_pending = false;
    }

    // ------------------------------------------------------------------ hot cue countdown
    /// The platter's countdown to the next hot cue (`UiPlayer::checkHotCueCountDown`): within
    /// five turns of the platter before it, the arc's length in degrees (359 at five turns,
    /// 0 at the cue) and the cue's colour.
    pub fn hot_cue_countdown(&self) -> Option<(f32, egui::Color32)> {
        if !self.prefs.deck.hot_cue_countdown || self.deck.audio.is_none() {
            return None;
        }
        let window = 5.0 / self.platter_rps();
        let p = self.pos();
        let next = self.deck.cues.iter().filter(|c| c.kind == CueKind::Hot && c.start > p && c.start - p <= window).min_by(|a, b| a.start.total_cmp(&b.start))?;
        Some((((next.start - p) / window * 359.0) as f32, crate::theme::hotcue_color(next.color)))
    }

    // ------------------------------------------------------------------ to the audio thread
    /// Hand the deck engine's settings of every deck to the audio thread (from
    /// `push_deck_controls`, which has set the plain tempo and faders).
    pub(crate) fn push_deck_engine(&mut self, own_bpm: [f64; 4], master_bpm: f64) {
        let perf = self.performance();
        let count = self.deck_count();
        let gains = crossfader_curve(self.px.crossfader, crate::automix::curve_override(self).unwrap_or(self.prefs.deck.crossfader_curve));
        let head = headroom_gain(self.prefs.deck.headroom);
        let smp_master = self.sampler_is_master();
        let master = self.px.master.min(count.saturating_sub(1));
        let (multi, beat, eq_nxs) = (self.prefs.deck.multi_bpm_sync, self.prefs.deck.beat_sync, self.prefs.deck.eq_nxs);
        let bpm_only = crate::mpl::bpm_sync_decks(self);
        let limited = self.limited_sync_rate();
        if eq_nxs {
            // (Read here, on the interface thread, before the audio thread needs it.)
            let _ = nxs_table();
        }
        for n in 0..count {
            let own = own_bpm[n as usize];
            let (brake, start) = (self.prefs.deck.vinyl_brake[(n % 2) as usize], self.prefs.deck.vinyl_start[(n % 2) as usize]);
            let assign = self.px.mix[n as usize].assign;
            let d = self.deck_of_mut(n);
            let follower = perf && d.x.beat_sync && (n != master || smp_master) && d.track.is_some() && own > 0.0 && master_bpm > 0.0;
            let mut speed = None;
            let mut went_out = false;
            if follower {
                let rate = if !multi { 1.0 } else { d.x.dk.sync_rate.unwrap_or_else(|| sync_rate_auto(master_bpm, own)) };
                d.x.dk.rate_now = rate;
                speed = sync_speed_limited(master_bpm, own, rate, limited);
                let out = speed.is_none() && master_bpm >= SYNC_MIN_BPM && own >= SYNC_MIN_BPM;
                went_out = out && !d.x.dk.out_of_range;
                d.x.dk.out_of_range = out;
            } else {
                d.x.dk.rate_now = 1.0;
                d.x.dk.out_of_range = false;
                d.x.dk.beat_lost = false;
            }
            let (jog_bend, lost, tempo_adj, rate_now) = (d.x.dk.jog_bend, d.x.dk.beat_lost, d.x.tempo_adj, d.x.dk.rate_now);
            if went_out {
                // (`PlayCuePanel::updatePlayerComponent` picks the message by the limit.)
                self.toast_err(if limited { "Out of range.BPM Sync is not available because Tempo Range exceeds +/-50%." } else { "Out of range. BPM Sync is not available because Tempo Range exceeds 100%." });
            }
            let mut s = self.player.decks[n as usize].lock();
            s.dk.start_ms = vinyl_ms(start);
            s.dk.stop_ms = vinyl_ms(brake);
            if perf {
                s.bend += jog_bend;
                if follower {
                    // The master's tempo at the rate; out of range, the deck's own tempo.
                    s.tempo = speed.unwrap_or(1.0 + tempo_adj);
                    s.sync_follow = speed.is_some() && beat && !lost && !smp_master && !bpm_only[n as usize];
                    s.dk.sync_rate = rate_now;
                }
                s.dk.route = assign.min(2);
                s.dk.xf = [gains.0, gains.1];
                s.trim *= head;
                if s.strip == 1 && eq_nxs {
                    s.strip = 3;
                }
            } else {
                s.dk.route = 255;
            }
        }
    }
}

// ---------------------------------------------------------------------- interface hooks

/// The platter under the pointer (called from `ui_perf::platter_panel` with the platter's
/// centre and outer radius): pressing it is a touch, dragging around the centre scratches,
/// letting go over another deck's area makes that deck an instant double of this one.
/// Draws the hot cue countdown arc.
pub fn platter_hook(app: &mut App, ui: &mut egui::Ui, n: u8, c: egui::Pos2, radius: f32) {
    use egui::{vec2, Rect, Sense, Stroke};
    // Countdown: an arc ahead of the position marker, as long as the way to the next hot cue.
    if let Some((deg, col)) = app.hot_cue_countdown() {
        // (From the position mark on, `drawHotCueCountDown`.)
        let a0 = platter_angle_deg(app.pos(), app.platter_rps() * 60.0).to_radians();
        let steps = (deg / 6.0).ceil().max(1.0) as usize;
        let pts: Vec<egui::Pos2> = (0..=steps).map(|i| a0 + (deg.to_radians() * i as f32 / steps as f32)).map(|a| c + vec2(a.cos(), a.sin()) * (radius - 1.5)).collect();
        ui.painter().add(egui::Shape::line(pts, Stroke::new(2.0_f32, col)));
    }
    if app.deck.audio.is_none() {
        return;
    }
    // The ring between the inner disc (where the BPM and tempo buttons are) and the edge.
    let inner = 0.78 * radius;
    let resp = ui.interact(Rect::from_center_size(c, vec2(radius * 2.0, radius * 2.0)), ui.id().with(("platterjog", n)), Sense::drag());
    let Some(pp) = ui.input(|i| i.pointer.latest_pos()) else { return };
    let now = ui.input(|i| i.time);
    if resp.drag_started() {
        let Some(origin) = ui.input(|i| i.pointer.press_origin()) else { return };
        let dist = (origin - c).length();
        if dist < inner || dist > radius {
            return;
        }
        let v = pp - c;
        app.deck.x.dk.platter = Some((v.y.atan2(v.x), now, 0.0));
        app.jog_touch(true);
    }
    if app.deck.x.dk.platter.is_none() {
        return;
    }
    if resp.dragged() {
        ui.ctx().request_repaint();
        let v = pp - c;
        let (a0, t0, _) = app.deck.x.dk.platter.unwrap_or((0.0, now, 0.0));
        let dt = now - t0;
        // Inside the platter the pointer turns it; outside it the platter is only held (the
        // deck is on its way to another deck).
        if v.length() <= radius * 1.6 && dt > 1e-4 {
            let a = v.y.atan2(v.x);
            let mut da = (a - a0) as f64;
            if da > std::f64::consts::PI {
                da -= std::f64::consts::TAU;
            } else if da < -std::f64::consts::PI {
                da += std::f64::consts::TAU;
            }
            let revs = da / std::f64::consts::TAU;
            if !app.loop_edge_jog(revs) {
                let speed = revs / dt / app.platter_rps();
                if speed == 0.0 {
                    app.jog_touch(true);
                } else {
                    app.jog_speed(speed, true);
                }
                app.deck.x.dk.platter = Some((a, now, speed));
            } else {
                app.deck.x.dk.platter = Some((a, now, 0.0));
            }
        }
    }
    if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
        app.deck.x.dk.platter = None;
        app.jog_touch(false);
        // Let go over another deck: instant doubles there.
        let rects = app.px.sampler.deck_rects;
        if let Some(dst) = (0..app.deck_count()).find(|d| *d != n && rects[*d as usize].contains(pp)) {
            app.with_deck(dst, |a| a.instant_double_from(n));
        }
    }
}

/// Preferences > Controller > Deck: the cards of this file (rekordbox's titles and labels).
pub fn prefs_deck(app: &mut App, ui: &mut egui::Ui) {
    use crate::ui_kit::{card, note, radio, sub, toggle};
    let mut changed = false;
    let p = &mut app.prefs.deck;
    card(ui, "Needle Lock", |ui| {
        for (on, label) in [(true, "Lock"), (false, "Unlock")] {
            if radio(ui, p.needle_lock == on, label) {
                p.needle_lock = on;
                changed = true;
            }
        }
    });
    card(ui, "Vinyl Speed Adjust", |ui| {
        for (title, vals) in [("Touch/Brake", &mut p.vinyl_brake), ("Release/Start", &mut p.vinyl_start)] {
            sub(ui, title);
            for (i, label) in ["Deck1,3", "Deck2,4"].iter().enumerate() {
                let mut t = vals[i] as f32 / 255.0;
                let ms = vinyl_ms(vals[i]);
                crate::ui_kit::line(ui, &format!("{label}: {} ({})", vals[i], if ms >= 1000.0 { format!("{:.2} s", ms / 1000.0) } else { format!("{ms:.0} ms") }));
                if crate::ui_kit::slider(ui, &mut t).changed() {
                    vals[i] = (t * 255.0).round() as u8;
                    changed = true;
                }
            }
        }
    });
    card(ui, "JOG", |ui| {
        sub(ui, "JOG Rotation Speed");
        for (v, label) in [(false, "33rpm"), (true, "45rpm")] {
            if radio(ui, p.jog_45 == v, label) {
                p.jog_45 = v;
                changed = true;
            }
        }
    });
    card(ui, "BEAT/BPM SYNC", |ui| {
        sub(ui, "Sync Type");
        for (v, label) in [(true, "BEAT SYNC"), (false, "BPM SYNC")] {
            if radio(ui, p.beat_sync == v, label) {
                p.beat_sync = v;
                changed = true;
            }
        }
        changed |= toggle(ui, &mut p.multi_bpm_sync, "Half / double tempo sync (x1/2, x2)");
    });
    card(ui, "Quantize", |ui| {
        sub(ui, "Setting");
        for (v, label) in [(false, "Each Deck / Sampler Deck"), (true, "All Decks")] {
            if radio(ui, p.quantize_one_go == v, label) {
                p.quantize_one_go = v;
                changed = true;
            }
        }
        sub(ui, "Beat value");
        for (i, label) in ["SNAP", "HOT CUE", "LOOP/SAMPLER(LOOP)", "REVERSE"].iter().enumerate() {
            crate::ui_kit::labeled(ui, label, |ui| {
                let names = ["1/16", "1/8", "1/4", "1/2", "1/1"];
                let cur = (p.quantize_units[i].clamp(2, 6) - 2) as usize;
                crate::ui_kit::combo(("qunit", i), 90.0, names[cur], 12.0).show_ui(ui, |ui| {
                    for (k, name) in names.iter().enumerate() {
                        if ui.selectable_label(k == cur, *name).clicked() {
                            p.quantize_units[i] = k as u8 + 2;
                            changed = true;
                        }
                    }
                });
            });
        }
        note(ui, "With Q on: cue points are set on the SNAP value, hot cues are called on the HOT CUE value, loops start on the LOOP value and reverse turns on the REVERSE value.");
    });
    if changed {
        if app.prefs.deck.quantize_one_go {
            // "All Decks": every deck takes the Q state in force (`setQuantizeOnIfAny`).
            let any = app.prefs.quantize || app.px.parked_quantize.iter().take(app.deck_count() as usize).any(|q| *q);
            app.prefs.quantize = any;
            app.px.parked_quantize = [any; 4];
        }
        app.poll_decks();
        app.save_prefs();
    }
}

/// Preferences > Controller > Mixer: EQ Type, CROSSFADER Curve and Output Level > Headroom.
pub fn prefs_mixer(app: &mut App, ui: &mut egui::Ui) {
    use crate::ui_kit::{card, radio, sub};
    let mut changed = false;
    let p = &mut app.prefs.deck;
    card(ui, "EQ Type", |ui| {
        for (v, label) in [(true, "DJM-900NXS"), (false, "DJM-900NXS2")] {
            if radio(ui, p.eq_nxs == v, label) {
                p.eq_nxs = v;
                changed = true;
            }
        }
    });
    card(ui, "CROSSFADER Curve", |ui| {
        let mut t = p.crossfader_curve as f32 / 32.0;
        crate::ui_kit::line(ui, &format!("{} (0: the two sides blend over the whole travel; 32: each side is at full level until the far end)", p.crossfader_curve));
        if crate::ui_kit::slider(ui, &mut t).changed() {
            p.crossfader_curve = (t * 32.0).round() as u8;
            changed = true;
        }
    });
    card(ui, "Output Level", |ui| {
        sub(ui, "Headroom");
        let cur = (p.headroom.clamp(1, 8) - 1) as usize;
        crate::ui_kit::combo_row(ui, 22.0, |ui| {
            crate::ui_kit::combo("headroom", 160.0, HEADROOM_LABELS[cur], 12.0).show_ui(ui, |ui| {
                for (k, name) in HEADROOM_LABELS.iter().enumerate() {
                    if ui.selectable_label(k == cur, *name).clicked() {
                        p.headroom = k as u8 + 1;
                        changed = true;
                    }
                }
            });
        });
    });
    if changed {
        app.poll_decks();
        app.save_prefs();
    }
}

/// The sync rate button of a follower, left of the sync button: x1/2, x1 or x2 in
/// rekordbox's yellow; a click steps it (`TrackInfoView::updateSyncRateButtonDrawing`).
pub fn sync_rate_hook(app: &mut App, ui: &mut egui::Ui, n: u8, sync_btn: egui::Rect) {
    use egui::{pos2, vec2, Align2, Rect, Sense};
    if !(app.prefs.deck.multi_bpm_sync && app.prefs.deck.show_sync_rate && app.sync_follower() && app.deck.track.is_some()) {
        return;
    }
    let r = Rect::from_min_size(pos2(sync_btn.min.x - 27.0, sync_btn.max.y - 10.0), vec2(25.0, 10.0));
    let resp = ui.interact(r, ui.id().with(("syncrate", n)), Sense::click());
    let out = app.deck.x.dk.out_of_range;
    ui.painter().rect_filled(r, 1.0, if resp.hovered() { crate::theme::hex(0x323232) } else { crate::theme::hex(0x282828) });
    ui.painter().text(r.center(), Align2::CENTER_CENTER, app.sync_rate_label(), crate::theme::caps(7.5), if out { crate::theme::hex(0x646464) } else { crate::theme::hex(0xFFFF64) });
    if resp.on_hover_text("Sync rate: follow the master at half, the same or double its tempo.").clicked() {
        app.sync_rate_toggle();
    }
}

/// A-GAIN on the TRIM label (`MixerControlComp::buttonClicked`): a click on the label swaps
/// the knob between TRIM and the loaded track's auto gain, shown in rekordbox's orange with
/// the label A-GAIN; turning it sets the track's gain (kept with the track).
pub fn again_hook(app: &mut App, ui: &mut egui::Ui, n: u8, knob_c: egui::Pos2, label_c: egui::Pos2) {
    use egui::{vec2, Align2, Rect, Sense};
    let lr = Rect::from_center_size(label_c, vec2(36.0, 10.0));
    let resp = ui.interact(lr, ui.id().with(("againl", n)), Sense::click());
    if resp.clicked() {
        let d = app.deck_of_mut(n);
        d.x.dk.again = !d.x.dk.again;
    }
    if !app.deck_of(n).x.dk.again {
        return;
    }
    let orange = crate::theme::hex(0xFFA300);
    let p = ui.painter().clone();
    let panel = crate::theme::hex(0x1E1E1E);
    p.rect_filled(Rect::from_center_size(knob_c, vec2(26.0, 26.0)), 0.0, panel);
    p.rect_filled(lr, 0.0, panel);
    p.text(label_c, Align2::CENTER_CENTER, "A-GAIN", crate::theme::caps(8.0), orange);
    let loaded = app.deck_of(n).track.is_some();
    let db = app.with_deck(n, |a| a.gain_db());
    let pos = again_pos(db);
    let new = crate::ui_perf::knob(ui, knob_c, ("again", n), pos, again_pos(0.0), orange);
    if let (Some(v), true) = (new, loaded) {
        let db = again_db(v).max(-crate::player_ext::GAIN_RANGE_DB);
        app.with_deck(n, |a| a.set_gain_db(db, true));
    }
    let _ = resp.on_hover_text(format!("{db:+.1}dB"));
}

/// The scripted run's calls (`perf_selftest.rs`, steps named `DECK:<what>`), on the active deck.
pub fn selftest_call(a: &mut App, what: &str) {
    match what {
        "reverse" => a.reverse_toggle(false),
        "slipreverse" => a.reverse_toggle(true),
        "reverseoff" => a.reverse_off(),
        "syncrate" => a.sync_rate_toggle(),
        "loop4" => a.loop_4beat(),
        "loopmove" => a.loop_move(1),
        "loophalve" => a.loop_step(-1),
        "loopexit" => a.loop_exit(),
        "scan" => a.scan(1),
        "scanoff" => a.scan(0),
        "needle" => {
            a.needle_search(0.5);
        }
        "touch" => a.jog_touch(true),
        "release" => a.jog_touch(false),
        "cup" => a.cup(),
        // (The flag stands for the held button: without it the CUE button's own code lets go.)
        "cuedown" => {
            a.deck.x.wave_cue_held = true;
            a.cue_down();
        }
        "cueup" => {
            a.deck.x.wave_cue_held = false;
            a.cue_up();
        }
        "play" => a.play_pause_synced(),
        // (The rules added after the first deck build.)
        "limited" => {
            a.prefs.deck.limited_sync = !a.prefs.deck.limited_sync;
            a.push_deck_controls();
        }
        "loopin" => a.loop_in(),
        "loopout" => a.loop_out(),
        "autoloop14" => a.auto_loop(0.25),
        "eqiso" => {
            a.prefs.eq_isolator = !a.prefs.eq_isolator;
            a.push_deck_controls();
        }
        "eqnxs" => {
            a.prefs.deck.eq_nxs = !a.prefs.deck.eq_nxs;
            a.push_deck_controls();
        }
        "slicer" => {
            a.set_pad_mode(crate::pads::PadMode::Slicer);
        }
        "hotcue" => {
            a.set_pad_mode(crate::pads::PadMode::HotCue);
        }
        "slip" => a.toggle_slip(),
        "timemode" => a.time_mode_toggle(),
        "spscratch" => a.sample_scratch_pad(0),
        "userbpm" => {
            let it = crate::pads::PadItem { function: "Transport".into(), value: "BPM + MEDIUM".into(), key: None };
            a.user_pad_down(&it, false);
            a.user_pad_up();
        }
        "gridtab" => a.deck.x.dk.grid_tab = !a.deck.x.dk.grid_tab,
        _ => {}
    }
}

/// One line of deck `n`'s engine state for the scripted run.
pub fn state_line(a: &App, n: u8) -> String {
    let d = a.deck_of(n);
    // (Copied out first: nothing is formatted while the audio thread could want the state.)
    struct Snap {
        pos: f64,
        playing: bool,
        dk: Engine,
        loop_range: Option<(f64, f64)>,
        slip_pos: Option<f64>,
        search: f64,
        sync_follow: bool,
    }
    let (s, sr) = {
        let g = a.player.decks[n as usize].lock();
        (Snap { pos: g.pos, playing: g.playing, dk: g.dk, loop_range: g.loop_range, slip_pos: g.slip_pos, search: g.search, sync_follow: g.sync_follow }, g.audio.as_ref().map(|x| x.sr as f64).unwrap_or(1.0))
    };
    let status = if d.cue_preview { CUE_HELD } else if s.playing { PLAYING } else if d.audio.is_some() && (s.pos / sr - d.cue_point).abs() < 0.01 { CUED } else { PAUSED };
    format!(
        "DECK{} status {status} pos {:.2}s cue {:.2}s speed {:+.3} reverse {} (slip {}) jog {:?} loop {:?} ({} beats) slip-bg {:?} scan {} sync {} rate x{} follow {} out-of-range {} beat-lost {} master {} | limited {} eq-strip {} armed-loop {:?} loop-in {:?} SLIP {} grid-tab {} grid-first-beat {:?} grid-undo {} tempo {:+.2}% time-mode {} | last message {:?}",
        n + 1,
        s.pos / sr,
        d.cue_point,
        s.dk.vel,
        s.dk.reverse,
        s.dk.slip_reverse,
        s.dk.jog,
        s.loop_range.map(|l| ((l.0 * 1000.0).round() / 1000.0, (l.1 * 1000.0).round() / 1000.0)),
        a.prefs.loop_beats,
        s.slip_pos.map(|p| (p / sr * 100.0).round() / 100.0),
        s.search,
        d.x.beat_sync,
        d.x.dk.rate_now,
        s.sync_follow,
        d.x.dk.out_of_range,
        d.x.dk.beat_lost,
        a.px.master + 1,
        a.limited_sync_rate(),
        s.dk.eqsw.mode,
        s.dk.arm_loop.map(|l| ((l.0 * 1000.0).round() / 1000.0, (l.1 * 1000.0).round() / 1000.0)),
        d.loop_in.map(|t| (t * 1000.0).round() / 1000.0),
        d.x.slip,
        d.x.dk.grid_tab,
        d.beats.first().map(|b| (b.time * 10000.0).round() / 10000.0),
        d.grid_undo.len(),
        d.x.tempo_adj * 100.0,
        a.prefs.deck.player_time_mode[(n as usize).min(3)],
        a.toasts.last().map(|t| t.text.as_str())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{render_deck, Decoded, PlayState};
    use std::sync::Arc;

    const SR: u32 = 44100;

    fn set(mut s: PlayState, f: impl FnOnce(&mut PlayState)) -> PlayState {
        f(&mut s);
        s
    }

    /// A ramp, so that a sample's value tells its position (frame * 1e-5).
    fn ramp_deck(frames: usize) -> PlayState {
        let x: Vec<f32> = (0..frames).flat_map(|i| [i as f32 * 1e-5, i as f32 * 1e-5]).collect();
        set(PlayState::default(), |s| {
            s.audio = Some(Arc::new(Decoded { sr: SR, samples: x, raw_frames: frames, damaged: 0, truncated: false }));
            s.playing = true;
            s.volume = 1.0;
            s.gain = 1.0;
        })
    }

    /// A constant 0.5, with the fades and ramps on.
    fn dc_deck(frames: usize) -> PlayState {
        set(PlayState::default(), |s| {
            s.audio = Some(Arc::new(Decoded { sr: SR, samples: vec![0.5; frames * 2], raw_frames: frames, damaged: 0, truncated: false }));
            s.playing = true;
            s.volume = 1.0;
            s.gain = 1.0;
            s.smooth = true;
        })
    }

    fn run(s: &mut PlayState, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        render_deck(s, &mut out, 2, SR);
        out.chunks(2).map(|c| c[0]).collect()
    }

    fn max_step(x: &[f32]) -> f32 {
        x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
    }

    fn grid(n: usize, change_at: usize) -> Vec<Beat> {
        // 120 BPM up to beat `change_at`, 150 BPM after it.
        let mut t = 0.0;
        (0..n)
            .map(|i| {
                let bpm = if i < change_at { 120.0 } else { 150.0 };
                let b = Beat { time: t, bpm, beat: (i % 4) as u8 + 1, index: i };
                t += 60.0 / bpm;
                b
            })
            .collect()
    }

    #[test]
    fn reverse_plays_the_same_samples_backwards() {
        let mut s = ramp_deck(20000);
        s.pos = 5000.0;
        let fwd = run(&mut s, 1000);
        assert_eq!(s.pos, 6000.0);
        s.pos = 5999.0;
        s.dk.reverse = true;
        let back = run(&mut s, 1000);
        assert!(back.iter().eq(fwd.iter().rev()));
        assert_eq!(s.pos, 4999.0);
        // Backwards into the start of the track: the deck stops there.
        run(&mut s, 6000);
        assert!(!s.playing && s.pos == 0.0);
    }

    #[test]
    fn slip_reverse_returns_to_where_the_track_would_be() {
        let mut s = set(ramp_deck(44100 * 4), |s| { s.smooth = true; });
        run(&mut s, 6000);
        let at = s.pos;
        s.dk.reverse = true;
        s.dk.slip_reverse = true;
        run(&mut s, 3000);
        assert!(s.pos < at - 2990.0 && s.slip_pos.is_some(), "{}", s.pos);
        s.dk.reverse = false;
        s.dk.slip_reverse = false;
        run(&mut s, 100);
        assert!((s.pos - (at + 3100.0)).abs() < 1.0, "{}", s.pos - (at + 3100.0));
        assert!(s.slip_pos.is_none());
        // Plain reverse (no SLIP): playback goes on from where reverse ended.
        let mut s = set(ramp_deck(44100 * 4), |s| { s.smooth = true; });
        run(&mut s, 5000);
        let at = s.pos;
        s.dk.reverse = true;
        run(&mut s, 3000);
        s.dk.reverse = false;
        run(&mut s, 100);
        assert!((s.pos - (at - 2900.0)).abs() < 1.0);
        // With the SLIP button on, plain reverse slips too.
        let mut s = set(ramp_deck(44100 * 4), |s| { s.smooth = true; s.slip = true; });
        run(&mut s, 5000);
        let at = s.pos;
        s.dk.reverse = true;
        run(&mut s, 1000);
        s.dk.reverse = false;
        run(&mut s, 100);
        assert!((s.pos - (at + 1100.0)).abs() < 1.0);
    }

    #[test]
    fn vinyl_ramps_take_the_time_of_the_setting() {
        // The table's values at steps the trace report quotes.
        for (idx, ms) in [(0u8, 1.0f32), (11, 1.0), (12, 10.0), (25, 10.0), (50, 40.0), (75, 90.0), (150, 570.0), (200, 1760.0), (250, 5110.0), (255, 5540.0)] {
            assert_eq!(vinyl_ms(idx), ms, "step {idx}");
        }
        for idx in [0u8, 50, 75, 150] {
            let mut s = dc_deck(44100 * 10);
            s.dk.stop_ms = vinyl_ms(idx);
            s.dk.start_ms = vinyl_ms(idx);
            // Start: from rest to full speed in the set time.
            let want = vinyl_ms(idx) as f64 * 44.1;
            let mut n = 0.0;
            while s.dk.vel < 1.0 && n < 400000.0 {
                run(&mut s, 1);
                n += 1.0;
            }
            assert!((n - want).abs() <= 2.0, "start, step {idx}: {n} frames, want {want}");
            run(&mut s, 500);
            // Stop: pause brakes to rest in the set time, without a step, and ends silent.
            s.playing = false;
            let pos = s.pos;
            let mut out = Vec::new();
            let mut n = 0.0;
            loop {
                out.extend(run(&mut s, 1));
                n += 1.0;
                if s.dk.vel == 0.0 || n > 400000.0 {
                    break;
                }
            }
            assert!((n - want).abs() <= 2.0, "stop, step {idx}: {n} frames, want {want}");
            assert!(max_step(&out) < 0.02 && *out.last().unwrap() == 0.0);
            // The platter travels half the ramp's length while it brakes.
            assert!(((s.pos - pos) - want / 2.0).abs() < 2.0, "{}", s.pos - pos);
        }
        // Master Tempo with a short ramp: pause only fades (44 frames), the speed is kept.
        let mut s = set(dc_deck(44100), |s| { s.master_tempo = true; });
        run(&mut s, 500);
        s.playing = false;
        let out = run(&mut s, 100);
        assert!(out[20] > 0.0 && out[43] == 0.0 && max_step(&out) < 0.02);
    }

    #[test]
    fn scratch_follows_the_platter() {
        // A paused deck moved by hand: the sound is the samples the platter passes over.
        let mut s = set(ramp_deck(40000), |s| { s.playing = false; });
        s.pos = 10000.0;
        let mut expect = 10000.0f64;
        for (speed, frames) in [(0.5, 1000usize), (-1.0, 500), (2.0, 300), (0.0, 50), (-0.25, 400)] {
            s.dk.jog = Jog::Speed(speed);
            let out = run(&mut s, frames);
            for (i, v) in out.iter().enumerate() {
                let p = expect + speed * i as f64;
                assert!((*v as f64 - p * 1e-5).abs() < 2e-6, "speed {speed} frame {i}: {v} for {p}");
            }
            expect += speed * frames as f64;
            assert!((s.pos - expect).abs() < 1e-6);
        }
        // Let go while paused: silence, and the deck stays where the hand left it.
        s.dk.jog = Jog::None;
        assert!(run(&mut s, 100).iter().all(|v| *v == 0.0));
        assert!((s.pos - expect).abs() < 1e-6);
        // A touch on a playing deck brakes it to rest in the Touch/Brake time and it is
        // silent; letting go starts it again in the Release/Start time.
        let mut s = dc_deck(44100 * 4);
        s.dk.stop_ms = 40.0;
        s.dk.start_ms = 90.0;
        run(&mut s, 5000);
        s.dk.jog = Jog::Touch;
        let out = run(&mut s, 1764 + 2);
        assert!(s.dk.vel == 0.0 && *out.last().unwrap() == 0.0 && out[800] > 0.0 && max_step(&out) < 0.02);
        let held = s.pos;
        run(&mut s, 1000);
        assert_eq!(s.pos, held);
        s.dk.jog = Jog::None;
        let out = run(&mut s, 3969 + 2);
        assert!(s.dk.vel == 1.0 && (out[3900] - 0.5).abs() < 1e-6 && max_step(&out) < 0.02, "{}", s.dk.vel);
        // Scratching with SLIP: the track runs on underneath and playback returns to it.
        let mut s = set(ramp_deck(44100 * 4), |s| { s.smooth = true; s.slip = true; });
        run(&mut s, 2000);
        let at = s.pos;
        s.dk.jog = Jog::Speed(-1.5);
        run(&mut s, 1000);
        s.dk.jog = Jog::None;
        run(&mut s, 200);
        // (Less what the start ramp from the scratch's speed costs: 2.5 speeds over 1 ms.)
        let lag = at + 1200.0 - s.pos;
        assert!(lag >= 0.0 && lag < 60.0, "{lag}");
    }

    #[test]
    fn inertia_rule() {
        // Playing forward at 1.05: a free platter still scratches while it is faster than
        // that or turns backwards; once it has slowed to the playing speed the deck plays on.
        assert!(scratchable_by_inertia(1.5, 1.05));
        assert!(scratchable_by_inertia(-0.2, 1.05));
        assert!(!scratchable_by_inertia(1.05, 1.05));
        assert!(!scratchable_by_inertia(0.4, 1.05));
        // In reverse the same, mirrored.
        assert!(scratchable_by_inertia(-1.5, -1.0));
        assert!(scratchable_by_inertia(0.3, -1.0));
        assert!(!scratchable_by_inertia(-0.5, -1.0));
    }

    #[test]
    fn sync_rate_rules() {
        // Half under 2/3 of the master's BPM, double over 1.5 times; the thresholds
        // themselves are still x1.
        assert_eq!(sync_rate_auto(150.0, 100.0), 1.0);
        assert_eq!(sync_rate_auto(150.0, 99.99), 0.5);
        assert_eq!(sync_rate_auto(100.0, 150.0), 1.0);
        assert_eq!(sync_rate_auto(100.0, 150.01), 2.0);
        assert_eq!(sync_rate_auto(140.0, 70.0), 0.5);
        assert_eq!(sync_rate_auto(87.0, 174.0), 2.0);
        assert_eq!(sync_rate_auto(128.0, 126.0), 1.0);
        // The speed, its cap of 2.0 and the 40 BPM floor.
        assert_eq!(sync_speed(140.0, 70.0, 0.5), Some(1.0));
        assert_eq!(sync_speed(140.0, 70.0, 1.0), Some(2.0));
        assert_eq!(sync_speed(141.0, 70.0, 1.0), None);
        assert_eq!(sync_speed(39.9, 80.0, 1.0), None);
        assert_eq!(sync_speed(120.0, 39.0, 1.0), None);
        // The button: x1 -> x2 when the deck can play that fast, else x1/2; x2 -> x1/2;
        // x1/2 -> x1.
        assert_eq!(sync_rate_toggle(1.0, 100.0, 120.0), 2.0);
        assert_eq!(sync_rate_toggle(1.0, 130.0, 120.0), 0.5);
        assert_eq!(sync_rate_toggle(2.0, 100.0, 120.0), 0.5);
        assert_eq!(sync_rate_toggle(0.5, 100.0, 120.0), 1.0);
        assert_eq!(sync_rate_toggle(0.5, 250.0, 120.0), 0.5);
        // Phase: at x1 the follower's beats on the master's; at x1/2 every follower beat on
        // every second master beat; at x2 every second follower beat on a master beat.
        assert!(sync_phase_error(10.25, 4.25, 1.0).abs() < 1e-12);
        assert!((sync_phase_error(10.3, 4.25, 1.0) - 0.05).abs() < 1e-9);
        assert!(sync_phase_error(3.5, 7.0, 0.5).abs() < 1e-12);
        assert!((sync_phase_error(3.55, 7.0, 0.5) - 0.1).abs() < 1e-9);
        assert!(sync_phase_error(14.5, 7.25, 2.0).abs() < 1e-12);
    }

    #[test]
    fn loops_follow_the_grid_through_a_tempo_change() {
        // 120 BPM (0.5 s a beat) up to beat 8 at 4.0 s, then 150 BPM (0.4 s).
        let g = grid(40, 8);
        let dur = 14.0;
        let l = (2.0, 4.0); // beats 4..8
        // Double: 8 beats, ending at beat 12 = 4.0 + 4 * 0.4.
        let d = loop_scaled(&g, l, 2.0, SR, dur).unwrap();
        assert!((d.0 - 2.0).abs() < 1e-9 && (d.1 - 5.6).abs() < 1e-4, "{d:?}");
        // Double again: 16 beats, to beat 20 = 4.0 + 12 * 0.4.
        let d2 = loop_scaled(&g, d, 2.0, SR, dur).unwrap();
        assert!((d2.1 - 8.8).abs() < 1e-4);
        // Halve: back, and on to 2 beats and 1 beat.
        let h = loop_scaled(&g, d, 0.5, SR, dur).unwrap();
        assert!((h.1 - 4.0).abs() < 1e-4);
        let h2 = loop_scaled(&g, h, 0.5, SR, dur).unwrap();
        assert!((h2.1 - 3.0).abs() < 1e-4);
        // A 4-beat loop across the change (beats 6..10): halved it ends at beat 8.
        let across = (3.0, 4.8);
        assert!((loop_scaled(&g, across, 0.5, SR, dur).unwrap().1 - 4.0).abs() < 1e-4);
        // Move on by its own length: beats 4..8 become 8..12, and back.
        let m = loop_moved(&g, l, 1, SR, dur);
        assert!((m.0 - 4.0).abs() < 1e-4 && (m.1 - 5.6).abs() < 1e-4, "{m:?}");
        let back = loop_moved(&g, m, -1, SR, dur);
        assert!((back.0 - 2.0).abs() < 1e-4 && (back.1 - 4.0).abs() < 1e-4);
        // Not before the start of the track, not past its end.
        let first = loop_moved(&g, (0.5, 1.5), -1, SR, dur);
        assert!(first.0 == 0.0 && (first.1 - 1.0).abs() < 1e-4);
        let end = loop_moved(&g, (12.0, 13.6), 1, SR, dur);
        assert!(end.1 <= dur && (end.1 - end.0 - 1.6).abs() < 1e-3);
        // The minimum loop (6.667 ms) and the end of the track refuse a halve / double.
        assert!(loop_scaled(&[], (1.0, 1.012), 0.5, SR, dur).is_none());
        assert!(loop_scaled(&[], (1.0, 1.02), 0.5, SR, dur).is_some());
        assert!(loop_scaled(&g, (10.0, 13.0), 2.0, SR, dur).is_none());
        // Edges: whole frames, never closer than the minimum, never outside the track.
        let e = loop_edge((1.0, 1.5), true, 10.0, SR, dur);
        assert!((e.1 - e.0 - MIN_LOOP_SECS).abs() < 1.0 / SR as f64);
        assert_eq!(loop_edge((1.0, 1.5), false, 100.0, SR, dur).1, dur);
        let f = loop_edge((1.0, 1.5), true, -0.0101, SR, dur).0 * SR as f64;
        assert!((f - f.round()).abs() < 1e-6);
    }

    #[test]
    fn loop_wrap_is_sample_exact() {
        // At a speed that puts the playhead between frames, every pass through the loop
        // continues from the in point by exactly what it was past the out point.
        let mut s = ramp_deck(20000);
        s.tempo = 1.07;
        s.pos = 2900.0;
        let (a, b) = (3000.0f64, 3441.0f64);
        s.loop_range = Some((a / SR as f64, b / SR as f64));
        let out = run(&mut s, 4000);
        let mut p = 2900.0f64;
        let mut wraps = 0;
        for (i, v) in out.iter().enumerate() {
            if p >= b {
                p = a + (p - b) % (b - a);
                wraps += 1;
            }
            assert!((*v as f64 - p * 1e-5).abs() < 2e-6, "frame {i}: {v} for {p}");
            p += 1.07;
        }
        assert!(wraps >= 9 && (s.pos - p).abs() < 1e-6);
        // Backwards the wrap is at the in point.
        let mut s = ramp_deck(20000);
        s.dk.reverse = true;
        s.pos = 3200.0;
        s.loop_range = Some((a / SR as f64, b / SR as f64));
        run(&mut s, 1000);
        assert!(s.pos >= a && s.pos < b);
        // A quantized jump waiting for its place is taken on the sample.
        let mut s = ramp_deck(20000);
        s.pos = 1000.0;
        s.dk.warp = Some((1500.0, 9000.0));
        let out = run(&mut s, 1000);
        assert_eq!(out[499], 1499.0 * 1e-5);
        assert_eq!(out[500], 9000.0 * 1e-5);
        assert!(s.pos == 9500.0 && s.dk.warp.is_none());
    }

    #[test]
    fn fades_have_rekordboxs_shape_and_no_step() {
        // The curve: 44 steps of a raised cosine from 0 to 1.
        assert!(fade_curve(0).abs() < 1e-7 && (fade_curve(43) - 1.0).abs() < 1e-6 && (fade_curve(21) + fade_curve(22) - 1.0).abs() < 1e-6);
        let mut s = dc_deck(44100);
        let a = run(&mut s, 200);
        for i in 0..43 {
            assert!((a[i] - 0.5 * fade_curve(i + 1)).abs() < 1e-6, "{i}");
        }
        assert!(a[100] == 0.5);
        // A jump: 43 frames out, the move, 43 frames in.
        s.pos += 5000.0;
        let b = run(&mut s, 200);
        for i in 0..43 {
            assert!((b[i] - 0.5 * fade_curve(42 - i)).abs() < 1e-6, "{i}");
            assert!((b[43 + i] - 0.5 * fade_curve(i + 1)).abs() < 1e-6, "{i}");
        }
        assert!(b[42] == 0.0 && b[86] == 0.5 && max_step(&b) < 0.02);
        // A loop wrap dips the same way; a loop of 2 ms or less is left alone.
        let mut s = dc_deck(44100);
        run(&mut s, 2000);
        s.loop_range = Some((0.04, 0.05));
        let c = run(&mut s, 2000);
        assert!(c.iter().any(|v| *v == 0.0) && max_step(&c) < 0.02);
        let mut s = dc_deck(44100);
        run(&mut s, 2000);
        s.loop_range = Some((0.04, 0.04 + 80.0 / 44100.0));
        let c = run(&mut s, 2000);
        assert!(c.iter().all(|v| *v == 0.5));
    }

    #[test]
    fn crossfader_curves_and_assign() {
        // Curve 0: a straight blend over the whole travel.
        let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 2e-4 && (a.1 - b.1).abs() < 2e-4;
        assert!(near(crossfader_curve(0.0, 0), (1.0, 0.0)) && near(crossfader_curve(1.0, 0), (0.0, 1.0)));
        assert!(near(crossfader_curve(0.5, 0), (0.4995, 0.5005)), "{:?}", crossfader_curve(0.5, 0));
        // Curve 16 (the default): full level to step 450, 0.893 / 0.895 at the centre.
        assert!(near(crossfader_curve(0.0, 16), (1.0, 0.0)) && near(crossfader_curve(1.0, 16), (0.0, 1.0)));
        assert!(near(crossfader_curve(0.5, 16), (0.8928, 0.8945)), "{:?}", crossfader_curve(0.5, 16));
        assert!(crossfader_curve(450.0 / 1023.0, 16).0 == 1.0 && crossfader_curve(452.0 / 1023.0, 16).0 < 1.0);
        // Curve 32: both sides at full level until the very end.
        assert!(near(crossfader_curve(0.0, 32), (1.0, 0.0)) && near(crossfader_curve(1.0, 32), (0.0, 1.0)));
        assert!(near(crossfader_curve(0.5, 32), (1.0, 1.0)) && near(crossfader_curve(0.999, 32), (1.0, 1.0)));
        // Knees as in rekordbox's family.
        for (bank, knee) in [(0u8, 0.0f32), (1, 28.0), (5, 141.0), (12, 337.0), (15, 422.0), (17, 486.0), (24, 736.0), (31, 987.0), (32, 1023.0)] {
            assert_eq!(crossfader_knee(bank), knee, "bank {bank}");
        }
        // In the deck: the crossfader gain moves over 2 ms (88 frames).
        let mut s = set(dc_deck(44100), |s| { s.smooth = false; });
        s.dk.route = 1;
        s.dk.xf = [1.0, 0.0];
        run(&mut s, 500);
        s.dk.xf = [0.0, 1.0];
        let out = run(&mut s, 200);
        assert!(out[86] > 0.0 && out[87] == 0.0 && max_step(&out) < 0.5 / 87.0);
        // A new assignment: the old side out over 3.3 ms (146 frames), then the new side in.
        s.dk.xf = [1.0, 1.0];
        run(&mut s, 500);
        s.dk.route = 2;
        let out1 = run(&mut s, 146);
        let out2 = run(&mut s, 146);
        assert!(out1[0] > 0.49 && out1[145] == 0.0 && out2[0] > 0.0 && out2[0] < 0.01 && out2[145] == 0.5);
        assert!(max_step(&out1) < 0.5 / 145.0 && max_step(&out2) < 0.5 / 145.0);
    }

    #[test]
    fn headroom_and_again() {
        // Headroom NONE: the trim's centre is 0.21 dB over unity; each step takes 3 dB off.
        let db = |g: f32| 20.0 * g.log10();
        assert!((db(headroom_gain(1)) - 0.21).abs() < 0.01);
        assert!((db(headroom_gain(4)) + 8.79).abs() < 0.01);
        assert!((db(headroom_gain(8)) + 20.79).abs() < 0.01);
        assert_eq!(headroom_gain(0), headroom_gain(4));
        // A-GAIN: the knob's centre reads about 0 dB, its top +9 dB; position and dB invert.
        assert!((again_db(0.5) - 0.21).abs() < 0.05 && (again_db(1.0) - 9.0).abs() < 0.05);
        for d in [-12.0f32, -3.0, 0.0, 4.5] {
            assert!((again_db(again_pos(d)) - d).abs() < 0.1, "{d}");
        }
    }

    #[test]
    fn quantize_rules() {
        let g = grid(40, 100); // 120 BPM: a beat every 0.5 s
        // Thresholds: half the unit for SNAP; 1/32 beat for HOT CUE and LOOP (half the unit
        // when the unit is that small).
        assert_eq!(quantize_threshold(1.0, 0.5), 0.5);
        assert_eq!(quantize_threshold(0.25, 0.5), 0.125);
        assert_eq!(quantize_threshold(0.25, 0.03125), 0.03125);
        assert_eq!(quantize_threshold(0.03125, 0.03125), 0.015625);
        // SNAP: the nearest point, a tie to the later one.
        let q = |t: f64, kind: QType, unit: f64| quantize_point(&g, t, kind, unit, false);
        assert!((q(0.74, QType::Snap, 1.0) - 0.5).abs() < 1e-9);
        assert!((q(0.76, QType::Snap, 1.0) - 1.0).abs() < 1e-9);
        assert!((q(0.75, QType::Snap, 1.0) - 1.0).abs() < 1e-9);
        assert!((q(0.80, QType::Snap, 0.5) - 0.75).abs() < 1e-9);
        // HOT CUE on 1/4 beat: the next point, unless one was passed by under 1/32 beat.
        assert!((q(1.01, QType::HotCue, 0.25) - 1.0).abs() < 1e-9);
        assert!((q(1.02, QType::HotCue, 0.25) - 1.125).abs() < 1e-9);
        assert!((q(1.124, QType::HotCue, 0.25) - 1.125).abs() < 1e-9);
        // Units as stored.
        assert_eq!([unit_beats(2), unit_beats(4), unit_beats(6)], [0.0625, 0.25, 1.0]);
        // The grid mapping follows a tempo change.
        let g = grid(40, 8);
        assert!((grid_pos(&g, 4.2).unwrap() - 8.5).abs() < 1e-9 && (grid_time(&g, 9.5).unwrap() - 4.6).abs() < 1e-9);
    }

    #[test]
    fn bend_slope_scan_and_tempo_limits() {
        // Bend slope: the bend grows by the slope every millisecond, up to 0.42.
        let mut s = dc_deck(44100 * 30);
        s.dk.bend_slope = 0.0001;
        for _ in 0..10 {
            run(&mut s, 441);
        }
        assert!((s.dk.bend_acc - 0.01).abs() < 1e-9);
        s.dk.bend_slope = 0.01;
        run(&mut s, 44100);
        assert_eq!(s.dk.bend_acc, BEND_LIMIT);
        // The engine's tempo limit is +-100 %: speed 0 .. 2.
        let mut s = set(ramp_deck(44100), |s| { s.smooth = false; s.tempo = 3.0; });
        run(&mut s, 1000);
        assert_eq!(s.pos, 2000.0);
        let mut s = set(ramp_deck(44100), |s| { s.smooth = false; s.tempo = -0.5; });
        s.pos = 500.0;
        run(&mut s, 1000);
        assert_eq!(s.pos, 500.0);
        // Scan: 20 times the speed in 133 ms grains, on a paused deck too.
        let mut s = set(ramp_deck(44100 * 60), |s| { s.playing = false; s.search = SCAN_SPEED; });
        s.dk.scan_grain = SCAN_GRAIN_SECS;
        for _ in 0..100 {
            run(&mut s, 441);
        }
        let secs = s.pos / 44100.0;
        assert!(secs > 17.0 && secs < 21.0, "{secs}");
    }

    #[test]
    fn djm_900nxs_eq_type() {
        // Level of a sine through the equalizer.
        let level = |knobs: [f32; 3], freq: f32| -> f32 {
            let mut eq = crate::mixer::Eq3::new();
            eq.nxs = true;
            eq.prepare(48000, knobs, 48000);
            let n = 48000;
            let mut sum = 0.0f64;
            for i in 0..n {
                let y = eq.run(0, (i as f32 * freq * std::f32::consts::TAU / 48000.0).sin());
                if i >= n / 2 {
                    sum += (y * y) as f64;
                }
            }
            10.0 * ((sum / (n / 2) as f64) * 2.0).log10() as f32
        };
        // Flat at the centre; +6 dB at each band's frequency fully right, -26 dB fully left.
        for f in [70.0, 1000.0, 13000.0] {
            assert!(level([0.5; 3], f).abs() < 0.05, "{f}");
        }
        assert!((level([0.5, 0.5, 1.0], 13000.0) - 6.0).abs() < 0.3);
        assert!((level([0.5, 1.0, 0.5], 1000.0) - 6.0).abs() < 0.3);
        assert!((level([1.0, 0.5, 0.5], 70.0) - 6.0).abs() < 0.3);
        assert!((level([0.0, 0.5, 0.5], 70.0) + 26.4).abs() < 0.5);
        assert!((level([0.5, 0.5, 0.0], 13000.0) + 26.4).abs() < 0.5);
        // The bands are peaks, not shelves: LOW fully left takes far less off 1 kHz than off 70 Hz.
        assert!(level([0.0, 0.5, 0.5], 1000.0) > -6.0, "{}", level([0.0, 0.5, 0.5], 1000.0));
    }

    #[test]
    fn limited_sync_rate_window_and_toggle() {
        // Normal: up to double speed; limited: 0.5 ..= 1.5 (`isBpmWithinRange`).
        assert_eq!(sync_speed_limited(180.0, 100.0, 1.0, false), Some(1.8));
        assert_eq!(sync_speed_limited(180.0, 100.0, 1.0, true), None);
        assert_eq!(sync_speed_limited(150.0, 100.0, 1.0, true), Some(1.5));
        assert_eq!(sync_speed_limited(49.0, 100.0, 1.0, true), None);
        assert_eq!(sync_speed_limited(50.0, 100.0, 1.0, true), Some(0.5));
        // `getToggleSyncRate` with +69 set: master 128, own 126 (r = 1.016).
        assert_eq!(sync_rate_toggle_limited(1.0, 128.0, 126.0, true), 0.5); // 2r > 1.5, r/2 >= 0.5
        assert_eq!(sync_rate_toggle_limited(0.5, 128.0, 126.0, true), 1.0);
        assert_eq!(sync_rate_toggle_limited(2.0, 128.0, 126.0, true), 0.5);
        // master 70, own 140 (r = 0.5): x1 -> x2 (2r = 1.0 fits).
        assert_eq!(sync_rate_toggle_limited(1.0, 70.0, 140.0, true), 2.0);
        // x2 with r/2 under 0.5 and r in range -> x1.
        assert_eq!(sync_rate_toggle_limited(2.0, 70.0, 140.0, true), 1.0);
        // Unlimited keeps the old rule.
        assert_eq!(sync_rate_toggle_limited(1.0, 128.0, 126.0, false), sync_rate_toggle(1.0, 128.0, 126.0));
        assert!(motorized_jog_device("DDJ-REV7") && motorized_jog_device("Pioneer DDJ-REV7 MIDI") && !motorized_jog_device("DDJ-FLX10"));
        // A deck master's play head has no ms threshold: any error over 0.01 beat jumps.
        assert!(jump_threshold_deviation(0.011, 128.0, DECK_MASTER_JUMP_THRESHOLD_MS));
        assert!(!jump_threshold_deviation(0.02, 120.0, 20.0) && jump_threshold_deviation(0.05, 120.0, 20.0));
    }

    #[test]
    fn loop_out_and_auto_loop_points_follow_the_loop_value() {
        let g = grid(64, 64); // 0.5 s beats
        // LOOP value 1 beat: OUT 0.3 beat after a beat waits for the next beat...
        assert!((quantize_point(&g, 1.15, QType::Loop, 1.0, false) - 1.5).abs() < 1e-9);
        // ...but one passed by less than 1/32 beat is taken now.
        assert!((quantize_point(&g, 1.0 + 0.01 * 0.5, QType::Loop, 1.0, false) - 1.0).abs() < 1e-9);
        // LOOP IN with SNAP 1 beat: the nearest beat.
        assert!((quantize_point(&g, 1.2, QType::Snap, 1.0, false) - 1.0).abs() < 1e-9);
        assert!((quantize_point(&g, 1.3, QType::Snap, 1.0, false) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn a_loop_behind_the_playhead_starts_on_its_lattice() {
        // Loop 1000..1500 frames armed with a warp at 2500 -> 1000: until 2500 the deck plays
        // on, then it is inside the loop and wraps at 1500 from then on.
        let mut s = ramp_deck(20000);
        s.smooth = false;
        s.pos = 2000.0;
        s.dk.warp = Some((2500.0, 1000.0));
        s.dk.arm_loop = Some((1000.0 / SR as f64, 1500.0 / SR as f64));
        let out = run(&mut s, 1200);
        let at = |i: usize| (out[i] / 1e-5).round() as i64;
        assert_eq!(at(0), 2000);
        assert_eq!(at(499), 2499);
        assert_eq!(at(500), 1000);
        assert_eq!(at(999), 1499);
        assert_eq!(at(1000), 1000);
        assert!(s.loop_range.is_some() && s.dk.arm_loop.is_none());
    }

    #[test]
    fn eq_type_switch_fades_out_and_in_over_3_3_ms() {
        // A constant through the strip: the change of type dips to silence and comes back,
        // 3.3 ms down and 3.3 ms up (146 frames each at 44.1 kHz).
        let mut s = dc_deck(44100);
        s.strip = 1;
        s.trim = 1.0;
        let _ = run(&mut s, 2000);
        s.strip = 2;
        let out = run(&mut s, 1000);
        let n = (EQ_SWITCH_SECS * SR as f32).round() as usize;
        let min_i = out.iter().enumerate().min_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|x| x.0).unwrap();
        assert!(out[min_i].abs() < 0.01, "dip {}", out[min_i]);
        assert!((min_i as i64 - n as i64).abs() <= 2, "{min_i} vs {n}");
        assert!(out[2 * n + 20].abs() > 0.3);
        assert_eq!(s.dk.eqsw.mode, 2);
        // No change, no dip.
        let out = run(&mut s, 400);
        assert!(out.iter().all(|v| v.abs() > 0.3));
    }

    #[test]
    fn nxs_eq_moves_in_filter_steps() {
        let t = nxs_table();
        assert_eq!(t[0], 0);
        assert_eq!(t[255], NXS_STEPS);
        assert!(t.windows(2).all(|w| w[0] <= w[1]));
        // The centre of the knob is the middle of its step's knob range: flat.
        assert!((nxs_knob_of(nxs_step_of(0.5) as f32) - 0.5).abs() < 0.02);
        // One step per 1/3 ms: from the left end to the right end takes 487 steps.
        let mut st = NxsSteps::default();
        st.advance([0.0; 3], 0, 48000);
        let k = st.advance([1.0; 3], 1600, 48000); // 1600 frames = 33.3 ms = 100 steps
        assert!((st.cur[0] - 100.0).abs() < 0.5, "{}", st.cur[0]);
        assert!(k[0] > 0.0 && k[0] < 0.5);
        let k = st.advance([1.0; 3], 48000, 48000);
        assert_eq!(st.cur[0], NXS_STEPS as f32);
        // (The top step is the knob's last few positions; their middle is just under 1.)
        assert!(k[0] > 0.99, "{}", k[0]);
    }

    #[test]
    fn platter_angle_and_bpm_drag() {
        // 33 1/3 rpm: 1.8 s is one turn; the marks sit 44.041 degrees before the turn.
        assert!((platter_angle_deg(0.0, RPM_33) + 44.041).abs() < 1e-3);
        assert!((platter_angle_deg(0.45, RPM_33) - (90.0 - 44.041)).abs() < 0.05);
        assert!((platter_angle_deg(0.45, 45.0) - (121.5 - 44.041)).abs() < 0.05);
        // BPM drag: 250 pixels for the whole range, up is faster, steps of 0.02.
        let v = bpm_drag_value(128.0, 128.0, 0.10, -25.0);
        assert!((v - 130.56).abs() < 1e-6, "{v}");
        let v = bpm_drag_value(128.0, 128.0, 0.10, 1.0);
        assert!(((v - 128.0) / 0.02 - ((v - 128.0) / 0.02).round()).abs() < 1e-6 && v < 128.0);
        assert!((bpm_drag_value(128.0, 128.0, 0.10, -1000.0) - 140.8).abs() < 1e-9);
    }

}
