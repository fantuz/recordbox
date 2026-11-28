//! Native macOS menu bar, laid out as rekordbox 7.2.19 builds it for the current mode
//! (`MainComponent::getMenuBarNames` 0x1009643d8 and `getMenuForIndex` 0x100964638; notes in
//! `checks/re-7.2.19/shell-menus-traffic.md`): File / View / Track / Playlist / Help in EXPORT
//! and PERFORMANCE, File / Edit / View / Help in EDIT. recordbox's own commands, which rekordbox
//! has no item for, come after a separator at the end of the menu they belong to; the macOS app
//! menu and an Edit menu (Undo, Redo and the clipboard commands) are recordbox's too.
//! Items are enabled and ticked from the app's state every frame (`sync`), and their
//! accelerators follow Preferences > Keyboard.

use crate::app::*;
use crate::model::*;
use eframe::egui;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::cell::RefCell;
use std::collections::HashMap;

enum Entry {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

#[derive(Default)]
struct Handles {
    items: HashMap<&'static str, Entry>,
    subs: HashMap<&'static str, Submenu>,
    /// The macOS app menu (always first).
    app_menu: Option<Submenu>,
    /// What each item was last set to: (enabled, ticked).
    shown: HashMap<&'static str, (bool, bool)>,
    /// What each submenu was last set to.
    subs_shown: HashMap<&'static str, bool>,
    /// The keymap the accelerators were last built from.
    keys: String,
    /// The mode the menus were last laid out for (`MODE_NONE` before the first time).
    mode: u8,
    /// Signatures of what the filled-in submenus were last built from.
    playlists_sig: u64,
    devices_sig: u64,
    /// Text of "Remove from Collection" / "Remove from Playlist".
    remove_label: &'static str,
}

const MODE_NONE: u8 = 255;

thread_local! {
    static HANDLES: RefCell<Handles> = RefCell::new(Handles { mode: MODE_NONE, ..Handles::default() });
}

/// Every menu command: (id, label, ticked item). The order here is not the menu order. Some
/// commands are in no menu (they are reached from Preferences > Keyboard or a context menu).
pub const COMMANDS: &[(&str, &str, bool)] = &[
    ("edit_cut", "Cut", false),
    ("edit_copy", "Copy", false),
    ("edit_paste", "Paste", false),
    ("edit_select_all", "Select All", false),
    ("about", "About recordbox", false),
    ("prefs", "Preferences…", false),
    ("import_track", "Import Track", false),
    ("import_folder", "Import Folder", false),
    ("import_playlist", "Import Playlist", false),
    ("backup", "Backup Library", false),
    ("restore", "Restore Library", false),
    ("missing", "Display All Missing Files", false),
    ("collection_cleanup", "Review collection cleanup…", false),
    ("dupes", "Display All Duplicated Files", false),
    ("device_backup_mgmt", "Backed up device library management", false),
    ("import_v5", "Import your library from ver. 5 (or earlier)", false),
    ("pack_sample_dl", "Download", false),
    ("pack_sample_import", "Import", false),
    ("pack_gc_dl", "Download", false),
    ("pack_gc_import", "Import", false),
    ("export_xml", "Export Collection in xml format", false),
    ("export_playlists_info", "Export Playlists Information", false),
    ("sync_rekordbox", "Review desktop library sync…", false),
    ("import_xml", "Import Collection from rekordbox xml…", false),
    ("relocate_missing", "Relocate Missing Files…", false),
    ("library_folder", "Show Library Folder", false),
    ("undo", "Undo", false),
    ("redo", "Redo", false),
    ("fullscreen", "Full screen", true),
    ("appearance_dark", "Dark", true),
    ("appearance_light", "Light", true),
    ("layout_player", "1 Player", true),
    ("layout_2players", "2 Players", true),
    ("layout_dual", "Dual Player", true),
    ("layout_simple", "Simple Player", true),
    ("layout_browser", "Full Browser", true),
    ("perf_2h", "2 Deck Horizontal", true),
    ("perf_2v", "2 Deck Vertical", true),
    ("perf_4h", "4 Deck Horizontal", true),
    ("perf_4v", "4 Deck Vertical", true),
    ("perf_browse", "Browse", true),
    ("pad_platter", "Show / Hide Pad and Platter", false),
    ("split_screen", "Show Split Screen", true),
    ("mytag_window", "My Tag Configuration Window", true),
    ("related_subpanel", "\"Related tracks\" on Subpanel", true),
    ("suggest_subpanel", "\"Track Suggestion\" on Subpanel", true),
    ("info_window", "Information Window", true),
    ("sub_browser", "Vertical sub-browser", true),
    ("sub_browser_h", "Horizontal sub-browser", true),
    ("automix", "Automix", true),
    ("related_window", "Related Track List Window", true),
    ("filter", "Filter", true),
    ("wave_blue", "Waveform: BLUE", true),
    ("wave_rgb", "Waveform: RGB", true),
    ("wave_3band", "Waveform: 3Band", true),
    ("t_import_track", "Import Track", false),
    ("import_to_collection", "Import To Collection", false),
    ("analyze", "Analyze Track", false),
    ("look_up_match_tags", "Look up related track tags…", false),
    ("analyze_missing", "Analyze Missing Data", false),
    ("analyze_key", "Analyze Key", false),
    ("analysis_lock", "Analysis Lock", true),
    ("lock_on", "On", false),
    ("lock_off", "Off", false),
    ("auto_on", "Enable", true),
    ("auto_off", "Disable", true),
    ("stop_analysis", "Stop Analyzing", false),
    ("export_tracks", "Export Track", false),
    ("reload_tags", "Reload Tag", false),
    ("relocate", "Relocate", false),
    ("auto_relocate", "Auto Relocate", false),
    ("show_finder", "Show in Finder", false),
    ("remove_collection", "Remove from Collection", false),
    ("property", "Property", false),
    ("load", "Load to Player", false),
    ("trash", "Restore Removed Tracks…", false),
    ("stem_vocals", "Vocals", false),
    ("stem_inst", "Instrumental", false),
    ("stem_stop", "Stop Exporting Stems", false),
    ("p_import_playlist", "Import Playlist", false),
    ("new_playlist", "Create New Playlist", false),
    ("new_smart", "Create New Intelligent Playlist", false),
    ("new_folder", "Create New Folder", false),
    ("delete_playlist", "Delete Playlist", false),
    ("rename_playlist", "Rename Playlist", false),
    ("export_playlist", "Export Playlist", false),
    ("sort_items", "Sort Items", false),
    ("renumber", "Renumber Track Order", false),
    ("save_playlist_info", "Save Playlist Info", false),
    ("save_info_txt", "Export a playlist to a file (*.txt)", false),
    ("save_info_m3u8", "Export a playlist to a file for music apps (*.m3u8)", false),
    ("export_device", "Export to USB Device…", false),
    ("manual", "User's Manual", false),
    ("shortcuts", "Keyboard Shortcuts", false),
    ("h_about", "About recordbox", false),
    ("copy_to_playlists", "Copy to Playlists", false),
];

/// One entry of a menu's layout: a command, a separator or a submenu (by key).
#[derive(Clone, Copy)]
enum It {
    C(&'static str),
    Sep,
    Sub(&'static str),
}
use It::{Sep, Sub, C};

/// Submenus: (key, title). "add_to_playlist", "export_track" and "export_playlist" are filled
/// in from the playlist tree and the connected devices (`fill_playlists`, `fill_devices`).
const SUBMENUS: [(&str, &str); 17] = [
    ("file", "File"),
    ("import", "Import"),
    ("additional", "Additional contents"),
    ("sample_pack", "Sample Pack"),
    ("gc_pack", "GROOVE CIRCUIT FACTORY SAMPLES PACK"),
    ("library", "Library"),
    ("edit", "Edit"),
    ("view", "View"),
    ("appearance", "Appearance"),
    ("track", "Track"),
    ("analysis_lock", "Analysis Lock"),
    ("add_to_playlist", "Add To Playlist"),
    ("export_track", "Export Track"),
    ("export_stems", "Export Stems"),
    ("playlist", "Playlist"),
    ("export_playlist", "Export Playlist"),
    ("save_info", "Save Playlist Info"),
];
const HELP: (&str, &str) = ("help", "Help");

/// The menu-bar mode for an application mode: 0 EXPORT, 1 PERFORMANCE, 2 EDIT.
fn menu_mode(app: &App) -> u8 {
    app.prefs.app_mode.min(2)
}

/// Top-level menus of a mode, in order (rekordbox's `getMenuBarNames`; "edit" outside EDIT
/// mode is recordbox's own).
fn top_level(mode: u8) -> &'static [&'static str] {
    match mode {
        2 => &["file", "edit", "view", "help"],
        _ => &["file", "edit", "view", "track", "playlist", "help"],
    }
}

/// What each menu and submenu holds in a mode, as rekordbox's `getMenuForIndex` builds it.
/// Items that depend on a plan, an account, a streaming service or the cloud are left out
/// (Cloud Library Sync, Mobile Library Sync, Liked Songs, Collaborative playlists, Show in
/// Spotify, the lighting settings in Library).
fn layout(mode: u8) -> Vec<(&'static str, Vec<It>)> {
    let perf = mode == 1;
    let mut file = vec![Sub("import")];
    if perf {
        file.extend([Sep, Sub("additional")]);
    }
    file.extend([Sep, C("missing"), C("dupes"), Sep, Sub("library"), Sep, C("export_xml"), C("export_playlists_info")]);
    // recordbox's own.
    file.extend([Sep, C("collection_cleanup"), C("sync_rekordbox"), C("import_xml"), C("relocate_missing"), C("library_folder")]);

    let mut view = vec![C("fullscreen"), Sep, Sub("appearance"), Sep];
    match mode {
        0 => view.extend([C("layout_player"), C("layout_2players"), C("layout_dual"), C("layout_simple"), C("layout_browser"), Sep, C("split_screen"), Sep]),
        1 => view.extend([C("perf_2h"), C("perf_2v"), C("perf_4h"), C("perf_4v"), C("perf_browse"), Sep, C("pad_platter"), Sep, C("split_screen"), Sep]),
        _ => view.extend([C("split_screen"), Sep]),
    }
    view.extend([C("mytag_window"), C("related_subpanel"), C("suggest_subpanel"), C("info_window"), C("sub_browser"), C("sub_browser_h")]);
    if perf {
        view.push(C("automix"));
    }

    let track = vec![
        C("import_to_collection"),
        C("analyze"),
        C("look_up_match_tags"),
        Sub("analysis_lock"),
        Sep,
        Sub("add_to_playlist"),
        C("reload_tags"),
        Sub("export_track"),
        Sep,
        C("remove_collection"),
        Sep,
        C("show_finder"),
        C("auto_relocate"),
        C("relocate"),
        // recordbox's own.
        Sep,
        C("trash"),
        Sub("export_stems"),
    ];
    let playlist = vec![
        Sub("export_playlist"),
        C("p_import_playlist"),
        C("copy_to_playlists"),
        Sep,
        C("new_playlist"),
        C("new_smart"),
        C("new_folder"),
        Sep,
        C("delete_playlist"),
        Sep,
        C("sort_items"),
        C("renumber"),
        Sub("save_info"),
        // recordbox's own.
        Sep,
        C("export_device"),
    ];
    vec![
        ("file", file),
        ("import", vec![C("import_track"), C("import_folder"), C("import_playlist")]),
        ("additional", vec![Sub("sample_pack"), Sub("gc_pack")]),
        ("sample_pack", vec![C("pack_sample_dl"), C("pack_sample_import")]),
        ("gc_pack", vec![C("pack_gc_dl"), C("pack_gc_import")]),
        ("library", vec![C("backup"), C("restore"), C("device_backup_mgmt"), C("import_v5")]),
        ("edit", vec![C("undo"), C("redo"), Sep, C("edit_cut"), C("edit_copy"), C("edit_paste"), C("edit_select_all")]),
        ("view", view),
        ("appearance", vec![C("appearance_dark"), C("appearance_light")]),
        ("track", track),
        ("export_stems", vec![C("stem_vocals"), C("stem_inst"), Sep, C("stem_stop")]),
        ("analysis_lock", vec![C("lock_on"), C("lock_off")]),
        ("playlist", playlist),
        ("save_info", vec![C("save_info_txt"), C("save_info_m3u8")]),
        ("help", vec![C("manual"), C("shortcuts"), Sep, C("h_about")]),
    ]
}

fn it<'a>(h: &'a Handles, id: &str) -> &'a dyn IsMenuItem {
    match h.items.get(id).unwrap_or_else(|| panic!("menu item {id}")) {
        Entry::Plain(i) => i,
        Entry::Check(i) => i,
    }
}

