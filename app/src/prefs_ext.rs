//! Preferences that came with the rekordbox-style Preferences window, the keyboard action
//! table and its dispatcher, and small helpers the menus share.
//!
//! Who reads which preference (all are fields of `Prefs` in app.rs):
//! - `tree_items`: the icon strip in ui_browser.rs (`Prefs::tree_item(name)`).
//! - `mytag_to_comments`: `App::set_tag` calls `sync_mytag_comments`.
//! - `cue_markers_on_preview`: the Preview column in ui_browser.rs.
//! - `wave_half`: the enlarged and overview waveforms in ui_player.rs.
//! - `color_names`: every colour menu, through `Prefs::color_name`; the device colour table.
//! - `use_recordbox_estimator`, `refine_beatgrid_transients`, `use_installed_beatgrid_model`, `analysis_mode`: the beat grid analysis settings.
//! - `memory_cue_first_beat`: `App::apply_analysis` calls `memory_cue_at_first_beat`.
//! - `audio_buffer`, `audio_rate`, `audio_channel`: the output stream (audio.rs).
//! - `device_bg_color`, `delete_history_after_import`: the USB code.
//! - `hot_cue_auto_load_on_import`: `App::poll_import`.
//! - `palette_enabled`, `playlist_search_filter`, `all_tracks_in_playlists`,
//!   `playlist_track_counts`: the browser (ui_browser.rs).
//! - `metronome_sound`: the metronome click in audio.rs, through `click_hz`.
//! - `preview_volume`: the preview player's volume (`App::tick_ext` in library_ext.rs).
//! - `stem_reset_on_load`: `App::load_track`. `stem_solo`: `stem_part_click` here, which the
//!   stem strip is to call. `stem_waveform`, `phrase_enlarged`, `phrase_types_always`,
//!   `vocal_enlarged`: no reader yet (their controls say so; see the *_WIRED constants in
//!   ui_prefs.rs).

use crate::app::*;
use crate::model::*;

pub fn default_color_names() -> [String; 8] {
    std::array::from_fn(|i| COLOR_NAMES[i + 1].to_string())
}

/// Every source tile of the browser's icon strip, in its order.
#[allow(dead_code)]
pub const TREE_ITEMS: [&str; 11] = ["Collection", "Playlists", "Related Tracks", "Matching", "Tag List", "Hot Cue Bank List", "iTunes", "rekordbox xml", "Explorer", "Devices", "Histories"];

/// Preferences > View > Layout > Media Browser as rekordbox 7.2.19 lays it out: the left and
/// the right column, top to bottom. Hot Cue Bank List (EXPORT mode only in rekordbox) takes
/// the free place at the end of the right column.
pub const MEDIA_BROWSER: [[&str; 7]; 2] = [
    ["Related Tracks", "Apple Music", "SoundCloud", "Tidal", "Track Suggestion", "Explorer", "CloudDirectPlay Filter"],
    ["iTunes", "Spotify", "Beatport", "Inflyte", "rekordbox xml", "SEARCH MOBILE button", "Hot Cue Bank List"],
];

/// Source tiles Recordbox has that rekordbox always shows (it has no checkbox for them).
pub const TREE_ITEMS_OWN: [&str; 5] = ["Playlists", "Matching", "Tag List", "Devices", "Histories"];

/// Why a Media Browser item of rekordbox cannot be ticked in Recordbox.
pub fn media_absent_reason(name: &str) -> String {
    match name {
        "Apple Music" | "SoundCloud" | "Tidal" | "Spotify" | "Beatport" | "Inflyte" => format!("recordbox has no streaming services, so there is no {name} source."),
        "Track Suggestion" => "Track Suggestion is not available.".to_string(),
        "CloudDirectPlay Filter" => "recordbox has no cloud service, so there is nothing to filter.".to_string(),
        "SEARCH MOBILE button" => "recordbox has no mobile device search.".to_string(),
        _ => format!("recordbox has no {name} source yet."),
    }
}

/// Tree View items Recordbox has a source for. The others are shown disabled in Preferences
/// until their source exists (add the name here when it does).
pub fn tree_item_available(name: &str) -> bool {
    matches!(name, "Collection" | "Playlists" | "Related Tracks" | "Explorer" | "Devices" | "Histories") || TREE_ITEMS_EXTRA.contains(&name)
}
/// Sources added by the library code since (kept apart so adding one is a one-word edit).
pub const TREE_ITEMS_EXTRA: &[&str] = &["Matching", "Tag List", "iTunes", "rekordbox xml", "Hot Cue Bank List", "Track Suggestion", "Recordings"];

impl Prefs {
    /// Whether a Tree View source is shown. Collection always is.
    pub fn tree_item(&self, name: &str) -> bool {
        // As in rekordbox, Tag List, Hot Cue Bank List and rekordbox xml are off until ticked in
        // Preferences > View > Layout.
        name == "Collection" || self.tree_items.get(name).copied().unwrap_or(!matches!(name, "Tag List" | "Hot Cue Bank List" | "rekordbox xml"))
    }
    /// The name of track colour `i` (0 = "No Color", 1..=8 the user's Color comments).
    pub fn color_name(&self, i: u8) -> String {
        match i {
            1..=8 => {
                let n = self.color_names[i as usize - 1].trim();
                if n.is_empty() { COLOR_NAMES[i as usize].to_string() } else { n.to_string() }
            }
            _ => COLOR_NAMES[0].to_string(),
        }
    }
}

// ------------------------------------------------------------------ keyboard actions

