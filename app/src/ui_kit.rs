//! The control kit of the Preferences window and the dialogs: switches, radio buttons,
//! checkboxes, dropdowns, sliders, "?" buttons, buttons and the card layout, with the sizes
//! and colours measured from the original's Preferences window (points = Retina pixels / 2).
//! Everything is drawn with painter geometry; no artwork is loaded.
//!
//! The Preferences controls follow rekordbox's own drawing code (7.2.19: `drawRadioButton`
//! 0x100496d88, `PreferenceBaseLookAndFeel::drawToggleButton` 0x100496968, `drawComboBox`
//! 0x1004973cc, `drawLinearSlider` 0x10049701c, `PreferenceToggleAndColorBoxLookAndFeel::
//! drawToggleButton` 0x10049b314) and its face and sizes (`prefs_font.rs`).

use crate::theme::*;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
pub use crate::prefs_font::{pref, pref_bold, Just};

/// Height of a switch, radio button or check box row in rekordbox's layouts, and the size of
/// the control and of its caption's font in it (0.85 of the row).
pub const ROW_H: f32 = 19.0;
pub const CTL: f32 = 0.85 * ROW_H;

/// The blue of selected rows, tabs, switches and radio buttons.
pub const PBLUE: Color32 = hex(0x1473EB);
/// Separator lines inside cards, scroll bar.
pub const SEP: Tc = gr(0x323232);
/// Switch (off) and radio (off) grey, checkbox outline.
pub const GREY: Tc = gr(0x969696);
/// Dropdown arrow.
pub const ARROW: Tc = gr(0x878787);
/// Width of a card when the page does not scroll (with a scroll bar it is 4 pt narrower).
pub const CARD_W: f32 = 628.0;

pub fn inter(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("inter".into()))
}

pub fn inter_bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("inter-bold".into()))
}

/// Text in the kit's regular face.
pub fn text(s: impl Into<String>, size: f32) -> RichText {
    RichText::new(s).font(inter(size))
}

/// Fonts, dropdown, text field and scroll bar looks for everything inside `ui`.
pub fn apply_style(ui: &mut Ui) {
    let s = ui.style_mut();
    use egui::TextStyle::*;
    for (ts, size) in [(Small, 11.0), (Body, 13.0), (Button, 13.0), (Heading, 17.0)] {
        s.text_styles.insert(ts, inter(size));
    }
    s.visuals.selection.bg_fill = PBLUE;
    s.visuals.selection.stroke = Stroke::new(1.0, ON_ACCENT);
    s.visuals.extreme_bg_color = gy(0x000000);
    let w = &mut s.visuals.widgets;
    for v in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        v.corner_radius = 3.0.into();
        v.bg_stroke = Stroke::NONE;
        v.expansion = 0.0;
        v.weak_bg_fill = gy(0x000000);
    }
    w.inactive.bg_fill = SEP.c();
    w.hovered.bg_fill = gy(0x3C3C3C);
    w.active.bg_fill = gy(0x464646);
    w.inactive.fg_stroke.color = WHITE.c();
    w.hovered.fg_stroke.color = WHITE.c();
    s.spacing.interact_size.y = 22.0;
    s.spacing.button_padding = vec2(6.5, 2.0);
    s.spacing.scroll.floating = false;
    s.spacing.scroll.bar_width = 6.0;
    s.spacing.scroll.bar_inner_margin = 1.0;
    s.spacing.scroll.bar_outer_margin = 5.0;
    s.spacing.scroll.foreground_color = false;
}

/// The look of a dialog window: #1E1E1E, hairline outline, 16 pt margin.
pub fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.style()).fill(BG2.c()).stroke(Stroke::new(1.0, gy(0x3A3A3A))).inner_margin(16.0).corner_radius(4.0)
}

// ------------------------------------------------------------------ layout flow

/// What the previous row of a card was; decides the gap before the next one.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Title,
    Sub,
    Ctl,
    Sep,
    Note,
}

#[derive(Clone, Copy)]
struct Flow {
    prev: Kind,
    indent: f32,
}

fn flow_id() -> egui::Id {
    egui::Id::new("kit_flow")
}

fn flow(ui: &Ui) -> Flow {
    ui.ctx().data(|d| d.get_temp(flow_id())).unwrap_or(Flow { prev: Kind::Title, indent: 52.0 })
}

fn set_flow(ui: &Ui, f: Flow) {
    ui.ctx().data_mut(|d| d.insert_temp(flow_id(), f));
}

/// Left edge (from the card's left) of the controls that follow.
pub fn indent(ui: &Ui, x: f32) {
    let mut f = flow(ui);
    f.indent = x;
    set_flow(ui, f);
}

/// The vertical gap between two kinds of rows, as measured.
fn gap(ui: &mut Ui, cur: Kind) {
    let mut f = flow(ui);
    let g = match (f.prev, cur) {
        (Kind::Title, Kind::Sub) => 13.5,
        (Kind::Title, _) => 13.0,
        (Kind::Sub, Kind::Note) => 8.0,
        (Kind::Sub, _) => 13.5,
        (Kind::Sep, _) => 15.5,
        (_, Kind::Sep) => 16.0,
        (Kind::Note, _) | (_, Kind::Note) => 6.0,
        (Kind::Ctl, Kind::Sub) => 16.0,
        _ => 10.0,
    };
    ui.add_space(g);
    f.prev = cur;
    set_flow(ui, f);
}

