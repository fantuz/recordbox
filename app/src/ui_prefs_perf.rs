//! Preferences > Controller (Deck, Mixer, Sampler, Others) and Preferences > DVS, laid out by
//! rekordbox 7.2.19's own item bounds (`checks/re-7.2.19/preferences-skins-tree.md`, items
//! 67-77, 90-94, 122-127, 136-143, 161-165), with its captions and defaults. The parts that
//! need a connected controller (the device heading and the items after it) are left out, as
//! rekordbox leaves them out without one.

use crate::app::*;
use crate::prefs_rb::{QRow, Q_BEATS};
use crate::ui_kit::Just;
use crate::ui_prefs_item::{item, item_bare, Item};
use eframe::egui;
use egui::Ui;

/// What recordbox does with a setting it only stores.
pub const STORED: &str = "recordbox keeps this setting; nothing acts on it yet.";

fn stored(it: &mut Item, what: &str) {
    it.hint(format!("{what} {STORED}"));
}

/// A section heading on the window's black (rekordbox's 42 pt text items).
pub fn heading(ui: &mut Ui, name: &'static str, s: &str) {
    item_bare(ui, name, 42.0, |it| it.title(s));
}

/// "Lock" / "Unlock" (two radio buttons); `lock` is the stored state.
fn lock_item(ui: &mut Ui, name: &'static str, title: &str, lock: &mut bool, hint: &str) -> bool {
    item(ui, name, 118.0, |it| {
        it.title(title);
        it.hint(hint);
        if it.radio(52.0, 56.0, *lock, "Lock") {
            *lock = true;
        }
        if it.radio(52.0, 82.0, !*lock, "Unlock") {
            *lock = false;
        }
        it.changed
    })
}

// ------------------------------------------------------------------ Deck

