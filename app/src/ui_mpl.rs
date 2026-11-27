//! MIX POINT LINK on screen: the LINK button of the top bar, the strip that takes the place
//! of the stacked waveforms, and Preferences > Controller > MIX POINT LINK.
//!
//! Geometry: rekordbox 7.2.19's `MixPointLinkComp::resized` 0x1014f0694 and `paint`
//! 0x1014f1240, `TimeScaleView::paint` 0x101501c88, and reference
//! `rekordbox-reference/performance/07` measured at 2 px per pt. Positions below are in pt
//! from the strip's top-left (the bottom of the 28 pt top bar); the strip is 244 pt tall with
//! two decks (the waveforms it replaces are 143 pt):
//!
//! - 0..2 black, 2..26 the toolbar (#1E1E1E): MIX IN (20..80) and MIX OUT (82..144) at
//!   y 6..22, MIX POINT SELECT ‹ (164..189) and › (199..224), SILENT (264..282, y 5..23), EDIT
//!   (from 299), Waveform Scrolling (367..383, y 10..18), MIX POINT LINK (426..530, y 5..23),
//!   1 pt #323232 dividers at 244, 344 and 406.
//! - From x 100: the overview (a rail per loaded deck 4 pt down, 3 pt apart, with its playhead;
//!   the white 16 pt rectangle is the part of the track the lanes show) and the time labels
//!   (six divisions of the lanes' width).
//! - Deck boxes x 1..96, 55 pt tall, 69 pt apart from y 81 (#323232, at 40 % when the mode
//!   cannot take the deck; the chosen deck framed in #1473EB with a 16 pt column of that
//!   colour behind its number), with the MIX IN / MIX OUT markers at x 25 and 60.
//! - The lanes: one per deck from x 100, centred on its box.
//! - "MIX IN :" at x 26 and "MIX OUT :" at x 205.5, 18 pt above the bottom; guidance from 400.

use crate::app::*;
use crate::model::*;
use crate::mpl::*;
use crate::theme::*;
use crate::ui_kit as kit;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui};

/// Where the strip's controls were last drawn (the scripted run aims at these), and the
/// lanes' scroll positions (seconds at the left edge, per deck).
#[derive(Default, Clone)]
pub struct StripMap {
    pub rects: Vec<(String, Rect)>,
    pub view: [f64; 4],
    /// (deck, x of time 0, pt per second) of each lane as drawn.
    pub lanes: [Option<(Rect, f64, f64)>; 4],
}

impl StripMap {
    pub fn rect(&self, name: &str) -> Option<Rect> {
        self.rects.iter().find(|x| x.0 == name).map(|x| x.1)
    }
}

const STRIP_H: f32 = 244.0;
const LANE_X: f32 = 100.0;
/// rekordbox's blue of lit skin buttons (0, 125, 225).
const LIT: Color32 = hex(0x007DE1);

const TIP_PANEL: &str = "Show/Hide MIX POINT LINK panel";
const TIP_IN: &str = "MIX IN selection mode:\nAfter the MIX IN deck is selected, the MIX POINT for the start of the mixing can be selected from the Hot Cues/Memory Cues.";
const TIP_OUT: &str = "MIX OUT selection mode:\nAfter the MIX OUT deck is selected, the MIX POINT for the start of the mixing can be selected from the Hot Cues/Memory Cues.";
const TIP_SILENT: &str = "SILENT on/off:\nWhen turned on, the tracks in the MIX IN deck are muted from the start of a track to the Silent end point.";
const TIP_EDIT: &str = "SILENT EDIT on/off:\nWhen turned on, the Silent end point can be changed. The location of the Silent end point can be changed to any point between the start of a track and the MIX POINT.";
const TIP_MOVE: &str = "Waveform Scrolling on/off:\nWhen turned on, the mouse can be used to click and drag the waveforms. ";
const TIP_LINK: &str = "MIX POINNT LINK:\nAfter the MIX POINT is set, click this to set the specific point for the track to start playing automatically. \nIf the MIX POINT is not in the correct place after the track has begun playing automatically, click this again to edit.";
const TIP_MARK: &str = "Displays that the MIX POINT has been set.\nThe MIX POINT setting can be canceled by clicking this button after the MIX POINT has been set.";
const TIP_SELECT: &str = "MIX POINT SELECT:\nSelect Hot Cues/Memory Cues in order as MIX POINT.\nFor tracks without set Hot Cues/Memory Cues, select the current Cue.";
const HELP_SYNC: &str = "When using the BEAT SYNC function, the MIX POINT LINK function can be executed by automatically changing the SYNC setting from BEAT SYNC to BPM SYNC for tracks without a constant BPM.\nWhen this option is not selected, the MIX POINT LINK synchronization may be canceled when a track does not have a constant BPM.";

fn shown(app: &App) -> bool {
    app.mixx.mpl.shown && app.performance() && app.prefs.mixx.mpl.enable
}

