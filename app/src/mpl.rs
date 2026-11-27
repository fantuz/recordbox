//! MIX POINT LINK (PERFORMANCE mode): a MIX OUT point on a playing deck and a MIX IN point on
//! a waiting deck are linked, and the waiting deck starts by itself so that the two points
//! pass at the same instant.
//!
//! Traced from rekordbox 7.2.19 (`checks/re-7.2.19/pads-sampler-automix.md` C5 to C7, and for
//! this file `MixPointLinkComp::updateSelectableDeckStatus` 0x1014f7e34, `setSelectDeckNo`
//! 0x1014f925c, `getMixPointTimeByNonCue` 0x1014fbc64, `isMixPointLinkOffNeeded` 0x1014faaac,
//! `getMixPointLinkPlayStartTime` 0x1014fb5a8, `getRealMixPointTime` 0x101501628 and the
//! engine's `DjPlayerMixPointLink::onPreRender` 0x102b4ca00).
//!
//! Three parts: the model (`MplState`: which deck holds which point, the pair statuses and
//! every refusal), the link computation (`start_out_ms`), and the engine that runs at the top
//! of every audio block (`pre_render`). Nothing here is written to the library: the points
//! live for the session and go when another track is loaded, as in rekordbox.

use crate::app::*;
use crate::audio::PlayState;
use crate::model::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- preferences

/// Preferences > Controller > MIX POINT LINK (rekordbox's eight settings and defaults).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MplPrefs {
    /// `MixPointLinkEnable`.
    pub enable: bool,
    /// `MixPointTypeDeult`: 1 = MIX IN, 2 = MIX OUT.
    pub default_type: u8,
    /// `MixPointAutoChanged`.
    pub auto_change: bool,
    /// `MixPointLinkShowTimeScale`.
    pub show_time_scale: bool,
    /// `MixPointLinkShowBeatGrid` and `MixPointLinkShowBeatGridWhenZoomOut` ("Display only
    /// when waveform is enlarged").
    pub show_grid: bool,
    pub grid_when_enlarged: bool,
    /// `MixPointLinkBpmSyncAutoChange`.
    pub bpm_sync_auto: bool,
    /// `MixPointLinkZoomOutLimit`.
    pub zoom_out_limit: bool,
}

impl Default for MplPrefs {
    fn default() -> Self {
        MplPrefs { enable: true, default_type: 2, auto_change: true, show_time_scale: true, show_grid: true, grid_when_enlarged: true, bpm_sync_auto: false, zoom_out_limit: true }
    }
}

// ---------------------------------------------------------------- messages

pub const MSG_NO_GRID: &str = "MIX POINT LINK function is not available because the track has no beat grid information.";
pub const MSG_CLEAR_MIX_IN: &str = "Invalid operation. When changing the deck for a MIX POINT, first clear the current MIX POINT (MIX IN).";
pub const MSG_CLEAR_MIX_OUT: &str = "Invalid operation. When changing the deck for a MIX POINT, first clear the current MIX POINT (MIX OUT).";
pub const MSG_NEED_PAUSED: &str = "Invalid operation. Specify a deck that is currently being paused.";
pub const MSG_IN_ON_OUT_DECK: &str = "Invalid operation. MIX POINT (MIX IN) cannot be set on a deck with a MIX POINT (MIX OUT).";
pub const MSG_NEED_PLAYING: &str = "Invalid operation. Specify a deck currently being played or a deck with MIX POINT (MIX IN).";
pub const MSG_OUT_AFTER_IN: &str = "Invalid operation. Set MIX POINT (MIX OUT) after MIX POINT (MIX IN).";
pub const MSG_ALREADY_SET: &str = "Invalid operation. Cannot operate since MIX POINT is set.";
pub const MSG_NOT_MOVED: &str = "Invalid operation. MIX POINT was not moved. MIX POINT can be moved by clicking a Cue marker/waveform/MIX POINT SELECT button.";
pub const MSG_SILENT_NEEDS_IN: &str = "Invalid operation. Specify a deck with MIX POINT (MIX IN).";
pub const MSG_SILENT_EDIT_OFF: &str = "Invalid operation. Cannot turn on SILENT EDIT because the track is set to start playing automatically.";
pub const MSG_PANEL_AUTOMIX: &str = "Invalid operation. MIX POINT LINK panel cannot be displayed while Automix is operating.";
pub const MSG_FN_AUTOMIX: &str = "Invalid operation. MIX POINT LINK function is not available while Automix is operating.";
pub const MSG_DISABLED: &str = "MIX POINT LINK setting has been disabled.";

/// The guidance line under the waveforms (rekordbox's six texts; 0 = none).
pub const GUIDANCE: [&str; 7] = [
    "",
    "A [MIX IN] deck can be selected for the start of mixing. To select a [MIX IN] deck, click the loaded Enlarged Waveform.",
    "A [MIX OUT] deck can be selected for the end of mixing. To select a [MIX OUT] deck, click the loaded Enlarged Waveform.",
    "A MIX POINT can be selected for the start of mixing. \nTo select a MIX POINT, click a Cue marker on the Enlarged Waveform for a [MIX IN] deck or click the MIX POINT SELECT button.",
    "A MIX POINT can be selected for the end of mixing. \nTo select a MIX POINT, click a Cue marker on the Enlarged Waveform for a [MIX OUT] deck or click the MIX POINT SELECT button.",
    "The track can be set to start playing automatically. Click the [MIX POINT LINK] button to set for the track to start playing automatically.",
    "The point where muting ends can be changed to any point between the start of a track and the MIX POINT. \nTo change the point where muting ends, click the Enlarged Waveform.",
];

// ---------------------------------------------------------------- the model

/// `eMPLSetMode`.
pub const MODE_NONE: u8 = 0;
pub const MODE_IN: u8 = 1;
pub const MODE_OUT: u8 = 2;

/// Pair status (rekordbox's values): 2 = nothing, 1 = both points set, 0 = linked.
pub const PAIR_NONE: u8 = 2;
pub const PAIR_READY: u8 = 1;
pub const PAIR_LINKED: u8 = 0;

/// What the model needs to know about a deck.
#[derive(Clone, Copy, Default, Debug)]
pub struct DeckInfo {
    pub loaded: bool,
    pub has_grid: bool,
    pub playing: bool,
    pub total_ms: i64,
}

/// One link for the engine: `DjPlayerUnit::updateMixPointLinkInfo`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkInfo {
    pub out_deck: usize,
    pub in_deck: usize,
    pub start_out_ms: f64,
    pub mix_out_ms: f64,
    pub mix_in_ms: f64,
    pub silent_on: bool,
    pub silent_end_ms: f64,
}

pub struct MplState {
    /// The strip is shown (the LINK button of the top bar).
    pub shown: bool,
    pub mode: u8,
    pub selected: Option<usize>,
    /// The decks holding the MIX IN and MIX OUT point being set.
    pub in_deck: Option<usize>,
    pub out_deck: Option<usize>,
    /// Points in ms; -1 = not set.
    pub mix_in: [i64; 4],
    pub mix_out: [i64; 4],
    pub silent_on: [bool; 4],
    pub silent_edit: [bool; 4],
    pub silent_end: [i64; 4],
    /// `pair[out][in]`.
    pub pair: [[u8; 4]; 4],
    /// Per MIX IN deck: the link is set; it has been started by the link. Per MIX OUT deck:
    /// it has passed its MIX OUT point.
    pub link_set: [bool; 4],
    pub auto_started: [bool; 4],
    pub reached: [bool; 4],
    /// Waveform scrolling: drags move the waveforms and clicks set nothing.
    pub move_mode: bool,
    /// MIX POINT SELECT: index of the cue chosen last.
    pub cue_sel: Option<usize>,
    pub guidance: u8,
    /// The time ruler: zoom step (1..10) and the time at the left edge (s).
    pub zoom: u8,
    pub view_start: f64,
    /// What each deck looked like on the last frame, to notice loads, jumps, tempo changes
    /// and starts and stops.
    pub seen_track: [Option<i64>; 4],
    pub seen_tempo: [f64; 4],
    pub seen_pos: [f64; 4],
    pub seen_playing: [bool; 4],
    pub seen_at: Option<std::time::Instant>,
    /// Where the strip's controls were drawn (for the scripted run).
    pub ui: crate::ui_mpl::StripMap,
    /// Decks switched to BPM SYNC (tempo only) by a link (see `needs_bpm_sync`); it lasts
    /// until the deck's sync is turned off.
    pub bpm_sync: [bool; 4],
}

