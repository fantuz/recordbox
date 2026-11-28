//! The preparation player (top of the window): track header, overview and enlarged
//! waveforms with beat grid and cue markers, transport, hot cues, memory cues, loops,
//! grid editing and the MEMORY / HOT CUE / INFO side panel.

use eframe::egui;
use crate::app::*;
use crate::model::*;
use crate::theme::*;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, Ui};

pub fn button(ui: &mut Ui, rect: Rect, id: &str, label: &str, fill: Color32, text: Color32, size: f32) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    let f = crate::ui_feel::face(&r, fill, lighten(fill), fill.gamma_multiply(0.7));
    ui.painter().rect_filled(rect, 3.0, f);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(size), text);
    r
}

pub(crate) fn lighten(c: Color32) -> Color32 {
    Color32::from_rgb(c.r().saturating_add(20), c.g().saturating_add(20), c.b().saturating_add(20))
}

pub(crate) fn tip(r: Response, app: &App, t: &str) -> Response {
    if app.prefs.tooltips {
        r.on_hover_text(t)
    } else {
        r
    }
}

/// An icon button: a box (optional) with a vector icon drawn by `draw`.
pub fn icon_button(ui: &mut Ui, rect: Rect, id: &str, fill: Option<Color32>, draw: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    if let Some(f) = fill {
        let f = crate::ui_feel::face(&r, f, lighten(f), f.gamma_multiply(0.7));
        ui.painter().rect_filled(rect, 2.0, f);
    } else {
        let h = if crate::motion::on() { crate::ui_feel::hover(&r) } else { r.hovered() as u8 as f32 };
        if h > 0.0 {
            ui.painter().rect_filled(rect, 2.0, crate::motion::fade(gy(0x1A1A1A), h));
        }
    }
    draw(ui.painter(), rect, WHITE.c());
    r
}

/// Share of half the enlarged waveform's height a full-scale value reaches (see `draw_detail`).
pub const DETAIL_FULL_SCALE: f32 = 0.68;

/// True once per press of a pad-like control: on the frame the button goes down on it, or,
/// when press and release arrive within one frame (a quick tap), on the click itself.
pub fn press_fired(ui: &Ui, resp: &Response) -> bool {
    let key = resp.id.with("press_fired");
    let down_now = resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed());
    if down_now {
        ui.ctx().data_mut(|d| d.insert_temp(key, true));
        return true;
    }
    if resp.clicked() {
        let was = ui.ctx().data_mut(|d| d.remove_temp::<bool>(key)).unwrap_or(false);
        return !was;
    }
    if !ui.input(|i| i.pointer.any_down()) {
        ui.ctx().data_mut(|d| d.remove::<bool>(key));
    }
    false
}

/// Horizontal positions in the player are measured on a 1512 pt wide window, where the
/// waveform area spans x = 80..1300. `sx` maps such an x to the current width.
pub(crate) fn sx(main: Rect, x: f32) -> f32 {
    main.min.x + (x - 80.0) * (main.width() / 1220.0)
}

/// The round CUE button (rekordbox's 36-unit button drawn `dia` pt across; 40 in the 1 Player
/// layout): a ring 1.5 units thick around a face of radius 15 units. `lit`: full-colour ring;
/// `loaded`: raised face, white label.
pub(crate) fn cue_button_face_t(p: &egui::Painter, c: Pos2, dia: f32, lit: bool, loaded: bool, hovered: f32, pressed: f32) {
    let k = dia / 36.0;
    // Unlit: the ring at 40 % over the #1E1E1E column.
    crate::icons_player::ring(p, c, 17.25 * k, 1.5 * k, if lit { CUE_ORANGE } else { hex(0x785C28) });
    let face = crate::motion::mix((if loaded { BTN } else { BG2 }).c(), gy(0x2E2E2E), hovered);
    p.circle_filled(c, 15.0 * k, crate::motion::mix(face, gy(0x161616), 0.6 * pressed));
    p.text(c - vec2(0.0, 0.25), Align2::CENTER_CENTER, "CUE", caps(9.0 * k), if loaded { WHITE.c() } else { gy(0x323232) });
}

pub(crate) fn play_control(app: &App, ui: &mut Ui, center: Pos2, hit_size: egui::Vec2, diameter: f32) -> bool {
    let enabled = !app.deck_options.disable_play;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(Rect::from_center_size(center, hit_size), ui.id().with("playbtn"), sense);
    if !enabled {
        play_button_face(ui.painter(), center, diameter, app.playing(), false);
        ui.painter().circle_filled(center, diameter / 2.0, Color32::from_black_alpha(150));
        response.on_hover_text("Playback is disabled for this session (--disable-play).");
        return false;
    }
    crate::ui_feel::round_face(ui.painter(), center, diameter, app.playing(), true, &response, false);
    tip(response, app, "Play/Pause").clicked()
}

/// The round PLAY button on the same grid: stopped, a 40 % green ring and a grey triangle;
/// playing, the full ring, a raised face and white pause bars.
pub(crate) fn play_button_face(p: &egui::Painter, c: Pos2, dia: f32, playing: bool, hovered: bool) {
    play_button_face_t(p, c, dia, playing, hovered as u8 as f32, 0.0)
}

/// `play_button_face` with hover and press as amounts.
pub(crate) fn play_button_face_t(p: &egui::Painter, c: Pos2, dia: f32, playing: bool, hovered: f32, pressed: f32) {
    let k = dia / 36.0;
    crate::icons_player::ring(p, c, 17.25 * k, 1.5 * k, if playing { PLAY_GREEN } else { hex(0x246822) });
    let face = crate::motion::mix((if playing { BTN } else { BG2 }).c(), gy(0x2A2A2A), hovered);
    p.circle_filled(c, 15.0 * k, crate::motion::mix(face, gy(0x161616), 0.6 * pressed));
    if playing {
        crate::icons_player::pause_bars(p, c, k, WHITE.c());
    } else {
        crate::icons_player::play_tri(p, c + vec2(0.4, 0.0), k, gy(0x646464));
    }
}

pub fn player_panel(app: &mut App, ui: &mut Ui) {
    // All positions below were measured on rekordbox 7.2.19 at 1512 pt (y from the player's
    // top edge, which is 28 pt below the window's content top).
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    let left = Rect::from_min_size(full.min, vec2(80.0, full.height()));
    let right = Rect::from_min_max(pos2(full.max.x - 202.0, full.min.y), full.max);
    let main = Rect::from_min_max(pos2(left.max.x, full.min.y), pos2(full.max.x - 212.0, full.max.y));
    p.rect_filled(left, 0.0, BG2);

    let pos = crate::ui_feel::shown_pos(app);
    let playing = app.playing();
    let loaded = app.deck.track.is_some();

    // ---------------- left column: 26 pt tiles at x 12–38 and 42–68; CUE and PLAY 40 pt across
    let lx = left.min.x;
    let ly = left.min.y;
    let tile = |x: f32, y: f32| Rect::from_min_size(pos2(lx + x, ly + y), vec2(26.0, 26.0));
    // Previous Track / Next Track. Previous Track first returns to the start of the track, as on a player.
    if tip(icon_button(ui, tile(12.0, 22.0), "front", Some(BTN.c()), |p, r, c| crate::icons_player::track_jump(p, r.center(), 1.5, c, -1.0)), app, "Previous Track").clicked() {
        app.previous_track();
    }
    if tip(icon_button(ui, tile(42.0, 22.0), "rear", Some(BTN.c()), |p, r, c| crate::icons_player::track_jump(p, r.center(), 1.5, c, 1.0)), app, "Next Track").clicked() {
        app.load_adjacent(1);
    }
    if app.searching() != 0 {
        app.search(0);
    }
    let bj = app.prefs.beat_jump;
    if tip(icon_button(ui, tile(12.0, 68.0), "bjl", Some(BTN.c()), |p, r, c| crate::icons_player::chevron_left(p, r.center(), 1.5, c)), app, "Jump Reverse").clicked() {
        app.beat_jump(-bj);
    }
    if tip(icon_button(ui, tile(42.0, 68.0), "bjr", Some(BTN.c()), |p, r, c| crate::icons_player::chevron_right(p, r.center(), 1.5, c)), app, "Jump Forward").clicked() {
        app.beat_jump(bj);
    }
    // Jump length: a square-cornered 56 × 26 tile under the two arrows.
    let bj_rect = Rect::from_min_size(pos2(lx + 12.0, ly + 98.0), vec2(56.0, 26.0));
    let bj_label = crate::player_ext::jump_label(bj, app.prefs.beat_jump_bars);
    let r = tip(ui.interact(bj_rect, ui.id().with("bjsize"), Sense::click()), app, "Select the beat/bar length jumping from the current position.");
    p.rect_filled(bj_rect, 0.0, crate::ui_feel::hl(&r, BTN.c(), lighten(BTN.c())));
    p.text(pos2(bj_rect.center().x, ly + 106.2), Align2::CENTER_CENTER, bj_label, bold(9.0), WHITE.c());
    crate::icons_player::chevron_down(&p, pos2(lx + 40.75, ly + 118.0), 1.0, WHITE.c());
    crate::ui_browser::dropdown(ui, &r, "bjpop", |ui: &mut Ui| {
        crate::ui_player2::jump_size_menu(app, ui);
    });
    // CUE
    let cue_c = pos2(lx + 40.0, ly + 164.0);
    let cue_rect = Rect::from_center_size(cue_c, vec2(42.0, 42.0));
    let cr = ui.interact(cue_rect, ui.id().with("cuebtn"), Sense::click_and_drag());
    let cue_lit = loaded && (!playing || app.deck.cue_preview);
    crate::ui_feel::round_face(&p, cue_c, 40.0, cue_lit, loaded, &cr, true);
    if press_fired(ui, &cr) && !app.deck.cue_preview {
        app.cue_down();
    }
    let cue_key_held = app.prefs.key_for("cue").map(|k| ui.input(|i| i.key_down(k))).unwrap_or(false);
    if !cr.is_pointer_button_down_on() && app.deck.cue_preview && !cue_key_held {
        app.cue_up();
    }
    // PLAY
    let play_c = pos2(lx + 40.0, ly + 214.0);
    if play_control(app, ui, play_c, vec2(42.0, 42.0), 40.0) {
        app.play_pause();
    }

    let my = main.min.y;
    // The waveform rectangle stays black in Light (`PlayerPanelL::paint`: its bottom is 48 pt
    // above the panel's; the top edge was not traced and is taken under the overview).
    p.rect_filled(Rect::from_min_max(pos2(main.min.x, my + 62.0), pos2(main.max.x, my + 208.0)), 0.0, WAVE_BG);
    // ---------------- enlarged waveform (drawn first; the zoom buttons sit on top of it):
    // beats from y 88 to 202, the playhead 692.5 pt from the window's left edge.
    let det = Rect::from_min_max(pos2(main.min.x, my + 88.0), pos2(main.max.x, my + 202.0));
    let mode = if app.deck.ctrl_tab == 1 { DetailMode::GridEdit } else { DetailMode::Full };
    draw_detail_at(app, ui, det, pos, 2.5 * (main.width() / 1220.0), mode);

    // ---------------- zoom column (overlaid on the left of the enlarged waveform), on an opaque
    // black strip so the waveform never shows through the buttons.
    ui.painter().rect_filled(Rect::from_min_max(pos2(main.min.x, my + 100.0), pos2(main.min.x + 26.0, my + 176.0)), 0.0, ZOOM_BG.c());
    let zc = |y: f32| Rect::from_center_size(pos2(main.min.x + 14.0, my + y), vec2(22.0, 20.0));
    if tip(icon_button(ui, zc(113.0), "zin", None, |p, r, c| crate::icons_player::zoom(p, r.center(), 7.0 / 6.0, c, true)), app, "Zoom In").clicked() && app.prefs.zoom > 0 {
        app.prefs.zoom -= 1;
    }
    if tip(button(ui, zc(137.4), "zrst", "RST", Color32::TRANSPARENT, WHITE.c(), 8.2), app, "Reset Zoom").clicked() {
        app.prefs.zoom = 2;
    }
    if tip(icon_button(ui, zc(163.0), "zout", None, |p, r, c| crate::icons_player::zoom(p, r.center(), 7.0 / 6.0, c, false)), app, "Zoom Out").clicked() && app.prefs.zoom + 1 < ZOOMS.len() {
        app.prefs.zoom += 1;
    }

    // ---------------- header row (title, artist, times, key, BPM)
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    let dur = app.deck.duration;
    // 1 pt dividers from y 2 to 19.
    let div = |x: f32| p.rect_filled(Rect::from_min_max(pos2(x.floor(), my + 2.0), pos2(x.floor() + 1.0, my + 19.0)), 0.0, OFF);
    let x_artist = sx(main, 733.0);
    let x_time = sx(main, 1011.0);
    let x_key = sx(main, 1168.0);
    let x_bpm = sx(main, 1224.0);
    if let Some(t) = &track {
        // The title starts at x 156.5 (rekordbox puts a streaming service's mark before it,
        // which moves it to 179; local files have none).
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(sx(main, 156.5), my), pos2(x_artist - 8.0, my + 21.0)));
        clip.text(pos2(sx(main, 156.5), my + HDR_TITLE_Y), Align2::LEFT_CENTER, t.display_title(), bold(15.5), WHITE.c());
        let clip = p.with_clip_rect(Rect::from_min_max(pos2(x_artist, my), pos2(x_time - 8.0, my + 21.0)));
        clip.text(pos2(x_artist + 16.5, my + HDR_TEXT_Y), Align2::LEFT_CENTER, &t.artist, FontId::proportional(12.25), DIM.c());
    }
    for x in [x_artist, x_time, x_key, x_bpm] {
        div(x);
    }
    let remain = (dur - pos).max(0.0);
    let dim_t = if loaded { DIM.c() } else { OFF.c() };
    let bright_t = if loaded { WHITE.c() } else { OFF.c() };
    let (big, small) = split_time(if loaded { remain } else { 0.0 });
    let (c1, c2) = if app.prefs.time_remain { (bright_t, dim_t) } else { (dim_t, bright_t) };
    // Remaining time from x = 1032, elapsed from 1100.5; tenths in a smaller size on the same baseline.
    let tr = p.text(pos2(x_time + 20.5, my + HDR_TIME_Y), Align2::LEFT_CENTER, format!("-{big}"), FontId::proportional(15.0), c1);
    p.text(pos2(tr.max.x, my + HDR_TIME_Y + 1.4), Align2::LEFT_CENTER, format!(".{small}"), FontId::proportional(11.5), c1);
    let (big2, small2) = split_time(if loaded { pos } else { 0.0 });
    let er = p.text(pos2(x_time + 89.0, my + HDR_TIME_Y), Align2::LEFT_CENTER, &big2, FontId::proportional(15.0), c2);
    p.text(pos2(er.max.x, my + HDR_TIME_Y + 1.4), Align2::LEFT_CENTER, format!(".{small2}"), FontId::proportional(11.5), c2);
    let time_hit = ui.interact(Rect::from_min_max(pos2(x_time, my), pos2(x_key, my + 21.0)), ui.id().with("timesw"), Sense::click());
    if tip(time_hit, app, "Switch Time/Remain").clicked() {
        app.prefs.time_remain = !app.prefs.time_remain;
    }
    if let Some(t) = &track {
        p.text(pos2((x_key + x_bpm) / 2.0 + 0.5, my + HDR_TEXT_Y - 0.5), Align2::CENTER_CENTER, t.key_label(app.prefs.key_alnum), FontId::proportional(13.0), WHITE.c());
    }
    if loaded {
        let b = app.current_bpm();
        p.text(pos2(sx(main, 1263.0), my + HDR_TEXT_Y - 0.5), Align2::CENTER_CENTER, if b > 0.0 { format!("{b:.2}") } else { "--.--".into() }, FontId::proportional(13.0), WHITE.c());
    }

    // ---------------- artwork: a 48 pt box at x 95, y 11; the picture keeps its shape inside it
    // (an empty deck shows a 48 pt tile there)
    let art = Rect::from_min_size(pos2(sx(main, 95.0), my + 11.0), vec2(48.0, 48.0));
    match &track {
        Some(t) => {
            if let Some(tex) = crate::ui_browser::artwork_texture(app, &t.artwork) {
                p.image(tex.id(), fit_rect(art, tex.size_vec2()), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                p.rect_filled(art, 0.0, gy(0x141414));
                draw_note_icon(&p, art);
            }
        }
        None => {
            // Empty deck (reference 01): a dark tile with a grey disc and hub.
            let tile = Rect::from_min_size(pos2(sx(main, 95.0), my + 11.0), vec2(48.0, 48.0));
            p.rect_filled(tile, 0.0, gy(0x0F0F0F));
            // The disc (radius 21.5) and its hub (radius 5.5) have soft edges.
            for (rad, col) in [(21.5, 0x141414), (21.0, 0x181818), (20.5, 0x1D1D1D), (19.5, 0x1E1E1E), (6.5, 0x191919), (6.0, 0x141414), (5.5, 0x0F0F0F)] {
                // (Light: `TitleView::lookAndFeelChanged` recolours rekordbox's disc to ids
                // 0x01000700, #F0F0F0, and 0x01000900, #FFFFFF, for the hub.)
                p.circle_filled(tile.center(), rad, Tc(hex(col), hex(if rad > 10.0 { 0xF0F0F0 } else { 0xFFFFFF })).c());
            }
        }
    }

    if loaded {
        let ar = ui.interact(art, ui.id().with("artwork"), Sense::click());
        if tip(ar, app, "Artwork:\nClick it to unload the track.").clicked() {
            app.unload_track();
        }
    }

    // ---------------- overview waveform: track start at x 163.5, end at 1290; cue letters from
    // y 21, baseline at 51.
    let ov = Rect::from_min_max(pos2(sx(main, 163.5), my + 21.0), pos2(sx(main, 1290.0), my + 54.0));
    draw_overview(app, ui, ov, pos);

    // ---------------- control strip: 48 pt tall, from y 208
    let strip = Rect::from_min_max(pos2(left.max.x, my + 208.0), pos2(main.max.x, full.max.y));
    control_strip(app, ui, strip, main);

    // ---------------- right sub panel
    side_panel(app, ui, right, &PANEL_1P);

    // A track dragged from the list and dropped on the player is loaded.
    crate::ui_player2::drop_to_load(app, ui, full);

}

