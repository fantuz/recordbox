//! The FX panel of PERFORMANCE mode: FX1 and FX2, each with its assign buttons, multi mode
//! (three effect slots: name = on/off, list, LEVEL/DEPTH, `<` value `>`) or single mode (one
//! effect with LEVEL/DEPTH, `<` value `>`, two knobs, two buttons and a snapshot), RELEASE FX
//! (press and hold) and the FX tempo (AUTO / TAP); under it the CFX row in single or multi
//! mode. Positions measured on rekordbox 7.2.19 at 1512 pt
//! (`rekordbox-reference/performance/06-fx.jpg`): the row is 52 pt tall, a unit 756 pt wide.
//! That reference shows multi mode and CFX single mode; the single-mode slot and the CFX
//! multi-mode row are laid out here without a reference.
//!
//! The panel's choices are kept between sessions (`FxSaved`, in recordbox's own settings),
//! holding what rekordbox keeps in its FxUnitSettings file.

use crate::app::*;
use crate::fx::{self, Kind, UnitParams};
use crate::theme::*;
use crate::ui_player::{lighten, tip};
use eframe::egui;
use egui::{pos2, vec2, Align2, FontId, Rect, Sense, Stroke, Ui};
use serde::{Deserialize, Serialize};

/// Height of the FX panel: the Beat FX row (52 pt) and the CFX row (26 pt).
pub const FX_H: f32 = 78.0;
/// MERGE FX's row under the CFX row (rekordbox shows it as a further row; its own height was
/// not traced, so it takes the CFX row's 24 pt and the 3 pt gap above it).
pub const MFX_ROW_H: f32 = 27.0;

/// The FX panel's height as laid out now: with MERGE FX enabled its row is added.
pub fn fx_h(app: &App) -> f32 {
    // (View > Layout > Effect Panel chooses the rows; MERGE FX hangs under the CFX row.)
    let (beat, color) = app.prefs.rb.fx_rows();
    let base = match (beat, color) {
        (true, true) => FX_H,
        (true, false) => FX_H - 27.0,
        _ => FX_H - 53.0,
    };
    if app.prefs.fx.mfx.enabled && color { base + MFX_ROW_H } else { base }
}
const BEAT_H: f32 = 52.0;

/// A single-mode snapshot: knob 1, knob 2, button 1, button 2.
pub type Snap = (f32, f32, bool, bool);

/// One FX unit as the interface holds it.
pub struct FxUi {
    pub params: UnitParams,
    /// Index into each multi slot's beat table, and single mode's.
    pub idx: [usize; 3],
    pub one_idx: usize,
    /// The beat index each effect was left at (by its place in `fx::INFO`): the three multi
    /// slots, then single mode.
    pub beats: [[u8; 36]; 4],
    /// Single mode's snapshot of each effect.
    pub snaps: [Option<Snap>; 36],
    pub release_kind: u8,
    /// Index into the RELEASE FX beat table, kept per RELEASE FX.
    pub release_idx: [usize; 3],
    /// FX tempo: AUTO (the decks' own) or TAP.
    pub auto: bool,
    pub tap: fx::Tap,
    pub tap_bpm: f32,
}

fn pos(k: Kind) -> usize {
    fx::INFO.iter().position(|i| i.kind == k).unwrap_or(0)
}

impl FxUi {
    pub fn new() -> FxUi {
        let params = UnitParams::new();
        let defaults: [u8; 36] = std::array::from_fn(|i| fx::INFO[i].def as u8);
        let idx = [fx::beat_table(params.slots[0].kind).1, fx::beat_table(params.slots[1].kind).1, fx::beat_table(params.slots[2].kind).1];
        FxUi { params, idx, one_idx: fx::beat_table(params.one.kind).1, beats: [defaults; 4], snaps: [None; 36], release_kind: 0, release_idx: [fx::RELEASE[0].1, fx::RELEASE[1].1, fx::RELEASE[2].1], auto: true, tap: fx::Tap::new(), tap_bpm: 120.0 }
    }

    /// Put effect `k` in slot `s` (0..2 = the multi slots, 3 = single mode): switched off,
    /// at the beat value it was left at; single mode takes the effect's snapshot.
    pub fn set_kind(&mut self, s: usize, k: Kind) {
        let table = fx::beat_table(k).0;
        let idx = (self.beats[s][pos(k)] as usize).min(table.len().saturating_sub(1));
        let mut sp = fx::SlotParams::new(k);
        sp.value = table.get(idx).copied().unwrap_or(0.0);
        if s < 3 {
            sp.level = self.params.slots[s].level;
            self.params.slots[s] = sp;
            self.idx[s] = idx;
        } else {
            sp.level = self.params.one.level;
            if let Some((a, b, c, d)) = self.snaps[pos(k)] {
                sp.knob = [a, b];
                sp.btn = [c, d];
            }
            self.params.one = sp;
            self.one_idx = idx;
        }
    }

    pub fn set_index(&mut self, s: usize, idx: usize) {
        let sp = if s < 3 { &mut self.params.slots[s] } else { &mut self.params.one };
        let table = fx::beat_table(sp.kind).0;
        if table.is_empty() {
            return;
        }
        let idx = idx.min(table.len() - 1);
        sp.value = table[idx];
        self.beats[s][pos(sp.kind)] = idx as u8;
        if s < 3 {
            self.idx[s] = idx;
        } else {
            self.one_idx = idx;
        }
    }

    /// What is kept between sessions.
    pub fn saved(&self) -> UnitSaved {
        let by_id = |slot: usize, single: bool, n: usize| -> Vec<u8> { (0..n).map(|id| fx::from_id(id as i64, single).map(|k| self.beats[slot][pos(k)]).unwrap_or(0)).collect() };
        UnitSaved {
            single_shown: self.params.single,
            single_fx: fx::info_of(self.params.one.kind).single_id,
            single_beat: by_id(3, true, 34),
            multi_fx: std::array::from_fn(|s| fx::info_of(self.params.slots[s].kind).multi_id),
            multi_beat: std::array::from_fn(|s| by_id(s, false, 35)),
            release_fx: self.release_kind,
            release_beat: std::array::from_fn(|i| self.release_idx[i] as u8),
            snapshots: (0..34).map(|id| fx::from_id(id, true).and_then(|k| self.snaps[pos(k)])).collect(),
        }
    }

    pub fn load(&mut self, s: &UnitSaved) {
        for (slot, single, list) in [(0, false, &s.multi_beat[0]), (1, false, &s.multi_beat[1]), (2, false, &s.multi_beat[2]), (3, true, &s.single_beat)] {
            for (id, b) in list.iter().enumerate() {
                if let Some(k) = fx::from_id(id as i64, single) {
                    let n = fx::beat_table(k).0.len();
                    self.beats[slot][pos(k)] = (*b as usize).min(n.saturating_sub(1)) as u8;
                }
            }
        }
        for (id, snap) in s.snapshots.iter().enumerate() {
            if let Some(k) = fx::from_id(id as i64, true) {
                self.snaps[pos(k)] = snap.map(|(a, b, c, d)| (a.clamp(0.0, 1.0), b.clamp(0.0, 1.0), c, d));
            }
        }
        // (An id that is no effect leaves the slot's effect as it is.)
        for slot in 0..3 {
            self.set_kind(slot, fx::from_id(s.multi_fx[slot] as i64, false).unwrap_or(self.params.slots[slot].kind));
        }
        self.set_kind(3, fx::from_id(s.single_fx as i64, true).unwrap_or(self.params.one.kind));
        self.params.single = s.single_shown;
        self.release_kind = s.release_fx.min(2);
        for i in 0..3 {
            self.release_idx[i] = (s.release_beat[i] as usize).min(fx::RELEASE_BEATS.len() - 1);
        }
    }
}

/// One unit's kept choices, as rekordbox's FxUnitSettings holds them: the mode, the effects
/// by engine id, the beat-table index of every effect by engine id (per multi slot and for
/// single mode), the RELEASE FX and its beat indices, and single mode's snapshots. Not kept,
/// as in rekordbox: on/off, LEVEL/DEPTH, the assign buttons, AUTO / TAP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UnitSaved {
    pub single_shown: bool,
    pub single_fx: i8,
    pub single_beat: Vec<u8>,
    pub multi_fx: [i8; 3],
    pub multi_beat: [Vec<u8>; 3],
    pub release_fx: u8,
    pub release_beat: [u8; 3],
    pub snapshots: Vec<Option<Snap>>,
}

impl Default for UnitSaved {
    fn default() -> Self {
        FxUi::new().saved()
    }
}

/// The CFX row's kept choices: the mode, DEFAULT / USER, the USER effect, PARAM per effect,
/// and multi mode's effect per channel (decks 1 to 4, master, sampler).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CfxSaved {
    pub multi_shown: bool,
    pub user: bool,
    pub user_fx: u8,
    pub params: [f32; 9],
    pub multi_fx: [u8; 6],
}

