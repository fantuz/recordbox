//! Generic controllers, based on the upstream universal MIDI and virtual rig scenarios.
//! Exercises mapping, routing and queued feedback without opening any system MIDI ports.
use super::*;

fn learned(name: &str, deck: u8, kind: Kind) -> LearnEntry {
    LearnEntry { name: name.into(), deck, kind, min: -1, max: -1, comment: String::new() }
}

#[test]
fn every_learn_menu_command_survives_export_and_reaches_its_own_binding() {
    let name = "Universal test controller";
    let reg = registry();
    let mut map = MidiMap::new(name);
    let mut expected = Vec::new();
    let (mut notes, mut controls) = (0u16, 0u16);
    for (tab, _) in TABS {
        for f in reg.of_tab(tab) {
            let kind = kind_of_type(f.ty);
            let code = match kind {
                Kind::Button | Kind::Pad(_) | Kind::Indicator => {
                    assert!(notes < 16 * 128);
                    let code = 0x9000 | (notes / 128) << 8 | notes % 128;
                    notes += 1;
                    code
                }
                _ => {
                    // Reserve CC 32..63 on each channel for fine adjustments.
                    assert!(controls < 16 * 32);
                    let code = 0xb000 | (controls / 32) << 8 | controls % 32;
                    controls += 1;
                    code
                }
            };
            let row = learned(&f.name, u8::from(f.per_deck), kind);
            let result = if kind.output_only() {
                map.change_output(&row, None, Some(code), &|n| reg.learnable(n))
            } else {
                map.change_input(&row, None, Some(code), &|n| reg.learnable(n))
            };
            result.unwrap_or_else(|e| panic!("could not learn {} at {code:04X}: {e:?}", f.name));
            expected.push((row, code));
        }
    }
    let bytes = map.save_text();
    let mut imported = MidiMap::load(name, None, Some(bytes.as_bytes())).unwrap();
    imported.tag_inputs(|n| reg.tag(n));
    for (row, code) in expected {
        if row.kind.output_only() {
            let outs = imported.outputs_at(code);
            assert_eq!(outs, [row.entry(code, name)], "{} output", row.name);
            continue;
        }
        assert_eq!(imported.input_at(code), Some(&row.entry(code, name)));
        let [status, data] = code.to_be_bytes();
        let mut event = imported.handle(&[status, data, 1], 0.0);
        if row.kind.is_14bit() {
            assert!(event.is_none(), "{} requires the low byte", row.name);
            event = imported.handle(&[status, data + 32, 2], 1.0);
        }
        let event = event.unwrap_or_else(|| panic!("no event for {} at {code:04X}", row.name));
        assert_eq!((reg.get(event.tag).unwrap().name.as_str(), event.deck), (row.name.as_str(), row.deck));
    }
}

fn generic_hub(dir: &Path) -> (Arc<Hub>, crossbeam_channel::Receiver<UiEvent>) {
    let decks = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
    let (mut hub, events) = Hub::new(decks, dir.into(), true);
    let setup = Arc::get_mut(&mut hub).unwrap();
    setup.native_user_dirs.clear();
    setup.factory_dirs.clear();
    setup.routing.lock().performance = true;
    (hub, events)
}

fn decoded(secs: usize) -> Arc<crate::audio::Decoded> {
    let frames = 44100 * secs;
    Arc::new(crate::audio::Decoded {
        sr: 44100,
        samples: vec![0.1; frames * 2],
        raw_frames: frames,
        damaged: 0,
        truncated: false,
    })
}

