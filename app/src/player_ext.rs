//! Player logic beyond the basics in app.rs: the second deck (2 Players and Dual Player
//! layouts) with tempo, Master Tempo, pitch bend, BEAT SYNC / MASTER, KEY SYNC and key shift,
//! the crossfader, A-GAIN and the isolator switches; PHRASE EDIT; loop adjust, cues and loops
//! set from the waveform menu, "Export Loop As Wav", the Beat Count Display, search (scan),
//! previous / next track and layouts.

use crate::app::*;
use crate::model::*;
use eframe::egui;

/// Per-deck state added by this module (a field of `Deck`).
#[derive(Default)]
pub struct DeckExt {
    /// Loop adjust: 0 = off, 1 = the loop-in point follows ←/→ and drags on the enlarged
    /// waveform, 2 = the loop-out point.
    pub loop_adjust: u8,
    /// Position the enlarged waveform's right-click menu acts at (seconds, not yet quantized).
    pub menu_time: Option<f64>,
    /// A drag adjusting a loop edge: pointer x and the edge's time when it began.
    pub adjust_drag: Option<(f32, f64)>,
    /// The GRID panel's BPM field is being typed in (otherwise it shows the current BPM).
    pub bpm_editing: bool,
    /// The right mouse button is holding CUE on the waveform ("Click on the waveform for PLAY
    /// and CUE").
    pub wave_cue_held: bool,
    /// Tempo set with TEMPO − / +, a typed BPM or RST, as the change from the track's own
    /// speed (0.03 = +3 %). A BEAT SYNC follower plays at the master's tempo instead.
    pub tempo_adj: f64,
    /// MT (Master Tempo): the tempo changes without the pitch.
    pub master_tempo: bool,
    /// Key shift in semitones (-12..12), set with ‹ › or by KEY SYNC.
    pub key_shift: i32,
    pub key_sync: bool,
    pub beat_sync: bool,
    /// A pitch bend button is held: -1 slower, +1 faster.
    pub bend: i32,
    /// HIGH / MID / LOW switched off (index 0 = low, 1 = mid, 2 = high).
    pub eq_kill: [bool; 3],
    /// The tempo field is being typed in, and its text.
    pub tempo_editing: bool,
    pub tempo_text: String,
    /// The deck's phrases carry PHRASE EDIT changes (kept per track; CLEAR removes them).
    pub phrases_edited: bool,
    /// The beats last handed to the audio thread for BEAT SYNC (count, first and last time).
    pub grid_sig: (usize, u64, u64),
    /// Prepared static overview columns for this loaded track (not carried to another track).
    pub overview_cache: crate::ui_player::OverviewCache,
    /// A-GAIN knob drag: pointer y and the gain (dB) when it began.
    pub gain_drag: Option<(f32, f32)>,
    /// Seconds the enlarged waveform was scrubbed or scrolled this frame (for the waveform link).
    pub scrub: f64,
    /// STEM Waveform Display: the waveform of the parts that sound (which parts, waveform),
    /// and the one being worked out in the background.
    pub stem_wave: Option<([bool; 4], Waveform)>,
    pub stem_wave_rx: Option<(i64, [bool; 4], crossbeam_channel::Receiver<Waveform>)>,
    /// SLIP is on for this deck (Performance).
    pub slip: bool,
    /// Pad pages: HOT CUE shows I-P instead of A-H; BEAT JUMP's and BEAT LOOP's second page.
    pub pad_page: u8,
    /// Performance: how many steps the platter's tempo range is past its default of ±10
    /// (0 = ±10, 1 = ±16, 2 = WIDE, 3 = ±6).
    pub tempo_range: u8,
    /// The deck engine's state for the interface (`deck.rs`): reverse, jog, sync rate.
    pub dk: crate::deck::DeckUi,
}

impl DeckExt {
    /// What stays on a deck when another track is loaded or it is emptied: the switches of
    /// the deck itself (MT, BEAT SYNC, KEY SYNC, isolator), not the settings made for a track.
    /// The deck's switches and tempo as plain values (for handing to the audio thread).
    pub fn carried_all(&self) -> DeckSwitches {
        DeckSwitches { bend: self.bend, master_tempo: self.master_tempo, key_shift: self.key_shift, slip: self.slip, eq_kill: self.eq_kill }
    }

    pub fn carried(&self) -> DeckExt {
        DeckExt { master_tempo: self.master_tempo, beat_sync: self.beat_sync, key_sync: self.key_sync, eq_kill: self.eq_kill, tempo_range: self.tempo_range, slip: self.slip, pad_page: self.pad_page, dk: crate::deck::DeckUi { again: self.dk.again, ..Default::default() }, ..Default::default() }
    }
}

#[derive(Clone, Copy)]
pub struct DeckSwitches {
    pub bend: i32,
    pub master_tempo: bool,
    pub key_shift: i32,
    pub slip: bool,
    pub eq_kill: [bool; 3],
}

/// Player state shared by the layouts (a field of `App`).
pub struct PlayerExt {
    /// The decks that are not active, by deck number (0 = deck 1): `parked[n]` holds deck n
    /// while another deck is `App::deck`; the active deck's own slot holds an empty deck.
    pub parked: [Deck; 4],
    /// `App::deck_backup` of the parked decks.
    pub parked_backup: [Option<(Deck, f64, f32)>; 4],
    /// Which deck `App::deck` is (0 = deck 1 .. 3 = deck 4).
    pub active: u8,
    /// Master mute.
    pub muted: bool,
    /// The deck BEAT SYNC and KEY SYNC follow (MASTER), by deck number.
    pub master: u8,
    /// Crossfader position: 0.0 = deck A only, 0.5 = both, 1.0 = deck B only.
    pub crossfader: f32,
    /// Dual Player's dual control: beat jumps move both decks together.
    pub dual_control: bool,
    /// The waveform link: scrubbing or scrolling one enlarged waveform moves both decks.
    pub wave_link: bool,
    /// The Performance mixer, per deck (index 0 = deck 1). Not kept between runs, as in
    /// rekordbox.
    pub mix: [MixStrip; 4],
    /// Headphones MIX and LEVEL knobs (0..1).
    pub hp_mix: f32,
    pub hp_level: f32,
    /// Master level meter as shown (dB, L and R).
    pub master_db: [f32; 2],
    /// Pad mode of each deck: 0 = HOT CUE, 1 = STEMS.
    pub pad_mode: [u8; 4],
    /// Loop box of each deck shows the manual loop buttons (MA) instead of the auto loop (AU).
    pub loop_manual: [bool; 4],
    /// STEM FX switches (VOCAL, INST, BASS, DRUMS), all on at start, as in rekordbox.
    pub stem_fx: [bool; 4],
    /// The two FX units of the Performance FX panel.
    pub fx: [crate::ui_fx::FxUi; 2],
    /// The CFX row: the effect, DEFAULT (FILTER) or USER (the chosen one), PARAM, and the
    /// COLOR knobs of decks 1 to 4 and the master (0.5 = off).
    pub cfx_kind: u8,
    pub cfx_user: bool,
    pub cfx_param: f32,
    pub cfx_color: [f32; 5],
    /// The rest of the FX panel's state: CFX multi mode, PARAM per effect, what was last saved.
    pub fxx: crate::ui_fx::FxExtra,
    /// Quantize of the parked decks (`Prefs::quantize` is the active deck's): each deck has
    /// its own Q, as in rekordbox.
    pub parked_quantize: [bool; 4],
    /// Each deck's auto loop size while it is not the active deck.
    pub parked_loop: [Option<f64>; 4],
    /// The sampler panel's state.
    pub sampler: crate::ui_sampler::SamplerUi,
}

/// One channel strip of the Performance mixer (knob and fader positions, 0..1).
#[derive(Clone, Copy)]
pub struct MixStrip {
    pub trim: f32,
    /// LOW, MID, HIGH.
    pub eq: [f32; 3],
    pub fader: f32,
    /// Headphones CUE.
    pub cue: bool,
    /// STEM ISO mode: the three knobs set the level of VOCAL, INST (with BASS) and DRUMS.
    pub stem_iso_on: bool,
    /// DRUMS, INST, VOCAL (bottom to top, like LOW / MID / HIGH).
    pub stem_iso: [f32; 3],
    /// Crossfader side: 0 = THRU, 1 = A, 2 = B.
    pub assign: u8,
    /// Level meter as shown (dB, L and R).
    pub shown_db: [f32; 2],
}

impl MixStrip {
    fn new(assign: u8) -> MixStrip {
        MixStrip { trim: 0.5, eq: [0.5; 3], fader: 1.0, cue: false, stem_iso_on: false, stem_iso: [0.5; 3], assign, shown_db: [-120.0; 2] }
    }
}

impl Default for PlayerExt {
    fn default() -> Self {
        PlayerExt { parked: std::array::from_fn(|_| Deck::default()), parked_backup: [None, None, None, None], active: 0, muted: false, master: 0, crossfader: 0.5, dual_control: false, wave_link: false, mix: [MixStrip::new(1), MixStrip::new(2), MixStrip::new(1), MixStrip::new(2)], hp_mix: 0.5, hp_level: 0.5, master_db: [-120.0; 2], pad_mode: [0; 4], loop_manual: [false; 4], stem_fx: [true; 4], fx: [crate::ui_fx::FxUi::new(), crate::ui_fx::FxUi::new()], cfx_kind: 0, cfx_user: false, cfx_param: 0.5, cfx_color: [0.5; 5], fxx: Default::default(), parked_quantize: [true; 4], parked_loop: [None; 4], sampler: crate::ui_sampler::SamplerUi::new() }
    }
}

/// Tempo range of the TEMPO controls and typed BPM values (±16 %) outside Performance mode.
pub const TEMPO_RANGE: f64 = 0.16;
/// Performance: the ranges the platter's range label steps through (±6, ±10, ±16, WIDE), in
/// percent. rekordbox keeps the slider position when the range changes.
pub const TEMPO_RANGES: [f64; 4] = [6.0, 10.0, 16.0, 100.0];
/// One click of TEMPO − / + (0.1 % of the track's tempo).
pub const TEMPO_STEP: f64 = 0.001;
/// Speed change while a pitch bend button is held (4 %).
pub const BEND_AMOUNT: f64 = 0.04;
/// Range of the A-GAIN knob in dB (its travel is ±135° around 0 dB).
pub const GAIN_RANGE_DB: f32 = 20.0;

/// The tempo of a grid at `t` seconds (the first beat's before it); 0.0 without beats.
pub fn bpm_at(beats: &[Beat], t: f64) -> f64 {
    let i = beats.partition_point(|b| b.time <= t);
    if i > 0 {
        beats[i - 1].bpm
    } else {
        beats.first().map(|b| b.bpm).unwrap_or(0.0)
    }
}

/// A key (model index: 0..11 = 1A..12A, 12..23 = 1B..12B) moved by `semis` semitones. One
/// semitone up is seven steps round the wheel.
pub fn shift_key(k: i32, semis: i32) -> i32 {
    if !(0..24).contains(&k) {
        return k;
    }
    (k % 12 + 7 * semis).rem_euclid(12) + (k / 12) * 12
}

/// Two keys mix harmonically: the same key, its relative major / minor, or a neighbour on
/// the wheel (a fifth up or down).
pub fn keys_compatible(a: i32, b: i32) -> bool {
    if !(0..24).contains(&a) || !(0..24).contains(&b) {
        return false;
    }
    let (na, nb) = (a % 12, b % 12);
    if a / 12 == b / 12 {
        let d = (na - nb).rem_euclid(12);
        d == 0 || d == 1 || d == 11
    } else {
        na == nb
    }
}

/// KEY SYNC: the smallest key shift (-6..6 semitones) that makes `own` compatible with
/// `master`; of two equally small shifts the one that lands on the master's own key, then
/// the downward one. 0 when either key is unknown.
pub fn key_sync_shift(own: i32, master: i32) -> i32 {
    if !(0..24).contains(&own) || !(0..24).contains(&master) {
        return 0;
    }
    let mut best: Option<(i32, bool, i32)> = None;
    for d in -6..=6 {
        let k = shift_key(own, d);
        if keys_compatible(k, master) {
            let cand = (d.abs(), k != master, d);
            if best.map(|b| cand < b).unwrap_or(true) {
                best = Some(cand);
            }
        }
    }
    best.map(|b| b.2).unwrap_or(0)
}

