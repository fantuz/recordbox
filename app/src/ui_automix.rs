//! The Automix playlist pane, its header buttons and the "Automix settings" dialog.
//!
//! rekordbox 7.2.19 (`checks/re-7.2.19/pads-sampler-automix.md` C2, and for this file
//! `browse::LowerContainer::resized` 0x101bf217c, `onAutomixBrowse` 0x101bf332c,
//! `AutomixListHeader::resized` / `paint` 0x100b755ac / 0x100b75250 and the dialog's
//! constructor and `resized` 0x101cb5df0 / 0x101cb67e0):
//!
//! - The pane is one of the windows beside the track list, opened by the sixth tile of the
//!   right-hand strip (PERFORMANCE mode only; `automix/autoMixBrows_btn`), and it takes half
//!   of the list's width (`onBrowseButtonClicked(-0.5)`).
//! - Its header is 26 pt: the title "Automix (<n> Tracks) " from 10 pt, then Start/Stop
//!   (22 x 22) at x0 = max(width - 168, title + 17), a 1 pt divider 20 pt before and 40 pt
//!   after it, Clear at x0 + 60, Save at x0 + 100 and Settings at x0 + 140 (20 x 20 each).
//! - The dialog is 504 x 266: title (20 pt bold) centred at y 16, three 16 pt check rows at
//!   y 48 / 80 / 112 with their labels from x 36, "Auto crossfade manual start" at y 164, the
//!   pull-down at (36, 184, 460 x 22) and OK (100 x 30) centred at y 228.
//!
//! There is no picture of the pane in the reference set: the positions above are rekordbox's
//! own numbers, the colours and the list's columns are recordbox's (they follow its other lists).

use crate::app::*;
use crate::automix::{Ask, State};
use crate::model::*;
use crate::theme::*;
use crate::ui_browser::{list_font, ROW_H};
use crate::ui_kit as kit;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui};

/// Where the pane's controls were last drawn (the scripted run aims at these).
#[derive(Default, Clone)]
pub struct PaneMap {
    /// The mixer's on-screen crossfader (for the scripted run).
    pub xfader: Option<egui::Rect>,
    pub tile: Option<Rect>,
    pub pane: Option<Rect>,
    pub play: Option<Rect>,
    pub clear: Option<Rect>,
    pub save: Option<Rect>,
    pub settings: Option<Rect>,
    pub rows: Vec<Rect>,
    /// The dialog's controls: repeat, random, double-click, pull-down, OK.
    pub dialog: [Option<Rect>; 5],
    /// Buttons of the question or alert on screen.
    pub buttons: Vec<Rect>,
    /// A playlist being dragged from the tree on the last frame.
    pub node_drag: Option<i64>,
}

const TIP_TILE: &str = "Display Automix window.";
const TIP_START: &str = "Start or stop Automix";
const TIP_CLEAR: &str = "Clear Automix playlist";
const TIP_SAVE: &str = "Save Automix playlist";
const TIP_SETTINGS: &str = "Change Automix settings";
const EMPTY: &str = "Drag and drop the playlist from Tree View";
pub const TIMING_ITEMS: [(u8, &str); 3] = [(2, "Start auto crossfade when loading a track to a standby deck."), (3, "Start auto crossfade when starting a track on a standby deck."), (1, "Disable auto crossfade manual start")];

/// The Automix glyph of the strip tile: two rings that pass through each other.
fn tile_icon(p: &Painter, r: Rect, c: Color32) {
    let k = r.width().min(r.height()) / 18.0;
    for dx in [-2.6f32, 2.6] {
        let o = r.center() + vec2(dx * k, 0.0);
        let pts: Vec<Pos2> = (0..=28).map(|i| { let a = i as f32 / 28.0 * std::f32::consts::TAU; o + vec2(a.cos() * 3.7 * k, a.sin() * 3.3 * k) }).collect();
        p.add(Shape::line(pts, Stroke::new(1.5 * k, c)));
    }
}

