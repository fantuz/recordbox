//! Serialize worker actions with mode changes and keep the release of a momentary
//! control tied to the controller and deck that received its press.
use super::*;

/// Absolute-position controllers use a hardware clock for rotation speed. Its origin
/// can differ from the MIDI service's clock by hours. Measure inactivity on the host,
/// then present the elapsed time in the wheel's own timestamp domain.
pub(super) struct Platter {
    pub(super) jog: Jog,
    last_rotation: Option<(f64, Instant)>,
}

impl Platter {
    pub(super) fn new(cpr: f64) -> Self {
        Self {
            jog: Jog::new(cpr),
            last_rotation: None,
        }
    }

    pub(super) fn rotate(
        &mut self,
        delta: i32,
        timestamp_ms: f64,
        scratch: bool,
        at: Instant,
    ) -> JogOut {
        self.last_rotation = Some((timestamp_ms, at));
        self.jog.rotate(delta, timestamp_ms, scratch)
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        let (timestamp, at) = self.last_rotation?;
        let remaining_ms = (self.jog.idle_deadline_ms()? - timestamp).max(0.0);
        Some(at + Duration::from_micros((remaining_ms * 1000.0).ceil() as u64))
    }

    fn idle(&mut self, now: Instant) -> JogOut {
        let Some(deadline) = self.deadline() else { return JogOut::Nothing };
        if now < deadline { return JogOut::Nothing; }
        let (timestamp, at) = self.last_rotation.unwrap();
        // Round forward if a hardware tick falls between representable host instants.
        let in_clock = (timestamp + now.saturating_duration_since(at).as_secs_f64() * 1000.0)
            .max(self.jog.idle_deadline_ms().unwrap());
        let out = self.jog.idle(in_clock);
        if self.jog.idle_deadline_ms().is_none() {
            self.last_rotation = None;
        }
        out
    }

    fn reset(&mut self) {
        *self = Self::new(self.jog.cpr);
    }
}

pub(super) fn browsing(act: Act) -> bool {
    matches!(
        act,
        Act::Browse
            | Act::BrowseStep(_)
            | Act::Forward
            | Act::Back
            | Act::TagList
            | Act::Related
            | Act::Area
            | Act::Sort(_)
    )
}

pub(super) fn momentary(act: Act) -> bool {
    matches!(
        act,
        Act::Cue
            | Act::HotCue(_)
            | Act::Search(_)
            | Act::PitchBend(_)
            | Act::SamplerPad(..)
            | Act::FxReleaseOn(_)
            | Act::DrumRelease
            | Act::FxTapLong(_)
            | Act::FxAssignHold(..)
            | Act::Shift
            | Act::JogTouch
            | Act::Stub(Stub::PadFx(..) | Stub::SlicerPad(..) | Stub::SlipReverse)
    )
}

struct Held {
    dev: Arc<Device>,
    event: Event,
    /// Physical binding at press time for a held Drum Release FX lever. Importing
    /// a replacement map can reuse the entry index for a different input.
    drum_binding: Option<(u16, u8, Kind)>,
}

fn is_drum_release(event: &Event) -> bool {
    registry().get(event.tag).is_some_and(|f| f.act == Act::DrumRelease)
}

fn drum_binding(dev: &Device, event: &Event) -> Option<(u16, u8, Kind)> {
    let map = dev.map.lock();
    let input = map.inputs.get(event.entry)?.as_ref()?;
    (input.tag == event.tag).then_some((input.e.code, input.e.deck, input.e.kind))
}

#[derive(Default)]
pub(super) struct Routing {
    pub(super) performance: bool,
    held: Vec<Held>,
    jog_owner: [Option<Arc<Device>>; 4],
}

impl Routing {
    pub(super) fn search_held(&self, deck: u8) -> bool {
        self.held.iter().any(|held| {
            Some(held.event.deck) == deck.checked_add(1)
                && registry().get(held.event.tag).is_some_and(|f| matches!(f.act, Act::Search(_)))
        })
    }