/// The largest rectangle of a picture's proportions centred in `bx`.
pub(crate) fn fit_rect(bx: Rect, size: egui::Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return bx;
    }
    let k = (bx.width() / size.x).min(bx.height() / size.y);
    Rect::from_center_size(bx.center(), size * k)
}

/// Vertical centres of the header row's texts (from the player's top edge).
const HDR_TITLE_Y: f32 = 10.8;
const HDR_TEXT_Y: f32 = 11.2;
const HDR_TIME_Y: f32 = 9.9;

pub(crate) fn split_time(t: f64) -> (String, String) {
    let tenths = ((t * 10.0).floor() as i64).max(0);
    let secs = tenths / 10;
    (format!("{:02}:{:02}", secs / 60, secs % 60), format!("{}", tenths % 10))
}

pub(crate) fn draw_note_icon(p: &egui::Painter, r: Rect) {
    let c = r.center();
    p.circle_filled(c + vec2(-4.0, 8.0), 5.0, OFF);
    p.line_segment([c + vec2(0.5, 8.0), c + vec2(0.5, -12.0)], Stroke::new(2.0_f32, OFF));
    p.line_segment([c + vec2(0.5, -12.0), c + vec2(9.0, -8.0)], Stroke::new(2.0_f32, OFF));
}

/// Colour + height for one waveform column according to the chosen display style.
#[derive(Clone, Copy, Debug)]
pub struct WaveBars {
    layers: [(f32, Color32); 3],
    len: usize,
}

impl WaveBars {
    fn one(height: f32, color: Color32) -> Self {
        WaveBars { layers: [(height, color), (0.0, Color32::TRANSPARENT), (0.0, Color32::TRANSPARENT)], len: 1 }
    }

    fn three(layers: [(f32, Color32); 3]) -> Self {
        WaveBars { layers, len: 3 }
    }

    fn scaled(mut self, scale: f32) -> Self {
        for (height, _) in &mut self.layers[..self.len] {
            *height *= scale;
        }
        self
    }
}

impl IntoIterator for WaveBars {
    type Item = (f32, Color32);
    type IntoIter = std::iter::Take<std::array::IntoIter<Self::Item, 3>>;

    fn into_iter(self) -> Self::IntoIter {
        self.layers.into_iter().take(self.len)
    }
}

pub fn wave_bars(mode: u8, v: [u8; 4]) -> WaveBars {
    let f = |x: u8| x as f32 / 255.0;
    match mode {
        0 => {
            let white = (f(v[3]) / (f(v[0]) + 0.05)).min(1.0);
            let c = Color32::from_rgb((30.0 + 180.0 * white) as u8, (120.0 + 110.0 * white) as u8, 255);
            WaveBars::one(f(v[0]), c)
        }
        1 => {
            let (r, g, b) = (f(v[1]), f(v[2]), f(v[3]) * 1.3);
            let m = r.max(g).max(b).max(0.01);
            let c = Color32::from_rgb((255.0 * r / m) as u8, (255.0 * g / m) as u8, (255.0 * (b / m).min(1.0)) as u8);
            WaveBars::one(f(v[0]), c)
        }
        // 3Band from display levels (track-list previews): blue body as the envelope, then
        // smaller orange and white layers, matching the original's balance of colours.
        _ => WaveBars::three([(f(v[0]), WAVE_LOW), (f(v[2]) * 0.6, WAVE_MID), (f(v[3]) * 0.35, WAVE_HIGH)]),
    }
}

type NativeWaveform = std::sync::Arc<(Vec<[u8; 3]>, Vec<[u8; 3]>)>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct OverviewKey {
    track: Option<i64>,
    wave_ptr: usize,
    wave_len: usize,
    width: usize,
    mode: u8,
    native_ptr: usize,
    native_len: usize,
}

#[derive(Clone, Copy)]
struct OverviewColumn {
    bars: WaveBars,
    played_colors: [Color32; 3],
}

/// One prepared overview per deck. Waveforms are immutable between load/analysis installs;
/// those installs reset this cache even if an allocator reuses a buffer address.
#[derive(Default)]
pub struct OverviewCache {
    key: Option<OverviewKey>,
    columns: Vec<OverviewColumn>,
    // Keep native identity alive while its pointer is part of the key.
    native: Option<NativeWaveform>,
    #[cfg(test)]
    rebuilds: usize,
}

impl OverviewCache {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    fn prepared(&mut self, track: Option<i64>, wave: &Waveform, width: usize, mode: u8, native: Option<&NativeWaveform>) -> &[OverviewColumn] {
        let key = OverviewKey {
            track,
            wave_ptr: wave.frames.as_ptr() as usize,
            wave_len: wave.frames.len(),
            width,
            mode,
            native_ptr: native.map(|w| std::sync::Arc::as_ptr(w) as usize).unwrap_or(0),
            native_len: native.map(|w| w.1.len()).unwrap_or(0),
        };
        if self.key != Some(key) {
            self.columns = prepare_overview(wave, width, mode, native.map(|w| w.1.as_slice()));
            self.key = Some(key);
            self.native = native.cloned();
            #[cfg(test)]
            { self.rebuilds += 1; }
        }
        &self.columns
    }
}

fn prepare_overview(wave: &Waveform, width: usize, mode: u8, native: Option<&[[u8; 3]]>) -> Vec<OverviewColumn> {
    let cols = wave.columns(width);
    let top = cols.iter().map(|c| { let d = Waveform::disp(c); d[0].max(d[1]) }).max().unwrap_or(255).max(1) as f32;
    let norm = (255.0 / top).min(4.0);
    cols.iter().enumerate().map(|(i, c)| {
        let mut d = Waveform::disp(c);
        for v in d.iter_mut() {
            *v = (*v as f32 * norm).min(255.0) as u8;
        }
        let mut bars = if mode >= 2 {
            // Native overview units correspond to 0.25 / 0.115 / 0.085 points per
            // low/mid/high unit in the original 24-point overview area.
            let [lo, mi, hi] = match native {
                Some(values) if !values.is_empty() => values[(i * values.len() / width.max(1)).min(values.len() - 1)],
                _ => crate::export::anlz::three_band_overview(c),
            };
            let (b, o, w) = (lo as f32 * 0.25 / 24.0, mi as f32 * 0.115 / 24.0, hi as f32 * 0.085 / 24.0);
            WaveBars::three([(b + o + w, WAVE_HIGH), (b + o, WAVE_MID), (b, WAVE_LOW)])
        } else {
            wave_bars(mode, d)
        };
        let mut played_colors = [Color32::TRANSPARENT; 3];
        for (index, (height, color)) in bars.layers[..bars.len].iter_mut().enumerate() {
            *height = height.min(1.0) * 24.0;
            played_colors[index] = color.gamma_multiply(0.45);
        }
        OverviewColumn { bars, played_colors }
    }).collect()
}

fn overview_bars(columns: &[OverviewColumn], r: Rect, base: f32, duration: f64, pos: f64, scale: f32) -> crate::ui_kit::Bars {
    let mut bars = crate::ui_kit::Bars::new();
    for (i, column) in columns.iter().enumerate() {
        let x = r.min.x + i as f32 + 0.5;
        let played = (i as f64 / columns.len() as f64) * duration < pos;
        for (index, (height, color)) in column.bars.into_iter().enumerate() {
            let color = if played { column.played_colors[index] } else { color };
            bars.column(x, base, base - height * scale, color);
        }
    }
    bars
}

pub(crate) fn paint_deck_overview(app: &mut App, painter: &egui::Painter, area: Rect, duration: f64, pos: f64) -> bool {
    let Some(wave) = app.deck.wave.as_ref() else { return false };
    let width = area.width().max(1.0) as usize;
    let columns = app.deck.x.overview_cache.prepared(app.deck.track, wave, width, app.prefs.wave_color, app.deck.rb_wave.as_ref());
    overview_bars(columns, area, area.max.y, duration, pos, area.height() / 24.0).paint(painter);
    true
}

#[cfg(test)]
mod waveform_cache_tests {
    use super::*;

    fn wave(len: usize) -> Waveform {
        Waveform { frames: (0..len).map(|i| std::array::from_fn(|k| (i.wrapping_mul(37) + k * 53) as u8)).collect() }
    }

