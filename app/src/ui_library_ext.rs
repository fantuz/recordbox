//! Browser screens added for library management: attribute icons, in-place editing, the
//! Track Filter, the docked Information window, Missing File Manager, Histories, Tag List,
//! Matching, Related Tracks lists, the Playlist Palette and the iTunes / rekordbox xml sources.

use crate::ui_kit::{self as kit, btn};
use crate::app::*;
use crate::library_ext::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_browser::{list_font, tree_row, TreeIcon, ROW_H};
use crate::ui_player::star;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui};
use std::collections::HashSet;

// ------------------------------------------------------------------------------------ icons
pub fn icon_itunes(p: &egui::Painter, r: Rect, c: Color32) {
    // A beamed pair of notes.
    let m = r.center();
    p.circle_filled(m + vec2(-3.2, 3.4), 2.1, c);
    p.circle_filled(m + vec2(3.0, 2.4), 2.1, c);
    let s = Stroke::new(1.3, c);
    p.line_segment([m + vec2(-1.4, 3.4), m + vec2(-1.4, -3.6)], s);
    p.line_segment([m + vec2(4.8, 2.4), m + vec2(4.8, -4.6)], s);
    p.line_segment([m + vec2(-1.4, -3.2), m + vec2(4.8, -4.2)], Stroke::new(2.2, c));
}

pub fn icon_xml(p: &egui::Painter, r: Rect, c: Color32) {
    // A sheet with a folded corner and angle brackets.
    let b = Rect::from_center_size(r.center(), vec2(10.0, 12.0));
    let s = Stroke::new(1.2, c);
    let pts = vec![b.left_top(), pos2(b.right() - 3.0, b.top()), pos2(b.right(), b.top() + 3.0), b.right_bottom(), b.left_bottom()];
    p.add(egui::Shape::closed_line(pts, s));
    let m = b.center() + vec2(0.0, 1.5);
    p.add(egui::Shape::line(vec![m + vec2(-1.2, -2.2), m + vec2(-3.2, 0.0), m + vec2(-1.2, 2.2)], Stroke::new(1.0, c)));
    p.add(egui::Shape::line(vec![m + vec2(1.2, -2.2), m + vec2(3.2, 0.0), m + vec2(1.2, 2.2)], Stroke::new(1.0, c)));
}

/// Matching: two overlapping rings (the original's `matching` glyph).
pub fn icon_matching(p: &egui::Painter, r: Rect, c: Color32) {
    let m = r.center();
    p.circle_stroke(m + vec2(-2.4, 1.6), 3.9, Stroke::new(1.5, c));
    p.circle_stroke(m + vec2(2.4, -1.6), 3.9, Stroke::new(1.5, c));
}

/// Playlist Palette: three bullet rows (the original's `playListPalette` glyph).
pub fn icon_palette(p: &egui::Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center(), vec2(10.0, 10.0));
    for k in 0..3 {
        let y = b.top() + k as f32 * 3.9;
        p.rect_filled(Rect::from_min_size(pos2(b.left(), y), vec2(2.0, 2.2)), 0.0, c);
        p.rect_filled(Rect::from_min_size(pos2(b.left() + 3.0, y), vec2(7.0, 2.2)), 0.0, c);
    }
}

fn icon_tag_list(p: &egui::Painter, c: egui::Pos2, col: Color32) {
    // A list sheet with a tick over its corner (the original's tag list glyph, 13 pt).
    let b = Rect::from_min_size(c + vec2(-6.0, -6.0), vec2(9.0, 10.0));
    p.rect_filled(b, 0.0, col);
    for k in 0..3 {
        let y = b.top() + 2.5 + k as f32 * 2.2;
        p.line_segment([pos2(b.left() + 1.8, y), pos2(b.right() - 1.8, y)], Stroke::new(1.0, gy(0x000000)));
    }
    p.add(egui::Shape::line(vec![c + vec2(-1.5, 2.0), c + vec2(1.2, 5.0), c + vec2(6.5, -1.0)], Stroke::new(1.8, col)));
}

fn icon_missing(p: &egui::Painter, c: egui::Pos2) {
    // Warning triangle with an exclamation mark.
    let pts = vec![c + vec2(0.0, -5.5), c + vec2(6.0, 5.0), c + vec2(-6.0, 5.0)];
    p.add(egui::Shape::convex_polygon(pts, CAUTION_TX, Stroke::NONE));
    p.line_segment([c + vec2(0.0, -2.2), c + vec2(0.0, 1.6)], Stroke::new(1.5, WHITE));
    p.circle_filled(c + vec2(0.0, 3.4), 0.9, WHITE);
}

fn icon_unsupported(p: &egui::Painter, c: egui::Pos2) {
    p.circle_stroke(c, 5.0, Stroke::new(1.5, CAUTION_TX));
    p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], Stroke::new(1.5, CAUTION_TX));
}

fn ring(p: &egui::Painter, c: egui::Pos2, rad: f32, from: f32, sweep: f32, s: Stroke) {
    let n = 24;
    let pts: Vec<egui::Pos2> = (0..=n).map(|i| {
        let a = from + sweep * i as f32 / n as f32;
        c + vec2(a.cos(), a.sin()) * rad
    }).collect();
    p.add(egui::Shape::line(pts, s));
}

/// The "CUE" mark of tracks with hot cues or memory cues.
fn icon_cue(p: &egui::Painter, left_center: egui::Pos2, col: Color32) {
    let r = Rect::from_min_size(left_center + vec2(0.0, -4.5), vec2(16.5, 9.0));
    p.rect_stroke(r, 1.0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
    p.text(r.center() + vec2(0.0, 0.2), Align2::CENTER_CENTER, "CUE", bold(6.5), col);
}

/// Attribute column (60 pt, two 30 pt halves as in the original): left half now playing,
/// missing file and Tag List marks plus the colour; right half the analysis state and CUE.
pub fn status_cell(app: &mut App, ui: &mut Ui, t: &Track, r: Rect) {
    let p = ui.painter().with_clip_rect(r);
    let a = Rect::from_min_size(r.min, vec2(30.0, r.height()));
    // Three 30 pt cells: state, (rekordbox's cloud status, not used here), attributes.
    let b = Rect::from_min_max(pos2((r.max.x - 30.0).max(r.min.x + 30.0), r.min.y), r.max);
    p.line_segment([pos2(a.max.x - 0.5, r.min.y), pos2(a.max.x - 0.5, r.max.y)], Stroke::new(1.0, BG));
    p.line_segment([pos2(b.min.x - 0.5, r.min.y), pos2(b.min.x - 0.5, r.max.y)], Stroke::new(1.0, BG));
    let cy = r.center().y;
    let mut tips: Vec<&str> = vec![];
    let loaded = app.deck.track == Some(t.id);
    let in_tag = app.ext.tag_set.contains(&t.id);
    // Left half.
    if t.missing {
        icon_missing(&p, pos2(a.min.x + 9.0, cy));
        tips.push("File is missing");
    } else if loaded {
        // The original's `playing` glyph: a 7×8 pt triangle.
        let c = pos2(a.min.x + 9.0, cy);
        let col = if app.playing() { PLAY_GREEN } else { WHITE.c() };
        p.add(egui::Shape::convex_polygon(vec![c + vec2(-3.5, -4.0), c + vec2(3.5, 0.0), c + vec2(-3.5, 4.0)], col, Stroke::NONE));
        tips.push("Now playing");
    }
    if in_tag {
        icon_tag_list(&p, pos2(a.min.x + 21.0, cy), gy(0xA0A0A0));
        tips.push("Track exists in the Tag List");
    } else if t.color > 0 {
        p.rect_filled(Rect::from_center_size(pos2(a.min.x + 21.0, cy), vec2(8.0, 8.0)), 1.0, label_color(t.color));
    }
    // Right half.
    let cued = app.cue_counts.get(&t.id).map(|(h, m)| h + m > 0).unwrap_or(false);
    let ic = pos2(b.min.x + 8.0, cy);
    match t.analyzed {
        3 => {
            // Analysing: a ring that fills with the progress of the analysis run.
            let total = app.analysis_total.max(1) as f32;
            let frac = (app.analysis_done as f32 / total).clamp(0.0, 1.0);
            let tt = app.ctx.input(|i| i.time) as f32;
            ring(&p, ic, 4.6, 0.0, std::f32::consts::TAU, Stroke::new(1.4, gy(0x4A4A4A)));
            ring(&p, ic, 4.6, tt * 4.0, std::f32::consts::TAU * (0.15 + 0.85 * frac), Stroke::new(1.6, BLUE));
            p.text(pos2(b.min.x + 15.0, cy), Align2::LEFT_CENTER, format!("{}%", (frac * 100.0) as i32), FontId::proportional(8.0), DIM2.c());
            app.ctx.request_repaint_after(std::time::Duration::from_millis(100));
            tips.push("Now Analyzing");
        }
        2 => {
            icon_unsupported(&p, ic);
            tips.push("Track analysis failed");
        }
        0 => {
            // Not analysed: an empty ring.
            p.circle_stroke(ic, 4.6, Stroke::new(1.3, gy(0x787878)));
            p.circle_filled(ic, 1.2, gy(0x787878));
            tips.push("Track is not analyzed");
        }
        _ => {}
    }
    // Reference 01/04: the skin's 21 x 9 pt "CUE" mark (#969696) starts 12 pt into the half and
    // runs under the column line; the analysed star (12 x 11 pt, white) sits over its first
    // letter, or in the middle of the half when there is no cue.
    if cued && t.analyzed != 3 {
        p.with_clip_rect(Rect::from_min_max(b.min, pos2(b.max.x - 1.0, b.max.y))).text(pos2(b.min.x + 12.2, cy + 0.6), Align2::LEFT_CENTER, "CUE", rb(10.2), if t.analyzed == 1 { DIM.c() } else { OFF.c() });
        tips.push("Track has Hot Cue/Memory Cue");
    }
    if t.analyzed == 1 {
        let sc = pos2(b.min.x + if cued { 7.5 } else { 14.5 }, cy - 0.5);
        crate::icons_src::star(&p, sc + vec2(0.0, 0.0), true, WHITE.c());
    }
    // The middle cell: EDIT and Track Type attributes (ui_edit.rs).
    crate::ui_edit::attribute_icons(app, &p, Rect::from_min_max(pos2(a.max.x, r.min.y), pos2(b.min.x, r.max.y)), t.id, &mut tips);
    if !tips.is_empty() && app.prefs.tooltips && ui.rect_contains_pointer(r) {
        let resp = ui.interact(r, ui.id().with(("attr_tip", t.id, r.min.y as i32)), Sense::hover());
        resp.on_hover_text(tips.join("\n"));
    }
}

// ------------------------------------------------------------------------------------ cells
/// The text a column shows for a track (None: the column is drawn, not written).
pub fn col_text(app: &App, t: &Track, c: Col) -> Option<String> {
    let num = |n: i32| if n > 0 { n.to_string() } else { String::new() };
    Some(match c {
        Col::Title => t.display_title(),
        Col::Artist => t.artist.clone(),
        Col::Album => t.album.clone(),
        Col::Genre => t.genre.clone(),
        Col::Bpm => if t.bpm > 0.0 { format!("{:.2}", t.bpm) } else { String::new() },
        Col::Time => if t.duration > 0.0 { fmt_time(t.duration, false) } else { String::new() },
        Col::Key => t.key_label(app.prefs.key_alnum),
        Col::DateAdded => t.date_added.get(..10).unwrap_or(&t.date_added).to_string(),
        Col::DateCreated => t.date_created.get(..10).unwrap_or(&t.date_created).to_string(),
        Col::Comments => t.comment.clone(),
        Col::Label => t.label.clone(),
        Col::Remixer => t.remixer.clone(),
        Col::OriginalArtist => t.original_artist.clone(),
        Col::Composer => t.composer.clone(),
        Col::Year => num(t.year),
        Col::Bitrate => if t.bitrate > 0 { format!("{} Kbps", t.bitrate) } else { String::new() },
        Col::SampleRate => if t.sample_rate > 0 { format!("{:.1} kHz", t.sample_rate as f64 / 1000.0) } else { String::new() },
        Col::FileType => t.file_type.to_uppercase(),
        Col::Size => format!("{:.1} MB", t.file_size as f64 / 1e6),
        Col::Location => t.path.clone(),
        Col::FileName => t.file_name(),
        Col::PlayCount => t.play_count.to_string(),
        Col::MixName => t.mix_name.clone(),
        Col::TrackNumber => num(t.track_no),
        Col::MyTag => app.tag_text(t.id),
        Col::ReleaseDate => t.release_date.clone(),
        Col::AlbumArtist => t.album_artist.clone(),
        Col::DiscNumber => num(t.disc_no),
        Col::Bitdepth => if t.bit_depth > 0 { format!("{} bit", t.bit_depth) } else { String::new() },
        Col::Grouping => t.grouping.clone(),
        Col::Lyricist => t.lyricist.clone(),
        Col::Message => t.message.clone(),
        Col::HotCue => {
            let n = app.cue_counts.get(&t.id).map(|c| c.0).unwrap_or(0);
            if n > 0 { n.to_string() } else { String::new() }
        }
        Col::Num | Col::Status | Col::Preview | Col::Artwork | Col::Rating | Col::Color => return None,
    })
}

/// "Auto-size this column": wide enough for its header and its longest text in the list.
pub fn autosize_column(app: &mut App, ui: &Ui, c: Col) {
    let font = list_font(app);
    let width_of = |s: &str, f: FontId| ui.fonts(|fonts| fonts.layout_no_wrap(s.to_string(), f, WHITE.c()).size().x);
    let mut w = width_of(c.label(), FontId::proportional(11.0)) + 26.0;
    let mut text = false;
    for id in app.list.iter().take(5000) {
        let Some(t) = app.track(*id) else { continue };
        let Some(s) = col_text(app, t, c) else { break };
        text = true;
        if !s.is_empty() {
            w = w.max(width_of(&s, font.clone()) + 14.0);
        }
    }
    if text {
        app.prefs.widths.insert(c.key(), w.clamp(30.0, 700.0).ceil());
    } else {
        app.prefs.widths.remove(&c.key());
    }
}

/// Header right-click menu: auto-size, then the checklist of every column.
pub fn header_menu(app: &mut App, ui: &mut Ui, c: Col) {
    if ui.button("Auto-size this column").clicked() {
        autosize_column(app, ui, c);
        app.save_prefs();
        ui.close_menu();
    }
    if ui.button("Auto-size all columns").clicked() {
        for col in app.prefs.columns.clone() {
            autosize_column(app, ui, col);
        }
        app.save_prefs();
        ui.close_menu();
    }
    ui.separator();
    column_checklist(app, ui);
}

pub fn column_checklist(app: &mut App, ui: &mut Ui) {
    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
        let playlist_like = app.prefs.columns_for_playlist;
        for c in ALL_COLS {
            if c == Col::Status || (c == Col::Num && !playlist_like) {
                continue;
            }
            let mut on = app.prefs.columns.contains(&c);
            let label = if c == Col::Num { "Track Order" } else { c.label() };
            if ui.checkbox(&mut on, label).changed() {
                if on {
                    if c == Col::Num {
                        app.prefs.columns.insert(0, c);
                    } else {
                        app.prefs.columns.push(c);
                    }
                } else {
                    app.prefs.columns.retain(|x| *x != c);
                }
                app.save_prefs();
            }
        }
    });
}

