//! EDIT mode (the track editor): its state on `App`, its settings, and what its controls do.
//! Loading a track or a project, saving, the save prompts, rendering and the import of the
//! rendered file. The screen itself is drawn by `ui_edit_view.rs`; the model, the project
//! file, the audio engine and the renderer are `edit_model.rs`, `edit_project.rs`,
//! `edit_audio.rs` and `edit_render.rs`. Spec: checks/re-7.2.19/edit-mode.md.

use crate::app::*;
use crate::audio::Decoded;
use crate::edit_audio::EDIT;
use crate::edit_model::*;
use crate::edit_project::{self as proj, ProjectData, SongRef};
use crate::edit_render as render;
use crate::model::*;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MODE_EDIT: u8 = 2;

/// recordbox's own copy of rekordbox's EditSettings (never rekordbox's file): kept with the
/// other preferences in recordbox's data folder.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct EditPrefs {
    /// The browser is shown under the editor (rekordbox's ShowEditBrowser).
    pub show_browser: bool,
    pub loop_mode: bool,
    pub show_tool: bool,
    /// 0 = BEAT SELECT, 1 = SELECT.
    pub pad_tab: u8,
    /// "Do not show this popup again." of the render tip shown at the first save.
    pub no_render_tip: bool,
    /// Recently used project files, newest first.
    pub recent: Vec<String>,
    /// The render dialog's choices.
    pub render_show: bool,
    pub render_format: u8,
    pub render_rate: u32,
    pub render_aac_bps: u32,
    pub render_flac_level: u8,
    pub render_dir: String,
}

impl Default for EditPrefs {
    fn default() -> Self {
        EditPrefs { show_browser: true, loop_mode: true, show_tool: true, pad_tab: 0, no_render_tip: false, recent: vec![], render_show: true, render_format: 1, render_rate: 44100, render_aac_bps: 192_000, render_flac_level: 5, render_dir: String::new() }
    }
}

/// The open project: one track and its editing session.
pub struct Project {
    pub session: EditSession,
    pub track: Track,
    pub audio: Arc<Decoded>,
    pub wave: Option<Arc<Waveform>>,
    pub file: Option<PathBuf>,
    /// The song as a project file written by rekordbox named it (its own id and uuid), kept
    /// so that saving the project again does not lose them.
    pub song: Option<SongRef>,
}

impl Project {
    /// The name in the project bar: the file's name, or "New Project" until it is saved.
    pub fn name(&self) -> String {
        match &self.file {
            Some(f) => f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            None => "New Project".into(),
        }
    }
    pub fn status(&self) -> u8 {
        proj::status(true, self.file.is_some(), self.session.modified())
    }
}

/// Something that closes or replaces the open project, and so asks about unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Load(i64),
    OpenFile(PathBuf),
    OpenDialog,
    /// "Create new project" and the artwork's eject button: back to "Not Loaded.".
    Close,
    SwitchMode(u8),
    Quit,
}

pub struct RenderDlg {
    pub track: String,
    pub artist: String,
}

pub enum Modal {
    /// "Save changes made to the project?" Yes / No / Cancel.
    SavePrompt(Pending),
    Alert { title: String, text: String },
    /// A project written by a newer version: OK / Cancel.
    ConfirmNewer(PathBuf, Box<ProjectData>),
    /// The one-time tip before the first save; then the save goes on.
    RenderTip { save_as: bool, then: Option<Pending>, dont_show: bool },
    Render(RenderDlg),
    Rendering,
    /// "Are you sure you want to stop playing and switch to EDIT mode?"
    ConfirmStop,
}

type Loading = (i64, crossbeam_channel::Receiver<Result<Arc<Decoded>, String>>, Option<(PathBuf, ProjectData)>);

/// A pointer gesture in progress on the enlarged waveform or the palette.
#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    None,
    /// Selecting: the beat under the press, where it was pressed, and the range selected at
    /// that moment when the press was inside it (it can then be dragged to the palette).
    Select { anchor: f64, from: egui::Pos2, inside: Option<(f64, f64)>, moved: bool },
    /// The selected range being dragged out of the waveform.
    DragSelection,
    /// A palette entry being dragged.
    DragPalette(u32),
    /// The bar ruler being dragged (view start at the press, press x).
    Scroll { start: f64, x: f32 },
}

pub struct EditState {
    pub project: Option<Project>,
    pub loading: Option<Loading>,
    /// Zoom step 1..=15 of the enlarged waveform and the beat at its left edge.
    pub zoom: usize,
    pub view_start: f64,
    pub gesture: Gesture,
    pub modal: Option<Modal>,
    /// The palette's delete buttons are showing; the entry being renamed.
    pub palette_delete: bool,
    pub renaming: Option<(u32, String)>,
    pub zoom_ctrl_open: bool,
    pub job: Option<(render::Job, RenderDlg, bool)>,
    /// The window may close without asking (the save prompt has been answered).
    pub quit_ok: bool,
    /// Where things were last drawn, for the scripted pointer run.
    pub map: crate::ui_edit_view::Map,
    /// Commands from the menu bar and the keys, run at the start of the next frame.
    pub queue: Vec<Cmd>,
    pub(crate) menu: Option<MenuItems>,
    /// Last state pushed to the menu items.
    menu_shown: Vec<bool>,
    /// Tracks made by a render (their `edit_of_<id>` link), read once and kept current.
    pub edited: Option<std::collections::HashSet<i64>>,
    /// The Attribute column's filter list: None = "Select all"; else (Edited Tracks, No
    /// Attribute) ticked.
    pub attr_filter: Option<(bool, bool)>,
    pub attr_popup: bool,
}

impl Default for EditState {
    fn default() -> Self {
        EditState { project: None, loading: None, zoom: ZOOM_DEFAULT, view_start: 0.0, gesture: Gesture::None, modal: None, palette_delete: false, renaming: None, zoom_ctrl_open: true, job: None, quit_ok: false, map: Default::default(), queue: vec![], menu: None, menu_shown: vec![], edited: None, attr_filter: None, attr_popup: false }
    }
}

/// Everything a control, a key or a menu item can ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Edit(EditCmd),
    Select(SelectCmd),
    AddToPalette,
    PlayPause,
    Head,
    Loop,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Browser,
    NewProject,
    OpenProject,
    OpenRecent(PathBuf),
    Save,
    SaveAs,
    Render,
    Unload,
}

pub fn is_edit(app: &App) -> bool {
    app.prefs.app_mode == MODE_EDIT
}

// ---------------------------------------------------------------------------- units and view

impl EditState {
    /// The quantize unit in beats right now.
    pub fn unit(&self) -> u32 {
        self.project.as_ref().map(|p| p.session.quantize.unit(self.zoom)).unwrap_or(1)
    }
    /// Points per beat of the enlarged waveform: the zoom table's number is waveform frames
    /// (1/150 s) per point.
    pub fn pt_per_beat(&self) -> f32 {
        let bpm = self.project.as_ref().map(|p| p.session.cur.bpm).unwrap_or(120.0);
        (150.0 / ZOOM_TABLE[self.zoom.clamp(1, 15)] as f64 * 60.0 / bpm) as f32
    }
    /// Change the zoom step keeping the beat at `anchor_x` points from the left where it is.
    pub fn set_zoom(&mut self, zoom: usize, anchor_x: f32) {
        let zoom = zoom.clamp(1, 15);
        if zoom == self.zoom {
            return;
        }
        let beat = self.view_start + (anchor_x / self.pt_per_beat()) as f64;
        self.zoom = zoom;
        self.view_start = beat - (anchor_x / self.pt_per_beat()) as f64;
    }
}