/// The rule of `doMixPointLinkStart` (0x1014fa050) for a MIX IN deck that is in BEAT SYNC:
/// it changes to BPM SYNC unless both MIX points sit on beats and the "constant BPM" test
/// does not apply (`auto` is "Automatically changes the BEAT/BPM SYNC setting").
pub fn needs_bpm_sync(both_on_beats: bool, out_variable_bpm: bool, auto: bool) -> bool {
    !both_on_beats || (out_variable_bpm && auto)
}

/// A grid with more than one BPM value (rekordbox compares the BPM of its beat entries).
pub fn variable_bpm(beats: &[Beat]) -> bool {
    beats.len() >= 2 && beats.iter().any(|b| (b.bpm - beats[0].bpm).abs() > 1e-6)
}

/// Decks a link put into BPM SYNC (read by the deck engine: their tempo follows the master,
/// their beats do not).
pub fn bpm_sync_decks(app: &App) -> [bool; 4] {
    let mut v = app.mixx.mpl.bpm_sync;
    for (d, x) in v.iter_mut().enumerate() {
        *x &= d < app.deck_count() as usize && app.deck_of(d as u8).x.beat_sync;
    }
    v
}

/// The MIX IN deck's bar count while its link is set (`operateWaveViewSetMixPointBeatCountMode`
/// on the deck's wave view): bars and beats counted from the MIX IN point, which is 1.1; the
/// beats before it count down from -1.4 ("-1.2Bars" is three beats before the point).
pub fn mix_point_bars(beats: &[Beat], mix_in_s: f64, t: f64) -> String {
    if beats.is_empty() {
        return String::new();
    }
    let k = beats.partition_point(|b| b.time <= mix_in_s + 1e-3).max(1) - 1;
    let i = beats.partition_point(|b| b.time <= t + 1e-4).max(1) - 1;
    if i < k {
        let back = k - i;
        return format!("-{}.{}Bars", (back + 3) / 4, 4 - (back - 1) % 4);
    }
    let d = i - k;
    format!("{}.{}Bars", d / 4 + 1, d % 4 + 1)
}

impl Default for MplState {
    fn default() -> Self {
        MplState {
            shown: false,
            mode: MODE_NONE,
            selected: None,
            in_deck: None,
            out_deck: None,
            mix_in: [-1; 4],
            bpm_sync: [false; 4],
            mix_out: [-1; 4],
            silent_on: [false; 4],
            silent_edit: [false; 4],
            silent_end: [-1; 4],
            pair: [[PAIR_NONE; 4]; 4],
            link_set: [false; 4],
            auto_started: [false; 4],
            reached: [false; 4],
            move_mode: false,
            cue_sel: None,
            guidance: 0,
            zoom: DEFAULT_ZOOM,
            view_start: 0.0,
            seen_track: [None; 4],
            seen_tempo: [1.0; 4],
            seen_pos: [0.0; 4],
            seen_playing: [false; 4],
            seen_at: None,
            ui: Default::default(),
        }
    }
}

/// Zoom steps 1..10 (`reqWaveViewZoomUpDown`). The step the reference picture shows
/// (84.2 s across the lanes) is taken as step 6; a step halves or doubles the time shown.
pub const DEFAULT_ZOOM: u8 = 6;
pub fn zoom_span(step: u8) -> f64 {
    84.2 * 2f64.powi(step.clamp(1, 10) as i32 - DEFAULT_ZOOM as i32)
}

impl MplState {
    /// The deck's link as the MIX OUT deck: (status, the MIX IN partner).
    pub fn as_out(&self, d: usize) -> (u8, Option<usize>) {
        (0..4).filter(|i| *i != d).find(|i| self.pair[d][*i] != PAIR_NONE).map(|i| (self.pair[d][i], Some(i))).unwrap_or((PAIR_NONE, None))
    }

    /// The deck's link as the MIX IN deck: (status, the MIX OUT partner).
    pub fn as_in(&self, d: usize) -> (u8, Option<usize>) {
        (0..4).filter(|o| *o != d).find(|o| self.pair[*o][d] != PAIR_NONE).map(|o| (self.pair[o][d], Some(o))).unwrap_or((PAIR_NONE, None))
    }

    pub fn any_ready(&self) -> bool {
        self.pair.iter().flatten().any(|s| *s == PAIR_READY)
    }

    pub fn any_linked(&self) -> bool {
        self.pair.iter().flatten().any(|s| *s == PAIR_LINKED)
    }

    fn base_guidance(&self) -> u8 {
        match (self.mode, self.selected.is_some()) {
            (MODE_IN, false) => 1,
            (MODE_OUT, false) => 2,
            (MODE_IN, true) => 3,
            (MODE_OUT, true) => 4,
            _ => 0,
        }
    }

    /// `changeMixPointSetMode`: MIX IN or MIX OUT; the deck is chosen again for the new mode.
    pub fn set_mode(&mut self, mode: u8) {
        self.mode = mode.min(2);
        self.selected = None;
        self.cue_sel = None;
        self.guidance = if self.any_ready() { 5 } else { self.base_guidance() };
    }

    /// `updateSelectableDeckStatus`: which decks the current mode can take.
    pub fn selectable(&self, decks: &[DeckInfo; 4]) -> [bool; 4] {
        let mut sel = [false; 4];
        let mut seen_ready_in = false;
        for d in 0..4 {
            if !decks[d].loaded {
                continue;
            }
            let (as_in, _) = self.as_in(d);
            let (as_out, partner) = self.as_out(d);
            if !decks[d].has_grid {
                continue;
            }
            if self.mode != MODE_IN || seen_ready_in {
                // MIX OUT: a playing deck without a MIX IN point that is in no link; the MIX
                // OUT deck of a pair whose MIX IN deck is not link-set yet; or a deck that was
                // itself brought in by a link (its own MIX IN point set and linked).
                if self.mode == MODE_OUT
                    && ((decks[d].playing && self.mix_in[d] == -1 && as_out != PAIR_LINKED && as_in != PAIR_LINKED)
                        || partner.is_some_and(|p| !self.link_set[p])
                        || (self.mix_in[d] != -1 && as_in == PAIR_LINKED && as_out != PAIR_LINKED))
                {
                    sel[d] = true;
                }
            } else {
                // MIX IN: a paused deck without a MIX OUT point, or the deck that has the MIX
                // IN point already.
                if (!decks[d].playing && self.mix_out[d] == -1) || self.mix_in[d] != -1 {
                    sel[d] = true;
                }
                seen_ready_in = as_in == PAIR_READY;
            }
        }
        sel
    }

