//! Review a local collection before moving selected references to Removed Tracks.
use super::{review_path_cancellable, store, Candidate, Report, Row};
use crate::app::App;
use crate::theme::CAUTION_TX;
use crate::ui_kit::{self as kit, btn};
use crossbeam_channel::{Receiver, TryRecvError};
use eframe::egui::{self, Context, Ui};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

enum Outcome {
    Scan(Report),
    Applied(PathBuf),
}

enum Phase {
    Start,
    Working {
        applying: bool,
        result: Receiver<anyhow::Result<Outcome>>,
    },
    Browse,
    Confirm,
    Done(PathBuf),
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Candidates,
    Protected,
}

pub struct Review {
    library: PathBuf,
    scope: Option<BTreeSet<i64>>,
    phase: Phase,
    report: Option<Arc<Report>>,
    os_summary: String,
    cancel_scan: Arc<AtomicBool>,
    visible: Vec<usize>,
    selected: BTreeSet<usize>,
    keepers: BTreeMap<i64, usize>,
    filter: Filter,
    volume: Option<usize>,
    compare: bool,
    confirmed_local: bool,
    confirmation: String,
}

pub fn open(app: &mut App, scope: Option<BTreeSet<i64>>) {
    if app.ext.cleanup.is_some() {
        return;
    }
    let Some(path) = app.db.conn.path().filter(|path| !path.is_empty()) else {
        return app.toast_err("Open a local recordbox library before reviewing cleanup.");
    };
    app.ext.cleanup = Some(Review {
        library: PathBuf::from(path),
        scope,
        phase: Phase::Start,
        report: None,
        os_summary: String::new(),
        cancel_scan: Arc::new(AtomicBool::new(false)),
        visible: Vec::new(),
        selected: BTreeSet::new(),
        keepers: BTreeMap::new(),
        filter: Filter::All,
        volume: None,
        compare: false,
        confirmed_local: false,
        confirmation: String::new(),
    });
}

impl Review {
    fn work(
        &mut self,
        ctx: &Context,
        applying: bool,
        task: impl FnOnce() -> anyhow::Result<Outcome> + Send + 'static,
    ) {
        let (tx, result) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        self.phase = match std::thread::Builder::new()
            .name("collection-cleanup".into())
            .spawn(move || {
                let _ = tx.send(task());
                ctx.request_repaint();
            }) {
            Ok(_) => Phase::Working { applying, result },
            Err(e) => Phase::Failed(format!("Could not start the cleanup worker. {e}")),
        };
    }

    fn scan(&mut self, ctx: &Context, compare: bool) {
        self.selected.clear();
        self.clear_confirmation();
        self.volume = None;
        self.compare = compare;
        self.report = None;
        self.visible.clear();
        let path = self.library.clone();
        let cancel = Arc::clone(&self.cancel_scan);
        cancel.store(false, Ordering::Relaxed);
        self.work(ctx, false, move || {
            review_path_cancellable(&path, compare, &cancel).map(Outcome::Scan)
        });
    }

    fn clear_confirmation(&mut self) {
        self.confirmed_local = false;
        self.confirmation.clear();
        self.keepers.clear();
    }