    // Original allocation-based layer preparation, retained as the visual reference.
    fn original_layers(mode: u8, v: [u8; 4]) -> Vec<(f32, Color32)> {
        let f = |x: u8| x as f32 / 255.0;
        match mode {
            0 => {
                let white = (f(v[3]) / (f(v[0]) + 0.05)).min(1.0);
                vec![(f(v[0]), Color32::from_rgb((30.0 + 180.0 * white) as u8, (120.0 + 110.0 * white) as u8, 255))]
            }
            1 => {
                let (r, g, b) = (f(v[1]), f(v[2]), f(v[3]) * 1.3);
                let m = r.max(g).max(b).max(0.01);
                vec![(f(v[0]), Color32::from_rgb((255.0 * r / m) as u8, (255.0 * g / m) as u8, (255.0 * (b / m).min(1.0)) as u8))]
            }
            _ => vec![(f(v[0]), WAVE_LOW), (f(v[2]) * 0.6, WAVE_MID), (f(v[3]) * 0.35, WAVE_HIGH)],
        }
    }

    fn original_overview(wave: &Waveform, width: usize, mode: u8, native: Option<&[[u8; 3]]>, r: Rect, duration: f64, pos: f64) -> egui::Mesh {
        let cols = wave.columns(width);
        let top = cols.iter().map(|c| { let d = Waveform::disp(c); d[0].max(d[1]) }).max().unwrap_or(255).max(1) as f32;
        let norm = (255.0 / top).min(4.0);
        let mut bars = crate::ui_kit::Bars::new();
        let base = r.min.y + 30.0;
        for (i, c) in cols.iter().enumerate() {
            let mut d = Waveform::disp(c);
            for v in d.iter_mut() {
                *v = (*v as f32 * norm).min(255.0) as u8;
            }
            let layers = if mode >= 2 {
                let [lo, mi, hi] = match native {
                    Some(values) if !values.is_empty() => values[(i * values.len() / width.max(1)).min(values.len() - 1)],
                    _ => crate::export::anlz::three_band_overview(c),
                };
                let (b, o, w) = (lo as f32 * 0.25 / 24.0, mi as f32 * 0.115 / 24.0, hi as f32 * 0.085 / 24.0);
                vec![(b + o + w, WAVE_HIGH), (b + o, WAVE_MID), (b, WAVE_LOW)]
            } else {
                original_layers(mode, d)
            };
            let played = (i as f64 / width as f64) * duration < pos;
            for (height, color) in layers {
                let color = if played { color.gamma_multiply(0.45) } else { color };
                bars.column(r.min.x + i as f32 + 0.5, base, base - height.min(1.0) * 24.0, color);
            }
        }
        bars.into_mesh()
    }

    #[test]
    fn stack_layers_keep_exact_counts_order_heights_and_colors() {
        for mode in [0, 1, 2, 5] {
            for v in 0..=255u8 {
                let input = [v, 255 - v, v.rotate_left(1), v.rotate_right(1)];
                let expected = original_layers(mode, input);
                assert_eq!(wave_bars(mode, input).into_iter().collect::<Vec<_>>(), expected);
                let scaled: Vec<_> = expected.iter().map(|(h, c)| (h * DETAIL_FULL_SCALE, *c)).collect();
                assert_eq!(wave_bars(mode, input).scaled(DETAIL_FULL_SCALE).into_iter().collect::<Vec<_>>(), scaled);
                assert_eq!(expected.len(), if mode < 2 { 1 } else { 3 });
            }
        }
    }

