use super::{App, StemMsg, Track};
use crate::stems::{self, Stems};
use anyhow::{ensure, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const MODEL: &str = "htdemucs_ft";
const FRAMES: usize = 4_410;

// The cache and library both use PREPDECK_DATA. Set it only in a child
// process so parallel tests and the developer's library are unaffected.
fn isolated(test: &str, run: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    const CHILD: &str = "RECORDBOX_STEM_CACHE_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return run(&PathBuf::from(std::env::var_os("PREPDECK_DATA").unwrap()));
    }
    let profile = tempfile::tempdir()?;
    let module = module_path!().split_once("::").unwrap().1;
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", &format!("{module}::{test}"), "--nocapture"])
        .env(CHILD, test)
        .env("PREPDECK_DATA", profile.path())
        .env_remove("PREPDECK_STEM_MODEL")
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn open_app() -> App {
    let options = crate::DeckOptions {
        disable_play: true,
        disable_load: false,
    };
    let mut app = App::open_library(&Default::default(), None, options);
    app.stem.model = Some(MODEL.into());
    app.prefs.stems_auto_separate = false;
    app.prefs.vocal_analysis = false;
    app
}

fn track(app: &App, dir: &Path, name: &str) -> Result<(i64, PathBuf)> {
    let path = dir.join(name);
    crate::demo::write_wav(&path, 44_100, &[0.1; 2 * FRAMES])?;
    let t = Track {
        path: path.to_str().unwrap().into(),
        analyzed: 1,
        ..Default::default()
    };
    Ok((app.db.insert_track(&t)?, stems::cache_path(&t.path, MODEL)))
}

fn save_parts(path: &Path, sample: f32) -> Result<()> {
    stems::save(
        path,
        &Stems {
            len: FRAMES,
            data: vec![sample; 8 * FRAMES],
        },
    )
}

fn finish_loading(app: &mut App) {
    let start = Instant::now();
    loop {
        app.poll_load();
        app.poll_stems();
        assert!(
            app.stem.separating.is_none(),
            "cache restoration started inference"
        );
        if app.stem.model.is_some() && app.deck.loading.is_none() && !app.loading_stems() {
            assert!(app.deck.audio.is_some(), "the generated WAV did not load");
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "deck/cache load timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

// A cache worker may publish its vocal summary before the completed audio.
fn wait_for_cache_result(app: &App) -> Result<StemMsg> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut summaries = Vec::new();
    loop {
        let message = app.stem.rx.recv_deadline(deadline)?;
        if matches!(message, StemMsg::VocalLevels(..)) {
            summaries.push(message);
        } else {
            for summary in summaries { app.stem.tx.send(summary).unwrap(); }
            return Ok(message);
        }
    }
}

fn assert_parts(app: &App, sample: f32) {
    let player = app.player.state.lock();
    let parts = player
        .stems
        .as_ref()
        .expect("the deck should have its cached parts");
    assert_eq!(parts.len, FRAMES);
    assert!(parts
        .data
        .iter()
        .all(|&v| (v - sample).abs() < 1.0 / 32767.0));
}

#[test]
fn failed_hardware_check_reports_a_terminal_state_and_allows_retry() -> Result<()> {
    isolated("failed_hardware_check_reports_a_terminal_state_and_allows_retry", |_| {
        let mut app = open_app();
        app.prefs.stems_auto_download = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.stem.hw.is_none() {
            app.poll_stems();
            ensure!(Instant::now() < deadline, "initial passive hardware check did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
        app.stem.hw = None;
        app.stem.tx.send(StemMsg::Hardware(Err(anyhow::anyhow!("fixture device error"))))?;
        app.poll_stems();
        ensure!(app.stem_hardware_error().is_some_and(|e| e.contains("fixture device error")));
        app.stems_init();
        ensure!(app.stem_hardware_error().is_none());
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.stem.hw.is_none() {
            app.poll_stems();
            ensure!(Instant::now() < deadline, "retry did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    })
}

#[test]
fn export_finds_completed_parts_on_the_other_deck_and_shows_queue_status_per_track() -> Result<()> {
    isolated("export_finds_completed_parts_on_the_other_deck_and_shows_queue_status_per_track", |dir| {
        use crate::stem_export::{self, Part};
        let mut app = open_app();
        let (id, cache) = track(&app, dir, "dual-memory.wav")?;
        let (queued, _) = track(&app, dir, "queued.wav")?;
        app.reload_tracks();
        for n in 0..2 {
            app.with_deck(n, |app| {
                app.load_track_here(id);
                finish_loading(app);
            });
        }
        // The cache was never saved and only deck B has the completed separation.
        assert!(!cache.exists());
        app.player.decks[1].lock().stems = Some(std::sync::Arc::new(Stems { len: FRAMES, data: vec![0.2; FRAMES * 8] }));
        app.prefs.stemx.explain = false;
        app.prefs.stemx.format = 1;
        app.prefs.stemx.dir = dir.join("export").to_string_lossy().into_owned();
        stem_export::request(&mut app, &[id, queued], Part::Vocals, false);
        assert!(stem_export::status_for(&app, id).unwrap().contains("Encoding"));
        assert_eq!(stem_export::status_for(&app, queued).as_deref(), Some("Queued for export · Vocals"));
        let until = Instant::now() + Duration::from_secs(10);
        while stem_export::busy_for(&app, Some(id)) {
            stem_export::poll(&mut app);
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(std::fs::read_dir(dir.join("export"))?.count(), 1);
        assert!(app.stem.separating.is_none());
        Ok(())
    })
}

#[test]
fn disconnected_auxiliary_stem_workers_release_their_slots_and_keep_completed_keys() -> Result<()> {
    isolated("disconnected_auxiliary_stem_workers_release_their_slots_and_keep_completed_keys", |dir| {
        let mut app = open_app();
        let (id, _) = track(&app, dir, "key-batch.wav")?;
        app.reload_tracks();
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send((id, 4))?;
        drop(tx);
        app.key_job = Some((rx, 2, 0));
        app.poll_key_job();
        assert!(app.key_job.is_none());
        assert_eq!(app.track(id).unwrap().key, 4);
        assert!(app.toasts.iter().any(|t| t.error && t.text.contains("1 of 2")));
        let (tx, rx) = crossbeam_channel::bounded(1);
        drop(tx);
        app.deck.vocal_rx = Some(rx);
        app.poll_vocal();
        assert!(app.deck.vocal_rx.is_none());
        Ok(())
    })
}

#[test]
fn cache_write_failure_reports_the_reason_and_keeps_completed_stems_on_the_deck() -> Result<()> {
    isolated("cache_write_failure_reports_the_reason_and_keeps_completed_stems_on_the_deck", |dir| {
        let mut app = open_app();
        let (id, cache) = track(&app, dir, "memory-only.wav")?;
        app.reload_tracks();
        app.load_track(id);
        finish_loading(&mut app);
        let key = app.deck.stem_cache_key.clone().unwrap();
        // Occupy the destination so the real cache writer cannot publish, on any filesystem.
        std::fs::create_dir(&cache)?;
        let sentinel = cache.join("keep");
        std::fs::write(&sentinel, b"original")?;
        let parts = std::sync::Arc::new(Stems { len: FRAMES, data: vec![0.25; 8 * FRAMES] });
        let saved = key.save_completed(&parts, app.track(id).unwrap(), || app.prefs.stem_cache_limit());
        assert!(saved.is_err());
        app.stem.active_key = Some(key.clone());
        app.stem.separating = Some((id, 1.0));
        app.stem.tx.send(StemMsg::CacheWrite(key, saved)).unwrap();
        app.stem.tx.send(StemMsg::Done(id, Ok(parts))).unwrap();
        app.poll_stems();

        assert_parts(&app, 0.25);
        assert!(app.stem.error_for(Some(id)).is_none());
        let warning = app.stem.cache_warning().unwrap();
        assert!(warning.contains("memory-only") && warning.contains("from memory"));
        assert!(warning.contains(cache.to_str().unwrap()));
        assert!(app.toasts.iter().any(|t| t.error && t.text == warning));
        assert_eq!(std::fs::read(sentinel)?, b"original");
        Ok(())
    })
}

#[test]
fn deck_reuses_parts_after_switching_tracks_and_reopening_the_library() -> Result<()> {
    isolated(
        "deck_reuses_parts_after_switching_tracks_and_reopening_the_library",
        |dir| {
            let mut app = open_app();
            let (a, cache_a) = track(&app, dir, "A.wav")?;
            let (b, cache_b) = track(&app, dir, "B.wav")?;
            save_parts(&cache_a, 0.25)?;
            save_parts(&cache_b, -0.5)?;
            app.reload_tracks();

            for (id, sample) in [(a, 0.25), (b, -0.5)] {
                app.load_track(id);
                finish_loading(&mut app);
                assert_parts(&app, sample);
            }
            app.load_track(a);
            assert!(app.deck.audio.is_none());
            // No models installed. Hold downloads so a regression can't contact the network.
            app.stem.downloading = Some((0, 1));
            app.separate_loaded();
            assert!(
                app.stem.pending.is_empty(),
                "manual request queued inference for a cached track"
            );
            finish_loading(&mut app);
            assert_parts(&app, 0.25);
            drop(app);

            let mut reopened = open_app();
            reopened.reload_tracks();
            reopened.load_track(a);
            finish_loading(&mut reopened);
            assert_parts(&reopened, 0.25);
            Ok(())
        },
    )
}

#[test]
fn late_speed_result_preserves_the_selected_quality_parts() -> Result<()> {
    isolated(
        "late_speed_result_preserves_the_selected_quality_parts",
        |dir| {
            let mut app = open_app();
            let (id, quality_path) = track(&app, dir, "two-models.wav")?;
            app.reload_tracks();
            let speed = stems::spec("htdemucs").unwrap();
            let speed_path = stems::cache_path(&app.track(id).unwrap().path, speed.id);
            save_parts(&quality_path, 0.5)?;
            save_parts(&speed_path, -0.25)?;
            app.select_stem_model(speed);
            app.load_track(id);
            app.stem.downloading = Some((0, 1));
            app.separate_loaded();

            // Deliver the real speed read last, independently of OS thread scheduling.
            let delayed = wait_for_cache_result(&app)?;
            assert!(matches!(&delayed, StemMsg::Cached(key, Ok(_)) if key.model == speed.id));
            app.select_stem_model(stems::spec(MODEL).unwrap());
            finish_loading(&mut app);
            assert_parts(&app, 0.5);
            app.stem.tx.send(delayed).unwrap();
            app.poll_stems();
            assert_parts(&app, 0.5);

            app.select_stem_model(speed);
            finish_loading(&mut app);
            assert_parts(&app, -0.25);
            Ok(())
        },
    )
}

#[test]
fn progressive_preview_follows_the_model_and_stops_without_saving() -> Result<()> {
    isolated("progressive_preview_follows_the_model_and_stops_without_saving", |dir| {
        let mut app = open_app();
        let (id, cache) = track(&app, dir, "preview.wav")?;
        app.reload_tracks();
        app.load_track(id);
        finish_loading(&mut app);
        let key = app.deck.stem_cache_key.clone().unwrap();
        app.stem.separating = Some((id, 0.25));
        app.stem.active_key = Some(key.clone());
        let preview = stems::Live::test_prefix(FRAMES, &vec![0.25; FRAMES * 8], FRAMES);
        app.stem.tx.send(StemMsg::Preview(key.clone(), preview.clone())).unwrap();
        app.poll_stems();
        assert!(app.player.state.lock().has_playable_stems());
        app.select_stem_model(stems::spec("htdemucs").unwrap());
        assert!(!app.player.state.lock().has_playable_stems());
        app.stem.tx.send(StemMsg::Preview(key.clone(), preview.clone())).unwrap();
        app.poll_stems();
        assert!(!app.player.state.lock().has_playable_stems());
        app.select_stem_model(stems::spec(MODEL).unwrap());
        assert!(app.player.state.lock().has_playable_stems());
        app.stop_deck_stems();
        app.stem.tx.send(StemMsg::Preview(key, preview)).unwrap();
        app.stem.tx.send(StemMsg::Done(id, Err(anyhow::anyhow!("stopped")))).unwrap();
        app.poll_stems();
        assert!(!app.player.state.lock().has_playable_stems());
        assert!(!cache.exists());
        Ok(())
    })
}

#[test]
fn performance_part_button_can_read_a_cache_while_another_deck_separates() -> Result<()> {
    isolated(
        "performance_part_button_can_read_a_cache_while_another_deck_separates",
        |dir| {
            let mut app = open_app();
            let (a, _) = track(&app, dir, "uncached.wav")?;
            let (b, cache_b) = track(&app, dir, "cached.wav")?;
            save_parts(&cache_b, 0.5)?;
            app.reload_tracks();
            app.set_app_mode(1);
            app.load_track(a);
            finish_loading(&mut app);

            // Stand in for a long-running inference job on deck 1.
            app.stem.separating = Some((a, 0.5));
            app.stem.active_key = app.deck.stem_cache_key.clone();
            app.activate_deck(1);
            app.load_track(b);
            crate::prefs_ext::stem_part_click(&mut app, &[3]);
            assert!(
                app.loading_stems(),
                "the part button did not request the saved stems"
            );
            let result = wait_for_cache_result(&app)?;
            app.stem.tx.send(result).unwrap();
            app.poll_stems();
            assert_parts(&app, 0.5);
            assert_eq!(app.stem.separating, Some((a, 0.5)));
            assert!(app.stem.pending.is_empty());
            Ok(())
        },
    )
}

#[test]
fn performance_decks_restore_speed_and_quality_in_three_part_mode() -> Result<()> {
    isolated(
        "performance_decks_restore_speed_and_quality_in_three_part_mode",
        |dir| {
            let mut app = open_app();
            let speed = stems::spec("htdemucs").unwrap();
            let quality = stems::spec(MODEL).unwrap();
            let (a, quality_a) = track(&app, dir, "left.wav")?;
            let (b, quality_b) = track(&app, dir, "right.wav")?;
            app.reload_tracks();
            for (id, path, sample) in [(a, quality_a, 0.25), (b, quality_b, 0.5)] {
                save_parts(&path, sample)?;
                save_parts(
                    &stems::cache_path(&app.track(id).unwrap().path, speed.id),
                    -sample,
                )?;
            }
            app.prefs.stem_parts = 3;
            app.prefs.stems_auto_download = false;
            app.set_app_mode(1);
            let load_decks = |app: &mut App| {
                for (deck, id) in [(0, a), (1, b)] {
                    app.activate_deck(deck);
                    app.load_track(id);
                }
            };
            let check_decks = |app: &mut App, left, right| {
                for (deck, sample) in [(0, left), (1, right)] {
                    app.with_deck(deck, |app| {
                        finish_loading(app);
                        assert_parts(app, sample);
                    });
                }
                assert!(app.stem.pending.is_empty() && app.stem.downloading.is_none());
            };

            load_decks(&mut app);
            check_decks(&mut app, 0.25, 0.5);
            app.select_stem_model(speed);
            check_decks(&mut app, -0.25, -0.5);
            app.save_prefs();
            drop(app);

            let mut reopened = open_app();
            assert!(reopened.performance());
            assert_eq!(reopened.prefs.stem_parts, 3);
            reopened.stem.model = None;
            reopened.stems_init();
            reopened.reload_tracks();
            load_decks(&mut reopened);
            check_decks(&mut reopened, -0.25, -0.5);
            reopened.select_stem_model(quality);
            check_decks(&mut reopened, 0.25, 0.5);
            // The INST button controls bass and instruments together on the active deck.
            crate::prefs_ext::stem_part_click(&mut reopened, &[2, 1]);
            assert_eq!(
                reopened.player.decks[1].lock().stem_on,
                [true, false, false, true]
            );
            assert_eq!(reopened.player.decks[0].lock().stem_on, [true; 4]);
            Ok(())
        },
    )
}

#[test]
fn damaged_cache_allows_an_explicit_separation_retry() -> Result<()> {
    isolated("damaged_cache_allows_an_explicit_separation_retry", |dir| {
        let mut app = open_app();
        let (id, cache) = track(&app, dir, "damaged.wav")?;
        std::fs::write(cache, b"PDSTEMS1")?;
        app.reload_tracks();
        app.load_track(id);
        finish_loading(&mut app);
        assert!(app.stem.error_for(Some(id)).is_some());
        assert!(app.player.state.lock().stems.is_none());

        // An explicit retry may wait for weights, but must get past the damaged entry.
        app.stem.downloading = Some((0, 1));
        app.separate_loaded();
        finish_loading(&mut app);
        assert_eq!(
            app.stem.pending.front().map(|(key, _)| key.track),
            Some(id)
        );
        Ok(())
    })
}

#[test]
fn dual_player_preserves_both_requests_during_download_and_scopes_stop_to_each_track() -> Result<()> {
    isolated("dual_player_preserves_both_requests_during_download_and_scopes_stop_to_each_track", |dir| {
        let mut app = open_app();
        let (a, _) = track(&app, dir, "left.wav")?;
        let (b, cache_b) = track(&app, dir, "right.wav")?;
        app.reload_tracks();
        app.set_player_layout(2);
        for (deck, track) in [(0, a), (1, b)] {
            app.load_track_to(deck, track);
            app.with_deck(deck, finish_loading);
        }
        // No external models: hold the download, then deliver worker messages over the real channel.
        app.stem.downloading = Some((0, 1));
        for deck in [0, 1, 0] {
            app.with_deck(deck, |app| {
                app.separate_loaded();
                assert!(app.queued_stems());
            });
        }
        assert_eq!(app.stem.pending.iter().map(|(k, _)| k.track).collect::<Vec<_>>(), [a, b]);
        let (key, _) = app.stem.pending.pop_front().unwrap();
        app.stem.downloading = None;
        app.stem.separating = Some((a, 0.0));
        app.stem.active_key = Some(key);
        app.stem.tx.send(StemMsg::Phase(a, stems::SeparationPhase::WaitingForGpu)).unwrap();
        app.poll_stems();
        assert!(app.stem.separation_status().unwrap().contains("Waiting for GPU"));
        app.with_deck(1, |app| {
            app.stop_deck_stems();
            assert!(!app.queued_stems());
            assert!(!app.stem.cancel.load(std::sync::atomic::Ordering::Relaxed));
            app.separate_loaded();
            assert!(app.queued_stems());
        });
        app.stem.tx.send(StemMsg::Phase(a, stems::SeparationPhase::Separating)).unwrap();
        app.stem.tx.send(StemMsg::Progress(a, 0.001)).unwrap();
        app.poll_stems();
        assert_eq!(app.stem.separation_stage().as_deref(), Some("<1%"));
        app.set_player_layout(0); // B stays loaded while parked.
        assert!(app.separating_stems());
        app.stop_deck_stems();
        app.set_player_layout(2);
        assert!(app.stem.pending_for(b));
        assert!(app.stem.cancel.load(std::sync::atomic::Ordering::Relaxed));
        // A stopped worker wakes B; a cache written in the meantime satisfies B without inference.
        save_parts(&cache_b, 0.5)?;
        app.stem.tx.send(StemMsg::Done(a, Err(anyhow::anyhow!("stopped")))).unwrap();
        app.poll_stems();
        app.with_deck(1, |app| { finish_loading(app); assert_parts(app, 0.5); });
        assert!(app.stem.pending.is_empty());
        assert!(app.player.decks[0].lock().stems.is_none());
        Ok(())
    })
}

#[test]
fn dual_player_worker_unwind_reports_error_and_continues_with_next_loaded_deck() -> Result<()> {
    isolated("dual_player_worker_unwind_reports_error_and_continues_with_next_loaded_deck", |dir| {
        let mut app = open_app();
        let (a, cache_a) = track(&app, dir, "next.wav")?;
        let (b, _) = track(&app, dir, "worker.wav")?;
        app.reload_tracks();
        app.set_player_layout(2);
        for (deck, id) in [(0, a), (1, b)] {
            app.load_track_to(deck, id);
            app.with_deck(deck, finish_loading);
        }
        app.with_deck(1, |app| {
            app.stem.separating = Some((b, 0.0));
            app.stem.active_key = app.deck.stem_cache_key.clone();
        });
        app.with_deck(0, |app| app.separate_loaded());
        save_parts(&cache_a, 0.25)?;
        super::finish_stem_job(&app.stem.tx, b, || panic!("synthetic inference error"));
        app.poll_stems();
        assert!(app.stem.error_for(Some(b)).unwrap().detail.contains("synthetic inference error"));
        assert!(app.stem.separating.is_none());
        app.with_deck(0, |app| { finish_loading(app); assert_parts(app, 0.25); });
        Ok(())
    })
}

#[path = "stem_mode_tests.rs"]
mod modes;

#[test]
fn stem_wave_coalesces_masks_and_falls_back_after_a_disconnected_worker() -> Result<()> {
    isolated("stem_wave_coalesces_masks_and_falls_back_after_a_disconnected_worker", |dir| {
        use crate::model::Waveform;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let mut app = open_app();
        let (id, cache) = track(&app, dir, "waveform.wav")?;
        save_parts(&cache, 0.25)?;
        app.reload_tracks();
        app.load_track(id);
        finish_loading(&mut app);
        app.deck.wave = Some(Waveform { frames: vec![[3; 8]] });
        let (started, first) = crossbeam_channel::bounded(1);
        let (release, gate) = crossbeam_channel::bounded(1);
        let work = Arc::new(AtomicUsize::new(0));
        app.player.state.lock().stem_on = [true, true, true, false];
        app.poll_stem_wave_with(move |_, _, _| {
            started.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            Waveform { frames: vec![[1; 8]] }
        });
        first.recv_timeout(Duration::from_secs(10))?;
        let final_mask = [true, false, false, true];
        for mask in [[true; 4], [false, true, false, true], final_mask] {
            app.player.state.lock().stem_on = mask;
            let work = work.clone();
            app.poll_stem_wave_with(move |_, _, _| {
                work.fetch_add(1, Ordering::SeqCst);
                Waveform { frames: vec![[9; 8]] }
            });
        }
        release.send(())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app.stem_wave_shown() {
            let work = work.clone();
            app.poll_stem_wave_with(move |_, mask, _| {
                assert_eq!(mask, final_mask);
                work.fetch_add(1, Ordering::SeqCst);
                Waveform { frames: vec![[2; 8]] }
            });
            ensure!(Instant::now() < deadline, "the latest part mask never reached the deck");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(work.load(Ordering::SeqCst), 1);
        assert_eq!(app.deck.x.stem_wave.as_ref().unwrap().1.frames, vec![[2; 8]]);
        app.player.state.lock().stem_on = [false, true, true, true];
        let failures = Arc::new(AtomicUsize::new(0));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.stem_wave_preparing() {
            let failures = failures.clone();
            app.poll_stem_wave_with(move |_, _, _| {
                failures.fetch_add(1, Ordering::SeqCst);
                panic!("synthetic waveform error")
            });
            ensure!(Instant::now() < deadline, "waveform stayed blank after failure");
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..3 { app.poll_stem_wave(); }
        assert_eq!(failures.load(Ordering::SeqCst), 1);
        assert!(!app.stem_wave_shown() && app.deck.wave.is_some());
        app.player.state.lock().stem_on = [false, false, true, true];
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app.stem_wave_shown() {
            app.poll_stem_wave();
            ensure!(Instant::now() < deadline, "a new mask failed to restore the STEMS waveform");
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    })
}
