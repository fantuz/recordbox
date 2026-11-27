//! Browser windows and views matched to rekordbox 7.2.19 (references export/10-15): the My Tag
//! window, the Related Tracks and Track Suggestion windows, the playlist Column View, the
//! CUE Analysis Playlist's [Start] button, the "+" of a selected playlist and the sub-browser
//! (beside or under the browser). The logic is in browser_ext.rs.

use crate::app::*;
use crate::browser_ext::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_browser::{artwork_texture, dropdown, tree_row, TreeIcon, TREE_FONT};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, Rect, RichText, Sense, Stroke, Ui};
use std::collections::HashSet;

// ------------------------------------------------------------------------------ right-hand windows

fn panel_flags(app: &App) -> [bool; 5] {
    [app.prefs.show_mytag, app.prefs.bx.related_panel, app.prefs.bx.suggest_panel, app.prefs.info_panel, app.prefs.sub_browser]
}

/// Which window is open left of the right icon strip: 0 none, 1 My Tag, 2 Related Tracks,
/// 3 Track Suggestion, 4 Information, 5 Sub-browser.
pub fn right_panel(app: &App) -> u8 {
    panel_flags(app).iter().position(|f| *f).map(|i| i as u8 + 1).unwrap_or(0)
}

/// Opens one of the windows (closing the others), or closes them all with 0.
pub fn set_right_panel(app: &mut App, k: u8) {
    app.prefs.show_mytag = k == 1;
    app.prefs.bx.related_panel = k == 2;
    app.prefs.bx.suggest_panel = k == 3;
    app.prefs.info_panel = k == 4;
    app.prefs.sub_browser = k == 5;
    if k == 5 {
        app.prefs.bx.sub_horizontal = false;
    }
    app.ext.right_prev = panel_flags(app);
    app.save_prefs();
}

/// One window at a time: a window switched on elsewhere (menus, Preferences) closes the rest.
pub fn sync_right_panels(app: &mut App) {
    let now = panel_flags(app);
    if now.iter().filter(|f| **f).count() > 1 {
        let prev = app.ext.right_prev;
        let pick = (0..5).find(|i| now[*i] && !prev[*i]).or_else(|| (0..5).find(|i| now[*i])).unwrap_or(0);
        set_right_panel(app, pick as u8 + 1);
    } else if now[4] && app.prefs.bx.sub_horizontal {
        app.prefs.bx.sub_horizontal = false;
    }
    app.ext.right_prev = panel_flags(app);
}

/// Scroll bars of the windows: an 8 pt #323232 handle on black, 2 pt from the content and the edge.
pub fn bar_style(ui: &mut Ui) {
    ui.spacing_mut().scroll = egui::style::ScrollStyle {
        floating: false,
        bar_width: 8.0,
        bar_inner_margin: 2.0,
        bar_outer_margin: 2.0,
        foreground_color: false,
        dormant_background_opacity: 1.0,
        active_background_opacity: 1.0,
        interact_background_opacity: 1.0,
        dormant_handle_opacity: 1.0,
        active_handle_opacity: 1.0,
        interact_handle_opacity: 1.0,
        ..egui::style::ScrollStyle::solid()
    };
    ui.visuals_mut().extreme_bg_color = FIELD.c();
    ui.visuals_mut().widgets.inactive.bg_fill = gy(0x323232);
    ui.visuals_mut().widgets.hovered.bg_fill = gy(0x4A4A4A);
    ui.visuals_mut().widgets.active.bg_fill = gy(0x5A5A5A);
}

// ------------------------------------------------------------------------------ My Tag window

pub const MYTAG_FONT: f32 = 13.5;

/// A text field in a list row: returns the text once the field gives up focus.
fn row_edit(ui: &mut Ui, r: Rect, text: &mut String, hint: &str) -> bool {
    let resp = ui.put(r, egui::TextEdit::singleline(text).font(rb(12.0)).margin(egui::Margin::symmetric(3, 1)).hint_text(RichText::new(hint).color(OFF)));
    // Whether the field just gave up focus must be read before focus is asked for again.
    let gave_up = resp.lost_focus();
    if !gave_up {
        resp.request_focus();
    }
    gave_up
}