    #[test]
    fn cached_overview_mesh_matches_original_at_live_positions() {
        let native = vec![[3, 100, 255], [240, 4, 17], [0, 0, 0]];
        for len in [0, 1, 7, 119] {
            let wave = wave(len);
            for width in [0, 1, 3, 127] {
                for mode in [0, 1, 2, 5] {
                    for values in [None, Some(&[][..]), Some(native.as_slice())] {
                        let prepared = prepare_overview(&wave, width, mode, values);
                        for (x, y, duration, pos) in [(0.0, 0.0, 180.0, -1.0), (10.25, 4.5, 180.0, 0.0), (-3.5, 8.75, 180.0, 90.0), (0.0, 2.0, 180.0, 181.0)] {
                            let r = Rect::from_min_size(pos2(x, y), vec2(width as f32 + 0.75, 33.0));
                            let expected = original_overview(&wave, width, mode, values, r, duration, pos);
                            assert_eq!(overview_bars(&prepared, r, y + 30.0, duration, pos, 1.0).into_mesh(), expected, "len={len} width={width} mode={mode}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn overview_cache_rebuilds_for_each_static_input() {
        let mut cache = OverviewCache::default();
        let mut wave = wave(7);
        let native = std::sync::Arc::new((vec![], vec![[1, 2, 3]; 3]));
        cache.prepared(Some(1), &wave, 10, 2, Some(&native));
        assert_eq!(cache.rebuilds, 1);
        cache.prepared(Some(1), &wave, 10, 2, Some(&native));
        assert_eq!(cache.rebuilds, 1);
        cache.prepared(Some(2), &wave, 10, 2, Some(&native));
        cache.prepared(Some(2), &wave, 11, 2, Some(&native));
        cache.prepared(Some(2), &wave, 11, 1, Some(&native));
        cache.prepared(Some(2), &wave, 11, 1, None);
        assert_eq!(cache.rebuilds, 5);
        let replacement = Waveform { frames: wave.frames.clone() };
        cache.prepared(Some(2), &replacement, 11, 1, None);
        assert_eq!(cache.rebuilds, 6, "same-length replacement is a different source");
        wave.frames.push([255; 8]);
        cache.prepared(Some(2), &wave, 11, 1, None);
        assert_eq!(cache.rebuilds, 7);
        let replacement = std::sync::Arc::new((vec![], vec![[40, 50, 60]; 3]));
        cache.prepared(Some(2), &wave, 11, 2, Some(&native));
        cache.prepared(Some(2), &wave, 11, 2, Some(&replacement));
        assert_eq!(cache.rebuilds, 9, "native replacement with the same length invalidates");
        cache.prepared(Some(2), &wave, 1, 2, Some(&replacement));
        assert_eq!(cache.columns.len(), 1, "only the latest width is retained");
    }

    #[test]
    fn overview_cache_clear_and_track_carry_end_source_lifetimes() {
        let wave = wave(7);
        let native = std::sync::Arc::new((vec![], vec![[1, 2, 3]; 3]));
        let weak = std::sync::Arc::downgrade(&native);
        let mut deck = crate::player_ext::DeckExt::default();
        deck.master_tempo = true;
        deck.overview_cache.prepared(Some(1), &wave, 10, 2, Some(&native));
        drop(native);
        assert!(weak.upgrade().is_some(), "cache pins the native identity against allocator reuse");
        let carried = deck.carried();
        assert!(carried.master_tempo);
        assert!(carried.overview_cache.key.is_none(), "loading another track carries no prepared waveform");
        assert!(deck.overview_cache.key.is_some(), "a parked/backup deck keeps its own cache");
        deck.overview_cache.clear();
        assert!(weak.upgrade().is_none());
        assert!(deck.overview_cache.columns.is_empty());
        // Analysis installs explicitly clear first: even address/length reuse must rebuild.
        let mut same_buffer = wave;
        deck.overview_cache.prepared(Some(1), &same_buffer, 10, 2, None);
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 33.0));
        let old = overview_bars(&deck.overview_cache.columns, r, 30.0, 1.0, 0.0, 1.0).into_mesh();
        same_buffer.frames.fill([0; 8]);
        deck.overview_cache.clear();
        let new = deck.overview_cache.prepared(Some(1), &same_buffer, 10, 2, None);
        assert_ne!(overview_bars(new, r, 30.0, 1.0, 0.0, 1.0).into_mesh(), old);
        assert_eq!(deck.overview_cache.rebuilds, 1);
    }

    /// Opt-in, synthetic and device-free: compare preparation plus Bars mesh generation.
    /// Run with `cargo test --release cpu_overview_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cpu_overview_cost() {
        fn measure(mut frame: impl FnMut(usize)) -> f64 {
            let mut times = vec![];
            for _ in 0..5 {
                let start = std::time::Instant::now();
                for i in 0..300 { frame(i); }
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            times.sort_by(f64::total_cmp);
            times[2]
        }
        for (seconds, width, mode, has_native) in [(180, 1126, 2, false), (180, 1126, 2, true), (600, 1600, 2, false), (180, 1126, 0, false), (180, 1126, 1, false)] {
            let wave = wave(seconds * 150);
            let native = std::sync::Arc::new((vec![], vec![[30, 100, 200], [200, 40, 10], [0, 0, 0]]));
            let native = has_native.then_some(&native);
            let values = native.map(|w| w.1.as_slice());
            let r = Rect::from_min_size(pos2(163.5, 21.0), vec2(width as f32, 33.0));
            let duration = seconds as f64;
            let mut cache = OverviewCache::default();
            for pos in [0.0, duration / 3.0, duration] {
                assert_eq!(overview_bars(cache.prepared(Some(1), &wave, width, mode, native), r, 51.0, duration, pos, 1.0).into_mesh(), original_overview(&wave, width, mode, values, r, duration, pos));
            }
            let legacy = measure(|i| { std::hint::black_box(original_overview(&wave, width, mode, values, r, duration, i as f64 / 300.0 * duration)); });
            let cold = measure(|i| { cache.clear(); std::hint::black_box(overview_bars(cache.prepared(Some(1), &wave, width, mode, native), r, 51.0, duration, i as f64 / 300.0 * duration, 1.0).into_mesh()); });
            let warm = measure(|i| { std::hint::black_box(overview_bars(cache.prepared(Some(1), &wave, width, mode, native), r, 51.0, duration, i as f64 / 300.0 * duration, 1.0).into_mesh()); });
            eprintln!("cpu_overview_cost seconds={seconds} width={width} mode={mode} native={has_native} frames=300 median_of=5 legacy_full_mesh_ms={legacy:.3} cold_full_mesh_ms={cold:.3} warm_full_mesh_ms={warm:.3}");
        }
    }
}

pub(crate) fn draw_overview(app: &mut App, ui: &mut Ui, r: Rect, pos: f64) {
    let p = ui.painter().clone();
    let dur = app.deck.duration.max(0.001);
    // Measured from the original: baseline 30 pt below the top (the top 11 pt hold the hot cue
    // letters), bars up to 24 pt tall, a 3 pt progress frame from 3 pt under the baseline and
    // white minute ticks from 8 to 10 pt under it.
    let base = r.min.y + 30.0;
    // The overview always shows the whole mix, as in rekordbox: only the enlarged waveform
    // follows the parts that sound.
    let area = Rect::from_min_max(pos2(r.min.x, base - 24.0), pos2(r.max.x, base));
    if !paint_deck_overview(app, &p, area, dur, pos) && app.deck.track.is_some() {
        p.text(r.center(), Align2::CENTER_CENTER, "Analyzing…", FontId::proportional(11.0), DIM.c());
    }
    // Show overall separation progress underneath the overview. The parts become playable
    // together once the completed separation has been loaded.
    if let (Some((sid, pr)), Some(tid)) = (app.stem.separating, app.deck.track) {
        if sid == tid && pr > 0.0 {
            p.rect_filled(Rect::from_min_max(pos2(r.min.x, base + 1.5), pos2(r.min.x + r.width() * pr, base + 2.5)), 0.0, hex(0x077DE1));
        }
    }
    // Vocal positions (rekordbox's vocal detection): a band along the top, brighter where the
    // vocal level is higher.
    if let (true, Some(v)) = (app.prefs.show_vocal, &app.deck.vocal) {
        let (step, levels) = (v.0, &v.1);
        let w = r.width().max(1.0) as usize;
        let mut bars = crate::ui_kit::Bars::new();
        for i in 0..w {
            let t0 = i as f64 / w as f64 * dur;
            let t1 = (i + 1) as f64 / w as f64 * dur;
            let (a, b) = ((t0 / step) as usize, ((t1 / step) as usize).max((t0 / step) as usize + 1));
            let lvl = levels.get(a.min(levels.len())..b.min(levels.len())).and_then(|s| s.iter().max().copied()).unwrap_or(0);
            if lvl > 0 {
                let x = r.min.x + i as f32 + 0.5;
                let col = hex(0x4FC3D9).gamma_multiply(0.35 + 0.65 * (lvl.min(4) as f32 / 4.0));
                bars.column(x, r.min.y + 1.0, r.min.y + 3.5, col);
            }
        }
        bars.paint(&p);
    }
    // Phrase bar (as rekordbox shows phrase analysis): one coloured block per phrase.
    if let (Some(st), false, true) = (&app.deck.phrases, app.deck.beats.is_empty(), app.prefs.show_phrase_bar) {
        let y0 = base + 12.5;
        let beat_t = |b: u16| app.deck.beats.get((b.max(1) - 1) as usize).map(|x| x.time).unwrap_or(dur);
        for (k, ph) in st.phrases.iter().enumerate() {
            let t0 = beat_t(ph.beat);
            let t1 = st.phrases.get(k + 1).map(|n| beat_t(n.beat)).unwrap_or_else(|| beat_t(st.end_beat));
            let x0 = r.min.x + (t0 / dur) as f32 * r.width();
            let x1 = r.min.x + (t1 / dur) as f32 * r.width();
            let col = hex(crate::phrase::color(st.mood, ph.kind));
            p.rect_filled(Rect::from_min_max(pos2(x0 + 0.5, y0), pos2((x1 - 0.5).max(x0 + 1.0), y0 + 3.0)), 0.0, col);
        }
    }
    // progress frame and minute ticks (nothing at all on an empty deck, as in reference 01)
    if app.deck.track.is_some() {
        // The frame: #6B6B6B outside, #515151 inside, half a point each, around a 1 pt gap.
        let fr = Rect::from_min_max(pos2(r.min.x - 1.0, base + 3.0), pos2(r.max.x, base + 6.0));
        p.rect_filled(fr, 0.0, gy(0x6B6B6B));
        p.rect_filled(fr.shrink(0.5), 0.0, gy(0x515151));
        p.rect_filled(fr.shrink(1.0), 0.0, gy(0x020202));
        let minute = 60.0 / dur * r.width() as f64;
        let mut k = 1.0;
        while k * minute < r.width() as f64 + 0.5 {
            let x = (r.min.x + (k * minute) as f32 - 0.5).round();
            p.rect_filled(Rect::from_min_max(pos2(x, base + 8.0), pos2(x + 1.0, base + 10.0)), 0.0, WHITE);
            k += 1.0;
        }
        let x_of = |t: f64| r.min.x + (t / dur) as f32 * r.width();
        // loops
        for c in &app.deck.cues {
            if let Some(e) = c.end.filter(|e| *e > c.start) {
                p.rect_filled(Rect::from_min_max(pos2(x_of(c.start), r.min.y), pos2(x_of(e).max(x_of(c.start) + 1.0), base)), 0.0, LOOP.gamma_multiply(0.25));
            }
        }
        // memory cues (below) and hot cues (above)
        for c in &app.deck.cues {
            let x = x_of(c.start);
            if c.kind == CueKind::Memory {
                let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
                tri_up(&p, pos2(x, base + 0.5), 5.0, col);
            } else {
                let col = hotcue_color(c.color);
                // A 12 × 11 pt box around the cue line, with a black letter (reference 02).
                p.line_segment([pos2(x, r.min.y), pos2(x, base)], Stroke::new(1.0_f32, col));
                let lr = Rect::from_min_size(pos2(x - 5.5, r.min.y), vec2(12.0, 11.0));
                p.rect_filled(lr, 0.0, col);
                p.text(lr.center() + vec2(-0.25, 0.0), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), FontId::proportional(10.0), Color32::BLACK);
            }
        }
        // playhead: a 1 pt white line from under the letters to 12 pt under the baseline
        let px = x_of(pos);
        p.rect_filled(Rect::from_min_max(pos2(px - 0.5, r.min.y + 11.0), pos2(px + 0.5, base + 12.0)), 0.0, WHITE);
        // cue point: the current-cue triangle, 9.5 pt tall, its apex just above the baseline
        crate::icons_player::cue_tri(&p, pos2(x_of(app.deck.cue_point), base - 0.5), 9.5, CUE_MARK);
    }
    let resp = ui.interact(r.expand2(vec2(0.0, 4.0)), ui.id().with("overview"), Sense::click_and_drag());
    if (resp.clicked() || resp.dragged()) && app.deck.audio.is_some() {
        if let Some(pp) = resp.interact_pointer_pos() {
            // Performance: needle search by rekordbox's rules (Needle Lock, 400 steps).
            if app.needle_search(((pp.x - r.min.x) / r.width()) as f64) {
                return;
            }
            let t = ((pp.x - r.min.x) / r.width()).clamp(0.0, 1.0) as f64 * dur;
            app.seek(t);
            if !app.playing() {
                app.deck.cue_point = app.quantized(t);
                app.seek(app.deck.cue_point);
            }
        }
    }
}

pub(crate) fn tri_up(p: &egui::Painter, tip: Pos2, s: f32, c: Color32) {
    p.add(Shape::convex_polygon(vec![tip, tip + vec2(s, s * 1.2), tip + vec2(-s, s * 1.2)], c, Stroke::NONE));
}

/// How the enlarged waveform is drawn.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum DetailMode {
    /// 1 Player: the waveform mirrored about the middle, beat marks at both edges.
    Full,
    /// 1 Player with the GRID tab open: as Full, with blue beat lines and red bar lines.
    GridEdit,
    /// Deck A of the two-deck layouts: the upper half of the waveform standing on the bottom
    /// edge of `r`, beat marks along the top.
    HalfUp,
    /// Deck B: the same hanging from the top edge, beat marks along the bottom.
    HalfDown,
}

/// The enlarged waveform in `r`, with the playhead `cx_off` pt right of the middle of `r`
/// (rekordbox's 1 Player layout has it 2.5 pt right of the middle).
pub(crate) fn draw_detail_at(app: &mut App, ui: &mut Ui, r: Rect, pos: f64, cx_off: f32, mode: DetailMode) {
    // `r` is the beat/waveform area; the 19 pt row above it holds the bar.beat readout and the
    // hot cue letters (measured on reference 02).
    let half_mode = matches!(mode, DetailMode::HalfUp | DetailMode::HalfDown);
    let flip = mode == DetailMode::HalfDown;
    let grid_edit = mode == DetailMode::GridEdit;
    // In the half modes everything is laid out for deck A and mirrored for deck B.
    let fy = |y: f32| if flip { r.min.y + r.max.y - y } else { y };
    let dir = if flip { -1.0f32 } else { 1.0 };
    let p = ui.painter_at(match mode {
        DetailMode::HalfUp => Rect::from_min_max(r.min - vec2(0.0, 27.0), r.max + vec2(0.0, 0.5)),
        DetailMode::HalfDown => Rect::from_min_max(r.min - vec2(0.0, 0.5), r.max + vec2(0.0, 27.0)),
        _ => Rect::from_min_max(r.min - vec2(0.0, 20.0), r.max + vec2(0.0, 6.0)),
    });
    let span = crate::ui_perf_layouts::span(app).unwrap_or(ZOOMS[app.prefs.zoom.min(ZOOMS.len() - 1)]);
    // Absolute picks follow the painted markers. Relative transport commands below use
    // the selected zoom and the audio clock; grid editing always shows that exact scale.
    let transport_px_per_s = r.width() as f64 / span;
    let (pos, span) = if grid_edit { (app.pos(), span) } else { (pos, crate::ui_feel::zoom_span(ui, r, span)) };
    let px_per_s = r.width() as f64 / span;
    let cx = r.center().x + cx_off;
    let t_of = |x: f32| pos + (x - cx) as f64 / px_per_s;
    app.deck.detail_map = Some((r, pos, px_per_s));
    let x_of = |t: f64| cx + ((t - pos) * px_per_s) as f32;
    let mid = r.center().y + 0.5;
    let half = r.height() / 2.0 - 9.0;
    let dur = app.deck.duration;
    // loops
    if app.loop_active().is_none() {
        app.deck.x.loop_adjust = 0;
        app.deck.x.adjust_drag = None;
    }
    if let Some((a, b)) = app.loop_active() {
        p.rect_filled(Rect::from_min_max(pos2(x_of(a), r.min.y), pos2(x_of(b), r.max.y)), 0.0, LOOP.gamma_multiply(0.22));
        // Loop adjust: the edge that ←/→ and dragging move is drawn bright, with its name.
        let adj = app.deck.x.loop_adjust;
        if adj != 0 {
            let x = x_of(if adj == 1 { a } else { b });
            p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(2.0_f32, LOOP));
            let tag = Rect::from_min_size(pos2(if adj == 1 { x } else { x - 26.0 }, r.max.y - 13.0), vec2(26.0, 13.0));
            p.rect_filled(tag, 0.0, LOOP);
            p.text(tag.center(), Align2::CENTER_CENTER, if adj == 1 { "IN" } else { "OUT" }, bold(8.5), Color32::BLACK);
        }
    }
    for c in &app.deck.cues {
        if let Some(e) = c.end.filter(|e| *e > c.start) {
            p.rect_filled(Rect::from_two_pos(pos2(x_of(c.start), fy(r.max.y - 6.0)), pos2(x_of(e), fy(r.max.y))), 0.0, LOOP.gamma_multiply(0.6));
        }
    }
    // Beat lines run behind the waveform in dim grey (bars get a white line on top, below).
    if !grid_edit {
        let (tl, tr) = (t_of(r.min.x), t_of(r.max.x));
        let i0 = app.deck.beats.partition_point(|b| b.time < tl);
        let (y0, y1) = if half_mode { (fy(r.min.y + 4.0), fy(r.max.y)) } else { (r.min.y + 5.0, r.max.y - 5.0) };
        for b in app.deck.beats[i0..].iter().take_while(|b| b.time <= tr) {
            // In the half modes a bar has no dim line (only its white one, with the waveform).
            if half_mode && b.beat == 1 {
                continue;
            }
            let x = x_of(b.time);
            p.line_segment([pos2(x, y0), pos2(x, y1)], Stroke::new(1.0_f32, hex(0x4C4C4C)));
        }
    }
    let stem_view = app.stem_wave_shown();
    if app.stem_wave_preparing() {
        // Empty while the waveform of the sounding parts is worked out.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    } else if let Some(w) = app.deck.x.stem_wave.as_ref().filter(|_| stem_view).map(|s| &s.1).or(app.deck.wave.as_ref()) {
        let fps = WAVE_FPS;
        let n = w.frames.len();
        let mut x = r.min.x;
        let mut cols = crate::ui_kit::Bars::new();
        while x < r.max.x {
            let t0 = t_of(x);
            let t1 = t_of(x + 1.0);
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
                    let bars = if app.prefs.wave_color >= 2 {
                        // 3Band detail: each band centred at its own height, low, then mid, then
                        // high drawn over it (the original's layering), from the fitted model.
                        let [lo, mi, hi] = match &app.deck.rb_wave {
                            // rekordbox's own detail values (150 per second), when available.
                            Some(rw) if !rw.0.is_empty() && !stem_view => {
                                let ia = ((t0.max(0.0) * 150.0) as usize).min(rw.0.len() - 1);
                                let ib = ((t1.max(0.0) * 150.0).ceil() as usize).clamp(ia + 1, rw.0.len());
                                rw.0[ia..ib].iter().fold([0u8; 3], |acc, v| [acc[0].max(v[0]), acc[1].max(v[1]), acc[2].max(v[2])])
                            }
                            _ => crate::export::anlz::three_band_detail(&m),
                        };
                        // Measured against rekordbox 7.2.19 (its own PWV7 values for a track vs
                        // its enlarged waveform, export/01 of the reference set): 0.39 pt of
                        // half-height per unit in a 147 pt tall waveform, so full scale (127)
                        // reaches 0.68 of half the area.
                        let s = |v: u8| (v as f32 / 127.0).min(1.0) * DETAIL_FULL_SCALE;
                        WaveBars::three([(s(lo), WAVE_LOW), (s(mi), WAVE_MID), (s(hi), WAVE_HIGH)])
                    } else {
                        // BLUE and RGB use the same share of the area as 3Band.
                        wave_bars(app.prefs.wave_color, Waveform::disp(&m)).scaled(DETAIL_FULL_SCALE)
                    };
                    // Heights are a share of half the waveform area (not of the area less
                    // its margins), as measured.
                    let half = r.height() / 2.0;
                    for (h, col) in bars {
                        let hh = h * half;
                        if half_mode {
                            // Half of the waveform, from this deck's edge of the shared area.
                            let hh = h * r.height();
                            cols.column(x + 0.5, fy(r.max.y), fy(r.max.y - hh), col);
                            continue;
                        }
                        // Preferences > View > Color: Half Waveform draws the upper half only.
                        if app.prefs.wave_half {
                            cols.column(x + 0.5, mid + half - 2.0 * hh, mid + half, col);
                            continue;
                        }
                        cols.column(x + 0.5, mid - hh, mid + hh, col);
                    }
                }
            }
            x += 1.0;
        }
        cols.paint(&p);
    } else if app.deck.track.is_some() {
        p.text(r.center() + vec2(60.0, 0.0), Align2::LEFT_CENTER, if app.deck.loading.is_some() { "Loading…" } else { "Analyzing…" }, FontId::proportional(12.0), fixed(0x969696).c());
    }
    // Preferences > View: vocal positions and phrases on the enlarged waveform. Vocals are a
    // thin band just inside the beat marks; phrases a band along the other edge, each with
    // its type in a tag at its start (always, or under the pointer).
    if let (true, Some(v)) = (app.prefs.vocal_enlarged, &app.deck.vocal) {
        let (step, levels) = (v.0, &v.1);
        let (y0, y1) = (fy(r.min.y + 5.0), fy(r.min.y + 7.0));
        let mut x = r.min.x;
        let mut band = crate::ui_kit::Bars::new();
        while x < r.max.x {
            let t0 = t_of(x);
            if t0 >= 0.0 && t0 <= dur {
                let a = (t0 / step) as usize;
                let b = ((t_of(x + 1.0) / step) as usize).max(a + 1);
                let lvl = levels.get(a.min(levels.len())..b.min(levels.len())).and_then(|s| s.iter().max().copied()).unwrap_or(0);
                if lvl > 0 {
                    band.rect(Rect::from_two_pos(pos2(x, y0), pos2(x + 1.0, y1)), hex(0x4FC3D9).gamma_multiply(0.35 + 0.65 * (lvl.min(4) as f32 / 4.0)));
                }
            }
            x += 1.0;
        }
        band.paint(&p);
    }
    if let (true, Some(st), false) = (app.prefs.phrase_enlarged, &app.deck.phrases, app.deck.beats.is_empty()) {
        let beat_t = |b: u16| app.deck.beats.get((b.max(1) - 1) as usize).map(|x| x.time).unwrap_or(dur);
        let hover = ui.ctx().pointer_latest_pos().filter(|pp| r.expand(8.0).contains(*pp));
        let (y0, y1) = if half_mode { (fy(r.max.y - 3.0), fy(r.max.y)) } else { (r.max.y + 2.0, r.max.y + 5.0) };
        for (k, ph) in st.phrases.iter().enumerate() {
            let t0 = beat_t(ph.beat);
            let t1 = st.phrases.get(k + 1).map(|n| beat_t(n.beat)).unwrap_or_else(|| beat_t(st.end_beat));
            let (x0, x1) = (x_of(t0), x_of(t1));
            if x1 < r.min.x || x0 > r.max.x {
                continue;
            }
            let col = hex(crate::phrase::color(st.mood, ph.kind));
            p.rect_filled(Rect::from_two_pos(pos2(x0.max(r.min.x) + 0.5, y0), pos2((x1.min(r.max.x) - 0.5).max(x0 + 1.0), y1)), 0.0, col);
            let hovered = hover.map(|pp| pp.x >= x0 && pp.x < x1).unwrap_or(false);
            if (app.prefs.phrase_types_always || hovered) && x0 >= r.min.x - 60.0 {
                let name = crate::phrase::label(st.mood, ph.kind);
                let tag = Rect::from_min_size(pos2(x0 + 1.0, if half_mode { fy(r.max.y - 9.0) - if flip { 0.0 } else { 5.5 } } else { r.max.y - 16.0 }), vec2(name.len() as f32 * 5.2 + 7.0, 11.0));
                p.rect_filled(tag, 1.0, col);
                p.text(tag.center(), Align2::CENTER_CENTER, name, FontId::proportional(8.5), Color32::BLACK);
            }
        }
    }
    // beat grid
    let tl = t_of(r.min.x);
    let tr = t_of(r.max.x);
    let i0 = app.deck.beats.partition_point(|b| b.time < tl);
    for b in app.deck.beats[i0..].iter().take_while(|b| b.time <= tr) {
        // Lines sit on whole points so that they stay 1 pt wide.
        let x = (x_of(b.time) - 0.5).round() + 0.5;
        if half_mode {
            // Marks along this deck's outer edge only; the lines run to the shared middle.
            if b.beat == 1 {
                crate::icons_player::bar_cap(&p, x, fy(r.min.y), dir, BAR_RED);
                if app.deck.wave.is_some() {
                    p.rect_filled(Rect::from_two_pos(pos2(x - 0.5, fy(r.min.y + 4.0)), pos2(x + 0.5, fy(r.max.y))), 0.0, ON_ACCENT);
                }
            } else {
                crate::icons_player::beat_cap(&p, x, fy(r.min.y), dir, hex(0x7C7C7C));
            }
            continue;
        }
        if grid_edit {
            // GRID: every beat is a line to drag the eye along: light blue, red on bars, with
            // a dark edge, between the marks at both ends.
            let (core, edge) = if b.beat == 1 { (hex(0xD43024), hex(0x450F0B)) } else { (hex(0x52B1E1), hex(0x04263A)) };
            let line = Rect::from_min_max(pos2(x - 0.5, r.min.y + 4.0), pos2(x + 0.5, r.max.y - 5.0));
            p.rect_filled(line.expand2(vec2(0.5, 0.0)), 0.0, edge);
            p.rect_filled(line, 0.0, core);
            if b.beat == 1 {
                crate::icons_player::bar_cap(&p, x, r.min.y - 1.0, 1.0, BAR_RED);
                crate::icons_player::bar_cap(&p, x, r.max.y, -1.0, BAR_RED);
            } else {
                crate::icons_player::beat_cap(&p, x, r.min.y - 1.0, 1.0, hex(0x7C7C7C));
                crate::icons_player::beat_cap(&p, x, r.max.y, -1.0, hex(0x7C7C7C));
            }
            continue;
        }
        if b.beat == 1 {
            // Bar: red 5 × 4 pt triangles at both ends, a white line between them.
            crate::icons_player::bar_cap(&p, x, r.min.y, 1.0, BAR_RED);
            crate::icons_player::bar_cap(&p, x, r.max.y, -1.0, BAR_RED);
            p.rect_filled(Rect::from_min_max(pos2(x - 0.5, r.min.y + 5.0), pos2(x + 0.5, r.max.y - 5.0)), 0.0, ON_ACCENT);
        } else {
            // Beat: a 3 pt grey wedge with a 4 pt stem at both ends.
            crate::icons_player::beat_cap(&p, x, r.min.y, 1.0, hex(0x7C7C7C));
            crate::icons_player::beat_cap(&p, x, r.max.y, -1.0, hex(0x7C7C7C));
        }
    }
    if app.deck.track.is_some() {
        // bar.beat readout, ending 2 pt left of the playhead (hot cue letters cover its end)
        let mem: Vec<f64> = if app.prefs.beat_count_mode == 0 { vec![] } else { app.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).map(|c| c.start).collect() };
        let label = crate::player_ext::beat_count_label(&app.deck.beats, &mem, pos, app.prefs.beat_count_mode);
        let ly = if half_mode { fy(r.min.y + 6.5) } else { r.min.y - 10.2 };
        p.text(pos2(cx - 2.0, ly), Align2::RIGHT_CENTER, label, bold(14.0), BEAT_BLUE.c());
    }
    // cues
    for c in &app.deck.cues {
        let x = x_of(c.start);
        if x < r.min.x - 20.0 || x > r.max.x + 20.0 {
            continue;
        }
        if c.kind == CueKind::Hot {
            let col = hotcue_color(c.color);
            // An 11 pt letter box centred on the cue, in the row above the beats, with a black
            // letter (no comment text: the original shows none there).
            p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.0_f32, col));
            let lr = if half_mode { Rect::from_center_size(pos2(x, fy(r.min.y - 6.0)), vec2(11.0, 11.0)) } else { Rect::from_min_size(pos2(x - 5.5, r.min.y - 10.5), vec2(11.0, 11.0)) };
            p.rect_filled(lr, 0.0, col);
            p.text(lr.center(), Align2::CENTER_CENTER, ((b'A' + c.slot as u8) as char).to_string(), FontId::proportional(10.0), Color32::BLACK);
        } else {
            let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
            if half_mode {
                let tip = pos2(x, fy(r.max.y - 7.2));
                p.add(Shape::convex_polygon(vec![tip, pos2(x + 6.0, fy(r.max.y)), pos2(x - 6.0, fy(r.max.y))], col, Stroke::NONE));
            } else {
                tri_up(&p, pos2(x, r.max.y - 9.0), 6.0, col);
            }
        }
    }
    if app.deck.track.is_some() && half_mode {
        // current cue: a 10 pt triangle standing on the waveform's base line
        let x = x_of(app.deck.cue_point);
        p.add(Shape::convex_polygon(vec![pos2(x, fy(r.max.y - 10.0)), pos2(x + 6.3, fy(r.max.y)), pos2(x - 6.3, fy(r.max.y))], CUE_MARK, Stroke::NONE));
        // playhead: from just outside the beat marks to the base line
        let ph = Rect::from_two_pos(pos2(cx - 0.5, fy(r.min.y - 2.0)), pos2(cx + 0.5, fy(r.max.y)));
        p.rect_filled(ph.expand2(vec2(1.0, 0.0)), 0.0, Color32::from_black_alpha(128));
        p.rect_filled(ph, 0.0, hex(0xE0E0E0));
    } else if app.deck.track.is_some() {
        // current cue: an 11.5 pt triangle whose base is 4.5 pt under the beats
        crate::icons_player::cue_tri(&p, pos2(x_of(app.deck.cue_point), r.max.y - 7.0), 11.5, CUE_MARK);
        // playhead: a 1 pt #E0E0E0 line with a 1 pt half-black edge on both sides
        let ph = Rect::from_min_max(pos2(cx - 0.5, r.min.y - 18.0), pos2(cx + 0.5, r.max.y + 4.0));
        p.rect_filled(ph.expand2(vec2(1.0, 0.0)), 0.0, Color32::from_black_alpha(128));
        p.rect_filled(ph, 0.0, hex(0xE0E0E0));
    }
    // interaction: drag a cue marker to move it, drag elsewhere to scrub, scroll to move,
    // ctrl/cmd + scroll to zoom
    let hit_rect = match mode {
        DetailMode::HalfUp => Rect::from_min_max(r.min - vec2(0.0, 27.0), r.max),
        DetailMode::HalfDown => Rect::from_min_max(r.min, r.max + vec2(0.0, 27.0)),
        _ => Rect::from_min_max(r.min - vec2(0.0, 19.0), r.max),
    };
    let resp = ui.interact(hit_rect, ui.id().with("detail"), Sense::click_and_drag());
    // Markers are found as they are laid out for deck A (deck B's are mirrored).
    let cue_marker_hit = |cues: &[Cue], pp: Pos2, r: Rect, x_of: &dyn Fn(f64) -> f32| cue_marker_hit(cues, pos2(pp.x, fy(pp.y)), r, x_of);
    let dur = app.deck.duration;
    // Right-click menu, acting at the clicked position. With "Click on the waveform for PLAY
    // and CUE" on, a left click is PLAY/PAUSE and the right button is CUE (held: cue preview);
    // the menu then opens with Option (Alt) + right-click, or from the ≡ menu (at the playhead).
    let click_mode = app.prefs.wave_click_play_cue;
    let (alt, sec_pressed, sec_down) = ui.input(|i| (i.modifiers.alt, i.pointer.button_pressed(egui::PointerButton::Secondary), i.pointer.button_down(egui::PointerButton::Secondary)));
    if app.deck.audio.is_some() {
        if resp.hovered() && sec_pressed {
            if click_mode && !alt {
                app.deck.x.wave_cue_held = true;
                app.cue_down();
            } else if let Some(pp) = resp.hover_pos() {
                app.deck.x.menu_time = Some(t_of(pp.x));
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
            resp.context_menu(|ui| {
                let t = app.deck.x.menu_time.unwrap_or_else(|| app.pos());
                crate::ui_player2::wave_menu(app, ui, t);
            });
        }
    }
    if app.deck.audio.is_some() {
        if app.deck.cue_drag.is_none() {
            if let Some(hp) = resp.hover_pos() {
                if cue_marker_hit(&app.deck.cues, hp, r, &x_of).is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
        }
        if resp.drag_started() {
            // Where the button went down: a drag is only recognised after the pointer has
            // moved a few pixels, and the marker under the press is the one being dragged.
            let pp = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
            let adj = app.deck.x.loop_adjust;
            match pp.and_then(|pp| cue_marker_hit(&app.deck.cues, pp, r, &x_of).map(|id| (id, pp))) {
                // Loop adjust: the drag moves the loop edge.
                _ if adj != 0 && app.loop_active().is_some() => {
                    let (a, b) = app.loop_active().unwrap_or((0.0, 0.0));
                    app.deck.x.adjust_drag = Some((pp.map(|p| p.x).unwrap_or(cx), if adj == 1 { a } else { b }));
                }
                Some((id, pp)) => {
                    let start = app.deck.cues.iter().find(|c| c.id == id).map(|c| c.start).unwrap_or(0.0);
                    app.deck.cue_drag = Some((id, app.deck.cues.clone(), t_of(pp.x) - start));
                }
                None => app.deck.drag_origin = Some((pp.map(|p| p.x).unwrap_or(cx), app.pos())),
            }
        }
        if let Some((id, _, grab)) = app.deck.cue_drag.clone() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.commit_cue_drag(true);
            } else if resp.dragged() {
                if let Some(pp) = resp.interact_pointer_pos() {
                    let t = app.quantized(t_of(pp.x) - grab);
                    if let Some(c) = app.deck.cues.iter_mut().find(|c| c.id == id) {
                        *c = moved_cue(c, t, dur);
                    }
                }
            }
            if resp.drag_stopped() {
                app.commit_cue_drag(false);
            }
        } else if let Some((x0, t0)) = app.deck.x.adjust_drag {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            if let (true, Some(pp)) = (resp.dragged(), resp.interact_pointer_pos()) {
                app.loop_adjust_to(t0 + (pp.x - x0) as f64 / transport_px_per_s);
            }
            if resp.drag_stopped() || !resp.dragged() {
                app.deck.x.adjust_drag = None;
            }
        } else if resp.dragged() {
            if let (Some((x0, p0)), Some(pp)) = (app.deck.drag_origin, resp.interact_pointer_pos()) {
                let t = p0 - (pp.x - x0) as f64 / transport_px_per_s;
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
                if scroll.y > 0.0 && app.prefs.zoom > 0 {
                    app.prefs.zoom -= 1;
                } else if scroll.y < 0.0 && app.prefs.zoom + 1 < ZOOMS.len() {
                    app.prefs.zoom += 1;
                }
            } else if scroll.x.abs() > 0.5 || scroll.y.abs() > 0.5 {
                let d = if scroll.x.abs() > scroll.y.abs() { scroll.x } else { scroll.y };
                let transport = app.pos();
                let t = transport - d as f64 / transport_px_per_s;
                app.deck.x.scrub += t.clamp(0.0, dur.max(0.0)) - transport;
                app.seek(t);
            }
        }
    }
}