/// A row of exactly `h` pt, its content starting `x` pt from the card's left edge and
/// centred vertically.
pub fn row_at<R>(ui: &mut Ui, kind: Kind, h: f32, x: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    gap(ui, kind);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let inner = Rect::from_min_max(pos2(rect.min.x + x, rect.min.y), rect.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing = Vec2::ZERO;
    add(&mut child)
}

/// A control row at the current indent.
pub fn row<R>(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let x = flow(ui).indent;
    row_at(ui, Kind::Ctl, h, x, add)
}

/// Free-form content (grids, text fields, lists) at the current indent.
pub fn block<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    block_kind(ui, Kind::Ctl, add)
}

fn block_kind<R>(ui: &mut Ui, kind: Kind, add: impl FnOnce(&mut Ui) -> R) -> R {
    gap(ui, kind);
    let x = flow(ui).indent;
    let keep = flow(ui);
    let r = ui
        .horizontal_top(|ui| {
            ui.add_space(x);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                ui.set_max_width((ui.available_width() - 16.0).max(40.0));
                add(ui)
            })
            .inner
        })
        .inner;
    set_flow(ui, keep);
    r
}

/// Card on #1E1E1E with a 17 pt bold heading 25.5 pt from its left edge; 4 pt of black after it.
pub fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    card_help(ui, title, None, add)
}

pub fn card_help(ui: &mut Ui, title: &str, tip: Option<&str>, add: impl FnOnce(&mut Ui)) {
    card_x(ui, title, vec2(0.0, 0.0), tip, add)
}

/// A card whose heading is moved by `shift` from its usual place (the original sets the
/// heading of Audio > Metronome 8 pt further left and 5 pt higher than every other one).
pub fn card_x(ui: &mut Ui, title: &str, shift: Vec2, tip: Option<&str>, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width().min(CARD_W);
    egui::Frame::default().fill(BG2.c()).show(ui, |ui| {
        ui.set_width(w);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.add_space(20.0);
        set_flow(ui, Flow { prev: Kind::Title, indent: 52.0 });
        let (r, _) = ui.allocate_exact_size(vec2(w, 23.0), Sense::hover());
        // (A juce::Label at 20, 20, 580 x 22 with Font(20, bold): its text starts 5 pt in and 1 pt down.)
        crate::prefs_font::draw(ui, Rect::from_min_size(pos2(r.min.x + 25.0 + shift.x, r.min.y + 1.0 + shift.y), vec2(570.0, 20.0)), title, 20.0, true, WHITE.c(), Just::TopLeft);
        if let Some(tip) = tip {
            help_in(ui, Rect::from_min_size(pos2(r.min.x + help_x(ui, title, 20.0), r.min.y - 1.0 + 4.2), vec2(14.0, 11.6)), tip);
        }
        add(ui);
        ui.add_space(20.0);
    });
    ui.add_space(4.0);
}

/// 16 pt bold sub heading, 57.5 pt in; the controls after it are 84 pt in.
pub fn sub(ui: &mut Ui, text: &str) {
    sub_help(ui, text, None)
}

pub fn sub_help(ui: &mut Ui, text: &str, tip: Option<&str>) {
    row_at(ui, Kind::Sub, 20.0, 57.0, |ui| {
        let r = ui.max_rect();
        // (A juce::Label 52 pt in, 20 pt tall, Font(18, bold).)
        crate::prefs_font::draw(ui, Rect::from_min_size(pos2(r.min.x, r.min.y + 1.0), vec2(540.0, 18.0)), text, 18.0, true, WHITE.c(), Just::TopLeft);
        if let Some(tip) = tip {
            help_in(ui, Rect::from_min_size(pos2(r.min.x - 57.0 + 32.0 + help_x(ui, text, 18.0), r.min.y + 4.2), vec2(14.0, 11.6)), tip);
        }
    });
    indent(ui, 84.0);
}

/// 1 pt line across the card; the controls after it are back at 52 pt.
pub fn separator(ui: &mut Ui) {
    separator_col(ui, SEP.c())
}

/// The same line in another colour (Audio > Metronome has a white one).
pub fn separator_col(ui: &mut Ui, col: Color32) {
    gap(ui, Kind::Sep);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(Rect::from_min_max(pos2(r.min.x + 20.0, r.min.y), pos2((r.min.x + 608.0).min(r.max.x - 16.0), r.max.y)), 0.0, col);
    indent(ui, 52.0);
}

/// Small grey explanation (Recordbox's own; the original has none).
pub fn note(ui: &mut Ui, s: &str) {
    block_kind(ui, Kind::Note, |ui| {
        ui.add(egui::Label::new(text(s, 11.0).color(DIM)).wrap());
    });
}

/// A line of body text at the current indent.
pub fn line(ui: &mut Ui, s: &str) {
    block_kind(ui, Kind::Note, |ui| {
        ui.add(egui::Label::new(text(s, 13.0).color(TEXT)).wrap());
    });
}

// ------------------------------------------------------------------ controls

/// Text laid out with `tracking` pt added between letters. The original's text engine kerns
/// and spaces each kind of label a little differently; the kit's fonts are not kerned, so
/// each kind carries the average difference measured against the reference.
pub fn galley(ui: &Ui, s: &str, font: FontId, color: Color32, tracking: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(s.to_string(), font, color, f32::INFINITY);
    for sec in &mut job.sections {
        sec.format.extra_letter_spacing = tracking;
    }
    ui.painter().layout_job(job)
}

/// Paint tracked text anchored at `pos`; returns its rectangle.
pub fn paint_text(ui: &Ui, pos: egui::Pos2, align: Align2, s: &str, font: FontId, color: Color32, tracking: f32) -> Rect {
    let g = galley(ui, s, font, color, tracking);
    let r = align.anchor_size(pos, g.size());
    ui.painter().galley(r.min, g, color);
    r
}

