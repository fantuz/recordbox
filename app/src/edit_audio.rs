//! EDIT mode playback (spec: checks/re-7.2.19/edit-mode.md section 3). A `Voice` plays a score:
//! the play position runs along the timeline at the score's constant tempo, and for every
//! output frame the position score says which song beat sounds there. Each song beat is read
//! in exactly one timeline beat (60000 / bpm ms), so a song whose own tempo drifts comes out
//! at a constant tempo. Where the song position jumps (a join between sections, a loop wrap,
//! a seek) the old sound is cross-faded into the new one; muted sections fade to silence.
//!
//! The engine is mixed into the audio callback by `mix` (one call in `audio.rs`). The
//! callback never allocates or blocks: scores and audio are `Arc`s the interface puts in and
//! takes out, and the voices' buffers are fixed arrays.

use crate::audio::Decoded;
use crate::edit_model::ScoreSet;
use parking_lot::Mutex;
use std::sync::Arc;

/// Length of the cross-fade at a join, loop wrap or seek, and of the play / pause fade. The
/// same 4 ms recordbox's decks use for a jump (rekordbox's own length was not traced).
pub const XFADE_SECS: f32 = 0.004;
/// A mute edge takes 3.3 ms (rekordbox's gain ramp, as for a muted stem).
pub const MUTE_RAMP_SECS: f32 = 0.0033;

#[inline]
fn cubic(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
    let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c = -0.5 * y0 + 0.5 * y2;
    ((a * t + b) * t + c) * t + y1
}

/// Left and right at source frame `p` (fractional). Silence outside the file.
#[inline]
fn at(a: &Decoded, p: f64) -> (f32, f32) {
    let frames = a.samples.len() / 2;
    if p < -1.0 || p >= frames as f64 || frames < 4 {
        return (0.0, 0.0);
    }
    let p = p.max(0.0);
    let i = p as usize;
    let t = (p - i as f64) as f32;
    let s = &a.samples;
    let ix = |k: isize| (k.clamp(0, frames as isize - 1) as usize) * 2;
    let (i0, i1, i2, i3) = (ix(i as isize - 1), ix(i as isize), ix(i as isize + 1), ix(i as isize + 2));
    (cubic(s[i0], s[i1], s[i2], s[i3], t), cubic(s[i0 + 1], s[i1 + 1], s[i2 + 1], s[i3 + 1], t))
}

#[inline]
fn mono(a: &Decoded, i: isize) -> f32 {
    let frames = (a.samples.len() / 2) as isize;
    if i < 0 || i >= frames {
        return 0.0;
    }
    a.samples[i as usize * 2] + a.samples[i as usize * 2 + 1]
}

const TMPL: usize = 256;

/// Time stretch that holds the pitch (overlap-add with waveform matching): the output is
/// grains of the source read at the file's own rate and cross-faded every `hop` frames; each
/// new grain starts where the play position says, moved by up to 12 ms to where it best
/// continues the grain before it. Used when the song's grid is not steady.
struct Stretch {
    active: bool,
    cur: f64,
    prev: f64,
    same: bool,
    n: usize,
    hop: usize,
    tmpl: [f32; TMPL],
}

impl Stretch {
    const fn new() -> Stretch {
        Stretch { active: false, cur: 0.0, prev: 0.0, same: true, n: 0, hop: 882, tmpl: [0.0; TMPL] }
    }

    /// Where the sound that is fading in is being read right now.
    fn reading(&self, ps: f64) -> f64 {
        self.cur + self.n as f64 * ps
    }

