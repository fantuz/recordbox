//! Recording, as rekordbox 7.2.19 has it (`RecControlPanel`; spec
//! `checks/re-7.2.19/mixer-fx-recording.md` section 4): the recording panel of PERFORMANCE and
//! EXPORT mode, what happens when a recording ends (separating, level, tag information window,
//! import, cue sheet), the Recordings source of the Tree View, Preferences > Controller >
//! Recordings and the keyboard commands. The engine is `recorder.rs`.
//!
//! Layout numbers are rekordbox's own (`RecControlPanel::resized` 0x1018b5ed8: the Performance
//! panel is 344 x 38 pt, the Export one 438 x 44) and measurements of
//! `rekordbox-reference/performance/09-recording.jpg` and `preferences/11`.

use crate::app::*;
use crate::recorder::{self as rec, CueHead, CueTrack, Recorded, TagInfo};
use crate::theme::*;
use crate::ui_kit::{self as kit, btn, card, inter, inter_bold, radio, separator, toggle};
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Stroke, Ui};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

// rekordbox's words.
const MSG_COMPLETE: &str = "Recording completed. The recorded file has been saved. The file can be found at Recordings in Tree View.";
const MSG_NO_FOLDER: &str = "Invalid operation. To retrieve, set location to save the recorded file in [Preferences].";
const MSG_CANNOT_SAVE: &str = "Recording stopped. Recorded file cannot be saved.";
const MSG_DISK_FULL: &str = "Recording stopped. Disk full.";
const TIP_SOURCE: &str = "Recording source:\nSelect a channel to be recorded.";
const TIP_KNOB: &str = "Recording Level knob:\nAdjust recording level.";
const TIP_METER: &str = "Recording Level Indicator:\nShow recording level.";
const TIP_REC: &str = "Start/Stop recording.";
const TIP_TIME: &str = "Show recorded time and remaining time.";
const HELP_SILENCE: &str = "You can select a threshold value to judge there is no sound.\nUse the slider bar to select a threshold.\nThis threshold is used for the following features:\n - To start recording (To detect audio input).\n - To end recording (To detect 20 sec. of silence).\n - To automatically separate files when silence continues.";

/// Names of the settings of Preferences > Controller > Recordings, for the search field.
pub const SETTINGS: [&str; 9] = [
    "Recordings",
    "Location of Files",
    "Silence Level Threshold",
    "Recording starts when:",
    "Recording ends when:",
    "Automatically separate files when silence continues.",
    "Automatically optimize audio level of a recorded file.",
    "Automatically import a recorded file.",
    "Automatically display tag information window of a recorded file.",
];

/// Recording settings (rekordbox's keys in brackets) and the panel's own state.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct RecPrefs {
    /// The recording panel is shown in PERFORMANCE mode / in EXPORT mode.
    pub panel: bool,
    pub panel_export: bool,
    /// Location of Files [RecordSaveDirectory]; empty = recordbox's own Recordings folder.
    pub dir: String,
    /// Folders chosen before (the pull-down's list).
    pub dirs: Vec<String>,
    /// Silence Level Threshold in dB, -78..-6 [RecSilenceLevelThreshold].
    pub silence_db: f32,
    /// Recording starts when audio is detected [RecordTrigger 2] or at the REC press [1].
    pub start_on_audio: bool,
    /// Recording also ends after 20 seconds of silence [RecStopTrigger 2].
    pub stop_on_silence: bool,
    /// Automatically separate files when silence continues [RecAutoTrackSplit_PerformanceMode,
    /// _ExportMode].
    pub auto_split: bool,
    pub auto_split_export: bool,
    /// "When silence continues for:" in seconds, 1..10 [RecSilenceDurationThreshold].
    pub silence_secs: u8,
    /// Automatically optimize audio level of a recorded file [RecLevelNormalize].
    pub normalize: bool,
    /// Automatically import a recorded file [RecAutoImport].
    pub auto_import: bool,
    /// Automatically display tag information window of a recorded file
    /// [RecTrackInfoInputWizard].
    pub tag_window: bool,
    /// The level knob, -1..1 around the centre, in each mode.
    pub level: f32,
    pub level_export: f32,
    /// The DJ name typed last into the tag information window [DJName].
    pub dj_name: String,
}

impl Default for RecPrefs {
    fn default() -> Self {
        RecPrefs {
            panel: false,
            panel_export: false,
            dir: String::new(),
            dirs: vec![],
            silence_db: -38.0,
            start_on_audio: true,
            stop_on_silence: true,
            auto_split: false,
            auto_split_export: true,
            silence_secs: 3,
            normalize: false,
            auto_import: false,
            tag_window: true,
            level: 0.0,
            level_export: 0.0,
            dj_name: String::new(),
        }
    }
}

/// recordbox's own folder for recordings.
pub fn default_dir() -> PathBuf {
    crate::db::data_dir().join("Recordings")
}

/// rekordbox's folders are never written to.
fn forbidden(p: &Path) -> bool {
    let Some(home) = dirs::home_dir() else { return false };
    p.starts_with(home.join("Music").join("rekordbox")) || p.starts_with(home.join("Library").join("Pioneer"))
}

/// The folder recordings are saved in.
pub fn rec_dir(p: &RecPrefs) -> PathBuf {
    let chosen = p.dir.trim();
    if chosen.is_empty() || forbidden(Path::new(chosen)) { default_dir() } else { PathBuf::from(chosen) }
}

enum Stage {
    AskSplit,
    Normalize,
    Working,
    Tags,
    Finish,
}

struct Job {
    rx: crossbeam_channel::Receiver<Result<Vec<(PathBuf, f64)>, String>>,
    progress: Arc<AtomicU32>,
    label: &'static str,
    next: Stage,
}

/// A finished recording on its way through rekordbox's steps (`notifyOfRecResult`).
struct Post {
    /// The files (one, or the separated ones) and their lengths in ms.
    files: Vec<(PathBuf, f64)>,
    splits: Vec<f64>,
    tracks: Vec<CueTrack>,
    export_mode: bool,
    stage: Stage,
    job: Option<Job>,
    /// The tag information window: one entry per file, the one showing, its three fields.
    tags: Vec<TagInfo>,
    edited: Vec<bool>,
    cur: usize,
    fields: [String; 3],
    fields_for: Option<usize>,
    confirm_delete: bool,
    /// The files carry the window's tags already (else the plain ones are written).
    tagged: bool,
    page: usize,
}

#[derive(Default)]
pub struct RecUi {
    session: Option<rec::Session>,
    /// REC was pressed and the recording waits for sound (the button blinks).
    pub armed: bool,
    blink: bool,
    blink_at: f64,
    log: rec::OnAirLog,
    log_at: f64,
    shown_db: [f32; 2],
    hold: [(usize, f64); 2],
    meter_at: f64,
    free: Option<u64>,
    free_at: f64,
    posts: VecDeque<Post>,
    /// The files of the recording that finished last, its frames, and the highest level that
    /// went into the one being made or made last (checks).
    pub last: Vec<PathBuf>,
    pub last_frames: u64,
    pub last_dropped: u64,
    pub peak: f32,
    list_sel: Option<String>,
    noise: [f32; 2],
    noise_hold: [(f32, f64); 2],
    noise_at: f64,
    colours: Option<Vec<Color32>>,
    prefs_seen: f64,
    ready: bool,
    /// Where the tag information window's title field and OK button are (the scripted run).
    tag_pts: Option<(Pos2, Pos2)>,
    /// "Now recording. Quit rekordbox?" is asking; the answer was Yes (the window may close).
    pub quit_ask: bool,
    pub quit_ok: bool,
    /// Where the question's Yes and No are (the scripted run).
    quit_pts: Option<(Pos2, Pos2)>,
    /// The separate-files window's top left (the scripted run).
    split_o: Option<Pos2>,
    /// A recording dragged from the Recordings list that is not in the collection yet: it is
    /// imported, and the drag carries it once it is there.
    drag_file: Option<String>,
    /// Tracks made from the files of the Recordings list (their tags), by path and size.
    rows: std::collections::HashMap<(String, u64), crate::model::Track>,
}

impl RecUi {
    pub fn recording(&self) -> bool {
        self.session.is_some()
    }
    /// A finished recording is still going through its windows and steps.
    pub fn busy(&self) -> bool {
        !self.posts.is_empty()
    }
}

fn tip(resp: egui::Response, app: &App, text: &str) -> egui::Response {
    if app.prefs.tooltips { resp.on_hover_text(text) } else { resp }
}

/// The colours of the level indicator's 88 segments: rekordbox's own strip (read from the
/// installed rekordbox, one row per segment), or the same run from green over amber to red.
fn segment_colours() -> Vec<Color32> {
    let skin = Path::new("/Applications/rekordbox 7/rekordbox.app/Contents/Resources/skins/p10574_decklv.png");
    if let Ok(img) = image::open(skin) {
        let img = img.to_rgba8();
        let (w, h) = (img.width(), img.height());
        if w > 0 && h >= 88 {
            return (0..88).map(|k| { let p = img.get_pixel(w / 2, h - 1 - (k * h / 88).min(h - 1)); Color32::from_rgb(p[0], p[1], p[2]) }).collect();
        }
    }
    let stops = [(0.0, [5.0, 153.0, 66.0]), (44.0, [10.0, 153.0, 65.0]), (70.0, [224.0, 143.0, 0.0]), (78.0, [224.0, 143.0, 0.0]), (87.0, [196.0, 12.0, 10.0])];
    (0..88)
        .map(|k| {
            let x = k as f32;
            let i = stops.iter().rposition(|s| s.0 <= x).unwrap_or(0).min(stops.len() - 2);
            let t = ((x - stops[i].0) / (stops[i + 1].0 - stops[i].0)).clamp(0.0, 1.0);
            let c: Vec<u8> = (0..3).map(|j| (stops[i].1[j] + (stops[i + 1].1[j] - stops[i].1[j]) * t) as u8).collect();
            Color32::from_rgb(c[0], c[1], c[2])
        })
        .collect()
}

impl App {
    fn rec_level(&self) -> f32 {
        if self.performance() { self.prefs.rec.level } else { self.prefs.rec.level_export }
    }

    fn rec_base_db(&self) -> f32 {
        if self.performance() { rec::BASE_DB_PERFORMANCE } else { 0.0 }
    }

    fn rec_auto_split(&self) -> bool {
        if self.performance() { self.prefs.rec.auto_split } else { self.prefs.rec.auto_split_export }
    }

    /// The panel is on screen in the mode showing.
    pub fn rec_panel_shown(&self) -> bool {
        if self.performance() { self.prefs.rec.panel } else { self.prefs.rec.panel_export }
    }

    /// Show or hide the recording panel of the mode showing.
    pub fn rec_toggle_panel(&mut self) {
        if self.performance() {
            self.prefs.rec.panel = !self.prefs.rec.panel;
        } else {
            self.prefs.rec.panel_export = !self.prefs.rec.panel_export;
        }
        self.save_prefs();
    }