/// Height of the strip, or 0 when it is hidden. Four decks: lanes 48 pt apart instead of 69.
pub fn strip_height(app: &App) -> f32 {
    if !shown(app) {
        return 0.0;
    }
    let four = app.deck_count() == 4;
    STRIP_H + if four { 4.0 * 48.0 - 2.0 * 69.0 } else { 0.0 } - if app.prefs.mixx.mpl.show_time_scale { 0.0 } else { 24.0 }
}

/// How much taller the player area is while the strip is shown.
pub fn extra_height(app: &App) -> f32 {
    let h = strip_height(app);
    if h > 0.0 {
        h - if app.deck_count() == 4 { 198.0 } else { 143.0 }
    } else {
        0.0
    }
}

/// The LINK button of the top bar (text centred 418.5 pt in, a 40 x 2 pt bar under it).
pub fn link_button(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    let c = pos2(r.min.x + 418.5, r.min.y + 11.0);
    let br = Rect::from_min_max(pos2(r.min.x + 396.0, r.min.y + 3.0), pos2(r.min.x + 441.0, r.max.y - 3.0));
    let resp = ui.interact(br, ui.id().with("mpl_link_btn"), Sense::click());
    let enabled = app.prefs.mixx.mpl.enable;
    p.text(c + vec2(0.0, 0.75), Align2::CENTER_CENTER, "LINK", caps(12.6), if !enabled { OFF.c() } else { WHITE.c() });
    p.rect_filled(Rect::from_min_max(pos2(r.min.x + 398.0, r.min.y + 20.0), pos2(r.min.x + 438.0, r.min.y + 22.0)), 0.0, if app.mixx.mpl.any_linked() { BLUE.c() } else { gy(0x282828) });
    app.mixx.mpl.ui.rects.retain(|x| x.0 != "panel");
    app.mixx.mpl.ui.rects.push(("panel".into(), br));
    let resp = if app.prefs.tooltips { resp.on_hover_text(TIP_PANEL) } else { resp };
    if resp.clicked() {
        app.mpl_toggle_panel();
    }
}

/// After the stacked waveforms are drawn: the ranges Automix fades over, on decks 1 and 2.
pub fn after_waves(app: &mut App, ui: &mut Ui) {
    for d in 0..2 {
        crate::ui_automix::wave_overlay(app, ui, d);
    }
}

fn tip(resp: egui::Response, app: &App, text: &str) -> egui::Response {
    if app.prefs.tooltips { resp.on_hover_text(text) } else { resp }
}

/// SILENT (`mpl_silent`, 18 x 18): a ring, a small waveform and a stroke through both.
fn silent_icon(p: &Painter, r: Rect, c: Color32) {
    let o = r.center();
    p.circle_stroke(o, 8.3, Stroke::new(1.0, c));
    for (dx, h) in [(-6.0f32, 3.0f32), (-4.0, 6.0), (-2.0, 3.0), (0.0, 8.0), (2.0, 4.0), (4.0, 9.0), (6.0, 3.0)] {
        p.rect_filled(Rect::from_center_size(o + vec2(dx, 0.5), vec2(1.0, h)), 0.0, c);
    }
    p.rect_filled(Rect::from_center_size(o + vec2(0.0, 0.5), vec2(15.0, 1.0)), 0.0, c);
    p.line_segment([o + vec2(-6.0, -6.0), o + vec2(6.0, 6.0)], Stroke::new(1.0, c));
}

/// Waveform Scrolling (`mpl_move_waveform`, 16 x 8): an arrow with two heads.
fn move_icon(p: &Painter, r: Rect, c: Color32) {
    let o = r.center() - vec2(8.0, 4.0);
    let at = |x: f32, y: f32| o + vec2(x, y);
    p.rect_filled(Rect::from_min_max(at(4.0, 3.0), at(12.0, 5.0)), 0.0, c);
    p.add(Shape::convex_polygon(vec![at(0.0, 4.0), at(4.0, 0.0), at(4.0, 8.0)], c, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(16.0, 4.0), at(12.0, 8.0), at(12.0, 0.0)], c, Stroke::NONE));
}

/// A marker button (`mpl_mix_in_*` / `mpl_mix_out_*`, 23 x 23): round at the outer side,
/// with the bracket; lit, it is blue and carries the deck's letter.
fn marker_icon(p: &Painter, r: Rect, out: bool, on: bool, deck: usize) {
    let fill = if on { LIT } else { gy(0x1B1B1B) };
    let fg = if on { Color32::BLACK } else { WHITE.c() };
    let o = r.min;
    let (w, h) = (r.width(), r.height());
    let (round, square) = if out { (r, Rect::from_min_max(o, o + vec2(w / 2.0, h))) } else { (r, Rect::from_min_max(o + vec2(w / 2.0, 0.0), r.max)) };
    p.rect_filled(round, w / 2.0, fill);
    p.rect_filled(square, 0.0, fill);
    // The bracket: 3.5 pt wide, 9.5 pt tall, 1 pt strokes.
    let bx = if out { 6.5 } else { 14.5 };
    let (x0, x1) = (o.x + bx, o.x + bx + 3.5);
    let (y0, y1) = (o.y + 7.5, o.y + 17.0);
    let stem = if out { x1 - 1.0 } else { x0 };
    p.rect_filled(Rect::from_min_max(pos2(stem, y0), pos2(stem + 1.0, y1)), 0.0, fg);
    p.rect_filled(Rect::from_min_max(pos2(x0, y0), pos2(x1, y0 + 1.0)), 0.0, fg);
    p.rect_filled(Rect::from_min_max(pos2(x0, y1 - 1.0), pos2(x1, y1)), 0.0, fg);
    if on {
        let lx = if out { 14.5 } else { 8.5 };
        p.text(o + vec2(lx, 11.5), Align2::CENTER_CENTER, ((b'A' + deck as u8) as char).to_string(), caps(8.0), fg);
    }
}

