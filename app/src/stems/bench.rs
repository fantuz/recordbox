//! Speed and sameness checks for stem separation on real tracks. All are ignored tests, run
//! by hand (`cargo test --release <name> -- --ignored --nocapture`); none writes to the stems
//! cache or the user's data.
//!
//! Environment (paths stay out of the code; the test tracks are the user's own files):
//! - `PREPDECK_STEM_TEST_AUDIO`: the whole-track test file.
//! - `PREPDECK_STEM_EXCERPTS`: `path@start_seconds` entries separated by `|`; 45 s of each.
//! - `PREPDECK_STEM_REF`: folder for the reference outputs (raw little-endian f32, exactly
//!   what the separation returns).
//! - `PREPDECK_STEM_REF_MAKE=1`: write the reference instead of checking against it.
//! - `PREPDECK_STEM_CASES`: only run cases whose name contains one of these `,`-separated
//!   words.

use super::*;
use std::time::Instant;

const EXCERPT_SECS: usize = 45;

/// Decoded test audio: (name, interleaved stereo, rate).
fn inputs() -> Vec<(String, Vec<f32>, u32)> {
    let mut out = vec![];
    if let Ok(p) = std::env::var("PREPDECK_STEM_TEST_AUDIO") {
        if !p.is_empty() {
            let d = crate::audio::decode_file(&p).expect("decoding the whole-track test file");
            out.push(("whole".to_string(), d.samples, d.sr));
        }
    }
    if let Ok(list) = std::env::var("PREPDECK_STEM_EXCERPTS") {
        for (i, e) in list.split('|').filter(|e| !e.is_empty()).enumerate() {
            let (path, start) = e.rsplit_once('@').expect("excerpt as path@seconds");
            let d = crate::audio::decode_file(path).expect("decoding an excerpt");
            let start: usize = start.trim().parse::<usize>().unwrap() * d.sr as usize;
            let len = EXCERPT_SECS * d.sr as usize;
            let a = (2 * start).min(d.samples.len());
            let b = (2 * (start + len)).min(d.samples.len());
            out.push((format!("ex{}", i + 1), d.samples[a..b].to_vec(), d.sr));
        }
    }
    out
}

fn wanted(case: &str) -> bool {
    match std::env::var("PREPDECK_STEM_CASES") {
        Ok(f) if !f.is_empty() => f.split(',').any(|w| case.contains(w.trim())),
        _ => true,
    }
}

/// Signal-to-error ratio of `new` against `reference`, in dB (infinite when identical).
pub(super) fn snr_db(reference: &[f32], new: &[f32]) -> f64 {
    let (mut s, mut e) = (0f64, 0f64);
    for (a, b) in reference.iter().zip(new) {
        s += (*a as f64).powi(2);
        e += ((*a - *b) as f64).powi(2);
    }
    if e == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (s / e).log10()
    }
}

fn write_f32(path: &Path, v: &[f32]) {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

fn read_f32(path: &Path) -> Vec<f32> {
    std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display())).chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// One way the app runs a model, and what it returns.
enum Run {
    /// `separate` with this model: all four stems, stereo.
    Full(&'static str),
    /// `separate_one(source)`: one source, mono (the fine-tuned specialist when downloaded).
    One(usize),
    /// HTDemucs (single model) asked for one source at the analysis overlap, as
    /// `separate_one` runs it when the fine-tuned set is not downloaded.
    HtOne(usize),
}

fn run(r: &Run, samples: &[f32], sr: u32) -> (Vec<f32>, usize) {
    match r {
        Run::Full(id) => (separate(spec(id).unwrap(), samples, sr, &mut |_| true).unwrap().data, 4),
        Run::One(s) => (separate_one(*s, samples, sr, &mut |_| true).unwrap(), 1),
        Run::HtOne(s) => {
            let (planar, len) = planar_44k(samples, sr);
            let g = graph(CATALOG[1].files[0], SeparationDevice::Selected).unwrap();
            let y = htdemucs::separate(&g, &device(), &planar, len, &htdemucs::Options { overlap: ANALYSIS_OVERLAP, source: Some(*s) }, &mut |_, _| true).unwrap();
            htdemucs::release_scratch();
            (y[..len].iter().zip(&y[len..]).map(|(a, b)| 0.5 * (a + b)).collect(), 1)
        }
    }
}

/// Every stem of every case against the reference made with the code as it was before the
/// 2026-11-27 speed work. A change passes when every stem agrees to at least 100 dB.
#[test]
#[ignore]
fn stems_match_reference() {
    let Ok(dir) = std::env::var("PREPDECK_STEM_REF") else {
        println!("set PREPDECK_STEM_REF");
        return;
    };
    let dir = PathBuf::from(dir);
    let make = std::env::var("PREPDECK_STEM_REF_MAKE").is_ok_and(|v| v == "1");
    std::fs::create_dir_all(&dir).unwrap();
    let ins = inputs();
    let runs: Vec<(String, Run)> = vec![
        ("htdemucs".into(), Run::Full("htdemucs")),
        ("htdemucs_ft".into(), Run::Full("htdemucs_ft")),
        ("vocal".into(), Run::One(3)),
        ("drums".into(), Run::One(0)),
        ("htvocal".into(), Run::HtOne(3)),
    ];
    let mut worst = f64::INFINITY;
    let mut stems = 0;
    for (rname, r) in &runs {
        for (iname, samples, sr) in &ins {
            let case = format!("{rname}-{iname}");
            // The drums and single-model vocal paths are the same code as `vocal`; the
            // excerpts are enough for them.
            if !wanted(&case) || (matches!(r, Run::One(0) | Run::HtOne(_)) && iname == "whole") {
                continue;
            }
            let t = Instant::now();
            let (y, n_src) = run(r, samples, *sr);
            let secs = t.elapsed().as_secs_f64();
            let path = dir.join(format!("{case}.f32"));
            if make {
                write_f32(&path, &y);
                println!("{case}: reference written ({} values, {secs:.2} s)", y.len());
                continue;
            }
            let reference = read_f32(&path);
            assert_eq!(reference.len(), y.len(), "{case}: length differs from the reference");
            let per = y.len() / n_src;
            let names: Vec<&str> = if n_src == 4 { htdemucs::SOURCES.to_vec() } else { vec![if matches!(r, Run::One(0)) { "drums" } else { "vocals" }] };
            let mut line = format!("{case:<22} {secs:6.2} s ");
            for (k, name) in names.iter().enumerate() {
                let snr = snr_db(&reference[k * per..(k + 1) * per], &y[k * per..(k + 1) * per]);
                worst = worst.min(snr);
                stems += 1;
                line += &if snr.is_infinite() { format!(" {name} identical") } else { format!(" {name} {snr:.1} dB") };
            }
            println!("{line}");
        }
    }
    if !make {
        println!("REFERENCE CHECK: {stems} stems, lowest SNR {}", if worst.is_infinite() { "inf (all identical)".to_string() } else { format!("{worst:.1} dB") });
        assert!(stems > 0 && worst >= 100.0, "a stem differs from the reference by more than rounding");
    }
}
