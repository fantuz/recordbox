//! Review tracks with the same nonempty title and artist (exact, case-sensitive metadata).
//! Membership in a group is only a search result. The collection cleanup review independently
//! checks file identity, drive availability, provenance and protected metadata before any write.

use crate::app::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_kit as kit;
use eframe::egui;
use egui::{pos2, vec2, Align2, Rect, Sense};
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub struct Dupes {
    /// Result of the last search: the duplicated tracks in list order with their group.
    pub rows: Vec<(i64, usize)>,
    pub open: bool,
    pub sel: BTreeSet<i64>,
    anchor: Option<usize>,
    /// Where each listed track is ("Playlists / ..."), read when the list is made.
    pub paths: HashMap<i64, Vec<String>>,
    /// The path list dropped from a row's last cell: the track and where it opens.
    pub path_popup: Option<(i64, egui::Pos2)>,
}

/// Groups of duplicated tracks: same title (not empty) and same artist, in the order of
/// rekordbox's search (sorted by title, then artist; tracks that compare equal keep their
/// collection order).
pub fn find_groups(tracks: &[Track]) -> Vec<Vec<i64>> {
    let mut v: Vec<&Track> = tracks.iter().collect();
    v.sort_by(|a, b| a.title.as_str().cmp(b.title.as_str()).then_with(|| a.artist.as_str().cmp(b.artist.as_str())));
    let mut out: Vec<Vec<i64>> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i + 1;
        while j < v.len() && !v[i].title.is_empty() && v[j].title == v[i].title && v[j].artist == v[i].artist {
            j += 1;
        }
        if j - i > 1 {
            out.push(v[i..j].iter().map(|t| t.id).collect());
        }
        i = j;
    }
    out
}

impl App {
    pub fn dupes_search(&mut self) {
        let groups = find_groups(&self.tracks);
        let rows: Vec<(i64, usize)> = groups.iter().enumerate()
            .flat_map(|(group, ids)| ids.iter().map(move |id| (*id, group))).collect();
        self.dupes = Dupes { open: true, rows, ..Dupes::default() };
        if self.dupes.rows.is_empty() {
            return;
        }

        // Read each playlist/history once, even when many tracks share the same title.
        let hits: BTreeSet<i64> = self.dupes.rows.iter().map(|(id, _)| *id).collect();
        let mut add_path = |members: Vec<i64>, name: String| {
            let matching: BTreeSet<_> = members.into_iter().filter(|id| hits.contains(id)).collect();
            for id in matching {
                self.dupes.paths.entry(id).or_default().push(name.clone());
            }
        };
        for node in self.nodes.iter().filter(|n| n.kind == NodeKind::Playlist) {
            let Ok(members) = self.db.playlist_tracks(node.id) else { continue };
            let mut parts = vec![node.name.as_str()];
            let mut parent = node.parent;
            let mut visited = BTreeSet::new();
            while parent != 0 && visited.insert(parent) {
                let Some(folder) = self.nodes.iter().find(|n| n.id == parent) else { break };
                parts.push(&folder.name);
                parent = folder.parent;
            }
            parts.push("Playlists");
            parts.reverse();
            add_path(members, parts.join(" / "));
        }
        for history in self.db.histories().unwrap_or_default() {
            if let Ok(members) = self.db.history_tracks(history.id) {
                add_path(members, format!("Histories / {}", history.name));
            }
        }
    }
}

/// The manager's columns (see the notes at the top) and the 20 pt path column after them.
const COLS: [(Col, f32); 6] = [(Col::Status, 90.0), (Col::Preview, 110.0), (Col::Artwork, -1.0), (Col::Title, 190.0), (Col::Artist, 140.0), (Col::Location, 0.0)];

/// A column's width: its own, or (-1) the collection list's width for it.
fn col_w(app: &App, c: (Col, f32)) -> f32 {
    if c.1 < 0.0 { crate::ui_browser::col_width(app, c.0) } else { c.1 }
}
const PATH_W: f32 = 20.0;