/// Accelerators that are not part of Preferences > Keyboard.
fn fixed_accelerator(id: &str) -> Option<Accelerator> {
    let cmd = Modifiers::META;
    let (m, c) = match id {
        "new_playlist" => (cmd, Code::KeyN),
        "export_device" => (cmd, Code::KeyE),
        // rekordbox: Information Window is Cmd+I, Show in Finder Cmd+R.
        "info_window" => (cmd, Code::KeyI),
        "show_finder" => (cmd, Code::KeyR),
        // Edit commands carry their own shortcuts and are passed on to the focused text field
        // or the track list (the system's predefined items never reach the egui view).
        "edit_cut" => (cmd, Code::KeyX),
        "edit_copy" => (cmd, Code::KeyC),
        "edit_paste" => (cmd, Code::KeyV),
        "edit_select_all" => (cmd, Code::KeyA),
        _ => return None,
    };
    Some(Accelerator::new(m, c))
}

/// Menu choices travel through this channel; see `wake_on_menu`.
fn menu_channel() -> &'static (crossbeam_channel::Sender<MenuEvent>, crossbeam_channel::Receiver<MenuEvent>) {
    static CH: std::sync::OnceLock<(crossbeam_channel::Sender<MenuEvent>, crossbeam_channel::Receiver<MenuEvent>)> = std::sync::OnceLock::new();
    CH.get_or_init(crossbeam_channel::unbounded)
}

/// A menu choice must be acted on at once: the window only redraws on input, so without a
/// wake-up a command chosen from the menu bar would wait for the next mouse move.
pub fn wake_on_menu(ctx: &eframe::egui::Context) {
    let ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
        let _ = menu_channel().0.send(ev);
        ctx.request_repaint();
    }));
}

fn clear(sub: &Submenu) {
    while sub.remove_at(0).is_some() {}
}

