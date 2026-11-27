//! PREPDECK_SHOT_PLAYER=keytest (with --screenshot, PREPDECK_SHOT_VIEW=performance and
//! PREPDECK_BENCH=<frames> to keep the window up, about 1400): presses real keys, one command
//! after another, in PERFORMANCE, EXPORT and EDIT mode, and prints what each changed. The keys
//! come from the mapping in use: rekordbox's "Performance 1" and "Export" presets, and a user
//! mapping ("keytest", a copy of the preset) that gives function keys to commands the preset
//! leaves without one. Held commands (CUE, a hot cue on a stopped deck, SLIP REVERSE, pitch
//! bend, a sampler slot) are printed while held and after the release. Also run: a click on
//! deck 2's title area, which loads the selected track there.
//! Only on a throwaway copy of the data folder.

use crate::app::*;
use crate::prefs_rb::Mode;
use crate::ui_prefs_keys as keys;
use eframe::egui;

/// (mode 0 EXPORT / 1 PERFORMANCE / 2 EDIT, command id, held, key to give it in the user
/// mapping when its preset has none, what it is)
type Step = (u8, u16, bool, &'static str, &'static str);

const STEPS: &[Step] = &[
    // ---- PERFORMANCE, preset Performance 1 (decks 1 and 2 loaded, deck 1 at its cue)
    (1, 0x3007, true, "", "Deck 1 CUE held (cue sampler), then let go"),
    (1, 0x3006, false, "", "Deck 1 Play/Pause (plays)"),
    (1, 0x300a, true, "", "Deck 1 Loop In (held a moment so that the loop is long enough)"),
    (1, 0x300b, false, "", "Deck 1 Loop Out"),
    (1, 0x3018, false, "", "Deck 1 Halve the loop"),
    (1, 0x300c, false, "", "Deck 1 Exit/Reloop"),
    (1, 0x3014, false, "", "Deck 1 4 Beat Loop (command + 4)"),
    (1, 0x3014, false, "", "Deck 1 4 Beat Loop again (ends it)"),
    (1, 0x301e, false, "", "Deck 1 Set Hot Cue A (sets it while playing)"),
    (1, 0x304f, true, "", "Deck 1 Pitch Bend + held, then let go"),
    (1, 0x305b, false, "", "Deck 1 Increase Tempo (medium)"),
    (1, 0x304d, false, "", "Deck 1 Master Tempo"),
    (1, 0x3061, true, "", "Deck 1 Slip Reverse held, then let go"),
    (1, 0x304b, false, "", "Deck 1 BEAT SYNC"),
    (1, 0x309a, false, "F1", "Deck 1 Forward 4 beats"),
    (1, 0x30b9, false, "F2", "Deck 1 Semitone Up"),
    (1, 0x3066, false, "F3", "Deck 1 Pad mode - Beat Jump"),
    (1, 0x306c, false, "F4", "Deck 1 Pad B (BEAT JUMP pad 2)"),
    (1, 0x3063, false, "F5", "Deck 1 Pad mode - Hot Cue"),
    (1, 0x3060, false, "F6", "Deck 1 Reverse (turn waits for the REVERSE beat with Q on)"),
    (1, 0x3060, false, "F6", "Deck 1 Reverse again"),
    (1, 0x3006, false, "", "Deck 1 Play/Pause (pauses)"),
    (1, 0x301e, true, "", "Deck 1 Hot Cue A on the paused deck held (Gate Cue off: plays on)"),
    (1, 0x3006, false, "", "Deck 1 Play/Pause (pauses)"),
    (1, 0x3106, false, "", "Deck 2 Play/Pause (N)"),
    (1, 0x3139, false, "", "Deck 2 Call Next Memory Cue (P)"),
    (1, 0x3106, false, "", "Deck 2 Play/Pause (pauses)"),
    (1, 0x3206, false, "F7", "Deck 3 Play/Pause (4-deck layout only: nothing in 2-deck)"),
    (1, 0xd118, false, "F8", "Mixer CH2 Lower Ch Fader (medium)"),
    (1, 0xd003, false, "F9", "Mixer Slide CROSSFADER to the right (small)"),
    (1, 0xd014, false, "F10", "Mixer CH1 Headphones Cue"),
    (1, 0xe000, false, "F11", "FX1 Assign Deck 1"),
    (1, 0xe00a, false, "F12", "FX1 FX1 On/Off"),
    (1, 0xe107, false, "F13", "FX2 Select FX 1"),
    (1, 0xe01b, false, "F14", "FX Common CFX3"),
    (1, 0xf000, true, "", "Sampler Playback slot 1 (X) held"),
    (1, 0xf021, false, "F15", "Sampler Select Sampler Bank 2"),
    (1, 0xd0f1, false, "F16", "Recordings Increase REC level"),
    (1, 0xcd03, false, "F17", "MIX POINT LINK Silent On/Off"),
    (1, 0x3000, false, "", "All Decks Zoom In (=)"),
    (1, 0x3004, false, "", "General Volume Down (command + F11)"),
    (1, 0x5506, false, "F18", "Playlist Column View Mode"),
    (1, 0x5505, false, "F20", "Playlist Tree View Mode"),
    (1, 0xb12a, false, "", "Browse Load track to Deck 2 (shift + cursor right)"),
    (1, 0xb12f, false, "", "Browse Instant Doubles Deck 1 (shift + command + cursor left)"),
    (1, 0xb047, false, "F19", "View 4 Deck Vertical"),
    (1, 0xb12b, false, "F22", "Browse Load track to Deck 3 (4-deck layout)"),
    (1, 1, false, "", "(waiting for deck 3's track)"),
    (1, 0x3206, false, "F7", "Deck 3 Play/Pause in the 4-deck layout"),
    (1, 0x3206, false, "F7", "Deck 3 Play/Pause (pauses)"),
    (1, 0xb044, false, "F21", "View 2 Deck Horizontal"),
    (1, 6, false, "", "(Preferences > Controller > Deck > HOT CUE: Smart Cue on)"),
    (1, 0x3022, false, "", "Deck 1 Clear Hot Cue B (shift + command + 2)"),
    (1, 7, false, "", "(deck 1 moved to 20 s, away from its cue point)"),
    (1, 0x3006, true, "", "Deck 1 Play/Pause (plays; held a moment)"),
    (1, 0x301f, false, "", "Deck 1 Set Hot Cue B on the playing deck (Smart Cue: the cue point goes there)"),
    (1, 0x3006, false, "", "Deck 1 Play/Pause (pauses)"),
    (1, 4, false, "", "(Preferences > Controller > Deck: Auto Cue on for deck 2; the next track loaded there)"),
    (1, 5, false, "", "(Preferences > Controller > Deck: Continuous Playing on; deck 1 played into its end)"),
    (1, 3, false, "", "(Preferences > Advanced > Browse: Library Protection on; the selected track's comment edited)"),
    (1, 0, false, "", "(Preferences > Controller > Deck > HOT CUE: Gate Cue on)"),
    (1, 0x301e, true, "", "Deck 1 Hot Cue A on the paused deck held with Gate Cue on (returns on release)"),
    // ---- the tree with the keyboard focus, under the preset: its own arrow keys
    (1, 8, false, "", "(Change Focus to the tree; a closed folder selected)"),
    (1, 20, false, "", "Tree: cursor right (opens the folder)"),
    (1, 20, false, "", "Tree: cursor right again (selects its first child)"),
    (1, 21, false, "", "Tree: cursor left (back to the folder)"),
    (1, 21, false, "", "Tree: cursor left again (closes it)"),
    (1, 22, false, "", "Tree: cursor down (the next playlist or folder)"),
    (1, 9, false, "", "(focus back on the track list)"),
    // ---- EXPORT, preset Export (Player B is Player A with Shift)
    (0, 0x3006, false, "", "Player A Play/Pause (spacebar)"),
    (0, 0x3106, false, "", "Player B Play/Pause (shift + spacebar)"),
    (0, 0x3009, false, "", "Player A Jump Reverse"),
    (0, 0x3006, false, "", "Player A Play/Pause (pauses)"),
    (0, 0x3106, false, "", "Player B Play/Pause (pauses)"),
    (0, 0xd010, false, "", "Mixer CH1 Trim (no mixer in EXPORT: nothing)"),
    // ---- EDIT, preset Edit Mode
    (2, 2, false, "", "(the selected track opened in the editor)"),
    (2, 0xcc18, false, "", "EDIT Zoom In (W)"),
    (2, 0xcc0e, false, "F1", "EDIT BEAT SELECT - 4 Beats (no key in the preset: the run's own mapping gives F1)"),
    (2, 0xcc1b, false, "", "EDIT Browser (B)"),
    (2, 0xcc1b, false, "", "EDIT Browser again (B)"),
];