    /// Which decks are on air (rekordbox's mixer rule), and what each has loaded.
    fn rec_on_air(&self) -> ([bool; 4], [Option<(String, String, String)>; 4]) {
        let perf = self.performance();
        let master = self.player.state.lock().volume;
        let mut on = [false; 4];
        let mut loaded: [Option<(String, String, String)>; 4] = [None, None, None, None];
        for n in 0..self.deck_count().min(4) {
            let m = &self.px.mix[n as usize];
            on[n as usize] = if perf { rec::on_air(master, self.px.crossfader, m.assign, m.fader, self.prefs.ch_fader_curve, self.prefs.eq_isolator, m.eq, m.trim) } else { master * 1023.0 >= 23.0 };
            loaded[n as usize] = self.deck_of(n).track.and_then(|id| self.track(id)).map(|t| (t.title.clone(), t.artist.clone(), t.path.clone()));
        }
        (on, loaded)
    }

    /// Start a recording now (`RecControlPanel::startRecording`).
    pub fn rec_start(&mut self) -> bool {
        if self.deck_options.disable_play {
            self.toast_err("Recording requires audio output. Playback is disabled for this session (--disable-play).");
            return false;
        }
        if self.rec.session.is_some() {
            return true;
        }
        let dir = rec_dir(&self.prefs.rec);
        if self.prefs.rec.dir.trim().is_empty() {
            let _ = std::fs::create_dir_all(&dir);
        }
        if !dir.is_dir() {
            self.toast_err(MSG_NO_FOLDER);
            return false;
        }
        let wav = rec::next_rec_path(&dir, &chrono::Local::now().format("%Y-%m-%d").to_string());
        let p = &self.prefs.rec;
        let settings = rec::RecSettings { silence_db: p.silence_db as f64, silence_ms: p.silence_secs.clamp(1, 10) as f64 * 1000.0, auto_split: self.rec_auto_split(), auto_stop: p.stop_on_silence };
        let e = rec::engine();
        e.set_gain(rec::knob_gain(self.rec_level(), self.rec_base_db()));
        e.set_preroll(p.start_on_audio);
        match rec::Session::start_on(e, wav, settings) {
            Ok(s) => {
                self.rec.session = Some(s);
                self.rec.armed = false;
                self.rec.log.clear();
                self.rec.log_at = 0.0;
                self.rec.peak = 0.0;
                true
            }
            Err(_) => {
                self.toast_err(MSG_NO_FOLDER);
                false
            }
        }
    }

    /// Stop the recording and send the file on its way through the steps after it.
    pub fn rec_stop(&mut self) {
        let Some(s) = self.rec.session.take() else { return };
        // One more look at the decks, so that a short recording has its tracks too.
        if self.rec.log.tracks.is_empty() {
            let (on, loaded) = self.rec_on_air();
            self.rec.log.poll(on, &loaded, 0);
        }
        let done = s.finish(rec::engine());
        self.rec_finished(done);
    }

    fn rec_finished(&mut self, done: Recorded) {
        let tracks = std::mem::take(&mut self.rec.log.tracks);
        self.rec.log.clear();
        self.rec.last_frames = done.frames;
        self.rec.last_dropped = done.dropped;
        match done.error {
            Some(rec::RecError::DiskFull) => self.toast_err(MSG_DISK_FULL),
            Some(_) => self.toast_err(MSG_CANNOT_SAVE),
            None => {}
        }
        if done.frames == 0 || !done.file.is_file() {
            let _ = std::fs::remove_file(&done.file);
            self.rec.last = vec![];
            return;
        }
        let ms = done.length_ms();
        let export_mode = !self.performance();
        let ask = self.rec_auto_split() && !done.splits.is_empty() && done.error.is_none();
        let stage = if done.error.is_some() { Stage::Finish } else if ask { Stage::AskSplit } else { Stage::Normalize };
        self.rec.posts.push_back(Post { files: vec![(done.file, ms)], splits: done.splits, tracks, export_mode, stage, job: None, tags: vec![], edited: vec![], cur: 0, fields: Default::default(), fields_for: None, confirm_delete: false, tagged: false, page: 0 });
    }

    /// The REC button (`RecControlPanel::buttonClicked`): with "Detecting audio input." the
    /// first press arms the recording and a second one takes that back; a press while
    /// recording stops it.
    pub fn rec_button(&mut self) {
        if self.rec.session.is_some() {
            self.rec_stop();
        } else if self.prefs.rec.start_on_audio {
            if self.rec.armed {
                self.rec.armed = false;
            } else {
                let dir = rec_dir(&self.prefs.rec);
                if self.prefs.rec.dir.trim().is_empty() {
                    let _ = std::fs::create_dir_all(&dir);
                }
                if dir.is_dir() {
                    self.rec.armed = true;
                    self.rec.blink = true;
                    // (The trigger looks at what comes from now on.)
                    let _ = rec::engine().take_peak(rec::METER_TRIGGER);
                } else {
                    self.toast_err(MSG_NO_FOLDER);
                }
            }
        } else {
            self.rec_start();
        }
    }

    /// The keyboard commands (`RecControlPanel::perform`): 0 Start/Pause REC, 1 and 2 the
    /// level up and down by 3 % of its range, 3 Start REC, 4 Pause REC. They show the panel.
    pub fn rec_command(&mut self, cmd: u8) {
        if !self.rec_panel_shown() {
            self.rec_toggle_panel();
        }
        let active = self.rec.session.is_some() || self.rec.armed;
        match cmd {
            0 => self.rec_button(),
            1 | 2 => {
                let d = if cmd == 1 { 0.06 } else { -0.06 };
                if self.performance() {
                    self.prefs.rec.level = (self.prefs.rec.level + d).clamp(-1.0, 1.0);
                } else {
                    self.prefs.rec.level_export = (self.prefs.rec.level_export + d).clamp(-1.0, 1.0);
                }
                self.save_prefs();
            }
            3 if !active => self.rec_button(),
            4 if active => self.rec_button(),
            _ => {}
        }
    }

    /// Once per frame: the level and the trigger, the three-hour limit, the on-air log, and
    /// the steps after a recording.
    pub fn poll_rec(&mut self) {
        let now = self.ctx.input(|i| i.time);
        let prefs_open = now - self.rec.prefs_seen < 0.5;
        let wanted = self.rec_panel_shown() || self.rec.session.is_some() || self.rec.armed || prefs_open;
        if !wanted {
            self.rec_post();
            return;
        }
        let e = rec::engine();
        if !self.rec.ready {
            self.rec.ready = true;
            self.rec.shown_db = [-120.0; 2];
            self.rec.noise = [-120.0; 2];
            self.rec.noise_hold = [(-120.0, 0.0); 2];
            // recordbox's own Recordings folder is there from the start, as rekordbox's is.
            if self.prefs.rec.dir.trim().is_empty() {
                let _ = std::fs::create_dir_all(default_dir());
            }
        }
        e.set_gain(rec::knob_gain(self.rec_level(), self.rec_base_db()));
        e.set_preroll(self.prefs.rec.start_on_audio);
        // The writer ended the recording itself: silence, a full disk or a write error.
        if self.rec.session.as_ref().map(|s| s.finished()).unwrap_or(false) {
            if let Some(s) = self.rec.session.take() {
                let done = s.finish(e);
                self.rec_finished(done);
            }
        }
        // Standby: recording starts when the level passes the threshold.
        if self.rec.armed && self.rec.session.is_none() {
            let p = e.take_peak(rec::METER_TRIGGER);
            if rec::to_db(p[0].max(p[1])) > self.prefs.rec.silence_db {
                self.rec_start();
            }
            if now - self.rec.blink_at > 0.25 {
                self.rec.blink = !self.rec.blink;
                self.rec.blink_at = now;
            }
        }
        // Free space, once a second.
        if now - self.rec.free_at >= 1.0 || self.rec.free_at == 0.0 {
            self.rec.free_at = now.max(0.001);
            let dir = rec_dir(&self.prefs.rec);
            self.rec.free = if dir.is_dir() { crate::export::usb::free_space(&dir) } else { Some(0) };
        }
        if self.rec.session.is_some() {
            let elapsed = e.elapsed() as u64;
            // The on-air log looks at the mixer every 500 ms.
            if now - self.rec.log_at >= 0.5 {
                self.rec.log_at = now;
                let (on, loaded) = self.rec_on_air();
                self.rec.log.poll(on, &loaded, e.elapsed_ms());
            }
            // Three hours, or the disk is full: the file is closed; after three hours the
            // next one starts at once.
            if rec::remaining_secs(elapsed, self.rec.free) == 0 {
                let by_time = elapsed >= rec::MAX_SECS;
                self.rec_stop();
                if by_time {
                    self.rec_start();
                } else {
                    self.toast_err(MSG_DISK_FULL);
                }
            }
        }
        // The level indicator: read every 36 ms; up at once, down 2 dB a reading; the highest
        // segment stays for two seconds.
        let dt = (now - self.rec.meter_at) as f32;
        if dt >= 0.036 {
            self.rec.meter_at = now;
            let p = e.take_peak(rec::METER_PANEL);
            if self.rec.session.is_some() {
                self.rec.peak = self.rec.peak.max(p[0]).max(p[1]);
            }
            for c in 0..2 {
                self.rec.shown_db[c] = rec::to_db(p[c]).max(self.rec.shown_db[c] - 2.0 * dt.min(0.5) / 0.036).max(-120.0);
                let seg = rec::meter_segments(self.rec.shown_db[c]);
                if seg >= self.rec.hold[c].0 || now > self.rec.hold[c].1 {
                    self.rec.hold[c] = (seg, now + 2.0);
                }
            }
        }
        if self.rec.session.is_some() || self.rec.armed || self.rec.shown_db.iter().any(|d| *d > -100.0) {
            self.ctx.request_repaint_after(std::time::Duration::from_millis(36));
        }
        self.rec_post();
    }

    /// The steps after a recording, in rekordbox's order: separate (asked), optimize the
    /// level, tag information window, plain tags, import, message, cue sheet.
    fn rec_post(&mut self) {
        let Some(mut post) = self.rec.posts.pop_front() else { return };
        let mut keep = true;
        match post.stage {
            Stage::AskSplit | Stage::Tags => {}
            Stage::Normalize => {
                if self.prefs.rec.normalize && !post.files.is_empty() {
                    let files = post.files.clone();
                    let target = self.prefs.target_lufs;
                    let progress = Arc::new(AtomicU32::new(0));
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    let (pr, ctx) = (progress.clone(), self.ctx.clone());
                    std::thread::spawn(move || {
                        let n = files.len() as u32;
                        for (i, (f, _)) in files.iter().enumerate() {
                            let one = AtomicU32::new(0);
                            // (A file that cannot be rewritten is left as it was recorded.)
                            let _ = std::thread::scope(|s| {
                                let h = s.spawn(|| rec::normalize_file(f, target, &one));
                                while !h.is_finished() {
                                    pr.store((i as u32 * 1000 + one.load(Relaxed)) / n.max(1), Relaxed);
                                    ctx.request_repaint();
                                    std::thread::sleep(std::time::Duration::from_millis(30));
                                }
                                h.join()
                            });
                        }
                        pr.store(1000, Relaxed);
                        let _ = tx.send(Ok(files));
                        ctx.request_repaint();
                    });
                    post.job = Some(Job { rx, progress, label: "Optimizing audio level of a recorded file...", next: Stage::Tags });
                    post.stage = Stage::Working;
                } else {
                    post.stage = Stage::Tags;
                }
            }
            Stage::Working => {
                if let Some(job) = &post.job {
                    if let Ok(res) = job.rx.try_recv() {
                        match res {
                            Ok(files) => post.files = files,
                            Err(e) => self.toast_err(e),
                        }
                        let job = post.job.take().unwrap();
                        post.stage = job.next;
                    }
                }
            }
            Stage::Finish => {
                self.rec_post_finish(&mut post);
                keep = false;
            }
        }
        if matches!(post.stage, Stage::Tags) && (!self.prefs.rec.tag_window || post.files.is_empty()) {
            post.stage = Stage::Finish;
        }
        if keep {
            self.rec.posts.push_front(post);
            if !matches!(self.rec.posts[0].stage, Stage::AskSplit | Stage::Tags) {
                self.ctx.request_repaint();
            }
        }
    }

