//! Audio decoding (symphonia) and the preview player (cpal).

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::sync::{Arc, OnceLock};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decoded audio, interleaved stereo f32.
pub struct Decoded {
    pub sr: u32,
    pub samples: Vec<f32>,
    /// Length in sample frames of the file's full (untrimmed) decode.
    pub raw_frames: usize,
    /// Packets that could not be decoded and were skipped (damaged file).
    pub damaged: usize,
    /// The file ends more than a second before the length its header declares (cut short).
    pub truncated: bool,
}

impl Decoded {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sr.max(1) as f64
    }
    pub fn mono(&self) -> Vec<f32> {
        self.samples.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect()
    }
}

/// One clock per process: a decode worker, a playback deck and a cache lookup
/// must all interpret a sample position the same way.
#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum Mp3Timeline {
    #[default]
    Legacy,
    Rekordbox,
}

static MP3_TIMELINE: OnceLock<Mp3Timeline> = OnceLock::new();

impl Mp3Timeline {
    pub fn from_args(args: &[String]) -> Result<Self> {
        let mut selected = None;
        let mut args = args.iter().map(String::as_str);
        while let Some(arg) = args.next() {
            let value = if arg == "--mp3-timeline" {
                args.next().unwrap_or("")
            } else if let Some(value) = arg.strip_prefix("--mp3-timeline=") {
                value
            } else {
                continue;
            };
            anyhow::ensure!(selected.is_none(), "Specify --mp3-timeline only once.");
            selected = Some(match value {
                "legacy" => Self::Legacy,
                "rekordbox" => Self::Rekordbox,
                _ => anyhow::bail!("Expected --mp3-timeline legacy or --mp3-timeline rekordbox."),
            });
        }
        Ok(selected.unwrap_or_default())
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Rekordbox => "rekordbox",
        }
    }

    pub fn initialize(self) {
        MP3_TIMELINE.set(self).expect("initialize MP3 timeline only at startup");
    }
}

pub fn mp3_timeline() -> Mp3Timeline {
    MP3_TIMELINE.get().copied().unwrap_or_default()
}

pub fn is_mp3(path: &str) -> bool {
    std::path::Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mp3"))
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Mp3Timing {
    NotMp3,
    Lame,
    Ffmpeg,
    Untagged,
}

/// Samples dropped from the start of MP3s without an encoder tag (one MPEG-1 Layer III frame).
pub const MP3_UNTAGGED_SKIP: usize = 1152;

/// Look at the first MP3 frame for a LAME-style info tag and its encoder name.
pub fn mp3_timing_class(path: &str) -> Mp3Timing {
    use std::io::Read;
    if !is_mp3(path) {
        return Mp3Timing::NotMp3;
    }
    let Ok(mut f) = std::fs::File::open(path) else { return Mp3Timing::NotMp3 };
    let mut head = vec![0u8; 10];
    if f.read_exact(&mut head).is_err() {
        return Mp3Timing::Untagged;
    }
    let mut skip = 0u64;
    if &head[..3] == b"ID3" {
        skip = 10 + (((head[6] as u64 & 0x7f) << 21) | ((head[7] as u64 & 0x7f) << 14) | ((head[8] as u64 & 0x7f) << 7) | (head[9] as u64 & 0x7f));
        if head[5] & 0x10 != 0 {
            skip += 10;
        }
    }
    use std::io::Seek;
    if f.seek(std::io::SeekFrom::Start(skip)).is_err() {
        return Mp3Timing::Untagged;
    }
    let mut buf = vec![0u8; 8192];
    let n = f.read(&mut buf).unwrap_or(0);
    let buf = &buf[..n];
    // first frame sync
    let Some(sync) = buf.windows(2).position(|w| w[0] == 0xff && (w[1] & 0xe0) == 0xe0) else { return Mp3Timing::Untagged };
    let frame = &buf[sync..(sync + 2000).min(buf.len())];
    let has = |pat: &[u8]| frame.windows(pat.len()).any(|w| w == pat);
    if !(has(b"Xing") || has(b"Info")) {
        return Mp3Timing::Untagged;
    }
    if has(b"LAME") || has(b"GOGO") {
        Mp3Timing::Lame
    } else if has(b"Lavc") || has(b"Lavf") {
        Mp3Timing::Ffmpeg
    } else {
        Mp3Timing::Untagged
    }
}

/// Decode a whole file. The source file is only read.
pub fn decode_file(path: &str) -> Result<Decoded> {
    decode_until(path, || false)
}

pub fn decode_until(path: &str, cancelled: impl Fn() -> bool) -> Result<Decoded> {
    decode_with_timeline(path, mp3_timeline(), cancelled)
}

fn decode_with_timeline(path: &str, timeline: Mp3Timeline, cancelled: impl Fn() -> bool) -> Result<Decoded> {
    if cancelled() { anyhow::bail!("stopped"); }
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    // Earlier recordbox collections used an encoder-name heuristic. The optional
    // Rekordbox clock keeps decoder delay and counts a matching leading Info frame.
    let (gapless, trim_untagged, lead) = match timeline {
        Mp3Timeline::Legacy => {
            let class = mp3_timing_class(path);
            (class == Mp3Timing::Ffmpeg, class == Mp3Timing::Untagged, 0)
        }
        Mp3Timeline::Rekordbox => (false, false, crate::analysis::mp3_timebase::lead_frames(path)),
    };
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions { enable_gapless: gapless, ..Default::default() }, &MetadataOptions::default())?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no audio track"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let track_id = track.id;
    let mut sr = track.codec_params.sample_rate.unwrap_or(44100);
    let mut out = vec![0.0; lead * 2];
    let declared_frames = track.codec_params.n_frames;
    let time_base = track.codec_params.time_base;
    if let Some(n) = track.codec_params.n_frames {
        out.reserve((n as usize).min(60 * 60 * 48000) * 2);
    }
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut damaged = 0usize;
    loop {
        if cancelled() { anyhow::bail!("stopped"); }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                // Silence for the packet's length, so everything after stays where it belongs.
                damaged += 1;
                let n = (packet.dur as usize).min(1 << 20);
                out.extend(std::iter::repeat(0.0f32).take(n * 2));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        sr = spec.rate;
        let ch = spec.channels.count();
        if sbuf.as_ref().map(|b| b.capacity() < decoded.capacity() * ch).unwrap_or(true) {
            sbuf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let b = sbuf.as_mut().unwrap();
        b.copy_interleaved_ref(decoded);
        let s = b.samples();
        if ch == 1 {
            for v in s {
                out.push(*v);
                out.push(*v);
            }
        } else if ch == 2 {
            for f in s.chunks_exact(2) {
                out.push(f[0]);
                out.push(f[1]);
            }
        } else {
            // More than two channels: a standard stereo downmix by channel position (centre and
            // surrounds at -3 dB, LFE left out) instead of keeping only the first two.
            use symphonia::core::audio::Channels;
            let g = std::f32::consts::FRAC_1_SQRT_2;
            let w: Vec<(f32, f32)> = spec
                .channels
                .iter()
                .map(|c| {
                    if c == Channels::FRONT_LEFT {
                        (1.0, 0.0)
                    } else if c == Channels::FRONT_RIGHT {
                        (0.0, 1.0)
                    } else if c == Channels::FRONT_CENTRE {
                        (g, g)
                    } else if c == Channels::LFE1 || c == Channels::LFE2 {
                        (0.0, 0.0)
                    } else if c == Channels::REAR_LEFT || c == Channels::SIDE_LEFT || c == Channels::FRONT_LEFT_CENTRE || c == Channels::REAR_LEFT_CENTRE {
                        (g, 0.0)
                    } else if c == Channels::REAR_RIGHT || c == Channels::SIDE_RIGHT || c == Channels::FRONT_RIGHT_CENTRE || c == Channels::REAR_RIGHT_CENTRE {
                        (0.0, g)
                    } else {
                        (0.5 * g, 0.5 * g)
                    }
                })
                .collect();
            for f in s.chunks_exact(ch) {
                let (mut l, mut r) = (0f32, 0f32);
                for (v, (wl, wr)) in f.iter().zip(&w) {
                    l += v * wl;
                    r += v * wr;
                }
                out.push(l);
                out.push(r);
            }
        }
    }
    let decoded_frames = out.len() / 2 - lead;
    // Compared in seconds: some containers (MP4) count the declared length in their own time
    // units rather than sample frames.
    let declared_secs = declared_frames.map(|n| match time_base {
        Some(tb) => {
            let t = tb.calc_time(n);
            t.seconds as f64 + t.frac
        }
        None => n as f64 / sr.max(1) as f64,
    });
    let truncated = declared_secs.map(|d| decoded_frames as f64 / sr.max(1) as f64 + 1.0 < d).unwrap_or(false);
    let mut raw_frames = decoded_frames + lead;
    if gapless {
        raw_frames += track.codec_params.delay.unwrap_or(0) as usize + track.codec_params.padding.unwrap_or(0) as usize;
    }
    if trim_untagged {
        let drop = (MP3_UNTAGGED_SKIP * 2).min(out.len());
        out.drain(..drop);
    }
    if decoded_frames == 0 || out.is_empty() {
        return Err(anyhow!("no audio decoded"));
    }
    Ok(Decoded { sr, samples: out, raw_frames, damaged, truncated })
}

#[cfg(test)]
mod mp3_timeline_tests {
    use super::*;

    #[test]
    fn legacy_remains_default_and_invalid_or_repeated_clocks_are_rejected() {
        let parse = |args: &[&str]| Mp3Timeline::from_args(&args.iter().copied().map(String::from).collect::<Vec<_>>());
        assert_eq!(parse(&[]).unwrap(), Mp3Timeline::Legacy);
        assert_eq!(parse(&["--mp3-timeline", "legacy"]).unwrap(), Mp3Timeline::Legacy);
        assert_eq!(parse(&["--disable-load", "--mp3-timeline=rekordbox"]).unwrap(), Mp3Timeline::Rekordbox);
        assert!(parse(&["--mp3-timeline"]).is_err());
        assert!(parse(&["--mp3-timeline", "other"]).is_err());
        assert!(parse(&["--mp3-timeline=rekordbox", "--mp3-timeline=legacy"]).is_err());
    }

    #[test]
    fn untagged_mp3_keeps_its_first_frame_only_on_the_rekordbox_clock() -> Result<()> {
        // Eight MPEG-1 Layer III frames at 320 kbps, 44.1 kHz, with silent granules.
        let mut frame = vec![0; 1044];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0xe0, 0x64]);
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("silent.MP3");
        std::fs::write(&path, frame.repeat(8))?;
        let path = path.to_str().unwrap();
        let legacy = decode_with_timeline(path, Mp3Timeline::Legacy, || false)?;
        let rb = decode_with_timeline(path, Mp3Timeline::Rekordbox, || false)?;
        assert_eq!(rb.raw_frames, legacy.raw_frames);
        assert_eq!(rb.frames(), legacy.frames() + MP3_UNTAGGED_SKIP);
        assert_eq!(legacy.samples, rb.samples[MP3_UNTAGGED_SKIP * 2..]);
        let polls = std::cell::Cell::new(0);
        assert!(decode_with_timeline(path, Mp3Timeline::Rekordbox, || {
            polls.set(polls.get() + 1);
            polls.get() == 3
        }).err().is_some_and(|error| error.to_string() == "stopped"));
        Ok(())
    }
}

/// Shared playback state read by the audio callback.
pub struct PlayState {
    pub audio: Option<Arc<Decoded>>,
    pub pos: f64,
    pub playing: bool,
    pub volume: f32,
    pub gain: f32,
    pub loop_range: Option<(f64, f64)>,
    pub metronome: Option<Arc<Vec<f64>>>,
    pub metronome_vol: f32,
    /// Separated stems of the loaded track (44.1 kHz) and which are audible, in source order
    /// drums, bass, other, vocals. With all on (or no stems) the original mix plays.
    pub stems: Option<Arc<crate::stems::Stems>>,
    pub stem_preview: Option<Arc<crate::stems::Live>>,
    pub stem_on: [bool; 4],
    /// STEM ISO gain of each source (same order): 0 kills it, 1 leaves it, up to +6 dB
    /// (`stem_iso_gain` turns a knob position into this).
    pub stem_iso: [f32; 4],
    /// Search (scan): 0 = off; otherwise the speed and direction (e.g. 8.0 forward, -8.0
    /// backward). Short grains are played at normal pitch and the position skips between
    /// them, as a CDJ's SEARCH buttons do; it sounds whether or not the deck is playing.
    pub search: f64,
    /// Master mute (the level is kept).
    pub muted: bool,
    /// Playing speed: 1.0 = the track's own tempo (TEMPO − / +, typed BPM, BEAT SYNC).
    pub tempo: f64,
    /// Pitch bend: added to `tempo` while a bend button is held.
    pub bend: f64,
    /// Master Tempo (key lock): speed changes leave the pitch alone.
    pub master_tempo: bool,
    /// Key shift in semitones (KEY SYNC, key shift ‹ ›), independent of the tempo.
    pub key_shift: f32,
    /// Crossfader gain of this deck (1.0 outside the two-deck layouts).
    pub fader: f32,
    /// The 3-band isolator is in the signal path (Dual Player), with these band gains
    /// (low, mid, high; 0.0 = killed).
    pub eq_on: bool,
    pub eq_gain: [f32; 3],
    /// The Performance mixer's channel strip (`mixer.rs`). `strip`: 0 = none (the Export
    /// layouts), 1 = equalizer, 2 = isolator. `eq_pos`: LOW / MID / HIGH knobs, 0..1.
    pub strip: u8,
    pub eq_pos: [f32; 3],
    /// TRIM and channel fader gains (1.0 = unity).
    pub trim: f32,
    pub ch_fader: f32,
    /// Highest level since the meter last read it, after the EQ and before the fader (L, R).
    pub peak: [f32; 2],
    /// Fade in and out over a few milliseconds at play and pause, and cross-fade over a jump
    /// of the position (a cue, a hot cue, a beat jump, a click on the overview), so that none
    /// of them clicks. The app always sets it; off by default so that tests of the plain
    /// signal path see every sample.
    pub smooth: bool,
    /// Which deck this is (0 = deck 1), for the FX units' assignment.
    pub deck_no: u8,
    /// Sound Color FX on this channel: the effect, its COLOR knob (0.5 = off) and PARAM.
    pub cfx_kind: u8,
    pub cfx_color: f32,
    pub cfx_param: f32,
    /// Headphones CUE: how much of this deck (after its EQ, before its faders) goes to the
    /// headphones output; 0 when CUE is off.
    pub cue_gain: f32,
    /// SLIP: while a loop runs or a hot cue is held (`slip_hold`), the position the track
    /// would have reached keeps moving in the background (`slip_pos`, source frames), and
    /// playback returns to it when the loop ends or the pad is released.
    pub slip: bool,
    pub slip_hold: bool,
    pub slip_pos: Option<f64>,
    /// Beat times of the loaded track (seconds), for BEAT SYNC.
    pub grid: Option<Arc<Vec<f64>>>,
    /// BEAT SYNC: this deck follows the other deck's beat phase while both play.
    pub sync_follow: bool,
    /// Speed correction the audio callback applies to hold the beat phase (BEAT SYNC).
    pub sync_bend: f64,
    /// The deck engine's own state (`deck.rs`): reverse, jog, vinyl ramps, sync rate, routes.
    pub dk: crate::deck::Engine,
    /// The pad modes' engines (slicer, active censor): `pad_audio.rs`.
    pub pads: crate::pad_audio::DeckPads,
    click_phase: usize,
    next_click: usize,
    search_count: usize,
    dsp: Box<DeckDsp>,
}

pub const DEFAULT_VOLUME: f32 = 0.9;

impl PlayState {
    /// Mute and solo become available as soon as a playable prefix exists.
    pub fn has_playable_stems(&self) -> bool {
        self.stems.is_some() || self.stem_preview.as_ref().is_some_and(|p| p.ready_frames() >= 4)
    }
}

impl Default for PlayState {
    fn default() -> Self {
        PlayState {
            audio: None,
            pos: 0.0,
            playing: false,
            volume: DEFAULT_VOLUME,
            gain: 1.0,
            loop_range: None,
            metronome: None,
            metronome_vol: 0.5,
            stems: None,
            stem_preview: None,
            stem_on: [true; 4],
            stem_iso: [1.0; 4],
            search: 0.0,
            muted: false,
            tempo: 1.0,
            bend: 0.0,
            master_tempo: false,
            key_shift: 0.0,
            fader: 1.0,
            eq_on: false,
            eq_gain: [1.0; 3],
            strip: 0,
            eq_pos: [0.5; 3],
            trim: 1.0,
            ch_fader: 1.0,
            peak: [0.0; 2],
            smooth: false,
            deck_no: 0,
            cue_gain: 0.0,
            cfx_kind: 0,
            cfx_color: 0.5,
            cfx_param: 0.5,
            slip: false,
            slip_hold: false,
            slip_pos: None,
            grid: None,
            sync_follow: false,
            sync_bend: 0.0,
            dk: Default::default(),
            pads: Default::default(),
            click_phase: usize::MAX,
            next_click: 0,
            search_count: 0,
            dsp: Box::new(DeckDsp::new()),
        }
    }
}

