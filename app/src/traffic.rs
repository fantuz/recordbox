//! Traffic Light: tracks in the browser whose key matches the key of the track on a chosen
//! deck get a green Key cell. Traced from rekordbox 7.2.19 (notes in
//! `checks/re-7.2.19/shell-menus-traffic.md`):
//!
//! - the control in the track list's header is `browse::TrafficLightSelector` (0x1013df28c): a
//!   button that switches the feature on and off (off by default) with a pull-down that picks
//!   the target. EXPORT: MASTER, PLAYER A, and PLAYER B in the two-player layouts (PLAYER A
//!   by default; PLAYER B falls back to PLAYER A when the layout loses the second player).
//!   PERFORMANCE: MASTER, DECK1 .. DECK4 (MASTER by default). The choice is kept per mode.
//! - the target key (`BrowseListViewer::updateTrafficLight` 0x1007a56cc): the key of the
//!   target deck's track, moved by the deck's key shift when Master Tempo is on, else by the
//!   semitones its tempo change amounts to.
//! - the range (`TrafficLightRange`, Preferences, default 4): 1 Same Key; 2 Related Key 1 adds
//!   the relative major / minor; 3 Related Key 2 adds the neighbours on the wheel; 4 Related
//!   Key 3 adds the neighbours' relatives. rekordbox's own example for 2A: 2A / 2A 2B /
//!   2A 2B 1A 3A / 2A 2B 1A 3A 1B 3B.
//! - painting (`BrowseListViewer::paintKeyColumn` 0x1007a6a6c): only in the main list while
//!   it shows the collection or a playlist; the Key cell is filled #00FF00 from 1 pt under the
//!   row's top to 1 pt above its foot and the key is written in black.
//!
//! - the button's look (`SelectorButton` ctor 0x100f03ad4, `lookAndFeelChanged` 0x100f04100,
//!   `CustomTextButton::paintButton` 0x1002d0e20): Arial 11 bold white text on #282828 in
//!   both states (colour ids 0x1000281 / 0x3000012); switched on it gets a 1 pt #1473EB
//!   outline (colour 0x300001F off = no line, on = the `0x1473EB` constant of
//!   `__GLOBAL__sub_I_SelectorButton`), round on the left only; the 16 pt pull-down part is
//!   #282828 with `pullDown.svg` in white (0x3000016). Off draws no outline (the paint style
//!   switches the line on for the "on" state only).

use crate::app::*;
use crate::model::*;
use serde::{Deserialize, Serialize};

/// Targets, with rekordbox's numbers: 1 MASTER, 2..5 DECK1..DECK4, 6 PLAYER A, 7 PLAYER B.
pub const MASTER: u8 = 1;
pub const PLAYER_A: u8 = 6;
pub const PLAYER_B: u8 = 7;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TrafficPrefs {
    /// The header button: Traffic Light is on.
    pub on: bool,
    /// The target in EXPORT mode (1, 6 or 7) and in PERFORMANCE mode (1..5).
    pub target_export: u8,
    pub target_perf: u8,
    /// Preferences > Traffic Light: 1 Same Key .. 4 Related Key 3.
    pub range: u8,
}

impl Default for TrafficPrefs {
    fn default() -> Self {
        TrafficPrefs { on: false, target_export: PLAYER_A, target_perf: MASTER, range: 4 }
    }
}

pub const RANGE_NAMES: [&str; 4] = ["Same Key", "Related Key 1", "Related Key 2", "Related Key 3"];
pub const RANGE_HELP: &str = "The range of keys to be highlighted can be set with the Traffic Light feature.\nFor example, if the KEY of the loaded track is 2A(Alphanumeric), \ntracks with the following keys in the library are highlighted.\nSame Key:\n2A\nRelated Key 1:\n2A/2B\nRelated Key 2:\n2A/2B/1A/3A\nRelated Key 3:\n2A/2B/1A/3A/1B/3B";
pub const TIP_ENABLE: &str = "Enable/disable Traffic Light feature.\nTraffic Light feature:\nWhen it is enabled, tracks in browser area that match the key of the loaded track are highlighted in green.";
pub const TIP_TARGET: &str = "Select the target deck for Traffic Light feature.";

/// Paint the header button (see the notes above): fill, then the 1 pt blue outline when on.
pub fn paint_button(p: &eframe::egui::Painter, left: eframe::egui::Rect, right: eframe::egui::Rect, on: bool) {
    use crate::theme::{BLUE, BTN};
    use eframe::egui::{CornerRadius, Stroke, StrokeKind};
    let lr = CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 };
    p.rect_filled(left, lr, BTN);
    p.rect_filled(right, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, BTN);
    if on {
        p.rect_stroke(left, lr, Stroke::new(1.0, BLUE), StrokeKind::Inside);
    }
}

/// The keys a range lights up for a target key (model index: 0..11 = 1A..12A, 12..23 =
/// 1B..12B). Empty when the target key is unknown.
pub fn matching_keys(key: i32, range: u8) -> Vec<i32> {
    if !(0..24).contains(&key) {
        return Vec::new();
    }
    let (n, mode) = (key % 12, key / 12);
    let at = |n: i32, mode: i32| n.rem_euclid(12) + mode * 12;
    let other = 1 - mode;
    let mut v = vec![key];
    if range >= 2 {
        v.push(at(n, other));
    }
    if range >= 3 {
        v.push(at(n - 1, mode));
        v.push(at(n + 1, mode));
    }
    if range >= 4 {
        v.push(at(n - 1, other));
        v.push(at(n + 1, other));
    }
    v
}

