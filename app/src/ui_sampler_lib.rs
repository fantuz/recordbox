//! The Sampler tree and its lists on screen.
//!
//! What is known of rekordbox 7.2.19 (there is no picture of this tree in the reference set):
//! - PERFORMANCE mode has a "Sampler" tile in the source strip, the fifth, right after Track
//!   Suggestion (reference performance/07; Export mode's strip has none): nine squares of 4 pt
//!   on a 5 pt pitch.
//! - The tree's root is "Sampler" (`TreeViewer::addSamplerRoot` 0x10155a0b4); its rows are
//!   sorted "All Samples", "Capture", then the lists and folders by name
//!   (`SamplerItemSorter::compareElements` 0x101591f74).
//! - Menu texts: "Create New Sampler List", "Delete Sampler List", "Remove from Sampler List",
//!   "Add To Sampler List", "Load the top 16 tracks into the current sampler bank", and
//!   "Track Type" with "Oneshot Sample", "Loop Sample", "Normal Track"
//!   (`BrowsePopupMenuManager::addSamplerMenu` 0x1001147e4).
//! The list's columns and the rows' glyphs are recordbox's own.

use crate::app::*;
use crate::sampler_lib::*;
use crate::theme::*;
use crate::ui_browser::{list_font, ROW_H, TREE_FONT};
use crate::ui_kit as kit;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Rect, Sense, Stroke, Ui};

/// The strip tile's glyph: 3 x 3 squares.
pub fn tile_icon(p: &Painter, r: Rect, c: Color32, _bg: Color32) {
    let k = r.width().min(r.height()) / 18.0;
    let o = r.center() - vec2(9.0, 9.0) * k;
    for gy in 0..3 {
        for gx in 0..3 {
            p.rect_filled(Rect::from_min_size(o + vec2(2.0 + 5.0 * gx as f32, 2.0 + 5.0 * gy as f32) * k, vec2(4.0, 4.0) * k), 0.0, c);
        }
    }
}

/// The Sampler tile is shown in PERFORMANCE mode only (rekordbox's Export strip has none).
pub fn tile_shown(app: &App, name: &str) -> bool {
    name != "Sampler" || app.performance()
}

/// The tile was clicked: the tree shows the Sampler root, the list All Samples.
pub fn open(app: &mut App) {
    app.smplib_ready();
    if !matches!(app.view, View::Sampler(_)) {
        app.set_view(View::Sampler(ALL_SAMPLES));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Root(bool),
    All,
    Capture,
    Folder(bool),
    List,
}

/// One row of the tree, laid out like the playlist tree's rows (15 pt, 20 pt per level, the
/// glyph centred 31 pt in and the text from 43 pt, counts ending 30.5 pt from the right).
fn row(ui: &mut Ui, depth: usize, text: &str, count: Option<usize>, selected: bool, glyph: Glyph) -> egui::Response {
    use crate::icons_src as src;
    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 15.0), Sense::click());
    let p = ui.painter();
    let bg = if selected { ROW_SEL } else if resp.hovered() { hex(0x262626) } else { BG2 };
    if selected || resp.hovered() {
        p.rect_filled(rr, 0.0, bg);
    }
    let d = depth as f32 * 20.0;
    let cy = rr.center().y;
    let ic = pos2(rr.min.x + 31.0 + d, cy);
    let col = if selected { WHITE } else { TREE_ICON };
    let mut tx = rr.min.x + 43.0 + d;
    match glyph {
        Glyph::Root(open) => {
            src::tree_arrow(p, pos2(rr.min.x + 15.0 + d, cy), open, WHITE);
            tx = rr.min.x + 28.0 + d;
        }
        Glyph::Folder(open) => {
            src::tree_arrow(p, pos2(ic.x - 23.5, cy), open, WHITE);
            src::tree_folder(p, ic, col);
        }
        Glyph::List => src::tree_list(p, ic, col, bg),
        Glyph::All => {
            // Two sheets, one behind the other (`samplerAllSample_icon`).
            p.rect_filled(Rect::from_min_max(ic + vec2(-1.5, -1.0), ic + vec2(6.5, 5.5)), 0.0, col);
            p.rect_filled(Rect::from_min_max(ic + vec2(-6.5, -5.5), ic + vec2(2.0, 0.5)), 0.0, bg);
            p.rect_filled(Rect::from_min_max(ic + vec2(-6.0, -5.0), ic + vec2(1.5, 0.0)), 0.0, col);
            p.rect_filled(Rect::from_min_max(ic + vec2(-4.5, -3.0), ic + vec2(0.0, -2.0)), 0.0, bg);
        }
        Glyph::Capture => {
            // Scissors (`samplerCapture_icon`): two blades and two rings.
            p.line_segment([ic + vec2(-3.5, -5.0), ic + vec2(2.5, 2.5)], Stroke::new(1.3, col));
            p.line_segment([ic + vec2(3.5, -5.0), ic + vec2(-2.5, 2.5)], Stroke::new(1.3, col));
            p.circle_stroke(ic + vec2(-3.6, 3.6), 1.9, Stroke::new(1.2, col));
            p.circle_stroke(ic + vec2(3.6, 3.6), 1.9, Stroke::new(1.2, col));
        }
    }
    let right = if count.is_some() { rr.max.x - 62.0 } else { rr.max.x - 4.0 };
    p.with_clip_rect(Rect::from_min_max(pos2(tx, rr.min.y), pos2(right.max(tx + 20.0), rr.max.y))).text(pos2(tx, cy), Align2::LEFT_CENTER, text, rb(TREE_FONT), WHITE);
    if let Some(c) = count {
        p.text(pos2(rr.max.x - 30.5, cy), Align2::RIGHT_CENTER, c.to_string(), rb(TREE_FONT), WHITE);
    }
    resp
}