    fn poll(&mut self, app: &mut App) {
        let Phase::Working { result, .. } = &self.phase else {
            return;
        };
        let done = match result.try_recv() {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!(
                "The cleanup worker stopped. Scan the library again."
            )),
        };
        self.phase = match done {
            Ok(Outcome::Scan(report)) => {
                self.os_summary = ["Windows", "macOS", "Unclassified"]
                    .map(|os| {
                        let count: usize = report
                            .volumes
                            .iter()
                            .filter(|v| v.os == os)
                            .map(|v| v.total)
                            .sum();
                        format!("{os} {count}")
                    })
                    .join(" · ");
                self.report = Some(Arc::new(report));
                self.refilter();
                Phase::Browse
            }
            Ok(Outcome::Applied(backup)) => {
                let report = self.report.as_ref().expect("applied reviewed selection");
                let removed: BTreeSet<_> =
                    self.selected.iter().map(|i| report.rows[*i].id).collect();
                // Reloading the entire library here would stat every file and could start
                // unrelated analyses on changed audio. Only these references moved.
                app.tracks.retain(|t| !removed.contains(&t.id));
                app.index = app
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (t.id, i))
                    .collect();
                app.collection_categories = Default::default();
                app.selected.retain(|id| !removed.contains(id));
                app.sub_sel.retain(|id| !removed.contains(id));
                app.previews.retain(|id, _| !removed.contains(id));
                app.cue_counts.retain(|id, _| !removed.contains(id));
                app.hot_positions.retain(|id, _| !removed.contains(id));
                app.list_dirty = true;
                Phase::Done(backup)
            }
            Err(e) => Phase::Failed(format!("{e:#}")),
        };
    }

    fn refilter(&mut self) {
        let Some(report) = &self.report else { return };
        let volume = self.volume.map(|i| &report.volumes[i]);
        self.visible = report
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                let in_volume = volume.is_none_or(|v| v.os == row.os && v.volume == row.volume);
                let in_scope = self.scope.as_ref().is_none_or(|ids| ids.contains(&row.id));
                let matches = match self.filter {
                    Filter::All => true,
                    Filter::Candidates => row.candidate.is_some(),
                    Filter::Protected => row.protected(),
                };
                (in_volume && in_scope && matches).then_some(i)
            })
            .collect();
    }

    fn toggle_volume(&mut self, i: usize) {
        self.volume = (self.volume != Some(i)).then_some(i);
        self.refilter();
    }

    fn browse(&mut self, ui: &mut Ui) {
        let report = Arc::clone(self.report.as_ref().expect("completed scan"));
        if !report.can_apply {
            let warning = if cfg!(target_os = "linux") {
                "Review only on Linux, including mounted Windows and macOS volumes. Use the computer that owns the Music folder to apply a cleanup."
            } else {
                "Review only on this system. Cleanup requires a verified local Windows or macOS Music folder."
            };
            ui.colored_label(CAUTION_TX, warning);
        }
        ui.label(format!(
            "{} collection references · {}",
            report.rows.len(),
            self.os_summary
        ));
        if ui.available_width() < 525.0 {
            for (i, v) in report.volumes.iter().enumerate() {
                if ui
                    .selectable_label(self.volume == Some(i), format!("{} · {}", v.os, v.volume))
                    .clicked()
                {
                    self.toggle_volume(i);
                }
                ui.small(format!(
                    "Total {} · Present {} · Missing¹ {} · Protected {} · Eligible {}",
                    v.total, v.present, v.missing, v.protected, v.candidates
                ));
            }
        } else {
            egui::Grid::new("cleanup_counts")
                .striped(true)
                .show(ui, |ui| {
                    for label in [
                        "Path origin",
                        "Drive / volume",
                        "Total",
                        "Present",
                        "Missing¹",
                        "Protected",
                        "Eligible",
                    ] {
                        ui.strong(label);
                    }
                    ui.end_row();
                    for (i, v) in report.volumes.iter().enumerate() {
                        ui.label(v.os);
                        if ui
                            .selectable_label(self.volume == Some(i), &v.volume)
                            .on_hover_text("Filter tracks to this volume")
                            .clicked()
                        {
                            self.toggle_volume(i);
                        }
                        for n in [v.total, v.present, v.missing, v.protected, v.candidates] {
                            ui.label(n.to_string());
                        }
                        ui.end_row();
                    }
                });
        }
        ui.small("¹ Missing means a file was absent under a verified local parent. Unavailable drives and remote paths stay protected.");
        ui.horizontal_wrapped(|ui| {
            for (status, count) in &report.statuses {
                ui.label(format!("{} · {count}", status.label()));
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (filter, name) in [(Filter::All, "All tracks"), (Filter::Candidates, "Eligible"), (Filter::Protected, "Protected")] {
                if ui.selectable_label(self.filter == filter, name).clicked() {
                    self.filter = filter;
                    self.refilter();
                }
            }
            if self.volume.is_some() && ui.add(btn("Show all volumes")).clicked() {
                self.volume = None;
                self.refilter();
            }
            if self.scope.is_some() && ui.add(btn("Show entire collection")).on_hover_text("The track list currently shows only the references chosen in Missing File Manager.").clicked() {
                self.scope = None;
                self.refilter();
            }
            ui.label(format!("{} shown", self.visible.len()));
            if report.can_apply {
                ui.label(format!("{} of {} selected", self.selected.len(), store::MAX_SELECTION));
            }
        });

        // Reserve the list in the dialog's one scroll area, painting only visible rows.
        // The drive table and records then move together on narrow or short screens.
        let step = 37.0 + ui.spacing().item_spacing.y;
        let size = egui::vec2(ui.available_width(), step * self.visible.len() as f32);
        let (_, list) = ui.allocate_space(size);
        let clip = ui.clip_rect().intersect(list);
        if !clip.is_positive() {
            return;
        }
        let first = (((clip.top() - list.top()) / step).floor() as usize).min(self.visible.len());
        let end = (((clip.bottom() - list.top()) / step).ceil() as usize).min(self.visible.len());
        for (offset, &i) in self.visible[first..end].iter().enumerate() {
            let row_top = list.top() + (first + offset) as f32 * step;
            let row_rect = egui::Rect::from_min_size(
                egui::pos2(list.left(), row_top),
                egui::vec2(list.width(), step),
            );
            let row = &report.rows[i];
            let mut on = self.selected.contains(&i);
            ui.scope_builder(
                egui::UiBuilder::new().id_salt(row.id).max_rect(row_rect),
                |ui| {
                    ui.horizontal(|ui| {
                        let enabled = report.can_apply
                            && row.candidate.is_some()
                            && (on || self.selected.len() < store::MAX_SELECTION);
                        if ui.add_enabled(enabled, kit::Check(&mut on)).changed() {
                            if on {
                                self.selected.insert(i);
                            } else {
                                self.selected.remove(&i);
                            }
                        }
                        ui.vertical(|ui| {
                            ui.add(
                                egui::Label::new(format!(
                                    "{} · {} · #{}",
                                    row.title,
                                    reason(row),
                                    row.id
                                ))
                                .truncate(),
                            );
                            ui.add(
                                egui::Label::new(egui::RichText::new(&row.path).small()).truncate(),
                            )
                            .on_hover_text(&row.path);
                        });
                    })
                },
            );
        }
    }

    fn browse_actions(&mut self, ui: &mut Ui) {
        let report = Arc::clone(self.report.as_ref().expect("completed scan"));
        ui.separator();
        if report.can_apply {
            if ui.checkbox(&mut self.compare, "My Music folder stays offline-accessible and outside cloud sync. Allow byte comparison (up to 8 GiB).").changed() && !self.compare && report.compare_bytes {
                self.scan(ui.ctx(), false);
                return;
            }
            if report.compare_bytes {
                ui.label(format!(
                    "Compared {} files · {} bytes read · {} unverified",
                    report.hashed_files, report.hashed_bytes, report.unverified_files
                ));
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui.add(btn("Scan references again")).clicked() {
                self.compare = false;
                self.scan(ui.ctx(), false);
            } else if report.can_apply && ui.add_enabled(self.compare, btn("Compare audio bytes…")).on_hover_text("Reads eligible local files with matching sizes, up to 8 GiB per scan. Clears the previous selection.").clicked() {
                self.scan(ui.ctx(), true);
            } else if ui.add_enabled(report.can_apply && !self.selected.is_empty(), btn(format!("Review {} selected…", self.selected.len()))).clicked() {
                self.clear_confirmation();
                let keep: BTreeSet<_> = self.selected.iter().filter_map(|i| match report.rows[*i].candidate {
                    Some(Candidate::Duplicate { keeper }) => Some(keeper),
                    _ => None,
                }).collect();
                if !keep.is_empty() {
                    self.keepers = report.rows.iter().enumerate().filter(|(_, r)| keep.contains(&r.id)).map(|(i, r)| (r.id, i)).collect();
                }
                if self.keepers.len() != keep.len() {
                    self.phase = Phase::Failed("A retained reference could not be found. Scan the collection again.".into());
                    return;
                }
                self.phase = Phase::Confirm;
            }
        });
    }

    fn confirm(&mut self, app: &App, ui: &mut Ui) {
        let report = Arc::clone(self.report.as_ref().expect("reviewed scan"));
        let count = self.selected.len();
        ui.colored_label(
            CAUTION_TX,
            format!("Move {count} collection reference(s) to Removed Tracks?"),
        );
        let duplicates = self
            .selected
            .iter()
            .filter(|i| {
                matches!(
                    report.rows[**i].candidate,
                    Some(Candidate::Duplicate { .. })
                )
            })
            .count();
        ui.label(format!("Selected · {} verified missing · {duplicates} exact duplicates. {} of {} references will remain.", count - duplicates, report.rows.len() - count, report.rows.len()));
        chosen_paths(ui, &report, &self.selected, &self.keepers);
        ui.label("Recordbox rechecks the selected references and verifies a recovery backup before moving them. Music files, USB exports and the AlphaTheta library stay in place.");
        if let Some(parent) = self.library.parent() {
            ui.label(format!(
                "Backup folder · {}",
                parent.join("cleanup-backups").display()
            ));
        }
        ui.label("For 30 days, right-click All in Collection and choose Restore Removed Tracks. To restore the backup, open Preferences → Advanced → Database → Restore Library.");
        ui.checkbox(&mut self.confirmed_local, "I verified these tracks were imported on this computer from this Music folder. The folder is always local and outside every sync service.");
        let phrase = format!("MOVE {count}");
        ui.label(format!("Type {phrase} to confirm."));
        ui.text_edit_singleline(&mut self.confirmation);
        let busy = app.db.conn.path().map(std::path::Path::new) != Some(self.library.as_path())
            || busy(app, &report, &self.selected);
        if busy {
            ui.label("Close this review, finish library jobs and unload any selected tracks from decks or preview, then scan again.");
        }
        if app.prefs.rb.library_protection {
            ui.label("Library Protection is enabled. Cleanup is read-only.");
        }
        ui.horizontal(|ui| {
            if ui.add(btn("Back to review")).clicked() {
                self.clear_confirmation();
                self.phase = Phase::Browse;
            } else if ui
                .add_enabled(
                    !busy
                        && !app.prefs.rb.library_protection
                        && report.can_apply
                        && count > 0
                        && count <= store::MAX_SELECTION
                        && self.confirmed_local
                        && self.confirmation == phrase,
                    btn("Move reviewed references"),
                )
                .clicked()
            {
                let ids = self.selected.iter().map(|i| report.rows[*i].id).collect();
                let path = self.library.clone();
                let confirmation = self.confirmation.clone();
                let confirmed_local = self.confirmed_local;
                self.work(ui.ctx(), true, move || {
                    store::apply(&path, &report, &ids, confirmed_local, &confirmation)
                        .map(Outcome::Applied)
                });
            }
        });
    }
}

