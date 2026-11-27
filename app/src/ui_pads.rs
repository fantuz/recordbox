//! The pad area of a Performance deck: rekordbox's `MemoryCuePanel` with its pad views and the
//! pad mode pull-down (`checks/re-7.2.19/pads-sampler-automix.md`, section A).
//!
//! Geometry is from rekordbox 7.2.19's layout code for 2Deck / 4Deck Horizontal and from the
//! references `performance/01` and `03`:
//! - `MemoryCuePanel::resized` (0x10067fdd8): the pad area is the panel less 78 pt for the
//!   loop box, 87 pt tall (77 with four decks); the pull-down is 70 x 20, 3 pt under it, at
//!   the end away from the loop box; a mode's own controls are 18 pt tall in the same row at
//!   the other end.
//! - LIST Display (the default without a controller): pads in columns of four, two columns
//!   showing, the rest reached with a 5 pt scroll bar under them (`MemoryCueScrollBarLook
//!   AndFeel`: a 2 pt #323232 track with a 3 pt #555555 thumb). `HotCueTab::resized`
//!   (0x10068bd60), `BeatJumpView::resized` (0x101dd9d4c), `BeatLoopView::resized`
//!   (0x101240338), `KeyShiftView::resized` (0x101d6289c), `SlicerView::resized` (0x100743934),
//!   `ActiveCensorView::resized` (0x101083c80).
//! - PAD Display: 2 x 4 pads (2 x 2 cells for BEAT JUMP) and a page control.
//! - Colours are rekordbox's dark theme: pads #0F0F0F (id 0x30000A6), their text #FFFFFF
//!   (0x30000A4) or #323232 when off (0x30000A5), a lit pad's outline #1473EB (0x30000A7).
//!   Pad labels are Arial 12 (14 in PAD Display).

use crate::app::*;
use crate::model::*;
use crate::pads::*;
use crate::theme::*;
use crate::ui_player::press_fired;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, Ui};

/// Pad face: 0x030000A6 (`HotCuePad::paint`, `BeatJumpPad`) D #0F0F0F L #FFFFFF.
const PAD_BG: Tc = tc(0x0F0F0F, 0xFFFFFF);
/// Pad text: 0x030000A4 (`HotCueComp::lookAndFeelChanged`) D #FFFFFF L #000000.
const PAD_TEXT: Tc = tc(0xFFFFFF, 0x000000);
const PAD_OFF: Tc = gr(0x323232);
const PAD_LIT: Color32 = hex(0x1473EB);
const BAR_TRACK: Tc = gr(0x323232);
const BAR_THUMB: Tc = gr(0x555555);
const PULL: Tc = gr(0x282828);
const GREY: Tc = gr(0x969696);
/// The scroll bar's strip (`getDefaultScrollbarWidth`).
const BAR: f32 = 5.0;

/// Where things are in one deck's pad panel.
#[derive(Clone, Copy)]
struct Geom {
    /// The pads' viewport.
    area: Rect,
    /// The pull-down.
    pull: Rect,
    /// Top of the 18 pt control row.
    ctrl_y: f32,
    /// A left-hand deck (1 or 3): pull-down at the left, controls at the right.
    left: bool,
    c4: bool,
    /// 2Deck Vertical: the LIST views run down one column (`v2_list`).
    v2: bool,
}

impl Geom {
    fn new(r: Rect, n: u8, c4: bool) -> Geom {
        let left = n % 2 == 0;
        // 2Deck Vertical (`MemoryCuePanel::resized`, layout 274): the views get W - 78 x
        // H - 43 from y 10 (160 pt) and the pull-down sits at H - 30 (173 pt into the deck's
        // lower part; reference 02).
        if crate::ui_perf_layouts::dk(c4) == crate::ui_perf_layouts::Dk::V2 {
            // (W - 78 is 269 pt at the reference size: 264 pt pads, then the 5 pt scroll
            // strip; the pads rect handed in for deck 2 is 5 pt narrower than that.)
            let area = Rect::from_min_size(r.min, vec2(r.width().floor().max(264.0 + BAR), 160.0));
            let pull = Rect::from_min_size(pos2(if left { area.min.x - 2.0 } else { area.max.x + 2.0 - 70.0 }, area.min.y + 163.0), vec2(70.0, 20.0));
            return Geom { area, pull, ctrl_y: pull.min.y + 2.0, left, c4, v2: true };
        }
        let ah = if c4 { 77.0 } else { 87.0 };
        let area = Rect::from_min_size(r.min, vec2(r.width().floor(), ah));
        let pull = Rect::from_min_size(pos2(if left { area.min.x - 2.0 } else { area.max.x + 2.0 - 70.0 }, area.max.y + 3.0), vec2(70.0, 20.0));
        Geom { area, pull, ctrl_y: area.max.y + 5.0, left, c4, v2: false }
    }
    /// A control strip `w` wide at the end opposite the pull-down.
    fn ctrl(&self, w: f32) -> Rect {
        let x = if self.left { self.area.max.x - w } else { self.area.min.x };
        Rect::from_min_size(pos2(x, self.ctrl_y), vec2(w, 18.0))
    }
    /// A control `w` wide `off` pt after the pull-down's outer edge (KEYBOARD's two).
    fn beside_pull(&self, off: f32, w: f32) -> Rect {
        let x = if self.left { self.area.min.x + off } else { self.area.max.x - off - w };
        Rect::from_min_size(pos2(x, self.ctrl_y), vec2(w, 18.0))
    }
}

/// Arial at `size` pt (the face of rekordbox's pad labels). The face is added to the fonts the
/// first time it is asked for and is there from the next frame on; until then, and where it
/// is not installed, the interface's own face stands in.
fn arial(ui: &Ui, size: f32) -> FontId {
    // (JUCE gives a font's height; Arial's point size is 0.87 of it.)
    let size = size * 0.87;
    static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let fam = egui::FontFamily::Name("arial".into());
    if ui.ctx().fonts(|f| f.families().contains(&fam)) {
        return FontId::new(size, fam);
    }
    if !ASKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        if let Ok(b) = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf") {
            use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
            ui.ctx().add_font(FontInsert::new("arial-regular", egui::FontData::from_owned(b), vec![InsertFontFamily { family: fam, priority: FontPriority::Highest }]));
        }
    }
    FontId::proportional(size)
}

/// How a pad looks: rekordbox's `CustomTextButton` with a fill, a text colour and an outline
/// that shows while it is lit. Hover adds white at 10 %, a press draws at 70 %.
#[derive(Clone, Copy)]
struct Look {
    fill: Color32,
    text: Color32,
    outline: Option<Color32>,
    radius: f32,
}

