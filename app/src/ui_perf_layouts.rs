//! PERFORMANCE mode layouts: which one is showing, how tall it is, where every piece sits, and
//! the layouts beyond the two horizontal ones (2Deck Vertical, 4Deck Vertical, Browse, and the
//! horizontal layouts without pad and platter). `ui_perf.rs` draws the deck pieces; the numbers
//! they use come from here.
//!
//! Sources: rekordbox 7.2.19's own layout code (`checks/re-7.2.19/performance-decks.md` and
//! its geometry file; `PlayerLayout::LayoutMode`, `PlayerPanelL::getPlayerHeight`,
//! `WaveViewArea::ControlView`, `PlayerWaveView::resized`, `UiPlayer::getWaveRatioValueForDJ`)
//! and the captures `rekordbox-reference/performance/01` to `05` at 1512 pt. Positions are in
//! points; "player" coordinates start just under the 28 pt top bar.

use crate::app::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_player::{lighten, tip};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, Ui};
use serde::{Deserialize, Serialize};
use std::cell::Cell;

// ------------------------------------------------------------------ which layout

/// What is kept of the Performance layout besides `Prefs::perf_four` (2 or 4 decks).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct LayoutPrefs {
    /// The enlarged waveforms run top to bottom between the decks (rekordbox's bit 0x100).
    pub vertical: bool,
    /// The Browse layout: deck headers only, the browser takes the rest.
    pub browse: bool,
    /// rekordbox's `HidePlatter`: the horizontal layouts without pads and platters.
    pub hide_platter: bool,
    /// Zoom step (0 to 6; 4 is what RST returns to) of the horizontal layouts, of 2Deck
    /// Vertical and of 4Deck Vertical.
    pub zoom: [u8; 3],
    /// The control strip beside the waveforms is open (rekordbox's `WaveControlOpen`), and
    /// its page: 0 = zoom, 1 = matching and cut, 2 = capture.
    pub ctl_open: bool,
    pub ctl_page: u8,
    /// The MIXER panel is hidden (the mixer column and the crossfader row).
    pub mixer_hidden: bool,
}

impl Default for LayoutPrefs {
    fn default() -> Self {
        LayoutPrefs { vertical: false, browse: false, hide_platter: false, zoom: [ZOOM_RESET; 3], ctl_open: true, ctl_page: 0, mixer_hidden: false }
    }
}

/// The eight Performance layouts, named as in rekordbox.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Layout {
    H2,
    V2,
    H4,
    V4,
    Browse2,
    Browse4,
    /// 2Deck / 4Deck Horizontal with pad and platter hidden.
    Simple2,
    Simple4,
}

impl Layout {
    /// rekordbox's `PlayerLayout::LayoutMode` value (kept as its `DjLayout` setting).
    pub fn mode(self) -> u32 {
        match self {
            Layout::H2 => 18,
            Layout::H4 => 20,
            Layout::V2 => 274,
            Layout::V4 => 276,
            Layout::Browse2 => 4098,
            Layout::Browse4 => 4100,
            Layout::Simple2 => 4114,
            Layout::Simple4 => 4116,
        }
    }
    pub fn decks(self) -> u8 {
        (self.mode() & 6) as u8
    }
    /// Bit 0x10: a full deck layout, where the mixer may be shown.
    pub fn full(self) -> bool {
        self.mode() & 0x10 != 0
    }
    pub fn vertical(self) -> bool {
        self.mode() & 0x100 != 0
    }
    /// Bit 0x1000: no pad and platter.
    pub fn simple(self) -> bool {
        self.mode() & 0x1000 != 0
    }
    /// Row of the layout pull-down (2Deck Horizontal, 2Deck Vertical, 4Deck Horizontal,
    /// 4Deck Vertical, Browse).
    pub fn row(self) -> usize {
        match self {
            Layout::H2 | Layout::Simple2 => 0,
            Layout::V2 => 1,
            Layout::H4 | Layout::Simple4 => 2,
            Layout::V4 => 3,
            Layout::Browse2 | Layout::Browse4 => 4,
        }
    }
}

pub(crate) const LAYOUT_NAMES: [&str; 5] = ["2Deck Horizontal", "2Deck Vertical", "4Deck Horizontal", "4Deck Vertical", "Browse"];
/// The selector's marks in rekordbox's skin folder (the misspelling is rekordbox's).
const LAYOUT_MARKS: [&str; 5] = ["globalsection/2Deck Horizontal.svg", "globalsection/2Deck Vartical.svg", "globalsection/4Deck Horizontal.svg", "globalsection/4Deck Vartical.svg", "globalsection/BROWSER.svg"];

pub(crate) fn layout_of(four: bool, l: &LayoutPrefs) -> Layout {
    match (l.browse, l.vertical, four) {
        (true, _, false) => Layout::Browse2,
        (true, _, true) => Layout::Browse4,
        (false, true, false) => Layout::V2,
        (false, true, true) => Layout::V4,
        (false, false, false) => {
            if l.hide_platter {
                Layout::Simple2
            } else {
                Layout::H2
            }
        }
        (false, false, true) => {
            if l.hide_platter {
                Layout::Simple4
            } else {
                Layout::H4
            }
        }
    }
}

pub(crate) fn layout(app: &App) -> Layout {
    layout_of(app.prefs.perf_four, &app.prefs.perf_lay)
}

/// A row of the layout pull-down was chosen (`changeDeckLayoutInPerformanceMode`: Browse keeps
/// the number of decks; the horizontal rows follow `HidePlatter`).
pub(crate) fn choose_row(four: &mut bool, l: &mut LayoutPrefs, row: usize) {
    match row {
        0 | 2 => {
            *four = row == 2;
            l.vertical = false;
            l.browse = false;
        }
        1 | 3 => {
            *four = row == 3;
            l.vertical = true;
            l.browse = false;
        }
        _ => l.browse = true,
    }
}

pub(crate) fn set_row(app: &mut App, row: usize) {
    choose_row(&mut app.prefs.perf_four, &mut app.prefs.perf_lay, row);
    app.poll_decks();
    app.save_prefs();
}

/// The mixer column and the crossfader row are showing (`shouldBeMixerVisible`, and the
/// layout allows it).
pub(crate) fn mixer_shown(app: &App) -> bool {
    let l = layout(app);
    l.full() && !l.simple() && !app.prefs.perf_lay.mixer_hidden
}

/// Whether a panel button of the top bar is available in the layout showing
/// (`MainComponent::enableToolbarButton`): none in Browse; with pad and platter hidden only
/// REC; FX, sampler, mixer and REC otherwise. `i`: the buttons in rekordbox's order, 0 FX,
/// 1 Mix Point Link, 2 Sampler, 3 Mixer, 4 Rec (the skin's `glb_sampler` tile is the 3 x 3 grid,
/// `glb_mix_point_link` the two stepped waveforms of capture 01).
pub(crate) fn panel_enabled(app: &App, i: usize) -> bool {
    let l = layout(app);
    if !l.full() {
        return false;
    }
    !l.simple() || i == 4
}

// ------------------------------------------------------------------ heights

/// Extra height of one deck for the rows the View preferences add: the vocal row (7 pt), the
/// phrase row (5 pt, 16 pt with the phrase names) and the stem part buttons (21 pt).
/// (`getPlayerHeight`: the same amounts, doubled in the 4-deck layouts.)
pub(crate) fn deck_extra(app: &App) -> f32 {
    vocal_row_h(app) + phrase_row_h(app) + parts_row_h(app)
}
pub(crate) fn vocal_row_h(app: &App) -> f32 {
    if app.prefs.show_vocal {
        7.0
    } else {
        0.0
    }
}
pub(crate) fn phrase_row_h(app: &App) -> f32 {
    match (app.prefs.show_phrase_bar, app.prefs.phrase_types_always) {
        (false, _) => 0.0,
        (true, true) => 16.0,
        (true, false) => 5.0,
    }
}
pub(crate) fn parts_row_h(app: &App) -> f32 {
    if app.prefs.show_stems {
        21.0
    } else {
        0.0
    }
}

/// rekordbox's `getPlayerHeight` for the layout showing, without the FX and sampler panels.
pub(crate) fn player_height(app: &App) -> f32 {
    let l = layout(app);
    let mixer = if mixer_shown(app) { 43.0 } else { 0.0 };
    let rows = if l.decks() == 4 { 2.0 } else { 1.0 };
    match l {
        Layout::Browse2 => 95.0,
        Layout::Browse4 => 193.0,
        Layout::Simple2 => 236.0,
        Layout::Simple4 => 390.0,
        Layout::H2 => 368.0 + mixer + rows * deck_extra(app),
        Layout::V2 => 297.0 + mixer + rows * deck_extra(app),
        Layout::H4 => 576.0 + mixer + rows * deck_extra(app),
        Layout::V4 => 415.0 + mixer + rows * deck_extra(app),
    }
}

/// Height of the whole Performance area under the top bar. The captures show 4 pt more than
/// `getPlayerHeight` in every layout: 1 pt above the decks and a 3 pt gutter above the browser.
pub(crate) fn height(app: &App) -> f32 {
    let l = layout(app);
    let fx = if app.prefs.perf_fx && panel_enabled(app, 0) { crate::ui_fx::fx_h(app) } else { 0.0 };
    let smp = if app.prefs.perf_sampler && panel_enabled(app, 2) { crate::ui_sampler::SAMPLER_H } else { 0.0 };
    let _ = l;
    player_height(app) + 4.0 + fx + smp
}

// ------------------------------------------------------------------ zoom

/// What RST returns to: the fifth of the seven steps (`getWaveRatioValueForDJ` falls back to
/// index 4; the captures show 8 s, 2 s and 4 s across the waveform).
pub const ZOOM_RESET: u8 = 4;
/// rekordbox's zoom steps: milliseconds of music across the length of the waveform view
/// (`horizontalWaveRatio`, `verticalWaveRatio2`, `verticalWaveRatio4`).
pub const ZOOM_MS: [[f64; 7]; 3] = [[1000.0, 2000.0, 3000.0, 4000.0, 8000.0, 16000.0, 24000.0], [250.0, 500.0, 750.0, 1000.0, 2000.0, 4000.0, 6000.0], [500.0, 1000.0, 1500.0, 2000.0, 4000.0, 8000.0, 12000.0]];

fn zoom_family(l: Layout) -> usize {
    match l {
        Layout::V2 => 1,
        Layout::V4 => 2,
        _ => 0,
    }
}

/// Seconds of music across the enlarged waveform in Performance mode; `None` in Export mode.
pub(crate) fn span(app: &App) -> Option<f64> {
    if !app.performance() {
        return None;
    }
    let f = zoom_family(layout(app));
    Some(ZOOM_MS[f][(app.prefs.perf_lay.zoom[f] as usize).min(6)] / 1000.0)
}

/// Zoom in (-1), out (+1) or reset (0) for the layout showing. Returns whether it changed.
pub(crate) fn zoom_step(app: &mut App, dir: i32) -> bool {
    let f = zoom_family(layout(app));
    let z = &mut app.prefs.perf_lay.zoom[f];
    let new = match dir {
        0 => ZOOM_RESET,
        d if d < 0 => z.saturating_sub(1),
        _ => (*z + 1).min(6),
    };
    let changed = new != *z;
    *z = new;
    if changed {
        app.save_prefs();
    }
    changed
}

// ------------------------------------------------------------------ rekordbox's skin images

thread_local! {
    static SKIN: std::cell::RefCell<std::collections::HashMap<String, Option<egui::TextureHandle>>> = std::cell::RefCell::new(Default::default());
    /// The deck variant the pieces of `ui_perf.rs` are being drawn for (see `with_dk`).
    static DK: Cell<Option<Dk>> = const { Cell::new(None) };
    /// The control strip's cut (capture) button is on (`PlayerWaveView::onCaptureButton`).
    static CAPTURE_MODE: Cell<bool> = const { Cell::new(false) };
    /// A capture being dragged from a waveform: the deck, slicer (true) or loop, and where it
    /// was picked up.
    static CAPTURE_DRAG: Cell<Option<(u8, bool, Pos2)>> = const { Cell::new(None) };
    /// Where the sampler panel was last drawn (for the scripted run's capture drag).
    static LAST_SAMPLER: Cell<Option<Rect>> = const { Cell::new(None) };
}

/// The waveform's capture mode (the control strip's cut button).
pub(crate) fn capture_mode() -> bool {
    CAPTURE_MODE.with(|c| c.get())
}

/// Over a deck's waveform (`lane`; vertical: the time runs down) while capture mode is on:
/// a press on the running loop or on the slicer's domain picks it up to be dropped on a
/// sampler slot (`WidgetWaveView::startCaptureDragging`). Called after the waveform has been
/// drawn, so that this takes the drag before the waveform's own scrubbing.
fn capture_pick(app: &mut App, ui: &mut Ui, lane: Rect, n: u8, vertical: bool, pos: f64, pps: f64) {
    if !capture_mode() || app.deck.audio.is_none() {
        return;
    }
    let ni = (n as usize).min(3);
    let slicer = app.pads.deck[ni].slicer.active;
    let region = if slicer {
        let b = app.pads.deck[ni].slicer.b;
        Some((b[0], b[8]))
    } else {
        app.loop_active()
    };
    let Some((a, b)) = region else {
        let resp = ui.interact(lane, ui.id().with(("capnone", n)), Sense::click());
        if resp.clicked() {
            app.toast_err("Invalid operation. Capture is available during Loop or Slicer performance.");
        }
        return;
    };
    let rr = capture_region(lane, vertical, pos, pps, a, b);
    if rr.width() <= 0.0 || rr.height() <= 0.0 {
        return;
    }
    // (rekordbox shades the area that would be captured.)
    ui.painter().rect_filled(rr, 0.0, Color32::from_rgba_unmultiplied(20, 115, 235, 50));
    let resp = ui.interact(rr, ui.id().with(("cappick", n)), Sense::drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if resp.drag_started() {
        let pp = ui.input(|i| i.pointer.press_origin()).unwrap_or(rr.center());
        CAPTURE_DRAG.with(|c| c.set(Some((n, slicer, pp))));
    }
}

/// Project the selected loop or slicer range using the transform already painted this frame.
fn capture_region(lane: Rect, vertical: bool, pos: f64, pps: f64, a: f64, b: f64) -> Rect {
    let center = if vertical { lane.center().y } else { lane.center().x };
    let px = |t| center + ((t - pos) * pps) as f32;
    let (lo, hi) = (px(a.min(b)), px(a.max(b)));
    if vertical {
        Rect::from_min_max(pos2(lane.min.x, lo), pos2(lane.max.x, hi))
    } else {
        Rect::from_min_max(pos2(lo, lane.min.y), pos2(hi, lane.max.y))
    }.intersect(lane)
}

/// The capture being dragged: drawn under the pointer, and dropped on a sampler slot when the
/// button is let go there (the sampler panel is the last row; slots as `ui_sampler::panel`
/// lays them out: 16 of 44 pt height, pads 1-8 on the left, 9-16 on the right).
pub(crate) fn capture_drop(app: &mut App, ui: &mut Ui, full: Rect) {
    let sampler = if app.prefs.perf_sampler { Some(Rect::from_min_max(pos2(full.min.x, full.max.y - crate::ui_sampler::SAMPLER_H), full.max)) } else { None };
    LAST_SAMPLER.with(|c| c.set(sampler));
    let Some((n, slicer, _)) = CAPTURE_DRAG.with(|c| c.get()) else { return };
    let ptr = ui.ctx().pointer_latest_pos();
    let down = ui.input(|i| i.pointer.primary_down());
    let slot_at = |pp: Pos2| -> Option<usize> {
        let s = sampler?;
        let w = s.width();
        let slot_w = ((w - 80.0) / 8.0).floor().max(146.0);
        (0..app.sampler_pads()).find(|pad| {
            let (col, row) = (pad & 3, (pad & 7) >> 2);
            let x = if *pad < 8 { col as f32 * slot_w + 4.0 } else { w + (col as f32 - 4.0) * slot_w - 4.0 };
            Rect::from_min_size(pos2(s.min.x + x, s.min.y + 44.0 * row as f32 + 4.0), vec2(slot_w, 44.0)).contains(pp)
        })
    };
    if down {
        if let Some(pp) = ptr {
            let g = Rect::from_center_size(pp, vec2(90.0, 22.0));
            let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("capdrag")));
            lp.rect_filled(g, 2.0, Color32::from_rgba_unmultiplied(20, 115, 235, 160));
            lp.text(g.center(), Align2::CENTER_CENTER, if slicer { "SLICER" } else { "LOOP" }, arial_b(11.0), WHITE.c());
            if let Some(pad) = slot_at(pp) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
                let _ = pad;
            }
        }
        ui.ctx().request_repaint();
        return;
    }
    CAPTURE_DRAG.with(|c| c.set(None));
    let Some(pad) = ptr.and_then(slot_at) else { return };
    let idx = app.prefs.sampler.bank.min(3) as usize * 16 + pad;
    if slicer {
        app.with_deck(n, |a| {
            a.slicer_capture();
        });
    } else {
        app.sampler_capture_loop(n, Some(idx));
    }
}

const SKINS: &str = "/Applications/rekordbox 7/rekordbox.app/Contents/Resources/skins";

/// An image of the installed rekordbox's skin folder (read at run time, never copied), as a
/// texture; `None` when rekordbox is not installed or the file cannot be drawn. SVGs are
/// rendered at four times their size so that they stay sharp on a Retina screen.
pub(crate) fn skin(ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
    SKIN.with(|s| {
        if let Some(t) = s.borrow().get(name) {
            return t.clone();
        }
        // "<file>#light": the file with its greys in their Light values (see `recolour_svg`);
        // "<file>#swap:<from>><to>,…": with the colours listed replaced.
        let (file, how) = name.split_once('#').unwrap_or((name, ""));
        let path = format!("{SKINS}/{file}");
        let tex = std::fs::read(&path).ok().and_then(|b| {
            let img = if file.ends_with(".svg") && !how.is_empty() {
                let b = recolour_svg(&String::from_utf8_lossy(&b), how).into_bytes();
                egui_extras::image::load_svg_bytes_with_size(&b, Some(egui::SizeHint::Scale(4.0.into()))).ok()?
            } else if file.ends_with(".svg") {
                // (Glyphs drawn in black are made white, so that every glyph can be tinted.)
                let b = String::from_utf8_lossy(&b).replace("fill=\"black\"", "fill=\"white\"").replace("stroke=\"black\"", "stroke=\"white\"").into_bytes();
                egui_extras::image::load_svg_bytes_with_size(&b, Some(egui::SizeHint::Scale(4.0.into()))).ok()?
            } else {
                let i = image::load_from_memory(&b).ok()?.to_rgba8();
                egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], &i)
            };
            Some(ctx.load_texture(format!("skin:{name}"), img, egui::TextureOptions::LINEAR))
        });
        s.borrow_mut().insert(name.to_string(), tex.clone());
        tex
    })
}

