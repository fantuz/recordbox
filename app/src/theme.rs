//! Colours and sizes measured from the original's skins and screenshots (sRGB), and the
//! Dark / Light appearance (rekordbox's `ColorThemeMode` 1 / 2).
//!
//! A themed colour is a [`Tc`]: its Dark and its Light value. The appearance is one flag
//! ([`is_light`]), set once per frame by [`sync`]; `Tc::c()` picks the value, so reading a
//! token costs a load and a select and Dark gets exactly the values it had before. Light
//! values come from rekordbox's colour table (`colorTheme::colorThemeColourId`, 306 ids,
//! `checks/re-7.2.19/preferences-skins-colours.tsv`); each token names the id it was matched to
//! (`checks/ui-compare/light-2026-11-28/colour-inventory.tsv` has every site). Colours rekordbox
//! keeps in both appearances (waveforms, hot cues, the accent blue, selection) are `fixed`.

use eframe::egui;
use egui::Color32;
use std::cell::Cell;

pub const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

// Painting and its temporary selected-row scopes belong to the current UI thread.
thread_local! { static LIGHT: Cell<bool> = const { Cell::new(false) }; }

/// The Light appearance is drawn this frame.
#[inline(always)]
pub fn is_light() -> bool {
    LIGHT.get()
}

/// A themed colour: `.0` in Dark, `.1` in Light.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tc(pub Color32, pub Color32);

/// A themed colour from two `0xRRGGBB` values (Dark, Light).
pub const fn tc(dark: u32, light: u32) -> Tc {
    Tc(hex(dark), hex(light))
}

/// A colour that is the same in both appearances.
pub const fn fixed(v: u32) -> Tc {
    Tc(hex(v), hex(v))
}

impl Tc {
    /// The value for the appearance drawn this frame.
    #[inline(always)]
    pub fn c(self) -> Color32 {
        self.pick(is_light())
    }
    /// The value for the given appearance.
    #[inline(always)]
    pub fn pick(self, light: bool) -> Color32 {
        if light { self.1 } else { self.0 }
    }
    #[inline]
    pub fn gamma_multiply(self, f: f32) -> Color32 {
        self.c().gamma_multiply(f)
    }
    #[inline]
    pub fn linear_multiply(self, f: f32) -> Color32 {
        self.c().linear_multiply(f)
    }
    #[inline]
    pub fn additive(self) -> Color32 {
        self.c().additive()
    }
    #[inline]
    pub fn r(self) -> u8 {
        self.c().r()
    }
    #[inline]
    pub fn g(self) -> u8 {
        self.c().g()
    }
    #[inline]
    pub fn b(self) -> u8 {
        self.c().b()
    }
    #[inline]
    pub fn a(self) -> u8 {
        self.c().a()
    }
    #[inline]
    pub fn to_array(self) -> [u8; 4] {
        self.c().to_array()
    }
    #[inline]
    pub fn lerp_to_gamma(self, other: Color32, t: f32) -> Color32 {
        self.c().lerp_to_gamma(other, t)
    }
}

impl From<Tc> for Color32 {
    #[inline(always)]
    fn from(t: Tc) -> Color32 {
        t.c()
    }
}

impl From<Tc> for egui::Rgba {
    #[inline]
    fn from(t: Tc) -> egui::Rgba {
        t.c().into()
    }
}

impl PartialEq<Color32> for Tc {
    fn eq(&self, o: &Color32) -> bool {
        self.c() == *o
    }
}

impl PartialEq<Tc> for Color32 {
    fn eq(&self, o: &Tc) -> bool {
        *self == o.c()
    }
}

/// While alive, tokens resolve to their Dark values (when `on`). rekordbox fills a selected row
/// with fixed blues (#1473EB focused, #294C69 not: `ListViewer::paintRowBackground`
/// 0x1003f24c0, `CustomItemComponent::paint` 0x100a21534) and draws its text white in both
/// appearances (`ListViewer::findTextColour` 0x1003efb70, `CustomItemComponent::findItemTextColour`
/// 0x100a21374), so a selected row is drawn as in Dark.
pub struct DarkScope(Option<bool>);

