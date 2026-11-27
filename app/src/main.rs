//! Recordbox: local DJ library, track analysis, set preparation and Pioneer-compatible USB export.

use eframe::egui;
mod analysis;
mod app;
mod app_hooks;
mod audio;
mod automix;
mod backup;
mod brand;
mod cfx;
mod cfx_space;
mod browser_ext;
mod collection_cleanup;
mod db;
mod cue_tools;
mod deck;
mod demo;
mod dupes;
mod device_ext;
mod onelibrary;
mod edit_audio;
mod edit_model;
mod edit_project;
mod edit_render;
mod edit_selftest;
mod export;
mod fx;
mod fx_core;
mod mfx;
mod icons;
mod icons_player;
mod icons_src;
mod importer;
mod itunes;
mod jobs;
mod keycmd_data;
mod keycmd;
mod keytest_selftest;
mod key;
mod library_ext;
mod menus;
mod midi;
mod midi_csv;
mod midi_map;
mod mix_selftest;
mod mixer;
mod groove;
mod ui_groove;
mod pad_audio;
mod pads;
mod pads_ext;
mod pads_selftest;
mod mpl;
mod perf_selftest;
mod sampler;
mod sampler_ext;
mod sampler_lib;
mod sequencer;
mod ui_sampler;
mod model;
mod manual;
mod phrase;
mod phrase_rb;
mod phrase_rb_b;
mod phrase_rb_c;
mod phrase_rb_d;
mod phrase_rb_d1;
mod phrase_rb_d2;
mod phrase_rb_d3;
mod phrase_rb_d4;
mod phrase_rb_d5;
mod phrase_rb_d6;
mod phrase_rb_d7;
mod phrase_rb_d8;
mod phrase_rb_d9;
mod phrase_rb_d10;
mod phrase_rb_d11;
mod phrase_rb_d12;
mod phrase_rb_d13;
mod phrase_rb_d14;
mod phrase_rb_dsp;
mod playlist_files;
mod preview;
mod player_ext;
mod prefs_bridge;
mod prefs_ext;
mod prefs_font;
mod prefs_rb;
mod prefs_selftest;
mod prefs_shell;
mod rbimport;
mod recorder;
mod stems;
mod smart;
mod playlist_art;
mod theme;
mod traffic;
mod ui_automix;
mod ui_browser;
mod ui_browser_ext;
mod ui_dialogs;
mod ui_edit;
mod ui_edit_view;
mod ui_kit;
mod ui_library_ext;
mod ui_midi;
mod ui_mpl;
mod ui_sampler_lib;
mod ui_player;
mod ui_prefs;
mod ui_sync;
mod ui_rec;
mod ui_prefs_adv;
mod ui_prefs_item;
mod ui_prefs_keys;
mod ui_prefs_perf;
mod ui_prefs_view;
mod ui_fx;
mod ui_pads;
mod ui_padfx;
mod ui_perf;
mod ui_perf_layouts;
mod ui_player2;
mod xmlexport;
#[cfg(target_os = "linux")]
mod x11_fullscreen;
mod vocal_rb;
#[cfg(all(test, target_os = "macos"))]
mod vocal_rb_mp3;
mod vocal_rb_net;

use app::App;

struct Gui {
    app: App,
    pending_docked_size: Option<egui::Vec2>,
}

const MAIN_MIN_SIZE: egui::Vec2 = egui::vec2(1100.0, 700.0);

struct DetachedLibrary {
    builder: egui::ViewportBuilder,
    docked_size: Option<egui::Vec2>,
    deck_window_height: f32,
    pending_fullscreen: Option<PendingFullscreen>,
}

struct PendingFullscreen {
    origin: egui::ViewportId,
    fullscreen: bool,
    root_pass: u64,
}

impl PendingFullscreen {
    fn finish(self, ctx: &egui::Context) {
        // In the immediate library callback, eframe has refreshed both native windows.
        let still_active = ctx.input(|i| {
            let Some(root) = i.raw.viewports.get(&egui::ViewportId::ROOT) else { return false };
            let Some(library) = i.raw.viewports.get(&DetachedLibrary::id()) else { return false };
            let origin = if self.origin == egui::ViewportId::ROOT { root } else { library };
            (root.focused == Some(true) || library.focused == Some(true))
                && !root.close_requested() && !library.close_requested()
                && origin.minimized != Some(true) && origin.fullscreen != Some(self.fullscreen)
        });
        if still_active {
            ctx.send_viewport_cmd_to(self.origin, egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
    }
}

impl DetachedLibrary {
    const TITLE: &'static str = "recordbox — Library";

    fn id() -> egui::ViewportId {
        egui::ViewportId::from_hash_of("recordbox_library")
    }

    fn open(ctx: &egui::Context, player_height: f32) -> Self {
        let size = ctx.screen_rect().size();
        let docked_size = windowed(ctx).then_some(size);
        let deck_window_height = player_height + 56.0; // Header and status bar.
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(MAIN_MIN_SIZE.x, deck_window_height)));
        if docked_size.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(size.x, deck_window_height)));
        }
        Self {
            builder: egui::ViewportBuilder::default().with_title(Self::TITLE)
                .with_inner_size([size.x, (size.y - deck_window_height).max(400.0)])
                .with_min_inner_size([900.0, 360.0])
                .with_fullscreen(main_fullscreen(ctx)),
            docked_size,
            deck_window_height,
            pending_fullscreen: None,
        }
    }

    fn fit_decks(&mut self, ctx: &egui::Context, player_height: f32) {
        let height = player_height + 56.0;
        if self.deck_window_height != height {
            let size = ctx.screen_rect().size();
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(MAIN_MIN_SIZE.x, height)));
            // Follow deck layout changes only while the user has kept the compact height.
            if windowed(ctx) && (size.y - self.deck_window_height).abs() <= 2.0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(size.x, height)));
            }
            self.deck_window_height = height;
        }
    }

    fn reattach(self, ctx: &egui::Context) -> Option<egui::Vec2> {
        ctx.request_repaint();
        if self.docked_size.is_some() && !windowed(ctx) {
            // The WM must first be allowed to restore its saved, deck-height window.
            // Raising the minimum now can prevent it from leaving maximized mode on X11.
            return self.docked_size;
        }
        restore_docked_size(ctx, self.docked_size);
        None
    }
}

