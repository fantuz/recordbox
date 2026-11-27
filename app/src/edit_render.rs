//! "Render audio files" (spec: checks/re-7.2.19/edit-mode.md section 5): the whole score is
//! played offline through the same `Voice` that plays it live (loop off, from beat 0) and
//! written as WAV, AIFF, FLAC or M4A. WAV and AIFF are written here; FLAC by the small encoder
//! below (fixed predictors with Rice coding, lossless, checked against the decoder in the
//! tests); M4A by macOS's own AAC encoder through `afconvert`, since there is no AAC encoder
//! in Rust in the tree. All files are 16-bit stereo (rekordbox's bit depth was not traced).

use crate::audio::Decoded;
use crate::edit_audio::Voice;
use crate::edit_model::{ScoreSet, RENDER_MAX_MS};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// rekordbox's format numbers and the order of its File Format list.
pub const FORMATS: [(&str, &str); 4] = [(".FLAC", "flac"), (".WAV", "wav"), (".AIFF", "aiff"), (".M4A", "m4a")];
pub const RATES: [u32; 2] = [48000, 44100];
pub const AAC_RATES: [u32; 3] = [128_000, 192_000, 256_000];

pub const MSG_TOO_LONG: &str = "The maximum length of tracks that can be rendered is 180 minutes.";
pub const MSG_NO_SPACE: &str = "Rendering will stop. There is not enough storage space.";
pub const MSG_NO_DEST: &str = "Rendering will stop. The save destination cannot be found.";

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// 0 .flac, 1 .wav, 2 .aiff, 3 .m4a
    pub format: u8,
    /// WAV / AIFF sample rate.
    pub rate: u32,
    pub aac_bps: u32,
    /// 0 (fastest, largest) ..= 8 (smallest).
    pub flac_level: u8,
}

impl Options {
    /// The sample rate the file gets: the chosen one for WAV and AIFF, the source file's for
    /// FLAC and M4A.
    pub fn out_rate(&self, source: u32) -> u32 {
        if matches!(self.format, 1 | 2) {
            self.rate
        } else {
            source
        }
    }
    pub fn extension(&self) -> &'static str {
        FORMATS[(self.format as usize).min(3)].1
    }
}

/// In-memory reference used to compare file renders with the live voice.
#[cfg(test)]
fn bounce(audio: Arc<Decoded>, score: Arc<ScoreSet>, rate: u32, progress: &AtomicU32, cancel: &AtomicBool) -> Option<Vec<f32>> {
    let len = score.len();
    let want = (score.len_ms() * 0.001 * rate as f64).round() as usize;
    let mut v = Voice::new();
    v.keep_pitch = score.song_grid.is_dynamic();
    v.audio = Some(audio);
    v.score = Some(score);
    v.instant = true;
    v.playing = true;
    let mut out: Vec<f32> = Vec::with_capacity(want * 2 + 8192);
    let mut buf = vec![0.0f32; 8192];
    while v.playing && out.len() < (want + rate as usize) * 2 {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        buf.iter_mut().for_each(|x| *x = 0.0);
        v.render(&mut buf, 2, rate, 1.0);
        out.extend_from_slice(&buf);
        progress.store(((v.pos / len.max(1e-9)).clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    }
    out.truncate(want * 2);
    Some(out)
}

#[cfg(test)]
fn to_i16(x: &[f32]) -> Vec<i16> {
    x.iter().map(|v| (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16).collect()
}

fn wav_header(data: u32, rate: u32, channels: u16, bits: u16) -> Vec<u8> {
    let mut o = Vec::with_capacity(44);
    o.extend_from_slice(b"RIFF");
    o.extend_from_slice(&(36 + data + data % 2).to_le_bytes());
    o.extend_from_slice(b"WAVEfmt ");
    o.extend_from_slice(&16u32.to_le_bytes());
    o.extend_from_slice(&1u16.to_le_bytes());
    o.extend_from_slice(&channels.to_le_bytes());
    o.extend_from_slice(&rate.to_le_bytes());
    o.extend_from_slice(&(rate * u32::from(channels * (bits / 8))).to_le_bytes());
    o.extend_from_slice(&(channels * (bits / 8)).to_le_bytes());
    o.extend_from_slice(&bits.to_le_bytes());
    o.extend_from_slice(b"data");
    o.extend_from_slice(&data.to_le_bytes());
    o
}

/// A sample rate as the 80-bit extended float AIFF's COMM chunk wants.
fn extended(rate: u32) -> [u8; 10] {
    let mut o = [0u8; 10];
    if rate == 0 {
        return o;
    }
    let shift = rate.leading_zeros();
    let exp = 16383 + 31 - shift as u16;
    o[..2].copy_from_slice(&exp.to_be_bytes());
    o[2..6].copy_from_slice(&(rate << shift).to_be_bytes());
    o
}

fn aiff_header(data: u32, rate: u32, channels: u16, bits: u16) -> Vec<u8> {
    let mut o = Vec::with_capacity(54);
    o.extend_from_slice(b"FORM");
    o.extend_from_slice(&(4 + 26 + 16 + data + data % 2).to_be_bytes());
    o.extend_from_slice(b"AIFFCOMM");
    o.extend_from_slice(&18u32.to_be_bytes());
    o.extend_from_slice(&channels.to_be_bytes());
    o.extend_from_slice(&(data / u32::from(channels * (bits / 8))).to_be_bytes());
    o.extend_from_slice(&bits.to_be_bytes());
    o.extend_from_slice(&extended(rate));
    o.extend_from_slice(b"SSND");
    o.extend_from_slice(&(8 + data).to_be_bytes());
    o.extend_from_slice(&[0u8; 8]);
    o
}

// ---------------------------------------------------------------------------- FLAC

struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits { out: Vec::new(), acc: 0, n: 0 }
    }
    fn put(&mut self, v: u64, bits: u32) {
        let mut bits = bits;
        while bits > 0 {
            let take = bits.min(32);
            let part = (v >> (bits - take)) & ((1u64 << take) - 1);
            self.acc = (self.acc << take) | part;
            self.n += take;
            while self.n >= 8 {
                self.out.push((self.acc >> (self.n - 8)) as u8);
                self.n -= 8;
            }
            self.acc &= (1u64 << self.n) - 1;
            bits -= take;
        }
    }
    fn signed(&mut self, v: i32, bits: u32) {
        self.put((v as i64 as u64) & ((1u64 << bits) - 1), bits);
    }
    fn unary(&mut self, q: u32) {
        let mut q = q;
        while q >= 32 {
            self.put(0, 32);
            q -= 32;
        }
        self.put(1, q + 1);
    }
    fn align(&mut self) {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
    }
}

fn crc8(d: &[u8]) -> u8 {
    let mut c = 0u8;
    for b in d {
        c ^= b;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
        }
    }
    c
}

