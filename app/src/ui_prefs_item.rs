//! A Preferences item (rekordbox's `PreferenceBaseItem`: one card of a page) laid out by
//! rekordbox's own coordinates.
//!
//! rekordbox 7.2.19 builds each item in its constructor with fixed bounds in points relative
//! to the item (`checks/re-7.2.19/preferences-skins-tree.md` lists them for all 259 items):
//! the heading label at (20, 20, 580 x 22), the first row of controls at y 56 from x 52, rows
//! 26 apart, sub headings 20 tall with their controls from x 88, and a size (628 x h) per
//! item. The pages written with this module give those bounds as they are, so a page is
//! right where it has been captured and where it has not.
//!
//! Drawing follows rekordbox's look-and-feel code (see `ui_kit.rs`): a toggle button's control
//! is 0.85 of its height and its caption starts at the top left, 5 pt after the control; a
//! `juce::Label`'s text sits 5 pt in and 1 pt down; the "?" hint is placed 100 pt after the
//! width its label's text has in the regular face (`ToolTipComponent::setLeftPositionByLabel`
//! 0x101898e48) and its 14 x 11.6 pt image 3 pt further in.

use crate::prefs_font as pf;
use crate::theme::*;
use crate::ui_kit::{self as kit, Just, CTL, GREY, PBLUE, ROW_H, SEP};
use eframe::egui;
use egui::{pos2, vec2, Color32, Pos2, Rect, Response, Sense, Ui, Vec2};

/// Width of an item.
pub const W: f32 = 628.0;

/// What a page laid out: (item name, height, y of its top in the window) and every control
/// (item name, kind, caption, rectangle). Filled only while a check asks for it
/// (`record(true)`): the scripted pointer run and the geometry check read it.
#[derive(Default)]
pub struct Trace {
    pub on: bool,
    pub items: Vec<(String, f32, f32)>,
    pub controls: Vec<(String, &'static str, String, Rect)>,
}

pub static TRACE: parking_lot::Mutex<Trace> = parking_lot::Mutex::new(Trace { on: false, items: Vec::new(), controls: Vec::new() });

/// Notes a control drawn by the flow kit (ui_kit.rs), for the scripted pointer run.
pub fn kit_control(kind: &'static str, caption: &str, r: Rect) {
    let mut t = TRACE.lock();
    if t.on {
        t.controls.push(("kit".to_string(), kind, caption.to_string(), r));
    }
}

/// Notes where a tab or category button is (for the scripted pointer run).
pub fn tab_seen(name: &str, r: Rect) {
    let mut t = TRACE.lock();
    if t.on {
        t.controls.push(("#shell".to_string(), "tab", name.to_string(), r));
    }
}

/// Start (or stop) recording the layout; clears what was recorded.
pub fn record(on: bool) {
    let mut t = TRACE.lock();
    t.on = on;
    t.items.clear();
    t.controls.clear();
}

/// Forget the last frame's layout (called before a page is drawn again).
pub fn begin_frame() {
    let mut t = TRACE.lock();
    if t.on {
        t.items.clear();
        t.controls.retain(|c| c.0 == "#shell");
    }
}

/// Forget the shell's buttons too (called before the window is drawn).
pub fn begin_window() {
    let mut t = TRACE.lock();
    if t.on {
        t.items.clear();
        t.controls.clear();
    }
}

pub struct Item<'a> {
    pub ui: &'a mut Ui,
    name: &'static str,
    /// Top left of the item, and of the child component being laid out.
    o: Pos2,
    n: u32,
    /// Controls added while this is false are drawn as rekordbox draws a disabled control
    /// (0.3 or 0.5 opacity) and do not react.
    pub en: bool,
    /// Shown when the pointer rests on the controls added next: why a control is greyed, or
    /// what recordbox does (or does not yet do) with the setting.
    pub hint: Option<String>,
    /// A control was changed.
    pub changed: bool,
}

/// An item `h` pt tall on rekordbox's item background (#1E1E1E), followed by the 4 pt gap.
pub fn item<R>(ui: &mut Ui, name: &'static str, h: f32, add: impl FnOnce(&mut Item) -> R) -> R {
    item_bg(ui, name, h, Some(BG2), add)
}

/// An item without a background: rekordbox's section headings ("SAMPLER", "Quantize") and the
/// switches that stand above the cards ("Enable the STEMS Function").
pub fn item_bare<R>(ui: &mut Ui, name: &'static str, h: f32, add: impl FnOnce(&mut Item) -> R) -> R {
    item_bg(ui, name, h, None, add)
}

