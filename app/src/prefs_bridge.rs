//! Preferences rows that belong to features in other files: the Preferences pages keep every
//! rekordbox setting in `prefs.rb` (prefs_rb.rs), and the deck engine, the pads, Groove
//! Circuit and Traffic Light keep the ones they act on in their own settings. This copies the
//! values across so that a row changed in Preferences takes effect, and holds the small rules
//! of the rows that act at one place in another file (each such file calls one function here).
//!
//! Traced in rekordbox 7.2.19 (arm64, Hex-Rays; addresses in the functions' docs):
//! Auto Cue, the load start position, Smart Cue, Gate Cue, Memory Cue Call Lock, the quantize
//! mode, Continuous Playing, the played time, Hide Title / Artist / Artwork and the waveform
//! drawing rate.

use crate::app::*;
use crate::deck::QType;
use crate::model::*;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// Gate Cue (`HotCueSampler`) for the MIDI workers' hot cue path (midi.rs `Hub::hot_cue`).
pub static GATE_CUE: AtomicBool = AtomicBool::new(false);
/// View > Display Type > "Display key information on the database" (`ShowOriginalKey`): the
/// key as the library spelled it, else in the chosen notation (model.rs `Track::key_label`).
pub static SHOW_ORIGINAL_KEY: AtomicBool = AtomicBool::new(true);
/// Frames per second the waveforms are drawn at (Waveform Drawing Rate); 0 = every frame.

/// State of the rows that act over time (a field of `App`).
#[derive(Default)]
pub struct WiringState {
    /// Continuous Playing: a deck whose track ended, waiting for the next one to be read so
    /// that it can start it.
    pub auto_play: [Option<i64>; 4],
    /// A deck was playing in the last frame.
    was_playing: [bool; 4],
    /// Quantized reverse waiting for its beat: (deck, the time it turns at, slip).
    pub reverse_at: [Option<(f64, bool)>; 4],
    /// The Auto Gain setting the decks' gains were last set for.
    auto_gain_seen: Option<bool>,
    /// The Tag List's "delete when loaded" rule: the tracks already taken off for a load.
    last_loaded: [Option<i64>; 4],
    /// A track row held for a second: where to open its menu (as a right-click, next frame),
    /// and the press that already opened one.
    pub long_press: Option<(eframe::egui::ViewportId, eframe::egui::Pos2)>,
    pub long_press_done: Option<f64>,
}

impl App {
    /// Called at start-up and on every frame the Preferences window is open.
    pub fn prefs_bridge(&mut self) {
        let p = &mut self.prefs;
        let rb = &p.rb;
        // ---- the deck engine and the mixer (deck.rs)
        let d = &mut p.deck;
        d.needle_lock = rb.needle_lock;
        d.vinyl_brake = rb.brake;
        d.vinyl_start = rb.start;
        d.jog_45 = rb.jog_speed == 2;
        d.beat_sync = rb.beat_sync_type != 2;
        d.multi_bpm_sync = rb.multi_bpm_sync;
        d.show_sync_rate = rb.show_sync_rate;
        d.quantize_one_go = rb.quantize_in_one_go;
        d.crossfader_curve = rb.crossfader_curve.min(32);
        d.headroom = rb.headroom.clamp(1, 8);
        d.eq_nxs = rb.eq_type == 0;
        d.hot_cue_countdown = rb.jog_indicator == 2;
        // Controller > Others > Quantize > Type: the beat values of SNAP, HOT CUE, LOOP and
        // REVERSE (`PlayerWaveView::updateQuantizeSettings` 0x100c8f10c hands each type's
        // status and beat to the engine).
        d.quantize_units = [rb.snap.beat.clamp(2, 6), rb.q_hot_cue.beat.clamp(2, 6), rb.q_loop.beat.clamp(2, 6), rb.q_reverse.beat.clamp(2, 6)];
        // ---- pads (pads.rs): PadModeLayoutType 1 Auto, 2 LIST Display, 3 PAD Display
        p.pads.layout = if rb.pad_mode_custom { rb.pad_mode_layout.clamp(1, 2) + 1 } else { 1 };
        p.pads.hot_cue_color = rb.hot_cue_color.clamp(1, 4);
        // ---- Groove Circuit (groove.rs)
        let g = &mut p.groove;
        g.enable = rb.groove_circuit;
        g.load_lock = rb.drum_swap_lock;
        g.show_title = rb.drum_swap_title;
        g.capture_stem = rb.drum_capture_stem;
        g.capture_dir = rb.drum_capture_dir.clone();
        g.auto_import = rb.drum_auto_import;
        // ---- Traffic Light (traffic.rs)
        p.traffic.range = rb.traffic_light.clamp(1, 4);
        // ---- the rows read from other threads
        GATE_CUE.store(rb.gate_cue, Relaxed);
        // ---- Histories: the played time (library_ext.rs `PlayCredit`)
        self.ext.credit.need = self.prefs.rb.played_time.clamp(1, 60) as f64;
    }
}

