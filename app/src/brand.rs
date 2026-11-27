//! The recordbox logo: one SVG asset (mark + wordmark in a single colour, `assets/brand/`),
//! drawn wherever the app shows its name as a logo, and the mark as the app icon.

use eframe::egui;
use egui::{Color32, Rect, Ui};

/// Mark and wordmark, `fill="currentColor"`: the colour is set when drawing.
pub const LOGO_SVG: &str = include_str!("../assets/brand/recordbox-logo.svg");
/// The app icon (a record split along its middle on a rounded tile), small-size drawing:
/// the mark of the logo, the manual's page icon.
pub const MARK_SVG: &str = include_str!("../assets/brand/recordbox-icon.svg");
/// The app icon as pictures, for the window and the Dock: the regular one and the one for
/// the system's dark appearance.
const ICON_PNG: &[u8] = include_bytes!("../assets/brand/recordbox-icon-512.png");
const ICON_DARK_PNG: &[u8] = include_bytes!("../assets/brand/recordbox-icon-dark-512.png");
/// Width / height of the logo's view box (616 x 83 units).
pub const LOGO_ASPECT: f32 = 616.0 / 83.0;

/// The logo as one SVG: the wordmark in `col`, and the app icon where the mark stands (a
/// 75-unit square at 21, 283.8 of the logo's view box).
fn logo_svg(col: &str) -> String {
    let word: String = LOGO_SVG.lines().filter(|l| !l.contains("mask=\"url(#mark-cut)\"")).collect::<Vec<_>>().join("\n").replace("currentColor", col);
    let icon = MARK_SVG.replacen("<svg ", "<svg x=\"21\" y=\"283.8\" ", 1).replacen("width=\"1024\" height=\"1024\"", "width=\"75\" height=\"75\"", 1);
    word.replacen("</svg>", &format!("{icon}\n</svg>"), 1)
}

fn hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// The logo in colour `c` as an image of the given height (pt), its width following.
pub fn logo_image(c: Color32, height: f32) -> egui::Image<'static> {
    let col = hex(c);
    let svg = logo_svg(&col);
    egui::Image::from_bytes(format!("bytes://recordbox-logo2-{col}.svg"), svg.into_bytes()).fit_to_exact_size(egui::vec2(height * LOGO_ASPECT, height))
}

/// Draws the logo with its view box's top-left corner at `rect.min`, `rect.height()` tall.
pub fn paint_logo(ui: &Ui, rect: Rect, c: Color32) {
    let r = Rect::from_min_size(rect.min, egui::vec2(rect.height() * LOGO_ASPECT, rect.height()));
    logo_image(c, rect.height()).paint_at(ui, r);
}

/// The app icon as RGBA pixels, for the window and Dock icon. `dark`: the variant for the
/// system's dark appearance.
pub fn app_icon(dark: bool) -> Option<egui::IconData> {
    let img = image::load_from_memory(if dark { ICON_DARK_PNG } else { ICON_PNG }).ok()?.to_rgba8();
    let (width, height) = (img.width(), img.height());
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

/// The system is in its dark appearance (macOS: the global AppleInterfaceStyle setting).
pub fn system_dark() -> bool {
    std::process::Command::new("/usr/bin/defaults").args(["read", "-g", "AppleInterfaceStyle"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "Dark").unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    #[test]
    fn logo_and_icon_render() {
        let svg = super::logo_svg("#FFFFFF");
        let img = egui_extras::image::load_svg_bytes_with_size(svg.as_bytes(), Some(egui::SizeHint::Size(616, 83))).unwrap();
        assert_eq!(img.size, [616, 83]);
        let at = |x: usize, y: usize| img.pixels[y * 616 + x];
        // The icon's tile is drawn where the mark stands, in its own colour (not the wordmark's).
        let tile = at(30 - 19, 300 - 278);
        assert!(tile.a() > 200 && !(tile.r() > 250 && tile.g() > 250 && tile.b() > 250), "tile {tile:?}");
        // Letters drawn in the asked colour ("d" stem near unit 434).
        assert!((0..83).any(|y| at(434 - 19, y).a() > 200 && at(434 - 19, y).r() > 200));
        for dark in [false, true] {
            let m = super::app_icon(dark).unwrap();
            assert_eq!((m.width, m.height), (512, 512));
        }
    }
}