fn label_galley(ui: &Ui, s: &str, size: f32, color: Color32, _tracking: f32) -> std::sync::Arc<egui::Galley> {
    // Captions are Font(0.85 x row height): 16.15 in the usual 19 pt row (`size` 14).
    crate::prefs_font::galley(ui, s, CTL * size / 14.0, false, color)
}

/// A colour at JUCE's `setOpacity` for a control that is switched off (0.3 for switches,
/// 0.5 for radio buttons and check boxes).
fn faded(c: Color32, enabled: bool, opacity: f32) -> Color32 {
    if enabled { c } else { c.gamma_multiply(opacity) }
}

/// The pill switch at `min`, in a row `h` pt tall: a track 2t x t (t = 0.85 h) with fully
/// round ends, and a white knob 0.8 t across, 0.1 t in from the left end or 1.1 t when on.
pub fn paint_pill(p: &egui::Painter, min: egui::Pos2, h: f32, on: bool, enabled: bool, hovered: bool) {
    paint_pill_t(p, min, h, on as u8 as f32, enabled, hovered as u8 as f32);
}

/// `paint_pill` with the on state and the hover as amounts (0..1, from `ui_feel`): the knob
/// slides and the track's colour and hover light fade. At 0 and 1 it draws exactly what
/// `paint_pill` draws.
pub fn paint_pill_t(p: &egui::Painter, min: egui::Pos2, h: f32, on: f32, enabled: bool, hovered: f32) {
    let t = 0.85 * h;
    let track = Rect::from_min_size(min, vec2(2.0 * t, t));
    let col = if on >= 1.0 { PBLUE } else if on <= 0.0 { GREY.c() } else { crate::motion::mix(GREY.c(), PBLUE, on) };
    p.rect_filled(track, t / 2.0, faded(col, enabled, 0.3));
    let kx = if on >= 1.0 { 1.1 * t } else if on <= 0.0 { 0.1 * t } else { (0.1 + on) * t };
    p.circle_filled(pos2(min.x + kx + 0.4 * t, min.y + 0.5 * t), 0.4 * t, faded(ON_ACCENT.c(), enabled, 0.3));
    if enabled && hovered > 0.0 {
        p.rect_filled(track, t / 2.0, Color32::from_white_alpha(if hovered >= 1.0 { 26 } else { (26.0 * hovered).round() as u8 }));
    }
}

/// The radio button at `min`: a disc t across (t = 0.85 h), blue with a white dot 0.425 h
/// across when chosen.
pub fn paint_radio_at(p: &egui::Painter, min: egui::Pos2, h: f32, on: bool, enabled: bool) {
    paint_radio_t(p, min, h, on as u8 as f32, enabled);
}

/// `paint_radio_at` with the choice as an amount (0..1): the disc's blue fades in and the dot
/// grows. Exactly `paint_radio_at` at 0 and 1.
pub fn paint_radio_t(p: &egui::Painter, min: egui::Pos2, h: f32, on: f32, enabled: bool) {
    let t = 0.85 * h;
    let c = pos2(min.x + t / 2.0, min.y + t / 2.0);
    let col = if on >= 1.0 { PBLUE } else if on <= 0.0 { GREY.c() } else { crate::motion::mix(GREY.c(), PBLUE, on) };
    p.circle_filled(c, t / 2.0, faded(col, enabled, 0.5));
    if on > 0.0 {
        let r = if on >= 1.0 { 0.2125 * h } else { 0.2125 * h * (0.4 + 0.6 * on) };
        p.circle_filled(pos2(min.x + 0.425 * h, min.y + 0.425 * h), r, faded(ON_ACCENT.c(), enabled, 0.5));
    }
}

/// The check box at `min`: a square t across with a 1 pt grey outline; blue with JUCE's tick
/// when ticked.
pub fn paint_check_at(p: &egui::Painter, min: egui::Pos2, t: f32, on: bool, enabled: bool) {
    let r = Rect::from_min_size(min, vec2(t, t));
    if on {
        p.rect_filled(r, 0.0, faded(PBLUE, enabled, 0.5));
    }
    p.rect_stroke(r, 0.0, Stroke::new(1.0, faded(GREY.c(), enabled, 0.5)), StrokeKind::Middle);
    if on {
        // rekordbox's tick, measured on View > Layout (in 32nds of the box): thicker than
        // the box's own lines and reaching just above its top right corner.
        let at = |x: f32, y: f32| pos2(min.x + x * t / 32.0, min.y + y * t / 32.0);
        let c = faded(WHITE.c(), enabled, 0.5);
        p.add(egui::Shape::convex_polygon(vec![at(3.0, 12.5), at(7.5, 9.5), at(13.5, 17.0), at(14.5, 24.5)], c, Stroke::NONE));
        p.add(egui::Shape::convex_polygon(vec![at(13.5, 17.0), at(26.0, -0.5), at(32.0, 1.5), at(14.5, 24.5)], c, Stroke::NONE));
    }
}

/// The switch alone: 32 x 16 pt pill, 12.5 pt white knob.
pub fn paint_toggle(p: &egui::Painter, r: Rect, on: bool, enabled: bool) {
    paint_pill(p, pos2(r.min.x, r.center().y - CTL / 2.0), ROW_H, on, enabled, false);
}

