//! Library management beyond the basic browser: in-place editing, the Track Filter, play
//! histories, the Tag List, Matching, playlist files, the palette and shortcuts, missing files,
//! track clipboard and the iTunes / rekordbox xml sources. State lives in `App::ext`.

use crate::app::*;
use crate::db::Db;
use crate::itunes::{SourceLib, SrcNode};
use crate::model::*;
use crate::playlist_files as plf;
use anyhow::Result;
use crossbeam_channel::Receiver;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Seconds a track must play before it enters the day's history and its DJ Play Count goes
/// up (rekordbox records a track once it has played for about a minute).
pub const PLAY_CREDIT_SECS: f64 = 60.0;
/// "Up to 100 tracks can be saved in the Tag List."
pub const TAG_LIST_MAX: usize = 100;
pub const PALETTE_SLOTS: usize = 4;
pub const SHORTCUT_MAX: usize = 8;

/// A saved "Related Tracks" list: a name and its criteria.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RelatedList {
    pub name: String,
    pub rules: RelatedRules,
}

/// Track Filter bar: each part has its own on/off tick.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackFilter {
    pub bpm_on: bool,
    pub bpm: f64,
    /// ± range in percent (0 = that BPM, to the nearest whole number).
    pub bpm_pct: f64,
    pub key_on: bool,
    pub key: i32,
    /// Also show keys next to it on the key wheel and its relative major/minor.
    pub key_related: bool,
    pub rating_on: bool,
    pub rating: u8,
    pub color_on: bool,
    /// Bit n set: colour n (1–8) passes.
    pub colors: u16,
    pub tags_on: bool,
}

impl Default for TrackFilter {
    fn default() -> Self {
        TrackFilter { bpm_on: false, bpm: 0.0, bpm_pct: 3.0, key_on: false, key: -1, key_related: false, rating_on: false, rating: 0, color_on: false, colors: 0, tags_on: true }
    }
}

/// BPM limits the filter lets through.
pub fn bpm_window(bpm: f64, pct: f64) -> (f64, f64) {
    let tol = if pct <= 0.0 { 0.5 } else { bpm * pct / 100.0 };
    (bpm - tol, bpm + tol)
}

pub fn filter_pass(f: &TrackFilter, t: &Track) -> bool {
    if f.bpm_on && f.bpm > 0.0 {
        let (lo, hi) = bpm_window(f.bpm, f.bpm_pct);
        if t.bpm <= 0.0 || t.bpm < lo - 1e-9 || t.bpm > hi + 1e-9 {
            return false;
        }
    }
    if f.key_on && f.key >= 0 {
        let ok = if f.key_related { compatible_key(f.key, t.key) } else { t.key == f.key };
        if !ok {
            return false;
        }
    }
    if f.rating_on && t.rating < f.rating {
        return false;
    }
    if f.color_on && f.colors != 0 && (f.colors >> t.color) & 1 == 0 {
        return false;
    }
    true
}

/// Columns whose cells can be edited in place.
pub fn editable(c: Col) -> bool {
    matches!(
        c,
        Col::Title | Col::Artist | Col::Album | Col::Genre | Col::Comments | Col::Bpm | Col::Key | Col::Label | Col::Remixer | Col::OriginalArtist | Col::Composer | Col::Year | Col::MixName | Col::TrackNumber | Col::DiscNumber | Col::AlbumArtist | Col::Grouping | Col::Lyricist | Col::Message | Col::ReleaseDate
    )
}

/// The text an editor starts with for a cell (the stored value, not its display form).
pub fn edit_text(t: &Track, c: Col, alnum: bool) -> String {
    let num = |n: i32| if n > 0 { n.to_string() } else { String::new() };
    match c {
        Col::Title => t.title.clone(),
        Col::Artist => t.artist.clone(),
        Col::Album => t.album.clone(),
        Col::Genre => t.genre.clone(),
        Col::Comments => t.comment.clone(),
        Col::Bpm => if t.bpm > 0.0 { format!("{:.2}", t.bpm) } else { String::new() },
        Col::Key => t.key_label(alnum),
        Col::Label => t.label.clone(),
        Col::Remixer => t.remixer.clone(),
        Col::OriginalArtist => t.original_artist.clone(),
        Col::Composer => t.composer.clone(),
        Col::Year => num(t.year),
        Col::MixName => t.mix_name.clone(),
        Col::TrackNumber => num(t.track_no),
        Col::DiscNumber => num(t.disc_no),
        Col::AlbumArtist => t.album_artist.clone(),
        Col::Grouping => t.grouping.clone(),
        Col::Lyricist => t.lyricist.clone(),
        Col::Message => t.message.clone(),
        Col::ReleaseDate => t.release_date.clone(),
        _ => String::new(),
    }
}

/// Put edited text into a track's field. Err: the text is not valid for that field.
pub fn apply_cell_edit(t: &mut Track, c: Col, text: &str) -> std::result::Result<(), String> {
    let s = text.trim().to_string();
    let number = |what: &str, max: i32| -> std::result::Result<i32, String> {
        if s.is_empty() {
            return Ok(0);
        }
        s.parse::<i32>().ok().filter(|n| (0..=max).contains(n)).ok_or_else(|| format!("{what} must be a number between 0 and {max}."))
    };
    match c {
        Col::Title => t.title = s,
        Col::Artist => t.artist = s,
        Col::Album => t.album = s,
        Col::Genre => t.genre = s,
        Col::Comments => t.comment = s,
        Col::Label => t.label = s,
        Col::Remixer => t.remixer = s,
        Col::OriginalArtist => t.original_artist = s,
        Col::Composer => t.composer = s,
        Col::MixName => t.mix_name = s,
        Col::AlbumArtist => t.album_artist = s,
        Col::Grouping => t.grouping = s,
        Col::Lyricist => t.lyricist = s,
        Col::Message => t.message = s,
        Col::ReleaseDate => t.release_date = s,
        Col::Year => t.year = number("Year", 9999)?,
        Col::TrackNumber => t.track_no = number("Track number", 9999)?,
        Col::DiscNumber => t.disc_no = number("Disc number", 999)?,
        Col::Bpm => {
            let v = s.replace(',', ".").parse::<f64>().ok().filter(|v| (40.0..=499.0).contains(v)).ok_or("BPM value should be between 40 and 499.")?;
            t.bpm = (v * 100.0).round() / 100.0;
        }
        Col::Key => {
            if s.is_empty() {
                t.key = -1;
                t.key_text.clear();
                t.key_manual = false;
            } else {
                let k = parse_key(&s);
                if k < 0 {
                    return Err(format!("\"{s}\" is not a key (write it like 8A or Am)."));
                }
                t.set_key(k);
                t.key_manual = true;
            }
        }
        _ => return Err("This field cannot be edited.".into()),
    }
    Ok(())
}

/// When a loaded track has played long enough to count as played.
#[derive(Default, Debug)]
pub struct PlayCredit {
    pub track: Option<i64>,
    pub secs: f64,
    pub done: bool,
    /// Preferences > Advanced > Browse > Played time (seconds; 0 = `PLAY_CREDIT_SECS`). As in
    /// rekordbox the time must be played in one go: a pause starts it again
    /// (`UiPlayer::eventPlaySetStatus` 0x101ba6208 stops the timer, a new PLAY restarts it).
    pub need: f64,
}

impl PlayCredit {
    /// A (new) load of `track`: its play time starts again.
    pub fn arm(&mut self, track: i64) {
        *self = PlayCredit { track: Some(track), secs: 0.0, done: false, need: self.need };
    }
    /// One frame: returns the track once, when it reaches the play time that counts. Tracks
    /// shorter than a minute count after 90% of their length.
    pub fn tick(&mut self, loaded: Option<i64>, playing: bool, dt: f64, duration: f64) -> Option<i64> {
        if loaded != self.track {
            *self = PlayCredit { track: loaded, secs: 0.0, done: false, need: self.need };
        }
        let t = self.track?;
        if !playing && self.need > 0.0 {
            self.secs = 0.0;
        }
        if self.done || !playing {
            return None;
        }
        // A stalled frame is not play time.
        self.secs += dt.clamp(0.0, 0.5);
        let base = if self.need > 0.0 { self.need } else { PLAY_CREDIT_SECS };
        let need = if duration > 0.0 { base.min(duration * 0.9).max(1.0) } else { base };
        if self.secs >= need {
            self.done = true;
            Some(t)
        } else {
            None
        }
    }
}

/// The stored order after "Renumber Track Order": the displayed order, provided it shows
/// exactly the playlist's entries (nothing filtered out).
pub fn renumbered(stored: &[i64], displayed: &[i64]) -> Option<Vec<i64>> {
    let (mut a, mut b) = (stored.to_vec(), displayed.to_vec());
    a.sort();
    b.sort();
    (a == b && stored != displayed).then(|| displayed.to_vec())
}

/// "Sort Items": children by name, ignoring case and accents; numbers in names in numeric order.
pub fn sorted_by_name(items: &[(i64, String)]) -> Vec<i64> {
    fn key(s: &str) -> Vec<(u8, u64, String)> {
        let f = crate::smart::fold(s);
        let mut out = vec![];
        let mut cur = String::new();
        let mut digits = false;
        for ch in f.chars() {
            if ch.is_ascii_digit() != digits && !cur.is_empty() {
                out.push(if digits { (0, cur.parse().unwrap_or(u64::MAX), String::new()) } else { (1, 0, cur.clone()) });
                cur.clear();
            }
            digits = ch.is_ascii_digit();
            cur.push(ch);
        }
        if !cur.is_empty() {
            out.push(if digits { (0, cur.parse().unwrap_or(u64::MAX), String::new()) } else { (1, 0, cur) });
        }
        out
    }
    let mut v: Vec<(Vec<(u8, u64, String)>, i64)> = items.iter().map(|(id, n)| (key(n), *id)).collect();
    v.sort();
    v.into_iter().map(|x| x.1).collect()
}

/// A name not yet used among `taken`: "Name", "Name (2)", "Name (3)"…
pub fn unique_name(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == name) {
        return name.to_string();
    }
    (2..).map(|n| format!("{name} ({n})")).find(|c| !taken.iter().any(|t| t == c)).unwrap()
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryInfo {
    pub id: i64,
    pub name: String,
    /// yyyy-mm-dd
    pub date: String,
    pub count: usize,
}

pub struct CellEdit {
    pub track: i64,
    pub col: Col,
    pub text: String,
    pub focused: bool,
}

/// One outside library shown in the tree (iTunes, rekordbox xml).
#[derive(Default)]
pub struct SourceState {
    pub lib: Option<Arc<SourceLib>>,
    pub rx: Option<Receiver<std::result::Result<SourceLib, String>>>,
    pub error: Option<String>,
    /// Expanded folders (node keys).
    pub open: HashSet<String>,
    /// Selected rows of the list (track indexes).
    pub sel: HashSet<usize>,
    pub anchor: Option<usize>,
    /// Playlist to fill once the running import has added its files: (playlist, paths in order).
    pub pending_playlist: Vec<(i64, Vec<String>)>,
}

pub const SRC_ITUNES: u8 = 0;
pub const SRC_RBXML: u8 = 1;

