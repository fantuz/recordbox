//! The stem export's pieces of the interface (logic in `stem_export.rs`): the EXPORT control
//! of the deck's stem strip, the "Export Stems" items of the track menu, the explanation
//! shown at the first export, the progress line in the footer and the group on Preferences >
//! Extensions > STEMS. Everything is drawn with the kits the surrounding screens use.

use crate::app::*;
use crate::stem_export::{self as sx, Part};
use crate::theme::*;
use crate::ui_kit as kit;
use eframe::egui;
use egui::{pos2, vec2, Align2, Rect, Ui};

/// Each frame: follow the exports and show the explanation while it waits for an answer.
pub fn frame(app: &mut App, ctx: &egui::Context) {
    sx::poll(app);
    explanation(app, ctx);
}

/// A folder as the user knows it ("~/Music/recordbox/Stems").
pub fn shown_dir(p: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(relative) = p.strip_prefix(home) {
            return std::path::Path::new("~").join(relative).display().to_string();
        }
    }
    p.display().to_string()
}

/// The first export's explanation: the tracks are added to the library (and where the files
/// go), the playlist choice when the export was started in a playlist, "Don't show this
/// again", OK / Cancel. Cancel starts nothing.
fn explanation(app: &mut App, ctx: &egui::Context) {
    let Some(mut ask) = app.stemx.ask.take() else { return };
    let dir = shown_dir(&sx::export_dir(&app.prefs.stemx));
    let mut answer = None;
    kit::dialog(ctx, "Export Stems").id(egui::Id::new("stem_export_explain")).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        kit::apply_style(ui);
        ui.set_width(500.0);
        kit::message(ui, "Exported stem tracks are added to the library.");
        kit::message_note(ui, &format!("The export saves a new file in {dir} with the original’s beat grid, cues and track information. It uses completed parts for the selected separation model. Load and separate uncached tracks on a deck first, or export directly from the loaded deck."));
        ui.add_space(12.0);
        match &ask.playlist {
            Some((_, name)) => {
                kit::message_note(ui, &format!("This export was started in the playlist \"{name}\"."));
                ui.add_space(6.0);
                kit::checkbox_raw(ui, &mut ask.add, "Also add exported stem tracks to the playlist they are exported from");
            }
            None => kit::message_note(ui, "When you export from inside a playlist, the new track can also be added to that playlist, right after the original. Change this in Preferences > Advanced > Others (EXPORT) or Extensions > STEMS (PERFORMANCE)."),
        }
        ui.add_space(8.0);
        kit::checkbox_raw(ui, &mut ask.dont_show, "Don't show this again");
        answer = kit::dialog_buttons(ui, &["OK", "Cancel"]);
    });
    match answer {
        Some(i) => sx::answered(app, ask, i == 0),
        None => app.stemx.ask = Some(ask),
    }
}

/// The stem strip's EXPORT control (left of the part buttons, where the strip is empty): a
/// raised button in the strip's own style opening Vocals / Instrumental. `at` places a
/// control by the player's measured x and width.
pub fn strip_control(app: &mut App, ui: &mut Ui, r: Rect) {
    let loaded = app.deck.track.is_some();
    let resp = crate::ui_player::bold_button(ui, r, "stem_export", "EXPORT");
    let resp = crate::ui_player::tip(resp, app, "Export the vocals or the instrumental of this track as a new track");
    if !loaded {
        return;
    }
    crate::ui_browser::dropdown(ui, &resp, "stem_export_pop", |ui: &mut Ui| {
        for (part, label) in [(Part::Vocals, "Vocals"), (Part::Instrumental, "Instrumental")] {
            let b = ui.button(label);
            if b.clicked() {
                ui.memory_mut(|m| m.close_popup());
                if let Some(t) = app.deck.track {
                    sx::request(app, &[t], part, true);
                }
            }
        }
    });
}

/// The strip's status line while an export of the deck's track runs.
pub fn strip_status(app: &App) -> Option<String> {
    if !sx::busy_for(app, app.deck.track) {
        return None;
    }
    sx::status(app).or_else(|| Some("Waiting to export stems…".into()))
}