impl Look {
    fn pad(enabled: bool, lit: bool) -> Look {
        Look { fill: PAD_BG.c(), text: if enabled { PAD_TEXT.c() } else { PAD_OFF.c() }, outline: if lit { Some(PAD_LIT) } else { None }, radius: 0.0 }
    }
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Draw a pad's body in `r` (clipped by the painter) and return its response. `clip`: the
/// part of the panel the pad may be pressed in.
fn pad_body(ui: &mut Ui, p: &Painter, r: Rect, clip: Rect, id: egui::Id, look: Look, enabled: bool) -> egui::Response {
    let hit = r.intersect(clip);
    let resp = ui.interact(if hit.is_positive() { hit } else { Rect::NOTHING }, id, Sense::click());
    let down = resp.is_pointer_button_down_on() && enabled;
    let mut fill = look.fill;
    if resp.hovered() && enabled {
        fill = blend(fill, Color32::WHITE, 0.1);
    }
    if down {
        fill = blend(BG2.c(), fill, 0.7);
    }
    p.rect_filled(r, look.radius, fill);
    if let Some(c) = look.outline {
        p.rect_stroke(r, look.radius, Stroke::new(1.0_f32, c), egui::StrokeKind::Inside);
    }
    resp
}

fn pad_text(p: &Painter, r: Rect, text: &str, font: FontId, col: Color32) {
    if text.contains('\n') {
        let mut y = r.center().y - font.size * 0.55 * (text.lines().count() as f32 - 1.0);
        for line in text.lines() {
            p.text(pos2(r.center().x, y), Align2::CENTER_CENTER, line, font.clone(), col);
            y += font.size * 1.1;
        }
    } else {
        p.text(r.center(), Align2::CENTER_CENTER, text, font, col);
    }
}

/// A scrolling list of pad columns (LIST Display) or rows (PAD Display).
struct Scroll {
    /// Where pads show (the area without the scroll bar's strip).
    view: Rect,
    /// Offset of the content in points along the scrolling axis.
    off: f32,
    vertical: bool,
}

impl Scroll {
    fn origin(&self) -> Pos2 {
        if self.vertical {
            pos2(self.view.min.x, self.view.min.y - self.off)
        } else {
            pos2(self.view.min.x - self.off, self.view.min.y)
        }
    }
}

/// The viewport of a pad list with its scroll bar: `content` points long along the axis, of
/// which the viewport shows its own length. The position is kept per deck and mode.
fn scroll_area(app: &mut App, ui: &mut Ui, area: Rect, n: u8, mode: PadMode, content: f32, vertical: bool) -> Scroll {
    let p = ui.painter().clone();
    let view = if vertical { Rect::from_min_max(area.min, pos2(area.max.x - BAR, area.max.y)) } else { Rect::from_min_max(area.min, pos2(area.max.x, area.max.y - BAR)) };
    let len = if vertical { view.height() } else { view.width() };
    let over = (content - len).max(0.0);
    let slot = &mut app.pads.deck[(n as usize).min(3)].scroll[(mode.value() as usize).min(20)];
    let mut frac = slot.unwrap_or(0.0);
    if over <= 0.5 {
        *slot = Some(0.0);
        return Scroll { view, off: 0.0, vertical };
    }
    // The strip: the track is its outer 2 pt, the thumb its outer 3 pt.
    let strip = if vertical { Rect::from_min_max(pos2(area.max.x - BAR, area.min.y), area.max) } else { Rect::from_min_max(pos2(area.min.x, area.max.y - BAR), area.max) };
    let full = if vertical { strip.height() } else { strip.width() };
    let thumb = (full * len / content).max(12.0);
    let resp = ui.interact(strip.expand(2.0), ui.id().with(("padscroll", mode.value())), Sense::click_and_drag());
    if resp.dragged() {
        let d = if vertical { resp.drag_delta().y } else { resp.drag_delta().x };
        frac += d / (full - thumb).max(1.0);
    } else if resp.clicked() {
        if let Some(pp) = resp.interact_pointer_pos() {
            let at = if vertical { pp.y - strip.min.y } else { pp.x - strip.min.x };
            let t0 = frac * (full - thumb);
            // (A click beside the thumb moves one viewport, as a scroll bar does.)
            if at < t0 {
                frac -= len / over;
            } else if at > t0 + thumb {
                frac += len / over;
            }
        }
    }
    if ui.rect_contains_pointer(area) {
        let d = ui.input(|i| i.smooth_scroll_delta);
        let d = if d.x.abs() > d.y.abs() { d.x } else { d.y };
        if d != 0.0 {
            frac -= d / over;
        }
    }
    frac = frac.clamp(0.0, 1.0);
    app.pads.deck[(n as usize).min(3)].scroll[(mode.value() as usize).min(20)] = Some(frac);
    let t0 = frac * (full - thumb);
    if vertical {
        p.rect_filled(Rect::from_min_max(pos2(strip.max.x - 2.0, strip.min.y), strip.max), 0.0, BAR_TRACK);
        p.rect_filled(Rect::from_min_size(pos2(strip.max.x - 3.0, strip.min.y + t0), vec2(3.0, thumb)), 0.0, BAR_THUMB);
    } else {
        p.rect_filled(Rect::from_min_max(pos2(strip.min.x, strip.max.y - 2.0), strip.max), 0.0, BAR_TRACK);
        p.rect_filled(Rect::from_min_size(pos2(strip.min.x + t0, strip.max.y - 3.0), vec2(thumb, 3.0)), 0.0, BAR_THUMB);
    }
    // (Whole points, so that pads stay sharp.)
    Scroll { view, off: (frac * over).round(), vertical }
}

/// LIST Display in 2Deck Vertical: `count` items in one column of 19 pt pads on a 21 pt
/// pitch, the scroll bar at the right (reference 02: HOT CUE A..H, STEMS' eight rows).
/// Returns the scroll, the pad width and a cell's rectangle by index.
fn v2_list(app: &mut App, ui: &mut Ui, g: Geom, n: u8, mode: PadMode, count: usize) -> (Scroll, f32, f32) {
    let sc = scroll_area(app, ui, g.area, n, mode, count as f32 * 21.0 - 2.0, true);
    let cw = sc.view.width().floor();
    (sc, cw, 19.0)
}

fn v2_cell(sc: &Scroll, cw: f32, i: usize) -> Rect {
    Rect::from_min_size(sc.origin() + vec2(0.0, i as f32 * 21.0), vec2(cw, 19.0))
}

/// The pull-down's rectangle and the pads' viewport of deck `n` (for the scripted run).
pub fn layout_of(r: Rect, n: u8, c4: bool) -> (Rect, Rect) {
    let g = Geom::new(r, n, c4);
    (g.area, g.pull)
}

/// Hook at the top of `ui_perf::pads`: draws the whole pad area for the modes this file owns
/// and returns true. For the modes drawn there (STEMS, SAMPLER, SEQ. CALL, PAD FX) it returns
/// false, and `pull_down` is called after their pads.
pub fn host(app: &mut App, ui: &mut Ui, r: Rect, n: u8, c4: bool) -> bool {
    app.pads.rects[(n as usize).min(3)] = r;
    app.pads.c4 = c4;
    app.pads_tick();
    let mode = app.pad_mode();
    slicer_overlay(app, ui);
    censor_overlay(app, ui);
    if !mode.own_view() {
        return false;
    }
    let g = Geom::new(r, n, c4);
    // (The panel behind the pads is #1E1E1E, as in the references.)
    ui.painter().rect_filled(Rect::from_min_max(g.area.min, pos2(g.area.max.x, g.pull.max.y)).expand2(vec2(2.0, 1.0)), 0.0, BG2);
    let pad_layout = app.pads_pad_layout();
    match mode {
        PadMode::HotCue => hot_cue_view(app, ui, g, n, pad_layout, false),
        PadMode::BeatJump => beat_jump_view(app, ui, g, n, pad_layout),
        PadMode::BeatLoop => beat_loop_view(app, ui, g, n, pad_layout),
        PadMode::KeyShift => key_view(app, ui, g, n, pad_layout, false),
        PadMode::Keyboard => key_view(app, ui, g, n, pad_layout, true),
        PadMode::Slicer => slicer_view(app, ui, g, n, pad_layout),
        PadMode::ActiveCensor => censor_view(app, ui, g, n, pad_layout),
        PadMode::User(u) => user_view(app, ui, g, n, u as usize),
        PadMode::MemoryCue => memory_view(app, ui, g, n),
        // --- PAD FX (the FX track): its pad view is drawn by `ui_perf::pads` for
        // `pads::PAD_FX_CODE`; see `PadMode::own_view`. Nothing to do here.
        PadMode::PadFx | PadMode::Stems | PadMode::Sampler | PadMode::SeqCall | PadMode::SamplerScratch => {}
    }
    pull_down_at(app, ui, g, n);
    true
}

/// Hook in `ui_perf::pads` after the pads it draws itself: the pull-down, and the page
/// control of the SAMPLER pads. Returns true (the caller then draws no pull-down of its own).
pub fn pull_down(app: &mut App, ui: &mut Ui, r: Rect, n: u8, c4: bool) -> bool {
    let g = Geom::new(r, n, c4);
    if app.pad_mode() == PadMode::Sampler {
        // Pads 1-8 or 9-16.
        let page = app.deck.x.pad_page.min(1);
        if let Some(up) = page_buttons(app, ui, g.ctrl(50.0), "smppage", page > 0, page < 1, "Page") {
            app.deck.x.pad_page = if up { 1 } else { 0 };
        }
    }
    pull_down_at(app, ui, g, n);
    true
}

/// The pad mode pull-down: rekordbox's eleven modes in its order, the current one ticked,
/// then the USER modes the Pad Editor lists.
fn pull_down_at(app: &mut App, ui: &mut Ui, g: Geom, n: u8) {
    let p = ui.painter().clone();
    let mode = app.pad_mode();
    let resp = ui.interact(g.pull, ui.id().with("padmode"), Sense::click());
    p.rect_filled(g.pull, 2.0, if resp.hovered() { blend(PULL.c(), Color32::WHITE, 0.1) } else { PULL.c() });
    // (rekordbox fits the caption into the button: a long one is drawn narrower, down to 0.7.)
    let avail = g.pull.width() - 7.5 - 17.5;
    let label = mode.label();
    let wide = p.layout_no_wrap(label.clone(), caps(8.7), WHITE.c()).size().x;
    let size = 8.7 * (avail / wide.max(1.0)).clamp(0.7, 1.0);
    p.text(pos2(g.pull.min.x + 7.5, g.pull.center().y), Align2::LEFT_CENTER, label, caps(size), WHITE.c());
    // (`pullDown.svg`: an 8 x 4 chevron, 1 pt, in an 11 x 10 box at the right.)
    crate::icons_player::chevron_down(&p, pos2(g.pull.max.x - 8.5, g.pull.center().y + 0.5), 1.35, WHITE.c());
    let resp = crate::ui_player::tip(resp, app, "Displays the PAD mode.");
    let ctx = app.pad_select_ctx();
    let stems_on = app.prefs.show_stems;
    // (The lower decks of the 4-deck layouts open the menu upwards, over their pads: below
    // them there is only the browser. JUCE's `PopupMenu` puts it where it fits.)
    let up = g.c4 && n >= 2;
    let pid = ui.id().with(("dropdown", "padmodepop"));
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(pid));
    }
    let side = if up { egui::AboveOrBelow::Above } else { egui::AboveOrBelow::Below };
    egui::popup::popup_above_or_below_widget(ui, pid, &resp, side, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(160.0);
        let mut pick = None;
        for m in menu_for(ctx) {
            let (enabled, why) = match m {
                PadMode::Stems if !stems_on => (false, "Enable the STEMS Function in Preferences > Extensions > STEMS."),
                PadMode::PadFx if !PAD_FX_READY => (false, "PAD FX is not built in recordbox yet."),
                _ => (true, ""),
            };
            let label = if mode == m { format!("✔ {}", m.label()) } else { format!("    {}", m.label()) };
            if ui.add_enabled(enabled, egui::SelectableLabel::new(mode == m, label)).on_disabled_hover_text(why).clicked() {
                pick = Some(m);
            }
        }
        let users: Vec<u8> = (0..8u8).filter(|u| app.prefs.pads.user_shown[*u as usize]).collect();
        if !users.is_empty() {
            ui.separator();
            for u in users {
                let m = PadMode::User(u);
                let label = if mode == m { format!("✔ {}", m.label()) } else { format!("    {}", m.label()) };
                if ui.selectable_label(mode == m, label).clicked() {
                    pick = Some(m);
                }
            }
        }
        // (In rekordbox SAMPLER is a pad mode of the controllers only; recordbox has no
        // controller pads, so it is offered here, and the Pad Editor with it: rekordbox
        // opens that from the PAD button of its top bar.)
        ui.separator();
        let label = if mode == PadMode::Sampler { "✔ SAMPLER" } else { "    SAMPLER" };
        if ui.selectable_label(mode == PadMode::Sampler, label).clicked() {
            pick = Some(PadMode::Sampler);
        }
        if ui.selectable_label(false, "    PAD EDITOR...").on_hover_text("Open/Close the PAD EDITOR window.").clicked() {
            app.pads.editor_open = !app.pads.editor_open;
            ui.memory_mut(|m| m.close_popup());
        }
        if let Some(m) = pick {
            app.set_pad_mode(m);
            ui.memory_mut(|m| m.close_popup());
        }
    });
    let _ = n;
    pad_editor(app, ui);
}

