//! The Preferences window, laid out as rekordbox's: categories on the left (View, Audio,
//! Analysis, DJ System, Keyboard, Advanced), tabs across the top, cards of settings. Also the
//! Keyboard Shortcuts window and the Backup / Restore Library windows.

use crate::analysis::BPM_RANGES;
use crate::app::*;
use crate::export::settings::{SettingFile, DEV_OVERVIEW_TYPE, DEV_WAVEFORM_COLOR, DEV_WAVEFORM_POSITION};
use crate::prefs_ext::{ACTIONS, ACTION_CATEGORIES};
use crate::theme::*;
use eframe::egui;
use crate::ui_kit::{self as kit, btn, card, inter, inter_bold, note, radio, separator, sub, toggle, PBLUE};
use egui::{pos2, vec2, Color32, Context, Rect, RichText, Ui};

// Settings another part of the app has to act on. While one is false its control is shown
// switched off with the reason, so nothing in the window pretends to work.
/// The device writer deletes an imported history (reads `prefs.delete_history_after_import`).
const HISTORY_DELETE_WIRED: bool = true;
/// The device writer stores `prefs.device_bg_color`.
const BG_COLOR_WRITTEN: bool = false;
// The player (ui_player.rs) is to act on these; set each to true once it does.
/// The enlarged waveform draws the phrases when `prefs.phrase_enlarged`.
const PHRASE_ENLARGED_WIRED: bool = true;
/// The phrase display shows phrase types always (`prefs.phrase_types_always`) or on hover.
const PHRASE_TYPES_WIRED: bool = true;
/// The enlarged waveform draws the vocal positions when `prefs.vocal_enlarged`.
const VOCAL_ENLARGED_WIRED: bool = true;
/// The stem strip's part buttons call `prefs_ext::stem_part_click` (MUTE / SOLO mode).
const STEM_SOLO_WIRED: bool = true;
/// The waveforms show only the parts that sound when `prefs.stem_waveform`.
const STEM_WAVEFORM_WIRED: bool = true;

use crate::prefs_rb::{self as rb, Mode};

/// Why rekordbox's PLAN and CLOUD categories are dimmed.
fn absent_reason(c: u8) -> Option<&'static str> {
    match c {
        rb::PLAN => Some("recordbox has no subscription plans."),
        rb::CLOUD => Some("recordbox has no cloud service."),
        _ => None,
    }
}

/// The mode the window is laid out for: the application's (PREPDECK_PREFS_MODE=export|
/// performance|lighting|edit overrides it for screenshots of modes recordbox cannot enter yet).
pub fn prefs_mode(app: &App) -> Mode {
    static FORCED: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    let forced = FORCED.get_or_init(|| match std::env::var("PREPDECK_PREFS_MODE").ok().as_deref() {
        Some("export") => Some(Mode::Export),
        Some("performance") => Some(Mode::Performance),
        Some("lighting") => Some(Mode::Lighting),
        Some("edit") => Some(Mode::Edit),
        _ => None,
    });
    forced.unwrap_or(Mode::of(app.prefs.app_mode))
}

fn tabs_of(page: u8, mode: Mode) -> &'static [&'static str] {
    rb::tabs(page, mode)
}

/// Every setting by name, for the search field: (page, tab, name).
const SETTINGS: &[(u8, &str, &str)] = &[
    (7, "Effect", "Apply CFX to Sampler using MIC COLOR control knob."),
    (7, "Sampler", "SLOT"),
    (7, "Sampler", "Do not overwrite the loaded Slot."),
    (7, "Sampler", "Capture the loop in an empty slot"),
    (7, "Sampler", "Auto Panel Mode"),
    (7, "Sampler", "Sequence Load"),
    (7, "Others", "SEQUENCER"),
    (0, "Display Type", "Language"),
    (0, "Display Type", "Show Tooltips"),
    (0, "Display Type", "Font Size"),
    (0, "Display Type", "Bold"),
    (0, "Display Type", "Line Space"),
    (0, "Display Type", "Show the selected track to the center"),
    (0, "Display Type", "Key display format"),
    (0, "Display Type", "Full/Preview Waveform"),
    (0, "Display Type", "Full Waveform"),
    (0, "Display Type", "Half Waveform"),
    (0, "Display Type", "Beat Count Display"),
    (0, "Layout", "Media Browser"),
    (0, "Layout", "Browser panel"),
    (0, "Layout", "CUE Analysis Playlist"),
    (0, "Layout", "Playlist Palette"),
    (0, "Layout", "Display Cue Markers on Preview"),
    (0, "Layout", "Playlist search filter"),
    (0, "Layout", "Display All Tracks in the Playlists"),
    (0, "Layout", "Display the number of tracks in a playlist on the Tree View"),
    (0, "Layout", "Display SYNC RATE"),
    (0, "Layout", "Phrase (Enlarged Waveform)"),
    (0, "Layout", "Phrase (Full Waveform)"),
    (0, "Layout", "Always show types of phrases"),
    (0, "Layout", "Vocal (Enlarged Waveform)"),
    (0, "Layout", "Vocal (Full Waveform)"),
    (0, "Color", "Appearance"),
    (0, "Color", "Waveform color"),
    (0, "Color", "HOT CUE color"),
    (7, "Others", "Quantize"),
    (4, "Others", "Tree View"),
    (4, "Others", "My Tag Configuration Window"),
    (4, "Others", "Column browser"),
    (4, "Others", "Sub-Browser Window"),
    (5, "Configuration", "Audio"),
    (5, "Configuration", "Driver"),
    (5, "Configuration", "Sample Rate"),
    (5, "Configuration", "Buffer size"),
    (5, "Configuration", "Metronome"),
    (5, "Configuration", "Click Sound"),
    (5, "Configuration", "Metronome Volume"),
    (5, "Configuration", "Preview Volume"),
    (5, "Input/Output", "Output channels"),
    (5, "Input/Output", "Master Output"),
    (1, "Track Analysis", "Beat grid estimator"),
    (1, "Track Analysis", "MP3 cue timing"),
    (1, "Track Analysis", "High precision: refine beat timing"),
    (1, "Track Analysis", "Analysis Mode"),
    (1, "Track Analysis", "BPM Range"),
    (1, "Track Analysis", "BPM / Grid"),
    (1, "Track Analysis", "KEY"),
    (1, "Track Analysis", "Phrase"),
    (1, "Track Analysis", "Phrase estimator"),
    (1, "Track Analysis", "Vocal"),
    (1, "Track Analysis", "Auto Analysis"),
    (1, "Track Analysis", "Auto Gain"),
    (1, "CUE Analysis", "Set Memory Cue at the first beat of track when analyzing"),
    (1, "CUE Analysis", "Set Hot Cue A on the first downbeat"),
    (2, "General", "Background Color"),
    (2, "General", "Waveform color displayed on CDJ/XDJ"),
    (2, "General", "Waveform Current Position"),
    (2, "General", "Waveform Divisions"),
    (2, "General", "Type of the Overview Waveform"),
    (2, "Category", "Category"),
    (2, "Sort", "Sort"),
    (2, "Column", "Column"),
    (2, "My Settings", "My Settings"),
    (2, "My Settings", "QUANTIZE"),
    (2, "My Settings", "AUTO CUE"),
    (2, "My Settings", "TIME MODE"),
    (2, "My Settings", "JOG MODE"),
    (2, "My Settings", "TEMPO RANGE"),
    (2, "My Settings", "MASTER TEMPO"),
    (2, "My Settings", "HOT CUE AUTO LOAD"),
    (2, "My Settings", "DJM mixer settings"),
    (2, "Device", "Delete playlist tracks from device if playlist is deleted"),
    (2, "Device", "Import the play history automatically"),
    (2, "Device", "Delete from the device after importing the play history"),
    (2, "Others", "Prepare Hot Cue Auto Load setting for CDJ/XDJ use (on import)"),
    (3, "Keyboard", "Keyboard shortcuts"),
    (4, "Database", "Database"),
    (4, "Others", "Backup Library"),
    (4, "Others", "Restore Library"),
    (4, "Database", "iTunes Library File"),
    (4, "Database", "Imported Library"),
    (4, "Database", "rekordbox xml"),
    (4, "Database", "Auto Relocate Search Folders"),
    (4, "Browse", "Add \"My Tag\" to the \"Comments\""),
    (4, "Browse", "Color comments"),
    (4, "Browse", "Customize Color Comments"),
    (4, "Others", "rekordbox Library"),
    (7, "Deck", "Eject/Load Lock"),
    (7, "Deck", "Instant Doubles"),
    (7, "Mixer", "EQ"),
    (7, "Mixer", "EQ/ISOLATOR"),
    (7, "Mixer", "Channel Fader Curve"),
    (6, "STEMS", "Enable the STEMS Function"),
    (6, "STEMS", "STEMS mode"),
    (6, "STEMS", "STEMS Analysis Process"),
    (6, "STEMS", "Prioritize sound quality"),
    (6, "STEMS", "Prioritize speed"),
    (6, "STEMS", "STEM Waveform Display"),
    (6, "STEMS", "ACTIVE STEM Setting"),
    (6, "STEMS", "MUTE / SOLO"),
    (6, "STEMS", "Reset the MUTE/SOLO state while loading"),
    (6, "STEMS", "Stem separation model"),
    (6, "STEMS", "Allow automatic separation on CPU"),
    (6, "STEMS", "Keep all saved stems"),
    (6, "STEMS", "STEMS storage budget"),
    (0, "Display Type", "Playback animation"),
    (1, "Track Analysis", "Analysis workers"),
    (1, "Track Analysis", "Use installed beat grid model"),
    (1, "Track Analysis", "Hardware setup"),
    (2, "Others", "Supported devices"),
    (0, "Display Type", "Tooltips"),
    (0, "Display Type", "Waveform Drawing Rate"),
    (0, "Display Type", "Display key information on the database"),
    (0, "Display Type", "Deck track information display"),
    (0, "Display Type", "Click on the waveform for PLAY and CUE"),
    (0, "Display Type", "Switch JOG Display"),
    (0, "Display Type", "Traffic Light"),
    (0, "Layout", "Effect Panel"),
    (0, "Layout", "Enlarged Waveform"),
    (0, "Layout", "PAD Mode"),
    (0, "Color", "PAD FX color"),
    (0, "Color", "SAMPLER color"),
    (5, "Configuration", "Control Device Information"),
    (5, "Input/Output", "Mixer Mode"),
    (5, "Input/Output", "Input channels"),
    (5, "Input/Output", "Headphones Output"),
    (5, "Input/Output", "Booth Output"),
    (1, "CUE Analysis", "Intelligent Cue Creation"),
    (2, "General", "Image On-Jog Display"),
    (2, "General", "Key display format"),
    (2, "Others", "LINK MONITOR"),
    (7, "Deck", "Needle Lock"),
    (7, "Deck", "Memory Cue Call Lock"),
    (7, "Deck", "Load"),
    (7, "Deck", "Auto Cue"),
    (7, "Deck", "Vinyl Speed Adjust"),
    (7, "Deck", "JOG"),
    (7, "Deck", "Continuous Playing"),
    (7, "Deck", "BEAT/BPM SYNC"),
    (7, "Deck", "HOT CUE"),
    (7, "Deck", "AUTO BEAT LOOP"),
    (7, "Mixer", "Auto Gain"),
    (7, "Mixer", "EQ Type"),
    (7, "Mixer", "CROSSFADER Curve"),
    (7, "Mixer", "Output Level"),
    (7, "Mixer", "Headroom"),
    (7, "Sampler", "Capture"),
    (7, "Others", "Mode"),
    (7, "Others", "SNAP"),
    (7, "Others", "Setting"),
    (7, "Others", "Ableton Link"),
    (8, "DVS", "Enable DVS Function"),
    (8, "DVS", "Control Signal Setting"),
    (8, "DVS", "Waveform display"),
    (8, "DVS", "Control Vinyl"),
    (8, "DVS", "Routing Setting"),
    (4, "Database", "Auto Export"),
    (4, "Database", "Database management"),
    (4, "Browse", "Display speed"),
    (4, "Browse", "Drop-down menu"),
    (4, "Browse", "Tag List"),
    (4, "Browse", "Library Protection"),
    (4, "Browse", "Library Edit"),
    (4, "Browse", "Playback time setting"),
    (4, "Others", "Computer Name"),
    (4, "Others", "QUANTIZE BEAT VALUE"),
    (4, "Others", "BEAT/BPM SYNC"),
    (4, "Others", "HOT CUE"),
    (4, "Others", "Export Loop As WAV"),
    (4, "Others", "Track information"),
    (4, "Others", "Time Mode"),
    (6, "STEMS", "Layout"),
    (6, "STEMS", "STEM Instant Doubles"),
    (6, "STEMS", "Memory"),
    (6, "STEMS", "Multi-thread"),
    (6, "STEMS", "Enable the GROOVE CIRCUIT Function"),
    (6, "STEMS", "DRUM SWAP"),
    (6, "STEMS", "DRUM CAPTURE"),
    (6, "STEMS", "Drum Loop Audio"),
];