/// The play position on the timeline, beats.
pub fn play_pos() -> f64 {
    EDIT.lock().main.pos
}

pub fn playing() -> bool {
    EDIT.lock().main.playing
}

// ---------------------------------------------------------------------------- loading

/// rekordbox's load checks, in its order. Err: (alert, message); a plain message goes to the
/// status bar.
pub fn load_check(t: &Track, has_grid: bool) -> Result<(), (bool, &'static str)> {
    if t.missing || !Path::new(&t.path).is_file() {
        return Err((false, "Load error. The file cannot be found."));
    }
    if t.analyzed != 1 {
        return Err((true, "Tracks that have not been analyzed cannot be edited.\nRight click the track and select [Analyze Track] from the context menu."));
    }
    if !has_grid {
        return Err((true, "The tracks without grids cannot be edited. \nInput [BPM] at the GRID panel of EXPORT mode or PERFORMANCE mode."));
    }
    if !(t.bpm >= BPM_MIN && t.bpm <= BPM_MAX) {
        return Err((true, "Tracks with a BPM value out of the 40 to 499 range cannot be edited.\nChange the track list [BPM] to a value within the 40 to 499 range."));
    }
    Ok(())
}

/// The score of a freshly loaded track: its grid stretched over the file, its cues in beats.
pub fn new_score(t: &Track, grid: &BeatGrid, cues: &[Cue], duration: f64) -> ScoreSet {
    let beats = grid.beats(duration);
    let org: Vec<(f64, f64)> = beats.iter().map(|b| (b.bpm, b.time * 1000.0)).collect();
    // The analysed beat k is a bar line when its number in the bar is 1.
    let off = beats.first().map(|b| (5 - b.beat.clamp(1, 4) as i32) % 4).unwrap_or(0);
    let g = SongGrid::new(org, off, (duration + 1.0) * 1000.0, t.bpm);
    let cue = |c: &Cue, id: i32| CueLoop {
        cue_id: id,
        pos_in: g.ms_to_ppq(c.start * 1000.0),
        pos_out: c.end.filter(|e| *e > c.start).map(|e| g.ms_to_ppq(e * 1000.0)).unwrap_or(NOT_LOOP),
        appended: CueAppended { db_cue_id: c.id, status: i32::from(c.active_loop), color: c.color, comment: c.comment.clone(), loop_numer: 0, loop_denom: 0, color_table: 0 },
    };
    let prepared = Prepared {
        hot: cues.iter().filter(|c| c.kind == CueKind::Hot).map(|c| cue(c, c.slot)).collect(),
        memory: cues.iter().filter(|c| c.kind == CueKind::Memory).enumerate().map(|(i, c)| cue(c, i as i32)).collect(),
        censor: vec![],
    };
    ScoreSet::new(t.id, t.bpm, g, prepared)
}

fn alert(app: &mut App, title: &str, text: impl Into<String>) {
    app.edit.modal = Some(Modal::Alert { title: title.into(), text: text.into() });
}

fn decode_in_background(app: &App, path: String) -> crossbeam_channel::Receiver<Result<Arc<Decoded>, String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = app.ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(crate::audio::decode_file(&path).map(Arc::new).map_err(|e| e.to_string()));
        ctx.request_repaint();
    });
    rx
}

/// Load a collection track into the editor (after the save prompt, when there are changes).
pub fn load_track(app: &mut App, id: i64) {
    request(app, Pending::Load(id));
}

fn start_load(app: &mut App, id: i64, project: Option<(PathBuf, ProjectData)>) {
    if app.deck_options.disable_load {
        return app.toast_err("Deck loading is disabled for this session (--disable-load).");
    }
    let Some(t) = app.track(id).cloned() else { return };
    let has_grid = app.db.grid(id).map(|g| !g.is_empty()).unwrap_or(false) || project.is_some();
    if let Err((is_alert, msg)) = load_check(&t, has_grid) {
        if project.is_some() {
            alert(app, "Project load error", format!("{msg}\n\nFile Name: {}\nTitle: {}\nArtist: {}\nAlbum: {}", t.file_name(), t.display_title(), t.artist, t.album));
        } else if is_alert {
            alert(app, "Load error", msg);
        } else {
            app.toast_err(msg);
        }
        return;
    }
    app.edit.loading = Some((id, decode_in_background(app, t.path.clone()), project));
}

fn finish_load(app: &mut App, id: i64, audio: Arc<Decoded>, project: Option<(PathBuf, ProjectData)>) {
    let Some(t) = app.track(id).cloned() else { return };
    unload(app);
    let (session, file, song) = match project {
        Some((path, mut data)) => {
            // The file names the song by the id of the library that wrote it; here it is
            // this library's track.
            let song = data.song.clone();
            data.score.track_id = id;
            for p in data.palettes.iter_mut() {
                Arc::make_mut(&mut p.clip).track_id = id;
            }
            let mut s = EditSession::new(data.score);
            s.palettes = data.palettes;
            s.quantize = data.quantize;
            proj::touch_recent(&mut app.prefs.edit.recent, &path);
            (s, Some(path), Some(song))
        }
        None => {
            let grid = app.db.grid(id).unwrap_or_default();
            let cues = app.db.cues(id).unwrap_or_default();
            (EditSession::new(new_score(&t, &grid, &cues, audio.duration())), None, None)
        }
    };
    if !session.cur.is_valid() {
        alert(app, "Load error", "Load error. The file cannot be edited.");
        return;
    }
    let too_long = session.cur.len_ms() >= RENDER_MAX_MS;
    {
        let mut e = EDIT.lock();
        e.main.audio = Some(audio.clone());
        e.main.score = Some(session.cur.clone());
        e.main.keep_pitch = session.cur.song_grid.is_dynamic();
        e.main.pos = 0.0;
        e.main.playing = false;
        e.main.loop_on = app.prefs.edit.loop_mode;
        e.main.loop_range = None;
        for v in e.pals.iter_mut() {
            v.audio = Some(audio.clone());
        }
        e.gain = if t.gain_db.is_finite() { 10f32.powf(t.gain_db / 20.0).min(4.0) } else { 1.0 };
    }
    app.edit.project = Some(Project { session, wave: app.db.wave(id).map(Arc::new), track: t, audio, file, song });
    app.edit.zoom = ZOOM_DEFAULT;
    app.edit.view_start = 0.0;
    app.edit.gesture = Gesture::None;
    app.edit.palette_delete = false;
    app.edit.renaming = None;
    app.save_prefs();
    if too_long {
        app.toast(render::MSG_TOO_LONG);
    }
}

/// Close the project: nothing loaded, nothing playing, undo history and clipboard gone.
pub fn unload(app: &mut App) {
    let freed = {
        let mut e = EDIT.lock();
        e.pal_ids = [0; crate::edit_audio::PAL_VOICES];
        let pals: Vec<_> = e.pals.iter_mut().map(|v| v.clear()).collect();
        (e.main.clear(), pals)
    };
    drop(freed);
    app.edit.project = None;
    app.edit.gesture = Gesture::None;
    app.edit.renaming = None;
    app.edit.palette_delete = false;
}

fn poll_loading(app: &mut App) {
    let Some((id, rx, _)) = &app.edit.loading else { return };
    let id = *id;
    if let Ok(r) = rx.try_recv() {
        let (_, _, project) = app.edit.loading.take().unwrap();
        match r {
            Ok(a) => finish_load(app, id, a, project),
            Err(e) => app.toast_err(format!("Load error. The file cannot be edited. ({e})")),
        }
    }
}