pub struct Player {
    /// The active deck's playback state.
    pub state: Arc<Mutex<PlayState>>,
    /// All four decks' playback states by deck number (0 = deck 1); all are mixed to the
    /// output. `state` is the same object as `decks[active]`: it follows the active deck.
    pub decks: [Arc<Mutex<PlayState>>; 4],
    /// Set by the stream when its device stops working (unplugged, switched off).
    pub lost: Arc<std::sync::atomic::AtomicBool>,
    stream: Option<cpal::Stream>,
    output_enabled: bool,
    buffer_load: Arc<std::sync::atomic::AtomicU32>,
    idle_since: Option<std::time::Instant>,
    parked: bool,
    pub device_rate: u32,
    pub error: Option<String>,
}

impl Player {
    #[allow(dead_code)]
    pub fn new() -> Player {
        Self::with_device(None, true)
    }

    /// Open the named output device (None: the system's default output).
    pub fn with_device(device: Option<&str>, output_enabled: bool) -> Player {
        let decks: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|_| Arc::new(Mutex::new(PlayState::default())));
        let state = decks[0].clone();
        let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut player = Player { state, decks, lost, stream: None, output_enabled, buffer_load: Default::default(), idle_since: None, parked: false, device_rate: 44100, error: None };
        if let Err(e) = player.switch_device(device) {
            player.error = Some(e.to_string());
        }
        player
    }

    /// Names of the output devices this Mac offers. Listing devices can block indefinitely on
    /// some systems (seen with a CoreAudio device on the development Mac), so it runs on its
    /// own thread and gives up after `timeout`: None then.
    pub fn output_devices_within(timeout: std::time::Duration) -> Option<Vec<String>> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait};
            let names: Vec<String> = cpal::default_host().output_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default();
            let _ = tx.send(names);
        });
        rx.recv_timeout(timeout).ok()
    }

    /// The named output device, searched on its own thread (see `output_devices_within`).
    fn find_device(want: &str, timeout: std::time::Duration) -> Option<cpal::Device> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let want = want.to_string();
        std::thread::spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait};
            let d = cpal::default_host().output_devices().ok().and_then(|mut it| it.find(|d| d.name().map(|n| n == want).unwrap_or(false)));
            let _ = tx.send(d);
        });
        rx.recv_timeout(timeout).ok().flatten()
    }

    /// Switch to another output device, keeping what is loaded and playing.
    pub fn switch_device(&mut self, device: Option<&str>) -> Result<()> {
        if !self.output_enabled { return Ok(()); }
        let buffer_load = Arc::default();
        let (s, rate) = Self::open_stream(self.decks.clone(), device, self.lost.clone(), Arc::clone(&buffer_load))?;
        self.buffer_load = buffer_load;
        self.lost.store(false, std::sync::atomic::Ordering::Relaxed);
        self.stream = Some(s);
        self.parked = false;
        self.idle_since = None;
        self.device_rate = rate;
        self.error = None;
        Ok(())
    }

    pub fn transport_running(&self) -> bool {
        self.decks.iter().any(|deck| {
            let s = deck.lock();
            s.playing || s.search != 0.0 || matches!(s.dk.jog, crate::deck::Jog::Speed(_)) || crate::groove::wants_render(&s)
        })
    }

    /// Linux ALSA plugins can keep servicing a silent stream after the decks stop.
    /// Keep a short cue/resume window before actually releasing the device.
    #[cfg(target_os = "linux")]
    pub fn service_output(&mut self, keep_ready: bool, running: bool, device: Option<&str>) -> Result<Option<std::time::Duration>> {
        use std::time::{Duration, Instant};
        if !self.output_enabled { return Ok(None); }
        if keep_ready || running {
            self.idle_since = None;
            if self.parked || (running && self.stream.is_none() && !self.lost.load(std::sync::atomic::Ordering::Relaxed)) {
                self.parked = false;
                self.switch_device(device)?;
            }
            return Ok(None);
        }
        if self.stream.is_none() { return Ok(None); }
        let since = self.idle_since.get_or_insert_with(Instant::now);
        let remaining = Duration::from_secs(2).saturating_sub(since.elapsed());
        if !remaining.is_zero() { return Ok(Some(remaining)); }
        self.stream = None;
        self.parked = true;
        HP_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(None)
    }

    /// Elapsed time in the most recent output callback as a percentage of its buffer period.
    pub fn buffer_load_percent(&self) -> Option<f32> {
        self.stream.as_ref()?;
        Some(f32::from_bits(self.buffer_load.load(std::sync::atomic::Ordering::Relaxed)))
    }

    fn open_stream(state: [Arc<Mutex<PlayState>>; 4], device: Option<&str>, lost: Arc<std::sync::atomic::AtomicBool>, buffer_load: Arc<std::sync::atomic::AtomicU32>) -> Result<(cpal::Stream, u32)> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let named = device.and_then(|want| Self::find_device(want, std::time::Duration::from_secs(3)));
        // A device that is no longer connected falls back to the default output.
        let dev = match named {
            Some(d) => d,
            None => host.default_output_device().ok_or_else(|| anyhow!("no audio output device"))?,
        };
        // Sample rate, buffer size and output channel pair from Preferences > Audio.
        let (cfg, config, first_ch) = crate::prefs_ext::stream_config(&dev)?;
        let rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let record_time = move |start: std::time::Instant, samples: usize| {
            let frames = samples / channels;
            if frames > 0 {
                let percent = start.elapsed().as_secs_f64() * f64::from(rate) * 100.0 / frames as f64;
                buffer_load.store((percent as f32).to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        };
        let st = state.clone();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => {
                let lost = lost.clone();
                let mut pair: Vec<f32> = Vec::with_capacity(8192);
                let mut phones: Vec<f32> = Vec::with_capacity(8192);
                // Headphones on their own channel pair of the same device, when it has one
                // besides the master's (an interface or controller with four outputs).
                let hp = usize::try_from(HP_CHANNEL.load(std::sync::atomic::Ordering::Relaxed)).ok().filter(|h| h + 1 < channels && *h != first_ch && h + 1 != first_ch && *h != first_ch + 1);
                HP_ACTIVE.store(hp.is_some(), std::sync::atomic::Ordering::Relaxed);
                dev.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let start = std::time::Instant::now();
                    match hp {
                        Some(h) => {
                            let frames = data.len() / channels.max(1);
                            pair.clear();
                            pair.resize(frames * 2, 0.0);
                            phones.clear();
                            phones.resize(frames * 2, 0.0);
                            render_cue(&st, &mut pair, &mut phones, 2, rate);
                            data.fill(0.0);
                            for ((f, m), c) in data.chunks_mut(channels).zip(pair.chunks(2)).zip(phones.chunks(2)) {
                                f[first_ch] = m[0];
                                f[first_ch + 1] = m[1];
                                f[h] = c[0].clamp(-1.0, 1.0);
                                f[h + 1] = c[1].clamp(-1.0, 1.0);
                            }
                        }
                        None if first_ch == 0 => render(&st, data, channels, rate),
                        None => crate::prefs_ext::render_pair(data, channels, first_ch, &mut pair, |b| render(&st, b, 2, rate)),
                    }
                    record_time(start, data.len());
                },
                move |e| {
                    eprintln!("audio error: {e}");
                    if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                        lost.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                },
                None,
            )?
            }
            _ => {
                let st2 = st.clone();
                // Scratch buffer reused across callbacks (no allocation on the audio thread once
                // it has reached the device's buffer size).
                let mut tmp: Vec<f32> = Vec::with_capacity(8192);
                dev.build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        let start = std::time::Instant::now();
                        tmp.resize(data.len(), 0.0);
                        render(&st2, &mut tmp, channels, rate);
                        for (d, s) in data.iter_mut().zip(tmp.iter().copied()) {
                            *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                        record_time(start, data.len());
                    },
                    move |e| {
                        eprintln!("audio error: {e}");
                        if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                            lost.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    },
                    None,
                )?
            }
        };
        stream.play()?;
        Ok((stream, rate))
    }
}

/// Catmull-Rom interpolation between y1 (t = 0) and y2 (t = 1).
#[inline]
fn cubic(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
    let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c = -0.5 * y0 + 0.5 * y2;
    ((a * t + b) * t + c) * t + y1
}

// ---------------------------------------------------------------------- deck DSP

/// Crossfader law for a position `x` from 0.0 (deck A's end) to 1.0 (deck B's end): both decks
/// at full level in the middle, the far deck falling smoothly (a quarter sine) to silence over
/// its half of the travel. Returns (gain of A, gain of B).
pub fn crossfader_gains(x: f32) -> (f32, f32) {
    let x = x.clamp(0.0, 1.0);
    let g = |d: f32| (std::f32::consts::FRAC_PI_2 * (2.0 * d).clamp(0.0, 1.0)).sin();
    (g(1.0 - x), g(x))
}

/// How far through its beat a deck is at `t` seconds: 0.0 on a beat, 0.5 halfway to the
/// next. Before the first and after the last beat the nearest interval is continued. None
/// without at least two beats.
pub fn beat_phase(beats: &[f64], t: f64) -> Option<f64> {
    if beats.len() < 2 {
        return None;
    }
    let n = beats.len();
    let i = beats.partition_point(|b| *b <= t);
    let (a, len) = if i == 0 {
        (beats[0], beats[1] - beats[0])
    } else if i >= n {
        (beats[n - 1], beats[n - 1] - beats[n - 2])
    } else {
        (beats[i - 1], beats[i] - beats[i - 1])
    };
    if len <= 0.0 {
        return None;
    }
    Some(((t - a) / len).rem_euclid(1.0))
}

/// Where `t` is on the beat grid, in beats from its first beat (whole part = the beat's
/// number, fraction = `beat_phase`). For Beat FX quantize.
pub fn beat_pos(beats: &[f64], t: f64) -> Option<f64> {
    let phase = beat_phase(beats, t)?;
    let i = beats.partition_point(|b| *b <= t);
    Some(if i == 0 { ((t - beats[0]) / (beats[1] - beats[0])).floor() + phase } else { (i - 1) as f64 + if i >= beats.len() { ((t - beats[i - 1]) / (beats[i - 1] - beats[i - 2])).floor() } else { 0.0 } + phase })
}

/// Beat phase of the follower minus the master's, wrapped to -0.5..0.5 beats (positive: the
/// follower is ahead).
pub fn phase_error(follower: &[f64], t_f: f64, master: &[f64], t_m: f64) -> Option<f64> {
    let e = beat_phase(follower, t_f)? - beat_phase(master, t_m)?;
    Some((e + 0.5).rem_euclid(1.0) - 0.5)
}

/// Speed correction (fraction of normal speed) that pulls a phase error of `e` beats back to
/// zero: proportional, at most 8 %, nothing inside a 0.3 % dead zone.
pub fn sync_correction(e: f64) -> f64 {
    if e.abs() < 0.003 {
        0.0
    } else {
        (-e).clamp(-0.08, 0.08)
    }
}

/// The speed at which a track of `own_bpm` plays at `master_bpm`. When the tempos are nearer
/// an octave apart the half or double speed is matched instead (a 70 BPM track against 140).
pub fn sync_rate(master_bpm: f64, own_bpm: f64) -> f64 {
    if master_bpm <= 0.0 || own_bpm <= 0.0 {
        return 1.0;
    }
    // (rekordbox's thresholds: half under 2/3 of the master's BPM, double over 1.5 times.)
    master_bpm * crate::deck::sync_rate_auto(master_bpm, own_bpm) / own_bpm
}

/// One second-order filter section for both channels (transposed direct form II).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    /// kind: 0 low-pass, 1 high-pass, 2 all-pass; Butterworth Q.
    fn set(&mut self, kind: u8, f: f32, rate: f32) {
        let w = 2.0 * std::f32::consts::PI * (f / rate).min(0.45);
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let b = match kind {
            0 => [(1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0],
            1 => [(1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0],
            _ => [1.0 - alpha, -2.0 * cs, 1.0 + alpha],
        };
        self.b = [b[0] / a0, b[1] / a0, b[2] / a0];
        self.a = [-2.0 * cs / a0, (1.0 - alpha) / a0];
    }
    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.b[0] * x + self.z1[ch];
        self.z1[ch] = self.b[1] * x - self.a[0] * y + self.z2[ch];
        self.z2[ch] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Crossover frequencies of the isolator (Hz): LOW below the first, HIGH above the second.
/// rekordbox's isolator crossovers (`Isolator::applyFilter`, 0x102ab9828 in 7.2.19).
pub const ISO_LOW_HZ: f32 = 284.0;
pub const ISO_HIGH_HZ: f32 = 4649.0;

/// 3-band isolator: fourth-order Linkwitz-Riley crossovers, whose bands add back to the
/// input's level at every frequency (the low band passes an all-pass matching the upper
/// crossover's phase), so with all three bands on it is inaudible and each band can be
/// switched off completely.
#[derive(Clone, Copy, Default)]
pub struct Iso3 {
    rate: u32,
    lp1: [Biquad; 2],
    hp1: [Biquad; 2],
    lp2: [Biquad; 2],
    hp2: [Biquad; 2],
    ap: Biquad,
    /// Band gains in use (moving toward the deck's `eq_gain`).
    g: [f32; 3],
}

impl Iso3 {
    pub fn new() -> Iso3 {
        Iso3 { g: [1.0; 3], ..Default::default() }
    }
    fn prepare(&mut self, rate: u32) {
        if self.rate == rate {
            return;
        }
        *self = Iso3 { rate, g: self.g, ..Default::default() };
        let r = rate as f32;
        for k in 0..2 {
            self.lp1[k].set(0, ISO_LOW_HZ, r);
            self.hp1[k].set(1, ISO_LOW_HZ, r);
            self.lp2[k].set(0, ISO_HIGH_HZ, r);
            self.hp2[k].set(1, ISO_HIGH_HZ, r);
        }
        self.ap.set(2, ISO_HIGH_HZ, r);
    }
    /// One sample of channel `ch` with the band gains `g`.
    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let low = self.lp1[0].run(ch, x);
        let low = self.lp1[1].run(ch, low);
        let rest = self.hp1[0].run(ch, x);
        let rest = self.hp1[1].run(ch, rest);
        let mid = self.lp2[0].run(ch, rest);
        let mid = self.lp2[1].run(ch, mid);
        let high = self.hp2[0].run(ch, rest);
        let high = self.hp2[1].run(ch, high);
        self.g[0] * self.ap.run(ch, low) + self.g[1] * mid + self.g[2] * high
    }
    /// A mono signal through the isolator at fixed band gains (tests and offline use).
    #[allow(dead_code)]
    pub fn process(rate: u32, gains: [f32; 3], x: &[f32]) -> Vec<f32> {
        let mut iso = Iso3::new();
        iso.prepare(rate);
        iso.g = gains;
        x.iter().map(|v| iso.run(0, *v)).collect()
    }
}

/// The loaded audio as the callback reads it: the original mix and, once the track is
/// separated, its parts, each with the gain in force for the frame being rendered.
///
/// As rekordbox 7.2.19 mixes them (`StretchableAudioSource::read`, 0x10287fa18): while no
/// part is muted or moved off unity, only the original file plays (bit for bit); otherwise
/// the original fades out and the audible parts fade in. The instruments part is what is
/// left of the mix after the other parts, so all parts together are always the mix.
struct Src<'a> {
    audio: &'a Decoded,
    frames: usize,
    stems: Option<&'a crate::stems::Stems>,
    preview: Option<(&'a crate::stems::Live, usize)>,
    /// Gains for this frame: the original, then drums, bass, other, vocals.
    g: std::cell::Cell<[f32; 5]>,
    stem_ratio: f64,
}

impl Src<'_> {
    /// Left and right at source frame `p` (fractional).
    #[inline]
    fn at(&self, p: f64) -> (f32, f32) {
        let p = p.max(0.0);
        // 4-point cubic (Catmull-Rom) between frames: exact at whole positions, far less
        // high-frequency loss than linear interpolation when the file's rate differs from
        // the device's (e.g. 44.1 kHz files on a 48 kHz output).
        let i = p as usize;
        let fr = (p - i as f64) as f32;
        let frames = self.frames as isize;
        let at = |k: isize, c: usize| self.audio.samples[(k.clamp(0, frames - 1) as usize) * 2 + c];
        let ii = i as isize;
        let (ol, or) = (cubic(at(ii - 1, 0), at(ii, 0), at(ii + 1, 0), at(ii + 2, 0), fr), cubic(at(ii - 1, 1), at(ii, 1), at(ii + 1, 1), at(ii + 2, 1), fr));
        let g = self.g.get();
        if g == [1.0, 0.0, 0.0, 0.0, 0.0] {
            return (ol, or);
        }
        let sp = p * self.stem_ratio;
        let (len, ready) = match (self.stems, self.preview) {
            (Some(st), _) => (st.len, st.len),
            (None, Some((live, ready))) => (live.len(), ready),
            _ => return (ol * g[0], or * g[0]),
        };
        if len < 2 { return (ol, or); }
        let sp = sp.min((len - 1) as f64);
        let si = sp as usize;
        let sf = (sp - si as f64) as f32;
        let taps = [si.saturating_sub(1), si, (si+1).min(len-1), (si+2).min(len-1)];
        if taps[3] >= ready {
            // A seek, loop or jog can immediately reach an unfinished portion of the track.
            return (ol, or);
        }
        let plane = |index: usize| -> Option<f32> {
            if let Some(st) = self.stems {
                let row = &st.data[index*len..(index+1)*len];
                return Some(cubic(row[taps[0]], row[si], row[taps[2]], row[taps[3]], sf));
            }
            let live = self.preview?.0;
            let first = taps[0];
            if let Some(span) = live.plane_span(index, first, taps[3]+1) {
                return Some(cubic(span[0], span[si-first], span[taps[2]-first], span[taps[3]-first], sf));
            }
            // Interpolating across the boundary between two finished slabs.
            Some(cubic(live.sample(index, first)?, live.sample(index, si)?, live.sample(index, taps[2])?, live.sample(index, taps[3])?, sf))
        };
        let part = |src: usize| Some((plane(2*src)?, plane(2*src+1)?));
        let (Some(d), Some(b), Some(v)) = (part(0), part(1), part(3)) else { return (ol, or) };
        // Instruments: the mix less drums, bass and vocals.
        let o = (ol - d.0 - b.0 - v.0, or - d.1 - b.1 - v.1);
        let mixed = (g[0] * ol + g[1] * d.0 + g[2] * b.0 + g[3] * o.0 + g[4] * v.0, g[0] * or + g[1] * d.1 + g[2] * b.1 + g[3] * o.1 + g[4] * v.1);
        if ready < len {
            // Blend into the original over the last 3.3 ms of a growing prefix.
            let wet = (((ready - 1) as f64 - sp) / (STEM_RAMP_SECS as f64 * crate::stems::htdemucs::SAMPLE_RATE as f64)).clamp(0.0, 1.0) as f32;
            (ol + wet * (mixed.0 - ol), or + wet * (mixed.1 - or))
        } else {
            mixed
        }
    }
    /// Left + right of whole source frame `i` (for aligning grains).
    #[inline]
    fn mono(&self, i: isize) -> f32 {
        let k = i.clamp(0, self.frames as isize - 1) as usize * 2;
        self.audio.samples[k] + self.audio.samples[k + 1]
    }
}

