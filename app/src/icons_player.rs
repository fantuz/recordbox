//! Glyphs of the header bar and the player, drawn with the painter on the same coordinate
//! grids as rekordbox's own artwork (the grid of each glyph is given in its comment). Every
//! function takes the glyph's centre `c` and a scale `k` (points per grid unit), so a glyph
//! keeps its proportions wherever it is used. Sizes on screen were measured on rekordbox 7.2.19.

use eframe::egui;
use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

/// A ring whose stroke of width `w` is centred on radius `r` (the painter's own circle
/// stroke lies outside its radius).
pub fn ring(p: &Painter, c: Pos2, r: f32, w: f32, col: Color32) {
    p.circle_stroke(c, r - w / 2.0, Stroke::new(w, col));
}

fn fill(p: &Painter, pts: Vec<Pos2>, c: Color32) {
    p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
}

/// Pull-down chevron: 8 × 4 units, 1 unit stroke.
pub fn chevron_down(p: &Painter, c: Pos2, k: f32, col: Color32) {
    p.add(Shape::line(vec![c + vec2(-4.0, -2.0) * k, c + vec2(0.0, 2.0) * k, c + vec2(4.0, -2.0) * k], Stroke::new(k * 0.9, col)));
}
/// Left chevron: 4 × 8 units, 1 unit stroke.
pub fn chevron_left(p: &Painter, c: Pos2, k: f32, col: Color32) {
    p.add(Shape::line(vec![c + vec2(2.0, -4.0) * k, c + vec2(-2.0, 0.0) * k, c + vec2(2.0, 4.0) * k], Stroke::new(k * 0.9, col)));
}
/// Right chevron: 4 × 8 units, 1 unit stroke.
pub fn chevron_right(p: &Painter, c: Pos2, k: f32, col: Color32) {
    p.add(Shape::line(vec![c + vec2(-2.0, -4.0) * k, c + vec2(2.0, 0.0) * k, c + vec2(-2.0, 4.0) * k], Stroke::new(k * 0.9, col)));
}

/// Previous Track: arrow head, shaft and stop bar on a 10 × 11 grid (`dir` = -1), or its
/// mirror image, Next Track (`dir` = 1).
pub fn track_jump(p: &Painter, c: Pos2, k: f32, col: Color32, dir: f32) {
    // Grid coordinates of the "previous" glyph, relative to the grid centre (5, 5.5).
    let g = |x: f32, y: f32| c + vec2(dir * (5.0 - x), y - 5.5) * k;
    fill(p, vec![g(3.5, 3.5), g(1.0, 5.5), g(3.5, 7.5)], col);
    p.rect_filled(Rect::from_two_pos(g(2.5, 5.0), g(7.0, 6.0)), 0.0, col);
    p.rect_filled(Rect::from_two_pos(g(8.0, 3.5), g(9.0, 7.5)), 0.0, col);
}

/// Zoom In / Zoom Out: lens, handle and sign on a 12 × 12 grid, hairline strokes.
pub fn zoom(p: &Painter, c: Pos2, k: f32, col: Color32, plus: bool) {
    let g = |x: f32, y: f32| c + vec2(x - 6.0, y - 6.0) * k;
    // The painter feathers strokes by half a pixel a side, so they are set a little under 1 unit.
    let s = Stroke::new(k * 0.75, col);
    ring(p, g(5.5, 5.0), 4.5 * k, k * 0.85, col);
    p.line_segment([g(9.0, 8.5), g(11.5, 11.0)], Stroke::new(k * 0.9, col));
    p.line_segment([g(3.0, 5.0), g(8.0, 5.0)], s);
    if plus {
        p.line_segment([g(5.5, 2.5), g(5.5, 7.5)], s);
    }
}

/// Menu: three 8 × 1.5 bars on a 10 × 10 grid.
pub fn menu(p: &Painter, c: Pos2, k: f32, col: Color32) {
    let g = |x: f32, y: f32| c + vec2(x - 5.0, y - 5.0) * k;
    for y in [1.0, 4.25, 7.5] {
        p.rect_filled(Rect::from_min_max(g(1.0, y), g(9.0, y + 1.5)), 0.0, col);
    }
}

