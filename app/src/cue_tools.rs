//! The track menu's cue tools and track type, as traced from rekordbox 7.2.19
//! (`checks/re-7.2.19/shell-menus-traffic.md`, section 5):
//!
//! - Convert Memory Cues to Hot Cues (`ConvertMemoryCueThread::run` 0x1023c4ea0): a track's
//!   memory cues and memory loops, in time order, become hot cues in the lowest free slots of
//!   A..P. One that sits exactly where a hot cue already is stays a memory cue.
//! - Sort Hot Cues by time (`ListViewer::popupSortHotCueByTime` 0x1004035a0), after rekordbox's
//!   "cannot be undone" question. Traced (`HotCueTimeSortService` thread 0x10010a9e8 ->
//!   `DatabaseIF::sortCues` -> `CueManager::SortCues` 0x1000587d8 ->
//!   `TimedBaseSortStrategy::sort` 0x100878718): the hot cues are stable-sorted by time and
//!   the i-th one goes to the i-th slot (A, B, C, ... from rekordbox's kind table), so gaps
//!   between used slots are closed up; a track whose cues already sit that way is "already
//!   sorted" (`HotCueVisitor::isApplicatedTimedBaseSort` 0x100878ae8).
//! - Track Type (Oneshot / Loop / Normal): the track's sampler play mode, kept per track and
//!   shared with the sampler list's Track Type (`sampler_lib.rs`); a track loaded into a
//!   sampler slot loops or plays once by it.

use crate::app::*;
use crate::model::*;
use crate::ui_kit as kit;
use eframe::egui;
use std::collections::HashMap;

/// Hot cue slots a track has in rekordbox (A..P).
pub const SLOTS: i32 = 16;