/// (id, label, default shortcut in stored form or "" for none, category).
pub const ACTIONS: [(&str, &str, &str, &str); 145] = [
    ("play", "Play/Pause", "Space", "Player"),
    ("cue", "Cue", "C", "Player"),
    ("jump_back", "Jump Reverse", "ArrowLeft", "Player"),
    ("jump_fwd", "Jump Forward", "ArrowRight", "Player"),
    ("search_back", "Search Backward", "Comma", "Player"),
    ("search_fwd", "Search Forward", "Period", "Player"),
    ("prev_track", "Previous Track", "Alt+ArrowUp", "Player"),
    ("next_track", "Next Track", "Alt+ArrowDown", "Player"),
    ("vol_up", "Volume Up", "Equals", "Player"),
    ("vol_down", "Volume Down", "Minus", "Player"),
    ("zoom_in", "Zoom In", "Shift+ArrowUp", "Player"),
    ("zoom_out", "Zoom Out", "Shift+ArrowDown", "Player"),
    ("zoom_reset", "Reset Zoom", "Shift+0", "Player"),
    ("time_mode", "Switch Time/Remain", "T", "Player"),
    ("quantize", "Quantize ON/OFF", "Q", "Player"),
    ("hot1", "Set/Play Hot Cue A", "1", "Hot Cue"),
    ("hot2", "Set/Play Hot Cue B", "2", "Hot Cue"),
    ("hot3", "Set/Play Hot Cue C", "3", "Hot Cue"),
    ("hot4", "Set/Play Hot Cue D", "4", "Hot Cue"),
    ("hot5", "Set/Play Hot Cue E", "5", "Hot Cue"),
    ("hot6", "Set/Play Hot Cue F", "6", "Hot Cue"),
    ("hot7", "Set/Play Hot Cue G", "7", "Hot Cue"),
    ("hot8", "Set/Play Hot Cue H", "8", "Hot Cue"),
    ("clear_hot1", "Clear Hot Cue A", "Shift+1", "Hot Cue"),
    ("clear_hot2", "Clear Hot Cue B", "Shift+2", "Hot Cue"),
    ("clear_hot3", "Clear Hot Cue C", "Shift+3", "Hot Cue"),
    ("clear_hot4", "Clear Hot Cue D", "Shift+4", "Hot Cue"),
    ("clear_hot5", "Clear Hot Cue E", "Shift+5", "Hot Cue"),
    ("clear_hot6", "Clear Hot Cue F", "Shift+6", "Hot Cue"),
    ("clear_hot7", "Clear Hot Cue G", "Shift+7", "Hot Cue"),
    ("clear_hot8", "Clear Hot Cue H", "Shift+8", "Hot Cue"),
    ("memory", "Memory Cue", "M", "Memory Cue"),
    ("mem_next", "Call Next Memory Cue", "Alt+ArrowRight", "Memory Cue"),
    ("mem_prev", "Call Previous Memory Cue", "Alt+ArrowLeft", "Memory Cue"),
    ("mem_call1", "Memory Cue 1", "Alt+1", "Memory Cue"),
    ("mem_call2", "Memory Cue 2", "Alt+2", "Memory Cue"),
    ("mem_call3", "Memory Cue 3", "Alt+3", "Memory Cue"),
    ("mem_call4", "Memory Cue 4", "Alt+4", "Memory Cue"),
    ("mem_call5", "Memory Cue 5", "Alt+5", "Memory Cue"),
    ("mem_call6", "Memory Cue 6", "Alt+6", "Memory Cue"),
    ("mem_call7", "Memory Cue 7", "Alt+7", "Memory Cue"),
    ("mem_call8", "Memory Cue 8", "Alt+8", "Memory Cue"),
    ("mem_call9", "Memory Cue 9", "Alt+9", "Memory Cue"),
    ("mem_call10", "Memory Cue 10", "Alt+0", "Memory Cue"),
    ("mem_delete", "Delete Memory Cue", "Alt+M", "Memory Cue"),
    ("mem_delete1", "Delete Memory Cue 1", "", "Memory Cue"),
    ("mem_delete2", "Delete Memory Cue 2", "", "Memory Cue"),
    ("mem_delete3", "Delete Memory Cue 3", "", "Memory Cue"),
    ("mem_delete4", "Delete Memory Cue 4", "", "Memory Cue"),
    ("mem_delete5", "Delete Memory Cue 5", "", "Memory Cue"),
    ("mem_delete6", "Delete Memory Cue 6", "", "Memory Cue"),
    ("mem_delete7", "Delete Memory Cue 7", "", "Memory Cue"),
    ("mem_delete8", "Delete Memory Cue 8", "", "Memory Cue"),
    ("mem_delete9", "Delete Memory Cue 9", "", "Memory Cue"),
    ("mem_delete10", "Delete Memory Cue 10", "", "Memory Cue"),
    ("loop_in", "Loop In", "I", "Loop"),
    ("loop_out", "Loop Out", "O", "Loop"),
    ("reloop", "Exit/Reloop", "P", "Loop"),
    ("auto_loop", "4 Beat Loop", "L", "Loop"),
    ("loop_halve", "Halve the loop", "OpenBracket", "Loop"),
    ("loop_double", "Double the loop", "CloseBracket", "Loop"),
    ("grid_left", "Move the beat grid to the left", "Alt+Comma", "Grid"),
    ("grid_right", "Move the beat grid to the right", "Alt+Period", "Grid"),
    ("grid_downbeat", "Set the first beat of a bar to the current position.", "Alt+B", "Grid"),
    ("grid_tap", "Set BPM with the tapping interval", "Alt+T", "Grid"),
    // The sampler deck (PERFORMANCE mode): rekordbox's `SamplerControlPanel` commands with the
    // keys of its "Performance 1" preset (`getCommandInfo` 0x100b15b8c).
    ("smp_play1", "Playback slot 1", "X", "Sampler"),
    ("smp_play2", "Playback slot 2", "C", "Sampler"),
    ("smp_play3", "Playback slot 3", "V", "Sampler"),
    ("smp_play4", "Playback slot 4", "B", "Sampler"),
    ("smp_play5", "Playback slot 5", "", "Sampler"),
    ("smp_play6", "Playback slot 6", "", "Sampler"),
    ("smp_play7", "Playback slot 7", "", "Sampler"),
    ("smp_play8", "Playback slot 8", "", "Sampler"),
    ("smp_play9", "Playback slot 9", "M", "Sampler"),
    ("smp_play10", "Playback slot 10", "Comma", "Sampler"),
    ("smp_play11", "Playback slot 11", "Period", "Sampler"),
    ("smp_play12", "Playback slot 12", "Slash", "Sampler"),
    ("smp_play13", "Playback slot 13", "", "Sampler"),
    ("smp_play14", "Playback slot 14", "", "Sampler"),
    ("smp_play15", "Playback slot 15", "", "Sampler"),
    ("smp_play16", "Playback slot 16", "", "Sampler"),
    ("smp_pause1", "Pause slot 1", "Shift+X", "Sampler"),
    ("smp_pause2", "Pause slot 2", "Shift+C", "Sampler"),
    ("smp_pause3", "Pause slot 3", "Shift+V", "Sampler"),
    ("smp_pause4", "Pause slot 4", "Shift+B", "Sampler"),
    ("smp_pause5", "Pause slot 5", "", "Sampler"),
    ("smp_pause6", "Pause slot 6", "", "Sampler"),
    ("smp_pause7", "Pause slot 7", "", "Sampler"),
    ("smp_pause8", "Pause slot 8", "", "Sampler"),
    ("smp_pause9", "Pause slot 9", "Shift+M", "Sampler"),
    ("smp_pause10", "Pause slot 10", "Shift+Comma", "Sampler"),
    ("smp_pause11", "Pause slot 11", "Shift+Period", "Sampler"),
    ("smp_pause12", "Pause slot 12", "Shift+Slash", "Sampler"),
    ("smp_pause13", "Pause slot 13", "", "Sampler"),
    ("smp_pause14", "Pause slot 14", "", "Sampler"),
    ("smp_pause15", "Pause slot 15", "", "Sampler"),
    ("smp_pause16", "Pause slot 16", "", "Sampler"),
    ("smp_bank1", "Select Sampler Bank 1", "", "Sampler"),
    ("smp_bank2", "Select Sampler Bank 2", "", "Sampler"),
    ("smp_bank3", "Select Sampler Bank 3", "", "Sampler"),
    ("smp_bank4", "Select Sampler Bank 4", "", "Sampler"),
    ("smp_gain_up", "Increase Sampler Gain value", "", "Sampler"),
    ("smp_gain_down", "Decrease Sampler Gain value", "", "Sampler"),
    ("smp_vol_up", "Increase Sampler Gain volume", "", "Sampler"),
    ("smp_vol_down", "Decrease Sampler Gain volume", "", "Sampler"),
    ("smp_quantize", "Quantize (Sampler)", "", "Sampler"),
    ("smp_sync", "Sampler Sync", "", "Sampler"),
    ("smp_master", "Sampler Sync Master", "", "Sampler"),
    ("smp_bpm_up", "Increase Sampler BPM", "", "Sampler"),
    ("smp_bpm_up_large", "Increase Sampler BPM (large)", "", "Sampler"),
    ("smp_bpm_down", "Decrease Sampler BPM", "", "Sampler"),
    ("smp_bpm_down_large", "Decrease Sampler BPM (large)", "", "Sampler"),
    ("seq_rec", "REC Sequence", "", "Sampler"),
    ("seq_play", "Playback Sequence", "Cmd+Y", "Sampler"),
    ("seq_save", "Save Sequence", "", "Sampler"),
    ("seq_prev", "Previous Sequence", "Cmd+J", "Sampler"),
    ("seq_next", "Next Sequence", "Cmd+K", "Sampler"),
    ("seq_delete", "Delete Sequence", "", "Sampler"),
    ("seq_mute", "Sequence Mute mode", "", "Sampler"),
    ("seq_erase", "Sequence Erase mode", "", "Sampler"),
    ("seq_load1", "Sequence Load (Deck1)", "", "Sampler"),
    ("seq_load2", "Sequence Load (Deck2)", "", "Sampler"),
    ("seq_load3", "Sequence Load (Deck3)", "", "Sampler"),
    ("seq_load4", "Sequence Load (Deck4)", "", "Sampler"),
    ("padmode_sampler", "Pad mode - Sampler", "", "Sampler"),
    ("padmode_seqcall", "Pad mode - Sequence Call", "", "Sampler"),
    // Recording (PERFORMANCE mode): rekordbox's `RecControlPanel` commands; Start/Pause REC
    // has the key of its "Performance 1" preset.
    ("rec_toggle", "Start/Pause REC", "Cmd+Enter", "Recordings"),
    ("rec_level_up", "Increase REC level", "", "Recordings"),
    ("rec_level_down", "Decrease REC level", "", "Recordings"),
    ("rec_start", "Start REC", "", "Recordings"),
    ("rec_pause", "Pause REC", "", "Recordings"),
    ("list_up", "Up", "ArrowUp", "Browse"),
    ("list_down", "Down", "ArrowDown", "Browse"),
    ("load", "Load track to player 1", "Enter", "Browse"),
    ("remove", "Remove from Playlist", "Delete", "Browse"),
    ("focus", "Change Focus", "Tab", "Browse"),
    ("filter", "Display/Hide Track Filter", "Alt+F", "Browse"),
    ("import_file", "Import file", "Cmd+O", "Others"),
    ("import_dir", "Import directory", "Shift+Cmd+O", "Others"),
    ("fullscreen", "Fullscreen mode", "Ctrl+Cmd+F", "Others"),
    ("backup", "Backup Library", "", "Others"),
    ("restore", "Restore Library", "", "Others"),
    ("export_playlists_info", "Export Playlists information", "", "Others"),
    ("prefs", "Preferences", "Cmd+Comma", "Others"),
    ("about", "About...", "", "Others"),
];

