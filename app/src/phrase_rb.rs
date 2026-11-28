//! Phrase analysis ported from the reference application's analyser
//! (`analyzer::SongStructureAnalyze`, see `checks/phrase-analyser-trace.md`).
//!
//! Entry: the track's audio (stereo, any rate the decoder gives, on the 16-bit scale), the
//! tempo, the first beat's time and which of the first four beats is a downbeat. The analyser
//! works on a left-weighted mono mix (0.99 L + 0.01 R) in blocks of sr·128/75 samples. Stage A/B
//! looks at the first 210 seconds for the bar position; stage C/D goes over the whole track and
//! decides mood, phrases, fills and the ending.

use crate::model::Beat;

/// What the analyser is given (mirrors `SongStructureAnalyze::Initialize`).
#[derive(Clone, Debug)]
pub struct Input {
    pub sr: i32,
    /// Track length in sample frames.
    pub total: i32,
    /// Tempo × 100.
    pub bpm100: i32,
    /// Time of the first beat in ms (0 ≤ first_ms < one beat).
    pub first_ms: i32,
    /// 1-based index (1..=4) of the first downbeat among the first four beats.
    pub first_bar: i32,
    /// Key index of the analyser's key numbering (0..=23, < 12 major); 0 when unknown.
    pub key: i32,
}

/// The analyser's result (`SongStructureAnalyzeResult`): header and one row per phrase.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RbResult {
    /// First beat time (ms) after the analyser's own check.
    pub first_ms: i32,
    /// 1 High, 2 Mid, 3 Low.
    pub mood: i32,
    pub first_bar: i32,
    /// PSSI header byte 28 and 29.
    pub h4: i32,
    pub h5: i32,
    /// Last beat number of the track.
    pub end_beat: i32,
    /// Per phrase: beat, kind, k1, k2, b, beat2, beat3, beat4, k3, fill, fill beat.
    pub rows: Vec<[i32; 11]>,
}

/// The first downbeat's position among the first four beats, as the analyser's caller maps
/// the first beat's number in the bar (1..4 → 1, 4, 3, 2).
pub fn first_bar_index(beat_number: u8) -> i32 {
    match beat_number {
        2 => 4,
        3 => 3,
        4 => 2,
        _ => 1,
    }
}

/// This analyser extrapolates the whole grid from the first beat and a hundredth-BPM
/// clock. Leave variable-tempo grids and edited bar resets to the beat-aware analyser.
pub fn grid_input(beats: &[Beat]) -> Option<(i32, i32, i32)> {
    let b = beats.first()?;
    if !b.bpm.is_finite() || !b.time.is_finite() || b.time < 0.0 || !(1..=4).contains(&b.beat) {
        return None;
    }
    let bpm100 = (b.bpm * 100.0).round() as i32;
    let ms = (b.time * 1000.0).round() as i32;
    if !(2000..=40000).contains(&bpm100) || ms < 0 || (6_000_000.0 / bpm100 as f64) < (ms as f64) {
        return None;
    }
    let period = 6000.0 / f64::from(bpm100);
    let first = f64::from(ms) / 1000.0;
    for (i, beat) in beats.iter().enumerate() {
        let expected_number = 1 + ((usize::from(b.beat) - 1 + i) % 4) as u8;
        // ANLZ stores millisecond timestamps, so independently rounded beats can
        // differ slightly from the clock reconstructed using the rounded first beat.
        if !beat.time.is_finite() || !beat.bpm.is_finite() ||
            (beat.bpm * 100.0).round() != f64::from(bpm100) || beat.beat != expected_number ||
            (beat.time - (first + i as f64 * period)).abs() > 0.002 {
            return None;
        }
    }
    Some((bpm100, ms, first_bar_index(b.beat)))
}

/// Interleaved stereo floats (±1) to the analyser's 16-bit scale, rounded as a 16-bit decoder
/// output would be.
#[cfg(test)]
pub fn s16_channels(samples: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let q = |x: f32| (x * 32768.0).round().clamp(-32768.0, 32767.0);
    let n = samples.len() / 2;
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for c in samples.chunks_exact(2) {
        l.push(q(c[0]));
        r.push(q(c[1]));
    }
    (l, r)
}

/// The unmasked PSSI section bytes for a result, field for field as the reference writes it
/// (`CMusicAnalyzeFile::set_pmai_ext_song_struct_data`).
pub fn pssi_bytes(r: &RbResult) -> Vec<u8> {
    let n = r.rows.len();
    let mut v = Vec::with_capacity(32 + 24 * n);
    v.extend_from_slice(b"PSSI");
    v.extend_from_slice(&32u32.to_be_bytes());
    v.extend_from_slice(&((32 + 24 * n) as u32).to_be_bytes());
    v.extend_from_slice(&24u32.to_be_bytes());
    v.extend_from_slice(&(n as u16).to_be_bytes());
    v.extend_from_slice(&(r.mood as u16).to_be_bytes());
    v.extend_from_slice(&[0u8; 6]);
    v.extend_from_slice(&(r.end_beat as u16).to_be_bytes());
    v.push(r.h4 as u8);
    v.push(r.h5 as u8);
    v.extend_from_slice(&[0u8; 2]);
    for (i, e) in r.rows.iter().enumerate() {
        let f = [(i + 1) as i32, e[0], e[1], (e[2] != 0) as i32, (e[3] != 0) as i32, e[4], e[5], e[6], e[7], (e[8] != 0) as i32, (e[9] != 0) as i32, e[10]];
        for x in f {
            v.extend_from_slice(&(x as u16).to_be_bytes());
        }
    }
    v
}

/// Parse an unmasked or masked PSSI section into a result (everything the section keeps).
#[cfg(test)]
pub fn parse_pssi(section: &[u8]) -> Option<RbResult> {
    let mut s = section.to_vec();
    if crate::phrase::is_masked(&s) {
        crate::phrase::mask_section(&mut s);
    }
    if s.len() < 32 || &s[..4] != b"PSSI" {
        return None;
    }
    let u = |o: usize| u16::from_be_bytes([s[o], s[o + 1]]) as i32;
    let n = u(16) as usize;
    let mut rows = vec![];
    for k in 0..n {
        let o = 32 + 24 * k;
        if o + 24 > s.len() {
            return None;
        }
        let mut e = [0i32; 11];
        for (j, x) in e.iter_mut().enumerate() {
            *x = u(o + 2 + 2 * j);
        }
        rows.push(e);
    }
    Some(RbResult { first_ms: 0, mood: u(18), first_bar: 0, h4: s[28] as i32, h5: s[29] as i32, end_beat: u(26), rows })
}


