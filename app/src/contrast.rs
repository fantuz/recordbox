//! Legibility check for the screenshots (`PREPDECK_CONTRAST=<file.tsv>` with `--screenshot`):
//! every text run of the frame that is saved, with its colour and its rectangle in points, so
//! that it can be compared against the background pixels in the saved picture.

use eframe::egui;

/// Writes one line per text shape: layer, x0, y0, x1, y1 (points, clipped), colour #RRGGBBAA,
/// text (tabs and line breaks replaced by spaces).
pub fn dump(ctx: &egui::Context, path: &str) {
    let mut order: Vec<egui::LayerId> = vec![egui::LayerId::background()];
    order.extend(ctx.memory(|m| m.layer_ids().collect::<Vec<_>>()).into_iter().filter(|l| *l != egui::LayerId::background()));
    let mut out = String::from("layer\tx0\ty0\tx1\ty1\tcolour\ttext\n");
    // (The areas, back to front, so that text under a window or pop-up can be left out.)
    for id in &order {
        if let Some(r) = ctx.memory(|m| m.area_rect(id.id)) {
            out.push_str(&format!("#area {}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t\t\n", id.short_debug_format(), r.min.x, r.min.y, r.max.x, r.max.y));
        }
    }
    ctx.graphics(|g| {
        for id in &order {
            let Some(list) = g.get(*id) else { continue };
            for cs in list.all_entries() {
                collect(&cs.shape, cs.clip_rect, id, &mut out);
            }
        }
    });
    let _ = std::fs::write(path, out);
}

fn collect(shape: &egui::Shape, clip: egui::Rect, id: &egui::LayerId, out: &mut String) {
    match shape {
        egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, clip, id, out)),
        egui::Shape::Text(t) => {
            let r = t.galley.rect.translate(t.pos.to_vec2()).intersect(clip);
            if r.width() < 1.0 || r.height() < 1.0 || t.galley.text().trim().is_empty() {
                return;
            }
            let sec = t.galley.job.sections.first().map(|s| s.format.color).unwrap_or(t.fallback_color);
            let c = t.override_text_color.unwrap_or(if sec == egui::Color32::PLACEHOLDER { t.fallback_color } else { sec });
            let c = c.gamma_multiply(t.opacity_factor);
            let [r8, g8, b8, a8] = c.to_srgba_unmultiplied();
            let text: String = t.galley.text().chars().map(|ch| if ch == '\t' || ch == '\n' { ' ' } else { ch }).take(60).collect();
            out.push_str(&format!("{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t#{r8:02X}{g8:02X}{b8:02X}{a8:02X}\t{text}\n", id.short_debug_format(), r.min.x, r.min.y, r.max.x, r.max.y));
        }
        _ => {}
    }
}