/// My Tag configuration window (reference 10): 223 pt wide; a 3 pt grip and a 1 pt line on its
/// left and top, then black. Each category is a 24 pt #191919 band with its name in grey; its
/// tags are 24 pt rows with a 12 pt #282828 box 8 pt in and the name 28.5 pt in; the last row of
/// a category carries the "create a new My Tag" disc 13.5 pt from the right. Clicking a row sets
/// or clears the tag on the selected tracks; Alt-click filters the list by it.
pub fn mytag_panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, BG2);
    let inner = Rect::from_min_max(pos2(r.min.x + 3.0, r.min.y + 3.0), pos2(r.max.x, r.max.y));
    p.rect_filled(inner, 0.0, BG);
    let body = Rect::from_min_max(pos2(r.min.x + 4.0, r.min.y + 4.0), pos2(r.max.x, r.max.y - 2.0));
    let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
    let cats: Vec<MyTag> = app.mytags.iter().filter(|t| t.parent == 0).cloned().collect();
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("mytag_win"));
    bui.set_clip_rect(body.intersect(ui.clip_rect()));
    bar_style(&mut bui);
    bui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let bg_resp = bui.interact(body, bui.id().with("mytag_bg"), Sense::click());
    egui::ScrollArea::vertical().id_salt("mytag_scroll").auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(&mut bui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let w = ui.available_width();
        for c in cats {
            // Category band.
            let (hr, hresp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
            ui.painter().rect_filled(hr, 0.0, gy(0x191919));
            let renaming = matches!(&app.dialogs.rename_tag, Some((id, _)) if *id == c.id);
            if renaming {
                let mut n = app.dialogs.rename_tag.clone().map(|x| x.1).unwrap_or_default();
                let done = row_edit(ui, hr.shrink2(vec2(6.0, 3.0)), &mut n, "");
                app.dialogs.rename_tag = Some((c.id, n.clone()));
                if done {
                    if !n.trim().is_empty() {
                        let _ = app.db.rename_mytag(c.id, n.trim());
                    }
                    app.dialogs.rename_tag = None;
                    app.reload_tags();
                }
            } else {
                ui.painter().with_clip_rect(hr).text(pos2(hr.min.x + 7.5, hr.center().y + 0.75), Align2::LEFT_CENTER, &c.name, rb(MYTAG_FONT), DIM.c());
            }
            if hresp.double_clicked() {
                app.dialogs.rename_tag = Some((c.id, c.name.clone()));
            }
            let cc = c.clone();
            hresp.context_menu(|ui| {
                if ui.button("Create a new My Tag.").clicked() {
                    app.dialogs.new_tag = Some((cc.id, String::new()));
                    ui.close_menu();
                }
                if ui.button("Rename").clicked() {
                    app.dialogs.rename_tag = Some((cc.id, cc.name.clone()));
                    ui.close_menu();
                }
                if ui.button("Delete").clicked() {
                    let _ = app.db.delete_mytag(cc.id);
                    app.reload_tags();
                    ui.close_menu();
                }
                ui.separator();
                panel_menu(app, ui);
            });
            let tags: Vec<MyTag> = app.mytags.iter().filter(|t| t.parent == c.id).cloned().collect();
            let adding = matches!(&app.dialogs.new_tag, Some((id, _)) if *id == c.id);
            let n_rows = tags.len().max(1);
            for row in 0..n_rows {
                let (rr, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
                let last = row + 1 == n_rows;
                // The disc that adds a tag to this category.
                let plus = Rect::from_center_size(pos2(rr.max.x - 13.5, rr.center().y), vec2(14.0, 14.0));
                let mut plus_clicked = false;
                if last && !adding {
                    let presp = ui.interact(plus, ui.id().with(("mytag_plus", c.id)), Sense::click());
                    crate::icons_src::plus_disc(ui.painter(), plus.center(), if presp.hovered() { WHITE.c() } else { gy(0xA0A0A0) }, BG.c());
                    let presp = if app.prefs.tooltips { presp.on_hover_text("Create a new My Tag.") } else { presp };
                    plus_clicked = presp.clicked();
                    if plus_clicked {
                        app.dialogs.new_tag = Some((c.id, String::new()));
                        ui.ctx().request_repaint();
                    }
                }
                let Some(tg) = tags.get(row) else { continue };
                if matches!(&app.dialogs.rename_tag, Some((id, _)) if *id == tg.id) {
                    let mut n = app.dialogs.rename_tag.clone().map(|x| x.1).unwrap_or_default();
                    let done = row_edit(ui, Rect::from_min_max(pos2(rr.min.x + 26.0, rr.min.y + 3.0), pos2(rr.max.x - 24.0, rr.max.y - 3.0)), &mut n, "");
                    app.dialogs.rename_tag = Some((tg.id, n.clone()));
                    if done {
                        if !n.trim().is_empty() {
                            let _ = app.db.rename_mytag(tg.id, n.trim());
                        }
                        app.dialogs.rename_tag = None;
                        app.reload_tags();
                    }
                    continue;
                }
                let count = sel.iter().filter(|t| app.track_tags.get(t).map(|s| s.contains(&tg.id)).unwrap_or(false)).count();
                let on = !sel.is_empty() && count == sel.len();
                let partial = count > 0 && !on;
                let filtering = app.filter_tags.contains(&tg.id);
                let pt = ui.painter();
                if resp.hovered() && !plus_clicked {
                    pt.rect_filled(rr, 0.0, gy(0x141414));
                }
                let bx = Rect::from_min_size(pos2(rr.min.x + 8.0, rr.center().y - 6.0), vec2(12.0, 12.0));
                pt.rect_filled(bx, 0.0, gy(0x282828));
                pt.rect_stroke(bx, 0.0, Stroke::new(0.5, gy(0x181818)), egui::StrokeKind::Inside);
                if on {
                    crate::icons_src::tick(pt, bx.center(), 12.0, WHITE.c());
                } else if partial {
                    pt.rect_filled(Rect::from_center_size(bx.center(), vec2(6.0, 1.5)), 0.0, DIM2);
                }
                pt.with_clip_rect(Rect::from_min_max(rr.min, pos2(rr.max.x - 24.0, rr.max.y))).text(pos2(rr.min.x + 28.5, rr.center().y + 0.75), Align2::LEFT_CENTER, &tg.name, rb(MYTAG_FONT), if filtering { CUE_ORANGE } else { WHITE.c() });
                if resp.clicked() && !plus_clicked {
                    if ui.input(|i| i.modifiers.alt) {
                        if !app.filter_tags.insert(tg.id) {
                            app.filter_tags.remove(&tg.id);
                        }
                        app.list_dirty = true;
                    } else if !sel.is_empty() {
                        app.set_tag(&sel, tg.id, !on);
                    }
                }
                if resp.double_clicked() && sel.is_empty() {
                    app.dialogs.rename_tag = Some((tg.id, tg.name.clone()));
                }
                let t2 = tg.clone();
                resp.context_menu(|ui| {
                    if ui.button(if app.filter_tags.contains(&t2.id) { "Stop filtering the list by this tag" } else { "Filter the list by this tag" }).clicked() {
                        if !app.filter_tags.insert(t2.id) {
                            app.filter_tags.remove(&t2.id);
                        }
                        app.list_dirty = true;
                        ui.close_menu();
                    }
                    if ui.button("Rename").clicked() {
                        app.dialogs.rename_tag = Some((t2.id, t2.name.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Delete").clicked() {
                        let _ = app.db.delete_mytag(t2.id);
                        app.reload_tags();
                        ui.close_menu();
                    }
                    ui.separator();
                    panel_menu(app, ui);
                });
            }
            if adding {
                let (rr, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                let mut n = app.dialogs.new_tag.clone().map(|x| x.1).unwrap_or_default();
                let done = row_edit(ui, Rect::from_min_max(pos2(rr.min.x + 26.0, rr.min.y + 3.0), pos2(rr.max.x - 24.0, rr.max.y - 3.0)), &mut n, "New My Tag");
                app.dialogs.new_tag = Some((c.id, n.clone()));
                if done {
                    if !n.trim().is_empty() {
                        let _ = app.db.add_mytag(c.id, n.trim());
                        app.reload_tags();
                    }
                    app.dialogs.new_tag = None;
                }
            }
        }
    });
    bg_resp.context_menu(|ui| panel_menu(app, ui));
}

/// Menu of the My Tag window itself.
fn panel_menu(app: &mut App, ui: &mut Ui) {
    if ui.button("Create a new column").clicked() {
        let id = app.db.add_mytag(0, "Untitled Column").unwrap_or(0);
        app.reload_tags();
        app.dialogs.rename_tag = Some((id, "Untitled Column".into()));
        ui.close_menu();
    }
    let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
    if ui.add_enabled(!sel.is_empty(), egui::Button::new("Add \"My Tag\" to the \"Comments\"")).clicked() {
        for id in &sel {
            let tags = app.tag_text(*id);
            if tags.is_empty() {
                continue;
            }
            if let Some(t) = app.track_mut(*id) {
                let add = format!("/* {tags} */");
                if !t.comment.contains(&add) {
                    t.comment = if t.comment.is_empty() { add } else { format!("{} {}", t.comment, add) };
                }
            }
            app.save_track(*id);
        }
        ui.close_menu();
    }
    if ui.add_enabled(!app.filter_tags.is_empty(), egui::Button::new("Show all tracks (clear the My Tag filter)")).clicked() {
        app.filter_tags.clear();
        app.list_dirty = true;
        ui.close_menu();
    }
}

// ------------------------------------------------------------------------------ Related / Suggestion

fn target_menu(app: &mut App, ui: &mut Ui, which: usize) {
    let cur = if which == 0 { app.prefs.bx.related_target } else { app.prefs.bx.suggest_target };
    let mut pick: Option<i64> = None;
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        if ui.selectable_label(cur == 0, "Collection").clicked() {
            pick = Some(0);
        }
        fn add(app: &App, ui: &mut Ui, parent: i64, depth: usize, cur: i64, pick: &mut Option<i64>) {
            for n in app.nodes.iter().filter(|n| n.parent == parent) {
                let label = format!("{}{}{}", "    ".repeat(depth), if n.kind == NodeKind::Folder { "▸ " } else { "" }, n.name);
                if ui.selectable_label(cur == n.id, label).clicked() {
                    *pick = Some(n.id);
                }
                if n.kind == NodeKind::Folder && depth < 12 {
                    add(app, ui, n.id, depth + 1, cur, pick);
                }
            }
        }
        add(app, ui, 0, 0, cur, &mut pick);
    });
    if let Some(t) = pick {
        if which == 0 {
            app.prefs.bx.related_target = t;
        } else {
            app.prefs.bx.suggest_target = t;
        }
        app.ext.panel_cache[which] = None;
        app.save_prefs();
        ui.close_menu();
    }
}

/// Related Tracks window (`which` 0, reference 11) and Track Suggestion window (1, reference
/// 12), 334 pt wide on #1E1E1E. Three 26 pt rows with 1 pt black lines between them: the list's
/// name behind an 18 pt #282828 tile (and, for Related Tracks, the criteria gear 25.5 pt from
/// the right behind a divider 60 pt from the right); the LIST / MASTER / A source buttons
/// (60 x 16 pt, the chosen one outlined in blue) with the source track's name after a note;
/// the search target list and what the rows show. Then 30 pt rows, alternately #0F0F0F and
/// #1E1E1E: artwork, title over artist, tempo and key.
pub fn related_window(app: &mut App, ui: &mut Ui, r: Rect, which: usize) {
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    p.rect_filled(r, 0.0, BG2);
    let (l, t, right) = (r.min.x, r.min.y, r.max.x);
    for y in [26.0, 53.0] {
        p.rect_filled(Rect::from_min_max(pos2(l, t + y), pos2(right, t + y + 1.0)), 0.0, BG);
    }
    let tips = app.prefs.tooltips;
    // Row 1: the list.
    let tile = Rect::from_min_size(pos2(l + 11.0, t + 4.0), vec2(18.0, 18.0));
    let tresp = ui.interact(Rect::from_min_max(tile.min, pos2(right - 62.0, tile.max.y)), ui.id().with(("relwin_tile", which)), Sense::click());
    if which == 0 {
        p.rect_filled(tile, 2.0, if tresp.hovered() { gy(0x3A3A3A) } else { BTN.c() });
        crate::icons_src::related(&p, tile, WHITE.c(), BTN.c());
        let name = app.prefs.related_lists.get(app.prefs.related_sel).map(|x| x.name.clone()).unwrap_or_else(|| "Related Tracks".into());
        p.with_clip_rect(Rect::from_min_max(pos2(l, t), pos2(right - 62.0, t + 26.0))).text(pos2(l + 30.5, t + 13.0), Align2::LEFT_CENTER, name, rb(SUB_NAME_FONT), WHITE.c());
        p.rect_filled(Rect::from_min_max(pos2(right - 60.0, t), pos2(right - 59.0, t + 26.0)), 0.0, BG);
        let gc = pos2(right - 25.5, t + 13.0);
        let gresp = ui.interact(Rect::from_center_size(gc, vec2(20.0, 20.0)), ui.id().with("relwin_gear"), Sense::click());
        crate::icons_src::gear(&p, gc, if app.ext.criteria_open { BLUE.c() } else { WHITE.c() }, BG2.c());
        let gresp = if tips { gresp.on_hover_text("Customize the criteria") } else { gresp };
        if gresp.clicked() {
            app.ext.criteria_open = !app.ext.criteria_open;
        }
        let tresp = if tips { tresp.on_hover_text("Change the criteria for \"Related tracks\"") } else { tresp };
        dropdown(ui, &tresp, "relwin_lists", |ui| {
            ui.label(RichText::new("Saved \"RELATED TRACKS\"").size(11.0).color(DIM));
            let names: Vec<String> = app.prefs.related_lists.iter().map(|x| x.name.clone()).collect();
            for (i, n) in names.iter().enumerate() {
                if ui.selectable_label(app.prefs.related_sel == i, n).clicked() {
                    app.related_select(i);
                    app.ext.panel_cache[0] = None;
                    app.save_prefs();
                }
            }
            ui.separator();
            if ui.button("Create New \"Related Tracks\"").clicked() {
                app.related_new(false);
                app.ext.criteria_open = true;
            }
            if ui.button("Copy the criteria to create new \"Related Tracks\"").clicked() {
                app.related_new(true);
                app.ext.criteria_open = true;
            }
            if ui.button("Turn the \"Related Tracks\" into a playlist").clicked() {
                if let Some(pl) = app.related_to_playlist() {
                    app.jump_to_playlist(pl);
                }
            }
        });
    } else {
        // Skin subBrows_trackSuggestion_btn: a 16 pt #323232 tile with a #969696 atom.
        let st = Rect::from_center_size(tile.center(), vec2(16.0, 16.0));
        p.rect_filled(st, 2.0, if tresp.hovered() { gy(0x3E3E3E) } else { gy(0x323232) });
        crate::icons_src::atom(&p, st.center(), 6.0, 2.2, 1.0, DIM.c());
        let kind = app.prefs.bx.suggest_kind.min(3);
        if kind > 0 {
            p.text(pos2(l + 30.5, t + 13.0), Align2::LEFT_CENTER, SUGGEST_KINDS[kind as usize], rb(SUB_NAME_FONT), WHITE.c());
        }
        let tresp = if tips { tresp.on_hover_text(SUGGEST_TIPS[kind as usize]) } else { tresp };
        dropdown(ui, &tresp, "sugwin_kinds", |ui| {
            ui.label(RichText::new("Specify how tracks are suggested").size(11.0).color(DIM));
            for (i, n) in SUGGEST_KINDS.iter().enumerate() {
                let label = if i == 0 { "Track Suggestion (Rank)" } else { n };
                let b = ui.selectable_label(kind as usize == i, label);
                let b = if tips { b.on_hover_text(SUGGEST_TIPS[i]) } else { b };
                if b.clicked() {
                    app.prefs.bx.suggest_kind = i as u8;
                    app.ext.panel_cache[1] = None;
                    app.save_prefs();
                }
            }
        });
    }
    // Row 2: whose tracks.
    let src_now = if which == 0 { app.prefs.bx.related_src } else { app.prefs.bx.suggest_src };
    let two = app.two_players();
    let src_now = if src_now == 3 && !two { 2 } else { src_now };
    let mut buttons: Vec<(u8, &str, f32, f32, &str)> = vec![
        (0, "LIST", 10.0, 60.0, "\"Related tracks\" of the selected track in the track list is displayed."),
        (1, "MASTER", 76.0, 60.0, "\"Related tracks\" of the tracks loaded on Master player is displayed."),
        (2, "A", 142.0, 16.0, "\"Related tracks\" of the tracks loaded on Player A is displayed."),
    ];
    if two {
        buttons.push((3, "B", 164.0, 16.0, "\"Related tracks\" of the tracks loaded on Player B is displayed."));
    }
    for (k, label, x, w, tip) in buttons {
        let br = Rect::from_min_size(pos2(l + x, t + 32.0), vec2(w, 16.0));
        let resp = ui.interact(br, ui.id().with(("relwin_src", which, k)), Sense::click());
        let on = src_now == k;
        p.rect_stroke(br, 2.0, Stroke::new(1.0, if on { BLUE.c() } else if resp.hovered() { gy(0x8C8C8C) } else { OFF.c() }), egui::StrokeKind::Inside);
        p.text(br.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, label, rb_bold(if k >= 2 { 9.75 } else { 9.0 }), if k >= 2 { BLUE.c() } else { WHITE.c() });
        let resp = if tips { resp.on_hover_text(tip) } else { resp };
        if resp.clicked() {
            if which == 0 {
                app.prefs.bx.related_src = k;
            } else {
                app.prefs.bx.suggest_src = k;
            }
            app.ext.panel_cache[which] = None;
            app.save_prefs();
        }
    }
    let src = app.panel_source(src_now);
    crate::icons_src::sub_note(&p, Rect::from_min_size(pos2(right - 78.5, t + 33.0), vec2(14.0, 14.0)), WHITE.c());
    let src_name = src.and_then(|id| app.track(id)).map(|x| x.display_title()).unwrap_or_default();
    p.text(pos2(right - 62.0, t + 40.0), Align2::LEFT_CENTER, format!(": {src_name}"), rb(SUB_NAME_FONT), WHITE.c());
    // Row 3: where to look and what to show.
    let tt = Rect::from_min_size(pos2(l + 11.0, t + 58.0), vec2(18.0, 18.0));
    let target = if which == 0 { app.prefs.bx.related_target } else { app.prefs.bx.suggest_target };
    let target = if app.node(target).is_some() { target } else { 0 };
    let tname = app.node(target).map(|n| n.name.clone()).unwrap_or_else(|| "Collection".into());
    let tr = ui.interact(Rect::from_min_max(tt.min, pos2(l + 160.0, tt.max.y)), ui.id().with(("relwin_target", which)), Sense::click());
    p.rect_filled(tt, 2.0, if tr.hovered() { gy(0x3A3A3A) } else { BTN.c() });
    for y in [5.0, 8.5, 12.0] {
        p.rect_filled(Rect::from_min_max(tt.min + vec2(4.5, y - 1.0), tt.min + vec2(13.5, y + 1.0)), 0.0, gy(0xC9C9C9));
    }
    p.with_clip_rect(Rect::from_min_max(pos2(l, t + 54.0), pos2(l + 160.0, t + 80.0))).text(pos2(l + 32.5, t + 67.0), Align2::LEFT_CENTER, tname, rb(SUB_NAME_FONT), WHITE.c());
    let tr = if tips { tr.on_hover_text("Select a search target list for \"Related tracks\"") } else { tr };
    dropdown(ui, &tr, if which == 0 { "relwin_target_menu" } else { "sugwin_target_menu" }, |ui| target_menu(app, ui, which));
    let info = if which == 0 { app.prefs.bx.related_info } else { app.prefs.bx.suggest_info }.min(4);
    let ir = ui.interact(Rect::from_min_max(pos2(l + 164.0, t + 58.0), pos2(l + 250.0, t + 76.0)), ui.id().with(("relwin_info", which)), Sense::click());
    crate::icons_src::sub_sort(&p, pos2(l + 174.0, t + 67.0), if ir.hovered() { WHITE.c() } else { DIM.c() });
    p.text(pos2(l + 185.0, t + 67.0), Align2::LEFT_CENTER, INFO_CHOICES[info as usize], rb(SUB_NAME_FONT), WHITE.c());
    dropdown(ui, &ir, if which == 0 { "relwin_info_menu" } else { "sugwin_info_menu" }, |ui| {
        for (i, n) in INFO_CHOICES.iter().enumerate() {
            if ui.selectable_label(info as usize == i, *n).clicked() {
                if which == 0 {
                    app.prefs.bx.related_info = i as u8;
                } else {
                    app.prefs.bx.suggest_info = i as u8;
                }
                app.save_prefs();
            }
        }
    });
    // Rows (re-read at most once a second, or when what they depend on changes).
    let now = ui.input(|i| i.time);
    let key = format!("{:?}|{}|{}|{:?}|{}|{}", src, target, app.tracks.len(), if which == 0 { format!("{:?}", app.prefs.related) } else { app.prefs.bx.suggest_kind.to_string() }, app.ext.matches.len(), app.prefs.related_sel);
    let rows = match app.ext.panel_cache[which].clone().filter(|c| c.0 == key && now - c.1 < 1.0) {
        Some(c) => c.2,
        None => {
            let v: Vec<(i64, f64)> = match src {
                None => vec![],
                Some(s) if which == 0 => app.related_for(s, target).into_iter().map(|id| (id, 0.0)).collect(),
                Some(s) => app.suggestions(s, target, app.prefs.bx.suggest_kind),
            };
            let v = std::sync::Arc::new(v);
            app.ext.panel_cache[which] = Some((key, now, v.clone()));
            v
        }
    };
    let body = Rect::from_min_max(pos2(l, t + 80.0), pos2(right, r.max.y));
    p.rect_filled(body, 0.0, BG);
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt(("relwin_body", which)));
    bui.set_clip_rect(body.intersect(ui.clip_rect()));
    bar_style(&mut bui);
    bui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let alnum = app.prefs.key_alnum;
    egui::ScrollArea::vertical().id_salt(("relwin_rows", which)).auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show_rows(&mut bui, 30.0, rows.len(), |ui, range| {
        for i in range {
            let (id, score) = rows[i];
            let Some(tr) = app.track(id).cloned() else { continue };
            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click_and_drag());
            let sel = app.ext.panel_sel[which] == Some(id);
            // (A selected row keeps rekordbox's fixed blue and white text in Light: `ListViewer::findTextColour`.)
            let _sel_dark = crate::theme::dark_scope(sel);
            let pt = ui.painter().with_clip_rect(rr.intersect(ui.clip_rect()));
            pt.rect_filled(rr, 0.0, if sel { ROW_SEL } else if i % 2 == 0 { BG1 } else { BG2 });
            let ar = Rect::from_min_size(rr.min, vec2(30.0, 30.0));
            match artwork_texture(app, &tr.artwork) {
                Some(tex) => {
                    pt.image(tex.id(), ar, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    pt.rect_filled(ar, 0.0, gy(0x101010));
                    pt.circle_filled(ar.center(), 10.0, gy(0x1E1E1E));
                    pt.circle_filled(ar.center(), 2.6, gy(0x101010));
                }
            }
            let text_right = rr.max.x - 108.0;
            pt.with_clip_rect(Rect::from_min_max(rr.min, pos2(rr.max.x - 6.0, rr.max.y)).intersect(ui.clip_rect())).text(pos2(rr.min.x + 35.5, rr.min.y + 8.5), Align2::LEFT_CENTER, tr.display_title(), rb(SUB_TITLE_FONT), if tr.missing { DIM.c() } else { WHITE.c() });
            pt.with_clip_rect(Rect::from_min_max(rr.min, pos2(text_right, rr.max.y)).intersect(ui.clip_rect())).text(pos2(rr.min.x + 35.5, rr.min.y + 21.5), Align2::LEFT_CENTER, &tr.artist, rb(SUB_ARTIST_FONT), WHITE.c());
            // The right-hand value: BPM and key (Auto) or another column; the window's right
            // edge is 12 pt past the row (BPM ends 70 pt and the key starts 59.5 pt before it).
            let edge = rr.max.x + 12.0;
            let y = rr.min.y + 22.25;
            match info {
                0 => {
                    if tr.bpm > 0.0 {
                        pt.text(pos2(edge - 70.0, y), Align2::RIGHT_CENTER, format!("{:.2}", tr.bpm), rb(SUB_ARTIST_FONT), WHITE.c());
                    }
                    pt.text(pos2(edge - 59.5, y), Align2::LEFT_CENTER, tr.key_label(alnum), rb(SUB_ARTIST_FONT), WHITE.c());
                }
                1 => {
                    pt.text(pos2(edge - 20.0, y), Align2::RIGHT_CENTER, &tr.genre, rb(SUB_ARTIST_FONT), WHITE.c());
                }
                2 => {
                    for s in 0..5u8 {
                        crate::icons_src::star(&pt, pos2(edge - 98.0 + s as f32 * 14.0, y), tr.rating > s, if tr.rating > s { WHITE.c() } else { OFF.c() });
                    }
                }
                3 => {
                    if tr.duration > 0.0 {
                        pt.text(pos2(edge - 20.0, y), Align2::RIGHT_CENTER, fmt_time(tr.duration, false), rb(SUB_ARTIST_FONT), WHITE.c());
                    }
                }
                _ => {
                    pt.text(pos2(edge - 20.0, y), Align2::RIGHT_CENTER, tr.date_added.get(..10).unwrap_or(&tr.date_added), rb(SUB_ARTIST_FONT), WHITE.c());
                }
            }
            if which == 1 && tips && resp.hovered() {
                resp.clone().on_hover_text(format!("{} {:.0} %", SUGGEST_KINDS[app.prefs.bx.suggest_kind.min(3) as usize], (score.min(1.0)) * 100.0));
            }
            drop(_sel_dark);
            if resp.clicked() {
                app.ext.panel_sel[which] = Some(id);
            }
            if resp.double_clicked() {
                app.load_track(id);
            }
            if resp.drag_started() && app.drag.is_none() {
                app.ext.panel_sel[which] = Some(id);
                app.drag = Some(vec![id]);
                app.sub_drag = true;
            }
            resp.context_menu(|ui| {
                app.ext.panel_sel[which] = Some(id);
                crate::ui_browser::track_menu_items(app, ui, &[id]);
            });
        }
    });
    // The criteria of the selected "Related Tracks", in a small window by the gear.
    if which == 0 && app.ext.criteria_open {
        let mut open = true;
        egui::Window::new("Criteria").id(egui::Id::new("relwin_criteria")).open(&mut open).collapsible(false).resizable(false).default_pos(pos2(right - 290.0, t + 30.0)).show(ui.ctx(), |ui| {
            crate::ui_library_ext::related_criteria(app, ui);
        });
        if !open {
            app.ext.criteria_open = false;
        }
    }
}