pub fn page_deck(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    let p = &mut app.prefs;
    changed |= lock_item(ui, "ControllerEjectLoadLock", "Eject/Load Lock", &mut p.deck_load_lock, "Lock: a deck that is playing refuses a new track.");
    changed |= lock_item(ui, "ControllerNeedleLock", "Needle Lock", &mut p.rb.needle_lock, "Lock: the full waveform does not move a deck that is playing.");
    changed |= lock_item(ui, "ControllerMemoryCueCallLock", "Memory Cue Call Lock", &mut p.rb.memory_cue_call_lock, "Lock: memory cues cannot be called while the deck plays (\"Memory Cue Call Lock. Not available during playback.\").");
    changed |= item(ui, "ControllerLoad", 118.0, |it| {
        it.title("Load");
        it.hint("On: a track loaded in PERFORMANCE stands at its first memory cue or hot cue, whichever comes first.");
        it.check(52.0, 56.0, &mut p.rb.load_start_cue, "Playback starts at Memory/Hot Cue nearest to the beginning of the track");
        it.hint("Loading the track that is already on the other deck takes over its position, tempo and play state.");
        it.check(52.0, 82.0, &mut p.instant_doubles, "Instant Doubles");
        it.changed
    });
    changed |= item(ui, "ControllerAutoCue", 190.0, |it| {
        it.title_tip("Auto Cue", "When Auto Cue is enabled, a cue point is set at the beginning of the track according to the Auto Cue Level. When a track is loaded, the track will pause at the beginning of the track.");
        it.sub(52.0, 56.0, "Auto Cue Level");
        it.hint("A deck with Auto Cue on cues a track it loads to its first sound at this level or louder (a track with a memory cue goes to that instead).");
        let names = ["-36dB", "-42dB", "-48dB", "-54dB", "-60dB", "-66dB", "-72dB", "-78dB"];
        let mut sel = (p.rb.auto_cue_level.clamp(1, 8) - 1) as usize;
        if it.choice(88.0, 90.0, 520.0, &mut sel, &names) {
            p.rb.auto_cue_level = sel as u8 + 1;
        }
        for (i, (lx, lw, bx, y)) in [(88.0, 54.0, 150.0, 128.0), (377.0, 53.0, 439.0, 128.0), (88.0, 54.0, 150.0, 154.0), (377.0, 53.0, 439.0, 154.0)].into_iter().enumerate() {
            it.text(lx, y, lw + 20.0, 18.0, &format!("Deck {}", i + 1), 16.0, Just::TopLeft);
            it.pill2(bx, y, &mut p.rb.auto_cue_deck[i], "Enable", "Disable");
        }
        it.changed
    });
    changed |= item(ui, "ControllerVinylSpeedAdjust", 258.0, |it| {
        it.title("Vinyl Speed Adjust");
        it.hint("How fast a deck stops and starts in vinyl mode.");
        it.sub(52.0, 56.0, "Touch/Brake");
        for (i, x) in [88.0, 270.0].into_iter().enumerate() {
            let mut t = p.rb.brake[i] as f32 / 255.0;
            if it.knob(x, 90.0, &mut t).changed() {
                p.rb.brake[i] = (t * 255.0).round() as u8;
            }
            it.text(x + 66.0, 106.0, 70.0, 18.0, if i == 0 { "Deck1,3" } else { "Deck2,4" }, 16.0, Just::Left);
        }
        it.sub(52.0, 156.0, "Release/Start");
        for (i, (x, y)) in [(88.0, 188.0), (270.0, 190.0)].into_iter().enumerate() {
            let mut t = p.rb.start[i] as f32 / 255.0;
            if it.knob(x, y, &mut t).changed() {
                p.rb.start[i] = (t * 255.0).round() as u8;
            }
            it.text(x + 66.0, 204.0, 70.0, 18.0, if i == 0 { "Deck1,3" } else { "Deck2,4" }, 16.0, Just::Left);
        }
        it.changed
    });
    changed |= item(ui, "ControllerJog", 216.0, |it| {
        it.title("JOG");
        it.sub(52.0, 56.0, "JOG Rotation Speed");
        it.hint("The platter's turning speed for scratching and the hot cue countdown. Below: CUE held while the jog is spun returns to the cue point when the jog is let go.");
        if it.radio(88.0, 90.0, p.rb.jog_speed == 1, "33rpm") {
            p.rb.jog_speed = 1;
        }
        if it.radio(88.0, 116.0, p.rb.jog_speed == 2, "45rpm") {
            p.rb.jog_speed = 2;
        }
        it.pill_h(88.0, 164.0, 38.0, &mut p.rb.jog_sampler_mode, "After turning on the cue button and spinning the jog,\nthe track returns to the cue point and pause.");
        it.changed
    });
    changed |= item(ui, "ControllerAutoPlay", 108.0, |it| {
        it.title("Continuous Playing");
        it.hint("A deck whose track ends loads the next track of the list and plays it (not while Automix has the deck).");
        it.pill_h(52.0, 56.0, 38.0, &mut p.rb.auto_play, "Automatically load tracks in a list\nand have them play continuously one after another.");
        it.changed
    });
    changed |= sync_type_item(ui, p);
    changed |= hot_cue_item(ui, p);
    changed |= item(ui, "ControllerAutoBeatLoop", 98.0, |it| {
        it.title_tip("AUTO BEAT LOOP", "You can select the length of Auto Beat Loop. When you press the button (or rotary encoder) on the connected hardware, the Auto Beat Loop will start with the selected length.");
        it.hint("The length a controller's or a key's Auto Beat Loop starts with.");
        let names = ["Loop for specified beats", "1 Beat Loop", "2 Beat Loop", "4 Beat Loop", "8 Beat Loop", "16 Beat Loop", "32 Beat Loop"];
        let mut sel = (p.rb.auto_beat_loop.clamp(1, 7) - 1) as usize;
        if it.choice(52.0, 56.0, 556.0, &mut sel, &names) {
            p.rb.auto_beat_loop = sel as u8 + 1;
        }
        it.changed
    });
    if changed {
        app.save_prefs();
    }
}

/// BEAT/BPM SYNC (Controller > Deck, and Advanced > Others in Export mode).
pub fn sync_type_item(ui: &mut Ui, p: &mut Prefs) -> bool {
    item(ui, "ControllerSyncType", 200.0, |it| {
        it.title("BEAT/BPM SYNC");
        it.sub(52.0, 56.0, "Sync Type");
        it.hint("BEAT SYNC matches tempo and beat position, BPM SYNC the tempo only.");
        if it.radio(88.0, 90.0, p.rb.beat_sync_type == 1, "BEAT SYNC") {
            p.rb.beat_sync_type = 1;
        }
        if it.radio(88.0, 116.0, p.rb.beat_sync_type == 2, "BPM SYNC") {
            p.rb.beat_sync_type = 2;
        }
        it.pill(88.0, 164.0, &mut p.rb.multi_bpm_sync, "Allow BEAT/BPM SYNC with double/half BPM.");
        it.changed
    })
}

