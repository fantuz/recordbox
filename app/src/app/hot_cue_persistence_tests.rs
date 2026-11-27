use super::{App, Cue, CueKind, Track};
use anyhow::{ensure, Result};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

mod first_analysis;
mod keyboard;

// Set PREPDECK_DATA only in a child process so that opening an App cannot touch another test's library.
fn isolated(test: &str, run: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    const CHILD: &str = "RECORDBOX_HOT_CUE_TEST";
    let name = test.split_once("::").unwrap().1;
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return run(Path::new(&std::env::var_os("PREPDECK_DATA").unwrap()));
    }
    let profile = tempfile::tempdir()?;
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, name)
        .env("PREPDECK_DATA", profile.path())
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
    let mut app = App::open_library(
        &Default::default(),
        None,
        crate::DeckOptions {
            disable_play: true,
            disable_load: false,
        },
    );
    app.prefs.stems_auto_separate = false;
    app.prefs.vocal_analysis = false;
    app.prefs.phrase_analysis = false;
    app.reload_all();
    app
}

fn add_track(app: &mut App, path: &Path) -> Result<i64> {
    crate::demo::write_wav(path, 44_100, &vec![0.1; 8 * 44_100 * 2])?;
    let id = app.db.insert_track(&Track {
        path: path.to_str().unwrap().into(),
        duration: 8.0,
        analyzed: 1,
        ..Default::default()
    })?;
    app.reload_tracks();
    Ok(id)
}