/// Text drawn from `pos` (left, vertical centre), made narrower when it is wider than `max_w`
/// (as a JUCE label does), keeping its height.
fn squeezed_text(ui: &Ui, p: &Painter, pos: Pos2, text: &str, font: FontId, col: Color32, max_w: f32) {
    let g = p.layout_no_wrap(text.to_string(), font, col);
    let w = g.size().x;
    let at = pos2(pos.x, pos.y - g.size().y / 2.0);
    if w <= max_w {
        p.galley(at, g, col);
        return;
    }
    let k = max_w / w;
    let shape = egui::epaint::ClippedShape { clip_rect: Rect::EVERYTHING, shape: Shape::galley(at, g, col) };
    for prim in ui.ctx().tessellate(vec![shape], ui.ctx().pixels_per_point()) {
        if let egui::epaint::Primitive::Mesh(mut m) = prim.primitive {
            for v in m.vertices.iter_mut() {
                v.pos.x = at.x + (v.pos.x - at.x) * k;
            }
            p.add(Shape::mesh(m));
        }
    }
}

fn close_icon(p: &Painter, r: Rect, c: Color32) {
    let o = r.center();
    p.line_segment([o + vec2(-3.5, -3.5), o + vec2(3.5, 3.5)], Stroke::new(1.3, c));
    p.line_segment([o + vec2(-3.5, 3.5), o + vec2(3.5, -3.5)], Stroke::new(1.3, c));
}

fn mmss(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{:3}:{:02}", s / 60, s % 60)
}

