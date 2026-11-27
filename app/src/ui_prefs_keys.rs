//! Preferences: the sidebar's category icons, and the Keyboard page.

use crate::prefs_rb::{self as rb, Mode};
use crate::theme::*;
use eframe::egui;
use egui::{vec2, Rect, Ui};

/// rekordbox's file name part for a category's icon (`preferece_<name>_{on,off}.svg`, sic).
fn icon_name(cat: u8) -> &'static str {
    match cat {
        rb::PLAN => "plan",
        rb::CLOUD => "cloud",
        rb::VIEW => "view",
        rb::AUDIO => "audio",
        rb::ANALYSIS => "anlysys",
        rb::DJ_SYSTEM => "djsystem",
        rb::CONTROLLER => "controller",
        rb::KEYBOARD => "keyboard",
        rb::DVS => "dvs",
        rb::EXTENSIONS => "extensions",
        _ => "advanced",
    }
}

/// A category's 30 pt icon at `o`: rekordbox's own SVG, read from the installed rekordbox
/// (the `_on` file for the selected category), or recordbox's drawing of the same shapes
/// when rekordbox is not installed. `dim`: a category recordbox does not have.
pub fn side_icon(ui: &Ui, o: egui::Pos2, cat: u8, selected: bool, dim: bool) {
    let file = format!("preferece_{}_{}.svg", icon_name(cat), if selected { "on" } else { "off" });
    if let Some((tex, size)) = crate::prefs_font::skin(ui.ctx(), &file) {
        // (The files are #AAAAAA or white already; a dimmed category is drawn at #646464.)
        let tint = if dim { gy(0x969696) } else { WHITE.c() };
        crate::prefs_font::paint_skin(ui.painter(), &tex, Rect::from_min_size(o, size), tint);
        return;
    }
    let (c, bg) = if selected { (WHITE.c(), crate::ui_kit::PBLUE) } else if dim { (OFF.c(), BG2.c()) } else { (DIM2.c(), BG2.c()) };
    crate::ui_prefs::side_icon_drawn(ui.painter(), o, cat, c, bg);
    let _ = vec2(0.0, 0.0);
}

// ------------------------------------------------------------------ Keyboard

use crate::app::{App, Prefs, Shortcut};
use crate::keycmd_data::{Cmd, CATS, CMDS, EXPORT_KEYS, EXPORT_NAMES};
use std::collections::BTreeMap;

/// rekordbox's presets as `PresetKeyMapping::getPresetList` (0x101bca260) offers them: id and
/// name. (A "Traktor" preset, id 0000000000020, is named in the binary but not in this list.)
pub const PRESETS: [(&str, &str); 4] = [("0000000000000", "Performance 1 (Preset)"), ("0000000000010", "Performance 2 (Preset)"), ("0000000000030", "Export (Preset)"), ("9999999999999", "None (Preset)")];
pub const EDIT_PRESET: (&str, &str) = ("0000000000000", "Edit Mode (Preset)");
/// The mapping recordbox had before rekordbox's presets were added: its own keys, editable.
pub const OWN: &str = "recordbox";

/// One command as the editor lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u16,
    pub name: String,
    pub cat: String,
    /// Index into `CMDS` and which unit (deck, channel, FX unit) of it this is.
    cmd: usize,
    pub unit: u8,
}

/// The order rekordbox's editor shows the categories in (`getCommandCategories` 0x10241532c).
const CAT_ORDER: [&str; 26] = ["Browse", "Player A", "Deck 1", "Player B", "Deck 2", "Link Export", "Deck 3", "Deck 4", "All Decks ", "Sampler", "Mixer", "Recordings", "FX1", "FX2", "FX Common", "MIX POINT LINK", "Video", "General", "File", "View", "Track", "Playlist", "Help", "Deck", "Action pad", "Tool area"];

/// The commands of a mode, in the editor's order.
pub fn commands(mode: Mode) -> Vec<Row> {
    let bit = match mode {
        Mode::Export | Mode::Lighting => 1,
        Mode::Performance => 2,
        Mode::Edit => 8,
    };
    let mut v = vec![];
    for (ci, c) in CMDS.iter().enumerate() {
        let cat = CATS[c.cat as usize];
        if mode == Mode::Edit {
            // (Edit mode replaces the list by its own three categories, with the project
            // commands of File and General's "Browser", which exist in EDIT only.)
            if c.scope == 0 && (matches!(cat, "Deck" | "Action pad" | "Tool area") || matches!(c.id, 0x2013..=0x2017 | 0xcc1b)) {
                v.push(Row { id: c.id, name: c.name.to_string(), cat: cat.to_string(), cmd: ci, unit: 0 });
            }
            continue;
        }
        if c.modes & bit == 0 || matches!(cat, "Action pad" | "Tool area" | "Video") || (c.scope == 0 && cat == "Deck") || (cat == "Link Export" && mode == Mode::Performance) {
            continue;
        }
        match c.scope {
            1 => {
                let units = if mode == Mode::Performance { 4 } else { 2 };
                for u in 0..units {
                    let cat = if mode == Mode::Performance { format!("Deck {}", u + 1) } else { format!("Player {}", ["A", "B"][u as usize]) };
                    let name = if mode == Mode::Performance { c.name } else { EXPORT_NAMES.iter().find(|e| e.0 == c.id).map(|e| e.1).unwrap_or(c.name) };
                    v.push(Row { id: c.id + 0x100 * u as u16, name: name.to_string(), cat, cmd: ci, unit: u });
                }
            }
            2 => {
                for u in 0..4u8 {
                    v.push(Row { id: c.id + 0x100 * u as u16, name: format!("CH{} {}", u + 1, c.name), cat: "Mixer".into(), cmd: ci, unit: u });
                }
            }
            3 => {
                for u in 0..2u8 {
                    v.push(Row { id: c.id + 0x100 * u as u16, name: c.name.to_string(), cat: format!("FX{}", u + 1), cmd: ci, unit: u });
                }
            }
            _ => v.push(Row { id: c.id, name: c.name.to_string(), cat: cat.to_string(), cmd: ci, unit: 0 }),
        }
    }
    let rank = |c: &str| CAT_ORDER.iter().position(|x| *x == c).unwrap_or(CAT_ORDER.len());
    v.sort_by(|a, b| rank(&a.cat).cmp(&rank(&b.cat)).then(a.unit.cmp(&b.unit)).then(a.id.cmp(&b.id)));
    v
}