/// The cue whose marker is under `p` on the zoomed waveform `r`: a hot cue by its letter box
/// in the row above the beats, a memory cue by its triangle along the bottom (the waveform body scrubs).
pub fn cue_marker_hit(cues: &[Cue], p: Pos2, r: Rect, x_of: impl Fn(f64) -> f32) -> Option<i64> {
    let mut best: Option<(f32, i64)> = None;
    for c in cues {
        let x = x_of(c.start);
        let (hit, d) = match c.kind {
            CueKind::Hot => (p.y >= r.min.y - 13.0 && p.y <= r.min.y && (p.x - x).abs() <= 6.0, (p.x - x).abs()),
            _ => (p.y >= r.max.y - 18.0 && p.y <= r.max.y && (p.x - x).abs() <= 7.0, (p.x - x).abs()),
        };
        if hit && best.map(|b| d < b.0).unwrap_or(true) {
            best = Some((d, c.id));
        }
    }
    best.map(|b| b.1)
}

/// `c` moved to start at `t`: a loop keeps its length, and the cue stays inside the track.
pub fn moved_cue(c: &Cue, t: f64, duration: f64) -> Cue {
    let len = c.end.map(|e| e - c.start).filter(|l| *l > 0.0);
    let max = if duration > 0.0 { (duration - len.unwrap_or(0.0)).max(0.0) } else { f64::MAX };
    let start = t.clamp(0.0, max);
    Cue { start, end: match len { Some(l) => Some(start + l), None => c.end.map(|_| start) }, ..c.clone() }
}