pub fn dark_scope(on: bool) -> DarkScope {
    DarkScope(on.then(|| LIGHT.replace(false)))
}

impl Drop for DarkScope {
    fn drop(&mut self) {
        if let Some(previous) = self.0 {
            LIGHT.set(previous);
        }
    }
}

/// Sets the appearance for this frame (`ColorThemeMode` 2 = Light). rekordbox draws EDIT mode
/// in Dark whatever the setting (`MainComponent::changeModeToEdit` 0x10098865c), and applies a
/// change at once, without a restart (`MainComponent::replaceCustomColorTheme` 0x100975dbc).
pub fn sync(ctx: &egui::Context, light: bool) {
    LIGHT.set(light);
    let current = ctx.style();
    if current.visuals.dark_mode == light {
        let mut style = (*current).clone();
        visuals(&mut style.visuals);
        ctx.set_style(style);
        ctx.request_repaint();
    }
}

/// The Light value of a grey `0xRRGGBB` drawn where no rekordbox colour id was traced for the
/// surface: what most of the table's ids with the nearest Dark grey turn into (counts are
/// "ids with that Light value / ids with that Dark grey" among the 0x030000xx rows). Colours
/// that are not grey are not themed and keep their value.
pub const fn ramp(v: u32) -> u32 {
    let (r, g, b) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
    let (lo, hi) = (min3(r, g, b), max3(r, g, b));
    // (Near-greys such as #242427 count as the grey of their mean.)
    if hi - lo > 6 {
        return v;
    }
    match (r + g + b) / 3 {
        0x00..=0x07 => 0xFFFFFF, // #000000: #FFFFFF 18/33
        0x08..=0x14 => 0xFFFFFF, // #0F0F0F: #FFFFFF 5/9
        0x15..=0x1B => 0xFFFFFF, // #191919: #FFFFFF 6/12
        0x1C..=0x23 => 0xF0F0F0, // #1E1E1E: #F0F0F0 10/11
        0x24..=0x2D => 0xC8C8C8, // #282828: #C8C8C8 14/16
        0x2E..=0x3F => 0xBEBEBE, // #323232: #BEBEBE 11/40 (#C8C8C8 10)
        0x40..=0x4F => 0xC8C8C8, // #4B4B4B: #C8C8C8 2/5
        0x50..=0x5C => 0x787878, // #555555: #787878 2/5 (#646464 2)
        0x5D..=0x6E => 0x000000, // #646464: #000000 10/19
        0x6F..=0x79 => 0x787878, // #787878: #787878 1/1
        0x7A..=0x81 => 0x000000, // #7D7D7D: #000000 7/10
        0x82..=0x8E => 0x000000, // #878787: #000000 8/9
        0x8F..=0x9A => 0x000000, // #969696: #000000 8/14
        0x9B..=0xA4 => 0x000000, // #A0A0A0: #000000 4/4
        0xA5..=0xB8 => 0x9B9B9B, // #AAAAAA: #9B9B9B 2/2
        0xB9..=0xD4 => 0x323232, // #C8C8C8: #323232 1/2 (#C8C8C8 1)
        _ => 0x000000,           // #E1E1E1, #F0F0F0, #FFFFFF: #000000 1/1, 1/1, 44/56
    }
}

const fn min3(a: u32, b: u32, c: u32) -> u32 {
    let m = if a < b { a } else { b };
    if m < c { m } else { c }
}

const fn max3(a: u32, b: u32, c: u32) -> u32 {
    let m = if a > b { a } else { b };
    if m > c { m } else { c }
}

/// A grey themed by [`ramp`].
pub const fn gr(v: u32) -> Tc {
    tc(v, ramp(v))
}

/// A grey themed by [`ramp`], for the appearance drawn now.
#[inline(always)]
pub fn gy(v: u32) -> Color32 {
    gr(v).c()
}