/// Settings (and keyboard commands) of `mode`'s pages whose name contains the search text:
/// (category, tab, name).
pub fn search_settings(q: &str, mode: Mode) -> Vec<(u8, usize, String)> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let mut v = vec![];
    for &(page, tab, name) in SETTINGS {
        if !name.to_lowercase().contains(&q) || !rb::categories(mode).contains(&page) {
            continue;
        }
        if let Some(i) = tabs_of(page, mode).iter().position(|t| *t == tab) {
            if rb::tab_out_of_scope(page, tab).is_none() {
                v.push((page, i, name.to_string()));
            }
        }
    }
    if rb::categories(mode).contains(&rb::KEYBOARD) {
        for name in crate::ui_prefs_keys::search(&q, mode) {
            v.push((rb::KEYBOARD, 0, name));
        }
    }
    v
}

// ------------------------------------------------------------------ widgets

/// A switch that only works once `wired`; until then it is shown off, with the reason.
fn toggle_if(app: &mut App, ui: &mut Ui, wired: bool, pick: fn(&mut Prefs) -> &mut bool, label: &str, why: &str) {
    if wired {
        toggle(ui, pick(&mut app.prefs), label);
    } else if kit::toggle_off(ui, label, why) {
        app.toast(why);
    }
}

/// A shortcut as the window shows it. The app's font has no ⌃ or ⌥ glyph, so those two are
/// spelled out.
fn key_text(sc: &Shortcut) -> String {
    sc.label().replace('⌃', "Ctrl ").replace('⌥', "Opt ").replace('⇧', "Shift ")
}

fn path_row(ui: &mut Ui, value: &Option<String>, empty: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(500.0), 22.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 3.0, BG);
    let text = value.clone().unwrap_or_else(|| empty.to_string());
    let col = if value.is_some() { WHITE } else { DIM };
    ui.painter().with_clip_rect(r).text(pos2(r.min.x + 6.5, r.center().y), egui::Align2::LEFT_CENTER, text, inter(12.5), col.c());
}

/// The category icons of the sidebar, in a 30 pt box at `o` (shapes as the original's).
pub(crate) fn side_icon_drawn(p: &egui::Painter, o: egui::Pos2, page: u8, c: Color32, bg: Color32) {
    let at = |x: f32, y: f32| pos2(o.x + x, o.y + y);
    let bar = |x: f32, y: f32, w: f32, h: f32, col: Color32| p.rect_filled(Rect::from_min_size(at(x, y), vec2(w, h)), 0.0, col);
    match page {
        // View: an eye.
        0 => {
            let n = 28;
            let half = |sign: f32| (0..=n).map(move |i| {
                let u = -1.0 + 2.0 * i as f32 / n as f32;
                (15.005 + u * 8.27 * sign, 15.005 - sign * 5.79 * (u * std::f32::consts::FRAC_PI_2).cos().powf(0.92))
            });
            let pts: Vec<egui::Pos2> = half(1.0).chain(half(-1.0)).map(|(x, y)| at(x, y)).collect();
            p.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.29, c)));
            p.circle_filled(at(15.005, 15.005), 3.86, c);
        }
        // Audio: a speaker.
        5 => {
            p.add(egui::Shape::convex_polygon(vec![at(8.45, 8.68), at(16.17, 12.54), at(16.17, 17.69), at(8.45, 21.55)], c, egui::Stroke::NONE));
            bar(17.45, 12.54, 3.86, 5.15, c);
        }
        // Analysis: seven bars, the middle one tall.
        1 => {
            bar(14.36, 7.29, 1.29, 15.44, c);
            for x in [6.64, 9.22, 11.79, 16.93, 19.51, 22.08] {
                bar(x, 11.15, 1.29, 7.72, c);
            }
        }
        // DJ System: a player seen from above.
        2 => {
            bar(6.0, 8.57, 18.01, 12.87, c);
            p.circle_filled(at(11.147, 15.005), 3.86, bg);
            p.circle_filled(at(11.146, 15.005), 1.29, c);
            for y in [10.50, 13.08, 15.65, 18.22] {
                bar(16.29, y, 2.57, 1.29, bg);
                bar(20.15, y, 2.57, 1.29, bg);
            }
        }
        // Keyboard: a key cap with an A.
        3 => {
            p.rect_filled(Rect::from_min_max(at(7.66, 7.40), at(23.10, 22.83)), 1.29, c);
            p.text(at(15.38, 15.3), egui::Align2::CENTER_CENTER, "A", inter_bold(10.6), bg);
        }
        // PLAN: a disc with two quarter arcs cut out.
        100 => {
            p.circle_filled(at(15.007, 15.005), 7.72, c);
            let arc = |a0: f32| (0..=10).map(|i| { let a = a0 + i as f32 / 10.0 * std::f32::consts::FRAC_PI_2; at(15.007 + 5.145 * a.cos(), 15.005 + 5.145 * a.sin()) }).collect::<Vec<_>>();
            p.add(egui::Shape::line(arc(std::f32::consts::PI), egui::Stroke::new(2.57, bg)));
            p.add(egui::Shape::line(arc(0.0), egui::Stroke::new(2.57, bg)));
            p.circle_filled(at(15.007, 15.005), 2.57, bg);
        }
        // CLOUD: a cloud with the two sync arrows.
        101 => {
            p.circle_filled(at(14.36, 14.2), 5.63, c);
            p.circle_filled(at(9.86, 17.58), 3.86, c);
            p.circle_filled(at(20.0, 16.6), 4.2, c);
            p.rect_filled(Rect::from_min_max(at(9.86, 16.5), at(20.15, 21.44)), 1.0, c);
            let arc = |a0: f32| (0..=8).map(|i| { let a = a0 + i as f32 / 8.0 * 2.4; at(15.004 + 3.43 * a.cos(), 15.97 + 3.43 * a.sin()) }).collect::<Vec<_>>();
            p.add(egui::Shape::line(arc(std::f32::consts::PI), egui::Stroke::new(0.86, bg)));
            p.add(egui::Shape::line(arc(0.0), egui::Stroke::new(0.86, bg)));
            p.add(egui::Shape::line(vec![at(18.43, 12.11), at(18.43, 14.68), at(15.86, 14.68)], egui::Stroke::new(0.86, bg)));
            p.add(egui::Shape::line(vec![at(11.57, 19.83), at(11.57, 17.26), at(14.15, 17.26)], egui::Stroke::new(0.86, bg)));
        }
        // Controller: a box with two rings (skins/preferece_controller_off.svg).
        7 => {
            p.rect_filled(Rect::from_min_max(at(6.0, 8.57), at(24.01, 21.44)), 0.0, c);
            for cx in [11.145, 18.864] {
                p.circle_filled(at(cx, 15.005), 2.573, bg);
                p.circle_filled(at(cx, 15.005), 1.286, c);
            }
        }
        // Extensions: two gears (skins/preferece_extensions_off.svg: 9 pt across, eight teeth,
        // a 3.86 pt hole; centres 19.51, 11.79 and 10.50, 18.22).
        6 => {
            for (cx, cy) in [(19.508, 11.789), (10.504, 18.221)] {
                let c0 = at(cx, cy);
                p.circle_filled(c0, 3.5, c);
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4;
                    let (dir, side) = (vec2(a.sin(), -a.cos()), vec2(a.cos(), a.sin()));
                    p.add(egui::Shape::convex_polygon(vec![c0 + dir * 2.8 - side * 1.04, c0 + dir * 4.5 - side * 0.9, c0 + dir * 4.5 + side * 0.9, c0 + dir * 2.8 + side * 1.04], c, egui::Stroke::NONE));
                }
                p.circle_filled(c0, 1.93, bg);
            }
        }
        // Advanced: three lines.
        _ => {
            for y in [10.61, 14.47, 18.33] {
                bar(8.82, y, 12.86, 1.29, c);
            }
        }
    }
}

// ------------------------------------------------------------------ window

/// Width and height of the window's content (the original's 800 x 828 window less its
/// 28 pt title bar).
const WIN: egui::Vec2 = vec2(800.0, 800.0);

pub fn prefs_window(app: &mut App, ctx: &Context) {
    if !app.dialogs.prefs_open {
        app.dialogs.confirm_clear_stems = false;
        app.stem.cache_usage = None;
        return;
    }
    // Preferences opens in its own window, like the original. Screenshot mode (and backends
    // without multiple windows) draw it inside the main window instead.
    if app.screenshot.is_some() {
        prefs_embedded(app, ctx);
        return;
    }
    let vid = egui::ViewportId::from_hash_of("recordbox_preferences");
    let builder = egui::ViewportBuilder::default().with_title("Preferences").with_inner_size([WIN.x, WIN.y]).with_min_inner_size([WIN.x, 520.0]).with_max_inner_size([WIN.x, 4000.0]);
    ctx.show_viewport_immediate(vid, builder, |vctx, class| {
        if class == egui::ViewportClass::Embedded {
            prefs_embedded(app, vctx);
            return;
        }
        crate::menus::apply_edit_commands(app, vctx);
        let assigning_key = app.dialogs.prefs_tab == 3 && app.dialogs.ext.prefs_search.trim().is_empty()
            && crate::ui_prefs_keys::assigning_key(vctx);
        if !assigning_key {
            crate::window_fullscreen(&app.prefs, vctx, None);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG.c())).show(vctx, |ui| {
            let h = ui.available_height();
            prefs_body(app, ui, h)
        });
        if vctx.input(|i| i.viewport().close_requested()) {
            app.dialogs.prefs_open = false;
        }
    });
}

fn prefs_embedded(app: &mut App, ctx: &Context) {
    // Screenshots: the window's content alone at the top left of the picture, at the
    // original's size, so it can be laid over the reference.
    if app.screenshot.is_some() {
        egui::Area::new(egui::Id::new("prefs_flat")).fixed_pos(pos2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            let r = Rect::from_min_size(pos2(0.0, 0.0), WIN);
            ui.set_min_size(WIN);
            ui.painter().rect_filled(r, 0.0, BG);
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| prefs_body(app, ui, WIN.y));
        });
        return;
    }
    let mut open = true;
    let frame = egui::Frame::window(&ctx.style()).fill(BG.c()).inner_margin(0.0);
    let h = (ctx.screen_rect().height() - 90.0).clamp(480.0, WIN.y);
    egui::Window::new("Preferences").open(&mut open).default_size(vec2(WIN.x, h)).collapsible(false).resizable(false).frame(frame).show(ctx, |ui| prefs_body(app, ui, h));
    app.dialogs.prefs_open &= open;
}