/// The in-place editor of the cell being edited (drawn over the cell).
pub fn inline_editor(app: &mut App, ui: &mut Ui, r: Rect) {
    let Some(mut ed) = app.ext.edit.take() else { return };
    let font = list_font(app);
    ui.painter().rect_filled(r, 0.0, BG);
    let te = ui.put(r.shrink2(vec2(0.5, 0.0)), egui::TextEdit::singleline(&mut ed.text).font(font).margin(egui::Margin::symmetric(4, 0)).vertical_align(egui::Align::Center));
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, BLUE), egui::StrokeKind::Inside);
    app.ext.edit_drawn = true;
    if !ed.focused {
        te.request_focus();
        ed.focused = true;
        ui.ctx().request_repaint();
        app.ext.edit = Some(ed);
        return;
    }
    if te.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return;
        }
        app.ext.edit = Some(ed);
        app.commit_cell_edit();
        return;
    }
    app.ext.edit = Some(ed);
}

/// The 8-colour chooser opened from a Color cell.
pub fn color_picker(app: &mut App, ctx: &egui::Context) {
    let Some((id, pos, opened)) = app.ext.color_pick else { return };
    let now = ctx.input(|i| i.time);
    let mut chosen: Option<u8> = None;
    let area = egui::Area::new(egui::Id::new("color_pick")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(BG2.c()).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
            let cur = app.track(id).map(|t| t.color).unwrap_or(0);
            for i in (1..=8u8).chain([0u8]) {
                let (rr, resp) = ui.allocate_exact_size(vec2(118.0, 17.0), Sense::click());
                if resp.hovered() || cur == i {
                    ui.painter().rect_filled(rr, 2.0, crate::ui_feel::hl(&resp, BTN, ROW_SEL));
                }
                let chip = Rect::from_center_size(pos2(rr.min.x + 12.0, rr.center().y), vec2(14.0, 9.0));
                if i == 0 {
                    ui.painter().rect_stroke(chip, 1.0, Stroke::new(1.0, DIM), egui::StrokeKind::Inside);
                } else {
                    ui.painter().rect_filled(chip, 1.0, label_color(i));
                }
                ui.painter().text(pos2(rr.min.x + 26.0, rr.center().y), Align2::LEFT_CENTER, COLOR_NAMES[i as usize], FontId::proportional(11.5), TEXT.c());
                if resp.clicked() {
                    chosen = Some(i);
                }
            }
        });
    });
    if let Some(c) = chosen {
        let ids: Vec<i64> = if app.selected.contains(&id) { app.list.iter().copied().filter(|t| app.selected.contains(t)).collect() } else { vec![id] };
        for tid in ids {
            if let Some(t) = app.track_mut(tid) {
                t.color = c;
            }
            app.save_track(tid);
        }
        app.ext.color_pick = None;
        return;
    }
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().map(|p| !area.response.rect.contains(p)).unwrap_or(false));
    if (outside && now - opened > 0.15) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ext.color_pick = None;
    }
}

// ------------------------------------------------------------------------------------ filter
fn tick(ui: &mut Ui, on: &mut bool, label: &str, tips: bool) -> bool {
    let r = ui.checkbox(on, RichText::new(label).size(11.0).color(if *on { WHITE } else { DIM2 }));
    let changed = r.changed();
    if tips {
        r.on_hover_text("Tick/untick to enable/disable Filter");
    }
    changed
}

/// Track Filter bar: BPM with a ± range, KEY (with related keys), RATING, COLOR and My Tag,
/// each with its own tick.
pub fn filter_bar(app: &mut App, ui: &mut Ui) {
    let tips = app.prefs.tooltips;
    let alnum = app.prefs.key_alnum;
    let before = app.ext.filter.clone();
    let before_all = app.filter_tags_all;
    ui.spacing_mut().item_spacing.x = 5.0;
    ui.spacing_mut().interact_size.y = 18.0;
    let mut f = app.ext.filter.clone();
    // BPM
    tick(ui, &mut f.bpm_on, "BPM", tips);
    let d = ui.add(egui::DragValue::new(&mut f.bpm).range(0.0..=499.0).speed(0.25).fixed_decimals(2));
    if d.changed() && f.bpm > 0.0 {
        f.bpm_on = true;
    }
    ui.spacing_mut().slider_width = 70.0;
    let s = ui.add(egui::Slider::new(&mut f.bpm_pct, 0.0..=16.0).step_by(1.0).show_value(false));
    if s.changed() {
        f.bpm_on = f.bpm > 0.0;
    }
    ui.label(RichText::new(format!("±{}%", f.bpm_pct as i32)).size(11.0).color(if f.bpm_on { WHITE } else { DIM2 }));
    // Take BPM and KEY from the loaded track.
    let src = app.deck.track.and_then(|id| app.track(id)).map(|t| (t.bpm, t.key));
    let mb = ui.add_enabled(src.is_some(), egui::Button::new(RichText::new("MASTER").size(9.5)).min_size(vec2(0.0, 16.0)));
    let mb = if tips { mb.on_hover_text("Set [BPM] and [KEY] as those of the track on MASTER PLAYER") } else { mb };
    if let (true, Some((bpm, key))) = (mb.clicked(), src) {
        if bpm > 0.0 {
            f.bpm = (bpm * 100.0).round() / 100.0;
            f.bpm_on = true;
        }
        if key >= 0 {
            f.key = key;
            f.key_on = true;
        }
        if bpm <= 0.0 || key < 0 {
            app.toast_err("The track loaded on MASTER PLAYER on CDJ has no BPM or KEY information.".replace(" on CDJ", ""));
        }
    }
    ui.separator();
    // KEY: a button showing the key opens the 24 key buttons.
    tick(ui, &mut f.key_on, "KEY", tips);
    let kb = ui.add(egui::Button::new(RichText::new(if f.key < 0 { "—".to_string() } else { key_name(f.key, alnum) }).size(11.0).color(WHITE)).min_size(vec2(40.0, 16.0)));
    let pid = ui.id().with("filter_keys");
    if kb.clicked() {
        ui.memory_mut(|m| m.toggle_popup(pid));
    }
    egui::popup::popup_below_widget(ui, pid, &kb, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(12.0 * 34.0 + 8.0);
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        // Two rows in key-wheel order: minor (A) keys above major (B) keys.
        for minor in [true, false] {
            ui.horizontal(|ui| {
                let mut keys: Vec<i32> = (0..24).filter(|k| key_alnum(*k).ends_with(if minor { 'A' } else { 'B' })).collect();
                keys.sort_by_key(|k| key_alnum(*k).trim_end_matches(['A', 'B']).parse::<i32>().unwrap_or(0));
                for k in keys {
                    let on = f.key == k;
                    if ui.add(egui::Button::new(RichText::new(key_name(k, alnum)).size(10.5).color(WHITE)).fill(if on { BLUE } else { BTN }).min_size(vec2(32.0, 18.0))).clicked() {
                        f.key = if on { -1 } else { k };
                        f.key_on = f.key >= 0;
                    }
                }
            });
        }
    });
    let rk = ui.add(egui::Button::new(RichText::new("Related key").size(10.0).color(WHITE)).fill(if f.key_related { BLUE } else { BTN }).min_size(vec2(0.0, 16.0)));
    if rk.clicked() {
        f.key_related = !f.key_related;
    }
    if tips {
        rk.on_hover_text("Also show keys next to it on the key wheel and its relative major/minor.");
    }
    ui.separator();
    // RATING
    tick(ui, &mut f.rating_on, "RATING", tips);
    let (sr, sresp) = ui.allocate_exact_size(vec2(5.0 * 13.0 + 2.0, 16.0), Sense::click());
    for s in 1..=5u8 {
        star(ui.painter(), pos2(sr.min.x + 7.0 + (s - 1) as f32 * 13.0, sr.center().y), 5.6, if f.rating >= s { WHITE.c() } else { DIM.c() }, f.rating >= s);
    }
    if sresp.clicked() {
        if let Some(pp) = sresp.interact_pointer_pos() {
            let s = (((pp.x - sr.min.x) / 13.0).floor() as i32 + 1).clamp(1, 5) as u8;
            f.rating = if f.rating == s { 0 } else { s };
            f.rating_on = true;
        }
    }
    ui.separator();
    // COLOR
    tick(ui, &mut f.color_on, "COLOR", tips);
    for i in 1..=8u8 {
        let (cr, cresp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        let on = (f.colors >> i) & 1 == 1;
        ui.painter().rect_filled(cr.shrink(2.0), 1.0, if on || f.colors == 0 { label_color(i) } else { label_color(i).gamma_multiply(0.35) });
        if on {
            ui.painter().rect_stroke(cr, 2.0, Stroke::new(1.2, WHITE), egui::StrokeKind::Inside);
        }
        let cresp = if tips { cresp.on_hover_text(COLOR_NAMES[i as usize]) } else { cresp };
        if cresp.clicked() {
            f.colors ^= 1 << i;
            f.color_on = f.colors != 0;
        }
    }
    ui.separator();
    // MY TAG
    tick(ui, &mut f.tags_on, "MY TAG", tips);
    let n = app.filter_tags.len();
    let all = app.filter_tags_all;
    for (is_all, label, tip) in [(true, "AND", "When multiple My Tags are selected, tracks that are related to all of the selected My Tags will be displayed. "), (false, "OR", "When multiple My Tags are selected, tracks that are related to any of the selected My Tags will be displayed.")] {
        let b = ui.add(egui::Button::new(RichText::new(label).size(10.0).color(WHITE)).fill(if all == is_all { BLUE } else { BTN }).min_size(vec2(0.0, 16.0)));
        let b = if tips { b.on_hover_text(tip) } else { b };
        if b.clicked() {
            app.filter_tags_all = is_all;
        }
    }
    let tb = ui.add(egui::Button::new(RichText::new(if n == 0 { "none".to_string() } else { format!("{n} selected") }).size(10.5).color(if n == 0 { DIM2 } else { WHITE })).min_size(vec2(0.0, 16.0)));
    let tpid = ui.id().with("filter_tags");
    if tb.clicked() {
        ui.memory_mut(|m| m.toggle_popup(tpid));
    }
    egui::popup::popup_below_widget(ui, tpid, &tb, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(200.0);
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            let cats: Vec<MyTag> = app.mytags.iter().filter(|t| t.parent == 0).cloned().collect();
            for c in cats {
                ui.label(RichText::new(&c.name).size(11.0).color(DIM2).strong());
                let tags: Vec<MyTag> = app.mytags.iter().filter(|t| t.parent == c.id).cloned().collect();
                for tg in tags {
                    let mut on = app.filter_tags.contains(&tg.id);
                    if ui.checkbox(&mut on, &tg.name).changed() {
                        if on {
                            app.filter_tags.insert(tg.id);
                        } else {
                            app.filter_tags.remove(&tg.id);
                        }
                        app.list_dirty = true;
                    }
                }
            }
        });
    });
    ui.separator();
    if ui.add(egui::Button::new(RichText::new("RESET").size(10.0)).min_size(vec2(0.0, 16.0))).clicked() {
        f = TrackFilter::default();
        app.filter_tags.clear();
        app.list_dirty = true;
    }
    if f != before || app.filter_tags_all != before_all {
        app.ext.filter = f;
        app.list_dirty = true;
    }
}

// ------------------------------------------------------------------------------------ menus
/// "Add To Playlist" submenu: the playlist tree with its folders.
pub fn add_to_playlist_menu(app: &mut App, ui: &mut Ui, sel: &[i64]) {
    fn level(app: &mut App, ui: &mut Ui, parent: i64, sel: &[i64], depth: usize) {
        if depth > 32 {
            return;
        }
        let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == parent && n.kind != NodeKind::Smart).cloned().collect();
        for n in kids {
            if n.kind == NodeKind::Folder {
                ui.menu_button(&n.name, |ui| level(app, ui, n.id, sel, depth + 1));
            } else if ui.button(&n.name).clicked() {
                app.add_to_playlist(n.id, sel);
                ui.close_menu();
            }
        }
        if depth > 0 && !app.nodes.iter().any(|n| n.parent == parent && n.kind != NodeKind::Smart) {
            ui.label(RichText::new("(empty)").color(DIM));
        }
        ui.separator();
        if ui.button("Create New Playlist").clicked() {
            let id = app.create_node(parent, NodeKind::Playlist);
            app.add_to_playlist(id, sel);
            app.side = SideMode::Playlists;
            ui.close_menu();
        }
    }
    level(app, ui, 0, sel, 0);
}

/// A menu entry with a tick box (rekordbox's checkmark entries); true when clicked.
fn check_item(ui: &mut Ui, on: bool, label: &str) -> egui::Response {
    let mut v = on;
    ui.checkbox(&mut v, label)
}

fn match_lookup_item(app: &mut App, ui: &mut Ui, sel: &[i64]) {
    if ui.button("Look up related track tags…").clicked() {
        crate::ui_dialogs::open_match_lookup(app, sel);
        ui.close_menu();
    }
}