pub const SUB_NAME_FONT: f32 = 11.5;
pub const SUB_TITLE_FONT: f32 = 11.5;
pub const SUB_ARTIST_FONT: f32 = 9.75;

/// Track Suggestion in the tree pane: the Rank list and its Era / Mood / Association lists.
pub fn suggest_tree(app: &mut App, ui: &mut Ui) {
    ui.add_space(1.0);
    let tips = app.prefs.tooltips;
    let root = tree_row(ui, 0, "Track Suggestion", None, app.view == View::Suggest(0), TreeIcon::Open, 15.0);
    let root = if tips { root.on_hover_text(SUGGEST_TIPS[0]) } else { root };
    if root.clicked() {
        app.set_view(View::Suggest(0));
    }
    for k in 1..4u8 {
        let r = tree_row(ui, 1, SUGGEST_KINDS[k as usize], None, app.view == View::Suggest(k), TreeIcon::Playlist, 15.0);
        let r = if tips { r.on_hover_text(SUGGEST_TIPS[k as usize]) } else { r };
        if r.clicked() {
            app.set_view(View::Suggest(k));
        }
    }
    ui.add_space(6.0);
    let name = app.related_source().and_then(|id| app.track(id)).map(|t| t.display_title());
    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(match name {
            Some(n) => format!("Ranks every track in your library according to how well it matches with \"{n}\"."),
            None => "Load or select a track.".to_string(),
        }).size(10.5).color(DIM));
    });
}

