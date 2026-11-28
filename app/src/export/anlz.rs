//! Writer for the players' analysis files (ANLZ0000.DAT / .EXT / .2EX), big-endian tagged
//! sections inside a PMAI container. Section layouts are listed in MECHANICS.md.

pub(crate) mod grid_pair;

use crate::model::{Beat, Cue, CueKind, Waveform};

fn section(tag: &[u8; 4], header_extra: &[u8], body: &[u8]) -> Vec<u8> {
    let hl = 12 + header_extra.len();
    let mut v = Vec::with_capacity(hl + body.len());
    v.extend_from_slice(tag);
    v.extend_from_slice(&(hl as u32).to_be_bytes());
    v.extend_from_slice(&((hl + body.len()) as u32).to_be_bytes());
    v.extend_from_slice(header_extra);
    v.extend_from_slice(body);
    v
}

fn file(sections: Vec<Vec<u8>>) -> Vec<u8> {
    let total: usize = 28 + sections.iter().map(|s| s.len()).sum::<usize>();
    let mut v = Vec::with_capacity(total);
    v.extend_from_slice(b"PMAI");
    v.extend_from_slice(&28u32.to_be_bytes());
    v.extend_from_slice(&(total as u32).to_be_bytes());
    v.extend_from_slice(&[0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for s in sections {
        v.extend_from_slice(&s);
    }
    v
}

fn ppth(path: &str) -> Vec<u8> {
    let mut body = vec![];
    for u in path.encode_utf16() {
        body.extend_from_slice(&u.to_be_bytes());
    }
    body.extend_from_slice(&[0, 0]);
    section(b"PPTH", &(body.len() as u32).to_be_bytes(), &body)
}

/// The path in a PPTH section: header 16 bytes with the byte length of the UTF-16BE path
/// (including its terminating NUL) at 12. None when the lengths disagree or the text is not
/// NUL-terminated UTF-16.
pub fn parse_ppth(sec: &[u8]) -> Option<String> {
    if sec.get(..4)? != b"PPTH" {
        return None;
    }
    let hl = u32::from_be_bytes(sec.get(4..8)?.try_into().ok()?) as usize;
    let total = u32::from_be_bytes(sec.get(8..12)?.try_into().ok()?) as usize;
    let n = u32::from_be_bytes(sec.get(12..16)?.try_into().ok()?) as usize;
    if hl != 16 || total != hl + n || n < 2 || n % 2 != 0 || total > sec.len() {
        return None;
    }
    let units: Vec<u16> = sec[16..16 + n].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    if *units.last()? != 0 {
        return None;
    }
    String::from_utf16(&units[..units.len() - 1]).ok()
}

/// Seek index: 400 zero entries followed by the total sample count of the file's full decode
/// (observed in the original's files for 108 local tracks).
fn pvbr(raw_samples: u64) -> Vec<u8> {
    let mut body = vec![0u8; 1600];
    body.extend_from_slice(&(raw_samples.min(u32::MAX as u64) as u32).to_be_bytes());
    section(b"PVBR", &0u32.to_be_bytes(), &body)
}

/// Linear models from this app's band levels to each waveform channel of the original's
/// files, fitted on 70 local tracks against the original's own analysis files.
/// Features: [1, peak full, low, mid, high, rms full, low, mid, high, sqrt peak full, low, mid, high].
mod model {
    pub const PWAV_HEIGHT: [f64; 13] = [3.03135, 17.12007, 0.69868, -5.55112, -5.75931, 57.90399, -25.85376, -10.60182, -3.10607, -35.62296, 16.23435, 14.38976, 7.32306];
    pub const PWAV_WHITE: [f64; 13] = [4.50570, -5.29187, 2.72777, 0.91879, -1.11088, -8.11968, 6.93162, 4.49034, 7.36645, 5.90496, -6.36616, -3.26908, 4.61879];
    pub const PWV2_HEIGHT: [f64; 13] = [2.25964, 19.70139, -22.61358, -6.68805, -1.51015, 8.95450, 1.41428, -13.41762, 0.47900, -35.83856, 36.88592, 17.87151, 2.04287];
    pub const PWV3_HEIGHT: [f64; 13] = [-2.16261, 24.90020, 12.05697, -7.79336, -32.09341, -7.40268, 9.70756, -26.52655, 45.37813, -15.67638, 1.71053, 16.95374, 17.63361];
    pub const PWV3_WHITE: [f64; 13] = [5.30439, 2.50468, 8.96202, -8.88094, -9.63281, -0.52760, -0.90728, 9.84407, 9.62981, -4.38970, -14.52101, 9.12481, 7.21387];
    pub const PWV4: [[f64; 13]; 6] = [
        [1.37612, 108.77636, 2.98012, 12.80932, -0.12571, 44.38642, -36.63809, -10.86371, 17.72992, 4.45720, 7.78352, -17.88567, 4.00188],
        [66.04648, -435.79297, 17.38686, -11.55472, 21.64486, -61.89455, 51.75436, 23.46968, 13.76904, 511.92365, -26.69181, 22.22247, -32.06844],
        [3.77519, 57.17777, 22.96768, 12.12057, 1.79882, 74.79551, -75.22735, 21.32916, -76.46239, -44.99123, 89.19619, -9.27890, -8.95646],
        [-0.59613, -21.73611, 119.44813, 21.94283, -0.91354, -22.46012, 26.58133, 0.21008, 2.54063, 32.66253, 7.76133, -38.23688, 4.44855],
        [1.98613, 13.34314, -9.12154, 66.09742, -6.18194, 139.45423, -139.31893, 26.29133, -142.39120, -27.92399, 23.61765, -1.07809, 6.25276],
        [1.96826, -49.01155, -4.50476, 17.22408, 166.87556, -58.47098, 59.95905, 29.62783, 129.29758, 86.21188, 3.98830, -13.04481, -134.28980],
    ];
    pub const PWV5: [[f64; 13]; 4] = [
        [-2.40537, 22.14186, 13.27247, -6.16213, -32.03175, -8.25699, 9.58504, -27.28274, 46.47147, -12.96487, 1.54238, 15.74140, 17.55403],
        [2.50192, 5.33513, -15.84096, 3.56750, 5.08640, -0.20227, 0.13397, -6.98770, -10.73829, -6.80494, 23.60747, -4.83515, -2.24034],
        [2.60047, 6.78224, -0.47220, -15.08857, -3.84492, 0.43141, 1.17405, 2.13511, -0.28079, -13.11151, -2.89172, 22.17049, 2.34895],
        [5.69052, 10.57925, 2.35099, -8.12937, -22.91164, -0.50346, 0.23687, 1.55529, 15.62641, -18.11316, -5.04928, 10.01469, 21.28717],
    ];
    pub const PWV6: [[f64; 13]; 3] = [
        [-0.44578, -8.51963, 4.62029, 6.83503, -1.50459, -100.18567, 152.21663, 92.37922, 57.61135, 13.34355, 7.55685, -12.06794, 3.89098],
        [-2.76985, -0.20643, -4.97277, -24.33844, -8.22292, -43.46421, 39.34981, 209.74523, 85.15138, 3.46844, 2.66735, 25.32570, 4.29872],
        [-2.07634, 12.06332, 1.11387, -5.75550, -43.41470, -24.98885, 21.18503, 40.04945, 344.46849, -17.04930, -2.05763, 6.66002, 48.77071],
    ];
    pub const PWV7: [[f64; 13]; 3] = [
        [7.36570, -5.74102, -23.95699, 35.25720, 10.71000, -4.59174, 7.14800, -90.60947, 0.72244, 35.45054, 91.57040, -28.34832, -19.11846],
        [-4.88456, 34.76648, 6.80177, -101.04472, -71.85855, 6.05436, 23.10675, -3.74485, 46.64680, -64.17889, -2.05040, 166.34076, 40.62119],
        [-4.81462, 30.39690, 8.48572, -37.01762, -128.22948, -1.55879, 7.55559, -12.14972, 274.29239, -55.50643, 5.13877, 38.86642, 100.70658],
    ];
}

/// rekordbox-style 3-band detail values (low, mid, high; PWV7 scale) for one frame or column.
pub fn three_band_detail(c: &[u8; 8]) -> [u8; 3] {
    let f = feats(c);
    [apply(&model::PWV7[0], &f, 255) as u8, apply(&model::PWV7[1], &f, 255) as u8, apply(&model::PWV7[2], &f, 255) as u8]
}

/// rekordbox-style 3-band overview values (low, mid, high; PWV6 scale) for one column.
pub fn three_band_overview(c: &[u8; 8]) -> [u8; 3] {
    let f = feats(c);
    [apply(&model::PWV6[0], &f, 255) as u8, apply(&model::PWV6[1], &f, 255) as u8, apply(&model::PWV6[2], &f, 255) as u8]
}

fn feats(c: &[u8; 8]) -> [f64; 13] {
    let p = |k: usize| c[k] as f64 / 255.0;
    let r = |k: usize| c[4 + k] as f64 / 510.0;
    [1.0, p(0), p(1), p(2), p(3), r(0), r(1), r(2), r(3), p(0).sqrt(), p(1).sqrt(), p(2).sqrt(), p(3).sqrt()]
}

fn apply(w: &[f64; 13], f: &[f64; 13], max: u32) -> u32 {
    let v: f64 = w.iter().zip(f).map(|(a, b)| a * b).sum();
    v.round().clamp(0.0, max as f64) as u32
}

/// Frames padded (with silence) to the count the original uses: ceil(raw samples / hop).
fn padded_frames(w: &Waveform, raw_samples: u64, sample_rate: u32) -> Vec<[u8; 8]> {
    let mut v = w.frames.clone();
    if raw_samples > 0 && sample_rate > 0 {
        let hop = sample_rate as f64 / crate::model::WAVE_FPS;
        let n = (raw_samples as f64 / hop).ceil() as usize;
        v.resize(n.max(v.len().min(n)), [0u8; 8]);
        v.truncate(n);
    }
    v
}

/// Beat number, centi-BPM, and whole milliseconds as written in classic USB analysis files.
pub(crate) fn beat_record(b: &Beat) -> (u16, u16, u32) {
    (u16::from(b.beat), (b.bpm * 100.0).round() as u16, ms(b.time))
}

fn pqtz(beats: &[Beat]) -> Vec<u8> {
    let mut h = vec![];
    h.extend_from_slice(&0u32.to_be_bytes());
    h.extend_from_slice(&0x0008_0000u32.to_be_bytes());
    h.extend_from_slice(&(beats.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(beats.len() * 8);
    for b in beats {
        let (bar, tempo, whole_ms) = beat_record(b);
        body.extend_from_slice(&bar.to_be_bytes());
        body.extend_from_slice(&tempo.to_be_bytes());
        body.extend_from_slice(&whole_ms.to_be_bytes());
    }
    section(b"PQTZ", &h, &body)
}

/// The (beat number, tempo × 100, milliseconds) entries of a PQTZ section.
pub fn pqtz_entries(sec: &[u8]) -> Vec<(u16, u16, u32)> {
    if sec.len() < 24 || &sec[..4] != b"PQTZ" {
        return vec![];
    }
    let n = u32::from_be_bytes(sec[20..24].try_into().unwrap()) as usize;
    (0..n)
        .filter_map(|i| sec.get(24 + 8 * i..32 + 8 * i))
        .map(|e| (u16::from_be_bytes([e[0], e[1]]), u16::from_be_bytes([e[2], e[3]]), u32::from_be_bytes([e[4], e[5], e[6], e[7]])))
        .collect()
}

/// Extended beat grid (PQT2, in the .EXT file), derived from the PQTZ entries of the same
/// track. Layout established from 102 real files (rekordcrate's demo export and 100 local
/// rekordbox 7 analysis files), every one of which this function reproduces byte for byte
/// from its own PQTZ entries and fractions:
///
/// | offset | content |
/// |---|---|
/// | 12 | u32 0 |
/// | 16 | u32 0x01000002 |
/// | 20 | u32 0 |
/// | 24 | the first PQTZ entry (beat u16, tempo u16, ms u32) |
/// | 32 | the last PQTZ entry |
/// | 40 | u32 number of beats |
/// | 44 | u32 sum over all PQTZ entries of beat + tempo + ms |
/// | 48 | 8 zero bytes |
/// | 56 | one u16 per beat: the beat time's microseconds within its millisecond (0–999) |
///
/// `fractions` holds that last value per beat (missing ones are written as 0). Nothing in the
/// section is left underived; files with several tempos were not among the real samples, so
/// that the two header entries stay "first" and "last" there is an assumption.
pub fn pqt2(entries: &[(u16, u16, u32)], fractions: &[u16]) -> Vec<u8> {
    let mut h = vec![];
    h.extend_from_slice(&0u32.to_be_bytes());
    h.extend_from_slice(&0x0100_0002u32.to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    for e in [entries.first(), entries.last()] {
        let (beat, tempo, time) = e.copied().unwrap_or((0, 0, 0));
        h.extend_from_slice(&beat.to_be_bytes());
        h.extend_from_slice(&tempo.to_be_bytes());
        h.extend_from_slice(&time.to_be_bytes());
    }
    h.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    let sum = entries.iter().fold(0u32, |a, e| a.wrapping_add(e.0 as u32).wrapping_add(e.1 as u32).wrapping_add(e.2));
    h.extend_from_slice(&sum.to_be_bytes());
    h.extend_from_slice(&[0u8; 8]);
    let mut body = Vec::with_capacity(entries.len() * 2);
    for i in 0..entries.len() {
        body.extend_from_slice(&fractions.get(i).copied().unwrap_or(0).min(999).to_be_bytes());
    }
    section(b"PQT2", &h, &body)
}

/// The per-beat microsecond fractions stored in a PQT2 section.
pub fn pqt2_fractions(sec: &[u8]) -> Vec<u16> {
    if sec.len() < 56 || &sec[..4] != b"PQT2" {
        return vec![];
    }
    sec[56..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect()
}

fn pwav(w: &Waveform) -> Vec<u8> {
    let cols = w.columns(400);
    let body: Vec<u8> = cols
        .iter()
        .map(|c| {
            let f = feats(c);
            let h = apply(&model::PWAV_HEIGHT, &f, 31);
            let white = apply(&model::PWAV_WHITE, &f, 7);
            (h | (white << 5)) as u8
        })
        .collect();
    let mut h = vec![];
    h.extend_from_slice(&400u32.to_be_bytes());
    h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    section(b"PWAV", &h, &body)
}

fn pwv2(w: &Waveform) -> Vec<u8> {
    let body: Vec<u8> = w.columns(100).iter().map(|c| apply(&model::PWV2_HEIGHT, &feats(c), 15) as u8).collect();
    let mut h = vec![];
    h.extend_from_slice(&100u32.to_be_bytes());
    h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    section(b"PWV2", &h, &body)
}

fn pwv3(frames: &[[u8; 8]]) -> Vec<u8> {
    let body: Vec<u8> = frames
        .iter()
        .map(|c| {
            let f = feats(c);
            let h = apply(&model::PWV3_HEIGHT, &f, 31);
            let white = apply(&model::PWV3_WHITE, &f, 7);
            (h | (white << 5)) as u8
        })
        .collect();
    let mut h = vec![];
    h.extend_from_slice(&1u32.to_be_bytes());
    h.extend_from_slice(&(body.len() as u32).to_be_bytes());
    h.extend_from_slice(&0x0096_0000u32.to_be_bytes());
    section(b"PWV3", &h, &body)
}

fn pwv4(w: &Waveform) -> Vec<u8> {
    let cols = w.columns(1200);
    let mut body = Vec::with_capacity(1200 * 6);
    for c in &cols {
        let f = feats(c);
        for m in &model::PWV4 {
            body.push(apply(m, &f, 255) as u8);
        }
    }
    let mut h = vec![];
    h.extend_from_slice(&6u32.to_be_bytes());
    h.extend_from_slice(&1200u32.to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    section(b"PWV4", &h, &body)
}

fn pwv5(frames: &[[u8; 8]]) -> Vec<u8> {
    let mut body = Vec::with_capacity(frames.len() * 2);
    for c in frames {
        let f = feats(c);
        let h = apply(&model::PWV5[0], &f, 31);
        let r = apply(&model::PWV5[1], &f, 7);
        let g = apply(&model::PWV5[2], &f, 7);
        let b = apply(&model::PWV5[3], &f, 7);
        let v = ((r << 13) | (g << 10) | (b << 7) | (h << 2)) as u16;
        body.extend_from_slice(&v.to_be_bytes());
    }
    let mut h = vec![];
    h.extend_from_slice(&2u32.to_be_bytes());
    h.extend_from_slice(&(frames.len() as u32).to_be_bytes());
    h.extend_from_slice(&0x0096_0305u32.to_be_bytes());
    section(b"PWV5", &h, &body)
}

fn pwv6(w: &Waveform) -> Vec<u8> {
    let mut body = Vec::with_capacity(1200 * 3);
    for c in w.columns(1200) {
        let f = feats(&c);
        for m in &model::PWV6 {
            body.push(apply(m, &f, 255) as u8);
        }
    }
    let mut h = vec![];
    h.extend_from_slice(&3u32.to_be_bytes());
    h.extend_from_slice(&1200u32.to_be_bytes());
    section(b"PWV6", &h, &body)
}

fn pwv7(frames: &[[u8; 8]]) -> Vec<u8> {
    let mut body = Vec::with_capacity(frames.len() * 3);
    for c in frames {
        let f = feats(c);
        for m in &model::PWV7 {
            body.push(apply(m, &f, 255) as u8);
        }
    }
    let mut h = vec![];
    h.extend_from_slice(&3u32.to_be_bytes());
    h.extend_from_slice(&(frames.len() as u32).to_be_bytes());
    h.extend_from_slice(&0x0096_0000u32.to_be_bytes());
    section(b"PWV7", &h, &body)
}

fn pwvc() -> Vec<u8> {
    // Per-band display calibration; meaning not established, values from a real export.
    section(b"PWVC", &[0, 0], &[0x00, 0x5b, 0x00, 0x8a, 0x01, 0x8c])
}

/// Whole milliseconds of a time, with the time first rounded to the microsecond (so 1.9 s is
/// 1900, not 1899) and the rest cut off; [`micros`] is the part cut.
fn ms(t: f64) -> u32 {
    ((t.max(0.0) * 1e6).round() as u64 / 1000).min(u32::MAX as u64) as u32
}

/// Microseconds within the millisecond (0–999).
fn micros(t: f64) -> u16 {
    ((t.max(0.0) * 1e6).round() as u64 % 1000) as u16
}

/// A loop's length as a rekordbox beat-loop size (numerator, denominator), when it is one of
/// the sizes the players and rekordbox offer (1/64 … 512 beats) at the grid's tempo there, to
/// within a millisecond (and 2 % for the shortest). None for any other length: rekordbox
/// writes 0/0 unless both parts are set.
pub fn beat_loop_size(start: f64, end: f64, beats: &[Beat]) -> Option<(u16, u16)> {
    if beats.is_empty() || end <= start {
        return None;
    }
    let near = beats.iter().min_by(|a, b| (a.time - start).abs().partial_cmp(&(b.time - start).abs()).unwrap())?;
    if near.bpm <= 0.0 {
        return None;
    }
    let beat = 60.0 / near.bpm;
    let len = end - start;
    for p in -6i32..=9 {
        let size = 2f64.powi(p) * beat;
        if (len - size).abs() <= 0.0011_f64.min(size * 0.02).max(0.0002) {
            return Some(if p >= 0 { (1 << p, 1) } else { (1, 1 << -p) });
        }
    }
    None
}

/// Legacy cue list (PCOB with PCPT entries).
fn pcob(hot: bool, cues: &[&Cue]) -> Vec<u8> {
    let mut body = vec![];
    let n = cues.len();
    for (i, c) in cues.iter().enumerate() {
        let mut e = vec![];
        e.extend_from_slice(&(if hot { (c.slot + 1) as u32 } else { 0 }).to_be_bytes());
        e.extend_from_slice(&(if c.active_loop { 4u32 } else { 0 }).to_be_bytes());
        // Offsets 20/22 are 1/0 (RF-07). Offsets 24/26 link memory records to their
        // neighbours in destination order; hot cues keep 0xffff in both.
        e.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        let (prev, next) = if hot { (0xffff, 0xffff) } else { (if i == 0 { 0xffff } else { (i - 1) as u16 }, if i + 1 == n { 0xffff } else { (i + 1) as u16 }) };
        e.extend_from_slice(&prev.to_be_bytes());
        e.extend_from_slice(&next.to_be_bytes());
        let mut rest = vec![];
        rest.push(if c.is_loop() { 2 } else { 1 });
        rest.push(0);
        rest.extend_from_slice(&1000u16.to_be_bytes());
        rest.extend_from_slice(&ms(c.start).to_be_bytes());
        rest.extend_from_slice(&(if c.is_loop() { ms(c.end.unwrap()) } else { 0xffff_ffff }).to_be_bytes());
        rest.extend_from_slice(&[0u8; 16]);
        body.extend_from_slice(&section(b"PCPT", &e, &rest));
    }
    let mut h = vec![];
    h.extend_from_slice(&(hot as u32).to_be_bytes());
    h.extend_from_slice(&0u16.to_be_bytes());
    h.extend_from_slice(&(n as u16).to_be_bytes());
    h.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
    section(b"PCOB", &h, &body)
}

/// Extended cue list (PCO2 with PCP2 entries) carrying colours and comments.
///
/// PCP2 record (88 bytes + comment), per rekordcrate and rekordbox-rea's
/// cue-export-projection report: 12 slot, 16 type, 18 the constant 1000, 20 start ms, 24 end ms,
/// 28 memory-cue colour, 29 the constant 1, 30/32 the microseconds within the millisecond of
/// start and end (each 0–999), 34 zero, 36/38 beat-loop size numerator/denominator (both set
/// or both zero), 40 comment byte count, then the comment, palette index + RGB, and 40 bytes
/// of seek data (for each end: sample position and byte offset of the audio frame to seek to
/// and its length). rekordbox copies the seek data only when its own decoder recorded it; it
/// cannot be derived without that decoder, so it is written as zeros, which is also what
/// rekordbox writes for a cue without it. The one real record available (rekordcrate's
/// test, a cue at 287,479 ms) carries sample position 0xc17000, byte offset 0x2307761 and
/// frame length 0x1000 there.
fn pco2(hot: bool, cues: &[&Cue], beats: &[Beat]) -> Vec<u8> {
    let mut body = vec![];
    for c in cues {
        let mut e = vec![];
        e.extend_from_slice(&(if hot { (c.slot + 1) as u32 } else { 0 }).to_be_bytes());
        let mut r = vec![];
        r.push(if c.is_loop() { 2 } else { 1 });
        r.push(0);
        r.extend_from_slice(&1000u16.to_be_bytes());
        r.extend_from_slice(&ms(c.start).to_be_bytes());
        r.extend_from_slice(&(if c.is_loop() { ms(c.end.unwrap()) } else { 0xffff_ffff }).to_be_bytes());
        // Memory cue colour (track colour ids 0..8); hot cues use the fields further on.
        // Byte 29 is always 1 in the original's converter.
        r.push(if hot { 0 } else { c.color });
        r.push(1);
        r.extend_from_slice(&micros(c.start).to_be_bytes());
        r.extend_from_slice(&(if c.is_loop() { micros(c.end.unwrap()) } else { 0 }).to_be_bytes());
        r.extend_from_slice(&0u16.to_be_bytes());
        let (num, den) = if c.is_loop() { beat_loop_size(c.start, c.end.unwrap(), beats).unwrap_or((0, 0)) } else { (0, 0) };
        r.extend_from_slice(&num.to_be_bytes());
        r.extend_from_slice(&den.to_be_bytes());
        let mut cm: Vec<u8> = vec![];
        if !c.comment.is_empty() {
            for u in c.comment.encode_utf16() {
                cm.extend_from_slice(&u.to_be_bytes());
            }
            cm.extend_from_slice(&[0, 0]);
        }
        r.extend_from_slice(&(cm.len() as u32).to_be_bytes());
        r.extend_from_slice(&cm);
        if hot {
            // Explicit palette indices 1–64 keep their index; RGB comes from the device palette.
            let idx = if (1..=64).contains(&c.color) { c.color } else { crate::model::hotcue_default_color(c.slot, c.is_loop()) };
            let rgb = crate::model::device_rgb(idx);
            r.push(idx);
            r.extend_from_slice(&rgb);
        } else {
            r.extend_from_slice(&[0, 0, 0, 0]);
        }
        // Seek data (four u64, two u32): not derivable here, zero as for a cue without it.
        r.extend_from_slice(&[0u8; 40]);
        body.extend_from_slice(&section(b"PCP2", &e, &r));
    }
    let mut h = vec![];
    h.extend_from_slice(&(hot as u32).to_be_bytes());
    h.extend_from_slice(&(cues.len() as u16).to_be_bytes());
    h.extend_from_slice(&0u16.to_be_bytes());
    section(b"PCO2", &h, &body)
}

pub struct AnlzFiles {
    pub dat: Vec<u8>,
    pub ext: Vec<u8>,
    pub ex2: Vec<u8>,
}

/// Build the three analysis files. `usb_path` is the media path on the drive
/// (e.g. "/Contents/Artist/Album/file.mp3"). Hot cues beyond H are not exported.
/// `raw_samples`/`sample_rate`: the file's full decoded length, which the original uses for the
/// seek index total and the number of detail waveform frames (0 = unknown, use the waveform).
pub fn build(usb_path: &str, beats: &[Beat], wave: &Waveform, cues: &[Cue], raw_samples: u64, sample_rate: u32) -> AnlzFiles {
    build_with(usb_path, beats, wave, cues, raw_samples, sample_rate, None)
}

/// Sections taken from rekordbox's own analysis of the same audio file (for tracks synced from
/// the rekordbox library). Waveforms, the VBR seek index and vocal detection always apply; the
/// beat grid, extended grid (PQT2) and phrases (PSSI) only when `grid_same` (the track's grid
/// is still the one rekordbox analysed, since phrases are counted in its beats).
pub struct Overrides {
    pub sections: std::collections::HashMap<String, Vec<u8>>,
    pub grid_same: bool,
    /// Recordbox's own phrase section (unmasked), used when rekordbox's can't be.
    pub pssi: Option<Vec<u8>>,
    /// The track's analysis files already on the drive, by extension ("DAT", "EXT", "2EX"), in
    /// file order. Audio-derived sections fall back to these, and sections Recordbox does not
    /// know are carried over unchanged.
    pub device: std::collections::HashMap<&'static str, Vec<(String, Vec<u8>)>>,
    /// Recordbox's own vocal section, used when rekordbox's is not available.
    pub pvdi: Option<Vec<u8>>,
}

/// Vocal detection section (2EX): discriminator 0, hop 1024 at 22,050 Hz, stride 1, then one
/// level 0–4 per hop, as rekordbox writes it unscrambled.
pub fn pvdi(levels: &[u8]) -> Vec<u8> {
    let mut h = vec![];
    for w in [0u16, 1024, 22050, 1] {
        h.extend_from_slice(&w.to_be_bytes());
    }
    h.extend_from_slice(&(levels.len() as u32).to_be_bytes());
    section(b"PVDI", &h, levels)
}

// The 7.2.19 USB writer uses this mask for the level payload of every track.
// It starts five characters into the key and repeats after nineteen bytes.
fn vocal_mask(i: usize) -> u8 {
    const KEY: &[u8; 19] = b"Kanzen-niRikaishita";
    KEY[(i % KEY.len() + 5) % KEY.len()].wrapping_add(b'0') ^ 0x80
}

/// Recognize the measured fixed-hop mono layout. An unknown revision, extended
/// header or differently shaped payload remains opaque when refreshing a device.
fn vocal_payload(section: &[u8]) -> Option<(bool, &[u8])> {
    if section.get(..4)? != b"PVDI" || section.get(4..8)? != 24u32.to_be_bytes()
        || u32::from_be_bytes(section.get(8..12)?.try_into().ok()?) as usize != section.len()
        || section.get(14..20)? != [0x04, 0, 0x56, 0x22, 0, 1] {
        return None;
    }
    let encoded = match u16::from_be_bytes(section.get(12..14)?.try_into().ok()?) {
        0 => false,
        0x8000 => true,
        _ => return None,
    };
    let count = u32::from_be_bytes(section.get(20..24)?.try_into().ok()?) as usize;
    let levels = section.get(24..)?;
    if count != levels.len() || levels.iter().enumerate().any(|(i, &byte)| {
        (if encoded { byte ^ vocal_mask(i) } else { byte }) > 4
    }) {
        return None;
    }
    Some((encoded, levels))
}

pub(crate) fn parse_pvdi(section: &[u8]) -> Option<std::sync::Arc<(f64, Vec<u8>)>> {
    let (encoded, levels) = vocal_payload(section)?;
    let mut levels = levels.to_vec();
    if encoded {
        for (i, level) in levels.iter_mut().enumerate() { *level ^= vocal_mask(i); }
    }
    Some(std::sync::Arc::new((1024.0 / 22_050.0, levels)))
}

fn append_device_vocals(ex2: &mut Vec<Vec<u8>>, ov: &Overrides) {
    let mut existing = ov.device.get("2EX").into_iter().flatten()
        .filter(|(tag, _)| tag == "PVDI").map(|(_, section)| section);
    let first = existing.next();
    // A later format can add fields or use more than one section. Carry the
    // complete original bytes in their order instead of substituting local data.
    if existing.clone().next().is_some() || first.is_some_and(|s| vocal_payload(s).is_none()) {
        ex2.extend(first.into_iter().chain(existing).cloned());
        return;
    }
    // Only native sections carry opaque extensions worth preserving. Emit a
    // newly synthesized section only when it contains valid measured levels.
    if let Some(source) = ov.sections.get("PVDI").or(first)
        .or_else(|| ov.pvdi.as_ref().filter(|s| vocal_payload(s).is_some())) {
        let mut section = source.clone();
        if matches!(vocal_payload(source), Some((false, _))) {
            for (i, b) in section[24..].iter_mut().enumerate() { *b ^= vocal_mask(i); }
            section[12..14].copy_from_slice(&0x8000u16.to_be_bytes());
        }
        ex2.push(section);
    }
}

/// Section tags Recordbox writes itself (or deliberately drops when the grid changed).
const KNOWN_TAGS: [&str; 16] = ["PPTH", "PVBR", "PQTZ", "PWAV", "PWV2", "PCOB", "PWV3", "PCO2", "PQT2", "PWV5", "PWV4", "PSSI", "PWV7", "PWV6", "PWVC", "PVDI"];
/// Sections that depend only on the audio, so a copy already on the drive stays valid.
const AUDIO_TAGS: [&str; 9] = ["PVBR", "PWAV", "PWV2", "PWV3", "PWV4", "PWV5", "PWV6", "PWV7", "PWVC"];

/// Every top-level section of an ANLZ file in order, including repeated tags.
pub fn read_section_list(path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    std::fs::read(path).map(|b| section_list(&b)).unwrap_or_default()
}

/// Sections of a file on a drive, only when its framing is valid (a damaged file's pieces are
/// not carried into a new one).
pub fn read_valid_section_list(path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    match std::fs::read(path) {
        Ok(b) if validate(&b).is_ok() => section_list(&b),
        _ => vec![],
    }
}

/// A USB catalog's analysis reference and the PPTH stored inside each classic file
/// must identify the same device track before its cues or grid can be reused.
pub(super) fn device_companion(path: &std::path::Path, media: &str) -> anyhow::Result<Option<Vec<u8>>> {
    use anyhow::{ensure, Context};
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("Inspecting {}", path.display())),
    };
    ensure!(metadata.is_file(), "Expected a regular analysis file at {}", path.display());
    let bytes = std::fs::read(path)?;
    // Native .3EX embeddings have a different encoding. Preserve their bytes along
    // with their verified DAT. If a future .3EX is PMAI, check its path as well.
    if !bytes.starts_with(b"PMAI") && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("3EX")) {
        return Ok(Some(bytes));
    }
    validate(&bytes).map_err(anyhow::Error::msg).with_context(|| format!("Invalid analysis file {}", path.display()))?;
    let mut offset = 28;
    let mut matched = false;
    while offset < bytes.len() {
        // validate() checked the complete section directory and its bounds.
        let length = u32::from_be_bytes(bytes[offset + 8..offset + 12].try_into()?) as usize;
        if &bytes[offset..offset + 4] == b"PPTH" {
            let recorded = parse_ppth(&bytes[offset..offset + length]).context("Unreadable USB analysis track path")?;
            ensure!(!matched && super::usb::path_key(&recorded) == super::usb::path_key(media),
                "{} has a conflicting track path ({recorded}). The USB catalog references {media}", path.display());
            matched = true;
        }
        offset += length;
    }
    ensure!(matched, "{} lacks the USB track path required to match {media}", path.display());
    Ok(Some(bytes))
}

/// Check an ANLZ file's framing: the PMAI header (28 bytes, declared total = file length) and
/// every section's header and total lengths lying inside the file and tiling it exactly.
pub fn validate(b: &[u8]) -> Result<(), String> {
    let be32 = |o: usize| -> Option<usize> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
    if b.len() < 28 || &b[..4] != b"PMAI" {
        return Err("not an analysis file".into());
    }
    let (hl, total) = (be32(4).unwrap_or(0), be32(8).unwrap_or(0));
    if hl != 28 {
        return Err(format!("header length {hl}, expected 28"));
    }
    if total != b.len() {
        return Err(format!("declares {total} bytes, file has {}", b.len()));
    }
    // The first section is the file's path.
    if b.len() >= hl + 12 && &b[hl..hl + 4] == b"PPTH" {
        let st = be32(hl + 8).unwrap_or(0);
        if b.len() < hl + st || parse_ppth(&b[hl..hl + st]).is_none() {
            return Err("the path section (PPTH) is malformed".into());
        }
    }
    let mut i = hl;
    let mut sections = 0;
    while i < b.len() {
        let (Some(shl), Some(st)) = (be32(i + 4), be32(i + 8)) else { return Err(format!("section at {i} is cut short")) };
        sections += 1;
        if sections > 1024 {
            return Err("analysis file exceeds the supported 1024 top-level sections".into());
        }
        if !b[i..i + 4].iter().all(|c| matches!(c, 32..=126)) {
            return Err(format!("invalid analysis section identifier at {i}"));
        }
        if st < 12 || shl < 12 || shl > st || i + st > b.len() {
            return Err(format!("section {} at {i} has header {shl} / total {st} outside the file", String::from_utf8_lossy(&b[i..(i + 4).min(b.len())])));
        }
        i += st;
    }
    Ok(())
}

/// Every top-level section of ANLZ file bytes, in order.
pub fn section_list(b: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    if b.len() < 12 || &b[..4] != b"PMAI" {
        return out;
    }
    let be32 = |o: usize| -> Option<usize> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
    let mut i = be32(4).unwrap_or(b.len());
    while i + 12 <= b.len() {
        let Some(total) = be32(i + 8) else { break };
        if total < 12 || i + total > b.len() {
            break;
        }
        out.push((String::from_utf8_lossy(&b[i..i + 4]).to_string(), b[i..i + total].to_vec()));
        i += total;
    }
    out
}

/// Load only the supported PSSI section for deck analysis and USB export planning.
/// Waveforms can dwarf the phrase table, so seek past every other section.
pub fn read_pssi(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    let size = |s: &[u8]| u32::from_be_bytes(s.try_into().unwrap()) as u64;
    if &header[..4] != b"PMAI" || size(&header[4..8]) != 28 || size(&header[8..]) != len { return None; }
    let mut offset = 28;
    for _ in 0..1024 {
        if offset + 12 > len { return None; }
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut header).ok()?;
        let section_len = size(&header[8..]);
        if !(12..=section_len).contains(&size(&header[4..8])) || section_len > len - offset { return None; }
        if &header[..4] == b"PSSI" {
            // 32-byte header and at most u16::MAX entries, each 24 bytes.
            if !(32..=32 + 24 * u16::MAX as u64).contains(&section_len) { return None; }
            let mut bytes = vec![0; section_len as usize];
            bytes[..12].copy_from_slice(&header);
            file.read_exact(&mut bytes[12..]).ok()?;
            return crate::phrase::supported_section(&bytes).then_some(bytes);
        }
        offset += section_len;
    }
    None
}

/// Every top-level section of an ANLZ file, by tag (first occurrence), as raw bytes.
pub fn read_sections(path: &std::path::Path) -> std::collections::HashMap<String, Vec<u8>> {
    let mut out = std::collections::HashMap::new();
    let Ok(b) = std::fs::read(path) else { return out };
    if b.len() < 12 || &b[..4] != b"PMAI" {
        return out;
    }
    let be32 = |o: usize| -> Option<usize> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
    let mut i = be32(4).unwrap_or(b.len());
    while i + 12 <= b.len() {
        let Some(total) = be32(i + 8) else { break };
        if total < 12 || i + total > b.len() {
            break;
        }
        let tag = String::from_utf8_lossy(&b[i..i + 4]).to_string();
        out.entry(tag).or_insert_with(|| b[i..i + total].to_vec());
        i += total;
    }
    out
}

/// Cues stored in a track's analysis files: the EXT file's PCO2 lists (names, colours) when
/// present, else the DAT file's PCOB lists.
pub fn parse_cues(ext: &[(String, Vec<u8>)], dat: &[(String, Vec<u8>)]) -> Vec<Cue> {
    let be32 = |b: &[u8], o: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?)) };
    let mut out = vec![];
    let mk = |hot: u32, kind: u8, start: u32, end: u32, color: u8, comment: String| Cue {
        id: 0,
        track_id: 0,
        kind: if hot > 0 { CueKind::Hot } else { CueKind::Memory },
        slot: if hot > 0 { hot as i32 - 1 } else { -1 },
        start: start as f64 / 1000.0,
        end: (kind == 2 && end != u32::MAX).then(|| end as f64 / 1000.0),
        color,
        comment,
        active_loop: false,
    };
    let entries = |sec: &[u8]| -> Vec<Vec<u8>> {
        let mut v = vec![];
        let mut i = be32(sec, 4).unwrap_or(u32::MAX) as usize;
        while i + 12 <= sec.len() {
            let Some(total) = be32(sec, i + 8).map(|x| x as usize) else { break };
            if total < 12 || i + total > sec.len() {
                break;
            }
            v.push(sec[i..i + total].to_vec());
            i += total;
        }
        v
    };
    let pco2: Vec<&Vec<u8>> = ext.iter().filter(|(t, _)| t == "PCO2").map(|(_, b)| b).collect();
    if !pco2.is_empty() {
        for sec in pco2 {
            for e in entries(sec) {
                // Only point cues (1) and loops (2) are known entry types.
                if &e[..4] != b"PCP2" || e.len() < 44 || !matches!(e[16], 1 | 2) {
                    continue;
                }
                let hot = be32(&e, 12).unwrap_or(0);
                let n = be32(&e, 40).unwrap_or(0) as usize;
                let raw = e.get(44..44 + n).unwrap_or(&[]);
                let comment = String::from_utf16_lossy(&raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect::<Vec<_>>());
                let color = if hot > 0 { e.get(44 + n).copied().unwrap_or(0) } else { e[28] };
                out.push(mk(hot, e[16], be32(&e, 20).unwrap_or(0), be32(&e, 24).unwrap_or(u32::MAX), color, comment));
            }
        }
        return out;
    }
    for (_, sec) in dat.iter().filter(|(t, _)| t == "PCOB") {
        for e in entries(sec) {
            if &e[..4] != b"PCPT" || e.len() < 40 || !matches!(e[28], 1 | 2) {
                continue;
            }
            let hot = be32(&e, 12).unwrap_or(0);
            let mut c = mk(hot, e[28], be32(&e, 32).unwrap_or(0), be32(&e, 36).unwrap_or(u32::MAX), 0, String::new());
            c.active_loop = be32(&e, 16) == Some(4);
            out.push(c);
        }
    }
    out
}