    // Multiple controllers may hold the same function. Only the last release stops
    // it. Different hot cues still retrigger, but share a deck's single cue gate.
    pub(super) fn hold(&mut self, dev: &Arc<Device>, event: &mut Event) -> bool {
        let Val::On(down, _) = event.val else {
            return true;
        };
        let previous = self
            .held
            .iter()
            .position(|h| Arc::ptr_eq(&h.dev, dev) && h.event.entry == event.entry);
        if down {
            if previous.is_some() {
                return false;
            }
            let independent = registry()
                .get(event.tag)
                .is_some_and(|f| f.act == Act::JogTouch);
            let shared = self.held.iter().find(|h| {
                !independent && h.event.deck == event.deck && same_release(h.event.tag, event.tag)
            });
            let duplicate = shared.is_some_and(|h| h.event.tag == event.tag);
            if let Some(first) = shared {
                // Mixing queued and fast releases can otherwise stop a preview before the
                // UI has handled its press. Use one ordering for all owners of the gate.
                event.fast = first.event.fast;
            }
            self.held.push(Held {
                dev: dev.clone(),
                event: *event,
                drum_binding: if is_drum_release(event) { drum_binding(dev, event) } else { None },
            });
            return !duplicate;
        }
        let Some(i) = previous else { return false };
        let old = self.held.swap_remove(i).event;
        *event = Event {
            val: event.val,
            ..old
        };
        registry()
            .get(old.tag)
            .is_some_and(|f| f.act == Act::JogTouch)
            || !self
                .held
                .iter()
                .any(|h| h.event.deck == old.deck && same_release(h.event.tag, old.tag))
    }
}

fn same_release(a: u32, b: u32) -> bool {
    if a == b {
        return true;
    }
    let action = |t| registry().get(t).map(|f| f.act);
    matches!(
        (action(a), action(b)),
        (Some(Act::HotCue(_)), Some(Act::HotCue(_)))
            | (Some(Act::PitchBend(_)), Some(Act::PitchBend(_)))
    )
}

impl Hub {
    pub(super) fn idle_jogs(&self, dev: &Arc<Device>, now: Instant) {
        let mut route = self.routing.lock();
        for n in 0..4 {
            let out = dev.jog.lock()[n].idle(now);
            self.routed_jog(&mut route, dev, n, out, true);
        }
    }

    pub(super) fn routed_jog(
        &self,
        route: &mut Routing,
        dev: &Arc<Device>,
        n: usize,
        out: JogOut,
        idle: bool,
    ) {
        if !self.playback_enabled || out == JogOut::Nothing {
            return;
        }
        let owner = &mut route.jog_owner[n];
        let mine = owner.as_ref().is_some_and(|d| Arc::ptr_eq(d, dev));
        let ending = matches!(out, JogOut::Release | JogOut::Spin(_) | JogOut::Bend(0.0));
        if (idle || ending) && !mine {
            return;
        }
        if ending {
            *owner = None;
        } else if !mine {
            // A new platter takes control from the old one, including its bend/scratch.
            if owner.is_some() {
                self.reset_jog(n);
            }
            *owner = Some(dev.clone());
        }
        self.jog_out(n, out);
    }

    fn reset_jog(&self, n: usize) {
        self.jog_out(n, JogOut::Release);
        self.jog_out(n, JogOut::Bend(0.0));
    }

    // A final release uses the same fast/UI delivery as a physical release.
    fn release_matching(&self, route: &mut Routing, matches: impl Fn(&Held) -> bool) {
        while let Some(h) = route.held.iter().find(|h| matches(h)) {
            let dev = h.dev.clone();
            let mut release = Event {
                val: Val::On(false, 0),
                ..h.event
            };
            if route.hold(&dev, &mut release) {
                self.deliver(route, &dev, release);
            }
        }
    }

    /// A removed, reassigned or synthetic peer binding may no longer be able
    /// to deliver the lever's release. An unrelated output edit keeps a
    /// physical lever on the same input, deck and message type.
    pub(crate) fn try_edit_mapping(
        &self,
        dev: &Arc<Device>,
        edit: impl FnOnce(&mut MidiMap) -> Result<(), crate::midi_map::Reject>,
    ) -> Result<(), crate::midi_map::Reject> {
        // Keep MIDI input outside the interval between changing the table and
        // tagging its entries, including when an import keeps the same binding.
        let mut route = self.routing.lock();
        if dev.profile_trial_active() {
            return Err(crate::midi_map::Reject::Revert);
        }
        edit(&mut dev.map.lock())?;
        dev.reindex();
        self.release_matching(&mut route, |h| {
            Arc::ptr_eq(&h.dev, dev) && is_drum_release(&h.event)
                && (h.drum_binding.is_none() || h.drum_binding != drum_binding(dev, &h.event))
        });
        Ok(())
    }

    pub(crate) fn edit_mapping(&self, dev: &Arc<Device>, edit: impl FnOnce(&mut MidiMap)) {
        let _ = self.try_edit_mapping(dev, |map| {
            edit(map);
            Ok(())
        });
    }