    fn rec_post_finish(&mut self, post: &mut Post) {
        if post.files.is_empty() {
            self.rec.last = vec![];
            return;
        }
        // Without the window's tags a file gets the plain ones: its name as the title.
        if !post.tagged {
            post.tags = post.files.iter().enumerate().map(|(i, (f, _))| TagInfo { title: f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), track_no: if post.files.len() > 1 { i as u32 + 1 } else { 0 }, ..Default::default() }).collect();
            for ((f, _), t) in post.files.iter().zip(&post.tags) {
                let _ = rec::write_tags(f, t);
            }
        }
        let files: Vec<PathBuf> = post.files.iter().map(|f| f.0.clone()).collect();
        if self.prefs.rec.auto_import {
            self.import_paths(files.clone(), None);
        }
        self.toast(MSG_COMPLETE);
        // A cue sheet beside each file, from the tracks that went on air.
        let mut start = 0.0;
        for (i, (f, ms)) in post.files.iter().enumerate() {
            let tag = post.tags.get(i).cloned().unwrap_or_default();
            let date = std::fs::metadata(f).and_then(|m| m.created()).ok().map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %I:%M %p").to_string()).unwrap_or_default();
            let head = CueHead { date, recorded_by: "recordbox".into(), title: tag.title, performer: tag.artist, file: f.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), ..Default::default() };
            let _ = rec::write_cue(f, &rec::cue_text(&head, &post.tracks, start as i64, *ms as i64));
            start += ms;
        }
        // The Recordings source shows where the file went.
        if self.side == SideMode::Recordings {
            let folder = files[0].parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            self.set_view(View::Explorer(folder));
        }
        self.rec.last = files;
    }
}

// ------------------------------------------------------------------------------ the panel

/// The recording panel in the bottom row of the PERFORMANCE screen: 344 x 38 pt, 10 pt from
/// the row's right end (reference performance/09).
pub fn panel(app: &mut App, ui: &mut Ui, row: Rect) {
    let o = pos2(row.max.x - 10.0 - 344.0, row.min.y);
    controls(app, ui, o, false);
}

/// The recording panel of EXPORT mode, a 44 pt strip under the top bar: 438 x 44 pt.
pub fn export_strip(app: &mut App, ui: &mut Ui) {
    let r = ui.max_rect();
    ui.painter().rect_filled(r, 0.0, BG2);
    controls(app, ui, pos2(r.max.x - 10.0 - 438.0, r.min.y), true);
}

/// Height of the EXPORT mode strip.
pub const EXPORT_H: f32 = 44.0;

/// The source selector, the level knob and indicator, REC and the two times, laid out as
/// `RecControlPanel::resized` does for the mode. `o`: the panel's top left.
fn controls(app: &mut App, ui: &mut Ui, o: Pos2, export: bool) {
    let p = ui.painter().clone();
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(o.x + x, o.y + y), vec2(w, h));
    let now = ui.input(|i| i.time);
    // (bounds: source, knob, indicator, REC, elapsed, slash, remaining)
    let b: [Rect; 7] = if export {
        [at(0.0, 14.0, 88.0, 18.0), at(112.0, 14.0, 24.0, 24.0), at(141.0, 18.0, 80.0, 11.0), at(245.0, 6.0, 32.0, 32.0), at(301.0, 12.0, 70.0, 20.0), at(373.0, 16.0, 9.0, 15.0), at(384.0, 16.0, 54.0, 15.0)]
    } else {
        [at(0.0, 10.0, 88.0, 18.0), at(98.0, 10.0, 24.0, 24.0), at(127.0, 15.0, 80.0, 9.0), at(217.0, 9.0, 21.0, 19.0), at(243.0, 13.0, 50.0, 14.0), at(295.0, 15.0, 7.0, 11.0), at(304.0, 15.0, 40.0, 11.0)]
    };
    // ---- source selector: "Master Out" is what there is to record without DJ hardware.
    let src = ui.interact(b[0], ui.id().with("recsrc"), Sense::click());
    p.text(pos2(b[0].min.x + 6.0, b[0].center().y + 0.25), Align2::LEFT_CENTER, "Master Out", caps(11.75), WHITE.c());
    let cc = pos2(b[0].max.x - 4.25, b[0].center().y + 0.25);
    p.add(egui::Shape::line(vec![cc + vec2(-3.6, -2.0), cc + vec2(0.0, 2.0), cc + vec2(3.6, -2.0)], Stroke::new(1.0_f32, WHITE)));
    crate::ui_browser::dropdown(ui, &src, "recsrcpop", |ui| {
        if ui.selectable_label(true, "Master Out").clicked() {
            ui.memory_mut(|m| m.close_popup());
        }
        // rekordbox lists these with DJ hardware that has the inputs.
        for name in ["Channel1", "Channel2", "Channel3", "Channel4", "MIC"] {
            ui.add_enabled(false, egui::SelectableLabel::new(false, name)).on_disabled_hover_text("Needs DJ hardware with this input; recordbox records its own master output.");
        }
    });
    let _ = tip(src, app, TIP_SOURCE);
    // ---- level knob
    let level = app.rec_level();
    let kc = b[1].center();
    let kresp = ui.interact(b[1], ui.id().with("reclevel"), Sense::click_and_drag());
    let mut v = level;
    if kresp.double_clicked() {
        v = 0.0;
    } else if kresp.dragged() {
        v = (level - kresp.drag_delta().y / 75.0).clamp(-1.0, 1.0);
    }
    let ang = |x: f32| (-225.0 + 270.0 * x).to_radians();
    let ring: Vec<Pos2> = (0..=36).map(|i| kc + vec2(ang(i as f32 / 36.0).cos(), ang(i as f32 / 36.0).sin()) * 8.75).collect();
    p.add(egui::Shape::line(ring, Stroke::new(2.5_f32, crate::ui_feel::hot_col(&kresp, gy(0x323232), gy(0x464646)))));
    let a = ang((v + 1.0) / 2.0);
    p.line_segment([kc + vec2(a.cos(), a.sin()) * 5.5, kc + vec2(a.cos(), a.sin()) * 10.0], Stroke::new(2.0_f32, WHITE));
    if v != level {
        if app.performance() {
            app.prefs.rec.level = v;
        } else {
            app.prefs.rec.level_export = v;
        }
    }
    if kresp.drag_stopped() || kresp.double_clicked() {
        app.save_prefs();
    }
    let _ = tip(kresp, app, TIP_KNOB);
    // ---- level indicator: two rows (L over R) of 88 segments.
    if app.rec.colours.is_none() {
        app.rec.colours = Some(segment_colours());
    }
    let bar_h = if export { 4.0 } else { 3.0 };
    for c in 0..2 {
        let br = Rect::from_min_size(pos2(b[2].min.x, if c == 0 { b[2].min.y } else { b[2].max.y - bar_h }), vec2(b[2].width(), bar_h));
        p.rect_filled(br, 0.0, gy(0x323232));
        let lit = rec::meter_segments(app.rec.shown_db[c]);
        let seg = br.width() / 88.0;
        if let Some(cols) = &app.rec.colours {
            let mut bars = kit::Bars::new();
            for k in 0..lit {
                bars.rect(Rect::from_min_size(pos2(br.min.x + k as f32 * seg, br.min.y), vec2(seg, bar_h)), cols[k.min(87)]);
            }
            let (held, until) = app.rec.hold[c];
            if held > lit && held >= 1 && now <= until {
                bars.rect(Rect::from_min_size(pos2(br.min.x + (held - 1) as f32 * seg, br.min.y), vec2(seg, bar_h)), cols[(held - 1).min(87)]);
            }
            bars.paint(&p);
        }
    }
    let _ = tip(ui.interact(b[2].expand(2.0), ui.id().with("recmeter"), Sense::hover()), app, TIP_METER);
    // ---- REC: rekordbox's `recPanel_rec_btn_off` / `_on` (21 x 19: a rounded tile with a
    // 5 pt disc), scaled to the button. Lit while recording; it blinks in standby.
    let recording = app.rec.session.is_some();
    let lit = recording || (app.rec.armed && app.rec.blink);
    let rresp = ui.interact(b[3], ui.id().with("recbtn"), Sense::click());
    let k = (b[3].width() / 21.0).min(b[3].height() / 19.0);
    let tile = Rect::from_center_size(b[3].center(), vec2(21.0 * k, 19.0 * k));
    let red = Color32::from_rgb(200, 0, 0);
    p.rect_filled(tile, 2.0 * k, if lit { red } else { crate::ui_feel::hl(&rresp, gy(0x292929), gy(0x343434)) });
    p.circle_filled(tile.center() + vec2(0.0, -0.5 * k), 5.0 * k, if lit { gy(0xAFAFAF) } else { red });
    if rresp.clicked() {
        app.rec_button();
    }
    let _ = tip(rresp, app, TIP_REC);
    // ---- recorded time / time left
    let elapsed = if recording { rec::engine().elapsed() as u64 } else { 0 };
    let left = rec::remaining_secs(elapsed, app.rec.free);
    let grey = gy(0x878787);
    let (f_el, f_sl, f_re) = if export { (rb(17.0), rb(12.5), rb(13.4)) } else { (rb(9.0), rb(9.0), rb(9.0)) };
    p.text(pos2(b[4].max.x + 0.25, b[4].center().y + 0.25), Align2::RIGHT_CENTER, rec::hms(elapsed), f_el, if export { grey } else { WHITE.c() });
    p.text(pos2(b[5].center().x - 0.5, b[5].center().y + 0.25), Align2::CENTER_CENTER, "/", f_sl, grey);
    p.text(pos2(b[6].min.x, b[6].center().y + 0.25), Align2::LEFT_CENTER, rec::hms(left), f_re, grey);
    let _ = tip(ui.interact(b[4].union(b[6]), ui.id().with("rectime"), Sense::hover()), app, TIP_TIME);
}

// ---------------------------------------------------------------- windows after a recording

fn hms_ms(ms: f64) -> String {
    rec::hms((ms * 0.001) as u64)
}

/// The windows a finished recording brings up: "separate?", the progress of a file being
/// rewritten, and the tag information window.
pub fn windows(app: &mut App, ctx: &egui::Context) {
    quit_guard(app, ctx);
    let Some(mut post) = app.rec.posts.pop_front() else { return };
    match post.stage {
        Stage::AskSplit => split_window(app, ctx, &mut post),
        Stage::Working => {
            if let Some(job) = &post.job {
                kit::dialog(ctx, "recordbox").id(egui::Id::new("rec_progress")).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
                    kit::apply_style(ui);
                    ui.set_width(360.0);
                    kit::message(ui, job.label);
                    ui.add_space(8.0);
                    ui.add(egui::ProgressBar::new(job.progress.load(Relaxed) as f32 / 1000.0).desired_width(360.0));
                });
            }
        }
        Stage::Tags => tag_window(app, ctx, &mut post),
        _ => {}
    }
    app.rec.posts.push_front(post);
}

