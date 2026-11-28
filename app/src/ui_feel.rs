//! recordbox's own feel layer (rekordbox has none of this): how controls answer the pointer
//! and how the picture moves between states. Everything here is presentation on top of state
//! that changes exactly when it did before: a click still acts on its own frame, a drag still
//! sets the value it always set; only what is drawn eases (`motion.rs`).
//!
//! With Preferences > Advanced > Others > recordbox > Interface motion set to Off, every helper
//! here returns what the call site drew before, so the app looks and behaves as it did.
//!
//! - Hover, press and release faces (`hl`, `face`, `hover`, `press`), switch / radio / check
//!   transitions (`on_amt`).
//! - Knobs and faders: Shift for fine adjustment (a tenth of the travel), the drawn value
//!   gliding to a value set by a click or a double-click reset (`shown`), value read-outs that
//!   fade in while a control is moved (`readout`), grab cursors.
//! - Lists: smooth scrolling to the selected
//!   row (`scroll_to`, `scroll_anim`), an eased row hover and selection (`row_bg`), live guides
//!   for column resizing and reordering.
//! - Drag and drop: eased drop-target highlights (`drop_glow`), a drag label with a shadow, a
//!   snap-back when a drop is refused, an insertion line that slides between rows.
//! - Panels, pages, layouts and modes: the area that changed fades in from its background
//!   (`veil`); nothing waits for it and no layout moves.
//! - Waveforms: the drawn play position advances with the frame clock between audio
//!   callbacks (`shown_pos`), so a playing deck scrolls by the same amount every frame.

use crate::app::App;
use crate::motion::{self, Kind, PerWindow};
use crate::theme::{tc, Tc};
use eframe::egui;
use egui::{pos2, vec2, Color32, CursorIcon, Id, Pos2, Rect, Response, Stroke, Ui};
use std::cell::RefCell;

// ---- Colours of what this layer adds (tokens with a Dark and a Light value).
/// Read-out bubble behind a value.
pub const READOUT_BG: Tc = tc(0x242424, 0xFFFFFF);
/// Its outline.
pub const READOUT_EDGE: Tc = tc(0x4A4A4A, 0xA0A0A0);
/// Its text.
pub const READOUT_TX: Tc = tc(0xFFFFFF, 0x000000);
/// A track list row under the pointer (an overlay over the row's own colour).
pub const ROW_HOVER: Tc = tc(0x232323, 0xE6E6E6);
/// Drop-target wash (the outline is the accent blue).
pub const DROP_WASH: Tc = tc(0x1473EB, 0x1473EB);
/// Shadow under a drag label.
pub const SHADOW: Tc = tc(0x000000, 0x000000);
/// Column resize / reorder guide.
pub const GUIDE: Tc = tc(0x1473EB, 0x1473EB);

// ------------------------------------------------------------------ hover, press, release

/// How much `resp` is hovered (0..1, eased: quick in, soft out).
pub fn hover(resp: &Response) -> f32 {
    hover_if(resp, true)
}

pub fn hover_if(resp: &Response, cond: bool) -> f32 {
    let h = (resp.hovered() && cond) as u8 as f32;
    motion::toward(resp.id.with("feel_h"), h, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade)
}

/// How much `resp` is pressed: 1 on the frame the button goes down, easing back after release.
pub fn press(resp: &Response) -> f32 {
    let d = resp.is_pointer_button_down_on() as u8 as f32;
    motion::toward(resp.id.with("feel_p"), d, 0.0, motion::RELEASE, Kind::Fade)
}

/// How much `resp` is "hot" (hovered or held), eased; for handles that light up.
pub fn hot(resp: &Response) -> f32 {
    let h = (resp.hovered() || resp.dragged()) as u8 as f32;
    motion::toward(resp.id.with("feel_hot"), h, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade)
}

/// `press` that reacts only when `cond` holds.
pub fn press_if(resp: &Response, cond: bool) -> f32 {
    let d = (resp.is_pointer_button_down_on() && cond) as u8 as f32;
    motion::toward(resp.id.with("feel_p"), d, 0.0, motion::RELEASE, Kind::Fade)
}

/// The colour of a hover-only fill (None when nothing is to be drawn): `c` while hovered,
/// fading in and out.
pub fn hover_fill(resp: &Response, c: impl Into<Color32>) -> Option<Color32> {
    hover_fill_if(resp, true, c)
}

pub fn hover_fill_if(resp: &Response, cond: bool, c: impl Into<Color32>) -> Option<Color32> {
    let c = c.into();
    if !motion::on() {
        return (resp.hovered() && cond).then_some(c);
    }
    let h = hover_if(resp, cond);
    (h > 0.0).then(|| motion::fade(c, h))
}

/// `base`, or `hot` while hovered, eased.
pub fn hl(resp: &Response, base: impl Into<Color32>, hot: impl Into<Color32>) -> Color32 {
    hl_if(resp, true, base, hot)
}