// ---------------------------------------------------------------------------- projects

fn project_data(p: &Project) -> ProjectData {
    let song = match &p.song {
        // (Saved under this library's track id: the reader matches the file path first.)
        Some(s) => SongRef { id: p.track.id, ..s.clone() },
        None => SongRef { id: p.track.id, uuid: String::new(), title: p.track.display_title(), artist: p.track.artist.clone(), album: p.track.album.clone(), filepath: p.track.path.clone() },
    };
    ProjectData { version: proj::FILE_VERSION, song, score: (*p.session.cur).clone(), palettes: p.session.palettes.clone(), quantize: p.session.quantize }
}

/// The save panel (or, in a scripted run, the file named by PREPDECK_EDIT_SAVE).
fn choose_save_file(p: &Project, save_as: bool) -> Option<PathBuf> {
    if let Ok(f) = std::env::var("PREPDECK_EDIT_SAVE") {
        return Some(PathBuf::from(f));
    }
    let dir = proj::project_dir();
    let _ = std::fs::create_dir_all(&dir);
    let name = proj::default_file(&dir, &p.track.display_title());
    rfd::FileDialog::new()
        .set_title(if save_as { "Save project file as" } else { "Save project file" })
        .add_filter("rekordbox project", &[proj::EXT])
        .set_directory(&dir)
        .set_file_name(name.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())
        .save_file()
        .map(|f| if f.extension().is_none() { f.with_extension(proj::EXT) } else { f })
}

/// Save the project; `then` runs afterwards when it worked. Returns whether it was saved
/// now (false also when a tip or a panel is in the way or was cancelled).
pub fn save_project(app: &mut App, save_as: bool, then: Option<Pending>) -> bool {
    let Some(p) = &app.edit.project else { return false };
    let needs_file = p.file.is_none() || save_as;
    if p.file.is_none() && !app.prefs.edit.no_render_tip && !matches!(app.edit.modal, Some(Modal::RenderTip { .. })) {
        app.edit.modal = Some(Modal::RenderTip { save_as, then, dont_show: false });
        return false;
    }
    let path = if needs_file {
        match choose_save_file(p, save_as) {
            Some(f) => f,
            None => return false,
        }
    } else {
        p.file.clone().unwrap()
    };
    match proj::save(&path, &project_data(p)) {
        Ok(()) => {
            let p = app.edit.project.as_mut().unwrap();
            p.file = Some(path.clone());
            p.session.mark_saved();
            proj::touch_recent(&mut app.prefs.edit.recent, &path);
            app.save_prefs();
            if let Some(next) = then {
                run_pending(app, next);
            }
            true
        }
        Err(e) => {
            alert(app, "Save project", e.message());
            false
        }
    }
}

fn open_file(app: &mut App, path: &Path) {
    match proj::load(path) {
        Err(e) => {
            proj::prune_recent(&mut app.prefs.edit.recent);
            alert(app, "Project load error", e.message());
        }
        Ok(data) if data.version > proj::FILE_VERSION => app.edit.modal = Some(Modal::ConfirmNewer(path.to_path_buf(), Box::new(data))),
        Ok(data) => open_data(app, path.to_path_buf(), data),
    }
}

/// Find the project's track in the collection (by file path, else by id and title) and load it.
pub fn open_data(app: &mut App, path: PathBuf, data: ProjectData) {
    let s = &data.song;
    let found = app.tracks.iter().find(|t| t.path == s.filepath).or_else(|| app.tracks.iter().find(|t| t.id == s.id && t.display_title() == s.title)).map(|t| t.id);
    match found {
        Some(id) => start_load(app, id, Some((path, data))),
        None => {
            let file = Path::new(&s.filepath).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let text = format!(
                "The following track has been deleted from the library.\n\nFile Name: {file}\nTitle: {}\nArtist: {}\nAlbum: {}\n\n*If the library has been backed up, you can return the library to the state before backup from [File] menu > [Library] > [Restore Library].",
                s.title, s.artist, s.album
            );
            alert(app, "Project load error", text);
        }
    }
}

// ---------------------------------------------------------------------------- save prompts

/// Do `what`, asking "Save changes made to the project?" first when there are changes.
pub fn request(app: &mut App, what: Pending) {
    if app.edit.project.as_ref().map(|p| p.session.modified()).unwrap_or(false) {
        app.edit.modal = Some(Modal::SavePrompt(what));
    } else {
        run_pending(app, what);
    }
}

pub fn run_pending(app: &mut App, what: Pending) {
    match what {
        Pending::Load(id) => start_load(app, id, None),
        Pending::OpenFile(f) => open_file(app, &f),
        Pending::OpenDialog => {
            let dir = proj::project_dir();
            let mut d = rfd::FileDialog::new().set_title("Select project file").add_filter("rekordbox project", &[proj::EXT]);
            if dir.is_dir() {
                d = d.set_directory(&dir);
            }
            if let Some(f) = d.pick_file() {
                open_file(app, &f);
            }
        }
        Pending::Close => unload(app),
        Pending::SwitchMode(m) => {
            unload(app);
            crate::midi::change_mode(app, m.min(MODE_EDIT));
            app.poll_decks();
            app.save_prefs();
        }
        Pending::Quit => {
            app.edit.quit_ok = true;
            app.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

// ---------------------------------------------------------------------------- hooks

/// `App::load_track`: in EDIT mode a track loads into the editor, not a deck.
pub fn intercept_load(app: &mut App, id: i64) -> bool {
    if !is_edit(app) {
        return false;
    }
    // (PREPDECK_SHOT_VIEW=edit is the screen with nothing loaded.)
    if app.screenshot.is_some() && std::env::var("PREPDECK_SHOT_VIEW").map(|v| v.trim_start_matches("light,").to_string()).map(|v| v == "edit" || v == "edittools" || v == "editpads").unwrap_or(false) {
        return true;
    }
    load_track(app, id);
    true
}

/// `App::set_app_mode`: leaving EDIT asks about unsaved changes; entering it stops the decks
/// (after asking, when one is playing).
pub fn intercept_mode(app: &mut App, mode: u8) -> bool {
    if is_edit(app) && mode != MODE_EDIT {
        EDIT.lock().main.playing = false;
        request(app, Pending::SwitchMode(mode));
        return true;
    }
    if mode == MODE_EDIT && !is_edit(app) {
        if app.player.decks.iter().any(|d| d.lock().playing) {
            app.edit.modal = Some(Modal::ConfirmStop);
        } else {
            enter(app);
        }
        return true;
    }
    false
}

pub fn enter(app: &mut App) {
    crate::midi::change_mode(app, MODE_EDIT);
    for d in &app.player.decks {
        d.lock().playing = false;
    }
    app.poll_decks();
    app.save_prefs();
}

/// The screenshot switches: PREPDECK_SHOT_VIEW=edit (nothing loaded, browser and palette
/// shown), edittools (palette collapsed), editpads (browser hidden), editload (a track
/// loaded: the first analysed one, or PREPDECK_SHOT_TRACK).
pub fn shot_view(app: &mut App, name: &str) -> bool {
    let (browser, tool) = match name {
        "edit" | "editload" => (true, true),
        "edittools" => (true, false),
        "editpads" | "editloadpads" => (false, false),
        _ => return false,
    };
    app.prefs.app_mode = MODE_EDIT;
    app.prefs.edit.show_browser = browser;
    app.prefs.edit.show_tool = tool;
    true
}

/// Poll editor jobs and queued menu actions once per application frame. Only the main
/// window handles a request to quit; closing the detached browser reattaches it.
pub fn frame(app: &mut App, ctx: &egui::Context) {
    app.edit.map.rects.clear();
    poll_loading(app);
    poll_render(app);
    sync_menu(app);
    if !is_edit(app) {
        return;
    }
    // The master volume and mute of the header apply to the editor as to a deck.
    {
        let (v, m) = {
            let s = app.player.state.lock();
            (s.volume, s.muted)
        };
        let mut e = EDIT.lock();
        e.volume = v;
        e.muted = m;
    }
    // The window's close button: ask about unsaved changes first.
    if ctx.input(|i| i.viewport().close_requested()) && !app.edit.quit_ok && app.edit.project.as_ref().map(|p| p.session.modified()).unwrap_or(false) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        app.edit.modal = Some(Modal::SavePrompt(Pending::Quit));
    }
    for c in std::mem::take(&mut app.edit.queue) {
        run(app, c);
    }
    if playing() { app.repaint_playback(ctx); }
}

/// EDIT mode's keys: those of the mapping chosen in Preferences > Keyboard (rekordbox's
/// "Edit Mode (Preset)" unless a mapping of one's own is chosen; keycmd.rs). The command key
/// with C, X and V also reaches the app through the menu bar's Cut / Copy / Paste items
/// (`menu_command`).
pub(crate) fn keys(app: &mut App, ctx: &egui::Context) {
    let mut out = crate::keycmd::edit_keys(app, ctx);
    ctx.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::Copy => out.push(Cmd::Edit(EditCmd::Copy)),
                egui::Event::Cut => out.push(Cmd::Edit(EditCmd::Cut)),
                egui::Event::Paste(_) => out.push(Cmd::Edit(EditCmd::Insert)),
                _ => {}
            }
        }
    });
    out.dedup();
    for command in out { run(app, command); }
}