/// Quitting while recording (`RekordboxApplication::systemRequestedQuit` 0x10084394c): an
/// alert with no title and no icon, "Now recording. Quit rekordbox?" (here with recordbox's
/// name), Yes / No. Yes stops the recording at once (`RecControlPanel::stopRecSync`: the file
/// is finished, the steps after a recording are not run) and the app quits; No keeps both.
pub const MSG_QUIT: &str = "Now recording. Quit recordbox?";

fn quit_guard(app: &mut App, ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().close_requested()) && app.rec.recording() && !app.rec.quit_ok {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        app.rec.quit_ask = true;
    }
    if !app.rec.quit_ask {
        return;
    }
    if !app.rec.recording() {
        app.rec.quit_ask = false;
        return;
    }
    let mut answer = None;
    let mut pts = None;
    kit::dialog(ctx, " ").id(egui::Id::new("rec_quit")).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        kit::apply_style(ui);
        ui.set_width(360.0);
        kit::message(ui, MSG_QUIT);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(((ui.available_width() - 200.0) / 2.0).max(0.0));
            let y = ui.add(btn("Yes").min_size(vec2(96.0, 24.0)));
            let n = ui.add(btn("No").min_size(vec2(96.0, 24.0)));
            pts = Some((y.rect.center(), n.rect.center()));
            if y.clicked() {
                answer = Some(0);
            }
            if n.clicked() {
                answer = Some(1);
            }
        });
    });
    app.rec.quit_pts = pts;
    match answer {
        Some(0) => {
            app.rec.quit_ask = false;
            app.rec.quit_ok = true;
            app.rec_stop();
            // (Only the file is finished; the questions and the tag window are not shown.)
            app.rec.posts.clear();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        Some(_) => app.rec.quit_ask = false,
        None => {}
    }
}

/// "You can separate the recorded file ..." (`RecAutoTrackSplitComponent`, 580 x 400).
fn split_window(app: &mut App, ctx: &egui::Context, post: &mut Post) {
    let total = post.files.first().map(|f| f.1).unwrap_or(0.0);
    let lengths = rec::split_lengths(total, &post.splits);
    let mut answer: Option<bool> = None;
    let mut split_o = None;
    kit::dialog(ctx, "recordbox").id(egui::Id::new("rec_split")).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
        kit::apply_style(ui);
        let (r, _) = ui.allocate_exact_size(vec2(580.0 - 32.0, 400.0 - 32.0), Sense::hover());
        let o = r.min - vec2(16.0, 16.0);
        split_o = Some(o);
        let p = ui.painter().clone();
        let text = |x: f32, y: f32, s: &str| {
            p.text(pos2(o.x + x, o.y + y + 12.0), Align2::LEFT_CENTER, s, inter(13.0), WHITE.c());
        };
        text(30.0, 30.0, "You can separate the recorded file and save them as follows:");
        text(60.0, 80.0, "Total :");
        text(130.0, 80.0, &hms_ms(total));
        // Ten tracks a page in two columns of five.
        let pages = (lengths.len() + 9) / 10;
        post.page = post.page.min(pages.saturating_sub(1));
        for (k, len) in lengths.iter().enumerate().skip(post.page * 10).take(10) {
            let (col, row) = ((k - post.page * 10) / 5, (k - post.page * 10) % 5);
            let y = 110.0 + 27.0 * row as f32;
            text(60.0 + 170.0 * col as f32, y, &format!("Track {} :", k + 1));
            text(130.0 + 170.0 * col as f32, y, &hms_ms(*len));
        }
        if pages > 1 {
            let prev = Rect::from_min_size(o + vec2(430.0, 150.0), vec2(60.0, 24.0));
            let next = Rect::from_min_size(o + vec2(490.0, 150.0), vec2(60.0, 24.0));
            if ui.put(prev, btn("<").exact(vec2(60.0, 24.0))).clicked() && post.page > 0 {
                post.page -= 1;
            }
            if ui.put(next, btn(">").exact(vec2(60.0, 24.0))).clicked() && post.page + 1 < pages {
                post.page += 1;
            }
        }
        text(30.0, 272.0, "Will you save the separate recorded files?");
        text(50.0, 299.0, "* If you click \"No\", the recorded file will be saved as a whole.");
        if ui.put(Rect::from_min_size(o + vec2(210.0, 348.0), vec2(60.0, 24.0)), btn("Yes").exact(vec2(60.0, 24.0))).clicked() {
            answer = Some(true);
        }
        if ui.put(Rect::from_min_size(o + vec2(280.0, 348.0), vec2(60.0, 24.0)), btn("No").exact(vec2(60.0, 24.0))).clicked() {
            answer = Some(false);
        }
    });
    app.rec.split_o = split_o;
    match answer {
        Some(true) => {
            let file = post.files[0].0.clone();
            let progress = Arc::new(AtomicU32::new(0));
            let (tx, rx) = crossbeam_channel::bounded(1);
            let (pr, ctx2) = (progress.clone(), app.ctx.clone());
            std::thread::spawn(move || {
                let res = rec::split_file(&file, &lengths, &pr).map(|v| v.into_iter().map(|(p, frames)| (p, frames as f64 * 1000.0 / rec::REC_RATE as f64)).collect()).map_err(|e| format!("The recorded file could not be separated: {e}"));
                let _ = tx.send(res);
                ctx2.request_repaint();
            });
            post.job = Some(Job { rx, progress, label: "Separating the recorded file...", next: Stage::Normalize });
            post.stage = Stage::Working;
        }
        Some(false) => post.stage = Stage::Normalize,
        None => {}
    }
}

/// The tag information window (`RecTrackInfoInputComponent`, 600 x 400): DJ, Album and Mix
/// Title in PERFORMANCE mode, Artist, Album and Track Title in EXPORT mode; one page per
/// file; OK or Delete.
fn tag_window(app: &mut App, ctx: &egui::Context, post: &mut Post) {
    let n = post.files.len();
    if post.tags.len() != n {
        post.tags = (0..n).map(|i| TagInfo { track_no: i as u32 + 1, ..Default::default() }).collect();
        post.edited = vec![false; n];
        post.cur = 0;
        post.fields_for = None;
    }
    let perf = !post.export_mode;
    // A page that opens: its fields, and in PERFORMANCE mode what goes into empty ones.
    if post.fields_for != Some(post.cur) {
        let t = &post.tags[post.cur];
        post.fields = [t.artist.clone(), t.album.clone(), t.title.clone()];
        if perf {
            if post.fields[0].is_empty() {
                post.fields[0] = if app.prefs.rec.dj_name.trim().is_empty() { "Unknown Artist".into() } else { app.prefs.rec.dj_name.clone() };
            }
            if post.fields[1].is_empty() {
                post.fields[1] = "Unknown Album".into();
            }
            if post.fields[2].is_empty() {
                post.fields[2] = post.files[post.cur].0.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            }
        }
        post.fields_for = Some(post.cur);
    }
    let (mut go, mut ok, mut delete) = (0i32, false, false);
    let mut pts = None;
    kit::dialog(ctx, "recordbox").id(egui::Id::new("rec_tags")).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
        kit::apply_style(ui);
        let (r, _) = ui.allocate_exact_size(vec2(600.0 - 32.0, 400.0 - 32.0), Sense::hover());
        let o = r.min - vec2(16.0, 16.0);
        // The Mix Title / Track Title field's centre (the third field: 184 + 24 .. + 48) and
        // OK's. (The scripted run once aimed 24 pt below the field, so it could not type.)
        pts = Some((o + vec2(290.0, 184.0 + 24.0 + 12.0), o + vec2(210.0, 360.0)));
        let p = ui.painter().clone();
        let text = |x: f32, y: f32, s: &str, c: Color32| {
            p.text(pos2(o.x + x, o.y + y + 12.0), Align2::LEFT_CENTER, s, inter(13.0), c);
        };
        text(30.0, 20.0, if perf { "Enter the DJ name, album title and mix title." } else { "Enter an artist name, an album name and a track title." }, WHITE.c());
        let labels = if perf { ["DJ", "Album", "Mix Title"] } else { ["Artist", "Album", "Track Title"] };
        for (i, l) in labels.iter().enumerate() {
            let y = 68.0 + 58.0 * i as f32;
            text(80.0, y, l, WHITE.c());
            let field = Rect::from_min_size(o + vec2(90.0, y + 24.0), vec2(400.0, 24.0));
            // (rekordbox keeps the characters a file name cannot hold out of these fields.)
            let resp = ui.put(field, egui::TextEdit::singleline(&mut post.fields[i]).id(egui::Id::new(("rec_tag_field", i))).font(inter(13.0)));
            if resp.changed() {
                post.fields[i].retain(|c| !matches!(c, '/' | ':' | '\\' | '\0'));
            }
        }
        let prev = ui.add_enabled_ui(post.cur > 0, |ui| ui.put(Rect::from_min_size(o + vec2(90.0, 266.0), vec2(60.0, 24.0)), btn("<").exact(vec2(60.0, 24.0)))).inner;
        let next = ui.add_enabled_ui(post.cur + 1 < n, |ui| ui.put(Rect::from_min_size(o + vec2(150.0, 266.0), vec2(60.0, 24.0)), btn(">").exact(vec2(60.0, 24.0)))).inner;
        if prev.clicked() {
            go = -1;
        }
        if next.clicked() {
            go = 1;
        }
        text(240.0, 266.0, "Track : ", DIM2.c());
        text(300.0, 266.0, &format!("{} / {}", post.cur + 1, n), WHITE.c());
        text(360.0, 266.0, "Time : ", DIM2.c());
        text(420.0, 266.0, &hms_ms(post.files[post.cur].1), WHITE.c());
        if post.confirm_delete {
            text(90.0, 310.0, "Are you sure you want to delete the selected file(s)?", WHITE.c());
            if ui.put(Rect::from_min_size(o + vec2(150.0, 348.0), vec2(120.0, 24.0)), btn("Ok").exact(vec2(120.0, 24.0))).clicked() {
                delete = true;
            }
            if ui.put(Rect::from_min_size(o + vec2(330.0, 348.0), vec2(120.0, 24.0)), btn("Cancel").exact(vec2(120.0, 24.0))).clicked() {
                post.confirm_delete = false;
            }
        } else {
            if ui.put(Rect::from_min_size(o + vec2(150.0, 348.0), vec2(120.0, 24.0)), btn("OK").exact(vec2(120.0, 24.0))).clicked() {
                ok = true;
            }
            if ui.put(Rect::from_min_size(o + vec2(330.0, 348.0), vec2(120.0, 24.0)), btn("Delete").exact(vec2(120.0, 24.0))).clicked() {
                post.confirm_delete = true;
            }
        }
    });
    app.rec.tag_pts = pts;
    // Leaving a page keeps what was typed; a page not seen yet takes the artist and album.
    let keep = |post: &mut Post| {
        let t = &mut post.tags[post.cur];
        (t.artist, t.album, t.title) = (post.fields[0].clone(), post.fields[1].clone(), post.fields[2].clone());
        post.edited[post.cur] = true;
    };
    if go != 0 {
        keep(post);
        let to = (post.cur as i32 + go).clamp(0, n as i32 - 1) as usize;
        if !post.edited[to] {
            (post.tags[to].artist, post.tags[to].album) = (post.fields[0].clone(), post.fields[1].clone());
        }
        post.cur = to;
    }
    if delete {
        for (f, _) in &post.files {
            let _ = std::fs::remove_file(f);
        }
        post.files.clear();
        post.stage = Stage::Finish;
    }
    if ok {
        keep(post);
        if perf {
            app.prefs.rec.dj_name = post.fields[0].clone();
            app.save_prefs();
        }
        let (artist, album) = (post.fields[0].clone(), post.fields[1].clone());
        for i in 0..n {
            if !post.edited[i] {
                (post.tags[i].artist, post.tags[i].album) = (artist.clone(), album.clone());
            }
        }
        // Nothing entered at all: the files stay where they are with their plain tags.
        if !post.tags.iter().all(|t| t.artist.trim().is_empty() && t.album.trim().is_empty() && t.title.trim().is_empty()) {
            if let Err(e) = file_tagged(post) {
                app.toast_err(e);
            }
            post.tagged = true;
        }
        post.stage = Stage::Finish;
    }
}