#[derive(Default)]
struct Run {
    at: usize,
    sub: u32,
    wait: u32,
    before: String,
    key: Option<(egui::Key, egui::Modifiers)>,
    setup: bool,
    done: bool,
    acted: u32,
    changed: u32,
    title_click: u8,
    ready: bool,
    sub_wait: u32,
}

static RUN: parking_lot::Mutex<Option<Run>> = parking_lot::Mutex::new(None);

fn mods_of(sc: &Shortcut) -> egui::Modifiers {
    egui::Modifiers { alt: sc.alt, ctrl: sc.ctrl, shift: sc.shift, mac_cmd: sc.cmd, command: sc.cmd }
}

/// What the commands change, compact: the decks, the mixer, effects, sampler, view.
fn state(a: &App) -> String {
    let mut v = vec![];
    for n in 0..a.deck_count().min(4) {
        let d = a.deck_of(n);
        let (pos, playing, lr, bend, rev, sr) = {
            let s = a.player.decks[n as usize].lock();
            let sr = s.audio.as_ref().map(|x| x.sr as f64).unwrap_or(1.0);
            (s.pos / sr, s.playing, s.loop_range, s.bend, s.dk.reverse, sr)
        };
        let _ = sr;
        let hots = d.cues.iter().filter(|c| c.kind == crate::model::CueKind::Hot).count();
        let title = d.track.and_then(|t| a.track(t)).map(|t| t.title.chars().take(14).collect::<String>().replace(' ', "_")).unwrap_or_default();
        v.push(format!(
            "D{} title={} play={} pos={:.2} cue={:.2} prev={} loop={:?} hot={} tempo={:+.4} bend={:+.3} mt={} key={:+} sync={} rev={} pad={:?}",
            n + 1,
            title,
            playing,
            pos,
            d.cue_point,
            d.cue_preview || d.hot_preview.is_some(),
            lr.map(|l| ((l.0 * 100.0).round() / 100.0, (l.1 * 100.0).round() / 100.0)),
            hots,
            d.x.tempo_adj,
            bend,
            d.x.master_tempo,
            d.x.key_shift,
            d.x.beat_sync,
            rev,
            a.with_deck_ref(n),
        ));
    }
    let m = &a.px.mix;
    let smp_play = crate::sampler::SAMPLER.lock().as_ref().map(|e| e.slot_state(0, a.prefs.sampler.bank as usize).0).unwrap_or(false);
    let (tv, topen) = match a.view {
        View::Playlist(id) => (a.node(id).map(|n| n.name.replace(' ', "_")).unwrap_or_default(), a.expanded.contains(&id)),
        _ => (format!("{:?}", a.view).replace(' ', "_"), false),
    };
    v.push(format!("tree focus={} sel={} open={}", a.dialogs.ext.focus_tree, tv, topen));
    v.push(format!("smp1 loaded={} playing={} | msg text={:?}", a.sampler_loaded(a.prefs.sampler.bank as usize * 16), smp_play, a.toasts.last().map(|t| t.text.replace(' ', "_")).unwrap_or_default()));
    v.push(format!("mix fader2={:.3} xf={:.3} cue1={} | fx fx1assign1={} fx1on1={} fx2slot1={:?} cfx={} | smp bank={} | rec level={:.2} | mpl silent={} | view zoom={} vol={:.2} column={} perf4={} vert={} | edit zoom={} browser={} sel={:?}",
        m[1].fader, a.px.crossfader, m[0].cue, a.px.fx[0].params.assign[0], a.px.fx[0].params.slots[0].on, a.px.fx[1].params.slots[0].kind, a.px.cfx_kind, a.prefs.sampler.bank, a.prefs.rec.level, a.mixx.mpl.silent_on.iter().map(|x| *x as u8).map(|x| x.to_string()).collect::<String>(), a.prefs.zoom, a.player.state.lock().volume, a.prefs.bx.column_view, a.prefs.perf_four, a.prefs.perf_lay.vertical, a.edit.zoom, a.prefs.edit.show_browser, a.edit.project.as_ref().and_then(|p| p.session.selection.range()).map(|r| format!("{:.1}-{:.1}", r.0, r.1)).unwrap_or_default()));
    v.join(" | ")
}