    fn pick(&mut self, a: &Decoded, nominal: f64, natural: f64, ps: f64) -> f64 {
        let dec = (self.hop / TMPL).max(1) * 2;
        let m = (self.hop / dec).clamp(8, TMPL);
        let d = ps * dec as f64;
        let mut energy = 0.0f32;
        for j in 0..m {
            let v = mono(a, (natural + j as f64 * d).round() as isize);
            self.tmpl[j] = v;
            energy += v * v;
        }
        if energy < 1e-9 {
            return nominal;
        }
        let delta = (0.012 * a.sr as f64) as isize;
        let base = nominal.round() as isize;
        let score = |k: isize, tmpl: &[f32]| -> f32 {
            let (mut c, mut e) = (0.0f32, 0.0f32);
            for (j, t) in tmpl.iter().enumerate() {
                let v = mono(a, base + k + (j as f64 * d).round() as isize);
                c += t * v;
                e += v * v;
            }
            c / (e + 1e-9).sqrt()
        };
        let (mut best, mut best_k) = (f32::MIN, 0isize);
        let mut k = -delta;
        while k <= delta {
            let sc = score(k, &self.tmpl[..m]);
            if sc > best {
                best = sc;
                best_k = k;
            }
            k += 3;
        }
        for k in [best_k - 1, best_k + 1] {
            let sc = score(k, &self.tmpl[..m]);
            if sc > best {
                best = sc;
                best_k = k;
            }
        }
        (base + best_k) as f64
    }

    /// One output frame with the play position at source frame `pos`, read at `ps` source
    /// frames per output frame.
    #[inline]
    fn frame(&mut self, a: &Decoded, pos: f64, ps: f64, hop: usize) -> (f32, f32) {
        let h = hop as f64;
        if !self.active || self.hop != hop {
            self.active = true;
            self.hop = hop;
            self.cur = pos;
            self.prev = pos - h * ps;
            self.same = true;
            self.n = 0;
        } else if self.n >= hop {
            let natural = self.cur + h * ps;
            self.prev = self.cur;
            self.cur = if (pos - natural).abs() < 0.5 { natural } else { self.pick(a, pos, natural, ps) };
            self.same = (self.cur - natural).abs() < 1e-6;
            self.n = 0;
        }
        let n = self.n as f64;
        self.n += 1;
        let i = at(a, self.cur + n * ps);
        if self.same {
            return i;
        }
        let o = at(a, self.prev + (h + n) * ps);
        let w = 0.5 - 0.5 * (std::f32::consts::PI * ((n as f32 + 0.5) / h as f32)).cos();
        (o.0 + (i.0 - o.0) * w, o.1 + (i.1 - o.1) * w)
    }
}

/// One player: the main timeline, or a palette clip.
pub struct Voice {
    pub audio: Option<Arc<Decoded>>,
    pub score: Option<Arc<ScoreSet>>,
    /// Play position on the timeline, beats.
    pub pos: f64,
    pub playing: bool,
    /// Loop of the selected range (only while `loop_on`).
    pub loop_on: bool,
    pub loop_range: Option<(f64, f64)>,
    /// Hold the pitch while stretching (set from the song grid: `SongGrid::is_dynamic`).
    pub keep_pitch: bool,
    /// Offline rendering: no fade-in at the start.
    pub instant: bool,
    st: Stretch,
    /// Source frame the next output frame is expected to read (a difference is a jump).
    expect: f64,
    primed: bool,
    /// The sound from before a jump, fading out: (source frame, frames left).
    tail: Option<(f64, u32)>,
    /// Mute gain the fading-out side had.
    tail_g: f32,
    gate: f32,
    mute_g: f32,
    /// Section of the position score the play position was last in.
    sec: usize,
    /// Where the fade-out after pause reads from.
    out_src: f64,
    step: f64,
}

impl Voice {
    pub const fn new() -> Voice {
        Voice { audio: None, score: None, pos: 0.0, playing: false, loop_on: false, loop_range: None, keep_pitch: false, instant: false, st: Stretch::new(), expect: 0.0, primed: false, tail: None, tail_g: 1.0, gate: 0.0, mute_g: 1.0, sec: 0, out_src: 0.0, step: 1.0 }
    }