fn load(app: &mut App, deck: u8, track: i64) {
    app.activate_deck(deck);
    app.load_track_here(track);
    let started = Instant::now();
    while app.deck.loading.is_some() {
        app.poll_load();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "track {track} never finished loading on deck {deck}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(app.deck.track, Some(track));
    assert!(app.deck.audio.is_some());
}

// Undo/Redo can allocate new SQLite row IDs. Compare every persisted cue field other than the ID.
fn cue_data(cues: &[Cue]) -> serde_json::Value {
    let mut cues = cues.to_vec();
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    for cue in &mut cues {
        cue.id = 0;
    }
    serde_json::to_value(cues).unwrap()
}

fn assert_track_cues(app: &App, track: i64, expected: &[Cue]) -> Result<()> {
    let expected_data = cue_data(expected);
    let saved = app.db.cues(track)?;
    assert_eq!(cue_data(&saved), expected_data, "saved cues");
    for n in [0, 1] {
        let deck = app.deck_of(n);
        assert_eq!(deck.track, Some(track));
        assert_eq!(cue_data(&deck.cues), expected_data, "deck {n}");
        for cue in &deck.cues {
            assert!(
                saved.iter().any(|s| s.id == cue.id && s.start == cue.start),
                "deck {n} holds a stale cue ID"
            );
        }
    }
    let mut positions = app.hot_positions.get(&track).cloned().unwrap_or_default();
    let mut hot: Vec<_> = expected
        .iter()
        .filter(|c| c.kind == CueKind::Hot)
        .map(|c| (c.start, c.slot, c.color))
        .collect();
    positions.sort_by(|a, b| a.0.total_cmp(&b.0));
    hot.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(positions, hot);
    assert_eq!(
        app.cue_counts.get(&track).copied().unwrap_or_default(),
        (hot.len(), expected.len() - hot.len())
    );
    Ok(())
}

#[test]
fn hot_cue_can_be_cleared_recreated_and_reloaded_on_both_decks() -> Result<()> {
    isolated(
        concat!(module_path!(), "::hot_cue_can_be_cleared_recreated_and_reloaded_on_both_decks"),
        |dir| {
            let mut app = open_app();
            let track = add_track(&mut app, &dir.join("edited.wav"))?;
            let other = add_track(&mut app, &dir.join("untouched.wav"))?;
            load(&mut app, 0, track);
            load(&mut app, 1, track);
            app.seek(1.25);
            app.hot_cue_down(0);
            let saved = app.db.cues(track)?;
            assert_eq!(saved.len(), 1);
            assert_eq!((saved[0].kind, saved[0].slot, saved[0].start), (CueKind::Hot, 0, 1.25));
            assert_track_cues(&app, track, &saved)?;

            // Simulate older/imported rows written after the decks loaded. Undo must capture
            // that database state, including the second A and the other cues' metadata.
            let cue = &saved[0];
            app.db.save_cue(&Cue {
                id: 0,
                start: 2.0,
                end: Some(3.0),
                comment: "second A".into(),
                ..cue.clone()
            })?;
            app.db.save_cue(&Cue {
                id: 0,
                slot: 1,
                start: 4.0,
                color: 3,
                comment: "keep B".into(),
                ..cue.clone()
            })?;
            app.db.save_cue(&Cue {
                id: 0,
                kind: CueKind::Memory,
                slot: -1,
                start: 0.5,
                end: Some(1.0),
                active_loop: true,
                comment: "memory loop".into(),
                ..cue.clone()
            })?;
            app.db.save_cue(&Cue {
                id: 0,
                track_id: other,
                ..cue.clone()
            })?;
            let original = app.db.cues(track)?;
            let untouched = app.db.cues(other)?;
            let cleared: Vec<_> = original.iter().filter(|c| c.kind != CueKind::Hot || c.slot != 0).cloned().collect();

            app.activate_deck(0);
            let before_clear = app.undo_len();
            app.clear_hot_cue(0);
            assert_track_cues(&app, track, &cleared)?;
            app.clear_hot_cue(0);
            assert_eq!(app.undo_len(), before_clear + 1, "one Undo step for the entire occupied slot");
            app.undo_edit();
            assert_track_cues(&app, track, &original)?;
            app.redo_edit();
            assert_track_cues(&app, track, &cleared)?;

            load(&mut app, 1, track);
            assert_track_cues(&app, track, &cleared)?;
            app.seek(5.5);
            app.hot_cue_down(0);
            let mut recreated = cleared.clone();
            recreated.push(Cue { start: 5.5, ..cue.clone() });
            assert_track_cues(&app, track, &recreated)?;
            app.undo_edit();
            assert_track_cues(&app, track, &cleared)?;
            app.redo_edit();
            assert_track_cues(&app, track, &recreated)?;
            drop(app);

            let mut reopened = open_app();
            load(&mut reopened, 0, track);
            load(&mut reopened, 1, track);
            assert_track_cues(&reopened, track, &recreated)?;
            assert_eq!(cue_data(&reopened.db.cues(other)?), cue_data(&untouched));
            Ok(())
        },
    )
}

#[test]
fn a_failed_hot_cue_write_preserves_the_saved_slot_both_decks_and_undo() -> Result<()> {
    isolated(
        concat!(
            module_path!(),
            "::a_failed_hot_cue_write_preserves_the_saved_slot_both_decks_and_undo"
        ),
        |dir| {
            let mut app = open_app();
            let track = add_track(&mut app, &dir.join("rollback.wav"))?;
            load(&mut app, 0, track);
            load(&mut app, 1, track);
            app.seek(2.5);
            app.hot_cue_down(0);
            let original = app.db.cues(track)?;
            assert_eq!(original.len(), 1);
            let undo = app.undo_len();

            // The old slot has already been deleted when this insert fails.
            app.db.conn.execute_batch(
                "CREATE TEMP TRIGGER reject_hot_insert BEFORE INSERT ON cues
            WHEN NEW.kind = 1 BEGIN SELECT RAISE(ABORT, 'simulated hot cue insert failure'); END;",
            )?;
            app.toasts.clear();
            app.set_hot_cue_at(0, 6.0);
            assert!(app.toasts.iter().any(|t| t.error), "the failed replacement should be visible");
            assert_eq!(app.undo_len(), undo);
            assert_track_cues(&app, track, &original)?;

            app.db.conn.execute_batch("PRAGMA query_only=ON")?;
            app.toasts.clear();
            app.clear_hot_cue(0);
            assert!(app.toasts.iter().any(|t| t.error), "the rejected clear should be visible");
            assert_eq!(app.undo_len(), undo);
            assert_track_cues(&app, track, &original)?;
            Ok(())
        },
    )
}

#[test]
fn cue_undo_and_redo_can_retry_after_a_partial_restore_fails() -> Result<()> {
    isolated(
        concat!(module_path!(), "::cue_undo_and_redo_can_retry_after_a_partial_restore_fails"),
        |dir| {
            let mut app = open_app();
            let track = add_track(&mut app, &dir.join("retry.wav"))?;
            load(&mut app, 0, track);
            load(&mut app, 1, track);
            app.seek(1.0);
            app.hot_cue_down(0);
            let only_a = app.db.cues(track)?;
            app.seek(4.0);
            app.hot_cue_down(1);
            let both = app.db.cues(track)?;
            assert_eq!((only_a.len(), both.len()), (1, 2));

            // Undo must reinsert A after deleting both rows. Redo then reinserts A before reaching B.
            for (operation, failing_slot, before, after) in [
                (App::undo_edit as fn(&mut App), 0, &both, &only_a),
                (App::redo_edit, 1, &only_a, &both),
            ] {
                let saved_rows = serde_json::to_value(app.db.cues(track)?)?;
                let history = (app.undo.len(), app.redo.len());
                app.db.conn.execute_batch(&format!(
                    "CREATE TEMP TRIGGER reject_cue_restore BEFORE INSERT ON cues WHEN NEW.slot = {failing_slot}
                 BEGIN SELECT RAISE(ABORT, 'simulated cue restore failure'); END;"
                ))?;
                app.toasts.clear();
                operation(&mut app);
                assert!(app.toasts.iter().any(|t| t.error), "the rejected restore should be visible");
                assert_eq!((app.undo.len(), app.redo.len()), history);
                assert_eq!(
                    serde_json::to_value(app.db.cues(track)?)?,
                    saved_rows,
                    "rollback must preserve row IDs"
                );
                assert_track_cues(&app, track, before)?;

                app.db.conn.execute_batch("DROP TRIGGER reject_cue_restore")?;
                operation(&mut app);
                assert_track_cues(&app, track, after)?;
            }
            Ok(())
        },
    )
}

#[test]
fn midi_active_loop_survives_reload_and_brake_follows_the_selected_side() -> Result<()> {
    use crate::midi::{apply, registry, Hub};
    use crate::midi_map::Val;
    isolated(concat!(module_path!(), "::midi_active_loop_survives_reload_and_brake_follows_the_selected_side"), |dir| {
        let mut app = open_app();
        let track = add_track(&mut app, &dir.join("loop.wav"))?;
        app.db.save_cue(&Cue {
            id: 0, track_id: track, kind: CueKind::Memory, slot: -1,
            start: 1.0, end: Some(1.5), color: 0, comment: String::new(), active_loop: false,
        })?;
        app.set_app_mode(1);
        load(&mut app, 0, track);
        let (hub, _) = Hub::new(app.player.decks.clone(), dir.to_path_buf(), false);
        let active_loop = registry().by_name("ActiveLoop").unwrap();
        app.deck.cue_point = 2.0;
        apply(&mut app, &hub, "fixture", active_loop, 1, Val::On(true, 127), false, 0);
        assert!(!app.db.cues(track)?[0].active_loop);
        app.deck.cue_point = 1.0;
        apply(&mut app, &hub, "fixture", active_loop, 1, Val::On(true, 127), false, 0);
        assert!(app.db.cues(track)?[0].active_loop);
        assert!(app.deck.cues[0].active_loop);

        let original_left = app.prefs.rb.brake[0];
        let brake = registry().by_name("VinylAdjustBrake.Right").unwrap();
        apply(&mut app, &hub, "fixture", brake, 0, Val::Raw(127), false, 0);
        assert_eq!(app.prefs.rb.brake, [original_left, u8::MAX]);
        assert_eq!(app.prefs.rb.brake, app.prefs.deck.vinyl_brake);
        app.save_prefs();
        drop(app);
        let reopened = open_app();
        assert!(reopened.db.cues(track)?[0].active_loop);
        assert_eq!(reopened.prefs.rb.brake, [original_left, u8::MAX]);
        Ok(())
    })
}