fn crc16(d: &[u8]) -> u16 {
    let mut c = 0u16;
    for b in d {
        c ^= (*b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x8005 } else { c << 1 };
        }
    }
    c
}

fn fixed_residual(x: &[i32], order: usize, out: &mut Vec<i32>) {
    out.clear();
    for i in order..x.len() {
        out.push(match order {
            0 => x[i],
            1 => x[i] - x[i - 1],
            2 => x[i] - 2 * x[i - 1] + x[i - 2],
            3 => x[i] - 3 * x[i - 1] + 3 * x[i - 2] - x[i - 3],
            _ => x[i] - 4 * x[i - 1] + 6 * x[i - 2] - 4 * x[i - 3] + x[i - 4],
        });
    }
}

#[inline]
fn fold(r: i32) -> u32 {
    ((r << 1) ^ (r >> 31)) as u32
}

/// Cheapest Rice parameter (0..=14) for these residuals and the bits it takes.
fn best_rice(res: &[i32]) -> (u32, u64) {
    let sum: u64 = res.iter().map(|r| fold(*r) as u64).sum();
    let n = res.len() as u64;
    let mut best = (0u32, u64::MAX);
    // The cost is convex in k; start near log2(mean) and look at the neighbours.
    let mean = if n > 0 { sum / n } else { 0 };
    let guess = (64 - mean.leading_zeros()).saturating_sub(1).min(14);
    for k in guess.saturating_sub(1)..=(guess + 1).min(14) {
        let bits: u64 = n * (k as u64 + 1) + res.iter().map(|r| (fold(*r) >> k) as u64).sum::<u64>();
        if bits < best.1 {
            best = (k, bits);
        }
    }
    best
}

struct Plan {
    order: usize,
    /// Rice partition order and each partition's parameter.
    po: u32,
    ks: Vec<u32>,
    bits: u64,
    res: Vec<i32>,
}

/// The cheapest fixed predictor (up to `max_order`) and Rice partitioning for one channel
/// of a block. None when storing the samples as they are is cheaper.
fn plan_channel(x: &[i32], bps: u32, max_order: usize, max_po: u32) -> Option<Plan> {
    let n = x.len();
    let mut best: Option<Plan> = None;
    let mut res = Vec::with_capacity(n);
    for order in 0..=max_order.min(4).min(n.saturating_sub(1)) {
        fixed_residual(x, order, &mut res);
        for po in 0..=max_po {
            let parts = 1usize << po;
            if n % parts != 0 || (n >> po) <= order {
                break;
            }
            let mut bits = order as u64 * bps as u64 + 6;
            let mut ks = Vec::with_capacity(parts);
            let mut at = 0usize;
            for p in 0..parts {
                let cnt = (n >> po) - if p == 0 { order } else { 0 };
                let (k, b) = best_rice(&res[at..at + cnt]);
                bits += 4 + b;
                ks.push(k);
                at += cnt;
            }
            if best.as_ref().map(|b| bits < b.bits).unwrap_or(true) {
                best = Some(Plan { order, po, ks, bits, res: res.clone() });
            }
        }
    }
    best.filter(|b| b.bits < n as u64 * bps as u64)
}

