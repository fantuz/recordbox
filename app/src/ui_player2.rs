//! Player UI beyond the 1 Player panel in ui_player.rs: the enlarged waveform's right-click
//! menu, the GRID panel (GRID EDIT and PHRASE EDIT), loop size and beat jump lists, the
//! 2 Players and Dual Player layouts (decks A and B one above the other) and the Simple Player.

use crate::app::*;
use crate::model::*;
use crate::player_ext::{beats_text, JUMP_BARS, JUMP_BEATS, LOOP_SIZES};
use crate::theme::*;
use crate::ui_player::{button, draw_detail_at, draw_note_icon, draw_overview, icon_button, lighten, seg_button, split_time, tip, DetailMode};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui};

/// Height of the Simple Player row.
pub const SIMPLE_HEIGHT: f32 = 59.0;

// ---------------------------------------------------------------------- menus

/// Right-click menu of the enlarged waveform; everything acts at `t` (the clicked position).
pub fn wave_menu(app: &mut App, ui: &mut Ui, t: f64) {
    ui.set_min_width(250.0);
    let has_loop = app.defined_loop().is_some();
    if ui.button("Set Cue(In) Point").clicked() {
        app.set_cue_at(t);
        ui.close_menu();
    }
    if ui.button("Set Out Point").clicked() {
        app.set_out_at(t);
        ui.close_menu();
    }
    ui.separator();
    for slot in 0..4 {
        if ui.button(format!("Set Hot Cue {}", (b'A' + slot as u8) as char)).clicked() {
            app.set_hot_cue_at(slot, t);
            ui.close_menu();
        }
    }
    for slot in 0..4 {
        if ui.add_enabled(has_loop, egui::Button::new(format!("Set Hot Loop {}", (b'A' + slot as u8) as char))).clicked() {
            app.set_hot_loop(slot);
            ui.close_menu();
        }
    }
    if ui.add_enabled(has_loop, egui::Button::new("Set Memory Loop")).clicked() {
        app.set_memory_loop();
        ui.close_menu();
    }
    if ui.add_enabled(has_loop, egui::Button::new("Export Loop As Wav")).clicked() {
        ui.close_menu();
        app.export_loop_wav();
    }
    ui.separator();
    ui.menu_button("Beat Count Display", |ui| {
        if beat_count_items(app, ui) {
            ui.close_menu();
        }
    });
    let mut on = app.prefs.wave_click_play_cue;
    if ui.checkbox(&mut on, "Click on the waveform for PLAY and CUE").clicked() {
        app.prefs.wave_click_play_cue = on;
        app.save_prefs();
        ui.close_menu();
    }
    ui.menu_button("Change waveform color", |ui| {
        if wave_color_items(app, ui) {
            ui.close_menu();
        }
    });
}

/// The three Beat Count Display choices; true when one was picked.
fn beat_count_items(app: &mut App, ui: &mut Ui) -> bool {
    let mut picked = false;
    for (i, name) in ["Current Position (Bars)", "Count to the next MEMORY CUE (Bars)", "Count to the next MEMORY CUE (Beats)"].iter().enumerate() {
        if ui.radio(app.prefs.beat_count_mode == i as u8, *name).clicked() {
            app.prefs.beat_count_mode = i as u8;
            app.save_prefs();
            picked = true;
        }
    }
    picked
}

fn wave_color_items(app: &mut App, ui: &mut Ui) -> bool {
    let mut picked = false;
    for (i, name) in ["BLUE", "RGB", "3Band"].iter().enumerate() {
        if ui.radio(app.prefs.wave_color.min(2) == i as u8, *name).clicked() {
            app.prefs.wave_color = i as u8;
            app.save_prefs();
            picked = true;
        }
    }
    picked
}

/// The rest of the ≡ menu of the CUE/LOOP strip: the waveform menu's settings (reachable
/// here when right-click is CUE), loop export, mute and the Recordbox extras.
pub fn strip_menu_extra(app: &mut App, ui: &mut Ui) {
    ui.set_min_width(250.0);
    let has_loop = app.defined_loop().is_some();
    ui.separator();
    if ui.add_enabled(has_loop, egui::Button::new("Set Memory Loop")).clicked() {
        app.set_memory_loop();
    }
    if ui.add_enabled(has_loop, egui::Button::new("Export Loop As Wav")).clicked() {
        app.export_loop_wav();
    }
    ui.separator();
    ui.label(egui::RichText::new("Beat Count Display").size(10.5).color(DIM));
    beat_count_items(app, ui);
    ui.separator();
    let mut on = app.prefs.wave_click_play_cue;
    if ui.checkbox(&mut on, "Click on the waveform for PLAY and CUE").clicked() {
        app.prefs.wave_click_play_cue = on;
        app.save_prefs();
    }
    ui.separator();
    ui.label(egui::RichText::new("Change waveform color").size(10.5).color(DIM));
    wave_color_items(app, ui);
    ui.separator();
    let mut m = app.px.muted;
    if ui.checkbox(&mut m, "Mute").clicked() {
        app.toggle_mute();
    }
    ui.separator();
    ui.label(egui::RichText::new("recordbox extras").size(10.5).color(DIM));
    let mut changed = ui.checkbox(&mut app.prefs.show_stems, "STEMS button").clicked();
    changed |= ui.checkbox(&mut app.prefs.show_phrase_bar, "Phrase bar").clicked();
    changed |= ui.checkbox(&mut app.prefs.show_vocal, "Vocal positions").clicked();
    if changed {
        app.save_prefs();
    }
}

/// Auto beat loop sizes: picking one starts a loop of that length.
pub fn loop_size_menu(app: &mut App, ui: &mut Ui) {
    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
        for v in LOOP_SIZES {
            if ui.selectable_label(app.prefs.loop_beats == v, format!("{} Beat Loop", beats_text(v))).clicked() {
                app.pick_auto_loop(v);
                ui.memory_mut(|m| m.close_popup());
            }
        }
    });
}

/// Beat jump lengths, in beats and in bars.
pub fn jump_size_menu(app: &mut App, ui: &mut Ui) {
    let pick = |app: &mut App, beats: f64, bars: bool| {
        app.prefs.beat_jump = beats;
        app.prefs.beat_jump_bars = bars;
        app.save_prefs();
    };
    let cur = (app.prefs.beat_jump, app.prefs.beat_jump_bars);
    for v in JUMP_BEATS {
        let l = if v <= 1.0 { format!("{} Beat", beats_text(v)) } else { format!("{v} Beats") };
        if ui.selectable_label(cur == (v, false), l).clicked() {
            pick(app, v, false);
        }
    }
    ui.separator();
    for v in JUMP_BARS {
        if ui.selectable_label(cur == (v * 4.0, true), format!("{} {}", v, if v == 1.0 { "Bar" } else { "Bars" })).clicked() {
            pick(app, v * 4.0, true);
        }
    }
}

/// Manual loop buttons IN, OUT and RELOOP/EXIT. `at(x, w)` places a button `w` wide centred
/// on the 1 Player strip's design x (IN 968, OUT 1005, RELOOP/EXIT 1060). During a loop IN
/// and OUT switch loop adjust for their edge on and off (the button then blinks).
pub fn manual_loop_buttons(app: &mut App, ui: &mut Ui, at: impl Fn(f32, f32) -> Rect) {
    let active = app.loop_active().is_some();
    let adj = app.deck.x.loop_adjust;
    let blink = (ui.input(|i| i.time) * 2.5) as i64 % 2 == 0;
    let fill = |on: bool, adjusting: bool| {
        if adjusting {
            if blink {
                LOOP
            } else {
                LOOP.gamma_multiply(0.4)
            }
        } else if on {
            LOOP.gamma_multiply(0.7)
        } else {
            BTN.c()
        }
    };
    let in_on = active || app.deck.loop_in.is_some();
    if tip(button(ui, at(960.5, 37.0), "lin", "IN", fill(in_on, adj == 1), WHITE.c(), 9.5), app, "Manual Loop:\nSet a Loop In point.").clicked() {
        app.loop_in_pressed();
    }
    if tip(button(ui, at(999.5, 37.0), "lout", "OUT", fill(active, adj == 2), WHITE.c(), 9.5), app, "Manual Loop:\nSet a Loop Out point.").clicked() {
        app.loop_out_pressed();
    }
    if tip(button(ui, at(1055.0, 70.0), "lre", "RELOOP/EXIT", fill(false, false), if active || app.deck.last_loop.is_some() { WHITE.c() } else { OFF.c() }, 9.5), app, "Back to Loop playback (Reloop), or cancel Loop playback (Loop Exit).").clicked() {
        app.exit_reloop();
    }
    if adj != 0 {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// A track dragged from a list and released over `rect` is loaded into the active deck.
pub fn drop_to_load(app: &mut App, ui: &mut Ui, rect: Rect) {
    if app.deck_options.disable_load { return; }
    let Some(ids) = app.drag.clone() else { return };
    let Some(pp) = ui.ctx().pointer_latest_pos() else { return };
    if !rect.contains(pp) {
        return;
    }
    ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0_f32, BLUE), egui::StrokeKind::Inside);
    if ui.input(|i| i.pointer.any_released()) {
        if let Some(id) = ids.first() {
            app.load_track_here(*id);
        }
    }
}