/// Categories in the order the Keyboard page shows them.
pub const ACTION_CATEGORIES: [&str; 9] = ["Player", "Hot Cue", "Memory Cue", "Loop", "Grid", "Sampler", "Recordings", "Browse", "Others"];

/// Actions the key loop in main.rs handles itself, because they react to key releases,
/// key repeats or held modifiers.
pub const HELD_ACTIONS: [&str; 19] = ["search_back", "search_fwd", "play", "cue", "memory", "hot1", "hot2", "hot3", "hot4", "hot5", "hot6", "hot7", "hot8", "jump_back", "jump_fwd", "list_up", "list_down", "load", "remove"];

/// Actions that also have a menu item: with a ⌘ or ⌃ shortcut the menu bar receives the keys
/// (as its accelerator); otherwise the key loop does.
pub const MENU_ACTIONS: [(&str, &str); 4] = [("import_file", "import_track"), ("import_dir", "import_folder"), ("fullscreen", "fullscreen"), ("prefs", "prefs")];

/// What a one-shot keyboard action does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Track(i32),
    Volume(f32),
    Zoom(i32),
    TimeMode,
    Quantize,
    ClearHot(i32),
    MemJump(i32),
    MemCall(usize),
    MemDeleteCurrent,
    MemDelete(usize),
    LoopIn,
    LoopOut,
    Reloop,
    AutoLoop,
    LoopResize(f64),
    GridShift(f64),
    GridDownbeat,
    GridTap,
    Focus,
    Filter,
    Menu(&'static str),
    Sampler(crate::sampler_ext::SmpAct),
    /// A recording command (ui_rec.rs: `App::rec_command`).
    Rec(u8),
}

