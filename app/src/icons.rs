//! Simple vector icons drawn with the painter, in the line style of the original's skin
//! (1.2–1.6 pt strokes, rounded). Each draws inside `r` with colour `c`.

use eframe::egui;
use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

fn s(c: Color32, w: f32) -> Stroke {
    Stroke::new(w, c)
}

fn poly(p: &Painter, pts: &[Pos2], c: Color32, w: f32) {
    p.add(Shape::line(pts.to_vec(), s(c, w)));
}

pub fn chevron_left(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let h = r.height().min(r.width()) * 0.32;
    poly(p, &[m + vec2(h * 0.5, -h), m + vec2(-h * 0.5, 0.0), m + vec2(h * 0.5, h)], c, 1.0);
}
pub fn chevron_right(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let h = r.height().min(r.width()) * 0.32;
    poly(p, &[m + vec2(-h * 0.5, -h), m + vec2(h * 0.5, 0.0), m + vec2(-h * 0.5, h)], c, 1.0);
}
pub fn chevron_down(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let h = r.height().min(r.width()) * 0.3;
    poly(p, &[m + vec2(-h, -h * 0.45), m + vec2(0.0, h * 0.45), m + vec2(h, -h * 0.45)], c, 1.0);
}

/// "↤": arrow pointing left with a stop bar at its tail.
pub fn jump_front(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let w = r.width() * 0.26;
    p.line_segment([m + vec2(-w, 0.0), m + vec2(w, 0.0)], s(c, 1.4));
    poly(p, &[m + vec2(-w + 4.0, -3.5), m + vec2(-w, 0.0), m + vec2(-w + 4.0, 3.5)], c, 1.4);
    p.line_segment([m + vec2(w, -4.0), m + vec2(w, 4.0)], s(c, 1.4));
}
/// "↦": arrow pointing right with a stop bar at its tail.
pub fn jump_rear(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let w = r.width() * 0.26;
    p.line_segment([m + vec2(-w, 0.0), m + vec2(w, 0.0)], s(c, 1.4));
    poly(p, &[m + vec2(w - 4.0, -3.5), m + vec2(w, 0.0), m + vec2(w - 4.0, 3.5)], c, 1.4);
    p.line_segment([m + vec2(-w, -4.0), m + vec2(-w, 4.0)], s(c, 1.4));
}

pub fn magnifier(p: &Painter, r: Rect, c: Color32, sign: i32) {
    let rad = r.height().min(r.width()) * 0.3;
    let m = r.center() - vec2(1.5, 1.5);
    p.circle_stroke(m, rad, s(c, 1.4));
    let d = rad * 0.707;
    p.line_segment([m + vec2(d, d), m + vec2(d + 4.0, d + 4.0)], s(c, 1.6));
    if sign != 0 {
        p.line_segment([m - vec2(rad * 0.5, 0.0), m + vec2(rad * 0.5, 0.0)], s(c, 1.3));
        if sign > 0 {
            p.line_segment([m - vec2(0.0, rad * 0.5), m + vec2(0.0, rad * 0.5)], s(c, 1.3));
        }
    }
}