impl Default for CfxSaved {
    fn default() -> Self {
        CfxSaved { multi_shown: false, user: false, user_fx: 0, params: [0.5; 9], multi_fx: [0; 6] }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxSaved {
    pub units: [UnitSaved; 2],
    pub cfx: CfxSaved,
    /// The PAD FX pads, the MERGE FX settings and Preferences > Controller > Effect.
    pub padfx: crate::ui_padfx::PadFxPrefs,
    pub mfx: crate::mfx::MfxPrefs,
    pub effect: EffectPrefs,
}

/// Preferences > Controller > Effect > RELEASE FX (rekordbox's `ReleaseFxUnitNum`, default 2,
/// and `ReleaseFxOnCfx`, default off).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectPrefs {
    pub release_units: u8,
    pub release_on_cfx: bool,
}

impl Default for EffectPrefs {
    fn default() -> Self {
        EffectPrefs { release_units: 2, release_on_cfx: false }
    }
}

/// The FX panel's state outside the two units.
pub struct FxExtra {
    /// The kept choices have been put in place (once, at the first frame).
    pub loaded: bool,
    /// A change waits to be written.
    pub pending: Option<std::time::Instant>,
    /// CFX multi mode: an effect and an on/off per channel (decks 1 to 4, master, sampler).
    pub cfx_multi: bool,
    pub cfx_multi_kind: [u8; 6],
    pub cfx_multi_on: [bool; 6],
    /// PARAM of each CFX effect.
    pub cfx_params: [f32; 9],
    /// PAD FX as it is being played, and MERGE FX.
    pub pad: crate::ui_padfx::PadFxState,
    pub mfx: crate::mfx::MfxState,
}

impl Default for FxExtra {
    fn default() -> Self {
        FxExtra { loaded: false, pending: None, cfx_multi: false, cfx_multi_kind: [0; 6], cfx_multi_on: [true; 6], cfx_params: [0.5; 9], pad: Default::default(), mfx: Default::default() }
    }
}

fn cfx_ok(k: u8) -> u8 {
    if crate::cfx::LIST.iter().any(|x| x.1 == Some(k)) { k } else { 0 }
}

/// The kept choices as they stand now.
pub fn snapshot(app: &App) -> FxSaved {
    let x = &app.px.fxx;
    FxSaved { units: [app.px.fx[0].saved(), app.px.fx[1].saved()], cfx: CfxSaved { multi_shown: x.cfx_multi, user: app.px.cfx_user, user_fx: app.px.cfx_kind, params: x.cfx_params, multi_fx: x.cfx_multi_kind }, padfx: app.prefs.fx.padfx.clone(), mfx: app.prefs.fx.mfx.clone(), effect: app.prefs.fx.effect.clone() }
}

/// Put kept choices in place.
pub fn restore(app: &mut App, s: &FxSaved) {
    for (u, saved) in app.px.fx.iter_mut().zip(&s.units) {
        u.load(saved);
    }
    let x = &mut app.px.fxx;
    x.cfx_multi = s.cfx.multi_shown;
    x.cfx_multi_kind = s.cfx.multi_fx.map(cfx_ok);
    x.cfx_params = s.cfx.params.map(|p| if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.5 });
    app.px.cfx_user = s.cfx.user;
    app.px.cfx_kind = cfx_ok(s.cfx.user_fx);
    app.px.cfx_param = x.cfx_params[(if app.px.cfx_user { app.px.cfx_kind } else { 0 }) as usize % 9];
}

/// The Sound Color FX of a channel (0..3 = decks 1 to 4, 4 = master, 5 = sampler): effect,
/// COLOR, PARAM. Single mode: one effect for every channel; multi mode: the channel's own,
/// out of the path while its button is off.
pub fn cfx_for(app: &App, ch: usize) -> (u8, f32, f32) {
    let x = &app.px.fxx;
    let mut color = if ch == fx::SAMPLER { app.px.sampler.cfx_color } else { app.px.cfx_color[ch.min(4)] };
    // "Apply RELEASE FX on CFX": a held RELEASE FX switches the channel's CFX off too.
    if app.prefs.fx.effect.release_on_cfx && app.px.fx.iter().any(|f| f.params.release.is_some() && f.params.assign[ch.min(5)]) {
        color = 0.5;
    }
    if x.cfx_multi {
        let k = x.cfx_multi_kind[ch.min(5)];
        (k, if x.cfx_multi_on[ch.min(5)] { color } else { 0.5 }, x.cfx_params[k as usize % 9])
    } else {
        (if app.px.cfx_user { app.px.cfx_kind } else { 0 }, color, app.px.cfx_param)
    }
}

/// Once per frame: hand the units' settings and each channel's tempo to the audio thread.
/// The rack (its delay lines) is made the first time the FX panel is used.
pub fn sync(app: &mut App) {
    if !app.px.fxx.loaded {
        app.px.fxx.loaded = true;
        let s = app.prefs.fx.clone();
        restore(app, &s);
        // Screenshot switches (PREPDECK_SHOT_FX=single,cfxmulti,mfx,padfx,user).
        if let Ok(v) = std::env::var("PREPDECK_SHOT_FX") {
            for w in v.split(',') {
                match w.trim() {
                    "single" => app.px.fx.iter_mut().for_each(|f| f.params.single = true),
                    "cfxmulti" => app.px.fxx.cfx_multi = true,
                    "user" => app.px.cfx_user = true,
                    "mfx" => {
                        app.prefs.fx.mfx.enabled = true;
                        app.px.fxx.mfx.show = true;
                    }
                    "padfx" => app.px.pad_mode = [crate::ui_padfx::PAD_FX; 4],
                    _ => {}
                }
            }
        }
    }
    // Keep the choices: written within half a second of a change.
    let now = snapshot(app);
    if now != app.prefs.fx {
        app.prefs.fx = now;
        // (Timed from the first change, so a run of changes cannot put the write off.)
        app.px.fxx.pending.get_or_insert_with(std::time::Instant::now);
    }
    if app.px.fxx.pending.map(|t| t.elapsed().as_millis() > 500).unwrap_or(false) {
        app.px.fxx.pending = None;
        app.save_prefs();
    }
    let perf = app.performance();
    let on = perf && app.prefs.perf_fx;
    // A deck that left PAD FX (or Performance mode) lets go of everything its pads held.
    for d in 0..4usize {
        let p = &app.px.fxx.pad.decks[d];
        // (A USER pad holding a PAD FX pad is PAD FX too: pads_ext.rs.)
        let user_fx = matches!(app.pads.deck[d].user_held, Some(crate::pads_ext::UserHeld::PadFx(_)));
        if (!perf || app.px.pad_mode[d] != crate::ui_padfx::PAD_FX) && !user_fx && (!p.held.is_empty() || p.release.is_some() || p.slip.is_some() || p.edit) {
            app.px.fxx.pad.decks[d].edit = false;
            if (d as u8) < app.deck_count() {
                app.with_deck(d as u8, |a| crate::ui_padfx::stop_deck(a, d));
            } else {
                app.px.fxx.pad.decks[d].stop();
            }
        }
    }
    // PAD FX and MERGE FX work with the FX panel hidden too; outside Performance mode
    // nothing does.
    let pad_wanted = if perf { app.px.fxx.pad.wanted() } else { [false; 4] };
    let pad_sets = if perf { app.px.fxx.pad.sets(&app.prefs.fx.padfx) } else { [fx::PadSet::NONE; 4] };
    let merge = perf && app.prefs.fx.mfx.enabled;
    if !on && !merge && !pad_wanted.iter().any(|w| *w) {
        if let Some(mut g) = fx::RACK.try_lock() {
            if let Some(r) = g.as_mut() {
                r.enabled = false;
                r.pad_set = [fx::PadSet::NONE; 4];
                r.master_release = None;
                if let Some(m) = r.merge.as_deref_mut() {
                    m.stop(&mut app.px.fxx.mfx);
                }
                r.performance = perf;
            }
        }
        return;
    }
    // Tempo per channel: each deck's own, and the master deck's for the master. Beat FX
    // quantize follows each deck's Q.
    let mut bpm = [120.0f32; fx::CHANNELS];
    let mut quantize = [false; fx::CHANNELS];
    for n in 0..app.deck_count() {
        let b = app.with_deck(n, |a| if a.deck.track.is_some() { a.effective_bpm() } else { 0.0 });
        if b > 0.0 {
            bpm[n as usize] = b as f32;
        }
        quantize[n as usize] = if n == app.px.active { app.prefs.quantize } else { app.px.parked_quantize[n as usize] };
    }
    let m = app.px.master.min(3) as usize;
    bpm[fx::MASTER] = bpm[m];
    quantize[fx::MASTER] = quantize[m];
    bpm[fx::SAMPLER] = app.prefs.sampler.bpm.clamp(40.0, 499.0);
    let master_cfx = cfx_for(app, fx::MASTER);
    if merge {
        // (Read before the rack is locked: MERGE FX restarts its repeat grid on a playing deck.)
        for n in 0..app.deck_count().min(4) {
            app.px.fxx.mfx.playing[n as usize] = app.with_deck(n, |a| a.player.state.lock().playing);
        }
    }
    // With RELEASE FX > Unit Number 1 the one RELEASE FX is the master's (FX1's panel).
    let master_release = app.prefs.fx.effect.release_units == 1;
    let mut samples = if merge { crate::mfx::prepare_samples(app) } else { Default::default() };
    let stop = {
        let settings = app.px.fx.each_ref().map(|unit| {
            let mut params = unit.params;
            if !on {
                params.assign.fill(false);
            }
            params
        });
        let mut rack = fx::lock_prepared(&fx::RACK, settings, pad_wanted, pad_sets, master_release && on, merge);
        rack.performance = perf;
        rack.enabled = on;
        rack.master_cfx_set = master_cfx;
        for (u, ui) in rack.units.iter_mut().zip(app.px.fx.iter()) {
            let mut p = ui.params;
            p.bpm = if ui.auto { bpm } else { [ui.tap_bpm; fx::CHANNELS] };
            p.quantize = quantize;
            if master_release {
                p.release = None;
            }
            u.params = p;
        }
        rack.master_release = if master_release && on { app.px.fx[0].params.release } else { None };
        rack.master_bpm = if app.px.fx[0].auto { bpm[fx::MASTER] } else { app.px.fx[0].tap_bpm };
        for d in 0..4 {
            rack.pad_bpm[d] = bpm[d];
            rack.pad_quantize[d] = quantize[d];
        }
        rack.pad_set = pad_sets;
        if merge || rack.merge.is_some() {
            crate::mfx::sync(app, &mut rack, &bpm, &mut samples)
        } else {
            Vec::new()
        }
    };
    // MERGE FX has dropped: "Stop playing the operating deck after MERGE FX finishes".
    for d in stop {
        if (d as u8) < app.deck_count() {
            app.with_deck(d as u8, |a| {
                if a.player.state.lock().playing {
                    a.play_pause();
                }
            });
        }
    }
}

