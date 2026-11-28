//! Windows: Preferences, track Property editor, intelligent playlist editor, export to device,
//! confirmations.

use eframe::egui;
use crate::app::*;
use crate::model::*;
use crate::onetagger::Provider;
use crate::smart::*;
use crate::theme::*;
use crate::ui_kit::{self as kit, btn};
use egui::{vec2, Context, RichText, Ui};

pub fn library_dialogs(app: &mut App, ctx: &Context) {
    crate::ui_prefs::extra_windows(app, ctx);
    crate::ui_sync::show(app, ctx);
    match_lookup_window(app, ctx);
    info_window(app, ctx);
    smart_window(app, ctx);
    export_window(app, ctx);
    confirm_windows(app, ctx);
}

pub(crate) enum MatchLookupDialog {
    Selection { ids: Vec<i64>, analyze_audio: bool, providers: Vec<Provider> },
    TooMany(usize),
    Progress,
}

impl MatchLookupDialog {
    fn for_selection(ids: &[i64]) -> Self {
        match ids.len() {
            0 => Self::Progress,
            n if n > MATCH_LOOKUP_LIMIT => Self::TooMany(n),
            _ => Self::Selection { ids: ids.to_vec(), analyze_audio: false, providers: vec![Provider::MusicBrainz] },
        }
    }

    fn can_start(&self, cli_selected: bool, running: bool) -> bool {
        cli_selected && !running && matches!(self, Self::Selection { providers, .. } if !providers.is_empty())
    }
}

pub fn open_match_lookup(app: &mut App, ids: &[i64]) {
    if app.match_enrichment_running() {
        app.dialogs.match_lookup = Some(MatchLookupDialog::Progress);
    } else if !matches!(&app.dialogs.match_lookup, Some(MatchLookupDialog::Selection { ids: current, .. }) if current == ids) {
        app.dialogs.match_lookup = Some(MatchLookupDialog::for_selection(ids));
    }
}

fn match_lookup_window(app: &mut App, ctx: &Context) {
    let Some(mut dialog) = app.dialogs.match_lookup.take() else { return };
    let mut open = true;
    let mut close = false;
    let mut start = false;
    let running = app.match_enrichment_running();
    kit::dialog(ctx, "Related track metadata").open(&mut open).default_width(440.0).show(ctx, |ui| {
        kit::apply_style(ui);
        match &mut dialog {
            MatchLookupDialog::Selection { ids, analyze_audio, providers } => {
                let count = ids.len();
                let noun = if count == 1 { "track" } else { "tracks" };
                ui.label(format!("{count} selected {noun}"));
                ui.label("OneTagger sends only the catalog artist and title, including any mix name, to each service you select. Matching evidence is saved in this library. Audio file tags stay unchanged.");
                for provider in Provider::ALL {
                    let mut selected = providers.contains(&provider);
                    let label = match provider {
                        Provider::MusicBrainz => "MusicBrainz · recording genres",
                        Provider::Itunes => "Apple iTunes (US) · song genre",
                        Provider::Deezer => "Deezer · release genres and label",
                    };
                    if ui.checkbox(&mut selected, label).changed() {
                        if selected {
                            providers.push(provider);
                        } else {
                            providers.retain(|p| *p != provider);
                        }
                    }
                }
                if providers.is_empty() {
                    ui.label("Choose at least one service to look up tags.");
                } else {
                    ui.label("Each selected service can add evidence. Saved results are reused. An unavailable service pauses while the others continue.")
                        .on_hover_text("Each service has its own request interval. A later lookup can retry an unavailable service or a track that had no match.");
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Choose OneTagger CLI…").clicked() {
                        let picker = rfd::FileDialog::new().set_title("Choose the OneTagger CLI executable");
                        #[cfg(windows)]
                        let picker = picker.add_filter("Executable", &["exe"]);
                        if let Some(path) = picker.pick_file() {
                            if path.is_absolute() && path.is_file() {
                                app.dialogs.onetagger_cli = Some(path);
                            } else {
                                app.toast("Select the installed OneTagger CLI executable.");
                            }
                        }
                    }
                    if let Some(path) = &app.dialogs.onetagger_cli {
                        ui.label(path.file_name().unwrap_or_default().to_string_lossy().as_ref()).on_hover_text(path.display().to_string());
                    }
                });
                ui.label("Apple iTunes and Deezer require OneTagger CLI 2.4.0 or later.");
                ui.hyperlink_to("Get OneTagger CLI", "https://github.com/fantuz/onetagger/releases");
                ui.checkbox(analyze_audio, "Also run local track analysis")
                    .on_hover_text("Queues Analyze Track using your current analysis settings. The lookup runs separately.");
            }
            MatchLookupDialog::TooMany(n) => {
                ui.label(format!("{n} tracks selected. Choose up to {MATCH_LOOKUP_LIMIT} tracks for each lookup."));
            }
            MatchLookupDialog::Progress => {}
        }
        let status = app.match_enrichment_status();
        if !status.is_empty() {
            ui.separator();
            if matches!(dialog, MatchLookupDialog::Progress) {
                ui.label(status);
            } else {
                ui.collapsing("Last lookup in this library", |ui| { ui.label(status); });
            }
        }
        ui.horizontal(|ui| {
            if let MatchLookupDialog::Selection { analyze_audio, .. } = &dialog {
                let label = if *analyze_audio { "Look up and analyze" } else { "Look up tags" };
                start = ui.add_enabled(dialog.can_start(app.dialogs.onetagger_cli.is_some(), running), btn(label)).clicked();
            }
            if running && ui.add(btn("Cancel lookup")).clicked() {
                app.cancel_match_enrichment();
            }
            close = ui.add(btn("Close")).clicked();
        });
    });
    if start {
        if let (MatchLookupDialog::Selection { ids, analyze_audio, providers }, Some(cli)) = (&dialog, app.dialogs.onetagger_cli.clone()) {
            app.queue_match_enrichment(ids, &cli, providers);
            if *analyze_audio {
                app.reanalyze(ids);
            }
            dialog = MatchLookupDialog::Progress;
        }
    }
    if open && !close {
        app.dialogs.match_lookup = Some(dialog);
    }
}