/// Hot cue colour code for a memory cue colour (rekordbox's codes 0..7 are recordbox's 1..8).
pub fn hot_colour(memory_colour: u8) -> u8 {
    match memory_colour {
        1..=8 => [49, 42, 38, 32, 22, 9, 1, 56][memory_colour as usize - 1],
        _ => 0,
    }
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub struct Converted {
    /// Memory cues the track had.
    pub memory: usize,
    /// Left alone because a hot cue is at the same place.
    pub same_place: usize,
    pub converted: usize,
    /// Some went to slots I..P (EXPORT shows A..H only).
    pub past_h: bool,
}

/// Convert a track's memory cues into hot cues (see the module notes).
pub fn convert(cues: &mut [Cue]) -> Converted {
    let mut free = [true; SLOTS as usize];
    let mut any_active = false;
    for c in cues.iter().filter(|c| c.kind == CueKind::Hot) {
        if (0..SLOTS).contains(&c.slot) {
            free[c.slot as usize] = false;
        }
        any_active |= c.active_loop;
    }
    let hot: Vec<(f64, Option<f64>)> = cues.iter().filter(|c| c.kind == CueKind::Hot).map(|c| (c.start, c.end)).collect();
    let mut order: Vec<usize> = (0..cues.len()).filter(|i| cues[*i].kind == CueKind::Memory).collect();
    order.sort_by(|a, b| cues[*a].start.partial_cmp(&cues[*b].start).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Converted { memory: order.len(), ..Default::default() };
    for i in order {
        if hot.iter().any(|(s, e)| *s == cues[i].start && *e == cues[i].end) {
            out.same_place += 1;
            continue;
        }
        let Some(slot) = free.iter().position(|f| *f) else { continue };
        free[slot] = false;
        let c = &mut cues[i];
        if c.is_loop() && !any_active {
            c.active_loop = true;
            any_active = true;
        }
        c.kind = CueKind::Hot;
        c.slot = slot as i32;
        c.color = hot_colour(c.color);
        out.converted += 1;
        out.past_h |= slot >= 8;
    }
    out
}

/// Put the hot cues into A, B, C, ... in time order (gaps close up). False when nothing
/// changed.
pub fn sort_hot(cues: &mut [Cue]) -> bool {
    let mut idx: Vec<usize> = (0..cues.len()).filter(|i| cues[*i].kind == CueKind::Hot).collect();
    idx.sort_by(|a, b| cues[*a].start.partial_cmp(&cues[*b].start).unwrap_or(std::cmp::Ordering::Equal).then(cues[*a].slot.cmp(&cues[*b].slot)));
    let mut changed = false;
    for (i, slot) in idx.into_iter().zip(0..SLOTS) {
        changed |= cues[i].slot != slot;
        cues[i].slot = slot;
    }
    changed
}

impl CueTools {
    pub fn cached_type(&self, id: i64) -> Option<u8> {
        self.types.as_ref().and_then(|m| m.get(&id).copied()).filter(|t| *t != 0)
    }
    pub fn has_type(&self, id: i64) -> bool {
        self.cached_type(id).is_some()
    }
}

#[derive(Default)]
pub struct CueTools {
    /// Tracks waiting for the answer to "Sorting the Hot Cues cannot be undone...".
    pub sort_ask: Option<Vec<i64>>,
    /// A result to show: (title, text).
    pub message: Option<(String, String)>,
    /// Sampler play mode per track: 1 Oneshot, 2 Loop (absent: Normal). Loaded on first use.
    types: Option<HashMap<i64, u8>>,
}

impl App {
    /// Store a track's cues and bring every place that shows them up to date.
    fn cue_tools_apply(&mut self, track: i64, cues: &[Cue]) {
        let _ = self.db.replace_cues(track, cues);
        let now = self.db.cues(track).unwrap_or_default();
        if self.deck.track == Some(track) {
            self.deck.cues = now.clone();
        }
        for d in self.px.parked.iter_mut() {
            if d.track == Some(track) {
                d.cues = now.clone();
            }
        }
        let hp: Vec<(f64, i32, u8)> = now.iter().filter(|c| c.kind == CueKind::Hot).map(|c| (c.start, c.slot, c.color)).collect();
        if hp.is_empty() {
            self.hot_positions.remove(&track);
        } else {
            self.hot_positions.insert(track, hp);
        }
        let h = now.iter().filter(|c| c.kind == CueKind::Hot).count();
        let m = now.len() - h;
        if h + m == 0 {
            self.cue_counts.remove(&track);
        } else {
            self.cue_counts.insert(track, (h, m));
        }
    }

    /// Convert Memory Cues to Hot Cues for the selected tracks, with rekordbox's messages.
    pub fn convert_memory_cues(&mut self, sel: &[i64]) {
        let (mut memory, mut done, mut same) = (0, 0, 0);
        let mut short: Vec<String> = Vec::new();
        for id in sel {
            let mut cues = self.db.cues(*id).unwrap_or_default();
            let r = convert(&mut cues);
            memory += r.memory;
            done += r.converted;
            same += r.same_place;
            if r.converted > 0 {
                self.cue_tools_apply(*id, &cues);
            }
            if r.converted + r.same_place < r.memory {
                short.push(self.track(*id).map(|t| t.display_title()).unwrap_or_default());
            }
        }
        let text = if memory == 0 {
            "There are no Memory Cues set for any of the selected tracks.".to_string()
        } else if done + same == memory {
            "The Memory Cues have been converted into Hot Cues.".to_string()
        } else if done == 0 {
            "There are no Hot Cues left that can be set in all selected tracks.".to_string()
        } else {
            format!("There are tracks for which Memory Cues were not converted to Hot Cues because there are no Hot Cues left that can be set.\n*8 Hot Cues can be set  to A to H in EXPORT mode, and 16 can be set  to A to P in other modes.\n{}", short.join("\n"))
        };
        self.cue_tools.message = Some(("MEMORY CUE to HOT CUE".into(), text));
    }

    fn sort_hot_cues_now(&mut self, sel: &[i64]) {
        let mut sorted = 0;
        for id in sel {
            let mut cues = self.db.cues(*id).unwrap_or_default();
            if sort_hot(&mut cues) {
                self.cue_tools_apply(*id, &cues);
                sorted += 1;
            }
        }
        let text = if sorted == 0 {
            "Could not sort Hot Cues by time because they are already sorted or not set."
        } else if sorted == sel.len() {
            "Sorted the Hot Cues of the selected tracks by time."
        } else {
            "Sorted the Hot Cues by time for one or more tracks."
        };
        self.cue_tools.message = Some(("Sort Hot Cues by time".into(), text.into()));
    }

    fn track_types(&mut self) -> &mut HashMap<i64, u8> {
        if self.cue_tools.types.is_none() {
            let m: HashMap<i64, u8> = self.db.setting("track_types").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            self.cue_tools.types = Some(m);
        }
        self.cue_tools.types.as_mut().unwrap()
    }

    /// A track's sampler play mode: 0 Normal, 1 Oneshot, 2 Loop. One setting with the sampler
    /// list's Track Type: the sample row made from the track wins when there is one
    /// (`sampler_lib.rs`), else the value kept here.
    pub fn track_type(&mut self, id: i64) -> u8 {
        match self.smplib_track_type(id) {
            Some(t) => t,
            None => self.cue_tools_type_only(id),
        }
    }

    /// Read the kept Track Types once (for drawing, which then reads `cue_tools.cached_type`).
    pub fn track_types_ready(&mut self) {
        let _ = self.track_types();
    }

    /// The value kept for tracks that have no sample row.
    pub fn cue_tools_type_only(&mut self, id: i64) -> u8 {
        self.track_types().get(&id).copied().unwrap_or(0)
    }

    pub fn set_track_type(&mut self, ids: &[i64], mode: u8) {
        self.smplib_set_track_type(ids, mode);
        let m = self.track_types();
        for id in ids {
            if mode == 0 {
                m.remove(id);
            } else {
                m.insert(*id, mode);
            }
        }
        let s = serde_json::to_string(&*m).unwrap_or_default();
        self.db.set_setting("track_types", &s);
    }
}

/// The question and the result boxes. Called every frame.
pub fn windows(app: &mut App, ctx: &egui::Context) {
    let center = egui::Align2::CENTER_CENTER;
    if let Some(sel) = app.cue_tools.sort_ask.clone() {
        let mut answer = None;
        kit::dialog(ctx, "Sort Hot Cues by time").id(egui::Id::new("sort_hot_ask")).resizable(false).anchor(center, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(400.0);
            kit::message(ui, "Sorting the Hot Cues cannot be undone. Are you sure you want to continue?");
            answer = kit::dialog_buttons(ui, &["OK", "Cancel"]);
        });
        if let Some(a) = answer {
            app.cue_tools.sort_ask = None;
            if a == 0 {
                app.sort_hot_cues_now(&sel);
            }
        }
    }
    if let Some((title, text)) = app.cue_tools.message.clone() {
        let mut close = false;
        kit::dialog(ctx, title).id(egui::Id::new("cue_tools_msg")).resizable(false).anchor(center, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(420.0);
            kit::message(ui, &text);
            close = kit::dialog_buttons(ui, &["OK"]).is_some();
        });
        if close {
            app.cue_tools.message = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(kind: CueKind, slot: i32, start: f64, end: Option<f64>, color: u8) -> Cue {
        Cue { id: 0, track_id: 1, kind, slot, start, end, color, comment: String::new(), active_loop: false }
    }

    #[test]
    fn memory_cues_take_the_lowest_free_slots_in_time_order() {
        let mut c = vec![
            cue(CueKind::Hot, 0, 1.0, None, 5),
            cue(CueKind::Hot, 2, 9.0, None, 5),
            cue(CueKind::Memory, -1, 30.0, None, 2),
            cue(CueKind::Memory, -1, 10.0, Some(12.0), 0),
            cue(CueKind::Memory, -1, 1.0, None, 3), // where hot cue A is: stays
        ];
        let r = convert(&mut c);
        assert_eq!(r, Converted { memory: 3, same_place: 1, converted: 2, past_h: false });
        // The earlier one (the loop) gets B, the later one D; the loop becomes the active loop.
        assert_eq!((c[3].kind, c[3].slot, c[3].active_loop, c[3].color), (CueKind::Hot, 1, true, 0));
        assert_eq!((c[2].kind, c[2].slot, c[2].color), (CueKind::Hot, 3, 42));
        assert_eq!(c[4].kind, CueKind::Memory);
    }

    #[test]
    fn conversion_stops_when_sixteen_slots_are_used() {
        let mut c: Vec<Cue> = (0..15).map(|s| cue(CueKind::Hot, s, s as f64, None, 0)).collect();
        c.push(cue(CueKind::Memory, -1, 100.0, None, 1));
        c.push(cue(CueKind::Memory, -1, 200.0, None, 1));
        let r = convert(&mut c);
        assert_eq!((r.converted, r.past_h), (1, true));
        assert_eq!((c[15].slot, c[15].color), (15, 49));
        assert_eq!(c[16].kind, CueKind::Memory);
    }

    #[test]
    fn sorting_closes_the_gaps_as_rekordbox_does() {
        let mut c = vec![cue(CueKind::Hot, 0, 50.0, None, 0), cue(CueKind::Hot, 4, 10.0, None, 0), cue(CueKind::Hot, 2, 30.0, None, 0), cue(CueKind::Memory, -1, 1.0, None, 0)];
        assert!(sort_hot(&mut c));
        // A, C, E by time become A, B, C; the memory cue is not touched.
        assert_eq!((c[1].slot, c[2].slot, c[0].slot, c[3].slot), (0, 1, 2, -1));
        // In time order already but with a gap: the gap closes, so it is not "already sorted".
        let mut g = vec![cue(CueKind::Hot, 0, 1.0, None, 0), cue(CueKind::Hot, 3, 2.0, None, 0)];
        assert!(sort_hot(&mut g));
        assert_eq!((g[0].slot, g[1].slot), (0, 1));
        assert!(!sort_hot(&mut c));
        assert!(!sort_hot(&mut []));
    }
}