/// The analyser's left-weighted mono mix of 16-bit-scale samples: 0.99·L + 0.01·R, scaled to ±1
/// (one fused multiply-add, as compiled).
#[cfg(test)]
pub fn mono_mix(l: &[f32], r: &[f32]) -> Vec<f64> {
    let c1 = f64::from_bits(0x3EFFAE147AE147AE);
    let c2 = f64::from_bits(0x3E947AE147AE147B);
    l.iter().zip(r).map(|(a, b)| (*b as f64).mul_add(c2, *a as f64 * c1)).collect()
}

/// Block length of the analyser's input loop (sr · 1.7066666666666668, rounded).
pub fn block_len(sr: i32) -> usize {
    (sr as f64 * 1.7066666666666668 + 0.5) as i32 as usize
}

/// Stage A: the first 210 s (or the whole track) low-passed and decimated by 8.
#[cfg(test)]
pub fn stage_a(mono: &[f64], sr: i32) -> Vec<f64> {
    let sr2 = sr as f64 * 0.125;
    let limit = ((sr2 * 210.0) as i32).min(mono.len() as i32 / 8).max(0) as usize;
    let mut d = crate::phrase_rb_dsp::Decimator::default();
    d.init_lowpass(sr2 * 0.4, sr as f64, 8);
    let mut out = Vec::with_capacity(limit + 16);
    let mut tmp = Vec::new();
    for blk in mono.chunks(block_len(sr).max(1)) {
        d.downsample(blk, &mut tmp);
        out.extend_from_slice(&tmp);
        if out.len() >= limit {
            break;
        }
    }
    out.resize(limit, 0.0);
    out
}

/// The analyser's key input for a recordbox key index (0..11 minor 1A..12A, 12..23 major
/// 1B..12B, −1 unknown), as rekordbox's analysis thread maps its own key index (even = minor,
/// odd = major) before `Initialize` (table at 0x103752aa0). Unknown → 0. Only PSSI byte 29
/// (key < 12, i.e. major or unknown) depends on it. recordbox passes −1 (see `phrase::analyze_audio`).
pub fn analyser_key(key: i32) -> i32 {
    const T: [i32; 24] = [23, 2, 18, 9, 13, 4, 20, 11, 15, 6, 22, 1, 17, 8, 12, 3, 19, 10, 14, 5, 21, 0, 16, 7];
    if !(0..24).contains(&key) {
        return 0;
    }
    let i = if key < 12 { 2 * key } else { 2 * (key - 12) + 1 };
    T[i as usize]
}

/// The whole port of rekordbox's phrase analyser: interleaved stereo samples (±1), the beat
/// grid and the key index → the result rekordbox stores. `None` where the port does not cover
/// the track (no usable grid, or a structure path that is not ported yet).
#[cfg(test)]
pub fn analyze(samples: &[f32], sr: u32, beats: &[Beat], key: i32) -> Option<RbResult> {
    analyze_with_cancel(samples, sr, beats, key, &|| false)
}

// Stage C keeps two 1024-bin f64 spectra per beat and a 72-band chromagram.
// Bound both the time-based and beat-based arrays before admitting a job.
pub(crate) const MAX_SECONDS: usize = 900;
pub(crate) const MAX_BEATS: usize = 4096;
static ACTIVE: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// The reference's 16-bit quantization and weighted stereo mix, one block at a time.
fn mix_block(samples: &[f32], out: &mut Vec<f64>) -> Option<()> {
    out.clear();
    let q = |v: f32| f64::from((v * 32768.0).round().clamp(-32768.0, 32767.0));
    let left_weight = f64::from_bits(0x3EFFAE147AE147AE);
    let right_weight = f64::from_bits(0x3E947AE147AE147B);
    for frame in samples.chunks_exact(2) {
        if !frame[0].is_finite() || !frame[1].is_finite() { return None; }
        out.push(q(frame[1]).mul_add(right_weight, q(frame[0]) * left_weight));
    }
    Some(())
}

pub fn analyze_with_cancel(samples: &[f32], sr: u32, beats: &[Beat], key: i32, cancelled: &dyn Fn() -> bool) -> Option<RbResult> {
    let frames = samples.len() / 2;
    if cancelled() || !(22_050..=96_000).contains(&sr) || samples.len() % 2 != 0 ||
        frames == 0 || frames > MAX_SECONDS * sr as usize || !(8..=MAX_BEATS).contains(&beats.len()) {
        return None;
    }
    let (bpm100, first_ms, first_bar) = grid_input(beats)?;
    // The reference reader gives the classifier length minus one. Its eight-beat
    // admission check starts at the first beat and excludes 32 filter frames.
    let frames = frames - 1;
    let beat_frames = frames as f64 - 32.0 - (f64::from(first_ms) * f64::from(sr) / 1000.0);
    let beats_per_frame = f64::from(bpm100) / (6000.0 * f64::from(sr));
    if beat_frames * beats_per_frame < 8.0 || frames as f64 * beats_per_frame > MAX_BEATS as f64 {
        return None;
    }
    let inp = Input { sr: sr as i32, total: frames as i32, bpm100, first_ms, first_bar, key };
    // Rapid deck changes and simultaneous USB exports must not multiply the large
    // working set. A cancelled job also leaves the queue before acquiring the lane.
    let _lane = loop {
        if cancelled() { return None; }
        if let Some(lane) = ACTIVE.try_lock_for(std::time::Duration::from_millis(50)) { break lane; }
    };
    // An unmodelled structure must not strand a deck worker or fail an entire export.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| analyze_blocks(&samples[..2 * frames], &inp, cancelled))).ok().flatten()
}

