//! The MIDI setting window (MIDI Learn), the `MIDI` button of the Performance top bar and the
//! controller rows of Preferences, after rekordbox 7.2.19 (`djplay::MidiLearnComponent`,
//! `checks/re-7.2.19/controllers-midi.md` 2.2 and 5).
//!
//! There is no picture of rekordbox's window. Its geometry is its `resized()` (0x10165718c),
//! its colours are those its look-and-feel sets (`MidiLearnLookAndFeel` 0x10165f698) and
//! those its table paints (`paintRowBackground` 0x101663b7c, `paintCell` 0x101663c04,
//! `drawTableHeaderBackground` 0x10165fadc), and the widgets are drawn the way JUCE's
//! LookAndFeel_V4 draws them with those colours (buttons 6 pt corners, combo boxes 3 pt, tabs
//! of LookAndFeel_V3). What that leaves unverified is listed in PARITY-GAPS.md.

use crate::app::*;
use crate::midi::*;
use crate::midi_csv::CsvFile;
use crate::midi_map::{LearnEntry, LearnFilter, Reject, NO_FUNCTION};
use crate::theme::*;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};
use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;

/// Content size of the window, and its title bar.
pub const W: f32 = 1000.0;
pub const H: f32 = 600.0;
const TITLE_H: f32 = 28.0;
/// Table row and header heights (JUCE's defaults: 22 and 28).
const ROW: f32 = 22.0;
const HEAD: f32 = 28.0;

/// (header, width): the six columns rekordbox shows. CSVNAME, Encoder Mode and Color exist
/// in its table and are hidden: `MidiLearnTableComponent` (0x101663738) adds every column with
/// flags 19 (visible, resizable, sortable; not on a column menu, not draggable), width limits
/// 50..400, then hides columns 2, 6 and 8, so they cannot be brought back on screen. The
/// Encoder Mode cell's list (" ", "7F/01", "41/3F") is therefore never seen.
const COLS: [(&str, f32); 6] = [("Function", 200.0), ("Deck", 100.0), ("MIDI IN", 100.0), ("Type", 150.0), ("MIDI OUT", 100.0), ("Comment", 200.0)];

const TEXT: Color32 = hex(0xEDEDED);
const BTN_BG: Color32 = hex(0x0A0A0A);
const BTN_ON: Color32 = hex(0x808080);
const BOX_BG: Color32 = hex(0x202020);
const MENU_HI: Color32 = hex(0x555555);
const TAB: Color32 = hex(0x555555);

/// A font of JUCE height `h` px: Arial's point size is 0.895 of its height. (Helvetica
/// stands in for Arial regular, which recordbox does not load.)
fn jf(h: f32) -> FontId {
    rb(h * 0.895)
}

/// One row of the table.
#[derive(Clone, Debug, PartialEq)]
pub struct LearnRow {
    pub name: String,
    pub display: String,
    /// 0 for a command without a deck, else 1..4.
    pub deck: u8,
    pub input: Option<u16>,
    pub output: Option<u16>,
    pub ty: u8,
    pub comment: String,
    pub min: i32,
    pub max: i32,
}

