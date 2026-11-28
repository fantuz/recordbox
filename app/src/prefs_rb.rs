//! The settings of rekordbox 7.2.19's Preferences window that recordbox did not have yet, and
//! which categories and tabs the window shows in which mode.
//!
//! Every field is one key of rekordbox's `rekordbox3.settings` and keeps rekordbox's own
//! encoding and default (`checks/re-7.2.19/preferences-skins-settings.tsv`), so that a page
//! shows what rekordbox shows on a new installation. `SPEC` lists key, default and the field
//! for the tests. Who acts on a setting is said at the field; "stored" means nothing reads it
//! yet (the page says so on hover).

use serde::{Deserialize, Serialize};

/// The header's mode list (`ApplicationMode`): rekordbox's combo ids 1..4 less one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Export,
    Performance,
    Lighting,
    Edit,
}

impl Mode {
    /// From `Prefs::app_mode` (0 EXPORT, 1 PERFORMANCE, 2 LIGHTING, 3 EDIT).
    pub fn of(app_mode: u8) -> Mode {
        match app_mode {
            1 => Mode::Performance,
            // (recordbox has no LIGHTING mode: 2 is EDIT, as ui_edit.rs sets it.)
            2 => Mode::Edit,
            _ => Mode::Export,
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn name(self) -> &'static str {
        ["EXPORT", "PERFORMANCE", "LIGHTING", "EDIT"][self.index()]
    }
}

/// The window's categories as recordbox numbers them (`Dialogs::prefs_tab`). 100 and 101 are
/// rekordbox's PLAN and CLOUD, which recordbox shows dimmed.
pub const PLAN: u8 = 100;
pub const CLOUD: u8 = 101;
pub const VIEW: u8 = 0;
pub const ANALYSIS: u8 = 1;
pub const DJ_SYSTEM: u8 = 2;
pub const KEYBOARD: u8 = 3;
pub const ADVANCED: u8 = 4;
pub const AUDIO: u8 = 5;
pub const EXTENSIONS: u8 = 6;
pub const CONTROLLER: u8 = 7;
pub const DVS: u8 = 8;

pub fn category_name(c: u8) -> &'static str {
    match c {
        PLAN => "PLAN",
        CLOUD => "CLOUD",
        VIEW => "View",
        AUDIO => "Audio",
        ANALYSIS => "Analysis",
        DJ_SYSTEM => "DJ System",
        CONTROLLER => "Controller",
        KEYBOARD => "Keyboard",
        ADVANCED => "Advanced",
        DVS => "DVS",
        EXTENSIONS => "Extensions",
        _ => "",
    }
}

/// The categories of a mode in sidebar order (`PreferenceCategoryComponent::getCategoryList`
/// 0x10136ff30; the Lighting and Edit lists are read from its branches, not from a capture).
pub fn categories(mode: Mode) -> &'static [u8] {
    match mode {
        Mode::Export => &[PLAN, CLOUD, VIEW, AUDIO, ANALYSIS, DJ_SYSTEM, KEYBOARD, ADVANCED, EXTENSIONS],
        Mode::Performance => &[PLAN, CLOUD, VIEW, AUDIO, ANALYSIS, CONTROLLER, KEYBOARD, ADVANCED, DVS, EXTENSIONS],
        Mode::Lighting => &[PLAN, CLOUD, VIEW, AUDIO, ANALYSIS, CONTROLLER, EXTENSIONS],
        Mode::Edit => &[PLAN, CLOUD, VIEW, AUDIO, ANALYSIS, CONTROLLER, KEYBOARD, ADVANCED, EXTENSIONS],
    }
}

/// The tabs of a category in a mode (`PreferencePageSelectComponent::setCategoryType`
/// 0x1012016c0), without the device tabs a connected controller adds.
pub fn tabs(category: u8, mode: Mode) -> &'static [&'static str] {
    match category {
        VIEW => &["Display Type", "Layout", "Color"],
        AUDIO => &["Configuration", "Input/Output"],
        ANALYSIS => &["Track Analysis", "CUE Analysis"],
        DJ_SYSTEM => &["General", "Category", "Sort", "Column", "My Settings", "Device", "Others"],
        CONTROLLER => &["Deck", "Mixer", "Effect", "Sampler", "Recordings", "MIX POINT LINK", "Others"],
        KEYBOARD => &["Keyboard"],
        DVS => &["DVS"],
        EXTENSIONS => match mode {
            Mode::Performance => &["STEMS", "Video", "Lighting"],
            Mode::Export | Mode::Edit => &["STEMS"],
            _ => &["Lighting"],
        },
        _ => match mode {
            Mode::Export => &["Database", "Browse", "Recordings", "Others"],
            Mode::Lighting => &["Browse", "Others"],
            _ => &["Database", "Browse", "Others"],
        },
    }
}