// ------------------------------------------------------------------------------ tree extras

/// What a playlist row carries beyond its name and count (reference 10): the CUE Analysis
/// Playlist's [Start] button (39.5 x 12 pt, ending 72.5 pt from the row's right edge) and, on
/// the selected playlist or folder, the 12 pt disc that creates a playlist, 10.5 pt from the
/// right edge.
pub fn node_row_extras(app: &mut App, ui: &mut Ui, n: &PlaylistNode, rr: Rect, sel: bool, is_cue: bool) {
    let tips = app.prefs.tooltips;
    if is_cue {
        let br = Rect::from_min_max(pos2(rr.max.x - 112.0, rr.center().y - 6.0), pos2(rr.max.x - 72.5, rr.center().y + 6.0));
        if br.min.x > rr.min.x + 60.0 {
            let resp = ui.interact(br, ui.id().with(("cue_start", n.id)), Sense::click());
            let busy = app.ext.cue_wait;
            // (A selected row keeps rekordbox's fixed blue and white text in Light: `ListViewer::findTextColour`.)
            let _sel_dark = crate::theme::dark_scope(sel);
            let col = if busy { BLUE } else if resp.hovered() { WHITE } else { DIM };
            ui.painter().rect_filled(br, 0.0, if sel { ROW_SEL } else { BG2 });
            ui.painter().rect_stroke(br, 0.0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
            ui.painter().text(br.center(), Align2::CENTER_CENTER, "Start", rb(9.5), col.c());
            let resp = if tips {
                resp.on_hover_text("Analyzes CUE points of tracks registered to this playlist and generates personal CUE Tendency Information.\nThe more track CUE point tendencies are collected, the more you will be capable of achieving CUE points similar to your CUE points.\nTo add/remove tracks in a playlist, click the button again and update the personal CUE Tendency Information.\nYour personal CUE Tendency information can be reset by pressing the button when there are no tracks registered to the playlist.")
            } else {
                resp
            };
            if resp.clicked() && !busy {
                app.cue_analysis_start();
                ui.ctx().request_repaint();
            }
        }
        if app.ext.cue_wait {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        }
    }
    if sel && !is_cue && n.kind != NodeKind::Smart {
        let c = pos2(rr.max.x - 10.5, rr.center().y);
        let resp = ui.interact(Rect::from_center_size(c, vec2(14.0, 14.0)), ui.id().with(("node_plus", n.id)), Sense::click());
        crate::icons_src::plus_disc(ui.painter(), c, if resp.hovered() { WHITE.c() } else { gy(0xA0A0A0) }, ROW_SEL.c());
        let parent = if n.kind == NodeKind::Folder { n.id } else { n.parent };
        dropdown(ui, &resp, "node_plus_menu", |ui| {
            for (label, kind) in [("Create New Playlist", NodeKind::Playlist), ("Create New Intelligent Playlist", NodeKind::Smart), ("Create New Folder", NodeKind::Folder)] {
                if ui.button(label).clicked() {
                    if n.kind == NodeKind::Folder {
                        app.expanded.insert(n.id);
                    }
                    app.create_node(parent, kind);
                }
            }
        });
    }
}

/// The folders a playlist is in (the pull-down beside the list's title): each opens that level.
pub fn path_menu(app: &mut App, ui: &mut Ui) {
    let View::Playlist(id) = app.view else { return };
    let path = column_path(&app.nodes, Some(id));
    if ui.selectable_label(false, "Playlists").clicked() {
        app.side = SideMode::Playlists;
        app.set_view(View::Collection);
    }
    for (depth, nid) in path.iter().enumerate() {
        let Some(n) = app.node(*nid).cloned() else { continue };
        let label = format!("{}{}", "    ".repeat(depth + 1), n.name);
        if ui.selectable_label(*nid == id, label).clicked() && *nid != id {
            app.side = SideMode::Playlists;
            app.expanded.insert(n.id);
            app.set_view(View::Playlist(n.id));
        }
    }
}

// ------------------------------------------------------------------------------ column view

const CV_COL_W: f32 = 150.0;

/// Playlists as columns: the first column lists the top level, a chosen folder opens its
/// contents in the next column, a chosen playlist (or folder) fills the track list.
pub fn column_view(app: &mut App, ui: &mut Ui, drop: &mut Vec<(Rect, i64)>) {
    if app.ext.tree_search_open {
        crate::ui_library_ext::tree_search_field(app, ui);
    }
    let query = if app.ext.tree_search_open { crate::smart::fold(app.ext.tree_search.trim()) } else { String::new() };
    let shown = match app.view {
        View::Playlist(id) => Some(id),
        _ => None,
    };
    let cols = columns_for(&app.nodes, shown);
    let area = ui.available_rect_before_wrap();
    let area = Rect::from_min_max(area.min, pos2(area.max.x + 12.0, area.max.y));
    let mut aui = ui.new_child(egui::UiBuilder::new().max_rect(area).id_salt("column_view"));
    aui.set_clip_rect(area.intersect(ui.clip_rect()));
    bar_style(&mut aui);
    aui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let n_cols = cols.len();
    // A newly shown node brings the last column into view.
    let last_key = egui::Id::new("cv_last_shown");
    let last: Option<i64> = ui.ctx().data(|d| d.get_temp(last_key)).unwrap_or(None);
    if last != shown {
        ui.ctx().data_mut(|d| d.insert_temp(last_key, shown));
        app.ext.cv_scroll_end = true;
    }
    let scroll_end = std::mem::take(&mut app.ext.cv_scroll_end);
    let mut sa = egui::ScrollArea::horizontal().id_salt("cv_h").auto_shrink([false, false]);
    if scroll_end {
        sa = sa.horizontal_scroll_offset((n_cols as f32 * CV_COL_W - area.width()).max(0.0));
    }
    sa.show(&mut aui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (full, _) = ui.allocate_exact_size(vec2((n_cols as f32 * CV_COL_W).max(ui.available_width()), ui.available_height()), Sense::hover());
        for (ci, (parent, chosen)) in cols.iter().enumerate() {
            let cr = Rect::from_min_size(pos2(full.min.x + ci as f32 * CV_COL_W, full.min.y), vec2(CV_COL_W, full.height()));
            if ci > 0 {
                ui.painter().rect_filled(Rect::from_min_max(cr.min, pos2(cr.min.x + 1.0, cr.max.y)), 0.0, BG);
            }
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(cr.min.x + 1.0, cr.min.y), cr.max)).id_salt(("cv_col", ci)));
            cui.set_clip_rect(cr.intersect(ui.clip_rect()));
            cui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            egui::ScrollArea::vertical().id_salt(("cv_rows", ci)).auto_shrink([false, false]).show(&mut cui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                if ci == 0 {
                    let all = cv_row(ui, "All Tracks", TreeIcon::Note, Some(app.tracks.len()), false, app.view == View::Collection);
                    if all.clicked() {
                        app.set_view(View::Collection);
                    }
                }
                let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == *parent).cloned().collect();
                for n in kids {
                    if !query.is_empty() && !crate::ui_library_ext::node_matches(app, n.id, &query, 0) {
                        continue;
                    }
                    if matches!(&app.dialogs.rename, Some((rid, _)) if *rid == n.id) {
                        let (rr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                        let mut name = app.dialogs.rename.clone().map(|x| x.1).unwrap_or_default();
                        let done = row_edit(ui, rr.shrink2(vec2(4.0, 0.0)), &mut name, "");
                        app.dialogs.rename = Some((n.id, name.clone()));
                        if done {
                            if !name.trim().is_empty() {
                                let _ = app.db.rename_node(n.id, name.trim());
                            }
                            app.dialogs.rename = None;
                            app.reload_playlists();
                        }
                        continue;
                    }
                    let is_cue = n.kind == NodeKind::Playlist && app.cue_playlist() == Some(n.id);
                    let (icon, count) = match n.kind {
                        NodeKind::Folder => (TreeIcon::FolderClosed, None),
                        NodeKind::Smart => (TreeIcon::Smart, n.smart.as_ref().map(|r| app.smart_tracks(r).len())),
                        NodeKind::Playlist => (if is_cue { TreeIcon::CuePlaylist } else { TreeIcon::Playlist }, Some(app.node_counts.get(&n.id).copied().unwrap_or(0))),
                    };
                    let on = *chosen == Some(n.id);
                    let resp = cv_row(ui, &n.name, icon, count.filter(|_| app.prefs.playlist_track_counts), n.kind == NodeKind::Folder, on);
                    if n.kind == NodeKind::Playlist {
                        drop.push((resp.rect, n.id));
                    }
                    if resp.clicked() {
                        app.set_view(View::Playlist(n.id));
                        app.ext.cv_scroll_end = n.kind == NodeKind::Folder;
                        ui.ctx().request_repaint();
                    }
                    if resp.double_clicked() {
                        if n.kind == NodeKind::Smart {
                            app.dialogs.smart_edit = Some((Some(n.id), n.parent, n.name.clone(), n.smart.clone().unwrap_or_default()));
                        } else {
                            app.dialogs.rename = Some((n.id, n.name.clone()));
                        }
                    }
                    let nn = n.clone();
                    resp.context_menu(|ui| crate::ui_library_ext::node_menu(app, ui, &nn));
                }
                // The rest of the column: a menu for creating at this level.
                let rest = ui.available_rect_before_wrap();
                if rest.height() > 4.0 {
                    let bresp = ui.interact(rest, ui.id().with("cv_blank"), Sense::click());
                    let into = *parent;
                    bresp.context_menu(|ui| {
                        for (label, kind) in [("Create New Playlist", NodeKind::Playlist), ("Create New Intelligent Playlist", NodeKind::Smart), ("Create New Folder", NodeKind::Folder)] {
                            if ui.button(label).clicked() {
                                app.create_node(into, kind);
                                ui.close_menu();
                            }
                        }
                    });
                }
            });
        }
    });
}