    /// `setSelectDeckNo`: the first click on a deck's waveform chooses the deck for the mode.
    pub fn select_deck(&mut self, d: usize, decks: &[DeckInfo; 4]) -> Result<(), &'static str> {
        if d > 3 || !decks[d].loaded {
            return Ok(());
        }
        if !decks[d].has_grid {
            return Err(MSG_NO_GRID);
        }
        let sel = self.selectable(decks);
        match self.mode {
            MODE_IN => {
                if self.link_set[d] {
                    return Err(MSG_ALREADY_SET);
                }
                if !sel[d] {
                    return Err(if self.mix_out[d] == -1 { MSG_NEED_PAUSED } else { MSG_IN_ON_OUT_DECK });
                }
                // Another deck has the MIX IN point and it is in no link yet.
                let linked_as_in = self.as_in(d).0 == PAIR_LINKED;
                if let Some(i) = self.in_deck.filter(|i| *i != d && self.mix_in[*i] != -1 && !linked_as_in && !self.link_set[*i]) {
                    let _ = i;
                    return Err(MSG_CLEAR_MIX_IN);
                }
            }
            MODE_OUT => {
                if self.as_out(d).0 == PAIR_LINKED {
                    return Err(MSG_ALREADY_SET);
                }
                if !sel[d] {
                    return Err(MSG_NEED_PLAYING);
                }
                if self.out_deck.is_some_and(|o| o != d && self.mix_out[o] != -1 && self.as_out(o).0 != PAIR_LINKED) {
                    return Err(MSG_CLEAR_MIX_OUT);
                }
            }
            _ => return Ok(()),
        }
        self.selected = Some(d);
        self.cue_sel = None;
        self.guidance = if self.silent_edit[d] { 6 } else { self.base_guidance() };
        Ok(())
    }

    /// `getMixPointTimeByNonCue`: a plain click on the waveform at `t_ms` (`quantized_ms`:
    /// the same time moved to the grid when the deck's quantize is on).
    pub fn click_time(&self, d: usize, t_ms: i64, quantized_ms: Option<i64>, decks: &[DeckInfo; 4]) -> Result<i64, &'static str> {
        if t_ms < 0 || t_ms > decks[d].total_ms {
            return Err("");
        }
        if !self.silent_edit[d] && self.mode != MODE_IN && (self.mode != MODE_OUT || (self.mix_in[d] != -1 && self.mix_in[d] >= t_ms)) {
            return Err(MSG_OUT_AFTER_IN);
        }
        Ok(quantized_ms.unwrap_or(t_ms))
    }

    /// `setSilentEndTime`: with EDIT on, a click sets where muting ends, never after the MIX
    /// IN point.
    pub fn set_silent_end(&mut self, d: usize, t_ms: i64) {
        if self.mix_in[d] != -1 {
            self.silent_end[d] = t_ms.clamp(0, self.mix_in[d]);
        }
    }

    /// Set the point of the current mode on the selected deck (`setMixPointTime`,
    /// `isMixPointSetEnabled`). `by_select`: it came from MIX POINT SELECT, which does not
    /// switch the mode afterwards.
    pub fn set_point(&mut self, d: usize, t_ms: i64, by_select: bool, decks: &[DeckInfo; 4], prefs: &MplPrefs) -> Result<(), &'static str> {
        if !decks[d].has_grid {
            return Err(MSG_NO_GRID);
        }
        let t = t_ms.clamp(0, decks[d].total_ms.max(0));
        match self.mode {
            MODE_IN => {
                if self.link_set[d] {
                    return Err(if self.auto_started[d] { MSG_NOT_MOVED } else { MSG_ALREADY_SET });
                }
                if self.mix_out[d] != -1 {
                    return Err(MSG_IN_ON_OUT_DECK);
                }
                self.mix_in[d] = t;
                self.silent_end[d] = t;
                self.in_deck = Some(d);
            }
            MODE_OUT => {
                if self.as_out(d).0 == PAIR_LINKED {
                    return Err(MSG_ALREADY_SET);
                }
                if self.mix_in[d] != -1 && self.mix_in[d] >= t {
                    return Err(MSG_OUT_AFTER_IN);
                }
                self.mix_out[d] = t;
                self.out_deck = Some(d);
            }
            _ => return Ok(()),
        }
        if let (Some(o), Some(i)) = (self.out_deck, self.in_deck) {
            if o != i && self.mix_out[o] != -1 && self.mix_in[i] != -1 && self.pair[o][i] != PAIR_LINKED {
                self.pair[o][i] = PAIR_READY;
            }
        }
        if self.any_ready() {
            self.guidance = 5;
        } else if !by_select && prefs.auto_change {
            // "Automatically switches between MIX IN/MIX OUT".
            self.set_mode(if self.mode == MODE_IN { MODE_OUT } else { MODE_IN });
        }
        Ok(())
    }

    /// SILENT on/off for the selected deck.
    pub fn toggle_silent(&mut self, decks: &[DeckInfo; 4]) -> Result<bool, &'static str> {
        let Some(d) = self.selected.or(self.in_deck).filter(|d| decks[*d].loaded && self.mix_in[*d] != -1) else { return Err(MSG_SILENT_NEEDS_IN) };
        self.silent_on[d] = !self.silent_on[d];
        Ok(self.silent_on[d])
    }

    /// SILENT EDIT on/off: not once the deck is set to start by itself.
    pub fn toggle_silent_edit(&mut self) -> Result<bool, &'static str> {
        let Some(d) = self.selected.or(self.in_deck).filter(|d| self.mix_in[*d] != -1) else { return Err(MSG_SILENT_NEEDS_IN) };
        if self.link_set[d] {
            return Err(MSG_SILENT_EDIT_OFF);
        }
        self.silent_edit[d] = !self.silent_edit[d];
        self.guidance = if self.silent_edit[d] { 6 } else if self.any_ready() { 5 } else { self.base_guidance() };
        Ok(self.silent_edit[d])
    }

    /// `reqMixPointLink`: every pair with both points becomes linked. Returns the pairs.
    pub fn link(&mut self, prefs: &MplPrefs) -> Vec<(usize, usize)> {
        let mut out = vec![];
        for o in 0..4 {
            for i in 0..4 {
                if o != i && self.pair[o][i] == PAIR_READY {
                    self.pair[o][i] = PAIR_LINKED;
                    self.link_set[i] = true;
                    self.auto_started[i] = false;
                    self.reached[o] = false;
                    self.silent_edit[i] = false;
                    out.push((o, i));
                }
            }
        }
        if !out.is_empty() {
            self.in_deck = None;
            self.out_deck = None;
            self.set_mode(prefs.default_type);
        }
        out
    }

    /// Undo everything deck `d` takes part in with the point `which` (MODE_IN / MODE_OUT):
    /// the point, its pair and the link. Returns the MIX IN decks whose link ended.
    pub fn cancel(&mut self, d: usize, which: u8) -> Vec<usize> {
        let mut ended = vec![];
        if which == MODE_IN {
            for o in 0..4 {
                if self.pair[o][d] != PAIR_NONE {
                    self.pair[o][d] = PAIR_NONE;
                    // The MIX OUT point of a pair that is gone stays set, as a point.
                }
            }
            if self.link_set[d] {
                ended.push(d);
            }
            self.mix_in[d] = -1;
            self.silent_end[d] = -1;
            self.silent_on[d] = false;
            self.silent_edit[d] = false;
            self.link_set[d] = false;
            self.auto_started[d] = false;
            if self.in_deck == Some(d) {
                self.in_deck = None;
            }
        } else {
            for i in 0..4 {
                if self.pair[d][i] != PAIR_NONE {
                    self.pair[d][i] = PAIR_NONE;
                    if self.link_set[i] {
                        self.link_set[i] = false;
                        self.auto_started[i] = false;
                        ended.push(i);
                    }
                }
            }
            self.mix_out[d] = -1;
            self.reached[d] = false;
            if self.out_deck == Some(d) {
                self.out_deck = None;
            }
        }
        if self.selected == Some(d) {
            self.selected = None;
        }
        self.guidance = if self.any_ready() { 5 } else { self.base_guidance() };
        ended
    }

    /// `initDeckInfoOnLoaded`: another track on the deck clears all it had.
    pub fn on_load(&mut self, d: usize) -> Vec<usize> {
        let mut e = self.cancel(d, MODE_IN);
        e.extend(self.cancel(d, MODE_OUT));
        e
    }

    /// Everything off (Automix started, the function was disabled, PERFORMANCE mode left).
    pub fn cancel_all(&mut self) -> bool {
        let had = self.any_linked() || self.any_ready() || self.mix_in.iter().chain(self.mix_out.iter()).any(|t| *t != -1);
        let (shown, zoom, view, ui) = (self.shown, self.zoom, self.view_start, std::mem::take(&mut self.ui));
        *self = MplState { shown, zoom, view_start: view, ui, ..Default::default() };
        had
    }

    /// The MIX OUT deck passed its point: that link is over and its points are free again.
    pub fn on_reached(&mut self, out: usize) {
        for i in 0..4 {
            if self.pair[out][i] == PAIR_LINKED {
                self.pair[out][i] = PAIR_NONE;
                self.link_set[i] = false;
                self.auto_started[i] = false;
                self.mix_in[i] = -1;
                self.silent_on[i] = false;
                self.silent_end[i] = -1;
            }
        }
        self.mix_out[out] = -1;
        self.reached[out] = true;
        if self.out_deck == Some(out) {
            self.out_deck = None;
        }
        self.guidance = if self.any_ready() { 5 } else { self.base_guidance() };
    }

    /// `isMixPointLinkOffNeeded`: deck `d` changed how it plays (`started`: it began to
    /// play; `paused`: it stopped; otherwise it jumped or entered a loop). True when its
    /// link has to go.
    pub fn off_needed(&self, d: usize, started: bool, paused: bool, looping: bool) -> bool {
        let (as_out, partner) = self.as_out(d);
        let (as_in, _) = self.as_in(d);
        let partner_started = partner.is_some_and(|p| self.auto_started[p]);
        if started {
            // A linked MIX OUT deck that had stopped, a pair whose MIX IN deck already runs,
            // or the MIX IN deck started by hand before its time.
            return (as_out == PAIR_LINKED && self.mix_in[d] == -1) || partner_started || as_in == PAIR_LINKED;
        }
        if (as_out != PAIR_NONE && self.mix_in[d] == -1) || partner_started {
            return true;
        }
        if as_in != PAIR_LINKED {
            return false;
        }
        // The MIX IN deck of a set link: waiting paused (or looping) is fine until it has
        // been started; anything else ends the link.
        if !(paused || looping) {
            return true;
        }
        self.auto_started[d] && paused
    }
}

// ---------------------------------------------------------------- the link computation

