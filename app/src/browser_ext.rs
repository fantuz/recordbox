//! Browser additions matched to rekordbox 7.2.19: Track Suggestion (ranking from the local
//! library), the playlist Column View model, the CUE Analysis Playlist (personal CUE
//! tendency) and the settings of the right-hand panels. The drawing lives in
//! ui_browser_ext.rs.

use crate::app::{key_distance, App, View};
use crate::model::{BeatGrid, Cue, CueKind, NodeKind, PlaylistNode, Track};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const CUE_PLAYLIST_NAME: &str = "CUE Analysis Playlist";

/// Browser layout and panel settings kept with the preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserPrefs {
    /// Right edge of the tree pane in points from the window's left edge (the divider between
    /// the tree and the track list can be dragged).
    pub tree_w: f32,
    /// Related Tracks window (right strip).
    pub related_panel: bool,
    /// Track Suggestion window (right strip).
    pub suggest_panel: bool,
    /// Sub-browser under the browser (the tile above the tree) instead of beside it.
    pub sub_horizontal: bool,
    /// Playlists shown as columns instead of a tree.
    pub column_view: bool,
    /// Whose related tracks / suggestions are listed: 0 LIST (selected track), 1 MASTER,
    /// 2 player A, 3 player B.
    pub related_src: u8,
    pub suggest_src: u8,
    /// Search target list: 0 = Collection, else a playlist or folder.
    pub related_target: i64,
    pub suggest_target: i64,
    /// What a row shows on its right: 0 Auto (BPM and key), 1 Genre, 2 Rating, 3 Time,
    /// 4 Date Added.
    pub related_info: u8,
    pub suggest_info: u8,
    /// Track Suggestion ranking: 0 Rank, 1 Era, 2 Mood, 3 Association.
    pub suggest_kind: u8,
    /// Track name and search target rows of the two windows are shown.
    pub sub_head: bool,
    /// Personal CUE tendency learnt from the CUE Analysis Playlist.
    pub cue_tendency: CueTendency,
}

impl Default for BrowserPrefs {
    fn default() -> Self {
        BrowserPrefs { tree_w: 248.0, related_panel: false, suggest_panel: false, sub_horizontal: false, column_view: false, related_src: 1, suggest_src: 1, related_target: 0, suggest_target: 0, related_info: 0, suggest_info: 0, suggest_kind: 0, sub_head: true, cue_tendency: Default::default() }
    }
}

pub const INFO_CHOICES: [&str; 5] = ["Auto", "Genre", "Rating", "Time", "Date Added"];
pub const SUGGEST_KINDS: [&str; 4] = ["Rank", "Era", "Mood", "Association"];
pub const SUGGEST_TIPS: [&str; 4] = [
    "Track Suggestion:\nRanks every track in your library according to how well it matches with the one you're currently playing.",
    "Era:\nTracks with a close year of release.",
    "Mood:\nTracks with a similar mood.",
    "Association:\nTracks sharing traits such as artist, label etc.",
];

// ------------------------------------------------------------------------------ track suggestion

fn year_of(t: &Track) -> Option<i32> {
    if t.year > 1900 {
        return Some(t.year);
    }
    t.release_date.get(..4).and_then(|y| y.parse::<i32>().ok()).filter(|y| *y > 1900)
}

/// Era: 1 for the same year, falling to 0 ten years apart. None when a year is unknown.
pub fn era_score(a: &Track, b: &Track) -> Option<f64> {
    let d = (year_of(a)? - year_of(b)?).abs() as f64;
    Some((1.0 - d / 10.0).max(0.0))
}

fn people(s: &str) -> HashSet<String> {
    let folded = crate::smart::fold(s);
    let mut cut = folded.clone();
    for sep in [" feat. ", " feat ", " ft. ", " ft ", " featuring ", " vs. ", " vs ", " x ", " & ", " and ", ";", "/"] {
        cut = cut.replace(sep, ",");
    }
    cut.split(',').map(|p| p.trim().trim_matches(|c| c == '(' || c == ')').to_string()).filter(|p| !p.is_empty()).collect()
}