/// rekordbox's artwork in the Light appearance: the code recolours each SVG with
/// `Drawable::replaceColour` from a fixed colour to a colour id's value (e.g. the panel tiles
/// in `MainComponent::lookAndFeelChanged` 0x100979750: #282828 -> 0x03000005, white ->
/// 0x03000006). `how` is "light" (every grey to its `theme::ramp` value, "white" and "black"
/// included) or "swap:RRGGBB>RRGGBB,…" (the pairs traced for that image).
fn recolour_svg(s: &str, how: &str) -> String {
    let swaps: Vec<(u32, u32)> = how.strip_prefix("swap:").map(|l| l.split(',').filter_map(|p| {
        let (a, b) = p.split_once('>')?;
        Some((u32::from_str_radix(a, 16).ok()?, u32::from_str_radix(b, 16).ok()?))
    }).collect()).unwrap_or_default();
    let map = |v: u32| -> u32 {
        if how == "light" { crate::theme::ramp(v) } else { swaps.iter().find(|(a, _)| *a == v).map(|(_, b)| *b).unwrap_or(v) }
    };
    // Only literal paint values are colours. Gradients and masks may have six-digit
    // hexadecimal IDs; rewriting their href or url(#...) would disconnect the artwork.
    let mut out = String::with_capacity(s.len());
    let mut copied = 0;
    for (start, _) in s.match_indices(['#', 'w', 'b']) {
        let before = s[..start].trim_end();
        let (property, end) = if let Some(quote) = before.chars().next_back().filter(|c| matches!(c, '\'' | '"')) {
            let Some(name) = before[..before.len() - 1].trim_end().strip_suffix('=') else { continue };
            (name, Some(quote))
        } else if let Some(name) = before.strip_suffix(':') {
            (name, None)
        } else {
            continue;
        };
        let property = property.trim_end().rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '-').next();
        if !matches!(property, Some("fill" | "stroke" | "color" | "stop-color" | "flood-color" | "lighting-color")) {
            continue;
        }
        let value = &s[start..];
        let (len, rgb) = if let Some(hex) = value.strip_prefix('#').and_then(|v| v.get(..6)) {
            let Ok(rgb) = u32::from_str_radix(hex, 16) else { continue };
            (7, rgb)
        } else if value.starts_with("white") {
            (5, 0xFFFFFF)
        } else if value.starts_with("black") {
            (5, 0)
        } else {
            continue;
        };
        let terminator = value[len..].trim_start().chars().next();
        if end.is_some() && terminator != end || end.is_none() && !matches!(terminator, Some(';' | '}' | '\'' | '"' | '!')) {
            continue;
        }
        if map(rgb) != rgb {
            use std::fmt::Write;
            out.push_str(&s[copied..start]);
            write!(out, "#{:06X}", map(rgb)).unwrap();
            copied = start + len;
        }
    }
    out.push_str(&s[copied..]);
    out
}

/// Draw a skin image into `r`, tinted. Returns false when it is not available.
pub(crate) fn skin_at(p: &Painter, name: &str, r: Rect, tint: Color32) -> bool {
    match skin(p.ctx(), name) {
        Some(t) => {
            p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
            true
        }
        None => false,
    }
}

// ------------------------------------------------------------------ deck geometry

/// The deck variants the pieces of `ui_perf.rs` know.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Dk {
    H2,
    V2,
    H4,
    V4,
    /// Header and overview only (Browse, and the horizontal layouts without pad and platter).
    Browse,
    Simple2,
    Simple4,
}

impl Dk {
    /// The compact sizes of the 4-deck layouts (30 pt buttons, 108 pt platter).
    pub fn compact(self) -> bool {
        matches!(self, Dk::H4 | Dk::V4)
    }
    pub fn has_lower(self) -> bool {
        matches!(self, Dk::H2 | Dk::V2 | Dk::H4 | Dk::V4)
    }
}

/// The variant in force: the one set by `with_dk`, else 2Deck / 4Deck Horizontal by `c4`.
pub(crate) fn dk(c4: bool) -> Dk {
    DK.with(|d| d.get()).unwrap_or(if c4 { Dk::H4 } else { Dk::H2 })
}

/// Run `f` with the deck pieces of `ui_perf.rs` laid out for `d`.
pub(crate) fn with_dk<R>(d: Dk, f: impl FnOnce() -> R) -> R {
    let prev = DK.with(|c| c.replace(Some(d)));
    let r = f();
    DK.with(|c| c.set(prev));
    r
}

/// JUCE font heights are ascent + descent; Arial's is 1.1172 em.
pub(crate) fn juce(h: f32) -> f32 {
    h / 1.1172
}

/// Arial regular at a JUCE font height.
pub(crate) fn arial(h: f32) -> FontId {
    FontId::new(juce(h), egui::FontFamily::Name("arial".into()))
}
/// Arial Bold at a JUCE font height.
pub(crate) fn arial_b(h: f32) -> FontId {
    caps(juce(h))
}

/// Header and overview of a deck (rekordbox's `TrackInfoView`), relative to the deck's
/// top-left corner; `w` is the deck's width.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeckGeo {
    pub dk: Dk,
    /// Height of the grey header band.
    pub hdr_h: f32,
    /// Height of header and overview together, before the rows the preferences add.
    pub info_h: f32,
    /// Top of the KEY SYNC / BEAT SYNC / MASTER block.
    pub y0: f32,
    /// Centre of the deck number.
    pub num: Pos2,
    pub art: Rect,
    /// Left edge of title and artist, and their centre lines.
    pub text_x: f32,
    pub title_y: f32,
    pub sub_y: f32,
    /// JUCE font heights of the title, of artist / BPM / key and of the times.
    pub title_f: f32,
    pub sub_f: f32,
    pub sub_bold: bool,
    pub time_f: f32,
    /// Right edge of the BPM, left edge of the key.
    pub bpm_right: f32,
    pub key_x: f32,
    pub time_y: f32,
    /// Right edges of the remaining and of the elapsed time, measured from the deck's right.
    pub remain_r: f32,
    pub elapsed_r: f32,
    /// Base line of the overview waveform and its full height.
    pub ov_base: f32,
    pub ov_h: f32,
    /// Height of the part with pads, loop box and platter (0: none).
    pub lower_h: f32,
}

pub(crate) fn deck_geo(d: Dk) -> DeckGeo {
    let at = |x: f32, y: f32, s: f32| Rect::from_min_size(pos2(x, y), vec2(s, s));
    match d {
        Dk::H2 | Dk::Simple2 => DeckGeo { dk: d, hdr_h: 54.0, info_h: 94.0, y0: 8.0, num: pos2(13.5, 13.75), art: at(25.0, 11.0, 34.0), text_x: 64.0, title_y: 19.0, sub_y: 38.0, title_f: 18.0, sub_f: 14.0, sub_bold: false, time_f: 16.0, bpm_right: 223.0, key_x: 228.0, time_y: 36.25, remain_r: 245.5, elapsed_r: 191.0, ov_base: 85.0, ov_h: 25.0, lower_h: if d == Dk::H2 { 130.0 } else { 0.0 } },
        Dk::Simple4 => DeckGeo { dk: d, hdr_h: 54.0, info_h: 93.0, y0: 8.0, num: pos2(13.5, 13.75), art: at(25.0, 11.0, 34.0), text_x: 64.0, title_y: 19.0, sub_y: 37.0, title_f: 18.0, sub_f: 14.0, sub_bold: false, time_f: 16.0, bpm_right: 223.0, key_x: 228.0, time_y: 36.25, remain_r: 245.5, elapsed_r: 191.0, ov_base: 85.0, ov_h: 25.0, lower_h: 0.0 },
        Dk::V2 => DeckGeo { dk: d, hdr_h: 50.0, info_h: 91.0, y0: 6.0, num: pos2(10.5, 10.75), art: at(25.0, 7.0, 34.0), text_x: 64.0, title_y: 15.0, sub_y: 36.0, title_f: 18.0, sub_f: 12.0, sub_bold: true, time_f: 16.0, bpm_right: 170.0, key_x: 175.0, time_y: 34.25, remain_r: 245.5, elapsed_r: 191.0, ov_base: 82.0, ov_h: 26.0, lower_h: 203.0 },
        Dk::H4 => DeckGeo { dk: d, hdr_h: 42.0, info_h: 73.0, y0: 2.0, num: pos2(13.5, 13.75), art: at(25.0, 7.0, 30.0), text_x: 60.0, title_y: 14.0, sub_y: 32.0, title_f: 16.0, sub_f: 12.0, sub_bold: false, time_f: 14.0, bpm_right: 202.0, key_x: 207.0, time_y: 31.5, remain_r: 240.0, elapsed_r: 192.5, ov_base: 64.5, ov_h: 16.5, lower_h: 112.0 },
        Dk::V4 => DeckGeo { dk: d, hdr_h: 50.0, info_h: 91.0, y0: 6.0, num: pos2(13.5, 13.75), art: at(25.0, 11.0, 30.0), text_x: 60.0, title_y: 18.0, sub_y: 36.0, title_f: 16.0, sub_f: 12.0, sub_bold: false, time_f: 14.0, bpm_right: 188.0, key_x: 193.0, time_y: 35.5, remain_r: 240.0, elapsed_r: 192.5, ov_base: 82.0, ov_h: 26.0, lower_h: 112.0 },
        Dk::Browse => DeckGeo { dk: d, hdr_h: 54.0, info_h: 91.0, y0: 6.0, num: pos2(13.5, 13.75), art: at(25.0, 11.0, 34.0), text_x: 64.0, title_y: 19.0, sub_y: 37.0, title_f: 18.0, sub_f: 14.0, sub_bold: false, time_f: 16.0, bpm_right: 223.0, key_x: 228.0, time_y: 35.25, remain_r: 245.5, elapsed_r: 191.0, ov_base: 86.0, ov_h: 26.0, lower_h: 0.0 },
    }
}

/// Where the lower part's pieces sit, relative to the top-left corner of the lower part.
/// `right`: a deck right of the mixer (its platter is at its left, the tab column at its right).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LowerGeo {
    /// The pad area (the pads, then the pad mode pull-down under them).
    pub pads: Rect,
    /// Columns and rows of pads showing, the pads' height and their pitch.
    pub cols: usize,
    pub rows: usize,
    pub pad_h: f32,
    pub pitch: f32,
    /// Top-left of the pad mode pull-down's row (its buttons hang from `pads`' outer edge).
    pub mode_y: f32,
    pub mode_h: f32,
    /// Top-left of the loop box (68 pt wide) and of the platter panel.
    pub loop_o: Pos2,
    pub panel_o: Pos2,
    /// Left edge of the 28 pt tab column.
    pub tabs_x: f32,
}

pub(crate) fn lower_geo(d: Dk, w: f32, right: bool) -> LowerGeo {
    // (panel width from its left edge to the mixer side of the deck, loop box top, pad rows)
    let (pw, panel_y, loop_y, cols, rows, pad_h, pitch, pads_y, mode_y, mode_h) = match d {
        Dk::V2 => (232.0, 0.0, 31.0, 1, 8, 19.0, 21.0, 10.0, 173.0, 20.0),
        Dk::H4 | Dk::V4 => (198.0, 0.0, 7.0, 2, 4, 16.0, 18.0, 6.0, 86.0, 18.0),
        _ => (210.0, 0.0, 10.0, 2, 4, 19.0, 21.0, 10.0, 100.0, 20.0),
    };
    let pads_h = if d == Dk::V2 { 160.0 } else { rows as f32 * pitch };
    if !right {
        let panel_x = w - pw;
        let loop_x = panel_x - if d == Dk::V2 { 80.0 } else { 78.0 };
        LowerGeo { pads: Rect::from_min_max(pos2(38.0, pads_y), pos2(loop_x - if d == Dk::V2 { 11.0 } else { 10.0 }, pads_y + pads_h)), cols, rows, pad_h, pitch, mode_y, mode_h, loop_o: pos2(loop_x, loop_y), panel_o: pos2(panel_x, panel_y), tabs_x: 0.0 }
    } else {
        // (The platter panel's own width: its SLIP / Q / MT column ends it.)
        let panel_w = match d {
            Dk::V2 => 222.0,
            Dk::H4 | Dk::V4 => 188.0,
            _ => 200.0,
        };
        let loop_x = 10.0 + panel_w + if d == Dk::V2 { 12.0 } else { 10.0 };
        LowerGeo { pads: Rect::from_min_max(pos2(loop_x + 68.0 + 10.0, pads_y), pos2(w - 28.0 - match d { Dk::V2 => 15.0, Dk::V4 => 11.0, _ => 12.0 }, pads_y + pads_h)), cols, rows, pad_h, pitch, mode_y, mode_h, loop_o: pos2(loop_x, loop_y), panel_o: pos2(10.0, panel_y), tabs_x: w - 28.0 }
    }
}

/// Where the eight pads of a deck are inside the pad area `r`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PadGrid {
    pub r: Rect,
    pub dk: Dk,
    pub cols: usize,
    pub pad_h: f32,
    pub pitch: f32,
    /// Top of the pad mode pull-down under the pads, from the pad area's top, and its height.
    pub mode_dy: f32,
    pub mode_h: f32,
}

/// `stems`: the STEMS pads, which share the area's height evenly instead of the fixed pitch
/// of the other pad modes (22.3 pt in 2Deck Horizontal, 20.25 pt in 2Deck Vertical, 20 pt in
/// the 4-deck layouts; the others 21 pt, or 18 pt with four decks).
pub(crate) fn pad_grid(d: Dk, r: Rect, stems: bool) -> PadGrid {
    let (cols, pad_h, pitch, mode_dy, mode_h) = match (d, stems) {
        (Dk::V2, false) => (1, 19.0, 21.0, 163.0, 20.0),
        (Dk::V2, true) => (1, 17.5, 20.25, 163.0, 20.0),
        (Dk::H4 | Dk::V4, false) => (2, 16.0, 18.0, 80.0, 18.0),
        (Dk::H4 | Dk::V4, true) => (2, 17.5, 20.0, 80.0, 18.0),
        (_, false) => (2, 19.0, 21.0, 90.0, 20.0),
        (_, true) => (2, 20.0, 22.33, 90.0, 20.0),
    };
    PadGrid { r, dk: d, cols, pad_h, pitch, mode_dy, mode_h }
}

impl PadGrid {
    /// Pad `row` (0 to 3) of column `c` (0 or 1); with one column the second four follow the
    /// first four.
    pub fn pad(&self, c: usize, row: usize) -> Rect {
        if self.cols == 1 {
            return Rect::from_min_size(pos2(self.r.min.x, self.r.min.y + (c * 4 + row) as f32 * self.pitch), vec2(self.r.width(), self.pad_h));
        }
        let col_w = ((self.r.width() - 2.0) / 2.0).floor();
        Rect::from_min_size(pos2(self.r.min.x + c as f32 * (col_w + 2.0), self.r.min.y + row as f32 * self.pitch), vec2(col_w, self.pad_h))
    }
    /// The pad mode pull-down, `w` wide: under the pads at the deck's outer side.
    pub fn mode_rect(&self, right: bool, w: f32) -> Rect {
        Rect::from_min_size(pos2(if right { self.r.max.x + if self.cols == 1 { 5.0 } else { 2.0 } - w } else { self.r.min.x }, self.r.min.y + self.mode_dy), vec2(w, self.mode_h))
    }
    /// The pads' scroll bar and its handle for `page` (0 or 1): 3 pt under the pads, half
    /// their width; in 2Deck Vertical beside them, half their height.
    pub fn scroll_bar(&self, page: u8) -> (Rect, Rect) {
        if self.cols == 1 {
            let track = Rect::from_min_size(pos2(self.r.max.x + 2.0, self.r.min.y), vec2(3.0, 160.0));
            let thumb = Rect::from_min_size(pos2(track.min.x, track.min.y + if page == 0 { 0.0 } else { 83.0 }), vec2(3.0, 77.0));
            return (track, thumb);
        }
        let col_w = ((self.r.width() - 2.0) / 2.0).floor();
        let y = self.r.min.y + 3.0 * self.pitch + self.pad_h + 4.0;
        let track = Rect::from_min_size(pos2(self.r.min.x, y), vec2(self.r.width(), 3.0));
        let thumb = Rect::from_min_size(pos2(self.r.min.x + if page == 0 { 0.0 } else { self.r.width() - col_w - 2.0 }, y), vec2(col_w + 2.0, 3.0));
        (track, thumb)
    }
}

/// rekordbox's `VirtualDeckPanel` numbers (its `resized` and `setDeckPanelLayout`), relative
/// to the panel's top-left corner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlatterGeo {
    /// The platter's square and the diameter its rings are fractions of (size - 10).
    pub plat: Rect,
    pub d: f32,
    /// CUE and PLAY/PAUSE: their centres and their size.
    pub cue_c: egui::Vec2,
    pub play_c: egui::Vec2,
    pub btn: f32,
    /// SLIP, Q and MT: left edge and the three tops (30 x 18 each).
    pub col_x: f32,
    pub ys: [f32; 3],
    /// The play mode pull-down (INT): its top.
    pub int_y: f32,
    /// Inside the platter, relative to its square: BPM value, tempo %, tempo range, the tempo
    /// - / + buttons and RESET; JUCE font heights of the BPM and of the small texts.
    pub bpm: Rect,
    pub tempo: Rect,
    pub pct_w: f32,
    pub range_w: f32,
    pub minus: Rect,
    pub plus: Rect,
    pub reset: Rect,
    pub bpm_f: f32,
    pub small_f: f32,
}

pub(crate) fn platter_geo(d: Dk) -> PlatterGeo {
    let r = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x, y), vec2(w, h));
    match d {
        Dk::V2 => PlatterGeo { plat: r(40.0, 26.0, 150.0, 150.0), d: 140.0, cue_c: vec2(20.0, 107.0), play_c: vec2(20.0, 153.0), btn: 36.0, col_x: 190.0, ys: [31.0, 59.0, 153.0], int_y: 31.0, bpm: r(40.0, 53.0, 70.0, 26.0), tempo: r(20.0, 81.0, 42.0, 15.0), pct_w: 16.0, range_w: 40.0, minus: r(55.0, 102.0, 15.0, 15.0), plus: r(80.0, 102.0, 15.0, 15.0), reset: r(55.0, 102.0, 40.0, 15.0), bpm_f: 26.0, small_f: 14.0 },
        Dk::H4 | Dk::V4 => PlatterGeo { plat: r(45.0, 2.0, 108.0, 108.0), d: 98.0, cue_c: vec2(20.0, 48.0), play_c: vec2(20.0, 84.0), btn: 30.0, col_x: 158.0, ys: [7.0, 35.0, 87.0], int_y: 7.0, bpm: r(28.0, 36.0, 54.0, 23.0), tempo: r(18.0, 57.0, 26.0, 11.0), pct_w: 8.0, range_w: 32.0, minus: r(37.0, 70.0, 13.0, 13.0), plus: r(58.0, 70.0, 13.0, 13.0), reset: r(37.0, 70.0, 34.0, 13.0), bpm_f: 20.0, small_f: 10.0 },
        _ => PlatterGeo { plat: r(45.0, 5.0, 120.0, 120.0), d: 110.0, cue_c: vec2(20.0, 56.0), play_c: vec2(20.0, 102.0), btn: 36.0, col_x: 170.0, ys: [10.0, 38.0, 102.0], int_y: 10.0, bpm: r(33.0, 41.0, 54.0, 23.0), tempo: r(23.0, 64.0, 26.0, 11.0), pct_w: 8.0, range_w: 32.0, minus: r(43.0, 80.0, 13.0, 13.0), plus: r(64.0, 80.0, 13.0, 13.0), reset: r(43.0, 80.0, 34.0, 13.0), bpm_f: 20.0, small_f: 10.0 },
    }
}

