use super::*;
use crate::{DetachedLibrary, Gui};
use eframe::egui::{self, ViewportId};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

fn input(id: ViewportId) -> egui::RawInput {
    let mut raw = egui::RawInput {
        viewport_id: id,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1550.0, 1000.0))),
        ..Default::default()
    };
    raw.viewports.entry(DetachedLibrary::id()).or_default().parent = Some(ViewportId::ROOT);
    raw
}

fn root_frame(gui: &mut Gui) {
    let ctx = gui.app.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    let _ = ctx.run(input(ViewportId::ROOT), |ctx| eframe::App::update(gui, ctx, &mut frame));
}

#[test]
fn a_parked_deck_keeps_receiving_stems_as_modes_and_library_windows_change() -> Result<()> {
    isolated("modes::a_parked_deck_keeps_receiving_stems_as_modes_and_library_windows_change", |dir| {
        let mut gui = Gui { app: open_app(), pending_docked_size: None };
        let app = &mut gui.app;
        app.prefs.stems_auto_download = false;
        let (id, _) = track(app, dir, "deck-four.wav")?;
        app.reload_tracks();
        app.set_app_mode(1);
        crate::app_hooks::set_perf_layout(app, 3);
        app.activate_deck(3);
        app.load_track(id);
        finish_loading(app);
        app.stem.separating = Some((id, 0.0));
        app.stem.active_key = app.deck.stem_cache_key.clone();

        // Render a distinct immediate child EgUI pass as the native integration does;
        // closing it exercises the production update's reattachment path.
        let child_frames = Rc::new(Cell::new(0usize));
        let close = Rc::new(Cell::new(false));
        let (calls, requested) = (child_frames.clone(), close.clone());
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut child| {
            assert_eq!(child.ids.this, DetachedLibrary::id());
            let mut raw = input(child.ids.this);
            if requested.get() {
                raw.viewports.get_mut(&child.ids.this).unwrap().events.push(egui::ViewportEvent::Close);
            }
            calls.set(calls.get() + 1);
            let _ = ctx.run(raw, |ctx| (child.viewport_ui_cb)(ctx));
        });
        gui.app.ctx.set_embed_viewports(false);
        gui.app.toggle_library_window = true;
        let mut p = 0.0;
        let next = |gui: &mut Gui, p: &mut f32| -> Result<()> {
            *p += 0.04;
            gui.app.stem.tx.send(StemMsg::Progress(id, *p))?;
            root_frame(gui);
            assert_eq!(gui.app.stem.separating, Some((id, *p)));
            assert!(!gui.app.stem.cancel.load(std::sync::atomic::Ordering::Relaxed));
            assert!(gui.app.deck_of(3).stem_cache_key.as_ref().is_some_and(|k| k.track == id));
            Ok(())
        };

        for row in [3, 2, 1, 0, 4] {
            crate::app_hooks::set_perf_layout(&mut gui.app, row);
            next(&mut gui, &mut p)?;
            assert_eq!(crate::app_hooks::perf_layout(&gui.app), row);
            assert!(gui.app.library_window.is_some());
        }
        assert!(child_frames.get() >= 5);
        gui.app.set_app_mode(0);
        for row in [0, 1, 2, 3, 4] {
            gui.app.set_player_layout(row);
            next(&mut gui, &mut p)?;
            assert_eq!(gui.app.player_layout(), row);
        }
        gui.app.set_app_mode(crate::ui_edit::MODE_EDIT);
        next(&mut gui, &mut p)?;
        assert!(crate::ui_edit::is_edit(&gui.app));
        let count = child_frames.get();
        close.set(true);
        next(&mut gui, &mut p)?;
        assert!(gui.app.library_window.is_none());
        assert!(child_frames.get() > count);
        // Deck 4 is parked for editing. Its result may only attach to that deck.
        let parts = Arc::new(Stems { len: FRAMES, data: vec![0.5; 8 * FRAMES] });
        gui.app.stem.tx.send(StemMsg::Done(id, Ok(parts.clone())))?;
        root_frame(&mut gui);
        assert!(gui.app.stem.separating.is_none());
        for n in 0..3 {
            assert!(gui.app.player.decks[n].lock().stems.is_none());
        }
        gui.app.set_app_mode(1);
        crate::app_hooks::set_perf_layout(&mut gui.app, 3);
        gui.app.with_deck(3, |app| {
            assert!(Arc::ptr_eq(app.player.state.lock().stems.as_ref().unwrap(), &parts));
            crate::prefs_ext::stem_part_click(app, &[3]);
            assert!(!app.player.state.lock().stem_on[3]);
        });
        assert!(gui.app.player.decks[..3].iter().all(|d| d.lock().stem_on[3]));
        Ok(())
    })
}

#[test]
fn child_initiated_cache_read_wakes_the_root_even_if_the_library_closes() -> Result<()> {
    isolated("modes::child_initiated_cache_read_wakes_the_root_even_if_the_library_closes", |dir| {
        let mut app = open_app();
        app.prefs.stems_auto_download = false;
        let (id, cache) = track(&app, dir, "detach.wav")?;
        app.reload_tracks();
        app.load_track(id);
        finish_loading(&mut app);
        save_parts(&cache, 0.25)?;
        let ctx = app.ctx.clone();
        for _ in 0..4 {
            let _ = ctx.run(input(ViewportId::ROOT), |_| {});
        }
        let (send, wakes) = std::sync::mpsc::channel();
        let ui_thread = std::thread::current().id();
        ctx.set_request_repaint_callback(move |info| {
            if std::thread::current().id() != ui_thread && info.delay.is_zero() {
                let _ = send.send(info.viewport_id);
            }
        });
        let mut restored = false;
        let _ = ctx.run(input(DetachedLibrary::id()), |ctx| {
            if restored {
                return;
            }
            assert_eq!(ctx.viewport_id(), DetachedLibrary::id());
            app.separate_loaded();
            assert!(app.loading_stems());
            assert_eq!(wakes.recv_timeout(Duration::from_secs(5)).unwrap(), ViewportId::ROOT);
            restored = true;
        });
        app.poll_stems();
        assert_parts(&app, 0.25);
        assert!(!app.loading_stems());
        Ok(())
    })
}
