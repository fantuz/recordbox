//! Recording of the master output (rekordbox 7.2.19's `RecControlPanel` and
//! `rbxfrm::GainAdjustableRecorderUnit`; spec `checks/re-7.2.19/mixer-fx-recording.md` section 4,
//! plus what was traced for this file: the cue sheet text, the silence judge, the normaliser,
//! the tag window's file naming and JUCE's WAV header).
//!
//! The audio callback hands the mix (after the master's effects, before master volume and the
//! limiter: rekordbox's REC OUT) to `Engine::tap`. The tap applies the recording level, keeps
//! the level meters, and writes into a single-producer single-consumer ring without locking,
//! allocating or touching a file. A writer thread takes the samples out of the ring, converts
//! them to 44.1 kHz 16-bit and writes the WAV file.

use std::cell::UnsafeCell;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::{Arc, OnceLock};

/// The file's sample rate: rekordbox records 44.1 kHz 16-bit stereo whatever the device runs
/// at (its `RecInfo` is fixed and the recorder holds a sampling-rate converter).
pub const REC_RATE: u32 = 44100;
/// One file holds three hours at most; a new one starts then.
pub const MAX_SECS: u64 = 10800;
/// With "Detecting audio input." the second before the trigger is kept.
pub const PREROLL_MS: u64 = 1000;
/// rekordbox adds 9 dB at the knob's centre in Performance mode, which undoes the 8.79 dB its
/// TRIM takes off at its centre. recordbox's TRIM is unity at the centre, so what is left of
/// the 9 dB is 0.21 dB.
pub const BASE_DB_PERFORMANCE: f32 = 9.0 - 8.79;
/// Samples of bytes before the audio in the file (JUCE's header: RIFF, JUNK, fmt, data).
pub const WAV_HEADER: u64 = 104;

// ------------------------------------------------------------------------------- hand-off

/// Single-producer single-consumer ring of f32 samples. `push` is for the audio thread,
/// `pop` for the writer thread; neither waits for the other.
pub struct Ring {
    buf: Box<[UnsafeCell<f32>]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// One thread writes the cells between `head` and `tail + len`, the other reads the cells
// between `tail` and `head`; the two ranges never overlap.
unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    pub fn new(min: usize) -> Ring {
        let n = min.max(2).next_power_of_two();
        Ring { buf: (0..n).map(|_| UnsafeCell::new(0.0)).collect(), mask: n - 1, head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }
    pub fn len(&self) -> usize {
        self.head.load(Acquire).wrapping_sub(self.tail.load(Acquire))
    }
    /// All of `src` or nothing (false: no room).
    pub fn push(&self, src: &[f32]) -> bool {
        let head = self.head.load(Relaxed);
        let used = head.wrapping_sub(self.tail.load(Acquire));
        if self.buf.len() - used < src.len() {
            return false;
        }
        for (i, v) in src.iter().enumerate() {
            unsafe { *self.buf[(head.wrapping_add(i)) & self.mask].get() = *v };
        }
        self.head.store(head.wrapping_add(src.len()), Release);
        true
    }
    /// Up to `dst.len()` samples; returns how many.
    pub fn pop(&self, dst: &mut [f32]) -> usize {
        let tail = self.tail.load(Relaxed);
        let n = self.head.load(Acquire).wrapping_sub(tail).min(dst.len());
        for (i, d) in dst.iter_mut().take(n).enumerate() {
            *d = unsafe { *self.buf[(tail.wrapping_add(i)) & self.mask].get() };
        }
        self.tail.store(tail.wrapping_add(n), Release);
        n
    }
    /// Throw away what is in it (writer side, between recordings).
    pub fn clear(&self) {
        self.tail.store(self.head.load(Acquire), Release);
    }
}

/// What only the audio thread touches.
struct AudioSide {
    /// The last second (pre-roll), circular, interleaved stereo.
    pre: Vec<f32>,
    pre_pos: usize,
    pre_len: usize,
    was_recording: bool,
    gain: f32,
    rate: u32,
}

/// Three readers take the level meters: the panel, the standby trigger and Preferences.
pub const METER_PANEL: usize = 0;
pub const METER_TRIGGER: usize = 1;
pub const METER_PREFS: usize = 2;

pub struct Engine {
    pub ring: Ring,
    /// The tap writes into the ring.
    recording: AtomicBool,
    /// The tap keeps the last second while not recording ("Detecting audio input.").
    preroll: AtomicBool,
    /// The tap is inside its block (the writer waits for it before its last read).
    busy: AtomicBool,
    gain_bits: AtomicU32,
    /// Frames given to the ring since the recording started, at the device's rate.
    frames: AtomicU64,
    /// Frames that found the ring full.
    pub dropped: AtomicU64,
    rate: AtomicU32,
    peaks: [[AtomicU32; 2]; 3],
    audio: UnsafeCell<AudioSide>,
}

// `audio` belongs to the one thread that calls `tap`.
unsafe impl Sync for Engine {}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// The app's recorder (made on first use, from the interface's thread).
pub fn engine() -> &'static Engine {
    ENGINE.get_or_init(Engine::new)
}

/// The master volume and mute, taken off the decks for the length of one audio callback so
/// that the mix reaches the REC tap before them, as in rekordbox's master section
/// (`sum -> master FX -> [REC OUT] -> MasterVolume -> Limitter`).
#[derive(Clone, Copy)]
pub struct MasterLift {
    saved: [(f32, bool); 4],
    pub gain: f32,
}