/// Association: shared artists (title "feat." names count), remixer, label, album and genre.
/// 0 when nothing is shared.
pub fn association_score(a: &Track, b: &Track) -> f64 {
    let same = |x: &str, y: &str| !x.trim().is_empty() && crate::smart::fold(x.trim()) == crate::smart::fold(y.trim());
    let pa: HashSet<String> = people(&a.artist).union(&people(&a.remixer)).cloned().chain(people(&a.original_artist)).collect();
    let pb: HashSet<String> = people(&b.artist).union(&people(&b.remixer)).cloned().chain(people(&b.original_artist)).collect();
    let mut s = 0.0;
    let shared = pa.intersection(&pb).count();
    if shared > 0 {
        s += 0.5 + 0.1 * (shared.min(3) - 1) as f64;
    } else {
        // "Including when the title contains artist name (e.g. feat..)"
        let (ta, tb) = (crate::smart::fold(&a.title), crate::smart::fold(&b.title));
        if pa.iter().any(|p| p.len() > 2 && tb.contains(p.as_str())) || pb.iter().any(|p| p.len() > 2 && ta.contains(p.as_str())) {
            s += 0.35;
        }
    }
    if same(&a.label, &b.label) {
        s += 0.3;
    }
    if same(&a.album, &b.album) {
        s += 0.15;
    }
    if same(&a.genre, &b.genre) {
        s += 0.1;
    }
    f64::min(s, 1.0)
}

/// Spectral balance and dynamics of a track from its stored preview waveform (400 columns of
/// peak and RMS levels: full, low, mid, high): low, mid and high share of the level, and how
/// much the level varies along the track.
pub fn mood_features(preview: &[u8]) -> Option<[f64; 4]> {
    let cols: Vec<&[u8]> = preview.chunks_exact(8).collect();
    if cols.len() < 16 {
        return None;
    }
    let mean = |k: usize| cols.iter().map(|c| c[k] as f64).sum::<f64>() / cols.len() as f64;
    let (full, low, mid, high) = (mean(4), mean(5), mean(6), mean(7));
    let sum = low + mid + high;
    if full <= 0.5 || sum <= 0.5 {
        return None;
    }
    let var = cols.iter().map(|c| (c[4] as f64 - full).powi(2)).sum::<f64>() / cols.len() as f64;
    Some([low / sum, mid / sum, high / sum, (var.sqrt() / full).min(1.0)])
}

/// Mood: the same mode (major/minor), a similar sound (balance and dynamics of the waveform),
/// a similar tempo and shared My Tags. None when neither track gives anything to compare.
pub fn mood_score(a: &Track, b: &Track, fa: Option<[f64; 4]>, fb: Option<[f64; 4]>, tags_a: Option<&HashSet<i64>>, tags_b: Option<&HashSet<i64>>) -> Option<f64> {
    let mut parts: Vec<(f64, f64)> = vec![];
    if (0..24).contains(&a.key) && (0..24).contains(&b.key) {
        parts.push((if a.key / 12 == b.key / 12 { 1.0 } else { 0.0 }, 0.25));
    }
    if let (Some(x), Some(y)) = (fa, fb) {
        let d = ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt() / 0.25 + (x[3] - y[3]).abs() / 0.5;
        parts.push(((1.0 - d).max(0.0), 0.45));
    }
    if let Some(d) = bpm_gap(a.bpm, b.bpm) {
        parts.push(((1.0 - d / 12.0).max(0.0), 0.15));
    }
    if let (Some(x), Some(y)) = (tags_a, tags_b) {
        if !x.is_empty() && !y.is_empty() {
            let inter = x.intersection(y).count() as f64;
            parts.push((inter / (x.len() + y.len()) as f64 * 2.0, 0.15));
        }
    }
    weighted(&parts)
}

/// Smallest tempo difference in percent, counting half and double tempo.
pub fn bpm_gap(a: f64, b: f64) -> Option<f64> {
    if a <= 0.0 || b <= 0.0 {
        return None;
    }
    Some([b, b * 2.0, b / 2.0].iter().map(|x| ((x - a) / a * 100.0).abs()).fold(f64::MAX, f64::min))
}

/// How well two tracks mix: tempo within 8 % and keys close on the key wheel.
pub fn mix_score(a: &Track, b: &Track) -> Option<f64> {
    let mut parts: Vec<(f64, f64)> = vec![];
    if let Some(d) = bpm_gap(a.bpm, b.bpm) {
        parts.push(((1.0 - d / 8.0).max(0.0), 0.55));
    }
    if let Some(d) = key_distance(a.key, b.key) {
        parts.push(((1.0 - d as f64 / 3.0).max(0.0), 0.45));
    }
    weighted(&parts)
}