/// HOT CUE (Controller > Deck, and Advanced > Others in Export mode).
pub fn hot_cue_item(ui: &mut Ui, p: &mut Prefs) -> bool {
    item(ui, "ControllerHotCue", 120.0, |it| {
        it.title_tip("HOT CUE", "For Hot Cues, the Smart Cue option and the Gate Cue option (during Pause) can be selected.\n\nSmart Cue:\nWhen you set a Hot Cue or start playback from a Hot Cue, that point is set as the new Cue point.\n\nGate Cue:\nPlayback starts from the Hot Cue position when the Hot Cue button is pressed.\nKeep pressing the Hot Cue button, the playback continues.\nWhen the Hot Cue button is released, the playback position will return to the Hot Cue position and the playback will pause.");
        it.pill(52.0, 56.0, &mut p.rb.smart_cue, "Overwrite the current Cue with a Hot Cue (Smart Cue)");
        it.pill(52.0, 84.0, &mut p.rb.gate_cue, "Gate playback during Pause (Gate Cue)");
        it.changed
    })
}

// ------------------------------------------------------------------ Mixer

/// The three channel fader curve glyphs (24 x 12 pt), drawn from the curves themselves.
fn curve_glyph(it: &mut Item, x: f32, y: f32, kind: u8) {
    let (p, o) = it.painter();
    let (x0, y1, w, h) = (o.x + x, o.y + y + 14.0, 24.0, 15.0);
    let f = |t: f32| match kind {
        0 => t.powf(4.0),
        1 => t,
        _ => 0.6 + 0.4 * t.powf(0.35),
    };
    let mut pts = vec![egui::pos2(x0, y1)];
    for i in 0..=24 {
        let t = i as f32 / 24.0;
        pts.push(egui::pos2(x0 + t * w, y1 - (f(t) * h).max(if kind == 0 { 1.5 } else { 0.0 })));
    }
    pts.push(egui::pos2(x0 + w, y1));
    let mut m = egui::Mesh::default();
    for q in &pts {
        m.colored_vertex(*q, crate::theme::WHITE.c());
    }
    for i in 1..pts.len() as u32 - 1 {
        // (a fan from the bottom-right corner keeps every triangle inside the shape)
        m.add_triangle(pts.len() as u32 - 1, i - 1, i);
    }
    p.add(egui::Shape::mesh(m));
}