    /// Unload: nothing sounds any more. Returns what was loaded so that the caller frees it
    /// (never the audio thread).
    pub fn clear(&mut self) -> (Option<Arc<Decoded>>, Option<Arc<ScoreSet>>) {
        self.playing = false;
        self.pos = 0.0;
        self.loop_range = None;
        self.primed = false;
        self.tail = None;
        self.gate = 0.0;
        self.st.active = false;
        (self.audio.take(), self.score.take())
    }

    /// The song beat and mute state at timeline beat `p` (with the last section remembered).
    #[inline]
    fn map(&mut self, s: &ScoreSet, p: f64) -> Option<(f64, bool)> {
        let v = &s.position;
        let ok = |i: usize| v.get(i).map(|x| p >= x.start && p < x.end).unwrap_or(false);
        if !ok(self.sec) {
            self.sec = if ok(self.sec + 1) { self.sec + 1 } else { v.partition_point(|x| x.end <= p) };
        }
        let x = v.get(self.sec)?;
        if p < x.start {
            return None;
        }
        Some((x.song_start + (p - x.start), s.muted_at(p)))
    }

    /// Add this voice's sound to `out` (interleaved, `channels` wide) at `gain` and move the
    /// position on. Returns the peak (left, right) of what it added.
    pub fn render(&mut self, out: &mut [f32], channels: usize, rate: u32, gain: f32) -> [f32; 2] {
        let mut peak = [0.0f32; 2];
        if !self.playing && self.gate <= 0.0 {
            self.primed = false;
            self.tail = None;
            self.st.active = false;
            return peak;
        }
        // (Clones of the Arcs: no allocation, and `self` still holds them, so nothing is freed
        // here.)
        let (Some(audio), Some(score)) = (self.audio.clone(), self.score.clone()) else {
            self.playing = false;
            return peak;
        };
        let (audio, score): (&Decoded, &ScoreSet) = (&audio, &score);
        let grid = &score.song_grid;
        let sr = audio.sr as f64;
        let ps = sr / rate as f64;
        let beat_step = score.bpm / 60.0 / rate as f64;
        let len = score.len();
        let fade_n = ((XFADE_SECS * rate as f32) as u32).max(1);
        let mute_k = 1.0 / (MUTE_RAMP_SECS * rate as f32);
        let hop = ((rate as usize) / 50).clamp(64, 4096);
        let ch = channels.max(1);
        for f in out.chunks_mut(ch) {
            let fading_out = !self.playing;
            if fading_out {
                self.gate -= 1.0 / fade_n as f32;
                if self.gate <= 0.0 {
                    self.gate = 0.0;
                    self.primed = false;
                    self.tail = None;
                    self.st.active = false;
                    break;
                }
            } else if self.gate < 1.0 {
                self.gate = if self.instant { 1.0 } else { (self.gate + 1.0 / fade_n as f32).min(1.0) };
            }
            let (mut l, mut r);
            if fading_out {
                // The sound carries on from where it was while it fades.
                (l, r) = at(audio, self.out_src);
                self.out_src += self.step;
            } else {
                if self.loop_on {
                    if let Some((a, b)) = self.loop_range.filter(|(a, b)| b > a) {
                        if self.pos >= b {
                            self.pos = a + (self.pos - b).min(b - a - beat_step).max(0.0);
                        }
                    }
                }
                if self.pos >= len {
                    self.pos = len;
                    self.playing = false;
                    continue;
                }
                let p = self.pos.max(0.0);
                let Some((sb, muted)) = self.map(score, p) else {
                    self.playing = false;
                    continue;
                };
                let ms = grid.ppq_to_ms(sb);
                let src = ms * 0.001 * sr;
                let step = (grid.ppq_to_ms(sb + beat_step) - ms) * 0.001 * sr;
                let target = if muted { 0.0 } else { 1.0 };
                if self.primed && (src - self.expect).abs() > 2.0 * step.max(1.0) {
                    // A join, a loop wrap or a seek: what was sounding fades out from where
                    // it was being read, the new place fades in (and starts at its own mute
                    // state: the cross-fade is the ramp).
                    let from = if self.keep_pitch && self.st.active { self.st.reading(ps) } else { self.expect };
                    self.tail = Some((from, fade_n));
                    self.tail_g = self.mute_g;
                    self.mute_g = target;
                    self.st.active = false;
                }
                if !self.primed {
                    self.mute_g = target;
                }
                self.primed = true;
                (l, r) = if self.keep_pitch { self.st.frame(audio, src, ps, hop) } else { at(audio, src) };
                self.mute_g += (target - self.mute_g).clamp(-mute_k, mute_k);
                l *= self.mute_g;
                r *= self.mute_g;
                if let Some((op, left)) = self.tail {
                    // Equal power: the two sides of a join are different audio.
                    let x = 1.0 - left as f32 / fade_n as f32;
                    let (wi, wo) = ((x * std::f32::consts::FRAC_PI_2).sin(), (x * std::f32::consts::FRAC_PI_2).cos() * self.tail_g);
                    let (ol, or) = at(audio, op);
                    l = l * wi + ol * wo;
                    r = r * wi + or * wo;
                    let tail_step = if self.keep_pitch { ps } else { self.step };
                    self.tail = if left > 1 { Some((op + tail_step, left - 1)) } else { None };
                }
                self.step = if self.keep_pitch { ps } else { step };
                self.expect = src + step;
                self.out_src = if self.keep_pitch { self.st.reading(ps) } else { src + step };
                self.pos += beat_step;
            }
            let g = gain * (0.5 - 0.5 * (std::f32::consts::PI * self.gate).cos());
            l *= g;
            r *= g;
            peak[0] = peak[0].max(l.abs());
            peak[1] = peak[1].max(r.abs());
            if ch >= 2 {
                f[0] += l;
                f[1] += r;
            } else {
                f[0] += 0.5 * (l + r);
            }
        }
        peak
    }
}

