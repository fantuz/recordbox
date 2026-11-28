use super::*;

#[test]
fn pitch_faders_follow_deck_reset_master_and_range_across_queued_events() {
    // Isolate the library and controller settings from other tests and user profiles.
    const CHILD: &str = "RECORDBOX_TEMPO_FIXTURE";
    if std::env::var_os(CHILD).is_none() {
        let profile = tempfile::tempdir().unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "midi::tempo_tests::pitch_faders_follow_deck_reset_master_and_range_across_queued_events", "--nocapture"])
            .env(CHILD, "1").env("PREPDECK_DATA", profile.path()).output().unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(child.status.success() && stdout.contains("1 passed"), "{stdout}\n{}", String::from_utf8_lossy(&child.stderr));
        return;
    }
    let mut app = App::open_library(&Default::default(), None, crate::DeckOptions { disable_play: true, disable_load: true });
    app.prefs.app_mode = 1;
    app.prefs.perf_four = true;
    let (hub, rx) = Hub::new(app.player.decks.clone(), PathBuf::new(), true);
    let dev = Device::mapped("Test faders", "port".into(), MidiMap::load("Test faders", None, None).unwrap(), false, None);
    hub.devices.lock().push(dev.clone());
    hub.routing.lock().performance = true;
    app.midi.hub = Some(hub.clone());
    app.midi.rx = Some(rx);
    app.midi.started = true;
    let audio = Arc::new(crate::audio::Decoded { sr: 44100, samples: vec![0.0; 200], raw_frames: 100, damaged: 0, truncated: false });
    for n in [0u8, 3] {
        app.deck_of_mut(n).audio = Some(audio.clone());
        app.deck_of_mut(n).track = Some(i64::from(n) + 1);
        hub.decks[usize::from(n)].lock().audio = Some(audio.clone());
    }
    let pitch = |entry, deck, v, fast| {
        let event = Event { entry, deck, val: Val::Abs { v, lo: 0, mid: 8192, hi: 16383 }, fast, tag: registry().tag("TempoSlider") };
        hub.dispatch(&mut hub.routing.lock(), &dev, event);
    };
    poll(&mut app);
    pitch(0, 1, 8192, true);
    pitch(0, 1, 12288, true);
    assert!(hub.decks[0].lock().tempo > 1.04); // worker leads the UI
    // Reset to the already-displayed zero while that newer hardware move is queued.
    app.tempo_reset();
    poll(&mut app);
    assert_eq!((app.deck.x.tempo_adj, hub.decks[0].lock().tempo), (0.0, 1.0));
    assert_eq!(hub.takeover_state(0).leds(), (true, false));
    pitch(0, 1, 11000, true);
    poll(&mut app);
    assert_eq!(app.deck.x.tempo_adj, 0.0);
    pitch(0, 1, 8192, true);
    poll(&mut app);
    assert_eq!(hub.takeover_state(0).state, 5);

    // Deck 4 is a sync follower. Becoming master by itself changes which fader can act.
    app.deck_of_mut(3).x.tempo_adj = 0.03;
    app.deck_of_mut(3).x.beat_sync = true;
    app.push_deck_controls();
    pitch(3, 4, 0, true);
    poll(&mut app);
    assert_eq!((hub.takeover_state(3).state, hub.takeover_state(3).leds()), (2, (false, false)));
    app.with_deck(3, |a| a.set_master());
    assert_eq!((hub.takeover_state(3).state, hub.takeover_state(3).leds()), (1, (false, true)));
    assert_eq!(hub.takeover_state(0).state, 5);

    // A synced +12% held when the selected range covers only +/-10%: one movement
    // widens deck 4 to +/-16% while preserving the actual tempo and deck 1's range.
    app.deck_of_mut(3).x.tempo_adj = 0.12;
    app.push_deck_controls();
    pitch(3, 4, 16383, true);
    poll(&mut app);
    assert_eq!(TempoTarget::read(&app, 3).range, 0.16);
    assert_eq!(TempoTarget::read(&app, 0).range, 0.10);
    assert_eq!(app.deck_of(3).x.tempo_adj, 0.12);
    assert_eq!(hub.takeover_state(3).leds(), (true, false));
    pitch(3, 4, 14335, false); // same result with a non-Fast mapping
    poll(&mut app);
    assert_eq!(hub.takeover_state(3).state, 5);
    assert!((hub.decks[3].lock().tempo - 1.12).abs() < 0.0001);

    // An unnumbered control belongs to the active deck at the instant of the packet.
    pitch(7, 0, 8192, true);
    pitch(7, 0, 12288, true);
    app.activate_deck(3);
    poll(&mut app);
    assert!(app.deck_of(0).x.tempo_adj > 0.04);
    assert!((app.deck_of(3).x.tempo_adj - 0.12).abs() < 0.0001);
    pitch(7, 0, 8192, true);
    poll(&mut app);
    assert_eq!(hub.takeover_state(3).leds(), (false, true));
    app.activate_deck(0);
    poll(&mut app);
    pitch(7, 0, 8192, true);
    poll(&mut app);
    assert_eq!(hub.takeover_state(0).leds(), (false, true));
    assert!(app.deck_of(0).x.tempo_adj > 0.04);
}