/// `base`, or `hot` while hovered or dragged (a handle), eased.
pub fn hot_col(resp: &Response, base: impl Into<Color32>, hot_c: impl Into<Color32>) -> Color32 {
    let (base, hot_c) = (base.into(), hot_c.into());
    if !motion::on() {
        return if resp.hovered() || resp.dragged() { hot_c } else { base };
    }
    motion::mix(base, hot_c, hot(resp))
}

/// `hl` that reacts only when `cond` holds (a disabled control does not light up).
pub fn hl_if(resp: &Response, cond: bool, base: impl Into<Color32>, hot: impl Into<Color32>) -> Color32 {
    let (base, hot) = (base.into(), hot.into());
    let h = resp.hovered() && cond;
    if !motion::on() {
        return if h { hot } else { base };
    }
    motion::mix(base, hot, hover_if(resp, cond))
}

/// A push face: `down` from the press frame, `hot` while hovered, `base` at rest; the press
/// shows at once and lets go softly.
pub fn face(resp: &Response, base: Color32, hot: Color32, down: Color32) -> Color32 {
    face_if(resp, true, base, hot, down)
}

pub fn face_if(resp: &Response, cond: bool, base: Color32, hot: Color32, down: Color32) -> Color32 {
    let (h, d) = (resp.hovered() && cond, resp.is_pointer_button_down_on() && cond);
    if !motion::on() {
        return if d { down } else if h { hot } else { base };
    }
    let p = motion::toward(resp.id.with("feel_p"), d as u8 as f32, 0.0, motion::RELEASE, Kind::Fade);
    motion::mix(motion::mix(base, hot, hover_if(resp, cond)), down, p)
}

/// A two-state amount for a switch, radio or check box (0 off, 1 on), eased.
pub fn on_amt(id: Id, on: bool) -> f32 {
    motion::toward(id.with("feel_on"), on as u8 as f32, motion::FEEDBACK, motion::FEEDBACK, Kind::Move)
}

/// A new status-line message fades in (160 ms) instead of replacing the last one in a frame.
pub fn toast_in(text: &str) -> f32 {
    thread_local! { static LAST: PerWindow<std::cell::Cell<u64>> = PerWindow::new(|| std::cell::Cell::new(0)); }
    let h = Id::new(text).value();
    let id = Id::new("feel_toast");
    if LAST.with(|l| l.replace(h)) != h && motion::on() {
        motion::set(id, 0.0);
    }
    motion::toward(id, 1.0, motion::HOVER_OUT, 0.0, Kind::Fade)
}

/// How much of something that shows only while `on` (signs under the pointer) is drawn:
/// fades in 110 ms, out 160 ms; exactly 0 or 1 at rest.
pub fn appear(id: Id, on: bool) -> f32 {
    motion::toward(id.with("feel_ap"), on as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade)
}

/// How far a control is into its lit state (0..1), eased (a colour change, not a move).
pub fn state_amt(id: Id, on: bool) -> f32 {
    motion::toward(id.with("feel_st"), on as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade)
}

/// A colour change between two states of a control (lit / unlit), eased.
pub fn state_col(id: Id, on: bool, off_col: Color32, on_col: Color32) -> Color32 {
    if !motion::on() {
        return if on { on_col } else { off_col };
    }
    let t = motion::toward(id.with("feel_st"), on as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade);
    motion::mix(off_col, on_col, t)
}

/// The round CUE (`cue`) or PLAY face of the players, lit / loaded as before, with eased
/// hover and a press that darkens the face on its own frame.
pub fn round_face(p: &egui::Painter, c: Pos2, dia: f32, lit: bool, loaded: bool, resp: &Response, cue: bool) {
    let (h, d) = if motion::on() { (hover(resp), press(resp)) } else { (resp.hovered() as u8 as f32, 0.0) };
    if cue {
        crate::ui_player::cue_button_face_t(p, c, dia, lit, loaded, h, d);
    } else {
        crate::ui_player::play_button_face_t(p, c, dia, lit, h, d);
    }
}

/// The kit's dark push button (`ui_kit::paint_button`) with eased hover and press.
pub fn button_face(p: &egui::Painter, r: Rect, resp: &Response) {
    if motion::on() {
        crate::ui_kit::paint_button_t(p, r, hover(resp), press(resp));
    } else {
        crate::ui_kit::paint_button(p, r, resp.hovered(), resp.is_pointer_button_down_on());
    }
}

// ------------------------------------------------------------------ cursors

/// A draggable control: an open hand over it, a closed one while it is held.
pub fn grab_cursor(resp: &Response) {
    if !motion::extras() {
        return;
    }
    if resp.dragged() || resp.is_pointer_button_down_on() {
        resp.ctx.set_cursor_icon(CursorIcon::Grabbing);
    } else if resp.hovered() {
        resp.ctx.set_cursor_icon(CursorIcon::Grab);
    }
}

