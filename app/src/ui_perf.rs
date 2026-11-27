//! PERFORMANCE mode, 2Deck Horizontal: the two enlarged waveforms stacked along the top, deck 1
//! on the left and deck 2 on the right with the mixer between them, and the mixer's
//! headphones / crossfader row along the bottom.
//!
//! Geometry is measured on rekordbox 7.2.19 at 1512 pt (`rekordbox-reference/performance/01`,
//! `stems/14` and `30`) and from its layout code (`checks/re-7.2.19/performance-decks.md`:
//! platter panel, button and font sizes). Positions below are in points from the top-left of
//! the player area (just under the 28 pt top bar).

use crate::app::*;
use crate::model::*;
use crate::theme::*;
use crate::ui_perf_layouts as lay;
use crate::ui_player::{lighten, press_fired, split_time, tip};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, Ui};

/// Height of the stacked waveforms.
const WAVES_H: f32 = 143.0;
/// Width of a deck and x of deck 2 and of the mixer column (at 1512 pt).
const DECK_W: f32 = 700.0;
const MIXER_X: f32 = 705.0;
const MIXER_W: f32 = 104.0;
const DECK2_X: f32 = 812.0;
/// Extra height of the deck area while the stem part buttons are shown.
const STEM_ROW: f32 = 21.0;
/// Extra height while the phrase bar is shown under the overview.
pub const PHRASE_ROW: f32 = 6.0;

/// Deck panels: 0x03000061 (`PlayerPanelL::paint` 0x102282eb4 in PERFORMANCE) D #1E1E1E L #F0F0F0.
const PANEL: Tc = tc(0x1E1E1E, 0xF0F0F0);
const PAD: Tc = gr(0x141414);
const PART: [(&str, usize, Color32); 4] = [("VOCAL", 3, Color32::from_rgb(0x02, 0xDA, 0x0C)), ("INST", 2, Color32::from_rgb(0xDA, 0x1B, 0x02)), ("BASS", 1, Color32::from_rgb(0xD2, 0x00, 0x90)), ("DRUMS", 0, Color32::from_rgb(0x46, 0x55, 0xFF))];

pub fn performance(app: &mut App, ui: &mut Ui) {
    // (Once: the per-deck Q and BEAT SYNC rekordbox keeps, deck.rs.)
    app.restore_player_settings();
    lay::shot_tick(app);
    // The vertical layouts, Browse and the layouts without pad and platter.
    if lay::performance_alt(app, ui) {
        return;
    }
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);
    let (l, mut t) = (full.min.x, full.min.y);
    // The FX panel, when shown, sits above the waveforms.
    if app.prefs.perf_fx {
        crate::ui_fx::panel(app, ui, Rect::from_min_size(pos2(l, t), vec2(full.width(), crate::ui_fx::fx_h(app) - 1.0)));
        t += crate::ui_fx::fx_h(app);
    }
    // Two decks (2Deck Horizontal) or four (4Deck Horizontal: decks 1 and 3 on the left, 2 and
    // 4 on the right, the mixer's four strips between, in rekordbox's order 3, 1, 2, 4).
    // Where the pieces sit: `ui_perf_layouts::frame` (measured at 1512 pt: the waveforms take
    // 142 / 198 pt, a deck is 700 x 227 / 658 x 188 pt, the mixer 106 / 190 pt wide).
    let four = app.deck_count() == 4;
    let cx = full.center().x;
    let _ = (DECK_W, DECK2_X, MIXER_X, MIXER_W, WAVES_H);
    let stems_row = if app.prefs.show_stems { STEM_ROW } else { 0.0 };
    // The sampler panel, when shown, is the last row, under the mixer's bottom row.
    let sampler_h = if app.prefs.perf_sampler { crate::ui_sampler::SAMPLER_H } else { 0.0 };
    app.sampler_poll();
    // MIX POINT LINK: its strip takes the place of the stacked waveforms (horizontal layouts).
    let mpl_h = lay::mpl_strip(app);
    let count = app.deck_count();
    let fr = lay::frame(app, Rect::from_min_max(pos2(l, t), pos2(full.max.x, full.max.y - sampler_h)));

    // ---- the enlarged waveforms, one lane per deck, and the control strip over their left end
    let lanes = if mpl_h > 0.0 {
        crate::ui_mpl::strip(app, ui, Rect::from_min_size(pos2(l, t), vec2(full.width(), mpl_h)));
        Vec::new()
    } else {
        let lanes = lay::waves_h(app, ui, &fr);
        if !lay::layout(app).vertical() {
            crate::ui_mpl::after_waves(app, ui);
        }
        lanes
    };

    // ---- decks
    let mut rects = [Rect::NOTHING; 4];
    for n in 0..count {
        let whole = fr.decks[n as usize];
        rects[n as usize] = whole;
        // (The GROOVE CIRCUIT panel takes the last rows of the deck's rectangle.)
        let dr = Rect::from_min_max(whole.min, pos2(whole.max.x, whole.max.y - lay::groove_row(app)));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(dr).id_salt(("perfdeck", n)));
        child.set_clip_rect(dr.intersect(ui.clip_rect()));
        let is_active = app.px.active == n;
        app.with_deck(n, |app| deck(app, &mut child, dr, n, is_active, stems_row, four));
        app.with_deck(n, |app| crate::ui_player2::drop_to_load(app, ui, dr));
    }

    if lay::groove_row(app) > 0.0 {
        crate::ui_groove::row(app, ui, &rects[..count as usize]);
    }

    // ---- mixer
    if let Some(mr) = fr.mixer {
        if four { mixer4(app, ui, mr) } else { mixer(app, ui, mr) }
    }
    if let Some(br) = fr.bottom {
        bottom_row(app, ui, br, cx);
        if app.prefs.rec.panel {
            crate::ui_rec::panel(app, ui, br);
        }
    }
    if sampler_h > 0.0 {
        crate::ui_sampler::panel(app, ui, Rect::from_min_max(pos2(l, full.max.y - sampler_h), full.max));
    }

    // Keyboard shortcuts follow the pressed deck.
    lay::finish(app, ui, full, &rects[..count as usize], &lanes);
}

/// A flat text button of the deck (KEY SYNC, MASTER, SLIP, Q, MT, AU, MA ...).
fn flat(ui: &mut Ui, r: Rect, id: &str, label: &str, size: f32, fill: Color32, text: Color32) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    ui.painter().rect_filled(r, 2.0, if resp.hovered() && fill != BLUE { lighten(fill) } else { fill });
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, caps(size), text);
    resp
}