// ------------------------------------------------------------------ mixer geometry

/// One channel strip of the mixer, relative to the mixer panel's top-left corner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Strip {
    /// Deck (0 = deck 1).
    pub deck: usize,
    /// x of the HIGH / MID / LOW knobs, of the TRIM knob, of CUE and the fader, and the left
    /// edge of the 2 pt level meter.
    pub eq_x: f32,
    pub trim_x: f32,
    pub fader_x: f32,
    pub meter_x: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct MixGeo {
    pub strips: Vec<Strip>,
    /// The deck number above each strip (4-deck layouts).
    pub num_y: Option<f32>,
    /// Knob centres (TRIM, then HIGH / MID / LOW) and how far under a knob its label sits.
    pub trim_y: f32,
    pub eq_y: [f32; 3],
    pub label_dy: f32,
    /// CUE: its centre line (32 x 18 pt).
    pub cue_y: f32,
    /// The fader's travel, and the STEM button's centre line.
    pub fader: (f32, f32),
    pub stem_y: f32,
    /// The level meter's top and bottom.
    pub meter: (f32, f32),
}

pub(crate) fn mix_geo(d: Dk) -> MixGeo {
    match d {
        // 2Deck Vertical (248 x 147): TRIM, CUE and the fader in an outer column, the EQ in an
        // inner one, the meters at the panel's edges.
        Dk::V2 => MixGeo {
            strips: vec![Strip { deck: 0, eq_x: 82.0, trim_x: 44.0, fader_x: 44.0, meter_x: 16.0 }, Strip { deck: 1, eq_x: 167.0, trim_x: 205.0, fader_x: 205.0, meter_x: 231.0 }],
            num_y: None,
            trim_y: 20.0,
            eq_y: [20.0, 55.0, 90.0],
            label_dy: 12.75,
            cue_y: 55.0,
            fader: (74.0, 117.0),
            stem_y: 131.0,
            meter: (8.0, 117.0),
        },
        // 4Deck Vertical (248 x 234): four strips 62 pt apart, in rekordbox's order 3, 1, 2, 4.
        Dk::V4 => MixGeo {
            strips: [2usize, 0, 1, 3].iter().enumerate().map(|(i, n)| { let x = 33.0 + 62.0 * i as f32; Strip { deck: *n, eq_x: x, trim_x: x, fader_x: x, meter_x: x - 20.0 } }).collect(),
            num_y: None,
            trim_y: 16.0,
            eq_y: [48.0, 80.0, 112.0],
            label_dy: 12.75,
            cue_y: 142.0,
            fader: (160.0, 198.5),
            stem_y: 215.0,
            meter: (4.0, 144.0),
        },
        // 4Deck Horizontal (190 x 379): four strips 44 pt apart.
        Dk::H4 => MixGeo {
            strips: [2usize, 0, 1, 3].iter().enumerate().map(|(i, n)| { let x = 31.0 + 44.0 * i as f32; Strip { deck: *n, eq_x: x, trim_x: x, fader_x: x, meter_x: x - 20.0 } }).collect(),
            num_y: Some(14.0),
            trim_y: 42.0,
            eq_y: [82.0, 122.0, 162.0],
            label_dy: 13.25,
            cue_y: 198.0,
            fader: (226.0, 343.5),
            stem_y: 362.0,
            meter: (30.0, 187.0),
        },
        // 2Deck Horizontal (106 x 227): the two strips, the meters between them.
        _ => MixGeo {
            strips: vec![Strip { deck: 0, eq_x: 27.0, trim_x: 27.0, fader_x: 27.0, meter_x: 45.0 }, Strip { deck: 1, eq_x: 79.0, trim_x: 79.0, fader_x: 79.0, meter_x: 59.0 }],
            num_y: None,
            trim_y: 20.0,
            eq_y: [53.0, 86.0, 119.0],
            label_dy: 13.25,
            cue_y: 155.0,
            fader: (173.0, 220.5),
            stem_y: 234.5,
            meter: (8.0, 137.0),
        },
    }
}

// ------------------------------------------------------------------ overview with its rows

/// The whole-track waveform of a Performance deck with the rows above and under it: the vocal
/// row (7 pt, when shown), the waveform, the phrase row (5 pt, or 16 pt with the names).
/// `o`: the deck's top-left corner, `w` its width. Returns nothing; the rows' heights are
/// `vocal_row_h` and `phrase_row_h`.
pub(crate) fn overview(app: &mut App, ui: &mut Ui, o: Pos2, w: f32, g: &DeckGeo, pos: f64) {
    let p = ui.painter().clone();
    let (voc, phr) = (vocal_row_h(app), phrase_row_h(app));
    let dur = app.deck.duration.max(0.001);
    // The black box: 2 pt of the header's grey stay at both sides.
    let bx = Rect::from_min_max(pos2(o.x + 2.0, o.y + g.hdr_h), pos2(o.x + w - 2.0, o.y + g.info_h + voc + phr + 3.0));
    p.rect_filled(bx, 0.0, BG);
    let (x0, x1) = (o.x + 4.0, o.x + w - 4.0);
    let width = x1 - x0;
    let base = o.y + g.ov_base + voc;
    let full_h = g.ov_h;
    let x_of = |t: f64| x0 + (t / dur) as f32 * width;
    let area = Rect::from_min_max(pos2(x0, base - full_h), pos2(x1, base));
    if !crate::ui_player::paint_deck_overview(app, &p, area, dur, pos) && app.deck.track.is_some() {
        p.text(pos2((x0 + x1) / 2.0, base - full_h / 2.0), Align2::CENTER_CENTER, "Analyzing…", FontId::proportional(11.0), DIM.c());
    }
    // The same track can be on two decks using different models (e.g. Retry CPU).
    // Show measured segment progress only for the requested track and model.
    if app.separating_stems() {
        if let Some((_, progress)) = app.stem.separating.filter(|(_, progress)| *progress > 0.0) {
            p.rect_filled(Rect::from_min_max(pos2(x0, base + 3.0), pos2(x0 + width * progress.clamp(0.0, 1.0), base + 5.0)), 0.0, hex(0x077DE1));
        }
    }
    // Vocal row: where the vocal is, brighter where it is stronger.
    if voc > 0.0 {
        if let Some(v) = &app.deck.vocal {
            let (step, levels) = (v.0, &v.1);
            let n = width.max(1.0) as usize;
            let (y0, y1) = (o.y + g.hdr_h + 2.0, o.y + g.hdr_h + 5.0);
            let mut bars = crate::ui_kit::Bars::new();
            for i in 0..n {
                let t0 = i as f64 / n as f64 * dur;
                let t1 = (i + 1) as f64 / n as f64 * dur;
                let (a, b) = ((t0 / step) as usize, ((t1 / step) as usize).max((t0 / step) as usize + 1));
                let lvl = levels.get(a.min(levels.len())..b.min(levels.len())).and_then(|s| s.iter().max().copied()).unwrap_or(0);
                if lvl > 0 {
                    bars.column(x0 + i as f32 + 0.5, y0, y1, hex(0x4FC3D9).gamma_multiply(0.35 + 0.65 * (lvl.min(4) as f32 / 4.0)));
                }
            }
            bars.paint(&p);
        }
    }
    // Phrase row: one block per phrase, with its name when the row is the tall one.
    if phr > 0.0 {
        if let (Some(st), false) = (&app.deck.phrases, app.deck.beats.is_empty()) {
            let y0 = o.y + g.info_h + voc - 2.0;
            let beat_t = |b: u16| app.deck.beats.get((b.max(1) - 1) as usize).map(|x| x.time).unwrap_or(dur);
            for (k, ph) in st.phrases.iter().enumerate() {
                let t0 = beat_t(ph.beat);
                let t1 = st.phrases.get(k + 1).map(|n| beat_t(n.beat)).unwrap_or_else(|| beat_t(st.end_beat));
                let r = Rect::from_min_max(pos2(x_of(t0) + 0.5, y0), pos2((x_of(t1) - 0.5).max(x_of(t0) + 1.0), y0 + phr));
                let col = hex(crate::phrase::color(st.mood, ph.kind));
                p.rect_filled(r, 0.0, col);
                if phr > 10.0 && r.width() > 14.0 {
                    p.with_clip_rect(r).text(pos2(r.min.x + 3.0, r.center().y), Align2::LEFT_CENTER, crate::phrase::label(st.mood, ph.kind), arial(11.0), Color32::BLACK);
                }
            }
        }
    }
    if app.deck.track.is_some() {
        // Minute marks: white, 1 x 2 pt, 4 pt under the base line.
        let minute = 60.0 / dur * width as f64;
        let mut k = 1.0;
        while k * minute < width as f64 + 0.5 {
            let x = (x0 + (k * minute) as f32 - 0.5).round();
            p.rect_filled(Rect::from_min_max(pos2(x, base + 4.0), pos2(x + 1.0, base + 6.0)), 0.0, WHITE);
            k += 1.0;
        }
        let top = base - full_h - 3.0;
        for c in &app.deck.cues {
            if let Some(e) = c.end.filter(|e| *e > c.start) {
                p.rect_filled(Rect::from_min_max(pos2(x_of(c.start), top), pos2(x_of(e).max(x_of(c.start) + 1.0), base)), 0.0, LOOP.gamma_multiply(0.25));
            }
        }
        for c in &app.deck.cues {
            let x = x_of(c.start);
            if c.kind == CueKind::Memory {
                let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
                crate::ui_player::tri_up(&p, pos2(x, base + 0.5), 4.0, col);
            } else {
                let col = hotcue_color(c.color);
                p.line_segment([pos2(x, top), pos2(x, base)], Stroke::new(1.0_f32, col));
                let lr = Rect::from_min_size(pos2(x - 5.0, top), vec2(10.0, 9.0));
                p.rect_filled(lr, 0.0, col);
                p.text(lr.center(), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), arial_b(10.0), Color32::BLACK);
            }
        }
        // Playhead: a 1 pt line over the waveform's height.
        let px = x_of(pos);
        p.rect_filled(Rect::from_min_max(pos2(px - 0.5, top), pos2(px + 0.5, base)), 0.0, gy(0xC8C8C8));
        // The current cue (rekordbox's `CurrentCueImageUp`): 10.7 pt wide, from 4.5 pt above
        // the base line to 4 pt under it.
        let cx = x_of(app.deck.cue_point);
        p.with_clip_rect(bx).add(egui::Shape::convex_polygon(vec![pos2(cx, base - 4.5), pos2(cx + 5.35, base + 4.0), pos2(cx - 5.35, base + 4.0)], CUE_MARK, Stroke::NONE));
    }
    let hit = Rect::from_min_max(pos2(x0, base - full_h - 3.0), pos2(x1, base + 6.0));
    let resp = ui.interact(hit, ui.id().with("overview"), Sense::click_and_drag());
    if app.prefs.tooltips && resp.hovered() {
        if app.separating_stems() {
            if let Some(status) = app.stem.separation_status() { resp.clone().on_hover_text(status); }
        } else if app.queued_stems() {
            resp.clone().on_hover_text("Stem separation queued for this deck");
        }
    }
    if (resp.clicked() || resp.dragged()) && app.deck.audio.is_some() {
        if let Some(pp) = resp.interact_pointer_pos() {
            // Needle search, in rekordbox's 400 steps across the waveform.
            let step = (((pp.x - x0) / width).clamp(0.0, 1.0) * 400.0).round() as f64 / 400.0;
            let t = step * dur;
            app.seek(t);
            if !app.playing() {
                app.deck.cue_point = app.quantized(t);
                app.seek(app.deck.cue_point);
            }
        }
    }
}

/// The deck's tab column (28 pt): the pad view's mark in the upper half, the grid view's in
/// the lower one (rekordbox's `padIcon` and `gridIcon`), the half showing in the panel grey.
/// Returns true when the grid half was clicked.
pub(crate) fn tab_column(ui: &mut Ui, r: Rect, grid_on: bool) -> Option<bool> {
    let p = ui.painter().clone();
    let half = r.height() / 2.0;
    let top = Rect::from_min_size(r.min, vec2(r.width(), half));
    let bot = Rect::from_min_max(pos2(r.min.x, r.min.y + half), r.max);
    p.rect_filled(top, 0.0, if grid_on { BG1 } else { BG2 });
    p.rect_filled(bot, 0.0, if grid_on { BG2 } else { BG1 });
    let mut out = None;
    for (i, (rr, name)) in [(top, "padIcon.svg"), (bot, "gridIcon.svg")].into_iter().enumerate() {
        let resp = ui.interact(rr, ui.id().with(("decktab", i)), Sense::click());
        let on = (i == 1) == grid_on;
        let col = if on { WHITE.c() } else { crate::ui_feel::hl(&resp, gy(0x646464), gy(0x969696)) };
        let ir = Rect::from_center_size(rr.center() + vec2(0.0, 0.5), vec2(12.0, 13.0));
        if !skin_at(&p, name, ir, col) {
            if i == 0 {
                for (dx, dy) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
                    p.rect_filled(Rect::from_center_size(rr.center() + vec2(dx, dy), vec2(5.0, 5.0)), 0.5, col);
                }
            } else {
                for (dx, h) in [(-5.4, 9.6), (-3.0, 9.6), (0.0, 12.0), (3.0, 9.6), (5.4, 9.6)] {
                    p.rect_filled(Rect::from_center_size(rr.center() + vec2(dx, 0.0), vec2(if dx == 0.0 { 2.4 } else { 1.2 }, h)), 0.0, col);
                }
            }
        }
        if resp.on_hover_text(if i == 0 { "PAD" } else { "GRID" }).clicked() {
            out = Some(i == 1);
        }
    }
    out
}

// ------------------------------------------------------------------ the GRID tab's panel