    /// A manual, reversible switch for this connection. The usual factory and
    /// user loaders also remain authoritative when the device next connects.
    pub(crate) fn set_profile_trial(&self, dev: &Arc<Device>, on: bool) {
        let Some(profile) = crate::midi_csv::trial_profile(&dev.name) else { return };
        if dev.needs_auth {
            return;
        }
        let mut route = self.routing.lock();
        let mut original = dev.profile_original.lock();
        if !dev.alive.load(Relaxed) || original.is_some() == on {
            return;
        }
        dev.input_epoch.fetch_add(1, Relaxed);
        dev.learning.store(false, Relaxed);
        self.release_holds(&mut route, Some(dev));
        *dev.load_at.lock() = Default::default();
        let mut map = dev.map.lock();
        let mut offs = Vec::new();
        if let Some(previous) = original.take() {
            if dev.has_output {
                // A pending OFF for one of the trial's lamps
                // may still be queued when the old map is restored.
                offs.extend(map.outputs.iter().map(|e| (map.led_message(e, 0), e.opts.priority)));
            }
            *map = previous;
        } else {
            let csv: &[u8] = match profile {
                crate::midi_csv::TrialProfile::Rx3 => include_bytes!("profiles/XDJ-RX3.csv"),
                crate::midi_csv::TrialProfile::OpusQuad => include_bytes!("profiles/OPUS-QUAD.csv"),
            };
            let pilot = MidiMap::load(&dev.name, Some(csv), None).expect("built-in MIDI profile");
            *original = Some(std::mem::replace(&mut *map, pilot));
        }
        dev.reindex_map(&mut map, &offs);
    }

    pub(super) fn release_hidden_drum_decks(&self, count: u8) {
        self.release_matching(&mut self.routing.lock(), |h| h.event.deck > count && is_drum_release(&h.event));
    }

    fn release_holds(&self, route: &mut Routing, device: Option<&Arc<Device>>) {
        let matches = |d: &Arc<Device>| device.is_none_or(|who| Arc::ptr_eq(who, d));
        self.release_matching(route, |h| matches(&h.dev));
        for (n, owner) in route.jog_owner.iter_mut().enumerate() {
            if owner.as_ref().is_some_and(&matches) {
                *owner = None;
                self.reset_jog(n);
            }
        }
        if let Some(dev) = device {
            dev.map.lock().reset_dual();
            *dev.scan.lock() = Default::default();
            for jog in dev.jog.lock().iter_mut() {
                jog.reset();
            }
        }
    }

    pub(super) fn release_device(&self, dev: &Arc<Device>) {
        self.release_holds(&mut self.routing.lock(), Some(dev));
    }

    pub(crate) fn set_learning(&self, device: &Arc<Device>, on: bool) {
        let mut route = self.routing.lock();
        if on && device.profile_trial_active() { return; }
        if device.learning.swap(on, Relaxed) != on {
            device.input_epoch.fetch_add(1, Relaxed);
            if on {
                device.clear_button_feedback();
                self.release_holds(&mut route, Some(device));
            }
        }
    }
}