fn sorted_children(app: &App, parent: i64) -> Vec<SamplerList> {
    let mut v: Vec<SamplerList> = app.mixx.lib.lists.iter().filter(|l| l.parent == parent).cloned().collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    v
}

fn import_pack_dialog(app: &mut App) {
    if let Some(f) = rfd::FileDialog::new().add_filter("Sampler pack", &["zip"]).set_title("Import Sampler Plus Pack").pick_file() {
        app.smplib_import_pack(&f);
    }
}

/// The tree pane while the Sampler tile is chosen.
pub fn tree(app: &mut App, ui: &mut Ui, _drop: &mut Vec<(Rect, i64)>) {
    // The Sampler tree belongs to PERFORMANCE mode.
    if !app.performance() {
        app.side = SideMode::Playlists;
        app.set_view(View::Collection);
        return;
    }
    app.smplib_ready();
    let mut map: Vec<(String, Rect)> = vec![];
    let released = ui.input(|i| i.pointer.any_released());
    let ptr = ui.ctx().pointer_latest_pos();
    ui.add_space(2.0);
    egui::ScrollArea::vertical().id_salt("sampler_tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let open = !app.mixx.lib.root_closed;
        let root = row(ui, 0, "Sampler", None, false, Glyph::Root(open));
        map.push(("root".into(), root.rect));
        if root.clicked() {
            app.mixx.lib.root_closed = open;
        }
        root.context_menu(|ui| {
            if ui.button("Create New Sampler List").clicked() {
                app.smplib_create(0);
                ui.close_menu();
            }
            if ui.button("Import Sampler Plus Pack").clicked() {
                ui.close_menu();
                import_pack_dialog(app);
            }
        });
        if !open {
            return;
        }
        for (id, name, glyph) in [(ALL_SAMPLES, "All Samples", Glyph::All), (CAPTURE, "Capture", Glyph::Capture)] {
            let count = if id == ALL_SAMPLES { app.mixx.lib.counts.get(&ALL_SAMPLES).copied() } else { None };
            let r = row(ui, 1, name, count, app.view == View::Sampler(id), glyph);
            map.push((name.to_string(), r.rect));
            if r.clicked() {
                app.set_view(View::Sampler(id));
            }
            r.context_menu(|ui| {
                if ui.button("Load the top 16 tracks into the current sampler bank").clicked() {
                    app.smplib_load_top16(id);
                    ui.close_menu();
                }
            });
        }
        list_rows(app, ui, 0, 1, &mut map, released, ptr);
    });
    app.mixx.lib.ui = map;
}