pub fn lift_master(decks: &mut [Option<parking_lot::MutexGuard<'_, crate::audio::PlayState>>; 4]) -> MasterLift {
    let mut lift = MasterLift { saved: [(1.0, false); 4], gain: f32::NAN };
    for (i, g) in decks.iter_mut().enumerate() {
        if let Some(s) = g.as_deref_mut() {
            lift.saved[i] = (s.volume, s.muted);
            if lift.gain.is_nan() {
                lift.gain = if s.muted { 0.0 } else { s.volume };
            }
            s.volume = 1.0;
            s.muted = false;
        }
    }
    if lift.gain.is_nan() {
        lift.gain = 1.0;
    }
    lift
}

pub fn restore_master(decks: &mut [Option<parking_lot::MutexGuard<'_, crate::audio::PlayState>>; 4], lift: MasterLift) {
    for (i, g) in decks.iter_mut().enumerate() {
        if let Some(s) = g.as_deref_mut() {
            (s.volume, s.muted) = lift.saved[i];
        }
    }
}

/// The audio callback's REC tap, then the master volume.
pub fn tap(out: &mut [f32], channels: usize, rate: u32, lift: MasterLift) {
    if let Some(e) = ENGINE.get() {
        e.tap(out, channels, rate);
    }
    if lift.gain != 1.0 {
        for v in out.iter_mut() {
            *v *= lift.gain;
        }
    }
}

impl Engine {
    pub fn new() -> Engine {
        Engine {
            // About six seconds at 44.1 kHz, three at 96 kHz.
            ring: Ring::new(1 << 19),
            recording: AtomicBool::new(false),
            preroll: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
            frames: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            rate: AtomicU32::new(REC_RATE),
            peaks: std::array::from_fn(|_| [AtomicU32::new(0), AtomicU32::new(0)]),
            // One second at up to 192 kHz.
            audio: UnsafeCell::new(AudioSide { pre: vec![0.0; 192_000 * 2], pre_pos: 0, pre_len: 0, was_recording: false, gain: 1.0, rate: 0 }),
        }
    }

    /// Called from the audio callback with the mix before master volume. No locks, no
    /// allocation, no file access.
    pub fn tap(&self, mix: &[f32], channels: usize, rate: u32) {
        let ch = channels.max(1);
        let frames = mix.len() / ch;
        if frames == 0 {
            return;
        }
        // Two output callbacks can briefly overlap during device changes. Only one
        // producer may mutably access the pre-roll buffer and feed the recorder ring.
        if self.busy.compare_exchange(false, true, SeqCst, SeqCst).is_err() {
            if self.recording.load(SeqCst) {
                self.dropped.fetch_add(frames as u64, Relaxed);
            }
            return;
        }
        let recording = self.recording.load(SeqCst);
        let preroll = self.preroll.load(Relaxed);
        self.rate.store(rate, Relaxed);
        let a = unsafe { &mut *self.audio.get() };
        let target = f32::from_bits(self.gain_bits.load(Relaxed));
        let step = (target - a.gain) / frames as f32;
        if a.rate != rate {
            (a.rate, a.pre_len, a.pre_pos) = (rate, 0, 0);
        }
        let pre_cap = ((rate as u64 * PREROLL_MS / 1000) as usize * 2).min(a.pre.len());
        if recording && !a.was_recording {
            // The first block of a recording: the kept second goes in ahead of it.
            let mut sent = 0u64;
            if preroll && a.pre_len > 0 && pre_cap > 0 {
                let n = a.pre_len.min(pre_cap);
                let start = (a.pre_pos + pre_cap - n) % pre_cap;
                let first = (pre_cap - start).min(n);
                if self.ring.push(&a.pre[start..start + first]) && (n == first || self.ring.push(&a.pre[..n - first])) {
                    sent = (n / 2) as u64;
                }
            }
            a.pre_len = 0;
            a.pre_pos = 0;
            self.frames.store(sent, Relaxed);
        }
        a.was_recording = recording;
        let mut peak = [0f32; 2];
        let mut chunk = [0f32; 512];
        let mut fi = 0;
        while fi < frames {
            let n = (frames - fi).min(256);
            for k in 0..n {
                let f = &mix[(fi + k) * ch..];
                let (l, r) = (f[0], if ch >= 2 { f[1] } else { f[0] });
                a.gain += step;
                let (l, r) = (l * a.gain, r * a.gain);
                peak[0] = peak[0].max(l.abs());
                peak[1] = peak[1].max(r.abs());
                chunk[k * 2] = l;
                chunk[k * 2 + 1] = r;
            }
            if recording {
                if self.ring.push(&chunk[..n * 2]) {
                    self.frames.fetch_add(n as u64, Relaxed);
                } else {
                    self.dropped.fetch_add(n as u64, Relaxed);
                }
            } else if preroll && pre_cap > 0 {
                for v in &chunk[..n * 2] {
                    a.pre[a.pre_pos] = *v;
                    a.pre_pos = (a.pre_pos + 1) % pre_cap;
                }
                a.pre_len = (a.pre_len + n * 2).min(pre_cap);
            }
            fi += n;
        }
        a.gain = target;
        if !preroll && !recording {
            a.pre_len = 0;
        }
        for set in &self.peaks {
            for c in 0..2 {
                if peak[c] > f32::from_bits(set[c].load(Relaxed)) {
                    set[c].store(peak[c].to_bits(), Relaxed);
                }
            }
        }
        self.busy.store(false, SeqCst);
    }

    /// Recording level as a gain (the knob: base ±15 dB).
    pub fn set_gain(&self, gain: f32) {
        self.gain_bits.store(gain.to_bits(), Relaxed);
    }
    pub fn set_preroll(&self, on: bool) {
        self.preroll.store(on, Relaxed);
    }
    pub fn is_recording(&self) -> bool {
        self.recording.load(SeqCst)
    }
    /// Highest level since this reader last asked (L, R), after the recording level.
    pub fn take_peak(&self, reader: usize) -> [f32; 2] {
        let p = &self.peaks[reader.min(2)];
        [f32::from_bits(p[0].swap(0, Relaxed)), f32::from_bits(p[1].swap(0, Relaxed))]
    }
    /// Seconds recorded so far.
    pub fn elapsed(&self) -> f64 {
        self.frames.load(Relaxed) as f64 / self.rate.load(Relaxed).max(1) as f64
    }
    pub fn elapsed_ms(&self) -> i64 {
        (self.elapsed() * 1000.0) as i64
    }
    pub fn device_rate(&self) -> u32 {
        self.rate.load(Relaxed)
    }
}

// ------------------------------------------------------------------------- level and gain

/// The level knob (-1 = fully left, 0 = centre, +1 = fully right) to a gain: the base level
/// ±15 dB, `10^(dB/20)` (`RecControlPanel::sliderValueChanged`).
pub fn knob_gain(knob: f32, base_db: f32) -> f32 {
    10f32.powf((base_db + 15.0 * knob.clamp(-1.0, 1.0)) / 20.0)
}

/// The dB steps of the panel's level indicator (`djplay::rekordboxRec_dB_table`, 89 values:
/// coarse below -40 dB, 1 dB steps to -17, then two, three, four and five segments per dB up
/// to 0, and 0, 0, 1, 1, 2, 3 at the top).
pub fn meter_table() -> Vec<i32> {
    let mut t = vec![-100, -80, -70, -60, -55, -50, -45, -40];
    t.extend(-39..=-17);
    for (range, n) in [(-16..=-14, 2), (-13..=-6, 3), (-5..=-3, 4), (-2..=-1, 5)] {
        for db in range {
            t.extend(std::iter::repeat(db).take(n));
        }
    }
    t.extend([0, 0, 1, 1, 2, 3]);
    t
}

/// How many of the indicator's 88 segments are lit at `db` (`djplay::LevelMeter::draw`: the
/// number of table values below the level).
pub fn meter_segments(db: f32) -> usize {
    static T: OnceLock<Vec<i32>> = OnceLock::new();
    let t = T.get_or_init(meter_table);
    let level = db.round() as i32;
    let i = t.iter().take_while(|v| **v < level).count();
    ((88.0 * i as f32 / (t.len() - 1) as f32).round() as usize).min(88)
}

pub fn to_db(x: f32) -> f32 {
    if x <= 1e-10 { -200.0 } else { 20.0 * x.log10() }
}

// ----------------------------------------------------------------------------- silence

/// What the silence judge reports after a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Judged {
    Nothing,
    /// Sound came back after a long enough silence: a new track starts at this time (ms).
    Split(f64),
    /// Silence has lasted the auto-stop time.
    Stop,
}

/// rekordbox's `AudioRecorderUnit::AutoTrackSplitJudge` (`processBlock` 0x102911420): the
/// level of every audio block is averaged over half a second; a window at or below the
/// threshold is silent. Silence longer than `silence_ms` ends the current track; from then,
/// sound that lasts longer than five seconds starts the next one (at the time that sound
/// began), and silence that reaches `stop_ms` in all stops the recording.
pub struct SilenceJudge {
    rate: f64,
    pub threshold: f32,
    window: u32,
    silence: u32,
    content: u32,
    stop: u32,
    content_ms: f64,
    /// 0 = sound, 1 = silence, 2 = stopped.
    state: u8,
    silent: bool,
    run: u32,
    sum: f32,
    count: u32,
    frames: u32,
    pub split: bool,
    pub auto_stop: bool,
}

impl SilenceJudge {
    pub fn new(rate: u32, threshold_db: f64, silence_ms: f64, split: bool, auto_stop: bool) -> SilenceJudge {
        let r = rate as f64 * 0.001;
        let silence_ms = silence_ms.max(0.0);
        SilenceJudge {
            rate: rate as f64,
            threshold: 10f64.powf(threshold_db.min(0.0) * 0.05) as f32,
            window: (r * 500.0) as u32,
            silence: (r * silence_ms) as u32,
            content: (r * 5000.0) as u32,
            stop: (r * silence_ms.max(20000.0)) as u32,
            content_ms: 5000.0,
            state: 1,
            silent: true,
            run: 0,
            sum: 0.0,
            count: 0,
            frames: 0,
            split,
            auto_stop,
        }
    }

    /// One audio block: its level (linear peak), its length in frames, and the frames
    /// recorded up to and including it.
    pub fn block(&mut self, level: f32, frames: u32, recorded: u64) -> Judged {
        if !self.split && !self.auto_stop {
            return Judged::Nothing;
        }
        self.sum += level;
        self.count += 1;
        self.frames += frames;
        if self.frames < self.window {
            return Judged::Nothing;
        }
        let silent = self.sum / self.count as f32 <= self.threshold;
        if silent == self.silent {
            self.run += self.frames;
        } else {
            self.silent = silent;
            self.run = self.frames;
        }
        (self.sum, self.count, self.frames) = (0.0, 0, 0);
        match self.state {
            0 => {
                if silent && self.run > self.silence {
                    self.state = 1;
                }
                Judged::Nothing
            }
            1 if silent => {
                if self.run > self.stop {
                    self.state = 2;
                    if self.auto_stop {
                        return Judged::Stop;
                    }
                }
                Judged::Nothing
            }
            1 => {
                if self.run <= self.content {
                    return Judged::Nothing;
                }
                self.state = 0;
                let at = recorded as f64 * 1000.0 / self.rate - (self.content_ms + 500.0);
                if self.split && at > 0.0 { Judged::Split(at) } else { Judged::Nothing }
            }
            _ => Judged::Nothing,
        }
    }
}

// ----------------------------------------------------------------------------- resampling

/// Streaming sample-rate converter (windowed sinc, 32 taps), device rate to the file's.
pub struct Resampler {
    step: f64,
    pos: f64,
    hist: Vec<f32>,
    table: Vec<f32>,
    bypass: bool,
}

const RS_HALF: usize = 16;
const RS_OVER: usize = 256;

impl Resampler {
    pub fn new(from: u32, to: u32) -> Resampler {
        let step = from as f64 / to as f64;
        // Low-pass at the lower of the two Nyquist frequencies.
        let cut = (1.0 / step).min(1.0) * 0.97;
        let n = RS_HALF * RS_OVER;
        let bessel = |x: f64| {
            let (mut s, mut t, mut k) = (1.0, 1.0, 1.0);
            while t > 1e-12 * s {
                t *= (x / (2.0 * k)) * (x / (2.0 * k));
                s += t;
                k += 1.0;
            }
            s
        };
        let beta = 9.0;
        let table = (0..=n)
            .map(|i| {
                let x = i as f64 / RS_OVER as f64;
                let w = bessel(beta * (1.0 - (x / RS_HALF as f64).powi(2)).max(0.0).sqrt()) / bessel(beta);
                let s = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * x * cut).sin() / (std::f64::consts::PI * x * cut) };
                (s * cut * w) as f32
            })
            .collect();
        Resampler { step, pos: 0.0, hist: vec![0.0; RS_HALF * 2 * 2], table, bypass: from == to }
    }

    /// Interleaved stereo in, interleaved stereo appended to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.bypass {
            out.extend_from_slice(input);
            return;
        }
        self.hist.extend_from_slice(input);
        let frames = self.hist.len() / 2;
        // `pos` is where the next output frame lies, in input frames from the start of `hist`
        // less the RS_HALF frames of history kept before it.
        while self.pos + (2 * RS_HALF) as f64 <= (frames - 1) as f64 {
            let centre = self.pos + RS_HALF as f64;
            let i0 = centre.floor() as usize;
            let frac = centre - i0 as f64;
            let (mut l, mut r) = (0f32, 0f32);
            for k in 0..2 * RS_HALF {
                let idx = i0 + k + 1 - RS_HALF;
                let d = ((k as f64 + 1.0 - RS_HALF as f64) - frac).abs() * RS_OVER as f64;
                let ti = d as usize;
                let w = if ti >= self.table.len() - 1 { 0.0 } else { self.table[ti] + (self.table[ti + 1] - self.table[ti]) * (d - ti as f64) as f32 };
                l += self.hist[idx * 2] * w;
                r += self.hist[idx * 2 + 1] * w;
            }
            out.push(l);
            out.push(r);
            self.pos += self.step;
        }
        // Keep what the next call still reads.
        let drop = (self.pos.floor() as usize).min(frames);
        self.hist.drain(..drop * 2);
        self.pos -= drop as f64;
    }
}

// ------------------------------------------------------------------------------- the file

/// JUCE's float to 16-bit conversion (to 32 bits, rounded, then the top 16 bits).
pub fn to_i16(x: f32) -> i16 {
    (((x.clamp(-1.0, 1.0) as f64 * 2147483647.0).round() as i32) >> 16) as i16
}

/// The 104 bytes rekordbox's (JUCE's) WAV files start with: RIFF, a 52-byte JUNK chunk kept
/// free for a 64-bit header, a 16-byte PCM format chunk, and the data chunk's header.
pub fn wav_header(data_bytes: u64, rate: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(WAV_HEADER as usize);
    let padded = data_bytes + (data_bytes & 1);
    h.extend(b"RIFF");
    h.extend(((padded + 96).min(u32::MAX as u64) as u32).to_le_bytes());
    h.extend(b"WAVE");
    h.extend(b"JUNK");
    h.extend(52u32.to_le_bytes());
    h.extend([0u8; 52]);
    h.extend(b"fmt ");
    h.extend(16u32.to_le_bytes());
    h.extend(1u16.to_le_bytes());
    h.extend(2u16.to_le_bytes());
    h.extend(rate.to_le_bytes());
    h.extend((rate * 4).to_le_bytes());
    h.extend(4u16.to_le_bytes());
    h.extend(16u16.to_le_bytes());
    h.extend(b"data");
    h.extend((data_bytes.min(u32::MAX as u64) as u32).to_le_bytes());
    h
}

/// 16-bit stereo WAV writer; the sizes in the header are filled in by `finish`.
pub struct WavWriter {
    file: std::io::BufWriter<std::fs::File>,
    pub frames: u64,
    rate: u32,
}

impl WavWriter {
    pub fn create(path: &Path, rate: u32) -> std::io::Result<WavWriter> {
        Self::from_file(std::fs::File::create_new(path)?, rate)
    }
    fn from_file(file: std::fs::File, rate: u32) -> std::io::Result<Self> {
        let mut file = std::io::BufWriter::with_capacity(1 << 16, file);
        file.write_all(&wav_header(0, rate))?;
        Ok(WavWriter { file, frames: 0, rate })
    }
    pub fn write(&mut self, interleaved: &[f32]) -> std::io::Result<()> {
        let mut bytes = [0u8; 4096];
        for part in interleaved.chunks(2048) {
            for (i, s) in part.iter().enumerate() {
                bytes[i * 2..i * 2 + 2].copy_from_slice(&to_i16(*s).to_le_bytes());
            }
            self.file.write_all(&bytes[..part.len() * 2])?;
        }
        self.frames += (interleaved.len() / 2) as u64;
        Ok(())
    }
    pub fn finish(mut self) -> std::io::Result<u64> {
        self.file.flush()?;
        let mut f = self.file.into_inner().map_err(|e| e.into_error())?;
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&wav_header(self.frames * 4, self.rate))?;
        f.sync_all()?;
        Ok(self.frames)
    }
}