/// A splitter or column edge (only where the call site did not set one already).
pub fn resize_cursor(resp: &Response, horizontal: bool) {
    if motion::extras() && (resp.hovered() || resp.dragged()) {
        resp.ctx.set_cursor_icon(if horizontal { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
    }
}

// ------------------------------------------------------------------ knobs and faders

/// The vertical drag of a knob this frame, a tenth of it while Shift is held (fine
/// adjustment; Shift has no other meaning on a knob).
pub fn knob_dy(ui: &Ui, resp: &Response) -> f32 {
    let dy = resp.drag_delta().y;
    if motion::extras() && ui.input(|i| i.modifiers.shift) {
        dy * 0.1
    } else {
        dy
    }
}

/// A fader's value under Shift: relative to where the drag started, at a tenth of the
/// pointer's travel, instead of the pointer's absolute place. `travel` is the length of the
/// fader in points; `along` is the pointer's coordinate along it (y up = larger value: pass
/// -y). Returns None when the plain absolute rule should be used (no Shift, or Off).
pub fn fader_fine(ui: &Ui, resp: &Response, value: f32, travel: f32, along: f32) -> Option<f32> {
    if !motion::extras() {
        return None;
    }
    let key = resp.id.with("feel_fine");
    let shift = ui.input(|i| i.modifiers.shift);
    let held = resp.is_pointer_button_down_on() || resp.dragged();
    if !held {
        ui.data_mut(|d| d.remove::<(f32, f32)>(key));
        return None;
    }
    if !shift {
        ui.data_mut(|d| d.remove::<(f32, f32)>(key));
        return None;
    }
    if press_started(ui, resp) { ui.data_mut(|d| d.remove::<(f32, f32)>(key)); }
    let (v0, a0) = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || (value, along)));
    Some((v0 + (along - a0) / travel.max(1.0) * 0.1).clamp(0.0, 1.0))
}

/// The value a control is drawn at: `value` itself while it is dragged or set by MIDI (so
/// the handle never lags the pointer), gliding to it when `jumped` (a click on the track, a
/// double-click reset, a label click).
pub fn shown(id: Id, value: f32, jumped: bool) -> f32 {
    let jump = if jumped { 0.0 } else { f32::INFINITY };
    motion::glide(id.with("feel_v"), value, jump, motion::GLIDE, Kind::Move)
}

/// A double-click reset can glide. Every MIDI movement and direct drag appears immediately.
pub fn knob_shown(resp: &Response, v: f32) -> f32 {
    let jump = if resp.double_clicked() { 0.0 } else { f32::INFINITY };
    motion::glide(resp.id.with("feel_v"), v, jump, motion::GLIDE, Kind::Move)
}

/// The value a fader is drawn at: gliding to where a press on its track put it and to a
/// double-click reset, exact while it is dragged.
pub fn fader_shown(ui: &Ui, resp: &Response, v: f32) -> f32 {
    let jump = if press_started(ui, resp) || resp.double_clicked() { 0.0 } else { f32::INFINITY };
    motion::glide(resp.id.with("feel_v"), v, jump, motion::GLIDE, Kind::Move)
}

thread_local! {
    static KNOB_FMT: std::cell::Cell<Option<fn(f32) -> String>> = const { std::cell::Cell::new(None) };
}

/// How the next knob drawn shows its value in its read-out (default: percent, or the offset
/// from the centre for a knob that resets to the centre).
pub fn knob_fmt(f: fn(f32) -> String) {
    KNOB_FMT.with(|c| c.set(Some(f)));
}

/// The read-out of a knob at `c` (its centre), while it is dragged.
pub fn knob_readout(ui: &Ui, resp: &Response, c: Pos2, v: f32, reset: f32) {
    let f = KNOB_FMT.with(|k| k.take());
    if !motion::extras() {
        return;
    }
    let active = resp.dragged() || resp.is_pointer_button_down_on();
    readout(ui, resp.id, pos2(c.x, c.y - 14.0), active, || match f {
        Some(f) => f(v),
        None if (reset - 0.5).abs() < 1e-6 => {
            let o = ((v - 0.5) * 200.0).round();
            if o == 0.0 { "0".to_string() } else { format!("{o:+.0}") }
        }
        None => pct(v),
    });
}

/// A channel fader's cursor and read-out (its level in dB) at the handle `h`.
pub fn fader_feel(ui: &Ui, resp: &Response, h: Pos2, v: f32, curve: u8) {
    grab_cursor(resp);
    readout(ui, resp.id, pos2(h.x + 34.0, h.y + 7.0), resp.dragged() || resp.is_pointer_button_down_on(), || {
        let g = crate::mixer::channel_fader_gain(v, curve);
        if g <= 0.0 { "-inf dB".to_string() } else { format!("{:.1} dB", 20.0 * g.log10()) }
    });
}

/// The crossfader's cursor and read-out (the two sides' shares) above the handle `h`.
pub fn xfader_feel(ui: &Ui, resp: &Response, h: Pos2, v: f32) {
    grab_cursor(resp);
    readout(ui, resp.id, h, resp.dragged() || resp.is_pointer_button_down_on(), || format!("A {:.0} · {:.0} B", (1.0 - v) * 100.0, v * 100.0));
}

/// Read-out text of the TRIM knob.
pub fn fmt_trim(v: f32) -> String {
    db(crate::mixer::trim_db(v))
}

/// Read-out text of HIGH and LOW.
pub fn fmt_shelf(v: f32) -> String {
    db(crate::mixer::shelf_db(v))
}