impl Act {
    pub fn parse(id: &str) -> Option<Act> {
        let num = |p: &str| id.strip_prefix(p).and_then(|n| n.parse::<usize>().ok());
        Some(match id {
            "prev_track" => Act::Track(-1),
            "next_track" => Act::Track(1),
            "vol_up" => Act::Volume(0.05),
            "vol_down" => Act::Volume(-0.05),
            "zoom_in" => Act::Zoom(-1),
            "zoom_out" => Act::Zoom(1),
            "zoom_reset" => Act::Zoom(0),
            "time_mode" => Act::TimeMode,
            "quantize" => Act::Quantize,
            "mem_next" => Act::MemJump(1),
            "mem_prev" => Act::MemJump(-1),
            "mem_delete" => Act::MemDeleteCurrent,
            "loop_in" => Act::LoopIn,
            "loop_out" => Act::LoopOut,
            "reloop" => Act::Reloop,
            "auto_loop" => Act::AutoLoop,
            "loop_halve" => Act::LoopResize(0.5),
            "loop_double" => Act::LoopResize(2.0),
            "grid_left" => Act::GridShift(-0.001),
            "grid_right" => Act::GridShift(0.001),
            "grid_downbeat" => Act::GridDownbeat,
            "grid_tap" => Act::GridTap,
            "focus" => Act::Focus,
            "filter" => Act::Filter,
            "import_file" => Act::Menu("import_track"),
            "import_dir" => Act::Menu("import_folder"),
            "fullscreen" => Act::Menu("fullscreen"),
            "backup" => Act::Menu("backup"),
            "restore" => Act::Menu("restore"),
            "export_playlists_info" => Act::Menu("export_playlists_info"),
            "prefs" => Act::Menu("prefs"),
            "about" => Act::Menu("about"),
            "rec_toggle" => Act::Rec(0),
            "rec_level_up" => Act::Rec(1),
            "rec_level_down" => Act::Rec(2),
            "rec_start" => Act::Rec(3),
            "rec_pause" => Act::Rec(4),
            _ => {
                if let Some(n) = num("clear_hot").filter(|n| (1..=8).contains(n)) {
                    Act::ClearHot(n as i32 - 1)
                } else if let Some(n) = num("mem_call").filter(|n| (1..=10).contains(n)) {
                    Act::MemCall(n - 1)
                } else if let Some(n) = num("mem_delete").filter(|n| (1..=10).contains(n)) {
                    Act::MemDelete(n - 1)
                } else if let Some(a) = crate::sampler_ext::SmpAct::parse(id) {
                    Act::Sampler(a)
                } else {
                    return None;
                }
            }
        })
    }
    /// Held keys repeat the action (searching, volume, grid nudges).
    pub fn repeats(&self) -> bool {
        matches!(self, Act::Volume(_) | Act::GridShift(_)) || matches!(self, Act::Sampler(a) if a.repeats())
    }
}

/// Carry out a one-shot keyboard action.
pub fn run_action(app: &mut App, act: Act) {
    match act {
        // (rekordbox has the recording commands in PERFORMANCE mode only.)
        Act::Rec(cmd) => {
            if app.performance() {
                app.rec_command(cmd);
            }
        }
        Act::Track(dir) => app.load_adjacent(dir),
        Act::Volume(d) => {
            let mut s = app.player.state.lock();
            s.volume = (s.volume + d).clamp(0.0, 1.0);
        }
        Act::Zoom(d) => {
            app.prefs.zoom = match d {
                0 => 2,
                d if d < 0 => app.prefs.zoom.saturating_sub(1),
                _ => (app.prefs.zoom + 1).min(ZOOMS.len() - 1),
            };
            app.save_prefs();
        }
        Act::TimeMode => {
            app.prefs.time_remain = !app.prefs.time_remain;
            app.save_prefs();
        }
        Act::Quantize => {
            app.prefs.quantize = !app.prefs.quantize;
            app.save_prefs();
            app.toast(if app.prefs.quantize { "Quantize ON" } else { "Quantize OFF" });
        }
        Act::ClearHot(slot) => app.clear_hot_cue(slot),
        Act::MemJump(d) => app.memory_jump(d),
        Act::MemCall(n) => {
            if let Some(c) = app.memory_cues().get(n).cloned() {
                app.call_memory(&c);
            }
        }
        Act::MemDeleteCurrent => app.memory_delete_current(),
        Act::MemDelete(n) => {
            if let Some(id) = app.memory_cues().get(n).map(|c| c.id) {
                app.delete_cue(id);
            }
        }
        Act::LoopIn => app.loop_in_pressed(),
        Act::LoopOut => app.loop_out_pressed(),
        Act::Reloop => app.exit_reloop(),
        Act::AutoLoop => {
            if app.loop_active().is_some() {
                app.loop_exit();
            } else {
                app.auto_loop(4.0);
            }
        }
        Act::LoopResize(f) => app.resize_loop(f),
        Act::GridShift(s) => app.grid_shift(s),
        Act::GridDownbeat => app.grid_set_downbeat(),
        Act::GridTap => app.grid_tap(),
        Act::Focus => {
            app.dialogs.ext.focus_tree = !app.dialogs.ext.focus_tree;
            app.toast(if app.dialogs.ext.focus_tree { "Keyboard focus: Tree View" } else { "Keyboard focus: track list" });
        }
        Act::Filter => crate::menus::handle(app, "filter"),
        Act::Menu(id) => crate::menus::handle(app, id),
        Act::Sampler(a) => app.sampler_action(a),
    }
}

/// Playlists and folders as the tree shows them (folders that are open show their children).
pub fn visible_nodes(nodes: &[PlaylistNode], expanded: &std::collections::HashSet<i64>) -> Vec<i64> {
    fn walk(nodes: &[PlaylistNode], expanded: &std::collections::HashSet<i64>, parent: i64, out: &mut Vec<i64>, depth: u8) {
        if depth > 32 {
            return;
        }
        let mut kids: Vec<&PlaylistNode> = nodes.iter().filter(|n| n.parent == parent).collect();
        kids.sort_by_key(|n| n.seq);
        for k in kids {
            out.push(k.id);
            if k.kind == NodeKind::Folder && expanded.contains(&k.id) {
                walk(nodes, expanded, k.id, out, depth + 1);
            }
        }
    }
    let mut out = vec![];
    walk(nodes, expanded, 0, &mut out, 0);
    out
}

/// Up/Down while the keyboard focus is on the Tree View: the previous/next playlist or
/// folder (Collection sits above the first one).
pub fn tree_step(app: &mut App, dir: i32) {
    let vis = visible_nodes(&app.nodes, &app.expanded);
    let cur = match &app.view {
        View::Playlist(id) => vis.iter().position(|x| x == id).map(|i| i as i64),
        _ => None,
    };
    let next = cur.unwrap_or(-1) + dir as i64;
    if next < 0 {
        app.side = SideMode::Collection;
        app.set_view(View::Collection);
    } else if let Some(id) = vis.get(next as usize) {
        app.side = SideMode::Playlists;
        app.set_view(View::Playlist(*id));
    }
}

