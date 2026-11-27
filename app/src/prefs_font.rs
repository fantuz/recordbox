//! The face and text metrics of rekordbox's Preferences window, and the few pieces of its skin
//! that are read from an installed rekordbox at run time.
//!
//! The face: measured on the 7.2.19 captures (`rekordbox-reference/preferences`), the window's
//! text is Arial, regular and bold. Rendered at the size JUCE gives it, a card heading
//! ("Memory Cue Call Lock", "Deck track information display") correlates 0.97 with Arial Bold
//! and 0.53 with Inter Bold, and line widths agree with Arial within 1 px in 900 where Inter
//! is 3 % too narrow. The binary names only "Arial" besides the system faces, which agrees.
//!
//! The size: JUCE's `Font(h)` is `h` points from ascender to descender, not `h` points per em.
//! Arial's ascender and descender are 1854 and 434 of 2048 units, so its em size is
//! `h * 2048 / 2288`. rekordbox's sizes in the code are 20 (card headings, bold), 18 (sub
//! headings, bold), 16 (labels, dropdown values), 14 (slider end labels) and 0.85 x the row
//! height for the captions of switches, radio buttons and check boxes (16.15 in a 19 pt row).
//!
//! Arial is read from the system's font folder, where every macOS has it; nothing is bundled.
//! Without it the bundled Inter stands in, at the size that gives the same height.

use eframe::egui;
use egui::{pos2, Color32, FontId, Rect};
use std::sync::atomic::{AtomicBool, Ordering};

static ARIAL: AtomicBool = AtomicBool::new(false);

/// Registers the families "pref" and "pref-bold" (called once from `theme::apply`).
pub fn install(fonts: &mut egui::FontDefinitions) {
    let mut ok = true;
    for (fam, face, path, fallback) in [
        ("pref", "arial", "/System/Library/Fonts/Supplemental/Arial.ttf", "inter"),
        ("pref-bold", "arial-bold", "/System/Library/Fonts/Supplemental/Arial Bold.ttf", "inter-bold"),
    ] {
        let mut list = vec![];
        if fonts.font_data.contains_key(face) {
            list.push(face.to_owned());
        } else if let Ok(b) = std::fs::read(path) {
            fonts.font_data.insert(face.to_owned(), std::sync::Arc::new(egui::FontData::from_owned(b)));
            list.push(face.to_owned());
        } else {
            ok = false;
        }
        list.extend(fonts.families.get(&egui::FontFamily::Name(fallback.into())).cloned().unwrap_or_default());
        list.extend(fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default());
        fonts.families.insert(egui::FontFamily::Name(fam.into()), list);
    }
    ARIAL.store(ok, Ordering::Relaxed);
}

/// Em size per point of JUCE font height.
fn em_per_h() -> f32 {
    if ARIAL.load(Ordering::Relaxed) { 2048.0 / 2288.0 } else { 2048.0 / 2480.0 }
}

/// The regular face at JUCE font height `h`.
pub fn pref(h: f32) -> FontId {
    FontId::new(h * em_per_h(), egui::FontFamily::Name("pref".into()))
}

/// The bold face at JUCE font height `h`.
pub fn pref_bold(h: f32) -> FontId {
    FontId::new(h * em_per_h(), egui::FontFamily::Name("pref-bold".into()))
}

/// Where JUCE puts text in its box (`juce::Justification`: 9, 33, 36, 34, 10).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Just {
    TopLeft,
    Left,
    Centre,
    Right,
    TopRight,
}

/// Text laid out at JUCE height `h`; lines are `h` apart, and the top of the galley is the
/// font's ascender line, as in a JUCE text box.
pub fn galley(ui: &egui::Ui, s: &str, h: f32, bold: bool, color: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    let mut f = egui::TextFormat::simple(if bold { pref_bold(h) } else { pref(h) }, color);
    f.line_height = Some(h);
    // egui draws a font at a whole number of pixels from ascender to descender (16.15 pt on
    // a Retina display becomes 32 px instead of 32.3), which would make a line about 1 %
    // short. The difference is put back between the letters (Arial's letters average 0.52 em).
    let px = h * ui.ctx().pixels_per_point();
    if px >= 1.0 {
        f.extra_letter_spacing = (px / px.round() - 1.0) * 0.52 * h * em_per_h();
    }
    job.append(s, 0.0, f);
    job.wrap.max_width = f32::INFINITY;
    ui.painter().layout_job(job)
}

/// What `juce::Font(h).getStringWidth(s)` gives.
pub fn width(ui: &egui::Ui, s: &str, h: f32, bold: bool) -> f32 {
    galley(ui, s, h, bold, Color32::WHITE).size().x
}