impl LearnRow {
    fn entry(&self) -> LearnEntry {
        // "Button(for Pad)" is a Button with LED values; they default to 0 / 127.
        let (min, max) = match self.ty {
            T_BUTTON_PAD => (if self.min == -1 && self.max == -1 { 0 } else { self.min }, if self.max == -1 { 127 } else { self.max }),
            T_BUTTON => (-1, -1),
            _ => (self.min, self.max),
        };
        LearnEntry { name: self.name.clone(), deck: self.deck, kind: kind_of_type(self.ty), min, max, comment: self.comment.clone() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Delete,
    /// DEFAULT: export first?
    Default,
    /// IMPORT: save first?
    Import,
}

/// The window's state (a field of `MidiState`).
#[derive(Default)]
pub struct Win {
    /// Column widths after the user dragged a header edge (None: rekordbox's widths).
    pub widths: Option<[f32; 6]>,
    pub open: bool,
    /// Top-left of the content on screen.
    pub pos: Option<Pos2>,
    /// The selected device ("" = Please Select).
    pub device: String,
    pub tab: usize,
    pub rows: Vec<Vec<LearnRow>>,
    pub sel: Vec<Option<usize>>,
    pub learn: bool,
    filter: LearnFilter,
    /// An alert: (title, message).
    pub alert: Option<(String, String)>,
    pub ask: Option<Ask>,
    /// A cell being typed in: (row, column, text).
    edit: Option<(usize, usize, String)>,
    /// What the last edit ran into: the input-side and the output-side entry.
    clash: (Option<crate::midi_csv::Entry>, Option<crate::midi_csv::Entry>),
    sort: Option<(usize, bool)>,
    /// Where things were drawn, by name (for the scripted check).
    pub at: HashMap<String, Pos2>,
    /// LEARN wrote a code (for the scripted check).
    pub learned: u32,
}

fn tab_of(i: usize) -> Tab {
    TABS[i.min(TABS.len() - 1)].0
}

fn tab_index(t: Tab) -> Option<usize> {
    TABS.iter().position(|x| x.0 == t)
}

/// The tabs on screen: VIDEO is added only when video is enabled, which it never is here.
fn shown_tabs() -> Vec<usize> {
    (0..TABS.len()).filter(|i| tab_of(*i) != Tab::Video).collect()
}

// ---------------------------------------------------------------------- the top bar's button

/// The `MIDI` text button of the Performance top bar. rekordbox places it 88 pt left of its
/// plan label (`MainComponent::resized`: 30 x 16 at y 8); `r` is that rectangle.
pub fn header_button(app: &mut App, ui: &mut Ui, r: Rect) {
    let enabled = app.midi.connected();
    // Without a controller the window closes (rekordbox: `updateMidiLearnButton`).
    if !enabled && app.midi.win.open {
        close(app);
    }
    let resp = ui.interact(r.expand2(vec2(2.0, 4.0)), ui.id().with("midi_button"), if enabled { Sense::click() } else { Sense::hover() });
    // Arial Bold, JUCE height 12. Disabled: #363636 as measured on the reference
    // (performance/01); enabled: the bar's white (not captured).
    let col = if !enabled { hex(0x363636) } else if app.midi.win.open { BLUE } else if resp.hovered() { WHITE } else { hex(0xE6E6E6) };
    ui.painter().text(r.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, "MIDI", caps(10.7), col);
    app.midi.win.at.insert("MIDI".into(), r.center());
    let base = "MIDI LEARN :\nThe buttons, knobs, and faders on MIDI controllers can be mapped to perform different functions.";
    let tip = if enabled { base.to_string() } else { format!("{base}\nThis can be used after a MIDI controller is connected.") };
    if resp.on_hover_text(tip).clicked() && enabled {
        if app.midi.win.open {
            close(app);
        } else {
            app.midi.win.open = true;
        }
    }
}

// ---------------------------------------------------------------------- window logic

/// Close the window: learning stops and the mapping is saved.
pub fn close(app: &mut App) {
    set_learn(app, false);
    save(app);
    let w = &mut app.midi.win;
    w.open = false;
    w.edit = None;
    w.alert = None;
    w.ask = None;
}

fn device(app: &App) -> Option<std::sync::Arc<Device>> {
    app.midi.hub.as_ref()?.device(&app.midi.win.device)
}

fn save(app: &mut App) {
    if let (Some(hub), Some(dev)) = (app.midi.hub.clone(), device(app)) {
        if let Err(e) = hub.save_map(&dev) {
            app.toast_err(format!("The MIDI setting could not be saved: {e}"));
        }
    }
}

pub fn set_learn(app: &mut App, on: bool) {
    let on = on && device(app).is_some();
    app.midi.win.learn = on;
    app.midi.win.filter.reset();
    if let Some(hub) = app.midi.hub.as_ref() {
        for d in hub.devices.lock().iter() {
            d.learning.store(on && d.name == app.midi.win.device, Relaxed);
        }
    }
}

/// Pick the device to edit (or none): learning stops, the table shows its mapping.
pub fn select_device(app: &mut App, name: &str) {
    set_learn(app, false);
    save(app);
    app.midi.win.device = name.to_string();
    app.midi.win.edit = None;
    rebuild_rows(app);
}

/// The table of the selected device (`addMidiLearnTableData`): every input that is not
/// read-only and whose function is one of the window's commands, with its output; then the
/// outputs that have no input (indicators).
pub fn rebuild_rows(app: &mut App) {
    let reg = registry();
    let mut rows: Vec<Vec<LearnRow>> = vec![vec![]; TABS.len()];
    if let Some(dev) = device(app) {
        let map = dev.map.lock();
        let mut ins: Vec<&crate::midi_csv::Entry> = map.live_inputs().map(|i| &i.e).collect();
        ins.sort_by_key(|e| e.code);
        let mut used = vec![false; map.outputs.len()];
        let mk = |e: &crate::midi_csv::Entry, input: Option<u16>, output: Option<u16>| {
            let f = reg.by_name(&e.name)?;
            let t = tab_index(f.tab)?;
            Some((t, LearnRow { name: e.name.clone(), display: f.display.clone(), deck: e.deck, input, output, ty: type_of_entry(e), comment: e.comment.clone(), min: e.opts.min, max: e.opts.max }))
        };
        for e in ins {
            if e.opts.ro || e.kind == crate::midi_csv::Kind::Parameter {
                continue;
            }
            // Its output: the same function and deck, the same code first.
            let same = |o: &crate::midi_csv::Entry| o.name == e.name && o.deck == e.deck;
            let oi = map.outputs.iter().enumerate().position(|(i, o)| !used[i] && same(o) && o.code == e.code).or_else(|| map.outputs.iter().enumerate().position(|(i, o)| !used[i] && same(o)));
            if let Some((t, row)) = mk(e, Some(e.code), oi.map(|i| map.outputs[i].code)) {
                if let Some(i) = oi {
                    used[i] = true;
                }
                rows[t].push(row);
            }
        }
        for (i, o) in map.outputs.iter().enumerate() {
            if used[i] || o.opts.ro {
                continue;
            }
            if let Some((t, row)) = mk(o, None, Some(o.code)) {
                rows[t].push(row);
            }
        }
    }
    let w = &mut app.midi.win;
    w.rows = rows;
    w.sel = vec![None; TABS.len()];
    w.sort = None;
}

/// The device list changed.
pub fn devices_changed(app: &mut App) {
    let name = app.midi.win.device.clone();
    if !name.is_empty() && !app.midi.devices.contains(&name) {
        // The selected controller was unplugged.
        app.midi.win.learn = false;
        app.midi.win.device.clear();
        app.midi.win.edit = None;
        rebuild_rows(app);
    }
    if app.midi.devices.is_empty() && app.midi.win.open {
        close(app);
    }
}

/// The text of a refusal (`showAlertIfDuplicated` 0x10165d4f4).
fn clash_alert(input: &Option<crate::midi_csv::Entry>, output: &Option<crate::midi_csv::Entry>) -> Option<String> {
    const CANNOT: &str = "You cannot change the assignment for this MIDI code.";
    const TAKEN: &str = "This MIDI code is already assigned.";
    let reg = registry();
    let known = |e: &crate::midi_csv::Entry| reg.by_name(&e.name).filter(|f| f.tab != Tab::None).map(|f| (TABS.iter().find(|t| t.0 == f.tab).map(|t| t.1).unwrap_or(""), f.display.clone()));
    let code_in = |e: &crate::midi_csv::Entry| format!("\nMIDI IN: {:04X}", e.code);
    let code_out = |e: &crate::midi_csv::Entry| format!("\nMIDI OUT: {:04X}", e.code);
    let named = |e: &crate::midi_csv::Entry| known(e).map(|(tab, name)| format!("{tab}: \n{name}\n")).unwrap_or_default();
    match (input, output) {
        (None, None) => None,
        (i, o) if i.as_ref().map(|e| e.opts.ro).unwrap_or(false) || o.as_ref().map(|e| e.opts.ro).unwrap_or(false) => Some(format!("{CANNOT}\n{}{}", i.as_ref().map(code_in).unwrap_or_default(), o.as_ref().map(code_out).unwrap_or_default())),
        (Some(i), None) => Some(if known(i).is_none() { format!("{CANNOT}\n{}", code_in(i)) } else { format!("{TAKEN}\n\n{}{}", named(i), code_in(i)) }),
        (None, Some(o)) => Some(if known(o).is_none() { format!("{CANNOT}\n{}", code_out(o)) } else { format!("{TAKEN}\n\n{}{}", named(o), code_out(o)) }),
        (Some(i), Some(o)) => Some(if known(i).is_none() && known(o).is_none() {
            format!("{CANNOT}\n{}{}", code_in(i), code_out(o))
        } else if i.name == o.name {
            format!("{TAKEN}\n\n{}{}{}", named(i), code_in(i), code_out(o))
        } else {
            format!("{TAKEN}\n\n{}{}\n\n{}{}", named(i), code_in(i), named(o), code_out(o))
        }),
    }
}

fn show_clash(app: &mut App) {
    let (i, o) = std::mem::take(&mut app.midi.win.clash);
    if let Some(text) = clash_alert(&i, &o) {
        app.midi.win.alert = Some((String::new(), text));
    }
}

/// MIDI IN of row `r` of the current tab becomes `new`. True when it was taken.
fn commit_in(app: &mut App, r: usize, new: Option<u16>) -> bool {
    let tab = app.midi.win.tab;
    let Some(row) = app.midi.win.rows.get(tab).and_then(|v| v.get(r)).cloned() else { return false };
    let Some(dev) = device(app) else { return false };
    if row.input == new {
        return true;
    }
    let reg = registry();
    let res = dev.map.lock().change_input(&row.entry(), row.input, new, &|n| reg.learnable(n));
    match res {
        Ok(()) => {
            app.midi.win.rows[tab][r].input = new;
            dev.reindex();
            true
        }
        Err(Reject::Taken(e)) => {
            app.midi.win.clash.0 = Some(e);
            false
        }
        Err(Reject::Revert) => false,
    }
}

fn commit_out(app: &mut App, r: usize, new: Option<u16>) -> bool {
    let tab = app.midi.win.tab;
    let Some(row) = app.midi.win.rows.get(tab).and_then(|v| v.get(r)).cloned() else { return false };
    let Some(dev) = device(app) else { return false };
    if row.output == new {
        return true;
    }
    let reg = registry();
    let res = dev.map.lock().change_output(&row.entry(), row.output, new, &|n| reg.learnable(n));
    match res {
        Ok(()) => {
            app.midi.win.rows[tab][r].output = new;
            dev.reindex();
            true
        }
        Err(Reject::Taken(e)) => {
            app.midi.win.clash.1 = Some(e);
            false
        }
        Err(Reject::Revert) => false,
    }
}

/// Deck, Type or Comment of a row changed: its entries are rewritten in place.
fn commit_row(app: &mut App, r: usize) {
    let tab = app.midi.win.tab;
    let Some(row) = app.midi.win.rows.get(tab).and_then(|v| v.get(r)).cloned() else { return };
    let Some(dev) = device(app) else { return };
    dev.map.lock().rewrite(&row.entry(), row.input, row.output);
    dev.reindex();
    save(app);
}

/// A message seen while LEARN is armed (`eventPropertyChangedDelegate`): its code goes into
/// the selected row of the current tab as MIDI IN and as MIDI OUT.
pub fn learn_message(app: &mut App, dev_name: &str, msg: [u8; 3]) {
    let w = &mut app.midi.win;
    // Nothing is learned while another window asks something.
    if !w.open || !w.learn || w.device != dev_name || w.alert.is_some() || w.ask.is_some() {
        return;
    }
    let Some(code) = w.filter.feed(&msg) else { return };
    let Some(r) = w.sel.get(w.tab).copied().flatten() else { return };
    w.clash = (None, None);
    let a = commit_in(app, r, Some(code));
    let b = commit_out(app, r, Some(code));
    if a || b {
        app.midi.win.learned += 1;
        save(app);
    }
    show_clash(app);
}

/// ADD: the command joins the current tab without codes, on deck 1 when it has decks.
pub fn add_row(app: &mut App, name: &str) {
    let Some(f) = registry().by_name(name) else { return };
    let w = &mut app.midi.win;
    let tab = w.tab;
    if w.rows.len() <= tab {
        return;
    }
    w.rows[tab].push(LearnRow { name: f.name.clone(), display: f.display.clone(), deck: f.per_deck as u8, input: None, output: None, ty: f.ty, comment: String::new(), min: -1, max: -1 });
    w.sel[tab] = Some(w.rows[tab].len() - 1);
}

/// DUPLICATE: a copy of the selected row without its codes, right under it.
pub fn duplicate_row(app: &mut App) {
    let w = &mut app.midi.win;
    let tab = w.tab;
    let Some(r) = w.sel.get(tab).copied().flatten() else { return };
    let Some(mut row) = w.rows[tab].get(r).cloned() else { return };
    row.input = None;
    row.output = None;
    w.rows[tab].insert(r + 1, row);
    w.sel[tab] = Some(r + 1);
}

/// DELETE (after the confirmation): the row goes, and its codes stay switched off.
pub fn delete_row(app: &mut App) {
    let tab = app.midi.win.tab;
    let Some(r) = app.midi.win.sel.get(tab).copied().flatten() else { return };
    if r >= app.midi.win.rows[tab].len() {
        return;
    }
    let row = app.midi.win.rows[tab].remove(r);
    app.midi.win.sel[tab] = None;
    if let Some(dev) = device(app) {
        dev.map.lock().delete(row.input, row.output);
        dev.reindex();
    }
    save(app);
}

fn pick_file(save_name: Option<&str>) -> Option<std::path::PathBuf> {
    // (The scripted check names the file instead of using the panel.)
    if let Ok(p) = std::env::var("PREPDECK_MIDI_FILE") {
        return Some(p.into());
    }
    let docs = dirs::document_dir().unwrap_or_else(|| ".".into());
    match save_name {
        Some(n) => rfd::FileDialog::new().set_title("Select a location").set_directory(docs).set_file_name(n).add_filter("csv", &["csv"]).save_file(),
        None => rfd::FileDialog::new().set_title("Select MIDI setting file").set_directory(docs).add_filter("csv", &["csv"]).pick_file(),
    }
}

/// EXPORT: the user's part of the mapping, in rekordbox's file format. False when the file
/// could not be written; None when no place was chosen.
pub fn export(app: &mut App) -> Option<bool> {
    let dev = device(app)?;
    let path = pick_file(Some(&crate::midi_csv::export_name(&dev.name)))?;
    let text = dev.map.lock().save_text();
    let ok = std::fs::write(&path, text).is_ok();
    if !ok {
        app.midi.win.alert = Some((String::new(), "Export failed.".into()));
    }
    Some(ok)
}

/// IMPORT: the chosen file becomes the user's mapping of the selected device.
pub fn import(app: &mut App) {
    let (Some(hub), Some(dev)) = (app.midi.hub.clone(), device(app)) else { return };
    let Some(path) = pick_file(None) else { return };
    let Ok(bytes) = std::fs::read(&path) else {
        app.midi.win.alert = Some((String::new(), "Import failed.".into()));
        return;
    };
    match CsvFile::parse(&bytes) {
        Ok(f) if f.device.eq_ignore_ascii_case(&dev.name) => {
            let _ = std::fs::create_dir_all(&hub.user_dir);
            if std::fs::write(hub.user_file(&dev.name), &bytes).is_err() {
                app.midi.win.alert = Some((String::new(), "Import failed.".into()));
                return;
            }
        }
        Ok(_) => {
            app.midi.win.alert = Some(("Cannot import the MIDI setting file".into(), "You can import a MIDI setting file only for the selected equipment.".into()));
            return;
        }
        Err(_) => {
            // Not a mapping file: the setting goes back to the default.
            let _ = std::fs::write(hub.user_file(&dev.name), crate::midi_csv::empty_user_file(&dev.name));
            app.midi.win.alert = Some((String::new(), "Import failed.\nThe file is not in a supported format.\nThe MIDI setting will be reset back to default.".into()));
        }
    }
    *dev.map.lock() = hub.load_map(&dev.name).0;
    dev.reindex();
    rebuild_rows(app);
}

/// DEFAULT: the user's mapping is emptied (a supported controller is back on its factory
/// mapping; any other has nothing mapped).
pub fn reset_default(app: &mut App) {
    let (Some(hub), Some(dev)) = (app.midi.hub.clone(), device(app)) else { return };
    let _ = std::fs::create_dir_all(&hub.user_dir);
    if std::fs::write(hub.user_file(&dev.name), crate::midi_csv::empty_user_file(&dev.name)).is_err() {
        app.midi.win.alert = Some((String::new(), "MIDI settings did not reset back to default.".into()));
        return;
    }
    *dev.map.lock() = hub.load_map(&dev.name).0;
    dev.reindex();
    rebuild_rows(app);
}

fn sort_rows(w: &mut Win, col: usize) {
    let asc = match w.sort {
        Some((c, a)) if c == col => !a,
        _ => true,
    };
    w.sort = Some((col, asc));
    let tab = w.tab;
    let Some(rows) = w.rows.get_mut(tab) else { return };
    let hexs = |c: Option<u16>| c.map(|c| format!("{c:04X}")).unwrap_or_default();
    let key = |r: &LearnRow| match col {
        0 => r.display.clone(),
        1 => r.deck.to_string(),
        2 => hexs(r.input),
        3 => type_text(r.ty).to_string(),
        4 => hexs(r.output),
        _ => r.comment.clone(),
    };
    rows.sort_by(|a, b| if asc { key(a).cmp(&key(b)) } else { key(b).cmp(&key(a)) });
    w.sel[tab] = None;
}

// ---------------------------------------------------------------------- drawing

/// A text button as JUCE's LookAndFeel_V4 draws it: 6 pt corners (square on a side joined to
/// its neighbour), 1 pt outline, text at 0.6 of the height.
fn text_button(ui: &mut Ui, r: Rect, id: &str, label: &str, on: bool, enabled: bool, join: (bool, bool)) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), if enabled { Sense::click() } else { Sense::hover() });
    let mut bg = if on { BTN_ON } else { BTN_BG };
    if enabled && resp.is_pointer_button_down_on() {
        bg = mix(bg, 0.2);
    } else if enabled && resp.hovered() {
        bg = mix(bg, 0.05);
    }
    let a = if enabled { 1.0 } else { 0.5 };
    let cr = |sq: bool| if sq { 0 } else { 6 };
    let rounding = egui::CornerRadius { nw: cr(join.0), sw: cr(join.0), ne: cr(join.1), se: cr(join.1) };
    let p = ui.painter();
    p.rect_filled(r.shrink(0.5), rounding, bg.gamma_multiply(a));
    p.rect_stroke(r.shrink(0.5), rounding, Stroke::new(1.0_f32, Color32::BLACK), egui::StrokeKind::Middle);
    p.text(r.center(), Align2::CENTER_CENTER, label, jf((r.height() * 0.6).min(16.0)), TEXT.gamma_multiply(a));
    resp
}

