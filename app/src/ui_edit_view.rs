//! EDIT mode's screen. Geometry from rekordbox 7.2.19's `resized` functions (spec:
//! checks/re-7.2.19/edit-mode.md sections 1 and 6) and measured on the three captures in
//! `rekordbox-reference/edit/`; rekordbox's own artwork (the pads' PNGs, the icons' SVGs) is
//! read from the installed rekordbox at run time and never copied. With a track loaded there
//! is no capture: the timeline follows the spec's numbers and colours.

use crate::app::*;
use crate::edit_audio::EDIT;
use crate::edit_model::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_edit::*;
use crate::ui_kit as kit;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Stroke, Ui};
use std::collections::HashMap;

const SKIN: &str = "/Applications/rekordbox 7/rekordbox.app/Contents/Resources/skins/";
const ICON: Color32 = hex(0x969696);
const ICON_OVER: Color32 = hex(0xAFAFAF);
const ICON_DOWN: Color32 = hex(0x7D7D7D);
const BAR: Color32 = hex(0x191919);
const FRAME: Color32 = hex(0x323232);
const TAB_TEXT: Color32 = hex(0xAFAFAF);
const CURSOR: Color32 = hex(0x007DE1);
const SEL: Color32 = hex(0x40A0FF);
const TOOL_OPEN: f32 = 320.0;
const TOOL_CLOSED: f32 = 22.0;
/// A JUCE font "height" in em points (Arial: ascent + descent = 1.117 em).
const fn em(juce: f32) -> f32 {
    juce * 0.895
}

fn st(w: f32, c: Color32) -> Stroke {
    Stroke::new(w, c)
}

/// Where the controls were drawn this frame (for the scripted pointer run).
pub struct Map {
    pub wave: Rect,
    pub whole: Rect,
    pub tool: Rect,
    pub rects: HashMap<String, Rect>,
}

impl Default for Map {
    fn default() -> Self {
        Map { wave: Rect::NOTHING, whole: Rect::NOTHING, tool: Rect::NOTHING, rects: HashMap::new() }
    }
}

impl Map {
    pub fn at(&self, name: &str) -> Option<Pos2> {
        self.rects.get(name).map(|r| r.center())
    }
}

// ---------------------------------------------------------------------------- skin assets

/// A texture from rekordbox's skin folder: an SVG as a white shape to tint, a PNG as it is.
fn skin(app: &mut App, name: &str) -> Option<egui::TextureHandle> {
    let key = format!("skin:{name}");
    if let Some(t) = app.textures.get(&key) {
        return t.clone();
    }
    let path = format!("{SKIN}{name}");
    let img = if name.ends_with(".svg") {
        std::fs::read(&path).ok().and_then(|b| egui_extras::image::load_svg_bytes_with_size(&b, Some(egui::SizeHint::Scale(4.0.into()))).ok()).map(|mut ci| {
            for px in ci.pixels.iter_mut() {
                let a = px.a();
                *px = Color32::from_rgba_premultiplied(a, a, a, a);
            }
            ci
        })
    } else {
        image::open(&path).ok().map(|i| {
            let i = i.to_rgba8();
            egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], &i)
        })
    };
    let tex = img.map(|ci| app.ctx.load_texture(&key, ci, egui::TextureOptions::LINEAR));
    app.textures.insert(key, tex.clone());
    tex
}

fn paint_tex(p: &egui::Painter, t: &egui::TextureHandle, r: Rect, tint: Color32) {
    p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
}

/// Plain shapes for when rekordbox is not installed (its icons cannot be read then).
fn fallback_icon(p: &egui::Painter, name: &str, r: Rect, c: Color32) {
    let (l, t, rt, b, m) = (r.min.x, r.min.y, r.max.x, r.max.y, r.center());
    match name {
        "edit/play.svg" => {
            p.add(egui::Shape::convex_polygon(vec![pos2(l, t), pos2(rt, m.y), pos2(l, b)], c, Stroke::NONE));
        }
        "edit/pause.svg" => {
            p.rect_filled(Rect::from_min_max(pos2(l, t), pos2(l + r.width() * 0.36, b)), 0.0, c);
            p.rect_filled(Rect::from_min_max(pos2(rt - r.width() * 0.36, t), pos2(rt, b)), 0.0, c);
        }
        "edit/stop.svg" => {
            p.rect_filled(r, 0.0, c);
        }
        "edit/head.svg" => {
            p.rect_filled(Rect::from_min_max(pos2(l, t), pos2(l + 1.0, b)), 0.0, c);
            p.add(egui::Shape::convex_polygon(vec![pos2(rt, t), pos2(rt, b), pos2(l + 2.0, m.y)], c, Stroke::NONE));
        }
        "edit/loop.svg" => {
            p.rect_stroke(r.shrink2(vec2(3.0, 2.5)), 5.0, st(2.5, c), egui::StrokeKind::Middle);
        }
        "edit/newProject.svg" => {
            p.rect_stroke(r, 1.0, st(1.1, c), egui::StrokeKind::Inside);
            p.line_segment([pos2(l + 3.0, m.y), pos2(rt - 3.0, m.y)], st(1.3, c));
            p.line_segment([pos2(m.x, m.y - 4.0), pos2(m.x, m.y + 4.0)], st(1.3, c));
        }
        "edit/saveProject.svg" => {
            p.rect_stroke(r, 1.0, st(1.1, c), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_max(pos2(l + 3.0, t), pos2(rt - 3.0, m.y - 1.0)), 0.0, c);
            p.rect_stroke(Rect::from_min_max(pos2(l + 2.0, m.y + 1.0), pos2(rt - 2.0, b)), 0.0, st(1.0, c), egui::StrokeKind::Inside);
        }
        "edit/renderAudio.svg" => {
            p.line_segment([pos2(l, m.y), pos2(rt - 2.0, m.y)], st(1.4, c));
            p.line_segment([pos2(m.x, t + 1.0), pos2(rt - 2.0, m.y)], st(1.4, c));
            p.line_segment([pos2(m.x, b - 1.0), pos2(rt - 2.0, m.y)], st(1.4, c));
        }
        "edit/trashBox.svg" => {
            p.rect_stroke(Rect::from_min_max(pos2(l + 2.0, t + 3.0), pos2(rt - 2.0, b)), 1.0, st(1.2, c), egui::StrokeKind::Inside);
            p.line_segment([pos2(l, t + 2.0), pos2(rt, t + 2.0)], st(1.4, c));
        }
        "zoomIn.svg" | "zoomOut.svg" => crate::icons_player::zoom(p, m, r.width() / 12.0, c, name == "zoomIn.svg"),
        _ => {
            p.rect_stroke(r, 1.0, st(1.2, c), egui::StrokeKind::Inside);
        }
    }
}

/// An icon button in the editor's colours (#969696, lighter under the pointer, darker when
/// pressed, 40 % when it cannot be used). `lit`: the colour while switched on.
fn icon_btn(app: &mut App, ui: &mut Ui, r: Rect, name: &str, asset: &str, enabled: bool, lit: Option<Color32>, tip: &str) -> egui::Response {
    let resp = ui.interact(r.expand(3.0), ui.id().with(("edit_icon", name)), if enabled { Sense::click() } else { Sense::hover() });
    let base = lit.unwrap_or(ICON);
    let col = if !enabled {
        base.gamma_multiply(0.4)
    } else if resp.is_pointer_button_down_on() {
        if lit.is_some() { base.gamma_multiply(0.8) } else { ICON_DOWN }
    } else if resp.hovered() {
        if lit.is_some() { Color32::from_rgb(base.r().saturating_add(40), base.g().saturating_add(40), base.b().saturating_add(40)) } else { ICON_OVER }
    } else {
        base
    };
    let p = ui.painter().clone();
    match skin(app, asset) {
        Some(t) => paint_tex(&p, &t, r, col),
        None => fallback_icon(&p, asset, r, col),
    }
    app.edit.map.rects.insert(name.to_string(), r);
    if app.prefs.tooltips && !tip.is_empty() {
        resp.on_hover_text(tip)
    } else {
        resp
    }
}

/// rekordbox's tab: a trapezoid whose right edge leans out by 20 pt, the caption 15 pt in.
fn tab(ui: &Ui, p: &egui::Painter, r: Rect, text: &str, front: bool, text_dx: f32, text_dy: f32) {
    let fill = if front { FRAME } else { BAR };
    // (The body as a rectangle, so that its straight edges stay sharp.)
    p.rect_filled(Rect::from_min_max(r.min, pos2(r.max.x - 20.0, r.max.y)), 0.0, fill);
    p.add(egui::Shape::convex_polygon(vec![pos2(r.max.x - 20.0, r.min.y), r.right_bottom(), pos2(r.max.x - 20.0, r.max.y)], fill, Stroke::NONE));
    // Helvetica at 10.5 pt; rekordbox's text is kerned, which the letter spacing stands in for
    // (fitted per caption on the references, to within a quarter point).
    let tracking = match text {
        "PALETTE" => 0.136,
        "EDIT" => 0.08,
        "BROWSER" => 0.25,
        "BEAT SELECT" => 0.106,
        _ => 0.08,
    };
    let g = kit::galley(ui, text, rb(10.5), TAB_TEXT, tracking);
    p.galley(pos2(r.min.x + text_dx, r.center().y + text_dy - g.size().y / 2.0), g, TAB_TEXT);
}

// ---------------------------------------------------------------------------- header

/// The top bar in EDIT mode: New Project, Save Project and Render Audio after the mode list,
/// and the same right-hand side as the other modes.
pub fn header(app: &mut App, ui: &mut Ui, r: Rect) {
    let open = app.edit.project.is_some();
    let o = r.min + vec2(165.0, 0.0);
    for (name, asset, x, w, h, cmd, tip) in [
        ("new", "edit/newProject.svg", 0.0, 15.0, 17.0, Cmd::NewProject, "Create new project:\nSaves the current project and starts a new project."),
        ("save", "edit/saveProject.svg", 35.0, 13.0, 14.0, Cmd::Save, "Save project:\nSaves edited content to the project."),
        ("render", "edit/renderAudio.svg", 68.0, 19.0, 14.0, Cmd::Render, "Render audio files:\nRenders edit results as an audio file.\nRendered audio files are automatically imported to Collection."),
    ] {
        let rect = Rect::from_min_size(o + vec2(x, 7.0), vec2(w, h));
        if icon_btn(app, ui, rect, name, asset, open, None, tip).clicked() {
            app.edit.queue.push(cmd);
        }
    }
}