pub struct LibExt {
    pub edit: Option<CellEdit>,
    /// The editor was drawn this frame (a row scrolled out of view commits its edit).
    pub edit_drawn: bool,
    /// A click on an already-selected row's cell, waiting to see it is not a double-click:
    /// (track, column, time of the click).
    pub edit_pending: Option<(i64, Col, f64)>,
    /// The tree (or source strip) was clicked last, not the track list: the lit source tile is
    /// bright then and muted otherwise, as in rekordbox.
    pub tree_focus: bool,
    /// Colour chooser open for a track at a screen position.
    pub color_pick: Option<(i64, eframe::egui::Pos2, f64)>,
    pub preview: crate::preview::Preview,
    pub filter: TrackFilter,
    /// Stored playlist position (1-based) of each row of `App::list`.
    pub list_num: Vec<usize>,
    pub histories: Vec<HistoryInfo>,
    pub credit: PlayCredit,
    pub last_tick: Option<std::time::Instant>,
    pub tag_list: Vec<i64>,
    pub tag_set: HashSet<i64>,
    pub matches: HashMap<i64, Vec<i64>>,
    pub cleanup: Option<crate::collection_cleanup::ui::Review>,
    pub missing_open: bool,
    pub missing_sel: HashSet<i64>,
    /// Folder of a file just relocated by hand: offer to look there for the other missing files.
    pub reloc_offer: Option<PathBuf>,
    pub clipboard: Vec<i64>,
    /// Adding these tracks to this playlist would add duplicates: waiting for Add / Skip.
    pub dup_confirm: Option<(i64, Vec<i64>)>,
    pub tree_search_open: bool,
    pub tree_search: String,
    pub info_tab: u8,
    /// Field being edited in the Information window.
    pub info_edit: Option<(Col, String)>,
    pub sources: [SourceState; 2],
    pub history_confirm: Option<Option<i64>>,
    pub related_rename: Option<(usize, String)>,
    pub criteria_open: bool,
    pub palette_pick: Option<usize>,
    /// [Start] of the CUE Analysis Playlist is waiting for track analysis to finish.
    pub cue_wait: bool,
    /// The sub-browser's own browser state (ui_browser_ext.rs).
    pub sub: Option<Box<crate::ui_browser_ext::SubState>>,
    /// Right-hand panels shown last frame (a panel switched on closes the others).
    pub right_prev: [bool; 5],
    /// Cached rows of the Related Tracks and Track Suggestion windows: (key, time, rows).
    pub panel_cache: [Option<(String, f64, std::sync::Arc<Vec<(i64, f64)>>)>; 2],
    pub panel_sel: [Option<i64>; 2],
    /// New My Tag being typed in the My Tag window: (category, text).
    pub cv_scroll_end: bool,
    /// The sub-browser is being drawn (its state is swapped into the app's browser fields).
    pub in_sub: bool,
    /// The sub-browser was clicked last: list keys go to it.
    pub sub_focus: bool,
    /// Preferences were saved while the sub-browser's state was swapped in.
    pub sub_save_wanted: std::cell::Cell<bool>,
    /// New My Tag being typed in the My Tag window.
    pub panel_menu: Option<u8>,
}

impl Default for LibExt {
    fn default() -> Self {
        LibExt {
            edit: None,
            edit_drawn: false,
            edit_pending: None,
            tree_focus: false,
            color_pick: None,
            preview: Default::default(),
            filter: Default::default(),
            list_num: vec![],
            histories: vec![],
            credit: Default::default(),
            last_tick: None,
            tag_list: vec![],
            tag_set: HashSet::new(),
            matches: HashMap::new(),
            cleanup: None,
            missing_open: false,
            missing_sel: HashSet::new(),
            reloc_offer: None,
            clipboard: vec![],
            dup_confirm: None,
            tree_search_open: false,
            tree_search: String::new(),
            info_tab: 0,
            info_edit: None,
            sources: Default::default(),
            history_confirm: None,
            related_rename: None,
            criteria_open: false,
            palette_pick: None,
            cue_wait: false,
            sub: None,
            right_prev: [false; 5],
            panel_cache: [None, None],
            panel_sel: [None, None],
            cv_scroll_end: false,
            in_sub: false,
            sub_focus: false,
            sub_save_wanted: std::cell::Cell::new(false),
            panel_menu: None,
        }
    }
}