fn weighted(parts: &[(f64, f64)]) -> Option<f64> {
    let w: f64 = parts.iter().map(|p| p.1).sum();
    if w <= 0.0 {
        return None;
    }
    Some(parts.iter().map(|p| p.0 * p.1).sum::<f64>() / w)
}

/// The four scores of a candidate against the source track: (rank, era, mood, association),
/// each 0..1. Rank blends the others with how well the two tracks mix; a part that cannot be
/// judged (no year, not analysed) is left out of the blend.
pub fn suggestion_scores(src: &Track, t: &Track, f_src: Option<[f64; 4]>, f_t: Option<[f64; 4]>, tags_src: Option<&HashSet<i64>>, tags_t: Option<&HashSet<i64>>, matched: bool) -> [f64; 4] {
    let era = era_score(src, t);
    let mood = mood_score(src, t, f_src, f_t, tags_src, tags_t);
    let assoc = association_score(src, t);
    let mut parts: Vec<(f64, f64)> = vec![(assoc, 0.2)];
    if let Some(m) = mix_score(src, t) {
        parts.push((m, 0.45));
    }
    if let Some(m) = mood {
        parts.push((m, 0.2));
    }
    if let Some(e) = era {
        parts.push((e, 0.15));
    }
    let mut rank = weighted(&parts).unwrap_or(0.0);
    // A pair the user registered as a good match always ranks above the rest.
    if matched {
        rank = 1.0 + rank;
    }
    [rank, era.unwrap_or(0.0), mood.unwrap_or(0.0), assoc]
}

/// Orders candidates by one of the scores, best first; candidates scoring 0 on Era, Mood or
/// Association are left out (they share nothing), ties keep the candidates' order.
pub fn rank_suggestions(mut scored: Vec<(i64, [f64; 4])>, kind: u8) -> Vec<(i64, f64)> {
    let k = (kind as usize).min(3);
    if k > 0 {
        scored.retain(|s| s.1[k] > 0.0);
    }
    scored.sort_by(|a, b| b.1[k].partial_cmp(&a.1[k]).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|s| (s.0, s.1[k])).collect()
}

impl App {
    /// Source track of a sub-window: the selected track (LIST) or the track on a player.
    pub fn panel_source(&self, src: u8) -> Option<i64> {
        match src {
            0 => self.anchor_id.filter(|id| self.selected.contains(id)).or_else(|| self.list.iter().copied().find(|id| self.selected.contains(id))),
            3 => self.deck_track(1).or(self.deck.track),
            2 => self.deck_track(0).or(self.deck.track),
            _ => self.deck.track,
        }
    }

    /// Track loaded on player `n` (0 = A, 1 = B), whichever deck is the active one.
    fn deck_track(&self, n: u8) -> Option<i64> {
        self.deck_of(n).track
    }

    /// Tracks a sub-window searches: the collection, or a playlist / folder.
    pub fn target_tracks(&self, target: i64) -> Vec<i64> {
        if target != 0 && self.node(target).is_some() {
            self.view_tracks_of(&View::Playlist(target))
        } else {
            self.tracks.iter().map(|t| t.id).collect()
        }
    }

    /// Track Suggestion for `src` among `target`, best first, with the score shown as Rank.
    pub fn suggestions(&self, src: i64, target: i64, kind: u8) -> Vec<(i64, f64)> {
        let Some(s) = self.track(src) else { return vec![] };
        let f_src = self.previews.get(&src).and_then(|p| mood_features(p));
        let tags_src = self.track_tags.get(&src);
        let scored: Vec<(i64, [f64; 4])> = self
            .target_tracks(target)
            .into_iter()
            .filter(|id| *id != src)
            .filter_map(|id| self.track(id))
            .filter(|t| !t.missing)
            .map(|t| {
                let f_t = self.previews.get(&t.id).and_then(|p| mood_features(p));
                (t.id, suggestion_scores(s, t, f_src, f_t, tags_src, self.track_tags.get(&t.id), self.is_match(src, t.id)))
            })
            .collect();
        rank_suggestions(scored, kind)
    }

    /// Related tracks of `src` by the selected criteria, limited to a search target list.
    pub fn related_for(&self, src: i64, target: i64) -> Vec<i64> {
        let all = self.related_tracks_of(src);
        if target != 0 && self.node(target).is_some() {
            let keep: HashSet<i64> = self.target_tracks(target).into_iter().collect();
            all.into_iter().filter(|t| keep.contains(t)).collect()
        } else {
            all
        }
    }
}