/// OK in the tag information window (`RecTrackInfoInputComponent::finish`): the files move to
/// `<artist>/<album>/NN <title>.wav` under the folder they were recorded in ("Compilations"
/// when the artists differ; an album folder that exists gets a number), then the tags are
/// written.
fn file_tagged(post: &mut Post) -> Result<(), String> {
    let base = post.files[0].0.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut artists: Vec<&str> = post.tags.iter().map(|t| t.artist.trim()).filter(|a| !a.is_empty()).collect();
    artists.dedup();
    let dir_info = TagInfo { artist: if artists.len() > 1 { "Compilations".into() } else { post.tags[0].artist.clone() }, album: post.tags[0].album.clone(), ..Default::default() };
    let probe = rec::tagged_path(&base, &dir_info, "wav");
    let artist_dir = probe.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or(base.clone());
    let album_name = probe.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let album_dir = (|| -> std::io::Result<PathBuf> {
        // Follow the recording folder selected by the user, but do not follow an
        // artist symlink or junction out of it when filing the new recording.
        match std::fs::create_dir(&artist_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let artist = artist_dir.canonicalize()?;
        if artist.parent() != Some(base.canonicalize()?.as_path()) || std::fs::symlink_metadata(&artist_dir)?.is_symlink() {
            return Err(std::io::Error::other("Artist folder is outside the recording folder or is a link"));
        }
        for n in 1usize.. {
            let album = artist.join(if n == 1 { album_name.clone() } else { format!("{album_name} ({n})") });
            match std::fs::create_dir(&album) {
                Ok(()) => return Ok(album),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    })();
    let mut err = album_dir.as_ref().map(|_| ()).map_err(|e| format!("Could not create the recording folder: {e}"));
    for (i, (f, _)) in post.files.iter_mut().enumerate() {
        let t = &post.tags[i];
        if let Ok(album_dir) = &album_dir {
            let ext = f.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "wav".into());
            let named = rec::tagged_path(&base, &TagInfo { track_no: t.track_no, title: t.title.clone(), ..Default::default() }, &ext);
            let dest = album_dir.join(named.file_name().unwrap_or_default());
            if rec::move_recording(f, &dest).is_ok() {
                *f = dest;
            } else {
                err = Err("A recorded file could not be moved to its folder; it stays where it was recorded.".to_string());
            }
        }
        let _ = rec::write_tags(f, t);
    }
    err
}

// ------------------------------------------------------------------------- Recordings tree

fn sub_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(true)).collect())
        .unwrap_or_default();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

fn wav_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && crate::audio::has_audio_extension(p)).collect())
        .unwrap_or_default();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

/// `dir` is the Recordings folder or inside it (its lists are the Recordings source's).
pub fn owns(app: &App, dir: &str) -> bool {
    app.side == SideMode::Recordings && Path::new(dir).starts_with(rec_dir(&app.prefs.rec))
}

/// The Recordings source of the Tree View: the save folder, and under it the artist and album
/// folders the tag information window files recordings in.
pub fn tree(app: &mut App, ui: &mut Ui) {
    use crate::ui_browser::{tree_row, TreeIcon};
    let root = rec_dir(&app.prefs.rec);
    let rs = root.to_string_lossy().to_string();
    let now = ui.input(|i| i.time);
    // Folder listings are read twice a second at most.
    let listing = |ui: &Ui, dir: &Path| -> Vec<PathBuf> {
        let key = egui::Id::new(("rec_dirs", dir.to_string_lossy().to_string()));
        let cached: Option<(f64, Vec<PathBuf>)> = ui.ctx().data(|d| d.get_temp(key));
        match cached {
            Some((t, v)) if now - t < 0.5 => v,
            _ => {
                let v = sub_dirs(dir);
                ui.ctx().data_mut(|d| d.insert_temp(key, (now, v.clone())));
                v
            }
        }
    };
    let open_key = egui::Id::new("rec_tree_open");
    let open = ui.ctx().data(|d| d.get_temp::<bool>(open_key)).unwrap_or(true);
    let sel = app.view == View::Explorer(rs.clone());
    let r = tree_row(ui, 0, "Recordings", None, sel, if open { TreeIcon::Open } else { TreeIcon::Closed }, 15.0);
    if r.clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(open_key, !open || !sel));
        app.set_view(View::Explorer(rs));
    }
    if !open {
        return;
    }
    egui::ScrollArea::vertical().id_salt("rec_tree").auto_shrink([false, false]).show(ui, |ui| {
        for artist in listing(ui, &root) {
            let aps = artist.to_string_lossy().to_string();
            let key = egui::Id::new(("rec_open", aps.clone()));
            let albums = listing(ui, &artist);
            let inside = matches!(&app.view, View::Explorer(d) if Path::new(d).starts_with(&artist));
            let open = ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(inside);
            let sel = app.view == View::Explorer(aps.clone());
            let name = artist.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let r = tree_row(ui, 1, &name, None, sel, if open { TreeIcon::FolderOpen } else { TreeIcon::FolderClosed }, 15.0);
            if r.clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(key, !open || !sel));
                app.set_view(View::Explorer(aps));
            }
            if open {
                for album in albums {
                    let bps = album.to_string_lossy().to_string();
                    let sel = app.view == View::Explorer(bps.clone());
                    let name = album.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    if tree_row(ui, 2, &name, Some(wav_files(&album).len()), sel, TreeIcon::Playlist, 15.0).clicked() {
                        app.set_view(View::Explorer(bps));
                    }
                }
            }
        }
    });
}

/// A recording as a row of the track list: its tags, length, size and file details.
fn row_track(f: &Path, size: u64, secs: f64) -> crate::model::Track {
    let tag = rec::read_tags(f);
    let mut t = crate::model::Track { path: f.to_string_lossy().to_string(), key: -1, ..Default::default() };
    t.title = if tag.title.is_empty() { f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() } else { tag.title };
    t.artist = tag.artist;
    t.album = tag.album;
    t.duration = secs;
    t.file_size = size;
    t.file_type = "wav".into();
    t.sample_rate = rec::REC_RATE as u32;
    t.bitrate = 1411;
    t.track_no = tag.track_no as i32;
    if let Ok(m) = std::fs::metadata(f).and_then(|m| m.modified()) {
        let d: chrono::DateTime<chrono::Local> = m.into();
        t.date_created = d.format("%Y-%m-%d").to_string();
    }
    t
}

/// The collection's columns as the Recordings list shows them (order and widths of the
/// collection's list).
pub fn rec_columns(app: &App) -> Vec<(Col, f32)> {
    app.prefs.columns.iter().copied().filter(|c| !matches!(c, Col::Artwork)).map(|c| (c, crate::ui_browser::col_width(app, c))).collect()
}

/// The recordings in a folder of the Recordings source. A double click loads one (importing
/// it first when it is not in the collection yet).
pub fn list(app: &mut App, ui: &mut Ui, dir: &str) {
    let r = ui.max_rect();
    let now = ui.input(|i| i.time);
    let key = egui::Id::new(("rec_files", dir.to_string()));
    let cached: Option<(f64, Vec<(PathBuf, u64, f64)>)> = ui.ctx().data(|d| d.get_temp(key));
    let files = match cached {
        Some((t, v)) if now - t < 1.0 => v,
        _ => {
            let v: Vec<(PathBuf, u64, f64)> = wav_files(Path::new(dir))
                .into_iter()
                .map(|p| {
                    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    let secs = rec::wav_info(&p).map(|(rate, ch, bits, _, bytes)| bytes as f64 / (rate.max(1) as f64 * ch.max(1) as f64 * (bits.max(8) / 8) as f64)).unwrap_or(0.0);
                    (p, size, secs)
                })
                .collect();
            ui.ctx().data_mut(|d| d.insert_temp(key, (now, v.clone())));
            v
        }
    };
    let in_collection: std::collections::HashMap<&str, i64> = app.tracks.iter().map(|t| (t.path.as_str(), t.id)).collect();
    let ids: Vec<Option<i64>> = files.iter().map(|f| in_collection.get(f.0.to_string_lossy().as_ref()).copied()).collect();
    // A row as the collection shows it: the collection's own track, else one made from the
    // file's tags (read once per file and size).
    let rows: Vec<crate::model::Track> = files
        .iter()
        .zip(&ids)
        .map(|((f, size, secs), id)| {
            if let Some(t) = id.and_then(|id| app.track(id)) {
                return t.clone();
            }
            let key = (f.to_string_lossy().to_string(), *size);
            app.rec.rows.entry(key).or_insert_with(|| row_track(f, *size, *secs)).clone()
        })
        .collect();
    // A dragged recording that had to be imported first: the drag carries it once it is in.
    if let Some(df) = app.rec.drag_file.clone() {
        if !ui.input(|i| i.pointer.any_down()) {
            app.rec.drag_file = None;
        } else if let Some(id) = app.tracks.iter().find(|t| t.path == df).map(|t| t.id) {
            app.drag = Some(vec![id]);
            app.rec.drag_file = None;
        }
    }
    let bar = Rect::from_min_size(r.min, vec2(r.width(), 26.0));
    let p = ui.painter().clone();
    p.rect_filled(bar, 0.0, BG2);
    let root = rec_dir(&app.prefs.rec);
    let title = match Path::new(dir).strip_prefix(&root) {
        Ok(rel) if !rel.as_os_str().is_empty() => format!("Recordings / {} ({} Tracks)", rel.to_string_lossy().replace('/', " / "), files.len()),
        _ => format!("Recordings ({} Tracks)", files.len()),
    };
    p.text(pos2(bar.min.x + 10.0, bar.center().y), Align2::LEFT_CENTER, title, rb(crate::ui_browser::TITLE_FONT), WHITE.c());
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(r.min.x, r.min.y + 26.0), r.max)));
    // The collection's columns in its order and widths (the ones drawn rather than written,
    // such as the artwork and the attribute icons, stay empty: a recording has none).
    let cols: Vec<(Col, f32)> = rec_columns(app);
    let hdr = body.allocate_exact_size(vec2(body.available_width(), 14.0), Sense::hover()).0;
    body.painter().rect_filled(hdr, 0.0, BG2);
    let mut x = hdr.min.x;
    for (c, w) in &cols {
        body.painter().with_clip_rect(Rect::from_min_max(pos2(x, hdr.min.y), pos2(x + w - 2.0, hdr.max.y))).text(pos2(x + 4.0, hdr.center().y), Align2::LEFT_CENTER, c.label(), rb(10.5), WHITE.c());
        body.painter().rect_filled(Rect::from_min_max(pos2(x + w - 1.0, hdr.min.y), pos2(x + w, hdr.max.y)), 0.0, BG);
        x += w;
    }
    let mut sel = app.rec.list_sel.clone();
    egui::ScrollArea::vertical().id_salt("rec_list").auto_shrink([false, false]).show_rows(&mut body, crate::ui_browser::ROW_H, files.len(), |ui, range| {
        for i in range {
            let (f, _, _) = &files[i];
            let fs = f.to_string_lossy().to_string();
            let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), crate::ui_browser::ROW_H), Sense::click_and_drag());
            // (A selected row keeps rekordbox's fixed blue and white text in Light: `ListViewer::findTextColour`.)
            let _sel_dark = crate::theme::dark_scope(sel.as_deref() == Some(fs.as_str()));
            if sel.as_deref() == Some(fs.as_str()) {
                ui.painter().rect_filled(rr, 0.0, ROW_SEL);
            } else if i % 2 == 1 {
                ui.painter().rect_filled(rr, 0.0, gy(0x0E0E0E));
            }
            let t = &rows[i];
            let mut x = rr.min.x;
            for (c, w) in &cols {
                let v = if *c == Col::Num { (i + 1).to_string() } else { crate::ui_library_ext::col_text(app, t, *c).unwrap_or_default() };
                ui.painter().with_clip_rect(Rect::from_min_max(pos2(x, rr.min.y), pos2(x + w - 4.0, rr.max.y))).text(pos2(x + 4.0, rr.center().y), Align2::LEFT_CENTER, v, rb(10.5), WHITE.c());
                x += w;
            }
            if resp.clicked() || resp.drag_started() {
                sel = Some(fs.clone());
            }
            // Dragged onto a deck, a sampler slot or a playlist, as a collection row is; a
            // recording the collection does not have yet is imported first.
            if resp.drag_started() {
                match ids[i] {
                    Some(id) => app.drag = Some(vec![id]),
                    None => {
                        app.rec.drag_file = Some(fs.clone());
                        app.import_paths(vec![f.clone()], None);
                    }
                }
            }
            if resp.double_clicked() {
                match ids[i] {
                    Some(id) => app.load_track(id),
                    None => {
                        app.load_after_import = Some(fs.clone());
                        app.import_paths(vec![f.clone()], None);
                    }
                }
            }
            resp.context_menu(|ui| {
                if ids[i].is_none() && ui.button("Import To Collection").clicked() {
                    app.import_paths(vec![f.clone()], None);
                    ui.close_menu();
                }
                if ui.button(crate::ui_browser::REVEAL_FILE).clicked() {
                    if let Err(err) = crate::ui_browser::reveal_path(std::path::Path::new(f)) {
                        app.toast_err(format!("Could not open the file location: {err}"));
                    }
                    ui.close_menu();
                }
            });
        }
    });
    app.rec.list_sel = sel;
}

