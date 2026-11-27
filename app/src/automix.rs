//! Automix (PERFORMANCE mode): the playlist that plays itself on decks 1 and 2.
//!
//! Traced from rekordbox 7.2.19 (`checks/re-7.2.19/pads-sampler-automix.md` C1 to C4, and the
//! functions read for this file: `AutoMixUnit::reserveAutoMix` 0x100b39158,
//! `activatePlayerIfNeeded` 0x100b3a4a8, the four player behaviours, `AutoMixTrackStructure::init`
//! 0x1012f7364, `RandomTable::init` 0x101a3b280). The rules, in the order things happen:
//!
//! - Start puts both decks in Reserved. A Reserved deck loads the next track of the list as soon
//!   as it is "loadable" (not on air and playing, or at the end of its track) and becomes Ready
//!   (paused on the intro, sync off, Master Tempo on, tempo 0 %).
//! - When both decks are Ready, the one that loaded first has the crossfader snapped to its side
//!   and becomes Active (plays, sync master).
//! - A 100 ms watcher fires once when the Active deck passes the start of its fade-out range (the
//!   last phrase, or the last 30 s without phrases). The crossfader then runs a linear ramp to
//!   the other deck in 100 ms steps over the crossfade length; the other deck starts from its
//!   fade-in point, BPM-synced when the two tempos are within 8 %.
//! - When the ramp ends the new deck is Active (its tempo returns to 0 %), and the old one loads
//!   the next track and is Ready again. When the list has no next track the old deck is
//!   unloaded, the last track plays on and Automix ends.
//!
//! The engine talks to the decks through `Rig`, so the tests drive the same code with a
//! simulated pair of decks.

use crate::app::*;
use crate::model::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- settings and constants

/// "Automix settings" (rekordbox: `AutoMixRepeatMode`, `AutoMixRandomMode`,
/// `kAutoMixDoubleClickLoadabled`, `kAutoMixRemovePlayedTrack`, `kAutoMixCrossfadeTiming`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AutomixPrefs {
    /// The Automix playlist pane is shown beside the track list.
    pub pane: bool,
    pub repeat: bool,
    pub random: bool,
    pub double_click_load: bool,
    /// (rekordbox 7.2.19 keeps this setting but its dialog has no control for it.)
    pub remove_played: bool,
    /// 1 = "Disable auto crossfade manual start", 2 = "...when loading a track to a standby
    /// deck.", 3 = "...when starting a track on a standby deck."
    pub crossfade_timing: u8,
}

impl Default for AutomixPrefs {
    fn default() -> Self {
        AutomixPrefs { pane: false, repeat: true, random: false, double_click_load: false, remove_played: false, crossfade_timing: 2 }
    }
}

/// This track's preferences and session state, hung off `Prefs` and `App` as one field each.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct MixPrefs {
    pub automix: AutomixPrefs,
    pub mpl: crate::mpl::MplPrefs,
    pub lib: crate::sampler_lib::LibPrefs,
}

#[derive(Default)]
pub struct MixExt {
    pub automix: AutomixState,
    pub mpl: crate::mpl::MplState,
    pub lib: crate::sampler_lib::LibState,
}

/// `AutoMixConfig` (ctor 0x100b22f20).
pub const DEFAULT_BPM: f64 = 120.0;
pub const BEATS_SAME_GENRE: f64 = 16.0;
pub const BEATS_OTHER_GENRE: f64 = 8.0;
pub const MAX_CROSSFADE_MS: f64 = 24000.0;
/// BPM window of the sync rule: ±8 %.
pub const BPM_WINDOW: f64 = 0.08;
pub const TEMPO_RETURN_CAP_MS: f64 = 16000.0;
/// A list of this many tracks or more is not saved.
pub const MAX_SAVED: usize = 10000;
/// `AutoMixBeforeTriggerEndTimeMs`: where the outro starts in a track without phrases.
pub const TRIGGER_BEFORE_END_MS: i32 = 30000;
/// `kAutoMixCrossfadeRate` (no control in rekordbox).
pub const CROSSFADE_RATE: f64 = 1.0;
/// Every Automix timer of rekordbox runs at 100 ms.
pub const TICK_MS: u64 = 100;
/// Name of rekordbox's Automix playlist file, kept in recordbox's data folder.
pub const PLAYLIST_FILE: &str = "automixPlaylist6.xml";

/// `calculateLengthMs` 0x100b22fcc.
pub fn calculate_length_ms(bpm: f64, beats: f64) -> i32 {
    if bpm <= 0.0 || beats <= 0.0 {
        -1
    } else {
        (beats * 60000.0 / bpm) as i32
    }
}

/// `areBpmsNearEnough` 0x100b22f68: `a` within ±8 % of `b`.
pub fn bpms_near_enough(a: f64, b: f64) -> bool {
    a > 0.0 && b > 0.0 && b * (1.0 + BPM_WINDOW) >= a && b * (1.0 - BPM_WINDOW) <= a
}

/// A range of a track in ms; (-1, -1) is rekordbox's `INVALID_RANGE`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Range {
    pub start: i32,
    pub end: i32,
}

impl Range {
    pub const INVALID: Range = Range { start: -1, end: -1 };
    pub fn is_valid(&self) -> bool {
        *self != Range::INVALID
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
    pub fn len(&self) -> i32 {
        self.end - self.start
    }
}

impl Default for Range {
    fn default() -> Self {
        Range::INVALID
    }
}

/// `AutoMixTrackStructure`: where a track's intro and outro are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackStructure {
    /// The u16 at the head of the phrase analysis (its mood: 1 High, 2 Mid, 3 Low); -1 without
    /// phrase analysis. rekordbox calls it `genreNo`.
    pub genre: i32,
    pub intro: Range,
    pub outro: Range,
    pub total_ms: i32,
}

impl Default for TrackStructure {
    fn default() -> Self {
        TrackStructure { genre: -1, intro: Range::INVALID, outro: Range::INVALID, total_ms: 0 }
    }
}

impl TrackStructure {
    /// `AutoMixTrackStructure::init` 0x1012f7364. With phrases: the intro is the first
    /// phrase's beat (a 1 ms range) and the outro runs from the last phrase's beat to the end;
    /// one phrase alone gives no outro of its own. Without: no intro, and a "dummy" outro of
    /// the last `trigger_before_end_ms`.
    pub fn init(phrases: Option<&crate::phrase::Structure>, beats: &[Beat], total_ms: i32, trigger_before_end_ms: i32) -> TrackStructure {
        let beat_ms = |n: u16| -> i32 { beats.get((n as usize).wrapping_sub(1)).map(|b| (b.time * 1000.0) as i32).unwrap_or(-1) };
        let dummy = Range { start: (total_ms - trigger_before_end_ms).clamp(0, total_ms.max(0)), end: total_ms };
        let Some(s) = phrases.filter(|s| !s.phrases.is_empty()) else {
            return TrackStructure { genre: phrases.map(|s| s.mood as i32).unwrap_or(-1), intro: Range::INVALID, outro: dummy, total_ms };
        };
        let first = beat_ms(s.phrases[0].beat);
        let last = if s.phrases.len() >= 2 { beat_ms(s.phrases[s.phrases.len() - 1].beat) } else { -1 };
        let intro = if first >= 0 && first < total_ms { Range { start: first, end: first + 1 } } else { Range::INVALID };
        let outro = if last >= 0 && last < total_ms { Range { start: last, end: total_ms } } else { dummy };
        TrackStructure { genre: s.mood as i32, intro, outro, total_ms }
    }
}

/// `calculateCrossfadeLengthMs` 0x1023597e4: `incoming` is the deck just loaded, `outgoing`
/// the one playing; the BPMs are the tracks' own (0 % tempo).
pub fn crossfade_length_ms(in_bpm: f64, out_bpm: f64, incoming: &TrackStructure, outgoing: &TrackStructure, rate: f64) -> i32 {
    let (bpm, beats) = if in_bpm <= 0.0 || out_bpm <= 0.0 || incoming.genre < 0 || outgoing.genre < 0 || incoming.intro.is_empty() || outgoing.outro.is_empty() {
        (DEFAULT_BPM, BEATS_SAME_GENRE)
    } else {
        (if bpms_near_enough(in_bpm, out_bpm) { out_bpm } else { DEFAULT_BPM }, if incoming.genre == outgoing.genre { BEATS_SAME_GENRE } else { BEATS_OTHER_GENRE })
    };
    let len = ((rate * beats * 60000.0 / bpm) as i32).min(outgoing.outro.len()).min((rate * MAX_CROSSFADE_MS) as i32);
    len.max(0)
}

/// The Active deck's fade-out range (`AutoMixActivePlayerBehavior::updateAutoMixCrossfade
/// LengthChanged` 0x100cc4838): from the outro's start, the crossfade length stretched by
/// the deck's tempo, clamped to the track.
pub fn fade_out_range(s: &TrackStructure, len: i32, tempo_pct: f64, total_ms: i32) -> Range {
    if len < 0 || s.outro.is_empty() || s.outro.start < 0 || total_ms <= s.outro.start {
        return Range::INVALID;
    }
    let start = s.outro.start;
    let end = (start + (len as f32 + (tempo_pct as f32 * 0.01) * len as f32) as i32).min(total_ms).max(start);
    Range { start, end }
}

/// The Ready deck's fade-in range (0x1008f15f0): from the intro's start, or the current cue
/// when the track has no intro.
pub fn fade_in_range(s: &TrackStructure, len: i32, tempo_pct: f64, total_ms: i32, cue_ms: i32) -> Range {
    if len < 0 {
        return Range::INVALID;
    }
    let start = if s.intro.is_empty() { cue_ms } else { s.intro.start };
    if start < 0 || start >= total_ms {
        return Range::INVALID;
    }
    let end = (start + (len as f32 + (tempo_pct as f32 * 0.01) * len as f32) as i32).min(total_ms).max(start);
    Range { start, end }
}

/// `AutoMixTempoController::start` 0x1008fb048: how long the tempo takes to return to 0 %
/// (4 beats per 1 % of difference, at most 16 s; 8 s when the BPM is not known).
pub fn tempo_return_ms(plain_bpm: f64, output_bpm: f64) -> i32 {
    if plain_bpm <= 0.0 || output_bpm <= 0.0 {
        return 8000;
    }
    let beats = (plain_bpm - output_bpm).abs() * 400.0 / plain_bpm;
    (beats * 60000.0 / output_bpm).min(TEMPO_RETURN_CAP_MS) as i32
}

/// `AutoMixSmoother`: a linear ramp over wall-clock time, read every 100 ms.
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    pub from: f32,
    pub to: f32,
    pub start_ms: u64,
    pub len_ms: u32,
}

