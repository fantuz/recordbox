//! The GROOVE CIRCUIT panel of PERFORMANCE mode and the DRUM CAPTURE buttons, after rekordbox
//! 7.2.19. rekordbox has no reference picture of this panel in `rekordbox-reference/`, so
//! the layout is the one its code sets:
//!
//! - where the panel sits: `DJDeck::setBounds` (0x1022893a8, its two layout lambdas
//!   0x1022930dc / 0x102293768) puts each deck's `GrooveCircuitView` under the deck's pads,
//!   1 pt in from the deck's sides, 52 pt high, and `PlayerPanelL::getPlayerHeight`
//!   (0x10228a2d0) makes the player 53 pt taller per row of decks;
//! - the panel: `GrooveCircuitView::resized` 0x100f17294 (every rectangle below), its
//!   constructor 0x100f15530 (labels, fonts, icon files and where each icon sits in its
//!   button), `paint` 0x100f17260 (background, colour id 50331657);
//! - a slot: `DrumSwapSlot::resized` 0x10084e8fc, `paint` 0x10084d8f4, its constructor
//!   0x10084c078, `mouseEnter` / `mouseExit` 0x10084fe78 / 0x10084ff64 (the waveform and the
//!   title give way to Play, Eject, the GAIN knob, BPM and beats while the pointer is over
//!   it), `setWaveViewImage` 0x10084e648 (beat marks, the 30 % shade), `PlayPosition::draw`;
//! - the buttons' look: `CustomTextButton::paintButton` 0x1002d0e20 (2 pt corners, 1 pt
//!   outline, 10 % white while hovered, 70 % while pressed, 30 % when disabled) and
//!   `SelectorButton` 0x100f03ad4 / `resized` 0x100f03f64;
//! - the DRUM CAPTURE buttons: `WaveViewArea::ControlView` (constructor 0x100c98420,
//!   `resized` 0x100c9992c, `showView` 0x100c94ca8): the third page of the column at the left
//!   of the waveforms; the capture area on the waveform: `drawDrumCaptureAreaGL` 0x102126b20.
//!
//! Colours are those the code names: the colour table's dark values for the ids it looks up
//! and the constants of its palette (#000000, #323232, #969696, #646464, #FFFFFF, #1473EB,
//! #00FF00, #8214EB at 40 %). Icons are rekordbox's own SVG files, read from the installed
//! rekordbox when the panel is drawn (plain shapes stand in when rekordbox is not there).
//! Not traced: the slot waveform's colours (rekordbox draws the track's own waveform image
//! in the chosen waveform colour), the small rotary knob's look, the level meter's scale.

use crate::app::*;
use crate::groove::{self, DECKS, ROWS, SLOTS};
use crate::theme::*;
use crate::ui_kit as kit;
use crate::ui_player::tip;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, Ui};

/// The panel's height (the player grows by one point more).
pub const PANEL_H: f32 = 52.0;
/// GROOVE CIRCUIT panel face: id not traced; the #1E1E1E pane colour 0x03000009 (D #1E1E1E L #F0F0F0).
const PANEL: Tc = tc(0x1E1E1E, 0xF0F0F0);
const BTN_FILL: Tc = gr(0x323232);
const OUTLINE: Tc = gr(0x969696);
const GREEN: Color32 = hex(0x00FF00);
const SKINS: &str = "/Applications/rekordbox 7/rekordbox.app/Contents/Resources/skins";

/// rekordbox's icon files: (name, file under its skins folder, the SVG's size, the box its
/// shapes fill). JUCE fits that box, not the whole SVG, into the place a button gives its icon.
const ICONS: [(&str, &str, (f32, f32), [f32; 4]); 14] = [
    ("play", "groovecircuit/gc_play.svg", (11.0, 10.0), [2.0, 1.36, 8.49, 8.64]),
    ("stop", "groovecircuit/gc_stop.svg", (11.0, 10.0), [2.0, 1.5, 9.0, 8.5]),
    ("eject", "groovecircuit/gc_eject.svg", (10.0, 10.0), [1.0, 1.0, 9.0, 9.0]),
    ("left", "groovecircuit/gc_left.svg", (13.0, 12.0), [2.65, 0.4, 8.85, 11.6]),
    ("right", "groovecircuit/gc_right.svg", (13.0, 12.0), [3.65, 0.4, 9.85, 11.6]),
    ("load", "groovecircuit/gc_load.svg", (15.0, 12.0), [0.25, 0.0, 14.25, 12.0]),
    ("save", "groovecircuit/gc_save.svg", (13.0, 12.0), [0.25, 0.0, 12.25, 12.0]),
    ("capture", "groovecircuit/gc_capture.svg", (14.0, 13.0), [0.27, 0.0, 13.72, 12.5]),
    ("caplen", "groovecircuit/gc_capture_length.svg", (3.0, 3.0), [0.0, 0.5, 3.0, 2.5]),
    ("pulldown", "pullDown.svg", (11.0, 10.0), [1.33, 3.0, 10.33, 8.0]),
    ("up", "wave_up.svg", (10.0, 10.0), [0.5, 2.0, 9.5, 7.0]),
    ("down", "wave_down.svg", (10.0, 10.0), [0.5, 3.0, 9.5, 8.0]),
    ("gc_on", "globalsection/glb_gc_ON.svg", (20.0, 20.0), [0.0, 0.0, 20.0, 20.0]),
    ("gc_off", "globalsection/glb_gc_OFF.svg", (20.0, 20.0), [0.0, 0.0, 20.0, 20.0]),
];

fn texture(app: &mut App, name: &'static str) -> Option<egui::TextureHandle> {
    if let Some(t) = app.gc.textures.get(name) {
        return t.clone();
    }
    let tex = ICONS.iter().find(|i| i.0 == name).and_then(|(_, file, _, _)| {
        let bytes = std::fs::read(std::path::Path::new(SKINS).join(file)).ok()?;
        // Four pixels per point: sharp on a Retina screen at any of the sizes used.
        let img = egui_extras::image::load_svg_bytes_with_size(&bytes, Some(egui::SizeHint::Scale(4.0.into()))).ok()?;
        Some(app.ctx.load_texture(format!("gc_{name}"), img, egui::TextureOptions::LINEAR))
    });
    app.gc.textures.insert(name, tex.clone());
    tex
}