/// `getRealMixPointTime`: the time `t_ms` of a track counted in beats of even length (the
/// grid's own BPM in the beat that holds it), then divided by the deck's speed.
pub fn real_time_ms(beats: &[Beat], t_ms: f64, tempo_pct: f64) -> f64 {
    let speed = if tempo_pct == -100.0 { 0.00010002 } else { 1.0 + tempo_pct * 0.01 };
    if beats.len() < 2 {
        return t_ms / speed;
    }
    // The beat interval that holds `t` (the first one for a time before the grid; the last
    // for a time after it, where rekordbox has no rule and returns 0).
    let i = beats.partition_point(|b| b.time * 1000.0 <= t_ms).saturating_sub(1).min(beats.len() - 2);
    let (b0, b1) = (beats[i].time * 1000.0, beats[i + 1].time * 1000.0);
    let nominal = if beats[i].bpm > 0.0 { 60000.0 / beats[i].bpm } else { b1 - b0 };
    let span = if b1 == b0 { nominal } else { b1 - b0 };
    ((t_ms - b0) * nominal / span + b0) / speed
}

/// `getMixPointLinkPlayStartTime`: where the MIX OUT deck is when the MIX IN deck has to
/// start from the top of its track for the two points to pass together.
pub fn start_out_ms(out_beats: &[Beat], mix_out_ms: f64, tempo_out: f64, in_beats: &[Beat], mix_in_ms: f64, tempo_in: f64) -> f64 {
    let k = if tempo_out == -100.0 { 0.010002 } else { tempo_out + 100.0 };
    (real_time_ms(out_beats, mix_out_ms, tempo_out) - real_time_ms(in_beats, mix_in_ms, tempo_in)) * 0.01 * k
}

// ---------------------------------------------------------------- the engine

/// One link as the audio thread holds it (indexed by the MIX IN deck).
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub active: bool,
    pub out_deck: u8,
    pub start_out_ms: f64,
    pub mix_out_ms: f64,
    pub mix_in_ms: f64,
    pub silent_on: bool,
    pub silent_end_ms: f64,
    /// Waiting for the MIX OUT deck to reach `start_out_ms`.
    pub waiting: bool,
    /// Started (or passed over): SILENT ends when the deck reaches its end point.
    pub started: bool,
    pub unmuted: bool,
    /// The MIX OUT deck has passed its point.
    pub done: bool,
    pub last_out_ms: f64,
    /// Where the deck was started (ms into its track), for the log and the tests.
    pub started_at_ms: f64,
}

impl Link {
    pub const OFF: Link = Link { active: false, out_deck: 0, start_out_ms: 0.0, mix_out_ms: 0.0, mix_in_ms: 0.0, silent_on: false, silent_end_ms: 0.0, waiting: false, started: false, unmuted: false, done: false, last_out_ms: 0.0, started_at_ms: -1.0 };
}

/// Notes from the audio thread to the interface, read and cleared by `MplState`'s poll.
pub const EV_STARTED: u8 = 1;
pub const EV_REACHED: u8 = 2;
pub const EV_PASSED: u8 = 4;

pub struct LinkEngine {
    pub links: [Link; 4],
    pub events: [u8; 4],
}

pub static ENGINE: parking_lot::Mutex<LinkEngine> = parking_lot::Mutex::new(LinkEngine { links: [Link::OFF; 4], events: [0; 4] });

fn pos_ms(s: &PlayState) -> Option<f64> {
    s.audio.as_ref().map(|a| s.pos / a.sr as f64 * 1000.0)
}

impl LinkEngine {
    /// Set or update the link whose MIX IN deck is `info.in_deck` (`updateMixPointLinkInfo`).
    /// `out_pos_ms`: where the MIX OUT deck is now.
    pub fn set(&mut self, info: &LinkInfo, out_pos_ms: f64) {
        let l = &mut self.links[info.in_deck.min(3)];
        let fresh = !l.active || l.out_deck as usize != info.out_deck;
        l.active = true;
        l.out_deck = info.out_deck as u8;
        l.start_out_ms = info.start_out_ms;
        l.mix_out_ms = info.mix_out_ms;
        l.mix_in_ms = info.mix_in_ms;
        l.silent_on = info.silent_on;
        l.silent_end_ms = info.silent_end_ms;
        if fresh {
            l.waiting = true;
            l.started = false;
            l.unmuted = false;
            l.done = false;
            l.last_out_ms = out_pos_ms;
            l.started_at_ms = -1.0;
        }
    }

    pub fn clear(&mut self, in_deck: usize) {
        self.links[in_deck.min(3)] = Link::OFF;
    }

    /// `DjPlayerMixPointLink::onPreRender`, once per audio block before the decks render.
    /// When the MIX OUT deck has reached the start position, the MIX IN deck starts this
    /// block; the block boundary rarely falls on the position itself, so the deck starts
    /// that much further in (`over`, scaled by the two decks' speeds), which puts it exactly
    /// where a start on the very frame would have. Nothing here allocates.
    pub fn run(&mut self, decks: &mut [Option<&mut PlayState>; 4]) {
        for i in 0..4 {
            let mut l = self.links[i];
            if !l.active {
                continue;
            }
            let o = (l.out_deck as usize).min(3);
            if o == i {
                continue;
            }
            let Some((out_ms, out_playing)) = decks[o].as_deref().and_then(|s| pos_ms(s).map(|p| (p, s.playing && s.search == 0.0))) else { continue };
            if l.waiting {
                if out_playing && out_ms >= l.start_out_ms {
                    if out_ms - l.last_out_ms <= 100.0 {
                        let over = out_ms - l.start_out_ms;
                        let start_in = if l.mix_out_ms != l.start_out_ms && l.mix_in_ms != 0.0 { over * l.mix_in_ms / (l.mix_out_ms - l.start_out_ms) } else { over };
                        l.waiting = false;
                        l.started = true;
                        if let Some(s) = decks[i].as_deref_mut() {
                            // (A deck already started by hand is left alone; the interface
                            // then ends the link.)
                            if !s.playing {
                                if let Some(sr) = s.audio.as_ref().map(|a| a.sr as f64) {
                                    s.pos = (start_in / 1000.0 * sr).max(0.0);
                                    s.loop_range = None;
                                    s.playing = true;
                                    l.started_at_ms = start_in;
                                }
                            }
                        }
                        self.events[i] |= EV_STARTED;
                    } else {
                        // The MIX OUT deck jumped past the start: no start.
                        l.waiting = false;
                        l.started = true;
                        self.events[i] |= EV_PASSED;
                    }
                }
                l.last_out_ms = out_ms;
            }
            // SILENT: muted from the link on, until the deck reaches the end point after it
            // has been started.
            if l.silent_on && !l.unmuted {
                if let Some(s) = decks[i].as_deref_mut() {
                    let at = pos_ms(s).unwrap_or(0.0);
                    if l.started && at >= l.silent_end_ms {
                        l.unmuted = true;
                        s.muted = false;
                    } else {
                        s.muted = true;
                    }
                }
            }
            if !l.done && out_ms >= l.mix_out_ms {
                l.done = true;
                self.events[o] |= EV_REACHED;
            }
            self.links[i] = l;
        }
    }
}