/// JUCE's `Colour::contrasting(amount)` for the greys used here: towards white for a dark
/// colour, towards black for a light one.
fn mix(c: Color32, amount: f32) -> Color32 {
    let to = if c.r() as u32 + c.g() as u32 + c.b() as u32 >= 384 { 0.0 } else { 255.0 };
    let f = |v: u8| (v as f32 + (to - v as f32) * amount).round() as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// A combo box as LookAndFeel_V4 draws it: 3 pt corners, the arrow in the 20 pt zone 30 pt
/// from the right edge.
fn combo_box(ui: &mut Ui, r: Rect, id: impl std::hash::Hash, text: &str, dim: bool, enabled: bool) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    p.rect_filled(r, 3.0, BOX_BG);
    p.rect_stroke(r.shrink(0.5), 3.0, Stroke::new(1.0_f32, Color32::BLACK), egui::StrokeKind::Middle);
    let (ax, cy) = (r.max.x - 30.0, r.center().y);
    let col = Color32::WHITE.gamma_multiply(if enabled { 0.9 } else { 0.2 });
    p.add(egui::Shape::line(vec![pos2(ax + 3.0, cy - 2.0), pos2(ax + 10.0, cy + 3.0), pos2(ax + 17.0, cy - 2.0)], Stroke::new(2.0_f32, col)));
    let tc = TEXT.gamma_multiply(if dim || !enabled { 0.5 } else { 1.0 });
    p.with_clip_rect(Rect::from_min_max(r.min, pos2(r.max.x - 30.0, r.max.y))).text(pos2(r.min.x + 6.0, cy), Align2::LEFT_CENTER, text, jf((r.height() * 0.85).min(15.0)), tc);
    resp
}