// ---------------------------------------------------------------------- GRID panel

/// Icons of the GRID panel, drawn after the original skin's 30 × 30 pt buttons
/// (Grid_Export_Button_*): `m` maps a point of that 30 pt square into the button.
#[derive(Clone, Copy, PartialEq)]
enum GridIcon {
    MoveLeft,
    MoveRight,
    Narrow,
    Wide,
    Double,
    Half,
    Center,
    Whole,
    FromCurrent,
    Undo,
    Redo,
    Metronome(bool),
    Volume(u8),
    Lock(bool),
}

const SKIN_BLUE: Color32 = hex(0x0096FF);

fn grid_icon(p: &Painter, r: Rect, icon: GridIcon, c: Color32) {
    let k = r.height() / 30.0;
    let m = |x: f32, y: f32| pos2(r.center().x + (x - 15.0) * k, r.center().y + (y - 15.0) * k);
    let tri = |a: Pos2, b: Pos2, d: Pos2, col: Color32| {
        p.add(Shape::convex_polygon(vec![a, b, d], col, Stroke::NONE));
    };
    // Three thin beat lines starting at x (3 pt apart).
    let bars = |x: f32, y0: f32, y1: f32, col: Color32| {
        for i in 0..3 {
            let xx = x + 3.0 * i as f32 + 0.5;
            p.line_segment([m(xx, y0), m(xx, y1)], Stroke::new(1.0_f32, col));
        }
    };
    let grey = gy(0x646464);
    match icon {
        GridIcon::MoveLeft => {
            tri(m(5.0, 15.0), m(10.0, 12.0), m(10.0, 18.0), c);
            bars(15.0, 9.0, 21.0, grey);
        }
        GridIcon::MoveRight => {
            tri(m(25.0, 15.0), m(20.0, 18.0), m(20.0, 12.0), c);
            bars(8.0, 9.0, 21.0, grey);
        }
        GridIcon::Narrow => {
            // Arrows pointing in at the beat lines.
            tri(m(9.0, 15.0), m(4.0, 18.0), m(4.0, 12.0), c);
            tri(m(22.0, 15.0), m(27.0, 12.0), m(27.0, 18.0), c);
            bars(12.0, 9.0, 21.0, grey);
        }
        GridIcon::Wide => {
            tri(m(4.0, 15.0), m(9.0, 12.0), m(9.0, 18.0), c);
            tri(m(27.0, 15.0), m(22.0, 18.0), m(22.0, 12.0), c);
            bars(12.0, 9.0, 21.0, grey);
        }
        GridIcon::Double | GridIcon::Half => {
            // Three beat lines, a multiplication cross and "2" (Double) or "1/2" as a stacked
            // fraction (Half), where the skin's artwork has them.
            let half = icon == GridIcon::Half;
            let dx = if half { -1.0 } else { 0.0 };
            bars(4.0 + dx, 9.0, 21.0, grey);
            let xs = Stroke::new(1.6 * k, c);
            p.line_segment([m(13.9 + dx, 12.4), m(18.1 + dx, 17.6)], xs);
            p.line_segment([m(18.1 + dx, 12.4), m(13.9 + dx, 17.6)], xs);
            if half {
                p.text(m(23.0, 8.3), Align2::CENTER_CENTER, "1", caps(9.8 * k), c);
                p.rect_filled(Rect::from_min_max(m(20.0, 14.0), m(27.0, 15.0)), 0.0, c);
                p.text(m(23.3, 20.8), Align2::CENTER_CENTER, "2", caps(11.2 * k), c);
            } else {
                p.text(m(24.1, 14.0), Align2::CENTER_CENTER, "2", caps(12.0 * k), c);
            }
        }
        GridIcon::Center => {
            // A beat line, white above and red below, as beats are marked on the waveform.
            p.rect_filled(Rect::from_min_max(m(14.0, 7.0), m(16.0, 15.0)), 0.0, c);
            p.rect_filled(Rect::from_min_max(m(14.0, 15.0), m(16.0, 23.0)), 0.0, hex(0xFA0000));
        }
        GridIcon::Whole | GridIcon::FromCurrent => {
            bars(5.0, 10.0, 20.0, if icon == GridIcon::Whole { c } else { gy(0x4B4B4B) });
            bars(18.0, 10.0, 20.0, c);
            tri(m(15.0, 9.0), m(13.0, 6.0), m(17.0, 6.0), grey);
            tri(m(15.0, 21.0), m(17.0, 24.0), m(13.0, 24.0), grey);
        }
        GridIcon::Undo | GridIcon::Redo => {
            // A three-quarter turn with an arrow head at its open end (mirrored for Redo).
            let sgn = if icon == GridIcon::Undo { 1.0 } else { -1.0 };
            let mm = |x: f32, y: f32| m(15.0 + sgn * (x - 15.0), y);
            let mut pts = vec![];
            for i in 0..=16 {
                let a = -std::f32::consts::FRAC_PI_2 + i as f32 / 16.0 * std::f32::consts::PI;
                pts.push(mm(13.6 + a.cos() * 4.6, 15.0 + a.sin() * 4.2));
            }
            // The skin draws Undo / Redo in #969696.
            let c = if c == OFF { c } else { gy(0x969696) };
            p.add(Shape::line(pts, Stroke::new(1.7 * k / 0.8, c)));
            p.add(Shape::convex_polygon(vec![mm(9.0, 10.8), mm(13.6, 7.9), mm(13.6, 13.7)], c, Stroke::NONE));
        }
        GridIcon::Metronome(on) => {
            // A solid body (white; blue while it ticks) with the pendulum cut out of it.
            let col = if c == OFF { c } else if on { SKIN_BLUE } else { WHITE.c() };
            p.add(Shape::convex_polygon(vec![m(11.5, 7.0), m(18.5, 7.0), m(21.0, 23.0), m(9.0, 23.0)], col, Stroke::NONE));
            let cut = Stroke::new(1.6 * k / 0.8, gy(0x282828));
            p.line_segment([m(11.0, 8.0), m(18.6, 18.6)], cut);
            p.line_segment([m(9.5, 18.8), m(20.5, 18.8)], cut);
            p.add(Shape::closed_line(vec![m(12.4, 8.2), m(17.6, 8.2), m(19.4, 18.0), m(10.6, 18.0)], Stroke::new(0.9 * k / 0.8, gy(0x282828))));
        }
        GridIcon::Volume(v) => {
            for (i, (x, y0, y1)) in [(9.0, 12.0, 17.0), (13.0, 10.0, 19.0), (17.0, 8.0, 22.0)].iter().enumerate() {
                p.rect_filled(Rect::from_min_max(m(*x, *y0), m(*x + 2.0, *y1)), 0.0, if c == OFF { c } else if (i as u8) < v { SKIN_BLUE } else { WHITE.c() });
            }
        }
        GridIcon::Lock(on) => {
            let col = if c == OFF { c } else if on { hex(0x007DE1) } else { WHITE.c() };
            let body = Rect::from_min_max(m(9.0, 14.0), m(21.0, 23.0));
            p.rect_filled(body, 1.0, col);
            let mut pts = vec![];
            // Closed shackle when locked, swung open when not.
            let (cx, lift) = if on { (15.0, 0.0) } else { (15.0, -2.5) };
            for i in 0..=12 {
                let a = std::f32::consts::PI + i as f32 * std::f32::consts::PI / 12.0;
                pts.push(m(cx + a.cos() * 4.0, 11.0 + lift + a.sin() * 4.5));
            }
            let mut line = vec![m(cx - 4.0, 14.0)];
            line.extend(pts);
            if on {
                line.push(m(cx + 4.0, 14.0));
            }
            p.add(Shape::line(line, Stroke::new(1.8 * k / 0.8, col)));
            p.circle_filled(m(15.0, 18.0), 1.4 * k / 0.8, Color32::BLACK);
        }
    }
}