// ----------------------------------------------------- Preferences > Controller > Recordings

/// A sub heading as the kit's, with its own letter spacing (this page's headings are set a
/// touch closer than those the kit was fitted to).
fn sub_t(ui: &mut Ui, text: &str, tracking: f32) {
    kit::row_at(ui, kit::Kind::Sub, 20.0, 57.0, |ui| {
        let r = ui.max_rect();
        kit::paint_text(ui, pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, text, inter_bold(15.75), WHITE.c(), tracking);
    });
    kit::indent(ui, 84.0);
}

/// Preferences > Controller > Recordings, laid out as rekordbox's (reference preferences/11).
pub fn prefs_page(app: &mut App, ui: &mut Ui) {
    let now = ui.input(|i| i.time);
    app.rec.prefs_seen = now;
    let mut changed = false;
    // (The card's heading is set a little wider than the kit's other headings: drawn here.)
    let top = ui.cursor().min;
    let mut title_ui = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
    card(ui, "", |ui| {
        kit::paint_text(&title_ui, pos2(top.x + 25.0, top.y + 31.0), Align2::LEFT_CENTER, "Recordings", inter_bold(17.0), WHITE.c(), 0.6);
        let _ = &mut title_ui;
        // ---- Location of Files
        sub_t(ui, "Location of Files", 0.12);
        let dir = rec_dir(&app.prefs.rec).to_string_lossy().to_string();
        let n = dir.chars().count();
        let shown = if n > 38 { format!("…{}", dir.chars().skip(n - 37).collect::<String>()) } else { dir.clone() };
        kit::combo_row(ui, 22.0, |ui| {
            kit::combo("rec_dir", 356.0, shown, 14.0).show_ui(ui, |ui| {
                let def = default_dir().to_string_lossy().to_string();
                let mut list = vec![def.clone()];
                list.extend(app.prefs.rec.dirs.iter().filter(|d| **d != def).cloned());
                for d in list {
                    if ui.selectable_label(d == dir, &d).clicked() {
                        app.prefs.rec.dir = if d == def { String::new() } else { d };
                        changed = true;
                    }
                }
            });
            ui.add_space(4.0);
            if ui.add(btn("Browse").exact(vec2(100.0, 22.0))).clicked() {
                if let Some(d) = rfd::FileDialog::new().set_title("Location of Files").set_directory(rec_dir(&app.prefs.rec)).pick_folder() {
                    if forbidden(&d) {
                        app.toast_err("recordbox does not save into rekordbox's own folders. Choose another folder.");
                    } else {
                        let s = d.to_string_lossy().to_string();
                        if !app.prefs.rec.dirs.contains(&s) {
                            app.prefs.rec.dirs.push(s.clone());
                        }
                        app.prefs.rec.dir = s;
                        changed = true;
                    }
                }
            }
        });
        separator(ui);
        // ---- Silence Level Threshold: the value, the slider, and the level of what would
        // be recorded under it.
        let sub_y = ui.cursor().min.y + 13.5 + 10.0;
        sub_t(ui, "Silence Level Threshold", 0.12);
        let p = ui.painter().clone();
        let hr = Rect::from_min_size(pos2(top.x + 327.0, sub_y - 7.0), vec2(14.0, 12.0));
        let hresp = ui.interact(hr, ui.id().with("rec_silence_help"), Sense::hover());
        p.rect_stroke(hr, 1.5, Stroke::new(1.0_f32, crate::ui_feel::hl(&hresp, gy(0x4A4A4A), gy(0x8A8A8A))), egui::StrokeKind::Inside);
        p.text(pos2(hr.center().x, hr.center().y + 0.25), Align2::CENTER_CENTER, "?", inter_bold(9.5), crate::ui_feel::hl(&hresp, gy(0x8A8A8A), WHITE.c()));
        let _ = hresp.on_hover_text(HELP_SILENCE);
        ui.add_space(13.5);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        let o = row.min;
        let vr = Rect::from_min_size(pos2(o.x + 84.0, o.y), vec2(100.0, 20.0));
        p.rect_filled(vr, 0.0, ON_ACCENT);
        p.text(vr.center(), Align2::CENTER_CENTER, format!("{}dB", app.prefs.rec.silence_db.round() as i32), inter(13.5), Color32::BLACK);
        let tr = Rect::from_min_size(pos2(o.x + 200.0, o.y), vec2(398.0, 20.0));
        let resp = ui.interact(tr, ui.id().with("rec_silence"), Sense::click_and_drag());
        let handle = 5.0;
        if resp.double_clicked() || (resp.clicked() && ui.input(|i| i.modifiers.alt)) {
            app.prefs.rec.silence_db = -38.0;
            changed = true;
        } else if let Some(pp) = resp.interact_pointer_pos() {
            let t = ((pp.x - tr.min.x - handle / 2.0 - 2.5) / (tr.width() - handle)).clamp(0.0, 1.0);
            let v = (-78.0 + 72.0 * t).round();
            if v != app.prefs.rec.silence_db {
                app.prefs.rec.silence_db = v;
                changed = true;
            }
        }
        p.rect_filled(Rect::from_min_max(pos2(tr.min.x, tr.center().y - 2.0), pos2(tr.max.x, tr.center().y + 2.0)), 0.0, gy(0x0F0F0F));
        let hx = tr.min.x + (app.prefs.rec.silence_db + 78.0) / 72.0 * (tr.width() - handle) + 2.5;
        p.rect_filled(Rect::from_min_size(pos2(hx, tr.min.y), vec2(handle, 20.0)), 0.5, gy(0xE6E6E6));
        ui.add_space(16.0);
        // The live level, -78 dB at the left end and -6 dB at the right: two rows 300 x 15.
        let (mr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 45.0), Sense::hover());
        let m = mr.min + vec2(164.0, 0.0);
        let e = rec::engine();
        let dt = (now - app.rec.noise_at) as f32;
        if dt >= 0.036 {
            app.rec.noise_at = now;
            let pk = e.take_peak(rec::METER_PREFS);
            for c in 0..2 {
                let db = rec::to_db(pk[c]);
                app.rec.noise[c] = db.max(app.rec.noise[c] - 2.0 * dt.min(0.5) / 0.036).max(-120.0);
                if db >= app.rec.noise_hold[c].0 || now > app.rec.noise_hold[c].1 {
                    app.rec.noise_hold[c] = (db, now + 1.0);
                }
            }
        }
        for c in 0..2 {
            let br = Rect::from_min_size(m + vec2(0.0, 15.0 * c as f32), vec2(300.0, 15.0));
            // (rekordbox draws each row 280 x 12 with a 1 px black frame and stretches it.)
            p.rect_filled(br, 0.0, Color32::BLACK);
            p.rect_filled(Rect::from_min_max(br.min + vec2(1.3, 1.25), br.max - vec2(1.0, 1.0)), 0.0, WHITE);
            let x = |db: f32| br.min.x + br.width() * ((db.clamp(-78.0, -6.0) + 78.0) / 72.0);
            if app.rec.noise[c] > -78.0 {
                p.rect_filled(Rect::from_min_max(br.min, pos2(x(app.rec.noise[c]), br.max.y)), 0.0, Color32::BLACK);
            }
            if app.rec.noise_hold[c].0 > -78.0 {
                let hx = x(app.rec.noise_hold[c].0);
                p.rect_filled(Rect::from_min_max(pos2(hx, br.min.y), pos2((hx + 2.0).min(br.max.x), br.max.y)), 0.0, Color32::BLACK);
            }
        }
        p.text(m + vec2(5.0, 37.5), Align2::LEFT_CENTER, "-78dB", inter(12.0), WHITE.c());
        p.text(m + vec2(294.5, 37.5), Align2::RIGHT_CENTER, "-6dB", inter(12.0), WHITE.c());
        if app.rec.noise.iter().any(|d| *d > -100.0) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(36));
        }
        ui.add_space(2.5);
        separator(ui);
        ui.add_space(0.5);
        // ---- start and end
        sub_t(ui, "Recording starts when:", 0.07);
        ui.add_space(-0.5);
        if radio(ui, !app.prefs.rec.start_on_audio, "Pressing the REC button.") {
            app.prefs.rec.start_on_audio = false;
            changed = true;
        }
        if radio(ui, app.prefs.rec.start_on_audio, "Detecting audio input.") {
            app.prefs.rec.start_on_audio = true;
            changed = true;
        }
        separator(ui);
        ui.add_space(0.5);
        sub_t(ui, "Recording ends when:", 0.07);
        ui.add_space(-0.5);
        if radio(ui, !app.prefs.rec.stop_on_silence, "Pressing the REC button.") {
            app.prefs.rec.stop_on_silence = false;
            changed = true;
        }
        if radio(ui, app.prefs.rec.stop_on_silence, "Detecting 20 sec. of silence.") {
            app.prefs.rec.stop_on_silence = true;
            changed = true;
        }
        separator(ui);
        ui.add_space(-0.5);
        // ---- the switches
        let perf = app.performance();
        let split = if perf { &mut app.prefs.rec.auto_split } else { &mut app.prefs.rec.auto_split_export };
        changed |= toggle(ui, split, "Automatically separate files when silence continues.");
        ui.add_space(6.0);
        kit::row_at(ui, kit::Kind::Ctl, 22.0, 55.0, |ui| {
            ui.spacing_mut().interact_size.y = 22.0;
            let (lr, _) = ui.allocate_exact_size(vec2(345.0, 22.0), Sense::hover());
            kit::paint_text(ui, pos2(lr.min.x, lr.center().y), Align2::LEFT_CENTER, "When silence continues for:", inter(14.0), WHITE.c(), -0.21);
            let s = app.prefs.rec.silence_secs.clamp(1, 10);
            kit::combo("rec_silence_secs", 200.0, format!("{s}second"), 14.0).show_ui(ui, |ui| {
                for k in 1..=10u8 {
                    if ui.selectable_label(s == k, format!("{k}second")).clicked() {
                        app.prefs.rec.silence_secs = k;
                        changed = true;
                    }
                }
            });
        });
        separator(ui);
        ui.add_space(-0.5);
        changed |= toggle(ui, &mut app.prefs.rec.normalize, "Automatically optimize audio level of a recorded file.");
        separator(ui);
        ui.add_space(-0.5);
        changed |= toggle(ui, &mut app.prefs.rec.auto_import, "Automatically import a recorded file.");
        separator(ui);
        ui.add_space(-0.5);
        changed |= toggle(ui, &mut app.prefs.rec.tag_window, "Automatically display tag information window\nof a recorded file.");
    });
    if changed {
        app.save_prefs();
    }
}