/// Height of a galley made by `galley` (its rows are `h` apart, the last one included).
pub fn height(g: &egui::Galley, h: f32) -> f32 {
    g.rows.len().max(1) as f32 * h
}

/// Draws text as `juce::Graphics::drawText(s, r, just)` does at font height `h`; returns the
/// rectangle the text takes.
pub fn draw(ui: &egui::Ui, r: Rect, s: &str, h: f32, bold: bool, color: Color32, just: Just) -> Rect {
    let g = galley(ui, s, h, bold, color);
    let (w, th) = (g.size().x, height(&g, h));
    let x = match just {
        Just::TopLeft | Just::Left => r.min.x,
        Just::Centre => r.center().x - w / 2.0,
        Just::Right | Just::TopRight => r.max.x - w,
    };
    let y = match just {
        Just::TopLeft | Just::TopRight => r.min.y,
        _ => r.center().y - th / 2.0,
    };
    // (Bold text lands half a point above rekordbox's with egui's pixel rounding of the
    // baseline: measured on every heading of the captures.)
    let y = if bold { y + 0.5 } else { y };
    ui.painter().galley(pos2(x, y), g, color);
    Rect::from_min_size(pos2(x, y), egui::vec2(w, th))
}

/// A `juce::Label` at `r`: the text sits inside its 1 pt (top, bottom) and 5 pt (left, right)
/// border.
pub fn label(ui: &egui::Ui, r: Rect, s: &str, h: f32, bold: bool, color: Color32, just: Just) -> Rect {
    draw(ui, Rect::from_min_max(pos2(r.min.x + 5.0, r.min.y + 1.0), pos2(r.max.x - 5.0, r.max.y - 1.0)), s, h, bold, color, just)
}

// ------------------------------------------------------------------ skin files

/// rekordbox's skin folder, when rekordbox is installed. Its artwork is only ever read from
/// there at run time; none of it is part of recordbox.
pub fn skin_dir() -> Option<std::path::PathBuf> {
    static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::path::PathBuf::from("/Applications/rekordbox 7/rekordbox.app/Contents/Resources/skins");
        d.is_dir().then_some(d)
    })
    .clone()
}

type TexSlot = Option<(egui::TextureHandle, egui::Vec2)>;

/// A skin image as a texture, with its size in points: an SVG drawn at three pixels per point
/// (sharp on any display), a PNG as it is (one pixel per point, as JUCE draws it). `None`
/// when rekordbox is not installed or the file cannot be read; the caller then draws its own
/// stand-in.
pub fn skin(ctx: &egui::Context, name: &str) -> TexSlot {
    let id = egui::Id::new(("prefs_skin", name));
    if let Some(t) = ctx.data(|d| d.get_temp::<TexSlot>(id)) {
        return t;
    }
    let slot = load_skin(ctx, name);
    ctx.data_mut(|d| d.insert_temp(id, slot.clone()));
    slot
}

fn load_skin(ctx: &egui::Context, name: &str) -> TexSlot {
    let bytes = std::fs::read(skin_dir()?.join(name)).ok()?;
    if name.ends_with(".svg") {
        let text = String::from_utf8_lossy(&bytes);
        let num = |key: &str| -> Option<f32> {
            let i = text.find(key)? + key.len();
            text[i..].split('"').next()?.trim_end_matches("px").parse().ok()
        };
        let (w, h) = (num("width=\"")?, num("height=\"")?);
        if !(1.0..=512.0).contains(&w) || !(1.0..=512.0).contains(&h) {
            return None;
        }
        let img = egui_extras::image::load_svg_bytes_with_size(&bytes, Some(egui::SizeHint::Size((w * 3.0) as u32, (h * 3.0) as u32))).ok()?;
        Some((ctx.load_texture(format!("skin:{name}"), img, egui::TextureOptions::LINEAR), egui::vec2(w, h)))
    } else {
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        if w == 0 || h == 0 || w > 2048 || h > 2048 {
            return None;
        }
        let ci = egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw());
        Some((ctx.load_texture(format!("skin:{name}"), ci, egui::TextureOptions::LINEAR), egui::vec2(w as f32, h as f32)))
    }
}

/// Paints a skin image into `r`.
pub fn paint_skin(p: &egui::Painter, tex: &egui::TextureHandle, r: Rect, tint: Color32) {
    p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn juce_heights_become_em_sizes() {
        // Without the font loaded the stand-in factor applies; with Arial, 20 -> 17.9 pt.
        ARIAL.store(true, Ordering::Relaxed);
        assert!((pref_bold(20.0).size - 17.902).abs() < 0.01);
        assert!((pref(0.85 * 19.0).size - 14.456).abs() < 0.01);
        assert!((pref(14.0).size - 12.531).abs() < 0.01);
    }
}