fn item_bg<R>(ui: &mut Ui, name: &'static str, h: f32, bg: Option<Color32>, add: impl FnOnce(&mut Item) -> R) -> R {
    let w = ui.available_width().min(W);
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    if let Some(bg) = bg {
        ui.painter().rect_filled(r, 0.0, bg);
    }
    {
        let mut t = TRACE.lock();
        if t.on {
            t.items.push((name.to_string(), h, r.min.y));
        }
    }
    let mut it = Item { ui, name, o: r.min, n: 0, en: true, hint: None, changed: false };
    let out = add(&mut it);
    ui.add_space(4.0);
    out
}

impl Item<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.o.x + x, self.o.y + y)
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.at(x, y), vec2(w, h))
    }
    fn id(&mut self) -> egui::Id {
        self.n += 1;
        egui::Id::new(("prefs_item", self.name, self.n))
    }
    fn hover(&self, resp: Response) -> Response {
        match &self.hint {
            Some(h) if !h.is_empty() => resp.on_hover_text(h.as_str()),
            _ => resp,
        }
    }
    fn note(&self, kind: &'static str, caption: &str, r: Rect) {
        let mut t = TRACE.lock();
        if t.on && self.en {
            t.controls.push((self.name.to_string(), kind, caption.to_string(), r));
        }
    }
    /// Sets the text shown on hover for the controls added next (None: no text).
    pub fn hint(&mut self, s: impl Into<String>) {
        self.hint = Some(s.into());
    }
    pub fn no_hint(&mut self) {
        self.hint = None;
    }

    /// Lays out a child component whose bounds start at (x, y).
    pub fn child<R>(&mut self, x: f32, y: f32, add: impl FnOnce(&mut Self) -> R) -> R {
        let keep = self.o;
        self.o = self.at(x, y);
        let r = add(self);
        self.o = keep;
        r
    }

    // -------------------------------------------------------------- text

    /// The item's heading: a label at (20, 20, 580 x 22), Font(20, bold).
    pub fn title(&mut self, s: &str) {
        pf::label(self.ui, self.rect(20.0, 20.0, 580.0, 22.0), s, 20.0, true, WHITE, Just::TopLeft);
    }

    /// The heading with its "?" hint.
    pub fn title_tip(&mut self, s: &str, tip: &str) {
        self.title(s);
        self.tip_by_label(20.0, 19.0, s, 20.0, tip);
    }

    /// A sub heading: a label at (x, y, 550 x 20), Font(18, bold).
    pub fn sub(&mut self, x: f32, y: f32, s: &str) {
        pf::label(self.ui, self.rect(x, y, 550.0, 20.0), s, 18.0, true, WHITE, Just::TopLeft);
    }

    pub fn sub_tip(&mut self, x: f32, y: f32, s: &str, tip: &str) {
        self.sub(x, y, s);
        self.tip_by_label(x, y - 1.0, s, 18.0, tip);
    }

    /// A label in the regular face at font height `fh`.
    pub fn text(&mut self, x: f32, y: f32, w: f32, h: f32, s: &str, fh: f32, just: Just) -> Rect {
        let c = if self.en { WHITE } else { WHITE.gamma_multiply(0.5) };
        pf::label(self.ui, self.rect(x, y, w, h), s, fh, false, c, just)
    }

    /// A label in the bold face.
    pub fn text_bold(&mut self, x: f32, y: f32, w: f32, h: f32, s: &str, fh: f32, just: Just) -> Rect {
        pf::label(self.ui, self.rect(x, y, w, h), s, fh, true, WHITE, just)
    }

    /// The 1 pt line rekordbox draws 16 pt above a section of an item, from 20 to 608.
    pub fn sep(&mut self, y: f32) {
        self.sep_col(y, SEP);
    }

    pub fn sep_col(&mut self, y: f32, col: Color32) {
        let w = (self.ui.clip_rect().max.x - self.o.x).min(608.0);
        self.ui.painter().rect_filled(Rect::from_min_max(self.at(20.0, y), self.at(w, y + 1.0)), 0.0, col);
    }

    // -------------------------------------------------------------- hints

    /// The "?" hint of a label that starts at `lx` with text `s` in Font(`fh`): its 20 pt
    /// button starts 100 pt after the text's width in the regular face, at height `y`.
    pub fn tip_by_label(&mut self, lx: f32, y: f32, s: &str, fh: f32, tip: &str) {
        let x = lx + pf::width(self.ui, s, fh, false).round() + 100.0;
        self.tip_at(x, y, tip);
    }

    /// The "?" hint of a switch at (bx, by) with caption `s`
    /// (`ToolTipComponent::setLeftPositionByButton`: 55 pt after control and caption).
    pub fn tip_by_button(&mut self, bx: f32, by: f32, s: &str, tip: &str) {
        let w = s.lines().map(|l| pf::width(self.ui, l, CTL, false).round()).fold(0.0, f32::max);
        self.tip_at(bx + CTL.floor() + w + 55.0, by - 1.0, tip);
    }

    /// The "?" hint whose 20 pt button is at (x, y).
    pub fn tip_at(&mut self, x: f32, y: f32, tip: &str) {
        let r = self.rect(x + 3.0, y + 4.2, 14.0, 11.6);
        kit::help_in(self.ui, r, tip);
    }

    // -------------------------------------------------------------- toggles

    fn toggle_row(&mut self, kind: &'static str, x: f32, y: f32, h: f32, cap_x: f32, s: &str, opacity: f32) -> (Rect, Response, bool) {
        let c = if self.en { WHITE } else { WHITE.gamma_multiply(opacity) };
        let lines = s.lines().count().max(1) as f32;
        let t = 0.85 * h / lines;
        let g = pf::galley(self.ui, s, t, false, c);
        let r = self.rect(x, y, cap_x + g.size().x, h);
        let id = self.id();
        let resp = self.ui.interact(r, id, if self.en { Sense::click() } else { Sense::hover() });
        self.ui.painter().galley(pos2(r.min.x + cap_x, r.min.y), g, c);
        self.note(kind, s, r);
        let clicked = self.en && resp.clicked();
        let resp = self.hover(resp);
        (r, resp, clicked)
    }

    /// A radio button (19 pt row): true when it was clicked.
    pub fn radio(&mut self, x: f32, y: f32, on: bool, s: &str) -> bool {
        let (r, _, clicked) = self.toggle_row("radio", x, y, ROW_H, (CTL + 5.0).floor(), s, 0.5);
        kit::paint_radio_at(self.ui.painter(), r.min, ROW_H, on, self.en);
        self.changed |= clicked && !on;
        clicked
    }

    /// One radio button per name; `sel` is the chosen one. True when the choice changed.
    pub fn radios(&mut self, x: f32, y: f32, sel: &mut usize, names: &[&str]) -> bool {
        let mut changed = false;
        for (i, n) in names.iter().enumerate() {
            if self.radio(x, y + 26.0 * i as f32, *sel == i, n) && *sel != i {
                *sel = i;
                changed = true;
            }
        }
        changed
    }

    /// A check box (19 pt row): true when it was changed.
    pub fn check(&mut self, x: f32, y: f32, v: &mut bool, s: &str) -> bool {
        let (r, _, clicked) = self.toggle_row("check", x, y, ROW_H, (CTL + 5.0).floor(), s, 0.5);
        if clicked {
            *v = !*v;
            self.changed = true;
        }
        kit::paint_check_at(self.ui.painter(), r.min, CTL, *v, self.en);
        clicked
    }

    /// A pill switch in a row `h` pt tall (19, or 38 for a caption of two lines).
    pub fn pill_h(&mut self, x: f32, y: f32, h: f32, v: &mut bool, s: &str) -> bool {
        let lines = s.lines().count().max(1) as f32;
        let t = 0.85 * h / lines;
        let (r, resp, clicked) = self.toggle_row("switch", x, y, h, (2.0 * t + 5.0).floor(), s, 0.3);
        if clicked {
            *v = !*v;
            self.changed = true;
        }
        // (The track is t tall, centred on 0.425 of the row.)
        kit::paint_pill(self.ui.painter(), pos2(r.min.x, r.min.y + 0.425 * h - t / 2.0), h / lines, *v, self.en, resp.hovered());
        clicked
    }

    pub fn pill(&mut self, x: f32, y: f32, v: &mut bool, s: &str) -> bool {
        self.pill_h(x, y, ROW_H, v, s)
    }

    /// rekordbox's `ButtonSwitchComponent`: a pill whose caption follows its state.
    pub fn pill2(&mut self, x: f32, y: f32, v: &mut bool, on: &str, off: &str) -> bool {
        let s = if *v { on } else { off };
        self.pill_h(x, y, ROW_H, v, s)
    }

    // -------------------------------------------------------------- dropdowns, sliders, buttons

    /// A dropdown at (x, y), `w` wide and 22 tall, showing `value`.
    pub fn combo(&mut self, x: f32, y: f32, w: f32, value: &str, add: impl FnOnce(&mut Ui)) {
        let r = self.rect(x, y, w, 22.0);
        let id = self.id();
        self.note("combo", value, r);
        // A value wider than the field is cut at its end.
        let mut shown = value.to_string();
        if pf::width(self.ui, &shown, 16.0, true) > w - 34.0 {
            while shown.chars().count() > 1 && pf::width(self.ui, &format!("{shown}…"), 16.0, true) > w - 34.0 {
                shown.pop();
            }
            shown.push('…');
        }
        let value = shown.as_str();
        let en = self.en;
        let hint = self.hint.clone();
        let mut child = self.ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
        child.spacing_mut().interact_size.y = 22.0;
        child.spacing_mut().button_padding = vec2(6.5, 2.0);
        if en {
            let resp = kit::combo(id, w, value, 14.0).show_ui(&mut child, add).response;
            if let Some(h) = hint.filter(|h| !h.is_empty()) {
                resp.on_hover_text(h);
            }
        } else {
            // As rekordbox shows a disabled dropdown: the value at half strength, the arrow at 0.2.
            let resp = egui::ComboBox::from_id_salt(id).width(w).selected_text(egui::RichText::new(value).font(pf::pref_bold(16.0)).color(hex(0x878787))).icon(kit::chevron_dim).show_ui(&mut child, |ui| {
                if let Some(h) = &hint {
                    ui.label(kit::text(h.as_str(), 12.0).color(DIM));
                }
            });
            if let Some(h) = hint.filter(|h| !h.is_empty()) {
                resp.response.on_hover_text(h);
            }
        }
    }

    /// A dropdown over a list of names; true when another one was chosen.
    pub fn choice(&mut self, x: f32, y: f32, w: f32, sel: &mut usize, names: &[&str]) -> bool {
        let mut pick = None;
        let cur = (*sel).min(names.len().saturating_sub(1));
        self.combo(x, y, w, names.get(cur).copied().unwrap_or(""), |ui| {
            for (i, n) in names.iter().enumerate() {
                if ui.selectable_label(cur == i, *n).clicked() {
                    pick = Some(i);
                }
            }
        });
        match pick {
            Some(i) if i != *sel => {
                *sel = i;
                self.changed = true;
                true
            }
            _ => false,
        }
    }

    /// A slider with bounds (x, y, w x 20): its track runs from 10 pt in to 10 pt before the
    /// end, the handle's left edge marks the value. `t` is 0..=1.
    pub fn slider(&mut self, x: f32, y: f32, w: f32, t: &mut f32) -> Response {
        let r = self.rect(x, y, w, 20.0);
        let track = Rect::from_min_max(pos2(r.min.x + 10.0, r.min.y), pos2(r.max.x - 10.0, r.max.y));
        let id = self.id();
        let mut resp = self.ui.interact(r, id, if self.en { Sense::click_and_drag() } else { Sense::hover() });
        if self.en {
            if let Some(p) = resp.interact_pointer_pos() {
                let n = ((p.x - track.min.x - 2.5) / track.width()).clamp(0.0, 1.0);
                if n != *t {
                    *t = n;
                    resp.mark_changed();
                    self.changed = true;
                }
            }
        }
        kit::paint_slider(self.ui.painter(), track, track.min.x + t.clamp(0.0, 1.0) * track.width(), self.en, resp.hovered() || resp.dragged());
        self.note("slider", "", r);
        self.hover(resp)
    }

    /// A slider with `max + 1` positions.
    pub fn steps(&mut self, x: f32, y: f32, w: f32, v: &mut u32, max: u32) -> bool {
        let mut t = (*v).min(max) as f32 / max.max(1) as f32;
        let resp = self.slider(x, y, w, &mut t);
        let n = (t * max as f32).round() as u32;
        let changed = resp.changed() && n != *v;
        if changed {
            *v = n;
        }
        changed
    }

    /// A push button.
    pub fn button(&mut self, x: f32, y: f32, w: f32, h: f32, s: &str) -> Response {
        let r = self.rect(x, y, w, h);
        self.note("button", s, r);
        let en = self.en;
        let mut child = self.ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let resp = child.add_enabled(en, kit::btn(s).exact(vec2(w, h)).font_size(if h < 26.0 { 12.0 } else { 13.0 }));
        self.hover(resp)
    }

    /// A value box (a dropdown's black field without the arrow): paths and names that are shown
    /// but not chosen from a list.
    pub fn field(&mut self, x: f32, y: f32, w: f32, s: &str) -> Response {
        let r = self.rect(x, y, w, 22.0);
        self.ui.painter().rect_filled(r, 3.0, Color32::BLACK);
        let c = if self.en { WHITE } else { hex(0x878787) };
        let g = pf::galley(self.ui, s, 16.0, true, c);
        self.ui.painter().with_clip_rect(r.shrink2(vec2(4.0, 0.0))).galley(pos2(r.min.x + 6.0, r.center().y - 8.0), g, c);
        let id = self.id();
        let resp = self.ui.interact(r, id, Sense::hover());
        self.hover(resp)
    }

    /// A rotary control with bounds (x, y, 60 x 50), as rekordbox's CROSSFADER Curve and
    /// Vinyl Speed Adjust knobs: a ring open at the bottom, blue from the start to the value,
    /// with a white mark at the value. `t` is 0..=1; drag up or down to turn it.
    pub fn knob(&mut self, x: f32, y: f32, t: &mut f32) -> Response {
        let r = self.rect(x, y, 60.0, 50.0);
        let id = self.id();
        let mut resp = self.ui.interact(r, id, if self.en { Sense::click_and_drag() } else { Sense::hover() });
        if self.en && resp.dragged() {
            let d = -resp.drag_delta().y / 150.0;
            if d != 0.0 {
                *t = (*t + d).clamp(0.0, 1.0);
                resp.mark_changed();
                self.changed = true;
            }
        }
        if self.en && resp.double_clicked() {
            *t = 0.5;
            resp.mark_changed();
            self.changed = true;
        }
        paint_knob(self.ui.painter(), r, *t, self.en);
        self.note("knob", "", r);
        self.hover(resp)
    }

    /// The painter and the item's origin, for the few things drawn by hand.
    pub fn painter(&self) -> (&egui::Painter, Pos2) {
        (self.ui.painter(), self.o)
    }

    /// Interaction with a hand-drawn part at (x, y, w x h).
    pub fn sense(&mut self, x: f32, y: f32, w: f32, h: f32, kind: &'static str, caption: &str) -> Response {
        let r = self.rect(x, y, w, h);
        let id = self.id();
        self.note(kind, caption, r);
        let resp = self.ui.interact(r, id, if self.en { Sense::click() } else { Sense::hover() });
        self.hover(resp)
    }
}