/// What differs between two states: "segment.name=value" for each changed value.
fn changes(a: &str, b: &str) -> String {
    let segs = |s: &str| -> Vec<(String, Vec<(String, String)>)> {
        s.split(" | ")
            .map(|seg| {
                let mut it = seg.split(' ');
                let head = it.next().unwrap_or("").to_string();
                let kv = it.map(|t| match t.split_once('=') { Some((k, v)) => (k.to_string(), v.to_string()), None => (t.to_string(), String::new()) }).collect();
                (head, kv)
            })
            .collect()
    };
    let (sa, sb) = (segs(a), segs(b));
    let mut out = vec![];
    for (head, kv) in &sb {
        match sa.iter().find(|x| &x.0 == head) {
            None => out.push(format!("{head} appears")),
            Some((_, old)) => {
                for (k, v) in kv {
                    if old.iter().find(|x| &x.0 == k).map(|x| &x.1) != Some(v) {
                        out.push(format!("{head}.{k}={v}"));
                    }
                }
            }
        }
    }
    for (head, _) in &sa {
        if !sb.iter().any(|x| &x.0 == head) {
            out.push(format!("{head} gone"));
        }
    }
    if out.is_empty() { "nothing changed".into() } else { out.join(" ") }
}

impl App {
    fn with_deck_ref(&self, n: u8) -> String {
        let m = self.px.pad_mode.get(n as usize).copied().unwrap_or(0);
        format!("{m}")
    }

