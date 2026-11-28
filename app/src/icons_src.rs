//! Filled pictograms for the source strip (left) and the window strip (right), drawn on
//! rekordbox's 18 x 18 tile grid: coordinates below are in tile units, so each glyph keeps
//! its proportions at any tile size. `c` is the glyph colour, `bg` the tile's fill (used for
//! the parts cut out of a glyph).

use eframe::egui;
use egui::{vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

pub type SrcIcon = fn(&Painter, Rect, Color32, Color32);

struct T {
    o: Pos2,
    k: f32,
}

impl T {
    fn new(r: Rect) -> T {
        // Glyphs are laid out on an 18-unit square centred in the tile.
        let k = r.width().min(r.height()) / 18.0;
        T { o: r.center() - vec2(9.0, 9.0) * k, k }
    }
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.o + vec2(x, y) * self.k
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.p(x0, y0), self.p(x1, y1))
    }
    fn fill(&self, p: &Painter, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        p.rect_filled(self.rect(x0, y0, x1, y1), 0.0, c);
    }
    fn poly(&self, p: &Painter, pts: &[(f32, f32)], c: Color32) {
        p.add(Shape::convex_polygon(pts.iter().map(|(x, y)| self.p(*x, *y)).collect(), c, Stroke::NONE));
    }
    fn line(&self, p: &Painter, pts: &[(f32, f32)], w: f32, c: Color32) {
        p.add(Shape::line(pts.iter().map(|(x, y)| self.p(*x, *y)).collect(), Stroke::new(w * self.k, c)));
    }
}

/// Collection: a bookmark.
pub fn collection(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    // One rectangle with the notch cut out of its foot (two triangles would show a seam).
    t.fill(p, 4.75, 4.0, 13.25, 15.0, c);
    t.poly(p, &[(4.75, 15.2), (9.0, 12.0), (13.25, 15.2)], bg);
}

/// Playlists: a filled sheet with three bullet rows cut out.
pub fn playlist(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    t.fill(p, 3.0, 3.0, 15.0, 15.0, c);
    for y in [5.0, 8.0, 11.0] {
        t.fill(p, 5.0, y, 7.0, y + 2.0, bg);
        t.fill(p, 8.0, y, 13.0, y + 2.0, bg);
    }
}

/// Related Tracks: two dots joined by a line.
pub fn related(p: &Painter, r: Rect, c: Color32, _bg: Color32) {
    let t = T::new(r);
    t.line(p, &[(5.0, 13.0), (13.0, 5.0)], 1.5, c);
    p.circle_filled(t.p(12.5, 5.5), 2.5 * t.k, c);
    p.circle_filled(t.p(5.5, 12.5), 2.5 * t.k, c);
}

/// iTunes: a beamed pair of notes.
pub fn itunes(p: &Painter, r: Rect, c: Color32, _bg: Color32) {
    let t = T::new(r);
    p.circle_filled(t.p(6.0, 12.6), 2.1 * t.k, c);
    p.circle_filled(t.p(12.0, 11.4), 2.1 * t.k, c);
    t.fill(p, 6.9, 5.2, 8.1, 12.6, c);
    t.fill(p, 12.9, 4.0, 14.1, 11.4, c);
    t.poly(p, &[(6.9, 4.6), (14.1, 3.2), (14.1, 5.8), (6.9, 7.2)], c);
}

/// rekordbox xml: a sheet with a clipped corner and angle brackets cut out.
pub fn xml(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    t.poly(p, &[(7.0, 3.0), (14.0, 3.0), (14.0, 15.0), (4.0, 15.0), (4.0, 6.0)], c);
    t.line(p, &[(7.8, 7.0), (5.35, 9.5), (7.8, 12.0)], 1.2, bg);
    t.line(p, &[(10.2, 7.0), (12.65, 9.5), (10.2, 12.0)], 1.2, bg);
}

/// Explorer: a laptop.
pub fn explorer(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    p.rect_filled(t.rect(3.6, 3.5, 14.6, 11.5), 0.3 * t.k, c);
    p.rect_filled(t.rect(4.6, 4.5, 13.6, 10.5), 0.3 * t.k, bg);
    t.poly(p, &[(2.1, 12.5), (15.9, 12.5), (15.3, 14.5), (2.7, 14.5)], c);
}