/// The deck's GRID panel in Performance (`djplay::GridViewPanel`, `resized` 0x102331cc8),
/// drawn over the area of the pads and the loop box (`MemoryCuePanel::setPanelType(2)`;
/// JUCE clips it to that area). Positions are rekordbox's, from the panel's top left:
/// x0 = (16 or 0) - 45 (- 10 in 4Deck Vertical), rows at 256 / 273 / 304 / 335 + y0 with
/// y0 = -253 in 4Deck Horizontal and -248 elsewhere. Columns: A-GAIN ("Set", the knob, PLAY),
/// CENTER / BPM / TAP, the four grid moves, the four interval changes, x2 / 1/2 / whole /
/// from here, then undo / redo, the analysis lock and the metronome; PHRASE EDIT at x0 + 452
/// (in 2Deck Vertical under the grid, at y0 + 386). The buttons are rekordbox's skin images
/// (read from the installed app); labels Arial 12 #969696, buttons Arial 14 #969696 on #191919.
pub(crate) fn grid_panel(app: &mut App, ui: &mut Ui, area: Rect, n: u8, right: bool, d: Dk) {
    let p = ui.painter().with_clip_rect(area.intersect(ui.clip_rect()));
    p.rect_filled(area, 0.0, gy(0x1E1E1E));
    let loaded = app.deck.track.is_some();
    let locked = app.deck.track.and_then(|t| app.track(t)).map(|t| t.grid_locked).unwrap_or(false);
    let has_grid = !app.deck.grid.is_empty();
    let x0 = if right { 16.0 } else { 0.0 } - 45.0 + if d == Dk::V4 { -10.0 } else { 0.0 };
    let y0 = if d == Dk::H4 { -253.0 } else { -248.0 };
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(area.min + vec2(x0 + x, y0 + y), vec2(w, h));
    let grey = gy(0x969696);
    let label = |p: &Painter, r: Rect, t: &str| {
        p.text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, t, arial(12.0 * 0.87), grey);
    };
    label(&p, at(48.0, 256.0, 80.0, 11.0), "GRID EDIT");
    let enabled = loaded && !locked;
    // A skin button: the image, dimmed while it cannot be used; returns the click (and true
    // while the press repeats, after 1 s every 100 ms as rekordbox's `setRepeatSpeed`).
    let skin_btn = |ui: &mut Ui, app: &App, r: Rect, name: &str, id: &str, tipt: &str, on: bool| -> (bool, bool) {
        let resp = ui.interact(r, ui.id().with(("gridp", n, id)), Sense::click());
        let tint = if on { Color32::WHITE } else { Color32::from_white_alpha(77) };
        if !skin_at(&p, name, r, tint) {
            p.rect_filled(r, 2.0, gy(0x191919));
        }
        if let Some(c) = crate::ui_feel::hover_fill_if(&resp, on, Color32::from_white_alpha(25)) {
            p.rect_filled(r, 2.0, c);
        }
        let held = on && resp.is_pointer_button_down_on() && {
            let now = ui.input(|i| i.time);
            let t0 = ui.input(|i| i.pointer.press_start_time()).unwrap_or(now);
            ui.ctx().request_repaint();
            let key = resp.id.with("rep");
            let last = ui.ctx().data(|dd| dd.get_temp::<f64>(key)).unwrap_or(0.0);
            if now - t0 > 1.0 && now - last >= 0.1 {
                ui.ctx().data_mut(|dd| dd.insert_temp(key, now));
                true
            } else {
                false
            }
        };
        (tip(resp, app, tipt).clicked() && on, held)
    };
    // Column 1: Set (the 1.1 Bars), A-GAIN, PLAY/PAUSE.
    let tb = |ui: &mut Ui, p: &Painter, r: Rect, id: &str, t: &str, on: bool| -> egui::Response {
        let resp = ui.interact(r, ui.id().with(("gridt", n, id)), Sense::click());
        p.rect_filled(r, 2.0, crate::ui_feel::hl_if(&resp, on, gy(0x191919), lighten(gy(0x191919))));
        p.text(r.center(), Align2::CENTER_CENTER, t, arial(14.0 * 0.87), if on { grey } else { gy(0x505050) });
        resp
    };
    if tb(ui, &p, at(50.0, 273.0, 50.0, 26.0), "set11", "Set", enabled && has_grid).on_hover_text("Set the 1.1 Bars position to the current position.").clicked() && enabled && has_grid {
        app.grid_set_downbeat();
    }
    let kc = at(63.0, 306.0, 24.0, 24.0).center();
    let db = app.gain_db();
    let orange = hex(0xFFA300);
    if let Some(v) = crate::ui_perf::knob(ui, kc, ("gridagain", n), crate::deck::again_pos(db), crate::deck::again_pos(0.0), orange) {
        if loaded {
            let db = crate::deck::again_db(v).max(-crate::player_ext::GAIN_RANGE_DB);
            app.set_gain_db(db, true);
        }
    }
    p.text(at(50.0, 326.0, 50.0, 9.0).center(), Align2::CENTER_CENTER, "A-GAIN", arial(9.0 * 0.87), grey);
    let pr = at(62.0, 335.0, 26.0, 26.0);
    let playing = app.playing();
    let presp = ui.interact(pr, ui.id().with(("gridplay", n)), Sense::click());
    // (Light: `PlayCuePanel::replaceColorTheme` 0x10000d404 swaps #282828 -> 0x03000090 and
    // white -> 0x0300008E on the lit images, #1E1E1E -> 0x03000091 and #323232 -> 0x0300008F
    // on the off one.)
    let img = match (playing, loaded, is_light()) {
        (true, _, false) => "pauseBtn.svg",
        (false, true, false) => "playBtn.svg",
        (false, false, false) => "playBtn_off.svg",
        (true, _, true) => "pauseBtn.svg#swap:282828>C8C8C8,FFFFFF>000000",
        (false, true, true) => "playBtn.svg#swap:282828>C8C8C8,FFFFFF>000000",
        (false, false, true) => "playBtn_off.svg#swap:1E1E1E>F0F0F0,323232>787878",
    };
    if !skin_at(&p, img, pr, Color32::WHITE) {
        p.circle_stroke(pr.center(), 12.0, Stroke::new(1.0_f32, hex(0x2DD728)));
    }
    if tip(presp, app, "Play/Pause").clicked() && loaded {
        app.play_pause_synced();
    }
    // Column 2: CENTER, the BPM, TAP.
    if skin_btn(ui, app, at(105.0, 273.0, 50.0, 26.0), "Grid_CenterButton.svg", "center", "Set a beat to the current position.", enabled).0 {
        app.grid_center();
    }
    let br = at(105.0, 304.0, 50.0, 26.0);
    p.rect_filled(br, 2.0, gy(0x0F0F0F));
    if !app.deck.x.bpm_editing {
        let b = app.current_bpm();
        app.deck.bpm_text = if loaded && b > 0.0 { format!("{b:.2}") } else { String::new() };
    }
    let mut txt = app.deck.bpm_text.clone();
    let resp = ui.put(br.shrink2(vec2(2.0, 5.0)), egui::TextEdit::singleline(&mut txt).horizontal_align(egui::Align::Center).frame(false).font(arial(12.0 * 0.87)).text_color(Color32::WHITE).interactive(enabled));
    app.deck.bpm_text = txt;
    if resp.has_focus() {
        app.deck.x.bpm_editing = true;
    }
    if resp.lost_focus() {
        app.deck.x.bpm_editing = false;
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let t = app.deck.bpm_text.clone();
            app.grid_enter_bpm(&t);
        }
    }
    if tb(ui, &p, at(105.0, 335.0, 50.0, 26.0), "tap", "TAP", enabled).on_hover_text("Set BPM with the tapping interval").clicked() && enabled {
        app.grid_tap();
    }
    // The grid moves (1 ms, fast 3 ms; held 10 / 30) and the interval changes (0.01 BPM, fast
    // 0.03; held 0.10 / 0.30): `GridViewPanel::buttonClicked`.
    let moves: [(f32, &str, f64, &str); 4] = [(160.0, "Grid_Button_MoveLeftFast.svg", -3.0, "Shift Beatgrid left (fast)"), (215.0, "Grid_Button_MoveLeft.svg", -1.0, "Shift Beatgrid left"), (270.0, "Grid_Button_MoveRight.svg", 1.0, "Shift Beatgrid right"), (325.0, "Grid_Button_MoveRightFast.svg", 3.0, "Shift Beatgrid right (fast)")];
    for (x, name, ms, t) in moves {
        let (c, held) = skin_btn(ui, app, at(x, 273.0, 50.0, 26.0), name, name, t, enabled);
        if c || held {
            app.grid_shift(ms * if held { 10.0 } else { 1.0 } / 1000.0);
        }
    }
    let ints: [(f32, &str, f64, &str); 4] = [(160.0, "Grid_Button_ShrinkIntvlFast.svg", 3.0, "Shrink beat intervals (fast)"), (215.0, "Grid_Button_ShrinkIntvl.svg", 1.0, "Shrink beat intervals"), (270.0, "Grid_Button_ExpandIntvl.svg", -1.0, "Expand beat intervals"), (325.0, "Grid_Button_ExpandIntvlFast.svg", -3.0, "Expand beat intervals (fast)")];
    for (x, name, steps, t) in ints {
        let (c, held) = skin_btn(ui, app, at(x, 304.0, 50.0, 26.0), name, name, t, enabled);
        if c || held {
            let k = steps * if held { 10.0 } else { 1.0 } * 0.01;
            app.grid_scale_bpm(|v| ((v + k) * 100.0).round() / 100.0);
        }
    }
    if skin_btn(ui, app, at(160.0, 335.0, 50.0, 26.0), "grid_beat2timesBtn.svg", "x2", "Double the BPM value", enabled).0 {
        app.grid_double_half(true);
    }
    if skin_btn(ui, app, at(215.0, 335.0, 50.0, 26.0), "grid_beatHalfBtn.svg", "x05", "Halve the BPM value", enabled).0 {
        app.grid_double_half(false);
    }
    let fc = app.prefs.grid_from_current;
    if skin_btn(ui, app, at(270.0, 335.0, 50.0, 26.0), "Grid_Button_AdjOnWholeTrack.svg", "whole", "Adjust whole Beatgrid", enabled && fc).0 {
        app.prefs.grid_from_current = false;
        app.save_prefs();
    }
    if skin_btn(ui, app, at(325.0, 335.0, 50.0, 26.0), "Grid_Button_AdjFromCurPos.svg", "fromcur", "Adjust partial Beatgrid", enabled && !fc).0 {
        app.prefs.grid_from_current = true;
        app.save_prefs();
    }
    // Undo / redo, the analysis lock, the metronome and its volume.
    if skin_btn(ui, app, at(381.0, 273.0, 24.0, 26.0), "grid_unDoBtn.svg", "undo", "Undo", loaded && !app.deck.grid_undo.is_empty()).0 {
        app.grid_undo();
    }
    if skin_btn(ui, app, at(407.0, 273.0, 24.0, 26.0), "grid_reDoBtn.svg", "redo", "Redo", loaded && !app.deck.grid_redo.is_empty()).0 {
        app.grid_redo();
    }
    let lr = at(381.0, 304.0, 50.0, 26.0);
    let lresp = ui.interact(lr, ui.id().with(("gridlock", n)), Sense::click());
    p.rect_filled(lr, 2.0, gy(0x191919));
    skin_at(&p, if locked { "grid_key_lockbtn_on.svg" } else { "grid_key_lockbtn_off.svg" }, Rect::from_center_size(lr.center(), vec2(12.0, 16.0)), Color32::WHITE);
    if tip(lresp, app, if locked { "Track Analysis, Key Analysis, and Beatgrid Adjustment are locked." } else { "Enable/Disable Track Analysis, Key Analysis, and Beatgrid Adjustment." }).clicked() && loaded {
        if let Some(id) = app.deck.track {
            if let Some(t) = app.track_mut(id) {
                t.grid_locked = !t.grid_locked;
            }
            app.save_track(id);
        }
    }
    let met = app.deck.metronome;
    if skin_btn(ui, app, at(381.0, 335.0, 24.0, 26.0), if met { "grid_metroNomeBtn_On.svg" } else { "grid_metroNomeBtn_Off.svg" }, "met", "Metronome On/Off", loaded).0 {
        app.deck.metronome = !met;
        app.update_metronome();
    }
    let vol = app.prefs.metronome_vol.clamp(1, 3);
    let vr = at(407.0, 335.0, 24.0, 26.0);
    let vresp = ui.interact(vr, ui.id().with(("gridmetvol", n)), Sense::click());
    p.rect_filled(vr, 2.0, gy(0x191919));
    for k in 0..3u8 {
        let h = 4.0 + 3.0 * k as f32;
        p.rect_filled(Rect::from_min_size(pos2(vr.center().x - 6.0 + 4.5 * k as f32, vr.center().y + 6.0 - h), vec2(3.0, h)), 0.0, if k < vol { grey } else { gy(0x3C3C3C) });
    }
    if tip(vresp, app, "Metronome Volume").clicked() {
        app.set_metronome_volume(vol % 3 + 1);
    }
    // PHRASE EDIT: CUT, the phrase's type, CLEAR.
    let (px, py) = if d == Dk::V2 { (48.0, 386.0) } else { (452.0, 256.0) };
    label(&p, at(px, py, 80.0, 11.0), "PHRASE EDIT");
    let cur = app.current_phrase();
    let has = cur.is_some();
    let (cut_r, list_r, clear_r) = if d == Dk::V2 { (at(50.0, 403.0, 50.0, 26.0), at(105.0, 403.0, 64.0, 26.0), at(174.0, 403.0, 50.0, 26.0)) } else { (at(454.0, 273.0, 50.0, 26.0), at(454.0, 304.0, 64.0, 26.0), at(454.0, 335.0, 50.0, 26.0)) };
    if tb(ui, &p, cut_r, "phcut", "CUT", has).on_hover_text("Cut phrases.").clicked() && has {
        app.phrase_cut();
    }
    let lresp = ui.interact(list_r, ui.id().with(("gridphkind", n)), Sense::click());
    p.rect_filled(list_r, 2.0, gy(0x191919));
    if let Some((mood, kind)) = cur {
        p.with_clip_rect(list_r.shrink(2.0)).text(pos2(list_r.min.x + 5.0, list_r.center().y), Align2::LEFT_CENTER, crate::phrase::label(mood, kind), arial(12.0 * 0.87), grey);
    }
    skin_at(&p, "pullDown.svg", Rect::from_center_size(pos2(list_r.max.x - 9.0, list_r.center().y), vec2(11.0, 10.0)), if has { grey } else { gy(0x505050) });
    if let Some((mood, kind)) = cur {
        crate::ui_browser::dropdown(ui, &lresp, "gridphkindpop", |ui: &mut Ui| {
            for (k, name) in crate::phrase::kinds(mood) {
                if ui.selectable_label(crate::phrase::label(mood, kind) == *name, *name).clicked() {
                    app.phrase_set_kind(*k);
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
    }
    let edited = app.deck.x.phrases_edited;
    if tb(ui, &p, clear_r, "phclear", "CLEAR", edited).on_hover_text("Clear all the analyzed phrases. ").clicked() && edited {
        app.phrase_clear();
    }
}

// ------------------------------------------------------------------ the control strip (zoom)

/// rekordbox's `WaveViewArea::ControlView`: the strip along the left of the horizontal
/// waveforms (32 pt wide, 10 pt when closed) or along the top of the vertical ones (28 pt
/// tall, 10 pt when closed). Page 0: zoom in, RST, zoom out; page 1: matching and cut;
/// page 2: capture. `area`: the waveform area it lies over.
pub(crate) fn control_strip(app: &mut App, ui: &mut Ui, area: Rect, vertical: bool) {
    let p = ui.painter().clone();
    let open = app.prefs.perf_lay.ctl_open;
    let page = app.prefs.perf_lay.ctl_page.min(2);
    // (Groove Circuit shades its capture area while the strip shows the capture page.)
    app.gc.page = if open { page } else { 0 };
    let r = if vertical { Rect::from_min_size(area.min, vec2(area.width(), if open { 28.0 } else { 10.0 })) } else { Rect::from_min_size(area.min, vec2(if open { 32.0 } else { 10.0 }, area.height())) };
    let (w, h) = (r.width(), r.height());
    let at = |x: f32, y: f32, bw: f32, bh: f32| Rect::from_min_size(r.min + vec2(x, y), vec2(bw, bh));
    let icon = |ui: &mut Ui, br: Rect, id: &str, name: &str, tipt: &str| -> egui::Response {
        let resp = ui.interact(br.expand(2.0), ui.id().with(("wctl", id)), Sense::click());
        let col = if resp.hovered() { WHITE } else { TEXT };
        let img = Rect::from_center_size(br.center(), if name.contains("zoom") { vec2(14.0, 14.0) } else if name.starts_with("wave_m") || name.starts_with("wave_c") { vec2(15.0, 15.0) } else { vec2(10.0, 10.0) });
        if !skin_at(ui.painter(), name, img, col.c()) {
            let c = br.center();
            let pt = ui.painter();
            match name {
                "wave_zoom_in.svg" => crate::icons_player::zoom(pt, c, 1.0, col.c(), true),
                "wave_zoom_out.svg" => crate::icons_player::zoom(pt, c, 1.0, col.c(), false),
                "wave_up.svg" => { pt.add(egui::Shape::line(vec![c + vec2(4.0, 1.5), c + vec2(0.0, -2.5), c + vec2(-4.0, 1.5)], Stroke::new(1.0_f32, col))); }
                "wave_down.svg" => { pt.add(egui::Shape::line(vec![c + vec2(4.0, -1.5), c + vec2(0.0, 2.5), c + vec2(-4.0, -1.5)], Stroke::new(1.0_f32, col))); }
                "wave_left.svg" => { pt.add(egui::Shape::line(vec![c + vec2(2.0, -4.0), c + vec2(-2.0, 0.0), c + vec2(2.0, 4.0)], Stroke::new(1.0_f32, col))); }
                "wave_right.svg" => { pt.add(egui::Shape::line(vec![c + vec2(-2.0, -4.0), c + vec2(2.0, 0.0), c + vec2(-2.0, 4.0)], Stroke::new(1.0_f32, col))); }
                _ => { pt.circle_stroke(c, 5.0, Stroke::new(1.0_f32, col)); }
            }
        }
        resp.on_hover_text(tipt.to_string())
    };
    if !open {
        // Closed: only the button that opens it again.
        let br = if vertical { at((w - 18.0) / 2.0, h - 10.0, 18.0, 10.0) } else { at(w - 10.0, (h - 18.0) / 2.0, 10.0, 18.0) };
        p.rect_filled(br, 2.0, BTN);
        if icon(ui, br, "show", if vertical { "wave_down.svg" } else { "wave_right.svg" }, "Show").clicked() {
            app.prefs.perf_lay.ctl_open = true;
            app.save_prefs();
        }
        return;
    }
    p.rect_filled(r, 0.0, BG);
    // Page buttons at the strip's ends (previous at the top / left, next at the bottom / right).
    let (prev, next) = if vertical { (at(2.0, 4.0, 10.0, 10.0), at(w - 12.0, 4.0, 10.0, 10.0)) } else { (at(6.0, 2.0, 10.0, 10.0), at(6.0, h - 12.0, 10.0, 10.0)) };
    if icon(ui, prev, "prev", if vertical { "wave_left.svg" } else { "wave_up.svg" }, "Previous").clicked() {
        app.prefs.perf_lay.ctl_page = (page + 2) % 3;
        app.save_prefs();
    }
    if icon(ui, next, "next", if vertical { "wave_right.svg" } else { "wave_down.svg" }, "Next").clicked() {
        app.prefs.perf_lay.ctl_page = (page + 1) % 3;
        app.save_prefs();
    }
    // The button that closes the strip: at its inner edge, half way along.
    let hide = if vertical { at((w - 18.0) / 2.0, h - 10.0, 18.0, 10.0) } else { at(w - 10.0, (h - 18.0) / 2.0, 10.0, 18.0) };
    p.rect_filled(hide, 2.0, BTN);
    if icon(ui, hide, "hide", if vertical { "wave_up.svg" } else { "wave_left.svg" }, "Hide").clicked() {
        app.prefs.perf_lay.ctl_open = false;
        app.save_prefs();
    }
    match page {
        0 => {
            let (zin, rst, zout) = if vertical { let x = (w - 80.0) / 2.0; (at(x, 2.0, 14.0, 14.0), at((w - 18.0) / 2.0, 2.0, 18.0, 14.0), at(x + 66.0, 2.0, 14.0, 14.0)) } else { let y = ((h - 64.0) / 2.0).floor(); (at(4.0, y, 14.0, 14.0), at(2.0, ((h - 14.0) / 2.0).floor(), 18.0, 14.0), at(4.0, y + 50.0, 14.0, 14.0)) };
            if tip(icon(ui, zin, "zin", "wave_zoom_in.svg", "Zoom In"), app, "Zoom In").clicked() {
                zoom_step(app, -1);
            }
            let rresp = ui.interact(rst, ui.id().with(("wctl", "rst")), Sense::click());
            p.text(rst.center(), Align2::CENTER_CENTER, "RST", arial(9.5), WHITE.c());
            if tip(rresp, app, "Reset Zoom").clicked() {
                zoom_step(app, 0);
            }
            if tip(icon(ui, zout, "zout", "wave_zoom_out.svg", "Zoom Out"), app, "Zoom Out").clicked() {
                zoom_step(app, 1);
            }
        }
        1 => {
            // Matching (2-deck layouts only) and cut: rekordbox's Mix Point Link tools.
            let two = layout(app).decks() == 2;
            let cut = if vertical { at(if two { (w - 48.0) / 2.0 + 34.0 } else { (w - 14.0) / 2.0 }, 2.0, 14.0, 14.0) } else { at(4.0, if two { ((h - 44.0) / 2.0).floor() + 30.0 } else { ((h - 14.0) / 2.0).floor() }, 14.0, 14.0) };
            if two {
                // Matching (`UiPlayer::eventLikeButton`): the tracks on decks 1 and 2 are
                // registered as a good match of each other (Related Tracks' Matching), or no
                // longer; lit while they are one.
                let m = if vertical { at((w - 48.0) / 2.0, 2.0, 14.0, 14.0) } else { at(4.0, ((h - 44.0) / 2.0).floor(), 14.0, 14.0) };
                let (a, b) = (app.deck_of(0).track, app.deck_of(1).track);
                let lit = matches!((a, b), (Some(a), Some(b)) if app.is_match(a, b));
                if lit {
                    p.rect_filled(m.expand(2.0), 2.0, BLUE);
                }
                if icon(ui, m, "match", "wave_matching.svg", "Matching").clicked() {
                    match (a, b) {
                        (Some(a), Some(b)) => app.matching_set(a, b, !lit),
                        _ => app.toast_err("Invalid operation. Matching can be set when tracks are loaded to Deck 1 and 2."),
                    }
                }
            }
            // Cut: capture mode, where a loop or the slicer's domain is dragged from the
            // waveform onto a sampler slot; switching it on shows the sampler panel
            // (`MainComponent::notifyCaptureButtonEvent`).
            let on = capture_mode();
            if on {
                p.rect_filled(cut.expand(2.0), 2.0, BLUE);
            }
            if icon(ui, cut, "cut", "wave_cut.svg", "Capture").clicked() {
                CAPTURE_MODE.with(|c| c.set(!on));
                if !on && !app.prefs.perf_sampler {
                    app.prefs.perf_sampler = true;
                    app.save_prefs();
                }
            }
        }
        _ => {
            // DRUM CAPTURE (Groove Circuit, ui_groove.rs): a button per deck and the length.
            if !vertical && app.groove_visible() {
                crate::ui_groove::capture_buttons(app, ui, r.min, h);
            } else {
                let c = if vertical { at((w - 14.0) / 2.0, 2.0, 14.0, 14.0) } else { at(4.0, ((h - 14.0) / 2.0).floor(), 14.0, 14.0) };
                let _ = icon(ui, c, "cap", "groovecircuit/gc_capture_length.svg", "DRUM CAPTURE: show the GROOVE CIRCUIT panel in a horizontal layout to use it.");
            }
        }
    }
}

// ------------------------------------------------------------------ vertical waveform

/// rekordbox's bar count at `t`: bars from the grid's first down beat and the beat in the
/// bar ("12.3Bars"); before the first down beat the bars count backwards from -1, as capture
/// 01 shows ("-1.2Bars" three beats before the first bar).
pub(crate) fn bars_text(beats: &[Beat], t: f64) -> String {
    if beats.is_empty() {
        return String::new();
    }
    let i = beats.partition_point(|b| b.time <= t + 1e-4).max(1) - 1;
    let fd = beats.iter().position(|x| x.beat == 1).unwrap_or(0);
    if i < fd {
        let back = fd - i; // beats to the first down beat: 1..
        return format!("-{}.{}Bars", (back + 3) / 4, 4 - (back - 1) % 4);
    }
    let bars = beats[fd..=i].iter().filter(|x| x.beat == 1).count().max(1);
    format!("{}.{}Bars", bars, beats[i].beat)
}

/// The beat-count text of a deck (the bar count, or the count to the next memory cue).
pub(crate) fn beat_count_text(app: &App, pos: f64) -> String {
    if app.deck.track.is_none() {
        return String::new();
    }
    if app.prefs.beat_count_mode == 0 {
        return bars_text(&app.deck.beats, pos);
    }
    let mem: Vec<f64> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).map(|c| c.start).collect();
    crate::player_ext::beat_count_label(&app.deck.beats, &mem, pos, app.prefs.beat_count_mode)
}

/// The enlarged waveform of the active deck drawn vertically in `lane`: time runs from top to
/// bottom and the playhead is a line across the middle. It is the horizontal waveform with x
/// and y exchanged: what the horizontal one has along its top edge (beat marks, hot cue
/// letters) is along the left edge here, what it has along its bottom (the current cue,
/// memory cues, phrases) along the right. The waveform is one mesh, as `ui_kit::Bars`.
fn draw_detail_vertical(app: &mut App, ui: &mut Ui, lane: Rect, pos: f64, pps: f64, transport_pps: f64) {
    let p = ui.painter_at(lane);
    let cy = lane.center().y;
    // The beat marks' outer ends: 1 pt in from the left, 2.5 pt in from the right.
    let (xl, xr) = (lane.min.x + 1.0, lane.max.x - 2.5);
    let xc = (xl + xr) / 2.0;
    let y_of = |t: f64| cy + ((t - pos) * pps) as f32;
    let t_of = |y: f32| pos + (y - cy) as f64 / pps;
    let dur = app.deck.duration;
    let (tt, tb) = (t_of(lane.min.y), t_of(lane.max.y));
    // Loops: the running one shaded across the lane, saved ones as a band along the right.
    if app.loop_active().is_none() {
        app.deck.x.loop_adjust = 0;
        app.deck.x.adjust_drag = None;
    }
    if let Some((a, b)) = app.loop_active() {
        p.rect_filled(Rect::from_min_max(pos2(xl, y_of(a)), pos2(xr, y_of(b))), 0.0, LOOP.gamma_multiply(0.22));
        let adj = app.deck.x.loop_adjust;
        if adj != 0 {
            let y = y_of(if adj == 1 { a } else { b });
            p.line_segment([pos2(xl, y), pos2(xr, y)], Stroke::new(2.0_f32, LOOP));
        }
    }
    for c in &app.deck.cues {
        if let Some(e) = c.end.filter(|e| *e > c.start) {
            p.rect_filled(Rect::from_min_max(pos2(xr - 6.0, y_of(c.start)), pos2(xr, y_of(e))), 0.0, LOOP.gamma_multiply(0.6));
        }
    }
    // Beat lines behind the waveform: dim grey between the marks (bars get a white line on top).
    let i0 = app.deck.beats.partition_point(|b| b.time < tt);
    {
        let mut lines = crate::ui_kit::Bars::new();
        for b in app.deck.beats[i0..].iter().take_while(|b| b.time <= tb) {
            let y = (y_of(b.time) - 0.5).round() + 0.5;
            lines.rect(Rect::from_min_max(pos2(xl + 5.0, y - 0.5), pos2(xr - 5.0, y + 0.5)), gy(0x4C4C4C));
        }
        lines.paint(&p);
    }
    let stem_view = app.stem_wave_shown();
    if app.stem_wave_preparing() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    } else if let Some(w) = app.deck.x.stem_wave.as_ref().filter(|_| stem_view).map(|s| &s.1).or(app.deck.wave.as_ref()) {
        let fps = WAVE_FPS;
        let n = w.frames.len();
        let half = (xr - xl) / 2.0;
        let mut rows = crate::ui_kit::Bars::new();
        let mut y = lane.min.y;
        while y < lane.max.y {
            let (t0, t1) = (t_of(y), t_of(y + 1.0));
            if t1 >= 0.0 && t0 <= dur {
                let a = ((t0 * fps).max(0.0) as usize).min(n);
                let b = (((t1 * fps).ceil()) as usize).clamp(a + 1, n.max(a + 1)).min(n);
                if a < n {
                    let mut m = [0u8; 8];
                    for f in &w.frames[a..b.max(a + 1).min(n)] {
                        for k in 0..8 {
                            m[k] = m[k].max(f[k]);
                        }
                    }
                    let s = crate::ui_player::DETAIL_FULL_SCALE;
                    let mut put = |h: f32, col: Color32| {
                        let hh = h * half;
                        if app.prefs.wave_half {
                            rows.rect(Rect::from_min_max(pos2(xr - 2.0 * hh, y), pos2(xr, y + 1.0)), col);
                        } else {
                            rows.rect(Rect::from_min_max(pos2(xc - hh, y), pos2(xc + hh, y + 1.0)), col);
                        }
                    };
                    if app.prefs.wave_color >= 2 {
                        let [lo, mi, hi] = match &app.deck.rb_wave {
                            Some(rw) if !rw.0.is_empty() && !stem_view => {
                                let ia = ((t0.max(0.0) * 150.0) as usize).min(rw.0.len() - 1);
                                let ib = ((t1.max(0.0) * 150.0).ceil() as usize).clamp(ia + 1, rw.0.len());
                                rw.0[ia..ib].iter().fold([0u8; 3], |acc, v| [acc[0].max(v[0]), acc[1].max(v[1]), acc[2].max(v[2])])
                            }
                            _ => crate::export::anlz::three_band_detail(&m),
                        };
                        let f = |v: u8| (v as f32 / 127.0).min(1.0) * s;
                        put(f(lo), WAVE_LOW);
                        put(f(mi), WAVE_MID);
                        put(f(hi), WAVE_HIGH);
                    } else {
                        for (h, c) in crate::ui_player::wave_bars(app.prefs.wave_color, Waveform::disp(&m)) {
                            put(h * s, c);
                        }
                    }
                }
            }
            y += 1.0;
        }
        rows.paint(&p);
    } else if app.deck.track.is_some() {
        p.text(pos2(xc, cy + 20.0), Align2::CENTER_CENTER, if app.deck.loading.is_some() { "Loading…" } else { "Analyzing…" }, arial(11.0), DIM.c());
    }
    // Vocal positions: a thin band just inside the beat marks (left); phrases: a band along
    // the right edge, each with its name at its start.
    if let (true, Some(v)) = (app.prefs.vocal_enlarged, &app.deck.vocal) {
        let (step, levels) = (v.0, &v.1);
        let mut band = crate::ui_kit::Bars::new();
        let mut y = lane.min.y;
        while y < lane.max.y {
            let t0 = t_of(y);
            if t0 >= 0.0 && t0 <= dur {
                let a = (t0 / step) as usize;
                let b = ((t_of(y + 1.0) / step) as usize).max(a + 1);
                let lvl = levels.get(a.min(levels.len())..b.min(levels.len())).and_then(|s| s.iter().max().copied()).unwrap_or(0);
                if lvl > 0 {
                    band.rect(Rect::from_min_max(pos2(xl + 5.0, y), pos2(xl + 7.0, y + 1.0)), hex(0x4FC3D9).gamma_multiply(0.35 + 0.65 * (lvl.min(4) as f32 / 4.0)));
                }
            }
            y += 1.0;
        }
        band.paint(&p);
    }
    if let (true, Some(st), false) = (app.prefs.phrase_enlarged, &app.deck.phrases, app.deck.beats.is_empty()) {
        let beat_t = |b: u16| app.deck.beats.get((b.max(1) - 1) as usize).map(|x| x.time).unwrap_or(dur);
        let hover = ui.ctx().pointer_latest_pos().filter(|pp| lane.contains(*pp));
        for (k, ph) in st.phrases.iter().enumerate() {
            let t0 = beat_t(ph.beat);
            let t1 = st.phrases.get(k + 1).map(|n| beat_t(n.beat)).unwrap_or_else(|| beat_t(st.end_beat));
            let (y0, y1) = (y_of(t0), y_of(t1));
            if y1 < lane.min.y || y0 > lane.max.y {
                continue;
            }
            let col = hex(crate::phrase::color(st.mood, ph.kind));
            p.rect_filled(Rect::from_min_max(pos2(xr - 3.0, y0.max(lane.min.y) + 0.5), pos2(xr, (y1.min(lane.max.y) - 0.5).max(y0 + 1.0))), 0.0, col);
            let hovered = hover.map(|pp| pp.y >= y0 && pp.y < y1).unwrap_or(false);
            if (app.prefs.phrase_types_always || hovered) && y0 >= lane.min.y - 12.0 {
                let name = crate::phrase::label(st.mood, ph.kind);
                let tw = (name.len() as f32 * 5.2 + 7.0).min(xr - xl - 8.0);
                let tag = Rect::from_min_size(pos2(xr - 4.0 - tw, y0 + 1.0), vec2(tw, 11.0));
                p.rect_filled(tag, 1.0, col);
                p.with_clip_rect(tag).text(tag.center(), Align2::CENTER_CENTER, name, FontId::proportional(8.5), Color32::BLACK);
            }
        }
    }
    // The beat grid's marks at both ends of each line.
    let cap = |p: &Painter, x: f32, y: f32, dir: f32, col: Color32, bar: bool| {
        if bar {
            p.add(egui::Shape::convex_polygon(vec![pos2(x, y - 2.5), pos2(x, y + 2.5), pos2(x + dir * 4.0, y)], col, Stroke::NONE));
        } else {
            p.add(egui::Shape::convex_polygon(vec![pos2(x, y - 1.5), pos2(x, y + 1.5), pos2(x + dir * 3.0, y)], col, Stroke::NONE));
            p.rect_filled(Rect::from_two_pos(pos2(x, y - 0.5), pos2(x + dir * 4.0, y + 0.5)), 0.0, col);
        }
    };
    for b in app.deck.beats[i0..].iter().take_while(|b| b.time <= tb) {
        let y = (y_of(b.time) - 0.5).round() + 0.5;
        if b.beat == 1 {
            cap(&p, xl, y, 1.0, BAR_RED, true);
            cap(&p, xr, y, -1.0, BAR_RED, true);
            if app.deck.wave.is_some() {
                p.rect_filled(Rect::from_min_max(pos2(xl + 5.0, y - 0.5), pos2(xr - 5.0, y + 0.5)), 0.0, ON_ACCENT);
            }
        } else {
            cap(&p, xl, y, 1.0, gy(0x7C7C7C), false);
            cap(&p, xr, y, -1.0, gy(0x7C7C7C), false);
        }
    }
    // Cues: hot cues as a line with the letter box at the left, memory cues as a triangle at
    // the right, the current cue as rekordbox's orange triangle at the right.
    for c in &app.deck.cues {
        let y = y_of(c.start);
        if y < lane.min.y - 12.0 || y > lane.max.y + 12.0 {
            continue;
        }
        if c.kind == CueKind::Hot {
            let col = hotcue_color(c.color);
            p.line_segment([pos2(xl, y), pos2(xr, y)], Stroke::new(1.0_f32, col));
            let lr = Rect::from_center_size(pos2(xl + 5.5, y), vec2(11.0, 11.0));
            p.rect_filled(lr, 0.0, col);
            p.text(lr.center(), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), FontId::proportional(10.0), Color32::BLACK);
        } else {
            let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
            p.add(egui::Shape::convex_polygon(vec![pos2(xr - 7.2, y), pos2(xr, y - 6.0), pos2(xr, y + 6.0)], col, Stroke::NONE));
        }
    }
    if app.deck.track.is_some() {
        let y = y_of(app.deck.cue_point);
        p.add(egui::Shape::convex_polygon(vec![pos2(xr - 8.5, y), pos2(xr, y - 5.35), pos2(xr, y + 5.35)], CUE_MARK, Stroke::NONE));
        // Playhead: a 1 pt line across the lane with a half-black edge on both sides.
        let ph = Rect::from_min_max(pos2(lane.min.x, cy - 0.5), pos2(lane.max.x - 1.5, cy + 0.5));
        p.rect_filled(ph.expand2(vec2(0.0, 1.0)), 0.0, Color32::from_black_alpha(128));
        p.rect_filled(ph, 0.0, hex(0xE0E0E0));
    }
    // Interaction: drag to scrub, scroll to move, Cmd / Ctrl + scroll to zoom, right-click for
    // the waveform menu (or PLAY / CUE with "Click on the waveform for PLAY and CUE").
    let resp = ui.interact(lane, ui.id().with("vdetail"), Sense::click_and_drag());
    let click_mode = app.prefs.wave_click_play_cue;
    let (alt, sec_pressed, sec_down) = ui.input(|i| (i.modifiers.alt, i.pointer.button_pressed(egui::PointerButton::Secondary), i.pointer.button_down(egui::PointerButton::Secondary)));
    if app.deck.audio.is_none() {
        return;
    }
    if resp.hovered() && sec_pressed {
        if click_mode && !alt {
            app.deck.x.wave_cue_held = true;
            app.cue_down();
        } else if let Some(pp) = resp.hover_pos() {
            app.deck.x.menu_time = Some(t_of(pp.y));
        }
    }
    if app.deck.x.wave_cue_held && !sec_down {
        app.deck.x.wave_cue_held = false;
        app.cue_up();
    }
    if click_mode && resp.clicked() {
        app.play_pause();
    }
    if !click_mode || alt || resp.context_menu_opened() {
        let t = app.deck.x.menu_time.unwrap_or_else(|| app.pos());
        resp.context_menu(|ui| crate::ui_player2::wave_menu(app, ui, t));
    }
    // Cue markers can be dragged along the lane as on the horizontal waveforms (hot cues by
    // their letter box at the left, memory cues by the triangle at the right; quantized,
    // Escape cancels): the same rule as `ui_player::draw_detail_at`, turned upright.
    let hit = |cues: &[Cue], pp: Pos2| -> Option<i64> {
        let mut best: Option<(f32, i64)> = None;
        for c in cues {
            let y = y_of(c.start);
            let (ok, d) = match c.kind {
                CueKind::Hot => (pp.x >= xl && pp.x <= xl + 12.0 && (pp.y - y).abs() <= 6.0, (pp.y - y).abs()),
                _ => (pp.x >= xr - 9.0 && pp.x <= xr + 1.0 && (pp.y - y).abs() <= 7.0, (pp.y - y).abs()),
            };
            if ok && best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, c.id));
            }
        }
        best.map(|b| b.1)
    };
    if app.deck.cue_drag.is_none() {
        if let Some(hp) = resp.hover_pos() {
            if hit(&app.deck.cues, hp).is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
        }
    }
    if resp.drag_started() {
        let pp = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        let adj = app.deck.x.loop_adjust;
        if adj != 0 && app.loop_active().is_some() {
            let (a, b) = app.loop_active().unwrap_or((0.0, 0.0));
            app.deck.x.adjust_drag = Some((pp.map(|p| p.y).unwrap_or(cy), if adj == 1 { a } else { b }));
        } else if let Some((id, pp)) = pp.and_then(|pp| hit(&app.deck.cues, pp).map(|id| (id, pp))) {
            let start = app.deck.cues.iter().find(|c| c.id == id).map(|c| c.start).unwrap_or(0.0);
            app.deck.cue_drag = Some((id, app.deck.cues.clone(), t_of(pp.y) - start));
        } else {
            app.deck.drag_origin = Some((pp.map(|p| p.y).unwrap_or(cy), app.pos()));
        }
    }
    if let Some((id, _, grab)) = app.deck.cue_drag.clone() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.commit_cue_drag(true);
        } else if resp.dragged() {
            if let Some(pp) = resp.interact_pointer_pos() {
                let t = app.quantized(t_of(pp.y) - grab);
                if let Some(c) = app.deck.cues.iter_mut().find(|c| c.id == id) {
                    *c = crate::ui_player::moved_cue(c, t, dur);
                }
            }
        }
        if resp.drag_stopped() {
            app.commit_cue_drag(false);
        }
    } else if let Some((y0, t0)) = app.deck.x.adjust_drag {
        if let (true, Some(pp)) = (resp.dragged(), resp.interact_pointer_pos()) {
            app.loop_adjust_to(t0 + (pp.y - y0) as f64 / transport_pps);
        }
        if resp.drag_stopped() || !resp.dragged() {
            app.deck.x.adjust_drag = None;
        }
    } else if resp.dragged() {
        if let (Some((y0, p0)), Some(pp)) = (app.deck.drag_origin, resp.interact_pointer_pos()) {
            let t = p0 - (pp.y - y0) as f64 / transport_pps;
            app.deck.x.scrub += t.clamp(0.0, dur.max(0.0)) - app.pos();
            app.seek(t);
        }
    }
    if resp.drag_stopped() && app.deck.drag_origin.is_some() {
        app.deck.drag_origin = None;
        if !app.playing() {
            let q = app.quantized(app.pos());
            app.deck.cue_point = q;
            app.seek(q);
        }
    }
    if resp.hovered() {
        let (scroll, zoom_mod) = ui.input(|i| (i.raw_scroll_delta, i.modifiers.command || i.modifiers.ctrl));
        if zoom_mod && scroll.y.abs() > 0.5 {
            zoom_step(app, if scroll.y > 0.0 { -1 } else { 1 });
        } else if scroll.y.abs() > 0.5 || scroll.x.abs() > 0.5 {
            let d = if scroll.y.abs() >= scroll.x.abs() { scroll.y } else { scroll.x };
            let current = app.pos();
            let t = current - d as f64 / transport_pps;
            app.deck.x.scrub += t.clamp(0.0, dur.max(0.0)) - current;
            app.seek(t);
        }
    }
}