fn write_subframe(w: &mut Bits, x: &[i32], bps: u32, plan: &Option<Plan>) {
    match plan {
        None => {
            // VERBATIM
            w.put(0b0000_0010, 8);
            for s in x {
                w.signed(*s, bps);
            }
        }
        Some(p) => {
            w.put(((0b001000 | p.order as u64) << 1) | 0, 8);
            for s in &x[..p.order] {
                w.signed(*s, bps);
            }
            w.put(0, 2);
            w.put(p.po as u64, 4);
            let n = x.len();
            let mut at = 0usize;
            for (i, k) in p.ks.iter().enumerate() {
                let cnt = (n >> p.po) - if i == 0 { p.order } else { 0 };
                w.put(*k as u64, 4);
                for r in &p.res[at..at + cnt] {
                    let u = fold(*r);
                    w.unary(u >> k);
                    if *k > 0 {
                        w.put((u & ((1 << k) - 1)) as u64, *k);
                    }
                }
                at += cnt;
            }
        }
    }
}

fn utf8_number(w: &mut Bits, v: u64) {
    if v < 0x80 {
        w.put(v, 8);
        return;
    }
    let bytes = if v < 0x800 { 2 } else if v < 0x1_0000 { 3 } else if v < 0x20_0000 { 4 } else if v < 0x400_0000 { 5 } else { 6 };
    let lead = (0xFFu64 << (8 - bytes)) & 0xFF;
    w.put(lead | (v >> (6 * (bytes - 1))), 8);
    for i in (0..bytes - 1).rev() {
        w.put(0x80 | ((v >> (6 * i)) & 0x3F), 8);
    }
}

/// Mono/stereo, 16/24-bit FLAC. `level` 0..=8 sets how hard the encoder looks for the smallest form
/// of each block (predictor order, mid/side, Rice partitions); every level is lossless.
fn write_flac(
    out: &mut dyn Write, frames: usize, rate: u32, channels: u16, bits: u16, level: u8,
    mut fill: impl FnMut(&mut [i32]) -> Result<(), String>,
) -> Result<(), String> {
    let level = level.min(8) as usize;
    let block = if level < 3 { 1152 } else { 4096 };
    let max_order = [1, 2, 2, 3, 4, 4, 4, 4, 4][level];
    let max_po = [0, 0, 2, 2, 3, 4, 5, 6, 8][level];
    let bps = u32::from(bits);
    let try_ms = channels == 2 && level >= 3;
    out.write_all(b"fLaC").map_err(|e| io_message(&e))?;
    let mut si = Bits::new();
    si.put(0x80, 8);
    si.put(34, 24);
    si.put(block as u64, 16);
    si.put(block as u64, 16);
    si.put(0, 24);
    si.put(0, 24);
    si.put(rate as u64, 20);
    si.put(u64::from(channels - 1), 3);
    si.put(u64::from(bits - 1), 5);
    si.put(frames as u64, 36);
    out.write_all(&si.out).and_then(|()| out.write_all(&[0; 16])).map_err(|e| io_message(&e))?;
    let mut samples = vec![0i32; usize::from(channels) * block];
    let (mut l, mut r, mut m, mut s) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut number = 0u64;
    let mut at = 0usize;
    while at < frames {
        let n = block.min(frames - at);
        l.clear();
        r.clear();
        m.clear();
        s.clear();
        let pcm = &mut samples[..usize::from(channels) * n];
        fill(pcm)?;
        for frame in pcm.chunks_exact(usize::from(channels)) {
            l.push(frame[0]);
            if channels == 2 {
                r.push(frame[1]);
                m.push((frame[0] + frame[1]) >> 1);
                s.push(frame[0] - frame[1]);
            }
        }
        // (Partitions must divide the block, so a short last block gets none.)
        let po = if n == block { max_po } else { 0 };
        let cost = |p: &Option<Plan>, bps: u32| p.as_ref().map(|p| p.bits).unwrap_or(n as u64 * bps as u64);
        let pl = plan_channel(&l, bps, max_order, po);
        let pr = (channels == 2).then(|| plan_channel(&r, bps, max_order, po)).flatten();
        let mut choice = (u64::from(channels - 1), [(&l, bps, pl), (&r, bps, pr)]);
        if try_ms {
            let (pm, ps) = (plan_channel(&m, bps, max_order, po), plan_channel(&s, bps + 1, max_order, po));
            let lr = cost(&choice.1[0].2, bps) + cost(&choice.1[1].2, bps);
            if cost(&pm, bps) + cost(&ps, bps + 1) < lr {
                choice = (10, [(&m, bps, pm), (&s, bps + 1, ps)]);
            }
        }
        let mut w = Bits::new();
        w.put(0b1111_1111_1111_1000, 16);
        w.put(if n == 4096 { 0b1100 } else if n == 1152 { 0b0011 } else { 0b0111 }, 4);
        w.put(match rate {
            44100 => 0b1001,
            48000 => 0b1010,
            _ => 0,
        }, 4);
        w.put(choice.0, 4);
        w.put(if bits == 24 { 0b110 } else { 0b100 }, 3);
        w.put(0, 1);
        utf8_number(&mut w, number);
        if n != 4096 && n != 1152 {
            w.put(n as u64 - 1, 16);
        }
        let c = crc8(&w.out);
        w.put(c as u64, 8);
        for (x, bps, plan) in &choice.1[..usize::from(channels)] {
            write_subframe(&mut w, x, *bps, plan);
        }
        w.align();
        let c = crc16(&w.out);
        w.put(c as u64, 16);
        out.write_all(&w.out).map_err(|e| io_message(&e))?;
        number += 1;
        at += n;
    }
    Ok(())
}