// ------------------------------------------------------------------------------------ database
impl Db {
    pub fn histories(&self) -> Result<Vec<HistoryInfo>> {
        let mut st = self.conn.prepare("SELECT h.id, h.name, h.date, (SELECT COUNT(*) FROM history_entries e WHERE e.history_id = h.id) FROM histories h ORDER BY h.date DESC, h.id DESC")?;
        let v = st.query_map([], |r| Ok(HistoryInfo { id: r.get(0)?, name: r.get(1)?, date: r.get(2)?, count: r.get::<_, i64>(3)? as usize }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }
    /// The history of `date` (yyyy-mm-dd), created when it does not exist yet.
    pub fn history_for_date(&self, date: &str) -> Result<i64> {
        if let Ok(id) = self.conn.query_row("SELECT id FROM histories WHERE date=?1 ORDER BY id LIMIT 1", [date], |r| r.get::<_, i64>(0)) {
            return Ok(id);
        }
        self.conn.execute("INSERT INTO histories(name, date) VALUES(?1, ?2)", params![format!("HISTORY {date}"), date])?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn history_add(&self, history: i64, track: i64, played_at: i64) -> Result<()> {
        self.conn.execute("INSERT INTO history_entries(history_id, track_id, played_at) VALUES(?1, ?2, ?3)", params![history, track, played_at])?;
        Ok(())
    }
    pub fn history_tracks(&self, history: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT track_id FROM history_entries WHERE history_id=?1 ORDER BY id")?;
        let v = st.query_map([history], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(v)
    }
    pub fn history_remove_tracks(&self, history: i64, tracks: &[i64]) -> Result<()> {
        for t in tracks {
            self.conn.execute("DELETE FROM history_entries WHERE history_id=?1 AND track_id=?2", params![history, t])?;
        }
        Ok(())
    }
    pub fn history_delete(&self, history: i64) -> Result<()> {
        self.conn.execute("DELETE FROM history_entries WHERE history_id=?1", [history])?;
        self.conn.execute("DELETE FROM histories WHERE id=?1", [history])?;
        Ok(())
    }
    pub fn history_delete_all(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM history_entries; DELETE FROM histories;")?;
        Ok(())
    }
    pub fn tag_list(&self) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT track_id FROM tag_list ORDER BY seq, track_id")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(v)
    }
    pub fn set_tag_list(&self, tracks: &[i64]) -> Result<()> {
        self.conn.execute_batch("BEGIN")?;
        let r = (|| -> Result<()> {
            self.conn.execute("DELETE FROM tag_list", [])?;
            for (i, t) in tracks.iter().enumerate() {
                self.conn.execute("INSERT OR IGNORE INTO tag_list(track_id, seq) VALUES(?1, ?2)", params![t, i as i64])?;
            }
            Ok(())
        })();
        self.conn.execute_batch(if r.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
        r
    }
    /// Every good-match pair, both ways round.
    pub fn track_matches(&self) -> Result<HashMap<i64, Vec<i64>>> {
        let mut st = self.conn.prepare("SELECT a, b FROM track_matches ORDER BY rowid")?;
        let mut m: HashMap<i64, Vec<i64>> = HashMap::new();
        for (a, b) in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.flatten() {
            m.entry(a).or_default().push(b);
            m.entry(b).or_default().push(a);
        }
        Ok(m)
    }
    pub fn set_match(&self, a: i64, b: i64, on: bool) -> Result<()> {
        if a == b {
            return Ok(());
        }
        let (a, b) = (a.min(b), a.max(b));
        if on {
            self.conn.execute("INSERT OR IGNORE INTO track_matches(a, b) VALUES(?1, ?2)", params![a, b])?;
        } else {
            self.conn.execute("DELETE FROM track_matches WHERE a=?1 AND b=?2", params![a, b])?;
        }
        Ok(())
    }
    pub fn set_node_seq(&self, id: i64, seq: i64) -> Result<()> {
        self.conn.execute("UPDATE playlists SET seq=?2 WHERE id=?1", params![id, seq])?;
        Ok(())
    }
}

// ------------------------------------------------------------------------------------ app logic
impl App {
    /// Load the tables this module keeps in memory.
    pub fn reload_library_ext(&mut self) {
        self.ext.histories = self.db.histories().unwrap_or_default();
        self.ext.tag_list = self.db.tag_list().unwrap_or_default();
        self.ext.tag_set = self.ext.tag_list.iter().copied().collect();
        self.ext.matches = self.db.track_matches().unwrap_or_default();
        // Palette slots and shortcuts whose playlist is gone are dropped.
        let ids: HashSet<i64> = self.nodes.iter().map(|n| n.id).collect();
        self.prefs.palette.resize(PALETTE_SLOTS, 0);
        for p in self.prefs.palette.iter_mut() {
            if *p != 0 && !ids.contains(p) {
                *p = 0;
            }
        }
        self.prefs.shortcuts.retain(|s| ids.contains(s));
        if self.prefs.related_lists.is_empty() {
            self.prefs.related_lists.push(RelatedList { name: "Related Tracks".into(), rules: self.prefs.related.clone() });
            self.prefs.related_sel = 0;
        }
        self.prefs.related_sel = self.prefs.related_sel.min(self.prefs.related_lists.len() - 1);
    }

    /// Once per frame: play-time credit (history and DJ Play Count), preview housekeeping and
    /// background results.
    pub fn poll_library_ext(&mut self) {
        self.poll_cue_analysis();
        let now = std::time::Instant::now();
        let dt = self.ext.last_tick.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.ext.last_tick = Some(now);
        let playing = self.playing() && !self.deck.cue_preview && self.deck.hot_preview.is_none();
        let dur = self.deck.duration;
        if let Some(t) = self.ext.credit.tick(self.deck.track, playing, dt, dur) {
            self.history_add_now(t);
        }
        // The preview follows the master volume and ends with the window's focus.
        let vol = self.player.state.lock().volume;
        // Preferences > Audio > Preview Volume: the middle of the slider is the master volume.
        self.ext.preview.set_volume(vol * (self.prefs.preview_volume * 2.0).clamp(0.0, 2.0));
        if self.ext.preview.poll() {
            if !self.ctx.input(|i| i.focused) && self.screenshot.is_none() {
                self.ext.preview.stop();
            } else {
                // A decode completion wakes the UI. Only an advancing playhead needs ticks.
                if self.ext.preview.state.lock().playing {
                    if self.prefs.playback_fps == 0 {
                        self.ctx.request_repaint_after(std::time::Duration::from_millis(33));
                    } else {
                        self.repaint_playback(&self.ctx);
                    }
                }
            }
        }
        let err = self.ext.preview.state.lock().error.take();
        if let Some(e) = err {
            self.toast_err(format!("This file cannot be played: {e}"));
        }
        self.poll_sources();
    }

    // ---------------------------------------------------------------- in-place editing
    pub fn commit_cell_edit(&mut self) {
        let Some(e) = self.ext.edit.take() else { return };
        self.edit_field(&[e.track], e.col, &e.text);
    }

    /// Set one field of several tracks from text (the Information window's multi-track edit
    /// and the list's in-place editor). Every changed track is one Undo step.
    pub fn edit_field(&mut self, ids: &[i64], col: Col, text: &str) -> bool {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return false;
        }
        let alnum = self.prefs.key_alnum;
        let mut ok = true;
        for id in ids {
            let Some(mut t) = self.track(*id).cloned() else { continue };
            if edit_text(&t, col, alnum) == text.trim() {
                continue;
            }
            if col == Col::Bpm && t.grid_locked {
                self.toast_err("This track's analysis is locked; unlock it to change its BPM.");
                ok = false;
                continue;
            }
            match apply_cell_edit(&mut t, col, text) {
                Err(m) => {
                    self.toast_err(m);
                    return false;
                }
                Ok(()) => {
                    let bpm = t.bpm;
                    if let Some(slot) = self.track_mut(*id) {
                        *slot = t;
                    }
                    self.save_track(*id);
                    if col == Col::Bpm {
                        // The grid follows the typed tempo from its first beat.
                        if let Some(g) = self.db.grid(*id).filter(|g| !g.is_empty()) {
                            let a = g.anchors[0];
                            let ng = BeatGrid::constant(a.time, bpm, a.beat);
                            if self.deck.track == Some(*id) {
                                self.set_deck_grid(ng, true);
                            } else {
                                let _ = self.db.save_grid(*id, &ng);
                            }
                        }
                    }
                }
            }
        }
        ok
    }

    // ---------------------------------------------------------------- preview
    /// Audition `track` from `frac` (0–1) of its length without loading it.
    pub fn preview_play(&mut self, track: i64, frac: f64) {
        if self.deck_options.disable_play { return self.toast_err("Preview is disabled for this session (--disable-play)."); }
        let Some(t) = self.track(track).cloned() else { return };
        if t.missing {
            return self.toast_err("File can not be found at the registered location.\nSelect [Relocate] from the right-click menu to register it again.");
        }
        let gain = if t.gain_db.is_finite() { 10f32.powf(t.gain_db / 20.0).min(4.0) } else { 1.0 };
        let dev = self.prefs.audio_device.clone();
        let ctx = self.ctx.clone();
        self.ext.preview.play(track, &t.path, frac, gain, dev.as_deref(), &ctx);
        if let Some(e) = self.ext.preview.error.clone() {
            self.toast_err(format!("Audio output unavailable: {e}"));
        }
    }
    pub fn preview_stop(&mut self) {
        self.ext.preview.stop();
    }
    pub fn preview_status(&self) -> Option<(i64, Option<f64>)> {
        self.ext.preview.status()
    }

    // ---------------------------------------------------------------- playlists
    /// "Renumber Track Order": the playlist's stored order becomes the order now displayed.
    /// Only when it is sorted by something other than # and nothing is filtered out.
    pub fn renumber_track_order(&mut self, pl: i64) -> bool {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return false;
        }
        if self.view != View::Playlist(pl) || self.is_manual_playlist() != Some(pl) {
            return false;
        }
        if self.list_dirty {
            self.rebuild_list();
        }
        let stored = self.db.playlist_tracks(pl).unwrap_or_default();
        let stored: Vec<i64> = stored.into_iter().filter(|t| self.index.contains_key(t)).collect();
        let Some(next) = renumbered(&stored, &self.list) else {
            if self.prefs.sort_col == Col::Num {
                self.toast("The track order is already in the order shown.");
            } else {
                self.toast_err("Clear the search and the Track Filter to renumber the track order.");
            }
            return false;
        };
        let _ = self.db.set_playlist_tracks(pl, &next);
        self.prefs.sort_col = Col::Num;
        self.prefs.sort_asc = true;
        self.save_prefs();
        self.reload_playlists();
        true
    }
    pub fn can_renumber(&self) -> bool {
        self.is_manual_playlist().is_some() && !(self.prefs.sort_col == Col::Num && self.prefs.sort_asc)
    }

    /// "Sort Items": the playlists and folders directly inside `parent` (0 = the Playlists root)
    /// in name order.
    pub fn sort_node_children(&mut self, parent: i64) {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        let kids: Vec<(i64, String)> = self.nodes.iter().filter(|n| n.parent == parent).map(|n| (n.id, n.name.clone())).collect();
        for (i, id) in sorted_by_name(&kids).into_iter().enumerate() {
            let _ = self.db.set_node_seq(id, i as i64);
        }
        self.reload_playlists();
    }

    fn playlist_entries(&self, node: i64) -> Vec<plf::Entry> {
        let alnum = self.prefs.key_alnum;
        self.view_tracks_of(&View::Playlist(node))
            .iter()
            .filter_map(|id| self.track(*id))
            .map(|t| plf::Entry { path: t.path.clone(), title: t.display_title(), artist: t.artist.clone(), album: t.album.clone(), genre: t.genre.clone(), bpm: t.bpm, rating: t.rating, duration: t.duration, key: t.stored_key_label(alnum), date_added: t.date_added.get(..10).unwrap_or(&t.date_added).to_string() })
            .collect()
    }

    /// "Save Playlist Info" / "Export a playlist to a file": write one playlist (or the tracks
    /// of a folder or intelligent playlist) as .m3u8, .pls or .txt, chosen by the file name.
    pub fn export_playlist_file(&mut self, node: i64, path: &Path) -> bool {
        let Some(format) = plf::Format::of(path) else {
            self.toast_err("Choose a file name ending in .m3u8, .pls or .txt.");
            return false;
        };
        let entries = self.playlist_entries(node);
        match std::fs::write(path, plf::encode(format, &entries)) {
            Ok(()) => {
                self.toast("Playlist export complete");
                true
            }
            Err(e) => {
                self.toast_err(format!("Could not write the file: {e}"));
                false
            }
        }
    }

    /// "Export Playlists Information": every playlist as a .txt file in `dir`, folders as
    /// subfolders. Returns how many files were written.
    pub fn export_all_playlists_info(&mut self, dir: &Path) -> usize {
        fn walk(app: &App, parent: i64, dir: &Path, n: &mut usize, err: &mut Option<String>) {
            let mut used: Vec<String> = vec![];
            let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|x| x.parent == parent).cloned().collect();
            for k in kids {
                let name = unique_name(&plf::safe_file_name(&k.name), &used);
                used.push(name.clone());
                if k.kind == NodeKind::Folder {
                    let sub = dir.join(&name);
                    if let Err(e) = std::fs::create_dir_all(&sub) {
                        err.get_or_insert(e.to_string());
                        continue;
                    }
                    walk(app, k.id, &sub, n, err);
                } else {
                    let entries = app.playlist_entries(k.id);
                    match std::fs::write(dir.join(format!("{name}.txt")), plf::encode(plf::Format::Txt, &entries)) {
                        Ok(()) => *n += 1,
                        Err(e) => {
                            err.get_or_insert(e.to_string());
                        }
                    }
                }
            }
        }
        let (mut n, mut err) = (0, None);
        if let Err(e) = std::fs::create_dir_all(dir) {
            self.toast_err(format!("Cannot create directory: {e}"));
            return 0;
        }
        walk(self, 0, dir, &mut n, &mut err);
        match err {
            Some(e) => self.toast_err(format!("Exported {n} playlist(s); some could not be written ({e}).")),
            None => self.toast(format!("Playlist export complete ({n} playlists)")),
        }
        n
    }

    /// "Import Playlist": read an .m3u/.m3u8/.pls or rekordbox .txt playlist into a new playlist
    /// inside `parent` (0 = top level). Files not yet in the collection are imported; entries
    /// without a file are matched by title and artist. Returns the new playlist.
    pub fn import_playlist_file(&mut self, path: &Path, parent: i64) -> Option<i64> {
        let Some(format) = plf::Format::of(path) else {
            self.toast_err(format!("Cannot import the playlist: {}", path.display()));
            return None;
        };
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.toast_err(format!("Cannot import the playlist: {e}"));
                return None;
            }
        };
        let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let entries = plf::parse(format, &bytes, &base);
        // Each entry: a collection track, or a file to import first.
        let mut order: Vec<String> = vec![];
        let mut have: HashMap<String, i64> = HashMap::new();
        let mut to_import: Vec<PathBuf> = vec![];
        for e in &entries {
            if !e.path.is_empty() {
                if let Some(id) = self.db.track_by_path(&e.path) {
                    have.insert(e.path.clone(), id);
                    order.push(e.path.clone());
                    continue;
                }
                if Path::new(&e.path).is_file() {
                    if !to_import.iter().any(|p| p.to_string_lossy() == e.path.as_str()) {
                        to_import.push(PathBuf::from(&e.path));
                    }
                    order.push(e.path.clone());
                    continue;
                }
            }
            if let Some(id) = (!e.title.is_empty()).then(|| self.match_device_track("", &e.title, &e.artist)).flatten() {
                let key = format!("\u{1}{id}");
                have.insert(key.clone(), id);
                order.push(key);
            }
        }
        if order.is_empty() {
            self.toast_err("Cannot import the playlist:\nAll songs in the list do not exist in Collection or the list is empty.");
            return None;
        }
        let parent = if self.node(parent).map(|n| n.kind == NodeKind::Folder).unwrap_or(false) { parent } else { 0 };
        let taken: Vec<String> = self.nodes.iter().filter(|n| n.parent == parent).map(|n| n.name.clone()).collect();
        let name = unique_name(&path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled Playlist".into()), &taken);
        let id = self.db.create_node(parent, &name, NodeKind::Playlist, None).ok()?;
        let now: Vec<i64> = order.iter().filter_map(|k| have.get(k).copied()).collect();
        let _ = self.db.set_playlist_tracks(id, &now);
        self.reload_playlists();
        if parent > 0 {
            self.expanded.insert(parent);
        }
        let skipped = entries.len() - order.len();
        if to_import.is_empty() {
            self.toast(format!("Imported the playlist \"{name}\" ({} tracks{}).", now.len(), if skipped > 0 { format!(", {skipped} not found") } else { String::new() }));
        } else {
            // The files are read in the background; the playlist is put in file order when
            // they have arrived.
            self.ext.sources[0].pending_playlist.push((id, order));
            self.import_paths(to_import, None);
        }
        Some(id)
    }

    /// Give playlists waiting for imported files their final order.
    fn finish_pending_playlists(&mut self) {
        if self.importing.is_some() {
            return;
        }
        let pending: Vec<(i64, Vec<String>)> = self.ext.sources.iter_mut().flat_map(|s| std::mem::take(&mut s.pending_playlist)).collect();
        if pending.is_empty() {
            return;
        }
        for (pl, order) in pending {
            if self.node(pl).is_none() {
                continue;
            }
            let ids: Vec<i64> = order.iter().filter_map(|k| match k.strip_prefix('\u{1}') {
                Some(id) => id.parse::<i64>().ok().filter(|id| self.index.contains_key(id)),
                None => self.db.track_by_path(k),
            }).collect();
            let _ = self.db.set_playlist_tracks(pl, &ids);
        }
        self.reload_playlists();
    }

    /// Would adding `ids` to `pl` add tracks it already has? Then the question is put to the
    /// user and true is returned (nothing added yet).
    pub fn dup_pending(&mut self, pl: i64, ids: &[i64]) -> bool {
        let cur: HashSet<i64> = self.db.playlist_tracks(pl).unwrap_or_default().into_iter().collect();
        if ids.iter().any(|i| cur.contains(i)) {
            self.ext.dup_confirm = Some((pl, ids.to_vec()));
            true
        } else {
            false
        }
    }
    /// Answer to the duplicates question: add everything, or only the tracks not yet there.
    pub fn resolve_duplicates(&mut self, add_duplicates: bool) {
        let Some((pl, ids)) = self.ext.dup_confirm.take() else { return };
        let ids: Vec<i64> = if add_duplicates {
            ids
        } else {
            let cur: HashSet<i64> = self.db.playlist_tracks(pl).unwrap_or_default().into_iter().collect();
            ids.into_iter().filter(|i| !cur.contains(i)).collect()
        };
        if ids.is_empty() {
            return;
        }
        self.add_to_playlist_force(pl, &ids);
    }