/// Two 24 x 18 buttons side by side (`PageSelector`): previous and next. Returns Some(up).
fn page_buttons(app: &App, ui: &mut Ui, r: Rect, id: &str, can_down: bool, can_up: bool, tip: &str) -> Option<bool> {
    let p = ui.painter().clone();
    let mut out = None;
    for (i, (en, up)) in [(can_down, false), (can_up, true)].into_iter().enumerate() {
        let br = Rect::from_min_size(pos2(r.min.x + i as f32 * 26.0, r.min.y), vec2(24.0, 18.0));
        let resp = ui.interact(br, ui.id().with((id, i)), Sense::click());
        p.rect_filled(br, 0.0, if resp.hovered() && en { blend(PAD_BG.c(), Color32::WHITE, 0.1) } else { PAD_BG.c() });
        // (`keyUpBtn.svg` / `keyDownBtn.svg`: a 6 x 10 chevron in #646464.)
        let col = if en { gy(0x646464) } else { gy(0x323232) };
        if up {
            crate::icons_player::chevron_right(&p, br.center(), 1.1, col);
        } else {
            crate::icons_player::chevron_left(&p, br.center(), 1.1, col);
        }
        if crate::ui_player::tip(resp, app, tip).clicked() && en {
            out = Some(up);
        }
    }
    out
}

/// A filled triangle pointing left or right in an 8 x 9 box (`beatjump_left_a.svg`, the shift
/// buttons).
fn triangle(p: &Painter, c: Pos2, w: f32, h: f32, left: bool, col: Color32) {
    let s = if left { -1.0 } else { 1.0 };
    p.add(egui::Shape::convex_polygon(vec![c + vec2(s * w / 2.0, 0.0), c + vec2(-s * w / 2.0, -h / 2.0), c + vec2(-s * w / 2.0, h / 2.0)], col, Stroke::NONE));
}

// ------------------------------------------------------------------------------- HOT CUE

/// HOT CUE: sixteen pads A-P. LIST Display: four columns of four, A-H showing. `pick`: the
/// KEYBOARD edit view, where a pad chooses the key cue.
fn hot_cue_view(app: &mut App, ui: &mut Ui, g: Geom, n: u8, pad_layout: bool, pick: bool) {
    let loaded = app.deck.track.is_some();
    let (w, h) = (g.area.width(), g.area.height());
    // (`HotCueTab` is twice the viewport plus one along the scrolling axis.)
    let (cw, ch, sc) = if pad_layout {
        let ch = ((2.0 * h + 1.0 - 6.0) / 4.0).floor();
        let sc = scroll_area(app, ui, g.area, n, PadMode::HotCue, 4.0 * (ch + 2.0) - 2.0, true);
        (((sc.view.width() - 6.0) / 4.0).floor(), ch, sc)
    } else if g.v2 {
        let (sc, cw, ch) = v2_list(app, ui, g, n, PadMode::HotCue, 16);
        (cw, ch, sc)
    } else {
        let cw = ((2.0 * w + 1.0 - 6.0) / 4.0).floor();
        let sc = scroll_area(app, ui, g.area, n, PadMode::HotCue, 4.0 * (cw + 2.0) - 2.0, false);
        (cw, ((sc.view.height() - 6.0) / 4.0).floor(), sc)
    };
    let p = ui.painter().with_clip_rect(sc.view);
    let o = sc.origin();
    let font = arial(ui, if pad_layout { 15.0 } else { 12.0 });
    let key_cue = app.pads.deck[(n as usize).min(3)].key_cue;
    for slot in 0..16usize {
        let (col, row) = if pad_layout { (slot % 4, slot / 4) } else if g.v2 { (0, slot) } else { (slot / 4, slot % 4) };
        let pr = if g.v2 && !pad_layout { v2_cell(&sc, cw, slot) } else { Rect::from_min_size(o + vec2(col as f32 * (cw + 2.0), row as f32 * (ch + 2.0)), vec2(cw, ch)) };
        if !pr.intersects(sc.view) {
            continue;
        }
        let cue = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32).cloned();
        let letter = ((b'A' + slot as u8) as char).to_string();
        let lit = pick && key_cue == Some(slot as i32) && cue.is_some();
        let look = Look { fill: PAD_BG.c(), text: PAD_TEXT.c(), outline: if lit { Some(PAD_LIT) } else { None }, radius: 2.0 };
        let resp = pad_body(ui, &p, pr, sc.view, ui.id().with(("hotpad", slot)), look, loaded);
        let cy = pr.center().y;
        let mut over_delete = false;
        match &cue {
            Some(c) if pad_layout => {
                // A 15 pt chip in the middle of the pad.
                let chip = Rect::from_center_size(pr.center(), vec2(15.0, 15.0));
                p.rect_filled(chip, 0.0, hotcue_color(c.color));
                p.text(chip.center(), Align2::CENTER_CENTER, &letter, font.clone(), Color32::BLACK);
            }
            Some(c) => {
                // The 11 pt chip with the letter, the time, the comment, a loop's mark and the
                // delete cross (`HotCueComp::resized` 0x100688fe8).
                let chip = Rect::from_min_size(pos2(pr.min.x + 10.0, (cy - 5.5).round()), vec2(11.0, 11.0));
                p.rect_filled(chip, 0.0, hotcue_color(c.color));
                p.text(chip.center(), Align2::CENTER_CENTER, &letter, arial(ui, 10.5), Color32::BLACK);
                let tp = p.with_clip_rect(Rect::from_min_max(pr.min, pos2(pr.max.x - 26.0, pr.max.y)).intersect(sc.view));
                let tr = tp.text(pos2(pr.min.x + 31.0, cy), Align2::LEFT_CENTER, fmt_time(c.start, false), FontId::proportional(10.7), gy(0xF4F4F4));
                let mut x = tr.max.x.max(pr.min.x + 60.0) + 7.0;
                if !c.comment.is_empty() {
                    let cr = tp.text(pos2(x, cy), Align2::LEFT_CENTER, &c.comment, FontId::proportional(10.7), gy(0xF4F4F4));
                    x = cr.max.x + 4.0;
                }
                if c.end.is_some() {
                    // (`cueLoopIcon.svg`, 11 x 6.6, #FF8205.)
                    tp.rect_stroke(Rect::from_min_size(pos2(x, cy - 3.3), vec2(11.0, 6.6)), 3.0, Stroke::new(1.4_f32, LOOP), egui::StrokeKind::Inside);
                }
                if !pick {
                    let dr = Rect::from_min_size(pos2(pr.max.x - 24.0, cy - 7.0), vec2(14.0, 14.0));
                    let dresp = ui.interact(dr.intersect(sc.view), ui.id().with(("hotdel", slot)), Sense::click());
                    over_delete = dresp.hovered() || dresp.is_pointer_button_down_on();
                    crate::icons_player::cross(&p, dr.center(), 8.0, 1.0, if dresp.hovered() { WHITE.c() } else { GREY.c() });
                    if dresp.on_hover_text("Clear a Hot Cue.").clicked() {
                        app.delete_cue(c.id);
                        continue;
                    }
                }
            }
            None => {
                // An empty pad: its letter in #646464 where the chip would be.
                let at = if pad_layout { pr.center() } else { pos2(pr.min.x + 15.5, cy) };
                p.text(at, Align2::CENTER_CENTER, &letter, font.clone(), if loaded { gy(0x646464) } else { PAD_OFF.c() });
            }
        }
        if press_fired(ui, &resp) && loaded && !over_delete {
            if pick {
                app.pad_key_cue(slot as i32);
            } else {
                app.hot_cue_down(slot as i32);
            }
        }
        if !pick {
            if let Some(c) = cue {
                resp.context_menu(|ui| crate::ui_player::hot_cue_menu(app, ui, &c));
            }
        }
    }
    // A pad held while paused plays until it is let go (and SLIP comes back).
    let held = app.deck.hot_preview.is_some() || app.player.state.lock().slip_hold;
    if held && !ui.input(|i| i.pointer.any_down()) && !crate::keycmd::holding(n) {
        app.hot_cue_up();
    }
}

// ------------------------------------------------------------------------------- BEAT JUMP

/// One BEAT JUMP cell (`BeatJumpPad::resized` 0x101ddbab8): back and forward buttons side by
/// side, 2 pt apart, each with an 8 x 9 arrow in its middle, and the length on a label over
/// the gap.
fn jump_cell(app: &mut App, ui: &mut Ui, p: &Painter, r: Rect, clip: Rect, n: u8, cell: usize, pad_layout: bool) {
    let loaded = app.deck.track.is_some();
    let looping = app.loop_active().is_some();
    let half = ((r.width() - 2.0) / 2.0).floor();
    let down_now = app.pads.deck[(n as usize).min(3)].jump_down;
    for dir in 0..2u8 {
        let id = (cell * 2) as u8 + dir;
        let br = Rect::from_min_size(pos2(r.min.x + dir as f32 * (half + 2.0), r.min.y), vec2(half, r.height()));
        let resp = pad_body(ui, p, br, clip, ui.id().with(("bjpad", id)), Look::pad(loaded, false), loaded);
        let held = resp.is_pointer_button_down_on() && loaded;
        // (`beatjump_*_a.svg` red, `_b` orange while a loop runs, `_on` blue while pressed.)
        let col = if !loaded {
            PAD_OFF.c()
        } else if held || down_now == Some(id) {
            hex(0x0178D7)
        } else if looping {
            hex(0xE0641B)
        } else {
            hex(0xE62828)
        };
        triangle(p, pos2(br.min.x + (r.width() / 4.0).floor(), br.center().y), 6.4, 7.2, dir == 0, col);
        if press_fired(ui, &resp) && loaded {
            app.pad_beat_jump(id);
            app.pads.deck[(n as usize).min(3)].jump_down = Some(id);
        }
    }
    if down_now.is_some() && !ui.input(|i| i.pointer.any_down()) {
        app.pads.deck[(n as usize).min(3)].jump_down = None;
    }
    let font = arial(ui, 12.0);
    let label = JUMP_LABELS[cell.min(11)];
    let tw = p.layout_no_wrap(label.to_string(), font.clone(), PAD_TEXT.c()).size().x + 2.0;
    let lh = if pad_layout { 16.0 } else { 10.0 };
    let lr = Rect::from_center_size(pos2((r.center().x).round(), r.center().y), vec2(tw + if pad_layout { 4.0 } else { 0.0 }, lh));
    p.rect_filled(lr, 0.0, PAD_BG);
    p.text(lr.center(), Align2::CENTER_CENTER, label, font, if loaded { PAD_TEXT.c() } else { PAD_OFF.c() });
}