pub fn page_mixer(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    let p = &mut app.prefs;
    changed |= item(ui, "ControllerAutoGain", 92.0, |it| {
        it.title("Auto Gain");
        it.hint("Tracks play at the gain their analysis found (`prefs.rb.auto_gain`).");
        it.pill2(52.0, 56.0, &mut p.rb.auto_gain, "Enable", "Disable");
        it.changed
    });
    changed |= item(ui, "ControllerEqualizerMode", 260.0, |it| {
        it.title("EQ");
        it.sub(52.0, 56.0, "EQ/ISOLATOR");
        it.hint("EQ: HIGH, MID and LOW cut to -26 dB and boost to +6 dB. ISOLATOR: each band can be removed completely.");
        if it.radio(88.0, 90.0, !p.eq_isolator, "EQ") {
            p.eq_isolator = false;
        }
        if it.radio(88.0, 116.0, p.eq_isolator, "ISOLATOR") {
            p.eq_isolator = true;
        }
        it.sep(148.0);
        it.sub(52.0, 164.0, "EQ Type");
        it.hint("The equalizer's curves: DJM-900NXS or DJM-900NXS2.");
        if it.radio(88.0, 198.0, p.rb.eq_type == 0, "DJM-900NXS") {
            p.rb.eq_type = 0;
        }
        if it.radio(88.0, 224.0, p.rb.eq_type == 1, "DJM-900NXS2") {
            p.rb.eq_type = 1;
        }
        it.changed
    });
    changed |= item(ui, "ControllerChannelFaderCurve", 144.0, |it| {
        it.title("Channel Fader Curve");
        for (i, name) in ["Exponential Curve", "Linear Curve", "Logarithmic Curve"].iter().enumerate() {
            let y = 56.0 + 26.0 * i as f32;
            // (An image radio button: the curve's picture 21 pt after the button, the caption
            // 50 pt after it and 2 pt lower than a plain radio button's.)
            let w = crate::prefs_font::width(it.ui, name, crate::ui_kit::CTL, false);
            if it.sense(52.0, y, 50.0 + w, 19.0, "radio", name).clicked() && p.ch_fader_curve != i as u8 {
                p.ch_fader_curve = i as u8;
                it.changed = true;
            }
            let o = it.painter().1;
            crate::ui_kit::paint_radio_at(it.ui.painter(), egui::pos2(o.x + 52.0, o.y + y), 19.0, p.ch_fader_curve == i as u8, true);
            crate::prefs_font::draw(it.ui, egui::Rect::from_min_size(egui::pos2(o.x + 102.0, o.y + y + 2.0), egui::vec2(400.0, 19.0)), name, crate::ui_kit::CTL, false, crate::theme::WHITE.c(), Just::TopLeft);
            curve_glyph(it, 73.0, y, i as u8);
        }
        it.changed
    });
    changed |= item(ui, "ControllerCrossFaderCurve", 126.0, |it| {
        it.title("CROSSFADER Curve");
        it.hint("How sharply the crossfader cuts in (`prefs.rb.crossfader_curve`, 0 to 32).");
        let mut t = p.rb.crossfader_curve.min(32) as f32 / 32.0;
        if it.knob(83.0, 56.0, &mut t).changed() {
            p.rb.crossfader_curve = (t * 32.0).round() as u8;
        }
        // The two curve pictures beside the knob (rekordbox's p4355/p4356 glyphs): a smooth
        // cross on the left, a sharp cut on the right.
        let (pa, o) = it.painter();
        let s = egui::Stroke::new(1.5, crate::theme::WHITE);
        let at = |x: f32, y: f32| egui::pos2(o.x + x, o.y + y);
        pa.line_segment([at(56.0, 87.0), at(76.0, 87.0)], s);
        pa.line_segment([at(57.0, 98.0), at(75.0, 88.0)], s);
        pa.line_segment([at(57.0, 88.0), at(75.0, 98.0)], s);
        pa.line_segment([at(146.0, 87.0), at(166.0, 87.0)], s);
        pa.line_segment([at(147.0, 98.0), at(152.0, 87.5)], s);
        pa.line_segment([at(165.0, 98.0), at(160.0, 87.5)], s);
        it.changed
    });
    changed |= item(ui, "ControllerOutPutLevel", 132.0, |it| {
        it.title_tip("Output Level", "Output level of decks can be adjusted.\nClipping is likely to occur if the level is too high when applying FX or mixing.\nClipping will enable the limiter to prevent distortion, but at the same time it will spoil the attack of a sound.");
        it.sub(52.0, 56.0, "Headroom");
        it.hint("How far the decks are turned down before the mix (`prefs.rb.headroom`).");
        let names = ["NONE", "-3dB", "-6dB", "-9dB", "-12dB", "-15dB", "-18dB", "-21dB"];
        let mut sel = (p.rb.headroom.clamp(1, 8) - 1) as usize;
        if it.choice(88.0, 90.0, 500.0, &mut sel, &names) {
            p.rb.headroom = sel as u8 + 1;
        }
        it.changed
    });
    if changed {
        app.poll_decks();
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Sampler

pub fn page_sampler(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    heading(ui, "ControllerSamplerText", "SAMPLER");
    let default_dir = crate::sampler_ext::capture_dir("").to_string_lossy().to_string();
    let p = &mut app.prefs;
    changed |= item(ui, "ControllerSlotNumber", 150.0, |it| {
        it.title("SLOT");
        if it.radio(52.0, 56.0, p.sampler.slots16, "16") {
            p.sampler.slots16 = true;
        }
        if it.radio(52.0, 82.0, !p.sampler.slots16, "8") {
            p.sampler.slots16 = false;
        }
        it.pill(52.0, 114.0, &mut p.sampler.load_lock, "Do not overwrite the loaded Slot.");
        it.changed
    });
    let mut browse = false;
    changed |= item(ui, "ControllerCapture", 236.0, |it| {
        it.title("Capture");
        it.sub(52.0, 56.0, "Location of Files");
        let dir = if p.sampler.capture_dir.is_empty() { default_dir.clone() } else { p.sampler.capture_dir.clone() };
        let mut reset = false;
        it.combo(88.0, 90.0, 410.0, &dir, |ui| {
            if ui.selectable_label(p.sampler.capture_dir.is_empty(), default_dir.as_str()).clicked() {
                reset = true;
            }
        });
        if reset {
            p.sampler.capture_dir.clear();
            it.changed = true;
        }
        browse = it.button(508.0, 90.0, 100.0, 22.0, "Browse").clicked();
        it.sub(52.0, 128.0, "Location of a Bank for Slicer Capture");
        let mut sel = (p.sampler.slicer_bank.clamp(1, 4) - 1) as usize;
        if it.choice(88.0, 162.0, 520.0, &mut sel, &["Bank1", "Bank2", "Bank3", "Bank4"]) {
            p.sampler.slicer_bank = sel as u8 + 1;
        }
        it.pill(52.0, 200.0, &mut p.sampler.capture_empty, "Capture the loop in an empty slot");
        it.changed
    });
    if browse {
        if let Some(d) = rfd::FileDialog::new().set_title("Location of Files").pick_folder() {
            p.sampler.capture_dir = d.to_string_lossy().to_string();
            changed = true;
        }
    }
    changed |= item(ui, "ControllerAutoPanelMode", 92.0, |it| {
        it.title("Auto Panel Mode");
        it.hint("Enable: the sampler panel opens by itself when the sampler is played.");
        let mut on = !p.sampler.auto_panel_disable;
        if it.pill2(52.0, 56.0, &mut on, "Enable", "Disable") {
            p.sampler.auto_panel_disable = !on;
        }
        it.changed
    });
    heading(ui, "ControllerSequnceText", "SEQUENCER");
    changed |= item(ui, "ControllerSequenceLoad", 92.0, |it| {
        it.title("Sequence Load");
        it.pill(52.0, 56.0, &mut p.sampler.seq.load_autoplay, "Load the sequence and automatically play.");
        it.changed
    });
    if changed {
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Others

/// One line of the quantize table: label, switch and beat value. `first` is the first beat id
/// the dropdown offers (2 = 1/16 for the deck rows, 1 = 1/32 for the sequencer).
fn q_row(it: &mut Item, x: f32, y: f32, label: &str, row: &mut QRow, first: u8) {
    it.child(x, y, |it| {
        it.text(32.0, 3.0, 200.0, 18.0, label, 16.0, Just::Left);
        it.pill2(212.0, 3.0, &mut row.on, "Enable", "Disable");
        let names = &Q_BEATS[first as usize - 1..];
        let mut sel = (row.beat.clamp(first, 6) - first) as usize;
        if it.choice(333.0, 0.0, 200.0, &mut sel, names) {
            row.beat = sel as u8 + first;
        }
    });
}

pub fn page_others(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    let performance = app.performance();
    heading(ui, "ControllerQuantizeText", "Quantize");
    let p = &mut app.prefs;
    changed |= item(ui, "ControllerQuantizeMode", 92.0, |it| {
        it.title_tip("Mode", "When this option is selected, HOT CUE and LOOP/SAMPLER(LOOP) point jumps before reaching the next beat.\nIf this option is not selected, the point jumps when it reaches the next beat.\nYou can select the beat length at [Type] > [QUANTIZE] > [HOT CUE] and [LOOP/SAMPLER(LOOP)].");
        it.hint("On: a hot cue or loop pressed less than half a beat after a beat acts at once; off: it waits for the next beat unless the beat passed less than 1/32 beat ago.");
        it.pill(52.0, 56.0, &mut p.rb.quantize_jump_before, "Jump before reaching the next beat.");
        it.changed
    });
    let mut snap = QRow { on: p.quantize, beat: crate::prefs_rb::RbPrefs::quantize_id_of(p.quantize_div.max(1)) };
    let snap0 = snap;
    let mut seq = QRow { on: p.sampler.seq.quantize_on, beat: p.sampler.seq.quantize.clamp(1, 6) };
    let seq0 = seq;
    changed |= item(ui, "ControllerQuantize", 284.0, |it| {
        it.title("Type ");
        it.hint("Cues and loops set while Q is on go to the nearest line of this grid.");
        q_row(it, 20.0, 56.0, "SNAP", &mut snap, 2);
        it.sep(94.0);
        it.sub(52.0, 110.0, "QUANTIZE");
        it.hint("Each type's switch and beat value: HOT CUE for hot cues called on a playing deck, REVERSE for the turn of REVERSE.");
        q_row(it, 52.0, 146.0, "HOT CUE", &mut p.rb.q_hot_cue, 2);
        q_row(it, 52.0, 178.0, "LOOP/SAMPLER(LOOP)", &mut p.rb.q_loop, 2);
        q_row(it, 52.0, 210.0, "REVERSE", &mut p.rb.q_reverse, 2);
        it.hint("Notes recorded into a sequence go to the nearest line of this grid while the sampler's Q is on.");
        q_row(it, 52.0, 242.0, "SEQUENCER", &mut seq, 1);
        it.changed
    });
    if snap != snap0 {
        p.quantize = snap.on;
        p.quantize_div = crate::prefs_rb::RbPrefs::quantize_div_of(snap.beat);
        p.rb.snap = snap;
        p.rb.deck_quantize = snap.beat;
    }
    if seq != seq0 {
        p.sampler.seq.quantize_on = seq.on;
        p.sampler.seq.quantize = seq.beat;
    }
    if performance {
        changed |= item(ui, "ControllerQuantizeInOneGo", 118.0, |it| {
            it.title("Setting");
            it.hint("All Decks: a deck's Q switches every deck and the sampler.");
            if it.radio(52.0, 56.0, !p.rb.quantize_in_one_go, "Each Deck / Sampler Deck") {
                p.rb.quantize_in_one_go = false;
            }
            if it.radio(52.0, 82.0, p.rb.quantize_in_one_go, "All Decks") {
                p.rb.quantize_in_one_go = true;
            }
            it.changed
        });
    }
    heading(ui, "ControllerAbletonLinkText", "Ableton Link");
    changed |= item(ui, "ControllerAbletonLink", 104.0, |it| {
        it.hint(format!("recordbox has no Ableton Link. {STORED}"));
        it.pill(52.0, 20.0, &mut p.rb.link_start_stop, "Enable Start Stop Sync.");
        it.sep(52.0);
        it.pill(52.0, 68.0, &mut p.rb.link_show_window, "Show the Ableton Link subscreen when Ableton Link is enabled.");
        it.changed
    });
    if changed {
        app.save_prefs();
    }
}

/// A tab another track of the rebuild provides (Recordings, MIX POINT LINK): until its page
/// is merged in, the tab says so instead of showing a made-up page.
pub fn page_pending(ui: &mut Ui, name: &'static str) {
    item(ui, "Pending", 98.0, |it| {
        it.title(name);
        it.text(52.0, 56.0, 550.0, 18.0, "This page is not part of this build of recordbox yet.", 16.0, Just::Left);
    });
}

// ------------------------------------------------------------------ DVS

/// Preferences > DVS as rekordbox shows it with no DVS audio interface connected: the
/// function switch, and the four items greyed.
pub fn page_dvs(app: &mut App, ui: &mut Ui) {
    let mut changed = false;
    let p = &mut app.prefs;
    let why = "recordbox has no DVS (control vinyl / control CD) input.";
    changed |= item_bare(ui, "DVSFunction", 39.0, |it| {
        it.hint(format!("{why} {STORED}"));
        it.pill(20.0, 20.0, &mut p.rb.dvs_enable, "Enable DVS Function");
        it.changed
    });
    item(ui, "DVSControlSignalSetting", 530.0, |it| {
        it.text_bold(20.0, 20.0, 580.0, 22.0, "Control Signal Setting", 18.0, Just::Left);
        it.tip_by_label(20.0, 19.0, "Control Signal Setting", 18.0, "-AUTO\nSTEP1. Adjust input signals.\nStart the turntable and click [Start] of L/R and P/A Balance.\n\n-MANUAL\nSTEP1. Adjust input signals (L/R Balance, P/A Balance)\nSTEP2. Adjust threshold for reading of control signal.");
        it.en = false;
        it.hint(why);
        // The four signal scopes (one per deck): without an input they are empty frames.
        for (i, (x, y)) in [(20.0, 56.0), (324.0, 56.0), (20.0, 288.0), (324.0, 288.0)].into_iter().enumerate() {
            let (pa, o) = it.painter();
            let r = egui::Rect::from_min_size(egui::pos2(o.x + x, o.y + y), egui::vec2(294.0, 222.0));
            pa.rect_filled(r, 0.0, crate::theme::gy(0x191919));
            let scope = egui::Rect::from_min_size(egui::pos2(r.min.x + 12.0, r.min.y + 36.0), egui::vec2(120.0, 120.0));
            pa.rect_filled(scope, 0.0, egui::Color32::BLACK);
            pa.line_segment([scope.center_top(), scope.center_bottom()], egui::Stroke::new(1.0, crate::theme::gy(0x323232)));
            pa.line_segment([scope.left_center(), scope.right_center()], egui::Stroke::new(1.0, crate::theme::gy(0x323232)));
            it.text(x + 8.0, y + 8.0, 120.0, 18.0, &format!("DECK{}", i + 1), 16.0, Just::Left);
        }
    });
    changed |= item(ui, "DVSWaveformDisplay", 142.0, |it| {
        it.title_tip("Waveform display", "With DVS, the enlarged waveform can keep its beat spacing or follow the tempo of the control signal.");
        it.sub(52.0, 56.0, "When the tempo is changed");
        it.en = false;
        it.hint(why);
        it.pill_h(88.0, 90.0, 38.0, &mut p.rb.dvs_fixed_bpm_scale, "The beat position intervals of the enlarged waveform \nare changed.");
        it.changed
    });
    changed |= item(ui, "DVSControlVinyl", 142.0, |it| {
        it.title("Control Vinyl");
        it.sub_tip(52.0, 56.0, "Needle operation", "What happens when the needle is put down before the first groove of the control signal.");
        it.en = false;
        it.hint(why);
        it.pill_h(88.0, 90.0, 38.0, &mut p.rb.dvs_lead_in, "When the needle is placed on the lead-in part of the vinyl, \nthe playback position jumps to the beginning of the track.");
        it.changed
    });
    changed |= item(ui, "DVSRoutingSetting", 591.0, |it| {
        it.title("Routing Setting");
        it.en = false;
        it.hint(why);
        it.child(20.0, 56.0, |it| {
            it.sub(32.0, 0.0, "Control Hardware");
            let mut sel = p.rb.dvs_hardware.min(2) as usize;
            it.radios(64.0, 34.0, &mut sel, &["Turntables", "CDJs", "Customized Setting"]);
        });
        it.child(20.0, 190.0, |it| {
            it.sub(32.0, 0.0, "Audio Routing");
            // Two decks above the mixer channels they are wired to.
            for (i, x) in [64.0, 354.0].into_iter().enumerate() {
                let (pa, o) = it.painter();
                let r = egui::Rect::from_min_size(egui::pos2(o.x + x, o.y + 32.0 + 27.0), egui::vec2(150.0, 133.0));
                pa.rect_stroke(r, 4.0, egui::Stroke::new(1.0, crate::theme::gy(0x646464)), egui::StrokeKind::Inside);
                pa.circle_stroke(r.center(), 44.0, egui::Stroke::new(1.0, crate::theme::gy(0x646464)));
                it.text(x + 33.0, 32.0, 84.0, 27.0, &format!("Deck {}", i + 1), 20.0, Just::Centre);
            }
            for (i, x) in [74.0, 152.0, 230.0, 308.0, 386.0, 464.0].into_iter().enumerate() {
                let (pa, o) = it.painter();
                pa.circle_stroke(egui::pos2(o.x + x + 15.0, o.y + 256.0 + 15.0), 9.0, egui::Stroke::new(1.5, crate::theme::gy(0x646464)));
                it.text(x - 5.0, 256.0 + 30.0, 40.0, 30.0, &format!("{}", i + 1), 20.0, Just::Centre);
            }
            it.text(64.0, 322.0, 440.0, 22.0, "AUDIO IF CHANNEL", 20.0, Just::Centre);
            it.text(32.0, 352.0, 556.0, 18.0, "Open [Preferences] > [Audio] and follow instructions below.", 14.0, Just::Left);
        });
        it.changed
    });
    if changed {
        app.save_prefs();
    }
}