/// Palette clips that can sound at the same time (rekordbox gives every palette clip a
/// player of its own, `EditPlayerSlotManager`; here eight are mixed at once and a ninth start
/// takes the place of the one started longest ago).
pub const PAL_VOICES: usize = 8;

/// What the audio callback plays in EDIT mode: the timeline and the palette clips.
pub struct Engine {
    pub main: Voice,
    pub pals: [Voice; PAL_VOICES],
    /// Which palette entry each palette player holds (0 = none) and when it was started.
    pub pal_ids: [u32; PAL_VOICES],
    pub pal_started: [u64; PAL_VOICES],
    pub pal_seq: u64,
    /// Master volume and the track's auto gain (as a deck applies them); mute.
    pub volume: f32,
    pub gain: f32,
    pub muted: bool,
    /// Highest output level since the meter last read it.
    pub peak: [f32; 2],
}

impl Engine {
    pub const fn new() -> Engine {
        Engine { main: Voice::new(), pals: [Voice::new(), Voice::new(), Voice::new(), Voice::new(), Voice::new(), Voice::new(), Voice::new(), Voice::new()], pal_ids: [0; PAL_VOICES], pal_started: [0; PAL_VOICES], pal_seq: 0, volume: 0.9, gain: 1.0, muted: false, peak: [0.0; 2] }
    }

    /// The player holding a palette entry.
    pub fn pal_of(&self, id: u32) -> Option<usize> {
        (0..PAL_VOICES).find(|k| self.pal_ids[*k] == id && id != 0)
    }

    /// A palette entry is playing.
    pub fn pal_playing(&self, id: u32) -> bool {
        self.pal_of(id).map(|k| self.pals[k].playing).unwrap_or(false)
    }

    pub fn any_pal_playing(&self) -> bool {
        self.pals.iter().any(|v| v.playing)
    }