    pub fn shot_key_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        if !std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "keytest")).unwrap_or(false) || frame < 80 {
            return;
        }
        raw.focused = true;
        let mut g = RUN.lock();
        let run = g.get_or_insert_with(Run::default);
        if run.done {
            return;
        }
        if !run.setup {
            run.setup = true;
            drop(g);
            self.keytest_setup();
            return;
        }
        // Wait for the two tracks to be read.
        if self.deck_of(0).audio.is_none() || self.deck_of(1).audio.is_none() {
            return;
        }
        if !run.ready {
            // (PREPDECK_BENCH started the deck in focus, muted: both decks stand at their cue.)
            run.ready = true;
            for n in 0..2u8 {
                self.with_deck(n, |a| {
                    a.set_playing(false);
                    let c = a.deck.cue_point;
                    a.seek(c);
                });
            }
            run.wait = 5;
            return;
        }
        if run.wait > 0 {
            run.wait -= 1;
            return;
        }
        let Some(&(mode, id, held, _, what)) = STEPS.get(run.at) else {
            // Last: deck 2's title area clicked with a track selected.
            match run.title_click {
                0 => {
                    if self.prefs.app_mode != 1 {
                        self.set_app_mode(1);
                        run.wait = 10;
                        return;
                    }
                    self.rebuild_list();
                    if let Some(i) = self.list.iter().position(|t| Some(*t) != self.deck_of(1).track && self.track(*t).map(|x| !x.missing).unwrap_or(false)) {
                        self.set_anchor(i);
                        self.selected.clear();
                        self.selected.insert(self.list[i]);
                    }
                    run.before = state(self);
                    let p = title_point(self);
                    raw.events.push(egui::Event::PointerMoved(p));
                    run.title_click = 1;
                }
                1 => {
                    let p = title_point(self);
                    raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
                    raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
                    run.title_click = 2;
                    run.wait = 20;
                }
                _ => {
                    let after = state(self);
                    eprintln!("keytest: click on deck 2's title area with \"{}\" selected -> {}", self.anchor_id.and_then(|t| self.track(t)).map(|t| t.title.clone()).unwrap_or_default(), changes(&run.before, &after));
                    eprintln!("keytest: done, {} key presses sent, {} changed something", run.acted, run.changed);
                    run.done = true;
                }
            }
            return;
        };
        match run.sub {
            0 => {
                // The mode and the mapping of the step.
                if self.prefs.app_mode != mode {
                    self.set_app_mode(mode);
                    mode_mapping(self, mode);
                    if mode == 0 {
                        // (Player B is there in the 2 Players layout.)
                        self.set_player_layout(1);
                    }
                    if mode == 2 {
                        crate::keycmd::release_all(self);
                    }
                    eprintln!("keytest: == {} mode, mapping \"{}\"", Mode::of(mode).name(), keys::current(&self.prefs, Mode::of(mode)));
                    run.wait = 30;
                    return;
                }
                if id == 8 || id == 9 {
                    if id == 8 {
                        let folder = self.nodes.iter().find(|n| n.kind == crate::model::NodeKind::Folder && self.nodes.iter().any(|c| c.parent == n.id)).map(|n| n.id);
                        match folder {
                            Some(f) => {
                                self.expanded.remove(&f);
                                self.side = SideMode::Playlists;
                                self.set_view(View::Playlist(f));
                                self.dialogs.ext.focus_tree = true;
                            }
                            None => eprintln!("keytest: {what}: no folder with playlists in this library"),
                        }
                    } else {
                        self.dialogs.ext.focus_tree = false;
                        // (Back to the collection with deck 1's track selected, for EDIT.)
                        self.side = SideMode::Collection;
                        self.set_view(View::Collection);
                        self.rebuild_list();
                        if let Some(i) = self.deck_of(0).track.and_then(|t| self.list.iter().position(|x| *x == t)) {
                            self.set_anchor(i);
                            self.selected.clear();
                            self.selected.insert(self.list[i]);
                        }
                    }
                    eprintln!("keytest: {what}");
                    run.at += 1;
                    run.wait = 3;
                    return;
                }
                if (20..=22).contains(&id) {
                    let key = [egui::Key::ArrowRight, egui::Key::ArrowLeft, egui::Key::ArrowDown][(id - 20) as usize];
                    run.before = state(self);
                    raw.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
                    run.key = Some((key, Default::default()));
                    run.acted += 1;
                    eprintln!("keytest: {what}: key pressed");
                    run.sub = 1;
                    return;
                }
                if id == 7 {
                    self.with_deck(0, |a| a.seek(20.0));
                    eprintln!("keytest: {what}");
                    run.at += 1;
                    run.wait = 3;
                    return;
                }
                if id == 6 {
                    self.prefs.rb.smart_cue = true;
                    eprintln!("keytest: {what}");
                    run.at += 1;
                    return;
                }
                if id == 3 {
                    self.prefs.rb.library_protection = true;
                    let sel: Vec<i64> = self.anchor_id.into_iter().collect();
                    let before = sel.first().and_then(|t| self.track(*t)).map(|t| t.comment.clone()).unwrap_or_default();
                    let ok = self.edit_field(&sel, Col::Comments, "keytest");
                    let after = sel.first().and_then(|t| self.track(*t)).map(|t| t.comment.clone()).unwrap_or_default();
                    eprintln!("keytest: {what}: edit accepted {ok}, comment {:?} -> {:?}, message {:?}", before, after, self.toasts.last().map(|t| t.text.clone()));
                    self.prefs.rb.library_protection = false;
                    run.at += 1;
                    return;
                }
                if id == 4 {
                    // Auto Cue on deck 2, and a track without memory cues loaded there.
                    if run.sub_wait == 0 {
                        self.prefs.rb.auto_cue_deck[1] = true;
                        let cand = self.list.iter().copied().find(|t| Some(*t) != self.deck_of(1).track && self.track(*t).map(|x| !x.missing && x.duration > 60.0).unwrap_or(false) && !self.db.cues(*t).unwrap_or_default().iter().any(|c| c.kind == crate::model::CueKind::Memory));
                        match cand {
                            Some(t) => self.with_deck(1, |a| a.load_track_here(t)),
                            None => {
                                eprintln!("keytest: {what}: no track without memory cues");
                                run.at += 1;
                                return;
                            }
                        }
                    }
                    let d = self.deck_of(1);
                    if (d.audio.is_none() || d.loading.is_some()) && run.sub_wait < 900 {
                        run.sub_wait += 1;
                        return;
                    }
                    let first = d.audio.as_ref().and_then(|a| crate::prefs_bridge::auto_cue_frame(&a.samples, a.sr, crate::prefs_bridge::auto_cue_db(self.prefs.rb.auto_cue_level)).map(|f| f as f64 / a.sr as f64));
                    let grid = d.beats.first().map(|b| b.time);
                    eprintln!("keytest: {what}: \"{}\" cue point {:.3} s (first sound at -60 dB {:?}, first beat {:?})", d.track.and_then(|t| self.track(t)).map(|t| t.title.clone()).unwrap_or_default(), d.cue_point, first, grid);
                    self.prefs.rb.auto_cue_deck[1] = false;
                    run.sub_wait = 0;
                    run.at += 1;
                    return;
                }
                if id == 5 {
                    if run.sub_wait == 0 {
                        self.prefs.rb.auto_play = true;
                        run.before = self.deck_of(0).track.and_then(|t| self.track(t)).map(|t| t.title.clone()).unwrap_or_default();
                        self.with_deck(0, |a| {
                            let d = a.deck.duration;
                            a.seek((d - 0.4).max(0.0));
                            a.set_playing(true);
                        });
                    }
                    let playing = self.player.decks[0].lock().playing;
                    let title = self.deck_of(0).track.and_then(|t| self.track(t)).map(|t| t.title.clone()).unwrap_or_default();
                    if (title == run.before || !playing) && run.sub_wait < 900 {
                        run.sub_wait += 1;
                        return;
                    }
                    eprintln!("keytest: {what}: deck 1 had {:?}, now {:?}, playing {} (after {} frames)", run.before, title, playing, run.sub_wait);
                    self.prefs.rb.auto_play = false;
                    self.with_deck(0, |a| a.set_playing(false));
                    run.sub_wait = 0;
                    run.at += 1;
                    return;
                }
                if id == 2 {
                    if self.edit.project.is_none() && run.sub_wait == 0 {
                        if let Some(t) = self.anchor_id {
                            self.load_track(t);
                        }
                    }
                    if self.edit.project.is_none() && run.sub_wait < 900 {
                        run.sub_wait += 1;
                        return;
                    }
                    eprintln!("keytest: {what}: project open: {}", self.edit.project.is_some());
                    // EDIT's own user mapping: a copy of its preset with F1 for BEAT SELECT 4.
                    keys::copy(&mut self.prefs, Mode::Edit, keys::EDIT_PRESET.0, "keytest edit");
                    keys::select(&mut self.prefs, Mode::Edit, "keytest edit");
                    if let Some(r) = keys::commands(Mode::Edit).iter().find(|r| r.id == 0xcc0e) {
                        keys::assign(&mut self.prefs, Mode::Edit, "keytest edit", r, "F1");
                    }
                    run.sub_wait = 0;
                    run.at += 1;
                    return;
                }
                if id == 1 {
                    // Wait (at most 600 frames) for deck 3's track to be read.
                    if self.deck_of(2).audio.is_none() && run.sub_wait < 600 {
                        run.sub_wait += 1;
                        return;
                    }
                    eprintln!("keytest: {what}: deck 3 read: {}", self.deck_of(2).audio.is_some());
                    run.sub_wait = 0;
                    run.at += 1;
                    return;
                }
                if id == 0 {
                    self.prefs.rb.gate_cue = true;
                    eprintln!("keytest: {what}");
                    run.at += 1;
                    run.wait = 3;
                    return;
                }
                let m = Mode::of(mode);
                let rows = keys::commands(m);
                let Some(row) = rows.iter().find(|r| r.id == id) else {
                    eprintln!("keytest: {what}: command {id:x} is not listed in {}", m.name());
                    run.at += 1;
                    return;
                };
                let k = keys::key_of(&self.prefs, m, &keys::current(&self.prefs, m), row);
                let Some(sc) = keys::juce_to_stored(&k).and_then(|s| Shortcut::parse(&s)) else {
                    eprintln!("keytest: {what}: no key ({k:?})");
                    run.at += 1;
                    return;
                };
                run.before = state(self);
                let mods = mods_of(&sc);
                raw.modifiers = mods;
                raw.events.push(egui::Event::Key { key: sc.key, physical_key: None, pressed: true, repeat: false, modifiers: mods });
                run.key = Some((sc.key, mods));
                run.acted += 1;
                eprintln!("keytest: {what} [{}]: key \"{}\" pressed", crate::ui_prefs_keys::stored_to_juce(&sc.name()), k);
                run.sub = if held { 10 } else { 1 };
            }
            1 => {
                if let Some((key, mods)) = run.key {
                    raw.modifiers = mods;
                    raw.events.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: mods });
                }
                raw.modifiers = Default::default();
                run.sub = 2;
                run.wait = 6;
            }
            2 => {
                let after = state(self);
                let c = changes(&run.before, &after);
                if c != "nothing changed" {
                    run.changed += 1;
                }
                eprintln!("keytest:     -> {c}");
                run.sub = 0;
                run.at += 1;
            }
            10 => {
                // Held for a while: what it does while held (Loop In: long enough for a loop).
                run.sub = 11;
                run.wait = if id == 0x300a { 200 } else { 30 };
            }
            11 => {
                let mid = state(self);
                eprintln!("keytest:     held -> {}", changes(&run.before, &mid));
                run.before = mid;
                if let Some((key, mods)) = run.key {
                    raw.events.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: mods });
                }
                raw.modifiers = Default::default();
                eprintln!("keytest:     released");
                run.sub = 2;
                run.wait = 8;
                run.changed += 1;
            }
            _ => run.sub = 0,
        }
    }

    /// Two tracks on decks 1 and 2 of PERFORMANCE, the presets chosen, and a user mapping
    /// with function keys for the commands the presets leave without one.
    fn keytest_setup(&mut self) {
        self.set_app_mode(1);
        self.prefs.perf_four = false;
        self.rebuild_list();
        let ids: Vec<i64> = self.list.iter().copied().filter(|t| self.track(*t).map(|x| !x.missing && x.duration > 60.0).unwrap_or(false)).take(2).collect();
        for (n, id) in ids.iter().enumerate() {
            self.with_deck(n as u8, |a| a.load_track_here(*id));
        }
        if let Some(i) = self.list.iter().position(|t| Some(t) == ids.first()) {
            self.set_anchor(i);
            self.selected.clear();
            self.selected.insert(self.list[i]);
        }
        let m = Mode::Performance;
        keys::copy(&mut self.prefs, m, "0000000000000", "keytest");
        keys::select(&mut self.prefs, m, "keytest");
        let rows = keys::commands(m);
        for &(mode, id, _, key, _) in STEPS {
            if mode == 1 && !key.is_empty() {
                if let Some(r) = rows.iter().find(|r| r.id == id) {
                    keys::assign(&mut self.prefs, m, "keytest", r, key);
                }
            }
        }
        // (The data folder keeps the settings of an earlier run: start from the defaults the
        // steps expect.)
        self.prefs.rb.gate_cue = false;
        self.prefs.rb.smart_cue = false;
        self.prefs.rb.auto_play = false;
        self.prefs.rb.library_protection = false;
        self.prefs.sampler.bank = 0;
        self.prefs.zoom = 2;
        self.prefs.bx.column_view = false;
        self.prefs.rec.level = 0.0;
        self.px.cfx_kind = 0;
        self.px.cfx_user = false;
        // A sample on slot 1 for the sampler's key.
        if let Some(id) = self.list.iter().copied().filter(|t| !ids.contains(t) && self.track(*t).map(|x| !x.missing).unwrap_or(false)).nth(0) {
            self.sampler_load(0, id);
        }
        // EXPORT uses rekordbox's Export preset, EDIT its own.
        self.prefs.rb.key_mapping[1] = String::new();
        eprintln!("keytest: decks 1 and 2 load {:?}; PERFORMANCE mapping \"keytest\" (Performance 1 with function keys added)", ids.iter().map(|t| self.track(*t).map(|x| x.title.clone()).unwrap_or_default()).collect::<Vec<_>>());
    }
}

/// EXPORT: the Export preset is chosen when EXPORT is entered (the key mapping is one per
/// mode family, so the run switches it at the mode change).
pub fn mode_mapping(app: &mut App, mode: u8) {
    if !std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "keytest")).unwrap_or(false) {
        return;
    }
    let want = match mode {
        0 => "0000000000030",
        2 => keys::EDIT_PRESET.0,
        _ => "keytest",
    };
    if keys::current(&app.prefs, Mode::of(mode)) != want && app.prefs.rb.key_mappings.contains_key("keytest") {
        keys::select(&mut app.prefs, Mode::of(mode), want);
    }
}

/// A point in deck 2's title area.
fn title_point(a: &App) -> egui::Pos2 {
    // (The deck's rectangle as the last frame drew it.)
    a.px.sampler.deck_rects[1].min + egui::vec2(150.0, 12.0)
}