/// Settings gear on a 14 × 14 grid: eight square teeth around a ring; `bg` shows through the hole.
pub fn gear(p: &Painter, c: Pos2, k: f32, col: Color32, bg: Color32) {
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (out, side) = (vec2(a.sin(), -a.cos()), vec2(a.cos(), a.sin()));
        let (r0, r1, w) = (4.6 * k, 7.0 * k, 1.615 * k);
        fill(p, vec![c + out * r0 - side * w, c + out * r1 - side * w, c + out * r1 + side * w, c + out * r0 + side * w], col);
    }
    p.circle_filled(c, 5.75 * k, col);
    p.circle_filled(c, 3.95 * k, bg);
}

/// The header's round record-style mark: a 13 pt ring with a 7 pt dot.
pub fn record(p: &Painter, c: Pos2, col: Color32) {
    ring(p, c, 6.5, 1.0, col);
    p.circle_filled(c, 3.5, col);
}

/// Layout selector mark (1 Player): an 18 × 9 frame, 1.5 pt thick, with a 3 pt block at its left.
pub fn layout(p: &Painter, c: Pos2, col: Color32) {
    let r = Rect::from_center_size(c, vec2(18.0, 9.0));
    p.rect_stroke(r, 0.0, Stroke::new(1.5, col), egui::StrokeKind::Inside);
    p.rect_filled(Rect::from_min_size(r.min + vec2(3.0, 3.0), vec2(3.0, 3.0)), 0.0, col);
}

/// The blue arc and pointer show the master volume, from bottom left to bottom right.
pub fn master_volume(p: &Painter, c: Pos2, volume: f32) {
    let start = 135f32.to_radians();
    let sweep = 270f32.to_radians();
    let at = |angle: f32| vec2(angle.cos(), angle.sin());
    let arc = |v: f32| (0..=28).map(|i| c + at(start + sweep * v * i as f32 / 28.0) * 8.2).collect();
    p.add(Shape::line(arc(1.0), Stroke::new(2.7, crate::theme::gy(0x323232))));
    if volume > 0.0 { p.add(Shape::line(arc(volume), Stroke::new(2.7, crate::theme::BLUE))); }
    let pointer = at(start + sweep * volume);
    p.line_segment([c + pointer * 4.4, c + pointer * 9.4], Stroke::new(2.1, crate::theme::WHITE));
}

/// Call Previous / Next Memory Cue: an 8 × 8 triangle whose tip is at `tip`.
pub fn mem_tri(p: &Painter, tip: Pos2, dir: f32, col: Color32) {
    fill(p, vec![tip, tip + vec2(-dir * 8.0, -4.0), tip + vec2(-dir * 8.0, 4.0)], col);
}

/// A diagonal cross spanning `size` pt, with stroke `w`.
pub fn cross(p: &Painter, c: Pos2, size: f32, w: f32, col: Color32) {
    let h = size / 2.0 - w * 0.354;
    p.line_segment([c + vec2(-h, -h), c + vec2(h, h)], Stroke::new(w, col));
    p.line_segment([c + vec2(-h, h), c + vec2(h, -h)], Stroke::new(w, col));
}

/// The current-cue marker: an upward triangle `h` tall with its apex at `apex` (half-width
/// 0.63 h, the proportions of rekordbox's marker).
pub fn cue_tri(p: &Painter, apex: Pos2, h: f32, col: Color32) {
    fill(p, vec![apex, apex + vec2(0.63 * h, h), apex + vec2(-0.63 * h, h)], col);
}

/// A beat's end cap on the enlarged waveform: a 3 × 3 triangle pointing into the waveform
/// with a 1 pt stem (`dir` = 1 at the top edge `y`, -1 at the bottom edge).
pub fn beat_cap(p: &Painter, x: f32, y: f32, dir: f32, col: Color32) {
    fill(p, vec![pos2(x - 1.5, y), pos2(x + 1.5, y), pos2(x, y + dir * 3.0)], col);
    p.rect_filled(Rect::from_two_pos(pos2(x - 0.5, y), pos2(x + 0.5, y + dir * 4.0)), 0.0, col);
}

/// A bar's end cap: a 5 × 4 triangle.
pub fn bar_cap(p: &Painter, x: f32, y: f32, dir: f32, col: Color32) {
    fill(p, vec![pos2(x - 2.5, y), pos2(x + 2.5, y), pos2(x, y + dir * 4.0)], col);
}

/// Play triangle of the round PLAY button (36-unit button grid, centre 18,18).
pub fn play_tri(p: &Painter, c: Pos2, k: f32, col: Color32) {
    fill(p, vec![c + vec2(-3.0, -3.55) * k, c + vec2(4.35, 0.0) * k, c + vec2(-3.0, 3.55) * k], col);
}