/// One deck: track information, overview, stem part buttons, pads, loop box and the platter
/// panel. `r`: the deck's rectangle. Sizes and places are those of the layout showing
/// (`ui_perf_layouts::deck_geo`, `lower_geo`); `c4` asks for the compact 4-deck sizes.
pub(crate) fn deck(app: &mut App, ui: &mut Ui, r: Rect, n: u8, is_active: bool, stems_row: f32, c4: bool) {
    let p = ui.painter().clone();
    let (x0, y0, w) = (r.min.x, r.min.y, r.width());
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x0 + x, y0 + y), vec2(w, h));
    let pos = app.pos();
    let loaded = app.deck.track.is_some();
    let track = app.deck.track.and_then(|id| app.track(id)).cloned();
    let dim = gy(0x323232);
    let dk = lay::dk(c4);
    let g = lay::deck_geo(dk);
    // Rows the View preferences add to the overview: vocal (above it) and phrase (under it).
    let info_h = g.info_h + lay::vocal_row_h(app) + lay::phrase_row_h(app);

    // ---- track information: the grey header; 2 pt of it run down both sides of the overview
    p.rect_filled(at(0.0, 0.0, w, info_h), 0.0, PANEL);
    // ---- overview (the whole track), laid out before the header's buttons so that they win
    // the click where its top reaches under them.
    lay::overview(app, ui, r.min, w, &g, pos);
    if dk == lay::Dk::Browse {
        // (Browse: the header's grey closes the box at the bottom.)
        p.rect_filled(at(0.0, info_h + 3.0, w, 1.0), 0.0, PANEL);
    }

    p.text(r.min + g.num.to_vec2() + vec2(1.0, 0.0), Align2::CENTER_CENTER, format!("{}", n + 1), lay::arial_b(10.0), if is_active { WHITE.c() } else { DIM.c() });
    // (View > "Hide Title, Artist, and Artwork": the empty artwork and no names.)
    let hide = crate::prefs_bridge::hide_deck_info(app);
    crate::ui_player2::artwork(app, ui, g.art.translate(r.min.to_vec2()), if hide { &None } else { &track });
    let (title, artist) = match &track {
        Some(_) if hide => (String::new(), String::new()),
        Some(tr) => (tr.display_title(), tr.artist.clone()),
        None => ("Not Loaded.".to_string(), String::new()),
    };
    // The deck's place (the sequencer strip can be dropped on it), and its title as a handle:
    // dragged to a sampler slot, the deck's track loads there.
    app.px.sampler.deck_rects[(n as usize).min(3)] = r;
    let tx0 = g.text_x;
    let title_w = (w - 300.0 - tx0).max(60.0);
    crate::app_hooks::deck_title_click(app, ui, g.art.translate(r.min.to_vec2()).union(at(tx0, 0.0, title_w, g.hdr_h)), n);
    if let Some(id) = app.deck.track {
        let hr = at(tx0, 4.0, title_w.min(240.0), g.title_y + 8.0);
        let hresp = ui.interact(hr, ui.id().with("deckdrag"), Sense::click_and_drag());
        if hresp.drag_started() && app.drag.is_none() {
            app.drag = Some(vec![id]);
        }
        // (A click there loads the selected track, as on the rest of the title area.)
        if hresp.clicked() {
            crate::app_hooks::load_selected_here(app);
        }
    }
    let sub_font = if g.sub_bold { lay::arial_b(g.sub_f) } else { lay::arial(g.sub_f) };
    let text_clip = p.with_clip_rect(at(tx0, 0.0, title_w, g.hdr_h));
    text_clip.text(pos2(x0 + tx0, y0 + g.title_y), Align2::LEFT_CENTER, title, lay::arial_b(g.title_f), if loaded { WHITE.c() } else { dim });
    let artist_clip = p.with_clip_rect(at(tx0, 0.0, g.bpm_right - 44.0 - 4.0 - tx0, g.hdr_h));
    artist_clip.text(pos2(x0 + tx0, y0 + g.sub_y), Align2::LEFT_CENTER, artist, sub_font.clone(), gy(0x8C8C8C));
    if loaded {
        let bpm = app.effective_bpm();
        p.text(pos2(x0 + g.bpm_right, y0 + g.sub_y), Align2::RIGHT_CENTER, format!("{bpm:.2}"), sub_font.clone(), WHITE.c());
        p.text(pos2(x0 + g.key_x, y0 + g.sub_y), Align2::LEFT_CENTER, app.result_key_label(), sub_font.clone(), WHITE.c());
    }
    // Remaining and elapsed time, right-aligned before the key controls.
    let remain = (app.deck.duration - pos).max(0.0);
    let (b2, s2) = split_time(if loaded { pos } else { 0.0 });
    let tcol = if loaded { WHITE.c() } else { dim };
    let time_font = lay::arial(g.time_f);
    // (Elapsed in colour id 0x3000076 = #878787, remaining 0x3000074 = white: `TimeView::
    // setSongTime`.)
    p.text(pos2(x0 + w - g.elapsed_r, y0 + g.time_y), Align2::RIGHT_CENTER, format!("{b2}.{s2}"), time_font.clone(), if loaded { gy(0x878787) } else { dim });
    let (b1, s1) = split_time(if loaded { remain } else { 0.0 });
    p.text(pos2(x0 + w - g.remain_r, y0 + g.time_y), Align2::RIGHT_CENTER, format!("-{b1}.{s1}"), time_font.clone(), tcol);
    if dk == lay::Dk::Browse && loaded {
        // Browse has no platter: the BPM and the tempo stand above the times.
        let pct = (app.deck_speed() - 1.0) * 100.0;
        // (Dimmed while Automix brings the tempo back: automix.rs `tempo_dimmed`.)
        let tdim = if crate::automix::tempo_dimmed(app, n) { 0.3 } else { 1.0 };
        let shown = ((pct.abs() + 0.005) * 10.0).floor() / 10.0;
        let sign = if pct < -0.05 { "-" } else if pct > 0.05 { "+" } else { "" };
        p.text(pos2(x0 + w - 248.0, y0 + 18.25), Align2::RIGHT_CENTER, format!("{:.2}", app.effective_bpm()), lay::arial(14.0), gy(0x8C8C8C));
        p.text(pos2(x0 + w - 190.5, y0 + 18.25), Align2::RIGHT_CENTER, format!("{sign}{shown:.1}%"), lay::arial(14.0), gy(0x8C8C8C).gamma_multiply(tdim));
    }

    // KEY SYNC, key shift, BEAT SYNC and MASTER at the right (rekordbox's TrackInfoView:
    // KEY SYNC 70 x 13 at w - 162, BEAT SYNC 44 x 23 and MASTER 70 x 13 at w - 78, the key
    // arrows at w - 162 and w - 102, the key at w - 148, the shift at w - 121).
    let rx = x0 + w;
    let by = y0 + g.y0;
    let ks = app.deck.x.key_sync;
    if tip(flat(ui, Rect::from_min_size(pos2(rx - 162.0, by + 4.0), vec2(70.0, 13.0)), "keysync", "KEY SYNC", 8.5, if ks { hex(0x007DE1) } else { BTN.c() }, WHITE.c()), app, "KEY SYNC").clicked() {
        app.toggle_key_sync();
    }
    let ky = by + 32.0;
    let kl = ui.interact(Rect::from_center_size(pos2(rx - 157.0, ky), vec2(12.0, 14.0)), ui.id().with("keydn"), Sense::click());
    crate::icons_player::chevron_left(&p, pos2(rx - 157.0, ky), 0.9, if loaded { WHITE.c() } else { dim });
    if tip(kl, app, "Key Shift Down").clicked() {
        app.key_shift_step(-1);
    }
    let kr = ui.interact(Rect::from_center_size(pos2(rx - 97.0, ky), vec2(12.0, 14.0)), ui.id().with("keyup"), Sense::click());
    crate::icons_player::chevron_right(&p, pos2(rx - 97.0, ky), 0.9, if loaded { WHITE.c() } else { dim });
    if tip(kr, app, "Key Shift Up").clicked() {
        app.key_shift_step(1);
    }
    if loaded {
        let reset = ui.interact(Rect::from_min_max(pos2(rx - 150.0, ky - 7.0), pos2(rx - 104.0, ky + 7.0)), ui.id().with("keyreset"), Sense::click());
        p.text(pos2(rx - 148.0, ky), Align2::LEFT_CENTER, app.result_key_label(), lay::arial_b(12.5), WHITE.c());
        p.text(pos2(rx - 106.0, ky), Align2::RIGHT_CENTER, crate::player_ext::shift_text(app.deck.x.key_shift), lay::arial_b(12.5), WHITE.c());
        if tip(reset, app, "Key Reset").clicked() {
            app.key_reset();
        }
    }
    let bs = app.deck.x.beat_sync;
    let bsr = Rect::from_min_size(pos2(rx - 78.0, by - 0.5), vec2(44.0, 23.0));
    let bresp = ui.interact(bsr, ui.id().with("beatsync"), Sense::click());
    p.rect_filled(bsr, 2.0, if bs { BLUE.c() } else if bresp.hovered() { lighten(BTN.c()) } else { BTN.c() });
    p.text(pos2(bsr.center().x, bsr.min.y + 6.75), Align2::CENTER_CENTER, if app.prefs.deck.beat_sync { "BEAT" } else { "BPM" }, caps(8.5), WHITE.c());
    crate::deck::sync_rate_hook(app, ui, n, bsr);
    p.text(pos2(bsr.center().x, bsr.min.y + 16.5), Align2::CENTER_CENTER, "SYNC", caps(8.5), WHITE.c());
    if tip(bresp, app, "BEAT SYNC").clicked() {
        app.toggle_beat_sync();
    }
    // MASTER: an outlined button, the outline blue on the master deck.
    let is_master = app.px.master == n && !app.sampler_is_master();
    let mrect = Rect::from_min_size(pos2(rx - 78.0, by + 26.0), vec2(70.0, 13.0));
    let mresp = ui.interact(mrect, ui.id().with("master"), Sense::click());
    p.rect_filled(mrect, 1.0, if mresp.hovered() { lighten(PANEL.c()) } else { PANEL.c() });
    p.rect_stroke(mrect, 1.0, Stroke::new(1.0_f32, if is_master { BLUE.c() } else { gy(0xC8C8C8) }), egui::StrokeKind::Inside);
    p.text(mrect.center(), Align2::CENTER_CENTER, "MASTER", caps(8.5), WHITE.c());
    if tip(mresp, app, "MASTER").clicked() {
        app.set_master();
    }

    // ---- stem part buttons (MUTE / SOLO): an 18 pt row under the overview, at the mixer's
    // side of the deck (right-aligned on the left decks, left-aligned on the right ones).
    let right = n % 2 == 1;
    if stems_row > 0.0 {
        let sy = y0 + info_h + 3.0;
        p.rect_filled(Rect::from_min_size(pos2(x0, sy), vec2(w, 18.0)), 0.0, PANEL);
        let on = app.player.state.lock().stem_on;
        let three = app.prefs.stem_parts == 3;
        let parts: Vec<(&str, Vec<usize>, Color32)> = PART.iter().filter(|x| !(three && x.0 == "BASS")).map(|x| (x.0, if three && x.0 == "INST" { vec![2, 1] } else { vec![x.1] }, x.2)).collect();
        let mut bx = if right { x0 + 46.0 } else { rx - 5.0 - parts.len() as f32 * 50.0 + 5.0 };
        p.text(pos2(bx - 27.0, sy + 9.0), Align2::CENTER_CENTER, if app.prefs.stem_solo { "SOLO" } else { "MUTE" }, lay::arial(11.0), gy(0x8C8C8C));
        for (i, (label, srcs, col)) in parts.iter().enumerate() {
            let br = Rect::from_min_size(pos2(bx, sy + 1.0), vec2(45.0, 16.0));
            let resp = ui.interact(br, ui.id().with(("part", i)), Sense::click());
            let lit = srcs.iter().all(|s| on[*s]);
            p.rect_filled(br, 1.0, if resp.hovered() { gy(0x1E1E1E) } else { gy(0x0F0F0F) });
            if lit {
                p.rect_stroke(br, 1.0, Stroke::new(1.0_f32, *col), egui::StrokeKind::Inside);
            }
            p.text(br.center(), Align2::CENTER_CENTER, *label, lay::arial(11.0), if lit { *col } else { gy(0x8C8C8C) });
            if resp.on_hover_text(format!("STEM MUTE ({label}):\nThe {label} Stem is output when set to on. The {label} Stem is muted when set to off.")).clicked() && loaded {
                crate::prefs_ext::stem_part_click(app, srcs);
            }
            bx += 50.0;
        }
    }
    if !dk.has_lower() {
        return;
    }

    // ---- lower part: pads, loop box and the platter panel. The left decks have the platter
    // at the mixer's side (right) and the tab column at the left; the right decks mirrored.
    let ly = y0 + info_h + 3.0 + stems_row;
    p.rect_filled(Rect::from_min_size(pos2(x0, ly), vec2(w, g.lower_h)), 0.0, PANEL);
    let lo = lay::lower_geo(dk, w, right);
    let o = pos2(x0, ly);
    // Tab column: the pad view's mark and the grid view's.
    let grid_on = app.deck.x.dk.grid_tab;
    if let Some(g) = lay::tab_column(ui, Rect::from_min_size(pos2(x0 + lo.tabs_x, ly), vec2(28.0, g.lower_h)), grid_on) {
        app.deck.x.dk.grid_tab = g;
    }
    if grid_on {
        // The GRID panel takes the place of the pads and the loop box (`MemoryCuePanel`).
        let pr = lo.pads.translate(o.to_vec2());
        let (x_a, x_b) = if right { (o.x + lo.loop_o.x, pr.max.x + 10.0) } else { (pr.min.x - 10.0, o.x + lo.loop_o.x + 78.0) };
        let mcp = Rect::from_min_max(pos2(x_a, ly), pos2(x_b, ly + g.lower_h));
        lay::grid_panel(app, ui, mcp, n, right, dk);
    } else {
        pads(app, ui, lo.pads.translate(o.to_vec2()), n, c4);
        loop_box(app, ui, o + lo.loop_o.to_vec2(), n, c4);
    }
    platter_panel(app, ui, o + lo.panel_o.to_vec2(), n, is_active, c4);
}