pub fn tri_left(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let h = r.height().min(r.width()) * 0.24;
    p.add(Shape::convex_polygon(vec![m + vec2(-h, 0.0), m + vec2(h * 0.8, -h), m + vec2(h * 0.8, h)], c, Stroke::NONE));
}
pub fn tri_right(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let h = r.height().min(r.width()) * 0.24;
    p.add(Shape::convex_polygon(vec![m + vec2(h, 0.0), m + vec2(-h * 0.8, -h), m + vec2(-h * 0.8, h)], c, Stroke::NONE));
}
pub fn cross(p: &Painter, r: Rect, c: Color32, w: f32) {
    let m = r.center();
    let h = r.height().min(r.width()) * 0.26;
    p.line_segment([m + vec2(-h, -h), m + vec2(h, h)], s(c, w));
    p.line_segment([m + vec2(-h, h), m + vec2(h, -h)], s(c, w));
}
pub fn hamburger(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let w = r.width().min(r.height() * 1.4) * 0.34;
    for dy in [-5.0, 0.0, 5.0] {
        p.line_segment([m + vec2(-w, dy), m + vec2(w, dy)], s(c, 2.0));
    }
}
pub fn gear(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let rad = r.height().min(r.width()) * 0.28;
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::PI / 4.0;
        let d = vec2(a.cos(), a.sin());
        p.line_segment([m + d * rad, m + d * (rad + 3.2)], s(c, 3.0));
    }
    p.circle_stroke(m, rad, s(c, 2.4));
    p.circle_filled(m, rad * 0.35, c);
}
pub fn layout_icon(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center(), vec2(15.0, 9.0));
    p.rect_stroke(b, 1.0, s(c, 1.3), egui::StrokeKind::Inside);
    p.rect_filled(Rect::from_min_size(b.min + vec2(2.5, 2.5), vec2(5.0, 4.0)), 0.0, c);
}
pub fn record(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let rad = r.height().min(r.width()) * 0.36;
    p.circle_stroke(m, rad, s(c, 1.6));
    p.circle_filled(m, rad * 0.55, c);
}
pub fn bookmark(p: &Painter, r: Rect, c: Color32, filled: bool) {
    let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.48, r.height() * 0.62));
    let pts = vec![b.left_top(), b.right_top(), b.right_bottom(), pos2(b.center().x, b.bottom() - b.height() * 0.28), b.left_bottom()];
    if filled {
        p.add(Shape::convex_polygon(vec![b.left_top(), b.right_top(), b.right_bottom(), pos2(b.center().x, b.bottom() - b.height() * 0.28)], c, Stroke::NONE));
        p.add(Shape::convex_polygon(vec![b.left_top(), pos2(b.center().x, b.bottom() - b.height() * 0.28), b.left_bottom()], c, Stroke::NONE));
    } else {
        let mut q = pts.clone();
        q.push(pts[0]);
        poly(p, &q, c, 1.3);
    }
}
pub fn list(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.62, r.height() * 0.62));
    p.rect_stroke(b, 1.0, s(c, 1.3), egui::StrokeKind::Inside);
    for k in 1..4 {
        let y = b.top() + b.height() * k as f32 / 4.0;
        p.line_segment([pos2(b.left() + 3.0, y), pos2(b.right() - 3.0, y)], s(c, 1.2));
    }
}
pub fn link(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let d = r.width() * 0.2;
    p.line_segment([m + vec2(-d, d), m + vec2(d, -d)], s(c, 2.2));
    p.circle_stroke(m + vec2(-d * 1.2, d * 1.2), 2.4, s(c, 1.6));
    p.circle_stroke(m + vec2(d * 1.2, -d * 1.2), 2.4, s(c, 1.6));
}
pub fn monitor(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center() - vec2(0.0, 1.5), vec2(r.width() * 0.62, r.height() * 0.44));
    p.rect_stroke(b, 1.0, s(c, 1.3), egui::StrokeKind::Inside);
    p.line_segment([pos2(b.left() - 1.5, b.bottom() + 3.0), pos2(b.right() + 1.5, b.bottom() + 3.0)], s(c, 1.4));
}
pub fn usb(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center() + vec2(0.0, 1.5), vec2(r.width() * 0.46, r.height() * 0.5));
    p.rect_stroke(b, 1.0, s(c, 1.3), egui::StrokeKind::Inside);
    let t = Rect::from_min_max(pos2(b.left() + 2.0, b.top() - 4.0), pos2(b.right() - 2.0, b.top()));
    p.rect_stroke(t, 0.0, s(c, 1.2), egui::StrokeKind::Inside);
    p.line_segment([pos2(b.center().x - 2.0, b.top() + 3.0), pos2(b.center().x - 2.0, b.top() + 6.0)], s(c, 1.2));
    p.line_segment([pos2(b.center().x + 2.0, b.top() + 3.0), pos2(b.center().x + 2.0, b.top() + 6.0)], s(c, 1.2));
}
pub fn history(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let rad = r.height().min(r.width()) * 0.3;
    let mut pts = vec![];
    for i in 0..=20 {
        let a = -2.6 + i as f32 * 0.25;
        pts.push(m + vec2(a.cos(), a.sin()) * rad);
    }
    poly(p, &pts, c, 1.4);
    poly(p, &[m + vec2(0.0, -rad * 0.55), m, m + vec2(rad * 0.45, rad * 0.3)], c, 1.3);
    let tip = pts[0];
    poly(p, &[tip + vec2(-3.0, -1.0), tip, tip + vec2(1.0, -3.2)], c, 1.3);
}
pub fn tag(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let w = r.width() * 0.3;
    let pts = vec![m + vec2(-w, -w * 0.2), m + vec2(-w * 0.2, -w), m + vec2(w, -w), m + vec2(w, w * 0.2), m + vec2(w * 0.2, w)];
    p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
    p.circle_filled(m + vec2(w * 0.45, -w * 0.45), 1.6, crate::theme::gy(0x1E1E1E));
}
pub fn info(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let rad = r.height().min(r.width()) * 0.34;
    p.circle_stroke(m, rad, s(c, 1.4));
    p.line_segment([m + vec2(0.0, -rad * 0.05), m + vec2(0.0, rad * 0.55)], s(c, 1.8));
    p.circle_filled(m + vec2(0.0, -rad * 0.45), 1.3, c);
}
pub fn split_panes(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.56, r.height() * 0.56));
    p.rect_stroke(b, 1.0, s(c, 1.3), egui::StrokeKind::Inside);
    p.rect_filled(Rect::from_min_max(b.min, pos2(b.center().x - 1.0, b.max.y)), 0.0, c);
}
pub fn window_icon(p: &Painter, r: Rect, c: Color32) {
    // Reference 04: a 12 × 10 pt window with a 4 pt title bar and a 1.5 pt frame (in an 18 × 16 rect).
    let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.667, r.height() * 0.625));
    p.rect_stroke(b, 1.0, s(c, 1.5), egui::StrokeKind::Inside);
    p.rect_filled(Rect::from_min_max(b.min, pos2(b.max.x, b.min.y + 4.0)), 0.0, c);
}
pub fn view_art_list(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.6, r.height() * 0.6));
    for k in 0..3 {
        let y = b.top() + 2.0 + k as f32 * (b.height() - 4.0) / 2.0;
        p.rect_filled(Rect::from_center_size(pos2(b.left() + 1.5, y), vec2(3.0, 3.0)), 0.0, c);
        p.line_segment([pos2(b.left() + 5.0, y), pos2(b.right(), y)], s(c, 1.4));
    }
}
pub fn view_list(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.6, r.height() * 0.6));
    for k in 0..4 {
        let y = b.top() + 1.0 + k as f32 * (b.height() - 2.0) / 3.0;
        p.line_segment([pos2(b.left(), y), pos2(b.right(), y)], s(c, 1.3));
    }
}
pub fn view_columns(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.56, r.height() * 0.56));
    p.rect_stroke(b, 1.0, s(c, 1.3), egui::StrokeKind::Inside);
    for k in 1..3 {
        let x = b.left() + b.width() * k as f32 / 3.0;
        p.line_segment([pos2(x, b.top()), pos2(x, b.bottom())], s(c, 2.0));
    }
}
pub fn funnel(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let w = r.width().min(r.height()) * 0.32;
    p.add(Shape::convex_polygon(vec![m + vec2(-w, -w), m + vec2(w, -w), m + vec2(w * 0.2, 0.0), m + vec2(-w * 0.2, 0.0)], c, Stroke::NONE));
    p.rect_filled(Rect::from_min_max(m + vec2(-w * 0.2, 0.0), m + vec2(w * 0.2, w)), 0.0, c);
}
pub fn vinyl(p: &Painter, r: Rect) {
    let m = r.center();
    let rad = r.height().min(r.width()) * 0.42;
    p.circle_filled(m, rad, crate::theme::gy(0x1C1C1C));
    for k in [0.85, 0.7, 0.55] {
        p.circle_stroke(m, rad * k, Stroke::new(1.0, crate::theme::gy(0x282828)));
    }
    p.circle_filled(m, rad * 0.18, crate::theme::gy(0x0C0C0C));
}
pub fn lock(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let b = Rect::from_center_size(m + vec2(0.0, 2.0), vec2(9.0, 7.0));
    p.rect_filled(b, 1.0, c);
    let mut pts = vec![];
    for i in 0..=10 {
        let a = std::f32::consts::PI + i as f32 * std::f32::consts::PI / 10.0;
        pts.push(pos2(m.x + a.cos() * 3.0, b.top() - 0.5 + a.sin() * 3.6));
    }
    poly(p, &pts, c, 1.4);
}
pub fn sync(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let rad = r.height().min(r.width()) * 0.3;
    for (start, flip) in [(0.3f32, 1.0f32), (3.44, -1.0)] {
        let mut pts = vec![];
        for i in 0..=10 {
            let a = start + i as f32 * 0.26;
            pts.push(m + vec2(a.cos(), a.sin()) * rad);
        }
        poly(p, &pts, c, 1.4);
        let t = *pts.last().unwrap();
        poly(p, &[t + vec2(-3.0 * flip, 0.0), t, t + vec2(0.0, -3.0 * flip)], c, 1.3);
    }
}
pub fn speech(p: &Painter, r: Rect, c: Color32) {
    let b = Rect::from_center_size(r.center() - vec2(0.0, 1.0), vec2(r.width() * 0.6, r.height() * 0.42));
    p.rect_filled(b, 2.0, c);
    p.add(Shape::convex_polygon(vec![pos2(b.left() + 2.0, b.bottom()), pos2(b.left() + 6.0, b.bottom()), pos2(b.left() + 2.0, b.bottom() + 3.5)], c, Stroke::NONE));
}