/// Read-out text of MID.
pub fn fmt_mid(v: f32) -> String {
    db(crate::mixer::mid_db_q(v).0)
}

/// True on the frame a press starts on `resp` (the pointer went down on it).
pub fn press_started(ui: &Ui, resp: &Response) -> bool {
    resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed())
}

/// A small value label at `at` (its bottom centre) that fades in while `active` and out after.
/// `text` is only called while the label is visible (no work and no allocation otherwise).
pub fn readout(ui: &Ui, id: Id, at: Pos2, active: bool, text: impl FnOnce() -> String) {
    if !motion::extras() {
        return;
    }
    let a = motion::toward(id.with("feel_ro"), active as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT * 1.5, Kind::Fade);
    if a <= 0.0 {
        return;
    }
    let text = text();
    let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("feel_readout")));
    let font = crate::theme::rb_bold(10.0);
    let g = lp.layout_no_wrap(text, font, motion::fade(READOUT_TX.c(), a));
    let r = Rect::from_center_size(pos2(at.x, at.y - 8.0 - 2.0 * (1.0 - a)), vec2(g.size().x + 10.0, 15.0));
    lp.rect_filled(r.translate(vec2(0.0, 1.0)), 3.0, motion::fade(SHADOW.c(), 0.35 * a));
    lp.rect_filled(r, 3.0, motion::fade(READOUT_BG.c(), 0.96 * a));
    lp.rect_stroke(r, 3.0, Stroke::new(1.0, motion::fade(READOUT_EDGE.c(), a)), egui::StrokeKind::Inside);
    lp.galley(pos2(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0), g, READOUT_TX.c());
}

/// Read-out text for a 0..1 control shown as a percentage.
pub fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

/// Read-out text in dB ("-inf dB" at silence).
pub fn db(v: f32) -> String {
    if v.is_finite() { format!("{v:+.1} dB") } else { "-inf dB".to_string() }
}

// ------------------------------------------------------------------ lists

/// A list row's background: the selection colour fades in and out, and the row under the
/// pointer gets a soft lift (recordbox's own; rekordbox's list has no hover). Returns the
/// colour the row is filled with; `base` is its colour unselected, `sel` selected.
pub fn row_bg(ui: &Ui, key: Id, rect: Rect, selected: bool, base: Color32, sel: Color32) -> Color32 {
    if !motion::on() {
        let c = if selected { sel } else { base };
        if motion::extras() && !selected && hover_row(ui, rect) {
            return ROW_HOVER.c();
        }
        return c;
    }
    let s = motion::toward(key.with("feel_sel"), selected as u8 as f32, motion::FEEDBACK * 0.6, motion::HOVER_OUT, Kind::Fade);
    let h = motion::toward(key.with("feel_rh"), (!selected && hover_row(ui, rect)) as u8 as f32, motion::HOVER_IN, motion::HOVER_OUT, Kind::Fade);
    motion::mix(motion::mix(base, ROW_HOVER.c(), h), sel, s)
}

/// The pointer is over `rect` and nothing is being dragged or pressed elsewhere.
fn hover_row(ui: &Ui, rect: Rect) -> bool {
    let Some(p) = ui.ctx().pointer_hover_pos() else { return false };
    rect.contains(p) && ui.clip_rect().contains(p) && !ui.ctx().dragged_id().is_some() && ui.ctx().layer_id_at(p).map_or(true, |l| l == ui.layer_id())
}

#[derive(Clone, Copy)]
struct ScrollAnim {
    from: f32,
    to: f32,
    t0: f64,
    applied: f32,
}

thread_local! {
    static SCROLLS: PerWindow<RefCell<Vec<(Id, ScrollAnim)>>> = PerWindow::new(|| RefCell::new(Vec::new()));
}

/// Scroll `sa` (the scroll area whose state id is `state_id`) to `y`: at once when motion is
/// Off or Reduced, else smoothly (`scroll_anim` applies it frame by frame).
pub fn scroll_to(ui: &Ui, sa: egui::ScrollArea, state_id: Id, y: f32) -> egui::ScrollArea {
    if !motion::moves() {
        return sa.vertical_scroll_offset(y);
    }
    let cur = egui::scroll_area::State::load(ui.ctx(), state_id).map(|s| s.offset.y).unwrap_or(y);
    if (cur - y).abs() < 0.5 {
        return sa.vertical_scroll_offset(y);
    }
    // The first step is taken on this frame, so the list starts moving with the key press.
    let t0 = motion::now() - motion::frame_lead() as f64;
    let first = cur + (y - cur) * motion::ease(motion::frame_lead() / motion::SCROLL);
    SCROLLS.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|(i, _)| *i != state_id);
        s.push((state_id, ScrollAnim { from: cur, to: y, t0, applied: first }));
    });
    motion::keep_alive();
    sa.vertical_scroll_offset(first)
}

/// A manual scrollbar grab immediately takes priority over an animated selection.
pub fn cancel_scroll(state_id: Id) {
    SCROLLS.with(|s| s.borrow_mut().retain(|(id, _)| *id != state_id));
}