// ---------------------------------------------------------------------------- files

fn io_message(e: &std::io::Error) -> String {
    // ENOSPC = 28 on macOS and Linux.
    if e.raw_os_error() == Some(28) {
        MSG_NO_SPACE.into()
    } else if e.kind() == std::io::ErrorKind::NotFound {
        MSG_NO_DEST.into()
    } else {
        format!("Rendering will stop. {e}")
    }
}

/// Pull a block of score samples at a time. The encoder never holds the whole render in RAM.
fn write_blocks(
    out: &mut dyn Write, format: u8, frames: usize, rate: u32, flac_level: u8,
    mut fill: impl FnMut(&mut [i16]) -> Result<(), String>,
) -> Result<(), String> {
    let mut narrow = [0i16; 8192];
    write_pcm(out, format, frames, rate, 2, 16, flac_level, |dst| {
        let src = &mut narrow[..dst.len()];
        fill(src)?;
        for (sample, &v) in dst.iter_mut().zip(src.iter()) { *sample = i32::from(v); }
        Ok(())
    })
}

/// Stream full-scale integer PCM through the same lossless encoders as the score renderer.
pub(crate) fn write_pcm(
    out: &mut dyn Write, format: u8, frames: usize, rate: u32, channels: u16, bits: u16, level: u8,
    mut fill: impl FnMut(&mut [i32]) -> Result<(), String>,
) -> Result<(), String> {
    if !(1..=2).contains(&channels) || !matches!(bits, 16 | 24) || !(1..=655350).contains(&rate) {
        return Err("Unsupported audio channel count, sample rate or bit depth.".into());
    }
    if format == 0 {
        if frames as u64 >= (1u64 << 36) { return Err("Too many frames for a FLAC stream.".into()); }
        return write_flac(out, frames, rate, channels, bits, level, fill);
    }
    let width = usize::from(bits / 8);
    let count = frames.checked_mul(usize::from(channels)).ok_or("Audio is too long.")?;
    let data = count.checked_mul(width).and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= u32::MAX - 47)
        .ok_or_else(|| "The render exceeds the 4 GiB limit of this audio container.".to_owned())?;
    let aiff = format == 2;
    let header = if aiff { aiff_header(data, rate, channels, bits) } else { wav_header(data, rate, channels, bits) };
    out.write_all(&header).map_err(|e| io_message(&e))?;
    let mut pcm = [0i32; 8192];
    let mut bytes = [0u8; 3 * 8192];
    let mut remaining = count;
    while remaining > 0 {
        let n = remaining.min(pcm.len());
        fill(&mut pcm[..n])?;
        for (sample, chunk) in pcm[..n].iter().zip(bytes.chunks_exact_mut(width)) {
            if aiff {
                chunk.copy_from_slice(&sample.to_be_bytes()[4-width..]);
            } else {
                chunk.copy_from_slice(&sample.to_le_bytes()[..width]);
            }
        }
        out.write_all(&bytes[..width * n]).map_err(|e| io_message(&e))?;
        remaining -= n;
    }
    if data % 2 == 1 { out.write_all(&[0]).map_err(|e| io_message(&e))?; }
    Ok(())
}