fn control_strip(app: &mut App, ui: &mut Ui, s: Rect, main: Rect) {
    let p = ui.painter().clone();
    // The strip (#1E1E1E) runs from the tabs to the side panel; the tabs are 80 pt wide from
    // 3 pt right of the transport column: CUE/LOOP 24 pt tall, GRID below it.
    p.rect_filled(Rect::from_min_max(pos2(s.min.x + 83.0, s.min.y), pos2(main.max.x, s.max.y)), 0.0, BG2);
    let tab1 = Rect::from_min_max(pos2(s.min.x + 3.0, s.min.y), pos2(s.min.x + 83.0, s.min.y + 24.0));
    let tab2 = Rect::from_min_max(pos2(s.min.x + 3.0, s.min.y + 24.0), pos2(s.min.x + 83.0, s.max.y));
    if tab_button(ui, tab1, "tab_cue", "CUE/LOOP", app.deck.ctrl_tab == 0).clicked() {
        app.deck.ctrl_tab = 0;
    }
    if tab_button(ui, tab2, "tab_grid", "GRID", app.deck.ctrl_tab == 1).clicked() {
        app.deck.ctrl_tab = 1;
    }
    // Every control is centred 24 pt below the strip's top; buttons are 20 pt tall.
    let y = s.min.y + 24.0;
    let at = |x: f32, w: f32| Rect::from_center_size(pos2(sx(main, x), y), vec2(w, 20.0));
    if app.deck.ctrl_tab == 0 {
        // hot cue pads: 28 pt #0F0F0F squares on a 33 pt pitch from x = 407; a set cue shows
        // as a 15 pt square of its colour behind a black letter.
        for slot in 0..8 {
            let r = Rect::from_center_size(pos2(sx(main, 421.0 + slot as f32 * 33.0), y), vec2(28.0, 28.0));
            let cue = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).cloned();
            let resp = ui.interact(r, ui.id().with(("pad", slot)), Sense::click_and_drag());
            p.rect_filled(r, 1.0, crate::ui_feel::hl(&resp, gy(0x0F0F0F), gy(0x1A1A1A)));
            let tc = match &cue {
                Some(c) => {
                    p.rect_filled(Rect::from_center_size(r.center() - vec2(0.5, 0.5), vec2(15.0, 15.0)), 0.0, hotcue_color(c.color));
                    Color32::BLACK
                }
                None => OFF.c(),
            };
            p.text(r.center() + vec2(-0.3, -1.1), Align2::CENTER_CENTER, ((b'A' + slot as u8) as char).to_string(), FontId::proportional(13.0), tc);
            if crate::ui_player::press_fired(ui, &resp) {
                if ui.input(|i| i.modifiers.shift) {
                    if let Some(c) = &cue {
                        app.delete_cue(c.id);
                    }
                } else {
                    app.hot_cue_down(slot);
                }
            }
            if !resp.is_pointer_button_down_on() && app.deck.hot_preview.is_some() && ui.input(|i| i.pointer.primary_released()) {
                app.hot_cue_up();
            }
            let letter = (b'A' + slot as u8) as char;
            let resp = tip(resp, app, &format!("Set/Play Hot Cue(Loop) {letter}"));
            if let Some(c) = cue {
                resp.context_menu(|ui| hot_cue_menu(app, ui, &c));
                // Clear: a small cross in the pad's corner while the pointer is over the pad.
                let xr = Rect::from_center_size(pos2(r.max.x - 4.5, r.min.y + 4.5), vec2(10.0, 10.0));
                let xresp = ui.interact(xr, ui.id().with(("padclr", slot)), Sense::click());
                if ui.rect_contains_pointer(r.expand(2.0)) {
                    p.rect_filled(xr, 2.0, crate::ui_feel::hl(&xresp, gy(0x323232), gy(0x505050)));
                    close_cross(&p, xr.center(), 5.0, WHITE.c());
                }
                if tip(xresp, app, &format!("Clear Hot Cue(Loop) {letter}")).clicked() {
                    app.delete_cue(c.id);
                }
            }
        }
        // memory: MEMORY x 696..756, then 20 pt tiles at 761, 786 and 811 (◀ ▶ ✕)
        if tip(bold_button(ui, at(726.0, 60.0), "mem", "MEMORY"), app, "Set Memory Cue(Loop)").clicked() {
            app.memory_set();
        }
        if tip(icon_button(ui, at(771.0, 20.0), "memprev", Some(BTN.c()), |p, r, c| crate::icons_player::mem_tri(p, r.center() + vec2(-5.0, 0.0), -1.0, c)), app, "Call Previous Memory Cue").clicked() {
            app.memory_jump(-1);
        }
        if tip(icon_button(ui, at(796.0, 20.0), "memnext", Some(BTN.c()), |p, r, c| crate::icons_player::mem_tri(p, r.center() + vec2(5.0, 0.0), 1.0, c)), app, "Call Next Memory Cue").clicked() {
            app.memory_jump(1);
        }
        if tip(icon_button(ui, at(821.0, 20.0), "memdel", Some(BTN.c()), |p, r, c| crate::icons_player::cross(p, r.center(), 10.0, 1.3, c)), app, "Delete Memory Cue(Loop)").clicked() {
            app.memory_delete_current();
        }
        // loop group: AU 861..898, MA 900..937, ‹ tile 942..962, field 964..1034, › tile 1036..1056
        let au = app.prefs.loop_auto;
        if tip(seg_button(ui, at(879.5, 37.0), "au", "AU", au), app, "Auto Beat Loop").clicked() {
            app.prefs.loop_auto = true;
        }
        if tip(seg_button(ui, at(918.5, 37.0), "ma", "MA", !au), app, "Manual Loop").clicked() {
            app.prefs.loop_auto = false;
        }
        if au {
            if tip(icon_button(ui, at(952.0, 20.0), "lhalf", Some(BTN.c()), |p, r, c| crate::icons_player::chevron_left(p, r.center(), 1.25, c)), app, "Auto Beat Loop:\nMake Loop length shorter.").clicked() {
                app.resize_loop(0.5);
            }
            let lb = app.prefs.loop_beats;
            let label = crate::player_ext::beats_text(lb);
            let active = app.loop_active().is_some();
            let field = at(999.0, 70.0);
            if tip(button(ui, field, "lbeat", &label, if active { LOOP.gamma_multiply(0.7) } else { BTN.c() }, WHITE.c(), 11.0), app, "Auto Beat Loop:\nSet/Reset Loop of displayed beat length.").clicked() {
                if active {
                    app.loop_exit();
                } else {
                    app.auto_loop(lb);
                }
            }
            // Size list (1/64 … 512 beats): the small arrow at the field's right end.
            let dr = Rect::from_min_max(pos2(field.max.x - 14.0, field.min.y), field.max);
            let dresp = tip(ui.interact(dr, ui.id().with("lsize"), Sense::click()), app, "Beat Loop");
            // rekordbox shows no arrow in the field: ours appears only under the pointer.
            if ui.rect_contains_pointer(field) {
                crate::icons_player::chevron_down(&p, dr.center(), 0.9, crate::ui_feel::hl(&dresp, DIM.c(), WHITE.c()));
            }
            crate::ui_browser::dropdown(ui, &dresp, "lsizepop", |ui: &mut Ui| crate::ui_player2::loop_size_menu(app, ui));
            if tip(icon_button(ui, at(1046.0, 20.0), "ldouble", Some(BTN.c()), |p, r, c| crate::icons_player::chevron_right(p, r.center(), 1.25, c)), app, "Auto Beat Loop:\nMake Loop length longer.").clicked() {
                app.resize_loop(2.0);
            }
        } else {
            // Manual loop: IN 951..985, OUT 988..1022, RELOOP/EXIT 1025..1095.
            crate::ui_player2::manual_loop_buttons(app, ui, |x, w| at(x, w));
        }
        // Q 1206..1239, menu: a 21 pt tile at 1249
        let q = app.prefs.quantize;
        if app.prefs.show_stems && tip(seg_button(ui, at(1165.0, 46.0), "stems_btn", "STEMS", false), app, "Show the stem (part) controls").clicked() {
            app.deck.ctrl_tab = 2;
        }
        if tip(seg_button(ui, at(1222.5, 33.0), "quant", "Q", q), app, "Quantize ON/OFF").clicked() {
            app.prefs.quantize = !q;
            app.save_prefs();
        }
        let menu_r = tip(icon_button(ui, Rect::from_center_size(pos2(sx(main, 1259.5), y - 0.5), vec2(21.0, 21.0)), "stripmenu", Some(BTN.c()), |p, r, c| crate::icons_player::menu(p, r.center(), 1.625, c)), app, "Show Menu items");
        crate::ui_browser::dropdown(ui, &menu_r, "strippop", |ui: &mut Ui| {
            if ui.button("Clear all hot cues").clicked() {
                let ids: Vec<i64> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Hot).map(|c| c.id).collect();
                for i in ids {
                    app.delete_cue(i);
                }
            }
            if ui.button("Clear all memory cues").clicked() {
                let ids: Vec<i64> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).map(|c| c.id).collect();
                for i in ids {
                    app.delete_cue(i);
                }
            }
            if ui.button("Memory cues from hot cues").clicked() {
                let hot: Vec<Cue> = app.deck.cues.iter().filter(|c| c.kind == CueKind::Hot).cloned().collect();
                for h in hot {
                    if !app.deck.cues.iter().any(|c| c.kind == CueKind::Memory && (c.start - h.start).abs() < 0.001) {
                        let mut c = Cue { id: 0, kind: CueKind::Memory, slot: -1, color: 0, ..h };
                        app.snapshot_cues();
                        if let Ok(id) = app.db.save_cue(&c) {
                            c.id = id;
                            app.deck.cues.push(c);
                        }
                    }
                }
                if let Some(t) = app.deck.track {
                    app.refresh_cue_counts(t);
                }
            }
            crate::ui_player2::strip_menu_extra(app, ui);
        });
    } else if app.deck.ctrl_tab == 2 {
        stems_strip(app, ui, s, main);
    } else {
        crate::ui_player2::grid_strip(app, ui, s, main);
    }
}

pub(crate) fn stem_part_buttons(app: &mut App, ui: &mut Ui, rect_for: impl Fn(usize) -> Rect) {
    let loaded = app.deck.track.is_some();
    let on = app.player.state.lock().stem_on;
    let p = ui.painter().clone();
    // (label, source indices, colour). Three parts put bass with the instruments. Colours as
    // measured from rekordbox 7.2.19 (reference set, stems/13).
    const VOCAL: Color32 = Color32::from_rgb(0x02, 0xDA, 0x0C);
    const INST: Color32 = Color32::from_rgb(0xDA, 0x1B, 0x02);
    const BASS: Color32 = Color32::from_rgb(0xD2, 0x00, 0x90);
    const DRUMS: Color32 = Color32::from_rgb(0x46, 0x55, 0xFF);
    let parts: &[(&str, &[usize], Color32)] = if app.prefs.stem_parts == 3 {
        &[("VOCAL", &[3], VOCAL), ("INST", &[2, 1], INST), ("DRUMS", &[0], DRUMS)]
    } else {
        &[("VOCAL", &[3], VOCAL), ("INST", &[2], INST), ("BASS", &[1], BASS), ("DRUMS", &[0], DRUMS)]
    };
    for (i, (label, srcs, col)) in parts.iter().enumerate() {
        let r = rect_for(i);
        let resp = ui.interact(r, ui.id().with(("stem", i)), Sense::click());
        let lit = srcs.iter().all(|s| on[*s]);
        p.rect_filled(r, 1.0, crate::ui_feel::hl_if(&resp, loaded, gy(0x0F0F0F), gy(0x1E1E1E)));
        if lit {
            p.rect_stroke(r, 1.0, Stroke::new(1.0_f32, *col), egui::StrokeKind::Inside);
        }
        p.text(r.center(), Align2::CENTER_CENTER, *label, FontId::proportional(10.0), if lit { *col } else { gy(0x8C8C8C) });
        let resp = resp.on_hover_text(format!("STEM MUTE ({label}):\nThe {label} Stem is output when set to on. The {label} Stem is muted when set to off."));
        if resp.clicked() && loaded {
            // MUTE or SOLO, as chosen in Preferences > STEMS > ACTIVE STEM Setting.
            crate::prefs_ext::stem_part_click(app, srcs);
        }
    }
}