// ------------------------------------------------------------------------------ column view

/// Column View: the chain of folders from the top level down to the shown node; a playlist
/// ends the chain, a folder is part of it (its children fill the next column).
pub fn column_path(nodes: &[PlaylistNode], shown: Option<i64>) -> Vec<i64> {
    let mut path = vec![];
    let mut cur = shown.unwrap_or(0);
    let mut guard = 0;
    while cur != 0 && guard < 64 {
        let Some(n) = nodes.iter().find(|n| n.id == cur) else { return vec![] };
        path.push(n.id);
        cur = n.parent;
        guard += 1;
    }
    path.reverse();
    path
}

/// The columns to draw: (parent, selected child) from left to right. The first column lists
/// the top level; each selected folder adds a column for its children.
pub fn columns_for(nodes: &[PlaylistNode], shown: Option<i64>) -> Vec<(i64, Option<i64>)> {
    let path = column_path(nodes, shown);
    let mut cols = vec![(0, path.first().copied())];
    for (i, id) in path.iter().enumerate() {
        if nodes.iter().any(|n| n.id == *id && n.kind == NodeKind::Folder) {
            cols.push((*id, path.get(i + 1).copied()));
        }
    }
    cols
}

// ------------------------------------------------------------------------------ CUE analysis

/// Personal CUE tendency: where the user's cues sit, in whole bars from the first downbeat.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CueTendency {
    /// Tracks the tendency was learnt from.
    pub tracks: usize,
    /// Hot cue slot (0 = A) and its usual bar (0 = the first downbeat).
    pub hot: Vec<(i32, i32)>,
    /// Usual bars of memory cues.
    pub memory: Vec<i32>,
}

impl CueTendency {
    pub fn is_empty(&self) -> bool {
        self.hot.is_empty() && self.memory.is_empty()
    }
}

/// First downbeat and bar length (seconds) of a grid.
fn bar_frame(grid: &BeatGrid, duration: f64) -> Option<(f64, f64)> {
    let beats = grid.beats(duration.max(1.0));
    let first = beats.iter().find(|b| b.beat == 1)?;
    if first.bpm <= 0.0 {
        return None;
    }
    Some((first.time, 240.0 / first.bpm))
}

/// A cue's bar counted from the first downbeat, if it sits on a bar line (within an eighth
/// of a bar).
pub fn cue_bar(time: f64, first_downbeat: f64, bar_len: f64) -> Option<i32> {
    if bar_len <= 0.0 {
        return None;
    }
    let bars = (time - first_downbeat) / bar_len;
    let r = bars.round();
    if (bars - r).abs() <= 0.125 && r >= 0.0 {
        Some(r as i32)
    } else {
        None
    }
}

/// Learns the tendency from tracks given as (grid, duration, cues): a hot cue slot's usual bar
/// is the most frequent one, kept when at least half of the tracks using that slot agree; a
/// memory cue bar is kept when at least half of the tracks have one there.
pub fn learn_tendency(tracks: &[(BeatGrid, f64, Vec<Cue>)]) -> CueTendency {
    let mut hot: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut mem: HashMap<i32, usize> = HashMap::new();
    let mut used = 0;
    for (grid, dur, cues) in tracks {
        let Some((first, bar)) = bar_frame(grid, *dur) else { continue };
        let mut any = false;
        let mut seen_mem = HashSet::new();
        for c in cues {
            let Some(b) = cue_bar(c.start, first, bar) else { continue };
            any = true;
            match c.kind {
                CueKind::Hot => hot.entry(c.slot).or_default().push(b),
                _ => {
                    if seen_mem.insert(b) {
                        *mem.entry(b).or_default() += 1;
                    }
                }
            }
        }
        if any {
            used += 1;
        }
    }
    let mut out = CueTendency { tracks: used, ..Default::default() };
    for (slot, bars) in hot {
        let mut freq: HashMap<i32, usize> = HashMap::new();
        for b in &bars {
            *freq.entry(*b).or_default() += 1;
        }
        let mut best: Vec<(i32, usize)> = freq.into_iter().collect();
        best.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        if let Some((bar, n)) = best.first() {
            if *n * 2 >= bars.len() {
                out.hot.push((slot, *bar));
            }
        }
    }
    out.hot.sort();
    out.memory = mem.into_iter().filter(|(_, n)| *n * 2 >= used.max(1)).map(|(b, _)| b).collect();
    out.memory.sort();
    out
}