impl Ramp {
    /// rekordbox's step count: `round(len / 100)`.
    pub fn steps(&self) -> u32 {
        (self.len_ms as f32 / TICK_MS as f32).round() as u32
    }
    /// The value at `now` and whether the ramp has ended (it then stands on the target).
    pub fn at(&self, now_ms: u64) -> (f32, bool) {
        let el = now_ms.saturating_sub(self.start_ms);
        if self.steps() == 0 || el >= self.len_ms as u64 {
            return (self.to, true);
        }
        (self.from + (self.to - self.from) * (el as f32 / self.len_ms as f32), false)
    }
}

/// `RandomTable::init` 0x101a3b280: the rows shuffled (libc++ `std::shuffle`), then the row
/// played last in the table before is moved off the first place, then `first` (the row
/// Automix was started on) is swapped to the front.
pub fn shuffle_table<R: rand::Rng + ?Sized>(count: usize, first: Option<usize>, prev_last: Option<usize>, rng: &mut R) -> Vec<usize> {
    let mut t: Vec<usize> = (0..count).collect();
    if count >= 2 {
        for i in 0..count - 1 {
            let j = rng.gen_range(0..=(count - 1 - i));
            t.swap(i, i + j);
        }
        if prev_last == Some(t[0]) {
            t.swap(0, 1);
        }
        if let Some(f) = first.filter(|f| *f < count && t[0] != *f) {
            if let Some(k) = t.iter().position(|x| *x == f) {
                t.swap(0, k);
            }
        }
    }
    t
}

// ---------------------------------------------------------------- the playlist file

/// `AutoMixLoaderWrapper::writePlaylistToXml` 0x101a35a58: one PLAYLIST element per track in
/// list order. recordbox has one database, so both id pairs name the same track.
pub fn playlist_xml(ids: &[i64]) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n<AUTOMIX version=\"1\" browse_type=\"0\">\n");
    for id in ids {
        s.push_str(&format!("  <PLAYLIST contents_dbid=\"0\" contents_id=\"{id}\" master_dbid=\"0\" master_contents_id=\"{id}\"/>\n"));
    }
    s.push_str("</AUTOMIX>\n");
    s
}

/// The track ids of an Automix playlist file (`contents_id` of each PLAYLIST element).
pub fn parse_playlist_xml(xml: &str) -> Vec<i64> {
    let mut out = vec![];
    for el in xml.split('<').filter(|e| e.starts_with("PLAYLIST")) {
        if let Some(v) = crate::sampler_lib::attr(el, "contents_id").and_then(|v| v.parse::<i64>().ok()) {
            out.push(v);
        }
    }
    out
}

pub fn playlist_path() -> std::path::PathBuf {
    crate::db::data_dir().join(PLAYLIST_FILE)
}

// ---------------------------------------------------------------- the engine

/// What the engine needs from the two decks and the mixer.
pub trait Rig {
    /// The track whose audio is on the deck (None while empty or still loading).
    fn loaded_track(&self, d: usize) -> Option<i64>;
    fn is_loading(&self, d: usize) -> bool;
    fn playing(&self, d: usize) -> bool;
    /// The deck reaches the master output (channel fader up, crossfader not shut on its side).
    fn on_air(&self, d: usize) -> bool;
    fn elapsed_ms(&self, d: usize) -> i32;
    fn total_ms(&self, d: usize) -> i32;
    fn cue_ms(&self, d: usize) -> i32;
    /// The track's own BPM at the playhead, and the BPM heard.
    fn plain_bpm(&self, d: usize) -> f64;
    fn output_bpm(&self, d: usize) -> f64;
    fn tempo_pct(&self, d: usize) -> f64;
    /// Phrase analysis of the loaded track is known (it can arrive after the load).
    fn has_phrases(&self, d: usize) -> bool;
    fn structure(&self, d: usize) -> TrackStructure;
    fn playable(&self, track: i64) -> bool;
    fn load(&mut self, d: usize, track: i64) -> bool;
    fn unload(&mut self, d: usize);
    fn play(&mut self, d: usize);
    fn pause(&mut self, d: usize);
    fn play_from_ms(&mut self, d: usize, ms: i32);
    fn set_cue_ms(&mut self, d: usize, ms: i32);
    fn set_sync(&mut self, d: usize, on: bool);
    fn set_master(&mut self, d: usize);
    fn set_master_tempo(&mut self, d: usize, on: bool);
    fn set_tempo_pct(&mut self, d: usize, pct: f64);
    fn crossfader(&self) -> f32;
    fn set_crossfader(&mut self, v: f32);
    /// Remember the crossfader assigns and put deck 1 on side A, deck 2 on side B; and back.
    fn save_and_set_mixer(&mut self);
    fn restore_mixer(&mut self);
}