/// Switch with its label, inside a row. Returns true when it was switched.
pub fn toggle_raw(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    let en = ui.is_enabled();
    let g = label_galley(ui, label, 14.0, faded(WHITE.c(), en, 0.3), -0.21);
    let gh = caption_h(&g);
    let (r, resp) = ui.allocate_exact_size(vec2(37.0 + g.size().x, gh.max(16.0)), Sense::click());
    if crate::motion::on() {
        // (The new state, so the knob starts sliding on the click's own frame.)
        let to = *on != resp.clicked();
        paint_pill_t(ui.painter(), pos2(r.min.x, r.center().y - CTL / 2.0), ROW_H, crate::ui_feel::on_amt(resp.id, to), en, crate::ui_feel::hover(&resp));
    } else {
        paint_pill(ui.painter(), pos2(r.min.x, r.center().y - CTL / 2.0), ROW_H, *on, en, resp.hovered());
    }
    if en {
        crate::ui_prefs_item::kit_control("switch", label, r);
    }
    ui.painter().galley(pos2(r.min.x + 37.0, r.center().y - gh / 2.0), g, WHITE.c());
    if resp.clicked() {
        *on = !*on;
    }
    resp.clicked()
}

/// Switch row (a two-line label makes the row 32 pt).
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    let h = if label.contains('\n') { 32.0 } else { 16.0 };
    row(ui, h, |ui| toggle_raw(ui, on, label))
}

/// A switch for something Recordbox does not have: it is off and stays off. It looks like any
/// other switch that is off; hovering it gives the reason, and a click returns true so the
/// caller can say why nothing changed.
pub fn toggle_off(ui: &mut Ui, label: &str, why: &str) -> bool {
    let h = if label.contains('\n') { 32.0 } else { 16.0 };
    row(ui, h, |ui| toggle_off_raw(ui, label, why))
}

pub fn toggle_off_raw(ui: &mut Ui, label: &str, why: &str) -> bool {
    let g = label_galley(ui, label, 14.0, WHITE.c(), -0.21);
    let gh = caption_h(&g);
    let (r, resp) = ui.allocate_exact_size(vec2(37.0 + g.size().x, gh.max(16.0)), Sense::click());
    paint_pill(ui.painter(), pos2(r.min.x, r.center().y - CTL / 2.0), ROW_H, false, true, false);
    ui.painter().galley(pos2(r.min.x + 37.0, r.center().y - gh / 2.0), g, WHITE.c());
    resp.on_hover_text(why).clicked()
}

/// Height of a caption laid out by `label_galley` (its lines are 16.15 pt apart).
fn caption_h(g: &egui::Galley) -> f32 {
    crate::prefs_font::height(g, CTL)
}

/// A switch that cannot be changed here: drawn in the state it really is in; hovering gives
/// the reason and a click returns true so the caller can say it.
pub fn toggle_locked(ui: &mut Ui, on: bool, label: &str, why: &str) -> bool {
    row(ui, 16.0, |ui| {
        let g = label_galley(ui, label, 14.0, WHITE.c(), -0.21);
        let gh = caption_h(&g);
        let (r, resp) = ui.allocate_exact_size(vec2(37.0 + g.size().x, gh.max(16.0)), Sense::click());
        paint_pill(ui.painter(), pos2(r.min.x, r.center().y - CTL / 2.0), ROW_H, on, true, false);
        ui.painter().galley(pos2(r.min.x + 37.0, r.center().y - gh / 2.0), g, WHITE.c());
        resp.on_hover_text(why).clicked()
    })
}

/// A checkbox for something Recordbox does not have: greyed and not ticked, with the reason
/// on hover. Inside a row.
pub fn checkbox_off_raw(ui: &mut Ui, label: &str, why: &str) -> Response {
    let g = label_galley(ui, label, 14.0, faded(WHITE.c(), false, 0.5), 0.1);
    let gh = caption_h(&g);
    let (r, resp) = ui.allocate_exact_size(vec2(21.0 + g.size().x, gh.max(16.0)), Sense::hover());
    paint_check_at(ui.painter(), pos2(r.min.x, r.center().y - CTL / 2.0), CTL, false, false);
    ui.painter().galley(pos2(r.min.x + 21.0, r.center().y - gh / 2.0), g, WHITE.c());
    resp.on_hover_text(why)
}

pub fn checkbox_off(ui: &mut Ui, label: &str, why: &str) {
    row(ui, 16.0, |ui| {
        checkbox_off_raw(ui, label, why);
    });
}

/// A radio button that cannot be chosen: greyed, with the reason on hover.
pub fn radio_off(ui: &mut Ui, on: bool, label: &str, why: &str) {
    row(ui, 16.0, |ui| {
        ui.add_enabled_ui(false, |ui| radio_raw(ui, on, label)).inner.on_disabled_hover_text(why);
    });
}

/// Continuous slider (Audio page): 516 pt track 62 pt from the card's left, 5 x 20 pt handle.
/// `t` is 0..=1. Returns true while it is being moved.
pub fn slider(ui: &mut Ui, t: &mut f32) -> Response {
    let handle_w = 5.0_f32;
    row_at(ui, Kind::Ctl, 20.0, 62.0, |ui| {
        let w = 516.0_f32.min(ui.available_width() - 20.0);
        let (r, mut resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click_and_drag());
        if let Some(p) = resp.interact_pointer_pos() {
            let n = ((p.x - r.min.x - handle_w / 2.0) / (w - handle_w)).clamp(0.0, 1.0);
            if n != *t {
                *t = n;
                resp.mark_changed();
            }
        }
        let en = ui.is_enabled();
        crate::ui_feel::grab_cursor(&resp);
        paint_slider_t(ui.painter(), r, r.min.x + t.clamp(0.0, 1.0) * (w - handle_w), en, crate::ui_feel::hot(&resp));
        resp
    })
}

