//! Colours and sizes measured from the original's skins and screenshots (sRGB).

use eframe::egui;
use egui::Color32;

pub const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const BG: Color32 = hex(0x000000);
pub const BG1: Color32 = hex(0x0F0F0F);
pub const BG2: Color32 = hex(0x1E1E1E);
pub const BTN: Color32 = hex(0x282828);
pub const SCROLL: Color32 = hex(0x323232);
pub const OFF: Color32 = hex(0x646464);
pub const DIM: Color32 = hex(0x969696);
pub const DIM2: Color32 = hex(0xAAAAAA);
pub const TEXT: Color32 = hex(0xF0F0F0);
pub const WHITE: Color32 = hex(0xFFFFFF);
pub const BLUE: Color32 = hex(0x1473EB);
pub const BEAT_BLUE: Color32 = hex(0x007DE1);
pub const SELECT: Color32 = hex(0x294C6A);
/// Selected row in rekordbox's browser (tree and track list), sampled from the references.
pub const ROW_SEL: Color32 = hex(0x294C6A);
/// Lit source tile / "Tree View" button in the browser.
pub const TILE_ON: Color32 = hex(0x1473EB);
/// rekordbox's tree glyph grey (skins: rgb 160).
pub const TREE_ICON: Color32 = hex(0xA0A0A0);
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
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG2;
    v.window_fill = BG2;
    v.extreme_bg_color = BG;
    v.faint_bg_color = BG1;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = BLUE;
    v.selection.stroke = egui::Stroke::new(1.0, WHITE);
    v.widgets.noninteractive.bg_fill = BG2;
    v.widgets.inactive.bg_fill = BTN;
    v.widgets.inactive.weak_bg_fill = BTN;
    v.widgets.hovered.bg_fill = hex(0x3A3A3A);
    v.widgets.hovered.weak_bg_fill = hex(0x3A3A3A);
    v.widgets.active.bg_fill = BLUE;
    v.widgets.active.weak_bg_fill = BLUE;
    v.window_stroke = egui::Stroke::new(1.0, hex(0x3A3A3A));
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.scroll.bar_width = 8.0;
    ctx.set_style(style);
}