/// Screenshots only: PREPDECK_PREFS_JSON='{"stem_solo":true,"rb":{"needle_lock":false}}' is
/// laid over the settings once, so a page can be pictured in the state a reference shows.
fn shot_overrides(app: &mut App) {
    static DONE: std::sync::Once = std::sync::Once::new();
    if app.screenshot.is_none() {
        return;
    }
    DONE.call_once(|| {
        let Ok(text) = std::env::var("PREPDECK_PREFS_JSON") else { return };
        let (Ok(over), Ok(mut cur)) = (serde_json::from_str::<serde_json::Value>(&text), serde_json::to_value(&app.prefs)) else { return };
        fn merge(a: &mut serde_json::Value, b: &serde_json::Value) {
            match (a, b) {
                (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                    for (k, v) in b {
                        merge(a.entry(k.clone()).or_insert(serde_json::Value::Null), v);
                    }
                }
                (a, b) => *a = b.clone(),
            }
        }
        merge(&mut cur, &over);
        match serde_json::from_value::<Prefs>(cur) {
            Ok(p) => app.prefs = p,
            Err(e) => eprintln!("PREPDECK_PREFS_JSON: {e}"),
        }
    });
}

/// PREPDECK_SHOT_SCROLL=<points>: how far down the page a screenshot starts.
fn shot_scroll() -> Option<f32> {
    static V: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("PREPDECK_SHOT_SCROLL").ok().and_then(|v| v.parse().ok()))
}

/// The context the window was last drawn in (the scripted run reads the open tab from it).
pub fn last_ctx() -> Option<Context> {
    LAST_CTX.lock().clone()
}

static LAST_CTX: parking_lot::Mutex<Option<Context>> = parking_lot::Mutex::new(None);

fn prefs_body(app: &mut App, ui: &mut Ui, height: f32) {
    crate::ui_prefs_item::begin_window();
    shot_overrides(app);
    if crate::ui_prefs_item::TRACE.lock().on {
        *LAST_CTX.lock() = Some(ui.ctx().clone());
    }
    kit::apply_style(ui);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let o = ui.cursor().min;
    let (full, _) = ui.allocate_exact_size(vec2(WIN.x, height), egui::Sense::hover());
    let at = |x: f32, y: f32| pos2(o.x + x, o.y + y);
    ui.painter().rect_filled(full, 0.0, BG);
    // Sidebar 160 pt, #1E1E1E: search field, then the categories 33 pt apart.
    ui.painter().rect_filled(Rect::from_min_max(o, at(160.0, height)), 0.0, BG2);
    let sr = Rect::from_min_max(at(8.0, 16.0), at(152.0, 40.0));
    ui.painter().rect_filled(sr, 0.0, BG);
    let lens = egui::Stroke::new(1.0, gy(0x555555));
    ui.painter().circle_stroke(at(12.0 + 6.857, 20.0 + 6.857), 5.214, lens);
    ui.painter().line_segment([at(12.0 + 10.64, 20.0 + 11.075), at(12.0 + 14.07, 20.0 + 14.5)], lens);
    let field = Rect::from_min_max(pos2(sr.min.x + 24.0, sr.min.y + 3.0), pos2(sr.max.x - 2.0, sr.max.y));
    ui.put(field, egui::TextEdit::singleline(&mut app.dialogs.ext.prefs_search).frame(false).font(inter(13.0)).text_color(WHITE.c()));
    let mode = prefs_mode(app);
    let cats = rb::categories(mode);
    // A category the mode does not have (the mode was changed since): the first real one.
    if !cats.contains(&app.dialogs.prefs_tab) || app.dialogs.prefs_tab >= 100 {
        app.dialogs.prefs_tab = rb::VIEW;
    }
    for (n, i) in cats.iter().enumerate() {
        let name = rb::category_name(*i);
        let r = Rect::from_min_size(at(8.0, 56.0 + n as f32 * 33.0), vec2(144.0, 30.0));
        if let Some(why) = absent_reason(*i) {
            crate::ui_prefs_keys::side_icon(ui, r.min, *i, false, true);
            crate::prefs_font::draw(ui, Rect::from_min_size(pos2(r.min.x + 40.0, r.min.y), vec2(104.0, 30.0)), name, 15.0, false, OFF.c(), kit::Just::Left);
            ui.interact(r, ui.id().with(("prefs_absent", *i)), egui::Sense::hover()).on_hover_text(why);
            continue;
        }
        let sel = app.dialogs.prefs_tab == *i && app.dialogs.ext.prefs_search.trim().is_empty();
        let resp = ui.interact(r, ui.id().with(("prefs_page", *i)), egui::Sense::click());
        let bg = if sel { PBLUE } else { crate::ui_feel::hl(&resp, BG2.c(), gy(0x2A2A2A)) };
        if bg != BG2 {
            ui.painter().rect_filled(r, 0.0, bg);
        }
        crate::ui_prefs_keys::side_icon(ui, r.min, *i, sel, false);
        crate::prefs_font::draw(ui, Rect::from_min_size(pos2(r.min.x + 40.0, r.min.y), vec2(104.0, 30.0)), name, 15.0, false, if sel { ON_ACCENT.c() } else { PREF_CAT.c() }, kit::Just::Left);
        crate::ui_prefs_item::tab_seen(name, r);
        if resp.clicked() {
            app.dialogs.prefs_tab = *i;
            app.dialogs.ext.prefs_search.clear();
        }
    }
    let page = app.dialogs.prefs_tab;
    let tabs = tabs_of(page, mode);
    let key = format!("prefs_subtab_{page}");
    let mut sub_tab: usize = ui.ctx().data_mut(|d| *d.get_temp_mut_or(egui::Id::new(&key), 0usize)).min(tabs.len() - 1);
    if rb::tab_out_of_scope(page, tabs[sub_tab]).is_some() {
        sub_tab = tabs.iter().position(|t| rb::tab_out_of_scope(page, t).is_none()).unwrap_or(0);
    }
    let searching = !app.dialogs.ext.prefs_search.trim().is_empty();
    if page != rb::EXTENSIONS || tabs[sub_tab] != "STEMS" || searching {
        app.dialogs.confirm_clear_stems = false;
        app.stem.cache_usage = None;
    }
    // Tab strip on black, 30 pt tall: the active tab blue, the others #1E1E1E with #AAAAAA
    // text; a tab is its text and 4 pt wide more, 4 pt apart. (Measured on the captures: the
    // tab captions fit Font(14), the category names Font(15).)
    let mut x = 164.0;
    let names: Vec<&str> = if searching { vec!["Search results"] } else { tabs.to_vec() };
    for (i, t) in names.iter().enumerate() {
        let sel = searching || sub_tab == i;
        let out = if searching { None } else { rb::tab_out_of_scope(page, t) };
        let w = crate::prefs_font::width(ui, t, 14.0, false).round() + 4.0;
        if x + w > WIN.x - 2.0 {
            break;
        }
        let r = Rect::from_min_size(at(x, 4.0), vec2(w, 30.0));
        let resp = ui.interact(r, ui.id().with(("prefs_tab", page, i)), egui::Sense::click());
        ui.painter().rect_filled(r, 0.0, if sel { PBLUE } else { crate::ui_feel::hl_if(&resp, out.is_none(), BG2.c(), gy(0x2A2A2A)) });
        crate::prefs_font::draw(ui, r, t, 14.0, false, if sel { ON_ACCENT.c() } else if out.is_some() { OFF.c() } else { PREF_CAT.c() }, kit::Just::Centre);
        crate::ui_prefs_item::tab_seen(t, r);
        if let Some(why) = out {
            resp.on_hover_text(why);
        } else if resp.clicked() && !searching {
            sub_tab = i;
        }
        x += w + 4.0;
    }
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(&key), sub_tab));
    app.prefs.rb.last_page[mode.index()] = (page, sub_tab as u8);
    crate::ui_prefs_item::begin_frame();
    let before = format!("{:?}", app.prefs);
    // The cards, from 38 pt down to 40 pt above the bottom edge. DJ System > General has no
    // "Reset to defaults" in the original: its cards run to 4 pt above the bottom edge.
    let no_reset = false;
    let area = Rect::from_min_max(at(164.0, 38.0), at(WIN.x, height - if no_reset { 4.0 } else { 40.0 }));
    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
    let mut scroll = egui::ScrollArea::vertical().id_salt(("prefs_scroll", page, sub_tab, searching)).max_height(area.height()).auto_shrink([false, false]);
    if let Some(y) = shot_scroll().filter(|_| app.screenshot.is_some()) {
        scroll = scroll.vertical_scroll_offset(y);
    }
    scroll.show(&mut content, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        if searching {
            search_results(app, ui);
        } else {
            // (Controller's tabs: 0 Deck, 1 Mixer, 2 Effect, 3 Sampler, 4 Recordings,
            // 5 MIX POINT LINK, 6 Others. Advanced's tabs depend on the mode and are told
            // apart by name inside `page_advanced`.)
            match page {
                0 => page_view(app, ui, sub_tab),
                1 => page_analysis(app, ui, sub_tab),
                2 => page_dj_system(app, ui, sub_tab),
                3 => page_keyboard(app, ui, sub_tab),
                5 => page_audio(app, ui, sub_tab),
                6 => page_stems(app, ui),
                7 if sub_tab == 0 => page_deck(app, ui),
                7 if sub_tab == 1 => {
                    page_mixer(app, ui);
                    // TakeOver, Needle Lock and JOG cards while a controller is connected.
                    if crate::ui_midi::prefs_rows_shown(app) {
                        crate::ui_midi::prefs_rows(app, ui);
                    }
                }
                7 if sub_tab == 2 => crate::ui_fx::prefs_effect_page(app, ui),
                7 if sub_tab == 3 => page_sampler(app, ui),
                7 if sub_tab == 4 => crate::ui_rec::prefs_page(app, ui),
                7 if sub_tab == 5 => crate::ui_mpl::prefs_page(app, ui),
                7 => page_controller_others(app, ui),
                8 => crate::ui_prefs_perf::page_dvs(app, ui),
                _ => page_advanced(app, ui, sub_tab),
            }
        }
    });
    if format!("{:?}", app.prefs) != before {
        app.save_prefs();
    }
    let foot = Rect::from_min_max(at(164.5, height - 35.75), at(WIN.x - 8.0, height - 4.75));
    if !no_reset || app.dialogs.ext.reset_ask.is_some() {
        let mut foot_ui = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)));
        reset_row(app, &mut foot_ui);
    }
}

fn search_results(app: &mut App, ui: &mut Ui) {
    let mode = prefs_mode(app);
    let hits = search_settings(&app.dialogs.ext.prefs_search, mode);
    card(ui, "Search results", |ui| {
        if hits.is_empty() {
            kit::line(ui, "No setting has that in its name.");
        }
        kit::block(ui, |ui| {
            for (page, tab, name) in hits {
                let place = format!("{} > {}", rb::category_name(page), tabs_of(page, mode).get(tab).copied().unwrap_or(""));
                ui.horizontal(|ui| {
                    if ui.add(btn(name.as_str()).min_size(vec2(300.0, 22.0))).clicked() {
                        app.dialogs.prefs_tab = page;
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(format!("prefs_subtab_{page}")), tab));
                        app.dialogs.ext.prefs_search.clear();
                    }
                    ui.label(kit::text(place, 12.0).color(DIM));
                });
            }
        });
    });
}

/// "Reset to defaults": asks first, in the original's words; keyboard shortcuts, columns and
/// this computer's devices and folders are kept.
fn reset_row(app: &mut App, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.x = 8.0;
    match app.dialogs.ext.reset_ask {
        None => {
            if ui.add(btn("Reset to defaults").exact(vec2(249.0, 31.0))).clicked() {
                app.dialogs.ext.reset_ask = Some(None);
            }
        }
        Some(scope) => {
            let text = if scope == Some(3) { "Are you sure you want to reset all the key-mappings to their default state?" } else { "Do you want to reset the settings to defaults ?" };
            ui.label(kit::text(text, 13.0).color(WHITE)).on_hover_text("Keyboard shortcuts, columns and this computer's devices and folders are kept.");
            if ui.add(btn("OK").exact(vec2(80.0, 29.0))).clicked() {
                if scope == Some(3) {
                    let mode = prefs_mode(app);
                    crate::ui_prefs_keys::reset_current(&mut app.prefs, mode);
                } else {
                    app.prefs = reset_prefs(&app.prefs);
                    app.pool.set_threads(app.prefs.analysis_threads);
                    crate::prefs_ext::set_stream_opts(&app.prefs);
                    let _ = app.player.switch_device(app.prefs.audio_device.as_deref());
                    app.list_dirty = true;
                }
                app.save_prefs();
                app.dialogs.ext.reset_ask = None;
            }
            if ui.add(btn("Cancel").exact(vec2(80.0, 29.0))).clicked() {
                app.dialogs.ext.reset_ask = None;
            }
        }
    }
}

