//! Performance controls shared by the factory CSV importer and MIDI Learn. Actions
//! use the same deck, pad and effect state as the on-screen controls.
use super::{Act, App, InClass, Stub, Tab, Val, T_BUTTON, T_KNOB7, T_PAD, T_ROTARY};
use crate::pads::PadMode;
use crate::sampler_ext::SmpAct;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Action {
    KeyPad(u8),
    KeyPage(i8),
    Censor(u8),
    CensorCall(bool),
    CensorBeat(bool),
    JumpBank(bool),
    Jump(i8),
    PadFxPage(u8),
    Bank(u8),
    CfxOn,
    CfxSub,
    FxAuto(bool),
    ChannelCurve(u8),
    MfxKnob(u8),
    MfxOn(u8),
    MfxAbort(u8),
    MfxAssign(u8, bool),
    MfxSelect(u8, i8),
    MfxVolume,
}

pub(super) fn register(add: &mut impl FnMut(Tab, &'static str, &str, &str, u8, bool, Act)) {
    use Act::Performance as P;
    use Action::*;
    const CENSOR: [&str; 8] = ["FX 1", "FX 2", "FX 3", "FX 4", "In", "Out", "Clear", "On/Off"];
    for (family, mode) in [("KeyShift", PadMode::KeyShift), ("ActiveCensor", PadMode::ActiveCensor)] {
        add(
            Tab::Pad,
            family,
            &format!("{family} Mode"),
            family,
            T_BUTTON,
            true,
            Act::PadMode(mode.code()),
        );
    }
    for slot in 0..16 {
        let n = slot + 1;
        add(
            Tab::Pad,
            "KeyShift",
            &format!("KeyShift Pad{n}"),
            &format!("PAD{n}_KeyShift"),
            T_PAD,
            true,
            P(KeyPad(slot)),
        );
        for (family, action) in [
            ("KeyShift", P(KeyPad(slot))),
            ("BeatJump", Act::BeatJumpPad(slot)),
            ("BeatLoop", Act::BeatLoopPad(slot)),
            ("PadFx1", Act::Stub(Stub::PadFx(1, slot))),
            ("PadFx2", Act::Stub(Stub::PadFx(2, slot))),
        ] {
            add(Tab::None, "", "", &format!("PAD{n}_{family}+Shift"), T_PAD, true, action);
        }
    }
    for (slot, label) in CENSOR.iter().enumerate() {
        let name = format!("PAD{}_ActiveCensor", slot + 1);
        let action = P(Censor(slot as u8));
        add(Tab::Pad, "ActiveCensor", &format!("ActiveCensor {label}"), &name, T_PAD, true, action);
        add(Tab::None, "", "", &format!("{name}+Shift"), T_PAD, true, action);
    }
    for (side, right, step) in [("Left", false, -1), ("Right", true, 1)] {
        for (family, parameter, display, action, kind) in [
            ("KeyShift", 1, "Page", KeyPage(step), T_PAD),
            ("ActiveCensor", 1, "Call", CensorCall(right), T_BUTTON),
            ("ActiveCensor", 2, "Beat", CensorBeat(right), T_BUTTON),
            ("BeatJump", 2, "Bank", JumpBank(right), T_BUTTON),
        ] {
            add(
                Tab::Pad,
                family,
                &format!("{family} {display} {side}"),
                &format!("Parameter{parameter}{side}_{family}"),
                kind,
                true,
                P(action),
            );
        }
        add(Tab::None, "", "", &format!("Page{side}_BeatJump"), T_BUTTON, true, P(JumpBank(right)));
        for bank in 1..=2 {
            add(
                Tab::None,
                "",
                "",
                &format!("Page{side}_PadFx{bank}"),
                T_BUTTON,
                true,
                P(PadFxPage(u8::from(right))),
            );
        }
        add(
            Tab::None,
            "",
            "",
            &format!("Parameter1{side}_VelocitySampler"),
            T_BUTTON,
            true,
            Act::SamplerBank(i32::from(step)),
        );
    }
    for beats in [4, 8, 16] {
        for (suffix, amount) in [("Fwd", beats), ("Prev", -beats)] {
            add(Tab::None, "", "", &format!("Jump{beats}{suffix}"), T_BUTTON, true, P(Jump(amount)));
        }
    }
    for slot in 0..4 {
        add(Tab::None, "", "", &format!("PAD{}_Bank", slot + 1), T_BUTTON, true, P(Bank(slot)));
    }
    for (part, source, fx) in [("Vocal", 3, 0), ("Inst", 2, 1), ("Drums", 0, 3)] {
        add(
            Tab::None,
            "",
            "",
            &format!("ActivePartButton{part}"),
            T_BUTTON,
            true,
            Act::StemPart(source),
        );
        add(Tab::None, "", "", &format!("FXPartSelect{part}"), T_BUTTON, false, Act::StemFx(fx));
    }
    for (name, action) in [
        ("SamplerBpmUp", Act::Smp(SmpAct::Bpm(0.01))),
        ("SamplerBpmDown", Act::Smp(SmpAct::Bpm(-0.01))),
        ("TempoCenter", Act::TempoReset),
        ("PadFx", Act::Stub(Stub::PadFxMode(1))),
    ] {
        add(
            Tab::None,
            "",
            "",
            name,
            T_BUTTON,
            matches!(action, Act::TempoReset | Act::Stub(_)),
            action,
        );
    }
    for (i, name) in ["STRAIGHT", "MID", "SHARP"].iter().enumerate() {
        add(
            Tab::None,
            "",
            "",
            &format!("ChFaderCurve.{name}"),
            T_BUTTON,
            false,
            P(ChannelCurve(i as u8)),
        );
    }
    add(Tab::None, "", "", "FxBpmAutoMode", T_BUTTON, false, P(FxAuto(true)));
    add(Tab::None, "", "", "FxBpmTapMode", T_BUTTON, false, P(FxAuto(false)));
    add(Tab::Fx, "CFX", "CFX On", "CFXOn", T_BUTTON, false, P(CfxOn));
    add(Tab::None, "", "", "CFXSubParameter", T_KNOB7, false, P(CfxSub));
    for (u, family) in ["MFX1", "MFX2"].into_iter().enumerate() {
        let unit = u as u8;
        add(Tab::Fx, family, &format!("{family} Parameter"), family, T_ROTARY, false, P(MfxKnob(unit)));
        for (suffix, label, action) in [
            ("On".into(), "On", MfxOn(unit)),
            ("Abort".into(), "Cancel", MfxAbort(unit)),
            ("SelectUp".into(), "Select Next", MfxSelect(unit, 1)),
            ("SelectDown".into(), "Select Back", MfxSelect(unit, -1)),
            (format!("Assign{}", u + 1), "Assign Lower Deck", MfxAssign(unit, false)),
            (format!("Assign{}", u + 3), "Assign Upper Deck", MfxAssign(unit, true)),
        ] {
            add(
                Tab::Fx,
                family,
                &format!("{family} {label}"),
                &format!("{family}{suffix}"),
                T_BUTTON,
                false,
                P(action),
            );
        }
    }
    add(Tab::Fx, "", "MFX Sample Volume", "MFXSampleVolume", T_ROTARY, false, P(MfxVolume));
}

fn selected_cfx(app: &App) -> usize {
    let kind = if app.px.cfx_user { app.px.cfx_kind } else { crate::cfx::DEFAULT_LIST[0] };
    usize::from(kind) % app.px.fxx.cfx_params.len()
}

impl Action {
    pub(super) fn accepts(self, class: InClass) -> bool {
        match self {
            Self::MfxKnob(_) | Self::MfxVolume => matches!(class, InClass::Abs | InClass::Rel),
            Self::CfxSub => class == InClass::Abs,
            _ => class == InClass::Press,
        }
    }

    pub(super) fn has_led(self) -> bool {
        matches!(
            self,
            Self::Censor(_) | Self::KeyPad(_) | Self::Bank(_) | Self::CfxOn | Self::MfxOn(_) | Self::MfxAssign(..)
        )
    }

    pub(super) fn led(self, app: &App, loaded: bool) -> Option<bool> {
        Some(match self {
            Self::Censor(_) | Self::KeyPad(_) => loaded,
            Self::Bank(bank) => app.prefs.sampler.bank == bank,
            Self::CfxOn => app.px.cfx_user,
            Self::MfxOn(u) => app.performance() && app.prefs.fx.mfx.enabled && !app.deck_options.disable_play && app.px.fxx.mfx.units[u as usize].on,
            Self::MfxAssign(u, upper) => app.prefs.fx.mfx.upper[u as usize] == upper,
            _ => return None,
        })
    }

    pub(super) fn apply(self, app: &mut App, n: u8, val: Val) {
        use Action::*;
        let ni = usize::from(n);
        match self {
            MfxKnob(u) => {
                let knob = &mut app.px.fxx.mfx.units[u as usize].knob;
                if let Some(value) = val.unit() {
                    *knob = value * 2.0 - 1.0;
                } else if let Val::Rel(steps) = val {
                    *knob = (*knob + steps as f32 / 64.0).clamp(-1.0, 1.0);
                }
            }
            MfxVolume => {
                let volume = &mut app.prefs.fx.mfx.volume;
                let previous = *volume;
                if let Some(value) = val.unit() {
                    *volume = value * 24.0 - 12.0;
                } else if let Val::Rel(steps) = val {
                    *volume = (*volume + steps as f32 * 0.5).clamp(-12.0, 12.0);
                }
                if *volume != previous {
                    app.px.fxx.pending.get_or_insert_with(std::time::Instant::now);
                }
            }
            CfxSub => {
                if let Some(value) = val.unit() {
                    app.px.cfx_param = value;
                    let kind = selected_cfx(app);
                    app.px.fxx.cfx_params[kind] = value;
                }
            }
            _ if !matches!(val, Val::On(true, _)) => {}
            KeyPad(slot) => {
                let page = app.prefs.pads.key_shift_page[ni];
                app.with_deck(n, |a| a.pad_key(crate::pads::key_pad_pos(page, slot as usize)));
            }
            KeyPage(step) => {
                let page = &mut app.prefs.pads.key_shift_page[ni];
                *page = page.saturating_add_signed(step).clamp(1, 5);
                app.save_prefs();
            }
            Censor(slot) => app.with_deck(n, |a| match slot {
                0..=3 => a.censor_set_kind(slot),
                4 => a.censor_in(),
                5 => {
                    a.censor_out();
                }
                6 => {
                    a.censor_clear();
                }
                _ => a.censor_onoff(),
            }),
            CensorCall(next) => app.with_deck(n, |a| a.censor_call(next)),
            CensorBeat(right) => app.with_deck(n, |a| a.censor_beat(right)),
            JumpBank(right) => {
                let page = &mut app.prefs.pads.beat_jump_page[ni];
                *page = crate::pads::jump_page_step(*page, right, true);
                app.save_prefs();
            }
            Jump(amount) => app.with_deck(n, |a| a.beat_jump(f64::from(amount))),
            PadFxPage(page) => app.with_deck(n, |a| a.deck.x.pad_page = page),
            Bank(bank) => app.sampler_action(SmpAct::Bank(bank)),
            CfxOn => {
                app.px.cfx_user = !app.px.cfx_user;
                app.px.cfx_param = app.px.fxx.cfx_params[selected_cfx(app)];
            }
            FxAuto(auto) => {
                for unit in &mut app.px.fx {
                    unit.auto = auto;
                }
            }
            ChannelCurve(p) => {
                app.prefs.ch_fader_curve = p;
                app.save_prefs();
            }
            MfxOn(u) => {
                if app.prefs.fx.mfx.enabled && !app.deck_options.disable_play {
                    crate::mfx::toggle(app, u as usize, false);
                }
            }
            MfxAbort(u) => crate::mfx::toggle(app, u as usize, true),
            MfxAssign(u, upper) => {
                app.prefs.fx.mfx.upper[u as usize] = upper;
                app.save_prefs();
            }
            MfxSelect(u, dir) => {
                let preset = &mut app.prefs.fx.mfx.select[u as usize];
                *preset = (i16::from(*preset) + i16::from(dir)).rem_euclid(4) as u8;
                app.save_prefs();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::{act_accepts, act_has_led, apply, apply_pending, input_acts, leds, registry, Device, Hub};
    use crate::midi_csv::Kind;
    use std::sync::Arc;

    #[test]
    fn factory_shift_layers_share_the_original_pad_action_and_feedback() {
        let registry = registry();
        for (family, count) in [
            ("BeatJump", 16),
            ("BeatLoop", 16),
            ("PadFx1", 16),
            ("PadFx2", 16),
            ("KeyShift", 16),
            ("ActiveCensor", 8),
        ] {
            for n in 1..=count {
                let name = format!("PAD{n}_{family}");
                let regular = registry.by_name(&name).unwrap().act;
                let shifted = registry.by_name(&format!("{name}+Shift")).unwrap().act;
                assert_eq!(shifted, regular, "{name}");
                assert!(act_accepts(shifted, InClass::Press), "{name}");
                assert!(!act_accepts(shifted, InClass::Abs), "{name}");
                assert_eq!(act_has_led(regular), act_has_led(shifted));
            }
        }
        assert!(input_acts("MFX1", Kind::Rotary) && input_acts("MFX2", Kind::KnobSlider));
        assert!(!input_acts("MFX1", Kind::Button) && !input_acts("CFXSubParameter", Kind::Rotary));
        assert_eq!(registry.by_name("ActivePartButtonInst").unwrap().act, Act::StemPart(2));
        assert_eq!(registry.by_name("FXPartSelectDrums").unwrap().act, Act::StemFx(3));
    }

    #[test]
    fn mappings_update_pad_pages_and_merge_fx_with_feedback_and_transport_guards() {
        const TEST: &str = "midi::performance::tests::mappings_update_pad_pages_and_merge_fx_with_feedback_and_transport_guards";
        const CHILD: &str = "RECORDBOX_MIDI_PERFORMANCE_TEST_PROFILE";
        let Some(profile) = std::env::var_os(CHILD) else {
            let tmp = tempfile::tempdir().unwrap();
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, tmp.path())
                .env("PREPDECK_DATA", tmp.path())
                .output()
                .unwrap();
            let out = String::from_utf8_lossy(&child.stdout);
            assert!(
                child.status.success() && out.contains("1 passed"),
                "{out}\n{}",
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
                disable_load: true,
            },
        );
        app.prefs.app_mode = 1;
        app.prefs.perf_four = true;
        // Addresses are invented for this custom test map, not a physical controller profile.
        const DEVICE: &str = "Performance test";
        const MAP: &[u8] = b"@file,1,Performance test\nMFX2On,Switch,Button,9018,0,,,,9018,0,,,,,\n";
        let (mut hub, rx) = Hub::new(app.player.decks.clone(), profile.into(), true);
        let setup = Arc::get_mut(&mut hub).unwrap();
        setup.factory_dirs.clear();
        setup.native_user_dirs.clear();
        hub.write_override(DEVICE, MAP).unwrap();
        let (map, factory) = hub.load_map(DEVICE);
        assert_eq!(map.rejected, 0);
        let mut device = Device::mapped(DEVICE, DEVICE.into(), map, factory, None);
        Arc::get_mut(&mut device).unwrap().has_output = true;
        hub.devices.lock().push(device.clone());
        hub.routing.lock().performance = true;
        hub.snap.lock().count = 4;
        app.midi.rx = Some(rx);
        let click = |app: &mut App| {
            for value in [127, 0] {
                hub.input_message(&device, [0x90, 0x18, value], 0.0, 0, 0);
            }
            apply_pending(app, &hub);
            leds(app, &hub);
            let mut output = device.out.lock();
            std::iter::from_fn(|| output.queue.pop()).collect::<Vec<_>>()
        };
        assert!(click(&mut app).contains(&[0x90, 0x18, 0]));
        assert!(!app.px.fxx.mfx.units[1].on);
        app.deck_options.disable_play = false;
        assert!(!app.prefs.fx.mfx.enabled);
        click(&mut app);
        assert!(!app.px.fxx.mfx.units[1].on);
        app.prefs.fx.mfx.enabled = true;
        app.deck_options.disable_play = true;
        click(&mut app);
        assert!(!app.px.fxx.mfx.units[1].on);
        app.deck_options.disable_play = false;
        assert!(click(&mut app).contains(&[0x90, 0x18, 127]));
        assert!(app.px.fxx.mfx.units[1].on && !app.px.fxx.mfx.units[0].on);
        assert!(click(&mut app).contains(&[0x90, 0x18, 0]));
        let fire = |app: &mut App, name: &str, deck, value| apply(app, &hub, DEVICE, registry().by_name(name).unwrap(), deck, value, false, 0);
        let press = Val::On(true, 127);
        fire(&mut app, "Parameter1Right_BeatJump", 2, press);
        assert_eq!(app.prefs.pads.beat_jump_page, [5, 6, 5, 5]);
        fire(&mut app, "PageLeft_BeatJump", 2, press);
        assert_eq!(app.prefs.pads.beat_jump_page, [5; 4]);
        fire(&mut app, "Parameter2Left_BeatJump", 2, press);
        assert_eq!(app.prefs.pads.beat_jump_page, [5, 1, 5, 5]);
        fire(&mut app, "KeyShift", 2, press);
        assert_eq!(app.px.pad_mode[1], PadMode::KeyShift.code());
        fire(&mut app, "Parameter1Right_KeyShift", 2, press);
        assert_eq!(app.prefs.pads.key_shift_page, [2, 3, 2, 2]);
        fire(&mut app, "PAD4_Bank", 1, press);
        assert_eq!(app.prefs.sampler.bank, 3);
        fire(&mut app, "MFX2", 0, Val::Rel(-8));
        assert_eq!(app.px.fxx.mfx.units[1].knob, -0.125);
        fire(
            &mut app,
            "MFX2",
            0,
            Val::Abs {
                v: 127,
                lo: 0,
                mid: 64,
                hi: 127,
            },
        );
        assert_eq!(app.px.fxx.mfx.units[1].knob, 1.0);
        fire(&mut app, "MFX2Abort", 0, press);
        assert_eq!(app.px.fxx.mfx.units[1].knob, 0.0);
        fire(&mut app, "MFX2Assign4", 0, press);
        assert_eq!(Action::MfxAssign(1, true).led(&app, false), Some(true));
        assert_eq!(Action::MfxAssign(1, false).led(&app, false), Some(false));
        app.px.cfx_kind = 1;
        app.px.cfx_user = false;
        app.px.fxx.cfx_params[1] = 0.75;
        fire(
            &mut app,
            "CFXSubParameter",
            0,
            Val::Abs {
                v: 0,
                lo: 0,
                mid: 64,
                hi: 127,
            },
        );
        assert_eq!(app.px.fxx.cfx_params[0], 0.0);
        fire(&mut app, "CFXOn", 0, press);
        assert_eq!(app.px.cfx_param, 0.75);
        let cfx = app.px.cfx_user;
        app.prefs.app_mode = 0;
        fire(&mut app, "CFXOn", 0, press);
        assert_eq!(app.px.cfx_user, cfx);
    }
}