/// `menus::handle`: menu items that mean something else in EDIT mode, and the editor's own.
pub fn menu_command(app: &mut App, id: &str) -> bool {
    if id == "ed_quit" {
        request(app, Pending::Quit);
        return true;
    }
    if let Some(i) = id.strip_prefix("ed_recent_") {
        if let Some(f) = i.parse::<usize>().ok().and_then(|i| app.prefs.edit.recent.get(i).cloned()) {
            app.edit.queue.push(Cmd::OpenRecent(PathBuf::from(f)));
            if !is_edit(app) {
                app.set_app_mode(MODE_EDIT);
            }
        }
        return true;
    }
    if !is_edit(app) {
        return false;
    }
    let typing = app.ctx.wants_keyboard_input() || app.edit.modal.is_some();
    let c = match id {
        // The Edit menu's own items act on text while a field has the keyboard.
        "edit_cut" if !typing => Cmd::Edit(EditCmd::Cut),
        "edit_copy" if !typing => Cmd::Edit(EditCmd::Copy),
        // rekordbox's quirk: Cmd+V inserts, Cmd+P pastes over.
        "edit_paste" if !typing => Cmd::Edit(EditCmd::Insert),
        "undo" if !typing => Cmd::Edit(EditCmd::Undo),
        "redo" if !typing => Cmd::Edit(EditCmd::Redo),
        "ed_open" => Cmd::OpenProject,
        "ed_save" => Cmd::Save,
        "ed_save_as" => Cmd::SaveAs,
        "ed_render" => Cmd::Render,
        "ed_clone" => Cmd::Edit(EditCmd::Clone),
        "ed_paste" => Cmd::Edit(EditCmd::Paste),
        "ed_delete" => Cmd::Edit(EditCmd::Delete),
        "ed_clear" => Cmd::Edit(EditCmd::Clear),
        _ => return false,
    };
    app.edit.queue.push(c);
    app.ctx.request_repaint();
    true
}

// ---------------------------------------------------------------------------- commands

/// Whether a control is live right now (pads and menu items grey out otherwise).
pub fn can(app: &App, c: &Cmd) -> bool {
    let p = app.edit.project.as_ref();
    match c {
        Cmd::Edit(e) => p.map(|p| p.session.can_edit(*e)).unwrap_or(false),
        Cmd::Select(s) => p.map(|p| p.session.can_select(*s, app.edit.unit())).unwrap_or(false),
        Cmd::AddToPalette => p.map(|p| p.session.selection.range().is_some()).unwrap_or(false),
        Cmd::PlayPause => p.is_some() && !app.deck_options.disable_play,
        Cmd::Head | Cmd::ZoomIn | Cmd::ZoomOut | Cmd::ZoomReset | Cmd::Unload => p.is_some(),
        Cmd::NewProject | Cmd::Save | Cmd::SaveAs | Cmd::Render => p.is_some(),
        Cmd::Loop | Cmd::Browser | Cmd::OpenProject | Cmd::OpenRecent(_) => true,
    }
}

/// After the score or the selection changed: the engine gets the score and the loop range.
pub fn sync_engine(app: &mut App) {
    let Some(p) = &app.edit.project else { return };
    let cur = p.session.cur.clone();
    let range = p.session.selection.range();
    let old = {
        let mut e = EDIT.lock();
        let m = &mut e.main;
        let old = if m.score.as_ref().map(|s| !Arc::ptr_eq(s, &cur)).unwrap_or(true) { m.score.replace(cur.clone()) } else { None };
        m.loop_on = app.prefs.edit.loop_mode;
        if m.loop_range != range {
            m.loop_range = range;
            // A new loop range while playing outside it: playback goes to its start.
            if let (true, true, Some((a, b))) = (m.loop_on, m.playing, range) {
                if m.pos < a || m.pos >= b {
                    m.pos = a;
                }
            }
        }
        m.pos = m.pos.clamp(0.0, cur.len());
        old
    };
    drop(old);
}

pub fn run(app: &mut App, c: Cmd) {
    if !can(app, &c) {
        return;
    }
    let unit = app.edit.unit();
    match c {
        Cmd::Edit(e) => {
            let p = app.edit.project.as_mut().unwrap();
            if p.session.edit(e) {
                sync_engine(app);
            }
        }
        Cmd::Select(s) => {
            let p = app.edit.project.as_mut().unwrap();
            if p.session.select(s, unit) {
                sync_engine(app);
            }
        }
        Cmd::AddToPalette => add_to_palette(app),
        Cmd::PlayPause => {
            let p = app.edit.project.as_ref().unwrap();
            let (len, start) = (p.session.cur.len(), p.session.selection.point().unwrap_or(0.0));
            let mut e = EDIT.lock();
            if e.main.playing {
                e.main.playing = false;
            } else {
                // At the end of the timeline, play starts again from the selected point.
                if e.main.pos >= len {
                    e.main.pos = if start < len { start } else { 0.0 };
                }
                e.main.playing = true;
            }
        }
        Cmd::Head => {
            EDIT.lock().main.pos = 0.0;
            app.edit.view_start = 0.0;
        }
        Cmd::Loop => {
            app.prefs.edit.loop_mode = !app.prefs.edit.loop_mode;
            app.save_prefs();
            sync_engine(app);
            EDIT.lock().main.loop_on = app.prefs.edit.loop_mode;
        }
        Cmd::ZoomIn | Cmd::ZoomOut | Cmd::ZoomReset => {
            // Zoom holds the select point where it is when it is on screen, else the middle.
            let w = app.edit.map.wave.width().max(1.0);
            let ppb = app.edit.pt_per_beat();
            let sel = app.edit.project.as_ref().and_then(|p| p.session.selection.point()).map(|b| ((b - app.edit.view_start) as f32) * ppb).filter(|x| *x >= 0.0 && *x <= w);
            let z = match c {
                Cmd::ZoomIn => app.edit.zoom.saturating_sub(1),
                Cmd::ZoomOut => app.edit.zoom + 1,
                _ => ZOOM_DEFAULT,
            };
            app.edit.set_zoom(z, sel.unwrap_or(w / 2.0));
        }
        Cmd::Browser => {
            app.prefs.edit.show_browser = !app.prefs.edit.show_browser;
            app.save_prefs();
        }
        Cmd::NewProject | Cmd::Unload => request(app, Pending::Close),
        Cmd::OpenProject => request(app, Pending::OpenDialog),
        Cmd::OpenRecent(f) => request(app, Pending::OpenFile(f)),
        Cmd::Save => {
            save_project(app, false, None);
        }
        Cmd::SaveAs => {
            save_project(app, true, None);
        }
        Cmd::Render => open_render(app),
    }
}

