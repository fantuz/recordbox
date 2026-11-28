use super::*;

fn opus() -> Arc<Device> {
    let map = MidiMap::load("OPUS-QUAD", Some(include_bytes!("profiles/OPUS-QUAD.csv")), None).unwrap();
    Device::mapped("OPUS-QUAD", "opus-test".into(), map, true, None)
}

#[test]
fn transport_feedback_follows_held_preview_playback_and_track_end() {
    let decks = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
    let (hub, _) = Hub::new(decks, PathBuf::new(), true);
    let dev = opus();
    hub.devices.lock().push(dev.clone());
    let check = |play, cue| {
        hub.transport_leds(0);
        let leds = dev.leds.lock();
        for deck in [0, 1] {
            assert_eq!(leds[&(registry().tag("PlayPause"), deck)], play);
            assert_eq!(leds[&(registry().tag("Cue"), deck)], cue);
        }
    };
    check(Led::Off, Led::Off);
    let decoded = || Arc::new(crate::audio::Decoded {
        sr: 1000, samples: vec![0.0; 2000], raw_frames: 1000, damaged: 0, truncated: false,
    });
    hub.decks[0].lock().audio = Some(decoded());
    check(Led::BlinkMs(1000), Led::On);
    assert!(hub.cue(0, true));
    hub.decks[0].lock().pos = 150.0;
    check(Led::BlinkMs(1000), Led::On);
    hub.play_pause(0);
    assert!(hub.cue(0, false));
    check(Led::On, Led::On);
    hub.snap.lock().decks[0].hot[0] = Some((0.50, None));
    assert!(hub.hot_cue(0, 0, true));
    check(Led::On, Led::On); // A pad pressed during playback is not a gated preview.
    assert!(hub.hot_cue(0, 0, false));
    hub.play_pause(0);
    check(Led::BlinkMs(1000), Led::BlinkMs(500));
    hub.snap.lock().decks[0].ui_preview = true;
    hub.decks[0].lock().playing = true;
    check(Led::BlinkMs(1000), Led::On);
    hub.snap.lock().decks[0].ui_preview = false;
    {
        let mut s = hub.decks[0].lock();
        s.playing = false;
        s.pos = 999.0;
    }
    check(Led::BlinkMs(1000), Led::Off);

    hub.decks[0].lock().pos = 0.0;
    assert!(hub.cue(0, true));
    hub.decks[0].lock().audio = Some(decoded());
    // The previous track's held preview must not control its replacement.
    check(Led::On, Led::On);
    assert!(hub.cue(0, false));
    assert!(hub.decks[0].lock().playing);
}

#[test]
fn browser_packets_follow_filtered_rows_tree_and_folder_focus() {
    const TEST: &str = "midi::browser_transport_tests::browser_packets_follow_filtered_rows_tree_and_folder_focus";
    const PROFILE: &str = "RECORDBOX_MIDI_BROWSER_TEST_PROFILE";
    if std::env::var_os(PROFILE).is_none() {
        let dir = tempfile::tempdir().unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(PROFILE, dir.path()).env("PREPDECK_DATA", dir.path()).output().unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(child.status.success() && stdout.contains("1 passed"), "{stdout}\n{}", String::from_utf8_lossy(&child.stderr));
        return;
    }
    assert_eq!(std::env::var_os(PROFILE), std::env::var_os("PREPDECK_DATA"));
    let mut app = App::open_library(&Default::default(), None, crate::DeckOptions { disable_play: true, disable_load: true });
    app.prefs.app_mode = 0;
    let (hub, rx) = Hub::new(app.player.decks.clone(), PathBuf::new(), false);
    let dev = opus();
    app.midi.rx = Some(rx);
    let packet = |app: &mut App, bytes| {
        hub.input_message(&dev, bytes, 0.0, 0, 0);
        apply_pending(app, &hub);
    };
    let click = |app: &mut App, name| {
        let f = registry().by_name(name).unwrap();
        for down in [true, false] {
            apply(app, &hub, "test", f, 0, Val::On(down, if down { 127 } else { 0 }), false, 0);
        }
    };
    let parent = app.db.create_node(0, "Folder", NodeKind::Folder, None).unwrap();
    let child = app.db.create_node(parent, "First", NodeKind::Playlist, None).unwrap();
    let empty = app.db.create_node(parent, "Empty", NodeKind::Folder, None).unwrap();
    app.reload_playlists();
    app.set_view(View::Playlist(parent));
    packet(&mut app, [0x90, 0x32, 127]); // OPUS-QUAD Back from the track list.
    packet(&mut app, [0x90, 0x32, 0]);
    assert!(app.dialogs.ext.focus_tree && app.ext.tree_focus);
    click(&mut app, "Forward");
    assert_eq!(app.view, View::Playlist(child));
    assert!(app.expanded.contains(&parent) && app.ext.tree_focus);
    packet(&mut app, [0xb0, 0x4f, 1]);
    assert_eq!(app.view, View::Playlist(empty));
    click(&mut app, "Forward");
    assert!(app.expanded.contains(&empty) && app.ext.tree_focus);
    click(&mut app, "Back");
    assert!(!app.expanded.contains(&empty));
    click(&mut app, "Back");
    assert_eq!(app.view, View::Playlist(parent));
    assert!(!app.expanded.contains(&parent));
    packet(&mut app, [0xb0, 0x4f, 0]);
    assert!(!app.dialogs.ext.focus_tree && !app.ext.tree_focus);

    for i in 0..48 {
        app.db.insert_track(&Track {
            path: format!("/test/song-{i}.mp3"), title: format!("MIDI-filter {i:02}"), ..Track::default()
        }).unwrap();
    }
    app.reload_tracks();
    app.set_view(View::Collection);
    app.search = "MIDI-filter".into();
    app.prefs.sort_col = Col::Title;
    app.prefs.sort_asc = true;
    app.list_dirty = true;
    // The initial 30 groups into eight jumps of three. First jump selects row zero.
    packet(&mut app, [0xb0, 0x4f, 30]);
    assert_eq!(app.list.len(), 48);
    assert_eq!(app.anchor, Some(21));
    packet(&mut app, [0xb0, 0x4f, 13]); // Four rows in each of three groups.
    assert_eq!(app.anchor, Some(33));
    packet(&mut app, [0xb0, 0x4f, 98]);
    assert_eq!(app.anchor, Some(9));
    click(&mut app, "AddToTagList");
    assert_eq!(app.ext.tag_list, vec![app.list[9]]);
    let old = app.ext.tag_list.clone();
    packet(&mut app, [0xb0, 0x4f, 1]);
    app.search = "MIDI-filter 47".into();
    app.list_dirty = true;
    click(&mut app, "AddToTagList"); // The old row is outside the new filter.
    assert_eq!(app.ext.tag_list, old);
    packet(&mut app, [0xb0, 0x4f, 1]);
    click(&mut app, "AddToTagList");
    assert_eq!(app.ext.tag_list[1], app.list[0]);
}