const ST_WIN: usize = 16384;
const ST_TMPL: usize = 1024;

/// Reads a sampler clip: plainly, or through the time stretcher when its pitch is held (MT)
/// while its speed follows the sampler's BPM.
pub struct ClipReader {
    st: Stretch,
}

impl ClipReader {
    pub fn new() -> ClipReader {
        ClipReader { st: Stretch::new() }
    }
    pub fn reset(&mut self) {
        self.st.active = false;
    }
    /// Left and right at source frame `pos`. `step`: source frames the position moves per
    /// output frame; `pitch_step`: source frames per output frame the sound is read at.
    #[inline]
    pub fn frame(&mut self, audio: &Decoded, pos: f64, step: f64, pitch_step: f64, hop: usize) -> (f32, f32) {
        let src = Src { audio, frames: audio.frames(), stems: None, preview: None, g: std::cell::Cell::new([1.0, 0.0, 0.0, 0.0, 0.0]), stem_ratio: 1.0 };
        if (step - pitch_step).abs() > 1e-9 {
            // A grain reads on at the pitch's rate while the position moves at `step`, so its
            // sound falls behind (or runs ahead) by up to hop x (step - pitch) frames before
            // the next grain starts. Starting each grain half of that ahead keeps a beat of a
            // tempo-synced loop within half a grain's drift of its place instead of always late.
            let lead = 0.5 * hop as f64 * (step - pitch_step);
            // A loop's wrap (the position goes back by more than a grain) starts a new grain
            // at once: fading into it would soften the first beat of every pass.
            if self.st.active && pos + lead < self.st.expect - hop as f64 {
                self.st.active = false;
            }
            let out = self.st.frame(&src, pos + lead, pitch_step, hop);
            // Where the next frame is expected: without this every frame looked like a seek
            // and the stretcher never got past the first sample of a grain.
            self.st.expect = pos + lead + step;
            out
        } else {
            self.st.active = false;
            src.at(pos)
        }
    }
}

/// The sampler deck joins the mix after the decks (before the master's effects and limiter).
/// With `rack`, and the sampler assigned to a Beat FX unit or CFX set to act on it, its mix
/// goes through them first (after its GAIN, before its fader: the decks' order).
fn mix_sampler(out: &mut [f32], cue: &mut [f32], channels: usize, rate: u32, rack: Option<&mut crate::fx::Rack>) {
    if channels != 2 {
        return;
    }
    if let Some(mut g) = lock_for_audio(&crate::sampler::SAMPLER) {
        if let Some(e) = g.as_deref_mut() {
            let fx = rack.filter(|r| r.sampler_in_use());
            e.direct = fx.is_none();
            e.process(out, cue, rate);
            if let Some(rack) = fx {
                let n = out.len() / 2;
                let (g, cue_g) = e.out_gains();
                let cue_g = if cue.len() >= out.len() { cue_g } else { 0.0 };
                let buf = e.mix_mut(n);
                rack.process(crate::fx::SAMPLER, buf, rate);
                let mut peak = (0.0f32, 0.0f32);
                for i in 0..n {
                    let (l, r) = (buf[i * 2], buf[i * 2 + 1]);
                    out[i * 2] += l * g;
                    out[i * 2 + 1] += r * g;
                    peak = (peak.0.max((l * g).abs()), peak.1.max((r * g).abs()));
                    if cue_g > 0.0 {
                        cue[i * 2] += l * cue_g;
                        cue[i * 2 + 1] += r * cue_g;
                    }
                }
                e.note_peak(peak.0, peak.1);
            }
        }
    }
}

/// The sampler deck is the sync master: BEAT SYNC followers hold their beats on its clock.
pub static SAMPLER_MASTER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Time stretcher (WSOLA: waveform-similarity overlap-add). The output is made of 40 ms
/// grains of the source, read at the wanted pitch and cross-faded every 20 ms; each new grain
/// starts where the play position says, moved by up to ±12 ms to the place where it best
/// continues the grain before it, so periodic material stays in phase across the joins. The
/// play position itself advances at exactly the wanted speed. It reads the source directly
/// (nothing is queued, no latency) and its buffers are allocated once, off the audio thread.
struct Stretch {
    active: bool,
    /// Source frame where the grain fading in began, and the one fading out.
    cur: f64,
    prev: f64,
    /// The two grains are the same stretch of audio (nothing to cross-fade).
    same: bool,
    /// Output frames into the current hop, and the hop length.
    n: usize,
    hop: usize,
    /// Source frames per output frame the grains are read at.
    ps: f64,
    /// The play position after the last frame, to notice seeks and loop jumps.
    expect: f64,
    win: Vec<f32>,
    tmpl: Vec<f32>,
    off: Vec<u32>,
}

impl Stretch {
    fn new() -> Stretch {
        Stretch { active: false, cur: 0.0, prev: 0.0, same: true, n: 0, hop: 882, ps: 1.0, expect: 0.0, win: vec![0.0; ST_WIN], tmpl: vec![0.0; ST_TMPL], off: vec![0; ST_TMPL] }
    }

    /// Where the next grain starts: near `nominal`, at the offset (within ±12 ms) whose audio
    /// best matches what follows the current grain (`natural`).
    fn pick(&mut self, src: &Src, nominal: f64, natural: f64) -> f64 {
        let sr = src.audio.sr as f64;
        let mut dec = 4usize;
        while self.hop / dec > ST_TMPL {
            dec *= 2;
        }
        let d = self.ps * dec as f64;
        let mut m = (self.hop / dec).max(1);
        let delta = ((0.012 * sr) as usize).clamp(8, ST_WIN / 4);
        while m > 8 && 2 * delta + (m as f64 * d) as usize + 4 > ST_WIN {
            m /= 2;
        }
        let span = 2 * delta + ((m - 1) as f64 * d).round() as usize + 2;
        if span > ST_WIN {
            return nominal;
        }
        let base = nominal.floor() as isize - delta as isize;
        for (i, w) in self.win[..span].iter_mut().enumerate() {
            *w = src.mono(base + i as isize);
        }
        let mut te = 0.0f32;
        for j in 0..m {
            let o = j as f64 * d;
            self.off[j] = o.round() as u32;
            let v = src.mono((natural + o).round() as isize);
            self.tmpl[j] = v;
            te += v * v;
        }
        if te < 1e-9 {
            return nominal;
        }
        let (tmpl, off, win) = (&self.tmpl[..m], &self.off[..m], &self.win[..span]);
        let score = |k: usize| -> f32 {
            let (mut c, mut e) = (0.0f32, 0.0f32);
            for j in 0..m {
                let v = win[k + off[j] as usize];
                c += tmpl[j] * v;
                e += v * v;
            }
            c / (e + 1e-9).sqrt()
        };
        let (mut best, mut best_k) = (f32::MIN, delta);
        let mut k = 0;
        while k <= 2 * delta {
            let sc = score(k);
            if sc > best {
                best = sc;
                best_k = k;
            }
            k += 2;
        }
        for k in [best_k.saturating_sub(1), (best_k + 1).min(2 * delta)] {
            let sc = score(k);
            if sc > best {
                best = sc;
                best_k = k;
            }
        }
        (base + best_k as isize) as f64 + (nominal - nominal.floor())
    }

    /// One output frame with the play position at source frame `pos`, read at `ps` source
    /// frames per output frame (the pitch); `hop` output frames per grain step.
    #[inline]
    fn frame(&mut self, src: &Src, pos: f64, ps: f64, hop: usize) -> (f32, f32) {
        let h = hop as f64;
        if !self.active || self.hop != hop {
            // Start: both grains are the audio from here on.
            self.active = true;
            self.hop = hop;
            self.ps = ps;
            self.cur = pos;
            self.prev = pos - h * ps;
            self.same = true;
            self.n = 0;
        } else if (pos - self.expect).abs() > 0.5 {
            // The position was moved (seek, loop, hot cue): fade from what was sounding into
            // the new place.
            self.prev = self.cur + (self.n as f64 - h) * self.ps;
            self.ps = ps;
            self.cur = pos;
            self.same = false;
            self.n = 0;
        } else if self.n >= hop {
            let natural = self.cur + h * self.ps;
            self.prev = self.cur;
            self.cur = if (ps - self.ps).abs() < 1e-12 && ps > 0.0 { self.pick(src, pos, natural) } else { pos };
            // The grain fading out keeps the pitch it was started with.
            self.prev += h * (self.ps - ps);
            self.ps = ps;
            self.same = (self.cur - natural).abs() < 1e-6;
            self.n = 0;
        }
        let n = self.n as f64;
        self.n += 1;
        let i = src.at(self.cur + n * self.ps);
        if self.same {
            return i;
        }
        let o = src.at(self.prev + (h + n) * self.ps);
        let w = 0.5 - 0.5 * (std::f32::consts::PI * ((n as f32 + 0.5) / h as f32)).cos();
        (o.0 + (i.0 - o.0) * w, o.1 + (i.1 - o.1) * w)
    }
}

/// Per-deck signal processing state owned by the audio callback.
struct DeckDsp {
    st: Stretch,
    eq: Iso3,
    /// Crossfader gain in use (moving toward the deck's `fader`).
    fader: f32,
    /// Gains in use for the original and the four parts (moving toward their targets).
    stem_g: [f32; 5],
    /// The Performance mixer's equalizer, and its TRIM x channel fader gains in use.
    eq3: crate::mixer::Eq3,
    trim: f32,
    ch_fader: f32,
    /// Where the position was going to be at the next frame (a difference means a jump).
    expect: f64,
    /// The source frame that sounds. It is the deck's position, except while the sound of
    /// the place a jump left fades out (`pending`): then it runs on and the position waits.
    snd: f64,
    pending: bool,
    /// Speed in use (1.0 = the track's own speed, negative backwards), and the vinyl ramp
    /// running: 0 none, 1 start, 2 stop.
    vel: f64,
    ramp: u8,
    /// Fade in use as a step of the fade curve (0 silent .. FADE_N - 1 full).
    att: u8,
    /// Output frames since a loop wrap or a quantized jump (for its fade in).
    since: u8,
    was_playing: bool,
    was_jog: crate::deck::Jog,
    /// Crossfader gain (2 ms ramp) and crossfader assign (3.3 ms out, then in).
    xfade: crate::deck::Ramp,
    assign: crate::deck::Ramp,
    route: u8,
    cfx: crate::cfx::Cfx,
}

impl DeckDsp {
    fn new() -> DeckDsp {
        DeckDsp { st: Stretch::new(), eq: Iso3::new(), fader: 1.0, stem_g: [1.0, 0.0, 0.0, 0.0, 0.0], eq3: crate::mixer::Eq3::new(), trim: 1.0, ch_fader: 1.0, expect: 0.0, snd: 0.0, pending: false, vel: 0.0, ramp: 0, att: 0, since: u8::MAX, was_playing: false, was_jog: crate::deck::Jog::None, xfade: crate::deck::Ramp::at(1.0), assign: crate::deck::Ramp::at(1.0), route: 255, cfx: crate::cfx::Cfx::new() }
    }
}

/// How often the audio callback found a deck's state held by the interface, and how often it
/// had to give the buffer up (silence) because the state stayed held.
pub static AUDIO_CONTENDED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static AUDIO_DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A deck's state for the audio callback. The interface holds it for well under a microsecond
/// at a time (to read the position or set a control), but it does so many times per frame, so
/// a single `try_lock` loses now and then, and losing means a whole buffer of silence: an
/// audible click. Waiting out such a hold costs far less than a buffer, so the callback spins
/// for it, and gives up only after a quarter of a millisecond (something is then really wrong).
fn lock_for_audio<T>(m: &Mutex<T>) -> Option<parking_lot::MutexGuard<'_, T>> {
    use std::sync::atomic::Ordering::Relaxed;
    if let Some(g) = m.try_lock() {
        return Some(g);
    }
    AUDIO_CONTENDED.fetch_add(1, Relaxed);
    let t = std::time::Instant::now();
    loop {
        for _ in 0..64 {
            std::hint::spin_loop();
        }
        if let Some(g) = m.try_lock() {
            return Some(g);
        }
        if t.elapsed() > std::time::Duration::from_micros(250) {
            AUDIO_DROPPED.fetch_add(1, Relaxed);
            return None;
        }
    }
}

/// The audio callback: both decks mixed into `out`.
fn render(states: &[Arc<Mutex<PlayState>>], out: &mut [f32], channels: usize, rate: u32) {
    render_cue(states, out, &mut [], channels, rate)
}

/// The deck BEAT SYNC followers hold their phase to (0 = deck 1).
pub static MASTER_DECK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The master share of the headphones output (MIX and LEVEL; f32 bits).
pub static HP_MASTER_GAIN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The open output stream has a headphones pair.
pub static HP_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// First channel of the headphones pair wanted (Preferences > Audio > Input/Output); -1 = none.
pub static HP_CHANNEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// BEAT SYNC: the follower plays at the master's tempo (set by the interface), and here its
/// beats are put on the master's: when they are more than 0.01 beat apart the deck jumps the
/// shortest way (rekordbox's `shouldJumpForSyncBeat` / `jumpToSyncBeat`; jumps under 10
/// frames are left). Not while it is bent, scratched, reversed, looping or mid-fade.
fn sync_hold(f: &mut PlayState, master: &[f64], t_m: f64, rate: u32) {
    let top = (crate::deck::FADE_N - 1) as u8;
    if f.dk.jog != crate::deck::Jog::None || f.dk.reverse || f.bend != 0.0 || f.dk.bend_acc != 0.0 || f.dk.bend_slope != 0.0 || f.loop_range.is_some() {
        return;
    }
    if f.smooth && (f.dsp.pending || f.dsp.ramp != 0 || f.dsp.att < top) {
        return;
    }
    let (Some(gf), Some(af)) = (f.grid.clone(), f.audio.clone()) else { return };
    let sr = af.sr as f64;
    let t_f = f.pos / sr;
    let (Some(pf), Some(pm)) = (beat_pos(&gf, t_f), beat_pos(master, t_m)) else { return };
    let r = if f.dk.sync_rate > 0.0 { f.dk.sync_rate } else { 1.0 };
    let e = crate::deck::sync_phase_error(pf, pm, r);
    if e.abs() <= 0.01 {
        return;
    }
    let i = gf.partition_point(|b| *b <= t_f).clamp(1, gf.len() - 1);
    let len = (gf[i] - gf[i - 1]) * sr;
    let jump = -e * r.min(1.0) * len;
    if jump.abs() < 10.0 {
        return;
    }
    // (The deck fades out for 43 frames before it moves; it lands where it should be by then.)
    let lead = if f.smooth { (top - 1) as f64 * (sr / rate as f64) * (f.tempo + f.sync_bend).clamp(0.0, 2.0) } else { 0.0 };
    f.pos = (f.pos + jump + lead).clamp(0.0, af.frames().saturating_sub(2) as f64);
}