/// The selected range as a palette entry, named after the track with its bar position.
pub fn add_to_palette(app: &mut App) {
    let Some(p) = app.edit.project.as_mut() else { return };
    let Some((a, _)) = p.session.selection.range() else { return };
    let (bar, beat) = p.session.cur.bar_beat(a as i64);
    let name = format!("{} {bar}.{beat}", p.track.display_title());
    if let Err(msg) = p.session.add_to_palette(&name) {
        if !msg.is_empty() {
            app.toast_err(msg);
        }
    }
}

/// Click on the enlarged or the whole waveform: the play position goes there.
pub fn seek(app: &mut App, beat: f64) {
    let Some(p) = &app.edit.project else { return };
    EDIT.lock().main.pos = beat.clamp(0.0, p.session.cur.len());
}

/// Play or stop a palette entry from `from` beats into it.
pub fn palette_play(app: &mut App, id: u32, from: Option<f64>) {
    if app.deck_options.disable_play { return; }
    let Some(p) = &app.edit.project else { return };
    let Some(clip) = p.session.palettes.iter().find(|c| c.id == id).map(|c| c.clip.clone()) else { return };
    let old = {
        let mut e = EDIT.lock();
        if e.pal_playing(id) && from.is_none() {
            if let Some(k) = e.pal_of(id) {
                e.pals[k].playing = false;
            }
            None
        } else {
            // Each palette clip has a player of its own: others keep sounding.
            let k = e.pal_slot(id);
            let v = &mut e.pals[k];
            let old = v.score.replace(clip.clone());
            v.keep_pitch = clip.song_grid.is_dynamic();
            v.audio = Some(p.audio.clone());
            v.pos = from.unwrap_or(0.0).clamp(0.0, clip.len());
            v.playing = true;
            old
        }
    };
    drop(old);
}

pub fn palette_remove(app: &mut App, id: u32) {
    let old = {
        let mut e = EDIT.lock();
        match e.pal_of(id) {
            Some(k) => {
                e.pals[k].playing = false;
                e.pal_ids[k] = 0;
                e.pals[k].score.take()
            }
            None => None,
        }
    };
    drop(old);
    if let Some(p) = app.edit.project.as_mut() {
        p.session.remove_palette(id);
        if p.session.palettes.is_empty() {
            app.edit.palette_delete = false;
        }
    }
}

// ---------------------------------------------------------------------------- render

/// Where rendered files go until the user picks a folder: recordbox's own folder in Music
/// (rekordbox's default folder was not traced).
pub fn render_dir(app: &App) -> PathBuf {
    if !app.prefs.edit.render_dir.is_empty() {
        return PathBuf::from(&app.prefs.edit.render_dir);
    }
    if std::env::var("PREPDECK_DATA").is_ok() {
        return crate::db::data_dir().join("Edit Rendered");
    }
    dirs::audio_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("recordbox").join("Edit Rendered")
}

pub fn open_render(app: &mut App) {
    let Some(p) = &app.edit.project else { return };
    if p.session.cur.len_ms() >= RENDER_MAX_MS {
        alert(app, "Render Audio", render::MSG_TOO_LONG);
        return;
    }
    let track = match &p.file {
        Some(_) => p.name(),
        None => p.track.display_title(),
    };
    app.edit.modal = Some(Modal::Render(RenderDlg { track, artist: p.track.artist.clone() }));
}

/// The dialog's Render button.
pub fn start_render(app: &mut App, dlg: RenderDlg) {
    let Some(p) = &app.edit.project else { return };
    let e = &app.prefs.edit;
    let opt = render::Options { format: e.render_format.min(3), rate: e.render_rate, aac_bps: e.render_aac_bps, flac_level: e.render_flac_level.min(8) };
    let dir = render_dir(app);
    if app.prefs.edit.render_dir.is_empty() {
        let _ = std::fs::create_dir_all(&dir);
    }
    // An audio file that is already there is never written over: the name gets a number.
    let base = proj::sanitized(&dlg.track);
    let mut path = dir.join(format!("{base}.{}", opt.extension()));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{base} ({n}).{}", opt.extension()));
        n += 1;
    }
    EDIT.lock().main.playing = false;
    let show = app.prefs.edit.render_show;
    match render::start(p.audio.clone(), p.session.cur.clone(), path, opt, Some(app.ctx.clone())) {
        Ok(job) => {
            app.edit.job = Some((job, dlg, show));
            app.edit.modal = Some(Modal::Rendering);
        }
        Err(msg) => alert(app, "Render Audio", msg),
    }
}

fn poll_render(app: &mut App) {
    let Some((job, _, _)) = &app.edit.job else { return };
    let Ok(r) = job.rx.try_recv() else { return };
    let (_, dlg, show) = app.edit.job.take().unwrap();
    if matches!(app.edit.modal, Some(Modal::Rendering)) {
        app.edit.modal = None;
    }
    match r {
        Ok(path) => match import_render(app, &path, &dlg, show) {
            Some(_) => app.toast(format!("Rendered \"{}\" and added it to the collection.", path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())),
            None => app.toast_err("The rendered file could not be added to the collection."),
        },
        // (An empty message: cancelled.)
        Err(msg) if msg.is_empty() => {}
        Err(msg) => alert(app, "Render Audio", msg),
    }
}

// ------------------------------------------------------------ the browser's EDIT attribute
//
// rekordbox marks a rendered track as edited (`DatabaseIF::setTrackEdited`): the browser's
// Attribute cell shows `edit/attribute.svg` (`ListViewer::paintAttributeColumn` 0x1003f07b0)
// with the tooltip "Tracks created in EDIT mode." (`getAttributeCellTooltip` 0x1003ebc2c),
// and the Attribute column's filter list (`AttributeFilterList`, `db::AttributeFilter::
// execFilter` 0x1014d8380, opened from the column's header) has "Edited Tracks" beside
// "Select all", the cloud items, "LINK file", "Karaoke file" and "No Attribute". The same cell
// shows `sampler.svg` / `samplerLoop.svg` for a track whose Track Type is Oneshot / Loop.
// recordbox lists the items that can apply to its library: Select all, Edited Tracks, No
// Attribute (no cloud, no video LINK or karaoke files).

/// The rendered tracks (read from the settings the first time).
pub fn edited_set(app: &mut App) -> &std::collections::HashSet<i64> {
    if app.edit.edited.is_none() {
        let mut set = std::collections::HashSet::new();
        // (Stem exports, `stem_of_<id>`, count as tracks recordbox made from another: stem_export.rs.)
        if let Ok(mut st) = app.db.conn.prepare("SELECT key FROM settings WHERE substr(key, 1, 8) IN ('edit_of_', 'stem_of_')") {
            if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                for k in rows.flatten() {
                    if let Some(id) = k.strip_prefix("edit_of_").or_else(|| k.strip_prefix("stem_of_")).and_then(|v| v.parse::<i64>().ok()) {
                        set.insert(id);
                    }
                }
            }
        }
        app.edit.edited = Some(set);
    }
    app.edit.edited.as_ref().unwrap()
}