/// Left/Right while the keyboard focus is on the Tree View: close/open the selected folder.
pub fn tree_expand(app: &mut App, open: bool) {
    if let View::Playlist(id) = app.view {
        if app.node(id).map(|n| n.kind == NodeKind::Folder).unwrap_or(false) {
            if open {
                app.expanded.insert(id);
            } else {
                app.expanded.remove(&id);
            }
        }
    }
}

// ------------------------------------------------------------------ My Tag in Comments

/// rekordbox's form: the tag names in a `/* A / B */` block at the end of the comment.
pub fn comment_with_tags(comment: &str, tags: &[String]) -> String {
    let base = match (comment.rfind("/*"), comment.trim_end().ends_with("*/")) {
        (Some(i), true) => comment[..i].trim_end(),
        _ => comment.trim_end(),
    };
    if tags.is_empty() {
        return base.to_string();
    }
    let block = format!("/* {} */", tags.join(" / "));
    if base.is_empty() { block } else { format!("{base} {block}") }
}

pub fn sync_mytag_comments(app: &mut App, ids: &[i64]) {
    for id in ids {
        let set = app.track_tags.get(id).cloned().unwrap_or_default();
        let names: Vec<String> = app.mytags.iter().filter(|t| t.parent != 0 && set.contains(&t.id)).map(|t| t.name.clone()).collect();
        let Some(cur) = app.track(*id).map(|t| t.comment.clone()) else { continue };
        let next = comment_with_tags(&cur, &names);
        if next != cur {
            if let Some(t) = app.track_mut(*id) {
                t.comment = next;
            }
            app.save_track_quiet(*id);
        }
    }
}

// ------------------------------------------------------------------ CUE Analysis

/// "Set Memory Cue at the first beat of track when analyzing": for a track that has no
/// memory cue yet.
pub fn memory_cue_at_first_beat(app: &App, id: i64, grid: &BeatGrid) -> bool {
    let dur = app.track(id).map(|t| t.duration).unwrap_or(0.0).max(1.0);
    let Some(first) = grid.beats(dur).first().map(|b| b.time) else { return false };
    if !app.db.cues(id).is_ok_and(|cues| cues.iter().all(|c| c.kind != CueKind::Memory)) {
        return false;
    }
    let c = Cue { id: 0, track_id: id, kind: CueKind::Memory, slot: -1, start: first.max(0.0), end: None, color: 0, comment: String::new(), active_loop: false };
    app.db.save_cue(&c).is_ok()
}

// ------------------------------------------------------------------ audio output

/// Preferences > Audio, as the output stream reads them when it opens: (sample rate, buffer
/// size in frames, first channel of the output pair); 0 = the device's default.
pub static STREAM_OPTS: parking_lot::Mutex<(u32, u32, u16)> = parking_lot::Mutex::new((0, 0, 0));

pub fn set_stream_opts(p: &Prefs) {
    *STREAM_OPTS.lock() = (p.audio_rate, p.audio_buffer, p.audio_channel);
    crate::audio::HP_CHANNEL.store(p.hp_channel.map(|c| c as i32).unwrap_or(-1), std::sync::atomic::Ordering::Relaxed);
    CLICK_SOUND.store(p.metronome_sound.min(2), std::sync::atomic::Ordering::Relaxed);
}

/// Preferences > Audio > Metronome: which of the three click sounds the metronome plays
/// (the audio callback reads it for every click).
pub static CLICK_SOUND: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// Pitch of "Click Sound 01", "02" and "03".
pub const CLICK_HZ: [f32; 3] = [1500.0, 1000.0, 2200.0];

pub fn click_hz() -> f32 {
    CLICK_HZ[(CLICK_SOUND.load(std::sync::atomic::Ordering::Relaxed) as usize).min(2)]
}

/// Buffer sizes the Audio page's slider offers (rekordbox's runs from 64 to 2048 samples,
/// evenly spaced in samples).
pub const SLIDER_BUFFERS: [u32; 6] = [64, 128, 256, 512, 1024, 2048];

/// Where a buffer size sits on the slider (0..=1), and the size nearest to a position.
pub fn buffer_to_slider(b: u32) -> f32 {
    ((b.clamp(64, 2048) - 64) as f32 / (2048 - 64) as f32).clamp(0.0, 1.0)
}

pub fn slider_to_buffer(t: f32, range: Option<(u32, u32)>) -> u32 {
    let want = 64.0 + t.clamp(0.0, 1.0) * (2048 - 64) as f32;
    let ok = |b: &u32| range.map(|(lo, hi)| *b >= lo && *b <= hi).unwrap_or(true);
    let mut best = SLIDER_BUFFERS.iter().copied().filter(ok).collect::<Vec<_>>();
    if best.is_empty() {
        best = SLIDER_BUFFERS.to_vec();
    }
    best.into_iter().min_by(|a, b| (*a as f32 - want).abs().total_cmp(&(*b as f32 - want).abs())).unwrap_or(512)
}

// ------------------------------------------------------------------ stems

/// What a click on a stem part button does to the four sources' on/off states
/// (source index: 0 drums, 1 bass, 2 other, 3 vocals; `srcs` are the part's sources).
/// MUTE mode: the part is switched off, or back on. SOLO mode, as rekordbox 7.2.19 derives it
/// from the same on/off states (`PartMuteSoloComp::getNextStatesOnSoloMode`, 0x102075b08):
/// the part plays alone, unless it is sounding while some other part is off, in which case
/// every part comes back.
pub fn stem_click(on: [bool; 4], srcs: &[usize], solo: bool) -> [bool; 4] {
    let mut next = on;
    if solo {
        let restore = on.iter().any(|x| !*x) && srcs.iter().all(|s| on[*s]);
        for (i, n) in next.iter_mut().enumerate() {
            *n = restore || srcs.contains(&i);
        }
    } else {
        let target = !srcs.iter().all(|s| on[*s]);
        for s in srcs {
            next[*s] = target;
        }
    }
    next
}

/// A click on a part button of the stem strip, in the mode chosen in Preferences > STEMS >
/// ACTIVE STEM Setting (`Prefs::stem_solo`). The strip calls this with the part's sources.
#[allow(dead_code)]
pub fn stem_part_click(app: &mut App, srcs: &[usize]) {
    let (have, on) = {
        let s = app.player.state.lock();
        (s.has_playable_stems(), s.stem_on)
    };
    if !have {
        // rekordbox's status line for a part pressed before its data exists. recordbox also
        // starts the separation if it is not running yet (it is, when tracks are separated
        // on load).
        if app.separating_stems() || app.loading_stems() {
            app.toast("There is no analysis data for the STEMS function.");
        } else {
            app.separate_loaded();
        }
        return;
    }
    let next = stem_click(on, srcs, app.prefs.stem_solo);
    app.player.state.lock().stem_on = next;
}