/// Devices: a USB plug seen from the front.
pub fn device(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    p.rect_filled(t.rect(5.0, 3.0, 13.0, 8.0), 0.4 * t.k, c);
    t.fill(p, 6.5, 5.0, 8.0, 6.5, bg);
    t.fill(p, 10.0, 5.0, 11.5, 6.5, bg);
    t.fill(p, 4.0, 8.0, 5.0, 16.0, c);
    t.fill(p, 13.0, 8.0, 14.0, 16.0, c);
}

/// Histories: a note inside an anticlockwise arrow.
pub fn history(p: &Painter, r: Rect, c: Color32, _bg: Color32) {
    let t = T::new(r);
    let (cx, cy, rad) = (9.5, 9.2, 5.2);
    let pts: Vec<(f32, f32)> = (0..=28)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + 1.75 * i as f32 / 28.0);
            (cx + rad * a.cos(), cy + rad * a.sin())
        })
        .collect();
    t.line(p, &pts, 1.3, c);
    t.poly(p, &[(2.3, 8.7), (6.7, 8.7), (4.55, 11.45)], c);
    t.fill(p, 9.0, 6.5, 10.0, 10.9, c);
    t.line(p, &[(9.9, 7.1), (11.0, 7.4), (11.8, 8.3)], 1.2, c);
    p.add(Shape::ellipse_filled(t.p(8.5, 10.8), vec2(1.5, 1.2) * t.k, c));
}

/// Recordings: a dot in a ring.
pub fn recording(p: &Painter, r: Rect, c: Color32, _bg: Color32) {
    let t = T::new(r);
    p.circle_stroke(t.p(9.0, 9.0), 5.4 * t.k, Stroke::new(1.2 * t.k, c));
    p.circle_filled(t.p(9.0, 9.0), 3.6 * t.k, c);
}

/// Hot Cue Bank Lists: three pads with their rows.
pub fn hot_cue_bank(p: &Painter, r: Rect, c: Color32, _bg: Color32) {
    let t = T::new(r);
    for y in [4.0, 8.0, 12.0] {
        t.fill(p, 3.0, y, 6.0, y + 3.0, c);
        t.fill(p, 8.0, y + 1.0, 15.0, y + 2.0, c);
    }
}

/// My Tag: a label with its eyelet.
pub fn mytag(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    t.poly(p, &[(13.6, 4.45), (14.05, 9.35), (7.4, 16.0), (2.05, 10.65), (8.7, 4.0)], c);
    p.circle_filled(t.p(11.05, 7.0), 1.5 * t.k, bg);
}

/// Information: an "i" cut out of a disc.
pub fn info(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    p.circle_filled(t.p(9.0, 9.0), 6.0 * t.k, c);
    p.circle_filled(t.p(9.0, 5.75), 1.25 * t.k, bg);
    t.fill(p, 8.0, 8.0, 10.0, 13.0, bg);
}

/// Sub-browser: a window whose right half is cut out.
pub fn sub_browser(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    t.fill(p, 3.0, 4.0, 15.0, 14.0, c);
    t.fill(p, 9.0, 5.0, 14.0, 13.0, bg);
}

/// Matching (skin: track-suggestion): a folder with an atom cut out of it.
pub fn matching(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    t.fill(p, 2.0, 4.5, 16.0, 15.5, c);
    t.poly(p, &[(2.0, 2.5), (6.5, 2.5), (8.0, 4.5), (2.0, 4.5)], c);
    for k in 0..3 {
        let rot = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 3.0;
        let pts: Vec<(f32, f32)> = (0..=32)
            .map(|i| {
                let a = i as f32 / 32.0 * std::f32::consts::TAU;
                let (x, y) = (4.75 * a.cos(), 1.75 * a.sin());
                (8.75 + x * rot.cos() - y * rot.sin(), 10.25 + x * rot.sin() + y * rot.cos())
            })
            .collect();
        t.line(p, &pts, 0.95, bg);
    }
    p.circle_filled(t.p(8.75, 10.25), 0.9 * t.k, bg);
}