// ---------------------------------------------------------------------------- the panel

pub fn hides_browser(app: &App) -> bool {
    is_edit(app) && !app.prefs.edit.show_browser
}

/// The editor between the top bar and the browser (or the status bar, when the browser is
/// hidden).
pub fn top_panel(app: &mut App, ctx: &egui::Context) {
    let avail = ctx.available_rect();
    // rekordbox: 291 pt with the browser shown (its top bar is 1 pt taller than recordbox's);
    // the browser draws from one point above its own panel, so this one is a point taller.
    let h = if app.prefs.edit.show_browser { 293.0_f32.min(avail.height()) } else { avail.height() };
    egui::TopBottomPanel::top("edit_panel").exact_height(h).show_separator_line(false).frame(egui::Frame::default().fill(Color32::BLACK)).show(ctx, |ui| panel(app, ui));
    modals(app, ctx);
}

fn panel(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let browser = app.prefs.edit.show_browser;
    let o = full.min + vec2(0.0, 1.0);
    let w = full.width();
    // Without the browser rekordbox's panel runs 5 pt under the status bar's top edge (its
    // status bar is that much lower); the BROWSER tab is drawn as far as rekordbox shows it.
    let h = if browser { 291.0 } else { full.max.y - o.y + 5.0 };
    let t = if app.prefs.edit.show_tool { TOOL_OPEN } else { TOOL_CLOSED };
    let l = w - t;
    let tab_r = Rect::from_min_size(o + vec2(2.0, if browser { 261.0 } else { h - 30.0 }), vec2((l - 6.0) / 2.0, 30.0));
    let pad_r = Rect::from_min_size(pos2(o.x + 2.0, tab_r.min.y - 211.0), vec2(l - 6.0, 209.0));
    let lower = if browser { tab_r.min.y - 2.0 } else { pad_r.min.y - 2.0 };
    let player = Rect::from_min_max(o + vec2(2.0, 34.0), pos2(o.x + l - 4.0, lower));
    let tool = Rect::from_min_max(o + vec2(l - 2.0, 34.0), pos2(o.x + l - 2.0 + t, if browser { o.y + h } else { pad_r.max.y }));
    app.edit.map.tool = tool;
    project_bar(app, ui, Rect::from_min_size(o + vec2(2.0, 2.0), vec2(w - 4.0, 30.0)));
    player_panel(app, ui, player);
    if !browser {
        pads(app, ui, pad_r);
    }
    // The strip's arrow sits in the middle of the tool area's full height, which with the
    // browser shown reaches down behind it (rekordbox lays the area out as if it were hidden).
    let hidden_pad_bottom = ui.ctx().screen_rect().max.y - 28.0 + 5.0 - 30.0 - 2.0;
    tool_panel(app, ui, tool, (o.y + 34.0 + hidden_pad_bottom) / 2.0 - 0.25);
    // BROWSER tab.
    {
        let mut p = ui.painter().clone();
        p.set_clip_rect(Rect::from_min_max(full.min, pos2(full.max.x, full.max.y + 2.0)));
        tab(ui, &p, tab_r, "BROWSER", browser, 15.0, 0.25);
        let hit = Rect::from_min_max(tab_r.min, pos2(tab_r.max.x, tab_r.max.y.min(full.max.y)));
        app.edit.map.rects.insert("browser".into(), hit);
        let resp = ui.interact(hit, ui.id().with("edit_browser_tab"), Sense::click());
        let resp = if app.prefs.tooltips { resp.on_hover_text("Browser tab:\nShows/hides the browser.") } else { resp };
        if resp.clicked() {
            app.edit.queue.push(Cmd::Browser);
        }
    }
    drag_ghost(app, ui);
}

fn project_bar(app: &mut App, ui: &mut Ui, bar: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(bar, 0.0, BAR);
    let loaded = app.edit.project.is_some();
    let cy = bar.min.y + 15.0;
    // Project name and the recent projects.
    let combo = Rect::from_min_size(bar.min + vec2(18.0, 4.0), vec2(400.0, 22.0));
    let name = app.edit.project.as_ref().map(|p| p.name()).unwrap_or_else(|| "New Project".into());
    let g = kit::galley(ui, &name, caps(em(16.0)), WHITE, 0.0);
    p.with_clip_rect(Rect::from_min_max(combo.min, pos2(combo.max.x - 14.0, combo.max.y))).galley(pos2(combo.min.x + 6.1, cy - g.size().y / 2.0 - 0.5), g, WHITE);
    crate::icons_player::chevron_down(&p, pos2(combo.max.x - 4.25, cy + 0.5), 1.0, WHITE);
    let resp = ui.interact(combo, ui.id().with("edit_project"), Sense::click());
    app.edit.map.rects.insert("project".into(), combo);
    let resp = if app.prefs.tooltips { resp.on_hover_text("Recently used projects:\nOpens a recently used project.") } else { resp };
    crate::edit_project::prune_recent(&mut app.prefs.edit.recent);
    let recent = app.prefs.edit.recent.clone();
    crate::ui_browser::dropdown(ui, &resp, "edit_recent", |ui| {
        ui.set_min_width(380.0);
        if recent.is_empty() {
            ui.add_enabled(false, egui::Label::new("No recently used projects"));
        }
        for f in &recent {
            let label = std::path::Path::new(f).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if ui.selectable_label(false, label).on_hover_text(f.as_str()).clicked() {
                app.edit.queue.push(Cmd::OpenRecent(f.into()));
                ui.memory_mut(|m| m.close_popup());
            }
        }
        ui.separator();
        if ui.selectable_label(false, "Open project…").clicked() {
            app.edit.queue.push(Cmd::OpenProject);
            ui.memory_mut(|m| m.close_popup());
        }
    });
    // Transport, around the middle of the bar.
    let c = bar.min.x + bar.width() / 2.0;
    let y = bar.min.y;
    if icon_btn(app, ui, Rect::from_min_size(pos2(c - 27.0, y + 8.0), vec2(12.0, 14.0)), "head", "edit/head.svg", loaded, None, "The play position is moved to the beginning of the timeline.").clicked() {
        app.edit.queue.push(Cmd::Head);
    }
    let is_playing = loaded && playing();
    let (asset, pr) = if is_playing { ("edit/pause.svg", Rect::from_min_size(pos2(c + 10.0, y + 8.0), vec2(14.0, 14.0))) } else { ("edit/play.svg", Rect::from_min_size(pos2(c + 12.0, y + 8.0), vec2(10.0, 14.0))) };
    if icon_btn(app, ui, pr, "play", asset, loaded && !app.deck_options.disable_play, None, "Play/Pause\nPlays or pauses.").clicked() {
        app.edit.queue.push(Cmd::PlayPause);
    }
    p.rect_filled(Rect::from_min_size(pos2(c + 47.0, y + 10.0), vec2(1.0, 10.0)), 0.0, FRAME);
    let lit = app.prefs.edit.loop_mode.then_some(LOOP);
    if icon_btn(app, ui, Rect::from_min_size(pos2(c + 68.0, y + 7.79), vec2(23.0, 15.14)), "loop", "edit/loop.svg", true, lit, "Loop:\nLoops within the selected range if played in the ON state.").clicked() {
        app.edit.queue.push(Cmd::Loop);
    }
    p.rect_filled(Rect::from_min_size(pos2(c + 111.0, y + 10.0), vec2(1.0, 10.0)), 0.0, FRAME);
    p.text(pos2(c + 141.5, cy), Align2::CENTER_CENTER, "Q", rb(em(16.0)), hex(0xFF0000));
    p.text(pos2(c + 159.5, cy + 0.5), Align2::CENTER_CENTER, ":", rb(em(16.0)), WHITE);
    let q = Rect::from_min_size(pos2(c + 162.0, y + 6.0), vec2(100.0, 18.0));
    let cur = app.edit.project.as_ref().map(|p| p.session.quantize).unwrap_or(Quantize::Auto);
    p.text(pos2(q.min.x + 6.0, cy), Align2::LEFT_CENTER, cur.label(), caps(em(13.0)), WHITE);
    crate::icons_player::chevron_down(&p, pos2(q.max.x - 4.25, cy + 0.5), 1.0, WHITE);
    let qresp = ui.interact(q, ui.id().with("edit_quantize"), Sense::click());
    app.edit.map.rects.insert("quantize".into(), q);
    let qresp = if app.prefs.tooltips {
        qresp.on_hover_text("Quantize:\nSets the unit for selection.\nClick/drag operations of the enlarged waveform can be performed for every 4 beats when zooming out over a certain amount from the enlarged waveform if set to AUTO.")
    } else {
        qresp
    };
    crate::ui_browser::dropdown(ui, &qresp, "edit_quantize", |ui| {
        ui.set_min_width(100.0);
        for (i, v) in [Quantize::Auto, Quantize::One, Quantize::Four].into_iter().enumerate() {
            let r = ui.selectable_label(cur == v, v.label());
            app.edit.map.rects.insert(format!("quantize{i}"), r.rect);
            if r.clicked() {
                if let Some(p) = app.edit.project.as_mut() {
                    if p.session.quantize != v {
                        p.session.quantize = v;
                        p.session.extra_dirty = true;
                    }
                }
                ui.memory_mut(|m| m.close_popup());
            }
        }
    });
}

fn fmt_len(ms: f64) -> (String, String) {
    let t = (ms / 100.0).floor().max(0.0) as i64;
    (format!("{:02}:{:02}", t / 600, (t / 10) % 60), format!(".{}", t % 10))
}

