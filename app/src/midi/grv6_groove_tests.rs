//! Replay published DDJ-GRV6 messages through the same CSV import and MIDI routing as a
//! connected controller. Read feedback from the port's queue before it reaches the OS.
use super::*;
use crate::groove::{Engine, GcClip};

const DEVICE: &str = "DDJ-GRV6";
const FIXTURE: &[u8] = include_bytes!("../../../docs/studies/DDJ-GRV6-groove-check.csv");

struct Replay {
    hub: Arc<Hub>,
    device: Arc<Device>,
    events: crossbeam_channel::Receiver<UiEvent>,
}

impl Replay {
    fn new() -> Self {
        Self::with_decks(std::array::from_fn(|_| {
            Arc::new(Mutex::new(PlayState::default()))
        }))
    }

    fn with_decks(decks: [Arc<Mutex<PlayState>>; 4]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let (mut hub, events) = Hub::new(decks, tmp.path().into(), true);
        let setup = Arc::get_mut(&mut hub).unwrap();
        setup.factory_dirs.clear();
        setup.native_user_dirs.clear();
        hub.write_override(DEVICE, FIXTURE).unwrap();
        let (map, factory) = hub.load_map(DEVICE);
        assert!(!factory);
        assert_eq!(map.rejected, 0);
        let mut device = Device::mapped(DEVICE, DEVICE.to_owned(), map, factory, None);
        Arc::get_mut(&mut device).unwrap().has_output = true;
        hub.routing.lock().performance = true;
        hub.snap.lock().count = 4;
        Self {
            hub,
            device,
            events,
        }
    }

    fn send(&self, bytes: [u8; 3]) {
        self.hub.input_message(&self.device, bytes, 0.0, 0, 0);
    }

    fn value(&self, expected: Act, deck: u8) -> Val {
        let Ok(UiEvent::Control {
            tag,
            deck: actual,
            val,
            ..
        }) = self.events.try_recv()
        else {
            panic!("missing {expected:?} for deck {deck}");
        };
        assert_eq!(registry().get(tag).unwrap().act, expected);
        assert_eq!(actual, deck);
        assert!(self.events.is_empty());
        val
    }

    fn feedback(&self, app: &mut App) -> Vec<[u8; 3]> {
        apply_pending(app, &self.hub);
        leds(app, &self.hub);
        let mut port = self.device.out.lock();
        std::iter::from_fn(|| port.queue.pop()).collect()
    }

    fn click(&self, app: &mut App, channel: u8, note: u8) -> Vec<[u8; 3]> {
        self.send([0x90 | channel, note, 127]);
        self.send([0x90 | channel, note, 0]);
        self.feedback(app)
    }
}

#[test]
fn published_groove_buttons_route_to_the_named_actions_on_each_deck() {
    let replay = Replay::new();
    for channel in 0..4 {
        let deck = channel + 1;
        let note_on = 0x90 | channel;
        for (note, action) in [
            (0x00, Act::DrumSwap(0)),
            (0x01, Act::DrumSwap(1)),
            (0x02, Act::DrumSwap(2)),
            (0x03, Act::DrumSwap(3)),
            (0x08, Act::DrumCapture),
            (0x24, Act::DrumRelease),
        ] {
            replay.send([note_on, note, 127]);
            assert_eq!(replay.value(action, deck), Val::On(true, 127));
            replay.send([note_on, note, 0]);
            assert_eq!(replay.value(action, deck), Val::On(false, 0));
        }
    }
    // Button feedback follows the resulting slot and capture state. A held lever has no LED.
    assert_eq!(replay.device.out.lock().queue.pop(), None);
}

#[test]
fn gain_joins_the_two_published_ccs_independently_on_all_four_channels() {
    let replay = Replay::new();
    let cases = [
        (0x00, 0x00, 0),
        (0x40, 0x00, 8192),
        (0x7f, 0x7f, 16383),
        (0x28, 0x13, 5139),
    ];
    for round in 0..4 {
        // Leave a different MSB pending on each deck, then deliver the LSBs in reverse order.
        for channel in 0..4u8 {
            let (msb, _, _) = cases[(round + usize::from(channel)) % cases.len()];
            replay.send([0xB0 | channel, 0x12, msb]);
        }
        assert!(replay.events.is_empty());
        for channel in (0..4u8).rev() {
            let (_, lsb, raw) = cases[(round + usize::from(channel)) % cases.len()];
            replay.send([0xB0 | channel, 0x32, lsb]);
            let val = replay.value(Act::DrumGain, channel + 1);
            assert_eq!(
                val,
                Val::Abs {
                    v: raw,
                    lo: 0,
                    mid: 8192,
                    hi: 16383
                }
            );
            match raw {
                0 => assert_eq!(val.unit(), Some(0.0)),
                8192 => assert_eq!(val.unit(), Some(0.5)),
                16383 => assert_eq!(val.unit(), Some(1.0)),
                _ => {}
            }
        }
    }
}

