//! PREPDECK_SHOT_MIX=automix | mpl | sampler | mplshot | mplprefs | automixshot (with
//! --screenshot, and PREPDECK_BENCH=<frames> to keep the window up for the scripted runs):
//! operates Automix, MIX POINT LINK and the Sampler tree with real pointer events, one control
//! after another, and prints what each did ("mixtest: ..."). A stand-in for a hand test where
//! the screen cannot be taken over. The controls are found through the rectangles the panes
//! record when they draw, so the run follows the layout.

use crate::app::*;
use crate::automix::State;
use eframe::egui;
use egui::Pos2;

fn mode() -> String {
    std::env::var("PREPDECK_SHOT_MIX").unwrap_or_default()
}

/// Screenshot runs only: wait for a deck's audio.
fn wait_load(app: &mut App, d: u8) {
    app.with_deck(d, |a| {
        if let Some(rx) = a.deck.loading.take() {
            if let Ok(msg) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
                let (tx2, rx2) = crossbeam_channel::bounded(1);
                let _ = tx2.send(msg);
                a.deck.loading = Some(rx2);
                a.poll_load();
            }
        }
    });
}

fn playable(app: &App, n: usize) -> Vec<i64> {
    let mut v: Vec<&crate::model::Track> = app.tracks.iter().filter(|t| !t.missing && t.analyzed == 1 && t.duration > 90.0 && t.duration < 600.0 && t.bpm > 0.0).collect();
    v.sort_by(|a, b| a.duration.partial_cmp(&b.duration).unwrap());
    v.into_iter().take(n).map(|t| t.id).collect()
}