/// The track context menu, in the order rekordbox 7.2.19 builds it for collection and
/// playlist rows (`BrowsePopupMenuManager::showListViewPopupMenu`, traced in
/// `checks/re-7.2.19/shell-menus-traffic.md` section 5). recordbox's own entries, which
/// rekordbox has no item for, come after the last separator.
pub fn track_menu(app: &mut App, ui: &mut Ui, sel: &[i64]) {
    if sel.is_empty() {
        ui.label(RichText::new("No track selected").color(DIM));
        return;
    }
    let one = sel.len() == 1;
    let manual = app.is_manual_playlist();
    let remove_items = |app: &mut App, ui: &mut Ui| {
        if let Some(pl) = manual {
            if ui.button("Remove from Playlist").clicked() {
                let s: HashSet<i64> = sel.iter().copied().collect();
                app.remove_from_playlist(pl, &s);
                ui.close_menu();
            }
        } else if let View::History(h) = app.view {
            if ui.button("Remove from History").clicked() {
                app.history_remove_tracks(h, sel);
                ui.close_menu();
            }
        } else if ui.button("Remove from Collection").clicked() {
            app.dialogs.confirm_remove = Some(sel.to_vec());
            ui.close_menu();
        }
    };
    // Rows whose file is gone get rekordbox's short menu.
    if sel.iter().all(|id| app.track(*id).map(|t| t.missing).unwrap_or(false)) {
        ui.add_enabled(false, egui::Button::new("File is Missing"));
        ui.separator();
        if ui.button("Auto Relocate").clicked() {
            ui.close_menu();
            app.auto_relocate(Some(sel.to_vec()));
        }
        if ui.add_enabled(one, egui::Button::new("Relocate")).clicked() {
            ui.close_menu();
            if let Some(f) = rfd::FileDialog::new().pick_file() {
                app.relocate_to(sel[0], &f);
            }
        }
        if app.is_manual_playlist().is_some() || matches!(app.view, View::History(_)) {
            remove_items(app, ui);
        } else if ui.button("Review collection cleanup…").clicked() {
            crate::collection_cleanup::ui::open(app, Some(sel.iter().copied().collect()));
            ui.close_menu();
        }
        match_lookup_item(app, ui, sel);
        return;
    }
    // Load > Deck1..Deck4: always listed, usable in PERFORMANCE.
    let perf = app.performance();
    if !perf && one && ui.add_enabled(!app.deck_options.disable_load, egui::Button::new("Load to Player")).clicked() {
        app.load_track(sel[0]);
        ui.close_menu();
    }
    ui.add_enabled_ui(perf && one && !app.deck_options.disable_load, |ui| {
        ui.menu_button("Load", |ui| {
            let count = app.deck_count();
            for n in 0..4u8 {
                if ui.add_enabled(n < count, egui::Button::new(format!("Deck{}", n + 1))).clicked() {
                    let id = sel[0];
                    app.with_deck(n, |a| a.load_track_here(id));
                    ui.close_menu();
                }
            }
        });
    });
    ui.separator();
    // Collection rows are in the collection already.
    ui.add_enabled(false, egui::Button::new("Import To Collection"));
    if ui.button(if one { "Analyze Track" } else { "Analyze Tracks" }).clicked() {
        app.reanalyze(sel);
        ui.close_menu();
    }
    match_lookup_item(app, ui, sel);
    let locked = sel.iter().all(|id| app.track(*id).map(|t| t.grid_locked).unwrap_or(false));
    let unlocked = sel.iter().all(|id| app.track(*id).map(|t| !t.grid_locked).unwrap_or(false));
    ui.menu_button("Analysis Lock", |ui| {
        for (on, label, enabled) in [(true, "On", !locked), (false, "Off", !unlocked)] {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                for id in sel {
                    if let Some(t) = app.track_mut(*id) {
                        t.grid_locked = on;
                    }
                    app.save_track(*id);
                }
                ui.close_menu();
            }
        }
    });
    ui.separator();
    ui.menu_button("Add To Playlist", |ui| add_to_playlist_menu(app, ui, sel));
    let all_tagged = sel.iter().all(|t| app.ext.tag_set.contains(t));
    if all_tagged {
        if ui.button("Remove from Tag List").clicked() {
            app.tag_list_remove(sel);
            ui.close_menu();
        }
    } else if ui.button("Add To Tag List").clicked() {
        app.tag_list_add(sel);
        ui.close_menu();
    }
    if ui.button(if one { "Reload Tag" } else { "Reload Tags" }).clicked() {
        app.reload_tags_from_files(sel);
        ui.close_menu();
    }
    if ui.button("Get Info from iTunes").clicked() {
        app.get_info_from_itunes(sel);
        ui.close_menu();
    }
    // PERFORMANCE: Add To Sampler List (ui_sampler_lib.rs).
    crate::ui_sampler_lib::track_menu(app, ui, sel);
    // Track Type: how the track plays in the sampler.
    let kind = if one { app.track_type(sel[0]) } else { 255 };
    ui.menu_button("Track Type", |ui| {
        for (mode, label) in [(1u8, "Oneshot"), (2, "Loop"), (0, "Normal")] {
            if ui.add_enabled(kind != mode, egui::Button::new(label)).clicked() {
                app.set_track_type(sel, mode);
                ui.close_menu();
            }
        }
    });
    ui.separator();
    ui.menu_button(if one { "Export Track" } else { "Export Tracks" }, |ui| {
        let devices: Vec<(String, String)> = app.devices.iter().map(|d| (d.root.clone(), d.name.clone())).collect();
        for (root, name) in devices {
            if ui.button(name).clicked() {
                crate::ui_browser::open_export(app, vec![], sel.to_vec());
                app.dialogs.export_root = Some(root);
                ui.close_menu();
            }
        }
        if !app.devices.is_empty() {
            ui.separator();
        }
        if ui.button("Export to USB Device…").clicked() {
            crate::ui_browser::open_export(app, vec![], sel.to_vec());
            ui.close_menu();
        }
    });
    ui.separator();
    ui.menu_button("Auto Load Hot Cue", |ui| {
        let auto = sel.iter().all(|id| app.track(*id).map(|t| t.autoload_hotcue).unwrap_or(false));
        let none = sel.iter().all(|id| app.track(*id).map(|t| !t.autoload_hotcue).unwrap_or(false));
        for (on, label, now) in [(true, "Enable Auto Load Hot Cue", auto), (false, "Disable Auto Load Hot Cue", none)] {
            if check_item(ui, now, label).clicked() {
                for id in sel {
                    if let Some(t) = app.track_mut(*id) {
                        t.autoload_hotcue = on;
                    }
                    app.save_track(*id);
                }
                ui.close_menu();
            }
        }
    });
    if ui.button(if one { "Reset DJ Play Count" } else { "Reset DJ Play Counts" }).clicked() {
        for id in sel {
            if let Some(t) = app.track_mut(*id) {
                t.play_count = 0;
            }
            app.save_track(*id);
        }
        ui.close_menu();
    }
    if ui.button("Add New Analysis Data").clicked() {
        app.analyze_missing(sel);
        ui.close_menu();
    }
    if ui.button("Convert Memory Cues to Hot Cues").clicked() {
        app.convert_memory_cues(sel);
        ui.close_menu();
    }
    if ui.button("Sort Hot Cues by time").clicked() {
        app.cue_tools.sort_ask = Some(sel.to_vec());
        ui.close_menu();
    }
    ui.separator();
    remove_items(app, ui);
    ui.separator();
    let info = crate::ui_browser_ext::right_panel(app) == 4;
    if ui.button(if info { "Hide information" } else { "Show information" }).clicked() {
        crate::ui_browser_ext::set_right_panel(app, if info { 0 } else { 4 });
        ui.close_menu();
    }
    if ui.button(crate::ui_browser::REVEAL_FILE).clicked() {
        if let Some(t) = app.track(sel[0]) {
            if let Err(err) = crate::ui_browser::reveal_path(std::path::Path::new(&t.path)) {
                app.toast_err(format!("Could not open the file location: {err}"));
            }
        }
        ui.close_menu();
    }
    // PERFORMANCE: Add tracks to Automix playlist (ui_automix.rs).
    if app.performance() {
        ui.separator();
        crate::ui_automix::track_menu(app, ui, sel);
    }
    // ---- recordbox's own
    ui.separator();
    if ui.button(if one { "Analyze Key" } else { "Analyze Keys" }).clicked() {
        app.analyze_key_tracks(sel);
        ui.close_menu();
    }
    crate::ui_stem_export::track_menu(app, ui, sel);
    if app.analysis_total > 0 && ui.button("Stop Analyzing").clicked() {
        app.pool.stop();
        ui.close_menu();
    }
    if one && ui.button("Change Artwork").clicked() {
        ui.close_menu();
        add_artwork(app, sel);
    }
    // Matching with the track in the player.
    if let Some(src) = app.deck.track.filter(|s| !(one && sel[0] == *s)) {
        let name = app.track(src).map(|t| t.display_title()).unwrap_or_default();
        let others: Vec<i64> = sel.iter().copied().filter(|t| *t != src).collect();
        let linked = others.iter().all(|t| app.is_match(src, *t));
        ui.menu_button("Matching", |ui| {
            if check_item(ui, linked, &format!("Good match with \"{name}\"")).on_hover_text("Set a good match combination of tracks.\nSee also Related Track List window.(Matching)").clicked() {
                for t in &others {
                    app.matching_set(src, *t, !linked);
                }
                ui.close_menu();
            }
        });
    }
}

/// Collection ("All" / "All Tracks") row menu.
pub fn collection_menu(app: &mut App, ui: &mut Ui) {
    if ui.button("Import Track").clicked() {
        ui.close_menu();
        if let Some(files) = rfd::FileDialog::new().set_title("Import Track").add_filter("Audio", &["mp3", "m4a", "aac", "wav", "aif", "aiff", "flac", "alac", "mp4"]).pick_files() {
            app.import_paths(files, None);
        }
    }
    if ui.button("Import Folder").clicked() {
        ui.close_menu();
        if let Some(d) = rfd::FileDialog::new().set_title("Import Folder").pick_folder() {
            app.import_paths(vec![d], None);
        }
    }
    if ui.button("Export Collection in xml format").clicked() {
        ui.close_menu();
        if let Some(f) = rfd::FileDialog::new().set_file_name("collection.xml").save_file() {
            app.export_collection_xml(&f);
        }
    }
    ui.separator();
    if ui.button("Display All Missing Files").clicked() {
        ui.close_menu();
        app.open_missing_file_manager();
    }
    if ui.button("Review collection cleanup…").clicked() {
        ui.close_menu();
        crate::collection_cleanup::ui::open(app, None);
    }
    if ui.button("Restore Removed Tracks…").clicked() {
        ui.close_menu();
        crate::menus::handle(app, "trash");
    }
}

/// "Import Playlist" file chooser, into `parent` (0 = the Playlists root).
pub fn import_playlist_dialog(app: &mut App, parent: i64) {
    if let Some(files) = rfd::FileDialog::new().set_title("Import Playlist *.m3u, *.pls").add_filter("Playlist", &["m3u", "m3u8", "pls", "txt"]).pick_files() {
        for f in files {
            app.import_playlist_file(&f, parent);
        }
    }
}

/// "Save Playlist Info": choose the file (its extension picks the format).
pub fn export_playlist_dialog(app: &mut App, node: i64) {
    let name = app.node(node).map(|n| crate::playlist_files::safe_file_name(&n.name)).unwrap_or_else(|| "playlist".into());
    let dlg = rfd::FileDialog::new().set_title("Export a playlist to a file").set_file_name(format!("{name}.txt")).add_filter("Export a playlist to a file (*.txt)", &["txt"]).add_filter("Export a playlist to a file for music apps (*.m3u8)", &["m3u8"]).add_filter("PLS (*.pls)", &["pls"]);
    if let Some(f) = dlg.save_file() {
        app.export_playlist_file(node, &f);
    }
}