/// Tag List: a sheet of rows with a tick.
pub fn tag_list(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let t = T::new(r);
    t.fill(p, 3.0, 3.0, 13.0, 14.0, c);
    for y in [5.0, 7.0, 9.0] {
        t.fill(p, 5.0, y, 6.0, y + 1.0, bg);
        t.fill(p, 7.0, y, 11.0, y + 1.0, bg);
    }
    t.line(p, &[(6.6, 11.2), (10.7, 15.0), (16.3, 8.6)], 2.6, bg);
    t.line(p, &[(7.2, 11.6), (10.7, 14.8), (15.8, 8.9)], 1.3, c);
}

/// Collapse chevron of the tree area (skin: 6 x 10, a 2.4 pt thick angle).
pub fn chevron(p: &Painter, r: Rect, c: Color32) {
    let (kx, ky) = (r.width() / 6.0, r.height() / 10.0);
    let q = |x: f32, y: f32| r.min + vec2(x * kx, y * ky);
    p.add(Shape::convex_polygon(vec![q(0.0, 0.0), q(6.0, 5.09), q(3.2, 5.09), q(0.0, 2.37)], c, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![q(3.2, 5.09), q(6.0, 5.09), q(0.0, 10.0), q(0.0, 7.82)], c, Stroke::NONE));
}

/// Sub-window glyph (skin: subBrowse_vertical, 16 x 16): a frame whose lower pane is cut out.
pub fn window_split(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let k = r.width() / 16.0;
    let q = |x: f32, y: f32| r.min + vec2(x, y) * k;
    p.rect_filled(Rect::from_min_max(q(2.0, 3.17), q(14.0, 12.83)), 0.0, c);
    p.rect_filled(Rect::from_min_max(q(3.17, 7.0), q(12.83, 11.67)), 0.0, bg);
}

/// Library sync mark (skin: syncManager, 18 x 12): two half-circle arrows.
pub fn sync_arrows(p: &Painter, r: Rect, c: Color32) {
    let k = r.width() / 18.0;
    let q = |x: f32, y: f32| r.min + vec2(x, y) * k;
    let arc = |a0: f32, a1: f32| -> Vec<Pos2> {
        (0..=14).map(|i| { let a = (a0 + (a1 - a0) * i as f32 / 14.0).to_radians(); q(9.03 + 5.25 * a.cos(), 6.0 + 5.25 * a.sin()) }).collect()
    };
    p.add(Shape::line(arc(-175.0, -62.0), Stroke::new(1.1 * k, c)));
    p.add(Shape::line(arc(5.0, 118.0), Stroke::new(1.1 * k, c)));
    p.add(Shape::convex_polygon(vec![q(11.9, 5.75), q(16.3, 5.75), q(14.1, 3.3)], c, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![q(1.75, 6.25), q(6.15, 6.25), q(3.95, 8.7)], c, Stroke::NONE));
}

/// List glyph of the column-menu and palette tiles (skin: 10 x 10 at 4,4 of an 18 pt tile):
/// three bullets with their rows.
pub fn bullet_rows(p: &Painter, r: Rect, c: Color32) {
    let t = T::new(r);
    for y in [4.0, 7.89, 11.78] {
        t.fill(p, 4.0, y, 6.0, y + 2.22, c);
        t.fill(p, 7.0, y, 14.0, y + 2.22, c);
    }
}

/// "Show the upper part of the artwork" (skin: artworkPart): four bullets with their rows.
pub fn bullet_rows4(p: &Painter, r: Rect, c: Color32) {
    let t = T::new(r);
    for y in [4.0, 6.79, 9.58, 12.37] {
        t.fill(p, 4.0, y, 6.03, y + 1.63, c);
        t.fill(p, 6.88, y, 14.0, y + 1.63, c);
    }
}

/// Column browser: three upright bars.
pub fn columns3(p: &Painter, r: Rect, c: Color32) {
    let t = T::new(r);
    for x in [4.0, 7.75, 11.5] {
        t.fill(p, x, 4.0, x + 2.5, 14.0, c);
    }
}