/// Lay the menus out for a mode: the top-level menus and what each one holds.
fn fill(menu: &Menu, h: &Handles, mode: u8) {
    while menu.items().len() > 1 {
        if menu.remove_at(1).is_none() {
            break;
        }
    }
    for (key, items) in layout(mode) {
        let Some(sub) = h.subs.get(key) else { continue };
        clear(sub);
        for i in items {
            let _ = match i {
                C(id) => sub.append(it(h, id)),
                Sep => sub.append(&PredefinedMenuItem::separator()),
                Sub(k) => match h.subs.get(k) {
                    Some(s) => sub.append(s),
                    None => Ok(()),
                },
            };
        }
    }
    for key in top_level(mode) {
        if let Some(s) = h.subs.get(key) {
            let _ = menu.append(s);
        }
    }
}

pub fn build() -> Option<Menu> {
    let mut h = Handles { mode: MODE_NONE, remove_label: "Remove from Collection", ..Handles::default() };
    for &(id, label, check) in COMMANDS {
        let e = if check { Entry::Check(CheckMenuItem::with_id(id, label, true, false, fixed_accelerator(id))) } else { Entry::Plain(MenuItem::with_id(id, label, true, fixed_accelerator(id))) };
        h.items.insert(id, e);
    }
    for (key, title) in SUBMENUS.iter().chain(std::iter::once(&HELP)) {
        h.subs.insert(key, Submenu::new(title, true));
    }
    let sep = PredefinedMenuItem::separator;
    let menu = Menu::new();
    let app_menu = Submenu::new("recordbox", true);
    app_menu
        .append_items(&[it(&h, "about"), &sep(), it(&h, "prefs"), &sep(), &PredefinedMenuItem::hide(None), &PredefinedMenuItem::hide_others(None), &PredefinedMenuItem::show_all(None), &sep(), &PredefinedMenuItem::quit(None)])
        .ok()?;
    menu.append(&app_menu).ok()?;
    fill(&menu, &h, 0);
    h.mode = 0;
    h.app_menu = Some(app_menu);
    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();
    HANDLES.with(|x| *x.borrow_mut() = h);
    Some(menu)
}

/// "Add To Playlist": the playlist tree with its folders (intelligent playlists left out).
fn fill_playlists(h: &Handles, app: &App) {
    fn level(sub: &Submenu, app: &App, parent: i64, depth: usize) {
        if depth > 32 {
            return;
        }
        for n in app.nodes.iter().filter(|n| n.parent == parent && n.kind != NodeKind::Smart) {
            if n.kind == NodeKind::Folder {
                let s = Submenu::new(&n.name, true);
                level(&s, app, n.id, depth + 1);
                let _ = sub.append(&s);
            } else {
                let _ = sub.append(&MenuItem::with_id(format!("addpl:{}", n.id), &n.name, true, None));
            }
        }
    }
    if let Some(sub) = h.subs.get("add_to_playlist") {
        clear(sub);
        level(sub, app, 0, 0);
    }
}

/// "Export Track" and "Export Playlist": one item per connected device.
fn fill_devices(h: &Handles, app: &App) {
    for (key, prefix) in [("export_track", "exptrk"), ("export_playlist", "exppl")] {
        if let Some(sub) = h.subs.get(key) {
            clear(sub);
            for (i, d) in app.devices.iter().enumerate() {
                let _ = sub.append(&MenuItem::with_id(format!("{prefix}:{i}"), &d.name, true, None));
            }
        }
    }
}

fn menu_signature<T: std::hash::Hash>(items: impl ExactSizeIterator<Item = T>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut state = std::collections::hash_map::DefaultHasher::new();
    items.len().hash(&mut state);
    for item in items {
        item.hash(&mut state);
    }
    state.finish() | 1
}

pub fn poll(app: &mut App) {
    while let Ok(ev) = menu_channel().1.try_recv() {
        let id: &str = ev.id().as_ref();
        handle(app, id);
    }
}

/// Native menu accelerators consume the keystrokes before egui sees them. Deliver their edit
/// events where the user is working, including in the immediate library viewport.
pub fn apply_edit_commands(app: &mut App, ctx: &egui::Context) {
    if !app.edit_events.is_empty() && ctx.input(|i| i.focused) {
        ctx.input_mut(|i| i.events.append(&mut app.edit_events));
    }
}

fn selected(app: &App) -> Vec<i64> {
    if app.selected.is_empty() {
        return Vec::new();
    }
    app.list.iter().copied().filter(|t| app.selected.contains(t)).collect()
}

/// The playlist, intelligent playlist or folder the tree has selected.
fn current_node(app: &App) -> Option<&PlaylistNode> {
    match &app.view {
        View::Playlist(id) => app.node(*id),
        _ => None,
    }
}

/// Whether the menu bar holds this keyboard action's shortcut as an accelerator (then the
/// key loop leaves it alone).
pub fn owns_shortcut(app: &App, action: &str) -> bool {
    app.menu.is_some() && crate::prefs_ext::MENU_ACTIONS.iter().any(|(a, _)| *a == action) && app.prefs.shortcut_for(action).map(|s| crate::prefs_ext::accelerator(&s).is_some()).unwrap_or(false)
}

/// The macOS menu bar is shared by all windows, including the detached library.
fn fullscreen_viewport(ctx: &egui::Context) -> Option<(egui::ViewportId, bool)> {
    ctx.input(|i| {
        let (&id, window) = if i.focused {
            (&i.raw.viewport_id, i.viewport())
        } else {
            i.raw.viewports.iter().find(|(_, v)| v.focused == Some(true))?
        };
        Some((id, window.fullscreen.unwrap_or(false)))
    })
}

/// The list shows collection tracks gathered by something other than a playlist (a history,
/// the Tag List, related tracks ...), which "Copy to Playlists category" can turn into one.
fn copyable_list(app: &App) -> bool {
    matches!(app.view, View::History(_) | View::TagList | View::Related | View::Matching | View::Suggest(_) | View::Category { .. }) && !app.list.is_empty()
}