/// Stem controls: VOCAL / INST / BASS / DRUMS (mute a part by switching it off), the model in
/// use and separation progress.
fn stems_strip(app: &mut App, ui: &mut Ui, s: Rect, main: Rect) {
    let p = ui.painter().clone();
    let y = s.min.y + 24.0;
    let at = |x: f32, w: f32| Rect::from_center_size(pos2(sx(main, x), y), vec2(w, 26.0));
    let (have, preview_frames) = {
        let player = app.player.state.lock();
        (player.stems.is_some(), player.stem_preview.as_ref().map_or(0, |p| p.ready_frames()))
    };
    let loaded = app.deck.track.is_some();
    // As rekordbox's deck strip (reference set, stems/14 and 30): the mode ("MUTE" or "SOLO")
    // in grey, then one 44 x 17 pt button per part on a 50 pt pitch. A sounding part has its
    // name and a 1 pt outline in its colour; a muted one is grey with no outline.
    p.text(pos2(sx(main, 438.0), y), Align2::RIGHT_CENTER, if app.prefs.stem_solo { "SOLO" } else { "MUTE" }, FontId::proportional(10.0), gy(0x8C8C8C));
    stem_part_buttons(app, ui, |i| at(474.0 + i as f32 * 50.0, 44.0).shrink2(vec2(0.0, 4.5)));
    // Status and actions.
    let model_info = app.stem.status_model();
    let model = model_info.map_or("checking this computer…", |m| m.name);
    let exporting = crate::stem_export::busy_for(app, app.deck.track);
    let queued = app.queued_stems();
    let model_download = app.unrequested_stem_download();
    let busy = exporting || app.separating_stems() || queued || model_download;
    let loading = app.loading_stems();
    let error = app.stem.error_for(app.deck.track).filter(|_| !busy && !loading);
    let retry_cpu = error.is_some_and(|e| e.retry_model.is_some());
    let status = if let Some(s) = crate::ui_stem_export::strip_status(app) {
        s
    } else if let Some(status) = app.stem.download_status() {
        status
    } else if queued {
        "Queued for separation…".to_owned()
    } else if loading {
        "Loading saved stems…".to_owned()
    } else if let Some(status) = app.stem.separation_status() {
        if app.separating_stems() {
            if preview_frames > 0 {
                let secs = preview_frames / crate::stems::htdemucs::SAMPLE_RATE as usize;
                format!("{status} · Preview to {}:{:02}", secs / 60, secs % 60)
            } else {
                status
            }
        } else {
            "Separating another track or model…".to_owned()
        }
    } else if retry_cpu {
        "GPU separation failed · Click to copy details".to_owned()
    } else if error.is_some() {
        "STEMS failed · Click to copy details".to_owned()
    } else if have {
        "Parts ready".to_owned()
    } else if !loaded {
        "Load a track to separate it into parts.".to_owned()
    } else if crate::stems::backend_hold().is_some() {
        format!("GPU separation paused · {model} is available on CPU.")
    } else {
        format!("Not separated yet. Model: {model}.")
    };
    let status_rect = Rect::from_min_max(pos2(sx(main, 700.0), y - 10.0), pos2(at(1150.0, 84.0).left() - 4.0, y + 10.0));
    p.with_clip_rect(status_rect).text(status_rect.left_center(), Align2::LEFT_CENTER, status, FontId::proportional(11.5), if error.is_some() { CAUTION_TX } else { DIM2.c() });
    if let Some(error) = error {
        let response = ui.interact(status_rect, ui.id().with("stem_failure_details"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_ui(|ui| { ui.set_max_width(460.0); ui.label(&error.detail); });
        if response.clicked() {
            ui.ctx().copy_text(error.detail.clone());
            app.toast("Copied STEMS error details.");
        }
    } else if let Some(reason) = crate::stems::backend_hold() {
        ui.interact(status_rect, ui.id().with("stem_driver_status"), Sense::hover()).on_hover_text(reason);
    } else if let Some(m) = model_info {
        ui.interact(status_rect, ui.id().with("stem_model_info"), Sense::hover()).on_hover_text(format!("{} — {}. Licence: {}. Checked against its published checksum after download.", m.name, m.source, m.licence));
    }
    if busy {
        if tip(bold_button(ui, at(1165.0, 54.0), "stem_stop", "STOP"), app, "Stop").clicked() {
            if exporting {
                crate::stem_export::stop(app);
            } else if model_download {
                app.stem.download_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                app.stop_deck_stems();
            }
        }
    } else if !have && loaded && !loading {
        let (label, help) = if retry_cpu { ("RETRY CPU", "Retry this track on the CPU with the same model") }
            else if app.manual_stem_device() == crate::stems::SeparationDevice::Cpu { ("SEPARATE ON CPU", "Separate this track on the CPU while GPU separation is paused") }
            else { ("SEPARATE", "Split this track into vocal, instrument, bass and drum parts") };
        if tip(bold_button(ui, at(1150.0, 84.0), "stem_go", label), app, help).clicked() {
            app.separate_loaded();
        }
    }
    // recordbox's stem export: left of the part buttons, where the strip is empty.
    crate::ui_stem_export::strip_control(app, ui, at(370.0, 64.0).shrink2(vec2(0.0, 3.0)));
    if tip(seg_button(ui, at(1222.5, 40.0), "stems_back", "BACK", false), app, "Back to the cue and loop controls").clicked() {
        app.deck.ctrl_tab = 0;
    }
}

/// MEMORY-style raised button with a bold caps label.
pub(crate) fn bold_button(ui: &mut Ui, rect: Rect, id: &str, label: &str) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(rect, 2.0, crate::ui_feel::face(&r, BTN.c(), lighten(BTN.c()), BG2.c()));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, caps(8.8), WHITE.c());
    r
}

/// Stacked CUE/LOOP and GRID tabs: active #1E1E1E, inactive black, bold caps.
fn tab_button(ui: &mut Ui, rect: Rect, id: &str, label: &str, active: bool) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(rect, 0.0, if active { BG2 } else { BG });
    ui.painter().text(rect.center() + vec2(0.0, -0.25), Align2::CENTER_CENTER, label, caps(8.8), WHITE.c());
    r
}

/// Toggle buttons (AU / MA / Q, the panel tabs): blue when on, #1E1E1E with a 1 pt #646464 outline when off.
pub(crate) fn seg_button(ui: &mut Ui, rect: Rect, id: &str, label: &str, on: bool) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    let lit = crate::ui_feel::state_amt(r.id, on);
    let fill = crate::motion::mix(crate::ui_feel::hl(&r, BG2.c(), gy(0x2A2A2A)), BLUE.c(), lit);
    ui.painter().rect_filled(rect, 2.0, fill);
    if lit < 1.0 {
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0_f32, crate::motion::fade(OFF.c(), 1.0 - lit)), egui::StrokeKind::Inside);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, caps(8.8), WHITE.c());
    r
}

pub(crate) fn hot_cue_menu(app: &mut App, ui: &mut Ui, c: &Cue) {
    ui.label(format!("Hot Cue {}", (b'A' + c.slot as u8) as char));
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(180.0);
        for idx in HOTCUE_PICKER {
            let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            ui.painter().rect_filled(r, 2.0, hotcue_color(idx));
            if c.color == idx {
                ui.painter().rect_stroke(r, 2.0, Stroke::new(2.0_f32, WHITE), egui::StrokeKind::Inside);
            }
            if resp.clicked() {
                let mut n = c.clone();
                n.color = idx;
                app.update_cue(n);
                ui.close_menu();
            }
        }
    });
    if ui.button("Add comments").clicked() {
        app.deck.panel_tab = 1;
        app.deck.editing_comment = Some(c.id);
        ui.close_menu();
    }
    if ui.button("Delete").clicked() {
        app.delete_cue(c.id);
        ui.close_menu();
    }
}

/// Heights of the MEMORY / HOT CUE / INFO panel: the pitch of the ten memory rows and the
/// eight hot cue rows, where the list ends and where the tabs are (from the panel's top).
pub(crate) struct PanelGeom {
    pub mem_pitch: f32,
    pub hot_pitch: f32,
    pub list_bottom: f32,
    pub tabs_y: f32,
}

/// 1 Player: MEMORY 10 × 21 pt on a 22 pt pitch, HOT CUE 8 × 26 pt on a 27 pt pitch, tabs at y 230.
pub(crate) const PANEL_1P: PanelGeom = PanelGeom { mem_pitch: 22.0, hot_pitch: 27.0, list_bottom: 225.0, tabs_y: 230.0 };
/// 2 Players / Dual Player (one panel per deck, 186 pt tall): MEMORY 10 × 14 pt on a 15 pt
/// pitch, the eight hot cues in the same height, tabs at y 162.
pub(crate) const PANEL_2P: PanelGeom = PanelGeom { mem_pitch: 15.0, hot_pitch: 18.75, list_bottom: 155.0, tabs_y: 162.0 };