fn player_panel(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    let w = r.width();
    let hinfo = 30.0;
    let loaded = app.edit.project.is_some();
    // Artwork; with a track loaded the pointer over it shows the eject button.
    let art = Rect::from_min_size(r.min + vec2(10.0, 10.0), vec2(58.0, 58.0));
    let art_path = app.edit.project.as_ref().map(|p| p.track.artwork.clone()).unwrap_or_default();
    let own = crate::ui_browser::artwork_texture(app, &art_path);
    match own.or_else(|| skin(app, "p0510_artwork50_.png")) {
        Some(t) => paint_tex(&p, &t, art, WHITE),
        None => {
            p.rect_filled(art, 0.0, hex(0x181818));
            crate::icons::vinyl(&p, art);
        }
    }
    if loaded {
        let resp = ui.interact(art, ui.id().with("edit_art"), Sense::click());
        app.edit.map.rects.insert("eject".into(), art);
        if resp.hovered() {
            p.rect_filled(art, 0.0, Color32::from_black_alpha(120));
            let er = Rect::from_center_size(art.center(), vec2(19.0, 19.0));
            match skin(app, "p10522_eject.png") {
                Some(t) => paint_tex(&p, &t, er, WHITE),
                None => {
                    p.add(egui::Shape::convex_polygon(vec![pos2(er.min.x, er.center().y + 2.0), pos2(er.center().x, er.min.y), pos2(er.max.x, er.center().y + 2.0)], WHITE, Stroke::NONE));
                    p.rect_filled(Rect::from_min_max(pos2(er.min.x, er.max.y - 4.0), er.max), 0.0, WHITE);
                }
            }
        }
        let resp = if app.prefs.tooltips { resp.on_hover_text("Artwork:\nClick it to unload the track.") } else { resp };
        if resp.clicked() {
            app.edit.queue.push(Cmd::Unload);
        }
    }
    // Title, time, key, BPM.
    let time_x = r.min.x + w - 182.0;
    let cy = r.min.y + hinfo / 2.0;
    let title_clip = Rect::from_min_max(pos2(r.min.x + 80.0, r.min.y), pos2(time_x - 8.0, r.min.y + hinfo));
    let (title, tcol, tip) = match &app.edit.project {
        Some(pr) => (pr.track.display_title(), WHITE, format!("Track Title: {}\nArtist: {}\nAlbum: {}", pr.track.display_title(), pr.track.artist, pr.track.album)),
        None => ("Not Loaded.".to_string(), hex(0x630000), String::new()),
    };
    if app.edit.loading.is_some() && !loaded {
        p.text(pos2(title_clip.min.x + 5.5, cy - 0.5), Align2::LEFT_CENTER, "Loading…", caps(em(16.0)), hex(0x969696));
    } else {
        // ("Not Loaded." is 1.5 pt wider in rekordbox than the plain face sets it.)
        let g = kit::galley(ui, &title, caps(em(16.0)), tcol, if loaded { 0.0 } else { 0.15 });
        p.with_clip_rect(title_clip).galley(pos2(title_clip.min.x + 4.7, cy - 0.5 - g.size().y / 2.0), g, tcol);
        if !tip.is_empty() && app.prefs.tooltips {
            ui.interact(title_clip, ui.id().with("edit_title"), Sense::hover()).on_hover_text(tip);
        }
    }
    let (len_ms, key, bpm) = match &app.edit.project {
        Some(pr) => (pr.session.cur.len_ms(), pr.track.key_label(app.prefs.key_alnum), format!("{:.2}", pr.session.cur.bpm)),
        None => (0.0, String::new(), String::new()),
    };
    let (big, small) = fmt_len(len_ms);
    let gb = kit::galley(ui, &big, rb(em(16.0)), WHITE, 0.0);
    let gs = kit::galley(ui, &small, rb(em(12.0)), WHITE, 0.0);
    let tx = time_x + (50.0 - (gb.size().x + gs.size().x)) / 2.0 + 1.75;
    let base = cy + gb.size().y / 2.0 + 0.5;
    let (bw, sh) = (gb.size().x, gs.size().y);
    p.galley(pos2(tx, base - gb.size().y), gb, WHITE);
    // (The tenths sit on the same baseline as the minutes and seconds.)
    p.galley(pos2(tx + bw, base - sh - 0.6), gs, WHITE);
    for x in [w - 123.0, w - 72.0] {
        p.rect_filled(Rect::from_min_size(r.min + vec2(x, 5.0), vec2(1.0, 20.0)), 0.0, FRAME);
    }
    if loaded {
        p.text(pos2(r.min.x + w - 97.0, cy - 0.5), Align2::CENTER_CENTER, key, rb(em(16.0)), WHITE);
        p.text(pos2(r.min.x + w - 38.0, cy - 0.5), Align2::CENTER_CENTER, bpm, rb(em(16.0)), WHITE);
    }
    if !loaded {
        app.edit.map.wave = Rect::NOTHING;
        app.edit.map.whole = Rect::NOTHING;
        drop_target(app, ui, r);
        return;
    }
    whole_waveform(app, ui, Rect::from_min_size(r.min + vec2(80.0, hinfo), vec2(w - 82.0, 47.0)));
    // The enlarged waveform between two 6 pt strips, 380 pt of content at most.
    let content = r.height().min(380.0);
    let top = r.min.y + hinfo + 47.0;
    let d = Rect::from_min_max(pos2(r.min.x, top + 6.0), pos2(r.max.x, r.min.y + content - 8.0));
    p.rect_filled(Rect::from_min_max(pos2(r.min.x, top), pos2(r.max.x, top + 6.0)), 0.0, BAR);
    p.rect_filled(Rect::from_min_max(pos2(r.min.x, d.max.y), pos2(r.max.x, d.max.y + 6.0)), 0.0, BAR);
    if d.height() > 30.0 {
        detail(app, ui, d);
    }
    drop_target(app, ui, r);
}

/// A track dragged from the browser, or a file from the Finder, dropped on the player loads.
fn drop_target(app: &mut App, ui: &mut Ui, r: Rect) {
    let over = ui.ctx().pointer_latest_pos().map(|pp| r.contains(pp)).unwrap_or(false);
    if let Some(ids) = app.drag.clone() {
        if over {
            ui.painter().rect_stroke(r.shrink(1.0), 0.0, st(1.0, WHITE), egui::StrokeKind::Inside);
            if ui.input(|i| i.pointer.any_released()) {
                if let Some(id) = ids.first() {
                    load_track(app, *id);
                }
            }
        }
    }
    let files: Vec<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if let Some(f) = files.first().filter(|_| over || app.screenshot.is_some()) {
        ui.ctx().input_mut(|i| i.raw.dropped_files.clear());
        let path = f.to_string_lossy().to_string();
        if f.extension().map(|e| e == crate::edit_project::EXT).unwrap_or(false) {
            app.edit.queue.push(Cmd::OpenRecent(f.clone()));
        } else if let Some(id) = app.db.track_by_path(&path) {
            load_track(app, id);
        } else if crate::audio::has_audio_extension(f) {
            // Not in the collection yet: imported first, then loaded.
            app.load_after_import = Some(path);
            app.import_paths(vec![f.clone()], None);
        }
    }
}

// ---------------------------------------------------------------------------- waveforms

/// The loudest waveform frame between two song positions (ms).
fn wave_max(w: &Waveform, ms0: f64, ms1: f64) -> Option<[u8; 8]> {
    let n = w.frames.len();
    if ms1 < 0.0 || n == 0 {
        return None;
    }
    let a = (ms0.max(0.0) * 0.001 * WAVE_FPS) as usize;
    if a >= n {
        return None;
    }
    let b = ((ms1 * 0.001 * WAVE_FPS).ceil() as usize).clamp(a + 1, n);
    let mut m = [0u8; 8];
    for f in &w.frames[a..b] {
        for k in 0..8 {
            m[k] = m[k].max(f[k]);
        }
    }
    Some(m)
}

/// Bars of one waveform column in the chosen waveform colours: (share of half the height,
/// colour), drawn in order.
fn column_bars(mode: u8, m: &[u8; 8]) -> Vec<(f32, Color32)> {
    if mode >= 2 {
        let [lo, mi, hi] = crate::export::anlz::three_band_detail(m);
        let s = |v: u8| (v as f32 / 127.0).min(1.0) * crate::ui_player::DETAIL_FULL_SCALE;
        vec![(s(lo), WAVE_LOW), (s(mi), WAVE_MID), (s(hi), WAVE_HIGH)]
    } else {
        crate::ui_player::wave_bars(mode, Waveform::disp(m)).into_iter().map(|(h, c)| (h * crate::ui_player::DETAIL_FULL_SCALE, c)).collect()
    }
}

/// Draw the score's waveform over `r`: timeline beats `b0`.. at `ppb` points a beat, each
/// column read from the song where the position score points.
fn score_wave(p: &egui::Painter, r: Rect, s: &ScoreSet, w: &Waveform, mode: u8, b0: f64, ppb: f64, scale: f32, skip_muted: bool) {
    let mut bars = kit::Bars::new();
    let (mid, half) = (r.center().y, r.height() / 2.0);
    let len = s.len();
    let db = 1.0 / ppb;
    let mut x = r.min.x;
    while x < r.max.x {
        let b = b0 + (x - r.min.x) as f64 * db;
        x += 1.0;
        if b < 0.0 {
            continue;
        }
        if b >= len {
            break;
        }
        if skip_muted && s.muted_at(b) {
            continue;
        }
        let Some(sb) = s.song_beat(b) else { continue };
        let (m0, m1) = (s.song_grid.ppq_to_ms(sb), s.song_grid.ppq_to_ms(sb + db));
        if let Some(m) = wave_max(w, m0, m1) {
            for (h, c) in column_bars(mode, &m) {
                let hh = (h * scale).min(1.0) * half;
                bars.column(x - 0.5, mid - hh, mid + hh, c);
            }
        }
    }
    bars.paint(p);
}

