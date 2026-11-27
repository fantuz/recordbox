use super::{add_track, assert_track_cues, isolated, load, open_app};
use crate::analysis::AnalysisResult;
use crate::app::App;
use crate::browser_ext::CueTendency;
use crate::model::{BeatGrid, Cue, CueKind, Track, Waveform};
use anyhow::Result;

fn load_both_decks(app: &mut App, id: i64) {
    load(app, 0, id);
    load(app, 1, id);
}

#[test]
fn bulk_missing_analysis_queues_only_unique_incomplete_tracks() -> Result<()> {
    isolated(concat!(module_path!(), "::bulk_missing_analysis_queues_only_unique_incomplete_tracks"), |profile| {
        let mut app = open_app();
        app.prefs.analyze_grid = true;
        app.prefs.analyze_key = true;
        let mut ids = Vec::new();
        let valid_grid = BeatGrid::constant(0.25, 122.0, 3);
        let tx = app.db.conn.unchecked_transaction()?;
        for i in 0..320 {
            let path = profile.join(format!("track-{i}.wav"));
            if i != 6 {
                // A present but undecodable file fails the job without audio hardware.
                std::fs::write(&path, [])?;
            }
            let id = app.db.insert_track(&Track {
                path: path.to_string_lossy().into_owned(),
                title: format!("Fixture track {i}"),
                analyzed: if i == 0 { 0 } else { 1 },
                key: if matches!(i, 2 | 9) { -1 } else { 4 },
                key_text: if i == 9 { "Imported key".into() } else { String::new() },
                grid_locked: i == 7,
                ..Default::default()
            })?;
            ids.push(id);
            if i != 1 {
                app.db.save_wave(id, &Waveform::default(), &[])?;
            }
            match i {
                3 => app.db.save_grid(id, &BeatGrid::default())?,
                4 | 6 | 7 | 8 => (),
                5 => {
                    // Valid first anchor, malformed second one. Checking array length alone misses this.
                    app.db.conn.execute("INSERT INTO grids VALUES(?1,?2)", rusqlite::params![
                        id, r#"{"anchors":[{"time":0,"bpm":122,"beat":1},{"time":2}]}"#,
                    ])?;
                }
                _ => app.db.save_grid(id, &valid_grid)?,
            }
        }
        tx.commit()?;
        app.reload_all();
        app.track_mut(ids[8]).unwrap().analyzed = 3;
        // Playlists may contain the same track twice, and selection may include a stale row.
        let mut selection = ids.clone();
        selection.extend([ids[0], ids[3], i64::MAX]);
        assert_eq!(app.analyze_missing(&selection), 6);
        assert_eq!(app.analysis_total, 6);
        for (i, id) in ids.iter().enumerate() {
            let expected = if i < 6 || i == 8 { 3 } else { 1 };
            assert_eq!(app.track(*id).unwrap().analyzed, expected, "track {i}");
        }
        assert_eq!(app.analyze_missing(&selection), 0);
        // A user may still edit metadata between a repair being submitted and applied.
        app.track_mut(ids[1]).unwrap().rating = 4;
        app.save_track_quiet(ids[1]);
        assert_eq!(app.db.track(ids[1]).unwrap().analyzed, 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.analysis_total != 0 {
            assert!(std::time::Instant::now() < deadline, "analysis did not finish");
            app.poll_analysis();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        for (i, id) in ids[..6].iter().enumerate() {
            let expected = if i == 0 { 2 } else { 1 };
            assert_eq!(app.track(*id).unwrap().analyzed, expected);
            assert_eq!(app.db.track(*id).unwrap().analyzed, expected);
        }
        assert_eq!(app.db.track(ids[1]).unwrap().rating, 4);
        assert_eq!(app.db.grid(ids[1]), Some(valid_grid.clone()));
        assert!(!app.db.has_wave(ids[1])?);
        assert!(app.db.has_wave(ids[2])?);
        let toast = app.toasts.last().unwrap();
        assert!(toast.error && toast.text.contains("6 of 6 tracks") && toast.text.contains("Fixture track "), "{}", toast.text);
        assert_eq!(app.db.grid(ids[9]), Some(valid_grid));
        assert_eq!(app.db.track(ids[9]).unwrap().key_text, "Imported key");
        Ok(())
    })
}

fn complete_first_analysis(app: &mut App, id: i64) -> Result<Vec<Cue>> {
    assert!(app.db.wave(id).is_none());
    assert_eq!(app.track(id).unwrap().analyzed, 3);
    let result = AnalysisResult {
        duration: 8.0,
        raw_samples: 352_800,
        sample_rate: 44_100,
        bpm: 120.0,
        grid: Some(BeatGrid::constant(0.0, 120.0, 1)),
        key: -1,
        wave: Waveform { frames: vec![[0; 8]; 1_200] },
        gain_db: 0.0,
        peak: 0.5,
        vocal: None,
    };
    app.apply_analysis(id, Ok(result));
    assert!(app.db.wave(id).is_some());
    let saved = app.db.cues(id)?;
    assert_track_cues(app, id, &saved)?;
    Ok(saved)
}

fn clear_while_analysis_is_queued(app: &mut App, id: i64, slot: i32) -> Result<()> {
    load_both_decks(app, id);
    app.track_mut(id).unwrap().analyzed = 3;
    app.hot_cue_down(slot);
    assert!(app.db.cues(id)?.iter().any(|cue| cue.kind == CueKind::Hot && cue.slot == slot));
    app.clear_hot_cue(slot);
    assert!(app.db.cues(id)?.is_empty());
    Ok(())
}

#[test]
fn clearing_hot_a_during_first_analysis_affects_only_that_track_and_job() -> Result<()> {
    isolated(concat!(module_path!(), "::clearing_hot_a_during_first_analysis_affects_only_that_track_and_job"), |profile| {
        let mut app = open_app();
        app.prefs.auto_first_downbeat_cue = true;
        app.prefs.memory_cue_first_beat = true;
        let edited = add_track(&mut app, &profile.join("edited.wav"))?;
        let untouched = add_track(&mut app, &profile.join("untouched.wav"))?;
        let retried = add_track(&mut app, &profile.join("retried.wav"))?;

        clear_while_analysis_is_queued(&mut app, edited, 0)?;
        clear_while_analysis_is_queued(&mut app, retried, 0)?;
        app.apply_analysis(retried, Err("stopped".into()));
        for id in [untouched, retried] {
            load_both_decks(&mut app, id);
            app.track_mut(id).unwrap().analyzed = 3;
            let cues = complete_first_analysis(&mut app, id)?;
            assert_eq!(cues.len(), 2);
            assert!(cues.iter().any(|cue| cue.kind == CueKind::Hot && cue.slot == 0 && cue.comment == "1.1Bars"));
            assert!(cues.iter().any(|cue| cue.kind == CueKind::Memory));
        }
        load_both_decks(&mut app, edited);
        let cues = complete_first_analysis(&mut app, edited)?;
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].kind, CueKind::Memory);
        Ok(())
    })
}