/// Start / Stop (`autoMixPlay_btn`): a ring with the Automix arrow; grey ring and glyph when
/// stopped, rekordbox's blue (0, 125, 225) while Automix runs.
fn play_icon(p: &Painter, r: Rect, on: bool) {
    let c = r.center();
    let (ring, glyph) = if on { (BEAT_BLUE, BEAT_BLUE) } else { (hex(0x4B4B4B), hex(0x969696)) };
    p.circle_stroke(c, 10.5, Stroke::new(1.0, ring));
    let o = r.min;
    // The two loops and the arrow head (skin coordinates, 22 x 22).
    for cx in [7.25f32, 11.9] {
        p.circle_stroke(o + vec2(cx, 11.0), 2.3, Stroke::new(1.8, glyph));
    }
    p.add(Shape::convex_polygon(vec![o + vec2(12.56, 5.01), o + vec2(18.99, 11.0), o + vec2(12.56, 16.99)], glyph, Stroke::NONE));
}

/// Clear (`playListClear_btn`, 17 x 16): a list sheet with a crossed disc at its corner.
fn clear_icon(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let o = r.center() - vec2(8.5, 8.0);
    p.rect_filled(Rect::from_min_max(o + vec2(0.0, 4.0), o + vec2(13.0, 16.0)), 0.0, c);
    for y in [6.0f32, 9.0, 12.0] {
        p.rect_filled(Rect::from_min_max(o + vec2(2.0, y), o + vec2(4.0, y + 2.0)), 0.0, bg);
        p.rect_filled(Rect::from_min_max(o + vec2(5.0, y), o + vec2(11.0, y + 2.0)), 0.0, bg);
    }
    p.circle_filled(o + vec2(12.0, 5.0), 5.6, bg);
    p.circle_filled(o + vec2(12.0, 5.0), 5.0, c);
    for (a, b) in [((9.5, 2.5), (14.5, 7.5)), ((14.5, 2.5), (9.5, 7.5))] {
        p.line_segment([o + vec2(a.0, a.1), o + vec2(b.0, b.1)], Stroke::new(1.4, bg));
    }
}

/// Save (`save_btn`, 13 x 13): a floppy disk.
fn save_icon(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let o = r.center() - vec2(6.5, 6.5);
    p.add(Shape::convex_polygon(vec![o, o + vec2(11.0, 0.0), o + vec2(13.0, 2.7), o + vec2(13.0, 13.0), o + vec2(0.0, 13.0)], c, Stroke::NONE));
    p.rect_filled(Rect::from_min_max(o + vec2(2.0, 0.0), o + vec2(11.0, 5.0)), 0.0, bg);
    p.rect_filled(Rect::from_min_max(o + vec2(6.0, 0.0), o + vec2(9.0, 3.0)), 0.0, c);
    p.rect_filled(Rect::from_min_max(o + vec2(2.0, 7.0), o + vec2(11.0, 11.0)), 0.0, bg);
}