/// Pause bars of the round PLAY button (same grid): two 2.75 × 8 bars.
pub fn pause_bars(p: &Painter, c: Pos2, k: f32, col: Color32) {
    for x in [-4.0, 1.25] {
        p.rect_filled(Rect::from_min_max(c + vec2(x, -4.0) * k, c + vec2(x + 2.75, 4.0) * k), 0.5 * k, col);
    }
}

/// Rating star outline, 10.5 pt across.
pub fn star(p: &Painter, c: Pos2, col: Color32, filled: bool) {
    let c = c + vec2(0.0, 0.4);
    let pts: Vec<Pos2> = (0..10).map(|i| {
        let a = std::f32::consts::PI / 5.0 * i as f32 - std::f32::consts::FRAC_PI_2;
        let r = if i % 2 == 0 { 5.6 } else { 2.3 };
        c + vec2(a.cos(), a.sin()) * r
    }).collect();
    if filled {
        for i in 0..10 {
            fill(p, vec![c, pts[i], pts[(i + 1) % 10]], col);
        }
    } else {
        p.add(Shape::closed_line(pts, Stroke::new(0.8, col)));
    }
}

/// Comment mark: a 12 × 7 rounded bubble with a tail (12 × 10 grid), top-left at `o`.
pub fn bubble(p: &Painter, o: Pos2, col: Color32) {
    p.rect_filled(Rect::from_min_size(o, vec2(12.0, 7.0)), 2.0, col);
    fill(p, vec![o + vec2(2.0, 6.5), o + vec2(6.5, 6.5), o + vec2(2.0, 10.0)], col);
}

/// The header's layout mark for a layout in menu order (0 = 1 Player, 1 = 2 Players,
/// 2 = Dual Player, 3 = Simple Player, 4 = Full Browser), centred on `c`: 1.5 pt frames.
pub fn layout_mark(p: &Painter, c: Pos2, kind: u8, col: Color32) {
    let frame = |r: Rect| {
        p.rect_stroke(r, 0.0, Stroke::new(1.5, col), egui::StrokeKind::Inside);
    };
    match kind {
        1 => {
            // Two decks one above the other, each with its block.
            for dy in [-4.25f32, 4.25] {
                let r = Rect::from_center_size(c + vec2(0.0, dy), vec2(18.0, 7.0));
                p.rect_stroke(r, 0.0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
                p.rect_filled(Rect::from_min_size(r.min + vec2(2.5, 2.5), vec2(2.0, 2.0)), 0.0, col);
            }
        }
        2 => {
            // Two frames overlapping.
            frame(Rect::from_min_size(c + vec2(-9.0, -7.0), vec2(14.0, 10.0)));
            frame(Rect::from_min_size(c + vec2(-5.0, -3.0), vec2(14.0, 10.0)));
        }
        3 => frame(Rect::from_center_size(c, vec2(18.0, 9.0))),
        4 => {
            let r = Rect::from_center_size(c, vec2(18.0, 13.0));
            frame(r);
            p.rect_filled(Rect::from_min_size(r.min + vec2(1.5, 2.5), vec2(15.0, 1.0)), 0.0, col);
        }
        _ => layout(p, c, col),
    }
}

/// Grid buttons of the two-deck layouts (Grid_Export2_Button_*, 18 × 18 grid centred on
/// `c`): three 2 × 10 beat bars with an arrow head at the side they move to (`dir` -1 left,
/// 1 right).
pub fn grid2_move(p: &Painter, c: Pos2, dir: f32, col: Color32) {
    // Grid coordinates of the "move right" glyph, mirrored for left.
    let g = |x: f32, y: f32| c + vec2(dir * (x - 9.0), y - 9.0);
    for x in [3.75, 6.75, 9.75] {
        p.rect_filled(Rect::from_two_pos(g(x, 4.0), g(x + 2.0, 14.0)), 0.0, col);
    }
    fill(p, vec![g(12.75, 7.0), g(15.75, 9.0), g(12.75, 11.0)], col);
}

/// Grid_Export2_Button_Center: one beat bar, white above and red below.
pub fn grid2_center(p: &Painter, c: Pos2, col: Color32, red: Color32) {
    p.rect_filled(Rect::from_min_max(c + vec2(-1.0, -5.0), c + vec2(1.0, 0.0)), 0.0, col);
    p.rect_filled(Rect::from_min_max(c + vec2(-1.0, 0.0), c + vec2(1.0, 5.0)), 0.0, red);
}

/// Pitch bend wedges (bendLeft / bendRight, 10 × 10 grid at `k` pt per unit): a shaft with a
/// barb rising to the side it bends to (`dir` -1 slower, 1 faster).
pub fn bend(p: &Painter, c: Pos2, k: f32, dir: f32, col: Color32) {
    // bendRight: M10 6.5 L5.5 2.5 L6 5 H0 V6.5 Z
    let g = |x: f32, y: f32| c + vec2(dir * (x - 5.0), y - 4.5) * k;
    p.rect_filled(Rect::from_two_pos(g(0.0, 5.0), g(10.0, 6.5)), 0.0, col);
    fill(p, vec![g(10.0, 6.5), g(5.5, 2.5), g(6.0, 5.0)], col);
}

/// TEMPO − / +: a 12 × 2 bar, crossed for plus.
pub fn tempo_sign(p: &Painter, c: Pos2, plus: bool, col: Color32) {
    p.rect_filled(Rect::from_center_size(c, if plus { vec2(11.0, 1.5) } else { vec2(10.0, 2.0) }), 0.0, col);
    if plus {
        p.rect_filled(Rect::from_center_size(c, vec2(1.5, 11.0)), 0.0, col);
    }
}

/// The waveform link (link.svg, 12 × 8 at `k` pt per unit): two rings hooked into each other.
pub fn link(p: &Painter, c: Pos2, k: f32, col: Color32) {
    let arc = |o: Pos2, a0: f32, a1: f32| {
        let pts: Vec<Pos2> = (0..=20).map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / 20.0).to_radians();
            o + vec2(a.cos(), a.sin()) * 3.5 * k
        }).collect();
        p.add(Shape::line(pts, Stroke::new(1.45 * k, col)));
    };
    // Each ring is open where the other passes through it.
    arc(c + vec2(-2.2, 1.7) * k, -10.0, 290.0);
    arc(c + vec2(2.2, -1.7) * k, 170.0, 470.0);
}