/// Tabs rekordbox has that are outside recordbox's scope: listed as rekordbox lists them,
/// greyed, with the reason.
pub fn tab_out_of_scope(category: u8, tab: &str) -> Option<&'static str> {
    match (category, tab) {
        (EXTENSIONS, "Video") => Some("Video is outside recordbox's scope."),
        (EXTENSIONS, "Lighting") => Some("Lighting is outside recordbox's scope."),
        _ => None,
    }
}

/// One line of a QUANTIZE table (Controller > Others > Type): on/off and the beat value's id
/// (rekordbox's combo ids: 1 = 1/32, 2 = 1/16, 3 = 1/8, 4 = 1/4, 5 = 1/2, 6 = 1/1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QRow {
    pub on: bool,
    pub beat: u8,
}

/// The beat values of the quantize dropdowns.
pub const Q_BEATS: [&str; 6] = ["1/32", "1/16", "1/8", "1/4", "1/2", "1/1"];

/// What the header shows in a mode: the panel toggles and the layout, restored when the mode
/// is entered again (rekordbox's `ShowPanel_*`, `WindowLayout`, `DjLayout`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModeShell {
    /// False until the mode has been left once (nothing to restore before that).
    pub saved: bool,
    pub fx: bool,
    pub sampler: bool,
    pub four_decks: bool,
    pub player_layout: u8,
    pub browser_only: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RbPrefs {
    // ---- View > Display Type
    /// `ShowOriginalKey` "Display key information on the database". Stored.
    pub show_original_key: bool,
    /// Waveform Drawing Rate: 0 High Speed, 1 Medium Speed, 2 Low Speed (`RenderDelay`). Stored.
    pub draw_rate: u8,
    /// `HideDeckInfo` "Hide Title, Artist, and Artwork" (Performance decks). Stored.
    pub hide_deck_info: bool,
    /// `DeckJogIndicator`: 1 Current CUE / SLIP, 2 HOT CUE COUNTDOWN. Stored.
    pub jog_indicator: u8,
    /// `TrafficLightRange` (id of the dropdown entry). Stored.
    pub traffic_light: u8,
    // ---- View > Layout
    /// `showBpmSyncRate` "Display SYNC RATE". Stored.
    pub show_sync_rate: bool,
    /// `FxPanelLayoutType`: 1 BEAT FX, 2 SOUND COLOR FX, 3 both. Read by `fx_rows`.
    pub fx_panel_layout: u8,
    /// Enlarged Waveform: Customize instead of Auto, and the two layouts
    /// (`ZoomWaveHorizontalLayoutType`, `ZoomWaveVerticalLayoutType`). Stored.
    pub zoom_wave_custom: bool,
    pub zoom_wave_h: u8,
    pub zoom_wave_v: u8,
    /// PAD Mode: Customize instead of Auto, and `PadModeLayoutType`. Stored.
    pub pad_mode_custom: bool,
    pub pad_mode_layout: u8,
    // ---- View > Color
    /// `ColorThemeMode`: 1 dark, 2 light. EDIT uses its dark workspace in either appearance.
    pub color_theme: u8,
    /// `HotCueColorType`: 1 CDJ, 2 COLD1, 3 COLD2, 4 COLORFUL. Stored (new hot cues are COLORFUL).
    pub hot_cue_color: u8,
    /// `SamplerColorType`: 0 Sync with Frequency, 1 Fixed (the default; traced in
    /// `SamplerControlComp::onLoadedClipChanged` 0x100b19f94). Read by `sampler_color_code`.
    pub sampler_color: u8,
    /// `PadFxColor<FX NAME>`: pad colour ids chosen per effect. Stored.
    pub pad_fx_colors: std::collections::BTreeMap<String, u8>,
    // ---- Audio
    /// Mixer Mode: External instead of Internal. Stored (recordbox mixes internally).
    pub mixer_external: bool,
    /// First channel of the Booth Output pair (None: "-"). Stored.
    pub booth_channel: Option<u16>,
    // ---- Controller > Deck
    /// `DeckNeedleLock` (1 = Lock). Stored.
    pub needle_lock: bool,
    /// `DeckMemoryCueCallLock` (1 = Lock). Stored.
    pub memory_cue_call_lock: bool,
    /// `DeckLoadingStartPosition` "Playback starts at Memory/Hot Cue nearest to the beginning
    /// of the track". Stored.
    pub load_start_cue: bool,
    /// `AutoCueLeveldB`: id of the Auto Cue Level entry (5 = -60dB), and `AutoCueEnableDeck<n>`.
    /// Stored.
    pub auto_cue_level: u8,
    pub auto_cue_deck: [bool; 4],
    /// Vinyl Speed Adjust: `BrakeValue0/2`, `BrakeValue1/3`, `StartValue0/2`, `StartValue1/3`
    /// (0..255). Stored.
    pub brake: [u8; 2],
    pub start: [u8; 2],
    /// `DeckJogSpeed`: 1 = 33rpm, 2 = 45rpm; `jogSamplerMode`. Stored.
    pub jog_speed: u8,
    pub jog_sampler_mode: bool,
    /// `AutoPlay` Continuous Playing. Stored.
    pub auto_play: bool,
    /// `BeatSyncType`: 1 BEAT SYNC, 2 BPM SYNC; `multiBpmSyncMode` "Allow BEAT/BPM SYNC with
    /// double/half BPM.". Stored.
    pub beat_sync_type: u8,
    pub multi_bpm_sync: bool,
    /// `SmartCueEnabled` and `HotCueSampler` (Gate Cue). Stored.
    pub smart_cue: bool,
    pub gate_cue: bool,
    /// `AutoBeatLoop`: id of the AUTO BEAT LOOP entry. Stored.
    pub auto_beat_loop: u8,
    // ---- Controller > Mixer
    /// `AutoGain`. Stored (the deck track reads it).
    pub auto_gain: bool,
    /// `EqualizerType`: 0 DJM-900NXS, 1 DJM-900NXS2. Stored (the mixer has the NXS2 curves).
    pub eq_type: u8,
    /// `CrossFaderCurve`, 0..=32 (16 in the middle). Stored (the deck track reads it).
    pub crossfader_curve: u8,
    /// `HeadRoomLevel`: id of the Headroom entry. Stored (the deck track reads it).
    pub headroom: u8,
    // ---- Controller > Others
    /// `DeckQuantizeMode` (2 = "Jump before reaching the next beat." on). Stored.
    pub quantize_jump_before: bool,
    /// Quantize > Type: SNAP, and QUANTIZE for HOT CUE, LOOP/SAMPLER(LOOP), REVERSE
    /// (`InputQuantizeStatus`, `InputQuantizeBeat`; the SEQUENCER row is `prefs.sampler.seq`).
    /// SNAP's value is read by `App` as the quantize resolution; the others are stored.
    pub snap: QRow,
    pub q_hot_cue: QRow,
    pub q_loop: QRow,
    pub q_reverse: QRow,
    /// `DeckQuantizeInOneGo`: All Decks instead of Each Deck / Sampler Deck. Stored.
    pub quantize_in_one_go: bool,
    /// `AbletonLinkStartStopSync`, `AbletonLinkShowWindow`. Stored (no Ableton Link).
    pub link_start_stop: bool,
    pub link_show_window: bool,
    // ---- DVS
    /// `EnableDVSFunction`, `DvsWaveViewBpmScaleMode`, `DvsVinylLeadInSongHead`, Control
    /// Hardware (0 Turntables, 1 CDJs, 2 Customized Setting). Stored (no DVS).
    pub dvs_enable: bool,
    pub dvs_fixed_bpm_scale: bool,
    pub dvs_lead_in: bool,
    pub dvs_hardware: u8,
    // ---- Advanced > Database
    /// `MusicAppLoadingType`: 1 Framework (Software), 0 iTunes Library XML. Stored (the
    /// iTunes source tries the chosen file, then the usual places, then the Music app).
    pub itunes_framework: bool,
    /// `bridgeConvertiTunesGrouping`, `OverwriteSyncList`. Stored.
    pub itunes_grouping_to_label: bool,
    pub itunes_overwrite_sync: bool,
    /// `bridgeExportBeatGrid`: 0 No, 1 BPM change points (the default; traced in
    /// `DetailrbXmlExportBeatInfoComponent::buttonClicked` 0x10143f39c). Read by xmlexport.rs.
    pub xml_beatgrid: u8,
    /// Auto Relocate Search Folders: Music, Video, Desktop, Specified user folders
    /// (`AutoRelocateSelect*Folder`). `relocate_roots` gives the folders; Auto Relocate itself
    /// still searches the listed user folders only.
    pub relocate_music: bool,
    pub relocate_video: bool,
    pub relocate_desktop: bool,
    pub relocate_user: bool,
    /// `autoReExport` Auto Export. Stored.
    pub auto_export: bool,
    // ---- Advanced > Browse
    /// `CacheTrackList` "Display playlists quicker.". Stored (lists are always cached).
    pub cache_track_list: bool,
    /// `ContextMenu` "Display the menu when long-pressing a track for 1 second.". Stored.
    pub long_press_menu: bool,
    /// Tag List: two independent check boxes, both off by default (`TaglistLoadOption`,
    /// `TaglistDoNotSaveOption`; getters 0x1015d3fc0 / 0x1015d4138 return false for a missing
    /// key, `DetailTagList::reset` 0x101c94268 clears both). Read by prefs_bridge.rs: a track
    /// loaded from the Tag List leaves it (`LoadedContentsManager::removeFromTaglist`
    /// 0x10139dfb8); the list is emptied on quitting (`DatabaseMediator::~DatabaseMediator`
    /// 0x101e4a594 calls `cleanTaglist`).
    pub tag_list_on_load: bool,
    pub tag_list_on_exit: bool,
    /// `LibraryProtection` "Protect library edit.". Stored.
    pub library_protection: bool,
    /// `EnabledInlineEditWithDblClick` Library Edit "Double-click". Stored.
    pub inline_edit_dblclick: bool,
    /// `playedTime`: seconds before a track is added to Histories (1..=60). Stored.
    pub played_time: u8,
    // ---- Advanced > Others
    /// `exportName` Computer Name > Export Name (empty: the computer's name). Stored.
    pub export_name: String,
    /// `DeckQuantize` QUANTIZE BEAT VALUE (Export): id as in `Q_BEATS` less the two finest
    /// (1 = 1/8 .. 4 = 1/1 ... see `quantize_div`). Read by `App` as the quantize resolution.
    pub deck_quantize: u8,
    /// `LoopAsWavSampleRate`: 48000 or 44100. Stored.
    pub loop_wav_rate: u32,
    // ---- DJ System
    /// `useLinkMonitor` "Use "LINK MONITOR" of DJ Mixers". Stored.
    pub link_monitor: bool,
    /// Image On-Jog Display: Custom Image instead of Artwork. Stored.
    pub jog_custom_image: bool,
    /// DJ System > General > Key display format on CDJ/XDJ: Alphanumeric instead of Classic.
    /// Stored.
    pub cdj_key_alnum: bool,
    // ---- Extensions > STEMS
    /// `PartLayout`: 0 Auto, 1 Customize, 2 Do not display; `CustomPartLayout` (id of the
    /// dropdown entry). Stored.
    pub part_layout: u8,
    pub custom_part_layout: u8,
    /// `PartInstDoublesDirection`: Left/Right decks (true) or Up/Down decks. Stored.
    pub part_inst_doubles_lr: bool,
    /// `IncreaseMemoryPartAnalysis`, `ApplyMultithreadingPartAnalysis`. Stored.
    pub part_more_memory: bool,
    pub part_multithread: bool,
    /// `GrooveCircuitEnable`, `DrumSwapSlotLoadLock` (1 = Lock), `ShowTitleToDrumSwapSlot`,
    /// `DrumCaptureStem` (1 = DRUMS Stem, 2 = Original track), `DrumCaptureSaveDirectory`,
    /// `DrumAutomaticallyImport`. Stored (the groove circuit track reads them).
    pub groove_circuit: bool,
    pub drum_swap_lock: bool,
    pub drum_swap_title: bool,
    pub drum_capture_stem: u8,
    pub drum_capture_dir: String,
    pub drum_auto_import: bool,
    // ---- Keyboard
    /// The key mapping in use per mode family (`performaceKeyMapping`): a preset's id or a user
    /// mapping's name. Index 0: Export / Performance / Lighting, 1: Edit.
    pub key_mapping: [String; 2],
    /// User mappings: name -> (command id in hex -> key text as rekordbox writes it).
    pub key_mappings: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
    // ---- Shell
    /// What the header showed in each mode when it was last left (`ShowPanel_*`, layouts).
    pub shell: [ModeShell; 4],
    /// The category and tab last open per mode (`LastOpenedSettingPageNumber*`).
    pub last_page: [(u8, u8); 4],
}