/// The sixth tile of the right-hand strip. `y0`: centre of the strip's first tile.
pub fn strip_tile(app: &mut App, ui: &mut Ui, r: Rect, y0: f32) {
    if !app.performance() {
        app.mixx.automix.ui.tile = None;
        return;
    }
    let rr = Rect::from_center_size(pos2(r.min.x + 12.0, r.min.y + y0 + 34.0 * 5.0), vec2(18.0, 18.0));
    app.mixx.automix.ui.tile = Some(rr);
    let on = app.prefs.mixx.automix.pane;
    let resp = ui.interact(rr, ui.id().with("ricon_automix"), Sense::click());
    if on {
        ui.painter().rect_filled(rr.expand(1.0), 0.0, TILE_ON);
        ui.painter().rect_stroke(rr, 1.5, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
    } else {
        ui.painter().rect_filled(rr, 1.5, hex(0x212121));
        ui.painter().rect_stroke(rr, 1.5, Stroke::new(1.0, if resp.hovered() { hex(0x8C8C8C) } else { hex(0x565656) }), egui::StrokeKind::Inside);
    }
    tile_icon(ui.painter(), rr, WHITE);
    let resp = if app.prefs.tooltips { resp.on_hover_text(TIP_TILE) } else { resp };
    if resp.clicked() {
        toggle_pane(app);
    }
}

/// Show or hide the pane; it is one of the windows beside the list, so the others close.
pub fn toggle_pane(app: &mut App) {
    let on = !app.prefs.mixx.automix.pane;
    if on {
        crate::ui_browser_ext::set_right_panel(app, 0);
        app.automix_ensure_loaded();
    }
    app.prefs.mixx.automix.pane = on;
    app.save_prefs();
}

fn icon_button(ui: &mut Ui, r: Rect, id: &str, tip: &str, tips: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    if tips { resp.on_hover_text(tip) } else { resp }
}

/// The pane beside the track list. Takes the list's rectangle and returns what is left of it.
pub fn pane(app: &mut App, ui: &mut Ui, list: Rect) -> Rect {
    let m = &mut app.mixx.automix.ui;
    (m.pane, m.play, m.clear, m.save, m.settings) = (None, None, None, None, None);
    m.rows.clear();
    // One window at a time: another window opened since closes this one.
    if app.prefs.mixx.automix.pane && crate::ui_browser_ext::right_panel(app) != 0 {
        app.prefs.mixx.automix.pane = false;
    }
    let dragged_node = std::mem::replace(&mut app.mixx.automix.ui.node_drag, app.node_drag);
    if !app.prefs.mixx.automix.pane || !app.performance() || list.width() < 420.0 {
        return list;
    }
    app.automix_ensure_loaded();
    let pw = ((list.width() - 5.0) * 0.5).round();
    let r = Rect::from_min_max(pos2(list.max.x - pw, list.min.y), list.max);
    let rest = Rect::from_min_max(list.min, pos2(r.min.x - 5.0, list.max.y));
    app.mixx.automix.ui.pane = Some(r);
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    p.rect_filled(r, 0.0, BG);
    let tips = app.prefs.tooltips;

    // ---- header
    let bar = Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 26.0));
    p.rect_filled(bar, 0.0, BG2);
    let n = app.mixx.automix.list.len();
    let title = format!("Automix ({n} {}) ", if n == 1 { "Track" } else { "Tracks" });
    let g = p.layout_no_wrap(title, FontId::proportional(11.5), WHITE);
    let tw = g.size().x.min((bar.width() - 200.0).max(40.0));
    p.with_clip_rect(Rect::from_min_max(bar.min, pos2(bar.min.x + 10.0 + tw, bar.max.y))).galley(pos2(bar.min.x + 10.0, bar.center().y - g.size().y / 2.0), g, WHITE);
    let x0 = bar.min.x + (bar.width() - 168.0).max(tw + 17.0);
    for dx in [-20.0f32, 40.0] {
        p.rect_filled(Rect::from_min_size(pos2(x0 + dx, bar.min.y), vec2(1.0, bar.height())), 0.0, BG);
    }
    let running = app.automix_processing();
    let play = Rect::from_min_size(pos2(x0, bar.min.y + 2.0), vec2(22.0, 22.0));
    let resp = icon_button(ui, play, "amx_play", TIP_START, tips);
    play_icon(&p, play, running);
    if resp.clicked() {
        app.automix_start_stop();
    }
    let small = |dx: f32| Rect::from_min_size(pos2(x0 + dx, bar.min.y + 3.0), vec2(20.0, 20.0));
    let (cr, sr, gr) = (small(60.0), small(100.0), small(140.0));
    let grey = |resp: &egui::Response| if resp.hovered() { WHITE } else { hex(0x969696) };
    let resp = icon_button(ui, cr, "amx_clear", TIP_CLEAR, tips);
    clear_icon(&p, cr, grey(&resp), BG2);
    if resp.clicked() {
        // An empty list is simply cleared; otherwise rekordbox asks about saving it first.
        if app.mixx.automix.list.is_empty() {
            app.automix_clear();
        } else {
            app.mixx.automix.ask = Some(Ask::Clear);
        }
    }
    let resp = icon_button(ui, sr, "amx_save", TIP_SAVE, tips);
    save_icon(&p, sr, grey(&resp), BG2);
    if resp.clicked() {
        if let Some(id) = app.automix_save_as_playlist() {
            let name = app.node(id).map(|x| x.name.clone()).unwrap_or_default();
            app.toast(format!("Saved as playlist \"{name}\"."));
        }
    }
    let resp = icon_button(ui, gr, "amx_settings", TIP_SETTINGS, tips);
    crate::icons_src::gear(&p, gr.center(), grey(&resp), BG2);
    if resp.clicked() {
        app.mixx.automix.settings_open = true;
    }
    let m = &mut app.mixx.automix.ui;
    (m.play, m.clear, m.save, m.settings) = (Some(play), Some(cr), Some(sr), Some(gr));

    // ---- drops: tracks dragged from a list are added; a playlist dragged from the tree
    // becomes the Automix playlist (asked first when the list has tracks).
    let body = Rect::from_min_max(pos2(r.min.x, bar.max.y + 4.0), r.max);
    let ptr = ui.ctx().pointer_latest_pos();
    let released = ui.input(|i| i.pointer.any_released());
    let over = ptr.is_some_and(|pp| r.contains(pp));
    if over && (app.drag.is_some() || dragged_node.is_some() || app.node_drag.is_some()) {
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, BLUE), egui::StrokeKind::Inside);
    }
    if released && over {
        if let Some(ids) = app.drag.clone() {
            app.automix_add(&ids);
        } else if let Some(node) = dragged_node.or(app.node_drag) {
            select_as_automix(app, node);
        }
    }

    // ---- the list
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("automix_list"));
    bui.set_clip_rect(body.intersect(ui.clip_rect()));
    bui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let flex = (body.width() - 34.0 - 22.0 - 56.0 - 48.0 - 40.0 - 12.0).max(120.0);
    let cols: [(&str, f32, bool); 7] = [("#", 34.0, true), ("", 22.0, false), ("Track Title", (flex * 0.6).round(), false), ("Artist", (flex * 0.4).round(), false), ("BPM", 56.0, true), ("Time", 48.0, true), ("Key", 40.0, false)];
    let (hr, _) = bui.allocate_exact_size(vec2(bui.available_width(), 13.5), Sense::hover());
    bui.painter().rect_filled(hr, 0.0, BG2);
    let mut x = hr.min.x;
    for (c, w, right) in cols.iter() {
        if *right {
            bui.painter().text(pos2(x + w - 6.0, hr.center().y), Align2::RIGHT_CENTER, *c, FontId::proportional(11.0), WHITE);
        } else {
            bui.painter().text(pos2(x + 5.0, hr.center().y), Align2::LEFT_CENTER, *c, FontId::proportional(11.0), WHITE);
        }
        bui.painter().line_segment([pos2(x + w - 0.5, hr.min.y + 1.0), pos2(x + w - 0.5, hr.max.y - 1.0)], Stroke::new(1.0, BG));
        x += w;
    }
    if n == 0 {
        // `AutomixEmpty`.
        bui.painter().text(body.center(), Align2::CENTER_CENTER, EMPTY, FontId::proportional(12.0), DIM);
        return rest;
    }
    let font = list_font(app);
    let alnum = app.prefs.key_alnum;
    let on_deck: [Option<(usize, State)>; 2] = std::array::from_fn(|d| {
        let pl = &app.mixx.automix.engine.players[d];
        pl.row.filter(|_| running && pl.loading.is_none()).map(|row| (row, pl.state)).or(pl.loading.filter(|_| running).map(|(_, row)| (row, State::Reserved)))
    });
    let ids = app.mixx.automix.list.clone();
    let mut remove: Option<Vec<usize>> = None;
    let mut load: Option<(usize, usize)> = None;
    let mut rects = vec![];
    egui::ScrollArea::vertical().id_salt("automix_rows").auto_shrink([false, false]).show_rows(&mut bui, ROW_H, ids.len(), |ui, range| {
        for ri in range {
            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            rects.push(rr);
            let sel = app.mixx.automix.selected.contains(&ri);
            let deck = (0..2).find(|d| on_deck[*d].is_some_and(|(row, _)| row == ri));
            ui.painter().rect_filled(rr, 0.0, if sel { ROW_SEL } else if ri % 2 == 0 { BG1 } else { BG2 });
            let t = app.track(ids[ri]);
            let vals = [
                format!("{}", ri + 1),
                String::new(),
                t.map(|t| t.display_title()).unwrap_or_default(),
                t.map(|t| t.artist.clone()).unwrap_or_default(),
                t.filter(|t| t.bpm > 0.0).map(|t| format!("{:.2}", t.bpm)).unwrap_or_default(),
                t.filter(|t| t.duration > 0.0).map(|t| fmt_time(t.duration, false)).unwrap_or_default(),
                t.map(|t| t.key_label(alnum)).unwrap_or_default(),
            ];
            let missing = t.is_none_or(|t| t.missing);
            let mut x = rr.min.x;
            for (ci, (v, (_, w, right))) in vals.iter().zip(cols.iter()).enumerate() {
                let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(*w, rr.height()));
                let p = ui.painter().with_clip_rect(cr.shrink2(vec2(3.0, 0.0)));
                if ci == 1 {
                    // The deck the row is on: lit while it is the one playing out.
                    if let Some(d) = deck {
                        let active = on_deck[d].is_some_and(|(_, s)| s == State::Active);
                        let br = Rect::from_center_size(cr.center(), vec2(13.0, 11.0));
                        p.rect_filled(br, 1.5, if active { BLUE } else { hex(0x505050) });
                        p.text(br.center(), Align2::CENTER_CENTER, format!("{}", d + 1), caps(8.0), WHITE);
                    }
                } else if *right {
                    p.text(pos2(cr.max.x - 6.0, cr.center().y), Align2::RIGHT_CENTER, v, font.clone(), if missing { OFF } else { TEXT });
                } else {
                    p.text(pos2(cr.min.x + 6.0, cr.center().y), Align2::LEFT_CENTER, v, font.clone(), if missing { OFF } else { TEXT });
                }
                x += w;
            }
            if resp.clicked() || (resp.secondary_clicked() && !sel) {
                let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                let s = &mut app.mixx.automix;
                match (shift, s.anchor, cmd) {
                    (true, Some(a), _) => s.selected = (a.min(ri)..=a.max(ri)).collect(),
                    (_, _, true) => {
                        if !s.selected.insert(ri) {
                            s.selected.remove(&ri);
                        }
                        s.anchor = Some(ri);
                    }
                    _ => {
                        s.selected = [ri].into_iter().collect();
                        s.anchor = Some(ri);
                    }
                }
            }
            // Double-click loads to a deck only when the setting allows it, or with Alt held.
            if resp.double_clicked() && (app.prefs.mixx.automix.double_click_load || ui.input(|i| i.modifiers.alt)) {
                load = Some((ri, standby_deck(app)));
            }
            resp.context_menu(|ui| {
                for d in 0..2usize {
                    if ui.button(format!("Load to Deck {}", d + 1)).clicked() {
                        load = Some((ri, d));
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Delete tracks from the playlist").clicked() {
                    let s = &app.mixx.automix.selected;
                    remove = Some(if s.contains(&ri) { s.iter().copied().collect() } else { vec![ri] });
                    ui.close_menu();
                }
            });
        }
    });
    app.mixx.automix.ui.rows = rects;
    // Delete / Backspace removes the selected rows while the pointer is over the pane.
    if over && !app.mixx.automix.selected.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) && ui.memory(|m| m.focused().is_none()) {
        remove = Some(app.mixx.automix.selected.iter().copied().collect());
    }
    if let Some(mut rows) = remove {
        rows.sort_unstable_by(|a, b| b.cmp(a));
        for r in rows {
            if r < app.mixx.automix.list.len() {
                app.mixx.automix.list.remove(r);
            }
        }
        app.mixx.automix.selected.clear();
        app.automix_changed();
    }
    if let Some((row, deck)) = load {
        app.automix_load_row(row, deck);
    }
    rest
}