/// A popup list in the window's menu colours. Returns the index picked.
fn menu_popup(ui: &mut Ui, resp: &egui::Response, id: egui::Id, width: f32, items: &[String], at: &mut HashMap<String, Pos2>) -> Option<usize> {
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    let mut hit = None;
    menu_style(ui);
    egui::popup::popup_below_widget(ui, id, resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(width);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for (i, it) in items.iter().enumerate() {
            let r = menu_item(ui, it, false);
            at.insert(format!("item:{it}"), r.rect.center());
            if r.clicked() {
                hit = Some(i);
                ui.memory_mut(|m| m.close_popup());
            }
        }
    });
    hit
}

fn menu_style(ui: &mut Ui) {
    let v = ui.visuals_mut();
    v.window_fill = BOX_BG;
    v.panel_fill = BOX_BG;
    v.window_stroke = Stroke::new(1.0_f32, hex(0x101010));
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.menu_corner_radius = egui::CornerRadius::same(0);
}

fn menu_item(ui: &mut Ui, label: &str, sub: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(140.0), 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, MENU_HI);
    }
    ui.painter().text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, label, jf(15.0), TEXT);
    if sub {
        let c = pos2(r.max.x - 10.0, r.center().y);
        ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-3.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-3.0, 4.0)], TEXT, Stroke::NONE));
    }
    resp
}

/// The window, when it is open.
pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.midi.win.open {
        return;
    }
    if app.midi.win.rows.len() != TABS.len() {
        rebuild_rows(app);
    }
    let origin = *app.midi.win.pos.get_or_insert(pos2(100.0, 128.0));
    let outer = Rect::from_min_size(origin - vec2(0.0, TITLE_H), vec2(W, H + TITLE_H));
    let modal = app.midi.win.alert.is_some() || app.midi.win.ask.is_some();
    let mut close_now = false;
    egui::Area::new(egui::Id::new("midi_setting")).order(egui::Order::Foreground).fixed_pos(outer.min).show(ctx, |ui| {
        ui.set_clip_rect(outer.expand(1.0));
        let (_, bg) = ui.allocate_exact_size(outer.size(), Sense::click());
        let _ = bg;
        let p = ui.painter().clone();
        // The title bar (a native one in rekordbox): the close button and the title.
        let title = Rect::from_min_size(outer.min, vec2(W, TITLE_H));
        p.rect_filled(outer.expand(1.0), 6.0, hex(0x3A3A3A));
        p.rect_filled(title, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, hex(0x2A2A2C));
        p.text(title.center(), Align2::CENTER_CENTER, "MIDI setting", bold(13.0), hex(0xD0D0D0));
        let drag = ui.interact(title, ui.id().with("title"), Sense::drag());
        if drag.dragged() {
            if let Some(pos) = app.midi.win.pos.as_mut() {
                *pos += drag.drag_delta();
            }
        }
        let cb = Rect::from_center_size(pos2(title.min.x + 14.0, title.center().y), vec2(12.0, 12.0));
        let cresp = ui.interact(cb.expand(3.0), ui.id().with("close"), Sense::click());
        p.circle_filled(cb.center(), 6.0, hex(0xFF5F57));
        if cresp.hovered() {
            let c = cb.center();
            p.line_segment([c + vec2(-2.5, -2.5), c + vec2(2.5, 2.5)], Stroke::new(1.2_f32, hex(0x4D0000)));
            p.line_segment([c + vec2(-2.5, 2.5), c + vec2(2.5, -2.5)], Stroke::new(1.2_f32, hex(0x4D0000)));
        }
        app.midi.win.at.insert("close".into(), cb.center());
        if cresp.clicked() && !modal {
            close_now = true;
        }
        let body = Rect::from_min_size(origin, vec2(W, H));
        p.rect_filled(body, 0.0, Color32::BLACK);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("midi_body"));
        child.set_clip_rect(body);
        if modal {
            child.disable();
        }
        content(app, &mut child, origin, !modal);
    });
    if close_now {
        close(app);
        return;
    }
    dialogs(app, ctx);
}

fn content(app: &mut App, ui: &mut Ui, o: Pos2, live: bool) {
    let p = ui.painter().clone();
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(o + vec2(x, y), vec2(w, h));
    let has_dev = device(app).is_some();
    let tips = app.prefs.tooltips;
    let tip = move |r: egui::Response, t: &str| if tips { r.on_hover_text(t) } else { r };

    // ---- the device
    p.text(o + vec2(32.0 + 5.0, 24.0), Align2::LEFT_CENTER, "Connected device", jf(15.0), Color32::WHITE);
    let names = app.midi.devices.clone();
    let cur = app.midi.win.device.clone();
    let shown = if !cur.is_empty() { cur.clone() } else if names.is_empty() { "(no choices)".to_string() } else { "Please Select".to_string() };
    let cr = at(16.0, 40.0, 208.0, 24.0);
    let cresp = combo_box(ui, cr, "device", &shown, cur.is_empty(), live);
    app.midi.win.at.insert("device".into(), cr.center());
    let cresp = tip(cresp, "Display devices connected to the computer that can set MIDI setting.\nWhen multiple rekordbox dj supporting devices are connected, only one device will be displayed. \nYou can edit MIDI setting of the selected device.");
    let mut at_map = std::mem::take(&mut app.midi.win.at);
    if let Some(i) = menu_popup(ui, &cresp, ui.id().with("device_pop"), 208.0, &names, &mut at_map) {
        app.midi.win.at = at_map;
        select_device(app, &names[i]);
    } else {
        app.midi.win.at = at_map;
    }
    if let Some(d) = device(app).filter(|d| d.needs_auth) {
        p.text(o + vec2(240.0, 52.0), Align2::LEFT_CENTER, format!("{}: rekordbox unlocks this model with a hardware handshake that recordbox does not have. It may not respond.", d.name), jf(13.0), hex(0xE04848));
    }

    // ---- DEFAULT
    let r = at(W - 184.0, 40.0, 150.0, 24.0);
    app.midi.win.at.insert("DEFAULT".into(), r.center());
    if tip(text_button(ui, r, "default", "DEFAULT", false, has_dev && live, (false, false)), "Reset MIDI setting of the selected equipment back to default.").clicked() {
        app.midi.win.ask = Some(Ask::Default);
    }

    // ---- tabs
    let tabs = shown_tabs();
    let mut tx = o.x + 8.0;
    for ti in &tabs {
        let name = TABS[*ti].1;
        let wd = (ui.painter().layout_no_wrap(name.to_string(), jf(18.0), Color32::WHITE).size().x - 2.0).clamp(60.0, 240.0).round();
        let r = Rect::from_min_size(pos2(tx, o.y + 72.0), vec2(wd, 30.0));
        let resp = ui.interact(r, ui.id().with(("tab", *ti)), Sense::click());
        let front = app.midi.win.tab == *ti;
        if front {
            p.rect_filled(r, 0.0, TAB);
        } else {
            // LookAndFeel_V3: lighter at the bottom, darker at the top.
            let mut m = egui::Mesh::default();
            let (top, bot) = (hex(0x4D4D4D), hex(0x717171));
            m.colored_vertex(r.left_top(), top);
            m.colored_vertex(r.right_top(), top);
            m.colored_vertex(r.right_bottom(), bot);
            m.colored_vertex(r.left_bottom(), bot);
            m.add_triangle(0, 1, 2);
            m.add_triangle(0, 2, 3);
            p.add(egui::Shape::mesh(m));
        }
        p.text(r.center(), Align2::CENTER_CENTER, name, jf(15.0), Color32::WHITE.gamma_multiply(if resp.hovered() { 1.0 } else { 0.8 }));
        app.midi.win.at.insert(format!("tab:{name}"), r.center());
        if resp.clicked() {
            app.midi.win.tab = *ti;
            app.midi.win.edit = None;
        }
        tx += wd + 1.0;
    }
    // The tab's content: a 1 pt outline, then the table.
    let frame = at(8.0, 102.0, W - 18.0, H - 117.0 - 30.0);
    p.rect_filled(frame, 0.0, hex(0x8E989B));
    let tr = frame.shrink(1.0);
    p.rect_filled(tr, 0.0, hex(0x1B1B1E));
    table(app, ui, tr, live && has_dev);

    // ---- the buttons along the bottom
    let by = H - 33.0;
    let tab = app.midi.win.tab;
    let has_sel = app.midi.win.sel.get(tab).copied().flatten().is_some();
    let en = has_dev && live;
    let r = at(16.0, by, 150.0, 24.0);
    app.midi.win.at.insert("LEARN".into(), r.center());
    if tip(text_button(ui, r, "learn", "LEARN", app.midi.win.learn, en, (false, false)), "Click to enter the MIDI LEARN standby mode. \nUsing a MIDI controller, you can operate selected feature.").clicked() {
        let on = !app.midi.win.learn;
        set_learn(app, on);
    }
    let r = at(176.0, by, 150.0, 24.0);
    app.midi.win.at.insert("ADD".into(), r.center());
    let add = tip(text_button(ui, r, "add", "ADD", false, en, (false, true)), "Add functions that can be selected for MIDI LEARN");
    add_menu(app, ui, &add);
    let r = at(328.0, by, 150.0, 24.0);
    app.midi.win.at.insert("DUPLICATE".into(), r.center());
    if tip(text_button(ui, r, "dup", "DUPLICATE", false, en && has_sel, (true, true)), "Duplicate the selected item").clicked() {
        duplicate_row(app);
    }
    let r = at(480.0, by, 150.0, 24.0);
    app.midi.win.at.insert("DELETE".into(), r.center());
    if tip(text_button(ui, r, "del", "DELETE", false, en && has_sel, (true, false)), "Delete the selected item").clicked() {
        app.midi.win.ask = Some(Ask::Delete);
    }
    let r = at(W - 336.0, by, 150.0, 24.0);
    app.midi.win.at.insert("IMPORT".into(), r.center());
    if tip(text_button(ui, r, "import", "IMPORT", false, en, (false, true)), "Import MIDI setting file").clicked() {
        app.midi.win.ask = Some(Ask::Import);
    }
    let r = at(W - 184.0, by, 150.0, 24.0);
    app.midi.win.at.insert("EXPORT".into(), r.center());
    if tip(text_button(ui, r, "export", "EXPORT", false, en, (true, false)), "Export MIDI setting file").clicked() {
        export(app);
    }
}