/// The key a preset gives a command, as rekordbox writes keys ("shift + command + 1").
pub fn preset_key(row: &Row, preset: &str) -> String {
    let c: &Cmd = &CMDS[row.cmd];
    let unit = |k: &[&'static str; 2]| -> String {
        match (c.scope, row.unit) {
            (1, 0) | (0, _) | (2, 0) | (3, 0) => k[0].to_string(),
            // Deck 2 has its own keys, deck 4 the same again, deck 3 none.
            (1, 1) | (1, 3) => k[1].to_string(),
            _ => String::new(),
        }
    };
    match preset {
        "0000000000000" => unit(&c.p1),
        "0000000000010" => unit(&c.p2),
        "0000000000030" => EXPORT_KEYS.iter().find(|e| e.0 == row.id).map(|e| e.1.to_string()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The recordbox action a rekordbox command carries out, where recordbox has one. Commands
/// without one are listed and can be given keys, and do nothing.
pub fn action_of(id: u16, name: &str) -> Option<&'static str> {
    const MAP: [(u16, &str); 44] = [
        (0x3006, "play"), (0x3007, "cue"), (0x3008, "jump_fwd"), (0x3009, "jump_back"), (0x300a, "loop_in"), (0x300b, "loop_out"), (0x300c, "reloop"), (0x3014, "auto_loop"),
        (0x3018, "loop_halve"), (0x3019, "loop_double"), (0x301a, "next_track"), (0x301b, "prev_track"), (0x301c, "quantize"), (0x301d, "time_mode"),
        (0x301e, "hot1"), (0x301f, "hot2"), (0x3020, "hot3"), (0x3054, "hot4"), (0x307f, "hot5"), (0x3080, "hot6"), (0x3081, "hot7"), (0x3082, "hot8"),
        (0x3021, "clear_hot1"), (0x3022, "clear_hot2"), (0x3023, "clear_hot3"), (0x3055, "clear_hot4"), (0x308b, "clear_hot5"), (0x308c, "clear_hot6"), (0x308d, "clear_hot7"), (0x308e, "clear_hot8"),
        (0x3024, "memory"), (0x3039, "mem_next"), (0x303a, "mem_prev"), (0x303b, "mem_delete"), (0x3043, "grid_right"), (0x3044, "grid_left"), (0x304a, "grid_tap"), (0x30c4, "grid_downbeat"),
        (0x3000, "zoom_in"), (0x3001, "zoom_out"), (0x3002, "zoom_reset"), (0x2000, "import_file"), (0x200a, "prefs"), (0xb04e, "fullscreen"),
    ];
    if let Some(m) = MAP.iter().find(|m| m.0 == id) {
        return Some(m.1);
    }
    match id {
        0x3025..=0x302e => Some(["mem_call1", "mem_call2", "mem_call3", "mem_call4", "mem_call5", "mem_call6", "mem_call7", "mem_call8", "mem_call9", "mem_call10"][(id - 0x3025) as usize]),
        0x302f..=0x3038 => Some(["mem_delete1", "mem_delete2", "mem_delete3", "mem_delete4", "mem_delete5", "mem_delete6", "mem_delete7", "mem_delete8", "mem_delete9", "mem_delete10"][(id - 0x302f) as usize]),
        0xd0f0..=0xd0f4 => Some(["rec_toggle", "rec_level_up", "rec_level_down", "rec_start", "rec_pause"][(id - 0xd0f0) as usize]),
        0x3003 => Some("vol_up"),
        0x3004 => Some("vol_down"),
        // The sampler's commands carry rekordbox's names in recordbox's action table.
        0xf000..=0xf0ff => crate::prefs_ext::ACTIONS.iter().find(|a| a.3 == "Sampler" && a.1 == name).map(|a| a.0),
        _ => None,
    }
}

/// rekordbox's key text ("shift + command + cursor left") in recordbox's stored form
/// ("Shift+Cmd+ArrowLeft"); None for a key recordbox cannot take.
pub fn juce_to_stored(text: &str) -> Option<String> {
    let mut sc = Shortcut { key: egui::Key::Space, ctrl: false, alt: false, shift: false, cmd: false };
    let mut key = None;
    for part in text.split(" + ") {
        match part.trim() {
            "ctrl" => sc.ctrl = true,
            "option" | "alt" => sc.alt = true,
            "shift" => sc.shift = true,
            "command" => sc.cmd = true,
            k => key = Some(k.to_string()),
        }
    }
    let k = key?;
    let name = match k.as_str() {
        "spacebar" => "Space".to_string(),
        "cursor left" => "ArrowLeft".into(),
        "cursor right" => "ArrowRight".into(),
        "cursor up" => "ArrowUp".into(),
        "cursor down" => "ArrowDown".into(),
        "return" => "Enter".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "tab" => "Tab".into(),
        "escape" => "Escape".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "page up" => "PageUp".into(),
        "page down" => "PageDown".into(),
        // (The key rekordbox names "¥" on a JIS layout is the backslash key; "<" and ">" are
        // the comma and full stop keys with Shift.)
        "¥" => "Backslash".into(),
        "<" => "Comma".into(),
        ">" => "Period".into(),
        other => other.to_string(),
    };
    sc.key = egui::Key::from_name(&name)?;
    Some(sc.name())
}

/// The reverse: a stored shortcut as rekordbox writes it.
pub fn stored_to_juce(stored: &str) -> String {
    let Some(sc) = Shortcut::parse(stored) else { return String::new() };
    let mut parts = vec![];
    for (on, n) in [(sc.ctrl, "ctrl"), (sc.shift, "shift"), (sc.alt, "option"), (sc.cmd, "command")] {
        if on {
            parts.push(n.to_string());
        }
    }
    let k = match sc.key.name() {
        "Space" => "spacebar".to_string(),
        "ArrowLeft" | "Left" => "cursor left".into(),
        "ArrowRight" | "Right" => "cursor right".into(),
        "ArrowUp" | "Up" => "cursor up".into(),
        "ArrowDown" | "Down" => "cursor down".into(),
        "Enter" => "return".into(),
        "Backspace" => "backspace".into(),
        "Delete" => "delete".into(),
        "Tab" => "tab".into(),
        "Escape" => "escape".into(),
        n => match sc.key.symbol_or_name() {
            s if s.chars().count() == 1 => s.to_uppercase(),
            _ => n.to_string(),
        },
    };
    parts.push(k);
    parts.join(" + ")
}

fn family(mode: Mode) -> usize {
    (mode == Mode::Edit) as usize
}

/// The mapping in use: a preset's id, or a user mapping's name (`OWN` when none was chosen).
pub fn current(prefs: &Prefs, mode: Mode) -> String {
    let c = &prefs.rb.key_mapping[family(mode)];
    if c.is_empty() { if mode == Mode::Edit { EDIT_PRESET.0.to_string() } else { OWN.to_string() } } else { c.clone() }
}

pub fn is_preset(name: &str, mode: Mode) -> bool {
    if mode == Mode::Edit { name == EDIT_PRESET.0 } else { PRESETS.iter().any(|p| p.0 == name) }
}

/// Every mapping the dropdown lists: (id or name, shown name).
pub fn mappings(prefs: &Prefs, mode: Mode) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = if mode == Mode::Edit { vec![(EDIT_PRESET.0.into(), EDIT_PRESET.1.into())] } else { PRESETS.iter().map(|p| (p.0.to_string(), p.1.to_string())).collect() };
    if mode != Mode::Edit {
        v.push((OWN.into(), OWN.into()));
    }
    // (User mappings are listed in the mode family they hold keys for: EDIT's rows are
    // 0x2013..0x2017 and 0xccxx, nobody else's.)
    let edit_row = |k: &String| u16::from_str_radix(k, 16).map(|id| matches!(id, 0x2013..=0x2017 | 0xcc00..=0xccff)).unwrap_or(false);
    for (k, m) in &prefs.rb.key_mappings {
        let is_edit = m.keys().any(edit_row) || m.get("family").map(|f| f == "edit").unwrap_or(false);
        if k != OWN && is_edit == (mode == Mode::Edit) {
            v.push((k.clone(), k.clone()));
        }
    }
    v
}

/// The key of a command under a mapping, in rekordbox's text form.
pub fn key_of(prefs: &Prefs, mode: Mode, mapping: &str, row: &Row) -> String {
    if mode == Mode::Edit && is_preset(mapping, mode) {
        return CMDS[row.cmd].p1[0].to_string();
    }
    if is_preset(mapping, mode) {
        return preset_key(row, mapping);
    }
    let hex = format!("{:x}", row.id);
    if mapping == OWN && current(prefs, mode) == OWN {
        // recordbox's own keys live in `Prefs::keymap` (action -> key).
        if let Some(a) = action_of(row.id, &row.name) {
            return prefs.shortcut_for(a).map(|s| stored_to_juce(&s.name())).unwrap_or_default();
        }
    }
    prefs.rb.key_mappings.get(mapping).and_then(|m| m.get(&hex)).cloned().unwrap_or_default()
}

/// Makes `mapping` the one in use: every recordbox action gets the key the mapping gives its
/// command (or none), and the choice is stored. recordbox's own keys are kept aside when
/// they are left, and come back when "recordbox" is chosen again.
pub fn select(prefs: &mut Prefs, mode: Mode, mapping: &str) {
    if mode == Mode::Edit {
        // (EDIT's keys are read from the mapping by keycmd::edit_keys; nothing to copy.)
        prefs.rb.key_mapping[family(mode)] = if mapping == EDIT_PRESET.0 { String::new() } else { mapping.to_string() };
        return;
    }
    let was = current(prefs, mode);
    if was == mapping {
        return;
    }
    if was == OWN {
        let own = prefs.rb.key_mappings.entry(OWN.to_string()).or_default();
        own.retain(|k, _| !k.starts_with("act:"));
        for (a, k) in &prefs.keymap {
            own.insert(format!("act:{a}"), k.clone());
        }
        own.insert("act:".into(), String::new());
    }
    prefs.rb.key_mapping[family(mode)] = mapping.to_string();
    if mapping == OWN {
        prefs.keymap = prefs.rb.key_mappings.get(OWN).map(|m| m.iter().filter(|(k, _)| k.starts_with("act:") && k.len() > 4).map(|(k, v)| (k[4..].to_string(), v.clone())).collect()).unwrap_or_default();
        return;
    }
    // Performance commands exist for four decks; recordbox's actions follow the active deck,
    // so the first deck's (and Player A's) keys are the ones that act.
    let rows = commands(Mode::Performance);
    let mut keymap = BTreeMap::new();
    for (action, ..) in crate::prefs_ext::ACTIONS {
        if let Some(row) = rows.iter().find(|r| r.unit == 0 && action_of(r.id, &r.name) == Some(action)) {
            let key = key_of(prefs, mode, mapping, row);
            keymap.insert(action.to_string(), juce_to_stored(&key).unwrap_or_default());
        }
    }
    prefs.keymap = keymap;
}

/// Gives a command a key in a user mapping (an empty key clears it).
pub fn assign(prefs: &mut Prefs, mode: Mode, mapping: &str, row: &Row, key: &str) {
    if is_preset(mapping, mode) {
        return;
    }
    let m = prefs.rb.key_mappings.entry(mapping.to_string()).or_default();
    if key.is_empty() {
        m.remove(&format!("{:x}", row.id));
    } else {
        // A key belongs to one command.
        m.retain(|k, v| k.starts_with("act:") || v != key);
        m.insert(format!("{:x}", row.id), key.to_string());
    }
    if current(prefs, mode) == mapping && row.unit == 0 {
        if let Some(a) = action_of(row.id, &row.name) {
            let stored = juce_to_stored(key).unwrap_or_default();
            if !stored.is_empty() {
                let others: Vec<String> = prefs.keymap.iter().filter(|(k, v)| **v == stored && k.as_str() != a).map(|(k, _)| k.clone()).collect();
                for o in others {
                    prefs.keymap.insert(o, String::new());
                }
            }
            prefs.keymap.insert(a.to_string(), stored);
        }
    }
}

/// A copy of a mapping under a new name (rekordbox's Edit > Copy): every command's key.
pub fn copy(prefs: &mut Prefs, mode: Mode, from: &str, name: &str) {
    let mut m = BTreeMap::new();
    for row in commands(mode) {
        let k = key_of(prefs, mode, from, &row);
        if !k.is_empty() {
            m.insert(format!("{:x}", row.id), k);
        }
    }
    if mode == Mode::Edit {
        m.insert("family".into(), "edit".into());
    }
    prefs.rb.key_mappings.insert(name.to_string(), m);
}

/// A name for a copy that is not taken yet ("Export (Preset) 2" ...).
pub fn free_name(prefs: &Prefs, base: &str) -> String {
    let base = base.trim_end_matches(" (Preset)");
    (1..).map(|n| if n == 1 { format!("{base} copy") } else { format!("{base} copy {n}") }).find(|n| !prefs.rb.key_mappings.contains_key(n) && n != OWN).unwrap()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A mapping as a rekordbox `.mappings` file (a JUCE properties file with the name and the
/// key mappings as XML).
pub fn to_mappings_file(prefs: &Prefs, mode: Mode, mapping: &str, shown: &str) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n<PROPERTIES>\n");
    s.push_str(&format!("  <VALUE name=\"keyMappingName\" val=\"{}\"/>\n  <VALUE name=\"keyMappingXml\">\n    <KEYMAPPINGS basedOnDefaults=\"0\">\n", xml_escape(shown)));
    for row in commands(mode) {
        let k = key_of(prefs, mode, mapping, &row);
        if !k.is_empty() {
            s.push_str(&format!("      <MAPPING commandId=\"{:x}\" description=\"{}\" key=\"{}\"/>\n", row.id, xml_escape(&row.name), xml_escape(&k)));
        }
    }
    s.push_str("    </KEYMAPPINGS>\n  </VALUE>\n</PROPERTIES>\n");
    s
}

/// Reads a `.mappings` file: its name and (command id in hex -> key). A file that only names
/// a preset ("based on defaults", no MAPPING lines) gives an empty map.
pub fn from_mappings_file(text: &str) -> Option<(String, BTreeMap<String, String>)> {
    let attr = |tag: &str, name: &str| -> Option<String> {
        let i = tag.find(&format!("{name}=\""))? + name.len() + 2;
        let j = tag[i..].find('"')? + i;
        Some(tag[i..j].replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#165;", "¥").replace("&amp;", "&"))
    };
    if !text.contains("<PROPERTIES") {
        return None;
    }
    let mut name = None;
    let mut map = BTreeMap::new();
    for tag in text.split('<').skip(1) {
        if tag.starts_with("VALUE") && tag.contains("name=\"keyMappingName\"") {
            name = attr(tag, "val");
        } else if tag.starts_with("MAPPING") {
            if let (Some(id), Some(key)) = (attr(tag, "commandId"), attr(tag, "key")) {
                if u16::from_str_radix(&id, 16).is_ok() && !key.is_empty() {
                    map.insert(id.to_lowercase(), key);
                }
            }
        }
    }
    Some((name?, map))
}

const KEY_WAIT_ID: &str = "prefs_key_wait";

pub(crate) fn assigning_key(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Option<u16>>(egui::Id::new(KEY_WAIT_ID))).flatten().is_some()
}

/// Preferences > Keyboard: rekordbox's `KeyboardPreference` item (628 x 598): the mapping
/// dropdown with "(Import)" as its last entry, the Edit button's menu, and the command list by
/// category.
pub fn page(app: &mut App, ui: &mut Ui, mode: Mode) {
    use crate::ui_prefs_item::item;
    let cur = current(&app.prefs, mode);
    let all = mappings(&app.prefs, mode);
    let shown = all.iter().find(|m| m.0 == cur).map(|m| m.1.clone()).unwrap_or_else(|| cur.clone());
    let preset = is_preset(&cur, mode);
    let wait_id = egui::Id::new(KEY_WAIT_ID);
    let open_id = egui::Id::new(("prefs_key_open", mode.index()));
    let rename_id = egui::Id::new("prefs_key_rename");
    let mut waiting: Option<u16> = ui.ctx().data(|d| d.get_temp(wait_id)).flatten();
    let mut open: std::collections::BTreeSet<String> = ui.ctx().data(|d| d.get_temp(open_id)).unwrap_or_default();
    let mut renaming: Option<String> = ui.ctx().data(|d| d.get_temp(rename_id)).flatten();
    let rows = commands(mode);
    let mut act: Option<&'static str> = None;
    let mut pick: Option<String> = None;
    let mut dirty = false;
    // A key pressed while a command waits for one.
    if let Some(id) = waiting {
        let got = ui.input(|i| i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } => Some(Shortcut::from_event(*key, modifiers)),
            _ => None,
        }));
        if let Some(sc) = got {
            if let Some(row) = rows.iter().find(|r| r.id == id) {
                let plain = !(sc.ctrl || sc.alt || sc.shift || sc.cmd);
                if sc.key == egui::Key::Escape {
                } else if plain && matches!(sc.key, egui::Key::Delete | egui::Key::Backspace) {
                    assign(&mut app.prefs, mode, &cur, row, "");
                    dirty = true;
                } else {
                    assign(&mut app.prefs, mode, &cur, row, &stored_to_juce(&sc.name()));
                    dirty = true;
                }
            }
            waiting = None;
        }
    }
    item(ui, "KeyboardPreference", 598.0, |it| {
        it.combo(20.0, 20.0, 480.0, &shown, |ui| {
            for (id, name) in &all {
                if ui.selectable_label(*id == cur, name.as_str()).clicked() {
                    pick = Some(id.clone());
                }
            }
            if ui.selectable_label(false, "(Import)").clicked() {
                act = Some("import");
            }
        });
        let edit = it.button(508.0, 20.0, 100.0, 22.0, "Edit");
        let pid = it.ui.make_persistent_id("prefs_key_edit_menu");
        if edit.clicked() {
            it.ui.memory_mut(|m| m.toggle_popup(pid));
        }
        egui::popup_below_widget(it.ui, pid, &edit, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            crate::ui_kit::apply_style(ui);
            ui.set_min_width(100.0);
            if ui.add_enabled(true, egui::Button::new("Copy")).clicked() {
                act = Some("copy");
            }
            if ui.add_enabled(!preset && cur != OWN, egui::Button::new("Rename")).clicked() {
                act = Some("rename");
            }
            if ui.add_enabled(!preset && cur != OWN, egui::Button::new("Delete")).clicked() {
                act = Some("delete");
            }
            if ui.add_enabled(true, egui::Button::new("Export")).clicked() {
                act = Some("export");
            }
            if ui.add_enabled(!preset, egui::Button::new("Reset")).clicked() {
                act = Some("reset");
            }
        });
        // The command list: categories that open, a command per row, its key as a button.
        let (pa, o) = it.painter();
        let area = egui::Rect::from_min_size(egui::pos2(o.x + 20.0, o.y + 58.0), egui::vec2(588.0, 520.0));
        pa.rect_filled(area, 0.0, BG);
        let mut child = it.ui.new_child(egui::UiBuilder::new().max_rect(area.shrink(1.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        egui::ScrollArea::vertical().id_salt(("prefs_key_list", mode.index())).auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            let w = ui.available_width();
            let mut cat = String::new();
            for row in &rows {
                if row.cat != cat {
                    cat = row.cat.clone();
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::click());
                    let is_open = open.contains(&cat);
                    let c = r.left_center() + egui::vec2(12.0, 0.0);
                    let tri = if is_open { vec![c + egui::vec2(-4.0, -2.5), c + egui::vec2(4.0, -2.5), c + egui::vec2(0.0, 3.5)] } else { vec![c + egui::vec2(-2.5, -4.0), c + egui::vec2(3.5, 0.0), c + egui::vec2(-2.5, 4.0)] };
                    ui.painter().add(egui::Shape::convex_polygon(tri, DIM2, egui::Stroke::NONE));
                    crate::prefs_font::draw(ui, egui::Rect::from_min_max(egui::pos2(r.min.x + 24.0, r.min.y), r.max), cat.trim(), 16.0, true, WHITE.c(), crate::ui_kit::Just::Left);
                    crate::ui_prefs_item::tab_seen(&format!("keycat:{}", cat.trim()), r);
                    if resp.clicked() {
                        if is_open { open.remove(&cat); } else { open.insert(cat.clone()); }
                    }
                }
                if !open.contains(&cat) {
                    continue;
                }
                let (r, _) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::hover());
                if !ui.is_rect_visible(r) {
                    continue;
                }
                let acts = crate::keycmd::acts(row.id, mode);
                crate::prefs_font::draw(ui, egui::Rect::from_min_max(egui::pos2(r.min.x + 40.0, r.min.y), egui::pos2(r.max.x - 190.0, r.max.y)), &row.name, 15.0, false, if acts { WHITE.c() } else { DIM2.c() }, crate::ui_kit::Just::Left);
                let key = if waiting == Some(row.id) { "Press a key…".to_string() } else { key_of(&app.prefs, mode, &cur, row) };
                let br = egui::Rect::from_min_size(egui::pos2(r.max.x - 186.0, r.min.y + 2.0), egui::vec2(170.0, 20.0));
                let resp = ui.interact(br, ui.id().with(("keybtn", row.id)), if preset { egui::Sense::hover() } else { egui::Sense::click() });
                ui.painter().rect_filled(br, 3.0, if waiting == Some(row.id) { crate::ui_kit::PBLUE } else if resp.hovered() && !preset { gy(0x3C3C3C) } else { gy(0x323232) });
                crate::prefs_font::draw(ui, br, &key, 14.0, false, if preset { DIM2.c() } else { WHITE.c() }, crate::ui_kit::Just::Centre);
                let why;
                let hint = if preset {
                    "A preset cannot be changed. Edit > Copy makes a mapping of your own from it."
                } else if acts {
                    "Click, then press the new key (Esc cancels, Delete clears)."
                } else {
                    why = format!("This key does nothing in recordbox: {} The key is kept with the mapping.", crate::keycmd::missing_reason_in(crate::keycmd::split_id(row.id).0, mode));
                    why.as_str()
                };
                if resp.on_hover_text(hint).clicked() {
                    waiting = Some(row.id);
                }
            }
        });
    });
    if let Some(p) = pick {
        select(&mut app.prefs, mode, &p);
        dirty = true;
    }
    match act {
        Some("copy") => {
            let name = free_name(&app.prefs, &shown);
            copy(&mut app.prefs, mode, &cur, &name);
            select(&mut app.prefs, mode, &name);
            dirty = true;
        }
        Some("rename") => renaming = Some(cur.clone()),
        Some("delete") => {
            select(&mut app.prefs, mode, OWN);
            app.prefs.rb.key_mappings.remove(&cur);
            dirty = true;
        }
        Some("reset") => app.dialogs.ext.reset_ask = Some(Some(3)),
        Some("export") => {
            if let Some(f) = rfd::FileDialog::new().set_file_name(format!("{}.mappings", shown.trim_end_matches(" (Preset)"))).add_filter("rekordbox key mapping", &["mappings"]).save_file() {
                if let Err(e) = std::fs::write(&f, to_mappings_file(&app.prefs, mode, &cur, &shown)) {
                    app.toast_err(format!("Could not write {}: {e}", f.display()));
                }
            }
        }
        Some("import") => {
            if let Some(f) = rfd::FileDialog::new().add_filter("rekordbox key mapping", &["mappings"]).pick_file() {
                match std::fs::read_to_string(&f).ok().and_then(|t| from_mappings_file(&t)) {
                    Some((name, map)) => {
                        let name = if is_preset(&name, mode) || name == OWN || PRESETS.iter().any(|p| p.1 == name) || app.prefs.rb.key_mappings.contains_key(&name) { free_name(&app.prefs, &name) } else { name };
                        app.prefs.rb.key_mappings.insert(name.clone(), map);
                        select(&mut app.prefs, mode, &name);
                        dirty = true;
                    }
                    None => app.toast_err("The selected file is not a key mapping file."),
                }
            }
        }
        _ => {}
    }
    if let Some(old) = renaming.clone() {
        let mut close = false;
        let text_id = egui::Id::new("prefs_key_rename_text");
        let mut text: String = ui.ctx().data(|d| d.get_temp(text_id)).unwrap_or_else(|| old.clone());
        crate::ui_kit::dialog(ui.ctx(), "Rename").resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ui.ctx(), |ui| {
            crate::ui_kit::apply_style(ui);
            ui.set_width(320.0);
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(300.0));
            match crate::ui_kit::dialog_buttons(ui, &["OK", "Cancel"]) {
                Some(0) => {
                    let new = text.trim().to_string();
                    if !new.is_empty() && new != old && new != OWN && !app.prefs.rb.key_mappings.contains_key(&new) && !PRESETS.iter().any(|p| p.1 == new) {
                        if let Some(m) = app.prefs.rb.key_mappings.remove(&old) {
                            app.prefs.rb.key_mappings.insert(new.clone(), m);
                            app.prefs.rb.key_mapping[family(mode)] = new;
                            dirty = true;
                        }
                    }
                    close = true;
                }
                Some(_) => close = true,
                None => {}
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(text_id, text));
        if close {
            renaming = None;
            ui.ctx().data_mut(|d| d.remove::<String>(text_id));
        }
    }
    ui.ctx().data_mut(|d| {
        d.insert_temp(wait_id, waiting);
        d.insert_temp(open_id, open);
        d.insert_temp(rename_id, renaming);
    });
    if dirty {
        app.save_prefs();
    }
}