/// The lanes of the horizontal layouts: decks in the order they are stacked (4-deck: 1 to 4,
/// 12 pt in from both sides, with the deck numbers in the margins) and each lane's rectangle
/// inside the waveform area `wa` (rekordbox's `PlayerWaveView`).
pub(crate) fn h_lanes(wa: Rect, count: u8) -> Vec<Rect> {
    let inset = if count == 4 { 12.0 } else { 0.0 };
    let lh = (wa.height() / count as f32).floor();
    (0..count).map(|n| Rect::from_min_size(pos2(wa.min.x + inset, wa.min.y + lh * n as f32), vec2(wa.width() - 2.0 * inset, lh))).collect()
}

/// A horizontal lane's waveform: `draw_detail_at` between the beat marks (1 pt under the
/// lane's top, 2.5 pt above its bottom), with the Performance zoom and wheel zoom.
pub(crate) fn h_lane(app: &mut App, ui: &mut Ui, lane: Rect, n: u8) {
    let r = Rect::from_min_max(pos2(lane.min.x, lane.min.y + 1.0), pos2(lane.max.x, lane.max.y - 2.5));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(lane).id_salt(("perfwave", n)));
    child.set_clip_rect(lane.intersect(ui.clip_rect()));
    app.with_deck(n, |app| {
        let pos = if app.deck.x.dk.grid_tab { app.pos() } else { crate::ui_feel::shown_pos(app) };
        // (Cmd / Ctrl + wheel changes `prefs.zoom` in there: taken as a Performance zoom step.)
        let z = app.prefs.zoom;
        crate::ui_player::draw_detail_at(app, &mut child, r, pos, 0.0, crate::ui_player::DetailMode::Full);
        if let Some((rect, shown, pps)) = app.deck.detail_map {
            capture_pick(app, &mut child, rect, n, false, shown, pps);
        }
        if app.prefs.zoom != z {
            let dir = if app.prefs.zoom < z { -1 } else { 1 };
            app.prefs.zoom = z;
            zoom_step(app, dir);
        }
        // Hot cue letters: the waveform draws them in a row above its beats, which a lane does
        // not have (it is clipped to the lane), so they are drawn again inside the lane's top
        // edge, centred on the cue line.
        if let Some((dr, dpos, pps)) = app.deck.detail_map {
            let cx = dr.center().x;
            let p = child.painter().clone();
            for c in app.deck.cues.iter().filter(|c| c.kind == CueKind::Hot) {
                let x = cx + ((c.start - dpos) * pps) as f32;
                if x < lane.min.x - 6.0 || x > lane.max.x + 6.0 {
                    continue;
                }
                let lr = Rect::from_min_size(pos2(x - 5.5, lane.min.y + 1.0), vec2(11.0, 11.0));
                p.rect_filled(lr, 0.0, hotcue_color(c.color));
                p.text(lr.center(), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), FontId::proportional(10.0), Color32::BLACK);
            }
        }
        // The beat count: ends 2 pt left of the playhead, at the top of the lane.
        let label = beat_count_text(app, pos);
        child.painter().text(pos2(r.center().x - 2.0, lane.min.y + 8.0), Align2::RIGHT_CENTER, label, arial_b(16.4), BEAT_BLUE.c());
    });
}