fn beat_jump_view(app: &mut App, ui: &mut Ui, g: Geom, n: u8, pad_layout: bool) {
    let ni = (n as usize).min(3);
    if pad_layout {
        // Four cells of the page, 2 x 2, and the page buttons.
        let page = app.prefs.pads.beat_jump_page[ni].clamp(1, 9);
        let (cw, ch) = (((g.area.width() - 2.0) / 2.0).floor(), ((g.area.height() - 2.0) / 2.0).floor());
        let p = ui.painter().with_clip_rect(g.area);
        for (k, cell) in jump_page_cells(page).into_iter().enumerate() {
            if cell >= 12 {
                continue;
            }
            let r = Rect::from_min_size(g.area.min + vec2((k % 2) as f32 * (cw + 2.0), (k / 2) as f32 * (ch + 2.0)), vec2(cw, ch));
            jump_cell(app, ui, &p, r, g.area, n, cell, true);
        }
        let shift = ui.input(|i| i.modifiers.shift);
        if let Some(up) = page_buttons(app, ui, g.ctrl(50.0), "bjpage", page > 1, page < 9, &format!("PAGE {page}")) {
            app.prefs.pads.beat_jump_page[ni] = jump_page_step(page, up, shift);
            app.save_prefs();
        }
        return;
    }
    // LIST Display: twelve cells in three columns of four (one column in 2Deck Vertical).
    let (sc, cw, ch) = if g.v2 {
        v2_list(app, ui, g, n, PadMode::BeatJump, 12)
    } else {
        let cw = ((g.area.width() - 2.0) / 2.0).floor();
        let sc = scroll_area(app, ui, g.area, n, PadMode::BeatJump, 3.0 * (cw + 2.0) - 2.0, false);
        let ch = ((sc.view.height() - 6.0) / 4.0).floor();
        (sc, cw, ch)
    };
    let p = ui.painter().with_clip_rect(sc.view);
    let o = sc.origin();
    for cell in 0..12usize {
        let r = if g.v2 { v2_cell(&sc, cw, cell) } else { Rect::from_min_size(o + vec2((cell / 4) as f32 * (cw + 2.0), (cell % 4) as f32 * (ch + 2.0)), vec2(cw, ch)) };
        if r.intersects(sc.view) {
            jump_cell(app, ui, &p, r, sc.view, n, cell, false);
        }
    }
}

// ------------------------------------------------------------------------------- BEAT LOOP

fn beat_loop_view(app: &mut App, ui: &mut Ui, g: Geom, n: u8, pad_layout: bool) {
    let loaded = app.deck.track.is_some();
    let lit = app.beat_loop_lit();
    let (cw, ch, sc) = if pad_layout {
        let ch = ((g.area.height() - 2.0) / 2.0).floor();
        let sc = scroll_area(app, ui, g.area, n, PadMode::BeatLoop, 4.0 * (ch + 2.0) - 2.0, true);
        (((sc.view.width() - 6.0) / 4.0).floor(), ch, sc)
    } else if g.v2 {
        let (sc, cw, ch) = v2_list(app, ui, g, n, PadMode::BeatLoop, 16);
        (cw, ch, sc)
    } else {
        let cw = ((g.area.width() - 2.0) / 2.0).floor();
        let sc = scroll_area(app, ui, g.area, n, PadMode::BeatLoop, 4.0 * (cw + 2.0) - 2.0, false);
        (cw, ((sc.view.height() - 6.0) / 4.0).floor(), sc)
    };
    let p = ui.painter().with_clip_rect(sc.view);
    let o = sc.origin();
    let font = arial(ui, if pad_layout { 14.0 } else { 12.0 });
    for i in 0..16usize {
        let (col, row) = if pad_layout { (i % 4, i / 4) } else { (i / 4, i % 4) };
        let r = if g.v2 && !pad_layout { v2_cell(&sc, cw, i) } else { Rect::from_min_size(o + vec2(col as f32 * (cw + 2.0), row as f32 * (ch + 2.0)), vec2(cw, ch)) };
        if !r.intersects(sc.view) {
            continue;
        }
        let look = Look::pad(loaded, lit == Some(i));
        let resp = pad_body(ui, &p, r, sc.view, ui.id().with(("blpad", i)), look, loaded);
        pad_text(&p, r, LOOP_LABELS[i], font.clone(), look.text);
        if press_fired(ui, &resp) && loaded {
            app.pad_beat_loop(i);
        }
    }
}

// ------------------------------------------------------------------------------- KEYBOARD / KEY SHIFT

fn key_view(app: &mut App, ui: &mut Ui, g: Geom, n: u8, pad_layout: bool, keyboard: bool) {
    let ni = (n as usize).min(3);
    let loaded = app.deck.track.is_some();
    let mode = if keyboard { PadMode::Keyboard } else { PadMode::KeyShift };
    let edit = keyboard && app.pads.deck[ni].key_edit;
    if keyboard {
        keyboard_controls(app, ui, g, n);
    }
    if edit {
        // The edit view: the hot cue pads, to choose or set the key cue.
        hot_cue_view(app, ui, g, n, pad_layout, true);
        return;
    }
    let current = app.pad_current_key();
    let key_sync = app.deck.x.key_sync;
    let s1 = app.key_scale1();
    let look_of = |pos: usize| -> Look {
        let key = key_at_in(pos, s1);
        let lit = match key {
            KEY_SYNC => key_sync,
            k => key_semis(k).is_some_and(|s| s == current) && (-12..=12).contains(&k),
        };
        let mut look = Look::pad(loaded, lit);
        // (The 0 pad keeps a #4B4B4B outline while it is not the key in use.)
        if key == 0 && !lit {
            look.outline = Some(gy(0x4B4B4B));
        }
        look
    };
    let press = |app: &mut App, ui: &Ui, resp: &egui::Response, pos: usize| {
        if press_fired(ui, resp) && loaded {
            if keyboard {
                app.pad_keyboard_down(pos);
            } else {
                app.pad_key(pos);
            }
        }
    };
    if pad_layout {
        let page = if keyboard { app.prefs.pads.keyboard_page[ni] } else { app.prefs.pads.key_shift_page[ni] }.clamp(1, 5);
        let (cw, ch) = (((g.area.width() - 6.0) / 4.0).floor(), ((g.area.height() - 2.0) / 2.0).floor());
        let p = ui.painter().with_clip_rect(g.area);
        let font = arial(ui, 12.0);
        for k in 0..8usize {
            let pos = (page as usize - 1) * 8 + k;
            let r = Rect::from_min_size(g.area.min + vec2((k % 4) as f32 * (cw + 2.0), (k / 4) as f32 * (ch + 2.0)), vec2(cw, ch));
            if key_at_in(pos, s1) == KEY_NONE {
                p.rect_filled(r, 0.0, PAD_BG);
                continue;
            }
            let look = look_of(pos);
            let resp = pad_body(ui, &p, r, g.area, ui.id().with(("keypad", pos)), look, loaded);
            pad_text(&p, r, &key_label_in(pos, false, s1), font.clone(), look.text);
            press(app, ui, &resp, pos);
        }
        let tip = if keyboard { "Change the page of the KEYBOARD mode." } else { "Change the page of the KEY SHIFT mode." };
        if let Some(up) = page_buttons(app, ui, g.ctrl(50.0), "keypage", page > 1, page < 5, tip) {
            let np = if up { page + 1 } else { page - 1 }.clamp(1, 5);
            if keyboard {
                app.prefs.pads.keyboard_page[ni] = np;
            } else {
                app.prefs.pads.key_shift_page[ni] = np;
            }
            app.save_prefs();
        }
    } else {
        // LIST Display: the 32 listed keys in columns of four, from +12 down.
        let list = key_list_in(s1);
        let cols = list.len().div_ceil(4);
        let (sc, cw, ch) = if g.v2 {
            v2_list(app, ui, g, n, mode, list.len())
        } else {
            let cw = ((g.area.width() - 2.0) / 2.0).floor();
            let sc = scroll_area(app, ui, g.area, n, mode, cols as f32 * (cw + 2.0) - 2.0, false);
            let ch = ((sc.view.height() - 6.0) / 4.0).floor();
            (sc, cw, ch)
        };
        let p = ui.painter().with_clip_rect(sc.view);
        let o = sc.origin();
        let font = arial(ui, 12.0);
        for (li, pos) in list.iter().enumerate() {
            let r = if g.v2 { v2_cell(&sc, cw, li) } else { Rect::from_min_size(o + vec2((li / 4) as f32 * (cw + 2.0), (li % 4) as f32 * (ch + 2.0)), vec2(cw, ch)) };
            if !r.intersects(sc.view) {
                continue;
            }
            let look = look_of(*pos);
            let resp = pad_body(ui, &p, r, sc.view, ui.id().with(("keypad", *pos)), look, loaded);
            pad_text(&p, r, &key_label_in(*pos, true, s1), font.clone(), look.text);
            press(app, ui, &resp, *pos);
        }
    }
    if keyboard {
        let held = app.deck.hot_preview.is_some() || app.player.state.lock().slip_hold;
        if held && !ui.input(|i| i.pointer.any_down()) && !crate::keycmd::holding(n) {
            app.pad_keyboard_up();
        }
    }
}