/// `AutoMixPlayer`'s behaviours.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum State {
    #[default]
    Disable,
    Reserved,
    Ready,
    Active,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerSt {
    pub state: State,
    /// The track Automix knows on the deck, and its row in the list.
    pub track: Option<i64>,
    pub row: Option<usize>,
    /// A load asked for and not yet finished: (track, row).
    pub loading: Option<(i64, usize)>,
    pub structure: TrackStructure,
    pub phrases_seen: bool,
    pub fade_in: Range,
    pub fade_out: Range,
    /// BPM sync when this deck comes in (`setSyncPolicy`).
    pub sync_policy: bool,
    /// The fade point watcher has fired for this track.
    pub fired: bool,
    /// The tempo return (`AutoMixTempoController`) and the tempo reset of a deck that came
    /// in without sync (`TempoReseter`: at most 10 tries).
    pub tempo_ramp: Option<Ramp>,
    pub reset_tries: Option<u8>,
    last_playing: bool,
    last_tempo: f64,
    /// Start playing when the load finishes (the list ended while this deck was loading).
    play_when_loaded: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Crossfade {
    pub incoming: usize,
    pub ramp: Ramp,
}

/// `AutoMixLoaderWrapper`: which row comes next.
#[derive(Default, Debug)]
pub struct Loader {
    pub current: Option<usize>,
    first: Option<usize>,
    table: Vec<usize>,
    table_pos: usize,
}

impl Loader {
    pub fn start(&mut self, row: Option<usize>) {
        *self = Loader { current: None, first: row, table: vec![], table_pos: 0 };
    }

    /// The next row (`getNextOrderRow` 0x101a3a364 / `getNextRandomRow` 0x101a3aafc): the row
    /// after the current one, or the next of the random table; with repeat the list starts
    /// again (a new table in random order), without it there is no next row at the end.
    pub fn next_row<R: rand::Rng + ?Sized>(&mut self, count: usize, opts: &AutomixPrefs, rng: &mut R) -> Option<usize> {
        if count == 0 {
            return None;
        }
        if opts.random {
            if self.table.len() != count {
                self.table = shuffle_table(count, self.first.take(), None, rng);
                self.table_pos = 0;
            }
            if self.table_pos >= self.table.len() {
                if !opts.repeat {
                    return None;
                }
                let prev = self.table.last().copied();
                self.table = shuffle_table(count, None, prev, rng);
                self.table_pos = 0;
            }
            let r = self.table[self.table_pos];
            self.table_pos += 1;
            self.current = Some(r);
            return Some(r);
        }
        let r = match (self.first.take(), self.current) {
            (Some(f), _) => f,
            (None, None) => 0,
            (None, Some(c)) => c + 1,
        };
        let r = if r >= count {
            if !opts.repeat {
                return None;
            }
            0
        } else {
            r
        };
        self.current = Some(r);
        Some(r)
    }
}

/// Why Automix did not start (rekordbox's alert texts, `checkStartableStateAndShowError`
/// 0x1018dab88).
pub const MSG_NO_FILES: &str = "No playable files.";
pub const MSG_ON_AIR: &str = "Unable to start Automix when deck 1/2 are on-air and playing.";
pub const MSG_ASSIGN: &str = "CROSSFADER Channel Assign has been changed. Check the mixer panel for details.";

#[derive(Default)]
pub struct Engine {
    pub processing: bool,
    pub players: [PlayerSt; 2],
    /// Decks in the order their loads were asked for (the first one becomes Active).
    queue: Vec<usize>,
    /// The deck Start was given: it loads first.
    first: usize,
    pub crossfade: Option<Crossfade>,
    /// The crossfade length worked out for the pair now on the decks; -1 = undefined.
    pub crossfade_len: i32,
    /// A deck the user loaded by hand: the crossfade starts as soon as it has loaded (timing 2).
    fade_on_loaded: Option<usize>,
    pub loader: Loader,
    next_tick: u64,
    /// Transitions completed since the start (for the scripted run and the tests).
    pub transitions: u32,
    /// What happened, for the log of the scripted run (not kept beyond 64 lines).
    pub log: Vec<String>,
    /// A fixed seed for the random order (tests); otherwise the thread's generator.
    pub seed: Option<u64>,
    draws: u64,
}

impl Engine {
    fn note(&mut self, s: String) {
        if self.log.len() >= 64 {
            self.log.remove(0);
        }
        self.log.push(s);
    }

    /// rekordbox's status word (`AutoMixPlayer::getStatus` 0x100248e84).
    pub fn status(&self, d: usize) -> u32 {
        let p = &self.players[d];
        let base = match p.state {
            State::Disable => 0,
            State::Reserved => 0x1100,
            State::Ready => 0x1200,
            State::Active => 0x1400,
        };
        base | if p.tempo_ramp.is_some() { 0x10 } else { 0 }
    }

    /// `AutoMixIF::getReadyPlayerID` 0x1018da250.
    pub fn ready_deck(&self) -> Option<usize> {
        (0..2).find(|d| self.players[*d].state == State::Ready)
    }

    /// `AutoMixLoadableStatus`: a deck can take a track when it is not both on air and
    /// playing, or stands at the end of its track.
    pub fn loadable(rig: &dyn Rig, d: usize) -> bool {
        !(rig.on_air(d) && rig.playing(d)) || Self::at_end(rig, d)
    }

    fn at_end(rig: &dyn Rig, d: usize) -> bool {
        rig.loaded_track(d).is_some() && rig.total_ms(d) > 0 && rig.elapsed_ms(d) >= rig.total_ms(d) - 30
    }

    /// The checks of Start, in rekordbox's order. (recordbox has no External Mixer mode and
    /// no Ableton Link, so those two questions never come up.)
    pub fn check_startable(rig: &dyn Rig, list: &[i64], deck: Option<usize>) -> Result<(), &'static str> {
        if !list.iter().any(|t| rig.playable(*t)) {
            return Err(MSG_NO_FILES);
        }
        if deck.is_none() && !Self::loadable(rig, 0) && !Self::loadable(rig, 1) {
            return Err(MSG_ON_AIR);
        }
        Ok(())
    }

    /// `AutoMixIF::start(row, deck)`: `row` None = from the top (or the random table's first),
    /// `deck` None = deck 1.
    pub fn start(&mut self, rig: &mut dyn Rig, list: &[i64], row: Option<usize>, deck: Option<usize>, now_ms: u64) -> Result<(), &'static str> {
        Self::check_startable(rig, list, deck)?;
        let first = deck.unwrap_or(0).min(1);
        let seed = self.seed;
        *self = Engine { processing: true, crossfade_len: -1, seed, next_tick: now_ms, first, ..Default::default() };
        self.loader.start(row);
        // `reserveAutoMix`: the chosen deck first, then the other.
        for d in [first, 1 - first] {
            self.players[d] = PlayerSt { state: State::Reserved, last_playing: rig.playing(d), last_tempo: rig.tempo_pct(d), ..Default::default() };
        }
        self.note(format!("start: row {row:?}, deck {}", first + 1));
        Ok(())
    }

    /// `AutoMixUnit::finishAutoMix` 0x100b393d0: the mixer settings come back, both players
    /// are Disable; the decks stay as they are.
    pub fn finish(&mut self, rig: &mut dyn Rig) {
        if !self.processing {
            return;
        }
        if let Some(cf) = self.crossfade.take() {
            // A ramp cut short leaves the crossfader where it is.
            let _ = cf;
        }
        rig.restore_mixer();
        for p in self.players.iter_mut() {
            *p = PlayerSt::default();
        }
        self.queue.clear();
        self.processing = false;
        self.fade_on_loaded = None;
        self.note("finish".into());
    }

    fn rng_pick(&mut self, count: usize, opts: &AutomixPrefs) -> Option<usize> {
        match self.seed {
            Some(s) => {
                use rand::SeedableRng;
                self.draws += 1;
                let mut r = rand::rngs::StdRng::seed_from_u64(s.wrapping_add(self.draws));
                self.loader.next_row(count, opts, &mut r)
            }
            None => self.loader.next_row(count, opts, &mut rand::thread_rng()),
        }
    }

    /// The next playable track: (row, id).
    fn next_track(&mut self, rig: &dyn Rig, list: &[i64], opts: &AutomixPrefs) -> Option<(usize, i64)> {
        for _ in 0..list.len().max(1) {
            let row = self.rng_pick(list.len(), opts)?;
            if rig.playable(list[row]) {
                return Some((row, list[row]));
            }
        }
        None
    }

    /// `AutoMixPlayerBehavior::loadTrack`: the next track onto deck `d`. False when the list
    /// has no next track (the caller then ends Automix).
    fn load_next(&mut self, rig: &mut dyn Rig, list: &mut Vec<i64>, opts: &AutomixPrefs, d: usize) -> bool {
        // `removePlayedTrack`: the row this deck has just played leaves the list first.
        if opts.remove_played {
            if let Some(r) = self.players[d].row.take().filter(|r| *r < list.len() && self.players[d].fired) {
                list.remove(r);
                let fix = |x: &mut Option<usize>| {
                    if let Some(v) = x.as_mut().filter(|v| **v > r) {
                        *v -= 1;
                    }
                };
                fix(&mut self.players[1 - d].row);
                match self.loader.current {
                    Some(c) if c > r => self.loader.current = Some(c - 1),
                    Some(c) if c == r => self.loader.current = c.checked_sub(1),
                    _ => {}
                }
                self.loader.table.clear();
            }
        }
        loop {
            let Some((row, id)) = self.next_track(rig, list, opts) else { return false };
            // (rekordbox: the other deck's sync off, this deck's sync off, this deck paused.)
            rig.set_sync(1 - d, false);
            rig.set_sync(d, false);
            rig.pause(d);
            if rig.load(d, id) {
                self.players[d].loading = Some((id, row));
                self.players[d].fired = false;
                self.note(format!("deck {}: load row {} (track {id})", d + 1, row + 1));
                return true;
            }
            // A track that will not load is skipped; when none will, the list is over.
            if !list.iter().any(|t| rig.playable(*t)) {
                return false;
            }
        }
    }

    /// `standbyPlayer` 0x1012f1f18: cue on the intro (when known), sync off, Master Tempo on,
    /// tempo 0 %.
    fn standby(&mut self, rig: &mut dyn Rig, d: usize) {
        let s = self.players[d].structure;
        if !s.intro.is_empty() && s.intro.start >= 0 {
            rig.set_cue_ms(d, s.intro.start);
        }
        rig.set_sync(d, false);
        rig.set_master_tempo(d, true);
        rig.set_tempo_pct(d, 0.0);
    }

    /// A track has finished loading on deck `d` (asked for by Automix, or put there by hand).
    fn on_loaded(&mut self, rig: &mut dyn Rig, d: usize, track: i64, row: Option<usize>) {
        let p = &mut self.players[d];
        p.track = Some(track);
        p.row = row;
        p.loading = None;
        p.structure = rig.structure(d);
        p.phrases_seen = rig.has_phrases(d);
        p.fired = false;
        p.tempo_ramp = None;
        p.reset_tries = None;
        let play = std::mem::take(&mut p.play_when_loaded);
        match p.state {
            State::Reserved => {
                self.standby(rig, d);
                self.players[d].state = State::Ready;
                rig.pause(d);
            }
            State::Ready => self.standby(rig, d),
            State::Active => rig.play(d),
            State::Disable => {}
        }
        if play {
            rig.play(d);
        }
        self.note(format!("deck {}: loaded track {track}, {:?}", d + 1, self.players[d].state));
        self.refresh_regions(rig);
        if self.fade_on_loaded == Some(d) {
            self.fade_on_loaded = None;
            self.fade_immediately(rig);
        }
    }

    /// The crossfade length and both decks' ranges for the pair now on the decks
    /// (`AutoMixCrossfader::onLoadedTrackChanged` 0x1023597b4, `updateAutoMixCrossfadeRegion`
    /// 0x100b3a9bc).
    fn refresh_regions(&mut self, rig: &dyn Rig) {
        let (out, inc) = match (self.players[0].state, self.players[1].state) {
            (State::Active, State::Ready) => (0, 1),
            (State::Ready, State::Active) => (1, 0),
            _ => {
                self.crossfade_len = -1;
                return;
            }
        };
        let ready = self.players[inc].track.is_some() && self.players[inc].loading.is_none();
        let len = if ready { crossfade_length_ms(rig.plain_bpm(inc), rig.plain_bpm(out), &self.players[inc].structure, &self.players[out].structure, CROSSFADE_RATE) } else { -1 };
        self.crossfade_len = len;
        self.players[inc].sync_policy = len >= 1 && bpms_near_enough(rig.plain_bpm(inc), rig.output_bpm(out));
        self.players[out].fade_in = Range::INVALID;
        self.players[out].fade_out = fade_out_range(&self.players[out].structure, len, rig.tempo_pct(out), rig.total_ms(out));
        self.players[inc].fade_out = Range::INVALID;
        self.players[inc].fade_in = fade_in_range(&self.players[inc].structure, len, rig.tempo_pct(inc), rig.total_ms(inc), rig.cue_ms(inc));
    }

    /// `AutoMixActivePlayerBehavior::fadeImmediately` 0x100cc4c14: the fade-out range moves
    /// to where the Active deck is now, with the length it had (manual crossfade start).
    pub fn fade_immediately(&mut self, rig: &dyn Rig) {
        if self.crossfade.is_some() {
            return;
        }
        let Some(a) = (0..2).find(|d| self.players[*d].state == State::Active) else { return };
        if self.players[1 - a].state != State::Ready || self.players[1 - a].loading.is_some() {
            return;
        }
        let old = self.players[a].fade_out;
        let len = if old.is_valid() { old.len() } else { self.crossfade_len.max(calculate_length_ms(DEFAULT_BPM, CROSSFADE_RATE * BEATS_SAME_GENRE)) };
        let (total, el) = (rig.total_ms(a), rig.elapsed_ms(a).max(0));
        let end = (el + (len as f32 + (rig.tempo_pct(a) as f32 * 0.01) * len as f32) as i32).min(total).max(el);
        let r = Range { start: el, end };
        // (`setDummyOutroRegionMs`: the track's outro is this range from now on.)
        self.players[a].structure.outro = r;
        self.players[a].fade_out = r;
        self.players[a].fired = false;
        self.note(format!("deck {}: fade now, range {}..{} ms", a + 1, r.start, r.end));
    }

    /// `AutoMixCrossfader::startCrossfade` 0x1023592b4.
    fn start_crossfade(&mut self, rig: &mut dyn Rig, incoming: usize, len: i32, now_ms: u64) {
        let len = if len < 0 { calculate_length_ms(DEFAULT_BPM, CROSSFADE_RATE * BEATS_SAME_GENRE) } else { len };
        let target = if incoming == 0 { 0.0 } else { 1.0 };
        let ramp = Ramp { from: rig.crossfader(), to: target, start_ms: now_ms, len_ms: len.max(0) as u32 };
        self.crossfade = Some(Crossfade { incoming, ramp });
        self.note(format!("crossfade to deck {} over {len} ms ({} steps), sync {}", incoming + 1, ramp.steps(), self.players[incoming].sync_policy));
        // `onAutoMixCrossfadeStarted`: the outgoing deck is the sync master; the incoming one
        // is synced when the tempos are near enough (else its tempo is reset), gets Master
        // Tempo and starts from its fade-in point.
        let out = 1 - incoming;
        if self.players[out].state == State::Active {
            rig.set_master(out);
        }
        if self.players[incoming].state == State::Ready {
            let policy = self.players[incoming].sync_policy;
            rig.set_sync(incoming, policy);
            self.players[incoming].reset_tries = if policy { None } else { Some(0) };
            rig.set_master_tempo(incoming, true);
            let fi = self.players[incoming].fade_in;
            if fi.is_valid() && fi.start >= 0 && fi.start < rig.total_ms(incoming) {
                rig.play_from_ms(incoming, fi.start);
            } else if len > 0 {
                rig.play(incoming);
            }
        }
        if ramp.steps() == 0 {
            self.step_crossfade(rig, now_ms);
        }
    }

    /// One 100 ms step of the ramp; at its end the crossfader stands on the target and the
    /// decks change roles. Returns whether a ramp ended.
    fn step_crossfade(&mut self, rig: &mut dyn Rig, now_ms: u64) -> Option<usize> {
        let cf = self.crossfade?;
        let (v, done) = cf.ramp.at(now_ms);
        rig.set_crossfader(v);
        if !done {
            return None;
        }
        self.crossfade = None;
        Some(cf.incoming)
    }

    /// `onAutoMixCrossfadeCompleted`: the outgoing deck (Active) loads the next track and is
    /// Ready again, or the list is over; the incoming deck (Ready) becomes Active.
    fn on_crossfade_completed(&mut self, rig: &mut dyn Rig, list: &mut Vec<i64>, opts: &AutomixPrefs, incoming: usize) {
        let out = 1 - incoming;
        let mut ended = false;
        if self.players[out].state == State::Active {
            self.transitions += 1;
            self.players[out].tempo_ramp = None;
            if self.load_next(rig, list, opts, out) {
                self.players[out].state = State::Ready;
                self.players[out].fade_in = Range::INVALID;
                self.players[out].fade_out = Range::INVALID;
            } else {
                ended = true;
            }
        }
        if ended {
            self.on_last_track_loaded(rig);
            return;
        }
        if self.players[incoming].state == State::Ready {
            self.enter_active(rig, incoming);
        }
        self.refresh_regions(rig);
    }

    /// `AutoMixActivePlayerBehavior::onEnter` 0x100cc4520: Master Tempo on, play, sync master,
    /// and the tempo starts its return to 0 %.
    fn enter_active(&mut self, rig: &mut dyn Rig, d: usize) {
        self.players[d].state = State::Active;
        self.players[d].reset_tries = None;
        rig.set_master_tempo(d, true);
        rig.play(d);
        // The deck keeps the speed it was synced to when it becomes the master.
        let pct = rig.tempo_pct(d);
        rig.set_master(d);
        rig.set_sync(d, false);
        rig.set_tempo_pct(d, pct);
        let t = tempo_return_ms(rig.plain_bpm(d), rig.output_bpm(d));
        if t < 1 || pct.abs() < 1e-6 {
            rig.set_tempo_pct(d, 0.0);
            self.players[d].tempo_ramp = None;
        } else {
            self.players[d].tempo_ramp = Some(Ramp { from: pct as f32, to: 0.0, start_ms: self.next_tick.saturating_sub(TICK_MS), len_ms: t as u32 });
        }
        self.players[d].last_tempo = rig.tempo_pct(d);
        self.note(format!("deck {}: Active (tempo {pct:+.2} %, return over {t} ms)", d + 1));
    }

    /// `onAutoMixLastTrackLoaded` 0x100b3a108: Ready decks play, the Active one is unloaded,
    /// and Automix ends.
    fn on_last_track_loaded(&mut self, rig: &mut dyn Rig) {
        self.note("the list has no next track".into());
        for d in 0..2 {
            match self.players[d].state {
                State::Ready | State::Disable => {
                    if self.players[d].loading.is_some() {
                        // (finish clears the player; the deck simply stays loaded.)
                    } else if self.players[d].track.is_some() {
                        rig.play(d);
                        // (The deck that plays on is left as the master, out of sync mode.)
                        if self.players[d].state == State::Ready && rig.playing(d) {
                            let pct = rig.tempo_pct(d);
                            rig.set_master(d);
                            rig.set_sync(d, false);
                            rig.set_tempo_pct(d, pct);
                        }
                    }
                }
                State::Active => rig.unload(d),
                State::Reserved => {}
            }
        }
        self.finish(rig);
    }

    /// Load row `row` onto `deck` while Automix runs (double-click in the list): the standby
    /// deck takes it, and with crossfade timing 2 the crossfade starts once it has loaded
    /// (`fadeImmediatelyOnLoaded` 0x100b395dc).
    pub fn load_row(&mut self, rig: &mut dyn Rig, list: &[i64], opts: &AutomixPrefs, row: usize, deck: usize) -> Result<(), &'static str> {
        let (d, Some(id)) = (deck.min(1), list.get(row).copied()) else { return Ok(()) };
        if self.players[d].state == State::Active && rig.playing(d) {
            return Err("Load Lock. Not available during playback.");
        }
        if !rig.load(d, id) {
            return Ok(());
        }
        self.players[d].loading = Some((id, row));
        self.loader.current = Some(row);
        if self.players[d].state == State::Disable {
            self.players[d].state = State::Reserved;
        }
        if opts.crossfade_timing == 2 && self.players[1 - d].state == State::Active {
            self.fade_on_loaded = Some(d);
        }
        Ok(())
    }

    /// Run the 100 ms timers. Call it every frame; it does nothing between ticks.
    pub fn tick(&mut self, rig: &mut dyn Rig, list: &mut Vec<i64>, opts: &AutomixPrefs, now_ms: u64) {
        if !self.processing || now_ms < self.next_tick {
            return;
        }
        self.next_tick = (self.next_tick + TICK_MS).max(now_ms.saturating_sub(TICK_MS) + 1);
        // 1. Loads that have finished, and tracks put on a deck by hand.
        for d in 0..2 {
            if rig.is_loading(d) {
                continue;
            }
            let on_deck = rig.loaded_track(d);
            match self.players[d].loading {
                Some((id, row)) if on_deck == Some(id) => self.on_loaded(rig, d, id, Some(row)),
                Some(_) => {
                    // The load failed: the next track instead, or the list is over.
                    self.players[d].loading = None;
                    if !self.load_next(rig, list, opts, d) {
                        self.on_last_track_loaded(rig);
                        return;
                    }
                }
                None => {
                    if let Some(id) = on_deck.filter(|id| self.players[d].state != State::Disable && self.players[d].state != State::Reserved && self.players[d].track != Some(*id)) {
                        let row = list.iter().position(|t| *t == id);
                        self.on_loaded(rig, d, id, row);
                    }
                }
            }
        }
        // 2. Reserved decks load as soon as they can take a track.
        for k in 0..2 {
            let d = if self.first == 1 { 1 - k } else { k };
            if self.players[d].state == State::Reserved && self.players[d].loading.is_none() && Self::loadable(rig, d) {
                if self.load_next(rig, list, opts, d) {
                    if !self.queue.contains(&d) {
                        self.queue.push(d);
                    }
                } else {
                    // One track for two decks: the deck that has it plays it and Automix ends.
                    let o = 1 - d;
                    if self.players[o].loading.is_some() {
                        self.players[o].play_when_loaded = true;
                        self.players[d].state = State::Disable;
                        continue;
                    }
                    self.on_last_track_loaded(rig);
                    return;
                }
            }
        }
        // (A single-track list: the lone deck has loaded; play it and end.)
        if let Some(d) = (0..2).find(|d| self.players[*d].state == State::Ready && self.players[1 - *d].state == State::Disable && self.players[*d].loading.is_none()) {
            if self.players[d].track.is_some() && self.crossfade.is_none() {
                rig.save_and_set_mixer();
                rig.set_crossfader(if d == 0 { 0.0 } else { 1.0 });
                rig.play(d);
                self.finish(rig);
                return;
            }
        }
        // 3. `activatePlayerIfNeeded`: both Ready, and one of them was loaded by the start.
        if !self.queue.is_empty() && self.players.iter().all(|p| p.state == State::Ready && p.loading.is_none()) {
            let first = self.queue[0];
            self.queue.clear();
            rig.save_and_set_mixer();
            self.note(format!("first activation: deck {}", first + 1));
            self.start_crossfade(rig, first, 0, now_ms);
            // (The first deck's tempo is not ramped: `setAutoTempoControlEnabled(false)`.)
            if self.crossfade.is_none() {
                self.on_crossfade_completed(rig, list, opts, first);
                rig.set_tempo_pct(first, 0.0);
                self.players[first].tempo_ramp = None;
            }
        }
        for d in 0..2 {
            // 4. Phrase analysis that arrived after the load, and tempo changes: new ranges.
            if self.players[d].track.is_some() && self.players[d].loading.is_none() {
                let (ph, tp) = (rig.has_phrases(d), rig.tempo_pct(d));
                let manual = self.players[d].structure.outro;
                if ph != self.players[d].phrases_seen && !self.players[d].fired {
                    self.players[d].phrases_seen = ph;
                    self.players[d].structure = rig.structure(d);
                    if self.players[d].state == State::Ready {
                        self.standby(rig, d);
                    }
                    let _ = manual;
                    self.refresh_regions(rig);
                } else if (tp - self.players[d].last_tempo).abs() > 1e-9 {
                    self.refresh_regions(rig);
                }
                self.players[d].last_tempo = tp;
            }
            // 5. Timing 3: the standby deck was started by hand while the other is Active.
            let playing = rig.playing(d);
            if playing && !self.players[d].last_playing && opts.crossfade_timing == 3 && self.players[d].state == State::Ready && self.players[1 - d].state == State::Active && self.crossfade.is_none() {
                self.fade_immediately(rig);
            }
            self.players[d].last_playing = playing;
        }
        // 6. The watcher of the Active deck: the fade point, or the end of the track.
        if self.crossfade.is_none() {
            if let Some(a) = (0..2).find(|d| self.players[*d].state == State::Active) {
                let other_ready = self.players[1 - a].state == State::Ready && self.players[1 - a].loading.is_none() && self.players[1 - a].track.is_some();
                let fo = self.players[a].fade_out;
                let el = rig.elapsed_ms(a);
                if !self.players[a].fired && other_ready && ((fo.is_valid() && el > fo.start) || Self::at_end(rig, a)) {
                    self.players[a].fired = true;
                    let len = if Self::at_end(rig, a) && !(fo.is_valid() && el > fo.start && el < fo.end) { 0 } else { self.crossfade_len };
                    self.start_crossfade(rig, 1 - a, len, now_ms);
                    if self.crossfade.is_none() {
                        self.on_crossfade_completed(rig, list, opts, 1 - a);
                    }
                }
            }
        } else if let Some(inc) = self.step_crossfade(rig, now_ms) {
            self.on_crossfade_completed(rig, list, opts, inc);
        }
        if !self.processing {
            return;
        }
        // 7. Tempo: the Active deck's return to 0 %, and the reset of a deck that came in
        // without sync (every 100 ms until it reads 0, at most 10 times).
        for d in 0..2 {
            if let Some(r) = self.players[d].tempo_ramp {
                let (v, done) = r.at(now_ms);
                rig.set_tempo_pct(d, if done { 0.0 } else { v as f64 });
                self.players[d].last_tempo = rig.tempo_pct(d);
                if done {
                    self.players[d].tempo_ramp = None;
                    self.refresh_regions(rig);
                }
            }
            if let Some(n) = self.players[d].reset_tries {
                rig.set_tempo_pct(d, 0.0);
                self.players[d].last_tempo = rig.tempo_pct(d);
                self.players[d].reset_tries = if rig.tempo_pct(d).abs() < 1e-9 || n >= 9 { None } else { Some(n + 1) };
            }
        }
    }
}