/// "Export Stems > Vocals / Instrumental" (and Stop) in the track list's menu.
pub fn track_menu(app: &mut App, ui: &mut Ui, sel: &[i64]) {
    if !app.prefs.show_stems {
        return;
    }
    ui.menu_button("Export Stems", |ui| {
        for (part, label) in [(Part::Vocals, "Vocals"), (Part::Instrumental, "Instrumental")] {
            if ui.button(label).clicked() {
                ui.close_menu();
                sx::request(app, sel, part, false);
                // The main window owns the confirmation, including when the library is detached.
                if app.stemx.ask.is_some() && ui.ctx().viewport_id() != egui::ViewportId::ROOT {
                    ui.ctx().send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                    ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                }
            }
        }
        if sx::busy(app) {
            ui.separator();
            if ui.button("Stop Exporting Stems").clicked() {
                ui.close_menu();
                sx::stop(app);
            }
        }
    });
}

/// The footer's progress line while exports run, with "Stop Exporting" as the analysis line
/// has "Stop Analyzing". True when it drew.
pub fn footer(app: &mut App, ui: &mut Ui, p: &egui::Painter, mid: Rect) -> bool {
    let Some(text) = sx::footer_text(app) else { return false };
    p.text(pos2(mid.min.x + 8.0, mid.center().y), Align2::LEFT_CENTER, text, crate::theme::rb(11.5), TEXT.c());
    let stop = ui.interact(Rect::from_min_size(pos2(mid.max.x - 120.0, mid.min.y), vec2(110.0, mid.height())), ui.id().with("stop_stem_export"), egui::Sense::click());
    p.text(pos2(mid.max.x - 64.0, mid.center().y), Align2::CENTER_CENTER, "Stop Exporting", crate::theme::rb(11.0), BLUE.c());
    if stop.clicked() {
        sx::stop(app);
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    true
}

/// Preferences > Extensions > STEMS, at the end under the "recordbox" heading: the playlist
/// rule, the file format, the folder, and bringing the explanation back.
pub fn prefs_group(app: &mut App, ui: &mut Ui) {
    use crate::ui_prefs_item::item;
    let before = app.prefs.stemx.clone();
    let dir = sx::export_dir(&app.prefs.stemx);
    let (mut browse, mut reset) = (false, false);
    item(ui, "RecordboxStemExport", 332.0, |it| {
        it.title_tip("Stem Export", "EXPORT in the deck's STEMS strip, or Track > Export Stems, saves the vocals or the instrumental of a track as a new audio file and adds it to the collection with the original's beat grid, cues and track information.");
        let p = &mut app.prefs.stemx;
        it.pill(52.0, 56.0, &mut p.to_playlist, "Add exported stem tracks to the playlist they are exported from");
        it.sep(96.0);
        it.sub_tip(52.0, 112.0, "File Format", "Same as the original: an MP3 stays an MP3 at its bitrate, FLAC, WAV and AIFF keep their bit depth, an AAC file stays M4A (AAC). Other lossless formats use FLAC. Other lossy formats use AAC on macOS and FLAC elsewhere. AAC encoding requires macOS.\nA lossy original exported to a lossy format is encoded once more.");
        let mut sel = (p.format as usize).min(sx::FORMAT_NAMES.len() - 1);
        if it.choice(84.0, 146.0, 300.0, &mut sel, &sx::FORMAT_NAMES) {
            p.format = sel as u8;
        }
        it.sep(186.0);
        it.sub(52.0, 202.0, "Location of Files");
        it.field(84.0, 236.0, 316.0, &shown_dir(&dir));
        browse = it.button(408.0, 236.0, 96.0, 22.0, "Browse").clicked();
        it.en = !p.dir.is_empty();
        reset = it.button(512.0, 236.0, 96.0, 22.0, "Default").clicked();
        it.en = true;
        it.sep(276.0);
        it.pill(52.0, 292.0, &mut p.explain, "Show the explanation at the next export");
    });
    if browse {
        if let Some(d) = rfd::FileDialog::new().set_title("Location of Files").set_directory(&dir).pick_folder() {
            app.prefs.stemx.dir = d.to_string_lossy().to_string();
        }
    }
    if reset {
        app.prefs.stemx.dir.clear();
    }
    if app.prefs.stemx != before {
        app.save_prefs();
    }
}
