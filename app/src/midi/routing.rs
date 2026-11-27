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
        self.last_rotation
            .map(|(_, at)| at + Duration::from_millis(61))
    }

    fn idle(&mut self, now: Instant) -> JogOut {
        let Some((timestamp, at)) = self.last_rotation else {
            return JogOut::Nothing;
        };
        let elapsed = now.saturating_duration_since(at);
        if elapsed <= Duration::from_millis(60) {
            return JogOut::Nothing;
        }
        self.last_rotation = None;
        self.jog.idle(timestamp + elapsed.as_secs_f64() * 1000.0)
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
            | Act::Shift
            | Act::JogTouch
            | Act::Stub(Stub::PadFx(..) | Stub::SlicerPad(..) | Stub::SlipReverse)
    )
}

struct Held {
    dev: Arc<Device>,
    event: Event,
}

#[derive(Default)]
pub(super) struct Routing {
    pub(super) performance: bool,
    held: Vec<Held>,
    jog_owner: [Option<Arc<Device>>; 4],
}

impl Routing {
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
    fn release_holds(&self, route: &mut Routing, device: Option<&Arc<Device>>) {
        let matches = |d: &Arc<Device>| device.is_none_or(|who| Arc::ptr_eq(who, d));
        while let Some(h) = route.held.iter().find(|h| matches(&h.dev)) {
            let dev = h.dev.clone();
            let mut release = Event {
                val: Val::On(false, 0),
                ..h.event
            };
            if route.hold(&dev, &mut release) {
                self.deliver(route, &dev, release);
            }
        }
        for (n, owner) in route.jog_owner.iter_mut().enumerate() {
            if owner.as_ref().is_some_and(&matches) {
                *owner = None;
                self.reset_jog(n);
            }
        }
        if let Some(dev) = device {
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
        if device.learning.swap(on, Relaxed) != on {
            device.input_epoch.fetch_add(1, Relaxed);
            if on {
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
}