/// The ADD menu: the current tab's commands, grouped in submenus by category.
fn add_menu(app: &mut App, ui: &mut Ui, resp: &egui::Response) {
    let id = ui.id().with("add_pop");
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    let tab = tab_of(app.midi.win.tab);
    let reg = registry();
    let mut pick: Option<String> = None;
    let mut at = std::mem::take(&mut app.midi.win.at);
    menu_style(ui);
    egui::popup::popup_above_or_below_widget(ui, id, resp, egui::AboveOrBelow::Above, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(190.0);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        egui::ScrollArea::vertical().max_height(430.0).show(ui, |ui| {
            let mut cats: Vec<&'static str> = vec![];
            for f in reg.of_tab(tab) {
                if !cats.contains(&f.category) {
                    cats.push(f.category);
                }
            }
            if cats.is_empty() {
                ui.add_enabled(false, egui::Label::new(egui::RichText::new("(no functions)").font(jf(15.0)).color(TEXT)));
            }
            let open_id = id.with("open_cat");
            let open: Option<String> = ui.ctx().data(|d| d.get_temp(open_id));
            for c in cats {
                if c.is_empty() {
                    for f in reg.of_tab(tab).filter(|f| f.category.is_empty()) {
                        let r = menu_item(ui, &f.display, false);
                        at.insert(format!("add:{}", f.name), r.rect.center());
                        if r.clicked() {
                            pick = Some(f.name.clone());
                        }
                    }
                } else {
                    let r = menu_item(ui, c, true);
                    at.insert(format!("cat:{c}"), r.rect.center());
                    let is_open = open.as_deref() == Some(c);
                    if r.clicked() {
                        ui.ctx().data_mut(|d| if is_open { d.remove::<String>(open_id) } else { d.insert_temp(open_id, c.to_string()) });
                    }
                    if is_open {
                        for f in reg.of_tab(tab).filter(|f| f.category == c) {
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                let r = menu_item(ui, &f.display, false);
                                at.insert(format!("add:{}", f.name), r.rect.center());
                                if r.clicked() {
                                    pick = Some(f.name.clone());
                                }
                            });
                        }
                    }
                }
            }
        });
    });
    app.midi.win.at = at;
    if let Some(name) = pick {
        ui.memory_mut(|m| m.close_popup());
        add_row(app, &name);
    }
}