/// The right-click menu of a playlist, intelligent playlist or folder in the tree, in the
/// order rekordbox 7.2.19 builds it (`BrowsePopupMenuManager::showTreeViewPopupMenu`
/// 0x100118eec; `checks/re-7.2.19/shell-menus-traffic.md`). Left out: cloud, sharing and
/// Request Catalog items. recordbox's own entries
/// come after the last separator. A name is changed by double-clicking it, as in rekordbox;
/// Rename is also kept among recordbox's own entries.
pub fn node_menu(app: &mut App, ui: &mut Ui, n: &PlaylistNode) {
    let folder = n.kind == NodeKind::Folder;
    let parent_for_new = if folder { n.id } else { n.parent };
    if n.kind == NodeKind::Smart && ui.button("Edit the Intelligent Playlist").clicked() {
        app.dialogs.smart_edit = Some((Some(n.id), n.parent, n.name.clone(), n.smart.clone().unwrap_or_default()));
        ui.close_menu();
    }
    ui.menu_button(if folder { "Export Folder" } else { "Export Playlist" }, |ui| {
        let devices: Vec<(String, String)> = app.devices.iter().map(|d| (d.root.clone(), d.name.clone())).collect();
        for (root, name) in devices {
            if ui.button(name).clicked() {
                crate::ui_browser::open_export(app, vec![n.id], vec![]);
                app.dialogs.export_root = Some(root);
                ui.close_menu();
            }
        }
        if !app.devices.is_empty() {
            ui.separator();
        }
        if ui.button("Export to USB Device…").clicked() {
            crate::ui_browser::open_export(app, vec![n.id], vec![]);
            ui.close_menu();
        }
    });
    ui.separator();
    for (label, kind) in [("Create New Playlist", NodeKind::Playlist), ("Create New Intelligent Playlist", NodeKind::Smart), ("Create New Folder", NodeKind::Folder)] {
        if ui.button(label).clicked() {
            app.create_node(parent_for_new, kind);
            ui.close_menu();
        }
    }
    ui.separator();
    // Playlist artwork (playlist_art.rs): between the Create items and Delete Playlist.
    if n.kind == NodeKind::Playlist {
        crate::playlist_art::menu(app, ui, n.id);
    }
    if ui.button(if folder { "Delete Folder" } else { "Delete Playlist" }).clicked() {
        app.dialogs.confirm_delete_node = Some((n.id, n.name.clone(), folder));
        ui.close_menu();
    }
    ui.separator();
    if folder {
        if ui.button("Sort Items").clicked() {
            app.sort_node_children(n.id);
            ui.close_menu();
        }
    } else {
        ui.menu_button("Save Playlist Info", |ui| {
            for (label, ext) in [("Export a playlist to a file (*.txt)", "txt"), ("Export a playlist to a file for music apps (*.m3u8)", "m3u8")] {
                if ui.button(label).clicked() {
                    ui.close_menu();
                    let name = crate::playlist_files::safe_file_name(&n.name);
                    if let Some(f) = rfd::FileDialog::new().set_title("Export a playlist to a file").set_file_name(format!("{name}.{ext}")).add_filter(label, &[ext]).save_file() {
                        app.export_playlist_file(n.id, &f);
                    }
                }
            }
        });
    }
    ui.separator();
    if app.prefs.shortcuts.contains(&n.id) {
        if ui.button("Delete Shortcut").clicked() {
            app.shortcut_remove(n.id);
            ui.close_menu();
        }
    } else if ui.button("Add To Shortcut").clicked() {
        app.shortcut_add(n.id);
        ui.close_menu();
    }
    if !folder && app.performance() {
        ui.separator();
        crate::ui_automix::node_menu(app, ui, n.id);
        ui.separator();
        if ui.button("Load the top 16 tracks into the current sampler bank").clicked() {
            crate::app_hooks::playlist_to_sampler_bank(app, n.id);
            ui.close_menu();
        }
    }
    // ---- recordbox's own
    ui.separator();
    if ui.button("Rename").clicked() {
        app.dialogs.rename = Some((n.id, n.name.clone()));
        ui.close_menu();
    }
    if ui.button("Import Playlist").clicked() {
        ui.close_menu();
        import_playlist_dialog(app, parent_for_new);
    }
    if n.kind == NodeKind::Smart && ui.button("Convert to a normal playlist").clicked() {
        let tracks = n.smart.as_ref().map(|r| app.smart_tracks(r)).unwrap_or_default();
        let id = app.db.create_node(n.parent, &format!("{} (copy)", n.name), NodeKind::Playlist, None).unwrap_or(0);
        let _ = app.db.set_playlist_tracks(id, &tracks);
        app.reload_playlists();
        ui.close_menu();
    }
    if n.kind == NodeKind::Playlist {
        let can = app.view == View::Playlist(n.id) && app.can_renumber();
        if ui.add_enabled(can, egui::Button::new("Renumber Track Order")).on_disabled_hover_text("Sort the playlist by a column other than # first.").clicked() {
            app.renumber_track_order(n.id);
            ui.close_menu();
        }
        if let Some(slot) = app.prefs.palette.iter().position(|p| *p == n.id) {
            if ui.button("Remove from Palette").clicked() {
                app.palette_remove(slot);
                ui.close_menu();
            }
        } else {
            ui.menu_button("Playlist Palette", |ui| {
                for slot in 0..PALETTE_SLOTS {
                    let cur = app.prefs.palette.get(slot).copied().unwrap_or(0);
                    let label = format!("{}: {}", slot + 1, app.node(cur).map(|x| x.name.clone()).unwrap_or_else(|| "not assigned".into()));
                    if ui.button(label).clicked() {
                        app.palette_assign(slot, n.id);
                        app.prefs.palette_open = true;
                        app.save_prefs();
                        ui.close_menu();
                    }
                }
            });
        }
    }
    if n.parent != 0 && ui.button("Move to top level").clicked() {
        let _ = app.db.move_node(n.id, 0, 0);
        app.reload_playlists();
        ui.close_menu();
    }
    ui.menu_button("Move into folder", |ui| {
        let folders: Vec<PlaylistNode> = app.nodes.iter().filter(|f| f.kind == NodeKind::Folder && f.id != n.id).cloned().collect();
        for f in folders {
            if ui.button(&f.name).clicked() {
                let _ = app.db.move_node(n.id, f.id, 0);
                app.reload_playlists();
                ui.close_menu();
            }
        }
    });
}

/// Menu of the "Playlists" root row (after the create entries).
pub fn root_menu_extra(app: &mut App, ui: &mut Ui) {
    ui.separator();
    if ui.button("Import Playlist").clicked() {
        ui.close_menu();
        import_playlist_dialog(app, 0);
    }
    if ui.button("Export Playlists Information").clicked() {
        ui.close_menu();
        if let Some(d) = rfd::FileDialog::new().set_title("Export Playlists Information").pick_folder() {
            app.export_all_playlists_info(&d);
        }
    }
    if ui.button("Sort Items").clicked() {
        app.sort_node_children(0);
        ui.close_menu();
    }
}

// ------------------------------------------------------------------------------------ tree bits
/// The tree search field (under the Tree View row while the magnifier tile is on).
pub fn tree_search_field(app: &mut App, ui: &mut Ui) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 21.0), Sense::hover());
    let fr = Rect::from_min_max(pos2(row.min.x + 2.0, row.min.y + 1.0), pos2(row.max.x - 2.0, row.max.y - 2.0));
    ui.painter().rect_filled(fr, 0.0, BG);
    let mut s = app.ext.tree_search.clone();
    let te = ui.put(Rect::from_min_max(pos2(fr.min.x + 4.0, fr.min.y), pos2(fr.max.x - 16.0, fr.max.y)), egui::TextEdit::singleline(&mut s).frame(false).hint_text(RichText::new("Search").color(DIM)).vertical_align(egui::Align::Center).font(FontId::proportional(11.5)));
    if te.changed() {
        app.ext.tree_search = s;
    }
    let focus_key = egui::Id::new("tree_search_focus");
    if ui.ctx().data(|d| d.get_temp::<bool>(focus_key)).unwrap_or(false) {
        te.request_focus();
        ui.ctx().data_mut(|d| d.insert_temp(focus_key, false));
    }
    let xr = Rect::from_center_size(pos2(fr.max.x - 8.0, fr.center().y), vec2(12.0, 12.0));
    let x = ui.interact(xr, ui.id().with("tree_search_clear"), Sense::click());
    crate::icons::cross(ui.painter(), xr.shrink(3.0), crate::ui_feel::hl(&x, DIM2.c(), WHITE.c()), 1.2);
    if x.clicked() {
        app.ext.tree_search.clear();
    }
}

/// Tag List row and shortcuts at the top of the playlist tree. Rows that take dropped tracks
/// are added to `drop` (Tag List as -1).
pub fn tree_top_rows(app: &mut App, ui: &mut Ui, drop: &mut Vec<(Rect, i64)>) {
    let n = app.ext.tag_list.len();
    if app.prefs.tree_item("Tag List") {
    let r = tree_row(ui, 0, "Tag List", Some(n), app.view == View::TagList, TreeIcon::Note, 15.0);
    {
        // The tag-list glyph over the note placeholder.
        let ir = Rect::from_min_size(pos2(r.rect.min.x + 4.0 + 15.0 - 1.0, r.rect.min.y), vec2(14.0, 15.0));
        ui.painter().rect_filled(ir, 0.0, if app.view == View::TagList { ROW_SEL.c() } else { crate::ui_feel::hl(&r, BG2.c(), gy(0x262626)) });
        icon_tag_list(ui.painter(), ir.center() + vec2(0.0, 0.5), DIM2.c());
    }
    crate::ui_browser::register_drop_target(ui, drop, r.interact_rect, -1);
    if r.clicked() {
        app.set_view(View::TagList);
    }
    r.context_menu(|ui| {
        if ui.add_enabled(n > 0, egui::Button::new("Save as Playlist")).clicked() {
            app.tag_list_save_as_playlist();
            ui.close_menu();
        }
        if ui.add_enabled(n > 0, egui::Button::new("Delete All")).clicked() {
            app.tag_list_clear();
            ui.close_menu();
        }
    });
    }
    // Shortcuts (up to 8), behind an open/close row.
    if !app.prefs.shortcuts.is_empty() {
        let open = app.prefs.shortcuts_open;
        let h = tree_row(ui, 0, "Shortcuts", None, false, if open { TreeIcon::Open } else { TreeIcon::Closed }, 15.0);
        let h = if app.prefs.tooltips { h.on_hover_text("Open/close shortcuts") } else { h };
        if h.clicked() {
            app.prefs.shortcuts_open = !open;
            app.save_prefs();
        }
        if open {
            for id in app.prefs.shortcuts.clone() {
                let Some(node) = app.node(id).cloned() else { continue };
                let icon = if node.kind == NodeKind::Folder { TreeIcon::FolderClosed } else if node.kind == NodeKind::Smart { TreeIcon::Smart } else { TreeIcon::Playlist };
                let count = (node.kind == NodeKind::Playlist).then(|| app.node_counts.get(&id).copied().unwrap_or(0));
                let r = tree_row(ui, 1, &node.name, count, false, icon, 15.0);
                if node.kind == NodeKind::Playlist {
                    crate::ui_browser::register_drop_target(ui, drop, r.interact_rect, id);
                }
                if r.clicked() {
                    app.jump_to_playlist(id);
                }
                r.context_menu(|ui| {
                    if ui.button("Delete Shortcut").clicked() {
                        app.shortcut_remove(id);
                        ui.close_menu();
                    }
                });
            }
        }
    }
}

/// Playlist Palette: four slots under the track list. Click opens the playlist, tracks dropped
/// on a slot are added to it, a playlist dragged from the tree is assigned to it.
pub fn palette_bar(app: &mut App, ui: &mut Ui, r: Rect, drop: &mut Vec<(Rect, i64)>) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, BG2);
    p.text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, "Playlist Palette", FontId::proportional(10.5), DIM2.c());
    let x0 = r.min.x + 96.0;
    let w = ((r.max.x - 6.0 - x0) / PALETTE_SLOTS as f32).max(40.0);
    app.prefs.palette.resize(PALETTE_SLOTS, 0);
    for slot in 0..PALETTE_SLOTS {
        let sr = Rect::from_min_size(pos2(x0 + slot as f32 * w, r.min.y + 3.0), vec2(w - 4.0, r.height() - 6.0));
        let pl = app.prefs.palette[slot];
        let node = app.node(pl).cloned();
        let resp = ui.interact(sr, ui.id().with(("palette", slot)), Sense::click());
        let showing = node.as_ref().map(|n| app.view == View::Playlist(n.id)).unwrap_or(false);
        p.rect_filled(sr, 2.0, if showing { ROW_SEL.c() } else { crate::ui_feel::hl(&resp, BTN.c(), gy(0x3A3A3A)) });
        p.text(pos2(sr.min.x + 7.0, sr.center().y), Align2::LEFT_CENTER, (slot + 1).to_string(), bold(10.0), DIM2.c());
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(sr.min.x + 20.0, sr.min.y), pos2(sr.max.x - 30.0, sr.max.y)));
        match &node {
            Some(n) => {
                clip.text(pos2(sr.min.x + 20.0, sr.center().y), Align2::LEFT_CENTER, &n.name, FontId::proportional(11.5), WHITE.c());
                p.text(pos2(sr.max.x - 7.0, sr.center().y), Align2::RIGHT_CENTER, app.node_counts.get(&n.id).copied().unwrap_or(0).to_string(), FontId::proportional(11.0), WHITE.c());
                crate::ui_browser::register_drop_target(ui, drop, resp.interact_rect.intersect(r), n.id);
            }
            None => {
                clip.text(pos2(sr.min.x + 20.0, sr.center().y), Align2::LEFT_CENTER, "not assigned", FontId::proportional(11.0), DIM.c());
            }
        }
        // A playlist dragged from the tree lands here.
        if let Some(dragged) = app.node_drag {
            if ui.rect_contains_pointer(resp.interact_rect.intersect(r)) && app.node(dragged).map(|n| n.kind == NodeKind::Playlist).unwrap_or(false) {
                p.rect_stroke(sr, 2.0, Stroke::new(1.5, BLUE), egui::StrokeKind::Inside);
                if ui.input(|i| i.pointer.any_released()) {
                    app.palette_assign(slot, dragged);
                    app.node_drag = None;
                }
            }
        }
        let resp = if app.prefs.tooltips { resp.on_hover_text("You can assign a playlist to a palette. You can drag and drop a track to the playlist of the palette. ") } else { resp };
        if resp.clicked() {
            match &node {
                Some(n) => app.jump_to_playlist(n.id),
                None => app.ext.palette_pick = Some(slot),
            }
        }
        let pid = ui.id().with(("palette_pick", slot));
        if app.ext.palette_pick == Some(slot) {
            ui.memory_mut(|m| m.open_popup(pid));
            app.ext.palette_pick = None;
        }
        egui::popup::popup_above_or_below_widget(ui, pid, &resp, egui::AboveOrBelow::Above, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(180.0);
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                let pls: Vec<(i64, String)> = app.nodes.iter().filter(|n| n.kind == NodeKind::Playlist).map(|n| (n.id, n.name.clone())).collect();
                if pls.is_empty() {
                    ui.label(RichText::new("No playlists yet").color(DIM));
                }
                for (id, name) in pls {
                    if ui.selectable_label(false, name).clicked() {
                        app.palette_assign(slot, id);
                        ui.memory_mut(|m| m.close_popup());
                    }
                }
            });
        });
        resp.context_menu(|ui| {
            if ui.add_enabled(node.is_some(), egui::Button::new("Remove from Palette")).clicked() {
                app.palette_remove(slot);
                ui.close_menu();
            }
        });
    }
}