/// rekordbox lays the FX panel out in design units, 1280 across (`EffectPanelLayout`,
/// `SimpleMultiFxUnitLayout`, `SimpleSingleFxUnitLayout`, `SimpleReleaseFxLayout`,
/// `SimpleBPMController`, `SingleCFXDefaultSimple`, `MultiCFXSimple`, `MultiCfxControlSimple`):
/// x and width are scaled with the panel's width, y and height are not.
#[derive(Clone, Copy)]
struct Lay {
    x0: f32,
    y0: f32,
    k: f32,
}

impl Lay {
    /// A control's rectangle, as rekordbox's `resized` functions round it: the left edge down
    /// and the right edge up to whole points.
    fn r(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_max(pos2(self.x0 + (x * self.k).floor(), self.y0 + y), pos2(self.x0 + ((x + w) * self.k).ceil(), self.y0 + y + h))
    }

    /// A panel inside this one (`x`, `w` in this panel's units): its own controls are scaled
    /// by its rounded width over its design width.
    fn sub(&self, x: f32, w: f32) -> Lay {
        let r = self.r(x, 0.0, w, 0.0);
        Lay { x0: r.min.x, y0: self.y0, k: r.width() / w }
    }
}

/// The colour of the panel's `<` `>` arrows (measured on reference performance/06).
const ARROW: egui::Color32 = hex(0x888888);
const ARROW_OFF: egui::Color32 = hex(0x464646);
/// The outline of the buttons that are not lit (RELEASE FX, TAP, USER).
const OUTLINE: egui::Color32 = hex(0x646464);

fn small(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, lit: bool, enabled: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let p = ui.painter();
    p.rect_filled(r, 2.0, if lit { BLUE.c() } else { crate::ui_feel::hl_if(&resp, enabled, gy(0x282828), lighten(BTN.c())) });
    p.text(r.center(), Align2::CENTER_CENTER, label, caps(9.0), if enabled { WHITE.c() } else { OFF.c() });
    resp
}

/// A button with an outline when it is not the chosen one (AUTO / TAP, DEFAULT / USER).
fn choice(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, lit: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let p = ui.painter();
    p.rect_filled(r, 2.0, if lit { BLUE.c() } else { crate::ui_feel::hl(&resp, gy(0x1E1E1E), lighten(BTN.c())) });
    if !lit {
        p.rect_stroke(r, 2.0, Stroke::new(1.0_f32, OUTLINE), egui::StrokeKind::Inside);
    }
    p.text(r.center(), Align2::CENTER_CENTER, label, caps(9.0), WHITE.c());
    resp
}

/// The FX panel's knob, as rekordbox draws it (measured on reference performance/06): a ring
/// 17.5 pt across (`rad` 7.6), open at the bottom, blue from its start to the value
/// (`bipolar`: from the centre to the value), and a white mark at the value. Drag up or down
/// to turn (a full sweep over 150 pt), double click for `reset`. Returns the new value when
/// it changed.
fn fx_knob_r(ui: &mut Ui, c: egui::Pos2, rad: f32, id: impl std::hash::Hash, value: f32, reset: f32, bipolar: bool) -> (Option<f32>, bool) {
    let p = ui.painter().clone();
    let resp = ui.interact(Rect::from_center_size(c, vec2(rad * 2.0 + 7.0, rad * 2.0 + 7.0)), ui.id().with(id), Sense::click_and_drag());
    let mut out = None;
    if resp.double_clicked() {
        out = Some(reset);
    } else if resp.dragged() {
        let dy = crate::ui_feel::knob_dy(ui, &resp);
        if dy != 0.0 {
            out = Some((value - dy / 150.0).clamp(0.0, 1.0));
        }
    }
    let v = out.unwrap_or(value);
    crate::ui_feel::grab_cursor(&resp);
    crate::ui_feel::knob_readout(ui, &resp, egui::pos2(c.x, c.y - rad + 6.0), v, if bipolar { 0.5 } else { -1.0 });
    let v = crate::ui_feel::knob_shown(&resp, v);
    let ang = |x: f32| (-225.0 + 270.0 * x).to_radians();
    let arc = |a: f32, b: f32| -> Vec<egui::Pos2> { (0..=32).map(|i| { let t = ang(a + (b - a) * i as f32 / 32.0); c + vec2(t.cos(), t.sin()) * rad }).collect() };
    let active = resp.hovered() || resp.dragged();
    p.add(egui::Shape::line(arc(0.0, 1.0), Stroke::new(2.4_f32, if active { gy(0x464646) } else { gy(0x323232) })));
    let from = if bipolar { 0.5 } else { 0.0 };
    if (v - from).abs() > 0.012 {
        p.add(egui::Shape::line(arc(from, v), Stroke::new(2.4_f32, BLUE)));
    }
    let a = ang(v);
    p.line_segment([c + vec2(a.cos(), a.sin()) * (rad - 2.4), c + vec2(a.cos(), a.sin()) * (rad + 2.0)], Stroke::new(2.0_f32, WHITE));
    (out, active)
}

fn fx_knob(ui: &mut Ui, c: egui::Pos2, id: impl std::hash::Hash, value: f32, reset: f32, bipolar: bool) -> Option<f32> {
    fx_knob_r(ui, c, 7.6, id, value, reset, bipolar).0
}

/// A mode button: three dots (multi) or one (single), lit for the mode in use.
fn mode_button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, dots: i32, lit: bool, tooltip: &str) -> bool {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let p = ui.painter();
    if lit {
        p.rect_filled(r, 2.0, BLUE);
    } else {
        p.rect_filled(r, 2.0, crate::ui_feel::hl(&resp, gy(0x181818), lighten(BTN.c())));
        p.rect_stroke(r, 2.0, Stroke::new(1.0_f32, gy(0x646464)), egui::StrokeKind::Inside);
    }
    for k in 0..dots {
        p.circle_filled(r.center() + vec2((k as f32 - (dots - 1) as f32 / 2.0) * 5.3, 0.0), 1.8, WHITE);
    }
    let clicked = resp.clicked();
    let _ = resp.on_hover_text(tooltip);
    clicked
}

/// The read-out of a single-mode knob (rekordbox: most are the position as 0..100; PITCH
/// runs -100..+100 with a dead zone around 0; STAGE is the number of stages).
pub fn knob_text(label: &str, v: f32) -> String {
    match label {
        "PITCH" => {
            let x = ((v - 0.5) * 200.0).round();
            if x.abs() < 2.5 { "0".into() } else { format!("{x:+.0}") }
        }
        "STAGE" => format!("{}", { let (l, r) = fx::phaser_stages(v); l.max(r) }),
        _ => format!("{:.0}", v * 100.0),
    }
}