/// The Attribute filter lets this track through (call `edited_set` once before filtering).
pub fn attribute_pass(app: &App, id: i64) -> bool {
    let Some((edited, none)) = app.edit.attr_filter else { return true };
    let is_edited = app.edit.edited.as_ref().map(|s| s.contains(&id)).unwrap_or(false);
    attr_ok(Some((edited, none)), is_edited, app.cue_tools.has_type(id))
}

/// The filter list's rule: a ticked item lets its tracks through; "No Attribute" is a track
/// with no attribute icon at all.
pub fn attr_ok(filter: Option<(bool, bool)>, is_edited: bool, has_type: bool) -> bool {
    match filter {
        None => true,
        Some((edited, none)) => (edited && is_edited) || (none && !is_edited && !has_type),
    }
}

/// rekordbox's `edit/attribute.svg` (19 x 16, grey 149) drawn with its top left at `o`,
/// scaled by `k`.
pub fn attribute_icon(p: &egui::Painter, o: egui::Pos2, k: f32, col: egui::Color32) {
    let at = |x: f32, y: f32| o + egui::vec2(x * k, y * k);
    for (x0, y0, x1, y1) in [(0.0, 0.0, 2.0, 16.0), (4.0, 4.0, 6.0, 12.0)] {
        p.rect_filled(egui::Rect::from_min_max(at(x0, y0), at(x1, y1)), 0.0, col);
    }
    for pts in [vec![(13.0, 15.0), (9.0, 11.0), (15.0, 5.0), (19.0, 9.0)], vec![(12.0, 3.0), (14.0, 3.0), (14.0, 4.029), (12.0, 5.979)], vec![(8.0, 1.0), (10.0, 1.0), (10.0, 8.0), (8.0, 10.0)], vec![(12.0, 16.0), (8.0, 16.0), (8.0, 12.0)]] {
        p.add(egui::Shape::convex_polygon(pts.into_iter().map(|(x, y)| at(x, y)).collect(), col, egui::Stroke::NONE));
    }
}

/// The attribute icons of a row (EDIT, Track Type) in `cell`, with their tooltips.
pub fn attribute_icons(app: &mut App, p: &egui::Painter, cell: egui::Rect, id: i64, tips: &mut Vec<&'static str>) {
    use egui::vec2;
    let edited = edited_set(app).contains(&id);
    app.track_types_ready();
    let ty = app.cue_tools.cached_type(id);
    let col = crate::theme::hex(0x959595);
    let mut x = cell.min.x + 4.0;
    let cy = cell.center().y;
    if edited {
        // (At 75 %: 14 x 12 pt in the 30 pt cell.)
        attribute_icon(p, egui::pos2(x, cy - 6.0), 0.75, col);
        tips.push(if crate::stem_export::is_stem_track(app, id) { "Stem track exported from another track." } else { "Tracks created in EDIT mode." });
        x += 16.0;
    }
    match ty {
        Some(1) => {
            // sampler.svg: an arrow, 10 x 9.
            let o = egui::pos2(x, cy - 4.5);
            p.add(egui::Shape::convex_polygon(vec![o + vec2(5.195, 0.0), o + vec2(9.996, 4.5), o + vec2(5.195, 8.996)], col, egui::Stroke::NONE));
            p.rect_filled(egui::Rect::from_min_max(o + vec2(0.0, 4.0), o + vec2(5.2, 5.0)), 0.0, col);
        }
        Some(2) => {
            // samplerLoop.svg: a loop with an arrow, 12 x 8.
            let o = egui::pos2(x, cy - 4.0);
            p.rect_stroke(egui::Rect::from_min_max(o + vec2(3.0, 1.5), o + vec2(10.8, 6.4)), 1.4, egui::Stroke::new(1.4, col), egui::StrokeKind::Middle);
            p.add(egui::Shape::convex_polygon(vec![o + vec2(3.05, 0.87), o + vec2(5.7, 4.93), o + vec2(0.4, 4.93)], col, egui::Stroke::NONE));
        }
        _ => {}
    }
}

/// The Attribute cell of the list header: a click opens the filter list. Returns true when
/// the click was taken. `cell`: the header's attribute cell.
pub fn attribute_header(app: &mut App, ui: &mut egui::Ui, resp: &egui::Response, cell: egui::Rect) -> bool {
    use egui::{vec2, Align2, RichText};
    let on = app.edit.attr_filter.is_some();
    crate::icons_src::funnel(ui.painter(), egui::pos2(cell.center().x, cell.center().y - 1.0), if on { crate::theme::TILE_ON.c() } else { crate::theme::DIM.c() });
    let hit = resp.clicked() && resp.interact_pointer_pos().map(|pp| cell.contains(pp)).unwrap_or(false);
    if hit {
        app.edit.attr_popup = !app.edit.attr_popup;
        edited_set(app);
    }
    if app.edit.attr_popup {
        let mut close = false;
        let area = egui::Area::new(egui::Id::new("attr_filter_list")).order(egui::Order::Foreground).fixed_pos(cell.left_bottom() + vec2(0.0, 2.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).fill(crate::theme::hex(0x202020)).show(ui, |ui| {
                ui.set_min_width(170.0);
                let (mut all, (mut ed, mut none)) = (app.edit.attr_filter.is_none(), app.edit.attr_filter.unwrap_or((true, true)));
                let mut changed = false;
                changed |= ui.checkbox(&mut all, RichText::new("Select all").size(11.5)).changed();
                if changed {
                    ed = all;
                    none = all;
                }
                let c1 = ui.checkbox(&mut ed, RichText::new("Edited Tracks").size(11.5)).changed();
                let c2 = ui.checkbox(&mut none, RichText::new("No Attribute").size(11.5)).changed();
                if changed || c1 || c2 {
                    app.edit.attr_filter = if ed && none { None } else { Some((ed, none)) };
                    app.list_dirty = true;
                }
            });
        });
        let _ = Align2::LEFT_TOP;
        if !hit && ui.input(|i| i.pointer.any_pressed()) && !area.response.rect.contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default())) {
            close = true;
        }
        if close {
            app.edit.attr_popup = false;
        }
    }
    hit
}

/// Key of the setting that links a rendered track to the track it was edited from.
pub fn edited_key(id: i64) -> String {
    format!("edit_of_{id}")
}

/// The cues of the final score as collection cues of the rendered file: beats on the new
/// constant grid become seconds. A hot cue slot that the edits duplicated keeps its first copy.
pub fn cues_for_render(s: &ScoreSet, track_id: i64) -> Vec<Cue> {
    let secs = |b: f64| b * 60.0 / s.bpm;
    let len = s.len();
    let mut out = vec![];
    let mut used = std::collections::HashSet::new();
    let mut hot = s.prepared.hot.clone();
    hot.sort_by(|a, b| a.pos_in.partial_cmp(&b.pos_in).unwrap_or(std::cmp::Ordering::Equal));
    for c in hot.iter().filter(|c| c.pos_in >= 0.0 && c.pos_in <= len) {
        if !used.insert(c.cue_id) {
            continue;
        }
        out.push(Cue { id: 0, track_id, kind: CueKind::Hot, slot: c.cue_id, start: secs(c.pos_in), end: c.is_loop().then(|| secs(c.pos_out.min(len))), color: c.appended.color, comment: c.appended.comment.clone(), active_loop: c.appended.status != 0 && c.is_loop() });
    }
    for c in s.prepared.memory.iter().filter(|c| c.pos_in >= 0.0 && c.pos_in <= len) {
        out.push(Cue { id: 0, track_id, kind: CueKind::Memory, slot: -1, start: secs(c.pos_in), end: c.is_loop().then(|| secs(c.pos_out.min(len))), color: c.appended.color, comment: c.appended.comment.clone(), active_loop: c.appended.status != 0 && c.is_loop() });
    }
    out
}