/// GRID panel (the strip under the waveform while the GRID tab is selected), as rekordbox
/// 7.2.19 lays it out at 1512 pt. "GRID EDIT": first beat of a bar at 173, the BPM field
/// 207–257, TAP 262, beat grid left / right 296 / 325, expand / shrink 359 / 388, ×2 / ×½
/// 422 / 451, the range of the changes 485 / 514, Undo / Redo 548 / 577, metronome and its
/// volume 605 / 634 and the analysis lock 663; "PHRASE EDIT": CUT 880, the phrase type list
/// 940 and CLEAR 1030; Q and the menu at the right as on the CUE/LOOP panel. Buttons are
/// 24 pt squares centred 24 pt below the strip's top.
pub fn grid_strip(app: &mut App, ui: &mut Ui, s: Rect, main: Rect) {
    let p = ui.painter().clone();
    let y = s.min.y + 24.0;
    let face = BTN;
    let locked = app.deck.track.and_then(|t| app.track(t)).map(|t| t.grid_locked).unwrap_or(false);
    let loaded = app.deck.track.is_some();
    let x_of = |x: f32| crate::ui_player::sx(main, x);
    p.text(pos2(x_of(173.0), s.min.y + 6.0), Align2::LEFT_CENTER, "GRID EDIT", caps(8.8), WHITE.c());
    // One icon button at design x; returns its response. `dim`: drawn as unavailable.
    let ib = |ui: &mut Ui, app: &App, x: f32, id: &str, icon: GridIcon, tipt: &str, dim: bool| -> egui::Response {
        let r = Rect::from_min_size(pos2(x_of(x), y - 12.0), vec2(24.0, 24.0));
        let resp = icon_button(ui, r, id, Some(if dim { gy(0x212121) } else { face.c() }), |p, r, c| grid_icon(p, r.expand2(vec2(2.0, 0.0)), icon, if loaded && !dim { c } else { OFF.c() }));
        tip(resp, app, tipt)
    };
    let fine = ui.input(|i| i.modifiers.shift);
    let fc = app.prefs.grid_from_current;
    if ib(ui, app, 173.0, "gdown", GridIcon::Center, "Set the first beat of a bar to the current position.", false).clicked() {
        app.grid_set_downbeat();
    }
    // BPM: shows the tempo at the playhead; type a value (40–499) and press Enter to set it.
    let r = Rect::from_min_max(pos2(x_of(207.0), y - 12.0), pos2(x_of(257.0), y + 12.0));
    p.rect_filled(r, 2.0, gy(0x0F0F0F));
    if !app.deck.x.bpm_editing {
        let b = app.current_bpm();
        app.deck.bpm_text = if loaded && b > 0.0 { format!("{b:.2}") } else { String::new() };
    }
    let mut txt = app.deck.bpm_text.clone();
    let resp = ui.put(r.shrink2(vec2(3.0, 3.0)), egui::TextEdit::singleline(&mut txt).horizontal_align(egui::Align::Center).frame(false).font(FontId::proportional(11.5)).text_color(WHITE.c()).interactive(loaded));
    app.deck.bpm_text = txt;
    if resp.gained_focus() || resp.has_focus() {
        app.deck.x.bpm_editing = true;
    }
    if resp.lost_focus() {
        app.deck.x.bpm_editing = false;
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let t = app.deck.bpm_text.clone();
            app.grid_enter_bpm(&t);
        }
    }
    let _ = tip(resp, app, "Enter a BPM value");
    let r = Rect::from_min_size(pos2(x_of(262.0), y - 12.0), vec2(24.0, 24.0));
    if tip(button(ui, r, "gtap", "TAP", gy(0x212121), if loaded { gy(0x969696) } else { gy(0x505050) }, 8.5), app, "Set BPM with the tapping interval").clicked() {
        app.grid_tap();
    }
    // Beat grid left / right: 1 ms a click (Shift: 5 ms).
    let step = if fine { 0.005 } else { 0.001 };
    if ib(ui, app, 296.0, "gl", GridIcon::MoveLeft, if fc { "Shift Beatgrid left" } else { "Shift the whole Beatgrid left" }, false).clicked() {
        app.grid_shift(-step);
    }
    if ib(ui, app, 325.0, "gr", GridIcon::MoveRight, if fc { "Shift Beatgrid right" } else { "Shift the whole Beatgrid right" }, false).clicked() {
        app.grid_shift(step);
    }
    // Beat intervals: 0.01 BPM a click.
    if ib(ui, app, 359.0, "gw", GridIcon::Wide, "Expand beat intervals", false).clicked() {
        app.grid_scale_bpm(|v| ((v - 0.01) * 100.0).round() / 100.0);
    }
    if ib(ui, app, 388.0, "gn", GridIcon::Narrow, "Shrink beat intervals", false).clicked() {
        app.grid_scale_bpm(|v| ((v + 0.01) * 100.0).round() / 100.0);
    }
    if ib(ui, app, 422.0, "g2", GridIcon::Double, "Double the BPM value", false).clicked() {
        app.grid_double_half(true);
    }
    if ib(ui, app, 451.0, "gh", GridIcon::Half, "Halve the BPM value", false).clicked() {
        app.grid_double_half(false);
    }
    // Range of the changes: the one in use is the raised one.
    if ib(ui, app, 485.0, "gwhole", GridIcon::Whole, "Adjust whole Beatgrid", fc).clicked() {
        app.prefs.grid_from_current = false;
        app.save_prefs();
    }
    if ib(ui, app, 514.0, "gcur", GridIcon::FromCurrent, "Adjust partial Beatgrid", !fc).clicked() {
        app.prefs.grid_from_current = true;
        app.save_prefs();
    }
    if ib(ui, app, 548.0, "gundo", GridIcon::Undo, "Undo", app.deck.grid_undo.is_empty()).clicked() {
        app.grid_undo();
    }
    if ib(ui, app, 577.0, "gredo", GridIcon::Redo, "Redo", app.deck.grid_redo.is_empty()).clicked() {
        app.grid_redo();
    }
    let met = app.deck.metronome;
    if ib(ui, app, 605.0, "gmet", GridIcon::Metronome(met), "Metronome On/Off", false).clicked() {
        app.deck.metronome = !met;
        app.update_metronome();
    }
    let vol = app.prefs.metronome_vol.clamp(1, 3);
    if ib(ui, app, 634.0, "gmetvol", GridIcon::Volume(vol), "Metronome Volume", false).clicked() {
        app.set_metronome_volume(vol % 3 + 1);
    }
    let lock_tip = if locked { "Track Analysis, Key Analysis, and Beatgrid Adjustment are locked." } else { "Enable/Disable Track Analysis, Key Analysis, and Beatgrid Adjustment." };
    if ib(ui, app, 663.0, "glock", GridIcon::Lock(locked), lock_tip, false).clicked() {
        if let Some(id) = app.deck.track {
            if let Some(t) = app.track_mut(id) {
                t.grid_locked = !t.grid_locked;
            }
            app.save_track(id);
        }
    }

    // ---- PHRASE EDIT: CUT, the type of the phrase under the playhead, CLEAR
    let cur = app.current_phrase();
    let has = cur.is_some();
    let ink = if has { WHITE.c() } else { gy(0x505050) };
    p.text(pos2(x_of(880.0), s.min.y + 6.0), Align2::LEFT_CENTER, "PHRASE EDIT", caps(8.8), if has { WHITE.c() } else { gy(0x787878) });
    let pb = |x0: f32, x1: f32| Rect::from_min_max(pos2(x_of(x0), y - 8.5), pos2(x_of(x1), y + 8.5));
    let flat = |ui: &mut Ui, r: Rect, id: &str, label: &str, col: Color32| -> egui::Response {
        let resp = ui.interact(r, ui.id().with(id), Sense::click());
        ui.painter().rect_filled(r, 2.0, if resp.hovered() && col == WHITE { lighten(gy(0x212121)) } else { gy(0x212121) });
        ui.painter().text(r.center(), Align2::CENTER_CENTER, label, caps(8.8), col);
        resp
    };
    if tip(flat(ui, pb(880.0, 930.5), "phcut", "CUT", ink), app, "Cut the phrase at the current position.").clicked() && has {
        app.phrase_cut();
    }
    let lr = pb(940.0, 1020.0);
    let lresp = ui.interact(lr, ui.id().with("phkind"), Sense::click());
    p.rect_filled(lr, 2.0, gy(0x212121));
    if let Some((mood, kind)) = cur {
        p.with_clip_rect(lr).text(pos2(lr.min.x + 8.0, lr.center().y), Align2::LEFT_CENTER, crate::phrase::label(mood, kind), FontId::proportional(11.0), WHITE.c());
    }
    crate::icons_player::chevron_down(&p, pos2(lr.max.x - 10.0, lr.center().y), 1.0, if has { WHITE.c() } else { gy(0x505050) });
    let lresp = tip(lresp, app, "Select the phrase type.");
    if let Some((mood, kind)) = cur {
        crate::ui_browser::dropdown(ui, &lresp, "phkindpop", |ui: &mut Ui| {
            for (k, name) in crate::phrase::kinds(mood) {
                if ui.selectable_label(crate::phrase::label(mood, kind) == *name, *name).clicked() {
                    app.phrase_set_kind(*k);
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
    }
    let edited = app.deck.x.phrases_edited;
    if tip(flat(ui, pb(1030.0, 1080.5), "phclear", "CLEAR", if edited { WHITE.c() } else { gy(0x505050) }), app, "Clear the edited phrases.").clicked() && edited {
        app.phrase_clear();
    }

    // ---- Q and the menu, where the CUE/LOOP panel has them
    let q = app.prefs.quantize;
    if tip(seg_button(ui, Rect::from_center_size(pos2(x_of(1222.5), y), vec2(33.0, 20.0)), "quant", "Q", q), app, "Quantize ON/OFF").clicked() {
        app.prefs.quantize = !q;
        app.save_prefs();
    }
    let menu_r = tip(icon_button(ui, Rect::from_center_size(pos2(x_of(1259.5), y - 0.5), vec2(21.0, 21.0)), "stripmenu", Some(BTN.c()), |p, r, c| crate::icons_player::menu(p, r.center(), 1.625, c)), app, "Show Menu items");
    crate::ui_browser::dropdown(ui, &menu_r, "gridpop", |ui: &mut Ui| {
        if ui.button("Set a beat to the current position").clicked() {
            app.grid_center();
        }
        strip_menu_extra(app, ui);
    });
}

// ---------------------------------------------------------------------- transport pieces

/// CUE and PLAY/PAUSE as round buttons of radius `rad` (18.5 pt in the 1 Player layout).
/// `keys`: this deck is the one the keyboard operates.
pub(crate) fn cue_play(app: &mut App, ui: &mut Ui, cue_c: Pos2, play_c: Pos2, rad: f32, keys: bool) {
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let playing = app.playing();
    let size = vec2(rad * 2.0 + 5.0, rad * 2.0 + 5.0);
    let cr = ui.interact(Rect::from_center_size(cue_c, size), ui.id().with("cuebtn"), Sense::click_and_drag());
    let cue_lit = loaded && (!playing || app.deck.cue_preview);
    // The same button faces as the 1 Player layout, scaled to this layout's size.
    let dia = rad * 2.0 + 3.0;
    crate::ui_player::cue_button_face(&p, cue_c, dia, cue_lit, loaded, cr.hovered());
    if crate::ui_player::press_fired(ui, &cr) && !app.deck.cue_preview {
        app.cue_down();
    }
    let cue_key_held = keys && app.prefs.key_for("cue").map(|k| ui.input(|i| i.key_down(k))).unwrap_or(false);
    if !cr.is_pointer_button_down_on() && app.deck.cue_preview && !cue_key_held && !app.deck.x.wave_cue_held {
        app.cue_up();
    }
    let _ = tip(cr, app, "Set a Cue point if you click during pause. \nMove to the Cue point if you click during playback.");
    if crate::ui_player::play_control(app, ui, play_c, size, dia) {
        app.play_pause_synced();
    }
}

/// Artwork tile; a click unloads the track.
pub(crate) fn artwork(app: &mut App, ui: &mut Ui, art: Rect, track: &Option<Track>) {
    let p = ui.painter().clone();
    match track {
        Some(t) => {
            if let Some(tex) = crate::ui_browser::artwork_texture(app, &t.artwork) {
                p.image(tex.id(), crate::ui_player::fit_rect(art, tex.size_vec2()), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                p.rect_filled(art, 0.0, gy(0x141414));
                draw_note_icon(&p.with_clip_rect(art), art);
            }
            let ar = ui.interact(art, ui.id().with("artwork"), Sense::click());
            if tip(ar, app, "Artwork:\nClick it to unload the track.").clicked() {
                app.unload_track();
            }
        }
        None => {
            // Empty deck: a dark tile with a grey disc and hub, soft at the edges.
            p.rect_filled(art, 0.0, gy(0x0F0F0F));
            let k = art.height().min(art.width()) / 48.0;
            for (rad, col) in [(21.5, 0x141414), (21.0, 0x181818), (20.5, 0x1D1D1D), (19.5, 0x1E1E1E), (6.5, 0x191919), (6.0, 0x141414), (5.5, 0x0F0F0F)] {
                p.circle_filled(art.center(), rad * k, Tc(hex(col), hex(if rad > 10.0 { 0xF0F0F0 } else { 0xFFFFFF })).c());
            }
        }
    }
}

// ---------------------------------------------------------------------- 2 Players / Dual Player

/// Where things are in the two-deck layouts (points; measured on rekordbox 7.2.19 at
/// 1512 pt: the player is 375 pt tall, deck A's rows from the top, deck B's mirrored from the
/// bottom, the two enlarged waveforms sharing the 154 pt in between).
struct TwoGeom {
    full: Rect,
    /// Left and right ends of the deck rows (79 or, with the Dual Player's extra column,
    /// 124 from the left; 212 from the right, where the cue panels begin).
    mx0: f32,
    mx1: f32,
    dual: bool,
}

/// Y positions of one deck's rows from the player's top.
struct DeckRows {
    /// Top of the artwork / title block (36 pt tall).
    title: f32,
    /// Top of the overview waveform's rect (33 pt tall).
    overview: f32,
    /// Top of the 26 pt control row.
    strip: f32,
    /// Beat area of the enlarged waveform: its outer edge (beat marks) and base line.
    wave: (f32, f32),
    /// Top and bottom of the MEMORY / HOT CUE / INFO panel.
    panel: (f32, f32),
}

const ROWS_A: DeckRows = DeckRows { title: 6.0, overview: 43.0, strip: 85.0, wave: (138.0, 187.0), panel: (0.0, 187.0) };
const ROWS_B: DeckRows = DeckRows { title: 334.0, overview: 293.0, strip: 265.0, wave: (189.0, 238.0), panel: (188.0, 375.0) };

/// 2 Players and Dual Player: decks A and B one above the other. Each has its own track,
/// overview and control row (grid nudge, MEMORY, loop, tempo, pitch bend, MT, RST, Q), BEAT
/// SYNC and MASTER, and a MEMORY / HOT CUE / INFO panel; the enlarged waveforms share the
/// middle, with one zoom for both. On the left are CUE and PLAY for each deck around the A–B
/// crossfader. Dual Player adds beat jump, A-GAIN, HIGH / MID / LOW, KEY SYNC and key shift.
/// The deck last clicked is the active one: keyboard shortcuts follow it. The browser's
/// PLAYER A/B selector chooses where double-click and Return load; a track dropped on a deck's
/// half loads there.
pub fn two_players(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let dual = app.dual_player();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    let (l, t) = (full.min.x, full.min.y);
    let col_w = if dual { 70.0 } else { 76.0 };
    p.rect_filled(Rect::from_min_max(pos2(l, t + 1.0), pos2(l + col_w, full.max.y)), 0.0, BG2);
    if dual {
        p.rect_filled(Rect::from_min_max(pos2(l + 73.0, t + 1.0), pos2(l + 121.0, full.max.y)), 0.0, BG2);
    }
    let g = TwoGeom { full, mx0: l + if dual { 124.0 } else { 79.0 }, mx1: full.max.x - 212.0, dual };
    let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    let mid = t + 188.0;
    let mut scrub = [0.0f64; 2];
    let mut jumped: Option<(u8, f64)> = None;
    for n in 0..2u8 {
        let is_active = app.px.active == n;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(full).id_salt(("deck", n)));
        child.set_clip_rect(full.intersect(ui.clip_rect()));
        let rows = if n == 0 { &ROWS_A } else { &ROWS_B };
        app.with_deck(n, |app| {
            app.deck.x.scrub = 0.0;
            if let Some(b) = deck_rows(app, &mut child, &g, rows, n, is_active) {
                jumped = Some((n, b));
            }
            scrub[n as usize] = app.deck.x.scrub;
        });
    }
    // Dual control: a beat jump on one deck is made on the other too.
    if let (true, Some((n, beats))) = (app.px.dual_control && dual, jumped) {
        app.with_deck(1 - n, |a| a.beat_jump(beats));
    }
    // Waveform link: scrubbing or scrolling one waveform moves the other deck as far.
    if app.px.wave_link {
        for n in 0..2u8 {
            let d = scrub[n as usize];
            if d != 0.0 {
                app.with_deck(1 - n, |a| {
                    if a.deck.audio.is_some() {
                        let t = a.pos() + d;
                        a.seek(t);
                        if !a.playing() {
                            a.deck.cue_point = a.pos();
                        }
                    }
                });
            }
        }
    }
    shared_controls(app, ui, &g);
    // The deck whose half was pressed becomes the active one.
    if let Some(pp) = pressed.filter(|pp| full.contains(*pp)) {
        app.activate_deck(if pp.y < mid { 0 } else { 1 });
    }
}

/// A raised 17 pt tall button of the deck rows with a caps label; `lit`: blue (on).
fn row_button(ui: &mut Ui, r: Rect, id: &str, label: &str, lit: bool, enabled: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let fill = if lit { BLUE.c() } else if resp.is_pointer_button_down_on() { BG2.c() } else if resp.hovered() { lighten(BTN.c()) } else { BTN.c() };
    ui.painter().rect_filled(r, 2.0, fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, caps(8.8), if enabled { WHITE.c() } else { OFF.c() });
    resp
}

/// A raised icon tile of the deck rows.
fn row_icon(ui: &mut Ui, r: Rect, id: &str, draw: impl FnOnce(&Painter, Pos2, Color32, bool)) -> egui::Response {
    row_icon_on(ui, r, id, true, draw)
}

/// `row_icon`; with `enabled` false the tile lies flat in the row (an empty deck).
fn row_icon_on(ui: &mut Ui, r: Rect, id: &str, enabled: bool, draw: impl FnOnce(&Painter, Pos2, Color32, bool)) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let fill = if !enabled { gy(0x212121) } else if resp.is_pointer_button_down_on() { BG2.c() } else if resp.hovered() { lighten(BTN.c()) } else { BTN.c() };
    ui.painter().rect_filled(r, 2.0, fill);
    draw(ui.painter(), r.center(), WHITE.c(), resp.is_pointer_button_down_on());
    resp
}

/// True on a click, and again every 50 ms once the button has been held for 0.4 s.
fn click_or_repeat(ui: &Ui, resp: &egui::Response) -> bool {
    if resp.clicked() {
        return true;
    }
    if resp.is_pointer_button_down_on() {
        ui.ctx().request_repaint();
        let key = resp.id.with("repeat");
        let now = ui.input(|i| i.time);
        let held = ui.input(|i| i.pointer.press_start_time()).map(|s| now - s).unwrap_or(0.0);
        if held > 0.4 {
            let last = ui.ctx().data(|d| d.get_temp::<f64>(key)).unwrap_or(0.0);
            if now - last >= 0.05 {
                ui.ctx().data_mut(|d| d.insert_temp(key, now));
                return true;
            }
        }
    }
    false
}

/// One deck of the two-deck layouts. Returns the beats of a beat jump made on it (for the
/// Dual Player's dual control).
fn deck_rows(app: &mut App, ui: &mut Ui, g: &TwoGeom, rows: &DeckRows, n: u8, is_active: bool) -> Option<f64> {
    let p = ui.painter().clone();
    let (l, t) = (g.full.min.x, g.full.min.y);
    let (mx0, mx1) = (g.mx0, g.mx1);
    let pos = app.pos();
    let loaded = app.deck.track.is_some();
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    let mut jumped = None;

    // ---- transport column: CUE and PLAY (and, in Dual Player, beat jump)
    if g.dual {
        let (cue_y, play_y, jump_y) = if n == 0 { (48.0, 86.0, 112.0) } else { (290.0, 328.0, 211.0) };
        cue_play(app, ui, pos2(l + 35.0, t + cue_y), pos2(l + 35.0, t + play_y), 14.5, is_active);
        let bj = app.prefs.beat_jump;
        let tile = |x: f32| Rect::from_min_size(pos2(l + x, t + jump_y), vec2(25.0, 22.0));
        if tip(icon_button(ui, tile(8.0), "bjl", Some(BTN.c()), |p, r, c| crate::icons_player::chevron_left(p, r.center(), 1.35, c)), app, "Jump Reverse").clicked() {
            app.beat_jump(-bj);
            jumped = Some(-bj);
        }
        if tip(icon_button(ui, tile(37.0), "bjr", Some(BTN.c()), |p, r, c| crate::icons_player::chevron_right(p, r.center(), 1.35, c)), app, "Jump Forward").clicked() {
            app.beat_jump(bj);
            jumped = Some(bj);
        }
        let bj_rect = Rect::from_min_size(pos2(l + 8.0, t + jump_y + 26.0), vec2(54.0, 27.0));
        let r = tip(ui.interact(bj_rect, ui.id().with("bjsize"), Sense::click()), app, "Select the beat/bar length jumping from the current position.");
        p.rect_filled(bj_rect, 0.0, if r.hovered() { lighten(BTN.c()) } else { BTN.c() });
        p.text(pos2(bj_rect.center().x, bj_rect.min.y + 8.7), Align2::CENTER_CENTER, crate::player_ext::jump_label(bj, app.prefs.beat_jump_bars), bold(9.0), WHITE.c());
        crate::icons_player::chevron_down(&p, pos2(bj_rect.center().x + 0.75, bj_rect.min.y + 20.5), 1.0, WHITE.c());
        crate::ui_browser::dropdown(ui, &r, "bjpop", |ui: &mut Ui| jump_size_menu(app, ui));
    } else {
        let (cue_y, play_y) = if n == 0 { (57.0, 107.0) } else { (273.0, 323.0) };
        cue_play(app, ui, pos2(l + 38.0, t + cue_y), pos2(l + 38.0, t + play_y), 18.5, is_active);
    }

    // ---- Dual Player: A-GAIN and the isolator switches
    if g.dual {
        let cx = l + 97.0;
        let (db_y, knob_y, name_y, eq_y) = if n == 0 { (29.8, 47.5, 64.0, 79.0) } else { (309.8, 328.0, 344.0, 241.0) };
        let db = app.gain_db();
        let dim = if loaded { gy(0x686868) } else { gy(0x454545) };
        p.text(pos2(cx, t + db_y), Align2::CENTER_CENTER, format!("{db:.1}dB"), bold(9.0), dim);
        p.text(pos2(cx, t + name_y), Align2::CENTER_CENTER, "A-GAIN", bold(9.0), dim);
        let kc = pos2(cx, t + knob_y);
        let kr = ui.interact(Rect::from_center_size(kc, vec2(26.0, 26.0)), ui.id().with("again"), Sense::click_and_drag());
        crate::icons_player::gain_knob(&p, kc, crate::player_ext::gain_angle(db), loaded);
        if loaded {
            if kr.drag_started() {
                app.deck.x.gain_drag = kr.interact_pointer_pos().map(|pp| (pp.y, db));
            }
            if let (true, Some((y0, db0)), Some(pp)) = (kr.dragged(), app.deck.x.gain_drag, kr.interact_pointer_pos()) {
                // 100 pt of travel covers the knob's range.
                app.set_gain_db(db0 + (y0 - pp.y) * (2.0 * crate::player_ext::GAIN_RANGE_DB / 100.0), false);
            }
            if kr.drag_stopped() {
                app.deck.x.gain_drag = None;
                let v = app.gain_db();
                app.set_gain_db(v, true);
            }
            if kr.double_clicked() {
                app.set_gain_db(0.0, true);
            }
        }
        let _ = tip(kr, app, "Auto Gain knob:\nAdjusts Auto Gain value.");
        for (i, (name, band, tipt)) in [("HIGH", 2usize, "Kill Switch (High):\nSwitches the high frequency of the audio off."), ("MID", 1, "Kill Switch (Mid):\nSwitches the mid frequency of the audio off."), ("LOW", 0, "Kill Switch (Low):\nSwitches the low frequency of the audio off.")].iter().enumerate() {
            let r = Rect::from_min_size(pos2(l + 79.0, t + eq_y + i as f32 * 20.0), vec2(36.0, 16.0));
            let resp = ui.interact(r, ui.id().with(("eqkill", i)), Sense::click());
            let killed = app.deck.x.eq_kill[*band];
            p.rect_filled(r, 2.0, if resp.hovered() { lighten(BTN.c()) } else { BTN.c() });
            if killed {
                p.rect_stroke(r, 2.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
            }
            p.text(r.center(), Align2::CENTER_CENTER, *name, caps(8.8), WHITE.c());
            if tip(resp, app, tipt).clicked() {
                app.toggle_eq_kill(*band);
            }
        }
    }

    // ---- title block: artwork, title, artist; time, key, BPM; BEAT SYNC / MASTER
    let y0 = t + rows.title;
    artwork(app, ui, Rect::from_min_size(pos2(mx0 + 5.0, y0), vec2(36.0, 36.0)), &track);
    let shift = if g.dual { 84.0 } else { 0.0 };
    let (d1, d2, d3) = (mx1 - 372.5 - shift, mx1 - 231.5 - shift, mx1 - 175.5 - shift);
    if let Some(tr) = &track {
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(mx0 + 49.0, y0 - 4.0), pos2(d1 - 8.0, y0 + 38.0)));
        clip.text(pos2(mx0 + 49.5, y0 + 8.3), Align2::LEFT_CENTER, tr.display_title(), bold(15.5), WHITE.c());
        clip.text(pos2(mx0 + 49.5, y0 + 28.7), Align2::LEFT_CENTER, &tr.artist, FontId::proportional(12.25), DIM.c());
    }
    let ty = y0 + 28.0;
    for x in [d1, d2, d3] {
        p.rect_filled(Rect::from_min_max(pos2(x.floor(), y0 + 21.0), pos2(x.floor() + 1.0, y0 + 36.0)), 0.0, OFF);
    }
    // Remaining and elapsed time; a click switches which one is bright.
    {
        let remain = (app.deck.duration - pos).max(0.0);
        let (dim_t, bright_t) = if loaded { (DIM.c(), WHITE.c()) } else { (gy(0x323232), gy(0x323232)) };
        let (c1, c2) = if app.prefs.time_remain { (bright_t, dim_t) } else { (dim_t, bright_t) };
        let (big, small) = split_time(if loaded { remain } else { 0.0 });
        p.text(pos2(d1 + 22.0, ty), Align2::LEFT_CENTER, format!("-{big}.{small}"), FontId::proportional(12.0), c1);
        let (big2, small2) = split_time(if loaded { pos } else { 0.0 });
        p.text(pos2(d1 + 79.5, ty), Align2::LEFT_CENTER, format!("{big2}.{small2}"), FontId::proportional(12.0), c2);
        let hit = ui.interact(Rect::from_min_max(pos2(d1 + 4.0, y0 + 19.0), pos2(d2 - 4.0, y0 + 38.0)), ui.id().with("timesw"), Sense::click());
        if tip(hit, app, "Switch Time/Remain").clicked() {
            app.prefs.time_remain = !app.prefs.time_remain;
        }
    }
    if loaded {
        p.text(pos2((d2 + d3) / 2.0, ty), Align2::CENTER_CENTER, app.result_key_label(), FontId::proportional(12.0), WHITE.c());
        let b = app.effective_bpm();
        p.text(pos2(d3 + 38.5, ty), Align2::CENTER_CENTER, if b > 0.0 { format!("{b:.2}") } else { "--.--".into() }, FontId::proportional(12.0), WHITE.c());
    }
    let is_master = app.px.master == n;
    let sync_on = app.deck.x.beat_sync;
    let top_btn = |x: f32, row: f32| Rect::from_min_size(pos2(mx1 - x, y0 + row), vec2(70.0, 16.0));
    if tip(row_button(ui, top_btn(75.0, 0.0), "beatsync", "BEAT SYNC", sync_on, true), app, "On/Off Sync.").clicked() {
        app.toggle_beat_sync();
    }
    {
        // MASTER: outlined; blue while this deck is the master.
        let r = top_btn(75.0, 20.0);
        let resp = ui.interact(r, ui.id().with("master"), Sense::click());
        p.rect_filled(r, 2.0, if resp.hovered() { gy(0x1A1A1A) } else { BG.c() });
        p.rect_stroke(r, 2.0, Stroke::new(1.0_f32, if is_master { BLUE } else { OFF }), egui::StrokeKind::Inside);
        p.text(r.center(), Align2::CENTER_CENTER, "MASTER", caps(8.8), WHITE.c());
        if tip(resp, app, "Click here to set this deck as a master for Sync.").clicked() {
            app.set_master();
        }
    }
    if g.dual {
        if tip(row_button(ui, top_btn(159.0, 0.0), "keysync", "KEY SYNC", app.deck.x.key_sync, true), app, "On/Off Key Sync.").clicked() {
            app.toggle_key_sync();
        }
        // Key shift: ‹ key ±n ›
        let col = if loaded { WHITE.c() } else { gy(0x3C3C3C) };
        let lr = Rect::from_center_size(pos2(mx1 - 154.0, ty), vec2(14.0, 16.0));
        let rr = Rect::from_center_size(pos2(mx1 - 94.0, ty), vec2(14.0, 16.0));
        let lresp = ui.interact(lr, ui.id().with("keydn"), Sense::click());
        let rresp = ui.interact(rr, ui.id().with("keyup"), Sense::click());
        crate::icons_player::chevron_left(&p, lr.center(), 1.0, col);
        crate::icons_player::chevron_right(&p, rr.center(), 1.0, col);
        if tip(lresp, app, "Key Shift:\nLower the key by a semitone.").clicked() {
            app.key_shift_step(-1);
        }
        if tip(rresp, app, "Key Shift:\nRaise the key by a semitone.").clicked() {
            app.key_shift_step(1);
        }
        if loaded {
            p.text(pos2(mx1 - 136.5, ty), Align2::CENTER_CENTER, app.result_key_label(), bold(10.5), WHITE.c());
            p.text(pos2(mx1 - 109.0, ty), Align2::CENTER_CENTER, crate::player_ext::shift_text(app.deck.x.key_shift), bold(10.5), WHITE.c());
            let mid = ui.interact(Rect::from_min_max(pos2(mx1 - 146.0, ty - 8.0), pos2(mx1 - 101.0, ty + 8.0)), ui.id().with("keyreset"), Sense::click());
            if tip(mid, app, "Key Reset:\nDouble click here to reset the key.").double_clicked() {
                app.key_reset();
            }
        }
    }

    // ---- overview
    let ov = Rect::from_min_max(pos2(mx0 + 11.5, t + rows.overview), pos2(mx1 - 9.0, t + rows.overview + 33.0));
    draw_overview(app, ui, ov, pos);

    // ---- enlarged waveform (this deck's half of the shared area)
    let wr = Rect::from_min_max(pos2(mx0, t + rows.wave.0), pos2(mx1, t + rows.wave.1));
    let mode = if n == 0 { DetailMode::HalfUp } else { DetailMode::HalfDown };
    // (Black in both appearances, as rekordbox's waveform views.)
    ui.painter().rect_filled(wr, 0.0, WAVE_BG);
    draw_detail_at(app, ui, wr, pos, if g.dual { 0.5 } else { 0.0 }, mode);

    // ---- control row
    let s = Rect::from_min_max(pos2(mx0, t + rows.strip), pos2(mx1, t + rows.strip + 26.0));
    p.rect_filled(s, 0.0, BG2);
    let cy = s.min.y + 13.5;
    let b = |x: f32, w: f32| Rect::from_min_size(pos2(mx0 + x, cy - 8.5), vec2(w, 17.0));
    let br = |x: f32, w: f32| Rect::from_min_size(pos2(mx1 - x, cy - 8.5), vec2(w, 17.0));
    let ink = if loaded { WHITE.c() } else { gy(0x404040) };
    let fine = ui.input(|i| i.modifiers.shift);
    let step = if fine { 0.005 } else { 0.001 };
    if tip(row_icon_on(ui, b(5.0, 40.0), "g2l", loaded, |p, c, _, _| crate::icons_player::grid2_move(p, c, -1.0, ink)), app, "Shift Beatgrid left").clicked() {
        app.grid_shift(-step);
    }
    if tip(row_icon_on(ui, b(49.0, 30.0), "g2c", loaded, |p, c, _, _| crate::icons_player::grid2_center(p, c, ink, if loaded { BAR_RED } else { hex(0x631818) })), app, "Shift Beatgrid to the center").clicked() {
        app.grid_center();
    }
    if tip(row_icon_on(ui, b(83.0, 40.0), "g2r", loaded, |p, c, _, _| crate::icons_player::grid2_move(p, c, 1.0, ink)), app, "Shift Beatgrid right").clicked() {
        app.grid_shift(step);
    }
    if tip(row_button(ui, b(147.0, 70.0), "mem", "MEMORY", false, true), app, "Set Memory Cue(Loop)").clicked() {
        app.memory_set();
    }
    let au = app.prefs.loop_auto;
    if tip(seg_button(ui, b(241.0, 40.0), "au", "AU", au), app, "Auto Beat Loop").clicked() {
        app.prefs.loop_auto = true;
    }
    if tip(seg_button(ui, b(285.0, 40.0), "ma", "MA", !au), app, "Manual Loop").clicked() {
        app.prefs.loop_auto = false;
    }
    if au {
        if tip(row_icon(ui, b(335.0, 20.0), "lhalf", |p, c, col, _| crate::icons_player::chevron_left(p, c, 1.25, col)), app, "Auto Beat Loop:\nMake Loop length shorter.").clicked() {
            app.resize_loop(0.5);
        }
        let lb = app.prefs.loop_beats;
        let active = app.loop_active().is_some();
        let field = b(359.0, 66.0);
        if tip(button(ui, field, "lbeat", &beats_text(lb), if active { LOOP.gamma_multiply(0.7) } else { BTN.c() }, WHITE.c(), 11.0), app, "Auto Beat Loop:\nSet/Reset Loop of displayed beat length.").clicked() {
            if active {
                app.loop_exit();
            } else {
                app.auto_loop(lb);
            }
        }
        let dr = Rect::from_min_max(pos2(field.max.x - 14.0, field.min.y), field.max);
        let dresp = tip(ui.interact(dr, ui.id().with("lsize"), Sense::click()), app, "Beat Loop");
        if ui.rect_contains_pointer(field) {
            crate::icons_player::chevron_down(&p, dr.center(), 0.9, if dresp.hovered() { WHITE.c() } else { DIM.c() });
        }
        crate::ui_browser::dropdown(ui, &dresp, "lsizepop", |ui: &mut Ui| loop_size_menu(app, ui));
        if tip(row_icon(ui, b(429.0, 20.0), "ldouble", |p, c, col, _| crate::icons_player::chevron_right(p, c, 1.25, col)), app, "Auto Beat Loop:\nMake Loop length longer.").clicked() {
            app.resize_loop(2.0);
        }
    } else {
        // IN, OUT and RELOOP/EXIT in the place of the loop length (335..449).
        manual_loop_buttons(app, ui, |x, _| {
            if x < 980.0 {
                b(335.0, 26.0)
            } else if x < 1020.0 {
                b(363.0, 26.0)
            } else {
                b(391.0, 58.0)
            }
        });
    }
    // Tempo: − [BPM] +
    let follower = app.sync_follower();
    let minus = row_icon_on(ui, b(473.0, 30.0), "tminus", loaded, |p, c, _, _| crate::icons_player::tempo_sign(p, c, false, if loaded { WHITE.c() } else { gy(0x626262) }));
    if click_or_repeat(ui, &minus) && loaded {
        app.tempo_step(-1);
    }
    let _ = tip(minus, app, "Decrease Tempo");
    let field = b(507.0, 65.0);
    p.rect_filled(field, 2.0, BTN);
    if app.deck.x.tempo_editing {
        let mut txt = app.deck.x.tempo_text.clone();
        let resp = ui.put(field.shrink2(vec2(4.0, 0.0)), egui::TextEdit::singleline(&mut txt).horizontal_align(egui::Align::Center).frame(false).font(FontId::proportional(11.5)).text_color(WHITE.c()));
        app.deck.x.tempo_text = txt;
        // Whether the field just gave up focus must be read before focus is asked for again.
        let gave_up = resp.lost_focus();
        if gave_up {
            app.deck.x.tempo_editing = false;
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let v = app.deck.x.tempo_text.clone();
                app.tempo_enter_bpm(&v);
            }
        } else if !resp.has_focus() {
            resp.request_focus();
        }
    } else {
        let bpm = app.effective_bpm();
        if loaded && bpm > 0.0 {
            p.text(field.center(), Align2::CENTER_CENTER, format!("{bpm:.2}"), FontId::proportional(11.0), if follower { tc(0x6FA8F5, 0x24559A).c() } else { WHITE.c() });
        }
        let resp = ui.interact(field, ui.id().with("tempofield"), Sense::click());
        if tip(resp, app, "Display the BPM of the track currently playing.\nClick here to enter a BPM value (within the Tempo Range) directly.").clicked() && loaded && bpm > 0.0 {
            if follower {
                app.toast_err("Invalid operation. In Sync mode, Tempo Slider is only available for Master Deck.");
            } else {
                app.deck.x.tempo_editing = true;
                app.deck.x.tempo_text = format!("{bpm:.2}");
                ui.ctx().request_repaint();
            }
        }
    }
    let plus = row_icon_on(ui, b(576.0, 30.0), "tplus", loaded, |p, c, _, _| crate::icons_player::tempo_sign(p, c, true, if loaded { WHITE.c() } else { gy(0x626262) }));
    if click_or_repeat(ui, &plus) && loaded {
        app.tempo_step(1);
    }
    let _ = tip(plus, app, "Increase Tempo");
    // Pitch bend, while held.
    let bl = row_icon(ui, b(630.0, 40.0), "bendl", |p, c, col, _| crate::icons_player::bend(p, c, 1.5, -1.0, col));
    let brr = row_icon(ui, b(675.0, 40.0), "bendr", |p, c, col, _| crate::icons_player::bend(p, c, 1.5, 1.0, col));
    let dir = if bl.is_pointer_button_down_on() { -1 } else if brr.is_pointer_button_down_on() { 1 } else { 0 };
    app.pitch_bend(if loaded { dir } else { 0 });
    if dir != 0 {
        ui.ctx().request_repaint();
    }
    let _ = tip(bl, app, "Pitch bend :\nIncrease or decrease the playing speed.");
    let _ = tip(brr, app, "Pitch bend :\nIncrease or decrease the playing speed.");
    // MT, RST, Q
    if tip(row_button(ui, br(193.0, 60.0), "mt", "MT", app.deck.x.master_tempo, true), app, "On/Off Master Tempo (adjusting playback speed without changing the pitch)").clicked() {
        app.toggle_master_tempo();
    }
    if tip(row_button(ui, br(129.0, 60.0), "rst", "RST", false, true), app, "Tempo Reset").clicked() {
        app.tempo_reset();
    }
    let q = app.prefs.quantize;
    if tip(seg_button(ui, br(45.0, 40.0), "quant", "Q", q), app, "Quantize ON/OFF").clicked() {
        app.prefs.quantize = !q;
        app.save_prefs();
    }

    // ---- MEMORY / HOT CUE / INFO panel
    let panel = Rect::from_min_max(pos2(g.full.max.x - 202.0, t + rows.panel.0), pos2(g.full.max.x, t + rows.panel.1));
    crate::ui_player::side_panel(app, ui, panel, &crate::ui_player::PANEL_2P);

    // A track dropped on this deck's half loads here.
    let half = if n == 0 { Rect::from_min_max(g.full.min, pos2(g.full.max.x, t + 188.0)) } else { Rect::from_min_max(pos2(g.full.min.x, t + 188.0), g.full.max) };
    drop_to_load(app, ui, half);
    jumped
}