/// Controller > Deck > AUTO BEAT LOOP: the length a controller's (or a key's) Auto Beat Loop
/// starts with. 1 "Loop for specified beats" is the size the deck's loop box shows; 2..7 are
/// 1, 2, 4, 8, 16 and 32 beats (`RDSDDJAutoBeatLoop`, the entries in rekordbox's order).
pub fn auto_beat_loop_beats(p: &Prefs) -> f64 {
    match p.rb.auto_beat_loop {
        2 => 1.0,
        3 => 2.0,
        4 => 4.0,
        5 => 8.0,
        6 => 16.0,
        7 => 32.0,
        _ => p.loop_beats,
    }
}

// ------------------------------------------------------------------ Controller > Deck

/// Auto Cue Level: the entry's id (1..8) as a level in dB (`UiPlayer::onAutoCueLevelSetting`
/// 0x101aa44f4: -36 - 6 (id - 1); any other id is -60 dB).
pub fn auto_cue_db(id: u8) -> f32 {
    if (1..=8).contains(&id) { -36.0 - 6.0 * (id as f32 - 1.0) } else { -60.0 }
}

/// The first sound of a track for Auto Cue (`AutoCueBehavior::execSearchAutoCuePoint`
/// 0x102b00030): the first frame where a channel reaches the level (`10^(dB/20)`, linear
/// peak), taken down to a whole unit of `sr / 150` frames (`prepareToPlay` 0x102afff74).
/// None when nothing reaches the level.
pub fn auto_cue_frame(samples: &[f32], sr: u32, db: f32) -> Option<usize> {
    let level = 10f32.powf(db * 0.05);
    let unit = ((sr as f64 * 0.006_666_666_67) as usize).max(1);
    let f = samples.chunks_exact(2).position(|s| s[0].abs().max(s[1].abs()) >= level)?;
    Some(f / unit * unit)
}

/// Deck `n` (0 = deck 1) has Auto Cue on (Preferences > Controller > Deck, PERFORMANCE).
pub fn auto_cue_on(app: &App, n: u8) -> bool {
    app.performance() && app.prefs.rb.auto_cue_deck.get(n as usize).copied().unwrap_or(false)
}

/// The deck's AUTO CUE switch (a controller's button, or the key): Auto Cue of deck `n` on /
/// off, as Preferences shows it.
pub fn toggle_auto_cue(app: &mut App, n: u8) {
    if let Some(v) = app.prefs.rb.auto_cue_deck.get_mut(n as usize) {
        *v = !*v;
        let on = *v;
        app.save_prefs();
        app.toast(if on { "AUTO CUE ON" } else { "AUTO CUE OFF" });
    }
}

/// app.rs `load_track_here`, after the cue point was chosen: "Playback starts at Memory/Hot
/// Cue nearest to the beginning of the track" (`UiPlayer::setCurrentCueAfterLoad`
/// 0x101ba0624, PERFORMANCE only; `findFirstCuePoint` 0x101ba08ec takes the first memory cue
/// and the earliest hot cue, whichever is first).
pub fn on_load(app: &mut App) {
    if !(app.performance() && app.prefs.rb.load_start_cue) {
        return;
    }
    if let Some(t) = load_start_point(&app.deck.cues) {
        app.deck.cue_point = t;
    }
}