/// The in-flight smooth scroll of `state_id`, if any, applied to `sa`. A wheel, trackpad or
/// scroll-bar move by the user cancels it.
pub fn scroll_anim(ui: &Ui, sa: egui::ScrollArea, state_id: Id) -> egui::ScrollArea {
    let user = ui.input(|i| i.raw_scroll_delta != egui::Vec2::ZERO || i.smooth_scroll_delta != egui::Vec2::ZERO);
    let cur = egui::scroll_area::State::load(ui.ctx(), state_id).map(|s| s.offset.y);
    let now = motion::now();
    SCROLLS.with(|s| {
        let mut s = s.borrow_mut();
        let Some(k) = s.iter().position(|(i, _)| *i == state_id) else { return sa };
        let a = s[k].1;
        let moved_by_someone = cur.map_or(false, |c| (c - a.applied).abs() > 1.0);
        if user || moved_by_someone || !motion::moves() {
            s.remove(k);
            return if motion::moves() { sa } else { sa.vertical_scroll_offset(a.to) };
        }
        let p = ((now - a.t0) as f32 / motion::SCROLL).clamp(0.0, 1.0);
        let v = if p >= 1.0 { a.to } else { a.from + (a.to - a.from) * motion::ease(p) };
        if p >= 1.0 {
            s.remove(k);
        } else {
            s[k].1.applied = v;
            motion::keep_alive();
        }
        sa.vertical_scroll_offset(v)
    })
}

/// A vertical guide across the list while a column edge is dragged or a column is moved.
pub fn column_guide(ui: &Ui, x: f32, top: f32, bottom: f32, strength: f32) {
    if !motion::extras() || strength <= 0.0 {
        return;
    }
    let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("feel_guide")));
    lp.rect_filled(Rect::from_min_max(pos2(x - 0.5, top), pos2(x + 0.5, bottom)), 0.0, motion::fade(GUIDE.c(), 0.8 * strength));
}

/// The x of a column-move insertion mark, sliding between column edges.
pub fn slide_x(id: Id, x: f32) -> f32 {
    motion::follow(id.with("feel_slx"), x, 0.025, 0.25, Kind::Move)
}

/// The y of a list insertion line, sliding between rows.
pub fn slide_y(id: Id, y: f32) -> f32 {
    motion::follow(id.with("feel_sly"), y, 0.025, 0.25, Kind::Move)
}

// ------------------------------------------------------------------ drag and drop

thread_local! {
    static DROP: PerWindow<RefCell<DropState>> = PerWindow::new(|| RefCell::new(DropState { targets: Vec::new(), was_dragging: false, origin: None, label: String::new(), last: None, snap: None }));
}

struct DropState {
    /// Drop targets drawn this frame (for telling an accepted drop from a refused one).
    targets: Vec<Rect>,
    was_dragging: bool,
    origin: Option<Pos2>,
    label: String,
    last: Option<Pos2>,
    /// A refused drop's label going home: (from, to, start time).
    snap: Option<(Pos2, Pos2, f64)>,
}

/// A drop target's highlight: a blue wash and outline that fade in while a drag is over it.
/// Returns true when the old outline should be drawn instead (motion Off).
pub fn drop_glow(ui: &Ui, id: Id, rect: Rect, over: bool, rounding: f32) -> bool {
    DROP.with(|d| d.borrow_mut().targets.push(rect));
    if !motion::extras() {
        return over;
    }
    let a = motion::toward(id.with("feel_drop"), over as u8 as f32, motion::FEEDBACK, motion::HOVER_OUT, Kind::Fade);
    if a > 0.0 {
        let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("feel_drop")));
        let r = rect.intersect(ui.clip_rect());
        lp.rect_filled(r, rounding, motion::fade(DROP_WASH.c(), 0.16 * a));
        lp.rect_stroke(r, rounding, Stroke::new(1.5, motion::fade(crate::theme::BLUE.c(), a)), egui::StrokeKind::Inside);
    }
    false
}

/// The label that follows a drag (tracks, a playlist): drawn on top, with a soft shadow.
/// Returns false when the call site should draw its old label (motion Off).
pub fn drag_label(ui: &Ui, pp: Pos2, label: &str) -> bool {
    if !motion::extras() {
        return false;
    }
    DROP.with(|d| {
        let mut d = d.borrow_mut();
        if !d.was_dragging || d.origin.is_none() {
            d.origin = ui.input(|i| i.pointer.press_origin()).or(Some(pp));
        }
        d.label.clear();
        d.label.push_str(label);
        d.last = Some(pp);
    });
    let a = motion::toward(Id::new("feel_drag_in"), 1.0, motion::FEEDBACK, 0.0, Kind::Fade);
    paint_label(ui.ctx(), pp, label, a);
    true
}