// ---------------------------------------------------------------- the app's side

/// A question the pane is asking (rekordbox's Yes / No / Cancel boxes).
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// "Clear current Automix playlist".
    Clear,
    /// "Assign a playlist": the playlist (node id) that would replace the list.
    Assign(i64),
}

#[derive(Default)]
pub struct AutomixState {
    pub engine: Engine,
    /// The Automix playlist: track ids in order.
    pub list: Vec<i64>,
    pub loaded: bool,
    /// The list changed: write the file at this time (rekordbox defers the save with a timer).
    pub save_at: Option<std::time::Instant>,
    pub selected: std::collections::BTreeSet<usize>,
    pub anchor: Option<usize>,
    pub settings_open: bool,
    pub ask: Option<Ask>,
    /// An alert with an OK button.
    pub alert: Option<String>,
    pub started: Option<std::time::Instant>,
    /// The crossfader assigns as they were before Automix set them.
    pub saved_assign: Option<[u8; 2]>,
    /// Where the pane's controls were drawn (for the scripted run).
    pub ui: crate::ui_automix::PaneMap,
}

/// The decks as the engine sees them.
struct AppRig<'a>(&'a mut App);

impl AppRig<'_> {
    fn state(&self, d: usize) -> parking_lot::MutexGuard<'_, crate::audio::PlayState> {
        self.0.player.decks[d.min(3)].lock()
    }
}