/// Preferences > ACTIVE STEM Setting. Going from MUTE to SOLO brings every part back unless
/// exactly one part is sounding (`PartMuteSoloComp::selectPartMuteMode`, 0x102075628); going
/// back to MUTE keeps the parts as they are.
pub fn set_stem_solo(app: &mut App, solo: bool) {
    if solo && !app.prefs.stem_solo {
        for st in app.player.decks.iter() {
            let mut s = st.lock();
            s.stem_on = stems_entering_solo(s.stem_on, app.prefs.stem_parts == 3);
        }
    }
    app.prefs.stem_solo = solo;
}

/// The on/off states after switching to SOLO mode (`three`: bass belongs to the instruments).
pub fn stems_entering_solo(on: [bool; 4], three: bool) -> [bool; 4] {
    let parts: &[&[usize]] = if three { &[&[3], &[2, 1], &[0]] } else { &[&[3], &[2], &[1], &[0]] };
    let sounding = parts.iter().filter(|p| p.iter().all(|s| on[*s])).count();
    if sounding == 1 { on } else { [true; 4] }
}

/// The stream configuration for a device: its default, with the chosen sample rate and
/// buffer size where the device offers them, and the first channel of the output pair.
pub fn stream_config(dev: &cpal::Device) -> anyhow::Result<(cpal::SupportedStreamConfig, cpal::StreamConfig, usize)> {
    use cpal::traits::DeviceTrait;
    let (rate, buffer, ch) = *STREAM_OPTS.lock();
    let mut cfg = dev.default_output_config()?;
    if rate != 0 && rate != cfg.sample_rate().0 {
        if let Ok(mut ranges) = dev.supported_output_configs() {
            let (fmt, chans) = (cfg.sample_format(), cfg.channels());
            if let Some(r) = ranges.find(|r| r.sample_format() == fmt && r.channels() == chans && r.min_sample_rate().0 <= rate && rate <= r.max_sample_rate().0) {
                cfg = r.with_sample_rate(cpal::SampleRate(rate));
            }
        }
    }
    let mut config: cpal::StreamConfig = cfg.clone().into();
    if buffer != 0 {
        let ok = match cfg.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => (*min..=*max).contains(&buffer),
            cpal::SupportedBufferSize::Unknown => true,
        };
        if ok {
            config.buffer_size = cpal::BufferSize::Fixed(buffer);
        }
    }
    let first = if (ch as usize) + 1 < cfg.channels() as usize { ch as usize } else { 0 };
    Ok((cfg, config, first))
}

/// Render stereo and place it on channels `first` and `first + 1` of a wider device; the
/// other channels are silent.
pub fn render_pair(out: &mut [f32], channels: usize, first: usize, tmp: &mut Vec<f32>, mut render: impl FnMut(&mut [f32])) {
    let frames = out.len() / channels.max(1);
    tmp.clear();
    tmp.resize(frames * 2, 0.0);
    render(tmp);
    out.fill(0.0);
    if first + 1 < channels {
        for (f, s) in out.chunks_mut(channels).zip(tmp.chunks(2)) {
            f[first] = s[0];
            f[first + 1] = s[1];
        }
    }
}

/// What an output device offers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioCaps {
    pub name: String,
    pub default_rate: u32,
    pub rates: Vec<u32>,
    pub channels: u16,
    /// Buffer sizes (frames) the device accepts, when it says.
    pub buffer: Option<(u32, u32)>,
}

pub const COMMON_RATES: [u32; 6] = [44100, 48000, 88200, 96000, 176400, 192000];
pub const BUFFER_SIZES: [u32; 7] = [64, 128, 256, 512, 1024, 2048, 4096];

/// Ask a device (None: the default output) what it offers. Device queries can block on some
/// systems, so this runs on its own thread and gives up after `timeout`.
pub fn audio_caps_within(device: Option<String>, timeout: std::time::Duration) -> Option<AudioCaps> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        let dev = device.and_then(|want| host.output_devices().ok().and_then(|mut it| it.find(|d| d.name().map(|n| n == want).unwrap_or(false)))).or_else(|| host.default_output_device());
        let caps = dev.and_then(|d| {
            let def = d.default_output_config().ok()?;
            let mut c = AudioCaps { name: d.name().unwrap_or_default(), default_rate: def.sample_rate().0, channels: def.channels(), ..Default::default() };
            if let cpal::SupportedBufferSize::Range { min, max } = def.buffer_size() {
                c.buffer = Some((*min, *max));
            }
            for r in d.supported_output_configs().ok()? {
                if r.sample_format() != def.sample_format() || r.channels() != def.channels() {
                    continue;
                }
                for rate in COMMON_RATES {
                    if r.min_sample_rate().0 <= rate && rate <= r.max_sample_rate().0 && !c.rates.contains(&rate) {
                        c.rates.push(rate);
                    }
                }
            }
            c.rates.sort();
            Some(c)
        });
        let _ = tx.send(caps);
    });
    rx.recv_timeout(timeout).ok().flatten()
}

// ------------------------------------------------------------------ menu helpers

/// The menu-bar accelerator for a shortcut (None: the key loop handles it).
pub fn accelerator(sc: &Shortcut) -> Option<muda::accelerator::Accelerator> {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    if !(sc.cmd || sc.ctrl) {
        return None;
    }
    let name = sc.key.name();
    let code_name = match name {
        n if n.len() == 1 && n.as_bytes()[0].is_ascii_alphabetic() => format!("Key{}", n.to_ascii_uppercase()),
        n if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() => format!("Digit{n}"),
        "Left" | "Right" | "Up" | "Down" => format!("Arrow{name}"),
        "Equals" => "Equal".into(),
        "OpenBracket" => "BracketLeft".into(),
        "CloseBracket" => "BracketRight".into(),
        "Backtick" => "Backquote".into(),
        n => n.to_string(),
    };
    let code: Code = code_name.parse().ok()?;
    let mut m = Modifiers::empty();
    if sc.cmd {
        m |= Modifiers::META;
    }
    if sc.ctrl {
        m |= Modifiers::CONTROL;
    }
    if sc.alt {
        m |= Modifiers::ALT;
    }
    if sc.shift {
        m |= Modifiers::SHIFT;
    }
    Some(Accelerator::new(m, code))
}

