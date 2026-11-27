//! Application state and operations (library, player, analysis, export).

use eframe::egui;
use crate::analysis::{AnalysisSettings, BPM_RANGES};
use crate::audio::{Decoded, Player};
use crate::db::Db;
use crate::jobs::*;
use crate::model::*;
use crate::smart::SmartRule;
use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
mod stem_cache_tests;
#[cfg(test)]
mod hot_cue_persistence_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Col {
    Num,
    Status,
    Preview,
    Artwork,
    Title,
    Artist,
    Album,
    Genre,
    Bpm,
    Rating,
    Time,
    Key,
    DateAdded,
    Color,
    Comments,
    Label,
    Remixer,
    OriginalArtist,
    Composer,
    Year,
    Bitrate,
    SampleRate,
    FileType,
    Size,
    Location,
    FileName,
    PlayCount,
    MixName,
    TrackNumber,
    MyTag,
    ReleaseDate,
    AlbumArtist,
    DiscNumber,
    Bitdepth,
    Grouping,
    Lyricist,
    Message,
    HotCue,
    DateCreated,
}

pub const ALL_COLS: [Col; 39] = [
    Col::Num,
    Col::Status,
    Col::Preview,
    Col::Artwork,
    Col::Title,
    Col::Artist,
    Col::Album,
    Col::Genre,
    Col::Bpm,
    Col::Rating,
    Col::Time,
    Col::Key,
    Col::DateAdded,
    Col::Color,
    Col::Comments,
    Col::Label,
    Col::Remixer,
    Col::OriginalArtist,
    Col::Composer,
    Col::Year,
    Col::Bitrate,
    Col::SampleRate,
    Col::FileType,
    Col::Size,
    Col::Location,
    Col::FileName,
    Col::PlayCount,
    Col::MixName,
    Col::TrackNumber,
    Col::MyTag,
    Col::ReleaseDate,
    Col::AlbumArtist,
    Col::DiscNumber,
    Col::Bitdepth,
    Col::Grouping,
    Col::Lyricist,
    Col::Message,
    Col::HotCue,
    Col::DateCreated,
];

impl Col {
    pub fn label(self) -> &'static str {
        match self {
            Col::Num => "#",
            Col::Status => "",
            Col::Preview => "Preview",
            Col::Artwork => "Artwork",
            Col::Title => "Track Title",
            Col::Artist => "Artist",
            Col::Album => "Album",
            Col::Genre => "Genre",
            Col::Bpm => "BPM",
            Col::Rating => "Rating",
            Col::Time => "Time",
            Col::Key => "Key",
            Col::DateAdded => "Date Added",
            Col::Color => "Color",
            Col::Comments => "Comments",
            Col::Label => "Label",
            Col::Remixer => "Remixer",
            Col::OriginalArtist => "Original Artist",
            Col::Composer => "Composer",
            Col::Year => "Year",
            Col::Bitrate => "Bitrate",
            Col::SampleRate => "Sample Rate",
            Col::FileType => "File Type",
            Col::Size => "Size",
            Col::Location => "Location",
            Col::FileName => "File Name",
            Col::PlayCount => "DJ Play Count",
            Col::MixName => "Mix Name",
            Col::TrackNumber => "Track number",
            Col::MyTag => "My Tag",
            Col::ReleaseDate => "Release Date",
            Col::AlbumArtist => "Album Artist",
            Col::DiscNumber => "Disc number",
            Col::Bitdepth => "Bitdepth",
            Col::Grouping => "Grouping",
            Col::Lyricist => "Lyricist",
            Col::Message => "Message",
            Col::HotCue => "Hot Cue",
            Col::DateCreated => "Date Created",
        }
    }
    pub fn key(self) -> String {
        format!("{:?}", self)
    }
    pub fn width(self) -> f32 {
        match self {
            // Default widths measured on the original's Collection view (pt).
            Col::Num => 60.0,
            Col::Status => 90.0,
            Col::Preview => 141.0,
            Col::Artwork => 80.0,
            Col::Title | Col::Artist | Col::Album | Col::Genre => 128.0,
            Col::Bpm => 80.0,
            Col::Rating => 90.0,
            Col::Time => 80.0,
            Col::Key => 128.0,
            Col::DateAdded | Col::ReleaseDate | Col::DateCreated => 82.0,
            Col::Color => 46.0,
            Col::Location => 260.0,
            Col::Comments | Col::MyTag => 160.0,
            Col::HotCue => 90.0,
            _ => 100.0,
        }
    }
    pub fn right_aligned(self) -> bool {
        matches!(self, Col::Bpm | Col::Time | Col::Bitrate | Col::SampleRate | Col::Size | Col::PlayCount | Col::Year | Col::TrackNumber | Col::DiscNumber | Col::Bitdepth | Col::Num)
    }
}

/// The experimental hybrid backend is exercised by an explicit separation request.
pub(crate) const MANUAL_STEMS_PREVIEW: bool = cfg!(all(target_os = "linux", feature = "vulkan"));

impl Prefs {
    pub(crate) fn auto_stems_on_load(&self, gpu: bool) -> bool {
        !MANUAL_STEMS_PREVIEW && self.show_stems && self.stems_auto_separate && (gpu || self.stems_auto_cpu) && crate::stems::backend_hold().is_none()
    }

    fn stem_cache_limit(&self) -> Option<u64> {
        (!self.keep_all_stems).then_some(u64::from(self.stem_cache_gb) * 1_000_000_000)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub key_alnum: bool,
    pub bpm_range: usize,
    pub analyze_grid: bool,
    pub analyze_key: bool,
    pub auto_analysis: bool,
    pub wave_color: u8,
    pub quantize: bool,
    pub time_remain: bool,
    pub beat_jump: f64,
    pub loop_beats: f64,
    pub loop_auto: bool,
    pub zoom: usize,
    pub columns: Vec<Col>,
    pub sort_col: Col,
    pub sort_asc: bool,
    pub delete_device_tracks: bool,
    pub target_lufs: f64,
    pub tooltips: bool,
    pub show_mytag: bool,
    pub grid_from_current: bool,
    pub devices_extra: Vec<String>,
    pub widths: HashMap<String, f32>,
    pub browser_only: bool,
    /// Copy and sync the installed rekordbox library (master.db) every time Recordbox starts.
    // Before 0.6 startup sync was enabled even without a deliberate opt-in.
    pub import_rekordbox_at_startup: bool,
    /// Player settings written to every exported USB (Preferences > DJ System).
    pub usb_settings: crate::export::settings::UsbSettings,
    /// Which field the track-list search looks in (0 = all; see SEARCH_SCOPES).
    pub search_scope: u8,
    pub related: RelatedRules,
    /// Browse categories and sort options written to USB (Preferences > DJ System).
    pub usb_browse: crate::export::usb::UsbBrowse,
    /// Separation model chosen for this machine (None: choose on next launch).
    pub stem_model: Option<String>,
    /// Download the chosen model in the background at launch.
    pub stems_auto_download: bool,
    /// Preferences > Controller > Deck (Performance mode). Eject/Load Lock: a playing deck
    /// refuses a new track. Instant Doubles: loading the track that is on the other deck
    /// takes over its position, tempo and play state. Both off by default, as in rekordbox.
    pub deck_load_lock: bool,
    pub instant_doubles: bool,
    /// Performance: the FX panel is shown.
    pub perf_fx: bool,
    /// Performance: 4Deck Horizontal instead of 2Deck Horizontal.
    pub perf_four: bool,
    /// The rest of the Performance layout (vertical, Browse, zoom ...): ui_perf_layouts.rs.
    pub perf_lay: crate::ui_perf_layouts::LayoutPrefs,
    /// Performance: the sampler panel is shown.
    pub perf_sampler: bool,
    /// The sampler deck: its slots and settings.
    pub sampler: crate::ui_sampler::SamplerPrefs,
    /// Traffic Light (traffic.rs): on / off, target per mode, range.
    pub traffic: crate::traffic::TrafficPrefs,
    /// The Performance deck's settings (Preferences > Controller > Deck and Mixer; `deck.rs`).
    #[serde(default)]
    pub deck: crate::deck::DeckPrefs,
    /// Groove Circuit: Preferences > Extensions > STEMS, its rows (groove.rs).
    pub groove: crate::groove::GroovePrefs,
    /// Controller settings (midi.rs).
    pub midi: crate::midi::MidiPrefs,
    /// Recording: the panel and Preferences > Controller > Recordings (ui_rec.rs).
    pub rec: crate::ui_rec::RecPrefs,
    /// The pad modes' settings (layout, BEAT JUMP pages, hot cue colour set, slicer, USER pads).
    pub pads: crate::pads::PadsPrefs,
    /// Automix, MIX POINT LINK and the sampler lists (automix.rs, mpl.rs, sampler_lib.rs).
    pub mixx: crate::automix::MixPrefs,
    /// The FX panel's choices (rekordbox's FxUnitSettings): modes, effects, beat values,
    /// single-mode snapshots, RELEASE FX and the CFX row.
    pub fx: crate::ui_fx::FxSaved,
    /// 0 = EXPORT, 1 = PERFORMANCE (the header's mode list).
    pub app_mode: u8,
    /// EDIT mode's settings (ui_edit.rs); `app_mode` 2 is EDIT.
    pub edit: crate::ui_edit::EditPrefs,
    /// Preferences > Controller > Mixer: HIGH / MID / LOW as an isolator (else an equalizer),
    /// and the channel fader curve (0 Exponential, 1 Linear, 2 Logarithmic).
    pub eq_isolator: bool,
    pub ch_fader_curve: u8,
    /// Separate a track when it is loaded, as rekordbox does while its STEMS function is on.
    pub stems_auto_separate: bool,
    /// Also start uncached model inference on Load when no accelerator is available.
    pub stems_auto_cpu: bool,
    /// Preview finalized sections during eligible GPU jobs started after this preference is set.
    pub stems_progressive_preview: bool,
    /// Storage budget in decimal GB. At zero, each new save keeps only its own result.
    pub stem_cache_gb: u32,
    /// Skip automatic eviction and retain the budget in case the user switches back.
    pub keep_all_stems: bool,
    /// Import play histories from connected devices automatically.
    pub import_device_history: bool,
    /// Keyboard shortcuts changed from their defaults: action id -> key name.
    pub keymap: std::collections::BTreeMap<String, String>,
    /// Column browser (Genre / Artist / Album) above the track list.
    pub column_browser: bool,
    /// Auto gain never lifts a track's measured peak to full scale (rekordbox's optional cap).
    pub auto_gain_peak_cap: bool,
    /// Quantize resolution: 1 = whole beats, 2 = half, 4 = quarter, 8 = eighth beats.
    pub quantize_div: u8,
    /// CUE Analysis: hot cue A ("1.1Bars") on the first downbeat when a track is first analysed.
    pub auto_first_downbeat_cue: bool,
    /// Stem pads: 4 (vocal, inst, bass, drums) or 3 (vocal, inst with bass, drums).
    pub stem_parts: u8,
    /// Settings this version doesn't know (written by a newer Recordbox), kept as they are
    /// when the preferences are saved again.
    #[serde(flatten)]
    pub other: std::collections::BTreeMap<String, serde_json::Value>,
    /// Audio output device by name; None follows the system's default output.
    pub audio_device: Option<String>,
    /// Sub-browser: a second track list under the main one.
    pub sub_browser: bool,
    /// Preferences > View > Browse: font size and line space steps (0–4), bold text, and
    /// keeping the selected track in the middle of the list.
    pub browse_font: u8,
    pub browse_line: u8,
    pub browse_bold: bool,
    pub center_selected: bool,
    /// Vocal positions above the overview waveform.
    pub show_vocal: bool,
    /// Detect vocals when analysing (rekordbox's detector, `vocal_rb`; the stem model without rekordbox).
    pub vocal_analysis: bool,
    /// The column set and sort not in use: playlists and the collection each keep their own
    /// (rekordbox shows # and fewer columns in playlists). `columns_for_playlist` says which
    /// set `columns` currently holds.
    pub columns_other: Vec<Col>,
    pub sort_other: (Col, bool),
    pub columns_for_playlist: bool,
    // ---- Preferences window (prefs_ext.rs documents each and who reads it) ----
    /// View > Layout > Tree View: source name -> shown (a missing name is shown).
    pub tree_items: std::collections::BTreeMap<String, bool>,
    /// View > Layout: Add "My Tag" to the "Comments".
    pub mytag_to_comments: bool,
    /// View > Layout: "Display Cue Markers on Preview".
    pub cue_markers_on_preview: bool,
    /// View > Color: Half Waveform (upper half only) instead of Full Waveform (mirrored).
    pub wave_half: bool,
    /// View > Color > Customize Color Comments: names of the 8 track colours (Pink..Purple).
    pub color_names: [String; 8],
    /// Select the earlier recordbox estimator for both Normal and Dynamic analysis.
    pub use_recordbox_estimator: bool,
    /// Refine the selected detector's grid against transient attacks in the audio.
    pub refine_beatgrid_transients: bool,
    /// Additional model-based beat grid analysis, requested by the user.
    pub use_installed_beatgrid_model: bool,
    /// CUE Analysis: "Set Memory Cue at the first beat of track when analyzing".
    pub memory_cue_first_beat: bool,
    /// Audio: buffer size in frames (0 = the device's default), sample rate (0 = the
    /// device's default) and the first channel of the output pair (0 = channels 1/2).
    pub audio_buffer: u32,
    pub audio_rate: u32,
    pub audio_channel: u16,
    /// First channel of the headphones output pair on the same device (None: no headphones
    /// output).
    pub hp_channel: Option<u16>,
    /// DJ System > General: Background Color (0 Default Color, 1..8 Pink..Purple).
    pub device_bg_color: u8,
    /// DJ System > Device: "Delete from the device after importing the play history".
    pub delete_history_after_import: bool,
    /// Folder that "Import To Collection" copies a device's music into (empty: the default,
    /// see `App::device_import_dir`).
    pub device_import_dir: String,
    /// DJ System > Others: Hot Cue Auto Load switched on for tracks as they are imported.
    pub hot_cue_auto_load_on_import: bool,
    /// Phrase analysis: shown in the player and written to USB exports.
    pub phrase_analysis: bool,
    /// Library (see library_ext.rs). Auto Relocate Search Folders: missing files are looked
    /// for under these.
    pub relocate_folders: Vec<String>,
    /// iTunes Library File chosen by the user (None: the usual places, else the Music app).
    pub itunes_xml: Option<String>,
    /// The rekordbox xml file browsed under "rekordbox xml" in the tree.
    pub rekordbox_xml_path: Option<String>,
    /// Track list artwork: false shows the upper part in normal rows, true the entire artwork
    /// in taller rows.
    pub artwork_full: bool,
    /// Playlist Palette: up to four playlists (0 = not assigned) and whether it is shown.
    pub palette: Vec<i64>,
    pub palette_open: bool,
    /// Tree shortcuts to playlists and folders.
    pub shortcuts: Vec<i64>,
    pub shortcuts_open: bool,
    /// Information window docked at the right of the browser.
    pub info_panel: bool,
    /// Saved "Related Tracks" lists; `related` holds the criteria of the selected one.
    pub related_lists: Vec<crate::library_ext::RelatedList>,
    pub related_sel: usize,
    /// Browser layout and right-hand panels (browser_ext.rs).
    pub bx: crate::browser_ext::BrowserPrefs,
    /// Player (see player_ext.rs). Beat Count Display above the enlarged waveform: 0 Current
    /// Position (Bars), 1 Count to the next MEMORY CUE (Bars), 2 the same in Beats.
    pub beat_count_mode: u8,
    /// "Click on the waveform for PLAY and CUE".
    pub wave_click_play_cue: bool,
    /// Player layout: 0 = 1 Player, 1 = 2 Players, 2 = Simple Player (`browser_only` hides it).
    pub player_layout: u8,
    /// Maximum automatic playback animation rate. Zero follows the display.
    pub playback_fps: u8,
    /// Metronome volume step 1–3.
    pub metronome_vol: u8,
    /// Beat jump length shown and chosen in bars (`beat_jump` stays in beats).
    pub beat_jump_bars: bool,
    /// Recordbox extras in the player: the STEMS button and the phrase bar (the vocal layer
    /// is `show_vocal`).
    pub show_stems: bool,
    pub show_phrase_bar: bool,
    /// Analysis Mode: 0 = Normal (one tempo), 1 = Dynamic (the grid follows tempo changes).
    pub analysis_mode: u8,
    /// Maximum CPU workers for library analysis. Zero selects the automatic budget.
    pub analysis_threads: usize,
    /// A key already written in the file's tag (for example by Mixed In Key) is kept when the
    /// track is first analysed; Analyze Key still replaces it.
    pub keep_tag_key: bool,
    // ---- Preferences pages added from the rekordbox 7.2.19 references (ui_prefs.rs) ----
    /// View > Layout > Browser panel: "Playlist Palette" (the palette and its switch exist at all;
    /// `palette_open` is whether it is showing).
    pub palette_enabled: bool,
    /// View > Layout > Browser panel: "Playlist search filter" (the search tile of the playlist tree).
    pub playlist_search_filter: bool,
    /// View > Layout > Browser panel: "Display All Tracks in the Playlists".
    pub all_tracks_in_playlists: bool,
    /// View > Layout > Browser panel: "Display the number of tracks in a playlist on the Tree View".
    pub playlist_track_counts: bool,
    /// View > Layout > Phrases: "Phrase (Enlarged Waveform)" and "Always show types of phrases"
    /// ("Phrase (Full Waveform)" is `show_phrase_bar`).
    pub phrase_enlarged: bool,
    pub phrase_types_always: bool,
    /// View > Layout > Vocal: "Vocal (Enlarged Waveform)" ("Vocal (Full Waveform)" is `show_vocal`).
    pub vocal_enlarged: bool,
    /// Audio > Metronome: click sound 0–2 ("Click Sound 01".."03").
    pub metronome_sound: u8,
    /// Audio > Preview Volume, 0..1; the middle (0.5) plays previews at the master volume.
    pub preview_volume: f32,
    /// STEMS > ACTIVE STEM Setting: false = MUTE (a part button mutes its part), true = SOLO
    /// (a part button plays that part alone). See `prefs_ext::stem_part_click`.
    pub stem_solo: bool,
    /// STEMS: "Reset the MUTE/SOLO state while loading".
    pub stem_reset_on_load: bool,
    /// STEMS > STEM Waveform Display: the waveform shows only the parts that sound.
    pub stem_waveform: bool,
    /// The settings of rekordbox's Preferences that came with the full window (prefs_rb.rs).
    #[serde(default = "crate::prefs_rb::legacy_library_preferences")]
    pub rb: crate::prefs_rb::RbPrefs,
}

/// Keyboard actions: (id, label, default shortcut in stored form or "" for none, category).
/// The table lives in prefs_ext.rs.
pub use crate::prefs_ext::ACTIONS;

/// A key with modifiers, stored as "Ctrl+Alt+Shift+Cmd+Key" (any subset, in that order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub key: egui::Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

impl Shortcut {
    pub fn parse(s: &str) -> Option<Shortcut> {
        let mut sc = Shortcut { key: egui::Key::Space, ctrl: false, alt: false, shift: false, cmd: false };
        let mut parts: Vec<&str> = s.split('+').collect();
        // "Shift++" style is not used: the key name is always the last part.
        let key = parts.pop()?;
        for m in parts {
            match m {
                "Ctrl" => sc.ctrl = true,
                "Alt" => sc.alt = true,
                "Shift" => sc.shift = true,
                "Cmd" => sc.cmd = true,
                _ => return None,
            }
        }
        sc.key = egui::Key::from_name(key)?;
        Some(sc)
    }
    pub fn from_event(key: egui::Key, m: &egui::Modifiers) -> Shortcut {
        Shortcut { key, ctrl: m.ctrl && !m.mac_cmd, alt: m.alt, shift: m.shift, cmd: m.mac_cmd || (m.command && !m.ctrl) }
    }
    /// Stored form.
    pub fn name(&self) -> String {
        let mut s = String::new();
        for (on, n) in [(self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+"), (self.cmd, "Cmd+")] {
            if on {
                s.push_str(n);
            }
        }
        s.push_str(self.key.name());
        s
    }
    /// As shown in menus on macOS: ⌃⌥⇧⌘ then the key.
    pub fn label(&self) -> String {
        let mut s = String::new();
        for (on, n) in [(self.ctrl, "⌃"), (self.alt, "⌥"), (self.shift, "⇧"), (self.cmd, "⌘")] {
            if on {
                s.push_str(n);
            }
        }
        s.push_str(self.key.name());
        s
    }
    /// The held modifiers are exactly this shortcut's. `free_shift`: Shift may be added
    /// (beat jumps use it for a one-beat jump).
    pub fn mods_match(&self, m: &egui::Modifiers, free_shift: bool) -> bool {
        let o = Shortcut::from_event(self.key, m);
        o.ctrl == self.ctrl && o.alt == self.alt && o.cmd == self.cmd && (o.shift == self.shift || (free_shift && !self.shift))
    }
}

impl Prefs {
    /// Snapshot the settings shared by manual analysis, automatic analysis and device export.
    pub fn analysis_settings(&self) -> AnalysisSettings {
        let (lo, hi) = BPM_RANGES[self.bpm_range.min(BPM_RANGES.len() - 1)];
        AnalysisSettings {
            bpm_min: lo,
            bpm_max: hi,
            do_grid: self.analyze_grid,
            use_recordbox_estimator: self.use_recordbox_estimator,
            high_precision: self.refine_beatgrid_transients,
            model_grid_mode: crate::analysis::rb_hp::mode_from_preferences(self.use_installed_beatgrid_model, self.analysis_mode),
            dynamic: self.analysis_mode == 1,
            vocal: self.vocal_analysis,
            do_key: self.analyze_key,
            target_lufs: self.target_lufs,
        }
    }

    /// The shortcut assigned to an action (the user's choice, else the default).
    pub fn shortcut_for(&self, action: &str) -> Option<Shortcut> {
        let name = self.keymap.get(action).cloned().or_else(|| ACTIONS.iter().find(|a| a.0 == action).map(|a| a.2.to_string()))?;
        Shortcut::parse(&name)
    }
    /// The key of an action's shortcut (releases end previews whatever modifiers are held).
    pub fn key_for(&self, action: &str) -> Option<egui::Key> {
        self.shortcut_for(action).map(|s| s.key)
    }
}

/// Related Tracks rules (the original offers Same Key / Related Key 1–3, BPM range and more).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelatedRules {
    /// 0 off, 1 Same Key, 2 Related Key 1, 3 Related Key 2, 4 Related Key 3.
    pub key: u8,
    pub bpm: bool,
    /// Allowed BPM difference in percent (half and double tempo also count).
    pub bpm_pct: f64,
    pub genre: bool,
    pub artist: bool,
    pub label: bool,
    pub color: bool,
    pub rating: bool,
    /// Only tracks registered as a good match of the source track ("Matching").
    pub matching: bool,
}

impl Default for RelatedRules {
    fn default() -> Self {
        RelatedRules { key: 2, bpm: true, bpm_pct: 6.0, genre: false, artist: false, label: false, color: false, rating: false, matching: false }
    }
}

/// An explicit reanalysis can replace the grid it saw when queued. Edits made
/// while its audio was being analyzed take precedence over the worker's result.
fn keep_saved_grid(locked: bool, saved: &Option<BeatGrid>, queued: Option<&Option<BeatGrid>>, measured: bool) -> bool {
    locked || match queued {
        Some(before) => before != saved || !measured,
        None => saved.is_some(),
    }
}

#[cfg(test)]
mod reanalysis_tests {
    use super::{keep_saved_grid, BeatGrid};

    #[test]
    fn replacing_a_saved_grid_requires_a_successful_explicit_analysis_and_no_intervening_edit() {
        let imported = Some(BeatGrid::constant(0.125, 122.0, 2));
        let edited = Some(BeatGrid::constant(0.181, 122.0, 1));
        assert!(keep_saved_grid(false, &imported, None, true));
        assert!(keep_saved_grid(false, &imported, Some(&imported), false));
        assert!(keep_saved_grid(true, &imported, Some(&imported), true));
        assert!(keep_saved_grid(false, &edited, Some(&imported), true));
        assert!(keep_saved_grid(false, &edited, Some(&None), true));
        assert!(keep_saved_grid(false, &None, Some(&imported), true));
        assert!(!keep_saved_grid(false, &imported, Some(&imported), true));
        assert!(!keep_saved_grid(false, &None, None, true));
    }
}

#[cfg(test)]
mod prefs_tests {
    fn cache_key(track: i64, model: &'static str) -> super::StemCacheKey {
        super::StemCacheKey { track, model, path: "example.stems".into() }
    }

    #[test]
    fn stem_storage_preferences_preserve_legacy_budgets_and_opt_in() {
        let default: super::Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(default.stem_cache_limit(), Some(20_000_000_000));
        for gb in [0, 7, u32::MAX] {
            let mut prefs: super::Prefs = serde_json::from_value(serde_json::json!({"stem_cache_gb": gb})).unwrap();
            assert_eq!(prefs.stem_cache_limit(), Some(u64::from(gb) * 1_000_000_000));
            prefs.keep_all_stems = true;
            let saved: super::Prefs = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
            assert_eq!(saved.stem_cache_limit(), None);
            assert_eq!(saved.stem_cache_gb, gb);
        }
    }

    #[test]
    fn a_running_stem_job_uses_the_budget_in_effect_after_its_save() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("000000000000000000000001-htdemucs.stems");
        let new = dir.path().join("000000000000000000000001-htdemucs_ft.stems");
        std::fs::write(&old, b"old").unwrap();
        std::fs::write(&new, b"new").unwrap();
        let mut prefs = super::Prefs { stem_cache_gb: 0, ..Default::default() };
        let state = super::StemState::new();
        state.update_cache_limit(&prefs);
        let worker_limit = state.cache_limit.clone();
        let finish_save = || crate::stems::trim_cache(&new, || *worker_limit.lock());

        prefs.keep_all_stems = true;
        state.update_cache_limit(&prefs);
        finish_save();
        assert!(old.exists() && new.exists());

        prefs.keep_all_stems = false;
        prefs.stem_cache_gb = 1;
        state.update_cache_limit(&prefs);
        finish_save();
        assert!(old.exists() && new.exists());

        prefs.stem_cache_gb = 0;
        state.update_cache_limit(&prefs);
        assert!(old.exists(), "changing the budget must not run eviction");
        finish_save();
        assert!(!old.exists());
        assert!(new.exists(), "even a zero budget keeps the result just saved");
    }

    #[test]
    fn cache_save_warning_survives_unload_until_the_same_parts_are_saved() {
        let mut state = super::StemState::new();
        let key = cache_key(7, "htdemucs");
        state.record_cache_write(key.clone(), Err("Disk full".into()));
        state.retain_deck_tracks([None; 4]);
        state.record_cache_write(cache_key(8, "htdemucs"), Ok(()));
        state.record_cache_write(cache_key(7, "htdemucs_ft"), Ok(()));
        assert_eq!(state.cache_warning(), Some("Disk full"));
        state.record_cache_write(key, Ok(()));
        assert!(state.cache_warning().is_none());
    }

    #[test]
    fn startup_import_requires_an_opt_in_after_upgrading() {
        for legacy in [r#"{}"#, r#"{"rekordbox_sync":true}"#, r#"{"rekordbox_sync":false}"#] {
            let prefs: super::Prefs = serde_json::from_str(legacy).unwrap();
            assert!(!prefs.import_rekordbox_at_startup);
        }
        let mut prefs = super::Prefs::default();
        assert!(!prefs.import_rekordbox_at_startup);
        prefs.import_rekordbox_at_startup = true;
        let saved = serde_json::to_string(&prefs).unwrap();
        assert!(serde_json::from_str::<super::Prefs>(&saved).unwrap().import_rekordbox_at_startup);
    }