fn restore_docked_size(ctx: &egui::Context, size: Option<egui::Vec2>) {
    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(MAIN_MIN_SIZE));
    if let Some(size) = size {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    }
}

fn windowed(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized != Some(true)) && !main_fullscreen(ctx)
}

fn main_fullscreen(ctx: &egui::Context) -> bool {
    #[cfg(target_os = "linux")]
    if let Some(fullscreen) = x11_fullscreen::main_fullscreen(ctx) {
        return fullscreen;
    }
    ctx.input(|i| i.viewport().fullscreen == Some(true))
}

fn window_input(app: &mut App, ctx: &egui::Context) {
    menus::apply_edit_commands(app, ctx);
    window_fullscreen(&app.prefs, ctx, app.library_window.as_mut());
    if ctx.input(|i| i.focused || i.events.contains(&egui::Event::WindowFocused(false))) {
        shortcuts(app, ctx);
    }
}

fn library_panel(app: &mut App, ctx: &egui::Context) {
    egui::CentralPanel::default().frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
        ui_browser::browser(app, ui);
    });
}

impl eframe::App for Gui {
    /// PREPDECK_SHOT_DRAG (with --screenshot): drag hot cue A 100 px to the right with real
    /// pointer events, to check marker dragging end to end; the result goes to stderr.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        ui_groove::frame_hook(&mut self.app, raw);
        app_hooks::raw_input(&mut self.app, raw);
        let Some((_, frame)) = self.app.screenshot.clone() else { return };
        self.app.shot_player_events(frame, raw);
        self.app.shot_perf_events(frame, raw);
        self.app.shot_midi_events(frame, raw);
        self.app.shot_rec_events(frame, raw);
        self.app.shot_edit_events(frame, raw);
        self.app.shot_pads_events(frame, raw);
        crate::ui_perf_layouts::shot_layout_events(&mut self.app, frame, raw);
        self.app.shot_mix_events(frame, raw);
        self.app.shot_prefs_events(frame, raw);
        self.app.shot_key_events(frame, raw);
        if std::env::var("PREPDECK_SHOT_DRAG").is_err() {
            return;
        }
        let Some((r, pos, pps)) = self.app.deck.detail_map else { return };
        let Some(a) = self.app.deck.cues.iter().find(|c| c.kind == model::CueKind::Hot && c.slot == 0).cloned() else { return };
        let ax = r.center().x + ((a.start - pos) * pps) as f32 + 2.0;
        let y = r.min.y - 7.0;
        static START: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
        let press = |down: bool, x: f32| egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Primary, pressed: down, modifiers: Default::default() };
        let mv = |x: f32| egui::Event::PointerMoved(egui::pos2(x, y));
        match frame {
            80 => {
                let x0 = *START.get_or_init(|| ax);
                eprintln!("drag: hot cue A at {:.3} s, grabbing at x {x0:.1}", a.start);
                raw.events.push(mv(x0));
            }
            81 => raw.events.push(press(true, *START.get().unwrap_or(&ax))),
            82..=91 => {
                let x0 = *START.get().unwrap_or(&ax);
                raw.events.push(mv(x0 + 10.0 * (frame - 81) as f32));
            }
            92 => raw.events.push(press(false, START.get().copied().unwrap_or(ax) + 100.0)),
            100 => {
                let saved = self.app.db.cues(a.track_id).unwrap_or_default().into_iter().find(|c| c.kind == model::CueKind::Hot && c.slot == 0);
                eprintln!("drag: after: deck {:.3} s, saved {:?}, undo steps {}, 100 px = {:.3} s", a.start, saved.map(|c| c.start), self.app.undo_len(), 100.0 / pps);
            }
            101 => {
                self.app.undo_edit();
                let saved = self.app.db.cues(a.track_id).unwrap_or_default().into_iter().find(|c| c.kind == model::CueKind::Hot && c.slot == 0);
                eprintln!("drag: after Undo: deck {:.3} s, saved {:?}", self.app.deck.cues.iter().find(|c| c.id == a.id).map(|c| c.start).unwrap_or(-1.0), saved.map(|c| c.start));
            }
            // Second drag, cancelled with Esc while the button is still down.
            102 => raw.events.push(mv(ax)),
            103 => raw.events.push(press(true, ax)),
            104..=108 => raw.events.push(mv(ax + 15.0 * (frame - 103) as f32)),
            109 => eprintln!("drag: during second drag: deck {:.3} s", a.start),
            110 => raw.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }),
            111 => raw.events.push(mv(ax + 120.0)),
            112 => raw.events.push(press(false, ax + 120.0)),
            114 => {
                let saved = self.app.db.cues(a.track_id).unwrap_or_default().into_iter().find(|c| c.kind == model::CueKind::Hot && c.slot == 0);
                eprintln!("drag: after Esc: deck {:.3} s, saved {:?}, undo steps {}", a.start, saved.map(|c| c.start), self.app.undo_len());
            }
            _ => {}
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let bench_t0 = std::time::Instant::now();
        let app = &mut self.app;
        if self.pending_docked_size.is_some() && windowed(ctx) {
            restore_docked_size(ctx, self.pending_docked_size.take());
        }
        // PREPDECK_BENCH: name any step that holds a frame up for more than 4 ms.
        macro_rules! step {
            ($e:expr) => {{
                let t = std::time::Instant::now();
                $e;
                if bench_on() && t.elapsed().as_millis() >= 4 {
                    eprintln!("bench: slow step {} {:.1} ms", stringify!($e), t.elapsed().as_secs_f32() * 1000.0);
                }
            }};
        }
        step!(app.poll_import());
        step!(app.poll_analysis());
        step!(app.poll_load());
        step!(midi::poll(app));
        step!(prefs_bridge::frame(app));
        step!(app.poll_decks());
        step!(automix::poll(app, ctx));
        step!(app.poll_export());
        step!(app.poll_rekordbox_sync());
        step!(app.poll_stems());
        step!(app.poll_stem_waves());
        step!(app.poll_phrases());
        step!(app.poll_vocal());
        step!(app.poll_key_job());
        step!(app.poll_audio_device());
        step!(app.poll_relocate());
        step!(app.poll_rec());
        step!(backup::poll(app));
        step!(menus::poll(app));
        step!(menus::sync(app));
        step!(ui_edit::frame(app, ctx));
        window_input(app, ctx);
        // Preferences > View > Tooltips off: no hover text anywhere, including the ones drawn
        // with egui's own `on_hover_text` (a delay that never runs out; infinity would panic
        // in egui's repaint timer).
        let tip_delay = if app.prefs.tooltips { 0.5 } else { 1.0e9 };
        if ctx.style().interaction.tooltip_delay != tip_delay {
            ctx.all_styles_mut(|s| s.interaction.tooltip_delay = tip_delay);
        }

        let recording_strip = !app.performance() && app.prefs.rec.panel_export;
        let deck_height = if ui_edit::is_edit(app) { 293.0 } else { app.player_height() };
        let deck_height = deck_height + if recording_strip { ui_rec::EXPORT_H } else { 0.0 };
        let toggle_library = egui::TopBottomPanel::top("header").exact_height(28.0).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG1)).show(ctx, |ui| {
            ui_browser::header_bar(app, ui)
        }).inner;
        let menu_toggle = std::mem::take(&mut app.toggle_library_window);
        if toggle_library || menu_toggle {
            if let Some(window) = app.library_window.take() {
                self.pending_docked_size = window.reattach(ctx);
            } else if !ctx.embed_viewports() {
                let mut window = DetachedLibrary::open(ctx, deck_height);
                window.docked_size = window.docked_size.or(self.pending_docked_size.take());
                app.library_window = Some(window);
            }
        }
        egui::TopBottomPanel::bottom("footer").exact_height(28.0).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| {
            ui_browser::footer(app, ui);
        });
        // EXPORT mode's recording panel, a strip under the top bar.
        if recording_strip {
            egui::TopBottomPanel::top("recstrip").exact_height(ui_rec::EXPORT_H).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG2)).show(ctx, |ui| ui_rec::export_strip(app, ui));
        }
        let decks_visible = !ui_edit::is_edit(app) && (!app.full_browser() || app.performance());
        if ui_edit::is_edit(app) {
            step!(ui_edit_view::top_panel(app, ctx));
        } else if decks_visible {
            // The header is 28 pt and 1 Player / 2 Players 256 pt tall with no rule between them
            // (measured on rekordbox: the player ends 284 pt below the window top); the Simple
            // Player is one 58 pt row.
            let h = app.player_height();
            let perf = app.performance();
            let draw_players = |app: &mut App| {
                egui::TopBottomPanel::top(egui::Id::new(("player", if perf { 9 } else { app.prefs.player_layout.min(3) }))).exact_height(h).show_separator_line(false).frame(egui::Frame::default().fill(theme::BG)).show(ctx, |ui| match app.prefs.player_layout {
                    _ if perf => step!(ui_perf::performance(app, ui)),
                    1 | 3 => step!(ui_player2::two_players(app, ui)),
                    2 => step!(ui_player2::simple_player(app, ui)),
                    _ => step!(ui_player::player_panel(app, ui)),
                });
            };
            if app.library_window.is_some() {
                app.with_detached_player_drag(ctx, draw_players);
            } else {
                draw_players(app);
            }
        }
        let library_id = DetachedLibrary::id();
        if let Some(window) = &mut app.library_window {
            window.fit_decks(ctx, deck_height);
            // An egui layout retry still belongs to the same native update. Only a new update
            // can finish a transition after the peer's fullscreen command has been delivered.
            let root_pass = ctx.cumulative_pass_nr();
            let mut pending = if ctx.output(|o| o.num_completed_passes == 0) {
                window.pending_fullscreen.take_if(|request| request.root_pass < root_pass)
            } else {
                None
            };
            let reattach = ctx.show_viewport_immediate(library_id, window.builder.clone(), |child, class| {
                if class == egui::ViewportClass::Embedded || child.input(|i| i.viewport().close_requested()) {
                    return true;
                }
                if let Some(request) = pending.take() {
                    request.finish(child);
                }
                window_input(app, child);
                step!(library_panel(app, child));
                step!(ui_dialogs::library_dialogs(app, child));
                false
            });
            if reattach {
                if let Some(window) = app.library_window.take() {
                    self.pending_docked_size = window.reattach(ctx);
                }
            }
        }
        if app.library_window.is_none() && !ui_edit_view::hides_browser(app) {
            step!(library_panel(app, ctx));
        }
        step!(ui_prefs::prefs_window(app, ctx));
        if app.library_window.is_none() {
            step!(ui_dialogs::library_dialogs(app, ctx));
        } else if app.ext.cleanup.is_some() {
            egui::Modal::new("detached_collection_cleanup".into()).show(ctx, |ui| {
                ui.label("Collection cleanup is open in the library browser.");
                if ui.button("Show library").clicked() {
                    ctx.send_viewport_cmd_to(library_id, egui::ViewportCommand::Focus);
                }
            });
        }
        collection_cleanup::ui::guard_app_close(app, ctx);
        app.update_transport(ctx, decks_visible);
        // Preferences rows of other files' features take effect (prefs_bridge.rs).
        if app.dialogs.prefs_open || app.ctx.cumulative_pass_nr() < 3 {
            app.prefs_bridge();
        }
        step!(dupes::windows(app, ctx));
        step!(cue_tools::windows(app, ctx));
        step!(ui_midi::window(app, ctx));
        step!(ui_rec::windows(app, ctx));

        if app.analysis_total > 0 || app.importing.is_some() || app.export_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        // PREPDECK_BENCH=<frames> (with --screenshot): play the loaded track muted for that many
        // frames before the picture and print how long building each frame took.
        let bench: u32 = std::env::var("PREPDECK_BENCH").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        if bench > 0 {
            if let Some((_, frames)) = app.screenshot.clone() {
                static TIMES: std::sync::Mutex<Vec<f32>> = std::sync::Mutex::new(Vec::new());
                // PREPDECK_BENCH_MUTE=<frame>: mute the vocals at that frame (the stem waveform).
                if std::env::var("PREPDECK_BENCH_MUTE").ok().and_then(|v| v.parse::<u32>().ok()) == Some(frames) {
                    eprintln!("bench: stems ready at the mute {}", app.player.state.lock().stems.is_some());
                    app.player.state.lock().stem_on[3] = false;
                }
                if frames == 60 {
                    if !app.px.muted {
                        app.toggle_mute();
                    }
                    if !app.playing() {
                        app.play_pause();
                    }
                }
                let mut t = TIMES.lock().unwrap();
                if frames > 90 {
                    t.push(bench_t0.elapsed().as_secs_f32() * 1000.0);
                }
                if frames == 119 + bench && !t.is_empty() {
                    let mut v = t.clone();
                    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let avg = v.iter().sum::<f32>() / v.len() as f32;
                    eprintln!("bench: {} frames, update avg {:.2} ms, median {:.2}, p95 {:.2}, max {:.2}", v.len(), avg, v[v.len() / 2], v[v.len() * 95 / 100], v[v.len() - 1]);
                    use std::sync::atomic::Ordering::Relaxed;
                    eprintln!("bench: stems ready {}, separating {:?}, stem error {:?}", app.player.state.lock().stems.is_some(), app.stem.separating, app.stem.error_for(app.deck.track));
                    eprintln!("bench: mixer meters deck 1 {:.1} dB, master {:.1} dB, limiter on {}", app.px.mix[0].shown_db[0], app.px.master_db[0], audio::LIMITER_ON.load(Relaxed));
                    eprintln!("bench: audio callback found a deck held {} times, gave a buffer up {} times", audio::AUDIO_CONTENDED.load(Relaxed), audio::AUDIO_DROPPED.load(Relaxed));
                }
            }
        }
        // --screenshot: draw ~120 frames, then save the window picture and quit.
        if let Some((path, frames)) = app.screenshot.clone() {
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
            });
            if let Some(img) = shot {
                let [w, h] = img.size;
                let mut buf = image::RgbaImage::new(w as u32, h as u32);
                for (i, px) in img.pixels.iter().enumerate() {
                    let [r, g, b, a] = px.to_srgba_unmultiplied();
                    buf.put_pixel((i % w) as u32, (i / w) as u32, image::Rgba([r, g, b, a]));
                }
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = buf.save(&path);
                std::process::exit(0);
            }
            // Taken once the launch-time rekordbox sync has finished, so the picture shows the
            // synced library (up to ~20 s).
            static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if frames >= 120 + bench && (app.rb_job.is_none() || frames >= 1200 + bench) && !ASKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            if frames > 1500 + bench {
                std::process::exit(1);
            }
            app.screenshot = Some((path, frames + 1));
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        prefs_bridge::on_exit(&mut self.app);
        self.app.save_prefs();
        let _ = self.app.db.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
}