fn table(app: &mut App, ui: &mut Ui, tr: Rect, live: bool) {
    let p = ui.painter().clone();
    // The header: #242428, 1 pt separators over its lower 18 pt, text #C0C0C0 at 13.
    let head = Rect::from_min_size(tr.min, vec2(tr.width(), HEAD));
    p.rect_filled(head, 0.0, hex(0x242428));
    let mut x = tr.min.x;
    let mut xs = [0.0f32; 7];
    let mut ws = app.midi.win.widths.unwrap_or(COLS.map(|c| c.1));
    for (i, (name, _)) in COLS.iter().enumerate() {
        let w = &ws[i];
        xs[i] = x;
        let hr = Rect::from_min_size(pos2(x, head.min.y), vec2(*w, HEAD));
        let resp = ui.interact(hr, ui.id().with(("head", i)), if live { Sense::click() } else { Sense::hover() });
        p.rect_filled(Rect::from_min_size(pos2(x + w - 1.0, head.max.y - 18.0), vec2(1.0, 18.0)), 0.0, hex(0x404042));
        p.with_clip_rect(hr).text(pos2(x + 4.0, head.center().y), Align2::LEFT_CENTER, *name, jf(13.0), hex(0xC0C0C0));
        if let Some((c, asc)) = app.midi.win.sort {
            if c == i {
                let c0 = pos2(x + w - 12.0, head.center().y);
                let d = if asc { -1.0 } else { 1.0 };
                p.add(egui::Shape::convex_polygon(vec![c0 + vec2(-4.0, -2.5 * d), c0 + vec2(4.0, -2.5 * d), c0 + vec2(0.0, 2.5 * d)], hex(0xC0C0C0), Stroke::NONE));
            }
        }
        if resp.clicked() {
            sort_rows(&mut app.midi.win, i);
        }
        // Each column's right edge is a resize handle (widths 50..400, as rekordbox sets them).
        let edge = Rect::from_center_size(pos2(x + w - 0.5, head.center().y), vec2(6.0, HEAD));
        let er = ui.interact(edge, ui.id().with(("headsize", i)), if live { Sense::drag() } else { Sense::hover() });
        if er.hovered() || er.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let mut grown = 0.0;
        if er.dragged() {
            grown = er.drag_delta().x;
        }
        x += w;
        if grown != 0.0 {
            ws[i] = (ws[i] + grown).clamp(50.0, 400.0);
            app.midi.win.widths = Some(ws);
        }
    }
    xs[6] = x;
    let tab = app.midi.win.tab;
    let n = app.midi.win.rows.get(tab).map(|r| r.len()).unwrap_or(0);
    let body = Rect::from_min_max(pos2(tr.min.x, head.max.y), tr.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt(("midi_table", tab)));
    child.set_clip_rect(body);
    {
        let s = child.style_mut();
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = false;
        s.visuals.extreme_bg_color = hex(0x1B1B1B);
        s.visuals.widgets.inactive.bg_fill = hex(0x878787);
        s.visuals.widgets.hovered.bg_fill = hex(0x878787);
        s.visuals.widgets.active.bg_fill = hex(0x878787);
    }
    let mut actions: Vec<(usize, RowAct)> = vec![];
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(&mut child, ROW, n, |ui, range| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for r in range {
            let (rr, resp) = ui.allocate_exact_size(vec2(body.width() - 10.0, ROW), if live { Sense::click() } else { Sense::hover() });
            let row = app.midi.win.rows[tab][r].clone();
            let selected = app.midi.win.sel[tab] == Some(r);
            let p = ui.painter().clone();
            p.rect_filled(rr, 0.0, if selected { hex(0x352A79) } else if r % 2 == 1 { hex(0x131314) } else { hex(0x0A0A0B) });
            if resp.clicked() {
                actions.push((r, RowAct::Select));
            }
            if r < 12 {
                app.midi.win.at.insert(format!("row:{r}"), pos2(xs[0] + 60.0, rr.center().y));
            }
            for (c, w) in ws.iter().enumerate() {
                let cell = Rect::from_min_size(pos2(xs[c], rr.min.y), vec2(*w, ROW));
                // The column separator: white at 20 %.
                p.rect_filled(Rect::from_min_size(pos2(cell.max.x - 1.0, cell.min.y), vec2(1.0, ROW)), 0.0, Color32::WHITE.gamma_multiply(0.2));
                let editing = app.midi.win.edit.as_ref().map(|e| e.0 == r && e.1 == c).unwrap_or(false);
                let text = match c {
                    0 => row.display.clone(),
                    1 => if row.deck == 0 { String::new() } else { row.deck.to_string() },
                    2 => row.input.map(|v| format!("{v:04X}")).unwrap_or_default(),
                    3 => type_text(row.ty).to_string(),
                    4 => row.output.map(|v| format!("{v:04X}")).unwrap_or_default(),
                    _ => row.comment.clone(),
                };
                match c {
                    1 | 3 => {
                        // A list in the cell: decks 1 to 4, or the types of the row's family.
                        let per_deck = row.deck != 0;
                        let choices: Vec<String> = if c == 1 { if per_deck { (1..=4).map(|d| d.to_string()).collect() } else { vec![] } } else { type_choices(row.ty).iter().map(|t| type_text(*t).to_string()).collect() };
                        let en = live && !choices.is_empty();
                        let cresp = combo_box(ui, cell.shrink2(vec2(1.0, 1.0)), ("cell", r, c), &text, false, en);
                        if cresp.clicked() {
                            actions.push((r, RowAct::Select));
                        }
                        let mut none = HashMap::new();
                        if let Some(i) = menu_popup(ui, &cresp, ui.id().with(("cellpop", r, c)), *w - 2.0, &choices, &mut none) {
                            actions.push((r, if c == 1 { RowAct::Deck(i as u8 + 1) } else { RowAct::Type(type_choices(row.ty)[i]) }));
                        }
                    }
                    2 | 4 | 5 if editing => {
                        let mut t = app.midi.win.edit.as_ref().map(|e| e.2.clone()).unwrap_or_default();
                        let te = egui::TextEdit::singleline(&mut t).font(jf(18.0)).text_color(Color32::WHITE).frame(false).margin(egui::Margin::symmetric(2, 0)).char_limit(if c == 5 { 120 } else { 4 });
                        let er = ui.put(cell.shrink2(vec2(1.0, 0.0)), te);
                        if !er.has_focus() && !er.lost_focus() {
                            er.request_focus();
                        }
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if esc {
                            actions.push((r, RowAct::EndEdit(None)));
                        } else if er.lost_focus() || enter {
                            actions.push((r, RowAct::EndEdit(Some((c, t.clone())))));
                        } else if let Some(e) = app.midi.win.edit.as_mut() {
                            e.2 = t;
                        }
                    }
                    _ => {
                        p.with_clip_rect(cell.shrink2(vec2(1.0, 0.0))).text(pos2(cell.min.x + 2.0, cell.center().y), Align2::LEFT_CENTER, &text, jf(18.0), Color32::WHITE);
                        if matches!(c, 2 | 4 | 5) && live {
                            let cr = ui.interact(cell, ui.id().with(("cellhit", r, c)), Sense::click());
                            if cr.clicked() {
                                actions.push((r, RowAct::Select));
                                if selected {
                                    actions.push((r, RowAct::Edit(c, text.clone())));
                                }
                            }
                            if cr.double_clicked() {
                                actions.push((r, RowAct::Edit(c, text.clone())));
                            }
                        }
                    }
                }
            }
        }
    });
    for (r, a) in actions {
        match a {
            RowAct::Select => {
                if app.midi.win.sel[tab] != Some(r) {
                    app.midi.win.edit = None;
                }
                app.midi.win.sel[tab] = Some(r);
            }
            RowAct::Deck(d) => {
                app.midi.win.rows[tab][r].deck = d;
                commit_row(app, r);
            }
            RowAct::Type(t) => {
                app.midi.win.rows[tab][r].ty = t;
                commit_row(app, r);
            }
            RowAct::Edit(c, text) => app.midi.win.edit = Some((r, c, text)),
            RowAct::EndEdit(done) => {
                app.midi.win.edit = None;
                if let Some((c, text)) = done {
                    end_edit(app, r, c, &text);
                }
            }
        }
    }
}

enum RowAct {
    Select,
    Deck(u8),
    Type(u8),
    Edit(usize, String),
    EndEdit(Option<(usize, String)>),
}

/// A typed cell is committed: MIDI IN / MIDI OUT as four hex digits (through the same rules
/// as LEARN), or the comment.
fn end_edit(app: &mut App, r: usize, c: usize, text: &str) {
    let tab = app.midi.win.tab;
    if c == 5 {
        // (A comma would end the field in the file.)
        app.midi.win.rows[tab][r].comment = text.replace(',', " ");
        commit_row(app, r);
        return;
    }
    let t = text.trim();
    let code = if t.is_empty() { Some(None) } else { u16::from_str_radix(t, 16).ok().filter(|v| *v >= 0x8000).map(Some) };
    let Some(code) = code else { return };
    app.midi.win.clash = (None, None);
    let ok = if c == 2 { commit_in(app, r, code) } else { commit_out(app, r, code) };
    if ok {
        save(app);
    }
    show_clash(app);
}

/// The alerts and questions of the window.
fn dialogs(app: &mut App, ctx: &egui::Context) {
    use crate::ui_kit as kit;
    let center = Align2::CENTER_CENTER;
    if let Some((title, msg)) = app.midi.win.alert.clone() {
        let mut done = false;
        kit::dialog(ctx, if title.is_empty() { "recordbox".to_string() } else { title }).order(egui::Order::Tooltip).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
            kit::apply_style(ui);
            ui.set_width(380.0);
            kit::message(ui, &msg);
            let b = kit::dialog_buttons(ui, &["OK"]);
            app.midi.win.at.insert("alert:OK".into(), pos2(ui.min_rect().center().x, ui.min_rect().max.y - 12.0));
            done = b.is_some();
        });
        if done {
            app.midi.win.alert = None;
        }
        return;
    }
    let Some(ask) = app.midi.win.ask.clone() else { return };
    let (title, msg, buttons): (&str, &str, &[&str]) = match ask {
        Ask::Delete => ("DELETE", "Are you sure you really want to delete the selected MIDI setting item?", &["OK", "Cancel"]),
        Ask::Default => ("Reset MIDI setting back to default", "The MIDI setting will be reset back to default.\nDo you want to export (save) the file you are currently editing?", &["Yes", "No", "Cancel"]),
        Ask::Import => ("MIDI setting file Import", "The MIDI setting file will be imported.\nDo you want to save the file you are currently editing?", &["Yes", "No", "Cancel"]),
    };
    let mut answer = None;
    kit::dialog(ctx, title).order(egui::Order::Tooltip).resizable(false).anchor(center, vec2(0.0, 0.0)).show(ctx, |ui| {
        kit::apply_style(ui);
        ui.set_width(420.0);
        kit::message(ui, msg);
        answer = kit::dialog_buttons(ui, buttons);
        // (Where the buttons are, for the scripted check: 96 pt wide, 8 pt apart, centred.)
        let n = buttons.len() as f32;
        let x0 = ui.min_rect().center().x - (n * 96.0 + (n - 1.0) * 8.0) / 2.0;
        for (i, b) in buttons.iter().enumerate() {
            app.midi.win.at.insert(format!("ask:{b}"), pos2(x0 + 48.0 + i as f32 * 104.0, ui.min_rect().max.y - 12.0));
        }
    });
    let Some(a) = answer else { return };
    app.midi.win.ask = None;
    match (ask, a) {
        (Ask::Delete, 0) => delete_row(app),
        (Ask::Default, 0) | (Ask::Default, 1) => {
            // Yes: export first (and stop if that was given up or failed); No: go on.
            if a == 1 || export(app) == Some(true) {
                reset_default(app);
            }
        }
        (Ask::Import, 0) | (Ask::Import, 1) => {
            if a == 1 || export_for_import(app) {
                import(app);
            }
        }
        _ => {}
    }
}