/// Cues the tendency gives a track: each on its bar line, as long as it lies inside the track.
pub fn tendency_cues(t: &CueTendency, track: i64, grid: &BeatGrid, duration: f64) -> Vec<Cue> {
    let Some((first, bar)) = bar_frame(grid, duration) else { return vec![] };
    let mut out = vec![];
    for (slot, b) in &t.hot {
        let time = first + *b as f64 * bar;
        if time < duration - 1.0 && (0..8).contains(slot) {
            out.push(Cue { id: 0, track_id: track, kind: CueKind::Hot, slot: *slot, start: time, end: None, color: 0, comment: format!("{}.1Bars", b + 1), active_loop: false });
        }
    }
    for b in &t.memory {
        let time = first + *b as f64 * bar;
        if time < duration - 1.0 {
            out.push(Cue { id: 0, track_id: track, kind: CueKind::Memory, slot: -1, start: time, end: None, color: 0, comment: String::new(), active_loop: false });
        }
    }
    out
}

impl App {
    /// The CUE Analysis Playlist (the first playlist of that name), if there is one.
    pub fn cue_playlist(&self) -> Option<i64> {
        self.nodes.iter().find(|n| n.kind == NodeKind::Playlist && n.name == CUE_PLAYLIST_NAME).map(|n| n.id)
    }

    /// [Start] of the CUE Analysis Playlist: analyse the playlist's tracks that still need it,
    /// then learn the personal CUE tendency from their cues. With no tracks registered the
    /// tendency is reset.
    pub fn cue_analysis_start(&mut self) {
        let Some(pl) = self.cue_playlist() else { return };
        let ids: Vec<i64> = self.db.playlist_tracks(pl).unwrap_or_default().into_iter().filter(|t| self.index.contains_key(t)).collect();
        if ids.is_empty() {
            self.prefs.bx.cue_tendency = Default::default();
            self.save_prefs();
            self.toast("Your personal CUE Tendency Information has been reset.");
            return;
        }
        let pending: Vec<i64> = ids.iter().copied().filter(|id| self.track(*id).map(|t| t.analyzed != 1 && !t.missing).unwrap_or(false)).collect();
        if !pending.is_empty() {
            self.analyze(&pending, false);
            self.ext.cue_wait = true;
            self.toast("Track Analysis may take a moment.");
            return;
        }
        self.cue_analysis_finish(&ids);
    }

    fn cue_analysis_finish(&mut self, ids: &[i64]) {
        let data: Vec<(BeatGrid, f64, Vec<Cue>)> = ids
            .iter()
            .filter_map(|id| {
                let g = self.db.grid(*id)?;
                let dur = self.track(*id)?.duration;
                Some((g, dur, self.db.cues(*id).unwrap_or_default()))
            })
            .collect();
        let t = learn_tendency(&data);
        if t.is_empty() {
            self.toast("Add tracks to the playlist or wait until the track analysis is complete.");
        } else {
            self.toast(format!("CUE Analysis: personal CUE Tendency Information updated from {} track(s).", t.tracks));
        }
        self.prefs.bx.cue_tendency = t;
        self.save_prefs();
    }

    /// Finishes a [Start] that had to wait for track analysis.
    pub fn poll_cue_analysis(&mut self) {
        if !self.ext.cue_wait {
            return;
        }
        let Some(pl) = self.cue_playlist() else {
            self.ext.cue_wait = false;
            return;
        };
        let ids: Vec<i64> = self.db.playlist_tracks(pl).unwrap_or_default().into_iter().filter(|t| self.index.contains_key(t)).collect();
        if ids.iter().any(|id| self.track(*id).map(|t| t.analyzed == 3 || (t.analyzed == 0 && !t.missing)).unwrap_or(false)) {
            return;
        }
        self.ext.cue_wait = false;
        self.cue_analysis_finish(&ids);
    }

