//! Manual desktop library sync. An import is always inbound. Writing into rekordbox
//! requires its own field preview, selection and confirmation.
use crate::app::App;
use crate::db::Db;
use crate::rbimport::writeback::{self, Plan, Receipt};
use crate::theme::hex;
use crate::ui_kit::{self as kit, btn};
use crossbeam_channel::Receiver;
use eframe::egui::{self, Context, RichText, Ui};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Import,
    WriteBack,
}

enum ResultView {
    Preview(Plan),
    Applied(Receipt),
    Recovered(PathBuf),
}

enum Outbound {
    Ready,
    Working {
        label: &'static str,
        writing: bool,
        rx: Receiver<anyhow::Result<ResultView>>,
    },
    Preview {
        plan: Plan,
        selected: BTreeSet<usize>,
        links: Vec<Option<usize>>,
        page: usize,
        confirm: bool,
    },
    Applied(Receipt),
    Recovered(PathBuf),
    Failed(String),
}

pub struct Review {
    desktop: Option<PathBuf>,
    local: Option<PathBuf>,
    found: bool,
    direction: Option<Direction>,
    outbound: Outbound,
    recovery: Result<bool, String>,
    confirm_recovery: bool,
}

impl Review {
    pub fn new(db: &Db) -> Self {
        let desktop = crate::rbimport::default_library_dir();
        let found = desktop.as_ref().is_some_and(|dir| dir.join("master.db").is_file());
        let recovery = Self::check_recovery(desktop.as_deref());
        Self {
            desktop,
            local: db.conn.path().filter(|p| !p.is_empty()).map(PathBuf::from),
            found,
            direction: None,
            outbound: Outbound::Ready,
            recovery,
            confirm_recovery: false,
        }
    }

    fn check_recovery(desktop: Option<&Path>) -> Result<bool, String> {
        match desktop {
            Some(dir) => writeback::pending(dir)
                .map_err(|e| format!("Could not check the desktop library for an interrupted write-back. {e:#}")),
            None => Ok(false),
        }
    }

    pub fn status(&self) -> Option<&'static str> {
        match &self.outbound {
            Outbound::Working { label, .. } => Some(label),
            _ => None,
        }
    }

    fn start(
        &mut self,
        ctx: &Context,
        label: &'static str,
        writing: bool,
        work: impl FnOnce(&Path, &Path) -> anyhow::Result<ResultView> + Send + 'static,
    ) {
        let (Some(local), Some(desktop)) = (self.local.clone(), self.desktop.clone()) else {
            self.outbound =
                Outbound::Failed("Could not locate both libraries. Reopen library sync to check their paths.".into());
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        self.outbound = match std::thread::Builder::new().name("rb-writeback".into()).spawn(move || {
            let _ = tx.send(work(&local, &desktop));
            ctx.request_repaint();
        }) {
            Ok(_) => Outbound::Working { label, writing, rx },
            Err(e) => Outbound::Failed(format!("Could not start the worker. {e}")),
        };
    }

    fn poll(&mut self) {
        let Outbound::Working { rx, .. } = &self.outbound else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(anyhow::anyhow!(
                "The worker stopped before it could report a result. Check for pending recovery and preview again."
            )),
        };
        self.recovery = Self::check_recovery(self.desktop.as_deref());
        self.confirm_recovery = false;
        self.outbound = match result {
            Ok(ResultView::Preview(plan)) => Outbound::Preview {
                links: selection_links(&plan),
                plan,
                selected: BTreeSet::new(),
                page: 0,
                confirm: false,
            },
            Ok(ResultView::Applied(receipt)) => Outbound::Applied(receipt),
            Ok(ResultView::Recovered(path)) => Outbound::Recovered(path),
            Err(error) => Outbound::Failed(format!("{error:#}")),
        };
    }
}

