//! Device-free timing of the stereo audio callback, including deck rendering and the FX rack.
//! Run this ignored test on its own, with --nocapture and --test-threads=1. For a CPU
//! contention run, set RECORDBOX_FX_CPU_LOAD=1 and pin the process to one available CPU.
//! The other thread then competes with the callback at ordinary priority.

use super::*;
use crate::fx::{Kind, Rack, SlotParams, UnitParams, RACK};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const RATE: u32 = 48_000;
const WARM: usize = 64;
const SAMPLES: usize = 300;

#[test]
#[ignore = "paced hardware-dependent timing, run alone"]
fn performance_fx_callback_latency() {
    let stop = Arc::new(AtomicBool::new(false));
    let load = std::env::var_os("RECORDBOX_FX_CPU_LOAD").is_some();
    let worker = load.then(|| {
        let stop = stop.clone();
        std::thread::spawn(move || {
            // Repeat a small dense matrix/vector product. This provides CPU pressure
            // without requiring any model weights, GPU, or a particular STEMS provider.
            let matrix: Vec<f32> = (0..256 * 256).map(|i| (i % 97) as f32 / 97.0).collect();
            let vector = [1.0_f32 / 256.0; 256];
            while !stop.load(Ordering::Relaxed) {
                for row in matrix.chunks_exact(256) {
                    black_box(row.iter().zip(black_box(&vector)).map(|(a, b)| a * b).sum::<f32>());
                }
            }
        })
    });
    let prior = RACK.lock().take();
    for frames in [128, 512] {
        let dry = measure(None, frames, load);
        for effect in [Kind::Echo, Kind::ReverbUp] {
            assert_ne!(measure(Some(effect), frames, load), dry, "{effect:?} bypassed for all buffers");
        }
    }
    *RACK.lock() = prior;
    stop.store(true, Ordering::Relaxed);
    if let Some(worker) = worker {
        worker.join().unwrap();
    }
}

fn measure(effect: Option<Kind>, frames: usize, load: bool) -> u64 {
    let n = (WARM + SAMPLES + 2) * frames;
    let pcm = (0..n).flat_map(|i| {
        let v = 0.2 * (i as f32 * 330.0 * std::f32::consts::TAU / RATE as f32).sin();
        [v, v * 0.8]
    }).collect();
    let audio = Arc::new(Decoded { sr: RATE, samples: pcm, raw_frames: n, damaged: 0, truncated: false });
    let states: Vec<_> = (0..2).map(|deck_no| Arc::new(Mutex::new(PlayState {
        deck_no, audio: Some(audio.clone()), playing: true, volume: 1.0, gain: 1.0,
        smooth: true, ..Default::default()
    }))).collect();
    *RACK.lock() = effect.map(|kind| {
        let mut rack = Box::new(Rack::new());
        let mut unit = UnitParams::new();
        unit.assign[0] = true;
        unit.single = true;
        unit.one = SlotParams { on: true, level: 0.55, ..SlotParams::new(kind) };
        rack.enabled = true;
        rack.units[0].params = unit;
        rack.units[0].prepare(&unit.assign);
        rack.prepare_rev();
        rack
    });
    let mut out = vec![0.0; frames * 2];
    for _ in 0..WARM {
        render(&states, &mut out, 2, RATE);
    }
    let period = Duration::from_secs_f64(frames as f64 / f64::from(RATE));
    let start = Instant::now();
    let mut durations = Vec::with_capacity(SAMPLES);
    let mut missed = 0;
    let mut digest = 0xcbf29ce484222325_u64;
    for block in 0..SAMPLES {
        let due = start + period * block as u32;
        std::thread::sleep(due.saturating_duration_since(Instant::now()));
        let entered = Instant::now();
        render(black_box(&states), black_box(&mut out), 2, RATE);
        let done = Instant::now();
        durations.push(done - entered);
        missed += usize::from(done > due + period);
        // Outside the timed span. Repeated runs with matching digests also covered
        // identical audio despite different scheduler delays or provider load.
        for sample in &out {
            digest = (digest ^ u64::from(sample.to_bits())).wrapping_mul(0x100000001b3);
        }
    }
    assert!(states.iter().all(|s| s.lock().playing));
    assert!(out.iter().all(|v| v.is_finite()) && out.iter().any(|v| v.abs() > 1e-6));
    durations.sort_unstable();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    let over = durations.iter().filter(|d| **d > period).count();
    println!("FX callback {effect:?}, synthetic_cpu_load={load}, frames={frames}, blocks={SAMPLES}, budget_us={:.1}, p50_us={:.1}, p99_us={:.1}, max_us={:.1}, over_budget={over}, late_completions={missed}, audio_digest={digest:016x}",
        us(period), us(durations[(SAMPLES - 1) / 2]), us(durations[(99 * SAMPLES).div_ceil(100) - 1]), us(durations[SAMPLES - 1]));
    digest
}