/// Draw an icon so that its shapes fit `dest` (centred, in proportion), tinted `tint`.
fn icon(app: &mut App, p: &Painter, name: &'static str, dest: Rect, tint: Color32) {
    let Some((_, _, vb, bb)) = ICONS.iter().find(|i| i.0 == name).copied() else { return };
    let (bw, bh) = (bb[2] - bb[0], bb[3] - bb[1]);
    let k = (dest.width() / bw).min(dest.height() / bh);
    let o = dest.center() - vec2(bw, bh) * k / 2.0 - vec2(bb[0], bb[1]) * k;
    match texture(app, name) {
        Some(t) => {
            p.image(t.id(), Rect::from_min_size(o, vec2(vb.0, vb.1) * k), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        None => fallback_icon(p, name, Rect::from_center_size(dest.center(), vec2(bw, bh) * k), tint),
    }
}

/// Plain shapes for when rekordbox's skin files are not installed.
fn fallback_icon(p: &Painter, name: &str, r: Rect, c: Color32) {
    let s = Stroke::new(1.2_f32, c);
    match name {
        "play" => {
            p.add(egui::Shape::convex_polygon(vec![r.left_top(), pos2(r.max.x, r.center().y), r.left_bottom()], c, Stroke::NONE));
        }
        "stop" => {
            p.rect_filled(r, 0.5, c);
        }
        "eject" => {
            p.add(egui::Shape::convex_polygon(vec![pos2(r.center().x, r.min.y), pos2(r.max.x, r.min.y + r.height() * 0.62), pos2(r.min.x, r.min.y + r.height() * 0.62)], c, Stroke::NONE));
            p.rect_filled(Rect::from_min_max(pos2(r.min.x, r.max.y - r.height() * 0.19), r.max), 0.0, c);
        }
        "left" => {
            p.add(egui::Shape::line(vec![r.right_top(), pos2(r.min.x, r.center().y), r.right_bottom()], s));
        }
        "right" => {
            p.add(egui::Shape::line(vec![r.left_top(), pos2(r.max.x, r.center().y), r.left_bottom()], s));
        }
        "pulldown" | "down" => {
            p.add(egui::Shape::line(vec![r.left_top(), pos2(r.center().x, r.max.y), r.right_top()], Stroke::new(1.0_f32, c)));
        }
        "up" => {
            p.add(egui::Shape::line(vec![r.left_bottom(), pos2(r.center().x, r.min.y), r.right_bottom()], Stroke::new(1.0_f32, c)));
        }
        "caplen" => {
            p.add(egui::Shape::convex_polygon(vec![r.left_top(), r.right_top(), pos2(r.center().x, r.max.y)], c, Stroke::NONE));
        }
        "load" => {
            p.rect_stroke(Rect::from_min_max(pos2(r.min.x, r.min.y + 2.0), r.max), 0.0, s, egui::StrokeKind::Inside);
            p.line_segment([r.left_top(), pos2(r.min.x + r.width() * 0.4, r.min.y)], s);
        }
        "save" => {
            p.rect_stroke(r, 0.0, s, egui::StrokeKind::Inside);
            p.rect_stroke(Rect::from_min_max(pos2(r.min.x + 3.0, r.min.y), pos2(r.max.x - 3.0, r.center().y)), 0.0, s, egui::StrokeKind::Inside);
        }
        "capture" => {
            p.rect_stroke(Rect::from_min_max(r.min, pos2(r.max.x - 3.0, r.max.y - 3.0)), 2.0, s, egui::StrokeKind::Inside);
            p.circle_stroke(pos2(r.max.x - 2.0, r.max.y - 2.0), 1.6, s);
        }
        _ => {
            // The top bar's tile: two crossing lines.
            p.rect_filled(r, 2.0, if name == "gc_on" { BLUE } else { BTN });
            let s = Stroke::new(2.0_f32, WHITE);
            p.line_segment([r.left_top() + vec2(4.0, 6.0), r.right_bottom() - vec2(5.0, 6.0)], s);
            p.line_segment([r.left_bottom() + vec2(4.0, -6.0), r.right_top() + vec2(-5.0, 6.0)], s);
        }
    }
}

/// How a `CustomTextButton` shows hover, press and disabled: (opacity, hover highlight).
fn feedback(resp: &egui::Response, enabled: bool) -> (f32, bool) {
    if !enabled {
        (0.3, false)
    } else if resp.is_pointer_button_down_on() {
        (0.7, false)
    } else {
        (1.0, resp.hovered())
    }
}

fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}

/// A, B, SINGLE, MULTI: a 1 pt outline (#969696, blue when on), white bold text.
fn outline_button(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, label: &str, on: bool, font: FontId) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let (a, hot) = feedback(&resp, true);
    let p = ui.painter();
    p.rect_stroke(r, 2.0, Stroke::new(1.0_f32, fade(if on { BLUE.c() } else { OUTLINE.c() }, a)), egui::StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, label, font, fade(WHITE.c(), a));
    if hot {
        p.rect_filled(r, 2.0, Color32::from_white_alpha(26));
    }
    resp
}

/// A filled icon button (Load, Save, Eject).
fn icon_button(app: &mut App, ui: &mut Ui, r: Rect, id: impl std::hash::Hash, name: &'static str, at: Rect, enabled: bool, sense: Sense) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), if enabled { sense } else { Sense::hover() });
    let (a, hot) = feedback(&resp, enabled);
    let p = ui.painter().clone();
    p.rect_filled(r, 2.0, fade(BTN_FILL.c(), a));
    icon(app, &p, name, at.translate(r.min.to_vec2()), fade(WHITE.c(), a));
    if hot {
        p.rect_filled(r, 2.0, Color32::from_white_alpha(26));
    }
    resp
}

/// An 18 pt rotary knob for a value 0..100 (step 1): drag up or down, double click for 50.
fn small_knob(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, value: f32, enabled: bool) -> (Option<f32>, egui::Response) {
    let resp = ui.interact(r, ui.id().with(id), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let mut out = None;
    let acc = resp.id.with("acc");
    if resp.double_clicked() {
        out = Some(50.0);
    } else if resp.dragged() {
        // (The fraction of a step a slow drag leaves is kept for the next frame.)
        let mut v: f32 = ui.ctx().data_mut(|d| d.get_temp(acc)).unwrap_or(value);
        v = (v - resp.drag_delta().y * 100.0 / 150.0).clamp(0.0, 100.0);
        ui.ctx().data_mut(|d| d.insert_temp(acc, v));
        if v.round() != value {
            out = Some(v.round());
        }
    } else if resp.drag_started() || !resp.is_pointer_button_down_on() {
        ui.ctx().data_mut(|d| d.insert_temp(acc, value));
    }
    let v = out.unwrap_or(value) / 100.0;
    let a = if enabled { 1.0 } else { 0.3 };
    let p = ui.painter();
    let c = r.center();
    // 3.927 to 8.639 rad from 12 o'clock, clockwise (the slot knob's rotary range): 270 degrees.
    let ang = |x: f32| (-225.0 + 270.0 * x).to_radians();
    let pts: Vec<Pos2> = (0..=30).map(|i| c + vec2(ang(i as f32 / 30.0).cos(), ang(i as f32 / 30.0).sin()) * 7.0).collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0_f32, fade(if resp.hovered() || resp.dragged() { gy(0x464646) } else { BTN_FILL.c() }, a))));
    let t = ang(v);
    p.line_segment([c + vec2(t.cos(), t.sin()) * 3.5, c + vec2(t.cos(), t.sin()) * 8.0], Stroke::new(1.6_f32, fade(WHITE.c(), a)));
    (out, resp)
}

/// The top bar's GROOVE CIRCUIT tile (rekordbox's `glb_gc_ON.svg` / `glb_gc_OFF.svg`).
pub fn header_icon(app: &mut App, ui: &Ui, tile: Rect, lit: bool) {
    let p = ui.painter().clone();
    icon(app, &p, if lit { "gc_on" } else { "gc_off" }, tile, WHITE.c());
}

/// The top bar's button pressed: show or hide the panel.
pub fn toggle_panel(app: &mut App) {
    if !app.groove_available() {
        app.toast_err("GROOVE CIRCUIT needs [Preferences] > [Extensions] > [STEMS] > \"Enable the STEMS Function\" and \"Enable the GROOVE CIRCUIT Function\".");
        return;
    }
    app.prefs.groove.panel = !app.prefs.groove.panel;
    app.save_prefs();
}

/// All decks' panels (each in the last 52 pt of its deck's rectangle), the capture area on
/// the waveforms, drops from Finder and the preset windows. Called once per frame after the
/// decks are drawn.
pub fn row(app: &mut App, ui: &mut Ui, decks: &[Rect]) {
    if !app.groove_visible() {
        return;
    }
    for (n, dr) in decks.iter().enumerate().take(DECKS) {
        let r = Rect::from_min_max(pos2(dr.min.x + 1.0, dr.max.y - PANEL_H), pos2(dr.max.x - 1.0, dr.max.y));
        // (The strip above the panel belongs to the deck: its pad area's colour.)
        ui.painter().rect_filled(Rect::from_min_max(pos2(dr.min.x, r.min.y - 1.0), dr.max), 0.0, gy(0x141414));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).id_salt(("groove", n)));
        child.set_clip_rect(r.intersect(ui.clip_rect()));
        panel(app, &mut child, r, n);
    }
    capture_area(app, ui);
    dropped_files(app, ui);
    dialogs(app, ui.ctx());
    if (0..DECKS).any(|d| groove::ENGINES[d].lock().as_deref().is_some_and(|e| e.sounding || e.any_played())) || app.gc.capture_on.is_some() {
        ui.ctx().request_repaint();
    }
}