fn list_rows(app: &mut App, ui: &mut Ui, parent: i64, depth: usize, map: &mut Vec<(String, Rect)>, released: bool, ptr: Option<egui::Pos2>) {
    for l in sorted_children(app, parent) {
        let folder = l.attribute == ATTR_FOLDER;
        let open = app.mixx.lib.open.contains(&l.id);
        let sel = app.view == View::Sampler(l.id);
        // Renaming in place.
        if app.mixx.lib.rename.as_ref().is_some_and(|(id, _)| *id == l.id) {
            let (rr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 15.0), Sense::hover());
            let er = Rect::from_min_max(pos2(rr.min.x + 41.0 + depth as f32 * 20.0, rr.min.y - 1.0), pos2(rr.max.x - 6.0, rr.max.y + 1.0));
            let mut text = app.mixx.lib.rename.as_ref().map(|x| x.1.clone()).unwrap_or_default();
            let resp = ui.put(er, egui::TextEdit::singleline(&mut text).font(rb(TREE_FONT)).margin(vec2(2.0, 0.0)));
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            let done = resp.lost_focus();
            if done {
                let name = text.trim();
                if !name.is_empty() && name != l.name && !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    let _ = rename_list(&app.db.conn, l.id, name);
                    app.smplib_reload();
                }
                app.mixx.lib.rename = None;
            } else {
                app.mixx.lib.rename = Some((l.id, text));
            }
            continue;
        }
        let count = if folder { None } else { Some(app.mixx.lib.counts.get(&l.id).copied().unwrap_or(0)) };
        let r = row(ui, depth, &l.name, count, sel, if folder { Glyph::Folder(open) } else { Glyph::List });
        map.push((l.name.clone(), r.rect));
        if r.clicked() {
            if folder {
                if open {
                    app.mixx.lib.open.remove(&l.id);
                } else {
                    app.mixx.lib.open.insert(l.id);
                }
            }
            app.set_view(View::Sampler(l.id));
        }
        if r.double_clicked() {
            app.mixx.lib.rename = Some((l.id, l.name.clone()));
        }
        // Tracks dragged from a list and let go on a sampler list are added to it.
        if !folder && app.drag.is_some() && ptr.is_some_and(|pp| r.rect.contains(pp)) {
            ui.painter().rect_stroke(r.rect, 2.0, Stroke::new(1.5, BLUE), egui::StrokeKind::Inside);
            if released {
                let ids = app.drag.clone().unwrap_or_default();
                let samples = app.smplib_samples_for_tracks(&ids);
                app.smplib_add(l.id, samples, false);
            }
        }
        r.context_menu(|ui| {
            if ui.button("Load the top 16 tracks into the current sampler bank").clicked() {
                app.smplib_load_top16(l.id);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Create New Sampler List").clicked() {
                app.smplib_create(if folder { l.id } else { l.parent });
                ui.close_menu();
            }
            if ui.button("Rename").clicked() {
                app.mixx.lib.rename = Some((l.id, l.name.clone()));
                ui.close_menu();
            }
            if ui.button("Delete Sampler List").clicked() {
                app.mixx.lib.ask_delete = Some(l.id);
                ui.close_menu();
            }
        });
        if folder && open {
            list_rows(app, ui, l.id, depth + 1, map, released, ptr);
        }
    }
}

fn view_name(app: &App, id: i64) -> String {
    match id {
        ALL_SAMPLES => "All Samples".into(),
        CAPTURE => "Capture".into(),
        _ => app.mixx.lib.lists.iter().find(|l| l.id == id).map(|l| l.name.clone()).unwrap_or_default(),
    }
}

/// The first empty slot of the current bank (None when the bank is full).
/// Note where something was drawn (the scripted run's targets).
pub fn note(app: &mut App, key: String, r: Rect) {
    let v = &mut app.mixx.lib.ui;
    v.retain(|x| x.0 != key);
    v.push((key, r));
}

pub fn note_slot(app: &mut App, idx: usize, r: Rect) {
    let v = &mut app.mixx.lib.slots_ui;
    v.retain(|x| x.0 != idx);
    v.push((idx, r));
}

/// Start dragging sampler list rows (see `list`).
pub fn start_drag(app: &mut App, picked: Vec<Sample>) {
    if picked.is_empty() {
        return;
    }
    let ids: Vec<i64> = picked.iter().map(|s| s.track_id).filter(|t| *t > 0 && app.track(*t).is_some()).collect();
    if ids.len() == picked.len() {
        app.drag = Some(ids);
    } else if let Some(s) = picked.iter().find(|s| s.track_id <= 0 || app.track(s.track_id).is_none()) {
        if let Some(id) = app.db.track_by_path(&s.path) {
            app.drag = Some(vec![id]);
        }
    }
    app.mixx.lib.drag = Some(picked);
    app.mixx.lib.drag_import = None;
}