/// A Column View row: 15 pt; glyph centred 11 pt in, name from 22 pt, the count (or a folder's
/// arrow) at the right.
fn cv_row(ui: &mut Ui, text: &str, icon: TreeIcon, count: Option<usize>, folder: bool, selected: bool) -> egui::Response {
    use crate::icons_src as src;
    let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 15.0), Sense::click());
    let p = ui.painter().with_clip_rect(rr.intersect(ui.clip_rect()));
    let bg = if selected { ROW_SEL.c() } else if resp.hovered() { gy(0x262626) } else { BG2.c() };
    // (A selected row keeps rekordbox's fixed blue and white text in Light: `ListViewer::findTextColour`.)
    let _sel_dark = crate::theme::dark_scope(selected);
    if selected || resp.hovered() {
        p.rect_filled(rr, 0.0, bg);
    }
    let col = if selected { WHITE } else { TREE_ICON };
    let ic = pos2(rr.min.x + 11.0, rr.center().y);
    match icon {
        TreeIcon::Note => src::tree_note(&p, ic, col.c()),
        TreeIcon::FolderClosed | TreeIcon::FolderOpen => src::tree_folder(&p, ic, col.c()),
        TreeIcon::Smart => src::tree_smart(&p, ic + vec2(2.5, 2.5), col.c(), bg),
        TreeIcon::CuePlaylist => src::tree_cue_list(&p, ic, col.c()),
        _ => src::tree_list(&p, ic, col.c(), bg),
    }
    let right = if folder { rr.max.x - 14.0 } else if count.is_some() { rr.max.x - 34.0 } else { rr.max.x - 4.0 };
    p.with_clip_rect(Rect::from_min_max(rr.min, pos2(right, rr.max.y)).intersect(ui.clip_rect())).text(pos2(rr.min.x + 22.0, rr.center().y), Align2::LEFT_CENTER, text, rb(TREE_FONT), WHITE.c());
    if folder {
        src::tree_arrow(&p, pos2(rr.max.x - 8.0, rr.center().y), false, if selected { WHITE.c() } else { DIM.c() });
    } else if let Some(c) = count {
        p.text(pos2(rr.max.x - 6.0, rr.center().y), Align2::RIGHT_CENTER, c.to_string(), rb(TREE_FONT), WHITE.c());
    }
    resp
}

