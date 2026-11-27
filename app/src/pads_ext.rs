//! What the pad modes do to a deck: the state each deck keeps for its pads and the actions
//! behind them (BEAT JUMP, BEAT LOOP, KEYBOARD / KEY SHIFT, SLICER with its capture, ACT.
//! CENSR with its regions in the database, USER pads). The rules are in `pads.rs`, the audio
//! thread's part in `pad_audio.rs`. Every method here acts on the active deck; the pad host
//! runs inside `App::with_deck`.

use crate::app::*;
use crate::model::*;
use crate::pad_audio::{CensorSpan, SlicerAudio};
use crate::pads::*;
use std::sync::Arc;

/// The slicer of one deck, as the interface keeps it.
#[derive(Default)]
pub struct SlicerUi {
    pub active: bool,
    /// SLICER LOOP (rekordbox's mode 2) instead of SLICER (3).
    pub loop_mode: bool,
    /// Beat position where the slicer was switched on, and where the domain starts now.
    pub anchor: f64,
    pub start: f64,
    /// The domain's boundaries in seconds.
    pub b: [f64; 9],
    /// Pads held down, newest last.
    pub held: Vec<u8>,
    pub track: Option<i64>,
    /// The track's beat times, and what they were made from (count, first, last).
    pub grid: Arc<Vec<f64>>,
    sig: (usize, u64, u64),
}

/// ACT. CENSR of one deck.
#[derive(Default)]
pub struct CensorUi {
    /// The track the regions belong to.
    pub track: Option<i64>,
    pub on: bool,
    /// Effect type 0..3.
    pub kind: u8,
    pub regions: Vec<CensorRegion>,
    /// IN was pressed: the in point waiting for OUT.
    pub pending_in: Option<f64>,
    /// IN / OUT adjust (1 / 2) of region `adjust_i`: its edge follows the playhead.
    pub adjust: u8,
    pub adjust_i: usize,
}

/// What a held USER pad lets go of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UserHeld {
    Cue,
    Bend,
    PadFx(u8),
}

/// What one deck keeps for its pads while the app runs.
pub struct DeckPadState {
    /// Where each pad mode's list is scrolled to (0 = left end, 1 = right end), by rekordbox's
    /// mode value; None until the mode is first shown.
    pub scroll: [Option<f32>; 21],
    pub slicer: SlicerUi,
    pub censor: CensorUi,
    /// KEYBOARD: the hot cue the pads play (the key cue), and the edit view.
    pub key_cue: Option<i32>,
    pub key_edit: bool,
    /// BEAT JUMP: the logical pad that is down (its arrow is lit).
    pub jump_down: Option<u8>,
    /// A sampler pad of a USER mode is down.
    pub user_sampler: Option<usize>,
    /// A USER pad that acts while held: a transport item (CUE, PITCH BEND) or a PAD FX pad
    /// (its index in the deck's PAD FX pads).
    pub user_held: Option<UserHeld>,
}

impl Default for DeckPadState {
    fn default() -> DeckPadState {
        DeckPadState { scroll: [None; 21], slicer: Default::default(), censor: Default::default(), key_cue: None, key_edit: false, jump_down: None, user_sampler: None, user_held: None }
    }
}

/// The pads' state (a field of `App`).
pub struct PadsState {
    pub deck: [DeckPadState; 4],
    /// The Pad Editor window: open, the USER mode showing, the pad being edited.
    pub editor_open: bool,
    pub editor_user: usize,
    /// Where each deck's pad panel was last drawn (for the scripted run), and in which layout.
    pub rects: [eframe::egui::Rect; 4],
    pub c4: bool,
    /// The censor's tables exist in the database.
    tables: bool,
    /// USER templates read from rekordbox's installation.
    templates: Option<Vec<Vec<PadItem>>>,
}

impl Default for PadsState {
    fn default() -> PadsState {
        PadsState { deck: Default::default(), editor_open: false, editor_user: 0, rects: [eframe::egui::Rect::NOTHING; 4], c4: false, tables: false, templates: None }
    }
}

fn encode_beats(b: &[(u16, u16); 4]) -> String {
    b.iter().map(|x| format!("{}/{}", x.0, x.1)).collect::<Vec<_>>().join(",")
}

fn decode_beats(s: &str) -> [(u16, u16); 4] {
    let mut out = CENSOR_DEFAULT_BEATS;
    for (i, part) in s.split(',').take(4).enumerate() {
        if let Some((n, d)) = part.split_once('/') {
            if let (Ok(n), Ok(d)) = (n.trim().parse::<u16>(), d.trim().parse::<u16>()) {
                if n > 0 && d > 0 {
                    out[i] = (n, d);
                }
            }
        }
    }
    out
}

/// rekordbox's status messages for the slicer (7.2.19 `kSlicerOperateError`,
/// `kSlipOperateErrorSlipOff`).
pub const SLIP_OFF_BY_SLICER: &str = "Slip off. Slip can not be turned on when Slicer is on.";

impl App {
    fn pd(&mut self) -> &mut DeckPadState {
        let n = (self.px.active as usize).min(3);
        &mut self.pads.deck[n]
    }
    fn pdr(&self) -> &DeckPadState {
        &self.pads.deck[(self.px.active as usize).min(3)]
    }

    /// The active deck's pad mode.
    pub fn pad_mode(&self) -> PadMode {
        let m = PadMode::from_code(self.px.pad_mode[(self.px.active as usize).min(3)]);
        if m == PadMode::Stems && !self.prefs.show_stems {
            PadMode::HotCue
        } else {
            m
        }
    }

    pub fn pad_select_ctx(&self) -> SelectCtx {
        SelectCtx { external: false, pc_mix: false, loaded: self.deck.track.is_some(), has_grid: self.deck.beats.len() >= 2, pos: self.pos() }
    }