/// Native radios and checkboxes paint their text, dot and checkmark with the widget
/// foreground. Use the window's actual fill because system theme changes can give the
/// review a light background while other controls still use the dark preferences kit.
fn review_style(ui: &mut Ui) {
    kit::apply_style(ui);
    let visuals = ui.visuals_mut();
    let surface = visuals.window_fill();
    let light = egui::Rgba::from(surface).intensity() > 0.179;
    let (ink, hover, active, disabled_tint, warning) = if light {
        (hex(0x17212B), hex(0xE2EEFA), hex(0xCEE2F5), hex(0xA0A8AD), hex(0xA32929))
    } else {
        (hex(0xF2F5FA), hex(0x2D3E50), hex(0x364F66), hex(0x505960), hex(0xFF9999))
    };
    // egui mixes disabled widgets halfway towards this color. Keep locked choices legible.
    visuals.widgets.noninteractive.weak_bg_fill = disabled_tint;
    visuals.error_fg_color = warning;
    let w = &mut visuals.widgets;
    for (v, fill) in [(&mut w.inactive, surface), (&mut w.hovered, hover), (&mut w.active, active), (&mut w.open, active)] {
        v.fg_stroke.color = ink;
        v.bg_fill = fill;
        v.bg_stroke = egui::Stroke::new(1.5_f32, ink);
    }
}

fn direction_choices(ui: &mut Ui, direction: &mut Option<Direction>, enabled: bool) {
    ui.add_enabled_ui(enabled, |ui| {
        ui.vertical(|ui| {
            ui.radio_value(direction, Some(Direction::Import), "Import · AlphaTheta Rekordbox → recordbox");
            ui.add_space(2.0);
            ui.radio_value(direction, Some(Direction::WriteBack), "Write back · recordbox → AlphaTheta Rekordbox");
        });
    });
}

pub fn show(app: &mut App, ctx: &Context) {
    let Some(review) = &mut app.dialogs.library_sync_review else {
        return;
    };
    review.poll();
    if matches!(review.outbound, Outbound::Working { writing: true, .. }) && ctx.input(|i| i.viewport().close_requested()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    }
    let mut open = true;
    let mut close = false;
    let mut import = false;
    let working = review.status().is_some();
    let mut window = egui::Window::new("Review desktop library sync")
        .id(egui::Id::new("library_sync_review"))
        .collapsible(false)
        .default_width(620.0)
        .vscroll(ctx.screen_rect().height() < 680.0);
    if !working {
        window = window.open(&mut open);
    }
    window.show(ctx, |ui| {
        review_style(ui);
        ui.heading("Sync direction");
        if app.prefs.rb.library_protection {
            ui.label("Library Protection is enabled. You can preview changes and review interrupted write-back recovery.");
        }
        let previous = review.direction;
        direction_choices(ui, &mut review.direction, !working);
        ui.separator();
        if review.direction != previous {
            review.outbound = Outbound::Ready;
            review.confirm_recovery = false;
        }

        let source = review.desktop.as_ref().map(|dir| dir.join("master.db"));
        let local = review.local.as_deref();
        match review.direction {
            Some(Direction::Import) => {
                library_path(ui, "Read from · AlphaTheta Rekordbox library", source.as_deref());
                library_path(ui, "Write to · active recordbox library", local);
            }
            Some(Direction::WriteBack) => {
                library_path(ui, "Read from · active recordbox library", local);
                library_path(ui, "Write to · original AlphaTheta Rekordbox library", source.as_deref());
            }
            None => {
                library_path(ui, "AlphaTheta Rekordbox library", source.as_deref());
                library_path(ui, "Active recordbox library", local);
            }
        }
        if !review.found {
            ui.colored_label(ui.visuals().error_fg_color, "Desktop master.db could not be found at the default location.");
        }
        if let Err(error) = &review.recovery {
            ui.colored_label(ui.visuals().error_fg_color, error);
            if ui.add(btn("Recheck recovery status")).clicked() {
                review.recovery = Review::check_recovery(review.desktop.as_deref());
            }
        }
        ui.separator();
        match review.direction {
            Some(Direction::Import) => {
                ui.label("Import reads a private snapshot of the desktop database and its write-ahead log, plus available analysis and audio metadata.");
                ui.label("Source changes can replace local metadata, cues, grids and My Tags. Import rebuilds the managed rekordbox playlists and histories using locally accessible tracks. Keep your own playlists outside that folder.");
                if app.prefs.auto_analysis {
                    ui.label("Auto Analysis is enabled. Adding tracks can queue analysis in recordbox.");
                }
                if matches!(review.recovery, Ok(true)) {
                    ui.colored_label(ui.visuals().error_fg_color, "An interrupted write-back needs review. Choose Write back to inspect recovery before importing.");
                }
                import = ui.add_enabled(!app.prefs.rb.library_protection && review.found && matches!(review.recovery, Ok(false)), btn("Import into recordbox")).clicked();
            }
            Some(Direction::WriteBack) => writeback_view(review, ui, app.prefs.rb.library_protection),
            None => {}
        }
        if !working {
            close = ui.add(btn("Close")).clicked();
        }
    });
    if import || close || !open {
        app.dialogs.library_sync_review = None;
    }
    if import {
        app.start_rekordbox_sync();
    }
}