#[cfg(test)]
mod match_lookup_dialog_tests {
    use super::*;

    #[test]
    fn every_selection_starts_metadata_only_and_large_requests_are_never_truncated() {
        assert!(matches!(MatchLookupDialog::for_selection(&[]), MatchLookupDialog::Progress));
        let ids: Vec<_> = (1..=MATCH_LOOKUP_LIMIT as i64).collect();
        let selection = MatchLookupDialog::for_selection(&ids);
        assert!(selection.can_start(true, false));
        assert!(!selection.can_start(false, false));
        assert!(!selection.can_start(true, true));
        assert!(matches!(selection, MatchLookupDialog::Selection { ids: saved, analyze_audio: false, providers }
            if saved == ids && providers == [Provider::MusicBrainz]));
        let too_many = vec![0; MATCH_LOOKUP_LIMIT + 1];
        assert!(matches!(MatchLookupDialog::for_selection(&too_many), MatchLookupDialog::TooMany(n) if n == too_many.len()));
    }

    #[test]
    fn empty_services_never_start_and_each_new_request_has_its_own_opt_in() {
        let mut first = MatchLookupDialog::for_selection(&[1]);
        if let MatchLookupDialog::Selection { providers, .. } = &mut first {
            providers.clear();
        }
        assert!(!first.can_start(true, false));
        if let MatchLookupDialog::Selection { providers, .. } = &mut first {
            providers.extend([Provider::Deezer, Provider::Itunes]);
        }
        assert!(first.can_start(true, false));
        assert!(matches!(MatchLookupDialog::for_selection(&[2]), MatchLookupDialog::Selection { providers, .. }
            if providers == [Provider::MusicBrainz]));
        assert!(!MatchLookupDialog::Progress.can_start(true, false));
    }
}