/// Once per frame while sampler rows are dragged: the imported sample joins the drag, and
/// letting go ends it; a label follows the pointer.
pub fn drag_frame(app: &mut App, ctx: &egui::Context) {
    let Some(rows) = app.mixx.lib.drag.clone() else { return };
    // A sample that is not in the collection is imported once it is dragged over a deck (a
    // sampler slot takes the file as it is); the drag then carries the new track.
    let over_deck = ctx.pointer_latest_pos().map(|pp| app.px.sampler.deck_rects.iter().any(|r| r.contains(pp))).unwrap_or(false);
    if app.drag.is_none() && app.mixx.lib.drag_import.is_none() && over_deck {
        if let Some(s) = rows.first() {
            app.mixx.lib.drag_import = Some(s.path.clone());
            app.import_paths(vec![std::path::PathBuf::from(&s.path)], None);
        }
    }
    if let Some(path) = app.mixx.lib.drag_import.clone() {
        if let Some(id) = app.db.track_by_path(&path).filter(|id| app.track(*id).is_some()) {
            if app.drag.is_none() {
                app.drag = Some(vec![id]);
            }
            app.mixx.lib.drag_import = None;
        }
    }
    if let Some(pp) = ctx.pointer_latest_pos() {
        let lp = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("smplib_drag")));
        let name = if rows.len() == 1 { rows[0].name.clone() } else { format!("{} samples", rows.len()) };
        let r = Rect::from_min_size(pp + vec2(12.0, 6.0), vec2(8.0 + name.chars().count().min(40) as f32 * 6.5, 18.0));
        lp.rect_filled(r, 3.0, hex(0x2A2A2A));
        lp.text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, name, FontId::proportional(10.5), WHITE);
        ctx.request_repaint();
    }
    // (Kept through the frame the button is let go in, so that the slot under it sees it.)
    if !ctx.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
        app.mixx.lib.drag = None;
        app.mixx.lib.drag_import = None;
    }
}

fn free_slot(app: &App) -> Option<usize> {
    let bank = app.prefs.sampler.bank.min(3) as usize;
    (0..app.sampler_pads()).map(|k| bank * 16 + k).find(|i| !app.sampler_loaded(*i))
}