fn whole_waveform(app: &mut App, ui: &mut Ui, r: Rect) {
    let Some(pr) = app.edit.project.as_ref() else { return };
    let s = pr.session.cur.clone();
    let sel = pr.session.selection;
    let p = ui.painter().with_clip_rect(r);
    app.edit.map.whole = r;
    let len = s.len().max(1.0);
    let ppb = r.width() as f64 / len;
    p.rect_filled(r, 0.0, hex(0x4B4B4B));
    let mut k = 32.0;
    while k < len {
        let x = r.min.x + (k * ppb) as f32;
        p.rect_filled(Rect::from_min_max(pos2(x, r.min.y), pos2(x + 1.0, r.max.y)), 0.0, BAR);
        k += 32.0;
    }
    if let Some(w) = &pr.wave {
        score_wave(&p, r.shrink2(vec2(0.0, 2.0)), &s, w, app.prefs.wave_color, 0.0, ppb, 1.0, true);
    }
    let x_of = |b: f64| r.min.x + (b * ppb) as f32;
    // The part the enlarged waveform shows, and the selection.
    if app.edit.map.wave.width() > 0.0 {
        let vis = app.edit.map.wave.width() as f64 / app.edit.pt_per_beat() as f64;
        let v = Rect::from_min_max(pos2(x_of(app.edit.view_start.max(0.0)), r.min.y), pos2(x_of((app.edit.view_start + vis).min(len)).max(x_of(app.edit.view_start.max(0.0)) + 2.0), r.max.y));
        p.rect_stroke(v, 0.0, st(1.0, CURSOR), egui::StrokeKind::Inside);
    }
    match sel {
        Selection::Range(a, b) => {
            for x in [x_of(a), x_of(b)] {
                p.rect_filled(Rect::from_min_max(pos2(x, r.min.y), pos2(x + 1.0, r.max.y)), 0.0, CURSOR);
            }
        }
        Selection::Point(a) => {
            p.rect_filled(Rect::from_min_max(pos2(x_of(a), r.min.y), pos2(x_of(a) + 1.0, r.max.y)), 0.0, CURSOR);
        }
        Selection::None => {}
    }
    // Play cursor: 3 pt blue on 4 pt black, 43 pt tall.
    let cx = x_of(play_pos());
    p.rect_filled(Rect::from_min_max(pos2(cx - 2.0, r.min.y + 2.0), pos2(cx + 2.0, r.max.y - 2.0)), 0.0, Color32::BLACK);
    p.rect_filled(Rect::from_min_max(pos2(cx - 1.5, r.min.y + 2.0), pos2(cx + 1.5, r.max.y - 2.0)), 0.0, CURSOR);
    // Click or drag: the play position and the enlarged view go there.
    let resp = ui.interact(r, ui.id().with("edit_whole"), Sense::click_and_drag());
    if let Some(pp) = resp.hover_pos() {
        let b = ((pp.x - r.min.x) as f64 / ppb).clamp(0.0, len);
        let (bar, beat) = s.bar_beat(b.floor() as i64);
        let (m, t) = fmt_len(b * s.beat_ms());
        let text = format!("{m}{t}   {bar}.{beat}");
        let g = kit::galley(ui, &text, rb(em(12.0)), WHITE, 0.0);
        let tx = (pp.x + 6.0).min(r.max.x - g.size().x - 4.0);
        let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("edit_hover")));
        lp.rect_filled(Rect::from_min_size(pos2(tx - 3.0, r.min.y - 15.0), g.size() + vec2(6.0, 2.0)), 2.0, Color32::from_black_alpha(200));
        lp.galley(pos2(tx, r.min.y - 14.0), g, WHITE);
    }
    if resp.clicked() || resp.dragged() {
        if let Some(pp) = resp.interact_pointer_pos() {
            let b = ((pp.x - r.min.x) as f64 / ppb).clamp(0.0, len);
            seek(app, b);
            let vis = app.edit.map.wave.width().max(1.0) as f64 / app.edit.pt_per_beat() as f64;
            app.edit.view_start = b - vis / 2.0;
        }
    }
}

/// Keep the view inside the timeline (a little room is left before beat 0 and after the end).
fn clamp_view(st: &mut EditState, len: f64, width: f32) {
    let vis = width as f64 / st.pt_per_beat() as f64;
    let lo = -vis * 0.02;
    let hi = (len - vis * 0.5).max(lo);
    st.view_start = st.view_start.clamp(lo, hi);
}

fn detail(app: &mut App, ui: &mut Ui, d: Rect) {
    let Some(pr) = app.edit.project.as_ref() else { return };
    let s = pr.session.cur.clone();
    let sel = pr.session.selection;
    let wave_data = pr.wave.clone();
    let len = s.len();
    let ruler = Rect::from_min_max(d.min, pos2(d.max.x, d.min.y + 16.0));
    let wave = Rect::from_min_max(pos2(d.min.x, ruler.max.y), d.max);
    app.edit.map.wave = wave;
    let pos = play_pos();
    let is_playing = playing();
    // While playing the view turns the page when the cursor leaves it.
    {
        let vis = wave.width() as f64 / app.edit.pt_per_beat() as f64;
        if is_playing && app.edit.gesture == Gesture::None && (pos < app.edit.view_start || pos > app.edit.view_start + vis) {
            app.edit.view_start = pos - vis * 0.1;
        }
    }
    clamp_view(&mut app.edit, len, wave.width());
    let ppb = app.edit.pt_per_beat() as f64;
    let v0 = app.edit.view_start;
    let x_of = |b: f64| wave.min.x + ((b - v0) * ppb) as f32;
    let b_of = |x: f32| v0 + (x - wave.min.x) as f64 / ppb;
    let p = ui.painter().with_clip_rect(d.intersect(ui.clip_rect()));
    p.rect_filled(d, 0.0, Color32::BLACK);
    let (x0, x1) = (x_of(0.0).max(d.min.x), x_of(len).min(d.max.x));
    p.rect_filled(Rect::from_min_max(pos2(x0, ruler.min.y), pos2(x1, ruler.max.y)), 0.0, FRAME);
    p.rect_filled(Rect::from_min_max(pos2(x0, wave.min.y), pos2(x1, wave.max.y)), 0.0, BAR);
    // Beat lines (bars stronger), thinned out as the view zooms out; "bar.beat" on the ruler.
    let first = b_of(d.min.x).floor().max(0.0) as i64;
    let last = b_of(d.max.x).ceil().min(len) as i64;
    let line_step: i64 = if ppb >= 5.0 { 1 } else if ppb * 4.0 >= 5.0 { 4 } else if ppb * 16.0 >= 5.0 { 16 } else { 64 };
    let label_step: i64 = [1i64, 4, 8, 16, 32, 64, 128, 256].into_iter().find(|n| *n as f64 * ppb >= 38.0).unwrap_or(512);
    let off = s.bar_offset.rem_euclid(4) as i64;
    let mut lines = kit::Bars::new();
    for k in first..=last {
        let rel = k - off;
        let is_bar = rel.rem_euclid(4) == 0;
        if (line_step == 1 || rel.rem_euclid(line_step) == 0) && k as f64 <= len {
            let x = (x_of(k as f64) - 0.5).round() + 0.5;
            lines.rect(Rect::from_min_max(pos2(x - 0.5, if is_bar { ruler.min.y + 10.0 } else { ruler.min.y + 13.0 }), pos2(x + 0.5, wave.max.y)), if is_bar { hex(0x4B4B4B) } else { FRAME });
        }
        if rel.rem_euclid(label_step) == 0 && (label_step == 1 || is_bar) && (k as f64) < len {
            let (bar, beat) = s.bar_beat(k);
            p.text(pos2(x_of(k as f64) + 3.0, ruler.min.y + 7.0), Align2::LEFT_CENTER, format!("{bar}.{beat}"), rb(em(12.0)), hex(0xAFAFAF));
        }
    }
    lines.paint(&p);
    if let Some(w) = &wave_data {
        score_wave(&p, wave, &s, w, app.prefs.wave_color, v0, ppb, 1.0, false);
    }
    // Muted ranges.
    for m in s.mute.iter().filter(|m| m.muted) {
        let (a, b) = (x_of(m.start).max(wave.min.x), x_of(m.end).min(wave.max.x));
        if b > a {
            p.rect_filled(Rect::from_min_max(pos2(a, wave.min.y), pos2(b, wave.max.y)), 0.0, Color32::from_rgba_unmultiplied(0x70, 0x70, 0x70, 204));
        }
    }
    // Hot cues: a line with its letter on the ruler; loops shade their length.
    for c in &s.prepared.hot {
        let x = x_of(c.pos_in);
        if x < d.min.x - 20.0 || x > d.max.x + 2.0 {
            continue;
        }
        let col = hotcue_color(if c.appended.color == 0 { hotcue_default_color(c.cue_id, c.is_loop()) } else { c.appended.color });
        if c.is_loop() {
            p.rect_filled(Rect::from_min_max(pos2(x, wave.max.y - 5.0), pos2(x_of(c.pos_out), wave.max.y)), 0.0, col.gamma_multiply(0.6));
        }
        p.rect_filled(Rect::from_min_max(pos2(x - 0.5, ruler.min.y), pos2(x + 0.5, wave.max.y)), 0.0, col);
        let flag = Rect::from_min_size(pos2(x, ruler.min.y + 1.0), vec2(12.0, 12.0));
        p.rect_filled(flag, 1.0, col);
        let letter = (b'A' + (c.cue_id.clamp(0, 25)) as u8) as char;
        p.text(flag.center(), Align2::CENTER_CENTER, letter, bold(8.5), Color32::BLACK);
    }
    for c in &s.prepared.memory {
        let x = x_of(c.pos_in);
        if x >= d.min.x - 6.0 && x <= d.max.x + 6.0 {
            crate::icons_player::mem_tri(&p, pos2(x, wave.min.y), 1.0, hex(0xFF0000));
        }
    }
    // Selection: a range is shaded between two lines with its length written in it; a point
    // is one line.
    match sel {
        Selection::Range(a, b) => {
            let (xa, xb) = (x_of(a), x_of(b));
            p.rect_filled(Rect::from_min_max(pos2(xa, wave.min.y), pos2(xb, wave.max.y)), 0.0, Color32::from_rgba_unmultiplied(0x40, 0xA0, 0xFF, 76));
            for x in [xa, xb] {
                p.rect_filled(Rect::from_min_max(pos2(x - 0.5, ruler.min.y), pos2(x + 0.5, wave.max.y)), 0.0, SEL);
            }
            let (bars, beats, time) = readout(b - a, s.bpm);
            let cx = ((xa.max(wave.min.x) + xb.min(wave.max.x)) / 2.0).clamp(wave.min.x + 50.0, wave.max.x - 50.0);
            let mut y = wave.center().y - 26.0;
            for (text, size) in [(bars, 18.0), (beats, 14.0), (time, 14.0)] {
                // White with a 1 pt black outline.
                for dxy in [vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0)] {
                    p.text(pos2(cx, y) + dxy, Align2::CENTER_TOP, &text, rb(em(size)), Color32::BLACK);
                }
                p.text(pos2(cx, y), Align2::CENTER_TOP, &text, rb(em(size)), WHITE);
                y += size + 1.0;
            }
        }
        Selection::Point(a) => {
            let x = x_of(a);
            p.rect_filled(Rect::from_min_max(pos2(x - 0.5, ruler.min.y), pos2(x + 0.5, wave.max.y)), 0.0, SEL);
            p.add(egui::Shape::convex_polygon(vec![pos2(x - 4.0, ruler.min.y), pos2(x + 4.0, ruler.min.y), pos2(x, ruler.min.y + 6.0)], SEL, Stroke::NONE));
        }
        Selection::None => {}
    }
    // Play cursor.
    {
        let x = x_of(pos);
        p.rect_filled(Rect::from_min_max(pos2(x - 1.5, ruler.min.y), pos2(x + 1.5, wave.max.y)), 0.0, Color32::BLACK);
        p.rect_filled(Rect::from_min_max(pos2(x - 0.5, ruler.min.y), pos2(x + 0.5, wave.max.y)), 0.0, WHITE);
    }
    // BPM, top left: "126." large and "00" small.
    {
        let b = Rect::from_min_size(d.min + vec2(0.0, 31.0), vec2(56.0, 26.0));
        p.rect_filled(b, 0.0, Color32::from_black_alpha(153));
        let whole = format!("{}.", s.bpm.floor() as i64);
        let frac = format!("{:02}", ((s.bpm - s.bpm.floor()) * 100.0).round() as i64 % 100);
        let g = kit::galley(ui, &whole, rb(em(18.0)), WHITE, 0.0);
        let gw = g.size().x;
        p.galley(pos2(b.min.x + 5.0, b.center().y - g.size().y / 2.0), g, WHITE);
        p.text(pos2(b.min.x + 5.0 + gw, b.center().y + 2.0), Align2::LEFT_CENTER, frac, rb(em(13.0)), WHITE);
    }
    wave_input(app, ui, d, ruler, wave, &s, v0, ppb);
    zoom_control(app, ui, Rect::from_min_size(pos2(d.min.x, d.max.y - 38.0), vec2(116.0, 30.0)));
}