/// KEYBOARD's two controls beside the pull-down (`KeyboardView::resized` 0x101463f90): the
/// edit button, 20 x 18 at 84 pt, and the key cue pull-down, 35 x 18, 10 pt after it.
fn keyboard_controls(app: &mut App, ui: &mut Ui, g: Geom, n: u8) {
    let ni = (n as usize).min(3);
    let p = ui.painter().clone();
    let er = g.beside_pull(84.0, 20.0);
    let edit = app.pads.deck[ni].key_edit;
    let resp = ui.interact(er, ui.id().with("kbedit"), Sense::click());
    // (`settingBtn.svg` / `settingOnBtn.svg`: a cog, white, or #007DE1 while editing.)
    let col = if edit { hex(0x007DE1) } else { WHITE.c() };
    p.circle_stroke(er.center(), 4.2, Stroke::new(2.2_f32, col));
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::TAU / 8.0;
        let d = vec2(a.cos(), a.sin());
        p.line_segment([er.center() + d * 4.6, er.center() + d * 7.0], Stroke::new(2.0_f32, col));
    }
    if crate::ui_player::tip(resp, app, "KEYBOARD Edit button:\nSet or select a Hot Cue to use in a KEYBOARD function.\nIf you click a Hot Cue already set, you can use it in a KEYBOARD function.").clicked() {
        app.pads.deck[ni].key_edit = !edit;
    }
    let kr = g.beside_pull(114.0, 35.0);
    let key_cue = app.pads.deck[ni].key_cue.filter(|s| app.deck.cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == *s));
    let resp = ui.interact(kr, ui.id().with("kbcue"), Sense::click());
    p.rect_filled(kr, 2.0, if resp.hovered() { blend(PULL.c(), Color32::WHITE, 0.1) } else { PULL.c() });
    let text = key_cue.map(|s| ((b'A' + s as u8) as char).to_string()).unwrap_or_else(|| "---".into());
    p.text(pos2(kr.min.x + 5.0, kr.center().y), Align2::LEFT_CENTER, text, caps(10.0), WHITE.c());
    crate::icons_player::chevron_down(&p, pos2(kr.max.x - 8.0, kr.center().y + 0.5), 0.8, WHITE.c());
    let resp = crate::ui_player::tip(resp, app, "Select a Hot Cue used in a KEYBOARD function.\nIf no Hot Cues are displayed, click a KEYBOARD Edit button to set a Hot Cue.");
    let slots: Vec<i32> = {
        let mut v: Vec<i32> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Hot && (0..16).contains(&c.slot)).map(|c| c.slot).collect();
        v.sort();
        v
    };
    crate::ui_browser::dropdown(ui, &resp, "kbcuepop", |ui| {
        ui.set_min_width(110.0);
        for s in &slots {
            if ui.selectable_label(key_cue == Some(*s), format!("HOT CUE {}", (b'A' + *s as u8) as char)).clicked() {
                app.pads.deck[ni].key_cue = Some(*s);
                ui.memory_mut(|m| m.close_popup());
            }
        }
        if slots.is_empty() {
            ui.add_enabled(false, egui::Label::new("---"));
        }
    });
}

// ------------------------------------------------------------------------------- SLICER

/// Eight pads in two columns of four, 2 pt apart (PAD Display: four by two), and the
/// controller strip: loop, shift back / forward, LENGTH, ROLL (`Controller::resized`
/// 0x100746224: 24, 16, 16, 61 and 61 pt wide; 5, 5, 5 and 1 pt apart).
fn slicer_view(app: &mut App, ui: &mut Ui, g: Geom, n: u8, pad_layout: bool) {
    let ni = (n as usize).min(3);
    let active = app.pads.deck[ni].slicer.active;
    let (now, _) = app.slicer_now();
    let p = ui.painter().with_clip_rect(g.area);
    let (cols, rows) = if pad_layout { (4usize, 2usize) } else if g.v2 { (1, 8) } else { (2, 4) };
    let _ = rows;
    let cw = ((g.area.width() - 2.0 * (cols as f32 - 1.0)) / cols as f32).floor();
    // (The view is the pad area and the 23 pt row under it: (h - 29) / 4, or (h - 25) / 2.)
    let ch = if pad_layout { ((g.area.height() + 23.0 - 25.0) / 2.0).floor() } else if g.v2 { 19.0 } else { ((g.area.height() + 23.0 - 29.0) / 4.0).floor() };
    let font = arial(ui, 12.0);
    let released = ui.input(|i| i.pointer.any_released()) || !ui.input(|i| i.pointer.any_down());
    for i in 0..8usize {
        let (col, row) = if pad_layout { (i % 4, i / 4) } else if g.v2 { (0, i) } else { (i / 4, i % 4) };
        let r = Rect::from_min_size(g.area.min + vec2(col as f32 * (cw + 2.0), row as f32 * (ch + 2.0)), vec2(cw, ch));
        let held = app.pads.deck[ni].slicer.held.contains(&(i as u8));
        let mut look = Look::pad(active, now == Some(i));
        if held {
            look.fill = blend(BG2.c(), PAD_BG.c(), 0.7);
        }
        let resp = pad_body(ui, &p, r, g.area, ui.id().with(("slpad", i)), look, active);
        pad_text(&p, r, &format!("{}", i + 1), font.clone(), look.text);
        if press_fired(ui, &resp) && active {
            app.slicer_pad(i as u8, true);
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("slpad_held", n)), i as u8));
        }
        resp.context_menu(|ui| {
            if ui.add_enabled(active, egui::Button::new(format!("Slicer Capture (Bank{})", app.prefs.sampler.slicer_bank.clamp(1, 4)))).on_disabled_hover_text(MSG_CAPTURE).clicked() {
                app.slicer_capture();
                ui.close_menu();
            }
        });
    }
    if released {
        if let Some(h) = ui.ctx().data_mut(|d| d.remove_temp::<u8>(egui::Id::new(("slpad_held", n)))) {
            app.slicer_pad(h, false);
        }
    }
    // The controller strip.
    let s = g.ctrl(194.0);
    let cp = ui.painter().clone();
    let at = |x: f32, w: f32| Rect::from_min_size(pos2(s.min.x + x, s.min.y), vec2(w, 18.0));
    let loop_mode = app.pads.deck[ni].slicer.loop_mode;
    let lr = at(0.0, 24.0);
    let resp = ui.interact(lr, ui.id().with("slloop"), Sense::click());
    // (`Loop_2DeckHorizontal.svg`: a loop arrow, #969696, #007DE1 when on.)
    let lc = if loop_mode { hex(0x007DE1) } else { GREY.c() };
    let body = Rect::from_center_size(lr.center() + vec2(1.0, 0.0), vec2(11.0, 7.0));
    cp.rect_stroke(body, 2.5, Stroke::new(1.8_f32, lc), egui::StrokeKind::Middle);
    cp.rect_filled(Rect::from_center_size(pos2(body.min.x, body.center().y + 0.5), vec2(4.0, 4.0)), 0.0, BG2);
    triangle(&cp, pos2(body.min.x, body.center().y - 0.5), 5.0, 6.0, false, lc);
    if crate::ui_player::tip(resp, app, "Slicer Loop:\nFix the position of Slicer for Loop playback.").clicked() {
        app.slicer_toggle_loop();
    }
    for (x, fwd, tip) in [(29.0, false, "Shift range of the Slicer backward."), (50.0, true, "Shift range of the Slicer forward.")] {
        let br = at(x, 16.0);
        let resp = ui.interact(br, ui.id().with(("slshift", fwd)), Sense::click());
        // (`ShiftL_2DeckHorizontal.svg`: a 6 x 8 triangle, #969696.)
        triangle(&cp, br.center(), 6.0, 8.0, !fwd, if resp.hovered() { WHITE.c() } else { GREY.c() });
        if crate::ui_player::tip(resp, app, tip).clicked() {
            app.slicer_shift(fwd);
        }
    }
    let li = (app.prefs.pads.slicer_length[ni] as usize).min(3);
    let ri = (app.prefs.pads.slicer_roll[ni] as usize).min(7);
    if let Some(up) = selector(app, ui, at(71.0, 61.0), "sllen", &format!("{}", SLICER_LENGTHS[li]), li > 0, li < 3, ["Shrink the Slicer range (beat length).", "Expand the Slicer range (beat length)."]) {
        app.slicer_set_length(if up { li + 1 } else { li.saturating_sub(1) });
    }
    let (lo, hi) = (slicer_clamp_roll(li, 0), slicer_clamp_roll(li, 7));
    if let Some(up) = selector(app, ui, at(133.0, 61.0), "slroll", &slicer_roll_label(ri), ri > lo, ri < hi, ["ROLL", "ROLL"]) {
        app.slicer_set_roll(if up { ri + 1 } else { ri.saturating_sub(1) });
    }
}