    /// A player for a palette entry: its own, else a silent one, else the one started first.
    pub fn pal_slot(&mut self, id: u32) -> usize {
        let k = self.pal_of(id).or_else(|| (0..PAL_VOICES).find(|k| !self.pals[*k].playing && self.pals[*k].gate <= 0.0)).unwrap_or_else(|| (0..PAL_VOICES).min_by_key(|k| self.pal_started[*k]).unwrap_or(0));
        self.pal_ids[k] = id;
        self.pal_seq += 1;
        self.pal_started[k] = self.pal_seq;
        k
    }
}

pub static EDIT: Mutex<Engine> = Mutex::new(Engine::new());

/// The audio callback's part: add the editor's sound to the mix. The interface holds the
/// engine only for moments (to set a control or read the position), so a held lock is waited
/// out briefly rather than giving up a whole buffer.
pub fn mix(out: &mut [f32], channels: usize, rate: u32) {
    let mut g = match EDIT.try_lock() {
        Some(g) => g,
        None => {
            let t = std::time::Instant::now();
            loop {
                for _ in 0..64 {
                    std::hint::spin_loop();
                }
                if let Some(g) = EDIT.try_lock() {
                    break g;
                }
                if t.elapsed() > std::time::Duration::from_micros(250) {
                    return;
                }
            }
        }
    };
    let e = &mut *g;
    if !e.main.playing && e.main.gate <= 0.0 && e.pals.iter().all(|v| !v.playing && v.gate <= 0.0) {
        return;
    }
    let gain = if e.muted { 0.0 } else { e.volume * e.gain };
    let a = e.main.render(out, channels, rate, gain);
    let mut pk = a;
    for v in e.pals.iter_mut() {
        if v.playing || v.gate > 0.0 {
            let b = v.render(out, channels, rate, gain);
            pk = [pk[0].max(b[0]), pk[1].max(b[1])];
        }
    }
    e.peak = [e.peak[0].max(pk[0]), e.peak[1].max(pk[1])];
}

#[cfg(test)]
pub mod tests {
    #[test]
    fn palette_clips_have_players_of_their_own() {
        let mut e = Engine::new();
        let tones = beat_tones(8, &[22050]);
        let sc = Arc::new(score(8, &[22050], 120.0));
        let mut start = |e: &mut Engine, id: u32| {
            let k = e.pal_slot(id);
            e.pals[k].audio = Some(tones.clone());
            e.pals[k].score = Some(sc.clone());
            e.pals[k].playing = true;
            k
        };
        let a = start(&mut e, 1);
        let b = start(&mut e, 2);
        assert_ne!(a, b, "two clips, two players");
        assert!(e.pal_playing(1) && e.pal_playing(2));
        // Both sound: rendering the two players adds up to twice one of them.
        let mut one = vec![0.0f32; 1024];
        let mut both = vec![0.0f32; 1024];
        {
            let mut solo = Voice::new();
            solo.audio = Some(tones.clone());
            solo.score = Some(sc.clone());
            solo.playing = true;
            solo.render(&mut one, 2, SR, 1.0);
        }
        for k in [a, b] {
            e.pals[k].render(&mut both, 2, SR, 1.0);
        }
        let (p1, p2) = (one.iter().fold(0.0f32, |m, v| m.max(v.abs())), both.iter().fold(0.0f32, |m, v| m.max(v.abs())));
        assert!((p2 - 2.0 * p1).abs() < 1e-3 * p1.max(1e-6) + 1e-4, "{p1} {p2}");
        // Starting the same clip again reuses its player; a ninth clip takes the oldest one's.
        assert_eq!(start(&mut e, 1), a);
        for id in 3..=8 {
            start(&mut e, id);
        }
        assert!((1..=8).all(|id| e.pal_playing(id)));
        let k9 = start(&mut e, 9);
        assert_eq!(k9, b, "the player started longest ago (clip 2) is taken");
        assert!(!e.pal_playing(2) && e.pal_playing(9));
    }

    use super::*;
    use crate::edit_model::*;

    pub const SR: u32 = 44100;