/// Pointer on the enlarged waveform: click, drag, shift-click, the context menu, the ruler,
/// the wheel.
#[allow(clippy::too_many_arguments)]
fn wave_input(app: &mut App, ui: &mut Ui, d: Rect, ruler: Rect, wave: Rect, s: &ScoreSet, v0: f64, ppb: f64) {
    let len = s.len();
    let b_of = |x: f32| v0 + (x - wave.min.x) as f64 / ppb;
    let unit = app.edit.unit();
    let ptr = ui.ctx().pointer_latest_pos();
    let shift = ui.input(|i| i.modifiers.shift);
    // Ruler: drag to scroll.
    let rresp = ui.interact(ruler, ui.id().with("edit_ruler"), Sense::drag());
    if rresp.drag_started() {
        if let Some(pp) = ui.input(|i| i.pointer.press_origin()) {
            app.edit.gesture = Gesture::Scroll { start: v0, x: pp.x };
        }
    }
    if let (Gesture::Scroll { start, x }, Some(pp)) = (app.edit.gesture.clone(), ptr) {
        if rresp.dragged() {
            app.edit.view_start = start - (pp.x - x) as f64 / ppb;
        }
        if rresp.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
            app.edit.gesture = Gesture::None;
        }
    }
    let resp = ui.interact(wave, ui.id().with("edit_wave"), Sense::click_and_drag());
    if resp.clicked() {
        if let Some(pp) = resp.interact_pointer_pos() {
            let b = b_of(pp.x);
            if let Some(pr) = app.edit.project.as_mut() {
                if shift {
                    pr.session.extend_to(b, unit);
                } else {
                    pr.session.select_point(b, unit);
                    let q = pr.session.selection.point().unwrap_or(0.0);
                    seek(app, q);
                }
            }
            sync_engine(app);
        }
    }
    if resp.drag_started_by(egui::PointerButton::Primary) {
        if let Some(pp) = ui.input(|i| i.pointer.press_origin()) {
            let anchor = b_of(pp.x);
            let inside = app.edit.project.as_ref().and_then(|p| p.session.selection.range()).filter(|(a, b)| anchor >= *a && anchor < *b);
            app.edit.gesture = Gesture::Select { anchor, from: pp, inside, moved: false };
        }
    }
    if let (Gesture::Select { anchor, from, inside, .. }, Some(pp)) = (app.edit.gesture.clone(), ptr) {
        if resp.dragged() {
            let outside = pp.y < d.min.y - 6.0 || pp.y > d.max.y + 6.0 || pp.x > wave.max.x + 6.0;
            if let (Some((a, b)), true) = (inside, outside) {
                // The selected range leaves the waveform: it is being carried (to the palette).
                if let Some(pr) = app.edit.project.as_mut() {
                    pr.session.selection = Selection::Range(a, b);
                }
                app.edit.gesture = Gesture::DragSelection;
            } else {
                if let Some(pr) = app.edit.project.as_mut() {
                    pr.session.select_range(anchor, b_of(pp.x.clamp(wave.min.x, wave.max.x)), unit);
                }
                app.edit.gesture = Gesture::Select { anchor, from, inside, moved: true };
                // Dragging past an edge scrolls.
                if pp.x > wave.max.x - 4.0 {
                    app.edit.view_start += 12.0 / ppb;
                } else if pp.x < wave.min.x + 4.0 {
                    app.edit.view_start -= 12.0 / ppb;
                }
                ui.ctx().request_repaint();
            }
            sync_engine(app);
        }
    }
    if resp.drag_stopped() {
        if matches!(app.edit.gesture, Gesture::Select { .. }) {
            app.edit.gesture = Gesture::None;
            // rekordbox moves the play position to the start of what was selected when it
            // is not playing.
            if !playing() {
                if let Some(a) = app.edit.project.as_ref().and_then(|p| p.session.selection.point()) {
                    seek(app, a);
                }
            }
        }
    }
    // Context menu at the clicked beat.
    let ctx_id = egui::Id::new("edit_ctx_beat");
    if resp.secondary_clicked() {
        if let Some(pp) = resp.interact_pointer_pos() {
            ui.ctx().data_mut(|dd| dd.insert_temp(ctx_id, b_of(pp.x)));
        }
    }
    resp.context_menu(|ui| {
        let at: f64 = ui.ctx().data(|dd| dd.get_temp(ctx_id)).unwrap_or(0.0);
        let mut pick: Option<Cmd> = None;
        let mut select_range = false;
        if ui.button("Select the range").clicked() {
            select_range = true;
        }
        for (label, cmd) in [
            ("Add to palette", Cmd::AddToPalette),
            ("Clone", Cmd::Edit(EditCmd::Clone)),
            ("Copy", Cmd::Edit(EditCmd::Copy)),
            ("Cut", Cmd::Edit(EditCmd::Cut)),
            ("Paste", Cmd::Edit(EditCmd::Paste)),
            ("Insert", Cmd::Edit(EditCmd::Insert)),
            ("Delete", Cmd::Edit(EditCmd::Delete)),
            ("Clear", Cmd::Edit(EditCmd::Clear)),
            ("Undo", Cmd::Edit(EditCmd::Undo)),
            ("Redo", Cmd::Edit(EditCmd::Redo)),
        ] {
            if label == "Undo" {
                ui.separator();
            }
            if ui.add_enabled(can(app, &cmd), egui::Button::new(label)).clicked() {
                pick = Some(cmd);
            }
        }
        if select_range {
            // From the selected point to the click; with nothing to start from, the click
            // becomes the point (and the play position, when not playing).
            if let Some(pr) = app.edit.project.as_mut() {
                match pr.session.selection {
                    Selection::Point(a) if (at - a).abs() >= 0.5 => pr.session.select_range(a, at.max(a), unit),
                    _ => {
                        pr.session.select_point(at, unit);
                        let q = pr.session.selection.point().unwrap_or(0.0);
                        if !playing() {
                            seek(app, q);
                        }
                    }
                }
            }
            sync_engine(app);
            ui.close_menu();
        }
        if let Some(c) = pick {
            app.edit.queue.push(c);
            ui.close_menu();
        }
    });
    // A palette entry dropped here is inserted at that beat.
    if let (Gesture::DragPalette(id), Some(pp)) = (app.edit.gesture.clone(), ptr) {
        if d.contains(pp) {
            let x = (wave.min.x + ((b_of(pp.x).round().clamp(0.0, len) - v0) * ppb) as f32 - 0.5).round() + 0.5;
            ui.painter().with_clip_rect(d).rect_filled(Rect::from_min_max(pos2(x - 1.0, d.min.y), pos2(x + 1.0, d.max.y)), 0.0, WHITE);
            if ui.input(|i| i.pointer.any_released()) {
                if let Some(pr) = app.edit.project.as_mut() {
                    pr.session.insert_palette(id, b_of(pp.x));
                }
                sync_engine(app);
            }
        }
    }
    // Wheel: sideways scrolls, up and down zooms (as does a pinch), around the pointer.
    if let Some(pp) = ptr.filter(|pp| d.contains(*pp)) {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        if scroll.x.abs() > scroll.y.abs() && scroll.x != 0.0 {
            app.edit.view_start -= scroll.x as f64 / ppb;
        } else if scroll.y != 0.0 || pinch != 1.0 {
            let acc_id = egui::Id::new("edit_wheel_acc");
            let mut acc: f32 = ui.ctx().data(|dd| dd.get_temp(acc_id)).unwrap_or(0.0);
            acc += scroll.y + (pinch - 1.0) * 200.0;
            let steps = (acc / 40.0).trunc();
            if steps != 0.0 {
                acc -= steps * 40.0;
                let z = (app.edit.zoom as i32 - steps as i32).clamp(1, 15) as usize;
                app.edit.set_zoom(z, pp.x - wave.min.x);
            }
            ui.ctx().data_mut(|dd| dd.insert_temp(acc_id, acc));
        }
    }
}