fn library_path(ui: &mut Ui, label: &str, path: Option<&Path>) {
    ui.label(RichText::new(label).strong());
    ui.add(
        egui::Label::new(
            path.map(|p| p.display().to_string())
                .unwrap_or_else(|| "Path unavailable".into()),
        )
        .selectable(true)
        .wrap(),
    );
}

fn changes(n: usize) -> String {
    format!("{n} change{}", if n == 1 { "" } else { "s" })
}

fn writeback_view(review: &mut Review, ui: &mut Ui, library_protected: bool) {
    if let Some(status) = review.status() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(status);
        });
        return;
    }
    if let Outbound::Failed(message) = &review.outbound {
        ui.colored_label(ui.visuals().error_fg_color, message);
    }
    if review.recovery.is_err() {
        return;
    }
    if matches!(review.recovery, Ok(true)) {
        ui.label(
            "An interrupted write-back has a recovery record. Close AlphaTheta Rekordbox and its agents before resolving it.",
        );
        if review.confirm_recovery {
            ui.label("Recovery keeps a verified database commit or restores the previous analysis files when the database did not commit. It stops if the files no longer match the saved recovery record.");
            ui.horizontal(|ui| {
                if ui.add(btn("Confirm recovery")).clicked() {
                    review.start(
                        ui.ctx(),
                        "Recovering the interrupted write-back…",
                        true,
                        |local, desktop| writeback::recover(local, desktop).map(ResultView::Recovered),
                    );
                }
                if ui.add(btn("Back")).clicked() {
                    review.confirm_recovery = false;
                }
            });
        } else if ui.add(btn("Review recovery…")).clicked() {
            review.confirm_recovery = true;
        }
        return;
    }
    let mut apply = false;
    match &mut review.outbound {
        Outbound::Preview {
            plan,
            selected,
            links,
            page,
            confirm,
        } => {
            if *confirm {
                let count = selected.len();
                ui.label(format!(
                    "Apply {} to the original desktop library shown above?",
                    changes(count)
                ));
                if let Some(path) = review.local.as_deref().and_then(|p| writeback::backup_root(p).ok()) {
                    library_path(ui, "Create recovery backup in", Some(&path));
                }
                ui.label("recordbox rechecks the selected fields and creates a verified recovery copy of the database and affected analysis files before writing. Close AlphaTheta Rekordbox and its agents. Use a local library with Cloud Library Sync turned off on every linked computer.");
                ui.horizontal_wrapped(|ui| {
                    apply = ui
                        .add_enabled(!library_protected && count > 0, btn(format!("Back up and apply {}", changes(count))))
                        .clicked();
                    if ui.add(btn("Back to selection")).clicked() {
                        *confirm = false;
                    }
                });
                if !apply {
                    return;
                }
            } else {
                preview_rows(ui, plan, selected, links, page);
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(!library_protected && !selected.is_empty(), btn("Review selected changes")).clicked() {
                        *confirm = true;
                    }
                });
            }
        }
        Outbound::Applied(receipt) => {
            ui.label(format!(
                "Applied {} to the AlphaTheta Rekordbox library.",
                changes(receipt.applied)
            ));
            library_path(ui, "Recovery backup", Some(&receipt.backup));
        }
        Outbound::Recovered(backup) => {
            ui.label("Recovery completed. A fresh preview will show the current values.");
            library_path(ui, "Recovery backup", Some(backup));
        }
        Outbound::Ready => {
            ui.label("Compare imported ratings, cues, BPM, keys and beat grids. Use a local library with Cloud Library Sync turned off on every linked computer.");
        }
        Outbound::Failed(_) | Outbound::Working { .. } => {}
    }

    if apply {
        if let Outbound::Preview { plan, selected, .. } = std::mem::replace(&mut review.outbound, Outbound::Ready) {
            let rows = selected.into_iter().collect::<Vec<_>>();
            review.start(
                ui.ctx(),
                "Backing up and applying the selected changes…",
                true,
                move |local, desktop| writeback::apply(local, desktop, &plan, &rows).map(ResultView::Applied),
            );
        }
    } else if ui
        .add_enabled(
            review.found && review.local.is_some(),
            btn(if matches!(review.outbound, Outbound::Ready) {
                "Preview changes"
            } else {
                "Refresh preview"
            }),
        )
        .clicked()
    {
        review.start(
            ui.ctx(),
            "Comparing recordbox edits with the desktop library…",
            false,
            |local, desktop| writeback::preview(local, desktop).map(ResultView::Preview),
        );
    }
}