// ----------------------------------------------------------------- screenshots and the check

/// `PREPDECK_SHOT_VIEW=perfrec` (PERFORMANCE with the recording panel) and `exportrec`
/// (EXPORT with its strip).
pub fn shot_view(app: &mut App, name: &str) {
    match name {
        "perfrec" => {
            app.prefs.app_mode = 1;
            app.prefs.perf_four = false;
            app.prefs.perf_fx = false;
            app.prefs.perf_sampler = false;
            // (The scripted run also checks the import into the collection.)
            let run = std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.contains("rectest")).unwrap_or(false);
            app.prefs.rec = RecPrefs { panel: true, auto_import: run, ..Default::default() };
        }
        _ => {
            app.prefs.app_mode = 0;
            app.prefs.rec = RecPrefs { panel_export: true, ..Default::default() };
        }
    }
}

impl App {
    /// `PREPDECK_SHOT_PLAYER=rectest` (with --screenshot, `PREPDECK_SHOT_VIEW=perfrec` and
    /// `PREPDECK_BENCH=<frames>`): works the recording panel with real pointer and key events
    /// while deck 1 plays, records twice, and checks the files against what was played.
    pub fn shot_rec_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        if !std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "rectest")).unwrap_or(false) {
            return;
        }
        // The panel's place in the 1512 pt screenshot window: the bottom row starts 28 (top
        // bar) + 143 (waveforms) + 225 (decks) + 1.5 pt down, plus the rows that are showing.
        let extra = if self.prefs.show_stems { 21.0 } else { 0.0 } + if self.prefs.show_phrase_bar { crate::ui_perf::PHRASE_ROW } else { 0.0 };
        let top = 28.0 + self.performance_height() - 43.0 + 1.5 - if self.prefs.perf_sampler { crate::ui_sampler::SAMPLER_H } else { 0.0 };
        let _ = extra;
        let o = pos2(1512.0 - 10.0 - 344.0, top);
        let rec_btn = o + vec2(227.5, 18.5);
        let knob = o + vec2(110.0, 22.0);
        let c = pos2(756.0, 470.0);
        // (name, from, to): a click where the two are the same, else a drag.
        let steps: Vec<(&str, Pos2, Pos2)> = vec![
            ("Rec panel button in the top bar (hides the panel)", pos2(353.0, 14.0), pos2(353.0, 14.0)),
            ("Rec panel button again (shows it)", pos2(353.0, 14.0), pos2(353.0, 14.0)),
            ("level knob up", knob, knob + vec2(0.0, -30.0)),
            ("DBL level knob (back to the centre)", knob, knob),
            ("source selector (opens)", o + vec2(40.0, 19.0), o + vec2(40.0, 19.0)),
            ("Master Out in the list", o + vec2(40.0, 19.0 + 22.0), o + vec2(40.0, 19.0 + 22.0)),
            ("REC (Detecting audio input: deck 1 plays, so it starts with the second before)", rec_btn, rec_btn),
            ("wait 1", c, c),
            ("wait 2", c, c),
            ("wait 3", c, c),
            ("wait 4", c, c),
            ("QUIT:close|the window's close button while recording (asks, the recording goes on)", c, c),
            ("QUIT:No|No (keeps recording)", c, c),
            ("REC (stop: the tag information window opens)", rec_btn, rec_btn),
            ("wait 5", c, c),
            ("TAGS:Mix Title field", c, c),
            ("TAGS:OK", c, c),
            ("wait 6 (the file is filed, tagged and imported)", c, c),
            ("wait 7", c, c),
            ("Recordings in the icon strip", c, c),
            ("wait 8", c, c),
            ("CHECK:first", c, c),
            ("PREFS:REC press, stop on press, no tag window", c, c),
            ("REC (Pressing the REC button: starts at once)", rec_btn, rec_btn),
            ("wait 9", c, c),
            ("wait 10", c, c),
            ("KEY:Cmd+Enter|Start/Pause REC (stops)", c, c),
            ("wait 11", c, c),
            ("CHECK:second", c, c),
            ("PREFS2:optimize level on (the level window runs)", c, c),
            ("REC (a third recording)", rec_btn, rec_btn),
            ("wait 12", c, c),
            ("wait 13", c, c),
            ("wait 14", c, c),
            ("STOPSPLIT:stop, with a silence split point half way (the separate-files window asks)", c, c),
            ("SPLITYES:Yes in the separate-files window", c, c),
            ("wait 15 (separating, then optimizing the level)", c, c),
            ("wait 16", c, c),
            ("wait 17", c, c),
            ("CHECK:third", c, c),
        ];
        const START: u32 = 150;
        const SPAN: u32 = 40;
        if frame < START {
            return;
        }
        let i = ((frame - START) / SPAN) as usize;
        let f = (frame - START) % SPAN;
        let Some((name, a, b)) = steps.get(i).copied() else { return };
        let say = |app: &App, name: &str| eprintln!("rectest: {name}: {}", app.rec_state());
        if name.starts_with("wait") {
            // (A wait lasts about 0.7 s whatever the screen's refresh rate.)
            std::thread::sleep(std::time::Duration::from_millis(17));
            if f == 0 {
                raw.events.push(egui::Event::PointerMoved(a));
            }
            if f == 30 {
                say(self, name);
            }
            return;
        }
        if let Some(rest) = name.strip_prefix("KEY:") {
            let combo = rest.split('|').next().unwrap_or("");
            if let Some(sc) = Shortcut::parse(combo) {
                let modifiers = egui::Modifiers { alt: sc.alt, ctrl: sc.ctrl, shift: sc.shift, mac_cmd: sc.cmd, command: sc.cmd };
                let key = |pressed: bool| egui::Event::Key { key: sc.key, physical_key: None, pressed, repeat: false, modifiers };
                match f {
                    3 => {
                        raw.modifiers = modifiers;
                        raw.events.push(key(true));
                    }
                    4..=11 => raw.modifiers = modifiers,
                    12 => raw.events.push(key(false)),
                    30 => say(self, name),
                    _ => {}
                }
            }
            return;
        }
        if let Some(rest) = name.strip_prefix("QUIT:") {
            let what = rest.split('|').next().unwrap_or("");
            match (what, f) {
                ("close", 3) => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                ("No", 0) => {
                    if let Some((_, no)) = self.rec.quit_pts {
                        raw.events.push(egui::Event::PointerMoved(no));
                    }
                }
                ("No", 3) | ("No", 5) => {
                    if let Some((_, no)) = self.rec.quit_pts {
                        raw.events.push(egui::Event::PointerButton { pos: no, button: egui::PointerButton::Primary, pressed: f == 3, modifiers: Default::default() });
                    }
                }
                (_, 30) => eprintln!("rectest: {name}: asking {} | {}", self.rec.quit_ask, self.rec_state()),
                _ => {}
            }
            return;
        }
        if name.starts_with("TAGS:") {
            // The tag information window sits in the middle of the window: 600 x 400.
            let (field, ok) = self.rec.tag_pts.unwrap_or((a, a));
            let (pt, typed) = if name.ends_with("OK") { (ok, None) } else { (field, Some("Friday Warm Up")) };
            let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            match f {
                0 => raw.events.push(egui::Event::PointerMoved(pt)),
                3 => raw.events.push(button(pt, true)),
                5 => raw.events.push(button(pt, false)),
                8 if typed.is_some() => {
                    raw.modifiers = egui::Modifiers::COMMAND | egui::Modifiers::MAC_CMD;
                    raw.events.push(egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: raw.modifiers });
                }
                11 if typed.is_some() => raw.events.push(egui::Event::Text(typed.unwrap().to_string())),
                30 => {
                    let fields = self.rec.posts.front().map(|p| p.fields.clone());
                    eprintln!("rectest: {name}: fields {fields:?} | {}", self.rec_state());
                }
                _ => {}
            }
            return;
        }
        if name.starts_with("Recordings in the icon strip") {
            if f == 3 {
                // (Its place depends on which sources are switched on; the tile's own code runs.)
                self.side = SideMode::Recordings;
                let root = rec_dir(&self.prefs.rec).to_string_lossy().to_string();
                self.set_view(View::Explorer(self.rec.last.first().and_then(|p| p.parent()).map(|p| p.to_string_lossy().to_string()).unwrap_or(root)));
            }
            if f == 30 {
                say(self, name);
            }
            return;
        }
        if name.starts_with("PREFS2:") {
            if f == 3 {
                self.prefs.rec.normalize = true;
            }
            if f == 30 {
                say(self, name);
            }
            return;
        }
        if name.starts_with("STOPSPLIT:") {
            if f == 3 {
                self.rec_stop();
                // (A silence split point needs 20 s of quiet; the run puts one half way.)
                if let Some(p) = self.rec.posts.back_mut() {
                    let half = p.files.first().map(|f| f.1 / 2.0).unwrap_or(0.0);
                    p.splits = vec![half];
                    p.stage = Stage::AskSplit;
                }
            }
            if f == 30 {
                say(self, name);
            }
            return;
        }
        if name.starts_with("SPLITYES:") {
            let yes = self.rec.split_o.map(|o| o + vec2(240.0, 360.0));
            if let Some(y) = yes {
                match f {
                    0 => raw.events.push(egui::Event::PointerMoved(y)),
                    3 | 5 => raw.events.push(egui::Event::PointerButton { pos: y, button: egui::PointerButton::Primary, pressed: f == 3, modifiers: Default::default() }),
                    _ => {}
                }
            }
            if f == 30 {
                say(self, name);
            }
            return;
        }
        if name.starts_with("PREFS:") {
            if f == 3 {
                self.prefs.rec.start_on_audio = false;
                self.prefs.rec.stop_on_silence = false;
                self.prefs.rec.tag_window = false;
            }
            if f == 30 {
                say(self, name);
            }
            return;
        }
        if name.starts_with("CHECK:") {
            if f == 20 {
                self.rec_check(name);
            }
            return;
        }
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        match f {
            0 => raw.events.push(egui::Event::PointerMoved(a)),
            3 => {
                raw.events.push(button(a, true));
                if name.starts_with("DBL") {
                    raw.events.push(button(a, false));
                }
            }
            6 if name.starts_with("DBL") => {
                raw.events.push(button(a, true));
                raw.events.push(button(a, false));
            }
            4..=11 => {
                if a != b {
                    raw.events.push(egui::Event::PointerMoved(a + (b - a) * ((f - 3) as f32 / 8.0)));
                }
            }
            12 => {
                if !name.starts_with("DBL") {
                    raw.events.push(button(b, false));
                }
            }
            30 => say(self, name),
            _ => {}
        }
    }

    fn rec_state(&self) -> String {
        let e = rec::engine();
        let stage = self.rec.posts.front().map(|p| match p.stage {
            Stage::AskSplit => "separate?",
            Stage::Normalize => "level",
            Stage::Working => "working",
            Stage::Tags => "tag window",
            Stage::Finish => "finishing",
        });
        let playing: Vec<bool> = self.player.decks.iter().take(2).map(|d| d.lock().playing).collect();
        format!(
            "panel {} | level knob {:+.2} ({:+.2} dB) | armed {} recording {} elapsed {:.2} s | meter {:.1} / {:.1} dB | time left {} | on-air log {:?} | after: {:?} | last {:?} | side {:?} view {:?} | decks playing {:?} muted {}",
            self.rec_panel_shown(),
            self.rec_level(),
            rec::to_db(rec::knob_gain(self.rec_level(), self.rec_base_db())),
            self.rec.armed,
            self.rec.session.is_some(),
            if self.rec.session.is_some() { e.elapsed() } else { 0.0 },
            self.rec.shown_db[0],
            self.rec.shown_db[1],
            rec::hms(rec::remaining_secs(if self.rec.session.is_some() { e.elapsed() as u64 } else { 0 }, self.rec.free)),
            self.rec.log.tracks.iter().map(|t| (t.title.clone(), t.at_ms)).collect::<Vec<_>>(),
            stage,
            self.rec.last.iter().map(|p| p.strip_prefix(rec_dir(&self.prefs.rec)).unwrap_or(p).to_string_lossy().to_string()).collect::<Vec<_>>(),
            self.side,
            match &self.view {
                View::Explorer(d) => format!("Explorer({})", Path::new(d).strip_prefix(rec_dir(&self.prefs.rec)).map(|p| format!("Recordings/{}", p.to_string_lossy())).unwrap_or_else(|_| d.clone())),
                other => format!("{other:?}").chars().take(40).collect(),
            },
            playing,
            self.px.muted,
        )
    }

    /// Read the file of the last recording back and compare it with what went in.
    fn rec_check(&self, name: &str) {
        let Some(f) = self.rec.last.first() else {
            eprintln!("rectest: {name}: NO FILE");
            return;
        };
        let info = rec::wav_info(f);
        let Ok((rate, ch, bits, data, bytes)) = info else {
            eprintln!("rectest: {name}: {} is not a WAV file", f.display());
            return;
        };
        let all = std::fs::read(f).unwrap_or_default();
        let samples: Vec<i16> = all[data as usize..((data + bytes) as usize).min(all.len())].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let frames = samples.len() / 2;
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0) as f32 / 32768.0;
        let fed = self.rec.peak.min(1.0);
        let tags = rec::read_tags(f);
        let cue = std::fs::read_to_string(f.with_extension("cue")).unwrap_or_default();
        let path = f.to_string_lossy().to_string();
        let in_collection = self.tracks.iter().any(|t| t.path == path);
        let root = rec_dir(&self.prefs.rec);
        let tree: Vec<String> = sub_dirs(&root).iter().flat_map(|a| { let an = a.file_name().unwrap_or_default().to_string_lossy().to_string(); let mut v = vec![an.clone()]; v.extend(sub_dirs(a).iter().map(|b| format!("{an}/{} ({} files)", b.file_name().unwrap_or_default().to_string_lossy(), wav_files(b).len()))); v }).collect();
        let loose: Vec<String> = wav_files(&root).iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string()).collect();
        let leftovers = std::fs::read_dir(f.parent().unwrap_or(&root)).map(|rd| rd.flatten().filter(|e| e.path().extension().map(|x| x == "rbrec").unwrap_or(false)).count()).unwrap_or(0) + std::fs::read_dir(&root).map(|rd| rd.flatten().filter(|e| e.path().extension().map(|x| x == "rbrec").unwrap_or(false)).count()).unwrap_or(0);
        eprintln!(
            "rectest: {name}: file {} | {rate} Hz {ch} ch {bits} bit, header {data} bytes, {frames} frames = {:.3} s (writer reported {} frames, {} dropped) | file peak {:.4} ({:.2} dB), highest level fed to it {:.4} ({:.2} dB), difference {:.5} | tags {:?} | in collection {} | Recordings tree {:?}, loose files {:?} | .rbrec left {} | cue sheet:\n{}",
            f.strip_prefix(&root).unwrap_or(f).display(),
            frames as f64 / rate.max(1) as f64,
            self.rec.last_frames,
            self.rec.last_dropped,
            peak,
            rec::to_db(peak),
            fed,
            rec::to_db(fed),
            (peak - fed).abs(),
            tags,
            in_collection,
            tree,
            loose,
            leftovers,
            cue.replace("\r\n", "\\r\\n\n").replace('\t', "\\t"),
        );
        if name.contains("third") {
            // Separated in two at the split point, then each file's level optimized: the two
            // files' frames add up to what was recorded and the level went up.
            let parts: Vec<u64> = self.rec.last.iter().filter_map(|p| rec::wav_info(p).ok()).map(|(_, ch, bits, _, bytes)| bytes as u64 / (ch.max(1) as u64 * (bits.max(8) / 8) as u64)).collect();
            let sum: u64 = parts.iter().sum();
            let ok = parts.len() == 2 && sum == self.rec.last_frames && peak > fed * 1.5;
            eprintln!("rectest: {name}: files {} frames {:?} (sum {sum}, recorded {}) | level: fed {:.4}, first file's peak after optimizing {:.4} | {}", parts.len(), parts, self.rec.last_frames, fed, peak, if ok { "PASS (separated at the split point and the level optimized)" } else { "FAIL" });
            return;
        }
        let ok = rate == 44100 && ch == 2 && bits == 16 && data == rec::WAV_HEADER && frames as u64 == self.rec.last_frames && self.rec.last_dropped == 0 && (peak - fed).abs() < 0.0005 && frames > 44100;
        eprintln!("rectest: {name}: {}", if ok { "PASS (format, length and peak agree with what was recorded)" } else { "FAIL" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_have_rekordboxs_defaults_and_survive_a_round_trip() {
        let p = RecPrefs::default();
        assert_eq!((p.silence_db, p.start_on_audio, p.stop_on_silence, p.auto_split, p.auto_split_export, p.silence_secs), (-38.0, true, true, false, true, 3));
        assert_eq!((p.normalize, p.auto_import, p.tag_window), (false, false, true));
        let mut q = p.clone();
        q.dir = "/tmp/rec".into();
        q.level = 0.5;
        q.panel = true;
        let back: RecPrefs = serde_json::from_str(&serde_json::to_string(&q).unwrap()).unwrap();
        assert_eq!((back.dir.as_str(), back.level, back.panel), ("/tmp/rec", 0.5, true));
        // Settings files from before recording existed still load.
        let old: RecPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(old.silence_db, -38.0);
    }

    #[test]
    fn recordings_never_go_into_rekordboxs_folders() {
        let home = dirs::home_dir().unwrap();
        let mut p = RecPrefs::default();
        assert_eq!(rec_dir(&p), default_dir());
        assert!(default_dir().ends_with("Recordings"));
        p.dir = home.join("Music/rekordbox/Recording").to_string_lossy().to_string();
        assert_eq!(rec_dir(&p), default_dir(), "rekordbox's own recording folder is refused");
        p.dir = home.join("Library/Pioneer/rekordbox").to_string_lossy().to_string();
        assert_eq!(rec_dir(&p), default_dir());
        p.dir = "/tmp/my recordings".into();
        assert_eq!(rec_dir(&p), PathBuf::from("/tmp/my recordings"));
    }

    #[test]
    fn tagged_recordings_are_filed_under_artist_and_album() {
        let d = std::env::temp_dir().join(format!("recordbox-rec-ui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let make = |name: &str| {
            let p = d.join(name);
            let mut w = rec::WavWriter::create(&p, 44100).unwrap();
            w.write(&[0.1; 2000]).unwrap();
            w.finish().unwrap();
            p
        };
        let post = |files: Vec<PathBuf>, tags: Vec<TagInfo>| Post { files: files.into_iter().map(|f| (f, 1000.0)).collect(), splits: vec![], tracks: vec![], export_mode: false, stage: Stage::Tags, job: None, edited: vec![true; tags.len()], tags, cur: 0, fields: Default::default(), fields_for: None, confirm_delete: false, tagged: false, page: 0 };
        let t = |artist: &str, album: &str, title: &str, n: u32| TagInfo { artist: artist.into(), album: album.into(), title: title.into(), track_no: n };
        let mut one = post(vec![make("REC-2026-11-27.wav")], vec![t("DJ A", "Friday", "Warm Up", 1)]);
        file_tagged(&mut one).unwrap();
        assert_eq!(one.files[0].0, d.join("DJ A/Friday/01 Warm Up.wav"));
        assert!(one.files[0].0.is_file() && !d.join("REC-2026-11-27.wav").exists());
        assert_eq!(rec::read_tags(&one.files[0].0), t("DJ A", "Friday", "Warm Up", 1));
        // The same album again gets a folder of its own, as rekordbox makes one.
        let mut again = post(vec![make("REC-2026-11-27.wav")], vec![t("DJ A", "Friday", "Warm Up", 1)]);
        file_tagged(&mut again).unwrap();
        assert_eq!(again.files[0].0, d.join("DJ A/Friday (2)/01 Warm Up.wav"));
        // Separated files with different artists go under Compilations, numbered.
        let mut two = post(vec![make("REC-2026-11-27-01.wav"), make("REC-2026-11-27-02.wav")], vec![t("A", "Tape", "", 1), t("B", "Tape", "Side B", 2)]);
        file_tagged(&mut two).unwrap();
        assert_eq!(two.files[0].0, d.join("Compilations/Tape/01 No Title.wav"));
        assert_eq!(two.files[1].0, d.join("Compilations/Tape/02 Side B.wav"));
        assert_eq!(sub_dirs(&d).iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>(), vec!["Compilations", "DJ A"]);
        assert_eq!(wav_files(&d.join("DJ A/Friday")).len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn indicator_colours_run_from_green_to_red() {
        let c = segment_colours();
        assert_eq!(c.len(), 88);
        assert!(c[0].g() > c[0].r() && c[87].r() > c[87].g(), "{:?} {:?}", c[0], c[87]);
    }
}