impl Default for RbPrefs {
    fn default() -> Self {
        RbPrefs {
            show_original_key: true,
            draw_rate: 0,
            hide_deck_info: false,
            jog_indicator: 1,
            traffic_light: 4,
            show_sync_rate: true,
            fx_panel_layout: 3,
            zoom_wave_custom: false,
            zoom_wave_h: 1,
            zoom_wave_v: 1,
            pad_mode_custom: false,
            pad_mode_layout: 1,
            color_theme: 1,
            hot_cue_color: 4,
            sampler_color: 1,
            pad_fx_colors: Default::default(),
            mixer_external: false,
            booth_channel: None,
            needle_lock: true,
            memory_cue_call_lock: true,
            load_start_cue: false,
            auto_cue_level: 5,
            auto_cue_deck: [false; 4],
            brake: [0; 2],
            start: [0; 2],
            jog_speed: 1,
            jog_sampler_mode: true,
            auto_play: false,
            beat_sync_type: 1,
            multi_bpm_sync: true,
            smart_cue: false,
            gate_cue: false,
            auto_beat_loop: 1,
            auto_gain: true,
            eq_type: 1,
            crossfader_curve: 16,
            headroom: 4,
            quantize_jump_before: true,
            snap: QRow { on: true, beat: 6 },
            q_hot_cue: QRow { on: true, beat: 4 },
            q_loop: QRow { on: true, beat: 6 },
            q_reverse: QRow { on: true, beat: 6 },
            quantize_in_one_go: false,
            link_start_stop: false,
            link_show_window: false,
            dvs_enable: true,
            dvs_fixed_bpm_scale: false,
            dvs_lead_in: false,
            dvs_hardware: 0,
            itunes_framework: true,
            itunes_grouping_to_label: false,
            itunes_overwrite_sync: true,
            xml_beatgrid: 1,
            relocate_music: true,
            relocate_video: true,
            relocate_desktop: true,
            relocate_user: false,
            auto_export: false,
            cache_track_list: true,
            long_press_menu: true,
            tag_list_on_load: false,
            tag_list_on_exit: false,
            library_protection: false,
            inline_edit_dblclick: false,
            played_time: 60,
            export_name: String::new(),
            deck_quantize: 5,
            loop_wav_rate: 48000,
            link_monitor: false,
            jog_custom_image: false,
            cdj_key_alnum: false,
            part_layout: 0,
            custom_part_layout: 5,
            part_inst_doubles_lr: true,
            part_more_memory: false,
            part_multithread: true,
            groove_circuit: false,
            drum_swap_lock: true,
            drum_swap_title: true,
            drum_capture_stem: 1,
            drum_capture_dir: String::new(),
            drum_auto_import: true,
            key_mapping: [String::new(), String::new()],
            key_mappings: Default::default(),
            shell: Default::default(),
            last_page: [(VIEW, 0); 4],
        }
    }
}