// ------------------------------------------------------------------------------------ histories
/// Histories: Recordbox's own play histories by year and month, then the histories players
/// saved on connected devices.
pub fn history_tree(app: &mut App, ui: &mut Ui) {
    if app.devices_scanned.map(|t| t.elapsed().as_secs() > 4).unwrap_or(true) && app.export_job.is_none() {
        app.scan_devices();
    }
    ui.add_space(1.0);
    let root = tree_row(ui, 0, "Histories", None, matches!(app.view, View::History(0)), TreeIcon::Open, 15.0);
    if root.clicked() {
        app.set_view(View::History(0));
    }
    let any_dev = app.devices.iter().any(|d| d.lib.as_ref().map(|l| !l.histories.is_empty()).unwrap_or(false));
    let have = !app.ext.histories.is_empty();
    root.context_menu(|ui| {
        if ui.add_enabled(have, egui::Button::new("Clear History")).clicked() {
            app.ext.history_confirm = Some(None);
            ui.close_menu();
        }
        if ui.add_enabled(any_dev, egui::Button::new("Import All Histories")).clicked() {
            app.import_device_histories();
            ui.close_menu();
        }
    });
    egui::ScrollArea::vertical().id_salt("history_tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let groups = history_groups(&app.ext.histories);
        let newest = groups.first().map(|y| (y.0.clone(), y.1.first().map(|m| m.0.clone()).unwrap_or_default()));
        for (year, months) in groups {
            let yk = egui::Id::new(("hist_year", year.clone()));
            let ydef = newest.as_ref().map(|n| n.0 == year).unwrap_or(false);
            let yopen = ui.ctx().data(|d| d.get_temp::<bool>(yk)).unwrap_or(ydef);
            if tree_row(ui, 1, &year, None, false, if yopen { TreeIcon::FolderOpen } else { TreeIcon::FolderClosed }, 15.0).clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(yk, !yopen));
            }
            if !yopen {
                continue;
            }
            for (month, list) in months {
                let mk = egui::Id::new(("hist_month", year.clone(), month.clone()));
                let mdef = newest.as_ref().map(|n| n.0 == year && n.1 == month).unwrap_or(false);
                let mopen = ui.ctx().data(|d| d.get_temp::<bool>(mk)).unwrap_or(mdef);
                if tree_row(ui, 2, month_name(&month), None, false, if mopen { TreeIcon::FolderOpen } else { TreeIcon::FolderClosed }, 15.0).clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(mk, !mopen));
                }
                if !mopen {
                    continue;
                }
                for h in list {
                    let sel = app.view == View::History(h.id);
                    let r = tree_row(ui, 3, &h.name, Some(h.count), sel, TreeIcon::Playlist, 15.0);
                    if r.clicked() {
                        app.set_view(View::History(h.id));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Save as Playlist").on_hover_text("Turn the play history into a playlist").clicked() {
                            if let Some(pl) = app.history_save_as_playlist(h.id) {
                                app.jump_to_playlist(pl);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Delete History").clicked() {
                            app.ext.history_confirm = Some(Some(h.id));
                            ui.close_menu();
                        }
                    });
                }
            }
        }
        // Device histories.
        let devs: Vec<(String, String, Vec<(u32, String, usize)>)> = app.devices.iter().filter_map(|d| d.lib.as_ref().map(|l| (d.root.clone(), d.name.clone(), l.histories.iter().map(|h| (h.id, h.name.clone(), h.entries.len())).collect()))).collect();
        for (root, name, hist) in devs {
            if hist.is_empty() {
                continue;
            }
            let dr = tree_row(ui, 1, &name, None, false, TreeIcon::Device, 15.0);
            dr.context_menu(|ui| {
                if ui.button("Import All Histories").clicked() {
                    app.import_device_histories();
                    ui.close_menu();
                }
            });
            for (id, hname, n) in hist {
                let sel = app.view == View::DeviceHistory { root: root.clone(), id };
                let r = tree_row(ui, 2, &hname, Some(n), sel, TreeIcon::Playlist, 15.0);
                if r.clicked() {
                    app.set_view(View::DeviceHistory { root: root.clone(), id });
                }
                let root2 = root.clone();
                r.context_menu(|ui| {
                    if ui.button("import History").clicked() {
                        if let Some(pl) = app.import_device_history(&root2, id) {
                            app.jump_to_playlist(pl);
                        }
                        ui.close_menu();
                    }
                });
            }
        }
        if !have && !any_dev {
            ui.add_space(8.0);
            ui.label(RichText::new("Tracks you play for about a minute are listed here by day. Histories saved by CDJ/XDJ players appear when their USB drive is connected.").size(11.0).color(DIM));
        }
    });
}

// ------------------------------------------------------------------------------------ related / matching
/// Related Tracks tree pane: the saved lists, Matching, and the criteria of the selected list.
pub fn related_panel(app: &mut App, ui: &mut Ui) {
    ui.add_space(1.0);
    let head = tree_row(ui, 0, "Related Tracks", None, false, TreeIcon::Open, 15.0);
    head.context_menu(|ui| related_menu(app, ui, None));
    let lists: Vec<String> = app.prefs.related_lists.iter().map(|l| l.name.clone()).collect();
    for (i, name) in lists.iter().enumerate() {
        if let Some((ri, text)) = app.ext.related_rename.clone() {
            if ri == i {
                let mut s = text;
                let resp = ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    ui.add(egui::TextEdit::singleline(&mut s).desired_width(150.0))
                }).inner;
                // Whether the field just gave up focus must be read before focus is asked for again.
                let gave_up = resp.lost_focus();
                if !gave_up {
                    resp.request_focus();
                }
                app.ext.related_rename = Some((ri, s.clone()));
                if gave_up {
                    if !s.trim().is_empty() {
                        app.prefs.related_lists[i].name = s.trim().to_string();
                        app.save_prefs();
                    }
                    app.ext.related_rename = None;
                }
                continue;
            }
        }
        let sel = app.view == View::Related && app.prefs.related_sel == i;
        let r = tree_row(ui, 1, name, None, sel, TreeIcon::Playlist, 15.0);
        if r.clicked() {
            app.related_select(i);
            app.set_view(View::Related);
        }
        if r.double_clicked() {
            app.ext.related_rename = Some((i, name.clone()));
        }
        r.context_menu(|ui| related_menu(app, ui, Some(i)));
    }
    if app.prefs.tree_item("Matching") {
        let m = tree_row(ui, 1, "Matching", None, app.view == View::Matching, TreeIcon::Playlist, 15.0);
        let m = if app.prefs.tooltips { m.on_hover_text("Show tracks based on good match combinations.") } else { m };
        if m.clicked() {
            app.set_view(View::Matching);
        }
    }
    ui.add_space(6.0);
    let src = app.related_source();
    let src_name = src.and_then(|id| app.track(id)).map(|t| t.display_title());
    if app.view == View::Matching {
        ui.label(RichText::new("Matching").size(12.0).color(WHITE));
        ui.label(RichText::new(match &src_name {
            Some(n) => format!("Good matches of \"{n}\"."),
            None => "Load or select a track to see its good matches.".to_string(),
        }).size(11.0).color(DIM2));
        ui.add_space(4.0);
        // Link the selected track(s) with the source track.
        let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t) && Some(*t) != src).collect();
        if let (Some(s), false) = (src, sel.is_empty()) {
            if ui.button(RichText::new("Remove the selected match").size(11.5)).clicked() {
                for t in &sel {
                    app.matching_set(s, *t, false);
                }
            }
        }
        ui.add_space(4.0);
        ui.label(RichText::new("To register a match: load a track, right-click another track and choose Matching, or drag tracks onto this panel.").size(10.5).color(DIM));
        return;
    }
    // Criteria.
    let open = app.ext.criteria_open;
    let cr = ui.add(egui::Button::new(RichText::new("Criteria").size(11.5).color(WHITE)).fill(if open { BLUE } else { BTN }));
    let cr = if app.prefs.tooltips { cr.on_hover_text("Customize the criteria") } else { cr };
    if cr.clicked() {
        app.ext.criteria_open = !open;
    }
    if !app.ext.criteria_open {
        ui.add_space(4.0);
        ui.label(RichText::new(match &src_name {
            Some(n) => format!("\"Related tracks\" of \"{n}\" is displayed."),
            None => "Load or select a track.".to_string(),
        }).size(10.5).color(DIM));
        return;
    }
    related_criteria(app, ui);
}

pub const INFO_TAB_FONT: f32 = 14.25;
pub const INFO_LABEL_FONT: f32 = 12.5;
pub const INFO_ROW_FONT: f32 = 14.0;
pub const INFO_SMALL_FONT: f32 = 11.5;
pub const INFO_VALUE_FONT: f32 = 13.5;

/// The criteria of the selected "Related Tracks" list.
pub fn related_criteria(app: &mut App, ui: &mut Ui) {
    let before = app.prefs.related.clone();
    let r = &mut app.prefs.related;
    ui.add_space(2.0);
    ui.label(RichText::new("Key").size(12.0).color(WHITE));
    egui::ComboBox::from_id_salt("rel_key").width(180.0).selected_text(["Off", "Same Key", "Related key 1", "Related key 2", "Related key 3"][r.key as usize % 5]).show_ui(ui, |ui| {
        for (i, n) in ["Off", "Same Key", "Related key 1", "Related key 2", "Related key 3"].iter().enumerate() {
            ui.selectable_value(&mut r.key, i as u8, *n);
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut r.bpm, RichText::new("BPM within ±").size(12.0));
        ui.add(egui::DragValue::new(&mut r.bpm_pct).range(1.0..=50.0).speed(0.5).suffix("%"));
    });
    ui.label(RichText::new("Including double/half").size(10.5).color(DIM));
    ui.checkbox(&mut r.genre, RichText::new("Genre").size(12.0));
    ui.checkbox(&mut r.artist, RichText::new("Same artist").size(12.0));
    ui.checkbox(&mut r.label, RichText::new("Label").size(12.0));
    ui.checkbox(&mut r.color, RichText::new("Color").size(12.0));
    ui.checkbox(&mut r.rating, RichText::new("Rating at least as high").size(12.0));
    ui.checkbox(&mut r.matching, RichText::new("Matching").size(12.0)).on_hover_text("Set a combination of tracks that match well.\nMatching tracks can be shown in \"RELATED TRACKS\".\nNote: Select [Matching] as a filter in the [Criteria].");
    if app.prefs.related != before {
        app.related_store();
        app.list_dirty = true;
        app.save_prefs();
    }
    if app.match_enrichment_running() && ui.button("View lookup progress…").clicked() {
        crate::ui_dialogs::open_match_lookup(app, &[]);
    }
}

fn related_menu(app: &mut App, ui: &mut Ui, i: Option<usize>) {
    if ui.button("Create New \"Related Tracks\"").clicked() {
        app.related_new(false);
        app.set_view(View::Related);
        ui.close_menu();
    }
    let Some(i) = i else { return };
    if ui.button("Copy the criteria to create new \"Related Tracks\"").clicked() {
        app.related_select(i);
        app.related_new(true);
        app.set_view(View::Related);
        ui.close_menu();
    }
    if ui.button("Turn the \"Related Tracks\" into a playlist").clicked() {
        app.related_select(i);
        if let Some(pl) = app.related_to_playlist() {
            app.jump_to_playlist(pl);
        }
        ui.close_menu();
    }
    if ui.button("Rename").clicked() {
        app.ext.related_rename = Some((i, app.prefs.related_lists[i].name.clone()));
        ui.close_menu();
    }
    if ui.add_enabled(app.prefs.related_lists.len() > 1, egui::Button::new("Delete \"Related Tracks\"")).clicked() {
        app.related_delete(i);
        ui.close_menu();
    }
}

// ------------------------------------------------------------------------------------ sources
fn source_name(kind: u8) -> &'static str {
    if kind == SRC_ITUNES { "iTunes" } else { "rekordbox xml" }
}

/// Tree of an outside library: All Tracks and its playlists.
pub fn source_tree(app: &mut App, ui: &mut Ui, kind: u8) {
    let k = kind as usize % 2;
    ui.add_space(1.0);
    let head = tree_row(ui, 0, source_name(kind), None, false, TreeIcon::Open, 15.0);
    // Reload button at the right end of the heading.
    let rr = Rect::from_center_size(pos2(head.rect.max.x - 12.0, head.rect.center().y), vec2(16.0, 16.0));
    let rb = ui.interact(rr, ui.id().with(("src_reload", kind)), Sense::click());
    crate::icons::sync(ui.painter(), rr.shrink(1.0), crate::ui_feel::hl(&rb, DIM2.c(), WHITE.c()));
    let rb = if app.prefs.tooltips { rb.on_hover_text(if kind == SRC_ITUNES { "Reload:\nReload iTunes library." } else { "Reload" }) } else { rb };
    if rb.clicked() {
        app.source_reload(kind);
    }
    head.context_menu(|ui| {
        if ui.button(if kind == SRC_ITUNES { "Reload iTunes Library" } else { "Reload" }).clicked() {
            app.source_reload(kind);
            ui.close_menu();
        }
        if ui.button(if kind == SRC_ITUNES { "iTunes Library File…" } else { "rekordbox xml file…" }).clicked() {
            ui.close_menu();
            choose_source_file(app, kind);
        }
    });
    let loading = app.ext.sources[k].rx.is_some();
    let lib = app.ext.sources[k].lib.clone();
    let error = app.ext.sources[k].error.clone();
    if loading {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(if kind == SRC_ITUNES { "Loading iTunes Library." } else { "Loading XML library" }).size(11.5).color(DIM2));
        });
        return;
    }
    let Some(lib) = lib else {
        ui.add_space(8.0);
        if let Some(e) = error {
            ui.label(RichText::new(e).size(11.0).color(CAUTION_TX));
            ui.add_space(4.0);
        }
        // Load on its own when there is a file to read; asking the Music app waits for a click.
        if kind == SRC_ITUNES {
            let has_file = crate::itunes::itunes_xml_candidates(app.prefs.itunes_xml.as_deref()).iter().any(|p| p.is_file());
            if has_file && app.ext.sources[k].error.is_none() {
                app.source_reload(kind);
            }
            if ui.button(RichText::new("Load iTunes").size(11.5)).on_hover_text("Reads the Music app's library and playlists. Nothing in the Music library is changed.").clicked() {
                app.source_reload(kind);
            }
            if ui.button(RichText::new("iTunes Library File…").size(11.5)).clicked() {
                choose_source_file(app, kind);
            }
        } else {
            if app.prefs.rekordbox_xml_path.as_deref().map(|p| !p.is_empty()).unwrap_or(false) && app.ext.sources[k].error.is_none() {
                app.source_reload(kind);
            }
            if ui.button(RichText::new("rekordbox xml file…").size(11.5)).clicked() {
                choose_source_file(app, kind);
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Choose a collection exported in xml format to browse its tracks and playlists here.").size(10.5).color(DIM));
        }
        return;
    };
    egui::ScrollArea::vertical().id_salt(("src_tree", kind)).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let all = View::Source { kind, node: String::new() };
        if tree_row(ui, 0, "All Tracks", Some(lib.tracks.len()), app.view == all, TreeIcon::Note, 15.0).clicked() {
            app.ext.sources[k].sel.clear();
            app.set_view(all);
        }
        tree_row(ui, 0, "Playlists", None, false, TreeIcon::Open, 15.0);
        fn rows(app: &mut App, ui: &mut Ui, kind: u8, nodes: &[crate::itunes::SrcNode], prefix: &str, depth: usize) {
            let k = kind as usize % 2;
            for (i, n) in nodes.iter().enumerate() {
                let key = if prefix.is_empty() { i.to_string() } else { format!("{prefix}/{i}") };
                let view = View::Source { kind, node: key.clone() };
                let open = app.ext.sources[k].open.contains(&key);
                let icon = if n.folder { if open { TreeIcon::FolderOpen } else { TreeIcon::FolderClosed } } else { TreeIcon::Playlist };
                let r = tree_row(ui, depth, &n.name, (!n.folder).then_some(n.tracks.len()), app.view == view, icon, 15.0);
                if r.clicked() {
                    if n.folder {
                        if open {
                            app.ext.sources[k].open.remove(&key);
                        } else {
                            app.ext.sources[k].open.insert(key.clone());
                        }
                    }
                    app.ext.sources[k].sel.clear();
                    app.set_view(view);
                }
                let key2 = key.clone();
                r.context_menu(|ui| {
                    if ui.button("Import Playlist").on_hover_text("Import as a playlist").clicked() {
                        if let Some(id) = app.source_import_node(kind, &key2) {
                            app.jump_to_playlist(id);
                        }
                        ui.close_menu();
                    }
                });
                if n.folder && open && depth < 20 {
                    rows(app, ui, kind, &n.children, &key, depth + 1);
                }
            }
        }
        rows(app, ui, kind, &lib.nodes, "", 1);
    });
}

