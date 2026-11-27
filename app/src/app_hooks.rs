//! Small functions the menu bar calls for features that live in other modules: the
//! Performance layouts, the pad and platter switch, Automix and the sample pack imports.
//! One place to connect them keeps `menus.rs` free of the details.

use crate::app::*;

/// The Performance layouts in rekordbox's View menu order: 0 = 2 Deck Horizontal,
/// 1 = 2 Deck Vertical, 2 = 4 Deck Horizontal, 3 = 4 Deck Vertical, 4 = Browse
/// (ui_perf_layouts.rs).
pub fn perf_layout(app: &App) -> u8 {
    crate::ui_perf_layouts::layout(app).row() as u8
}

pub fn perf_layout_available(k: u8) -> bool {
    k < 5
}

pub fn set_perf_layout(app: &mut App, k: u8) {
    crate::ui_perf_layouts::set_row(app, k as usize);
}

/// "Show / Hide Pad and Platter" (rekordbox: the horizontal layouts only).
pub fn pad_platter_available(app: &App) -> bool {
    !app.prefs.perf_lay.vertical && !app.prefs.perf_lay.browse
}

pub fn toggle_pad_platter(app: &mut App) {
    app.prefs.perf_lay.hide_platter = !app.prefs.perf_lay.hide_platter;
    app.poll_decks();
    app.save_prefs();
}

/// View > Automix: the Automix playlist pane beside the track list (ui_automix.rs).
pub fn automix_available() -> bool {
    true
}

pub fn automix_shown(app: &App) -> bool {
    app.prefs.mixx.automix.pane
}

pub fn toggle_automix(app: &mut App) {
    crate::ui_automix::toggle_pane(app);
}

/// File > Additional contents > Sample Pack > Import: a Sampler Plus Pack into the sampler
/// library (sampler_lib.rs). The GROOVE CIRCUIT factory pack is unpacked from the installed
/// rekordbox by itself the first time its panel is shown, so its Import item has nothing to do.
pub fn pack_import_available(id: &str) -> bool {
    id == "pack_sample_import" || id == "pack_gc_import"
}

pub fn pack_import(app: &mut App, id: &str) {
    if id == "pack_sample_import" {
        if let Some(f) = rfd::FileDialog::new().add_filter("Sampler pack", &["zip"]).set_title("Import Sampler Plus Pack").pick_file() {
            app.smplib_import_pack(&f);
        }
    } else if id == "pack_gc_import" {
        // (Asks for the pack file itself, starting in rekordbox's own pack folder.)
        app.groove_import_pack(None);
    }
}

/// Playlist menu (PERFORMANCE) > Load the top 16 tracks into the current sampler bank: the
/// playlist's first sixteen tracks go to slots 1..16 of the bank that is showing.
pub fn playlist_to_sampler_bank(app: &mut App, playlist: i64) {
    let bank = app.prefs.sampler.bank as usize;
    let tracks: Vec<i64> = app.db.playlist_tracks(playlist).unwrap_or_default().into_iter().take(16).collect();
    for (slot, id) in tracks.into_iter().enumerate() {
        app.sampler_load(bank * 16 + slot, id);
    }
}

/// MIX POINT LINK: its top bar tile and strip (ui_mpl.rs). The tile works while the function
/// is enabled in Preferences > Controller > MIX POINT LINK.
pub fn mpl_available(app: &App) -> bool {
    app.prefs.mixx.mpl.enable
}

pub fn mpl_shown(app: &App) -> bool {
    crate::ui_mpl::strip_height(app) > 0.0
}

pub fn toggle_mpl(app: &mut App) {
    app.mpl_toggle_panel();
}

/// PERFORMANCE: a click on a deck's title area (artwork, title and artist) loads the track
/// selected in the list (rekordbox's tooltip there: "Load to the Deck:\n- Select a track and
/// click here.\n- Drag & Drop a track here."). Called while deck `n` is the one in place.
pub fn deck_title_click(app: &mut App, ui: &mut eframe::egui::Ui, r: eframe::egui::Rect, n: u8) {
    let resp = ui.interact(r, ui.id().with(("deckload", n)), eframe::egui::Sense::click());
    let resp = if app.prefs.tooltips { resp.on_hover_text("Load to the Deck:\n- Select a track and click here.\n- Drag & Drop a track here.") } else { resp };
    if resp.clicked() {
        load_selected_here(app);
    }
}

/// The track selected in the list onto the deck in place.
pub fn load_selected_here(app: &mut App) {
    if app.list_dirty {
        app.rebuild_list();
    }
    if let Some(id) = app.anchor_id.filter(|id| app.list.contains(id)) {
        app.load_track_here(id);
    }
}

/// Advanced > Browse > "Display the menu when long-pressing a track for 1 second." A track row
/// held for a second without moving opens its menu, as a right-click would (the click is
/// given to egui at the start of the next frame by `raw_input`).
pub fn long_press_menu(app: &mut App, ui: &eframe::egui::Ui, resp: &eframe::egui::Response, _id: i64, _row: usize) {
    if !app.prefs.rb.long_press_menu || !resp.is_pointer_button_down_on() || resp.dragged() {
        return;
    }
    let (now, start, down, pos) = ui.input(|i| (i.time, i.pointer.press_start_time(), i.pointer.primary_down(), i.pointer.interact_pos()));
    let (Some(start), true, Some(pos)) = (start, down, pos) else { return };
    if app.wiring.long_press_done == Some(start) {
        return;
    }
    if now - start >= 1.0 {
        app.wiring.long_press_done = Some(start);
        app.wiring.long_press = Some((ui.ctx().viewport_id(), pos));
        ui.ctx().request_repaint();
    } else {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// Before a frame: input that features ask for (the long press's right-click).
pub fn raw_input(app: &mut App, raw: &mut eframe::egui::RawInput) {
    use eframe::egui::{Event, PointerButton};
    if let Some((_, pos)) = app.wiring.long_press.take_if(|(viewport, _)| *viewport == raw.viewport_id) {
        let m = raw.modifiers;
        raw.events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: m });
        raw.events.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed: true, modifiers: m });
        raw.events.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, modifiers: m });
    }
}