/// rekordbox's `SelectorComponent`: a #282828 box, 16 pt tall, with a 21 pt arrow button at
/// each end and the value (Arial 14, #969696) between them. Returns Some(up) on a press.
fn selector(app: &App, ui: &mut Ui, r: Rect, id: &str, value: &str, can_down: bool, can_up: bool, tips: [&str; 2]) -> Option<bool> {
    let p = ui.painter().clone();
    let r = Rect::from_min_size(pos2(r.min.x, r.min.y + 1.0), vec2(r.width(), 16.0));
    p.rect_filled(r, 0.0, gy(0x282828));
    p.text(r.center(), Align2::CENTER_CENTER, value, arial(ui, 12.0), GREY.c());
    let mut out = None;
    for (i, (en, up)) in [(can_down, false), (can_up, true)].into_iter().enumerate() {
        let br = Rect::from_min_size(pos2(if up { r.max.x - 21.0 } else { r.min.x }, r.min.y), vec2(21.0, 16.0));
        let resp = ui.interact(br, ui.id().with((id, i)), Sense::click());
        // (`LengthRoll_ArrowL.svg`: a 5 x 8 chevron, #969696.)
        let col = if !en { gy(0x424242) } else if resp.hovered() { WHITE.c() } else { GREY.c() };
        if up {
            crate::icons_player::chevron_right(&p, br.center(), 0.9, col);
        } else {
            crate::icons_player::chevron_left(&p, br.center(), 0.9, col);
        }
        if crate::ui_player::tip(resp, app, tips[i]).clicked() && en {
            out = Some(up);
        }
    }
    out
}

/// The slicer on the enlarged waveform: the domain's eight sections, the one sounding, and
/// the background position while a section is held.
fn slicer_overlay(app: &mut App, ui: &mut Ui) {
    let n = (app.px.active as usize).min(3);
    let s = &app.pads.deck[n].slicer;
    if !s.active {
        return;
    }
    let Some((r, pos, pps)) = app.deck.detail_map else { return };
    let p = ui.painter().with_clip_rect(r);
    let x_of = |t: f64| r.center().x + ((t - pos) * pps) as f32;
    let (now, rolling) = app.slicer_now();
    let s = &app.pads.deck[n].slicer;
    for k in 0..8usize {
        let (x0, x1) = (x_of(s.b[k]), x_of(s.b[k + 1]));
        if x1 < r.min.x || x0 > r.max.x {
            continue;
        }
        let sec = Rect::from_min_max(pos2(x0, r.min.y), pos2(x1, r.max.y));
        let lit = now == Some(k);
        p.rect_filled(sec, 0.0, if lit { Color32::from_rgba_unmultiplied(20, 115, 235, if rolling { 90 } else { 50 }) } else { Color32::from_rgba_unmultiplied(255, 255, 255, if k % 2 == 0 { 16 } else { 8 }) });
        p.line_segment([pos2(x0, r.min.y), pos2(x0, r.max.y)], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 120)));
        p.text(pos2(x0 + 4.0, r.min.y + 3.0), Align2::LEFT_TOP, format!("{}", k + 1), FontId::proportional(9.0), Color32::from_rgba_unmultiplied(255, 255, 255, 200));
    }
    let xe = x_of(s.b[8]);
    p.line_segment([pos2(xe, r.min.y), pos2(xe, r.max.y)], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 120)));
    // The background position (where the track would be).
    let bg = {
        let st = app.player.state.lock();
        let sr = st.audio.as_ref().map(|a| a.sr as f64).unwrap_or(44100.0);
        st.pads.slicer.bg.map(|b| b / sr)
    };
    if let Some(t) = bg {
        let x = x_of(t);
        p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.5_f32, hex(0xFFD80A)));
    }
}

// ------------------------------------------------------------------------------- ACT. CENSR

/// REV ROLL, TRANS, ECHO, V.BRAKE in the first column, IN, OUT, CLEAR, ON/OFF in the second
/// (`ActiveCensorPad::paintButton` 0x1010868d8), and the strip of four buttons: previous and
/// next in point (30 pt each), beat down and up (24 pt each).
fn censor_view(app: &mut App, ui: &mut Ui, g: Geom, n: u8, pad_layout: bool) {
    let ni = (n as usize).min(3);
    let loaded = app.deck.track.is_some();
    let (kind, on, pending) = {
        let c = &app.pads.deck[ni].censor;
        (c.kind as usize, c.on, c.pending_in.is_some())
    };
    let (inside, adjust) = app.censor_adjust_state();
    let p = ui.painter().with_clip_rect(g.area);
    let (cols, rows) = if pad_layout { (4usize, 2usize) } else if g.v2 { (1, 8) } else { (2, 4) };
    let _ = rows;
    let cw = ((g.area.width() - 2.0 * (cols as f32 - 1.0)) / cols as f32).floor();
    let ch = if pad_layout { ((g.area.height() + 23.0 - 25.0) / 2.0).floor() } else if g.v2 { 19.0 } else { ((g.area.height() + 23.0 - 29.0) / 4.0).floor() };
    let font = arial(ui, 12.0);
    let beat = app.censor_beat_text();
    for i in 0..8usize {
        let (col, row) = if pad_layout { (i % 4, i / 4) } else if g.v2 { (0, i) } else { (i / 4, i % 4) };
        let r = Rect::from_min_size(g.area.min + vec2(col as f32 * (cw + 2.0), row as f32 * (ch + 2.0)), vec2(cw, ch));
        let lit = match i {
            0..=3 => kind == i,
            4 => pending || adjust == 1,
            5 => adjust == 2,
            7 => on,
            _ => false,
        };
        let look = Look::pad(loaded, lit);
        let resp = pad_body(ui, &p, r, g.area, ui.id().with(("acpad", i)), look, loaded);
        let label = match i {
            0..=3 => CENSOR_FX[i],
            4 if inside => "IN ADJ",
            4 => "IN",
            5 if inside => "OUT ADJ",
            5 => "OUT",
            6 => "CLEAR",
            _ => "ON/OFF",
        };
        pad_text(&p, r, label, font.clone(), look.text);
        // TRANS and V.BRAKE show their beat at the lower right (28 x 10 at w - 33, h - 12).
        if (i == 1 || i == 3) && kind == i {
            p.text(pos2(r.max.x - 5.0, r.max.y - 7.0), Align2::RIGHT_CENTER, &beat, font.clone(), look.text);
        }
        if press_fired(ui, &resp) && loaded {
            match i {
                0..=3 => app.censor_set_kind(i as u8),
                4 => app.censor_in(),
                5 => {
                    app.censor_out();
                }
                6 => {
                    app.censor_clear();
                }
                _ => app.censor_onoff(),
            }
        }
    }
    let _ = inside;
    let s = g.ctrl(120.0);
    let cp = ui.painter().clone();
    let at = |x: f32, w: f32| Rect::from_min_size(pos2(s.min.x + x, s.min.y), vec2(w, 18.0));
    for (x, next) in [(0.0, false), (35.0, true)] {
        let br = at(x, 30.0);
        let resp = ui.interact(br, ui.id().with(("accall", next)), Sense::click());
        // (`p10651_shiftl.svg`: a 6 x 8 triangle in a 30 x 16 box, #969696.)
        triangle(&cp, br.center(), 6.0, 8.0, !next, if resp.hovered() { WHITE.c() } else { GREY.c() });
        if resp.clicked() {
            app.censor_call(next);
        }
    }
    for (x, right) in [(70.0, false), (95.0, true)] {
        let br = at(x, 24.0);
        let resp = ui.interact(br, ui.id().with(("acbeat", right)), Sense::click());
        let col = if resp.hovered() { WHITE.c() } else { GREY.c() };
        if right {
            crate::icons_player::chevron_right(&cp, br.center(), 0.9, col);
        } else {
            crate::icons_player::chevron_left(&cp, br.center(), 0.9, col);
        }
        if resp.clicked() {
            app.censor_beat(right);
        }
    }
}

/// The censored regions on the enlarged waveform, and an in point that waits for its OUT.
fn censor_overlay(app: &mut App, ui: &mut Ui) {
    let n = (app.px.active as usize).min(3);
    let c = &app.pads.deck[n].censor;
    if c.regions.is_empty() && c.pending_in.is_none() {
        return;
    }
    // (Shown with the censor's pads, or while it is switched on.)
    if app.pad_mode() != PadMode::ActiveCensor && !c.on {
        return;
    }
    let Some((r, pos, pps)) = app.deck.detail_map else { return };
    let p = ui.painter().with_clip_rect(r);
    let x_of = |t: f64| r.center().x + ((t - pos) * pps) as f32;
    // (`activeCenserOn.svg` is #93850E, `activeCenserOff.svg` #969696.)
    let col = if c.on { Color32::from_rgba_unmultiplied(147, 133, 14, 110) } else { Color32::from_rgba_unmultiplied(150, 150, 150, 70) };
    for reg in &c.regions {
        let (x0, x1) = (x_of(reg.a), x_of(reg.b));
        if x1 < r.min.x || x0 > r.max.x {
            continue;
        }
        p.rect_filled(Rect::from_min_max(pos2(x0, r.min.y), pos2(x1.max(x0 + 1.0), r.max.y)), 0.0, col);
    }
    if let Some(t) = c.pending_in {
        let x = x_of(t);
        p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.5_f32, hex(0x93850E)));
    }
}

// ------------------------------------------------------------------------------- USER 1..8