fn info_window(app: &mut App, ctx: &Context) {
    let Some(mut t) = app.dialogs.info_track.clone() else { return };
    // The track as it was when the window opened (for this track), so only fields edited here
    // are saved. One slot: switching the window to another track starts a new snapshot.
    let orig_id = egui::Id::new("prop_orig");
    let orig: Track = match ctx.data(|d| d.get_temp::<Track>(orig_id)).filter(|o| o.id == t.id) {
        Some(o) => o,
        None => {
            ctx.data_mut(|d| d.insert_temp(orig_id, t.clone()));
            t.clone()
        }
    };
    let mut open = true;
    let mut save = false;
    kit::dialog(ctx, "Property").open(&mut open).default_width(460.0).show(ctx, |ui| {
        kit::apply_style(ui);
        egui::Grid::new("prop").num_columns(2).spacing(vec2(10.0, 5.0)).show(ui, |ui| {
            let field = |ui: &mut Ui, label: &str, v: &mut String| {
                ui.label(RichText::new(label).color(INFO_LBL));
                ui.add(egui::TextEdit::singleline(v).desired_width(320.0));
                ui.end_row();
            };
            field(ui, "Track Title", &mut t.title);
            field(ui, "Artist", &mut t.artist);
            field(ui, "Album", &mut t.album);
            field(ui, "Album Artist", &mut t.album_artist);
            field(ui, "Genre", &mut t.genre);
            field(ui, "Label", &mut t.label);
            field(ui, "Remixer", &mut t.remixer);
            field(ui, "Original Artist", &mut t.original_artist);
            field(ui, "Composer", &mut t.composer);
            field(ui, "Lyricist", &mut t.lyricist);
            field(ui, "Mix Name", &mut t.mix_name);
            field(ui, "Grouping", &mut t.grouping);
            field(ui, "Comments", &mut t.comment);
            field(ui, "Message", &mut t.message);
            field(ui, "Release Date", &mut t.release_date);
            ui.label(RichText::new("Year / Track / Disc").color(INFO_LBL));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut t.year).range(0..=9999));
                ui.add(egui::DragValue::new(&mut t.track_no).range(0..=999));
                ui.add(egui::DragValue::new(&mut t.disc_no).range(0..=99));
            });
            ui.end_row();
            ui.label(RichText::new("Key").color(INFO_LBL));
            let alnum = app.prefs.key_alnum;
            kit::combo_plain("propkey").selected_text(if t.key < 0 { "—".to_string() } else { key_name(t.key, alnum) }).show_ui(ui, |ui| {
                ui.selectable_value(&mut t.key, -1, "—");
                for k in 0..24 {
                    ui.selectable_value(&mut t.key, k, format!("{} ({})", key_name(k, alnum), key_name(k, !alnum)));
                }
            });
            ui.end_row();
            ui.label(RichText::new("Rating").color(INFO_LBL));
            ui.add(egui::Slider::new(&mut t.rating, 0..=5));
            ui.end_row();
            ui.label(RichText::new("Color").color(INFO_LBL));
            kit::combo_plain("propcolor").selected_text(app.prefs.color_name(t.color)).show_ui(ui, |ui| {
                for i in 0..9u8 {
                    ui.selectable_value(&mut t.color, i, app.prefs.color_name(i));
                }
            });
            ui.end_row();
            ui.label(RichText::new("Location").color(INFO_LBL));
            ui.label(RichText::new(&t.path).size(10.5).color(DIM));
            ui.end_row();
            ui.label(RichText::new("File").color(INFO_LBL));
            ui.label(RichText::new(format!("{} · {} kbps · {} Hz · {:.1} MB · {}", t.file_type.to_uppercase(), t.bitrate, t.sample_rate, t.file_size as f64 / 1e6, fmt_time(t.duration, false))).size(10.5).color(DIM));
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Changes are saved in your library only; the audio file is not modified.").size(10.5).color(DIM));
        match kit::dialog_buttons(ui, &["OK", "Cancel"]) {
            Some(0) => save = true,
            Some(_) => app.dialogs.info_track = None,
            None => {}
        }
    });
    if save {
        let id = t.id;
        if let Some(cur) = app.track_mut(id) {
            // Apply only what was edited in this window onto the track as it is now (analysis
            // or a sync may have changed other fields meanwhile).
            macro_rules! take {
                ($($f:ident),*) => { $( if t.$f != orig.$f { cur.$f = t.$f.clone(); } )* };
            }
            take!(title, artist, album, album_artist, genre, label, remixer, original_artist, composer, lyricist, mix_name, grouping, comment, message, release_date, year, track_no, disc_no, rating, color);
            if t.key != orig.key {
                cur.set_key(t.key);
                cur.key_manual = t.key >= 0;
            }
        }
        app.save_track(id);
        app.dialogs.info_track = None;
        ctx.data_mut(|d| d.remove::<Track>(orig_id));
    } else if !open || app.dialogs.info_track.is_none() {
        app.dialogs.info_track = None;
        ctx.data_mut(|d| d.remove::<Track>(orig_id));
    } else {
        app.dialogs.info_track = Some(t);
    }
}