fn paint_label(ctx: &egui::Context, pp: Pos2, label: &str, a: f32) {
    let lp = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("dragl")));
    let font = crate::theme::rb(11.0);
    let g = lp.layout_no_wrap(label.to_string(), font, motion::fade(crate::theme::ON_ACCENT.c(), a));
    let rr = Rect::from_min_size(pp + vec2(12.0, 6.0), vec2(g.size().x + 12.0, 18.0));
    lp.rect_filled(rr.translate(vec2(0.0, 2.0)).expand(1.0), 4.0, motion::fade(SHADOW.c(), 0.30 * a));
    lp.rect_filled(rr, 3.0, motion::fade(crate::theme::ROW_SEL.c(), 0.97 * a));
    lp.galley(pos2(rr.min.x + 6.0, rr.center().y - g.size().y / 2.0), g, crate::theme::ON_ACCENT.c());
}

/// Once per frame after everything is drawn: a drag that ended over nothing that takes drops
/// sends its label back to where it started.
fn drops_end_frame(app: &App, ctx: &egui::Context) {
    let dragging = app.drag.is_some() || app.node_drag.is_some();
    DROP.with(|d| {
        let mut d = d.borrow_mut();
        if d.was_dragging && !dragging && motion::moves() {
            let at = ctx.input(|i| i.pointer.latest_pos()).or(d.last);
            let over_target = at.map_or(false, |p| d.targets.iter().any(|r| r.contains(p)));
            if let (Some(from), Some(to), false) = (at, d.origin, over_target) {
                if from.distance(to) > 8.0 {
                    d.snap = Some((from, to, motion::now()));
                }
            }
        }
        if dragging {
            if !d.was_dragging {
                motion::set(Id::new("feel_drag_in"), 0.0);
            }
        } else {
            d.origin = None;
        }
        d.was_dragging = dragging;
        if let Some((from, to, t0)) = d.snap {
            let p = ((motion::now() - t0) as f32 / motion::SNAP_BACK).clamp(0.0, 1.0);
            if p >= 1.0 {
                d.snap = None;
            } else {
                let e = motion::ease(p);
                let label = d.label.clone();
                paint_label(ctx, from + (to - from) * e, &label, 1.0 - e);
                motion::keep_alive();
            }
        }
        d.targets.clear();
    });
}

// ------------------------------------------------------------------ veils (panel, page, mode changes)

#[derive(Clone, Copy)]
struct Veil {
    rect: Rect,
    t0: f64,
    from: f32,
    col: Color32,
}