/// rekordbox's `brws_list_pathsystem_pulldown.svg`: two grey chevrons, 10 x 16 pt.
fn path_icon(p: &egui::Painter, c: egui::Pos2) {
    let col = gy(0x646464);
    let o = c - vec2(5.0, 8.0);
    let up = vec![o + vec2(5.092, 0.0), o + vec2(10.0, 5.998), o + vec2(7.817, 5.998), o + vec2(5.089, 2.79), o + vec2(2.367, 5.998), o + vec2(0.0, 5.998)];
    let down = vec![o + vec2(4.908, 16.0), o + vec2(0.0, 10.001), o + vec2(2.183, 10.001), o + vec2(4.911, 13.21), o + vec2(7.633, 10.001), o + vec2(10.0, 10.001)];
    for pts in [up, down] {
        p.add(egui::Shape::Path(egui::epaint::PathShape { points: pts, closed: true, fill: col, stroke: egui::epaint::PathStroke::NONE }));
    }
}

/// The visible rows and their playlist/history memberships.
fn track_table(app: &mut App, ui: &mut egui::Ui, table: Rect, ids: &[i64]) {
    let salt = "dupes_table";
    let p = ui.painter().clone();
    p.rect_filled(table, 0.0, BG);
    let font = rb(11.5);
    let cols: Vec<(Col, f32)> = COLS.iter().map(|c| (c.0, col_w(app, *c))).collect();
    let fixed: f32 = cols.iter().map(|c| c.1).sum::<f32>() + PATH_W;
    // (Location takes the width that is left; a long path is cut, as in rekordbox's list.)
    let content_w = (fixed + 120.0).max(table.width());
    let loc = content_w - fixed;
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(table).id_salt(salt));
    tui.set_clip_rect(table);
    crate::ui_browser_ext::bar_style(&mut tui);
    tui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let (cmd, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
    let mut open_paths: Option<(i64, egui::Pos2)> = None;
    let row_h = crate::ui_browser::row_height(app);
    egui::ScrollArea::horizontal().id_salt((salt, "h")).auto_shrink([false, false]).show(&mut tui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (hr, _) = ui.allocate_exact_size(vec2(content_w, 22.0), Sense::hover());
        ui.painter().rect_filled(hr, 0.0, gy(0x1A1A1A));
        let mut x = hr.min.x;
        for (i, (c, cw)) in cols.iter().enumerate() {
            if i > 0 {
                ui.painter().rect_filled(Rect::from_min_max(pos2(x - 1.0, hr.min.y), pos2(x, hr.max.y)), 0.0, BG);
            }
            let label = if *c == Col::Status { "" } else { c.label() };
            ui.painter().with_clip_rect(Rect::from_min_size(pos2(x, hr.min.y), vec2(if *cw > 0.0 { *cw } else { loc }, hr.height()))).text(pos2(x + 6.0, hr.center().y), Align2::LEFT_CENTER, label, font.clone(), DIM.c());
            x += if *cw > 0.0 { *cw } else { loc };
        }
        ui.painter().rect_filled(Rect::from_min_max(pos2(x - 1.0, hr.min.y), pos2(x, hr.max.y)), 0.0, BG);
        egui::ScrollArea::vertical().id_salt((salt, "v")).auto_shrink([false, false]).show_rows(ui, row_h, ids.len(), |ui, range| {
            for i in range {
                let Some(t) = app.track(ids[i]).cloned() else { continue };
                let (rr, resp) = ui.allocate_exact_size(vec2(content_w, row_h), Sense::click());
                let selected = app.dupes.sel.contains(&t.id);
                let _selected_theme = crate::theme::dark_scope(selected);
                if selected {
                    ui.painter().rect_filled(rr, 0.0, ROW_SEL);
                } else if i % 2 == 1 {
                    ui.painter().rect_filled(rr, 0.0, gy(0x0A0A0A));
                }
                let mut x = rr.min.x;
                for (c, cw) in cols.iter() {
                    let w = if *cw > 0.0 { *cw } else { loc };
                    let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(w, rr.height())).intersect(ui.clip_rect());
                    if cr.width() > 0.0 {
                        crate::ui_browser::draw_cell(app, ui, *c, &t, cr, i);
                    }
                    x += w;
                }
                // The path column: the chevrons when the track is in a playlist or history; a
                // click drops the list of where it is.
                let pr = Rect::from_min_size(pos2(x, rr.min.y), vec2(PATH_W, rr.height()));
                let mut path_click = false;
                if app.dupes.paths.contains_key(&t.id) {
                    path_icon(&ui.painter().with_clip_rect(pr.intersect(ui.clip_rect())), pr.center());
                    path_click = resp.clicked() && resp.interact_pointer_pos().map(|pp| pr.contains(pp)).unwrap_or(false);
                }
                if path_click {
                    open_paths = Some((t.id, resp.interact_pointer_pos().unwrap_or(pr.center())));
                }
                if resp.clicked() {
                    if let Some(anchor) = app.dupes.anchor.filter(|_| shift) {
                        if !cmd {
                            app.dupes.sel.clear();
                        }
                        app.dupes.sel.extend(ids[anchor.min(i)..=anchor.max(i)].iter().copied());
                    } else {
                        if !cmd {
                            app.dupes.sel.clear();
                        }
                        if !app.dupes.sel.insert(t.id) {
                            app.dupes.sel.remove(&t.id);
                        }
                        app.dupes.anchor = Some(i);
                    }
                }
            }
        });
    });
    if let Some(o) = open_paths {
        app.dupes.path_popup = Some(o);
    }
    // The dropped list of paths (rekordbox's `PathSystemComboBox` popup).
    if let Some((id, at)) = app.dupes.path_popup.clone() {
        let list = app.dupes.paths.get(&id).cloned().unwrap_or_default();
        let mut close = false;
        let area = egui::Area::new(egui::Id::new((salt, "paths"))).order(egui::Order::Tooltip).fixed_pos(at).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).fill(gy(0x202020)).show(ui, |ui| {
                ui.set_min_width(200.0);
                for l in &list {
                    if ui.add(egui::Label::new(egui::RichText::new(l).font(rb(11.5)).color(WHITE)).sense(Sense::click())).clicked() {
                        close = true;
                    }
                }
            });
        });
        if close || (ui.input(|i| i.pointer.any_pressed()) && !area.response.rect.contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default())) && open_paths.is_none()) {
            app.dupes.path_popup = None;
        }
    }
}