    /// A sine whose frequency says which song beat is sounding: beat k is 200 + 20 k Hz
    /// (continuous phase inside a beat, 120 BPM, `beats` beats long).
    pub fn beat_tones(beats: usize, beat_frames: &[usize]) -> Arc<Decoded> {
        let mut s = Vec::new();
        for k in 0..beats {
            let n = beat_frames[k.min(beat_frames.len() - 1)];
            let f = 200.0 + 20.0 * k as f64;
            for i in 0..n {
                let v = (0.5 * (2.0 * std::f64::consts::PI * f * i as f64 / SR as f64).sin()) as f32;
                s.push(v);
                s.push(v);
            }
        }
        let frames = s.len() / 2;
        Arc::new(Decoded { sr: SR, samples: s, raw_frames: frames, damaged: 0, truncated: false })
    }

    pub fn score(beats: usize, beat_frames: &[usize], bpm: f64) -> ScoreSet {
        let mut t = 0.0;
        let mut org = vec![];
        for k in 0..=beats {
            org.push((bpm, t));
            t += beat_frames[k.min(beat_frames.len() - 1)] as f64 * 1000.0 / SR as f64;
        }
        let last = org.last().unwrap().1;
        ScoreSet::new(1, bpm, SongGrid::new(org, 0, last, bpm), Prepared::default())
    }

    fn voice(a: Arc<Decoded>, s: ScoreSet) -> Voice {
        let mut v = Voice::new();
        v.keep_pitch = s.song_grid.is_dynamic();
        v.audio = Some(a);
        v.score = Some(Arc::new(s));
        v.playing = true;
        v.instant = true;
        v
    }

    fn run(v: &mut Voice, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        for c in out.chunks_mut(512) {
            v.render(c, 2, SR, 1.0);
        }
        out.chunks(2).map(|c| c[0]).collect()
    }

