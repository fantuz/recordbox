//! fx7b / fx8b track tests that run whole slots (kept out of `fx.rs`'s test module so the
//! parallel tracks' edits merge).

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

/// VINYL BRAKE's BPF (`BeatFxVinylBrake::updateStateAndParameter`): while the brake runs, the
/// highs (band 0, over 4.6 kHz) pass live beside it; without BPF a stopped brake is silent.
#[test]
fn vinyl_brake_bpf_passes_the_outer_bands_live() {
    let tone = |n: usize| -> Vec<f32> { (0..n).flat_map(|i| { let v = 0.5 * (i as f32 * std::f32::consts::TAU * 10000.0 / 48000.0).sin(); [v, v] }).collect() };
    let n = 48000 * 6;
    let (mut a, mut b) = (tone(n), tone(n));
    run(Kind::VinylBrake, 1.0, [0.5, 0.5], [false, true], &mut a);
    run(Kind::VinylBrake, 1.0, [0.5, 0.5], [false, false], &mut b);
    let tail = 48000 * 5 * 2..n * 2;
    let dry = rms(&tone(n)[tail.clone()]);
    let (ra, rb) = (rms(&a[tail.clone()]), rms(&b[tail]));
    assert!((ra / dry - 1.0).abs() < 0.05, "BPF: the highs pass live ({ra} vs {dry})");
    assert!(rb < 0.01 * dry, "no BPF: the stopped brake is silent ({rb})");
}

/// STRETCH's reads (`RingBuffer::readByLagrangeInterpolation` 0x102989008): a cubic through
/// four frames is read back at any place between the middle two.
#[test]
fn stretch_lagrange_read_is_rekordbox_s() {
    let f = |x: f32| x * x * x - 2.0 * x * x + 3.0;
    for t in [0.0f32, 0.25, 0.5, 0.9] {
        let v = crate::fx::lagrange4(f(-1.0), f(0.0), f(1.0), f(2.0), t);
        assert!((v - f(t)).abs() < 2e-4, "{t}: {v} vs {}", f(t));
    }
    assert_eq!(crate::fx::lagrange4(0.0, 1.0, 0.0, 0.0, 0.0), 1.0);
}