/// A slider's track and handle as rekordbox draws them: the track is the middle fifth of the
/// height (#0F0F0F), the handle a 5 pt wide bar from `x`, white at 0.9 (full when the pointer
/// is on it).
pub fn paint_slider(p: &egui::Painter, track: Rect, x: f32, enabled: bool, hot: bool) {
    paint_slider_t(p, track, x, enabled, hot as u8 as f32);
}

/// `paint_slider` with the handle's light as an amount (0..1, eased by `ui_feel::hot`).
pub fn paint_slider_t(p: &egui::Painter, track: Rect, x: f32, enabled: bool, hot: f32) {
    let h = track.height();
    p.rect_filled(Rect::from_min_size(pos2(track.min.x, track.min.y + (0.4 * h).round()), vec2(track.width(), (0.2 * h).round())), 0.0, faded(gy(0x0F0F0F), enabled, 0.3));
    let a = if !enabled { 0.35 } else if hot >= 1.0 { 1.0 } else if hot <= 0.0 { 0.9 } else { 0.9 + 0.1 * hot };
    p.rect_filled(Rect::from_min_size(pos2(x, track.min.y), vec2(5.0, h)), 0.0, WHITE.gamma_multiply(a));
}

pub fn paint_radio(p: &egui::Painter, c: egui::Pos2, on: bool, enabled: bool) {
    paint_radio_at(p, pos2(c.x - CTL / 2.0, c.y - CTL / 2.0), ROW_H, on, enabled);
}

/// 16 pt radio button with its label 21 pt from its left edge. True when clicked.
pub fn radio_raw(ui: &mut Ui, on: bool, label: &str) -> Response {
    let en = ui.is_enabled();
    let g = label_galley(ui, label, 14.0, faded(WHITE.c(), en, 0.5), -0.05);
    let gh = caption_h(&g);
    let (r, resp) = ui.allocate_exact_size(vec2(21.0 + g.size().x, gh.max(16.0)), Sense::click());
    let to = if crate::motion::on() { on || (en && resp.clicked()) } else { on };
    paint_radio_t(ui.painter(), pos2(r.min.x, r.center().y - CTL / 2.0), ROW_H, crate::ui_feel::on_amt(resp.id, to), en);
    if en {
        crate::ui_prefs_item::kit_control("radio", label, r);
    }
    ui.painter().galley(pos2(r.min.x + 21.0, r.center().y - gh / 2.0), g, WHITE.c());
    resp
}

pub fn radio(ui: &mut Ui, on: bool, label: &str) -> bool {
    row(ui, 16.0, |ui| radio_raw(ui, on, label).clicked())
}

/// `paint_check_at` with the tick as an amount (0..1): the blue and the tick fade in.
/// Exactly `paint_check_at` at 0 and 1.
pub fn paint_check_t(p: &egui::Painter, min: egui::Pos2, t: f32, on: f32, enabled: bool) {
    if on <= 0.0 || on >= 1.0 {
        return paint_check_at(p, min, t, on >= 1.0, enabled);
    }
    let r = Rect::from_min_size(min, vec2(t, t));
    p.rect_filled(r, 0.0, crate::motion::fade(faded(PBLUE, enabled, 0.5), on));
    p.rect_stroke(r, 0.0, Stroke::new(1.0, faded(GREY.c(), enabled, 0.5)), StrokeKind::Middle);
    let at = |x: f32, y: f32| pos2(min.x + x * t / 32.0, min.y + y * t / 32.0);
    let c = crate::motion::fade(faded(WHITE.c(), enabled, 0.5), on);
    p.add(egui::Shape::convex_polygon(vec![at(3.0, 12.5), at(7.5, 9.5), at(13.5, 17.0), at(14.5, 24.5)], c, Stroke::NONE));
    p.add(egui::Shape::convex_polygon(vec![at(13.5, 17.0), at(26.0, -0.5), at(32.0, 1.5), at(14.5, 24.5)], c, Stroke::NONE));
}

pub fn paint_checkbox(p: &egui::Painter, r: Rect, on: bool, enabled: bool) {
    paint_check_at(p, r.min, r.width().min(r.height()), on, enabled);
}