// ---- The palette. Dark values are what recordbox drew before the Light appearance; the
// Light value is the one rekordbox's table gives the colour id named (D = dark, L = light).

/// Panel background: id 0x03000008 (`PlayerPanelL::paint`, `BrowseBasicView::paint`, 83
/// users) D #000000 L #FFFFFF.
pub const BG: Tc = tc(0x000000, 0xFFFFFF);
/// Preferences' category and tab labels (not selected): `PreferenceCategoryComponent::
/// lookAndFeelChanged` 0x10136e690 sets 0x0300011A, L #000000 (D: recordbox's measured #AAAAAA;
/// the table has #646464); selected labels are 0x03000119, white in both (ON_ACCENT).
pub const PREF_CAT: Tc = tc(0xAAAAAA, 0x000000);
/// Labels of the information window: `browse::InfoComponent::InfoComponent` sets 0x01000284,
/// L #000000 (D: recordbox's #AAAAAA).
pub const INFO_LBL: Tc = tc(0xAAAAAA, 0x000000);
/// The header bar (mode, layout, panel toggles, clock): 0x0300002D, the colour
/// `MainComponent::paint` 0x100961ef0 sets last, D #0F0F0F L #F0F0F0.
pub const HEADER: Tc = tc(0x0F0F0F, 0xF0F0F0);
/// Text fields of the browser (search): 0x030000D4 (`browse::SearchFilter::SearchFilter`)
/// L #E1E1E1; its Dark value is transparent over recordbox's black field.
pub const FIELD: Tc = tc(0x000000, 0xE1E1E1);
/// A sampler slot's face: 0x030000B1 (`SamplerControlComp::paint`) D #000000 L #FFFFFF.
pub const SLOT: Tc = tc(0x000000, 0xFFFFFF);
/// Even list rows: 0x030000CA (`browse::ListViewer::paintRowBackground`) D #0F0F0F L #FFFFFF.
pub const BG1: Tc = tc(0x0F0F0F, 0xFFFFFF);
/// Item / odd row / pane background: 0x03000009 (242 users, Preferences items, browser panes)
/// and 0x030000CB (odd list rows) D #1E1E1E L #F0F0F0.
pub const BG2: Tc = tc(0x1E1E1E, 0xF0F0F0);
/// Button face (off): 0x03000012 (`TrackListHeader`, `AttributeFilterList`, `AuthButton`)
/// D #282828 L #C8C8C8.
pub const BTN: Tc = tc(0x282828, 0xC8C8C8);
/// Frames, scroll thumbs, filter buttons: 0x03000011 (`BrowseInfoView`, `BrowseMytagFilter`,
/// `ResetColumnFilterButton`) D #323232 L #C8C8C8.
pub const SCROLL: Tc = tc(0x323232, 0xC8C8C8);
/// Dim / disabled text and glyphs: 0x03000021 ("Not Loaded.") D #646464 L #646464.
pub const OFF: Tc = tc(0x646464, 0x646464);
/// Secondary text: 0x01000284 (68 users) D #969696 L #000000.
pub const DIM: Tc = tc(0x969696, 0x000000);
/// Captions ("BPM"): 0x030000A3 D #AAAAAA L #9B9B9B.
pub const DIM2: Tc = tc(0xAAAAAA, 0x9B9B9B);
/// Body text: 0x0300009D D #F0F0F0 L #000000.
pub const TEXT: Tc = tc(0xF0F0F0, 0x000000);
/// Text and glyphs: 0x0300000B (404 users) D #FFFFFF L #000000.
pub const WHITE: Tc = tc(0xFFFFFF, 0x000000);
/// White that stays white (on the accent blue, on coloured pads): 0x03000064 D/L #FFFFFF.
pub const ON_ACCENT: Tc = fixed(0xFFFFFF);
/// The accent blue (0x03000015, 0x03000023, …: the same in both).
pub const BLUE: Tc = fixed(0x1473EB);
pub const BEAT_BLUE: Tc = fixed(0x007DE1);
pub const SELECT: Tc = fixed(0x294C6A);
/// Selected row in rekordbox's browser (tree and track list), sampled from the references.
/// Not themed: `ListViewer::paintRowBackground` fills selected rows with fixed constants.
pub const ROW_SEL: Tc = fixed(0x294C6A);
/// Lit source tile / "Tree View" button in the browser.
pub const TILE_ON: Tc = fixed(0x1473EB);
/// Behind the waveforms: black in both appearances (`PlayerWaveView::paint` 0x100c8f978 fills
/// `Colour(0, 0, 0)`; EXPORT's `PlayerPanelL::paint` 0x102282eb4 fills the waveform rectangle
/// with `Colours::black`, the rest of the player is 0x03000008).
pub const WAVE_BG: Tc = fixed(0x000000);
/// The zoom buttons' strip over the waveform: 0x01000900 (`WaveViewArea::ZoomControlView::paint`
/// 0x100c96f7c) D #000000 in recordbox (#0F0F0F in the table) L #FFFFFF.
pub const ZOOM_BG: Tc = tc(0x000000, 0xFFFFFF);
/// rekordbox's tree glyph grey (skins: rgb 160): 0x03000025 (`BrowseTreeShortcut::createIcon`)
/// D #A0A0A0 L #000000.
pub const TREE_ICON: Tc = tc(0xA0A0A0, 0x000000);
pub const CUE_ORANGE: Color32 = hex(0xFFB937);
pub const PLAY_GREEN: Color32 = hex(0x2DD728);
pub const CUE_MARK: Color32 = hex(0xF8A030);
pub const LOOP: Color32 = hex(0xFF8205);
pub const BAR_RED: Color32 = hex(0xFF0000);
pub const CAUTION_BG: Color32 = hex(0x571718);
pub const CAUTION_TX: Color32 = hex(0xE04848);
pub const WAVE_LOW: Color32 = hex(0x0055E1);
pub const WAVE_MID: Color32 = hex(0xFFA202);
pub const WAVE_HIGH: Color32 = hex(0xF4EAD5);
pub const WAVE_BLUE: Color32 = hex(0x1E78FF);