/// rekordbox's steps after a render (spec 5.3): the file joins the collection with a
/// constant grid, the score's cues, the source track's details, My Tags and artwork, and a
/// link back to the source. Returns the new track's id.
pub fn import_render(app: &mut App, path: &Path, dlg: &RenderDlg, show: bool) -> Option<i64> {
    let (src, score) = {
        let p = app.edit.project.as_ref()?;
        (p.track.clone(), p.session.cur.clone())
    };
    let p = path.to_string_lossy().to_string();
    let id = match app.db.track_by_path(&p) {
        Some(id) => id,
        None => app.db.insert_track(&crate::importer::read_track(path)).ok()?,
    };
    // The timeline's grid: constant, from 0, with the timeline's bar lines.
    let first_beat_no = ((-score.bar_offset).rem_euclid(4) + 1) as u8;
    let _ = app.db.save_grid(id, &BeatGrid::constant(0.0, score.bpm, first_beat_no));
    let _ = app.db.replace_cues(id, &cues_for_render(&score, id));
    app.db.set_setting(&edited_key(id), &src.id.to_string());
    app.edit.edited.get_or_insert_with(Default::default).insert(id);
    if let Some(mut t) = app.db.track(id) {
        t.title = dlg.track.clone();
        t.artist = dlg.artist.clone();
        t.bpm = score.bpm;
        t.duration = score.len_ms() / 1000.0;
        t.album = src.album.clone();
        t.album_artist = src.album_artist.clone();
        t.genre = src.genre.clone();
        t.label = src.label.clone();
        t.remixer = src.remixer.clone();
        t.original_artist = src.original_artist.clone();
        t.composer = src.composer.clone();
        t.lyricist = src.lyricist.clone();
        t.mix_name = src.mix_name.clone();
        t.comment = src.comment.clone();
        t.grouping = src.grouping.clone();
        t.message = src.message.clone();
        t.year = src.year;
        t.track_no = src.track_no;
        t.disc_no = src.disc_no;
        t.release_date = src.release_date.clone();
        t.key = src.key;
        t.key_text = src.key_text.clone();
        t.key_manual = src.key_manual;
        t.rating = src.rating;
        t.color = src.color;
        t.artwork = src.artwork.clone();
        let _ = app.db.update_track(&t);
    }
    for tag in app.track_tags.get(&src.id).cloned().unwrap_or_default() {
        let _ = app.db.set_track_tag(id, tag, true);
    }
    app.reload_tracks();
    app.reload_tags();
    app.refresh_cue_counts(id);
    // Related: the new track matches the source and what the source already matched.
    let mut linked = app.matching_of(src.id);
    linked.push(src.id);
    for other in linked {
        if other != id {
            app.matching_set(other, id, true);
        }
    }
    // Waveform, gain and the rest come from the normal analysis of the new file (its grid,
    // already there, is kept).
    app.analyze(&[id], false);
    if show {
        app.prefs.edit.show_browser = true;
        app.search.clear();
        app.side = SideMode::Collection;
        app.set_view(View::Collection);
        app.rebuild_list();
        app.selected.clear();
        app.selected.insert(id);
        app.anchor_id = Some(id);
        app.anchor = app.list.iter().position(|t| *t == id);
        app.scroll_to_selected = true;
    }
    app.save_prefs();
    Some(id)
}

// ---------------------------------------------------------------------------- menu bar