/// The hook of the audio callback: called with all decks locked, before they render.
pub fn pre_render(guards: &mut [Option<parking_lot::MutexGuard<'_, PlayState>>; 4]) {
    // (The interface holds this lock only to copy a few numbers; a block that finds it held
    // runs the links one block later, and the start is compensated for that too.)
    let Some(mut e) = ENGINE.try_lock() else { return };
    if !e.links.iter().any(|l| l.active) {
        return;
    }
    let mut it = guards.iter_mut();
    let mut decks: [Option<&mut PlayState>; 4] = [None, None, None, None];
    for d in decks.iter_mut() {
        *d = it.next().and_then(|g| g.as_deref_mut());
    }
    e.run(&mut decks);
}

// ---------------------------------------------------------------- the app's side

/// What the model needs to know about the four decks.
pub fn deck_infos(app: &App) -> [DeckInfo; 4] {
    let mut out = [DeckInfo::default(); 4];
    for d in 0..app.deck_count().min(4) as usize {
        let dk = app.deck_of(d as u8);
        let playing = app.player.decks[d].lock().playing;
        out[d] = DeckInfo { loaded: dk.track.is_some() && dk.audio.is_some(), has_grid: dk.beats.len() >= 2, playing, total_ms: (dk.duration * 1000.0) as i64 };
    }
    out
}

fn deck_pos_ms(app: &App, d: usize) -> f64 {
    let s = app.player.decks[d.min(3)].lock();
    pos_ms(&s).unwrap_or(0.0)
}

fn deck_tempo_pct(app: &App, d: usize) -> f64 {
    (app.player.decks[d.min(3)].lock().tempo - 1.0) * 100.0
}

/// The link's numbers for the engine, from where the decks are now.
pub fn link_info(app: &App, out: usize, inn: usize) -> LinkInfo {
    let m = &app.mixx.mpl;
    let (mo, mi) = (m.mix_out[out] as f64, m.mix_in[inn] as f64);
    let s = start_out_ms(&app.deck_of(out as u8).beats, mo, deck_tempo_pct(app, out), &app.deck_of(inn as u8).beats, mi, deck_tempo_pct(app, inn));
    LinkInfo { out_deck: out, in_deck: inn, start_out_ms: s, mix_out_ms: mo, mix_in_ms: mi, silent_on: m.silent_on[inn], silent_end_ms: m.silent_end[inn].max(0) as f64 }
}

/// End the links of these MIX IN decks in the engine and take their mute off.
fn engine_clear(app: &mut App, ended: &[usize]) {
    if ended.is_empty() {
        return;
    }
    let mut e = ENGINE.lock();
    for d in ended {
        e.clear(*d);
    }
    drop(e);
    let muted = app.px.muted;
    for d in ended {
        app.player.decks[(*d).min(3)].lock().muted = muted;
    }
}

/// Automix has started (or something else that MIX POINT LINK cannot run beside): all links
/// off, with rekordbox's note when there was one.
pub fn finish_for(app: &mut App, note: &str) {
    let linked = app.mixx.mpl.any_linked();
    let ended: Vec<usize> = (0..4).filter(|d| app.mixx.mpl.link_set[*d]).collect();
    app.mixx.mpl.cancel_all();
    engine_clear(app, &ended);
    if linked {
        app.toast(note);
    }
    app.mixx.mpl.shown = false;
}

impl App {
    /// The LINK button of the top bar: show or hide the strip.
    pub fn mpl_toggle_panel(&mut self) {
        if self.mixx.mpl.shown {
            self.mixx.mpl.shown = false;
            return;
        }
        if !self.prefs.mixx.mpl.enable {
            return;
        }
        if self.automix_processing() {
            self.toast_err(MSG_PANEL_AUTOMIX);
            return;
        }
        let m = &mut self.mixx.mpl;
        m.shown = true;
        if m.mode == MODE_NONE {
            let t = self.prefs.mixx.mpl.default_type;
            m.set_mode(if t == MODE_IN { MODE_IN } else { MODE_OUT });
            // (rekordbox shows no guidance until something is clicked in the strip.)
            m.guidance = 0;
        }
    }

    pub fn mpl_set_mode(&mut self, mode: u8) {
        self.mixx.mpl.set_mode(mode);
    }

    fn mpl_refuse(&mut self, r: Result<(), &'static str>) -> bool {
        match r {
            Ok(()) => true,
            Err(m) => {
                if !m.is_empty() {
                    self.toast_err(m);
                }
                false
            }
        }
    }

    /// A click on deck `d`'s waveform at `t` seconds; `cue`: it was on a cue marker (the
    /// cue's own time, not quantized).
    pub fn mpl_wave_click(&mut self, d: usize, t: f64, cue: bool) {
        if self.mixx.mpl.move_mode {
            return;
        }
        if self.automix_processing() {
            self.toast_err(MSG_FN_AUTOMIX);
            return;
        }
        let decks = deck_infos(self);
        if self.mixx.mpl.selected != Some(d) {
            let r = self.mixx.mpl.select_deck(d, &decks);
            self.mpl_refuse(r);
            return;
        }
        let t_ms = (t * 1000.0).round() as i64;
        if self.mixx.mpl.silent_edit[d] {
            self.mixx.mpl.set_silent_end(d, t_ms);
            self.mpl_resend();
            return;
        }
        let ms = if cue {
            t_ms
        } else {
            // Quantize of that deck: the point goes to the grid.
            let q = self.with_deck(d as u8, |a| if a.prefs.quantize { Some((a.quantized(t) * 1000.0).round() as i64) } else { None });
            match self.mixx.mpl.click_time(d, t_ms, q, &decks) {
                Ok(v) => v,
                Err(m) => {
                    self.mpl_refuse(Err(m));
                    return;
                }
            }
        };
        self.mpl_set_point(d, ms, false);
    }

    fn mpl_set_point(&mut self, d: usize, ms: i64, by_select: bool) {
        let decks = deck_infos(self);
        let prefs = self.prefs.mixx.mpl.clone();
        let r = self.mixx.mpl.set_point(d, ms, by_select, &decks, &prefs);
        self.mpl_refuse(r);
    }

    /// MIX POINT SELECT ‹ ›: the selected deck's hot cues and memory cues in time order,
    /// one after the other; a track without cues gives its current cue.
    pub fn mpl_select_cue(&mut self, dir: i32) {
        let Some(d) = self.mixx.mpl.selected else {
            let g = self.mixx.mpl.guidance;
            let _ = g;
            return;
        };
        let (mut times, cue_point): (Vec<f64>, f64) = {
            let dk = self.deck_of(d as u8);
            (dk.cues.iter().map(|c| c.start).collect(), dk.cue_point)
        };
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let t = if times.is_empty() {
            self.mixx.mpl.cue_sel = None;
            cue_point
        } else {
            let n = times.len() as i32;
            let k = match self.mixx.mpl.cue_sel {
                Some(k) => (k as i32 + dir.signum()).rem_euclid(n),
                None if dir < 0 => n - 1,
                None => 0,
            };
            self.mixx.mpl.cue_sel = Some(k as usize);
            times[k as usize]
        };
        let keep = self.mixx.mpl.cue_sel;
        self.mpl_set_point(d, (t * 1000.0).round() as i64, true);
        self.mixx.mpl.cue_sel = keep;
    }

    pub fn mpl_toggle_silent(&mut self) {
        let decks = deck_infos(self);
        match self.mixx.mpl.toggle_silent(&decks) {
            Ok(_) => self.mpl_resend(),
            Err(m) => self.toast_err(m),
        }
    }

    pub fn mpl_toggle_silent_edit(&mut self) {
        if let Err(m) = self.mixx.mpl.toggle_silent_edit() {
            self.toast_err(m);
        }
    }

    /// The MIX POINT LINK button: link every pair that has both points
    /// (`doMixPointLinkStart` 0x1014fa050).
    pub fn mpl_request_link(&mut self) {
        if self.deck_options.disable_play {
            return self.toast_err("Mix Point Link playback is disabled for this session (--disable-play).");
        }
        if self.automix_processing() {
            self.toast_err(MSG_FN_AUTOMIX);
            return;
        }
        let prefs = self.prefs.mixx.mpl.clone();
        let pairs = self.mixx.mpl.link(&prefs);
        for (o, i) in pairs {
            let mi = self.mixx.mpl.mix_in[i] as f64 / 1000.0;
            // The MIX IN deck waits on its MIX IN point, which becomes its cue.
            let was_sync = self.deck_of(i as u8).x.beat_sync;
            self.with_deck(i as u8, |a| {
                if (a.deck.cue_point - mi).abs() > 1e-4 {
                    a.deck.cue_point = mi;
                }
                if !a.playing() {
                    a.seek(mi);
                }
            });
            // `doMixPointLinkStart` 0x1014fa050: a MIX IN deck in BEAT SYNC goes to BPM SYNC
            // (tempo only) with the MIX OUT deck as master unless both points sit on beats,
            // or, with "Automatically changes the BEAT/BPM SYNC setting" on, also when the
            // MIX OUT deck's grid has more than one BPM.
            if was_sync {
                let on_beat = |app: &App, d: usize, ms: i64| app.deck_of(d as u8).beats.iter().any(|b| (b.time * 1000.0 - ms as f64).abs() < 1.0);
                let both = on_beat(self, o, self.mixx.mpl.mix_out[o]) && on_beat(self, i, self.mixx.mpl.mix_in[i]);
                if needs_bpm_sync(both, variable_bpm(&self.deck_of(o as u8).beats), prefs.bpm_sync_auto) {
                    self.px.master = o as u8;
                    self.mixx.mpl.bpm_sync[i] = true;
                    self.push_deck_controls();
                }
            }
            let info = link_info(self, o, i);
            let at = deck_pos_ms(self, o);
            ENGINE.lock().set(&info, at);
        }
    }

    /// Send the links' numbers again (a tempo moved, SILENT changed).
    fn mpl_resend(&mut self) {
        for i in 0..4 {
            if !self.mixx.mpl.link_set[i] {
                continue;
            }
            if let (PAIR_LINKED, Some(o)) = self.mixx.mpl.as_in(i) {
                let info = link_info(self, o, i);
                let at = deck_pos_ms(self, o);
                ENGINE.lock().set(&info, at);
            }
        }
    }

    /// A lit marker was clicked, or a readout's cross: that point and its link go.
    pub fn mpl_cancel(&mut self, d: usize, which: u8) {
        let ended = self.mixx.mpl.cancel(d, which);
        engine_clear(self, &ended);
    }

    /// Once per frame: what the audio thread reported, and the changes that end a link.
    pub fn mpl_poll(&mut self) {
        // BPM SYNC from a link lasts until the deck's sync is turned off.
        for d in 0..4 {
            if self.mixx.mpl.bpm_sync[d] && (d >= self.deck_count() as usize || !self.deck_of(d as u8).x.beat_sync) {
                self.mixx.mpl.bpm_sync[d] = false;
            }
        }
        let active = self.mixx.mpl.shown || self.mixx.mpl.any_linked() || self.mixx.mpl.any_ready() || self.mixx.mpl.mix_in.iter().chain(self.mixx.mpl.mix_out.iter()).any(|t| *t != -1);
        if !active {
            self.mixx.mpl.seen_at = None;
            return;
        }
        if !self.performance() || !self.prefs.mixx.mpl.enable {
            finish_for(self, MSG_DISABLED);
            return;
        }
        // The audio thread's notes.
        let ev = {
            let mut e = ENGINE.lock();
            std::mem::take(&mut e.events)
        };
        for d in 0..4 {
            if ev[d] & (EV_STARTED | EV_PASSED) != 0 && self.mixx.mpl.link_set[d] {
                self.mixx.mpl.auto_started[d] = true;
                if ev[d] & EV_PASSED != 0 {
                    self.mpl_cancel(d, MODE_IN);
                }
            }
            if ev[d] & EV_REACHED != 0 {
                let ins: Vec<usize> = (0..4).filter(|i| self.mixx.mpl.pair[d][*i] == PAIR_LINKED).collect();
                self.mixx.mpl.on_reached(d);
                engine_clear(self, &ins);
            }
        }
        // Loads, starts and stops, jumps and tempo changes since the last frame.
        let now = std::time::Instant::now();
        let dt = self.mixx.mpl.seen_at.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        let first = self.mixx.mpl.seen_at.is_none();
        self.mixx.mpl.seen_at = Some(now);
        let mut resend = false;
        for d in 0..self.deck_count().min(4) as usize {
            let track = self.deck_of(d as u8).track;
            let (pos, playing, tempo, looping) = {
                let s = self.player.decks[d].lock();
                (pos_ms(&s).unwrap_or(0.0), s.playing, s.tempo, s.loop_range.is_some())
            };
            let m = &self.mixx.mpl;
            let (seen_track, seen_pos, seen_playing, seen_tempo) = (m.seen_track[d], m.seen_pos[d], m.seen_playing[d], m.seen_tempo[d]);
            let in_link = m.as_out(d).0 != PAIR_NONE || m.as_in(d).0 != PAIR_NONE;
            let mut off = false;
            if !first {
                if track != seen_track {
                    let ended = self.mixx.mpl.on_load(d);
                    engine_clear(self, &ended);
                } else if in_link {
                    let m = &self.mixx.mpl;
                    let just_auto = ev[d] & EV_STARTED != 0;
                    if playing != seen_playing && !just_auto {
                        off = m.off_needed(d, playing, !playing, looping);
                    } else if playing && !just_auto {
                        // A jump: the position is not where this frame's time puts it.
                        let expect = seen_pos + dt * 1000.0 * tempo;
                        if (pos - expect).abs() > 250.0 + dt * 200.0 {
                            off = m.off_needed(d, false, false, looping);
                        }
                    } else if !playing && (pos - seen_pos).abs() > 1.0 && m.as_in(d).0 == PAIR_LINKED && !m.auto_started[d] {
                        // The waiting MIX IN deck was moved off its point.
                        off = (pos - m.mix_in[d] as f64).abs() > 1.0;
                    }
                    if (tempo - seen_tempo).abs() > 1e-9 {
                        // A tempo moved by hand: the link goes when the MIX IN deck already
                        // runs; otherwise the start position is worked out again.
                        let m = &self.mixx.mpl;
                        let inn = if m.as_in(d).0 == PAIR_LINKED { Some(d) } else { m.as_out(d).1.filter(|_| m.as_out(d).0 == PAIR_LINKED) };
                        if inn.is_some_and(|i| m.auto_started[i]) {
                            off = true;
                        } else {
                            resend = true;
                        }
                    }
                }
            }
            if off {
                let which = if self.mixx.mpl.as_in(d).0 != PAIR_NONE { MODE_IN } else { MODE_OUT };
                self.mpl_cancel(d, which);
            }
            let m = &mut self.mixx.mpl;
            m.seen_track[d] = track;
            m.seen_pos[d] = pos;
            m.seen_playing[d] = playing;
            m.seen_tempo[d] = tempo;
        }
        if resend {
            self.mpl_resend();
        }
        if self.mixx.mpl.any_linked() {
            self.ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn link_switches_to_bpm_sync_as_rekordbox_does() {
        // Both points on beats and a constant grid: BEAT SYNC stays.
        assert!(!needs_bpm_sync(true, false, false));
        assert!(!needs_bpm_sync(true, false, true));
        // A point off the beat: BPM SYNC, whatever the setting.
        assert!(needs_bpm_sync(false, false, false));
        // A grid with more than one BPM: only with "Automatically changes ..." on.
        assert!(!needs_bpm_sync(true, true, false));
        assert!(needs_bpm_sync(true, true, true));
        let b = |t: f64, bpm: f64| Beat { time: t, bpm, beat: 1, index: 0 };
        assert!(!variable_bpm(&[b(0.0, 128.0), b(0.47, 128.0)]));
        assert!(variable_bpm(&[b(0.0, 128.0), b(0.47, 127.5)]));
    }

    #[test]
    fn bar_count_runs_from_the_mix_in_point() {
        let beats: Vec<Beat> = (0..40).map(|i| Beat { time: i as f64 * 0.5, bpm: 120.0, beat: (i % 4 + 1) as u8, index: i }).collect();
        // MIX IN on beat 8 (4.0 s): there it is 1.1, three beats before -1.2, one bar on 2.1.
        assert_eq!(mix_point_bars(&beats, 4.0, 4.0), "1.1Bars");
        assert_eq!(mix_point_bars(&beats, 4.0, 2.5), "-1.2Bars");
        assert_eq!(mix_point_bars(&beats, 4.0, 3.6), "-1.4Bars");
        assert_eq!(mix_point_bars(&beats, 4.0, 1.9), "-2.4Bars");
        assert_eq!(mix_point_bars(&beats, 4.0, 6.1), "2.1Bars");
        assert_eq!(mix_point_bars(&[], 4.0, 6.1), "");
    }

    use super::*;
    use std::sync::Arc;

    fn grid(bpm: f64, secs: f64, offset: f64) -> Vec<Beat> {
        let n = (secs * bpm / 60.0) as usize;
        (0..n).map(|i| Beat { time: offset + i as f64 * 60.0 / bpm, bpm, beat: (i % 4) as u8 + 1, index: i }).collect()
    }

    fn decks(playing: [bool; 4]) -> [DeckInfo; 4] {
        std::array::from_fn(|d| DeckInfo { loaded: d < 2, has_grid: d < 2, playing: playing[d], total_ms: 300_000 })
    }

    #[test]
    fn selectable_decks_by_mode() {
        let mut m = MplState::default();
        let dk = decks([true, false, false, false]);
        m.set_mode(MODE_OUT);
        assert_eq!(m.selectable(&dk), [true, false, false, false], "MIX OUT: the playing deck");
        m.set_mode(MODE_IN);
        assert_eq!(m.selectable(&dk), [false, true, false, false], "MIX IN: the paused deck");
        // A deck with a MIX OUT point cannot take the MIX IN point, paused or not.
        m.mix_out[1] = 5000;
        assert_eq!(m.selectable(&decks([false; 4])), [true, false, false, false]);
    }

    #[test]
    fn every_refusal_of_deck_and_point() {
        let prefs = MplPrefs { auto_change: false, ..Default::default() };
        let dk = decks([true, false, false, false]);
        // No beat grid.
        let mut m = MplState::default();
        m.set_mode(MODE_OUT);
        let mut nogrid = dk;
        nogrid[0].has_grid = false;
        assert_eq!(m.select_deck(0, &nogrid), Err(MSG_NO_GRID));
        // MIX OUT on a paused deck; MIX IN on a playing one.
        assert_eq!(m.select_deck(1, &dk), Err(MSG_NEED_PLAYING));
        m.set_mode(MODE_IN);
        assert_eq!(m.select_deck(0, &dk), Err(MSG_NEED_PAUSED));
        // The normal order: MIX OUT on deck 1, MIX IN on deck 2.
        m.set_mode(MODE_OUT);
        assert_eq!(m.select_deck(0, &dk), Ok(()));
        assert_eq!(m.guidance, 4);
        assert_eq!(m.set_point(0, 120_000, false, &dk, &prefs), Ok(()));
        // MIX IN on the deck that has the MIX OUT point.
        m.set_mode(MODE_IN);
        assert_eq!(m.select_deck(0, &decks([false; 4])), Err(MSG_IN_ON_OUT_DECK));
        assert_eq!(m.select_deck(1, &dk), Ok(()));
        assert_eq!(m.guidance, 3);
        assert_eq!(m.set_point(1, 16_000, false, &dk, &prefs), Ok(()));
        assert_eq!(m.pair[0][1], PAIR_READY);
        assert_eq!(m.guidance, 5, "both points: press MIX POINT LINK");
        // A second MIX OUT deck while deck 1 holds the point; a second MIX IN deck likewise.
        let three: [DeckInfo; 4] = std::array::from_fn(|d| DeckInfo { loaded: d < 3, has_grid: d < 3, playing: d == 0 || d == 2, total_ms: 300_000 });
        m.set_mode(MODE_OUT);
        assert_eq!(m.select_deck(2, &three), Err(MSG_CLEAR_MIX_OUT));
        let three_paused: [DeckInfo; 4] = std::array::from_fn(|d| DeckInfo { loaded: d < 3, has_grid: d < 3, playing: d == 0, total_ms: 300_000 });
        // (With a pair ready, later decks are not offered for MIX IN at all.)
        m.set_mode(MODE_IN);
        assert_eq!(m.select_deck(2, &three_paused), Err(MSG_NEED_PAUSED));
        let mut two_in = MplState::default();
        two_in.set_mode(MODE_IN);
        two_in.select_deck(1, &three_paused).unwrap();
        two_in.set_point(1, 16_000, false, &three_paused, &prefs).unwrap();
        assert_eq!(two_in.select_deck(2, &three_paused), Err(MSG_CLEAR_MIX_IN));
        // A click in MIX OUT mode before the deck's own MIX IN point.
        let mut c = MplState::default();
        c.set_mode(MODE_OUT);
        c.mix_in[0] = 30_000;
        assert_eq!(c.click_time(0, 20_000, None, &dk), Err(MSG_OUT_AFTER_IN));
        assert_eq!(c.click_time(0, 30_000, None, &dk), Err(MSG_OUT_AFTER_IN));
        assert_eq!(c.click_time(0, 40_000, None, &dk), Ok(40_000));
        assert_eq!(c.click_time(0, 40_000, Some(40_250), &dk), Ok(40_250), "quantize moves the point to the grid");
        assert_eq!(c.set_point(0, 20_000, false, &dk, &prefs), Err(MSG_OUT_AFTER_IN));
        // Linked: neither point moves any more.
        assert_eq!(m.link(&prefs), vec![(0, 1)]);
        assert_eq!(m.pair[0][1], PAIR_LINKED);
        assert!(m.link_set[1]);
        assert_eq!(m.mode, MODE_OUT, "after the link the mode is the default one");
        assert_eq!(m.select_deck(0, &dk), Err(MSG_ALREADY_SET));
        m.set_mode(MODE_IN);
        assert_eq!(m.select_deck(1, &dk), Err(MSG_ALREADY_SET));
        assert_eq!(m.set_point(1, 10_000, false, &dk, &prefs), Err(MSG_ALREADY_SET));
        m.auto_started[1] = true;
        assert_eq!(m.set_point(1, 10_000, false, &dk, &prefs), Err(MSG_NOT_MOVED));
        // SILENT and EDIT.
        let mut s = MplState::default();
        s.set_mode(MODE_IN);
        assert_eq!(s.toggle_silent(&dk), Err(MSG_SILENT_NEEDS_IN));
        assert_eq!(s.toggle_silent_edit(), Err(MSG_SILENT_NEEDS_IN));
        s.select_deck(1, &dk).unwrap();
        s.set_point(1, 16_000, false, &dk, &prefs).unwrap();
        assert_eq!(s.toggle_silent(&dk), Ok(true));
        assert_eq!(s.silent_end[1], 16_000, "muting ends on the MIX IN point by default");
        assert_eq!(s.toggle_silent_edit(), Ok(true));
        assert_eq!(s.guidance, 6);
        s.set_silent_end(1, 8000);
        assert_eq!(s.silent_end[1], 8000);
        s.set_silent_end(1, 20_000);
        assert_eq!(s.silent_end[1], 16_000, "never after the MIX IN point");
        s.link_set[1] = true;
        s.silent_edit[1] = false;
        assert_eq!(s.toggle_silent_edit(), Err(MSG_SILENT_EDIT_OFF));
    }

    #[test]
    fn mode_switches_by_itself_and_points_clamp() {
        let prefs = MplPrefs::default();
        let dk = decks([true, false, false, false]);
        let mut m = MplState::default();
        m.set_mode(MODE_OUT);
        m.select_deck(0, &dk).unwrap();
        m.set_point(0, 999_000, false, &dk, &prefs).unwrap();
        assert_eq!(m.mix_out[0], 300_000, "clamped to the track");
        assert_eq!((m.mode, m.selected, m.guidance), (MODE_IN, None, 1), "the mode went to MIX IN");
        m.select_deck(1, &dk).unwrap();
        // MIX POINT SELECT does not switch.
        m.set_point(1, 8000, true, &dk, &prefs).unwrap();
        assert_eq!(m.mode, MODE_IN);
        assert_eq!(m.pair[0][1], PAIR_READY);
    }

    #[test]
    fn cancel_rules() {
        let prefs = MplPrefs { auto_change: false, ..Default::default() };
        let dk = decks([true, false, false, false]);
        let linked = || {
            let mut m = MplState::default();
            m.set_mode(MODE_OUT);
            m.select_deck(0, &dk).unwrap();
            m.set_point(0, 120_000, false, &dk, &prefs).unwrap();
            m.set_mode(MODE_IN);
            m.select_deck(1, &dk).unwrap();
            m.set_point(1, 16_000, false, &dk, &prefs).unwrap();
            m.link(&prefs);
            m
        };
        let m = linked();
        // The MIX OUT deck pauses, jumps or loops: off. The MIX IN deck waiting paused: kept.
        assert!(m.off_needed(0, false, true, false));
        assert!(m.off_needed(0, false, false, false));
        assert!(m.off_needed(0, false, false, true));
        assert!(!m.off_needed(1, false, true, false));
        // The MIX IN deck started by hand before its time: off.
        assert!(m.off_needed(1, true, false, false));
        // After the automatic start: pausing the MIX IN deck ends the link.
        let mut a = linked();
        a.auto_started[1] = true;
        assert!(a.off_needed(1, false, true, false));
        assert!(a.off_needed(0, false, true, false));
        // A lit marker clicked: the point and the link go, the other point stays.
        let mut c = linked();
        assert_eq!(c.cancel(1, MODE_IN), vec![1]);
        assert_eq!((c.mix_in[1], c.mix_out[0], c.pair[0][1], c.link_set[1]), (-1, 120_000, PAIR_NONE, false));
        let mut c = linked();
        assert_eq!(c.cancel(0, MODE_OUT), vec![1]);
        assert_eq!((c.mix_in[1], c.mix_out[0], c.pair[0][1], c.link_set[1]), (16_000, -1, PAIR_NONE, false));
        // Another track on a deck clears what it had.
        let mut c = linked();
        assert_eq!(c.on_load(0), vec![1]);
        assert!(!c.any_linked());
        // The MIX OUT point passed: the link is over and both points are free.
        let mut c = linked();
        c.on_reached(0);
        assert_eq!((c.mix_in[1], c.mix_out[0], c.any_linked(), c.reached[0]), (-1, -1, false, true));
        // Automix: everything off.
        let mut c = linked();
        assert!(c.cancel_all());
        assert!(!c.any_linked() && c.mix_out[0] == -1);
    }

    #[test]
    fn link_computation() {
        let out = grid(125.0, 300.0, 0.0);
        let inn = grid(125.0, 300.0, 0.0);
        // Same tempo: the MIX IN deck starts when the MIX OUT deck is mixIn before its point.
        assert!((start_out_ms(&out, 120_000.0, 0.0, &inn, 16_000.0, 0.0) - 104_000.0).abs() < 1e-6);
        // Decks at other speeds: mixOut - mixIn * r_out / r_in.
        let s = start_out_ms(&out, 120_000.0, 4.0, &inn, 16_000.0, -2.0);
        assert!((s - (120_000.0 - 16_000.0 * 1.04 / 0.98)).abs() < 1e-6, "{s}");
        // Decks of different BPM, the MIX IN deck synced to the other's tempo (128 on 125).
        let inn2 = grid(128.0, 300.0, 0.1);
        let pct = (125.0 / 128.0 - 1.0) * 100.0;
        let s = start_out_ms(&out, 96_000.0, 0.0, &inn2, 15_100.0, pct);
        assert!((s - (96_000.0 - 15_100.0 * 128.0 / 125.0)).abs() < 1e-6, "{s}");
        // A beat that is longer than its BPM says counts as one beat of the BPM's length.
        let mut warped = grid(120.0, 10.0, 0.0);
        for b in warped.iter_mut().skip(5) {
            b.time += 0.1;
        }
        // Beat 5 (index 4) lasts 600 ms instead of 500: its middle is 250 nominal ms in.
        assert!((real_time_ms(&warped, 2300.0, 0.0) - 2250.0).abs() < 1e-6);
        assert!((real_time_ms(&warped, 2000.0, 0.0) - 2000.0).abs() < 1e-6);
        // Before the first beat the first interval is used.
        let late = grid(120.0, 10.0, 1.0);
        assert!((real_time_ms(&late, 500.0, 0.0) - 500.0).abs() < 1e-6);
    }

    fn silence(secs: f64, sr: u32) -> Arc<crate::audio::Decoded> {
        Arc::new(crate::audio::Decoded { sr, samples: vec![0.0; (secs * sr as f64) as usize * 2], raw_frames: (secs * sr as f64) as usize, damaged: 0, truncated: false })
    }

    fn deck(sr: u32, playing: bool, tempo_pct: f64) -> PlayState {
        let mut s = PlayState::default();
        s.audio = Some(silence(20.0, sr));
        s.playing = playing;
        s.tempo = 1.0 + tempo_pct / 100.0;
        s
    }

    /// Run two decks through the real deck renderer in blocks of `block` frames with the
    /// link engine at the top of each block, until the MIX OUT deck passes its point.
    /// Returns (the MIX IN deck's position in ms when the MIX OUT deck stands exactly on
    /// its point, worked back from the end of the run; the engine's link; events).
    fn run_link(block: usize, rate: u32, tempo_out: f64, tempo_in: f64, mix_out: f64, mix_in: f64, silent: Option<f64>) -> (f64, Link, [u8; 4], bool) {
        let (sr_out, sr_in) = (44100u32, 48000u32);
        let mut out = deck(sr_out, true, tempo_out);
        let mut inn = deck(sr_in, false, tempo_in);
        inn.pos = mix_in / 1000.0 * sr_in as f64;
        let ob = grid(120.0, 20.0, 0.0);
        let ib = grid(120.0, 20.0, 0.0);
        let start = start_out_ms(&ob, mix_out, tempo_out, &ib, mix_in, tempo_in);
        let mut e = LinkEngine { links: [Link::OFF; 4], events: [0; 4] };
        e.set(&LinkInfo { out_deck: 0, in_deck: 1, start_out_ms: start, mix_out_ms: mix_out, mix_in_ms: mix_in, silent_on: silent.is_some(), silent_end_ms: silent.unwrap_or(0.0) }, 0.0);
        let mut buf = vec![0.0f32; block * 2];
        let mut muted_at_start = false;
        let mut seen_start = false;
        for _ in 0..(20 * rate as usize / block) {
            {
                let mut d: [Option<&mut PlayState>; 4] = [Some(&mut out), Some(&mut inn), None, None];
                e.run(&mut d);
            }
            if !seen_start && inn.playing {
                seen_start = true;
                muted_at_start = inn.muted;
            }
            for v in buf.iter_mut() {
                *v = 0.0;
            }
            crate::audio::render_deck(&mut out, &mut buf, 2, rate);
            crate::audio::render_deck(&mut inn, &mut buf, 2, rate);
            if e.links[1].done {
                break;
            }
        }
        let out_ms = out.pos / sr_out as f64 * 1000.0;
        let in_ms = inn.pos / sr_in as f64 * 1000.0;
        // Both decks ran on from the MIX OUT point by the same stretch of output time.
        let past = (out_ms - mix_out) / (1.0 + tempo_out / 100.0);
        (in_ms - past * (1.0 + tempo_in / 100.0), e.links[1], e.events, muted_at_start)
    }

    #[test]
    fn auto_start_lands_on_the_exact_frame_for_any_buffer_size() {
        // One frame of the MIX IN deck is 1000 / 48000 = 0.0208 ms.
        let frame = 1000.0 / 48000.0;
        for rate in [44100u32, 48000] {
            for block in [64usize, 128, 256, 441, 512, 1024, 2048, 4096] {
                for (to, ti) in [(0.0, 0.0), (3.0, -2.5), (-6.0, 8.0)] {
                    let (at, link, ev, _) = run_link(block, rate, to, ti, 9000.0, 4000.0, None);
                    assert!(link.started_at_ms >= 0.0, "started (block {block})");
                    assert!(ev[1] & EV_STARTED != 0 && ev[0] & EV_REACHED != 0);
                    assert!((at - 4000.0).abs() <= frame, "block {block} at {rate} Hz, tempos {to} / {ti}: MIX IN deck at {at:.4} ms on the MIX OUT point (start {:.3} ms in)", link.started_at_ms);
                    // The start is late by less than one block, and made up for.
                    let block_ms = block as f64 / rate as f64 * 1000.0 * (1.0 + ti / 100.0);
                    assert!(link.started_at_ms <= block_ms + 1e-6, "{} <= {block_ms}", link.started_at_ms);
                }
            }
        }
    }

    #[test]
    fn silent_mutes_until_the_end_point_and_a_jump_cancels_the_start() {
        // SILENT: muted from the start of the track to the end point (here the MIX IN point).
        let (_, link, _, muted_at_start) = run_link(512, 48000, 0.0, 0.0, 9000.0, 4000.0, Some(4000.0));
        assert!(muted_at_start, "muted when it starts");
        assert!(link.unmuted, "audible from the end point on");
        // A MIX OUT deck that jumps over the start position: the other deck is not started.
        let mut out = deck(44100, true, 0.0);
        let mut inn = deck(44100, false, 0.0);
        let mut e = LinkEngine { links: [Link::OFF; 4], events: [0; 4] };
        e.set(&LinkInfo { out_deck: 0, in_deck: 1, start_out_ms: 5000.0, mix_out_ms: 9000.0, mix_in_ms: 4000.0, silent_on: false, silent_end_ms: 0.0 }, 0.0);
        out.pos = 1.0 * 44100.0;
        e.run(&mut [Some(&mut out), Some(&mut inn), None, None]);
        out.pos = 7.0 * 44100.0;
        e.run(&mut [Some(&mut out), Some(&mut inn), None, None]);
        assert!(!inn.playing && e.events[1] & EV_PASSED != 0);
        // A start position already behind the MIX OUT deck when the link is set: the other
        // deck starts at once, as far in as it would be by now.
        let mut out = deck(44100, true, 0.0);
        let mut inn = deck(44100, false, 0.0);
        out.pos = 6.0 * 44100.0;
        let mut e = LinkEngine { links: [Link::OFF; 4], events: [0; 4] };
        e.set(&LinkInfo { out_deck: 0, in_deck: 1, start_out_ms: 5000.0, mix_out_ms: 9000.0, mix_in_ms: 4000.0, silent_on: false, silent_end_ms: 0.0 }, 6000.0);
        e.run(&mut [Some(&mut out), Some(&mut inn), None, None]);
        assert!(inn.playing);
        assert!((inn.pos / 44.1 - 1000.0).abs() < 1e-6, "1 s late: 1 s in ({})", inn.pos / 44.1);
        // A MIX IN deck started by hand is left alone.
        let mut out = deck(44100, true, 0.0);
        let mut inn = deck(44100, true, 0.0);
        inn.pos = 123.0;
        out.pos = 5.0 * 44100.0;
        let mut e = LinkEngine { links: [Link::OFF; 4], events: [0; 4] };
        e.set(&LinkInfo { out_deck: 0, in_deck: 1, start_out_ms: 5000.0, mix_out_ms: 9000.0, mix_in_ms: 4000.0, silent_on: false, silent_end_ms: 0.0 }, 4990.0);
        e.run(&mut [Some(&mut out), Some(&mut inn), None, None]);
        assert_eq!(inn.pos, 123.0);
    }
}