/// A stable digest of a cue list as the drive stores it (millisecond positions, colour, name),
/// so edits made on a player can be told apart from what Recordbox exported.
pub fn cue_fingerprint(cues: &[Cue]) -> String {
    let mut rows: Vec<String> = cues
        .iter()
        .map(|c| format!("{}{}:{}:{}:{}:{}", if c.kind == CueKind::Hot { 'H' } else { 'M' }, c.slot, ms(c.start), c.end.map(ms).map(|x| x as i64).unwrap_or(-1), c.color, c.comment))
        .collect();
    rows.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rows.join("\u{1f}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The fingerprint Recordbox's own cues get once written to a drive (same normalisation as
/// [`build_with`]: hot cues A–H, default colours filled in).
pub fn local_cue_fingerprint(cues: &[Cue]) -> String {
    let files = build("/x", &[], &Waveform::default(), cues, 0, 44100);
    cue_fingerprint(&parse_cues(&section_list(&files.ext), &section_list(&files.dat)))
}

pub fn build_with(usb_path: &str, beats: &[Beat], wave: &Waveform, cues: &[Cue], raw_samples: u64, sample_rate: u32, ov: Option<&Overrides>) -> AnlzFiles {
    let mut hot: Vec<&Cue> = cues.iter().filter(|c| c.kind == CueKind::Hot && (0..8).contains(&c.slot)).collect();
    hot.sort_by_key(|c| c.slot);
    let mut mem: Vec<&Cue> = cues.iter().filter(|c| c.kind == CueKind::Memory).collect();
    mem.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    let frames = padded_frames(wave, raw_samples, sample_rate);
    let from_device = |tag: &str| -> Option<Vec<u8>> {
        if !AUDIO_TAGS.contains(&tag) {
            return None;
        }
        ov?.device.values().flatten().find(|(t, _)| t == tag).map(|(_, b)| b.clone())
    };
    let take = |tag: &str| ov.and_then(|o| o.sections.get(tag).cloned()).or_else(|| from_device(tag));
    // A signed shift or shortened native PQTZ can change beat numbering even
    // when its first tempo anchor still compares equal. Locally edited or
    // inferred phrases must be paired with exactly the grid used to number them.
    let grid_ov = ov.filter(|o| o.grid_same && o.sections.get("PQTZ").is_some_and(|s| {
        if o.pssi.is_some() { crate::rbimport::pqtz_matches_all_beats(s, beats) } else { crate::rbimport::valid_pqtz_section(s) }
    }));
    let take_grid = |tag: &str| grid_ov.and_then(|o| o.sections.get(tag).cloned());
    let or = |tag: &str, f: &dyn Fn() -> Vec<u8>| take(tag).unwrap_or_else(f);
    let dat = vec![
        ppth(usb_path),
        // The trailing sample count is written for MP3 files only (rekordbox writes 0 for WAV:
        // 13 of 13 reference files; 117 MP3s carry the count).
        or("PVBR", &|| pvbr(if usb_path.to_ascii_lowercase().ends_with(".mp3") { raw_samples } else { 0 })),
        take_grid("PQTZ").unwrap_or_else(|| pqtz(beats)),
        or("PWAV", &|| pwav(wave)),
        or("PWV2", &|| pwv2(wave)),
        pcob(true, &hot),
        pcob(false, &mem),
    ];
    // Section order as rekordbox writes it: PPTH, PWV3, PCOB×2, PCO2×2, PQT2, PWV5, PWV4, PSSI.
    let mut ext = vec![ppth(usb_path), or("PWV3", &|| pwv3(&frames)), pcob(true, &[]), pcob(false, &[]), pco2(true, &hot, beats), pco2(false, &mem, beats)];
    // Extended grid: rekordbox's own while its grid still applies, else derived from the beat
    // grid section this file set carries (always written for a track with beats).
    match take_grid("PQT2") {
        Some(s) => ext.push(s),
        None => {
            let entries = pqtz_entries(&dat[2]);
            if !entries.is_empty() {
                // Fractions come from Recordbox's own beat times when those are the beats
                // written (same count); for rekordbox's grid without its PQT2 they are unknown: 0.
                let own = take_grid("PQTZ").is_none();
                let fr: Vec<u16> = if own && beats.len() == entries.len() { beats.iter().map(|b| micros(b.time)).collect() } else { vec![] };
                ext.push(pqt2(&entries, &fr));
            }
        }
    }
    ext.push(or("PWV5", &|| pwv5(&frames)));
    ext.push(or("PWV4", &|| pwv4(wave)));
    // Phrases: rekordbox's own when its grid still applies, else Recordbox's. Written with the
    // XOR mask rekordbox 6+ applies to exported files.
    let valid_pssi = |bytes: &Vec<u8>| crate::phrase::supported_section(bytes);
    if let Some(mut s) = take_grid("PSSI").filter(valid_pssi).or_else(|| ov.and_then(|o| o.pssi.clone()).filter(valid_pssi)) {
        if !crate::phrase::is_masked(&s) {
            crate::phrase::mask_section(&mut s);
        }
        ext.push(s);
    }
    let mut ex2 = vec![ppth(usb_path), or("PWV7", &|| pwv7(&frames)), or("PWV6", &|| pwv6(wave)), or("PWVC", &pwvc)];
    if let Some(o) = ov { append_device_vocals(&mut ex2, o); }
    let mut dat = dat;
    // Sections a newer rekordbox (or another tool) wrote that Recordbox doesn't know stay in
    // their file, after Recordbox's own.
    if let Some(o) = ov {
        for (ext_name, list) in [("DAT", &mut dat), ("EXT", &mut ext), ("2EX", &mut ex2)] {
            for (tag, bytes) in o.device.get(ext_name).into_iter().flatten() {
                if !KNOWN_TAGS.contains(&tag.as_str()) || (tag == "PVDI" && ext_name != "2EX") {
                    list.push(bytes.clone());
                }
            }
        }
    }
    AnlzFiles { dat: file(dat), ext: file(ext), ex2: file(ex2) }
}

/// Folder for a track's analysis files, from its media path on the drive (hash recovered from
/// the original; reproduces real exports, see MECHANICS.md).
pub fn anlz_folder(usb_path: &str) -> String {
    let mut h: u32 = 0;
    for c in usb_path.encode_utf16() {
        h = h.wrapping_mul(23497).wrapping_add(c as u32).wrapping_mul(37813).wrapping_add(c as u32);
    }
    let n = h % 200_003;
    let bits = [0, 2, 6, 7, 9, 13, 16];
    let mut p = 0u32;
    for (k, b) in bits.iter().enumerate() {
        p |= ((n >> b) & 1) << k;
    }
    format!("/PIONEER/USBANLZ/P{:03X}/{:08X}", p, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocal_levels_match_the_usb_oracle_and_survive_a_second_export() {
        // ConvUSB_ScrambleVocalDetect in Rekordbox 7.2.19, upstream 9dac758:
        // two periods of the measured output for plain zero levels.
        let oracle = [0x1e, 0xdd, 0x1e, 0x19, 0x02, 0x19, 0x1b, 0x11, 0x19, 0x23,
            0x18, 0x19, 0x24, 0x11, 0xfb, 0x11, 0x1e, 0x2a, 0x15];
        let levels: Vec<u8> = (0..42).map(|i| if i < 38 { 0 } else { (i - 37) as u8 }).collect();
        let plain = pvdi(&levels);
        let mut ov = Overrides { sections: Default::default(), grid_same: false, pssi: None,
            device: Default::default(), pvdi: Some(plain.clone()) };
        let export = |ov: &Overrides| section_list(&build_with("/Contents/test.wav", &[], &Waveform::default(), &[], 0, 44_100, Some(ov)).ex2)
            .into_iter().find(|(tag, _)| tag == "PVDI").unwrap().1;
        let wire = export(&ov);
        assert_eq!(&wire[..12], &plain[..12]);
        assert_eq!(&wire[12..14], &[0x80, 0]);
        assert_eq!(&wire[14..24], &plain[14..24]);
        assert_eq!(&wire[24..43], &oracle);
        assert_eq!(&wire[43..62], &oracle);
        assert_eq!(&wire[62..], &[oracle[0] ^ 1, oracle[1] ^ 2, oracle[2] ^ 3, oracle[3] ^ 4]);
        assert_eq!(parse_pvdi(&wire), parse_pvdi(&plain));
        assert_eq!(parse_pvdi(&wire).unwrap().1, levels);
        ov.pvdi = None;
        ov.device.insert("2EX", vec![("PVDI".into(), wire.clone())]);
        assert_eq!(export(&ov), wire);
    }

    #[test]
    fn vocal_levels_reject_unknown_or_invalid_layouts_and_keep_opaque_device_bytes() {
        let plain = pvdi(&[1, 2, 3]);
        for (offset, value) in [(0, b'X'), (7, 23), (11, 26), (12, 0x7f), (13, 1),
            (15, 1), (17, 0), (19, 0), (19, 2), (23, 2), (24, 5), (24, 255)] {
            let mut damaged = plain.clone();
            damaged[offset] = value;
            assert!(parse_pvdi(&damaged).is_none(), "field {offset}");
        }
        let mut future = plain.clone();
        future[12..14].copy_from_slice(&0x8001u16.to_be_bytes());
        assert!(parse_pvdi(&future).is_none());
        let mut wrongly_marked = plain.clone();
        wrongly_marked[12] = 0x80;
        assert!(parse_pvdi(&wrongly_marked).is_none());
        assert!(parse_pvdi(&plain[..plain.len() - 1]).is_none());
        let invalid_local = Overrides { sections: Default::default(), grid_same: false, pssi: None,
            device: Default::default(), pvdi: Some(pvdi(&[42])) };
        let mut levels = vec![];
        append_device_vocals(&mut levels, &invalid_local);
        assert!(levels.is_empty());

        let mut ov = Overrides { sections: [("PVDI".into(), plain.clone())].into(),
            grid_same: false, pssi: None, device: Default::default(), pvdi: None };
        for originals in [vec![future.clone()], vec![plain.clone(), future.clone()]] {
            ov.device.insert("2EX", originals.iter().map(|s| ("PVDI".into(), s.clone())).collect());
            ov.device.insert("EXT", vec![("PVDI".into(), future.clone())]);
            let files = build_with("/Contents/test.wav", &[], &Waveform::default(), &[], 0, 44_100, Some(&ov));
            let vocals = |bytes: &[u8]| section_list(bytes).into_iter().filter(|(tag, _)| tag == "PVDI").map(|(_, bytes)| bytes).collect::<Vec<_>>();
            assert_eq!(vocals(&files.ex2), originals);
            assert_eq!(vocals(&files.ext), [future.clone()]);
        }
    }


    #[test]
    fn unknown_printable_sections_survive_but_invalid_identifiers_are_rejected() {
        let mut bytes = file(vec![section(b"PNEW", &[], &[])]);
        assert!(validate(&bytes).is_ok());
        bytes[29] = 0;
        assert!(validate(&bytes).is_err());
        assert!(validate(&file(vec![section(b"PNEW", &[], &[]); 1025])).is_err());
    }
    #[test]
    fn phrase_reader_seeks_over_waveforms_and_checks_extent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a.EXT");
        let phrase = crate::phrase::pssi(&crate::phrase::Structure { mood: 2, phrases: vec![], end_beat: 128, rb: None });
        let mut bytes = file(vec![section(b"PWV5", &[], &[42; 16384]), phrase.clone()]);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_pssi(&path), Some(phrase));
        bytes[36..40].copy_from_slice(&u32::MAX.to_be_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert!(read_pssi(&path).is_none());
    }

    #[test]
    fn irregular_model_grid_keeps_exact_dat_and_ext_beats_after_reopening_sqlite() -> anyhow::Result<()> {
        use crate::model::{BeatGrid, GridAnchor};
        let root = tempfile::tempdir()?;
        let path = root.path().join("library.db");
        let grid = BeatGrid::from_listed(vec![
            GridAnchor { time: 1.000501, bpm: 120.37, beat: 3 },
            GridAnchor { time: 1.800064, bpm: 120.37, beat: 4 },
            GridAnchor { time: 2.266999, bpm: 129.97, beat: 1 },
        ]);
        let id = {
            let db = crate::db::Db::open(&path)?;
            let track = crate::importer::read_track(&root.path().join("example.wav"));
            let id = db.insert_track(&track)?;
            db.save_grid(id, &grid)?;
            id
        };
        let db = crate::db::Db::open(&path)?;
        let loaded = db.grid_checked(id)?.unwrap();
        let beats = loaded.beats(10.0);
        let files = build("/Contents/example.wav", &beats, &Waveform::default(), &[], 0, 44100);
        let dat = section_list(&files.dat).into_iter().find(|(tag, _)| tag == "PQTZ").unwrap().1;
        let ext = section_list(&files.ext).into_iter().find(|(tag, _)| tag == "PQT2").unwrap().1;
        assert_eq!(pqtz_entries(&dat), [(3, 12037, 1000), (4, 12037, 1800), (1, 12997, 2266)]);
        assert_eq!(pqt2_fractions(&ext), [501, 64, 999]);
        Ok(())
    }

    #[test]
    fn local_phrase_rows_follow_the_exported_grid() {
        let beats = crate::model::BeatGrid::constant(0.443, 137.0, 1).beats(1.1);
        let local_grid = pqtz(&beats);
        let local_pssi = crate::phrase::pssi(&crate::phrase::Structure {
            mood: 2, phrases: vec![crate::phrase::Phrase { beat: 2, kind: 2 }], end_beat: 3, rb: None,
        });
        let mut ov = Overrides { grid_same: true, pssi: Some(local_pssi), sections: Default::default(), device: Default::default(), pvdi: None };
        let output = |ov: &Overrides| {
            let f = build_with("/a.wav", &beats, &Waveform::default(), &[], 0, 44100, Some(ov));
            assert!(section_list(&f.ext).iter().any(|(tag, _)| tag == "PSSI"));
            section_list(&f.dat).into_iter().find(|(tag, _)| tag == "PQTZ").unwrap().1
        };
        assert_eq!(output(&ov), local_grid, "no native grid");
        let mut same = local_grid.clone();
        same[16] = 7; // Opaque header marks that the compatible original is preserved.
        ov.sections.insert("PQTZ".into(), same.clone());
        assert_eq!(output(&ov), same);
        ov.grid_same = false;
        assert_eq!(output(&ov), local_grid, "changed local grid");
        ov.grid_same = true;
        let native_beats = crate::model::BeatGrid::constant(0.029, 137.0, 1).beats(1.0);
        let mut shifted = pqtz(&native_beats);
        shifted[18..20].copy_from_slice(&414i16.to_be_bytes());
        ov.sections.insert("PQTZ".into(), shifted);
        assert_eq!(output(&ov), local_grid, "positive native shift inserted a preceding local beat");
        ov.sections.insert("PQTZ".into(), pqtz(&beats[..beats.len()-1]));
        assert_eq!(output(&ov), local_grid, "local tail also needs beat records");
    }

    #[test]
    fn export_skips_invalid_pssi_and_preserves_valid_source_bytes() {
        let mut native = crate::phrase::pssi(&crate::phrase::Structure {
            mood: 1, phrases: vec![crate::phrase::Phrase { beat: 1, kind: 2 }], end_beat: 64, rb: None,
        });
        native[30] = 9;
        crate::phrase::mask_section(&mut native);
        let mut ov = Overrides { sections: Default::default(), grid_same: true, pssi: None, device: Default::default(), pvdi: None };
        let beats = crate::model::BeatGrid::constant(0.1, 120.0, 1).beats(1.1);
        let exported = |ov: &Overrides| section_list(&build_with("/a.wav", &beats, &Waveform::default(), &[], 0, 44100, Some(ov)).ext)
            .into_iter().find(|(tag, _)| tag == "PSSI").map(|(_, bytes)| bytes);
        ov.sections.insert("PSSI".into(), native.clone());
        assert!(exported(&ov).is_none(), "native PSSI requires an available companion grid");
        let mut shifted = pqtz(&beats);
        shifted[18..20].copy_from_slice(&600i16.to_be_bytes());
        ov.sections.insert("PQTZ".into(), shifted);
        assert_eq!(exported(&ov).as_ref(), Some(&native), "unchanged native pair retains its own numbering");
        ov.sections.insert("PQTZ".into(), pqtz(&beats));
        ov.sections.get_mut("PSSI").unwrap()[15] = 23;
        assert!(exported(&ov).is_none());
        ov.pssi = Some(native.clone());
        assert_eq!(exported(&ov).as_ref(), Some(&native));
        ov.pssi.as_mut().unwrap()[18..20].fill(0);
        assert!(exported(&ov).is_none());
    }

    /// Unknown sections already on the drive survive a rewrite; audio sections are reused.
    #[test]
    fn rewrite_keeps_device_sections() {
        let w = Waveform { frames: vec![[100, 80, 40, 20, 60, 50, 20, 10]; 1500] };
        let g = crate::model::BeatGrid::constant(0.1, 120.0, 1).beats(10.0);
        let first = build("/Contents/a.mp3", &g, &w, &[], 441_000, 44100);
        let dir = std::env::temp_dir().join(format!("recordbox_anlz_keep_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A device file with an extra unknown section appended.
        let mut ext_sections: Vec<Vec<u8>> = vec![];
        {
            let p = dir.join("A.EXT");
            std::fs::write(&p, &first.ext).unwrap();
            for (_, b) in read_section_list(&p) {
                ext_sections.push(b);
            }
        }
        let mut odd = b"PZZZ".to_vec();
        odd.extend_from_slice(&12u32.to_be_bytes());
        odd.extend_from_slice(&16u32.to_be_bytes());
        odd.extend_from_slice(&[1, 2, 3, 4]);
        ext_sections.push(odd.clone());
        std::fs::write(dir.join("A.EXT"), file(ext_sections)).unwrap();
        let mut marker = pwv4(&w);
        let n = marker.len();
        marker[n - 1] ^= 0x5a; // distinguishable from a freshly built PWV4
        let mut device = std::collections::HashMap::new();
        let mut list = read_section_list(&dir.join("A.EXT"));
        for (t, b) in list.iter_mut() {
            if t == "PWV4" {
                *b = marker.clone();
            }
        }
        device.insert("EXT", list);
        let ov = Overrides { sections: Default::default(), grid_same: false, pssi: None, device, pvdi: None };
        let again = build_with("/Contents/a.mp3", &g, &w, &[], 441_000, 44100, Some(&ov));
        std::fs::write(dir.join("B.EXT"), &again.ext).unwrap();
        let out = read_section_list(&dir.join("B.EXT"));
        assert_eq!(out.last().map(|(t, b)| (t.as_str(), b.clone())), Some(("PZZZ", odd)));
        assert_eq!(out.iter().find(|(t, _)| t == "PWV4").map(|(_, b)| b.clone()), Some(marker));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cues read back from built sections match what was written.
    #[test]
    fn cues_round_trip() {
        let w = Waveform { frames: vec![[100, 80, 40, 20, 60, 50, 20, 10]; 100] };
        let cues = vec![
            Cue { id: 1, track_id: 1, kind: CueKind::Hot, slot: 2, start: 1.234, end: Some(5.0), color: 9, comment: "Drop é".into(), active_loop: false },
            Cue { id: 2, track_id: 1, kind: CueKind::Memory, slot: -1, start: 2.5, end: None, color: 3, comment: String::new(), active_loop: false },
        ];
        let f = build("/a.mp3", &[], &w, &cues, 0, 44100);
        let back = parse_cues(&section_list(&f.ext), &section_list(&f.dat));
        assert_eq!(back.len(), 2);
        let h = back.iter().find(|c| c.kind == CueKind::Hot).unwrap();
        assert_eq!((h.slot, h.start, h.end, h.color, h.comment.as_str()), (2, 1.234, Some(5.0), 9, "Drop é"));
        let m = back.iter().find(|c| c.kind == CueKind::Memory).unwrap();
        assert_eq!((m.start, m.end, m.color), (2.5, None, 3));
        assert_eq!(cue_fingerprint(&back), local_cue_fingerprint(&cues));
        // DAT-only fallback keeps positions.
        let dat_only = parse_cues(&[], &section_list(&f.dat));
        assert_eq!(dat_only.len(), 2);
        let mut moved = cues.clone();
        moved[1].start = 2.6;
        assert_ne!(local_cue_fingerprint(&moved), local_cue_fingerprint(&cues));
    }

    #[test]
    fn validates_framing() {
        let w = Waveform { frames: vec![[10; 8]; 50] };
        let f = build("/a.mp3", &[], &w, &[], 0, 44100);
        assert!(validate(&f.dat).is_ok() && validate(&f.ext).is_ok() && validate(&f.ex2).is_ok());
        assert_eq!(parse_ppth(&ppth("/Contents/Ünï/a.mp3")).as_deref(), Some("/Contents/Ünï/a.mp3"));
        let mut odd = ppth("/a.mp3");
        let n = odd.len();
        odd[n - 1] = 0x41; // not NUL-terminated
        assert!(parse_ppth(&odd).is_none());
        assert!(validate(&f.dat[..f.dat.len() - 3]).is_err());
        let mut bad = f.ext.clone();
        bad[28 + 8..28 + 12].copy_from_slice(&9_999_999u32.to_be_bytes());
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn folder_hash_matches_real_exports() {
        assert_eq!(anlz_folder("/Contents/Loopmasters/UnknownAlbum/Demo Track 1.mp3"), "/PIONEER/USBANLZ/P016/0000875E");
        assert_eq!(anlz_folder("/Contents/Loopmasters/UnknownAlbum/Demo Track 2.mp3"), "/PIONEER/USBANLZ/P053/0001D21F");
    }
    #[test]
    fn sections_are_well_formed() {
        let w = Waveform { frames: vec![[100, 80, 40, 20, 60, 50, 20, 10]; 1500] };
        let g = crate::model::BeatGrid::constant(0.1, 120.0, 1).beats(10.0);
        let cue = Cue { id: 1, track_id: 1, kind: CueKind::Hot, slot: 0, start: 1.0, end: None, color: 43, comment: "Drop".into(), active_loop: false };
        let mem = Cue { id: 2, track_id: 1, kind: CueKind::Memory, slot: -1, start: 2.0, end: Some(4.0), color: 3, comment: String::new(), active_loop: false };
        let f = build("/Contents/a/b/c.mp3", &g, &w, &[cue, mem], 441_000, 44100);
        // Seek index ends with the total sample count; detail waveform spans the full length.
        assert!(f.dat.windows(4).any(|x| x == 441_000u32.to_be_bytes()));
        let pvbr_tail = |dat: &[u8]| section_list(dat).into_iter().find(|(t, _)| t == "PVBR").map(|(_, b)| b[b.len() - 4..].to_vec()).unwrap();
        assert_eq!(pvbr_tail(&f.dat), 441_000u32.to_be_bytes());
        let wav = build("/Contents/a/b/c.wav", &g, &w, &[], 441_000, 44100);
        assert_eq!(pvbr_tail(&wav.dat), [0, 0, 0, 0]);
        let t = std::env::temp_dir();
        let _ = std::fs::write(t.join("recordbox_ANLZ0000.DAT"), &f.dat);
        let _ = std::fs::write(t.join("recordbox_ANLZ0000.EXT"), &f.ext);
        let _ = std::fs::write(t.join("recordbox_ANLZ0000.2EX"), &f.ex2);
        for d in [&f.dat, &f.ext, &f.ex2] {
            assert_eq!(&d[0..4], b"PMAI");
            let total = u32::from_be_bytes([d[8], d[9], d[10], d[11]]) as usize;
            assert_eq!(total, d.len());
            let mut o = 28;
            while o < d.len() {
                let tl = u32::from_be_bytes([d[o + 8], d[o + 9], d[o + 10], d[o + 11]]) as usize;
                assert!(tl >= 12);
                o += tl;
            }
            assert_eq!(o, d.len());
        }
    }
}