fn bench_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("PREPDECK_BENCH").is_ok())
}

fn recordbox_actions() -> Vec<&'static str> {
    app::ACTIONS.iter().map(|a| a.0).collect()
}

/// F11 also works inside a search field. An explicit key binding for another action wins.
fn window_fullscreen(prefs: &app::Prefs, ctx: &egui::Context, library: Option<&mut DetachedLibrary>) {
    let fullscreen = ctx.input_mut(|i| {
        if !i.focused {
            return None;
        }
        let mut toggle = false;
        i.events.retain(|event| {
            if let egui::Event::Key { key: egui::Key::F11, pressed: true, repeat, modifiers, .. } = event {
                if modifiers.is_none() && !prefs.keymap.iter().any(|(action, key)| action != "fullscreen" && key == "F11") {
                    toggle |= !repeat;
                    return false;
                }
            }
            true
        });
        toggle.then(|| !i.viewport().fullscreen.unwrap_or(false))
    });
    if let Some(fullscreen) = fullscreen {
        set_window_fullscreen(ctx, ctx.viewport_id(), library, fullscreen);
    }
}

fn set_window_fullscreen(ctx: &egui::Context, origin: egui::ViewportId, library: Option<&mut DetachedLibrary>, fullscreen: bool) {
    #[cfg(target_os = "linux")]
    if x11_fullscreen::toggle(ctx, origin, library.is_some()) {
        if let Some(window) = library {
            window.pending_fullscreen = None;
        }
        return;
    }
    let root = egui::ViewportId::ROOT;
    let library_id = DetachedLibrary::id();
    if let Some(window) = library.filter(|_| origin == root || origin == library_id) {
        let peer = if origin == root { library_id } else { root };
        window.pending_fullscreen = None;
        let peer_needs_change = ctx.input(|i| i.raw.viewports.get(&peer).is_some_and(|v| !v.close_requested() && v.fullscreen != Some(fullscreen)));
        if peer_needs_change {
            // Native backends can activate a window entering fullscreen. eframe does not order
            // commands across viewports in one update, so transition the initiating window last.
            ctx.send_viewport_cmd_to(peer, egui::ViewportCommand::Fullscreen(fullscreen));
            window.pending_fullscreen = Some(PendingFullscreen { origin, fullscreen, root_pass: ctx.cumulative_pass_nr_for(root) });
            ctx.request_repaint_of(root);
            return;
        }
    }
    ctx.send_viewport_cmd_to(origin, egui::ViewportCommand::Fullscreen(fullscreen));
}