// ------------------------------------------------------------------------------ sub-browser

/// The sub-browser's own browser state. While it is drawn these are swapped with the app's
/// (and the preferences') fields, so the tree and the track list work in it as in the browser.
pub struct SubState {
    pub view: View,
    pub side: SideMode,
    pub list: Vec<i64>,
    pub list_dirty: bool,
    pub selected: HashSet<i64>,
    pub anchor: Option<usize>,
    pub anchor_id: Option<i64>,
    pub search: String,
    pub filter_open: bool,
    pub related_built_for: Option<i64>,
    pub list_num: Vec<usize>,
    pub columns: Vec<Col>,
    pub columns_other: Vec<Col>,
    pub sort_col: Col,
    pub sort_asc: bool,
    pub sort_other: (Col, bool),
    pub columns_for_playlist: bool,
    pub list_scroll_to: Option<f32>,
    pub scroll_to_selected: bool,
    pub tree_focus: bool,
    pub tree_search_open: bool,
    pub tree_search: String,
    pub column_view: bool,
    pub column_browser: bool,
    pub colbrowse: [Option<String>; 3],
    pub colbrowse_values: [Vec<(String, usize)>; 3],
    pub filter_tags: HashSet<i64>,
    pub palette_open: bool,
}

impl SubState {
    fn new() -> SubState {
        let cols = vec![Col::Status, Col::Artwork, Col::Title, Col::Artist, Col::Album, Col::Genre, Col::Bpm, Col::Rating, Col::Time, Col::Key, Col::DateAdded];
        SubState {
            view: View::Collection,
            side: SideMode::Playlists,
            list: vec![],
            list_dirty: true,
            selected: HashSet::new(),
            anchor: None,
            anchor_id: None,
            search: String::new(),
            filter_open: false,
            related_built_for: None,
            list_num: vec![],
            columns: cols.clone(),
            columns_other: cols,
            sort_col: Col::DateAdded,
            sort_asc: false,
            sort_other: (Col::Num, true),
            columns_for_playlist: false,
            list_scroll_to: None,
            scroll_to_selected: false,
            tree_focus: false,
            tree_search_open: false,
            tree_search: String::new(),
            column_view: false,
            column_browser: false,
            colbrowse: Default::default(),
            colbrowse_values: Default::default(),
            filter_tags: HashSet::new(),
            palette_open: false,
        }
    }
}