pub fn label_color(i: u8) -> Color32 {
    let c = crate::model::COLOR_RGB[(i as usize).min(8)];
    Color32::from_rgb(c[0], c[1], c[2])
}

pub fn hotcue_color(i: u8) -> Color32 {
    let c = crate::model::hotcue_rgb(i);
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Caps-label font (Arial Bold) at `size` pt.
pub fn caps(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("caps".into()))
}

/// Bold system font at `size` pt.
pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("bold".into()))
}

/// rekordbox's browser face (Helvetica; the references fit it at 10.5 pt) at `size` pt.
pub fn rb(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("rb".into()))
}

/// Bold of the browser face.
pub fn rb_bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("rb-bold".into()))
}

pub fn apply(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, path, idx) in [
        ("sf", "/System/Library/Fonts/SFNS.ttf", 0u32),
        ("helv", "/System/Library/Fonts/HelveticaNeue.ttc", 0),
        ("hiragino", "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", 0),
    ] {
        if let Ok(b) = std::fs::read(path) {
            let mut fd = egui::FontData::from_owned(b);
            fd.index = idx;
            fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(fd));
            let fam = fonts.families.entry(egui::FontFamily::Proportional).or_default();
            if name == "hiragino" {
                fam.push(name.to_owned());
            } else if name == "helv" {
                fam.insert(0, name.to_owned());
            }
        }
    }
    // Bold face (Helvetica Neue Bold, face 1 of the system collection) for titles and caps labels.
    if let Ok(b) = std::fs::read("/System/Library/Fonts/HelveticaNeue.ttc") {
        let mut fd = egui::FontData::from_owned(b);
        fd.index = 1;
        fonts.font_data.insert("helv-bold".to_owned(), std::sync::Arc::new(fd));
        let mut fam = vec!["helv-bold".to_owned()];
        fam.extend(fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default());
        fonts.families.insert(egui::FontFamily::Name("bold".into()), fam);
    } else {
        let fam = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
        fonts.families.insert(egui::FontFamily::Name("bold".into()), fam);
    }
    // Inter (SIL Open Font License, assets/fonts): the face of the Preferences window and the
    // dialogs, as in the original. Families "inter" and "inter-bold" (see ui_kit.rs).
    for (name, bytes) in [("inter", &include_bytes!("../assets/fonts/Inter-Regular.ttf")[..]), ("inter-bold", &include_bytes!("../assets/fonts/Inter-Bold.ttf")[..])] {
        fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(egui::FontData::from_static(bytes)));
        let mut fam = vec![name.to_owned()];
        fam.extend(fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default());
        fonts.families.insert(egui::FontFamily::Name(name.into()), fam);
    }
    // The face of rekordbox's Preferences window (families "pref" and "pref-bold", see prefs_font.rs).
    crate::prefs_font::install(&mut fonts);
    // The browser's face: Helvetica regular and bold (faces 0 and 1 of the system collection),
    // falling back to the proportional list.
    for (fam, face, idx) in [("rb", "rb-regular", 0u32), ("rb-bold", "rb-boldface", 1)] {
        let mut list = vec![];
        if let Ok(b) = std::fs::read("/System/Library/Fonts/Helvetica.ttc") {
            let mut fd = egui::FontData::from_owned(b);
            fd.index = idx;
            fonts.font_data.insert(face.to_owned(), std::sync::Arc::new(fd));
            list.push(face.to_owned());
        }
        let base = if idx == 1 { egui::FontFamily::Name("bold".into()) } else { egui::FontFamily::Proportional };
        list.extend(fonts.families.get(&base).cloned().unwrap_or_default());
        fonts.families.insert(egui::FontFamily::Name(fam.into()), list);
    }
    // Caps labels of the header and the player (EXPORT, CUE, MEMORY, …): Arial Bold, the face of
    // rekordbox's own button artwork. Falls back to the bold family when it is not installed.
    let mut caps_fam = fonts.families.get(&egui::FontFamily::Name("bold".into())).cloned().unwrap_or_default();
    if let Ok(b) = std::fs::read("/System/Library/Fonts/Supplemental/Arial Bold.ttf") {
        fonts.font_data.insert("arial-bold".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(b)));
        caps_fam.insert(0, "arial-bold".to_owned());
    }
    fonts.families.insert(egui::FontFamily::Name("caps".into()), caps_fam);
    // Arial regular: the face of rekordbox's Performance decks (ui_perf_layouts.rs).
    let mut arial_fam = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    if let Ok(b) = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf") {
        fonts.font_data.insert("arial-regular".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(b)));
        arial_fam.insert(0, "arial-regular".to_owned());
    }
    fonts.families.insert(egui::FontFamily::Name("arial".into()), arial_fam);
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::new(10.0, Proportional)),
        (TextStyle::Body, FontId::new(12.0, Proportional)),
        (TextStyle::Button, FontId::new(12.0, Proportional)),
        (TextStyle::Heading, FontId::new(16.0, Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, egui::FontFamily::Monospace)),
    ]
    .into();
    visuals(&mut style.visuals);
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.scroll.bar_width = 8.0;
    ctx.set_style(style);
}