    /// Choose a pad mode for the active deck (the pull-down, a keyboard shortcut). Refused
    /// with rekordbox's message when `canSelectPadMode` says no.
    pub fn set_pad_mode(&mut self, mode: PadMode) -> bool {
        match can_select(mode, self.pad_select_ctx()) {
            Ok(()) => {}
            Err(msg) => {
                if let Some(m) = msg {
                    self.toast_err(m);
                }
                return false;
            }
        }
        let n = (self.px.active as usize).min(3);
        self.px.pad_mode[n] = mode.code();
        self.pads_tick();
        true
    }

    /// The pads show PAD Display (2 x 4 pads) instead of LIST Display. No controller is
    /// connected to recordbox, so Auto is the list.
    pub fn pads_pad_layout(&self) -> bool {
        pad_layout(self.prefs.pads.layout, 0)
    }

    /// Once per frame and deck: the slicer follows the pad mode (it is on while its view
    /// shows and the track has a grid), and the censor follows the loaded track.
    pub fn pads_tick(&mut self) {
        let mode = self.pad_mode();
        let want = mode == PadMode::Slicer && self.deck.track.is_some() && self.deck.beats.len() > SLICER_UNITS + 1;
        let (active, track) = (self.pdr().slicer.active, self.pdr().slicer.track);
        if active && (!want || track != self.deck.track) {
            self.slicer_deactivate();
        }
        if want && !self.pdr().slicer.active {
            self.slicer_activate();
        }
        if self.pdr().slicer.active {
            self.slicer_follow();
        }
        if self.pdr().censor.track != self.deck.track {
            self.censor_load();
        }
        self.censor_follow();
    }

    // ------------------------------------------------------------------ BEAT JUMP / BEAT LOOP

    /// A BEAT JUMP pad (logical id 0..23). FINE moves 5 ms; with a loop running the loop
    /// moves with the position (`BeatSearchBehavior::doBeatSearch`).
    pub fn pad_beat_jump(&mut self, id: u8) {
        if self.deck.audio.is_none() {
            return;
        }
        let sign = if jump_forward(id) { 1.0 } else { -1.0 };
        let p = self.pos();
        let dur = self.deck.duration.max(0.0);
        let target = |from: f64, app: &App| -> f64 {
            match jump_beats(id) {
                None => from + sign * JUMP_FINE_SECS,
                Some(b) if b.fract() == 0.0 => grid_jump(&app.deck.beats, from, sign * b).unwrap_or(from + sign * b * app.beat_len()),
                Some(b) => {
                    // A part of a beat: that part of the beat interval the position is in.
                    let g = &app.deck.beats;
                    let i = g.partition_point(|x| x.time <= from).saturating_sub(1);
                    let len = if g.len() >= 2 { g[(i + 1).min(g.len() - 1)].time - g[i.min(g.len() - 2)].time } else { app.beat_len() };
                    from + sign * b * if len > 0.0 { len } else { app.beat_len() }
                }
            }
        };
        match self.loop_active() {
            Some((a, b)) => {
                let na = target(a, self).clamp(0.0, (dur - (b - a)).max(0.0));
                let d = na - a;
                {
                    let mut s = self.player.state.lock();
                    s.loop_range = Some((na, na + (b - a)));
                }
                self.deck.last_loop = Some((na, na + (b - a)));
                self.seek((p + d).clamp(0.0, dur));
            }
            None => {
                let t = target(p, self).clamp(0.0, dur);
                self.seek(t);
            }
        }
        if !self.playing() {
            self.deck.cue_point = self.pos();
        }
    }

    /// A BEAT LOOP pad (0..15): the loop of that length; the pad of the running loop ends it.
    pub fn pad_beat_loop(&mut self, i: usize) {
        if self.deck.audio.is_none() {
            return;
        }
        // (rekordbox refuses nothing here while the slicer is on: `kSlicerOperateError` is
        // used by SLIP alone; a loop only pauses the slicer's rolls, `isRollableState`.)
        let b = crate::player_ext::LOOP_SIZES[i.min(15)];
        if self.beat_loop_lit() == Some(i) {
            self.loop_exit();
        } else {
            self.prefs.loop_beats = b;
            self.auto_loop(b);
        }
    }

    /// The pad lit for the loop that runs.
    pub fn beat_loop_lit(&self) -> Option<usize> {
        let (a, b) = self.loop_active()?;
        loop_pad_for((b - a) / self.beat_len().max(1e-9))
    }

    // ------------------------------------------------------------------ KEYBOARD / KEY SHIFT

    /// The key shift the pads compare with. (rekordbox reads the playing key's shift; without
    /// Master Tempo it takes the semitones the tempo itself makes. recordbox's key shift is
    /// independent of the tempo, so its own value is used in both cases.)
    pub fn pad_current_key(&self) -> i32 {
        self.deck.x.key_shift
    }

    /// A KEY SHIFT pad at table position `pos` (0..39).
    pub fn pad_key(&mut self, pos: usize) {
        if self.deck.track.is_none() {
            return;
        }
        match key_press(key_at_in(pos, self.key_scale1()), self.pad_current_key(), self.deck.x.master_tempo) {
            KeyAct::Set(n) => {
                self.deck.x.key_sync = false;
                self.deck.x.key_shift = n.clamp(-12, 12);
                self.push_deck_controls();
            }
            KeyAct::Sync => self.toggle_key_sync(),
            KeyAct::Refuse(Some(m)) => self.toast_err(m),
            KeyAct::Refuse(None) | KeyAct::Nothing => {}
        }
    }