/// Window state added with the menu and Preferences work (a field of `Dialogs`).
#[derive(Default)]
pub struct DialogsExt {
    /// Help > Keyboard Shortcuts window.
    pub shortcuts_open: bool,
    /// Up/Down move in the Tree View instead of the track list (Change Focus).
    pub focus_tree: bool,
    /// Preferences sidebar search text.
    pub prefs_search: String,
    /// Backup Library: waiting for the answer to "Do you want to back up music files as well?".
    pub backup_ask: bool,
    /// Restore Library: the chosen archive, waiting for confirmation.
    pub restore_ask: Option<std::path::PathBuf>,
    /// Backup or restore running in the background.
    pub backup_job: Option<crate::backup::Job>,
    /// Result text of the last backup/restore, shown until dismissed: (title, text).
    pub backup_done: Option<(String, String)>,
    /// "Reset to defaults" waiting for confirmation: Some(page) for one page, Some(None) all.
    pub reset_ask: Option<Option<u8>>,
}

/// Playlist > Sort Items: the children of a folder (0 = the top level) by name.
pub fn sort_items(app: &mut App, parent: i64) {
    let mut kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == parent).cloned().collect();
    kids.sort_by_key(|n| n.name.to_lowercase());
    for (i, k) in kids.iter().enumerate() {
        let _ = app.db.move_node(k.id, parent, i as i64);
    }
    app.reload_playlists();
}