/// (rekordbox's key, its default as an integer, the field's value in rekordbox's encoding).
pub const SPEC: &[(&str, i64, fn(&RbPrefs) -> i64)] = &[
    ("ShowOriginalKey", 1, |p| p.show_original_key as i64),
    ("HideDeckInfo", 0, |p| p.hide_deck_info as i64),
    ("DeckJogIndicator", 1, |p| p.jog_indicator as i64),
    ("TrafficLightRange", 4, |p| p.traffic_light as i64),
    ("showBpmSyncRate", 1, |p| p.show_sync_rate as i64),
    ("FxPanelLayoutType", 3, |p| p.fx_panel_layout as i64),
    ("ZoomWaveHorizontalLayoutType", 1, |p| p.zoom_wave_h as i64),
    ("ZoomWaveVerticalLayoutType", 1, |p| p.zoom_wave_v as i64),
    ("PadModeLayoutType", 1, |p| p.pad_mode_layout as i64),
    ("ColorThemeMode", 1, |p| p.color_theme as i64),
    ("HotCueColorType", 4, |p| p.hot_cue_color as i64),
    ("SamplerColorType", 1, |p| p.sampler_color as i64),
    ("DeckNeedleLock", 1, |p| p.needle_lock as i64),
    ("DeckMemoryCueCallLock", 1, |p| p.memory_cue_call_lock as i64),
    ("DeckLoadingStartPosition", 0, |p| p.load_start_cue as i64),
    ("AutoCueLeveldB", 5, |p| p.auto_cue_level as i64),
    ("AutoCueEnableDeck1", 0, |p| p.auto_cue_deck[0] as i64),
    ("DeckJogSpeed", 1, |p| p.jog_speed as i64),
    ("jogSamplerMode", 1, |p| p.jog_sampler_mode as i64),
    ("AutoPlay", 0, |p| p.auto_play as i64),
    ("BeatSyncType", 1, |p| p.beat_sync_type as i64),
    ("multiBpmSyncMode", 1, |p| p.multi_bpm_sync as i64),
    ("SmartCueEnabled", 0, |p| p.smart_cue as i64),
    ("HotCueSampler", 0, |p| p.gate_cue as i64),
    ("AutoBeatLoop", 1, |p| p.auto_beat_loop as i64),
    ("AutoGain", 1, |p| p.auto_gain as i64),
    ("EqualizerType", 1, |p| p.eq_type as i64),
    ("CrossFaderCurve", 16, |p| p.crossfader_curve as i64),
    ("HeadRoomLevel", 4, |p| p.headroom as i64),
    ("DeckQuantizeMode", 2, |p| if p.quantize_jump_before { 2 } else { 1 }),
    ("InputQuantizeStatus", 2, |p| if p.q_hot_cue.on { 2 } else { 1 }),
    ("InputQuantizeBeat", 6, |p| p.q_loop.beat as i64),
    ("DeckQuantizeInOneGo", 0, |p| p.quantize_in_one_go as i64),
    ("AbletonLinkStartStopSync", 0, |p| p.link_start_stop as i64),
    ("AbletonLinkShowWindow", 0, |p| p.link_show_window as i64),
    ("EnableDVSFunction", 1, |p| p.dvs_enable as i64),
    ("DvsWaveViewBpmScaleMode", 0, |p| p.dvs_fixed_bpm_scale as i64),
    ("DvsVinylLeadInSongHead", 0, |p| p.dvs_lead_in as i64),
    ("bridgeConvertiTunesGrouping", 0, |p| p.itunes_grouping_to_label as i64),
    ("OverwriteSyncList", 1, |p| p.itunes_overwrite_sync as i64),
    ("bridgeExportBeatGrid", 1, |p| p.xml_beatgrid as i64),
    ("AutoRelocateSelectMusicFolder", 1, |p| p.relocate_music as i64),
    ("AutoRelocateSelectMovieFolder", 1, |p| p.relocate_video as i64),
    ("AutoRelocateSelectDesktopFolder", 1, |p| p.relocate_desktop as i64),
    ("AutoRelocateSelectUserFolder", 0, |p| p.relocate_user as i64),
    ("autoReExport", 0, |p| p.auto_export as i64),
    ("CacheTrackList", 1, |p| p.cache_track_list as i64),
    ("ContextMenu", 1, |p| p.long_press_menu as i64),
    ("TaglistLoadOption", 0, |p| p.tag_list_on_load as i64),
    ("TaglistDoNotSaveOption", 0, |p| p.tag_list_on_exit as i64),
    ("EnabledInlineEditWithDblClick", 0, |p| p.inline_edit_dblclick as i64),
    ("playedTime", 60, |p| p.played_time as i64),
    ("DeckQuantize", 5, |p| p.deck_quantize as i64),
    ("useLinkMonitor", 0, |p| p.link_monitor as i64),
    ("PartLayout", 0, |p| p.part_layout as i64),
    ("CustomPartLayout", 5, |p| p.custom_part_layout as i64),
    ("IncreaseMemoryPartAnalysis", 0, |p| p.part_more_memory as i64),
    ("ApplyMultithreadingPartAnalysis", 1, |p| p.part_multithread as i64),
    ("GrooveCircuitEnable", 0, |p| p.groove_circuit as i64),
    ("DrumSwapSlotLoadLock", 1, |p| p.drum_swap_lock as i64),
    ("ShowTitleToDrumSwapSlot", 1, |p| p.drum_swap_title as i64),
    ("DrumCaptureStem", 1, |p| p.drum_capture_stem as i64),
];