    pub fn palette_assign(&mut self, slot: usize, playlist: i64) {
        self.prefs.palette.resize(PALETTE_SLOTS, 0);
        if slot < PALETTE_SLOTS && self.node(playlist).map(|n| n.kind == NodeKind::Playlist).unwrap_or(false) {
            for p in self.prefs.palette.iter_mut() {
                if *p == playlist {
                    *p = 0;
                }
            }
            self.prefs.palette[slot] = playlist;
            self.save_prefs();
        }
    }
    pub fn palette_remove(&mut self, slot: usize) {
        if let Some(p) = self.prefs.palette.get_mut(slot) {
            *p = 0;
            self.save_prefs();
        }
    }
    pub fn shortcut_add(&mut self, node: i64) {
        if self.prefs.shortcuts.contains(&node) {
            return;
        }
        if self.prefs.shortcuts.len() >= SHORTCUT_MAX {
            return self.toast_err(format!("Up to {SHORTCUT_MAX} shortcuts can be created."));
        }
        self.prefs.shortcuts.push(node);
        self.prefs.shortcuts_open = true;
        self.save_prefs();
    }
    pub fn shortcut_remove(&mut self, node: i64) {
        self.prefs.shortcuts.retain(|s| *s != node);
        self.save_prefs();
    }
    /// Show a playlist or folder in the tree and list.
    pub fn jump_to_playlist(&mut self, node: i64) {
        let mut cur = self.node(node).map(|n| n.parent).unwrap_or(0);
        let mut guard = 0;
        while cur != 0 && guard < 64 {
            self.expanded.insert(cur);
            cur = self.node(cur).map(|n| n.parent).unwrap_or(0);
            guard += 1;
        }
        self.side = SideMode::Playlists;
        self.set_view(View::Playlist(node));
    }

    // ---------------------------------------------------------------- histories
    /// A track was loaded to the player: its play time starts counting. After about a minute
    /// of playing (`PLAY_CREDIT_SECS`) it is added to today's history and its DJ Play Count
    /// goes up by one. (The player is also watched every frame, so calling this is optional;
    /// it restarts the count when the same track is loaded again.)
    pub fn history_record(&mut self, track: i64) {
        self.ext.credit.arm(track);
    }

    /// Add `track` to today's history ("HISTORY yyyy-mm-dd") now and count the play.
    pub fn history_add_now(&mut self, track: i64) {
        if self.track(track).is_none() {
            return;
        }
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let now = chrono::Local::now().timestamp();
        if let Ok(h) = self.db.history_for_date(&date) {
            let _ = self.db.history_add(h, track, now);
            if self.view == View::History(h) {
                self.list_dirty = true;
            }
        }
        if let Some(t) = self.track_mut(track) {
            t.play_count = t.play_count.saturating_add(1);
        }
        self.save_track_quiet(track);
        self.ext.histories = self.db.histories().unwrap_or_default();
    }

    /// "Save as Playlist": a playlist with the history's tracks, in the "Histories" folder.
    pub fn history_save_as_playlist(&mut self, history: i64) -> Option<i64> {
        let name = self.ext.histories.iter().find(|h| h.id == history)?.name.clone();
        let tracks = self.db.history_tracks(history).unwrap_or_default();
        let taken: Vec<String> = self.nodes.iter().filter(|n| n.parent == 0).map(|n| n.name.clone()).collect();
        let id = self.db.create_node(0, &unique_name(&name, &taken), NodeKind::Playlist, None).ok()?;
        let _ = self.db.set_playlist_tracks(id, &tracks);
        self.reload_playlists();
        self.toast(format!("Saved \"{name}\" as a playlist."));
        Some(id)
    }
    pub fn history_delete(&mut self, history: i64) {
        let _ = self.db.history_delete(history);
        if self.view == View::History(history) {
            self.set_view(View::History(0));
        }
        self.ext.histories = self.db.histories().unwrap_or_default();
    }
    /// "Clear History": delete every history.
    pub fn history_clear_all(&mut self) {
        let _ = self.db.history_delete_all();
        if matches!(self.view, View::History(_)) {
            self.set_view(View::History(0));
        }
        self.ext.histories = vec![];
    }
    pub fn history_remove_tracks(&mut self, history: i64, tracks: &[i64]) {
        let _ = self.db.history_remove_tracks(history, tracks);
        self.ext.histories = self.db.histories().unwrap_or_default();
        self.list_dirty = true;
    }

    /// "import History": one device history as a playlist in the "Histories" folder.
    pub fn import_device_history(&mut self, root: &str, id: u32) -> Option<i64> {
        let dev = self.devices.iter().find(|d| d.root == root)?;
        let lib = dev.lib.clone()?;
        let dev_name = dev.name.clone();
        let h = lib.histories.iter().find(|h| h.id == id)?;
        let ids: Vec<i64> = h.entries.iter().filter_map(|e| lib.tracks.get(e)).filter_map(|dt| self.match_device_track(&dt.source, &dt.title, &dt.artist)).collect();
        let folder = match self.nodes.iter().find(|n| n.parent == 0 && n.name == "Histories" && n.kind == NodeKind::Folder) {
            Some(n) => n.id,
            None => self.db.create_node(0, "Histories", NodeKind::Folder, None).ok()?,
        };
        let name = format!("{} ({dev_name})", h.name);
        let existing = self.db.playlists().unwrap_or_default().into_iter().find(|n| n.parent == folder && n.name == name && n.kind == NodeKind::Playlist).map(|n| n.id);
        let pl = match existing {
            Some(p) => p,
            None => self.db.create_node(folder, &name, NodeKind::Playlist, None).ok()?,
        };
        let _ = self.db.set_playlist_tracks(pl, &ids);
        self.reload_playlists();
        self.toast(format!("Imported \"{}\" ({} of {} tracks are in the collection).", h.name, ids.len(), h.entries.len()));
        Some(pl)
    }

    // ---------------------------------------------------------------- tag list
    pub fn tag_list_add(&mut self, ids: &[i64]) {
        let mut list = self.ext.tag_list.clone();
        let mut full = false;
        for id in ids {
            if list.contains(id) || self.track(*id).is_none() {
                continue;
            }
            if list.len() >= TAG_LIST_MAX {
                full = true;
                break;
            }
            list.push(*id);
        }
        if full {
            self.toast_err("Up to 100 tracks can be saved in the Tag List.");
        }
        self.set_tag_list(list);
    }
    pub fn tag_list_remove(&mut self, ids: &[i64]) {
        let list: Vec<i64> = self.ext.tag_list.iter().copied().filter(|t| !ids.contains(t)).collect();
        self.set_tag_list(list);
    }
    /// "Delete All"
    pub fn tag_list_clear(&mut self) {
        self.set_tag_list(vec![]);
    }
    fn set_tag_list(&mut self, list: Vec<i64>) {
        let _ = self.db.set_tag_list(&list);
        self.ext.tag_set = list.iter().copied().collect();
        self.ext.tag_list = list;
        if self.view == View::TagList {
            self.list_dirty = true;
        }
    }
    /// "Save as Playlist"
    pub fn tag_list_save_as_playlist(&mut self) -> Option<i64> {
        if self.ext.tag_list.is_empty() {
            return None;
        }
        let taken: Vec<String> = self.nodes.iter().filter(|n| n.parent == 0).map(|n| n.name.clone()).collect();
        let id = self.db.create_node(0, &unique_name("Tag List", &taken), NodeKind::Playlist, None).ok()?;
        let list = self.ext.tag_list.clone();
        let _ = self.db.set_playlist_tracks(id, &list);
        self.reload_playlists();
        self.dialogs.rename = Some((id, self.node(id).map(|n| n.name.clone()).unwrap_or_default()));
        self.side = SideMode::Playlists;
        Some(id)
    }

    // ---------------------------------------------------------------- matching
    pub fn matching_of(&self, track: i64) -> Vec<i64> {
        self.ext.matches.get(&track).map(|v| v.iter().copied().filter(|t| self.index.contains_key(t)).collect()).unwrap_or_default()
    }
    pub fn is_match(&self, a: i64, b: i64) -> bool {
        self.ext.matches.get(&a).map(|v| v.contains(&b)).unwrap_or(false)
    }
    /// Register (or remove) `a` and `b` as a good match.
    pub fn matching_set(&mut self, a: i64, b: i64, on: bool) {
        if a == b {
            return;
        }
        let _ = self.db.set_match(a, b, on);
        self.ext.matches = self.db.track_matches().unwrap_or_default();
        if matches!(self.view, View::Matching | View::Related) {
            self.list_dirty = true;
        }
    }

    // ---------------------------------------------------------------- missing files
    /// File > Display All Missing Files: the Missing File Manager window.
    pub fn open_missing_file_manager(&mut self) {
        self.ext.missing_open = true;
        self.ext.missing_sel.clear();
        if !self.tracks.iter().any(|t| t.missing) {
            self.toast("All files exist.");
        }
    }

    /// "Auto Relocate": look for the missing files (all, or only `ids`) by name under the
    /// Auto Relocate Search Folders. Runs in the background.
    pub fn auto_relocate(&mut self, ids: Option<Vec<i64>>) {
        let folders: Vec<PathBuf> = crate::prefs_rb::relocate_roots(&self.prefs).into_iter().filter(|p| p.is_dir()).collect();
        if folders.is_empty() {
            self.ext.missing_open = true;
            return self.toast_err("Add a folder to the Auto Relocate Search Folders first.");
        }
        self.start_relocation_scan(folders, ids);
    }

    /// Apply the same filename, ownership and size checks for both folder searches.
    pub(crate) fn apply_relocation(&mut self, by_name: &HashMap<String, Vec<PathBuf>>, only: Option<&[i64]>) {
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        let selected = only.map(|ids| ids.iter().copied().collect::<HashSet<_>>());
        let missing = self.tracks.iter()
            .filter(|t| t.missing && selected.as_ref().is_none_or(|ids| ids.contains(&t.id)));
        let matches = relocation_matches(&self.db, by_name, missing);
        let total = matches.len();
        let mut moved = 0;
        let mut reasons = std::collections::BTreeMap::new();
        for (id, candidate) in matches {
            let reason = match candidate {
                RelocationMatch::Found(path, length) if regular_file_size(path) != Some(length) => "File changed during search",
                RelocationMatch::Found(path, _) => {
                    if self.relocate_track(id, &path.to_string_lossy()) {
                        moved += 1;
                        continue;
                    }
                    "Could not save the new path"
                }
                RelocationMatch::NoMatch => "No filename match in the searched folders",
                RelocationMatch::InUse => "Already used by another track",
                RelocationMatch::Shared => "Same candidate for multiple missing tracks",
                RelocationMatch::Multiple => "Multiple possible files",
                RelocationMatch::SizeMismatch => "Different file size",
                RelocationMatch::Unverified => "Unverified file attributes",
            };
            *reasons.entry(reason).or_insert(0usize) += 1;
        }
        self.list_dirty = true;
        let noun = if total == 1 { "track" } else { "tracks" };
        let mut message = format!("Relocated {moved} of {total} missing {noun}.");
        for (label, count) in reasons {
            message.push_str(&format!(" {label} ({count})."));
        }
        self.toast(message);
    }

    /// "Relocate": point one track at a file chosen by the user, then offer to look for the
    /// other missing files in that file's folder.
    pub fn relocate_to(&mut self, id: i64, file: &Path) -> bool {
        let ok = self.relocate_track(id, &file.to_string_lossy());
        if ok && self.tracks.iter().any(|t| t.missing) {
            self.ext.reloc_offer = file.parent().map(|p| p.to_path_buf());
        }
        ok
    }

    /// "Analyze Missing Data" / "Add New Analysis Data": analyse only the tracks that lack a
    /// beat grid, key or waveform. Analysis Lock is respected. Returns how many were queued.
    pub fn analyze_missing(&mut self, ids: &[i64]) -> usize {
        let (want_grid, want_key) = (self.prefs.analyze_grid, self.prefs.analyze_key);
        let mut needed = HashSet::new();
        let mut check_grids = Vec::new();
        let mut seen = HashSet::new();
        for &id in ids {
            if !seen.insert(id) {
                continue;
            }
            let Some(t) = self.track(id) else { continue };
            if t.missing || t.analyzed == 3 || (t.analyzed == 1 && t.grid_locked) {
                continue;
            }
            let no_key = want_key && t.key < 0 && t.key_text.trim().is_empty();
            if t.analyzed != 1 || no_key || !self.previews.contains_key(&id) {
                needed.insert(id);
            } else if want_grid {
                check_grids.push(id);
            }
        }
        if !check_grids.is_empty() {
            match self.db.missing_grids(&check_grids) {
                Ok(missing) => needed.extend(missing),
                Err(e) => {
                    self.toast_err(format!("Could not check the selected beat grids ({e})."));
                    return 0;
                }
            }
        }
        let targets: Vec<_> = ids.iter().copied().filter(|id| needed.remove(id)).collect();
        if targets.is_empty() {
            self.toast("No analysis data is missing for the selected tracks.");
        } else {
            self.analyze(&targets, true);
        }
        targets.len()
    }

