//! Replay published DDJ-GRV6 messages through the same CSV import and MIDI routing as a
//! connected controller. Read feedback from the port's queue before it reaches the OS.
use super::*;
use crate::groove::{Engine, GcClip};

const DEVICE: &str = "DDJ-GRV6";
const FIXTURE: &[u8] = include_bytes!("../../../docs/studies/DDJ-GRV6-groove-check.csv");
const SWAPS: [&str; 4] = ["DrumSwap1", "DrumSwap2", "DrumSwap3", "DrumSwap4"];

struct Replay {
    hub: Arc<Hub>,
    device: Arc<Device>,
    events: crossbeam_channel::Receiver<UiEvent>,
}

impl Replay {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let decks = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
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

    fn check_light(&self, name: &str, deck: u8, state: Led, expected: [u8; 3]) {
        self.device.set_led(registry().tag(name), deck, state);
        let mut port = self.device.out.lock();
        assert_eq!(port.queue.pop(), Some(expected), "{name} on deck {deck}");
        assert_eq!(port.queue.pop(), None);
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
fn slot_and_capture_states_encode_the_published_led_notes_on_each_deck() {
    let replay = Replay::new();
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
    assert_eq!(replay.device.out_keys().len(), 4 * 5);
    for channel in 0..4 {
        let deck = channel + 1;
        let mut engine = Engine::new(usize::from(channel));
        let bank = channel % 2;
        for (row, name) in SWAPS.iter().enumerate() {
            engine.set_clip(row, usize::from(bank), Some(clip.clone()));
            assert!(engine.launch(row, usize::from(bank)));
            let slot_lights = drum_slot_lights(Some(&engine), bank);
            assert_eq!(slot_lights.iter().filter(|&&s| s == Led::On).count(), 1);
            assert_eq!(drum_slot_lights(Some(&engine), 1 - bank), [Led::Off; 4]);
            let note = [0x00, 0x01, 0x02, 0x03][row];
            replay.check_light(name, deck, slot_lights[row], [0x90 | channel, note, 127]);
            engine.stop(row);
            replay.check_light(
                name,
                deck,
                drum_slot_lights(Some(&engine), bank)[row],
                [0x90 | channel, note, 0],
            );
        }
        let armed = drum_capture_light(Some(channel), channel);
        replay.check_light("DrumSwapCapture", deck, armed, [0x90 | channel, 0x08, 127]);
        let other_deck_armed = drum_capture_light(Some((channel + 1) % 4), channel);
        replay.check_light(
            "DrumSwapCapture",
            deck,
            other_deck_armed,
            [0x90 | channel, 0x08, 0],
        );
        assert_eq!(drum_capture_light(None, channel), Led::Off);
    }
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