/// 16.5 pt square checkbox with its label 21 pt from its left edge.
pub fn checkbox_raw(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let en = ui.is_enabled();
    let g = label_galley(ui, label, 14.0, faded(WHITE.c(), en, 0.5), 0.1);
    let gh = caption_h(&g);
    let w = if label.is_empty() { CTL } else { 21.0 + g.size().x };
    let (r, mut resp) = ui.allocate_exact_size(vec2(w, gh.max(16.0)), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    paint_check_t(ui.painter(), pos2(r.min.x, r.center().y - CTL / 2.0), CTL, crate::ui_feel::on_amt(resp.id, *on), en);
    if en {
        crate::ui_prefs_item::kit_control("check", label, r);
    }
    ui.painter().galley(pos2(r.min.x + 21.0, r.center().y - gh / 2.0), g, WHITE.c());
    resp
}

pub fn checkbox(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    row(ui, 16.0, |ui| checkbox_raw(ui, on, label))
}

/// Where the "?" box of a heading starts, from the left edge of the card: rekordbox puts it
/// 123 pt after the width the heading would have in the regular face (measured on "Auto Cue",
/// "STEMS Analysis Process" and "STEM Waveform Display": 123.4, 122.1 and 123.1).
pub fn help_x(ui: &Ui, title: &str, h: f32) -> f32 {
    crate::prefs_font::width(ui, title, h, false).floor() + 123.5
}

/// The "?" box (14 x 11.6 pt: rekordbox's 23 x 19 px hint image fitted into the 14 pt inside
/// of its 20 pt button); its text is shown on hover. The image is rekordbox's own when it is
/// installed; otherwise the same box is drawn.
pub fn help_in(ui: &mut Ui, r: Rect, tip: &str) -> Response {
    let resp = ui.interact(r.expand(3.0), ui.id().with(("help", r.min.x as i32, r.min.y as i32)), Sense::hover());
    if let Some((tex, _)) = crate::prefs_font::skin(ui.ctx(), "p3005_hint.png") {
        crate::prefs_font::paint_skin(ui.painter(), &tex, r, if resp.hovered() { WHITE.c() } else { Color32::from_white_alpha(255) });
    } else {
        ui.painter().rect_stroke(r, 1.0, Stroke::new(0.6, gy(0x646464)), StrokeKind::Inside);
        ui.painter().text(pos2(r.center().x, r.center().y + 0.25), Align2::CENTER_CENTER, "?", inter_bold(9.0), gy(0x8C8C8C));
    }
    resp.on_hover_text(tip)
}

/// "?" box `space` pt after the previous control of a row.
pub fn help(ui: &mut Ui, space: f32, tip: &str) {
    ui.add_space(space);
    let (r, _) = ui.allocate_exact_size(vec2(14.0, 11.6), Sense::hover());
    help_in(ui, r.translate(vec2(0.0, 1.0)), tip);
}

pub fn chevron(ui: &Ui, rect: Rect, _v: &egui::style::WidgetVisuals, _open: bool, _ab: egui::AboveOrBelow) {
    let c = if ui.is_enabled() { ARROW.c() } else { gy(0x4A4A4A) };
    chevron_col(ui, rect, c)
}

/// The arrow of a dropdown that is shown greyed.
pub fn chevron_dim(ui: &Ui, rect: Rect, _v: &egui::style::WidgetVisuals, _open: bool, _ab: egui::AboveOrBelow) {
    chevron_col(ui, rect, gy(0x2E2E2E))
}

fn chevron_col(ui: &Ui, rect: Rect, c: Color32) {
    // rekordbox's path, from the field's right edge w and middle height m: (w - 17, m - 2),
    // (w - 10, m + 3), (w - 3, m - 2), 2 pt wide.
    let w = rect.max.x + ui.spacing().button_padding.x;
    let m = rect.center().y.floor();
    ui.painter().add(egui::Shape::line(vec![pos2(w - 17.0, m - 2.0), pos2(w - 10.0, m + 3.0), pos2(w - 3.0, m - 2.0)], Stroke::new(2.0, c)));
}

/// Dropdown: black field with 3 pt corners, bold value, grey arrow. `size` is the value's
/// font size (14 in most places, 12 on the DJ System pages).
pub fn combo(id: impl std::hash::Hash, width: f32, value: impl Into<String>, size: f32) -> egui::ComboBox {
    // (The value is Font(16, bold) in a 22 pt field; `size` 12 is the DJ System pages' smaller one.)
    egui::ComboBox::from_id_salt(id).width(width).selected_text(RichText::new(value).font(pref_bold(size * 16.0 / 14.0)).color(WHITE)).icon(chevron)
}

/// A row holding one dropdown `h` pt tall (22, or 19 on the DJ System pages).
pub fn combo_row<R>(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    row(ui, h, |ui| {
        ui.spacing_mut().interact_size.y = h;
        ui.spacing_mut().button_padding.y = if h < 22.0 { 0.0 } else { 2.0 };
        add(ui)
    })
}

/// "Label   [dropdown]" row: label 90 pt in, the field 209 pt in.
pub fn labeled<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    row_at(ui, Kind::Ctl, 22.0, 90.0, |ui| {
        let en = ui.is_enabled();
        let (r, _) = ui.allocate_exact_size(vec2(119.0, 22.0), Sense::hover());
        crate::prefs_font::draw(ui, r, label, 16.0, false, if en { WHITE.c() } else { OFF.c() }, Just::Left);
        add(ui)
    })
}

/// Slider with `max + 1` positions: 4 pt track, 5 x 20 pt handle, "Small"/"Large" under it.
pub fn step_slider(ui: &mut Ui, v: &mut u32, max: u32, lo: &str, hi: &str) -> bool {
    let mut changed = false;
    let (track_w, handle_w) = (504.0_f32, 5.0_f32);
    row_at(ui, Kind::Ctl, 20.0, 94.0, |ui| {
        let w = track_w.min(ui.available_width() - 20.0);
        let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click_and_drag());
        if let Some(p) = resp.interact_pointer_pos() {
            let t = ((p.x - r.min.x - handle_w / 2.0) / (w - handle_w)).clamp(0.0, 1.0);
            let n = (t * max as f32).round() as u32;
            if n != *v {
                *v = n;
                changed = true;
            }
        }
        let x = r.min.x + (*v).min(max) as f32 / max.max(1) as f32 * (w - handle_w);
        // (Each step is a jump: the handle glides to it.)
        let x = crate::ui_feel::shown(resp.id, x, true);
        crate::ui_feel::grab_cursor(&resp);
        paint_slider_t(ui.painter(), r, x, true, crate::ui_feel::hot(&resp));
    });
    ui.add_space(12.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
    ui.add_space(1.0);
    let w = track_w.min(r.width() - 114.0);
    // (Labels 84 .. 334 and 338 .. 588 pt, Font(14), top left and top right.)
    crate::prefs_font::draw(ui, Rect::from_min_size(pos2(r.min.x + 89.0, r.min.y - 1.0), vec2(240.0, 14.0)), lo, 14.0, false, WHITE.c(), Just::TopLeft);
    crate::prefs_font::draw(ui, Rect::from_min_max(pos2(r.min.x + 343.0, r.min.y - 1.0), pos2(r.min.x + 94.0 + w + 9.0, r.min.y + 13.0)), hi, 14.0, false, WHITE.c(), Just::TopRight);
    // The next row keeps 16 pt of distance, as after a separator.
    let mut f = flow(ui);
    f.prev = Kind::Sep;
    set_flow(ui, f);
    changed
}