/// Pad modes of `PlayerExt::pad_mode` beyond HOT CUE (0), STEMS (1), BEAT JUMP (2) and
/// BEAT LOOP (3).
pub const PAD_SAMPLER: u8 = 4;
pub const PAD_SEQ_CALL: u8 = 5;

/// The pad area: eight pads in two columns and the pad mode pull-down under them.
pub(crate) fn pads(app: &mut App, ui: &mut Ui, r: Rect, n: u8, c4: bool) {
    if crate::ui_pads::host(app, ui, r, n, c4) {
        return;
    }
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let mode = match app.px.pad_mode[n as usize] {
        1 if !app.prefs.show_stems => 0,
        m => m,
    };
    let page = app.deck.x.pad_page.min(1);
    // Where the eight pads are (two columns of four, or one column of eight in 2Deck
    // Vertical; the STEMS pads have their own pitch): `ui_perf_layouts::PadGrid`.
    let pg = lay::pad_grid(lay::dk(c4), r, mode == 1);
    let pad = |c: usize, row: usize| pg.pad(c, row);
    if mode == 1 {
        // STEMS: the parts on the left (sounding = outlined), their FX switches on the right.
        let on = app.player.state.lock().stem_on;
        let three = app.prefs.stem_parts == 3;
        for (row, (label, src, col)) in PART.iter().enumerate() {
            let enabled = !(three && *label == "BASS");
            let srcs: Vec<usize> = if three && *label == "INST" { vec![2, 1] } else { vec![*src] };
            let lit = srcs.iter().all(|s| on[*s]);
            for c in 0..2 {
                let pr = pad(c, row);
                let resp = ui.interact(pr, ui.id().with(("stempad", c, row)), Sense::click());
                let active = if c == 0 { lit } else { app.px.stem_fx[row] };
                p.rect_filled(pr, 0.0, if resp.hovered() && enabled { gy(0x1A1A1A) } else { gy(0x0A0A0A) });
                if active && enabled {
                    p.rect_stroke(pr, 0.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside);
                }
                let a = if enabled && loaded { 1.0 } else { 0.5 };
                p.rect_filled(Rect::from_min_size(pos2(pr.min.x + 3.0, pr.center().y - 4.0), vec2(8.0, 8.0)), 0.0, col.gamma_multiply(a));
                let text = if c == 0 { label.to_string() } else { format!("FX {label}") };
                p.text(pos2(pr.min.x + 15.0, pr.center().y), Align2::LEFT_CENTER, text, FontId::proportional(11.5), if !enabled || !loaded { OFF.c() } else if active { hex(0x3C8CFF) } else { WHITE.c() });
                if c == 0 {
                    if press_fired(ui, &resp) && enabled && loaded {
                        crate::prefs_ext::stem_part_click(app, &srcs);
                    }
                } else if resp.on_hover_text(format!("STEM FX ({label}):\nSet when applying effects to the {label} Stem.\n(recordbox has no effects yet: the switch is kept but does nothing.)")).clicked() && enabled {
                    app.px.stem_fx[row] = !app.px.stem_fx[row];
                }
            }
        }
    } else if mode == 2 {
        // BEAT JUMP: one cell per length, back on its left and forward on its right. Page 1:
        // 1 to 128 beats (the lengths rekordbox's pads default to); page 2: 1/8 to 1/2.
        let lens: [f64; 8] = if page == 0 { [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0] } else { [0.125, 0.25, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0] };
        for (i, b) in lens.iter().enumerate() {
            let pr = pad(i / 4, i % 4);
            p.rect_filled(pr, 0.0, gy(0x161616));
            if *b <= 0.0 {
                continue;
            }
            p.text(pr.center(), Align2::CENTER_CENTER, crate::player_ext::beats_text(*b), bold(10.5), if loaded { WHITE.c() } else { OFF.c() });
            for (dir, x) in [(-1.0, pr.min.x), (1.0, pr.max.x - 30.0)] {
                let br = Rect::from_min_size(pos2(x, pr.min.y), vec2(30.0, pr.height()));
                let resp = ui.interact(br, ui.id().with(("bjpad", i, dir as i32)), Sense::click());
                p.rect_filled(br, 0.0, if resp.hovered() && loaded { lighten(BTN.c()) } else { BTN.c() });
                let col = if loaded { WHITE } else { OFF };
                if dir < 0.0 {
                    crate::icons_player::chevron_left(&p, br.center(), 1.1, col.c());
                } else {
                    crate::icons_player::chevron_right(&p, br.center(), 1.1, col.c());
                }
                if press_fired(ui, &resp) && loaded {
                    app.beat_jump(dir * *b);
                }
            }
        }
    } else if mode == crate::ui_padfx::PAD_FX {
        let (sy, sh) = if c4 { (81.0, 15.0) } else { (91.5, 17.0) };
        let strip = Rect::from_min_max(pos2(r.min.x + if n % 2 == 0 { 76.0 } else { 0.0 }, r.min.y + sy), pos2(r.max.x - if n % 2 == 0 { 0.0 } else { 76.0 }, r.min.y + sy + sh));
        crate::ui_padfx::pads(app, ui, n, &pad, strip);
    } else if mode == PAD_SAMPLER {
        // SAMPLER: the pads play the slots of the sampler's bank (1-8; the second page 9-16),
        // in the slot's colour with its sample's name. They follow the sequencer's MUTE /
        // ERASE modes, Shift stops, and an empty pad pressed while this deck loops captures
        // the loop (`Sampler.pad.csv`, `deviceSamplerPadSelectedSlot`).
        let bank = app.prefs.sampler.bank.min(3) as usize;
        let shift = ui.input(|i| i.modifiers.shift);
        let released = ui.input(|i| i.pointer.any_released());
        let pads_on = app.sampler_pads();
        for i in 0..8usize {
            let slot = i + 8 * page as usize;
            let idx = bank * 16 + slot;
            let pr = pad(i / 4, i % 4);
            let resp = ui.interact(pr, ui.id().with(("smppad", i)), Sense::click_and_drag());
            let used = slot < pads_on;
            let name = app.prefs.sampler.slots.get(idx).and_then(|s| s.as_ref()).map(|s| s.name.clone()).filter(|_| used);
            let ready = name.is_some() && app.px.sampler.ready(idx);
            let playing = ready && crate::sampler::SAMPLER.lock().as_ref().is_some_and(|e| e.slot_state(slot, bank).0);
            let muted = app.px.sampler.is_muted(idx);
            match &name {
                Some(nm) => {
                    let code = app.sampler_color_code(idx);
                    let col = if playing { hex(crate::sampler_ext::PAD_COLORS[code as usize]) } else { crate::sampler_ext::pad_color(code) };
                    let col = if muted || !ready { col.gamma_multiply(0.45) } else { col };
                    p.rect_filled(pr, 0.0, if resp.hovered() { lighten(col) } else { col });
                    p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, format!("{}", slot + 1), bold(10.5), gy(0xF4F4F4));
                    p.with_clip_rect(pr).text(pos2(pr.min.x + 26.0, pr.center().y), Align2::LEFT_CENTER, nm, FontId::proportional(10.5), gy(0xF4F4F4));
                }
                None => {
                    p.rect_filled(pr, 0.0, if resp.hovered() && used { gy(0x1E1E1E) } else { gy(0x161616) });
                    p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, format!("{}", slot + 1), FontId::proportional(10.5), if used { gy(0x8C8C8C) } else { gy(0x3C3C3C) });
                }
            }
            if press_fired(ui, &resp) && used {
                app.sampler_deck_pad_down(n, slot, shift);
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("smppad_held", n)), slot));
            }
            if released {
                if let Some(h) = ui.ctx().data_mut(|d| d.remove_temp::<usize>(egui::Id::new(("smppad_held", n)))) {
                    app.sampler_pad_up(h);
                }
            }
        }
    } else if mode == PAD_SEQ_CALL {
        // SEQ. CALL: the eight patterns ("P1".."P8", the name, "<n>Bars"). A press starts the
        // pattern, Shift + press stops it; right-click names it; the cross clears a saved one.
        let shift = ui.input(|i| i.modifiers.shift);
        let (cur, reserved, playing, bars) = match crate::sampler::SAMPLER.lock().as_ref() {
            Some(e) => (e.seq.current(), e.seq.reserved(), e.seq.playing, std::array::from_fn::<u8, 8, _>(|k| e.seq.pattern(k).bars)),
            None => (0, None, false, [1; 8]),
        };
        let blink = (ui.input(|i| i.time) / 0.3) as i64 % 2 == 0;
        for i in 0..8usize {
            let pr = pad(i / 4, i % 4);
            let resp = ui.interact(pr, ui.id().with(("seqpad", i)), Sense::click());
            let saved = app.prefs.sampler.seq.patterns.get(i).is_some_and(|x| x.is_some());
            let lit = (playing && cur == i && reserved.is_none()) || (reserved == Some(i) && blink);
            p.rect_filled(pr, 0.0, if lit { BLUE.c() } else if resp.hovered() { gy(0x1E1E1E) } else { gy(0x161616) });
            let tc = if saved || lit { gy(0xF4F4F4) } else { gy(0x8C8C8C) };
            p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, format!("P{}", i + 1), bold(10.5), tc);
            p.with_clip_rect(Rect::from_min_max(pr.min, pos2(pr.max.x - 56.0, pr.max.y))).text(pos2(pr.min.x + 30.0, pr.center().y), Align2::LEFT_CENTER, app.prefs.sampler.seq.name(i), FontId::proportional(10.5), tc);
            p.text(pos2(pr.max.x - 20.0, pr.center().y), Align2::RIGHT_CENTER, if bars[i] == 1 { "1Bar".to_string() } else { format!("{}Bars", bars[i]) }, FontId::proportional(9.5), tc);
            if saved {
                let dr = Rect::from_center_size(pos2(pr.max.x - 9.0, pr.center().y), vec2(12.0, 12.0));
                let dresp = ui.interact(dr, ui.id().with(("seqpad_del", i)), Sense::click());
                let (c, col) = (dr.center(), if dresp.hovered() { WHITE.c() } else { gy(0xA0A0A0) });
                p.line_segment([c + vec2(-3.0, -3.0), c + vec2(3.0, 3.0)], Stroke::new(1.2_f32, col));
                p.line_segment([c + vec2(-3.0, 3.0), c + vec2(3.0, -3.0)], Stroke::new(1.2_f32, col));
                if dresp.on_hover_text("Delete selected sequence").clicked() {
                    app.sampler_seq_delete(i);
                    continue;
                }
            }
            if press_fired(ui, &resp) {
                app.sampler_seq_call(i, shift);
            }
            resp.context_menu(|ui| {
                let mut name = app.prefs.sampler.seq.name(i);
                ui.label("Name");
                if ui.add(egui::TextEdit::singleline(&mut name).char_limit(32).desired_width(140.0)).changed() {
                    let sq = &mut app.prefs.sampler.seq;
                    sq.names.resize(8, String::new());
                    sq.names[i] = if name.trim() == format!("PATTERN {}", i + 1) { String::new() } else { name.trim().to_string() };
                    app.save_prefs();
                }
            });
        }
    } else if mode == 3 {
        // BEAT LOOP: the 16 lengths, 1/64 to 2 beats on the first page and 4 to 512 on the
        // second; the running loop's length is lit, and pressing it again ends the loop.
        let cur = app.loop_active().map(|(a, b)| (b - a) / app.beat_len().max(1e-9));
        for i in 0..8usize {
            let b = crate::player_ext::LOOP_SIZES[i + 8 * page as usize];
            let pr = pad(i / 4, i % 4);
            let resp = ui.interact(pr, ui.id().with(("blpad", i)), Sense::click());
            let lit = cur.map(|c| (c / b - 1.0).abs() < 0.02).unwrap_or(false);
            p.rect_filled(pr, 0.0, if lit { LOOP } else if resp.hovered() && loaded { gy(0x1E1E1E) } else { gy(0x161616) });
            p.text(pr.center(), Align2::CENTER_CENTER, crate::player_ext::beats_text(b), bold(10.5), if loaded { WHITE.c() } else { OFF.c() });
            if press_fired(ui, &resp) && loaded {
                if lit {
                    app.loop_exit();
                } else {
                    app.prefs.loop_beats = b;
                    app.auto_loop(b);
                }
            }
        }
    } else {
        // HOT CUE: A-D on the left, E-H on the right; the second page has I-P.
        for slot in (8 * page as usize)..(8 * page as usize + 8) {
            let pr = pad((slot % 8) / 4, slot % 4);
            let resp = ui.interact(pr, ui.id().with(("hotpad", slot)), Sense::click());
            let cue = app.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot as i32).cloned();
            let letter = ((b'A' + slot as u8) as char).to_string();
            match &cue {
                Some(c) => {
                    let col = hotcue_color(c.color);
                    p.rect_filled(pr, 0.0, if resp.hovered() { lighten(col) } else { col });
                    p.text(pos2(pr.min.x + 9.0, pr.center().y), Align2::LEFT_CENTER, letter, bold(10.5), gy(0xF4F4F4));
                    let (m, f) = split_time(c.start);
                    p.with_clip_rect(pr).text(pos2(pr.min.x + 26.0, pr.center().y), Align2::LEFT_CENTER, if c.comment.is_empty() { format!("{m}.{f}") } else { c.comment.clone() }, FontId::proportional(10.5), gy(0xF4F4F4));
                }
                None => {
                    p.rect_filled(pr, 0.0, if resp.hovered() && loaded { gy(0x1E1E1E) } else { gy(0x0F0F0F) });
                    p.text(pos2(pr.min.x + 10.0, pr.center().y), Align2::LEFT_CENTER, letter, lay::arial(13.0), gy(0x6E6E6E));
                }
            }
            if press_fired(ui, &resp) && loaded {
                app.hot_cue_down(slot as i32);
            }
            let held = app.deck.hot_preview.is_some() || app.player.state.lock().slip_hold;
            if held && !ui.input(|i| i.pointer.any_down()) && !app.midi.pad_held(n) && !crate::keycmd::holding(n) {
                app.hot_cue_up();
            }
            if let Some(c) = cue {
                resp.context_menu(|ui| crate::ui_player::hot_cue_menu(app, ui, &c));
            }
        }
    }
    if crate::ui_pads::pull_down(app, ui, r, n, c4) {
        return;
    }
    // (2Deck Vertical shows 160 pt of the pad list: the eighth pad is cut off.)
    if pg.cols == 1 && mode != 1 {
        p.rect_filled(Rect::from_min_max(pos2(r.min.x, r.min.y + 160.0), pos2(r.max.x, r.min.y + 163.0)), 0.0, PANEL);
        p.rect_filled(pg.scroll_bar(0).0.shrink2(vec2(0.5, 0.0)), 0.0, gy(0x323232));
    }
    // Pad mode pull-down (deck 1 at the left under the pads, deck 2 at the right).
    let label = match mode {
        1 => "STEMS",
        2 => "BEAT JUMP",
        3 => "BEAT LOOP",
        PAD_SAMPLER => "SAMPLER",
        PAD_SEQ_CALL => "SEQ. CALL",
        crate::ui_padfx::PAD_FX => "PAD FX",
        _ => "HOT CUE",
    };
    // (The pull-down is as wide as its label: 58.5 pt for STEMS, 68 pt for HOT CUE.)
    let mw = (p.layout_no_wrap(label.to_string(), caps(8.5), WHITE.c()).size().x + 30.0).round();
    let mr = pg.mode_rect(n % 2 == 1, mw);
    let resp = ui.interact(mr, ui.id().with("padmode"), Sense::click());
    p.rect_filled(mr, 2.0, if resp.hovered() { lighten(BTN.c()) } else { BTN.c() });
    p.text(pos2(mr.min.x + 5.0, mr.center().y), Align2::LEFT_CENTER, label, caps(8.5), WHITE.c());
    crate::icons_player::chevron_down(&p, pos2(mr.max.x - 9.0, mr.center().y + 0.5), 1.0, WHITE.c());
    let stems_on = app.prefs.show_stems;
    crate::ui_browser::dropdown(ui, &resp, "padmodepop", |ui| {
        if ui.selectable_label(mode == 0, "HOT CUE").clicked() {
            app.px.pad_mode[n as usize] = 0;
            ui.memory_mut(|m| m.close_popup());
        }
        if ui.add_enabled(stems_on, egui::SelectableLabel::new(mode == 1, "STEMS")).on_disabled_hover_text("Enable the STEMS Function in Preferences > Extensions > STEMS.").clicked() {
            app.px.pad_mode[n as usize] = 1;
            ui.memory_mut(|m| m.close_popup());
        }
        for (m, name) in [(2u8, "BEAT JUMP"), (3, "BEAT LOOP")] {
            if ui.selectable_label(mode == m, name).clicked() {
                app.px.pad_mode[n as usize] = m;
                ui.memory_mut(|m| m.close_popup());
            }
        }
        if ui.selectable_label(mode == PAD_SEQ_CALL, "SEQ. CALL").clicked() {
            app.px.pad_mode[n as usize] = PAD_SEQ_CALL;
            ui.memory_mut(|m| m.close_popup());
        }
        if ui.selectable_label(mode == crate::ui_padfx::PAD_FX, "PAD FX").clicked() {
            app.px.pad_mode[n as usize] = crate::ui_padfx::PAD_FX;
            ui.memory_mut(|m| m.close_popup());
        }
        for name in ["SLICER", "KEYBOARD", "KEY SHIFT", "ACT. CENSR", "MEMORY CUE"] {
            ui.add_enabled(false, egui::SelectableLabel::new(false, name)).on_disabled_hover_text("Not built in recordbox yet.");
        }
        // (In rekordbox SAMPLER is a pad mode of the controllers only; recordbox has no
        // controller pads yet, so it is offered here.)
        ui.separator();
        if ui.selectable_label(mode == PAD_SAMPLER, "SAMPLER").clicked() {
            app.px.pad_mode[n as usize] = PAD_SAMPLER;
            ui.memory_mut(|m| m.close_popup());
        }
    });
    // The second page (HOT CUE: I-P; BEAT JUMP and BEAT LOOP: their second page): rekordbox
    // scrolls the pads, with a 3 pt bar under them (beside them in 2Deck Vertical) whose
    // handle covers the half showing. A click on the bar's other half, or a drag, changes it.
    if mode != 1 && mode != PAD_SEQ_CALL && mode != crate::ui_padfx::PAD_FX {
        let (track, thumb) = pg.scroll_bar(page);
        let resp = ui.interact(track.expand(3.0), ui.id().with("padpage"), Sense::click_and_drag());
        if pg.cols == 2 {
            p.rect_filled(Rect::from_min_max(pos2(track.min.x, track.max.y - 1.0), track.max), 0.0, gy(0x323232));
        }
        p.rect_filled(thumb, 0.0, if resp.hovered() || resp.dragged() { gy(0x787878) } else { gy(0x555555) });
        if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.clicked() || resp.dragged()) {
            let second = if track.width() > track.height() { pp.x > track.center().x } else { pp.y > track.center().y };
            app.deck.x.pad_page = second as u8;
        }
        let _ = resp.on_hover_text(match (mode, page) {
            (2, 0) => "1 - 128",
            (2, _) => "1/8 - 1/2",
            (3, 0) => "1/64 - 2",
            (3, _) => "4 - 512",
            (PAD_SAMPLER, 0) => "1 - 8",
            (PAD_SAMPLER, _) => "9 - 16",
            (_, 0) => "A - H",
            _ => "I - P",
        });
    }
}