fn choose_source_file(app: &mut App, kind: u8) {
    let title = if kind == SRC_ITUNES { "Choose a new iTunes library file" } else { "rekordbox xml file" };
    if let Some(f) = rfd::FileDialog::new().set_title(title).add_filter("XML", &["xml"]).pick_file() {
        let s = f.to_string_lossy().to_string();
        if kind == SRC_ITUNES {
            app.prefs.itunes_xml = Some(s);
        } else {
            app.prefs.rekordbox_xml_path = Some(s);
        }
        app.save_prefs();
        app.ext.sources[kind as usize % 2].error = None;
        app.source_reload(kind);
    }
}

/// Track list of an outside library. Double-click imports (and loads) a track; the menu
/// imports the selection.
pub fn source_list(app: &mut App, ui: &mut Ui, kind: u8, node: &str) {
    let k = kind as usize % 2;
    let r = ui.max_rect();
    let bar = Rect::from_min_max(pos2(r.min.x, r.min.y + 5.0), pos2(r.max.x, r.min.y + 27.0));
    ui.painter().rect_filled(bar, 0.0, BG2);
    let Some(lib) = app.ext.sources[k].lib.clone() else {
        ui.painter().text(pos2(bar.min.x + 10.5, bar.center().y), Align2::LEFT_CENTER, source_name(kind), FontId::proportional(11.5), WHITE.c());
        return;
    };
    let rows = lib.tracks_of(node);
    let title = lib.node(node).map(|n| n.name.clone()).unwrap_or_else(|| "All Tracks".into());
    ui.painter().text(pos2(bar.min.x + 10.5, bar.center().y), Align2::LEFT_CENTER, format!("{} › {title} ({} Tracks)", source_name(kind), rows.len()), FontId::proportional(11.5), WHITE.c());
    // Import button on the right.
    let n_sel = app.ext.sources[k].sel.len();
    let ib = Rect::from_min_max(pos2(bar.max.x - 190.0, bar.min.y + 2.0), pos2(bar.max.x - 8.0, bar.max.y - 2.0));
    if !rows.is_empty() && ui.put(ib, egui::Button::new(RichText::new(if n_sel > 0 { format!("Import To Collection ({n_sel})") } else { "Import To Collection".to_string() }).size(11.0))).clicked() {
        let idx: Vec<usize> = if n_sel > 0 { rows.iter().copied().filter(|i| app.ext.sources[k].sel.contains(i)).collect() } else { rows.clone() };
        app.source_import_tracks(kind, &idx);
    }
    let in_col: HashSet<&str> = app.tracks.iter().map(|t| t.path.as_str()).collect();
    let present: Vec<bool> = rows.iter().map(|i| in_col.contains(lib.tracks[*i].path.as_str())).collect();
    drop(in_col);
    let body = Rect::from_min_max(pos2(r.min.x, bar.max.y + 4.0), r.max);
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    bui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let flex = (body.width() - 30.0 - 70.0 - 60.0 - 120.0 - 16.0).max(300.0);
    let cols: [(&str, f32, bool); 7] = [("", 30.0, false), ("Track Title", flex * 0.30, false), ("Artist", flex * 0.22, false), ("Album", flex * 0.20, false), ("Genre", 120.0, false), ("BPM", 70.0, true), ("Time", 60.0, true)];
    let loc_w = flex * 0.28;
    let (hr, _) = bui.allocate_exact_size(vec2(bui.available_width(), 13.5), Sense::hover());
    bui.painter().rect_filled(hr, 0.0, BG2);
    let mut x = hr.min.x;
    for (c, w, _) in cols.iter().chain([("Location", loc_w, false)].iter()) {
        bui.painter().text(pos2(x + 5.0, hr.center().y), Align2::LEFT_CENTER, *c, FontId::proportional(11.0), WHITE.c());
        bui.painter().line_segment([pos2(x + w - 0.5, hr.min.y + 1.0), pos2(x + w - 0.5, hr.max.y - 1.0)], Stroke::new(1.0, BG));
        x += w;
    }
    if rows.is_empty() {
        return;
    }
    let font = list_font(app);
    egui::ScrollArea::vertical().id_salt(("src_rows", kind)).auto_shrink([false, false]).show_rows(&mut bui, ROW_H, rows.len(), |ui, range| {
        for ri in range {
            let ti = rows[ri];
            let t = &lib.tracks[ti];
            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            let sel = app.ext.sources[k].sel.contains(&ti);
            // (A selected row keeps rekordbox's fixed blue and white text in Light: `ListViewer::findTextColour`.)
            let _sel_dark = crate::theme::dark_scope(sel);
            ui.painter().rect_filled(rr, 0.0, if sel { ROW_SEL } else if ri % 2 == 0 { BG1 } else { BG2 });
            let have = present[ri];
            if have {
                // "Track imported to the Collection": a tick.
                let c = pos2(rr.min.x + 15.0, rr.center().y);
                ui.painter().add(egui::Shape::line(vec![c + vec2(-4.0, 0.0), c + vec2(-1.0, 3.0), c + vec2(4.5, -3.5)], Stroke::new(1.6, WHITE)));
            }
            let vals = [String::new(), t.title.clone(), t.artist.clone(), t.album.clone(), t.genre.clone(), if t.bpm > 0.0 { format!("{:.2}", t.bpm) } else { String::new() }, if t.duration > 0.0 { fmt_time(t.duration, false) } else { String::new() }, t.path.clone()];
            let mut x = rr.min.x;
            for (v, (_, w, right)) in vals.iter().zip(cols.iter().chain([("Location", loc_w, false)].iter())) {
                let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(*w, rr.height()));
                let p = ui.painter().with_clip_rect(cr.shrink2(vec2(3.0, 0.0)));
                if *right {
                    p.text(pos2(cr.max.x - 6.0, cr.center().y), Align2::RIGHT_CENTER, v, font.clone(), TEXT.c());
                } else {
                    p.text(pos2(cr.min.x + 6.0, cr.center().y), Align2::LEFT_CENTER, v, font.clone(), TEXT.c());
                }
                x += w;
            }
            drop(_sel_dark);
            if resp.clicked() || (resp.secondary_clicked() && !sel) {
                let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                let s = &mut app.ext.sources[k];
                if shift && s.anchor.is_some() {
                    let a = s.anchor.unwrap().min(rows.len() - 1);
                    s.sel = rows[a.min(ri)..=a.max(ri)].iter().copied().collect();
                } else if cmd {
                    if !s.sel.insert(ti) {
                        s.sel.remove(&ti);
                    }
                    s.anchor = Some(ri);
                } else {
                    s.sel = [ti].into_iter().collect();
                    s.anchor = Some(ri);
                }
            }
            if resp.double_clicked() && !app.deck_options.disable_load {
                match app.db.track_by_path(&t.path) {
                    Some(id) => app.load_track(id),
                    None => {
                        let ids = app.source_import_tracks(kind, &[ti]);
                        match ids.first() {
                            Some(id) => app.load_track(*id),
                            None => app.load_after_import = Some(t.path.clone()),
                        }
                    }
                }
            }
            resp.context_menu(|ui| {
                if ui.button("Import To Collection").clicked() {
                    let idx: Vec<usize> = rows.iter().copied().filter(|i| app.ext.sources[k].sel.contains(i)).collect();
                    app.source_import_tracks(kind, &idx);
                    ui.close_menu();
                }
                if ui.button(crate::ui_browser::REVEAL_FILE).clicked() {
                    if let Err(err) = crate::ui_browser::reveal_path(std::path::Path::new(&t.path)) {
                        app.toast_err(format!("Could not open the file location: {err}"));
                    }
                    ui.close_menu();
                }
            });
            if have && app.prefs.tooltips && resp.hovered() {
                resp.on_hover_text("Track exists in the collection");
            }
        }
    });
}

// ------------------------------------------------------------------------------------ information window
fn artwork_large(app: &mut App, path: &str) -> Option<egui::TextureHandle> {
    if path.is_empty() {
        return None;
    }
    let key = format!("large:{path}");
    if let Some(t) = app.textures.get(&key) {
        return t.clone();
    }
    let tex = image::open(path).ok().map(|img| {
        let small = img.thumbnail(480, 480).to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], &small);
        app.ctx.load_texture(&key, ci, egui::TextureOptions::LINEAR)
    });
    app.textures.insert(key, tex.clone());
    tex
}

/// "Add Artwork": copy a chosen picture into the library's artwork folder for these tracks.
/// The audio files are not touched.
pub fn add_artwork(app: &mut App, ids: &[i64]) {
    let Some(f) = rfd::FileDialog::new().set_title("Select an artwork").add_filter("Image", &["jpg", "jpeg", "png"]).pick_file() else { return };
    for id in ids {
        let dest = crate::db::artwork_dir().join(format!("custom_{}.{}", id, f.extension().and_then(|e| e.to_str()).unwrap_or("jpg")));
        if std::fs::copy(&f, &dest).is_ok() {
            let tag = format!("custom_{id}.");
            app.textures.retain(|k, _| !k.contains(&tag));
            let new_path = dest.to_string_lossy().into_owned();
            app.list_artwork.forget(&new_path);
            if let Some(t) = app.track_mut(*id) {
                let old_path = std::mem::replace(&mut t.artwork, new_path);
                if !old_path.is_empty() && old_path != t.artwork {
                    app.list_artwork.forget(&old_path);
                }
            }
            app.save_track(*id);
        } else {
            app.toast_err("Artwork File Not Found");
        }
    }
}

fn common_text(app: &App, ids: &[i64], col: Col) -> Option<String> {
    let alnum = app.prefs.key_alnum;
    let mut it = ids.iter().filter_map(|id| app.track(*id)).map(|t| edit_text(t, col, alnum));
    let first = it.next()?;
    it.all(|s| s == first).then_some(first)
}

/// One editable field of the Information window. With several tracks the field is blank when
/// their values differ; what is typed is applied to all of them.
fn info_field(app: &mut App, ui: &mut Ui, label: &str, col: Col, ids: &[i64], width: f32) {
    ui.label(RichText::new(label).size(11.0).color(DIM2));
    let common = common_text(app, ids, col);
    let editing = matches!(&app.ext.info_edit, Some((c, _)) if *c == col);
    let mut s = if editing { app.ext.info_edit.as_ref().unwrap().1.clone() } else { common.clone().unwrap_or_default() };
    let mut te = egui::TextEdit::singleline(&mut s).desired_width(width).font(FontId::proportional(11.5)).margin(egui::Margin::symmetric(4, 1));
    if common.is_none() && ids.len() > 1 {
        te = te.hint_text(RichText::new("").color(DIM));
    }
    let resp = ui.add(te);
    if resp.has_focus() || resp.changed() {
        // Focus moved here from another field that was still being edited: that one is applied
        // first.
        if let Some((c, text)) = app.ext.info_edit.take() {
            if c != col {
                commit_info(app, ids, c, &text);
            }
        }
        app.ext.info_edit = Some((col, s.clone()));
    }
    if resp.lost_focus() && editing {
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if let Some((c, text)) = app.ext.info_edit.take() {
            if c == col && !esc {
                commit_info(app, ids, c, &text);
            } else if c != col {
                app.ext.info_edit = Some((c, text));
            }
        }
    }
    ui.end_row();
}

fn commit_info(app: &mut App, ids: &[i64], col: Col, text: &str) {
    let common = common_text(app, ids, col);
    let unchanged = common.as_deref() == Some(text.trim()) || common.as_deref() == Some(text) || (common.is_none() && text.is_empty());
    if !unchanged {
        app.edit_field(ids, col, text);
    }
}

fn info_readonly(ui: &mut Ui, label: &str, value: String) {
    ui.label(RichText::new(label).size(11.0).color(DIM2));
    ui.add(egui::Label::new(RichText::new(value).size(11.5).color(TEXT)).truncate());
    ui.end_row();
}

fn common_of<T: PartialEq + Clone>(app: &App, ids: &[i64], f: impl Fn(&Track) -> T) -> Option<T> {
    let mut it = ids.iter().filter_map(|id| app.track(*id)).map(f);
    let first = it.next()?;
    it.all(|s| s == first).then_some(first)
}

/// A value of the Summary page that can be typed over: click to edit, Return or a click
/// elsewhere applies it to the shown track(s), Esc leaves it.
fn summary_value(app: &mut App, ui: &mut Ui, r: Rect, col: Col, ids: &[i64], font: FontId) {
    let common = common_text(app, ids, col);
    let editing = matches!(&app.ext.info_edit, Some((c, _)) if *c == col);
    if editing {
        let mut s = app.ext.info_edit.as_ref().map(|x| x.1.clone()).unwrap_or_default();
        let resp = ui.put(r, egui::TextEdit::singleline(&mut s).font(font).margin(egui::Margin::symmetric(2, 0)));
        // Whether the field just gave up focus must be read before focus is asked for again.
        let gave_up = resp.lost_focus();
        if !gave_up {
            resp.request_focus();
            app.ext.info_edit = Some((col, s));
        } else {
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            app.ext.info_edit = None;
            if !esc {
                commit_info(app, ids, col, &s);
            }
        }
        return;
    }
    let resp = ui.interact(r, ui.id().with(("summary_value", col)), Sense::click());
    ui.painter().with_clip_rect(r.intersect(ui.clip_rect())).text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, common.clone().unwrap_or_default(), font, WHITE.c());
    if resp.clicked() && !ids.is_empty() {
        app.ext.info_edit = Some((col, common.unwrap_or_default()));
        ui.ctx().request_repaint();
    }
}