/// Default settings, keeping what is not a "setting" in the window's sense: the keyboard
/// profile, the track list's columns and sort, and this computer's devices and folders.
pub fn reset_prefs(old: &Prefs) -> Prefs {
    Prefs {
        keymap: old.keymap.clone(),
        columns: old.columns.clone(),
        columns_other: old.columns_other.clone(),
        columns_for_playlist: old.columns_for_playlist,
        sort_col: old.sort_col,
        sort_asc: old.sort_asc,
        sort_other: old.sort_other,
        widths: old.widths.clone(),
        devices_extra: old.devices_extra.clone(),
        audio_device: old.audio_device.clone(),
        stem_model: old.stem_model.clone(),
        relocate_folders: old.relocate_folders.clone(),
        itunes_xml: old.itunes_xml.clone(),
        rekordbox_xml_path: old.rekordbox_xml_path.clone(),
        other: old.other.clone(),
        rb: crate::prefs_rb::RbPrefs { key_mapping: old.rb.key_mapping.clone(), key_mappings: old.rb.key_mappings.clone(), shell: old.rb.shell.clone(), last_page: old.rb.last_page, ..Default::default() },
        ..Prefs::default()
    }
}

// ------------------------------------------------------------------ View

fn page_view(app: &mut App, ui: &mut Ui, tab: usize) {
    let mode = prefs_mode(app);
    crate::ui_prefs_view::page_view(app, ui, tab, mode);
}

// ------------------------------------------------------------------ Audio

fn apply_audio(app: &mut App) {
    crate::prefs_ext::set_stream_opts(&app.prefs);
    if let Err(e) = app.player.switch_device(app.prefs.audio_device.as_deref()) {
        // The device refused: go back to its own defaults.
        app.prefs.audio_rate = 0;
        app.prefs.audio_buffer = 0;
        app.prefs.audio_channel = 0;
        crate::prefs_ext::set_stream_opts(&app.prefs);
        let _ = app.player.switch_device(app.prefs.audio_device.as_deref());
        app.toast_err(format!("The audio device did not accept that setting: {e}"));
    }
}

/// Audio devices and what the one in use offers. Both questions can block, so they are asked
/// in the background (once per run, or again after Refresh).
struct AudioInfo {
    /// None while waiting, Some(None) if macOS gave no answer.
    state: Option<Option<Vec<String>>>,
    devices: Vec<String>,
    caps: Option<crate::prefs_ext::AudioCaps>,
    list_id: egui::Id,
    caps_id: egui::Id,
}

type DevSlot = std::sync::Arc<parking_lot::Mutex<Option<Option<Vec<String>>>>>;
type CapSlot = std::sync::Arc<parking_lot::Mutex<Option<Option<crate::prefs_ext::AudioCaps>>>>;

fn audio_info(app: &App, ui: &Ui) -> AudioInfo {
    let list_id = egui::Id::new("audio_devices");
    let slot: DevSlot = match ui.ctx().data(|d| d.get_temp::<DevSlot>(list_id)) {
        Some(s) => s,
        None => {
            let s: DevSlot = Default::default();
            let s2 = s.clone();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                *s2.lock() = Some(crate::audio::Player::output_devices_within(std::time::Duration::from_secs(5)));
                ctx.request_repaint();
            });
            ui.ctx().data_mut(|m| m.insert_temp(list_id, s.clone()));
            s
        }
    };
    let state = slot.lock().clone();
    let devices: Vec<String> = match &state {
        Some(Some(d)) => d.clone(),
        _ => vec![],
    };
    let current = app.prefs.audio_device.clone();
    let caps_id = egui::Id::new(("audio_caps", current.clone()));
    let cap_slot: CapSlot = match ui.ctx().data(|d| d.get_temp::<CapSlot>(caps_id)) {
        Some(s) => s,
        None => {
            let s: CapSlot = Default::default();
            let s2 = s.clone();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                *s2.lock() = Some(crate::prefs_ext::audio_caps_within(current, std::time::Duration::from_secs(5)));
                ctx.request_repaint();
            });
            ui.ctx().data_mut(|m| m.insert_temp(caps_id, s.clone()));
            s
        }
    };
    let caps = cap_slot.lock().clone().flatten();
    AudioInfo { state, devices, caps, list_id, caps_id }
}

fn page_audio(app: &mut App, ui: &mut Ui, tab: usize) {
    let info = audio_info(app, ui);
    let mode = prefs_mode(app);
    if tab == 1 {
        let chans = info.caps.as_ref().map(|c| c.channels).unwrap_or(2);
        let dev = info.caps.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        if crate::ui_prefs_view::audio_io(app, ui, mode, chans, &dev) {
            apply_audio(app);
        }
        return;
    }
    let AudioInfo { state, devices, caps, list_id, caps_id } = info;
    let current = app.prefs.audio_device.clone();
    card(ui, "Audio", |ui| {
        let shown = current.clone().or_else(|| caps.as_ref().map(|c| c.name.clone()).filter(|n| !n.is_empty())).unwrap_or_else(|| "System default".to_string());
        let mut pick: Option<Option<String>> = None;
        kit::combo_row(ui, 22.0, |ui| {
            kit::combo("audio_dev", 386.0, shown, 14.0).show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), "System default").clicked() {
                    pick = Some(None);
                }
                for d in &devices {
                    if ui.selectable_label(current.as_deref() == Some(d.as_str()), d).clicked() {
                        pick = Some(Some(d.clone()));
                    }
                }
                if devices.is_empty() && state.is_some() {
                    ui.label(RichText::new("No audio device").color(DIM));
                }
            });
            ui.add_space(4.0);
            // rekordbox opens the device driver's own settings here; on a Mac those are in
            // Audio MIDI Setup.
            #[cfg(target_os = "macos")]
            if ui.add(btn("Driver").exact(vec2(148.0, 22.0)).font_size(11.5)).on_hover_text("Open Audio MIDI Setup, where macOS keeps the settings of audio devices.").clicked() {
                let _ = std::process::Command::new("/usr/bin/open").args(["-a", "Audio MIDI Setup"]).spawn();
            }
        });
        if let Some(p) = pick {
            // Another device has its own rates, buffers and channels: start from its defaults.
            let keep = (app.prefs.audio_rate, app.prefs.audio_buffer, app.prefs.audio_channel);
            app.prefs.audio_rate = 0;
            app.prefs.audio_buffer = 0;
            app.prefs.audio_channel = 0;
            crate::prefs_ext::set_stream_opts(&app.prefs);
            match app.player.switch_device(p.as_deref()) {
                Ok(()) => app.prefs.audio_device = p,
                Err(e) => {
                    (app.prefs.audio_rate, app.prefs.audio_buffer, app.prefs.audio_channel) = keep;
                    crate::prefs_ext::set_stream_opts(&app.prefs);
                    app.toast_err(format!("Could not open that audio device: {e}"));
                }
            }
        }
        // The original leaves 32 pt free under the dropdown; Recordbox says there what is
        // running, and anything that went wrong.
        let dev = caps.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        let (status, warn) = match (&state, &current) {
            (None, _) => ("Looking for audio devices…".to_string(), false),
            (Some(None), _) => ("The audio device list timed out. The system's default output is used.".to_string(), true),
            (Some(Some(_)), Some(d)) if !devices.contains(d) => (format!("\"{d}\" is not connected; the system's default output is used."), true),
            _ => (format!("Running at {} Hz{}.", app.player.device_rate, if dev.is_empty() { String::new() } else { format!(" on {dev}") }), false),
        };
        kit::row_at(ui, kit::Kind::Note, 18.0, 52.0, |ui| {
            ui.label(kit::text(status, 11.0).color(if warn { CAUTION_TX } else { DIM.c() }));
            ui.add_space(10.0);
            if ui.add(btn("Refresh").small()).on_hover_text("Look for audio devices again").clicked() {
                ui.ctx().data_mut(|m| {
                    m.remove::<DevSlot>(list_id);
                    m.remove::<CapSlot>(caps_id);
                });
            }
        });
        ui.add_space(8.0);
    });
    card(ui, "Sample Rate", |ui| {
        let rate_text = |r: u32| format!("{} Hz", r);
        let sel = rate_text(if app.prefs.audio_rate == 0 { app.player.device_rate } else { app.prefs.audio_rate });
        let mut new_rate = None;
        kit::combo_row(ui, 22.0, |ui| {
            ui.add_enabled_ui(caps.is_some(), |ui| {
                kit::combo("audio_rate", 536.0, sel, 14.0).show_ui(ui, |ui| {
                    if ui.selectable_label(app.prefs.audio_rate == 0, "Device setting").clicked() {
                        new_rate = Some(0);
                    }
                    for r in caps.as_ref().map(|c| c.rates.clone()).unwrap_or_default() {
                        if ui.selectable_label(app.prefs.audio_rate == r, rate_text(r)).clicked() {
                            new_rate = Some(r);
                        }
                    }
                });
            });
        });
        if let Some(r) = new_rate.filter(|r| *r != app.prefs.audio_rate) {
            app.prefs.audio_rate = r;
            apply_audio(app);
        }
    });
    card(ui, "Buffer size", |ui| {
        let range = caps.as_ref().and_then(|c| c.buffer);
        let b = app.prefs.audio_buffer;
        let label = if b == 0 {
            "Device setting".to_string()
        } else {
            format!("{b} samples ({:.1} ms)", b as f64 / app.player.device_rate.max(1) as f64 * 1000.0)
        };
        ui.add_space(1.0);
        kit::row_at(ui, kit::Kind::Ctl, 16.0, 57.5, |ui| {
            let r = ui.max_rect();
            kit::paint_text(ui, pos2(r.min.x, r.center().y), egui::Align2::LEFT_CENTER, &label, inter(14.0), WHITE.c(), 0.1);
        });
        ui.add_space(5.0);
        let mut t = if b == 0 { 0.0 } else { crate::prefs_ext::buffer_to_slider(b) };
        let resp = ui.add_enabled_ui(caps.is_some(), |ui| kit::slider(ui, &mut t)).inner;
        // The stream is reopened when the handle is let go, not at every step on the way.
        let drag_id = egui::Id::new("audio_buffer_drag");
        let mut pending: Option<u32> = ui.ctx().data(|d| d.get_temp(drag_id)).flatten();
        if resp.changed() {
            pending = Some(crate::prefs_ext::slider_to_buffer(t, range));
        }
        if let Some(nb) = pending {
            app.prefs.audio_buffer = nb;
            if !resp.is_pointer_button_down_on() {
                pending = None;
                if nb != b || resp.drag_stopped() || resp.clicked() {
                    apply_audio(app);
                }
            }
        }
        ui.ctx().data_mut(|d| d.insert_temp(drag_id, pending));
        resp.on_hover_text("64 to 2048 samples. A larger buffer avoids drop-outs; a smaller one reacts sooner. \"Reset to defaults\" goes back to the device's own setting.");
    });
    kit::card_x(ui, "Metronome", vec2(-8.0, -5.0), None, |ui| {
        for (i, n) in ["Click Sound 01", "Click Sound 02", "Click Sound 03"].iter().enumerate() {
            if radio(ui, app.prefs.metronome_sound == i as u8, n) {
                app.prefs.metronome_sound = i as u8;
                crate::prefs_ext::set_stream_opts(&app.prefs);
            }
        }
        kit::separator_col(ui, gy(0xFEFEFE));
        sub(ui, "Volume");
        kit::indent(ui, 88.0);
        let vol = app.prefs.metronome_vol.clamp(1, 3);
        for (i, n) in ["Small", "Middle", "Large"].iter().enumerate() {
            if radio(ui, vol == i as u8 + 1, n) {
                app.set_metronome_volume(i as u8 + 1);
            }
        }
    });
    if mode == Mode::Performance {
        preview_and_device(app, ui);
    }
}