/// One deck's panel.
fn panel(app: &mut App, ui: &mut Ui, r: Rect, deck: usize) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    app.gc.panel_rects[deck] = r;
    let w = r.width().floor() as i32;
    let at = |x: i32, y: i32, w: i32, h: i32| Rect::from_min_size(pos2(r.min.x + x as f32, r.min.y + y as f32), vec2(w as f32, h as f32));
    let set = app.gc.set.decks[deck].clone();
    let bank = set.bank.min(1) as usize;

    // ---- row 1: DRUM LOOP BANK A / B and the four slots of the bank
    let bank_tip = "DRUM LOOP BANK switching:\nSwitches Banks of DRUM SWAP.";
    if tip(outline_button(ui, at(10, 4, 18, 18), "bankA", "A", bank == 0, caps(9.0)), app, bank_tip).clicked() {
        app.groove_set_bank(deck, 0);
    }
    if tip(outline_button(ui, at(30, 4, 18, 18), "bankB", "B", bank == 1, caps(9.0)), app, bank_tip).clicked() {
        app.groove_set_bank(deck, 1);
    }
    let slot_w = ((w - 98) / 4).max(40);
    let states: [groove::RowState; ROWS] = {
        let g = groove::ENGINES[deck].lock();
        std::array::from_fn(|i| g.as_deref().map(|e| e.row_state(i)).unwrap_or_default())
    };
    for row in 0..ROWS {
        let sr = at(58 + row as i32 * (slot_w + 10), 4, slot_w, 18);
        app.gc.slot_rects[deck][row] = sr;
        slot(app, ui, sr, deck, row + ROWS * bank, states[row]);
    }

    // ---- row 2
    let v9 = (w - 377) / 3;
    if tip(outline_button(ui, at(10, 30, 60, 18), "single", "SINGLE", !set.multi, caps(9.0)), app, "DRUM SWAP SINGLE MODE:\nPlays one DRUM SWAP SLOT.\nPlaying DRUM SWAP mutes ACTIVE STEM(DRUMS) of DECK.").clicked() {
        app.groove_set_multi(deck, false);
    }
    if tip(outline_button(ui, at(72, 30, 60, 18), "multi", "MULTI", set.multi, caps(9.0)), app, "DRUM SWAP MULTI MODE:\nPlays multiple DRUM SWAP SLOTs.").clicked() {
        app.groove_set_multi(deck, true);
    }
    p.text(at(v9 + 132, 30, 33, 18).left_center(), Align2::LEFT_CENTER, "GAIN", caps(9.0), WHITE.c());
    let (gv, gresp) = small_knob(ui, at(v9 + 165, 30, 18, 18), "gain", app.gc.gain[deck], true);
    if let Some(v) = gv {
        app.groove_set_deck_gain(deck, v);
    }
    tip(gresp, app, "DRUM SWAP GAIN:\nAdjusts volume of all DRUM SWAP SLOTs of the entire DECK.");

    // Load / Save Drum Preset: the menus.
    let (lr, sv) = (at(2 * v9 + 183, 30, 18, 18), at(2 * v9 + 206, 30, 18, 18));
    let icon_at = Rect::from_min_size(pos2(2.0, 3.0), vec2(14.0, 12.0));
    preset_menu(app, ui, lr, deck, true);
    preset_menu(app, ui, sv, deck, false);
    for (rr, name) in [(lr, "load"), (sv, "save")] {
        let hovered = ui.rect_contains_pointer(rr);
        p.rect_filled(rr, 2.0, BTN_FILL);
        icon(app, &p, name, icon_at.translate(rr.min.to_vec2()), WHITE.c());
        if hovered {
            p.rect_filled(rr, 2.0, Color32::from_white_alpha(26));
        }
    }

    // DRUM RELEASE FX: the name (on while it is held), the pull-down, and the beat.
    let fr = at(3 * v9 + 224, 30, 90, 18);
    let name_r = Rect::from_min_size(fr.min, vec2(73.0, 18.0));
    let arrow_r = Rect::from_min_size(pos2(fr.max.x - 16.0, fr.min.y), vec2(16.0, 18.0));
    let kind = set.fx_type.min(2) as usize;
    let nresp = ui.interact(name_r, ui.id().with("rfx"), Sense::click_and_drag());
    let held = nresp.is_pointer_button_down_on();
    app.groove_release_fx(deck, held || app.gc.rfx_held[deck] && scripted_hold(deck));
    let lit = app.groove_release_held(deck);
    let round_l = egui::CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 };
    let round_r = egui::CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 };
    p.rect_filled(name_r, round_l, BTN_FILL);
    if lit {
        p.rect_stroke(name_r, round_l, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
    }
    p.with_clip_rect(name_r).text(name_r.center(), Align2::CENTER_CENTER, groove::RFX_NAMES[kind], caps(9.0), WHITE.c());
    if nresp.hovered() && !lit {
        p.rect_filled(name_r, round_l, Color32::from_white_alpha(26));
    }
    tip(nresp, app, "DRUM RELEASE FX On/Off button:\nEffects are applied on DRUMS Stem.");
    let aresp = ui.interact(arrow_r, ui.id().with("rfxsel"), Sense::click());
    p.rect_filled(arrow_r, round_r, BTN_FILL);
    icon(app, &p, "pulldown", Rect::from_min_size(arrow_r.min + vec2(4.0, 7.0), vec2(8.0, 4.0)), WHITE.c());
    if aresp.hovered() {
        p.rect_filled(arrow_r, round_r, Color32::from_white_alpha(26));
    }
    crate::ui_browser::dropdown(ui, &aresp, "rfxpop", |ui| {
        ui.set_min_width(110.0);
        for (i, name) in groove::RFX_NAMES.iter().enumerate() {
            if ui.selectable_label(i == kind, *name).clicked() {
                app.groove_select_fx(deck, i as u8);
                ui.memory_mut(|m| m.close_popup());
            }
        }
    });
    tip(aresp, app, "DRUM RELEASE FX drop-down menu:\nSelect FX to apply.");
    let beat = set.fx_beats[kind];
    let (dn, up) = (at(3 * v9 + 323, 30, 12, 18), at(3 * v9 + 355, 30, 12, 18));
    let chev = Rect::from_min_size(pos2(3.0, 4.0), vec2(6.0, 10.0));
    for (rr, name, dir, text) in [(dn, "left", -1, "DRUM RELEASE FX Beat Down button:\nChange the beat value to sync the effect sound with."), (up, "right", 1, "DRUM RELEASE FX Beat Up button:\nChange the beat value to sync the effect sound with.")] {
        let resp = ui.interact(rr, ui.id().with(("rfxbeat", dir)), Sense::click());
        let (a, hot) = feedback(&resp, true);
        icon(app, &p, name, chev.translate(rr.min.to_vec2()), fade(WHITE.c(), a));
        if hot {
            p.rect_filled(rr, 2.0, Color32::from_white_alpha(26));
        }
        if tip(resp, app, text).clicked() {
            app.groove_fx_beat_step(deck, dir);
        }
    }
    p.text(Rect::from_min_max(dn.right_top(), up.left_bottom()).center(), Align2::CENTER_CENTER, groove::beat_text(beat), caps(10.0), WHITE.c());
}