/// `render`, also filling the headphones output `cue` (stereo; empty = none): the cued decks
/// before their faders plus the master mix, as the MIX knob balances them.
fn render_cue(states: &[Arc<Mutex<PlayState>>], out: &mut [f32], cue: &mut [f32], channels: usize, rate: u32) {
    for v in cue.iter_mut() {
        *v = 0.0;
    }
    for v in out.iter_mut() {
        *v = 0.0;
    }
    // Up to four decks. BEAT SYNC: every follower is held on the master deck's beat phase,
    // read here where all positions belong to the same instant.
    let mut guards: [Option<parking_lot::MutexGuard<'_, PlayState>>; 4] = [None, None, None, None];
    for (g, st) in guards.iter_mut().zip(states.iter()) {
        *g = lock_for_audio(st);
    }
    // Lift the common master control before Mix Point Link applies its per-deck
    // SILENT region. The muted part of a linked deck must also be silent on REC.
    let rec_lift = crate::recorder::lift_master(&mut guards);
    // MIX POINT LINK: a linked deck starts here, on the block its start position falls in.
    crate::mpl::pre_render(&mut guards);
    let master = MASTER_DECK.load(std::sync::atomic::Ordering::Relaxed) as usize;
    if master < states.len() {
        // The master's part of the comparison, taken before the followers are changed.
        let m = guards[master].as_deref().map(|m| (m.playing && m.dk.jog == crate::deck::Jog::None && !m.dk.reverse, m.search, m.grid.clone(), m.audio.as_ref().map(|a| a.sr), m.pos));
        for (i, g) in guards.iter_mut().enumerate() {
            if let Some(f) = g.as_deref_mut() {
                f.sync_bend = 0.0;
                if i == master {
                    continue;
                }
                if let Some((true, search, Some(gm), Some(sr), pos)) = m.clone() {
                    if f.sync_follow && f.playing && f.search == 0.0 && search == 0.0 {
                        sync_hold(f, &gm, pos / sr as f64, rate);
                    }
                }
            }
        }
    }
    // The sampler deck in the sync: as the master, every BEAT SYNC follower is held on its
    // beat clock; with its own BPM SYNC on, it is told where the master deck is in its beat.
    {
        use std::sync::atomic::Ordering::Relaxed;
        let mut phase = f64::NAN;
        if SAMPLER_MASTER.load(Relaxed) {
            let c = f64::from_bits(crate::sampler::CLOCK_BITS.load(Relaxed)).rem_euclid(1.0);
            for f in guards.iter_mut().filter_map(|g| g.as_deref_mut()) {
                f.sync_bend = 0.0;
                if f.sync_follow && f.playing && f.search == 0.0 {
                    if let (Some(gf), Some(af)) = (&f.grid, &f.audio) {
                        if let Some(p) = beat_phase(gf, f.pos / af.sr as f64) {
                            f.sync_bend = sync_correction(crate::sampler::phase_diff(p, c));
                        }
                    }
                }
            }
        } else if let Some(m) = guards.get(master).and_then(|g| g.as_deref()) {
            if let (true, true, Some(gm), Some(am)) = (m.playing, m.search == 0.0, &m.grid, &m.audio) {
                phase = beat_phase(gm, m.pos / am.sr as f64).unwrap_or(f64::NAN);
            }
        }
        crate::sampler::FOLLOW_PHASE.store(phase.to_bits(), Relaxed);
    }
    // With Beat FX in use each deck is rendered on its own, sent through the FX units
    // assigned to it (after its faders, as in rekordbox), then mixed; the master's effects
    // take the mix. Otherwise the decks go straight into the output.
    // (Waited for like the decks' state: the interface holds it only to copy the settings.)
    let mut rack = lock_for_audio(&crate::fx::RACK);
    match rack.as_deref_mut().and_then(|r| r.as_deref_mut()).filter(|r| r.active() && channels == 2) {
        Some(rack) => {
            rack.beat = [None; crate::fx::CHANNELS];
            for s in guards.iter_mut().filter_map(|g| g.as_deref_mut()) {
                let i = (s.deck_no as usize).min(3);
                let mut buf = std::mem::take(&mut rack.scratch[i]);
                if buf.len() < out.len() {
                    buf.resize(out.len(), 0.0);
                }
                buf[..out.len()].iter_mut().for_each(|v| *v = 0.0);
                // Beat FX quantize: where the deck is on its grid as this buffer starts.
                // (More than one state can carry a deck number; only a playing one sets it.)
                if let (true, Some(g), Some(a)) = (s.playing && s.search == 0.0, &s.grid, &s.audio) {
                    rack.beat[i] = beat_pos(g, s.pos / a.sr as f64);
                    if i == MASTER_DECK.load(std::sync::atomic::Ordering::Relaxed) as usize {
                        rack.beat[crate::fx::MASTER] = rack.beat[i];
                    }
                }
                render_deck_cue(s, &mut buf[..out.len()], cue, channels, rate);
                rack.process(i, &mut buf[..out.len()], rate);
                for (o, v) in out.iter_mut().zip(&buf) {
                    *o += *v;
                }
                rack.scratch[i] = buf;
            }
            mix_sampler(out, cue, channels, rate, Some(&mut *rack));
            rack.process(crate::fx::MASTER, out, rate);
        }
        None => {
            for s in guards.iter_mut().filter_map(|g| g.as_deref_mut()) {
                render_deck_cue(s, out, cue, channels, rate);
            }
            mix_sampler(out, cue, channels, rate, None);
        }
    }
    drop(rack);
    crate::recorder::restore_master(&mut guards, rec_lift);
    drop(guards);
    crate::edit_audio::mix(out, channels, rate);
    // The master section (Performance mixer): peak limiter, then the master level meter.
    use std::sync::atomic::Ordering::Relaxed;
    crate::recorder::tap(out, channels, rate, rec_lift);
    if let Some(mut lim) = MASTER_LIMITER.try_lock() {
        lim.process(out, channels, rate, LIMITER_ON.load(Relaxed));
    }
    // Headphones: the master mix joins the cued decks.
    let hp_master = f32::from_bits(HP_MASTER_GAIN.load(Relaxed));
    if hp_master > 0.0 && channels == 2 && cue.len() >= out.len() {
        for (c, o) in cue.iter_mut().zip(out.iter()) {
            *c += *o * hp_master;
        }
    }
    for (c, m) in MASTER_PEAK.iter().enumerate().take(channels.min(2)) {
        let p = out.iter().skip(c).step_by(channels.max(1)).fold(0.0f32, |a, v| a.max(v.abs()));
        if p > f32::from_bits(m.load(Relaxed)) {
            m.store(p.to_bits(), Relaxed);
        }
    }
}

static MASTER_LIMITER: Mutex<crate::mixer::Limiter> = Mutex::new(crate::mixer::Limiter::new());
/// The master peak limiter is in the signal path (the Performance mixer; off in Export).
pub static LIMITER_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Highest master output level since the meter last read it (L, R; f32 bits).
pub static MASTER_PEAK: [std::sync::atomic::AtomicU32; 2] = [std::sync::atomic::AtomicU32::new(0), std::sync::atomic::AtomicU32::new(0)];

/// Read and clear the master meter's peaks.
pub fn take_master_peak() -> [f32; 2] {
    use std::sync::atomic::Ordering::Relaxed;
    [f32::from_bits(MASTER_PEAK[0].swap(0, Relaxed)), f32::from_bits(MASTER_PEAK[1].swap(0, Relaxed))]
}

/// Time a part takes to fade fully in or out when it is muted or unmuted.
pub const STEM_RAMP_SECS: f32 = 0.0033;

/// STEM ISO: the gain for a knob position (0 = fully left, 0.5 = centre, 1 = fully right), as
/// rekordbox 7.2.19's 256-step curve (`partIsoCurve`, read at `round(value * 255)`): kill at
/// the left end with a soft toe, a straight line up to unity, a flat zone around the centre
/// (steps 124 to 131, so a centred knob leaves the original playing), then a straight line
/// up to +6 dB.
pub fn stem_iso_gain(value: f32) -> f32 {
    let i = (value.clamp(0.0, 1.0) * 255.0).round() as i32;
    match i {
        0..=9 => 0.0006 * (i * i) as f32,
        10 => 0.0591,
        11 => 0.0691,
        12 => 0.0791,
        13 => 0.0895,
        14..=123 => 1.0 - (124 - i) as f32 / 122.0,
        124..=131 => 1.0,
        132..=252 => 1.0 + (i - 131) as f32 * (0.9952623 / 122.0),
        _ => 1.9952623,
    }
}

/// Frames per search grain at the device rate (60 ms).
fn search_grain(rate: u32) -> usize {
    (rate as usize * 60 / 1000).max(1)
}

/// A zero-target one-pole gain can get stuck at a subnormal value indefinitely. Clear
/// only that tail, keeping all normal values and nonzero targets unchanged. Doing this
/// once per buffer adds no checks to the sample loop and bounds the tail to one buffer.
#[inline]
fn clear_zero_gain_tail(gain: &mut f32, target: f32) {
    if target == 0.0 && gain.is_subnormal() {
        *gain = target;
    }
}

/// Add one deck's audio to `out` (which the caller has cleared) and advance its position.
pub fn render_deck(s: &mut PlayState, out: &mut [f32], channels: usize, rate: u32) {
    render_deck_cue(s, out, &mut [], channels, rate)
}

/// `render_deck`, also adding the deck's headphones CUE feed to `cue` (stereo, as long as
/// `out` in frames; empty when there is no headphones output).
pub fn render_deck_cue(s: &mut PlayState, out: &mut [f32], cue: &mut [f32], channels: usize, rate: u32) {
    use crate::deck::{fade_curve, Jog, BEND_LIMIT, FADE_N};
    let searching = s.search != 0.0;
    let smooth = s.smooth;
    let jog = s.dk.jog;
    let top = (FADE_N - 1) as u8;
    // Nothing sounds and nothing moves: the deck rests where it is.
    if !s.playing && !searching && !matches!(jog, Jog::Speed(_)) && (!smooth || (s.dsp.att == 0 && s.dsp.ramp == 0)) && !crate::groove::wants_render(s) {
        s.search_count = 0;
        s.dsp.st.active = false;
        s.dsp.expect = s.pos;
        s.dsp.snd = s.pos;
        s.dsp.pending = false;
        s.dsp.vel = 0.0;
        s.dsp.ramp = 0;
        s.dsp.att = 0;
        s.dsp.was_playing = false;
        s.dsp.was_jog = jog;
        s.dk.vel = 0.0;
        if !s.slip {
            s.slip_pos = None;
        }
        return;
    }
    let Some(audio) = s.audio.clone() else { return };
    let step = audio.sr as f64 / rate as f64;
    let frames = audio.frames();
    let last = frames.saturating_sub(2) as f64;
    let vol = if s.muted { 0.0 } else { s.volume * s.gain * if searching { 0.7 } else { 1.0 } };
    let click_len = (rate as f64 * 0.03) as usize;
    let stems = s.stems.clone();
    let live = s.stem_preview.clone();
    let preview = live.as_deref().map(|p| (p, p.ready_frames()));
    let stem_ratio = crate::stems::htdemucs::SAMPLE_RATE as f64 / audio.sr.max(1) as f64;
    let preview_here = preview.is_some_and(|(p, ready)| ready >= 4 && (ready == p.len() || s.pos.max(0.0) * stem_ratio + 2.0 < ready as f64));
    // Where the gains of the original and the parts are heading. The parts are used only
    // while one of them is muted or off unity; until then the original plays untouched.
    let use_parts = (stems.is_some() || preview_here) && (0..4).any(|i| !s.stem_on[i] || s.stem_iso[i] != 1.0);
    let mut stem_target = [if use_parts { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0, 0.0];
    if use_parts {
        for i in 0..4 {
            stem_target[1 + i] = if s.stem_on[i] { s.stem_iso[i].clamp(0.0, 10.0) } else { 0.0 };
        }
    }
    if stems.is_none() && !preview_here {
        s.dsp.stem_g = [1.0, 0.0, 0.0, 0.0, 0.0];
    }
    // A full on/off change takes 3.3 ms (rekordbox's `udsp::GainController`: 145.53 frames
    // at 44.1 kHz), at a constant slope, so smaller moves are proportionally quicker.
    let stem_k = 1.0 / (STEM_RAMP_SECS * rate as f32);
    let src = Src { audio: &audio, frames, stems: stems.as_deref(), preview, g: std::cell::Cell::new(s.dsp.stem_g), stem_ratio };
    let metro = if searching { None } else { s.metronome.clone() };
    // Search grains: 60 ms, or the length the deck asks for (rekordbox's scan: 133 ms).
    let grain = if s.dk.scan_grain > 0.0 { ((s.dk.scan_grain * rate as f64) as usize).max(1) } else { search_grain(rate) };
    let n_out = out.len() / channels.max(1);
    // Bend slope (rekordbox's `BendTaskOnPreRender`): the bend grows by the slope for every
    // millisecond, up to +-0.42.
    if s.dk.bend_slope != 0.0 {
        s.dk.bend_acc = (s.dk.bend_acc + s.dk.bend_slope * n_out as f64 * 1000.0 / rate as f64).clamp(-BEND_LIMIT, BEND_LIMIT);
    }
    // Speed and pitch for this buffer. The base tempo is limited to -100 .. +100 % as in
    // rekordbox's engine (speed 0 .. 2). The pitch follows the speed unless Master Tempo
    // holds it; the key shift is on top of either.
    let speed = if searching { 1.0 } else { (s.tempo + s.bend + s.dk.bend_acc + s.sync_bend).clamp(0.0, 2.0) };
    let dir = if s.dk.reverse { -1.0 } else { 1.0 };
    let pitch = if searching { 1.0 } else { (if s.master_tempo { 1.0 } else { speed }) * 2f64.powf(s.key_shift as f64 / 12.0) };
    let stretch = (pitch - speed).abs() > 1e-9;
    let hop = ((rate as usize) / 50).clamp(64, 4096);
    let fader_k = (1.0 / (0.005 * rate as f32)).min(1.0);
    // The channel strip: none (Export), the equalizer (1: DJM-900NXS2 type, 3: DJM-900NXS
    // type), or the isolator (rekordbox's band gains: kill at the left end, flat at the
    // centre, +6 dB at the right, the same curve as STEM ISO).
    // A change of type fades the old one out and the new one in, 3.3 ms each (`EqIso`); the
    // DJM-900NXS type follows its knobs in filter steps (deck.rs).
    let want_strip = s.strip;
    let mut strip = s.dk.eqsw.begin(want_strip, rate);
    let mut eq_on = s.eq_on || strip == 2;
    if eq_on {
        s.dsp.eq.prepare(rate);
    }
    let eq_knobs = if strip == 3 {
        s.dk.nxs.advance(s.eq_pos, n_out, rate)
    } else {
        s.dk.nxs.ready = false;
        s.eq_pos
    };
    if strip == 1 || strip == 3 {
        s.dsp.eq3.nxs = strip == 3;
        s.dsp.eq3.prepare(rate, eq_knobs, n_out);
    }
    let iso_of = |strip: u8, pos: [f32; 3], gain: [f32; 3]| if strip == 2 { [stem_iso_gain(pos[0]), stem_iso_gain(pos[1]), stem_iso_gain(pos[2])] } else { gain };
    let mut iso_gain = iso_of(strip, s.eq_pos, s.eq_gain);
    let mut peak = s.peak;
    let cfx_on = strip != 0;
    if !stretch {
        s.dsp.st.active = false;
    }
    // Crossfader: the gain of the side the deck is assigned to moves in a 2 ms ramp; a new
    // assignment fades the old side out over 3.3 ms, then the new one in (rekordbox's
    // `CrossFaderVolumeControl` and `CrossFaderAssign`).
    let route_on = s.dk.route != 255;
    if route_on {
        let n33 = (0.0033 * rate as f32).round().max(1.0);
        let side = |r: u8| match r {
            1 => s.dk.xf[0],
            2 => s.dk.xf[1],
            _ => 1.0,
        };
        let d = &mut s.dsp;
        if d.route == 255 {
            d.route = s.dk.route;
            d.assign = crate::deck::Ramp::at(1.0);
            d.xfade = crate::deck::Ramp::at(side(d.route));
        }
        if d.route != s.dk.route && d.assign.idle() {
            if d.assign.cur > 0.0 {
                d.assign.set(0.0, n33);
            } else {
                d.route = s.dk.route;
                d.xfade = crate::deck::Ramp::at(side(d.route));
                d.assign.set(1.0, n33);
            }
        }
        d.xfade.set(side(d.route), (0.002 * rate as f32).round().max(1.0));
    }
    // Vinyl start and stop: the speed changes in a straight line, taking the set time for a
    // full speed's worth (rekordbox's `VinylSpeedAdjustor`). Which ramp runs is decided where
    // play, pause and the jog change.
    let (k_start, k_stop) = if smooth { (1000.0 / (s.dk.start_ms.max(0.1) as f64 * rate as f64), 1000.0 / (s.dk.stop_ms.max(0.1) as f64 * rate as f64)) } else { (f64::INFINITY, f64::INFINITY) };
    {
        let d = &mut s.dsp;
        if !smooth || searching {
            d.ramp = 0;
        } else if jog != d.was_jog {
            d.ramp = match jog {
                // A touch brakes the platter to a stop; letting go starts it again.
                Jog::Touch => 2,
                Jog::Speed(_) => d.ramp,
                Jog::None => s.playing as u8,
            };
        } else if s.playing != d.was_playing && jog == Jog::None {
            // With Master Tempo and less than two blocks of stop ramp, pause only fades.
            let short = d.vel.abs() / k_stop < 2.0 * 256.0 * rate as f64 / 44100.0;
            d.ramp = if s.playing { 1 } else if s.master_tempo && short { 0 } else { 2 };
        }
        // A real movement of the platter ends a ramp.
        if matches!(jog, Jog::Speed(v) if v.abs() >= 0.05) {
            d.ramp = 0;
        }
        d.was_jog = jog;
        d.was_playing = s.playing;
    }
    // SLIP: the background position starts when a loop begins, a pad is held, the deck is
    // reversed or scratched, and playback comes back to it when that ends. (Slip reverse
    // does the same without the SLIP button.)
    let slip_action = s.loop_range.is_some() || s.slip_hold || s.dk.reverse || jog != Jog::None;
    if s.playing && ((s.slip && slip_action) || (s.dk.reverse && s.dk.slip_reverse)) {
        if s.slip_pos.is_none() {
            s.slip_pos = Some(s.pos);
        }
    } else if let Some(b) = s.slip_pos.take() {
        if (s.slip || s.dk.slip_return) && s.playing {
            // (The deck fades out before it moves: it lands where the track is by then.)
            let lead = if smooth && s.dsp.att > 0 { (top - 1) as f64 * step * speed } else { 0.0 };
            s.pos = (b + lead).clamp(0.0, last);
        }
    }
    s.dk.slip_return = s.dk.reverse && s.dk.slip_reverse;
    // A position that is not where the last buffer left it has been moved. As in rekordbox
    // (`doFadeOutAndMovePlayingPos`), what was sounding fades out first (44 frames, raised
    // cosine), then the deck moves and fades in.
    if (s.pos - s.dsp.expect).abs() > 2.0 * step.max(1.0) {
        if smooth && s.dsp.att > 0 {
            s.dsp.pending = true;
        } else {
            s.dsp.snd = s.pos;
            s.dsp.pending = false;
        }
    } else if !s.dsp.pending {
        s.dsp.snd = s.pos;
    }
    let target = if searching {
        1.0
    } else {
        match jog {
            Jog::Speed(v) => v,
            Jog::Touch => 0.0,
            Jog::None => if s.playing { dir * speed } else { 0.0 },
        }
    };
    // The loop as whole source frames, so that every pass is the same samples. Its wrap is
    // smoothed (fade out before, fade in after) unless the loop is 2 ms or shorter.
    let mut lp = s.loop_range.map(|(a, b)| ((a * audio.sr as f64).round(), (b * audio.sr as f64).round())).filter(|(a, b)| b > a);
    let mut lp_smooth = smooth && lp.map(|(a, b)| (b - a) / audio.sr as f64 > crate::deck::LOOP_SMOOTH_SECS).unwrap_or(false);
    let cue_gain = if cue.len() * channels.max(1) >= out.len() * 2 { s.cue_gain * s.gain } else { 0.0 };
    // Groove Circuit: this deck's DRUM SWAP slots (and its drums under DRUM RELEASE FX).
    let gc = crate::groove::deck_block(s, &audio, speed, &mut stem_target, out.len() / channels.max(1), rate);
    for (fi, f) in out.chunks_mut(channels).enumerate() {
        let d = &mut s.dsp;
        // ---- speed
        match d.ramp {
            1 => {
                let diff = target - d.vel;
                if diff.abs() <= k_start {
                    d.vel = target;
                    d.ramp = 0;
                } else {
                    d.vel += k_start * diff.signum();
                }
            }
            2 => {
                if d.vel.abs() <= k_stop {
                    d.vel = 0.0;
                    d.ramp = 0;
                    d.att = 0;
                } else {
                    d.vel -= k_stop * d.vel.signum();
                }
            }
            // (After pause without a ramp the sound runs on for the length of the fade.)
            _ => {
                if target != 0.0 || d.att == 0 || !smooth || matches!(jog, Jog::Speed(_)) {
                    d.vel = target;
                }
            }
        }
        // ---- level
        let run = searching || matches!(jog, Jog::Speed(_)) || (s.playing && jog == Jog::None) || d.ramp != 0;
        let want = !d.pending && run;
        if !smooth {
            d.att = top;
        } else if want {
            if d.att < top {
                d.att += 1;
            }
        } else if d.att > 0 {
            d.att -= 1;
        }
        if d.att == 0 && d.pending {
            // Faded out: now the deck moves.
            d.pending = false;
            d.snd = s.pos;
            d.st.active = false;
        }
        if d.att == 0 && !(run && !d.pending) && d.ramp == 0 {
            d.vel = 0.0;
            d.expect = s.pos;
            break;
        }
        let g_stop = if d.ramp == 2 { fade_curve(((d.vel.abs() / k_stop) as usize).min(FADE_N - 1)) } else { 1.0 };
        // ---- position rules (for the deck's own sound, not for one that is fading out)
        let mut dip = 1.0f32;
        if !d.pending {
            if searching {
                // After each grain the position skips so that, overall, it moves at `search`
                // times normal speed (backward for negative values).
                s.search_count += 1;
                if s.search_count >= grain {
                    s.search_count = 0;
                    d.snd = (d.snd + (s.search - 1.0) * grain as f64 * step).clamp(0.0, last);
                }
            } else {
                // SLICER: its roll, return and loop-mode jumps (pad_audio.rs).
                s.pads.position(&mut d.snd, step * d.vel);
                // A quantized jump waiting for its place (sample exact, like a loop's wrap).
                if let Some((exit, enter)) = s.dk.warp {
                    if (d.vel > 0.0 && d.snd >= exit) || (d.vel < 0.0 && d.snd <= exit) {
                        d.snd = (enter + (d.snd - exit)).clamp(0.0, last);
                        s.dk.warp = None;
                        // (An auto beat loop entered on its lattice starts now; deck.rs.)
                        if let Some((a, b)) = s.dk.arm_loop.take() {
                            s.loop_range = Some((a, b));
                            lp = Some(((a * audio.sr as f64).round(), (b * audio.sr as f64).round())).filter(|(a, b)| b > a);
                            lp_smooth = smooth && (b - a) > crate::deck::LOOP_SMOOTH_SECS;
                        }
                        if smooth {
                            d.since = 0;
                        }
                    } else if smooth && d.vel != 0.0 {
                        let dist = (exit - d.snd) / (step * d.vel);
                        if dist >= 0.0 && dist < top as f64 {
                            dip = fade_curve(dist as usize);
                        }
                    }
                }
                if let Some((a, b)) = lp {
                    let len = b - a;
                    if d.vel >= 0.0 && d.snd >= b {
                        d.snd = a + (d.snd - b) % len;
                        if lp_smooth {
                            d.since = 0;
                        }
                    } else if d.vel < 0.0 && d.snd < a {
                        d.snd = b - (a - d.snd) % len;
                        if lp_smooth {
                            d.since = 0;
                        }
                    } else if lp_smooth && d.vel != 0.0 {
                        let dist = if d.vel > 0.0 { b - d.snd } else { d.snd - a } / (step * d.vel.abs());
                        if dist < top as f64 {
                            dip = dip.min(fade_curve(dist.max(0.0) as usize));
                        }
                    }
                }
            }
            if (d.since as usize) < FADE_N - 1 {
                dip = dip.min(fade_curve(d.since as usize));
                d.since += 1;
            }
        }
        // ---- the ends of the track
        let p = d.snd;
        if (p as usize + 1 >= frames && d.vel >= 0.0) || (p <= 0.0 && d.vel < 0.0) {
            if d.pending {
                d.pending = false;
                d.snd = s.pos;
                d.att = 0;
                continue;
            }
            if jog == Jog::None || searching {
                if p > 0.0 || !searching {
                    s.playing = false;
                }
                s.pos = if p <= 0.0 { 0.0 } else { frames.saturating_sub(1) as f64 };
                d.snd = s.pos;
                d.expect = s.pos;
                d.vel = 0.0;
                d.ramp = 0;
                d.att = 0;
                break;
            }
            // (Scratched against an end: the platter turns, the sound stays at the end.)
            d.snd = p.clamp(0.0, last);
        }
        let p = d.snd;
        if d.stem_g != stem_target {
            for (g, t) in d.stem_g.iter_mut().zip(stem_target) {
                *g += (t - *g).clamp(-stem_k, stem_k);
            }
            src.g.set(d.stem_g);
        }
        // Key lock only while the deck plays at its own speed: ramps, scratching and a sound
        // that is fading out are plain resampling (rekordbox's `updateTimeStretchMode`).
        let plain = !stretch || d.pending || d.ramp != 0 || jog != Jog::None || searching || d.vel != dir * speed;
        let (mut l, mut r) = if plain {
            d.st.active = false;
            src.at(p)
        } else {
            d.st.frame(&src, p, pitch * step * dir, hop)
        };
        // The settled attenuator is unity. Skip the trigonometric curve on ordinary
        // playback, while still applying seek/loop dips and a moving stop ramp.
        if smooth && (d.att < top || dip != 1.0 || g_stop != 1.0) {
            // The fade curve of rekordbox's `Attenuator`: a raised cosine over 44 frames.
            let g = fade_curve(d.att as usize) * dip * g_stop;
            l *= g;
            r *= g;
        }
        // (Groove Circuit's slots after the deck's own fades: a slot also sounds while the
        // deck stands.)
        if let Some(g) = gc.as_ref() {
            let (gl, gr) = g.at(fi);
            l += gl;
            r += gr;
        }
        if eq_on {
            for k in 0..3 {
                d.eq.g[k] += (iso_gain[k] - d.eq.g[k]) * fader_k;
            }
            l = d.eq.run(0, l);
            r = d.eq.run(1, r);
        }
        if strip != 0 {
            d.trim += (s.trim - d.trim) * fader_k;
            l *= d.trim;
            r *= d.trim;
            if strip == 1 || strip == 3 {
                l = d.eq3.run(0, l);
                r = d.eq3.run(1, r);
            }
            let (eg, switched) = s.dk.eqsw.next(want_strip, rate);
            if let Some(m) = switched {
                // The new type starts from a clean state (`initialize`).
                strip = m;
                eq_on = s.eq_on || strip == 2;
                d.eq = Iso3::new();
                d.eq.prepare(rate);
                d.eq3 = crate::mixer::Eq3::new();
                d.eq3.nxs = strip == 3;
                s.dk.nxs.ready = false;
                let k = if strip == 3 { s.dk.nxs.advance(s.eq_pos, 0, rate) } else { s.eq_pos };
                d.eq3.prepare(rate, k, n_out);
                iso_gain = iso_of(strip, s.eq_pos, s.eq_gain);
            }
            l *= eg;
            r *= eg;
            // Sound Color FX: after the EQ, before the meter, the CUE feed and the faders.
            if cfx_on {
                let mut fr = [l, r];
                d.cfx.process(s.cfx_kind, s.cfx_color, s.cfx_param, &mut fr, rate);
                l = fr[0];
                r = fr[1];
            }
            // The level meter's tap: after the EQ, before the faders (with the track's gain).
            peak[0] = peak[0].max((l * s.gain).abs());
            peak[1] = peak[1].max((r * s.gain).abs());
            // The headphones CUE feed is taken at the same point.
            if cue_gain > 0.0 {
                cue[fi * 2] += l * cue_gain;
                cue[fi * 2 + 1] += r * cue_gain;
            }
        }
        // ACT. CENSR: the effect inside a censored region (pad_audio.rs).
        s.pads.sound(p, &mut l, &mut r, rate);
        d.ch_fader += (s.ch_fader - d.ch_fader) * fader_k;
        let xf = if route_on {
            d.xfade.next() * d.assign.next()
        } else {
            d.fader += (s.fader - d.fader) * fader_k;
            d.fader
        };
        let vol = vol * xf * d.ch_fader;
        let mut click = 0.0f32;
        if let Some(beats) = metro.as_ref() {
            // The beat list can be replaced by a shorter one (another track, a grid edit).
            if s.next_click > beats.len() {
                s.next_click = 0;
            }
            let t = p / audio.sr as f64;
            while s.next_click < beats.len() && beats[s.next_click] < t - 0.05 {
                s.next_click += 1;
            }
            if s.next_click > 0 && beats[s.next_click - 1] > t {
                s.next_click = 0;
            }
            if s.next_click < beats.len() && t >= beats[s.next_click] {
                s.click_phase = 0;
                s.next_click += 1;
            }
            if s.click_phase < click_len {
                let env = 1.0 - s.click_phase as f32 / click_len as f32;
                click = (s.click_phase as f32 * 2.0 * std::f32::consts::PI * 1500.0 / rate as f32).sin() * env * s.metronome_vol * if s.muted { 0.0 } else { 1.0 };
                s.click_phase += 1;
            }
        }
        if channels >= 2 {
            f[0] += l * vol + click;
            f[1] += r * vol + click;
        } else {
            f[0] += 0.5 * (l + r) * vol + click;
        }
        // ---- move on
        d.snd += step * d.vel;
        if jog != Jog::None {
            d.snd = d.snd.clamp(0.0, last);
        }
        d.st.expect = d.snd;
        if !d.pending {
            s.pos = d.snd;
        }
        d.expect = s.pos;
        if s.playing {
            if let Some(b) = s.slip_pos.as_mut() {
                *b += step * speed;
            }
        }
    }
    s.dk.vel = s.dsp.vel;
    s.peak = peak;
    clear_zero_gain_tail(&mut s.dsp.ch_fader, s.ch_fader);
    clear_zero_gain_tail(&mut s.dsp.fader, s.fader);
    if strip != 0 {
        clear_zero_gain_tail(&mut s.dsp.trim, s.trim);
    }
    if eq_on {
        for (gain, target) in s.dsp.eq.g.iter_mut().zip(iso_gain) {
            clear_zero_gain_tail(gain, target);
        }
    }
}

/// Metronome click level for the three volume steps of the GRID panel (1 = quiet … 3 = loud).
pub fn metronome_level(step: u8) -> f32 {
    match step {
        0 | 1 => 0.2,
        2 => 0.45,
        _ => 0.9,
    }
}

pub fn has_audio_extension(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("mp3" | "m4a" | "aac" | "mp4" | "wav" | "aif" | "aiff" | "flac" | "alac" | "ogg")
    )
}

#[cfg(test)]
mod gapless_probe {
    #[test]
    #[ignore]
    fn trim_amounts() {
        use symphonia::core::formats::FormatOptions;
        for p in std::env::var("PREPDECK_AUDIO").unwrap_or_default().split('|').filter(|s| !s.is_empty()) {
            let mut n = [0usize; 2];
            for (k, g) in [false, true].iter().enumerate() {
                let file = std::fs::File::open(p).unwrap();
                let mss = symphonia::core::io::MediaSourceStream::new(Box::new(file), Default::default());
                let mut hint = symphonia::core::probe::Hint::new();
                hint.with_extension("mp3");
                let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions { enable_gapless: *g, ..Default::default() }, &Default::default()).unwrap();
                let mut f = probed.format;
                let tr = f.default_track().unwrap().clone();
                if k == 1 { eprintln!("delay {:?} padding {:?}", tr.codec_params.delay, tr.codec_params.padding); }
                let mut dec = symphonia::default::get_codecs().make(&tr.codec_params, &Default::default()).unwrap();
                while let Ok(pk) = f.next_packet() {
                    if let Ok(b) = dec.decode(&pk) { n[k] += b.frames(); }
                }
            }
            eprintln!("raw {} gapless {} diff {} :: {}", n[0], n[1], n[0] as i64 - n[1] as i64, p.rsplit('/').next().unwrap());
        }
    }
}