/// The editor's items in the native menu bar (File and Edit), added to what `menus.rs` built.
pub(crate) struct MenuItems {
    items: Vec<(&'static str, muda::MenuItem, Option<muda::accelerator::Accelerator>)>,
    recent: muda::Submenu,
    recent_shown: Vec<String>,
    accel_on: bool,
}

pub fn attach_menu(app: &mut App) {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
    let Some(menu) = &app.menu else { return };
    let cmd = Modifiers::META;
    let defs: [(&'static str, &str, Option<Accelerator>, u8); 8] = [
        ("ed_open", "Open project", None, 0),
        ("ed_save", "Save", Some(Accelerator::new(cmd, Code::KeyS)), 0),
        ("ed_save_as", "Save as", None, 0),
        ("ed_render", "Render audio files", Some(Accelerator::new(cmd, Code::KeyR)), 0),
        ("ed_clone", "Clone", Some(Accelerator::new(cmd, Code::KeyD)), 1),
        ("ed_paste", "Paste over", Some(Accelerator::new(cmd, Code::KeyP)), 1),
        ("ed_delete", "Delete", None, 1),
        ("ed_clear", "Clear", Some(Accelerator::new(cmd, Code::Backspace)), 1),
    ];
    let recent = Submenu::with_id("ed_recent", "Open a recently edited project", true);
    let mut items = vec![];
    let subs: Vec<Submenu> = menu.items().into_iter().filter_map(|k| if let MenuItemKind::Submenu(s) = k { Some(s) } else { None }).collect();
    // The items are put into File and Edit by `refill_menu`, each time the menu bar is laid
    // out for a mode (menus.rs): rekordbox shows them in EDIT mode only.
    for (id, label, acc, _) in defs.iter() {
        // (Accelerators are switched on only in EDIT mode: see `sync_menu`.)
        items.push((*id, MenuItem::with_id(*id, *label, false, None), *acc));
    }
    // Quit goes through the save prompt: the system's own Quit item cannot be asked to wait.
    if let Some(app_menu) = subs.first() {
        if let Some(MenuItemKind::Predefined(q)) = app_menu.items().into_iter().last() {
            let _ = app_menu.remove(&q);
            let _ = app_menu.append(&MenuItem::with_id("ed_quit", "Quit recordbox", true, Some(Accelerator::new(cmd, Code::KeyQ))));
        }
    }
    app.edit.menu = Some(MenuItems { items, recent, recent_shown: vec![String::from("\u{0}")], accel_on: false });
}

/// Put EDIT mode's items into the File and Edit menus after the menu bar was laid out for a
/// mode. rekordbox's File menu in EDIT starts with Open a recently edited project, Open
/// project, Save, Save as and Render audio files; its Edit menu has Clone, Paste (over),
/// Delete and Clear beside the clipboard commands.
pub fn refill_menu(app: &App, edit_mode: bool) {
    use muda::{MenuItemKind, PredefinedMenuItem, Submenu};
    if !edit_mode {
        return;
    }
    let (Some(menu), Some(m)) = (&app.menu, &app.edit.menu) else { return };
    let subs: Vec<Submenu> = menu.items().into_iter().filter_map(|k| if let MenuItemKind::Submenu(s) = k { Some(s) } else { None }).collect();
    let item = |id: &str| m.items.iter().find(|i| i.0 == id).map(|i| &i.1);
    if let Some(file) = subs.iter().find(|s| s.text() == "File") {
        let _ = file.prepend(&PredefinedMenuItem::separator());
        for id in ["ed_render", "ed_save_as", "ed_save", "ed_open"] {
            if let Some(i) = item(id) {
                let _ = file.prepend(i);
            }
        }
        let _ = file.prepend(&m.recent);
    }
    if let Some(edit) = subs.iter().find(|s| s.text() == "Edit") {
        let _ = edit.append(&PredefinedMenuItem::separator());
        for id in ["ed_clone", "ed_paste", "ed_delete", "ed_clear"] {
            if let Some(i) = item(id) {
                let _ = edit.append(i);
            }
        }
    }
}

fn sync_menu(app: &mut App) {
    if app.edit.menu.is_none() {
        return;
    }
    let edit = is_edit(app);
    let want: Vec<bool> = ["ed_open", "ed_save", "ed_save_as", "ed_render", "ed_clone", "ed_paste", "ed_delete", "ed_clear"]
        .iter()
        .map(|id| {
            edit && match *id {
                "ed_open" => true,
                "ed_save" | "ed_save_as" | "ed_render" => app.edit.project.is_some(),
                "ed_clone" => can(app, &Cmd::Edit(EditCmd::Clone)),
                "ed_paste" => can(app, &Cmd::Edit(EditCmd::Paste)),
                "ed_delete" => can(app, &Cmd::Edit(EditCmd::Delete)),
                _ => can(app, &Cmd::Edit(EditCmd::Clear)),
            }
        })
        .collect();
    let recent = app.prefs.edit.recent.clone();
    let m = app.edit.menu.as_mut().unwrap();
    if app.edit.menu_shown != want {
        for ((_, it, _), on) in m.items.iter().zip(&want) {
            it.set_enabled(*on);
        }
        app.edit.menu_shown = want;
    }
    if m.accel_on != edit {
        for (_, it, acc) in &m.items {
            let _ = it.set_accelerator(if edit { *acc } else { None });
        }
        m.accel_on = edit;
    }
    if m.recent_shown != recent {
        for k in m.recent.items() {
            if let muda::MenuItemKind::MenuItem(i) = k {
                let _ = m.recent.remove(&i);
            }
        }
        for (i, f) in recent.iter().enumerate() {
            let name = Path::new(f).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let _ = m.recent.append(&muda::MenuItem::with_id(format!("ed_recent_{i}"), name, true, None));
        }
        m.recent.set_enabled(!recent.is_empty());
        m.recent_shown = recent;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn attribute_filter_lets_edited_tracks_and_plain_ones_through() {
        assert!(attr_ok(None, false, false) && attr_ok(None, true, true));
        // Edited Tracks only.
        assert!(attr_ok(Some((true, false)), true, false));
        assert!(!attr_ok(Some((true, false)), false, false));
        // No Attribute only: neither edited nor a sampler type.
        assert!(attr_ok(Some((false, true)), false, false));
        assert!(!attr_ok(Some((false, true)), true, false) && !attr_ok(Some((false, true)), false, true));
        // Nothing ticked: nothing passes.
        assert!(!attr_ok(Some((false, false)), true, false));
    }

    use super::*;

    fn track() -> Track {
        Track { id: 9, path: "/x.wav".into(), title: "T".into(), bpm: 120.0, analyzed: 1, ..Default::default() }
    }

    #[test]
    fn load_checks_in_rekordboxs_order() {
        let dir = std::env::temp_dir().join(format!("recordbox_editload_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.wav");
        std::fs::write(&f, b"x").unwrap();
        let mut t = track();
        assert_eq!(load_check(&t, true), Err((false, "Load error. The file cannot be found.")));
        t.path = f.to_string_lossy().to_string();
        assert!(load_check(&t, true).is_ok());
        t.analyzed = 0;
        assert!(load_check(&t, true).unwrap_err().1.starts_with("Tracks that have not been analyzed"));
        t.analyzed = 1;
        assert!(load_check(&t, false).unwrap_err().1.starts_with("The tracks without grids"));
        for (bpm, ok) in [(39.99, false), (40.0, true), (499.0, true), (499.01, false), (0.0, false)] {
            t.bpm = bpm;
            assert_eq!(load_check(&t, true).is_ok(), ok, "{bpm}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_loaded_tracks_score_covers_the_file_with_its_cues_in_beats() {
        let t = track();
        // First beat at 0.25 s, beat 3 of its bar; 10 s long.
        let grid = BeatGrid::constant(0.25, 120.0, 3);
        let cues = vec![
            Cue { id: 5, track_id: 9, kind: CueKind::Hot, slot: 2, start: 2.25, end: None, color: 7, comment: "drop".into(), active_loop: false },
            Cue { id: 6, track_id: 9, kind: CueKind::Hot, slot: 0, start: 4.25, end: Some(6.25), color: 1, comment: String::new(), active_loop: true },
            Cue { id: 7, track_id: 9, kind: CueKind::Memory, slot: -1, start: 0.5, end: None, color: 0, comment: String::new(), active_loop: false },
        ];
        let s = new_score(&t, &grid, &cues, 10.0);
        assert!(s.is_valid());
        // Beats at 0.25, 0.75 ... 9.75 s (20), one added in front (-0.25 s), and enough at
        // the end to reach 11 s (duration + 1 s).
        assert_eq!((s.song_grid.org_beats.len(), s.song_grid.pre, s.song_grid.post), (20, 1, 3));
        assert_eq!(s.len(), 23.0);
        // The beat at 0.25 s is beat 3 of its bar, so bar lines are the analysed beats 2, 6 ...
        // and the timeline beats 3, 7 ...
        assert_eq!((s.song_grid.org_bar_offset, s.bar_offset), (2, 3));
        // 2.25 s is the 5th analysed beat: timeline beat 5 (one beat was added in front).
        let h = &s.prepared.hot;
        assert_eq!((h[0].cue_id, h[0].pos_in, h[0].is_loop(), h[0].appended.db_cue_id, h[0].appended.color), (2, 5.0, false, 5, 7));
        assert_eq!((h[1].pos_in, h[1].pos_out, h[1].appended.status), (9.0, 13.0, 1));
        assert_eq!(s.prepared.memory[0].pos_in, 1.5);
        // Rendered back: beats on the constant grid are seconds again; a slot cloned by an
        // edit keeps its first copy.
        let mut e = s.clone();
        assert!(e.duplicate(4.0, 8.0));
        let out = cues_for_render(&e, 77);
        let hot: Vec<(i32, f64, Option<f64>)> = out.iter().filter(|c| c.kind == CueKind::Hot).map(|c| (c.slot, c.start, c.end)).collect();
        assert_eq!(hot, vec![(2, 2.5, None), (0, 6.5, Some(8.5))]);
        assert!(out.iter().all(|c| c.track_id == 77));
        assert_eq!(out.iter().filter(|c| c.kind == CueKind::Memory).count(), 1);
        assert!(out.iter().find(|c| c.slot == 0).unwrap().active_loop);
    }

    #[test]
    fn zoom_keeps_the_anchor_beat_in_place() {
        let mut st = EditState::default();
        assert_eq!(st.zoom, 5);
        // No project: 120 BPM assumed. Step 5 is 8 frames per point: 150 / 8 * 0.5 pt a beat.
        assert!((st.pt_per_beat() - 9.375).abs() < 1e-4);
        st.view_start = 10.0;
        let beat_at = |st: &EditState, x: f32| st.view_start + (x / st.pt_per_beat()) as f64;
        let before = beat_at(&st, 300.0);
        st.set_zoom(4, 300.0);
        assert!((beat_at(&st, 300.0) - before).abs() < 1e-3);
        st.set_zoom(40, 300.0);
        assert_eq!(st.zoom, 15);
        assert!((beat_at(&st, 300.0) - before).abs() < 1e-2);
        st.set_zoom(0, 300.0);
        assert_eq!(st.zoom, 1);
    }
}