/// Audio > Configuration, Performance mode only: Preview Volume and Control Device Information.
fn preview_and_device(app: &mut App, ui: &mut Ui) {
    crate::ui_prefs_item::item(ui, "AudioPreviewVolume", 96.0, |it| {
        it.title("Preview Volume");
        it.hint("How loud tracks play from the Preview column. In the middle they play at the master volume.");
        let mut t = app.prefs.preview_volume.clamp(0.0, 1.0);
        if it.slider(52.0, 56.0, 536.0, &mut t).changed() {
            app.prefs.preview_volume = t;
        }
    });
    crate::ui_prefs_item::item(ui, "AudioControlDevice", 98.0, |it| {
        it.title("Control Device Information");
        it.hint("recordbox is not connected to a controller.");
        it.text(52.0, 56.0, 550.0, 18.0, "-", 16.0, kit::Just::TopLeft);
    });
}

#[allow(dead_code)]
fn preview_volume_old(app: &mut App, ui: &mut Ui) {
    card(ui, "Preview Volume", |ui| {
        let mut t = app.prefs.preview_volume.clamp(0.0, 1.0);
        if kit::slider(ui, &mut t).on_hover_text("How loud tracks play from the Preview column. In the middle they play at the master volume.").changed() {
            app.prefs.preview_volume = t;
        }
    });
}

// ------------------------------------------------------------------ Analysis

fn page_analysis(app: &mut App, ui: &mut Ui, tab: usize) {
    if tab == 1 {
        card(ui, "Intelligent Cue Creation", |ui| {
            toggle(ui, &mut app.prefs.memory_cue_first_beat, "Set Memory Cue at the first beat of track when analyzing");
            note(ui, "For tracks that have no memory cue yet.");
            separator(ui);
            toggle(ui, &mut app.prefs.auto_first_downbeat_cue, "Set Hot Cue A on the first downbeat (\"1.1Bars\") when analyzing");
            note(ui, "As rekordbox does on first analysis. Only for tracks that have no cues yet; tracks from your rekordbox library keep their own cues.");
        });
        return;
    }
    card(ui, "Hardware setup", |ui| {
        kit::line(ui, &app.hardware_setup_summary());
        note(ui, &app.hardware_extensions_summary());
        note(ui, "Track analysis uses CPU workers. GPU acceleration for stem separation has its own settings under Extensions → STEMS.");
    });
    let no_cloud = "recordbox has no cloud service: nothing is collected or uploaded.";
    card(ui, "Track Analysis", |ui| {
        kit::labeled(ui, "Beat grid estimator", |ui| {
            let choices = [(false, "Rekordbox-compatible (default)"), (true, "recordbox classic")];
            let selected = choices[usize::from(app.prefs.use_recordbox_estimator)].1;
            kit::combo("beatgrid_estimator", 400.0, selected, 14.0).show_ui(ui, |ui| {
                for (classic, name) in choices {
                    ui.selectable_value(&mut app.prefs.use_recordbox_estimator, classic, name);
                }
            });
        });
        note(ui, "Rekordbox-compatible uses the port of the original detector. Classic uses the earlier recordbox detector. Both support Normal and Dynamic.");
        kit::sub_help(ui, "MP3 cue timing", Some("Some MP3 encoders add a short lead-in. On one track the same cue might land about 25 ms earlier or later on a kick. On another it can land in exactly the same place. Both modes read the original audio file."));
        match crate::audio::mp3_timeline() {
            crate::audio::Mp3Timeline::Legacy => {
                kit::line(ui, "Active timing · Standard recordbox (usual collection)");
                note(ui, "Listen to a cue imported from AlphaTheta Rekordbox. Restart with --mp3-timeline rekordbox and import the same track and cue into the separate trial collection. Start normally to return.");
            }
            crate::audio::Mp3Timeline::Rekordbox => {
                kit::line(ui, "Active timing · Rekordbox-compatible (separate trial collection)");
                note(ui, "Import a cue you know from AlphaTheta Rekordbox. Jump to it and listen for the start of the beat before exporting or writing cues back. Restart normally to return to the cues in your usual collection.");
            }
        }
        kit::sub_help(ui, "Track Analysis Mode", Some("Normal: optimized for tracks which have a constant BPM.\nDynamic: for tracks whose tempo changes, the beat grid follows the tempo."));
        kit::labeled(ui, "Analysis Mode", |ui| {
            let modes = [("Normal", "Optimized for tracks which have a constant BPM"), ("Dynamic", "For tracks whose tempo changes: the beat grid follows the tempo")];
            let cur = usize::from(app.prefs.analysis_mode == 1);
            kit::combo("amode", 400.0, modes[cur].0, 14.0).show_ui(ui, |ui| {
                for (i, (n, tip)) in modes.iter().enumerate() {
                    if ui.selectable_label(cur == i, *n).on_hover_text(*tip).clicked() {
                        app.prefs.analysis_mode = i as u8;
                    }
                }
            });
        });
        kit::row(ui, 16.0, |ui| {
            kit::toggle_raw(ui, &mut app.prefs.refine_beatgrid_transients, "High precision: refine beat timing");
            kit::help(ui, 37.5, "Align beats and tempo to repeated attacks in the audio. Works with either estimator in Normal and Dynamic mode.");
        });
        let dynamic = app.local_grid_available(true);
        let local = if app.prefs.analysis_mode == 1 { dynamic } else { app.local_grid_available(false).zip(dynamic).map(|(steady, changing)| steady || changing) };
        let grid_reason = match local {
            None => Some("Checking the local beat grid model files."),
            Some(false) if app.prefs.analysis_mode == 1 => Some("The local Dynamic beat grid model is missing. Restart recordbox after installing the model to check again."),
            Some(false) => Some("No local beat grid model was found. Restart recordbox after installing the model to check again."),
            Some(true) => None,
        };
        capability_toggle(ui, &mut app.prefs.use_installed_beatgrid_model, "Use installed beat grid model", grid_reason);
        if grid_reason.is_none() {
            note(ui, "On eligible tracks up to 15 minutes long, Normal mode picks the model for a steady or changing tempo. Dynamic uses the changing-tempo model. The selected estimator supplies the grid if the local model cannot run.");
        }
        let busy = app.pool.pending.load(std::sync::atomic::Ordering::Acquire) != 0;
        kit::labeled(ui, "Analysis workers", |ui| {
            let before = app.prefs.analysis_threads;
            let label = if before == 0 { format!("Automatic ({})", app.pool.thread_count()) } else { app.pool.thread_count().to_string() };
            ui.add_enabled_ui(!busy, |ui| {
                kit::combo("analysis_workers", 170.0, label, 14.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.prefs.analysis_threads, 0, "Automatic");
                    for n in 1..=app.pool.available_cpus.min(12) {
                        ui.selectable_value(&mut app.prefs.analysis_threads, n, n.to_string());
                    }
                });
            }).response.on_hover_text("Automatic uses this computer’s recommended worker count. Choose a fixed count between batches.");
            if app.prefs.analysis_threads != before { app.pool.set_threads(app.prefs.analysis_threads); }
        });
        if busy {
            note(ui, "The analysis queue is running. Change the worker count after this batch finishes or is stopped.");
        }
        kit::labeled(ui, "BPM Range", |ui| {
            let (lo, hi) = BPM_RANGES[app.prefs.bpm_range.min(BPM_RANGES.len() - 1)];
            kit::combo("bpmrange", 400.0, format!("{lo:.0}-{hi:.0}"), 14.0).show_ui(ui, |ui| {
                for (i, (a, b)) in BPM_RANGES.iter().enumerate() {
                    ui.selectable_value(&mut app.prefs.bpm_range, i, format!("{a:.0}-{b:.0}"));
                }
            });
        });
        separator(ui);
        sub(ui, "Track Analysis Setting");
        kit::indent(ui, 110.0);
        kit::row(ui, 16.0, |ui| {
            kit::checkbox_raw(ui, &mut app.prefs.analyze_grid, "BPM / Grid");
            kit::help(ui, 54.0, "If tracks are not analyzed, BPM/Grid analysis starts when importing, loading, or exporting the tracks whether you tick this checkbox or not.");
        });
        kit::checkbox(ui, &mut app.prefs.analyze_key, "KEY");
        kit::checkbox(ui, &mut app.prefs.phrase_analysis, "Phrase").on_hover_text("Phrases (Intro, Up, Down, Chorus, Outro) are shown under the overview and written to USB exports. rekordbox's own phrases are used for tracks it analysed.");
        kit::labeled(ui, "Phrase estimator", |ui| {
            let label = if app.prefs.classic_phrase_estimator { "recordbox classic" } else { "Rekordbox-compatible" };
            kit::combo("phrase_estimator", 220.0, label, 14.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut app.prefs.classic_phrase_estimator, false, "Rekordbox-compatible");
                ui.selectable_value(&mut app.prefs.classic_phrase_estimator, true, "recordbox classic");
            });
        });
        note(ui, "Used for missing phrases on deck load and USB export. Rekordbox-compatible can fall back to classic. Existing phrase data takes priority.");
        kit::checkbox(ui, &mut app.prefs.vocal_analysis, "Vocal").on_hover_text("Use imported vocal positions when available. For other tracks, recordbox can use an explicitly configured local vocal model or its downloaded STEMS model.");
        separator(ui);
        sub(ui, "Auto Analysis");
        toggle(ui, &mut app.prefs.auto_analysis, "Enable");
        separator(ui);
        // The cloud services are not part of Recordbox: shown switched off.
        ui.add_space(-3.5);
        kit::sub_help(ui, "Cloud Analysis", Some(no_cloud));
        ui.add_space(1.5);
        let mut refused = kit::toggle_off(ui, "Participate in rekordbox data collection\nand upload analysis data", no_cloud);
        ui.add_space(5.0);
        separator(ui);
        ui.add_space(-3.5);
        kit::sub_help(ui, "Radar Analysis", Some(no_cloud));
        ui.add_space(1.5);
        refused |= kit::toggle_off(ui, "Enable", no_cloud);
        if refused {
            app.toast(no_cloud);
        }
    });
    card(ui, "Key Analysis", |ui| {
        let why = "Track analysis saves the key in recordbox. To review it for write-back to AlphaTheta Rekordbox, open library sync.";
        if kit::toggle_off(ui, "Write the value to the ID3 tag", why) {
            app.toast(why);
        }
    });
    card(ui, "Auto Gain (recordbox)", |ui| {
        kit::block(ui, |ui| {
            ui.add(egui::Slider::new(&mut app.prefs.target_lufs, -20.0..=-5.0).suffix(" LUFS").text("Reference loudness"));
        });
        toggle(ui, &mut app.prefs.auto_gain_peak_cap, "Limit gain so the loudest peak stays below full scale");
        note(ui, "Used for the preview player's gain and stored per track when analysing.");
        kit::block(ui, |ui| {
            if ui.add(btn("Re-analyze all tracks now")).clicked() {
                let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
                app.reanalyze(&ids);
            }
        });
    });
}

// ------------------------------------------------------------------ DJ System

/// A widget at an exact place inside a card that is laid out by coordinates.
fn place(ui: &mut Ui, r: Rect, enabled: bool, w: impl egui::Widget) -> egui::Response {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_enabled(enabled, w)
}