/// The effect's name (on/off), its list, LEVEL/DEPTH and `<` value `>`: a multi slot
/// (`s` 0..2, `dx` its offset in design units) or single mode's effect (`s` 3).
fn slot(app: &mut App, ui: &mut Ui, u: usize, s: usize, l: Lay, dx: f32) {
    let p = ui.painter().clone();
    // (The slots are laid out inside the unit's effect panel, 283 wide from 157.)
    let l = l.sub(157.0, 283.0);
    let l = Lay { x0: l.x0 - 157.0 * l.k, ..l };
    let single = s == 3;
    let sp = if single { app.px.fx[u].params.one } else { app.px.fx[u].params.slots[s] };
    let mut name_r = l.r(163.0 + dx, 2.0, 70.0, 18.0);
    name_r.max.x -= 2.0;
    let nresp = ui.interact(name_r, ui.id().with(("fxname", u, s)), Sense::click());
    p.rect_filled(name_r, 2.0, if sp.on { BLUE.c() } else { crate::ui_feel::hl(&nresp, BTN.c(), lighten(BTN.c())) });
    p.text(name_r.center(), Align2::CENTER_CENTER, fx::name(sp.kind), caps(8.75), WHITE.c());
    if tip(nresp, app, "Turns the effect on/off.").clicked() {
        let f = &mut app.px.fx[u].params;
        if single { f.one.on = !sp.on } else { f.slots[s].on = !sp.on }
    }
    let list_r = l.r(233.0 + dx, 2.0, 16.0, 18.0);
    let lresp = ui.interact(list_r, ui.id().with(("fxlist", u, s)), Sense::click());
    p.rect_filled(list_r, 2.0, crate::ui_feel::hl(&lresp, BTN.c(), lighten(BTN.c())));
    crate::icons_player::chevron_down(&p, list_r.center() + vec2(0.0, 0.0), 0.85, WHITE.c());
    crate::ui_browser::dropdown(ui, &lresp, &format!("fxpop{u}{s}"), |ui| {
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            // (rekordbox draws a line before the last nine, its "FX+ pack".)
            let n = fx::list(single).count();
            for (i, info) in fx::list(single).enumerate() {
                if i + 9 == n {
                    ui.separator();
                }
                if ui.selectable_label(sp.kind == info.kind, info.name).clicked() {
                    app.px.fx[u].set_kind(s, info.kind);
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
    });
    // LEVEL/DEPTH, then `<` value `>`.
    if let Some(v) = fx_knob(ui, l.r(167.0 + dx, 25.0, 20.0, 20.0).center(), ("fxlevel", u, s), sp.level, 0.5, false) {
        let f = &mut app.px.fx[u].params;
        if single { f.one.level = v } else { f.slots[s].level = v }
    }
    let table = fx::beat_table(sp.kind).0;
    let idx = if single { app.px.fx[u].one_idx } else { app.px.fx[u].idx[s] }.min(table.len().saturating_sub(1));
    let (can_down, can_up) = (idx > 0, idx + 1 < table.len());
    let down = ui.interact(l.r(190.0 + dx, 29.0, 15.0, 16.0), ui.id().with(("fxdown", u, s)), Sense::click());
    crate::icons_player::chevron_left(&p, pos2(down.rect.min.x + 8.5, down.rect.center().y), 1.15, if can_down { ARROW } else { ARROW_OFF });
    if down.clicked() && can_down {
        app.px.fx[u].set_index(s, idx - 1);
    }
    p.text(l.r(192.0 + dx, 29.0, 56.0, 16.0).center(), Align2::CENTER_CENTER, fx::value_text(sp.kind, sp.value), bold(9.0), if table.is_empty() { OFF.c() } else { WHITE.c() });
    let up = ui.interact(l.r(235.0 + dx, 29.0, 15.0, 16.0), ui.id().with(("fxup", u, s)), Sense::click());
    crate::icons_player::chevron_right(&p, pos2(up.rect.min.x + 6.0, up.rect.center().y), 1.15, if can_up { ARROW } else { ARROW_OFF });
    if up.clicked() && can_up {
        app.px.fx[u].set_index(s, idx + 1);
    }
}

/// Single mode's own controls at rekordbox's places: for each of the two parameters a label,
/// a small knob under it and its button beside the knob; the snapshot button at the right.
/// While a knob is under the pointer its label shows the value.
fn single_controls(app: &mut App, ui: &mut Ui, u: usize, l: Lay) {
    let p = ui.painter().clone();
    let l = l.sub(157.0, 283.0);
    let l = Lay { x0: l.x0 - 157.0 * l.k, ..l };
    let sp = app.px.fx[u].params.one;
    let labels = fx::info_of(sp.kind).labels;
    for k in 0..2usize {
        let dx = k as f32 * 88.0;
        let label = labels[k];
        let kc = l.r(267.0 + dx, 29.0, 16.0, 16.0).center();
        let lr = l.r(249.0 + dx, 14.0, 56.0, 11.0);
        if label.is_empty() {
            p.circle_stroke(kc, 6.0, Stroke::new(2.4_f32, gy(0x282828)));
        } else {
            // (PITCH, FILTER and the like rest at their centre: the blue runs from there.)
            let bipolar = matches!(label, "PITCH" | "FILTER" | "SWING" | "LR RATIO" | "LOW / HI CUT" | "SPEED" | "TIME");
            let (v, active) = fx_knob_r(ui, kc, 6.0, ("fxknob", u, k), sp.knob[k], 0.5, bipolar);
            if let Some(v) = v {
                app.px.fx[u].params.one.knob[k] = v;
            }
            let shown = v.unwrap_or(sp.knob[k]);
            p.text(lr.center(), Align2::CENTER_CENTER, if active { fx::single_knob_text(sp.kind, k, shown) } else { label.to_string() }, caps(8.0), WHITE.c());
        }
        let label = labels[2 + k];
        if label.is_empty() {
            continue;
        }
        let mut r = l.r(287.0 + dx + if k == 1 { 5.0 } else { 0.0 }, 28.0 - k as f32, 50.0, 18.0);
        r.max.x -= 2.0;
        let resp = ui.interact(r, ui.id().with(("fxbtn", u, k)), Sense::click());
        p.rect_filled(r, 2.0, if sp.btn[k] { BLUE.c() } else { crate::ui_feel::hl(&resp, BTN.c(), lighten(BTN.c())) });
        p.text(r.center(), Align2::CENTER_CENTER, label, caps(8.0), WHITE.c());
        if resp.clicked() {
            app.px.fx[u].params.one.btn[k] = !sp.btn[k];
        }
    }
    // Snapshot: keeps the two knobs and the two buttons for this effect (LEVEL/DEPTH is not
    // kept, as in rekordbox). Lit while the controls are as saved.
    let r = l.r(415.0, 2.0, 20.0, 18.0);
    let now: Snap = (sp.knob[0], sp.knob[1], sp.btn[0], sp.btn[1]);
    let saved = app.px.fx[u].snaps[pos(sp.kind)] == Some(now);
    let resp = ui.interact(r, ui.id().with(("fxsnap", u)), Sense::click());
    p.rect_filled(r, 2.0, crate::ui_feel::hl(&resp, BTN.c(), lighten(BTN.c())));
    let c = if saved { hex(0x3C8CFF) } else { WHITE.c() };
    let body = Rect::from_center_size(r.center() + vec2(0.0, 0.5), vec2(12.0, 8.0));
    p.rect_stroke(body, 1.5, Stroke::new(1.0_f32, c), egui::StrokeKind::Inside);
    p.circle_stroke(body.center(), 2.0, Stroke::new(1.0_f32, c));
    p.rect_filled(Rect::from_min_size(pos2(body.min.x + 2.0, body.min.y - 1.5), vec2(4.0, 1.5)), 0.0, c);
    if resp.clicked() {
        app.px.fx[u].snaps[pos(sp.kind)] = Some(now);
    }
    let _ = resp.on_hover_text("Snapshot: saves the positions of the knobs and buttons for this effect.");
}

/// The FX row: FX1 in the left half of `r`, FX2 in the right half; under it the CFX row and,
/// when MERGE FX is enabled (and `r` is `fx_h` tall), MERGE FX's row.
pub fn panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    let full = r;
    // (rekordbox: 3 pt of black above the Beat FX row and between it and the CFX row.)
    // (Measured on the reference: 1 pt of black, the 49 pt Beat FX row, 3 pt of black, the
    // 24 pt CFX row at 53.)
    p.rect_filled(full, 0.0, BG);
    let r = Rect::from_min_max(pos2(r.min.x, r.min.y + 1.0), pos2(r.max.x, r.min.y + 50.0));
    // (View > Layout > Effect Panel: BEAT FX, SOUND COLOR FX or both rows.)
    let (beat_row, color_row) = app.prefs.rb.fx_rows();
    let top = if beat_row { 53.0 } else { 0.0 };
    let bottom = Rect::from_min_max(pos2(full.min.x, full.min.y + top), pos2(full.max.x, full.min.y + top + 24.0));
    if color_row {
        cfx_row(app, ui, bottom);
        if app.prefs.fx.mfx.enabled {
            // MERGE FX's own row under the CFX row.
            let mrow = Rect::from_min_max(pos2(full.min.x, bottom.max.y + 3.0), pos2(full.max.x, bottom.max.y + 3.0 + 24.0));
            mfx_row(app, ui, mrow);
        }
    }
    if !beat_row {
        return;
    }
    p.rect_filled(r, 0.0, gy(0x1E1E1E));
    let whole = Lay { x0: full.min.x, y0: full.min.y + 1.0, k: full.width() / 1280.0 };
    let k = whole.k;
    p.rect_filled(Rect::from_min_max(pos2(whole.r(0.0, 0.0, 639.0, 0.0).max.x, full.min.y), pos2(whole.r(641.0, 0.0, 1.0, 0.0).min.x, full.min.y + 50.0)), 0.0, BG);
    let master_release = app.prefs.fx.effect.release_units == 1;
    for u in 0..2usize {
        let l = whole.sub(if u == 0 { 0.0 } else { 641.0 }, 639.0);
        let _ = k;
        p.text(pos2(l.x0 + 10.5, l.y0 + 12.0), Align2::LEFT_CENTER, format!("FX{}", u + 1), bold(12.5), gy(0x8C8C8C));
        // Assign: decks, the sampler (S) and the master (M alone, as in rekordbox: lighting M
        // clears the others, lighting another clears M). Decks 3 and 4 exist in the 4-deck
        // layout.
        let four = app.deck_count() == 4;
        let labels = [("1", Some(0usize)), ("2", Some(1)), ("S", Some(fx::SAMPLER)), ("3", if four { Some(2) } else { None }), ("4", if four { Some(3) } else { None }), ("M", Some(fx::MASTER))];
        for (i, (label, ch)) in labels.iter().enumerate() {
            let br = l.r(65.0 + (i % 3) as f32 * 18.0, 8.0 + (i / 3) as f32 * 18.0, 17.0, 17.0);
            let lit = ch.map(|c| app.px.fx[u].params.assign[c]).unwrap_or(false);
            let resp = small(ui, br, ("fxassign", u, i), label, lit, ch.is_some());
            if let Some(c) = ch {
                if resp.clicked() {
                    let a = &mut app.px.fx[u].params.assign;
                    a[*c] = !a[*c];
                    if a[*c] {
                        if *c == fx::MASTER {
                            for (i, d) in a.iter_mut().enumerate() {
                                if i != fx::MASTER {
                                    *d = false;
                                }
                            }
                        } else {
                            a[fx::MASTER] = false;
                        }
                    }
                }
            } else {
                let _ = resp.on_hover_text("Decks 3 and 4 are in the 4Deck layout.");
            }
        }
        // Mode: multi (three effects) or single (one effect with five parameters).
        let single = app.px.fx[u].params.single;
        if mode_button(ui, l.r(134.0, 5.0, 20.0, 20.0).shrink(0.75), ("fxmulti", u), 3, !single, "Beat FX multi-mode:\nThis mode allows to use up to three effects simultaneously.") {
            app.px.fx[u].params.single = false;
        }
        if mode_button(ui, l.r(134.0, 25.0, 20.0, 20.0).shrink(0.75), ("fxsingle", u), 1, single, "Beat FX single-mode:\nThis mode allows to use one effect with five parameters.") {
            app.px.fx[u].params.single = true;
        }
        if single {
            slot(app, ui, u, 3, l, 0.0);
            single_controls(app, ui, u, l);
        } else {
            for (s, dx) in [0.0, 95.0, 188.0].into_iter().enumerate() {
                slot(app, ui, u, s, l, dx);
            }
        }

        // RELEASE FX: held while the name is pressed. With Unit Number 1 there is one, the
        // master's, in FX1.
        let rl = l.sub(441.0, 99.0);
        if master_release && u == 1 {
            app.px.fx[u].params.release = None;
        } else {
            let rk = app.px.fx[u].release_kind.min(2) as usize;
            let rel_r = rl.r(15.0, 4.0, 66.0, 18.0).shrink2(vec2(0.5, 0.0));
            let rresp = ui.interact(rel_r, ui.id().with(("fxrel", u)), Sense::click_and_drag());
            let held = rresp.is_pointer_button_down_on();
            p.rect_filled(rel_r, 2.0, gy(0x1E1E1E));
            p.rect_stroke(rel_r, 2.0, Stroke::new(1.0_f32, if held { BLUE.c() } else { OUTLINE }), egui::StrokeKind::Inside);
            p.text(rel_r.center(), Align2::CENTER_CENTER, fx::RELEASE[rk].0, caps(8.75), if held { hex(0x3C8CFF) } else { WHITE.c() });
            let ridx = app.px.fx[u].release_idx[rk].min(fx::RELEASE_BEATS.len() - 1);
            app.px.fx[u].params.release = if held { Some((rk as u8, fx::RELEASE_BEATS[ridx])) } else { None };
            if held {
                ui.ctx().request_repaint();
            }
            let _ = rresp.on_hover_text(if master_release { "MASTER RELEASE FX: on while the button is held." } else { "RELEASE FX: on while the button is held." });
            let mut lr = rl.r(81.0, 4.0, 16.0, 18.0).shrink2(vec2(0.0, 0.5));
            lr.min.x += 1.5;
            let rlresp = ui.interact(lr, ui.id().with(("fxrellist", u)), Sense::click());
            p.rect_filled(lr, 2.0, crate::ui_feel::hl(&rlresp, BTN.c(), lighten(BTN.c())));
            crate::icons_player::chevron_down(&p, lr.center(), 0.85, WHITE.c());
            crate::ui_browser::dropdown(ui, &rlresp, &format!("fxrelpop{u}"), |ui| {
                for (i, (name, _)) in fx::RELEASE.iter().enumerate() {
                    if ui.selectable_label(rk == i, *name).clicked() {
                        app.px.fx[u].release_kind = i as u8;
                        ui.memory_mut(|m| m.close_popup());
                    }
                }
            });
            let rd = ui.interact(rl.r(20.0, 26.0, 15.0, 16.0), ui.id().with(("fxreldown", u)), Sense::click());
            crate::icons_player::chevron_left(&p, pos2(rd.rect.min.x + 8.5, rd.rect.center().y), 1.15, if ridx > 0 { ARROW } else { ARROW_OFF });
            if rd.clicked() && ridx > 0 {
                app.px.fx[u].release_idx[rk] = ridx - 1;
            }
            p.text(rl.r(22.0, 26.0, 56.0, 16.0).center() - vec2(0.0, 0.5), Align2::CENTER_CENTER, fx::beat_text(fx::RELEASE_BEATS[ridx], false), bold(8.5), WHITE.c());
            let ru = ui.interact(rl.r(65.0, 26.0, 15.0, 16.0), ui.id().with(("fxrelup", u)), Sense::click());
            crate::icons_player::chevron_right(&p, pos2(ru.rect.min.x + 6.0, ru.rect.center().y), 1.15, if ridx + 1 < fx::RELEASE_BEATS.len() { ARROW } else { ARROW_OFF });
            if ru.clicked() && ridx + 1 < fx::RELEASE_BEATS.len() {
                app.px.fx[u].release_idx[rk] = ridx + 1;
            }
        }

        // FX tempo: AUTO (each channel's own tempo) or TAP.
        let bl = l.sub(540.0, 99.0);
        let auto = app.px.fx[u].auto;
        let mut ar = bl.r(6.0, 3.5, 88.0, 18.0).shrink2(vec2(0.5, 1.0));
        ar.max.x -= 0.75;
        if choice(ui, ar, ("fxauto", u), "AUTO", auto).clicked() {
            app.px.fx[u].auto = true;
        }
        let label = if auto { "TAP".to_string() } else { format!("TAP  {:.1}", app.px.fx[u].tap_bpm) };
        let mut tr = bl.r(6.0, 25.0, 88.0, 18.0).shrink2(vec2(0.5, 1.0));
        tr.max.x -= 0.75;
        let tresp = choice(ui, tr, ("fxtap", u), &label, !auto);
        if crate::ui_player::press_fired(ui, &tresp) {
            let now = ui.input(|i| i.time);
            let f = &mut app.px.fx[u];
            if f.auto {
                f.auto = false;
            }
            if let Some(b) = f.tap.tap(now) {
                f.tap_bpm = b;
            }
        }
    }
}

/// A COLOR knob (centre = off, with a small catch so it comes to rest there).
fn color_knob(app: &mut App, ui: &mut Ui, kc: egui::Pos2, c: usize) {
    let v = if c == fx::SAMPLER { app.px.sampler.cfx_color } else { app.px.cfx_color[c] };
    if let Some(nv) = fx_knob(ui, kc, ("cfxcolor", c), v, 0.5, true) {
        let nv = if (nv - 0.5).abs() < 0.015 { 0.5 } else { nv };
        if c == fx::SAMPLER {
            app.px.sampler.cfx_color = nv;
        } else {
            app.px.cfx_color[c] = nv;
        }
    }
}

/// The CFX row at rekordbox's places. Single mode: DEFAULT / USER, the effect, PARAM, and a
/// COLOR knob per channel in the order S, 3, 1, 2, 4, M. Multi mode: PARAM, then per channel
/// its label, its effect (name = on/off, list) and its COLOR knob.
fn cfx_row(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, gy(0x1E1E1E));
    let l = Lay { x0: r.min.x, y0: r.center().y - 12.0, k: r.width() / 1280.0 };
    let cy = r.center().y;
    p.text(pos2(r.min.x + 10.5, cy - 0.5), Align2::LEFT_CENTER, "CFX", bold(12.5), gy(0x8C8C8C));
    let multi = app.px.fxx.cfx_multi;
    // (rekordbox's multi layout has the two mode buttons 26 x 24 at 70 and 101; the single
    // layout 18 x 18 at 70 and 90.)
    let (m1, m2) = if multi { (l.r(70.0, 3.0, 26.0, 18.0), l.r(101.0, 3.0, 26.0, 18.0)) } else { (l.r(70.0, 3.0, 18.0, 18.0), l.r(90.0, 3.0, 18.0, 18.0)) };
    if mode_button(ui, m1.shrink2(vec2(0.5, 0.0)), "cfxmulti", 3, multi, "CFX multi-mode: an effect for each channel.") {
        app.px.fxx.cfx_multi = true;
    }
    if mode_button(ui, m2.shrink2(vec2(0.5, 0.0)), "cfxsingle", 1, !multi, "CFX single-mode: one effect for all channels.") {
        app.px.fxx.cfx_multi = false;
    }
    // S (the sampler deck) is live when "Apply CFX to Sampler" is ticked in Preferences >
    // Controller > Effect; decks 3 and 4 in the 4-deck layout.
    let four = app.deck_count() == 4;
    let smp = if app.prefs.sampler.cfx_on_sampler { Some(fx::SAMPLER) } else { None };
    let channels = [("S", smp), ("3", if four { Some(2usize) } else { None }), ("1", Some(0)), ("2", Some(1)), ("4", if four { Some(3) } else { None }), ("M", Some(4))];
    if multi {
        // PARAM is the shown effect's... each effect keeps its own; the knob here sets the
        // one of the effect last touched (FILTER until then).
        let pk = app.px.cfx_kind as usize % 9;
        p.text(l.r(143.0, 1.0, 42.0, 22.0).center(), Align2::CENTER_CENTER, "PARAM", caps(9.0), WHITE.c());
        if let Some(v) = fx_knob(ui, l.r(185.0, 1.0, 22.0, 22.0).center(), "cfxparam", app.px.fxx.cfx_params[pk], 0.5, false) {
            app.px.fxx.cfx_params[pk] = v;
        }
        for (i, (label, ch)) in channels.into_iter().enumerate() {
            let c = Lay { x0: l.x0 + [227.0, 366.0, 505.0, 644.0, 783.0, 922.0][i] * l.k, ..l };
            p.text(c.r(0.0, 0.0, 20.0, 24.0).center(), Align2::CENTER_CENTER, label, caps(10.0), if ch.is_some() { WHITE.c() } else { OFF.c() });
            let kc = c.r(110.0, 1.0, 22.0, 22.0).center();
            let Some(ch) = ch else {
                p.circle_stroke(kc, 7.6, Stroke::new(2.4_f32, gy(0x282828)));
                continue;
            };
            let kind = app.px.fxx.cfx_multi_kind[ch];
            let on = app.px.fxx.cfx_multi_on[ch];
            let er = c.r(21.0, 3.0, 68.0, 18.0).shrink2(vec2(0.5, 0.0));
            let eresp = ui.interact(er, ui.id().with(("cfxmname", ch)), Sense::click());
            p.rect_filled(er, 2.0, if on { BLUE.c() } else { crate::ui_feel::hl(&eresp, BTN.c(), lighten(BTN.c())) });
            p.text(er.center(), Align2::CENTER_CENTER, crate::cfx::name(kind), caps(9.0), WHITE.c());
            if eresp.clicked() {
                app.px.fxx.cfx_multi_on[ch] = !on;
                app.px.cfx_kind = kind;
            }
            let _ = eresp.on_hover_text("Turns the effect on/off.");
            let lr = c.r(89.0, 3.0, 18.0, 18.0).shrink2(vec2(0.5, 0.0));
            let lresp = ui.interact(lr, ui.id().with(("cfxmlist", ch)), Sense::click());
            p.rect_filled(lr, 2.0, crate::ui_feel::hl(&lresp, BTN.c(), lighten(BTN.c())));
            crate::icons_player::chevron_down(&p, lr.center() + vec2(0.0, 0.5), 1.0, WHITE.c());
            crate::ui_browser::dropdown(ui, &lresp, &format!("cfxmpop{ch}"), |ui| {
                for (name, kd) in crate::cfx::LIST {
                    let Some(kd) = kd else { continue };
                    if ui.selectable_label(kind == kd, name).clicked() {
                        app.px.fxx.cfx_multi_kind[ch] = kd;
                        app.px.cfx_kind = kd;
                        ui.memory_mut(|m| m.close_popup());
                    }
                }
            });
            color_knob(app, ui, kc, ch);
        }
        return;
    }
    // DEFAULT (the mixer's effects: FILTER without hardware) / USER (the chosen effect).
    let user = app.px.cfx_user;
    if choice(ui, l.r(152.0, 3.5, 66.0, 16.0).shrink2(vec2(0.5, 0.0)), "cfxdefault", "DEFAULT", !user).clicked() {
        app.px.cfx_user = false;
        app.px.cfx_param = app.px.fxx.cfx_params[crate::cfx::DEFAULT_LIST[0] as usize];
    }
    if choice(ui, l.r(220.0, 3.5, 66.0, 16.0).shrink2(vec2(0.5, 0.0)), "cfxuser", "USER", user).clicked() {
        app.px.cfx_user = true;
        app.px.cfx_param = app.px.fxx.cfx_params[app.px.cfx_kind as usize % 9];
    }
    // The effect: FILTER in DEFAULT; in USER a click lists the nine types.
    let kind = if user { app.px.cfx_kind } else { crate::cfx::DEFAULT_LIST[0] };
    let mut er = l.r(323.0, 4.0, 58.0, 16.0);
    er.min.x += 1.0;
    er.max.x -= 4.0;
    let eresp = ui.interact(er, ui.id().with("cfxeffect"), Sense::click());
    p.rect_filled(er, 2.0, gy(0x1E1E1E));
    p.rect_stroke(er, 2.0, Stroke::new(0.75_f32, BLUE), egui::StrokeKind::Inside);
    p.text(er.center(), Align2::CENTER_CENTER, crate::cfx::name(kind), caps(9.0), WHITE.c());
    if user {
        crate::icons_player::chevron_down(&p, l.r(381.0, 4.0, 16.0, 16.0).center() + vec2(0.0, 0.5), 1.0, WHITE.c());
        let aresp = ui.interact(l.r(381.0, 4.0, 16.0, 16.0), ui.id().with("cfxeffectlist"), Sense::click());
        let resp = eresp.union(aresp);
        crate::ui_browser::dropdown(ui, &resp, "cfxpop", |ui| {
            for (name, kd) in crate::cfx::LIST {
                let Some(kd) = kd else { continue };
                if ui.selectable_label(kind == kd, name).clicked() {
                    app.px.cfx_kind = kd;
                    // (Each effect keeps its own PARAM.)
                    app.px.cfx_param = app.px.fxx.cfx_params[kd as usize % 9];
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
    }
    p.text(l.r(439.0, 1.0, 42.0, 22.0).center(), Align2::CENTER_CENTER, "PARAM", caps(9.0), WHITE.c());
    if let Some(v) = fx_knob(ui, l.r(481.0, 1.0, 22.0, 22.0).center(), "cfxparam", app.px.cfx_param, 0.5, false) {
        app.px.cfx_param = v;
        app.px.fxx.cfx_params[kind as usize % 9] = v;
    }
    for (i, (label, ch)) in channels.into_iter().enumerate() {
        p.text(l.r([538.0, 618.0, 698.0, 777.0, 857.0, 937.0][i], 0.0, 21.0, 24.0).center(), Align2::CENTER_CENTER, label, caps(10.0), if ch.is_some() { WHITE.c() } else { OFF.c() });
        let kc = l.r([560.0, 640.0, 720.0, 799.0, 879.0, 959.0][i], 1.0, 22.0, 22.0).center();
        match ch {
            Some(c) => color_knob(app, ui, kc, c),
            None => {
                p.circle_stroke(kc, 7.6, Stroke::new(2.4_f32, gy(0x282828)));
            }
        }
    }
}

/// MERGE FX's row: per unit (left for decks 1 / 3, right for decks 2 / 4) the deck buttons
/// (18 x 18 at 64 and 84), the MERGE FX selector (86 wide at 120), ON and the knob; at the
/// right end the sample volume and the settings. The first three are rekordbox's places
/// (`MFXUnitPanel::resized`); where its ON button and knob sit was not read, and there is no
/// capture of this panel.
fn mfx_row(app: &mut App, ui: &mut Ui, r: Rect) {
    use crate::mfx;
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, gy(0x1E1E1E));
    let l = Lay { x0: r.min.x, y0: r.center().y - 12.0, k: r.width() / 1280.0 };
    p.text(pos2(r.min.x + 10.5, r.center().y - 0.5), Align2::LEFT_CENTER, "MFX", bold(12.5), gy(0x8C8C8C));
    let four = app.deck_count() == 4;
    for u in 0..2usize {
        let h = Lay { x0: l.x0 + u as f32 * 400.0 * l.k, ..l };
        for (i, upper) in [false, true].into_iter().enumerate() {
            let deck = u + if upper { 2 } else { 0 };
            let lit = app.prefs.fx.mfx.upper[u] == upper;
            let resp = small(ui, h.r(64.0 + i as f32 * 20.0, 3.0, 18.0, 18.0), ("mfxdeck", u, i), &format!("{}", deck + 1), lit, !upper || four);
            if resp.clicked() && (!upper || four) && !lit && !app.px.fxx.mfx.units[u].on {
                app.prefs.fx.mfx.upper[u] = upper;
                app.save_prefs();
            }
        }
        let sel = app.prefs.fx.mfx.select[u].min(3) as usize;
        let sr = h.r(120.0, 3.0, 86.0, 18.0);
        let sresp = ui.interact(sr, ui.id().with(("mfxsel", u)), Sense::click());
        p.rect_filled(sr, 2.0, crate::ui_feel::hl(&sresp, BTN.c(), lighten(BTN.c())));
        p.text(pos2(sr.min.x + 6.0, sr.center().y), Align2::LEFT_CENTER, format!("MERGE FX {}", sel + 1), caps(9.0), WHITE.c());
        crate::icons_player::chevron_down(&p, pos2(sr.max.x - 8.0, sr.center().y + 0.5), 1.0, WHITE.c());
        let names: Vec<String> = (0..4).map(|i| format!("MERGE FX {}   {}", i + 1, mfx::PRESETS[app.prefs.fx.mfx.preset[i].min(7) as usize].0)).collect();
        crate::ui_browser::dropdown(ui, &sresp, &format!("mfxselpop{u}"), |ui| {
            for (i, n) in names.iter().enumerate() {
                if ui.selectable_label(sel == i, n).clicked() {
                    app.prefs.fx.mfx.select[u] = i as u8;
                    app.save_prefs();
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
        let on = app.px.fxx.mfx.units[u].on;
        let or = h.r(212.0, 3.0, 44.0, 18.0);
        let oresp = ui.interact(or, ui.id().with(("mfxon", u)), Sense::click());
        let busy = app.px.fxx.mfx.units[u].phase_on && !on;
        p.rect_filled(or, 2.0, if on { BLUE.c() } else if busy { hex(0x1E468C) } else { crate::ui_feel::hl(&oresp, BTN.c(), lighten(BTN.c())) });
        p.text(or.center(), Align2::CENTER_CENTER, "ON", caps(9.0), WHITE.c());
        if oresp.on_hover_text("MERGE FX ON/OFF (Shift + click: cancel)").clicked() {
            let shift = ui.input(|i| i.modifiers.shift);
            mfx::toggle(app, u, shift);
        }
        let kv = app.px.fxx.mfx.units[u].knob;
        if let Some(v) = fx_knob(ui, h.r(266.0, 1.0, 22.0, 22.0).center(), ("mfxknob", u), 0.5 + 0.5 * kv, 0.5, true) {
            if on {
                app.px.fxx.mfx.units[u].knob = (v - 0.5) * 2.0;
            }
        }
        let comps = app.prefs.fx.mfx.comps[sel];
        p.text(pos2(h.r(296.0, 0.0, 1.0, 24.0).min.x, r.center().y), Align2::LEFT_CENTER, format!("{} + {}", mfx::name(comps[0]), mfx::name(comps[1])), FontId::proportional(8.5), DIM2.c());
    }
    // Sample volume (-12 .. +12 dB) and the settings.
    let vol = app.prefs.fx.mfx.volume;
    p.text(l.r(1092.0, 0.0, 60.0, 24.0).center(), Align2::CENTER_CENTER, "SAMPLE VOL", caps(8.0), WHITE.c());
    if let Some(v) = fx_knob(ui, l.r(1160.0, 1.0, 22.0, 22.0).center(), "mfxvol", (vol + 12.0) / 24.0, 0.5, true) {
        app.prefs.fx.mfx.volume = ((v * 24.0 - 12.0) * 10.0).round() / 10.0;
        app.px.fxx.pending.get_or_insert_with(std::time::Instant::now);
    }
    p.text(l.r(1186.0, 0.0, 48.0, 24.0).center(), Align2::CENTER_CENTER, format!("{vol:+.1}dB"), bold(8.5), WHITE.c());
    let gr = l.r(1251.0, 3.0, 18.0, 18.0);
    let gresp = ui.interact(gr, ui.id().with("mfxsettings"), Sense::click());
    p.rect_filled(gr, 2.0, crate::ui_feel::hl(&gresp, BTN.c(), lighten(BTN.c())));
    let c = gr.center();
    p.circle_stroke(c, 3.2, Stroke::new(1.6_f32, WHITE));
    for i in 0..6 {
        let a = i as f32 * std::f32::consts::TAU / 6.0;
        p.line_segment([c + vec2(a.cos(), a.sin()) * 4.0, c + vec2(a.cos(), a.sin()) * 5.6], Stroke::new(1.6_f32, WHITE));
    }
    crate::ui_browser::dropdown(ui, &gresp, "mfxsettingspop", |ui| {
        // "MERGE FX Settings": for each of MERGE FX 1..4 a preset and its four parts.
        ui.set_min_width(640.0);
        ui.label(egui::RichText::new("MERGE FX Settings").strong()).on_hover_text(
            "Factory sounds are read from rekordbox/Sampler/MERGE FX in your Music folder. Place the original WAV files or MERGE FX.zip there, or set RECORDBOX_MERGE_FX_ARCHIVE to a local ZIP before starting recordbox. REPEAT SAMPLE and SAMPLER use the file you choose from a sampler bank."
        );
        egui::Grid::new("mfxgrid").num_columns(6).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("");
            ui.label("PRESET");
            for (title, _) in mfx::COLUMNS {
                ui.label(title);
            }
            ui.end_row();
            for i in 0..4usize {
                ui.label(format!("MERGE FX {}", i + 1));
                let cur = app.prefs.fx.mfx.preset[i].min(7) as usize;
                let custom = app.prefs.fx.mfx.comps[i] != mfx::PRESETS[cur].1;
                egui::ComboBox::from_id_salt(("mfxpreset", i)).width(110.0).selected_text(if custom { format!("{} *", mfx::PRESETS[cur].0) } else { mfx::PRESETS[cur].0.to_string() }).show_ui(ui, |ui| {
                    for (k, (name, comps)) in mfx::PRESETS.iter().enumerate() {
                        if ui.selectable_label(cur == k && !custom, *name).clicked() {
                            app.prefs.fx.mfx.preset[i] = k as u8;
                            app.prefs.fx.mfx.comps[i] = *comps;
                            app.prefs.fx.mfx.paths[i] = Default::default();
                            app.save_prefs();
                        }
                    }
                });
                for (col, (_, ids)) in mfx::COLUMNS.iter().enumerate() {
                    let id = app.prefs.fx.mfx.comps[i][col];
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("mfxcomp", i, col)).width(120.0).selected_text(mfx::name(id)).show_ui(ui, |ui| {
                            for k in ids.iter() {
                                if ui.selectable_label(id == *k, mfx::name(*k)).clicked() {
                                    app.prefs.fx.mfx.comps[i][col] = *k;
                                    app.save_prefs();
                                }
                            }
                        });
                        // BUILD SAMPLE and DROP SAMPLE: the file of REPEAT SAMPLE / SAMPLER, from
                        // the sampler's slots (rekordbox's sample button and its menu).
                        if col == 1 || col == 3 {
                            mfx_sample_button(app, ui, i, col / 2, mfx::path_selectable(id));
                        }
                    });
                }
                if ui.button("RESET").clicked() {
                    app.prefs.fx.mfx.comps[i] = mfx::PRESETS[cur].1;
                    app.prefs.fx.mfx.paths[i] = Default::default();
                    app.save_prefs();
                }
                ui.end_row();
            }
        });
    });
}

/// The sample button beside a MERGE FX column: lit while a file is chosen, enabled for
/// REPEAT SAMPLE / REPEAT UP SAMPLE / SAMPLER; its menu lists the file in use, then BANK 1..4
/// with the sampler's slots, the chosen one ticked (`PresetComponent::showSampleSelectionAt`).
fn mfx_sample_button(app: &mut App, ui: &mut Ui, i: usize, slot: usize, enabled: bool) {
    let path = app.prefs.fx.mfx.paths[i][slot].clone();
    let stem = |p: &str| std::path::Path::new(p).file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
    let lit = !path.is_empty();
    let resp = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new("SAMPLE").size(9.0)).selected(lit)).on_hover_text(if lit { stem(&path) } else { "No sample selected".into() });
    let mut pick = None;
    crate::ui_browser::dropdown(ui, &resp, &format!("mfxsample{i}{slot}"), |ui| {
        let banks = crate::mfx::sample_menu(&app.prefs.sampler.slots);
        ui.set_min_width(180.0);
        if lit {
            ui.add_enabled(false, egui::Button::new(stem(&path)).frame(false));
            ui.separator();
        }
        for (b, list) in banks.iter().enumerate() {
            ui.menu_button(format!("BANK {}", b + 1), |ui| {
                for (name, p) in list {
                    if ui.selectable_label(*p == path, name).clicked() {
                        pick = Some(p.clone());
                        ui.close_menu();
                    }
                }
            });
        }
    });
    if let Some(p) = pick {
        app.prefs.fx.mfx.paths[i][slot] = p;
        app.save_prefs();
    }
}