    // ---------------------------------------------------------------- clipboard / selection
    fn selection_in_order(&self) -> Vec<i64> {
        self.list.iter().copied().filter(|t| self.selected.contains(t)).collect()
    }
    /// Copy (or Cut, in a playlist) the selected tracks.
    pub fn copy_tracks(&mut self, cut: bool) {
        let sel = self.selection_in_order();
        if sel.is_empty() {
            return;
        }
        if cut {
            let Some(pl) = self.is_manual_playlist() else { return };
            let s: HashSet<i64> = sel.iter().copied().collect();
            self.remove_from_playlist(pl, &s);
        }
        self.ext.clipboard = sel;
    }
    /// Paste the copied tracks into the playlist shown (after the selected row, else at the end).
    pub fn paste_tracks(&mut self) {
        let Some(pl) = self.is_manual_playlist() else { return };
        let ids: Vec<i64> = self.ext.clipboard.iter().copied().filter(|t| self.index.contains_key(t)).collect();
        if ids.is_empty() {
            return;
        }
        let mut cur = self.db.playlist_tracks(pl).unwrap_or_default();
        let at = self.anchor_id.filter(|a| self.selected.contains(a)).and_then(|a| cur.iter().position(|t| *t == a)).map(|i| i + 1).unwrap_or(cur.len());
        for (k, t) in ids.iter().enumerate() {
            cur.insert(at + k, *t);
        }
        let _ = self.db.set_playlist_tracks(pl, &cur);
        self.reload_playlists();
    }
    pub fn can_paste(&self) -> bool {
        !self.ext.clipboard.is_empty() && self.is_manual_playlist().is_some()
    }
    pub fn select_all(&mut self) {
        if self.list_dirty {
            self.rebuild_list();
        }
        self.selected = self.list.iter().copied().collect();
    }

    // ---------------------------------------------------------------- related tracks lists
    /// Switch to another saved "Related Tracks" list (the current criteria are kept in theirs).
    pub fn related_select(&mut self, i: usize) {
        self.related_store();
        if let Some(l) = self.prefs.related_lists.get(i) {
            self.prefs.related = l.rules.clone();
            self.prefs.related_sel = i;
            self.list_dirty = true;
            self.save_prefs();
        }
    }
    /// Keep the criteria being edited in the selected saved list.
    pub fn related_store(&mut self) {
        let sel = self.prefs.related_sel;
        let cur = self.prefs.related.clone();
        if let Some(l) = self.prefs.related_lists.get_mut(sel) {
            l.rules = cur;
        }
    }
    /// Create New "Related Tracks" (copy: with the current criteria).
    pub fn related_new(&mut self, copy: bool) {
        self.related_store();
        let taken: Vec<String> = self.prefs.related_lists.iter().map(|l| l.name.clone()).collect();
        let (name, rules) = if copy {
            (unique_name(&format!("Copy (of {})", self.prefs.related_lists.get(self.prefs.related_sel).map(|l| l.name.as_str()).unwrap_or("Related Tracks")), &taken), self.prefs.related.clone())
        } else {
            (unique_name("Related Tracks", &taken), RelatedRules::default())
        };
        self.prefs.related_lists.push(RelatedList { name, rules: rules.clone() });
        self.prefs.related_sel = self.prefs.related_lists.len() - 1;
        self.prefs.related = rules;
        self.list_dirty = true;
        self.save_prefs();
    }
    pub fn related_delete(&mut self, i: usize) {
        if self.prefs.related_lists.len() <= 1 || i >= self.prefs.related_lists.len() {
            return;
        }
        self.related_store();
        self.prefs.related_lists.remove(i);
        let sel = if self.prefs.related_sel > i { self.prefs.related_sel - 1 } else { self.prefs.related_sel.min(self.prefs.related_lists.len() - 1) };
        self.prefs.related_sel = sel;
        self.prefs.related = self.prefs.related_lists[sel].rules.clone();
        self.list_dirty = true;
        self.save_prefs();
    }
    /// Turn the "Related Tracks" into a playlist
    pub fn related_to_playlist(&mut self) -> Option<i64> {
        let tracks = self.related_tracks();
        let name = self.prefs.related_lists.get(self.prefs.related_sel).map(|l| l.name.clone()).unwrap_or_else(|| "Related Tracks".into());
        let taken: Vec<String> = self.nodes.iter().filter(|n| n.parent == 0).map(|n| n.name.clone()).collect();
        let id = self.db.create_node(0, &unique_name(&name, &taken), NodeKind::Playlist, None).ok()?;
        let _ = self.db.set_playlist_tracks(id, &tracks);
        self.reload_playlists();
        Some(id)
    }

    // ---------------------------------------------------------------- iTunes / rekordbox xml
    /// (Re)load an outside library in the background. kind: SRC_ITUNES or SRC_RBXML.
    pub fn source_reload(&mut self, kind: u8) {
        let k = kind as usize % 2;
        if self.ext.sources[k].rx.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        if kind == SRC_ITUNES {
            let chosen = self.prefs.itunes_xml.clone();
            // Screenshot runs never talk to the Music app.
            let ask = self.screenshot.is_none();
            std::thread::spawn(move || {
                let _ = tx.send(crate::itunes::load_itunes(chosen.as_deref(), ask));
                ctx.request_repaint();
            });
        } else {
            let Some(path) = self.prefs.rekordbox_xml_path.clone().filter(|p| !p.trim().is_empty()) else {
                self.ext.sources[k].error = Some("Choose a rekordbox xml file first.".into());
                self.ext.sources[k].lib = None;
                return;
            };
            std::thread::spawn(move || {
                let _ = tx.send(crate::itunes::load_rekordbox_xml(Path::new(&path)));
                ctx.request_repaint();
            });
        }
        let s = &mut self.ext.sources[k];
        s.rx = Some(rx);
        s.error = None;
        s.sel.clear();
    }
    /// Reload iTunes Library
    pub fn itunes_reload(&mut self) {
        self.source_reload(SRC_ITUNES);
    }
    pub fn rekordbox_xml_reload(&mut self) {
        self.source_reload(SRC_RBXML);
    }

    fn poll_sources(&mut self) {
        for k in 0..2 {
            let got = match &self.ext.sources[k].rx {
                Some(rx) => rx.try_recv().ok(),
                None => None,
            };
            if let Some(r) = got {
                let s = &mut self.ext.sources[k];
                s.rx = None;
                match r {
                    Ok(lib) => {
                        s.lib = Some(Arc::new(lib));
                        s.error = None;
                    }
                    Err(e) => {
                        s.lib = None;
                        s.error = Some(e);
                    }
                }
            }
        }
        self.finish_pending_playlists();
    }

    /// "Import To Collection": tracks of an outside library by index. rekordbox xml tracks come
    /// with their cues and beat grids. Returns the collection ids already known (files are read
    /// in the background for iTunes).
    pub fn source_import_tracks(&mut self, kind: u8, idx: &[usize]) -> Vec<i64> {
        let Some(lib) = self.ext.sources[kind as usize % 2].lib.clone() else { return vec![] };
        if let Some(xml) = &lib.xml {
            let picked: Vec<&crate::importer::XmlTrack> = idx.iter().filter_map(|i| xml.tracks.get(*i)).collect();
            let (ids, added) = self.import_xml_tracks(&picked);
            self.toast(format!("{added} track(s) added to the collection"));
            return ids;
        }
        let mut have = vec![];
        let mut paths = vec![];
        for i in idx {
            let Some(t) = lib.tracks.get(*i) else { continue };
            match self.db.track_by_path(&t.path) {
                Some(id) => have.push(id),
                None => paths.push(PathBuf::from(&t.path)),
            }
        }
        if paths.is_empty() {
            self.toast("This track already exists in the current collection.");
        } else {
            self.import_paths(paths, None);
        }
        have
    }

    /// Import a playlist or folder of an outside library ("Import To Collection" on a tree row)
    /// into the Playlists root, with its tracks.
    pub fn source_import_node(&mut self, kind: u8, key: &str) -> Option<i64> {
        let lib = self.ext.sources[kind as usize % 2].lib.clone()?;
        let node = lib.node(key)?.clone();
        // rekordbox xml: tracks first (synchronously), so playlists can refer to them.
        let mut xml_ids: HashMap<usize, i64> = HashMap::new();
        let all = lib.tracks_of(key);
        let mut to_import: Vec<PathBuf> = vec![];
        if let Some(xml) = &lib.xml {
            let picked: Vec<&crate::importer::XmlTrack> = all.iter().filter_map(|i| xml.tracks.get(*i)).collect();
            let (ids, _) = self.import_xml_tracks(&picked);
            for (i, id) in all.iter().zip(ids) {
                xml_ids.insert(*i, id);
            }
        } else {
            for i in &all {
                let p = &lib.tracks[*i].path;
                if self.db.track_by_path(p).is_none() && Path::new(p).is_file() && !to_import.iter().any(|x| x.to_string_lossy() == p.as_str()) {
                    to_import.push(PathBuf::from(p));
                }
            }
        }
        fn add(app: &mut App, lib: &SourceLib, n: &SrcNode, parent: i64, xml_ids: &HashMap<usize, i64>, pending: &mut Vec<(i64, Vec<String>)>) -> Option<i64> {
            let taken: Vec<String> = app.db.playlists().unwrap_or_default().into_iter().filter(|x| x.parent == parent).map(|x| x.name).collect();
            let name = unique_name(if n.name.trim().is_empty() { "Untitled Playlist" } else { &n.name }, &taken);
            if n.folder {
                let id = app.db.create_node(parent, &name, NodeKind::Folder, None).ok()?;
                for c in &n.children {
                    add(app, lib, c, id, xml_ids, pending);
                }
                Some(id)
            } else {
                let id = app.db.create_node(parent, &name, NodeKind::Playlist, None).ok()?;
                if lib.xml.is_some() {
                    let ids: Vec<i64> = n.tracks.iter().filter_map(|i| xml_ids.get(i).copied()).collect();
                    let _ = app.db.set_playlist_tracks(id, &ids);
                } else {
                    let order: Vec<String> = n.tracks.iter().map(|i| lib.tracks[*i].path.clone()).collect();
                    let now: Vec<i64> = order.iter().filter_map(|p| app.db.track_by_path(p)).collect();
                    let _ = app.db.set_playlist_tracks(id, &now);
                    pending.push((id, order));
                }
                Some(id)
            }
        }
        let mut pending = vec![];
        let id = add(self, &lib, &node, 0, &xml_ids, &mut pending);
        self.reload_playlists();
        if to_import.is_empty() {
            self.toast(format!("Imported \"{}\" as a playlist.", node.name));
        } else {
            self.ext.sources[kind as usize % 2].pending_playlist.extend(pending);
            self.import_paths(to_import, None);
        }
        id
    }