    #[test]
    fn the_old_inactive_precision_switch_does_not_opt_users_into_new_work() {
        let old: super::Prefs = serde_json::from_str(r#"{"high_precision_grid":true}"#).unwrap();
        assert!(!old.refine_beatgrid_transients);
        assert!(!old.use_installed_beatgrid_model);
        let saved: super::Prefs = serde_json::from_str(r#"{"refine_beatgrid_transients":true}"#).unwrap();
        assert!(saved.refine_beatgrid_transients);
    }

    #[test]
    fn analysis_jobs_snapshot_the_saved_estimator_independently_of_mode_and_refinement() {
        let old: super::Prefs = serde_json::from_str(r#"{"analysis_mode":1,"refine_beatgrid_transients":true}"#).unwrap();
        let job = old.analysis_settings();
        assert!(!job.use_recordbox_estimator && job.dynamic && job.high_precision);
        let defaults = super::Prefs::default().analysis_settings();
        assert!(!defaults.use_recordbox_estimator && !defaults.dynamic && !defaults.high_precision);

        for mode in [0, 1] {
            for classic in [false, true] {
                let prefs = super::Prefs { analysis_mode: mode, use_recordbox_estimator: classic, use_installed_beatgrid_model: true, ..super::Prefs::default() };
                let saved: super::Prefs = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
                let job = saved.analysis_settings();
                assert_eq!(job.use_recordbox_estimator, classic);
                assert_eq!(job.dynamic, mode == 1);
                assert!(!job.high_precision);
                assert!(saved.use_installed_beatgrid_model);
                assert_eq!(job.model_grid_mode, crate::analysis::rb_hp::mode_from_preferences(true, mode));
            }
        }
    }

    #[test]
    fn cpu_retry_belongs_to_the_deck_with_the_provider_failure() {
        let mut state = super::StemState::new();
        let failure = anyhow::Error::new(crate::stems::DirectMlFailure("provider HRESULT".into())).context("Separation segment 2 of 4");
        state.errors.insert(Some(7), super::StemError::separation(Some("htdemucs_ft"), failure));
        let problem = state.error_for(Some(7)).unwrap();
        assert_eq!(problem.retry_model, Some("htdemucs_ft"));
        state.model = Some("htdemucs".into());
        use crate::stems::SeparationDevice::{Cpu, Selected};
        assert_eq!(state.manual_device(Some(7), false), Cpu);
        assert_eq!(state.manual_device(Some(8), false), Selected);
        assert_eq!(state.manual_device(Some(8), true), Cpu);
        assert_eq!(state.model_for(Some(7), crate::stems::SeparationDevice::Cpu).unwrap().id, "htdemucs_ft");
        assert_eq!(state.model_for(Some(7), crate::stems::SeparationDevice::Selected).unwrap().id, "htdemucs");
        let problem = state.error_for(Some(7)).unwrap();
        assert!(problem.detail.contains("provider HRESULT") && problem.detail.contains("2 of 4"));
        assert!(state.error_for(Some(8)).is_none() && state.error_for(None).is_none());
        for reason in ["stopped", "invalid checkpoint", "provider HRESULT"] {
            state.errors.insert(Some(8), super::StemError::separation(Some("htdemucs"), anyhow::anyhow!(reason)));
            assert!(state.error_for(Some(8)).unwrap().retry_model.is_none());
            assert!(state.error_for(Some(7)).unwrap().retry_model.is_some());
        }
        state.retain_deck_tracks([Some(8), None, None, None]);
        assert!(state.error_for(Some(7)).is_none());
        assert!(state.error_for(Some(8)).is_some());
    }

    #[cfg(all(target_os = "linux", feature = "vulkan"))]
    #[test]
    fn vulkan_dispatch_failure_offers_cpu_retry_with_the_same_model() {
        let failure = anyhow::Error::new(crate::stems::VulkanFailure("Vulkan dispatch timed out".into()))
            .context("Separation segment 1 of 2");
        let error = super::StemError::separation(Some("htdemucs"), failure);
        assert_eq!(error.retry_model, Some("htdemucs"));
        assert!(error.detail.contains("Vulkan dispatch timed out"));
    }

    #[test]
    fn manual_cpu_progress_identifies_startup_before_any_segment_finishes() {
        let mut state = super::StemState::new();
        state.execution = crate::stems::SeparationDevice::Cpu;
        state.active_key = Some(cache_key(7, "htdemucs"));
        assert!(state.separation_status().is_none());
        state.separating = Some((7, 0.0));
        assert_eq!(state.separation_status().as_deref(), Some("Starting HTDemucs on CPU · waiting for the first segment"));
        state.separating = Some((7, 0.25));
        assert_eq!(state.separation_status().as_deref(), Some("Separating with HTDemucs on CPU: 25%"));
    }

    #[test]
    fn model_details_follow_the_running_job_when_the_preference_changes() {
        let mut state = super::StemState::new();
        state.active_key = Some(cache_key(7, "htdemucs"));
        state.model = Some("htdemucs_ft".into());
        assert_eq!(state.status_model().unwrap().name, "HTDemucs");
        state.active_key = None;
        assert_eq!(state.status_model().unwrap().name, "HTDemucs fine-tuned");
    }

    #[test]
    fn legacy_preferences_require_opt_in_for_automatic_cpu_separation() {
        let mut p: super::Prefs = serde_json::from_str(r#"{"show_stems":true,"stems_auto_separate":true}"#).unwrap();
        assert!(!p.auto_stems_on_load(false));
        assert_eq!(p.auto_stems_on_load(true), !super::MANUAL_STEMS_PREVIEW);
        p.stems_auto_cpu = true;
        assert_eq!(p.auto_stems_on_load(false), !super::MANUAL_STEMS_PREVIEW);
        p.stems_auto_separate = false;
        assert!(!p.auto_stems_on_load(false));
        assert!(!p.auto_stems_on_load(true));
        p.show_stems = false;
        p.stems_auto_separate = true;
        assert!(!p.auto_stems_on_load(false));
    }

    #[test]
    fn separation_is_cancelled_when_the_last_deck_releases_its_track() {
        use std::sync::atomic::Ordering;
        let mut state = super::StemState::new();
        state.separating = Some((4, 0.25));
        state.pending = Some((cache_key(9, "htdemucs"), crate::stems::SeparationDevice::Cpu));
        state.retain_deck_tracks([None, Some(9), None, Some(4)]);
        assert!(!state.cancel.load(Ordering::Relaxed));
        state.retain_deck_tracks([None, Some(9), None, None]);
        assert!(state.cancel.load(Ordering::Relaxed));
        assert_eq!(state.pending, Some((cache_key(9, "htdemucs"), crate::stems::SeparationDevice::Cpu)));
        assert!(state.separating.is_some(), "another model must wait until the cancelled worker exits");
        state.retain_deck_tracks([None; 4]);
        assert!(state.pending.is_none());
        assert!(!state.download_cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn unknown_settings_survive_a_save() {
        let json = r#"{"tooltips": false, "future_setting": {"a": 1}, "another": [1, 2]}"#;
        let p: super::Prefs = serde_json::from_str(json).unwrap();
        assert!(!p.tooltips);
        let back: serde_json::Value = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back["future_setting"]["a"], 1);
        assert_eq!(back["another"][1], 2);
    }
}

#[cfg(test)]
mod relocation_tests {
    #[test]
    fn picks_only_unambiguous_files() {
        use super::RelocationMatch::*;
        use std::path::Path;
        let a = Path::new("/a/x.mp3");
        let b = Path::new("/b/x.mp3");
        assert_eq!(super::pick_relocation(&[(a, Some(100))], 100), Found(a, 100));
        assert_eq!(super::pick_relocation(&[(a, Some(99))], 100), SizeMismatch);
        assert_eq!(super::pick_relocation(&[(a, Some(99))], 0), Found(a, 99));
        assert_eq!(super::pick_relocation(&[(a, Some(99)), (b, Some(100))], 100), Found(b, 100));
        assert_eq!(super::pick_relocation(&[(a, Some(100)), (b, Some(100))], 100), Multiple);
        assert_eq!(super::pick_relocation(&[(a, Some(99)), (b, Some(98))], 100), SizeMismatch);
        assert_eq!(super::pick_relocation(&[(a, Some(99)), (b, Some(98))], 0), Multiple);
        assert_eq!(super::pick_relocation(&[(a, None)], 0), Unverified);
        assert_eq!(super::pick_relocation(&[(a, None), (b, Some(100))], 100), Unverified);
    }
}

#[cfg(test)]
mod lufs_migration_tests {
    #[test]
    fn shifts_library_and_trash_once() {
        let dir = std::env::temp_dir().join(format!("recordbox_lufs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("library.db")).unwrap();
        let mk = |p: &str, g: f32| crate::model::Track { path: p.into(), analyzed: 1, gain_db: g, ..Default::default() };
        let a = db.insert_track(&mk("/a.mp3", 3.0)).unwrap();
        let b = db.insert_track(&mk("/b.mp3", 1.5)).unwrap();
        db.trash_track(b).unwrap();
        let mut prefs = super::Prefs::default();
        prefs.target_lufs = -8.0;
        let next = super::migrate_lufs_default(&db, &prefs, None).unwrap();
        assert_eq!(next.target_lufs, -10.0);
        assert!((db.track(a).unwrap().gain_db - 1.0).abs() < 1e-6);
        let t = db.trash_list().unwrap()[0].0;
        let back = db.restore_trash(t).unwrap().unwrap();
        assert!((db.track(back).unwrap().gain_db - (-0.5)).abs() < 1e-6, "trashed gain shifted too");
        assert_eq!(db.setting("lufs_default_migrated").as_deref(), Some("1"));
        // A user already on -10 (or a new library) gets no shift.
        let c = db.insert_track(&mk("/c.mp3", 2.0)).unwrap();
        let _ = super::migrate_lufs_default(&db, &next, None).unwrap();
        assert!((db.track(c).unwrap().gain_db - 2.0).abs() < 1e-6);
        // Unparsable preferences: the old target is still found in the raw text.
        let d = db.insert_track(&mk("/d.mp3", 2.0)).unwrap();
        let fallback = super::Prefs::default();
        let n2 = super::migrate_lufs_default(&db, &fallback, Some(r#"{"target_lufs": -8.0, "broken": "#)).unwrap_or(fallback.clone());
        let _ = n2;
        let raw_ok = super::migrate_lufs_default(&db, &fallback, Some(r#"{"target_lufs": -8.0}"#)).unwrap();
        assert_eq!(raw_ok.target_lufs, -10.0);
        assert!((db.track(d).unwrap().gain_db - 0.0).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod history_key_tests {
    use std::collections::HashSet;
    #[test]
    fn keys_and_legacy_migration() {
        // Same drive and history names on two sticks: different first files, different keys.
        assert_ne!(super::history_key("NO NAME", "HISTORY 001", "/Contents/a.mp3"), super::history_key("NO NAME", "HISTORY 001", "/Contents/b.mp3"));
        // An earlier version's key with entries [3, 5] counts as 2 imported when the history now
        // starts with them; a different history with the same name does not match.
        let old: HashSet<String> = ["NO NAME\u{1f}HISTORY 001\u{1f}[3, 5]".to_string()].into_iter().collect();
        assert_eq!(super::legacy_imported(&old, "NO NAME", "HISTORY 001", &[3, 5, 9]), Some(2));
        assert_eq!(super::legacy_imported(&old, "NO NAME", "HISTORY 001", &[4, 5, 9]), None);
        assert_eq!(super::legacy_imported(&old, "NO NAME", "HISTORY 002", &[3, 5]), None);
    }
}

#[cfg(test)]
mod quantize_tests {
    use crate::model::Beat;
    #[test]
    fn snaps_to_beat_divisions() {
        let beats: Vec<Beat> = (0..8).map(|i| Beat { time: i as f64 * 0.5, bpm: 120.0, beat: (i % 4 + 1) as u8, index: i }).collect();
        assert!((super::quantize_to(&beats, 1.12, 1) - 1.0).abs() < 1e-9);
        assert!((super::quantize_to(&beats, 1.2, 2) - 1.25).abs() < 1e-9);
        assert!((super::quantize_to(&beats, 1.06, 4) - 1.0).abs() < 1e-9);
        assert!((super::quantize_to(&beats, 1.07, 8) - 1.0625).abs() < 1e-9);
        // Past the last beat the last interval continues.
        assert!((super::quantize_to(&beats, 3.8, 1) - 4.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod grid_jump_tests {
    use crate::model::Beat;
    #[test]
    fn follows_tempo_changes() {
        // 4 beats at 0.5 s, then 4 at 0.25 s.
        let times = [0.0, 0.5, 1.0, 1.5, 2.0, 2.25, 2.5, 2.75, 3.0];
        let beats: Vec<Beat> = times.iter().enumerate().map(|(i, t)| Beat { time: *t, bpm: 120.0, beat: (i % 4 + 1) as u8, index: i }).collect();
        // From halfway through the beat at 1.0 s, 4 beats on lands halfway through the one at 2.5 s.
        assert!((super::grid_jump(&beats, 1.25, 4.0).unwrap() - 2.625).abs() < 1e-9);
        assert!((super::grid_jump(&beats, 2.625, -4.0).unwrap() - 1.25).abs() < 1e-9);
        // Past the end the last interval continues.
        assert!((super::grid_jump(&beats, 2.75, 2.0).unwrap() - 3.25).abs() < 1e-9);
        assert!(super::grid_jump(&beats, 1.0, 0.5).is_none());
    }
}

#[cfg(test)]
mod pvdi_tests {
    #[test]
    fn reads_vocal_levels() {
        let mut s = b"PVDI".to_vec();
        s.extend_from_slice(&24u32.to_be_bytes());
        s.extend_from_slice(&28u32.to_be_bytes());
        for w in [0u16, 1024, 22050, 1] {
            s.extend_from_slice(&w.to_be_bytes());
        }
        s.extend_from_slice(&4u32.to_be_bytes());
        s.extend_from_slice(&[0, 2, 4, 1]);
        let v = super::parse_pvdi(&s).unwrap();
        assert!((v.0 - 1024.0 / 22050.0).abs() < 1e-9);
        assert_eq!(v.1, vec![0, 2, 4, 1]);
        // Scrambled (negative discriminator) and truncated sections are refused.
        let mut neg = s.clone();
        neg[12] = 0xff;
        assert!(super::parse_pvdi(&neg).is_none());
        assert!(super::parse_pvdi(&s[..26]).is_none());
    }

    #[test]
    fn own_vocal_section_reads_back() {
        let levels = vec![0u8, 0, 1, 3, 4, 4, 2, 0];
        let sec = crate::export::anlz::pvdi(&levels);
        let v = super::parse_pvdi(&sec).unwrap();
        assert_eq!(v.1, levels);
        assert!((v.0 - crate::stems::VOCAL_HOP as f64 / 44100.0).abs() < 1e-9);
        // Silence has no vocals; a loud vocal over a quiet mix does.
        let quiet = vec![0f32; 2048 * 100];
        assert!(crate::stems::vocal_levels(&quiet, &quiet).iter().all(|l| *l == 0));
        let voc: Vec<f32> = (0..2048 * 100).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect();
        let mix: Vec<f32> = voc.iter().map(|x| x * 1.2).collect();
        assert!(crate::stems::vocal_levels(&voc, &mix).iter().all(|l| *l >= 3));
    }
}

#[cfg(test)]
mod keymap_tests {
    #[test]
    fn default_and_custom_keys() {
        let mut p = super::Prefs::default();
        for (id, _, def, _) in super::ACTIONS {
            assert!(def.is_empty() || p.key_for(id).is_some(), "{id}: default key {def} is not a key name");
        }
        assert_eq!(p.key_for("play"), Some(eframe::egui::Key::Space));
        p.keymap.insert("play".into(), "P".into());
        assert_eq!(p.key_for("play"), Some(eframe::egui::Key::P));
        p.keymap.insert("cue".into(), String::new());
        assert_eq!(p.key_for("cue"), None);
        // Modifiers.
        p.keymap.insert("memory".into(), "Alt+Shift+M".into());
        let sc = p.shortcut_for("memory").unwrap();
        assert_eq!((sc.key, sc.alt, sc.shift, sc.ctrl, sc.cmd), (eframe::egui::Key::M, true, true, false, false));
        assert_eq!(sc.name(), "Alt+Shift+M");
        assert_eq!(sc.label(), "⌥⇧M");
        let m = eframe::egui::Modifiers { alt: true, shift: true, ..Default::default() };
        assert!(sc.mods_match(&m, false));
        assert!(!sc.mods_match(&eframe::egui::Modifiers::NONE, false));
        let left = p.shortcut_for("jump_back").unwrap();
        assert!(left.mods_match(&eframe::egui::Modifiers::SHIFT, true));
        assert!(!left.mods_match(&eframe::egui::Modifiers::SHIFT, false));
        assert!(super::Shortcut::parse("Hyper+K").is_none());
    }
}

#[cfg(test)]
mod related_tests {
    #[test]
    fn key_wheel_distance() {
        // 8A (Am, index 7) vs 8A, 9A, 8B (C), 9B, 10A, 2A
        assert_eq!(super::key_distance(7, 7), Some(0));
        assert_eq!(super::key_distance(7, 8), Some(1));
        assert_eq!(super::key_distance(7, 19), Some(1));
        assert_eq!(super::key_distance(7, 20), Some(2));
        assert_eq!(super::key_distance(7, 9), Some(2));
        assert_eq!(super::key_distance(7, 1), Some(6));
        // 12A and 1A are neighbours on the wheel
        assert_eq!(super::key_distance(11, 0), Some(1));
        assert_eq!(super::key_distance(-1, 3), None);
    }
}

/// Reuse the original ANLZ beat sections only when the saved anchors still
/// match the import. Allow floating-point round trips through the library.
/// Sub-millisecond timing and fractional BPM edits belong to the saved grid.
pub fn grids_match(a: &BeatGrid, b: &BeatGrid) -> bool {
    a.anchors.len() == b.anchors.len() && a.anchors.iter().zip(&b.anchors).all(|(x, y)| {
        x.beat == y.beat && (x.time - y.time).abs() <= 1e-9 && (x.bpm - y.bpm).abs() <= 1e-9
    })
}

/// Distance between keys around the wheel, plus one when the mode differs.
pub fn key_distance(a: i32, b: i32) -> Option<i32> {
    if !(0..24).contains(&a) || !(0..24).contains(&b) {
        return None;
    }
    let d = (a % 12 - b % 12).rem_euclid(12);
    Some(d.min(12 - d) + i32::from(a / 12 != b / 12))
}

/// Search field choices, in the order of the search box's menu.
pub const SEARCH_SCOPES: [&str; 8] = ["All", "Track Title", "Artist", "Album", "Genre", "Label", "Comments", "My Tag"];

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            key_alnum: false,
            bpm_range: 0,
            analyze_grid: true,
            analyze_key: true,
            auto_analysis: true,
            wave_color: 2,
            quantize: true,
            time_remain: true,
            beat_jump: 4.0,
            loop_beats: 4.0,
            loop_auto: true,
            zoom: 2,
            columns: vec![Col::Status, Col::Preview, Col::Artwork, Col::Title, Col::Artist, Col::Album, Col::Genre, Col::Bpm, Col::Rating, Col::Time, Col::Key, Col::DateAdded],
            sort_col: Col::DateAdded,
            sort_asc: false,
            import_rekordbox_at_startup: false,
            usb_settings: Default::default(),
            search_scope: 0,
            related: Default::default(),
            usb_browse: Default::default(),
            stem_model: None,
            stems_auto_download: true,
            deck_load_lock: false,
            instant_doubles: false,
            perf_fx: false,
            fx: Default::default(),
            perf_four: false,
            perf_lay: Default::default(),
            perf_sampler: false,
            sampler: Default::default(),
            traffic: Default::default(),
            deck: Default::default(),
            groove: Default::default(),
            midi: Default::default(),
            rec: Default::default(),
            pads: Default::default(),
            mixx: Default::default(),
            app_mode: 0,
            edit: Default::default(),
            eq_isolator: false,
            ch_fader_curve: 1,
            stems_auto_separate: true,
            stems_auto_cpu: false,
            stems_progressive_preview: false,
            stem_cache_gb: 20,
            keep_all_stems: false,
            import_device_history: true,
            keymap: Default::default(),
            column_browser: false,
            audio_device: None,
            stem_parts: 4,
            auto_first_downbeat_cue: true,
            quantize_div: 1,
            auto_gain_peak_cap: false,
            other: Default::default(),
            sub_browser: false,
            browse_font: 0,
            browse_line: 0,
            browse_bold: false,
            center_selected: false,
            show_vocal: true,
            vocal_analysis: true,
            columns_other: vec![Col::Num, Col::Status, Col::Preview, Col::Artwork, Col::Title, Col::Artist, Col::Genre, Col::Bpm, Col::Time, Col::Key, Col::DateAdded],
            sort_other: (Col::Num, true),
            columns_for_playlist: false,
            tree_items: Default::default(),
            mytag_to_comments: false,
            cue_markers_on_preview: true,
            wave_half: false,
            color_names: crate::prefs_ext::default_color_names(),
            use_recordbox_estimator: false,
            refine_beatgrid_transients: false,
            use_installed_beatgrid_model: false,
            memory_cue_first_beat: false,
            audio_buffer: 0,
            audio_rate: 0,
            audio_channel: 0,
            hp_channel: None,
            device_bg_color: 0,
            delete_history_after_import: false,
            device_import_dir: String::new(),
            hot_cue_auto_load_on_import: true,
            phrase_analysis: true,
            relocate_folders: vec![],
            itunes_xml: None,
            rekordbox_xml_path: None,
            artwork_full: false,
            palette: vec![0; 4],
            palette_open: false,
            shortcuts: vec![],
            shortcuts_open: true,
            info_panel: false,
            related_lists: vec![],
            related_sel: 0,
            bx: Default::default(),
            beat_count_mode: 0,
            wave_click_play_cue: false,
            player_layout: 0,
            playback_fps: if cfg!(target_os = "linux") { 30 } else { 0 },
            metronome_vol: 2,
            beat_jump_bars: false,
            show_stems: true,
            // As rekordbox: "Phrase (Full Waveform)" is not ticked until the user ticks it.
            show_phrase_bar: false,
            analysis_mode: 0,
            analysis_threads: 0,
            keep_tag_key: true,
            palette_enabled: true,
            playlist_search_filter: true,
            all_tracks_in_playlists: true,
            playlist_track_counts: true,
            phrase_enlarged: false,
            phrase_types_always: true,
            vocal_enlarged: false,
            metronome_sound: 0,
            preview_volume: 0.5,
            rb: Default::default(),
            stem_solo: false,
            stem_reset_on_load: true,
            stem_waveform: true,
            delete_device_tracks: false,
            // rekordbox's standard: gain = -10 - integrated loudness.
            target_lufs: -10.0,
            tooltips: true,
            show_mytag: false,
            grid_from_current: false,
            devices_extra: vec![],
            widths: HashMap::new(),
            browser_only: false,
        }
    }
}

/// Seconds shown across the enlarged waveform; the default (index 2) matches the original's default
/// zoom, measured at about 21 beats from the playhead to the right edge at 126 BPM.
/// One undoable library edit: the state before it.
#[derive(Clone)]
pub enum Edit {
    Cues { track: i64, before: Vec<Cue> },
    Track { before: Track },
    Grid { track: i64, before: BeatGrid },
    /// PHRASE EDIT: the track's edited phrases before the change (None: no edits yet).
    Phrases { track: i64, before: Option<crate::phrase::Structure> },
}

#[derive(Debug)]
pub struct StemError {
    pub detail: String,
    pub retry_model: Option<&'static str>,
}

impl StemError {
    fn separation(model: Option<&'static str>, error: anyhow::Error) -> Self {
        let provider_failure = error.is::<crate::stems::DirectMlFailure>();
        #[cfg(all(target_os = "linux", feature = "vulkan"))]
        let provider_failure = provider_failure || error.is::<crate::stems::VulkanFailure>();
        Self { retry_model: model.filter(|_| provider_failure),
            detail: format!("Stem separation failed: {error:#}") }
    }
}

pub enum StemMsg {
    Hardware(crate::stems::Hardware, String),
    Download(u64, u64),
    DownloadDone(Result<(), String>),
    Progress(i64, f32),
    Preview(StemCacheKey, Arc<crate::stems::Live>),
    CacheWrite(StemCacheKey, Result<(), String>),
    Cached(StemCacheKey, anyhow::Result<Arc<crate::stems::Stems>>),
    Done(i64, anyhow::Result<std::sync::Arc<crate::stems::Stems>>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StemCacheKey {
    track: i64,
    model: &'static str,
    path: std::path::PathBuf,
}

/// Stem separation status: hardware, model, download and the current separation.
pub struct StemState {
    pub hw: Option<crate::stems::Hardware>,
    pub model: Option<String>,
    pub model_ready: bool,
    pub downloading: Option<(u64, u64)>,
    pub separating: Option<(i64, f32)>,
    // An explicit request can arrive while a deck is already reading the same cache file.
    // Only that request may start separation after the read fails in the manual preview.
    loading: HashMap<StemCacheKey, Option<crate::stems::SeparationDevice>>,
    // None is a shared model download error. Deck errors and retries belong to their track.
    errors: HashMap<Option<i64>, StemError>,
    last_cache_warning: Option<(StemCacheKey, String)>,
    // Read by the worker after saving and during eviction. Preferences may change during inference.
    cache_limit: Arc<parking_lot::Mutex<Option<u64>>>,
    pub(crate) cache_usage: Option<Arc<std::sync::OnceLock<std::io::Result<u64>>>>,
    active_key: Option<StemCacheKey>,
    preview: Option<Arc<crate::stems::Live>>,
    pub execution: crate::stems::SeparationDevice,
    /// Track to separate as soon as the model download finishes.
    pending: Option<(StemCacheKey, crate::stems::SeparationDevice)>,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    pub download_cancel: Arc<std::sync::atomic::AtomicBool>,
    pub tx: crossbeam_channel::Sender<StemMsg>,
    pub rx: crossbeam_channel::Receiver<StemMsg>,
}

impl StemState {
    fn update_cache_limit(&self, prefs: &Prefs) {
        *self.cache_limit.lock() = prefs.stem_cache_limit();
    }

    pub(crate) fn cache_usage(&mut self, ctx: &egui::Context) -> Option<&std::io::Result<u64>> {
        let usage = self.cache_usage.get_or_insert_with(|| {
            let slot = Arc::new(std::sync::OnceLock::new());
            let result = slot.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = result.set(crate::stems::cache_size());
                ctx.request_repaint();
            });
            slot
        });
        usage.get()
    }

    pub(crate) fn reading_cache(&self) -> bool {
        !self.loading.is_empty()
    }

    fn gpu_available(&self) -> bool {
        self.hw.is_some() && crate::stems::backend_status().0
    }

    pub fn error_for(&self, track: Option<i64>) -> Option<&StemError> {
        self.errors.get(&track).or_else(|| self.errors.get(&None))
    }

    pub fn cache_warning(&self) -> Option<&str> {
        self.last_cache_warning.as_ref().map(|(_, warning)| warning.as_str())
    }

    fn record_cache_write(&mut self, key: StemCacheKey, result: Result<(), String>) {
        if result.is_ok() {
            self.cache_usage = None;
        }
        match result {
            Err(warning) => self.last_cache_warning = Some((key, warning)),
            Ok(()) if self.last_cache_warning.as_ref().is_some_and(|(failed, _)| failed == &key) => {
                self.last_cache_warning = None;
            }
            Ok(()) => {}
        }
    }

    fn manual_device(&self, track: Option<i64>, gpu_paused: bool) -> crate::stems::SeparationDevice {
        if gpu_paused || self.error_for(track).is_some_and(|e| e.retry_model.is_some()) {
            crate::stems::SeparationDevice::Cpu
        } else {
            crate::stems::SeparationDevice::Selected
        }
    }

    pub fn status_model(&self) -> Option<&'static crate::stems::ModelSpec> {
        self.active_key.as_ref().map(|key| key.model).or(self.model.as_deref()).and_then(crate::stems::spec)
    }

    pub fn separation_status(&self) -> Option<String> {
        let (_, progress) = self.separating?;
        let model = self.status_model()?.name;
        let backend = if self.execution == crate::stems::SeparationDevice::Cpu || !self.gpu_available() {
            " on CPU"
        } else if MANUAL_STEMS_PREVIEW {
            " with Vulkan + CPU"
        } else {
            ""
        };
        Some(if progress == 0.0 {
            format!("Starting {model}{backend} · waiting for the first segment")
        } else {
            format!("Separating with {model}{backend}: {:.0}%", progress * 100.0)
        })
    }

    fn model_for(&self, track: Option<i64>, execution: crate::stems::SeparationDevice) -> Option<&'static crate::stems::ModelSpec> {
        if execution == crate::stems::SeparationDevice::Cpu {
            if let Some(id) = self.error_for(track).and_then(|e| e.retry_model) {
                return crate::stems::spec(id);
            }
        }
        self.model.as_deref().and_then(crate::stems::spec)
    }

    fn retain_deck_tracks(&mut self, tracks: [Option<i64>; 4]) {
        self.errors.retain(|id, _| id.is_none() || tracks.contains(id));
        for (key, request) in &mut self.loading {
            if !tracks.contains(&Some(key.track)) { *request = None; }
        }
        if self.separating.is_some_and(|(id, _)| !tracks.contains(&Some(id))) {
            self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.pending.as_ref().is_some_and(|(key, _)| !tracks.contains(&Some(key.track))) {
            self.pending = None;
        }
    }

    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        StemState { hw: None, model: None, active_key: None, preview: None, model_ready: false, downloading: None, separating: None, loading: HashMap::new(), errors: HashMap::new(), last_cache_warning: None, cache_limit: Default::default(), cache_usage: None, execution: crate::stems::SeparationDevice::Selected, pending: None, cancel: Default::default(), download_cancel: Default::default(), tx, rx }
    }
}

pub const ZOOMS: [f64; 6] = [5.0, 10.0, 20.0, 40.0, 80.0, 160.0];

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Collection,
    Playlist(i64),
    Device { root: String, node: Option<u32> },
    /// A play history saved on a device by the players.
    DeviceHistory { root: String, id: u32 },
    /// Tracks related to the loaded (or selected) track by the Related Tracks rules.
    Related,
    /// Collection browsed by one field: 0 Date Added, 1 Genre, 2 Artist, 3 Album.
    Category { field: u8, value: String },
    /// A folder on disk (Explorer); its audio files are listed whether or not they are in the
    /// collection.
    Explorer(String),
    /// A day's play history kept by Recordbox (0: the Histories item itself, an empty list).
    History(i64),
    /// The Tag List (temporary pick list).
    TagList,
    /// Good matches registered for the loaded (or selected) track.
    Matching,
    /// Track Suggestion for the loaded (or selected) track: 0 Rank, 1 Era, 2 Mood,
    /// 3 Association (browser_ext.rs).
    Suggest(u8),
    /// An outside library (kind 0 iTunes, 1 rekordbox xml): node "" is All Tracks, else the
    /// playlist's key (see `SourceLib::node`). Its tracks are not collection tracks.
    Source { kind: u8, node: String },
    /// A row of the Sampler tree (sampler_lib.rs): -1 All Samples, -2 Capture, else a list.
    Sampler(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideMode {
    Collection,
    Playlists,
    Devices,
    History,
    Related,
    /// Track Suggestion.
    Suggest,
    Explorer,
    Itunes,
    RekordboxXml,
    /// The Recordings folder (ui_rec.rs).
    Recordings,
    /// The Sampler tree (PERFORMANCE mode).
    Sampler,
}

pub struct Deck {
    pub cancel_work: Arc<std::sync::atomic::AtomicBool>,
    /// Phrase structure shown under the overview (rekordbox's own, or Recordbox's).
    pub phrases: Option<crate::phrase::Structure>,
    /// Phrase analysis running in the background for (track id).
    pub phrases_rx: Option<Receiver<(i64, Option<crate::phrase::Structure>)>>,
    /// rekordbox's own 3-band waveforms for this audio file (detail at 150 per second, and
    /// the 1200-column overview), when the track came from the rekordbox library.
    pub rb_wave: Option<std::sync::Arc<(Vec<[u8; 3]>, Vec<[u8; 3]>)>>,
    /// Vocal activity (rekordbox's PVDI, else Recordbox's): (seconds per entry, level 0–4).
    pub vocal: Option<std::sync::Arc<(f64, Vec<u8>)>>,
    pub vocal_rx: Option<Receiver<(i64, Option<Vec<u8>>)>>,
    // Identifies the parts requested by this deck, including during an asynchronous load.
    pub(crate) stem_cache_key: Option<StemCacheKey>,
    pub track: Option<i64>,
    pub audio: Option<Arc<Decoded>>,
    pub loading: Option<Receiver<(i64, Result<Arc<Decoded>, String>)>>,
    pub wave: Option<Waveform>,
    pub grid: BeatGrid,
    pub beats: Vec<Beat>,
    pub cues: Vec<Cue>,
    pub cue_point: f64,
    pub duration: f64,
    pub cue_preview: bool,
    pub hot_preview: Option<f64>,
    pub loop_in: Option<f64>,
    pub last_loop: Option<(f64, f64)>,
    pub grid_undo: Vec<BeatGrid>,
    pub grid_redo: Vec<BeatGrid>,
    pub taps: Vec<Instant>,
    pub ctrl_tab: u8,
    pub panel_tab: u8,
    pub drag_origin: Option<(f32, f64)>,
    /// A cue marker being dragged on the zoomed waveform: its id, the cues before the drag
    /// (for Undo and Esc) and where on the marker it was grabbed (seconds after its start).
    pub cue_drag: Option<(i64, Vec<Cue>, f64)>,
    /// Where the zoomed waveform was last drawn (rect, playhead time, pixels per second), so
    /// the scripted screenshot check (PREPDECK_SHOT_DRAG) can aim at a marker.
    pub detail_map: Option<(egui::Rect, f64, f64)>,
    pub editing_comment: Option<i64>,
    pub metronome: bool,
    pub bpm_text: String,
    /// Loop adjust, waveform menu and other state added in player_ext.rs.
    pub x: crate::player_ext::DeckExt,
}

impl Default for Deck {
    fn default() -> Self {
        Deck {
            cancel_work: Default::default(),
            phrases: None,
            phrases_rx: None,
            rb_wave: None,
            vocal: None,
            vocal_rx: None,
            stem_cache_key: None,
            track: None,
            audio: None,
            loading: None,
            wave: None,
            grid: BeatGrid::default(),
            beats: vec![],
            cues: vec![],
            cue_point: 0.0,
            duration: 0.0,
            cue_preview: false,
            hot_preview: None,
            loop_in: None,
            last_loop: None,
            grid_undo: vec![],
            grid_redo: vec![],
            taps: vec![],
            ctrl_tab: 0,
            panel_tab: 0,
            drag_origin: None,
            cue_drag: None,
            detail_map: None,
            editing_comment: None,
            metronome: false,
            bpm_text: String::new(),
            x: Default::default(),
        }
    }
}

pub struct Toast {
    pub text: String,
    pub until: Instant,
    pub error: bool,
}

#[derive(Default)]
pub struct Dialogs {
    pub prefs_open: bool,
    pub confirm_clear_stems: bool,
    pub library_sync_review: Option<crate::ui_sync::Review>,
    pub prefs_tab: u8,
    /// Shortcuts window, backup/restore, reset confirmations (prefs_ext.rs).
    pub ext: crate::prefs_ext::DialogsExt,
    pub info_track: Option<Track>,
    pub smart_edit: Option<(Option<i64>, i64, String, SmartRule)>,
    pub rename: Option<(i64, String)>,
    pub new_tag: Option<(i64, String)>,
    pub rename_tag: Option<(i64, String)>,
    pub confirm_remove: Option<Vec<i64>>,
    /// Playlist/folder waiting for delete confirmation: (id, name, is folder).
    pub confirm_delete_node: Option<(i64, String, bool)>,
    pub export_open: bool,
    pub export_root: Option<String>,
    pub export_sel: HashSet<i64>,
    pub export_tracks: Vec<i64>,
    pub about: bool,
    pub device_remove: Option<(String, u32, String)>,
    /// Restore Removed Tracks window: the ticked trash entries.
    pub trash: Option<HashSet<i64>>,
    /// Cue edits found on a connected drive, waiting for review.
    pub player_edits: Option<PlayerEdits>,
    /// Unused files on a drive, waiting for confirmation: (root, device name, files with sizes).
    pub device_cleanup: Option<(String, String, Vec<(std::path::PathBuf, u64)>)>,
}

pub struct PlayerEdits {
    pub root: String,
    pub device: String,
    pub items: Vec<PlayerEditItem>,
}

pub struct PlayerEditItem {
    pub track: i64,
    pub edit: crate::export::usb::PlayerCueEdit,
    /// The track's cues were also changed in Recordbox since the export.
    pub conflict: bool,
    pub pick: bool,
}

pub struct DeviceInfo {
    pub root: String,
    pub name: String,
    pub fs: String,
    pub layout: Option<crate::export::usb::DeviceLayout>,
    pub lib: Option<crate::export::usb::DeviceLibrary>,
    pub write_blocker: Option<String>,
    pub total: Option<u64>,
    pub free: Option<u64>,
}

impl DeviceInfo {
    pub fn catalog_label(&self) -> &'static str {
        use crate::export::usb::DeviceLayout::*;
        match self.layout {
            Some(Classic) => "Classic Device Library",
            Some(Mixed) => "Classic Device Library + OneLibrary",
            Some(OneLibrary) => "OneLibrary / Device Library Plus",
            Some(Empty) => "No device catalog",
            None => "Unrecognized device folder",
        }
    }
}

/// Immutable Collection rows, prepared only for the categories the user opens.
/// These values depend on library metadata, never on the currently loaded deck.
#[derive(Default)]
pub(crate) struct CollectionCategories {
    rows: [Option<Arc<[(String, usize)]>>; 4],
}

impl CollectionCategories {
    fn clear(&mut self) {
        self.rows.fill(None);
    }

    pub(crate) fn rows(&mut self, tracks: &[Track], field: usize) -> Arc<[(String, usize)]> {
        self.rows[field].get_or_insert_with(|| {
            let mut counts = std::collections::BTreeMap::<String, usize>::new();
            for t in tracks {
                *counts.entry(App::category_value(t, field as u8)).or_default() += 1;
            }
            let mut rows: Vec<_> = counts.into_iter().collect();
            if field == 0 {
                rows.reverse(); // Date Added: newest day first, as before.
            }
            rows.into()
        }).clone()
    }
}

#[cfg(test)]
mod collection_category_tests {
    use super::*;

    #[test]
    fn collection_categories_match_existing_grouping_and_order() {
        let tracks = vec![
            Track { date_added: "2025-02-03T09:00".into(), genre: " House ".into(), artist: "B".into(), album: " a ".into(), ..Default::default() },
            Track { date_added: "2025-02-03T10:00".into(), genre: "House".into(), artist: "b".into(), album: "A".into(), ..Default::default() },
            Track { date_added: "2026-01-01".into(), genre: "\t".into(), artist: "é".into(), album: "".into(), ..Default::default() },
            Track::default(),
        ];
        let expected = [
            vec![("2026-01-01", 1), ("2025-02-03", 2), ("", 1)],
            vec![("", 2), ("House", 2)],
            vec![("", 1), ("B", 1), ("b", 1), ("é", 1)],
            vec![("", 2), ("A", 1), ("a", 1)],
        ];
        let mut cache = CollectionCategories::default();
        for (field, want) in expected.into_iter().enumerate() {
            let rows = cache.rows(&tracks, field);
            assert_eq!(&*rows, want.iter().map(|(s, n)| (s.to_string(), *n)).collect::<Vec<_>>());
            assert!(Arc::ptr_eq(&rows, &cache.rows(&tracks, field)), "unchanged frames share immutable rows");
        }
    }

    #[test]
    fn collection_categories_rebuild_after_metadata_and_library_changes() {
        let mut tracks = vec![Track { artist: "old".into(), album: "old".into(), genre: "old".into(), date_added: "2024-01-01".into(), ..Default::default() }];
        let mut cache = CollectionCategories::default();
        let old: Vec<_> = (0..4).map(|field| cache.rows(&tracks, field)).collect();
        // The same invalidation used by track_mut covers metadata edits and Undo/Redo.
        cache.clear();
        tracks[0].artist = "new artist".into();
        tracks[0].album = "new album".into();
        tracks[0].genre = "new genre".into();
        tracks[0].date_added = "2026-02-03T12:00".into();
        for field in 0..4 {
            let rows = cache.rows(&tracks, field);
            assert!(!Arc::ptr_eq(&rows, &old[field]));
            assert_eq!(&*rows, &[(App::category_value(&tracks[0], field as u8), 1)]);
        }
        // Reloads can add or remove tracks while old row snapshots remain alive.
        cache.clear();
        tracks.push(tracks[0].clone());
        assert_eq!(cache.rows(&tracks, 2)[0].1, 2);
        cache.clear();
        tracks.clear();
        assert!(cache.rows(&tracks, 2).is_empty());
        assert_eq!(&*old[2], &[("old".into(), 1)]);
    }
}

pub struct App {
    pub deck_options: crate::DeckOptions,
    pub db: Db,
    pub prefs: Prefs,
    pub tracks: Vec<Track>,
    pub(crate) collection_categories: CollectionCategories,
    pub index: HashMap<i64, usize>,
    pub previews: HashMap<i64, Vec<u8>>,
    pub cue_counts: HashMap<i64, (usize, usize)>,
    pub hot_positions: HashMap<i64, Vec<(f64, i32, u8)>>,
    pub nodes: Vec<PlaylistNode>,
    pub node_counts: HashMap<i64, usize>,
    pub mytags: Vec<MyTag>,
    pub track_tags: HashMap<i64, HashSet<i64>>,
    pub view: View,
    pub side: SideMode,
    pub expanded: HashSet<i64>,
    pub list: Vec<i64>,
    pub list_dirty: bool,
    pub selected: HashSet<i64>,
    pub anchor: Option<usize>,
    /// Track the keyboard/shift-click anchor points at; `anchor` is its row in `list` and is
    /// re-derived whenever the list is rebuilt (sort, search, view change).
    pub anchor_id: Option<i64>,
    pub search: String,
    pub filter_open: bool,
    /// Sub-browser source (shown while `prefs.sub_browser` is on) and its selection.
    pub sub_view: View,
    pub sub_sel: HashSet<i64>,
    /// A track drag that started in the sub-browser (the main list is then a drop target).
    pub sub_drag: bool,
    /// Column browser selections (Genre, Artist, Album); None = All.
    pub colbrowse: [Option<String>; 3],
    /// Values each column browser column offers, with track counts (cascading left to right).
    pub colbrowse_values: [Vec<(String, usize)>; 3],
    pub filter_bpm: (f64, f64),
    pub filter_key: i32,
    pub filter_rating: u8,
    pub filter_color: u8,
    pub filter_tags: HashSet<i64>,
    pub filter_tags_all: bool,
    pub player: Player,
    /// The active deck (deck A, or in the 2 Players layout the one last operated). The other
    /// deck is kept in `px` and the two are swapped when the active deck changes.
    pub deck: Deck,
    /// Second deck, active deck number and other player state (player_ext.rs).
    pub px: crate::player_ext::PlayerExt,
    /// Duplicate review and per-window controls.
    pub dupes: crate::dupes::Dupes,
    pub(crate) toggle_library_window: bool,
    pub cue_tools: crate::cue_tools::CueTools,
    pub traffic_now: Vec<i32>,
    /// Groove Circuit's state while the app runs (groove.rs).
    pub gc: crate::groove::GrooveUi,
    /// The recorder: the recording in progress and the steps after it (ui_rec.rs).
    pub rec: crate::ui_rec::RecUi,
    /// The pad modes' state per deck (`pads_ext.rs`).
    pub pads: crate::pads_ext::PadsState,
    /// Automix, MIX POINT LINK and the sampler lists (automix.rs, mpl.rs, sampler_lib.rs).
    pub mixx: crate::automix::MixExt,
    /// Preferences rows that act over time (prefs_bridge.rs).
    pub wiring: crate::prefs_bridge::WiringState,
    /// Deck that double-click / Return / "Load to Player" load into: 0 = PLAYER A, 1 = PLAYER B
    /// (B only in the 2 Players layout).
    pub load_target: u8,
    /// The deck as it was before the load in progress, with its playback position and gain;
    /// put back if the new file can't be decoded, so a failed load never empties the player.
    pub deck_backup: Option<(Deck, f64, f32)>,
    /// Source track the Related Tracks list was last built for.
    pub related_built_for: Option<i64>,
    /// File to load into the player once the import in progress has added it.
    pub load_after_import: Option<String>,
    /// Playlist/folder being dragged in the tree.
    pub node_drag: Option<i64>,
    pub stem: StemState,
    pub undo: Vec<Edit>,
    pub redo: Vec<Edit>,
    pub pool: AnalysisPool,
    /// Hot slots the user cleared while this track's library analysis was outstanding.
    cleared_analysis_hot_slots: HashMap<i64, HashSet<i32>>,
    pub analysis_total: usize,
    pub analysis_done: usize,
    pub importing: Option<Receiver<Option<Track>>>,
    pub import_count: usize,
    pub import_to_playlist: Option<i64>,
    pub export_job: Option<ExportJob>,
    /// What the last export did (shown in the export window until it is closed).
    pub export_report: Option<crate::export::usb::ExportReport>,
    /// Device windows and per-device state (device_ext.rs).
    pub device_ui: crate::device_ext::DeviceUi,
    /// Background snapshot + read of the rekordbox library, started at launch.
    pub rb_job: Option<crossbeam_channel::Receiver<anyhow::Result<Option<crate::rbimport::RbLibrary>>>>,
    pub devices: Vec<DeviceInfo>,
    pub devices_scanned: Option<Instant>,
    /// Drives already checked for player cue edits while connected.
    pub edits_checked: HashSet<String>,
    /// Key analysis on the drum-free mix in the background: results (track, key) and the
    /// number of tracks asked for.
    pub key_job: Option<(Receiver<(i64, i32)>, usize, usize)>,
    /// Last attempt to reopen a lost audio device, and whether the loss was reported.
    pub audio_retry: Option<(Instant, bool)>,
    /// Folder scan for Relocate Missing Files: file names (folded) to paths.
    pub relocate_rx: Option<RelocationSearch>,
    /// Tracks whose key was asked for with Analyze Key (their next result replaces the key).
    pub key_requested: HashSet<i64>,
    /// Unlocked grids explicitly requested for replacement by Analyze Track.
    pub grid_requested: HashMap<i64, Option<BeatGrid>>,
    /// Tracks whose vocal detection failed this session (not retried on every load).
    pub vocal_failed: HashSet<i64>,
    pub dialogs: Dialogs,
    pub toasts: Vec<Toast>,
    pub drag: Option<Vec<i64>>,
    pub(crate) library_window: Option<crate::DetachedLibrary>,
    /// A track being dragged between the decks or sampler while the library is in another window.
    detached_player_drag: Option<Vec<i64>>,
    pub textures: HashMap<String, Option<egui::TextureHandle>>,
    pub ctx: egui::Context,
    pub screenshot: Option<(String, u32)>,
    pub scroll_to_selected: bool,
    /// The track list's vertical scroll position set by dragging its scroll bar (applied on the
    /// next frame).
    pub list_scroll_to: Option<f32>,
    /// Native menu bar (kept alive for the app's lifetime).
    pub menu: Option<muda::Menu>,
    /// Native Edit menu actions waiting for the focused window's text field or track list.
    pub edit_events: Vec<egui::Event>,
    /// Library management state kept by `library_ext` (histories, Tag List, filter, preview…).
    pub ext: crate::library_ext::LibExt,
    /// MIDI controllers and the MIDI setting window (midi.rs).
    pub midi: crate::midi::MidiState,
    /// EDIT mode: the open project and the editor's state (ui_edit.rs).
    pub edit: crate::ui_edit::EditState,
}

impl App {
    /// Native windows have independent pointer positions and releases. Keep the decks' drag
    /// available to the sampler in their window and the browser drag available in the library.
    pub fn with_detached_player_drag(&mut self, ctx: &egui::Context, draw: impl FnOnce(&mut App)) {
        std::mem::swap(&mut self.drag, &mut self.detached_player_drag);
        draw(self);
        if ctx.input(|i| !i.focused || !i.pointer.any_down()) {
            self.drag = None;
        }
        std::mem::swap(&mut self.drag, &mut self.detached_player_drag);
    }

    fn open_library(ctx: &egui::Context, screenshot: Option<String>, deck_options: crate::DeckOptions) -> App {
        crate::theme::apply(ctx);
        // SVG images (the recordbox logo).
        egui_extras::install_image_loaders(ctx);
        let db_path = crate::db::data_dir().join("library.db");
        let db = Db::open(&db_path).expect("open library database");
        // Removed tracks stay restorable for 30 days.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let _ = db.purge_trash(Some(now - 30 * 86_400));
        // Unreadable saved preferences are kept under another key (not silently replaced).
        let raw_prefs = db.setting("prefs");
        let prefs: Prefs = match raw_prefs.as_deref().map(serde_json::from_str::<Prefs>) {
            Some(Ok(p)) => p,
            Some(Err(_)) => {
                let key = format!("prefs_unreadable_{}", chrono::Local::now().format("%Y%m%d%H%M%S"));
                db.set_setting(&key, raw_prefs.as_deref().unwrap_or_default());
                eprintln!("preferences could not be read; kept as {key}, using defaults");
                Prefs::default()
            }
            None => Prefs::default(),
        };
        // The reference loudness used to default to -8 LUFS; rekordbox's is -10. A saved -8 that
        // was never changed moves to the new default once.
        let mut prefs = prefs;
        if db.setting("lufs_default_migrated").is_none() {
            match migrate_lufs_default(&db, &prefs, raw_prefs.as_deref()) {
                Ok(next) => prefs = next,
                Err(e) => eprintln!("loudness default migration postponed: {e}"),
            }
        }
        crate::prefs_ext::set_stream_opts(&prefs);
        let audio_device = prefs.audio_device.clone();
        let pool = AnalysisPool::new(ctx.clone(), prefs.analysis_threads);
        let stem = StemState::new();
        stem.update_cache_limit(&prefs);
        App {
            deck_options,
            db,
            prefs,
            tracks: vec![],
            collection_categories: CollectionCategories::default(),
            index: HashMap::new(),
            previews: HashMap::new(),
            cue_counts: HashMap::new(),
            hot_positions: HashMap::new(),
            nodes: vec![],
            node_counts: HashMap::new(),
            mytags: vec![],
            track_tags: HashMap::new(),
            view: View::Collection,
            side: SideMode::Collection,
            expanded: HashSet::new(),
            list: vec![],
            list_dirty: true,
            selected: HashSet::new(),
            anchor: None,
            anchor_id: None,
            deck_backup: None,
            related_built_for: None,
            load_after_import: None,
            node_drag: None,
            stem,
            undo: vec![],
            redo: vec![],
            search: String::new(),
            filter_open: false,
            sub_view: View::Collection,
            sub_sel: HashSet::new(),
            sub_drag: false,
            colbrowse: Default::default(),
            colbrowse_values: Default::default(),
            filter_bpm: (0.0, 0.0),
            filter_key: -1,
            filter_rating: 0,
            filter_color: 0,
            filter_tags: HashSet::new(),
            filter_tags_all: false,
            player: Player::with_device(audio_device.as_deref(), !deck_options.disable_play),
            px: Default::default(),
            dupes: Default::default(),
            toggle_library_window: false,
            cue_tools: Default::default(),
            traffic_now: Vec::new(),
            gc: Default::default(),
            rec: Default::default(),
            pads: Default::default(),
            mixx: Default::default(),
            wiring: Default::default(),
            load_target: 0,
            deck: Deck::default(),
            pool,
            cleared_analysis_hot_slots: HashMap::new(),
            analysis_total: 0,
            analysis_done: 0,
            importing: None,
            import_count: 0,
            import_to_playlist: None,
            export_job: None,
            export_report: None,
            device_ui: Default::default(),
            rb_job: None,
            devices: vec![],
            devices_scanned: None,
            edits_checked: HashSet::new(),
            key_job: None,
            vocal_failed: HashSet::new(),
            key_requested: HashSet::new(),
            grid_requested: HashMap::new(),
            relocate_rx: None,
            audio_retry: None,
            dialogs: Dialogs::default(),
            toasts: vec![],
            drag: None,
            library_window: None,
            detached_player_drag: None,
            textures: HashMap::new(),
            ctx: ctx.clone(),
            screenshot: screenshot.map(|s| (s, 0)),
            scroll_to_selected: false,
            list_scroll_to: None,
            menu: None,
            edit_events: vec![],
            ext: Default::default(),
            midi: Default::default(),
            edit: Default::default(),
        }
    }

    pub fn new(cc: &eframe::CreationContext, screenshot: Option<String>, deck_options: crate::DeckOptions) -> App {
        let mut app = Self::open_library(&cc.egui_ctx, screenshot, deck_options);
        // Only AppKit installs the native menu. Other platforms dispatch its shortcuts
        // through egui, so an unused menu must not claim those keys or scan the library.
        if cfg!(target_os = "macos") {
            app.menu = crate::menus::build();
            crate::ui_edit::attach_menu(&mut app);
            crate::menus::wake_on_menu(&app.ctx);
        }
        app.reload_all();
        app.reload_library_ext();
        if let Some(e) = app.player.error.clone() {
            app.toast_err(format!("Audio output unavailable: {e}"));
        }
        // First start: add original demo loops (like the bundled demo tracks of the original).
        let rb_library = app.prefs.import_rekordbox_at_startup && crate::rbimport::library_dir().is_some();
        if app.tracks.is_empty() && app.db.setting("demo_added").is_none() && (!rb_library || (app.screenshot.is_some() && std::env::var("PREPDECK_SHOT_SYNC").is_err())) {
            app.db.set_setting("demo_added", "1");
            let files = crate::demo::ensure_demo_files();
            for f in files {
                let mut t = crate::importer::read_track(&f);
                t.artist = "recordbox".into();
                t.album = "Demo Tracks".into();
                t.genre = "Demo".into();
                let _ = app.db.insert_track(&t);
            }
            app.reload_all();
            if app.screenshot.is_some() {
                let s = app.analysis_settings();
                let todo: Vec<(i64, String)> = app.tracks.iter().map(|t| (t.id, t.path.clone())).collect();
                for (id, p) in todo {
                    if let Ok(r) = crate::analysis::analyze_file(&p, &s) {
                        app.apply_analysis(id, Ok(r));
                    }
                }
            } else {
                let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
                app.analyze(&ids, false);
            }
        }
        if app.screenshot.is_none() || std::env::var("PREPDECK_SHOT_STEMS").is_ok() {
            app.stems_init();
        }
        // Test switch: open Preferences at launch.
        if std::env::var("PREPDECK_OPEN_PREFS").is_ok() {
            app.dialogs.prefs_open = true;
        }
        if rb_library && (app.screenshot.is_none() || std::env::var("PREPDECK_SHOT_SYNC").is_ok()) {
            app.start_rekordbox_sync();
        }
        if app.screenshot.is_some() {
            // Optional state for checking other screens: PREPDECK_SHOT_VIEW=playlists|devices|grid|hotcue|prefs|mytag|smart|export|info
            // Several states can be combined with commas (PREPDECK_SHOT_VIEW=playlists,mytag).
            for v in std::env::var("PREPDECK_SHOT_VIEW").unwrap_or_default().split(',').filter(|v| !v.is_empty()).map(|v| v.to_string()) {
                match v.as_str() {
                    "playlists" | "smart" => {
                        app.side = SideMode::Playlists;
                        if app.nodes.is_empty() {
                            let f = app.db.create_node(0, "Friday Set", NodeKind::Folder, None).unwrap_or(0);
                            let p = app.db.create_node(f, "Warm Up", NodeKind::Playlist, None).unwrap_or(0);
                            let ids: Vec<i64> = app.tracks.iter().map(|t| t.id).collect();
                            let _ = app.db.set_playlist_tracks(p, &ids);
                            let mut rule = SmartRule::default();
                            rule.conditions[0] = crate::smart::Condition { field: crate::smart::Field::Bpm, op: crate::smart::Op::InRange, value: "120".into(), value2: "128".into(), months: false };
                            let _ = app.db.create_node(0, "120-128 BPM", NodeKind::Smart, Some(&rule));
                            app.reload_playlists();
                            app.expanded.insert(f);
                            app.view = View::Playlist(p);
                            app.set_view(View::Collection);
                            app.set_view(View::Playlist(p));
                        }
                        // PREPDECK_SHOT_PLAYLIST=1: open the first playlist of 20+ tracks (its folders
                        // expanded) and select its 29th row, the state of reference 04.
                        if std::env::var("PREPDECK_SHOT_PLAYLIST").is_ok() {
                            let pick = app.nodes.iter().find(|n| n.kind == NodeKind::Playlist && app.node_counts.get(&n.id).copied().unwrap_or(0) >= 20).cloned();
                            if let Some(n) = pick {
                                let mut up = n.parent;
                                while up != 0 {
                                    app.expanded.insert(up);
                                    up = app.nodes.iter().find(|x| x.id == up).map(|x| x.parent).unwrap_or(0);
                                }
                                app.set_view(View::Playlist(n.id));
                                app.rebuild_list();
                                if let Some(id) = app.list.get(28).copied() {
                                    app.selected.clear();
                                    app.selected.insert(id);
                                }
                            }
                        }
                        if v == "smart" {
                            let n = app.nodes.iter().find(|n| n.kind == NodeKind::Smart).cloned();
                            if let Some(n) = n {
                                app.dialogs.smart_edit = Some((Some(n.id), n.parent, n.name.clone(), n.smart.clone().unwrap_or_default()));
                            }
                        }
                    }
                    "devices" => {
                        app.side = SideMode::Devices;
                        app.view = View::Device { root: String::new(), node: None };
                    }
                    "history" => app.side = SideMode::History,
                    // Device windows and lists (device_ext.rs).
                    "devicelist" | "deviceprop" | "devicesync" | "devicereport" | "hotcuebank" => crate::device_ext::shot_setup(&mut app, v.as_str()),
                    "explorer" => {
                        app.side = SideMode::Explorer;
                        let d = dirs::audio_dir().unwrap_or_default().to_string_lossy().to_string();
                        app.view = View::Explorer(d);
                    }
                    "related" => {
                        app.side = SideMode::Related;
                        app.view = View::Related;
                    }
                    "grid" => app.deck.ctrl_tab = 1,
                    // EXPORT mode whatever mode was saved (the 1 Player layout: "exportmode1").
                    "exportmode" => app.prefs.app_mode = 0,
                    "exportmode1" => {
                        app.prefs.app_mode = 0;
                        app.prefs.browser_only = false;
                        app.prefs.player_layout = 0;
                    }
                    "dupes" | "dupesmerge" => crate::dupes::shot_setup(&mut app, v.as_str()),
                    // Traffic Light on, for the target PREPDECK_SHOT_TRAFFIC names (default: the
                    // mode's default target).
                    "traffic" => {
                        app.prefs.traffic.on = true;
                        if let Some(t) = std::env::var("PREPDECK_SHOT_TRAFFIC").ok().and_then(|v| v.parse::<u8>().ok()) {
                            app.prefs.traffic.target_export = t;
                            app.prefs.traffic.target_perf = t;
                        }
                    }
                    "hotcue" => app.deck.panel_tab = 1,
                    "info" => app.deck.panel_tab = 2,
                    "prefs" => {
                        app.dialogs.prefs_open = true;
                        // PREPDECK_SHOT_PREFS=<tab>,<subtab> (default 1,0).
                        let (t, st) = std::env::var("PREPDECK_SHOT_PREFS").ok().and_then(|v| {
                            let mut it = v.split(',').map(|x| x.trim().parse::<usize>().ok());
                            Some((it.next()??, it.next().flatten().unwrap_or(0)))
                        }).unwrap_or((1, 0));
                        app.dialogs.prefs_tab = t as u8;
                        cc.egui_ctx.data_mut(|d| d.insert_temp(egui::Id::new(format!("prefs_subtab_{t}")), st));
                    }
                    "mytag" => app.prefs.show_mytag = true,
                    "shortcuts" => app.dialogs.ext.shortcuts_open = true,
                    "backupask" => app.dialogs.ext.backup_ask = true,
                    "prefsearch" => {
                        app.dialogs.prefs_open = true;
                        app.dialogs.ext.prefs_search = std::env::var("PREPDECK_SHOT_SEARCH").unwrap_or_else(|_| "waveform".into());
                    }
                    "colbrowse" => {
                        app.prefs.column_browser = true;
                        app.list_dirty = true;
                    }
                    "trash" => {
                        if app.db.trash_list().map(|l| l.is_empty()).unwrap_or(true) {
                            let ids: Vec<i64> = app.tracks.iter().rev().take(2).map(|t| t.id).collect();
                            app.remove_from_collection(&ids);
                        }
                        app.dialogs.trash = Some(app.db.trash_list().unwrap_or_default().first().map(|e| e.0).into_iter().collect());
                    }
                    "playeredits" => {
                        // End-to-end check of player cue edits: export two tracks with hot cues to
                        // a scratch folder device, move one cue in its analysis file as a player
                        // would, then scan.
                        let root = std::path::PathBuf::from(std::env::var("PREPDECK_SHOT_DEVICE").unwrap_or_else(|_| std::env::temp_dir().join("recordbox_shot_device").to_string_lossy().to_string()));
                        let _ = std::fs::remove_dir_all(&root);
                        let _ = std::fs::create_dir_all(&root);
                        let ids: Vec<i64> = app.tracks.iter().filter(|t| !t.missing && app.db.cues(t.id).map(|c| c.iter().any(|c| c.kind == CueKind::Hot)).unwrap_or(false)).take(2).map(|t| t.id).collect();
                        let mut tracks = HashMap::new();
                        for id in &ids {
                            if let Some(t) = app.track(*id).cloned() {
                                tracks.insert(*id, crate::export::usb::ExportTrack { cues: app.db.cues(*id).unwrap_or_default(), grid: app.db.grid(*id), wave: app.db.wave(*id), track: t, tags: vec![], rb_anlz: None, grid_same: false, vocal: None, phrases: None });
                            }
                        }
                        let req = crate::export::usb::ExportRequest { root: root.clone(), nodes: vec![crate::export::usb::ExportNode { id: 0, name: "Check".into(), folder: false, children: vec![], tracks: ids.clone() }], tracks, tag_categories: vec![], skipped: vec![], analysis: app.analysis_settings(), settings: Default::default(), browse: Default::default(), phrases: false, ..Default::default() };
                        let cancel = std::sync::atomic::AtomicBool::new(false);
                        match crate::export::usb::run_export(req, &|_| {}, &cancel) {
                            Ok(m) => eprintln!("shot export: {m}"),
                            Err(e) => eprintln!("shot export failed: {e}"),
                        }
                        if let Ok(lib) = crate::export::usb::read_device(&root) {
                            if let Some(t) = lib.tracks.values().next() {
                                let base = root.join(t.analyze_path.trim_start_matches('/')).with_extension("");
                                let mut cues = crate::export::anlz::parse_cues(&crate::export::anlz::read_section_list(&base.with_extension("EXT")), &crate::export::anlz::read_section_list(&base.with_extension("DAT")));
                                if let Some(c) = cues.iter_mut().find(|c| c.kind == CueKind::Hot) {
                                    c.start += 2.0;
                                }
                                let f = crate::export::anlz::build(&t.file_path, &[], &Waveform::default(), &cues, 0, 44100);
                                let _ = std::fs::write(base.with_extension("DAT"), &f.dat);
                                let _ = std::fs::write(base.with_extension("EXT"), &f.ext);
                                if let Ok(file) = std::fs::File::options().write(true).open(root.join(".Recordbox/cues.json")) {
                                    let _ = file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60));
                                }
                            }
                        }
                        let r = root.to_string_lossy().to_string();
                        if !app.prefs.devices_extra.contains(&r) {
                            app.prefs.devices_extra.push(r.clone());
                        }
                        app.scan_devices();
                        if app.dialogs.player_edits.is_none() {
                            app.check_player_cue_edits(&r, true);
                        }
                    }
                    "updatedevice" => {
                        // Check of Update Playlists on This Device: the drive has "Update Check"
                        // with one track; the library's "Update Check" has three.
                        let root = std::path::PathBuf::from(std::env::var("PREPDECK_SHOT_DEVICE").unwrap_or_else(|_| std::env::temp_dir().join("recordbox_shot_update").to_string_lossy().to_string()));
                        let _ = std::fs::remove_dir_all(&root);
                        let _ = std::fs::create_dir_all(&root);
                        let ids: Vec<i64> = app.tracks.iter().filter(|t| !t.missing && t.analyzed == 1).take(3).map(|t| t.id).collect();
                        let pl = app.nodes.iter().find(|n| n.name == "Update Check").map(|n| n.id).unwrap_or_else(|| app.db.create_node(0, "Update Check", NodeKind::Playlist, None).unwrap_or(0));
                        let _ = app.db.set_playlist_tracks(pl, &ids);
                        app.reload_playlists();
                        let mut tracks = HashMap::new();
                        if let Some(t) = ids.first().and_then(|id| app.track(*id).cloned()) {
                            tracks.insert(t.id, crate::export::usb::ExportTrack { cues: vec![], grid: app.db.grid(t.id), wave: app.db.wave(t.id), track: t, tags: vec![], rb_anlz: None, grid_same: false, vocal: None, phrases: None });
                        }
                        let req = crate::export::usb::ExportRequest { root: root.clone(), nodes: vec![crate::export::usb::ExportNode { id: 0, name: "Update Check".into(), folder: false, children: vec![], tracks: ids[..1].to_vec() }], tracks, tag_categories: vec![], skipped: vec![], analysis: app.analysis_settings(), settings: Default::default(), browse: Default::default(), phrases: false, ..Default::default() };
                        let cancel = std::sync::atomic::AtomicBool::new(false);
                        let _ = crate::export::usb::run_export(req, &|_| {}, &cancel);
                        let r = root.to_string_lossy().to_string();
                        if !app.prefs.devices_extra.contains(&r) {
                            app.prefs.devices_extra.push(r.clone());
                        }
                        app.scan_devices();
                        app.update_device_playlists(&r);
                        // Let the export finish before the screenshot (the app exits after it).
                        let t0 = std::time::Instant::now();
                        while app.export_job.is_some() && t0.elapsed() < std::time::Duration::from_secs(120) {
                            app.poll_export();
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                    }
                    "relocate" => {
                        // Check of Relocate Missing Files: move one track's file into a subfolder of
                        // PREPDECK_SHOT_MOVED, then search that folder.
                        let dest = std::path::PathBuf::from(std::env::var("PREPDECK_SHOT_MOVED").unwrap_or_default());
                        if let Some(t) = app.tracks.iter().find(|t| !t.missing).cloned() {
                            let to = dest.join("nested").join(std::path::Path::new(&t.path).file_name().unwrap());
                            let _ = std::fs::create_dir_all(to.parent().unwrap());
                            if std::fs::rename(&t.path, &to).is_ok() {
                                app.reload_tracks();
                                app.relocate_missing(&dest);
                                let t0 = std::time::Instant::now();
                                while app.relocate_rx.is_some() && t0.elapsed() < std::time::Duration::from_secs(30) {
                                    app.poll_relocate();
                                    std::thread::sleep(std::time::Duration::from_millis(20));
                                }
                            }
                        }
                    }
                    "subbrowser" => {
                        app.prefs.sub_browser = true;
                        // The largest playlist.
                        let best = app.nodes.iter().filter(|n| n.kind == NodeKind::Playlist).max_by_key(|n| app.db.playlist_tracks(n.id).map(|v| v.len()).unwrap_or(0)).map(|n| n.id);
                        if let Some(id) = best {
                            app.sub_view = View::Playlist(id);
                        }
                    }
                    "export" => app.dialogs.export_open = true,
                    // Dialogs of ui_dialogs.rs.
                    "property" => app.dialogs.info_track = app.tracks.first().cloned(),
                    "confirmremove" => app.dialogs.confirm_remove = Some(app.tracks.iter().take(3).map(|t| t.id).collect()),
                    "confirmdelete" => app.dialogs.confirm_delete_node = Some((0, "Warm Up".into(), false)),
                    "about" => app.dialogs.about = true,
                    "performance" => app.prefs.app_mode = 1,
                    // Performance, 4Deck Horizontal.
                    // Performance with the sampler panel.
                    "perfsampler" => {
                        app.prefs.app_mode = 1;
                        app.prefs.perf_sampler = true;
                        app.prefs.perf_four = false;
                        app.prefs.perf_fx = false;
                        // Three slots loaded (one a loop), for the screenshot.
                        let ids: Vec<i64> = app.tracks.iter().filter(|t| !t.missing).take(3).map(|t| t.id).collect();
                        for (i, id) in ids.into_iter().enumerate() {
                            app.sampler_load([0usize, 1, 9][i], id);
                        }
                        if let Some(Some(sl)) = app.prefs.sampler.slots.get_mut(1) {
                            sl.looped = true;
                        }
                        // Deck 1's pads play the sampler, deck 2's call the sequences.
                        app.px.pad_mode = [crate::ui_perf::PAD_SAMPLER, crate::ui_perf::PAD_SEQ_CALL, 0, 0];
                    }
                    "perf4" => {
                        app.prefs.app_mode = 1;
                        app.prefs.perf_four = true;
                    }
                    // 4Deck Horizontal with the sampler: decks 1 and 3 in SAMPLER pad mode,
                    // decks 2 and 4 in SEQ. CALL.
                    "perf4sampler" => {
                        app.prefs.app_mode = 1;
                        app.prefs.perf_four = true;
                        app.prefs.perf_sampler = true;
                        app.prefs.perf_fx = false;
                        let ids: Vec<i64> = app.tracks.iter().filter(|t| !t.missing).take(3).map(|t| t.id).collect();
                        for (i, id) in ids.into_iter().enumerate() {
                            app.sampler_load([0usize, 1, 9][i], id);
                        }
                        app.px.pad_mode = [crate::ui_perf::PAD_SAMPLER, crate::ui_perf::PAD_SEQ_CALL, crate::ui_perf::PAD_SAMPLER, crate::ui_perf::PAD_SEQ_CALL];
                    }
                    // Performance with the FX panel open.
                    "perffx" | "perffx2" => {
                        app.prefs.app_mode = 1;
                        app.prefs.perf_fx = true;
                    }
                    // Performance with BEAT JUMP pads on deck 1 and BEAT LOOP pads on deck 2.
                    // The recording panel (ui_rec.rs).
                    "perfrec" | "exportrec" => crate::ui_rec::shot_view(&mut app, &v),
                    "perfpads" => {
                        app.prefs.app_mode = 1;
                        app.px.pad_mode = [2, 3, 0, 0];
                    }
                    v if crate::ui_edit::shot_view(&mut app, v) => {}
                    v if crate::ui_perf_layouts::shot_view(&mut app, v) => {}
                    other => crate::library_ext::shot_view(&mut app, other),
                }
            }
            // PREPDECK_SHOT_TRACK=<part of a title>: load that track (analysed first if needed).
            let wanted = std::env::var("PREPDECK_SHOT_TRACK").ok().map(|w| w.to_lowercase());
            let first = match &wanted {
                Some(w) => app.tracks.iter().find(|t| t.title.to_lowercase().contains(w) && !t.missing).map(|t| t.id),
                None => app.tracks.iter().find(|t| t.analyzed == 1 && !t.missing).map(|t| t.id),
            };
            if let Some(id) = first.filter(|id| app.track(*id).map(|t| t.analyzed != 1).unwrap_or(false)) {
                let s = app.analysis_settings();
                if let Some(p) = app.track(id).map(|t| t.path.clone()) {
                    if let Ok(r) = crate::analysis::analyze_file(&p, &s) {
                        app.apply_analysis(id, Ok(r));
                    }
                }
            }
            if let Some(t) = first {
                app.load_track(t);
                // ("row2" keeps its own selection: the second row of the list.)
                if !std::env::var("PREPDECK_SHOT_VIEW").map(|v| v.contains("row2")).unwrap_or(false) {
                    app.selected.insert(t);
                }
                // PREPDECK_SHOT_STEMS=1: show the stems strip; =separate: separate the whole
                // track first (synchronously, model from PREPDECK_STEM_MODEL) and mute vocals.
                if let Ok(v) = std::env::var("PREPDECK_SHOT_STEMS") {
                    app.deck.ctrl_tab = 2;
                    let m = std::env::var("PREPDECK_STEM_MODEL").ok().and_then(|id| crate::stems::spec(&id)).unwrap_or(&crate::stems::CATALOG[1]);
                    app.stem.model = Some(m.id.to_string());
                    if v == "separate" {
                        if let Some(tr) = app.track(t).cloned() {
                            let t0 = std::time::Instant::now();
                            let r = crate::audio::decode_file(&tr.path).map_err(|e| e.to_string()).and_then(|dec| crate::stems::separate(m, &dec.samples, dec.sr, &mut |_| true).map_err(|e| e.to_string()));
                            match r {
                                Ok(st) => {
                                    eprintln!("separated {} ({:.0} s of audio) in {:.1} s", tr.display_title(), st.len as f64 / 44100.0, t0.elapsed().as_secs_f64());
                                    let key = app.stem_cache_key(t, m.id).unwrap();
                                    let _ = crate::stems::save(&key.path, &st);
                                    app.deck.stem_cache_key = Some(key);
                                    let mut ps = app.player.state.lock();
                                    ps.stems = Some(std::sync::Arc::new(st));
                                    ps.stem_on = [true, true, true, false];
                                }
                                Err(e) => eprintln!("separation failed: {e}"),
                            }
                        }
                    }
                }
                if let Some(rx) = app.deck.loading.take() {
                    if let Ok(msg) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
                        let (tx2, rx2) = crossbeam_channel::bounded(1);
                        let _ = tx2.send(msg);
                        app.deck.loading = Some(rx2);
                        app.poll_load();
                    }
                }
                if std::env::var("PREPDECK_SHOT_VIEW").map(|v| v == "hotcue" || v == "grid").unwrap_or(false) && app.deck.cues.is_empty() {
                    let beats = app.deck.beats.clone();
                    for (slot, bi) in [(0, 16usize), (1, 48), (3, 80)] {
                        if let Some(b) = beats.get(bi) {
                            app.seek(b.time);
                            app.hot_cue_down(slot);
                        }
                    }
                    if let Some(b) = beats.get(32) {
                        app.deck.cue_point = b.time;
                        app.memory_set();
                    }
                    let c = app.deck.cue_point;
                    app.seek(c + 2.0);
                }
            }
            // PREPDECK_SHOT_PLAYER=…: player layouts and states (player_ext.rs).
            app.shot_player_setup();
            crate::mix_selftest::shot_setup(&mut app);
        }
        app
    }

    pub fn save_prefs(&self) {
        self.stem.update_cache_limit(&self.prefs);
        // While the sub-browser's own columns and sort are swapped in, nothing is written
        // (ui_browser_ext.rs saves afterwards).
        if self.ext.in_sub {
            self.ext.sub_save_wanted.set(true);
            return;
        }
        if let Ok(s) = serde_json::to_string(&self.prefs) {
            self.db.set_setting("prefs", &s);
        }
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toasts.push(Toast { text: s.into(), until: Instant::now() + std::time::Duration::from_secs(5), error: false });
    }
    pub fn toast_err(&mut self, s: impl Into<String>) {
        self.toasts.push(Toast { text: s.into(), until: Instant::now() + std::time::Duration::from_secs(8), error: true });
    }

    pub fn analysis_settings(&self) -> AnalysisSettings {
        self.prefs.analysis_settings()
    }

    // ------------------------------------------------------------------ loading
    pub fn reload_all(&mut self) {
        self.reload_tracks();
        self.previews = self.db.previews().unwrap_or_default();
        self.cue_counts = self.db.tracks_with_cues().unwrap_or_default();
        self.hot_positions = self.db.hot_positions().unwrap_or_default();
        self.reload_playlists();
        self.reload_tags();
    }
    pub fn reload_tracks(&mut self) {
        self.collection_categories.clear();
        let queued: HashSet<i64> = self.tracks.iter().filter(|t| t.analyzed == 3).map(|t| t.id).collect();
        self.tracks = self.db.all_tracks().unwrap_or_default();
        let mut replaced: Vec<i64> = vec![];
        let stamps = self.db.media_stamps();
        for t in self.tracks.iter_mut() {
            let meta = std::fs::metadata(&t.path).ok();
            t.missing = meta.is_none();
            if queued.contains(&t.id) {
                t.analyzed = 3;
            } else if t.analyzed == 3 {
                t.analyzed = 0;
            }
            // The audio file changed since Recordbox analysed it (size or modification time differ
            // from the stamp taken then): its waveform no longer fits.
            if let (Some(m), Some(stamp)) = (meta, stamps.get(&t.id)) {
                if t.analyzed == 1 && !t.grid_locked && media_stamp(&m) != *stamp {
                    replaced.push(t.id);
                }
            }
        }
        self.index = self.tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        self.list_dirty = true;
        if !replaced.is_empty() {
            // Waveform and gain are measured again; the grid (possibly edited by hand, and a tag
            // edit in another program also changes the size) is kept for the user to check.
            self.toast(format!("{} audio file(s) changed on disk since they were analysed. Their waveforms are being measured again; check their beat grids.", replaced.len()));
            self.analyze(&replaced, true);
        }
    }
    /// Move a playlist/folder before or after `target`, or into `target` when it is a folder.
    /// Refuses to put a folder inside itself or one of its own subfolders.
    pub fn drop_node(&mut self, id: i64, target: i64, zone: i8) {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        if id == target {
            return;
        }
        let Some(t) = self.node(target).cloned() else { return };
        let (parent, seq) = if zone == 0 && t.kind == NodeKind::Folder {
            (t.id, self.nodes.iter().filter(|n| n.parent == t.id).count() as i64)
        } else if zone < 0 {
            (t.parent, t.seq)
        } else {
            (t.parent, t.seq + 1)
        };
        let mut p = parent;
        while p != 0 {
            if p == id {
                self.toast_err("A folder cannot be moved into itself.");
                return;
            }
            p = self.node(p).map(|n| n.parent).unwrap_or(0);
        }
        let _ = self.db.move_node(id, parent, seq);
        // Renumber the destination's children so their order stays contiguous.
        if let Ok(all) = self.db.playlists() {
            let mut kids: Vec<_> = all.into_iter().filter(|n| n.parent == parent).collect();
            kids.sort_by_key(|n| (n.seq, n.id != id));
            for (i, n) in kids.iter().enumerate() {
                let _ = self.db.conn.execute("UPDATE playlists SET seq=?2 WHERE id=?1", rusqlite::params![n.id, i as i64]);
            }
        }
        if zone == 0 && t.kind == NodeKind::Folder {
            self.expanded.insert(t.id);
        }
        self.reload_playlists();
    }

    pub fn reload_playlists(&mut self) {
        self.nodes = self.db.playlists().unwrap_or_default();
        self.node_counts = self.db.playlist_counts().unwrap_or_default();
        self.list_dirty = true;
    }
    pub fn reload_tags(&mut self) {
        self.mytags = self.db.mytags().unwrap_or_default();
        self.track_tags = self.db.track_tags().unwrap_or_default();
        self.list_dirty = true;
    }

    pub fn track(&self, id: i64) -> Option<&Track> {
        self.index.get(&id).map(|i| &self.tracks[*i])
    }
    pub fn track_mut(&mut self, id: i64) -> Option<&mut Track> {
        let i = *self.index.get(&id)?;
        // Metadata dialogs, inline edits, tag reloads and Undo/Redo can update a track
        // without reloading the library. Invalidate before handing out mutable access.
        self.collection_categories.clear();
        Some(&mut self.tracks[i])
    }
    /// Save a track without recording an undo step (analysis, sync and other system updates).
    pub fn save_track_quiet(&mut self, id: i64) {
        if let Some(t) = self.track(id).cloned() {
            let mut t = t;
            if t.analyzed == 3 {
                t.analyzed = 0;
            }
            let _ = self.db.update_track(&t);
        }
        self.list_dirty = true;
    }

    pub fn save_track(&mut self, id: i64) {
        // The database still holds the previous values: keep them for Undo.
        if let Some(before) = self.db.track(id) {
            if self.track(id).map(|t| format!("{t:?}") != format!("{before:?}")).unwrap_or(false) {
                self.record(Edit::Track { before });
            }
        }
        if let Some(t) = self.track(id).cloned() {
            let mut t = t;
            if t.analyzed == 3 {
                t.analyzed = 0;
            }
            let _ = self.db.update_track(&t);
        }
        self.list_dirty = true;
    }

    pub fn node(&self, id: i64) -> Option<&PlaylistNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    // ------------------------------------------------------------------ list
    pub fn smart_tracks(&self, rule: &SmartRule) -> Vec<i64> {
        let today = crate::smart::today_days();
        let empty = HashSet::new();
        self.tracks
            .iter()
            .filter(|t| crate::smart::matches(rule, t, self.track_tags.get(&t.id).unwrap_or(&empty), today))
            .map(|t| t.id)
            .collect()
    }

    pub fn view_tracks(&self) -> Vec<i64> {
        self.view_tracks_of(&self.view)
    }

    /// Tracks of any list view (the main browser's or the sub-browser's).
    pub fn view_tracks_of(&self, view: &View) -> Vec<i64> {
        match view {
            View::Collection => self.tracks.iter().map(|t| t.id).collect(),
            View::Playlist(id) => match self.node(*id) {
                Some(n) if n.kind == NodeKind::Smart => n.smart.as_ref().map(|r| self.smart_tracks(r)).unwrap_or_default(),
                Some(n) if n.kind == NodeKind::Folder => {
                    let mut out = vec![];
                    let mut seen = HashSet::new();
                    self.folder_tracks(n.id, &mut out, &mut seen);
                    out
                }
                Some(_) => self.db.playlist_tracks(*id).unwrap_or_default().into_iter().filter(|t| self.index.contains_key(t)).collect(),
                None => vec![],
            },
            View::Device { .. } | View::DeviceHistory { .. } | View::Explorer(_) | View::Source { .. } => vec![],
            View::Sampler(_) => vec![],
            View::History(id) => self.db.history_tracks(*id).unwrap_or_default().into_iter().filter(|t| self.index.contains_key(t)).collect(),
            View::TagList => self.ext.tag_list.iter().copied().filter(|t| self.index.contains_key(t)).collect(),
            View::Matching => self.related_source().map(|s| self.matching_of(s)).unwrap_or_default(),
            View::Related => self.related_tracks(),
            View::Suggest(kind) => self.related_source().map(|s| self.suggestions(s, 0, *kind).into_iter().map(|x| x.0).collect()).unwrap_or_default(),
            View::Category { field, value } => self
                .tracks
                .iter()
                .filter(|t| &Self::category_value(t, *field) == value)
                .map(|t| t.id)
                .collect(),
        }
    }

    /// The value a track has for one Collection category (Date Added is grouped by day).
    pub fn category_value(t: &Track, field: u8) -> String {
        match field {
            0 => t.date_added.get(..10).unwrap_or(&t.date_added).to_string(),
            1 => t.genre.trim().to_string(),
            2 => t.artist.trim().to_string(),
            _ => t.album.trim().to_string(),
        }
    }

    /// Track the Related Tracks list is built around: the loaded track, else the selection.
    pub fn related_source(&self) -> Option<i64> {
        self.deck.track.or_else(|| self.anchor_id).or_else(|| self.selected.iter().next().copied())
    }

    /// Tracks matching the Related Tracks rules for the source track, best matches first.
    pub fn related_tracks(&self) -> Vec<i64> {
        self.related_source().map(|s| self.related_tracks_of(s)).unwrap_or_default()
    }

    /// The same for any source track.
    pub fn related_tracks_of(&self, src: i64) -> Vec<i64> {
        let Some(src) = self.track(src).cloned() else { return vec![] };
        let r = &self.prefs.related;
        let mut out: Vec<(i64, i64)> = self
            .tracks
            .iter()
            .filter(|t| t.id != src.id && !t.missing)
            .filter_map(|t| {
                let mut score = 0i64;
                if r.matching && !self.is_match(src.id, t.id) {
                    return None;
                }
                if r.key > 0 {
                    let d = key_distance(src.key, t.key)?;
                    if d > r.key as i32 - 1 {
                        return None;
                    }
                    score += (4 - d as i64) * 100;
                }
                if r.bpm {
                    if src.bpm <= 0.0 || t.bpm <= 0.0 {
                        return None;
                    }
                    let best = [t.bpm, t.bpm * 2.0, t.bpm / 2.0].iter().map(|b| ((b - src.bpm) / src.bpm * 100.0).abs()).fold(f64::MAX, f64::min);
                    if best > r.bpm_pct {
                        return None;
                    }
                    score += (100.0 - best * 10.0).max(0.0) as i64;
                }
                let same = |a: &str, b: &str| !a.trim().is_empty() && a.trim().eq_ignore_ascii_case(b.trim());
                if r.genre && !same(&src.genre, &t.genre) {
                    return None;
                }
                if r.artist && !same(&src.artist, &t.artist) {
                    return None;
                }
                if r.label && !same(&src.label, &t.label) {
                    return None;
                }
                if r.color && (src.color == 0 || src.color != t.color) {
                    return None;
                }
                if r.rating && t.rating < src.rating {
                    return None;
                }
                Some((t.id, score))
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out.into_iter().map(|x| x.0).collect()
    }

    fn folder_tracks(&self, id: i64, out: &mut Vec<i64>, seen: &mut HashSet<i64>) {
        for n in self.nodes.iter().filter(|n| n.parent == id) {
            match n.kind {
                NodeKind::Folder => self.folder_tracks(n.id, out, seen),
                NodeKind::Smart => {
                    for t in n.smart.as_ref().map(|r| self.smart_tracks(r)).unwrap_or_default() {
                        if seen.insert(t) {
                            out.push(t);
                        }
                    }
                }
                NodeKind::Playlist => {
                    for t in self.db.playlist_tracks(n.id).unwrap_or_default() {
                        if self.index.contains_key(&t) && seen.insert(t) {
                            out.push(t);
                        }
                    }
                }
            }
        }
    }

    pub fn is_manual_playlist(&self) -> Option<i64> {
        match self.view {
            View::Playlist(id) => self.node(id).filter(|n| n.kind == NodeKind::Playlist).map(|n| n.id),
            _ => None,
        }
    }

    pub fn tag_text(&self, id: i64) -> String {
        let names: HashMap<i64, &str> = self.mytags.iter().map(|t| (t.id, t.name.as_str())).collect();
        let mut v: Vec<&str> = self.track_tags.get(&id).map(|s| s.iter().filter_map(|x| names.get(x).copied()).collect()).unwrap_or_default();
        v.sort();
        v.join(", ")
    }

    pub fn rebuild_list(&mut self) {
        self.related_built_for = self.related_source();
        let base = self.view_tracks();
        let order: HashMap<i64, usize> = base.iter().enumerate().map(|(i, t)| (*t, i)).collect();
        let q = crate::smart::fold(self.search.trim());
        let words: Vec<&str> = q.split_whitespace().collect();
        // My Tag names once for the whole pass (not once per track looked at).
        let tag_names: HashMap<i64, &str> = self.mytags.iter().map(|t| (t.id, t.name.as_str())).collect();
        let tag_text = |id: i64| -> String {
            let mut v: Vec<&str> = self.track_tags.get(&id).map(|s| s.iter().filter_map(|x| tag_names.get(x).copied()).collect()).unwrap_or_default();
            v.sort();
            v.join(", ")
        };
        let mut v: Vec<i64> = base
            .into_iter()
            .filter(|id| {
                let Some(t) = self.track(*id) else { return false };
                if !words.is_empty() {
                    let hay = match self.prefs.search_scope {
                        1 => crate::smart::fold(&t.display_title()),
                        2 => crate::smart::fold(&format!("{} {} {}", t.artist, t.remixer, t.original_artist)),
                        3 => crate::smart::fold(&t.album),
                        4 => crate::smart::fold(&t.genre),
                        5 => crate::smart::fold(&t.label),
                        6 => crate::smart::fold(&t.comment),
                        7 => crate::smart::fold(&tag_text(t.id)),
                        _ => crate::smart::fold(&format!(
                        "{} {} {} {} {} {} {} {} {} {} {}",
                        t.display_title(),
                        t.artist,
                        t.album,
                        t.genre,
                        t.label,
                        t.remixer,
                        t.comment,
                        t.composer,
                        t.mix_name,
                        format!("{} {}", t.key_label(self.prefs.key_alnum), key_name(t.key, self.prefs.key_alnum)),
                        tag_text(t.id)
                    )),
                    };
                    if !words.iter().all(|w| hay.contains(w)) {
                        return false;
                    }
                }
                if !crate::ui_edit::attribute_pass(self, t.id) {
                    return false;
                }
                if self.filter_open {
                    if !crate::library_ext::filter_pass(&self.ext.filter, t) {
                        return false;
                    }
                    if self.ext.filter.tags_on && !self.filter_tags.is_empty() {
                        let empty = HashSet::new();
                        let tt = self.track_tags.get(&t.id).unwrap_or(&empty);
                        let ok = if self.filter_tags_all { self.filter_tags.iter().all(|x| tt.contains(x)) } else { self.filter_tags.iter().any(|x| tt.contains(x)) };
                        if !ok {
                            return false;
                        }
                    }
                }
                true
            })
            .collect();
        // Column browser: each column lists the values left after the columns to its left.
        if self.prefs.column_browser {
            let field = |t: &Track, i: usize| -> String {
                match i {
                    0 => t.genre.trim().to_string(),
                    1 => t.artist.trim().to_string(),
                    _ => t.album.trim().to_string(),
                }
            };
            let mut values: [Vec<(String, usize)>; 3] = Default::default();
            for i in 0..3 {
                let mut counts: HashMap<String, usize> = HashMap::new();
                for id in &v {
                    if let Some(t) = self.track(*id) {
                        *counts.entry(field(t, i)).or_default() += 1;
                    }
                }
                let mut list: Vec<(String, usize)> = counts.into_iter().collect();
                list.sort_by(|a, b| (a.0.is_empty(), crate::smart::fold(&a.0)).cmp(&(b.0.is_empty(), crate::smart::fold(&b.0))));
                // A selection that no longer exists here falls back to All.
                if let Some(sel) = &self.colbrowse[i] {
                    if !list.iter().any(|(x, _)| x == sel) {
                        self.colbrowse[i] = None;
                    }
                }
                if let Some(sel) = self.colbrowse[i].clone() {
                    v.retain(|id| self.track(*id).map(|t| field(t, i) == sel).unwrap_or(false));
                }
                values[i] = list;
            }
            self.colbrowse_values = values;
        }
        let col = self.prefs.sort_col;
        let asc = self.prefs.sort_asc;
        let alnum = self.prefs.key_alnum;
        let tagtexts: HashMap<i64, String> = if col == Col::MyTag { v.iter().map(|id| (*id, tag_text(*id))).collect() } else { HashMap::new() };
        // Text columns compare case- and accent-folded text: fold each row's text once, not
        // twice per comparison.
        let folded: HashMap<i64, String> = v
            .iter()
            .filter_map(|id| {
                let t = self.track(*id)?;
                let text = match col {
                    Col::Title => t.display_title(),
                    Col::Artist => t.artist.clone(),
                    Col::Album => t.album.clone(),
                    Col::Genre => t.genre.clone(),
                    Col::Comments => t.comment.clone(),
                    Col::Label => t.label.clone(),
                    Col::Remixer => t.remixer.clone(),
                    Col::OriginalArtist => t.original_artist.clone(),
                    Col::Composer => t.composer.clone(),
                    Col::FileName => t.file_name(),
                    Col::MixName => t.mix_name.clone(),
                    Col::AlbumArtist => t.album_artist.clone(),
                    Col::Grouping => t.grouping.clone(),
                    Col::Lyricist => t.lyricist.clone(),
                    Col::Message => t.message.clone(),
                    _ => return None,
                };
                Some((*id, crate::smart::fold(&text)))
            })
            .collect();
        v.sort_by(|a, b| {
            if let (Some(x), Some(y)) = (folded.get(a), folded.get(b)) {
                let o = x.cmp(y);
                let o = if asc { o } else { o.reverse() };
                return o.then(order[a].cmp(&order[b]));
            }
            let ta = self.track(*a).unwrap();
            let tb = self.track(*b).unwrap();
            use std::cmp::Ordering;
            let tc = |x: &str, y: &str| crate::smart::fold(x).cmp(&crate::smart::fold(y));
            let fc = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
            let o = match col {
                Col::Num | Col::Status | Col::Preview | Col::Artwork => order[a].cmp(&order[b]),
                Col::Title => tc(&ta.display_title(), &tb.display_title()),
                Col::Artist => tc(&ta.artist, &tb.artist),
                Col::Album => tc(&ta.album, &tb.album),
                Col::Genre => tc(&ta.genre, &tb.genre),
                Col::Bpm => fc(ta.bpm, tb.bpm),
                Col::Rating => ta.rating.cmp(&tb.rating),
                Col::Time => fc(ta.duration, tb.duration),
                Col::Key => {
                    if alnum {
                        (ta.key % 12, ta.key / 12).cmp(&(tb.key % 12, tb.key / 12))
                    } else {
                        key_name(ta.key, false).cmp(&key_name(tb.key, false))
                    }
                }
                Col::DateAdded => ta.date_added.cmp(&tb.date_added).then(ta.id.cmp(&tb.id)),
                Col::Color => ta.color.cmp(&tb.color),
                Col::Comments => tc(&ta.comment, &tb.comment),
                Col::Label => tc(&ta.label, &tb.label),
                Col::Remixer => tc(&ta.remixer, &tb.remixer),
                Col::OriginalArtist => tc(&ta.original_artist, &tb.original_artist),
                Col::Composer => tc(&ta.composer, &tb.composer),
                Col::Year => ta.year.cmp(&tb.year),
                Col::Bitrate => ta.bitrate.cmp(&tb.bitrate),
                Col::SampleRate => ta.sample_rate.cmp(&tb.sample_rate),
                Col::FileType => ta.file_type.cmp(&tb.file_type),
                Col::Size => ta.file_size.cmp(&tb.file_size),
                Col::Location => ta.path.cmp(&tb.path),
                Col::FileName => tc(&ta.file_name(), &tb.file_name()),
                Col::PlayCount => ta.play_count.cmp(&tb.play_count),
                Col::MixName => tc(&ta.mix_name, &tb.mix_name),
                Col::TrackNumber => ta.track_no.cmp(&tb.track_no),
                Col::MyTag => tagtexts.get(a).cmp(&tagtexts.get(b)),
                Col::ReleaseDate => ta.release_date.cmp(&tb.release_date),
                Col::AlbumArtist => tc(&ta.album_artist, &tb.album_artist),
                Col::DiscNumber => ta.disc_no.cmp(&tb.disc_no),
                Col::Bitdepth => ta.bit_depth.cmp(&tb.bit_depth),
                Col::Grouping => tc(&ta.grouping, &tb.grouping),
                Col::Lyricist => tc(&ta.lyricist, &tb.lyricist),
                Col::Message => tc(&ta.message, &tb.message),
                Col::HotCue => self.cue_counts.get(a).map(|c| c.0).cmp(&self.cue_counts.get(b).map(|c| c.0)),
                Col::DateCreated => ta.date_created.cmp(&tb.date_created),
            };
            let o = if asc { o } else { o.reverse() };
            o.then(order[a].cmp(&order[b]))
        });
        self.ext.list_num = v.iter().map(|id| order[id] + 1).collect();
        self.list = v;
        let idx = &self.index;
        self.selected.retain(|id| idx.contains_key(id));
        self.anchor = self.anchor_id.and_then(|id| self.list.iter().position(|x| *x == id));
        self.list_dirty = false;
    }

    /// Phrases for the loaded track, worked out on a background thread: rekordbox's own while
    /// its grid is unchanged, else analysed from the decoded audio on the current grid.
    fn start_deck_phrases(&mut self, a: Arc<Decoded>) {
        let Some(id) = self.deck.track else { return };
        // Phrases edited with PHRASE EDIT take the place of analysed ones.
        if let Some(st) = self.stored_phrases(id) {
            self.deck.phrases = Some(st);
            self.deck.x.phrases_edited = true;
            return;
        }
        self.deck.x.phrases_edited = false;
        if !self.prefs.phrase_analysis {
            return;
        }
        let Some(path) = self.track(id).map(|t| t.path.clone()) else { return };
        let map: HashMap<String, String> = self.db.setting("rb_anlz").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let base = map.get(&path).map(std::path::PathBuf::from);
        let grid = self.deck.grid.clone();
        let beats = self.deck.beats.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        let cancel = self.deck.cancel_work.clone();
        std::thread::spawn(move || {
            let from_rb = base.and_then(|b| {
                let dat = b.with_extension("DAT");
                let same = crate::rbimport::read_pqtz_grid(&dat).map(|r| grids_match(&r, &grid)).unwrap_or(false);
                if !same || !crate::rbimport::pqtz_beat_indices_match(&dat, &beats) {
                    return None;
                }
                crate::export::anlz::read_sections(&b.with_extension("EXT")).get("PSSI").and_then(|s| crate::phrase::parse(s))
            });
            let r = from_rb.or_else(|| if beats.is_empty() || cancel.load(std::sync::atomic::Ordering::Relaxed) { None } else { crate::phrase::analyze(&a.mono(), a.sr as f64, &beats) });
            let _ = tx.send((id, r));
            ctx.request_repaint();
        });
        self.deck.phrases_rx = Some(rx);
    }

/// Vocal detection for the loaded track when neither rekordbox nor an earlier run has it:
    /// the vocal stem of an already-downloaded separation model, in the background.
    fn start_deck_vocal(&mut self, a: Arc<Decoded>) {
        if MANUAL_STEMS_PREVIEW || !self.prefs.vocal_analysis || self.deck.vocal.is_some() || self.screenshot.is_some() {
            return;
        }
        // rekordbox's detector (CPU, about a second per track) unless PREPDECK_VOCAL=stems or
        // rekordbox is not installed.
        let rb = !crate::vocal_rb::use_stems();
        // The stem model only when downloaded and with a GPU (about a minute per track there;
        // much slower on the CPU).
        if !rb && (!crate::stems::CATALOG.iter().any(crate::stems::model_ready) || !self.stem.gpu_available()) {
            return;
        }
        let Some(id) = self.deck.track else { return };
        if self.vocal_failed.contains(&id) || self.loading_stems() || self.stem.separating.is_some_and(|(track, _)| track == id) {
            return;
        }
        let path = self.track(id).map(|t| t.path.clone()).unwrap_or_default();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        let cancel = self.deck.cancel_work.clone();
        std::thread::spawn(move || {
            let r = if rb && !cancel.load(std::sync::atomic::Ordering::Relaxed) {
                crate::vocal_rb::analyse_file(&path, Some(&a))
            } else if !cancel.load(std::sync::atomic::Ordering::Relaxed) && crate::stems::gpu_active() {
                crate::stems::separate_vocals(&a.samples, a.sr, &mut |_| !cancel.load(std::sync::atomic::Ordering::Relaxed)).ok().map(|v| crate::stems::vocal_levels(&v, &crate::stems::mono_44k(&a.samples, a.sr)))
            } else { None };
            let _ = tx.send((id, r));
            ctx.request_repaint();
        });
        self.deck.vocal_rx = Some(rx);
    }

/// Track > Analyze Key. With a downloaded separation model and a GPU the key is found on the
    /// mix without its drums (the drums stem subtracted), which agrees with rekordbox more
    /// often; otherwise with the ordinary analysis.
    pub fn analyze_key_tracks(&mut self, ids: &[i64]) {
        let stems_ok = !MANUAL_STEMS_PREVIEW && crate::stems::CATALOG.iter().any(crate::stems::model_ready) && self.stem.gpu_available();
        // Locked tracks keep their key (Analysis Lock).
        let ids: Vec<i64> = ids.iter().copied().filter(|id| self.track(*id).map(|t| !(t.grid_locked && t.analyzed == 1)).unwrap_or(false)).collect();
        let ids = &ids[..];
        if !stems_ok || self.key_job.is_some() {
            // Ordinary analysis with key detection on whatever the preference says; the key is
            // replaced only when detection succeeds (the old one stays otherwise).
            // Only tracks actually submitted get the request (missing or already queued ones
            // would otherwise carry it to some later, unrelated analysis).
            let submitted = self.analyze_forcing_key(ids);
            self.key_requested.extend(submitted);
            return;
        }
        let jobs: Vec<(i64, String)> = ids.iter().filter_map(|id| self.track(*id).filter(|t| !t.missing).map(|t| (t.id, t.path.clone()))).collect();
        let (tx, rx) = crossbeam_channel::unbounded();
        let ctx = self.ctx.clone();
        let n = jobs.len();
        std::thread::spawn(move || {
            for (id, path) in jobs {
                let key = (|| {
                    let d = crate::audio::decode_file(&path).ok()?;
                    let mono = crate::stems::mono_44k(&d.samples, d.sr);
                    if !crate::stems::gpu_active() { return Some(crate::key::detect_key(&mono, 44100.0)); }
                    let drums = match crate::stems::separate_one(0, &d.samples, d.sr, &mut |_| true) {
                        Ok(drums) => drums,
                        Err(_) if crate::stems::backend_hold().is_some() => return Some(crate::key::detect_key(&mono, 44100.0)),
                        Err(_) => return None,
                    };
                    let harm: Vec<f32> = mono.iter().zip(&drums).map(|(m, dr)| m - dr).collect();
                    Some(crate::key::detect_key(&harm, 44100.0))
                })()
                .unwrap_or(-1);
                if tx.send((id, key)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        self.key_job = Some((rx, n, 0));
        self.toast(format!("Analyzing the key of {n} track(s) with the stem model (about a minute each)…"));
    }

    pub fn poll_key_job(&mut self) {
        let Some((rx, n, done)) = &mut self.key_job else { return };
        let mut results = vec![];
        while let Ok(r) = rx.try_recv() {
            results.push(r);
            *done += 1;
        }
        let finished = *done >= *n;
        let total = *n;
        for (id, key) in results {
            if key >= 0 && self.track(id).map(|t| t.key != key || t.key_manual).unwrap_or(false) {
                if let Some(t) = self.track_mut(id) {
                    t.set_key(key);
                    t.key_manual = false;
                }
                self.save_track(id);
            }
        }
        if finished {
            self.key_job = None;
            self.list_dirty = true;
            self.toast(format!("Key analysis finished for {total} track(s)."));
        }
    }

/// The audio device went away (unplugged, switched off): stop, say so once, and try to
    /// reopen the chosen output (else the default) every two seconds until it works.
    pub fn poll_audio_device(&mut self) {
        if !self.player.lost.load(std::sync::atomic::Ordering::Relaxed) {
            self.audio_retry = None;
            return;
        }
        let (last, told) = self.audio_retry.unwrap_or((Instant::now() - std::time::Duration::from_secs(10), false));
        if !told {
            self.player.state.lock().playing = false;
            self.toast_err("The audio output stopped working (device disconnected?). recordbox will reconnect when it is available.");
        }
        if last.elapsed() >= std::time::Duration::from_secs(2) {
            // The system's default output: opening it needs no device search (which can block),
            // so this never stalls the window. A chosen device is used again from the next launch
            // or by picking it in Preferences > Audio.
            match self.player.switch_device(None) {
                Ok(()) => {
                    self.audio_retry = None;
                    self.toast("Audio output reconnected (system default output).");
                    return;
                }
                Err(_) => self.audio_retry = Some((Instant::now(), true)),
            }
        } else {
            self.audio_retry = Some((last, true));
        }
        self.ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    pub fn poll_vocal(&mut self) {
        let Some(rx) = &self.deck.vocal_rx else { return };
        if let Ok((id, r)) = rx.try_recv() {
            self.deck.vocal_rx = None;
            if r.is_none() {
                self.vocal_failed.insert(id);
            }
            if let Some(levels) = r {
                let _ = self.db.save_vocal(id, &levels);
                if self.deck.track == Some(id) {
                    self.deck.vocal = Some(std::sync::Arc::new((crate::stems::VOCAL_HOP as f64 / 44100.0, levels)));
                }
            }
        }
    }

    pub fn poll_phrases(&mut self) {
        let Some(rx) = &self.deck.phrases_rx else { return };
        if let Ok((id, r)) = rx.try_recv() {
            self.deck.phrases_rx = None;
            if self.deck.track == Some(id) {
                self.deck.phrases = r;
            }
        }
    }

    /// rekordbox's PWV7 (detail) and PWV6 (overview) 3-band waveforms for an audio file.
    fn rb_waveforms(&self, path: &str) -> Option<std::sync::Arc<(Vec<[u8; 3]>, Vec<[u8; 3]>)>> {
        let map: HashMap<String, String> = self.db.setting("rb_anlz").and_then(|s| serde_json::from_str(&s).ok())?;
        let base = std::path::PathBuf::from(map.get(path)?);
        let sec = crate::export::anlz::read_sections(&base.with_extension("2EX"));
        let triples = |s: &Vec<u8>| -> Option<Vec<[u8; 3]>> {
            let hlen = u32::from_be_bytes(s.get(4..8)?.try_into().ok()?) as usize;
            let n = u32::from_be_bytes(s.get(16..20)?.try_into().ok()?) as usize;
            let body = s.get(hlen..hlen + n * 3)?;
            Some(body.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect())
        };
        let detail = triples(sec.get("PWV7")?)?;
        let overview = triples(sec.get("PWV6")?)?;
        Some(std::sync::Arc::new((detail, overview)))
    }

/// rekordbox's vocal detection for this audio file (PVDI in the .2EX file: a u16 hop in
    /// samples and u16 sample rate, then one level 0–4 per hop).
    fn rb_vocals(&self, path: &str) -> Option<std::sync::Arc<(f64, Vec<u8>)>> {
        let map: HashMap<String, String> = self.db.setting("rb_anlz").and_then(|s| serde_json::from_str(&s).ok())?;
        let base = std::path::PathBuf::from(map.get(path)?);
        let sec = crate::export::anlz::read_sections(&base.with_extension("2EX"));
        parse_pvdi(sec.get("PVDI")?)
    }

    /// Point the selection anchor at row `i` of the current list.
    pub fn set_anchor(&mut self, i: usize) {
        self.anchor = Some(i);
        self.anchor_id = self.list.get(i).copied();
    }

    pub fn set_view(&mut self, v: View) {
        if self.view != v {
            self.view = v;
            self.colbrowse = Default::default();
            let manual = matches!(self.view, View::History(_) | View::TagList | View::Suggest(_)) || matches!(self.view, View::Playlist(id) if self.node(id).map(|n| n.kind == NodeKind::Playlist).unwrap_or(false));
            // Switch to the other column set and sort when moving between playlists and the
            // rest; each keeps the user's own choices.
            if manual != self.prefs.columns_for_playlist {
                let p = &mut self.prefs;
                std::mem::swap(&mut p.columns, &mut p.columns_other);
                let cur = (p.sort_col, p.sort_asc);
                (p.sort_col, p.sort_asc) = p.sort_other;
                p.sort_other = cur;
                p.columns_for_playlist = manual;
                if manual && !p.columns.contains(&Col::Num) {
                    p.columns.insert(0, Col::Num);
                }
                if !manual {
                    p.columns.retain(|c| *c != Col::Num);
                    if p.sort_col == Col::Num {
                        p.sort_col = Col::DateAdded;
                        p.sort_asc = false;
                    }
                }
            }
            self.list_dirty = true;
            self.anchor = None;
            self.anchor_id = None;
        }
    }

    // ------------------------------------------------------------------ import
    pub fn import_paths(&mut self, paths: Vec<std::path::PathBuf>, to_playlist: Option<i64>) {
        if crate::prefs_bridge::import_protected(self) {
            return;
        }
        if paths.is_empty() {
            return;
        }
        self.import_to_playlist = to_playlist;
        self.import_count = 0;
        self.importing = Some(spawn_import(paths, self.ctx.clone()));
    }

    pub fn poll_import(&mut self) {
        let Some(rx) = self.importing.clone() else { return };
        let mut added = vec![];
        let mut finished = false;
        while let Ok(m) = rx.try_recv() {
            match m {
                Some(mut t) => {
                    if let Some(id) = self.db.track_by_path(&t.path) {
                        added.push(id);
                        continue;
                    }
                    // Preferences > DJ System > Others: Hot Cue Auto Load for imported tracks.
                    t.autoload_hotcue = self.prefs.hot_cue_auto_load_on_import;
                    if let Ok(id) = self.db.insert_local_file(&t) {
                        added.push(id);
                        self.import_count += 1;
                    }
                }
                None => finished = true,
            }
        }
        if !added.is_empty() {
            self.reload_tracks();
            if let Some(pl) = self.import_to_playlist {
                let mut cur = self.db.playlist_tracks(pl).unwrap_or_default();
                for a in &added {
                    if !cur.contains(a) {
                        cur.push(*a);
                    }
                }
                let _ = self.db.set_playlist_tracks(pl, &cur);
                self.reload_playlists();
            }
            if self.prefs.auto_analysis {
                let todo: Vec<i64> = added.iter().copied().filter(|id| self.track(*id).map(|t| t.analyzed == 0).unwrap_or(false)).collect();
                self.analyze(&todo, false);
            }
        }
        if finished {
            self.importing = None;
            let n = self.import_count;
            self.toast(format!("{n} track(s) added to the collection"));
            if let Some(p) = self.load_after_import.take() {
                if let Some(id) = self.db.track_by_path(&p) {
                    self.load_track(id);
                }
            }
        }
    }

    // ------------------------------------------------------------------ stems
    /// Assess the hardware, settle on a model and download it if needed (background thread).
    pub fn stems_init(&mut self) {
        let tx = self.stem.tx.clone();
        let ctx = self.ctx.clone();
        let wanted = std::env::var("PREPDECK_STEM_MODEL").ok().or_else(|| self.prefs.stem_model.clone());
        std::thread::spawn(move || {
            let hw = crate::stems::assess();
            let m = wanted.as_deref().and_then(crate::stems::spec).unwrap_or_else(|| crate::stems::choose(&hw));
            let _ = tx.send(StemMsg::Hardware(hw, m.id.to_string()));
            ctx.request_repaint();
        });
    }

    pub fn stems_download(&mut self) {
        let Some(m) = self.stem.model.as_deref().and_then(crate::stems::spec) else { return };
        self.stems_download_for(m);
    }

    fn stems_download_for(&mut self, m: &'static crate::stems::ModelSpec) {
        if self.stem.downloading.is_some() {
            return;
        }
        self.stem.download_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        self.stem.downloading = Some((0, 0));
        self.stem.errors.remove(&None);
        let tx = self.stem.tx.clone();
        let ctx = self.ctx.clone();
        let cancel = self.stem.download_cancel.clone();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let ctx2 = ctx.clone();
            let mut last = std::time::Instant::now();
            let r = crate::stems::download(m, &mut |d, t, _| {
                if last.elapsed().as_millis() > 200 {
                    let _ = tx2.send(StemMsg::Download(d, t));
                    ctx2.request_repaint();
                    last = std::time::Instant::now();
                }
            }, &cancel);
            let _ = tx.send(StemMsg::DownloadDone(r.map_err(|e| e.to_string())));
            ctx.request_repaint();
        });
    }

    /// Stems for a track that has just been loaded: the kept ones if this track was separated
    /// before, else (as rekordbox does on every load while STEMS is on) a fresh separation in
    /// the background, during which the original plays. A separation still running for a
    /// track that is no longer on a deck is stopped.
    fn stems_for_loaded(&mut self, id: i64) {
        if self.deck.audio.is_none() { return; }
        let Some(model) = self.stem.model.as_deref().and_then(crate::stems::spec) else { return };
        // An early Separate or Retry CPU click has already chosen the model for this deck.
        let Some(key) = self.deck.stem_cache_key.clone().or_else(|| self.stem_cache_key(id, model.id)) else { return };
        if self.load_cached_stems(&key, None) { return; }
        let quiet = self.screenshot.is_some() && std::env::var("PREPDECK_SHOT_STEMS").is_err();
        let ready = crate::stems::spec(key.model).is_some_and(crate::stems::model_ready);
        if self.prefs.auto_stems_on_load(self.stem.gpu_available()) && ready && !quiet && self.deck.track == Some(id) {
            let execution = if self.prefs.stems_auto_cpu { crate::stems::SeparationDevice::Selected } else { crate::stems::SeparationDevice::GpuOnly };
            self.start_stem_separation(key, execution);
        }
    }

    /// Keep work shared by two decks until the last one releases the track.
    pub(crate) fn cancel_orphaned_stems(&mut self) {
        let tracks = std::array::from_fn(|n| self.deck_of(n as u8).track);
        self.stem.retain_deck_tracks(tracks);
    }

    fn resume_pending_stems(&mut self) {
        let Some((key, execution)) = self.stem.pending.take() else { return };
        if let Some(deck) = (0..4).find(|&n| self.deck_of(n).stem_cache_key.as_ref() == Some(&key)) {
            self.with_deck(deck, |app| {
                if !app.load_cached_stems(&key, Some(execution))
                    && (execution == crate::stems::SeparationDevice::Cpu || crate::stems::backend_hold().is_none()) {
                    app.start_stem_separation(key, execution);
                }
            });
        }
    }

    fn stem_cache_key(&self, id: i64, model: &'static str) -> Option<StemCacheKey> {
        let path = crate::stems::cache_path(&self.track(id)?.path, model);
        Some(StemCacheKey { track: id, model, path })
    }

    pub(crate) fn loading_stems(&self) -> bool {
        self.deck.stem_cache_key.as_ref().is_some_and(|key| self.stem.loading.contains_key(key))
    }

    pub(crate) fn separating_stems(&self) -> bool {
        self.deck.stem_cache_key.as_ref().is_some_and(|key| self.stem.active_key.as_ref() == Some(key))
    }

    // A model change can complete while reads or inference for the previous model are in flight.
    pub(crate) fn select_stem_model(&mut self, model: &'static crate::stems::ModelSpec) {
        self.prefs.stem_model = Some(model.id.to_owned());
        self.stem.model = self.prefs.stem_model.clone();
        self.stem.model_ready = crate::stems::model_ready(model);
        for n in 0..4 {
            self.with_deck(n, |app| {
                if let Some(key) = app.deck.track.and_then(|id| app.stem_cache_key(id, model.id)) {
                    app.load_cached_stems(&key, None);
                }
            });
        }
    }

    /// A cache hit or an in-flight read satisfies the request without a model download or inference.
    fn load_cached_stems(&mut self, key: &StemCacheKey, on_failure: Option<crate::stems::SeparationDevice>) -> bool {
        if self.deck.stem_cache_key.as_ref() != Some(key) {
            self.deck.stem_cache_key = Some(key.clone());
            if self.stem.errors.get(&Some(key.track)).is_some_and(|error| error.retry_model.is_none()) {
                self.stem.errors.remove(&Some(key.track));
            }
            let mut player = self.player.state.lock();
            player.stems = None;
            player.stem_preview = None;
            self.deck.x.stem_wave = None;
            self.deck.x.stem_wave_rx = None;
        }
        if self.player.state.lock().stems.is_some() { return true; }
        if let Some(parts) = (0..4).filter(|&n| self.deck_of(n).stem_cache_key.as_ref() == Some(key))
            .find_map(|n| self.player.decks[n as usize].lock().stems.clone()) {
            self.player.state.lock().stems = Some(parts);
            return true;
        }
        if self.stem.active_key.as_ref() == Some(key) && !self.stem.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(preview) = &self.stem.preview {
                self.player.state.lock().stem_preview = Some(preview.clone());
            }
        }
        if let Some(request) = self.stem.loading.get_mut(key) {
            if on_failure.is_some() { *request = on_failure; }
            return true;
        }
        let Some(saved) = crate::stems::existing_cache_path(&key.path) else { return false };
        let key = key.clone();
        self.stem.loading.insert(key.clone(), on_failure);
        let tx = self.stem.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = crate::stems::load(&saved).map(Arc::new).map_err(|error| anyhow::anyhow!("{}: {error:#}", saved.display()));
            let _ = tx.send(StemMsg::Cached(key, r));
            ctx.request_repaint();
        });
        true
    }

    pub fn manual_stem_device(&self) -> crate::stems::SeparationDevice {
        self.stem.manual_device(self.deck.track, crate::stems::backend_hold().is_some())
    }

    /// Separate the loaded track (downloading the model first if it is missing).
    pub fn separate_loaded(&mut self) {
        self.separate_loaded_on(self.manual_stem_device());
    }

    pub fn separate_loaded_on(&mut self, execution: crate::stems::SeparationDevice) {
        let Some(id) = self.deck.track else { return };
        let Some(m) = self.stem.model_for(Some(id), execution) else {
            self.toast_err("Checking this computer for stem separation; try again in a moment.");
            return;
        };
        let Some(key) = self.stem_cache_key(id, m.id) else { return };
        if !self.load_cached_stems(&key, Some(execution)) {
            self.start_stem_separation(key, execution);
        }
    }

    fn start_stem_separation(&mut self, key: StemCacheKey, execution: crate::stems::SeparationDevice) {
        if self.stem.separating.is_some() {
            if self.stem.active_key.as_ref() != Some(&key) || self.stem.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                self.stem.pending = Some((key, execution));
            }
            return;
        }
        let id = key.track;
        let Some(m) = crate::stems::spec(key.model) else { return };
        if !crate::stems::model_ready(m) {
            self.stem.pending = Some((key, execution));
            self.stems_download_for(m);
            return;
        }
        let Some(t) = self.track(id).cloned() else { return };
        self.stem.separating = Some((id, 0.0));
        self.stem.active_key = Some(key.clone());
        self.stem.execution = execution;
        self.stem.errors.remove(&None);
        self.stem.errors.remove(&Some(id));
        let tx = self.stem.tx.clone();
        let ctx = self.ctx.clone();
        // Each separation has its own stop flag, so stopping one (its track left the deck)
        // cannot stop the next.
        let cancel: std::sync::Arc<std::sync::atomic::AtomicBool> = Default::default();
        self.stem.cancel = cancel.clone();
        let audio = self.deck.audio.clone();
        let preview_hw = self.stem.hw.as_ref().filter(|hw|
            self.prefs.stems_progressive_preview && crate::stems::Live::supported(hw, m, execution)
        ).cloned();
        let cache_limit = self.stem.cache_limit.clone();
        let worker = std::thread::Builder::new().name("rb-stems".into()).spawn(move || {
            let run = || -> anyhow::Result<std::sync::Arc<crate::stems::Stems>> {
                let cancelled = || cancel.load(std::sync::atomic::Ordering::Relaxed);
                let dec = match audio {
                    Some(audio) => audio,
                    None => Arc::new(crate::audio::decode_until(&t.path, cancelled)?),
                };
                let preview = preview_hw.and_then(|hw| crate::stems::Live::for_job(&hw, m, execution, dec.frames(), dec.sr));
                if let Some(preview) = &preview {
                    if cancelled() { anyhow::bail!("stopped"); }
                    let _ = tx.send(StemMsg::Preview(key.clone(), preview.clone()));
                }
                let mut last_progress = 0.0;
                let st = crate::stems::separate_with_live(m, &dec.samples, dec.sr, &mut |p| {
                    if cancelled() { return false; }
                    if p > last_progress {
                        last_progress = p;
                        let _ = tx.send(StemMsg::Progress(id, p));
                        ctx.request_repaint();
                    }
                    true
                }, execution, preview.as_deref())?;
                if cancelled() { anyhow::bail!("stopped"); }
                let saved = match crate::stems::save(&key.path, &st) {
                    Ok(()) => {
                        crate::stems::trim_cache(&key.path, || *cache_limit.lock());
                        Ok(())
                    }
                    Err(error) => Err(format!(
                        "Could not cache {} parts for “{}”. The deck can play them until the track is unloaded. Check free space and write access to {} ({error:#}).",
                        m.name, t.display_title(), key.path.display()
                    )),
                };
                let _ = tx.send(StemMsg::CacheWrite(key, saved));
                Ok(std::sync::Arc::new(st))
            };
            let _ = tx.send(StemMsg::Done(id, run()));
            ctx.request_repaint();
        });
        if let Err(e) = worker {
            self.stem.separating = None;
            self.stem.active_key = None;
            self.stem.errors.insert(Some(id), StemError { retry_model: None, detail: format!("Could not start stem separation: {e}") });
        }
    }

    fn detach_stem_preview(&mut self) {
        if let Some(preview) = self.stem.preview.take() {
            for deck in &self.player.decks {
                let mut player = deck.lock();
                if player.stem_preview.as_ref().is_some_and(|p| Arc::ptr_eq(p, &preview)) {
                    player.stem_preview = None;
                }
            }
        }
    }

    pub fn stop_separation(&mut self) {
        self.stem.pending = None;
        for request in self.stem.loading.values_mut() { *request = None; }
        self.stem.download_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.stem.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.detach_stem_preview();
    }

    /// Mute or unmute one part (source index: 0 drums, 1 bass, 2 other, 3 vocals).
    pub fn toggle_stem(&mut self, src: usize) {
        let have = self.player.state.lock().has_playable_stems();
        if !have {
            self.separate_loaded();
            return;
        }
        let mut st = self.player.state.lock();
        st.stem_on[src] = !st.stem_on[src];
    }

    pub fn poll_stems(&mut self) {
        while let Ok(m) = self.stem.rx.try_recv() {
            match m {
                StemMsg::Hardware(hw, model) => {
                    // Preferences can select a model while device detection is still running.
                    let model = self.stem.model.clone().unwrap_or(model);
                    self.stem.model_ready = crate::stems::spec(&model).map(crate::stems::model_ready).unwrap_or(false);
                    if self.prefs.stem_model.is_none() {
                        self.prefs.stem_model = Some(model.clone());
                        self.save_prefs();
                    }
                    self.stem.model = Some(model);
                    self.stem.hw = Some(hw);
                    if !self.stem.model_ready && self.prefs.stems_auto_download {
                        self.stems_download();
                    }
                    for n in 0..4 {
                        if let Some(id) = self.deck_of(n).track {
                            self.with_deck(n, |app| app.stems_for_loaded(id));
                        }
                    }
                }
                StemMsg::Download(d, t) => self.stem.downloading = Some((d, t)),
                StemMsg::DownloadDone(r) => {
                    self.stem.downloading = None;
                    match r {
                        Ok(()) => {
                            self.stem.model_ready = self.stem.model.as_deref().and_then(crate::stems::spec).is_some_and(crate::stems::model_ready);
                            self.resume_pending_stems();
                        }
                        Err(e) if e == "stopped" => {
                            self.stem.pending = None;
                        }
                        Err(e) => {
                            self.stem.errors.insert(None, StemError { retry_model: None, detail: format!("Model download failed: {e}") });
                            self.stem.pending = None;
                        }
                    }
                }
                StemMsg::CacheWrite(key, result) => {
                    if let Err(warning) = &result { self.toast_err(warning.as_str()); }
                    self.stem.record_cache_write(key, result);
                }
                StemMsg::Preview(key, preview) => {
                    if self.stem.active_key.as_ref() == Some(&key) && !self.stem.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        self.stem.preview = Some(preview.clone());
                        for n in 0..4 {
                            if self.deck_of(n).stem_cache_key.as_ref() == Some(&key) {
                                self.player.decks[n as usize].lock().stem_preview = Some(preview.clone());
                            }
                        }
                    }
                }
                StemMsg::Progress(id, p) => {
                    // (Not for a separation that was stopped because its track left the deck.)
                    if self.stem.separating.map(|s| s.0) == Some(id) {
                        self.stem.separating = Some((id, p));
                    }
                }
                StemMsg::Cached(key, result) => {
                    let request = self.stem.loading.remove(&key).flatten();
                    let Some(deck) = (0..4).find(|&n| self.deck_of(n).stem_cache_key.as_ref() == Some(&key)) else { continue };
                    match (result, request) {
                        (Ok(st), _) => self.apply_stems(&key, Ok(st)),
                        (Err(_), Some(execution)) => {
                            self.with_deck(deck, |app| app.start_stem_separation(key, execution));
                        }
                        (Err(e), None) => {
                            self.stem.errors.insert(Some(key.track), StemError { retry_model: None,
                                detail: format!("Could not load saved stems from {e:#}. Choose Separate to rebuild them.") });
                        }
                    }
                }
                StemMsg::Done(id, result) => {
                    self.stem.separating = None;
                    if let Some(key) = self.stem.active_key.take() {
                        debug_assert_eq!(id, key.track);
                        if !self.stem.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                            self.apply_stems(&key, result);
                        }
                    }
                    self.detach_stem_preview();
                    self.resume_pending_stems();
                }
            }
        }
    }

    fn apply_stems(&mut self, key: &StemCacheKey, result: anyhow::Result<Arc<crate::stems::Stems>>) {
        let id = key.track;
        if !(0..4).any(|n| self.deck_of(n).stem_cache_key.as_ref() == Some(key)) { return; }
        match result {
            Ok(st) => {
                self.stem.errors.remove(&Some(id));
                // Separated stems give vocal detection for free.
                if self.db.vocal(id).is_none() && st.len > 0 {
                    let (l, r) = (&st.data[6 * st.len..7 * st.len], &st.data[7 * st.len..8 * st.len]);
                    let mono: Vec<f32> = l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect();
                    // The mix is the sum of the four stems.
                    let mix: Vec<f32> = (0..st.len).map(|i| (0..8).map(|c| st.data[c * st.len + i]).sum::<f32>() * 0.5).collect();
                    let levels = crate::stems::vocal_levels(&mono, &mix);
                    let _ = self.db.save_vocal(id, &levels);
                    let vocal = Arc::new((crate::stems::VOCAL_HOP as f64 / 44100.0, levels));
                    for n in 0..4 {
                        let deck = self.deck_of_mut(n);
                        if deck.track == Some(id) && deck.vocal.is_none() { deck.vocal = Some(vocal.clone()); }
                    }
                }
                for n in 0..4 {
                    if self.deck_of(n).stem_cache_key.as_ref() == Some(key) { self.player.decks[n as usize].lock().stems = Some(st.clone()); }
                }
            }
            Err(e) => { self.stem.errors.insert(Some(id), StemError::separation(Some(key.model), e)); }
        }
    }

    pub fn can_review_library_sync(&self) -> bool {
        self.rb_job.is_none() && self.dialogs.ext.restore_ask.is_none()
            && !self.dialogs.ext.backup_job.as_ref().is_some_and(|job| job.restore)
    }

    pub fn review_rekordbox_sync(&mut self) {
        if self.can_review_library_sync() && self.dialogs.library_sync_review.is_none() {
            self.dialogs.library_sync_review = Some(crate::ui_sync::Review::new(&self.db));
        }
    }

    /// Copy rekordbox's library into Recordbox's data folder and read the copy, off the UI thread.
    /// rekordbox's own files are only read (byte copy), never opened by a database or written.
    pub fn start_rekordbox_sync(&mut self) {
        if crate::prefs_bridge::import_protected(self) {
            return;
        }
        if !self.can_review_library_sync() || self.dialogs.library_sync_review.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        let dest = crate::db::data_dir().join("rekordbox-copy");
        match std::thread::Builder::new().name("rb-library-sync".into()).spawn(move || {
            let _ = tx.send(crate::rbimport::load_installed(&dest));
            ctx.request_repaint();
        }) {
            Ok(_) => self.rb_job = Some(rx),
            Err(e) => self.toast_err(format!("Could not start desktop library sync: {e}")),
        }
    }

    pub fn poll_rekordbox_sync(&mut self) {
        let Some(rx) = &self.rb_job else { return };
        let r = match rx.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.rb_job = None;
                self.toast_err("Desktop library sync stopped before the source snapshot could be imported.");
                return;
            }
        };
        self.rb_job = None;
        match r {
            Ok(Some(lib)) => self.apply_rekordbox(lib),
            Ok(None) => self.toast_err("The rekordbox source database could not be found."),
            Err(e) => self.toast_err(format!("Could not import the desktop rekordbox library: {e:#}")),
        }
    }

    /// Merge a rekordbox library read from a snapshot. New tracks are added; a track that
    /// rekordbox changed since the last sync gets rekordbox's metadata, grid, cues and My Tags;
    /// one it did not change keeps any edits made in Recordbox. The playlist tree is mirrored
    /// into the "rekordbox" folder, which is rebuilt on every sync.
    pub fn apply_rekordbox(&mut self, lib: crate::rbimport::RbLibrary) {
        let result = match crate::rbimport::merge::apply(&self.db, &lib) {
            Ok(summary) => summary,
            Err(e) => {
                self.toast_err(format!("Could not sync the Recordbox collection: {e:#}"));
                return;
            }
        };
        let (added, updated, first) = (result.added, result.updated, result.first);
        self.reload_all();
        // The loaded track may have new cues or a new grid now.
        if let Some(t) = self.deck.track {
            self.deck.cues = self.db.cues(t).unwrap_or_default();
            if let Some(g) = self.db.grid(t) {
                self.set_deck_grid(g, false);
            }
            self.refresh_cue_counts(t);
        }
        let mut msg = if first {
            format!("Loaded your rekordbox library: {added} tracks, with cues, grids, My Tags and playlists. rekordbox's own files were not changed.")
        } else if added + updated > 0 {
            format!("Synced from rekordbox: {added} new, {updated} updated.")
        } else {
            "Sync complete. No new or updated tracks.".to_string()
        };
        if lib.not_local > 0 {
            msg.push_str(&format!(" {} track(s) without an accessible local file were left out.", lib.not_local));
        }
        if first && lib.smart_lists > 0 {
            msg.push_str(&format!(" {} intelligent playlist(s) use a condition recordbox doesn't have and were not copied.", lib.smart_lists));
        }
        self.toast(msg);
        if added > 0 && self.prefs.auto_analysis {
            // Waveforms and loudness are Recordbox's own; grids and keys from rekordbox are kept.
            let todo: Vec<i64> = self.tracks.iter().filter(|t| t.analyzed == 0 && !t.missing).map(|t| t.id).collect();
            self.analyze(&todo, false);
        }
    }

    pub fn import_xml(&mut self, path: &std::path::Path) {
        // Bounded size: a 100,000-track collection is a few hundred MB.
        if std::fs::metadata(path).map(|m| m.len() > 1 << 30).unwrap_or(false) {
            return self.toast_err("This file is larger than 1 GB, too large to be a collection file.");
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return self.toast_err(format!("Could not read the file: {e}")),
        };
        let lib = match crate::importer::parse_rekordbox_xml(&text) {
            Ok(l) => l,
            Err(e) => return self.toast_err(format!("This is not a readable collection file: {e}")),
        };
        let mut idmap: HashMap<String, i64> = HashMap::new();
        let mut added = 0;
        // All or nothing: one transaction, rolled back on the first database error.
        let _ = self.db.conn.execute_batch("BEGIN");
        let mut failed: Option<String> = None;
        let mut note = |r: anyhow::Result<()>| {
            if let Err(e) = r {
                failed.get_or_insert(e.to_string());
            }
        };
        for xt in lib.tracks {
            let id = match self.db.track_by_path(&xt.track.path) {
                Some(id) => id,
                None => {
                    let mut t = xt.track.clone();
                    let real = crate::importer::read_track(std::path::Path::new(&t.path));
                    if real.sample_rate > 0 {
                        t.sample_rate = real.sample_rate;
                        t.bit_depth = real.bit_depth;
                        t.artwork = real.artwork;
                        if t.duration <= 0.0 {
                            t.duration = real.duration;
                        }
                    }
                    if t.date_added.is_empty() {
                        t.date_added = real.date_added;
                    }
                    match self.db.insert_track(&t) {
                        Ok(id) => {
                            added += 1;
                            id
                        }
                        Err(e) => {
                            note(Err(e));
                            continue;
                        }
                    }
                }
            };
            if let Some(g) = &xt.grid {
                note(self.db.save_grid(id, g));
            }
            if !xt.cues.is_empty() {
                note(self.db.replace_cues(id, &xt.cues));
            }
            note(self.db.protect_imported_track(id));
            // What the imported TEMPO/POSITION_MARK elements became, to tell later whether they
            // were edited.
            let mut extra = xt.extra.clone();
            extra.grid_fp = crate::importer::grid_fingerprint(&self.db.grid(id));
            extra.cues_fp = crate::importer::cues_fingerprint(&self.db.cues(id).unwrap_or_default());
            if xt.grid.is_none() {
                extra.tempo.clear();
            }
            if xt.cues.is_empty() {
                extra.marks.clear();
            }
            note(self.db.set_xml_extra(id, &extra));
            idmap.insert(xt.xml_id.clone(), id);
        }
        if let Some(e) = failed {
            let _ = self.db.conn.execute_batch("ROLLBACK");
            self.reload_all();
            return self.toast_err(format!("The collection could not be imported ({e}). Nothing was changed."));
        }
        fn add_nodes(app: &mut App, parent: i64, nodes: &[crate::importer::XmlNode], idmap: &HashMap<String, i64>, missing: &mut usize) {
            for n in nodes {
                if n.folder {
                    let id = app.db.create_node(parent, &n.name, NodeKind::Folder, None).unwrap_or(0);
                    add_nodes(app, id, &n.children, idmap, missing);
                } else {
                    let id = app.db.create_node(parent, &n.name, NodeKind::Playlist, None).unwrap_or(0);
                    // KeyType="1" playlists refer to tracks by file location.
                    let tr: Vec<i64> = n.track_keys.iter().filter_map(|k| if n.by_location { app.db.track_by_path(k) } else { idmap.get(k).copied() }).collect();
                    *missing += n.track_keys.len() - tr.len();
                    let _ = app.db.set_playlist_tracks(id, &tr);
                }
            }
        }
        let mut missing_refs = 0usize;
        if !lib.playlists.is_empty() {
            let root = self.db.create_node(0, "Imported collection", NodeKind::Folder, None).unwrap_or(0);
            add_nodes(self, root, &lib.playlists, &idmap, &mut missing_refs);
        }
        if let Err(e) = self.db.conn.execute_batch("COMMIT") {
            let _ = self.db.conn.execute_batch("ROLLBACK");
            self.reload_all();
            return self.toast_err(format!("The collection could not be imported ({e}). Nothing was changed."));
        }
        self.reload_all();
        let mut notes = lib.warnings.clone();
        if missing_refs > 0 {
            notes.push(format!("{missing_refs} playlist entr{} referred to tracks not in the collection and were left out", if missing_refs == 1 { "y" } else { "ies" }));
        }
        let extra = if notes.is_empty() { String::new() } else { format!(" Note: {}.", notes.join("; ")) };
        self.toast(format!("Imported {added} new track(s) with their cues, grids and playlists. Original files were not changed.{extra}"));
        if self.prefs.auto_analysis {
            let todo: Vec<i64> = self.tracks.iter().filter(|t| t.analyzed == 0 && !t.missing).map(|t| t.id).collect();
            self.analyze(&todo, false);
        }
    }

    // ------------------------------------------------------------------ analysis
    /// Re-analyse with key detection on, whatever Preferences > Analysis says.
    pub fn analyze_forcing_key(&mut self, ids: &[i64]) -> Vec<i64> {
        let mut s = self.analysis_settings();
        s.do_key = true;
        self.analyze_with(ids, true, s)
    }

    pub fn analyze(&mut self, ids: &[i64], force: bool) {
        let s = self.analysis_settings();
        self.analyze_with(ids, force, s);
    }

    /// Queue analysis jobs; returns the tracks actually submitted.
    fn analyze_with(&mut self, ids: &[i64], force: bool, s: crate::analysis::AnalysisSettings) -> Vec<i64> {
        let mut submitted = vec![];
        let mut n = 0;
        for id in ids {
            let Some(t) = self.track_mut(*id) else { continue };
            // Analysis Lock: a locked track is never re-analysed (its grid, key, waveform and
            // gain stay as they are), even by "Re-analyze".
            if t.missing || (t.analyzed == 3) || (!force && t.analyzed == 1) || (t.grid_locked && t.analyzed == 1) {
                continue;
            }
            t.analyzed = 3;
            let path = t.path.clone();
            n += 1;
            submitted.push(*id);
            self.pool.submit(AnalysisJob { track_id: *id, path, settings: s.clone() });
        }
        self.analysis_total += n;
        self.list_dirty = true;
        submitted
    }

    pub fn poll_analysis(&mut self) {
        while let Ok((id, r)) = self.pool.rx.try_recv() {
            self.analysis_done += 1;
            self.apply_analysis(id, r);
        }
        if self.analysis_done == self.analysis_total {
            self.analysis_total = 0;
            self.analysis_done = 0;
        }
    }

    pub(crate) fn suppress_pending_analysis_hot_slot(&mut self, track: i64, slot: i32) {
        if (0..8).contains(&slot) && self.track(track).is_some_and(|t| t.analyzed == 3) {
            self.cleared_analysis_hot_slots.entry(track).or_default().insert(slot);
        }
    }

    pub fn apply_analysis(&mut self, id: i64, r: Result<crate::analysis::AnalysisResult, String>) {
        let cleared_hot_slots = self.cleared_analysis_hot_slots.remove(&id).unwrap_or_default();
        let existing_grid = self.db.grid(id).filter(|g| !g.is_empty());
        // Earlier analysis means a stored waveform (a grid alone can come from an import).
        let had_analysis = self.db.wave(id).is_some();
        let keep_tag_key = self.prefs.keep_tag_key;
        let key_requested = self.key_requested.remove(&id);
        let requested_grid = self.grid_requested.remove(&id);
        let mut locked_peak: Option<f32> = None;
        let Some(t) = self.track_mut(id) else { return };
        match r {
            Err(e) => {
                t.analyzed = if e == "stopped" { 0 } else { 2 };
            }
            Ok(res) if t.grid_locked && had_analysis => {
                // Analysis Lock: a locked track keeps its grid, key, waveform and gain, whatever
                // the job's queued state (the lock is checked here, when results arrive). Its
                // measured peak is still recorded (for the optional peak cap).
                locked_peak = Some(res.peak);
                t.analyzed = 1;
            }
            Ok(res) => {
                // The job's queued state can't tell this (analyze() marks every job queued).
                let first_analysis = !had_analysis;
                t.analyzed = 1;
                t.duration = res.duration;
                t.sample_rate = res.sample_rate;
                t.raw_samples = res.raw_samples;
                t.gain_db = res.gain_db;
                let measured_peak = res.peak;
                let keep_grid = keep_saved_grid(t.grid_locked, &existing_grid, requested_grid.as_ref(), res.grid.is_some());
                // A key the user set (or one from an imported library) is only replaced when the
                // track had no key, or on its first analysis when the key came from tags.
                if res.key >= 0 && (key_requested || (!t.key_manual && (t.key < 0 || (first_analysis && !keep_grid && !keep_tag_key)))) {
                    // Analyze Key asked for it explicitly: replaces even a chosen key.
                    t.set_key(res.key);
                    if key_requested {
                        t.key_manual = false;
                    }
                }
                let mut save_grid = None;
                if !keep_grid {
                    if let Some(g) = res.grid {
                        t.bpm = g.first_bpm();
                        save_grid = Some(g);
                    }
                } else if let Some(g) = &existing_grid {
                    if g.first_bpm() > 0.0 {
                        t.bpm = g.first_bpm();
                    }
                }
                if let Ok(m) = std::fs::metadata(&t.path) {
                    let _ = self.db.set_media_stamp(id, media_stamp(&m));
                }
                if measured_peak > 0.0 {
                    let _ = self.db.set_peak(id, measured_peak);
                }
                let preview: Vec<u8> = res.wave.columns(400).iter().flat_map(|c| c.iter().copied()).collect();
                let _ = self.db.save_wave(id, &res.wave, &preview);
                self.previews.insert(id, preview);
                if let Some(v) = &res.vocal {
                    let _ = self.db.save_vocal(id, v);
                    if self.deck.track == Some(id) && self.deck.vocal.is_none() {
                        self.deck.vocal = Some(std::sync::Arc::new((crate::vocal_rb::hop_seconds(), v.clone())));
                    }
                }
                if let Some(g) = &save_grid {
                    let _ = self.db.save_grid(id, g);
                }
                if let Some(g) = save_grid.as_ref().filter(|_| first_analysis) {
                    let mut added_cues = self.apply_cue_tendency(id, g, &cleared_hot_slots);
                    // Rekordbox's default Hot Cue A ("1.1Bars") applies to a track without cues.
                    if self.prefs.auto_first_downbeat_cue && !cleared_hot_slots.contains(&0)
                        && !added_cues && self.db.cues(id).is_ok_and(|cues| cues.is_empty())
                    {
                        let dur = self.track(id).map(|t| t.duration).unwrap_or(0.0).max(1.0);
                        if let Some(b) = g.beats(dur).into_iter().find(|b| b.beat == 1) {
                            let c = Cue { id: 0, track_id: id, kind: CueKind::Hot, slot: 0, start: b.time, end: None, color: 0, comment: "1.1Bars".into(), active_loop: false };
                            added_cues = self.db.save_cue(&c).is_ok();
                        }
                    }
                    if self.prefs.memory_cue_first_beat {
                        added_cues |= crate::prefs_ext::memory_cue_at_first_beat(self, id, g);
                    }
                    if added_cues {
                        if let Ok(cues) = self.db.cues(id) {
                            self.set_track_cues(id, cues);
                        }
                    }
                }
                if self.deck.track == Some(id) {
                    self.deck.x.overview_cache.clear();
                    self.deck.wave = Some(res.wave);
                    if self.deck.audio.is_none() {
                        self.deck.duration = res.duration;
                    }
                    if let Some(g) = self.db.grid(id) {
                        self.set_deck_grid(g, false);
                    }
                }
            }
        }
        if let Some(pk) = locked_peak.filter(|p| *p > 0.0) {
            let _ = self.db.set_peak(id, pk);
        }
        // Analysis results are not user edits: keep them off the undo stack.
        self.save_track_quiet(id);
    }

    /// Explicitly request a new grid when the user selects BPM / Grid and Analyze Track.
    pub fn reanalyze(&mut self, ids: &[i64]) {
        let settings = self.analysis_settings();
        let with_grid = settings.do_grid;
        let submitted = self.analyze_with(ids, true, settings);
        if with_grid {
            for id in submitted {
                self.grid_requested.insert(id, self.db.grid(id).filter(|g| !g.is_empty()));
            }
        }
    }

    // ------------------------------------------------------------------ player
    pub fn pos(&self) -> f64 {
        let s = self.player.state.lock();
        match &s.audio {
            Some(a) => s.pos / a.sr as f64,
            None => 0.0,
        }
    }
    /// egui subtracts predicted_dt from nonzero repaint delays. Account for it
    /// so a 30 Hz preference does not request another frame after just 16 ms.
    pub fn repaint_playback(&self, ctx: &egui::Context) {
        let fps = if self.prefs.playback_fps == 0 { crate::prefs_bridge::draw_fps(self.prefs.rb.draw_rate) as u8 } else { self.prefs.playback_fps };
        match fps {
            fps @ (15 | 30 | 40) => {
                let delay = 1.0 / f32::from(fps) + ctx.input(|i| i.predicted_dt);
                ctx.request_repaint_after_secs(delay);
            }
            _ => ctx.request_repaint(),
        }
    }

    pub fn update_transport(&mut self, ctx: &egui::Context, decks_visible: bool) {
        let running = self.player.transport_running();
        #[cfg(target_os = "linux")]
        {
            // Performance mode, including an initialized sampler's independent
            // sequencer/quantization clock, always keeps its low-latency output.
            let editor_playing = {
                let edit = crate::edit_audio::EDIT.lock();
                edit.main.playing || edit.any_pal_playing()
            };
            let keep_ready = self.performance() || crate::sampler::SAMPLER.lock().is_some() || editor_playing || self.rec.recording();
            match self.player.service_output(keep_ready, running, self.prefs.audio_device.as_deref()) {
                Ok(Some(delay)) => ctx.request_repaint_after_secs(delay.as_secs_f32() + ctx.input(|i| i.predicted_dt)),
                Err(error) => {
                    self.player.error = Some(error.to_string());
                    self.player.lost.store(true, std::sync::atomic::Ordering::Relaxed);
                    for deck in &self.player.decks {
                        let mut s = deck.lock();
                        s.playing = false;
                        s.search = 0.0;
                    }
                    ctx.request_repaint();
                    return;
                }
                Ok(None) => {}
            }
        }
        if running {
            if decks_visible {
                self.repaint_playback(ctx);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
    }

    pub fn playing(&self) -> bool {
        self.player.state.lock().playing
    }
    pub fn seek(&mut self, t: f64) {
        let t = t.clamp(0.0, self.deck.duration.max(0.0));
        let mut s = self.player.state.lock();
        if let Some(a) = &s.audio {
            s.pos = t * a.sr as f64;
        }
    }
    pub fn set_playing(&mut self, p: bool) {
        self.player.state.lock().playing = p && !self.deck_options.disable_play && self.deck.audio.is_some();
    }

    /// Load a track into the load-target deck (`load_target`; deck A outside 2 Players).
    pub fn load_track(&mut self, id: i64) {
        if self.deck_options.disable_load {
            return self.toast_err("Deck loading is disabled for this session (--disable-load).");
        }
        if crate::ui_edit::intercept_load(self, id) { return; }
        self.load_track_to(self.load_target, id);
    }

    /// Load a track into the active deck.
    pub fn load_track_here(&mut self, id: i64) {
        if self.deck_options.disable_load {
            return self.toast_err("Deck loading is disabled for this session (--disable-load).");
        }
        // Eject/Load Lock (Performance): not while the deck plays, unless it stands at the end.
        if self.performance() && self.prefs.deck_load_lock && self.playing() && self.pos() < self.deck.duration - 0.05 {
            self.toast_err("Load Lock. Not available during playback.");
            return;
        }
        let Some(t) = self.track(id).cloned() else { return };
        if t.missing {
            self.toast_err(format!("The file for \"{}\" cannot be found.", t.display_title()));
            return;
        }
        let (old_pos, old_gain) = {
            let mut s = self.player.state.lock();
            let keep = (s.pos, s.gain);
            s.playing = false;
            s.audio = None;
            s.pos = 0.0;
            s.loop_range = None;
            s.metronome = None;
            s.search = 0.0;
            let mut g = if t.gain_db.is_finite() && crate::prefs_bridge::auto_gain_on(self) { 10f32.powf(t.gain_db / 20.0).min(4.0) } else { 1.0 };
            // rekordbox's optional peak cap (traced): when gain × peak reaches 1, the gain becomes
            // 0.9999998807907104 / peak.
            if self.prefs.auto_gain_peak_cap {
                if let Some(pk) = self.db.peak(t.id).filter(|p| *p > 0.0) {
                    if g * pk >= 1.0 {
                        g = 0.999_999_88 / pk;
                    }
                }
            }
            s.gain = g;
            keep
        };
        self.deck.cancel_work.store(true, std::sync::atomic::Ordering::Relaxed);
        let ctrl = self.deck.ctrl_tab;
        let panel = self.deck.panel_tab;
        let metronome = self.deck.metronome;
        let x = self.deck.x.carried();
        let old = std::mem::replace(&mut self.deck, Deck { ctrl_tab: ctrl, panel_tab: panel, metronome, x, ..Deck::default() });
        // Keep the last successfully loaded deck (not a deck that is itself still loading).
        if old.audio.is_some() {
            self.deck_backup = Some((old, old_pos, old_gain));
        }
        self.deck.track = Some(id);
        self.deck.duration = t.duration;
        self.deck.wave = self.db.wave(id);
        self.deck.rb_wave = self.rb_waveforms(&t.path);
        self.deck.vocal = self.rb_vocals(&t.path).or_else(|| self.db.vocal(id).map(|v| std::sync::Arc::new((crate::stems::VOCAL_HOP as f64 / 44100.0, v))));
        self.deck.cues = self.db.cues(id).unwrap_or_default();
        let g = self.db.grid(id).unwrap_or_default();
        self.set_deck_grid(g, false);
        self.deck.bpm_text = format!("{:.2}", self.deck.grid.first_bpm());
        let first_mem = self.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).map(|c| c.start).fold(f64::NAN, f64::min);
        self.deck.cue_point = if first_mem.is_nan() { self.deck.beats.first().map(|b| b.time).unwrap_or(0.0) } else { first_mem };
        crate::prefs_bridge::on_load(self);
        self.deck.loading = Some(spawn_load(id, t.path.clone(), self.ctx.clone(), self.deck.cancel_work.clone()));
        {
            let mut st = self.player.state.lock();
            st.stems = None;
            st.stem_preview = None;
            // Preferences > STEMS: "Reset the MUTE/SOLO state while loading".
            if self.prefs.stem_reset_on_load {
                st.stem_on = [true; 4];
            }
        }
        self.cancel_orphaned_stems();
        if t.analyzed == 0 {
            self.analyze(&[id], false);
        }
    }

    pub fn poll_load(&mut self) {
        let Some(rx) = &self.deck.loading else { return };
        if let Ok((id, r)) = rx.try_recv() {
            self.deck.loading = None;
            if self.deck.track != Some(id) {
                return;
            }
            match r {
                Ok(a) => {
                    self.deck_backup = None;
                    if a.damaged > 0 {
                        self.toast_err(format!("This file is damaged: {} part(s) could not be decoded and are silent or skipped.", a.damaged));
                    } else if a.truncated {
                        self.toast_err("This file is incomplete: it ends before the length its header declares (cut short when copied or downloaded?).");
                    }
                    self.deck.duration = a.duration();
                    crate::prefs_bridge::after_load(self, &a);
                    let cp = self.deck.cue_point;
                    let mut s = self.player.state.lock();
                    s.pos = cp * a.sr as f64;
                    s.audio = Some(a.clone());
                    drop(s);
                    self.deck.audio = Some(a.clone());
                    let g = self.deck.grid.clone();
                    self.set_deck_grid(g, false);
                    self.deck.phrases = None;
                    self.stems_for_loaded(id);
                    self.start_deck_phrases(a.clone());
                    self.start_deck_vocal(a.clone());
                    self.instant_double();
                }
                Err(e) => {
                    let backup = self.deck_backup.take();
                    self.unload_track();
                    if let Some((old, pos, gain)) = backup {
                        let mut s = self.player.state.lock();
                        s.audio = old.audio.clone();
                        s.pos = pos;
                        s.gain = gain;
                        drop(s);
                        self.deck = old;
                        self.deck.cancel_work = Default::default();
                        if let (Some(id), Some(audio)) = (self.deck.track, self.deck.audio.clone()) {
                            self.stems_for_loaded(id);
                            if self.deck.phrases.is_none() { self.start_deck_phrases(audio.clone()); }
                            if self.deck.vocal.is_none() { self.start_deck_vocal(audio); }
                        }
                    }
                    self.toast_err(format!("This file cannot be played: {e}"));
                }
            }
        }
    }

    pub fn set_deck_grid(&mut self, g: BeatGrid, save: bool) {
        let dur = if self.deck.duration > 0.0 { self.deck.duration } else { self.deck.track.and_then(|id| self.track(id)).map(|t| t.duration).unwrap_or(0.0) };
        let bpm = g.first_bpm();
        let tempo_changed = bpm != self.deck.grid.first_bpm();
        self.deck.beats = g.beats(dur);
        self.deck.bpm_text = format!("{bpm:.2}");
        self.deck.grid = g;
        if save {
            if let Some(id) = self.deck.track {
                if let Err(err) = self.db.save_grid(id, &self.deck.grid) {
                    self.toast_err(format!("Could not save the beat grid: {err}"));
                } else if tempo_changed {
                    // Native tracks can have a summary BPM distinct from their first beat.
                    if let Some(t) = self.track_mut(id) {
                        t.bpm = bpm;
                    }
                    self.save_track(id);
                }
            }
        }
        self.update_metronome();
    }

    pub fn update_metronome(&mut self) {
        let beats = if self.deck.metronome { Some(Arc::new(self.deck.beats.iter().map(|b| b.time).collect::<Vec<f64>>())) } else { None };
        let mut s = self.player.state.lock();
        s.metronome = beats;
        s.metronome_vol = crate::audio::metronome_level(self.prefs.metronome_vol);
    }

    pub fn quantized(&self, t: f64) -> f64 {
        if self.prefs.quantize && !self.deck.beats.is_empty() {
            quantize_to(&self.deck.beats, t, self.prefs.quantize_div)
        } else {
            t
        }
    }

    pub fn current_bpm(&self) -> f64 {
        let p = self.pos();
        let i = self.deck.beats.partition_point(|b| b.time <= p);
        if i > 0 {
            self.deck.beats[i - 1].bpm
        } else {
            self.deck.beats.first().map(|b| b.bpm).or(self.deck.track.and_then(|id| self.track(id)).map(|t| t.bpm)).unwrap_or(0.0)
        }
    }

    pub fn beat_len(&self) -> f64 {
        let b = self.current_bpm();
        if b > 0.0 {
            60.0 / b
        } else {
            0.5
        }
    }

    pub fn cue_down(&mut self) {
        if self.deck.audio.is_none() {
            return;
        }
        // As rekordbox's `eventCueOn`: CUE leaves loop adjust and ends a running loop first.
        self.deck.x.loop_adjust = 0;
        if self.playing() {
            self.set_playing(false);
            let c = self.deck.cue_point;
            self.seek(c);
            let mut s = self.player.state.lock();
            s.loop_range = None;
            s.dk.warp = None;
            s.dk.arm_loop = None;
        } else {
            let p = self.pos();
            if (p - self.deck.cue_point).abs() < 0.01 {
                if self.deck_options.disable_play { return; }
                self.deck.cue_preview = true;
                self.set_playing(true);
            } else if p >= self.deck.duration - 0.01 {
                // (At the end of the track CUE does nothing.)
            } else if self.before_start() {
            } else {
                let q = self.quantized(p);
                self.deck.cue_point = q;
                self.seek(q);
            }
        }
    }
    pub fn cue_up(&mut self) {
        if self.deck.cue_preview {
            self.deck.cue_preview = false;
            self.set_playing(false);
            let c = self.deck.cue_point;
            self.seek(c);
        }
    }
    pub fn play_pause(&mut self) {
        if self.deck_options.disable_play {
            return self.toast_err("Playback is disabled for this session (--disable-play).");
        }
        if self.deck.audio.is_none() {
            return;
        }
        self.deck.x.loop_adjust = 0;
        if self.deck.cue_preview {
            self.deck.cue_preview = false;
            return;
        }
        let p = !self.playing();
        self.set_playing(p);
    }

    pub fn hot_cue_down(&mut self, slot: i32) {
        if self.deck.track.is_none() || self.deck.audio.is_none() {
            return;
        }
        if let Some(c) = self.deck.cues.iter().find(|c| c.kind == CueKind::Hot && c.slot == slot).cloned() {
            {
                // SLIP on a playing deck: the hot cue sounds while the pad is held and the
                // track carries on underneath.
                let mut st = self.player.state.lock();
                if st.slip && st.playing && st.slip_pos.is_none() {
                    st.slip_pos = Some(st.pos);
                }
                st.slip_hold = st.slip && st.playing;
            }
            if c.end.is_some() || !self.hot_cue_quantized(c.start) {
                self.seek(c.start);
            }
            if let Some(e) = c.end.filter(|e| *e > c.start) {
                self.player.state.lock().loop_range = Some((c.start, e));
                self.deck.last_loop = Some((c.start, e));
            }
            crate::prefs_bridge::smart_cue(self, c.start);
            if !self.playing() {
                // Gate Cue: it plays while held; off, it plays on.
                if crate::prefs_bridge::gate_cue(self) {
                    self.deck.hot_preview = Some(c.start);
                }
                self.set_playing(true);
            }
        } else {
            let lr = self.player.state.lock().loop_range;
            let (start, end) = match lr {
                Some((a, b)) => (a, Some(b)),
                None => (self.quantized(self.pos()), None),
            };
            self.set_hot(slot, start, end);
        }
    }
    pub fn hot_cue_up(&mut self) {
        // SLIP: letting go of the pad returns to where the track would have been.
        self.player.state.lock().slip_hold = false;
        if let Some(start) = self.deck.hot_preview.take() {
            self.set_playing(false);
            self.player.state.lock().loop_range = None;
            self.seek(start);
        }
    }
    /// Remember the loaded track's cues before an edit (for Undo).
    pub fn snapshot_cues(&mut self) {
        if let Some(t) = self.deck.track {
            self.record(Edit::Cues { track: t, before: self.deck.cues.clone() });
        }
    }

    fn record(&mut self, e: Edit) {
        self.undo.push(e);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Remember a track's edited phrases before a PHRASE EDIT change (for Undo).
    pub fn record_phrases(&mut self, track: i64) {
        let before = self.stored_phrases(track);
        self.record(Edit::Phrases { track, before });
    }

    /// Work out the active deck's phrases again (after its phrase edits were removed).
    pub fn reanalyse_deck_phrases(&mut self) {
        self.deck.phrases = None;
        if let Some(a) = self.deck.audio.clone() {
            self.start_deck_phrases(a);
        }
    }

    /// Apply a history entry and capture its inverse. A failed cue write leaves the entry retryable.
    fn restore(&mut self, edit: &Edit) -> anyhow::Result<Option<Edit>> {
        Ok(match edit {
            Edit::Phrases { track, before } => {
                let previous = self.stored_phrases(*track);
                self.store_phrases(*track, before.clone());
                Some(Edit::Phrases { track: *track, before: previous })
            }
            Edit::Cues { track, before } => {
                let (previous, restored) = self.db.restore_cues(*track, before)?;
                for cue in previous.iter().filter(|cue| cue.kind == CueKind::Hot) {
                    if !restored.iter().any(|other| other.kind == CueKind::Hot && other.slot == cue.slot) {
                        self.suppress_pending_analysis_hot_slot(*track, cue.slot);
                    }
                }
                self.set_track_cues(*track, restored);
                Some(Edit::Cues { track: *track, before: previous })
            }
            Edit::Track { before } => {
                let previous = self.db.track(before.id);
                let _ = self.db.update_track(before);
                if let Some(t) = self.track_mut(before.id) {
                    t.clone_from(before);
                }
                self.list_dirty = true;
                previous.map(|before| Edit::Track { before })
            }
            Edit::Grid { track, before } => {
                let track = *track;
                let previous = Some(Edit::Grid { track, before: self.db.grid(track).unwrap_or_default() });
                // Analysis Lock also holds against Undo/Redo.
                if self.track(track).map(|t| t.grid_locked).unwrap_or(false) {
                    self.toast_err("This track's analysis is locked; unlock it to change its grid.");
                    return Ok(previous);
                }
                let _ = self.db.save_grid(track, before);
                for n in 0..4u8 {
                    if n != self.px.active && self.px.parked[n as usize].track == Some(track) {
                        let g = before.clone();
                        self.with_deck(n, |a| a.set_deck_grid(g, false));
                    }
                }
                if self.deck.track == Some(track) {
                    self.set_deck_grid(before.clone(), false);
                }
                previous
            }
        })
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn undo_edit(&mut self) {
        let Some(e) = self.undo.pop() else { return };
        match self.restore(&e) {
            Ok(Some(inverse)) => self.redo.push(inverse),
            Ok(None) => {}
            Err(error) => {
                self.undo.push(e);
                self.toast_err(format!("Could not undo cue edit: {error}"));
            }
        }
    }

    pub fn redo_edit(&mut self) {
        let Some(e) = self.redo.pop() else { return };
        match self.restore(&e) {
            Ok(Some(inverse)) => self.undo.push(inverse),
            Ok(None) => {}
            Err(error) => {
                self.redo.push(e);
                self.toast_err(format!("Could not redo cue edit: {error}"));
            }
        }
    }

    pub fn clear_hot_cue(&mut self, slot: i32) {
        self.edit_hot_cue(slot, None);
    }

    pub(crate) fn edit_hot_cue(&mut self, slot: i32, replacement: Option<&Cue>) {
        if !(0..8).contains(&slot) { return; }
        let Some(track) = self.deck.track else { return };
        let (before, after) = match self.db.edit_hot_cue(track, slot, replacement) {
            Ok(cues) => cues,
            Err(error) => return self.toast_err(format!("Could not change hot cue: {error}")),
        };
        if replacement.is_some() || after.len() != before.len() {
            self.record(Edit::Cues { track, before });
            if replacement.is_none() {
                self.suppress_pending_analysis_hot_slot(track, slot);
            }
        }
        self.set_track_cues(track, after);
        if let Some(cue) = replacement {
            crate::prefs_bridge::smart_cue(self, cue.start);
        }
    }

    pub fn delete_cue(&mut self, cue_id: i64) {
        let Some(cue) = self.deck.cues.iter().find(|c| c.id == cue_id) else { return };
        if cue.kind == CueKind::Hot {
            return self.clear_hot_cue(cue.slot);
        }
        if let Err(error) = self.db.delete_cue(cue_id) {
            return self.toast_err(format!("Could not remove cue: {error}"));
        }
        self.snapshot_cues();
        self.deck.cues.retain(|c| c.id != cue_id);
        if let Some(t) = self.deck.track { self.refresh_cue_counts(t); }
    }
    pub fn update_cue(&mut self, c: Cue) {
        self.snapshot_cues();
        let _ = self.db.save_cue(&c);
        if let Some(x) = self.deck.cues.iter_mut().find(|x| x.id == c.id) {
            *x = c;
        }
    }
    /// Finish dragging a cue marker: one Undo step for the whole drag, saved once.
    pub fn commit_cue_drag(&mut self, cancel: bool) {
        let Some((id, before, _)) = self.deck.cue_drag.take() else { return };
        let Some(tid) = self.deck.track else { return };
        let moved = self.deck.cues.iter().find(|c| c.id == id).cloned();
        let orig = before.iter().find(|c| c.id == id).cloned();
        match (moved, orig) {
            (Some(m), Some(o)) if !cancel && (m.start - o.start).abs() > 1e-6 => {
                self.record(Edit::Cues { track: tid, before });
                let _ = self.db.save_cue(&m);
                self.refresh_cue_counts(tid);
            }
            _ => self.deck.cues = before,
        }
    }
    pub fn refresh_cue_counts(&mut self, tid: i64) {
        self.set_track_cues(tid, self.deck.cues.clone());
    }

    fn set_track_cues(&mut self, tid: i64, cues: Vec<Cue>) {
        // A track has one set of cues, even when it is loaded on more than one deck.
        for deck in &mut self.px.parked {
            if deck.track == Some(tid) {
                deck.cues.clone_from(&cues);
            }
        }
        for (deck, _, _) in self.deck_backup.iter_mut().chain(self.px.parked_backup.iter_mut().flatten()) {
            if deck.track == Some(tid) {
                deck.cues.clone_from(&cues);
            }
        }
        let hp: Vec<(f64, i32, u8)> = cues.iter().filter(|c| c.kind == CueKind::Hot).map(|c| (c.start, c.slot, c.color)).collect();
        if hp.is_empty() {
            self.hot_positions.remove(&tid);
        } else {
            self.hot_positions.insert(tid, hp);
        }
        let h = cues.iter().filter(|c| c.kind == CueKind::Hot).count();
        let m = cues.iter().filter(|c| c.kind == CueKind::Memory).count();
        if h + m == 0 {
            self.cue_counts.remove(&tid);
        } else {
            self.cue_counts.insert(tid, (h, m));
        }
        if self.deck.track == Some(tid) {
            self.deck.cues = cues;
        }
    }

    pub fn memory_set(&mut self) {
        let Some(tid) = self.deck.track else { return };
        let lr = self.player.state.lock().loop_range;
        let (start, end) = match lr {
            Some((a, b)) => (a, Some(b)),
            None => (if self.playing() { self.quantized(self.pos()) } else { self.deck.cue_point }, None),
        };
        if self.deck.cues.iter().any(|c| c.kind == CueKind::Memory && (c.start - start).abs() < 0.001 && c.end == end) {
            return;
        }
        // One shared capacity of ten memory cues and loops per track (rekordbox-rea AUD-004:
        // the duplicate check comes first, then a new one is admitted only at a count of 0–9).
        if self.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).count() >= 10 {
            self.toast_err("This track already has 10 memory cues and loops, the most it can hold. Delete one to add another.");
            return;
        }
        let mut c = Cue { id: 0, track_id: tid, kind: CueKind::Memory, slot: -1, start, end, color: 0, comment: String::new(), active_loop: false };
        self.snapshot_cues();
        if let Ok(id) = self.db.save_cue(&c) {
            c.id = id;
            self.deck.cues.push(c);
            self.refresh_cue_counts(tid);
        }
    }
    pub fn memory_cues(&self) -> Vec<Cue> {
        let mut v: Vec<Cue> = self.deck.cues.iter().filter(|c| c.kind == CueKind::Memory).cloned().collect();
        v.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        v
    }
    pub fn memory_jump(&mut self, dir: i32) {
        if crate::prefs_bridge::memory_call_locked(self) {
            return;
        }
        let p = if self.playing() { self.pos() } else { self.deck.cue_point };
        let m = self.memory_cues();
        let target = if dir > 0 { m.iter().find(|c| c.start > p + 0.005) } else { m.iter().rev().find(|c| c.start < p - 0.005) };
        if let Some(c) = target.cloned() {
            self.call_memory(&c);
        }
    }
    pub fn call_memory(&mut self, c: &Cue) {
        if crate::prefs_bridge::memory_call_locked(self) {
            return;
        }
        self.deck.cue_point = c.start;
        self.seek(c.start);
        if let Some(e) = c.end.filter(|e| *e > c.start) {
            self.deck.last_loop = Some((c.start, e));
            if c.active_loop {
                self.player.state.lock().loop_range = Some((c.start, e));
            }
        }
    }
    pub fn memory_delete_current(&mut self) {
        let cp = self.deck.cue_point;
        if let Some(c) = self.deck.cues.iter().find(|c| c.kind == CueKind::Memory && (c.start - cp).abs() < 0.005).map(|c| c.id) {
            self.delete_cue(c);
        }
    }

    pub fn beat_jump(&mut self, beats: f64) {
        let p = self.pos();
        let t = grid_jump(&self.deck.beats, p, beats).unwrap_or(p + beats * self.beat_len());
        let t = t.clamp(0.0, self.deck.duration.max(0.0));
        self.seek(t);
        if !self.playing() {
            self.deck.cue_point = self.pos();
        }
    }

    pub fn auto_loop(&mut self, beats: f64) {
        if self.deck.audio.is_none() || self.perf_auto_loop(beats) {
            return;
        }
        let sr = self.deck.audio.as_ref().map(|a| a.sr).unwrap_or(44100);
        // With Q the loop starts on the nearest beat (rekordbox prepares an auto beat loop
        // with its SNAP rule; a loop ahead of the playhead begins when the playhead gets
        // there). Loops shorter than a beat start on the beat behind, so they sound at once.
        let start = if self.prefs.quantize && !self.deck.beats.is_empty() {
            let p = self.pos();
            let i = self.deck.beats.partition_point(|b| b.time <= p + 0.001);
            let behind = if i > 0 { self.deck.beats[i - 1].time } else { p };
            if self.performance() && beats >= 1.0 && self.playing() { crate::deck::quantize_point(&self.deck.beats, p, crate::deck::QType::Snap, 1.0, false) } else { behind }
        } else {
            self.pos()
        };
        let start = crate::deck::on_frame(start, sr);
        // The end is found on the grid (so a tempo change inside the loop is followed) and
        // both ends are whole frames; never shorter than rekordbox's minimum loop.
        let end = crate::deck::grid_pos(&self.deck.beats, start).and_then(|p| crate::deck::grid_time(&self.deck.beats, p + beats)).unwrap_or(start + beats * self.beat_len());
        let end = crate::deck::on_frame(end.max(start + crate::deck::MIN_LOOP_SECS), sr);
        self.player.state.lock().loop_range = Some((start, end));
        self.deck.last_loop = Some((start, end));
    }
    pub fn loop_active(&self) -> Option<(f64, f64)> {
        self.player.state.lock().loop_range
    }
    pub fn loop_exit(&mut self) {
        let mut s = self.player.state.lock();
        s.loop_range = None;
        if s.dk.arm_loop.take().is_some() {
            s.dk.warp = None;
        }
    }
    pub fn reloop(&mut self) {
        if let Some((a, b)) = self.deck.last_loop {
            self.player.state.lock().loop_range = Some((a, b));
            self.seek(a);
        }
    }
    pub fn loop_in(&mut self) {
        if self.perf_loop_in() {
            return;
        }
        let q = crate::prefs_bridge::loop_point(self, self.pos());
        self.deck.loop_in = Some(q);
    }
    pub fn loop_out(&mut self) {
        if self.perf_loop_out() {
            return;
        }
        if let Some(a) = self.deck.loop_in {
            let sr = self.deck.audio.as_ref().map(|a| a.sr).unwrap_or(44100);
            let (a, b) = (crate::deck::on_frame(a, sr), crate::deck::on_frame(crate::prefs_bridge::loop_point(self, self.pos()), sr));
            if b > a + crate::deck::MIN_LOOP_SECS {
                self.player.state.lock().loop_range = Some((a, b));
                self.deck.last_loop = Some((a, b));
                self.seek(a);
            }
        }
    }
    pub fn resize_loop(&mut self, factor: f64) {
        // (One step in the 16 sizes; a running loop is halved or doubled on the beat grid.)
        self.loop_step(if factor > 1.0 { 1 } else { -1 });
    }

    // ------------------------------------------------------------------ grid editing
    fn grid_editable(&mut self) -> bool {
        let Some(id) = self.deck.track else { return false };
        if self.deck.grid.is_empty() {
            self.toast_err("This track has no beat grid yet. Analyse it first.");
            return false;
        }
        if self.track(id).map(|t| t.grid_locked).unwrap_or(false) {
            self.toast_err("The beat grid is locked (Analysis Lock). Unlock it to edit.");
            return false;
        }
        true
    }
    fn push_grid(&mut self, g: BeatGrid) {
        let old = self.deck.grid.clone();
        if let Some(t) = self.deck.track {
            self.record(Edit::Grid { track: t, before: old.clone() });
        }
        self.deck.grid_undo.push(old);
        self.deck.grid_redo.clear();
        self.set_deck_grid(g, true);
    }
    fn anchor_at(&self, t: f64) -> usize {
        self.deck.grid.anchors.iter().rposition(|x| x.time <= t + 1e-6).unwrap_or(0)
    }
    pub fn grid_shift(&mut self, secs: f64) {
        if !self.grid_editable() {
            return;
        }
        let mut g = self.deck.grid.clone();
        let from = if self.prefs.grid_from_current { self.anchor_at(self.pos()) } else { 0 };
        for a in g.anchors.iter_mut().skip(from) {
            a.time += secs;
        }
        self.push_grid(g);
    }
    pub fn grid_scale_bpm(&mut self, f: impl Fn(f64) -> f64) {
        if !self.grid_editable() {
            return;
        }
        let mut g = self.deck.grid.clone();
        let p = self.pos();
        let Some(b) = nearest_beat(&self.deck.beats, p) else { return };
        if self.prefs.grid_from_current {
            // Keep the beat at the playhead and change the tempo from there on.
            let i = self.anchor_at(b.time);
            let bpm = f(g.anchors[i].bpm).clamp(40.0, 499.0);
            g.anchors.truncate(i + 1);
            if (g.anchors[i].time - b.time).abs() < 1e-6 {
                g.anchors[i].bpm = bpm;
            } else {
                g.anchors.push(GridAnchor { time: b.time, bpm, beat: b.beat });
            }
        } else {
            // Whole track: one tempo, keeping the beat nearest the playhead in place.
            let bpm = f(g.anchors[self.anchor_at(b.time)].bpm).clamp(40.0, 499.0);
            g = BeatGrid { anchors: vec![GridAnchor { time: b.time, bpm, beat: b.beat }] };
        }
        self.push_grid(g);
    }
    /// Double (`true`) or halve the BPM. For the whole grid this follows rekordbox's traced
    /// `bpmToDouble` / `bpmToHalf` (beats kept in place, every tempo change kept, beat numbers
    /// remapped); "from the current position" changes the tempo from the playhead on.
    pub fn grid_double_half(&mut self, double: bool) {
        if self.prefs.grid_from_current {
            self.grid_scale_bpm(|v| if double { v * 2.0 } else { v / 2.0 });
            return;
        }
        if !self.grid_editable() {
            return;
        }
        let dur = self.deck.beats.last().map(|b| b.time + 1e-3).unwrap_or(0.0);
        let g = if double { self.deck.grid.doubled(dur) } else { self.deck.grid.halved(dur) };
        if g.anchors.is_empty() || g.anchors.iter().any(|a| !(40.0..=499.0).contains(&a.bpm)) {
            self.toast_err("BPM value should be between 40 and 499.");
            return;
        }
        self.push_grid(g);
    }
    pub fn grid_set_bpm(&mut self, bpm: f64) {
        if !(40.0..=499.0).contains(&bpm) {
            self.toast_err("BPM value should be between 40 and 499.");
            return;
        }
        self.grid_scale_bpm(|_| bpm);
    }
    pub fn grid_set_downbeat(&mut self) {
        let Some(tid) = self.deck.track else { return };
        let p = self.pos();
        if self.track(tid).map(|t| t.grid_locked).unwrap_or(false) {
            self.toast_err("The beat grid is locked (Analysis Lock). Unlock it to edit.");
            return;
        }
        if self.deck.grid.is_empty() {
            let bpm = self.track(tid).map(|t| t.bpm).filter(|b| *b > 0.0).unwrap_or(120.0);
            self.push_grid(BeatGrid::constant(p, bpm, 1));
            return;
        }
        if !self.grid_editable() {
            return;
        }
        let mut g = self.deck.grid.clone();
        let i = self.anchor_at(p);
        let bpm = g.anchors[i].bpm;
        if self.prefs.grid_from_current {
            g.anchors.truncate(i + 1);
            if (g.anchors[i].time - p).abs() < 1e-6 {
                g.anchors[i] = GridAnchor { time: p, bpm, beat: 1 };
            } else {
                g.anchors.push(GridAnchor { time: p, bpm, beat: 1 });
            }
        } else {
            // Move the whole grid so that a bar's first beat lands on the playhead.
            let shift = p - g.anchors[i].time;
            for a in g.anchors.iter_mut() {
                a.time += shift;
            }
            g.anchors[i] = GridAnchor { time: p, bpm, beat: 1 };
        }
        self.push_grid(g);
    }
    pub fn grid_tap(&mut self) {
        if self.deck.track.and_then(|id| self.track(id)).map(|t| t.grid_locked).unwrap_or(false) {
            self.toast_err("The beat grid is locked (Analysis Lock). Unlock it to edit.");
            return;
        }
        let now = Instant::now();
        if let Some(l) = self.deck.taps.last() {
            if now.duration_since(*l).as_secs_f64() > 2.0 {
                self.deck.taps.clear();
            }
        }
        self.deck.taps.push(now);
        if self.deck.taps.len() >= 4 {
            let n = self.deck.taps.len();
            let span = self.deck.taps[n - 1].duration_since(self.deck.taps[0]).as_secs_f64();
            let bpm = ((n - 1) as f64 * 60.0 / span * 100.0).round() / 100.0;
            if self.deck.grid.is_empty() {
                let p = self.pos();
                self.push_grid(BeatGrid::constant(p, bpm, 1));
            } else {
                self.grid_set_bpm(bpm);
            }
        }
    }
    pub fn grid_undo(&mut self) {
        if !self.grid_editable() {
            return;
        }
        if let Some(g) = self.deck.grid_undo.pop() {
            let cur = self.deck.grid.clone();
            self.deck.grid_redo.push(cur);
            self.set_deck_grid(g, true);
        }
    }
    pub fn grid_redo(&mut self) {
        if !self.grid_editable() {
            return;
        }
        if let Some(g) = self.deck.grid_redo.pop() {
            let cur = self.deck.grid.clone();
            self.deck.grid_undo.push(cur);
            self.set_deck_grid(g, true);
        }
    }

    // ------------------------------------------------------------------ playlists
    pub fn add_to_playlist(&mut self, pl: i64, ids: &[i64]) {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        if self.node(pl).map(|n| n.kind != NodeKind::Playlist).unwrap_or(true) {
            return;
        }
        let mut cur = self.db.playlist_tracks(pl).unwrap_or_default();
        if cur.len() + ids.len() > 10000 {
            self.toast_err("Up to 10000 tracks can be added in one playlist");
            return;
        }
        // "There are duplicates being added to the playlist.": asked first.
        if self.dup_pending(pl, ids) {
            return;
        }
        self.add_to_playlist_force(pl, ids);
    }
    /// Add tracks to a playlist without asking about duplicates.
    pub fn add_to_playlist_force(&mut self, pl: i64, ids: &[i64]) {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        let mut cur = self.db.playlist_tracks(pl).unwrap_or_default();
        cur.extend_from_slice(ids);
        let _ = self.db.set_playlist_tracks(pl, &cur);
        self.reload_playlists();
        let name = self.node(pl).map(|n| n.name.clone()).unwrap_or_default();
        self.toast(format!("Added {} track(s) to \"{name}\"", ids.len()));
    }
    pub fn remove_from_playlist(&mut self, pl: i64, ids: &HashSet<i64>) {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        let cur: Vec<i64> = self.db.playlist_tracks(pl).unwrap_or_default().into_iter().filter(|t| !ids.contains(t)).collect();
        let _ = self.db.set_playlist_tracks(pl, &cur);
        self.reload_playlists();
    }
    pub fn move_in_playlist(&mut self, pl: i64, ids: &[i64], to: usize) {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        let cur = self.db.playlist_tracks(pl).unwrap_or_default();
        let set: HashSet<i64> = ids.iter().copied().collect();
        let before = cur.iter().take(to).filter(|t| set.contains(t)).count();
        let mut rest: Vec<i64> = cur.into_iter().filter(|t| !set.contains(t)).collect();
        let at = to.saturating_sub(before).min(rest.len());
        let moving: Vec<i64> = self.list.iter().filter(|t| set.contains(t)).copied().collect();
        for (k, t) in moving.into_iter().enumerate() {
            rest.insert(at + k, t);
        }
        let _ = self.db.set_playlist_tracks(pl, &rest);
        self.reload_playlists();
    }
    pub fn create_node(&mut self, parent: i64, kind: NodeKind) -> i64 {
        if crate::prefs_bridge::library_protected(self) {
            return 0;
        }
        if kind == NodeKind::Smart && self.nodes.iter().filter(|n| n.kind == NodeKind::Smart).count() >= 128 {
            self.toast_err("You can't create more than 128 intelligent lists.");
            return 0;
        }
        let name = match kind {
            NodeKind::Folder => "Untitled Folder",
            NodeKind::Smart => "Untitled Intelligent List",
            NodeKind::Playlist => "Untitled Playlist",
        };
        let smart = if kind == NodeKind::Smart { Some(SmartRule::default()) } else { None };
        let id = self.db.create_node(parent, name, kind.clone(), smart.as_ref()).unwrap_or(0);
        self.reload_playlists();
        if parent > 0 {
            self.expanded.insert(parent);
        }
        if kind == NodeKind::Smart {
            self.dialogs.smart_edit = Some((Some(id), parent, name.to_string(), SmartRule::default()));
        } else {
            self.dialogs.rename = Some((id, name.to_string()));
        }
        id
    }

    pub fn remove_from_collection(&mut self, ids: &[i64]) {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        // Remember removed files so a rekordbox sync does not add them back.
        let mut removed: HashSet<String> = self.db.setting("rb_removed_paths").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for id in ids {
            if let Some(t) = self.track(*id) {
                removed.insert(t.path.clone());
            }
        }
        let _ = self.db.conn.execute_batch("BEGIN");
        self.db.set_setting("rb_removed_paths", &serde_json::to_string(&removed).unwrap_or_default());
        let mut failed = None;
        for id in ids {
            if let Err(e) = self.db.trash_track(*id) {
                failed = Some(e);
                break;
            }
        }
        if let Some(e) = failed {
            let _ = self.db.conn.execute_batch("ROLLBACK");
            self.toast_err(format!("Could not remove the tracks ({e}). Nothing was changed."));
            return;
        }
        let _ = self.db.conn.execute_batch("COMMIT");
        for id in ids {
            if self.deck.track == Some(*id) {
                self.unload_track();
            }
        }
        self.selected.clear();
        self.reload_all();
        self.toast(format!("Removed {} track(s) from the collection. The files were not deleted; Track > Restore Removed Tracks brings them back.", ids.len()));
    }

    /// Put trashed tracks back, with their cues, grids, tags and playlist positions.
    pub fn restore_removed(&mut self, trash_ids: &[i64]) {
        let list = self.db.trash_list().unwrap_or_default();
        let mut removed: HashSet<String> = self.db.setting("rb_removed_paths").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let (mut ok, mut dup, mut err) = (0, 0, None);
        // Newest removal first, so each track goes back into the playlist positions it had.
        let mut trash_ids = trash_ids.to_vec();
        trash_ids.sort_by(|a, b| b.cmp(a));
        for id in &trash_ids {
            match self.db.restore_trash(*id) {
                Ok(Some(_)) => {
                    ok += 1;
                    if let Some(e) = list.iter().find(|e| e.0 == *id) {
                        removed.remove(&e.3);
                    }
                }
                Ok(None) => dup += 1,
                Err(e) => err = Some(e),
            }
        }
        self.db.set_setting("rb_removed_paths", &serde_json::to_string(&removed).unwrap_or_default());
        self.reload_all();
        if let Some(e) = err {
            self.toast_err(format!("Restored {ok} track(s); some could not be restored ({e})."));
        } else if dup > 0 {
            self.toast(format!("Restored {ok} track(s). {dup} are in the collection again already and were left in the list."));
        } else {
            self.toast(format!("Restored {ok} track(s) to the collection."));
        }
    }

    pub fn set_tag(&mut self, ids: &[i64], tag: i64, on: bool) {
        // (Advanced > Browse > Library Protection.)
        if crate::prefs_bridge::library_protected(self) {
            return;
        }
        for id in ids {
            let _ = self.db.set_track_tag(*id, tag, on);
            let e = self.track_tags.entry(*id).or_default();
            if on {
                e.insert(tag);
            } else {
                e.remove(&tag);
            }
        }
        // Preferences > View > Layout: Add "My Tag" to the "Comments".
        if self.prefs.mytag_to_comments {
            crate::prefs_ext::sync_mytag_comments(self, ids);
        }
        self.list_dirty = true;
    }

    pub fn reload_tags_from_files(&mut self, ids: &[i64]) {
        for id in ids {
            let Some(t) = self.track(*id).cloned() else { continue };
            let mut fresh = crate::importer::read_track(std::path::Path::new(&t.path));
            fresh.id = t.id;
            fresh.date_added = t.date_added.clone();
            fresh.rating = t.rating;
            fresh.color = t.color;
            fresh.analyzed = t.analyzed;
            fresh.grid_locked = t.grid_locked;
            fresh.autoload_hotcue = t.autoload_hotcue;
            fresh.gain_db = t.gain_db;
            fresh.play_count = t.play_count;
            fresh.missing = t.missing;
            fresh.raw_samples = t.raw_samples;
            // A key the user chose stays (tags never replace it).
            fresh.key_manual = t.key_manual;
            if t.key_manual {
                fresh.key = t.key;
                fresh.key_text = t.key_text.clone();
            }
            if t.analyzed == 1 {
                fresh.bpm = t.bpm;
                fresh.key = t.key;
                fresh.key_text = t.key_text.clone();
                fresh.duration = t.duration;
            }
            if let Some(slot) = self.track_mut(*id) {
                *slot = fresh;
            }
            self.save_track(*id);
        }
    }

    // ------------------------------------------------------------------ devices / export
    pub fn scan_devices(&mut self) {
        let mut found: Vec<(String, String, String)> = vec![];
        let mount = std::process::Command::new("/sbin/mount").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        for line in mount.lines() {
            // "/dev/disk4s1 on /Volumes/USB (msdos, local, nodev, nosuid, noowners)"
            let Some((_, rest)) = line.split_once(" on ") else { continue };
            let Some((mp, opts)) = rest.rsplit_once(" (") else { continue };
            if !mp.starts_with("/Volumes/") {
                continue;
            }
            let fs = opts.split(',').next().unwrap_or("").trim().to_string();
            if !matches!(fs.as_str(), "msdos" | "exfat" | "hfs" | "apfs") || opts.contains("read-only") {
                continue;
            }
            let name = mp.trim_start_matches("/Volumes/").to_string();
            let root = std::path::Path::new(mp);
            // Skip app installer images unless they already hold a device library.
            let installer = root.join("Applications").exists() || std::fs::read_dir(root).map(|d| d.flatten().any(|e| e.path().extension().map(|x| x == "app").unwrap_or(false))).unwrap_or(false);
            if name == "Recovery" || (installer && !root.join("PIONEER").exists()) {
                continue;
            }
            found.push((mp.to_string(), name, fs));
        }
        for extra in &self.prefs.devices_extra {
            if std::path::Path::new(extra).is_dir() {
                let name = std::path::Path::new(extra).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                found.push((extra.clone(), name, "folder".into()));
            }
        }
        let mut devices = vec![];
        for (root, name, fs) in found {
            let p = std::path::Path::new(&root);
            use crate::export::usb::{self, DeviceLayout};
            let mut target = usb::inspect_target(p);
            // A different catalog may now occupy a previously connected mount point.
            // Resolve any unfinished write before displaying the catalog as usable.
            let mut recovery_error = None;
            if self.export_job.is_none() && matches!(target, Ok((_, None))) {
                match usb::recover_checked(p) {
                    Ok(Some(msg)) => {
                        self.toast(format!("{name}: {msg}"));
                        target = usb::inspect_target(p);
                    }
                    Ok(None) => {}
                    Err(e) => recovery_error = Some(format!("USB recovery paused: {e:#}")),
                }
            }
            let (layout, lib, write_blocker) = match target {
                Ok((layout, _)) if recovery_error.is_some() => (Some(layout), None, recovery_error),
                Ok((layout @ (DeviceLayout::Classic | DeviceLayout::Mixed), blocker)) => match usb::read_device(p) {
                    Ok(lib) => (Some(layout), Some(lib), blocker),
                    Err(e) => (Some(layout), None, Some(format!("Could not read the classic device catalog: {e}"))),
                },
                Ok((layout, blocker)) => (Some(layout), None, blocker),
                Err(e) => (None, None, Some(format!("{e}"))),
            };
            devices.push(DeviceInfo { total: usb::total_space(p), free: usb::free_space(p), root, name, fs, layout, lib, write_blocker });
        }
        self.devices = devices;
        self.devices_scanned = Some(Instant::now());
        if self.prefs.import_device_history && self.screenshot.is_none() {
            self.import_device_histories();
            // "Delete from the device after importing the play history".
            if self.delete_imported_histories() > 0 {
                for d in self.devices.iter_mut().filter(|d| d.lib.is_some()) {
                    match crate::export::usb::read_device(std::path::Path::new(&d.root)) {
                        Ok(lib) => d.lib = Some(lib),
                        Err(e) => {
                            d.lib = None;
                            d.write_blocker = Some(e.to_string());
                        }
                    }
                }
            }
        }
        self.offer_auto_sync();
        // Look for player cue edits once per connection of each drive.
        let roots: HashSet<String> = self.devices.iter().map(|d| d.root.clone()).collect();
        self.edits_checked.retain(|r| roots.contains(r));
        if self.screenshot.is_none() && self.dialogs.player_edits.is_none() {
            let pending: Vec<String> = self.devices.iter().filter(|d| d.lib.is_some() && !self.edits_checked.contains(&d.root)).map(|d| d.root.clone()).collect();
            for root in pending {
                self.edits_checked.insert(root.clone());
                if self.check_player_cue_edits(&root, false) {
                    break;
                }
            }
        }
    }

    /// A device track's counterpart in the collection: the local file Recordbox exported, else
    /// the same title and artist.
    pub fn match_device_track(&self, source: &str, title: &str, artist: &str) -> Option<i64> {
        let by_source = (!source.is_empty()).then(|| self.db.track_by_path(source)).flatten();
        by_source.or_else(|| self.tracks.iter().find(|t| t.display_title().eq_ignore_ascii_case(title) && t.artist.eq_ignore_ascii_case(artist)).map(|t| t.id))
    }

    /// Compare the cues on a drive with what Recordbox last exported there and offer edits made
    /// on players for import. `asked`: the user chose the menu item (report when nothing is
    /// found, and include edits dismissed before). Returns whether the review dialog opened.
    pub fn check_player_cue_edits(&mut self, root: &str, asked: bool) -> bool {
        let Some(dev) = self.devices.iter().find(|d| d.root == root) else { return false };
        let Some(lib) = dev.lib.clone() else { return false };
        let device = dev.name.clone();
        let dismissed: HashSet<String> = self.db.setting("dismissed_player_edits").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut items = vec![];
        for edit in crate::export::usb::player_cue_edits(std::path::Path::new(root), &lib) {
            if !asked && dismissed.contains(&format!("{}\u{1f}{}", edit.file_path, edit.now)) {
                continue;
            }
            // Cues are replaced, so only the exact local file Recordbox exported counts as a match.
            let Some(track) = (!edit.source.is_empty()).then(|| self.db.track_by_path(&edit.source)).flatten() else { continue };
            let local = crate::export::anlz::local_cue_fingerprint(&self.db.cues(track).unwrap_or_default());
            if local == edit.now {
                continue;
            }
            let conflict = local != edit.exported;
            items.push(PlayerEditItem { track, edit, conflict, pick: !conflict });
        }
        if items.is_empty() {
            if asked {
                self.toast(format!("No cue changes made on players were found on {device}."));
            }
            return false;
        }
        self.dialogs.player_edits = Some(PlayerEdits { root: root.to_string(), device, items });
        true
    }

    /// Replace the chosen tracks' cues with the ones on the drive.
    pub fn apply_player_edits(&mut self, pe: &PlayerEdits) {
        let root = std::path::Path::new(&pe.root);
        let mut record = crate::export::usb::read_cue_record(root);
        let mut n = 0;
        for it in pe.items.iter().filter(|i| i.pick) {
            if self.db.replace_cues(it.track, &it.edit.cues).is_ok() {
                record.insert(it.edit.file_path.clone(), it.edit.now.clone());
                n += 1;
            }
        }
        if crate::export::usb::check_export_target(root).is_ok() {
            let _ = crate::export::usb::write_cue_record(root, &record);
        }
        self.dismiss_player_edits(pe, false);
        self.cue_counts = self.db.tracks_with_cues().unwrap_or_default();
        self.hot_positions = self.db.hot_positions().unwrap_or_default();
        if let Some(t) = self.deck.track {
            if pe.items.iter().any(|i| i.pick && i.track == t) {
                self.deck.cues = self.db.cues(t).unwrap_or_default();
            }
        }
        self.list_dirty = true;
        self.toast(format!("Imported cue changes for {n} track{} from {}.", if n == 1 { "" } else { "s" }, pe.device));
    }

    /// Don't offer these edits again (until the cues on the drive change once more).
    pub fn dismiss_player_edits(&mut self, pe: &PlayerEdits, only_unpicked: bool) {
        let mut dismissed: HashSet<String> = self.db.setting("dismissed_player_edits").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for it in pe.items.iter().filter(|i| !only_unpicked || !i.pick) {
            dismissed.insert(format!("{}\u{1f}{}", it.edit.file_path, it.edit.now));
        }
        self.db.set_setting("dismissed_player_edits", &serde_json::to_string(&dismissed).unwrap_or_default());
    }

    /// Copy play histories saved on connected devices into the "Histories" folder as playlists
    /// (once each). Device tracks are matched to the collection by the local file Recordbox
    /// exported, else by title and artist. The device itself is not changed.
    pub fn import_device_histories(&mut self) -> usize {
        // How many entries of each device history (device + name) were imported already. A
        // history that grew on a player since only adds its new entries (and plays).
        let mut counts: HashMap<String, usize> = self.db.setting("imported_history_counts").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        // Histories imported by an earlier version (keyed with their entries) count as done.
        let old: HashSet<String> = self.db.setting("imported_histories").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut imported = 0;
        let devices: Vec<(String, String, crate::export::usb::DeviceLibrary)> = self.devices.iter().filter_map(|d| d.lib.clone().map(|l| (d.root.clone(), d.name.clone(), l))).collect();
        for (_root, dev_name, lib) in devices {
            for h in &lib.histories {
                // A history is identified by the drive's name, its own name and the file of its
                // first entry (default volume names and history names repeat across sticks).
                // The first entry's file: fixed from the history's first play on, so the key
                // stays the same as the history grows.
                let head = h.entries.first().and_then(|e| lib.tracks.get(e)).map(|t| t.file_path.clone()).unwrap_or_default();
                let key = history_key(&dev_name, &h.name, &head);
                // An earlier version recorded the entries it imported; if they are the start of
                // this history, only what came after them is new.
                let legacy = legacy_imported(&old, &dev_name, &h.name, &h.entries);
                let done = counts.get(&key).copied().or(legacy).unwrap_or(0);
                if h.entries.len() <= done {
                    counts.insert(key, done);
                    continue;
                }
                let mut ids = vec![];
                let mut plays = vec![];
                for e in &h.entries[done..] {
                    let Some(dt) = lib.tracks.get(e) else { continue };
                    // Play counts only for the exact file Recordbox exported; the history list
                    // itself also takes title/artist matches.
                    let exact = (!dt.source.is_empty()).then(|| self.db.track_by_path(&dt.source)).flatten();
                    if let Some(id) = exact {
                        plays.push(id);
                    }
                    if let Some(id) = exact.or_else(|| self.match_device_track("", &dt.title, &dt.artist)) {
                        ids.push(id);
                    }
                }
                let folder = match self.nodes.iter().find(|n| n.parent == 0 && n.name == "Histories" && n.kind == NodeKind::Folder) {
                    Some(n) => n.id,
                    None => self.db.create_node(0, "Histories", NodeKind::Folder, None).unwrap_or(0),
                };
                let name = format!("{} ({dev_name})", h.name);
                let existing = self.db.playlists().unwrap_or_default().into_iter().find(|n| n.parent == folder && n.name == name && n.kind == NodeKind::Playlist).map(|n| n.id);
                let pl = match existing {
                    Some(pl) => Some(pl),
                    None => self.db.create_node(folder, &name, NodeKind::Playlist, None).ok(),
                };
                if let Some(pl) = pl {
                    let mut all = self.db.playlist_tracks(pl).unwrap_or_default();
                    all.extend(ids);
                    let _ = self.db.set_playlist_tracks(pl, &all);
                    imported += 1;
                    for id in &plays {
                        if let Some(t) = self.track_mut(*id) {
                            t.play_count = t.play_count.saturating_add(1);
                            self.save_track_quiet(*id);
                        }
                    }
                    counts.insert(key, h.entries.len());
                }
                self.reload_playlists();
            }
        }
        self.db.set_setting("imported_history_counts", &serde_json::to_string(&counts).unwrap_or_default());
        if imported > 0 {
            self.toast(format!("Imported {imported} play histor{} from your devices into the Histories folder.", if imported == 1 { "y" } else { "ies" }));
        }
        imported
    }

    pub fn fs_label(fs: &str) -> &'static str {
        match fs {
            "msdos" => "FAT32",
            "exfat" => "exFAT",
            "hfs" => "HFS+",
            "apfs" => "APFS (not readable by DJ players)",
            _ => "Folder",
        }
    }

    /// Device menu > Update Playlists on This Device: export again every playlist that is on the
    /// drive and in the library (same folder path and name), so the drive catches up with edits.
    /// Each is exported through its top-level folder, keeping the folder structure.
    pub fn update_device_playlists(&mut self, root: &str) {
        if self.export_job.is_some() {
            return self.toast_err("An export is already running.");
        }
        let Some(lib) = self.devices.iter().find(|d| d.root == root).and_then(|d| d.lib.clone()) else {
            return self.toast_err("This device has no library to update.");
        };
        let dev_path = |id: u32| -> Vec<String> {
            let mut out = vec![];
            let mut cur = Some(id);
            while let Some(c) = cur.filter(|c| *c != 0) {
                let Some(n) = lib.nodes.iter().find(|n| n.id == c) else { break };
                out.push(n.name.clone());
                cur = Some(n.parent);
            }
            out.reverse();
            out
        };
        let on_drive: HashSet<Vec<String>> = lib.nodes.iter().filter(|n| !n.folder).map(|n| dev_path(n.id)).collect();
        let lib_path = |app: &App, id: i64| -> (Vec<String>, i64) {
            let mut out = vec![];
            let mut top = id;
            let mut cur = id;
            while cur != 0 {
                let Some(n) = app.node(cur) else { break };
                out.push(n.name.clone());
                top = n.id;
                cur = n.parent;
            }
            out.reverse();
            (out, top)
        };
        // Only the matched playlists, each under copies of its folders, so the drive's
        // structure is kept and nothing new is added.
        fn insert(tree: &mut Vec<crate::export::usb::ExportNode>, path: &[String], tracks: Vec<i64>) {
            if path.len() == 1 {
                tree.push(crate::export::usb::ExportNode { id: 0, name: path[0].clone(), folder: false, children: vec![], tracks });
                return;
            }
            let i = match tree.iter().position(|n| n.folder && n.name == path[0]) {
                Some(i) => i,
                None => {
                    tree.push(crate::export::usb::ExportNode { id: 0, name: path[0].clone(), folder: true, children: vec![], tracks: vec![] });
                    tree.len() - 1
                }
            };
            insert(&mut tree[i].children, &path[1..], tracks);
        }
        let mut tree: Vec<crate::export::usb::ExportNode> = vec![];
        let mut matched = 0;
        let playlists: Vec<(i64, NodeKind, Option<crate::smart::SmartRule>)> = self.nodes.iter().filter(|n| n.kind != NodeKind::Folder).map(|n| (n.id, n.kind.clone(), n.smart.clone())).collect();
        for (id, kind, smart) in playlists {
            let (path, _) = lib_path(self, id);
            if on_drive.contains(&path) {
                matched += 1;
                let tracks = if kind == NodeKind::Smart { smart.as_ref().map(|r| self.smart_tracks(r)).unwrap_or_default() } else { self.db.playlist_tracks(id).unwrap_or_default() };
                insert(&mut tree, &path, tracks);
            }
        }
        if tree.is_empty() {
            return self.toast("None of this device's playlists are in your library under the same name.");
        }
        self.toast(format!("Updating {matched} playlist(s) on the device…"));
        self.start_export_nodes(root, tree, &[]);
    }

    /// Point a track at another file. Refused when another track already uses that file.
    pub fn relocate_track(&mut self, id: i64, path: &str) -> bool {
        if crate::prefs_bridge::library_protected(self) { return false; }
        if let Some(other) = self.db.track_by_path(path).filter(|o| *o != id) {
            let name = self.track(other).map(|t| t.display_title()).unwrap_or_default();
            self.toast_err(format!("That file already belongs to \"{name}\" in your collection."));
            return false;
        }
        let Some(before) = self.db.track(id) else {
            self.toast_err("The original track could not be read.");
            return false;
        };
        if let Err(error) = self.db.set_track_path(id, path) {
            self.toast_err(format!("Could not save the new location ({error})."));
            return false;
        }
        if before.path != path {
            self.record(Edit::Track { before });
        }
        if let Some(t) = self.track_mut(id) {
            t.path = path.to_string();
            t.missing = false;
        }
        self.list_dirty = true;
        true
    }

    /// File > Relocate Missing Files: look for each missing track's file name under `folder`
    /// (all subfolders). A unique match is used; several files with that name are narrowed to
    /// the one with the track's size, else left for the user (reported).
    pub fn relocate_missing(&mut self, folder: &std::path::Path) {
        if !self.tracks.iter().any(|t| t.missing) {
            return self.toast("No tracks are missing.");
        }
        self.start_relocation_scan(vec![folder.to_path_buf()], None);
    }

    pub(crate) fn start_relocation_scan(&mut self, folders: Vec<std::path::PathBuf>, selected: Option<Vec<i64>>) {
        if self.relocate_rx.is_some() {
            return self.toast_err("Already searching for missing files.");
        }
        let (tx, result) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::library_ext::scan_file_names(&folders));
            ctx.request_repaint();
        });
        self.relocate_rx = Some(RelocationSearch { result, selected });
        self.toast("Searching for missing files…");
    }

    pub fn poll_relocate(&mut self) {
        let Some(search) = &self.relocate_rx else { return };
        let by_name = match search.result.try_recv() {
            Ok(index) => index,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.relocate_rx = None;
                return self.toast_err("The folder search stopped. Run it again to retry.");
            }
        };
        let search = self.relocate_rx.take().unwrap();
        self.apply_relocation(&by_name, search.selected.as_deref());
    }

    pub fn start_export(&mut self, root: &str, playlist_ids: &[i64], extra_tracks: &[i64]) {
        if self.export_job.is_some() {
            self.toast_err("An export is already running.");
            return;
        }
        fn build(app: &App, id: i64) -> Option<crate::export::usb::ExportNode> {
            let n = app.node(id)?;
            match n.kind {
                NodeKind::Folder => {
                    let children: Vec<_> = app.nodes.iter().filter(|c| c.parent == id).filter_map(|c| build(app, c.id)).collect();
                    Some(crate::export::usb::ExportNode { id, name: n.name.clone(), folder: true, children, tracks: vec![] })
                }
                NodeKind::Smart => {
                    let tracks = n.smart.as_ref().map(|r| app.smart_tracks(r)).unwrap_or_default();
                    Some(crate::export::usb::ExportNode { id, name: n.name.clone(), folder: false, children: vec![], tracks })
                }
                NodeKind::Playlist => Some(crate::export::usb::ExportNode { id, name: n.name.clone(), folder: false, children: vec![], tracks: app.db.playlist_tracks(id).unwrap_or_default() }),
            }
        }
        // Children of a selected folder come with the folder.
        let sel: HashSet<i64> = playlist_ids.iter().copied().collect();
        let mut nodes = vec![];
        for id in playlist_ids {
            let mut p = self.node(*id).map(|n| n.parent).unwrap_or(0);
            let mut covered = false;
            while p != 0 {
                if sel.contains(&p) {
                    covered = true;
                    break;
                }
                p = self.node(p).map(|n| n.parent).unwrap_or(0);
            }
            if !covered {
                if let Some(n) = build(self, *id) {
                    nodes.push(n);
                }
            }
        }
        self.start_export_nodes(root, nodes, extra_tracks);
    }

    /// Export a prepared playlist tree (and extra tracks) to the drive at `root`.
    pub fn start_export_nodes(&mut self, root: &str, nodes: Vec<crate::export::usb::ExportNode>, extra_tracks: &[i64]) {
        if self.export_job.is_some() {
            self.toast_err("An export is already running.");
            return;
        }
        if let Err(e) = crate::export::usb::check_export_target(std::path::Path::new(root)) {
            self.device_ui.pending = None;
            self.toast_err(format!("Cannot export to {root}: {e}"));
            return;
        }
        let mut track_ids: HashSet<i64> = extra_tracks.iter().copied().collect();
        fn collect(n: &crate::export::usb::ExportNode, out: &mut HashSet<i64>) {
            out.extend(n.tracks.iter().copied());
            for c in &n.children {
                collect(c, out);
            }
        }
        for n in &nodes {
            collect(n, &mut track_ids);
        }
        let names: HashMap<i64, (String, i64)> = self.mytags.iter().map(|t| (t.id, (t.name.clone(), t.parent))).collect();
        let mut tracks = HashMap::new();
        let mut skipped: Vec<String> = vec![];
        let rb_anlz: HashMap<String, String> = self.db.setting("rb_anlz").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let settings = self.analysis_settings();
        let mut ids: Vec<i64> = track_ids.into_iter().collect();
        ids.sort();
        for id in &ids {
            let Some(t) = self.track(*id).cloned() else { continue };
            // A missing track still goes to the export job: if the drive already has its file,
            // the drive's copy is kept in the playlists; otherwise the job reports it skipped.
            if !t.missing && !crate::export::usb::player_supported(&t.file_type) {
                skipped.push(format!("{} (file type not supported by the players)", t.display_title()));
                continue;
            }
            let tags: Vec<(String, String)> = self
                .track_tags
                .get(id)
                .map(|s| s.iter().filter_map(|x| names.get(x)).map(|(n, p)| (names.get(p).map(|c| c.0.clone()).unwrap_or_default(), n.clone())).collect())
                .unwrap_or_default();
            // rekordbox's own analysis of this file, when the library was synced from it.
            let grid = self.db.grid(*id);
            let rb_anlz = rb_anlz.get(&t.path).map(std::path::PathBuf::from).filter(|b| b.with_extension("DAT").is_file());
            // Whether the grid still matches rekordbox's is worked out on the export thread.
            tracks.insert(*id, crate::export::usb::ExportTrack { cues: self.db.cues(*id).unwrap_or_default(), grid, wave: self.db.wave(*id), track: t, tags, rb_anlz, grid_same: false, vocal: self.db.vocal(*id), phrases: self.stored_phrases(*id) });
        }
        // Tracks without analysis are analysed by the export job itself (off the UI thread).
        let tag_categories: Vec<(String, Vec<String>)> = self
            .mytags
            .iter()
            .filter(|c| c.parent == 0)
            .map(|c| (c.name.clone(), self.mytags.iter().filter(|t| t.parent == c.id).map(|t| t.name.clone()).collect()))
            .collect();
        let req = crate::export::usb::ExportRequest { root: std::path::PathBuf::from(root), nodes, tracks, tag_categories, skipped, analysis: settings, settings: self.prefs.usb_settings.clone(), browse: self.prefs.usb_browse.clone(), phrases: self.prefs.phrase_analysis, ..self.export_options(root) };
        self.export_report = None;
        self.export_job = Some(spawn_export(req, self.ctx.clone()));
    }

    pub fn poll_export(&mut self) {
        let Some(job) = self.export_job.as_mut() else { return };
        let mut finished = None;
        let mut report = None;
        let mut analyzed = vec![];
        while let Ok(p) = job.rx.try_recv() {
            match p {
                crate::export::usb::Progress::Step { done, total, text } => job.last = (done, total, text),
                crate::export::usb::Progress::Analyzed(id, r) => analyzed.push((id, *r)),
                crate::export::usb::Progress::Report(r) => report = Some(r),
                crate::export::usb::Progress::Finished(r) => finished = Some(r),
            }
        }
        for (id, r) in analyzed {
            // The library worker owns any explicit grid/key request for this track.
            // An overlapping USB export uses its own analysis settings.
            if self.track(id).is_some_and(|t| t.analyzed != 3) {
                self.apply_analysis(id, r);
            }
        }
        if let Some(r) = report {
            self.export_report = Some(r);
        }
        if let Some(r) = finished {
            self.export_job = None;
            match r {
                Ok(msg) => self.toast(msg),
                Err(e) => self.toast_err(e),
            }
            self.scan_devices();
        }
    }

    pub fn export_collection_xml(&mut self, path: &std::path::Path) {
        let xml = crate::xmlexport::build(self);
        match std::fs::write(path, xml) {
            Ok(_) => self.toast("Collection exported in xml format."),
            Err(e) => self.toast_err(format!("Could not write the file: {e}")),
        }
    }
}

/// Same key or harmonically related (±1 on the wheel, or relative major/minor).
pub fn compatible_key(a: i32, b: i32) -> bool {
    if a < 0 || b < 0 {
        return false;
    }
    let (na, ma) = (a % 12, a / 12);
    let (nb, mb) = (b % 12, b / 12);
    if ma == mb {
        let d = (na - nb).rem_euclid(12);
        d == 0 || d == 1 || d == 11
    } else {
        na == nb
    }
}

pub struct RelocationSearch {
    result: Receiver<HashMap<String, Vec<std::path::PathBuf>>>,
    selected: Option<Vec<i64>>,
}

#[derive(Debug, PartialEq)]
pub enum RelocationMatch<'a> {
    Found(&'a std::path::Path, u64),
    Shared,
    NoMatch,
    InUse,
    Multiple,
    SizeMismatch,
    Unverified,
}

/// Classify the available same-named candidates. An unreadable candidate could
/// still share the saved size, so the user must resolve that case manually.
pub fn pick_relocation<'a>(candidates: &[(&'a std::path::Path, Option<u64>)], size: u64) -> RelocationMatch<'a> {
    if candidates.is_empty() {
        return RelocationMatch::NoMatch;
    }
    if candidates.iter().any(|(_, length)| length.is_none()) {
        return RelocationMatch::Unverified;
    }
    let mut matching = candidates.iter().filter(|(_, length)| size == 0 || *length == Some(size));
    match (matching.next(), matching.next()) {
        (Some((path, Some(length))), None) => RelocationMatch::Found(path, *length),
        (None, _) => RelocationMatch::SizeMismatch,
        _ => RelocationMatch::Multiple,
    }
}

/// One-time move of the reference loudness from the old default -8 LUFS to rekordbox's -10.
/// Stored gains (library and trash) are measured against the target, so they shift by -2 dB
/// with it. Everything happens in one transaction with the "done" flag; on error nothing
/// changes and the caller tries again next launch. Returns the preferences to use.
pub fn migrate_lufs_default(db: &Db, prefs: &Prefs, raw_prefs: Option<&str>) -> anyhow::Result<Prefs> {
    // Read from the raw text too, in case the preferences could not be parsed as a whole.
    let was_old_default = prefs.target_lufs == -8.0 || raw_prefs.and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok()).and_then(|v| v.get("target_lufs").and_then(|x| x.as_f64())) == Some(-8.0);
    let mut next = prefs.clone();
    if was_old_default {
        next.target_lufs = -10.0;
    }
    db.conn.execute_batch("BEGIN")?;
    let r = (|| -> anyhow::Result<()> {
        if was_old_default {
            db.conn.execute("UPDATE tracks SET gain_db = gain_db - 2.0 WHERE analyzed = 1 AND gain_db IS NOT NULL", [])?;
            db.conn.execute(
                "UPDATE trash SET json = json_set(json, '$.track.gain_db', json_extract(json, '$.track.gain_db') - 2.0) WHERE json_extract(json, '$.track.analyzed') = 1 AND json_extract(json, '$.track.gain_db') IS NOT NULL",
                [],
            )?;
            db.conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('prefs', ?1)", [serde_json::to_string(&next)?])?;
        }
        db.conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('lufs_default_migrated', '1')", [])?;
        Ok(())
    })();
    match r {
        Ok(()) => {
            db.conn.execute_batch("COMMIT")?;
            Ok(next)
        }
        Err(e) => {
            let _ = db.conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Identity of a device play history for "already imported" counting: drive name, history
/// name and the file of its first entry (stable as the history grows).
pub fn history_key(device: &str, name: &str, first_file: &str) -> String {
    format!("{device}\u{1f}{name}\u{1f}{first_file}")
}

/// Entries of `entries` an earlier version imported (its keys held the full entry list then):
/// the longest recorded list that is a prefix of the current one.
pub fn legacy_imported(old: &HashSet<String>, device: &str, name: &str, entries: &[u32]) -> Option<usize> {
    let prefix = format!("{device}\u{1f}{name}\u{1f}");
    old.iter()
        .filter_map(|k| k.strip_prefix(&prefix))
        .filter_map(|list| serde_json::from_str::<Vec<u32>>(list).ok())
        .filter(|prev| entries.starts_with(prev))
        .map(|prev| prev.len())
        .max()
}

/// Snap `t` to the nearest 1/`div` of a beat along the grid's own beats (div 1 = whole
/// beats). Beyond the grid's ends the nearest beat interval is extended.
pub fn quantize_to(beats: &[Beat], t: f64, div: u8) -> f64 {
    let div = div.max(1) as f64;
    if beats.len() < 2 {
        return nearest_beat(beats, t).map(|b| b.time).unwrap_or(t);
    }
    let n = beats.len();
    let i = beats.partition_point(|b| b.time <= t).saturating_sub(1).min(n - 2);
    let (a, b) = (beats[i].time, beats[i + 1].time);
    let len = (b - a).max(1e-9);
    let steps = ((t - a) / len * div).round();
    (a + steps / div * len).max(0.0)
}

/// Size and modification time (seconds) of an audio file, taken when it is analysed.
pub fn media_stamp(m: &std::fs::Metadata) -> (i64, i64) {
    let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    (m.len() as i64, mtime)
}

/// Jump a whole number of beats along the grid's own beats (so tempo changes are followed),
/// keeping the position within the beat. None for fractional jumps or an empty grid; beyond
/// the grid's ends the nearest beat interval is extended.
/// Where a loop of `n` beats starting at `start` ends, walking the grid's own beats so the
/// loop stays on the grid through a tempo change (AUD-006). Whole numbers of beats use
/// [`grid_jump`]; a fraction is that part of the beat interval the loop starts in.
pub fn loop_end_on_grid(beats: &[Beat], start: f64, n: f64) -> Option<f64> {
    if n <= 0.0 {
        return None;
    }
    if n.fract() == 0.0 {
        return grid_jump(beats, start, n).filter(|e| *e > start);
    }
    let whole = n.floor();
    let from = if whole > 0.0 { grid_jump(beats, start, whole)? } else { start };
    let next = grid_jump(beats, from, 1.0)?;
    Some(from + (next - from) * (n - whole)).filter(|e| *e > start)
}

pub fn grid_jump(beats: &[Beat], p: f64, n: f64) -> Option<f64> {
    if beats.len() < 2 || n.fract() != 0.0 {
        return None;
    }
    let len = beats.len() as i64;
    let i = (beats.partition_point(|b| b.time <= p) as i64 - 1).clamp(0, len - 2);
    let (a, b) = (beats[i as usize].time, beats[i as usize + 1].time);
    let frac = (p - a) / (b - a).max(1e-9);
    let k = i + n as i64;
    let at = |k: i64| -> f64 {
        if k < 0 {
            beats[0].time + k as f64 * (beats[1].time - beats[0].time)
        } else if k >= len - 1 {
            let d = beats[(len - 1) as usize].time - beats[(len - 2) as usize].time;
            beats[(len - 1) as usize].time + (k - (len - 1)) as f64 * d
        } else {
            beats[k as usize].time
        }
    };
    Some(at(k) + frac * (at(k + 1) - at(k)))
}

/// A PVDI section: header 24 bytes (u16 discriminator at 12, hop at 14, sample rate at 16,
/// stride at 18, count at 20), then count × stride bytes. Scrambled sections (negative
/// discriminator) are not readable yet.
pub fn parse_pvdi(s: &[u8]) -> Option<std::sync::Arc<(f64, Vec<u8>)>> {
    let u16at = |o: usize| -> Option<u16> { Some(u16::from_be_bytes(s.get(o..o + 2)?.try_into().ok()?)) };
    let hlen = u32::from_be_bytes(s.get(4..8)?.try_into().ok()?) as usize;
    if (u16at(12)? as i16) < 0 {
        return None;
    }
    let (hop, sr, stride) = (u16at(14)? as f64, u16at(16)? as f64, u16at(18)?.max(1) as usize);
    let n = u32::from_be_bytes(s.get(20..24)?.try_into().ok()?) as usize;
    if hop <= 0.0 || sr <= 0.0 {
        return None;
    }
    let body = s.get(hlen..hlen + n.checked_mul(stride)?)?;
    Some(std::sync::Arc::new((hop / sr, body.chunks_exact(stride).map(|c| c[0]).collect())))
}

pub fn fmt_time(secs: f64, tenths: bool) -> String {
    let s = secs.max(0.0);
    let m = (s / 60.0).floor() as u64;
    let r = s - m as f64 * 60.0;
    if tenths {
        format!("{:02}:{:04.1}", m, (r * 10.0).floor() / 10.0)
    } else {
        format!("{:02}:{:02}", m, r.floor() as u64)
    }
}