/// Preferences > Controller > Effect, after rekordbox 7.2.19 (reference preferences/09):
/// RELEASE FX (Unit Number 1 / 2, Apply RELEASE FX on CFX) and MERGE FX (Enable MERGE FX,
/// Stop playing the operating deck after MERGE FX finishes). The CFX card after them is the
/// row rekordbox shows with hardware that has a MIC COLOR knob; recordbox has that knob on
/// screen (S in the CFX row).
pub fn prefs_effect_page(app: &mut App, ui: &mut Ui) {
    use crate::ui_kit as kit;
    let mut changed = false;
    kit::card(ui, "RELEASE FX", |ui| {
        kit::sub_help(ui, "Unit Number", Some("1: one RELEASE FX, applied to the master output (MASTER RELEASE FX).\n2: a RELEASE FX in each of FX1 and FX2, applied to the channels the unit is assigned to."));
        for n in [1u8, 2] {
            if kit::radio(ui, app.prefs.fx.effect.release_units == n, &n.to_string()) {
                app.prefs.fx.effect.release_units = n;
                changed = true;
            }
        }
        kit::separator(ui);
        // (This switch stays at the radios' indent, as in the reference.)
        kit::indent(ui, 84.0);
        changed |= kit::toggle(ui, &mut app.prefs.fx.effect.release_on_cfx, "Apply RELEASE FX on CFX");
        // (Measured on the reference: this card is 196 pt high, the next one 140 pt.)
        ui.add_space(-4.5);
    });
    kit::card(ui, "MERGE FX", |ui| {
        changed |= kit::toggle(ui, &mut app.prefs.fx.mfx.enabled, "Enable MERGE FX");
        kit::separator(ui);
        changed |= kit::toggle(ui, &mut app.prefs.fx.mfx.stop_deck, "Stop playing the operating deck after MERGE FX finishes");
        ui.add_space(-0.5);
    });
    kit::card(ui, "CFX", |ui| {
        changed |= kit::toggle(ui, &mut app.prefs.sampler.cfx_on_sampler, "Apply CFX to Sampler using MIC COLOR control knob.");
        kit::note(ui, "The S knob of the CFX row in the FX panel then acts on the sampler deck.");
    });
    if changed {
        app.save_prefs();
    }
}