#[cfg(test)]
mod device_probe {
    #[test]
    #[ignore]
    fn lists_output_devices() {
        let t = std::time::Instant::now();
        let d = super::Player::output_devices_within(std::time::Duration::from_secs(5));
        eprintln!("devices after {:?}: {d:?}", t.elapsed());
    }
}

#[cfg(test)]
mod damaged_probe {
    /// Decode a file given in PREPDECK_DAMAGED and report skipped packets. --ignored.
    #[test]
    #[ignore]
    fn reports_damaged_packets() {
        let Ok(p) = std::env::var("PREPDECK_DAMAGED") else { return };
        let d = super::decode_file(&p).unwrap();
        println!("damaged packets: {}, truncated {}, duration {:.1}s", d.damaged, d.truncated, d.duration());
        let (l, r) = d.samples.chunks_exact(2).fold((0f32, 0f32), |a, f| (a.0.max(f[0].abs()), a.1.max(f[1].abs())));
        println!("peak left {l:.3}, right {r:.3}");
    }
}

#[cfg(test)]
mod interp_tests {
    #[test]
    fn cubic_is_exact_at_frames_and_better_than_linear() {
        assert_eq!(super::cubic(0.1, 0.2, 0.7, 0.4, 0.0), 0.2);
        assert!((super::cubic(0.1, 0.2, 0.7, 0.4, 1.0) - 0.7).abs() < 1e-6);
        // A 10 kHz sine sampled at 44.1 kHz, read halfway between samples.
        let f = |n: f64| (2.0 * std::f64::consts::PI * 10_000.0 * n / 44_100.0).sin() as f32;
        let (mut e_lin, mut e_cub) = (0f32, 0f32);
        for k in 1..400 {
            let n = k as f64;
            let truth = f(n + 0.5);
            e_lin += (0.5 * (f(n) + f(n + 1.0)) - truth).abs();
            e_cub += (super::cubic(f(n - 1.0), f(n), f(n + 1.0), f(n + 2.0), 0.5) - truth).abs();
        }
        assert!(e_cub < e_lin * 0.6, "cubic {e_cub} vs linear {e_lin}");
    }
}