pub(crate) fn side_panel(app: &mut App, ui: &mut Ui, r: Rect, g: &PanelGeom) {
    let p = ui.painter().clone();
    // Reference 01/03: a #1E1E1E panel (x 1303..1509, from 1 pt below the player's top) holding
    // the list (x 1311..1501, from y 6) as separate rounded rows; three 62 × 18 pt tabs below.
    let tabs_y = r.min.y + g.tabs_y;
    p.rect_filled(Rect::from_min_max(pos2(r.min.x - 7.0, r.min.y + 1.0), pos2(r.max.x - 3.0, r.max.y)), 0.0, gy(0x1E1E1E));
    let list = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 6.0), pos2(r.max.x - 11.0, r.min.y + g.list_bottom));
    for (i, name) in ["MEMORY", "HOT CUE", "INFO"].iter().enumerate() {
        let tr = Rect::from_min_size(pos2(r.min.x + 1.0 + i as f32 * 64.0, tabs_y), vec2(62.0, 18.0));
        let tipt = ["Show MEMORY Panel", "Show HOT CUE Panel", "Show INFO Panel"][i];
        if tip(seg_button(ui, tr, &format!("ptab{i}"), name, app.deck.panel_tab == i as u8), app, tipt).clicked() {
            app.deck.panel_tab = i as u8;
        }
    }
    let row_h = g.mem_pitch;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    match app.deck.panel_tab {
        0 => {
            let mems = app.memory_cues();
            for i in 0..10 {
                let rr = Rect::from_min_size(pos2(list.min.x, list.min.y + i as f32 * row_h), vec2(list.width(), row_h - 1.0));
                p.rect_filled(rr, 2.0, gy(0x0F0F0F));
                let Some(c) = mems.get(i).cloned() else {
                    crate::icons_player::cross(&p, pos2(rr.max.x - 10.0, rr.center().y - 0.5), 8.0, 0.9, gy(0x454545));
                    continue;
                };
                let col = if c.color > 0 { label_color(c.color) } else { BAR_RED };
                tri_up(&p, pos2(rr.min.x + 9.0, rr.center().y - 4.0), 4.0, col);
                p.text(pos2(rr.min.x + 18.0, rr.center().y), Align2::LEFT_CENTER, cue_time(c.start), FontId::proportional(10.5), WHITE.c());
                if c.is_loop() {
                    p.text(pos2(rr.min.x + 82.0, rr.center().y), Align2::LEFT_CENTER, "∞", FontId::proportional(11.0), if c.active_loop { LOOP } else { DIM.c() });
                }
                let del = Rect::from_center_size(pos2(rr.max.x - 10.0, rr.center().y), vec2(16.0, row_h));
                if app.deck.editing_comment == Some(c.id) {
                    let mut txt = c.comment.clone();
                    let er = Rect::from_min_max(pos2(rr.min.x + 96.0, rr.min.y + 1.0), pos2(rr.max.x - 20.0, rr.max.y - 1.0));
                    let resp = child.put(er, egui::TextEdit::singleline(&mut txt).font(FontId::proportional(10.5)));
                    // Whether the field just gave up focus must be read before focus is asked for again.
                    let gave_up = resp.lost_focus();
                    if !gave_up {
                        resp.request_focus();
                    }
                    if txt != c.comment {
                        let mut n = c.clone();
                        n.comment = txt;
                        app.update_cue(n);
                    }
                    if gave_up {
                        app.deck.editing_comment = None;
                    }
                } else {
                    p.text(pos2(rr.min.x + 96.0, rr.center().y), Align2::LEFT_CENTER, &c.comment, FontId::proportional(10.5), DIM2.c());
                }
                crate::icons_player::cross(&p, pos2(rr.max.x - 10.0, rr.center().y - 0.5), 8.0, 0.9, WHITE.c());
                let dr = child.interact(del, child.id().with(("memdel", c.id)), Sense::click());
                if dr.clicked() {
                    app.delete_cue(c.id);
                    continue;
                }
                let resp = child.interact(Rect::from_min_max(rr.min, pos2(del.min.x, rr.max.y)), child.id().with(("mem", c.id)), Sense::click());
                if c.is_loop() {
                    // The loop sign switches Active Loop Playback (the loop starts by itself
                    // when playback reaches it on a player).
                    let lr = Rect::from_center_size(pos2(rr.min.x + 87.0, rr.center().y), vec2(14.0, row_h - 2.0));
                    if tip(child.interact(lr, child.id().with(("memactive", c.id)), Sense::click()), app, "Set Active Loop").clicked() {
                        let mut n = c.clone();
                        n.active_loop = !n.active_loop;
                        app.update_cue(n);
                        continue;
                    }
                }
                if resp.clicked() {
                    app.call_memory(&c);
                }
                if resp.double_clicked() {
                    app.deck.editing_comment = Some(c.id);
                }
                resp.context_menu(|ui| {
                    if ui.button("Call Memory Cue(Loop)").clicked() {
                        app.call_memory(&c);
                        ui.close_menu();
                    }
                    if ui.button("Add comments to the memory cue (loop)").clicked() {
                        app.deck.editing_comment = Some(c.id);
                        ui.close_menu();
                    }
                    if c.is_loop() && ui.checkbox(&mut c.active_loop.clone(), "Active Loop Playback").clicked() {
                        let mut n = c.clone();
                        n.active_loop = !n.active_loop;
                        app.update_cue(n);
                        ui.close_menu();
                    }
                    ui.menu_button("Color", |ui| {
                        for i in 0..9u8 {
                            if ui.selectable_label(c.color == i, egui::RichText::new(format!("■ {}", app.prefs.color_name(i))).color(if i == 0 { DIM.c() } else { label_color(i) })).clicked() {
                                let mut n = c.clone();
                                n.color = i;
                                app.update_cue(n);
                                ui.close_menu();
                            }
                        }
                    });
                    if ui.button("Delete Memory Cue(Loop)").clicked() {
                        app.delete_cue(c.id);
                        ui.close_menu();
                    }
                });
            }
        }
        1 => {
            for slot in 0..8 {
                let rr = Rect::from_min_size(pos2(list.min.x, list.min.y + slot as f32 * g.hot_pitch), vec2(list.width(), g.hot_pitch - 1.0));
                p.rect_filled(rr, 2.0, gy(0x0F0F0F));
                let c = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).cloned();
                let chip = Rect::from_center_size(pos2(rr.min.x + 15.5, rr.center().y - 0.5), vec2(11.0, 11.0));
                let letter = ((b'A' + slot as u8) as char).to_string();
                match &c {
                    Some(c) => {
                        p.rect_filled(chip, 0.0, hotcue_color(c.color));
                        p.text(chip.center(), Align2::CENTER_CENTER, &letter, FontId::proportional(10.0), Color32::BLACK);
                        p.text(pos2(rr.min.x + 31.5, rr.center().y - 0.5), Align2::LEFT_CENTER, fmt_time(c.start, false), FontId::proportional(10.7), WHITE.c());
                        let del = Rect::from_center_size(pos2(rr.max.x - 17.0, rr.center().y), vec2(18.0, rr.height()));
                        if app.deck.editing_comment == Some(c.id) {
                            let mut txt = c.comment.clone();
                            let er = Rect::from_min_max(pos2(rr.min.x + 66.5, rr.min.y + 3.0), pos2(rr.max.x - 26.0, rr.max.y - 3.0));
                            let resp = child.put(er, egui::TextEdit::singleline(&mut txt).font(FontId::proportional(10.5)));
                            // Whether the field just gave up focus must be read before focus is asked for again.
                            let gave_up = resp.lost_focus();
                            if !gave_up {
                                resp.request_focus();
                            }
                            if txt != c.comment {
                                let mut n = c.clone();
                                n.comment = txt;
                                app.update_cue(n);
                            }
                            if gave_up {
                                app.deck.editing_comment = None;
                            }
                        } else {
                            p.text(pos2(rr.min.x + 67.0, rr.center().y - 0.5), Align2::LEFT_CENTER, &c.comment, FontId::proportional(10.7), WHITE.c());
                        }
                        crate::icons_player::cross(&p, pos2(rr.max.x - 17.0, rr.center().y), 8.0, 1.0, WHITE.c());
                        if child.interact(del, child.id().with(("hcdel", slot)), Sense::click()).clicked() {
                            app.delete_cue(c.id);
                            continue;
                        }
                        let resp = child.interact(Rect::from_min_max(rr.min, pos2(del.min.x, rr.max.y)), child.id().with(("hc", slot)), Sense::click());
                        if resp.clicked() {
                            app.seek(c.start);
                            if !app.playing() {
                                app.deck.cue_point = c.start;
                            }
                        }
                        if resp.double_clicked() {
                            app.deck.editing_comment = Some(c.id);
                        }
                        let cc = c.clone();
                        resp.context_menu(|ui| hot_cue_menu(app, ui, &cc));
                    }
                    None => {
                        // An empty slot shows only its letter, in grey.
                        p.text(chip.center() + vec2(0.0, -0.5), Align2::CENTER_CENTER, &letter, FontId::proportional(11.0), OFF.c());
                    }
                }
            }
        }
        _ => info_panel(app, &mut child, list),
    }
}

/// INFO panel (reference 04): a #0F0F0F sheet with a 3 pt scroll bar at its right and three
/// ruled rows: rating (31 pt), colour (30 pt) and comment (47 pt). My Tag and the file's
/// format follow underneath.
pub(crate) fn info_panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    let sheet = Rect::from_min_max(r.min, pos2(r.max.x - 5.0, r.max.y));
    p.rect_filled(sheet, 0.0, gy(0x0F0F0F));
    p.rect_filled(Rect::from_min_max(pos2(r.max.x - 3.0, r.min.y), pos2(r.max.x, r.max.y - 25.0)), 0.0, gy(0x555555));
    let (x0, y0) = (r.min.x, r.min.y);
    for (y, x1) in [(31.0, 179.0), (62.0, 179.0), (110.0, 170.0)] {
        p.rect_filled(Rect::from_min_max(pos2(x0 + 10.0, y0 + y), pos2(x0 + x1, y0 + y + 1.0)), 0.0, BG2);
    }
    let Some(id) = app.deck.track else {
        return;
    };
    let Some(t) = app.track(id).cloned() else { return };
    // Rating: five 10.5 pt stars on a 16 pt pitch.
    for s in 1..=5u8 {
        let c = pos2(x0 + 15.75 + (s - 1) as f32 * 16.0, y0 + 15.75);
        let resp = ui.interact(Rect::from_center_size(c, vec2(16.0, 20.0)), ui.id().with(("infostar", s)), Sense::click());
        crate::icons_player::star(&p, c, if t.rating >= s { WHITE.c() } else { DIM.c() }, t.rating >= s);
        if resp.clicked() {
            let nr = if t.rating == s { 0 } else { s };
            if let Some(tm) = app.track_mut(id) {
                tm.rating = nr;
            }
            app.save_track(id);
        }
    }
    // Colour: the track's colour and its name; the choices appear under the pointer.
    let crow = Rect::from_min_max(pos2(x0, y0 + 32.0), pos2(sheet.max.x, y0 + 62.0));
    if ui.rect_contains_pointer(crow) {
        for i in 0..9u8 {
            let rr = Rect::from_min_size(pos2(x0 + 10.0 + i as f32 * 18.0, y0 + 40.0), vec2(14.0, 14.0));
            let resp = ui.interact(rr, ui.id().with(("infocol", i)), Sense::click());
            p.rect_filled(rr, 2.0, if i == 0 { BG2.c() } else { label_color(i) });
            if t.color == i {
                p.rect_stroke(rr, 2.0, Stroke::new(1.5_f32, WHITE), egui::StrokeKind::Inside);
            }
            if resp.on_hover_text(app.prefs.color_name(i)).clicked() {
                if let Some(tm) = app.track_mut(id) {
                    tm.color = i;
                }
                app.save_track(id);
            }
        }
    } else if t.color > 0 {
        p.rect_filled(Rect::from_min_size(pos2(x0 + 10.0, y0 + 41.0), vec2(12.0, 12.0)), 2.0, label_color(t.color));
        p.text(pos2(x0 + 28.0, y0 + 47.0), Align2::LEFT_CENTER, app.prefs.color_name(t.color), FontId::proportional(11.0), DIM2.c());
    }
    // Comment: the bubble mark and the text.
    crate::icons_player::bubble(&p, pos2(x0 + 10.0, y0 + 72.0), DIM.c());
    let mut c = t.comment.clone();
    let er = Rect::from_min_max(pos2(x0 + 28.0, y0 + 67.0), pos2(sheet.max.x - 8.0, y0 + 87.0));
    let resp = ui.put(er, egui::TextEdit::singleline(&mut c).frame(false).font(FontId::proportional(11.0)).text_color(WHITE.c()));
    if resp.changed() {
        if let Some(tm) = app.track_mut(id) {
            tm.comment = c;
        }
        app.save_track(id);
    }
    let tags = app.tag_text(id);
    if !tags.is_empty() {
        p.with_clip_rect(sheet).text(pos2(x0 + 10.0, y0 + 126.0), Align2::LEFT_CENTER, tags, FontId::proportional(10.5), DIM2.c());
    }
    if ui.rect_contains_pointer(sheet) {
        // The file's format, shown while the pointer is over the panel.
        p.text(pos2(x0 + 10.0, r.max.y - 12.0), Align2::LEFT_CENTER, format!("{} · {} kbps · {} Hz", t.file_type.to_uppercase(), t.bitrate, t.sample_rate), FontId::proportional(10.0), OFF.c());
    }
}

pub fn star(p: &egui::Painter, c: Pos2, r: f32, col: Color32, filled: bool) {
    let mut pts = vec![];
    for i in 0..10 {
        let a = std::f32::consts::PI / 5.0 * i as f32 - std::f32::consts::FRAC_PI_2;
        let rr = if i % 2 == 0 { r } else { r * 0.45 };
        pts.push(c + vec2(a.cos() * rr, a.sin() * rr));
    }
    if filled {
        // convex fan from centre (star isn't convex)
        for i in 0..10 {
            p.add(Shape::convex_polygon(vec![c, pts[i], pts[(i + 1) % 10]], col, Stroke::NONE));
        }
    } else {
        let mut closed = pts.clone();
        closed.push(pts[0]);
        p.add(Shape::line(closed, Stroke::new(1.0_f32, col)));
    }
}

fn cue_time(t: f64) -> String {
    let ms = (t * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:03}", ms / 60000, (ms / 1000) % 60, ms % 1000)
}

#[cfg(test)]
mod marker_drag_tests {
    use super::*;

    fn cue(id: i64, kind: CueKind, start: f64, end: Option<f64>) -> Cue {
        Cue { id, track_id: 1, kind, slot: if kind == CueKind::Hot { 0 } else { -1 }, start, end, color: 0, comment: String::new(), active_loop: false }
    }

    #[test]
    fn markers_are_grabbed_by_their_handles_only() {
        let r = Rect::from_min_max(pos2(0.0, 100.0), pos2(1000.0, 200.0));
        // 100 px per second, 0 s at x = 0.
        let x_of = |t: f64| (t * 100.0) as f32;
        let cues = vec![cue(1, CueKind::Hot, 2.0, None), cue(2, CueKind::Memory, 5.0, None), cue(3, CueKind::Hot, 2.1, None)];
        // Hot cue letter box (the row above the beats): the nearer box wins.
        assert_eq!(cue_marker_hit(&cues, pos2(203.0, 93.0), r, x_of), Some(1));
        assert_eq!(cue_marker_hit(&cues, pos2(209.0, 93.0), r, x_of), Some(3));
        // Memory cue triangle (bottom strip).
        assert_eq!(cue_marker_hit(&cues, pos2(503.0, 195.0), r, x_of), Some(2));
        // The waveform body scrubs, it never grabs a marker.
        assert_eq!(cue_marker_hit(&cues, pos2(205.0, 150.0), r, x_of), None);
        assert_eq!(cue_marker_hit(&cues, pos2(503.0, 150.0), r, x_of), None);
        assert_eq!(cue_marker_hit(&cues, pos2(800.0, 93.0), r, x_of), None);
        assert_eq!(cue_marker_hit(&cues, pos2(203.0, 105.0), r, x_of), None);
    }

    #[test]
    fn moved_cues_keep_loop_length_and_stay_in_the_track() {
        let l = cue(1, CueKind::Hot, 10.0, Some(14.0));
        let m = moved_cue(&l, 20.0, 100.0);
        assert_eq!((m.start, m.end), (20.0, Some(24.0)));
        // A loop can't run past the end, and nothing goes before 0.
        assert_eq!(moved_cue(&l, 99.0, 100.0).start, 96.0);
        assert_eq!(moved_cue(&l, -3.0, 100.0).start, 0.0);
        let p = cue(2, CueKind::Memory, 5.0, None);
        assert_eq!(moved_cue(&p, 7.5, 100.0).end, None);
        let z = cue(3, CueKind::Hot, 5.0, Some(5.0));
        assert_eq!(moved_cue(&z, 8.0, 100.0).end, Some(8.0));
    }
}

/// The list rows' close button: a cross of `size` pt with a 1.5 pt stroke.
pub(crate) fn close_cross(p: &egui::Painter, c: Pos2, size: f32, col: Color32) {
    let h = size / 2.0;
    p.line_segment([c + vec2(-h, -h), c + vec2(h, h)], Stroke::new(1.5_f32, col));
    p.line_segment([c + vec2(-h, h), c + vec2(h, -h)], Stroke::new(1.5_f32, col));
}