// ------------------------------------------------------------------ the layouts

/// The layouts `ui_perf::performance` does not draw itself: the vertical ones, Browse and the
/// horizontal ones without pad and platter. Returns false for 2Deck / 4Deck Horizontal.
pub(crate) fn performance_alt(app: &mut App, ui: &mut Ui) -> bool {
    match layout(app) {
        Layout::H2 | Layout::H4 => false,
        Layout::V2 | Layout::V4 => {
            vertical(app, ui);
            true
        }
        Layout::Browse2 | Layout::Browse4 => {
            browse(app, ui);
            true
        }
        Layout::Simple2 | Layout::Simple4 => {
            simple(app, ui);
            true
        }
    }
}

/// Activate the pressed deck and deliver completed sampler captures to their pads.
pub(crate) fn finish(app: &mut App, ui: &mut Ui, full: Rect, rects: &[Rect], lanes: &[(u8, Rect)]) {
    let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    if let Some(pp) = pressed.filter(|pp| full.contains(*pp)) {
        if let Some(n) = rects.iter().position(|r| r.contains(pp)) {
            app.activate_deck(n as u8);
        } else if let Some((n, _)) = lanes.iter().find(|(_, r)| r.contains(pp)) {
            app.activate_deck(*n);
        }
    }
    capture_drop(app, ui, full);
}

/// One deck of a layout, as `ui_perf::deck` draws it for the variant `d`.
pub(crate) fn deck_at(app: &mut App, ui: &mut Ui, dr: Rect, n: u8, d: Dk) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(dr).id_salt(("perfdeck", n)));
    child.set_clip_rect(dr.intersect(ui.clip_rect()));
    let is_active = app.px.active == n;
    let parts = parts_row_h(app);
    app.with_deck(n, |app| with_dk(d, || crate::ui_perf::deck(app, &mut child, dr, n, is_active, parts, d.compact())));
    app.with_deck(n, |app| crate::ui_player2::drop_to_load(app, ui, dr));
}

/// The FX panel above a layout and the sampler panel under it, as the horizontal layouts have
/// them. Returns the area left for the layout.
pub(crate) fn panels(app: &mut App, ui: &mut Ui) -> Rect {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, BG);
    let mut t = full.min.y;
    if app.prefs.perf_fx && panel_enabled(app, 0) {
        crate::ui_fx::panel(app, ui, Rect::from_min_size(pos2(full.min.x, t), vec2(full.width(), crate::ui_fx::fx_h(app) - 1.0)));
        t += crate::ui_fx::fx_h(app);
    }
    let sampler_h = if app.prefs.perf_sampler && panel_enabled(app, 2) { crate::ui_sampler::SAMPLER_H } else { 0.0 };
    app.sampler_poll();
    if sampler_h > 0.0 {
        crate::ui_sampler::panel(app, ui, Rect::from_min_max(pos2(full.min.x, full.max.y - sampler_h), full.max));
    }
    Rect::from_min_max(pos2(full.min.x, t), pos2(full.max.x, full.max.y - sampler_h))
}

/// The mixer panel and its bottom row, when the layout shows them.
pub(crate) fn mixer_parts(app: &mut App, ui: &mut Ui, fr: &Frame, cx: f32) {
    if let Some(mr) = fr.mixer {
        with_dk(fr.dk, || if fr.count == 4 { crate::ui_perf::mixer4(app, ui, mr) } else { crate::ui_perf::mixer(app, ui, mr) });
    }
    if let Some(br) = fr.bottom {
        crate::ui_perf::bottom_row(app, ui, br, cx);
    }
}

/// 2Deck Vertical and 4Deck Vertical: the decks at both sides (1 and 3 left, 2 and 4 right),
/// between them a 254 pt column with the vertical waveforms above the mixer.
fn vertical(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let area = panels(app, ui);
    let p = ui.painter().clone();
    let fr = frame(app, area);
    let four = fr.count == 4;
    // rekordbox's PlayerWaveView: the lanes side by side (4 decks: 3, 1, 2, 4), the waveform
    // from 18 pt under its top (4 decks: the deck numbers' row) to 14 pt above its bottom (the
    // beat counts' row).
    let wa = fr.waves;
    p.rect_filled(wa, 0.0, WAVE_BG);
    let ord = crate::prefs_bridge::wave_order(app, true, fr.count);
    let order: &[u8] = if four { &ord } else { &[0, 1] };
    let lane_w = wa.width() / fr.count as f32;
    let view_top = wa.min.y + if four { 18.0 } else { 0.0 };
    let view_bot = wa.max.y - 14.0;
    let mut lanes = vec![];
    for (i, n) in order.iter().enumerate() {
        let lane = Rect::from_min_max(pos2(wa.min.x + lane_w * i as f32, view_top), pos2(wa.min.x + lane_w * (i + 1) as f32, view_bot));
        lanes.push((*n, lane));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(lane).id_salt(("perfvwave", *n)));
        child.set_clip_rect(lane.intersect(ui.clip_rect()));
        let label = app.with_deck(*n, |app| {
            let selected_span = span(app).unwrap_or(2.0);
            let (pos, shown_span) = if app.deck.x.dk.grid_tab {
                (app.pos(), selected_span)
            } else {
                (crate::ui_feel::shown_pos(app), crate::ui_feel::zoom_span(&child, lane, selected_span))
            };
            let pps = lane.height() as f64 / shown_span;
            draw_detail_vertical(app, &mut child, lane, pos, pps, lane.height() as f64 / selected_span);
            capture_pick(app, &mut child, lane, *n, true, pos, pps);
            beat_count_text(app, pos)
        });
        // The beat count under the lane: rekordbox's labels are 64 pt wide at 31 and 153 pt
        // (2 decks), 55 pt wide at 0, 58, 124 and 182 pt (4 decks), 12 pt above the bottom.
        let (lx, lw) = if four { ([0.0, 58.0, 124.0, 182.0][i], 55.0) } else { ([31.0, 153.0][i], 64.0) };
        p.text(pos2(wa.min.x + lx + lw / 2.0, wa.max.y - 6.5), Align2::CENTER_CENTER, label, arial_b(if four { 8.5 } else { 9.0 }), BEAT_BLUE.c());
        if four && (i == 0 || i == 3) {
            // Deck numbers above the outer lanes (the control strip's buttons are over the inner ones).
            let loaded = app.deck_of(*n).track.is_some();
            p.text(pos2(wa.min.x + if i == 0 { 35.0 } else { 213.0 }, wa.min.y + 9.0), Align2::CENTER_CENTER, format!("{}", n + 1), arial_b(13.0), if loaded { hex(0x969696) } else { hex(0x323232) });
        }
    }
    control_strip(app, ui, wa, true);
    for n in 0..fr.count {
        deck_at(app, ui, fr.decks[n as usize], n, fr.dk);
    }
    mixer_parts(app, ui, &fr, full.center().x);
    finish(app, ui, full, &fr.decks[..fr.count as usize], &lanes);
}

/// Browse: the decks' headers and overviews only, two side by side (four: 1 and 2 above 3
/// and 4); the browser has the rest of the window.
fn browse(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, BG);
    app.sampler_poll();
    let fr = frame(app, full);
    for n in 0..fr.count {
        deck_at(app, ui, fr.decks[n as usize], n, fr.dk);
    }
    finish(app, ui, full, &fr.decks[..fr.count as usize], &[]);
}

/// 2Deck / 4Deck Horizontal with pad and platter hidden (rekordbox's `HidePlatter`): the
/// waveforms and, under them, the decks' headers and overviews; no mixer.
fn simple(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, BG);
    app.sampler_poll();
    let fr = frame(app, full);
    let lanes = waves_h(app, ui, &fr);
    for n in 0..fr.count {
        deck_at(app, ui, fr.decks[n as usize], n, fr.dk);
    }
    finish(app, ui, full, &fr.decks[..fr.count as usize], &lanes);
}

/// The horizontal waveforms of a layout: one lane per deck, the deck numbers (4 decks) and
/// the control strip over their left end. Returns the lanes.
pub(crate) fn waves_h(app: &mut App, ui: &mut Ui, fr: &Frame) -> Vec<(u8, Rect)> {
    // rekordbox's PlayerWaveView fills its area black in both appearances.
    ui.painter().rect_filled(fr.waves, 0.0, WAVE_BG);
    let ls = h_lanes(fr.waves, fr.count);
    let mut lanes = vec![];
    // (View > Layout > Enlarged Waveform: the decks' order, top to bottom.)
    let order = crate::prefs_bridge::wave_order(app, false, fr.count);
    for (i, lane) in ls.iter().enumerate() {
        let n = order[i];
        h_lane(app, ui, *lane, n);
        lanes.push((n, *lane));
    }
    lane_numbers(app, ui, fr.waves, &ls);
    control_strip(app, ui, fr.waves, false);
    lanes
}

/// 4-deck horizontal waveforms: the deck numbers in the 12 pt margins at both ends of each
/// lane, lit for a loaded deck.
pub(crate) fn lane_numbers(app: &App, ui: &Ui, wa: Rect, lanes: &[Rect]) {
    if lanes.len() != 4 {
        return;
    }
    let p = ui.painter();
    let order = crate::prefs_bridge::wave_order(app, false, 4);
    for (i, lane) in lanes.iter().enumerate() {
        let n = order[i] as usize;
        let loaded = app.deck_of(n as u8).track.is_some();
        let col = if loaded { hex(0x969696) } else { hex(0x323232) };
        p.rect_filled(Rect::from_min_max(pos2(wa.min.x, lane.min.y), pos2(lane.min.x, lane.max.y)), 0.0, WAVE_BG);
        p.rect_filled(Rect::from_min_max(pos2(lane.max.x, lane.min.y), pos2(wa.max.x, lane.max.y)), 0.0, WAVE_BG);
        for x in [wa.min.x + 6.0, wa.max.x - 6.0] {
            p.text(pos2(x, lane.center().y), Align2::CENTER_CENTER, format!("{}", n + 1), arial_b(13.0), col);
        }
    }
}

// ------------------------------------------------------------------ where the big pieces sit

/// The rectangles of a layout inside `area` (the Performance area without the FX and sampler
/// panels): the decks, the waveform view, the mixer panel and the mixer's bottom row.
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub count: u8,
    pub dk: Dk,
    pub decks: [Rect; 4],
    /// rekordbox's `PlayerWaveView` (empty in Browse).
    pub waves: Rect,
    pub mixer: Option<Rect>,
    pub bottom: Option<Rect>,
}

pub(crate) fn frame_of(l: Layout, area: Rect, extra: f32, mixer_on: bool) -> Frame {
    frame_with(l, area, extra, mixer_on, 0.0)
}

/// `mpl_h`: the height of the MIX POINT LINK strip when it stands in for the stacked
/// waveforms of the horizontal layouts (0: the waveforms).
pub(crate) fn frame_with(l: Layout, area: Rect, extra: f32, mixer_on: bool, mpl_h: f32) -> Frame {
    let count = l.decks();
    let four = count == 4;
    let (x0, t, wd, cx) = (area.min.x, area.min.y, area.width(), area.center().x);
    let mut decks = [Rect::NOTHING; 4];
    let dk = match l {
        Layout::H2 => Dk::H2,
        Layout::H4 => Dk::H4,
        Layout::V2 => Dk::V2,
        Layout::V4 => Dk::V4,
        Layout::Browse2 | Layout::Browse4 => Dk::Browse,
        Layout::Simple2 => Dk::Simple2,
        Layout::Simple4 => Dk::Simple4,
    };
    let g = deck_geo(dk);
    match l {
        Layout::H2 | Layout::H4 => {
            // Waveforms across the top (142 / 198 pt with their 3 pt gutter), the decks under
            // them at both sides of the mixer (106 / 190 pt wide between 3 pt gutters).
            let waves_h = if mpl_h > 0.0 { mpl_h } else if four { 198.0 } else { 142.0 };
            let deck_h = g.info_h + 3.0 + g.lower_h + extra;
            let mw = if four { 190.0 } else { 106.0 };
            let (dw, d2x) = if mixer_on { ((cx - mw / 2.0 - 3.0 - x0).round(), (cx + mw / 2.0 + 3.0).round()) } else { (((wd - 3.0) / 2.0).floor(), x0 + ((wd - 3.0) / 2.0).floor() + 3.0) };
            let top = t + waves_h;
            for n in 0..count as usize {
                let right = n % 2 == 1;
                decks[n] = Rect::from_min_size(pos2(if right { d2x } else { x0 }, top + (n / 2) as f32 * (deck_h + 3.0)), vec2(if right { area.max.x - d2x } else { dw }, deck_h));
            }
            let decks_h = if four { deck_h * 2.0 + 3.0 } else { deck_h };
            Frame { count, dk, decks, waves: Rect::from_min_size(pos2(x0, t + 1.0), vec2(wd, waves_h - 4.0)), mixer: mixer_on.then(|| Rect::from_min_size(pos2(x0 + dw + 3.0, top), vec2(mw, decks_h))), bottom: mixer_on.then(|| Rect::from_min_size(pos2(x0, top + decks_h + 3.0), vec2(wd, 40.0))) }
        }
        Layout::V2 | Layout::V4 => {
            // The decks at both sides of a 254 pt column: the waveforms above the mixer panel
            // (147 / 234 pt), which keeps its height.
            let deck_h = g.info_h + 3.0 + g.lower_h + extra;
            let col_x = (cx - 127.0).round();
            let top = t + 1.0;
            for n in 0..count as usize {
                let right = n % 2 == 1;
                decks[n] = Rect::from_min_size(pos2(if right { col_x + 254.0 } else { x0 }, top + (n / 2) as f32 * (deck_h + 3.0)), vec2(if right { area.max.x - col_x - 254.0 } else { col_x - x0 }, deck_h));
            }
            let decks_h = if four { deck_h * 2.0 + 3.0 } else { deck_h };
            let mixer_h = if !mixer_on { 0.0 } else if four { 234.0 } else { 147.0 };
            let wave_h = decks_h - mixer_h - if mixer_on { 3.0 } else { 0.0 };
            Frame { count, dk, decks, waves: Rect::from_min_size(pos2(col_x + 3.0, top), vec2(248.0, wave_h)), mixer: mixer_on.then(|| Rect::from_min_size(pos2(col_x + 3.0, top + decks_h - mixer_h), vec2(248.0, mixer_h))), bottom: mixer_on.then(|| Rect::from_min_size(pos2(x0, top + decks_h + 3.0), vec2(wd, 40.0))) }
        }
        Layout::Browse2 | Layout::Browse4 => {
            // Decks 754 and 755 pt wide at 1512 pt, 3 pt apart; each 95 pt tall, 3 pt under each.
            let w1 = ((wd - 3.0) / 2.0).floor();
            for n in 0..count as usize {
                let right = n % 2 == 1;
                decks[n] = Rect::from_min_size(pos2(if right { x0 + w1 + 3.0 } else { x0 }, t + 1.0 + (n / 2) as f32 * 98.0), vec2(if right { wd - w1 - 3.0 } else { w1 }, 95.0));
            }
            Frame { count, dk, decks, waves: Rect::NOTHING, mixer: None, bottom: None }
        }
        Layout::Simple2 | Layout::Simple4 => {
            let waves_h = if four { 198.0 } else { 142.0 };
            let w1 = ((wd - 3.0) / 2.0).floor();
            for n in 0..count as usize {
                let right = n % 2 == 1;
                decks[n] = Rect::from_min_size(pos2(if right { x0 + w1 + 3.0 } else { x0 }, t + waves_h + (n / 2) as f32 * (g.info_h + 3.0)), vec2(if right { wd - w1 - 3.0 } else { w1 }, g.info_h));
            }
            Frame { count, dk, decks, waves: Rect::from_min_size(pos2(x0, t + 1.0), vec2(wd, waves_h - 4.0)), mixer: None, bottom: None }
        }
    }
}