fn analyze_blocks(samples: &[f32], inp: &Input, cancelled: &dyn Fn() -> bool) -> Option<RbResult> {
    let sr = f64::from(inp.sr);
    let bpm = f64::from(inp.bpm100) * 0.01;
    let block = block_len(inp.sr);
    let mut mono = Vec::with_capacity(block);
    let b = {
        let limit = (inp.total as usize / 8).min(inp.sr as usize * 210 / 8);
        let mut dec = Vec::with_capacity(limit);
        let mut tmp = Vec::new();
        let mut filter = crate::phrase_rb_dsp::Decimator::default();
        filter.init_lowpass(sr * 0.125 * 0.4, sr, 8);
        for stereo in samples.chunks(2 * block) {
            if cancelled() { return None; }
            mix_block(stereo, &mut mono)?;
            filter.downsample(&mono, &mut tmp);
            let needed = limit - dec.len();
            dec.extend_from_slice(&tmp[..needed.min(tmp.len())]);
            if dec.len() == limit { break; }
        }
        dec.resize(limit, 0.0);
        if cancelled() { return None; }
        crate::phrase_rb_b::sub_main_b_cancellable(&dec, sr * 0.125, bpm, inp.first_ms, inp.first_bar, cancelled)?
    };
    if cancelled() { return None; }
    let mut c = crate::phrase_rb_c::StageC::init(inp.sr, inp.total, bpm, b.first_ms, b.first_bar)?;
    if !c.go() { return None; }
    for stereo in samples.chunks(2 * block) {
        if cancelled() { return None; }
        mix_block(stereo, &mut mono)?;
        c.block(&mono)?;
        if c.state == 2 { break; }
    }
    if cancelled() { return None; }
    if c.state != 2 { c.block(&[])?; }
    if c.state != 2 { return None; }
    let din = crate::phrase_rb_d::DIn { c: &c, b: &b, bpm, sr, sr8: sr * 0.125, total: inp.total, ms_in: inp.first_ms, key: inp.key };
    let d = crate::phrase_rb_d::sub_main_d_cancellable(&din, &mut Default::default(), cancelled)?;
    if cancelled() || !(1..=3).contains(&d.mood) || !(1..=u16::MAX as i32).contains(&d.end) ||
        d.rows.is_empty() || d.rows.len() != usize::try_from(d.n).ok()? ||
        d.rows.iter().any(|row| !(1..=d.end).contains(&row[0])) ||
        d.rows.windows(2).any(|w| w[0][0] >= w[1][0]) {
        return None;
    }
    Some(RbResult { first_ms: d.first_ms, mood: d.mood, first_bar: d.bar, h4: d.h4, h5: d.h5, end_beat: d.end, rows: d.rows })
}

/// Stage C over the whole track in the analyser's blocks; stops after the block that ends it.
#[cfg(test)]
pub fn stage_c(mono: &[f64], inp: &Input, b: &crate::phrase_rb_b::StageB) -> Option<crate::phrase_rb_c::StageC> {
    let mut c = crate::phrase_rb_c::StageC::init(inp.sr, inp.total, inp.bpm100 as f64 * 0.01, b.first_ms, b.first_bar)?;
    if !c.go() {
        return None;
    }
    for blk in mono.chunks(block_len(inp.sr).max(1)) {
        c.block(blk)?;
        if c.state == 2 {
            return Some(c);
        }
    }
    if c.state != 2 {
        c.block(&[])?;
    }
    (c.state == 2).then_some(c)
}

/// The stage C arrays stage D reads, by the offset the reference moves them to.
#[cfg(test)]
pub fn stage_c_arrays(c: &crate::phrase_rb_c::StageC) -> Vec<(&'static str, Vec<f64>)> {
    let f = |v: &Vec<f64>| v.clone();
    let u = |v: &Vec<u8>, n: usize| v.iter().take(n).map(|x| *x as f64).collect::<Vec<f64>>();
    vec![
        ("s3664", f(&c.g2968)),
        ("s3696", f(&c.g3000)),
        ("s3736", f(&c.g3032)),
        ("s3768", f(&c.g3064)),
        ("s3808", c.g3616.d.clone()),
        ("s3832", f(&c.g3160)),
        ("s3864", f(&c.g3096)),
        ("s3896", u(&c.g3512, c.g3512.len())),
        ("s3928", u(&c.g3448, c.n3448)),
        ("s3960", f(&c.g3224)),
        ("s3992", u(&c.g3480, c.n3480)),
        ("s4024", f(&c.g3128)),
        ("s4056", f(&c.g3256)),
        ("s4088", f(&c.g3288)),
        ("s4120", f(&c.g3320)),
        ("s4176", c.g3592.d.clone()),
        ("s4208", f(&c.g3352)),
        ("s4240", c.g3568.d.clone()),
        ("s4264", vec![c.f888 as f64]),
        ("s4268", vec![c.f889 as f64]),
        ("s4272", f(&c.g3384)),
        ("s4304", f(&c.g3416)),
        ("s4336", vec![c.zero_flag as f64]),
        ("s4344", c.bt8.iter().map(|x| *x as f64).collect()),
    ]
}

#[cfg(test)]
pub const HOOK_D: &str = "3664:f,3696:f,3736:f,3768:f,3808:F,3832:f,3864:f,3896:u,3928:u,3960:f,3992:u,4024:f,4056:f,4088:f,4120:f,4176:F,4208:f,4240:F,4264:n,4268:n,4272:f,4304:f,4336:n,4344:i,3728:n,3800:n";

#[cfg(test)]
mod unit {
    use super::*;

    fn grid(n: usize, bpm: f64, first: f64, beat1: u8) -> Vec<Beat> {
        (0..n).map(|k| Beat { time: first + k as f64 * 60.0 / bpm, bpm, beat: ((beat1 as usize - 1 + k) % 4 + 1) as u8, index: k }).collect()
    }

    #[test]
    fn pssi_bytes_roundtrip() {
        let r = RbResult { first_ms: 0, mood: 1, first_bar: 0, h4: 0, h5: 1, end_beat: 400, rows: vec![[1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0], [65, 2, 0, 1, 1, 81, 89, 93, 0, 1, 127], [129, 5, 1, 0, 0, 0, 0, 0, 0, 0, 0]] };
        let b = pssi_bytes(&r);
        assert_eq!(b.len(), 32 + 24 * 3);
        assert_eq!((b[26], b[27], b[28], b[29]), (1, 144, 0, 1));
        assert_eq!(parse_pssi(&b), Some(r.clone()));
        // The structure writer gives the same bytes.
        assert_eq!(crate::phrase::pssi(&crate::phrase::parse(&b).unwrap()), b);
    }

    #[test]
    fn grid_input_maps_the_first_beat() {
        assert_eq!(grid_input(&grid(8, 128.0, 0.1, 1)), Some((12800, 100, 1)));
        assert_eq!(grid_input(&grid(8, 128.0, 0.1, 2)).map(|g| g.2), Some(4));
        assert_eq!(grid_input(&grid(8, 128.0, 0.1, 4)).map(|g| g.2), Some(2));
        // A first beat later than one beat period is not a grid the analyser accepts.
        assert_eq!(grid_input(&grid(8, 128.0, 0.6, 1)), None);
        assert_eq!(grid_input(&[]), None);
    }

    #[test]
    fn accepts_quantized_grid_but_declines_tempo_changes_and_bar_resets() {
        let mut beats = grid(513, 129.37, 0.0134, 3);
        for beat in &mut beats { beat.time = (1000.0 * beat.time).round() / 1000.0; }
        assert_eq!(grid_input(&beats), Some((12937, 13, 3)));
        let last = beats.last_mut().unwrap();
        last.bpm = 129.35;
        assert!(grid_input(&beats).is_none());
        beats.last_mut().unwrap().bpm = 129.37;
        beats[256].beat = 1;
        assert!(grid_input(&beats).is_none());
        beats[256].beat = 3;
        beats[256].time += 0.008;
        assert!(grid_input(&beats).is_none());
        beats[256].time = f64::NAN;
        assert!(grid_input(&beats).is_none());
    }