/// A row links to itself for a standalone change or to its partner for a coupled BPM/grid.
fn selection_links(plan: &Plan) -> Vec<Option<usize>> {
    let eligible: HashMap<_, _> = plan
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.reason.is_none())
        .map(|(index, r)| ((r.local_id, r.field), index))
        .collect();
    plan.rows
        .iter()
        .enumerate()
        .map(|(index, r)| {
            if r.reason.is_some() {
                return None;
            }
            match r.companion {
                Some(field) => eligible.get(&(r.local_id, field)).copied(),
                None => Some(index),
            }
        })
        .collect()
}

fn select_row(selected: &mut BTreeSet<usize>, links: &[Option<usize>], index: usize, checked: bool) {
    if let Some(partner) = links[index] {
        if checked {
            selected.extend([index, partner]);
        } else {
            selected.remove(&index);
            selected.remove(&partner);
        }
    }
}

fn preview_rows(ui: &mut Ui, plan: &Plan, selected: &mut BTreeSet<usize>, links: &[Option<usize>], page: &mut usize) {
    const PAGE_SIZE: usize = 20;
    if plan.tracked == 0 {
        ui.label("No tracks have an import baseline for this desktop library. Import it into recordbox before editing tracks for write-back.");
    } else if plan.rows.is_empty() {
        ui.label(format!("No changes to review across {} tracked tracks.", plan.tracked));
    }
    if plan.rows.is_empty() {
        return;
    }
    ui.label(format!(
        "{} to review · {} selected",
        changes(plan.rows.len()),
        selected.len()
    ));
    let start = *page * PAGE_SIZE;
    let end = plan.rows.len().min(start + PAGE_SIZE);
    egui::ScrollArea::both()
        .id_salt("writeback_changes")
        .max_height(265.0)
        .show(ui, |ui| {
            egui::Grid::new("writeback_rows")
                .striped(true)
                .num_columns(5)
                .max_col_width(135.0)
                .show(ui, |ui| {
                    for label in [
                        "Track and field",
                        "At import",
                        "In AlphaTheta Rekordbox",
                        "From recordbox",
                        "Status",
                    ] {
                        ui.strong(label);
                    }
                    ui.end_row();
                    for index in start..end {
                        let row = &plan.rows[index];
                        let mut checked = selected.contains(&index);
                        let label = format!("{} · {}", row.track, row.field.label());
                        if ui
                            .add_enabled(links[index].is_some(), egui::Checkbox::new(&mut checked, label))
                            .changed()
                        {
                            select_row(selected, links, index, checked);
                        }
                        for value in [&row.before, &row.current, &row.proposed] {
                            ui.add(egui::Label::new(value.as_str()).selectable(true).wrap());
                        }
                        if let Some(reason) = &row.reason {
                            ui.label(reason);
                        } else if let Some(field) = row.companion {
                            if links[index].is_some() {
                                ui.label(format!("Selected together with {}", field.label()));
                            } else {
                                ui.label(format!("Requires an eligible {} change", field.label()));
                            }
                        } else {
                            ui.label("Eligible");
                        }
                        ui.end_row();
                    }
                });
        });
    ui.horizontal_wrapped(|ui| {
        if ui.add(btn("Select eligible on this page")).clicked() {
            for index in start..end {
                select_row(selected, links, index, true);
            }
        }
        if ui.add_enabled(!selected.is_empty(), btn("Clear selection")).clicked() {
            selected.clear();
        }
        if plan.rows.len() > PAGE_SIZE {
            if ui.add_enabled(*page > 0, btn("Previous")).clicked() {
                *page -= 1;
            }
            ui.label(format!("Page {} of {}", *page + 1, plan.rows.len().div_ceil(PAGE_SIZE)));
            if ui.add_enabled(end < plan.rows.len(), btn("Next")).clicked() {
                *page += 1;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Shape};

    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |c| {
            let [r, g, b, _] = egui::Rgba::from(c).to_array();
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn painted_sync_directions_remain_legible_in_both_window_themes() {
        for palette in [egui::Visuals::light(), egui::Visuals::dark()] {
            let ctx = Context::default();
            crate::theme::apply(&ctx);
            ctx.set_visuals(palette);
            for selected in [None, Some(Direction::Import), Some(Direction::WriteBack)] {
                for enabled in [true, false] {
                    let mut direction = selected;
                    // Use the real review styles and radio widgets. Render one layout pass
                    // first, then hover the Import row to cover both hovered and idle labels.
                    let mut pointer = egui::Pos2::ZERO;
                    let mut frame = || {
                        let input = egui::RawInput { events: vec![egui::Event::PointerMoved(pointer)], ..Default::default() };
                        ctx.run(input, |ctx| {
                            egui::CentralPanel::default().frame(egui::Frame::window(&ctx.style()).shadow(Default::default())).show(ctx, |ui| {
                                review_style(ui);
                                pointer = ui.cursor().min + egui::vec2(15.0, 10.0);
                                direction_choices(ui, &mut direction, enabled);
                            });
                        })
                    };
                    frame();
                    let output = frame();
                    let mut surface = None;
                    let mut labels = Vec::new();
                    let mut rings = Vec::new();
                    let mut dot = None;
                    for clipped in &output.shapes {
                        match &clipped.shape {
                            Shape::Rect(r) if r.rect.width() > 400.0 && r.fill.a() == 255 => surface = Some(r.fill),
                            Shape::Text(_) => labels.push(clipped),
                            Shape::Circle(c) if c.stroke.width > 0.0 => rings.push(c),
                            Shape::Circle(c) => dot = Some(c),
                            _ => {}
                        }
                    }
                    let surface = surface.expect("the window surface was painted");
                    assert_eq!(labels.len(), 2);
                    assert_eq!(rings.len(), 2);
                    for clipped in labels {
                        // Tessellation resolves both the fallback text color and any disabled tint.
                        let mesh = ctx.tessellate(vec![clipped.clone()], output.pixels_per_point);
                        let egui::epaint::Primitive::Mesh(triangles) = &mesh[0].primitive else { panic!("text mesh") };
                        assert!(triangles.vertices.iter().all(|v| contrast(v.color, surface) >= 4.5));
                    }
                    for ring in &rings {
                        assert!(contrast(ring.stroke.color, surface) >= 3.0);
                        assert!(contrast(ring.stroke.color, ring.fill) >= 3.0);
                    }
                    assert_eq!(dot.is_some(), selected.is_some());
                    if let (Some(dot), Some(which)) = (dot, selected) {
                        let ring = rings[usize::from(which == Direction::WriteBack)];
                        assert_eq!(dot.center, ring.center);
                        assert!(contrast(dot.fill, ring.fill) >= 3.0);
                    }
                }
            }
        }
    }
}