/// The grey 20 pt square with a white arrow head under the Active list.
fn order_button(ui: &mut Ui, r: Rect, up: bool, enabled: bool, tip: &str) -> bool {
    let resp = ui.interact(r, ui.id().with(("order", up, r.min.x as i32)), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    ui.painter().rect_filled(r, 2.0, if !enabled { gy(0x3A3A3A) } else { crate::ui_feel::hl(&resp, gy(0x555555), gy(0x666666)) });
    let (a, b) = if up { (r.min.y + 12.0, r.min.y + 8.0) } else { (r.min.y + 8.0, r.min.y + 12.0) };
    let pts = vec![pos2(r.min.x + 6.0, a), pos2(r.min.x + 10.0, b), pos2(r.min.x + 14.0, a)];
    ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.0, if enabled { WHITE } else { OFF })));
    resp.on_hover_text(tip).clicked()
}

/// rekordbox's two lists: pick a name, then move it with the arrows.
fn dual_list(app: &mut App, ui: &mut Ui, is_cat: bool) {
    use crate::export::usb::{column_name, UsbBrowse, FIXED_CATEGORIES, FIXED_SORTS};
    let all = if is_cat { UsbBrowse::all_categories() } else { UsbBrowse::all_sorts() };
    let fixed: &[u16] = if is_cat { &FIXED_CATEGORIES } else { &FIXED_SORTS };
    let mut b = app.prefs.usb_browse.normalized();
    let active: Vec<u16> = if is_cat { b.categories.clone() } else { b.sorts.clone() };
    let mut next = active.clone();
    let sel_id = egui::Id::new(("dual_list_sel", is_cat));
    let mut sel: (Option<u16>, Option<u16>) = ui.ctx().data(|d| d.get_temp(sel_id)).unwrap_or((None, None));
    let mut inactive: Vec<(u16, &str)> = all.iter().copied().filter(|(id, _)| !active.contains(id)).collect();
    inactive.sort_by_key(|x| x.1.to_uppercase());
    let row = |ui: &mut Ui, name: &str, selected: bool, greyed: bool| -> bool {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), egui::Sense::click());
        if selected {
            ui.painter().rect_filled(r, 0.0, PBLUE);
        }
        ui.painter().text(pos2(r.min.x + 6.0, r.center().y), egui::Align2::LEFT_CENTER, name.to_uppercase(), inter(13.0), if greyed { OFF.c() } else { WHITE.c() });
        resp.clicked()
    };
    // The card and everything in it at the original's coordinates (628 x 484 pt).
    let w = ui.available_width().min(kit::CARD_W);
    let (card, _) = ui.allocate_exact_size(vec2(w, 484.0), egui::Sense::hover());
    ui.painter().rect_filled(card, 0.0, BG2);
    let at = |x: f32, y: f32| pos2(card.min.x + x, card.min.y + y);
    let left = Rect::from_min_max(at(20.0, 54.0), at(284.0, 464.0));
    let right = Rect::from_min_max(at(344.0, 54.0), at(608.0, 441.0));
    ui.painter().text(pos2(left.center().x, card.min.y + 29.5), egui::Align2::CENTER_CENTER, if is_cat { "Inactive Categories" } else { "Inactive Sort Options" }, inter_bold(14.0), WHITE.c());
    ui.painter().text(pos2(right.center().x, card.min.y + 30.75), egui::Align2::CENTER_CENTER, if is_cat { "Active Categories" } else { "Active Sort Options" }, inter_bold(15.75), WHITE.c());
    ui.painter().rect_filled(left, 0.0, BG);
    ui.painter().rect_filled(right, 0.0, BG);
    let list = |ui: &mut Ui, r: Rect, salt: &str, add: &mut dyn FnMut(&mut Ui)| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
        egui::ScrollArea::vertical().id_salt((salt, is_cat)).max_height(r.height()).auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.add_space(1.0);
            add(ui);
        });
    };
    list(ui, left, "dl_in", &mut |ui| {
        for (id, name) in &inactive {
            if row(ui, name, sel.0 == Some(*id), false) {
                sel = (Some(*id), None);
            }
        }
    });
    list(ui, right, "dl_ac", &mut |ui| {
        for id in &active {
            let name = column_name(*id).unwrap_or("?");
            if row(ui, name, sel.1 == Some(*id), fixed.contains(id)) {
                sel = (None, Some(*id));
            }
        }
    });
    let can_add = sel.0.is_some();
    if place(ui, Rect::from_min_size(at(304.0, 221.0), vec2(20.0, 20.0)), can_add, btn(">").exact(vec2(20.0, 20.0)).font_size(12.0)).on_hover_text("Make active").clicked() {
        if let Some(id) = sel.0.take() {
            next.push(id);
            sel.1 = Some(id);
        }
    }
    let can_remove = sel.1.map(|id| !fixed.contains(&id)).unwrap_or(false);
    if place(ui, Rect::from_min_size(at(304.0, 251.0), vec2(20.0, 20.0)), can_remove, btn("<").exact(vec2(20.0, 20.0)).font_size(12.0)).on_hover_text("Make inactive").on_disabled_hover_text("Greyed items are always shown on the players.").clicked() {
        if let Some(id) = sel.1.take() {
            next.retain(|x| *x != id);
            sel.0 = Some(id);
        }
    }
    // Order: categories move freely; fixed sort options stay where the players put them.
    let pos = sel.1.and_then(|id| active.iter().position(|x| *x == id));
    let movable = |i: usize| is_cat || !fixed.contains(&active[i]);
    let can_down = pos.map(|i| i + 1 < active.len() && movable(i) && movable(i + 1)).unwrap_or(false);
    let can_up = pos.map(|i| i > 0 && movable(i) && movable(i - 1)).unwrap_or(false);
    if order_button(ui, Rect::from_min_size(at(452.0, 446.0), vec2(20.0, 20.0)), false, can_down, "Down") {
        if let Some(i) = pos {
            next.swap(i, i + 1);
        }
    }
    if order_button(ui, Rect::from_min_size(at(478.0, 446.0), vec2(20.0, 20.0)), true, can_up, "Up") {
        if let Some(i) = pos {
            next.swap(i, i - 1);
        }
    }
    ui.add_space(4.0);
    ui.ctx().data_mut(|d| d.insert_temp(sel_id, sel));
    if next != active {
        if is_cat {
            b.categories = next;
        } else {
            b.sorts = next;
        }
        app.prefs.usb_browse = b.normalized();
    }
}

fn setting_rows(app: &mut App, ui: &mut Ui, id: &str, fields: &[crate::export::settings::Field]) {
    kit::block(ui, |ui| {
        ui.spacing_mut().interact_size.y = 19.0;
        ui.spacing_mut().button_padding.y = 0.0;
        egui::Grid::new(id).num_columns(2).spacing(vec2(24.0, 7.0)).show(ui, |ui| {
            for f in fields {
                // Waveform Divisions is on the General tab, as in rekordbox.
                if f.file == SettingFile::My2 && f.offset == 4 {
                    continue;
                }
                ui.label(kit::text(f.name.to_uppercase(), 13.0).color(WHITE));
                let cur = app.prefs.usb_settings.get(f.file, f.offset);
                let shown = f.options.iter().find(|o| o.1 == cur).map(|o| o.0).unwrap_or("—");
                let file_tag = match f.file {
                    SettingFile::My => 0u8,
                    SettingFile::My2 => 1,
                    SettingFile::Dev => 2,
                    SettingFile::Djm => 3,
                };
                kit::combo(("myset", f.offset, file_tag), 240.0, shown, 12.0).show_ui(ui, |ui| {
                    for (label, v) in f.options {
                        if ui.selectable_label(cur == *v, *label).clicked() {
                            app.prefs.usb_settings.set(f.file, f.offset, *v);
                        }
                    }
                });
                ui.end_row();
            }
        });
    });
}

pub(crate) fn page_dj_system(app: &mut App, ui: &mut Ui, tab: usize) {
    match tab {
        0 => {
            // No AlphaTheta account in Recordbox: the section is shown switched off.
            card(ui, "Display username on-screen", |ui| {
                sub(ui, "Display the AlphaTheta account nickname on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let why = "recordbox has no AlphaTheta account, so there is no nickname to show.";
                if kit::toggle_off(ui, "OFF", why) {
                    app.toast(why);
                }
            });
            card(ui, "Background Color", |ui| {
                sub(ui, "Color of device displayed on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let names = ["Default Color", "Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple"];
                let cur = (app.prefs.device_bg_color as usize).min(8);
                kit::combo_row(ui, 19.0, |ui| {
                    let r = kit::combo("bgcolor", 500.0, names[cur], 12.0).show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            let text = RichText::new(format!("■ {n}")).color(if i == 0 { DIM2.c() } else { label_color(i as u8) });
                            if ui.selectable_label(cur == i, text).clicked() {
                                app.prefs.device_bg_color = i as u8;
                            }
                        }
                    });
                    if !BG_COLOR_WRITTEN {
                        r.response.on_hover_text("Not written to the device yet: where the players keep this colour has not been confirmed. Your choice is kept for when it is.");
                    }
                });
                ui.add_space(-3.0);
            });
            card(ui, "Waveform color", |ui| {
                sub(ui, "Waveform color displayed on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_WAVEFORM_COLOR);
                for (label, v) in [("BLUE", 1u8), ("RGB", 3), ("3Band", 4)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_WAVEFORM_COLOR, v);
                    }
                }
            });
            card(ui, "Waveform Current Position", |ui| {
                sub(ui, "Waveform Current Position displayed on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_WAVEFORM_POSITION);
                for (label, v) in [("LEFT", 2u8), ("CENTER", 1)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_WAVEFORM_POSITION, v);
                    }
                }
            });
            kit::card_help(ui, "Waveform Divisions", Some("Time Scale: bar lines under the waveform.\nPhrase: the track's phrases (Intro, Up, Down, Chorus, Outro) under the waveform, on players that show them."), |ui| {
                sub(ui, "Waveform division displayed under the CDJ/XDJ waveform");
                kit::indent(ui, 88.0);
                let cur = app.prefs.usb_settings.get(SettingFile::My2, 4);
                for (label, v) in [("Time Scale", 0x80u8), ("Phrase", 0x81)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::My2, 4, v);
                    }
                }
            });
            card(ui, "Type of the Overview Waveform", |ui| {
                sub(ui, "Type of the Overview Waveform displayed on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let cur = app.prefs.usb_settings.get(SettingFile::Dev, DEV_OVERVIEW_TYPE);
                for (label, v) in [("Half Waveform", 1u8), ("Full Waveform", 2)] {
                    if radio(ui, cur == v, label) {
                        app.prefs.usb_settings.set(SettingFile::Dev, DEV_OVERVIEW_TYPE, v);
                    }
                }
            });
            crate::ui_prefs_adv::dj_general_more(app, ui);
        }
        1 => dual_list(app, ui, true),
        2 => dual_list(app, ui, false),
        3 => {
            card(ui, "Column", |ui| {
                sub(ui, "Select item which is shown next to track name on CDJ/XDJ");
                kit::indent(ui, 88.0);
                let why = "Where the players store this choice has not been confirmed, so recordbox does not write it; the players show their own default.";
                kit::combo_row(ui, 19.0, |ui| {
                    ui.add_enabled_ui(false, |ui| {
                        kit::combo("rightcol", 500.0, "Not Specified", 12.0).show_ui(ui, |_| {});
                    })
                    .response
                    .on_disabled_hover_text(why);
                });
                note(ui, "Not written yet: where the players store this choice has not been confirmed, so they show their own default.");
            });
        }
        4 => {
            card(ui, "My Settings", |ui| {
                kit::line(ui, "Player settings written to every USB you export. CDJ/XDJ players apply them when the USB is loaded.");
                sub(ui, "CDJ / XDJ");
                setting_rows(app, ui, "my_settings_grid", crate::export::settings_fields::MY_SETTINGS);
                separator(ui);
                sub(ui, "DJM");
                setting_rows(app, ui, "djm_settings_grid", crate::export::settings_fields::DJM_SETTINGS);
                kit::block(ui, |ui| {
                    if ui.add(btn("Reset to rekordbox defaults")).clicked() {
                        // The General tab's choices (DEVSETTING and Waveform Divisions) stay.
                        let keep = (app.prefs.usb_settings.dev.clone(), app.prefs.usb_settings.get(SettingFile::My2, 4));
                        app.prefs.usb_settings = crate::export::settings::UsbSettings { dev: keep.0, ..Default::default() };
                        app.prefs.usb_settings.set(SettingFile::My2, 4, keep.1);
                    }
                });
            });
        }
        5 => {
            card(ui, "Delete Tracks", |ui| {
                toggle(ui, &mut app.prefs.delete_device_tracks, "Delete playlist tracks from device\nif playlist is deleted");
            });
            card(ui, "History", |ui| {
                toggle(ui, &mut app.prefs.import_device_history, "Import the play history automatically");
                toggle_if(app, ui, HISTORY_DELETE_WIRED, |p| &mut p.delete_history_after_import, "Delete from the device after importing the play history", "recordbox does not remove histories from a device yet; imported histories are left on it.");
            });
        }
        _ => {
            card(ui, "HOT CUE AUTO LOAD", |ui| {
                toggle(ui, &mut app.prefs.hot_cue_auto_load_on_import, "Prepare Hot Cue Auto Load setting for CDJ/XDJ use (on import)");
                note(ui, "Tracks get Hot Cue Auto Load switched on as they are imported, so players set to \"rekordbox SETTING\" load their hot cues.");
            });
            crate::ui_prefs_adv::dj_link_monitor(app, ui);
            card(ui, "Waveform data written to the device", |ui| {
                kit::line(ui, "Exports always include the blue, RGB and 3-band waveforms; DJ System > General chooses which one the players show.");
            });
            card(ui, "Supported devices", |ui| {
                kit::line(ui, "Drive formats: FAT32, exFAT (some players), HFS+ (USB only).");
                kit::line(ui, "Files: MP3, AAC/M4A, WAV, AIFF, FLAC.");
                note(ui, "Classic Device Library uses export.pdb and exportExt.pdb, analysis files (DAT, EXT and 2EX), and music in /Contents.");
                note(ui, "On a mixed drive, recordbox updates the classic catalog and preserves existing shared files. Right-click the drive and choose Inspect OneLibrary playlists to view the other catalog with the optional reader.");
            });
        }
    }
}