    #[test]
    fn key_input_marks_major_keys() {
        // Byte 29 is 1 for major keys and for an unknown key, 0 for minor ones.
        assert!(analyser_key(-1) < 12);
        assert!((0..12).all(|k| analyser_key(k) >= 12), "minor 1A..12A");
        assert!((12..24).all(|k| analyser_key(k) < 12), "major 1B..12B");
    }

    #[test]
    fn streamed_mono_matches_the_reference_input_at_block_boundaries() {
        let samples = [-1.1, 1.1, -0.125, 0.25, 0.0, -0.00001, 0.31, -0.73, 0.99, 1.0];
        let (l, r) = s16_channels(&samples);
        let reference = mono_mix(&l, &r);
        let mut joined = Vec::new();
        let mut block = Vec::new();
        for chunk in samples.chunks(4) {
            mix_block(chunk, &mut block).unwrap();
            joined.extend_from_slice(&block);
        }
        assert_eq!(joined, reference);
        assert!(mix_block(&[f32::NAN, 0.0], &mut block).is_none());
    }

    #[test]
    fn cancellation_and_unsupported_input_stop_before_classification() {
        let sr = 22_050;
        let samples = vec![0.0; sr as usize * 2 * 10];
        let beats = grid(20, 120.0, 0.0, 1);
        let checks = std::cell::Cell::new(0);
        let cancelled = || { checks.set(checks.get() + 1); checks.get() >= 4 };
        assert!(analyze_with_cancel(&samples, sr, &beats, 0, &cancelled).is_none());
        assert_eq!(checks.get(), 4);
        assert!(analyze_with_cancel(&samples, 192_000, &beats, 0, &|| false).is_none());
        assert!(analyze_with_cancel(&samples, sr, &grid(20, f64::NAN, 0.0, 1), 0, &|| false).is_none());
    }

    #[test]
    fn disabled_diagnostics_do_not_build_or_copy_trace_buffers() {
        let mut tr = crate::phrase_rb_d::Trace::default();
        tr.rec("disabled", || panic!("disabled trace allocated its arguments"));
        tr.on = true;
        tr.rec("enabled", || vec![(0, vec![1.0])]);
        assert_eq!(tr.calls.len(), 1);
        assert_eq!(tr.calls[0].args[0].1, [1.0]);
    }

    #[test]
    fn type_two_phrases_match_upstream_on_layered_tone_and_isolated_kicks() {
        // The reference produces a two-phrase Low result for this tone and a
        // single Mid phrase for the kicks. Neither case used the old type-1 path.
        let sr = 22_050;
        let cases: &[(f64, i32, i32, &[(i32, i32)])] = &[
            (128.0, 3, 63, &[(1, 9), (33, 9)]),
            (94.0, 2, 64, &[(1, 1)]),
        ];
        for &(bpm, mood, end_beat, phrases) in cases {
            let period = 60.0 / bpm;
            let frames = (64.0 * period * f64::from(sr)) as usize;
            let mut stereo = Vec::with_capacity(frames * 2);
            for i in 0..frames {
                let t = i as f64 / f64::from(sr);
                let beat = (t / period) as usize;
                let phase = t - beat as f64 * period;
                let sample = if mood == 3 {
                    let level = if beat < 32 { 0.18 } else { 0.35 };
                    level * (std::f64::consts::TAU * 440.0 * t).sin()
                } else if beat % 2 == 0 {
                    0.3 * (-30.0 * phase).exp() * (std::f64::consts::TAU * 55.0 * phase).sin()
                } else {
                    0.0
                };
                stereo.extend([sample as f32; 2]);
            }
            let result = analyze_with_cancel(&stereo, sr, &grid(64, bpm, 0.0, 1), 0, &|| false).unwrap();
            assert_eq!((result.mood, result.end_beat), (mood, end_beat));
            let rows: Vec<_> = phrases.iter().map(|&(beat, kind)| [beat, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0]).collect();
            assert_eq!(result.rows, rows);
        }
    }

    /// A synthetic 4/4 track (hat-only intro, kick section, break, kick section, hat outro):
    /// the port gives an ordered High-mood structure inside the track.
    #[test]
    fn port_structure_on_a_synthetic_track() {
        let sr = 44100u32;
        let bpm = 128.0;
        let beat = 60.0 / bpm;
        let bars = [(16, 1u8), (32, 3), (16, 2), (32, 3), (16, 1)];
        let total_beats: usize = bars.iter().map(|b| b.0 * 4).sum();
        let n = (total_beats as f64 * beat * sr as f64) as usize + sr as usize;
        let mut x = vec![0f32; n];
        let mut b0 = 0usize;
        let mut seed = 1u32;
        for (len, kind) in bars {
            for k in 0..len * 4 {
                let t0 = ((b0 + k) as f64 * beat * sr as f64) as usize;
                for i in 0..(beat * sr as f64) as usize {
                    let t = i as f64 / sr as f64;
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let noise = (seed >> 9) as f64 / (1u32 << 23) as f64 - 1.0;
                    let mut v = 0.0;
                    if kind & 1 != 0 && (t - beat / 2.0).abs() < 0.02 {
                        v += 0.2 * noise;
                    }
                    if kind & 2 != 0 {
                        v += 0.25 * (t * 220.0 * std::f64::consts::TAU).sin();
                    }
                    if kind == 3 && t < 0.15 {
                        v += 0.8 * (-t * 25.0).exp() * (t * 55.0 * std::f64::consts::TAU).sin();
                    }
                    if let Some(s) = x.get_mut(t0 + i) {
                        *s = v as f32;
                    }
                }
            }
            b0 += len * 4;
        }
        let samples: Vec<f32> = x.iter().flat_map(|v| [*v, *v]).collect();
        let r = analyze(&samples, sr, &grid(total_beats, bpm, 0.0, 1), 0).expect("the port covers a High-mood track");
        assert_eq!(r.mood, 1);
        assert!(!r.rows.is_empty());
        assert!(r.rows.windows(2).all(|w| w[0][0] < w[1][0]), "phrase starts ascend: {:?}", r.rows);
        assert!(r.rows.iter().all(|e| (1..=6).contains(&e[1])), "High-mood kinds: {:?}", r.rows);
        assert!(r.end_beat as usize <= total_beats + 4 && r.rows.last().unwrap()[0] < r.end_beat);
    }
}

#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
pub(crate) mod oracle {
    //! Runs the native research harness (`checks/phrase-oracle-harness/rbphrase.c`) on the same
    //! PCM and grid as the port. PREPDECK_RBPHRASE = directory with `rbphrase` and
    //! `fixups.txt`, PREPDECK_RBSLICE = the arm64 slice.
    use super::*;