pub(crate) fn frame(app: &App, area: Rect) -> Frame {
    frame_with(layout(app), area, deck_extra(app) + groove_row(app), mixer_shown(app), mpl_strip(app))
}

/// Height of the MIX POINT LINK strip (ui_mpl.rs) when it is showing: it takes the place of
/// the stacked waveforms, so only in the two horizontal layouts.
pub(crate) fn mpl_strip(app: &App) -> f32 {
    if matches!(layout(app), Layout::H2 | Layout::H4) {
        crate::ui_mpl::strip_height(app)
    } else {
        0.0
    }
}

/// How much taller the player area is while that strip is shown.
pub(crate) fn mpl_extra(app: &App) -> f32 {
    let h = mpl_strip(app);
    if h > 0.0 {
        h - if app.deck_count() == 4 { 198.0 } else { 142.0 }
    } else {
        0.0
    }
}

/// Height of the GROOVE CIRCUIT panel under each deck's pads (ui_groove.rs): only in the
/// layouts that show the pads.
pub(crate) fn groove_row(app: &App) -> f32 {
    let l = layout(app);
    if l.full() && !l.simple() {
        app.groove_row_height()
    } else {
        0.0
    }
}

// ------------------------------------------------------------------ top bar

/// The layout pull-down of the top bar (rekordbox: 40 x 20 at x 135, y 4): the current
/// layout's mark and the list with rekordbox's names, marks and order.
pub(crate) fn layout_selector(app: &mut App, ui: &mut Ui, bar: Rect) {
    let p = ui.painter().clone();
    let r = Rect::from_min_size(bar.min + vec2(130.0, 3.0), vec2(50.0, 22.0));
    let resp = ui.interact(r, ui.id().with("perflayout"), Sense::click());
    if let Some(c) = crate::ui_feel::hover_fill(&resp, gy(0x1E1E1E)) {
        p.rect_filled(r, 2.0, c);
    }
    let row = layout(app).row();
    let mark = |p: &Painter, row: usize, c: Pos2| {
        if !skin_at(p, LAYOUT_MARKS[row], Rect::from_center_size(c, vec2(18.0, 18.0)), WHITE.c()) {
            // The marks as rekordbox draws them: 2 pt bars on an 18 pt square.
            let bar = |x: f32, y: f32, w: f32, h: f32| p.rect_filled(Rect::from_min_size(c + vec2(x - 9.0, y - 9.0), vec2(w, h)), 0.0, WHITE);
            match row {
                0 => { bar(0.0, 5.0, 18.0, 2.0); bar(0.0, 11.0, 18.0, 2.0); }
                1 => { bar(4.0, 0.0, 2.0, 18.0); bar(12.0, 0.0, 2.0, 18.0); }
                2 => { for y in [3.0, 6.6, 10.3, 14.0] { bar(0.0, y, 18.0, 1.5); } }
                3 => { for x in [2.0, 6.2, 10.4, 14.5] { bar(x, 0.0, 1.5, 18.0); } }
                _ => { p.rect_stroke(Rect::from_center_size(c, vec2(18.0, 11.0)), 0.0, Stroke::new(1.0_f32, WHITE), egui::StrokeKind::Inside); bar(0.0, 3.5, 18.0, 3.0); }
            }
        }
    };
    mark(&p, row, bar.min + vec2(145.0, 14.0));
    crate::icons_player::chevron_down(&p, bar.min + vec2(170.75, 14.5), 1.0, WHITE.c());
    let tips_on = app.prefs.tooltips;
    crate::ui_browser::dropdown(ui, &resp, "perflayoutpop", |ui| {
        for (i, name) in LAYOUT_NAMES.iter().enumerate() {
            let item = ui.selectable_label(row == i, format!("        {name}"));
            mark(ui.painter(), i, pos2(item.rect.min.x + 15.0, item.rect.center().y));
            if item.clicked() {
                set_row(app, i);
                ui.memory_mut(|m| m.close_popup());
            }
        }
        ui.separator();
        // rekordbox has this in its View menu ("Show / Hide Pad and Platter", setting HidePlatter).
        let hp = app.prefs.perf_lay.hide_platter;
        if ui.selectable_label(hp, "        Show / Hide Pad and Platter").clicked() {
            app.prefs.perf_lay.hide_platter = !hp;
            app.poll_decks();
            app.save_prefs();
            ui.memory_mut(|m| m.close_popup());
        }
    });
    if tips_on {
        let _ = resp.on_hover_text("Change Layout");
    }
}

/// A panel button of the top bar drawn with rekordbox's own tile (`glb_*_ON/OFF.svg`), over
/// what the caller drew; dimmed when the layout showing has no such panel. `i` as in
/// `panel_enabled`.
pub(crate) fn panel_tile(app: &App, ui: &Ui, i: usize, tr: Rect, lit: bool) {
    let name = ["FX", "mix_point_link", "sampler", "mixer", "rec"][i.min(4)];
    let on = lit && panel_enabled(app, i);
    let mut file = format!("globalsection/glb_{name}_{}.svg", if on { "ON" } else { "OFF" });
    // (Light: the OFF tile's #282828 face and white glyph become ids 0x03000005 and
    // 0x03000006, `MainComponent::lookAndFeelChanged`.)
    if !on && is_light() {
        file.push_str("#swap:282828>C8C8C8,FFFFFF>000000");
    }
    // (Mix Point Link's tile is dim while its function is switched off in Preferences.)
    let tint = if panel_enabled(app, i) && (i != 1 || crate::app_hooks::mpl_available(app)) { ON_ACCENT.c() } else { Color32::from_white_alpha(90) };
    let p = ui.painter();
    if skin(p.ctx(), &file).is_some() {
        p.rect_filled(tr.expand(0.5), 0.0, HEADER);
        skin_at(p, &file, tr, tint);
    } else if !panel_enabled(app, i) {
        p.rect_filled(tr, 2.0, HEADER.gamma_multiply(0.7));
    }
}

/// The MIXER panel button was pressed.
pub(crate) fn toggle_mixer(app: &mut App) {
    if panel_enabled(app, 3) {
        app.prefs.perf_lay.mixer_hidden = !app.prefs.perf_lay.mixer_hidden;
        app.poll_decks();
        app.save_prefs();
    }
}

/// LINK (Ableton Link), 35 pt after the last panel button: the label over its lamp line.
/// Ableton Link is not built in recordbox: the button is there, unlit, and says so.
pub(crate) fn link_button(ui: &mut Ui, bar: Rect, last_right: f32) {
    let p = ui.painter().clone();
    let r = Rect::from_min_size(pos2(last_right + 35.0, bar.min.y + 4.0), vec2(40.0, 19.0));
    let resp = ui.interact(r, ui.id().with("ablink"), Sense::click());
    p.text(pos2(r.center().x + 0.25, bar.min.y + 11.0), Align2::CENTER_CENTER, "LINK", arial_b(14.0), if resp.hovered() { WHITE.c() } else { gy(0xF0F0F0) });
    p.rect_filled(Rect::from_min_size(pos2(r.min.x, bar.min.y + 20.0), vec2(40.0, 2.0)), 0.0, gy(0x292929));
    let _ = resp.on_hover_text("Ableton Link: not built in recordbox yet.");
}

/// The right-hand side of the Performance top bar, at rekordbox's places (measured from the
/// right edge): PAD, MIDI and the information button. The shared header draws the master controls.
pub(crate) fn performance_tools(app: &mut App, ui: &mut Ui, bar: Rect) {
    let p = ui.painter().clone();
    let (rx, ty) = (bar.max.x, bar.min.y);
    // PAD (the Pad Editor, ui_pads.rs) and MIDI (the MIDI setting window, ui_midi.rs): 30 x 16.
    {
        let r = Rect::from_center_size(pos2(rx - 482.0, ty + 16.0), vec2(30.0, 16.0));
        let resp = ui.interact(r, ui.id().with(("top", "PAD")), Sense::click());
        let on = app.pads.editor_open;
        p.text(pos2(rx - 482.0, ty + 15.75), Align2::CENTER_CENTER, "PAD", arial_b(12.0), if on { WHITE.c() } else { crate::ui_feel::hl(&resp, gy(0x646464), gy(0x8C8C8C)) });
        if resp.on_hover_text("Open/Close the PAD EDITOR window.").clicked() {
            app.pads.editor_open = !on;
        }
    }
    crate::ui_midi::header_button(app, ui, Rect::from_min_size(pos2(rx - 457.0, ty + 8.0), vec2(30.0, 16.0)));
    // Information: rekordbox's hexagon (12 x 14); here it opens About.
    let ir = Rect::from_min_size(pos2(rx - 404.0, ty + 9.0), vec2(12.0, 14.0));
    let iresp = ui.interact(ir.expand(5.0), ui.id().with("topinfo"), Sense::click());
    if !skin_at(&p, "globalArea_header_info.svg", ir, WHITE.c()) {
        p.circle_filled(ir.center(), 7.0, WHITE);
        p.text(ir.center(), Align2::CENTER_CENTER, "i", bold(10.0), HEADER.c());
    }
    if iresp.on_hover_text("Display information window.").clicked() {
        app.dialogs.about = true;
    }
}

/// A knob as rekordbox draws them: a ring open at the bottom (260 degrees), the part up to
/// the value in `fill`, and a 2 pt pointer from the ring's outer edge inwards. `r`: the middle
/// of the ring, `t` its thickness.
pub(crate) fn ring_knob(p: &Painter, c: Pos2, r: f32, t: f32, v: f32, track: Color32, fill: Color32, pointer: Color32) {
    let ang = |x: f32| (140.0 + 260.0 * x).to_radians();
    let arc = |a: f32, b: f32, col: Color32| {
        let n = (((b - a) * 40.0).ceil() as usize).max(2);
        let pts: Vec<Pos2> = (0..=n).map(|i| { let g = ang(a + (b - a) * i as f32 / n as f32); c + vec2(g.cos(), g.sin()) * r }).collect();
        p.add(egui::Shape::line(pts, Stroke::new(t, col)));
    };
    arc(0.0, 1.0, track);
    if fill != track && v > 0.001 {
        arc(0.0, v.clamp(0.0, 1.0), fill);
    }
    let a = ang(v.clamp(0.0, 1.0));
    let (dir, nrm) = (vec2(a.cos(), a.sin()), vec2(-a.sin(), a.cos()));
    let (o, i) = (r + t / 2.0, r + t / 2.0 - 5.0);
    p.add(egui::Shape::convex_polygon(vec![c + dir * o + nrm, c + dir * i + nrm, c + dir * i - nrm, c + dir * o - nrm], pointer, Stroke::NONE));
}

// ------------------------------------------------------------------ screenshots and the scripted run

/// Screenshot states of the layouts (`PREPDECK_SHOT_VIEW=perfv|perf4v|perfbrowse|perfbrowse4|
/// perfsimple|perfsimple4|perfh|perf4h|perfnomixer|perfctl1|perfctlshut`). Returns false for
/// a name that is not one of them.
pub(crate) fn shot_view(app: &mut App, name: &str) -> bool {
    let (four, vertical, browse, hide) = match name {
        "perfh" => (false, false, false, false),
        "perf4h" => (true, false, false, false),
        "perfv" => (false, true, false, false),
        "perf4v" => (true, true, false, false),
        "perfbrowse" => (false, false, true, false),
        "perfbrowse4" => (true, false, true, false),
        "perfsimple" => (false, false, false, true),
        "perfsimple4" => (true, false, false, true),
        "perfnomixer" => {
            app.prefs.perf_lay.mixer_hidden = true;
            return true;
        }
        "perfctl1" => {
            app.prefs.perf_lay.ctl_page = 1;
            return true;
        }
        "perfctlshut" => {
            app.prefs.perf_lay.ctl_open = false;
            return true;
        }
        _ => return false,
    };
    app.prefs.app_mode = 1;
    app.prefs.perf_four = four;
    app.prefs.perf_lay = LayoutPrefs { vertical, browse, hide_platter: hide, ..Default::default() };
    true
}

/// `PREPDECK_SHOT_DECKS=all` (with `--screenshot`): once the first track is in, the other
/// decks of the layout get tracks too, each stopped at a different place with a hot cue, a
/// memory cue and a loop near it, so that every waveform has something to show.
pub(crate) fn shot_tick(app: &mut App) {
    thread_local! { static DONE: Cell<bool> = const { Cell::new(false) }; }
    if app.screenshot.is_none() || DONE.with(|d| d.get()) || std::env::var("PREPDECK_SHOT_DECKS").is_err() {
        return;
    }
    if app.deck_of(0).audio.is_none() && app.px.active == 0 {
        return;
    }
    DONE.with(|d| d.set(true));
    let have: Vec<i64> = (0..4).filter_map(|n| app.deck_of(n).track).collect();
    let ids: Vec<i64> = app.tracks.iter().filter(|t| t.analyzed == 1 && !t.missing && !have.contains(&t.id) && t.duration > 60.0).map(|t| t.id).take(3).collect();
    let count = layout(app).decks();
    for n in 0..count {
        if n > 0 {
            let Some(id) = ids.get(n as usize - 1).copied() else { continue };
            app.with_deck(n, |a| {
                a.load_track_here(id);
                a.shot_wait_load();
            });
        }
        app.with_deck(n, |a| {
            let beats = a.deck.beats.clone();
            let at = |i: usize| beats.get(i).map(|b| b.time);
            if let (Some(t0), Some(t1), Some(t2)) = (at(64 + 32 * n as usize), at(66 + 32 * n as usize), at(72 + 32 * n as usize)) {
                if !a.deck.cues.iter().any(|c| c.kind == CueKind::Hot) {
                    a.seek(t1);
                    a.hot_cue_down(n as i32);
                    a.hot_cue_up();
                }
                a.seek(t2);
                a.deck.cue_point = t2;
                a.seek(t0);
                a.deck.cue_point = t0;
            }
        });
    }
    // (`PREPDECK_SHOT_GRID=1`: every deck shows its GRID panel.)
    if std::env::var("PREPDECK_SHOT_GRID").is_ok() {
        for n in 0..4u8 {
            app.deck_of_mut(n).x.dk.grid_tab = true;
        }
    }
}

/// `PREPDECK_SHOT_PLAYER=laytest` (with `--screenshot`, a layout's `PREPDECK_SHOT_VIEW` and
/// `PREPDECK_BENCH=<frames>`): presses the main controls of the decks in the layout showing
/// with real pointer events, at the places the layout's own geometry gives, and prints the
/// state after each.
pub(crate) fn shot_layout_events(app: &mut App, frame: u32, raw: &mut egui::RawInput) {
    if !std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "laytest")).unwrap_or(false) || !app.performance() {
        return;
    }
    const START: u32 = 140;
    const SPAN: u32 = 30;
    if frame < START {
        return;
    }
    let steps = layout_steps(app);
    let i = ((frame - START) / SPAN) as usize;
    let f = (frame - START) % SPAN;
    let Some((name, a, b)) = steps.get(i).cloned() else { return };
    let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    match f {
        0 => raw.events.push(egui::Event::PointerMoved(a)),
        3 => raw.events.push(button(a, true)),
        4..=11 if a != b => raw.events.push(egui::Event::PointerMoved(a + (b - a) * ((f - 3) as f32 / 8.0))),
        12 => raw.events.push(button(b, false)),
        24 => eprintln!("laytest [{:?}] {name}: {}", layout(app), layout_state(app)),
        _ => {}
    }
}