#[test]
fn groove_packets_update_app_gain_slots_capture_and_led_feedback() {
    // A fresh process isolates the library, engine globals and settings from other tests.
    const TEST: &str =
        "midi::grv6_groove_tests::groove_packets_update_app_gain_slots_capture_and_led_feedback";
    const CHILD: &str = "RECORDBOX_GRV6_TEST_PROFILE";
    let Some(profile) = std::env::var_os(CHILD) else {
        let tmp = tempfile::tempdir().unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, tmp.path())
            .env("PREPDECK_DATA", tmp.path())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(
            child.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&child.stderr)
        );
        return;
    };
    assert_eq!(std::env::var_os("PREPDECK_DATA"), Some(profile.clone()));
    let mut app = App::open_library(
        &Default::default(),
        None,
        crate::DeckOptions {
            disable_play: true,
            disable_load: false,
        },
    );
    // The Player was constructed without opening an audio stream. Enable the UI commands
    // without starting the player, MIDI scanner, UI poll loop or model-download services.
    app.deck_options.disable_play = false;
    app.prefs.app_mode = 1;
    app.prefs.perf_four = true;
    app.prefs.show_stems = true;
    app.prefs.groove.enable = true;
    app.prefs.groove.panel = true;
    app.prefs.groove.auto_import = false;
    app.prefs.groove.capture_dir = Path::new(&profile)
        .join("captures")
        .to_string_lossy()
        .into_owned();

    let replay = Replay::with_decks(app.player.decks.clone());
    replay.hub.devices.lock().push(replay.device.clone());
    app.midi.rx = Some(replay.events.clone());
    let startup = replay.feedback(&mut app);
    assert_eq!(startup.len(), 4 * 5);
    assert!(startup.iter().all(|msg| msg[2] == 0));
    for (n, engine) in crate::groove::ENGINES.iter().enumerate() {
        *engine.lock() = Some(Box::new(Engine::new(n)));
    }
    app.groove_poll();

    let clip = GcClip {
        audio: Arc::new(crate::audio::Decoded {
            sr: 44100,
            samples: vec![0.0; 256],
            raw_frames: 128,
            damaged: 0,
            truncated: false,
        }),
        bpm: 120.0,
        gain: 1.0,
        first_beat: 0.0,
    };
    for channel in 0..4u8 {
        let n = usize::from(channel);
        let bank = n % 2;
        app.gc.set.decks[n].bank = bank as u8;
        {
            let mut engine = crate::groove::ENGINES[n].lock();
            for row in 0..4 {
                engine
                    .as_mut()
                    .unwrap()
                    .set_clip(row, bank, Some(clip.clone()));
                app.gc.wave[n][4 * bank + row] = Some(vec![0; 16]);
            }
        }
        for (msb, lsb, percent) in [
            (0, 0, 0.0),
            (64, 0, 50.0),
            (40, 19, 50.0 * 5139.0 / 8192.0),
            (127, 127, 100.0),
        ] {
            let old = app.gc.gain;
            replay.send([0xb0 | channel, 0x12, msb]);
            assert!(replay.feedback(&mut app).is_empty());
            assert_eq!(app.gc.gain, old);
            replay.send([0xb0 | channel, 0x32, lsb]);
            assert!(replay.feedback(&mut app).is_empty());
            let mut wanted = old;
            wanted[n] = percent;
            assert_eq!(app.gc.gain, wanted);
            app.groove_poll();
            assert_eq!(
                crate::groove::ENGINES[n].lock().as_ref().unwrap().gain,
                crate::groove::knob_gain(percent)
            );
        }
        for note in 0..4u8 {
            let slot = bank * 4 + usize::from(note);
            assert_eq!(
                replay.click(&mut app, channel, note),
                [[0x90 | channel, note, 127]]
            );
            assert!(app.groove_is_played(n, slot));
            assert!(!app.groove_is_played(n, (slot + 4) % 8));
            assert_eq!(
                replay.click(&mut app, channel, 0x08),
                [[0x90 | channel, 0x08, 127]]
            );
            assert_eq!(app.gc.capture_on, Some(channel));
            // A rejected capture (no source track) must not toggle the running drum loop.
            assert!(replay.click(&mut app, channel, note).is_empty());
            assert!(app.groove_is_played(n, slot));
            assert_eq!(
                replay.click(&mut app, channel, 0x08),
                [[0x90 | channel, 0x08, 0]]
            );
            assert_eq!(app.gc.capture_on, None);
            assert_eq!(
                replay.click(&mut app, channel, note),
                [[0x90 | channel, note, 0]]
            );
            assert!(!app.groove_is_played(n, slot));
        }
    }
    assert_eq!(replay.click(&mut app, 3, 0x08), [[0x93, 0x08, 127]]);
    app.prefs.perf_four = false;
    app.groove_poll();
    assert_eq!(app.gc.capture_on, None);
    assert_eq!(replay.feedback(&mut app), [[0x93, 0x08, 0]]);
    app.deck_options.disable_load = true;
    assert!(replay.click(&mut app, 0, 0x08).is_empty());
    assert_eq!(app.gc.capture_on, None);
    app.deck_options.disable_play = true;
    assert!(replay.click(&mut app, 0, 0x00).is_empty());
    assert!(!app.groove_is_played(0, 0));
    assert!(!Path::new(&app.prefs.groove.capture_dir).exists());
}

#[test]
fn coverage_counts_seven_groove_inputs_and_five_feedback_rows() {
    let file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/studies/DDJ-GRV6-groove-check.csv");
    let mut output = Vec::new();
    crate::midi_cover::report(&file, &mut output).unwrap();
    let report = std::str::from_utf8(&output).unwrap();
    for line in [
        "Mapping rows: 7, unassigned: 0",
        "Input rows: 7 supported, 0 unsupported, 0 disabled",
        "Output rows: 5 supported, 0 unsupported, 0 disabled",
    ] {
        assert!(report.lines().any(|l| l == line), "{report}");
    }
}