/// Put the app into the state the run (or the picture) starts from.
pub fn shot_setup(app: &mut App) {
    let m = mode();
    if m.is_empty() {
        return;
    }
    app.prefs.app_mode = 1;
    app.prefs.perf_four = false;
    app.prefs.perf_fx = false;
    app.prefs.perf_sampler = false;
    match m.as_str() {
        "automix" | "automixshot" => {
            crate::ui_browser_ext::set_right_panel(app, 0);
            app.prefs.mixx.automix = crate::automix::AutomixPrefs { pane: true, ..Default::default() };
            app.mixx.automix.loaded = true;
            app.mixx.automix.list = playable(app, 3);
            if m == "automixshot" {
                app.mixx.automix.settings_open = std::env::var("PREPDECK_SHOT_MIX_DIALOG").is_ok();
            }
        }
        "mplshot4" => {
            // The strip with four decks, each loaded.
            app.prefs.app_mode = 1;
            app.prefs.perf_four = true;
            app.poll_decks();
            app.prefs.mixx.mpl = Default::default();
            app.mixx.mpl.shown = true;
            let t = app.prefs.mixx.mpl.default_type;
            app.mixx.mpl.set_mode(t);
            app.mixx.mpl.guidance = 0;
            let ids = playable(app, 6);
            for d in 1..4u8 {
                if let Some(id) = ids.get(d as usize).copied() {
                    app.with_deck(d, |x| x.load_track_here(id));
                    wait_load(app, d);
                }
            }
            eprintln!("mixtest: mplshot4: {} decks, strip height {}", app.deck_count(), crate::ui_mpl::strip_height(app));
        }
        "mpl" | "mplshot" => {
            app.prefs.mixx.mpl = Default::default();
            app.mixx.mpl.shown = true;
            let t = app.prefs.mixx.mpl.default_type;
            app.mixx.mpl.set_mode(t);
            app.mixx.mpl.guidance = 0;
            if m == "mpl" {
                let a = app.deck.track;
                if let Some(id) = playable(app, 4).into_iter().find(|t| Some(*t) != a) {
                    app.with_deck(1, |x| x.load_track_here(id));
                    wait_load(app, 1);
                }
            }
        }
        "mplprefs" => {
            app.prefs.app_mode = 0;
            app.prefs.mixx.mpl = Default::default();
            app.dialogs.prefs_open = true;
            app.dialogs.prefs_tab = 7;
            app.ctx.data_mut(|d| d.insert_temp(egui::Id::new("prefs_subtab_7"), 4usize));
        }
        "sampler" => {
            app.side = SideMode::Sampler;
            app.view = View::Sampler(crate::sampler_lib::ALL_SAMPLES);
            app.prefs.perf_sampler = true;
        }
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum Act {
    Click(fn(&App) -> Option<Pos2>),
    Call(fn(&mut App)),
    /// Press at the first point, move to the second over a few frames, let go there.
    Drag(fn(&App) -> Option<Pos2>, fn(&App) -> Option<Pos2>),
    Until(fn(&App) -> bool, f32),
    Wait(f32),
}

struct Run {
    step: usize,
    since: u32,
    t0: Option<std::time::Instant>,
    done: bool,
}

static RUN: std::sync::Mutex<Run> = std::sync::Mutex::new(Run { step: 0, since: 0, t0: None, done: false });

fn center(r: Option<egui::Rect>) -> Option<Pos2> {
    r.map(|r| r.center())
}

fn amx_state(app: &App) -> String {
    let e = &app.mixx.automix.engine;
    let d = |n: usize| {
        let s = app.player.decks[n].lock();
        let pos = s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0);
        let p = &e.players[n];
        format!("deck {} {:?} row {:?} track {:?} {} pos {:.1}/{:.1} tempo {:+.2}% sync {} in {:?} out {:?}", n + 1, p.state, p.row.map(|r| r + 1), app.deck_of(n as u8).track, if s.playing { "playing" } else { "paused" }, pos, app.deck_of(n as u8).duration, (s.tempo - 1.0) * 100.0, app.deck_of(n as u8).x.beat_sync, (p.fade_in.start, p.fade_in.end), (p.fade_out.start, p.fade_out.end))
    };
    format!(
        "processing {} | {} | {} | crossfader {:.3} assign {:?} master {} locked {} curve {} (prefs {}) tempo dimmed {:?} | crossfade {:?} len {} | transitions {} | list {} pane {} dialog {} | repeat {} random {} | ask {:?} alert {:?}",
        e.processing,
        d(0),
        d(1),
        app.px.crossfader,
        [app.px.mix[0].assign, app.px.mix[1].assign],
        app.px.master + 1,
        crate::automix::crossfader_locked(app),
        crate::automix::curve_override(app).unwrap_or(app.prefs.deck.crossfader_curve),
        app.prefs.deck.crossfader_curve,
        [crate::automix::tempo_dimmed(app, 0), crate::automix::tempo_dimmed(app, 1)],
        e.crossfade.map(|c| (c.incoming + 1, c.ramp.len_ms)),
        e.crossfade_len,
        e.transitions,
        app.mixx.automix.list.len(),
        app.prefs.mixx.automix.pane,
        app.mixx.automix.settings_open,
        app.prefs.mixx.automix.repeat,
        app.prefs.mixx.automix.random,
        app.mixx.automix.ask,
        app.mixx.automix.alert
    )
}

fn mpl_state(app: &App) -> String {
    let m = &app.mixx.mpl;
    let d = |n: usize| {
        let s = app.player.decks[n].lock();
        let pos = s.audio.as_ref().map(|a| s.pos / a.sr as f64 * 1000.0).unwrap_or(0.0);
        let bars = if m.link_set[n] && m.mix_in[n] != -1 { format!(" bars {}", crate::mpl::mix_point_bars(&app.deck_of(n as u8).beats, m.mix_in[n] as f64 / 1000.0, pos / 1000.0)) } else { String::new() };
        format!("deck {} {} at {:.1} ms{} sync {}{} follows beats {}{}", n + 1, if s.playing { "playing" } else { "paused" }, pos, if s.muted { " muted" } else { "" }, app.deck_of(n as u8).x.beat_sync, if crate::mpl::bpm_sync_decks(app)[n] { " (BPM SYNC)" } else { "" }, s.sync_follow, bars)
    };
    let l = crate::mpl::ENGINE.lock().links[1];
    format!(
        "shown {} mode {} selected {:?} | MIX IN {:?} MIX OUT {:?} | pair[1][2] {} link set {:?} started {:?} reached {:?} | silent {:?} edit {:?} end {:?} | guidance {} | {} | {} | engine: active {} start_out {:.1} waiting {} started_at {:.3} | toast {:?}",
        m.shown,
        ["-", "MIX IN", "MIX OUT"][m.mode.min(2) as usize],
        m.selected.map(|d| d + 1),
        &m.mix_in[..2],
        &m.mix_out[..2],
        m.pair[0][1],
        &m.link_set[..2],
        &m.auto_started[..2],
        &m.reached[..2],
        &m.silent_on[..2],
        &m.silent_edit[..2],
        &m.silent_end[..2],
        m.guidance,
        d(0),
        d(1),
        l.active,
        l.start_out_ms,
        l.waiting,
        l.started_at_ms,
        app.toasts.last().map(|t| t.text.chars().take(70).collect::<String>())
    )
}

fn smp_state(app: &App) -> String {
    let lib = &app.mixx.lib;
    let bank = app.prefs.sampler.bank.min(3) as usize;
    let slots: Vec<String> = (0..16).filter_map(|k| app.prefs.sampler.slots.get(bank * 16 + k).and_then(|s| s.as_ref()).map(|s| format!("{}:{}{}", k + 1, s.name, if s.looped { "(loop)" } else { "" }))).collect();
    format!(
        "side sampler {} view {:?} | OSC list id {} | lists {:?} | counts {:?} | rows {} {:?} | bank {} slots [{}] | toast {:?}",
        app.side == SideMode::Sampler,
        app.view,
        app.prefs.mixx.lib.osc_list_id,
        lib.lists.iter().map(|l| format!("{}#{}{}", l.name, l.id, if l.attribute == crate::sampler_lib::ATTR_FOLDER { "/" } else { "" })).collect::<Vec<_>>(),
        { let mut c: Vec<_> = lib.counts.iter().collect(); c.sort(); c },
        lib.rows.len(),
        lib.rows.iter().take(4).map(|s| format!("{} {:.2}s bpm {} mode {}", s.name, s.duration, s.bpm100 as f64 / 100.0, s.mode)).collect::<Vec<_>>(),
        bank + 1,
        slots.join(", "),
        app.toasts.last().map(|t| t.text.chars().take(70).collect::<String>())
    )
}

/// A point on the on-screen crossfader (where `ui_perf` draws it: x 711..801 of the 1512 pt
/// mixer layout, on the mixer's bottom row).
fn xfader_at(app: &App, v: f32) -> Option<Pos2> {
    app.mixx.automix.ui.xfader.map(|r| egui::pos2(r.min.x + 6.0 + v * (r.width() - 12.0), r.center().y))
}

fn active_deck(app: &App) -> Option<usize> {
    (0..2).find(|d| app.mixx.automix.engine.players[*d].state == State::Active)
}

/// The time-skip switch: put the Active deck 2.5 s before its fade point.
fn skip_to_fade(app: &mut App) {
    let Some(a) = active_deck(app) else { return };
    let fo = app.mixx.automix.engine.players[a].fade_out;
    if fo.is_valid() {
        app.with_deck(a as u8, |x| x.seek(fo.start as f64 / 1000.0 - 2.5));
    }
}

fn lane_x(app: &App, d: usize, t: f64) -> Option<Pos2> {
    let (lr, v0, pps) = app.mixx.mpl.ui.lanes[d]?;
    Some(egui::pos2(lr.min.x + ((t - v0) * pps) as f32, lr.center().y))
}

fn deck_secs(app: &App, d: usize) -> f64 {
    let s = app.player.decks[d].lock();
    s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0)
}