fn smart_window(app: &mut App, ctx: &Context) {
    let Some((id, parent, mut name, mut rule)) = app.dialogs.smart_edit.clone() else { return };
    let mut open = true;
    let mut done: Option<bool> = None;
    let title = if id.is_some() { "Edit the Intelligent Playlist" } else { "Create New Intelligent Playlist" };
    kit::dialog(ctx, title).open(&mut open).default_width(640.0).show(ctx, |ui| {
        kit::apply_style(ui);
        ui.horizontal(|ui| {
            ui.label("List Name");
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(300.0));
        });
        ui.horizontal(|ui| {
            ui.label("Match the following condition:");
            kit::combo_plain("matchall").selected_text(if rule.match_all { "Match all of the items" } else { "Match any of the items" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut rule.match_all, true, "Match all of the items");
                ui.selectable_value(&mut rule.match_all, false, "Match any of the items");
            });
            ui.label("");
        });
        ui.separator();
        let mut remove = None;
        let alnum = app.prefs.key_alnum;
        let tags: Vec<MyTag> = app.mytags.clone();
        for (i, c) in rule.conditions.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                kit::combo_plain(("field", i)).width(130.0).selected_text(c.field.label()).show_ui(ui, |ui| {
                    for f in FIELDS {
                        if ui.selectable_label(c.field == f, f.label()).clicked() {
                            c.field = f;
                            let ops = Op::for_type(f.ftype());
                            if !ops.contains(&c.op) {
                                c.op = ops[0];
                            }
                            c.value.clear();
                            c.value2.clear();
                        }
                    }
                });
                let ops = Op::for_type(c.field.ftype());
                kit::combo_plain(("op", i)).width(120.0).selected_text(c.op.label()).show_ui(ui, |ui| {
                    for o in ops {
                        ui.selectable_value(&mut c.op, *o, o.label());
                    }
                });
                match c.field.ftype() {
                    FieldType::Key => {
                        let k = parse_key(&c.value);
                        kit::combo_plain(("kv", i)).selected_text(if k < 0 { "—".to_string() } else { key_name(k, alnum) }).show_ui(ui, |ui| {
                            for k in 0..24 {
                                if ui.selectable_label(false, key_name(k, alnum)).clicked() {
                                    c.value = key_name(k, false);
                                }
                            }
                        });
                    }
                    FieldType::Color => {
                        let v = c.value.parse::<usize>().unwrap_or(0).min(8);
                        kit::combo_plain(("cv", i)).selected_text(app.prefs.color_name(v as u8)).show_ui(ui, |ui| {
                            for k in 0..9 {
                                if ui.selectable_label(v == k, app.prefs.color_name(k as u8)).clicked() {
                                    c.value = k.to_string();
                                }
                            }
                        });
                    }
                    FieldType::Tag => {
                        let cur = c.value.parse::<i64>().unwrap_or(-1);
                        let label = tags.iter().find(|t| t.id == cur).map(|t| t.name.clone()).unwrap_or("—".into());
                        kit::combo_plain(("tv", i)).selected_text(label).show_ui(ui, |ui| {
                            for t in tags.iter().filter(|t| t.parent != 0) {
                                if ui.selectable_label(cur == t.id, &t.name).clicked() {
                                    c.value = t.id.to_string();
                                }
                            }
                        });
                    }
                    FieldType::Rating => {
                        let mut v = c.value.parse::<i32>().unwrap_or(0);
                        if ui.add(egui::DragValue::new(&mut v).range(0..=5).suffix(" ★")).changed() {
                            c.value = v.to_string();
                        }
                        if c.op == Op::InRange {
                            ui.label("to");
                            let mut v2 = c.value2.parse::<i32>().unwrap_or(5);
                            if ui.add(egui::DragValue::new(&mut v2).range(0..=5).suffix(" ★")).changed() {
                                c.value2 = v2.to_string();
                            }
                        }
                    }
                    _ => {
                        let hint = match c.field.ftype() {
                            FieldType::Date => "YYYY-MM-DD",
                            FieldType::Number if c.field == Field::Time => "mm:ss",
                            _ => "",
                        };
                        if matches!(c.op, Op::InLast | Op::NotInLast) {
                            ui.add(egui::TextEdit::singleline(&mut c.value).desired_width(50.0));
                            kit::combo_plain(("unit", i)).width(80.0).selected_text(if c.months { "month(s)" } else { "day(s)" }).show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.months, false, "day(s)");
                                ui.selectable_value(&mut c.months, true, "month(s)");
                            });
                        } else {
                            ui.add(egui::TextEdit::singleline(&mut c.value).desired_width(140.0).hint_text(hint));
                            if c.op == Op::InRange {
                                ui.label("to");
                                ui.add(egui::TextEdit::singleline(&mut c.value2).desired_width(120.0).hint_text(hint));
                            }
                        }
                    }
                }
                if ui.add(btn("−").small()).clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            rule.conditions.remove(i);
        }
        if ui.add(btn("+ Add condition").small()).clicked() {
            rule.conditions.push(Condition::default());
        }
        ui.separator();
        kit::checkbox_raw(ui, &mut rule.auto_update, "Update automatically");
        let n = app.smart_tracks(&rule).len();
        ui.label(RichText::new(format!("{n} track(s) match now")).color(DIM));
        match kit::dialog_buttons(ui, &["OK", "Cancel"]) {
            Some(0) => done = Some(true),
            Some(_) => done = Some(false),
            None => {}
        }
    });
    match done {
        Some(true) => {
            let nm = if name.trim().is_empty() { "Untitled Intelligent List".to_string() } else { name.trim().to_string() };
            match id {
                Some(id) => {
                    let _ = app.db.rename_node(id, &nm);
                    let _ = app.db.set_smart(id, &rule);
                }
                None => {
                    let _ = app.db.create_node(parent, &nm, NodeKind::Smart, Some(&rule));
                }
            }
            app.reload_playlists();
            app.dialogs.smart_edit = None;
        }
        Some(false) => app.dialogs.smart_edit = None,
        None => {
            if open {
                app.dialogs.smart_edit = Some((id, parent, name, rule));
            } else {
                app.dialogs.smart_edit = None;
            }
        }
    }
}