/// Loop size with halve / double, and AU (auto beat loop) or MA (manual: IN, OUT, RELOOP/EXIT).
pub(crate) fn loop_box(app: &mut App, ui: &mut Ui, o: Pos2, n: u8, c4: bool) {
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let looping = app.loop_active().is_some();
    let manual = app.px.loop_manual[n as usize];
    // (size box, arrows' top and height, AU/MA's top and height): 2-deck or 4-deck layout.
    // (Measured in all four layouts: 68 x 32, the arrows 34 pt down and 33 pt tall, AU / MA
    // 69 pt down and 18 pt tall.)
    let _ = c4;
    let (sh, ay, ah, by, bh) = (32.0, 33.5, 33.5, 69.0, 18.0);
    let size_r = Rect::from_min_size(o, vec2(68.0, sh));
    let lr = Rect::from_min_size(o + vec2(0.0, ay), vec2(33.0, ah));
    let rr = Rect::from_min_size(o + vec2(35.0, ay), vec2(33.0, ah));
    if manual {
        let re = ui.interact(size_r, ui.id().with("reloop"), Sense::click());
        p.rect_filled(size_r, 0.0, if looping { LOOP } else if re.hovered() { lighten(BTN.c()) } else { BTN.c() });
        p.text(size_r.center(), Align2::CENTER_CENTER, if looping { "EXIT" } else { "RELOOP" }, caps(9.0), WHITE.c());
        if tip(re, app, "Reloop/Exit").clicked() && loaded {
            app.exit_reloop();
        }
        let li = ui.interact(lr, ui.id().with("loopin"), Sense::click());
        p.rect_filled(lr, 0.0, if li.hovered() { lighten(BTN.c()) } else { BTN.c() });
        p.text(lr.center(), Align2::CENTER_CENTER, "IN", caps(9.0), WHITE.c());
        if tip(li, app, "Loop In").clicked() && loaded {
            app.loop_in_pressed();
        }
        let lo = ui.interact(rr, ui.id().with("loopout"), Sense::click());
        p.rect_filled(rr, 0.0, if lo.hovered() { lighten(BTN.c()) } else { BTN.c() });
        p.text(rr.center(), Align2::CENTER_CENTER, "OUT", caps(9.0), WHITE.c());
        if tip(lo, app, "Loop Out").clicked() && loaded {
            app.loop_out_pressed();
        }
    } else {
        let sz = ui.interact(size_r, ui.id().with("loopsize"), Sense::click());
        p.rect_filled(size_r, 0.0, if looping { LOOP } else if sz.hovered() { lighten(BTN.c()) } else { BTN.c() });
        p.text(size_r.center(), Align2::CENTER_CENTER, crate::player_ext::beats_text(app.prefs.loop_beats), bold(12.0), WHITE.c());
        if tip(sz, app, "Auto Beat Loop").clicked() && loaded {
            if looping {
                app.loop_exit();
            } else {
                app.auto_loop(app.prefs.loop_beats);
            }
        }
        let hl = ui.interact(lr, ui.id().with("loophalf"), Sense::click());
        p.rect_filled(lr, 0.0, if hl.hovered() { lighten(BTN.c()) } else { BTN.c() });
        crate::icons_player::chevron_left(&p, lr.center(), 1.35, WHITE.c());
        if tip(hl, app, "Loop Halve").clicked() {
            app.resize_loop(0.5);
        }
        let dl = ui.interact(rr, ui.id().with("loopdouble"), Sense::click());
        p.rect_filled(rr, 0.0, if dl.hovered() { lighten(BTN.c()) } else { BTN.c() });
        crate::icons_player::chevron_right(&p, rr.center(), 1.35, WHITE.c());
        if tip(dl, app, "Loop Double").clicked() {
            app.resize_loop(2.0);
        }
    }
    let au = Rect::from_min_size(o + vec2(0.0, by), vec2(33.0, bh));
    let ma = Rect::from_min_size(o + vec2(35.0, by), vec2(33.0, bh));
    if flat(ui, au, "loopau", "AU", 8.5, if manual { BTN.c() } else { BLUE.c() }, WHITE.c()).clicked() {
        app.px.loop_manual[n as usize] = false;
    }
    let mresp = ui.interact(ma, ui.id().with("loopma"), Sense::click());
    p.rect_filled(ma, 2.0, if manual { BLUE.c() } else { PAD.c() });
    if !manual {
        p.rect_stroke(ma, 2.0, Stroke::new(1.0_f32, gy(0x505050)), egui::StrokeKind::Inside);
    }
    p.text(ma.center(), Align2::CENTER_CENTER, "MA", caps(8.5), WHITE.c());
    if mresp.clicked() {
        app.px.loop_manual[n as usize] = true;
    }
}