/// The strip, in `r` (which starts under the top bar and is `strip_height` tall).
pub fn strip(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    p.rect_filled(r, 0.0, BG);
    let o = r.min;
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(o + vec2(x, y), vec2(w, h));
    let panel = app.mixx.mpl.ui.rect("panel");
    let mut map: Vec<(String, Rect)> = panel.map(|x| ("panel".to_string(), x)).into_iter().collect();
    let count = app.deck_count().min(4) as usize;
    let decks = deck_infos(app);
    let selectable = app.mixx.mpl.selectable(&decks);
    let prefs = app.prefs.mixx.mpl.clone();

    // ---- toolbar
    p.rect_filled(at(0.0, 2.0, r.width(), 24.0), 0.0, BG2);
    for (i, (label, mode, x, w, tipt)) in [("MIX IN", MODE_IN, 20.0f32, 60.0f32, TIP_IN), ("MIX OUT", MODE_OUT, 82.0, 62.0, TIP_OUT)].into_iter().enumerate() {
        let br = at(x, 6.0, w, 16.0);
        let on = app.mixx.mpl.mode == mode;
        let resp = ui.interact(br, ui.id().with(("mpl_mode", i)), Sense::click());
        p.rect_filled(br, 1.0, if on { BLUE.c() } else if resp.hovered() { gy(0x343434) } else { BTN.c() });
        p.text(br.center() + vec2(1.0, 0.5), Align2::CENTER_CENTER, label, rb(10.5), if on { Color32::BLACK } else { WHITE.c() });
        map.push((if mode == MODE_IN { "mix_in" } else { "mix_out" }.into(), br));
        if tip(resp, app, tipt).clicked() {
            app.mpl_set_mode(mode);
        }
    }
    for (i, (x, dir)) in [(164.0f32, -1i32), (199.0, 1)].into_iter().enumerate() {
        let br = at(x, 6.0, 25.0, 16.0);
        let resp = ui.interact(br, ui.id().with(("mpl_sel", i)), Sense::click());
        p.rect_filled(br, 2.0, if resp.hovered() { gy(0x343434) } else { BTN.c() });
        let c = br.center();
        let s = dir as f32;
        p.add(Shape::line(vec![c + vec2(-1.6 * s - 0.5, -3.1), c + vec2(1.6 * s - 0.5, 0.0), c + vec2(-1.6 * s - 0.5, 3.1)], Stroke::new(1.4, gy(0x969696))));
        map.push((if dir < 0 { "prev" } else { "next" }.into(), br));
        if tip(resp, app, TIP_SELECT).clicked() {
            app.mpl_select_cue(dir);
        }
    }
    for x in [244.0, 344.0, 406.0] {
        p.rect_filled(at(x, 8.0, 1.0, 12.0), 0.0, gy(0x323232));
    }
    // SILENT and EDIT act on the selected deck (or the one holding the MIX IN point).
    let sd = app.mixx.mpl.selected.or(app.mixx.mpl.in_deck).filter(|d| app.mixx.mpl.mix_in[*d] != -1);
    let (s_on, e_on) = sd.map(|d| (app.mixx.mpl.silent_on[d], app.mixx.mpl.silent_edit[d])).unwrap_or((false, false));
    let sr = at(264.0, 5.0, 18.0, 18.0);
    let resp = ui.interact(sr, ui.id().with("mpl_silent"), Sense::click());
    silent_icon(&p, sr, if s_on { LIT } else if sd.is_some() { gy(0x969696) } else { gy(0x404040) });
    map.push(("silent".into(), sr));
    if tip(resp, app, TIP_SILENT).clicked() {
        app.mpl_toggle_silent();
    }
    p.rect_filled(at(287.0, 13.5, 5.0, 1.0), 0.0, gy(0x404040));
    let er = at(297.0, 7.0, 27.0, 12.0);
    let resp = ui.interact(er.expand(2.0), ui.id().with("mpl_edit"), Sense::click());
    p.text(pos2(er.min.x + 1.0, er.center().y + 1.5), Align2::LEFT_CENTER, "EDIT", rb(11.0), if e_on { LIT } else if sd.is_some() { gy(0x969696) } else { gy(0x626262) });
    map.push(("edit".into(), er));
    if tip(resp, app, TIP_EDIT).clicked() {
        app.mpl_toggle_silent_edit();
    }
    let mr = at(364.0, 2.0, 22.0, 22.0);
    let resp = ui.interact(mr, ui.id().with("mpl_move"), Sense::click());
    move_icon(&p, mr, if app.mixx.mpl.move_mode { LIT } else { gy(0x969696) });
    map.push(("move".into(), mr));
    if tip(resp, app, TIP_MOVE).clicked() {
        app.mixx.mpl.move_mode = !app.mixx.mpl.move_mode;
    }
    let lr = at(426.0, 5.0, 104.0, 18.0);
    let (ready, linked) = (app.mixx.mpl.any_ready(), app.mixx.mpl.any_linked());
    let resp = ui.interact(lr, ui.id().with("mpl_link"), Sense::click());
    if linked && !ready {
        p.rect_filled(lr, 2.0, BLUE);
    }
    p.rect_stroke(lr, 2.0, Stroke::new(1.0, if ready { gy(0x969696) } else if linked { BLUE.c() } else { gy(0x434343) }), egui::StrokeKind::Inside);
    p.text(lr.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, "MIX POINT LINK", rb(11.8), if linked && !ready { Color32::BLACK } else if ready { WHITE.c() } else { gy(0x626262) });
    map.push(("link".into(), lr));
    if tip(resp, app, TIP_LINK).clicked() && ready {
        app.mpl_request_link();
    }

    // ---- geometry below the toolbar
    let four = count == 4;
    let up = if prefs.show_time_scale { 0.0 } else { 24.0 };
    let (pitch, box_y0, box_h) = if four { (48.0, 77.0 - up, 44.0) } else { (69.0, 81.0 - up, 55.0) };
    let lane_w = (r.width() - LANE_X).max(100.0);
    let span = zoom_span(app.mixx.mpl.zoom);
    let pps = lane_w as f64 / span;
    let longest = (0..count).map(|d| app.deck_of(d as u8).duration).fold(0.0f64, f64::max);

    // ---- overview and time scale
    let focus = app.mixx.mpl.selected.unwrap_or(0).min(count.saturating_sub(1));
    if longest > 0.0 {
        let ox = o.x + LANE_X;
        let oy = o.y + 31.0;
        let sx = |t: f64| ox + (t / longest) as f32 * lane_w;
        for d in 0..count {
            let dk = app.deck_of(d as u8);
            if dk.track.is_none() || dk.duration <= 0.0 {
                continue;
            }
            let y = oy + 3.0 + 3.0 * d as f32;
            p.rect_filled(Rect::from_min_max(pos2(ox, y + 0.5), pos2(sx(dk.duration), y + 1.5)), 0.0, gy(0x7D7D7D));
            let mark = |t: f64, c: Color32| {
                let x = sx(t).round();
                p.rect_filled(Rect::from_min_max(pos2(x - 1.0, y), pos2(x + 3.0, y + 3.0)), 0.0, BG);
                p.rect_filled(Rect::from_min_max(pos2(x, y), pos2(x + 2.0, y + 3.0)), 0.0, c);
            };
            let m = &app.mixx.mpl;
            for t in [m.mix_in[d], m.mix_out[d]] {
                if t != -1 {
                    mark(t as f64 / 1000.0, LIT);
                }
            }
            let pos = app.player.decks[d].lock().pos;
            let sr = dk.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
            mark(pos / sr, WHITE.c());
        }
        // The part of the track the lanes show.
        let v0 = app.mixx.mpl.ui.view[focus].clamp(0.0, longest);
        let x0 = sx(v0);
        let w = ((span / longest) as f32 * lane_w).min(lane_w - (x0 - ox)).max(4.0);
        p.rect_stroke(Rect::from_min_size(pos2(x0, oy + 1.0), vec2(w, 16.0)), 0.0, Stroke::new(1.0, WHITE), egui::StrokeKind::Inside);
        // Clicking or dragging on the overview moves the lanes there.
        let orr = Rect::from_min_size(pos2(ox, oy), vec2(lane_w, 18.0));
        let resp = ui.interact(orr, ui.id().with("mpl_overview"), Sense::click_and_drag());
        if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
            let t = ((pp.x - ox) / lane_w) as f64 * longest - span / 2.0;
            for v in app.mixx.mpl.ui.view.iter_mut() {
                *v = t.clamp(0.0, (longest - span * 0.5).max(0.0));
            }
        }
        // The wheel over the overview or the time labels zooms all lanes together.
        let zr = Rect::from_min_size(pos2(ox, oy), vec2(lane_w, 42.0));
        if ui.rect_contains_pointer(zr) {
            let dy = ui.input(|i| i.raw_scroll_delta.y);
            if dy.abs() > 0.5 {
                let z = app.mixx.mpl.zoom as i32 + if dy > 0.0 { -1 } else { 1 };
                // "Automatically limits the zoom out range": no further out once the
                // longest track fits.
                let fits = longest < span;
                if (1..=10).contains(&z) && !(z > app.mixx.mpl.zoom as i32 && prefs.zoom_out_limit && fits) {
                    app.mixx.mpl.zoom = z as u8;
                }
            }
        }
    }
    if prefs.show_time_scale {
        let v0 = app.mixx.mpl.ui.view[focus];
        for k in 0..6 {
            let x = o.x + LANE_X + k as f32 * ((lane_w - 3.5) / 6.0);
            let t = (v0 + (x - o.x - LANE_X) as f64 / pps).max(0.0);
            p.rect_filled(Rect::from_min_max(pos2(x, o.y + 62.0), pos2(x + 0.5, o.y + 71.0)), 0.0, gy(0x4A4A4A));
            // (The first label starts on its tick, the others are centred on theirs.)
            p.text(pos2(x + if k == 0 { 0.0 } else { 1.5 }, o.y + 58.5), if k == 0 { Align2::LEFT_CENTER } else { Align2::CENTER_CENTER }, format!("{:02}:{:02}", t.round() as i64 / 60, t.round() as i64 % 60), rb(11.6), gy(0x969696));
        }
    }

    // ---- deck boxes, markers and lanes
    let mut clicks: Vec<(usize, f64, bool)> = vec![];
    let mut cancels: Vec<(usize, u8)> = vec![];
    let mut lanes = [None; 4];
    for d in 0..count {
        let by = box_y0 + pitch * d as f32;
        let br = at(1.0, by, 95.0, box_h);
        let sel = app.mixx.mpl.selected == Some(d);
        p.rect_filled(br, 0.0, if selectable[d] || sel { gy(0x323232) } else { gy(0x323232).gamma_multiply(0.4) });
        if sel {
            p.rect_filled(Rect::from_min_size(br.min, vec2(16.0, box_h)), 0.0, BLUE);
            p.rect_stroke(br, 0.0, Stroke::new(1.0, BLUE), egui::StrokeKind::Inside);
        }
        p.text(pos2(o.x + 9.25, br.center().y + 2.0), Align2::CENTER_CENTER, format!("{}", d + 1), bold(10.5), if sel { Color32::BLACK } else { gy(0x969696) });
        map.push((format!("box{d}"), br));
        let m = &app.mixx.mpl;
        for (which, x, set) in [(MODE_IN, 25.5f32, m.mix_in[d] != -1), (MODE_OUT, 60.0, m.mix_out[d] != -1)] {
            let ir = Rect::from_min_size(pos2(o.x + x, br.center().y - 11.5), vec2(24.5, 24.5));
            let resp = ui.interact(ir, ui.id().with(("mpl_mark", d, which)), Sense::click());
            marker_icon(&p, ir, which == MODE_OUT, set, d);
            map.push((format!("{}{d}", if which == MODE_IN { "mark_in" } else { "mark_out" }), ir));
            if tip(resp, app, TIP_MARK).clicked() && set {
                cancels.push((d, which));
            }
        }
        // The lane.
        let cy = br.center().y;
        let lr = Rect::from_min_max(pos2(o.x + LANE_X, cy - pitch / 2.0), pos2(r.max.x, cy + pitch / 2.0));
        lanes[d] = Some((lr, app.mixx.mpl.ui.view[d], pps));
        map.push((format!("lane{d}"), lr));
        let dk = app.deck_of(d as u8);
        if dk.track.is_none() {
            continue;
        }
        let (pos, playing) = {
            let s = app.player.decks[d].lock();
            (s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0), s.playing)
        };
        // The lane follows its deck: when the playhead leaves the view it turns the page.
        if !app.mixx.mpl.move_mode && playing {
            let v = app.mixx.mpl.ui.view[d];
            if pos < v || pos > v + span * 0.9 {
                app.mixx.mpl.ui.view[d] = (pos - span * 0.1).max(0.0);
            }
        }
        let v0 = app.mixx.mpl.ui.view[d];
        let lp = p.with_clip_rect(Rect::from_min_max(pos2(lr.min.x - 3.0, lr.min.y), lr.max).intersect(ui.clip_rect()));
        let x_of = |t: f64| lr.min.x + ((t - v0) * pps) as f32;
        let t_of = |x: f32| v0 + (x - lr.min.x) as f64 / pps;
        lane_wave(app, &lp, lr, d, v0, pps, four);
        let dk = app.deck_of(d as u8);
        let (top, bot) = (lr.min.y + 1.5, lr.max.y - 1.5);
        // Beats: a small grey mark at both edges; bars red, joined by a white line.
        let beat_pt = dk.beats.get(1).map(|b| (b.time - dk.beats[0].time) * pps).unwrap_or(0.0);
        if prefs.show_grid && (!prefs.grid_when_enlarged || beat_pt >= 3.0) {
            let (tl, tr) = (t_of(lr.min.x), t_of(lr.max.x));
            let i0 = dk.beats.partition_point(|b| b.time < tl);
            for b in dk.beats[i0..].iter().take_while(|b| b.time <= tr) {
                let x = x_of(b.time).round();
                if b.beat == 1 {
                    lp.rect_filled(Rect::from_min_max(pos2(x, top + 5.0), pos2(x + 1.0, bot - 5.0)), 0.0, WHITE);
                    lp.add(Shape::convex_polygon(vec![pos2(x - 2.5, top), pos2(x + 3.5, top), pos2(x + 0.5, top + 4.0)], hex(0xE01010), Stroke::NONE));
                    lp.add(Shape::convex_polygon(vec![pos2(x - 2.5, bot), pos2(x + 3.5, bot), pos2(x + 0.5, bot - 4.0)], hex(0xE01010), Stroke::NONE));
                } else {
                    for (y0, y1, cap) in [(top, top + 3.5, top), (bot - 3.5, bot, bot - 1.0)] {
                        lp.rect_filled(Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1)), 0.0, gy(0x808080));
                        lp.rect_filled(Rect::from_min_max(pos2(x - 1.0, cap), pos2(x + 2.0, cap + 1.0)), 0.0, gy(0x808080));
                    }
                }
            }
        }
        // SILENT: the muted stretch, from the start of the track to the end point.
        let m = &app.mixx.mpl;
        if m.silent_on[d] && m.mix_in[d] != -1 {
            let xe = x_of(m.silent_end[d].max(0) as f64 / 1000.0);
            lp.rect_filled(Rect::from_min_max(pos2(x_of(0.0), lr.min.y + 6.0), pos2(xe, lr.max.y - 6.0)), 0.0, Color32::from_black_alpha(150));
            lp.rect_filled(Rect::from_min_max(pos2(xe - 0.5, lr.min.y + 6.0), pos2(xe + 0.5, lr.max.y - 6.0)), 0.0, gy(0x969696));
        }
        // Cues: hot cues in their colour and memory cues in red along the top, the current
        // cue in orange at the bottom.
        let mut cue_x: Vec<(f32, f64)> = vec![];
        for c in &dk.cues {
            let x = x_of(c.start);
            if x < lr.min.x - 6.0 || x > lr.max.x + 6.0 {
                continue;
            }
            let col = if c.kind == CueKind::Hot { hotcue_color(c.color) } else { hex(0xE01010) };
            lp.add(Shape::convex_polygon(vec![pos2(x - 4.0, top), pos2(x + 4.0, top), pos2(x, top + 7.0)], col, Stroke::NONE));
            cue_x.push((x, c.start));
        }
        let xc = x_of(dk.cue_point);
        lp.add(Shape::convex_polygon(vec![pos2(xc - 6.5, bot + 1.0), pos2(xc + 6.5, bot + 1.0), pos2(xc, bot - 9.0)], CUE_MARK, Stroke::NONE));
        cue_x.push((xc, dk.cue_point));
        // The MIX points: a 2 pt line in the markers' blue with the bracket at its head.
        for (t, out) in [(m.mix_in[d], false), (m.mix_out[d], true)] {
            if t == -1 {
                continue;
            }
            let x = x_of(t as f64 / 1000.0).round();
            lp.rect_filled(Rect::from_min_max(pos2(x - 1.0, lr.min.y), pos2(x + 1.0, lr.max.y)), 0.0, LIT);
            let tag = Rect::from_min_size(pos2(if out { x - 13.0 } else { x + 1.0 }, lr.min.y), vec2(12.0, 12.0));
            lp.rect_filled(tag, 0.0, LIT);
            lp.text(tag.center(), Align2::CENTER_CENTER, if out { "]" } else { "[" }, caps(9.0), Color32::BLACK);
        }
        // The playhead.
        let xp = x_of(pos).round();
        lp.rect_filled(Rect::from_min_max(pos2(xp, lr.min.y), pos2(xp + 1.0, lr.max.y)), 0.0, gy(0x717171));
        // A linked MIX IN deck counts its bars from the MIX IN point
        // (`operateWaveViewSetMixPointBeatCountMode`, set by `doMixPointLinkStart`): "-1.2Bars"
        // three beats before it, in the beat count's blue, ending 2 pt left of the playhead.
        if m.link_set[d] && m.mix_in[d] != -1 {
            let label = crate::mpl::mix_point_bars(&dk.beats, m.mix_in[d] as f64 / 1000.0, pos);
            lp.text(pos2(xp as f32 - 2.0, lr.min.y + 9.0), Align2::RIGHT_CENTER, label, crate::ui_perf_layouts::arial_b(if four { 11.0 } else { 13.0 }), crate::theme::BEAT_BLUE.c());
        }
        // Clicks and drags.
        let resp = ui.interact(lr, ui.id().with(("mpl_lane", d)), Sense::click_and_drag());
        if app.mixx.mpl.move_mode {
            if resp.dragged() {
                let dx = resp.drag_delta().x as f64;
                let max = (app.deck_of(d as u8).duration - span * 0.1).max(0.0);
                app.mixx.mpl.ui.view[d] = (app.mixx.mpl.ui.view[d] - dx / pps).clamp(0.0, max);
            }
        } else if resp.clicked() {
            if let Some(pp) = resp.interact_pointer_pos() {
                // On a cue marker the cue's own time is taken.
                let near_edge = pp.y < top + 9.0 || pp.y > bot - 10.0;
                let hit = cue_x.iter().filter(|(x, _)| near_edge && (pp.x - x).abs() <= 6.0).min_by(|a, b| (pp.x - a.0).abs().partial_cmp(&(pp.x - b.0).abs()).unwrap());
                match hit {
                    Some((_, t)) => clicks.push((d, *t, true)),
                    None => clicks.push((d, t_of(pp.x), false)),
                }
            }
        }
    }
    app.mixx.mpl.ui.lanes = lanes;

    // ---- readouts and guidance
    let ry = r.max.y - 18.25;
    let m = &app.mixx.mpl;
    let grey = gy(0x969696);
    let mut dels: Vec<(usize, u8)> = vec![];
    for (label, x, deck, t, which) in [("MIX IN :", 26.0f32, m.in_deck, m.in_deck.map(|d| m.mix_in[d]).unwrap_or(-1), MODE_IN), ("MIX OUT :", 205.5, m.out_deck, m.out_deck.map(|d| m.mix_out[d]).unwrap_or(-1), MODE_OUT)] {
        // (rekordbox's labels are 50 and 55 pt wide and squeeze their text to fit.)
        squeezed_text(ui, &p, pos2(o.x + x, ry + 1.0), label, rb(11.3), grey, if which == MODE_IN { 38.0 } else { 43.5 });
        if let (Some(d), true) = (deck, t != -1) {
            let vx = o.x + x + if which == MODE_IN { 50.0 } else { 55.0 };
            p.text(pos2(vx, ry), Align2::LEFT_CENTER, format!("DECK{}  {}", d + 1, mmss(t)), rb(11.6), WHITE.c());
            let cr = Rect::from_min_size(pos2(vx + 84.0, ry - 10.0), vec2(20.0, 20.0));
            let resp = ui.interact(cr, ui.id().with(("mpl_del", which)), Sense::click());
            close_icon(&p, cr, if resp.hovered() { WHITE.c() } else { grey });
            map.push((if which == MODE_IN { "del_in" } else { "del_out" }.into(), cr));
            let resp = if app.prefs.tooltips { resp.on_hover_text(format!("Clear the MIX POINT on the MIX {} deck.", if which == MODE_IN { "IN" } else { "OUT" })) } else { resp };
            if resp.clicked() {
                dels.push((d, which));
            }
        }
    }
    let g = GUIDANCE[(m.guidance as usize).min(6)];
    if !g.is_empty() && r.width() > 520.0 {
        let gal = p.layout(g.replace('\n', " "), rb(10.5), grey, r.width() - 410.0);
        p.galley(pos2(o.x + 400.0, ry - gal.size().y / 2.0), gal, grey);
    }
    app.mixx.mpl.ui.rects = map;
    for (d, which) in cancels.into_iter().chain(dels) {
        app.mpl_cancel(d, which);
    }
    for (d, t, cue) in clicks {
        app.mpl_wave_click(d, t, cue);
    }
    if app.player.decks.iter().any(|s| s.lock().playing) {
        ui.ctx().request_repaint();
    }
}