    /// A freshly analysed track without cues gets the cues of the personal tendency.
    pub fn apply_cue_tendency(&self, id: i64, grid: &BeatGrid, cleared_hot_slots: &HashSet<i32>) -> bool {
        if self.prefs.bx.cue_tendency.is_empty() || self.db.cues(id).map(|c| !c.is_empty()).unwrap_or(true) {
            return false;
        }
        let dur = self.track(id).map(|t| t.duration).unwrap_or(0.0);
        let mut added = false;
        for cue in tendency_cues(&self.prefs.bx.cue_tendency, id, grid, dur) {
            if cue.kind != CueKind::Hot || !cleared_hot_slots.contains(&cue.slot) {
                added |= self.db.save_cue(&cue).is_ok();
            }
        }
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GridAnchor;

    fn tr(id: i64, bpm: f64, key: i32, year: i32, artist: &str, label: &str, genre: &str) -> Track {
        Track { id, bpm, key, year, artist: artist.into(), label: label.into(), genre: genre.into(), ..Default::default() }
    }

    #[test]
    fn era_association_and_mix() {
        let a = tr(1, 128.0, 0, 2020, "Max Styler", "Nervous", "Tech House");
        let same_year = tr(2, 128.0, 0, 2020, "X", "", "");
        let five = tr(3, 128.0, 0, 2015, "X", "", "");
        let far = tr(4, 128.0, 0, 1995, "X", "", "");
        let unknown = tr(5, 128.0, 0, 0, "X", "", "");
        assert_eq!(era_score(&a, &same_year), Some(1.0));
        assert_eq!(era_score(&a, &five), Some(0.5));
        assert_eq!(era_score(&a, &far), Some(0.0));
        assert_eq!(era_score(&a, &unknown), None);
        // Shared artist beats shared label beats nothing; "feat." names count.
        let art = tr(6, 100.0, 5, 0, "Dale Howard, Max Styler", "", "");
        let lab = tr(7, 100.0, 5, 0, "Someone", "nervous", "");
        let none = tr(8, 100.0, 5, 0, "Someone", "Other", "House");
        let mut feat = tr(9, 100.0, 5, 0, "Someone", "", "");
        feat.title = "Night (feat. Max Styler)".into();
        assert!(association_score(&a, &art) > association_score(&a, &lab));
        assert!(association_score(&a, &lab) > 0.0);
        assert_eq!(association_score(&a, &none), 0.0);
        assert!(association_score(&a, &feat) > 0.0);
        // Mixing: same tempo and key is best; half tempo counts as the same tempo.
        let half = tr(10, 64.0, 0, 0, "", "", "");
        let off = tr(11, 140.0, 6, 0, "", "", "");
        assert_eq!(mix_score(&a, &same_year), Some(1.0));
        assert_eq!(mix_score(&a, &half), Some(1.0));
        assert!(mix_score(&a, &off).unwrap() < 0.2);
        assert_eq!(mix_score(&a, &tr(12, 0.0, -1, 0, "", "", "")), None);
    }

    #[test]
    fn mood_and_ranking() {
        let col = |low: u8, mid: u8, high: u8| -> Vec<u8> { (0..400).flat_map(|_| [200, low, mid, high, 100, low, mid, high]).collect() };
        let bassy = mood_features(&col(120, 40, 10)).unwrap();
        let bassy2 = mood_features(&col(115, 42, 12)).unwrap();
        let bright = mood_features(&col(20, 60, 110)).unwrap();
        assert!(mood_features(&[]).is_none());
        let a = tr(1, 128.0, 12, 0, "", "", "");
        let b = tr(2, 128.0, 14, 0, "", "", "");
        let close = mood_score(&a, &b, Some(bassy), Some(bassy2), None, None).unwrap();
        let apart = mood_score(&a, &b, Some(bassy), Some(bright), None, None).unwrap();
        assert!(close > 0.9 && apart < close - 0.3, "{close} {apart}");
        // Ranking: best first; Era leaves out tracks that score nothing; a registered match
        // always leads the Rank list.
        let src = tr(1, 128.0, 0, 2020, "A", "L", "House");
        let good = tr(2, 128.0, 0, 2020, "A", "L", "House");
        let ok = tr(3, 126.0, 7, 2012, "B", "", "House");
        let poor = tr(4, 90.0, 6, 1990, "C", "", "Rock");
        let sc = |t: &Track, m: bool| (t.id, suggestion_scores(&src, t, None, None, None, None, m));
        let rank = rank_suggestions(vec![sc(&poor, false), sc(&ok, false), sc(&good, false)], 0);
        assert_eq!(rank.iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 3, 4]);
        assert!(rank[0].1 > 0.95 && rank[2].1 < 0.2);
        let era = rank_suggestions(vec![sc(&poor, false), sc(&ok, false), sc(&good, false)], 1);
        assert_eq!(era.iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 3]);
        let assoc = rank_suggestions(vec![sc(&poor, false), sc(&ok, false), sc(&good, false)], 3);
        assert_eq!(assoc.iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 3]);
        let matched = rank_suggestions(vec![sc(&good, false), sc(&poor, true)], 0);
        assert_eq!(matched[0].0, 4);
    }

    fn node(id: i64, parent: i64, kind: NodeKind) -> PlaylistNode {
        PlaylistNode { id, parent, name: format!("n{id}"), kind, seq: id, smart: None }
    }

    #[test]
    fn column_view_model() {
        // 1 folder { 2 folder { 3 playlist }, 4 playlist }, 5 playlist
        let nodes = vec![node(1, 0, NodeKind::Folder), node(2, 1, NodeKind::Folder), node(3, 2, NodeKind::Playlist), node(4, 1, NodeKind::Playlist), node(5, 0, NodeKind::Playlist)];
        assert_eq!(columns_for(&nodes, None), vec![(0, None)]);
        assert_eq!(columns_for(&nodes, Some(5)), vec![(0, Some(5))]);
        assert_eq!(columns_for(&nodes, Some(1)), vec![(0, Some(1)), (1, None)]);
        assert_eq!(columns_for(&nodes, Some(4)), vec![(0, Some(1)), (1, Some(4))]);
        assert_eq!(columns_for(&nodes, Some(3)), vec![(0, Some(1)), (1, Some(2)), (2, Some(3))]);
        assert_eq!(column_path(&nodes, Some(3)), vec![1, 2, 3]);
        // A node that no longer exists gives the top level only.
        assert_eq!(columns_for(&nodes, Some(99)), vec![(0, None)]);
    }

    #[test]
    fn cue_tendency() {
        let grid = |first: f64, bpm: f64| BeatGrid::from_anchors(vec![GridAnchor { time: first, bpm, beat: 1 }]);
        let cue = |kind: CueKind, slot: i32, start: f64| Cue { id: 0, track_id: 0, kind, slot, start, end: None, color: 0, comment: String::new(), active_loop: false };
        // 120 BPM: a bar is 2 s. Hot A on the first downbeat, B at bar 16; memory cue at bar 32.
        let t1 = (grid(0.5, 120.0), 300.0, vec![cue(CueKind::Hot, 0, 0.5), cue(CueKind::Hot, 1, 32.5), cue(CueKind::Memory, -1, 64.5)]);
        let t2 = (grid(1.0, 120.0), 300.0, vec![cue(CueKind::Hot, 0, 1.0), cue(CueKind::Hot, 1, 33.0), cue(CueKind::Memory, -1, 65.0), cue(CueKind::Memory, -1, 9.0)]);
        // Off the bar line: ignored. A different bar for B: outvoted.
        let t3 = (grid(0.0, 120.0), 300.0, vec![cue(CueKind::Hot, 0, 0.0), cue(CueKind::Hot, 1, 16.0), cue(CueKind::Hot, 2, 7.1)]);
        assert_eq!(cue_bar(32.5, 0.5, 2.0), Some(16));
        assert_eq!(cue_bar(7.1, 0.0, 2.0), None);
        let t = learn_tendency(&[t1, t2, t3]);
        assert_eq!(t.tracks, 3);
        assert_eq!(t.hot, vec![(0, 0), (1, 16)]);
        assert_eq!(t.memory, vec![32]);
        // Applied to a 128 BPM track starting at 0.25 s: bar = 1.875 s.
        let cues = tendency_cues(&t, 7, &grid(0.25, 128.0), 200.0);
        assert_eq!(cues.len(), 3);
        assert!((cues[0].start - 0.25).abs() < 1e-9 && cues[0].slot == 0 && cues[0].comment == "1.1Bars");
        assert!((cues[1].start - (0.25 + 16.0 * 1.875)).abs() < 1e-9 && cues[1].slot == 1);
        assert!((cues[2].start - (0.25 + 32.0 * 1.875)).abs() < 1e-9 && cues[2].kind == CueKind::Memory);
        // A short track only gets the cues that fit; an empty tendency gives none.
        assert_eq!(tendency_cues(&t, 7, &grid(0.25, 128.0), 40.0).len(), 2);
        assert!(tendency_cues(&CueTendency::default(), 7, &grid(0.0, 120.0), 200.0).is_empty());
        assert!(learn_tendency(&[]).is_empty());
    }
}