impl Rig for AppRig<'_> {
    fn loaded_track(&self, d: usize) -> Option<i64> {
        let dk = self.0.deck_of(d as u8);
        if dk.audio.is_some() && dk.loading.is_none() { dk.track } else { None }
    }
    fn is_loading(&self, d: usize) -> bool {
        self.0.deck_of(d as u8).loading.is_some()
    }
    fn playing(&self, d: usize) -> bool {
        self.state(d).playing
    }
    fn on_air(&self, d: usize) -> bool {
        let m = self.0.px.mix[d];
        let (a, b) = crate::audio::crossfader_gains(self.0.px.crossfader);
        let x = match m.assign {
            1 => a,
            2 => b,
            _ => 1.0,
        };
        m.fader > 0.0 && x > 0.0
    }
    fn elapsed_ms(&self, d: usize) -> i32 {
        let s = self.state(d);
        s.audio.as_ref().map(|a| (s.pos / a.sr as f64 * 1000.0) as i32).unwrap_or(0)
    }
    fn total_ms(&self, d: usize) -> i32 {
        (self.0.deck_of(d as u8).duration * 1000.0) as i32
    }
    fn cue_ms(&self, d: usize) -> i32 {
        (self.0.deck_of(d as u8).cue_point * 1000.0) as i32
    }
    fn plain_bpm(&self, d: usize) -> f64 {
        let dk = self.0.deck_of(d as u8);
        if dk.beats.is_empty() {
            dk.track.and_then(|id| self.0.track(id)).map(|t| t.bpm).unwrap_or(0.0)
        } else {
            crate::player_ext::bpm_at(&dk.beats, self.elapsed_ms(d) as f64 / 1000.0)
        }
    }
    fn output_bpm(&self, d: usize) -> f64 {
        self.plain_bpm(d) * self.state(d).tempo
    }
    fn tempo_pct(&self, d: usize) -> f64 {
        (self.state(d).tempo - 1.0) * 100.0
    }
    fn has_phrases(&self, d: usize) -> bool {
        self.0.deck_of(d as u8).phrases.is_some()
    }
    fn structure(&self, d: usize) -> TrackStructure {
        let dk = self.0.deck_of(d as u8);
        TrackStructure::init(dk.phrases.as_ref(), &dk.beats, (dk.duration * 1000.0) as i32, TRIGGER_BEFORE_END_MS)
    }
    fn playable(&self, track: i64) -> bool {
        self.0.track(track).is_some_and(|t| !t.missing)
    }
    fn load(&mut self, d: usize, track: i64) -> bool {
        self.0.with_deck(d as u8, |a| {
            a.load_track_here(track);
            a.deck.track == Some(track) && (a.deck.loading.is_some() || a.deck.audio.is_some())
        })
    }
    fn unload(&mut self, d: usize) {
        self.0.with_deck(d as u8, |a| a.unload_track());
    }
    fn play(&mut self, d: usize) {
        self.0.with_deck(d as u8, |a| a.set_playing(true));
    }
    fn pause(&mut self, d: usize) {
        self.0.with_deck(d as u8, |a| a.set_playing(false));
    }
    fn play_from_ms(&mut self, d: usize, ms: i32) {
        self.0.with_deck(d as u8, |a| {
            a.seek(ms as f64 / 1000.0);
            a.set_playing(true);
            a.push_deck_controls();
            a.sync_align();
        });
    }
    fn set_cue_ms(&mut self, d: usize, ms: i32) {
        self.0.with_deck(d as u8, |a| {
            a.deck.cue_point = ms as f64 / 1000.0;
            if !a.playing() {
                a.seek(ms as f64 / 1000.0);
            }
        });
    }
    fn set_sync(&mut self, d: usize, on: bool) {
        // A follower that leaves sync keeps the speed it was synced to, as on a player.
        if !on && self.0.deck_of(d as u8).x.beat_sync && self.0.px.master != d as u8 {
            let sp = self.state(d).tempo;
            self.0.deck_of_mut(d as u8).x.tempo_adj = sp - 1.0;
        }
        self.0.deck_of_mut(d as u8).x.beat_sync = on;
        self.0.push_deck_controls();
    }
    fn set_master(&mut self, d: usize) {
        self.0.px.master = d as u8;
        self.0.px.sampler.master = false;
        self.0.push_deck_controls();
    }
    fn set_master_tempo(&mut self, d: usize, on: bool) {
        self.0.deck_of_mut(d as u8).x.master_tempo = on;
        self.0.push_deck_controls();
    }
    fn set_tempo_pct(&mut self, d: usize, pct: f64) {
        self.0.deck_of_mut(d as u8).x.tempo_adj = pct / 100.0;
        self.0.push_deck_controls();
    }
    fn crossfader(&self) -> f32 {
        self.0.px.crossfader
    }
    fn set_crossfader(&mut self, v: f32) {
        self.0.px.crossfader = v.clamp(0.0, 1.0);
        self.0.push_deck_controls();
    }
    fn save_and_set_mixer(&mut self) {
        let was = [self.0.px.mix[0].assign, self.0.px.mix[1].assign];
        if self.0.mixx.automix.saved_assign.is_none() {
            self.0.mixx.automix.saved_assign = Some(was);
        }
        if was != [1, 2] {
            self.0.px.mix[0].assign = 1;
            self.0.px.mix[1].assign = 2;
            self.0.toast(MSG_ASSIGN);
        }
        self.0.push_deck_controls();
    }
    fn restore_mixer(&mut self) {
        if let Some(a) = self.0.mixx.automix.saved_assign.take() {
            self.0.px.mix[0].assign = a[0];
            self.0.px.mix[1].assign = a[1];
            self.0.push_deck_controls();
        }
    }
}

/// Once per frame: Automix's timers, MIX POINT LINK's watch on the decks, the sampler
/// library's background work, and this area's dialogs.
pub fn poll(app: &mut App, ctx: &eframe::egui::Context) {
    app.automix_poll();
    app.mpl_poll();
    app.smplib_poll();
    crate::ui_automix::windows(app, ctx);
    crate::ui_sampler_lib::windows(app, ctx);
    crate::ui_sampler_lib::drag_frame(app, ctx);
}

/// While Automix crossfades (`AutoMixCrossfader::startCrossfade` 0x1023592b4 to
/// `completeCrossfade` 0x102359f7c), rekordbox disables the on-screen crossfader
/// (`CrossFaderComp::setAutomixCrossfaderEnabled` 0x1000d9cd8: the slider's
/// `setEnabled(false)`) and sets the crossfader curve to value 0 (the smoothest of the 33:
/// the two sides blend over the whole travel); at the end of the crossfade, or when Automix
/// stops (`finishAutoMix` 0x102359598), the crossfader is enabled again and the curve set in
/// Preferences comes back. No message goes with it (none exists in the binary).
pub fn crossfader_locked(app: &App) -> bool {
    app.mixx.automix.engine.processing && app.mixx.automix.engine.crossfade.is_some()
}

/// The crossfader curve Automix holds while it crossfades (see `crossfader_locked`).
pub fn curve_override(app: &App) -> Option<u8> {
    if crossfader_locked(app) { Some(0) } else { None }
}

/// While Automix brings a deck's tempo back to 0 % (`AutoMixTempoController::start`
/// 0x1008fb048 to its last `timerCallback` step), rekordbox dims that deck's tempo controls
/// to 30 % opacity (`AutoMixPlayerWrapper::setManualTempoSliderEnabled(false)` 0x100107c40:
/// `setAlpha(0.3)` on two of `VirtualDeckPanel`'s components); they are not disabled and no
/// message is shown. Decks 1 and 2 only (Automix uses no others).
pub fn tempo_dimmed(app: &App, deck: u8) -> bool {
    let e = &app.mixx.automix.engine;
    e.processing && (deck as usize) < 2 && e.players[deck as usize].tempo_ramp.is_some()
}

impl App {
    /// Milliseconds since Automix was first used (the engine's clock).
    fn automix_now(&mut self) -> u64 {
        let t0 = *self.mixx.automix.started.get_or_insert_with(std::time::Instant::now);
        t0.elapsed().as_millis() as u64
    }

    pub fn automix_processing(&self) -> bool {
        self.mixx.automix.engine.processing
    }

    /// Read the list from the file the first time it is needed.
    pub fn automix_ensure_loaded(&mut self) {
        if self.mixx.automix.loaded {
            return;
        }
        self.mixx.automix.loaded = true;
        if let Ok(s) = std::fs::read_to_string(playlist_path()) {
            self.mixx.automix.list = parse_playlist_xml(&s).into_iter().filter(|t| self.index.contains_key(t)).collect();
        }
    }

    /// The list changed: forget the selection's stale rows and save a moment later.
    pub fn automix_changed(&mut self) {
        let n = self.mixx.automix.list.len();
        self.mixx.automix.selected.retain(|r| *r < n);
        self.mixx.automix.save_at = Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
    }

    fn automix_save_now(&mut self) {
        self.mixx.automix.save_at = None;
        if self.mixx.automix.list.len() < MAX_SAVED {
            let _ = std::fs::write(playlist_path(), playlist_xml(&self.mixx.automix.list));
        }
    }

    /// Start or stop (`ListViewer::startStopAutomix` 0x1003f3d1c).
    pub fn automix_start_stop(&mut self) {
        if self.automix_processing() {
            self.automix_finish();
            return;
        }
        self.automix_ensure_loaded();
        if !self.performance() {
            return;
        }
        self.prefs.mixx.automix.pane = true;
        self.automix_start(None, None);
    }