/// The rotary control: JUCE's arc slider (track from 1.2 pi to 2.8 pi) in rekordbox's colours.
pub fn paint_knob(p: &egui::Painter, r: Rect, t: f32, enabled: bool) {
    // (Measured on Controller > Mixer: centre 30, 25 in the 60 x 50 box, outer radius 23,
    // ring 4.5 pt, the track #323232, the value arc #1473EB, a 1.5 pt white mark 9 pt long.)
    let c = pos2(r.min.x + 30.0, r.min.y + 25.0);
    let (rad, wd) = (20.25_f32, 5.5_f32);
    let (a0, a1) = (1.2 * std::f32::consts::PI, 2.8 * std::f32::consts::PI);
    let pt = |a: f32, rr: f32| pos2(c.x + rr * a.sin(), c.y - rr * a.cos());
    let arc = |from: f32, to: f32| -> Vec<Pos2> { (0..=40).map(|i| pt(from + (to - from) * i as f32 / 40.0, rad)).collect() };
    let fade = |col: Color32| if enabled { col } else { col.gamma_multiply(0.35) };
    p.add(egui::Shape::line(arc(a0, a1), egui::Stroke::new(wd, fade(SEP))));
    let av = a0 + (a1 - a0) * t.clamp(0.0, 1.0);
    if t > 0.0 {
        p.add(egui::Shape::line(arc(a0, av), egui::Stroke::new(wd, fade(PBLUE))));
    }
    p.line_segment([pt(av, rad - wd / 2.0 - 2.0), pt(av, rad + wd / 2.0 + 2.5)], egui::Stroke::new(1.5, fade(WHITE)));
    let _ = GREY;
}

/// Size of a galley-less caption: the width `s` takes as a toggle caption.
#[allow(dead_code)]
pub fn caption_w(ui: &Ui, s: &str) -> f32 {
    pf::width(ui, s, CTL, false)
}

#[allow(dead_code)]
pub fn origin_of(ui: &Ui) -> Vec2 {
    ui.cursor().min.to_vec2()
}