/// The scripted run of the FX panel (`PREPDECK_SHOT_VIEW=perffx2` with
/// `PREPDECK_SHOT_PLAYER=perftest`): single mode, CFX multi mode, MERGE FX and the PAD FX
/// pads, with real pointer events at the places the controls are drawn (1512 pt wide).
pub fn selftest_steps(app: &App) -> Vec<(&'static str, egui::Pos2, egui::Pos2)> {
    let k = 1512.0 / 1280.0;
    let top = 28.0 + 3.0;
    let (y1, y2, ya, cy) = (top + 11.0, top + 37.0, top + 16.0, 28.0 + 66.0);
    // (MERGE FX's row sits under the CFX row.)
    let my = cy + MFX_ROW_H;
    let p = |x: f32, y: f32| (pos2(x * k, y), pos2(x * k, y));
    let st = |n: &'static str, (a, b): (egui::Pos2, egui::Pos2)| (n, a, b);
    let pad = |i: usize| { let c = app.px.fxx.pad.rects[0][i].center(); (c, c) };
    vec![
        st("FX1 assign deck 1", p(73.0, ya)),
        st("FX1 single mode", p(144.0, top + 35.0)),
        st("FX1 single effect on (DELAY)", p(198.0, y1)),
        ("FX1 single knob 1 up", pos2(275.0 * k, y2), pos2(275.0 * k, y2 - 30.0)),
        st("FX1 single button 1 (LOW CUT)", p(312.0, y2)),
        st("FX1 single beat up", p(242.5, y2)),
        st("FX1 snapshot", p(425.0, y1)),
        st("FX1 multi mode", p(144.0, top + 15.0)),
        st("FX1 slot 3 (SPIRAL) on", p(163.0 + 188.0 + 35.0, y1)),
        st("FX1 slot 3 beat down", p(190.0 + 188.0 + 7.5, y2)),
        st("FX2 single mode", p(641.0 + 144.0, top + 35.0)),
        st("CFX multi mode", p(79.0, cy)),
        st("CFX multi: deck 1's effect off", p(505.0 + 55.0, cy)),
        st("CFX multi: deck 1's effect on", p(505.0 + 55.0, cy)),
        ("CFX multi: deck 1's COLOR to the right", pos2((505.0 + 121.0) * k, cy), pos2((505.0 + 121.0) * k, cy - 40.0)),
        st("CFX single mode", p(114.0, cy)),
        st("CFX multi mode again (deck 1 keeps its COLOR)", p(79.0, cy)),
        ("CFX multi: deck 1's COLOR back to the centre", pos2((505.0 + 121.0) * k, cy), pos2((505.0 + 121.0) * k, cy + 40.0)),
        st("FX1 single mode again (snapshot kept)", p(144.0, top + 35.0)),
        st("FX1 single effect off", p(198.0, y1)),
        // PAD FX on deck 1 (standard preset, PAD FX1): pad 5 ECHO 1/4, pad 4 V.BRAKE.
        st("FXCALL:padfx deck 1 shows PAD FX", p(0.0, 0.0)),
        st("PAD FX pad 5 (ECHO 1/4) held", pad(4)),
        st("PAD FX pad 1 (ROLL 1/2) held", pad(0)),
        st("PAD FX pad 4 (V.BRAKE 3/4, release) held", pad(3)),
        st("PAD FX bank 2", pad(9)),
        st("PAD FX pad 5 of bank 2 (REV ROLL 1/16) held", pad(4)),
        st("PAD FX edit mode on", pad(10)),
        st("PAD FX edit mode off", pad(10)),
        st("PAD FX bank 1", pad(8)),
        st("PAD FX page 9 - 16", pad(11)),
        st("PAD FX pad 9 (TRANS 1/2) held", pad(0)),
        st("PAD FX page 1 - 8", pad(11)),
        // MERGE FX: enabled in Preferences, its row shown; unit 1 (deck 1, MERGE FX 1 = SNARE
        // RISER) on, the knob up, off again: release (2.5 s), drop, deck 1 stops.
        st("FXCALL:mfx enabled and its row shown", p(0.0, 0.0)),
        st("MERGE FX unit 1 ON", p(234.0, my)),
        ("MERGE FX unit 1 knob to the right", pos2(277.0 * k, my), pos2(277.0 * k, my - 50.0)),
        st("FXCALL:wait while it builds", p(0.0, 0.0)),
        st("PAD FX pad 5 while MERGE FX is on (refused)", pad(4)),
        st("MERGE FX unit 1 OFF (release starts)", p(234.0, my)),
        st("FXCALL:wait 1 (release)", p(0.0, 0.0)),
        st("FXCALL:wait 2 (release)", p(0.0, 0.0)),
        st("FXCALL:wait 3 (release)", p(0.0, 0.0)),
        st("FXCALL:wait 4 (drop: deck 1 stopped)", p(0.0, 0.0)),
        st("FXCALL:wait 5 (at rest)", p(0.0, 0.0)),
        st("FXCALL:play deck 1 again", p(0.0, 0.0)),
        st("MERGE FX unit 1 ON again", p(234.0, my)),
        st("FXCALL:cancel unit 1 (Shift + ON)", p(0.0, 0.0)),
        st("FXCALL:wait 6 (cancelled: no drop)", p(0.0, 0.0)),
    ]
}