    /// Add rekordbox xml tracks to the collection with their grids and cues (existing tracks
    /// are left as they are). Returns each track's collection id, in order, and how many are new.
    pub fn import_xml_tracks(&mut self, tracks: &[&crate::importer::XmlTrack]) -> (Vec<i64>, usize) {
        let mut ids = vec![];
        let mut added = 0;
        let _ = self.db.conn.execute_batch("BEGIN");
        for xt in tracks {
            if let Some(id) = self.db.track_by_path(&xt.track.path) {
                ids.push(id);
                continue;
            }
            let mut t = xt.track.clone();
            let real = crate::importer::read_track(Path::new(&t.path));
            if real.sample_rate > 0 {
                t.sample_rate = real.sample_rate;
                t.bit_depth = real.bit_depth;
                t.artwork = real.artwork;
                if t.duration <= 0.0 {
                    t.duration = real.duration;
                }
            }
            if t.date_added.is_empty() {
                t.date_added = real.date_added;
            }
            if t.date_created.is_empty() {
                t.date_created = real.date_created;
            }
            let Ok(id) = self.db.insert_track(&t) else { continue };
            added += 1;
            if let Some(g) = &xt.grid {
                let _ = self.db.save_grid(id, g);
            }
            if !xt.cues.is_empty() {
                let _ = self.db.replace_cues(id, &xt.cues);
            }
            let mut extra = xt.extra.clone();
            extra.grid_fp = crate::importer::grid_fingerprint(&self.db.grid(id));
            extra.cues_fp = crate::importer::cues_fingerprint(&self.db.cues(id).unwrap_or_default());
            if xt.grid.is_none() {
                extra.tempo.clear();
            }
            if xt.cues.is_empty() {
                extra.marks.clear();
            }
            let _ = self.db.set_xml_extra(id, &extra);
            ids.push(id);
        }
        let _ = self.db.conn.execute_batch("COMMIT");
        if added > 0 {
            self.reload_all();
            if self.prefs.auto_analysis {
                let todo: Vec<i64> = ids.iter().copied().filter(|id| self.track(*id).map(|t| t.analyzed == 0 && !t.missing).unwrap_or(false)).collect();
                self.analyze(&todo, false);
            }
        }
        (ids, added)
    }

    /// "Get Info from iTunes": copy the iTunes library's title, artist, album, genre, composer,
    /// grouping, comments, year and rating to collection tracks with the same file.
    pub fn get_info_from_itunes(&mut self, ids: &[i64]) -> usize {
        let Some(lib) = self.ext.sources[SRC_ITUNES as usize].lib.clone() else {
            self.source_reload(SRC_ITUNES);
            self.toast("Loading iTunes Library. Try again when it has loaded.");
            return 0;
        };
        let by_path: HashMap<&str, &crate::itunes::SrcTrack> = lib.tracks.iter().map(|t| (t.path.as_str(), t)).collect();
        let mut n = 0;
        for id in ids {
            let Some(mut t) = self.track(*id).cloned() else { continue };
            let Some(s) = by_path.get(t.path.as_str()) else { continue };
            t.title = s.title.clone();
            t.artist = s.artist.clone();
            t.album = s.album.clone();
            t.genre = s.genre.clone();
            t.composer = s.composer.clone();
            t.grouping = s.grouping.clone();
            t.comment = s.comment.clone();
            if s.year > 0 {
                t.year = s.year;
            }
            t.rating = s.rating;
            if let Some(slot) = self.track_mut(*id) {
                *slot = t;
            }
            self.save_track(*id);
            n += 1;
        }
        if n == 0 {
            self.toast_err("The track is not found in iTunes.");
        } else {
            self.toast(format!("Updated {n} track(s) from the iTunes library."));
        }
        n
    }
}

/// File names (folded) to paths, for every file under `folders`.
pub fn scan_file_names(folders: &[PathBuf]) -> HashMap<String, Vec<PathBuf>> {
    let mut by_name: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for folder in folders {
        for e in walkdir::WalkDir::new(folder).into_iter().flatten() {
            if e.file_type().is_file() {
                if let Some(n) = e.file_name().to_str() {
                    let v = by_name.entry(crate::smart::fold(n)).or_default();
                    if !v.iter().any(|p| p == e.path()) {
                        v.push(e.path().to_path_buf());
                    }
                }
            }
        }
    }
    by_name
}

/// Decide the whole selection before changing the collection. Two missing
/// records that point to one new path must not be assigned by iteration order.
fn relocation_matches<'a, 't>(
    db: &Db,
    by_name: &'a HashMap<String, Vec<PathBuf>>,
    tracks: impl Iterator<Item = &'t Track>,
) -> Vec<(i64, RelocationMatch<'a>)> {
    let mut claims = HashMap::new();
    let mut matches = Vec::new();
    for track in tracks {
        let paths = by_name
            .get(&crate::smart::fold(&track.file_name()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let candidate = match_scanned_files(db, paths, track.id, track.file_size);
        if let RelocationMatch::Found(path, _) = candidate {
            *claims.entry(path).or_insert(0usize) += 1;
        }
        matches.push((track.id, candidate));
    }
    for (_, candidate) in &mut matches {
        if matches!(*candidate, RelocationMatch::Found(path, _) if claims[&path] > 1) {
            *candidate = RelocationMatch::Shared;
        }
    }
    matches
}

fn regular_file_size(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
}

fn match_scanned_files<'a>(db: &Db, paths: &'a [PathBuf], id: i64, size: u64) -> RelocationMatch<'a> {
    let candidates: Vec<_> = paths
        .iter()
        .filter(|path| db.track_by_path(&path.to_string_lossy()).is_none_or(|owner| owner == id))
        .map(|path| (path.as_path(), regular_file_size(path)))
        .collect();
    if !paths.is_empty() && candidates.is_empty() {
        RelocationMatch::InUse
    } else {
        pick_relocation(&candidates, size)
    }
}

/// Year → month → histories, newest first, for the Histories tree.
pub fn history_groups(list: &[HistoryInfo]) -> Vec<(String, Vec<(String, Vec<HistoryInfo>)>)> {
    let mut out: Vec<(String, Vec<(String, Vec<HistoryInfo>)>)> = vec![];
    for h in list {
        let year = h.date.get(..4).unwrap_or("").to_string();
        let month = h.date.get(5..7).unwrap_or("").to_string();
        if out.last().map(|y| y.0 != year).unwrap_or(true) {
            out.push((year, vec![]));
        }
        let months = &mut out.last_mut().unwrap().1;
        if months.last().map(|m| m.0 != month).unwrap_or(true) {
            months.push((month, vec![]));
        }
        months.last_mut().unwrap().1.push(h.clone());
    }
    out
}

pub fn month_name(mm: &str) -> &'static str {
    const M: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    mm.parse::<usize>().ok().and_then(|m| M.get(m.wrapping_sub(1))).copied().unwrap_or("")
}