/// Playlist search tile glyph (skin: playlist_search, 23 x 23): a sheet of rows behind a magnifier.
pub fn playlist_search(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let q = |x: f32, y: f32| r.min + vec2(x * r.width() / 23.0, y * r.height() / 23.0);
    p.rect_filled(Rect::from_min_max(q(5.0, 4.0), q(15.0, 16.0)), 0.0, c);
    for y in [6.0, 9.0, 12.5] {
        p.rect_filled(Rect::from_min_max(q(6.0, y), q(14.0, y + 1.5)), 0.0, bg);
    }
    p.circle_filled(q(12.44, 13.44), 5.5, bg);
    p.circle_stroke(q(12.44, 13.44), 3.9, Stroke::new(1.1, c));
    p.line_segment([q(15.4, 16.4), q(17.6, 18.6)], Stroke::new(1.2, c));
}

/// Sub-browser tile glyph as the references show it: a narrow pane beside a wide one.
pub fn side_pane(p: &Painter, r: Rect, c: Color32) {
    let t = T::new(r);
    t.fill(p, 4.0, 4.0, 6.5, 14.0, c);
    t.fill(p, 8.25, 4.0, 14.0, 14.0, c);
}

// ---------------------------------------------------------------- tree glyphs (skins: brows_treeicon_*)
/// Disclosure triangle of a tree row: 8 x 6.5 pt, centred on `c`.
pub fn tree_arrow(p: &Painter, c: Pos2, open: bool, col: Color32) {
    let pts = if open {
        vec![c + vec2(-4.0, -3.25), c + vec2(4.0, -3.25), c + vec2(0.0, 3.25)]
    } else {
        vec![c + vec2(-3.25, -4.0), c + vec2(3.25, 0.0), c + vec2(-3.25, 4.0)]
    };
    p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
}

/// Grid for a glyph of `w` x `h` skin units centred on `c` (1 unit = 1 pt).
struct G(Pos2);
impl G {
    fn at(c: Pos2, w: f32, h: f32) -> G {
        G(c - vec2(w, h) * 0.5)
    }
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.0 + vec2(x, y)
    }
    fn fill(&self, p: &Painter, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        p.rect_filled(Rect::from_min_max(self.p(x0, y0), self.p(x1, y1)), 0.0, c);
    }
}

/// All Tracks: an eighth note (11.5 x 13).
pub fn tree_note(p: &Painter, c: Pos2, col: Color32) {
    let g = G::at(c, 11.5, 13.0);
    p.add(Shape::ellipse_filled(g.p(3.0, 10.3), vec2(3.0, 2.6), col));
    g.fill(p, 5.0, 0.0, 6.1, 10.3, col);
    p.add(Shape::line(vec![g.p(5.6, 0.4), g.p(8.2, 2.6), g.p(10.6, 5.2), g.p(8.4, 8.4)], Stroke::new(1.15, col)));
}

/// Folder (12 x 10).
pub fn tree_folder(p: &Painter, c: Pos2, col: Color32) {
    let g = G::at(c, 12.0, 10.0);
    g.fill(p, 0.0, 2.0, 12.0, 10.0, col);
    p.add(Shape::convex_polygon(vec![g.p(1.06, 2.1), g.p(2.0, 0.0), g.p(5.0, 0.0), g.p(6.03, 2.1)], col, Stroke::NONE));
}

/// Playlist (10 x 11): a sheet with four bullet rows cut out.
pub fn tree_list(p: &Painter, c: Pos2, col: Color32, bg: Color32) {
    let g = G::at(c, 10.0, 11.0);
    g.fill(p, 0.0, 0.0, 10.0, 11.0, col);
    for y in [2.0, 4.0, 6.0, 8.0] {
        g.fill(p, 2.0, y, 3.0, y + 1.0, bg);
        g.fill(p, 4.0, y, 8.0, y + 1.0, bg);
    }
}

/// Intelligent playlist (15 x 16): the sheet with a gear over its lower right corner.
pub fn tree_smart(p: &Painter, c: Pos2, col: Color32, bg: Color32) {
    let g = G::at(c, 15.0, 16.0);
    g.fill(p, 0.0, 0.0, 10.0, 11.0, col);
    for y in [2.0, 4.0, 6.0, 8.0] {
        g.fill(p, 2.0, y, 3.0, y + 1.0, bg);
        if y < 3.0 {
            g.fill(p, 4.0, y, 8.0, y + 1.0, bg);
        }
    }
    let gc = g.p(9.5, 10.47);
    p.circle_filled(gc, 6.3, bg);
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let d = vec2(a.sin(), -a.cos()) * 5.5;
        p.line_segment([gc - d, gc + d], Stroke::new(1.25, col));
    }
    p.circle_filled(gc, 3.7, col);
    p.circle_filled(gc, 2.05, bg);
}