/// Paint the entire reviewed batch in the enclosing confirmation scroll area.
pub(super) fn chosen_paths(
    ui: &mut Ui,
    report: &Report,
    selected: &BTreeSet<usize>,
    keepers: &BTreeMap<i64, usize>,
) {
    for &i in selected {
        let row = &report.rows[i];
        ui.label(format!("#{} · {} · {}", row.id, row.title, reason(row)));
        ui.add(
            egui::Label::new(egui::RichText::new(&row.path).small())
                .selectable(true)
                .wrap(),
        );
        if let Some(Candidate::Duplicate { keeper }) = row.candidate {
            let retained = &report.rows[keepers[&keeper]];
            ui.add(
                egui::Label::new(format!(
                    "Keep #{} · {} · {}",
                    retained.id, retained.title, retained.path
                ))
                .selectable(true)
                .wrap(),
            );
        }
    }
}

fn reason(row: &Row) -> String {
    match &row.candidate {
        Some(Candidate::Duplicate { keeper }) => format!("{} · keep #{keeper}", row.explanation()),
        _ if row.protected() && row.explanation() != row.status.label() => {
            format!("{} · {}", row.status.label(), row.explanation())
        }
        _ => row.explanation().to_owned(),
    }
}

fn busy(app: &App, report: &Report, selected: &BTreeSet<usize>) -> bool {
    let selected_track =
        |id: Option<i64>| id.is_some_and(|id| selected.iter().any(|&i| report.rows[i].id == id));
    app.importing.is_some()
        || app.rb_job.is_some()
        || app.dialogs.library_sync_review.is_some()
        || app.analysis_total > 0
        || app.export_job.is_some()
        || app.key_job.is_some()
        || app.device_ui.import.is_some()
        || app.relocate_rx.is_some()
        || app.dialogs.ext.backup_job.is_some()
        || app.dialogs.ext.restore_ask.is_some()
        || (0..4).any(|d| selected_track(app.deck_of(d).track))
        || selected_track(app.ext.preview.state.lock().track)
}