fn export_window(app: &mut App, ctx: &Context) {
    // Device windows (Property, Sync Manager, import progress, confirmations): device_ext.rs.
    crate::device_ext::windows(app, ctx);
    if !app.dialogs.export_open {
        return;
    }
    let mut open = true;
    kit::dialog(ctx, "Export to device").open(&mut open).default_width(560.0).show(ctx, |ui| {
        kit::apply_style(ui);
        if let Some(job) = &app.export_job {
            let (d, t, txt) = job.last.clone();
            ui.label(RichText::new("Write to the device").font(kit::inter_bold(14.0)).color(WHITE));
            ui.label(kit::text("Exporting", 12.0).color(DIM2));
            ui.add(egui::ProgressBar::new(if t > 0 { d as f32 / t as f32 } else { 0.0 }).text(format!("{d} / {t}  ({} remaining)", t.saturating_sub(d))));
            ui.label(RichText::new(txt).size(11.0).color(DIM));
            if ui.add(btn("Stop")).clicked() {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            ui.ctx().request_repaint();
            return;
        }
        // What the last export did, until it is dismissed.
        if let Some(rep) = app.export_report.clone() {
            ui.label(RichText::new("Playlist export complete").font(kit::inter_bold(14.0)).color(WHITE));
            ui.label(format!("{} track(s) copied to the device, {} already there updated. The device now has {} tracks and {} playlists.", rep.copied, rep.updated, rep.tracks_on_device, rep.playlists_on_device));
            if rep.removed_lists > 0 || rep.removed_tracks > 0 {
                ui.label(format!("Removed from the device: {} list(s), {} track(s).", rep.removed_lists, rep.removed_tracks));
            }
            if !rep.skipped.is_empty() {
                ui.label(RichText::new(format!("{} track(s) were not exported:", rep.skipped.len())).color(CAUTION_TX));
                egui::ScrollArea::vertical().id_salt("export_skipped").max_height(140.0).show(ui, |ui| {
                    for s in &rep.skipped {
                        ui.label(RichText::new(s).size(11.0));
                    }
                });
            }
            if ui.add(btn("OK")).clicked() {
                app.export_report = None;
            }
            ui.separator();
        }
        ui.horizontal(|ui| {
            ui.label("Device");
            let cur = app.dialogs.export_root.clone().unwrap_or_default();
            let label = app.devices.iter().find(|d| d.root == cur).map(|d| format!("{} ({})", d.name, App::fs_label(&d.fs))).unwrap_or("Select a device".into());
            kit::combo_plain("exdev").width(300.0).selected_text(label).show_ui(ui, |ui| {
                for d in &app.devices {
                    if ui.selectable_label(cur == d.root, format!("{} ({})", d.name, App::fs_label(&d.fs))).clicked() {
                        app.dialogs.export_root = Some(d.root.clone());
                    }
                }
            });
            if ui.add(btn("Refresh")).clicked() {
                app.scan_devices();
            }
        });
        if app.devices.is_empty() {
            ui.label(RichText::new("No USB drive or SD card found. Connect one, or add a folder as a device in the Devices view.").color(CAUTION_TX));
        }
        let target = app.devices.iter().find(|d| Some(&d.root) == app.dialogs.export_root.as_ref());
        let blocker = target.map(|d| d.write_blocker.as_deref()).unwrap_or(Some("Select a connected device.")).map(str::to_owned);
        if let Some(d) = target {
            ui.label(format!("{} · {}", d.catalog_label(), d.root));
            if let (Some(f), Some(t)) = (d.free, d.total) {
                ui.label(RichText::new(format!("Available Space {:.1} GB of {:.1} GB", f as f64 / 1e9, t as f64 / 1e9)).size(11.0).color(DIM));
            }
            if d.fs == "apfs" {
                ui.label(RichText::new("This drive is formatted as APFS, which DJ players cannot read. Use FAT32 or exFAT.").color(CAUTION_TX));
            }
            if let Some(reason) = &blocker {
                ui.label(RichText::new(reason).color(CAUTION_TX));
            } else if d.layout == Some(crate::export::usb::DeviceLayout::Mixed) {
                ui.label(RichText::new("Only the classic catalog is updated. Existing shared audio, artwork, analysis and player settings are preserved. OneLibrary players use their own catalog.").size(11.0).color(CAUTION_TX));
            } else if d.lib.is_none() {
                ui.label("Export creates a classic Device Library in this folder.");
            }
            if let Some(l) = &d.lib {
                ui.label(RichText::new(format!("Classic catalog: {} tracks, {} playlists.", l.tracks.len(), l.nodes.iter().filter(|n| !n.folder).count())).size(11.0).color(DIM));
            }
        } else if app.dialogs.export_root.is_some() {
            ui.label(RichText::new("The selected device is no longer connected.").color(CAUTION_TX));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Export").font(kit::inter_bold(13.0)).color(WHITE));
            if kit::radio_raw(ui, !app.device_ui.export_tracks_only, "Selected playlists with their folders").on_hover_text("The ticked playlists and folders are created on the device, with their tracks.").clicked() {
                app.device_ui.export_tracks_only = false;
            }
            if kit::radio_raw(ui, app.device_ui.export_tracks_only, "Tracks only").on_hover_text("Only the tracks of the ticked playlists go to the device (its track list); no playlists are created or changed.").clicked() {
                app.device_ui.export_tracks_only = true;
            }
        });
        ui.label(RichText::new("Playlists to export").font(kit::inter_bold(13.0)).color(WHITE));
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            fn rows(app: &mut App, ui: &mut Ui, parent: i64, depth: usize) {
                let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == parent).cloned().collect();
                for n in kids {
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 16.0);
                        let mut on = app.dialogs.export_sel.contains(&n.id);
                        let count = match n.kind {
                            NodeKind::Folder => String::new(),
                            NodeKind::Smart => format!(" ({})", n.smart.as_ref().map(|r| app.smart_tracks(r).len()).unwrap_or(0)),
                            NodeKind::Playlist => format!(" ({})", app.node_counts.get(&n.id).copied().unwrap_or(0)),
                        };
                        let icon = match n.kind {
                            NodeKind::Folder => "▶",
                            NodeKind::Smart => "⚙",
                            NodeKind::Playlist => "☰",
                        };
                        if kit::checkbox_raw(ui, &mut on, &format!("{icon} {}{count}", n.name)).changed() {
                            if on {
                                app.dialogs.export_sel.insert(n.id);
                            } else {
                                app.dialogs.export_sel.remove(&n.id);
                            }
                        }
                    });
                    if n.kind == NodeKind::Folder {
                        rows(app, ui, n.id, depth + 1);
                    }
                }
            }
            rows(app, ui, 0, 0);
            if app.nodes.is_empty() {
                ui.label(RichText::new("No playlists yet. Create playlists, or export selected tracks.").color(DIM));
            }
        });
        if !app.dialogs.export_tracks.is_empty() {
            ui.label(format!("Plus {} selected track(s) (added to the device's track list)", app.dialogs.export_tracks.len()));
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Music files are copied to the device; your originals are only read. Tracks that are not analysed yet are analysed first.").size(10.5).color(DIM));
        ui.horizontal(|ui| {
            let can = blocker.is_none() && (!app.dialogs.export_sel.is_empty() || !app.dialogs.export_tracks.is_empty());
            if ui.add_enabled(can, btn("Export").min_size(vec2(96.0, 24.0))).on_disabled_hover_text(blocker.as_deref().unwrap_or("Choose at least one playlist or track to export.")).clicked() {
                let root = app.dialogs.export_root.clone().unwrap();
                let sel: Vec<i64> = app.dialogs.export_sel.iter().copied().collect();
                let tracks = app.dialogs.export_tracks.clone();
                // Free space is checked first; the reason is shown when there is not enough.
                app.export_from_dialog(&root, &sel, &tracks);
            }
            if ui.add(btn("Close").min_size(vec2(96.0, 24.0))).clicked() {
                app.dialogs.export_open = false;
            }
        });
    });
    if !open {
        app.dialogs.export_open = false;
    }
}

