//! fx9 track tests that run whole slots (kept out of `fx.rs`'s test module so the parallel
//! tracks' edits merge).

use crate::fx::{Kind, Rack, SlotParams, UnitParams};

/// Single-mode `kind` on deck 1 at 120 BPM, beat value 1, over `x` in 512-frame buffers.
fn run(kind: Kind, level: f32, knob: [f32; 2], btn: [bool; 2], x: &mut [f32]) {
    let mut rack = Rack::new();
    rack.enabled = true;
    let mut p = UnitParams::new();
    p.assign = [true, false, false, false, false, false];
    p.bpm = [120.0; 6];
    p.single = true;
    p.one = SlotParams { on: true, level, value: 1.0, knob, btn, ..SlotParams::new(kind) };
    rack.units[0].params = p;
    rack.units[0].prepare(&p.assign);
    rack.prepare_rev();
    for c in x.chunks_mut(512 * 2) {
        rack.process(0, c, 48000);
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// A stereo tone at `hz` for `secs` of `n` frames (silence after).
fn tone(hz: f32, secs: f32, n: usize) -> Vec<f32> {
    (0..n).flat_map(|i| { let v = if (i as f32) < secs * 48000.0 { 0.5 * (i as f32 * std::f32::consts::TAU * hz / 48000.0).sin() } else { 0.0 }; [v, v] }).collect()
}

/// The largest left / right difference over `x`'s frames from `from`.
fn side_diff(x: &[f32], from: usize) -> f32 {
    x.chunks(2).skip(from).map(|f| (f[0] - f[1]).abs()).fold(0.0, f32::max)
}

/// PAN's buttons (`SingleBeatFxPan::updateParameter` 0x102a4f0a8): button 1 takes the middle
/// band (284 Hz .. 4.6 kHz) out of the pan, button 2 the outer two. A 1 kHz tone is panned
/// unless button 1 is on; then it passes live, the same on both sides.
#[test]
fn pan_buttons_take_the_bands_rekordbox_takes() {
    let n = 48000 * 3;
    let (mut a, mut b, mut c) = (tone(1000.0, 3.0, n), tone(1000.0, 3.0, n), tone(1000.0, 3.0, n));
    run(Kind::Pan, 1.0, [0.5, 0.5], [true, false], &mut a);
    run(Kind::Pan, 1.0, [0.5, 0.5], [false, true], &mut b);
    run(Kind::Pan, 1.0, [0.5, 0.5], [false, false], &mut c);
    let (da, db, dc) = (side_diff(&a, 48000), side_diff(&b, 48000), side_diff(&c, 48000));
    assert!(da < 0.01, "button 1: the middle band is not panned ({da})");
    assert!(db > 0.2 && dc > 0.2, "button 2 / none: the middle band is panned ({db}, {dc})");
    // (A 60 Hz tone is the other way round.)
    let (mut a, mut b) = (tone(60.0, 3.0, n), tone(60.0, 3.0, n));
    run(Kind::Pan, 1.0, [0.5, 0.5], [true, false], &mut a);
    run(Kind::Pan, 1.0, [0.5, 0.5], [false, true], &mut b);
    let (da, db) = (side_diff(&a, 48000), side_diff(&b, 48000));
    assert!(da > 0.2 && db < 0.01, "60 Hz: panned with button 1 ({da}), live with button 2 ({db})");
}

/// TRANS's LOW CUT (`SingleBeatFxTrans::updateParameter` 0x102a51ac0, band 2 out of the gate,
/// added back after it): a 60 Hz tone is not gated.
#[test]
fn trans_low_cut_passes_the_lows_past_the_gate() {
    let n = 48000 * 3;
    let (mut a, mut b) = (tone(60.0, 3.0, n), tone(60.0, 3.0, n));
    run(Kind::Trans, 1.0, [0.5, 0.5], [true, false], &mut a);
    run(Kind::Trans, 1.0, [0.5, 0.5], [false, false], &mut b);
    let win = |x: &[f32]| -> f32 { x[48000 * 2..].chunks(2 * 480).map(rms).fold(f32::MAX, f32::min) };
    let (wa, wb) = (win(&a), win(&b));
    assert!(wa > 0.3, "LOW CUT: the lows are never shut ({wa})");
    assert!(wb < 0.05, "no cut: the gate shuts the lows ({wb})");
}

/// ROLL's LOW CUT (`SingleBeatFxRollBase::processEffectConcrete` 0x102a2ffac): the lows come
/// from the live sound, the roll keeps only its middle and high bands. A 60 Hz tone that stops
/// after 0.25 s keeps rolling without LOW CUT and is gone with it.
#[test]
fn roll_low_cut_takes_the_lows_live() {
    for kind in [Kind::Roll, Kind::SlipRoll] {
        let n = 48000 * 3;
        let (mut a, mut b) = (tone(60.0, 0.25, n), tone(60.0, 0.25, n));
        run(kind, 1.0, [0.5, 0.5], [true, false], &mut a);
        run(kind, 1.0, [0.5, 0.5], [false, false], &mut b);
        let tail = 48000 * 2 * 2..n * 2;
        let (ra, rb) = (rms(&a[tail.clone()]), rms(&b[tail]));
        assert!(rb > 0.05, "{kind:?} no cut: the lows roll ({rb})");
        assert!(ra < 0.01 * rb.max(1e-6) + 1e-3, "{kind:?} LOW CUT: the lows are live, so silent ({ra})");
    }
}

/// HELIX's HI CUT (`SingleBeatFxHelix::postProcessEffectHook` 0x1029f62d8): the highs come
/// from the live sound, the helix keeps its lower bands. A 14 kHz tone that stops after 0.25 s
/// is held by the helix without HI CUT and gone with it.
#[test]
fn helix_hi_cut_takes_the_highs_live() {
    let n = 48000 * 3;
    let (mut a, mut b) = (tone(14000.0, 0.25, n), tone(14000.0, 0.25, n));
    run(Kind::Helix, 1.0, [0.5, 0.5], [false, true], &mut a);
    run(Kind::Helix, 1.0, [0.5, 0.5], [false, false], &mut b);
    let tail = 48000 * 2 * 2..n * 2;
    let (ra, rb) = (rms(&a[tail.clone()]), rms(&b[tail]));
    assert!(rb > 0.05, "no cut: the helix holds the highs ({rb})");
    assert!(ra < 0.02 * rb, "HI CUT: the highs are live, so silent ({ra} vs {rb})");
}

/// VINYL BRAKE's BPF (`BeatFxVinylBrake::processEffectConcrete` 0x102a55028): the core is given
/// the module's fx bands and keeps its own history of them (`VinylBrakeCore::processBlock`
/// 0x102a584f4, `RingBufferForbiddenOverwriting::writeOnce`), so with BPF it brakes the middle
/// band only. A 10 kHz tone that stops after 1 s: once it has stopped, the brake (2 s at
/// LEVEL 0.25, still reading the tone's part of its history) is near silent with BPF.
#[test]
fn vinyl_brake_bpf_brakes_the_middle_band_s_history() {
    let n = 48000 * 2;
    let (mut a, mut b) = (tone(10000.0, 1.0, n), tone(10000.0, 1.0, n));
    run(Kind::VinylBrake, 0.25, [0.5, 0.5], [false, true], &mut a);
    run(Kind::VinylBrake, 0.25, [0.5, 0.5], [false, false], &mut b);
    let w = 48000 * 2 * 6 / 5..48000 * 2 * 9 / 5;
    let (ra, rb) = (rms(&a[w.clone()]), rms(&b[w]));
    assert!(rb > 0.05, "no BPF: the brake plays the tone back ({rb})");
    assert!(ra < 0.05 * rb, "BPF: the brake's history is the middle band ({ra} vs {rb})");
}

/// STRETCH's shift (`StretchCore::updatePitchSpeed` 0x102a4780c): REPITCH 0, else 1 / speed - 1
/// (speed 1 - L below half, 0.75 - L / 2 above); and the sound: with REPITCH a 1 kHz tone at
/// LEVEL/DEPTH 1 (a quarter speed) comes out near 250 Hz, without it near 1 kHz.
#[test]
fn stretch_repitch_is_rekordbox_s() {
    use crate::fx::{stretch_shift, stretch_speed};
    assert_eq!(stretch_shift(stretch_speed(1.0), true), 0.0);
    assert_eq!(stretch_shift(stretch_speed(1.0), false), 3.0);
    assert!((stretch_shift(stretch_speed(0.75), false) - (1.0 / 0.375 - 1.0)).abs() < 1e-6);
    assert!((stretch_shift(stretch_speed(0.25), false) - (1.0 / 0.75 - 1.0)).abs() < 1e-6);
    let n = 48000 * 2;
    let crossings = |x: &[f32]| x.chunks(2).skip(48000).collect::<Vec<_>>().windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count() as f32 / ((n - 48000) as f32 / 48000.0);
    let (mut a, mut b) = (tone(1000.0, 2.0, n), tone(1000.0, 2.0, n));
    // (Beat value 1 at 120 BPM x LEVEL 1: a turn of 0.5 s.)
    run(Kind::Stretch, 1.0, [0.5, 0.5], [true, false], &mut a);
    run(Kind::Stretch, 1.0, [0.5, 0.5], [false, false], &mut b);
    let (fa, fb) = (crossings(&a), crossings(&b));
    assert!((fa - 250.0).abs() < 40.0, "REPITCH: about 250 Hz ({fa})");
    assert!((fb - 1000.0).abs() < 80.0, "no REPITCH: about 1 kHz ({fb})");
}

/// The master's order (`FxUnitProcessor::processBlock` 0x102aa1388 runs its slots in turn;
/// `DjSystem::setMasterFx` slots: Beat FX 2, CFX 4): an ECHO on the master and the master
/// CFX CRUSH (not linear, so the order shows) give what the ECHO alone gives, crushed after.
#[test]
fn master_beat_fx_runs_before_the_master_cfx() {
    let n = 48000;
    let x: Vec<f32> = (0..n).flat_map(|i| { let v = if i % 12000 < 200 { 0.6 * (i as f32 * 0.05).sin() } else { 0.0 }; [v, v] }).collect();
    let rack_run = |cfx: bool, x: &mut [f32]| {
        let mut rack = Rack::new();
        rack.enabled = true;
        let mut p = UnitParams::new();
        p.assign = [false, false, false, false, true, false];
        p.bpm = [120.0; 6];
        p.single = true;
        p.one = SlotParams { on: true, level: 0.8, value: 0.5, knob: [0.5, 0.5], btn: [false, false], ..SlotParams::new(Kind::Echo) };
        rack.units[0].params = p;
        rack.units[0].prepare(&p.assign);
        rack.prepare_rev();
        if cfx {
            rack.master_cfx_set = (2, 0.9, 0.5);
        }
        for c in x.chunks_mut(512 * 2) {
            rack.process(crate::fx::MASTER, c, 48000);
        }
    };
    let mut both = x.clone();
    rack_run(true, &mut both);
    let mut want = x.clone();
    rack_run(false, &mut want);
    let mut cfx = crate::cfx::Cfx::new();
    for c in want.chunks_mut(512 * 2) {
        cfx.process(2, 0.9, 0.5, c, 48000);
    }
    let diff = both.iter().zip(&want).fold(0.0f32, |a, (p, q)| a.max((p - q).abs()));
    assert!(rms(&want) > 0.01 && diff < 1e-6, "ECHO then CRUSH ({diff})");
    // (The other order would sound otherwise: CRUSH, then the ECHO of it.)
    let mut other = x.clone();
    let mut cfx = crate::cfx::Cfx::new();
    for c in other.chunks_mut(512 * 2) {
        cfx.process(2, 0.9, 0.5, c, 48000);
    }
    rack_run(false, &mut other);
    let d2 = both.iter().zip(&other).fold(0.0f32, |a, (p, q)| a.max((p - q).abs()));
    assert!(d2 > 1e-3, "the order shows ({d2})");
}