/// (enabled, ticked) of every command for the app's present state.
pub fn state(app: &App) -> HashMap<&'static str, (bool, bool)> {
    let sel = selected(app);
    let has_sel = !sel.is_empty();
    let one = sel.len() == 1;
    let node = current_node(app);
    let manual = app.is_manual_playlist().is_some();
    let busy = app.dialogs.ext.backup_job.is_some();
    let layout = app.player_layout();
    let locked = has_sel && sel.iter().all(|id| app.track(*id).map(|t| t.grid_locked).unwrap_or(false));
    let unlocked = has_sel && sel.iter().all(|id| app.track(*id).map(|t| !t.grid_locked).unwrap_or(false));
    let any_missing = app.tracks.iter().any(|t| t.missing);
    let panel = crate::ui_browser_ext::right_panel(app);
    let perf = app.performance();
    let device_sel = matches!(app.view, View::Device { .. }) && !app.device_ui.sel.is_empty();
    let mut m = HashMap::new();
    for &(id, _, _) in COMMANDS {
        let v = match id {
            "backup" => (!busy, false),
            "restore" => (!busy && app.dialogs.library_sync_review.is_none() && app.rb_job.is_none(), false),
            "relocate_missing" | "auto_relocate" => (any_missing, false),
            "dupes" => (app.tracks.len() > 1 && !app.dupes.open, false),
            // Not in recordbox: backups of device libraries made by rekordbox, a rekordbox 5
            // library to bring over, and AlphaTheta's download servers.
            "device_backup_mgmt" | "import_v5" | "pack_sample_dl" | "pack_gc_dl" => (false, false),
            "pack_sample_import" | "pack_gc_import" => (perf && crate::app_hooks::pack_import_available(id), false),
            "export_xml" => (!app.tracks.is_empty(), false),
            "export_playlists_info" => (app.nodes.iter().any(|n| n.kind != NodeKind::Folder), false),
            "sync_rekordbox" => (app.can_review_library_sync(), false),
            "undo" => (!app.undo.is_empty(), false),
            "redo" => (!app.redo.is_empty(), false),
            "fullscreen" => (true, fullscreen_viewport(&app.ctx).is_some_and(|(_, fullscreen)| fullscreen)),
            "appearance_dark" => (!crate::ui_edit::is_edit(app), crate::ui_edit::is_edit(app) || app.prefs.rb.color_theme != 2),
            "appearance_light" => (!crate::ui_edit::is_edit(app), !crate::ui_edit::is_edit(app) && app.prefs.rb.color_theme == 2),
            "stem_vocals" | "stem_inst" => (has_sel && app.prefs.show_stems && !app.prefs.rb.library_protection, false),
            "stem_stop" => (crate::stem_export::busy(app), false),
            "layout_player" => (true, layout == 0),
            "layout_2players" => (true, layout == 1),
            "layout_dual" => (true, layout == 2),
            "layout_simple" => (true, layout == 3),
            "layout_browser" => (app.library_window.is_none(), layout == 4),
            "perf_2h" | "perf_2v" | "perf_4h" | "perf_4v" | "perf_browse" => {
                let k = ["perf_2h", "perf_2v", "perf_4h", "perf_4v", "perf_browse"].iter().position(|x| *x == id).unwrap_or(0) as u8;
                (perf && crate::app_hooks::perf_layout_available(k), perf && crate::app_hooks::perf_layout(app) == k)
            }
            "pad_platter" => (perf && crate::app_hooks::pad_platter_available(app), false),
            "split_screen" => (true, app.library_window.is_some()),
            "mytag_window" => (true, panel == 1),
            "related_subpanel" => (true, panel == 2),
            "suggest_subpanel" => (true, panel == 3),
            "info_window" => (true, panel == 4),
            "sub_browser" => (true, panel == 5),
            "sub_browser_h" => (true, app.prefs.bx.sub_horizontal),
            "automix" => (perf && crate::app_hooks::automix_available(), perf && crate::app_hooks::automix_shown(app)),
            "related_window" => (true, app.view == View::Related),
            "filter" => (true, app.filter_open),
            "wave_blue" => (true, app.prefs.wave_color == 0),
            "wave_rgb" => (true, app.prefs.wave_color == 1),
            "wave_3band" => (true, app.prefs.wave_color == 2),
            "import_to_collection" => (device_sel, false),
            "analyze" | "analyze_missing" | "analyze_key" | "export_tracks" | "reload_tags" | "remove_collection" => (has_sel, false),
            "look_up_match_tags" => (has_sel || !app.match_enrichment_status().is_empty(), false),
            "analysis_lock" => (has_sel, locked),
            "lock_on" => (has_sel && !locked, false),
            "lock_off" => (has_sel && !unlocked, false),
            "auto_on" => (true, app.prefs.auto_analysis),
            "auto_off" => (true, !app.prefs.auto_analysis),
            "stop_analysis" => (app.analysis_total > app.analysis_done, false),
            "load" => (one && !app.deck_options.disable_load, false),
            "relocate" | "show_finder" | "property" => (one, false),
            "delete_playlist" | "rename_playlist" => (node.is_some(), false),
            "export_playlist" | "save_playlist_info" | "save_info_txt" | "save_info_m3u8" => (node.map(|n| n.kind != NodeKind::Folder).unwrap_or(false), false),
            "copy_to_playlists" => (copyable_list(app), false),
            "sort_items" => (!app.nodes.is_empty(), false),
            "renumber" => (manual, false),
            _ => (true, false),
        };
        m.insert(id, v);
    }
    m
}

