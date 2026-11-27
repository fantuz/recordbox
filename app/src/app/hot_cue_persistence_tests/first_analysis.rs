use super::{add_track, assert_track_cues, isolated, load, open_app};
use crate::analysis::AnalysisResult;
use crate::app::App;
use crate::browser_ext::CueTendency;
use crate::model::{BeatGrid, Cue, CueKind, Waveform};
use anyhow::Result;

fn load_both_decks(app: &mut App, id: i64) {
    load(app, 0, id);
    load(app, 1, id);
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
