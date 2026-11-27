//! EDIT mode's data model, after rekordbox 7.2.19's `editfrw` (spec: checks/re-7.2.19/edit-mode.md,
//! section 2). A loaded track is a `ScoreSet`: parallel lists of sections over a timeline measured
//! in beats (1.0 = one beat; every edit boundary is a whole beat). The position score says which
//! song beats each stretch of the timeline plays, the mute score which stretches are silent, the
//! BPM score holds the one constant tempo, and the prepared data (hot cues, memory cues, Active
//! Censor) travels with the material it sits on. Edits rewrite the lists and never touch audio.
//! `EditSession` is the editing session on top: selection, clipboard, undo / redo, palette.

use std::sync::Arc;

/// rekordbox's tolerance for "is a whole beat" (DBL_EPSILON).
pub const EPS: f64 = f64::EPSILON;
/// A cue's `out` when it is not a loop (rekordbox writes -DBL_MAX).
pub const NOT_LOOP: f64 = -f64::MAX;
pub const PALETTE_MAX: usize = 100;
/// Zoom steps of the enlarged waveform, index 1..=15 (index 0 unused), and the reset step.
pub const ZOOM_TABLE: [u32; 16] = [0, 1, 2, 3, 4, 8, 16, 24, 32, 40, 48, 56, 112, 224, 448, 896];
pub const ZOOM_DEFAULT: usize = 5;
pub const BEAT_SELECT: [u32; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
pub const BPM_MIN: f64 = 40.0;
pub const BPM_MAX: f64 = 499.0;
/// Longest timeline that can be rendered (180 minutes).
pub const RENDER_MAX_MS: f64 = 10_800_000.0;

fn whole(x: f64) -> bool {
    x.is_finite() && (x.floor() - x).abs() <= EPS
}

#[derive(Clone, Debug, PartialEq)]
pub struct PosSec {
    pub start: f64,
    pub end: f64,
    pub song_start: f64,
    pub song_end: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MuteSec {
    pub start: f64,
    pub end: f64,
    pub muted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BpmSec {
    pub start: f64,
    pub end: f64,
    pub bpm: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CueAppended {
    pub db_cue_id: i64,
    pub status: i32,
    pub color: u8,
    pub comment: String,
    pub loop_numer: i32,
    pub loop_denom: i32,
    pub color_table: u8,
}

/// A hot cue or memory cue on the timeline (beats). `pos_out` is `NOT_LOOP` for a plain cue.
#[derive(Clone, Debug, PartialEq)]
pub struct CueLoop {
    pub cue_id: i32,
    pub pos_in: f64,
    pub pos_out: f64,
    pub appended: CueAppended,
}

impl CueLoop {
    pub fn is_loop(&self) -> bool {
        self.pos_out > self.pos_in
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CensorAppended {
    pub ac_id: i32,
    pub effect: u8,
    pub on: bool,
    pub params: Vec<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Censor {
    pub pos_in: f64,
    pub pos_out: f64,
    pub appended: CensorAppended,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Prepared {
    pub hot: Vec<CueLoop>,
    pub memory: Vec<CueLoop>,
    pub censor: Vec<Censor>,
}

// ---------------------------------------------------------------------------- song grid

/// The track's own beat grid stretched to cover the whole file (rekordbox's
/// `ImaginarySongGrid`): the analysed beats, with beats at the nominal tempo added before the
/// first and after the last so that every moment of the file lies on a beat interval.
#[derive(Clone, Debug, PartialEq)]
pub struct SongGrid {
    pub length_ms: f64,
    pub nominal_bpm: f64,
    /// The analysed beats: (bpm at the beat, position in ms).
    pub org_beats: Vec<(f64, f64)>,
    /// Which of the analysed beats are bar lines: index k with k = offset (mod 4).
    pub org_bar_offset: i32,
    /// Beats added before the first and after the last analysed beat.
    pub pre: usize,
    pub post: usize,
    /// Every beat of the extended grid, ms.
    ms: Vec<f64>,
}

impl SongGrid {
    pub fn new(org_beats: Vec<(f64, f64)>, org_bar_offset: i32, length_ms: f64, nominal_bpm: f64) -> SongGrid {
        let beat_ms = 60000.0 / nominal_bpm.max(1e-6);
        let (first, last) = match (org_beats.first(), org_beats.last()) {
            (Some(f), Some(l)) => (f.1, l.1),
            _ => (0.0, 0.0),
        };
        let pre = if org_beats.is_empty() { 0 } else { (first / beat_ms).ceil().max(0.0) as usize };
        let post = ((length_ms - last) / beat_ms).ceil().max(0.0) as usize;
        let mut ms = Vec::with_capacity(pre + org_beats.len() + post);
        for i in (1..=pre).rev() {
            ms.push(first - i as f64 * beat_ms);
        }
        ms.extend(org_beats.iter().map(|b| b.1));
        if org_beats.is_empty() {
            ms.push(0.0);
        }
        for i in 1..=post {
            ms.push(last + i as f64 * beat_ms);
        }
        SongGrid { length_ms, nominal_bpm, org_beats, org_bar_offset, pre, post, ms }
    }

    /// Number of beats of the extended grid.
    pub fn beats(&self) -> usize {
        self.ms.len()
    }

    /// Bar-line offset of the extended grid (shifted by the beats added in front).
    pub fn bar_offset(&self) -> i32 {
        (self.org_bar_offset + self.pre as i32).rem_euclid(4)
    }

    fn beat_ms(&self) -> f64 {
        60000.0 / self.nominal_bpm.max(1e-6)
    }

    /// Song position in ms of song beat `b` (linear inside each beat; the nominal tempo
    /// continues past both ends).
    pub fn ppq_to_ms(&self, b: f64) -> f64 {
        let n = self.ms.len();
        if n == 0 {
            return b * self.beat_ms();
        }
        if b <= 0.0 {
            return self.ms[0] + b * self.beat_ms();
        }
        let last = (n - 1) as f64;
        if b >= last {
            return self.ms[n - 1] + (b - last) * self.beat_ms();
        }
        let i = b.floor() as usize;
        self.ms[i] + (b - i as f64) * (self.ms[i + 1] - self.ms[i])
    }

    pub fn ms_to_ppq(&self, ms: f64) -> f64 {
        let n = self.ms.len();
        if n == 0 {
            return ms / self.beat_ms();
        }
        if ms <= self.ms[0] {
            return (ms - self.ms[0]) / self.beat_ms();
        }
        if ms >= self.ms[n - 1] {
            return (n - 1) as f64 + (ms - self.ms[n - 1]) / self.beat_ms();
        }
        let i = self.ms.partition_point(|t| *t <= ms) - 1;
        let d = self.ms[i + 1] - self.ms[i];
        i as f64 + if d > 0.0 { (ms - self.ms[i]) / d } else { 0.0 }
    }

    /// The song's tempo moves: two neighbouring beat intervals differ by 2.1 ms or more. Then
    /// playback holds the pitch while it stretches each beat (rekordbox's `shouldMasterTempo`).
    pub fn is_dynamic(&self) -> bool {
        self.ms.windows(3).any(|w| ((w[2] - w[1]) - (w[1] - w[0])).abs() >= 2.1)
    }
}

// ---------------------------------------------------------------------------- sections

trait Sec: Clone {
    fn range(&self) -> (f64, f64);
    /// The part of this section between `a` and `b` (both inside it).
    fn part(&self, a: f64, b: f64) -> Self;
    fn shift(&mut self, d: f64);
    /// This section and the one after it as one, when nothing changes between them.
    fn joined(&self, next: &Self) -> Option<Self>;
}

impl Sec for PosSec {
    fn range(&self) -> (f64, f64) {
        (self.start, self.end)
    }
    fn part(&self, a: f64, b: f64) -> Self {
        PosSec { start: a, end: b, song_start: self.song_start + (a - self.start), song_end: self.song_start + (b - self.start) }
    }
    fn shift(&mut self, d: f64) {
        self.start += d;
        self.end += d;
    }
    fn joined(&self, n: &Self) -> Option<Self> {
        ((self.song_end - n.song_start).abs() <= EPS).then(|| PosSec { start: self.start, end: n.end, song_start: self.song_start, song_end: n.song_end })
    }
}

impl Sec for MuteSec {
    fn range(&self) -> (f64, f64) {
        (self.start, self.end)
    }
    fn part(&self, a: f64, b: f64) -> Self {
        MuteSec { start: a, end: b, muted: self.muted }
    }
    fn shift(&mut self, d: f64) {
        self.start += d;
        self.end += d;
    }
    fn joined(&self, n: &Self) -> Option<Self> {
        (self.muted == n.muted).then(|| MuteSec { start: self.start, end: n.end, muted: self.muted })
    }
}

impl Sec for BpmSec {
    fn range(&self) -> (f64, f64) {
        (self.start, self.end)
    }
    fn part(&self, a: f64, b: f64) -> Self {
        BpmSec { start: a, end: b, bpm: self.bpm }
    }
    fn shift(&mut self, d: f64) {
        self.start += d;
        self.end += d;
    }
    fn joined(&self, n: &Self) -> Option<Self> {
        (self.bpm == n.bpm).then(|| BpmSec { start: self.start, end: n.end, bpm: self.bpm })
    }
}

/// The parts of `v` inside [a, b), keeping their positions.
fn parts<T: Sec>(v: &[T], a: f64, b: f64) -> Vec<T> {
    let mut out = Vec::new();
    for s in v {
        let (s0, s1) = s.range();
        let (x, y) = (s0.max(a), s1.min(b));
        if y > x {
            out.push(s.part(x, y));
        }
    }
    out
}

fn merged<T: Sec>(v: Vec<T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::with_capacity(v.len());
    for s in v {
        let (a, b) = s.range();
        if b <= a {
            continue;
        }
        if let Some(j) = out.last().and_then(|l| l.joined(&s)) {
            *out.last_mut().unwrap() = j;
        } else {
            out.push(s);
        }
    }
    out
}

fn clip_secs<T: Sec>(v: &[T], a: f64, b: f64) -> Vec<T> {
    let mut out = parts(v, a, b);
    out.iter_mut().for_each(|s| s.shift(-a));
    merged(out)
}

fn delete_secs<T: Sec>(v: &[T], a: f64, b: f64) -> Vec<T> {
    let mut out = parts(v, f64::MIN, a);
    let mut tail = parts(v, b, f64::MAX);
    tail.iter_mut().for_each(|s| s.shift(-(b - a)));
    out.extend(tail);
    merged(out)
}

fn insert_secs<T: Sec>(v: &[T], pos: f64, clip: &[T], len: f64) -> Vec<T> {
    let mut out = parts(v, f64::MIN, pos);
    out.extend(clip.iter().cloned().map(|mut s| {
        s.shift(pos);
        s
    }));
    let mut tail = parts(v, pos, f64::MAX);
    tail.iter_mut().for_each(|s| s.shift(len));
    out.extend(tail);
    merged(out)
}

/// Sorted, contiguous and not empty: the range the sections cover.
fn span<T: Sec>(v: &[T]) -> Option<(f64, f64)> {
    let first = v.first()?.range();
    let mut at = first.0;
    for s in v {
        let (a, b) = s.range();
        if (a - at).abs() > EPS || b <= a {
            return None;
        }
        at = b;
    }
    Some((first.0, at))
}

// ---------------------------------------------------------------------------- cues

fn clip_cues(v: &[CueLoop], a: f64, b: f64) -> Vec<CueLoop> {
    let mut out = Vec::new();
    for c in v {
        if c.is_loop() {
            let (x, y) = (c.pos_in.max(a), c.pos_out.min(b));
            if y > x {
                out.push(CueLoop { pos_in: x - a, pos_out: y - a, ..c.clone() });
            }
        } else if c.pos_in >= a && c.pos_in < b {
            out.push(CueLoop { pos_in: c.pos_in - a, ..c.clone() });
        }
    }
    out
}

fn delete_cues(v: &[CueLoop], a: f64, b: f64) -> Vec<CueLoop> {
    let len = b - a;
    let mut out = Vec::new();
    for c in v {
        if c.pos_in >= b {
            let mut c = c.clone();
            c.pos_in -= len;
            if c.is_loop() || c.pos_out > NOT_LOOP {
                c.pos_out -= len;
            }
            out.push(c);
        } else if c.pos_in >= a {
            // Starts inside the deleted range: gone with it.
        } else if c.is_loop() && c.pos_out > a {
            // A loop that reaches into (or over) the range gets shorter by what was removed.
            let end = if c.pos_out <= b { a } else { c.pos_out - len };
            if end > c.pos_in {
                out.push(CueLoop { pos_out: end, ..c.clone() });
            }
        } else {
            out.push(c.clone());
        }
    }
    out
}

fn insert_cues(v: &[CueLoop], pos: f64, clip: &[CueLoop], len: f64) -> Vec<CueLoop> {
    let mut out = Vec::new();
    for c in v {
        let mut c = c.clone();
        if c.pos_in >= pos {
            c.pos_in += len;
            if c.pos_out > NOT_LOOP {
                c.pos_out += len;
            }
        } else if c.is_loop() && c.pos_out > pos {
            // The insert lands inside the loop: the loop ends where the new material starts.
            c.pos_out = pos;
        }
        out.push(c);
    }
    out.extend(clip.iter().map(|c| CueLoop { pos_in: c.pos_in + pos, pos_out: if c.pos_out > NOT_LOOP { c.pos_out + pos } else { c.pos_out }, ..c.clone() }));
    out.sort_by(|x, y| x.pos_in.partial_cmp(&y.pos_in).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn as_cues(v: &[Censor]) -> Vec<CueLoop> {
    v.iter().enumerate().map(|(i, c)| CueLoop { cue_id: i as i32, pos_in: c.pos_in, pos_out: c.pos_out, appended: CueAppended::default() }).collect()
}

/// Active Censor regions go through the same rules as loops; their own data rides along by
/// index.
fn map_censor(src: &[Censor], clip: &[Censor], f: impl Fn(&[CueLoop], &[CueLoop]) -> Vec<CueLoop>) -> Vec<Censor> {
    let n = src.len() as i32;
    let b: Vec<CueLoop> = as_cues(clip).into_iter().map(|mut c| {
        c.cue_id += n;
        c
    }).collect();
    f(&as_cues(src), &b)
        .into_iter()
        .filter(|c| c.pos_out > c.pos_in)
        .map(|c| {
            let from = if c.cue_id < n { &src[c.cue_id as usize] } else { &clip[(c.cue_id - n) as usize] };
            Censor { pos_in: c.pos_in, pos_out: c.pos_out, appended: from.appended.clone() }
        })
        .collect()
}

impl Prepared {
    fn clip(&self, a: f64, b: f64) -> Prepared {
        Prepared { hot: clip_cues(&self.hot, a, b), memory: clip_cues(&self.memory, a, b), censor: map_censor(&self.censor, &[], |v, _| clip_cues(v, a, b)) }
    }
    fn delete(&self, a: f64, b: f64) -> Prepared {
        Prepared { hot: delete_cues(&self.hot, a, b), memory: delete_cues(&self.memory, a, b), censor: map_censor(&self.censor, &[], |v, _| delete_cues(v, a, b)) }
    }
    fn insert(&self, pos: f64, c: &Prepared, len: f64) -> Prepared {
        Prepared { hot: insert_cues(&self.hot, pos, &c.hot, len), memory: insert_cues(&self.memory, pos, &c.memory, len), censor: map_censor(&self.censor, &c.censor, |v, cl| insert_cues(v, pos, cl, len)) }
    }
}

// ---------------------------------------------------------------------------- score set

#[derive(Clone, Debug, PartialEq)]
pub struct ScoreSet {
    /// The collection track the audio comes from.
    pub track_id: i64,
    /// The timeline's one tempo.
    pub bpm: f64,
    /// Timeline beats k with k = bar_offset (mod 4) are bar lines.
    pub bar_offset: i32,
    pub position: Vec<PosSec>,
    pub mute: Vec<MuteSec>,
    pub tempo: Vec<BpmSec>,
    pub prepared: Prepared,
    pub song_grid: Arc<SongGrid>,
}

impl ScoreSet {
    /// A freshly loaded track: the whole song once, nothing muted. `prepared` is in song
    /// beats, which are timeline beats at this point.
    pub fn new(track_id: i64, bpm: f64, song_grid: SongGrid, prepared: Prepared) -> ScoreSet {
        let l = song_grid.beats().saturating_sub(1) as f64;
        let bar_offset = song_grid.bar_offset();
        ScoreSet {
            track_id,
            bpm,
            bar_offset,
            position: vec![PosSec { start: 0.0, end: l, song_start: 0.0, song_end: l }],
            mute: vec![MuteSec { start: 0.0, end: l, muted: false }],
            tempo: vec![BpmSec { start: 0.0, end: l, bpm }],
            prepared,
            song_grid: Arc::new(song_grid),
        }
    }

    /// Timeline length in beats.
    pub fn len(&self) -> f64 {
        self.position.last().map(|s| s.end).unwrap_or(0.0)
    }

    pub fn beat_ms(&self) -> f64 {
        60000.0 / self.bpm.max(1e-6)
    }

    /// Timeline length in ms.
    pub fn len_ms(&self) -> f64 {
        self.len() * self.beat_ms()
    }

    /// rekordbox's `ScoreSet::isValid`.
    pub fn is_valid(&self) -> bool {
        if self.track_id == 0 || self.song_grid.beats() == 0 || !(self.bpm > 0.0) {
            return false;
        }
        let (Some(p), Some(m), Some(t)) = (span(&self.position), span(&self.mute), span(&self.tempo)) else { return false };
        let same = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() <= EPS && (a.1 - b.1).abs() <= EPS;
        if !same(p, m) || !same(p, t) || p.0.abs() > EPS {
            return false;
        }
        if self.position.iter().any(|s| ((s.end - s.start) - (s.song_end - s.song_start)).abs() > 1e-9) {
            return false;
        }
        if self.mute.windows(2).any(|w| w[0].muted == w[1].muted) {
            return false;
        }
        self.tempo.iter().all(|s| s.bpm == self.bpm)
    }

    fn range_ok(&self, a: f64, b: f64) -> bool {
        whole(a) && whole(b) && a >= 0.0 && b > a && b <= self.len() + EPS
    }

    /// A new set holding only [a, b), moved to start at 0.
    pub fn clip(&self, a: f64, b: f64) -> Option<ScoreSet> {
        if !self.range_ok(a, b) {
            return None;
        }
        let c = ScoreSet {
            track_id: self.track_id,
            bpm: self.bpm,
            bar_offset: (self.bar_offset as f64 - a).rem_euclid(4.0) as i32,
            position: clip_secs(&self.position, a, b),
            mute: clip_secs(&self.mute, a, b),
            tempo: clip_secs(&self.tempo, a, b),
            prepared: self.prepared.clip(a, b),
            song_grid: self.song_grid.clone(),
        };
        c.is_valid().then_some(c)
    }

    fn insert_raw(&mut self, pos: f64, c: &ScoreSet) {
        let len = c.len();
        self.position = insert_secs(&self.position, pos, &c.position, len);
        self.mute = insert_secs(&self.mute, pos, &c.mute, len);
        self.tempo = insert_secs(&self.tempo, pos, &c.tempo, len);
        self.prepared = self.prepared.insert(pos, &c.prepared, len);
    }

    fn delete_raw(&mut self, a: f64, b: f64) {
        self.position = delete_secs(&self.position, a, b);
        self.mute = delete_secs(&self.mute, a, b);
        self.tempo = delete_secs(&self.tempo, a, b);
        self.prepared = self.prepared.delete(a, b);
    }

    /// Run `f` on a copy and keep the result only when it is still a valid set.
    fn attempt(&mut self, f: impl FnOnce(&mut ScoreSet) -> bool) -> bool {
        let mut next = self.clone();
        if f(&mut next) && next.is_valid() {
            *self = next;
            true
        } else {
            false
        }
    }

    fn insertable(&self, pos: f64, c: &ScoreSet) -> bool {
        whole(pos) && pos >= 0.0 && pos <= self.len() + EPS && c.is_valid() && c.bpm == self.bpm && c.track_id == self.track_id
    }

    /// Put `c` in at `pos`; everything from `pos` on moves later by its length.
    pub fn insert(&mut self, pos: f64, c: &ScoreSet) -> bool {
        if !self.insertable(pos, c) {
            return false;
        }
        self.attempt(|s| {
            s.insert_raw(pos, c);
            true
        })
    }

    /// Remove [a, b); what follows moves earlier.
    pub fn delete_range(&mut self, a: f64, b: f64) -> bool {
        if !self.range_ok(a, b) {
            return false;
        }
        self.attempt(|s| {
            s.delete_raw(a, b);
            true
        })
    }

    /// Overwrite from `pos`: as much as the clip is long is replaced (to the end at most).
    pub fn paste_at(&mut self, pos: f64, c: &ScoreSet) -> bool {
        if !self.insertable(pos, c) {
            return false;
        }
        let end = (pos + c.len()).min(self.len());
        self.attempt(|s| {
            if end > pos {
                s.delete_raw(pos, end);
            }
            s.insert_raw(pos, c);
            true
        })
    }

    /// Replace the range [a, b) with the clip (the length may change).
    pub fn paste_over(&mut self, a: f64, b: f64, c: &ScoreSet) -> bool {
        if !self.range_ok(a, b) || !c.is_valid() || c.bpm != self.bpm || c.track_id != self.track_id {
            return false;
        }
        self.attempt(|s| {
            s.delete_raw(a, b);
            s.insert_raw(a, c);
            true
        })
    }

    /// A copy of [a, b) inserted right after it.
    pub fn duplicate(&mut self, a: f64, b: f64) -> bool {
        let Some(c) = self.clip(a, b) else { return false };
        self.insert(b, &c)
    }

    /// Mute (or un-mute) [a, b). Only the mute score changes.
    pub fn mute_range(&mut self, a: f64, b: f64, on: bool) -> bool {
        if !self.range_ok(a, b) {
            return false;
        }
        self.attempt(|s| {
            let mut v = parts(&s.mute, f64::MIN, a);
            v.push(MuteSec { start: a, end: b, muted: on });
            v.extend(parts(&s.mute, b, f64::MAX));
            s.mute = merged(v);
            true
        })
    }

    /// The song beat that timeline beat `b` plays, and whether it is muted.
    pub fn song_beat(&self, b: f64) -> Option<f64> {
        let i = self.position.partition_point(|s| s.end <= b);
        let s = self.position.get(i)?;
        (b >= s.start).then(|| s.song_start + (b - s.start))
    }

    pub fn muted_at(&self, b: f64) -> bool {
        let i = self.mute.partition_point(|s| s.end <= b);
        self.mute.get(i).map(|s| s.muted && b >= s.start).unwrap_or(false)
    }

    /// Bar and beat number shown for timeline beat `k` ("3.2").
    pub fn bar_beat(&self, k: i64) -> (i64, i64) {
        let off = self.bar_offset.rem_euclid(4) as i64;
        let rel = k - off;
        (rel.div_euclid(4) + 1 + i64::from(off > 0), rel.rem_euclid(4) + 1)
    }

    pub fn is_bar_line(&self, k: i64) -> bool {
        (k - self.bar_offset as i64).rem_euclid(4) == 0
    }
}

// ---------------------------------------------------------------------------- selection

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Selection {
    None,
    Point(f64),
    Range(f64, f64),
}

impl Selection {
    pub fn range(&self) -> Option<(f64, f64)> {
        match *self {
            Selection::Range(a, b) if b > a => Some((a, b)),
            _ => None,
        }
    }
    /// The selected point: the point itself, or a range's start.
    pub fn point(&self) -> Option<f64> {
        match *self {
            Selection::None => None,
            Selection::Point(p) => Some(p),
            Selection::Range(a, _) => Some(a),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantize {
    Auto,
    One,
    Four,
}

impl Quantize {
    /// As stored in a project file (0 = AUTO; anything else unknown reads as AUTO).
    pub fn file_value(self) -> i32 {
        match self {
            Quantize::Auto => 0,
            Quantize::One => 1,
            Quantize::Four => 4,
        }
    }
    pub fn from_file(v: i32) -> Quantize {
        match v {
            1 => Quantize::One,
            4 => Quantize::Four,
            _ => Quantize::Auto,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Quantize::Auto => "AUTO",
            Quantize::One => "1",
            Quantize::Four => "4",
        }
    }
    /// The unit in beats at a zoom step: AUTO is 4 once zoomed out past step 6.
    pub fn unit(self, zoom: usize) -> u32 {
        match self {
            Quantize::Four => 4,
            Quantize::One => 1,
            Quantize::Auto => {
                if zoom > 6 {
                    4
                } else {
                    1
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Round {
    Nearest,
    Down,
    Up,
}

/// rekordbox's `getQuantizedPoint`: a whole beat inside the score; with a unit of 4, one of
/// the two bar lines around it (those outside the score do not count).
pub fn quantized_point(s: &ScoreSet, p: f64, unit: u32, round: Round) -> f64 {
    let len = s.len();
    let p = p.round().clamp(0.0, len);
    if unit < 4 {
        return p;
    }
    let off = s.bar_offset.rem_euclid(4) as f64;
    let lower = ((p - off) / 4.0).floor() * 4.0 + off;
    if (lower - p).abs() <= EPS {
        return p;
    }
    let upper = lower + 4.0;
    let lo = (lower >= 0.0).then_some(lower);
    let hi = (upper <= len).then_some(upper);
    match (lo, hi, round) {
        (Some(l), None, _) => l,
        (None, Some(h), _) => h,
        (None, None, _) => p,
        (Some(l), Some(_), Round::Down) => l,
        (Some(_), Some(h), Round::Up) => h,
        (Some(l), Some(h), Round::Nearest) => {
            if p - l < h - p {
                l
            } else {
                h
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectCmd {
    Half,
    Double,
    Cancel,
    ByBeat(u32),
}

/// rekordbox's edit command ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCmd {
    Copy = 0,
    Insert = 1,
    Paste = 2,
    Delete = 3,
    Clone = 4,
    Clear = 5,
    Cut = 6,
    Undo = 7,
    Redo = 8,
    CopyPalette = 9,
    InsertPalette = 10,
}

pub struct UndoRec {
    pub cmd: EditCmd,
    pub prev: Arc<ScoreSet>,
    pub next: Arc<ScoreSet>,
    pub sel_before: Selection,
    pub sel_after: Selection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaletteClip {
    /// 1-based id, as written to the project file.
    pub id: u32,
    pub name: String,
    pub clip: Arc<ScoreSet>,
}

/// One open project: the score, what is selected, the clipboard, the undo history and the
/// palette.
pub struct EditSession {
    pub cur: Arc<ScoreSet>,
    pub undo: Vec<UndoRec>,
    pub redo: Vec<UndoRec>,
    pub clipboard: Option<Arc<ScoreSet>>,
    pub selection: Selection,
    pub quantize: Quantize,
    pub palettes: Vec<PaletteClip>,
    /// rekordbox's edit counter: +1 for every edit and redo, -1 for an undo. The project is
    /// modified while it differs from its value at the last save.
    pub edit_counter: i32,
    pub saved_counter: i32,
    /// The palette or the quantize setting changed since the last save.
    pub extra_dirty: bool,
}

impl EditSession {
    pub fn new(score: ScoreSet) -> EditSession {
        EditSession { cur: Arc::new(score), undo: vec![], redo: vec![], clipboard: None, selection: Selection::Point(0.0), quantize: Quantize::Auto, palettes: vec![], edit_counter: 0, saved_counter: 0, extra_dirty: false }
    }

    pub fn modified(&self) -> bool {
        self.edit_counter != self.saved_counter || self.extra_dirty
    }

    pub fn mark_saved(&mut self) {
        self.saved_counter = self.edit_counter;
        self.extra_dirty = false;
    }

    // ---- selection

    pub fn select_point(&mut self, p: f64, unit: u32) {
        self.selection = Selection::Point(quantized_point(&self.cur, p, unit, Round::Nearest));
    }

    /// A range between two positions, both put on the unit; an empty one is a point.
    pub fn select_range(&mut self, a: f64, b: f64, unit: u32) {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let qa = quantized_point(&self.cur, a, unit, Round::Nearest);
        let qb = quantized_point(&self.cur, b, unit, Round::Nearest);
        self.selection = if qb > qa { Selection::Range(qa, qb) } else { Selection::Point(qa) };
    }

    /// Shift-click: the nearer edge of the range moves to the click; from a point, the range
    /// runs between the point and the click.
    pub fn extend_to(&mut self, p: f64, unit: u32) {
        let q = quantized_point(&self.cur, p, unit, Round::Nearest);
        match self.selection {
            Selection::None => self.selection = Selection::Point(q),
            Selection::Point(a) => self.select_range(a, q, unit),
            Selection::Range(a, b) => {
                if (q - a).abs() <= (q - b).abs() {
                    self.select_range(q, b, unit)
                } else {
                    self.select_range(a, q, unit)
                }
            }
        }
    }

    pub fn can_select(&self, cmd: SelectCmd, unit: u32) -> bool {
        match cmd {
            SelectCmd::Half => self.selection.range().map(|(a, b)| b - a > unit as f64).unwrap_or(false),
            SelectCmd::Double | SelectCmd::Cancel => self.selection.range().is_some(),
            SelectCmd::ByBeat(n) => n >= unit && n % unit.max(1) == 0 && self.cur.len() > 0.0,
        }
    }

    pub fn select(&mut self, cmd: SelectCmd, unit: u32) -> bool {
        if !self.can_select(cmd, unit) {
            return false;
        }
        let s = self.cur.clone();
        let q = |p: f64| quantized_point(&s, p, unit, Round::Nearest);
        match cmd {
            SelectCmd::Half => {
                let (a, b) = self.selection.range().unwrap();
                let e = q(a + ((b - a) / 2.0).floor());
                self.selection = if e > a { Selection::Range(a, e) } else { Selection::Point(a) };
            }
            SelectCmd::Double => {
                let (a, b) = self.selection.range().unwrap();
                let e = q((a + 2.0 * (b - a)).min(s.len()));
                self.selection = if e > a { Selection::Range(a, e) } else { Selection::Point(a) };
            }
            SelectCmd::Cancel => {
                let (a, _) = self.selection.range().unwrap();
                self.selection = Selection::Point(a);
            }
            SelectCmd::ByBeat(n) => {
                let a = q(self.selection.point().unwrap_or(0.0));
                let b = q(a + n as f64);
                if b <= a {
                    return false;
                }
                self.selection = Selection::Range(a, b);
            }
        }
        true
    }

    // ---- editing

    pub fn can_edit(&self, cmd: EditCmd) -> bool {
        match cmd {
            EditCmd::Copy | EditCmd::Delete | EditCmd::Clone | EditCmd::Clear | EditCmd::Cut => self.selection.range().is_some(),
            EditCmd::Insert | EditCmd::Paste => self.clipboard.is_some() && self.selection.point().is_some(),
            EditCmd::Undo => !self.undo.is_empty(),
            EditCmd::Redo => !self.redo.is_empty(),
            EditCmd::CopyPalette | EditCmd::InsertPalette => self.cur.len() > 0.0,
        }
    }

    /// rekordbox's `ScoreEditManager::edit`: run `f` on a copy of the score; when it worked,
    /// the copy becomes the score, the step goes on the undo stack and the redo stack empties.
    fn apply(&mut self, cmd: EditCmd, f: impl FnOnce(&mut ScoreSet) -> Option<Selection>) -> bool {
        let prev = self.cur.clone();
        let mut next = (*prev).clone();
        let Some(sel_after) = f(&mut next) else { return false };
        if !next.is_valid() {
            return false;
        }
        let next = Arc::new(next);
        let sel_before = self.selection;
        self.undo.push(UndoRec { cmd, prev, next: next.clone(), sel_before, sel_after });
        self.redo.clear();
        self.cur = next;
        self.selection = sel_after;
        self.edit_counter += 1;
        true
    }

    pub fn edit(&mut self, cmd: EditCmd) -> bool {
        if !self.can_edit(cmd) {
            return false;
        }
        let sel = self.selection;
        match cmd {
            EditCmd::Copy => {
                let (a, b) = sel.range().unwrap();
                match self.cur.clip(a, b) {
                    Some(c) => {
                        self.clipboard = Some(Arc::new(c));
                        true
                    }
                    None => false,
                }
            }
            EditCmd::Cut => {
                let (a, b) = sel.range().unwrap();
                let Some(c) = self.cur.clip(a, b) else { return false };
                // (The clipboard is only replaced when the delete went through.)
                if self.apply(cmd, |s| s.delete_range(a, b).then_some(Selection::Point(a))) {
                    self.clipboard = Some(Arc::new(c));
                    true
                } else {
                    false
                }
            }
            EditCmd::Insert => {
                let c = self.clipboard.clone().unwrap();
                let p = sel.point().unwrap();
                self.apply(cmd, |s| s.insert(p, &c).then_some(Selection::Range(p, p + c.len())))
            }
            EditCmd::Paste => {
                let c = self.clipboard.clone().unwrap();
                match sel {
                    Selection::Range(a, b) => self.apply(cmd, |s| s.paste_over(a, b, &c).then_some(Selection::Range(a, a + c.len()))),
                    _ => {
                        let p = sel.point().unwrap();
                        self.apply(cmd, |s| s.paste_at(p, &c).then_some(Selection::Range(p, p + c.len())))
                    }
                }
            }
            EditCmd::Delete => {
                let (a, b) = sel.range().unwrap();
                self.apply(cmd, |s| s.delete_range(a, b).then_some(Selection::Point(a)))
            }
            EditCmd::Clone => {
                let (a, b) = sel.range().unwrap();
                self.apply(cmd, |s| s.duplicate(a, b).then_some(Selection::Range(b, b + (b - a))))
            }
            EditCmd::Clear => {
                let (a, b) = sel.range().unwrap();
                self.apply(cmd, |s| s.mute_range(a, b, true).then_some(sel))
            }
            EditCmd::Undo => {
                let r = self.undo.pop().unwrap();
                self.cur = r.prev.clone();
                self.selection = r.sel_before;
                self.redo.push(r);
                self.edit_counter -= 1;
                true
            }
            EditCmd::Redo => {
                let r = self.redo.pop().unwrap();
                self.cur = r.next.clone();
                self.selection = r.sel_after;
                self.undo.push(r);
                self.edit_counter += 1;
                true
            }
            EditCmd::CopyPalette | EditCmd::InsertPalette => false,
        }
    }

    // ---- palette

    /// The selected range as a new palette entry. Err: rekordbox's status-bar message.
    pub fn add_to_palette(&mut self, name: &str) -> Result<u32, &'static str> {
        let Some((a, b)) = self.selection.range() else { return Err("") };
        if self.palettes.len() >= PALETTE_MAX {
            return Err("Only up to 100 can be added to the palette.");
        }
        let Some(c) = self.cur.clip(a, b) else { return Err("") };
        let id = self.palettes.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        self.palettes.push(PaletteClip { id, name: name.to_string(), clip: Arc::new(c) });
        self.extra_dirty = true;
        Ok(id)
    }

    pub fn remove_palette(&mut self, id: u32) {
        let n = self.palettes.len();
        self.palettes.retain(|p| p.id != id);
        self.extra_dirty |= self.palettes.len() != n;
    }

    pub fn rename_palette(&mut self, id: u32, name: &str) {
        if let Some(p) = self.palettes.iter_mut().find(|p| p.id == id) {
            if p.name != name {
                p.name = name.to_string();
                self.extra_dirty = true;
            }
        }
    }

    /// The palette entry's "Copy": its clip becomes the clipboard.
    pub fn copy_palette(&mut self, id: u32) -> bool {
        match self.palettes.iter().find(|p| p.id == id) {
            Some(p) => {
                self.clipboard = Some(p.clip.clone());
                true
            }
            None => false,
        }
    }

    /// A palette entry dropped on the waveform: inserted at that beat.
    pub fn insert_palette(&mut self, id: u32, at: f64) -> bool {
        let Some(c) = self.palettes.iter().find(|p| p.id == id).map(|p| p.clip.clone()) else { return false };
        let p = quantized_point(&self.cur, at, 1, Round::Nearest);
        self.apply(EditCmd::InsertPalette, |s| s.insert(p, &c).then_some(Selection::Range(p, p + c.len())))
    }
}

/// Length of a selection for the read-out: ("4.0 Bars", "16 Beats", "00:07").
pub fn readout(beats: f64, bpm: f64) -> (String, String, String) {
    let n = beats.round().max(0.0) as i64;
    let secs = (beats * 60.0 / bpm.max(1e-6)).floor() as i64;
    (format!("{}.{} Bars", n / 4, n % 4), format!("{n} Beats"), format!("{:02}:{:02}", secs / 60, secs % 60))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A 120 BPM song with a steady grid starting at 0: `beats` + 1 grid beats.
    pub fn song(beats: usize) -> ScoreSet {
        let org: Vec<(f64, f64)> = (0..=beats).map(|i| (120.0, i as f64 * 500.0)).collect();
        let g = SongGrid::new(org, 0, beats as f64 * 500.0, 120.0);
        let cue = |id: i32, a: f64, b: f64| CueLoop { cue_id: id, pos_in: a, pos_out: b, appended: CueAppended { db_cue_id: 100 + id as i64, color: 3, comment: format!("c{id}"), ..Default::default() } };
        let prepared = Prepared {
            hot: vec![cue(0, 4.0, NOT_LOOP), cue(1, 16.0, 24.0), cue(2, 40.5, NOT_LOOP)],
            memory: vec![cue(0, 8.0, NOT_LOOP)],
            censor: vec![Censor { pos_in: 20.0, pos_out: 22.0, appended: CensorAppended { ac_id: 7, effect: 2, on: true, params: vec![1, 2] } }],
        };
        ScoreSet::new(1, 120.0, g, prepared)
    }

    fn pos(s: &ScoreSet) -> Vec<(f64, f64, f64, f64)> {
        s.position.iter().map(|p| (p.start, p.end, p.song_start, p.song_end)).collect()
    }

    #[test]
    fn a_new_score_is_the_whole_song_once() {
        let s = song(64);
        assert!(s.is_valid());
        assert_eq!(s.len(), 64.0);
        assert_eq!(pos(&s), vec![(0.0, 64.0, 0.0, 64.0)]);
        assert_eq!(s.mute, vec![MuteSec { start: 0.0, end: 64.0, muted: false }]);
        assert_eq!(s.tempo.len(), 1);
        assert_eq!(s.len_ms(), 32000.0);
        assert_eq!(s.song_beat(10.5), Some(10.5));
    }

    #[test]
    fn song_grid_is_extended_at_the_nominal_tempo() {
        // First beat at 120 ms, last at 9620 ms, file 10.4 s long, 120 BPM.
        let org: Vec<(f64, f64)> = (0..20).map(|i| (120.0, 120.0 + i as f64 * 500.0)).collect();
        let g = SongGrid::new(org, 2, 10400.0, 120.0);
        assert_eq!((g.pre, g.post), (1, 2));
        assert_eq!(g.beats(), 23);
        assert_eq!(g.bar_offset(), 3);
        assert!((g.ppq_to_ms(0.0) - (-380.0)).abs() < 1e-9);
        assert!((g.ppq_to_ms(1.0) - 120.0).abs() < 1e-9);
        assert!((g.ppq_to_ms(1.5) - 370.0).abs() < 1e-9);
        assert!((g.ppq_to_ms(22.0) - 10620.0).abs() < 1e-9);
        for b in [0.0, 0.25, 1.0, 7.75, 22.0, 25.0, -2.0] {
            assert!((g.ms_to_ppq(g.ppq_to_ms(b)) - b).abs() < 1e-9, "{b}");
        }
        assert!(!g.is_dynamic());
        // A grid whose tempo moves: beats 500 ms apart, then 480 ms apart.
        let mut t = 0.0;
        let dynamic: Vec<(f64, f64)> = (0..16).map(|i| {
            let v = (if i < 8 { 120.0 } else { 125.0 }, t);
            t += if i < 8 { 500.0 } else { 480.0 };
            v
        }).collect();
        let d = SongGrid::new(dynamic, 0, t, 120.0);
        assert!(d.is_dynamic());
        // Inside a beat the mapping is linear in that beat's own length.
        assert!((d.ppq_to_ms(8.5) - (4000.0 + 240.0)).abs() < 1e-9);
        assert!((d.ms_to_ppq(4240.0) - 8.5).abs() < 1e-9);
    }

    #[test]
    fn clip_keeps_the_range_from_zero() {
        let s = song(64);
        let c = s.clip(16.0, 24.0).unwrap();
        assert!(c.is_valid());
        assert_eq!(pos(&c), vec![(0.0, 8.0, 16.0, 24.0)]);
        assert_eq!(c.len(), 8.0);
        // The loop that fills the range, and the censor region inside it, come along from 0.
        assert_eq!(c.prepared.hot.len(), 1);
        assert_eq!((c.prepared.hot[0].pos_in, c.prepared.hot[0].pos_out), (0.0, 8.0));
        assert_eq!(c.prepared.hot[0].appended.comment, "c1");
        assert_eq!((c.prepared.censor[0].pos_in, c.prepared.censor[0].pos_out, c.prepared.censor[0].appended.ac_id), (4.0, 6.0, 7));
        // A loop only partly inside is cut to the range.
        let c2 = s.clip(20.0, 32.0).unwrap();
        assert_eq!((c2.prepared.hot[0].pos_in, c2.prepared.hot[0].pos_out), (0.0, 4.0));
        // Not a whole beat, empty, or past the end: refused.
        assert!(s.clip(1.5, 4.0).is_none());
        assert!(s.clip(4.0, 4.0).is_none());
        assert!(s.clip(60.0, 65.0).is_none());
        assert_eq!(s.clip(3.0, 8.0).unwrap().bar_offset, 1);
    }

    #[test]
    fn delete_closes_the_gap_and_moves_what_follows() {
        let mut s = song(64);
        assert!(s.delete_range(8.0, 16.0));
        assert!(s.is_valid());
        assert_eq!(s.len(), 56.0);
        assert_eq!(pos(&s), vec![(0.0, 8.0, 0.0, 8.0), (8.0, 56.0, 16.0, 64.0)]);
        assert_eq!(s.song_beat(8.0), Some(16.0));
        // Cue at 4 stays, memory cue at 8 (inside) goes, the loop 16..24 is now 8..16, the
        // cue at 40.5 is at 32.5.
        let hot: Vec<(i32, f64, f64)> = s.prepared.hot.iter().map(|c| (c.cue_id, c.pos_in, c.pos_out)).collect();
        assert_eq!(hot, vec![(0, 4.0, NOT_LOOP), (1, 8.0, 16.0), (2, 32.5, NOT_LOOP)]);
        assert!(s.prepared.memory.is_empty());
        assert_eq!((s.prepared.censor[0].pos_in, s.prepared.censor[0].pos_out), (12.0, 14.0));
        // A loop that reaches into the deleted range is shortened; one that spans it loses
        // the range's length.
        let mut t = song(64);
        assert!(t.delete_range(20.0, 28.0));
        assert_eq!((t.prepared.hot[1].pos_in, t.prepared.hot[1].pos_out), (16.0, 20.0));
        let mut u = song(64);
        assert!(u.delete_range(18.0, 20.0));
        assert_eq!((u.prepared.hot[1].pos_in, u.prepared.hot[1].pos_out), (16.0, 22.0));
        // Everything cannot be deleted (a score is never empty), nor a fractional range.
        let mut e = song(64);
        assert!(!e.delete_range(0.0, 64.0));
        assert!(!e.delete_range(0.5, 2.0));
        assert_eq!(e, song(64));
    }

    #[test]
    fn insert_opens_a_gap_for_the_clip() {
        let mut s = song(64);
        let c = s.clip(32.0, 36.0).unwrap();
        assert!(s.insert(8.0, &c));
        assert!(s.is_valid());
        assert_eq!(s.len(), 68.0);
        assert_eq!(pos(&s), vec![(0.0, 8.0, 0.0, 8.0), (8.0, 12.0, 32.0, 36.0), (12.0, 68.0, 8.0, 64.0)]);
        // The memory cue at 8 is "at or after" the insert point and moves; hot cue 0 stays.
        assert_eq!(s.prepared.memory[0].pos_in, 12.0);
        assert_eq!(s.prepared.hot[0].pos_in, 4.0);
        assert_eq!((s.prepared.hot[1].pos_in, s.prepared.hot[1].pos_out), (20.0, 28.0));
        // Inserting at the very end appends; inserting material next to itself merges back.
        let mut t = song(64);
        let tail = t.clip(60.0, 64.0).unwrap();
        assert!(t.insert(64.0, &tail));
        assert_eq!(pos(&t), vec![(0.0, 64.0, 0.0, 64.0), (64.0, 68.0, 60.0, 64.0)]);
        let mut u = song(64);
        let head = u.clip(0.0, 4.0).unwrap();
        assert!(u.delete_range(0.0, 4.0));
        assert!(u.insert(0.0, &head));
        assert_eq!(pos(&u), vec![(0.0, 64.0, 0.0, 64.0)]);
        // An insert inside a loop ends the loop at the insert.
        let mut v = song(64);
        let c = v.clip(0.0, 4.0).unwrap();
        assert!(v.insert(20.0, &c));
        assert_eq!((v.prepared.hot[1].pos_in, v.prepared.hot[1].pos_out), (16.0, 20.0));
        // The clip's own cue comes in at its new place (hot cue 0 was at beat 4 of the song,
        // outside the clip 0..4, so nothing is added here).
        assert_eq!(v.prepared.hot.len(), 3);
        // Refused: a fractional position, past the end, another tempo.
        assert!(!v.insert(1.5, &c));
        assert!(!v.insert(100.0, &c));
        let mut other = c.clone();
        other.bpm = 121.0;
        other.tempo[0].bpm = 121.0;
        assert!(!v.insert(4.0, &other));
    }

    #[test]
    fn clone_puts_a_copy_after_the_range_with_its_cues() {
        let mut s = song(64);
        assert!(s.duplicate(0.0, 8.0));
        assert_eq!(s.len(), 72.0);
        assert_eq!(pos(&s), vec![(0.0, 8.0, 0.0, 8.0), (8.0, 72.0, 0.0, 64.0)]);
        // Hot cue 0 (beat 4) exists twice now: at 4 and at 12.
        let zero: Vec<f64> = s.prepared.hot.iter().filter(|c| c.cue_id == 0).map(|c| c.pos_in).collect();
        assert_eq!(zero, vec![4.0, 12.0]);
    }

    #[test]
    fn paste_overwrites_and_paste_over_replaces() {
        // At a point: as much as the clip is long is replaced.
        let mut s = song(64);
        let c = s.clip(32.0, 40.0).unwrap();
        assert!(s.paste_at(0.0, &c));
        assert_eq!(s.len(), 64.0);
        assert_eq!(pos(&s), vec![(0.0, 8.0, 32.0, 40.0), (8.0, 64.0, 8.0, 64.0)]);
        // Near the end the score grows by what does not fit.
        let mut t = song(64);
        assert!(t.paste_at(60.0, &c));
        assert_eq!(t.len(), 68.0);
        assert_eq!(pos(&t), vec![(0.0, 60.0, 0.0, 60.0), (60.0, 68.0, 32.0, 40.0)]);
        // Over a range: the range is replaced, the length follows the clip.
        let mut u = song(64);
        assert!(u.paste_over(0.0, 16.0, &c));
        assert_eq!(u.len(), 56.0);
        assert_eq!(pos(&u), vec![(0.0, 8.0, 32.0, 40.0), (8.0, 56.0, 16.0, 64.0)]);
        // A clip as long as the whole score can still replace all of it.
        let mut w = song(8);
        let all = w.clip(0.0, 8.0).unwrap();
        assert!(w.paste_over(0.0, 8.0, &all));
        assert_eq!(w.len(), 8.0);
    }

    #[test]
    fn mute_changes_only_the_mute_score_and_stays_merged() {
        let mut s = song(64);
        let before = s.clone();
        assert!(s.mute_range(8.0, 16.0, true));
        assert_eq!(s.position, before.position);
        assert_eq!(s.prepared, before.prepared);
        let m: Vec<(f64, f64, bool)> = s.mute.iter().map(|m| (m.start, m.end, m.muted)).collect();
        assert_eq!(m, vec![(0.0, 8.0, false), (8.0, 16.0, true), (16.0, 64.0, false)]);
        assert!(s.muted_at(8.0) && s.muted_at(15.9) && !s.muted_at(16.0) && !s.muted_at(7.9));
        // A neighbouring mute joins the first; un-muting everything is one section again.
        assert!(s.mute_range(16.0, 20.0, true));
        assert_eq!(s.mute.len(), 3);
        assert!(s.mute_range(0.0, 64.0, false));
        assert_eq!(s.mute, before.mute);
        assert!(s.is_valid());
        // Muted material stays muted when it is cloned.
        assert!(s.mute_range(4.0, 8.0, true));
        assert!(s.duplicate(0.0, 8.0));
        let m: Vec<(f64, f64, bool)> = s.mute.iter().map(|m| (m.start, m.end, m.muted)).collect();
        assert_eq!(m, vec![(0.0, 4.0, false), (4.0, 8.0, true), (8.0, 12.0, false), (12.0, 16.0, true), (16.0, 72.0, false)]);
    }

    #[test]
    fn invalid_sets_are_recognised() {
        let s = song(16);
        let mut a = s.clone();
        a.position[0].song_end = 20.0;
        assert!(!a.is_valid(), "section lengths must agree");
        let mut b = s.clone();
        b.mute = vec![MuteSec { start: 0.0, end: 8.0, muted: false }, MuteSec { start: 8.0, end: 16.0, muted: false }];
        assert!(!b.is_valid(), "neighbouring mute sections must differ");
        let mut c = s.clone();
        c.tempo[0].end = 12.0;
        assert!(!c.is_valid(), "all scores cover the same range");
        let mut d = s.clone();
        d.position = vec![PosSec { start: 0.0, end: 8.0, song_start: 0.0, song_end: 8.0 }, PosSec { start: 9.0, end: 17.0, song_start: 8.0, song_end: 16.0 }];
        assert!(!d.is_valid(), "sections are contiguous");
        let mut e = s.clone();
        e.track_id = 0;
        assert!(!e.is_valid());
    }

    #[test]
    fn quantize_points_and_units() {
        let mut s = song(64);
        s.bar_offset = 1;
        assert_eq!(quantized_point(&s, 6.4, 1, Round::Nearest), 6.0);
        assert_eq!(quantized_point(&s, 80.0, 1, Round::Nearest), 64.0);
        assert_eq!(quantized_point(&s, -3.0, 1, Round::Nearest), 0.0);
        // Bar lines at 1, 5, 9 ...
        assert_eq!(quantized_point(&s, 6.0, 4, Round::Nearest), 5.0);
        assert_eq!(quantized_point(&s, 7.0, 4, Round::Nearest), 9.0, "ties go up");
        assert_eq!(quantized_point(&s, 7.0, 4, Round::Down), 5.0);
        assert_eq!(quantized_point(&s, 6.0, 4, Round::Up), 9.0);
        assert_eq!(quantized_point(&s, 9.0, 4, Round::Down), 9.0);
        // Before the first bar line only the upper one is inside the score; after the last
        // one (61) only the lower.
        assert_eq!(quantized_point(&s, 0.0, 4, Round::Nearest), 1.0);
        assert_eq!(quantized_point(&s, 63.0, 4, Round::Nearest), 61.0);
        assert_eq!(Quantize::Auto.unit(6), 1);
        assert_eq!(Quantize::Auto.unit(7), 4);
        assert_eq!(Quantize::One.unit(15), 1);
        assert_eq!(Quantize::Four.unit(1), 4);
        assert_eq!(Quantize::from_file(Quantize::Four.file_value()), Quantize::Four);
        assert_eq!(Quantize::from_file(3), Quantize::Auto);
        assert_eq!(s.bar_beat(0), (1, 4));
        assert_eq!(s.bar_beat(1), (2, 1));
        assert_eq!(song(8).bar_beat(5), (2, 2));
    }

    #[test]
    fn selection_commands_follow_the_rules() {
        let mut e = EditSession::new(song(64));
        assert_eq!(e.selection, Selection::Point(0.0));
        // Nothing but BEAT SELECT works on a point.
        assert!(!e.can_select(SelectCmd::Half, 1) && !e.can_select(SelectCmd::Double, 1) && !e.can_select(SelectCmd::Cancel, 1));
        assert!(e.select(SelectCmd::ByBeat(8), 1));
        assert_eq!(e.selection, Selection::Range(0.0, 8.0));
        assert!(e.select(SelectCmd::Half, 1));
        assert_eq!(e.selection, Selection::Range(0.0, 4.0));
        assert!(e.select(SelectCmd::Double, 1));
        assert!(e.select(SelectCmd::Double, 1));
        assert_eq!(e.selection, Selection::Range(0.0, 16.0));
        // With a unit of 4 the 1 and 2 beat pads are off, and Half stops at one bar.
        assert!(!e.can_select(SelectCmd::ByBeat(1), 4) && !e.can_select(SelectCmd::ByBeat(2), 4) && e.can_select(SelectCmd::ByBeat(4), 4));
        assert!(e.select(SelectCmd::Half, 4));
        assert!(e.select(SelectCmd::Half, 4));
        assert_eq!(e.selection, Selection::Range(0.0, 4.0));
        assert!(!e.can_select(SelectCmd::Half, 4));
        // Half of an odd length rounds down; half of one beat is refused.
        e.select_range(10.0, 13.0, 1);
        assert!(e.select(SelectCmd::Half, 1));
        assert_eq!(e.selection, Selection::Range(10.0, 11.0));
        assert!(!e.select(SelectCmd::Half, 1));
        // Double stops at the end of the score.
        e.select_range(40.0, 60.0, 1);
        assert!(e.select(SelectCmd::Double, 1));
        assert_eq!(e.selection, Selection::Range(40.0, 64.0));
        // Cancel leaves the start as a point; BEAT SELECT at the end has nowhere to go.
        assert!(e.select(SelectCmd::Cancel, 1));
        assert_eq!(e.selection, Selection::Point(40.0));
        e.select_point(64.0, 1);
        assert!(!e.select(SelectCmd::ByBeat(4), 1));
        // Clicks and drags land on the unit; shift-click moves the nearer edge.
        e.select_point(5.4, 1);
        assert_eq!(e.selection, Selection::Point(5.0));
        e.select_range(9.2, 2.6, 1);
        assert_eq!(e.selection, Selection::Range(3.0, 9.0));
        e.extend_to(11.0, 1);
        assert_eq!(e.selection, Selection::Range(3.0, 11.0));
        e.extend_to(4.0, 1);
        assert_eq!(e.selection, Selection::Range(4.0, 11.0));
        e.select_point(20.0, 1);
        e.extend_to(12.0, 1);
        assert_eq!(e.selection, Selection::Range(12.0, 20.0));
        e.select_range(6.0, 6.3, 1);
        assert_eq!(e.selection, Selection::Point(6.0));
    }

    #[test]
    fn edit_commands_leave_the_selection_rekordbox_leaves() {
        let mut e = EditSession::new(song(64));
        // Without a range or a clipboard nothing can run.
        for c in [EditCmd::Copy, EditCmd::Cut, EditCmd::Delete, EditCmd::Clone, EditCmd::Clear, EditCmd::Insert, EditCmd::Paste, EditCmd::Undo, EditCmd::Redo] {
            assert!(!e.can_edit(c), "{c:?}");
            assert!(!e.edit(c));
        }
        e.select_range(8.0, 16.0, 1);
        // Copy: clipboard only, no undo step, not modified.
        assert!(e.edit(EditCmd::Copy));
        assert_eq!(e.selection, Selection::Range(8.0, 16.0));
        assert!(e.undo.is_empty() && !e.modified());
        assert_eq!(e.clipboard.as_ref().unwrap().len(), 8.0);
        // Clone: the copy is selected.
        assert!(e.edit(EditCmd::Clone));
        assert_eq!(e.selection, Selection::Range(16.0, 24.0));
        assert_eq!(e.cur.len(), 72.0);
        // Clear: selection unchanged.
        assert!(e.edit(EditCmd::Clear));
        assert_eq!(e.selection, Selection::Range(16.0, 24.0));
        assert!(e.cur.muted_at(16.0));
        // Delete: a point at the range's start.
        assert!(e.edit(EditCmd::Delete));
        assert_eq!(e.selection, Selection::Point(16.0));
        assert_eq!(e.cur.len(), 64.0);
        // Insert at the point: the inserted clip is selected.
        assert!(e.edit(EditCmd::Insert));
        assert_eq!(e.selection, Selection::Range(16.0, 24.0));
        assert_eq!(e.cur.len(), 72.0);
        // Insert with a range selected goes in at the range's start.
        assert!(e.edit(EditCmd::Insert));
        assert_eq!(e.selection, Selection::Range(16.0, 24.0));
        assert_eq!(e.cur.len(), 80.0);
        // Paste over a range replaces it; the pasted clip is selected.
        e.select_range(0.0, 4.0, 1);
        assert!(e.edit(EditCmd::Paste));
        assert_eq!(e.selection, Selection::Range(0.0, 8.0));
        assert_eq!(e.cur.len(), 84.0);
        // Paste at a point overwrites.
        e.select_point(40.0, 1);
        assert!(e.edit(EditCmd::Paste));
        assert_eq!(e.selection, Selection::Range(40.0, 48.0));
        assert_eq!(e.cur.len(), 84.0);
        // Cut: clipboard replaced, a point at the start.
        e.select_range(2.0, 4.0, 1);
        assert!(e.edit(EditCmd::Cut));
        assert_eq!(e.selection, Selection::Point(2.0));
        assert_eq!(e.clipboard.as_ref().unwrap().len(), 2.0);
        assert_eq!(e.cur.len(), 82.0);
        assert!(e.cur.is_valid());
        assert_eq!(e.undo.len(), 8);
        assert_eq!(e.edit_counter, 8);
    }

    #[test]
    fn undo_and_redo_restore_score_selection_and_the_modified_state() {
        let mut e = EditSession::new(song(64));
        let start = e.cur.clone();
        e.select_range(8.0, 16.0, 1);
        assert!(e.edit(EditCmd::Delete));
        let after_delete = e.cur.clone();
        e.select_range(0.0, 4.0, 1);
        assert!(e.edit(EditCmd::Clone));
        assert!(e.modified());
        // Undo puts back the score and the selection from before each step.
        assert!(e.edit(EditCmd::Undo));
        assert_eq!(*e.cur, *after_delete);
        assert_eq!(e.selection, Selection::Range(0.0, 4.0));
        assert!(e.edit(EditCmd::Undo));
        assert_eq!(*e.cur, *start);
        assert_eq!(e.selection, Selection::Range(8.0, 16.0));
        assert!(!e.modified(), "back at the saved state");
        assert!(!e.can_edit(EditCmd::Undo));
        // Redo walks forward again, with the selection each step left.
        assert!(e.edit(EditCmd::Redo));
        assert_eq!(*e.cur, *after_delete);
        assert_eq!(e.selection, Selection::Point(8.0));
        assert!(e.edit(EditCmd::Redo));
        assert_eq!(e.selection, Selection::Range(4.0, 8.0));
        assert!(!e.can_edit(EditCmd::Redo));
        // Saving here, then undoing, makes it modified again; a new edit clears redo.
        e.mark_saved();
        assert!(!e.modified());
        assert!(e.edit(EditCmd::Undo));
        assert!(e.modified());
        e.select_range(0.0, 2.0, 1);
        assert!(e.edit(EditCmd::Clear));
        assert!(e.redo.is_empty());
        // An edit that cannot work leaves everything as it was.
        e.select_range(0.0, e.cur.len(), 1);
        let n = e.undo.len();
        assert!(!e.edit(EditCmd::Delete));
        assert_eq!(e.undo.len(), n);
    }

    #[test]
    fn palette_holds_up_to_100_clips() {
        let mut e = EditSession::new(song(64));
        assert!(e.add_to_palette("x").is_err(), "needs a range");
        e.select_range(4.0, 8.0, 1);
        let id = e.add_to_palette("Intro").unwrap();
        assert_eq!(id, 1);
        assert!(e.modified());
        assert_eq!(e.selection, Selection::Range(4.0, 8.0));
        e.rename_palette(id, "Break");
        assert_eq!(e.palettes[0].name, "Break");
        // Copy makes it the clipboard; a drop inserts it and selects it, as one undo step.
        assert!(e.copy_palette(id));
        assert_eq!(e.clipboard.as_ref().unwrap().len(), 4.0);
        assert!(e.insert_palette(id, 16.2));
        assert_eq!(e.selection, Selection::Range(16.0, 20.0));
        assert_eq!(e.cur.len(), 68.0);
        assert_eq!(e.undo.last().unwrap().cmd, EditCmd::InsertPalette);
        e.select_range(0.0, 1.0, 1);
        for _ in 1..PALETTE_MAX {
            e.add_to_palette("n").unwrap();
        }
        assert_eq!(e.add_to_palette("n"), Err("Only up to 100 can be added to the palette."));
        e.remove_palette(id);
        assert_eq!(e.palettes.len(), PALETTE_MAX - 1);
        assert!(e.add_to_palette("again").is_ok());
    }

    #[test]
    fn readout_text() {
        assert_eq!(readout(16.0, 120.0), ("4.0 Bars".into(), "16 Beats".into(), "00:08".into()));
        assert_eq!(readout(6.0, 128.0), ("1.2 Bars".into(), "6 Beats".into(), "00:02".into()));
        assert_eq!(readout(512.0, 120.0).2, "04:16");
    }
}