/// "±0", "+2", "-1": a key shift as the Dual Player shows it.
pub fn shift_text(semis: i32) -> String {
    match semis {
        0 => "±0".into(),
        n if n > 0 => format!("+{n}"),
        n => format!("{n}"),
    }
}

/// A-GAIN knob angle in degrees from straight up for a gain in dB.
pub fn gain_angle(db: f32) -> f32 {
    (db / GAIN_RANGE_DB).clamp(-1.0, 1.0) * 135.0
}

/// Auto beat loop lengths rekordbox offers, in beats ("1/64 Beat Loop" … "512 Beat Loop").
pub const LOOP_SIZES: [f64; 16] = [1.0 / 64.0, 1.0 / 32.0, 1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0];
/// Beat jump lengths in beats, and in bars (shown as bars; a bar is four beats).
pub const JUMP_BEATS: [f64; 9] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
pub const JUMP_BARS: [f64; 6] = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// Search (scan) speed while a SEARCH button is held.
pub const SEARCH_SPEED: f64 = 8.0;

/// "1/4", "4", "512": a beat count as the loop and jump controls show it.
pub fn beats_text(b: f64) -> String {
    if b < 1.0 {
        format!("1/{}", (1.0 / b).round())
    } else {
        format!("{b}")
    }
}

/// The beat jump button's caption: "4Beats", "1/2Beat", "2Bars".
pub fn jump_label(beats: f64, bars: bool) -> String {
    if bars && beats >= 4.0 {
        let n = beats / 4.0;
        format!("{}{}", n, if n == 1.0 { "Bar" } else { "Bars" })
    } else if beats <= 1.0 {
        format!("{}Beat", beats_text(beats))
    } else {
        format!("{}Beats", beats)
    }
}

/// Current position as bars.beats ("1.1Bars" on the first downbeat).
pub fn bars_label(beats: &[Beat], t: f64) -> String {
    if beats.is_empty() {
        return String::new();
    }
    let i = beats.partition_point(|b| b.time <= t + 1e-4);
    if i == 0 {
        return "1.1Bars".into();
    }
    let b = beats[i - 1];
    let first_down = beats.iter().position(|x| x.beat == 1).unwrap_or(0);
    let bars = beats[..i].iter().skip(first_down).filter(|x| x.beat == 1).count().max(1);
    format!("{}.{}Bars", bars, b.beat)
}

/// Beats still to come before the next memory cue after `t`: the beat being played counts,
/// so it reads 16 for the whole of the sixteenth beat before the cue and 1 during the last.
/// None when there is no memory cue ahead (or no grid).
pub fn beats_to_next_memory(beats: &[Beat], memory: &[f64], t: f64) -> Option<usize> {
    if beats.is_empty() {
        return None;
    }
    let m = memory.iter().copied().filter(|m| *m > t + 1e-4).fold(f64::NAN, f64::min);
    if m.is_nan() {
        return None;
    }
    let from = beats.partition_point(|b| b.time <= t + 1e-4);
    let to = beats.partition_point(|b| b.time < m - 1e-4);
    Some(to.saturating_sub(from) + 1)
}

/// The readout above the enlarged waveform for a Beat Count Display mode: 0 = Current Position
/// (Bars), 1 = Count to the next MEMORY CUE (Bars), 2 = Count to the next MEMORY CUE (Beats).
/// The bar countdown runs like a player's: "3.4Bars" sixteen beats before the cue, "0.1Bars"
/// on the last beat.
pub fn beat_count_label(beats: &[Beat], memory: &[f64], t: f64, mode: u8) -> String {
    match mode {
        1 => match beats_to_next_memory(beats, memory, t) {
            Some(n) => format!("{}.{}Bars", (n - 1) / 4, (n - 1) % 4 + 1),
            None => "-.-Bars".into(),
        },
        2 => match beats_to_next_memory(beats, memory, t) {
            Some(n) => format!("{n}Beats"),
            None => "-Beats".into(),
        },
        _ => bars_label(beats, t),
    }
}

/// Shortest loop loop adjust leaves (seconds).
pub const MIN_LOOP: f64 = 0.005;

/// A loop with one edge moved by `delta` seconds (edge 1 = in, 2 = out). The edges never
/// cross and stay inside the track.
pub fn adjust_loop(l: (f64, f64), edge: u8, delta: f64, duration: f64) -> (f64, f64) {
    let (a, b) = l;
    match edge {
        1 => ((a + delta).clamp(0.0, (b - MIN_LOOP).max(0.0)), b),
        2 => (a, (b + delta).clamp(a + MIN_LOOP, duration.max(a + MIN_LOOP))),
        _ => l,
    }
}

/// The interleaved stereo samples between two times.
pub fn loop_samples(samples: &[f32], sr: u32, a: f64, b: f64) -> &[f32] {
    let frames = samples.len() / 2;
    let fa = ((a.max(0.0) * sr as f64).round() as usize).min(frames);
    let fb = ((b.max(0.0) * sr as f64).round() as usize).clamp(fa, frames);
    &samples[fa * 2..fb * 2]
}

