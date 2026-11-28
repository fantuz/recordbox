//! Preferences > View (Display Type, Layout, Color) and Audio > Input/Output, laid out by
//! rekordbox 7.2.19's own item bounds (tree items 13-37 and 44-54) for each mode.

use crate::app::*;
use crate::prefs_rb::Mode;
use crate::theme::*;
use crate::ui_kit::Just;
use crate::ui_prefs_item::{item, Item};
use crate::ui_prefs_perf::STORED;
use eframe::egui;
use egui::Ui;

fn stored(it: &mut Item, what: &str) {
    it.hint(format!("{what} {STORED}"));
}

pub fn page_view(app: &mut App, ui: &mut Ui, tab: usize, mode: Mode) {
    let before = app.prefs.rb.clone();
    match tab {
        0 => display_type(app, ui, mode),
        1 => layout(app, ui, mode),
        _ => color(app, ui, mode),
    }
    if app.prefs.rb != before {
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Display Type

fn display_type(app: &mut App, ui: &mut Ui, mode: Mode) {
    let perf = mode == Mode::Performance;
    item(ui, "ViewLanguage", 98.0, |it| {
        it.title("Language");
        it.hint("recordbox is in English.");
        it.combo(52.0, 56.0, 520.0, "English", |ui| {
            let _ = ui.selectable_label(true, "English");
        });
    });
    item(ui, "ViewPopupHint", 92.0, |it| {
        it.title("Tooltips");
        it.pill(52.0, 56.0, &mut app.prefs.tooltips, "Show Tooltips");
    });
    // Browse: the full card in Export and Performance, without "Coloring of played tracks"
    // in Lighting and Edit.
    let played = matches!(mode, Mode::Export | Mode::Performance);
    let mut refused = false;
    let mut dirty = false;
    item(ui, if played { "ViewBrowse1" } else { "ViewBrowse2" }, if played { 464.0 } else { 344.0 }, |it| {
        it.title("Browse");
        let p = &mut app.prefs;
        it.child(20.0, 56.0, |it| {
            it.sub(32.0, 0.0, "Font Size ");
            let mut f = p.browse_font as u32;
            if it.steps(64.0, 34.0, 524.0, &mut f, 4) {
                p.browse_font = f as u8;
                dirty = true;
            }
            it.text(64.0, 64.0, 250.0, 16.0, "Small", 14.0, Just::TopLeft);
            it.text(338.0, 64.0, 250.0, 16.0, "Large", 14.0, Just::TopRight);
            dirty |= it.pill(64.0, 94.0, &mut p.browse_bold, "Bold");
        });
        it.sep(182.0);
        it.child(20.0, 198.0, |it| {
            it.sub(32.0, 0.0, "Line Space");
            let mut l = p.browse_line as u32;
            if it.steps(64.0, 34.0, 524.0, &mut l, 4) {
                p.browse_line = l as u8;
                dirty = true;
            }
            it.text(64.0, 64.0, 250.0, 16.0, "Small", 14.0, Just::TopLeft);
            it.text(338.0, 64.0, 250.0, 16.0, "Large", 14.0, Just::TopRight);
        });
        it.sep(292.0);
        dirty |= it.pill(52.0, 308.0, &mut p.center_selected, "Show the selected track to the center");
        if played {
            it.sep(340.0);
            it.child(20.0, 356.0, |it| {
                it.sub(32.0, 0.0, "Coloring of played tracks ");
                // (LINK EXPORT and the play history colouring are not part of recordbox.)
                let why = "recordbox has no LINK EXPORT, so no tracks are coloured as played.";
                it.hint(why);
                let mut off = false;
                refused |= it.pill(64.0, 34.0, &mut off, "Reset color when exiting rekordbox");
                refused |= it.button(64.0, 66.0, 264.0, 22.0, "RESET").clicked();
                it.no_hint();
            });
        }
    });
    if refused {
        app.toast("recordbox has no LINK EXPORT, so no tracks are coloured as played.");
    }
    if perf {
        item(ui, "ViewDeckInfo", 106.0, |it| {
            it.title("Deck track information display");
            it.hint("The PERFORMANCE decks show no title, artist or artwork.");
            it.pill(52.0, 56.0, &mut app.prefs.rb.hide_deck_info, "Hide Title, Artist, and Artwork");
        });
    }
    item(ui, "ViewKeyOption", 144.0, |it| {
        it.title("Key display format");
        let p = &mut app.prefs;
        if it.radio(52.0, 56.0, !p.key_alnum, "Classic") {
            p.key_alnum = false;
            dirty = true;
        }
        if it.radio(52.0, 82.0, p.key_alnum, "Alphanumeric") {
            p.key_alnum = true;
            dirty = true;
        }
        it.hint("On: a key the library spelled its own way (\"Fm\", \"4A\") is shown so; off: every key in the format above.");
        let was = p.rb.show_original_key;
        it.pill(52.0, 108.0, &mut p.rb.show_original_key, "Display key information on the database");
        if was != p.rb.show_original_key {
            dirty = true;
        }
    });
    item(ui, "ViewWaveform", 286.0, |it| {
        it.title("Waveform");
        let p = &mut app.prefs;
        it.child(20.0, 56.0, |it| {
            it.sub(32.0, 0.0, "Playback animation");
            it.hint("Maximum frame rate during playback.");
            let rates = [(15, "15 frames/s"), (30, "30 frames/s"), (40, "40 frames/s"), (0, "Display rate")];
            let current = if p.playback_fps == 0 && p.rb.draw_rate != 0 { 40 } else { p.playback_fps };
            let label = rates.iter().find(|(fps, _)| *fps == current).map_or("Display rate", |(_, label)| *label);
            it.combo(64.0, 34.0, 180.0, label, |ui| {
                for (fps, label) in rates {
                    if ui.selectable_label(current == fps, label).clicked() {
                        p.playback_fps = fps;
                        p.rb.draw_rate = 0;
                        dirty = true;
                    }
                }
            });
            it.no_hint();
        });
        it.sep(174.0);
        it.child(20.0, 190.0, |it| {
            it.sub(32.0, 0.0, "Full/Preview Waveform");
            if it.radio(64.0, 34.0, p.wave_half, "Half Waveform") {
                p.wave_half = true;
                dirty = true;
            }
            if it.radio(66.0, 60.0, !p.wave_half, "Full Waveform") {
                p.wave_half = false;
                dirty = true;
            }
        });
    });
    item(ui, "ViewBeatCount", 144.0, |it| {
        it.title("Beat Count Display");
        let mut sel = app.prefs.beat_count_mode.min(2) as usize;
        if it.radios(52.0, 56.0, &mut sel, &["Current Position (Bars)", "Count to the next MEMORY CUE (Bars)", "Count to the next MEMORY CUE (Beats)"]) {
            app.prefs.beat_count_mode = sel as u8;
            dirty = true;
        }
    });
    if matches!(mode, Mode::Export | Mode::Lighting) {
        item(ui, "ViewWaveClick", 92.0, |it| {
            it.title("Click on the waveform for PLAY and CUE");
            dirty |= it.pill2(52.0, 56.0, &mut app.prefs.wave_click_play_cue, "Enable", "Disable");
        });
    }
    if perf {
        item(ui, "ViewJogDisplay", 118.0, |it| {
            it.title("Switch JOG Display");
            it.hint("What the platter's ring shows: the current cue and SLIP, or the countdown to the next hot cue.");
            let p = &mut app.prefs.rb;
            if it.radio(52.0, 56.0, p.jog_indicator == 1, "Current CUE / SLIP") {
                p.jog_indicator = 1;
            }
            if it.radio(52.0, 82.0, p.jog_indicator == 2, "HOT CUE COUNTDOWN") {
                p.jog_indicator = 2;
            }
        });
    }
    item(ui, "ViewTrafficLight", 98.0, |it| {
        it.title_tip("Traffic Light", "Tracks whose key suits the track on the master deck are marked in the track list. Choose how close the key has to be.");
        it.hint("Which keys count as suiting the target deck's key.");
        let mut sel = (app.prefs.rb.traffic_light.clamp(1, 4) - 1) as usize;
        if it.choice(52.0, 56.0, 520.0, &mut sel, &["Same Key", "Related Key 1", "Related Key 2", "Related Key 3"]) {
            app.prefs.rb.traffic_light = sel as u8 + 1;
        }
    });
    if dirty {
        app.list_dirty = true;
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Layout

fn layout(app: &mut App, ui: &mut Ui, mode: Mode) {
    // (Performance: Deck, Phrases, Vocal; Export: Phrases, Vocal; Lighting: Vocal; Edit: none.)
    // Measured on the Performance capture (preferences/05): the Media Browser has seven rows
    // 26 pt apart from y 92, Browser panel starts at 296, Deck at 511, Phrases at 596 (its
    // rows at +30 and +56, the switch at +85) and the next section 136 pt after it. The other
    // modes leave sections out and close up.
    let (h, deck_y, phr_y, voc_y) = match mode {
        Mode::Performance => (828.0, Some(511.0), Some(596.0), Some(732.0)),
        Mode::Export => (743.0, None, Some(511.0), Some(647.0)),
        Mode::Lighting => (607.0, None, None, Some(511.0)),
        Mode::Edit => (486.0, None, None, None),
    };
    let name = ["ViewLayout2", "ViewLayout1", "ViewLayout3", "ViewLayout4"][mode.index()];
    let mut dirty = false;
    item(ui, name, h, |it| {
        it.title("Layout");
        let p = &mut app.prefs;
        it.child(20.0, 56.0, |it| {
            it.sub(32.0, 0.0, "Media Browser");
            let [left, right] = crate::prefs_ext::MEDIA_BROWSER;
            for i in 0..left.len() {
                for (col, name) in [left[i], right[i]].into_iter().enumerate() {
                    let (x, y) = (64.0 + 287.0 * col as f32, 36.0 + 26.0 * i as f32);
                    if crate::prefs_ext::tree_item_available(name) {
                        let mut on = p.tree_item(name);
                        if it.check(x, y, &mut on, name) {
                            p.tree_items.insert(name.to_string(), on);
                            dirty = true;
                        }
                    } else {
                        it.en = false;
                        it.hint(crate::prefs_ext::media_absent_reason(name));
                        let mut off = false;
                        it.check(x, y, &mut off, name);
                        it.en = true;
                        it.no_hint();
                    }
                }
            }
        });
        it.sep(280.0);
        it.child(20.0, 296.0, |it| {
            it.sub(32.0, 0.0, "Browser panel");
            let mut cue_pl = p.tree_item("CUE Analysis Playlist");
            if it.check(64.0, 34.0, &mut cue_pl, "CUE Analysis Playlist") {
                p.tree_items.insert("CUE Analysis Playlist".to_string(), cue_pl);
                dirty = true;
            }
            dirty |= it.check(64.0, 60.0, &mut p.palette_enabled, "Playlist Palette");
            dirty |= it.check(64.0, 86.0, &mut p.cue_markers_on_preview, "Display Cue Markers on Preview");
            dirty |= it.check(64.0, 112.0, &mut p.playlist_search_filter, "Playlist search filter");
            dirty |= it.check(64.0, 138.0, &mut p.all_tracks_in_playlists, "Display All Tracks in the Playlists");
            dirty |= it.check(64.0, 164.0, &mut p.playlist_track_counts, "Display the number of tracks in a playlist on the Tree View");
        });
        if let Some(y) = deck_y {
            it.sep(y - 16.0);
            it.child(20.0, y, |it| {
                it.sub(32.0, 0.0, "Deck");
                it.hint("The x1/2, x1, x2 button next to BEAT SYNC.");
                it.check(64.0, 34.0, &mut p.rb.show_sync_rate, "Display SYNC RATE");
                it.no_hint();
            });
        }
        if let Some(y) = phr_y {
            it.sep(y - 16.0);
            it.sub(52.0, y, "Phrases");
            dirty |= it.check(84.0, y + 30.0, &mut p.phrase_enlarged, "Phrase (Enlarged Waveform)");
            dirty |= it.check(84.0, y + 56.0, &mut p.show_phrase_bar, "Phrase (Full Waveform)");
            dirty |= it.pill(84.0, y + 85.0, &mut p.phrase_types_always, "Always show types of phrases");
        }
        if let Some(y) = voc_y {
            it.sep(y - 16.0);
            it.child(20.0, y, |it| {
                it.sub_tip(32.0, 0.0, "Vocal", "Vocal positions found by the analysis are shown on the waveforms.");
                dirty |= it.check(84.0, 34.0, &mut p.vocal_enlarged, "Vocal (Enlarged Waveform)");
                dirty |= it.check(84.0, 60.0, &mut p.show_vocal, "Vocal (Full Waveform)");
            });
        }
    });
    if mode == Mode::Performance {
        let p = &mut app.prefs.rb;
        item(ui, "ViewEffectPanel", 144.0, |it| {
            it.title("Effect Panel");
            it.hint("Which rows the FX panel has (`prefs.rb.fx_rows()`).");
            let mut sel = (p.fx_panel_layout.clamp(1, 3) - 1) as usize;
            if it.radios(52.0, 56.0, &mut sel, &["BEAT FX", "SOUND COLOR FX", "BEAT FX + SOUND COLOR FX"]) {
                p.fx_panel_layout = sel as u8 + 1;
            }
        });
        item(ui, "ViewZoomWaveform", 180.0, |it| {
            it.title("Enlarged Waveform");
            it.hint("The order of the decks' enlarged waveforms in the 4-deck layouts (Auto: 1-2-3-4 horizontal, 3-1-2-4 vertical).");
            if it.radio(52.0, 56.0, !p.zoom_wave_custom, "Auto") {
                p.zoom_wave_custom = false;
            }
            if it.radio(52.0, 82.0, p.zoom_wave_custom, "Customize") {
                p.zoom_wave_custom = true;
            }
            it.en = p.zoom_wave_custom;
            it.text(52.0, 114.0, 270.0, 16.0, "Horizontal Waves", 14.0, Just::TopLeft);
            it.text(326.0, 114.0, 270.0, 16.0, "Vertical Waves", 14.0, Just::TopLeft);
            let names = ["1-2-3-4", "3-1-2-4"];
            let mut a = (p.zoom_wave_h.clamp(1, 2) - 1) as usize;
            if it.choice(52.0, 138.0, 260.0, &mut a, &names) {
                p.zoom_wave_h = a as u8 + 1;
            }
            let mut b = (p.zoom_wave_v.clamp(1, 2) - 1) as usize;
            if it.choice(326.0, 138.0, 260.0, &mut b, &names) {
                p.zoom_wave_v = b as u8 + 1;
            }
        });
        item(ui, "ViewPadMode", 150.0, |it| {
            it.title("PAD Mode");
            it.hint("How the pads are shown under the decks.");
            if it.radio(52.0, 56.0, !p.pad_mode_custom, "Auto") {
                p.pad_mode_custom = false;
            }
            if it.radio(52.0, 82.0, p.pad_mode_custom, "Customize") {
                p.pad_mode_custom = true;
            }
            it.en = p.pad_mode_custom;
            let mut a = (p.pad_mode_layout.clamp(1, 2) - 1) as usize;
            if it.choice(52.0, 108.0, 500.0, &mut a, &["LIST Display", "PAD Display"]) {
                p.pad_mode_layout = a as u8 + 1;
            }
        });
    }
    if dirty {
        app.list_dirty = true;
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Color

/// rekordbox's eight pad colours (the names of its colour dropdowns).
const PAD_COLORS: [&str; 9] = ["Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Teal"];

/// The dot colours of the PAD FX dropdowns (sampled from preferences/06 where shown).
fn pad_color(i: u8) -> egui::Color32 {
    [hex(0xFF5AA5), hex(0xF03C3C), hex(0xFF8C19), hex(0xF8E330), hex(0x24E01F), hex(0x00DFFE), hex(0x56B2FB), hex(0xB133FF), hex(0x28A092)][(i.clamp(1, 9) - 1) as usize]
}

fn color(app: &mut App, ui: &mut Ui, mode: Mode) {
    let mut dirty = false;
    if matches!(mode, Mode::Export | Mode::Performance) {
        item(ui, "ViewSkin", 118.0, |it| {
            it.title("Appearance");
            // `ViewSkin::buttonClicked`: the radios set `ColorThemeMode` (1 Dark, 2 Light) and
            // the whole window is redrawn at once (theme::sync).
            let light = app.prefs.rb.color_theme == 2;
            if it.radio(52.0, 56.0, !light, "Dark ") && light {
                app.prefs.rb.color_theme = 1;
                dirty = true;
            }
            if it.radio(52.0, 82.0, light, "Light ") && !light {
                app.prefs.rb.color_theme = 2;
                dirty = true;
            }
        });
    }
    item(ui, "ViewWaveColor", 144.0, |it| {
        it.title("Waveform color");
        let mut sel = app.prefs.wave_color.min(2) as usize;
        if it.radios(52.0, 56.0, &mut sel, &["BLUE", "RGB", "3Band"]) {
            app.prefs.wave_color = sel as u8;
            dirty = true;
        }
    });
    item(ui, "ViewHotCueColor", 98.0, |it| {
        it.title("HOT CUE color");
        it.hint(format!("recordbox gives new hot cues the COLORFUL colours. {STORED}"));
        let mut sel = (app.prefs.rb.hot_cue_color.clamp(1, 4) - 1) as usize;
        if it.choice(52.0, 56.0, 520.0, &mut sel, &["CDJ", "COLD1", "COLD2", "COLORFUL"]) {
            app.prefs.rb.hot_cue_color = sel as u8 + 1;
        }
    });
    if mode == Mode::Performance {
        let p = &mut app.prefs.rb;
        // PAD FX color: per effect a dropdown showing its pad colour as a dot (a click goes to
        // the next colour). The first sixteen effects and their colours are those of the
        // capture (preferences/06); the rest of the page is below what was captured.
        const BEAT: [(&str, u8); 16] = [("DELAY", 7), ("ECHO", 7), ("SPIRAL", 5), ("REVERB", 5), ("TRANS", 6), ("FILTER LFO", 9), ("FLANGER", 9), ("PHASER", 9), ("ROBOT", 8), ("SLIP ROLL", 6), ("ROLL", 6), ("REV ROLL", 6), ("MT DELAY", 7), ("UP ECHO", 7), ("DOWN ECHO", 7), ("REV DELAY", 7)];
        const RELEASE: [(&str, u8); 3] = [("VINYL BRAKE", 2), ("ECHO", 2), ("BACK SPIN", 2)];
        const SCENE: [(&str, u8); 10] = [("MOD", 3), ("ECHO", 3), ("NOISE", 3), ("SPIRAL UP", 3), ("REVERB UP", 3), ("CRUSH ECHO", 4), ("SPIRAL DOWN", 4), ("REVERB DOWN", 4), ("HPF ECHO", 4), ("LPF ECHO", 4)];
        let rows = |n: usize| ((n + 1) / 2) as f32 * 32.0;
        let (y2, y3) = (92.0 + rows(BEAT.len()) + 16.0, 92.0 + rows(BEAT.len()) + 16.0 + 36.0 + rows(RELEASE.len()) + 16.0);
        item(ui, "ViewPadFxColor", y3 + 36.0 + rows(SCENE.len()) + 8.0, |it| {
            it.title("PAD FX color");
            stored(it, "The pad colour of each effect in PAD FX mode.");
            let groups: [(&str, f32, &str, &[(&str, u8)]); 3] = [("BEAT FX/SOUND COLOR FX", 56.0, "", &BEAT), ("RELEASE FX", y2, "(RFX)", &RELEASE), ("SCENE FX(RMX EFFECTS)", y3, "(SFX)", &SCENE)];
            for (title, sy, suffix, names) in groups {
                it.sub(52.0, sy, title);
                for (i, (n, def)) in names.iter().enumerate() {
                    let (x, y) = (88.0 + 258.0 * (i % 2) as f32, sy + 36.0 + 32.0 * (i / 2) as f32);
                    let key = format!("{n}{suffix}");
                    let cur = p.pad_fx_colors.get(&key).copied().unwrap_or(*def).clamp(1, 9);
                    it.text(x, y, 110.0, 22.0, n, 16.0, Just::Left);
                    let o = it.painter().1;
                    let r = egui::Rect::from_min_size(egui::pos2(o.x + x + 110.0, o.y + y), egui::vec2(80.0, 22.0));
                    let pa = it.ui.painter();
                    pa.rect_filled(r, 0.0, crate::ui_kit::GREY);
                    pa.circle_filled(egui::pos2(r.min.x + 20.0, r.center().y), 5.0, pad_color(cur));
                    let (w, m) = (r.max.x - 10.0, r.center().y.floor());
                    pa.add(egui::Shape::line(vec![egui::pos2(w - 17.0, m - 2.0), egui::pos2(w - 10.0, m + 3.0), egui::pos2(w - 3.0, m - 2.0)], egui::Stroke::new(2.0, BG2)));
                    if it.sense(x + 110.0, y, 80.0, 22.0, "color", n).on_hover_text(PAD_COLORS[cur as usize - 1]).clicked() {
                        p.pad_fx_colors.insert(key, cur % 9 + 1);
                    }
                }
            }
        });
        item(ui, "ViewSamplerColor", 118.0, |it| {
            it.title("SAMPLER color");
            it.hint("Sync with Frequency: each pad in the colour of its sample's sound. Fixed: the colour chosen for the sample (`SamplerColorType` 0 / 1, `onLoadedClipChanged` 0x100b19f94).");
            if it.radio(52.0, 56.0, p.sampler_color == 0, "Sync with Frequency") {
                p.sampler_color = 0;
            }
            if it.radio(52.0, 82.0, p.sampler_color != 0, "Fixed") {
                p.sampler_color = 1;
            }
        });
    }
    if dirty {
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Audio > Input/Output

/// One "L / R" pair of dropdowns (rekordbox's AudioOutputChannelComponent, 588 x 88).
/// Returns the pair that was chosen: Some(None) for "-".
fn pair(it: &mut Item, y: f32, title: &str, cur: Option<u16>, chans: u16, taken: &[u16], dev: &str, dash: bool, w: f32) -> Option<Option<u16>> {
    let mut pick = None;
    it.child(20.0, y, |it| {
        it.sub(32.0, 0.0, title);
        let name = |ch: u16| if dev.is_empty() { format!("{}", ch + 1) } else { format!("{dev} : {}", ch + 1) };
        for (side, label) in ["L", "R"].iter().enumerate() {
            let yy = 34.0 + 32.0 * side as f32;
            it.text(64.0, yy, 40.0, 22.0, label, if side == 0 { 16.0 } else { 14.0 }, Just::Left);
            let shown = cur.map(|c| name(c + side as u16)).unwrap_or_else(|| "-".to_string());
            it.combo(110.0, yy, w, &shown, |ui| {
                if dash && ui.selectable_label(cur.is_none(), "-").clicked() {
                    pick = Some(None);
                }
                let mut first = 0;
                while first + 1 < chans {
                    if !taken.contains(&first) && ui.selectable_label(cur == Some(first), name(first + side as u16)).clicked() {
                        pick = Some(Some(first));
                    }
                    first += 2;
                }
                if chans <= 2 && dash {
                    ui.label(crate::ui_kit::text("This device has one stereo output: choose a device with four or more outputs (an audio interface, a controller, or an Aggregate Device made in Audio MIDI Setup).", 11.0).color(DIM));
                }
            });
        }
    });
    pick
}

/// Audio > Input/Output. `chans` and `dev` describe the output device in use.
/// Returns true when the output stream has to be reopened.
pub fn audio_io(app: &mut App, ui: &mut Ui, mode: Mode, chans: u16, dev: &str) -> bool {
    let perf = mode == Mode::Performance;
    let before = app.prefs.rb.clone();
    let mut reopen = false;
    if perf {
        item(ui, "AudioMixerMode", 118.0, |it| {
            it.title("Mixer Mode");
            let p = &mut app.prefs.rb;
            it.radio(52.0, 56.0, !p.mixer_external, "Internal");
            // (External hands every deck to its own output of a DJ mixer; recordbox mixes
            // the decks itself.)
            it.en = false;
            it.hint("recordbox mixes the decks itself; it has no external mixer mode.");
            it.radio(52.0, 82.0, p.mixer_external, "External");
        });
    }
    // Input channels: Setting Utility and Input Record, and the four deck inputs in
    // Performance mode. recordbox records its own master output and has no audio inputs.
    let why = "recordbox has no audio inputs: it records its own master output.";
    item(ui, if perf { "AudioInputChannel2" } else { "AudioInputChannel3" }, if perf { 666.0 } else { 202.0 }, |it| {
        it.title("Input channels");
        it.hint("rekordbox opens the connected device's setting utility here; recordbox has none to open.");
        it.en = false;
        it.button(61.0, 56.0, 524.0, 22.0, "Setting Utility");
        it.en = true;
        it.sep(94.0);
        it.child(20.0, 110.0, |it| {
            it.sub(32.0, 0.0, "Input Record");
            it.en = false;
            it.hint(why);
            it.combo(64.0, 34.0, 524.0, "MASTER OUT", |_| {});
            it.en = true;
        });
        if perf {
            for d in 0..4 {
                let y = 198.0 + 120.0 * d as f32;
                it.sep(y - 16.0);
                it.child(20.0, y, |it| {
                    it.sub(32.0, 0.0, &format!("Input Deck{}", d + 1));
                    it.hint(why);
                    for (side, label) in ["L", "R"].iter().enumerate() {
                        let yy = 34.0 + 32.0 * side as f32;
                        it.text(64.0, yy, 40.0, 22.0, label, if side == 0 { 16.0 } else { 14.0 }, Just::Left);
                        it.combo(110.0, yy, 340.0, "-", |ui| {
                            ui.label(crate::ui_kit::text(why, 11.0).color(DIM));
                        });
                    }
                    it.no_hint();
                });
            }
        }
    });
    let cur = if app.prefs.audio_channel + 1 < chans { app.prefs.audio_channel } else { 0 };
    let hp = app.prefs.hp_channel.filter(|h| h + 1 < chans && *h != cur);
    let booth = app.prefs.rb.booth_channel.filter(|h| h + 1 < chans && *h != cur);
    let (mut new_ch, mut new_hp, mut new_booth) = (None, None, None);
    item(ui, if perf { "AudioOutputChannel3" } else { "AudioOutputChannel5" }, if perf { 404.0 } else { 164.0 }, |it| {
        it.title("Output channels");
        new_ch = pair(it, 56.0, "Master Output", Some(cur), chans.max(2), &[], dev, false, 478.0);
        if perf {
            it.sep(160.0);
            it.hint("The CUE buttons' sound: another pair of outputs of the same device.");
            new_hp = pair(it, 176.0, "Headphones Output", hp, chans, &[cur], dev, true, 478.0);
            it.sep(280.0);
            it.hint(format!("A second copy of the master for the booth. {STORED}"));
            new_booth = pair(it, 296.0, "Booth Output", booth, chans, &[cur], dev, true, 478.0);
        }
    });
    if let Some(Some(c)) = new_ch {
        if c != app.prefs.audio_channel {
            app.prefs.audio_channel = c;
            if app.prefs.hp_channel == Some(c) {
                app.prefs.hp_channel = None;
            }
            reopen = true;
        }
    }
    if let Some(h) = new_hp {
        if h != app.prefs.hp_channel {
            app.prefs.hp_channel = h;
            reopen = true;
        }
    }
    if let Some(b) = new_booth {
        app.prefs.rb.booth_channel = b;
    }
    if app.prefs.rb != before {
        app.save_prefs();
    }
    reopen
}