thread_local! {
    static WATCH: PerWindow<RefCell<Vec<(&'static str, u64)>>> = PerWindow::new(|| RefCell::new(Vec::new()));
    static VEILS: PerWindow<RefCell<Vec<(&'static str, Veil)>>> = PerWindow::new(|| RefCell::new(Vec::new()));
}

/// Watch a piece of state that changes what an area shows (a panel shown, a page, a mode):
/// when it changes, the area fades in from `bg` (starting `strength` opaque). The new content
/// is there and live at once; the veil only covers it for the first `PANEL` seconds.
pub fn watch(key: &'static str, state: u64, rect: Rect, bg: Color32, strength: f32) {
    let changed = WATCH.with(|w| {
        let mut w = w.borrow_mut();
        match w.iter_mut().find(|(k, _)| *k == key) {
            Some((_, s)) if *s != state => {
                *s = state;
                true
            }
            Some(_) => false,
            None => {
                w.push((key, state));
                false
            }
        }
    });
    if changed && motion::on() && rect.is_positive() {
        let v = Veil { rect, t0: motion::now(), from: strength, col: bg };
        VEILS.with(|vs| {
            let mut vs = vs.borrow_mut();
            vs.retain(|(k, _)| *k != key);
            vs.push((key, v));
        });
    }
}

fn paint_veils(ctx: &egui::Context) {
    VEILS.with(|vs| {
        let mut vs = vs.borrow_mut();
        if vs.is_empty() {
            return;
        }
        if !motion::on() {
            vs.clear();
            return;
        }
        let now = motion::now();
        let lp = ctx.layer_painter(egui::LayerId::background());
        vs.retain(|(_, v)| {
            let dur = if motion::moves() { motion::PANEL } else { motion::PANEL * 0.7 };
            let p = ((now - v.t0) as f32 / dur).clamp(0.0, 1.0);
            if p >= 1.0 {
                return false;
            }
            let a = v.from * (1.0 - motion::ease(p));
            lp.with_clip_rect(v.rect).rect_filled(v.rect, 0.0, motion::fade(v.col, a));
            motion::keep_alive();
            true
        });
    });
}

/// The areas recordbox watches for changes: modes, layouts, panels, Preferences pages.
fn watch_app(app: &App, ctx: &egui::Context) {
    let screen = ctx.screen_rect();
    let bg = crate::theme::BG.c();
    let below_header = Rect::from_min_max(pos2(screen.min.x, screen.min.y + 28.0), screen.max);
    let perf = app.performance();
    let edit = crate::ui_edit::is_edit(app);
    let mode = if edit { 2 } else if perf { 1 } else { 0 };
    watch("mode", mode, below_header, bg, 0.85);
    let h = if app.prefs.browser_only && !perf { 0.0 } else { app.player_height() };
    let player = Rect::from_min_size(below_header.min, vec2(screen.width(), h));
    if perf {
        let lay = crate::ui_perf_layouts::layout(app) as u64;
        let panels = (app.prefs.perf_fx as u64) | (app.prefs.rec.panel as u64) << 1;
        watch("perf_layout", lay << 8 | mode, player, bg, 0.8);
        watch("perf_panels", panels << 8 | lay, player, bg, 0.6);
    } else if !edit {
        watch("player_layout", (app.prefs.player_layout as u64) << 1 | app.prefs.browser_only as u64, below_header, bg, 0.8);
    }
}

/// Watch only the library area in the viewport that actually paints it.
pub fn watch_library(app: &App, browser: Rect) {
    let rp = crate::ui_browser_ext::right_panel(app) as u64;
    let sub = (app.prefs.bx.sub_horizontal as u64) << 8 | (app.prefs.column_browser as u64) << 9;
    watch("browser_panels", rp | sub, browser, crate::theme::BG.c(), 0.5);
    // A new sort order: the rows fade in rather than jump.
    let sort = (app.prefs.sort_col as u64) << 1 | app.prefs.sort_asc as u64;
    watch("sort", sort, Rect::from_min_max(pos2(browser.min.x + app.prefs.bx.tree_w + 5.0, browser.min.y + 44.0), pos2(browser.max.x - 25.0, browser.max.y)), crate::theme::BG1.c(), 0.55);
}

// ------------------------------------------------------------------ waveforms: zoom

/// The time span a zoomed waveform shows, easing (in log scale, 140 ms) to a new zoom step
/// instead of jumping; exactly `span` once there. Keyed by the waveform's place `r` (two
/// players can share one `ui`).
pub fn zoom_span(ui: &Ui, r: Rect, span: f64) -> f64 {
    let target = (span as f32).ln();
    let v = motion::tween(ui.id().with(("feel_zoom", r.min.x as i32, r.min.y as i32)), target, motion::SCROLL, Kind::Move);
    if v == target { span } else { (v as f64).exp() }
}

// ------------------------------------------------------------------ waveforms: the drawn play position

#[derive(Clone, Copy, Default)]
struct PosTrack {
    shown: f64,
    real: f64,
    real_at: f64,
    rate: f64,
    frame: u64,
    /// Start of the window the rate is measured over (position, display time).
    anchor: (f64, f64),
}

thread_local! {
    static POS: RefCell<Vec<(usize, PosTrack)>> = const { RefCell::new(Vec::new()) };
    static FRAME_NO: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The play position (seconds) to draw the active deck's waveforms, platter and times at.
/// The audio thread moves the real position once per audio buffer (about 11 ms), which a
/// 120 Hz display samples unevenly (about a third of the frames see no move, others two
/// buffers' worth). The drawn position advances each frame by the playback rate measured over
/// the last 0.12 s or more, times one step of the display clock (`motion::display_time`), and
/// is pulled slowly onto the real one. A seek, a loop wrap, a scratch or a stop is followed at
/// once; a paused deck is drawn at exactly its position.
pub fn shown_pos(app: &App) -> f64 {
    let key = std::sync::Arc::as_ptr(&app.player.state) as usize;
    let (real, playing) = {
        let player = app.player.state.lock();
        let real = player.audio.as_ref().map_or(0.0, |audio| player.pos / f64::from(audio.sr));
        (real, player.playing && matches!(player.dk.jog, crate::deck::Jog::None) && player.search == 0.0)
    };
    if !motion::moves() || !playing {
        POS.with(|p| p.borrow_mut().retain(|(k, _)| *k != key));
        return real;
    }
    let frame = FRAME_NO.with(|f| f.get());
    let (now, step) = (motion::display_time(), motion::display_step());

    POS.with(|p| {
        let mut p = p.borrow_mut();
        let k = match p.iter().position(|(k, _)| *k == key) {
            Some(k) => k,
            None => {
                p.push((key, PosTrack::at(real, now, frame)));
                return real;
            }
        };
        p[k].1.position(real, now, step, frame)
    })
}

impl PosTrack {
    fn at(real: f64, now: f64, frame: u64) -> Self {
        Self { shown: real, real, real_at: now, rate: 0.0, frame, anchor: (real, now) }
    }

    fn position(&mut self, real: f64, now: f64, step: f64, frame: u64) -> f64 {
        if self.frame == frame {
            return self.shown;
        }
        let gap = frame.saturating_sub(self.frame);
        self.frame = frame;
        if gap > 2 || step <= 0.0 || step > 0.1 {
            *self = Self::at(real, now, frame);
            return real;
        }
        if real != self.real {
            // A move against the expected one is a seek, a loop wrap or a scratch.
            let expect = self.real + self.rate * (now - self.real_at);
            let jump = (real - self.real).abs() > 0.25 || (self.rate != 0.0 && (real - expect).abs() > 0.06);
            self.real = real;
            self.real_at = now;
            if jump {
                *self = Self::at(real, now, frame);
                return real;
            }
            let span = now - self.anchor.1;
            if span >= 0.12 {
                let r = (real - self.anchor.0) / span;
                self.rate = if self.rate == 0.0 { r } else { self.rate + (r - self.rate) * 0.5 };
                self.anchor = (real, now);
            }
        } else if now - self.real_at > 0.10 {
            // Avoid extrapolating through a stall, while allowing normal 2,048-frame buffers.
            *self = Self::at(real, now, frame);
            return real;
        }
        if self.rate == 0.0 {
            self.shown = real;
            return real;
        }
        // Where the audio is now (the last callback's position carried on at the rate), and
        // where the drawing would be after one more display step; the drawing is pulled onto
        // the audio slowly, so the frame-to-frame step stays even.
        let extrap = self.real + self.rate * (now - self.real_at);
        let predicted = self.shown + self.rate * step;
        let err = extrap - predicted;
        if err.abs() > 0.06 {
            self.shown = real;
        } else {
            self.shown = predicted + err * (1.0 - (-step / 0.3).exp());
        }
        // Playback owns its frame schedule, including the configured FPS cap.
        self.shown
    }
}

// ------------------------------------------------------------------ track list artwork off the frame

pub fn list_artwork(app: &mut App, path: &str) -> Option<egui::TextureHandle> {
    crate::artwork_thumbs::get(app, path)
}

/// The worker supplied a thumbnail for this viewport.
pub fn art_ready(path: &str) {
    if motion::on() { motion::set(Id::new(("feel_art", path)), 0.0); }
}

/// The tint a track list cover is drawn with: white, fading in when it has just arrived.
pub fn art_tint(path: &str) -> Color32 {
    let a = motion::toward(Id::new(("feel_art", path)), 1.0, motion::PANEL, 0.0, Kind::Fade);
    motion::fade(Color32::WHITE, a)
}

// ------------------------------------------------------------------ frame hooks

/// main.rs, at the start of `update`.
pub fn begin_frame(app: &mut App, ctx: &egui::Context) {
    let shot = app.screenshot.as_ref().map(|(_, f)| {
        let bench = crate::feel_selftest::bench_frames();
        let pic = 120 + bench;
        // From five frames before the picture is asked for, everything snaps to its target.
        (*f, *f + 5 >= pic && (app.rb_job.is_none() || *f + 5 >= 1200 + bench))
    });
    motion::begin_frame(ctx, &app.db, shot);
    FRAME_NO.with(|f| f.set(f.get() + 1));
    if shot.is_some() { crate::feel_selftest::frame_start(app, ctx); }
}

/// main.rs, at the end of `update`.
pub fn end_frame(app: &mut App, ctx: &egui::Context) {
    drops_end_frame(app, ctx);
    watch_app(app, ctx);
    paint_veils(ctx);
    if app.screenshot.is_some() { crate::feel_selftest::frame_end(app); }
    motion::end_frame(app, ctx);
}

/// The detached window owns its scroll, drop feedback and repaint clock. The main
/// window continues to own the application, transport, artwork jobs and screenshot probe.
pub fn library_frame(app: &mut App, ctx: &egui::Context, draw: impl FnOnce(&mut App)) {
    motion::in_library(|| {
        motion::start_clock(ctx);
        draw(app);
        drops_end_frame(app, ctx);
        paint_veils(ctx);
        motion::end_frame(app, ctx);
    });
}

// ------------------------------------------------------------------ Preferences

/// Preferences > Advanced > Others, under the "recordbox" heading: the interface motion level.
pub fn prefs_item(app: &mut App, ui: &mut Ui) {
    let cur = motion::setting();
    let mut pick = None;
    crate::ui_prefs_item::item(ui, "RecordboxMotion", 170.0, |it| {
        it.title("Interface motion");
        it.hint("Choose how controls, lists and panels animate. Off shows state changes immediately.");
        for (i, l) in [motion::Level::Full, motion::Level::Reduced, motion::Level::Off].into_iter().enumerate() {
            if it.radio(52.0, 56.0 + 26.0 * i as f32, cur == l, l.name()) {
                pick = Some(l);
            }
        }
        it.no_hint();
        let note = if motion::system_reduce_motion() { "Your system requests reduced motion. Full uses Reduced." } else { "Reduced keeps short fades and skips movement and smooth scrolling." };
        it.text(52.0, 136.0, 556.0, 18.0, note, 14.0, crate::ui_kit::Just::Left);
    });
    if let Some(l) = pick {
        if l != cur {
            motion::save_setting(&app.db, l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawn_position_spans_large_audio_buffers_and_snaps_on_a_seek_or_stall() {
        let mut track = PosTrack::at(0.0, 0.0, 0);
        let dt = 1.0 / 120.0;
        let buffer = 2048.0 / 44_100.0;
        let mut between = 0;
        for frame in 1..100 {
            let now = frame as f64 * dt;
            let real = (now / buffer).floor() * buffer;
            let drawn = track.position(real, now, dt, frame);
            if frame > 70 && now - track.real_at > 0.035 && (drawn - real).abs() > 0.001 {
                between += 1;
            }
        }
        assert!(between > 0, "the draw clock can advance between 46 ms audio callbacks");
        assert_eq!(track.position(18.0, 100.0 * dt, dt, 100), 18.0);
        assert_eq!(track.position(18.0, 2.0, 0.0, 101), 18.0);
    }
}