// ------------------------------------------------------------------ Keyboard

/// Shortcuts the menus and text fields keep for themselves.
const RESERVED: [&str; 9] = ["Cmd+N", "Cmd+E", "Cmd+I", "Cmd+Z", "Shift+Cmd+Z", "Cmd+A", "Cmd+F", "Cmd+Q", "Cmd+H"];

fn page_keyboard(app: &mut App, ui: &mut Ui, _tab: usize) {
    let mode = prefs_mode(app);
    crate::ui_prefs_keys::page(app, ui, mode);
}

// ------------------------------------------------------------------ Advanced

fn page_advanced(app: &mut App, ui: &mut Ui, tab: usize) {
    let mode = prefs_mode(app);
    let name = tabs_of(rb::ADVANCED, mode).get(tab).copied().unwrap_or("Others");
    crate::ui_prefs_adv::page_advanced(app, ui, name, mode);
}

/// Controller > Deck: rekordbox's Eject/Load Lock and the Instant Doubles switch of its Load
/// card (labels and defaults as in 7.2.19).
fn page_deck(app: &mut App, ui: &mut Ui) {
    crate::ui_prefs_perf::page_deck(app, ui);
}

/// Controller > Mixer, as far as the Performance mixer goes: rekordbox's EQ / ISOLATOR choice
/// and its three channel fader curves (labels and defaults as in 7.2.19).
fn page_mixer(app: &mut App, ui: &mut Ui) {
    crate::ui_prefs_perf::page_mixer(app, ui);
}

/// A section title over the cards (rekordbox's "SAMPLER", "SEQUENCER", "Quantize").
fn section(ui: &mut Ui, title: &str) {
    // (25 pt in, like a card's title; its middle 17 pt above the card: reference preferences/10.)
    ui.add_space(20.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(kit::CARD_W), 22.0), egui::Sense::hover());
    ui.painter().text(pos2(r.min.x + 25.0, r.center().y), egui::Align2::LEFT_CENTER, title, inter_bold(16.0), WHITE.c());
    ui.add_space(4.0);
}

/// Controller > Effect: the CFX switch for the sampler deck. (rekordbox shows this row with
/// hardware that has a MIC COLOR knob; recordbox has the knob on screen, S in the CFX row.)
fn page_effect(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    card(ui, "CFX", |ui| {
        changed |= toggle(ui, &mut app.prefs.sampler.cfx_on_sampler, "Apply CFX to Sampler using MIC COLOR control knob.");
        note(ui, "The S knob of the CFX row in the FX panel then acts on the sampler deck.");
    });
    if changed {
        app.save_prefs();
    }
}

/// Controller > Sampler, after rekordbox 7.2.19 (reference preferences/10): SLOT, Capture,
/// Auto Panel Mode and Sequence Load, with rekordbox's labels and defaults.
fn page_sampler(app: &mut App, ui: &mut Ui) {
    crate::ui_prefs_perf::page_sampler(app, ui);
}

/// Controller > Others, as far as it is built: the SEQUENCER row of Quantize > Type >
/// QUANTIZE (rekordbox's SequencerQuantizeStatus and SequencerQuantize; reference
/// preferences/13). It takes effect while the sampler's Q is on.
fn page_controller_others(app: &mut App, ui: &mut Ui) {
    crate::ui_prefs_perf::page_others(app, ui);
}

/// Extensions > STEMS (the page is in ui_prefs_adv.rs).
fn page_stems(app: &mut App, ui: &mut Ui) {
    crate::ui_prefs_adv::page_stems(app, ui);
}

fn gpu_state_label(state: crate::stems::GpuState) -> &'static str {
    use crate::stems::GpuState::*;
    match state {
        CpuOnly => "CPU separation",
        Unchecked => "GPU unverified",
        Detected => "GPU detected",
        Ready => "GPU initialized",
        Unavailable => "GPU unavailable",
        Paused => "GPU paused",
    }
}

/// A disabled control shows its effective state. The saved preference survives a change of device.
fn capability_toggle(ui: &mut Ui, preference: &mut bool, label: &str, unavailable: Option<&str>) {
    if let Some(reason) = unavailable {
        ui.add_enabled_ui(false, |ui| { toggle(ui, &mut false, label); });
        note(ui, reason);
    } else {
        toggle(ui, preference, label);
    }
}