const HOT_CUE_DIGITS: [egui::Key; 8] = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8];

/// A supported logical symbol (e.g. `!` or `/`) can mask the shifted number key. Only
/// fall back for a clear-cue binding, and let an existing logical binding take precedence.
fn clear_hot_key(prefs: &app::Prefs, performance: bool, key: egui::Key, digit: egui::Key, mods: &egui::Modifiers) -> egui::Key {
    if !mods.shift || HOT_CUE_DIGITS.contains(&key) || matches!(key, egui::Key::Num0 | egui::Key::Num9) {
        return key;
    }
    let logical = app::Shortcut::from_event(key, mods);
    let physical = app::Shortcut { key: digit, ..logical };
    // Shift can turn a keypad number into its navigation key (e.g. keypad 2 => Down).
    // The dedicated arrow keys have a different physical key. Honor explicit navigation bindings.
    use egui::Key::*;
    let keypad_navigation = matches!((digit, key), (Num1, End) | (Num2, ArrowDown) | (Num3, PageDown) | (Num4, ArrowLeft) | (Num6, ArrowRight) | (Num7, Home) | (Num8, ArrowUp));
    let mut clear = false;
    for (action, _, _, _) in app::ACTIONS {
        if !performance && sampler_ext::is_sampler_action(action) {
            continue;
        }
        let binding = prefs.shortcut_for(action);
        if binding.is_some_and(|sc| sc.key == key && sc.mods_match(mods, matches!(action, "jump_back" | "jump_fwd")))
            && (!keypad_navigation || prefs.keymap.contains_key(action)) {
            return key;
        }
        if binding == Some(physical) {
            if clear || !action.starts_with("clear_hot") {
                return key;
            }
            clear = true;
        }
    }
    if clear { digit } else { key }
}