/// Whether a submenu can be opened in the app's present state.
fn sub_state(app: &App) -> [(&'static str, bool); 4] {
    let has_sel = !app.selected.is_empty() && app.list.iter().any(|t| app.selected.contains(t));
    let node = current_node(app).map(|n| n.kind != NodeKind::Folder).unwrap_or(false);
    let playlists = app.nodes.iter().any(|n| n.kind == NodeKind::Playlist);
    [("analysis_lock", has_sel), ("add_to_playlist", has_sel && playlists), ("export_track", has_sel && !app.devices.is_empty()), ("export_playlist", node && !app.devices.is_empty())]
}

/// Bring the menu bar in line with the app: enabled, ticks and accelerators. Cheap when
/// nothing changed.
pub fn sync(app: &App) {
    if app.menu.is_none() {
        return;
    }
    let want = state(app);
    let keys = format!("{:?}", app.prefs.keymap);
    HANDLES.with(|h| {
        let mut h = h.borrow_mut();
        let h = &mut *h;
        // The menus of the mode (rekordbox rebuilds its menu bar when the mode changes).
        let mode = menu_mode(app);
        if h.mode != mode {
            if let Some(menu) = &app.menu {
                fill(menu, h, mode);
            }
            crate::ui_edit::refill_menu(app, mode == 2);
            h.mode = mode;
        }
        let ps = menu_signature(app.nodes.iter().map(|n| (n.id, n.parent, &n.name, n.kind == NodeKind::Folder, n.kind == NodeKind::Smart)));
        if h.playlists_sig != ps {
            fill_playlists(h, app);
            h.playlists_sig = ps;
        }
        let ds = menu_signature(app.devices.iter().map(|d| (&d.root, &d.name)));
        if h.devices_sig != ds {
            fill_devices(h, app);
            h.devices_sig = ds;
        }
        for (key, en) in sub_state(app) {
            if h.subs_shown.get(key) != Some(&en) {
                if let Some(s) = h.subs.get(key) {
                    s.set_enabled(en);
                }
                h.subs_shown.insert(key, en);
            }
        }
        // "Remove from Collection" in the collection, "Remove from Playlist" in a playlist.
        let label = if app.is_manual_playlist().is_some() { "Remove from Playlist" } else { "Remove from Collection" };
        if h.remove_label != label {
            if let Some(Entry::Plain(i)) = h.items.get("remove_collection") {
                i.set_text(label);
            }
            h.remove_label = label;
        }
        for (id, (en, on)) in &want {
            if h.shown.get(id) == Some(&(*en, *on)) {
                continue;
            }
            match h.items.get(id) {
                Some(Entry::Plain(i)) => i.set_enabled(*en),
                Some(Entry::Check(i)) => {
                    i.set_enabled(*en);
                    i.set_checked(*on);
                }
                None => {}
            }
            h.shown.insert(*id, (*en, *on));
        }
        // A click on a ticked item flips its tick natively; put back what the state says.
        for (id, e) in &h.items {
            if let (Entry::Check(i), Some((_, on))) = (e, want.get(id)) {
                if i.is_checked() != *on {
                    i.set_checked(*on);
                }
            }
        }
        if h.keys != keys || h.keys.is_empty() {
            for (action, item) in crate::prefs_ext::MENU_ACTIONS {
                let acc = app.prefs.shortcut_for(action).and_then(|s| crate::prefs_ext::accelerator(&s));
                match h.items.get(item) {
                    Some(Entry::Plain(i)) => {
                        let _ = i.set_accelerator(acc);
                    }
                    Some(Entry::Check(i)) => {
                        let _ = i.set_accelerator(acc);
                    }
                    None => {}
                }
            }
            h.keys = if keys.is_empty() { " ".into() } else { keys };
        }
    });
}

const AUDIO_EXT: [&str; 9] = ["mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "alac", "mp4"];

pub fn handle(app: &mut App, id: &str) {
    if app.ext.cleanup.is_some() && !matches!(id, "edit_cut" | "edit_copy" | "edit_paste" | "edit_select_all") {
        return;
    }
    if crate::ui_edit::menu_command(app, id) { return; }
    let sel = selected(app);
    // Items of the filled-in submenus: "addpl:<playlist>", "exptrk:<device>", "exppl:<device>".
    if let Some((kind, n)) = id.split_once(':') {
        let Ok(n) = n.parse::<i64>() else { return };
        match kind {
            "addpl" => {
                if !sel.is_empty() {
                    app.add_to_playlist(n, &sel);
                }
            }
            "exptrk" | "exppl" => {
                let Some(root) = app.devices.get(n as usize).map(|d| d.root.clone()) else { return };
                if kind == "exptrk" {
                    crate::ui_browser::open_export(app, vec![], sel);
                } else if let Some(node) = current_node(app).cloned() {
                    crate::ui_browser::open_export(app, vec![node.id], vec![]);
                } else {
                    return;
                }
                app.dialogs.export_root = Some(root);
            }
            _ => {}
        }
        return;
    }
    match id {
        "dupes" => app.dupes_search(),
        "pack_sample_import" | "pack_gc_import" => crate::app_hooks::pack_import(app, id),
        "perf_2h" | "perf_2v" | "perf_4h" | "perf_4v" | "perf_browse" => {
            let k = ["perf_2h", "perf_2v", "perf_4h", "perf_4v", "perf_browse"].iter().position(|x| *x == id).unwrap_or(0) as u8;
            crate::app_hooks::set_perf_layout(app, k);
        }
        "pad_platter" => crate::app_hooks::toggle_pad_platter(app),
        "split_screen" => app.toggle_library_window = true,
        "automix" => crate::app_hooks::toggle_automix(app),
        "related_subpanel" | "suggest_subpanel" => {
            let k = if id == "related_subpanel" { 2 } else { 3 };
            let now = crate::ui_browser_ext::right_panel(app);
            crate::ui_browser_ext::set_right_panel(app, if now == k { 0 } else { k });
        }
        "sub_browser_h" => {
            let on = !app.prefs.bx.sub_horizontal;
            if on {
                crate::ui_browser_ext::set_right_panel(app, 0);
            }
            app.prefs.bx.sub_horizontal = on;
            app.save_prefs();
        }
        "import_to_collection" => {
            if let View::Device { root, .. } = app.view.clone() {
                let ids: Vec<u32> = app.device_ui.sel.iter().copied().collect();
                if !ids.is_empty() {
                    app.import_from_device(&root, &ids, vec![]);
                }
            }
        }
        "lock_on" | "lock_off" => {
            for t in &sel {
                if let Some(tr) = app.track_mut(*t) {
                    tr.grid_locked = id == "lock_on";
                }
                app.save_track(*t);
            }
        }
        "save_info_txt" | "save_info_m3u8" => {
            if let Some(n) = current_node(app).cloned() {
                let (ext, kind) = if id == "save_info_txt" { ("txt", "Text") } else { ("m3u8", "M3U8 playlist") };
                if let Some(f) = rfd::FileDialog::new().set_title("Save Playlist Info").set_file_name(format!("{}.{ext}", n.name)).add_filter(kind, &[ext]).save_file() {
                    app.export_playlist_file(n.id, &f);
                }
            }
        }
        "copy_to_playlists" => {
            if copyable_list(app) {
                if app.list_dirty {
                    app.rebuild_list();
                }
                let ids = app.list.clone();
                let name = crate::ui_browser::view_title(app);
                let name = name.split(" (").next().unwrap_or("Playlist").trim().to_string();
                match app.db.create_node(0, if name.is_empty() { "Playlist" } else { &name }, NodeKind::Playlist, None) {
                    Ok(pl) => {
                        let _ = app.db.set_playlist_tracks(pl, &ids);
                        app.reload_playlists();
                        app.side = SideMode::Playlists;
                        app.set_view(View::Playlist(pl));
                    }
                    Err(e) => app.toast_err(format!("The playlist could not be created: {e}")),
                }
            }
        }
        "about" | "h_about" => app.dialogs.about = true,
        "edit_cut" => app.edit_events.push(egui::Event::Cut),
        "edit_copy" => app.edit_events.push(egui::Event::Copy),
        "edit_paste" => {
            if let Some(text) = arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok()) {
                app.edit_events.push(egui::Event::Paste(text));
            }
        }
        "edit_select_all" => {
            let modifiers = egui::Modifiers { command: true, mac_cmd: true, ..Default::default() };
            for pressed in [true, false] {
                app.edit_events.push(egui::Event::Key { key: egui::Key::A, physical_key: None, pressed, repeat: false, modifiers });
            }
        }
        "undo" => app.undo_edit(),
        "redo" => app.redo_edit(),
        "trash" => app.dialogs.trash = Some(Default::default()),
        "prefs" => app.dialogs.prefs_open = true,
        "manual" => match crate::manual::open(&app.prefs) {
            Ok(_) => {}
            Err(e) => app.toast_err(format!("The manual could not be opened: {e}")),
        },
        "shortcuts" => app.dialogs.ext.shortcuts_open = true,
        "import_track" | "t_import_track" => {
            if let Some(files) = rfd::FileDialog::new().set_title("Import Track").add_filter("Audio", &AUDIO_EXT).pick_files() {
                let pl = app.is_manual_playlist();
                app.import_paths(files, pl);
            }
        }
        "import_folder" => {
            if let Some(d) = rfd::FileDialog::new().set_title("Import Folder").pick_folder() {
                let pl = app.is_manual_playlist();
                app.import_paths(vec![d], pl);
            }
        }
        "import_playlist" | "p_import_playlist" => {
            if let Some(f) = rfd::FileDialog::new().set_title("Import Playlist").add_filter("Playlist", &["m3u", "m3u8", "pls", "txt"]).pick_file() {
                let parent = crate::prefs_ext::current_parent(app);
                if let Some(id) = app.import_playlist_file(&f, parent) {
                    app.side = SideMode::Playlists;
                    app.set_view(View::Playlist(id));
                }
            }
        }
        "backup" => {
            if app.dialogs.ext.backup_job.is_none() {
                app.dialogs.ext.backup_ask = true;
            }
        }
        "restore" => {
            if app.dialogs.ext.backup_job.is_none() && app.dialogs.library_sync_review.is_none() && app.rb_job.is_none() {
                if let Some(f) = rfd::FileDialog::new().set_title("Open backup file").add_filter("recordbox library backup", &["zip"]).pick_file() {
                    match crate::backup::read_manifest(&f) {
                        Ok(_) => app.dialogs.ext.restore_ask = Some(f),
                        Err(e) => app.dialogs.ext.backup_done = Some(("Library Restoration".into(), format!("Unable to restore the collection.\n{e}"))),
                    }
                }
            }
        }
        "missing" => app.open_missing_file_manager(),
        "collection_cleanup" => crate::collection_cleanup::ui::open(app, None),
        "relocate_missing" => {
            if let Some(d) = rfd::FileDialog::new().set_title("Folder to search for the missing files").pick_folder() {
                app.relocate_missing(&d);
            }
        }
        "auto_relocate" => {
            // The selected missing tracks, or every missing track when none is selected.
            let picked: Vec<i64> = sel.iter().copied().filter(|id| app.track(*id).map(|t| t.missing).unwrap_or(false)).collect();
            app.auto_relocate(if picked.is_empty() { None } else { Some(picked) });
        }
        "relocate" => {
            if let Some(t) = sel.first().and_then(|t| app.track(*t)).cloned() {
                let mut d = rfd::FileDialog::new().set_title(format!("Relocate {}", t.display_title())).add_filter("Audio", &AUDIO_EXT);
                if let Some(p) = std::path::Path::new(&t.path).parent().filter(|p| p.is_dir()) {
                    d = d.set_directory(p);
                }
                if let Some(f) = d.pick_file() {
                    if app.relocate_track(t.id, &f.to_string_lossy()) {
                        app.toast("Track relocated.");
                    } else {
                        app.toast_err("That file could not be used for this track.");
                    }
                }
            }
        }
        "sync_rekordbox" => app.review_rekordbox_sync(),
        "import_xml" => {
            let mut d = rfd::FileDialog::new().add_filter("XML", &["xml"]);
            if let Some(p) = app.prefs.rekordbox_xml_path.as_deref().and_then(|p| std::path::Path::new(p).parent()).filter(|p| p.is_dir()) {
                d = d.set_directory(p);
            }
            if let Some(f) = d.pick_file() {
                app.import_xml(&f);
            }
        }
        "export_xml" => {
            if let Some(f) = rfd::FileDialog::new().set_file_name("collection.xml").save_file() {
                app.export_collection_xml(&f);
            }
        }
        "export_playlists_info" => {
            if let Some(d) = rfd::FileDialog::new().set_title("Export Playlists Information").pick_folder() {
                app.export_all_playlists_info(&d);
            }
        }
        "library_folder" => {
            let Some(folder) = app.db.conn.path().and_then(|path| std::path::Path::new(path).parent()) else {
                app.toast_err("The active library has no folder to show.");
                return;
            };
            if let Err(e) = crate::ui_browser::open_directory(folder) {
                app.toast_err(format!("Could not show the library folder: {e}"));
            }
        }
        "fullscreen" => {
            if let Some((id, fullscreen)) = fullscreen_viewport(&app.ctx) {
                crate::set_window_fullscreen(&app.ctx, id, app.library_window.as_mut(), !fullscreen);
            }
        }
        "appearance_dark" if !crate::ui_edit::is_edit(app) => app.prefs.rb.color_theme = 1,
        "appearance_light" if !crate::ui_edit::is_edit(app) => app.prefs.rb.color_theme = 2,
        "layout_player" => app.set_player_layout(0),
        "layout_2players" => app.set_player_layout(1),
        "layout_dual" => app.set_player_layout(2),
        "layout_simple" => app.set_player_layout(3),
        "layout_browser" => app.set_player_layout(4),
        "mytag_window" | "info_window" | "sub_browser" => {
            let k = match id {
                "mytag_window" => 1,
                "info_window" => 4,
                _ => 5,
            };
            let now = crate::ui_browser_ext::right_panel(app);
            crate::ui_browser_ext::set_right_panel(app, if now == k { 0 } else { k });
        }
        "related_window" => {
            if app.view == View::Related {
                app.side = SideMode::Collection;
                app.set_view(View::Collection);
            } else {
                app.side = SideMode::Related;
                app.set_view(View::Related);
            }
        }
        "filter" => {
            app.filter_open = !app.filter_open;
            app.list_dirty = true;
        }
        "wave_blue" | "wave_rgb" | "wave_3band" => {
            app.prefs.wave_color = match id {
                "wave_blue" => 0,
                "wave_rgb" => 1,
                _ => 2,
            };
            app.save_prefs();
        }
        "load" => {
            // rekordbox (`ListViewer::returnKeyPressed`): player A, or player B with Shift held
            // in the layouts that have two players.
            if let Some(t) = sel.first() {
                let shift = app.ctx.input(|i| i.modifiers.shift);
                if !app.performance() && app.two_players() && shift {
                    app.load_track_to(1, *t);
                } else {
                    app.load_track(*t);
                }
            }
        }
        "analyze" => app.reanalyze(&sel),
        "look_up_match_tags" => crate::ui_dialogs::open_match_lookup(app, &sel),
        "analyze_missing" => {
            app.analyze_missing(&sel);
        }
        "analyze_key" => app.analyze_key_tracks(&sel),
        "analysis_lock" => {
            let locked = sel.iter().all(|id| app.track(*id).map(|t| t.grid_locked).unwrap_or(false));
            for id in &sel {
                if let Some(t) = app.track_mut(*id) {
                    t.grid_locked = !locked;
                }
                app.save_track(*id);
            }
        }
        "auto_on" | "auto_off" => {
            app.prefs.auto_analysis = id == "auto_on";
            app.save_prefs();
        }
        "stop_analysis" => app.pool.stop(),
        "export_tracks" => crate::ui_browser::open_export(app, vec![], sel),
        "property" => {
            if let Some(t) = sel.first() {
                app.dialogs.info_track = app.track(*t).cloned();
            }
        }
        "reload_tags" => app.reload_tags_from_files(&sel),
        "show_finder" => {
            if let Some(t) = sel.first().and_then(|t| app.track(*t)) {
                if let Err(e) = crate::ui_browser::reveal_path(std::path::Path::new(&t.path)) {
                    app.toast_err(format!("Could not show the selected file: {e}"));
                }
            }
        }
        "remove_collection" => {
            if let Some(pl) = app.is_manual_playlist().filter(|_| !sel.is_empty()) {
                let s: std::collections::HashSet<i64> = sel.iter().copied().collect();
                app.remove_from_playlist(pl, &s);
            } else if !sel.is_empty() {
                app.dialogs.confirm_remove = Some(sel);
            }
        }
        "new_playlist" | "new_smart" | "new_folder" => {
            let parent = match current_node(app) {
                Some(n) if n.kind == NodeKind::Folder => n.id,
                _ => 0,
            };
            app.side = SideMode::Playlists;
            if parent != 0 {
                app.expanded.insert(parent);
            }
            app.create_node(parent, match id {
                "new_playlist" => NodeKind::Playlist,
                "new_smart" => NodeKind::Smart,
                _ => NodeKind::Folder,
            });
        }
        "delete_playlist" => {
            if crate::prefs_bridge::library_protected(app) {
                return;
            }
            if let Some(n) = current_node(app).cloned() {
                app.dialogs.confirm_delete_node = Some((n.id, n.name.clone(), n.kind == NodeKind::Folder));
            }
        }
        "rename_playlist" => {
            if crate::prefs_bridge::library_protected(app) {
                return;
            }
            if let Some(n) = current_node(app).cloned() {
                app.side = SideMode::Playlists;
                app.dialogs.rename = Some((n.id, n.name));
            }
        }
        "export_playlist" => {
            if let Some(n) = current_node(app).cloned() {
                crate::ui_browser::open_export(app, vec![n.id], vec![]);
            }
        }
        "sort_items" => {
            let parent = match current_node(app) {
                Some(n) if n.kind == NodeKind::Folder => n.id,
                Some(n) => n.parent,
                None => 0,
            };
            crate::prefs_ext::sort_items(app, parent);
        }
        "renumber" => {
            // The order shown (after sorting by a column) becomes the playlist's own order.
            if let Some(pl) = app.is_manual_playlist() {
                if app.list_dirty {
                    app.rebuild_list();
                }
                let order = app.list.clone();
                let all = app.db.playlist_tracks(pl).unwrap_or_default();
                if order.len() == all.len() {
                    let _ = app.db.set_playlist_tracks(pl, &order);
                    app.prefs.sort_col = Col::Num;
                    app.prefs.sort_asc = true;
                    app.save_prefs();
                    app.list_dirty = true;
                    app.toast("Track order renumbered.");
                } else {
                    app.toast_err("Clear the search and filter first: Renumber Track Order needs the whole playlist.");
                }
            }
        }
        "save_playlist_info" => {
            if let Some(n) = current_node(app).cloned() {
                if let Some(f) = rfd::FileDialog::new().set_title("Save Playlist Info").set_file_name(format!("{}.txt", n.name)).add_filter("Text (rekordbox)", &["txt"]).add_filter("M3U8 playlist", &["m3u8"]).add_filter("PLS playlist", &["pls"]).save_file() {
                    app.export_playlist_file(n.id, &f);
                }
            }
        }
        "export_device" => crate::ui_browser::open_export(app, vec![], vec![]),
        "stem_vocals" => crate::stem_export::request(app, &sel, crate::stem_export::Part::Vocals, false),
        "stem_inst" => crate::stem_export::request(app, &sel, crate::stem_export::Part::Instrumental, false),
        "stem_stop" => crate::stem_export::stop(app),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fullscreen_commands(output: egui::FullOutput) -> Vec<(egui::ViewportId, bool)> {
        output.viewport_output.iter().flat_map(|(&id, viewport)| {
            viewport.commands.iter().filter_map(move |cmd| match cmd {
                egui::ViewportCommand::Fullscreen(on) => Some((id, *on)),
                _ => None,
            })
        }).collect()
    }

    #[test]
    fn f11_toggles_the_focused_window_when_the_peer_is_already_in_the_target_state() {
        let root = egui::ViewportId::ROOT;
        let library = crate::DetachedLibrary::id();
        let preferences = egui::ViewportId::from_hash_of("recordbox_preferences");
        for (id, detached) in [(root, false), (root, true), (library, true), (preferences, true)] {
            let ctx = egui::Context::default();
            ctx.set_embed_viewports(false);
            let frame = |prefs: &Prefs, pressed, modifiers, focused, fullscreen: bool| {
                let mut raw = egui::RawInput { viewport_id: id, focused, ..Default::default() };
                // One window may have entered fullscreen through its native controls.
                let peer = if id == root { library } else { root };
                if detached {
                    raw.viewports.entry(peer).or_default().fullscreen = Some(!fullscreen);
                }
                let window = raw.viewports.entry(id).or_default();
                window.fullscreen = Some(fullscreen);
                window.focused = Some(focused);
                raw.events.push(egui::Event::Key { key: egui::Key::F11, physical_key: None, pressed, repeat: false, modifiers });
                let output = ctx.run(raw, |ctx| {
                    if id == root {
                        // Keep outputs observable even for an unrelated or recently reattached window.
                        for child in [library, preferences] {
                            ctx.show_viewport_deferred(child, Default::default(), |_, _| {});
                        }
                    }
                    egui::CentralPanel::default().show(ctx, |ui| ui.text_edit_singleline(&mut String::new()).request_focus());
                    assert!(ctx.wants_keyboard_input());
                    let mut window = detached.then(|| crate::DetachedLibrary::open(ctx, 256.0));
                    crate::window_fullscreen(prefs, ctx, window.as_mut());
                });
                fullscreen_commands(output)
            };
            let mut prefs = Prefs::default();
            let plain = egui::Modifiers::NONE;
            assert_eq!(frame(&prefs, true, plain, true, false), [(id, true)]);
            assert!(frame(&prefs, true, plain, true, true).is_empty()); // OS key repeat
            assert!(frame(&prefs, false, plain, true, true).is_empty());
            assert_eq!(frame(&prefs, true, plain, true, true), [(id, false)]);
            frame(&prefs, false, plain, true, false);
            assert!(frame(&prefs, true, plain, false, false).is_empty());
            frame(&prefs, false, plain, true, false);
            assert!(frame(&prefs, true, egui::Modifiers::SHIFT, true, false).is_empty());
            frame(&prefs, false, plain, true, false);
            prefs.keymap.insert("play".into(), "F11".into());
            assert!(frame(&prefs, true, plain, true, false).is_empty());
        }
    }

    #[test]
    fn f11_preserves_user_assigned_native_commands_only_in_the_active_mode() {
        use crate::prefs_rb::Mode;
        use crate::ui_prefs_keys as keys;
        let run = |prefs: &Prefs| {
            let child = crate::DetachedLibrary::id();
            let ctx = egui::Context::default();
            ctx.set_embed_viewports(false);
            let mut raw = egui::RawInput { viewport_id: child, focused: true, ..Default::default() };
            raw.viewports.entry(child).or_default().focused = Some(true);
            raw.events.push(egui::Event::Key {
                key: egui::Key::F11, physical_key: Some(egui::Key::F11), pressed: true,
                repeat: false, modifiers: egui::Modifiers::NONE,
            });
            let mut available = false;
            let output = ctx.run(raw, |ctx| {
                crate::window_fullscreen(prefs, ctx, None);
                available = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::F11, pressed: true, .. })));
            });
            (available, !fullscreen_commands(output).is_empty())
        };
        let mut prefs = Prefs::default();
        prefs.app_mode = 1;
        keys::select(&mut prefs, Mode::Performance, "Custom FX");
        let fx = keys::commands(Mode::Performance).into_iter().find(|r| r.id == 0xe000).unwrap();
        keys::assign(&mut prefs, Mode::Performance, "Custom FX", &fx, "F11");
        assert_eq!(run(&prefs), (true, false), "the performance FX command receives F11");
        prefs.app_mode = 0;
        assert_eq!(run(&prefs), (false, true), "this FX command is absent from Export");
        prefs.app_mode = 2;
        keys::select(&mut prefs, Mode::Edit, "Custom EDIT");
        let play = keys::commands(Mode::Edit).into_iter().find(|r| r.id == 0xcc15).unwrap();
        keys::assign(&mut prefs, Mode::Edit, "Custom EDIT", &play, "F11");
        assert_eq!(run(&prefs), (true, false), "the editor receives its assigned key");
        keys::assign(&mut prefs, Mode::Edit, "Custom EDIT", &play, "shift + F11");
        assert_eq!(run(&prefs), (false, true), "modified keys leave plain F11 available");
        prefs.app_mode = 1;
        keys::select(&mut prefs, Mode::Performance, keys::PRESETS[0].0);
        assert_eq!(run(&prefs), (false, true), "an inactive custom mapping does not claim F11");
    }

    #[test]
    fn paired_fullscreen_changes_the_peer_then_the_origin_only_while_the_pair_is_active() {
        let root = egui::ViewportId::ROOT;
        let library = crate::DetachedLibrary::id();
        let inputs = |focused, closing, root_fs, library_fs| {
            let mut raw = egui::RawInput { focused: focused == Some(root), ..Default::default() };
            for (id, fullscreen) in [(root, root_fs), (library, library_fs)] {
                let info = raw.viewports.entry(id).or_default();
                info.fullscreen = Some(fullscreen);
                info.focused = Some(focused == Some(id));
                if closing == Some(id) {
                    info.events.push(egui::ViewportEvent::Close);
                }
            }
            raw
        };
        for (origin, peer) in [(root, library), (library, root)] {
            for target in [true, false] {
                let cases = [
                    (Some(origin), None, false),
                    (Some(peer), None, false),
                    (None, None, false),
                    (Some(origin), Some(root), false),
                    (Some(origin), Some(library), false),
                    (Some(peer), None, true),
                ];
                for (focused, closing, origin_minimized) in cases {
                    let ctx = egui::Context::default();
                    ctx.set_embed_viewports(false);
                    let mut window = None;
                    let output = ctx.run(inputs(Some(origin), None, !target, !target), |ctx| {
                        ctx.show_viewport_deferred(library, Default::default(), |_, _| {});
                        let window = window.insert(crate::DetachedLibrary::open(ctx, 256.0));
                        crate::set_window_fullscreen(ctx, origin, Some(window), target);
                    });
                    assert_eq!(fullscreen_commands(output), [(peer, target)]);
                    let mut window = window.unwrap();

                    let mut next_frame = |focused, closing, minimized| {
                        let mut raw = inputs(focused, closing, if peer == root { target } else { !target }, if peer == library { target } else { !target });
                        raw.viewports.get_mut(&origin).unwrap().minimized = Some(minimized);
                        fullscreen_commands(ctx.run(raw, |ctx| {
                            ctx.show_viewport_deferred(library, Default::default(), |_, _| {});
                            if let Some(pending) = window.pending_fullscreen.take() {
                                pending.finish(ctx);
                            }
                        }))
                    };
                    let completion = next_frame(focused, closing, origin_minimized);
                    if focused.is_some() && closing.is_none() && !origin_minimized {
                        assert_eq!(completion, [(origin, target)]);
                    } else {
                        assert!(completion.is_empty());
                    }
                    // Refocusing a window later must never replay a completed or cancelled F11.
                    assert!(next_frame(Some(origin), None, false).is_empty());
                }
            }
        }
    }

    #[test]
    fn fullscreen_menu_uses_the_focused_childs_state() {
        let library = crate::DetachedLibrary::id();
        let mut raw = egui::RawInput { focused: false, ..Default::default() };
        raw.viewports.insert(library, egui::ViewportInfo { focused: Some(true), fullscreen: Some(true), ..Default::default() });
        let ctx = egui::Context::default();
        let _ = ctx.run(raw, |ctx| assert_eq!(fullscreen_viewport(ctx), Some((library, true))));
        let _ = ctx.run(egui::RawInput { focused: false, ..Default::default() }, |ctx| assert_eq!(fullscreen_viewport(ctx), None));
    }

    #[test]
    fn menu_signature_tracks_renames_and_reordering() {
        let mut rows = [(1, String::from("Opening")), (2, String::from("Main set"))];
        let initial = menu_signature(rows.iter());
        assert_ne!(initial, 0); // zero marks a submenu that has not been populated yet
        assert_eq!(initial, menu_signature(rows.iter()));
        rows[1].1.push_str(" B2B");
        let renamed = menu_signature(rows.iter());
        assert_ne!(initial, renamed);
        rows.swap(0, 1);
        assert_ne!(renamed, menu_signature(rows.iter()));
    }

    #[test]
    fn every_mode_lays_out_known_commands_and_submenus() {
        let cmds: std::collections::HashSet<&str> = COMMANDS.iter().map(|c| c.0).collect();
        let subs: std::collections::HashSet<&str> = SUBMENUS.iter().map(|s| s.0).chain(["help"]).collect();
        for mode in 0..3u8 {
            let lay = layout(mode);
            let filled: std::collections::HashSet<&str> = lay.iter().map(|l| l.0).collect();
            for (key, items) in &lay {
                assert!(subs.contains(key), "{key}");
                let mut seen = std::collections::HashSet::new();
                for i in items {
                    match i {
                        C(id) => {
                            assert!(cmds.contains(id), "{key}: {id}");
                            assert!(seen.insert(*id), "{key}: {id} twice");
                        }
                        Sub(k) => assert!(subs.contains(k), "{key}: submenu {k}"),
                        Sep => {}
                    }
                }
                assert!(!matches!(items.first(), Some(Sep)) && !matches!(items.last(), Some(Sep)), "{key} starts or ends with a separator");
            }
            for top in top_level(mode) {
                assert!(filled.contains(top), "{top} has no layout");
            }
        }
        // rekordbox's menu bar: Track and Playlist are not in EDIT mode; Performance adds its
        // layouts, Split Screen, Automix and the sample packs.
        assert_eq!(top_level(2), ["file", "edit", "view", "help"]);
        let has = |mode: u8, menu: &str, id: &str| layout(mode).iter().any(|(k, items)| *k == menu && items.iter().any(|i| matches!(i, C(x) if *x == id)));
        assert!(has(0, "view", "layout_dual") && !has(1, "view", "layout_dual"));
        assert!(has(1, "view", "perf_4v") && has(1, "view", "split_screen") && has(1, "view", "automix") && !has(0, "view", "automix"));
        for mode in 0..3 {
            assert!(has(mode, "view", "split_screen"));
            for action in ["collection_cleanup", "sync_rekordbox", "export_playlists_info", "dupes"] {
                assert!(has(mode, "file", action), "mode {mode} is missing {action}");
            }
        }
    }

    #[test]
    fn commands_are_unique_and_keyboard_actions_reach_them() {
        let mut ids = std::collections::HashSet::new();
        for &(id, label, _) in COMMANDS {
            assert!(ids.insert(id), "duplicate menu command {id}");
            assert!(!label.is_empty());
        }
        for (_, item) in crate::prefs_ext::MENU_ACTIONS {
            assert!(ids.contains(item), "{item} is not a menu command");
        }
        // Keyboard actions that go through the menu handler name real commands.
        for (id, _, _, _) in crate::prefs_ext::ACTIONS {
            if let Some(crate::prefs_ext::Act::Menu(m)) = crate::prefs_ext::Act::parse(id) {
                assert!(ids.contains(m), "{id} -> {m}");
            }
        }
        // The menu's own fixed accelerators do not collide with default keyboard shortcuts.
        for fixed in ["Cmd+N", "Cmd+E", "Cmd+I", "Cmd+R"] {
            assert!(!crate::prefs_ext::ACTIONS.iter().any(|a| a.2 == fixed), "{fixed}");
        }
    }
}