/// The scripted run holds the release FX without a pointer on it (see `shot_events`).
fn scripted_hold(deck: usize) -> bool {
    SCRIPT_HOLD.load(std::sync::atomic::Ordering::Relaxed) == deck as i32
}
static SCRIPT_HOLD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// One DRUM SWAP slot. `idx`: row + 4 * bank.
fn slot(app: &mut App, ui: &mut Ui, r: Rect, deck: usize, idx: usize, st: groove::RowState) {
    let p = ui.painter().clone();
    let (row, bank) = (idx % ROWS, idx / ROWS);
    let info = app.groove_slot(deck, idx).cloned();
    let loaded = info.is_some() && app.groove_ready(deck, idx);
    let playing = st.playing == Some(bank);
    let waiting = st.pending == Some(bank);
    let capture = app.gc.capture_on.is_some();
    let (w, h) = (r.width(), r.height());
    let body = Rect::from_min_max(r.min, pos2(r.max.x - 2.0, r.max.y));
    p.rect_filled(r, 0.0, BG);
    let resp = ui.interact(r, ui.id().with(("slot", idx)), Sense::click());
    // The capture area dropped here: this slot captures it.
    if app.gc.drag_capture.is_some() && ui.rect_contains_pointer(r) {
        p.rect_stroke(r, 0.0, egui::Stroke::new(2.0, hex(0x8214EB)), egui::StrokeKind::Inside);
        if ui.input(|i| i.pointer.any_released()) {
            app.gc.drag_capture = None;
            app.groove_capture(deck, idx);
        }
    }
    let over = ui.rect_contains_pointer(r) && app.drag.is_none();
    let controls = over && !capture;

    // The level meter: the 2 pt at the right.
    let db = app.gc.meter[deck][row];
    if playing && db > -60.0 {
        let k = ((db + 40.0) / 40.0).clamp(0.0, 1.0);
        p.rect_filled(Rect::from_min_max(pos2(r.max.x - 2.0, r.max.y - h * k), r.max), 0.0, if db > -1.0 { hex(0xE63737) } else { GREEN });
    }

    if !controls {
        // The waveform over the top 15 pt, beat marks under it, a 30 % black shade over both.
        if let (Some(wave), Some(s)) = (app.gc.wave[deck][idx].as_ref(), info.as_ref()) {
            let wr = Rect::from_min_max(body.min, pos2(body.max.x, body.max.y - 3.0));
            let n = wave.len().max(1);
            let mut mesh = kit::Bars::new();
            let cols = wr.width().floor().max(1.0) as usize;
            for x in 0..cols {
                let v = wave[(x * n / cols).min(n - 1)] as f32 / 255.0;
                let hh = (v * wr.height()).max(1.0);
                mesh.column(wr.min.x + x as f32 + 0.5, wr.center().y - hh / 2.0, wr.center().y + hh / 2.0, hex(0x1E8CFF));
            }
            mesh.paint(&p);
            let beats = groove::loop_beats(s.length.max(0) as usize, s.samplingrate, s.bpm);
            if (1..=32).contains(&beats) {
                for b in 0..beats {
                    let x = (b as f32 / beats as f32 * (body.width() - 2.0)).floor().clamp(0.0, body.width() - 2.0);
                    p.rect_filled(Rect::from_min_size(pos2(body.min.x + x, r.max.y - 3.0), vec2(2.0, 2.0)), 0.0, WHITE);
                }
            }
            p.rect_filled(wr, 0.0, Color32::from_black_alpha(77));
        }
        // The play position: a green bar along the bottom 2 pt.
        if playing {
            p.rect_filled(Rect::from_min_size(pos2(body.min.x, r.max.y - 2.0), vec2(body.width() * st.progress.clamp(0.0, 1.0), 2.0)), 0.0, GREEN);
        }
        if let (true, Some(s)) = (app.prefs.groove.show_title, info.as_ref()) {
            p.with_clip_rect(Rect::from_min_max(pos2(r.min.x + 4.0, r.min.y), body.max)).text(pos2(r.min.x + 4.0, r.min.y + 8.0), Align2::LEFT_CENTER, &s.name, bold(9.0), WHITE.c());
        }
    } else {
        // Play (stop while it plays), Eject, the slot's GAIN, BPM | beats.
        let pr = Rect::from_min_size(r.min + vec2(4.0, 2.0), vec2(24.0, 14.0));
        let presp = ui.interact(pr, ui.id().with(("play", idx)), if loaded { Sense::click() } else { Sense::hover() });
        let (a, hot) = feedback(&presp, loaded);
        p.rect_filled(pr, 2.0, fade(BTN_FILL.c(), a));
        p.rect_stroke(pr.shrink(0.5), 2.0, Stroke::new(1.0_f32, fade(GREEN, a)), egui::StrokeKind::Middle);
        icon(app, &p, if playing || waiting { "stop" } else { "play" }, Rect::from_min_size(pr.min + vec2(8.0, 3.0), vec2(8.0, 8.0)), fade(WHITE.c(), a));
        if hot {
            p.rect_filled(pr, 2.0, Color32::from_white_alpha(26));
        }
        // (rekordbox's button acts on the press, not on the release.)
        if crate::ui_player::press_fired(ui, &presp) && loaded {
            app.groove_slot_play(deck, idx);
        }
        tip(presp, app, "Play button:\nPlay back audio of DRUM SWAP SLOT.\nClick to stop playing during playback.");
        let er = Rect::from_min_size(pos2(pr.max.x + 2.0, pr.min.y), vec2(16.0, 14.0));
        let eresp = icon_button(app, ui, er, ("eject", idx), "eject", Rect::from_min_size(pos2(4.0, 3.0), vec2(8.0, 8.0)), loaded, Sense::click());
        if eresp.clicked() {
            app.groove_eject(deck, idx, false);
        }
        tip(eresp, app, "EJECT (Unload) button:\nEject drum loop audio from the DRUM SWAP SLOT.");
        let room = (w - 46.0 - 54.0).max(0.0);
        let kr = Rect::from_min_size(pos2(r.min.x + 46.0 + (room / 2.0).floor(), r.min.y), vec2(18.0, 18.0));
        let gain = info.as_ref().map(|s| s.gain).unwrap_or(50.0);
        let (kv, kresp) = small_knob(ui, kr, ("slotgain", idx), gain, loaded);
        if let Some(v) = kv {
            app.groove_set_slot_gain(deck, idx, v);
        }
        if kresp.drag_stopped() || kresp.double_clicked() {
            app.groove_save();
        }
        let shown = if app.prefs.tooltips { format!("DRUM SWAP SLOT GAIN:\nAdjust volume of drum loop audio.\n{} dB", groove::knob_text(kv.unwrap_or(gain))) } else { format!("{} dB", groove::knob_text(kv.unwrap_or(gain))) };
        kresp.on_hover_text(shown);
        if let Some(s) = info.as_ref().filter(|_| loaded) {
            let beats = groove::loop_beats(s.length.max(0) as usize, s.samplingrate, s.bpm);
            let bpm_r = Rect::from_min_size(pos2(r.max.x - 35.0, r.min.y + 4.0), vec2(17.0, 11.0));
            let beat_r = Rect::from_min_size(pos2(r.max.x - 15.0, r.min.y + 4.0), vec2(12.0, 11.0));
            if s.bpm > 0.0 {
                p.text(bpm_r.center(), Align2::CENTER_CENTER, format!("{:.0}", s.bpm), bold(9.0), WHITE.c());
                p.line_segment([pos2(r.max.x - 15.5, r.min.y + 5.0), pos2(r.max.x - 15.5, r.min.y + 14.0)], Stroke::new(1.0_f32, WHITE));
                p.text(beat_r.center(), Align2::CENTER_CENTER, format!("{beats}"), bold(9.0), WHITE.c());
            }
            let info_resp = ui.interact(Rect::from_min_max(bpm_r.min, beat_r.max), ui.id().with(("slotinfo", idx)), Sense::hover());
            info_resp.on_hover_text(format!("Track title: {}\nBPM: {:.0}\nNumber of beats: {:2}", s.name, s.bpm, beats));
        }
    }

    // DRUM CAPTURE is on: every slot is a target, and a press on it captures.
    if capture {
        p.rect_stroke(r, 0.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
        if crate::ui_player::press_fired(ui, &resp) {
            app.groove_capture(deck, idx);
        }
    }
    // A track dragged from the browser or from a deck loads here.
    if let Some(ids) = app.drag.clone() {
        if ui.rect_contains_pointer(r) {
            p.rect_stroke(r, 0.0, Stroke::new(2.0_f32, BLUE), egui::StrokeKind::Inside);
            if ui.input(|i| i.pointer.any_released()) {
                if let Some(id) = ids.first() {
                    app.groove_load_track(deck, idx, *id);
                }
            }
        }
    }
}

/// The Load / Save Drum Preset menus, on an unseen button over `r` (the caller draws it).
fn preset_menu(app: &mut App, ui: &mut Ui, r: Rect, deck: usize, load: bool) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).id_salt(("gcpreset", load)));
    let presets: Vec<(i64, String)> = app.gc.set.presets.iter().map(|p| (p.id, p.name.clone())).collect();
    let out = egui::menu::menu_custom_button(&mut child, egui::Button::new("").frame(false).min_size(r.size()), |ui| {
        ui.set_min_width(180.0);
        if load {
            ui.menu_button("Factory presets", |ui| {
                if ui.button(groove::FACTORY_KIT).clicked() {
                    app.groove_load_factory(&[deck]);
                    ui.close_menu();
                }
                // rekordbox offers these as a download from its server; recordbox has no
                // such download.
                for k in groove::DOWNLOAD_KITS {
                    ui.add_enabled(false, egui::Button::new(k)).on_disabled_hover_text("A download from rekordbox's server: not available in recordbox.");
                }
            });
            ui.menu_button("User presets", |ui| {
                if presets.is_empty() {
                    ui.add_enabled(false, egui::Button::new("(none)"));
                }
                for (id, name) in &presets {
                    if ui.button(name).clicked() {
                        app.groove_preset_load(deck, *id);
                        ui.close_menu();
                    }
                }
            });
        } else {
            if ui.add_enabled(presets.len() < groove::MAX_USER_PRESETS, egui::Button::new("Save as New Preset")).clicked() {
                app.gc.name_dialog = Some((None, deck, "Untitled Preset".into()));
                ui.close_menu();
            }
            for (id, name) in &presets {
                ui.menu_button(name, |ui| {
                    if ui.button("Overwrite").clicked() {
                        app.groove_preset_overwrite(deck, *id);
                        ui.close_menu();
                    }
                    if ui.button("Rename").clicked() {
                        app.gc.name_dialog = Some((Some(*id), deck, name.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Delete").clicked() {
                        app.gc.confirm_delete = Some(*id);
                        ui.close_menu();
                    }
                });
            }
        }
    });
    if app.prefs.tooltips {
        out.response.on_hover_text(if load { "Load Drum Preset:\nLoads an Drum Preset on the BANK A and B DRUM SWAP SLOT." } else { "Save Drum Preset:\nSaves BANK A and B DRUM SWAP SLOT audio as an Drum Preset." });
    }
}

/// The preset name window ("Drum Preset name") and the delete question.
fn dialogs(app: &mut App, ctx: &egui::Context) {
    if let Some((id, deck, mut name)) = app.gc.name_dialog.clone() {
        let mut done: Option<bool> = None;
        kit::dialog(ctx, "Drum Preset name").default_width(300.0).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            let te = ui.add(egui::TextEdit::singleline(&mut name).desired_width(280.0));
            if !te.has_focus() && !te.lost_focus() {
                te.request_focus();
            }
            let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            match kit::dialog_buttons(ui, &["OK", "Cancel"]) {
                Some(0) => done = Some(true),
                Some(_) => done = Some(false),
                None if enter => done = Some(true),
                None => {}
            }
        });
        match done {
            Some(true) => {
                match id {
                    Some(id) => app.groove_preset_rename(id, &name),
                    None => {
                        app.groove_preset_save_new(deck, &name);
                    }
                }
                app.gc.name_dialog = None;
            }
            Some(false) => app.gc.name_dialog = None,
            None => app.gc.name_dialog = Some((id, deck, name)),
        }
    }
    if let Some(id) = app.gc.confirm_delete {
        let mut done: Option<bool> = None;
        kit::dialog(ctx, "recordbox").default_width(340.0).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            kit::message(ui, "Are you sure you want to delete this item?");
            match kit::dialog_buttons(ui, &["OK", "Cancel"]) {
                Some(0) => done = Some(true),
                Some(_) => done = Some(false),
                None => {}
            }
        });
        if let Some(yes) = done {
            if yes {
                app.groove_preset_delete(id);
            }
            app.gc.confirm_delete = None;
        }
    }
}