fn gradient(p: &egui::Painter, r: Rect, top: Color32, bot: Color32) {
    let mut m = egui::Mesh::default();
    m.colored_vertex(r.left_top(), top);
    m.colored_vertex(r.right_top(), top);
    m.colored_vertex(r.right_bottom(), bot);
    m.colored_vertex(r.left_bottom(), bot);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(m));
}

/// The dark push button: black outline, light hairline on top, #242427 to #1A1A1C.
pub fn paint_button(p: &egui::Painter, r: Rect, hovered: bool, down: bool) {
    paint_button_t(p, r, hovered as u8 as f32, down as u8 as f32);
}

/// `paint_button` with hover and press as amounts (0..1, from `ui_feel`); exactly
/// `paint_button` at 0 and 1.
pub fn paint_button_t(p: &egui::Painter, r: Rect, hovered: f32, down: f32) {
    p.rect_filled(r, 2.5, gy(0x000000));
    let i = r.shrink(0.75);
    let m = crate::motion::mix;
    let (top, bot) = (m(gy(0x242427), gy(0x2E2E32), hovered), m(gy(0x1A1A1C), gy(0x232326), hovered));
    let (top, bot) = (m(top, gy(0x161618), down), m(bot, gy(0x1C1C1E), down));
    gradient(p, i, top, bot);
    if down < 1.0 {
        let c = gy(0x323335);
        p.rect_filled(Rect::from_min_max(pos2(i.min.x + 0.5, i.min.y), pos2(i.max.x - 0.5, i.min.y + 0.5)), 0.0, if down <= 0.0 { c } else { crate::motion::fade(c, 1.0 - down) });
    }
}

/// Push button widget: `ui.add(btn("OK"))`, `ui.add_enabled(can, btn("Export").min_size(..))`.
pub struct Btn {
    text: String,
    min: Vec2,
    exact: Option<Vec2>,
    size: f32,
}

pub fn btn(text: impl Into<String>) -> Btn {
    Btn { text: text.into(), min: vec2(0.0, 22.0), exact: None, size: 13.0 }
}

impl Btn {
    pub fn min_size(mut self, v: Vec2) -> Self {
        self.min = v;
        self
    }
    pub fn exact(mut self, v: Vec2) -> Self {
        self.exact = Some(v);
        self
    }
    pub fn font_size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }
    /// The small button of list rows ("Remove", "Default").
    pub fn small(mut self) -> Self {
        self.size = 11.0;
        self.min = vec2(0.0, 18.0);
        self
    }
}

impl egui::Widget for Btn {
    fn ui(self, ui: &mut Ui) -> Response {
        let en = ui.is_enabled();
        let g = ui.painter().layout_no_wrap(self.text, inter(self.size), if en { WHITE.c() } else { OFF.c() });
        let size = self.exact.unwrap_or(vec2((g.size().x + 24.0).max(self.min.x), self.min.y));
        let (r, resp) = ui.allocate_exact_size(size, Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, en, g.text()));
        if crate::motion::on() {
            paint_button_t(ui.painter(), r, crate::ui_feel::hover_if(&resp, en), crate::ui_feel::press_if(&resp, en));
        } else {
            paint_button(ui.painter(), r, en && resp.hovered(), en && resp.is_pointer_button_down_on());
        }
        ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0), g, WHITE.c());
        resp
    }
}

/// Dialog buttons as the original orders them, right-aligned: the confirming button first,
/// "Cancel" (or "No") last. Returns the index of the clicked one.
pub fn dialog_buttons(ui: &mut Ui, labels: &[&str]) -> Option<usize> {
    let mut hit = None;
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let w: f32 = labels.len() as f32 * 96.0 + (labels.len().saturating_sub(1)) as f32 * 8.0;
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        for (i, l) in labels.iter().enumerate() {
            if ui.add(btn(*l).min_size(vec2(96.0, 24.0))).clicked() {
                hit = Some(i);
            }
        }
    });
    hit
}

/// The message of a dialog: 14 pt white, wrapped.
pub fn message(ui: &mut Ui, s: &str) {
    ui.add(egui::Label::new(text(s, 14.0).color(WHITE)).wrap());
}

/// Recordbox's own explanation under a dialog's message.
pub fn message_note(ui: &mut Ui, s: &str) {
    ui.add_space(4.0);
    ui.add(egui::Label::new(text(s, 11.0).color(DIM)).wrap());
}

/// A dialog window in the kit's look: bold 13 pt title, #1E1E1E panel, not collapsible.
/// Call `apply_style(ui)` first thing inside it.
pub fn dialog<'a>(ctx: &egui::Context, title: impl Into<String>) -> egui::Window<'a> {
    egui::Window::new(RichText::new(title).font(inter_bold(13.0)).color(Tc(hex(0xB3B3B4), hex(0x000000)))).collapsible(false).frame(dialog_frame(ctx))
}

/// A dropdown with the kit's arrow and the regular face for its value.
pub fn combo_plain(id: impl std::hash::Hash) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id).icon(chevron)
}

/// A checkbox without a label (list rows).
pub struct Check<'a>(pub &'a mut bool);