pub fn windows(app: &mut App, ctx: &egui::Context) {
    if !app.dupes.open {
        return;
    }
    let mut open = true;
    let mut review = false;
    kit::dialog(ctx, "Duplicated File Manager").id(egui::Id::new("dupes_manager"))
        .open(&mut open).resizable(true).default_size([830.0, 600.0]).min_size([520.0, 320.0])
        .show(ctx, |ui| {
            kit::apply_style(ui);
            ui.label(format!("{} tracks share a title and artist", app.dupes.rows.len()));
            if !app.dupes.rows.is_empty() {
                let ids: Vec<_> = app.dupes.rows.iter().map(|(id, _)| *id).collect();
                let (table, _) = ui.allocate_exact_size(vec2(ui.available_width(), (ui.available_height() - 36.0).max(120.0)), Sense::hover());
                track_table(app, ui, table, &ids);
                review = ui.add_enabled(!app.dupes.sel.is_empty(), kit::btn("Review selected for cleanup…")).clicked();
            }
        });
    app.dupes.open = open && !review;
    if review {
        crate::collection_cleanup::ui::open(app, Some(app.dupes.sel.clone()));
    }
}

pub fn shot_setup(app: &mut App, _name: &str) {
    app.dupes_search();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: i64, title: &str, artist: &str) -> Track {
        Track { id, title: title.into(), artist: artist.into(), ..Default::default() }
    }

    #[test]
    fn duplicates_are_same_title_and_artist() {
        let tracks = vec![t(1, "Pacha", "ZARO"), t(2, "Brickell", "ZARO"), t(3, "Pacha", "ZARO"), t(4, "Pacha", "Other"), t(5, "", ""), t(6, "", ""), t(7, "pacha", "ZARO"), t(8, "Pacha", "ZARO")];
        // Exact comparison: another artist, another case and empty titles are not duplicates.
        assert_eq!(find_groups(&tracks), vec![vec![1, 3, 8]]);
        assert!(find_groups(&[t(1, "A", "x"), t(2, "B", "x")]).is_empty());
        // Two groups come out in title order.
        let two = vec![t(1, "B", "x"), t(2, "A", "y"), t(3, "B", "x"), t(4, "A", "y")];
        assert_eq!(find_groups(&two), vec![vec![2, 4], vec![1, 3]]);
    }

}