fn swap_state(app: &mut App, s: &mut SubState) {
    use std::mem::swap;
    swap(&mut app.view, &mut s.view);
    swap(&mut app.side, &mut s.side);
    swap(&mut app.list, &mut s.list);
    swap(&mut app.list_dirty, &mut s.list_dirty);
    swap(&mut app.selected, &mut s.selected);
    swap(&mut app.anchor, &mut s.anchor);
    swap(&mut app.anchor_id, &mut s.anchor_id);
    swap(&mut app.search, &mut s.search);
    swap(&mut app.filter_open, &mut s.filter_open);
    swap(&mut app.related_built_for, &mut s.related_built_for);
    swap(&mut app.ext.list_num, &mut s.list_num);
    swap(&mut app.prefs.columns, &mut s.columns);
    swap(&mut app.prefs.columns_other, &mut s.columns_other);
    swap(&mut app.prefs.sort_col, &mut s.sort_col);
    swap(&mut app.prefs.sort_asc, &mut s.sort_asc);
    swap(&mut app.prefs.sort_other, &mut s.sort_other);
    swap(&mut app.prefs.columns_for_playlist, &mut s.columns_for_playlist);
    swap(&mut app.list_scroll_to, &mut s.list_scroll_to);
    swap(&mut app.scroll_to_selected, &mut s.scroll_to_selected);
    swap(&mut app.ext.tree_focus, &mut s.tree_focus);
    swap(&mut app.ext.tree_search_open, &mut s.tree_search_open);
    swap(&mut app.ext.tree_search, &mut s.tree_search);
    swap(&mut app.prefs.bx.column_view, &mut s.column_view);
    swap(&mut app.prefs.column_browser, &mut s.column_browser);
    swap(&mut app.colbrowse, &mut s.colbrowse);
    swap(&mut app.colbrowse_values, &mut s.colbrowse_values);
    swap(&mut app.filter_tags, &mut s.filter_tags);
    swap(&mut app.prefs.palette_open, &mut s.palette_open);
}