/// Screenshot states for this module's screens (PREPDECK_SHOT_VIEW=<name>).
pub fn shot_view(app: &mut App, name: &str) {
    let some_tracks = |app: &App, n: usize| -> Vec<i64> { app.tracks.iter().filter(|t| !t.missing).take(n).map(|t| t.id).collect() };
    let ensure_playlist = |app: &mut App| -> i64 {
        if let Some(n) = app.nodes.iter().filter(|n| n.kind == NodeKind::Playlist).max_by_key(|n| app.node_counts.get(&n.id).copied().unwrap_or(0)).filter(|n| app.node_counts.get(&n.id).copied().unwrap_or(0) > 1) {
            return n.id;
        }
        let p = app.db.create_node(0, "Warm Up", NodeKind::Playlist, None).unwrap_or(0);
        let ids: Vec<i64> = app.tracks.iter().take(12).map(|t| t.id).collect();
        let _ = app.db.set_playlist_tracks(p, &ids);
        app.reload_playlists();
        p
    };
    match name {
        "infopanel" | "infopanel_info" | "infopanel_art" | "infopanel_multi" => {
            app.prefs.info_panel = true;
            app.ext.info_tab = match name {
                "infopanel_info" | "infopanel_multi" => 1,
                "infopanel_art" => 2,
                _ => 0,
            };
            app.rebuild_list();
            // (The loaded track is selected by the screenshot setup: one track, or three.)
            let n = if name == "infopanel_multi" { 2 } else { 0 };
            app.selected = app.list.iter().take(n).copied().collect();
        }
        "filterbar" => {
            app.filter_open = true;
            app.ext.filter = TrackFilter { bpm_on: true, bpm: 126.0, bpm_pct: 4.0, key_on: true, key: app.tracks.iter().find(|t| t.key >= 0).map(|t| t.key).unwrap_or(0), key_related: true, rating_on: false, rating: 3, color_on: false, colors: 0b110, tags_on: true };
            app.list_dirty = true;
        }
        "taglist" => {
            let ids = some_tracks(app, 6);
            app.tag_list_add(&ids);
            app.side = SideMode::Playlists;
            app.set_view(View::TagList);
        }
        "matching" => {
            let ids = some_tracks(app, 5);
            if let Some((a, rest)) = ids.split_first() {
                for b in rest {
                    app.matching_set(*a, *b, true);
                }
            }
            app.side = SideMode::Related;
            app.set_view(View::Matching);
        }
        "histories" => {
            for id in some_tracks(app, 4) {
                app.history_add_now(id);
            }
            if let Ok(h) = app.db.history_for_date("2026-10-03") {
                for id in some_tracks(app, 2) {
                    let _ = app.db.history_add(h, id, 0);
                }
            }
            app.ext.histories = app.db.histories().unwrap_or_default();
            app.side = SideMode::History;
            let first = app.ext.histories.first().map(|h| h.id).unwrap_or(0);
            app.set_view(View::History(first));
        }
        "missing" => {
            // Mark a few tracks missing in memory only (nothing is moved or saved).
            if !app.tracks.iter().any(|t| t.missing) {
                for t in app.tracks.iter_mut().take(4) {
                    t.missing = true;
                }
            }
            app.open_missing_file_manager();
            app.ext.missing_sel = app.tracks.iter().filter(|t| t.missing).take(1).map(|t| t.id).collect();
        }
        "palette" => {
            let p = ensure_playlist(app);
            app.prefs.palette_open = true;
            app.palette_assign(0, p);
            app.shortcut_add(p);
            app.side = SideMode::Playlists;
            app.set_view(View::Playlist(p));
        }
        "treesearch" => {
            ensure_playlist(app);
            app.side = SideMode::Playlists;
            app.ext.tree_search_open = true;
            app.ext.tree_search = std::env::var("PREPDECK_SHOT_QUERY").unwrap_or_else(|_| "a".into());
        }
        "inline" => {
            app.rebuild_list();
            if let Some(id) = app.list.first().copied() {
                app.selected = [id].into_iter().collect();
                let text = app.track(id).map(|t| t.artist.clone()).unwrap_or_default();
                app.ext.edit = Some(CellEdit { track: id, col: Col::Artist, text, focused: false });
            }
        }
        "colorpick" => {
            app.rebuild_list();
            if let Some(id) = app.list.get(2).copied() {
                app.ext.color_pick = Some((id, eframe::egui::pos2(420.0, 420.0), 0.0));
            }
        }
        "preview" => {
            app.rebuild_list();
            let pick = app.list.iter().copied().find(|id| app.track(*id).map(|t| !t.missing && t.duration > 60.0).unwrap_or(false));
            if let Some(id) = pick {
                app.preview_play(id, 0.4);
            }
        }
        "dupes" => {
            let p = ensure_playlist(app);
            let ids = app.db.playlist_tracks(p).unwrap_or_default();
            app.side = SideMode::Playlists;
            app.set_view(View::Playlist(p));
            app.add_to_playlist(p, &ids[..ids.len().min(2)]);
        }
        "renumber" => {
            let p = ensure_playlist(app);
            app.side = SideMode::Playlists;
            app.set_view(View::Playlist(p));
            app.prefs.sort_col = Col::Title;
            app.prefs.sort_asc = true;
            app.rebuild_list();
            if std::env::var("PREPDECK_SHOT_RENUMBER").is_ok() {
                let ok = app.renumber_track_order(p);
                eprintln!("renumber: {ok}; stored order now {:?}", app.db.playlist_tracks(p).unwrap_or_default().iter().take(5).collect::<Vec<_>>());
            }
        }
        "itunes" | "rbxml" => {
            let kind = if name == "itunes" { SRC_ITUNES } else { SRC_RBXML };
            if let Ok(p) = std::env::var("PREPDECK_SHOT_SOURCE") {
                if kind == SRC_ITUNES {
                    app.prefs.itunes_xml = Some(p);
                } else {
                    app.prefs.rekordbox_xml_path = Some(p);
                }
            }
            app.side = if kind == SRC_ITUNES { SideMode::Itunes } else { SideMode::RekordboxXml };
            app.source_reload(kind);
            let t0 = std::time::Instant::now();
            while app.ext.sources[kind as usize].rx.is_some() && t0.elapsed() < std::time::Duration::from_secs(20) {
                app.poll_library_ext();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let key = std::env::var("PREPDECK_SHOT_NODE").unwrap_or_default();
            app.ext.sources[kind as usize].open.insert("0".into());
            app.view = View::Source { kind, node: key };
        }
        "related_lists" => {
            if app.prefs.related_lists.len() < 2 {
                app.related_new(true);
                app.prefs.related.matching = false;
                app.prefs.related.genre = true;
            }
            app.ext.criteria_open = true;
            app.side = SideMode::Related;
            app.view = View::Related;
        }
        "statusicons" => {
            // Every attribute icon at once (in memory only).
            app.rebuild_list();
            let ids: Vec<i64> = app.list.iter().take(8).copied().collect();
            for (k, id) in ids.iter().enumerate() {
                if let Some(t) = app.track_mut(*id) {
                    match k {
                        1 => t.analyzed = 0,
                        2 => t.analyzed = 3,
                        3 => t.missing = true,
                        4 => t.analyzed = 2,
                        _ => {}
                    }
                }
            }
            if let Some(id) = ids.get(5) {
                app.tag_list_add(&[*id]);
            }
        }
        "artfull" => {
            app.prefs.artwork_full = true;
            if !app.prefs.columns.contains(&Col::Artwork) {
                app.prefs.columns.insert(0, Col::Artwork);
            }
        }
        "libcheck" => {
            // End-to-end check of the playlist files, Sort Items, the Tag List, histories and
            // the clipboard on the scratch library; results go to stderr.
            let p = ensure_playlist(app);
            let dir = std::path::PathBuf::from(std::env::var("PREPDECK_SHOT_OUT").unwrap_or_else(|_| std::env::temp_dir().join("recordbox_libcheck").to_string_lossy().to_string()));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::create_dir_all(&dir);
            let want = app.view_tracks_of(&View::Playlist(p));
            let folder = app.db.create_node(0, "Round Trip", NodeKind::Folder, None).unwrap_or(0);
            app.reload_playlists();
            for ext in ["m3u8", "pls", "txt"] {
                let f = dir.join(format!("check.{ext}"));
                let ok = app.export_playlist_file(p, &f);
                let back = app.import_playlist_file(&f, folder);
                let got = back.map(|b| app.db.playlist_tracks(b).unwrap_or_default()).unwrap_or_default();
                eprintln!("libcheck {ext}: wrote {ok}, {} bytes, read back {} of {} tracks, same order: {}", std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0), got.len(), want.len(), got == want);
            }
            let n = app.export_all_playlists_info(&dir.join("all"));
            eprintln!("libcheck export all: {n} files");
            let kids = |app: &App| -> Vec<String> { app.nodes.iter().filter(|n| n.parent == folder).map(|n| n.name.clone()).collect() };
            let before = kids(app);
            let last = app.nodes.iter().filter(|n| n.parent == folder).last().map(|n| n.id).unwrap_or(0);
            let _ = app.db.rename_node(last, "Aardvark");
            app.reload_playlists();
            app.sort_node_children(folder);
            eprintln!("libcheck sort items: {before:?} -> {:?}", kids(app));
            let ids = some_tracks(app, 3);
            app.tag_list_add(&ids);
            app.tag_list_add(&ids);
            let saved = app.tag_list_save_as_playlist().map(|pl| app.db.playlist_tracks(pl).unwrap_or_default());
            eprintln!("libcheck tag list: {} entries, saved playlist same: {}", app.ext.tag_list.len(), saved.as_ref() == Some(&ids));
            let plays = app.track(ids[0]).map(|t| t.play_count).unwrap_or(0);
            app.history_add_now(ids[0]);
            let h = app.ext.histories.first().cloned();
            eprintln!("libcheck history: {:?}, play count {} -> {}", h.as_ref().map(|h| (&h.name, h.count)), plays, app.track(ids[0]).map(|t| t.play_count).unwrap_or(0));
            if let Some(h) = h {
                let pl = app.history_save_as_playlist(h.id);
                eprintln!("libcheck history as playlist: {:?}", pl.map(|pl| app.db.playlist_tracks(pl).unwrap_or_default().len()));
                app.history_delete(h.id);
                eprintln!("libcheck histories after delete: {}", app.ext.histories.len());
            }
            // Copy three tracks from the playlist and paste them into a new one.
            app.side = SideMode::Playlists;
            app.set_view(View::Playlist(p));
            app.rebuild_list();
            app.selected = app.list.iter().take(3).copied().collect();
            let copied: Vec<i64> = app.list.iter().take(3).copied().collect();
            app.copy_tracks(false);
            let dest = app.db.create_node(folder, "Pasted", NodeKind::Playlist, None).unwrap_or(0);
            app.reload_playlists();
            app.set_view(View::Playlist(dest));
            app.rebuild_list();
            app.selected.clear();
            app.paste_tracks();
            eprintln!("libcheck paste: {}", app.db.playlist_tracks(dest).unwrap_or_default() == copied);
            // In-place edit through the Undo stack.
            let id = copied[0];
            let old = app.track(id).map(|t| t.comment.clone()).unwrap_or_default();
            app.ext.edit = Some(CellEdit { track: id, col: Col::Comments, text: "edited in place".into(), focused: true });
            app.commit_cell_edit();
            let mid = app.db.track(id).map(|t| t.comment).unwrap_or_default();
            app.undo_edit();
            eprintln!("libcheck edit: {old:?} -> {mid:?} -> undo {:?}", app.db.track(id).map(|t| t.comment).unwrap_or_default());
            app.matching_set(copied[0], copied[1], true);
            eprintln!("libcheck matching: {:?} / {:?}", app.matching_of(copied[0]) == vec![copied[1]], app.matching_of(copied[1]) == vec![copied[0]]);
            app.dialogs.rename = None;
            app.expanded.insert(folder);
        }
        // Browser states of the 7.2.19 references (combine with commas after "playlists").
        "tree364" => app.prefs.bx.tree_w = 364.0,
        "refcols" => {
            // Column widths of the 7.2.19 reference screenshots.
            for (c, w) in [(Col::Preview, 200.0), (Col::Artwork, 80.0), (Col::Title, 245.0), (Col::Artist, 128.0), (Col::Genre, 128.0), (Col::Bpm, 80.0), (Col::Time, 79.0)] {
                app.prefs.widths.insert(c.key(), w);
            }
        }
        "rbroot" => {
            // The synced rekordbox tree without its "rekordbox" folder (scratch libraries only),
            // as the references show the original's own tree.
            if let Some(f) = app.nodes.iter().find(|n| n.parent == 0 && n.kind == NodeKind::Folder && n.name == "rekordbox").map(|n| n.id) {
                let kids: Vec<i64> = app.nodes.iter().filter(|n| n.parent == f).map(|n| n.id).collect();
                for (i, k) in kids.iter().enumerate() {
                    let _ = app.db.move_node(*k, 0, i as i64);
                }
                let _ = app.db.delete_node(f);
                app.reload_playlists();
            }
        }
        "row2" => {
            app.rebuild_list();
            if let Some(id) = app.list.get(1).copied() {
                app.selected = [id].into_iter().collect();
                app.anchor_id = Some(id);
            }
        }
        "relwin" => crate::ui_browser_ext::set_right_panel(app, 2),
        "sugwin" => crate::ui_browser_ext::set_right_panel(app, 3),
        "infowin" => crate::ui_browser_ext::set_right_panel(app, 4),
        "subv" | "subh" => {
            if name == "subv" {
                crate::ui_browser_ext::set_right_panel(app, 5);
            } else {
                crate::ui_browser_ext::set_right_panel(app, 0);
                app.prefs.bx.sub_horizontal = true;
            }
            if let View::Playlist(id) = app.view {
                app.sub_view = View::Playlist(id);
            }
        }
        "colview" => app.prefs.bx.column_view = true,
        "suggest" => {
            app.side = SideMode::Suggest;
            app.view = View::Suggest(0);
            app.list_dirty = true;
        }
        "browseronly" => app.prefs.browser_only = true,
        "cuestart" => {
            // End-to-end check of the CUE Analysis Playlist on the scratch library: register
            // the tracks that have cues, press [Start], and report the tendency.
            if let Some(pl) = app.cue_playlist() {
                let ids: Vec<i64> = app.tracks.iter().filter(|t| t.analyzed == 1 && app.cue_counts.get(&t.id).map(|c| c.0 + c.1 > 0).unwrap_or(false)).take(40).map(|t| t.id).collect();
                let _ = app.db.set_playlist_tracks(pl, &ids);
                app.reload_playlists();
                app.cue_analysis_start();
                eprintln!("cue analysis: {} tracks registered, tendency {:?}", ids.len(), app.prefs.bx.cue_tendency);
            } else {
                eprintln!("cue analysis: no CUE Analysis Playlist");
            }
        }
        "datecreated" => {
            if !app.prefs.columns.contains(&Col::DateCreated) {
                app.prefs.columns.push(Col::DateCreated);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(bpm: f64, key: i32, rating: u8, color: u8) -> Track {
        Track { bpm, key, rating, color, ..Default::default() }
    }

    #[test]
    fn filter_maths() {
        // BPM ± range: 126 ±4% is 120.96–131.04; 0% is that whole BPM.
        let (lo, hi) = bpm_window(126.0, 4.0);
        assert!((lo - 120.96).abs() < 1e-9 && (hi - 131.04).abs() < 1e-9);
        let mut f = TrackFilter { bpm_on: true, bpm: 126.0, bpm_pct: 4.0, ..Default::default() };
        assert!(filter_pass(&f, &track(121.0, 0, 0, 0)));
        assert!(filter_pass(&f, &track(131.04, 0, 0, 0)));
        assert!(!filter_pass(&f, &track(131.1, 0, 0, 0)));
        assert!(!filter_pass(&f, &track(0.0, 0, 0, 0)));
        f.bpm_pct = 0.0;
        assert!(filter_pass(&f, &track(126.4, 0, 0, 0)) && !filter_pass(&f, &track(126.6, 0, 0, 0)));
        // Switched off: everything passes, whatever the values say.
        f.bpm_on = false;
        assert!(filter_pass(&f, &track(60.0, 0, 0, 0)));
        // Key: exact, or with related keys (±1 on the wheel and the relative key).
        let mut f = TrackFilter { key_on: true, key: 0, ..Default::default() };
        assert!(filter_pass(&f, &track(0.0, 0, 0, 0)) && !filter_pass(&f, &track(0.0, 1, 0, 0)));
        f.key_related = true;
        assert!(filter_pass(&f, &track(0.0, 1, 0, 0)) && filter_pass(&f, &track(0.0, 11, 0, 0)) && filter_pass(&f, &track(0.0, 12, 0, 0)));
        assert!(!filter_pass(&f, &track(0.0, 5, 0, 0)) && !filter_pass(&f, &track(0.0, -1, 0, 0)));
        // Rating: that many stars or more.
        let f = TrackFilter { rating_on: true, rating: 3, ..Default::default() };
        assert!(filter_pass(&f, &track(0.0, 0, 3, 0)) && filter_pass(&f, &track(0.0, 0, 5, 0)) && !filter_pass(&f, &track(0.0, 0, 2, 0)));
        // Colours: any of the chosen ones; none chosen filters nothing.
        let mut f = TrackFilter { color_on: true, colors: (1 << 2) | (1 << 5), ..Default::default() };
        assert!(filter_pass(&f, &track(0.0, 0, 0, 2)) && filter_pass(&f, &track(0.0, 0, 0, 5)));
        assert!(!filter_pass(&f, &track(0.0, 0, 0, 3)) && !filter_pass(&f, &track(0.0, 0, 0, 0)));
        f.colors = 0;
        assert!(filter_pass(&f, &track(0.0, 0, 0, 3)));
    }

    #[test]
    fn cell_edits() {
        let mut t = Track::default();
        apply_cell_edit(&mut t, Col::Title, "  New name ").unwrap();
        assert_eq!(t.title, "New name");
        apply_cell_edit(&mut t, Col::Year, "2024").unwrap();
        assert_eq!(t.year, 2024);
        assert!(apply_cell_edit(&mut t, Col::Year, "next year").is_err());
        assert_eq!(t.year, 2024);
        apply_cell_edit(&mut t, Col::Year, "").unwrap();
        assert_eq!(t.year, 0);
        apply_cell_edit(&mut t, Col::Bpm, "127,5").unwrap();
        assert_eq!(t.bpm, 127.5);
        assert_eq!(apply_cell_edit(&mut t, Col::Bpm, "12").unwrap_err(), "BPM value should be between 40 and 499.");
        apply_cell_edit(&mut t, Col::Key, "8A").unwrap();
        assert!(t.key >= 0 && t.key_manual);
        assert_eq!(edit_text(&t, Col::Key, true), "8A");
        assert!(apply_cell_edit(&mut t, Col::Key, "loud").is_err());
        apply_cell_edit(&mut t, Col::Key, "").unwrap();
        assert!(t.key < 0 && !t.key_manual);
        assert!(apply_cell_edit(&mut t, Col::Time, "1:00").is_err());
        assert!(editable(Col::Comments) && !editable(Col::Time) && !editable(Col::Location) && !editable(Col::Rating));
        // Every editable column round-trips through its editor text.
        for c in ALL_COLS.into_iter().filter(|c| editable(*c) && !matches!(c, Col::Bpm | Col::Key | Col::Year | Col::TrackNumber | Col::DiscNumber)) {
            apply_cell_edit(&mut t, c, "abc").unwrap();
            assert_eq!(edit_text(&t, c, false), "abc", "{c:?}");
        }
    }

    #[test]
    fn play_credit() {
        let mut c = PlayCredit::default();
        // Not playing: no time counts.
        assert_eq!(c.tick(Some(7), false, 0.1, 300.0), None);
        let mut got = None;
        let mut frames = 0;
        while got.is_none() && frames < 10_000 {
            got = c.tick(Some(7), true, 0.1, 300.0);
            frames += 1;
        }
        // About a minute of play (600 frames of 0.1 s), reported once.
        assert_eq!(got, Some(7));
        assert!((599..=601).contains(&frames), "{frames}");
        assert_eq!(c.tick(Some(7), true, 0.1, 300.0), None);
        // Loading the same track again starts a new count; another track resets by itself.
        c.arm(7);
        assert!(!c.done && c.secs == 0.0);
        assert_eq!(c.tick(Some(8), true, 0.1, 300.0), None);
        assert_eq!(c.track, Some(8));
        // A short track counts after 90% of its length; a stalled frame adds at most 0.5 s.
        let mut c = PlayCredit::default();
        assert_eq!(c.tick(Some(1), true, 30.0, 8.0), None);
        assert_eq!(c.secs, 0.5);
        let mut n = 1;
        while c.tick(Some(1), true, 0.5, 8.0).is_none() {
            n += 1;
        }
        assert_eq!(n + 1, 15);
        // Nothing loaded: nothing to credit.
        assert_eq!(c.tick(None, true, 1.0, 0.0), None);
    }

    #[test]
    fn renumber_and_sort_items() {
        assert_eq!(renumbered(&[1, 2, 3], &[3, 1, 2]), Some(vec![3, 1, 2]));
        // Unchanged order, or a filtered list, renumbers nothing.
        assert_eq!(renumbered(&[1, 2, 3], &[1, 2, 3]), None);
        assert_eq!(renumbered(&[1, 2, 3], &[3, 1]), None);
        // Duplicates in a playlist are kept.
        assert_eq!(renumbered(&[1, 2, 1], &[1, 1, 2]), Some(vec![1, 1, 2]));
        let items = vec![(1, "Set 10".to_string()), (2, "set 2".to_string()), (3, "Älbum".to_string()), (4, "Zebra".to_string()), (5, "Beta".to_string())];
        assert_eq!(sorted_by_name(&items), [3, 5, 2, 1, 4]);
        assert_eq!(unique_name("A", &["B".into()]), "A");
        assert_eq!(unique_name("A", &["A".into(), "A (2)".into()]), "A (3)");
    }

    #[test]
    fn history_bookkeeping() {
        let dir = std::env::temp_dir().join(format!("recordbox_hist_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("library.db")).unwrap();
        let mut ids = vec![];
        for n in 0..3 {
            ids.push(db.insert_track(&Track { path: format!("/x/{n}.mp3"), title: format!("T{n}"), ..Default::default() }).unwrap());
        }
        // One history per day; the same day is reused.
        let h1 = db.history_for_date("2026-11-17").unwrap();
        assert_eq!(db.history_for_date("2026-11-17").unwrap(), h1);
        let h2 = db.history_for_date("2026-10-03").unwrap();
        let h3 = db.history_for_date("2025-12-31").unwrap();
        db.history_add(h1, ids[0], 1).unwrap();
        db.history_add(h1, ids[1], 2).unwrap();
        db.history_add(h1, ids[0], 3).unwrap();
        db.history_add(h2, ids[2], 4).unwrap();
        // Plays in order, repeats kept.
        assert_eq!(db.history_tracks(h1).unwrap(), [ids[0], ids[1], ids[0]]);
        let list = db.histories().unwrap();
        assert_eq!(list.iter().map(|h| (h.name.as_str(), h.count)).collect::<Vec<_>>(), [("HISTORY 2026-11-17", 3), ("HISTORY 2026-10-03", 1), ("HISTORY 2025-12-31", 0)]);
        let groups = history_groups(&list);
        assert_eq!(groups.len(), 2);
        assert_eq!((groups[0].0.as_str(), groups[0].1.len(), groups[0].1[0].0.as_str(), month_name(&groups[0].1[1].0)), ("2026", 2, "11", "October"));
        // Remove from History, Delete History.
        db.history_remove_tracks(h1, &[ids[0]]).unwrap();
        assert_eq!(db.history_tracks(h1).unwrap(), [ids[1]]);
        db.history_delete(h2).unwrap();
        assert_eq!(db.histories().unwrap().iter().map(|h| h.id).collect::<Vec<_>>(), [h1, h3]);
        // A track removed from the collection leaves the histories.
        db.delete_track(ids[1]).unwrap();
        assert!(db.history_tracks(h1).unwrap().is_empty());
        db.history_delete_all().unwrap();
        assert!(db.histories().unwrap().is_empty());

        // Tag List order and matches are kept in the same library file.
        db.set_tag_list(&[ids[2], ids[0]]).unwrap();
        assert_eq!(db.tag_list().unwrap(), [ids[2], ids[0]]);
        db.set_match(ids[2], ids[0], true).unwrap();
        db.set_match(ids[0], ids[2], true).unwrap();
        let m = db.track_matches().unwrap();
        assert_eq!((m[&ids[0]].clone(), m[&ids[2]].clone()), (vec![ids[2]], vec![ids[0]]));
        db.set_match(ids[0], ids[2], false).unwrap();
        assert!(db.track_matches().unwrap().is_empty());
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scans_names_for_relocation() {
        let dir = std::env::temp_dir().join(format!("recordbox_reloc_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/Song One.MP3"), b"12345").unwrap();
        std::fs::write(dir.join("a/other.mp3"), b"1").unwrap();
        // The same folder twice (or nested search folders) lists a file once.
        let m = scan_file_names(&[dir.clone(), dir.join("a")]);
        let hits = &m[&crate::smart::fold("song one.mp3")];
        assert_eq!(hits.len(), 1);
        assert_eq!(pick_relocation(&[(hits[0].as_path(), Some(5))], 5), RelocationMatch::Found(&hits[0], 5));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relocation_uses_file_evidence_and_preserves_track_ownership() {
        use RelocationMatch::*;
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("Café Mix.wav");
        std::fs::write(&audio, b"abcde").unwrap();
        let original = Track { path: r"C:\old\contents_42\Café Mix.wav".into(), title: "My edit".into(),
                               file_size: 5, rating: 4, ..Default::default() };
        let db = Db::open(Path::new(":memory:")).unwrap();
        let id = db.insert_track(&original).unwrap();
        let other_path = "/Volumes/USB/Café Mix.wav";
        let other = db.insert_track(&Track { path: other_path.into(), ..original.clone() }).unwrap();
        let index = scan_file_names(&[dir.path().to_path_buf()]);
        let paths = &index[&crate::smart::fold(&original.file_name())];
        assert_eq!(match_scanned_files(&db, paths, id, 6), SizeMismatch);
        assert_eq!(match_scanned_files(&db, &[], id, 5), NoMatch);
        assert_eq!(match_scanned_files(&db, paths, id, 5), Found(&audio, 5));

        let first = Track { id, ..original.clone() };
        let second = Track { id: other, path: other_path.into(), ..original };
        for order in [[&first, &second], [&second, &first]] {
            let results = relocation_matches(&db, &index, order.into_iter());
            assert_eq!(results, [(order[0].id, Shared), (order[1].id, Shared)]);
        }
        assert_eq!(relocation_matches(&db, &index, [&first].into_iter()), [(id, Found(&audio, 5))]);
        let different = Track { file_size: 9, ..second };
        assert_eq!(relocation_matches(&db, &index, [&first, &different].into_iter()),
                   [(id, Found(&audio, 5)), (other, SizeMismatch)]);

        db.set_track_path(id, audio.to_str().unwrap()).unwrap();
        let stored = db.track(id).unwrap();
        assert_eq!((stored.path.as_str(), stored.title.as_str(), stored.rating), (audio.to_str().unwrap(), "My edit", 4));
        assert_eq!(match_scanned_files(&db, paths, id, 5), Found(&audio, 5));
        assert_eq!(match_scanned_files(&db, paths, other, 5), InUse);
        assert!(db.set_track_path(other, audio.to_str().unwrap()).is_err());
        assert_eq!(db.track(other).unwrap().path, other_path);
        assert!(db.set_track_path(-1, "/absent.wav").is_err());
        assert_eq!(std::fs::read(&audio).unwrap(), b"abcde");

        // The index can outlive a mounted file or directory entry.
        std::fs::remove_file(&audio).unwrap();
        assert_eq!(match_scanned_files(&db, paths, id, 0), Unverified);
        std::fs::create_dir(&audio).unwrap();
        assert_eq!(match_scanned_files(&db, paths, id, 5), Unverified);
        #[cfg(unix)]
        {
            std::fs::remove_dir(&audio).unwrap();
            let target = dir.path().join("other.wav");
            std::fs::write(&target, b"abcde").unwrap();
            std::os::unix::fs::symlink(target, &audio).unwrap();
            assert_eq!(match_scanned_files(&db, paths, id, 5), Unverified);
        }
    }
}