fn zoom_control(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    let open = app.edit.zoom_ctrl_open;
    let arrow = Rect::from_min_size(pos2(r.min.x + if open { 90.0 } else { 0.0 }, r.min.y), vec2(26.0, 30.0));
    p.rect_filled(if open { r } else { arrow }, 0.0, Color32::from_black_alpha(153));
    if open {
        if icon_btn(app, ui, Rect::from_min_size(r.min + vec2(10.0, 7.5), vec2(15.0, 15.0)), "zoom_in", "zoomIn.svg", app.edit.zoom > 1, None, "Zoom In").clicked() {
            app.edit.queue.push(Cmd::ZoomIn);
        }
        let rst = Rect::from_min_size(r.min + vec2(35.0, 11.0), vec2(19.0, 8.0));
        let resp = ui.interact(rst.expand(4.0), ui.id().with("edit_rst"), Sense::click());
        p.text(rst.center(), Align2::CENTER_CENTER, "RST", caps(em(10.0)), if resp.is_pointer_button_down_on() { ICON_DOWN } else if resp.hovered() { ICON_OVER } else { ICON });
        app.edit.map.rects.insert("zoom_rst".into(), rst);
        if resp.on_hover_text("Reset Zoom").clicked() {
            app.edit.queue.push(Cmd::ZoomReset);
        }
        if icon_btn(app, ui, Rect::from_min_size(r.min + vec2(64.0, 7.5), vec2(15.0, 15.0)), "zoom_out", "zoomOut.svg", app.edit.zoom < 15, None, "Zoom Out").clicked() {
            app.edit.queue.push(Cmd::ZoomOut);
        }
    }
    let ar = Rect::from_center_size(arrow.center(), vec2(10.0, 6.0));
    let resp = ui.interact(arrow, ui.id().with("edit_zoom_fold"), Sense::click());
    let col = if resp.hovered() { ICON_OVER } else { hex(0x646464) };
    match skin(app, if open { "downArrow.svg" } else { "upArrow.svg" }) {
        Some(t) => paint_tex(&p, &t, ar, col),
        None => {
            let (a, b) = if open { (ar.max.y, ar.min.y) } else { (ar.min.y, ar.max.y) };
            p.add(egui::Shape::line(vec![pos2(ar.min.x, a), pos2(ar.center().x, b), pos2(ar.max.x, a)], st(1.5, col)));
        }
    }
    if resp.clicked() {
        app.edit.zoom_ctrl_open = !open;
    }
}

// ---------------------------------------------------------------------------- action pads

fn pad(app: &mut App, ui: &mut Ui, r: Rect, name: &str, asset: &str, label: &str, cmd: Cmd, tip: &str) {
    let enabled = can(app, &cmd);
    let resp = ui.interact(r, ui.id().with(("edit_pad", name)), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter().clone();
    let dim = if enabled { WHITE } else { WHITE.gamma_multiply(0.4) };
    match skin(app, asset) {
        Some(t) => paint_tex(&p, &t, r, dim),
        None => {
            // No rekordbox to read the pad from: the same tile, with the caption only.
            p.rect_filled(r, 0.0, hex(0x191919).gamma_multiply(if enabled { 1.0 } else { 0.4 }));
            p.rect_filled(r.shrink(1.0), 0.0, FRAME);
            p.rect_filled(Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 38.0), r.max - vec2(1.0, 1.0)), 0.0, hex(0x3C3C3C).gamma_multiply(if enabled { 1.0 } else { 0.4 }));
            p.text(pos2(r.center().x, r.min.y + 52.5), Align2::CENTER_CENTER, label, rb(11.0), dim);
        }
    }
    if enabled && resp.is_pointer_button_down_on() {
        p.rect_filled(r, 0.0, Color32::from_black_alpha(140));
    } else if enabled && resp.hovered() {
        p.rect_filled(r, 0.0, Color32::from_white_alpha(26));
    }
    app.edit.map.rects.insert(name.to_string(), r);
    let resp = if app.prefs.tooltips { resp.on_hover_text(tip) } else { resp };
    if resp.clicked() {
        app.edit.queue.push(cmd);
    }
}

fn pads(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    let left = Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y), pos2(r.min.x + 741.0_f32.min(r.width() / 2.0 - 1.0), r.max.y));
    let right = Rect::from_min_max(pos2(left.max.x + 1.0, r.min.y), pos2(r.max.x - 1.0, r.max.y));
    let tab_w = left.width() / 2.0;
    for half in [left, right] {
        p.rect_filled(Rect::from_min_max(pos2(half.min.x, half.min.y + 30.0), half.max), 0.0, FRAME);
    }
    let sel_tab = app.prefs.edit.pad_tab.min(1);
    let tabs = [("BEAT SELECT", "Action pad (beat select panel):\nSelects the range by beat units from the selected position."), ("SELECT", "Action pad (select panel):\nAdjusts the selection range.")];
    // The tab at the back is drawn first; the one in front lies over it.
    for pass in 0..2 {
        for (i, (name, tip)) in tabs.iter().enumerate() {
            let front = i as u8 == sel_tab;
            if front != (pass == 1) {
                continue;
            }
            let tr = Rect::from_min_size(pos2(left.min.x + tab_w * i as f32, left.min.y), vec2(tab_w, 30.0));
            tab(ui, &p, tr, name, front, 15.0, 0.5);
            let resp = ui.interact(tr, ui.id().with(("edit_padtab", i)), Sense::click());
            app.edit.map.rects.insert(format!("tab_{}", if i == 0 { "beat" } else { "select" }), tr);
            let resp = if app.prefs.tooltips { resp.on_hover_text(*tip) } else { resp };
            if resp.clicked() {
                app.prefs.edit.pad_tab = i as u8;
                app.save_prefs();
            }
        }
    }
    let etab = Rect::from_min_size(right.min, vec2(tab_w, 30.0));
    tab(ui, &p, etab, "EDIT", true, 15.0, 0.5);
    if app.prefs.tooltips {
        ui.interact(etab, ui.id().with("edit_padtab_edit"), Sense::hover()).on_hover_text("Action pad (edit panel):\nEdits the selected range.");
    }
    let body = |half: Rect| Rect::from_min_max(pos2(half.min.x, half.min.y + 30.0), half.max);
    let lb = body(left);
    if sel_tab == 0 {
        let o = pos2((lb.center().x - 219.0).round(), (lb.center().y - 72.0).floor());
        for (i, n) in BEAT_SELECT.iter().enumerate() {
            let pr = Rect::from_min_size(o + vec2((i % 4) as f32 * 112.0, (i / 4) as f32 * 77.0), vec2(102.0, 67.0));
            let tip = format!("{n} Beat{}:\nSelects the range by {n} beat{} from the selected position.", if *n == 1 { "" } else { "s" }, if *n == 1 { "" } else { "s" });
            pad(app, ui, pr, &format!("beat{n}"), &format!("edit/{n}.png"), &format!("{n} BEAT"), Cmd::Select(SelectCmd::ByBeat(*n)), &tip);
        }
    } else {
        let o = pos2((lb.center().x - 221.0).round(), (lb.center().y - 72.0).floor());
        for (i, (name, asset, label, cmd, tip)) in [
            ("half", "edit/half.png", "HALF", SelectCmd::Half, "Half:\nThe selection range length is set to half the length."),
            ("double", "edit/double.png", "DOUBLE", SelectCmd::Double, "Double:\nThe selection range length is set to twice the length."),
            ("cancel", "edit/cancel.png", "CANCEL", SelectCmd::Cancel, "Cancel:\nCancels the selected range."),
        ]
        .into_iter()
        .enumerate()
        {
            pad(app, ui, Rect::from_min_size(o + vec2(i as f32 * 113.0, 0.0), vec2(103.0, 67.0)), name, asset, label, Cmd::Select(cmd), tip);
        }
    }
    let rb_ = body(right);
    let o = pos2((rb_.center().x - 219.0).floor(), (rb_.center().y - 72.0).floor());
    for (i, (name, asset, label, cmd, tip)) in [
        ("clone", "edit/duplicate.png", "CLONE", EditCmd::Clone, "Clone:\nClones the selected range from the ending position of the selected range."),
        ("copy", "edit/copy.png", "COPY", EditCmd::Copy, "Copy:\nCopies the selected range."),
        ("paste", "edit/paste.png", "PASTE", EditCmd::Paste, "Paste:\nPastes from the selected position."),
        ("insert", "edit/insert.png", "INSERT", EditCmd::Insert, "Insert:\nInserts to the selected position."),
        ("delete", "edit/delete.png", "DELETE", EditCmd::Delete, "Delete:\nDeletes the selected range."),
        ("clear", "edit/clear.png", "CLEAR", EditCmd::Clear, "Clear:\nMutes the selected range."),
        ("undo", "edit/undo.png", "UNDO", EditCmd::Undo, "Undo:\nUndoes editing."),
        ("redo", "edit/redo.png", "REDO", EditCmd::Redo, "Redo:\nRedoes editing."),
    ]
    .into_iter()
    .enumerate()
    {
        pad(app, ui, Rect::from_min_size(o + vec2((i % 4) as f32 * 112.0, (i / 4) as f32 * 77.0), vec2(102.0, 67.0)), name, asset, label, Cmd::Edit(cmd), tip);
    }
}

// ---------------------------------------------------------------------------- tool area