    pub fn automix_start(&mut self, row: Option<usize>, deck: Option<usize>) {
        if self.deck_options.disable_load || self.deck_options.disable_play {
            self.toast_err("Automix requires deck loading and playback. Check the --disable-load and --disable-play startup options.");
            return;
        }
        self.automix_ensure_loaded();
        let now = self.automix_now();
        let list = self.mixx.automix.list.clone();
        let mut e = std::mem::take(&mut self.mixx.automix.engine);
        let deck = deck.or_else(|| e.ready_deck());
        let r = e.start(&mut AppRig(self), &list, row, deck, now);
        self.mixx.automix.engine = e;
        match r {
            Ok(()) => {
                // A set MIX POINT LINK ends when Automix starts.
                crate::mpl::finish_for(self, "MIX POINT LINK setting has been disabled.");
            }
            Err(m) => self.mixx.automix.alert = Some(m.to_string()),
        }
    }

    pub fn automix_finish(&mut self) {
        let mut e = std::mem::take(&mut self.mixx.automix.engine);
        e.finish(&mut AppRig(self));
        self.mixx.automix.engine = e;
    }

    /// Double-click (when allowed) or the menu: load a row to deck 1 or 2.
    pub fn automix_load_row(&mut self, row: usize, deck: usize) {
        let Some(id) = self.mixx.automix.list.get(row).copied() else { return };
        if !self.automix_processing() {
            self.with_deck(deck.min(1) as u8, |a| a.load_track_here(id));
            return;
        }
        let list = self.mixx.automix.list.clone();
        let opts = self.prefs.mixx.automix.clone();
        let mut e = std::mem::take(&mut self.mixx.automix.engine);
        let r = e.load_row(&mut AppRig(self), &list, &opts, row, deck);
        self.mixx.automix.engine = e;
        if let Err(m) = r {
            self.toast_err(m);
        }
    }

    /// Once per frame.
    pub fn automix_poll(&mut self) {
        if self.mixx.automix.save_at.is_some_and(|t| std::time::Instant::now() >= t) {
            self.automix_save_now();
        }
        if !self.automix_processing() {
            return;
        }
        // Automix belongs to PERFORMANCE mode and its first two decks.
        if !self.performance() {
            self.automix_finish();
            return;
        }
        let now = self.automix_now();
        let opts = self.prefs.mixx.automix.clone();
        let mut list = std::mem::take(&mut self.mixx.automix.list);
        let before = list.len();
        let mut e = std::mem::take(&mut self.mixx.automix.engine);
        e.tick(&mut AppRig(self), &mut list, &opts, now);
        self.mixx.automix.engine = e;
        let changed = list.len() != before;
        self.mixx.automix.list = list;
        if changed {
            self.automix_changed();
        }
        self.ctx.request_repaint_after(std::time::Duration::from_millis(TICK_MS));
    }

    /// Put tracks at the end of the Automix playlist ("Add tracks to Automix playlist").
    pub fn automix_add(&mut self, ids: &[i64]) {
        self.automix_ensure_loaded();
        self.mixx.automix.list.extend(ids.iter().copied().filter(|t| self.index.contains_key(t)));
        self.automix_changed();
    }

    /// Replace the list with a playlist's tracks ("Select as Automix playlist").
    pub fn automix_assign(&mut self, node: i64) {
        self.automix_ensure_loaded();
        let ids = self.view_tracks_of(&View::Playlist(node));
        if self.automix_processing() {
            self.automix_finish();
        }
        self.mixx.automix.list = ids;
        self.mixx.automix.selected.clear();
        self.prefs.mixx.automix.pane = true;
        self.automix_changed();
        self.save_prefs();
    }

    /// "Save Automix playlist": a normal playlist named "Untitled Automix playlist".
    pub fn automix_save_as_playlist(&mut self) -> Option<i64> {
        if self.mixx.automix.list.is_empty() {
            return None;
        }
        let mut name = "Untitled Automix playlist".to_string();
        let mut n = 1;
        while self.nodes.iter().any(|x| x.parent == 0 && x.name == name) {
            n += 1;
            name = format!("Untitled Automix playlist ({n})");
        }
        let id = self.db.create_node(0, &name, NodeKind::Playlist, None).ok()?;
        let _ = self.db.set_playlist_tracks(id, &self.mixx.automix.list);
        self.reload_playlists();
        Some(id)
    }

    pub fn automix_clear(&mut self) {
        if self.automix_processing() {
            self.automix_finish();
        }
        self.mixx.automix.list.clear();
        self.mixx.automix.selected.clear();
        self.automix_changed();
        // (rekordbox deletes the file with the list.)
        let _ = std::fs::remove_file(playlist_path());
        self.mixx.automix.save_at = None;
    }

    /// The fade-in and fade-out ranges Automix has on deck `d` (seconds), for the waveforms.
    pub fn automix_ranges(&self, d: usize) -> [Option<(f64, f64)>; 2] {
        let e = &self.mixx.automix.engine;
        if !e.processing || d > 1 {
            return [None, None];
        }
        let f = |r: Range| if r.is_valid() && r.end > r.start { Some((r.start as f64 / 1000.0, r.end as f64 / 1000.0)) } else { None };
        [f(e.players[d].fade_in), f(e.players[d].fade_out)]
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn beats(bpm: f64, secs: f64) -> Vec<Beat> {
        let n = (secs * bpm / 60.0) as usize;
        (0..n).map(|i| Beat { time: i as f64 * 60.0 / bpm, bpm, beat: (i % 4) as u8 + 1, index: i }).collect()
    }

    fn phrases(mood: u16, starts: &[u16]) -> crate::phrase::Structure {
        crate::phrase::Structure { mood, phrases: starts.iter().map(|b| crate::phrase::Phrase { beat: *b, kind: 1 }).collect(), end_beat: starts.last().copied().unwrap_or(1) + 32, rb: None }
    }

    #[test]
    fn length_and_bpm_rule() {
        assert_eq!(calculate_length_ms(120.0, 16.0), 8000);
        assert_eq!(calculate_length_ms(128.0, 16.0), 7500);
        assert_eq!(calculate_length_ms(0.0, 16.0), -1);
        assert_eq!(calculate_length_ms(120.0, 0.0), -1);
        // ±8 % of the second BPM, both ends included.
        assert!(bpms_near_enough(108.0, 100.0));
        assert!(bpms_near_enough(92.0, 100.0));
        assert!(!bpms_near_enough(108.1, 100.0));
        assert!(!bpms_near_enough(91.9, 100.0));
        assert!(!bpms_near_enough(0.0, 100.0));
        assert!(!bpms_near_enough(100.0, 0.0));
    }

    #[test]
    fn intro_and_outro_from_phrases() {
        let b = beats(120.0, 300.0);
        // Phrases at beats 1, 33, 513: intro at beat 1 (0 ms), outro from beat 513 (256 s).
        let s = TrackStructure::init(Some(&phrases(1, &[1, 33, 513])), &b, 300_000, 30000);
        assert_eq!(s.genre, 1);
        assert_eq!(s.intro, Range { start: 0, end: 1 });
        assert_eq!(s.outro, Range { start: 256_000, end: 300_000 });
        // A first phrase that does not start on beat 1.
        let s = TrackStructure::init(Some(&phrases(2, &[17, 513])), &b, 300_000, 30000);
        assert_eq!(s.intro, Range { start: 8000, end: 8001 });
        // One phrase only: the intro is known, the outro is the last 30 s.
        let s = TrackStructure::init(Some(&phrases(3, &[1])), &b, 300_000, 30000);
        assert_eq!((s.intro.start, s.outro), (0, Range { start: 270_000, end: 300_000 }));
        // No phrases: no intro, the last 30 s (or the whole of a shorter track).
        let s = TrackStructure::init(None, &b, 300_000, 30000);
        assert_eq!((s.genre, s.intro, s.outro), (-1, Range::INVALID, Range { start: 270_000, end: 300_000 }));
        let s = TrackStructure::init(None, &[], 20_000, 30000);
        assert_eq!(s.outro, Range { start: 0, end: 20_000 });
    }

    #[test]
    fn crossfade_length_formula() {
        let b = beats(128.0, 300.0);
        let a = TrackStructure::init(Some(&phrases(1, &[1, 33, 513])), &b, 300_000, 30000);
        let same = a;
        let other = TrackStructure { genre: 2, ..a };
        let none = TrackStructure::init(None, &b, 300_000, 30000);
        // Same genre, BPMs near: 16 beats at the outgoing BPM.
        assert_eq!(crossfade_length_ms(126.0, 128.0, &same, &a, 1.0), 7500);
        // Other genre: 8 beats.
        assert_eq!(crossfade_length_ms(126.0, 128.0, &other, &a, 1.0), 3750);
        // BPMs too far apart: counted at 120.
        assert_eq!(crossfade_length_ms(100.0, 128.0, &same, &a, 1.0), 8000);
        assert_eq!(crossfade_length_ms(100.0, 128.0, &other, &a, 1.0), 4000);
        // Something unknown (no BPM, no phrases on either side): 16 beats at 120.
        assert_eq!(crossfade_length_ms(0.0, 128.0, &same, &a, 1.0), 8000);
        assert_eq!(crossfade_length_ms(126.0, 128.0, &none, &a, 1.0), 8000);
        assert_eq!(crossfade_length_ms(126.0, 128.0, &same, &none, 1.0), 8000);
        // Never longer than the outro, nor than 24 s.
        let short = TrackStructure { outro: Range { start: 297_000, end: 300_000 }, ..a };
        assert_eq!(crossfade_length_ms(126.0, 128.0, &same, &short, 1.0), 3000);
        assert_eq!(crossfade_length_ms(30.0, 30.0, &same, &a, 1.0), 24000);
        // The rate scales the beats and the cap.
        assert_eq!(crossfade_length_ms(126.0, 128.0, &same, &a, 2.0), 15000);
    }

    #[test]
    fn fade_ranges_follow_the_tempo() {
        let b = beats(120.0, 300.0);
        let s = TrackStructure::init(Some(&phrases(1, &[17, 513])), &b, 300_000, 30000);
        assert_eq!(fade_out_range(&s, 8000, 0.0, 300_000), Range { start: 256_000, end: 264_000 });
        assert_eq!(fade_out_range(&s, 8000, 5.0, 300_000), Range { start: 256_000, end: 264_400 });
        assert_eq!(fade_out_range(&s, 60_000, 0.0, 300_000), Range { start: 256_000, end: 300_000 });
        assert_eq!(fade_out_range(&s, -1, 0.0, 300_000), Range::INVALID);
        assert_eq!(fade_in_range(&s, 8000, 0.0, 300_000, 1234), Range { start: 8000, end: 16000 });
        // Without an intro the current cue is where the track comes in.
        let n = TrackStructure::init(None, &b, 300_000, 30000);
        assert_eq!(fade_in_range(&n, 8000, -10.0, 300_000, 1234), Range { start: 1234, end: 8434 });
    }

    #[test]
    fn ramp_is_linear_in_100_ms_steps() {
        let r = Ramp { from: 0.0, to: 1.0, start_ms: 1000, len_ms: 8000 };
        assert_eq!(r.steps(), 80);
        assert_eq!(r.at(1000), (0.0, false));
        assert!((r.at(1100).0 - 0.0125).abs() < 1e-6);
        assert!((r.at(5000).0 - 0.5).abs() < 1e-6);
        assert_eq!(r.at(9000), (1.0, true));
        // Every step is the same size.
        let v: Vec<f32> = (0..=80).map(|k| r.at(1000 + 100 * k).0).collect();
        assert!(v.windows(2).all(|w| ((w[1] - w[0]) - 0.0125).abs() < 1e-5));
        // Shorter than half a step: no ramp, the target at once.
        assert_eq!(Ramp { from: 0.3, to: 0.0, start_ms: 0, len_ms: 40 }.at(0), (0.0, true));
        assert_eq!(Ramp { from: 1.0, to: 0.0, start_ms: 0, len_ms: 0 }.at(0), (0.0, true));
    }

    #[test]
    fn tempo_return_time() {
        // 4 beats per 1 %: 126 heard on a 120 track = 5 % = 20 beats at 126 = 9523 ms.
        assert_eq!(tempo_return_ms(120.0, 126.0), 9523);
        assert_eq!(tempo_return_ms(120.0, 120.0), 0);
        assert_eq!(tempo_return_ms(120.0, 129.6), 14814);
        assert_eq!(tempo_return_ms(120.0, 140.0), 16000);
        assert_eq!(tempo_return_ms(0.0, 126.0), 8000);
    }

    #[test]
    fn random_table() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        for _ in 0..50 {
            let t = shuffle_table(8, Some(5), Some(2), &mut rng);
            let mut s = t.clone();
            s.sort();
            assert_eq!(s, (0..8).collect::<Vec<_>>(), "every row once");
            assert_eq!(t[0], 5, "the row Automix was started on comes first");
        }
        for _ in 0..50 {
            let t = shuffle_table(8, None, Some(2), &mut rng);
            assert_ne!(t[0], 2, "the row just played does not open the next round");
        }
        assert_eq!(shuffle_table(1, None, Some(0), &mut rng), vec![0]);
        assert!(shuffle_table(0, None, None, &mut rng).is_empty());
    }