/// The first memory cue or hot cue of a track (loops by their start).
pub fn load_start_point(cues: &[Cue]) -> Option<f64> {
    let first = cues.iter().filter(|c| matches!(c.kind, CueKind::Memory | CueKind::Hot)).map(|c| c.start).fold(f64::NAN, f64::min);
    first.is_finite().then(|| first.max(0.0))
}

/// app.rs `poll_load`, once the audio has been read: Auto Cue puts the cue point at the
/// first sound of the track, when the track has no memory cue (recordbox's choice where
/// rekordbox was not traced: its first memory cue, or the load start position, win).
pub fn after_load(app: &mut App, a: &crate::audio::Decoded) {
    let n = app.px.active;
    if !auto_cue_on(app, n) {
        return;
    }
    let has_memory = app.deck.cues.iter().any(|c| c.kind == CueKind::Memory);
    if has_memory || (app.prefs.rb.load_start_cue && app.deck.cues.iter().any(|c| c.kind == CueKind::Hot)) {
        return;
    }
    if let Some(f) = auto_cue_frame(&a.samples, a.sr, auto_cue_db(app.prefs.rb.auto_cue_level)) {
        app.deck.cue_point = f as f64 / a.sr.max(1) as f64;
    }
}

/// Memory Cue Call Lock (PERFORMANCE): a memory cue is not called while the deck plays
/// (`WidgetPlayerDJ::operate` 0x100692744; rekordbox's message). True: refused.
pub fn memory_call_locked(app: &mut App) -> bool {
    if app.performance() && app.prefs.rb.memory_cue_call_lock && app.playing() {
        app.toast_err("Memory Cue Call Lock. Not available during playback.");
        return true;
    }
    false
}

/// Smart Cue (Controller > Deck > HOT CUE): a hot cue that is set or played becomes the
/// deck's cue point (`UiPlayer::onHotCueLaunched` 0x101ba4f78, `onHotCueChanged`).
pub fn smart_cue(app: &mut App, start: f64) {
    if app.prefs.rb.smart_cue {
        app.deck.cue_point = start;
    }
}

/// Gate Cue: a hot cue played from a paused deck plays only while it is held. Off (the
/// default), it plays on.
pub fn gate_cue(app: &App) -> bool {
    app.prefs.rb.gate_cue
}

/// Controller > Others > Quantize: the type an action is snapped with, or None when that
/// type's QUANTIZE is off. "Jump before reaching the next beat." (`DeckQuantizeMode` 2, the
/// default) gives every type the half-beat threshold of SNAP (`setQuantizeMode(mode == 1)`
/// at 0x100c8f408 switches the engine's per-type thresholds on only for mode 1:
/// `DjPlayerQuantizePolicy::thresholdPpq` falls back to `PPQ_8TH_NOTE` = 0.5 beat), so a
/// press just after a beat acts at once; off, HOT CUE and LOOP wait for the next point unless
/// one was passed by less than 1/32 beat.
pub fn quantize_kind(app: &App, kind: QType) -> Option<QType> {
    quantize_kind_for(&app.prefs.rb, kind)
}

pub fn quantize_kind_for(rb: &crate::prefs_rb::RbPrefs, kind: QType) -> Option<QType> {
    let on = match kind {
        QType::Snap => true,
        QType::HotCue => rb.q_hot_cue.on,
        QType::Loop => rb.q_loop.on,
        QType::Reverse => rb.q_reverse.on,
    };
    if !on {
        return None;
    }
    Some(if rb.quantize_jump_before { QType::Snap } else { kind })
}