/// egui reports repeats by logical key. Shift can change that key while a digit is held.
/// Keep the original press through its repeat and release; raw input stays available to editors.
fn shortcut_key_events(app: &App, ctx: &egui::Context) -> Vec<egui::Event> {
    #[derive(Clone, Copy, Default)]
    enum DigitState { #[default] Unknown, Released, Held(egui::Key, egui::Modifiers) }
    type HeldDigits = [DigitState; 8];
    let id = egui::Id::new((ctx.viewport_id(), "held_hot_cue_digits"));
    let mut events: Vec<_> = ctx.input(|i| i.events.iter().filter(|e| matches!(e, egui::Event::Key { .. })).cloned().collect());
    if !ctx.input(|i| i.focused) {
        ctx.data_mut(|d| d.remove::<HeldDigits>(id));
    } else if !events.is_empty() {
        ctx.data_mut(|d| {
            let held = d.get_temp_mut_or_default::<HeldDigits>(id);
            for event in &mut events {
                let egui::Event::Key { key, physical_key: Some(digit), modifiers, pressed, repeat } = event else { continue };
                let Some(slot) = HOT_CUE_DIGITS.iter().position(|n| n == digit) else { continue };
                if *pressed {
                    match held[slot] {
                        DigitState::Held(initial_key, initial_mods) => {
                            *repeat = true;
                            (*key, *modifiers) = (initial_key, initial_mods);
                        }
                        state => {
                            // A known physical release starts a new press even if the old
                            // logical symbol is still down in egui. After focus loss, honor repeat.
                            if matches!(state, DigitState::Released) { *repeat = false; }
                            *key = clear_hot_key(&app.prefs, app.performance(), *key, *digit, modifiers);
                            held[slot] = DigitState::Held(*key, *modifiers);
                        }
                    }
                } else if let DigitState::Held(initial_key, initial_mods) = std::mem::replace(&mut held[slot], DigitState::Released) {
                    (*key, *modifiers) = (initial_key, initial_mods);
                }
            }
        });
    }
    events
}

fn step_list_selection(app: &mut App, up: bool) {
    if app.list_dirty {
        app.rebuild_list();
    }
    if !app.list.is_empty() {
        let cur = app.anchor.unwrap_or(0);
        let next = if up { cur.saturating_sub(1) } else { (cur + 1).min(app.list.len() - 1) };
        let next = if app.anchor.is_none() { 0 } else { next };
        app.set_anchor(next);
        app.selected.clear();
        app.selected.insert(app.list[next]);
        app.scroll_to_selected = true;
    }
}

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    if app.ext.cleanup.is_none() {
        keycmd::frame(app, ctx);
    }
    let key_events = shortcut_key_events(app, ctx);
    let was_released = |binding: Option<app::Shortcut>| binding.is_some_and(|sc| key_events.iter().any(|event| match event {
        egui::Event::Key { key, physical_key, modifiers, pressed: false, .. } if *key == sc.key => {
            !physical_key.is_some_and(|k| HOT_CUE_DIGITS.contains(&k)) || sc.mods_match(modifiers, false)
        }
        _ => false,
    }));
    // Previews never outlive what holds them: losing the window's focus ends them (key-up
    // events are not delivered then), and a hot-cue preview ends once neither a hot-cue key nor
    // a mouse button is held.
    if !ctx.input(|i| i.focused) {
        if app.deck.cue_preview {
            app.cue_up();
        }
        if app.deck.hot_preview.is_some() {
            app.hot_cue_up();
        }
        // A Search key held when the window lost focus is never seen released.
        if app.searching() != 0 && !ctx.input(|i| i.pointer.any_down()) {
            app.search(0);
        }
        return;
    }
    if app.deck.hot_preview.is_some() {
        let hot_held = (1..=8).any(|n| app.prefs.key_for(&format!("hot{n}")).map(|k| ctx.input(|i| i.key_down(k))).unwrap_or(false));
        if !hot_held && !ctx.input(|i| i.pointer.any_down()) && !keycmd::holding(app.px.active) {
            app.hot_cue_up();
        }
    }
    if app.ext.cleanup.is_some() {
        return;
    }
    if ui_edit::is_edit(app) {
        if app.edit.modal.is_none() && !ctx.wants_keyboard_input() {
            if ctx.viewport_id() == DetachedLibrary::id() {
                let (up, down, load) = ctx.input_mut(|i| (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                ));
                if up || down {
                    if app.dialogs.ext.focus_tree {
                        prefs_ext::tree_step(app, if up { -1 } else { 1 });
                    } else {
                        step_list_selection(app, up);
                    }
                }
                if load {
                    if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
                        app.load_track(id);
                    }
                }
            }
            ui_edit::keys(app, ctx);
        }
        return;
    }
    // Library edit Undo/Redo (text fields keep their own undo while they are being edited).
    if !ctx.wants_keyboard_input() {
        let (z, shift) = ctx.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::Z), i.modifiers.shift));
        if z {
            if shift {
                app.redo_edit();
            } else {
                app.undo_edit();
            }
        }
    }
    // Key releases end cue/hot-cue previews even while a text field has focus, so a preview
    // can never be left playing.
    if ctx.wants_keyboard_input() {
        let rel = |p: &app::Prefs, a: &str| was_released(p.shortcut_for(a));
        let cue_rel = rel(&app.prefs, "cue");
        let hot_rel = (1..=8).any(|n| rel(&app.prefs, &format!("hot{n}")));
        if app.deck.cue_preview && cue_rel {
            app.cue_up();
        }
        if app.deck.hot_preview.is_some() && hot_rel {
            app.hot_cue_up();
        }
        return;
    }
    use egui::Key;
    let shortcuts: std::collections::HashMap<&str, app::Shortcut> = recordbox_actions().iter().filter(|a| keycmd::old_loop_runs(&app.prefs, a)).filter_map(|a| app.prefs.shortcut_for(a).map(|k| (*a, k))).collect();
    // A press of the action's key with exactly its modifiers (beat jumps also take Shift).
    // The sampler deck's keys work in PERFORMANCE mode only, and there they come before
    // another action on the same key (they are rekordbox's Performance preset).
    let perf = app.performance();
    let smp_keys: Vec<app::Shortcut> = if perf { shortcuts.iter().filter(|(a, _)| sampler_ext::is_sampler_action(a)).map(|(_, s)| *s).collect() } else { vec![] };
    let hit = |a: &str, repeats: bool| {
        let Some(sc) = shortcuts.get(a) else { return false };
        if sampler_ext::is_sampler_action(a) { if !perf { return false; } } else if smp_keys.contains(sc) { return false; }
        let free_shift = a == "jump_back" || a == "jump_fwd";
        key_events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if *key == sc.key && (repeats || !*repeat) && sc.mods_match(modifiers, free_shift)))
    };
    let pressed = |a: &str| hit(a, true);
    let released = |a: &str| was_released(shortcuts.get(a).copied());
    let first = |a: &str| hit(a, false);
    let shift = ctx.input(|i| i.modifiers.shift);
    let cmd_a = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::A));
    let (up, down, enter, left, right) = (pressed("list_up"), pressed("list_down"), pressed("load"), pressed("jump_back"), pressed("jump_fwd"));
    // Delete also answers to Backspace when it is on its default key.
    let del = pressed("remove") || (shortcuts.get("remove").map(|s| s.name()) == Some("Delete".into()) && ctx.input(|i| i.key_pressed(Key::Backspace) && i.modifiers.is_none()));
    if first("play") {
        app.play_pause();
    }
    if first("cue") {
        app.cue_down();
    }
    if released("cue") {
        app.cue_up();
    }
    if first("memory") {
        app.memory_set();
    }
    for k in 0..8 {
        let id = format!("hot{}", k + 1);
        if first(&id) {
            app.hot_cue_down(k as i32);
        }
        if released(&id) {
            app.hot_cue_up();
        }
    }
    // One-shot actions (prefs_ext.rs). A shortcut the menu bar holds as an accelerator is
    // the menu's to answer.
    let mut acts = vec![];
    for (id, _, _, _) in app::ACTIONS {
        if prefs_ext::HELD_ACTIONS.contains(&id) || menus::owns_shortcut(app, id) {
            continue;
        }
        if let Some(act) = prefs_ext::Act::parse(id) {
            if hit(id, act.repeats()) {
                acts.push(act);
            }
        }
    }
    for act in acts {
        prefs_ext::run_action(app, act);
    }
    // A sampler pad's key let go: gate samples stop.
    if perf {
        for n in 0..16usize {
            if released(&format!("smp_play{}", n + 1)) {
                app.sampler_pad_up(n);
            }
        }
    }
    // Search Backward / Forward run while their key is held.
    for (id, dir) in [("search_back", -1), ("search_fwd", 1)] {
        if first(id) {
            app.search(dir);
        } else if released(id) && app.searching() == dir {
            app.search(0);
        }
    }
    // Change Focus: with the keyboard focus on the Tree View the arrows move there.
    let tree = app.dialogs.ext.focus_tree;
    if tree && (left || right) {
        prefs_ext::tree_expand(app, right);
    }
    if tree && (up || down) {
        prefs_ext::tree_step(app, if up { -1 } else { 1 });
    }
    let (left, right, up, down) = (left && !tree, right && !tree, up && !tree, down && !tree);
    // In loop adjust the arrows move the loop edge (Shift: 1 ms) instead of jumping.
    if (left || right) && app.deck.x.loop_adjust != 0 && app.loop_active().is_some() {
        app.loop_adjust_step(if left { -1 } else { 1 }, shift);
    } else if left {
        app.beat_jump(-if shift { 1.0 } else { app.prefs.beat_jump });
    } else if right {
        app.beat_jump(if shift { 1.0 } else { app.prefs.beat_jump });
    }
    if up || down { step_list_selection(app, up); }
    if enter {
        if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
            app.load_track(id);
        }
    }
    if cmd_a {
        app.selected = app.list.iter().copied().collect();
    }
    if del && !app.selected.is_empty() {
        if let Some(pl) = app.is_manual_playlist() {
            let s = app.selected.clone();
            app.remove_from_playlist(pl, &s);
        }
    }
}


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeckOptions {
    pub disable_load: bool,
    pub disable_play: bool,
}