/// A WAV file's format and where its audio is: (rate, channels, bits, data offset, data bytes).
pub fn wav_info(path: &Path) -> std::io::Result<(u32, u16, u16, u64, u64)> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut head = [0u8; 12];
    f.read_exact(&mut head)?;
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not a WAV file");
    if &head[..4] != b"RIFF" || &head[8..] != b"WAVE" {
        return Err(bad());
    }
    let (mut rate, mut ch, mut bits) = (0u32, 0u16, 0u16);
    let mut pos = 12u64;
    while pos + 8 <= len {
        let mut ck = [0u8; 8];
        f.seek(SeekFrom::Start(pos))?;
        f.read_exact(&mut ck)?;
        let size = u32::from_le_bytes([ck[4], ck[5], ck[6], ck[7]]) as u64;
        match &ck[..4] {
            b"fmt " => {
                let mut b = [0u8; 16];
                f.read_exact(&mut b)?;
                ch = u16::from_le_bytes([b[2], b[3]]);
                rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                bits = u16::from_le_bytes([b[14], b[15]]);
            }
            b"data" => return Ok((rate, ch, bits, pos + 8, size.min(len - pos - 8))),
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    Err(bad())
}

/// Read `frames` 16-bit stereo frames starting at frame `start` (fewer at the end).
fn read_frames(f: &mut std::fs::File, data: u64, start: u64, frames: usize, out: &mut Vec<f32>) -> std::io::Result<usize> {
    let mut bytes = vec![0u8; frames * 4];
    f.seek(SeekFrom::Start(data + start * 4))?;
    let mut got = 0;
    while got < bytes.len() {
        let n = f.read(&mut bytes[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    out.clear();
    out.extend(bytes[..got - got % 4].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0));
    Ok(got / 4)
}

/// `REC-YYYY-MM-DD.wav` in `dir`; when a file whose name contains that stem is there already
/// (a finished one or one still being recorded), `REC-YYYY-MM-DD(2)`, `(3)` ...
/// (`RecControlPanel::startRecording`).
pub fn next_rec_path(dir: &Path, date: &str) -> PathBuf {
    let names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter(|e| e.path().is_file()).filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string())).collect())
        .unwrap_or_default();
    let base = format!("REC-{date}");
    let mut stem = base.clone();
    let mut n = 2;
    while names.iter().any(|s| s.contains(&stem)) {
        stem = format!("{base}({n})");
        n += 1;
    }
    dir.join(format!("{stem}.wav"))
}

/// The name a file carries while it is being recorded: `<stem>.rbrec`.
pub fn recording_name(wav: &Path) -> PathBuf {
    wav.with_extension("rbrec")
}

/// Seconds that can still be recorded: the rest of the three hours, or what the disk holds
/// (`RecControlPanel::timerCallback`).
pub fn remaining_secs(elapsed: u64, free_bytes: Option<u64>) -> u64 {
    let by_time = MAX_SECS.saturating_sub(elapsed);
    match free_bytes {
        Some(b) => by_time.min(b.saturating_sub(DISK_RESERVE) / (REC_RATE as u64 * 4)),
        None => by_time,
    }
}

/// Free space the recorder leaves alone.
pub const DISK_RESERVE: u64 = 20 << 20;

pub fn hms(secs: u64) -> String {
    format!("{:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

// ------------------------------------------------------------------------------ the writer

#[derive(Clone, Debug, PartialEq)]
pub enum RecError {
    /// The save folder is missing or the file cannot be made.
    Folder,
    /// Writing failed.
    Write,
    DiskFull,
}

/// What a finished recording hands back.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The finished file (`.wav`; the `.rbrec` name is gone).
    pub file: PathBuf,
    /// Its length in 44.1 kHz frames.
    pub frames: u64,
    /// Times (ms) where a new track starts, when auto split is on.
    pub splits: Vec<f64>,
    /// It ended by itself after 20 seconds of silence.
    pub auto_stopped: bool,
    pub error: Option<RecError>,
    /// Frames lost because the writer did not keep up (0 unless the disk stalls).
    pub dropped: u64,
}

impl Recorded {
    pub fn length_ms(&self) -> f64 {
        self.frames as f64 * 1000.0 / REC_RATE as f64
    }
}

/// Settings of one recording.
#[derive(Clone, Debug)]
pub struct RecSettings {
    pub silence_db: f64,
    pub silence_ms: f64,
    pub auto_split: bool,
    pub auto_stop: bool,
}

/// A recording in progress: the writer thread and what it shares with the interface.
pub struct Session {
    pub wav: PathBuf,
    stop: Arc<AtomicBool>,
    /// The writer has seen the end coming by itself (silence, disk full, write error).
    pub ended: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Recorded>>,
}

impl Session {
    /// Start recording into `wav` (written under its `.rbrec` name until it is finished).
    pub fn start_on(engine: &'static Engine, wav: PathBuf, settings: RecSettings) -> Result<Session, RecError> {
        let tmp = recording_name(&wav);
        let writer = WavWriter::create(&tmp, REC_RATE).map_err(|_| RecError::Folder)?;
        let stop = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        engine.ring.clear();
        engine.dropped.store(0, Relaxed);
        engine.frames.store(0, Relaxed);
        let (stop2, ended2, wav2) = (stop.clone(), ended.clone(), wav.clone());
        let handle = std::thread::Builder::new()
            .name("REC Thread".into())
            .spawn(move || write_loop(engine, writer, tmp, wav2, settings, stop2, ended2))
            .map_err(|_| RecError::Folder)?;
        engine.recording.store(true, SeqCst);
        Ok(Session { wav, stop, ended, handle: Some(handle) })
    }

    /// Ask the writer to finish (it writes what is still in the ring first).
    pub fn request_stop(&self, engine: &Engine) {
        engine.recording.store(false, SeqCst);
        self.stop.store(true, SeqCst);
    }

    pub fn finished(&self) -> bool {
        self.handle.as_ref().map(|h| h.is_finished()).unwrap_or(true)
    }

    /// Stop and wait for the file.
    pub fn finish(mut self, engine: &Engine) -> Recorded {
        self.request_stop(engine);
        let wav = self.wav.clone();
        self.handle.take().and_then(|h| h.join().ok()).unwrap_or(Recorded { file: wav, frames: 0, splits: vec![], auto_stopped: false, error: Some(RecError::Write), dropped: 0 })
    }
}

fn write_loop(engine: &Engine, mut writer: WavWriter, tmp: PathBuf, wav: PathBuf, s: RecSettings, stop: Arc<AtomicBool>, ended: Arc<AtomicBool>) -> Recorded {
    let mut buf = vec![0f32; 16384];
    let mut conv: Vec<f32> = Vec::with_capacity(32768);
    let mut rate = engine.rate.load(Relaxed).max(1);
    let mut rs = Resampler::new(rate, REC_RATE);
    let mut judge = SilenceJudge::new(rate, s.silence_db, s.silence_ms, s.auto_split, s.auto_stop);
    let mut splits = vec![];
    let (mut auto_stopped, mut error) = (false, None);
    let mut taken: u64 = 0;
    let mut last_disk = std::time::Instant::now();
    let dir = wav.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    // The judge sees blocks of 128 frames (rekordbox's default audio buffer).
    const BLOCK: usize = 128;
    let (mut blk_peak, mut blk_n) = (0f32, 0usize);
    let mut last = false;
    loop {
        let n = engine.ring.pop(&mut buf);
        if n == 0 {
            if last {
                break;
            }
            if stop.load(SeqCst) || ended.load(SeqCst) {
                // The tap may still be inside a block it started while recording was on.
                if !engine.busy.load(SeqCst) {
                    last = true;
                }
                std::thread::sleep(std::time::Duration::from_micros(500));
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
            continue;
        }
        let now = engine.rate.load(Relaxed).max(1);
        if now != rate {
            rate = now;
            rs = Resampler::new(rate, REC_RATE);
        }
        if error.is_none() {
            for f in buf[..n].chunks_exact(2) {
                blk_peak = blk_peak.max(f[0].abs()).max(f[1].abs());
                blk_n += 1;
                taken += 1;
                if blk_n == BLOCK {
                    match judge.block(blk_peak, BLOCK as u32, taken) {
                        Judged::Split(ms) => splits.push(ms),
                        Judged::Stop if !auto_stopped => {
                            auto_stopped = true;
                            engine.recording.store(false, SeqCst);
                            ended.store(true, SeqCst);
                        }
                        _ => {}
                    }
                    (blk_peak, blk_n) = (0.0, 0);
                }
            }
            conv.clear();
            rs.process(&buf[..n], &mut conv);
            if writer.write(&conv).is_err() {
                error = Some(RecError::Write);
                engine.recording.store(false, SeqCst);
                ended.store(true, SeqCst);
            }
        }
        if error.is_none() && last_disk.elapsed().as_millis() >= 1000 {
            last_disk = std::time::Instant::now();
            if !dir.is_dir() {
                error = Some(RecError::Write);
            } else if crate::export::usb::free_space(&dir).map(|b| b < DISK_RESERVE).unwrap_or(false) {
                error = Some(RecError::DiskFull);
            }
            if error.is_some() {
                engine.recording.store(false, SeqCst);
                ended.store(true, SeqCst);
            }
        }
    }
    let dropped = engine.dropped.load(Relaxed);
    let frames = match writer.finish() {
        Ok(f) => f,
        Err(_) => {
            error.get_or_insert(RecError::Write);
            0
        }
    };
    // The `.rbrec` name comes off once the file is whole.
    let file = match move_recording(&tmp, &wav) {
        Ok(()) => wav,
        Err(_) => {
            error.get_or_insert(RecError::Write);
            tmp
        }
    };
    Recorded { file, frames, splits, auto_stopped, error, dropped }
}

// ------------------------------------------------------------------------- after recording

/// Move a completed recording without overwriting another file. On failure the original
/// path still belongs to the user (including a recoverable .rbrec).
pub fn move_recording(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut path = tempfile::TempPath::try_from_path(from.to_path_buf())?;
    path.disable_cleanup(true);
    path.persist_noclobber(to).map_err(|e| e.error)
}

/// Lengths (ms) of the files a recording of `total_ms` separates into at `splits`.
pub fn split_lengths(total_ms: f64, splits: &[f64]) -> Vec<f64> {
    let mut out = vec![];
    let mut at = 0.0;
    for s in splits.iter().copied().filter(|s| *s > 0.0 && *s < total_ms) {
        if s > at {
            out.push(s - at);
            at = s;
        }
    }
    out.push(total_ms - at);
    out
}

/// `<stem>-NN<ext>` beside `file` (`RecInfo::TrackSplit::getFileAppendedIndex`).
pub fn split_name(file: &Path, n: usize) -> PathBuf {
    let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = file.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    file.with_file_name(format!("{stem}-{n:02}{ext}"))
}

/// Separate a recording into files of `lengths_ms` (the last takes what is left); the whole
/// file is removed when every part was written. `progress`: 0..1000.
pub fn split_file(file: &Path, lengths_ms: &[f64], progress: &AtomicU32) -> std::io::Result<Vec<(PathBuf, u64)>> {
    let (rate, _, _, data, bytes) = wav_info(file)?;
    if lengths_ms.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "no split lengths"));
    }
    let total = bytes / 4;
    let mut f = std::fs::File::open(file)?;
    let mut out = vec![];
    let mut created = vec![];
    let mut start = 0u64;
    let mut buf = vec![];
    for (i, ms) in lengths_ms.iter().enumerate() {
        let len = if i + 1 == lengths_ms.len() { total - start } else { ((ms * 0.001 * rate as f64) as u64).min(total - start) };
        let path = split_name(file, i + 1);
        if path.exists() {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "a separated file is there already"));
        }
        let part = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
        let mut w = WavWriter::from_file(part.reopen()?, rate)?;
        let mut done = 0u64;
        while done < len {
            let n = read_frames(&mut f, data, start + done, (len - done).min(1 << 16) as usize, &mut buf)?;
            if n == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
            }
            w.write(&buf)?;
            done += n as u64;
            progress.store((((start + done) as f64 / total.max(1) as f64) * 1000.0) as u32, Relaxed);
        }
        let frames = w.finish()?;
        part.persist_noclobber(&path)?;
        created.push(tempfile::TempPath::try_from_path(path.clone())?);
        out.push((path, frames));
        start += len;
    }
    drop(f);
    std::fs::remove_file(file)?;
    for mut path in created {
        path.disable_cleanup(true);
    }
    Ok(out)
}