/// The deck a double-clicked row goes to: the one waiting while Automix runs, else the
/// active deck when it is deck 1 or 2.
fn standby_deck(app: &App) -> usize {
    let e = &app.mixx.automix.engine;
    if e.processing {
        return (0..2).find(|d| e.players[*d].state != State::Active).unwrap_or(1);
    }
    (app.px.active as usize).min(1)
}

/// "Select as Automix playlist" (tree menu, or a playlist dropped on the pane).
pub fn select_as_automix(app: &mut App, node: i64) {
    app.automix_ensure_loaded();
    if app.node(node).is_none() {
        return;
    }
    if app.mixx.automix.list.is_empty() {
        app.automix_assign(node);
    } else {
        app.mixx.automix.ask = Some(Ask::Assign(node));
    }
}

/// Tree menu of a playlist (PERFORMANCE mode).
pub fn node_menu(app: &mut App, ui: &mut Ui, node: i64) {
    if !app.performance() {
        return;
    }
    if ui.button("Select as Automix playlist").clicked() {
        select_as_automix(app, node);
        ui.close_menu();
    }
}

/// Track menu ("Add tracks to Automix playlist"; rekordbox notes it is "Only for Automix
/// playlists in the PERFORMANCE mode.").
pub fn track_menu(app: &mut App, ui: &mut Ui, sel: &[i64]) {
    if !app.performance() {
        return;
    }
    if ui.button("Add tracks to Automix playlist").clicked() {
        app.automix_add(sel);
        ui.close_menu();
    }
}