impl DeckOptions {
    fn from_args(args: &[String]) -> Self {
        Self {
            disable_load: args.iter().any(|arg| arg == "--disable-load"),
            disable_play: args.iter().any(|arg| arg == "--disable-play"),
        }
    }
}

#[cfg(test)]
mod launch_tests {
    use super::DeckOptions;

    #[test]
    fn redocking_defers_size_constraints_until_the_window_manager_restores_the_player() {
        use super::{egui, restore_docked_size, DetachedLibrary, MAIN_MIN_SIZE};
        let ctx = egui::Context::default();
        let original = egui::vec2(1440.0, 840.0);
        let mut raw = egui::RawInput::default();
        raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().maximized = Some(true);
        let mut pending = None;
        let output = ctx.run(raw, |ctx| {
            let window = DetachedLibrary { builder: Default::default(), docked_size: Some(original), deck_window_height: 312.0, pending_fullscreen: None };
            pending = window.reattach(ctx);
        });
        assert_eq!(pending, Some(original));
        assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.iter().all(|cmd|
            !matches!(cmd, egui::ViewportCommand::MinInnerSize(_) | egui::ViewportCommand::InnerSize(_))
        ));
        let output = ctx.run(Default::default(), |ctx| restore_docked_size(ctx, pending.take()));
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::MinInnerSize(size) if *size == MAIN_MIN_SIZE)));
        assert!(commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::InnerSize(size) if *size == original)));
    }

    #[test]
    fn library_opens_in_the_main_windows_fullscreen_mode() {
        use super::{egui, DetachedLibrary};
        for (fullscreen, maximized) in [(true, false), (false, true), (false, false)] {
            let mut raw = egui::RawInput::default();
            let main = raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
            main.fullscreen = Some(fullscreen);
            main.maximized = Some(maximized);
            let _ = egui::Context::default().run(raw, |ctx| {
                let library = DetachedLibrary::open(ctx, 256.0);
                assert_eq!(library.builder.fullscreen, Some(fullscreen));
                assert_eq!(library.builder.maximized, None);
            });
        }
    }

    #[test]
    fn deck_switches_are_independent_and_session_only() {
        for (args, load, play) in [
            (vec![], false, false), (vec!["--disable-load"], true, false),
            (vec!["--disable-play"], false, true), (vec!["--disable-play", "--disable-load"], true, true)
        ] {
            let args: Vec<_> = args.into_iter().map(String::from).collect();
            assert_eq!(DeckOptions::from_args(&args), DeckOptions { disable_load: load, disable_play: play });
        }
    }
}