    /// Dominant frequency of a stretch of signal, from its zero crossings.
    fn freq(x: &[f32]) -> f64 {
        let mut n = 0;
        let (mut first, mut last) = (None, 0usize);
        for i in 1..x.len() {
            if x[i - 1] <= 0.0 && x[i] > 0.0 {
                n += 1;
                first.get_or_insert(i);
                last = i;
            }
        }
        match first {
            Some(f) if n > 1 => (n - 1) as f64 * SR as f64 / (last - f) as f64,
            _ => 0.0,
        }
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    const BEAT: usize = 22050;

    #[test]
    fn an_unedited_steady_track_plays_as_the_file() {
        let a = beat_tones(8, &[BEAT]);
        let mut v = voice(a.clone(), score(8, &[BEAT], 120.0));
        let out = run(&mut v, 4 * BEAT);
        let worst = out.iter().enumerate().map(|(i, s)| (s - a.samples[i * 2]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "difference from the file {worst}");
        assert!((v.pos - 4.0).abs() < 1e-6, "position {}", v.pos);
        // It stops at the end of the score.
        run(&mut v, 5 * BEAT);
        assert!(!v.playing);
        assert_eq!(v.pos, 8.0);
    }

    #[test]
    fn sections_play_the_song_beats_they_point_at() {
        // Delete beats 2..4: timeline beat 2 plays song beat 4 (280 Hz), on the sample.
        let a = beat_tones(8, &[BEAT]);
        let mut s = score(8, &[BEAT], 120.0);
        assert!(s.delete_range(2.0, 4.0));
        let mut v = voice(a, s);
        let out = run(&mut v, 6 * BEAT);
        let f = |beat: usize| freq(&out[beat * BEAT + 2000..(beat + 1) * BEAT - 2000]);
        assert!((f(0) - 200.0).abs() < 1.0 && (f(1) - 220.0).abs() < 1.0, "{} {}", f(0), f(1));
        assert!((f(2) - 280.0).abs() < 1.0 && (f(3) - 300.0).abs() < 1.0, "{} {}", f(2), f(3));
        // The join is on the beat: 10 ms before it is the old tone, 10 ms after the new one.
        let j = 2 * BEAT;
        assert!((freq(&out[j - 2205..j - 220]) - 220.0).abs() < 6.0);
        assert!((freq(&out[j + 220..j + 2205]) - 280.0).abs() < 6.0);
        // Cross-faded over 4 ms: no step larger than the signal itself makes.
        let step = out[j - 400..j + 400].windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(step < 0.06, "step at the join {step}");
        run(&mut v, 64);
        assert!(!v.playing && v.pos == 6.0);
    }

    #[test]
    fn muted_sections_are_silent_with_short_ramps() {
        let a = beat_tones(8, &[BEAT]);
        let mut s = score(8, &[BEAT], 120.0);
        assert!(s.mute_range(1.0, 2.0, true));
        let mut v = voice(a, s);
        let out = run(&mut v, 3 * BEAT);
        assert!(rms(&out[2000..BEAT - 2000]) > 0.3);
        // Silent from 3.3 ms after the edge until the section ends.
        let ramp = (MUTE_RAMP_SECS * SR as f32) as usize + 2;
        assert!(rms(&out[BEAT + ramp..2 * BEAT]) < 1e-6);
        assert!(rms(&out[BEAT..BEAT + ramp / 2]) > 0.01, "the edge is a ramp, not a cut");
        assert!(rms(&out[2 * BEAT + ramp..3 * BEAT]) > 0.3);
    }

    #[test]
    fn a_drifting_song_comes_out_at_a_constant_tempo_and_its_own_pitch() {
        // Song beats alternate between 0.5 s and 0.4 s; the score runs at 120 BPM, so every
        // song beat must fill exactly 0.5 s of output at its own pitch.
        let lens = [BEAT, 17640, BEAT, 17640, BEAT, 17640, BEAT, 17640];
        let a = beat_tones(8, &lens);
        let s = score(8, &lens, 120.0);
        assert!(s.song_grid.is_dynamic());
        let mut v = voice(a, s);
        let out = run(&mut v, 8 * BEAT);
        for beat in 0..8 {
            let f = freq(&out[beat * BEAT + 3000..(beat + 1) * BEAT - 3000]);
            let want = 200.0 + 20.0 * beat as f64;
            assert!((f - want).abs() < 2.5, "beat {beat}: {f} Hz, want {want}");
            assert!(rms(&out[beat * BEAT + 3000..(beat + 1) * BEAT - 3000]) > 0.25);
        }
        run(&mut v, 64);
        assert!(!v.playing && (v.pos - 8.0).abs() < 1e-9);
    }

    #[test]
    fn loop_wraps_on_the_beat_and_pause_fades() {
        let a = beat_tones(8, &[BEAT]);
        let mut v = voice(a, score(8, &[BEAT], 120.0));
        v.loop_on = true;
        v.loop_range = Some((2.0, 4.0));
        v.pos = 2.0;
        let out = run(&mut v, 5 * BEAT);
        // Beats 2, 3, 2, 3, 2 of the song.
        for (i, want) in [240.0, 260.0, 240.0, 260.0, 240.0].iter().enumerate() {
            let f = freq(&out[i * BEAT + 2000..(i + 1) * BEAT - 2000]);
            assert!((f - want).abs() < 1.0, "{i}: {f}");
        }
        assert!(v.playing && v.pos >= 2.0 && v.pos < 4.0);
        // With the loop off the same range plays through.
        v.loop_on = false;
        run(&mut v, 2 * BEAT);
        assert!(v.pos > 4.0);
        // Pause: the sound fades over 4 ms instead of stopping dead, the position stays.
        v.instant = false;
        let at_pause = v.pos;
        v.playing = false;
        let tail = run(&mut v, 1024);
        assert_eq!(v.pos, at_pause);
        assert!(rms(&tail[..60]) > 0.05 && rms(&tail[400..]) < 1e-6);
        // Play again: fades in.
        v.playing = true;
        let head = run(&mut v, 1024);
        assert!(head[0].abs() < 0.01 && rms(&head[400..]) > 0.25);
    }
}