/// Device (8 x 14): a USB stick.
pub fn tree_device(p: &Painter, c: Pos2, col: Color32, bg: Color32) {
    let g = G::at(c, 8.0, 14.0);
    g.fill(p, 0.0, 6.0, 8.0, 14.0, col);
    g.fill(p, 1.0, 0.0, 7.0, 5.0, col);
    g.fill(p, 2.0, 1.0, 3.0, 3.0, bg);
    g.fill(p, 5.0, 1.0, 6.0, 3.0, bg);
}

/// Hot Cue Bank List (10 x 11): three pills.
pub fn tree_bank(p: &Painter, c: Pos2, col: Color32) {
    let g = G::at(c, 10.0, 11.0);
    for y in [0.0, 4.0, 8.0] {
        p.rect_filled(Rect::from_min_max(g.p(0.0, y), g.p(10.0, y + 3.0)), 1.5, col);
    }
}

/// Padlock of a locked row (8 x 10).
pub fn tree_lock(p: &Painter, c: Pos2, col: Color32, bg: Color32) {
    let g = G::at(c, 8.0, 10.0);
    p.rect_filled(Rect::from_min_max(g.p(1.5, 0.0), g.p(6.5, 6.0)), 2.5, col);
    p.rect_filled(Rect::from_min_max(g.p(2.7, 1.2), g.p(5.3, 6.0)), 1.3, bg);
    g.fill(p, 0.0, 4.0, 8.0, 10.0, col);
    g.fill(p, 3.5, 6.0, 4.5, 8.0, bg);
}

/// Track filter funnel (skin: brows_filter, 10 x 10).
pub fn funnel(p: &Painter, c: Pos2, col: Color32) {
    let g = G::at(c, 10.0, 10.0);
    p.add(Shape::convex_polygon(vec![g.p(0.0, 0.0), g.p(10.0, 0.0), g.p(6.0, 4.8), g.p(4.0, 4.8)], col, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![g.p(4.0, 4.6), g.p(6.0, 4.6), g.p(6.0, 10.0), g.p(4.0, 8.0)], col, Stroke::NONE));
}

/// Rating star (skins: rating_on / rating_off, 12 x 11): filled, or a thin outline. The
/// skin's star has an outer radius of 5.83 and an inner one of 2.27 and sits 0.67 below the
/// box centre.
pub fn star(p: &Painter, c: Pos2, filled: bool, col: Color32) {
    let cc = c + vec2(0.0, 0.67);
    let pt = |i: usize, r: f32| {
        let a = (-90.0 + 36.0 * i as f32).to_radians();
        cc + vec2(a.cos(), a.sin()) * r
    };
    if filled {
        let inner: Vec<Pos2> = (0..5).map(|i| pt(2 * i + 1, 2.27)).collect();
        p.add(Shape::convex_polygon(inner.clone(), col, Stroke::NONE));
        for i in 0..5 {
            p.add(Shape::convex_polygon(vec![inner[(i + 4) % 5], pt(2 * i, 5.83), inner[i]], col, Stroke::NONE));
        }
    } else {
        let pts: Vec<Pos2> = (0..10).map(|i| pt(i, if i % 2 == 0 { 5.7 } else { 2.2 })).collect();
        p.add(Shape::closed_line(pts, Stroke::new(0.75, col)));
    }
}

/// Sort arrow of a column header (skins: sortUp / sortDown), in a 12 pt box centred on `c`.
pub fn sort_arrow(p: &Painter, c: Pos2, up: bool, col: Color32) {
    let s = if up { 1.0 } else { -1.0 };
    let st = Stroke::new(1.2, col);
    p.line_segment([c + vec2(0.0, -5.0 * s), c + vec2(0.0, 5.4 * s)], st);
    p.add(Shape::line(vec![c + vec2(-3.6, -1.2 * s), c + vec2(0.0, -4.8 * s), c + vec2(3.6, -1.2 * s)], st));
}