/// A USER mode: its sixteen pads, eight showing (the shift page is the second half).
fn user_view(app: &mut App, ui: &mut Ui, g: Geom, n: u8, u: usize) {
    let loaded = app.deck.track.is_some();
    let items = app.user_items(u);
    let (sc, cw, ch) = if g.v2 {
        v2_list(app, ui, g, n, PadMode::User(u as u8), 16)
    } else {
        let cw = ((g.area.width() - 2.0) / 2.0).floor();
        let sc = scroll_area(app, ui, g.area, n, PadMode::User(u as u8), 4.0 * (cw + 2.0) - 2.0, false);
        let ch = ((sc.view.height() - 6.0) / 4.0).floor();
        (sc, cw, ch)
    };
    let p = ui.painter().with_clip_rect(sc.view);
    let o = sc.origin();
    let font = arial(ui, 12.0);
    let shift = ui.input(|i| i.modifiers.shift);
    let lit_loop = app.beat_loop_lit();
    let key = app.pad_current_key();
    for (i, item) in items.iter().enumerate().take(16) {
        let r = if g.v2 { v2_cell(&sc, cw, i) } else { Rect::from_min_size(o + vec2((i / 4) as f32 * (cw + 2.0), (i % 4) as f32 * (ch + 2.0)), vec2(cw, ch)) };
        if !r.intersects(sc.view) {
            continue;
        }
        let act = user_act(item);
        // (Sampler and sequence pads work without a track on the deck.)
        let enabled = match act {
            UserAct::Sampler(_) | UserAct::SeqCall(_) | UserAct::PadFx(..) | UserAct::SampleScratch(_) | UserAct::Transport(Transport::Track(_)) => true,
            UserAct::Other => false,
            _ => loaded,
        };
        let lit = match &act {
            UserAct::BeatLoop(b) => lit_loop.is_some() && lit_loop == loop_pad_for(*b),
            UserAct::Key(k) => key_semis(*k).is_some_and(|s| s == key),
            _ => false,
        };
        let look = Look::pad(enabled, lit);
        let resp = pad_body(ui, &p, r, sc.view, ui.id().with(("userpad", i)), look, enabled);
        match &act {
            UserAct::HotCue(slot) => {
                // As a hot cue pad: the chip when the cue is set.
                let cue = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == *slot).cloned();
                let letter = ((b'A' + *slot as u8) as char).to_string();
                match cue {
                    Some(c) => {
                        let chip = Rect::from_min_size(pos2(r.min.x + 10.0, (r.center().y - 5.5).round()), vec2(11.0, 11.0));
                        p.rect_filled(chip, 0.0, hotcue_color(c.color));
                        p.text(chip.center(), Align2::CENTER_CENTER, &letter, arial(ui, 10.5), Color32::BLACK);
                        let ms = (c.start * 1000.0).round() as u64;
                        let mut text = format!("{:02}:{:02}:{:03}", ms / 60000, (ms / 1000) % 60, ms % 1000);
                        if let Some(k) = item.key {
                            text.push_str(&format!("  KEY {k}"));
                        }
                        p.text(pos2(r.min.x + 31.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(10.7), gy(0xF4F4F4));
                    }
                    None => {
                        p.text(pos2(r.min.x + 15.5, r.center().y), Align2::CENTER_CENTER, &letter, font.clone(), if loaded { gy(0x646464) } else { PAD_OFF.c() });
                    }
                }
            }
            UserAct::BeatJump(b, fwd) => {
                let col = if loaded { hex(0xE62828) } else { PAD_OFF.c() };
                triangle(&p, pos2(r.center().x + if *fwd { 22.0 } else { -22.0 }, r.center().y), 6.4, 7.2, !*fwd, col);
                pad_text(&p, r, &b.map(crate::player_ext::beats_text).unwrap_or_else(|| "FINE".into()), font.clone(), look.text);
            }
            _ => pad_text(&p, r, &user_label(item), font.clone(), look.text),
        }
        if press_fired(ui, &resp) && enabled {
            app.user_pad_down(item, shift);
        }
    }
    if !ui.input(|i| i.pointer.any_down()) {
        app.user_pad_up();
    }
}

/// The Pad Editor (rekordbox opens it with PAD in the top bar), laid out as rekordbox's
/// `PadEditorComponent` (`pads::editor_geo`, traced): the ASSIGN 1 / ASSIGN 2 tabs, the eight
/// PERFORMANCE MODE buttons (the mode each of a controller's pad mode buttons selects), the
/// TEMPLATES button and the sixteen PERFORMANCE PADS of the mode picked; a USER mode's pads
/// can be changed with their pull-down.
fn pad_editor(app: &mut App, ui: &mut Ui) {
    if !app.pads.editor_open {
        return;
    }
    // (Drawn once per frame, whichever deck comes first.)
    let frame = ui.ctx().cumulative_pass_nr();
    let seen = ui.ctx().data_mut(|d| {
        let last = d.get_temp::<u64>(egui::Id::new("pad_editor_frame"));
        d.insert_temp(egui::Id::new("pad_editor_frame"), frame);
        last == Some(frame)
    });
    if seen {
        return;
    }
    use crate::pads::editor_geo as eg;
    static MODES: std::sync::OnceLock<Vec<ModeEntry>> = std::sync::OnceLock::new();
    let modes = MODES.get_or_init(read_mode_csv);
    // The eight buttons' modes for the assign shown (rekordbox's `select` column by default).
    let assign = (app.prefs.pads.editor_assign as usize).min(1);
    while app.prefs.pads.editor_modes.len() < 2 {
        let defaults: Vec<String> = (1..=8u8).map(|k| modes.iter().find(|m| m.select == Some(k)).map(|m| m.function.clone()).unwrap_or_default()).collect();
        app.prefs.pads.editor_modes.push(defaults);
    }
    let slot = (app.prefs.pads.editor_slot as usize).min(7);
    let slot_fn = app.prefs.pads.editor_modes[assign].get(slot).cloned().unwrap_or_default();
    let user_of = |f: &str| f.strip_prefix("User").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=8).contains(n)).map(|n| n - 1);
    let text_of = |f: &str| modes.iter().find(|m| m.function == f).map(|m| m.text.clone()).unwrap_or_else(|| f.to_string());
    let mut open = true;
    let mut changed = false;
    // (The PAD EDITOR window keeps its fixed dark colours, so it is drawn Dark in Light: its
    // colours in rekordbox's Light appearance were not traced.)
    let _pe_dark = crate::theme::dark_scope(true);
    egui::Window::new(egui::RichText::new("PAD EDITOR").color(TEXT.0)).id(egui::Id::new("pad_editor")).open(&mut open).resizable(false).collapsible(false).frame(egui::Frame::window(&ui.ctx().style()).inner_margin(0.0).fill(hex(eg::BG))).show(ui.ctx(), |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(eg::W, eg::H), Sense::hover());
        let o = rect.min;
        let at = |g: (f32, f32, f32, f32)| Rect::from_min_size(o + vec2(g.0, g.1), vec2(g.2, g.3));
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, hex(eg::BG));
        p.rect_filled(Rect::from_min_size(o, vec2(eg::W, eg::TAB_H)), 0.0, hex(eg::STRIP));
        // Tabs: 124 wide, no overlap; the front one in the window's colour, the other #191919;
        // text 0.45 of the bar's height.
        for (k, name) in ["ASSIGN 1", "ASSIGN 2"].iter().enumerate() {
            let tr = at((124.0 * k as f32, 0.0, 124.0, eg::TAB_H));
            let resp = ui.interact(tr, ui.id().with(("petab", k)), Sense::click());
            let front = k == assign;
            p.rect_filled(tr, 0.0, if front { hex(eg::BG) } else { hex(eg::FILL) });
            p.text(tr.center(), Align2::CENTER_CENTER, *name, arial(ui, eg::TAB_H * 0.45), if front { WHITE.c() } else { hex(eg::TEXT) });
            if resp.clicked() && !front {
                app.prefs.pads.editor_assign = k as u8;
                changed = true;
            }
        }
        let label_font = arial(ui, 12.0);
        p.text(o + vec2(25.0, eg::MODE_LABEL_Y + eg::LABEL_H / 2.0), Align2::LEFT_CENTER, "PERFORMANCE MODE", label_font.clone(), hex(eg::TEXT));
        p.text(o + vec2(25.0, eg::PADS_LABEL_Y + eg::LABEL_H / 2.0), Align2::LEFT_CENTER, "PERFORMANCE PADS", label_font, hex(eg::TEXT));
        // The eight mode buttons: outlined, square; the picked one in blue; the pull-down
        // (`pullDown.svg` at 122, 4, 10 x 16) lists the modes of `Mode.pad.csv`.
        let mode_font = arial(ui, 14.0);
        for i in 0..8usize {
            let br = at(eg::mode_button(i));
            let resp = ui.interact(br, ui.id().with(("pemode", i)), Sense::click());
            let on = i == slot;
            let f = app.prefs.pads.editor_modes[assign].get(i).cloned().unwrap_or_default();
            p.rect_filled(br, 0.0, hex(eg::FILL));
            p.rect_stroke(br, 0.0, Stroke::new(1.0_f32, if on { hex(eg::ON) } else { hex(eg::OUTLINE) }), egui::StrokeKind::Inside);
            let col = if on { hex(eg::ON) } else { hex(eg::TEXT) };
            p.with_clip_rect(br).text(br.min + vec2(10.0, br.height() / 2.0), Align2::LEFT_CENTER, text_of(&f), mode_font.clone(), col);
            let arrow = Rect::from_min_size(br.min + vec2(122.0, 4.0), vec2(10.0, 16.0));
            triangle_down(&p, arrow.center(), 7.0, 4.0, col);
            if resp.clicked() {
                if ui.input(|i| i.pointer.interact_pos()).is_some_and(|pp| pp.x >= br.min.x + 110.0) {
                    ui.memory_mut(|m| m.toggle_popup(egui::Id::new(("pemodepop", i))));
                }
                app.prefs.pads.editor_slot = i as u8;
                changed = true;
            }
            egui::popup::popup_below_widget(ui, egui::Id::new(("pemodepop", i)), &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(160.0);
                for m in modes.iter() {
                    if m.kind == "separator" {
                        ui.separator();
                        continue;
                    }
                    if !m.mode {
                        continue;
                    }
                    if ui.selectable_label(m.function == f, &m.text).clicked() {
                        if let Some(v) = app.prefs.pads.editor_modes[assign].get_mut(i) {
                            *v = m.function.clone();
                        }
                        changed = true;
                    }
                }
            });
        }
        // TEMPLATES: copies a template's sixteen pads into the USER mode picked (rounded
        // corners 2, no outline; its arrow 10 x 4 at 122, 13).
        let tr = at(eg::TEMPLATES);
        let user = user_of(&slot_fn);
        let tresp = ui.interact(tr, ui.id().with("petemplates"), Sense::click());
        p.rect_filled(tr, 2.0, hex(eg::FILL));
        let tcol = if user.is_some() { hex(eg::TEXT) } else { gy(0x646464) };
        p.text(tr.min + vec2(10.0, tr.height() / 2.0), Align2::LEFT_CENTER, "TEMPLATES", mode_font.clone(), tcol);
        triangle_down(&p, tr.min + vec2(127.0, 15.0), 10.0, 4.0, tcol);
        if tresp.clicked() && user.is_some() {
            ui.memory_mut(|m| m.toggle_popup(egui::Id::new("petemplpop")));
        }
        egui::popup::popup_below_widget(ui, egui::Id::new("petemplpop"), &tresp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(180.0);
            for m in modes.iter().filter(|m| m.template) {
                if ui.button(&m.text).clicked() {
                    if let Some(u) = user {
                        let items = match user_of(&m.function) {
                            Some(src) => app.user_items(src),
                            None => mode_items(&m.function),
                        };
                        for (k, it) in items.iter().enumerate().take(16) {
                            app.user_set_item(u, k, Some((&it.function, &it.value)));
                        }
                    }
                }
            }
        });
        // The sixteen pads: the function (Arial Bold 14 at 10, 0, 120 x 29) and the value
        // under it; a USER mode's pads have a pull-down (10 x 16 at 122, 6).
        let items: Vec<PadItem> = match user {
            Some(u) => app.user_items(u),
            None => mode_items(&slot_fn),
        };
        let functions = pad_functions();
        let bold = egui::FontId::new(14.0 * 0.87, egui::FontFamily::Proportional);
        for k in 0..16usize {
            let pr = at(eg::pad(k));
            p.rect_filled(pr, 2.0, hex(eg::FILL));
            let Some(it) = items.get(k) else { continue };
            let name = functions.iter().find(|f| f.0 == it.function).map(|f| f.1.to_string()).unwrap_or_else(|| text_of(&it.function));
            let pc = p.with_clip_rect(pr);
            pc.text(pr.min + vec2(10.0, 14.5), Align2::LEFT_CENTER, &name, bold.clone(), hex(eg::TEXT));
            pc.text(pr.min + vec2(10.0, 45.0), Align2::LEFT_CENTER, &it.value, arial(ui, 12.0), WHITE.c());
            if let Some(u) = user {
                let ar = Rect::from_min_size(pr.min + vec2(122.0, 6.0), vec2(10.0, 16.0));
                triangle_down(&p, ar.center(), 7.0, 4.0, hex(eg::TEXT));
                let resp = ui.interact(pr, ui.id().with(("pepad", k)), Sense::click());
                if resp.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(egui::Id::new(("pepadpop", k))));
                }
                egui::popup::popup_below_widget(ui, egui::Id::new(("pepadpop", k)), &resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                    ui.set_min_width(170.0);
                    for (f, fname, values) in &functions {
                        ui.menu_button(*fname, |ui| {
                            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                for v in values {
                                    if ui.selectable_label(it.function == *f && it.value == *v, v).clicked() {
                                        app.user_set_item(u, k, Some((f, v)));
                                        ui.close_menu();
                                    }
                                }
                            });
                        });
                    }
                    ui.separator();
                    if ui.button("Reset").clicked() {
                        app.user_set_item(u, k, None);
                        ui.close_menu();
                    }
                });
            }
        }
        // (USER modes listed in the decks' pull-down: those assigned to a button.)
    });
    // A USER mode assigned to one of the buttons is listed in the decks' pad mode pull-down.
    let mut shown = [false; 8];
    for f in app.prefs.pads.editor_modes.iter().flatten() {
        if let Some(u) = f.strip_prefix("User").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=8).contains(n)) {
            shown[u - 1] = true;
        }
    }
    if shown != app.prefs.pads.user_shown {
        app.prefs.pads.user_shown = shown;
        changed = true;
    }
    if changed {
        app.save_prefs();
    }
    if !open {
        app.pads.editor_open = false;
    }
}