#[test]
fn two_reconnected_controllers_keep_identical_wires_separate_for_decks_and_lamps() {
    let tmp = tempfile::tempdir().unwrap();
    let profiles = [("Left controller", 1, 0x9201u16), ("Right controller", 2, 0x9202)];
    let (setup, _) = generic_hub(tmp.path());
    let learnable = |n: &str| registry().learnable(n);
    for (name, deck, led) in profiles {
        let dev = Device::mapped(name, name.into(), MidiMap::new(name), false, None);
        let play = learned("PlayPause", deck, Kind::Button);
        let gain = learned("Gain", deck, Kind::KnobSliderHiRes);
        setup.try_edit_mapping(&dev, |m| m.change_input(&play, None, Some(0x900b), &learnable)).unwrap();
        setup.try_edit_mapping(&dev, |m| m.change_output(&play, None, Some(led), &learnable)).unwrap();
        setup.try_edit_mapping(&dev, |m| m.change_input(&gain, None, Some(0xb013), &learnable)).unwrap();
        setup.save_map(&dev).unwrap();
    }
    drop(setup);

    let (hub, events) = generic_hub(tmp.path());
    let clip = decoded(1);
    let [left, right] = profiles.map(|(name, deck, _)| {
        let (map, factory) = hub.load_map(name);
        assert!(!factory);
        let dev = Device::mapped(name, name.into(), map, factory, None);
        dev.seed_output(ports::Endpoint { name: name.into(), id: name.into() });
        hub.decks[usize::from(deck - 1)].lock().audio = Some(clip.clone());
        hub.devices.lock().push(dev.clone());
        dev
    });
    let send = |dev, packet| hub.input_message(dev, packet, 0.0, 0, 0);
    for (dev, other, i) in [(&left, &right, 0usize), (&right, &left, 1)] {
        send(dev, [0x90, 0x0b, 127]);
        assert!(hub.decks[i].lock().playing);
        assert_eq!(hub.decks[1 - i].lock().playing, i == 1);
        let [status, note] = profiles[i].2.to_be_bytes();
        assert_eq!(dev.out.lock().queue.pop(), Some([status, note, 127]));
        assert!(other.out.lock().queue.pop().is_none());
        assert!(matches!(events.try_recv(), Ok(UiEvent::Control { device, tag, deck, fast_done: true, .. })
            if device == dev.name && tag == registry().tag("PlayPause") && deck == profiles[i].1));
    }

    // Identical CC numbers on both ports. The fine adjustments arrive in reverse order.
    send(&left, [0xb0, 0x13, 1]);
    send(&right, [0xb0, 0x13, 2]);
    assert!(events.try_recv().is_err());
    for (dev, deck, fine, expected) in [(&right, 2, 3, 259), (&left, 1, 4, 132)] {
        send(dev, [0xb0, 0x33, fine]);
        assert!(matches!(events.try_recv(), Ok(UiEvent::Control {
            device, deck: destination, tag, val: Val::Abs { v, .. }, fast_done: true, ..
        }) if device == dev.name && destination == deck && tag == registry().tag("Gain") && v == expected));
    }
    for (index, value) in [132, 259].into_iter().enumerate() {
        assert_eq!(hub.decks[index].lock().trim, crate::mixer::trim_gain(value as f32 / 16384.0));
    }
    assert!(events.try_recv().is_err());
}

#[test]
fn ergo_half_turn_moves_the_audio_half_a_revolution_in_either_direction() {
    let tmp = tempfile::tempdir().unwrap();
    let (hub, _events) = generic_hub(tmp.path());
    let name = "PIONEER DDJ-ERGO";
    let csv = format!(
        "@file,1,{name}\r\n\
        JogTouch,,JogTouch,9036,0,,,,,,,,,RO;,\r\n\
        JogScratch,,JogRotate,B022,0,,,,,,,,,RO;,\r\n"
    );
    let dev = Device::mapped(name, name.into(), MidiMap::load(name, Some(csv.as_bytes()), None).unwrap(), true, None);
    const RATE: u32 = 44_100;
    let start = f64::from(RATE);
    {
        let mut deck = hub.decks[0].lock();
        deck.audio = Some(decoded(3));
        deck.pos = start;
    }
    let mut buffer = [0.0; 512];
    let render = |count, buffer: &mut [f32]| {
        let mut deck = hub.decks[0].lock();
        for _ in 0..count {
            buffer.fill(0.0);
            crate::audio::render_deck(&mut deck, buffer, 2, RATE);
        }
    };
    let mut now = 1000.0;
    // At 33 1/3 rpm the ERGO's 1,150 counts are 0.9 seconds. Supply audio callbacks
    // throughout each gesture. Extra callbacks at the end must not extend the motion.
    for (byte, destination) in [(64 + 23, start * 1.9), (64 - 23, start)] {
        hub.input_message(&dev, [0x90, 0x36, 127], now, 0, 0);
        for _ in 0..50 {
            now += 5.0;
            hub.input_message(&dev, [0xb0, 0x22, byte], now, 0, 0);
            render(1, &mut buffer);
        }
        render(20, &mut buffer);
        let pos = hub.decks[0].lock().pos;
        assert!((pos - destination).abs() < 0.01, "expected frame {destination}, got {pos}");
        now += 100.0;
        hub.input_message(&dev, [0x80, 0x36, 0], now, 0, 0);
        render(5, &mut buffer);
        let deck = hub.decks[0].lock();
        assert_eq!(deck.dk.jog, crate::deck::Jog::None);
        assert!(!deck.playing);
        assert_eq!(deck.pos, pos);
    }
}