fn steps(m: &str) -> Vec<(&'static str, Act)> {
    use Act::*;
    match m {
        "automix" => vec![
            ("pause the decks (the run is silent: master mute)", Call(|a| {
                for d in 0..2 {
                    a.with_deck(d, |x| x.set_playing(false));
                }
                if !a.px.muted {
                    a.toggle_mute();
                }
            })),
            ("Automix tile of the right strip (hides the pane)", Click(|a| center(a.mixx.automix.ui.tile))),
            ("Automix tile again (shows the pane)", Click(|a| center(a.mixx.automix.ui.tile))),
            ("Settings button", Click(|a| center(a.mixx.automix.ui.settings))),
            ("dialog: Random playback on", Click(|a| center(a.mixx.automix.ui.dialog[1]))),
            ("dialog: Random playback off", Click(|a| center(a.mixx.automix.ui.dialog[1]))),
            ("dialog: Repeat playback off", Click(|a| center(a.mixx.automix.ui.dialog[0]))),
            ("dialog: OK", Click(|a| center(a.mixx.automix.ui.dialog[4]))),
            ("Start", Click(|a| center(a.mixx.automix.ui.play))),
            ("wait: both decks loaded, deck 1 Active", Until(|a| active_deck(a).is_some() && a.mixx.automix.engine.crossfade_len >= 0, 30.0)),
            ("time skip: 2.5 s before the fade point", Call(skip_to_fade)),
            ("wait: the crossfade has started", Until(|a| a.mixx.automix.engine.crossfade.is_some(), 10.0)),
            ("1 s into the ramp", Wait(1.0)),
            ("drag the on-screen crossfader during the ramp (disabled: it does not move)", Drag(|a| xfader_at(a, a.px.crossfader), |a| xfader_at(a, 0.0))),
            ("4 s into the ramp", Wait(3.0)),
            ("wait: transition 1 done", Until(|a| a.mixx.automix.engine.transitions >= 1, 30.0)),
            ("wait: the next track is ready and the ranges are set", Until(|a| active_deck(a).is_some_and(|d| a.mixx.automix.engine.players[d].fade_out.is_valid()), 30.0)),
            ("2 s later (tempo returning)", Wait(2.0)),
            ("time skip: 2.5 s before the fade point", Call(skip_to_fade)),
            ("wait: the crossfade has started", Until(|a| a.mixx.automix.engine.crossfade.is_some(), 10.0)),
            ("wait: transition 2 done (the list is over: Automix ends)", Until(|a| a.mixx.automix.engine.transitions >= 2 || !a.automix_processing(), 30.0)),
            ("1 s later", Wait(1.0)),
            ("Start again", Click(|a| center(a.mixx.automix.ui.play))),
            ("1.5 s later", Wait(1.5)),
            ("Stop (the same button)", Click(|a| center(a.mixx.automix.ui.play))),
            ("Save button", Click(|a| center(a.mixx.automix.ui.save))),
            ("Clear button (asks)", Click(|a| center(a.mixx.automix.ui.clear))),
            ("question: No", Click(|a| a.mixx.automix.ui.buttons.get(1).map(|r| r.center()))),
            ("pause the decks", Call(|a| {
                for d in 0..2 {
                    a.with_deck(d, |x| x.set_playing(false));
                }
            })),
        ],
        "mpl" => vec![
            ("deck 1 plays from 20 s, deck 2 waits (the run is silent: master mute)", Call(|a| {
                if !a.px.muted {
                    a.toggle_mute();
                }
                a.with_deck(1, |x| {
                    x.set_playing(false);
                    x.prefs.quantize = false;
                });
                a.with_deck(0, |x| {
                    x.prefs.quantize = false;
                    x.seek(20.0);
                    x.set_playing(true);
                });
            })),
            ("LINK button of the top bar (hides the strip)", Click(|a| center(a.mixx.mpl.ui.rect("panel")))),
            ("LINK button again (shows it)", Click(|a| center(a.mixx.mpl.ui.rect("panel")))),
            ("MIX IN", Click(|a| center(a.mixx.mpl.ui.rect("mix_in")))),
            ("click deck 1's waveform in MIX IN mode (refused: it plays)", Click(|a| lane_x(a, 0, deck_secs(a, 0) + 15.0))),
            ("MIX OUT", Click(|a| center(a.mixx.mpl.ui.rect("mix_out")))),
            ("click deck 2's waveform in MIX OUT mode (refused: it is paused)", Click(|a| lane_x(a, 1, 10.0))),
            ("click deck 1's waveform (chooses the deck)", Click(|a| lane_x(a, 0, deck_secs(a, 0) + 15.0))),
            ("click deck 1's waveform 9 s ahead (MIX OUT point; the mode goes to MIX IN)", Click(|a| lane_x(a, 0, deck_secs(a, 0) + 9.0))),
            ("click deck 2's waveform (chooses the deck)", Click(|a| lane_x(a, 1, 10.0))),
            ("click deck 2's waveform at 3 s (MIX IN point)", Click(|a| lane_x(a, 1, 3.0))),
            ("SILENT", Click(|a| center(a.mixx.mpl.ui.rect("silent")))),
            ("SYNC on for deck 2 (BEAT SYNC; the MIX OUT point is off the beat)", Call(|a| {
                if !a.deck_of(1).x.beat_sync {
                    a.with_deck(1, |x| x.toggle_beat_sync());
                }
            })),
            ("MIX POINT LINK", Click(|a| center(a.mixx.mpl.ui.rect("link")))),
            ("wait: deck 2 started by the link", Until(|a| a.mixx.mpl.auto_started[1], 20.0)),
            ("1 s later", Wait(1.0)),
            ("wait: deck 1 passed its MIX OUT point", Until(|a| a.mixx.mpl.reached[0], 20.0)),
            ("MIX OUT, deck 2 (now playing), a point 6 s ahead", Call(|a| {
                a.mpl_set_mode(crate::mpl::MODE_OUT);
                let t = deck_secs(a, 1);
                a.mpl_wave_click(1, t, false);
                a.mpl_wave_click(1, t + 6.0, false);
            })),
            ("marker of deck 2's MIX OUT point (clears it)", Click(|a| center(a.mixx.mpl.ui.rect("mark_out1")))),
            ("Waveform Scrolling on", Click(|a| center(a.mixx.mpl.ui.rect("move")))),
            ("pause the decks", Call(|a| {
                for d in 0..2 {
                    a.with_deck(d, |x| x.set_playing(false));
                }
            })),
        ],
        "sampler" => vec![
            ("wait: the OSC pack of the installed rekordbox is imported", Until(|a| a.prefs.mixx.lib.osc_list_id != -1, 15.0)),
            ("tree: OSC SAMPLER (a folder: opens)", Click(|a| a.mixx.lib.ui.iter().find(|x| x.0 == "OSC SAMPLER").map(|x| x.1.center()))),
            ("tree: PRESET ONESHOT", Click(|a| a.mixx.lib.ui.iter().find(|x| x.0 == "PRESET ONESHOT").map(|x| x.1.center()))),
            ("menu action: Load the top 16 tracks into the current sampler bank", Call(|a| {
                if let View::Sampler(id) = a.view {
                    a.smplib_load_top16(id);
                }
            })),
            ("menu action: Create New Sampler List, then Add To Sampler List with two tracks", Call(|a| {
                let id = a.smplib_create(0);
                a.mixx.lib.rename = None;
                let ids: Vec<i64> = a.tracks.iter().filter(|t| !t.missing).take(2).map(|t| t.id).collect();
                let s = a.smplib_samples_for_tracks(&ids);
                a.smplib_add(id, s, false);
            })),
            ("the same two tracks again (asks about duplicates)", Call(|a| {
                let id = a.mixx.lib.lists.iter().find(|l| l.name.starts_with("Untitled Sampler List")).map(|l| l.id).unwrap_or(0);
                let ids: Vec<i64> = a.tracks.iter().filter(|t| !t.missing).take(2).map(|t| t.id).collect();
                let s = a.smplib_samples_for_tracks(&ids);
                a.smplib_add(id, s, false);
                eprintln!("mixtest:   question pending: {}", a.mixx.lib.ask_dup.is_some());
                a.mixx.lib.ask_dup = None;
            })),
            ("tree: Untitled Sampler List", Click(|a| a.mixx.lib.ui.iter().find(|x| x.0.starts_with("Untitled Sampler List")).map(|x| x.1.center()))),
            ("tree: All Samples", Click(|a| a.mixx.lib.ui.iter().find(|x| x.0 == "All Samples").map(|x| x.1.center()))),
            ("tree: Capture", Click(|a| a.mixx.lib.ui.iter().find(|x| x.0 == "Capture").map(|x| x.1.center()))),
            ("menu action: Delete Sampler List (the untitled one)", Call(|a| {
                if let Some(id) = a.mixx.lib.lists.iter().find(|l| l.name.starts_with("Untitled Sampler List")).map(|l| l.id) {
                    a.smplib_delete(id);
                }
            })),
            ("tree: PRESET ONESHOT", Click(|a| a.mixx.lib.ui.iter().find(|x| x.0 == "PRESET ONESHOT").map(|x| x.1.center()))),
            ("wait: the pack samples' lengths are read", Until(|a| a.mixx.lib.measure_job.is_none() && a.mixx.lib.measured_all && a.mixx.lib.rows.iter().all(|s| s.duration > 0.0), 20.0)),
            ("slots noted on screen", Call(|a| {
                eprintln!("mixtest:   targets {:?} slots {:?}", a.mixx.lib.ui.iter().map(|x| x.0.clone()).collect::<Vec<_>>(), a.mixx.lib.slots_ui.iter().map(|x| x.0).collect::<Vec<_>>());
            })),
            ("drag the list's second row onto slot 6 (empty)", Drag(|a| a.mixx.lib.ui.iter().find(|x| x.0 == "row1").map(|x| x.1.center()), |a| {
                let idx = a.prefs.sampler.bank.min(3) as usize * 16 + 5;
                a.mixx.lib.slots_ui.iter().find(|x| x.0 == idx).map(|x| x.1.center())
            })),
            ("Track Type of a collection track set to Loop in the track menu: the sampler list's row says Loop too", Call(|a| {
                let ids: Vec<i64> = a.tracks.iter().filter(|t| !t.missing).take(1).map(|t| t.id).collect();
                let s = a.smplib_samples_for_tracks(&ids);
                a.set_track_type(&ids, 2);
                let row_mode = crate::sampler_lib::all_samples(&a.db.conn).unwrap_or_default().into_iter().find(|x| s.contains(&x.id)).map(|x| x.mode);
                eprintln!("mixtest:   track type {} / sample row mode {:?} (Loop = {})", a.track_type(ids[0]), row_mode, crate::sampler_lib::MODE_LOOP);
                // ... and the track loads into slot 15 as a loop.
                let idx = a.prefs.sampler.bank.min(3) as usize * 16 + 14;
                a.sampler_eject(idx);
                a.sampler_load(idx, ids[0]);
                eprintln!("mixtest:   slot 15 loops: {:?}", a.prefs.sampler.slots.get(idx).and_then(|x| x.as_ref()).map(|x| x.looped));
                a.set_track_type(&ids, 0);
            })),
        ],
        _ => vec![],
    }
}