/// A file dropped from Finder onto a slot loads there. The files are left where they are for
/// the sampler panel and the browser (which take them into the collection, as rekordbox
/// registers a dropped file before it loads it); the window is told the pointer's place only
/// with its next move, so they wait for that.
fn dropped_files(app: &mut App, ui: &mut Ui) {
    let now = ui.input(|i| i.time);
    let files: Vec<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if !files.is_empty() {
        app.gc.dropped = Some((files, now));
    }
    let Some((files, t0)) = app.gc.dropped.clone() else { return };
    let moved = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_))));
    if !moved && now - t0 <= 0.6 {
        ui.ctx().request_repaint();
        return;
    }
    app.gc.dropped = None;
    let Some(pp) = ui.ctx().pointer_latest_pos().filter(|_| moved) else { return };
    let hit = (0..DECKS).flat_map(|d| (0..ROWS).map(move |r| (d, r))).find(|(d, r)| app.gc.panel_rects[*d].contains(pp) && app.gc.slot_rects[*d][*r].contains(pp));
    if let Some((d, r)) = hit {
        let bank = app.gc.set.decks[d].bank.min(1) as usize;
        match crate::importer::collect_audio(&files).into_iter().find(|f| f.is_file()) {
            Some(f) => {
                let name = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                app.groove_load_file(d, r + ROWS * bank, f.to_string_lossy().to_string(), name, 0.0, 0.0);
            }
            None => app.toast_err("This file cannot be loaded into the DRUM SWAP SLOT."),
        }
    }
}

/// The part of the track a capture would take, shaded on the waveform of the deck DRUM
/// CAPTURE is on: #8214EB at 40 %, from the playing position.
fn capture_area(app: &mut App, ui: &mut Ui) {
    let Some(deck) = app.gc.capture_on else { return };
    let Some((a, b)) = app.groove_capture_span(deck) else { return };
    let Some((r, pos, pps)) = app.deck_of(deck).detail_map else { return };
    let x = |t: f64| r.center().x + ((t - pos) * pps) as f32;
    let clip = Rect::from_min_max(pos2(r.min.x + 28.0, r.min.y), r.max);
    let area = Rect::from_min_max(pos2(x(a), r.min.y), pos2(x(b), r.max.y)).intersect(clip);
    if area.width() > 0.0 {
        ui.painter().with_clip_rect(clip).rect_filled(area, 0.0, Color32::from_rgba_unmultiplied(0x82, 0x14, 0xEB, 102));
        // The area can be dragged onto a DRUM SWAP slot, which captures it
        // (`DrumSwapSlot::itemDroppedFromWave`).
        let resp = ui.interact(area, ui.id().with(("gc_capture_drag", deck)), Sense::drag());
        if resp.drag_started() {
            app.gc.drag_capture = Some(deck);
        }
    }
    if app.gc.drag_capture.is_some() {
        if let Some(pp) = ui.ctx().pointer_latest_pos() {
            let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("gc_capture_ghost")));
            lp.rect_filled(Rect::from_min_size(pp + vec2(10.0, 4.0), vec2(40.0, 14.0)), 2.0, Color32::from_rgba_unmultiplied(0x82, 0x14, 0xEB, 160));
        }
        if !ui.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
            app.gc.drag_capture = None;
        }
    }
}

/// The column at the left of the waveforms. With Groove Circuit shown it has pages, turned
/// with the arrows at its top and bottom: the zoom buttons (drawn by the caller) and DRUM
/// CAPTURE (one button per deck beside the deck's waveform, and the length button between).
/// Returns true when it has drawn the column's page itself.
pub fn wave_column(app: &mut App, ui: &mut Ui, o: Pos2, waves_h: f32) -> bool {
    if !app.groove_visible() {
        return false;
    }
    let p = ui.painter().clone();
    let h = waves_h.floor() as i32;
    let at = |x: i32, y: i32, w: i32, hh: i32| Rect::from_min_size(pos2(o.x + x as f32, o.y + y as f32), vec2(w as f32, hh as f32));
    for (name, r, id) in [("up", at(6, 2, 10, 10), "gcpgup"), ("down", at(6, h - 12, 10, 10), "gcpgdn")] {
        let resp = ui.interact(r.expand(2.0), ui.id().with(id), Sense::click());
        let (a, _) = feedback(&resp, true);
        icon(app, &p, name, Rect::from_center_size(r.center(), vec2(9.0, 5.0)), fade(if resp.hovered() { WHITE.c() } else { gy(0xAAAAAA) }, a));
        if resp.clicked() {
            app.gc.page = if app.gc.page == 2 { 0 } else { 2 };
        }
    }
    if app.gc.page != 2 {
        return false;
    }
    capture_buttons(app, ui, o, waves_h);
    true
}