/// "Reset" of the Edit menu: a user mapping goes back to having no keys of its own
/// (recordbox's own mapping back to its defaults).
pub fn reset_current(prefs: &mut Prefs, mode: Mode) {
    let cur = current(prefs, mode);
    if is_preset(&cur, mode) {
        return;
    }
    prefs.rb.key_mappings.remove(&cur);
    if cur == OWN {
        prefs.keymap.clear();
    } else {
        prefs.rb.key_mappings.insert(cur.clone(), BTreeMap::new());
        prefs.rb.key_mapping[family(mode)] = String::new();
        let keep = prefs.keymap.clone();
        select(prefs, mode, &cur);
        let _ = keep;
    }
}

/// Keyboard commands of `mode` whose name contains `q` (lower case).
pub fn search(q: &str, mode: Mode) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    commands(mode).into_iter().filter(|r| r.name.to_lowercase().contains(q)).filter(|r| seen.insert(r.name.clone())).map(|r| r.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_give_rekordbox_s_keys() {
        let perf = commands(Mode::Performance);
        let find = |id: u16| perf.iter().find(|r| r.id == id).unwrap();
        // Performance 1: deck 1 plays with Z and cues with A, deck 2 with N and H; deck 3 has
        // no keys and deck 4 has deck 2's; the sampler's first slot is X.
        assert_eq!(preset_key(find(0x3006), "0000000000000"), "Z");
        assert_eq!(preset_key(find(0x3007), "0000000000000"), "A");
        assert_eq!(preset_key(find(0x3106), "0000000000000"), "N");
        assert_eq!(preset_key(find(0x3206), "0000000000000"), "");
        assert_eq!(preset_key(find(0x3306), "0000000000000"), "N");
        assert_eq!(preset_key(find(0xf000), "0000000000000"), "X");
        assert_eq!(preset_key(find(0xf010), "0000000000000"), "shift + X");
        // Performance 2 (the binary's "Serato" branch).
        assert_eq!(preset_key(find(0x3006), "0000000000010"), "W");
        assert_eq!(preset_key(find(0xf000), "0000000000010"), "Z");
        // None has no keys at all.
        assert!(perf.iter().all(|r| preset_key(r, "9999999999999").is_empty()));
        // Export: Player A's keys, and Player B's are the same with Shift.
        let exp = commands(Mode::Export);
        let e = |id: u16| preset_key(exp.iter().find(|r| r.id == id).unwrap(), "0000000000030");
        assert_eq!(e(0x3006), "spacebar");
        assert_eq!(e(0x3106), "shift + spacebar");
        assert_eq!(e(0x3007), "C");
        assert_eq!(e(0x3107), "shift + C");
        assert_eq!(e(0x200a), "command + ,");
        // The categories of each mode.
        let cats = |m| { let mut v: Vec<String> = vec![]; for r in commands(m) { if v.last() != Some(&r.cat) { v.push(r.cat) } } v };
        let p = cats(Mode::Performance);
        for c in ["Browse", "Deck 1", "Deck 2", "Deck 3", "Deck 4", "Sampler", "Mixer", "Recordings", "FX1", "FX2"] {
            assert!(p.iter().any(|x| x == c), "{c} in {p:?}");
        }
        let x = cats(Mode::Export);
        assert!(x.iter().any(|c| c == "Player A") && x.iter().any(|c| c == "Player B") && !x.iter().any(|c| c == "Deck 3"));
        assert_eq!(cats(Mode::Edit), ["General", "File", "Deck", "Action pad", "Tool area"]);
        // No command id appears twice in a mode.
        for m in [Mode::Export, Mode::Performance, Mode::Edit] {
            let mut ids = std::collections::HashSet::new();
            for r in commands(m) {
                assert!(ids.insert(r.id), "{:x} twice in {m:?}", r.id);
            }
        }
    }

    #[test]
    fn key_text_converts_both_ways() {
        for (juce, stored) in [("spacebar", "Space"), ("shift + command + F", "Shift+Cmd+F"), ("command + cursor left", "Cmd+ArrowLeft"), ("ctrl + shift + F", "Ctrl+Shift+F"), ("option + 1", "Alt+1"), ("F10", "F10"), ("command + ,", "Cmd+Comma"), ("/", "Slash"), (";", "Semicolon"), ("return", "Enter")] {
            assert_eq!(juce_to_stored(juce).and_then(|s| Shortcut::parse(&s)), Shortcut::parse(stored), "{juce}");
            assert_eq!(juce_to_stored(&stored_to_juce(stored)).and_then(|s| Shortcut::parse(&s)), Shortcut::parse(stored), "{stored} -> {}", stored_to_juce(stored));
        }
        assert_eq!(stored_to_juce("Shift+Cmd+F"), "shift + command + F");
        assert_eq!(juce_to_stored("option + ¥").and_then(|s| Shortcut::parse(&s)), Shortcut::parse("Alt+Backslash"));
        assert_eq!(juce_to_stored(""), None);
        // Every key of the presets is one recordbox can take, or the command has no action.
        for row in commands(Mode::Performance).iter().chain(commands(Mode::Export).iter()) {
            for p in PRESETS {
                let k = preset_key(row, p.0);
                // (A few preset keys are symbols of another keyboard layout, "@" and ":";
                // egui has no key for them, so those commands have no key in recordbox.)
                let last = k.rsplit(" + ").next().unwrap_or("");
                if !k.is_empty() && action_of(row.id, &row.name).is_some() && !matches!(last, "@" | ":" | "*" | "^") {
                    assert!(juce_to_stored(&k).is_some(), "{} {k}", row.name);
                }
            }
        }
    }

    #[test]
    fn choosing_a_preset_sets_the_keys_and_recordbox_s_come_back() {
        let mut p = Prefs::default();
        let m = Mode::Performance;
        p.keymap.insert("play".into(), "P".into());
        assert_eq!(current(&p, m), OWN);
        assert_eq!(p.shortcut_for("play").unwrap().name(), "P");
        select(&mut p, m, "0000000000000");
        assert_eq!(p.shortcut_for("play").unwrap().name(), "Z");
        assert_eq!(p.shortcut_for("cue").unwrap().name(), "A");
        assert_eq!(p.shortcut_for("smp_play1").unwrap().name(), "X");
        assert!(p.shortcut_for("hot5").is_none(), "Set Hot Cue E has no key in Performance 1");
        select(&mut p, m, "0000000000030");
        assert_eq!(p.shortcut_for("play").unwrap().name(), "Space");
        assert_eq!(p.shortcut_for("memory").unwrap().name(), "M");
        select(&mut p, m, "9999999999999");
        assert!(p.shortcut_for("play").is_none() && p.shortcut_for("cue").is_none());
        // A preset cannot be edited; a copy can.
        let rows = commands(m);
        let play = rows.iter().find(|r| r.id == 0x3006).unwrap();
        assign(&mut p, m, "9999999999999", play, "Q");
        assert!(p.shortcut_for("play").is_none());
        copy(&mut p, m, "0000000000000", "mine");
        select(&mut p, m, "mine");
        assert_eq!(p.shortcut_for("play").unwrap().name(), "Z");
        assign(&mut p, m, "mine", play, "shift + Q");
        assert_eq!(p.shortcut_for("play").unwrap().name(), "Shift+Q");
        assert_eq!(key_of(&p, m, "mine", play), "shift + Q");
        // A command recordbox has no action for keeps its key and changes nothing else.
        let slip = rows.iter().find(|r| r.name == "Slip Reverse" && r.unit == 0).unwrap();
        assert!(action_of(slip.id, &slip.name).is_none());
        assign(&mut p, m, "mine", slip, "option + S");
        assert_eq!(key_of(&p, m, "mine", slip), "option + S");
        // The file form reads back.
        let text = to_mappings_file(&p, m, "mine", "mine");
        let (name, map) = from_mappings_file(&text).unwrap();
        assert_eq!(name, "mine");
        assert_eq!(map.get("3006").map(|s| s.as_str()), Some("shift + Q"));
        assert_eq!(map.get(&format!("{:x}", slip.id)).map(|s| s.as_str()), Some("option + S"));
        assert!(from_mappings_file("not a mapping").is_none());
        // rekordbox's own preset files (a name, no mappings) read as empty mappings.
        let (n, m2) = from_mappings_file("<?xml version=\"1.0\"?>\n<PROPERTIES>\n  <VALUE name=\"keyMappingName\" val=\"Performance 1 (Preset)\"/>\n</PROPERTIES>").unwrap();
        assert!(n == "Performance 1 (Preset)" && m2.is_empty());
        // Back to recordbox's own keys.
        select(&mut p, m, OWN);
        assert_eq!(p.shortcut_for("play").unwrap().name(), "P");
        assert_eq!(p.shortcut_for("cue").unwrap().name(), "C");
    }
}