#[test]
fn a_learned_cue_pattern_excludes_only_the_hot_slot_cleared_during_analysis() -> Result<()> {
    isolated(concat!(module_path!(), "::a_learned_cue_pattern_excludes_only_the_hot_slot_cleared_during_analysis"), |profile| {
        let mut app = open_app();
        app.prefs.bx.cue_tendency = CueTendency { tracks: 1, hot: vec![(0, 0), (7, 1)], memory: vec![2] };
        let id = add_track(&mut app, &profile.join("tendency.wav"))?;
        clear_while_analysis_is_queued(&mut app, id, 7)?;
        let cues = complete_first_analysis(&mut app, id)?;
        assert_eq!(cues.len(), 2);
        assert!(cues.iter().any(|cue| cue.kind == CueKind::Hot && cue.slot == 0));
        assert!(cues.iter().any(|cue| cue.kind == CueKind::Memory && cue.start == 4.0));
        Ok(())
    })
}

#[test]
fn undoing_the_first_hot_cue_during_analysis_preserves_the_empty_slot() -> Result<()> {
    isolated(concat!(module_path!(), "::undoing_the_first_hot_cue_during_analysis_preserves_the_empty_slot"), |profile| {
        let mut app = open_app();
        app.prefs.auto_first_downbeat_cue = true;
        let id = add_track(&mut app, &profile.join("undone.wav"))?;
        load_both_decks(&mut app, id);
        app.track_mut(id).unwrap().analyzed = 3;
        app.hot_cue_down(0);
        assert_eq!(app.db.cues(id)?.len(), 1);
        app.undo_edit();
        assert_track_cues(&app, id, &[])?;
        assert!(complete_first_analysis(&mut app, id)?.is_empty());
        Ok(())
    })
}

#[test]
fn rejected_automatic_cues_do_not_appear_on_decks_or_in_the_browser() -> Result<()> {
    isolated(concat!(module_path!(), "::rejected_automatic_cues_do_not_appear_on_decks_or_in_the_browser"), |profile| {
        let mut app = open_app();
        app.prefs.auto_first_downbeat_cue = true;
        app.prefs.memory_cue_first_beat = true;
        let id = add_track(&mut app, &profile.join("rejected.wav"))?;
        load_both_decks(&mut app, id);
        app.db.conn.execute_batch(
            "CREATE TEMP TRIGGER reject_automatic_cue BEFORE INSERT ON cues BEGIN SELECT RAISE(ABORT, 'test rejection'); END;",
        )?;
        app.track_mut(id).unwrap().analyzed = 3;
        assert!(complete_first_analysis(&mut app, id)?.is_empty());
        Ok(())
    })
}