/// One lane's waveform: the deck's track from `v0` seconds on, `pps` pt per second.
fn lane_wave(app: &App, p: &Painter, lr: Rect, d: usize, v0: f64, pps: f64, four: bool) {
    let dk = app.deck_of(d as u8);
    let Some(w) = dk.wave.as_ref() else { return };
    let n = w.frames.len();
    let fps = WAVE_FPS;
    let mid = lr.center().y;
    let half = lr.height() / 2.0;
    let _ = four;
    let dur = dk.duration;
    let mut cols = kit::Bars::new();
    let mut x = lr.min.x;
    while x < lr.max.x {
        let t0 = v0 + (x - lr.min.x) as f64 / pps;
        let t1 = t0 + 1.0 / pps;
        if t1 >= 0.0 && t0 <= dur {
            let a = ((t0 * fps).max(0.0) as usize).min(n);
            let b = ((t1 * fps).ceil() as usize).clamp(a + 1, n.max(a + 1)).min(n);
            if a < n {
                let mut m = [0u8; 8];
                for f in &w.frames[a..b.max(a + 1).min(n)] {
                    for k in 0..8 {
                        m[k] = m[k].max(f[k]);
                    }
                }
                let bars: Vec<(f32, Color32)> = if app.prefs.wave_color >= 2 {
                    let [lo, mi, hi] = match &dk.rb_wave {
                        Some(rw) if !rw.0.is_empty() => {
                            let ia = ((t0.max(0.0) * 150.0) as usize).min(rw.0.len() - 1);
                            let ib = ((t1.max(0.0) * 150.0).ceil() as usize).clamp(ia + 1, rw.0.len());
                            rw.0[ia..ib].iter().fold([0u8; 3], |acc, v| [acc[0].max(v[0]), acc[1].max(v[1]), acc[2].max(v[2])])
                        }
                        _ => crate::export::anlz::three_band_detail(&m),
                    };
                    let s = |v: u8| (v as f32 / 127.0).min(1.0) * crate::ui_player::DETAIL_FULL_SCALE;
                    vec![(s(lo), WAVE_LOW), (s(mi), WAVE_MID), (s(hi), WAVE_HIGH)]
                } else {
                    crate::ui_player::wave_bars(app.prefs.wave_color, Waveform::disp(&m)).into_iter().map(|(h, c)| (h * crate::ui_player::DETAIL_FULL_SCALE, c)).collect()
                };
                for (h, col) in bars {
                    let hh = (h * half).max(0.5);
                    cols.column(x + 0.5, mid - hh, mid + hh, col);
                }
            }
        }
        x += 1.0;
    }
    cols.paint(p);
}