/// The folder Sort Items and the Create commands act in: the selected folder, else the
/// folder of the selected playlist, else the top level.
pub fn current_parent(app: &App) -> i64 {
    match &app.view {
        View::Playlist(id) => match app.node(*id) {
            Some(n) if n.kind == NodeKind::Folder => n.id,
            Some(n) => n.parent,
            None => 0,
        },
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_dispatched_and_defaults_are_keys() {
        let mut ids = std::collections::HashSet::new();
        let mut keys = std::collections::HashMap::new();
        for (id, label, def, cat) in ACTIONS {
            assert!(ids.insert(id), "duplicate action {id}");
            assert!(!label.is_empty());
            assert!(ACTION_CATEGORIES.contains(&cat), "{id}: unknown category {cat}");
            assert!(HELD_ACTIONS.contains(&id) || Act::parse(id).is_some(), "{id} has no handler");
            if !def.is_empty() {
                let sc = Shortcut::parse(def).unwrap_or_else(|| panic!("{id}: {def} is not a shortcut"));
                assert_eq!(Shortcut::parse(&sc.name()), Some(sc), "{id}: stored form reads back");
                // (The sampler deck's keys are rekordbox's Performance preset; the others are
                // its Export preset. The two share keys, and in PERFORMANCE mode the
                // sampler's come first.)
                if let Some(other) = keys.insert((crate::sampler_ext::is_sampler_action(id), def), id) {
                    panic!("{id} and {other} share {def}");
                }
            }
        }
        for h in HELD_ACTIONS {
            assert!(ids.contains(h), "{h} is not an action");
        }
        for (a, _) in MENU_ACTIONS {
            assert!(matches!(Act::parse(a), Some(Act::Menu(_))), "{a}");
        }
        // Shift+arrow left/right stay free: beat jumps use Shift for a one-beat jump.
        assert!(!keys.contains_key(&(false, "Shift+ArrowLeft")) && !keys.contains_key(&(false, "Shift+ArrowRight")));
        assert_eq!(Act::parse("clear_hot8"), Some(Act::ClearHot(7)));
        assert_eq!(Act::parse("mem_call10"), Some(Act::MemCall(9)));
        assert_eq!(Act::parse("mem_delete3"), Some(Act::MemDelete(2)));
        assert_eq!(Act::parse("mem_delete"), Some(Act::MemDeleteCurrent));
        assert_eq!(Act::parse("clear_hot9"), None);
        assert_eq!(Act::parse("nonsense"), None);
    }

    #[test]
    fn menu_accelerators() {
        for (a, _) in MENU_ACTIONS {
            let def = ACTIONS.iter().find(|x| x.0 == a).unwrap().2;
            assert!(accelerator(&Shortcut::parse(def).unwrap()).is_some(), "{a}: {def}");
        }
        // Plain keys never become menu accelerators (they would take typing away from text fields).
        assert!(accelerator(&Shortcut::parse("F").unwrap()).is_none());
        assert!(accelerator(&Shortcut::parse("Shift+Cmd+O").unwrap()).is_some());
        assert!(accelerator(&Shortcut::parse("Cmd+1").unwrap()).is_some());
        assert!(accelerator(&Shortcut::parse("Cmd+ArrowUp").unwrap()).is_some());
    }

    #[test]
    fn new_prefs_round_trip() {
        let mut p = Prefs::default();
        assert!(p.tree_item("Devices") && p.cue_markers_on_preview && !p.refine_beatgrid_transients && !p.wave_half);
        p.tree_items.insert("Devices".into(), false);
        p.tree_items.insert("Collection".into(), false);
        p.mytag_to_comments = true;
        p.cue_markers_on_preview = false;
        p.wave_half = true;
        p.color_names[2] = "Peak time".into();
        p.color_names[3] = "  ".into();
        p.refine_beatgrid_transients = true;
        p.memory_cue_first_beat = true;
        p.audio_buffer = 256;
        p.audio_rate = 48000;
        p.audio_channel = 2;
        p.device_bg_color = 5;
        p.delete_history_after_import = true;
        p.hot_cue_auto_load_on_import = false;
        p.itunes_xml = Some("/tmp/iTunes Library.xml".into());
        p.rekordbox_xml_path = Some("/tmp/rekordbox.xml".into());
        p.relocate_folders = vec!["/Volumes/Music".into()];
        let s = serde_json::to_string(&p).unwrap();
        let q: Prefs = serde_json::from_str(&s).unwrap();
        assert_eq!(format!("{p:?}"), format!("{q:?}"));
        assert!(!q.tree_item("Devices"));
        assert!(q.tree_item("Collection"), "Collection cannot be hidden");
        assert_eq!(q.color_name(3), "Peak time");
        assert_eq!(q.color_name(4), "Yellow", "a blank name falls back to the colour");
        assert_eq!(q.color_name(0), "No Color");
        // Preferences saved before these settings existed load with the defaults.
        // The settings of the 7.2.19 pages.
        let mut p = Prefs::default();
        assert!(p.palette_enabled && p.playlist_search_filter && p.all_tracks_in_playlists && p.playlist_track_counts);
        assert!(!p.show_phrase_bar && !p.phrase_enlarged && p.phrase_types_always && !p.vocal_enlarged && p.show_vocal);
        assert!(!p.stem_solo && p.stem_reset_on_load && p.stem_waveform && p.metronome_sound == 0 && p.preview_volume == 0.5);
        // The Media Browser ticks of the reference: everything Recordbox has but rekordbox xml.
        assert!(p.tree_item("Related Tracks") && p.tree_item("iTunes") && p.tree_item("Explorer") && !p.tree_item("rekordbox xml"));
        p.palette_enabled = false;
        p.playlist_search_filter = false;
        p.all_tracks_in_playlists = false;
        p.playlist_track_counts = false;
        p.phrase_enlarged = true;
        p.phrase_types_always = false;
        p.vocal_enlarged = true;
        p.metronome_sound = 2;
        p.preview_volume = 0.25;
        p.stem_solo = true;
        p.stem_reset_on_load = false;
        p.stem_waveform = false;
        let q: Prefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(format!("{p:?}"), format!("{q:?}"));
        assert!(!q.palette_enabled && !q.playlist_search_filter && !q.all_tracks_in_playlists && !q.playlist_track_counts);
        assert!(q.phrase_enlarged && !q.phrase_types_always && q.vocal_enlarged && q.metronome_sound == 2 && q.preview_volume == 0.25);
        assert!(q.stem_solo && !q.stem_reset_on_load && !q.stem_waveform);
        let old: Prefs = serde_json::from_str(r#"{"key_alnum":true}"#).unwrap();
        assert!(old.palette_enabled && old.all_tracks_in_playlists && old.stem_reset_on_load && old.preview_volume == 0.5);
        assert!(old.key_alnum && old.cue_markers_on_preview && old.hot_cue_auto_load_on_import);
        assert_eq!(old.color_names, default_color_names());
        assert_eq!(old.audio_buffer, 0);
    }

    #[test]
    fn stem_buttons_in_mute_and_solo_mode() {
        let all = [true; 4];
        // MUTE: a click switches the part off, the next one back on; INST of three parts is two sources.
        assert_eq!(stem_click(all, &[3], false), [true, true, true, false]);
        assert_eq!(stem_click([true, true, true, false], &[3], false), all);
        assert_eq!(stem_click(all, &[2, 1], false), [true, false, false, true]);
        assert_eq!(stem_click([true, false, true, true], &[2, 1], false), all, "a half-muted part comes back whole");
        // SOLO: the part alone; another part takes over; the same part again restores all.
        assert_eq!(stem_click(all, &[3], true), [false, false, false, true]);
        assert_eq!(stem_click([false, false, false, true], &[0], true), [true, false, false, false]);
        assert_eq!(stem_click([true, false, false, false], &[0], true), all);
        assert_eq!(stem_click(all, &[2, 1], true), [false, true, true, false]);
        assert_eq!(stem_click([false, true, true, false], &[2, 1], true), all);
        // As rekordbox: a sounding part pressed while another is off restores all, also when
        // it is not alone; a silent part pressed takes over.
        assert_eq!(stem_click([true, false, true, true], &[3], true), all);
        assert_eq!(stem_click([true, false, true, false], &[3], true), [false, false, false, true]);
        // Switching to SOLO: all parts back unless exactly one sounds.
        assert_eq!(stems_entering_solo([true, true, true, false], false), all);
        assert_eq!(stems_entering_solo([false, false, false, true], false), [false, false, false, true]);
        assert_eq!(stems_entering_solo([false, true, true, false], true), [false, true, true, false], "INST of three parts is one part");
        assert_eq!(stems_entering_solo(all, false), all);
    }

    #[test]
    fn buffer_slider_and_click_sounds() {
        assert_eq!(buffer_to_slider(64), 0.0);
        assert_eq!(buffer_to_slider(2048), 1.0);
        assert!((buffer_to_slider(128) - 0.032).abs() < 0.001, "128 samples sits where the reference shows it");
        for b in SLIDER_BUFFERS {
            assert_eq!(slider_to_buffer(buffer_to_slider(b), None), b);
        }
        assert_eq!(slider_to_buffer(0.0, Some((128, 1024))), 128, "sizes the device refuses are skipped");
        assert_eq!(slider_to_buffer(1.0, Some((128, 1024))), 1024);
        let mut p = Prefs::default();
        p.metronome_sound = 1;
        set_stream_opts(&p);
        assert_eq!(click_hz(), CLICK_HZ[1]);
        p.metronome_sound = 0;
        set_stream_opts(&p);
        assert_eq!(click_hz(), CLICK_HZ[0]);
        assert!(CLICK_HZ[0] != CLICK_HZ[1] && CLICK_HZ[1] != CLICK_HZ[2]);
    }

    #[test]
    fn tags_in_comments() {
        let tags = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(comment_with_tags("", &tags(&["House", "Peak"])), "/* House / Peak */");
        assert_eq!(comment_with_tags("great intro", &tags(&["House"])), "great intro /* House */");
        assert_eq!(comment_with_tags("great intro /* House */", &tags(&["House", "Vocal"])), "great intro /* House / Vocal */");
        assert_eq!(comment_with_tags("great intro /* House */", &[]), "great intro");
        assert_eq!(comment_with_tags("/* House */", &[]), "");
        assert_eq!(comment_with_tags("a /* b */ c", &tags(&["X"])), "a /* b */ c /* X */");
    }

    #[test]
    fn output_pair_placement() {
        let mut out = vec![9.0f32; 8];
        let mut tmp = vec![];
        render_pair(&mut out, 4, 2, &mut tmp, |b| {
            assert_eq!(b.len(), 4);
            b.copy_from_slice(&[0.1, 0.2, 0.3, 0.4]);
        });
        assert_eq!(out, vec![0.0, 0.0, 0.1, 0.2, 0.0, 0.0, 0.3, 0.4]);
        // A pair the device does not have leaves silence rather than writing out of range.
        render_pair(&mut out, 4, 3, &mut tmp, |b| b.fill(1.0));
        assert!(out.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn tree_order() {
        let n = |id, parent, seq, kind| PlaylistNode { id, parent, name: format!("n{id}"), kind, seq, smart: None };
        let nodes = vec![n(1, 0, 1, NodeKind::Folder), n(2, 1, 0, NodeKind::Playlist), n(3, 0, 0, NodeKind::Playlist), n(4, 1, 1, NodeKind::Playlist)];
        let mut open = std::collections::HashSet::new();
        assert_eq!(visible_nodes(&nodes, &open), vec![3, 1]);
        open.insert(1);
        assert_eq!(visible_nodes(&nodes, &open), vec![3, 1, 2, 4]);
    }
}