/// The DRUM CAPTURE page of the waveforms' control strip (its third page): one capture button
/// per deck and the length button. `o`: the strip's top left; `waves_h`: its height.
pub fn capture_buttons(app: &mut App, ui: &mut Ui, o: Pos2, waves_h: f32) {
    let p = ui.painter().clone();
    let h = waves_h.floor() as i32;
    let at = |x: i32, y: i32, w: i32, hh: i32| Rect::from_min_size(pos2(o.x + x as f32, o.y + y as f32), vec2(w as f32, hh as f32));
    // One DRUM CAPTURE button per deck, in the middle of the deck's lane.
    let count = app.deck_count() as i32;
    let lane = h / count.max(1);
    let first = (lane - 14) / 2;
    for n in 0..count {
        let r = at(4, first + n * lane, 14, 14);
        let on = app.gc.capture_on == Some(n as u8);
        let resp = ui.interact(r, ui.id().with(("gccap", n)), Sense::click());
        let (a, hot) = feedback(&resp, true);
        icon(app, &p, "capture", Rect::from_min_size(r.min, vec2(14.0, 13.0)), fade(if on { BLUE.c() } else { WHITE.c() }, a));
        if hot {
            p.rect_filled(r, 2.0, Color32::from_white_alpha(26));
        }
        if tip(resp, app, "DRUM CAPTURE:\nYou can extract DRUMS Stem.\nClicking DRUM SWAP SLOT while DRUM CAPTURE is ON loads the extracted DRUMS Stem.").clicked() {
            app.groove_capture_toggle(n as u8);
        }
    }
    // The length: its number in a 1 pt outline, a small triangle in the corner.
    let lr = at(4, (h - 14) / 2, 14, 14);
    let resp = ui.interact(lr, ui.id().with("gccaplen"), Sense::click());
    let (a, hot) = feedback(&resp, true);
    p.rect_stroke(lr, 2.0, Stroke::new(1.0_f32, fade(gy(0x646464), a)), egui::StrokeKind::Inside);
    p.text(lr.center() - vec2(0.5, 0.5), Align2::CENTER_CENTER, format!("{}", app.gc.capture_len), FontId::proportional(8.0), fade(WHITE.c(), a));
    icon(app, &p, "caplen", Rect::from_min_size(lr.min + vec2(10.0, 11.0), vec2(3.0, 2.0)), fade(WHITE.c(), a));
    if hot {
        p.rect_filled(lr, 2.0, Color32::from_white_alpha(26));
    }
    if tip(resp, app, "Change the DRUM CAPTURE length to 4, 8, 16, or 32 beats.").clicked() {
        app.groove_capture_length_next();
    }
}

/// Preferences > Extensions > STEMS: "Enable the GROOVE CIRCUIT Function", DRUM SWAP, DRUM
/// CAPTURE and Drum Loop Audio, as rekordbox orders and words them (`ExtensionEnableGrooveCircuit`
/// 0x10153d824, `ExtensionDrumSwap` 0x1019ec18c, `ExtensionDrumCapture` 0x1007fe180,
/// `ExtensionDrumLoopAudio` 0x10159e70c). All of it is greyed while STEMS is off.
pub fn prefs_section(app: &mut App, ui: &mut Ui) {
    let stems = app.prefs.show_stems;
    let before = app.prefs.groove.clone();
    ui.add_enabled_ui(stems, |ui| {
        // The switch on the window's black, as "Enable the STEMS Function" above it.
        let (band, _) = ui.allocate_exact_size(vec2(ui.available_width().min(kit::CARD_W), 39.0), Sense::hover());
        let mut head = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(band.min.x + 20.0, band.min.y + 10.0), band.max)).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if !stems {
            head.disable();
        }
        kit::toggle_raw(&mut head, &mut app.prefs.groove.enable, "Enable the GROOVE CIRCUIT Function");
        kit::card(ui, "DRUM SWAP", |ui| {
            kit::sub(ui, "Eject/Load Lock");
            kit::indent(ui, 84.0);
            if kit::radio(ui, app.prefs.groove.load_lock, "Lock") {
                app.prefs.groove.load_lock = true;
            }
            if kit::radio(ui, !app.prefs.groove.load_lock, "Unlock") {
                app.prefs.groove.load_lock = false;
            }
            kit::indent(ui, 52.0);
            ui.add_space(10.0);
            kit::toggle(ui, &mut app.prefs.groove.show_title, "Display track titles on DRUM SWAP SLOT");
        });
        kit::card(ui, "DRUM CAPTURE", |ui| {
            if kit::radio(ui, app.prefs.groove.capture_stem != 2, "DRUMS Stem") {
                app.prefs.groove.capture_stem = 1;
            }
            if kit::radio(ui, app.prefs.groove.capture_stem == 2, "Original track") {
                app.prefs.groove.capture_stem = 2;
            }
            ui.add_space(6.0);
            kit::sub(ui, "Location of Files");
            let dir = crate::sampler_ext::capture_dir(&app.prefs.groove.capture_dir).to_string_lossy().to_string();
            let n = dir.chars().count();
            let shown = if n > 48 { format!("…{}", dir.chars().skip(n - 47).collect::<String>()) } else { dir.clone() };
            kit::combo_row(ui, 22.0, |ui| {
                kit::combo("gc_capdir", 410.0, shown, 14.0).show_ui(ui, |ui| {
                    // (In rekordbox choosing an entry here writes the sampler's capture
                    // folder, not this one: a slip that is not copied.)
                    if ui.selectable_label(app.prefs.groove.capture_dir.is_empty(), crate::sampler_ext::capture_dir("").to_string_lossy()).clicked() {
                        app.prefs.groove.capture_dir.clear();
                    }
                });
                ui.add_space(10.0);
                if ui.add(kit::btn("Browse").min_size(vec2(100.0, 22.0))).clicked() {
                    if let Some(d) = rfd::FileDialog::new().set_title("Location of Files").pick_folder() {
                        app.prefs.groove.capture_dir = d.to_string_lossy().to_string();
                    }
                }
            });
            kit::note(ui, "recordbox writes a captured drum loop as a 16-bit WAV file (rekordbox uses a file type of its own, .rbsc, for captures of the DRUMS Stem).");
        });
        kit::card(ui, "Drum Loop Audio", |ui| {
            kit::toggle(ui, &mut app.prefs.groove.auto_import, "Automatically import audio files");
            kit::note(ui, "The first time the GROOVE CIRCUIT panel is shown, rekordbox's factory drum loops are unpacked from the installed rekordbox into recordbox's data folder and loaded on the decks.");
        });
    });
    if app.prefs.groove != before {
        app.save_prefs();
    }
}

// ------------------------------------------------------------------------------- per frame, and the scripted run

fn shot_flags() -> Vec<String> {
    std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default()
}

/// Called at the start of every frame, in every mode: Groove Circuit's housekeeping, and with
/// `--screenshot` the switches for looking at the panel:
///
/// - `PREPDECK_SHOT_VIEW=perfgroove` (or `perfgroove4` for 4Deck Horizontal): PERFORMANCE
///   mode with the panel shown;
/// - `PREPDECK_SHOT_PLAYER=gchover` / `gcplay` / `gccapture`: the pointer over slot 2 of
///   deck 1; slot 1 playing; the DRUM CAPTURE page with capture on for deck 1;
/// - `PREPDECK_SHOT_PLAYER=gctest` with `PREPDECK_BENCH=<frames>` (so that deck 1 plays):
///   the scripted pointer run, which presses every control and prints the state after each
///   step (lines starting "gctest:").
pub fn frame_hook(app: &mut App, raw: &mut egui::RawInput) {
    if let Some((_, frame)) = app.screenshot.clone() {
        shot_events(app, frame, raw);
    }
    app.groove_poll();
}

/// The middle of deck 1's DRUM CAPTURE area on its waveform (for the scripted run).
fn cap_c(app: &App) -> Pos2 {
    let Some((a, b)) = app.groove_capture_span(0) else { return Pos2::ZERO };
    let Some((r, pos, pps)) = app.deck_of(0).detail_map else { return Pos2::ZERO };
    let x = |t: f64| r.center().x + ((t - pos) * pps) as f32;
    let xa = x(a).max(r.min.x + 30.0);
    let xb = x(b).min(r.max.x);
    pos2((xa + xb) / 2.0, r.center().y)
}