fn confirm_windows(app: &mut App, ctx: &Context) {
    let center = egui::Align2::CENTER_CENTER;
    if let Some((id, name, folder)) = app.dialogs.confirm_delete_node.clone() {
        let mut answer = None;
        kit::dialog(ctx, if folder { "Delete Folder" } else { "Delete Playlist" }).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(360.0);
            kit::message(ui, if folder { "Are you sure you want to delete this folder?" } else { "Are you sure you want to delete this playlist?" });
            kit::message_note(ui, &format!("\"{name}\". {}", if folder { "The folder and every playlist inside it are deleted. The tracks stay in the collection." } else { "The tracks stay in the collection." }));
            answer = kit::dialog_buttons(ui, &["OK", "Cancel"]);
        });
        if answer == Some(0) {
            let _ = app.db.delete_node(id);
            if app.view == View::Playlist(id) {
                app.set_view(View::Collection);
            }
            app.reload_playlists();
        }
        if answer.is_some() {
            app.dialogs.confirm_delete_node = None;
        }
    }
    if let Some(ids) = app.dialogs.confirm_remove.clone() {
        let mut answer = None;
        kit::dialog(ctx, "Remove from Collection").resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(400.0);
            kit::message(ui, if ids.len() == 1 { "Are you sure you want to remove the selected track(s) from your Collection?" } else { "Are you sure you want to remove the selected tracks?" });
            kit::message_note(ui, &format!("{} track(s). The music files are not deleted. For 30 days the tracks, with their cues, grids, tags and playlist positions, can be brought back with Track > Restore Removed Tracks.", ids.len()));
            answer = kit::dialog_buttons(ui, &["OK", "Cancel"]);
        });
        if answer == Some(0) {
            app.remove_from_collection(&ids);
        }
        if answer.is_some() {
            app.dialogs.confirm_remove = None;
        }
    }
    if let Some((root, id, name)) = app.dialogs.device_remove.clone() {
        let mut answer = None;
        let del = app.prefs.delete_device_tracks;
        let mixed = app.devices.iter().find(|d| d.root == root).is_some_and(|d| d.layout == Some(crate::export::usb::DeviceLayout::Mixed));
        kit::dialog(ctx, "Delete Playlist").id(egui::Id::new("Delete from device")).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(400.0);
            kit::message(ui, "Are you sure you want to delete this playlist?");
            kit::message_note(ui, &format!("\"{name}\" on the device. {}", if mixed && del { "Tracks used by no other classic playlist are removed from the classic catalog. Their files remain on the drive for OneLibrary." } else if del { "Its tracks that are in no other playlist are also deleted from the device." } else { "Tracks stay on the device (change this in Preferences > DJ System > Device)." }));
            answer = kit::dialog_buttons(ui, &["OK", "Cancel"]);
        });
        if answer == Some(0) {
            if app.export_job.is_some() {
                app.toast_err("Wait for the export to finish before changing the device.");
            } else {
                match crate::export::usb::remove_node(std::path::Path::new(&root), id, del) {
                    Ok(_) => app.toast(format!("Deleted \"{name}\" from the device")),
                    Err(e) => app.toast_err(format!("Could not change the device: {e}")),
                }
            }
            app.scan_devices();
            app.set_view(View::Device { root: root.clone(), node: None });
        }
        if answer.is_some() {
            app.dialogs.device_remove = None;
        }
    }
    if let Some((root, name, files)) = app.dialogs.device_cleanup.clone() {
        let mut close = false;
        let total: u64 = files.iter().map(|f| f.1).sum();
        kit::dialog(ctx, "Clean Up Unused Files").resizable(true).default_width(560.0).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.label(format!("{} file(s) on {name} ({:.1} MB) are not used by any track in its library.", files.len(), total as f64 / 1e6));
            ui.label(RichText::new("They were left by deleted playlists, other programs or copies made by hand. Deleting them cannot be undone.").size(11.0).color(DIM));
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for (p, sz) in files.iter().take(500) {
                    let rel = p.strip_prefix(&root).map(|r| r.to_string_lossy().to_string()).unwrap_or_default();
                    ui.label(RichText::new(format!("{rel}  ({:.1} MB)", *sz as f64 / 1e6)).size(11.0));
                }
                if files.len() > 500 {
                    ui.label(RichText::new(format!("… and {} more", files.len() - 500)).size(11.0).color(DIM));
                }
            });
            ui.horizontal(|ui| {
                let confirm_id = egui::Id::new("cleanup_confirm");
                let armed: bool = ui.ctx().data(|d| d.get_temp(confirm_id)).unwrap_or(false);
                let label = if armed { format!("Click again to delete {} file(s)", files.len()) } else { format!("Delete {} File(s)", files.len()) };
                if ui.add(btn(label)).clicked() {
                    if armed && app.export_job.is_some() {
                        app.toast_err("Wait for the export to finish before changing the device.");
                    } else if armed {
                        let paths: Vec<std::path::PathBuf> = files.iter().map(|f| f.0.clone()).collect();
                        match crate::export::usb::delete_unused(std::path::Path::new(&root), &paths) {
                            Ok(n) => app.toast(format!("Deleted {n} unused file(s) from {name}.")),
                            Err(e) => app.toast_err(format!("Could not clean up the device: {e}")),
                        }
                        close = true;
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(confirm_id, !armed));
                }
                if ui.add(btn("Cancel")).clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("cleanup_confirm"), false));
                    close = true;
                }
            });
        });
        if close {
            app.dialogs.device_cleanup = None;
            app.scan_devices();
        }
    }
    if app.dialogs.trash.is_some() {
        let mut open = true;
        let mut act: Option<bool> = None;
        let list = app.db.trash_list().unwrap_or_default();
        let picked = app.dialogs.trash.as_mut().unwrap();
        kit::dialog(ctx, "Restore Removed Tracks").open(&mut open).resizable(true).default_width(520.0).show(ctx, |ui| {
            kit::apply_style(ui);
            if list.is_empty() {
                ui.label("No removed tracks. Tracks removed from the collection stay here for 30 days.");
                return;
            }
            ui.label(RichText::new("Tracks removed from the collection in the last 30 days. Restoring brings back their cues, grids, tags and playlist positions.").size(11.0).color(DIM));
            ui.horizontal(|ui| {
                if ui.add(btn("Select All").small()).clicked() {
                    picked.extend(list.iter().map(|e| e.0));
                }
                if ui.add(btn("Select None").small()).clicked() {
                    picked.clear();
                }
            });
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                for (id, title, artist, path, at) in &list {
                    ui.horizontal(|ui| {
                        let mut on = picked.contains(id);
                        if ui.add(kit::Check(&mut on)).changed() {
                            if on {
                                picked.insert(*id);
                            } else {
                                picked.remove(id);
                            }
                        }
                        ui.label(if artist.is_empty() { title.clone() } else { format!("{title} – {artist}") }).on_hover_text(path);
                        let days = (now - at).max(0) / 86_400;
                        ui.label(RichText::new(if days == 0 { "today".to_string() } else { format!("{days} day(s) ago") }).size(11.0).color(DIM));
                    });
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!picked.is_empty(), btn(format!("Restore {}", picked.len()))).on_disabled_hover_text("Tick the tracks to restore.").clicked() {
                    act = Some(true);
                }
                // Two clicks: the entries cannot be restored afterwards.
                let confirm_id = egui::Id::new("trash_confirm");
                let armed: bool = ui.ctx().data(|d| d.get_temp(confirm_id)).unwrap_or(false);
                let label = if armed { format!("Click again to forget {} permanently", picked.len()) } else { "Delete Permanently".to_string() };
                if ui.add_enabled(!picked.is_empty(), btn(label)).on_hover_text("Forget these entries now; they can't be restored afterwards. The music files are not affected.").clicked() {
                    if armed {
                        act = Some(false);
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(confirm_id, !armed));
                }
            });
        });
        let ids: Vec<i64> = app.dialogs.trash.as_ref().map(|p| p.iter().copied().collect()).unwrap_or_default();
        match act {
            Some(true) => {
                app.restore_removed(&ids);
                app.dialogs.trash = Some(Default::default());
            }
            Some(false) => {
                for id in &ids {
                    let _ = app.db.conn.execute("DELETE FROM trash WHERE id=?1", [id]);
                }
                app.dialogs.trash = Some(Default::default());
            }
            None => {}
        }
        if !open {
            app.dialogs.trash = None;
        }
    }
    if app.dialogs.player_edits.is_some() {
        let mut close = None;
        let pe = app.dialogs.player_edits.as_mut().unwrap();
        kit::dialog(ctx, "Cue changes made on players").resizable(false).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.label(format!("{} track(s) on {} have cues that were changed after the last export from recordbox.", pe.items.len(), pe.device));
            ui.label(RichText::new("Importing replaces the track's hot cues and memory cues in recordbox with the ones on the device. The device is not changed.").size(11.0).color(DIM));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for it in pe.items.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.add(kit::Check(&mut it.pick));
                        let hot = it.edit.cues.iter().filter(|c| c.kind == CueKind::Hot).count();
                        let mem = it.edit.cues.len() - hot;
                        ui.label(format!("{} – {}", it.edit.title, it.edit.artist));
                        ui.label(RichText::new(format!("{hot} hot, {mem} memory")).size(11.0).color(DIM));
                        if it.conflict {
                            ui.label(RichText::new("also changed in recordbox").size(11.0).color(CAUTION_TX)).on_hover_text("The cues were edited in recordbox too since the export. Importing replaces those edits.");
                        }
                    });
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let n = pe.items.iter().filter(|i| i.pick).count();
                if ui.add_enabled(n > 0, btn(format!("Import {n}"))).on_disabled_hover_text("Tick the tracks whose cues to import.").clicked() {
                    close = Some(true);
                }
                if ui.add(btn("Not now")).on_hover_text("These changes are not offered again unless they change on the device.").clicked() {
                    close = Some(false);
                }
            });
        });
        if let Some(apply) = close {
            let pe = app.dialogs.player_edits.take().unwrap();
            if apply {
                app.apply_player_edits(&pe);
            } else {
                app.dismiss_player_edits(&pe, false);
            }
        }
    }
    if app.dialogs.about {
        let mut open = true;
        kit::dialog(ctx, "About recordbox").open(&mut open).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(380.0);
            // The logo (mark and wordmark) instead of the name in text.
            ui.add(crate::brand::logo_image(WHITE.c(), 22.0));
            ui.add_space(2.0);
            ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).size(11.0).color(DIM));
            ui.add_space(6.0);
            kit::message(ui, "Library management, track analysis, set preparation and USB export for CDJ/XDJ players.");
            kit::message_note(ui, "Everything stays on this Mac. Your music files and any existing rekordbox library are never modified.");
        });
        app.dialogs.about = open;
    }
}
