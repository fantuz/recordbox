//! Browser additions matched to rekordbox 7.2.19: Track Suggestion (ranking from the local
//! library), the playlist Column View model, the CUE Analysis Playlist (personal CUE
//! tendency) and the settings of the right-hand panels. The drawing lives in
//! ui_browser_ext.rs.

use crate::app::{App, View};
mod matching;
use crate::model::{BeatGrid, Cue, CueKind, NodeKind, PlaylistNode};
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
    "Track Suggestion:\nRanks tracks with evidence of a match. Registered pairs lead, followed by tempo, harmonic key, waveform, shared credits, tags and era.",
    "Era:\nTracks with a close year of release.",
    "Mood:\nTracks with a similar mood.",
    "Association:\nTracks sharing traits such as artist, label etc.",
];

// ------------------------------------------------------------------------------ track suggestion

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