#[cfg(test)]
mod deck_mix_tests {
    use super::*;

    fn dc(level: f32, frames: usize) -> Arc<Decoded> {
        Arc::new(Decoded { sr: 44100, samples: vec![level; frames * 2], raw_frames: frames, damaged: 0, truncated: false })
    }
    fn playing(level: f32) -> PlayState {
        PlayState { audio: Some(dc(level, 44100)), playing: true, volume: 1.0, gain: 1.0, ..Default::default() }
    }

    #[test]
    fn playback_disabled_never_opens_an_output() {
        let mut player = super::Player::with_device(None, false);
        assert!(player.buffer_load_percent().is_none());
        player.switch_device(None).unwrap();
        #[cfg(target_os = "linux")]
        assert!(player.service_output(true, true, None).unwrap().is_none());
        assert!(player.stream.is_none());
        assert!(player.buffer_load_percent().is_none());
    }

    #[test]
    fn two_decks_are_summed_and_a_paused_deck_is_silent() {
        let (mut a, mut b) = (playing(0.25), playing(0.5));
        let mut out = vec![0.0f32; 512];
        render_deck(&mut a, &mut out, 2, 44100);
        render_deck(&mut b, &mut out, 2, 44100);
        assert!(out.iter().all(|v| (*v - 0.75).abs() < 1e-5), "{:?}", &out[..4]);
        assert_eq!(a.pos, 256.0);
        assert_eq!(b.pos, 256.0);
        // Deck B paused: only deck A sounds, and B's position stays.
        b.playing = false;
        let mut out = vec![0.0f32; 512];
        render_deck(&mut a, &mut out, 2, 44100);
        render_deck(&mut b, &mut out, 2, 44100);
        assert!(out.iter().all(|v| (*v - 0.25).abs() < 1e-5));
        assert_eq!(b.pos, 256.0);
        // Mute silences a deck but it keeps moving.
        a.muted = true;
        let mut out = vec![0.0f32; 512];
        render_deck(&mut a, &mut out, 2, 44100);
        assert!(out.iter().all(|v| *v == 0.0));
        assert_eq!(a.pos, 768.0);
    }

    #[test]
    fn search_moves_at_its_speed_in_both_directions_even_when_paused() {
        let mut s = PlayState { audio: Some(dc(0.5, 44100 * 60)), volume: 1.0, ..Default::default() };
        s.pos = 44100.0 * 30.0;
        s.search = 8.0;
        let mut out = vec![0.0f32; 44100 * 2];
        render_deck(&mut s, &mut out, 2, 44100);
        // One second of output: about eight seconds of track, and it is audible.
        let moved = (s.pos - 44100.0 * 30.0) / 44100.0;
        assert!((moved - 8.0).abs() < 0.5, "{moved}");
        assert!(out.iter().any(|v| *v > 0.1));
        s.search = -8.0;
        let from = s.pos;
        let mut out = vec![0.0f32; 44100 * 2];
        render_deck(&mut s, &mut out, 2, 44100);
        let moved = (s.pos - from) / 44100.0;
        assert!((moved + 8.0).abs() < 0.5, "{moved}");
        // Search off and not playing: silent, position kept.
        s.search = 0.0;
        let from = s.pos;
        let mut out = vec![0.0f32; 512];
        render_deck(&mut s, &mut out, 2, 44100);
        assert!(out.iter().all(|v| *v == 0.0));
        assert_eq!(s.pos, from);
    }

    #[test]
    fn metronome_has_three_rising_levels() {
        assert!(metronome_level(1) < metronome_level(2) && metronome_level(2) < metronome_level(3));
        let mut s = playing(0.0);
        s.metronome = Some(Arc::new(vec![0.0]));
        let peak = |s: &mut PlayState, step: u8| {
            s.pos = 0.0;
            s.next_click = 0;
            s.click_phase = usize::MAX;
            s.metronome_vol = metronome_level(step);
            let mut out = vec![0.0f32; 2048];
            render_deck(s, &mut out, 2, 44100);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };
        let (p1, p2, p3) = (peak(&mut s, 1), peak(&mut s, 2), peak(&mut s, 3));
        assert!(p1 > 0.05 && p2 > p1 * 1.5 && p3 > p2 * 1.5, "{p1} {p2} {p3}");
    }
}

#[cfg(test)]
mod dsp_tests {
    use super::*;