fn file_type_name(t: &Track) -> String {
    match t.file_type.to_lowercase().as_str() {
        "mp3" => "MP3 File".to_string(),
        "m4a" | "mp4" => "M4A File".to_string(),
        "wav" => "WAV File".to_string(),
        "aif" | "aiff" => "AIFF File".to_string(),
        "aac" => "AAC File".to_string(),
        "flac" => "FLAC File".to_string(),
        other => other.to_uppercase(),
    }
}

/// "2026-10-18…" as rekordbox shows a date: 18/10/2026.
fn date_dmy(s: &str) -> String {
    let d = s.get(..10).unwrap_or(s);
    let parts: Vec<&str> = d.split('-').collect();
    if parts.len() == 3 { format!("{}/{}/{}", parts[2], parts[1], parts[0]) } else { d.to_string() }
}

pub const INFO_LABEL: Tc = gr(0x828282);

/// Information window at the right of the browser (reference 13), 445 pt wide on #1E1E1E:
/// Summary / Info / Artwork tabs 21 pt high from 8.5 pt down (the chosen one blue with dark
/// text, the others #323232 with grey text) and the reload mark 17 pt from the right. Summary:
/// the artwork in a 111 pt box 9.5 pt in and 42 pt down; Track Title, Artist and Album 174 pt
/// in on a 39 pt pitch, each a grey label over its value with a #313131 line under it; then
/// 24 pt rows between #313131 lines from 168 pt down, labels at the left, values 174 pt in
/// behind a short divider. Shown for the selected track(s), or the track in the player.
pub fn info_panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    p.rect_filled(r, 0.0, BG2);
    let (l, t, right) = (r.min.x, r.min.y, r.max.x);
    let sel: Vec<i64> = app.list.iter().copied().filter(|t| app.selected.contains(t)).collect();
    let mut seen = HashSet::new();
    let sel: Vec<i64> = sel.into_iter().filter(|t| seen.insert(*t)).collect();
    let ids: Vec<i64> = if !sel.is_empty() { sel } else { app.deck.track.into_iter().collect() };
    // Tabs.
    let (x0, x1, gap) = (l + 13.5, right - 37.0, 3.75);
    let w = (x1 - x0 - 2.0 * gap) / 3.0;
    for (i, name) in ["Summary", "Info", "Artwork"].iter().enumerate() {
        let on = app.ext.info_tab == i as u8;
        let rr = Rect::from_min_size(pos2(x0 + i as f32 * (w + gap), t + 8.5), vec2(w, 21.0));
        let resp = ui.interact(rr, ui.id().with(("info_tab", i)), Sense::click());
        p.rect_filled(rr, 0.0, if on { BLUE.c() } else { crate::ui_feel::hl(&resp, gy(0x323232), gy(0x3A3A3A)) });
        p.text(rr.center(), Align2::CENTER_CENTER, *name, rb(INFO_TAB_FONT), if on { hex(0x00021E) } else { DIM.c() });
        if resp.clicked() {
            app.ext.info_tab = i as u8;
            app.ext.info_edit = None;
        }
    }
    // Reload Tags (skin brws_refresh).
    let rc = pos2(right - 17.0, t + 19.0);
    let rresp = ui.interact(Rect::from_center_size(rc, vec2(18.0, 20.0)), ui.id().with("info_reload"), Sense::click());
    crate::icons_src::refresh(&p, rc, crate::ui_feel::hl(&rresp, gy(0xDCDCDC), WHITE.c()));
    let rresp = if app.prefs.tooltips { rresp.on_hover_text("Reload Tags") } else { rresp };
    if rresp.clicked() && !ids.is_empty() {
        app.reload_tags_from_files(&ids);
    }
    let first = ids.first().and_then(|id| app.track(*id)).cloned().unwrap_or_default();
    let multi = ids.len() > 1;
    let art = common_of(app, &ids, |t| t.artwork.clone()).unwrap_or_default();
    let body = Rect::from_min_max(pos2(l + 3.0, t + 34.0), pos2(right, r.max.y - 3.0));
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("info_body"));
    bui.set_clip_rect(body.intersect(ui.clip_rect()));
    bui.spacing_mut().scroll = egui::style::ScrollStyle { floating: false, bar_width: 8.0, bar_inner_margin: 2.0, bar_outer_margin: 2.0, foreground_color: false, dormant_background_opacity: 1.0, active_background_opacity: 1.0, interact_background_opacity: 1.0, dormant_handle_opacity: 1.0, active_handle_opacity: 1.0, interact_handle_opacity: 1.0, ..egui::style::ScrollStyle::solid() };
    bui.visuals_mut().extreme_bg_color = FIELD.c();
    bui.visuals_mut().widgets.inactive.bg_fill = gy(0x323232);
    bui.visuals_mut().widgets.hovered.bg_fill = gy(0x4A4A4A);
    bui.visuals_mut().widgets.active.bg_fill = gy(0x5A5A5A);
    if app.ext.info_tab == 0 {
        let ro = |f: &dyn Fn(&Track) -> String| if ids.is_empty() { String::new() } else { common_of(app, &ids, |t| f(t)).unwrap_or_default() };
        let rows: [(&str, String); 8] = [
            ("Time", ro(&|t| if t.duration > 0.0 { fmt_time(t.duration, false) } else { String::new() })),
            ("File Type", ro(&|t| file_type_name(t))),
            ("Size", ro(&|t| if t.file_size > 0 { format!("{:.1} MB", t.file_size as f64 / 1048576.0) } else { String::new() })),
            ("Date Created", ro(&|t| date_dmy(&t.date_created))),
            ("Sample Rate", ro(&|t| if t.sample_rate > 0 { format!("{} Hz", t.sample_rate) } else { String::new() })),
            ("Bitrate", ro(&|t| if t.bitrate > 0 { format!("{} kbps", t.bitrate) } else { String::new() })),
            ("DJ Play Count", ro(&|t| t.play_count.to_string())),
            ("Location", ro(&|t| t.path.clone())),
        ];
        let font = rb(INFO_ROW_FONT);
        let widest = rows.iter().map(|(_, v)| ui.painter().layout_no_wrap(v.clone(), font.clone(), WHITE.c()).size().x).fold(0.0, f32::max);
        let content_w = (171.0 + widest + 8.0).max(body.width());
        egui::ScrollArea::horizontal().id_salt("info_summary_h").auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(&mut bui, |ui| {
            let (cr, _) = ui.allocate_exact_size(vec2(content_w, ui.available_height()), Sense::hover());
            let p = ui.painter().clone();
            // Coordinates below are from the window's corner as measured.
            let (l, t) = (cr.min.x - 3.0, cr.min.y - 34.0);
            let line_right = cr.max.x;
            let ar = Rect::from_min_size(pos2(l + 9.5, t + 42.0), vec2(111.0, 111.0));
            p.rect_filled(ar, 0.0, gy(0x101010));
            match artwork_large(app, &art) {
                Some(tex) => {
                    p.image(tex.id(), ar.shrink(1.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    // The original's "no artwork" record.
                    p.circle_filled(ar.center(), 48.5, gy(0x1E1E1E));
                    p.circle_filled(ar.center(), 14.0, gy(0x101010));
                }
            }
            p.rect_stroke(ar, 0.0, Stroke::new(1.0, gy(0x4C4C4C)), egui::StrokeKind::Inside);
            let vx = l + 174.0;
            for (i, (label, col)) in [("Track Title", Col::Title), ("Artist", Col::Artist), ("Album", Col::Album)].into_iter().enumerate() {
                let y = t + 49.5 + 39.0 * i as f32;
                p.text(pos2(vx, y), Align2::LEFT_CENTER, label, rb(INFO_SMALL_FONT), INFO_LABEL.c());
                summary_value(app, ui, Rect::from_min_max(pos2(vx, y + 8.5), pos2(line_right, y + 27.0)), col, &ids, rb(INFO_VALUE_FONT));
                if i < 2 {
                    p.rect_filled(Rect::from_min_max(pos2(vx, y + 29.0), pos2(line_right, y + 30.0)), 0.0, gy(0x313131));
                }
            }
            for (i, (label, value)) in rows.iter().enumerate() {
                let y = t + 168.0 + 24.0 * i as f32;
                p.rect_filled(Rect::from_min_max(pos2(l + 9.5, y), pos2(line_right, y + 1.0)), 0.0, gy(0x313131));
                p.rect_filled(Rect::from_min_max(pos2(vx - 4.5, y + 3.5), pos2(vx - 3.5, y + 21.5)), 0.0, gy(0x313131));
                p.text(pos2(l + 9.5, y + 12.5), Align2::LEFT_CENTER, *label, rb(INFO_LABEL_FONT), INFO_LABEL.c());
                p.text(pos2(vx, y + 12.5), Align2::LEFT_CENTER, value, font.clone(), WHITE.c());
            }
            let y = t + 168.0 + 24.0 * rows.len() as f32;
            p.rect_filled(Rect::from_min_max(pos2(l + 9.5, y), pos2(line_right, y + 1.0)), 0.0, gy(0x313131));
            if multi {
                p.text(pos2(l + 9.5, y + 14.0), Align2::LEFT_CENTER, format!("{} tracks selected", ids.len()), rb(11.0), DIM.c());
            }
        });
        return;
    }
    if ids.is_empty() {
        return;
    }
    let pad = Rect::from_min_max(pos2(body.min.x + 6.5, body.min.y + 6.0), pos2(body.max.x, body.max.y));
    let mut pui = bui.new_child(egui::UiBuilder::new().max_rect(pad));
    let ui = &mut pui;
    let field_w = (ui.available_width() - 130.0).max(80.0);
    egui::ScrollArea::vertical().id_salt("info_panel").auto_shrink([false, false]).show(ui, |ui| {
        match app.ext.info_tab {
            1 => {
                egui::Grid::new("info_fields").num_columns(2).spacing(vec2(6.0, 3.0)).show(ui, |ui| {
                    for (label, col) in [
                        ("Title", Col::Title),
                        ("Artist", Col::Artist),
                        ("Album", Col::Album),
                        ("Album Artist", Col::AlbumArtist),
                        ("Genre", Col::Genre),
                        ("Comments", Col::Comments),
                        ("Key", Col::Key),
                        ("BPM", Col::Bpm),
                        ("Label", Col::Label),
                        ("Remixer", Col::Remixer),
                        ("Original Artist", Col::OriginalArtist),
                        ("Composer", Col::Composer),
                        ("Lyricist", Col::Lyricist),
                        ("Mix Name", Col::MixName),
                        ("Grouping", Col::Grouping),
                        ("Year", Col::Year),
                        ("Release Date", Col::ReleaseDate),
                        ("Track number", Col::TrackNumber),
                        ("Disc number", Col::DiscNumber),
                        ("Message", Col::Message),
                    ] {
                        info_field(app, ui, label, col, &ids, field_w);
                    }
                    // Rating
                    ui.label(RichText::new("Rating").size(11.0).color(DIM2));
                    let rating = common_of(app, &ids, |t| t.rating);
                    let (sr, sresp) = ui.allocate_exact_size(vec2(5.0 * 15.5 + 4.0, 16.0), Sense::click());
                    for s in 1..=5u8 {
                        let on = rating.map(|r| r >= s).unwrap_or(false);
                        star(ui.painter(), pos2(sr.min.x + 8.0 + (s - 1) as f32 * 15.5, sr.center().y), 6.4, if on { WHITE.c() } else { DIM.c() }, on);
                    }
                    if sresp.clicked() {
                        if let Some(pp) = sresp.interact_pointer_pos() {
                            let s = (((pp.x - sr.min.x) / 15.5).floor() as i32 + 1).clamp(1, 5) as u8;
                            let v = if rating == Some(s) { 0 } else { s };
                            for id in &ids {
                                if let Some(t) = app.track_mut(*id) {
                                    t.rating = v;
                                }
                                app.save_track(*id);
                            }
                        }
                    }
                    ui.end_row();
                    // Colour
                    ui.label(RichText::new("Color").size(11.0).color(DIM2));
                    let color = common_of(app, &ids, |t| t.color);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for i in (1..=8u8).chain([0u8]) {
                            let (cr, cresp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
                            if i == 0 {
                                ui.painter().rect_stroke(cr.shrink(2.0), 1.0, Stroke::new(1.0, DIM), egui::StrokeKind::Inside);
                            } else {
                                ui.painter().rect_filled(cr.shrink(2.0), 1.0, label_color(i));
                            }
                            if color == Some(i) && i > 0 {
                                ui.painter().rect_stroke(cr, 2.0, Stroke::new(1.2, WHITE), egui::StrokeKind::Inside);
                            }
                            if cresp.on_hover_text(COLOR_NAMES[i as usize]).clicked() {
                                for id in &ids {
                                    if let Some(t) = app.track_mut(*id) {
                                        t.color = i;
                                    }
                                    app.save_track(*id);
                                }
                            }
                        }
                    });
                    ui.end_row();
                    // File fields: fixed.
                    let ro = |f: &dyn Fn(&Track) -> String| common_of(app, &ids, |t| f(t)).unwrap_or_default();
                    info_readonly(ui, "Duration", ro(&|t| if t.duration > 0.0 { fmt_time(t.duration, false) } else { String::new() }));
                    info_readonly(ui, "File Type", ro(&|t| file_type_name(t)));
                    info_readonly(ui, "Size", ro(&|t| format!("{:.1} MB", t.file_size as f64 / 1e6)));
                    info_readonly(ui, "Sample Rate", ro(&|t| if t.sample_rate > 0 { format!("{} Hz", t.sample_rate) } else { String::new() }));
                    info_readonly(ui, "Bitrate", ro(&|t| if t.bitrate > 0 { format!("{} kbps", t.bitrate) } else { String::new() }));
                    info_readonly(ui, "Bitdepth", ro(&|t| if t.bit_depth > 0 { format!("{} bit", t.bit_depth) } else { String::new() }));
                    info_readonly(ui, "File Name", ro(&|t| t.file_name()));
                    info_readonly(ui, "Location", ro(&|t| t.path.clone()));
                    info_readonly(ui, "Date Added", ro(&|t| t.date_added.get(..10).unwrap_or(&t.date_added).to_string()));
                    info_readonly(ui, "Date Created", ro(&|t| t.date_created.get(..10).unwrap_or(&t.date_created).to_string()));
                    info_readonly(ui, "DJ Play Count", ro(&|t| t.play_count.to_string()));
                });
                ui.add_space(4.0);
                let auto = common_of(app, &ids, |t| t.autoload_hotcue);
                let mut on = auto.unwrap_or(false);
                let cb = ui.add(egui::Checkbox::new(&mut on, RichText::new("Allow to auto load HotCue on CDJ/XDJ").size(11.5)).indeterminate(auto.is_none()));
                if cb.changed() {
                    for id in &ids {
                        if let Some(t) = app.track_mut(*id) {
                            t.autoload_hotcue = on;
                        }
                        app.save_track(*id);
                    }
                }
            }
            _ => {
                let size = (ui.available_width() - 4.0).min(250.0);
                let (ar, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
                ui.painter().rect_filled(ar, 0.0, BG);
                match artwork_large(app, &art) {
                    Some(tex) => {
                        let ts = tex.size_vec2();
                        let scale = (ar.width() / ts.x).min(ar.height() / ts.y);
                        ui.painter().image(tex.id(), Rect::from_center_size(ar.center(), ts * scale), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().text(ar.center(), Align2::CENTER_CENTER, if art.is_empty() { "No Artwork" } else { "Artwork File Not Found" }, FontId::proportional(11.5), DIM.c());
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Add Artwork").size(11.5)).clicked() {
                        add_artwork(app, &ids);
                    }
                    let any = ids.iter().any(|id| app.track(*id).map(|t| !t.artwork.is_empty()).unwrap_or(false));
                    if ui.add_enabled(any, egui::Button::new(RichText::new("Delete Artwork").size(11.5))).clicked() {
                        for id in &ids {
                            if let Some(t) = app.track_mut(*id) {
                                t.artwork.clear();
                            }
                            app.save_track(*id);
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Artwork is kept in the recordbox library; the music files are not changed.").size(10.5).color(DIM));
            }
        }
    });
}

// ------------------------------------------------------------------------------------ windows
/// Windows and questions of this module (called once per frame).
pub fn windows(app: &mut App, ctx: &egui::Context) {
    color_picker(app, ctx);
    missing_file_manager(app, ctx);
    crate::collection_cleanup::ui::show(app, ctx);
    // Duplicates being added to a playlist.
    if let Some((pl, ids)) = app.ext.dup_confirm.clone() {
        let name = app.node(pl).map(|n| n.name.clone()).unwrap_or_default();
        let mut answer: Option<Option<bool>> = None;
        kit::dialog(ctx, "Add To Playlist").id(egui::Id::new("dup_confirm")).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(400.0);
            kit::message(ui, "There are duplicates being added to the playlist.\nWould you like to add the duplicates or skip them?");
            let cur: HashSet<i64> = app.db.playlist_tracks(pl).unwrap_or_default().into_iter().collect();
            let dups = ids.iter().filter(|i| cur.contains(i)).count();
            kit::message_note(ui, &format!("\"{name}\": {dups} of {} track(s) are already in the playlist.", ids.len()));
            match kit::dialog_buttons(ui, &["Add", "Skip", "Cancel"]) {
                Some(0) => answer = Some(Some(true)),
                Some(1) => answer = Some(Some(false)),
                Some(_) => answer = Some(None),
                None => {}
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                answer = Some(None);
            }
        });
        match answer {
            Some(Some(add)) => app.resolve_duplicates(add),
            Some(None) => app.ext.dup_confirm = None,
            None => {}
        }
    }
    // After a manual relocate: look for the others next to it?
    if let Some(dir) = app.ext.reloc_offer.clone() {
        let mut answer = None;
        kit::dialog(ctx, "Relocate").id(egui::Id::new("reloc_offer")).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(420.0);
            // The original's sentence, with this application's name.
            kit::message(ui, "Would you like recordbox to find other missing file using the location of this track ?");
            kit::message_note(ui, &dir.to_string_lossy());
            match kit::dialog_buttons(ui, &["Yes", "No"]) {
                Some(0) => answer = Some(true),
                Some(_) => answer = Some(false),
                None => {}
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                answer = Some(false);
            }
        });
        if let Some(yes) = answer {
            app.ext.reloc_offer = None;
            if yes {
                app.relocate_missing(&dir);
            }
        }
    }
    // Delete History / Clear History.
    if let Some(which) = app.ext.history_confirm {
        let mut answer = None;
        kit::dialog(ctx, if which.is_some() { "Delete History" } else { "Clear History" }).id(egui::Id::new("history_confirm")).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(360.0);
            match which {
                Some(id) => {
                    let name = app.ext.histories.iter().find(|h| h.id == id).map(|h| h.name.clone()).unwrap_or_default();
                    kit::message(ui, "Are you sure you want to delete this list?");
                    kit::message_note(ui, &name);
                }
                None => kit::message(ui, "Are you sure you want to delete all the play histories?"),
            }
            match kit::dialog_buttons(ui, &["OK", "Cancel"]) {
                Some(0) => answer = Some(true),
                Some(_) => answer = Some(false),
                None => {}
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                answer = Some(false);
            }
        });
        if let Some(yes) = answer {
            app.ext.history_confirm = None;
            if yes {
                match which {
                    Some(id) => app.history_delete(id),
                    None => app.history_clear_all(),
                }
            }
        }
    }
}