    #[derive(Clone, Debug, Default)]
    pub struct OracleOut {
        pub result: RbResult,
        pub secs: f64,
        pub stdout: String,
    }

    pub fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let base = std::env::var("PREPDECK_SCRATCH").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        let d = base.join(format!("phrase-oracle-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    pub fn write_s16(path: &std::path::Path, l: &[f32], r: &[f32]) -> std::io::Result<()> {
        let mut b = Vec::with_capacity(l.len() * 4);
        for (a, c) in l.iter().zip(r) {
            b.extend_from_slice(&(*a as i16).to_le_bytes());
            b.extend_from_slice(&(*c as i16).to_le_bytes());
        }
        std::fs::write(path, b)
    }

    pub fn parse_out(s: &str) -> RbResult {
        let mut r = RbResult::default();
        for l in s.lines() {
            let v: Vec<&str> = l.split_whitespace().collect();
            if v.first() == Some(&"result") && v.len() >= 15 {
                let g = |i: usize| v[i].parse::<i32>().unwrap_or(0);
                r.first_ms = g(4);
                r.mood = g(6);
                r.first_bar = g(8);
                r.h4 = g(10);
                r.h5 = g(12);
                r.end_beat = g(14);
            } else if v.first() == Some(&"e") && v.len() == 12 {
                let mut e = [0i32; 11];
                for (j, x) in e.iter_mut().enumerate() {
                    *x = v[j + 1].parse().unwrap_or(0);
                }
                for j in [2, 3, 8, 9] {
                    e[j] = (e[j] != 0) as i32;
                }
                r.rows.push(e);
            }
        }
        r
    }

    /// Run the harness; `env` adds variables (stage dumps).
    pub fn run(pcm: &std::path::Path, inp: &Input, env: &[(&str, String)]) -> Option<OracleOut> {
        let dir = std::env::var("PREPDECK_RBPHRASE").ok()?;
        let slice = std::env::var("PREPDECK_RBSLICE").ok()?;
        let mut cmd = std::process::Command::new(format!("{dir}/rbphrase"));
        cmd.args([slice.as_str(), &format!("{dir}/fixups.txt"), pcm.to_str()?, &inp.sr.to_string(), &inp.bpm100.to_string(), &inp.first_ms.to_string(), &rev_bar(inp.first_bar).to_string(), &inp.key.to_string()]);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let o = cmd.output().ok()?;
        let s = String::from_utf8_lossy(&o.stdout).to_string();
        let secs = s.lines().find_map(|l| l.strip_prefix("secs ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
        Some(OracleOut { result: parse_out(&s), secs, stdout: s })
    }

    /// The harness takes the first beat's number in the bar; map the index back.
    fn rev_bar(i: i32) -> i32 {
        match i {
            4 => 2,
            3 => 3,
            2 => 4,
            _ => 1,
        }
    }

    pub struct Track {
        pub audio: String,
        pub beats: Vec<Beat>,
        pub stored: RbResult,
    }

    /// Tracks of PREPDECK_ANLZ_PAIRS with a stored PSSI and grid (read only).
    pub fn tracks() -> Vec<Track> {
        let Ok(f) = std::env::var("PREPDECK_ANLZ_PAIRS") else { return vec![] };
        let mut out = vec![];
        for l in std::fs::read_to_string(f).unwrap_or_default().lines() {
            let Some((audio, base)) = l.split_once('\t') else { continue };
            let ext = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.EXT")));
            let Some(stored) = ext.get("PSSI").and_then(|s| parse_pssi(s)) else { continue };
            let dat = crate::export::anlz::read_sections(std::path::Path::new(&format!("{base}.DAT")));
            let Some(q) = dat.get("PQTZ") else { continue };
            let n = u32::from_be_bytes(q[20..24].try_into().unwrap()) as usize;
            let beats: Vec<Beat> = (0..n)
                .filter_map(|k| {
                    let o = 24 + k * 8;
                    let e = q.get(o..o + 8)?;
                    Some(Beat { time: u32::from_be_bytes(e[4..8].try_into().unwrap()) as f64 / 1000.0, bpm: u16::from_be_bytes([e[2], e[3]]) as f64 / 100.0, beat: u16::from_be_bytes([e[0], e[1]]) as u8, index: k })
                })
                .collect();
            if !std::path::Path::new(audio).exists() {
                continue;
            }
            out.push(Track { audio: audio.to_string(), beats, stored });
        }
        out
    }

    pub fn row_diff(a: &RbResult, b: &RbResult) -> String {
        let f = |r: &RbResult| r.rows.iter().map(|e| format!("{}:{}", e[0], e[1])).collect::<Vec<_>>().join(" ");
        format!("mood {}/{} end {}/{} h4 {}/{} h5 {}/{}\n    A {}\n    B {}", a.mood, b.mood, a.end_beat, b.end_beat, a.h4, b.h4, a.h5, b.h5, f(a), f(b))
    }

    /// Step 1 check: the harness on recordbox's decode and rekordbox's stored grid reproduces
    /// the stored PSSI. PREPDECK_ANLZ_PAIRS + PREPDECK_RBPHRASE + PREPDECK_RBSLICE.
    #[test]
    #[ignore]
    fn phrase_oracle_vs_stored() {
        let rows = tracks();
        let lim: usize = std::env::var("PREPDECK_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
        let env: Vec<(String, String)> = std::env::var("PREPDECK_ORACLE_ENV").map(|s| s.split(',').filter_map(|kv| kv.split_once('=').map(|(a, b)| (a.to_string(), b.to_string()))).collect()).unwrap_or_default();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let res = std::sync::Mutex::new(vec![]);
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= rows.len().min(lim) {
                        break;
                    }
                    let t = &rows[i];
                    let Ok(d) = crate::audio::decode_file(&t.audio) else { continue };
                    let Some((bpm100, first_ms, first_bar)) = grid_input(&t.beats) else {
                        res.lock().unwrap().push((i, None));
                        continue;
                    };
                    let (l, r) = s16_channels(&d.samples);
                    let inp = Input { sr: d.sr as i32, total: l.len() as i32, bpm100, first_ms, first_bar, key: 0 };
                    let dir = tmp_dir(&i.to_string());
                    let pcm = dir.join("pcm.s16");
                    let envr: Vec<(&str, String)> = env.iter().map(|(a, b)| (a.as_str(), b.clone())).collect();
                    let out = write_s16(&pcm, &l, &r).ok().and_then(|_| run(&pcm, &inp, &envr));
                    let _ = std::fs::remove_dir_all(&dir);
                    res.lock().unwrap().push((i, out));
                });
            }
        });
        let mut r = res.into_inner().unwrap();
        r.sort_by_key(|x| x.0);
        let (mut same, mut same_rows, mut n) = (0, 0, 0);
        for (i, o) in &r {
            let t = &rows[*i];
            let name = t.audio.rsplit('/').next().unwrap_or("");
            let Some(o) = o else {
                eprintln!("SKIP (grid) {name}");
                continue;
            };
            n += 1;
            let rows_eq = o.result.rows == t.stored.rows && o.result.mood == t.stored.mood;
            let all_eq = rows_eq && o.result.end_beat == t.stored.end_beat && o.result.h4 == t.stored.h4 && o.result.h5 == t.stored.h5;
            same += all_eq as usize;
            same_rows += rows_eq as usize;
            if !all_eq {
                eprintln!("DIFF {name} ({:.2} s)\n  {}", o.secs, row_diff(&o.result, &t.stored));
            }
        }
        eprintln!("oracle vs stored: {same}/{n} equal in every field, {same_rows}/{n} equal phrases+mood");
    }

    /// The port's final result against the stored PSSI and the harness (PREPDECK_NOHARNESS=1
    /// skips the harness). Also times both.
    #[test]
    #[ignore]
    fn phrase_port_vs_stored() {
        let rows = tracks();
        let lim: usize = std::env::var("PREPDECK_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
        let want = std::env::var("PREPDECK_TRACK").ok();
        let noh = std::env::var("PREPDECK_NOHARNESS").is_ok();
        let sel: Vec<usize> = (0..rows.len()).filter(|i| want.as_ref().map(|w| rows[*i].audio.contains(w.as_str())).unwrap_or(true)).take(lim).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let res = std::sync::Mutex::new(vec![]);
        let threads: usize = std::env::var("PREPDECK_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| loop {
                    let j = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&i) = sel.get(j) else { break };
                    let t = &rows[i];
                    let Ok(d) = crate::audio::decode_file(&t.audio) else { continue };
                    let t0 = std::time::Instant::now();
                    let port = analyze(&d.samples, d.sr, &t.beats, 0);
                    let tp = t0.elapsed().as_secs_f64();
                    let mut harness = None;
                    if !noh {
                        if let Some((bpm100, first_ms, first_bar)) = grid_input(&t.beats) {
                            let (l, r) = s16_channels(&d.samples);
                            let inp = Input { sr: d.sr as i32, total: l.len() as i32, bpm100, first_ms, first_bar, key: 0 };
                            let dir = tmp_dir(&format!("p{i}"));
                            let pcm = dir.join("pcm.s16");
                            harness = write_s16(&pcm, &l, &r).ok().and_then(|_| run(&pcm, &inp, &[]));
                            let _ = std::fs::remove_dir_all(&dir);
                        }
                    }
                    res.lock().unwrap().push((i, port, tp, harness));
                });
            }
        });
        let mut r = res.into_inner().unwrap();
        r.sort_by_key(|x| x.0);
        let (mut n, mut eq_s, mut eq_sr, mut eq_h, mut nh, mut none) = (0, 0, 0, 0, 0, 0);
        let (mut tp_sum, mut th_sum) = (0.0, 0.0);
        let full = |a: &RbResult, b: &RbResult| a.rows == b.rows && a.mood == b.mood && a.end_beat == b.end_beat && a.h4 == b.h4 && a.h5 == b.h5;
        for (i, p, tp, h) in &r {
            let t = &rows[*i];
            let name = t.audio.rsplit('/').next().unwrap_or("");
            n += 1;
            tp_sum += tp;
            let Some(p) = p else {
                none += 1;
                eprintln!("NONE {name}");
                continue;
            };
            let rows_eq = p.rows == t.stored.rows && p.mood == t.stored.mood;
            eq_sr += rows_eq as usize;
            let all = full(p, &t.stored);
            eq_s += all as usize;
            if let Some(h) = h {
                nh += 1;
                th_sum += h.secs;
                let he = full(p, &h.result) && p.first_ms == h.result.first_ms && p.first_bar == h.result.first_bar;
                eq_h += he as usize;
                if !he {
                    eprintln!("DIFF-H {name}\n  {}", row_diff(p, &h.result));
                }
            }
            if !all {
                eprintln!("DIFF-S {name}\n  {}", row_diff(p, &t.stored));
            }
        }
        eprintln!("port vs stored: {eq_s}/{n} equal in every field, {eq_sr}/{n} equal phrases+mood, {none} not covered");
        eprintln!("port vs harness: {eq_h}/{nh} equal in every field");
        eprintln!("time per track: port {:.3} s, harness {:.3} s (mean, {threads} threads in parallel)", tp_sum / n.max(1) as f64, th_sum / nh.max(1) as f64);
    }

    /// Parse the harness's stage dump (RB_DUMP) after pass `pass`.
    pub fn parse_dump(text: &str, pass: usize) -> std::collections::HashMap<String, Vec<f64>> {
        let mut out = std::collections::HashMap::new();
        let mut cur = usize::MAX;
        for l in text.lines() {
            if let Some(p) = l.strip_prefix("# after pass ") {
                cur = p.trim().parse().unwrap_or(usize::MAX);
                continue;
            }
            if cur != pass {
                continue;
            }
            let v: Vec<&str> = l.split_whitespace().collect();
            if v.is_empty() {
                continue;
            }
            if v[0] == "first_ms" {
                for kv in v.chunks(2) {
                    if kv.len() == 2 {
                        out.insert(kv[0].to_string(), vec![kv[1].parse().unwrap_or(0.0)]);
                    }
                }
            } else {
                out.insert(v[0].to_string(), v[2..].iter().map(|x| x.parse().unwrap_or(f64::NAN)).collect());
            }
        }
        out
    }

    /// Stage compare: the port's stage B values against the harness's on the same input.
    #[test]
    #[ignore]
    fn phrase_stage_compare() {
        let rows = tracks();
        let lim: usize = std::env::var("PREPDECK_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let res = std::sync::Mutex::new(vec![]);
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= rows.len().min(lim) {
                        break;
                    }
                    let t = &rows[i];
                    let Ok(d) = crate::audio::decode_file(&t.audio) else { continue };
                    let Some((bpm100, first_ms, first_bar)) = grid_input(&t.beats) else { continue };
                    let (l, r) = s16_channels(&d.samples);
                    let inp = Input { sr: d.sr as i32, total: l.len() as i32, bpm100, first_ms, first_bar, key: 0 };
                    let dir = tmp_dir(&format!("s{i}"));
                    let pcm = dir.join("pcm.s16");
                    let dump = dir.join("dump.txt");
                    let hk = dir.join("hooks.txt");
                    let out = write_s16(&pcm, &l, &r).ok().and_then(|_| run(&pcm, &inp, &[("RB_DUMP", dump.to_string_lossy().to_string()), ("RB_HOOKS", hk.to_string_lossy().to_string()), ("RB_HOOK_before_D", HOOK_D.to_string())]));
                    let text = std::fs::read_to_string(&dump).unwrap_or_default();
                    let hooks = parse_hooks(&std::fs::read_to_string(&hk).unwrap_or_default());
                    let _ = std::fs::remove_dir_all(&dir);
                    let mono = mono_mix(&l, &r);
                    let dec = stage_a(&mono, inp.sr);
                    let b = crate::phrase_rb_b::sub_main_b(&dec, inp.sr as f64 * 0.125, bpm100 as f64 * 0.01, first_ms, first_bar);
                    let o = parse_dump(&text, 0);
                    let mut diffs = vec![];
                    match (&b, out) {
                        (Some(b), Some(_)) => {
                            let g = |k: &str| o.get(k).cloned().unwrap_or_default();
                            let chk = |name: &str, mine: Vec<f64>, theirs: Vec<f64>, diffs: &mut Vec<String>| {
                                if mine != theirs {
                                    let first = mine.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(mine.len().min(theirs.len()));
                                    diffs.push(format!("{name}: len {}/{} first diff at {first}: {:?} vs {:?}", mine.len(), theirs.len(), mine.get(first), theirs.get(first)));
                                }
                            };
                            chk("f520", vec![b.f520 as f64], g("f520"), &mut diffs);
                            chk("f592", vec![b.f592 as f64], g("f592"), &mut diffs);
                            chk("f596", vec![b.f596 as f64], g("f596"), &mut diffs);
                            chk("f600", vec![b.f600 as f64], g("f600"), &mut diffs);
                            chk("m528", b.m528.iter().map(|x| *x as f64).collect(), g("m528"), &mut diffs);
                            chk("m560", b.m560.clone(), g("m560"), &mut diffs);
                            let c = stage_c(&mono, &inp, b);
                            match (c, hooks.iter().find(|h| h.0.starts_with("before_D"))) {
                                (Some(c), Some((_, h))) => {
                                    let mut sizes = vec![];
                                    for (name, mine) in stage_c_arrays(&c) {
                                        let theirs = h.get(name).cloned().unwrap_or_default();
                                        sizes.push(format!("{name}:{}", mine.len()));
                                        let theirs = if name == "s4240" || name == "s3808" || name == "s4176" { theirs.get(1..).map(|x| x.to_vec()).unwrap_or_default() } else { theirs };
                                        chk(name, mine, theirs, &mut diffs);
                                    }
                                    if std::env::var("PREPDECK_SIZES").is_ok() {
                                        eprintln!("sizes {}", sizes.join(" "));
                                    }
                                    let gi = |k: &str| h.get(k).and_then(|v| v.first().copied()).unwrap_or(-1.0);
                                    chk("s3728", vec![c.nbeats as f64], vec![gi("s3728")], &mut diffs);
                                }
                                (c, h) => diffs.push(format!("stage C: port {} oracle {}", c.is_some(), h.is_some())),
                            }
                        }
                        _ => diffs.push(format!("port {} oracle ran", b.is_some())),
                    }
                    res.lock().unwrap().push((i, diffs));
                });
            }
        });
        let mut r = res.into_inner().unwrap();
        r.sort_by_key(|x| x.0);
        let mut same = 0;
        for (i, d) in &r {
            if d.is_empty() {
                same += 1;
            } else {
                eprintln!("DIFF {}\n  {}", rows[*i].audio.rsplit('/').next().unwrap_or(""), d.join("\n  "));
            }
        }
        eprintln!("stages B+C: {same}/{} equal", r.len());
    }

    /// Parse a hooks file: for each "# name hit" block, the arrays "sOFF n values…".
    pub fn parse_hooks(text: &str) -> Vec<(String, std::collections::HashMap<String, Vec<f64>>)> {
        let mut out: Vec<(String, std::collections::HashMap<String, Vec<f64>>)> = vec![];
        for l in text.lines() {
            if let Some(h) = l.strip_prefix("# ") {
                out.push((h.trim().to_string(), Default::default()));
                continue;
            }
            let v: Vec<&str> = l.split_whitespace().collect();
            if v.len() >= 2 {
                if let Some(last) = out.last_mut() {
                    let skip = if v[0].starts_with('s') && l.split_whitespace().count() > 2 && v.get(2).is_some() { 2 } else { 2 };
                    last.1.insert(v[0].to_string(), v[skip.min(v.len())..].iter().map(|x| x.parse().unwrap_or(f64::NAN)).collect());
                }
            }
        }
        out
    }

    pub fn first_diff(a: &[f64], b: &[f64]) -> Option<usize> {
        if a.len() != b.len() {
            return Some(a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len())));
        }
        a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits() && !(x.is_nan() && y.is_nan()))
    }

    /// Debug probe on one track (PREPDECK_TRACK = part of the file name): compares named
    /// intermediate arrays of the port with the harness's call-site dumps.
    #[test]
    #[ignore]
    fn phrase_probe() {
        let Ok(want) = std::env::var("PREPDECK_TRACK") else { return };
        let Some(t) = tracks().into_iter().find(|t| t.audio.contains(&want)) else { return };
        let d = crate::audio::decode_file(&t.audio).unwrap();
        let (bpm100, first_ms, first_bar) = grid_input(&t.beats).unwrap();
        let (l, r) = s16_channels(&d.samples);
        let inp = Input { sr: d.sr as i32, total: l.len() as i32, bpm100, first_ms, first_bar, key: 0 };
        let dir = tmp_dir("probe");
        let pcm = dir.join("pcm.s16");
        let hk = dir.join("hooks.txt");
        write_s16(&pcm, &l, &r).unwrap();
        let mut env: Vec<(&str, String)> = vec![("RB_HOOKS", hk.to_string_lossy().to_string())];
        if let Ok(dt) = std::env::var("PREPDECK_DTRACE") {
            env.push(("RB_DTRACE", dt));
        }
        let specs: Vec<(String, String)> = std::env::var("PREPDECK_HOOKS").unwrap_or_default().split(';').filter_map(|kv| kv.split_once('=').map(|(a, b)| (format!("RB_HOOK_{a}"), b.to_string()))).collect();
        for (k, v) in &specs {
            env.push((k.as_str(), v.clone()));
        }
        let o = run(&pcm, &inp, &env).unwrap();
        let hooks = parse_hooks(&std::fs::read_to_string(&hk).unwrap_or_default());
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("oracle: {}", o.stdout.lines().take(2).collect::<Vec<_>>().join(" | "));
        let mono = mono_mix(&l, &r);
        let dec = stage_a(&mono, inp.sr);
        let mine: std::collections::HashMap<&str, Vec<f64>> = [("s480", dec.clone())].into_iter().collect();
        for (name, arrays) in &hooks {
            for (k, v) in arrays {
                if let Some(m) = mine.get(k.as_str()) {
                    match first_diff(m, v) {
                        None => eprintln!("{name} {k}: equal ({} values)", v.len()),
                        Some(i) => eprintln!("{name} {k}: len {}/{} first diff at {i}: {:?} vs {:?}", m.len(), v.len(), m.get(i), v.get(i)),
                    }
                }
            }
        }
    }

    /// Parse the harness's Sub_Main_D call dump (RB_DTRACE).
    pub fn parse_dtrace(text: &str) -> Vec<(String, std::collections::HashMap<usize, Vec<f64>>)> {
        let mut out: Vec<(String, std::collections::HashMap<usize, Vec<f64>>)> = vec![];
        for l in text.lines() {
            if let Some(h) = l.strip_prefix("# D ") {
                out.push((h.split_whitespace().next().unwrap_or("").to_string(), Default::default()));
                continue;
            }
            let v: Vec<&str> = l.split_whitespace().collect();
            if v.len() < 2 || !v[0].starts_with('a') {
                continue;
            }
            let Ok(k) = v[0][1..].parse::<usize>() else { continue };
            let skip = match v[1] {
                "F" | "I" | "U" => 3,
                "G" | "J" | "H" => 4,
                _ => 2,
            };
            let vals: Vec<f64> = v.get(skip..).unwrap_or(&[]).iter().map(|x| x.parse().unwrap_or(f64::NAN)).collect();
            if let Some(last) = out.last_mut() {
                last.1.insert(k, vals);
            }
        }
        out
    }

    /// Stage D call-by-call comparison (port vs harness dump) on PREPDECK_LIMIT tracks.
    #[test]
    #[ignore]
    fn phrase_d_compare() {
        let rows = tracks();
        let lim: usize = std::env::var("PREPDECK_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
        let want = std::env::var("PREPDECK_TRACK").ok();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let res = std::sync::Mutex::new(vec![]);
        let sel: Vec<usize> = (0..rows.len()).filter(|i| want.as_ref().map(|w| rows[*i].audio.contains(w.as_str())).unwrap_or(true)).take(lim).collect();
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| loop {
                    let j = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&i) = sel.get(j) else { break };
                    let t = &rows[i];
                    let Ok(d) = crate::audio::decode_file(&t.audio) else { continue };
                    let Some((bpm100, first_ms, first_bar)) = grid_input(&t.beats) else { continue };
                    let (l, r) = s16_channels(&d.samples);
                    let inp = Input { sr: d.sr as i32, total: l.len() as i32, bpm100, first_ms, first_bar, key: 0 };
                    let dir = tmp_dir(&format!("d{i}"));
                    let pcm = dir.join("pcm.s16");
                    let dt = dir.join("dtrace.txt");
                    let out = write_s16(&pcm, &l, &r).ok().and_then(|_| run(&pcm, &inp, &[("RB_DTRACE", dt.to_string_lossy().to_string()), ("RB_DCAP", "100000000".to_string())]));
                    let theirs = parse_dtrace(&std::fs::read_to_string(&dt).unwrap_or_default());
                    if let Ok(k) = std::env::var("PREPDECK_KEEPDT") {
                        let _ = std::fs::copy(&dt, format!("{k}.{i}"));
                    }
                    let _ = std::fs::remove_dir_all(&dir);
                    if std::env::var("PREPDECK_NAMES").is_ok() {
                        let mut seq: Vec<(String, usize)> = vec![];
                        for (n, _) in &theirs {
                            match seq.last_mut() {
                                Some(l) if l.0 == *n => l.1 += 1,
                                _ => seq.push((n.to_string(), 1)),
                            }
                        }
                        let s: Vec<String> = seq.iter().map(|(n, c)| if *c > 1 { format!("{n}x{c}") } else { n.clone() }).collect();
                        eprintln!("NAMES {}: {}", t.audio.rsplit('/').next().unwrap_or(""), s.join(" "));
                    }
                    let mono = mono_mix(&l, &r);
                    let dec = stage_a(&mono, inp.sr);
                    let mut msg = vec![];
                    if let Some(b) = crate::phrase_rb_b::sub_main_b(&dec, inp.sr as f64 * 0.125, bpm100 as f64 * 0.01, first_ms, first_bar) {
                        if let Some(c) = stage_c(&mono, &inp, &b) {
                            let din = crate::phrase_rb_d::DIn { c: &c, b: &b, bpm: bpm100 as f64 * 0.01, sr: inp.sr as f64, sr8: inp.sr as f64 * 0.125, total: inp.total, ms_in: first_ms, key: 0 };
                            let mut tr = crate::phrase_rb_d::Trace { on: true, calls: vec![] };
                            let _ = crate::phrase_rb_d::sub_main_d(&din, &mut tr);
                            let mut ti = 0usize;
                            for call in &tr.calls {
                                while ti < theirs.len() && theirs[ti].0 != call.name {
                                    ti += 1;
                                }
                                let Some((_, th)) = theirs.get(ti) else {
                                    msg.push(format!("{}: not in the reference's calls", call.name));
                                    break;
                                };
                                for (k, mine) in &call.args {
                                    let tv = th.get(k).cloned().unwrap_or_default();
                                    if let Some(p) = first_diff(mine, &tv) {
                                        msg.push(format!("{} a{k}: len {}/{} first diff at {p}: {:?} vs {:?}", call.name, mine.len(), tv.len(), mine.get(p), tv.get(p)));
                                    }
                                }
                                ti += 1;
                                if !msg.is_empty() {
                                    break;
                                }
                            }
                            if msg.is_empty() {
                                msg.push(format!("ok through {} calls ({})", tr.calls.len(), tr.calls.last().map(|c| c.name).unwrap_or("")));
                            }
                        } else {
                            msg.push("port stage C none".into());
                        }
                    }
                    let _ = out;
                    res.lock().unwrap().push((i, msg));
                });
            }
        });
        let mut r = res.into_inner().unwrap();
        r.sort_by_key(|x| x.0);
        let mut ok = 0;
        for (i, m) in &r {
            if m.len() == 1 && m[0].starts_with("ok") {
                ok += 1;
                if std::env::var("PREPDECK_VERBOSE").is_ok() {
                    eprintln!("OK {}: {}", rows[*i].audio.rsplit('/').next().unwrap_or(""), m[0]);
                }
            } else {
                eprintln!("DIFF {}\n  {}", rows[*i].audio.rsplit('/').next().unwrap_or(""), m.join("\n  "));
            }
        }
        eprintln!("stage D calls: {ok}/{} equal", r.len());
    }
}