/// rekordbox's defaults for settings recordbox already had under its own names, and what
/// recordbox's default is: (key, rekordbox's default, recordbox's default in the same
/// encoding, why they differ when they do).
pub fn existing_defaults(p: &crate::app::Prefs) -> Vec<(&'static str, i64, i64, &'static str)> {
    vec![
        ("showTooltips", 1, p.tooltips as i64, ""),
        ("scrollModeCenter", 0, p.center_selected as i64, ""),
        ("sourceFontStyle", 0, p.browse_bold as i64, ""),
        ("lineSpaceSize", 0, p.browse_line as i64, ""),
        ("WaveClickPlayEnabled", 0, p.wave_click_play_cue as i64, ""),
        ("showpalette", 1, p.palette_enabled as i64, ""),
        ("showPreviewCue", 1, p.cue_markers_on_preview as i64, ""),
        ("showPlaylistAllTracks", 1, p.all_tracks_in_playlists as i64, ""),
        ("showPlaylistTrackNum", 1, p.playlist_track_counts as i64, ""),
        ("showPlaylistSearch", 1, p.playlist_search_filter as i64, ""),
        ("showPhrase", 0, p.phrase_enlarged as i64, ""),
        ("showPhraseFull", 0, p.show_phrase_bar as i64, ""),
        ("showVocalZoom", 0, p.vocal_enlarged as i64, ""),
        ("selectMetronome", 0, p.metronome_sound as i64, ""),
        ("AutoInstantDoubles", 0, p.instant_doubles as i64, ""),
        ("SamplerSlotIs16", 1, p.sampler.slots16 as i64, ""),
        ("SamplerLoadLockAtLoadedSlot", 0, p.sampler.load_lock as i64, ""),
        ("ShouldLoopCaptureIfEmptySlot", 1, p.sampler.capture_empty as i64, ""),
        ("SlicerCaptureSaveBank", 4, p.sampler.slicer_bank as i64, ""),
        ("SequenceLoadAutoPlay", 1, p.sampler.seq.load_autoplay as i64, ""),
        ("SequencerQuantize", 4, p.sampler.seq.quantize as i64, ""),
        ("SequencerQuantizeStatus", 2, if p.sampler.seq.quantize_on { 2 } else { 1 }, ""),
        ("ChFaderCurve", 1, p.ch_fader_curve as i64, ""),
        ("EqualizerMode", 1, if p.eq_isolator { 2 } else { 1 }, ""),
        ("TrackSeparationEnable", 0, p.show_stems as i64, "recordbox offers stems in Export mode too and has no deck-unload step, so the switch can default to on"),
        ("Enable4Stems", 1, (p.stem_parts != 3) as i64, ""),
        ("StemWaveformDisplay", 1, p.stem_waveform as i64, ""),
        ("ActivePartMode", 1, !p.stem_solo as i64, ""),
        ("ResetMuteSoloStatusOnLoaded", 1, p.stem_reset_on_load as i64, ""),
        ("writeMytag", 0, p.mytag_to_comments as i64, ""),
        ("deletePlaylistTracks", 0, p.delete_device_tracks as i64, ""),
        ("importHistory", 1, p.import_device_history as i64, ""),
        ("DeleteHistoryWhenImported", 1, p.delete_history_after_import as i64, "recordbox leaves a device's histories on it unless asked"),
        ("hotCueAutoLoad", 0, p.hot_cue_auto_load_on_import as i64, "recordbox switches Hot Cue Auto Load on for imported tracks so players load their hot cues"),
        ("DeckLoadLock", 1, if p.deck_load_lock { 0 } else { 1 }, ""),
        ("showVocalWhole", 0, p.show_vocal as i64, "recordbox shows the vocal positions it detects"),
        ("showPhraseNameAlways", 0, p.phrase_types_always as i64, "recordbox shows phrase types without hovering"),
        ("SamplerPanelAutoOpen", 0, !p.sampler.auto_panel_disable as i64, "recordbox opens the sampler panel when a pad is played from the keyboard"),
        ("PreviewVolume", 50, (p.preview_volume * 100.0).round() as i64, ""),
    ]
}