fn shot_events(app: &mut App, frame: u32, raw: &mut egui::RawInput) {
    let view = std::env::var("PREPDECK_SHOT_VIEW").unwrap_or_default();
    if !view.starts_with("perfgroove") {
        return;
    }
    let flags = shot_flags();
    let has = |f: &str| flags.iter().any(|x| x == f);
    let test = has("gctest");
    if frame == 0 {
        app.prefs.app_mode = 1;
        app.prefs.show_stems = true;
        app.prefs.perf_four = view == "perfgroove4";
        app.prefs.groove.enable = true;
        app.prefs.rb.groove_circuit = true;
        // (The scripted run opens the panel itself, with the top bar's button.)
        app.prefs.groove.panel = !test;
        app.prefs.perf_fx = false;
        app.prefs.perf_sampler = false;
    }
    let slot_c = |app: &App, row: usize| app.gc.slot_rects[0][row].center();
    if !test {
        // The states for the pictures.
        if frame == 100 && (has("gcplay") || has("gchover")) {
            let d = app.px.active as usize;
            if !app.player.decks[d].lock().playing {
                app.play_pause();
            }
            if !app.px.muted {
                app.toggle_mute();
            }
        }
        if frame == 104 && has("gcplay") {
            app.groove_slot_play(app.px.active as usize, 0);
        }
        if has("gchover") && frame >= 100 {
            raw.events.push(egui::Event::PointerMoved(slot_c(app, 1)));
        }
        if frame == 100 && has("gccapture") {
            app.gc.page = 2;
            app.gc.capture_len = 8;
            app.groove_capture_toggle(0);
        }
        return;
    }

    // ---- the scripted run
    const START: u32 = 150;
    const SPAN: u32 = 36;
    if frame == 130 {
        fake_stems(app, 0);
    }
    if frame < START {
        return;
    }
    let i = ((frame - START) / SPAN) as usize;
    let f = (frame - START) % SPAN;
    let panel = app.gc.panel_rects[0];
    let w = panel.width().floor();
    let v9 = ((w as i32 - 377) / 3) as f32;
    let pp = |x: f32, y: f32| pos2(panel.min.x + x, panel.min.y + y);
    let play = |app: &App, row: usize| app.gc.slot_rects[0][row].min + vec2(16.0, 9.0);
    let eject = |app: &App, row: usize| app.gc.slot_rects[0][row].min + vec2(38.0, 9.0);
    let slot_knob = |app: &App, row: usize| {
        let r = app.gc.slot_rects[0][row];
        pos2(r.min.x + 46.0 + ((r.width() - 100.0) / 2.0).floor() + 9.0, r.min.y + 9.0)
    };
    let cap_y = 28.0 + ((143.0f32 / 2.0).floor() - 14.0) / 2.0;
    let _ = cap_y;
    let wy = 28.0;
    // (kind, name, from, to). Kinds: 'c' click (press on frame 3, release on 12), 'h' the same
    // with the state printed while it is held, 'd' drag, 'x' a call, 'w' wait, 'f' file drop.
    let steps: Vec<(char, &str, Pos2, Pos2)> = vec![
        ('c', "top bar: GROOVE CIRCUIT button (show the panel)", pos2(195.0 + 37.0 * 1.0 + 10.0, 14.0), Pos2::ZERO),
        ('w', "wait (the factory kit is unpacked and read)", Pos2::ZERO, Pos2::ZERO),
        ('w', "wait", Pos2::ZERO, Pos2::ZERO),
        ('x', "CALL load: the deck's own track from the collection into slot A4 (what a drag from the browser does)", Pos2::ZERO, Pos2::ZERO),
        ('f', "DROP: a file from Finder on slot A3", slot_c(app, 2), Pos2::ZERO),
        ('w', "wait (the files are read)", Pos2::ZERO, Pos2::ZERO),
        ('c', "slot A1: Play", play(app, 0), Pos2::ZERO),
        ('w', "wait (A1 plays on the deck's beat; SINGLE mutes the deck's DRUMS)", Pos2::ZERO, Pos2::ZERO),
        ('c', "slot A2: Play (SINGLE: A1 stops when A2 starts)", play(app, 1), Pos2::ZERO),
        ('w', "wait", Pos2::ZERO, Pos2::ZERO),
        ('c', "MULTI (all clips stop, the DRUMS come back)", pp(102.0, 39.0), Pos2::ZERO),
        ('c', "slot A1: Play", play(app, 0), Pos2::ZERO),
        ('c', "slot A2: Play (MULTI: both play, the DRUMS stay)", play(app, 1), Pos2::ZERO),
        ('w', "wait", Pos2::ZERO, Pos2::ZERO),
        ('c', "slot A2: Play again (stops it)", play(app, 1), Pos2::ZERO),
        ('c', "SINGLE (all clips stop)", pp(40.0, 39.0), Pos2::ZERO),
        ('c', "slot A1: Play", play(app, 0), Pos2::ZERO),
        ('w', "wait", Pos2::ZERO, Pos2::ZERO),
        ('h', "DRUM RELEASE FX held (VINYL BRAKE)", pp(3.0 * v9 + 224.0 + 36.0, 39.0), Pos2::ZERO),
        ('c', "release FX beat up", pp(3.0 * v9 + 361.0, 39.0), Pos2::ZERO),
        ('c', "release FX beat down", pp(3.0 * v9 + 329.0, 39.0), Pos2::ZERO),
        ('c', "release FX beat down", pp(3.0 * v9 + 329.0, 39.0), Pos2::ZERO),
        ('c', "release FX pull-down opened", pp(3.0 * v9 + 224.0 + 82.0, 39.0), Pos2::ZERO),
        ('c', "release FX pull-down: second item (ECHO)", pp(3.0 * v9 + 224.0 + 74.0 + 40.0, 39.0 + 9.0 + 14.0 + 24.0), Pos2::ZERO),
        ('h', "DRUM RELEASE FX held (the chosen one)", pp(3.0 * v9 + 224.0 + 36.0, 39.0), Pos2::ZERO),
        ('d', "DRUM SWAP GAIN knob up", pp(v9 + 174.0, 39.0), pp(v9 + 174.0, 39.0 - 30.0)),
        ('d', "slot A1 GAIN knob down", slot_knob(app, 0), slot_knob(app, 0) + vec2(0.0, 30.0)),
        ('c', "BANK B", pp(39.0, 13.0), Pos2::ZERO),
        ('c', "waveform column: page arrow (to DRUM CAPTURE)", pos2(11.0, wy + 143.0 - 7.0), Pos2::ZERO),
        ('c', "DRUM CAPTURE length (4 -> 8)", pos2(11.0, wy + 71.0), Pos2::ZERO),
        ('c', "DRUM CAPTURE length (8 -> 16)", pos2(11.0, wy + 71.0), Pos2::ZERO),
        ('c', "DRUM CAPTURE length (16 -> 32)", pos2(11.0, wy + 71.0), Pos2::ZERO),
        ('c', "DRUM CAPTURE length (32 -> 4)", pos2(11.0, wy + 71.0), Pos2::ZERO),
        ('c', "DRUM CAPTURE on for deck 1", pos2(11.0, wy + 35.0), Pos2::ZERO),
        ('c', "slot B1 pressed: 4 beats of the DRUMS part are captured into it", slot_c(app, 0), Pos2::ZERO),
        ('c', "slot B1: Play (the captured loop)", play(app, 0), Pos2::ZERO),
        ('w', "wait", Pos2::ZERO, Pos2::ZERO),
        ('c', "slot B1: Eject while it plays (Eject Lock)", eject(app, 0), Pos2::ZERO),
        ('c', "slot B1: Play again (stops it)", play(app, 0), Pos2::ZERO),
        ('c', "slot B1: Eject", eject(app, 0), Pos2::ZERO),
        ('x', "CALL capture on: DRUM CAPTURE on for deck 1 (for the drag)", Pos2::ZERO, Pos2::ZERO),
        ('d', "drag the DRUM CAPTURE area from deck 1's waveform onto slot B3 (it captures there)", cap_c(app), slot_c(app, 2)),
        ('w', "wait (the captured file is read)", Pos2::ZERO, Pos2::ZERO),
        ('x', "CALL capture: Original track, 8 beats, into slot B2", Pos2::ZERO, Pos2::ZERO),
        ('c', "BANK A", pp(19.0, 13.0), Pos2::ZERO),
        ('x', "CALL preset: Save as New Preset \"Set one\", then eject A2 and load the preset back", Pos2::ZERO, Pos2::ZERO),
        ('c', "Save Drum Preset menu opened", pp(2.0 * v9 + 215.0, 39.0), Pos2::ZERO),
        ('c', "(menu closed by a click on the panel)", pp(v9 + 140.0, 26.0), Pos2::ZERO),
        ('c', "slot A1: Play", play(app, 0), Pos2::ZERO),
        ('w', "wait", Pos2::ZERO, Pos2::ZERO),
        ('x', "CALL deck: pause deck 1 (its clips stop)", Pos2::ZERO, Pos2::ZERO),
        ('c', "slot A1: Play on the stopped deck (rekordbox: plays at once, unsynced)", play(app, 0), Pos2::ZERO),
        ('w', "wait (A1 sounds while the deck stands)", Pos2::ZERO, Pos2::ZERO),
        ('x', "CALL deck: play deck 1", Pos2::ZERO, Pos2::ZERO),
        ('w', "wait (A1 goes on when the deck starts)", Pos2::ZERO, Pos2::ZERO),
        ('c', "slot A1: Play", play(app, 0), Pos2::ZERO),
        ('w', "wait", Pos2::ZERO, Pos2::ZERO),
        ('c', "top bar: GROOVE CIRCUIT button (hide the panel: everything stops)", pos2(195.0 + 37.0 * 1.0 + 10.0, 14.0), Pos2::ZERO),
        ('c', "top bar: GROOVE CIRCUIT button (show it again for the picture)", pos2(195.0 + 37.0 * 1.0 + 10.0, 14.0), Pos2::ZERO),
    ];
    let Some((kind, name, a, b)) = steps.get(i).copied() else { return };
    let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    match kind {
        'w' => {
            // (About 0.6 s, so that what plays in real time gets somewhere.)
            std::thread::sleep(std::time::Duration::from_millis(16));
            if f == 0 {
                raw.events.push(egui::Event::PointerMoved(pos2(900.0, 500.0)));
            }
        }
        'x' => {
            if f == 3 {
                if name.starts_with("CALL load") {
                    if let Some(id) = app.deck_of(0).track {
                        app.groove_load_track(0, 3, id);
                    }
                } else if name.starts_with("CALL capture on") {
                    app.gc.capture_on = Some(0);
                } else if name.starts_with("CALL capture") {
                    app.prefs.groove.capture_stem = 2;
                    app.gc.capture_len = 8;
                    app.gc.capture_on = Some(0);
                    let ok = app.groove_capture(0, 5);
                    app.prefs.groove.capture_stem = 1;
                    app.gc.capture_len = 4;
                    eprintln!("gctest:   capture of the original track returned {ok}");
                } else if name.starts_with("CALL preset") {
                    let id = app.groove_preset_save_new(0, "Set one");
                    app.groove_eject(0, 1, true);
                    eprintln!("gctest:   saved preset {:?}; A2 ejected: {}", id.is_some(), app.groove_slot(0, 1).is_none());
                    if let Some(id) = id {
                        app.groove_preset_load(0, id);
                    }
                } else if name.starts_with("CALL deck") {
                    app.with_deck(0, |a| a.play_pause());
                }
            }
        }
        'f' => match f {
            3 => {
                // A factory sample as the dropped file (it is not in the collection).
                let f = groove::factory_dir().join("4-Floor Breaks Kit").join("Techno1.wav");
                raw.dropped_files.push(egui::DroppedFile { path: Some(f), ..Default::default() });
            }
            5 => raw.events.push(egui::Event::PointerMoved(a)),
            _ => {}
        },
        _ => match f {
            0 => raw.events.push(egui::Event::PointerMoved(a)),
            3 => raw.events.push(button(a, true)),
            4..=11 if kind == 'd' => {
                let k = (f - 3) as f32 / 8.0;
                raw.events.push(egui::Event::PointerMoved(a + (b - a) * k));
            }
            12 => raw.events.push(button(if kind == 'd' { b } else { a }, false)),
            _ => {}
        },
    }
    if kind == 'h' && f == 11 {
        eprintln!("gctest: {name} (while held): {}", state_line(app));
    }
    if f == 30 {
        eprintln!("gctest: {name}: {}", state_line(app));
    }
    if i + 1 == steps.len() && f == 31 {
        eprintln!("gctest: done ({} steps)", steps.len());
    }
}