/// Loop In / Loop Out with Q on: in PERFORMANCE on the LOOP/SAMPLER(LOOP) row's beat value
/// and switch (Controller > Others > Quantize), elsewhere on SNAP's grid as before.
pub fn loop_point(app: &App, t: f64) -> f64 {
    if app.performance() {
        if !app.prefs.quantize || app.deck.beats.len() < 2 {
            return t;
        }
        app.quantize_action(t, QType::Loop)
    } else {
        app.quantized(t)
    }
}

/// deck.rs `reverse_toggle`: with Q on and a grid, the turn of REVERSE waits for the REVERSE
/// beat value (the "quantized reverse" of the deck notes; the turn is made at the first frame
/// past the point, so up to one frame late). Returns true when the turn was put off (or
/// cancelled by a second press).
pub fn reverse_quantized(app: &mut App, slip: bool) -> bool {
    let ni = (app.px.active as usize).min(3);
    if app.wiring.reverse_at[ni].take().is_some() {
        return true;
    }
    // (SLIP REVERSE acts while held: it turns at once, as before.)
    if slip {
        return false;
    }
    if !(app.performance() && app.prefs.quantize && app.playing() && app.deck.beats.len() >= 2) || quantize_kind(app, QType::Reverse).is_none() {
        return false;
    }
    let p = app.pos();
    let at = app.quantize_action(p, QType::Reverse);
    let backwards = app.reversed();
    let reached = if backwards { at >= p } else { at <= p };
    if reached {
        return false;
    }
    app.wiring.reverse_at[ni] = Some((at, slip));
    true
}

/// Waveform Drawing Rate (`ViewWaveformDrawRateComponent::buttonClicked` 0x100ff6a8c): High
/// Speed draws at 120 frames a second, Medium and Low Speed at 40 (Low also turns rekordbox's
/// OpenGL off). recordbox draws everything in one pass, so the rate is that of the window.
pub fn draw_fps(rate: u8) -> u32 {
    match rate {
        1 | 2 => 40,
        _ => 0,
    }
}

/// Once per frame (main.rs): the rows that act over time.
pub fn frame(app: &mut App) {
    // The values other threads read, also when Preferences is closed.
    GATE_CUE.store(app.prefs.rb.gate_cue, Relaxed);
    XML_BEATGRID.store(app.prefs.rb.xml_beatgrid.min(1) as i64, Relaxed);
    SHOW_ORIGINAL_KEY.store(app.prefs.rb.show_original_key, Relaxed);
    app.ext.credit.need = app.prefs.rb.played_time.clamp(1, 60) as f64;
    let count = app.deck_count().min(4);
    for n in 0..count {
        let ni = n as usize;
        // ---- quantized reverse: the turn on its beat
        if let Some((at, slip)) = app.wiring.reverse_at[ni] {
            let (p, playing) = {
                let s = app.player.decks[ni].lock();
                (s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0), s.playing)
            };
            let back = app.deck_of(n).x.dk.reverse;
            // (A deck that stops before the beat turns at once.)
            if !playing || (!back && p >= at) || (back && p <= at) {
                app.wiring.reverse_at[ni] = None;
                app.with_deck(n, |a| {
                    if a.reversed() {
                        a.reverse_off();
                    } else {
                        // (Taken again by `reverse_quantized`, which now finds nothing waiting
                        // and no beat ahead.)
                        a.with_reverse_now(slip);
                    }
                });
            }
        }
        // ---- Continuous Playing (PERFORMANCE; `ListViewer::canLoadNextTrack` 0x10040f380:
        // not while Automix has the deck)
        let (playing, at_end, loaded) = {
            let s = app.player.decks[ni].lock();
            let end = s.audio.as_ref().map(|a| s.pos >= (a.frames() as f64 - 1.0).max(0.0)).unwrap_or(false);
            (s.playing, end, s.audio.is_some())
        };
        let automix = app.mixx.automix.engine.processing && ni < 2;
        if app.performance() && app.prefs.rb.auto_play && !automix {
            if app.wiring.was_playing[ni] && at_end && app.wiring.auto_play[ni].is_none() {
                let before = app.deck_of(n).track;
                app.with_deck(n, |a| a.load_adjacent(1));
                let after = app.deck_of(n).track;
                if after != before {
                    app.wiring.auto_play[ni] = after;
                }
            }
            if let Some(id) = app.wiring.auto_play[ni] {
                let d = app.deck_of(n);
                if d.track != Some(id) {
                    app.wiring.auto_play[ni] = None;
                } else if d.audio.is_some() && d.loading.is_none() {
                    app.wiring.auto_play[ni] = None;
                    app.with_deck(n, |a| a.set_playing(true));
                }
            }
        } else {
            app.wiring.auto_play[ni] = None;
        }
        app.wiring.was_playing[ni] = playing && loaded;
        // ---- Tag List: "Delete a track from Tag List when the track is loaded."
        // (`LoadedContentsManager::removeFromTaglist` 0x10139dfb8: only a track loaded from
        // the Tag List, which recordbox takes as the Tag List being the view shown.)
        let t = app.deck_of(n).track;
        if t != app.wiring.last_loaded[ni] {
            app.wiring.last_loaded[ni] = t;
            if let Some(id) = t {
                if tag_list_removes_on_load(&app.prefs, app.view == View::TagList, app.ext.tag_set.contains(&id)) {
                    app.tag_list_remove(&[id]);
                }
            }
        }
    }
    // ---- Mixer > Auto Gain (PERFORMANCE): off, the decks play at 0 dB whatever their
    // analysis found.
    let on = auto_gain_on(app);
    if app.wiring.auto_gain_seen != Some(on) {
        app.wiring.auto_gain_seen = Some(on);
        for n in 0..count {
            let id = app.deck_of(n).track;
            let g = id.and_then(|id| app.track(id)).map(|t| track_gain(app, t, on)).unwrap_or(1.0);
            app.player.decks[n as usize].lock().gain = g;
        }
    }
}