/// A small downward triangle (`pullDown.svg`).
fn triangle_down(p: &Painter, c: Pos2, w: f32, h: f32, col: Color32) {
    p.add(egui::Shape::convex_polygon(vec![pos2(c.x - w / 2.0, c.y - h / 2.0), pos2(c.x + w / 2.0, c.y - h / 2.0), pos2(c.x, c.y + h / 2.0)], col, Stroke::NONE));
}

// ------------------------------------------------------------------------------- MEMORY CUE

/// MEMORY CUE: the track's memory cues and loops as rows, two columns of four. A press calls
/// the cue; the cross deletes it.
fn memory_view(app: &mut App, ui: &mut Ui, g: Geom, n: u8) {
    let cues: Vec<Cue> = {
        let mut v: Vec<Cue> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).cloned().collect();
        v.sort_by(|a, b| a.start.total_cmp(&b.start));
        v
    };
    let cols = cues.len().div_ceil(4).max(2);
    let (sc, cw, ch) = if g.v2 {
        v2_list(app, ui, g, n, PadMode::MemoryCue, cols * 4)
    } else {
        let cw = ((g.area.width() - 2.0) / 2.0).floor();
        let sc = scroll_area(app, ui, g.area, n, PadMode::MemoryCue, cols as f32 * (cw + 2.0) - 2.0, false);
        let ch = ((sc.view.height() - 6.0) / 4.0).floor();
        (sc, cw, ch)
    };
    let p = ui.painter().with_clip_rect(sc.view);
    let o = sc.origin();
    for i in 0..cols * 4 {
        let r = if g.v2 { v2_cell(&sc, cw, i) } else { Rect::from_min_size(o + vec2((i / 4) as f32 * (cw + 2.0), (i % 4) as f32 * (ch + 2.0)), vec2(cw, ch)) };
        if !r.intersects(sc.view) {
            continue;
        }
        let Some(c) = cues.get(i) else {
            p.rect_filled(r, 2.0, PAD_BG);
            continue;
        };
        let resp = pad_body(ui, &p, r, sc.view, ui.id().with(("mempad", c.id)), Look { fill: PAD_BG.c(), text: PAD_TEXT.c(), outline: None, radius: 2.0 }, true);
        let cy = r.center().y;
        // A triangle in the cue's colour, the time, the comment.
        let col = if c.color == 0 { hex(0xE61E1E) } else { label_color(c.color) };
        triangle(&p, pos2(r.min.x + 14.0, cy), 7.0, 8.0, false, if c.end.is_some() { LOOP } else { col });
        let ms = (c.start * 1000.0).round() as u64;
        let tp = p.with_clip_rect(Rect::from_min_max(r.min, pos2(r.max.x - 26.0, r.max.y)).intersect(sc.view));
        let tr = tp.text(pos2(r.min.x + 26.0, cy), Align2::LEFT_CENTER, format!("{:02}:{:02}:{:03}", ms / 60000, (ms / 1000) % 60, ms % 1000), FontId::proportional(10.7), gy(0xF4F4F4));
        tp.text(pos2(tr.max.x + 8.0, cy), Align2::LEFT_CENTER, &c.comment, FontId::proportional(10.7), gy(0xF4F4F4));
        let dr = Rect::from_min_size(pos2(r.max.x - 24.0, cy - 7.0), vec2(14.0, 14.0));
        let dresp = ui.interact(dr.intersect(sc.view), ui.id().with(("memdel", c.id)), Sense::click());
        crate::icons_player::cross(&p, dr.center(), 8.0, 1.0, if dresp.hovered() { WHITE.c() } else { GREY.c() });
        if dresp.clicked() {
            app.delete_cue(c.id);
            continue;
        }
        if press_fired(ui, &resp) && !dresp.hovered() {
            app.seek(c.start);
            if let Some(e) = c.end.filter(|e| *e > c.start) {
                app.player.state.lock().loop_range = Some((c.start, e));
                app.deck.last_loop = Some((c.start, e));
            }
            if !app.playing() {
                app.deck.cue_point = c.start;
            }
        }
    }
}

// ------------------------------------------------------------------------------- Preferences

/// Preferences > View > Color > HOT CUE color: rekordbox's four colour sets for new hot cues.
pub fn prefs_hot_cue_color(app: &mut App, ui: &mut Ui) {
    use crate::ui_kit as kit;
    let cur = app.prefs.pads.hot_cue_color.clamp(1, 4);
    kit::combo_row(ui, 22.0, |ui| {
        kit::combo("hotcue_color", 520.0, HOT_CUE_SETS[(cur - 1) as usize].0, 14.0).show_ui(ui, |ui| {
            for (i, (name, _)) in HOT_CUE_SETS.iter().enumerate() {
                if ui.selectable_label(cur as usize == i + 1, *name).clicked() {
                    app.prefs.pads.hot_cue_color = i as u8 + 1;
                    app.save_prefs();
                }
            }
        });
    });
    // The set's eight colours (A to H).
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        for slot in 0..8 {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, hotcue_color(hot_cue_set_color(cur, slot, false)));
        }
    });
}

/// Preferences > View > Layout > PAD Mode: Auto, or Customize with LIST Display / PAD Display.
pub fn prefs_pad_mode(app: &mut App, ui: &mut Ui) {
    use crate::ui_kit as kit;
    kit::sub(ui, "PAD Mode");
    let auto = app.prefs.pads.layout <= 1;
    if kit::radio(ui, auto, "Auto") {
        app.prefs.pads.layout = 1;
        app.save_prefs();
    }
    if kit::radio(ui, !auto, "Customize") && auto {
        app.prefs.pads.layout = 2;
        app.save_prefs();
    }
    if !auto {
        let cur = if app.prefs.pads.layout == 3 { "PAD Display" } else { "LIST Display" };
        kit::combo_row(ui, 22.0, |ui| {
            kit::combo("pad_mode_layout", 260.0, cur, 14.0).show_ui(ui, |ui| {
                for (v, name) in [(2u8, "LIST Display"), (3, "PAD Display")] {
                    if ui.selectable_label(app.prefs.pads.layout == v, name).clicked() {
                        app.prefs.pads.layout = v;
                        app.save_prefs();
                    }
                }
            });
        });
    }
}