/// One K-weighted channel of the loudness measurement (BS.1770 as libebur128 sets it up).
struct KFilter {
    b: [f64; 5],
    a: [f64; 5],
    v: [[f64; 5]; 2],
}

impl KFilter {
    fn new(rate: f64) -> KFilter {
        let k = (std::f64::consts::PI * 1681.974450955533 / rate).tan();
        let (vh, q) = (10f64.powf(3.999843853973347 / 20.0), 0.7071752369554196);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        let pb = [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0];
        let pa = [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0];
        let k = (std::f64::consts::PI * 38.13547087602444 / rate).tan();
        let q = 0.5003270373238773;
        let rb = [1.0, -2.0, 1.0];
        let ra = [1.0, 2.0 * (k * k - 1.0) / (1.0 + k / q + k * k), (1.0 - k / q + k * k) / (1.0 + k / q + k * k)];
        KFilter {
            b: [pb[0] * rb[0], pb[0] * rb[1] + pb[1] * rb[0], pb[0] * rb[2] + pb[1] * rb[1] + pb[2] * rb[0], pb[1] * rb[2] + pb[2] * rb[1], pb[2] * rb[2]],
            a: [pa[0] * ra[0], pa[0] * ra[1] + pa[1] * ra[0], pa[0] * ra[2] + pa[1] * ra[1] + pa[2] * ra[0], pa[1] * ra[2] + pa[2] * ra[1], pa[2] * ra[2]],
            v: [[0.0; 5]; 2],
        }
    }
    fn run(&mut self, c: usize, x: f64) -> f64 {
        let v = &mut self.v[c];
        v[0] = x - self.a[1] * v[1] - self.a[2] * v[2] - self.a[3] * v[3] - self.a[4] * v[4];
        let y = self.b[0] * v[0] + self.b[1] * v[1] + self.b[2] * v[2] + self.b[3] * v[3] + self.b[4] * v[4];
        v[4] = v[3];
        v[3] = v[2];
        v[2] = v[1];
        v[1] = v[0];
        y
    }
}

/// Integrated loudness (EBU R128: 400 ms windows every 100 ms, absolute gate -70 LUFS,
/// relative gate -10 LU) and sample peak, fed in pieces so a three-hour file never has to be
/// in memory.
pub struct Loudness {
    k: KFilter,
    step: usize,
    acc: f64,
    n: usize,
    steps: std::collections::VecDeque<f64>,
    blocks: Vec<f64>,
    pub peak: f32,
}

impl Loudness {
    pub fn new(rate: u32) -> Loudness {
        Loudness { k: KFilter::new(rate as f64), step: (rate as usize + 5) / 10, acc: 0.0, n: 0, steps: Default::default(), blocks: vec![], peak: 0.0 }
    }
    pub fn feed(&mut self, interleaved: &[f32]) {
        for f in interleaved.chunks_exact(2) {
            self.peak = self.peak.max(f[0].abs()).max(f[1].abs());
            let (l, r) = (self.k.run(0, f[0] as f64), self.k.run(1, f[1] as f64));
            self.acc += l * l + r * r;
            self.n += 1;
            if self.n == self.step {
                self.steps.push_back(self.acc);
                (self.acc, self.n) = (0.0, 0);
                if self.steps.len() == 4 {
                    self.blocks.push(self.steps.iter().sum::<f64>() / (4 * self.step) as f64);
                    self.steps.pop_front();
                }
            }
        }
    }
    pub fn lufs(&self) -> f64 {
        let lk = |z: f64| -0.691 + 10.0 * z.max(1e-300).log10();
        let abs: Vec<f64> = self.blocks.iter().copied().filter(|z| lk(*z) >= -70.0).collect();
        if abs.is_empty() {
            return f64::NEG_INFINITY;
        }
        let mean = abs.iter().sum::<f64>() / abs.len() as f64;
        let gated: Vec<f64> = abs.into_iter().filter(|z| *z >= 0.1 * mean).collect();
        if gated.is_empty() { lk(mean) } else { lk(gated.iter().sum::<f64>() / gated.len() as f64) }
    }
}

/// The gain "Automatically optimize audio level" applies (`RecLevelNormalizeThread`): the
/// Auto Gain for the file's loudness (`target - loudness` dB), lowered to `1 / peak` when it
/// would take the peak over full scale. 1.0 for silence.
pub fn normalize_gain(lufs: f64, peak: f32, target_lufs: f64) -> f32 {
    if !lufs.is_finite() || peak <= 0.0 {
        return 1.0;
    }
    let g = 10f64.powf((target_lufs - lufs) * 0.05) as f32;
    if (peak * g).abs() > 1.0 { 1.0 / peak } else { g }
}

/// Rewrite the recording at its optimized level, staging the whole output before replacing it.
/// Returns the gain applied. `progress`: 0..1000 (the first half is the measurement).
pub fn normalize_file(file: &Path, target_lufs: f64, progress: &AtomicU32) -> std::io::Result<f32> {
    let (rate, _, _, data, bytes) = wav_info(file)?;
    let total = bytes / 4;
    let mut f = std::fs::File::open(file)?;
    let mut buf = vec![];
    let mut loud = Loudness::new(rate);
    let mut at = 0u64;
    while at < total {
        let n = read_frames(&mut f, data, at, (total - at).min(1 << 16) as usize, &mut buf)?;
        if n == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        loud.feed(&buf);
        at += n as u64;
        progress.store((at as f64 / total.max(1) as f64 * 500.0) as u32, Relaxed);
    }
    let gain = normalize_gain(loud.lufs(), loud.peak, target_lufs);
    let tmp = tempfile::NamedTempFile::new_in(file.parent().unwrap_or(Path::new(".")))?;
    let mut w = WavWriter::from_file(tmp.reopen()?, rate)?;
    let mut at = 0u64;
    while at < total {
        let n = read_frames(&mut f, data, at, (total - at).min(1 << 16) as usize, &mut buf)?;
        if n == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        for v in &mut buf {
            *v *= gain;
        }
        w.write(&buf)?;
        at += n as u64;
        progress.store(500 + (at as f64 / total.max(1) as f64 * 500.0) as u32, Relaxed);
    }
    w.finish()?;
    tmp.as_file().set_permissions(f.metadata()?.permissions())?;
    drop(f);
    tmp.persist(file)?;
    Ok(gain)
}

// --------------------------------------------------------------------------------- tags

/// What the tag information window collects for one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagInfo {
    pub artist: String,
    pub album: String,
    pub title: String,
    pub track_no: u32,
}

fn clean_name(s: &str) -> String {
    let t: String = s.trim_end().chars().map(|c| if matches!(c, '/' | ':' | '\\' | '\0') { '_' } else { c }).collect();
    t.trim_start_matches('.').to_string()
}

/// Where a tagged recording goes: `<folder>/<artist>/<album>/NN <title>.wav`, with
/// "Unknown Artist", "Unknown Album" and "No Title" for what is empty
/// (`RecTrackInfoInputComponent::finish`).
pub fn tagged_path(folder: &Path, info: &TagInfo, ext: &str) -> PathBuf {
    let or = |s: &str, d: &str| if clean_name(s).is_empty() { d.to_string() } else { clean_name(s) };
    folder.join(or(&info.artist, "Unknown Artist")).join(or(&info.album, "Unknown Album")).join(format!("{:02} {}.{ext}", info.track_no, or(&info.title, "No Title")))
}

fn id3_frame(id: &[u8; 4], text: &str) -> Vec<u8> {
    let mut f = Vec::new();
    let body_len = 1 + text.len();
    f.extend(id);
    // ID3v2.4: frame sizes are syncsafe.
    f.extend([(body_len >> 21) as u8 & 0x7F, (body_len >> 14) as u8 & 0x7F, (body_len >> 7) as u8 & 0x7F, body_len as u8 & 0x7F]);
    f.extend([0u8, 0]);
    f.push(3);
    f.extend(text.as_bytes());
    f
}