/// A 16-bit PCM WAV file's bytes for interleaved samples.
pub fn wav16_bytes(sr: u32, channels: u16, samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn write_wav16(path: &std::path::Path, sr: u32, channels: u16, samples: &[f32]) -> std::io::Result<()> {
    std::fs::write(path, wav16_bytes(sr, channels, samples))
}

/// Row to load for Previous Track (-1) / Next Track (+1): the neighbour of the loaded track
/// in the list that passes `ok`, or the first / last usable row when the loaded track is not
/// in the list.
pub fn adjacent_row(list: &[i64], current: Option<i64>, dir: i32, ok: impl Fn(i64) -> bool) -> Option<usize> {
    let n = list.len() as i64;
    let mut i = match current.and_then(|c| list.iter().position(|x| *x == c)) {
        Some(i) => i as i64 + dir as i64,
        None if dir > 0 => 0,
        None => n - 1,
    };
    while i >= 0 && i < n {
        if ok(list[i as usize]) {
            return Some(i as usize);
        }
        i += dir.signum() as i64;
    }
    None
}

impl App {
    // ------------------------------------------------------------------ decks and layouts
    /// Detaching a full browser reveals the last chosen player layout in the main window.
    pub fn full_browser(&self) -> bool {
        self.prefs.browser_only && self.library_window.is_none()
    }

    /// A layout with both decks is showing (2 Players or Dual Player).
    pub fn two_players(&self) -> bool {
        self.performance() || (!self.full_browser() && matches!(self.prefs.player_layout, 1 | 3))
    }

    /// The Dual Player layout is showing.
    pub fn dual_player(&self) -> bool {
        !self.performance() && !self.full_browser() && self.prefs.player_layout == 3
    }

    /// PERFORMANCE mode (decks with a mixer) is showing.
    pub fn performance(&self) -> bool {
        self.prefs.app_mode == 1
    }

    /// Key shift is on screen (Dual Player and Performance).
    pub fn key_controls(&self) -> bool {
        self.dual_player() || self.performance()
    }

    /// EXPORT (0) or PERFORMANCE (1).
    pub fn set_app_mode(&mut self, mode: u8) {
        if crate::ui_edit::intercept_mode(self, mode) { return; }
        self.prefs.app_mode = mode.min(1);
        self.poll_decks();
        self.save_prefs();
    }

    /// The active deck's tempo range as a fraction (0.16 = ±16 %).
    pub fn tempo_range(&self) -> f64 {
        if self.performance() {
            TEMPO_RANGES[(self.deck.x.tempo_range as usize + 1) % 4] / 100.0
        } else {
            TEMPO_RANGE
        }
    }

    /// Performance: the next tempo range (6, 10, 16, WIDE, 6 ...). The tempo is rescaled so
    /// that the slider position stays where it is, as in rekordbox (only with a track loaded).
    pub fn tempo_range_next(&mut self) {
        let old = self.tempo_range();
        self.deck.x.tempo_range = (self.deck.x.tempo_range + 1) % 4;
        let new = self.tempo_range();
        if self.deck.track.is_some() && !self.sync_follower() {
            self.deck.x.tempo_adj = self.deck.x.tempo_adj * new / old;
        }
        self.push_deck_controls();
    }

    /// Height of the Performance player area: the stacked waveforms (143 pt), the decks
    /// (225 pt, 21 pt more with the stem part buttons) and the mixer's bottom row (43 pt).
    /// rekordbox 7.2.19: 368 + 43 (+ 21).
    pub fn performance_height(&self) -> f32 {
        // (Every Performance layout's height: ui_perf_layouts.rs.) The GROOVE CIRCUIT rows
        // come on top (ui_groove.rs).
        crate::ui_perf_layouts::height(self) + crate::ui_perf_layouts::groove_row(self) * if self.deck_count() == 4 { 2.0 } else { 1.0 } + crate::ui_perf_layouts::mpl_extra(self)
    }

    /// How many decks exist in the layout showing: 4 (Performance, 4Deck), 2 or 1.
    pub fn deck_count(&self) -> u8 {
        if self.performance() && self.prefs.perf_four {
            4
        } else if self.two_players() {
            2
        } else {
            1
        }
    }

    /// The deck paired with the active one in the two-deck layouts (1 with 2, 3 with 4).
    pub fn partner(&self) -> u8 {
        self.px.active ^ 1
    }

    /// Deck `n`, whether it is the active one or parked.
    pub fn deck_of(&self, n: u8) -> &Deck {
        if n == self.px.active { &self.deck } else { &self.px.parked[n.min(3) as usize] }
    }

    pub fn deck_of_mut(&mut self, n: u8) -> &mut Deck {
        if n == self.px.active { &mut self.deck } else { &mut self.px.parked[n.min(3) as usize] }
    }

    /// The partner deck (see `partner`).
    pub fn other_deck(&self) -> &Deck {
        &self.px.parked[self.partner() as usize]
    }

    pub fn other_deck_mut(&mut self) -> &mut Deck {
        let n = self.partner() as usize;
        &mut self.px.parked[n]
    }

    /// Put deck `n` in place as `self.deck` / `self.player.state` / `prefs.quantize`.
    fn set_active(&mut self, n: u8) {
        let n = n.min(3);
        let a = self.px.active;
        if n == a {
            return;
        }
        let (a, n) = (a as usize, n as usize);
        std::mem::swap(&mut self.deck, &mut self.px.parked[a]);
        std::mem::swap(&mut self.deck, &mut self.px.parked[n]);
        std::mem::swap(&mut self.deck_backup, &mut self.px.parked_backup[a]);
        std::mem::swap(&mut self.deck_backup, &mut self.px.parked_backup[n]);
        self.px.parked_quantize[a] = self.prefs.quantize;
        // The loop size belongs to the deck too (a deck not used yet starts from the one showing).
        self.px.parked_loop[a] = Some(self.prefs.loop_beats);
        if let Some(b) = self.px.parked_loop[n] {
            self.prefs.loop_beats = b;
        }
        self.prefs.quantize = self.px.parked_quantize[n];
        self.player.state = self.player.decks[n].clone();
        self.px.active = n as u8;
    }

    /// Instant Doubles (Preferences > Controller > Deck > Load): the track just loaded is also
    /// on the other deck, so this deck takes over its position, tempo and play state.
    pub fn instant_double(&mut self) {
        if !(self.performance() && (self.prefs.instant_doubles || self.deck.x.dk.double_pending)) || self.deck.track.is_none() {
            return;
        }
        self.deck.x.dk.double_pending = false;
        // The first other deck (1, 2, 3, 4 in that order) that has the same track.
        let Some(src) = (0..self.deck_count()).find(|n| *n != self.px.active && self.deck_of(*n).track == self.deck.track) else { return };
        let (pos, playing, rate) = {
            let o = self.player.decks[src as usize].lock();
            (o.pos, o.playing, o.audio.as_ref().map(|a| a.sr))
        };
        if rate.is_none() {
            return;
        }
        let from = self.deck_of(src).x.carried();
        self.deck.x.tempo_adj = self.deck_of(src).x.tempo_adj;
        self.deck.x.tempo_range = from.tempo_range;
        self.deck.x.master_tempo = from.master_tempo;
        {
            let mut s = self.player.state.lock();
            if s.audio.as_ref().map(|a| a.sr) == rate {
                s.pos = pos;
                s.playing = playing && !self.deck_options.disable_play;
            }
        }
        self.push_deck_controls();
    }

    /// SLIP on / off for the active deck.
    pub fn toggle_slip(&mut self) {
        if self.slip_refused_by_slicer() {
            return;
        }
        self.deck.x.slip = !self.deck.x.slip;
        self.push_deck_controls();
    }

    /// The active deck's SLIP is holding a background position (the button blinks).
    pub fn slip_running(&self) -> bool {
        self.player.state.lock().slip_pos.is_some()
    }

    /// Run `f` with deck `n` (0 = A, 1 = B) in place as `self.deck` / `self.player.state`.
    pub fn with_deck<R>(&mut self, n: u8, f: impl FnOnce(&mut App) -> R) -> R {
        let prev = self.px.active;
        if n.min(3) == prev {
            return f(self);
        }
        self.set_active(n);
        let r = f(self);
        self.set_active(prev);
        r
    }

    /// Make deck `n` the one keyboard shortcuts and the browser act on.
    pub fn activate_deck(&mut self, n: u8) {
        self.set_active(n);
    }

    /// Load a track into deck 0 (A) or 1 (B); B exists only in the 2 Players layout.
    pub fn load_track_to(&mut self, deck: u8, id: i64) {
        // Performance: the deck that was last clicked (its number is lit) takes the track.
        let d = if self.performance() { self.px.active } else if self.two_players() { deck.min(self.deck_count() - 1) } else { self.px.active };
        self.with_deck(d, |a| a.load_track_here(id));
    }

    /// Layout in the order of the View menu and the header's list: 0 = 1 Player,
    /// 1 = 2 Players, 2 = Dual Player, 3 = Simple Player, 4 = Full Browser.
    /// (`Prefs::player_layout` keeps its stored numbering: 0 = 1 Player, 1 = 2 Players,
    /// 2 = Simple Player, 3 = Dual Player.)
    pub fn set_player_layout(&mut self, layout: u8) {
        if layout >= 4 {
            self.prefs.browser_only = true;
        } else {
            self.prefs.browser_only = false;
            self.prefs.player_layout = [0, 1, 3, 2][layout as usize];
        }
        self.poll_decks();
        self.save_prefs();
    }

    /// 0 = 1 Player, 1 = 2 Players, 2 = Dual Player, 3 = Simple Player, 4 = Full Browser.
    pub fn player_layout(&self) -> u8 {
        if self.full_browser() {
            4
        } else {
            [0, 1, 3, 2][self.prefs.player_layout.min(3) as usize]
        }
    }

    /// Height of the player area for the layout showing (measured on rekordbox 7.2.19:
    /// 1 Player 256 pt, 2 Players and Dual Player 375 pt, Simple Player 59 pt).
    pub fn player_height(&self) -> f32 {
        if self.performance() {
            return self.performance_height();
        }
        match self.prefs.player_layout {
            1 | 3 => 375.0,
            2 => crate::ui_player2::SIMPLE_HEIGHT,
            _ => 256.0,
        }
    }

    /// Once per frame: background results for the deck that is not active, one master volume
    /// and mute for both decks, and deck B put away outside the 2 Players layout.
    pub fn poll_decks(&mut self) {
        // Decks that do not exist in the layout showing are put away (stopped, not unloaded).
        let count = self.deck_count();
        if self.px.active >= count {
            self.activate_deck(0);
        }
        if self.load_target >= count {
            self.load_target = 0;
        }
        if self.px.master >= count {
            self.px.master = 0;
        }
        for n in count..4 {
            let mut o = self.player.decks[n as usize].lock();
            o.playing = false;
            o.search = 0.0;
        }
        for n in 0..count {
            if n == self.px.active {
                continue;
            }
            let o = &self.px.parked[n as usize];
            if o.loading.is_some() || o.phrases_rx.is_some() || o.vocal_rx.is_some() {
                self.with_deck(n, |a| {
                    a.poll_load();
                    a.poll_phrases();
                    a.poll_vocal();
                });
            }
        }
        let v = self.player.state.lock().volume;
        for st in self.player.decks.iter() {
            let mut o = st.lock();
            o.volume = v;
            o.muted = self.px.muted;
        }
        self.master_handover();
        self.push_deck_controls();
        crate::ui_fx::sync(self);
    }

    /// Hand each deck's tempo, Master Tempo, key shift, crossfader gain, isolator switches,
    /// BEAT SYNC state and beat grid to the audio thread. Outside the two-deck layouts the
    /// deck plays plainly (those controls are not on screen there).
    pub(crate) fn push_deck_controls(&mut self) {
        let two = self.two_players();
        let dual = self.dual_player();
        let perf = self.performance();
        let keys = self.key_controls();
        let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
        let (hp_mix, hp_level) = (self.px.hp_mix, crate::mixer::headphones_level(self.px.hp_level));
        let hp_master = if perf && hp_on { crate::mixer::cue_master_balance(hp_mix) * hp_level } else { 0.0 };
        crate::audio::HP_MASTER_GAIN.store(hp_master.to_bits(), std::sync::atomic::Ordering::Relaxed);
        let (isolator, curve, three) = (self.prefs.eq_isolator, self.prefs.ch_fader_curve, self.prefs.stem_parts == 3);
        crate::audio::LIMITER_ON.store(perf, std::sync::atomic::Ordering::Relaxed);
        let count = self.deck_count();
        let master = self.px.master.min(count.saturating_sub(1));
        // The sampler deck as the sync master: every deck is a follower of its tempo.
        let smp_master = self.sampler_is_master();
        crate::audio::MASTER_DECK.store(if smp_master { 255 } else { master }, std::sync::atomic::Ordering::Relaxed);
        let faders = crate::audio::crossfader_gains(self.px.crossfader);
        // Each deck's position, own BPM there and key.
        let mut infos = [(0.0f64, 0.0f64, -1i32); 4];
        for n in 0..count {
            let d = self.deck_of(n);
            let pos = {
                let s = self.player.decks[n as usize].lock();
                s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0)
            };
            let t = d.track.and_then(|id| self.track(id));
            let bpm = if d.beats.is_empty() { t.map(|t| t.bpm).unwrap_or(0.0) } else { bpm_at(&d.beats, pos) };
            infos[n as usize] = (pos, bpm, t.map(|t| t.key).unwrap_or(-1));
        }
        let i_m = infos[master as usize];
        let (m_adj, m_shift) = {
            let mx = &self.deck_of(master).x;
            (mx.tempo_adj, mx.key_shift)
        };
        let master_bpm = if smp_master { self.prefs.sampler.bpm.clamp(40.0, 499.0) as f64 } else { i_m.1 * (1.0 + m_adj) };
        let master_key = shift_key(i_m.2, m_shift);
        for n in 0..count {
            let (own_bpm, own_key) = (infos[n as usize].1, infos[n as usize].2);
            let d = self.deck_of_mut(n);
            let follower = two && (n != master || smp_master);
            if follower && !smp_master && d.x.key_sync && d.track.is_some() {
                d.x.key_shift = key_sync_shift(own_key, master_key);
            }
            let synced = follower && d.x.beat_sync && own_bpm > 0.0 && master_bpm > 0.0;
            let tempo = if !two {
                1.0
            } else if synced {
                crate::audio::sync_rate(master_bpm, own_bpm)
            } else {
                1.0 + d.x.tempo_adj
            };
            let sig = (d.beats.len(), d.beats.first().map(|b| b.time.to_bits()).unwrap_or(0), d.beats.last().map(|b| b.time.to_bits()).unwrap_or(0));
            let grid = if sig != d.x.grid_sig {
                d.x.grid_sig = sig;
                Some(if d.beats.len() >= 2 { Some(std::sync::Arc::new(d.beats.iter().map(|b| b.time).collect::<Vec<f64>>())) } else { None })
            } else {
                None
            };
            let x = d.x.carried_all();
            let st = self.player.decks[n as usize].clone();

            let mut s = st.lock();
            s.tempo = tempo;
            s.bend = if two { x.bend as f64 * BEND_AMOUNT } else { 0.0 };
            s.master_tempo = two && x.master_tempo;
            s.key_shift = if keys { x.key_shift as f32 } else { 0.0 };
            s.smooth = true;
            // Sound Color FX (only with the FX panel shown; DEFAULT is always FILTER).
            let (ck, cc, cp) = crate::ui_fx::cfx_for(self, n as usize);
            s.cfx_kind = ck;
            s.cfx_color = if perf && self.prefs.perf_fx { cc } else { 0.5 };
            s.cfx_param = cp;
            // Headphones CUE: the deck's share, by the MIX and LEVEL knobs (rekordbox's law).
            s.cue_gain = if perf && hp_on && self.px.mix[n as usize].cue { crate::mixer::cue_master_balance(1.0 - hp_mix) * hp_level } else { 0.0 };
            s.deck_no = n;
            s.slip = perf && x.slip;
            let m = self.px.mix[n as usize];
            s.fader = if perf {
                // Crossfader side as assigned (THRU: not through the crossfader at all).
                match m.assign {
                    1 => faders.0,
                    2 => faders.1,
                    _ => 1.0,
                }
            } else if two {
                if n % 2 == 0 { faders.0 } else { faders.1 }
            } else {
                1.0
            };
            // The Performance channel strip. In STEM ISO mode the knobs set the parts' levels
            // (INST also sets BASS) and the EQ is out of the way.
            s.strip = if !perf { 0 } else if isolator { 2 } else { 1 };
            s.trim = if perf { crate::mixer::trim_gain(m.trim) } else { 1.0 };
            s.ch_fader = if perf { crate::mixer::channel_fader_gain(m.fader, curve) } else { 1.0 };
            s.eq_pos = if perf && !m.stem_iso_on { m.eq } else { [0.5; 3] };
            let _ = three;
            s.stem_iso = if perf && m.stem_iso_on {
                let g = |v: f32| crate::audio::stem_iso_gain(v);
                [g(m.stem_iso[0]), g(m.stem_iso[1]), g(m.stem_iso[1]), g(m.stem_iso[2])]
            } else {
                [1.0; 4]
            };
            s.eq_on = dual;
            s.eq_gain = if dual { [!x.eq_kill[0] as u8 as f32, !x.eq_kill[1] as u8 as f32, !x.eq_kill[2] as u8 as f32] } else { [1.0; 3] };
            s.sync_follow = synced;
            if let Some(g) = grid {
                s.grid = g;
            }
        }
        // The deck engine's own rules on top: sync rate and range, vinyl ramps, crossfader
        // curve and assign, Headroom, EQ type.
        self.push_deck_engine(infos.map(|i| i.1), master_bpm);
        crate::midi::after_push(self);
    }

    // ------------------------------------------------------------------ tempo, sync and key
    /// The speed the active deck plays at (1.0 = the track's own tempo).
    pub fn deck_speed(&self) -> f64 {
        self.player.state.lock().tempo
    }

    /// The tempo heard on the active deck: the grid's BPM at the playhead times the speed.
    pub fn effective_bpm(&self) -> f64 {
        self.current_bpm() * self.deck_speed()
    }

    /// The active deck follows the master's tempo (BEAT SYNC on, and it is not the master).
    pub fn sync_follower(&self) -> bool {
        self.two_players() && self.deck.x.beat_sync && (self.px.active != self.px.master || self.sampler_is_master())
    }

    /// TEMPO − / +: one step slower or faster, inside the tempo range.
    pub fn tempo_step(&mut self, dir: i32) {
        if self.sync_follower() {
            self.toast_err("Invalid operation. In Sync mode, Tempo Slider is only available for Master Deck.");
            return;
        }
        let v = ((self.deck.x.tempo_adj + dir.signum() as f64 * TEMPO_STEP) * 1000.0).round() / 1000.0;
        let range = self.tempo_range();
        self.deck.x.tempo_adj = v.clamp(-range, range);
        self.push_deck_controls();
    }

    /// RST: back to the track's own tempo.
    pub fn tempo_reset(&mut self) {
        if self.sync_follower() {
            // Performance, as rekordbox's `requestTempoReset`: a follower leaves SYNC and
            // goes back to its own tempo.
            if !self.performance() {
                self.toast_err("Invalid operation. In Sync mode, Tempo Reset is only available for Master Deck.");
                return;
            }
            self.deck.x.beat_sync = false;
            self.prefs.deck.player_sync[(self.px.active as usize).min(3)] = false;
        }
        self.deck.x.tempo_adj = 0.0;
        self.push_deck_controls();
    }

    /// A BPM typed into the tempo field: the deck plays at it when it is inside the range.
    pub fn tempo_enter_bpm(&mut self, text: &str) {
        if self.sync_follower() {
            self.toast_err("Invalid operation. In Sync mode, Tempo Slider is only available for Master Deck.");
            return;
        }
        let own = self.current_bpm();
        let Ok(v) = text.trim().replace(',', ".").parse::<f64>() else {
            self.toast_err("Invalid Operation. Enter a BPM value within the Tempo Range.");
            return;
        };
        if own <= 0.0 {
            return;
        }
        let adj = v / own - 1.0;
        let range = self.tempo_range();
        if !(-range - 1e-9..=range + 1e-9).contains(&adj) {
            self.toast_err("Invalid Operation. Enter a BPM value within the Tempo Range.");
            return;
        }
        self.deck.x.tempo_adj = adj;
        self.push_deck_controls();
    }

    pub fn toggle_master_tempo(&mut self) {
        self.deck.x.master_tempo = !self.deck.x.master_tempo;
        self.push_deck_controls();
    }

    /// Hold (-1 / +1) or release (0) a pitch bend button.
    pub fn pitch_bend(&mut self, dir: i32) {
        if self.deck.x.bend != dir {
            self.deck.x.bend = dir;
            self.push_deck_controls();
        }
    }

    /// The sampler deck is the sync master (PERFORMANCE mode only).
    pub fn sampler_is_master(&self) -> bool {
        self.performance() && self.px.sampler.master
    }

    /// MASTER: the active deck becomes the one BEAT SYNC and KEY SYNC follow.
    pub fn set_master(&mut self) {
        self.px.master = self.px.active;
        self.px.sampler.master = false;
        self.push_deck_controls();
    }

    /// BEAT SYNC on / off for the active deck. Switched on while both decks play, the deck
    /// jumps onto the master's beat at once; from then on the audio thread holds it there.
    pub fn toggle_beat_sync(&mut self) {
        let on = !self.deck.x.beat_sync;
        if on && self.px.active != self.px.master && self.deck.track.is_some() && self.deck.beats.len() < 2 {
            self.toast_err("Beat Sync is unavailable. The track has no beat grid information.");
        }
        if !on && self.px.active != self.px.master {
            // The tempo it was synced to stays, as on a player.
            let sp = self.deck_speed();
            if sp > 0.0 {
                self.deck.x.tempo_adj = sp - 1.0;
            }
        }
        self.deck.x.beat_sync = on;
        // (A new SYNC starts on rekordbox's own rate and on the beat again.)
        self.deck.x.dk.sync_rate = None;
        self.deck.x.dk.beat_lost = false;
        self.deck.x.dk.out_of_range = false;
        self.prefs.deck.player_sync[(self.px.active as usize).min(3)] = on;
        self.push_deck_controls();
        if on {
            self.sync_align();
        }
    }

    /// Move the active deck (a playing BEAT SYNC follower) by less than a beat so that its
    /// beats fall on the master's.
    pub fn sync_align(&mut self) {
        // (With the sampler deck as master the audio thread pulls the deck onto its beats.)
        if !self.sync_follower() || self.sampler_is_master() {
            return;
        }
        let (pf, pm, gf, gm, both) = {
            let (f, m) = (self.player.state.lock(), self.player.decks[self.px.master.min(3) as usize].lock());
            let t = |s: &crate::audio::PlayState| s.audio.as_ref().map(|a| s.pos / a.sr as f64);
            (t(&f), t(&m), f.grid.clone(), m.grid.clone(), f.playing && m.playing)
        };
        let (Some(pf), Some(pm), Some(gf), Some(gm), true) = (pf, pm, gf, gm, both) else { return };
        if let Some(e) = crate::audio::phase_error(&gf, pf, &gm, pm) {
            let len = self.beat_len();
            self.seek(pf - e * len);
        }
    }

    /// PLAY/PAUSE in the two-deck layouts: a BEAT SYNC follower starts on the master's beat.
    pub fn play_pause_synced(&mut self) {
        self.play_pause();
        if self.playing() {
            self.sync_align();
        }
    }

    pub fn toggle_key_sync(&mut self) {
        self.deck.x.key_sync = !self.deck.x.key_sync;
        if !self.deck.x.key_sync {
            // KEY SYNC off: back to the track's own key.
            self.deck.x.key_shift = 0;
        }
        self.push_deck_controls();
    }

    /// Key shift ‹ ›: one semitone down or up (at most an octave). Takes the deck out of KEY SYNC.
    pub fn key_shift_step(&mut self, dir: i32) {
        if self.deck.track.is_none() {
            return;
        }
        self.deck.x.key_sync = false;
        self.deck.x.key_shift = (self.deck.x.key_shift + dir.signum()).clamp(-12, 12);
        self.push_deck_controls();
    }

    pub fn key_reset(&mut self) {
        self.deck.x.key_sync = false;
        self.deck.x.key_shift = 0;
        self.push_deck_controls();
    }

    /// The key the active deck sounds in (the track's key moved by the key shift), as shown.
    pub fn result_key_label(&self) -> String {
        let Some(t) = self.deck.track.and_then(|id| self.track(id)) else { return String::new() };
        let shift = if self.key_controls() { self.deck.x.key_shift } else { 0 };
        if shift == 0 {
            t.key_label(self.prefs.key_alnum)
        } else {
            key_name(shift_key(t.key, shift), self.prefs.key_alnum)
        }
    }

    // ------------------------------------------------------------------ mixer
    pub fn set_crossfader(&mut self, x: f32) {
        self.px.crossfader = x.clamp(0.0, 1.0);
        self.push_deck_controls();
    }

    /// HIGH / MID / LOW (band 2 / 1 / 0): switch a band of the active deck off or back on.
    pub fn toggle_eq_kill(&mut self, band: usize) {
        if band < 3 {
            self.deck.x.eq_kill[band] = !self.deck.x.eq_kill[band];
            self.push_deck_controls();
        }
    }

    /// The active deck's gain in dB (the loaded track's auto gain, or what A-GAIN set).
    pub fn gain_db(&self) -> f32 {
        let g = self.player.state.lock().gain;
        if self.deck.track.is_none() || g <= 0.0 {
            0.0
        } else {
            20.0 * g.log10()
        }
    }

    /// A-GAIN: set the active deck's gain. With `store` the value becomes the track's auto
    /// gain (kept in the library, as rekordbox keeps an adjusted Auto Gain value).
    pub fn set_gain_db(&mut self, db: f32, store: bool) {
        let Some(id) = self.deck.track else { return };
        let db = (db.clamp(-GAIN_RANGE_DB, 12.0) * 10.0).round() / 10.0;
        self.player.state.lock().gain = 10f32.powf(db / 20.0);
        if store {
            let changed = self.track(id).map(|t| !(t.gain_db - db).abs().lt(&0.05)).unwrap_or(false);
            if changed {
                if let Some(t) = self.track_mut(id) {
                    t.gain_db = db;
                }
                self.save_track(id);
            }
        }
    }

    // ------------------------------------------------------------------ stem waveform
    /// The waveforms show the parts that sound instead of the whole mix (Preferences > STEMS >
    /// STEM Waveform Display, with a part switched off and its waveform ready).
    pub fn stem_wave_shown(&self) -> bool {
        if !self.prefs.stem_waveform {
            return false;
        }
        let on = self.player.state.lock().stem_on;
        self.deck.x.stem_wave.as_ref().map(|w| w.0 == on).unwrap_or(false) && !on.iter().all(|x| *x)
    }

    /// The enlarged waveform is being worked out again for the parts that now sound. rekordbox
    /// shows it empty meanwhile (grid, cues and playhead stay), then draws the new one.
    pub fn stem_wave_preparing(&self) -> bool {
        if !self.prefs.stem_waveform {
            return false;
        }
        let s = self.player.state.lock();
        s.stems.is_some() && !s.stem_on.iter().all(|x| *x) && !self.deck.x.stem_wave.as_ref().map(|w| w.0 == s.stem_on).unwrap_or(false)
    }

    /// The stem waveform of every deck, each frame and in every mode. (It used to be polled
    /// from the Export player's panel only, so a Performance deck whose part was muted waited
    /// for a waveform that was never started and showed an empty lane.)
    pub fn poll_stem_waves(&mut self) {
        for n in 0..self.deck_count() {
            self.with_deck(n, |app| app.poll_stem_wave());
        }
    }

    /// Keep the active deck's stem waveform in step with its part switches: worked out on a
    /// background thread from the audible stems whenever the set of parts changes.
    pub fn poll_stem_wave(&mut self) {
        let (stems, on) = {
            let s = self.player.state.lock();
            (s.stems.clone(), s.stem_on)
        };
        let (Some(stems), Some(tid), true, false) = (stems, self.deck.track, self.prefs.stem_waveform, on.iter().all(|x| *x)) else {
            self.deck.x.stem_wave_rx = None;
            return;
        };
        if let Some((id, key, rx)) = &self.deck.x.stem_wave_rx {
            if let Ok(w) = rx.try_recv() {
                if *id == tid {
                    self.deck.x.stem_wave = Some((*key, w));
                }
                self.deck.x.stem_wave_rx = None;
            }
        }
        let have = self.deck.x.stem_wave.as_ref().map(|w| w.0 == on).unwrap_or(false);
        let running = self.deck.x.stem_wave_rx.as_ref().map(|j| j.1 == on && j.0 == tid).unwrap_or(false);
        if !have && !running {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let ctx = self.ctx.clone();
            // The instruments are the mix less the other parts (as in playback), which needs
            // the decoded original at the stems' rate; otherwise the model's own part is used.
            let orig = self.deck.audio.clone().filter(|a| a.sr == crate::stems::htdemucs::SAMPLE_RATE as u32);
            std::thread::spawn(move || {
                let n = stems.len;
                let mut mono = vec![0.0f32; n];
                let mut add = |src: usize, k: f32| {
                    let (l, r) = (&stems.data[src * 2 * n..(src * 2 + 1) * n], &stems.data[(src * 2 + 1) * n..(src * 2 + 2) * n]);
                    for i in 0..n {
                        mono[i] += k * 0.5 * (l[i] + r[i]);
                    }
                };
                match orig.as_deref().filter(|_| on[2]) {
                    Some(a) => {
                        for src in [0, 1, 3] {
                            if !on[src] {
                                add(src, -1.0);
                            }
                        }
                        for (i, m) in mono.iter_mut().enumerate().take(a.frames()) {
                            *m += 0.5 * (a.samples[i * 2] + a.samples[i * 2 + 1]);
                        }
                    }
                    None => {
                        for (src, on) in on.iter().enumerate() {
                            if *on {
                                add(src, 1.0);
                            }
                        }
                    }
                }
                let _ = tx.send(crate::analysis::waveform(&mono, crate::stems::htdemucs::SAMPLE_RATE as f64));
                ctx.request_repaint();
            });
            self.deck.x.stem_wave_rx = Some((tid, on, rx));
        }
    }

    // ------------------------------------------------------------------ phrase edit
    fn phrase_key(id: i64) -> String {
        format!("phrase_edit:{id}")
    }

    /// The phrases a track was given with PHRASE EDIT, if any.
    pub fn stored_phrases(&self, id: i64) -> Option<crate::phrase::Structure> {
        self.db.setting(&Self::phrase_key(id)).filter(|s| !s.is_empty()).and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Keep (or with None, forget) a track's edited phrases, and show them on any deck that
    /// has the track. Forgetting brings the analysed phrases back.
    pub fn store_phrases(&mut self, id: i64, st: Option<crate::phrase::Structure>) {
        self.db.set_setting(&Self::phrase_key(id), &st.as_ref().and_then(|s| serde_json::to_string(s).ok()).unwrap_or_default());
        let edited = st.is_some();
        for n in 0..self.deck_count() {
            let has = self.deck_of(n).track == Some(id);
            if has {
                let st = st.clone();
                self.with_deck(n, |a| {
                    a.deck.x.phrases_edited = edited;
                    match st {
                        Some(s) => a.deck.phrases = Some(s),
                        None => a.reanalyse_deck_phrases(),
                    }
                });
            }
        }
    }

    /// 1-based number of the beat under the active deck's playhead.
    fn playhead_beat(&self) -> u16 {
        crate::phrase::beat_number(&self.deck.beats, self.pos())
    }

    /// The phrase under the playhead: (mood, kind).
    pub fn current_phrase(&self) -> Option<(u16, u16)> {
        let st = self.deck.phrases.as_ref()?;
        if self.deck.beats.is_empty() {
            return None;
        }
        crate::phrase::index_at(st, self.playhead_beat()).map(|i| (st.mood, st.phrases[i].kind))
    }

    fn phrase_change(&mut self, f: impl FnOnce(&mut crate::phrase::Structure, u16) -> bool) -> bool {
        let (Some(id), Some(mut st)) = (self.deck.track, self.deck.phrases.clone()) else { return false };
        if self.deck.beats.is_empty() {
            return false;
        }
        let beat = self.playhead_beat();
        if !f(&mut st, beat) {
            return false;
        }
        self.record_phrases(id);
        self.store_phrases(id, Some(st));
        true
    }

    /// PHRASE EDIT > CUT: split the phrase at the playhead (on the beat it is in).
    pub fn phrase_cut(&mut self) {
        if self.deck.phrases.is_none() {
            self.toast_err("This track has no phrases yet.");
            return;
        }
        if !self.phrase_change(crate::phrase::cut) {
            self.toast("A phrase already starts here.");
        }
    }

    /// PHRASE EDIT's type list: change the type of the phrase under the playhead.
    pub fn phrase_set_kind(&mut self, kind: u16) {
        self.phrase_change(|s, b| crate::phrase::set_kind(s, b, kind));
    }

    /// PHRASE EDIT > CLEAR: drop the track's phrase edits (the analysed phrases come back).
    pub fn phrase_clear(&mut self) {
        let Some(id) = self.deck.track else { return };
        if self.stored_phrases(id).is_none() {
            return;
        }
        self.record_phrases(id);
        self.store_phrases(id, None);
    }

    pub fn toggle_mute(&mut self) {
        self.px.muted = !self.px.muted;
        self.poll_decks();
    }

    // ------------------------------------------------------------------ transport
    /// Previous Track (-1) / Next Track (+1): load the neighbouring row of the track list.
    pub fn load_adjacent(&mut self, dir: i32) {
        if self.list_dirty {
            self.rebuild_list();
        }
        let row = adjacent_row(&self.list, self.deck.track, dir, |id| self.track(id).map(|t| !t.missing).unwrap_or(false));
        if let Some(i) = row {
            let id = self.list[i];
            self.load_track_here(id);
            self.set_anchor(i);
            self.selected.clear();
            self.selected.insert(id);
            self.scroll_to_selected = true;
        }
    }

    /// Previous Track, as on a player: back to the start of the track when it is more than a
    /// second in, otherwise the previous row of the track list.
    pub fn previous_track(&mut self) {
        if self.deck.audio.is_some() && self.pos() > 1.0 {
            self.seek(0.0);
            if !self.playing() {
                self.deck.cue_point = 0.0;
            }
        } else {
            self.load_adjacent(-1);
        }
    }

    /// Hold a SEARCH button: -1 backward, +1 forward, 0 released.
    pub fn search(&mut self, dir: i32) {
        let mut s = self.player.state.lock();
        s.search = if !self.deck_options.disable_play && s.audio.is_some() { dir.signum() as f64 * SEARCH_SPEED } else { 0.0 };
    }

    pub fn searching(&self) -> i32 {
        let s = self.player.state.lock().search;
        if s > 0.0 {
            1
        } else if s < 0.0 {
            -1
        } else {
            0
        }
    }

    /// Empty the active deck (click on the artwork).
    pub fn unload_track(&mut self) {
        self.deck.cancel_work.store(true, std::sync::atomic::Ordering::Relaxed);
        {
            let mut s = self.player.state.lock();
            s.playing = false;
            s.audio = None;
            s.pos = 0.0;
            s.loop_range = None;
            s.metronome = None;
            s.stems = None;
            s.stem_on = [true; 4];
            s.search = 0.0;
        }
        let (ctrl_tab, panel_tab, metronome) = (self.deck.ctrl_tab, self.deck.panel_tab, self.deck.metronome);
        let x = self.deck.x.carried();
        self.deck = Deck { ctrl_tab, panel_tab, metronome, x, ..Deck::default() };
        self.deck_backup = None;
        self.cancel_orphaned_stems();
    }

    // ------------------------------------------------------------------ cues and loops at a position
    /// The loop in play, or the last one set.
    pub fn defined_loop(&self) -> Option<(f64, f64)> {
        self.loop_active().or(self.deck.last_loop).filter(|(a, b)| b > a)
    }

    /// "Set Cue(In) Point": the cue point, which is also where a loop set with "Set Out
    /// Point" begins. A stopped deck moves there.
    pub fn set_cue_at(&mut self, t: f64) {
        if self.deck.audio.is_none() {
            return;
        }
        let q = self.quantized(t.clamp(0.0, self.deck.duration.max(0.0)));
        self.deck.cue_point = q;
        self.deck.loop_in = Some(q);
        if !self.playing() {
            self.seek(q);
        }
    }

    /// "Set Out Point": loop from the in point (the cue point if none was set) to `t`.
    pub fn set_out_at(&mut self, t: f64) {
        if self.deck.audio.is_none() {
            return;
        }
        let a = self.deck.loop_in.unwrap_or(self.deck.cue_point);
        let b = self.quantized(t.clamp(0.0, self.deck.duration.max(0.0)));
        if b > a + MIN_LOOP {
            self.player.state.lock().loop_range = Some((a, b));
            self.deck.last_loop = Some((a, b));
            self.deck.loop_in = Some(a);
        } else {
            self.toast_err("The out point must be after the cue (in) point.");
        }
    }

    /// Store a hot cue (or, with `end`, a hot loop) in a slot, replacing what was there.
    pub fn set_hot(&mut self, slot: i32, start: f64, end: Option<f64>) {
        let Some(tid) = self.deck.track else { return };
        if self.deck.audio.is_none() {
            return;
        }
        let cue = Cue { id: 0, track_id: tid, kind: CueKind::Hot, slot, start, end,
            color: crate::pads::hot_cue_set_color(self.prefs.pads.hot_cue_color, slot, end.is_some()), comment: String::new(), active_loop: false };
        self.edit_hot_cue(slot, Some(&cue));
    }

    /// "Set Hot Cue A…": at a position of the waveform (quantized when Q is on).
    pub fn set_hot_cue_at(&mut self, slot: i32, t: f64) {
        let q = self.quantized(t.clamp(0.0, self.deck.duration.max(0.0)));
        self.set_hot(slot, q, None);
    }

    /// "Set Hot Loop A…": the loop in play (or last set) as a hot cue.
    pub fn set_hot_loop(&mut self, slot: i32) {
        if let Some((a, b)) = self.defined_loop() {
            self.set_hot(slot, a, Some(b));
        }
    }

    /// "Set Memory Loop": the loop in play (or last set) as a memory loop.
    pub fn set_memory_loop(&mut self) {
        let Some(tid) = self.deck.track else { return };
        let Some((a, b)) = self.defined_loop() else { return };
        if self.deck.cues.iter().any(|c| c.kind == CueKind::Memory && (c.start - a).abs() < 0.001 && c.end.map(|e| (e - b).abs() < 0.001).unwrap_or(false)) {
            return;
        }
        if self.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).count() >= 10 {
            self.toast_err("This track already has 10 memory cues and loops, the most it can hold. Delete one to add another.");
            return;
        }
        let mut c = Cue { id: 0, track_id: tid, kind: CueKind::Memory, slot: -1, start: a, end: Some(b), color: 0, comment: String::new(), active_loop: false };
        self.snapshot_cues();
        if let Ok(id) = self.db.save_cue(&c) {
            c.id = id;
            self.deck.cues.push(c);
            self.refresh_cue_counts(tid);
        }
    }

    /// "Export Loop As Wav": the loop's audio as a 16-bit WAV file the user chooses.
    pub fn export_loop_wav(&mut self) {
        if self.defined_loop().is_none() || self.deck.audio.is_none() {
            self.toast_err("Set a loop first.");
            return;
        }
        let t = self.deck.track.and_then(|id| self.track(id)).cloned();
        let title: String = t.as_ref().map(|t| t.display_title()).unwrap_or_else(|| "Loop".into()).chars().map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
        let Some(path) = rfd::FileDialog::new().set_title("Export Loop As Wav").add_filter("WAV File", &["wav"]).set_file_name(format!("{title} (Loop).wav")).save_file() else { return };
        // Never write over the track itself.
        if t.map(|t| std::path::Path::new(&t.path) == path).unwrap_or(false) {
            self.toast_err("Choose another name: that is the track's own file.");
            return;
        }
        match self.export_loop_to(&path) {
            Ok(()) => self.toast(format!("Loop exported: {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())),
            Err(e) => self.toast_err(format!("The loop could not be exported: {e}")),
        }
    }

    /// Write the defined loop's audio to `path` as a 16-bit WAV file.
    pub fn export_loop_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        let (Some((a, b)), Some(audio)) = (self.defined_loop(), self.deck.audio.clone()) else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "no loop is set"));
        };
        // (Advanced > Others > Export Loop As WAV > Sample Rate: 48 kHz or 44.1 kHz.)
        let rate = if matches!(self.prefs.rb.loop_wav_rate, 44100 | 48000) { self.prefs.rb.loop_wav_rate } else { audio.sr };
        let s = crate::prefs_bridge::resample_stereo(loop_samples(&audio.samples, audio.sr, a, b), audio.sr, rate);
        write_wav16(path, rate, 2, &s)
    }

    // ------------------------------------------------------------------ loop buttons
    /// Exit/Reloop: leave the loop in play, or go back to the last one.
    pub fn exit_reloop(&mut self) {
        self.deck.x.loop_adjust = 0;
        if self.loop_active().is_some() {
            self.loop_exit();
        } else {
            self.reloop();
        }
    }

    /// Loop In: sets the in point; during a loop it switches in-point adjust on and off.
    pub fn loop_in_pressed(&mut self) {
        if self.loop_active().is_some() {
            self.deck.x.loop_adjust = if self.deck.x.loop_adjust == 1 { 0 } else { 1 };
        } else {
            self.loop_in();
        }
    }

    /// Loop Out: closes the loop; during a loop it switches out-point adjust on and off.
    pub fn loop_out_pressed(&mut self) {
        if self.loop_active().is_some() {
            self.deck.x.loop_adjust = if self.deck.x.loop_adjust == 2 { 0 } else { 2 };
        } else {
            self.loop_out();
        }
    }

    /// Move the loop edge being adjusted to `t` (quantized when Q is on).
    pub fn loop_adjust_to(&mut self, t: f64) {
        let edge = self.deck.x.loop_adjust;
        let Some(l) = self.loop_active() else { return };
        let cur = if edge == 1 { l.0 } else { l.1 };
        let n = adjust_loop(l, edge, self.quantized(t) - cur, self.deck.duration);
        self.player.state.lock().loop_range = Some(n);
        self.deck.last_loop = Some(n);
    }

    /// ←/→ in loop adjust: one quantize step with Q on (5 ms without), 1 ms when `fine`.
    pub fn loop_adjust_step(&mut self, dir: i32, fine: bool) {
        let edge = self.deck.x.loop_adjust;
        let Some(l) = self.loop_active() else { return };
        let step = if fine {
            0.001
        } else if self.prefs.quantize && !self.deck.beats.is_empty() {
            self.beat_len() / self.prefs.quantize_div.max(1) as f64
        } else {
            0.005
        };
        let n = adjust_loop(l, edge, dir as f64 * step, self.deck.duration);
        self.player.state.lock().loop_range = Some(n);
        self.deck.last_loop = Some(n);
    }

    /// Start (or with the same length, stop) an auto beat loop of `beats`, remembering the size.
    pub fn pick_auto_loop(&mut self, beats: f64) {
        self.prefs.loop_beats = beats;
        self.deck.x.loop_adjust = 0;
        self.auto_loop(beats);
        self.save_prefs();
    }

    // ------------------------------------------------------------------ grid
    /// "Set a beat to the current position": slide the grid (whole track, or from the current
    /// position on) so that its nearest beat lands on the playhead.
    pub fn grid_center(&mut self) {
        let p = self.pos();
        match nearest_beat(&self.deck.beats, p) {
            Some(b) if (p - b.time).abs() > 1e-6 => self.grid_shift(p - b.time),
            Some(_) => {}
            // No grid: let grid_shift explain.
            None => self.grid_shift(0.0),
        }
    }

    pub fn set_metronome_volume(&mut self, step: u8) {
        self.prefs.metronome_vol = step.clamp(1, 3);
        self.update_metronome();
        self.save_prefs();
    }

    /// Text the GRID panel's BPM field accepted with Enter.
    pub fn grid_enter_bpm(&mut self, text: &str) {
        match text.trim().replace(',', ".").parse::<f64>() {
            Ok(v) => self.grid_set_bpm(v),
            Err(_) => self.toast_err("BPM value should be between 40 and 499."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize) -> Vec<Beat> {
        // 120 BPM from 1.0 s; the first beat is a downbeat.
        (0..n).map(|i| Beat { time: 1.0 + i as f64 * 0.5, bpm: 120.0, beat: (i % 4) as u8 + 1, index: i }).collect()
    }

    #[test]
    fn wav_header_and_samples() {
        let b = wav16_bytes(44100, 2, &[0.0, 1.0, -1.0, 0.5, 2.0, -2.0]);
        assert_eq!(b.len(), 44 + 12);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 12);
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes(b[20..22].try_into().unwrap()), 1, "PCM");
        assert_eq!(u16::from_le_bytes(b[22..24].try_into().unwrap()), 2, "channels");
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 44100);
        assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 44100 * 4, "byte rate");
        assert_eq!(u16::from_le_bytes(b[32..34].try_into().unwrap()), 4, "block align");
        assert_eq!(u16::from_le_bytes(b[34..36].try_into().unwrap()), 16, "bits");
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 12);
        let s: Vec<i16> = b[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        // Full scale, half scale, and clipping of out-of-range input.
        assert_eq!(s, vec![0, 32767, -32767, 16384, 32767, -32767]);
    }

    #[test]
    fn wav_file_decodes_back_to_the_loop() {
        // One second of a ramp at 8 kHz; export 0.25–0.75 s and read it back with the decoder.
        let sr = 8000u32;
        let samples: Vec<f32> = (0..sr as usize).flat_map(|i| { let v = i as f32 / sr as f32 - 0.5; [v, -v] }).collect();
        let part = loop_samples(&samples, sr, 0.25, 0.75);
        assert_eq!(part.len(), 4000 * 2);
        assert_eq!(part[0], samples[2000 * 2]);
        let dir = std::env::temp_dir().join(format!("recordbox-wav-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loop.wav");
        write_wav16(&path, sr, 2, part).unwrap();
        let d = crate::audio::decode_file(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(d.sr, sr);
        assert_eq!(d.frames(), 4000);
        assert!((d.samples[0] - part[0]).abs() < 1e-3 && (d.samples[1] - part[1]).abs() < 1e-3);
        assert!((d.samples[7998] - part[7998]).abs() < 1e-3);
        // Ranges outside the audio are clamped.
        assert!(loop_samples(&samples, sr, 2.0, 3.0).is_empty());
        assert_eq!(loop_samples(&samples, sr, -1.0, 9.0).len(), samples.len());
    }

    #[test]
    fn beat_count_display() {
        let b = grid(64);
        // Current Position (Bars).
        assert_eq!(beat_count_label(&b, &[], 0.0, 0), "1.1Bars");
        assert_eq!(beat_count_label(&b, &[], 1.0, 0), "1.1Bars");
        assert_eq!(beat_count_label(&b, &[], 1.6, 0), "1.2Bars");
        assert_eq!(beat_count_label(&b, &[], 3.0, 0), "2.1Bars");
        // A memory cue on beat index 16 (9.0 s), another on 32 (17.0 s).
        let mem = [9.0, 17.0];
        assert_eq!(beats_to_next_memory(&b, &mem, 1.0), Some(16));
        assert_eq!(beats_to_next_memory(&b, &mem, 1.2), Some(16));
        assert_eq!(beats_to_next_memory(&b, &mem, 1.5), Some(15));
        assert_eq!(beats_to_next_memory(&b, &mem, 8.7), Some(1));
        // On the cue itself the count turns to the one after it.
        assert_eq!(beats_to_next_memory(&b, &mem, 9.0), Some(16));
        assert_eq!(beats_to_next_memory(&b, &mem, 17.0), None);
        assert_eq!(beat_count_label(&b, &mem, 1.0, 1), "3.4Bars");
        assert_eq!(beat_count_label(&b, &mem, 7.0, 1), "0.4Bars");
        assert_eq!(beat_count_label(&b, &mem, 8.7, 1), "0.1Bars");
        assert_eq!(beat_count_label(&b, &mem, 1.0, 2), "16Beats");
        assert_eq!(beat_count_label(&b, &mem, 8.7, 2), "1Beats");
        assert_eq!(beat_count_label(&b, &mem, 20.0, 1), "-.-Bars");
        assert_eq!(beat_count_label(&b, &mem, 20.0, 2), "-Beats");
        assert_eq!(beat_count_label(&[], &mem, 1.0, 2), "-Beats");
    }

    #[test]
    fn loop_adjust_moves_one_edge_and_keeps_the_loop_valid() {
        let l = (10.0, 12.0);
        assert_eq!(adjust_loop(l, 1, 0.5, 300.0), (10.5, 12.0));
        assert_eq!(adjust_loop(l, 1, -0.5, 300.0), (9.5, 12.0));
        assert_eq!(adjust_loop(l, 2, 0.5, 300.0), (10.0, 12.5));
        assert_eq!(adjust_loop(l, 2, -0.5, 300.0), (10.0, 11.5));
        // The in point stops short of the out point and at the start of the track.
        assert_eq!(adjust_loop(l, 1, 5.0, 300.0), (12.0 - MIN_LOOP, 12.0));
        assert_eq!(adjust_loop(l, 1, -50.0, 300.0), (0.0, 12.0));
        // The out point stops short of the in point and at the end of the track.
        assert_eq!(adjust_loop(l, 2, -5.0, 300.0), (10.0, 10.0 + MIN_LOOP));
        assert_eq!(adjust_loop(l, 2, 500.0, 300.0), (10.0, 300.0));
        assert_eq!(adjust_loop(l, 0, 1.0, 300.0), l);
    }

    #[test]
    fn previous_and_next_track_rows() {
        let list = [10, 11, 12, 13];
        let all = |_| true;
        assert_eq!(adjacent_row(&list, Some(11), 1, all), Some(2));
        assert_eq!(adjacent_row(&list, Some(11), -1, all), Some(0));
        assert_eq!(adjacent_row(&list, Some(13), 1, all), None);
        assert_eq!(adjacent_row(&list, Some(10), -1, all), None);
        // Nothing loaded (or a track from another list): first / last row.
        assert_eq!(adjacent_row(&list, None, 1, all), Some(0));
        assert_eq!(adjacent_row(&list, Some(99), -1, all), Some(3));
        // Missing files are skipped.
        assert_eq!(adjacent_row(&list, Some(10), 1, |id| id != 11), Some(2));
        assert_eq!(adjacent_row(&[], None, 1, all), None);
    }

    #[test]
    fn key_shift_and_key_sync() {
        let k = |n: &str| parse_key(n);
        // One semitone up from A minor is B flat minor; an octave is the same key.
        assert_eq!(shift_key(k("Am"), 1), k("Bbm"));
        assert_eq!(shift_key(k("Am"), -2), k("Gm"));
        assert_eq!(shift_key(k("C"), 7), k("G"));
        assert_eq!(shift_key(k("Am"), 12), k("Am"));
        assert_eq!(shift_key(-1, 3), -1);
        // Compatible: same key, relative major / minor, a fifth either way.
        assert!(keys_compatible(k("Am"), k("Am")) && keys_compatible(k("Am"), k("C")) && keys_compatible(k("Am"), k("Em")) && keys_compatible(k("Am"), k("Dm")));
        assert!(!keys_compatible(k("Am"), k("Bm")) && !keys_compatible(k("Am"), k("G")));
        // KEY SYNC: already compatible keys stay; otherwise the smallest shift that fits.
        assert_eq!(key_sync_shift(k("Am"), k("Am")), 0);
        assert_eq!(key_sync_shift(k("Em"), k("Am")), 0);
        assert_eq!(key_sync_shift(k("Bbm"), k("Am")), -1);
        assert_eq!(key_sync_shift(k("Abm"), k("Am")), 1);
        // G minor: +2 reaches A minor itself, -2 only its neighbour... both two away: the master's key wins.
        assert_eq!(key_sync_shift(k("Bm"), k("Am")), -2);
        for own in 0..24 {
            for master in 0..24 {
                let d = key_sync_shift(own, master);
                assert!((-6..=6).contains(&d) && keys_compatible(shift_key(own, d), master), "{own} {master} {d}");
            }
        }
        assert_eq!(key_sync_shift(-1, 3), 0);
        assert_eq!(shift_text(0), "±0");
        assert_eq!(shift_text(3), "+3");
        assert_eq!(shift_text(-2), "-2");
    }

    #[test]
    fn tempo_at_and_gain_knob() {
        let b = grid(8);
        assert_eq!(bpm_at(&b, 0.0), 120.0);
        assert_eq!(bpm_at(&b, 2.0), 120.0);
        assert_eq!(bpm_at(&[], 2.0), 0.0);
        assert_eq!(gain_angle(0.0), 0.0);
        assert_eq!(gain_angle(-GAIN_RANGE_DB), -135.0);
        assert_eq!(gain_angle(99.0), 135.0);
        assert!((gain_angle(-2.6) + 17.55).abs() < 0.01);
    }

    #[test]
    fn edited_phrases_reach_the_usb_analysis_file() {
        use crate::phrase::*;
        // A track whose phrases were cut and relabelled: the EXT file's PSSI section is the edit.
        let beats = grid(128);
        let mut st = Structure { mood: 1, phrases: vec![Phrase { beat: 1, kind: INTRO }, Phrase { beat: 33, kind: UP }, Phrase { beat: 65, kind: CHORUS }], end_beat: 128 };
        assert!(cut(&mut st, 49) && set_kind(&mut st, 49, DOWN));
        let ov = crate::export::anlz::Overrides { sections: Default::default(), grid_same: false, pssi: Some(pssi(&st)), device: Default::default(), pvdi: None };
        let files = crate::export::anlz::build_with("/Contents/a.mp3", &beats, &Waveform::default(), &[], 0, 44100, Some(&ov));
        let ext = crate::export::anlz::section_list(&files.ext);
        let sec = ext.iter().find(|(t, _)| t == "PSSI").map(|(_, b)| b.clone()).expect("PSSI written");
        assert!(is_masked(&sec));
        assert_eq!(parse(&sec), Some(st));
    }

    #[test]
    fn captions() {
        assert_eq!(jump_label(4.0, false), "4Beats");
        assert_eq!(jump_label(0.5, false), "1/2Beat");
        assert_eq!(jump_label(1.0, false), "1Beat");
        assert_eq!(jump_label(4.0, true), "1Bar");
        assert_eq!(jump_label(32.0, true), "8Bars");
        assert_eq!(beats_text(1.0 / 64.0), "1/64");
        assert_eq!(beats_text(512.0), "512");
    }
}

// ---------------------------------------------------------------------- screenshot checks

/// The PREPDECK_SHOT_PLAYER switches (comma separated, with --screenshot), for looking at
/// player states: `empty` an empty deck · `2p` 2 Players with a second track on deck B · `simple` Simple Player ·
/// `ma` manual loop buttons · `loop` a 4-beat loop · `adjust` loop-out adjust · `count1` /
/// `count2` Beat Count Display modes · `metro` metronome on at volume 3 · `bars` beat jump
/// in bars · `clickmode` click-for-PLAY/CUE on · `menu` the waveform's right-click menu ·
/// `padclear`, `dropload`, `selftest`: scripted checks that print to stderr (scratch library
/// only). Combine with PREPDECK_SHOT_VIEW=grid for the GRID panel and PREPDECK_SHOT_CLICK.
fn shot_flags() -> Vec<String> {
    std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default()
}

impl App {
    /// Wait for the active deck's audio (screenshot runs only).
    pub(crate) fn shot_wait_load(&mut self) {
        if let Some(rx) = self.deck.loading.take() {
            if let Ok(msg) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
                let (tx2, rx2) = crossbeam_channel::bounded(1);
                let _ = tx2.send(msg);
                self.deck.loading = Some(rx2);
                self.poll_load();
            }
        }
    }

    /// Put the player into the states PREPDECK_SHOT_PLAYER names (after the track is loaded).
    pub fn shot_player_setup(&mut self) {
        let flags = shot_flags();
        let has = |f: &str| flags.iter().any(|x| x == f);
        if flags.is_empty() {
            return;
        }
        if has("2p") {
            self.prefs.browser_only = false;
            self.prefs.player_layout = 1;
            let a = self.deck.track;
            let second = self.tracks.iter().filter(|t| t.analyzed == 1 && !t.missing && Some(t.id) != a && t.duration > 60.0).map(|t| t.id).nth(3);
            if let Some(id) = second {
                self.load_track_to(1, id);
                self.with_deck(1, |app| {
                    app.shot_wait_load();
                    let beats = app.deck.beats.clone();
                    if let Some(b) = beats.get(64) {
                        app.seek(b.time);
                        app.hot_cue_down(0);
                        app.deck.cue_point = b.time;
                        app.memory_set();
                    }
                    if let Some(b) = beats.get(96) {
                        app.seek(b.time);
                        app.hot_cue_down(2);
                        app.prefs.loop_auto = true;
                        app.auto_loop(4.0);
                    }
                });
            }
        }
        if has("simple") {
            self.prefs.browser_only = false;
            self.prefs.player_layout = 2;
        }
        // `2pa` / `dual`: the two-deck layouts with deck B empty (references 05 and 06);
        // `dualb`: Dual Player with a second track; `syncb`: BEAT SYNC lit on deck B.
        if has("2pa") {
            self.prefs.browser_only = false;
            self.prefs.player_layout = 1;
        }
        if has("dual") || has("dualb") {
            self.prefs.browser_only = false;
            self.prefs.player_layout = 3;
        }
        if has("dualb") {
            let a = self.deck.track;
            if let Some(id) = self.tracks.iter().filter(|t| t.analyzed == 1 && !t.missing && Some(t.id) != a && t.duration > 60.0).map(|t| t.id).nth(3) {
                self.load_track_to(1, id);
                self.with_deck(1, |app| app.shot_wait_load());
            }
        }
        if has("syncb") {
            self.other_deck_mut().x.beat_sync = true;
        }
        // `plain`: without Recordbox's own additions (STEMS button, phrase bar, vocal marks).
        if has("plain") {
            self.prefs.show_stems = false;
            self.prefs.show_phrase_bar = false;
            self.prefs.show_vocal = false;
        }
        // `gridtab`: the GRID panel, nothing else changed.
        if has("gridtab") {
            self.deck.ctrl_tab = 1;
        }
        if has("ctltest") {
            self.shot_controls_selftest();
        }
        // `empty`: nothing on the deck (the state of reference 01).
        if has("empty") {
            self.unload_track();
        }
        if has("1p") {
            self.prefs.browser_only = false;
            self.prefs.player_layout = 0;
        }
        if has("ma") || has("adjust") {
            self.prefs.loop_auto = false;
        }
        if has("loop") || has("adjust") {
            let p = self.pos();
            self.seek(p + 4.0);
            self.auto_loop(4.0);
            let p = self.pos();
            self.seek(p + 0.6);
        }
        if has("adjust") {
            self.deck.x.loop_adjust = 2;
        }
        if has("count1") {
            self.prefs.beat_count_mode = 1;
        }
        if has("count2") {
            self.prefs.beat_count_mode = 2;
        }
        if has("metro") {
            self.deck.metronome = true;
            self.prefs.metronome_vol = 3;
            self.update_metronome();
        }
        if has("bars") {
            self.prefs.beat_jump = 8.0;
            self.prefs.beat_jump_bars = true;
        }
        if has("clickmode") {
            self.prefs.wave_click_play_cue = true;
        }
        if has("selftest") {
            self.shot_player_selftest();
        }
    }

    /// PREPDECK_SHOT_PLAYER=ctltest: run the two-deck controls and PHRASE EDIT on real tracks
    /// (use a scratch library) and print what happened to stderr.
    fn shot_controls_selftest(&mut self) {
        let Some(tid) = self.deck.track else { return eprintln!("ctltest: no track loaded") };
        self.prefs.browser_only = false;
        self.prefs.player_layout = 3;
        let other = self.tracks.iter().filter(|t| t.analyzed == 1 && !t.missing && t.id != tid && t.duration > 60.0 && t.bpm > 0.0).map(|t| t.id).nth(3);
        let Some(oid) = other else { return eprintln!("ctltest: needs a second analysed track") };
        self.load_track_to(1, oid);
        self.with_deck(1, |a| a.shot_wait_load());
        self.poll_decks();
        let (ba, bb) = (self.current_bpm(), self.with_deck(1, |a| a.current_bpm()));
        eprintln!("ctltest: deck A {:.2} BPM, deck B {:.2} BPM, device {}", ba, bb, self.player.error.clone().unwrap_or_else(|| "ok".into()));
        // Tempo steps, typed BPM, range, reset.
        for _ in 0..10 {
            self.tempo_step(1);
        }
        eprintln!("ctltest: TEMPO + ×10 -> speed {:.4}, {:.2} BPM", self.deck_speed(), self.effective_bpm());
        self.tempo_enter_bpm(&format!("{:.2}", ba * 1.05));
        eprintln!("ctltest: typed {:.2} -> speed {:.4}", ba * 1.05, self.deck_speed());
        self.tempo_enter_bpm(&format!("{:.2}", ba * 1.5));
        eprintln!("ctltest: typed {:.2} (out of range) -> speed {:.4}, message {:?}", ba * 1.5, self.deck_speed(), self.toasts.last().map(|t| t.text.clone()));
        // Playing at +5 % with and without Master Tempo: the position moves 5 % faster.
        self.toggle_master_tempo();
        let p0 = self.pos();
        self.set_playing(true);
        std::thread::sleep(std::time::Duration::from_millis(1000));
        self.set_playing(false);
        let mt = self.player.state.lock().master_tempo;
        eprintln!("ctltest: 1 s at +5 % with MT -> moved {:.3} s of track (MT {})", self.pos() - p0, mt);
        self.tempo_reset();
        eprintln!("ctltest: RST -> speed {:.4}", self.deck_speed());
        // BEAT SYNC: B follows A (the master).
        self.with_deck(1, |a| a.toggle_beat_sync());
        let (ot, of) = {
            let o = self.player.decks[self.partner() as usize].lock();
            (o.tempo, o.sync_follow)
        };
        eprintln!("ctltest: BEAT SYNC on B -> B speed {:.4} ({:.2} BPM, master {:.2}), follower {}", ot, bb * ot, ba, of);
        self.tempo_step(1);
        self.poll_decks();
        let ot = self.player.decks[self.partner() as usize].lock().tempo;
        eprintln!("ctltest: A +0.1 % -> B {:.2} BPM", bb * ot);
        self.with_deck(1, |a| a.tempo_step(1));
        eprintln!("ctltest: TEMPO + on the follower -> {:?}", self.toasts.last().map(|t| t.text.clone()));
        self.set_playing(true);
        self.with_deck(1, |a| {
            let t = a.deck.beats.get(40).map(|b| b.time + 0.11).unwrap_or(5.0);
            a.seek(t);
            a.play_pause_synced();
        });
        let err = |a: &App| -> Option<f64> {
            let (m, f) = (a.player.state.lock(), a.player.decks[a.partner() as usize].lock());
            crate::audio::phase_error(f.grid.as_ref()?, f.pos / f.audio.as_ref()?.sr as f64, m.grid.as_ref()?, m.pos / m.audio.as_ref()?.sr as f64)
        };
        let e0 = err(self);
        std::thread::sleep(std::time::Duration::from_millis(3000));
        eprintln!("ctltest: both playing, B started 0.11 s off a beat -> phase error at start {:?}, after 3 s {:?} beats", e0, err(self));
        self.set_playing(false);
        self.with_deck(1, |a| a.set_playing(false));
        // MASTER to B, KEY SYNC, key shift.
        self.with_deck(1, |a| a.set_master());
        eprintln!("ctltest: MASTER on B -> master {}", self.px.master);
        self.with_deck(0, |a| a.set_master());
        let keys = (self.track(tid).map(|t| t.key_label(false)).unwrap_or_default(), self.track(oid).map(|t| t.key_label(false)).unwrap_or_default());
        self.with_deck(1, |a| a.toggle_key_sync());
        self.poll_decks();
        eprintln!("ctltest: keys A {} / B {}; KEY SYNC on B -> shift {} = {}", keys.0, keys.1, shift_text(self.other_deck().x.key_shift), self.with_deck(1, |a| a.result_key_label()));
        self.with_deck(1, |a| {
            a.key_shift_step(1);
            a.key_shift_step(1);
        });
        let ks = self.player.decks[self.partner() as usize].lock().key_shift;
        eprintln!("ctltest: key shift ›› on B -> {} = {}, audio key_shift {}, KEY SYNC {}", shift_text(self.other_deck().x.key_shift), self.with_deck(1, |a| a.result_key_label()), ks, self.other_deck().x.key_sync);
        self.with_deck(1, |a| a.key_reset());
        // Crossfader, isolator, gain.
        self.set_crossfader(0.0);
        let faders = |a: &App| {
            let x = a.player.state.lock().fader;
            let y = a.player.decks[a.partner() as usize].lock().fader;
            (x, y)
        };
        let f = faders(self);
        self.set_crossfader(0.5);
        eprintln!("ctltest: crossfader at A -> gains {:?}; centre -> {:?}", f, faders(self));
        self.toggle_eq_kill(0);
        let eq = {
            let st = self.player.state.lock();
            (st.eq_gain, st.eq_on)
        };
        eprintln!("ctltest: LOW off on A -> eq {:?} (on {})", eq.0, eq.1);
        self.toggle_eq_kill(0);
        let g0 = self.gain_db();
        self.set_gain_db(g0 + 1.5, true);
        eprintln!("ctltest: A-GAIN {:.1} dB -> {:.1} dB, stored {:.1}", g0, self.gain_db(), self.db.track(tid).map(|t| t.gain_db).unwrap_or(f32::NAN));
        self.undo_edit();
        // PHRASE EDIT.
        self.seek(self.deck.beats.get(48).map(|b| b.time + 0.05).unwrap_or(20.0));
        for _ in 0..200 {
            if self.deck.phrases.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            self.poll_phrases();
        }
        let show = |a: &App| a.deck.phrases.as_ref().map(|s| s.phrases.iter().take(5).map(|p| format!("{}:{}", p.beat, crate::phrase::label(s.mood, p.kind))).collect::<Vec<_>>().join(" ")).unwrap_or_else(|| "none".into());
        eprintln!("ctltest: phrases {} … at beat {}: {:?}", show(self), crate::phrase::beat_number(&self.deck.beats, self.pos()), self.current_phrase());
        let u0 = self.undo_len();
        self.phrase_cut();
        eprintln!("ctltest: CUT -> {} … (edited {}, stored {}, undo +{})", show(self), self.deck.x.phrases_edited, self.stored_phrases(tid).is_some(), self.undo_len() - u0);
        self.phrase_set_kind(crate::phrase::DOWN);
        eprintln!("ctltest: type Down -> {} …, current {:?}", show(self), self.current_phrase());
        self.undo_edit();
        eprintln!("ctltest: Undo -> {} …", show(self));
        self.redo_edit();
        // What a USB export would write for this track.
        let pssi = self.stored_phrases(tid).map(|s| crate::phrase::pssi(&s));
        eprintln!("ctltest: Redo -> {} …; PSSI for export parses back equal: {:?}", show(self), pssi.as_ref().and_then(|b| crate::phrase::parse(b)) == self.stored_phrases(tid));
        self.phrase_clear();
        for _ in 0..200 {
            if self.deck.phrases.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            self.poll_phrases();
        }
        eprintln!("ctltest: CLEAR -> {} … (edited {}, stored {})", show(self), self.deck.x.phrases_edited, self.stored_phrases(tid).is_some());
        self.seek(0.3);
    }

    /// PREPDECK_SHOT_PLAYER=selftest: run the player's operations on the loaded track (use a
    /// scratch library) and print what happened to stderr.
    fn shot_player_selftest(&mut self) {
        let Some(tid) = self.deck.track else { return eprintln!("selftest: no track loaded") };
        let beats = self.deck.beats.clone();
        if beats.len() < 200 || self.deck.audio.is_none() {
            return eprintln!("selftest: needs an analysed, decoded track");
        }
        let hot = |a: &App, slot: i32| a.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).map(|c| (c.start, c.end));
        let saved_hot = |a: &App, slot: i32| a.db.cues(tid).unwrap_or_default().into_iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).map(|c| (c.start, c.end));
        self.prefs.quantize = true;
        self.prefs.quantize_div = 1;
        // Waveform menu actions at a position 0.1 s after beat 40 (quantize snaps to the beat).
        let (b40, b48) = (beats[40].time, beats[48].time);
        self.set_cue_at(b40 + 0.1);
        eprintln!("selftest: Set Cue(In) Point -> cue {:.3} (beat 40 at {:.3}), deck at {:.3}", self.deck.cue_point, b40, self.pos());
        self.set_out_at(b48 + 0.1);
        eprintln!("selftest: Set Out Point -> loop {:?} (beats 40..48 = {:.3}..{:.3})", self.loop_active(), b40, b48);
        let u0 = self.undo_len();
        self.set_hot_cue_at(1, beats[60].time + 0.05);
        eprintln!("selftest: Set Hot Cue B -> deck {:?}, saved {:?} (beat 60 at {:.3}), undo +{}", hot(self, 1), saved_hot(self, 1), beats[60].time, self.undo_len() - u0);
        self.set_hot_cue_at(1, beats[64].time);
        eprintln!("selftest: Set Hot Cue B again -> {:?}, hot cues in slot B: {}", hot(self, 1), self.deck.cues.iter().filter(|c| c.kind == CueKind::Hot && c.slot == 1).count());
        self.undo_edit();
        eprintln!("selftest: Undo -> B {:?}", saved_hot(self, 1));
        self.set_hot_loop(2);
        eprintln!("selftest: Set Hot Loop C -> {:?}", saved_hot(self, 2));
        let m0 = self.memory_cues().len();
        self.set_memory_loop();
        self.set_memory_loop();
        let ml = self.memory_cues().into_iter().find(|c| c.is_loop()).map(|c| (c.start, c.end));
        eprintln!("selftest: Set Memory Loop (twice) -> {} new, {:?}", self.memory_cues().len() - m0, ml);
        // Export Loop As Wav.
        let wav = std::env::temp_dir().join(format!("recordbox-selftest-{}.wav", std::process::id()));
        match self.export_loop_to(&wav).map_err(|e| e.to_string()).and_then(|_| crate::audio::decode_file(wav.to_str().unwrap_or_default()).map_err(|e| e.to_string())) {
            Ok(d) => eprintln!("selftest: Export Loop As Wav -> {} bytes, {:.3} s at {} Hz (loop is {:.3} s)", std::fs::metadata(&wav).map(|m| m.len()).unwrap_or(0), d.duration(), d.sr, b48 - b40),
            Err(e) => eprintln!("selftest: Export Loop As Wav FAILED: {e}"),
        }
        let _ = std::fs::remove_file(&wav);
        // Loop adjust and Exit/Reloop.
        self.loop_out_pressed();
        self.loop_adjust_step(1, false);
        eprintln!("selftest: OUT adjust, → -> loop {:?} (one beat longer: {:.3}), adjust {}", self.loop_active(), beats[49].time, self.deck.x.loop_adjust);
        self.loop_adjust_step(-1, true);
        eprintln!("selftest: Shift+← -> loop {:?}", self.loop_active());
        self.loop_in_pressed();
        self.loop_adjust_to(beats[42].time + 0.02);
        eprintln!("selftest: IN adjust, drag to beat 42 -> loop {:?} ({:.3})", self.loop_active(), beats[42].time);
        self.exit_reloop();
        let off = self.loop_active();
        self.exit_reloop();
        eprintln!("selftest: Exit/Reloop -> {:?}, again -> {:?}, deck at {:.3}", off, self.loop_active(), self.pos());
        self.exit_reloop();
        self.pick_auto_loop(0.25);
        eprintln!("selftest: 1/4 Beat Loop from the list -> {:?} (beat = {:.4} s)", self.loop_active().map(|(a, b)| b - a), self.beat_len());
        self.loop_exit();
        self.prefs.loop_beats = 4.0;
        // Grid: Center, BPM entry and its limits, Undo.
        let p = beats[80].time + 0.013;
        self.seek(p);
        let g0 = self.deck.grid.anchors.first().map(|a| a.time).unwrap_or(0.0);
        self.grid_center();
        let near = nearest_beat(&self.deck.beats, p).map(|b| b.time).unwrap_or(0.0);
        eprintln!("selftest: grid Center at {:.3} -> nearest beat {:.3}, first anchor moved {:+.3} s", p, near, self.deck.grid.anchors.first().map(|a| a.time).unwrap_or(0.0) - g0);
        self.grid_undo();
        eprintln!("selftest: grid Undo -> first anchor back by {:+.3}", self.deck.grid.anchors.first().map(|a| a.time).unwrap_or(0.0) - g0);
        let bpm0 = self.current_bpm();
        self.grid_enter_bpm("30");
        self.grid_enter_bpm("abc");
        eprintln!("selftest: BPM 30 / abc refused -> {:.2} (was {:.2}), message: {:?}", self.current_bpm(), bpm0, self.toasts.last().map(|t| t.text.clone()));
        self.grid_enter_bpm("126.5");
        eprintln!("selftest: BPM 126.5 -> {:.2}", self.current_bpm());
        self.grid_undo();
        eprintln!("selftest: grid Undo -> {:.2}", self.current_bpm());
        // Beat Count Display with the memory loop at beat 40.
        let mem: Vec<f64> = self.memory_cues().iter().map(|c| c.start).collect();
        eprintln!("selftest: Beat Count at beat 24: {} / {} / {}", beat_count_label(&self.deck.beats, &mem, beats[24].time, 0), beat_count_label(&self.deck.beats, &mem, beats[24].time, 1), beat_count_label(&self.deck.beats, &mem, beats[24].time, 2));
        // Search: the audio callback moves the deck while the button is held.
        let before = self.pos();
        self.search(1);
        std::thread::sleep(std::time::Duration::from_millis(500));
        self.search(0);
        eprintln!("selftest: Search Forward held 0.5 s -> moved {:+.2} s (audio device: {}), playing {}", self.pos() - before, self.player.error.clone().unwrap_or_else(|| "ok".into()), self.playing());
        // Metronome volume reaches the audio state.
        self.deck.metronome = true;
        self.set_metronome_volume(3);
        let (mv, mon) = { let s = self.player.state.lock(); (s.metronome_vol, s.metronome.is_some()) };
        eprintln!("selftest: metronome on at volume 3 -> level {mv}, clicks {mon}");
        self.deck.metronome = false;
        self.set_metronome_volume(2);
        // Tracks: Next, Previous (restart first), unload, and two decks.
        self.rebuild_list();
        let row = |a: &App| a.deck.track.and_then(|t| a.list.iter().position(|x| *x == t));
        let r0 = row(self);
        self.load_adjacent(1);
        self.shot_wait_load();
        let r1 = row(self);
        self.seek(30.0);
        self.previous_track();
        let restarted = (row(self), self.pos());
        self.previous_track();
        self.shot_wait_load();
        eprintln!("selftest: rows: start {:?}, Next Track {:?}, Previous Track at 30 s -> {:?} at {:.1} s, again -> {:?}", r0, r1, restarted.0, restarted.1, row(self));
        self.prefs.player_layout = 1;
        self.prefs.browser_only = false;
        let other = self.list.iter().copied().find(|id| Some(*id) != self.deck.track && self.track(*id).map(|t| !t.missing && t.duration > 60.0).unwrap_or(false));
        if let Some(id) = other {
            self.load_target = 1;
            self.load_track(id);
            self.with_deck(1, |a| a.shot_wait_load());
            self.with_deck(1, |a| a.set_playing(true));
            self.set_playing(true);
            std::thread::sleep(std::time::Duration::from_millis(400));
            let (pa, pb) = (self.pos(), self.with_deck(1, |a| a.pos()));
            let (ta, tb) = (self.deck.track, self.other_deck().track);
            eprintln!("selftest: 2 Players: A track {:?} at {:.2} s playing {}, B track {:?} at {:.2} s playing {}, active {}", ta, pa, self.playing(), tb, pb, self.player.decks[self.partner() as usize].lock().playing, self.px.active);
            self.activate_deck(1);
            eprintln!("selftest: deck B active -> deck.track {:?}, other {:?}, active {}", self.deck.track, self.other_deck().track, self.px.active);
            self.prefs.player_layout = 0;
            self.poll_decks();
            eprintln!("selftest: back to 1 Player -> active {}, deck.track {:?}, B playing {}, load target {}", self.px.active, self.deck.track, self.player.decks[self.partner() as usize].lock().playing, self.load_target);
            self.set_playing(false);
        }
        self.unload_track();
        eprintln!("selftest: unload -> track {:?}, audio {}", self.deck.track, self.player.state.lock().audio.is_some());
    }

    /// Pointer events for PREPDECK_SHOT_PLAYER=menu (right-click on the enlarged waveform,
    /// 150 pt right of the playhead), =padclear (click the clear cross of hot cue pad A) and
    /// =dropload (release a dragged track over the player). Results go to stderr.
    pub fn shot_player_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        let flags = shot_flags();
        let has = |f: &str| flags.iter().any(|x| x == f);
        let button = |pos: egui::Pos2, button: egui::PointerButton, pressed: bool| egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
        // PREPDECK_SHOT_CLICK=x,y (window points; ",r" for the right button): one click at
        // frame 100, to open a menu or press a button for the picture.
        if let Ok(v) = std::env::var("PREPDECK_SHOT_CLICK") {
            let parts: Vec<&str> = v.split(',').collect();
            if let (Some(Ok(x)), Some(Ok(y))) = (parts.first().map(|s| s.trim().parse::<f32>()), parts.get(1).map(|s| s.trim().parse::<f32>())) {
                let b = if parts.get(2).map(|s| s.trim() == "r").unwrap_or(false) { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                match frame {
                    98 => raw.events.push(egui::Event::PointerMoved(egui::pos2(x, y))),
                    100 => raw.events.push(button(egui::pos2(x, y), b, true)),
                    101 => raw.events.push(button(egui::pos2(x, y), b, false)),
                    _ => {}
                }
            }
        }
        let Some((r, _, _)) = self.deck.detail_map else { return };
        if has("menu") {
            let at = egui::pos2(r.center().x + 150.0, r.center().y - 20.0);
            match frame {
                80 => raw.events.push(egui::Event::PointerMoved(at)),
                82 => raw.events.push(button(at, egui::PointerButton::Secondary, true)),
                83 => raw.events.push(button(at, egui::PointerButton::Secondary, false)),
                _ => {}
            }
        }
        if has("padclear") {
            // Pad A: centred 420.4 pt on the 1512 pt design, 22.6 pt into the strip that starts
            // 121 pt below the waveform's top; its cross is in the top right corner.
            let at = egui::pos2(r.min.x + (420.4 - 81.0) * (r.width() / 1219.0) + 9.5, r.min.y + 121.0 + 22.6 - 9.5);
            let a = |app: &App| app.deck.cues.iter().filter(|c| c.kind == CueKind::Hot && c.slot == 0).count();
            match frame {
                80 => {
                    eprintln!("padclear: hot cue A set: {}", a(self));
                    raw.events.push(egui::Event::PointerMoved(at));
                }
                100 => raw.events.push(button(at, egui::PointerButton::Primary, true)),
                101 => raw.events.push(button(at, egui::PointerButton::Primary, false)),
                104 => eprintln!("padclear: after clicking the cross: {} (playing {})", a(self), self.playing()),
                105 => {
                    self.undo_edit();
                    eprintln!("padclear: after Undo: {}", a(self));
                }
                _ => {}
            }
        }
        if has("dropload") {
            let at = r.center();
            match frame {
                80 => raw.events.push(egui::Event::PointerMoved(at)),
                82 => {
                    let other = self.tracks.iter().find(|t| Some(t.id) != self.deck.track && !t.missing && t.analyzed == 1).map(|t| t.id);
                    eprintln!("dropload: loaded {:?}, dragging {:?}", self.deck.track, other);
                    self.drag = other.map(|id| vec![id]);
                }
                100 => raw.events.push(button(at, egui::PointerButton::Primary, false)),
                103 => eprintln!("dropload: after the drop: loaded {:?}, drag {:?}", self.deck.track, self.drag),
                _ => {}
            }
        }
    }
}