/// Mixer > Auto Gain is on in the mode showing (EXPORT always uses the tracks' gain).
pub fn auto_gain_on(app: &App) -> bool {
    !app.performance() || app.prefs.rb.auto_gain
}

/// A track's playing gain: its analysed gain (with the optional peak cap), or unity with Auto
/// Gain off.
pub fn track_gain(app: &App, t: &Track, on: bool) -> f32 {
    if !on {
        return 1.0;
    }
    let mut g = if t.gain_db.is_finite() { 10f32.powf(t.gain_db / 20.0).min(4.0) } else { 1.0 };
    if app.prefs.auto_gain_peak_cap {
        if let Some(pk) = app.db.peak(t.id).filter(|p| *p > 0.0) {
            if g * pk >= 1.0 {
                g = 0.999_999_88 / pk;
            }
        }
    }
    g
}

impl App {
    /// Reverse on at once (the quantized turn's moment has come).
    pub fn with_reverse_now(&mut self, slip: bool) {
        let ni = (self.px.active as usize).min(3);
        let q = self.prefs.quantize;
        // (Q off for this one call so that the turn is not put off again.)
        self.prefs.quantize = false;
        self.reverse_toggle(slip);
        self.prefs.quantize = q;
        self.wiring.reverse_at[ni] = None;
    }
}

// ------------------------------------------------------------------ View

/// View > Layout > Enlarged Waveform: the decks' order in the 4-deck layouts. Auto is
/// rekordbox's own (horizontal 1-2-3-4, vertical 3-1-2-4); Customize takes the chosen
/// order for each.
pub fn wave_order(app: &App, vertical: bool, count: u8) -> [u8; 4] {
    wave_order_for(&app.prefs.rb, vertical, count)
}

pub fn wave_order_for(rb: &crate::prefs_rb::RbPrefs, vertical: bool, count: u8) -> [u8; 4] {
    if count < 4 {
        return [0, 1, 2, 3];
    }
    let pick = if !rb.zoom_wave_custom { if vertical { 2 } else { 1 } } else if vertical { rb.zoom_wave_v } else { rb.zoom_wave_h };
    if pick == 2 { [2, 0, 1, 3] } else { [0, 1, 2, 3] }
}