fn tool_panel(app: &mut App, ui: &mut Ui, r: Rect, glyph_y: f32) {
    let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    let open = app.prefs.edit.show_tool;
    // The collapse / expand strip.
    let strip = Rect::from_min_max(r.min, pos2(r.min.x + 22.0, r.max.y));
    let sresp = ui.interact(strip, ui.id().with("edit_tool_strip"), Sense::click());
    p.rect_filled(strip, 0.0, FRAME);
    p.text(pos2(strip.center().x, glyph_y), Align2::CENTER_CENTER, if open { ">" } else { "<" }, rb(em(13.0)), if sresp.hovered() { WHITE } else { hex(0xC8C8C8) });
    app.edit.map.rects.insert("tool_strip".into(), strip);
    let sresp = if app.prefs.tooltips { sresp.on_hover_text("Shows/hides the tool area.") } else { sresp };
    if sresp.clicked() {
        app.prefs.edit.show_tool = !open;
        app.save_prefs();
    }
    if !open {
        return;
    }
    // 2 pt frame along the top and the right.
    p.rect_filled(Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 2.0)), 0.0, FRAME);
    p.rect_filled(Rect::from_min_max(pos2(r.max.x - 2.0, r.min.y), r.max), 0.0, FRAME);
    let area = Rect::from_min_max(r.min + vec2(24.0, 4.0), pos2(r.min.x + 316.0, r.max.y));
    let tab_r = Rect::from_min_size(area.min, vec2(146.0, 30.0));
    tab(ui, &p, tab_r, "PALETTE", true, 15.0, 0.5);
    if app.prefs.tooltips {
        ui.interact(tab_r, ui.id().with("edit_palette_tab"), Sense::hover()).on_hover_text(
            "Palette panel:\nDragging & dropping the selected range to the palette enables you to place waveforms of the selected range on the palette.\nWaveforms placed on the palette can be inserted in enlarged waveforms by dragging & dropping.",
        );
    }
    let body = Rect::from_min_max(pos2(area.min.x, area.min.y + 30.0), area.max);
    p.rect_filled(body, 0.0, FRAME);
    let count = app.edit.project.as_ref().map(|p| p.session.palettes.len()).unwrap_or(0);
    let del = app.edit.palette_delete && count > 0;
    let trash = Rect::from_min_size(pos2(r.max.x - 34.0, r.min.y + 9.0), vec2(14.0, 17.0));
    if icon_btn(app, ui, trash, "trash", "edit/trashBox.svg", count > 0, del.then_some(BLUE), "Show/hide the palette delete button:\nShows/hides the delete button of each waveform.").clicked() {
        app.edit.palette_delete = !del;
    }
    // Rows, then the "+" button; scrolls when there are more than fit.
    let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    cui.set_clip_rect(body.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical().id_salt("edit_palette").auto_shrink([false, false]).show(&mut cui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.add_space(2.0);
        let ids: Vec<u32> = app.edit.project.as_ref().map(|p| p.session.palettes.iter().map(|c| c.id).collect()).unwrap_or_default();
        for (i, id) in ids.into_iter().enumerate() {
            let (row, _) = ui.allocate_exact_size(vec2(body.width(), 82.0), Sense::hover());
            palette_row(app, ui, Rect::from_min_size(row.min + vec2(2.0, 0.0), vec2(body.width() - 4.0, 80.0)), id, i, del);
        }
        let (row, resp) = ui.allocate_exact_size(vec2(body.width(), 34.0), Sense::click());
        let add = Rect::from_min_size(row.min + vec2(2.0, 0.0), vec2(body.width() - 4.0, 34.0));
        let p = ui.painter().clone();
        p.rect_filled(add, 0.0, BAR);
        let can_add = can(app, &Cmd::AddToPalette);
        let col = if !can_add { ICON.gamma_multiply(0.4) } else if resp.is_pointer_button_down_on() { ICON_DOWN } else if resp.hovered() { ICON_OVER } else { ICON };
        let c = add.center();
        p.rect_filled(Rect::from_center_size(c, vec2(7.0, 1.0)), 0.0, col);
        p.rect_filled(Rect::from_center_size(c, vec2(1.0, 7.0)), 0.0, col);
        app.edit.map.rects.insert("palette_add".into(), add);
        let resp = if app.prefs.tooltips { resp.on_hover_text("Add to palette:\nAdds the selected range to the palette.") } else { resp };
        if resp.clicked() && can_add {
            app.edit.queue.push(Cmd::AddToPalette);
        }
    });
    // The selected range carried here from the waveform becomes an entry.
    if app.edit.gesture == Gesture::DragSelection {
        let over = ui.ctx().pointer_latest_pos().map(|pp| r.contains(pp)).unwrap_or(false);
        if over {
            ui.painter().rect_stroke(body, 0.0, st(1.0, WHITE), egui::StrokeKind::Inside);
        }
        if ui.input(|i| i.pointer.any_released()) {
            if over {
                app.edit.queue.push(Cmd::AddToPalette);
            }
            app.edit.gesture = Gesture::None;
        }
    }
}

fn palette_row(app: &mut App, ui: &mut Ui, r: Rect, id: u32, index: usize, del: bool) {
    let Some(pr) = app.edit.project.as_ref() else { return };
    let Some(clip) = pr.session.palettes.iter().find(|c| c.id == id).cloned() else { return };
    // (The row itself first, so that its controls, added after it, get their clicks.)
    let row = ui.interact(r, ui.id().with(("edit_pal_row", id)), Sense::click());
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, BAR);
    p.rect_stroke(r, 0.0, st(1.0, FRAME), egui::StrokeKind::Inside);
    // Waveform of the clip: black beat lines, its own play cursor.
    let wr = Rect::from_min_size(r.min + vec2(2.0, 2.0), vec2(r.width() - 4.0, 48.0));
    p.rect_filled(wr, 0.0, FRAME);
    let len = clip.clip.len().max(1.0);
    let ppb = wr.width() as f64 / len;
    let step = [1.0, 4.0, 16.0, 64.0].into_iter().find(|n| n * ppb >= 4.0).unwrap_or(256.0);
    let mut k = 0.0;
    while k < len {
        let x = wr.min.x + (k * ppb) as f32;
        p.rect_filled(Rect::from_min_max(pos2(x, wr.min.y), pos2(x + 1.0, wr.max.y)), 0.0, Color32::BLACK);
        k += step;
    }
    if let Some(w) = &pr.wave {
        score_wave(&p.with_clip_rect(wr), wr, &clip.clip, w, app.prefs.wave_color, 0.0, ppb, 1.2, true);
    }
    let (pal_playing, pal_pos) = {
        let e = EDIT.lock();
        (e.pal_playing(id), e.pal_of(id).map(|k| e.pals[k].pos).unwrap_or(0.0))
    };
    if pal_playing {
        let x = wr.min.x + (pal_pos * ppb) as f32;
        p.rect_filled(Rect::from_min_max(pos2(x - 0.5, wr.min.y), pos2(x + 0.5, wr.max.y)), 0.0, WHITE);
        ui.ctx().request_repaint();
    }
    let wresp = ui.interact(wr, ui.id().with(("edit_pal_wave", id)), Sense::click_and_drag());
    app.edit.map.rects.insert(format!("pal_wave{index}"), wr);
    if wresp.clicked() {
        if let Some(pp) = wresp.interact_pointer_pos() {
            palette_play(app, id, Some(((pp.x - wr.min.x) as f64 / ppb).floor()));
        }
    }
    if wresp.drag_started_by(egui::PointerButton::Primary) {
        app.edit.gesture = Gesture::DragPalette(id);
    }
    if app.edit.gesture == Gesture::DragPalette(id) && !ui.input(|i| i.pointer.any_down()) {
        app.edit.gesture = Gesture::None;
    }
    // Play / stop, the name, and in delete mode the row's delete button.
    let y = wr.max.y;
    let play = Rect::from_min_size(pos2(r.min.x + 14.0, y + 7.0), vec2(14.0, 14.0));
    let (asset, pr_) = if pal_playing { ("edit/stop.svg", Rect::from_center_size(play.center(), vec2(10.0, 10.0))) } else { ("edit/play.svg", Rect::from_center_size(play.center(), vec2(10.0, 14.0))) };
    if icon_btn(app, ui, pr_, &format!("pal_play{index}"), asset, !app.deck_options.disable_play, None, "Play/Stop:\nPlays/stops the waveform in the palette.").clicked() {
        palette_play(app, id, None);
    }
    let name_r = Rect::from_min_max(pos2(r.min.x + 50.0, y + 6.0), pos2(r.max.x - 34.0, y + 22.0));
    app.edit.map.rects.insert(format!("pal_name{index}"), name_r);
    if let Some((rid, mut text)) = app.edit.renaming.clone().filter(|(rid, _)| *rid == id) {
        let resp = ui.put(name_r, egui::TextEdit::singleline(&mut text).font(rb(em(13.0))).margin(vec2(2.0, 0.0)));
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        let done = resp.lost_focus();
        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if done {
            if !cancelled && !text.trim().is_empty() {
                if let Some(pr) = app.edit.project.as_mut() {
                    pr.session.rename_palette(rid, text.trim());
                }
            }
            app.edit.renaming = None;
        } else {
            app.edit.renaming = Some((rid, text));
        }
    } else {
        p.with_clip_rect(name_r).text(pos2(name_r.min.x + 2.0, name_r.center().y), Align2::LEFT_CENTER, &clip.name, rb(em(13.0)), hex(0xC8C8C8));
        let nresp = ui.interact(name_r, ui.id().with(("edit_pal_name", id)), Sense::click());
        if nresp.double_clicked() {
            app.edit.renaming = Some((id, clip.name.clone()));
        }
    }
    if del {
        let tr = Rect::from_min_size(pos2(r.max.x - 26.0, y + 6.0), vec2(14.0, 17.0));
        if icon_btn(app, ui, tr, &format!("pal_del{index}"), "edit/trashBox.svg", true, None, "Delete button:\nDeletes the waveform in the palette.").clicked() {
            palette_remove(app, id);
        }
    }
    row.context_menu(|ui| {
        if ui.button("Copy").clicked() {
            if let Some(pr) = app.edit.project.as_mut() {
                pr.session.copy_palette(id);
            }
            ui.close_menu();
        }
    });
}

/// What is being carried under the pointer: the selected range, or a palette entry.
fn drag_ghost(app: &mut App, ui: &mut Ui) {
    let label = match &app.edit.gesture {
        Gesture::DragSelection => app.edit.project.as_ref().and_then(|p| p.session.selection.range()).map(|(a, b)| readout(b - a, 120.0).1),
        Gesture::DragPalette(id) => app.edit.project.as_ref().and_then(|p| p.session.palettes.iter().find(|c| c.id == *id)).map(|c| c.name.clone()),
        _ => None,
    };
    let (Some(label), Some(pp)) = (label, ui.ctx().pointer_latest_pos()) else { return };
    let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("edit_drag")));
    let g = lp.layout_no_wrap(label, rb(11.0), WHITE);
    let rr = Rect::from_min_size(pp + vec2(12.0, 6.0), g.size() + vec2(10.0, 6.0));
    lp.rect_filled(rr, 2.0, Color32::from_rgba_unmultiplied(0x40, 0xA0, 0xFF, 178));
    lp.galley(rr.min + vec2(5.0, 3.0), g, WHITE);
    if !ui.input(|i| i.pointer.any_down()) {
        app.edit.gesture = Gesture::None;
    }
    ui.ctx().request_repaint();
}

// ---------------------------------------------------------------------------- dialogs