    /// A KEYBOARD pad: the key cue plays in the pad's key (the function pads act as in KEY
    /// SHIFT). Without a key cue nothing sounds.
    pub fn pad_keyboard_down(&mut self, pos: usize) {
        if self.deck.track.is_none() {
            return;
        }
        let key = key_at_in(pos, self.key_scale1());
        self.pad_key(pos);
        if key_semis(key).is_none() {
            return;
        }
        let Some(cue) = self.pdr().key_cue else { return };
        if self.deck.cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == cue) {
            self.hot_cue_down(cue);
        } else {
            self.pd().key_cue = None;
        }
    }

    pub fn pad_keyboard_up(&mut self) {
        self.hot_cue_up();
    }

    /// The KEYBOARD edit view: a pad with a hot cue makes it the key cue; an empty pad sets
    /// the hot cue (`setKeyCue` 0x101467360, `onHotCuePadPressed` 0x101468dc4).
    pub fn pad_key_cue(&mut self, slot: i32) {
        if self.deck.track.is_none() {
            return;
        }
        if self.deck.cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == slot) {
            self.pd().key_cue = Some(slot);
        } else {
            self.hot_cue_down(slot);
            self.hot_cue_up();
        }
    }

    /// KEYBOARD and KEY SHIFT use `KeyPositioner`'s scale 1 while a DDJ-GRV6 or XDJ-AN is
    /// connected (`KeyboardView::updateScale`); otherwise scale 0.
    pub fn key_scale1(&self) -> bool {
        self.midi.devices.iter().any(|d| key_scale1_device(d))
    }

    // ------------------------------------------------------------------ SLICER

    /// SLIP on a deck whose slicer is on is refused (`WidgetPlayerDJ::operateEnableSlip`).
    /// Returns true when it was refused. Loop controls are not refused while the slicer is on
    /// (no check in 7.2.19; a running loop only pauses the slicer's rolls).
    pub fn slip_refused_by_slicer(&mut self) -> bool {
        if self.pdr().slicer.active {
            self.toast_err(MSG_SLICER_ON);
            return true;
        }
        false
    }

    fn slicer_grid(&mut self) -> Arc<Vec<f64>> {
        let g = &self.deck.beats;
        let sig = (g.len(), g.first().map(|b| b.time.to_bits()).unwrap_or(0), g.last().map(|b| b.time.to_bits()).unwrap_or(0));
        if self.pdr().slicer.sig != sig {
            let v: Vec<f64> = self.deck.beats.iter().map(|b| b.time).collect();
            let s = &mut self.pd().slicer;
            s.grid = Arc::new(v);
            s.sig = sig;
        }
        self.pdr().slicer.grid.clone()
    }

    fn slicer_unit(&self) -> f64 {
        slicer_unit_beats(self.prefs.pads.slicer_length[(self.px.active as usize).min(3)] as usize)
    }

    fn slicer_roll(&self) -> f64 {
        slicer_roll_beats(self.prefs.pads.slicer_roll[(self.px.active as usize).min(3)] as usize)
    }

    /// Put the domain at beat `start` and tell the audio thread.
    fn slicer_set_domain(&mut self, start: f64) {
        let grid = self.slicer_grid();
        let unit = self.slicer_unit();
        let (start, b) = slicer_domain(&grid, start, unit, self.deck.duration);
        let loop_mode = {
            let s = &mut self.pd().slicer;
            s.start = start;
            s.b = b;
            s.loop_mode
        };
        let mut st = self.player.state.lock();
        let sr = st.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
        let sl = &mut st.pads.slicer;
        sl.on = true;
        sl.loop_mode = loop_mode;
        sl.b = b.map(|t| t * sr);
    }

    /// `SlicerBehavior::doActivate` (0x102b51768): the hot cue held while paused stops, the
    /// loop ends, SLIP goes off, and the domain starts at the beat playing now.
    fn slicer_activate(&mut self) {
        self.hot_cue_up();
        self.loop_exit();
        if self.deck.x.slip {
            // (`MemoryCuePanel::setSlicerTabVisible` says so when SLIP was on.)
            self.toast_err(SLIP_OFF_BY_SLICER);
            self.deck.x.slip = false;
            self.push_deck_controls();
        }
        let grid = self.slicer_grid();
        let start = time_beat(&grid, self.pos()).floor().max(0.0);
        {
            let track = self.deck.track;
            let s = &mut self.pd().slicer;
            s.active = true;
            s.anchor = start;
            s.held.clear();
            s.track = track;
        }
        self.player.state.lock().pads.slicer = SlicerAudio::default();
        self.slicer_set_domain(start);
        let st = self.pdr().slicer.start;
        self.pd().slicer.anchor = st;
    }

    fn slicer_deactivate(&mut self) {
        {
            let mut st = self.player.state.lock();
            // (A roll that is sounding ends where the track would have been.)
            if let Some(bg) = st.pads.slicer.bg {
                if st.pads.slicer.roll.is_some() {
                    st.pos = bg;
                }
            }
            st.pads.slicer = SlicerAudio::default();
        }
        let s = &mut self.pd().slicer;
        s.active = false;
        s.held.clear();
    }

    /// SLICER (not SLICER LOOP): once playback has left the domain, and no section is held or
    /// rolling, the domain moves on in whole domains from the anchor.
    fn slicer_follow(&mut self) {
        let (rolling, sr, pos) = {
            let st = self.player.state.lock();
            let sr = st.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
            (st.pads.slicer.rolling() || st.pads.slicer.bg.is_some(), sr, st.pos / sr)
        };
        let _ = sr;
        let s = &self.pdr().slicer;
        if s.loop_mode || rolling || !s.held.is_empty() {
            return;
        }
        if pos >= s.b[0] && pos < s.b[8] {
            return;
        }
        let grid = s.grid.clone();
        if pos >= grid.last().copied().unwrap_or(0.0) {
            return;
        }
        let beat = time_beat(&grid, pos);
        let start = slicer_advance(s.anchor, beat, self.slicer_unit() * SLICER_UNITS as f64);
        self.slicer_set_domain(start);
    }

    /// A slicer pad (0..7) pressed or let go (`SlicerView::padPressed`, `doSelectUnit`).
    pub fn slicer_pad(&mut self, pad: u8, down: bool) {
        if !self.pdr().slicer.active || pad >= SLICER_UNITS as u8 {
            return;
        }
        let Some(sel) = slicer_stack(&mut self.pd().slicer.held, pad, down) else { return };
        let quantize = self.prefs.quantize;
        let (grid, start, unit, roll) = (self.slicer_grid(), self.pdr().slicer.start, self.slicer_unit(), self.slicer_roll());
        let dur = self.deck.duration;
        let mut st = self.player.state.lock();
        let Some(sr) = st.audio.as_ref().map(|a| a.sr as f64) else { return };
        let pos = st.pos;
        let playing = st.playing;
        let sl = &mut st.pads.slicer;
        match sel {
            Some(k) => {
                let from = start + k as f64 * unit;
                let a = beat_time(&grid, from) * sr;
                let b = (beat_time(&grid, from + roll).min(if dur > 0.0 { dur } else { f64::MAX }) * sr).max(a + 2.0);
                if sl.bg.is_none() {
                    sl.bg = Some(pos);
                }
                sl.ret_at = None;
                if quantize && playing {
                    if a <= pos && pos + 512.0 < b {
                        // Already inside the section's roll: it holds from here.
                        sl.roll = Some((a, b));
                        sl.pending = None;
                        sl.starts = sl.starts.wrapping_add(1);
                    } else {
                        // At the next step of the roll's length.
                        let beat = time_beat(&grid, pos / sr);
                        let next = start + (((beat - start) / roll).floor() + 1.0) * roll;
                        sl.pending = Some((beat_time(&grid, next) * sr, a, b));
                    }
                } else {
                    sl.pending = Some((pos, a, b));
                }
            }
            None => {
                let Some(bg) = sl.bg else { return };
                if sl.roll.is_none() {
                    // Let go before its moment came: nothing happened.
                    sl.pending = None;
                    sl.bg = None;
                    return;
                }
                sl.pending = None;
                sl.ret_at = Some(if quantize && playing {
                    let beat = time_beat(&grid, bg / sr);
                    let next = start + (((beat - start) / unit).floor() + 1.0) * unit;
                    beat_time(&grid, next) * sr
                } else {
                    bg
                });
            }
        }
    }

    /// The shift buttons: the domain one length back or forward, the position with it. Not
    /// while a section is held.
    pub fn slicer_shift(&mut self, forward: bool) {
        let s = &self.pdr().slicer;
        if !s.active || !s.held.is_empty() {
            return;
        }
        if self.player.state.lock().pads.slicer.rolling() {
            return;
        }
        let grid = s.grid.clone();
        let (old, domain) = (s.start, self.slicer_unit() * SLICER_UNITS as f64);
        let target = slicer_shift(old, forward, domain, grid.len());
        let beat = time_beat(&grid, self.pos());
        self.slicer_set_domain(target);
        let new = self.pdr().slicer.start;
        self.pd().slicer.anchor = new;
        let t = beat_time(&grid, beat + (new - old));
        self.seek(t);
    }

    pub fn slicer_set_length(&mut self, li: usize) {
        let n = (self.px.active as usize).min(3);
        let li = li.min(3);
        self.prefs.pads.slicer_length[n] = li as u8;
        self.prefs.pads.slicer_roll[n] = slicer_roll_for_length(li) as u8;
        self.save_prefs();
        if self.pdr().slicer.active {
            let start = self.pdr().slicer.start;
            self.slicer_set_domain(start);
            let s = self.pdr().slicer.start;
            self.pd().slicer.anchor = s;
        }
    }

    pub fn slicer_set_roll(&mut self, ri: usize) {
        let n = (self.px.active as usize).min(3);
        self.prefs.pads.slicer_roll[n] = slicer_clamp_roll(self.prefs.pads.slicer_length[n] as usize, ri) as u8;
        self.save_prefs();
    }

    /// The loop button: SLICER or SLICER LOOP.
    pub fn slicer_toggle_loop(&mut self) {
        let on = !self.pdr().slicer.loop_mode;
        self.pd().slicer.loop_mode = on;
        if self.pdr().slicer.active {
            self.player.state.lock().pads.slicer.loop_mode = on;
            if !on {
                // Back to SLICER: the domain counts from where it stands.
                let s = self.pdr().slicer.start;
                self.pd().slicer.anchor = s;
            }
        }
    }

    /// The section sounding now (0..7), and whether it is a held one's roll.
    pub fn slicer_now(&self) -> (Option<usize>, bool) {
        let s = &self.pdr().slicer;
        if !s.active {
            return (None, false);
        }
        let st = self.player.state.lock();
        let sr = st.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
        (slicer_section(&s.b, st.pos / sr), st.pads.slicer.roll.is_some())
    }

    /// Slicer Capture (`SamplerControlPanel::deviceSlicerCapture` 0x100b0d83c): the eight
    /// sections as eight files, into the bank chosen in Preferences > Controller > Sampler.
    pub fn slicer_capture(&mut self) -> bool {
        let s = &self.pdr().slicer;
        if !s.active {
            self.toast_err(MSG_CAPTURE);
            return false;
        }
        let b = s.b;
        let Some(audio) = self.deck.audio.clone() else { return false };
        let bank = (self.prefs.sampler.slicer_bank.clamp(1, 4) - 1) as usize;
        let slots: Vec<usize> = (0..8).map(|i| bank * 16 + i).collect();
        if self.prefs.sampler.load_lock && slots.iter().any(|i| self.sampler_loaded(*i)) {
            self.toast_err(MSG_SLICER_SLOTS);
            return false;
        }
        let t = self.deck.track.and_then(|id| self.track(id));
        let title = t.map(|t| t.display_title()).unwrap_or_default();
        let bpm = self.current_bpm() as f32;
        let dir = crate::sampler_ext::capture_dir(&self.prefs.sampler.capture_dir);
        let sr = audio.sr as f64;
        let mut made = 0;
        for i in 0..8 {
            // (rekordbox renders each slice from its boundary to one sample before the next.)
            let f0 = ((b[i] * sr).round().max(0.0) as usize).min(audio.frames());
            let f1 = ((b[i + 1] * sr).round().max(0.0) as usize).min(audio.frames());
            if f1 <= f0 + 1 {
                continue;
            }
            let suffix = format!("_capture_s{}", i + 1);
            let Ok(path) = crate::sampler_ext::save_capture(&dir, &title, &suffix, audio.sr, &audio.samples[f0 * 2..f1 * 2]) else {
                self.toast_err("The captured slices could not be written.");
                return false;
            };
            let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            self.sampler_load_file(slots[i], path.to_string_lossy().to_string(), name, bpm);
            made += 1;
        }
        if made > 0 {
            // The bank that took the slices is the one showing.
            self.prefs.sampler.bank = bank as u8;
            self.save_prefs();
        }
        made == 8
    }

    // ------------------------------------------------------------------ ACT. CENSR

    fn censor_tables(&mut self) {
        if self.pads.tables {
            return;
        }
        // (rekordbox: a `djmdActiveCensor` row per region and on / effect per content.)
        let _ = self.db.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS active_censor(id INTEGER PRIMARY KEY, track_id INTEGER NOT NULL, in_ms REAL NOT NULL, out_ms REAL NOT NULL, params TEXT NOT NULL DEFAULT '');
             CREATE INDEX IF NOT EXISTS active_censor_track ON active_censor(track_id);
             CREATE TABLE IF NOT EXISTS active_censor_content(track_id INTEGER PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 0, effect INTEGER NOT NULL DEFAULT 0);",
        );
        self.pads.tables = true;
    }

    /// Read the loaded track's regions (`ActiveCensorView::getFromDb` 0x101087dd8).
    pub fn censor_load(&mut self) {
        self.censor_tables();
        let track = self.deck.track;
        let mut regions = Vec::new();
        let (mut on, mut kind) = (false, 0u8);
        if let Some(id) = track {
            if let Ok(mut q) = self.db.conn.prepare("SELECT id, in_ms, out_ms, params FROM active_censor WHERE track_id=?1 ORDER BY in_ms") {
                if let Ok(rows) = q.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, String>(3)?))) {
                    for (rid, a, b, p) in rows.flatten() {
                        regions.push(CensorRegion { id: rid, a: a / 1000.0, b: b / 1000.0, beats: decode_beats(&p) });
                    }
                }
            }
            if let Ok((e, k)) = self.db.conn.query_row("SELECT enabled, effect FROM active_censor_content WHERE track_id=?1", [id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))) {
                on = e != 0;
                kind = k.clamp(0, 3) as u8;
            }
        }
        let c = &mut self.pd().censor;
        c.track = track;
        c.regions = regions;
        c.on = on;
        c.kind = kind;
        c.pending_in = None;
        c.adjust = 0;
        self.censor_push();
    }

    fn censor_save_content(&mut self) {
        let (Some(id), on, kind) = (self.pdr().censor.track, self.pdr().censor.on, self.pdr().censor.kind) else { return };
        let _ = self.db.conn.execute("INSERT OR REPLACE INTO active_censor_content(track_id, enabled, effect) VALUES(?1, ?2, ?3)", rusqlite::params![id, on as i64, kind as i64]);
    }

    /// Hand the regions to the audio thread (with the beat of the effect in use, in seconds).
    pub fn censor_push(&mut self) {
        let (on, kind) = (self.pdr().censor.on, self.pdr().censor.kind as usize);
        let beats = &self.deck.beats;
        let fallback = self.deck.track.and_then(|id| self.track(id)).map(|t| t.bpm).unwrap_or(0.0);
        let spans: Vec<(f64, f64, f32)> = self
            .pdr()
            .censor
            .regions
            .iter()
            .map(|r| {
                let bpm = if beats.is_empty() { fallback } else { crate::player_ext::bpm_at(beats, r.a) };
                let beat = if bpm > 0.0 { 60.0 / bpm } else { 0.5 };
                (r.a, r.b, (r.beat(kind) * beat) as f32)
            })
            .collect();
        let mut st = self.player.state.lock();
        let sr = st.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
        let c = &mut st.pads.censor;
        if on && !spans.is_empty() {
            c.arm();
        }
        c.kind = kind as u8;
        c.regions = Arc::new(spans.into_iter().map(|(a, b, beat_secs)| CensorSpan { a: a * sr, b: b * sr, beat_secs }).collect());
        c.on = on;
    }

    /// Pads 1..4: the effect used inside the regions.
    pub fn censor_set_kind(&mut self, kind: u8) {
        if self.deck.track.is_none() {
            return;
        }
        self.pd().censor.kind = kind.min(3);
        self.censor_save_content();
        self.censor_push();
    }

    /// The region IN / OUT act on: the one being adjusted, else the one under the play head.
    fn censor_here(&self) -> Option<usize> {
        let c = &self.pdr().censor;
        if c.adjust != 0 && c.adjust_i < c.regions.len() {
            return Some(c.adjust_i);
        }
        censor_at(&c.regions, self.pos())
    }

    /// A time on the deck's SNAP lattice with Q on (`addRange` quantizes with type 0).
    fn censor_q(&self, t: f64) -> f64 {
        if self.performance() { self.quantize_action(t, crate::deck::QType::Snap) } else { self.quantized(t) }
    }

    /// IN (`onIn` 0x101084534). While IN adjust runs: it ends and the in point is saved.
    /// Inside a region ("IN ADJ"): OUT adjust ends (saved) and IN adjust starts, the playhead
    /// going to the in point (`doAdjustInPoint`). Outside: the in point is the playing time.
    pub fn censor_in(&mut self) {
        if self.deck.track.is_none() {
            return;
        }
        if self.pdr().censor.adjust == 1 {
            self.censor_end_adjust();
            return;
        }
        if let Some(i) = self.censor_here() {
            if self.pdr().censor.adjust == 2 {
                self.censor_end_adjust();
            }
            let a = self.pdr().censor.regions[i].a;
            let c = &mut self.pd().censor;
            c.adjust = 1;
            c.adjust_i = i;
            c.pending_in = None;
            self.seek(a);
            return;
        }
        let t = self.censor_q(self.pos());
        self.pd().censor.pending_in = Some(t);
    }

    /// OUT (`onOut` 0x101084744). While OUT adjust runs: it ends and the out point is saved.
    /// Inside a region ("OUT ADJ"): IN adjust ends (saved) and OUT adjust starts, the playhead
    /// going to the out point. Outside, with an in point waiting: the region up to the playing
    /// time is added (`addRange`: both ends on the SNAP lattice with Q on, an out that lands on
    /// the in point moves one step on, then one sample back; refused under 6.667 ms or over
    /// another region) and saved.
    pub fn censor_out(&mut self) -> bool {
        if self.deck.track.is_some() && self.pdr().censor.adjust == 2 {
            self.censor_end_adjust();
            return true;
        }
        if self.deck.track.is_some() {
            if let Some(i) = self.censor_here() {
                if self.pdr().censor.adjust == 1 {
                    self.censor_end_adjust();
                }
                let b = self.pdr().censor.regions[i].b;
                let c = &mut self.pd().censor;
                c.adjust = 2;
                c.adjust_i = i;
                self.seek(b);
                return true;
            }
        }
        let (Some(a), Some(track)) = (self.pdr().censor.pending_in, self.deck.track) else { return false };
        let sr = self.deck.audio.as_ref().map(|x| x.sr as f64).unwrap_or(44100.0);
        let mut b = self.censor_q(self.pos());
        if self.prefs.quantize && (b - a).abs() < 1e-6 && self.deck.beats.len() >= 2 {
            let unit = if self.performance() { self.quantize_unit(crate::deck::QType::Snap) } else { 1.0 / self.prefs.quantize_div.max(1) as f64 };
            b = crate::deck::grid_pos(&self.deck.beats, a).and_then(|p| crate::deck::grid_time(&self.deck.beats, p + unit)).unwrap_or(a + self.beat_len() * unit);
        }
        if self.prefs.quantize {
            b -= 1.0 / sr;
        }
        if !censor_can_add(&self.pdr().censor.regions, a, b) {
            return false;
        }
        self.censor_tables();
        let mut r = CensorRegion { id: 0, a, b, beats: CENSOR_DEFAULT_BEATS };
        if self.db.conn.execute("INSERT INTO active_censor(track_id, in_ms, out_ms, params) VALUES(?1, ?2, ?3, ?4)", rusqlite::params![track, a * 1000.0, b * 1000.0, encode_beats(&r.beats)]).is_ok() {
            r.id = self.db.conn.last_insert_rowid();
        }
        let c = &mut self.pd().censor;
        c.regions.push(r);
        c.regions.sort_by(|x, y| x.a.total_cmp(&y.a));
        c.pending_in = None;
        self.censor_push();
        true
    }

    /// Adjust running: the edge follows the playhead where `setInPoint` / `setOutPoint`
    /// allow it (`doActiveCensorIfNeeded`, before each block). Called every frame.
    fn censor_follow(&mut self) {
        let (adj, i) = (self.pdr().censor.adjust, self.pdr().censor.adjust_i);
        if adj == 0 {
            return;
        }
        if i >= self.pdr().censor.regions.len() {
            self.pd().censor.adjust = 0;
            return;
        }
        let t = self.pos();
        let r = self.pdr().censor.regions[i].clone();
        let (a, b) = if adj == 1 { (t, r.b) } else { (r.a, t) };
        if (a - r.a).abs() < 1e-4 && (b - r.b).abs() < 1e-4 {
            return;
        }
        if censor_edge_ok(&self.pdr().censor.regions, i, a, b) {
            let reg = &mut self.pd().censor.regions[i];
            reg.a = a;
            reg.b = b;
            self.censor_push();
        }
    }

    /// End IN / OUT adjust and save the region's points (`setInPointToDb` / `setOutPointToDb`).
    pub fn censor_end_adjust(&mut self) {
        let i = self.pdr().censor.adjust_i;
        self.pd().censor.adjust = 0;
        let Some(r) = self.pdr().censor.regions.get(i).cloned() else { return };
        if r.id > 0 {
            let _ = self.db.conn.execute("UPDATE active_censor SET in_ms=?1, out_ms=?2 WHERE id=?3", rusqlite::params![r.a * 1000.0, r.b * 1000.0, r.id]);
        }
    }

    /// The IN / OUT pads read "IN ADJ" / "OUT ADJ" while the playhead is in a region or an
    /// adjust runs (`ActiveCensorPad::paintButton`, flag +532); and which adjust runs.
    pub fn censor_adjust_state(&self) -> (bool, u8) {
        (self.censor_here().is_some(), self.pdr().censor.adjust)
    }

    /// CLEAR (`onClear` 0x101084ab0): the region under the play head goes.
    pub fn censor_clear(&mut self) -> bool {
        // (Not while adjusting, `onClear`.)
        if self.pdr().censor.adjust != 0 {
            return false;
        }
        let t = self.pos();
        let Some(i) = censor_at(&self.pdr().censor.regions, t) else {
            // (Nothing under the play head: an in point that waits is dropped.)
            self.pd().censor.pending_in = None;
            return false;
        };
        let r = self.pd().censor.regions.remove(i);
        if r.id > 0 {
            let _ = self.db.conn.execute("DELETE FROM active_censor WHERE id=?1", [r.id]);
        }
        self.censor_push();
        true
    }

    /// ON/OFF (`onOnOff` 0x101084b68), kept per track.
    pub fn censor_onoff(&mut self) {
        if self.deck.track.is_none() {
            return;
        }
        let on = !self.pdr().censor.on;
        self.pd().censor.on = on;
        self.censor_tables();
        self.censor_save_content();
        self.censor_push();
    }

    /// The side buttons: jump to the in point before or after the play head.
    pub fn censor_call(&mut self, next: bool) {
        let t = self.pos();
        if let Some(a) = censor_neighbour(&self.pdr().censor.regions, t, next) {
            self.seek(a);
        }
    }

    /// The beat buttons act on the region under the play head, or the one that comes next.
    pub fn censor_beat(&mut self, right: bool) {
        let t = self.pos();
        let kind = self.pdr().censor.kind as usize;
        let regions = &self.pdr().censor.regions;
        let Some(i) = censor_at(regions, t).or_else(|| regions.iter().position(|r| r.a >= t)).or(if regions.is_empty() { None } else { Some(regions.len() - 1) }) else { return };
        let cur = regions[i].beats[kind];
        let new = censor_beat_step(kind, cur, right);
        if new == cur {
            return;
        }
        let (id, text) = {
            let r = &mut self.pd().censor.regions[i];
            r.beats[kind] = new;
            (r.id, encode_beats(&r.beats))
        };
        if id > 0 {
            let _ = self.db.conn.execute("UPDATE active_censor SET params=?1 WHERE id=?2", rusqlite::params![text, id]);
        }
        self.censor_push();
    }

    /// The beat the beat buttons would change, as text.
    pub fn censor_beat_text(&self) -> String {
        let t = self.pos();
        let c = &self.pdr().censor;
        let kind = c.kind as usize;
        let r = censor_at(&c.regions, t).or_else(|| c.regions.iter().position(|r| r.a >= t)).or(if c.regions.is_empty() { None } else { Some(c.regions.len() - 1) });
        fraction_text(r.map(|i| c.regions[i].beats[kind]).unwrap_or(CENSOR_DEFAULT_BEATS[kind]))
    }

    // ------------------------------------------------------------------ USER 1..8

    /// The sixteen items of USER `u` (0..7): rekordbox's template with the Pad Editor's changes.
    pub fn user_items(&mut self, u: usize) -> Vec<PadItem> {
        let u = u.min(7);
        if self.pads.templates.is_none() {
            self.pads.templates = Some((0..8).map(user_template).collect());
        }
        let mut items = self.pads.templates.as_ref().map(|t| t[u].clone()).unwrap_or_default();
        items.resize(16, PadItem { function: String::new(), value: String::new(), key: None });
        if let Some(over) = self.prefs.pads.user.get(u) {
            for (i, o) in over.iter().enumerate().take(16) {
                if let Some(it) = o.as_ref().and_then(|l| parse_pad_line(l)) {
                    items[i] = it;
                }
            }
        }
        items
    }

    /// The Pad Editor: give pad `pad` (0..15) of USER `u` a function and value (None: back
    /// to the template's).
    pub fn user_set_item(&mut self, u: usize, pad: usize, item: Option<(&str, &str)>) {
        let (u, pad) = (u.min(7), pad.min(15));
        if self.prefs.pads.user.len() < 8 {
            self.prefs.pads.user.resize(8, vec![None; 16]);
        }
        if self.prefs.pads.user[u].len() < 16 {
            self.prefs.pads.user[u].resize(16, None);
        }
        self.prefs.pads.user[u][pad] = item.map(|(f, v)| format!("function={f},value={v}"));
        self.save_prefs();
    }

    /// A USER pad pressed: what its item says, through the same code as the mode it comes from.
    pub fn user_pad_down(&mut self, item: &PadItem, shift: bool) {
        match user_act(item) {
            UserAct::HotCue(s) => {
                if let Some(k) = item.key.filter(|k| (-12..=12).contains(k)) {
                    self.deck.x.key_sync = false;
                    self.deck.x.key_shift = k;
                    self.push_deck_controls();
                }
                self.hot_cue_down(s)
            }
            UserAct::BeatJump(b, forward) => {
                let id = match b {
                    None => 0,
                    Some(b) => (2..24).step_by(2).find(|i| jump_beats(*i).is_some_and(|x| (x - b).abs() < 1e-9)).unwrap_or(8),
                } + forward as u8;
                self.pad_beat_jump(id);
            }
            UserAct::BeatLoop(b) => {
                if let Some(i) = loop_pad_for(b) {
                    self.pad_beat_loop(i);
                } else if self.deck.audio.is_some() {
                    self.auto_loop(b);
                }
            }
            UserAct::Key(k) => {
                let s1 = self.key_scale1();
                let pos = (0..40).find(|p| key_at_in(*p, s1) == k);
                if let Some(p) = pos {
                    self.pad_key(p);
                }
            }
            UserAct::Sampler(slot) => {
                let n = self.px.active;
                self.sampler_deck_pad_down(n, slot, shift);
                self.pd().user_sampler = Some(slot);
            }
            UserAct::SeqCall(slot) => self.sampler_seq_call(slot, shift),
            UserAct::PadFx(bank, pad) => {
                let n = (self.px.active as usize).min(3);
                let idx = if bank == 2 {
                    self.pad_fx_combination_ready(n);
                    32 + pad
                } else {
                    bank * 16 + pad
                };
                crate::ui_padfx::pad_down(self, n, idx);
                self.pd().user_held = Some(UserHeld::PadFx(idx));
            }
            UserAct::Transport(t) => self.user_transport(t),
            UserAct::SampleScratch(slot) => self.sample_scratch_pad(slot),
            UserAct::Other => {}
        }
    }

    /// A transport item of a USER pad, through the deck's own functions.
    fn user_transport(&mut self, t: Transport) {
        match t {
            Transport::PlayPause => self.play_pause_synced(),
            Transport::Cue => {
                self.cue_down();
                self.pd().user_held = Some(UserHeld::Cue);
            }
            Transport::PitchBend(d) => {
                self.pitch_bend(d as i32);
                self.pd().user_held = Some(UserHeld::Bend);
            }
            Transport::Bpm(dir, hundredths) => {
                // The tempo slider moves by `hundredths` / 100 percent (never past its range).
                if self.sync_follower() {
                    self.toast_err("Invalid operation. In Sync mode, Tempo Slider is only available for Master Deck.");
                    return;
                }
                let range = self.tempo_range();
                let v = self.deck.x.tempo_adj + dir as f64 * hundredths as f64 * 0.0001;
                self.deck.x.tempo_adj = v.clamp(-range, range);
                self.push_deck_controls();
            }
            Transport::CueLoopCall(d) => self.memory_jump(d as i32),
            Transport::TempoReset => self.tempo_reset(),
            Transport::Slip => self.toggle_slip(),
            Transport::Track(d) => {
                if d < 0 {
                    self.previous_track();
                } else {
                    self.load_adjacent(1);
                }
            }
            Transport::ActiveLoop => self.toggle_cue_point_active_loop(),
        }
    }

    pub(crate) fn toggle_cue_point_active_loop(&mut self) {
        let cp = self.deck.cue_point;
        if let Some(mut cue) = self.deck.cues.iter().find(|c| c.kind == CueKind::Memory && c.end.is_some() && (c.start - cp).abs() < 0.005).cloned() {
            cue.active_loop = !cue.active_loop;
            self.update_cue(cue);
        }
    }

    /// PAD FX's combination bank (16 pads, `padFxCombinationPreset`) sits after the deck's two
    /// banks, so that a USER pad can play it through the PAD FX engine (indices 32..47).
    fn pad_fx_combination_ready(&mut self, n: usize) {
        let d = &mut self.prefs.fx.padfx.decks[n.min(3)];
        if d.len() < 48 {
            let std = crate::ui_padfx::preset(0);
            while d.len() < 32 {
                let k = d.len();
                d.push(std[k]);
            }
            d.truncate(32);
            d.extend(crate::ui_padfx::preset(2));
        }
    }

    /// SP.SCRATCH pad `pad` (0..15; `SamplerControlPanel::SampleScratch::doSampleScratch`):
    /// the sampler slot of the bank in use is loaded onto the deck to be scratched there, with
    /// the slot's sync; pads 9-16 only with 16 slots. A playing deck under Load Lock refuses
    /// (rekordbox's message). The pad mode is a controller's (rekordbox shows no view for it:
    /// `MemoryCuePanel::setPadMode` case 8 sets only the controller's pad mode).
    pub fn sample_scratch_pad(&mut self, pad: usize) {
        if pad > 15 || (pad >= 8 && !self.prefs.sampler.slots16) {
            return;
        }
        let idx = self.prefs.sampler.bank.min(3) as usize * 16 + pad;
        let Some(slot) = self.prefs.sampler.slots.get(idx).cloned().flatten() else { return };
        if slot.track <= 0 {
            return;
        }
        let before = self.deck.track;
        self.load_track_here(slot.track);
        if self.deck.track != Some(slot.track) || (before == Some(slot.track) && self.deck.audio.is_none()) {
            return;
        }
        if !slot.looped && slot.offset_ms > 0.0 {
            let t = slot.offset_ms as f64 / 1000.0;
            self.deck.cue_point = t;
            self.seek(t);
        }
        if self.deck.x.beat_sync != slot.sync {
            self.toggle_beat_sync();
        }
    }

    pub fn user_pad_up(&mut self) {
        if let Some(slot) = self.pd().user_sampler.take() {
            self.sampler_pad_up(slot);
        }
        match self.pd().user_held.take() {
            Some(UserHeld::Cue) => self.cue_up(),
            Some(UserHeld::Bend) => self.pitch_bend(0),
            Some(UserHeld::PadFx(idx)) => {
                let n = (self.px.active as usize).min(3);
                crate::ui_padfx::pad_up(self, n, idx);
            }
            None => {}
        }
        let held = self.deck.hot_preview.is_some() || self.player.state.lock().slip_hold;
        if held {
            self.hot_cue_up();
        }
    }

    /// A printable summary of the active deck's pad state (for the scripted run).
    pub fn pads_state_text(&self) -> String {
        let d = self.pdr();
        let n = (self.px.active as usize).min(3);
        let st = self.player.state.lock();
        let sr = st.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
        let secs = |f: f64| (f / sr * 1000.0).round() / 1000.0;
        let sl = &st.pads.slicer;
        let hot: Vec<char> = self.deck.cues.iter().filter(|c| c.kind == CueKind::Hot).map(|c| (b'A' + c.slot as u8) as char).collect();
        format!(
            "deck {} mode {} | playing {} pos {:.3}s loop {:?} key {:+} MT {} | jump page {} | slicer on {} loop-mode {} start-beat {} length {} roll {} held {:?} roll {:?} waiting {:?} bg {:?} starts {} | censor on {} fx {} regions {:?} in {:?} active {:?} adjust {} | key cue {:?} hot {:?} | tempo {:+.2}% padfx held {:?} | last message {:?}",
            n + 1,
            PadMode::from_code(self.px.pad_mode[n]).label(),
            st.playing,
            st.pos / sr,
            st.loop_range.map(|l| ((l.0 * 1000.0).round() / 1000.0, (l.1 * 1000.0).round() / 1000.0)),
            self.deck.x.key_shift,
            self.deck.x.master_tempo,
            self.prefs.pads.beat_jump_page[n],
            sl.on,
            sl.loop_mode,
            d.slicer.start,
            SLICER_LENGTHS[(self.prefs.pads.slicer_length[n] as usize).min(3)],
            slicer_roll_label(self.prefs.pads.slicer_roll[n] as usize),
            d.slicer.held,
            sl.roll.map(|r| (secs(r.0), secs(r.1))),
            sl.pending.map(|p| secs(p.0)),
            sl.bg.map(secs),
            sl.starts,
            st.pads.censor.on,
            CENSOR_FX[(d.censor.kind as usize).min(3)],
            d.censor.regions.iter().map(|r| ((r.a * 1000.0).round() / 1000.0, (r.b * 1000.0).round() / 1000.0, fraction_text(r.beats[(d.censor.kind as usize).min(3)]))).collect::<Vec<_>>(),
            d.censor.pending_in.map(|t| (t * 1000.0).round() / 1000.0),
            st.pads.censor.active,
            d.censor.adjust,
            d.key_cue.map(|c| (b'A' + c as u8) as char),
            hot,
            self.deck.x.tempo_adj * 100.0,
            self.px.fxx.pad.decks[n].held,
            self.toasts.last().map(|t| t.text.as_str()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn censor_beats_survive_the_database_text() {
        let b = [(32, 1), (1, 8), (1, 8), (1, 16)];
        assert_eq!(decode_beats(&encode_beats(&b)), b);
        assert_eq!(decode_beats(""), CENSOR_DEFAULT_BEATS);
        assert_eq!(decode_beats("x,1/0,3/4"), [(32, 1), (1, 4), (3, 4), (3, 4)]);
    }
}