/// A step of the scripted run that is not a pointer event.
pub fn selftest_call(app: &mut App, name: &str, f: u32, raw: &mut egui::RawInput) {
    if name.contains("wait") {
        // (About 0.65 s a step, so the audio, which runs in real time, gets somewhere.)
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    if f == 0 {
        raw.events.push(egui::Event::PointerMoved(pos2(700.0, 500.0)));
    }
    if f == 3 {
        if name.contains("padfx deck 1") {
            app.px.pad_mode[0] = crate::ui_padfx::PAD_FX;
        } else if name.contains("mfx enabled") {
            app.prefs.fx.mfx.enabled = true;
            app.px.fxx.mfx.show = true;
        } else if name.contains("play deck 1") {
            app.with_deck(0, |a| {
                if !a.player.state.lock().playing {
                    a.play_pause();
                }
            });
        } else if name.contains("cancel unit 1") {
            crate::mfx::toggle(app, 0, true);
        }
    }
    if f == 30 {
        eprintln!("perftest: {name}: {}", selftest_state(app));
    }
}

/// PAD FX and MERGE FX as the interface and the audio thread hold them.
pub fn selftest_state(app: &App) -> String {
    let d = &app.px.fxx.pad.decks[0];
    let m = &app.px.fxx.mfx.units[0];
    let audio = match fx::RACK.lock().as_ref() {
        Some(r) => {
            let s = r.pad_set[0];
            format!(
                "pad unit made {} beat {:?} colour {:?} release {:?} | merge {} | rack in use {}",
                r.has_pad(0),
                s.beat.map(|b| (fx::name(b.kind), b.value, (b.level * 100.0).round())),
                s.color,
                s.release,
                r.merge.as_ref().map(|e| format!("{:?} knob {:.2} drops {}", e.phase(0), e.cmd[0].knob, e.dropped[0])).unwrap_or("not made".into()),
                r.active()
            )
        }
        None => "no rack".into(),
    };
    format!(
        "PAD FX deck 1: mode {} bank {} page {} edit {} held {:?} lit beat {:?} colour {:?} release {:?} slip {:?} | MERGE FX: enabled {} unit 1 on {} knob {:.2} busy {} | deck 1 playing {} | audio: {audio} | toast {:?}",
        app.px.pad_mode[0] == crate::ui_padfx::PAD_FX,
        app.prefs.fx.padfx.bank[0] + 1,
        app.deck_of(0).x.pad_page + 1,
        d.edit,
        d.held,
        d.beat,
        d.color,
        d.release,
        d.slip,
        app.prefs.fx.mfx.enabled,
        m.on,
        m.knob,
        m.phase_on,
        app.player.decks[0].lock().playing,
        app.toasts.last().map(|t| t.text.clone())
    )
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panels_choices_survive_a_round_trip() {
        // Defaults as rekordbox's file: DELAY / ECHO / SPIRAL by engine id 0 / 1 / 3, every
        // beat index at its table default, RELEASE FX 0 with 4 / 5 / 7.
        let d = UnitSaved::default();
        assert_eq!(d.multi_fx, [0, 1, 3]);
        assert_eq!((d.release_fx, d.release_beat), (0, [4, 5, 7]));
        assert_eq!(d.single_beat.len(), 34);
        assert_eq!(d.multi_beat[0].len(), 35);
        // Single ids: 0 DELAY (B, 5), 4 REVERB (P1, 3), 6 FILTER (D, 6), 26 ENIGMA JET (S, 17),
        // 32 VINYL BRAKE (C, 6), 33 STRETCH (C, 5), 12 ROBOT (P2, 3), 16 PITCH (P2, 0).
        for (id, want) in [(0, 5), (4, 3), (6, 6), (26, 17), (32, 6), (33, 5), (12, 3), (16, 0)] {
            assert_eq!(d.single_beat[id], want, "single id {id}");
        }
        // Change everything that is kept, write it as JSON, read it back into a new unit.
        let mut u = FxUi::new();
        u.set_kind(0, Kind::Helix);
        u.set_index(0, 2);
        u.set_kind(1, Kind::EnigmaJet);
        u.set_index(1, 3);
        u.set_kind(2, Kind::ReverbUp);
        u.set_kind(3, Kind::PitchEcho);
        u.set_index(3, 8);
        u.params.one.knob = [0.2, 0.9];
        u.params.one.btn = [true, false];
        u.snaps[pos(Kind::PitchEcho)] = Some((0.2, 0.9, true, false));
        u.params.single = true;
        u.release_kind = 2;
        u.release_idx = [1, 2, 9];
        // Going to another effect and back keeps the first one's beat value.
        u.set_kind(0, Kind::Trans);
        u.set_kind(0, Kind::Helix);
        assert_eq!(u.idx[0], 2);
        let all = FxSaved { units: [u.saved(), UnitSaved::default()], cfx: CfxSaved { multi_shown: true, user: true, user_fx: 6, params: [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9], multi_fx: [0, 1, 2, 3, 4, 8] }, ..FxSaved::default() };
        let json = serde_json::to_string(&all).unwrap();
        let back: FxSaved = serde_json::from_str(&json).unwrap();
        assert_eq!(back, all);
        let mut v = FxUi::new();
        v.load(&back.units[0]);
        assert_eq!(v.params.slots.map(|s| s.kind), [Kind::Helix, Kind::EnigmaJet, Kind::ReverbUp]);
        assert_eq!(v.idx, [2, 3, 0]);
        assert_eq!(v.params.slots[1].value, -8.0);
        assert!(v.params.single);
        assert_eq!((v.params.one.kind, v.one_idx, v.params.one.value), (Kind::PitchEcho, 8, 8.0));
        assert_eq!((v.params.one.knob, v.params.one.btn), ([0.2, 0.9], [true, false]));
        assert_eq!((v.release_kind, v.release_idx), (2, [1, 2, 9]));
        assert_eq!(v.saved(), all.units[0]);
        // Nothing switched on by loading.
        assert!(!v.params.one.on && v.params.slots.iter().all(|s| !s.on));
        // An older settings file without the FX part, and one with a broken part, load.
        let old: FxSaved = serde_json::from_str("{}").unwrap();
        assert_eq!(old, FxSaved::default());
        let odd: FxSaved = serde_json::from_str(r#"{"units":[{"single_fx":99,"multi_fx":[-1,77,5],"multi_beat":[[200],[],[]]},{}]}"#).unwrap();
        let mut w = FxUi::new();
        w.load(&odd.units[0]);
        assert_eq!(w.params.slots.map(|s| s.kind), [Kind::Delay, Kind::Echo, Kind::Trans]);
        assert_eq!(w.idx[0], 9);
    }

    #[test]
    fn single_mode_read_outs() {
        // rekordbox's read-outs (`convertSingleParamString`).
        use fx::{single_knob_text as t, Kind as K};
        assert_eq!(t(K::Echo, 0, 0.5), "0.5");
        assert_eq!(t(K::Echo, 1, 1.0), "HPF 100");
        assert_eq!(t(K::Echo, 1, 0.5), "0");
        assert_eq!(t(K::Delay, 1, 0.0), "L:100%");
        assert_eq!(t(K::Spiral, 1, 0.0), "-50%");
        assert_eq!(t(K::Spiral, 1, 1.0), "+100%");
        assert_eq!(t(K::Phaser, 1, 1.0), "R STAGE:60");
        assert_eq!(t(K::Trans, 0, 1.0), "99%");
        assert_eq!(t(K::MtDelay, 1, 0.5), "4");
        assert_eq!(t(K::VinylBrake, 1, 0.5), "NONE");
        assert_eq!(t(K::VinylBrake, 0, 1.0), "4");
        assert_eq!(t(K::Stretch, 0, 0.0), "L:100%");
        assert_eq!(knob_text("FEEDBACK", 0.5), "50");
        assert_eq!(knob_text("PITCH", 0.5), "0");
        assert_eq!(knob_text("PITCH", 0.505), "0");
        assert_eq!(knob_text("PITCH", 1.0), "+100");
        assert_eq!(knob_text("PITCH", 0.0), "-100");
        assert_eq!(knob_text("STAGE", 0.5), "6");
        assert_eq!(knob_text("STAGE", 0.0), "60");
    }
}