/// A key moved by the deck's pitch: its key shift with Master Tempo on, else the whole
/// semitones its speed amounts to.
pub fn sounding_key(key: i32, master_tempo: bool, key_shift: i32, speed: f64) -> i32 {
    let semis = if master_tempo {
        key_shift
    } else if speed > 0.0 {
        (12.0 * speed.log2()).round() as i32
    } else {
        0
    };
    crate::player_ext::shift_key(key, semis)
}

/// Label and tooltip of a target.
pub fn target_label(t: u8) -> (&'static str, &'static str) {
    match t {
        2 => ("DECK1", "DECK1 - Traffic Light"),
        3 => ("DECK2", "DECK2 - Traffic Light"),
        4 => ("DECK3", "DECK3 - Traffic Light"),
        5 => ("DECK4", "DECK4 - Traffic Light"),
        PLAYER_A => ("PLAYER A", "PLAYER A - Traffic Light"),
        PLAYER_B => ("PLAYER B", "PLAYER B - Traffic Light"),
        _ => ("MASTER", "MASTER DECK - Traffic Light"),
    }
}

impl App {
    /// The targets the pull-down offers in the current mode and layout.
    pub fn traffic_targets(&self) -> Vec<u8> {
        if self.performance() {
            vec![MASTER, 2, 3, 4, 5]
        } else if self.two_players() {
            vec![MASTER, PLAYER_A, PLAYER_B]
        } else {
            vec![MASTER, PLAYER_A]
        }
    }

    /// The chosen target of the current mode (PLAYER B only while the layout has it).
    pub fn traffic_target(&self) -> u8 {
        let t = if self.performance() { self.prefs.traffic.target_perf } else { self.prefs.traffic.target_export };
        if self.traffic_targets().contains(&t) {
            t
        } else if self.performance() {
            MASTER
        } else {
            PLAYER_A
        }
    }

    pub fn set_traffic_target(&mut self, t: u8) {
        if self.performance() {
            self.prefs.traffic.target_perf = t;
        } else {
            self.prefs.traffic.target_export = t;
        }
        self.save_prefs();
    }

    /// The deck (0 = deck 1 / player A) a target stands for.
    fn traffic_deck(&self, target: u8) -> Option<u8> {
        let count = self.deck_count();
        let n = match target {
            MASTER => self.px.master,
            2..=5 => target - 2,
            PLAYER_A => 0,
            PLAYER_B => 1,
            _ => return None,
        };
        (n < count.max(1)).then_some(n)
    }

    /// The key the target deck sounds in, or -1 (no track there, or its key is unknown).
    pub fn traffic_key(&self) -> i32 {
        let Some(n) = self.traffic_deck(self.traffic_target()) else { return -1 };
        let deck = if n == self.px.active { &self.deck } else { &self.px.parked[n as usize] };
        let Some(t) = deck.track.and_then(|id| self.track(id)) else { return -1 };
        let speed = self.player.decks[n as usize].lock().tempo;
        sounding_key(t.key, deck.x.master_tempo, deck.x.key_shift, speed)
    }

    /// The keys to light up in the main track list this frame; empty when Traffic Light is
    /// off or the list is not one it applies to (the collection and playlists).
    pub fn traffic_matches(&self) -> Vec<i32> {
        if !self.prefs.traffic.on || !matches!(self.view, View::Collection | View::Playlist(_)) {
            return Vec::new();
        }
        matching_keys(self.traffic_key(), self.prefs.traffic.range.clamp(1, 4))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(keys: &[i32]) -> Vec<String> {
        keys.iter().map(|k| key_alnum(*k)).collect()
    }

    #[test]
    fn ranges_match_rekordboxs_own_example() {
        let k = parse_key("2A");
        assert_eq!(names(&matching_keys(k, 1)), ["2A"]);
        assert_eq!(names(&matching_keys(k, 2)), ["2A", "2B"]);
        assert_eq!(names(&matching_keys(k, 3)), ["2A", "2B", "1A", "3A"]);
        assert_eq!(names(&matching_keys(k, 4)), ["2A", "2B", "1A", "3A", "1B", "3B"]);
        // The wheel wraps: 1A's lower neighbour is 12A, 12B's upper one is 1B.
        assert_eq!(names(&matching_keys(parse_key("1A"), 3)), ["1A", "1B", "12A", "2A"]);
        assert_eq!(names(&matching_keys(parse_key("12B"), 4)), ["12B", "12A", "11B", "1B", "11A", "1A"]);
        assert!(matching_keys(-1, 4).is_empty());
    }

    #[test]
    fn the_target_key_follows_key_shift_or_tempo() {
        let am = parse_key("Am");
        // Master Tempo on: the key shift counts, the tempo does not.
        assert_eq!(key_name(sounding_key(am, true, 1, 1.06), false), "Bbm");
        assert_eq!(sounding_key(am, true, 0, 1.2), am);
        // Master Tempo off: +6 % is one semitone up (5.9 % is a semitone), +2 % is none.
        assert_eq!(key_name(sounding_key(am, false, 0, 1.06), false), "Bbm");
        assert_eq!(sounding_key(am, false, 0, 1.02), am);
        assert_eq!(key_name(sounding_key(am, false, 0, 0.944), false), "Abm");
        // An unknown key stays unknown.
        assert_eq!(sounding_key(-1, true, 3, 1.0), -1);
    }

    #[test]
    fn settings_default_as_in_rekordbox_and_old_prefs_load() {
        let d = TrafficPrefs::default();
        assert!(!d.on);
        assert_eq!((d.target_export, d.target_perf, d.range), (PLAYER_A, MASTER, 4));
        let p: TrafficPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, d);
        let p: TrafficPrefs = serde_json::from_str(r#"{"on":true,"range":2}"#).unwrap();
        assert!(p.on && p.range == 2 && p.target_export == PLAYER_A);
    }
}