fn main() -> eframe::Result<()> {
    let mut native_args = std::env::args_os().skip(1);
    if native_args.next().as_deref() == Some(std::ffi::OsStr::new("--inspect-onelibrary")) {
        if let Err(error) = onelibrary::cli(native_args) {
            eprintln!("OneLibrary inspection failed: {error:#}");
            std::process::exit(2);
        }
        return Ok(());
    }
    let args: Vec<String> = std::env::args().collect();
    if args.iter().skip(1).any(|arg| arg == "--help" || arg == "-h") {
        println!("USB inspection: recordbox --inspect-onelibrary USB_ROOT [--playlist ID [--classic-playlist ID]]");
        println!("Usage: recordbox [--disable-load] [--disable-play] [--mp3-timeline legacy|rekordbox] [--screenshot FILE] [--stems-cpu] [--stems-status | --stems-device | --compare-stems FILE]\n\n  --disable-load  Disable loading tracks into decks for this session.\n  --disable-play  Disable audio playback and previews for this session.\n  --mp3-timeline legacy|rekordbox\n                  Select the MP3 sample clock for this session. Legacy opens your usual library.\n                  Rekordbox uses a separate collection and a different MP3 time origin.\n                  Audition imported cues and grids before export or write-back.\n  --screenshot    Save a picture of the window to FILE and exit.\n  --stems-cpu     Use CPU separation for this session.\n  --stems-status  Show build, system memory and local model status without starting inference.\n  --stems-device  Check the selected separation device and exit.\n  --compare-stems FILE  Compare GPU and CPU separation of one local track and exit.");
        return Ok(());
    }
    let timeline = audio::Mp3Timeline::from_args(&args[1..]).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    timeline.initialize();
    if timeline == audio::Mp3Timeline::Rekordbox {
        eprintln!("MP3 timeline: rekordbox. Separate collection: {}", db::data_dir_for_inspection().join("library.db").display());
    }
    if args.iter().any(|a| a == "--stems-status") {
        println!("{}", stems::status_report());
        return Ok(());
    }
    if args.iter().any(|a| a == "--stems-cpu") {
        stems::force_cpu();
    }
    if let Some(i) = args.iter().position(|a| a == "--compare-stems") {
        let result = args.get(i + 1).ok_or_else(|| anyhow::anyhow!("Supply a local audio file after --compare-stems"))
            .and_then(|path| stems::compare_track(path));
        if let Err(error) = result {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--stems-device") {
        println!("STEMS backend: {}", stems::backend_label());
        return Ok(());
    }
    let deck_options = DeckOptions::from_args(&args[1..]);
    let screenshot = args.iter().position(|a| a == "--screenshot").and_then(|i| args.get(i + 1).cloned());
    let shot_mode = screenshot.is_some();
    let mut viewport = egui::ViewportBuilder::default().with_title("recordbox").with_inner_size([1512.0, 921.0]).with_min_inner_size(MAIN_MIN_SIZE);
    // The recordbox mark as the window and Dock icon.
    if let Some(icon) = brand::app_icon(brand::system_dark()) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    if shot_mode {
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    if shot_mode {
        options.event_loop_builder = Some(Box::new(|b| {
            #[cfg(target_os = "macos")]
            {
                use winit::platform::macos::EventLoopBuilderExtMacOS;
                b.with_activate_ignoring_other_apps(false);
            }
            let _ = b;
        }));
        options.run_and_return = false;
    }
    let t0 = std::time::Instant::now();
    eframe::run_native("recordbox", options, Box::new(move |cc| {
        #[cfg(target_os = "linux")]
        x11_fullscreen::install(cc);
        let app = App::new(cc, screenshot, deck_options);
        if bench_on() {
            eprintln!("bench: ready to draw {:.0} ms after launch", t0.elapsed().as_secs_f32() * 1000.0);
        }
        Ok(Box::new(Gui { app, pending_docked_size: None }))
    }))
}