    #[test]
    fn loader_order_repeat_and_random() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let mut o = AutomixPrefs { repeat: false, ..Default::default() };
        let mut l = Loader::default();
        l.start(None);
        assert_eq!((0..4).map(|_| l.next_row(3, &o, &mut rng)).collect::<Vec<_>>(), vec![Some(0), Some(1), Some(2), None]);
        o.repeat = true;
        l.start(Some(1));
        assert_eq!((0..5).map(|_| l.next_row(3, &o, &mut rng).unwrap()).collect::<Vec<_>>(), vec![1, 2, 0, 1, 2]);
        o.random = true;
        o.repeat = false;
        l.start(Some(2));
        let got: Vec<_> = (0..5).map(|_| l.next_row(4, &o, &mut rng)).collect();
        assert_eq!(got[0], Some(2));
        assert_eq!(got[4], None);
        let mut seen: Vec<usize> = got[..4].iter().map(|x| x.unwrap()).collect();
        seen.sort();
        assert_eq!(seen, vec![0, 1, 2, 3]);
        o.repeat = true;
        l.start(None);
        let got: Vec<usize> = (0..8).map(|_| l.next_row(4, &o, &mut rng).unwrap()).collect();
        assert_ne!(got[3], got[4], "a new round does not start with the row just played");
    }

    #[test]
    fn playlist_file_round_trip() {
        let ids = vec![12, 7, 7, 3000000001, 5];
        let xml = playlist_xml(&ids);
        assert!(xml.contains("<AUTOMIX version=\"1\" browse_type=\"0\">"));
        assert_eq!(xml.matches("<PLAYLIST ").count(), 5);
        assert_eq!(parse_playlist_xml(&xml), ids);
        // rekordbox's own file reads too (other attribute order, self-closed or not).
        let rb = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<AUTOMIX version=\"1\" browse_type=\"1\"><PLAYLIST master_dbid=\"x\" contents_id=\"44\" contents_dbid=\"abc\" master_contents_id=\"9\"></PLAYLIST><PLAYLIST contents_dbid=\"abc\" contents_id=\"45\"/></AUTOMIX>";
        assert_eq!(parse_playlist_xml(rb), vec![44, 45]);
        assert!(parse_playlist_xml("").is_empty());
    }

    /// Two simulated decks: positions move with the clock, loads take 300 ms.
    pub struct Sim {
        pub now: u64,
        pub track: [Option<i64>; 2],
        pub loading: [Option<(i64, u64)>; 2],
        pub playing: [bool; 2],
        pub pos: [f64; 2],
        pub cue: [i32; 2],
        pub tempo: [f64; 2],
        pub sync: [bool; 2],
        pub master: usize,
        pub mt: [bool; 2],
        pub xf: f32,
        pub assign: [u8; 2],
        pub saved: Option<[u8; 2]>,
        /// (bpm, length ms, phrase starts or none, mood) per track id.
        pub lib: std::collections::HashMap<i64, (f64, i32, Option<Vec<u16>>, u16)>,
        pub xf_writes: Vec<(u64, f32)>,
        pub events: Vec<String>,
    }

    impl Sim {
        pub fn new() -> Sim {
            Sim { now: 0, track: [None; 2], loading: [None; 2], playing: [false; 2], pos: [0.0; 2], cue: [0; 2], tempo: [0.0; 2], sync: [false; 2], master: 0, mt: [false; 2], xf: 0.5, assign: [1, 2], saved: None, lib: Default::default(), xf_writes: vec![], events: vec![] }
        }
        pub fn advance(&mut self, ms: u64) {
            self.now += ms;
            for d in 0..2 {
                if let Some((id, at)) = self.loading[d] {
                    if self.now >= at {
                        self.loading[d] = None;
                        self.track[d] = Some(id);
                        self.pos[d] = 0.0;
                    }
                }
                if self.playing[d] && self.track[d].is_some() {
                    let total = self.total_ms(d) as f64;
                    self.pos[d] = (self.pos[d] + ms as f64 * (1.0 + self.rate(d) / 100.0)).min(total);
                    if self.pos[d] >= total {
                        self.playing[d] = false;
                    }
                }
            }
        }
        /// The tempo heard in percent: a synced follower runs at the master's BPM.
        fn rate(&self, d: usize) -> f64 {
            if self.sync[d] && self.master != d && self.track[self.master].is_some() && self.track[d].is_some() {
                let m = self.bpm(self.master) * (1.0 + self.tempo[self.master] / 100.0);
                (m / self.bpm(d) - 1.0) * 100.0
            } else {
                self.tempo[d]
            }
        }
        fn bpm(&self, d: usize) -> f64 {
            self.track[d].and_then(|t| self.lib.get(&t)).map(|x| x.0).unwrap_or(0.0)
        }
    }

    impl Rig for Sim {
        fn loaded_track(&self, d: usize) -> Option<i64> {
            if self.loading[d].is_some() { None } else { self.track[d] }
        }
        fn is_loading(&self, d: usize) -> bool {
            self.loading[d].is_some()
        }
        fn playing(&self, d: usize) -> bool {
            self.playing[d]
        }
        fn on_air(&self, d: usize) -> bool {
            if d == 0 { self.xf < 1.0 } else { self.xf > 0.0 }
        }
        fn elapsed_ms(&self, d: usize) -> i32 {
            self.pos[d] as i32
        }
        fn total_ms(&self, d: usize) -> i32 {
            self.track[d].and_then(|t| self.lib.get(&t)).map(|x| x.1).unwrap_or(0)
        }
        fn cue_ms(&self, d: usize) -> i32 {
            self.cue[d]
        }
        fn plain_bpm(&self, d: usize) -> f64 {
            self.bpm(d)
        }
        fn output_bpm(&self, d: usize) -> f64 {
            self.bpm(d) * (1.0 + self.rate(d) / 100.0)
        }
        fn tempo_pct(&self, d: usize) -> f64 {
            self.rate(d)
        }
        fn has_phrases(&self, d: usize) -> bool {
            self.track[d].and_then(|t| self.lib.get(&t)).is_some_and(|x| x.2.is_some())
        }
        fn structure(&self, d: usize) -> TrackStructure {
            let Some(x) = self.track[d].and_then(|t| self.lib.get(&t)) else { return TrackStructure::default() };
            let b = beats(x.0, x.1 as f64 / 1000.0);
            let ph = x.2.as_ref().map(|s| phrases(x.3, s));
            TrackStructure::init(ph.as_ref(), &b, x.1, TRIGGER_BEFORE_END_MS)
        }
        fn playable(&self, track: i64) -> bool {
            self.lib.contains_key(&track)
        }
        fn load(&mut self, d: usize, track: i64) -> bool {
            self.track[d] = None;
            self.playing[d] = false;
            self.loading[d] = Some((track, self.now + 300));
            self.events.push(format!("{} load {} deck {}", self.now, track, d + 1));
            true
        }
        fn unload(&mut self, d: usize) {
            self.track[d] = None;
            self.playing[d] = false;
            self.events.push(format!("{} unload deck {}", self.now, d + 1));
        }
        fn play(&mut self, d: usize) {
            if self.track[d].is_some() && !self.playing[d] {
                self.playing[d] = true;
                self.events.push(format!("{} play deck {} at {}", self.now, d + 1, self.pos[d] as i32));
            }
        }
        fn pause(&mut self, d: usize) {
            self.playing[d] = false;
        }
        fn play_from_ms(&mut self, d: usize, ms: i32) {
            self.pos[d] = ms as f64;
            self.playing[d] = false;
            self.play(d);
        }
        fn set_cue_ms(&mut self, d: usize, ms: i32) {
            self.cue[d] = ms;
            if !self.playing[d] {
                self.pos[d] = ms as f64;
            }
        }
        fn set_sync(&mut self, d: usize, on: bool) {
            if !on && self.sync[d] {
                self.tempo[d] = self.rate(d);
            }
            self.sync[d] = on;
        }
        fn set_master(&mut self, d: usize) {
            self.master = d;
        }
        fn set_master_tempo(&mut self, d: usize, on: bool) {
            self.mt[d] = on;
        }
        fn set_tempo_pct(&mut self, d: usize, pct: f64) {
            self.tempo[d] = pct;
        }
        fn crossfader(&self) -> f32 {
            self.xf
        }
        fn set_crossfader(&mut self, v: f32) {
            self.xf = v;
            self.xf_writes.push((self.now, v));
        }
        fn save_and_set_mixer(&mut self) {
            if self.saved.is_none() {
                self.saved = Some(self.assign);
            }
            self.assign = [1, 2];
        }
        fn restore_mixer(&mut self) {
            if let Some(a) = self.saved.take() {
                self.assign = a;
            }
        }
    }

    fn run(e: &mut Engine, sim: &mut Sim, list: &mut Vec<i64>, o: &AutomixPrefs, ms: u64) {
        let mut t = 0;
        while t < ms && e.processing {
            sim.advance(20);
            let now = sim.now;
            e.tick(sim, list, o, now);
            t += 20;
        }
    }

    fn three_tracks() -> Sim {
        let mut s = Sim::new();
        // 1: 120 BPM, 60 s, phrases from beat 1, last phrase at beat 89 (44 s).
        s.lib.insert(1, (120.0, 60_000, Some(vec![1, 33, 89]), 1));
        // 2: 126 BPM (within 8 %), same mood, intro at beat 9, outro at beat 97.
        s.lib.insert(2, (126.0, 60_000, Some(vec![9, 41, 97]), 1));
        // 3: 90 BPM (too far for sync), no phrases.
        s.lib.insert(3, (90.0, 40_000, None, 0));
        s
    }

    #[test]
    fn states_from_start_through_two_transitions_to_the_end() {
        let mut sim = three_tracks();
        let mut list = vec![1, 2, 3];
        let o = AutomixPrefs { repeat: false, ..Default::default() };
        let mut e = Engine::default();
        sim.assign = [0, 0];
        e.start(&mut sim, &list, None, None, 0).unwrap();
        assert_eq!((e.players[0].state, e.players[1].state), (State::Reserved, State::Reserved));
        assert_eq!((e.status(0), e.status(1)), (0x1100, 0x1100));
        // Both decks load (rows 1 and 2) and are Ready; then deck 1 is Active.
        run(&mut e, &mut sim, &mut list, &o, 300);
        assert_eq!((sim.loading[0].map(|x| x.0), sim.loading[1].map(|x| x.0)), (Some(1), Some(2)));
        assert_eq!(e.players[0].state, State::Reserved, "Reserved until the track has loaded");
        run(&mut e, &mut sim, &mut list, &o, 600);
        assert_eq!((e.players[0].state, e.players[1].state), (State::Active, State::Ready));
        assert_eq!((e.status(0), e.status(1)), (0x1400, 0x1200));
        assert_eq!(sim.xf, 0.0, "the crossfader snapped to deck 1");
        assert_eq!(sim.assign, [1, 2], "the crossfader assigns were set");
        assert!(sim.playing[0] && !sim.playing[1]);
        assert!(sim.mt[0] && sim.mt[1], "Master Tempo on");
        assert_eq!(sim.cue[1], 3809, "deck 2 waits on its intro (beat 9 at 126 BPM)");
        assert_eq!(sim.pos[1] as i32, 3809);
        // The pair's crossfade: same mood, BPMs near: 16 beats at 120 = 8000 ms.
        assert_eq!(e.crossfade_len, 8000);
        assert_eq!(e.players[0].fade_out, Range { start: 44_000, end: 52_000 });
        assert_eq!(e.players[1].fade_in, Range { start: 3809, end: 11_809 });
        assert!(e.players[1].sync_policy);
        // Until the fade point nothing moves.
        run(&mut e, &mut sim, &mut list, &o, 42_000);
        assert!(e.crossfade.is_none() && !sim.playing[1]);
        let writes_before = sim.xf_writes.len();
        // The fade point: deck 2 starts on its intro, synced; the ramp runs 8 s in 100 ms steps.
        for _ in 0..40 {
            if e.crossfade.is_some() {
                break;
            }
            run(&mut e, &mut sim, &mut list, &o, 100);
        }
        let cf = e.crossfade.expect("the crossfade has started");
        assert!(sim.pos[0] >= 44_000.0 && sim.pos[0] < 44_000.0 + 220.0, "fired within a tick of the fade point: {}", sim.pos[0]);
        assert_eq!((cf.incoming, cf.ramp.len_ms, cf.ramp.steps()), (1, 8000, 80));
        assert!(sim.playing[1] && sim.sync[1] && sim.master == 0);
        assert!((sim.rate(1) - (120.0 / 126.0 - 1.0) * 100.0).abs() < 1e-9, "deck 2 runs at deck 1's BPM");
        run(&mut e, &mut sim, &mut list, &o, 4000);
        assert!(sim.xf > 0.35 && sim.xf < 0.75, "half way: {}", sim.xf);
        run(&mut e, &mut sim, &mut list, &o, 4500);
        assert!(e.crossfade.is_none());
        assert_eq!(sim.xf, 1.0);
        // The ramp was written every 100 ms, rising evenly.
        let w = &sim.xf_writes[writes_before..];
        assert!(w.len() >= 80 && w.len() <= 82, "{} writes", w.len());
        assert!(w.windows(2).all(|p| p[1].0 - p[0].0 == 100 && p[1].1 >= p[0].1 && p[1].1 - p[0].1 < 0.02));
        // Roles: deck 2 Active and master, its tempo returning to 0; deck 1 loads row 3.
        assert_eq!(e.transitions, 1);
        assert_eq!(e.players[1].state, State::Active);
        assert_eq!(sim.master, 1);
        assert!(!sim.sync[1]);
        assert!(e.status(1) & 0x10 != 0, "the tempo controller runs: {:?} tempo {:?}", e.log, sim.tempo);
        let t0 = sim.tempo[1];
        assert!(t0 < -4.0 && t0 > -4.9, "tempo kept from the sync: {t0}");
        run(&mut e, &mut sim, &mut list, &o, 1000);
        assert_eq!(e.players[0].state, State::Ready);
        assert_eq!(sim.track[0], Some(3));
        assert!(sim.tempo[1] > t0 && sim.tempo[1] < 0.0, "ramping back");
        // 4.76 % at 4 beats per 1 % = 19 beats at 120 = 9.5 s.
        run(&mut e, &mut sim, &mut list, &o, 10_000);
        assert_eq!(sim.tempo[1], 0.0);
        assert_eq!(e.status(1), 0x1400);
        // Second pair: track 3 has no phrases, so 16 beats at 120 and no sync (90 vs 126).
        assert_eq!(e.crossfade_len, 8000);
        assert!(!e.players[0].sync_policy);
        let fo = e.players[1].fade_out;
        assert_eq!(fo.start, 45_714, "beat 97 at 126 BPM");
        run(&mut e, &mut sim, &mut list, &o, 60_000);
        // After the second transition the list is over: deck 2 unloaded, deck 1 plays on.
        assert_eq!(e.transitions, 2);
        assert!(!e.processing);
        assert_eq!((e.players[0].state, e.players[1].state), (State::Disable, State::Disable));
        assert_eq!(sim.track, [Some(3), None]);
        assert!(sim.playing[0]);
        assert_eq!(sim.xf, 0.0);
        assert_eq!(sim.tempo[0], 0.0, "no sync: the incoming tempo was reset");
        assert_eq!(sim.assign, [0, 0], "the crossfader assigns came back");
    }

    #[test]
    fn repeat_wraps_and_stop_leaves_the_decks_playing() {
        let mut sim = three_tracks();
        let mut list = vec![1, 2];
        let o = AutomixPrefs::default();
        let mut e = Engine::default();
        e.start(&mut sim, &list, None, None, 0).unwrap();
        run(&mut e, &mut sim, &mut list, &o, 60_000);
        assert_eq!(e.transitions, 1);
        assert_eq!(sim.track[0], Some(1), "with repeat the list starts again");
        assert!(e.processing);
        e.finish(&mut sim);
        assert!(!e.processing && sim.playing[1]);
        assert_eq!(e.status(1), 0);
    }

    #[test]
    fn start_is_refused_with_rekordboxs_messages() {
        let mut sim = three_tracks();
        let mut e = Engine::default();
        assert_eq!(e.start(&mut sim, &[], None, None, 0), Err(MSG_NO_FILES));
        assert_eq!(e.start(&mut sim, &[99], None, None, 0), Err(MSG_NO_FILES));
        // Both decks on air and playing.
        sim.track = [Some(1), Some(2)];
        sim.playing = [true, true];
        sim.xf = 0.5;
        assert_eq!(e.start(&mut sim, &[1, 2], None, None, 0), Err(MSG_ON_AIR));
        assert!(!e.processing);
        // One of them off air: fine. It waits (Reserved) while the other still plays on air.
        sim.xf = 0.0;
        assert!(e.start(&mut sim, &[1, 2, 3], None, None, 0).is_ok());
        let mut list = vec![1, 2, 3];
        let o = AutomixPrefs::default();
        run(&mut e, &mut sim, &mut list, &o, 2000);
        assert_eq!((e.players[0].state, e.players[1].state), (State::Reserved, State::Ready), "deck 1 is on air and playing: it is not loaded over");
        assert!(sim.playing[0]);
    }

    #[test]
    fn manual_crossfade_start_by_timing() {
        // Timing 3: starting the standby deck by hand starts the crossfade at once.
        let mut sim = three_tracks();
        let mut list = vec![1, 2, 3];
        let o = AutomixPrefs { crossfade_timing: 3, ..Default::default() };
        let mut e = Engine::default();
        e.start(&mut sim, &list, None, None, 0).unwrap();
        run(&mut e, &mut sim, &mut list, &o, 5000);
        assert_eq!(e.players[0].state, State::Active);
        sim.playing[1] = true;
        run(&mut e, &mut sim, &mut list, &o, 400);
        assert!(e.crossfade.is_some(), "the crossfade starts when deck 2 is started");
        // Timing 1: nothing happens.
        let mut sim = three_tracks();
        let o1 = AutomixPrefs { crossfade_timing: 1, ..Default::default() };
        let mut e = Engine::default();
        e.start(&mut sim, &list, None, None, 0).unwrap();
        run(&mut e, &mut sim, &mut list, &o1, 5000);
        sim.playing[1] = true;
        run(&mut e, &mut sim, &mut list, &o1, 400);
        assert!(e.crossfade.is_none());
        // Timing 2: a row loaded to the standby deck by hand fades in once it has loaded.
        let mut sim = three_tracks();
        let o2 = AutomixPrefs::default();
        let mut e = Engine::default();
        e.start(&mut sim, &list, None, None, 0).unwrap();
        run(&mut e, &mut sim, &mut list, &o2, 5000);
        e.load_row(&mut sim, &list, &o2, 2, 1).unwrap();
        run(&mut e, &mut sim, &mut list, &o2, 200);
        assert!(e.crossfade.is_none(), "not before the load has finished");
        run(&mut e, &mut sim, &mut list, &o2, 600);
        assert_eq!(e.crossfade.map(|c| c.incoming), Some(1));
        assert_eq!(sim.track[1], Some(3));
    }
}