/// Preferences > Controller > MIX POINT LINK (reference preferences/12).
pub fn prefs_page(app: &mut App, ui: &mut Ui) {
    use kit::{card, radio, separator, sub, toggle};
    let before = app.prefs.mixx.mpl.clone();
    let (band, _) = ui.allocate_exact_size(vec2(ui.available_width().min(kit::CARD_W), 43.0), Sense::hover());
    let mut head = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(band.min.x + 20.0, band.min.y + 19.5), pos2(band.max.x, band.max.y + 8.0))).layout(egui::Layout::left_to_right(egui::Align::Min)));
    kit::toggle_raw(&mut head, &mut app.prefs.mixx.mpl.enable, "Enable MIX POINT LINK Function");
    let m = &mut app.prefs.mixx.mpl;
    card(ui, "MIX IN/MIX OUT", |ui| {
        sub(ui, "Default MIX POINT Setting");
        if radio(ui, m.default_type == MODE_IN, "MIX IN") {
            m.default_type = MODE_IN;
        }
        if radio(ui, m.default_type != MODE_IN, "MIX OUT") {
            m.default_type = MODE_OUT;
        }
        separator(ui);
        kit::indent(ui, 84.0);
        toggle(ui, &mut m.auto_change, "Automatically switches between MIX IN/MIX OUT");
    });
    card(ui, "Time scale", |ui| {
        toggle(ui, &mut m.show_time_scale, "Display time");
    });
    card(ui, "Grids display", |ui| {
        toggle(ui, &mut m.show_grid, "Display grids");
        ui.add_space(1.0);
        separator(ui);
        toggle(ui, &mut m.grid_when_enlarged, "Display only when waveform is enlarged");
    });
    kit::card_help(ui, "Automatic BEAT/BPM SYNC Setting", Some(HELP_SYNC), |ui| {
        toggle(ui, &mut m.bpm_sync_auto, "Automatically changes the BEAT/BPM SYNC setting");
    });
    card(ui, "Zoom range", |ui| {
        toggle(ui, &mut m.zoom_out_limit, "Automatically limits the zoom out range");
    });
    if app.prefs.mixx.mpl != before {
        app.save_prefs();
    }
}

#[allow(dead_code)]
fn unused(_: Pos2, _: FontId) {}