/// What the two decks share: the crossfader, the zoom buttons, the waveform link and the
/// Dual Player's dual control.
fn shared_controls(app: &mut App, ui: &mut Ui, g: &TwoGeom) {
    let p = ui.painter().clone();
    let (l, t) = (g.full.min.x, g.full.min.y);
    // Crossfader: A at the top, B at the bottom; the fader runs from y 162 to 215.
    let cx = l + if g.dual { 97.0 } else { 38.0 };
    let (y0, y1) = (t + 162.0, t + 215.0);
    let label = gy(0x757575);
    p.text(pos2(cx, t + 150.0), Align2::CENTER_CENTER, "A", bold(10.0), label);
    p.text(pos2(cx, t + if g.dual { 226.0 } else { 230.0 }), Align2::CENTER_CENTER, "B", bold(10.0), label);
    let rail = gy(0x323232);
    p.rect_filled(Rect::from_min_max(pos2(cx - 0.5, y0), pos2(cx + 0.5, y1)), 0.0, rail);
    p.rect_filled(Rect::from_min_max(pos2(cx - 12.0, y0), pos2(cx + 12.0, y0 + 1.0)), 0.0, rail);
    p.rect_filled(Rect::from_min_max(pos2(cx - 12.0, y1 - 1.0), pos2(cx + 12.0, y1)), 0.0, rail);
    let area = Rect::from_min_max(pos2(cx - 16.0, y0 - 5.0), pos2(cx + 16.0, y1 + 5.0));
    let resp = ui.interact(area, ui.id().with("crossfader"), Sense::click_and_drag());
    if resp.dragged() || resp.clicked() {
        if let Some(pp) = resp.interact_pointer_pos() {
            app.set_crossfader((pp.y - y0) / (y1 - y0));
        }
    }
    if resp.double_clicked() {
        app.set_crossfader(0.5);
    }
    let hy = y0 + app.px.crossfader * (y1 - y0);
    p.rect_filled(Rect::from_center_size(pos2(cx, hy), vec2(28.0, 3.0)), 0.0, if resp.hovered() || resp.dragged() { gy(0x8C8C8C) } else { gy(0x646464) });
    let _ = tip(resp, app, "CROSSFADER:\nOutputs audio of each channel according to the curve characteristics set in [Preferences].");

    // Zoom (both waveforms): on a black strip over the left end of the waveform area.
    let zx = g.mx0 + 11.0;
    p.rect_filled(Rect::from_min_max(pos2(g.mx0, t + 148.0), pos2(g.mx0 + 24.0, t + 224.0)), 0.0, ZOOM_BG.c());
    let zc = |y: f32| Rect::from_center_size(pos2(zx, t + y), vec2(22.0, 20.0));
    if tip(icon_button(ui, zc(161.0), "zin", None, |p, r, c| crate::icons_player::zoom(p, r.center(), 7.0 / 6.0, c, true)), app, "Zoom In").clicked() && app.prefs.zoom > 0 {
        app.prefs.zoom -= 1;
    }
    if tip(button(ui, zc(185.4), "zrst", "RST", Color32::TRANSPARENT, WHITE.c(), 8.2), app, "Reset Zoom").clicked() {
        app.prefs.zoom = 2;
    }
    if tip(icon_button(ui, zc(211.0), "zout", None, |p, r, c| crate::icons_player::zoom(p, r.center(), 7.0 / 6.0, c, false)), app, "Zoom Out").clicked() && app.prefs.zoom + 1 < ZOOMS.len() {
        app.prefs.zoom += 1;
    }

    // Waveform link: a 30 pt tile at the right end of the waveform area.
    let lr = Rect::from_min_max(pos2(g.mx1 - 30.0, t + 173.0), pos2(g.mx1, t + 203.0));
    let resp = ui.interact(lr, ui.id().with("wavelink"), Sense::click());
    p.rect_filled(lr, 0.0, if resp.hovered() { gy(0x1E1E1E) } else { gy(0x0A0A0A) });
    crate::icons_player::link(&p, lr.center(), 1.25, if app.px.wave_link { BLUE.c() } else { gy(0x686868) });
    if tip(resp, app, "Link the waveforms:\nScrolling one enlarged waveform moves both decks.").clicked() {
        app.px.wave_link = !app.px.wave_link;
    }

    // Dual control (Dual Player): between the two beat jump groups.
    if g.dual {
        let c = pos2(l + 35.0, t + 188.0);
        let resp = ui.interact(Rect::from_center_size(c, vec2(30.0, 26.0)), ui.id().with("dualctl"), Sense::click());
        if resp.hovered() {
            p.rect_filled(resp.rect, 2.0, gy(0x2A2A2A));
        }
        crate::icons_player::dual_control(&p, c, app.px.dual_control);
        if tip(resp, app, "Dual control:\nBeat jumps move both decks together.").clicked() {
            app.px.dual_control = !app.px.dual_control;
        }
    }
}