pub(crate) fn stem_model_card(app: &mut App, ui: &mut Ui) {
    let cur = app.stem.model.clone().or_else(|| app.prefs.stem_model.clone()).unwrap_or_default();
    card(ui, "Stem separation model (recordbox)", |ui| {
        let gpu = crate::stems::gpu_capability();
        kit::line(ui, &format!("STEMS build · {}", gpu.package));
        kit::line(ui, &format!("{} · {}", gpu_state_label(gpu.state), gpu.detail));
        if matches!(gpu.state, crate::stems::GpuState::Unchecked | crate::stems::GpuState::Detected) {
            note(ui, "Choosing a model saves a preference. Starting a separation checks the device.");
        }
        if let Some(hw) = &app.stem.hw {
            let ram = if hw.ram_gb > 0.0 { format!("{:.1} GB system RAM", hw.ram_gb) } else { "System RAM unknown".into() };
            kit::line(ui, &format!("{} · {} logical CPU threads · {ram}", hw.arch, hw.cores));
            let fma = if hw.fma { " · FMA" } else { "" };
            let cpu = if hw.cpu.is_empty() { String::new() } else { format!("{} · ", hw.cpu) };
            note(ui, &format!("{cpu}{}{fma}", hw.simd));
            note(ui, &format!("Automatic model · {}", app.automatic_stem_model(hw).name));
        } else if let Some(reason) = app.stem_hardware_error() {
            note(ui, reason);
            if kit::block(ui, |ui| ui.add(btn("Retry hardware check")).clicked()) {
                app.stems_init();
            }
        } else {
            note(ui, "Reading CPU and system memory information…");
        }
        for m in crate::stems::CATALOG.iter() {
            let files = match app.stem_weights_present(m.id) {
                Some(true) => "weights present",
                Some(false) => "weights missing",
                None => "checking weights",
            };
            note(ui, &format!("{}{} · {files}. {}", m.name, if cur == m.id { " (selected model)" } else { "" }, m.about));
        }
        toggle(ui, &mut app.prefs.stems_auto_download, "Download the model automatically at launch");
        if !app.stem.model_ready && app.stem.downloading.is_none() && kit::block(ui, |ui| ui.add(btn("Download now")).clicked()) {
            app.stems_download();
        }
        if let Some(status) = app.stem.download_status() {
            kit::line(ui, &status);
            if app.unrequested_stem_download() && !app.stem.download_cancel.load(std::sync::atomic::Ordering::Relaxed)
                && kit::block(ui, |ui| ui.add(btn("Cancel model download")).clicked()) {
                app.stem.download_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let preview_reason = match (app.stem.hw.as_ref(), crate::stems::spec(&cur)) {
            (Some(hw), Some(model)) => crate::stems::Live::preference_unavailable(hw, model, app.manual_stem_device()),
            _ => Some("Waiting for hardware and model information."),
        };
        capability_toggle(ui, &mut app.prefs.stems_progressive_preview, "Preview finished sections during new separations", preview_reason);
        if preview_reason.is_none() {
            note(ui, "Mute and solo work in finished sections, up to 6 min 20 s. Each job checks available memory. The original track plays until a section is ready.");
        }
        let automatic_reason = crate::stems::backend_hold().or(if crate::app::MANUAL_STEMS_PREVIEW {
            Some("The experimental Vulkan build starts separation manually. Load a track and choose Separate on the deck.")
        } else { None });
        capability_toggle(ui, &mut app.prefs.stems_auto_separate, "Separate tracks when they are loaded", automatic_reason);
        let cpu_reason = (!app.prefs.stems_auto_separate).then_some("Enable separation on track load to allow automatic CPU jobs.");
        // The load switch already displays the shared safety reason.
        if automatic_reason.is_some() {
            ui.add_enabled_ui(false, |ui| { toggle(ui, &mut false, "Allow automatic separation on CPU"); });
        } else {
            capability_toggle(ui, &mut app.prefs.stems_auto_cpu, "Allow automatic separation on CPU", cpu_reason);
        }
        note(ui, "Saved parts load automatically. New manual CPU separations can occupy several cores for minutes.");
        if let Some((id, status)) = app.stem.separating.map(|(id, _)| id).zip(app.stem.separation_status()) {
            let title = app.track(id).map(|t| t.display_title()).unwrap_or_default();
            kit::line(ui, &format!("{status} · {title}"));
            if app.separating_stems() && kit::block(ui, |ui| ui.add(btn("Stop separation")).clicked()) {
                app.stop_deck_stems();
            }
        } else if !app.prefs.show_stems {
            note(ui, "Enable the STEMS Function above to separate a track from the active deck.");
        } else if app.stem.downloading.is_none() {
            if let Some(id) = app.deck.track {
                if let Some(error) = app.stem.error_for(Some(id)) {
                    note(ui, &error.detail);
                }
                if app.loading_stems() {
                    note(ui, "Loading saved parts for the active deck…");
                } else if app.player.state.lock().stems.is_some() {
                    note(ui, "Separated parts are ready on the active deck.");
                } else if app.stem.model.is_some() {
                    let action = if app.manual_stem_device() == crate::stems::SeparationDevice::Cpu {
                        "Separate on CPU"
                    } else {
                        "Separate"
                    };
                    let label = format!("{action} · Deck {}", app.px.active + 1);
                    if kit::block(ui, |ui| ui.add(btn(label)).clicked()) {
                        app.separate_loaded();
                    }
                }
            } else {
                note(ui, "Load a track on the active deck to separate it.");
            }
        }
    });
    card(ui, "Saved stems", |ui| {
        note(ui, "Speed and quality each save four stereo parts, about 42 MB per minute per model. Export and Performance share these files.");
        toggle(ui, &mut app.prefs.keep_all_stems, "Keep all saved stems");
        kit::block(ui, |ui| {
            ui.add_enabled_ui(!app.prefs.keep_all_stems, |ui| {
                ui.horizontal(|ui| {
                    ui.label("STEMS storage budget");
                    ui.add(egui::DragValue::new(&mut app.prefs.stem_cache_gb).update_while_editing(false).suffix(" GB"));
                    let default_gb = crate::app::DEFAULT_STEM_CACHE_GB;
                    if app.prefs.stem_cache_gb != default_gb && ui.small_button(format!("Use {default_gb} GB default")).clicked() {
                        app.prefs.stem_cache_gb = default_gb;
                    }
                });
            });
        });
        if app.prefs.keep_all_stems {
            note(ui, "Saved parts stay on disk until you clear them.");
        } else if app.prefs.stem_cache_gb == 0 {
            note(ui, "With a zero budget, each successful save keeps only the latest result.");
        } else {
            note(ui, "After a successful save, the least recently used parts are removed first. The newest result can exceed this budget.");
        }
        note(ui, "Each save must leave at least 10% of the cache drive free, including while writing the temporary file. If space is low, the deck can still play the new parts until you unload the track.");
        match app.stem.cache_usage(ui.ctx()) {
            Some(Ok(bytes)) => kit::line(ui, &format!("{:.2} GB in use", *bytes as f64 / 1e9)),
            Some(Err(error)) => kit::line(ui, &format!("Could not measure the saved stems. {error}")),
            None => kit::line(ui, "Checking space in use…"),
        }
        let path = crate::db::data_dir_for_inspection().join("stems");
        kit::block(ui, |ui| {
            ui.add(egui::Label::new(format!("Cache folder · {}", path.display())).selectable(true).wrap());
            if let Some(warning) = app.stem.cache_warning() {
                ui.add(egui::Label::new(RichText::new(warning).color(CAUTION_TX)).selectable(true).wrap());
            }
            if app.dialogs.confirm_clear_stems {
                ui.label("Clear all saved speed and quality parts? Rebuilding them requires running separation again.");
                ui.horizontal(|ui| {
                    if ui.add(btn("Cancel")).clicked() {
                        app.dialogs.confirm_clear_stems = false;
                    }
                    if ui.add(btn("Clear saved parts")).clicked() {
                        app.dialogs.confirm_clear_stems = false;
                        app.stem.cache_usage = None;
                        if let Err(error) = crate::stems::clear_cache() {
                            app.toast_err(format!("Could not clear all saved stems. {error:#}"));
                        }
                    }
                });
            } else if ui.add(btn("Clear separated stems")).clicked() {
                app.dialogs.confirm_clear_stems = true;
            }
        });
    });
}

// ------------------------------------------------------------------ other windows

/// Keyboard Shortcuts, Backup Library and Restore Library windows.
pub fn extra_windows(app: &mut App, ctx: &Context) {
    crate::prefs_shell::confirm_window(app, ctx);
    shortcuts_window(app, ctx);
    backup_windows(app, ctx);
}

fn shortcuts_window(app: &mut App, ctx: &Context) {
    if !app.dialogs.ext.shortcuts_open {
        return;
    }
    let mut open = true;
    kit::dialog(ctx, "Keyboard Shortcuts").open(&mut open).default_size(vec2(520.0, 560.0)).show(ctx, |ui| {
        kit::apply_style(ui);
        egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
            for cat in ACTION_CATEGORIES {
                ui.add_space(4.0);
                ui.label(RichText::new(cat).font(bold(14.0)).color(WHITE));
                egui::Grid::new(("all_keys", cat)).num_columns(2).striped(true).spacing(vec2(24.0, 3.0)).min_col_width(250.0).show(ui, |ui| {
                    for (id, label, _, c) in ACTIONS {
                        if c != cat {
                            continue;
                        }
                        ui.label(RichText::new(label).size(12.5));
                        match app.prefs.shortcut_for(id) {
                            Some(k) => ui.label(RichText::new(key_text(&k)).size(12.5).color(WHITE)),
                            None => ui.label(RichText::new("—").size(12.5).color(OFF)),
                        };
                        ui.end_row();
                    }
                });
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Menus").font(bold(14.0)).color(WHITE));
            egui::Grid::new("menu_keys").num_columns(2).striped(true).spacing(vec2(24.0, 3.0)).min_col_width(250.0).show(ui, |ui| {
                for (n, k) in [("Create New Playlist", "⌘N"), ("Export to USB Device", "⌘E"), ("Property", "⌘I"), ("Search in the collection", "⌘F"), ("Select All", "⌘A"), ("Undo / Redo", "⌘Z / ⇧⌘Z")] {
                    ui.label(RichText::new(n).size(12.5));
                    ui.label(RichText::new(k).size(12.5).color(WHITE));
                    ui.end_row();
                }
            });
        });
        ui.separator();
        if ui.add(btn("Change shortcuts…")).clicked() {
            app.dialogs.prefs_open = true;
            app.dialogs.prefs_tab = 3;
        }
    });
    app.dialogs.ext.shortcuts_open = open;
}

fn backup_windows(app: &mut App, ctx: &Context) {
    let center = egui::Align2::CENTER_CENTER;
    if app.dialogs.ext.backup_ask {
        let mut choice: Option<Option<bool>> = None;
        kit::dialog(ctx, "Library Backup").resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(400.0);
            kit::message(ui, "Do you want to back up music files as well?");
            let n = app.tracks.len();
            let bytes: u64 = app.tracks.iter().filter(|t| !t.missing).map(|t| t.file_size as u64).sum();
            kit::message_note(ui, &format!("Database file, analysis and image files are always included. Music files: {n} track(s), about {:.1} GB.", bytes as f64 / 1e9));
            match kit::dialog_buttons(ui, &["Yes", "No", "Cancel"]) {
                Some(0) => choice = Some(Some(true)),
                Some(1) => choice = Some(Some(false)),
                Some(_) => choice = Some(None),
                None => {}
            }
        });
        if let Some(c) = choice {
            app.dialogs.ext.backup_ask = false;
            if let Some(music) = c {
                let name = format!("recordbox-backup-{}.zip", chrono::Local::now().format("%Y%m%d"));
                if let Some(f) = rfd::FileDialog::new().set_title("Backup file").set_file_name(name).add_filter("recordbox library backup", &["zip"]).save_file() {
                    app.save_prefs();
                    let _ = app.db.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
                    app.dialogs.ext.backup_job = Some(crate::backup::start_backup(ctx.clone(), f, music));
                }
            }
        }
    }
    if let Some(path) = app.dialogs.ext.restore_ask.clone() {
        let mut choice = None;
        kit::dialog(ctx, "Library Restoration").resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(440.0);
            kit::message(ui, "Are you sure you want to overwrite the present library with a backup?");
            kit::message_note(ui, &format!("Backup file: {}", path.display()));
            if let Ok(m) = crate::backup::read_manifest(&path) {
                kit::message_note(ui, &format!("Made {} · {} track(s) · {} artwork file(s) · {} music file(s)", m.created.get(..16).unwrap_or(&m.created).replace('T', " "), m.tracks, m.artwork_files, m.music.len()));
            }
            kit::message_note(ui, "The present library is copied into the backups folder first. Music files are only put back where the file is missing.");
            match kit::dialog_buttons(ui, &["Yes", "No"]) {
                Some(0) => choice = Some(true),
                Some(_) => choice = Some(false),
                None => {}
            }
        });
        if let Some(yes) = choice {
            app.dialogs.ext.restore_ask = None;
            if yes {
                app.save_prefs();
                app.dialogs.ext.backup_job = Some(crate::backup::start_restore(ctx.clone(), path));
            }
        }
    }
    if let Some(job) = &app.dialogs.ext.backup_job {
        let (done, total, what) = job.progress.lock().clone();
        let restore = job.restore;
        let mut cancel = false;
        kit::dialog(ctx, if restore { "Library Restoration" } else { "Library Backup" }).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(380.0);
            ui.label(RichText::new(if total == 0 { if restore { "Preparing restore..." } else { "Preparing backup..." } } else if restore { "Restore Collection" } else { "Back up Collection" }).size(14.0).color(WHITE));
            let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).show_percentage());
            ui.label(RichText::new(what).size(11.0).color(DIM));
            let cancelling = job.cancel.load(std::sync::atomic::Ordering::Relaxed);
            if ui.add_enabled(!cancelling, btn(if cancelling { "Cancelling…" } else { "Cancel" })).clicked() {
                cancel = true;
            }
        });
        if cancel {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    if let Some((title, text)) = app.dialogs.ext.backup_done.clone() {
        let mut close = false;
        kit::dialog(ctx, title).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_max_width(460.0);
            for (i, line) in text.lines().enumerate() {
                ui.label(RichText::new(line).size(if i == 0 { 14.0 } else { 12.0 }).color(if i == 0 { WHITE } else { DIM2 }));
            }
            if kit::dialog_buttons(ui, &["OK"]).is_some() {
                close = true;
            }
        });
        if close {
            app.dialogs.ext.backup_done = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_settings_and_shortcuts() {
        let ex = Mode::Export;
        assert!(search_settings("", ex).is_empty());
        let hits = search_settings("buffer", ex);
        assert_eq!(hits, vec![(5, 0, "Buffer size".to_string())]);
        assert!(search_settings("WAVEFORM", ex).len() >= 5, "search ignores case");
        assert!(search_settings("zzzz", ex).is_empty());
        assert_eq!(search_settings("grid estimator", ex), vec![(1, 0, "Beat grid estimator".to_string())]);
        assert_eq!(search_settings("MP3 cue", ex), vec![(1, 0, "MP3 cue timing".to_string())]);
        // A page that exists only in Performance mode is found only there.
        assert!(search_settings("needle lock", ex).is_empty());
        assert_eq!(search_settings("needle lock", Mode::Performance), vec![(7, 0, "Needle Lock".to_string())]);
        assert!(search_settings("Play/Pause", Mode::Performance).iter().any(|h| h.0 == 3));
        // Every indexed setting points at a tab that exists in some mode.
        for &(page, tab, name) in SETTINGS {
            assert!([Mode::Export, Mode::Performance].iter().any(|m| rb::categories(*m).contains(&page) && tabs_of(page, *m).contains(&tab)), "{name}: {page} > {tab}");
        }
    }

    #[test]
    fn reset_keeps_shortcuts_columns_and_local_folders() {
        let mut p = Prefs::default();
        p.keymap.insert("play".into(), "P".into());
        p.columns = vec![Col::Title, Col::Bpm];
        p.widths.insert("title".into(), 300.0);
        p.devices_extra = vec!["/tmp/dev".into()];
        p.relocate_folders = vec!["/Volumes/Music".into()];
        p.audio_device = Some("Speakers".into());
        p.tooltips = false;
        p.wave_half = true;
        p.audio_buffer = 256;
        p.color_names[0] = "Warm up".into();
        p.usb_settings.set(SettingFile::My, 10, 0x80);
        let r = reset_prefs(&p);
        assert_eq!(r.keymap.get("play").map(|s| s.as_str()), Some("P"));
        assert_eq!(r.columns, vec![Col::Title, Col::Bpm]);
        assert_eq!(r.widths.get("title"), Some(&300.0));
        assert_eq!(r.devices_extra, p.devices_extra);
        assert_eq!(r.relocate_folders, p.relocate_folders);
        assert_eq!(r.audio_device, p.audio_device);
        assert!(r.tooltips && !r.wave_half);
        assert_eq!(r.audio_buffer, 0);
        assert_eq!(r.color_names, crate::prefs_ext::default_color_names());
        assert_eq!(r.usb_settings, Default::default());
    }
}