/// View > Display Type > "Hide Title, Artist, and Artwork" (PERFORMANCE decks;
/// `TitleView::setEnternalInput` 0x101a57714 hides the title and artist, `setArtwork`
/// 0x101a5a65c shows the empty artwork).
pub fn hide_deck_info(app: &App) -> bool {
    app.performance() && app.prefs.rb.hide_deck_info
}

/// View > Layout > Effect Panel: the FX panel's height with the rows it shows (the Beat FX row
/// is 1 + 49 pt and 3 pt of black before the Sound Color FX row of 24 pt; 1 pt closes it).
pub fn fx_panel_height(app: &App) -> f32 {
    crate::ui_fx::fx_h(app)
}

// ------------------------------------------------------------------ Advanced

/// Advanced > Browse > Library Protection: edits of the library are refused with rekordbox's
/// message. True: refused.
pub fn library_protected(app: &mut App) -> bool {
    if app.prefs.rb.library_protection {
        app.toast_err("Unable to edit or delete because Library Protection is enabled.");
        return true;
    }
    false
}

/// The same for imports.
pub fn import_protected(app: &mut App) -> bool {
    if app.prefs.rb.library_protection {
        app.toast_err("Unable to import tracks because Library Protection is enabled.");
        return true;
    }
    false
}

/// "Delete a track from Tag List when the track is loaded.": a track loaded from the Tag List
/// (and on it) leaves it.
pub fn tag_list_removes_on_load(p: &Prefs, from_tag_list: bool, on_list: bool) -> bool {
    p.rb.tag_list_on_load && from_tag_list && on_list
}

/// "Delete all tracks from Tag List when exiting rekordbox." (off by default).
pub fn tag_list_clears_on_exit(p: &Prefs) -> bool {
    p.rb.tag_list_on_exit
}

/// Advanced > Browse > Tag List, on quitting: "Delete all tracks from Tag List when exiting
/// rekordbox." (`DatabaseMediator::~DatabaseMediator` 0x101e4a594 calls `cleanTaglist`).
pub fn on_exit(app: &mut App) {
    if tag_list_clears_on_exit(&app.prefs) && !app.ext.tag_list.is_empty() {
        app.tag_list_clear();
    }
}

/// Advanced > Others > Export Loop As WAV > Sample Rate: the loop's samples at that rate.
/// Linear resampling of the decoded stereo frames (recordbox's choice).
pub fn resample_stereo(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || to == 0 || samples.len() < 4 {
        return samples.to_vec();
    }
    let frames = samples.len() / 2;
    let out_frames = ((frames as f64) * to as f64 / from as f64).round() as usize;
    let step = from as f64 / to as f64;
    let mut v = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let x = i as f64 * step;
        let k = (x as usize).min(frames - 1);
        let k1 = (k + 1).min(frames - 1);
        let f = (x - k as f64) as f32;
        for c in 0..2 {
            v.push(samples[k * 2 + c] * (1.0 - f) + samples[k1 * 2 + c] * f);
        }
    }
    v
}

/// Advanced > Database > rekordbox xml > Export BeatGrid information (`bridgeExportBeatGrid`):
/// 0 "No" writes no TEMPO entries, 1 "BPM change points" the filtered beats (xmlexport.rs
/// `tempo_select`). Kept where the export, which has no `App`, reads it.
static XML_BEATGRID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