impl egui::Widget for Check<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (r, mut resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
        if resp.clicked() {
            *self.0 = !*self.0;
            resp.mark_changed();
        }
        let rr = Rect::from_center_size(r.center(), vec2(14.0, 14.0));
        paint_check_t(ui.painter(), rr.min, rr.width(), crate::ui_feel::on_amt(resp.id, *self.0), ui.is_enabled());
        resp
    }
}

/// Many solid axis-aligned bars as one mesh. Waveforms are thousands of one-point columns;
/// as separate line or rectangle shapes each is tessellated on its own every frame (about a
/// third of a frame's CPU time while a track plays), as one mesh they are copied as they are.
pub struct Bars {
    mesh: egui::Mesh,
}

impl Bars {
    pub fn new() -> Self {
        Bars { mesh: egui::Mesh::default() }
    }
    pub fn rect(&mut self, r: egui::Rect, c: egui::Color32) {
        if c.a() > 0 && r.width() > 0.0 && r.height() > 0.0 {
            self.mesh.add_colored_rect(r, c);
        }
    }
    /// A one-point-wide column centred on `x` between two heights (in either order).
    pub fn column(&mut self, x: f32, y0: f32, y1: f32, c: egui::Color32) {
        self.rect(egui::Rect::from_min_max(egui::pos2(x - 0.5, y0.min(y1)), egui::pos2(x + 0.5, y0.max(y1))), c);
    }
    pub fn into_mesh(self) -> egui::Mesh {
        self.mesh
    }
    pub fn paint(self, p: &egui::Painter) {
        if !self.mesh.is_empty() {
            p.add(egui::Shape::mesh(self.mesh));
        }
    }
}

/// Where the pointer says a fader should be this frame, or None when it is not being moved.
/// Covers the press, the drag, the frame the button is released in (a fast drag's last move
/// arrives together with the release) and a press and release that both arrive in one frame,
/// which egui reports as neither a click nor a drag when the pointer moved in between.
pub(crate) fn slider_pos(ui: &Ui, resp: &egui::Response, hit: Rect) -> Option<egui::Pos2> {
    if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.is_pointer_button_down_on() || resp.clicked() || resp.drag_stopped()) {
        ui.data_mut(|d| d.insert_temp(resp.id.with("slider_down"), resp.is_pointer_button_down_on()));
        return Some(p);
    }
    // The button was held on the fader in the frame before: a release now ends that drag,
    // wherever the pointer has got to (egui starts no drag when the move and the release
    // arrive together).
    let key = resp.id.with("slider_down");
    let was_down = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
    ui.data_mut(|d| d.insert_temp(key, resp.is_pointer_button_down_on()));
    let done = ui.input(|i| {
        let (mut from, mut out) = (was_down.then_some(None), None);
        for e in &i.events {
            if let egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } = e {
                if *pressed {
                    from = hit.contains(*pos).then_some(Some(*pos));
                } else if let Some(f) = from.take() {
                    out = Some((f, *pos));
                }
            }
        }
        out
    });
    let (from, to) = done?;
    // A press made in this same frame must not have gone through a menu or window lying
    // over the fader.
    from.map_or(true, |f| ui.ctx().layer_id_at(f).map_or(true, |l| l == resp.layer_id)).then_some(to)
}

#[cfg(test)]
mod slider_tests {
    use super::*;
    use egui::{pos2, vec2, Event, PointerButton, RawInput, Sense};

    /// Run one frame with these pointer events over a 100 pt fader at x 100..200 and return
    /// the value it would be set to.
    fn frame(ctx: &egui::Context, events: Vec<Event>) -> Option<f32> {
        let mut out = None;
        let raw = RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let hit = Rect::from_min_max(pos2(94.0, 30.0), pos2(206.0, 54.0));
                let resp = ui.interact(hit, ui.id().with("f"), Sense::click_and_drag());
                out = slider_pos(ui, &resp, hit).map(|p| ((p.x - 100.0) / 100.0).clamp(0.0, 1.0));
            });
        });
        out
    }
    fn button(x: f32, pressed: bool) -> Event {
        Event::PointerButton { pos: pos2(x, 42.0), button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }
    fn moved(x: f32) -> Event {
        Event::PointerMoved(pos2(x, 42.0))
    }

    #[test]
    fn a_fader_follows_slow_and_fast_gestures() {
        let ctx = egui::Context::default();
        frame(&ctx, vec![]);
        // A slow drag: press, move and release in frames of their own.
        frame(&ctx, vec![moved(150.0)]);
        assert_eq!(frame(&ctx, vec![button(150.0, true)]), Some(0.5));
        assert_eq!(frame(&ctx, vec![moved(180.0)]), Some(0.8));
        assert_eq!(frame(&ctx, vec![button(180.0, false)]), Some(0.8));
        assert_eq!(frame(&ctx, vec![]), None);
        // A click whose press and release arrive together.
        assert_eq!(frame(&ctx, vec![moved(125.0), button(125.0, true), button(125.0, false)]), Some(0.25));
        frame(&ctx, vec![]);
        // A drag whose last move arrives with the release.
        assert_eq!(frame(&ctx, vec![moved(150.0), button(150.0, true)]), Some(0.5));
        assert_eq!(frame(&ctx, vec![moved(110.0), button(110.0, false)]), Some(0.1));
        frame(&ctx, vec![]);
        // A whole drag in one frame.
        assert_eq!(frame(&ctx, vec![moved(150.0), button(150.0, true), moved(190.0), button(190.0, false)]), Some(0.9));
        frame(&ctx, vec![]);
        // A press outside the fader does nothing, wherever it is released.
        assert_eq!(frame(&ctx, vec![moved(300.0), button(300.0, true), moved(150.0), button(150.0, false)]), None);
    }
}