/// Encode into private staging files beside the destination. The caller publishes the
/// completed file without replacing a file that appeared while the render was running.
fn encode(
    path: &Path, opt: &Options, cancel: &AtomicBool, write_audio: impl FnOnce(&mut std::fs::File, u8) -> Result<(), String>,
) -> Result<tempfile::NamedTempFile, String> {
    let dir = path.parent().filter(|p| p.is_dir()).ok_or_else(|| MSG_NO_DEST.to_owned())?;
    // Preserve the extension for recordbox's decoder (MP3 timeline selection uses it).
    let suffix = match opt.format { 0 => ".flac", 2 => ".aiff", 3 => ".m4a", 4 => ".mp3", _ => ".wav" };
    let mut file = tempfile::Builder::new().prefix(".recordbox-").suffix(suffix).tempfile_in(dir).map_err(|e| io_message(&e))?;
    if opt.format == 3 {
        let tool = Path::new("/usr/bin/afconvert");
        if !tool.is_file() {
            return Err("M4A cannot be written here: macOS's audio converter (afconvert) was not found.".into());
        }
        // afconvert requires a new output path. Both the input and encoded output live
        // in a private directory; the public destination is never passed to the encoder.
        let stage = tempfile::tempdir_in(dir).map_err(|e| io_message(&e))?;
        let wav = stage.path().join("input.wav");
        let encoded = stage.path().join("output.m4a");
        let mut input = std::fs::File::create_new(&wav).map_err(|e| io_message(&e))?;
        write_audio(&mut input, 1)?;
        drop(input);
        let mut child = std::process::Command::new(tool)
            .args(["-f", "m4af", "-d", "aac", "-b", &opt.aac_bps.to_string()])
            .arg(&wav).arg(&encoded).spawn()
            .map_err(|e| format!("Rendering will stop. The AAC encoder could not be started: {e}"))?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("stopped".into());
            }
            if let Some(status) = child.try_wait().map_err(|e| io_message(&e))? {
                if !status.success() { return Err(format!("The AAC encoder exited with {status}.")); }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let mut output = std::fs::File::open(encoded).map_err(|e| io_message(&e))?;
        std::io::copy(&mut output, &mut file).map_err(|e| io_message(&e))?;
    } else {
        write_audio(file.as_file_mut(), opt.format)?;
    }
    file.as_file().sync_all().map_err(|e| io_message(&e))?;
    Ok(file)
}


/// MP3 bitrate: constant (kbps) or VBR quality (LAME's -V 0..=9, 0 best).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mp3Rate {
    Cbr(u32),
    Vbr(u8),
}