/// The track list while a row of the Sampler tree is chosen.
pub fn list(app: &mut App, ui: &mut Ui, id: i64) {
    let r = ui.max_rect();
    let bar = Rect::from_min_max(pos2(r.min.x, r.min.y + 5.0), pos2(r.max.x, r.min.y + 27.0));
    ui.painter().rect_filled(bar, 0.0, BG2);
    if !app.performance() {
        ui.painter().text(pos2(bar.min.x + 10.5, bar.center().y), Align2::LEFT_CENTER, "Sampler list is not available in Export mode.", FontId::proportional(11.5), WHITE);
        return;
    }
    app.smplib_ready();
    if app.mixx.lib.rows_for != Some(id) || app.mixx.lib.rows_dirty || id == CAPTURE {
        app.mixx.lib.rows = app.smplib_rows(id);
        if app.mixx.lib.rows_for != Some(id) {
            app.mixx.lib.selected.clear();
        }
        app.mixx.lib.rows_for = Some(id);
        app.mixx.lib.rows_dirty = false;
    }
    let rows = app.mixx.lib.rows.clone();
    let is_list = id > 0 && app.mixx.lib.lists.iter().any(|l| l.id == id && l.attribute == ATTR_LIST);
    ui.painter().text(pos2(bar.min.x + 10.5, bar.center().y), Align2::LEFT_CENTER, format!("Sampler › {} ({} {})", view_name(app, id), rows.len(), if rows.len() == 1 { "Track" } else { "Tracks" }), FontId::proportional(11.5), WHITE);
    let body = Rect::from_min_max(pos2(r.min.x, bar.max.y + 4.0), r.max);
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    bui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let flex = (body.width() - 34.0 - 26.0 - 70.0 - 60.0 - 16.0).max(300.0);
    let cols: [(&str, f32, bool); 6] = [("#", 34.0, true), ("", 26.0, false), ("Track Title", (flex * 0.4).round(), false), ("BPM", 70.0, true), ("Time", 60.0, true), ("Location", (flex * 0.6).round(), false)];
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
    if rows.is_empty() {
        return;
    }
    let font = list_font(app);
    let mut load: Option<(usize, usize)> = None;
    let mut set_type: Option<i64> = None;
    let mut remove = false;
    egui::ScrollArea::vertical().id_salt(("smp_rows", id)).auto_shrink([false, false]).show_rows(&mut bui, ROW_H, rows.len(), |ui, range| {
        for ri in range {
            let s = &rows[ri];
            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
            note(app, format!("row{ri}"), rr);
            // Dragged to a sampler slot (loads in its mode), a deck or a playlist (as the
            // collection track it is; a pack sample is imported first).
            if resp.drag_started() {
                if !app.mixx.lib.selected.contains(&ri) {
                    app.mixx.lib.selected = [ri].into_iter().collect();
                }
                let picked: Vec<Sample> = app.mixx.lib.selected.iter().filter_map(|i| rows.get(*i).cloned()).collect();
                start_drag(app, picked);
            }
            let sel = app.mixx.lib.selected.contains(&ri);
            ui.painter().rect_filled(rr, 0.0, if sel { ROW_SEL } else if ri % 2 == 0 { BG1 } else { BG2 });
            let vals = [format!("{}", ri + 1), String::new(), s.name.clone(), if s.bpm100 > 0 { format!("{:.2}", s.bpm100 as f64 / 100.0) } else { String::new() }, if s.duration > 0.0 { fmt_time(s.duration, false) } else { String::new() }, s.path.clone()];
            let mut x = rr.min.x;
            for (ci, (v, (_, w, right))) in vals.iter().zip(cols.iter()).enumerate() {
                let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(*w, rr.height()));
                let p = ui.painter().with_clip_rect(cr.shrink2(vec2(3.0, 0.0)));
                if ci == 1 {
                    // Track Type: an arrow for a one-shot, a closed loop for a loop sample.
                    let c = cr.center();
                    match s.mode {
                        MODE_LOOP => {
                            p.rect_stroke(Rect::from_center_size(c, vec2(11.0, 7.0)), 3.5, Stroke::new(1.2, TREE_ICON), egui::StrokeKind::Inside);
                        }
                        MODE_ONESHOT => {
                            p.line_segment([c + vec2(-5.0, 0.0), c + vec2(2.0, 0.0)], Stroke::new(1.2, TREE_ICON));
                            p.add(egui::Shape::convex_polygon(vec![c + vec2(1.0, -3.5), c + vec2(5.5, 0.0), c + vec2(1.0, 3.5)], TREE_ICON, Stroke::NONE));
                        }
                        _ => {}
                    }
                } else if *right {
                    p.text(pos2(cr.max.x - 6.0, cr.center().y), Align2::RIGHT_CENTER, v, font.clone(), TEXT);
                } else {
                    p.text(pos2(cr.min.x + 6.0, cr.center().y), Align2::LEFT_CENTER, v, font.clone(), TEXT);
                }
                x += w;
            }
            if resp.clicked() || (resp.secondary_clicked() && !sel) {
                let cmd = ui.input(|i| i.modifiers.command);
                let st = &mut app.mixx.lib.selected;
                if cmd {
                    if !st.insert(ri) {
                        st.remove(&ri);
                    }
                } else {
                    *st = [ri].into_iter().collect();
                }
            }
            // Double-click: into the first empty slot of the bank showing.
            if resp.double_clicked() {
                match free_slot(app) {
                    Some(idx) => load = Some((ri, idx)),
                    None => app.toast_err("Invalid operation. Load the sample to a vacant slot."),
                }
            }
            resp.context_menu(|ui| {
                ui.menu_button("Load to Sampler Slot", |ui| {
                    let bank = app.prefs.sampler.bank.min(3) as usize;
                    for k in 0..app.sampler_pads() {
                        if ui.button(format!("{}", k + 1)).clicked() {
                            load = Some((ri, bank * 16 + k));
                            ui.close_menu();
                        }
                    }
                });
                if s.id > 0 {
                    ui.menu_button("Track Type", |ui| {
                        for (m, name) in [(MODE_ONESHOT, "Oneshot Sample"), (MODE_LOOP, "Loop Sample"), (MODE_NORMAL, "Normal Track")] {
                            if ui.selectable_label(s.mode == m, name).clicked() {
                                set_type = Some(m);
                                ui.close_menu();
                            }
                        }
                    });
                }
                if is_list && ui.button("Remove from Sampler List").clicked() {
                    remove = true;
                    ui.close_menu();
                }
                if ui.button(crate::ui_browser::REVEAL_FILE).clicked() {
                    if let Err(err) = crate::ui_browser::reveal_path(std::path::Path::new(&s.path)) {
                        app.toast_err(format!("Could not open the file location: {err}"));
                    }
                    ui.close_menu();
                }
            });
        }
    });
    if let Some((ri, idx)) = load {
        if let Some(s) = rows.get(ri) {
            app.smplib_load_slot(idx, s);
        }
    }
    let picked: Vec<&Sample> = app.mixx.lib.selected.iter().filter_map(|i| rows.get(*i)).collect();
    if let Some(m) = set_type {
        let ids: Vec<i64> = picked.iter().map(|s| s.id).filter(|i| *i > 0).collect();
        let _ = set_mode(&app.db.conn, &ids, m);
        // A sample made from a collection track: the track menu's Track Type is the same one.
        let tracks: Vec<i64> = picked.iter().map(|s| s.track_id).filter(|t| *t > 0).collect();
        if !tracks.is_empty() {
            app.set_track_type(&tracks, crate::sampler_lib::type_of_mode(m));
        }
        app.smplib_reload();
    } else if remove {
        let members: Vec<i64> = picked.iter().map(|s| s.member).filter(|m| *m > 0).collect();
        let _ = remove_members(&app.db.conn, id, &members);
        app.mixx.lib.selected.clear();
        app.smplib_reload();
    }
}