fn modals(app: &mut App, ctx: &egui::Context) {
    let Some(modal) = app.edit.modal.take() else { return };
    // Everything behind a dialog is out of reach while it is up.
    egui::Area::new(egui::Id::new("edit_modal_shade")).order(egui::Order::Middle).fixed_pos(pos2(0.0, 0.0)).show(ctx, |ui| {
        let r = ctx.screen_rect();
        ui.allocate_response(r.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(110));
    });
    let centre = |w: egui::Window<'static>| w.anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0)).order(egui::Order::Foreground).resizable(false);
    let mut keep = Some(modal);
    match keep.take().unwrap() {
        Modal::SavePrompt(what) => {
            let mut hit = None;
            centre(kit::dialog(ctx, "recordbox")).show(ctx, |ui| {
                kit::apply_style(ui);
                ui.set_min_width(340.0);
                kit::message(ui, "Save changes made to the project?");
                hit = dialog_buttons(app, ui, &["Yes", "No", "Cancel"]);
            });
            match hit {
                Some(0) => {
                    save_project(app, false, Some(what));
                }
                Some(1) => {
                    // The changes are dropped: nothing asks again on the way out.
                    if let Some(p) = app.edit.project.as_mut() {
                        p.session.mark_saved();
                    }
                    run_pending(app, what);
                }
                Some(_) => {}
                None => keep = Some(Modal::SavePrompt(what)),
            }
        }
        Modal::Alert { title, text } => {
            let mut hit = None;
            centre(kit::dialog(ctx, title.clone())).show(ctx, |ui| {
                kit::apply_style(ui);
                ui.set_min_width(380.0);
                ui.set_max_width(520.0);
                kit::message(ui, &text);
                hit = dialog_buttons(app, ui, &["OK"]);
            });
            if hit.is_none() {
                keep = Some(Modal::Alert { title, text });
            }
        }
        Modal::ConfirmNewer(path, data) => {
            let mut hit = None;
            centre(kit::dialog(ctx, "recordbox")).show(ctx, |ui| {
                kit::apply_style(ui);
                ui.set_max_width(520.0);
                kit::message(ui, "This project file may not operate correctly since its format had been updated.\nUse of the latest version of rekordbox is recommended.\nOpen the file?");
                hit = dialog_buttons(app, ui, &["OK", "Cancel"]);
            });
            match hit {
                Some(0) => open_data(app, path, *data),
                Some(_) => {}
                None => keep = Some(Modal::ConfirmNewer(path, data)),
            }
        }
        Modal::RenderTip { save_as, then, mut dont_show } => {
            let mut hit = None;
            centre(kit::dialog(ctx, "recordbox")).show(ctx, |ui| {
                kit::apply_style(ui);
                ui.set_max_width(520.0);
                kit::message(ui, "Press the [Render audio files] button to add the edited track to the library.");
                ui.add_space(10.0);
                kit::checkbox(ui, &mut dont_show, "Do not show this popup again.");
                hit = dialog_buttons(app, ui, &["OK"]);
            });
            if hit.is_some() {
                if dont_show {
                    app.prefs.edit.no_render_tip = true;
                    app.save_prefs();
                }
                // The tip has been shown: the save goes on to the save panel.
                app.edit.modal = Some(Modal::RenderTip { save_as, then: None, dont_show });
                save_project(app, save_as, then);
                if matches!(app.edit.modal, Some(Modal::RenderTip { .. })) {
                    app.edit.modal = None;
                }
                return;
            }
            keep = Some(Modal::RenderTip { save_as, then, dont_show });
        }
        Modal::ConfirmStop => {
            let mut hit = None;
            centre(kit::dialog(ctx, "recordbox")).show(ctx, |ui| {
                kit::apply_style(ui);
                kit::message(ui, "Are you sure you want to stop playing and switch to EDIT mode?");
                hit = dialog_buttons(app, ui, &["OK", "Cancel"]);
            });
            match hit {
                Some(0) => enter(app),
                Some(_) => {}
                None => keep = Some(Modal::ConfirmStop),
            }
        }
        Modal::Render(dlg) => keep = render_dialog(app, ctx, dlg),
        Modal::Rendering => {
            let mut cancel = false;
            let progress = app.edit.job.as_ref().map(|j| j.0.progress.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0).unwrap_or(1.0);
            centre(kit::dialog(ctx, "Rendering")).show(ctx, |ui| {
                kit::apply_style(ui);
                ui.set_min_width(340.0);
                ui.add(egui::ProgressBar::new(progress).desired_width(340.0).show_percentage());
                cancel = dialog_buttons(app, ui, &["Cancel"]).is_some();
            });
            if cancel {
                if let Some((j, _, _)) = &app.edit.job {
                    j.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            if app.edit.job.is_some() {
                keep = Some(Modal::Rendering);
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
    }
    // (A handler above may have put up the next dialog.)
    if app.edit.modal.is_none() {
        app.edit.modal = keep;
    }
}

/// A centred row of dialog buttons; each one's place is noted for the scripted run
/// ("dlg_yes", "dlg_ok" ...).
fn dialog_buttons(app: &mut App, ui: &mut Ui, labels: &[&str]) -> Option<usize> {
    let mut hit = None;
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let w: f32 = labels.len() as f32 * 96.0 + (labels.len().saturating_sub(1)) as f32 * 8.0;
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        for (i, l) in labels.iter().enumerate() {
            let r = ui.add(kit::btn(*l).min_size(vec2(96.0, 24.0)));
            app.edit.map.rects.insert(format!("dlg_{}", l.to_lowercase()), r.rect);
            if r.clicked() {
                hit = Some(i);
            }
        }
    });
    hit
}

/// "Render Audio": track and artist names, where the file goes, the format and its one
/// setting, and whether to show the result in the collection. Rendering is free here.
fn render_dialog(app: &mut App, ctx: &egui::Context, mut dlg: RenderDlg) -> Option<Modal> {
    let mut go = false;
    let mut close = false;
    let dir = render_dir(app);
    kit::dialog(ctx, "Render Audio").anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0)).order(egui::Order::Foreground).resizable(false).show(ctx, |ui| {
        kit::apply_style(ui);
        ui.set_min_width(520.0);
        ui.label(egui::RichText::new("Render Audio").font(kit::inter_bold(em(22.0))).color(WHITE));
        ui.add_space(10.0);
        let label = |ui: &mut Ui, s: &str| {
            ui.add_sized([150.0, 22.0], egui::Label::new(kit::text(s, em(16.0)).color(hex(0xC8C8C8))).halign(egui::Align::LEFT));
        };
        egui::Grid::new("edit_render_grid").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
            label(ui, "Track");
            ui.add(egui::TextEdit::singleline(&mut dlg.track).desired_width(340.0));
            ui.end_row();
            label(ui, "Artist");
            ui.add(egui::TextEdit::singleline(&mut dlg.artist).desired_width(340.0));
            ui.end_row();
            label(ui, "Location of Files");
            ui.horizontal(|ui| {
                let mut shown = dir.to_string_lossy().to_string();
                ui.add_enabled(false, egui::TextEdit::singleline(&mut shown).desired_width(262.0));
                if ui.add(kit::btn("Browse").min_size(vec2(70.0, 22.0))).clicked() {
                    let mut d = rfd::FileDialog::new();
                    if dir.is_dir() {
                        d = d.set_directory(&dir);
                    }
                    if let Some(f) = d.pick_folder() {
                        app.prefs.edit.render_dir = f.to_string_lossy().to_string();
                        app.save_prefs();
                    }
                }
            });
            ui.end_row();
            label(ui, "File Format");
            let e = &mut app.prefs.edit;
            let cur = e.render_format.min(3) as usize;
            kit::combo_plain("edit_render_format").width(140.0).selected_text(crate::edit_render::FORMATS[cur].0).show_ui(ui, |ui| {
                for (i, (name, _)) in crate::edit_render::FORMATS.iter().enumerate() {
                    ui.selectable_value(&mut e.render_format, i as u8, *name);
                }
            });
            ui.end_row();
            match e.render_format {
                1 | 2 => {
                    label(ui, "Sampling Rate");
                    let text = |r: u32| if r == 48000 { "48 kHz" } else { "44.1 kHz" };
                    kit::combo_plain("edit_render_rate").width(140.0).selected_text(text(e.render_rate)).show_ui(ui, |ui| {
                        for r in crate::edit_render::RATES {
                            ui.selectable_value(&mut e.render_rate, r, text(r));
                        }
                    });
                }
                3 => {
                    label(ui, "Bitrate");
                    kit::combo_plain("edit_render_bps").width(140.0).selected_text(format!("{} kbps", e.render_aac_bps / 1000)).show_ui(ui, |ui| {
                        for r in crate::edit_render::AAC_RATES {
                            ui.selectable_value(&mut e.render_aac_bps, r, format!("{} kbps", r / 1000));
                        }
                    });
                }
                _ => {
                    label(ui, "Compression Levels");
                    let text = |l: u8| match l {
                        8 => "8 Most Compression (Smallest File)".to_string(),
                        0 => "0 Least Compression (Largest File)".to_string(),
                        n => n.to_string(),
                    };
                    kit::combo_plain("edit_render_flac").width(260.0).selected_text(text(e.render_flac_level)).show_ui(ui, |ui| {
                        for l in (0..=8u8).rev() {
                            ui.selectable_value(&mut e.render_flac_level, l, text(l));
                        }
                    });
                }
            }
            ui.end_row();
        });
        ui.add_space(8.0);
        kit::checkbox(ui, &mut app.prefs.edit.render_show, "Show track in collection after render");
        ui.add_space(6.0);
        ui.label(kit::text("Note: It may take longer to render audio files in the EDIT mode.", 11.5).color(hex(0x969696)));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 208.0).max(0.0) / 2.0);
            let ok = !dlg.track.trim().is_empty();
            let r = ui.add_enabled(ok, kit::btn("Render").min_size(vec2(100.0, 30.0)));
            app.edit.map.rects.insert("dlg_render".into(), r.rect);
            go = r.clicked();
            close = ui.add(kit::btn("Cancel").min_size(vec2(100.0, 30.0))).clicked();
        });
    });
    if go {
        app.save_prefs();
        start_render(app, dlg);
        return None;
    }
    if close {
        app.save_prefs();
        return None;
    }
    Some(Modal::Render(dlg))
}