/// The dialog, the Yes / No / Cancel questions and the alerts.
pub fn windows(app: &mut App, ctx: &egui::Context) {
    app.mixx.automix.ui.buttons.clear();
    let center = Align2::CENTER_CENTER;
    if app.mixx.automix.settings_open {
        settings_dialog(app, ctx);
    }
    if let Some(ask) = app.mixx.automix.ask.clone() {
        let (title, text) = match ask {
            Ask::Clear => ("Clear current Automix playlist", "The current Automix playlist will be cleared."),
            Ask::Assign(_) => ("Assign a playlist", "The current Automix playlist will be overwritten."),
        };
        let mut answer = None;
        let mut rects = vec![];
        kit::dialog(ctx, title).id(egui::Id::new("automix_ask")).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(420.0);
            kit::message(ui, text);
            kit::message(ui, "Do you want to save the current Automix playlist as a playlist?");
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add_space(((ui.available_width() - 3.0 * 96.0 - 16.0) / 2.0).max(0.0));
                for (i, l) in ["Yes", "No", "Cancel"].iter().enumerate() {
                    let resp = ui.add(kit::btn(*l).min_size(vec2(96.0, 24.0)));
                    rects.push(resp.rect);
                    if resp.clicked() {
                        answer = Some(i);
                    }
                }
            });
        });
        app.mixx.automix.ui.buttons = rects;
        if let Some(a) = answer {
            app.mixx.automix.ask = None;
            if a == 0 {
                app.automix_save_as_playlist();
            }
            if a != 2 {
                match ask {
                    Ask::Clear => app.automix_clear(),
                    Ask::Assign(node) => app.automix_assign(node),
                }
            }
        }
    }
    if let Some(text) = app.mixx.automix.alert.clone() {
        let mut ok = false;
        let mut rects = vec![];
        kit::dialog(ctx, "recordbox").id(egui::Id::new("automix_alert")).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(380.0);
            kit::message(ui, &text);
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                let resp = ui.add(kit::btn("OK").min_size(vec2(96.0, 24.0)));
                rects.push(resp.rect);
                ok = resp.clicked();
            });
        });
        app.mixx.automix.ui.buttons = rects;
        if ok {
            app.mixx.automix.alert = None;
        }
    }
}