/// rekordbox's `VirtualDeckPanel`: the play mode, CUE and PLAY/PAUSE, the platter with BPM,
/// tempo and tempo range, and SLIP / Q / MT. `o`: the panel's top-left. Its numbers for the
/// layout showing: `ui_perf_layouts::platter_geo` (platter 120, 150 or 108 pt).
pub(crate) fn platter_panel(app: &mut App, ui: &mut Ui, o: Pos2, n: u8, is_active: bool, c4: bool) {
    let p = ui.painter().clone();
    let loaded = app.deck.track.is_some();
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(o + vec2(x, y), vec2(w, h));
    let pg = lay::platter_geo(lay::dk(c4));
    let (col_x, ys, d) = (pg.col_x, pg.ys, pg.d);
    // Play mode: INT (internal) only.
    let int_r = at(0.0, pg.int_y, 40.0, 20.0);
    let int = ui.interact(int_r, ui.id().with("playmode"), Sense::hover());
    p.rect_filled(int_r, 2.0, BTN);
    p.text(pos2(int_r.min.x + 6.0, int_r.center().y), Align2::LEFT_CENTER, "INT", caps(8.5), WHITE.c());
    crate::icons_player::chevron_down(&p, pos2(int_r.max.x - 9.0, int_r.center().y + 0.5), 1.0, WHITE.c());
    let _ = int.on_hover_text("INTERNAL: the deck plays from the software. (DVS modes are not built in recordbox.)");
    crate::ui_player2::cue_play(app, ui, o + pg.cue_c, o + pg.play_c, (pg.btn - 3.0) / 2.0, is_active);

    // Platter: rings as fractions of d = its size less 10 (rekordbox: outer ring 1.0, body
    // 0.936, blue ring 0.8, inner disc 0.764).
    let pq = o + pg.plat.min.to_vec2();
    let c = pq + pg.plat.size() / 2.0;
    let playing = app.playing();
    let pos = app.pos();
    // (The outer ring is the turning part: on a loaded deck it is white with a gap behind
    // the red marker.)
    p.circle_filled(c, d / 2.0, if loaded { WHITE.c() } else { gy(0x2A2A2A) });
    p.circle_filled(c, 0.93636 * d / 2.0, PANEL);
    // The turn as rekordbox draws it (`VirtualDeckComp::drawCurrentCuePosition` /
    // `drawTextures`): frac(ms x rpm / 60000) x 360 degrees, less 44.041, from 3 o'clock
    // clockwise; the outer ring has a 15 degree gap there and the red mark sits on it; the
    // cue point is an 8 degree mark on the ring (measured on reference 01: gap 25.5-40.5
    // degrees at 0.3 s, red mark centred at 33 degrees, 0.855 of the ring's radius).
    let rpm = app.platter_rps() * 60.0;
    let deg = crate::deck::platter_angle_deg(pos, rpm);
    // (One mesh per wedge: separate anti-aliased slices left see-through seams.)
    let wedge = |a0: f32, a1: f32, r0: f32, r1: f32, col: Color32| {
        let n = ((a1 - a0) / 1.0).ceil().max(1.0) as u32;
        let mut mesh = egui::Mesh::default();
        for i in 0..=n {
            let b = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            mesh.colored_vertex(c + vec2(b.cos(), b.sin()) * r0, col);
            mesh.colored_vertex(c + vec2(b.cos(), b.sin()) * r1, col);
            if i > 0 {
                let k = 2 * i;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        p.add(egui::Shape::mesh(mesh));
    };
    if loaded {
        let (r0, r1) = (0.93 * d / 2.0, d / 2.0 + 1.0);
        let cue_deg = crate::deck::platter_angle_deg(app.deck.cue_point, rpm);
        wedge(cue_deg - 4.0, cue_deg + 4.0, r0, r1, hex(0xEB870F));
        wedge(deg - 7.5, deg + 7.5, r0, r1, PANEL.c());
    }
    p.circle_filled(c, 0.8 * d / 2.0, if loaded { BLUE.c() } else { hex(0x143C6E) });
    p.circle_filled(c, 0.76364 * d / 2.0, if loaded { BG.c() } else { gy(0x0F0F0F) });
    if loaded {
        // The red mark of the turning platter image, on the band between the blue ring and
        // the outer ring.
        let a = deg.to_radians();
        let m = c + vec2(a.cos(), a.sin()) * (0.855 * d / 2.0);
        let (u, v) = (vec2(a.cos(), a.sin()) * 0.027 * d, vec2(-a.sin(), a.cos()) * 0.027 * d);
        p.add(egui::Shape::convex_polygon(vec![m - u - v, m + u - v, m + u + v, m - u + v], hex(0xE61E1E), Stroke::NONE));
    }
    // (The jog: touch, scratch, drag to another deck, the hot cue countdown; `deck.rs`.)
    crate::deck::platter_hook(app, ui, n, c, d / 2.0);
    let plat = ui.interact(Rect::from_center_size(c, vec2(0.76 * d, 0.76 * d)), ui.id().with("platter"), Sense::hover());
    let bpm = if loaded { app.effective_bpm() } else { 0.0 };
    let tcol = if loaded { gy(0xE0E0E0) } else { gy(0x3C3C3C) };
    let whole = format!("{}", bpm.floor() as i64);
    let frac = format!(".{:02}", ((bpm - bpm.floor()) * 100.0).round().min(99.0) as i64);
    // BPM: Arial Bold, the hundredths smaller, centred in its box.
    let g = p.layout_no_wrap(if loaded { whole } else { "00".into() }, lay::arial_b(pg.bpm_f * 0.94), tcol);
    let g2 = p.layout_no_wrap(if loaded { frac } else { ".00".into() }, lay::arial_b(pg.bpm_f * if loaded { 0.68 } else { 0.94 }), tcol);
    let tw = g.size().x + g2.size().x;
    let bb = pg.bpm.translate(pq.to_vec2());
    let bx = bb.center().x - tw / 2.0;
    let (h1, h2) = (g.size().y, g2.size().y);
    let by = bb.center().y - h1 / 2.0;
    p.galley(pos2(bx, by), g.clone(), tcol);
    p.galley(pos2(bx + g.size().x, by + (h1 - h2) * 0.82), g2, tcol);
    // BPM drag (`BpmLabel`): up is faster, 250 pixels for the whole tempo range, 0.02 BPM
    // steps; not on a sync follower (`updateTempoAndTakeOverVisible`).
    if loaded && !app.sync_follower() {
        let dr = ui.interact(bb, ui.id().with(("bpmdrag", n)), Sense::drag());
        if dr.drag_started() {
            let y = ui.input(|i| i.pointer.press_origin().map(|p| p.y)).unwrap_or(bb.center().y);
            app.deck.x.dk.bpm_drag = Some((y, app.effective_bpm()));
        }
        if let (true, Some((y0, start)), Some(pp)) = (dr.dragged(), app.deck.x.dk.bpm_drag, ui.input(|i| i.pointer.latest_pos())) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            let own = app.current_bpm();
            let v = crate::deck::bpm_drag_value(start, own, app.tempo_range(), pp.y - y0);
            app.bpm_drag_to(v);
        }
        if dr.drag_stopped() {
            app.deck.x.dk.bpm_drag = None;
        }
    }
    // Tempo in percent (one decimal, half away from zero) and the range; a click on the
    // range steps it (±6, ±10, ±16, WIDE).
    let pct = (app.deck_speed() - 1.0) * 100.0;
    // (Dimmed while Automix brings the tempo back: automix.rs `tempo_dimmed`.)
    let tdim = if crate::automix::tempo_dimmed(app, n) { 0.3 } else { 1.0 };
    let shown = ((pct.abs() + 0.005) * 10.0).floor() / 10.0;
    let sign = if pct < -0.05 { "-" } else if pct > 0.05 { "+" } else { "" };
    let tr = pg.tempo.translate(pq.to_vec2());
    let small = lay::arial_b(pg.small_f + if pg.small_f > 12.0 { 1.5 } else { 0.5 });
    p.text(pos2(tr.max.x + pg.pct_w, tr.center().y), Align2::RIGHT_CENTER, format!("{sign}{shown:.1}%"), small.clone(), if loaded { gy(0x969696).gamma_multiply(tdim) } else { gy(0x3C3C3C) });
    let range = app.tempo_range() * 100.0;
    let rl = if !loaded { "± 0".to_string() } else if range >= 100.0 { "WIDE".to_string() } else { format!("±{range:.0}") };
    let rr = Rect::from_min_size(pos2(tr.max.x + pg.pct_w + 4.0, tr.min.y - 1.0), vec2(pg.range_w, tr.height() + 2.0));
    let rresp = ui.interact(rr, ui.id().with("temporange"), Sense::click());
    p.text(pos2(rr.min.x + 1.0, tr.center().y), Align2::LEFT_CENTER, rl, small, if loaded { hex(0xE6461E).gamma_multiply(tdim) } else { gy(0x3C3C3C) });
    if tip(rresp, app, "Tempo Range").clicked() {
        app.tempo_range_next();
    }
    // Tempo − / + and RESET appear inside the platter under the pointer.
    if loaded && (plat.hovered() || ui.rect_contains_pointer(Rect::from_center_size(c, vec2(0.8 * d, 0.8 * d)))) {
        let minus = pg.minus.translate(pq.to_vec2());
        let plus = pg.plus.translate(pq.to_vec2());
        let reset = Rect::from_min_max(pos2(minus.max.x, minus.min.y), pos2(plus.min.x, plus.max.y));
        let mr = ui.interact(minus, ui.id().with("tminus"), Sense::click());
        crate::icons_player::tempo_sign(&p, minus.center(), false, WHITE.gamma_multiply(tdim));
        let held = mr.is_pointer_button_down_on() && repeat(ui, &mr);
        if tip(mr, app, "Tempo -").clicked() || held {
            app.tempo_step(-1);
        }
        let pr = ui.interact(plus, ui.id().with("tplus"), Sense::click());
        crate::icons_player::tempo_sign(&p, plus.center(), true, WHITE.gamma_multiply(tdim));
        let held = pr.is_pointer_button_down_on() && repeat(ui, &pr);
        if tip(pr, app, "Tempo +").clicked() || held {
            app.tempo_step(1);
        }
        let rs = ui.interact(reset, ui.id().with("treset"), Sense::click());
        if rs.hovered() {
            p.text(pos2(reset.center().x, reset.max.y + 6.0), Align2::CENTER_CENTER, "RESET", caps(7.0), WHITE.c());
        }
        if tip(rs, app, "Tempo Reset").clicked() {
            app.tempo_reset();
        }
    }
    let _ = playing;

    // SLIP, Q, MT: 30 x 18 each.
    // SLIP: lit when on; blinks (500 ms) while the track runs on in the background.
    let slip_on = app.deck.x.slip;
    let blink = slip_on && app.slip_running() && (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
    if app.slip_running() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    if tip(flat(ui, at(col_x, ys[0], 30.0, 18.0), "slip", "SLIP", 8.5, if slip_on && !blink { BLUE.c() } else { BTN.c() }, WHITE.c()), app, "Slip").clicked() {
        app.toggle_slip();
    }
    let q = app.prefs.quantize;
    if tip(flat(ui, at(col_x, ys[1], 30.0, 18.0), "quant", "Q", 8.5, if q { BLUE.c() } else { BTN.c() }, WHITE.c()), app, "Quantize").clicked() {
        app.toggle_quantize();
    }
    let mt = app.deck.x.master_tempo;
    if tip(flat(ui, at(col_x, ys[2], 30.0, 18.0), "mt", "MT", 8.5, if mt { BLUE.c() } else { BTN.c() }, WHITE.c()), app, "Master Tempo").clicked() {
        app.toggle_master_tempo();
    }
    let _ = n;
}

/// A held button repeats every 50 ms after 0.4 s.
fn repeat(ui: &Ui, resp: &egui::Response) -> bool {
    ui.ctx().request_repaint();
    let key = resp.id.with("repeat");
    let now = ui.input(|i| i.time);
    let held = ui.input(|i| i.pointer.press_start_time()).map(|s| now - s).unwrap_or(0.0);
    if held > 0.4 {
        let last = ui.ctx().data(|d| d.get_temp::<f64>(key)).unwrap_or(0.0);
        if now - last >= 0.05 {
            ui.ctx().data_mut(|d| d.insert_temp(key, now));
            return true;
        }
    }
    false
}

/// A rotary knob 24 pt across: drag up or down to turn (a full sweep over 150 pt), double
/// click for `reset`. Returns the new value when it changed.
pub(crate) fn turn_knob(ui: &mut Ui, c: Pos2, id: impl std::hash::Hash, value: f32, reset: f32) -> (egui::Response, Option<f32>) {
    let r = Rect::from_center_size(c, vec2(24.0, 24.0));
    let resp = ui.interact(r, ui.id().with(id), Sense::click_and_drag());
    let mut out = None;
    if resp.double_clicked() {
        out = Some(reset);
    } else if resp.dragged() {
        let dy = resp.drag_delta().y;
        if dy != 0.0 {
            out = Some((value - dy / 150.0).clamp(0.0, 1.0));
        }
    }
    (resp, out)
}

pub(crate) fn knob(ui: &mut Ui, c: Pos2, id: impl std::hash::Hash, value: f32, reset: f32, colour: Color32) -> Option<f32> {
    let (resp, out) = turn_knob(ui, c, id, value, reset);
    let p = ui.painter();
    let v = out.unwrap_or(value);
    // rekordbox's knob: a 3 pt ring of 20 pt across, open at the bottom, and its skin's
    // 2 x 5 pt pointer from the ring's outer edge inwards.
    let ring = if resp.hovered() || resp.dragged() { gy(0x464646) } else { gy(0x323232) };
    lay::ring_knob(&p, c, 8.5, 3.0, v, ring, ring, colour);
    out
}

/// The mixer column: per deck TRIM, HIGH / MID / LOW (or VOCAL / INST / DRUMS in STEM ISO
/// mode), headphones CUE, the channel fader and the STEM button; the level meters between.
pub(crate) fn mixer(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    let (x0, y0) = (r.min.x, r.min.y);
    let stems = app.prefs.show_stems;
    let mut changed = false;
    // Where the strips' controls are in the layout showing (2Deck Horizontal: a column per
    // deck; 2Deck Vertical: TRIM, CUE and fader beside the EQ): `ui_perf_layouts::mix_geo`.
    let mg = lay::mix_geo(lay::dk(false));
    for st in mg.strips.iter().copied() {
        let n = st.deck;
        let cx = x0 + st.eq_x;
        let tx = x0 + st.trim_x;
        let mut m = app.px.mix[n];
        let lab = |p: &Painter, x: f32, y: f32, s: &str, col: Color32| {
            p.text(pos2(x, y0 + y), Align2::CENTER_CENTER, s, lay::arial_b(10.0), col);
        };
        // TRIM
        if let Some(v) = knob(ui, pos2(tx, y0 + mg.trim_y), ("trim", n), m.trim, 0.5, WHITE.c()) {
            m.trim = v;
            changed = true;
        }
        let tl = ui.interact(Rect::from_center_size(pos2(tx, y0 + mg.trim_y + mg.label_dy), vec2(36.0, 10.0)), ui.id().with(("triml", n)), Sense::hover());
        lab(&p, tx, mg.trim_y + mg.label_dy, "TRIM", WHITE.c());
        let tdb = crate::mixer::trim_db(m.trim);
        let _ = tl.on_hover_text(if tdb.is_finite() { format!("{tdb:+.1}dB") } else { "-inf dB".to_string() });
        crate::deck::again_hook(app, ui, n as u8, pos2(cx, y0 + 18.5), pos2(cx, y0 + 32.5));
        // HIGH / MID / LOW, or the parts in STEM ISO mode. A click on a label sets it to 0
        // (and back to the centre).
        let iso = m.stem_iso_on && stems;
        let names: [(&str, usize); 3] = if iso { [("VOCAL", 2), (if app.prefs.stem_parts == 3 { "INST" } else { "INST/BA." }, 1), ("DRUMS", 0)] } else { [("HIGH", 2), ("MID", 1), ("LOW", 0)] };
        for (i, (name, idx)) in names.iter().enumerate() {
            let ky = mg.eq_y[i];
            let val = if iso { m.stem_iso[*idx] } else { m.eq[*idx] };
            let col = if iso { PART[[3, 1, 0][*idx]].2 } else { WHITE.c() };
            let new = knob(ui, pos2(cx, y0 + ky), ("eq", n, i), val, 0.5, col);
            let lr = ui.interact(Rect::from_center_size(pos2(cx, y0 + ky + mg.label_dy), vec2(40.0, 10.0)), ui.id().with(("eql", n, i)), Sense::click());
            lab(&p, cx, ky + mg.label_dy, name, if val < 0.01 { hex(0xE61E1E) } else { WHITE.c() });
            let killed = lr.on_hover_text("Click to change the value to [0]").clicked();
            let v = if killed { Some(if val < 0.01 { 0.5 } else { 0.0 }) } else { new };
            if let Some(v) = v {
                if iso {
                    m.stem_iso[*idx] = v;
                } else {
                    m.eq[*idx] = v;
                }
                changed = true;
            }
        }
        // Headphones CUE.
        let fx_ = x0 + st.fader_x;
        let cr = Rect::from_center_size(pos2(fx_, y0 + mg.cue_y), vec2(32.0, 18.0));
        let cresp = ui.interact(cr, ui.id().with(("cue", n)), Sense::click());
        p.rect_filled(cr, 2.0, if m.cue { hex(0xFFA500) } else if cresp.hovered() { lighten(BTN.c()) } else { BTN.c() });
        p.text(cr.center(), Align2::CENTER_CENTER, "CUE", lay::arial_b(10.0), if m.cue { BG.c() } else { WHITE.c() });
        let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
        if cresp.on_hover_text(if hp_on { "Headphones CUE" } else { "Headphones CUE.\nNo headphones output is set: choose one in Preferences > Audio > Input/Output (it needs a device with four or more outputs)." }).clicked() {
            m.cue = !m.cue;
            changed = true;
        }
        // Channel fader: 50 pt of travel, the level shown in blue under the handle.
        let (ft, fb) = (y0 + mg.fader.0, y0 + mg.fader.1);
        let fr = Rect::from_min_max(pos2(fx_ - 12.0, ft - 4.0), pos2(fx_ + 12.0, fb + 4.0));
        let fresp = ui.interact(fr, ui.id().with(("fader", n)), Sense::click_and_drag());
        // (`slider_pos` also covers a quick click or drag whose events fall into one frame.)
        if let Some(pp) = crate::ui_kit::slider_pos(ui, &fresp, fr) {
            m.fader = ((fb - pp.y) / (fb - ft)).clamp(0.0, 1.0);
            changed = true;
        }
        if fresp.double_clicked() {
            m.fader = 1.0;
            changed = true;
        }
        let hy = fb - m.fader * (fb - ft);
        fader_face(&p, fx_, ft, fb, hy, fresp.hovered() || fresp.dragged());
        // STEM (STEM ISO mode), in the extra row the stem part buttons add.
        if stems {
            let sr = Rect::from_center_size(pos2(fx_, y0 + mg.stem_y), vec2(32.0, 16.0));
            let sresp = ui.interact(sr, ui.id().with(("stemiso", n)), Sense::click());
            p.rect_filled(sr, 2.0, if m.stem_iso_on { BLUE.c() } else if sresp.hovered() { lighten(BTN.c()) } else { BTN.c() });
            p.text(sr.center(), Align2::CENTER_CENTER, "STEM", caps(8.5), WHITE.c());
            if sresp.on_hover_text("STEM ISO Mode:\nSet to STEM ISO mode when set to on, enabling volume adjustment of each Stem. Set to EQ mode when set to off, enabling frequency band adjustment for HIGH/MID/LOW.").clicked() {
                m.stem_iso_on = !m.stem_iso_on;
                changed = true;
            }
        } else {
            m.stem_iso_on = false;
        }
        // Level meter: 2 pt wide, between the strips, from LOW up to TRIM.
        let mx = x0 + st.meter_x;
        let (mt, mb) = (y0 + mg.meter.0, y0 + mg.meter.1);
        p.rect_filled(Rect::from_min_max(pos2(mx, mt), pos2(mx + 2.0, mb)), 0.0, gy(0x323232));
        let dbv = m.shown_db[0].max(m.shown_db[1]);
        meter_bar(&p, Rect::from_min_max(pos2(mx, mt), pos2(mx + 2.0, mb)), dbv, true);
        app.px.mix[n] = m;
    }
    if changed {
        app.poll_decks();
    }
}

/// The 4-deck mixer column: four strips in rekordbox's order 3, 1, 2, 4 (measured on
/// reference performance/03: strip centres 30, 74, 118.5 and 162.5 pt into the 189 pt column;
/// deck number 14 pt down, TRIM 40, HIGH 80, MID 120, LOW 160 with their labels 15 pt under,
/// CUE 198, the fader from 224.5 to 344).
pub(crate) fn mixer4(app: &mut App, ui: &mut Ui, r: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    let (x0, y0) = (r.min.x, r.min.y);
    let stems = app.prefs.show_stems;
    let hp_on = crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
    let mut changed = false;
    // The strips' places in the layout showing (4Deck Horizontal: 44 pt apart with the deck
    // numbers above; 4Deck Vertical: 62 pt apart): `ui_perf_layouts::mix_geo`.
    let mg = lay::mix_geo(lay::dk(true));
    for st in mg.strips.iter().copied() {
        let n = st.deck;
        let cx = x0 + st.eq_x;
        let mut m = app.px.mix[n];
        if let Some(ny) = mg.num_y {
            p.text(pos2(cx, y0 + ny), Align2::CENTER_CENTER, format!("{}", n + 1), lay::arial(14.0), WHITE.c());
        }
        if let Some(v) = knob(ui, pos2(cx, y0 + mg.trim_y), ("trim4", n), m.trim, 0.5, WHITE.c()) {
            m.trim = v;
            changed = true;
        }
        let tl = ui.interact(Rect::from_center_size(pos2(cx, y0 + mg.trim_y + mg.label_dy), vec2(36.0, 10.0)), ui.id().with(("triml4", n)), Sense::hover());
        p.text(pos2(cx, y0 + mg.trim_y + mg.label_dy), Align2::CENTER_CENTER, "TRIM", lay::arial_b(10.0), WHITE.c());
        let tdb = crate::mixer::trim_db(m.trim);
        let _ = tl.on_hover_text(if tdb.is_finite() { format!("{tdb:+.1}dB") } else { "-inf dB".to_string() });
        crate::deck::again_hook(app, ui, n as u8, pos2(cx, y0 + 40.0), pos2(cx, y0 + 55.0));
        let iso = m.stem_iso_on && stems;
        let names: [(&str, usize); 3] = if iso { [("VOCAL", 2), (if app.prefs.stem_parts == 3 { "INST" } else { "INST/BA." }, 1), ("DRUMS", 0)] } else { [("HIGH", 2), ("MID", 1), ("LOW", 0)] };
        for (i, (name, idx)) in names.iter().enumerate() {
            let ky = mg.eq_y[i];
            let val = if iso { m.stem_iso[*idx] } else { m.eq[*idx] };
            let col_k = if iso { PART[[3, 1, 0][*idx]].2 } else { WHITE.c() };
            let new = knob(ui, pos2(cx, y0 + ky), ("eq4", n, i), val, 0.5, col_k);
            let lr = ui.interact(Rect::from_center_size(pos2(cx, y0 + ky + mg.label_dy), vec2(40.0, 10.0)), ui.id().with(("eql4", n, i)), Sense::click());
            p.text(pos2(cx, y0 + ky + mg.label_dy), Align2::CENTER_CENTER, *name, lay::arial_b(if iso { 8.5 } else { 10.0 }), if val < 0.01 { hex(0xE61E1E) } else { WHITE.c() });
            let killed = lr.on_hover_text("Click to change the value to [0]").clicked();
            let v = if killed { Some(if val < 0.01 { 0.5 } else { 0.0 }) } else { new };
            if let Some(v) = v {
                if iso {
                    m.stem_iso[*idx] = v;
                } else {
                    m.eq[*idx] = v;
                }
                changed = true;
            }
        }
        let cr = Rect::from_center_size(pos2(cx, y0 + mg.cue_y), vec2(32.0, 18.0));
        let cresp = ui.interact(cr, ui.id().with(("cue4", n)), Sense::click());
        p.rect_filled(cr, 2.0, if m.cue { hex(0xFFA500) } else if cresp.hovered() { lighten(BTN.c()) } else { BTN.c() });
        p.text(cr.center(), Align2::CENTER_CENTER, "CUE", lay::arial_b(10.0), if m.cue { BG.c() } else { WHITE.c() });
        if cresp.on_hover_text(if hp_on { "Headphones CUE" } else { "Headphones CUE.\nNo headphones output is set: choose one in Preferences > Audio > Input/Output (it needs a device with four or more outputs)." }).clicked() {
            m.cue = !m.cue;
            changed = true;
        }
        let (ft, fb) = (y0 + mg.fader.0, y0 + mg.fader.1);
        let fr = Rect::from_min_max(pos2(cx - 12.0, ft - 4.0), pos2(cx + 12.0, fb + 4.0));
        let fresp = ui.interact(fr, ui.id().with(("fader4", n)), Sense::click_and_drag());
        if let Some(pp) = crate::ui_kit::slider_pos(ui, &fresp, fr) {
            m.fader = ((fb - pp.y) / (fb - ft)).clamp(0.0, 1.0);
            changed = true;
        }
        if fresp.double_clicked() {
            m.fader = 1.0;
            changed = true;
        }
        let hy = fb - m.fader * (fb - ft);
        fader_face(&p, cx, ft, fb, hy, fresp.hovered() || fresp.dragged());
        // Level meter: 2 pt wide, left of the knobs.
        let mrect = Rect::from_min_max(pos2(x0 + st.meter_x, y0 + mg.meter.0), pos2(x0 + st.meter_x + 2.0, y0 + mg.meter.1));
        p.rect_filled(mrect, 0.0, gy(0x323232));
        meter_bar(&p, mrect, m.shown_db[0].max(m.shown_db[1]), true);
        if stems {
            let sr = Rect::from_center_size(pos2(cx, y0 + mg.stem_y), vec2(32.0, 16.0));
            let sresp = ui.interact(sr, ui.id().with(("stemiso4", n)), Sense::click());
            p.rect_filled(sr, 2.0, if m.stem_iso_on { BLUE.c() } else if sresp.hovered() { lighten(BTN.c()) } else { BTN.c() });
            p.text(sr.center(), Align2::CENTER_CENTER, "STEM", caps(8.5), WHITE.c());
            if sresp.on_hover_text("STEM ISO Mode:\nSet to STEM ISO mode when set to on, enabling volume adjustment of each Stem. Set to EQ mode when set to off, enabling frequency band adjustment for HIGH/MID/LOW.").clicked() {
                m.stem_iso_on = !m.stem_iso_on;
                changed = true;
            }
        } else {
            m.stem_iso_on = false;
        }
        app.px.mix[n] = m;
    }
    if changed {
        app.poll_decks();
    }
}

/// A channel fader as rekordbox draws it: marks at the middle and the bottom of its travel,
/// the level in blue from the handle down, the handle a 24 x 1.5 pt bar.
fn fader_face(p: &Painter, x: f32, ft: f32, fb: f32, hy: f32, hot: bool) {
    for y in [(ft + fb) / 2.0, fb] {
        p.rect_filled(Rect::from_center_size(pos2(x, y), vec2(20.0, 1.0)), 0.0, gy(0x323232));
    }
    p.rect_filled(Rect::from_min_max(pos2(x - 0.75, ft), pos2(x + 0.75, fb)), 0.0, gy(0x323232));
    p.rect_filled(Rect::from_min_max(pos2(x - 0.75, hy), pos2(x + 0.75, fb)), 0.0, BLUE);
    p.rect_filled(Rect::from_center_size(pos2(x, hy - 0.75), vec2(24.0, 1.5)), 0.0, if hot { WHITE.c() } else { gy(0x646464) });
}

/// A level meter bar filling `r` from the bottom (vertical) or the left: -40 dB empty, 0 dB
/// full; green, orange above -6 dB, red at full scale.
fn meter_bar(p: &Painter, r: Rect, db: f32, vertical: bool) {
    let f = ((db + 40.0) / 40.0).clamp(0.0, 1.0);
    if f <= 0.0 {
        return;
    }
    let col = if db >= -0.1 { hex(0xF00000) } else if db > -6.0 { hex(0xFF8C19) } else { hex(0x2DD728) };
    let fill = if vertical { Rect::from_min_max(pos2(r.min.x, r.max.y - r.height() * f), r.max) } else { Rect::from_min_max(r.min, pos2(r.min.x + r.width() * f, r.max.y)) };
    p.rect_filled(fill, 0.0, col);
}

/// Read the level meters from the audio thread about every 36 ms: a higher peak shows at
/// once, otherwise the level falls 2 dB per reading.
pub(crate) fn read_meters(app: &mut App, ui: &Ui) {
    let now = ui.input(|i| i.time);
    let key = egui::Id::new("master_meter_time");
    let last = ui.ctx().data(|d| d.get_temp::<f64>(key)).unwrap_or(now - 0.036);
    let elapsed = (now - last) as f32;
    let mut next = 0.036;
    if elapsed >= next {
        ui.ctx().data_mut(|d| d.insert_temp(key, now));
        if app.performance() {
            for (n, deck) in app.player.decks.iter().enumerate() {
                let peaks = std::mem::take(&mut deck.lock().peak);
                for (db, peak) in app.px.mix[n].shown_db.iter_mut().zip(peaks) {
                    *db = crate::mixer::meter_fall(*db, peak, elapsed);
                }
            }
        }
        for (db, peak) in app.px.master_db.iter_mut().zip(crate::audio::take_master_peak()) {
            *db = crate::mixer::meter_fall(*db, peak, elapsed);
        }
    } else {
        next -= elapsed;
    }
    // Playback schedules its own frames. Only wake a paused window while a
    // visible meter is falling or the sampler is still producing sound.
    let visible = app.px.master_db.iter().any(|db| *db > -40.0)
        || (app.performance() && app.px.mix.iter().any(|strip| strip.shown_db.iter().any(|db| *db > -100.0)));
    if visible && !app.player.transport_running() {
        ui.ctx().request_repaint_after_secs(next + ui.input(|i| i.predicted_dt));
    }
}

/// The mixer's bottom row: headphones MIX and LEVEL, the crossfader with its assign buttons.
pub(crate) fn bottom_row(app: &mut App, ui: &mut Ui, r: Rect, cx: f32) {
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, PANEL);
    let cy = r.min.y + 21.0;
    let x = |v: f32| cx + (v - 756.0);
    // Headphones.
    let hc = pos2(x(450.0), cy);
    p.add(egui::Shape::line((0..=16).map(|i| { let a = (180.0 + 180.0 * i as f32 / 16.0).to_radians(); hc + vec2(a.cos() * 7.5, a.sin() * 7.5 + 2.0) }).collect(), Stroke::new(1.8_f32, WHITE)));
    p.rect_filled(Rect::from_min_size(hc + vec2(-9.0, 1.5), vec2(3.5, 7.0)), 1.0, WHITE);
    p.rect_filled(Rect::from_min_size(hc + vec2(5.5, 1.5), vec2(3.5, 7.0)), 1.0, WHITE);
    let note = if crate::audio::HP_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) { "Headphones: MIX balances the cued decks (left) and the master (right); LEVEL is the headphones volume." } else { "Headphones.\nNo headphones output is set: choose one in Preferences > Audio > Input/Output (it needs a device with four or more outputs)." };
    if let Some(v) = knob(ui, pos2(x(491.0), cy), "hpmix", app.px.hp_mix, 0.5, WHITE.c()) {
        app.px.hp_mix = v;
    }
    p.text(pos2(x(513.5), cy - 0.25), Align2::CENTER_CENTER, "MIX", lay::arial_b(10.0), gy(0x8C8C8C));
    // (The dashes rekordbox has before both knobs.)
    for dx in [466.0, 530.0] {
        p.rect_filled(Rect::from_min_size(pos2(x(dx), cy - 0.5), vec2(8.0, 1.0)), 0.0, gy(0x646464));
    }
    if let Some(v) = knob(ui, pos2(x(554.5), cy), "hplevel", app.px.hp_level, 0.5, WHITE.c()) {
        app.px.hp_level = v;
    }
    // (LEVEL's ring is blue up to its pointer.)
    lay::ring_knob(&p, pos2(x(554.5), cy), 8.5, 3.0, app.px.hp_level, gy(0x323232), BLUE.c(), WHITE.c());
    p.text(pos2(x(584.25), cy - 0.25), Align2::CENTER_CENTER, "LEVEL", lay::arial_b(10.0), gy(0x8C8C8C));
    let _ = ui.interact(Rect::from_min_max(pos2(x(440.0), r.min.y), pos2(x(600.0), r.max.y)), ui.id().with("hpnote"), Sense::hover()).on_hover_text(note);

    // Crossfader assign: the left block puts a deck on side A, the right block on side B;
    // neither = THRU. Decks 3 and 4 do not exist yet.
    let mut changed = false;
    for side in 1..=2u8 {
        let bx = if side == 1 { x(629.0) } else { x(821.0) };
        for d in 0..4usize {
            let br = Rect::from_min_size(pos2(bx + (d % 2) as f32 * 32.0, r.min.y + 5.0 + (d / 2) as f32 * 16.0), vec2(30.0, 13.5));
            // (rekordbox lights the assignments of decks 3 and 4 in the 2-deck layouts too.)
            let exists = d < app.deck_count() as usize;
            let lit = if exists { app.px.mix[d].assign == side } else { (d % 2 + 1) as u8 == side };
            let resp = ui.interact(br, ui.id().with(("assign", side, d)), Sense::click());
            p.rect_filled(br, 1.0, if lit { BLUE.c() } else if resp.hovered() && exists { lighten(BTN.c()) } else { BTN.c() });
            p.text(br.center(), Align2::CENTER_CENTER, format!("{}", d + 1), lay::arial_b(10.0), WHITE.c());
            if resp.clicked() && exists {
                app.px.mix[d].assign = if lit { 0 } else { side };
                changed = true;
            }
        }
    }
    // Crossfader: 96 x 24 pt; a modifier-click or double click centres it.
    let (fl, fr) = (x(711.0), x(801.0));
    let cy = cy + 1.0;
    let rr = Rect::from_min_max(pos2(fl - 6.0, cy - 12.0), pos2(fr + 6.0, cy + 12.0));
    // (Disabled while Automix crossfades: automix.rs `crossfader_locked`.)
    let xf_locked = crate::automix::crossfader_locked(app);
    app.mixx.automix.ui.xfader = Some(rr);
    let resp = ui.interact(rr, ui.id().with("xfader"), if xf_locked { Sense::hover() } else { Sense::click_and_drag() });
    let mut xf = app.px.crossfader;
    if resp.double_clicked() || (resp.clicked() && ui.input(|i| i.modifiers.any())) {
        xf = 0.5;
        changed = true;
    } else if let Some(pp) = crate::ui_kit::slider_pos(ui, &resp, rr) {
        xf = ((pp.x - fl) / (fr - fl)).clamp(0.0, 1.0);
        changed = true;
    }
    p.rect_filled(Rect::from_min_max(pos2(fl, cy - 0.5), pos2(fr, cy + 0.5)), 0.0, gy(0x323232));
    p.rect_filled(Rect::from_min_max(pos2(fl, cy - 10.0), pos2(fl + 1.0, cy + 10.0)), 0.0, gy(0x323232));
    p.rect_filled(Rect::from_min_max(pos2(fr - 1.0, cy - 10.0), pos2(fr, cy + 10.0)), 0.0, gy(0x323232));
    let hx = fl + xf * (fr - fl);
    p.rect_filled(Rect::from_center_size(pos2(hx, cy), vec2(2.0, 24.0)), 0.0, if xf_locked { gy(0x646464).gamma_multiply(0.3) } else if resp.hovered() || resp.dragged() { WHITE.c() } else { gy(0x646464) });
    if changed {
        app.px.crossfader = xf;
        app.poll_decks();
    }
}
