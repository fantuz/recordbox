//! The window shell's mode switch: rekordbox's question before a switch stops playback, and
//! what the header shows per mode (`MainComponent::comboBoxChanged` 0x10097b8dc,
//! `changeModeToPerformance` 0x100986d40, `changeModeToExport` 0x100987be4).
//!
//! rekordbox keeps the panel toggles and the layout of each mode (`ShowPanel_FX`,
//! `ShowPanel_SAMPLER`, `WindowLayout`, `DjLayout`) and puts them back when the mode is entered
//! again. recordbox's header has the FX and sampler panels and the 2/4 deck layout in
//! Performance and the player layout in Export; the two modes share those fields, so they
//! are saved per mode when a mode is left (`RbPrefs::shell`).

use crate::app::*;
use crate::prefs_rb::{Mode, ModeShell};
use eframe::egui;

/// The mode asked for while something plays: waiting for the answer.
static PENDING: parking_lot::Mutex<Option<u8>> = parking_lot::Mutex::new(None);

/// rekordbox's question when a switch away from Performance would stop playback.
pub fn question(to: Mode) -> Option<&'static str> {
    match to {
        Mode::Export => Some("Are you sure you want to stop playing and switch to the Export mode?"),
        Mode::Edit => Some("Are you sure you want to stop playing and switch to Edit mode?"),
        Mode::Lighting => Some("Are you sure you want to stop playing and switch to Lighting mode?"),
        Mode::Performance => None,
    }
}

fn any_deck_playing(app: &App) -> bool {
    app.player.decks.iter().any(|d| d.lock().playing)
}

/// The header's mode list was used: switch, or ask first when a deck is playing and the
/// mode asked for stops it (rekordbox asks for Export, Edit and Lighting).
pub fn request_mode(app: &mut App, mode: u8) {
    if mode == app.prefs.app_mode {
        return;
    }
    if app.performance() && any_deck_playing(app) && question(Mode::of(mode)).is_some() {
        *PENDING.lock() = Some(mode);
    } else {
        switch(app, mode);
    }
}

/// What the header shows now, to be put back when this mode is entered again.
pub fn snapshot(p: &Prefs) -> ModeShell {
    ModeShell { saved: true, fx: p.perf_fx, sampler: p.perf_sampler, four_decks: p.perf_four, player_layout: p.player_layout, browser_only: p.browser_only }
}

/// Puts a mode's saved header state back (nothing is changed for a mode never left).
pub fn restore(p: &mut Prefs, mode: Mode) {
    let s = p.rb.shell[mode.index()].clone();
    if !s.saved {
        return;
    }
    match mode {
        Mode::Performance => {
            p.perf_fx = s.fx;
            p.perf_sampler = s.sampler;
            p.perf_four = s.four_decks;
        }
        _ => {
            p.player_layout = s.player_layout;
            p.browser_only = s.browser_only;
        }
    }
}

/// Leaves the current mode for `mode`: saves its header state, stops playback when the
/// decks are left, switches and restores the new mode's state.
pub fn switch(app: &mut App, mode: u8) {
    let from = Mode::of(app.prefs.app_mode);
    app.prefs.rb.shell[from.index()] = snapshot(&app.prefs);
    if from == Mode::Performance && question(Mode::of(mode)).is_some() {
        for d in app.player.decks.iter() {
            d.lock().playing = false;
        }
    }
    app.set_app_mode(mode);
    let now = Mode::of(app.prefs.app_mode);
    restore(&mut app.prefs, now);
    app.poll_decks();
    app.save_prefs();
}

/// The question's window, while one is waiting.
pub fn confirm_window(app: &mut App, ctx: &egui::Context) {
    let Some(mode) = *PENDING.lock() else { return };
    let Some(text) = question(Mode::of(mode)) else {
        *PENDING.lock() = None;
        return;
    };
    let mut answer = None;
    crate::ui_kit::dialog(ctx, "recordbox").resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
        crate::ui_kit::apply_style(ui);
        ui.set_width(420.0);
        crate::ui_kit::message(ui, text);
        answer = crate::ui_kit::dialog_buttons(ui, &["OK", "Cancel"]);
    });
    if let Some(a) = answer {
        *PENDING.lock() = None;
        if a == 0 {
            switch(app, mode);
        }
    }
}

/// For the scripted checks: whether a question is waiting.
pub fn pending() -> Option<u8> {
    *PENDING.lock()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_gets_its_panels_and_layout_back() {
        let mut p = Prefs::default();
        // Performance with FX and four decks; then Export with the 2 Players layout.
        p.app_mode = 1;
        p.perf_fx = true;
        p.perf_four = true;
        p.rb.shell[1] = snapshot(&p);
        p.app_mode = 0;
        restore(&mut p, Mode::Export);
        assert!(p.perf_fx, "a mode never left changes nothing");
        p.player_layout = 1;
        p.perf_fx = false;
        p.perf_four = false;
        p.rb.shell[0] = snapshot(&p);
        p.app_mode = 1;
        restore(&mut p, Mode::Performance);
        assert!(p.perf_fx && p.perf_four && !p.perf_sampler);
        p.player_layout = 2;
        p.rb.shell[1] = snapshot(&p);
        restore(&mut p, Mode::Export);
        assert_eq!(p.player_layout, 1);
        // The questions are rekordbox's, and Performance itself never asks.
        assert_eq!(question(Mode::Export), Some("Are you sure you want to stop playing and switch to the Export mode?"));
        assert!(question(Mode::Edit).unwrap().ends_with("switch to Edit mode?"));
        assert!(question(Mode::Performance).is_none());
        // The state survives a save.
        let q: Prefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(q.rb.shell, p.rb.shell);
    }
}