/// Write title, artist, album and track number into a WAV file, as an `ID3 ` chunk and a
/// `LIST`/`INFO` chunk after the audio (the two tags TagLib writes for rekordbox). Tags the
/// file already carries are replaced.
pub fn write_tags(file: &Path, info: &TagInfo) -> std::io::Result<()> {
    let (_, _, _, data, bytes) = wav_info(file)?;
    let end = data + bytes + (bytes & 1);
    let mut frames = Vec::new();
    for (id, text) in [(b"TIT2", info.title.clone()), (b"TPE1", info.artist.clone()), (b"TALB", info.album.clone()), (b"TRCK", if info.track_no > 0 { info.track_no.to_string() } else { String::new() })] {
        if !text.is_empty() {
            frames.extend(id3_frame(id, &text));
        }
    }
    let mut tail = Vec::new();
    if !frames.is_empty() {
        let n = frames.len();
        let mut id3 = b"ID3\x04\x00\x00".to_vec();
        id3.extend([(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]);
        id3.extend(frames);
        tail.extend(b"ID3 ");
        tail.extend((id3.len() as u32).to_le_bytes());
        tail.extend(&id3);
        if id3.len() & 1 == 1 {
            tail.push(0);
        }
        let mut list = b"INFO".to_vec();
        for (id, text) in [(b"INAM", info.title.clone()), (b"IART", info.artist.clone()), (b"IPRD", info.album.clone()), (b"IPRT", if info.track_no > 0 { info.track_no.to_string() } else { String::new() })] {
            if !text.is_empty() {
                let mut b = text.into_bytes();
                b.push(0);
                list.extend(id);
                list.extend((b.len() as u32).to_le_bytes());
                let odd = b.len() & 1 == 1;
                list.extend(b);
                if odd {
                    list.push(0);
                }
            }
        }
        tail.extend(b"LIST");
        tail.extend((list.len() as u32).to_le_bytes());
        tail.extend(list);
    }
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(file)?;
    f.set_len(end)?;
    f.seek(SeekFrom::Start(end))?;
    f.write_all(&tail)?;
    let size = (end + tail.len() as u64 - 8).min(u32::MAX as u64) as u32;
    f.seek(SeekFrom::Start(4))?;
    f.write_all(&size.to_le_bytes())?;
    f.sync_all()
}

/// The tags of a file `write_tags` wrote (title, artist, album, track number).
pub fn read_tags(file: &Path) -> TagInfo {
    let mut info = TagInfo::default();
    let Ok((_, _, _, data, bytes)) = wav_info(file) else { return info };
    let Ok(all) = std::fs::read(file).map(|v| v[((data + bytes + (bytes & 1)) as usize).min(v.len())..].to_vec()) else { return info };
    let mut pos = 0;
    while pos + 8 <= all.len() {
        let size = u32::from_le_bytes([all[pos + 4], all[pos + 5], all[pos + 6], all[pos + 7]]) as usize;
        let body = &all[pos + 8..(pos + 8 + size).min(all.len())];
        if &all[pos..pos + 4] == b"LIST" && body.len() >= 4 && &body[..4] == b"INFO" {
            let mut p = 4;
            while p + 8 <= body.len() {
                let n = u32::from_le_bytes([body[p + 4], body[p + 5], body[p + 6], body[p + 7]]) as usize;
                let text = String::from_utf8_lossy(&body[p + 8..(p + 8 + n).min(body.len())]).trim_end_matches('\0').to_string();
                match &body[p..p + 4] {
                    b"INAM" => info.title = text,
                    b"IART" => info.artist = text,
                    b"IPRD" => info.album = text,
                    b"IPRT" => info.track_no = text.parse().unwrap_or(0),
                    _ => {}
                }
                p += 8 + n + (n & 1);
            }
        }
        pos += 8 + size + (size & 1);
    }
    info
}

// ----------------------------------------------------------------------------- cue sheet

/// A deck's track that went on air during the recording.
#[derive(Clone, Debug, PartialEq)]
pub struct CueTrack {
    pub title: String,
    pub performer: String,
    /// The track's file (full path, as rekordbox writes it).
    pub file: String,
    /// When it went on air, in ms from the start of the recording.
    pub at_ms: i64,
}

/// The heading of a cue sheet.
#[derive(Clone, Debug, Default)]
pub struct CueHead {
    pub genre: String,
    pub date: String,
    pub comment: String,
    pub recorded_by: String,
    pub title: String,
    pub performer: String,
    /// The recording's file name.
    pub file: String,
}

/// JUCE's `String::quoted`: quotes go around the text unless it has them already.
fn quoted(s: &str) -> String {
    let mut t = s.to_string();
    if !t.starts_with('"') {
        t.insert(0, '"');
    }
    if !t.ends_with('"') || t.len() == 1 {
        t.push('"');
    }
    t
}

/// The text of a cue sheet exactly as rekordbox's `CueSheet::format` (0x10081f414) lays it
/// out: CRLF line ends, tabs before the track lines, quoted text, and `INDEX 01` as
/// hours:minutes:seconds. Tracks that went on air from `start_ms` and before the end of the
/// file are listed; their numbers are their places in the whole list. Times are counted from
/// `start_ms` (rekordbox leaves them counted from the start of the whole recording, which
/// puts them past the end of a separated file).
pub fn cue_text(head: &CueHead, tracks: &[CueTrack], start_ms: i64, length_ms: i64) -> String {
    let mut s = String::new();
    let mut line = |t: String| {
        s.push_str(&t);
        s.push_str("\r\n");
    };
    if !head.genre.is_empty() {
        line(format!("REM GENRE {}", quoted(&head.genre)));
    }
    if !head.date.is_empty() {
        line(format!("REM DATE {}", head.date));
    }
    if !head.comment.is_empty() {
        line(format!("REM COMMENT {}", quoted(&head.comment)));
    }
    if !head.recorded_by.is_empty() {
        line(format!("REM RECORDED_BY {}", quoted(&head.recorded_by)));
    }
    if !head.title.is_empty() {
        line(format!("TITLE {}", quoted(&head.title)));
    }
    if !head.performer.is_empty() {
        line(format!("PERFORMER {}", quoted(&head.performer)));
    }
    line(format!("FILE {} WAVE", quoted(&head.file)));
    let first = start_ms.max(0) / 1000;
    let end = if length_ms < 0 { -1 } else { (start_ms + length_ms) / 1000 };
    for (i, t) in tracks.iter().enumerate() {
        let sec = t.at_ms / 1000;
        if first > sec || !(end < 1 || end > sec) {
            continue;
        }
        line(format!("\tTRACK {:02} AUDIO", i + 1));
        if !t.title.is_empty() {
            line(format!("\t\tTITLE {}", quoted(&t.title)));
        }
        if !t.performer.is_empty() {
            line(format!("\t\tPERFORMER {}", quoted(&t.performer)));
        }
        if !t.file.is_empty() {
            line(format!("\t\tFILE {} WAVE", quoted(&t.file)));
        }
        let rel = ((t.at_ms - start_ms.max(0)) as f64 * 0.001) as i64;
        line(format!("\t\tINDEX 01 {:02}:{:02}:{:02}", rel / 3600, rel % 3600 / 60, rel % 3600 % 60));
    }
    s
}

/// Write a new cue sheet beside the recording, preserving any existing cue sheet.
pub fn write_cue(wav: &Path, text: &str) -> std::io::Result<PathBuf> {
    let p = wav.with_extension("cue");
    let mut tmp = tempfile::NamedTempFile::new_in(p.parent().unwrap_or(Path::new(".")))?;
    tmp.write_all(text.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(&p)?;
    Ok(p)
}

/// Collects the tracks that go on air while a recording runs (`CueSheetCreator`, an
/// `OnAirWatcher` that looks at the mixer every 500 ms from the start of the recording).
#[derive(Default)]
pub struct OnAirLog {
    was: [bool; 4],
    pub tracks: Vec<CueTrack>,
}

impl OnAirLog {
    /// One look at the decks: `on_air[d]` and the track loaded on deck d (title, artist,
    /// path), at `at_ms` of the recording. A deck that has just come on air adds its track
    /// unless the last track added is the same file.
    pub fn poll(&mut self, on_air: [bool; 4], loaded: &[Option<(String, String, String)>; 4], at_ms: i64) {
        for d in 0..4 {
            if on_air[d] && !self.was[d] {
                if let Some((title, performer, file)) = &loaded[d] {
                    if self.tracks.last().map(|t| &t.file) != Some(file) {
                        self.tracks.push(CueTrack { title: title.clone(), performer: performer.clone(), file: file.clone(), at_ms });
                    }
                }
            }
            self.was[d] = on_air[d];
        }
    }
    pub fn clear(&mut self) {
        *self = OnAirLog::default();
    }
}

/// rekordbox's on-air rule for a mixer channel (`DjMixerUnit::isOnAir` 0x102aa9a00; spec 1.9).
/// Positions are 0..1 as the controls hold them. `assign`: 0 THRU, 1 A, 2 B. `curve`: the
/// channel fader curve (0 exponential, 1 linear, 2 logarithmic).
pub fn on_air(master: f32, crossfader: f32, assign: u8, fader: f32, curve: u8, isolator: bool, eq: [f32; 3], trim: f32) -> bool {
    let xf = crossfader * 1023.0;
    let fader_min = match curve {
        0 => 23.0,
        1 => 5.0,
        _ => 6.0,
    };
    master * 1023.0 >= 23.0
        && match assign {
            1 => xf < 1020.0,
            2 => xf > 3.0,
            _ => true,
        }
        && fader * 1023.0 >= fader_min
        && !(isolator && eq.iter().all(|p| p * 255.0 < 3.0))
        && trim * 255.0 > 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of the test's own, removed again when the test ends.
    struct Tmp(PathBuf);
    impl std::ops::Deref for Tmp {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(name: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("recordbox-rec-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }

    fn leak() -> &'static Engine {
        Box::leak(Box::new(Engine::new()))
    }

    fn settings() -> RecSettings {
        RecSettings { silence_db: -38.0, silence_ms: 3000.0, auto_split: false, auto_stop: false }
    }

    /// A counting ramp that is exact in 16 bits: frame i holds (i mod 20000) / 32768 on the
    /// left and its negative on the right.
    fn ramp(from: usize, frames: usize) -> Vec<f32> {
        (from..from + frames).flat_map(|i| { let v = (i % 20000) as f32 / 32768.0; [v, -v] }).collect()
    }

    fn read_all(p: &Path) -> (u32, Vec<i16>) {
        let (rate, ch, bits, data, bytes) = wav_info(p).unwrap();
        assert_eq!((ch, bits), (2, 16));
        let all = std::fs::read(p).unwrap();
        (rate, all[data as usize..(data + bytes) as usize].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
    }

    #[test]
    fn wav_header_is_juces() {
        let h = wav_header(4000, 44100);
        assert_eq!(h.len() as u64, WAV_HEADER);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 4000 + 96);
        assert_eq!(&h[8..16], b"WAVEJUNK");
        assert_eq!(u32::from_le_bytes(h[16..20].try_into().unwrap()), 52);
        assert!(h[20..72].iter().all(|b| *b == 0));
        assert_eq!(&h[72..76], b"fmt ");
        // PCM, 2 channels, 44100 Hz, 176400 bytes a second, 4 bytes a frame, 16 bits.
        assert_eq!(&h[76..96], &[16, 0, 0, 0, 1, 0, 2, 0, 0x44, 0xAC, 0, 0, 0x10, 0xB1, 2, 0, 4, 0, 16, 0]);
        assert_eq!(&h[96..100], b"data");
        assert_eq!(u32::from_le_bytes(h[100..104].try_into().unwrap()), 4000);
        // The whole file is the RIFF size plus its own eight bytes.
        assert_eq!(4000 + 96 + 8, 4000 + WAV_HEADER);
    }

    #[test]
    fn samples_are_written_as_juce_converts_them() {
        assert_eq!(to_i16(1.0), 32767);
        assert_eq!(to_i16(2.0), 32767);
        assert_eq!(to_i16(-1.0), -32768);
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(to_i16(0.5), 16384);
        assert_eq!(to_i16(-0.5), -16384);
        assert_eq!(to_i16(100.0 / 32768.0), 100);
        assert_eq!(to_i16(20000.0 / 32768.0), 19999, "the top 16 of 32 bits: large values land one below");
        let d = tmp("wav");
        let p = d.join("a.wav");
        let mut w = WavWriter::create(&p, 44100).unwrap();
        w.write(&[0.25, -0.25, 1.0, -1.0]).unwrap();
        assert_eq!(w.finish().unwrap(), 2);
        let (rate, s) = read_all(&p);
        assert_eq!(rate, 44100);
        assert_eq!(s, vec![8192, -8192, 32767, -32768]);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), WAV_HEADER + 8);
    }

    #[test]
    fn names_count_up_past_finished_and_unfinished_files() {
        let d = tmp("names");
        assert_eq!(next_rec_path(&d, "2026-11-27").file_name().unwrap(), "REC-2026-11-27.wav");
        std::fs::write(d.join("REC-2026-11-27.wav"), b"x").unwrap();
        assert_eq!(next_rec_path(&d, "2026-11-27").file_name().unwrap(), "REC-2026-11-27(2).wav");
        // A recording in progress holds its name too.
        std::fs::write(d.join("REC-2026-11-27(2).rbrec"), b"x").unwrap();
        assert_eq!(next_rec_path(&d, "2026-11-27").file_name().unwrap(), "REC-2026-11-27(3).wav");
        // Another day starts again.
        assert_eq!(next_rec_path(&d, "2026-11-28").file_name().unwrap(), "REC-2026-11-28.wav");
        // Separated files ("REC-...-01.wav") contain the stem as well.
        let e = tmp("names2");
        std::fs::write(e.join("REC-2026-11-27-01.wav"), b"x").unwrap();
        assert_eq!(next_rec_path(&e, "2026-11-27").file_name().unwrap(), "REC-2026-11-27(2).wav");
        assert_eq!(recording_name(&d.join("REC-2026-11-27(3).wav")).file_name().unwrap(), "REC-2026-11-27(3).rbrec");
    }

    #[test]
    fn finalizing_a_take_preserves_both_files_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        let take = dir.path().join("take.rbrec");
        let dest = dir.path().join("take.wav");
        std::fs::write(&take, b"new recording").unwrap();
        std::fs::write(&dest, b"existing recording").unwrap();
        assert!(move_recording(&take, &dest).is_err());
        assert_eq!(std::fs::read(&take).unwrap(), b"new recording");
        assert_eq!(std::fs::read(&dest).unwrap(), b"existing recording");
    }

    #[test]
    fn level_knob_is_plus_minus_fifteen_db() {
        assert!((knob_gain(0.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((to_db(knob_gain(1.0, 0.0)) - 15.0).abs() < 1e-4);
        assert!((to_db(knob_gain(-1.0, 0.0)) + 15.0).abs() < 1e-4);
        assert!((to_db(knob_gain(0.5, 0.0)) - 7.5).abs() < 1e-4);
        assert!((to_db(knob_gain(3.0, 0.0)) - 15.0).abs() < 1e-4, "clamped");
        assert!((to_db(knob_gain(0.0, BASE_DB_PERFORMANCE)) - 0.21).abs() < 1e-4);
    }

    #[test]
    fn level_indicator_table() {
        let t = meter_table();
        assert_eq!(t.len(), 89);
        assert_eq!(&t[..9], &[-100, -80, -70, -60, -55, -50, -45, -40, -39]);
        assert_eq!(&t[83..], &[0, 0, 1, 1, 2, 3]);
        assert_eq!(t.iter().filter(|v| **v == -13).count(), 3);
        assert_eq!(t.iter().filter(|v| **v == -1).count(), 5);
        assert_eq!(meter_segments(-200.0), 0);
        assert_eq!(meter_segments(-100.0), 0);
        assert_eq!(meter_segments(-40.0), 7);
        assert_eq!(meter_segments(0.0), 83);
        assert_eq!(meter_segments(3.0), 88);
        assert_eq!(meter_segments(20.0), 88);
        assert!(meter_segments(-6.0) > meter_segments(-12.0));
    }

    /// Run a whole recording through an engine of its own: `feed` gives the tap its blocks.
    fn record(dir: &Path, s: RecSettings, rate: u32, preroll: bool, feed: impl FnOnce(&'static Engine, &dyn Fn())) -> Recorded {
        let e = leak();
        e.rate.store(rate, Relaxed);
        e.set_preroll(preroll);
        let wav = next_rec_path(dir, "2026-11-27");
        let cell: std::cell::RefCell<Option<Session>> = Default::default();
        let start = || {
            *cell.borrow_mut() = Some(Session::start_on(e, wav.clone(), s.clone()).unwrap());
            assert!(recording_name(&wav).exists(), "the .rbrec file is there while recording");
            assert!(!wav.exists());
        };
        feed(e, &start);
        let r = cell.borrow_mut().take().unwrap().finish(e);
        assert!(!recording_name(&r.file).exists(), "the .rbrec name is gone");
        r
    }

    #[test]
    fn nothing_is_lost_or_changed_between_the_callback_and_the_file() {
        // Small and large audio buffers, and one size that is not a multiple of anything.
        for block in [16usize, 64, 480, 2048, 4096, 1000] {
            let d = tmp(&format!("handoff{block}"));
            let total = 44100 * 2 + 333;
            let r = record(&d, settings(), 44100, false, |e, start| {
                start();
                let mut at = 0;
                while at < total {
                    let n = block.min(total - at);
                    e.tap(&ramp(at, n), 2, 44100);
                    at += n;
                    // The callback runs faster than real time here; let the writer keep up.
                    while e.ring.len() > e.ring.capacity() / 2 {
                        std::thread::yield_now();
                    }
                }
            });
            assert_eq!(r.error, None);
            assert_eq!(r.dropped, 0);
            assert_eq!(r.frames, total as u64, "block {block}");
            let (rate, s) = read_all(&r.file);
            assert_eq!(rate, 44100);
            assert_eq!(s.len(), total * 2);
            for i in 0..total {
                let v = (i % 20000) as i32;
                // (JUCE's conversion lands on the value or one below it.)
                assert!((s[i * 2] as i32 - v).abs() <= 1 && (s[i * 2 + 1] as i32 + v).abs() <= 1, "block {block} frame {i}: {} {}", s[i * 2], s[i * 2 + 1]);
            }
            assert_eq!(std::fs::metadata(&r.file).unwrap().len(), WAV_HEADER + total as u64 * 4);
        }
    }

    #[test]
    fn a_full_ring_drops_whole_blocks_and_counts_them() {
        let e = Engine::new();
        e.recording.store(true, SeqCst);
        let block = vec![0.5f32; 512];
        let cap = e.ring.capacity();
        let mut sent = 0;
        while sent + 512 <= cap + 4096 {
            e.tap(&block, 2, 44100);
            sent += 512;
        }
        assert_eq!(e.ring.len(), cap);
        assert_eq!(e.dropped.load(Relaxed), 4096 / 2);
        assert_eq!(e.frames.load(Relaxed), (cap / 2) as u64);
    }

    #[test]
    fn the_second_before_the_trigger_is_kept() {
        let d = tmp("preroll");
        let rate = 44100usize;
        // 2.5 s pass in standby; the recording starts there. The file holds the last second
        // of that (frames 66150..110250) and then what follows.
        let r = record(&d, settings(), 44100, true, |e, start| {
            let mut at = 0;
            while at < rate * 5 / 2 {
                e.tap(&ramp(at, 441), 2, 44100);
                at += 441;
            }
            assert_eq!(e.ring.len(), 0, "nothing reaches the ring in standby");
            start();
            for _ in 0..50 {
                e.tap(&ramp(at, 441), 2, 44100);
                at += 441;
            }
            assert!((e.elapsed() - 1.5).abs() < 0.02, "elapsed counts the kept second: {}", e.elapsed());
        });
        assert_eq!(r.frames as usize, rate + 50 * 441);
        let (_, s) = read_all(&r.file);
        let first = rate * 5 / 2 - rate;
        for i in [0usize, 1, 500, rate - 1, rate, rate + 1000] {
            let v = ((first + i) % 20000) as i32;
            assert!((s[i * 2] as i32 - v).abs() <= 1, "frame {i}: {} for {v}", s[i * 2]);
        }
        // Without "Detecting audio input." nothing from before the start is kept.
        let d = tmp("nopreroll");
        let r = record(&d, settings(), 44100, false, |e, start| {
            for k in 0..100 {
                e.tap(&ramp(k * 441, 441), 2, 44100);
            }
            start();
            e.tap(&ramp(44100, 441), 2, 44100);
        });
        assert_eq!(r.frames, 441);
        assert_eq!(read_all(&r.file).1[0], (44100 % 20000) as i16);
    }

    #[test]
    fn the_level_reaches_the_file_and_the_meters() {
        let d = tmp("gain");
        let r = record(&d, settings(), 44100, false, |e, start| {
            e.set_gain(knob_gain(-1.0, 0.0));
            // (The level glides to a new value over one block.)
            e.tap(&vec![0.5f32; 882], 2, 44100);
            let _ = e.take_peak(METER_PANEL);
            start();
            for _ in 0..10 {
                e.tap(&vec![0.5f32; 882], 2, 44100);
            }
            let p = e.take_peak(METER_PANEL);
            assert!((to_db(p[0]) - (to_db(0.5) - 15.0)).abs() < 0.01, "{}", to_db(p[0]));
            assert_eq!(e.take_peak(METER_PANEL), [0.0, 0.0], "read and cleared");
            assert!(e.take_peak(METER_TRIGGER)[1] > 0.0, "each reader has its own");
        });
        let (_, s) = read_all(&r.file);
        let want = 0.5 * 10f32.powf(-15.0 / 20.0) * 32768.0;
        assert!(s.iter().all(|v| (*v as f32 - want).abs() < 1.5), "{} for {want}", s[100]);
    }

    #[test]
    fn mono_and_wider_outputs_record_as_stereo() {
        let e = Engine::new();
        e.recording.store(true, SeqCst);
        e.tap(&[0.25, 0.5], 1, 44100);
        let mut out = [0f32; 4];
        assert_eq!(e.ring.pop(&mut out), 4);
        assert_eq!(out, [0.25, 0.25, 0.5, 0.5]);
        e.tap(&[0.1, 0.2, 0.9, 0.9], 4, 44100);
        assert_eq!(e.ring.pop(&mut out), 2);
        assert_eq!(&out[..2], &[0.1, 0.2]);
    }

    #[test]
    fn the_master_volume_comes_after_the_tap() {
        use crate::audio::PlayState;
        let decks: Vec<parking_lot::Mutex<PlayState>> = (0..4).map(|_| { let mut s = PlayState::default(); s.volume = 0.25; parking_lot::Mutex::new(s) }).collect();
        decks[1].lock().muted = true;
        let mut guards = [Some(decks[0].lock()), Some(decks[1].lock()), None, Some(decks[3].lock())];
        let lift = lift_master(&mut guards);
        assert_eq!(lift.gain, 0.25);
        assert!(guards.iter().flatten().all(|g| g.volume == 1.0 && !g.muted), "the decks render at full level");
        restore_master(&mut guards, lift);
        assert_eq!((guards[0].as_ref().unwrap().volume, guards[1].as_ref().unwrap().muted), (0.25, true));
        let mut out = [1.0f32, -1.0];
        tap(&mut out, 2, 44100, lift);
        assert_eq!(out, [0.25, -0.25]);
        // Muted: the output is silent, whatever the tap was given.
        drop(guards);
        let mut g = [Some(decks[1].lock()), None, None, None];
        assert_eq!(lift_master(&mut g).gain, 0.0);
    }

    #[test]
    fn silence_judge_stops_after_twenty_seconds_and_marks_new_tracks() {
        let rate = 44100u32;
        // Blocks of 128 frames at a steady level for `secs`; returns what the judge said.
        let run = |j: &mut SilenceJudge, at: &mut u64, level: f32, secs: f64| -> Vec<(f64, Judged)> {
            let mut out = vec![];
            let end = *at + (secs * rate as f64) as u64;
            while *at < end {
                *at += 128;
                let r = j.block(level, 128, *at);
                if r != Judged::Nothing {
                    out.push((*at as f64 / rate as f64, r));
                }
            }
            out
        };
        let loud = 0.5;
        let quiet = 10f32.powf(-50.0 / 20.0);
        // Auto stop: 60 s of music, then silence. It stops 20 s (and one half-second window) in.
        let mut j = SilenceJudge::new(rate, -38.0, 3000.0, false, true);
        let mut at = 0;
        assert!(run(&mut j, &mut at, loud, 60.0).is_empty());
        let r = run(&mut j, &mut at, quiet, 30.0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, Judged::Stop);
        assert!((r[0].0 - 80.5).abs() < 0.6, "stopped at {}", r[0].0);
        // 19 s of silence is not enough.
        let mut j = SilenceJudge::new(rate, -38.0, 3000.0, false, true);
        let mut at = 0;
        run(&mut j, &mut at, loud, 30.0);
        assert!(run(&mut j, &mut at, quiet, 19.0).is_empty());
        assert!(run(&mut j, &mut at, loud, 30.0).is_empty());
        // Nothing ever played: it stops after 20 s too.
        let mut j = SilenceJudge::new(rate, -38.0, 3000.0, false, true);
        let mut at = 0;
        assert_eq!(run(&mut j, &mut at, 0.0, 25.0).len(), 1);
        // Auto split: music, 4 s of silence (longer than the 3 s setting), music again. The
        // new track starts where the sound came back.
        let mut j = SilenceJudge::new(rate, -38.0, 3000.0, true, false);
        let mut at = 0;
        run(&mut j, &mut at, loud, 30.0);
        assert!(run(&mut j, &mut at, quiet, 4.0).is_empty());
        let r = run(&mut j, &mut at, loud, 20.0);
        assert_eq!(r.len(), 1);
        match r[0].1 {
            // (Half a second before the first half-second window that had sound in it.)
            Judged::Split(ms) => assert!((32900.0..=33600.0).contains(&ms), "split at {ms}"),
            other => panic!("{other:?}"),
        }
        // A 2 s gap (shorter than the setting) does not split; a 1 s setting does.
        let mut j = SilenceJudge::new(rate, -38.0, 3000.0, true, false);
        let mut at = 0;
        run(&mut j, &mut at, loud, 30.0);
        run(&mut j, &mut at, quiet, 2.0);
        assert!(run(&mut j, &mut at, loud, 20.0).is_empty());
        let mut j = SilenceJudge::new(rate, -38.0, 1000.0, true, false);
        let mut at = 0;
        run(&mut j, &mut at, loud, 30.0);
        run(&mut j, &mut at, quiet, 2.0);
        assert_eq!(run(&mut j, &mut at, loud, 20.0).len(), 1);
        // The threshold decides what silence is: -50 dB is sound for a -60 dB threshold.
        let mut j = SilenceJudge::new(rate, -60.0, 3000.0, false, true);
        let mut at = 0;
        assert!(run(&mut j, &mut at, quiet, 60.0).is_empty());
        // With neither setting on it does nothing.
        let mut j = SilenceJudge::new(rate, -38.0, 3000.0, false, false);
        let mut at = 0;
        assert!(run(&mut j, &mut at, 0.0, 60.0).is_empty());
    }

    #[test]
    fn a_recording_stops_itself_on_silence_and_reports_its_split() {
        let d = tmp("autostop");
        let s = RecSettings { silence_db: -38.0, silence_ms: 1000.0, auto_split: true, auto_stop: true };
        let block = |level: f32| vec![level; 1024];
        let r = record(&d, s, 44100, false, |e, start| {
            start();
            let mut feed = |level: f32, secs: f64| {
                for _ in 0..(secs * 44100.0 / 512.0) as usize {
                    if !e.is_recording() {
                        return false;
                    }
                    e.tap(&block(level), 2, 44100);
                    while e.ring.len() > 1 << 17 {
                        std::thread::yield_now();
                    }
                }
                true
            };
            assert!(feed(0.5, 8.0));
            assert!(feed(0.0, 3.0));
            assert!(feed(0.5, 8.0));
            // 25 s of silence: the writer ends the recording by itself on the way.
            let mut still = true;
            for _ in 0..50 {
                still = feed(0.0, 0.5);
                if !still {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!still, "the recording was stopped by the silence");
        });
        assert!(r.auto_stopped);
        assert_eq!(r.splits.len(), 1, "{:?}", r.splits);
        assert!((9900.0..=10600.0).contains(&r.splits[0]), "{:?}", r.splits);
        let secs = r.frames as f64 / 44100.0;
        assert!((39.0..41.5).contains(&secs), "8 + 3 + 8 s and 20.5 s of silence: {secs}");
    }

    #[test]
    fn split_lengths_and_files() {
        assert_eq!(split_lengths(10000.0, &[]), vec![10000.0]);
        assert_eq!(split_lengths(10000.0, &[4000.0, 7000.0]), vec![4000.0, 3000.0, 3000.0]);
        assert_eq!(split_lengths(10000.0, &[4000.0, 12000.0, -5.0]), vec![4000.0, 6000.0]);
        let d = tmp("split");
        let p = d.join("REC-2026-11-27.wav");
        let mut w = WavWriter::create(&p, 44100).unwrap();
        w.write(&ramp(0, 44100 * 3)).unwrap();
        w.finish().unwrap();
        let prog = AtomicU32::new(0);
        let taken = split_name(&p, 2);
        std::fs::write(&taken, b"existing audio").unwrap();
        assert!(split_file(&p, &[1000.0, 2000.0], &prog).is_err());
        assert!(p.exists() && !split_name(&p, 1).exists());
        assert_eq!(std::fs::read(&taken).unwrap(), b"existing audio");
        std::fs::remove_file(taken).unwrap();
        let parts = split_file(&p, &split_lengths(3000.0, &[1000.0, 2500.0]), &prog).unwrap();
        assert_eq!(parts.iter().map(|x| x.0.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>(), vec!["REC-2026-11-27-01.wav", "REC-2026-11-27-02.wav", "REC-2026-11-27-03.wav"]);
        assert_eq!(parts.iter().map(|x| x.1).collect::<Vec<_>>(), vec![44100, 66150, 22050]);
        assert!(!p.exists(), "the whole file is removed");
        assert_eq!(prog.load(Relaxed), 1000);
        // The parts carry on where the one before stopped.
        let (_, b) = read_all(&parts[1].0);
        assert_eq!(b[0], (44100 % 20000) as i16);
        let (_, c) = read_all(&parts[2].0);
        assert_eq!(c[0], ((44100 + 66150) % 20000) as i16);
        assert_eq!(c.len(), 22050 * 2);
    }

    fn tone(rate: u32, secs: f64, amp: f32) -> Vec<f32> {
        (0..(rate as f64 * secs) as usize).flat_map(|i| { let v = amp * (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / rate as f32).sin(); [v, v] }).collect()
    }

    #[test]
    fn loudness_is_bs1770() {
        // A 1 kHz tone at -20 dBFS on both channels is -20 LUFS by definition.
        let x = tone(44100, 12.0, 0.1);
        let mut l = Loudness::new(44100);
        for part in x.chunks(3000) {
            l.feed(part);
        }
        assert!((l.lufs() + 20.0).abs() < 0.05, "{}", l.lufs());
        let mut l48 = Loudness::new(48000);
        l48.feed(&tone(48000, 12.0, 0.1));
        assert!((l48.lufs() + 20.0).abs() < 0.05, "{}", l48.lufs());
        // (The library analysis's own filter reads the same tone 0.26 dB lower.)
        assert!((crate::analysis::r128_integrated(&x, 44100.0) - l.lufs()).abs() < 0.4);
        assert!((l.peak - 0.1).abs() < 1e-3);
        assert!(Loudness::new(44100).lufs().is_infinite());
    }

    #[test]
    fn normalise_brings_the_file_to_the_auto_gain_level_without_clipping() {
        assert_eq!(normalize_gain(f64::NEG_INFINITY, 0.0, -10.0), 1.0);
        assert!((to_db(normalize_gain(-20.0, 0.1, -10.0)) - 10.0).abs() < 1e-4);
        // A gain that would take the peak past full scale gives way to 1 / peak.
        assert!((normalize_gain(-20.0, 0.5, -10.0) - 2.0).abs() < 1e-6);
        assert!((to_db(normalize_gain(-6.0, 0.9, -10.0)) + 4.0).abs() < 1e-4, "quieter, too");
        let d = tmp("norm");
        let p = d.join("REC-2026-11-27.wav");
        let mut w = WavWriter::create(&p, 44100).unwrap();
        w.write(&tone(44100, 6.0, 0.05)).unwrap();
        w.finish().unwrap();
        let before = { let mut l = Loudness::new(44100); l.feed(&tone(44100, 6.0, 0.05)); l.lufs() };
        let prog = AtomicU32::new(0);
        let g = normalize_file(&p, -10.0, &prog).unwrap();
        assert!((to_db(g) - (-10.0 - before) as f32).abs() < 0.1, "gain {} dB for {before} LUFS", to_db(g));
        assert_eq!(prog.load(Relaxed), 1000);
        assert!(!d.join("temp.wav").exists());
        let (_, s) = read_all(&p);
        assert_eq!(s.len(), 44100 * 6 * 2);
        let x: Vec<f32> = s.iter().map(|v| *v as f32 / 32768.0).collect();
        let mut l = Loudness::new(44100);
        l.feed(&x);
        assert!((l.lufs() + 10.0).abs() < 0.1, "now {} LUFS", l.lufs());
        // A loud file is limited by its peak: full scale, not past it.
        let p = d.join("loud.wav");
        let mut w = WavWriter::create(&p, 44100).unwrap();
        w.write(&tone(44100, 6.0, 0.5)).unwrap();
        w.finish().unwrap();
        let g = normalize_file(&p, 0.0, &prog).unwrap();
        assert!((g - 2.0).abs() < 0.01, "{g}");
        assert!(read_all(&p).1.iter().map(|v| v.unsigned_abs()).max().unwrap() >= 32700);
    }

    #[test]
    fn resampler_keeps_length_level_and_pitch() {
        for from in [48000u32, 96000, 88200] {
            let x = tone(from, 2.0, 0.5);
            let mut rs = Resampler::new(from, 44100);
            let mut out = vec![];
            for part in x.chunks(1234 * 2) {
                rs.process(part, &mut out);
            }
            let frames = out.len() / 2;
            assert!((frames as i64 - 88200).abs() <= 40, "{from}: {frames} frames");
            // The middle second: a 1 kHz tone of the same level.
            let mid = &out[44100..44100 * 3];
            let peak = mid.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!((peak - 0.5).abs() < 0.01, "{from}: peak {peak}");
            let crossings = mid.chunks_exact(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
            assert!((crossings as i32 - 1000).abs() <= 1, "{from}: {crossings} cycles in a second");
        }
        // The same rate passes through untouched.
        let mut rs = Resampler::new(44100, 44100);
        let mut out = vec![];
        rs.process(&[0.1, 0.2, 0.3, 0.4], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn a_48_khz_device_gives_a_44_1_khz_file_of_the_same_length() {
        let d = tmp("rate48");
        let x = tone(48000, 3.0, 0.4);
        let r = record(&d, settings(), 48000, false, |e, start| {
            start();
            for part in x.chunks(480 * 2) {
                e.tap(part, 2, 48000);
            }
        });
        let (rate, s) = read_all(&r.file);
        assert_eq!(rate, 44100);
        assert!((r.frames as i64 - 132300).abs() <= 40, "{} frames", r.frames);
        let peak = s[44100..88200].iter().map(|v| v.unsigned_abs()).max().unwrap() as f32 / 32768.0;
        assert!((peak - 0.4).abs() < 0.01, "{peak}");
    }

    #[test]
    fn tags_are_written_after_the_audio_and_read_back() {
        let d = tmp("tags");
        let p = d.join("REC-2026-11-27.wav");
        let mut w = WavWriter::create(&p, 44100).unwrap();
        w.write(&ramp(0, 1001)).unwrap();
        w.finish().unwrap();
        let info = TagInfo { artist: "DJ Zoë".into(), album: "Friday".into(), title: "Warm Up Mix".into(), track_no: 1 };
        write_tags(&p, &info).unwrap();
        assert_eq!(read_tags(&p), info);
        // The audio is untouched and the RIFF size covers the tags.
        let (rate, s) = read_all(&p);
        assert_eq!((rate, s.len()), (44100, 2002));
        let all = std::fs::read(&p).unwrap();
        assert_eq!(u32::from_le_bytes(all[4..8].try_into().unwrap()) as usize, all.len() - 8);
        // Another tagger reads them.
        use lofty::file::TaggedFileExt;
        use lofty::prelude::Accessor;
        let t = lofty::read_from_path(&p).unwrap();
        let tag = t.primary_tag().or_else(|| t.first_tag()).unwrap();
        assert_eq!(tag.title().as_deref(), Some("Warm Up Mix"));
        assert_eq!(tag.artist().as_deref(), Some("DJ Zoë"));
        assert_eq!(tag.album().as_deref(), Some("Friday"));
        // Writing again replaces them.
        write_tags(&p, &TagInfo { title: "Other".into(), ..Default::default() }).unwrap();
        assert_eq!(read_tags(&p), TagInfo { title: "Other".into(), ..Default::default() });
        assert_eq!(read_all(&p).1.len(), 2002);
    }

    #[test]
    fn tagged_files_go_under_artist_and_album() {
        let f = Path::new("/r");
        let info = TagInfo { artist: "DJ A".into(), album: "Night".into(), title: "Mix".into(), track_no: 3 };
        assert_eq!(tagged_path(f, &info, "wav"), Path::new("/r/DJ A/Night/03 Mix.wav"));
        assert_eq!(tagged_path(f, &TagInfo { track_no: 12, ..Default::default() }, "wav"), Path::new("/r/Unknown Artist/Unknown Album/12 No Title.wav"));
        assert_eq!(tagged_path(f, &TagInfo { artist: "AC/DC ".into(), album: "a:b".into(), title: "x".into(), track_no: 1 }, "wav"), Path::new("/r/AC_DC/a_b/01 x.wav"));
    }

    #[test]
    fn cue_sheet_text_is_rekordboxs() {
        let head = CueHead { date: "2026-11-27 04:32 PM".into(), recorded_by: "recordbox".into(), title: "Warm Up".into(), performer: "DJ A".into(), file: "01 Warm Up.wav".into(), ..Default::default() };
        let tracks = vec![
            CueTrack { title: "Pacha (Extended Mix)".into(), performer: "ZARO".into(), file: "/m/pacha.mp3".into(), at_ms: 400 },
            CueTrack { title: "Cardio".into(), performer: String::new(), file: "/m/cardio.mp3".into(), at_ms: 3_725_900 },
        ];
        let text = cue_text(&head, &tracks, 0, 4_000_000);
        let want = "REM DATE 2026-11-27 04:32 PM\r\nREM RECORDED_BY \"recordbox\"\r\nTITLE \"Warm Up\"\r\nPERFORMER \"DJ A\"\r\nFILE \"01 Warm Up.wav\" WAVE\r\n\
                    \tTRACK 01 AUDIO\r\n\t\tTITLE \"Pacha (Extended Mix)\"\r\n\t\tPERFORMER \"ZARO\"\r\n\t\tFILE \"/m/pacha.mp3\" WAVE\r\n\t\tINDEX 01 00:00:00\r\n\
                    \tTRACK 02 AUDIO\r\n\t\tTITLE \"Cardio\"\r\n\t\tFILE \"/m/cardio.mp3\" WAVE\r\n\t\tINDEX 01 01:02:05\r\n";
        assert_eq!(text, want);
        // A track that went on air after the end of the file is left out.
        assert!(!cue_text(&head, &tracks, 0, 3_000_000).contains("Cardio"));
        // The second of two separated files lists what went on air in it, counted from its
        // start, with the track's number in the whole list.
        let second = cue_text(&head, &tracks, 3_700_000, 300_000);
        assert!(!second.contains("Pacha") && second.contains("\tTRACK 02 AUDIO\r\n") && second.contains("INDEX 01 00:00:25\r\n"), "{second}");
        // Genre and comment come first and are quoted; text that has quotes keeps its own.
        let full = cue_text(&CueHead { genre: "House".into(), comment: "\"live\"".into(), file: "a.wav".into(), ..Default::default() }, &[], 0, 1000);
        assert_eq!(full, "REM GENRE \"House\"\r\nREM COMMENT \"live\"\r\nFILE \"a.wav\" WAVE\r\n");
        let d = tmp("cue");
        let p = write_cue(&d.join("REC-2026-11-27.wav"), &text).unwrap();
        assert!(write_cue(&d.join("REC-2026-11-27.wav"), "replacement").is_err());
        assert_eq!(p.file_name().unwrap(), "REC-2026-11-27.cue");
        assert_eq!(std::fs::read(&p).unwrap(), want.as_bytes());
    }

    #[test]
    fn on_air_events_become_cue_tracks_at_their_times() {
        let t = |n: &str| Some((format!("Title {n}"), format!("Artist {n}"), format!("/m/{n}.mp3")));
        let loaded = [t("a"), t("b"), None, None];
        let mut log = OnAirLog::default();
        // At the first look deck 1 is on air (fader up), deck 2 is not (fader down).
        log.poll([true, false, false, false], &loaded, 480);
        log.poll([true, false, false, false], &loaded, 980);
        // Deck 2's fader comes up 95 s in; deck 1 goes off air later, which adds nothing.
        log.poll([true, true, false, false], &loaded, 95_480);
        log.poll([false, true, false, false], &loaded, 130_000);
        // Deck 1 comes back with the same track: it is listed again (deck 2's was the last).
        log.poll([true, true, false, false], &loaded, 200_000);
        // An empty deck on air adds nothing; the same file twice in a row is added once.
        log.poll([true, true, true, false], &loaded, 210_000);
        log.poll([false, true, false, false], &loaded, 220_000);
        log.poll([true, true, false, false], &loaded, 230_000);
        let got: Vec<(String, i64)> = log.tracks.iter().map(|c| (c.title.clone(), c.at_ms)).collect();
        assert_eq!(got, vec![("Title a".to_string(), 480), ("Title b".to_string(), 95_480), ("Title a".to_string(), 200_000)]);
        let text = cue_text(&CueHead { file: "r.wav".into(), ..Default::default() }, &log.tracks, 0, 300_000);
        assert!(text.contains("\tTRACK 01 AUDIO\r\n\t\tTITLE \"Title a\"\r\n\t\tPERFORMER \"Artist a\"\r\n\t\tFILE \"/m/a.mp3\" WAVE\r\n\t\tINDEX 01 00:00:00\r\n"));
        assert!(text.contains("\tTRACK 02 AUDIO\r\n\t\tTITLE \"Title b\"\r\n\t\tPERFORMER \"Artist b\"\r\n\t\tFILE \"/m/b.mp3\" WAVE\r\n\t\tINDEX 01 00:01:35\r\n"));
        assert!(text.contains("\tTRACK 03 AUDIO\r\n") && text.contains("INDEX 01 00:03:20\r\n"));
    }

    #[test]
    fn on_air_rule() {
        let on = |master: f32, xf: f32, assign: u8, fader: f32, curve: u8, iso: bool, eq: [f32; 3], trim: f32| on_air(master, xf, assign, fader, curve, iso, eq, trim);
        let mid = [0.5; 3];
        assert!(on(0.9, 0.5, 1, 1.0, 1, false, mid, 0.5));
        // Master volume below 23/1023.
        assert!(!on(22.0 / 1023.0, 0.5, 1, 1.0, 1, false, mid, 0.5));
        assert!(on(23.5 / 1023.0, 0.5, 1, 1.0, 1, false, mid, 0.5));
        // The crossfader at the other side's end stop; THRU does not care.
        assert!(!on(0.9, 1.0, 1, 1.0, 1, false, mid, 0.5));
        assert!(on(0.9, 1019.0 / 1023.0, 1, 1.0, 1, false, mid, 0.5));
        assert!(!on(0.9, 0.0, 2, 1.0, 1, false, mid, 0.5));
        assert!(on(0.9, 4.0 / 1023.0, 2, 1.0, 1, false, mid, 0.5));
        assert!(on(0.9, 0.0, 0, 1.0, 1, false, mid, 0.5) && on(0.9, 1.0, 0, 1.0, 1, false, mid, 0.5));
        // The channel fader's threshold depends on its curve: 23, 5 or 6 of 1023.
        assert!(!on(0.9, 0.5, 1, 22.0 / 1023.0, 0, false, mid, 0.5) && on(0.9, 0.5, 1, 23.5 / 1023.0, 0, false, mid, 0.5));
        assert!(!on(0.9, 0.5, 1, 4.0 / 1023.0, 1, false, mid, 0.5) && on(0.9, 0.5, 1, 5.5 / 1023.0, 1, false, mid, 0.5));
        assert!(!on(0.9, 0.5, 1, 5.5 / 1023.0, 2, false, mid, 0.5) && on(0.9, 0.5, 1, 6.5 / 1023.0, 2, false, mid, 0.5));
        // Isolator with all three bands killed; the equalizer's knobs at zero do not count.
        assert!(!on(0.9, 0.5, 1, 1.0, 1, true, [0.0; 3], 0.5));
        assert!(on(0.9, 0.5, 1, 1.0, 1, true, [0.0, 0.0, 0.5], 0.5));
        assert!(on(0.9, 0.5, 1, 1.0, 1, false, [0.0; 3], 0.5));
        // Trim at 2/255 or less.
        assert!(!on(0.9, 0.5, 1, 1.0, 1, false, mid, 2.0 / 255.0) && on(0.9, 0.5, 1, 1.0, 1, false, mid, 3.0 / 255.0));
    }

    #[test]
    fn time_left_is_three_hours_or_what_the_disk_holds() {
        assert_eq!(remaining_secs(0, None), 10800);
        assert_eq!(hms(remaining_secs(0, None)), "03:00:00");
        assert_eq!(remaining_secs(3600, None), 7200);
        assert_eq!(remaining_secs(10800, None), 0);
        assert_eq!(remaining_secs(20000, None), 0);
        // 176400 bytes a second; 20 MB are left alone.
        assert_eq!(remaining_secs(0, Some(DISK_RESERVE + 176_400 * 90)), 90);
        assert_eq!(remaining_secs(0, Some(1000)), 0);
        assert_eq!(hms(3725), "01:02:05");
    }
}