/// Missing File Manager: every track whose file is gone, with Relocate / Auto Relocate /
/// review selected references for cleanup, and the Auto Relocate Search Folders.
/// Missing File Manager (reference 16): a 602 x 528 pt window on #1E1E1E. A black table from
/// 4 pt in (Track Title, artist, album in 120 pt columns, then location; 27 pt header with grey
/// labels, 25 pt rows, the chosen ones #294C6A) with its scroll bar along the bottom; the
/// number of tracks at the right under it; then Auto Relocate, Relocate and Review cleanup (108 x 28 pt
/// from 15 pt in, 15.5 pt apart) and OK at the right. The buttons act on the chosen rows
/// (Auto Relocate on all of them when none is chosen); the search folders are in the table's
/// right-click menu.
fn missing_file_manager(app: &mut App, ctx: &egui::Context) {
    if !app.ext.missing_open {
        return;
    }
    let mut open = true;
    let missing: Vec<Track> = app.tracks.iter().filter(|t| t.missing).cloned().collect();
    let ids: HashSet<i64> = missing.iter().map(|t| t.id).collect();
    app.ext.missing_sel.retain(|t| ids.contains(t));
    let busy = app.relocate_rx.is_some();
    kit::dialog(ctx, "Missing File Manager").open(&mut open).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).frame(egui::Frame::window(&ctx.style()).fill(BG2.c()).inner_margin(egui::Margin::ZERO)).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(602.0, 528.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect_filled(r, 0.0, BG2.c());
        let table = Rect::from_min_max(pos2(r.min.x + 4.0, r.min.y + 4.5), pos2(r.max.x - 4.0, r.min.y + 453.5));
        p.rect_filled(table, 0.0, BG);
        let font = rb(MFM_FONT);
        let loc_w = missing.iter().map(|t| p.layout_no_wrap(t.path.clone(), font.clone(), WHITE.c()).size().x).fold(0.0, f32::max);
        let content_w = (360.0 + loc_w + 16.0).max(table.width());
        let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(table).id_salt("mfm_table"));
        tui.set_clip_rect(table);
        crate::ui_browser_ext::bar_style(&mut tui);
        tui.spacing_mut().scroll.bar_width = 4.0;
        tui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let cmd = ui.input(|i| i.modifiers.command);
        let bg = tui.interact(table, tui.id().with("mfm_bg"), Sense::click());
        egui::ScrollArea::horizontal().id_salt("mfm_h").auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(&mut tui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let cols: [(&str, f32); 4] = [("Track Title", 120.0), ("artist", 120.0), ("album", 120.0), ("location", content_w - 360.0)];
            let (hr, _) = ui.allocate_exact_size(vec2(content_w, 27.0), Sense::hover());
            ui.painter().rect_filled(hr, 0.0, gy(0x1A1A1A));
            let mut x = hr.min.x;
            for (i, (label, cw)) in cols.iter().enumerate() {
                if i > 0 {
                    ui.painter().rect_filled(Rect::from_min_max(pos2(x - 1.0, hr.min.y), pos2(x, hr.max.y)), 0.0, BG);
                }
                ui.painter().text(pos2(x + 7.0, hr.center().y), Align2::LEFT_CENTER, *label, font.clone(), DIM.c());
                x += cw;
            }
            egui::ScrollArea::vertical().id_salt("mfm_rows").auto_shrink([false, false]).show_rows(ui, 25.0, missing.len(), |ui, range| {
                for i in range {
                    let t = &missing[i];
                    let (rr, resp) = ui.allocate_exact_size(vec2(content_w, 25.0), Sense::click());
                    let on = app.ext.missing_sel.contains(&t.id);
                    // (A selected row keeps rekordbox's fixed blue and white text in Light: `ListViewer::findTextColour`.)
                    let _sel_dark = crate::theme::dark_scope(on);
                    if on {
                        ui.painter().rect_filled(Rect::from_min_max(pos2(rr.min.x, rr.min.y + 0.5), rr.max), 0.0, ROW_SEL);
                    } else if let Some(c) = crate::ui_feel::hover_fill(&resp, BG1) {
                        ui.painter().rect_filled(rr, 0.0, c);
                    }
                    let mut x = rr.min.x;
                    for (k, (v, (_, cw))) in [t.display_title(), t.artist.clone(), t.album.clone(), t.path.clone()].into_iter().zip(cols.iter()).enumerate() {
                        if k > 0 && on {
                            ui.painter().rect_filled(Rect::from_min_max(pos2(x - 1.0, rr.min.y), pos2(x, rr.max.y)), 0.0, hex(0x1F3A52));
                        }
                        let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(*cw - 2.0, rr.height()));
                        ui.painter().with_clip_rect(cr.intersect(ui.clip_rect())).text(pos2(cr.min.x + 5.0, cr.center().y), Align2::LEFT_CENTER, v, font.clone(), WHITE.c());
                        x += cw;
                    }
                    drop(_sel_dark);
                    if resp.clicked() {
                        if cmd {
                            if !app.ext.missing_sel.remove(&t.id) {
                                app.ext.missing_sel.insert(t.id);
                            }
                        } else {
                            app.ext.missing_sel.clear();
                            app.ext.missing_sel.insert(t.id);
                        }
                    }
                    let id = t.id;
                    resp.context_menu(|ui| {
                        if !app.ext.missing_sel.contains(&id) {
                            app.ext.missing_sel.clear();
                            app.ext.missing_sel.insert(id);
                        }
                        if ui.button("Relocate").clicked() {
                            ui.close_menu();
                            if let Some(f) = rfd::FileDialog::new().set_title("Relocate").pick_file() {
                                app.relocate_to(id, &f);
                            }
                        }
                        if ui.button("Auto Relocate").clicked() {
                            app.auto_relocate(Some(vec![id]));
                            ui.close_menu();
                        }
                        ui.separator();
                        search_folders_menu(app, ui);
                    });
                }
            });
        });
        bg.context_menu(|ui| search_folders_menu(app, ui));
        // Count, and what is going on.
        let n = missing.len();
        p.text(pos2(r.max.x - 15.5, r.min.y + 473.0), Align2::RIGHT_CENTER, format!("{n} Track{}", if n == 1 { "" } else { "s" }), font.clone(), gy(0xDCDCDC));
        if busy {
            p.text(pos2(r.min.x + 15.0, r.min.y + 473.0), Align2::LEFT_CENTER, "Searching…", font.clone(), DIM.c());
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        let picked: Vec<i64> = missing.iter().map(|t| t.id).filter(|t| app.ext.missing_sel.contains(t)).collect();
        let all: Vec<i64> = missing.iter().map(|t| t.id).collect();
        let target = if picked.is_empty() { all } else { picked.clone() };
        for (k, (label, x)) in [("Auto Relocate", 15.0), ("Relocate", 138.5), ("Review cleanup…", 262.5), ("OK", 474.5)].into_iter().enumerate() {
            let br = Rect::from_min_size(pos2(r.min.x + x, r.min.y + 489.0), vec2(108.5, 28.0));
            let enabled = match k {
                0 => !target.is_empty() && !busy,
                1 => picked.len() == 1,
                2 => !picked.is_empty(),
                _ => true,
            };
            let resp = ui.interact(br, ui.id().with(("mfm_btn", k)), if enabled { Sense::click() } else { Sense::hover() });
            p.rect_filled(br, 2.0, gy(0x141414));
            p.rect_filled(br.shrink(1.0), 1.5, if enabled && resp.is_pointer_button_down_on() { gy(0x2E2E2E) } else if enabled && resp.hovered() { gy(0x4A4A4A) } else { gy(0x3C3C3C) });
            p.text(br.center(), Align2::CENTER_CENTER, label, font.clone(), if enabled { gy(0xC8C8C8) } else { gy(0x787878) });
            if resp.clicked() {
                match k {
                    0 => app.auto_relocate(Some(target.clone())),
                    1 => {
                        if let Some(f) = rfd::FileDialog::new().set_title("Relocate").pick_file() {
                            app.relocate_to(picked[0], &f);
                        }
                    }
                    2 => {
                        app.ext.missing_open = false;
                        crate::collection_cleanup::ui::open(app, Some(picked.iter().copied().collect()));
                    }
                    _ => app.ext.missing_open = false,
                }
            }
        }
    });
    if !open {
        app.ext.missing_open = false;
    }
}

pub const MFM_FONT: f32 = 14.0;

/// Folders Auto Relocate looks in.
fn search_folders_menu(app: &mut App, ui: &mut Ui) {
    ui.menu_button("Auto Relocate Search Folders", |ui| {
        let folders = app.prefs.relocate_folders.clone();
        if folders.is_empty() {
            ui.label(RichText::new("none").size(11.0).color(DIM));
        }
        for (i, f) in folders.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("Remove").clicked() {
                    app.prefs.relocate_folders.remove(i);
                    app.save_prefs();
                }
                ui.label(RichText::new(f).size(11.0).color(if std::path::Path::new(f).is_dir() { TEXT.c() } else { CAUTION_TX }));
            });
        }
        if ui.button("Add Folder…").clicked() {
            ui.close_menu();
            if let Some(d) = rfd::FileDialog::new().set_title("Auto Relocate Search Folders").pick_folder() {
                let s = d.to_string_lossy().to_string();
                if !app.prefs.relocate_folders.contains(&s) {
                    app.prefs.relocate_folders.push(s);
                    app.save_prefs();
                }
            }
        }
    });
}