/// Highest bitrate no greater than the request (1 kbps metadata rounding tolerance).
/// The linked LAME supports MPEG-1 (32..320), MPEG-2 (8..160), and MPEG-2.5 (8..64).
/// Its safe Rust wrapper does not expose the 56 or 144 kbps constants.
pub(crate) fn mp3_bitrate(rate: u32, limit: u32) -> u32 {
    const RATES: [u32;16] = [8, 16, 24, 32, 40, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    let (lo, hi) = if rate < 16000 { (8,64) } else if rate < 32000 { (8,160) } else { (32,320) };
    let max = if limit == 0 { hi } else { limit.saturating_add(1).clamp(lo,hi) };
    RATES.into_iter().rev().find(|&b| b <= max).unwrap_or(lo)
}

/// MP3 with LAME (high quality, `-q 2`), joint stereo or mono, with the LAME / Info tag in the
/// first frame so players know the length and the encoder's delay and padding. The input
/// must be at a rate MP3 has (32, 44.1, 48 kHz or the MPEG-2 / 2.5 halves).
fn write_mp3(file: &mut std::fs::File, frames: usize, ch: u16, rate: u32, r: Mp3Rate,
    mut fill: impl FnMut(&mut [i32]) -> Result<(), String>) -> Result<(), String> {
    use mp3lame_encoder::{Bitrate, Builder, FlushGap, InterleavedPcm, Mode, MonoPcm, Quality, VbrMode};
    let err = |e: &dyn std::fmt::Debug| format!("The MP3 encoder failed: {e:?}");
    let mut b = Builder::new().ok_or_else(|| "The MP3 encoder could not be started.".to_string())?;
    b.set_num_channels(ch as u8).map_err(|e| err(&e))?;
    b.set_sample_rate(rate).map_err(|e| err(&e))?;
    b.set_output_sample_rate(std::num::NonZeroU32::new(rate)).map_err(|e| err(&e))?;
    b.set_mode(if ch == 1 { Mode::Mono } else { Mode::JointStereo }).map_err(|e| err(&e))?;
    b.set_quality(Quality::NearBest).map_err(|e| err(&e))?;
    match r {
        Mp3Rate::Cbr(k) => {
            let br = match k {
                8 => Bitrate::Kbps8,
                16 => Bitrate::Kbps16,
                24 => Bitrate::Kbps24,
                32 => Bitrate::Kbps32,
                40 => Bitrate::Kbps40,
                48 => Bitrate::Kbps48,
                64 => Bitrate::Kbps64,
                80 => Bitrate::Kbps80,
                96 => Bitrate::Kbps96,
                112 => Bitrate::Kbps112,
                128 => Bitrate::Kbps128,
                160 => Bitrate::Kbps160,
                192 => Bitrate::Kbps192,
                224 => Bitrate::Kbps224,
                256 => Bitrate::Kbps256,
                _ => Bitrate::Kbps320,
            };
            b.set_vbr_mode(VbrMode::Off).map_err(|e| err(&e))?;
            b.set_brate(br).map_err(|e| err(&e))?;
        }
        Mp3Rate::Vbr(q) => {
            let q = match q.min(9) {
                0 => Quality::Best,
                1 => Quality::SecondBest,
                2 => Quality::NearBest,
                3 => Quality::VeryNice,
                4 => Quality::Nice,
                5 => Quality::Good,
                6 => Quality::Decent,
                7 => Quality::Ok,
                8 => Quality::SecondWorst,
                _ => Quality::Worst,
            };
            b.set_vbr_mode(VbrMode::Mtrh).map_err(|e| err(&e))?;
            b.set_vbr_quality(q).map_err(|e| err(&e))?;
        }
    }
    b.set_to_write_vbr_tag(true).map_err(|e| err(&e))?;
    let mut enc = b.build().map_err(|e| err(&e))?;
    use std::io::{Seek, SeekFrom};
    let mut wide = [0i32; 8192];
    let mut narrow = [0i16; 8192];
    let mut encoded = Vec::with_capacity(8192 * 5 / 4 + 7200);
    let mut remaining = frames;
    while remaining > 0 {
        let n = remaining.min(wide.len() / ch as usize);
        let len = n * ch as usize;
        fill(&mut wide[..len])?;
        for (dst, &v) in narrow.iter_mut().zip(&wide[..len]) { *dst = v as i16; }
        encoded.clear();
        if ch == 1 {
            enc.encode_to_vec(MonoPcm(&narrow[..len]), &mut encoded).map_err(|e| err(&e))?;
        } else {
            enc.encode_to_vec(InterleavedPcm(&narrow[..len]), &mut encoded).map_err(|e| err(&e))?;
        }
        file.write_all(&encoded).map_err(|e| io_message(&e))?;
        remaining -= n;
    }
    encoded.clear();
    enc.flush_to_vec::<FlushGap>(&mut encoded).map_err(|e| err(&e))?;
    file.write_all(&encoded).map_err(|e| io_message(&e))?;
    let bytes = file.stream_position().map_err(|e| io_message(&e))?;
    let mut tag = Vec::with_capacity(enc.lame_tag_size().max(1));
    if enc.lame_tag_encode_to_vec(&mut tag).is_none() || tag.is_empty() || tag.len() as u64 > bytes {
        return Err("The MP3 encoder could not finalize its timing header.".into());
    }
    file.seek(SeekFrom::Start(0)).and_then(|_| file.write_all(&tag)).map_err(|e| io_message(&e))
}
/// How a file is written: format (0 FLAC, 1 WAV, 2 AIFF, 3 M4A, 4 MP3), bit depth of the
/// lossless ones (16 or 24), FLAC level, AAC bitrate (bits per second) and MP3 bitrate.
#[derive(Clone, Debug, PartialEq)]
pub struct Encode {
    pub format: u8,
    pub bits: u16,
    pub flac_level: u8,
    pub aac_bps: u32,
    pub mp3: Mp3Rate,
}


/// Keep the output private until the caller has checked it and chosen a free public name.
pub(crate) fn encode_part(
    path: &Path, enc: &Encode, channels: u16, frames: usize, rate: u32, cancel: &AtomicBool,
    mut fill: impl FnMut(&mut [i32]) -> Result<(), String>,
) -> Result<tempfile::NamedTempFile, String> {
    let opt = Options { format: enc.format, rate, aac_bps: enc.aac_bps, flac_level: enc.flac_level };
    encode(path, &opt, cancel, |f, format| {
        let fill = |dst: &mut [i32]| {
            if cancel.load(Ordering::Relaxed) { return Err("stopped".into()); }
            fill(dst)
        };
        if format == 4 {
            write_mp3(f, frames, channels, rate, enc.mp3, fill)
        } else {
            write_pcm(f, format, frames, rate, channels, enc.bits, enc.flac_level, fill)
        }
    })
}

/// A render running on its own thread.
pub struct Job {
    pub rx: crossbeam_channel::Receiver<Result<PathBuf, String>>,
    /// 0..=1000
    pub progress: Arc<AtomicU32>,
    pub cancel: Arc<AtomicBool>,
    pub path: PathBuf,
}

/// Start rendering `score` to `path`. Err at once when it cannot be rendered at all.
pub fn start(audio: Arc<Decoded>, score: Arc<ScoreSet>, path: PathBuf, opt: Options, ctx: Option<eframe::egui::Context>) -> Result<Job, String> {
    if score.len_ms() >= RENDER_MAX_MS {
        return Err(MSG_TOO_LONG.into());
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    let progress = Arc::new(AtomicU32::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (p2, c2, path2) = (progress.clone(), cancel.clone(), path.clone());
    std::thread::spawn(move || {
        let rate = opt.out_rate(audio.sr);
        let frames = (score.len_ms() * 0.001 * rate as f64).round() as usize;
        let mut voice = Voice::new();
        voice.keep_pitch = score.song_grid.is_dynamic();
        voice.audio = Some(audio);
        voice.score = Some(score);
        voice.instant = true;
        voice.playing = true;
        let r = encode(&path2, &opt, &c2, |file, format| {
            let mut samples = [0.0f32; 8192];
            let mut rendered = 0;
            write_blocks(file, format, frames, rate, opt.flac_level, |pcm| {
                if c2.load(Ordering::Relaxed) {
                    return Err(String::new());
                }
                let block = &mut samples[..pcm.len()];
                block.fill(0.0);
                voice.render(block, 2, rate, 1.0);
                for (dst, sample) in pcm.iter_mut().zip(block) {
                    *dst = (*sample * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
                }
                rendered += pcm.len() / 2;
                p2.store((rendered as f64 * 1000.0 / frames.max(1) as f64) as u32, Ordering::Relaxed);
                Ok(())
            })
        }).and_then(|file| {
            if c2.load(Ordering::Relaxed) {
                return Err(String::new());
            }
            file.persist_noclobber(&path2).map_err(|e| io_message(&e.error))?;
            Ok(path2.clone())
        });
        let _ = tx.send(r);
        if let Some(c) = ctx {
            c.request_repaint();
        }
    });
    Ok(Job { rx, progress, cancel, path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit_audio::tests::{beat_tones, score, SR};

    fn write_samples(out: &mut dyn Write, format: u8, rate: u32, level: u8, mut pcm: &[i16]) -> Result<(), String> {
        write_blocks(out, format, pcm.len() / 2, rate, level, |block| {
            let (data, rest) = pcm.split_at(block.len());
            block.copy_from_slice(data);
            pcm = rest;
            Ok(())
        })
    }

    fn encoded_samples(format: u8, pcm: &[i16], rate: u32, level: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_samples(&mut bytes, format, rate, level, pcm).unwrap();
        bytes
    }

    fn pcm_of(path: &Path) -> (u32, Vec<i16>) {
        let d = crate::audio::decode_file(&path.to_string_lossy()).unwrap();
        (d.sr, d.samples.iter().map(|v| (v * 32768.0).round().clamp(-32768.0, 32767.0) as i16).collect())
    }

    fn noise_music(frames: usize) -> Vec<i16> {
        // Two detuned tones per side plus a little noise: something a predictor can work on.
        let mut seed = 12345u32;
        (0..frames)
            .flat_map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let nz = ((seed >> 16) as i32 % 200 - 100) as f64;
                let t = i as f64 / 44100.0;
                let a = 9000.0 * (t * 220.0 * 6.283).sin() + 4000.0 * (t * 3300.0 * 6.283).sin() + nz;
                let b = 8000.0 * (t * 221.0 * 6.283).sin() + 3000.0 * (t * 5100.0 * 6.283).sin() - nz;
                [a as i16, b as i16]
            })
            .collect()
    }

    #[test]
    fn wav_aiff_and_flac_decode_to_the_samples_written() {
        let dir = std::env::temp_dir().join(format!("recordbox_render_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Not a multiple of a block, with full-scale and silent stretches.
        let mut pcm = noise_music(20000);
        pcm.extend(std::iter::repeat([32767i16, -32768]).take(300).flatten());
        pcm.extend(std::iter::repeat(0i16).take(1234));
        for (name, bytes) in [("a.wav", encoded_samples(1, &pcm, 44100, 0)), ("a.aiff", encoded_samples(2, &pcm, 48000, 0))] {
            let f = dir.join(name);
            std::fs::write(&f, bytes).unwrap();
            let (sr, back) = pcm_of(&f);
            assert_eq!(sr, if name == "a.wav" { 44100 } else { 48000 });
            assert_eq!(back, pcm, "{name}");
        }
        let mut sizes = vec![];
        for level in [0u8, 2, 3, 5, 8] {
            let f = dir.join(format!("l{level}.flac"));
            let bytes = encoded_samples(0, &pcm, 44100, level);
            sizes.push(bytes.len());
            std::fs::write(&f, bytes).unwrap();
            let (sr, back) = pcm_of(&f);
            assert_eq!(sr, 44100);
            assert_eq!(back.len(), pcm.len(), "level {level}");
            assert!(back == pcm, "FLAC level {level} is not lossless");
        }
        // Compressed, and a higher level is never larger than the fastest.
        assert!(sizes[0] < pcm.len() * 2, "{sizes:?}");
        assert!(sizes[4] <= sizes[0] && sizes[3] <= sizes[0], "{sizes:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_render_is_the_score_played_through_at_the_chosen_rate() {
        const BEAT: usize = 22050;
        let a = beat_tones(8, &[BEAT]);
        let mut s = score(8, &[BEAT], 120.0);
        assert!(s.delete_range(2.0, 4.0));
        assert!(s.mute_range(4.0, 5.0, true));
        let s = Arc::new(s);
        let (p, c) = (AtomicU32::new(0), AtomicBool::new(false));
        let out = bounce(a.clone(), s.clone(), SR, &p, &c).unwrap();
        // 6 beats at 120 BPM = 3 s, to the frame.
        assert_eq!(out.len(), 3 * SR as usize * 2);
        assert_eq!(p.load(Ordering::Relaxed), 1000);
        // Starts on the file's first sample (no fade-in), beat 2 is song beat 4, beat 4 is
        // muted.
        assert!((out[2 * 100] - a.samples[2 * 100]).abs() < 1e-4);
        let seg = |b: usize| &out[(b * BEAT + 2000) * 2..((b + 1) * BEAT - 2000) * 2];
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        assert!(rms(seg(2)) > 0.3 && rms(seg(4)) < 1e-6 && rms(seg(5)) > 0.3);
        // At 48 kHz the same 3 s are 144000 frames.
        let out48 = bounce(a.clone(), s.clone(), 48000, &p, &c).unwrap();
        assert_eq!(out48.len(), 3 * 48000 * 2);
        // Cancelled: nothing.
        c.store(true, Ordering::Relaxed);
        assert!(bounce(a.clone(), s.clone(), SR, &p, &c).is_none());
        // The whole job, to a WAV file.
        let dir = std::env::temp_dir().join(format!("recordbox_job_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let opt = Options { format: 1, rate: 44100, aac_bps: 192_000, flac_level: 5 };
        let job = start(a.clone(), s.clone(), dir.join("out.wav"), opt.clone(), None).unwrap();
        let path = job.rx.recv().unwrap().unwrap();
        let copy = std::fs::read(&path).unwrap();
        let second = start(a.clone(), s.clone(), path.clone(), opt.clone(), None).unwrap();
        assert!(second.rx.recv().unwrap().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), copy);
        let (sr, back) = pcm_of(&path);
        assert_eq!((sr, back.len()), (44100, 3 * 44100 * 2));
        assert_eq!(back, to_i16(&out));
        for (format, filename) in [(0, "out.flac"), (2, "out.aiff")] {
            let options = Options { format, flac_level: 0, ..opt.clone() };
            let job = start(a.clone(), s.clone(), dir.join(filename), options, None).unwrap();
            let path = job.rx.recv().unwrap().unwrap();
            assert_eq!(pcm_of(&path).1, back, "{filename}");
        }
        // A folder that is not there is reported with rekordbox's message.
        let job = start(a, s, dir.join("missing").join("out.wav"), opt, None).unwrap();
        assert_eq!(job.rx.recv().unwrap(), Err(MSG_NO_DEST.to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_render_can_stop_after_one_block_without_replacing_a_file() {
        let dir = tempfile::tempdir().unwrap();
        for format in [0, 1, 2] {
            let dest = dir.path().join("existing");
            std::fs::write(&dest, b"original recording").unwrap();
            let opt = Options { format, rate: 44100, aac_bps: 192_000, flac_level: 0 };
            let mut blocks = 0;
            let result = encode(&dest, &opt, &AtomicBool::new(false), |file, kind| {
                write_blocks(file, kind, 3 * 3600 * 44100, 44100, 0, |pcm| {
                    assert!(pcm.len() <= 8192);
                    blocks += 1;
                    if blocks == 2 { return Err(String::new()); }
                    pcm.fill(0);
                    Ok(())
                })
            });
            assert!(matches!(result, Err(ref message) if message.is_empty()));
            assert_eq!(blocks, 2);
            assert_eq!(std::fs::read(&dest).unwrap(), b"original recording");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn m4a_is_written_by_the_system_encoder_when_it_is_there() {
        if !Path::new("/usr/bin/afconvert").exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("recordbox_m4a_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pcm = noise_music(44100);
        let f = dir.join("a.m4a");
        let opt = Options { format: 3, rate: 44100, aac_bps: 192_000, flac_level: 5 };
        encode(&f, &opt, &AtomicBool::new(false), |output, format| write_samples(output, format, 44100, 5, &pcm))
            .unwrap().persist_noclobber(&f).unwrap();
        let d = crate::audio::decode_file(&f.to_string_lossy()).unwrap();
        assert_eq!(d.sr, 44100);
        assert!((d.duration() - 1.0).abs() < 0.1, "{}", d.duration());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "the temporary WAV is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