// ---------------------------------------------------------------------- Simple Player

/// The Simple Player (59 pt): PLAY in a 58 pt column, the artwork (66..114), title, artist,
/// times, key and BPM in one row with the overview waveform under it, and the rating /
/// colour / comment sheet of the INFO panel at the right. No enlarged waveform. x positions
/// are the 1 Player layout's, except the title at 120, the artist's divider at 640 and the
/// overview from 128.
pub fn simple_player(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    let pos = app.pos();
    let (l, t, r) = (full.min.x, full.min.y, full.max.x);
    let loaded = app.deck.track.is_some();
    p.rect_filled(Rect::from_min_max(pos2(l, t + 1.0), pos2(l + 58.0, full.max.y)), 0.0, BG2);
    // PLAY
    let play_c = pos2(l + 29.0, t + 30.0);
    if crate::ui_player::play_control(app, ui, play_c, vec2(42.0, 42.0), 40.0) {
        app.play_pause();
    }
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    artwork(app, ui, Rect::from_min_size(pos2(l + 66.0, t + 6.0), vec2(48.0, 48.0)), &track);
    // The row scales with the window as the 1 Player layout's does.
    let main = Rect::from_min_max(pos2(l + 80.0, t), pos2(r - 212.0, full.max.y));
    let x = |v: f32| crate::ui_player::sx(main, v);
    let (x_artist, x_time, x_key, x_bpm) = (x(640.0), x(1011.0), x(1168.0), x(1224.0));
    if let Some(tr) = &track {
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(l + 120.0, t), pos2(x_artist - 8.0, t + 27.0)));
        clip.text(pos2(l + 119.3, t + 16.3), Align2::LEFT_CENTER, tr.display_title(), bold(15.5), WHITE.c());
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(x_artist, t), pos2(x_time - 8.0, t + 27.0)));
        clip.text(pos2(x_artist + 16.5, t + 16.7), Align2::LEFT_CENTER, &tr.artist, FontId::proportional(12.25), DIM.c());
        p.text(pos2((x_key + x_bpm) / 2.0 + 0.5, t + 16.2), Align2::CENTER_CENTER, tr.key_label(app.prefs.key_alnum), FontId::proportional(13.0), WHITE.c());
        let b = app.current_bpm();
        p.text(pos2(x(1263.0), t + 16.2), Align2::CENTER_CENTER, if b > 0.0 { format!("{b:.2}") } else { "--.--".into() }, FontId::proportional(13.0), WHITE.c());
    }
    for dx in [x_artist, x_time, x_key, x_bpm] {
        p.rect_filled(Rect::from_min_max(pos2(dx.floor(), t + 8.0), pos2(dx.floor() + 1.0, t + 24.5)), 0.0, OFF);
    }
    // Remaining and elapsed time; tenths smaller on the same baseline.
    let remain = (app.deck.duration - pos).max(0.0);
    let (dim_t, bright_t) = if loaded { (DIM.c(), WHITE.c()) } else { (OFF.c(), OFF.c()) };
    let (c1, c2) = if app.prefs.time_remain { (bright_t, dim_t) } else { (dim_t, bright_t) };
    let ty = t + 15.4;
    let (big, small) = split_time(if loaded { remain } else { 0.0 });
    let tr = p.text(pos2(x_time + 20.5, ty), Align2::LEFT_CENTER, format!("-{big}"), FontId::proportional(15.0), c1);
    p.text(pos2(tr.max.x, ty + 1.4), Align2::LEFT_CENTER, format!(".{small}"), FontId::proportional(11.5), c1);
    let (big2, small2) = split_time(if loaded { pos } else { 0.0 });
    let er = p.text(pos2(x_time + 89.0, ty), Align2::LEFT_CENTER, &big2, FontId::proportional(15.0), c2);
    p.text(pos2(er.max.x, ty + 1.4), Align2::LEFT_CENTER, format!(".{small2}"), FontId::proportional(11.5), c2);
    let hit = ui.interact(Rect::from_min_max(pos2(x_time, t + 6.0), pos2(x_key, t + 27.0)), ui.id().with("timesw"), Sense::click());
    if tip(hit, app, "Switch Time/Remain").clicked() {
        app.prefs.time_remain = !app.prefs.time_remain;
    }
    // Overview: base line 47 pt below the top.
    draw_overview(app, ui, Rect::from_min_max(pos2(l + 128.0, t + 17.0), pos2(x(1290.0), t + 50.0)), pos);
    // The INFO sheet: rating, colour and comment (scroll for the rows below the first).
    p.rect_filled(Rect::from_min_max(pos2(r - 209.0, t + 1.0), pos2(r - 3.0, full.max.y)), 0.0, BG2);
    let sheet = Rect::from_min_max(pos2(r - 201.0, t + 6.0), pos2(r - 11.0, t + 54.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(sheet).id_salt("simpleinfo"));
    child.set_clip_rect(sheet);
    crate::ui_player::info_panel(app, &mut child, sheet);
    drop_to_load(app, ui, full);
}