fn settings_dialog(app: &mut App, ctx: &egui::Context) {
    let (w, h) = (504.0f32, 266.0f32);
    let mut close = false;
    let mut map = [None; 5];
    egui::Window::new("automix_settings")
        .title_bar(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(egui::Frame::window(&ctx.style()).fill(BG2).stroke(Stroke::new(1.0, hex(0x3A3A3A))).inner_margin(0.0).corner_radius(4.0))
        .show(ctx, |ui| {
            kit::apply_style(ui);
            let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
            let p = ui.painter().clone();
            let o = r.min;
            p.text(pos2(o.x + w / 2.0, o.y + 26.0), Align2::CENTER_CENTER, "Automix settings", kit::inter_bold(17.0), WHITE);
            let o2 = app.prefs.mixx.automix.clone();
            let rows: [(f32, f32, &str, bool); 3] = [
                (48.0, 16.0, "Repeat playback", o2.repeat),
                (80.0, 16.0, "Random playback", o2.random),
                (112.0, 36.0, "Allow loading to deck by double-clicking an item on a Automix playlist (or pressing the Alt/Option key + double-click).", o2.double_click_load),
            ];
            for (i, (y, lh, label, on)) in rows.iter().enumerate() {
                let br = Rect::from_min_size(o + vec2(16.0, *y), vec2(16.0, 16.0));
                let lr = Rect::from_min_size(o + vec2(36.0, *y - 1.0), vec2(466.0 - 4.0, *lh));
                let resp = ui.interact(br.union(Rect::from_min_size(lr.min, vec2(lr.width(), 16.0))), ui.id().with(("amx_set", i)), Sense::click());
                kit::paint_checkbox(&p, Rect::from_center_size(br.center(), vec2(14.0, 14.0)), *on, true);
                let g = p.layout(label.to_string(), kit::inter(13.5), WHITE, lr.width());
                p.galley(pos2(lr.min.x + 4.0, if *lh > 16.0 { lr.min.y } else { br.center().y - g.size().y / 2.0 }), g, WHITE);
                map[i] = Some(br);
                if resp.clicked() {
                    let a = &mut app.prefs.mixx.automix;
                    match i {
                        0 => a.repeat = !a.repeat,
                        1 => a.random = !a.random,
                        _ => a.double_click_load = !a.double_click_load,
                    }
                    app.save_prefs();
                }
            }
            p.text(o + vec2(20.0, 172.0), Align2::LEFT_CENTER, "Auto crossfade manual start", kit::inter(13.5), WHITE);
            // The pull-down: rekordbox's order (ids 2, 3, 1).
            let cr = Rect::from_min_size(o + vec2(36.0, 184.0), vec2(460.0, 22.0));
            map[3] = Some(cr);
            let cur = match app.prefs.mixx.automix.crossfade_timing {
                1 => 1,
                3 => 3,
                _ => 2,
            };
            let label = TIMING_ITEMS.iter().find(|x| x.0 == cur).map(|x| x.1).unwrap_or("");
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(cr).layout(egui::Layout::left_to_right(egui::Align::Center)));
            kit::combo("AutomixManualFade", 460.0, label, 12.5).height(120.0).show_ui(&mut cui, |ui| {
                for (id, text) in TIMING_ITEMS {
                    if ui.selectable_label(cur == id, kit::text(text, 12.5)).clicked() {
                        app.prefs.mixx.automix.crossfade_timing = id;
                        app.save_prefs();
                    }
                }
            });
            let ok = Rect::from_min_size(o + vec2(w / 2.0 - 50.0, 228.0), vec2(100.0, 30.0));
            map[4] = Some(ok);
            let resp = ui.interact(ok, ui.id().with("amx_set_ok"), Sense::click());
            kit::paint_button(&p, ok, resp.hovered(), resp.is_pointer_button_down_on());
            p.text(ok.center(), Align2::CENTER_CENTER, "OK", kit::inter(13.0), WHITE);
            if resp.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
    app.mixx.automix.ui.dialog = map;
    if close {
        app.mixx.automix.settings_open = false;
    }
}

/// The fade-in and fade-out ranges on a deck's enlarged waveform while Automix runs
/// (`WidgetWaveView::drawAutoMixFadeAreaGL`; rekordbox's colours are set at run time and
/// were not read, so recordbox tints the ranges in the crossfade's blue).
pub fn wave_overlay(app: &App, ui: &Ui, deck: usize) {
    let [fi, fo] = app.automix_ranges(deck);
    if fi.is_none() && fo.is_none() {
        return;
    }
    let Some((r, pos, pps)) = app.deck_of(deck as u8).detail_map else { return };
    let p = ui.painter().with_clip_rect(r);
    let x_of = |t: f64| r.center().x + ((t - pos) * pps) as f32;
    for (range, rising) in [(fi, true), (fo, false)] {
        let Some((a, b)) = range else { continue };
        let (xa, xb) = (x_of(a), x_of(b));
        if xb < r.min.x || xa > r.max.x {
            continue;
        }
        p.rect_filled(Rect::from_min_max(pos2(xa, r.min.y), pos2(xb, r.max.y)), 0.0, BLUE.gamma_multiply(0.18));
        // The level through the range: up for the deck coming in, down for the one going out.
        let (ya, yb) = if rising { (r.max.y - 2.0, r.min.y + 2.0) } else { (r.min.y + 2.0, r.max.y - 2.0) };
        p.line_segment([pos2(xa, ya), pos2(xb, yb)], Stroke::new(1.0, BLUE));
        for x in [xa, xb] {
            p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.0, BLUE));
        }
    }
}