/// Hovered item of egui's own widgets (menu and list items): `PopupMenu::highlightedBackground`
/// 0x01000900, L #FFFFFF.
pub const HOVER: Tc = tc(0x3A3A3A, 0xFFFFFF);
/// egui's window outline. L: no JUCE id; #BEBEBE, the table's light value for the #323232
/// frames (0x03000030).
pub const WIN_EDGE: Tc = tc(0x3A3A3A, 0xBEBEBE);
/// egui's scroll bar track and text field: `ScrollBar::backgroundColourId` 0x01000300 (and the
/// browser's search field 0x030000D4), L #E1E1E1.
pub const EXTREME: Tc = tc(0x000000, 0xE1E1E1);
/// egui's scroll bar thumb and plain button face: `ScrollBar::thumbColourId` 0x01000400,
/// L #969696.
pub const THUMB: Tc = tc(0x282828, 0x969696);

/// egui's own widget colours for the appearance drawn now.
fn visuals(v: &mut egui::Visuals) {
    *v = if is_light() { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = BG2.c();
    v.window_fill = BG2.c();
    v.extreme_bg_color = EXTREME.c();
    v.faint_bg_color = BG1.c();
    v.override_text_color = Some(TEXT.c());
    v.selection.bg_fill = BLUE.c();
    v.selection.stroke = egui::Stroke::new(1.0, ON_ACCENT);
    v.widgets.noninteractive.bg_fill = BG2.c();
    v.widgets.inactive.bg_fill = THUMB.c();
    v.widgets.inactive.weak_bg_fill = BTN.c();
    v.widgets.hovered.bg_fill = HOVER.c();
    v.widgets.hovered.weak_bg_fill = HOVER.c();
    v.widgets.active.bg_fill = BLUE.c();
    v.widgets.active.weak_bg_fill = BLUE.c();
    v.window_stroke = egui::Stroke::new(1.0, WIN_EDGE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_values_are_the_old_constants() {
        // The Dark side of every themed constant is the colour recordbox drew before.
        for (t, v) in [(BG, 0x000000), (BG1, 0x0F0F0F), (BG2, 0x1E1E1E), (BTN, 0x282828), (SCROLL, 0x323232), (OFF, 0x646464), (DIM, 0x969696), (DIM2, 0xAAAAAA), (TEXT, 0xF0F0F0), (WHITE, 0xFFFFFF), (BLUE, 0x1473EB), (ROW_SEL, 0x294C6A), (TREE_ICON, 0xA0A0A0), (HEADER, 0x0F0F0F)] {
            assert_eq!(t.0, hex(v));
        }
        assert_eq!(gr(0x3A3A3A).0, hex(0x3A3A3A));
    }

    #[test]
    fn ramp_follows_the_table() {
        assert_eq!(ramp(0x000000), 0xFFFFFF);
        assert_eq!(ramp(0x1E1E1E), 0xF0F0F0);
        assert_eq!(ramp(0x282828), 0xC8C8C8);
        assert_eq!(ramp(0x323232), 0xBEBEBE);
        assert_eq!(ramp(0x969696), 0x000000);
        assert_eq!(ramp(0xAAAAAA), 0x9B9B9B);
        assert_eq!(ramp(0xFFFFFF), 0x000000);
        // near-grey: the grey of its mean; colours are not themed
        assert_eq!(ramp(0x242427), 0xC8C8C8);
        assert_eq!(ramp(0x1473EB), 0x1473EB);
        assert_eq!(ramp(0xFFB937), 0xFFB937);
    }

    #[test]
    fn tc_picks_by_appearance() {
        let t = tc(0x101010, 0xEEEEEE);
        assert_eq!(t.pick(false), hex(0x101010));
        assert_eq!(t.pick(true), hex(0xEEEEEE));
        assert_eq!(WAVE_BG.pick(true), hex(0x000000));
    }

    #[test]
    fn selected_rows_restore_the_appearance_and_independent_contexts_sync() {
        let a = egui::Context::default();
        let b = egui::Context::default();
        for ctx in [&a, &b] {
            ctx.set_visuals(egui::Visuals::dark());
            sync(ctx, true);
            assert_eq!(ctx.style().visuals.panel_fill, BG2.pick(true));
            assert_eq!(WHITE.c(), Color32::BLACK);
            {
                let _selected = dark_scope(true);
                let _nested = dark_scope(false);
                assert_eq!(WHITE.c(), Color32::WHITE);
            }
            assert_eq!(WHITE.c(), Color32::BLACK);
        }
        sync(&a, false);
        assert!(a.style().visuals.dark_mode);
        assert!(!b.style().visuals.dark_mode);
    }
}