/// The library can redock during cleanup. Keep the process alive while changes are being applied.
pub fn guard_app_close(app: &App, ctx: &Context) {
    if ctx.input(|i| i.viewport().close_requested())
        && matches!(app.ext.cleanup.as_ref().map(|review| &review.phase), Some(Phase::Working { applying: true, .. }))
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    }
}

pub fn show(app: &mut App, ctx: &Context) {
    let Some(mut review) = app.ext.cleanup.take() else {
        return;
    };
    if matches!(review.phase, Phase::Start) {
        review.scan(ctx, false);
    }
    review.poll(app);
    let applying = matches!(review.phase, Phase::Working { applying: true, .. });
    let response = egui::Modal::new("collection_cleanup".into()).frame(kit::dialog_frame(ctx)).show(ctx, |ui| {
        kit::apply_style(ui);
        ui.set_width((ctx.screen_rect().width() - 50.0).clamp(250.0, 740.0));
        ui.heading("Review collection cleanup");
        let browsing = matches!(review.phase, Phase::Browse);
        let confirming = matches!(review.phase, Phase::Confirm);
        let footer = if browsing && review.report.as_ref().is_some_and(|r| r.can_apply) { 235.0 } else { 145.0 };
        let body_height = (ctx.screen_rect().height() - footer).clamp(60.0, 650.0);
        // Give the area its final viewport immediately when the asynchronous scan returns.
        let min_height = if browsing || confirming { body_height } else { 60.0 };
        egui::ScrollArea::vertical().id_salt(("cleanup_body", confirming)).max_height(body_height).min_scrolled_height(min_height).show(ui, |ui| {
            ui.label(format!("Active recordbox library · {}", review.library.display()));
            match &review.phase {
                Phase::Start => {}
                Phase::Working { applying, .. } => {
                    ui.spinner();
                    ui.label(if *applying { "Rechecking references, verifying the backup and moving reviewed records…" } else { "Checking collection references…" });
                    if !*applying && ui.add_enabled(!review.cancel_scan.load(Ordering::Relaxed), btn("Cancel scan")).clicked() {
                        review.cancel_scan.store(true, Ordering::Relaxed);
                    }
                }
                Phase::Browse => review.browse(ui),
                Phase::Confirm => review.confirm(app, ui),
                Phase::Done(backup) => {
                    ui.label(format!("Moved {} reference(s) to Removed Tracks.", review.selected.len()));
                    ui.label(format!("Verified recovery backup · {}", backup.display()));
                    ui.label("To recover individual records, right-click All in Collection and choose Restore Removed Tracks within 30 days. To restore the backup, open Preferences → Advanced → Database → Restore Library.");
                }
                Phase::Failed(error) => {
                    ui.colored_label(CAUTION_TX, error);
                    if ui.add(btn("Rescan library")).clicked() { review.scan(ctx, false); }
                }
            }
        });
        if matches!(review.phase, Phase::Browse) {
            review.browse_actions(ui);
        }
        ui.add_enabled(!matches!(review.phase, Phase::Working { .. }), btn("Close")).clicked()
    });
    if !response.inner || applying {
        app.ext.cleanup = Some(review);
    }
}