/// Dual control (dualControllOff / On, 23 × 21 grid): two 12.5 × 8.5 frames, apart and white
/// when off, overlapping and blue when on.
pub fn dual_control(p: &Painter, c: Pos2, on: bool) {
    let g = |x: f32, y: f32| c + vec2(x - 11.5, y - 10.5) * 1.13;
    let (a, b, col) = if on { (g(2.75, 3.75), g(7.75, 8.75), Color32::from_rgb(0x14, 0x73, 0xEB)) } else { (g(0.75, 0.75), g(9.75, 11.75), crate::theme::WHITE.c()) };
    for o in [a, b] {
        p.rect_stroke(Rect::from_min_size(o, vec2(12.5, 8.5) * 1.13).expand(0.75), 0.0, Stroke::new(1.5, col), egui::StrokeKind::Inside);
    }
}

/// The A-GAIN knob: a dark 18 pt cap, a blue arc from straight up to the pointer and a white
/// 2 × 5 pointer mark at `deg` degrees from straight up. `lit`: a track is loaded.
pub fn gain_knob(p: &Painter, c: Pos2, deg: f32, lit: bool) {
    p.circle_filled(c, 10.0, Color32::from_rgb(0x19, 0x19, 0x19));
    let at = |d: f32, r: f32| {
        let a = (d - 90.0).to_radians();
        c + vec2(a.cos(), a.sin()) * r
    };
    if lit && deg.abs() > 1.0 {
        let n = (deg.abs() / 4.0).ceil().max(2.0) as usize;
        let pts: Vec<Pos2> = (0..=n).map(|i| at(deg * i as f32 / n as f32, 8.75)).collect();
        p.add(Shape::line(pts, Stroke::new(2.5, Color32::from_rgb(0x14, 0x73, 0xEB))));
    }
    p.line_segment([at(deg, 4.5), at(deg, 10.0)], Stroke::new(2.0, if lit { Color32::WHITE } else { Color32::from_rgb(0xC8, 0xC8, 0xC8) }));
}

/// PHRASE EDIT's scissors-free CUT uses text; its list arrow is the pull-down chevron.
pub fn hamburger(p: &Painter, c: Pos2, col: Color32) {
    for dy in [-5.0f32, 0.0, 5.0] {
        p.rect_filled(Rect::from_center_size(c + vec2(0.0, dy), vec2(13.0, 2.0)), 0.0, col);
    }
}