impl RbPrefs {
    /// Which rows of the FX panel are shown (Beat FX, Sound Color FX), from Preferences > View
    /// > Layout > Effect Panel.
    pub fn fx_rows(&self) -> (bool, bool) {
        (self.fx_panel_layout != 2, self.fx_panel_layout != 1)
    }

    /// Quantize resolution as beats-per-step divisor (1 = whole beats .. 8 = eighth beats),
    /// from rekordbox's `DeckQuantize` id (3 = 1/8, 4 = 1/4, 5 = 1/2, 6 = 1/1 of a beat... the
    /// ids count from 1/32).
    pub fn quantize_div_of(id: u8) -> u8 {
        match id {
            1 | 2 | 3 => 8,
            4 => 4,
            5 => 2,
            _ => 1,
        }
    }

    /// The id that stands for a divisor.
    pub fn quantize_id_of(div: u8) -> u8 {
        match div {
            8 => 3,
            4 => 4,
            2 => 5,
            _ => 6,
        }
    }
}

/// Saved preferences from before the folder checkboxes searched only the user's listed folders.
pub fn legacy_library_preferences() -> RbPrefs {
    RbPrefs { relocate_music: false, relocate_video: false, relocate_desktop: false, relocate_user: true, ..Default::default() }
}

/// The folders Auto Relocate searches: the ticked standard folders and, when "Specified user
/// folders" is ticked, the listed ones.
pub fn relocate_roots(p: &crate::app::Prefs) -> Vec<std::path::PathBuf> {
    let mut v = vec![];
    if p.rb.relocate_music {
        v.extend(dirs::audio_dir());
    }
    if p.rb.relocate_video {
        v.extend(dirs::video_dir());
    }
    if p.rb.relocate_desktop {
        v.extend(dirs::desktop_dir());
    }
    if p.rb.relocate_user {
        v.extend(p.relocate_folders.iter().map(std::path::PathBuf::from));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preexisting_relocation_folders_keep_their_scope() {
        let json = r#"{"relocate_folders":["/old/collection"]}"#;
        let prefs: crate::app::Prefs = serde_json::from_str(json).unwrap();
        assert_eq!(relocate_roots(&prefs), [std::path::PathBuf::from("/old/collection")]);

        let mut saved = prefs.clone();
        saved.rb.relocate_user = false;
        let saved: crate::app::Prefs = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert!(relocate_roots(&saved).is_empty());
    }

    #[test]
    fn categories_and_tabs_follow_the_mode() {
        let names = |m| categories(m).iter().map(|c| category_name(*c)).collect::<Vec<_>>();
        assert_eq!(names(Mode::Export), ["PLAN", "CLOUD", "View", "Audio", "Analysis", "DJ System", "Keyboard", "Advanced", "Extensions"]);
        assert_eq!(names(Mode::Performance), ["PLAN", "CLOUD", "View", "Audio", "Analysis", "Controller", "Keyboard", "Advanced", "DVS", "Extensions"]);
        assert_eq!(names(Mode::Edit), ["PLAN", "CLOUD", "View", "Audio", "Analysis", "Controller", "Keyboard", "Advanced", "Extensions"]);
        assert!(!categories(Mode::Export).contains(&CONTROLLER));
        assert_eq!(tabs(EXTENSIONS, Mode::Export), ["STEMS"]);
        assert_eq!(tabs(EXTENSIONS, Mode::Edit), ["STEMS"]);
        assert_eq!(tabs(CONTROLLER, Mode::Performance), ["Deck", "Mixer", "Effect", "Sampler", "Recordings", "MIX POINT LINK", "Others"]);
        assert_eq!(tabs(ADVANCED, Mode::Export), ["Database", "Browse", "Recordings", "Others"]);
        assert_eq!(tabs(ADVANCED, Mode::Performance), ["Database", "Browse", "Others"]);
        assert_eq!(tabs(EXTENSIONS, Mode::Performance), ["STEMS", "Video", "Lighting"]);
        assert!(tab_out_of_scope(EXTENSIONS, "Video").is_some() && tab_out_of_scope(EXTENSIONS, "STEMS").is_none());
        for m in [Mode::Export, Mode::Performance, Mode::Lighting, Mode::Edit] {
            for c in categories(m) {
                assert!(!category_name(*c).is_empty());
                assert!(*c >= 100 || !tabs(*c, m).is_empty());
            }
        }
        // The app's mode numbers: 0 EXPORT, 1 PERFORMANCE, 2 EDIT (there is no LIGHTING mode).
        assert_eq!((Mode::of(0), Mode::of(1), Mode::of(2)), (Mode::Export, Mode::Performance, Mode::Edit));
    }

    #[test]
    fn defaults_are_rekordbox_s() {
        let p = RbPrefs::default();
        let mut seen = std::collections::HashSet::new();
        for (key, def, get) in SPEC {
            assert!(seen.insert(*key), "{key} listed twice");
            assert_eq!(get(&p), *def, "{key}");
        }
        // The same against the traced table, where the checkout has it (the specs are not
        // part of the public tree).
        let path = std::env::var("PREPDECK_SPEC_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../checks/re-7.2.19").to_string());
        let Ok(tsv) = std::fs::read_to_string(std::path::Path::new(&path).join("preferences-skins-settings.tsv")) else {
            eprintln!("settings table not found under {path}: compared with the built-in list only");
            return;
        };
        let mut table = std::collections::HashMap::new();
        for line in tsv.lines().skip(1) {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() > 3 {
                // "1", "0xFFFFFFFFLL|4 (literal in getter)", "kXxx != 0 (literal in getter)" ...
                let d = f[3].split(' ').next().unwrap_or("");
                let d = d.rsplit('|').next().unwrap_or(d);
                if let Ok(n) = d.parse::<i64>() {
                    table.insert(f[1].to_string(), n);
                }
            }
        }
        let (mut checked, mut missing) = (0, vec![]);
        for (key, def, _) in SPEC {
            match table.get(*key) {
                Some(n) => {
                    assert_eq!(n, def, "{key}: the traced default is {n}");
                    checked += 1;
                }
                None => missing.push(*key),
            }
        }
        let app = crate::app::Prefs::default();
        for (key, rb, ours, why) in existing_defaults(&app) {
            if let Some(n) = table.get(key) {
                if key != "PreviewVolume" {
                    assert_eq!(*n, rb, "{key}: the traced default is {n}");
                }
            }
            assert!(rb == ours || !why.is_empty(), "{key}: recordbox's default {ours} differs from rekordbox's {rb} without a reason");
        }
        eprintln!("{checked} defaults equal the traced table; no plain integer default traced for {missing:?}");
        assert!(checked >= 45, "only {checked} keys found in the table");
    }

    #[test]
    fn settings_round_trip_and_old_files_load() {
        let mut p = crate::app::Prefs::default();
        p.rb.needle_lock = false;
        p.rb.crossfader_curve = 3;
        p.rb.q_hot_cue = QRow { on: false, beat: 2 };
        p.rb.auto_cue_deck = [true, false, true, false];
        p.rb.pad_fx_colors.insert("ECHO".into(), 7);
        p.rb.export_name = "booth mac".into();
        p.rb.key_mapping = ["0000000000010".into(), String::new()];
        p.rb.key_mappings.entry("mine".into()).or_default().insert("3006".into(), "spacebar".into());
        p.rb.shell[1] = ModeShell { saved: true, fx: true, sampler: false, four_decks: true, player_layout: 0, browser_only: false };
        p.rb.last_page[1] = (CONTROLLER, 6);
        let s = serde_json::to_string(&p).unwrap();
        let q: crate::app::Prefs = serde_json::from_str(&s).unwrap();
        assert_eq!(q.rb, p.rb);
        // A prefs.json written before these settings existed, and one with only some of them.
        let old: crate::app::Prefs = serde_json::from_str(r#"{"key_alnum":true,"app_mode":1,"eq_isolator":true}"#).unwrap();
        // This library predates the folder checkboxes and had no configured search roots.
        assert!(relocate_roots(&old).is_empty());
        assert_eq!(old.rb, RbPrefs {
            relocate_music: false, relocate_video: false, relocate_desktop: false, relocate_user: true,
            ..RbPrefs::default()
        });
        assert!(old.key_alnum && old.eq_isolator && old.app_mode == 1);
        let part: crate::app::Prefs = serde_json::from_str(r#"{"rb":{"needle_lock":false,"snap":{"on":false,"beat":3},"unknown_later_setting":5}}"#).unwrap();
        assert!(!part.rb.needle_lock && !part.rb.snap.on && part.rb.snap.beat == 3);
        assert_eq!(part.rb.crossfader_curve, 16);
        // Reset to defaults puts rekordbox's defaults back and keeps the user's key mappings.
        let r = crate::ui_prefs::reset_prefs(&p);
        assert!(r.rb.needle_lock && r.rb.crossfader_curve == 16 && r.rb.export_name.is_empty());
        assert_eq!(r.rb.key_mappings, p.rb.key_mappings);
        assert_eq!(r.rb.key_mapping, p.rb.key_mapping);
    }

    #[test]
    fn derived_values() {
        let mut p = RbPrefs::default();
        assert_eq!(p.fx_rows(), (true, true));
        p.fx_panel_layout = 1;
        assert_eq!(p.fx_rows(), (true, false));
        p.fx_panel_layout = 2;
        assert_eq!(p.fx_rows(), (false, true));
        for d in [1u8, 2, 4, 8] {
            assert_eq!(RbPrefs::quantize_div_of(RbPrefs::quantize_id_of(d)), d);
        }
        assert_eq!(RbPrefs::quantize_div_of(6), 1);
    }
}