/// For the scripted run: give deck `d` a DRUMS part without running the separation model
/// (the first 90 s of the track at half level as DRUMS; the other parts silent), so that the
/// run does not depend on the model being downloaded.
fn fake_stems(app: &mut App, d: usize) {
    let Some(audio) = app.player.decks[d].lock().audio.clone() else { return };
    let sr = crate::stems::htdemucs::SAMPLE_RATE as f64;
    let len = ((audio.duration().min(90.0)) * sr) as usize;
    if len < 4 {
        return;
    }
    let mut data = vec![0.0f32; len * 8];
    let frames = audio.frames();
    for i in 0..len {
        let j = ((i as f64 * audio.sr as f64 / sr) as usize).min(frames - 1);
        data[i] = audio.samples[j * 2] * 0.5;
        data[len + i] = audio.samples[j * 2 + 1] * 0.5;
    }
    app.player.decks[d].lock().stems = Some(std::sync::Arc::new(crate::stems::Stems { len, data }));
    eprintln!("gctest: deck {} given a stand-in DRUMS part ({} s) for the run", d + 1, len as f64 / sr);
}

/// Deck 1's Groove Circuit in one line.
fn state_line(app: &App) -> String {
    let d = 0usize;
    let set = &app.gc.set.decks[d];
    let names: Vec<String> = (0..SLOTS)
        .filter_map(|i| app.groove_slot(d, i).map(|s| format!("{}{}={}({:.0}bpm,{}b{}{})", if i < 4 { 'A' } else { 'B' }, i % 4 + 1, s.name, s.bpm, groove::loop_beats(s.length.max(0) as usize, s.samplingrate, s.bpm), if s.gain != 50.0 { format!(",gain{}", s.gain) } else { String::new() }, if app.groove_ready(d, i) { "" } else { ",reading" })))
        .collect();
    let (rows, cut, fx): (Vec<String>, bool, bool) = match groove::ENGINES[d].lock().as_deref() {
        Some(e) => (
            (0..ROWS)
                .filter_map(|r| {
                    let st = e.row_state(r);
                    match (st.playing, st.pending) {
                        (Some(b), _) => Some(format!("{}{} playing {:.0}%", if b == 0 { 'A' } else { 'B' }, r + 1, st.progress * 100.0)),
                        (None, Some(b)) => Some(format!("{}{} waiting for the beat", if b == 0 { 'A' } else { 'B' }, r + 1)),
                        _ => None,
                    }
                })
                .collect(),
            e.cut,
            e.fx_active(),
        ),
        None => (vec![], false, false),
    };
    let (playing, drums_on, stems) = {
        let s = app.player.decks[d].lock();
        (s.playing, s.stem_on[0], s.stems.is_some())
    };
    format!(
        "panel {} | deck playing {playing} stems {stems} DRUMS {} (auto-mute {}) engine cuts drums {cut} | {} bank {} | slots [{}] | rows [{}] | GAIN {} | RFX {} {} held {} active {fx} | capture {:?} len {} page {} | presets {:?} | toasts {:?}",
        if app.groove_visible() { "shown" } else { "hidden" },
        if drums_on { "on" } else { "MUTED" },
        app.gc.auto[d].on,
        if set.multi { "MULTI" } else { "SINGLE" },
        if set.bank == 0 { 'A' } else { 'B' },
        names.join(", "),
        rows.join(", "),
        app.gc.gain[d],
        groove::RFX_NAMES[set.fx_type.min(2) as usize],
        groove::beat_text(set.fx_beats[set.fx_type.min(2) as usize]),
        app.groove_release_held(d),
        app.gc.capture_on,
        app.gc.capture_len,
        app.gc.page,
        app.gc.set.presets.iter().map(|p| (p.name.clone(), p.tracks.iter().filter(|t| t.is_some()).count())).collect::<Vec<_>>(),
        app.toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>(),
    )
}