/// "Save the file you are currently editing" before an import. (The scripted check names
/// one file for both panels, so the save is passed over there.)
fn export_for_import(app: &mut App) -> bool {
    if std::env::var("PREPDECK_MIDI_FILE").is_ok() {
        return true;
    }
    export(app) == Some(true)
}

// ---------------------------------------------------------------------- preferences

/// Whether Preferences > Controller > Mixer shows the controller rows: with a controller
/// connected (rekordbox shows TakeOver with a DDJ-REV5 or DDJ-REV7; in recordbox the pick-up
/// works for any controller whose EQ knobs are mapped).
pub fn prefs_rows_shown(app: &App) -> bool {
    app.midi.connected() || std::env::var("PREPDECK_SHOT_MIDI").map(|v| v.contains("prefs")).unwrap_or(false)
}

/// The rows of Preferences that belong to controllers: rekordbox's TakeOver card (Controller
/// > Mixer), and of its Deck tab the Needle Lock card and the JOG card's rotation speed.
pub fn prefs_rows(app: &mut App, ui: &mut Ui) {
    use crate::ui_kit::{card, radio, sub, toggle_raw};
    let mut changed = false;
    card(ui, "TakeOver", |ui| {
        sub(ui, "Takeover when switching to EQ/STEM ISO");
        crate::ui_kit::combo_row(ui, 22.0, |ui| {
            ui.add_space(36.0);
            changed |= toggle_raw(ui, &mut app.prefs.midi.takeover_eq_iso, "Enable");
        });
    });
    card(ui, "Needle Lock", |ui| {
        if radio(ui, app.prefs.midi.needle_lock, "Lock") {
            app.prefs.midi.needle_lock = true;
            changed = true;
        }
        if radio(ui, !app.prefs.midi.needle_lock, "Unlock") {
            app.prefs.midi.needle_lock = false;
            changed = true;
        }
    });
    card(ui, "JOG", |ui| {
        sub(ui, "JOG Rotation Speed");
        if radio(ui, !app.prefs.midi.jog_45rpm, "33rpm") {
            app.prefs.midi.jog_45rpm = false;
            changed = true;
        }
        if radio(ui, app.prefs.midi.jog_45rpm, "45rpm") {
            app.prefs.midi.jog_45rpm = true;
            changed = true;
        }
    });
    if changed {
        app.save_prefs();
    }
}

// ---------------------------------------------------------------------- scripted check

/// A controller made inside the app for the scripted check and the screenshots: a CoreMIDI
/// source that sends what the script asks and a destination that collects its LEDs.
struct TestCtl {
    out: midir::MidiOutputConnection,
    _inp: midir::MidiInputConnection<()>,
    leds: std::sync::Arc<parking_lot::Mutex<Vec<[u8; 3]>>>,
}

pub const TEST_NAME: &str = "recordbox test controller";

static TEST: parking_lot::Mutex<Option<TestCtl>> = parking_lot::Mutex::new(None);

#[cfg(not(unix))]
fn test_ctl_start() -> bool { false }

#[cfg(unix)]
fn test_ctl_start() -> bool {
    use midir::os::unix::{VirtualInput, VirtualOutput};
    let mut g = TEST.lock();
    if g.is_some() {
        return true;
    }
    let Some(out) = midir::MidiOutput::new("recordbox-test").ok().and_then(|o| o.create_virtual(TEST_NAME).ok()) else { return false };
    let leds = std::sync::Arc::new(parking_lot::Mutex::new(vec![]));
    let l2 = leds.clone();
    let Some(inp) = midir::MidiInput::new("recordbox-test").ok().and_then(|i| {
        i.create_virtual(TEST_NAME, move |_, m, _| {
            if m.len() == 3 {
                l2.lock().push([m[0], m[1], m[2]]);
            }
        }, ()).ok()
    }) else { return false };
    *g = Some(TestCtl { out, _inp: inp, leds });
    true
}

fn test_send(msg: [u8; 3]) {
    if let Some(t) = TEST.lock().as_mut() {
        let _ = t.out.send(&msg);
    }
}

fn test_leds() -> Vec<[u8; 3]> {
    TEST.lock().as_ref().map(|t| std::mem::take(&mut *t.leds.lock())).unwrap_or_default()
}

fn hexs(c: Option<u16>) -> String {
    c.map(|c| format!("{c:04X}")).unwrap_or_else(|| "----".into())
}

impl App {
    fn midi_state_line(&self) -> String {
        let w = &self.midi.win;
        let rows: Vec<String> = w.rows.get(w.tab).map(|v| v.iter().map(|r| format!("{}[d{} in {} out {} {}]", r.display, r.deck, hexs(r.input), hexs(r.output), type_text(r.ty))).collect()).unwrap_or_default();
        format!("open {} device {:?} tab {} learn {} sel {:?} rows {} {:?} alert {:?} ask {:?}", w.open, w.device, TABS[w.tab.min(9)].1, w.learn, w.sel.get(w.tab).copied().flatten(), rows.len(), rows, w.alert.as_ref().map(|a| a.1.replace('\n', " | ")), w.ask)
    }