impl App {
    pub fn shot_mix_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        let m = mode();
        let list = steps(&m);
        if list.is_empty() || frame < 75 {
            return;
        }
        let state = |a: &App| match m.as_str() {
            "automix" => amx_state(a),
            "mpl" => mpl_state(a),
            _ => smp_state(a),
        };
        let mut run = RUN.lock().unwrap();
        if run.done {
            return;
        }
        let Some((name, act)) = list.get(run.step).map(|x| (x.0, x.1)) else {
            run.done = true;
            eprintln!("mixtest: done ({} steps)", list.len());
            // Take the picture now instead of waiting out the frames asked for.
            let bench: u32 = std::env::var("PREPDECK_BENCH").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
            if let Some((path, f)) = self.screenshot.clone() {
                self.screenshot = Some((path, f.max(118 + bench)));
            }
            return;
        };
        let f = run.since;
        run.since += 1;
        let t0 = *run.t0.get_or_insert_with(std::time::Instant::now);
        let mut next = false;
        match act {
            Act::Click(target) => {
                static AT: std::sync::Mutex<Option<Pos2>> = std::sync::Mutex::new(None);
                let mut at = AT.lock().unwrap();
                match f {
                    0 => {
                        *at = target(self);
                        match *at {
                            Some(p) => raw.events.push(egui::Event::PointerMoved(p)),
                            None => {
                                eprintln!("mixtest: {name}: NOT ON SCREEN");
                                next = true;
                            }
                        }
                    }
                    2 | 3 => {
                        if let Some(p) = *at {
                            raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: f == 2, modifiers: Default::default() });
                        }
                    }
                    5 => raw.events.push(egui::Event::PointerMoved(egui::pos2(700.0, 5.0))),
                    9 => {
                        eprintln!("mixtest: {name} [click at {:.0}, {:.0}]: {}", at.map(|p| p.x).unwrap_or(0.0), at.map(|p| p.y).unwrap_or(0.0), state(self));
                        next = true;
                    }
                    _ => {}
                }
            }
            Act::Drag(from, to) => {
                static AT: std::sync::Mutex<Option<(Pos2, Pos2)>> = std::sync::Mutex::new(None);
                let mut at = AT.lock().unwrap();
                let btn = |p: Pos2, down: bool| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: down, modifiers: Default::default() };
                match f {
                    0 => {
                        *at = from(self).zip(to(self));
                        match *at {
                            Some((a, _)) => raw.events.push(egui::Event::PointerMoved(a)),
                            None => {
                                eprintln!("mixtest: {name}: NOT ON SCREEN");
                                next = true;
                            }
                        }
                    }
                    2 => {
                        if let Some((a, _)) = *at {
                            raw.events.push(btn(a, true));
                        }
                    }
                    3..=14 => {
                        if let Some((a, b)) = *at {
                            raw.events.push(egui::Event::PointerMoved(a + (b - a) * ((f - 2) as f32 / 12.0)));
                        }
                    }
                    16 => {
                        if let Some((_, b)) = *at {
                            raw.events.push(btn(b, false));
                        }
                    }
                    18 => raw.events.push(egui::Event::PointerMoved(egui::pos2(700.0, 5.0))),
                    24 => {
                        eprintln!("mixtest: {name}: {}", state(self));
                        next = true;
                    }
                    _ => {}
                }
            }
            Act::Call(func) => {
                if f == 0 {
                    func(self);
                } else if f >= 3 {
                    eprintln!("mixtest: {name}: {}", state(self));
                    next = true;
                }
            }
            Act::Until(cond, secs) => {
                let el = t0.elapsed().as_secs_f32();
                if cond(self) || el > secs {
                    eprintln!("mixtest: {name}{} [{el:.1} s]: {}", if cond(self) { "" } else { " — TIMED OUT" }, state(self));
                    next = true;
                }
            }
            Act::Wait(secs) => {
                if t0.elapsed().as_secs_f32() >= secs {
                    eprintln!("mixtest: {name}: {}", state(self));
                    next = true;
                }
            }
        }
        if next {
            run.step += 1;
            run.since = 0;
            run.t0 = None;
        }
    }
}