/// The steps of the scripted run for the layout showing: (what, press at, release at).
fn layout_steps(app: &App) -> Vec<(String, Pos2, Pos2)> {
    let l = layout(app);
    let fx = if app.prefs.perf_fx && panel_enabled(app, 0) { crate::ui_fx::fx_h(app) } else { 0.0 };
    let area = Rect::from_min_size(pos2(0.0, 28.0 + fx), vec2(1512.0, player_height(app) + 4.0));
    let fr = frame(app, area);
    let mut v: Vec<(String, Pos2, Pos2)> = vec![];
    let mut click = |v: &mut Vec<(String, Pos2, Pos2)>, n: String, p: Pos2| v.push((n, p, p));
    let g = deck_geo(fr.dk);
    let extra = deck_extra(app);
    for n in 0..fr.count as usize {
        let dr = fr.decks[n];
        let right = n % 2 == 1;
        let d = n + 1;
        // Header: KEY SYNC, BEAT SYNC, MASTER, key shift up.
        click(&mut v, format!("deck {d} MASTER"), pos2(dr.max.x - 78.0 + 35.0, dr.min.y + g.y0 + 26.0 + 6.5));
        click(&mut v, format!("deck {d} BEAT SYNC"), pos2(dr.max.x - 78.0 + 22.0, dr.min.y + g.y0 + 11.0));
        click(&mut v, format!("deck {d} key shift up"), pos2(dr.max.x - 102.0 + 5.0, dr.min.y + g.y0 + 32.0));
        if fr.dk.has_lower() {
            let lo = lower_geo(fr.dk, dr.width(), right);
            let pg = platter_geo(fr.dk);
            let o = pos2(dr.min.x, dr.min.y + g.info_h + 3.0 + extra);
            let po = o + lo.panel_o.to_vec2();
            click(&mut v, format!("deck {d} PLAY/PAUSE (play)"), po + pg.play_c);
            click(&mut v, format!("deck {d} SLIP"), po + vec2(pg.col_x + 15.0, pg.ys[0] + 9.0));
            click(&mut v, format!("deck {d} Q"), po + vec2(pg.col_x + 15.0, pg.ys[1] + 9.0));
            click(&mut v, format!("deck {d} MT"), po + vec2(pg.col_x + 15.0, pg.ys[2] + 9.0));
            click(&mut v, format!("deck {d} loop size (start loop)"), o + lo.loop_o.to_vec2() + vec2(34.0, 16.0));
            click(&mut v, format!("deck {d} loop size (end loop)"), o + lo.loop_o.to_vec2() + vec2(34.0, 16.0));
            click(&mut v, format!("deck {d} loop double"), o + lo.loop_o.to_vec2() + vec2(51.0, 50.0));
            click(&mut v, format!("deck {d} pad 2"), o + lo.pads.min.to_vec2() + vec2(40.0, lo.pitch + lo.pad_h / 2.0));
            click(&mut v, format!("deck {d} tempo range"), po + pg.plat.min.to_vec2() + vec2(pg.tempo.max.x + pg.pct_w + 4.0 + 10.0, pg.tempo.center().y));
            click(&mut v, format!("deck {d} CUE"), po + pg.cue_c);
            click(&mut v, format!("deck {d} PLAY/PAUSE (play again)"), po + pg.play_c);
            click(&mut v, format!("deck {d} PLAY/PAUSE (pause)"), po + pg.play_c);
        }
    }
    // The mixer: fader of deck 1 dragged down, CUE of the last deck.
    if let Some(mr) = fr.mixer {
        let mg = mix_geo(fr.dk);
        if let Some(s) = mg.strips.iter().find(|s| s.deck == 0) {
            v.push(("channel fader 1 down".into(), pos2(mr.min.x + s.fader_x, mr.min.y + mg.fader.0), pos2(mr.min.x + s.fader_x, mr.min.y + mg.fader.0 + (mg.fader.1 - mg.fader.0) * 0.5)));
            v.push(("HIGH 1 to the left".into(), pos2(mr.min.x + s.eq_x, mr.min.y + mg.eq_y[0]), pos2(mr.min.x + s.eq_x, mr.min.y + mg.eq_y[0] + 80.0)));
        }
        if let Some(s) = mg.strips.last() {
            click(&mut v, format!("headphones CUE of channel {}", s.deck + 1), pos2(mr.min.x + s.fader_x, mr.min.y + mg.cue_y));
        }
    }
    // The control strip: zoom in twice, out once, reset; close and open it; the next page and back.
    if !matches!(l, Layout::Browse2 | Layout::Browse4) {
        let wa = fr.waves;
        let (zin, rst, zout, hide, next, prev) = if l.vertical() {
            let x = wa.min.x + (wa.width() - 80.0) / 2.0;
            (pos2(x + 7.0, wa.min.y + 9.0), pos2(wa.center().x, wa.min.y + 9.0), pos2(x + 73.0, wa.min.y + 9.0), pos2(wa.center().x, wa.min.y + 23.0), pos2(wa.max.x - 7.0, wa.min.y + 9.0), pos2(wa.min.x + 7.0, wa.min.y + 9.0))
        } else {
            let y = wa.min.y + ((wa.height() - 64.0) / 2.0).floor();
            (pos2(wa.min.x + 11.0, y + 7.0), pos2(wa.min.x + 11.0, wa.min.y + (wa.height() / 2.0).floor()), pos2(wa.min.x + 11.0, y + 57.0), pos2(wa.min.x + 27.0, wa.center().y), pos2(wa.min.x + 11.0, wa.max.y - 7.0), pos2(wa.min.x + 11.0, wa.min.y + 7.0))
        };
        click(&mut v, "zoom in".into(), zin);
        click(&mut v, "zoom in".into(), zin);
        click(&mut v, "zoom out".into(), zout);
        click(&mut v, "zoom RST".into(), rst);
        click(&mut v, "control strip: hide".into(), hide);
        click(&mut v, "control strip: show".into(), if l.vertical() { pos2(wa.center().x, wa.min.y + 5.0) } else { pos2(wa.min.x + 5.0, wa.center().y) });
        click(&mut v, "control strip: next page".into(), next);
        click(&mut v, "control strip: previous page".into(), prev);
        // (perfrest track) Page 1 of the strip: Matching (2-deck layouts) and Cut (capture
        // mode, which shows the sampler panel); a running loop of deck 1 dragged from its
        // waveform onto sampler slot 1; capture mode off; back to page 0.
        let two = fr.count == 2;
        let (m_at, cut_at) = if l.vertical() {
            let x0 = wa.min.x + (wa.width() - 48.0) / 2.0;
            (pos2(x0 + 7.0, wa.min.y + 9.0), pos2(if two { x0 + 41.0 } else { wa.center().x }, wa.min.y + 9.0))
        } else {
            let y0 = wa.min.y + ((wa.height() - 44.0) / 2.0).floor();
            (pos2(wa.min.x + 11.0, y0 + 7.0), pos2(wa.min.x + 11.0, if two { y0 + 37.0 } else { wa.min.y + ((wa.height() - 14.0) / 2.0).floor() + 7.0 }))
        };
        click(&mut v, "control strip: next page (matching and cut)".into(), next);
        if two {
            click(&mut v, "strip: Matching (decks 1 and 2 become a match)".into(), m_at);
            click(&mut v, "strip: Matching again (no longer a match)".into(), m_at);
        }
        click(&mut v, "strip: Cut (capture mode on, sampler panel shown)".into(), cut_at);
        if let Some(dr) = fr.decks.first() {
            if fr.dk.has_lower() {
                let lo = lower_geo(fr.dk, dr.width(), false);
                let o = pos2(dr.min.x, dr.min.y + g.info_h + 3.0 + extra);
                click(&mut v, "deck 1 loop size (start a loop to capture)".into(), o + lo.loop_o.to_vec2() + vec2(34.0, 16.0));
            }
        }
        let lane_pt = if l.vertical() { pos2(wa.min.x + if fr.count == 4 { 93.0 } else { 62.0 }, wa.center().y + 6.0) } else { pos2(wa.center().x + 8.0, wa.min.y + if fr.count == 4 { 25.0 } else { 30.0 }) };
        if let Some(s) = LAST_SAMPLER.with(|c| c.get()).or(Some(Rect::from_min_max(pos2(0.0, 917.0 - crate::ui_sampler::SAMPLER_H), pos2(1512.0, 917.0)))) {
            let slot_w = ((s.width() - 80.0) / 8.0).floor().max(146.0);
            v.push(("capture drag: deck 1's loop onto sampler slot 1".into(), lane_pt, pos2(s.min.x + 4.0 + slot_w / 2.0, s.min.y + 4.0 + 22.0)));
        }
        click(&mut v, "strip: Cut (capture mode off)".into(), cut_at);
        click(&mut v, "control strip: previous page".into(), prev);
        // A cue marker of deck 1 dragged along the vertical waveform: hot cue pad A first (the
        // deck goes to the cue, so its letter box is at the playhead), then its box dragged
        // 20 pt down.
        if l.vertical() {
            if let Some(dr) = fr.decks.first() {
                let lo = lower_geo(fr.dk, dr.width(), false);
                let o = pos2(dr.min.x, dr.min.y + g.info_h + 3.0 + extra);
                click(&mut v, "deck 1 hot cue pad A (to the cue)".into(), o + lo.pads.min.to_vec2() + vec2(40.0, lo.pad_h / 2.0));
            }
            let lane_w = wa.width() / fr.count as f32;
            let i = if fr.count == 4 { 1.0 } else { 0.0 };
            let (top, bot) = (wa.min.y + if fr.count == 4 { 18.0 } else { 0.0 }, wa.max.y - 14.0);
            let x = wa.min.x + lane_w * i + 6.5;
            let cy = (top + bot) / 2.0;
            v.push(("deck 1 hot cue marker dragged down the vertical waveform".into(), pos2(x, cy), pos2(x, cy + 20.0)));
        }
        // Scrub the first waveform: drag along its time axis.
        let (a, b) = if l.vertical() { (pos2(wa.min.x + if fr.count == 4 { 93.0 } else { 62.0 }, wa.center().y + 10.0), pos2(wa.min.x + if fr.count == 4 { 93.0 } else { 62.0 }, wa.center().y + 40.0)) } else { (pos2(wa.center().x + 200.0, wa.min.y + 30.0), pos2(wa.center().x + 260.0, wa.min.y + 30.0)) };
        v.push(("waveform of deck 1 dragged back".into(), a, b));
    }
    v
}

/// What the scripted run prints after each step.
fn layout_state(app: &App) -> String {
    let n = layout(app).decks() as usize;
    let lp = &app.prefs.perf_lay;
    let decks: Vec<String> = (0..n)
        .map(|i| {
            let (d, s) = (app.deck_of(i as u8), app.player.decks[i].lock());
            let secs = s.audio.as_ref().map(|a| s.pos / a.sr as f64).unwrap_or(0.0);
            let q = if app.px.active as usize == i { app.prefs.quantize } else { app.px.parked_quantize[i] };
            format!("{}:{}{} {:.2}s loop {} slip {} Q {} MT {} sync {} key {:+} range {} hot {}", i + 1, if d.track.is_some() { "" } else { "empty " }, if s.playing { "playing" } else { "paused" }, secs, s.loop_range.is_some(), d.x.slip, q, d.x.master_tempo, d.x.beat_sync, d.x.key_shift, d.x.tempo_range, d.cues.iter().filter(|c| c.kind == CueKind::Hot).count())
        })
        .collect();
    let matched = matches!((app.deck_of(0).track, app.deck_of(1).track), (Some(a), Some(b)) if app.is_match(a, b));
    let slot1 = app.prefs.sampler.slots.get(app.prefs.sampler.bank.min(3) as usize * 16).and_then(|x| x.as_ref()).map(|x| x.name.clone());
    let hot_a = app.deck_of(0).cues.iter().filter(|c| c.kind == CueKind::Hot).map(|c| (c.start * 1000.0).round() / 1000.0).next();
    format!("master {} active {} | zoom {:?} strip open {} page {} | fader1 {:.2} high1 {:.2} cue{} {} | matching {} capture {} sampler panel {} slot 1 {:?} deck 1 first hot cue {:?} | last message {:?} | {}", app.px.master + 1, app.px.active + 1, lp.zoom, lp.ctl_open, lp.ctl_page, app.px.mix[0].fader, app.px.mix[0].eq[2], n, app.px.mix[n - 1].cue, matched, capture_mode(), app.prefs.perf_sampler, slot1, hot_a, app.toasts.last().map(|t| t.text.as_str()), decks.join(" | "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_hit_region_uses_the_rendered_playhead_and_scale_on_each_axis() {
        let lane = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 140.0));
        // Rendered head time 10.25 s, rendered scale 80 pt/s. The capture spans
        // 9.75–10.5 s. Both marker endpoints must lie inside the capture target.
        assert_eq!(capture_region(lane, false, 10.25, 80.0, 9.75, 10.5),
                   Rect::from_min_max(pos2(70.0, 20.0), pos2(130.0, 140.0)));
        assert_eq!(capture_region(lane, true, 10.25, 80.0, 9.75, 10.5),
                   Rect::from_min_max(pos2(10.0, 40.0), pos2(210.0, 100.0)));
        assert_eq!(capture_region(lane, true, 10.25, 80.0, 9.0, 11.0), lane);
        assert!(capture_region(lane, false, 10.25, 80.0, 12.0, 13.0).width() <= 0.0);
    }

    #[test]
    fn layout_modes_and_selector_rows_are_rekordboxs() {
        let mut l = LayoutPrefs::default();
        let mut four = false;
        assert_eq!(layout_of(four, &l).mode(), 18);
        let modes: Vec<u32> = (0..5).map(|row| { choose_row(&mut four, &mut l, row); layout_of(four, &l).mode() }).collect();
        // Browse after 4Deck Vertical keeps four decks.
        assert_eq!(modes, vec![18, 274, 20, 276, 4100]);
        choose_row(&mut four, &mut l, 1);
        choose_row(&mut four, &mut l, 4);
        assert_eq!(layout_of(four, &l).mode(), 4098);
        // HidePlatter selects 4114 / 4116 for the horizontal rows only.
        l.hide_platter = true;
        choose_row(&mut four, &mut l, 0);
        assert_eq!(layout_of(four, &l).mode(), 4114);
        choose_row(&mut four, &mut l, 2);
        assert_eq!(layout_of(four, &l).mode(), 4116);
        choose_row(&mut four, &mut l, 3);
        assert_eq!(layout_of(four, &l).mode(), 276);
        for (lay, decks, row) in [(Layout::H2, 2, 0), (Layout::V2, 2, 1), (Layout::H4, 4, 2), (Layout::V4, 4, 3), (Layout::Browse2, 2, 4), (Layout::Browse4, 4, 4), (Layout::Simple4, 4, 2)] {
            assert_eq!((lay.decks(), lay.row()), (decks, row), "{lay:?}");
        }
        assert!(Layout::V4.vertical() && Layout::V4.full() && !Layout::Browse4.full() && Layout::Simple2.simple() && Layout::Simple2.full());
    }

    #[test]
    fn layout_prefs_survive_a_round_trip_and_old_files() {
        let l = LayoutPrefs { vertical: true, zoom: [1, 6, 3], ctl_open: false, ..Default::default() };
        let back: LayoutPrefs = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert!(back.vertical && back.zoom == [1, 6, 3] && !back.ctl_open);
        // A settings file from before the layouts: all defaults (2Deck Horizontal, RST zoom).
        let old: LayoutPrefs = serde_json::from_str("{}").unwrap();
        assert!(!old.vertical && !old.browse && old.zoom == [ZOOM_RESET; 3] && old.ctl_open);
    }

    #[test]
    fn frames_match_the_captures_at_1512_pt() {
        let area = Rect::from_min_size(pos2(0.0, 28.0), vec2(1512.0, 900.0));
        let r = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x, 28.0 + y), vec2(w, h));
        // 2Deck Horizontal (capture 01): decks 700 wide at 0 and 812, 142 pt down; mixer 703..809.
        let f = frame_of(Layout::H2, area, 0.0, true);
        assert_eq!((f.decks[0], f.decks[1]), (r(0.0, 142.0, 700.0, 227.0), r(812.0, 142.0, 700.0, 227.0)));
        assert_eq!((f.mixer, f.bottom, f.waves), (Some(r(703.0, 142.0, 106.0, 227.0)), Some(r(0.0, 372.0, 1512.0, 40.0)), r(0.0, 1.0, 1512.0, 138.0)));
        // 4Deck Horizontal (03): decks 658 wide at 0 and 854, rows at 198 and 389; mixer 661..851.
        let f = frame_of(Layout::H4, area, 0.0, true);
        assert_eq!((f.decks[0], f.decks[3]), (r(0.0, 198.0, 658.0, 188.0), r(854.0, 389.0, 658.0, 188.0)));
        assert_eq!((f.mixer, f.bottom), (Some(r(661.0, 198.0, 190.0, 379.0)), Some(r(0.0, 580.0, 1512.0, 40.0))));
        // 2Deck Vertical (02): decks 629 wide at 0 and 883; waveforms 632..880 above the mixer at 151.
        let f = frame_of(Layout::V2, area, 0.0, true);
        assert_eq!((f.decks[0], f.decks[1]), (r(0.0, 1.0, 629.0, 297.0), r(883.0, 1.0, 629.0, 297.0)));
        assert_eq!((f.waves, f.mixer, f.bottom), (r(632.0, 1.0, 248.0, 147.0), Some(r(632.0, 151.0, 248.0, 147.0)), Some(r(0.0, 301.0, 1512.0, 40.0))));
        // 4Deck Vertical (04): rows at 1 and 210, mixer from 182, bottom row at 419.
        let f = frame_of(Layout::V4, area, 0.0, true);
        assert_eq!((f.decks[2], f.mixer, f.bottom), (r(0.0, 210.0, 629.0, 206.0), Some(r(632.0, 182.0, 248.0, 234.0)), Some(r(0.0, 419.0, 1512.0, 40.0))));
        // Browse (05): 754 and 755 wide, rows at 1 and 99.
        let f = frame_of(Layout::Browse4, area, 0.0, false);
        assert_eq!((f.decks[0], f.decks[3]), (r(0.0, 1.0, 754.0, 95.0), r(757.0, 99.0, 755.0, 95.0)));
    }

    #[test]
    fn bar_count_runs_backwards_before_the_first_bar() {
        // A grid whose first beat is the second of a bar: beats 2, 3, 4, then bar 1.
        let beats: Vec<Beat> = (0..12).map(|i| Beat { time: 0.37 + i as f64 * 0.4615, bpm: 130.0, beat: ((i + 1) % 4 + 1) as u8, index: i }).collect();
        assert_eq!(bars_text(&beats, 0.37), "-1.2Bars");
        assert_eq!(bars_text(&beats, 0.37 + 0.4615), "-1.3Bars");
        assert_eq!(bars_text(&beats, 0.37 + 2.0 * 0.4615), "-1.4Bars");
        assert_eq!(bars_text(&beats, 0.37 + 3.0 * 0.4615), "1.1Bars");
        assert_eq!(bars_text(&beats, 0.37 + 8.2 * 0.4615), "2.2Bars");
        assert_eq!(bars_text(&[], 1.0), "");
    }

    #[test]
    fn zoom_steps_are_rekordboxs_seven_per_layout() {
        assert_eq!(ZOOM_MS[0][ZOOM_RESET as usize], 8000.0);
        assert_eq!(ZOOM_MS[1][ZOOM_RESET as usize], 2000.0);
        assert_eq!(ZOOM_MS[2][ZOOM_RESET as usize], 4000.0);
        for f in ZOOM_MS {
            assert!(f.windows(2).all(|w| w[0] < w[1]));
        }
        assert_eq!((zoom_family(Layout::H4), zoom_family(Layout::Simple2), zoom_family(Layout::V2), zoom_family(Layout::V4)), (0, 0, 1, 2));
    }
}

#[cfg(test)]
mod light_tests {
    #[test]
    fn recolour_svg_swaps_and_ramps() {
        let s = r##"<rect fill="#282828"/><path fill="white" stroke="#2DD728"/><use href="#abcdef1"/>"##;
        let sw = super::recolour_svg(s, "swap:282828>C8C8C8,FFFFFF>000000");
        assert!(sw.contains(r##"fill="#C8C8C8""##) && sw.contains(r##"fill="#000000""##) && sw.contains("#2DD728") && sw.contains("#abcdef1"));
        let l = super::recolour_svg(s, "light");
        assert!(l.contains(r##"fill="#C8C8C8""##) && l.contains(r##"fill="#000000""##) && l.contains("#2DD728"));
        let gradient = r##"<linearGradient id="FFFFFF"><stop style="stop-color: white;" /></linearGradient><use href="#FFFFFF" fill="url(#FFFFFF)" stroke='black'/>"##;
        let themed = super::recolour_svg(gradient, "light");
        assert!(themed.contains(r##"href="#FFFFFF" fill="url(#FFFFFF)""##));
        assert!(themed.contains("stop-color: #000000;") && themed.contains("stroke='#FFFFFF'"));
    }
}