/// Sub-browser (references 14 and 15): a second browser with its own source strip (27 pt:
/// Collection, Playlists, iTunes, Devices, Histories, Recordings), tree pane (to 267.5 pt, or
/// 259 pt under the browser), and track list from 5 pt further. Tracks drag from it onto
/// playlists of either tree and into the main list's playlist; double-click loads.
pub fn sub_browser(app: &mut App, ui: &mut Ui, r: Rect, drop: &mut Vec<(Rect, i64)>) {
    use crate::icons_src as src;
    let below = app.prefs.bx.sub_horizontal;
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    p.rect_filled(r, 0.0, BG);
    let tree_right = r.min.x + if below { 259.0 } else { 267.5 };
    let pane = Rect::from_min_max(r.min, pos2(tree_right.min(r.max.x), r.max.y));
    p.rect_filled(pane, 0.0, BG2);
    p.rect_filled(Rect::from_min_max(pos2(tree_right, r.min.y), pos2(tree_right + 5.0, r.max.y)), 0.0, BG1);
    if ui.input(|i| i.pointer.primary_pressed()) {
        if let Some(pp) = ui.input(|i| i.pointer.interact_pos()) {
            if r.contains(pp) {
                app.ext.sub_focus = true;
            } else if ui.max_rect().contains(pp) {
                app.ext.sub_focus = false;
            }
        }
    }
    let mut st = app.ext.sub.take().unwrap_or_else(|| {
        let mut s = Box::new(SubState::new());
        s.view = View::Collection;
        s
    });
    let first = st.list_dirty && st.list.is_empty() && st.view == View::Collection && app.sub_view != View::Collection;
    let had_drag = app.drag.is_some();
    swap_state(app, &mut st);
    app.ext.in_sub = true;
    if first {
        // The list chosen before this version (or by a screenshot switch).
        let v = app.sub_view.clone();
        if let View::Playlist(id) = v {
            let mut up = app.node(id).map(|n| n.parent).unwrap_or(0);
            while up != 0 {
                app.expanded.insert(up);
                up = app.node(up).map(|n| n.parent).unwrap_or(0);
            }
        }
        app.set_view(v);
    }
    // Source strip: 18 pt tiles centred 14 pt in on a 20 pt pitch from 12 pt down.
    let tiles: [(src::SrcIcon, Option<SideMode>, &str); 6] = [
        (src::collection, Some(SideMode::Collection), "Collection"),
        (src::playlist, Some(SideMode::Playlists), "Playlists"),
        (src::itunes, Some(SideMode::Itunes), "iTunes"),
        (src::device, Some(SideMode::Devices), "Devices"),
        (src::history, Some(SideMode::History), "Histories"),
        (src::recording, Some(SideMode::Recordings), "Recordings"),
    ];
    for (i, (draw, mode, name)) in tiles.iter().enumerate() {
        let rr = Rect::from_center_size(pos2(r.min.x + 14.5, r.min.y + 12.0 + i as f32 * 20.0), vec2(18.0, 18.0));
        let on = *mode == Some(app.side);
        let resp = ui.interact(rr, ui.id().with(("sub_side", i)), Sense::click());
        // (A selected row keeps rekordbox's fixed blue and white text in Light: `ListViewer::findTextColour`.)
        let _sel_dark = crate::theme::dark_scope(on);
        let bg = if on {
            p.rect_filled(Rect::from_min_max(pos2(r.min.x, rr.min.y), pos2(r.min.x + 2.0, rr.max.y)), 0.0, ROW_SEL);
            p.rect_filled(rr, 1.5, ROW_SEL);
            p.rect_stroke(rr, 1.5, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
            ROW_SEL.c()
        } else {
            src::tile(&p, rr, false, resp.hovered(), mode.is_some(), BG2.c())
        };
        draw(&p, rr, if mode.is_none() { gy(0x4A4A4A) } else { WHITE.c() }, bg);
        let resp = if app.prefs.tooltips { resp.on_hover_text(*name) } else { resp };
        if resp.clicked() {
            if let Some(m) = *mode {
                app.side = m;
                match m {
                    SideMode::Collection => app.set_view(View::Collection),
                    SideMode::Devices => {
                        app.scan_devices();
                        app.set_view(View::Device { root: String::new(), node: None });
                    }
                    SideMode::History => app.set_view(View::History(0)),
                    SideMode::Itunes => app.set_view(View::Source { kind: crate::library_ext::SRC_ITUNES, node: String::new() }),
                    SideMode::Recordings => app.set_view(View::Explorer(crate::ui_rec::rec_dir(&app.prefs.rec).to_string_lossy().to_string())),
                    _ => {}
                }
            }
        }
    }
    // Tree pane.
    let inner = if app.side == SideMode::Collection {
        Rect::from_min_max(pos2(r.min.x + 35.0, r.min.y), pos2(pane.max.x - 2.0, r.max.y - 4.0))
    } else {
        Rect::from_min_max(pos2(r.min.x + 35.0, r.min.y), pos2(pane.max.x - 12.0, r.max.y - 4.0))
    };
    let mut sub_drop: Vec<(Rect, i64)> = vec![];
    {
        let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(inner).id_salt("sub_tree"));
        tui.set_clip_rect(Rect::from_min_max(pos2(r.min.x + 28.0, r.min.y), pane.max).intersect(ui.clip_rect()));
        tui.spacing_mut().item_spacing.y = 0.0;
        crate::ui_browser::tree_pane(app, &mut tui, &mut sub_drop);
    }
    // Track list.
    let list = Rect::from_min_max(pos2(tree_right + 5.0, r.min.y), r.max);
    if list.width() > 60.0 {
        let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(list).id_salt("sub_list"));
        lui.set_clip_rect(list.intersect(ui.clip_rect()));
        crate::ui_browser::list_for_view(app, &mut lui);
    }
    app.ext.in_sub = false;
    swap_state(app, &mut st);
    app.sub_view = st.view.clone();
    app.sub_sel = st.selected.clone();
    app.ext.sub = Some(st);
    drop.extend(sub_drop);
    if !had_drag && app.drag.is_some() {
        app.sub_drag = true;
    }
    if app.ext.sub_save_wanted.replace(false) {
        app.save_prefs();
    }
}