/// "Add To Sampler List" in the track menu: the sampler lists, and a new one.
pub fn track_menu(app: &mut App, ui: &mut Ui, sel: &[i64]) {
    if !app.performance() || sel.is_empty() {
        return;
    }
    ui.menu_button("Add To Sampler List", |ui| {
        app.smplib_ready();
        let lists: Vec<SamplerList> = app.mixx.lib.lists.iter().filter(|l| l.attribute == ATTR_LIST).cloned().collect();
        for l in lists {
            if ui.button(&l.name).clicked() {
                let samples = app.smplib_samples_for_tracks(sel);
                app.smplib_add(l.id, samples, false);
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.button("Create New Sampler List").clicked() {
            let id = app.smplib_create(0);
            app.mixx.lib.rename = None;
            let samples = app.smplib_samples_for_tracks(sel);
            app.smplib_add(id, samples, true);
            ui.close_menu();
        }
    });
}

/// The questions of this area.
pub fn windows(app: &mut App, ctx: &egui::Context) {
    let center = Align2::CENTER_CENTER;
    if let Some(id) = app.mixx.lib.ask_delete {
        let mut answer = None;
        kit::dialog(ctx, "Delete Sampler List").id(egui::Id::new("smp_delete")).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(360.0);
            kit::message(ui, "Are you sure you want to delete this sampler list?");
            kit::message_note(ui, &format!("\"{}\". The samples stay under All Samples.", view_name(app, id)));
            answer = kit::dialog_buttons(ui, &["OK", "Cancel"]);
        });
        if answer == Some(0) {
            app.smplib_delete(id);
        }
        if answer.is_some() {
            app.mixx.lib.ask_delete = None;
        }
    }
    if let Some((list, samples)) = app.mixx.lib.ask_dup.clone() {
        let mut answer = None;
        kit::dialog(ctx, "Add To Sampler List").id(egui::Id::new("smp_dup")).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(400.0);
            kit::message(ui, "There are duplicates being added to the sampler list.");
            answer = kit::dialog_buttons(ui, &["Add", "Skip", "Cancel"]);
        });
        if let Some(a) = answer {
            app.mixx.lib.ask_dup = None;
            match a {
                0 => app.smplib_add(list, samples, true),
                1 => {
                    let dups = duplicates(&app.db.conn, list, &samples);
                    let rest: Vec<i64> = samples.into_iter().filter(|s| !dups.contains(s)).collect();
                    app.smplib_add(list, rest, true);
                }
                _ => {}
            }
        }
    }
}