/// Apply held releases in the old layout, before its decks are parked or hidden.
/// The port worker cannot sneak a fast transport action across the boundary.
pub(crate) fn change_mode(app: &mut App, mode: u8) {
    let performance = mode == 1;
    let Some(hub) = app.midi.hub.clone() else {
        app.prefs.app_mode = mode;
        return;
    };
    {
        let mut route = hub.routing.lock();
        if route.performance && !performance {
            hub.mode_epoch.fetch_add(1, Relaxed);
            route.performance = false;
            hub.release_holds(&mut route, None);
            for dev in hub.devices.lock().iter() {
                dev.clear_button_feedback();
                dev.map.lock().reset_dual();
                *dev.scan.lock() = Default::default();
                for jog in dev.jog.lock().iter_mut() {
                    jog.reset();
                }
            }
        }
    }
    if app.performance() != performance {
        apply_pending(app, &hub);
        if !performance {
            // The confirmed mode change stops transport. A queued press may have
            // been issued after the header's initial stop and before the gate closed.
            for deck in &app.player.decks {
                deck.lock().playing = false;
            }
        }
        app.midi.pending_double = None;
        hub.overlay.lock().clear();
    }
    app.prefs.app_mode = mode;
    let mut route = hub.routing.lock();
    *hub.snap.lock() = snapshot(app);
    if route.performance != performance {
        hub.mode_epoch.fetch_add(1, Relaxed);
    }
    route.performance = performance;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> (Arc<Hub>, crossbeam_channel::Receiver<UiEvent>) {
        let decks = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let (hub, rx) = Hub::new(decks, PathBuf::new(), true);
        hub.decks[0].lock().audio = Some(Arc::new(crate::audio::Decoded {
            sr: 44100,
            samples: vec![0.0; 200],
            raw_frames: 100,
            damaged: 0,
            truncated: false,
        }));
        (hub, rx)
    }

    fn controller(name: &str) -> Arc<Device> {
        let text = format!("@file,1,{name}\r\nPlayPause,,Button,,900B,,,,,,,,,Fast;,\r\n");
        let map = MidiMap::load(name, None, Some(text.as_bytes())).unwrap();
        Device::mapped(name, name.to_owned(), map, false, None)
    }

    fn press(h: &Hub, d: &Arc<Device>, name: &str, deck: u8, down: bool) {
        let tag = registry().tag(name);
        assert_ne!(tag, 0);
        let ev = Event {
            entry: tag as usize,
            tag,
            deck,
            fast: true,
            val: Val::On(down, if down { 127 } else { 0 }),
        };
        h.dispatch(&mut h.routing.lock(), d, ev);
    }

    #[test]
    fn fast_play_requires_performance_and_learn_does_not_dispatch_its_old_binding() {
        let (hub, rx) = hub();
        let dev = controller("A");
        let play = [0x90, 0x0b, 127];
        hub.input_message(&dev, play, 0.0, 0, 0);
        assert!(rx.try_recv().is_err());
        assert!(!hub.decks[0].lock().playing);

        hub.routing.lock().performance = true;
        hub.set_learning(&dev, true);
        hub.input_message(&dev, play, 1.0, 0, 1);
        assert!(matches!(rx.try_recv(), Ok(UiEvent::Learn { msg, .. }) if msg == play));
        assert!(rx.try_recv().is_err());
        assert!(!hub.decks[0].lock().playing);

        hub.set_learning(&dev, false);
        // A busier port may have queued a press while still in Learn, or in the previous mode.
        hub.input_message(&dev, play, 1.8, 0, 1);
        hub.mode_epoch.store(1, Relaxed);
        hub.input_message(&dev, play, 1.9, 0, 2);
        assert!(rx.try_recv().is_err());
        hub.input_message(&dev, play, 2.0, 1, 2);
        assert!(hub.decks[0].lock().playing);
        assert!(matches!(
            rx.try_recv(),
            Ok(UiEvent::Control {
                fast_done: true,
                ..
            })
        ));
    }

    #[test]
    fn disconnect_releases_only_the_last_cue_holder_on_the_original_deck() {
        let (hub, _) = hub();
        hub.routing.lock().performance = true;
        let (a, b) = (controller("A"), controller("B"));
        press(&hub, &a, "Cue", 0, true);
        press(&hub, &b, "Cue", 0, true);
        assert!(hub.decks[0].lock().playing);
        hub.snap.lock().active = 1;
        hub.release_device(&a);
        assert!(hub.decks[0].lock().playing);
        hub.set_learning(&b, true);
        assert!(!hub.decks[0].lock().playing);
        assert!(hub.routing.lock().held.is_empty());
    }

    #[test]
    fn rx3_trial_releases_a_held_cue_drops_old_input_and_resets_only_its_lamps() {
        let (hub, _) = hub();
        hub.routing.lock().performance = true;
        let mut rx3 = controller("XDJ-RX3");
        Arc::get_mut(&mut rx3).unwrap().has_output = true;
        let peer = controller("Other controller");
        hub.set_profile_trial(&peer, true);
        assert!(!peer.profile_trial_active());
        let tag = registry().tag("PlayPause");
        hub.set_profile_trial(&rx3, true);
        assert!(rx3.profile_trial_active());
        assert_eq!(rx3.map.lock().live_inputs().count(), 39);
        assert_eq!(rx3.out_keys().len(), 4);
        hub.input_message(&rx3, [0x90, 0x01, 127], 0.0, 0, 0);
        assert!(!hub.decks[0].lock().playing, "queued before the trial");
        hub.input_message(&rx3, [0x90, 0x01, 127], 1.0, 0, 1);
        assert!(hub.decks[0].lock().playing);
        rx3.set_led(tag, 2, Led::On);
        hub.set_profile_trial(&rx3, false);
        assert!(!rx3.profile_trial_active());
        assert!(hub.routing.lock().held.is_empty());
        assert!(!hub.decks[0].lock().playing);
        assert_eq!(rx3.map.lock().live_inputs().count(), 1);
        let mut out = rx3.out.lock();
        let mut messages: Vec<_> = std::iter::from_fn(|| out.queue.pop()).collect();
        messages.sort();
        assert_eq!(messages, [[0x90, 0, 0], [0x90, 1, 0], [0x91, 0, 0], [0x91, 1, 0]]);
        drop(out);
        hub.input_message(&rx3, [0x90, 0x0B, 127], 2.0, 0, 1);
        assert!(!hub.decks[0].lock().playing, "queued during the trial");
        hub.input_message(&peer, [0x90, 0x0B, 127], 3.0, 0, 0);
        assert!(hub.decks[0].lock().playing, "other port keeps its epoch and map");
    }

    #[test]
    fn opus_quad_fourth_deck_plays_and_trial_stop_restores_the_user_map() {
        let (hub, _) = hub();
        hub.routing.lock().performance = true;
        hub.snap.lock().count = 4;
        let audio = hub.decks[0].lock().audio.clone();
        hub.decks[3].lock().audio = audio;
        let mut opus = controller("MIDIIN (OPUS-QUAD)");
        Arc::get_mut(&mut opus).unwrap().has_output = true;
        hub.set_profile_trial(&opus, true);
        assert!(opus.profile_trial_active());
        assert_eq!(opus.map.lock().live_inputs().count(), 162);
        assert_eq!(opus.out_keys().len(), 12);
        hub.input_message(&opus, [0x93, 0, 127], 1.0, 0, 1);
        assert!(hub.decks[3].lock().playing);
        assert!(!hub.decks[0].lock().playing);
        hub.set_profile_trial(&opus, false);
        assert!(!opus.profile_trial_active());
        assert_eq!(opus.map.lock().live_inputs().count(), 1);
        let mut out = opus.out.lock();
        let mut messages: Vec<_> = std::iter::from_fn(|| out.queue.pop()).collect();
        messages.sort();
        let expected: Vec<_> = (0x90..=0x93).flat_map(|ch| [[ch, 0, 0], [ch, 1, 0], [ch, 0x12, 0]]).collect();
        assert_eq!(messages, expected);
    }

    fn drum_map(name: &str, code: u16) -> MidiMap {
        // Assigned to the active deck; its physical note stays the same when focus moves.
        let csv = format!("@file,1,{name}\nDrumReleaseFx,,Button,{code:04X},,,,,,,,,,,\n");
        MidiMap::load(name, None, Some(csv.as_bytes())).unwrap()
    }

    fn drum_controller(name: &str) -> Arc<Device> {
        Device::mapped(name, name.to_owned(), drum_map(name, 0x9024), false, None)
    }

    fn drum_edges(rx: &crossbeam_channel::Receiver<UiEvent>) -> Vec<(u8, bool)> {
        rx.try_iter().map(|event| match event {
            UiEvent::Control { tag, deck, val: Val::On(on, _), .. } => {
                assert_eq!(registry().get(tag).unwrap().act, Act::DrumRelease);
                (deck, on)
            }
            _ => panic!("expected a drum release FX transition"),
        }).collect()
    }

    #[test]
    fn drum_levers_share_the_original_deck_and_learn_releases_the_final_owner() {
        let (hub, rx) = hub();
        hub.routing.lock().performance = true;
        let (a, b) = (drum_controller("A"), drum_controller("B"));
        hub.input_message(&a, [0x90, 0x24, 127], 1.0, 0, 0);
        hub.input_message(&b, [0x90, 0x24, 127], 2.0, 0, 0);
        assert_eq!(drum_edges(&rx), [(1, true)]);
        hub.snap.lock().active = 1;
        hub.release_device(&a);
        assert!(drum_edges(&rx).is_empty());
        hub.set_learning(&b, true);
        assert_eq!(drum_edges(&rx), [(1, false)]);
        // Note Off now belongs to Learn and must never stop the new deck or retrigger FX.
        hub.input_message(&b, [0x80, 0x24, 42], 3.0, 0, 1);
        assert!(matches!(rx.try_recv(), Ok(UiEvent::Learn { .. })));
        assert!(rx.try_recv().is_err() && hub.routing.lock().held.is_empty());
    }

    #[test]
    fn replacing_an_active_drum_binding_releases_even_if_its_index_is_reused() {
        let (hub, rx) = hub();
        hub.routing.lock().performance = true;
        let dev = drum_controller("Test");
        hub.input_message(&dev, [0x90, 0x24, 127], 0.0, 0, 0);
        assert_eq!(drum_edges(&rx), [(1, true)]);
        hub.edit_mapping(&dev, |map| *map = drum_map("Test", 0x9024));
        assert!(drum_edges(&rx).is_empty());
        hub.edit_mapping(&dev, |map| *map = drum_map("Test", 0x9025));
        assert_eq!(drum_edges(&rx), [(1, false)]);
        hub.input_message(&dev, [0x90, 0x24, 0], 1.0, 0, 0);
        hub.snap.lock().active = 1;
        hub.input_message(&dev, [0x90, 0x25, 127], 2.0, 0, 0);
        hub.input_message(&dev, [0x80, 0x25, 64], 3.0, 0, 0);
        assert_eq!(drum_edges(&rx), [(2, true), (2, false)]);
    }

    #[test]
    fn hiding_decks_releases_only_their_levers_and_mode_exit_releases_the_rest() {
        let (hub, rx) = hub();
        hub.routing.lock().performance = true;
        hub.snap.lock().count = 4;
        let (a, b) = (drum_controller("Front"), drum_controller("Rear"));
        hub.input_message(&a, [0x90, 0x24, 127], 0.0, 0, 0);
        hub.snap.lock().active = 3;
        hub.input_message(&b, [0x90, 0x24, 127], 1.0, 0, 0);
        assert_eq!(drum_edges(&rx), [(1, true), (4, true)]);
        hub.snap.lock().count = 2;
        hub.release_hidden_drum_decks(2);
        assert_eq!(drum_edges(&rx), [(4, false)]);
        let mut route = hub.routing.lock();
        route.performance = false;
        hub.release_holds(&mut route, None);
        assert_eq!(drum_edges(&rx), [(1, false)]);
        assert!(route.held.is_empty());
    }

    #[test]
    fn disconnect_after_replacing_a_track_preserves_its_transport_and_slip() {
        let (replacement, _) = hub();
        let (hub, _) = hub();
        hub.routing.lock().performance = true;
        let dev = controller("A");
        press(&hub, &dev, "Cue", 1, true);
        hub.snap.lock().decks[0].hot[0] = Some((0.0, None));
        press(&hub, &dev, "PAD1_HotCue", 1, true);
        let audio = replacement.decks[0].lock().audio.clone();
        {
            let mut deck = hub.decks[0].lock();
            deck.audio = audio;
            deck.playing = true;
            deck.slip_hold = true;
            deck.pos = 75.0;
        }
        hub.release_device(&dev);
        let deck = hub.decks[0].lock();
        assert!(deck.playing && deck.slip_hold);
        assert_eq!(deck.pos, 75.0);
        assert!(!hub.pad_held(0));
    }

    #[test]
    fn stale_platter_timeouts_and_disconnect_cannot_release_another_controller() {
        let (hub, _) = hub();
        hub.routing.lock().performance = true;
        let (a, b) = (controller("A"), controller("B"));
        {
            let mut route = hub.routing.lock();
            hub.routed_jog(&mut route, &a, 0, JogOut::Scratch(1.0), false);
            hub.routed_jog(&mut route, &b, 0, JogOut::Scratch(-1.0), false);
            hub.routed_jog(&mut route, &a, 0, JogOut::Scratch(0.0), true);
        }
        hub.release_device(&a);
        assert_eq!(hub.decks[0].lock().dk.jog, crate::deck::Jog::Speed(-1.0));
        hub.release_device(&b);
        assert_eq!(hub.decks[0].lock().dk.jog, crate::deck::Jog::None);
    }

    #[test]
    fn coremidi_mapping_routes_outer_wheel_cc_to_deck_bend_and_expires_it() {
        let dir = tempfile::tempdir().unwrap();
        let name = "recordbox test ctl";
        let (mut hub, rx) = hub();
        let setup = Arc::get_mut(&mut hub).unwrap();
        setup.user_dir = dir.path().into();
        setup.factory_dirs.clear();
        setup.native_user_dirs.clear();
        std::fs::write(setup.user_file(name), crate::midi::tests::virtual_controller_mapping(name)).unwrap();
        let (map, factory) = setup.load_map(name);
        let dev = Device::mapped(name, name.into(), map, factory, None);
        hub.routing.lock().performance = true;
        hub.snap.lock().decks[0].loaded = true;

        // Match the actual virtual controller's eight relative ticks. Controlled
        // MIDI timestamps let us observe the short bend without depending on OS scheduling.
        for tick in 0..8 {
            let timestamp = 123_000.0 + f64::from(tick * 8);
            hub.input_message(&dev, [0xB0, 0x23, 0x50], timestamp, 0, 0);
            let UiEvent::Control { tag, deck, val, fast_done, .. } = rx.try_recv().expect("mapped wheel tick") else {
                panic!("expected a jog control event");
            };
            assert_eq!(registry().get(tag).unwrap().act, Act::Jog(false));
            assert_eq!((deck, val, fast_done), (1, Val::Jog { delta: 16, t_ms: timestamp }, true));
        }
        let bend = hub.decks[0].lock().bend;
        assert!(bend > 0.0, "the mapped clockwise jog burst must bend deck 1");
        assert_eq!(hub.decks[1].lock().bend, 0.0);
        let end = dev.jog.lock()[0].deadline().expect("jog idle deadline");
        hub.idle_jogs(&dev, end - Duration::from_millis(2));
        assert_eq!(hub.decks[0].lock().bend, bend);
        hub.idle_jogs(&dev, end);
        assert_eq!(hub.decks[0].lock().bend, 0.0);
    }

    #[test]
    fn fader_messages_do_not_extend_a_jog_bend_deadline() {
        let (hub, _) = hub();
        hub.routing.lock().performance = true;
        let dev = controller("wheel");
        let now = Instant::now();
        let out = dev.jog.lock()[0].rotate(400, 123_000.0, false, now);
        hub.routed_jog(&mut hub.routing.lock(), &dev, 0, out, false);
        assert_ne!(hub.decks[0].lock().bend, 0.0);
        for time in (510..=580).step_by(10) {
            hub.input_message(&dev, [0xb0, 0x10, 80], time as f64, 0, 0);
            if time % 20 == 0 {
                hub.idle_jogs(&dev, now + Duration::from_millis(time - 500));
            }
        }
        assert_eq!(hub.decks[0].lock().bend, 0.0);
    }

    fn fader_controller(name: &str) -> Arc<Device> {
        let csv = format!(
            "@file,1,{name}\r\nChannelFader,ChannelFader,KnobSlider,,B010,B110,,,,,,,,Fast;Max=127;,\r\n"
        );
        let map = MidiMap::load(name, None, Some(csv.as_bytes())).unwrap();
        Device::mapped(name, name.into(), map, false, None)
    }

    #[test]
    fn input_worker_expires_a_due_jog_with_faders_already_queued() {
        let (hub, ui) = hub();
        hub.routing.lock().performance = true;
        let dev = fader_controller("wheel");
        let last_turn = Instant::now() - Duration::from_millis(100);
        let out = dev.jog.lock()[0].rotate(400, 123_000.0, false, last_turn);
        hub.routed_jog(&mut hub.routing.lock(), &dev, 0, out, false);
        assert_ne!(hub.decks[0].lock().bend, 0.0);

        let (tx, input) = crossbeam_channel::unbounded();
        for value in 0..128 {
            tx.send((123_100_000 + u64::from(value), [0xb0, 0x10, value], 0, 0)).unwrap();
        }
        // The final fader cannot finish until the observer unlocks deck 2. This
        // keeps the worker's disconnect cleanup from masking a missed jog deadline.
        let gate = hub.decks[1].lock();
        tx.send((123_101_000, [0xb1, 0x10, 64], 0, 0)).unwrap();
        drop(tx);
        let worker = {
            let hub = hub.clone();
            std::thread::spawn(move || hub.run_input(dev, input))
        };
        let first = ui.recv_timeout(Duration::from_secs(5));
        let bend_at_delivery = hub.decks[0].lock().bend;
        drop(gate);
        worker.join().unwrap();
        assert_eq!(bend_at_delivery, 0.0);

        let mut events = std::iter::once(first.expect("first fader event")).chain(ui.try_iter());
        for (deck, value) in (0..128).map(|v| (1, v)).chain([(2, 64)]) {
            assert!(matches!(events.next(), Some(UiEvent::Control {
                deck: d, tag, fast_done: true, val: Val::Abs { v, .. }, ..
            }) if d == deck && v == value && registry().get(tag).unwrap().act == Act::ChFader));
        }
        assert!(events.next().is_none());
    }

    #[test]
    fn acknowledging_faders_preserves_later_writes_from_both_ports() {
        let (hub, ui) = hub();
        hub.routing.lock().performance = true;
        std::thread::scope(|scope| {
            for (name, status) in [("left", 0xb0), ("right", 0xb1)] {
                let dev = fader_controller(name);
                let hub = &hub;
                scope.spawn(move || {
                    for value in 0..64 {
                        hub.input_message(&dev, [status, 0x10, value], f64::from(value), 0, 0);
                    }
                });
            }
        });
        let sent: Vec<_> = ui.try_iter().map(|event| {
            let UiEvent::Control { seq, deck, val, fast_done: true, .. } = event else {
                panic!("expected a fast fader");
            };
            (deck - 1, Param::ChFader, val.unit().unwrap(), seq)
        }).collect();
        assert_eq!(sent.len(), 128);
        assert!(sent.windows(2).all(|pair| pair[0].3 < pair[1].3));
        assert!(hub.overlay.lock().iter().eq(&sent));
        for (ack_index, remaining) in [(63, 64), (17, 64), (63, 64), (100, 101), (127, 128)] {
            hub.acknowledge_overlay(sent[ack_index].3);
            assert!(hub.overlay.lock().iter().eq(&sent[remaining..]));
        }
    }

    #[test]
    fn held_fx_mapping_survives_active_deck_changes_and_releases_on_disconnect() {
        let (hub, rx) = hub();
        hub.routing.lock().performance = true;
        let csv = b"@file,1,Helper\nFX1Assign.CH1,Channel,Button,9210,,,,,,,,,,,\n";
        let map = MidiMap::load("Helper", None, Some(csv)).unwrap();
        let dev = Device::mapped("Helper", "helper".into(), map, false, None);
        hub.input_message(&dev, [0x92, 0x10, 127], 1.0, 0, 0);
        assert!(matches!(rx.try_recv(), Ok(UiEvent::Control {deck:0, val:Val::On(true, _), tag, ..})
            if registry().get(tag).unwrap().act == Act::FxAssignHold(0, 0)));
        hub.snap.lock().active = 1;
        hub.release_device(&dev);
        assert!(matches!(rx.try_recv(), Ok(UiEvent::Control {deck:0, val:Val::On(false, _), ..})));
        assert!(hub.routing.lock().held.is_empty());
    }

    #[test]
    fn dual_search_releases_both_actual_decks_after_the_host_switch_changes() {
        let (hub, rx) = hub();
        hub.routing.lock().performance = true;
        hub.snap.lock().count = 4;
        let csv = b"@file,1,DDJ-RX\nSearchFwd,Forward,Button,9010,0,1,2,3,,,,,,Dual;,\n";
        let map = MidiMap::load("DDJ-RX", Some(csv), None).unwrap();
        let dev = Device::mapped("DDJ-RX", "dual".into(), map, true, None);
        hub.input_message(&dev, [0x96, 0x7c, 127], 1.0, 0, 0);
        hub.input_message(&dev, [0x90, 0x10, 127], 2.0, 0, 0);
        assert!(hub.routing.lock().search_held(0));
        assert!(hub.routing.lock().search_held(2));
        hub.input_message(&dev, [0x92, 0x3c, 127], 3.0, 0, 0);
        hub.input_message(&dev, [0x80, 0x10, 65], 4.0, 0, 0);
        assert!(hub.routing.lock().held.is_empty());
        let events: Vec<_> = rx.try_iter().map(|e| match e {
            UiEvent::Control { tag, deck, val, .. } => {
                assert_eq!(registry().get(tag).unwrap().act, Act::Search(1));
                (deck, val)
            }
            _ => panic!("expected a mapped search event"),
        }).collect();
        assert_eq!(events, [(1, Val::On(true, 127)), (3, Val::On(true, 127)),
                            (1, Val::On(false, 0)), (3, Val::On(false, 0))]);
    }

    #[test]
    fn scratch_carry_reschedules_again_in_the_controller_clock() {
        let mut p = Platter::new(2300.0);
        let now = Instant::now();
        let timestamp = 80_000.0;
        p.jog.touch(true, false);
        p.rotate(1, timestamp, true, now);
        let burst = now + Duration::from_millis(1);
        assert_eq!(p.rotate(25, timestamp + 1.0, true, burst), JogOut::Scratch(8.0));
        let step = p.deadline().unwrap();
        assert!(step < burst + Duration::from_millis(5));
        assert_eq!(p.idle(step), JogOut::Scratch(8.0));
        let end = p.deadline().unwrap();
        assert!(end > step && end < burst + Duration::from_millis(8));
        assert_eq!(p.idle(end), JogOut::Scratch(0.0));
        assert_eq!(p.deadline(), None);
    }
}