/// Magnifier with its scope triangle (skin: searchSelect, 25 x 14).
pub fn search_select(p: &Painter, r: Rect, col: Color32) {
    let k = r.width() / 25.0;
    let q = |x: f32, y: f32| r.min + vec2(x, y) * k;
    p.circle_stroke(q(4.98, 5.5), 4.25 * k, Stroke::new(1.7 * k, col));
    p.line_segment([q(8.3, 8.8), q(12.0, 12.5)], Stroke::new(2.1 * k, col));
    p.add(Shape::convex_polygon(vec![q(17.58, 4.98), q(24.4, 4.98), q(21.0, 9.37)], col, Stroke::NONE));
}

/// The tile itself: 18 pt, 1.5 pt corners, a 1 pt grey border; the selected one is blue with
/// a white border. Returns the fill the glyph's cut-outs should use.
pub fn tile(p: &Painter, r: Rect, on: bool, hovered: bool, enabled: bool, panel: Color32) -> Color32 {
    let blue = crate::theme::BLUE;
    if on {
        p.rect_filled(r, 1.5, blue);
        p.rect_stroke(r, 1.5, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
        blue.c()
    } else {
        let edge = if !enabled {
            Color32::from_rgb(0x3A, 0x3A, 0x3A)
        } else if hovered {
            Color32::from_rgb(0x8C, 0x8C, 0x8C)
        } else {
            Color32::from_rgb(0x64, 0x64, 0x64)
        };
        p.rect_stroke(r, 1.5, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
        panel
    }
}

// ---------------------------------------------------------------- 7.2.19 browser glyphs
/// An atom: three orbits 60 degrees apart around a dot, centred on `c` (`rx` x `ry` orbits).
pub fn atom(p: &Painter, c: Pos2, rx: f32, ry: f32, w: f32, col: Color32) {
    for k in 0..3 {
        let rot = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 3.0;
        let pts: Vec<Pos2> = (0..=40)
            .map(|i| {
                let a = i as f32 / 40.0 * std::f32::consts::TAU;
                let (x, y) = (rx * a.cos(), ry * a.sin());
                c + vec2(x * rot.cos() - y * rot.sin(), x * rot.sin() + y * rot.cos())
            })
            .collect();
        p.add(Shape::line(pts, Stroke::new(w, col)));
    }
    p.circle_filled(c, w * 1.1, col);
}

/// CUE Analysis Playlist (skin brows_treeicon_cuplaylist, 12 x 13): a sheet with two rows
/// whose lower right corner gives way to a downward triangle.
pub fn tree_cue_list(p: &Painter, c: Pos2, col: Color32) {
    let g = G::at(c, 12.0, 13.0);
    // Frame: top, left, and what is left of the bottom and right edges.
    g.fill(p, 0.86, 1.36, 9.43, 3.07, col);
    g.fill(p, 0.86, 1.36, 1.71, 11.64, col);
    g.fill(p, 8.57, 1.36, 9.43, 5.89, col);
    g.fill(p, 0.86, 9.93, 5.3, 11.64, col);
    // Rows.
    g.fill(p, 1.71, 4.36, 8.57, 5.64, col);
    g.fill(p, 1.71, 6.93, 3.57, 8.64, col);
    p.add(Shape::convex_polygon(vec![g.p(4.75, 6.91), g.p(11.17, 6.91), g.p(7.96, 12.48)], col, Stroke::NONE));
}

/// Skin brws_list_pathsystem_pulldown (10 x 16): an up and a down angle.
pub fn path_pulldown(p: &Painter, r: Rect, c: Color32) {
    let q = |x: f32, y: f32| r.min + vec2(x * r.width() / 10.0, y * r.height() / 16.0);
    p.add(Shape::convex_polygon(vec![q(5.09, 0.0), q(10.0, 6.0), q(7.82, 6.0), q(5.09, 2.79)], c, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![q(5.09, 0.0), q(5.09, 2.79), q(2.37, 6.0), q(0.0, 6.0)], c, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![q(4.91, 16.0), q(0.0, 10.0), q(2.18, 10.0), q(4.91, 13.21)], c, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![q(4.91, 16.0), q(4.91, 13.21), q(7.63, 10.0), q(10.0, 10.0)], c, Stroke::NONE));
}

/// Skin brows_treeicon_add_btn (12 x 12): a disc with a plus cut out.
pub fn plus_disc(p: &Painter, c: Pos2, col: Color32, bg: Color32) {
    p.circle_filled(c, 6.0, col);
    p.rect_filled(Rect::from_center_size(c, vec2(8.0, 2.0)), 0.0, bg);
    p.rect_filled(Rect::from_center_size(c, vec2(2.0, 8.0)), 0.0, bg);
}

/// Skin brws_setting (14 x 14): a gear with eight teeth and a round hole.
pub fn gear(p: &Painter, c: Pos2, col: Color32, bg: Color32) {
    p.circle_filled(c, 5.6, col);
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (a.cos(), a.sin());
        let (nx, ny) = (-dy, dx);
        let tip = c + vec2(dx, dy) * 7.0;
        let base = c + vec2(dx, dy) * 5.0;
        p.add(Shape::convex_polygon(vec![base + vec2(nx, ny) * 1.9, tip + vec2(nx, ny) * 1.6, tip - vec2(nx, ny) * 1.6, base - vec2(nx, ny) * 1.9], col, Stroke::NONE));
    }
    p.circle_filled(c, 3.5, bg);
}

/// Skin subBrows_related_note_icon (14 x 14): an eighth note.
pub fn sub_note(p: &Painter, r: Rect, col: Color32) {
    let q = |x: f32, y: f32| r.min + vec2(x, y) * (r.width() / 14.0);
    p.add(Shape::ellipse_filled(q(5.24, 10.45), vec2(2.16, 2.15) * (r.width() / 14.0), col));
    p.rect_filled(Rect::from_min_max(q(5.95, 1.4), q(7.39, 10.38)), 0.0, col);
    p.add(Shape::convex_polygon(vec![q(7.39, 1.4), q(10.85, 3.67), q(10.09, 4.83), q(6.67, 2.59)], col, Stroke::NONE));
}

/// Skin subBrows_related_sort_off_btn (12 x 13): an outlined sheet with three rows.
pub fn sub_sort(p: &Painter, c: Pos2, col: Color32) {
    let r = Rect::from_center_size(c, vec2(12.0, 13.0));
    p.rect_stroke(r, 2.0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
    for y in [4.0, 6.0, 8.0] {
        p.rect_filled(Rect::from_min_max(r.min + vec2(3.0, y), r.min + vec2(9.0, y + 1.0)), 0.0, col);
    }
}

/// Skin brws_refresh (12 x 16): two curved arrows chasing each other.
pub fn refresh(p: &Painter, c: Pos2, col: Color32) {
    let arc = |from: f32, to: f32| -> Vec<Pos2> { (0..=14).map(|i| { let a = from + (to - from) * i as f32 / 14.0; c + vec2(4.9 * a.cos(), 4.9 * a.sin()) }).collect() };
    use std::f32::consts::PI;
    p.add(Shape::line(arc(-0.25 * PI, 0.55 * PI), Stroke::new(1.7, col)));
    p.add(Shape::line(arc(0.75 * PI, 1.55 * PI), Stroke::new(1.7, col)));
    // Arrow heads: lower one points left at the bottom, upper one points right at the top.
    p.add(Shape::convex_polygon(vec![c + vec2(2.3, 2.0), c + vec2(2.3, 8.0), c + vec2(-2.2, 5.0)], col, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![c + vec2(-2.3, -2.0), c + vec2(-2.3, -8.0), c + vec2(2.2, -5.0)], col, Stroke::NONE));
}

/// Skin checkbox_on's tick, scaled to a `size` pt box centred on `c`.
pub fn tick(p: &Painter, c: Pos2, size: f32, col: Color32) {
    let k = size / 10.0;
    let q = |x: f32, y: f32| c + vec2(x - 5.0, y - 5.0) * k;
    p.add(Shape::convex_polygon(vec![q(2.0, 4.72), q(4.04, 6.5), q(4.04, 8.0), q(2.0, 6.05)], col, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![q(4.04, 6.5), q(8.0, 2.0), q(8.0, 3.57), q(4.04, 8.0)], col, Stroke::NONE));
}