    fn tone(freq: f64, secs: f64, amp: impl Fn(f64) -> f32) -> Arc<Decoded> {
        let sr = 44100u32;
        let n = (secs * sr as f64) as usize;
        let mut samples = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f64 / sr as f64;
            let v = (2.0 * std::f64::consts::PI * freq * t).sin() as f32 * amp(t);
            samples.push(v);
            samples.push(v);
        }
        Arc::new(Decoded { sr, samples, raw_frames: n, damaged: 0, truncated: false })
    }

    fn deck(a: Arc<Decoded>) -> PlayState {
        PlayState { audio: Some(a), playing: true, volume: 1.0, gain: 1.0, ..Default::default() }
    }

    /// Render `secs` of output in device-sized buffers; left channel.
    fn run(s: &mut PlayState, secs: f64) -> Vec<f32> {
        let mut all = vec![];
        let mut buf = vec![0.0f32; 1024];
        let total = (secs * 44100.0) as usize;
        while all.len() < total {
            let n = (total - all.len()).min(512);
            buf[..n * 2].iter_mut().for_each(|v| *v = 0.0);
            render_deck(s, &mut buf[..n * 2], 2, 44100);
            all.extend(buf[..n * 2].chunks(2).map(|f| f[0]));
        }
        all
    }

    /// Frequency of a steady tone from its upward zero crossings (first to last).
    fn freq_of(x: &[f32]) -> f64 {
        let mut first = None;
        let mut last = 0.0f64;
        let mut count = 0usize;
        for i in 1..x.len() {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                let t = (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64;
                if first.is_none() {
                    first = Some(t);
                } else {
                    count += 1;
                }
                last = t;
            }
        }
        count as f64 / ((last - first.unwrap_or(0.0)) / 44100.0)
    }

    fn cents(f: f64, reference: f64) -> f64 {
        1200.0 * (f / reference).log2()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn tempo_without_master_tempo_changes_speed_and_pitch_together() {
        let mut s = deck(tone(440.0, 12.0, |_| 0.5));
        s.tempo = 1.1;
        let out = run(&mut s, 4.0);
        assert!((s.pos - 4.0 * 44100.0 * 1.1).abs() < 1.0, "{}", s.pos);
        assert!(cents(freq_of(&out[4410..]), 484.0).abs() < 2.0, "{}", freq_of(&out[4410..]));
    }

    #[test]
    fn master_tempo_keeps_the_pitch_within_a_few_cents_and_the_tempo_exact() {
        for (tempo, f) in [(1.16, 440.0), (0.84, 440.0), (1.06, 110.0), (0.92, 2000.0), (1.03, 55.0)] {
            let mut s = deck(tone(f, 14.0, |_| 0.5));
            s.tempo = tempo;
            s.master_tempo = true;
            let out = run(&mut s, 5.0);
            // The position moved at exactly the tempo…
            assert!((s.pos - 5.0 * 44100.0 * tempo).abs() < 1.0, "tempo {tempo}: {}", s.pos);
            // …the pitch stayed…
            let c = cents(freq_of(&out[8820..]), f);
            assert!(c.abs() < 3.0, "tempo {tempo}, {f} Hz: {c} cents");
            // …and the level did too (no holes or doubling at the joins).
            let lv = rms(&out[8820..]) / (0.5 * std::f32::consts::FRAC_1_SQRT_2);
            assert!((lv - 1.0).abs() < 0.03, "tempo {tempo}, {f} Hz: level {lv}");
        }
    }

    #[test]
    fn master_tempo_output_follows_the_source_position() {
        // The tone gets louder at 4.0 s of the track; at 125 % that is heard after 3.2 s.
        let mut s = deck(tone(330.0, 12.0, |t| if t < 4.0 { 0.2 } else { 0.8 }));
        s.tempo = 1.25;
        s.master_tempo = true;
        let out = run(&mut s, 5.0);
        let at = |t: f64| rms(&out[(t * 44100.0) as usize..((t + 0.02) * 44100.0) as usize]);
        assert!(at(3.15) < 0.2, "{}", at(3.15));
        assert!(at(3.25) > 0.5, "{}", at(3.25));
    }

    #[test]
    fn key_shift_moves_the_pitch_by_semitones_at_the_same_speed() {
        for semis in [3.0f32, -2.0, 12.0, -5.0] {
            let mut s = deck(tone(440.0, 10.0, |_| 0.5));
            s.key_shift = semis;
            let out = run(&mut s, 4.0);
            assert!((s.pos - 4.0 * 44100.0).abs() < 1.0);
            let want = 440.0 * 2f64.powf(semis as f64 / 12.0);
            let c = cents(freq_of(&out[8820..]), want);
            assert!(c.abs() < 3.0, "{semis} semitones: {c} cents");
        }
        // With a tempo change and Master Tempo off the shift is on top of the tempo's pitch.
        let mut s = deck(tone(440.0, 10.0, |_| 0.5));
        s.key_shift = 2.0;
        s.tempo = 1.05;
        let out = run(&mut s, 4.0);
        let want = 440.0 * 1.05 * 2f64.powf(2.0 / 12.0);
        assert!(cents(freq_of(&out[8820..]), want).abs() < 3.0);
        assert!((s.pos - 4.0 * 44100.0 * 1.05).abs() < 1.0);
    }

    #[test]
    fn a_seek_while_stretching_lands_on_the_new_position() {
        let mut s = deck(tone(440.0, 20.0, |t| if t < 10.0 { 0.2 } else { 0.8 }));
        s.tempo = 0.9;
        s.master_tempo = true;
        run(&mut s, 1.0);
        s.pos = 12.0 * 44100.0;
        let out = run(&mut s, 0.5);
        assert!(rms(&out[4410..]) > 0.5);
        assert!((s.pos - (12.0 + 0.45) * 44100.0).abs() < 2.0);
    }

    #[test]
    fn pitch_bend_adds_to_the_tempo_while_it_is_held() {
        let mut s = deck(tone(440.0, 10.0, |_| 0.5));
        s.tempo = 1.02;
        s.bend = 0.04;
        run(&mut s, 1.0);
        assert!((s.pos - 44100.0 * 1.06).abs() < 1.0);
        s.bend = 0.0;
        let from = s.pos;
        run(&mut s, 1.0);
        assert!((s.pos - from - 44100.0 * 1.02).abs() < 1.0);
    }

    #[test]
    fn crossfader_law() {
        // Middle: both decks at full level. Ends: one deck only.
        assert_eq!(crossfader_gains(0.5), (1.0, 1.0));
        let (a, b) = crossfader_gains(0.0);
        assert!(a == 1.0 && b == 0.0);
        let (a, b) = crossfader_gains(1.0);
        assert!(a.abs() < 1e-6 && b == 1.0);
        // Deck B fades in over the first half and deck A out over the second, without steps.
        let mut prev = crossfader_gains(0.0);
        for i in 1..=100 {
            let g = crossfader_gains(i as f32 / 100.0);
            assert!(g.0 <= prev.0 + 1e-6 && g.1 >= prev.1 - 1e-6);
            assert!((g.0 - prev.0).abs() < 0.04 && (g.1 - prev.1).abs() < 0.04);
            prev = g;
        }
        assert!((crossfader_gains(0.75).0 - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        // Symmetric.
        assert!((crossfader_gains(0.3).1 - crossfader_gains(0.7).0).abs() < 1e-6);
    }

    #[test]
    fn crossfader_gain_reaches_the_output_smoothly() {
        let dc = Arc::new(Decoded { sr: 44100, samples: vec![0.5; 44100 * 2], raw_frames: 44100, damaged: 0, truncated: false });
        let mut s = deck(dc);
        s.fader = 0.0;
        let out = run(&mut s, 0.2);
        // No step at the start, silent once the fade is through.
        assert!(out.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01));
        assert!(out[4410..].iter().all(|v| v.abs() < 1e-4));
    }

    #[test]
    fn isolator_band_gains() {
        let sine = |f: f64| -> Vec<f32> { (0..44100).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / 44100.0).sin() as f32).collect() };
        let level = |g: [f32; 3], f: f64| -> f32 { rms(&Iso3::process(44100, g, &sine(f))[22050..]) / std::f32::consts::FRAC_1_SQRT_2 };
        let db = |x: f32| 20.0 * x.max(1e-9).log10();
        // All bands on: the level is unchanged everywhere, the crossovers included.
        for f in [30.0, 60.0, 284.0, 700.0, 1000.0, 4649.0, 8000.0, 15000.0] {
            assert!(db(level([1.0; 3], f)).abs() < 0.1, "{f} Hz: {} dB", db(level([1.0; 3], f)));
        }
        // LOW off: bass gone, mids and highs untouched; -6 dB at the crossover.
        assert!(db(level([0.0, 1.0, 1.0], 60.0)) < -45.0);
        assert!(db(level([0.0, 1.0, 1.0], 1000.0)).abs() < 0.2);
        assert!(db(level([0.0, 1.0, 1.0], 8000.0)).abs() < 0.1);
        assert!((db(level([0.0, 1.0, 1.0], ISO_LOW_HZ as f64)) + 6.0).abs() < 0.3);
        // MID off: only the band between the crossovers goes.
        assert!(db(level([1.0, 0.0, 1.0], 800.0)) < -18.0);
        assert!(db(level([1.0, 0.0, 1.0], 40.0)).abs() < 0.2);
        assert!(db(level([1.0, 0.0, 1.0], 12000.0)).abs() < 0.2);
        // HIGH off.
        assert!(db(level([1.0, 1.0, 0.0], 18000.0)) < -40.0);
        assert!(db(level([1.0, 1.0, 0.0], 100.0)).abs() < 0.1);
        // Everything off is silence.
        assert!(db(level([0.0; 3], 1000.0)) < -100.0);
    }

    #[test]
    fn isolator_in_the_deck_follows_the_kill_switches() {
        let mut s = deck(tone(60.0, 10.0, |_| 0.5));
        s.eq_on = true;
        let on = rms(&run(&mut s, 1.0)[22050..]);
        s.eq_gain = [0.0, 1.0, 1.0];
        let off = rms(&run(&mut s, 1.0)[22050..]);
        assert!((on - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01, "{on}");
        assert!(off < 0.002, "{off}");
    }

    #[test]
    fn sync_phase_maths() {
        let grid = |first: f64, bpm: f64, n: usize| -> Vec<f64> { (0..n).map(|i| first + i as f64 * 60.0 / bpm).collect() };
        let a = grid(0.5, 120.0, 400);
        assert!((beat_phase(&a, 0.5).unwrap()).abs() < 1e-9);
        assert!((beat_phase(&a, 0.75).unwrap() - 0.5).abs() < 1e-9);
        // Before the first and after the last beat the grid is continued.
        assert!((beat_phase(&a, 0.125).unwrap() - 0.25).abs() < 1e-9);
        assert!((beat_phase(&a, 0.5 + 400.0 * 0.5 + 0.1).unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(beat_phase(&[1.0], 1.0), None);
        // A 128 BPM track against it: on its beat while the master is a quarter beat in.
        let b = grid(0.2, 128.0, 400);
        let e = phase_error(&b, b[10], &a, a[7] + 0.125).unwrap();
        assert!((e + 0.25).abs() < 1e-9, "{e}");
        // Errors wrap to the nearest beat.
        let e = phase_error(&b, b[10] + 0.9 * 60.0 / 128.0, &a, a[7]).unwrap();
        assert!((e + 0.1).abs() < 1e-9, "{e}");
        // Ahead: slow down; behind: speed up; tiny errors are left alone; bounded.
        assert!(sync_correction(0.1) < 0.0 && sync_correction(-0.1) > 0.0);
        assert_eq!(sync_correction(0.001), 0.0);
        assert_eq!(sync_correction(0.5), -0.08);
        // Tempo match, with the octave rule.
        assert!((sync_rate(128.0, 120.0) - 128.0 / 120.0).abs() < 1e-12);
        assert!((sync_rate(140.0, 70.0) - 1.0).abs() < 1e-12);
        assert!((sync_rate(87.0, 174.0) - 1.0).abs() < 1e-12);
        assert_eq!(sync_rate(0.0, 120.0), 1.0);
    }

    #[test]
    fn beat_sync_pulls_the_follower_onto_the_masters_beats() {
        // Master at 120 BPM, follower a 126 BPM track played at 120/126 and started 80 ms late.
        let grid = |first: f64, bpm: f64| -> Arc<Vec<f64>> { Arc::new((0..600).map(|i| first + i as f64 * 60.0 / bpm).collect()) };
        let silence = || Arc::new(Decoded { sr: 44100, samples: vec![0.0; 44100 * 2 * 60], raw_frames: 44100 * 60, damaged: 0, truncated: false });
        let m = Arc::new(Mutex::new(PlayState { grid: Some(grid(0.0, 120.0)), ..deck(silence()) }));
        let f = Arc::new(Mutex::new(PlayState { grid: Some(grid(0.3, 126.0)), tempo: 120.0 / 126.0, sync_follow: true, master_tempo: true, ..deck(silence()) }));
        f.lock().pos = (0.3 + 0.08) * 44100.0;
        let states = [m.clone(), f.clone()];
        let mut buf = vec![0.0f32; 1024];
        for _ in 0..(10 * 44100 / 512) {
            render(&states, &mut buf, 2, 44100);
        }
        let (pm, pf) = (m.lock().pos / 44100.0, f.lock().pos / 44100.0);
        let e = phase_error(f.lock().grid.as_ref().unwrap(), pf, m.lock().grid.as_ref().unwrap(), pm).unwrap();
        assert!(e.abs() < 0.01, "phase error after 10 s: {e} beats");
        // Without BEAT SYNC the offset stays.
        f.lock().sync_follow = false;
        f.lock().pos += 0.1 * 44100.0;
        for _ in 0..(2 * 44100 / 512) {
            render(&states, &mut buf, 2, 44100);
        }
        let (pm, pf) = (m.lock().pos / 44100.0, f.lock().pos / 44100.0);
        let e = phase_error(f.lock().grid.as_ref().unwrap(), pf, m.lock().grid.as_ref().unwrap(), pm).unwrap();
        assert!(e.abs() > 0.15, "{e}");
    }

    #[test]
    fn beat_sync_jumps_when_more_than_a_hundredth_of_a_beat_off() {
        // Both at 120 BPM (22050 frames a beat). `off`: how far the follower starts ahead.
        let grid = || -> Arc<Vec<f64>> { Arc::new((0..600).map(|i| i as f64 * 0.5).collect()) };
        let silence = || Arc::new(Decoded { sr: 44100, samples: vec![0.0; 44100 * 2 * 30], raw_frames: 44100 * 30, damaged: 0, truncated: false });
        let error_after = |off_beats: f64, rate: f64, smooth: bool| -> f64 {
            let m = Arc::new(Mutex::new(PlayState { grid: Some(grid()), smooth, ..deck(silence()) }));
            let f = Arc::new(Mutex::new(PlayState { grid: Some(grid()), sync_follow: true, smooth, tempo: rate, ..deck(silence()) }));
            f.lock().dk.sync_rate = rate;
            let states = [m.clone(), f.clone()];
            let mut buf = vec![0.0f32; 512];
            // Both start together, the follower off the beat.
            m.lock().pos = 22050.0;
            f.lock().pos = 22050.0 * rate + off_beats * 22050.0;
            for _ in 0..40 {
                render(&states, &mut buf, 2, 44100);
            }
            let (pm, pf) = (m.lock().pos / 22050.0, f.lock().pos / 22050.0);
            crate::deck::sync_phase_error(pf, pm, rate)
        };
        for smooth in [false, true] {
            // Under the threshold the deck is left where it is.
            assert!((error_after(0.009, 1.0, smooth) - 0.009).abs() < 1e-3, "{}", error_after(0.009, 1.0, smooth));
            assert!((error_after(-0.008, 1.0, smooth) + 0.008).abs() < 1e-3);
            // Over it, it jumps onto the beat (the shortest way, either direction).
            assert!(error_after(0.02, 1.0, smooth).abs() < 2e-3, "{}", error_after(0.02, 1.0, smooth));
            assert!(error_after(-0.3, 1.0, smooth).abs() < 2e-3);
            assert!(error_after(0.45, 1.0, smooth).abs() < 2e-3);
        }
        // At double rate every second beat of the follower falls on a master beat.
        assert!(error_after(0.2, 2.0, false).abs() < 2e-3);
    }
}

#[cfg(test)]
mod stem_mix_tests {
    use super::*;

    #[test]
    fn progressive_stems_play_only_the_published_prefix() {
        let (mut player, original, parts) = deck();
        let completed = player.stems.take().unwrap();
        let preview = crate::stems::Live::test_prefix(completed.len, &completed.data, 16384);
        player.stem_preview = Some(preview.clone());
        assert!(player.has_playable_stems());
        let src = Src {
            audio: player.audio.as_ref().unwrap(),
            frames: original.len(),
            stems: None,
            preview: Some((&preview, preview.ready_frames())),
            g: std::cell::Cell::new([0.0, 1.0, 0.0, 0.0, 0.0]),
            stem_ratio: 1.0,
        };
        for frame in [1000, 8191, 8192, 12000] {
            assert_eq!(src.at(frame as f64), (parts[0][frame], parts[0][frame]));
        }
        // Forward seek past the completed prefix and a cubic read straddling its end.
        for frame in [16383, 20000] {
            assert_eq!(src.at(frame as f64), (original[frame], original[frame]));
        }
        player.stem_preview = None;
        assert!(!player.has_playable_stems());
    }

    /// A track whose parts are known: original = drums + bass + other + vocals, where the
    /// stored "other" is deliberately wrong (the instruments are taken as the residual).
    fn deck() -> (PlayState, Vec<f32>, [Vec<f32>; 4]) {
        let n = 44100;
        let tone = |f: f32, a: f32| -> Vec<f32> { (0..n).map(|i| (i as f32 * f * std::f32::consts::TAU / 44100.0).sin() * a).collect() };
        let parts = [tone(60.0, 0.2), tone(110.0, 0.15), tone(440.0, 0.1), tone(880.0, 0.12)];
        let mix: Vec<f32> = (0..n).map(|i| parts.iter().map(|p| p[i]).sum()).collect();
        let mut data = Vec::new();
        for (k, p) in parts.iter().enumerate() {
            let stored: Vec<f32> = if k == 2 { p.iter().map(|x| x * 0.5).collect() } else { p.clone() };
            data.extend_from_slice(&stored);
            data.extend_from_slice(&stored);
        }
        let audio = Decoded { sr: 44100, samples: mix.iter().flat_map(|x| [*x, *x]).collect(), raw_frames: n, damaged: 0, truncated: false };
        let s = PlayState { audio: Some(Arc::new(audio)), stems: Some(Arc::new(crate::stems::Stems { len: n, data })), playing: true, volume: 1.0, gain: 1.0, ..Default::default() };
        (s, mix, parts)
    }

    fn run(s: &mut PlayState, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        render_deck(s, &mut out, 2, 44100);
        out.chunks(2).map(|c| c[0]).collect()
    }

    #[test]
    fn all_parts_on_is_the_original_bit_for_bit() {
        let (mut s, mix, _) = deck();
        let out = run(&mut s, 4000);
        assert!(out.iter().zip(&mix).all(|(a, b)| a == b));
    }

    #[test]
    fn a_muted_part_fades_out_over_3_3_ms_and_the_rest_is_the_mix_without_it() {
        let (mut s, mix, parts) = deck();
        run(&mut s, 1000);
        s.stem_on[3] = false; // vocals
        let out = run(&mut s, 1000);
        // No jump at the switch: the first frame is still (almost) the whole mix.
        assert!((out[0] - mix[1000]).abs() < 0.01, "{} vs {}", out[0], mix[1000]);
        // Half way through the fade about half of the vocal is left.
        let half = 1000 + 73;
        let left = (out[73] - (mix[half] - parts[3][half])) / parts[3][half];
        assert!((left - 0.5).abs() < 0.08, "{left}");
        // After 146 frames (3.3 ms) it is the mix less the vocals, exactly as rekordbox sums it.
        for i in 150..1000 {
            let want = mix[1000 + i] - parts[3][1000 + i];
            assert!((out[i] - want).abs() < 1e-4, "frame {i}: {} vs {want}", out[i]);
        }
        // Back on: the original returns, bit for bit once the fade is over.
        s.stem_on[3] = true;
        let out = run(&mut s, 1000);
        assert!(out[200..].iter().zip(&mix[2200..]).all(|(a, b)| a == b));
    }

    #[test]
    fn instruments_are_what_is_left_of_the_mix() {
        let (mut s, mix, parts) = deck();
        s.stem_on = [false, false, true, false];
        let out = run(&mut s, 2000);
        // The stored "other" is half level; the residual is the true instruments part.
        for i in 300..2000 {
            assert!((out[i] - parts[2][i]).abs() < 1e-4, "frame {i}");
        }
        let _ = mix;
    }

    #[test]
    fn stem_iso_curve_and_gain() {
        assert_eq!(stem_iso_gain(0.0), 0.0);
        assert_eq!(stem_iso_gain(0.5), 1.0);
        assert!((stem_iso_gain(1.0) - 1.9952623).abs() < 1e-6);
        assert!((stem_iso_gain(64.0 / 255.0) - 0.5082).abs() < 1e-4);
        assert!((stem_iso_gain(16.0 / 255.0) - 0.1148).abs() < 1e-4);
        assert!((stem_iso_gain(132.0 / 255.0) - 1.0081578).abs() < 1e-5);
        assert!((stem_iso_gain(9.0 / 255.0) - 0.0486).abs() < 1e-6);
        // Drums at +6 dB: the mix plus one more helping of drums.
        let (mut s, mix, parts) = deck();
        s.stem_iso[0] = stem_iso_gain(1.0);
        let out = run(&mut s, 2000);
        for i in 300..2000 {
            assert!((out[i] - (mix[i] + 0.9952623 * parts[0][i])).abs() < 1e-4);
        }
    }
}

#[cfg(test)]
mod strip_tests {
    use super::*;

    fn deck() -> PlayState {
        let n = 44100;
        let x: Vec<f32> = (0..n).flat_map(|i| { let v = (i as f32 * 440.0 * std::f32::consts::TAU / 44100.0).sin() * 0.5; [v, v] }).collect();
        PlayState { audio: Some(Arc::new(Decoded { sr: 44100, samples: x, raw_frames: n, damaged: 0, truncated: false })), playing: true, volume: 1.0, gain: 1.0, ..Default::default() }
    }

    fn rms(s: &mut PlayState) -> f32 {
        let mut out = vec![0.0f32; 8192];
        render_deck(s, &mut out, 2, 44100);
        let mut out = vec![0.0f32; 8192];
        render_deck(s, &mut out, 2, 44100);
        (out.iter().map(|v| v * v).sum::<f32>() / out.len() as f32).sqrt()
    }

    #[test]
    fn performance_strip_applies_trim_fader_and_feeds_the_meter_before_the_fader() {
        // No strip (Export): the track as it is, and no meter reading.
        let mut s = deck();
        let plain = rms(&mut s);
        assert_eq!(s.peak, [0.0; 2]);
        // Strip with everything at unity and a flat EQ: the same level; the meter reads the peak.
        let mut s = PlayState { strip: 1, ..deck() };
        assert!((rms(&mut s) / plain - 1.0).abs() < 1e-3);
        assert!((s.peak[0] - 0.5).abs() < 0.01);
        // Fader at -12 dB: the output falls, the meter (before the fader) does not.
        let mut s = PlayState { strip: 1, ch_fader: crate::mixer::channel_fader_gain(0.5, 1), ..deck() };
        let half = rms(&mut s);
        assert!((20.0 * (half / plain).log10() + 12.0).abs() < 0.3, "{}", 20.0 * (half / plain).log10());
        assert!((s.peak[0] - 0.5).abs() < 0.01);
        // Trim fully right: +8.8 dB on both.
        let mut s = PlayState { strip: 1, trim: crate::mixer::trim_gain(1.0), ..deck() };
        assert!((20.0 * (rms(&mut s) / plain).log10() - 8.79).abs() < 0.1);
        // Isolator with MID at the left end: a 440 Hz tone (between the crossovers) falls to
        // what the LOW band still passes of it (about -16 dB, 0.6 octave above 284 Hz).
        let mut s = PlayState { strip: 2, eq_pos: [0.5, 0.0, 0.5], ..deck() };
        assert!(20.0 * (rms(&mut s) / plain).log10() < -12.0);
        // ...and with LOW at the left end too it is gone.
        let mut s = PlayState { strip: 2, eq_pos: [0.0, 0.0, 0.5], ..deck() };
        assert!(20.0 * (rms(&mut s) / plain).log10() < -45.0);
        // Equalizer with LOW at the left end: 440 Hz is barely touched (the cut shelf lies
        // below 250 Hz), where the isolator's LOW takes 16 dB off it.
        let mut s = PlayState { strip: 1, eq_pos: [0.0, 0.5, 0.5], ..deck() };
        let d = 20.0 * (rms(&mut s) / plain).log10();
        assert!(d < -0.3 && d > -3.0, "{d}");
    }
}

#[cfg(test)]
mod smooth_tests {
    use super::*;

    fn deck() -> PlayState {
        let n = 44100 * 4;
        // A ramp, so that a sample's value tells its position.
        let x: Vec<f32> = (0..n).flat_map(|i| { let v = 0.2 + 0.6 * i as f32 / n as f32; [v, v] }).collect();
        PlayState { audio: Some(Arc::new(Decoded { sr: 44100, samples: x, raw_frames: n, damaged: 0, truncated: false })), playing: true, volume: 1.0, gain: 1.0, smooth: true, ..Default::default() }
    }

    fn run(s: &mut PlayState, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        render_deck(s, &mut out, 2, 44100);
        out.chunks(2).map(|c| c[0]).collect()
    }

    fn max_step(x: &[f32]) -> f32 {
        x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
    }

    fn settle_at_play_speed(s: &mut PlayState) {
        s.dsp.att = (crate::deck::FADE_N - 1) as u8;
        s.dsp.vel = 1.0;
        s.dsp.was_playing = true;
    }

    #[test]
    fn settled_play_fade_preserves_every_output_sample() {
        for rate in [22050, 44100, 48000, 96000] {
            for channels in [1, 2, 4] {
                let (mut smooth, mut plain) = (deck(), deck());
                settle_at_play_speed(&mut smooth);
                plain.smooth = false;
                for frames in [1, 33, 512, 1025] {
                    let (mut a, mut b) = (vec![0.0; frames * channels], vec![0.0; frames * channels]);
                    render_deck(&mut smooth, &mut a, channels, rate);
                    render_deck(&mut plain, &mut b, channels, rate);
                    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
                            "rate {rate}, channels {channels}, frames {frames}");
                    assert_eq!(smooth.pos, plain.pos);
                    assert_eq!(smooth.dsp.att as usize, crate::deck::FADE_N - 1);
                }
            }
        }
    }

    #[test]
    fn gain_cleanup_preserves_normal_values_and_nonzero_targets() {
        for target in [0.0f32, -0.0] {
            for bits in [1u32, 0x007f_ffff, 0x8000_0001, 0x807f_ffff] {
                let mut gain = f32::from_bits(bits);
                clear_zero_gain_tail(&mut gain, target);
                assert_eq!(gain.to_bits(), target.to_bits());
            }
            for bits in [0u32, 0x8000_0000, f32::MIN_POSITIVE.to_bits(), (-f32::MIN_POSITIVE).to_bits(),
                1.0f32.to_bits(), f32::INFINITY.to_bits(), f32::NEG_INFINITY.to_bits(), 0x7fc0_0123] {
                let mut gain = f32::from_bits(bits);
                clear_zero_gain_tail(&mut gain, target);
                assert_eq!(gain.to_bits(), bits);
            }
        }
        for target in [f32::from_bits(1), -f32::from_bits(1), f32::MIN_POSITIVE, 1.0, f32::INFINITY, f32::NAN] {
            let mut gain = f32::from_bits(0x007f_ffff);
            clear_zero_gain_tail(&mut gain, target);
            assert_eq!(gain.to_bits(), 0x007f_ffff);
        }
    }

    #[test]
    fn normal_fader_response_matches_the_original_envelope() {
        // Keep the pre-cleanup one-pole envelope as an independent output expectation.
        // This includes a zero target, but remains far above underflow for this interval.
        for channel in [false, true] {
            for target in [0.0, -0.0, 0.1, 0.7, 1.0] {
                let mut s = deck();
                settle_at_play_speed(&mut s);
                if channel { s.ch_fader = target; } else { s.fader = target; }
                let audio = s.audio.clone().unwrap();
                let (mut gain, mut frame) = (1.0f32, 0usize);
                let k = 1.0 / (0.005 * 44100.0f32);
                for frames in [1, 7, 64, 512] {
                    let mut out = vec![0.0f32; frames * 2];
                    render_deck(&mut s, &mut out, 2, 44100);
                    for stereo in out.chunks_exact(2) {
                        gain += (target - gain) * k;
                        for ch in 0..2 {
                            assert_eq!(stereo[ch].to_bits(), (audio.samples[frame * 2 + ch] * gain).to_bits());
                        }
                        frame += 1;
                    }
                    let rendered = if channel { s.dsp.ch_fader } else { s.dsp.fader };
                    assert_eq!(rendered.to_bits(), gain.to_bits());
                }
            }
        }
    }

    #[test]
    fn zero_gain_tails_settle_silently_and_controls_recover() {
        for rate in [44100u32, 48000, 96000] {
            for control in 0..5 {
                let mut s = deck();
                match control {
                    0 => s.ch_fader = 0.0,
                    1 => s.fader = 0.0,
                    2 => { s.strip = 1; s.trim = 0.0; }
                    3 => { s.eq_on = true; s.eq_gain = [0.0; 3]; }
                    _ => { s.strip = 2; s.eq_pos = [0.0; 3]; }
                }
                let mut out = [0.0f32; 1024];
                for _ in 0..(rate as usize * 2).div_ceil(512) {
                    out.fill(0.0);
                    render_deck(&mut s, &mut out, 2, rate);
                }
                let gains: &[f32] = match control {
                    0 => std::slice::from_ref(&s.dsp.ch_fader),
                    1 => std::slice::from_ref(&s.dsp.fader),
                    2 => std::slice::from_ref(&s.dsp.trim),
                    _ => &s.dsp.eq.g,
                };
                assert!(gains.iter().all(|g| g.to_bits() == 0), "rate {rate}, control {control}, {gains:?}");
                let pos = s.pos;
                out.fill(0.0);
                render_deck(&mut s, &mut out, 2, rate);
                assert!(s.playing && s.pos > pos);
                assert!(out.iter().all(|v| *v == 0.0));
                match control {
                    0 => s.ch_fader = 1.0,
                    1 => s.fader = 1.0,
                    2 => s.trim = 1.0,
                    3 => s.eq_gain = [1.0; 3],
                    _ => s.eq_pos = [0.5; 3],
                }
                out.fill(0.0);
                render_deck(&mut s, &mut out, 2, rate);
                assert!(out.iter().all(|v| v.is_finite()) && out.iter().any(|v| v.abs() > 0.01));
            }
        }
    }

    /// Run with a clocked ALSA output (a PulseAudio null sink is sufficient).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a working Linux audio output"]
    fn linux_output_parks_only_after_idle_and_resumes_the_same_deck() {
        use std::time::{Duration, Instant};
        let mut player = Player::new();
        assert!(player.stream.is_some(), "{:?}", player.error);
        let audio = Arc::new(Decoded { sr: 44100, samples: vec![0.2; 44100 * 2 * 4],
            raw_frames: 44100 * 4, damaged: 0, truncated: false });
        player.decks[1].lock().audio = Some(audio.clone());
        player.decks[1].lock().pos = 4410.0;
        let keep = player.service_output(false, false, None).unwrap().unwrap();
        assert!(keep > Duration::ZERO);
        player.idle_since = Some(Instant::now() - Duration::from_secs(3));
        player.service_output(false, false, None).unwrap();
        assert!(player.parked && player.stream.is_none());
        assert_eq!(player.decks[1].lock().pos, 4410.0);
        player.decks[1].lock().playing = true;
        let running = player.transport_running();
        assert!(running);
        player.service_output(false, running, None).unwrap();
        assert!(player.stream.is_some() && !player.parked);
        let until = Instant::now() + Duration::from_secs(3);
        while player.decks[1].lock().pos == 4410.0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut deck = player.decks[1].lock();
        assert!(Arc::ptr_eq(deck.audio.as_ref().unwrap(), &audio));
        assert!(deck.pos > 4410.0);
        deck.playing = false;
        deck.search = 1.0;
        drop(deck);
        assert!(player.transport_running());
        player.decks[1].lock().search = 0.0;
        player.idle_since = Some(Instant::now() - Duration::from_secs(3));
        player.service_output(true, false, None).unwrap();
        assert!(player.stream.is_some() && !player.parked);
    }

    /// Opt-in CPU report using the actual callback, synthetic audio and no device.
    /// `cargo test --release cpu_playback_cost -- --ignored --nocapture --test-threads=1`
    #[test]
    #[ignore]
    fn cpu_playback_cost() {
        use std::hint::black_box;
        // The first three cases reproduce one deck with default playback controls,
        // including a common 44.1 kHz file on a 48 kHz Linux output.
        for (label, source_rate, rate, decks, smooth, strip, kind, color) in [
            ("default-44k", 44100u32, 44100u32, 1, true, 0, 0, 0.5),
            ("default-44k-to-48k", 44100, 48000, 1, true, 0, 0, 0.5),
            ("default-48k", 48000, 48000, 1, true, 0, 0, 0.5),
            ("plain-control", 48000, 48000, 1, false, 0, 0, 0.5),
            ("two-decks", 44100, 48000, 2, true, 0, 0, 0.5),
            ("four-decks", 44100, 48000, 4, true, 0, 0, 0.5),
            ("filter", 48000, 48000, 1, true, 1, 0, 0.17),
            ("gate-left", 48000, 48000, 1, true, 1, 8, 0.17),
            ("gate-right", 48000, 48000, 1, true, 1, 8, 0.83),
        ] {
            let n = source_rate as usize * 21;
            let samples = (0..n).flat_map(|i| {
                let v = (i as f32 * 330.0 * std::f32::consts::TAU / source_rate as f32).sin() * 0.3;
                [v, v * 0.8]
            }).collect();
            let audio = Arc::new(Decoded { sr: source_rate, samples, raw_frames: n, damaged: 0, truncated: false });
            let mut elapsed = Vec::new();
            let mut checksum = 0.0f64;
            for _ in 0..5 {
                let states: Vec<_> = (0..decks).map(|deck_no| Arc::new(Mutex::new(PlayState {
                    audio: Some(audio.clone()), playing: true, volume: 1.0, gain: 1.0,
                    smooth, strip, cfx_kind: kind, cfx_color: color, cfx_param: 0.63,
                    deck_no, ..Default::default() }))).collect();
                let mut out = [0.0f32; 1024];
                // Settle the play fade, knob glide and filter coefficients before timing.
                for _ in 0..16 { render(black_box(&states), black_box(&mut out), 2, rate); }
                let start = std::time::Instant::now();
                let blocks = rate as usize * 20 / 512;
                for _ in 0..blocks { render(black_box(&states), black_box(&mut out), 2, rate); }
                elapsed.push(start.elapsed().as_secs_f64());
                checksum += out.iter().map(|v| *v as f64).sum::<f64>();
            }
            elapsed.sort_by(f64::total_cmp);
            println!("cpu playback: {label}, source_rate {source_rate}, rate {rate}, decks {decks}, frames 512, median_seconds {:.9}, checksum {:.12}", elapsed[2], checksum);
        }
    }

    /// Actual callback cost after a zero fader has had time to decay below normal f32.
    /// The same benchmark runs before and after gain cleanup; the gain bits show whether
    /// the old subnormal plateau or an exact zero was timed.
    /// `cargo test --release cpu_zero_gain_cost -- --ignored --nocapture --test-threads=1`
    #[test]
    #[ignore]
    fn cpu_zero_gain_cost() {
        use std::hint::black_box;
        let source_rate = 44100u32;
        let n = source_rate as usize * 23;
        let samples = (0..n).flat_map(|i| {
            let v = (i as f32 * 330.0 * std::f32::consts::TAU / source_rate as f32).sin() * 0.3;
            [v, v * 0.8]
        }).collect();
        let audio = Arc::new(Decoded { sr: source_rate, samples, raw_frames: n, damaged: 0, truncated: false });
        for rate in [44100u32, 48000] {
            for (label, ch_fader, fader) in [("unity", 1.0, 1.0), ("channel-zero", 0.0, 1.0), ("cross-zero", 1.0, 0.0)] {
                let mut elapsed = Vec::new();
                let mut checksum = 0.0f64;
                let mut gain_bits = (0, 0);
                for _ in 0..5 {
                    let states: [Arc<Mutex<PlayState>>; 4] = std::array::from_fn(|deck_no| Arc::new(Mutex::new(if deck_no == 0 {
                        PlayState { audio: Some(audio.clone()), playing: true, volume: 1.0, gain: 1.0,
                            smooth: true, ch_fader, fader, ..Default::default() }
                    } else {
                        PlayState { deck_no: deck_no as u8, ..Default::default() }
                    })));
                    let mut out = [0.0f32; 1024];
                    // Two seconds settle even the old one-pole subnormal plateau. The source
                    // lasts 23 seconds so it keeps playing throughout the 20 timed seconds.
                    for _ in 0..(rate as usize * 2).div_ceil(512) {
                        render(black_box(&states), black_box(&mut out), 2, rate);
                    }
                    let start_pos = {
                        let s = states[0].lock();
                        gain_bits = (s.dsp.ch_fader.to_bits(), s.dsp.fader.to_bits());
                        if ch_fader == 0.0 { assert!(s.dsp.ch_fader.abs() < f32::MIN_POSITIVE); }
                        if fader == 0.0 { assert!(s.dsp.fader.abs() < f32::MIN_POSITIVE); }
                        s.pos
                    };
                    let blocks = rate as usize * 20 / 512;
                    let start = std::time::Instant::now();
                    for _ in 0..blocks {
                        render(black_box(&states), black_box(&mut out), 2, rate);
                    }
                    elapsed.push(start.elapsed().as_secs_f64());
                    black_box(&out);
                    checksum += out.iter().map(|v| *v as f64).sum::<f64>();
                    let s = states[0].lock();
                    assert!(s.playing && s.pos > start_pos + source_rate as f64 * 19.0);
                }
                elapsed.sort_by(f64::total_cmp);
                println!("cpu zero gain: {label}, source_rate {source_rate}, rate {rate}, frames 512, median_seconds {:.9}, gain_bits {gain_bits:?}, checksum {:.12}", elapsed[2], checksum);
            }
        }
    }

    #[test]
    fn play_pause_and_jumps_do_not_step() {
        let mut s = deck();
        // Play: the level rises over rekordbox's 44-step fade instead of starting at 0.2.
        let a = run(&mut s, 1000);
        assert!(a[0].abs() < 0.01 && (a[999] - 0.2).abs() < 0.01);
        assert!(max_step(&a) < 0.01);
        // A jump of two seconds (0.3 in level): what sounded fades out (the old place runs
        // on meanwhile), then the deck moves and fades in. No step; silent in between.
        s.pos += 88200.0;
        let b = run(&mut s, 1000);
        assert!(max_step(&b) < 0.02, "{}", max_step(&b));
        assert!((b[999] - 0.5).abs() < 0.02);
        assert!(b[10] > 0.15 && b[10] < 0.21 && b[42] == 0.0 && b[90] > 0.49);
        // Pause: the vinyl stop ramp (1 ms at the default setting) and the fade; the deck
        // comes to rest within that millisecond.
        s.playing = false;
        let pos = s.pos;
        let c = run(&mut s, 1000);
        assert!(max_step(&c) < 0.03 && c[999] == 0.0 && c[0] > 0.4, "{}", max_step(&c));
        assert!(s.pos >= pos && s.pos - pos < 45.0, "{}", s.pos - pos);
        // A loop wrap is at the same sample with and without smoothing; smoothed (loops
        // over 2 ms) it dips to silence around the wrap instead of stepping.
        let mut s = deck();
        run(&mut s, 1000);
        s.loop_range = Some((0.0, 1500.0 / 44100.0));
        let d = run(&mut s, 1000);
        assert!(max_step(&d) < 0.01, "{}", max_step(&d));
        let mut h = PlayState { smooth: false, ..deck() };
        run(&mut h, 1000);
        h.loop_range = Some((0.0, 1500.0 / 44100.0));
        let e = run(&mut h, 1000);
        assert!(max_step(&e) > 0.005);
        // (The smoothed deck is the 1 ms start ramp behind: half of 44 frames.)
        assert!((h.pos - s.pos - 21.5).abs() < 1.0, "{}", h.pos - s.pos);
        assert!(d.iter().skip(100).any(|v| *v < 0.001));
    }

    #[test]
    fn slip_returns_to_where_the_track_would_be() {
        let mut s = PlayState { slip: true, ..deck() };
        run(&mut s, 2000);
        let at = s.pos;
        // A loop of 500 frames for 3000 frames, then out of it.
        s.loop_range = Some((2000.0 / 44100.0, 2500.0 / 44100.0));
        run(&mut s, 3000);
        assert!(s.pos < 2501.0 && s.slip_pos.is_some());
        s.loop_range = None;
        run(&mut s, 100);
        assert!((s.pos - (at + 3100.0)).abs() < 1.5, "{}", s.pos);
        assert!(s.slip_pos.is_none());
        // Without SLIP the loop's end is where playback goes on.
        let mut s = deck();
        run(&mut s, 2000);
        s.loop_range = Some((2000.0 / 44100.0, 2500.0 / 44100.0));
        run(&mut s, 3000);
        s.loop_range = None;
        run(&mut s, 100);
        assert!(s.pos < 2700.0);
        // A held hot cue with SLIP: back to the background position on release.
        let mut s = PlayState { slip: true, ..deck() };
        run(&mut s, 2000);
        let at = s.pos;
        s.slip_pos = Some(s.pos);
        s.slip_hold = true;
        s.pos = 40000.0;
        run(&mut s, 1000);
        // (The pad's place is reached after the 1 ms fade out.)
        assert!((s.pos - 40958.0).abs() < 1.5, "{}", s.pos);
        s.slip_hold = false;
        run(&mut s, 100);
        assert!((s.pos - (at + 1100.0)).abs() < 1.5, "{}", s.pos);
    }
}

#[cfg(test)]
mod cue_tests {
    use super::*;

    #[test]
    fn a_cued_deck_reaches_the_headphones_before_its_fader() {
        let n = 44100;
        let x: Vec<f32> = (0..n).flat_map(|_| [0.25f32, 0.25]).collect();
        let deck = || PlayState { audio: Some(Arc::new(Decoded { sr: 44100, samples: x.clone(), raw_frames: n, damaged: 0, truncated: false })), playing: true, volume: 1.0, gain: 1.0, strip: 1, ..Default::default() };
        // Fader down, CUE on: silent on the master, present in the headphones.
        let mut s = PlayState { ch_fader: 0.0, cue_gain: 0.5, ..deck() };
        s.dsp.ch_fader = 0.0;
        let mut out = vec![0.0f32; 2048];
        let mut cue = vec![0.0f32; 2048];
        render_deck_cue(&mut s, &mut out, &mut cue, 2, 44100);
        assert!(out.iter().all(|v| v.abs() < 1e-6));
        assert!((cue[2000] - 0.125).abs() < 0.01, "{}", cue[2000]);
        // CUE off: nothing in the headphones. Outside Performance (no strip): nothing either.
        let mut s = deck();
        let mut cue = vec![0.0f32; 2048];
        render_deck_cue(&mut s, &mut vec![0.0f32; 2048], &mut cue, 2, 44100);
        assert!(cue.iter().all(|v| *v == 0.0));
        let mut s = PlayState { strip: 0, cue_gain: 1.0, ..deck() };
        let mut cue = vec![0.0f32; 2048];
        render_deck_cue(&mut s, &mut vec![0.0f32; 2048], &mut cue, 2, 44100);
        assert!(cue.iter().all(|v| *v == 0.0));
    }
}

#[cfg(test)]
mod beat_pos_tests {
    #[test]
    fn beat_pos_counts_beats_from_the_grids_first_beat() {
        let grid: Vec<f64> = (0..8).map(|i| 1.0 + i as f64 * 0.5).collect();
        let at = |t: f64| super::beat_pos(&grid, t).unwrap();
        assert!((at(1.0) - 0.0).abs() < 1e-9 && (at(1.25) - 0.5).abs() < 1e-9 && (at(3.125) - 4.25).abs() < 1e-9);
        // Before the first beat and after the last the grid carries on at its end spacing.
        assert!((at(0.75) + 0.5).abs() < 1e-9, "{}", at(0.75));
        assert!((at(5.25) - 8.5).abs() < 1e-9, "{}", at(5.25));
        assert!(super::beat_pos(&[1.0], 2.0).is_none());
    }
    #[test]
    fn the_sampler_stretcher_keeps_the_pitch_and_runs_its_grains() {
        // A 441 Hz tone read at 0.8 of its speed with the pitch kept: one second of output
        // still crosses zero about 882 times (not 706, as plain slow playback would).
        let sr = 44100usize;
        let x: Vec<f32> = (0..sr * 3).flat_map(|i| { let v = (i as f32 * 441.0 * std::f32::consts::TAU / sr as f32).sin() * 0.5; [v, v] }).collect();
        let audio = super::Decoded { sr: sr as u32, samples: x, raw_frames: sr * 3, damaged: 0, truncated: false };
        let mut r = super::ClipReader::new();
        let (mut pos, mut last, mut crossings) = (1000.0f64, 0.0f32, 0);
        for _ in 0..sr {
            let (l, _) = r.frame(&audio, pos, 0.8, 1.0, 882);
            if (l >= 0.0) != (last >= 0.0) {
                crossings += 1;
            }
            last = l;
            pos += 0.8;
        }
        assert!((860..=905).contains(&crossings), "zero crossings in a second: {crossings}");
    }

}