pub fn xml_beatgrid_mode() -> i64 {
    XML_BEATGRID.load(Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_cue_levels_and_first_sound() {
        assert_eq!(auto_cue_db(1), -36.0);
        assert_eq!(auto_cue_db(5), -60.0);
        assert_eq!(auto_cue_db(8), -78.0);
        assert_eq!(auto_cue_db(0), -60.0);
        // Silence, then a sound at -50 dB from frame 1000, then louder: with the -60 dB level
        // the cue is at 1000 taken down to a whole unit of 294 frames (44.1 kHz / 150).
        let mut s = vec![0.0f32; 4000 * 2];
        for f in 1000..4000 {
            s[f * 2 + 1] = if f < 2000 { 10f32.powf(-50.0 / 20.0) } else { 0.5 };
        }
        assert_eq!(auto_cue_frame(&s, 44100, -60.0), Some(1000 / 294 * 294));
        // At -36 dB the -50 dB part is too quiet: the louder part at 2000.
        assert_eq!(auto_cue_frame(&s, 44100, -36.0), Some(2000 / 294 * 294));
        assert_eq!(auto_cue_frame(&vec![0.0; 100], 44100, -78.0), None);
    }

    #[test]
    fn quantize_rows() {
        let mut rb = crate::prefs_rb::RbPrefs::default();
        // The default mode ("Jump before reaching the next beat.") gives HOT CUE and LOOP the
        // half-beat threshold of SNAP; off, each keeps its own.
        assert!(rb.quantize_jump_before);
        assert_eq!(quantize_kind_for(&rb, QType::HotCue), Some(QType::Snap));
        assert_eq!(quantize_kind_for(&rb, QType::Loop), Some(QType::Snap));
        rb.quantize_jump_before = false;
        assert_eq!(quantize_kind_for(&rb, QType::HotCue), Some(QType::HotCue));
        // A type switched off is not quantized at all; SNAP has no switch here.
        rb.q_reverse.on = false;
        rb.q_hot_cue.on = false;
        assert_eq!(quantize_kind_for(&rb, QType::Reverse), None);
        assert_eq!(quantize_kind_for(&rb, QType::HotCue), None);
        assert_eq!(quantize_kind_for(&rb, QType::Snap), Some(QType::Snap));
        // With the half-beat threshold a hot cue pressed 0.1 beat after a beat jumps at once
        // (the point it belongs to is behind it); with 1/32 it waits for the next point.
        let beats: Vec<Beat> = (0..16).map(|i| Beat { time: i as f64 * 0.5, bpm: 120.0, beat: (i % 4 + 1) as u8, index: i }).collect();
        let t = 1.05;
        assert!((crate::deck::quantize_point(&beats, t, QType::Snap, 1.0, false) - 1.0).abs() < 1e-9);
        assert!((crate::deck::quantize_point(&beats, t, QType::HotCue, 1.0, false) - 1.5).abs() < 1e-9);
        // The Preferences rows reach the deck's beat values.
        let mut p = Prefs::default();
        p.rb.q_hot_cue.beat = 2;
        p.rb.q_reverse.beat = 5;
        let d = &mut p.deck;
        d.quantize_units = [p.rb.snap.beat.clamp(2, 6), p.rb.q_hot_cue.beat.clamp(2, 6), p.rb.q_loop.beat.clamp(2, 6), p.rb.q_reverse.beat.clamp(2, 6)];
        assert_eq!(p.deck.quantize_units, [6, 2, 6, 5]);
    }

    #[test]
    fn load_start_and_wave_order() {
        let c = |kind, start| Cue { id: 0, track_id: 1, kind, slot: 0, start, end: None, color: 0, comment: String::new(), active_loop: false };
        assert_eq!(load_start_point(&[c(CueKind::Memory, 30.0), c(CueKind::Hot, 12.0), c(CueKind::Hot, 50.0)]), Some(12.0));
        assert_eq!(load_start_point(&[c(CueKind::Memory, 3.0), c(CueKind::Hot, 12.0)]), Some(3.0));
        assert_eq!(load_start_point(&[]), None);
        let mut rb = crate::prefs_rb::RbPrefs::default();
        assert_eq!(wave_order_for(&rb, false, 4), [0, 1, 2, 3]);
        assert_eq!(wave_order_for(&rb, true, 4), [2, 0, 1, 3], "Auto: rekordbox's vertical order");
        rb.zoom_wave_custom = true;
        rb.zoom_wave_h = 2;
        rb.zoom_wave_v = 1;
        assert_eq!(wave_order_for(&rb, false, 4), [2, 0, 1, 3]);
        assert_eq!(wave_order_for(&rb, true, 4), [0, 1, 2, 3]);
        assert_eq!(wave_order_for(&rb, true, 2), [0, 1, 2, 3]);
    }

    #[test]
    fn played_time_counts_in_one_go() {
        let mut c = crate::library_ext::PlayCredit::default();
        c.need = 10.0;
        // 6 s, a pause, 6 s: not yet (the count starts again); 10 s in one go: counted once.
        for _ in 0..12 {
            assert_eq!(c.tick(Some(7), true, 0.5, 300.0), None);
        }
        c.tick(Some(7), false, 0.5, 300.0);
        for _ in 0..12 {
            assert_eq!(c.tick(Some(7), true, 0.5, 300.0), None);
        }
        let mut hit = 0;
        for _ in 0..10 {
            if c.tick(Some(7), true, 0.5, 300.0).is_some() {
                hit += 1;
            }
        }
        assert_eq!(hit, 1);
        // Without the setting (need 0) recordbox's own minute, paused time kept.
        let mut d = crate::library_ext::PlayCredit::default();
        for _ in 0..100 {
            d.tick(Some(8), true, 0.5, 300.0);
        }
        d.tick(Some(8), false, 0.5, 300.0);
        assert!(d.secs >= 50.0);
    }

    #[test]
    fn xml_beatgrid_modes() {
        let recs: Vec<(u16, u16, u32)> = (0..8).map(|i| ((i % 4 + 1) as u16, 12800, (i as f64 * 468.75) as u32)).collect();
        assert!(crate::xmlexport::tempo_select(&recs, 0).is_empty(), "No");
        assert_eq!(crate::xmlexport::tempo_select(&recs, 1), vec![0], "BPM change points: one for a steady grid");
    }

    #[test]
    fn tag_list_rows_are_two_switches_both_off() {
        let mut p = Prefs::default();
        // rekordbox's defaults: neither row on, so quitting keeps the Tag List.
        assert!(!p.rb.tag_list_on_load && !p.rb.tag_list_on_exit);
        assert!(!tag_list_clears_on_exit(&p));
        assert!(!tag_list_removes_on_load(&p, true, true));
        // Each on its own.
        p.rb.tag_list_on_load = true;
        assert!(tag_list_removes_on_load(&p, true, true));
        assert!(!tag_list_removes_on_load(&p, false, true), "only a track loaded from the Tag List");
        assert!(!tag_list_removes_on_load(&p, true, false));
        assert!(!tag_list_clears_on_exit(&p));
        p.rb.tag_list_on_load = false;
        p.rb.tag_list_on_exit = true;
        assert!(tag_list_clears_on_exit(&p) && !tag_list_removes_on_load(&p, true, true));
        // A settings file written with the old single choice loads with both off.
        let old: Prefs = serde_json::from_str(r#"{"rb":{"tag_list_delete":1}}"#).unwrap();
        assert!(!old.rb.tag_list_on_load && !old.rb.tag_list_on_exit);
    }

    #[test]
    fn auto_beat_loop_lengths() {
        let mut p = Prefs::default();
        p.loop_beats = 16.0;
        assert_eq!(auto_beat_loop_beats(&p), 16.0, "Loop for specified beats");
        for (id, b) in [(2u8, 1.0), (3, 2.0), (4, 4.0), (5, 8.0), (6, 16.0), (7, 32.0)] {
            p.rb.auto_beat_loop = id;
            assert_eq!(auto_beat_loop_beats(&p), b);
        }
    }

    #[test]
    fn drawing_rates_and_resampling() {
        assert_eq!((draw_fps(0), draw_fps(1), draw_fps(2)), (0, 40, 40));
        let s: Vec<f32> = (0..4800).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let r = resample_stereo(&s, 48000, 44100);
        assert_eq!(r.len() / 2, 4410);
        assert!((r[2 * 100] - 100.0 * 48000.0 / 44100.0).abs() < 1e-3);
        assert_eq!(resample_stereo(&s, 44100, 44100), s);
    }
}