    /// PREPDECK_SHOT_MIDI=window|learn|add|alert|empty (with --screenshot,
    /// PREPDECK_SHOT_VIEW=performance): the MIDI setting window in that state, with a test
    /// controller. PREPDECK_SHOT_PLAYER=miditest (with PREPDECK_BENCH=<frames> to keep the
    /// window up): the scripted pointer run, which prints what each step did.
    pub fn shot_midi_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        // `PREPDECK_SHOT_PLAYER=midiprobe PREPDECK_MIDI_PROBE=<port>`: the read-only
        // hardware probe (midi.rs `probe`), started once (PREPDECK_MIDI_PROBE_SECS, default 8).
        if frame == 150 && std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "midiprobe")).unwrap_or(false) {
            if let Ok(port) = std::env::var("PREPDECK_MIDI_PROBE") {
                let secs: f64 = std::env::var("PREPDECK_MIDI_PROBE_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(8.0);
                std::thread::spawn(move || crate::midi::probe(&port, secs));
            }
        }
        let shot = std::env::var("PREPDECK_SHOT_MIDI").unwrap_or_default();
        let test = std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "miditest")).unwrap_or(false);
        if shot.is_empty() && !test {
            return;
        }
        if frame == 2 {
            if !test_ctl_start() {
                eprintln!("miditest: CoreMIDI virtual ports are not available");
            }
            if !shot.is_empty() {
                // A mapping to show: a few learned rows in the throwaway data folder.
                let dir = crate::db::data_dir().join("MidiMappings");
                let _ = std::fs::create_dir_all(&dir);
                let demo = format!("@file,1,{TEST_NAME}\r\nPlayPause,PlayPause,Button,,900B,910B,,,,900B,910B,,,Fast;Blink=1000;Priority=50;,\r\nCue,Cue,Button,,900C,910C,,,,900C,910C,,,Fast;Blink=500;Priority=50;,\r\nSync,Sync,Button,,9058,9158,,,,9058,9158,,,Blink=600;,\r\nTempoSlider,TempoSlider,KnobSliderHiRes,,B000,B100,,,,,,,,Fast;,pitch fader\r\nLoopIn,LoopIn,Button,,9010,9110,,,,9010,9110,,,Fast;,\r\nLoopOut,LoopOut,Button,,9011,9111,,,,9011,9111,,,Fast;,\r\nNeedleSearch,NeedleSearch,Value,,B003,,,,,,,,,Max=16383;,\r\nTempoUp,TempoUp,Indicator,,,,,,,9034,9134,,,,\r\n");
                let _ = std::fs::write(dir.join(format!("{TEST_NAME}.midi.csv")), demo);
            }
        }
        let click = |raw: &mut egui::RawInput, p: Pos2, down: bool| raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: down, modifiers: Default::default() });
        if !shot.is_empty() {
            if !self.midi.connected() {
                return;
            }
            if !self.midi.win.open && frame > 10 && shot != "prefs" {
                self.midi.win.open = true;
                if shot != "empty" {
                    select_device(self, TEST_NAME);
                    self.midi.win.sel[0] = Some(1);
                }
                if shot == "learn" {
                    set_learn(self, true);
                }
                if shot == "alert" {
                    self.midi.win.alert = Some((String::new(), "This MIDI code is already assigned.\n\nDECK: \nPlayPause\n\nMIDI IN: 900B\nMIDI OUT: 900B".into()));
                }
            }
            if shot == "add" {
                if let Some(p) = self.midi.win.at.get("ADD").copied() {
                    match frame {
                        100 => raw.events.push(egui::Event::PointerMoved(p)),
                        101 => click(raw, p, true),
                        102 => click(raw, p, false),
                        _ => {}
                    }
                }
                if let Some(p) = self.midi.win.at.get("cat:Loop").copied() {
                    match frame {
                        106 => raw.events.push(egui::Event::PointerMoved(p)),
                        107 => click(raw, p, true),
                        108 => click(raw, p, false),
                        _ => {}
                    }
                }
            }
            return;
        }
        // ---- the scripted run: a step every 8 frames from frame 150 (the test controller
        // is found by the port scan within a second or two of the start).
        const START: u32 = 150;
        if frame < START {
            return;
        }
        let step = ((frame - START) / 8) as usize;
        let phase = (frame - START) % 8;
        let file = std::env::var("PREPDECK_MIDI_FILE").unwrap_or_default();
        enum S {
            Click(&'static str),
            Send([u8; 3]),
            Note(&'static str),
        }
        let steps: Vec<(&str, S)> = vec![
            ("the MIDI button opens the window", S::Click("MIDI")),
            ("the device list opens", S::Click("device")),
            ("the test controller is picked", S::Click("item:recordbox test controller")),
            ("ADD opens its menu", S::Click("ADD")),
            ("PlayPause is added", S::Click("add:PlayPause")),
            ("LEARN is armed", S::Click("LEARN")),
            ("the controller sends note 0B: learned as MIDI IN and MIDI OUT", S::Send([0x90, 0x0B, 0x7F])),
            ("its release is passed over", S::Send([0x90, 0x0B, 0x00])),
            ("ADD again", S::Click("ADD")),
            ("Cue is added (and selected)", S::Click("add:Cue")),
            ("the same note for Cue: refused", S::Send([0x90, 0x0B, 0x7F])),
            ("the alert is closed", S::Click("alert:OK")),
            ("note 0C for Cue: learned", S::Send([0x90, 0x0C, 0x7F])),
            ("release", S::Send([0x90, 0x0C, 0x00])),
            ("LEARN is switched off", S::Click("LEARN")),
            ("the controller's PLAY now plays deck 1", S::Send([0x90, 0x0B, 0x7F])),
            ("release", S::Send([0x90, 0x0B, 0x00])),
            ("the lights that came back", S::Note("leds")),
            ("PLAY again pauses", S::Send([0x90, 0x0B, 0x7F])),
            ("release", S::Send([0x90, 0x0B, 0x00])),
            ("EXPORT writes the file", S::Click("EXPORT")),
            ("the exported file", S::Note("file")),
            ("the first row is selected", S::Click("row:0")),
            ("DELETE asks", S::Click("DELETE")),
            ("OK: the row goes", S::Click("ask:OK")),
            ("DUPLICATE needs a selected row", S::Click("row:0")),
            ("DUPLICATE copies it without codes", S::Click("DUPLICATE")),
            ("IMPORT asks", S::Click("IMPORT")),
            ("No: the exported file is read back", S::Click("ask:No")),
            ("the MIXER tab", S::Click("tab:MIXER")),
            ("back to DECK", S::Click("tab:DECK")),
            ("DEFAULT asks", S::Click("DEFAULT")),
            ("Cancel: nothing changes", S::Click("ask:Cancel")),
            ("the window closes", S::Click("close")),
        ];
        let Some((name, s)) = steps.get(step) else {
            if step == steps.len() && phase == 0 {
                eprintln!("miditest: done; {} mapped messages handled, {} codes learned", self.midi.handled, self.midi.win.learned);
            }
            return;
        };
        match s {
            S::Click(what) => {
                let Some(p) = self.midi.win.at.get(*what).copied() else {
                    if phase == 6 {
                        eprintln!("miditest: {name}: NOT FOUND on screen: {what}");
                    }
                    return;
                };
                match phase {
                    0 => raw.events.push(egui::Event::PointerMoved(p)),
                    1 => click(raw, p, true),
                    2 => click(raw, p, false),
                    6 => eprintln!("miditest: {name} -> {}", self.midi_state_line()),
                    _ => {}
                }
            }
            S::Send(m) => {
                if phase == 0 {
                    test_send(*m);
                }
                if phase == 6 {
                    let playing = self.player.decks[0].lock().playing;
                    eprintln!("miditest: {name} [{:02X} {:02X} {:02X}] -> deck 1 playing {playing}; {}", m[0], m[1], m[2], self.midi_state_line());
                }
            }
            S::Note(kind) => {
                if phase == 6 {
                    if *kind == "leds" {
                        let l: Vec<String> = test_leds().iter().map(|m| format!("{:02X}{:02X}={}", m[0], m[1], m[2])).collect();
                        eprintln!("miditest: {name}: {}", l.join(" "));
                    } else {
                        let text = std::fs::read_to_string(&file).unwrap_or_else(|e| format!("<{e}>"));
                        eprintln!("miditest: {name} {file}: {:?}", text);
                    }
                }
            }
        }
        let _ = NO_FUNCTION;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi_csv::{Entry, Kind, Opts};

    fn e(name: &str, code: u16, ro: bool) -> Entry {
        Entry { name: name.into(), function: name.into(), kind: Kind::Button, deck: 1, code, opts: Opts { ro, ..Default::default() }, comment: String::new(), user: false }
    }

    #[test]
    fn refusal_texts_follow_rekordbox() {
        // Another command of the window has the code: its tab and name are shown.
        assert_eq!(clash_alert(&Some(e("PlayPause", 0x900B, false)), &None).unwrap(), "This MIDI code is already assigned.\n\nDECK: \nPlayPause\n\nMIDI IN: 900B");
        assert_eq!(clash_alert(&None, &Some(e("TrackNext", 0x9010, false))).unwrap(), "This MIDI code is already assigned.\n\nDECK: \nTrackSearch Next\n\nMIDI OUT: 9010");
        // The same command on both sides: once, with both codes.
        assert_eq!(clash_alert(&Some(e("Cue", 0x900C, false)), &Some(e("Cue", 0x900C, false))).unwrap(), "This MIDI code is already assigned.\n\nDECK: \nCue\n\nMIDI IN: 900C\nMIDI OUT: 900C");
        // Two different ones: each with its code.
        assert_eq!(clash_alert(&Some(e("Cue", 0x900C, false)), &Some(e("Gain", 0xB004, false))).unwrap(), "This MIDI code is already assigned.\n\nDECK: \nCue\n\nMIDI IN: 900C\n\nMIXER: \nGain\n\nMIDI OUT: B004");
        // A read-only entry, or a function the window does not offer: it cannot be changed.
        assert_eq!(clash_alert(&Some(e("JogTouch", 0x9036, true)), &None).unwrap(), "You cannot change the assignment for this MIDI code.\n\nMIDI IN: 9036");
        assert_eq!(clash_alert(&Some(e("LoadedIndicator", 0x9060, false)), &None).unwrap(), "You cannot change the assignment for this MIDI code.\n\nMIDI IN: 9060");
        assert_eq!(clash_alert(&None, &None), None);
    }

    #[test]
    fn a_button_for_pad_row_keeps_led_values() {
        let mut r = LearnRow { name: "PlayPause".into(), display: "PlayPause".into(), deck: 1, input: None, output: None, ty: T_BUTTON, comment: String::new(), min: -1, max: -1 };
        assert_eq!((r.entry().min, r.entry().max, r.entry().kind), (-1, -1, Kind::Button));
        r.ty = T_BUTTON_PAD;
        assert_eq!((r.entry().min, r.entry().max, r.entry().kind), (0, 127, Kind::Button));
        r.ty = T_KNOB14;
        assert_eq!(r.entry().kind, Kind::KnobSliderHiRes);
        // The six columns rekordbox shows, at its widths.
        assert_eq!(COLS.iter().map(|c| c.1).sum::<f32>(), 850.0);
        assert_eq!(shown_tabs().len(), 9);
    }
}
