//! Device-side features of Export mode: importing from a device into the collection, deleting
//! from a device, the Sync Manager, the device Property window, My Settings import/delete and
//! dropping tracks or playlists on a device. Everything that writes to a drive goes through
//! the journaled writer in export/usb.rs.

use crate::app::*;
use crate::export::device_import::{self, DeviceProps, ImportedTrack, SyncConfig};
use crate::export::usb::{self, ExportNode};
use crate::model::*;
use crate::theme::*;
use eframe::egui;
use egui::{Context, RichText, Ui};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A playlist (or folder) to create in the collection once a device import has finished.
#[derive(Clone)]
pub struct ImportList {
    pub name: String,
    pub folder: bool,
    /// Index of the parent in the plan (None: top level).
    pub parent: Option<usize>,
    pub tracks: Vec<u32>,
}

pub enum ImportMsg {
    Track(Box<ImportedTrack>),
    Skipped(String),
    Done,
}

/// An import from a device that is running (files are copied off the window's thread).
pub struct DeviceImport {
    pub rx: crossbeam_channel::Receiver<ImportMsg>,
    pub device: String,
    pub total: usize,
    pub done: usize,
    pub added: Vec<i64>,
    /// Device track ID -> collection track ID.
    pub map: HashMap<u32, i64>,
    pub lists: Vec<ImportList>,
    pub skipped: Vec<String>,
}

/// The device Property window.
pub struct PropertyState {
    pub root: String,
    pub tab: usize,
    pub name: String,
    pub settings: crate::export::settings::UsbSettings,
    pub browse: usb::UsbBrowse,
    pub colors: [String; 8],
    pub bg: u8,
    /// What the window opened with, to tell whether Apply has anything to write.
    pub opened: (String, crate::export::settings::UsbSettings, usb::UsbBrowse, [String; 8], u8),
}

/// The Sync Manager window.
pub struct SyncState {
    pub root: String,
    pub cfg: SyncConfig,
    pub saved: SyncConfig,
    /// Asked: "This device is synchronized with another computer."
    pub other_computer: bool,
}

/// State of the device windows.
#[derive(Default)]
pub struct DeviceUi {
    /// Selection in the device track list: the device it belongs to and the track IDs.
    pub sel_root: String,
    pub sel: BTreeSet<u32>,
    pub anchor: Option<usize>,
    pub import: Option<DeviceImport>,
    pub property: Option<PropertyState>,
    pub sync: Option<SyncState>,
    pub one_inspector: crate::onelibrary::Inspector,
    /// Devices whose automatic synchronisation was offered since they were connected.
    pub auto_offered: HashSet<String>,
    /// "The selected lists are synchronized automatically at each connection." waiting for an answer.
    pub auto_prompt: Option<String>,
    /// Tracks waiting for "Delete Track(s)" confirmation: (root, device track IDs).
    pub confirm_delete: Option<(String, Vec<u32>)>,
    /// "Do you want to delete My Settings file in the device?"
    pub confirm_delete_settings: Option<String>,
    /// Export window: export the ticked playlists' tracks without creating the playlists.
    pub export_tracks_only: bool,
    /// Options for the next export to this root (a synchronisation): lists to remove, whether
    /// their unused tracks go too, and whether device cues are kept.
    pub pending: Option<(String, Vec<u32>, bool, bool)>,
    /// Device name to write with the next export to this root.
    pub pending_name: Option<(String, String)>,
    /// Background Color of each connected device (from its /.Recordbox/props.json).
    pub bg: HashMap<String, u8>,
    /// Hot Cue Bank List window (export/hotcue_bank.rs).
    pub banks: crate::export::hotcue_bank::BankUi,
}

/// Library playlists and folders ticked for a device, as the export tree: each ticked item
/// under copies of its folders, so the device mirrors the library's structure. Returns the
/// tree and every library ID in it.
pub fn sync_tree(app: &App, cfg: &SyncConfig) -> (Vec<ExportNode>, HashSet<i64>) {
    let ticked: HashSet<i64> = if cfg.all { app.nodes.iter().filter(|n| n.parent == 0).map(|n| n.id).collect() } else { cfg.playlists.iter().copied().filter(|id| app.node(*id).is_some()).collect() };
    fn build(app: &App, id: i64) -> Option<ExportNode> {
        let n = app.node(id)?;
        Some(match n.kind {
            NodeKind::Folder => ExportNode { id, name: n.name.clone(), folder: true, children: app.nodes.iter().filter(|c| c.parent == id).filter_map(|c| build(app, c.id)).collect(), tracks: vec![] },
            NodeKind::Smart => ExportNode { id, name: n.name.clone(), folder: false, children: vec![], tracks: n.smart.as_ref().map(|r| app.smart_tracks(r)).unwrap_or_default() },
            NodeKind::Playlist => ExportNode { id, name: n.name.clone(), folder: false, children: vec![], tracks: app.db.playlist_tracks(id).unwrap_or_default() },
        })
    }
    fn insert(tree: &mut Vec<ExportNode>, path: &[(i64, String)], leaf: ExportNode) {
        match path.split_first() {
            None => {
                if !tree.iter().any(|n| n.id == leaf.id) {
                    tree.push(leaf);
                }
            }
            Some(((id, name), rest)) => {
                let i = match tree.iter().position(|n| n.id == *id) {
                    Some(i) => i,
                    None => {
                        tree.push(ExportNode { id: *id, name: name.clone(), folder: true, children: vec![], tracks: vec![] });
                        tree.len() - 1
                    }
                };
                insert(&mut tree[i].children, rest, leaf);
            }
        }
    }
    let mut tree = vec![];
    // Library order, so the device lists follow it.
    let order: Vec<i64> = app.nodes.iter().map(|n| n.id).filter(|id| ticked.contains(id)).collect();
    for id in order {
        // Covered by a ticked folder above it?
        let mut ancestors = vec![];
        let mut p = app.node(id).map(|n| n.parent).unwrap_or(0);
        let mut covered = false;
        while p != 0 {
            if ticked.contains(&p) {
                covered = true;
                break;
            }
            let Some(n) = app.node(p) else { break };
            ancestors.push((n.id, n.name.clone()));
            p = n.parent;
        }
        if covered {
            continue;
        }
        ancestors.reverse();
        if let Some(leaf) = build(app, id) {
            insert(&mut tree, &ancestors, leaf);
        }
    }
    fn ids(n: &ExportNode, out: &mut HashSet<i64>) {
        out.insert(n.id);
        n.children.iter().for_each(|c| ids(c, out));
    }
    let mut all = HashSet::new();
    tree.iter().for_each(|n| ids(n, &mut all));
    (tree, all)
}

/// Device lists a synchronisation removes: the ones Recordbox itself put there for a library
/// playlist (per the drive's record) that is no longer part of the selection. Lists made by
/// rekordbox, by a player or by another computer are never touched.
pub fn sync_removals(record: &HashMap<i64, u32>, keep: &HashSet<i64>, lib: &usb::DeviceLibrary) -> Vec<u32> {
    let mut gone: Vec<u32> = record.iter().filter(|(lib_id, dev)| !keep.contains(lib_id) && lib.nodes.iter().any(|n| n.id == **dev)).map(|(_, dev)| *dev).collect();
    // A folder that still holds something Recordbox did not put there stays (its own lists go).
    let recorded: HashSet<u32> = record.values().copied().collect();
    fn foreign_inside(lib: &usb::DeviceLibrary, id: u32, recorded: &HashSet<u32>, gone: &[u32]) -> bool {
        lib.nodes.iter().filter(|n| n.parent == id).any(|n| !recorded.contains(&n.id) || !gone.contains(&n.id) || foreign_inside(lib, n.id, recorded, gone))
    }
    let all = gone.clone();
    gone.retain(|id| !foreign_inside(lib, *id, &recorded, &all));
    gone.sort_unstable();
    gone
}

impl App {
    /// The parts of an export request that depend on the drive and on Preferences: its
    /// filesystem, the colour names for its colour table, and what a pending
    /// synchronisation asked for.
    pub fn export_options(&mut self, root: &str) -> usb::ExportRequest {
        let fs = self.devices.iter().find(|d| d.root == root).map(|d| d.fs.clone()).unwrap_or_default();
        let color_names: Vec<String> = (1..=8u8).map(|i| self.prefs.color_name(i)).collect();
        let mut req = usb::ExportRequest { fs, color_names: Some(color_names), ..Default::default() };
        if let Some((r, remove, delete, keep_cues)) = self.device_ui.pending.take() {
            if r == root {
                req.remove_nodes = remove;
                req.delete_unreferenced = delete;
                req.keep_device_cues = keep_cues;
                req.recopy_changed = true;
            }
        }
        if let Some((r, name)) = self.device_ui.pending_name.take() {
            if r == root {
                req.device_name = Some(name);
            }
        }
        req
    }

    /// Folder device imports are copied into: Preferences' `device_import_dir`, else
    /// "Recordbox/Imported from Devices" in the Music folder (inside the data folder when
    /// Recordbox runs on a separate data folder, as the checks do).
    pub fn device_import_dir(&self) -> PathBuf {
        if !self.prefs.device_import_dir.trim().is_empty() {
            return PathBuf::from(self.prefs.device_import_dir.trim());
        }
        if std::env::var("PREPDECK_DATA").is_ok() {
            return crate::db::data_dir().join("Imported from Devices");
        }
        dirs::audio_dir().or_else(|| dirs::home_dir().map(|h| h.join("Music"))).unwrap_or_else(crate::db::data_dir).join("Recordbox").join("Imported from Devices")
    }

    /// "Import To Collection" for device tracks, and "Import as a playlist" when `lists`
    /// names playlists to create from them. Tracks Recordbox exported from a file that is
    /// still in the collection are not copied again; the rest are copied from the device with
    /// their cues, grid, analysis files, artwork and My Tags.
    pub fn import_from_device(&mut self, root: &str, ids: &[u32], lists: Vec<ImportList>) {
        if self.device_ui.import.is_some() {
            return self.toast_err("An import from a device is already running.");
        }
        let Some(dev) = self.devices.iter().find(|d| d.root == root) else { return self.toast_err("The device is no longer connected.") };
        let Some(lib) = dev.lib.clone() else { return self.toast_err("This device has no library.") };
        let device = dev.name.clone();
        let mut map = HashMap::new();
        let mut todo = vec![];
        let mut seen = HashSet::new();
        for id in ids {
            let Some(dt) = lib.tracks.get(id) else { continue };
            if !seen.insert(*id) {
                continue;
            }
            match (!dt.source.is_empty()).then(|| self.db.track_by_path(&dt.source)).flatten() {
                Some(tid) => {
                    map.insert(*id, tid);
                }
                None => todo.push(*id),
            }
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        let (music, data, art) = (self.device_import_dir(), crate::db::data_dir(), crate::db::artwork_dir());
        let (rootp, ctx, total) = (PathBuf::from(root), self.ctx.clone(), todo.len());
        std::thread::spawn(move || {
            for id in todo {
                let title = lib.tracks.get(&id).map(|t| t.title.clone()).unwrap_or_default();
                let msg = match device_import::import_track(&rootp, &lib, id, &music, &data, &art) {
                    Ok(t) => ImportMsg::Track(Box::new(t)),
                    Err(e) => ImportMsg::Skipped(format!("{title} ({e})")),
                };
                if tx.send(msg).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            let _ = tx.send(ImportMsg::Done);
            ctx.request_repaint();
        });
        self.device_ui.import = Some(DeviceImport { rx, device, total, done: 0, added: vec![], map, lists, skipped: vec![] });
    }

    /// Add one imported device track to the collection (or find it there already).
    fn add_imported(&mut self, it: ImportedTrack) -> Option<(i64, bool)> {
        if let Some(id) = self.db.track_by_path(&it.track.path) {
            self.db.protect_imported_track(id).ok()?;
            return Some((id, false));
        }
        let id = self.db.insert_track(&it.track).ok()?;
        if let Some(g) = &it.grid {
            let _ = self.db.save_grid(id, g);
        }
        let cues: Vec<Cue> = it.cues.iter().cloned().map(|mut c| { c.track_id = id; c.id = 0; c }).collect();
        if !cues.is_empty() {
            let _ = self.db.replace_cues(id, &cues);
        }
        // My Tags: categories and tags are created when the collection doesn't have them.
        for (cat, tag) in &it.tags {
            let existing = self.db.mytags().unwrap_or_default();
            let cat_name = if cat.is_empty() { "Device" } else { cat.as_str() };
            let cid = match existing.iter().find(|t| t.parent == 0 && t.name == cat_name) {
                Some(t) => t.id,
                None => self.db.add_mytag(0, cat_name).unwrap_or(0),
            };
            let tid = match existing.iter().find(|t| t.parent == cid && &t.name == tag) {
                Some(t) => t.id,
                None => self.db.add_mytag(cid, tag).unwrap_or(0),
            };
            if tid != 0 {
                let _ = self.db.set_track_tag(id, tid, true);
            }
        }
        if let Some(b) = &it.anlz_base {
            let mut m: HashMap<String, String> = self.db.setting("rb_anlz").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            m.insert(it.track.path.clone(), b.to_string_lossy().to_string());
            self.db.set_setting("rb_anlz", &serde_json::to_string(&m).unwrap_or_default());
        }
        Some((id, true))
    }

    /// Progress of device jobs; called every frame.
    pub fn poll_device_jobs(&mut self) {
        let Some(mut job) = self.device_ui.import.take() else { return };
        let mut finished = false;
        while let Ok(m) = job.rx.try_recv() {
            match m {
                ImportMsg::Track(t) => {
                    job.done += 1;
                    let dev_id = t.dev_id;
                    let title = t.track.title.clone();
                    match self.add_imported(*t) {
                        Some((id, fresh)) => {
                            job.map.insert(dev_id, id);
                            if fresh {
                                job.added.push(id);
                            }
                        }
                        None => job.skipped.push(format!("{title} (could not be added to the collection)")),
                    }
                }
                ImportMsg::Skipped(s) => {
                    job.done += 1;
                    job.skipped.push(s);
                }
                ImportMsg::Done => finished = true,
            }
        }
        if !finished {
            self.device_ui.import = Some(job);
            return;
        }
        self.reload_tracks();
        self.reload_tags();
        // Playlists, in plan order (parents before children).
        let mut made: Vec<i64> = vec![];
        for l in &job.lists {
            let parent = l.parent.and_then(|p| made.get(p).copied()).unwrap_or(0);
            let existing: Vec<String> = self.nodes.iter().filter(|n| n.parent == parent).map(|n| n.name.clone()).collect();
            let mut name = l.name.clone();
            let mut k = 2;
            while existing.contains(&name) {
                name = format!("{} ({k})", l.name);
                k += 1;
            }
            let id = self.db.create_node(parent, &name, if l.folder { NodeKind::Folder } else { NodeKind::Playlist }, None).unwrap_or(0);
            if !l.folder && id != 0 {
                let tracks: Vec<i64> = l.tracks.iter().filter_map(|t| job.map.get(t).copied()).collect();
                let _ = self.db.set_playlist_tracks(id, &tracks);
            }
            made.push(id);
            self.reload_playlists();
        }
        if self.prefs.auto_analysis {
            let todo: Vec<i64> = job.added.iter().copied().filter(|id| self.track(*id).map(|t| t.analyzed == 0).unwrap_or(false)).collect();
            self.analyze(&todo, false);
        }
        self.list_dirty = true;
        let lists = job.lists.iter().filter(|l| !l.folder).count();
        let mut msg = format!("{} track(s) added to the collection from {}", job.added.len(), job.device);
        let already = job.map.len().saturating_sub(job.added.len());
        if already > 0 {
            msg.push_str(&format!("; {already} were already in it"));
        }
        if lists > 0 {
            msg.push_str(&format!("; {lists} playlist(s) created"));
        }
        if !job.skipped.is_empty() {
            msg.push_str(&format!(". {} could not be imported: {}", job.skipped.len(), job.skipped.iter().take(4).cloned().collect::<Vec<_>>().join("; ")));
        }
        if job.skipped.is_empty() {
            self.toast(msg);
        } else {
            self.toast_err(msg);
        }
    }

    /// The import plan of a device playlist or folder: itself and everything under it.
    pub fn device_lists(lib: &usb::DeviceLibrary, node: u32) -> (Vec<ImportList>, Vec<u32>) {
        fn walk(lib: &usb::DeviceLibrary, id: u32, parent: Option<usize>, out: &mut Vec<ImportList>, tracks: &mut Vec<u32>) {
            let Some(n) = lib.nodes.iter().find(|n| n.id == id) else { return };
            out.push(ImportList { name: n.name.clone(), folder: n.folder, parent, tracks: n.entries.clone() });
            tracks.extend(n.entries.iter().copied());
            let me = out.len() - 1;
            let mut kids: Vec<&usb::DevNode> = lib.nodes.iter().filter(|c| c.parent == id).collect();
            kids.sort_by_key(|c| c.seq);
            for c in kids {
                walk(lib, c.id, Some(me), out, tracks);
            }
        }
        let (mut lists, mut tracks) = (vec![], vec![]);
        walk(lib, node, None, &mut lists, &mut tracks);
        (lists, tracks)
    }

    fn device_busy(&mut self) -> bool {
        if self.export_job.is_some() {
            self.toast_err("Wait for the export to finish before changing the device.");
            return true;
        }
        false
    }

    /// "Delete Track(s)": remove tracks from the device (its playlists, histories, files).
    pub fn delete_device_tracks(&mut self, root: &str, ids: &[u32]) {
        if self.device_busy() {
            return;
        }
        let onelib = usb::has_onelibrary(Path::new(root));
        match usb::delete_tracks(Path::new(root), ids) {
            Ok(n) if onelib => self.toast(format!("Deleted {n} track(s) from the device library. Their files were left on the drive because it also has a OneLibrary database that may use them.")),
            Ok(n) => self.toast(format!("Deleted {n} track(s) from the device")),
            Err(e) => self.toast_err(format!("Could not change the device: {e}")),
        }
        self.device_ui.sel.clear();
        self.scan_devices();
    }

    /// Remove a play history from a device (tables 11/12), journaled.
    pub fn delete_device_history(&mut self, root: &str, id: u32) -> bool {
        if self.device_busy() {
            return false;
        }
        match usb::delete_history(Path::new(root), id) {
            Ok(_) => true,
            Err(e) => {
                self.toast_err(format!("Could not delete the history from the device: {e}"));
                false
            }
        }
    }

    /// After the automatic history import: with "Delete from the device after importing the
    /// play history", histories that are fully imported are removed from their devices.
    pub fn delete_imported_histories(&mut self) -> usize {
        if !self.prefs.delete_history_after_import || self.export_job.is_some() {
            return 0;
        }
        let counts: HashMap<String, usize> = self.db.setting("imported_history_counts").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut doomed: Vec<(String, u32)> = vec![];
        for d in self.devices.iter().filter(|d| d.write_blocker.is_none()) {
            let Some(lib) = &d.lib else { continue };
            for h in &lib.histories {
                let head = h.entries.first().and_then(|e| lib.tracks.get(e)).map(|t| t.file_path.clone()).unwrap_or_default();
                let key = history_key(&d.name, &h.name, &head);
                if counts.get(&key).map(|n| *n >= h.entries.len()).unwrap_or(false) {
                    doomed.push((d.root.clone(), h.id));
                }
            }
        }
        let mut n = 0;
        for (root, id) in doomed {
            if usb::delete_history(Path::new(&root), id).is_ok() {
                n += 1;
            }
        }
        if n > 0 {
            self.toast(format!("{n} imported play histor{} deleted from the device.", if n == 1 { "y was" } else { "ies were" }));
        }
        n
    }

    /// This library's identity for "This device is synchronized with another computer."
    pub fn library_id(&self) -> String {
        if let Some(id) = self.db.setting("library_id") {
            return id;
        }
        let id = format!("{:016x}{:08x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0), std::process::id());
        self.db.set_setting("library_id", &id);
        id
    }

    /// "Synchronize": make the device match the ticked playlists. New and changed tracks are
    /// exported (audio copied again when the local file changed), tracks already there get
    /// their details, grid and (with "including cue points") cues again, and lists Recordbox
    /// put on the device that are no longer ticked or no longer exist are removed, with their
    /// unused tracks when Preferences > DJ System > Device says so.
    pub fn synchronize_device(&mut self, root: &str, cfg: &SyncConfig) {
        if self.export_job.is_some() {
            return self.toast_err("An export is already running.");
        }
        let Some(dev) = self.devices.iter().find(|d| d.root == root) else { return self.toast_err("The device is no longer connected.") };
        if let Some(reason) = dev.write_blocker.as_deref() {
            return self.toast_err(reason.to_owned());
        }
        let lib = dev.lib.clone().unwrap_or_default();
        let (tree, keep) = sync_tree(self, cfg);
        let remove = sync_removals(&usb::read_node_record(Path::new(root)), &keep, &lib);
        if tree.is_empty() && remove.is_empty() {
            return self.toast("Nothing to synchronize: no playlists are selected for this device.");
        }
        let mut saved = cfg.clone();
        saved.library = self.library_id();
        if let Err(e) = device_import::write_sync(Path::new(root), &saved) {
            return self.toast_err(format!("Could not save the synchronization settings on the device: {e}"));
        }
        self.device_ui.pending = Some((root.to_string(), remove, self.prefs.delete_device_tracks, !cfg.cues));
        self.dialogs.export_open = true;
        self.dialogs.export_root = Some(root.to_string());
        self.start_export_nodes(root, tree, &[]);
    }

    /// "Update Collection" on a device: every track of the device that Recordbox exported
    /// from a file still in the collection is exported again (details, grid, cues, analysis),
    /// without touching the device's playlists.
    pub fn update_device_collection(&mut self, root: &str) {
        let Some(lib) = self.devices.iter().find(|d| d.root == root).and_then(|d| d.lib.clone()) else { return self.toast_err("This device has no library to update.") };
        let ids: Vec<i64> = lib.tracks.values().filter(|t| !t.source.is_empty()).filter_map(|t| self.db.track_by_path(&t.source)).collect();
        if ids.is_empty() {
            return self.toast("None of this device's tracks were exported from your collection.");
        }
        self.device_ui.pending = Some((root.to_string(), vec![], false, false));
        self.dialogs.export_open = true;
        self.dialogs.export_root = Some(root.to_string());
        self.start_export_nodes(root, vec![], &ids);
    }

    /// "Import My Settings": the device's player settings become the ones in Preferences.
    pub fn import_my_settings(&mut self, root: &str) {
        let (s, found) = device_import::read_my_settings(Path::new(root), &self.prefs.usb_settings);
        if found.is_empty() {
            return self.toast_err("My Settings file is not available.");
        }
        self.prefs.usb_settings = s;
        self.save_prefs();
        self.toast(format!("My Settings imported from the device ({}).", found.join(", ")));
    }

    /// Open the Property window for a device.
    pub fn open_device_property(&mut self, root: &str) {
        let Some(dev) = self.devices.iter().find(|d| d.root == root) else { return };
        let lib = dev.lib.clone().unwrap_or_default();
        let (settings, _) = device_import::read_my_settings(Path::new(root), &self.prefs.usb_settings);
        // The drive's own category/sort tables; a drive without a library starts from Preferences.
        let browse = if dev.lib.is_some() { device_browse(Path::new(root)).unwrap_or_else(|| self.prefs.usb_browse.clone()) } else { self.prefs.usb_browse.clone() };
        let colors: [String; 8] = if lib.color_names.len() == 8 { std::array::from_fn(|i| lib.color_names[i].clone()) } else { std::array::from_fn(|i| self.prefs.color_name(i as u8 + 1)) };
        let bg = device_import::read_props(Path::new(root)).bg_color;
        let name = lib.device_name.clone();
        self.device_ui.property = Some(PropertyState { root: root.to_string(), tab: 0, opened: (name.clone(), settings.clone(), browse.clone(), colors.clone(), bg), name, settings, browse, colors, bg });
    }

    /// "Apply" in the Property window: the device's label, colour names and category/sort
    /// tables go into its database and the settings files into /PIONEER, in one journaled write.
    pub fn apply_device_property(&mut self, st: &PropertyState) -> bool {
        if self.device_busy() {
            return false;
        }
        let root = PathBuf::from(&st.root);
        let files = if st.settings == st.opened.1 && matches!(usb::check_export_target(&root), Ok(usb::DeviceLayout::Mixed)) {
            vec![]
        } else {
            st.settings.files().into_iter().map(|(n, b)| (root.join("PIONEER").join(n), b)).collect()
        };
        let (name, browse, colors) = (st.name.clone(), st.browse.clone(), st.colors.clone());
        let r = usb::change_device_with(&root, |lib| {
            lib.device_name = name.trim().chars().take(63).collect();
            lib.browse = Some(browse);
            lib.color_names = colors.iter().enumerate().map(|(i, c)| if c.trim().is_empty() { COLOR_NAMES[i + 1].to_string() } else { c.trim().to_string() }).collect();
            Ok((vec![], files))
        });
        match r.and_then(|_| device_import::write_props(&root, &DeviceProps { bg_color: st.bg })) {
            Ok(()) => {
                self.toast("Device settings written to the device.");
                self.scan_devices();
                true
            }
            Err(e) => {
                self.toast_err(format!("Could not write the device settings: {e}"));
                false
            }
        }
    }

    /// Every track of the given playlists and folders (folders with everything under them),
    /// in order, without duplicates.
    pub fn tracks_of_lists(&self, ids: &[i64]) -> Vec<i64> {
        fn walk(app: &App, id: i64, out: &mut Vec<i64>, seen: &mut HashSet<i64>) {
            let Some(n) = app.node(id) else { return };
            let tracks = match n.kind {
                NodeKind::Folder => {
                    let kids: Vec<i64> = app.nodes.iter().filter(|c| c.parent == id).map(|c| c.id).collect();
                    kids.into_iter().for_each(|k| walk(app, k, out, seen));
                    vec![]
                }
                NodeKind::Smart => n.smart.as_ref().map(|r| app.smart_tracks(r)).unwrap_or_default(),
                NodeKind::Playlist => app.db.playlist_tracks(id).unwrap_or_default(),
            };
            for t in tracks {
                if seen.insert(t) {
                    out.push(t);
                }
            }
        }
        let (mut out, mut seen) = (vec![], HashSet::new());
        for id in ids {
            walk(self, *id, &mut out, &mut seen);
        }
        out
    }

    /// Bytes an export of these tracks would add to the device (files not there yet) and the
    /// device's free space. The export itself checks again with the files' real sizes.
    pub fn export_space(&self, root: &str, tracks: &[i64]) -> (u64, Option<u64>) {
        let dev = self.devices.iter().find(|d| d.root == root);
        let on_device: HashSet<&str> = dev.and_then(|d| d.lib.as_ref()).map(|l| l.tracks.values().filter(|t| !t.source.is_empty()).map(|t| t.source.as_str()).collect()).unwrap_or_default();
        let need: u64 = tracks.iter().filter_map(|id| self.track(*id)).filter(|t| !t.missing && !on_device.contains(t.path.as_str())).map(|t| t.file_size).sum();
        (need, usb::free_space(Path::new(root)))
    }

    /// The export window's Export button: checks the free space first ("Not enough space
    /// available on the device"), then exports the ticked playlists with their folder
    /// structure, or only their tracks.
    pub fn export_from_dialog(&mut self, root: &str, lists: &[i64], extra: &[i64]) -> bool {
        let mut all = self.tracks_of_lists(lists);
        for t in extra {
            if !all.contains(t) {
                all.push(*t);
            }
        }
        let (need, free) = self.export_space(root, &all);
        if let Some(free) = free {
            // The same margin the export keeps for the databases and analysis files.
            if need + 50_000_000 > free {
                self.toast_err(format!("Not enough space available on the device ({:.1} MB needed, {:.1} MB free).", (need + 50_000_000) as f64 / 1e6, free as f64 / 1e6));
                return false;
            }
        }
        if self.device_ui.export_tracks_only {
            self.start_export(root, &[], &all);
        } else {
            self.start_export(root, lists, extra);
        }
        true
    }

    /// Automatic synchronisation: a newly connected device whose Sync Manager has it switched
    /// on (for this library) is offered once per connection.
    pub fn offer_auto_sync(&mut self) {
        let roots: HashSet<String> = self.devices.iter().filter(|d| d.write_blocker.is_none()).map(|d| d.root.clone()).collect();
        self.device_ui.auto_offered.retain(|r| roots.contains(r));
        self.device_ui.bg = roots.iter().map(|r| (r.clone(), device_import::read_props(Path::new(r)).bg_color)).collect();
        if !roots.contains(&self.device_ui.sel_root) {
            self.device_ui.sel.clear();
        }
        if self.screenshot.is_some() || self.export_job.is_some() || self.device_ui.auto_prompt.is_some() {
            return;
        }
        let mine = self.library_id();
        for r in roots {
            if self.device_ui.auto_offered.contains(&r) {
                continue;
            }
            self.device_ui.auto_offered.insert(r.clone());
            if device_import::read_sync(Path::new(&r)).map(|c| c.auto && c.library == mine).unwrap_or(false) {
                self.device_ui.auto_prompt = Some(r);
                break;
            }
        }
    }
}

/// The category/sort choices stored in a drive's menu tables (17, 18), read back.
pub fn device_browse(root: &Path) -> Option<usb::UsbBrowse> {
    let db = crate::export::pdb::Database::parse(std::fs::read(usb::pdb_path(root)).ok()?).ok()?;
    let mut cats: Vec<(u16, u16)> = vec![];
    for (h, o) in db.rows(17) {
        let r = db.row_bytes(h, o);
        if r.len() >= 8 && r[5] != 1 {
            cats.push((u16::from_le_bytes([r[6], r[7]]), u16::from_le_bytes([r[0], r[1]])));
        }
    }
    let mut sorts: Vec<(u8, u16)> = vec![];
    for (h, o) in db.rows(18) {
        let r = db.row_bytes(h, o);
        if r.len() >= 8 && r[4] != 1 {
            sorts.push((r[5], u16::from_le_bytes([r[0], r[1]])));
        }
    }
    if cats.is_empty() && sorts.is_empty() {
        return None;
    }
    cats.sort();
    sorts.sort();
    Some(usb::UsbBrowse { categories: cats.into_iter().map(|c| c.1).collect(), sorts: sorts.into_iter().map(|c| c.1).collect() }.normalized())
}

// ------------------------------------------------------------------------------------------
// Menus
// ------------------------------------------------------------------------------------------

/// Context menu of device tracks (the selection in the device list).
pub fn device_track_menu(app: &mut App, ui: &mut Ui, root: &str, ids: &[u32]) {
    if ui.button("Import To Collection").on_hover_text("Copy the music files from the device into your collection, with their cues, beat grids, My Tags and artwork.").clicked() {
        app.import_from_device(root, ids, vec![]);
        ui.close_menu();
    }
    ui.separator();
    let dev = app.devices.iter().find(|d| d.root == root);
    let blocker = dev.and_then(|d| d.write_blocker.as_deref());
    let hint = if dev.is_some_and(|d| d.layout == Some(usb::DeviceLayout::Mixed)) {
        "Remove from classic playlists and histories. Shared files stay on the drive."
    } else {
        "Remove from device playlists and histories, and delete unused media."
    };
    if ui.add_enabled(blocker.is_none(), egui::Button::new(if ids.len() == 1 { "Delete Track" } else { "Delete Tracks" })).on_disabled_hover_text(blocker.unwrap_or_default()).on_hover_text(hint).clicked() {
        app.device_ui.confirm_delete = Some((root.to_string(), ids.to_vec()));
        ui.close_menu();
    }
}

/// Context menu of a device playlist or folder.
pub fn device_node_menu(app: &mut App, ui: &mut Ui, root: &str, node: &usb::DevNode) {
    let lib = app.devices.iter().find(|d| d.root == root).and_then(|d| d.lib.clone());
    if ui.button("Import To Collection").on_hover_text("Copy this list's tracks from the device into your collection.").clicked() {
        if let Some(lib) = &lib {
            let (_, tracks) = App::device_lists(lib, node.id);
            app.import_from_device(root, &tracks, vec![]);
        }
        ui.close_menu();
    }
    if ui.button("Import as a playlist").on_hover_text("Copy the tracks into your collection and create this playlist (a folder with its playlists) there.").clicked() {
        if let Some(lib) = &lib {
            let (lists, tracks) = App::device_lists(lib, node.id);
            app.import_from_device(root, &tracks, lists);
        }
        ui.close_menu();
    }
    if !node.folder && ui.button("Export as a playlist").on_hover_text("Save this device playlist as an .m3u8 file that points at the files on the device.").clicked() {
        ui.close_menu();
        if let Some(lib) = &lib {
            if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{}.m3u8", node.name)).add_filter("M3U8 playlist", &["m3u8"]).save_file() {
                let entries: Vec<crate::playlist_files::Entry> = node.entries.iter().filter_map(|t| lib.tracks.get(t)).map(|t| device_entry(root, t)).collect();
                match std::fs::write(&path, crate::playlist_files::write_m3u8(&entries)) {
                    Ok(_) => app.toast(format!("Playlist saved to {}", path.display())),
                    Err(e) => app.toast_err(format!("Could not save the playlist: {e}")),
                }
            }
        }
    }
    ui.separator();
    let blocker = app.devices.iter().find(|d| d.root == root).and_then(|d| d.write_blocker.as_deref());
    if ui.add_enabled(blocker.is_none(), egui::Button::new(if node.folder { "Delete Folder" } else { "Delete Playlist" })).on_disabled_hover_text(blocker.unwrap_or_default()).clicked() {
        app.dialogs.device_remove = Some((root.to_string(), node.id, node.name.clone()));
        ui.close_menu();
    }
}

fn device_entry(root: &str, t: &usb::DevTrack) -> crate::playlist_files::Entry {
    crate::playlist_files::Entry {
        path: Path::new(root).join(t.file_path.trim_start_matches('/')).to_string_lossy().to_string(),
        title: t.title.clone(),
        artist: t.artist.clone(),
        album: t.album.clone(),
        genre: t.genre.clone(),
        bpm: t.bpm100 as f64 / 100.0,
        rating: t.rating,
        duration: t.duration as f64,
        key: t.key_name.clone(),
        ..Default::default()
    }
}

/// Items of a device row's context menu that belong to this file (the rest is in ui_browser).
pub fn device_row_menu(app: &mut App, ui: &mut Ui, root: &str) {
    let Some(dev) = app.devices.iter().find(|d| d.root == root) else { return };
    let blocker = dev.write_blocker.clone();
    let mixed = dev.layout == Some(usb::DeviceLayout::Mixed);
    let known = dev.layout.is_some();
    let one_library = mixed || dev.layout == Some(usb::DeviceLayout::OneLibrary);
    let catalog_blocker = blocker.clone().or_else(|| dev.lib.is_none().then(|| "This action needs a classic Device Library.".to_string()));
    if one_library && app.device_ui.one_inspector.button(ui, root) { ui.close_menu(); }
    if ui.add_enabled(blocker.is_none(), egui::Button::new("Sync Manager")).on_disabled_hover_text(blocker.as_deref().unwrap_or_default()).on_hover_text("Synchronize playlists with a device").clicked() {
        open_sync_manager(app, root);
        ui.close_menu();
    }
    if ui.add_enabled(catalog_blocker.is_none(), egui::Button::new("Update Collection")).on_disabled_hover_text(catalog_blocker.as_deref().unwrap_or_default()).on_hover_text("Export again every track on this device that came from your collection, so the device has your latest cues, grids and track details.").clicked() {
        app.update_device_collection(root);
        ui.close_menu();
    }
    ui.separator();
    let has = known && device_import::has_my_settings(Path::new(root));
    if ui.add_enabled(has, egui::Button::new("Import My Settings")).on_disabled_hover_text("My Settings file is not available.").clicked() {
        app.import_my_settings(root);
        ui.close_menu();
    }
    let delete_blocker = blocker.as_deref().or(mixed.then_some("Player settings on this drive are shared with OneLibrary.")).or((!has).then_some("My Settings file is not available."));
    if ui.add_enabled(delete_blocker.is_none(), egui::Button::new("Delete My Settings")).on_disabled_hover_text(delete_blocker.unwrap_or_default()).clicked() {
        app.device_ui.confirm_delete_settings = Some(root.to_string());
        ui.close_menu();
    }
    ui.separator();
    if ui.add_enabled(blocker.is_none(), egui::Button::new("Property")).on_disabled_hover_text(blocker.as_deref().unwrap_or_default()).clicked() {
        app.open_device_property(root);
        ui.close_menu();
    }
}

pub fn open_sync_manager(app: &mut App, root: &str) {
    let cfg = device_import::read_sync(Path::new(root)).unwrap_or_default();
    let other_computer = !cfg.library.is_empty() && cfg.library != app.library_id();
    app.device_ui.sync = Some(SyncState { root: root.to_string(), saved: cfg.clone(), cfg, other_computer });
}

// ------------------------------------------------------------------------------------------
// Windows
// ------------------------------------------------------------------------------------------

/// Every device window; called once per frame.
pub fn windows(app: &mut App, ctx: &Context) {
    app.poll_device_jobs();
    import_progress(app, ctx);
    property_window(app, ctx);
    sync_window(app, ctx);
    app.device_ui.one_inspector.window(ctx);
    confirm_windows(app, ctx);
    crate::export::hotcue_bank::windows(app, ctx);
}

fn import_progress(app: &mut App, ctx: &Context) {
    let Some(job) = &app.device_ui.import else { return };
    if job.total == 0 {
        return;
    }
    let (done, total, device) = (job.done, job.total, job.device.clone());
    egui::Window::new("Import To Collection").collapsible(false).resizable(false).show(ctx, |ui| {
        ui.label(format!("Copying tracks from {device}"));
        ui.add(egui::ProgressBar::new(done as f32 / total as f32).text(format!("{done} / {total}  ({} remaining)", total - done)));
    });
    ctx.request_repaint();
}

fn confirm_windows(app: &mut App, ctx: &Context) {
    if let Some((root, ids)) = app.device_ui.confirm_delete.clone() {
        egui::Window::new(if ids.len() == 1 { "Delete Track" } else { "Delete Tracks" }).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("Delete {} track(s) from the device?", ids.len()));
            let mixed = app.devices.iter().find(|d| d.root == root).is_some_and(|d| d.layout == Some(usb::DeviceLayout::Mixed));
            let detail = if mixed {
                "The tracks are removed from classic playlists and histories. Their music, analysis and artwork stay on the drive for OneLibrary."
            } else {
                "The tracks are removed from the device's playlists and histories. Their unused music, analysis and artwork files are deleted."
            };
            ui.label(RichText::new(detail).size(11.0).color(DIM));
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    app.device_ui.confirm_delete = None;
                    app.delete_device_tracks(&root, &ids);
                }
                if ui.button("Cancel").clicked() {
                    app.device_ui.confirm_delete = None;
                }
            });
        });
    }
    if let Some(root) = app.device_ui.confirm_delete_settings.clone() {
        egui::Window::new("Delete My Settings").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("Do you want to delete My Settings file in the device?");
            ui.horizontal(|ui| {
                if ui.button("Yes").clicked() {
                    app.device_ui.confirm_delete_settings = None;
                    if !app.device_busy() {
                        match device_import::delete_my_settings(Path::new(&root)) {
                            Ok(n) => app.toast(format!("{n} My Settings file(s) deleted from the device.")),
                            Err(e) => app.toast_err(format!("Could not delete My Settings: {e}")),
                        }
                    }
                }
                if ui.button("No").clicked() {
                    app.device_ui.confirm_delete_settings = None;
                }
            });
        });
    }
    if let Some(root) = app.device_ui.auto_prompt.clone() {
        let name = app.devices.iter().find(|d| d.root == root).map(|d| d.name.clone()).unwrap_or_default();
        egui::Window::new("Synchronization").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(RichText::new(name).strong());
            ui.label("The selected lists are synchronized automatically \nat each connection.");
            ui.label("Do you want to synchronize the modifications now?");
            ui.horizontal(|ui| {
                if ui.button("Yes").clicked() {
                    app.device_ui.auto_prompt = None;
                    if let Some(cfg) = device_import::read_sync(Path::new(&root)) {
                        app.synchronize_device(&root, &cfg);
                    }
                }
                if ui.button("No").clicked() {
                    app.device_ui.auto_prompt = None;
                }
            });
        });
    }
}

const PROPERTY_TABS: [&str; 5] = ["General", "Category", "Sort", "Column", "My Settings"];

fn property_window(app: &mut App, ctx: &Context) {
    let Some(mut st) = app.device_ui.property.take() else { return };
    let Some(dev) = app.devices.iter().find(|d| d.root == st.root).map(|d| (d.name.clone(), d.fs.clone(), d.layout, d.write_blocker.clone())) else { return };
    let mut open = true;
    let mut close = false;
    egui::Window::new("Property").id(egui::Id::new("device_property")).open(&mut open).collapsible(false).default_size([640.0, 560.0]).show(ctx, |ui| {
        ui.label(RichText::new(format!("{}  ({})", dev.0, App::fs_label(&dev.1))).strong());
        ui.horizontal(|ui| {
            for (i, t) in PROPERTY_TABS.iter().enumerate() {
                if ui.selectable_label(st.tab == i, *t).clicked() {
                    st.tab = i;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().max_height(430.0).auto_shrink([false, false]).show(ui, |ui| {
            if st.tab == 0 {
                ui.horizontal(|ui| {
                    ui.label("Device Name");
                    ui.add(egui::TextEdit::singleline(&mut st.name).desired_width(260.0).char_limit(63));
                });
                ui.label(RichText::new("Kept as the label of the device library's synchronization record (export.pdb), where rekordbox keeps a device or backup label.").size(10.5).color(DIM));
                ui.add_space(6.0);
            }
            // The Preferences pages for these tabs, working on this device's values.
            std::mem::swap(&mut app.prefs.usb_settings, &mut st.settings);
            std::mem::swap(&mut app.prefs.usb_browse, &mut st.browse);
            std::mem::swap(&mut app.prefs.device_bg_color, &mut st.bg);
            ui.push_id("device_property_pages", |ui| crate::ui_prefs::page_dj_system(app, ui, st.tab));
            std::mem::swap(&mut app.prefs.usb_settings, &mut st.settings);
            std::mem::swap(&mut app.prefs.usb_browse, &mut st.browse);
            std::mem::swap(&mut app.prefs.device_bg_color, &mut st.bg);
            if st.tab == 0 {
                ui.label(RichText::new("Background Color is kept on the drive for recordbox (it colours the device's row). The standard device library has no field for it; rekordbox stores it in the OneLibrary database, which recordbox does not write.").size(10.5).color(DIM));
                ui.add_space(8.0);
                ui.label(RichText::new("Customize Color Comments").strong());
                egui::Grid::new("device_color_names").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    for i in 0..8 {
                        ui.label(RichText::new(format!("■ {}", COLOR_NAMES[i + 1])).color(label_color(i as u8 + 1)));
                        ui.add(egui::TextEdit::singleline(&mut st.colors[i]).desired_width(260.0).char_limit(63));
                        ui.end_row();
                    }
                });
                ui.label(RichText::new("Written to the device's colour table; the players show these names in the COLOR category.").size(10.5).color(DIM));
            }
        });
        ui.separator();
        let mixed_settings = dev.2 == Some(usb::DeviceLayout::Mixed) && st.settings != st.opened.1;
        let blocker = dev.3.as_deref().or(mixed_settings.then_some("Player settings are shared with OneLibrary. Leave them as saved to apply classic catalog properties."));
        if let Some(reason) = blocker {
            ui.label(RichText::new(reason).color(CAUTION_TX));
        }
        let changed = (st.name.clone(), st.settings.clone(), st.browse.clone(), st.colors.clone(), st.bg) != st.opened;
        ui.horizontal(|ui| {
            if ui.add_enabled(changed && blocker.is_none(), egui::Button::new(RichText::new("Apply").strong())).on_disabled_hover_text(blocker.unwrap_or("Nothing has been changed.")).on_hover_text("Press to apply the customized setting to the DJ equipment").clicked() && app.apply_device_property(&st) {
                st.opened = (st.name.clone(), st.settings.clone(), st.browse.clone(), st.colors.clone(), st.bg);
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    });
    if open && !close {
        app.device_ui.property = Some(st);
    }
}

fn sync_window(app: &mut App, ctx: &Context) {
    let Some(mut st) = app.device_ui.sync.take() else { return };
    let Some(device) = app.devices.iter().find(|d| d.root == st.root) else { return };
    let name = device.name.clone();
    let format = device.catalog_label();
    let blocker = device.write_blocker.clone();
    let mixed = device.layout == Some(usb::DeviceLayout::Mixed);
    let one_library = mixed || device.layout == Some(usb::DeviceLayout::OneLibrary);
    let mut open = true;
    let mut close = false;
    egui::Window::new("Sync Manager").id(egui::Id::new("sync_manager")).open(&mut open).collapsible(false).default_width(520.0).show(ctx, |ui| {
        ui.label(RichText::new(format!("Synchronize playlists with a device: {name}")).strong());
        ui.label(RichText::new(format!("{format} · {}", st.root)).size(11.0).color(DIM));
        if one_library { app.device_ui.one_inspector.button(ui, &st.root); }
        if let Some(reason) = &blocker {
            ui.label(RichText::new(reason).color(CAUTION_TX));
            close = ui.button("Close").clicked();
            return;
        }
        if mixed {
            ui.label(RichText::new("Synchronizes the classic catalog. Files already shared with OneLibrary are preserved.").size(11.0).color(CAUTION_TX));
        }
        if st.other_computer {
            ui.label(RichText::new("This device is synchronized with another computer.").color(CAUTION_TX));
            ui.label("Are you sure you want to synchronize this with this computer?");
            ui.label(RichText::new("If you choose \"Yes\", the settings of the synchronization will be erased.").size(11.0).color(DIM));
            ui.horizontal(|ui| {
                if ui.button("Yes").clicked() {
                    st.cfg = SyncConfig::default();
                    st.saved = st.cfg.clone();
                    st.other_computer = false;
                }
                if ui.button("No").clicked() {
                    close = true;
                }
            });
            return;
        }
        ui.horizontal(|ui| {
            ui.label("Synchronization");
            if ui.radio(st.cfg.all, "All playlists").clicked() {
                st.cfg.all = true;
            }
            if ui.radio(!st.cfg.all, "Selected playlists").clicked() {
                st.cfg.all = false;
            }
        });
        egui::ScrollArea::vertical().max_height(280.0).auto_shrink([false, false]).show(ui, |ui| {
            fn rows(app: &App, ui: &mut Ui, cfg: &mut SyncConfig, parent: i64, depth: usize, inherited: bool) {
                let kids: Vec<PlaylistNode> = app.nodes.iter().filter(|n| n.parent == parent).cloned().collect();
                for n in kids {
                    let ticked = cfg.playlists.contains(&n.id);
                    let mut on = cfg.all || inherited || ticked;
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 16.0);
                        let count = match n.kind {
                            NodeKind::Folder => String::new(),
                            NodeKind::Smart => format!(" ({})", n.smart.as_ref().map(|r| app.smart_tracks(r).len()).unwrap_or(0)),
                            NodeKind::Playlist => format!(" ({})", app.node_counts.get(&n.id).copied().unwrap_or(0)),
                        };
                        let icon = match n.kind {
                            NodeKind::Folder => "▶",
                            NodeKind::Smart => "⚙",
                            NodeKind::Playlist => "☰",
                        };
                        if ui.add_enabled(!cfg.all && !inherited, egui::Checkbox::new(&mut on, format!("{icon} {}{count}", n.name))).changed() {
                            if on {
                                cfg.playlists.push(n.id);
                            } else {
                                cfg.playlists.retain(|x| *x != n.id);
                            }
                        }
                    });
                    if n.kind == NodeKind::Folder {
                        rows(app, ui, cfg, n.id, depth + 1, inherited || ticked);
                    }
                }
            }
            rows(app, ui, &mut st.cfg, 0, 0, false);
            if app.nodes.is_empty() {
                ui.label(RichText::new("No playlists yet.").color(DIM));
            }
        });
        ui.checkbox(&mut st.cfg.cues, "including cue points").on_hover_text("Tracks that are already on the device get your collection's hot cues and memory cues again. Switched off, cues set on players stay as they are on the device.");
        ui.checkbox(&mut st.cfg.auto, "Automatic synchronization").on_hover_text("The selected lists are synchronized automatically at each connection (recordbox asks first).");
        // What "Synchronize" will do.
        let (tree, keep) = sync_tree(app, &st.cfg);
        let lib = app.devices.iter().find(|d| d.root == st.root).and_then(|d| d.lib.as_ref());
        let remove = lib.map(|l| sync_removals(&usb::read_node_record(Path::new(&st.root)), &keep, l)).unwrap_or_default();
        fn count(n: &ExportNode) -> usize {
            (!n.folder) as usize + n.children.iter().map(count).sum::<usize>()
        }
        let lists: usize = tree.iter().map(count).sum();
        let mut summary = format!("{lists} playlist(s) will be on the device");
        if !remove.is_empty() {
            let names: Vec<&str> = remove.iter().filter_map(|id| lib?.nodes.iter().find(|n| n.id == *id)).map(|n| n.name.as_str()).collect();
            summary.push_str(&format!("; removed from the device: {}", names.join(", ")));
            summary.push_str(if mixed && app.prefs.delete_device_tracks { " (unlisted tracks leave the classic catalog, and their files stay on the drive)" } else if app.prefs.delete_device_tracks { " (with their tracks that no other playlist uses)" } else { " (their tracks stay on the device)" });
        }
        ui.label(RichText::new(summary).size(11.0).color(DIM));
        ui.separator();
        let dirty = st.cfg != st.saved;
        ui.horizontal(|ui| {
            let can = app.export_job.is_none() && (lists > 0 || !remove.is_empty());
            if ui.add_enabled(can, egui::Button::new(RichText::new("Synchronize").strong()).fill(BLUE)).on_hover_text("Synchronizes playlist(s) you selected to a device").on_disabled_hover_text("Select at least one playlist, and wait for a running export to finish.").clicked() {
                app.synchronize_device(&st.root, &st.cfg);
                close = true;
            }
            if ui.add_enabled(dirty, egui::Button::new("Apply")).on_hover_text("Save the selection on the device without synchronizing now.").clicked() {
                let mut c = st.cfg.clone();
                c.library = app.library_id();
                match device_import::write_sync(Path::new(&st.root), &c) {
                    Ok(_) => st.saved = st.cfg.clone(),
                    Err(e) => app.toast_err(format!("Could not save the synchronization settings on the device: {e}")),
                }
            }
            if ui.button("Close").clicked() {
                close = true;
            }
            if dirty {
                ui.label(RichText::new("Changes are not synchronized.").size(11.0).color(CAUTION_TX));
            }
        });
    });
    if open && !close {
        app.device_ui.sync = Some(st);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_removes_only_lists_recordbox_put_on_the_device() {
        let node = |id: u32, parent: u32, name: &str, folder: bool| usb::DevNode { id, parent, name: name.into(), folder, seq: 0, entries: vec![] };
        let lib = usb::DeviceLibrary {
            nodes: vec![node(1, 0, "Gigs", true), node(2, 1, "Friday", false), node(3, 1, "Made on a player", false), node(4, 0, "Old", false), node(5, 0, "rekordbox list", false), node(6, 0, "Empty folder", true), node(7, 6, "Inside", false)],
            ..Default::default()
        };
        // Library IDs 10 (Gigs), 11 (Friday), 12 (Old), 13/14 (Empty folder / Inside) were exported by Recordbox.
        let record: HashMap<i64, u32> = [(10, 1), (11, 2), (12, 4), (13, 6), (14, 7), (99, 42)].into();
        // Everything still ticked: nothing goes.
        let all: HashSet<i64> = [10, 11, 12, 13, 14].into();
        assert!(sync_removals(&record, &all, &lib).is_empty());
        // "Old" unticked or deleted in the library: it goes; lists Recordbox never made stay.
        let keep: HashSet<i64> = [10, 11, 13, 14].into();
        assert_eq!(sync_removals(&record, &keep, &lib), vec![4]);
        // "Gigs" and "Friday" unticked: Friday goes, but the folder still holds a playlist made
        // on a player, so the folder stays.
        let keep: HashSet<i64> = [12, 13, 14].into();
        assert_eq!(sync_removals(&record, &keep, &lib), vec![2]);
        // A folder with only Recordbox's own lists goes with them.
        let keep: HashSet<i64> = [10, 11, 12].into();
        assert_eq!(sync_removals(&record, &keep, &lib), vec![6, 7]);
        // A recorded node that is no longer on the device (42) is ignored.
        assert!(!sync_removals(&record, &HashSet::new(), &lib).contains(&42));
    }
}

/// Scratch devices live through all frames of a screenshot. Existing device folders are
/// available for browsing only, and are never used as destinations for example exports.
pub(crate) enum ScreenshotDevice {
    Scratch(tempfile::TempDir),
    Existing(PathBuf),
}

impl ScreenshotDevice {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Scratch(dir) => dir.path(),
            Self::Existing(path) => path,
        }
    }

    fn prepare(parent: Option<&Path>, browse: bool) -> std::io::Result<Self> {
        if let Some(parent) = parent {
            let parent = parent.canonicalize()?;
            if !parent.is_dir() {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "expected an existing device folder"));
            }
            if browse && !matches!(usb::inspect_target(&parent), Ok((usb::DeviceLayout::Empty, None))) {
                return Ok(Self::Existing(parent));
            }
            return tempfile::Builder::new().prefix("recordbox-shot-").tempdir_in(parent).map(Self::Scratch);
        }
        tempfile::Builder::new().prefix("recordbox-shot-").tempdir().map(Self::Scratch)
    }
}

impl App {
    pub(crate) fn screenshot_device(&mut self, browse: bool) -> Option<&ScreenshotDevice> {
        let parent = std::env::var_os("PREPDECK_SHOT_DEVICE").filter(|root| !root.is_empty());
        match ScreenshotDevice::prepare(parent.as_deref().map(Path::new), browse) {
            Ok(device) => {
                if let Some(i) = self.screenshot_devices.iter().position(|d| d.path() == device.path()) {
                    return self.screenshot_devices.get(i);
                }
                self.screenshot_devices.push(device);
                self.screenshot_devices.last()
            }
            Err(error) => {
                eprintln!("Could not prepare screenshot device: {error}");
                None
            }
        }
    }
}

#[cfg(test)]
#[test]
fn screenshot_exports_use_private_devices_and_preserve_existing_catalogs() -> std::io::Result<()> {
    let parent = tempfile::tempdir()?;
    let catalog = parent.path().join("PIONEER/rekordbox/export.pdb");
    std::fs::create_dir_all(catalog.parent().unwrap())?;
    std::fs::write(&catalog, b"original catalog")?;
    let display = ScreenshotDevice::prepare(Some(parent.path()), true)?;
    assert!(matches!(&display, ScreenshotDevice::Existing(_)));
    assert_eq!(display.path(), parent.path().canonicalize()?);
    let first = ScreenshotDevice::prepare(Some(parent.path()), false)?;
    let second = ScreenshotDevice::prepare(Some(parent.path()), false)?;
    let first_path = first.path().to_owned();
    assert_eq!(first.path().parent(), Some(display.path()));
    assert_ne!(first.path(), second.path());
    std::fs::write(first.path().join("fixture"), b"copied audio")?;
    drop(first);
    assert!(!first_path.exists());
    assert!(second.path().exists());
    assert_eq!(std::fs::read(catalog)?, b"original catalog");
    assert!(ScreenshotDevice::prepare(Some(&parent.path().join("missing")), false).is_err());
    Ok(())
}

/// PREPDECK_SHOT_DEVICE can point to a catalog to display. Sample exports for the
/// screenshot always use a new subfolder there or in the system temporary directory.
pub fn shot_setup(app: &mut App, what: &str) {
    let Some(device) = app.screenshot_device(what != "devicereport") else { return };
    let scratch = matches!(device, ScreenshotDevice::Scratch(_));
    let root = device.path().to_path_buf();
    let r = root.to_string_lossy().to_string();
    let mut report = None;
    if scratch {
        let ids: Vec<i64> = app.tracks.iter().filter(|t| !t.missing && usb::player_supported(&t.file_type)).take(6).map(|t| t.id).collect();
        let mut tracks = HashMap::new();
        for id in &ids {
            if let Some(t) = app.track(*id).cloned() {
                tracks.insert(*id, usb::ExportTrack { cues: app.db.cues(*id).unwrap_or_default(), grid: app.db.grid(*id), wave: app.db.wave(*id), track: t, tags: vec![], rb_anlz: None, grid_same: false, vocal: None, phrases: None });
            }
        }
        let half = ids.len() / 2;
        let nodes = vec![ExportNode { id: 0, name: "Friday".into(), folder: true, children: vec![ExportNode { id: 0, name: "Warm Up".into(), folder: false, children: vec![], tracks: ids[..half].to_vec() }, ExportNode { id: 0, name: "Peak Time".into(), folder: false, children: vec![], tracks: ids[half..].to_vec() }], tracks: vec![] }];
        let skipped = if what == "devicereport" { vec!["Example Track (file not found)".to_string()] } else { vec![] };
        let req = usb::ExportRequest { root: root.clone(), nodes, tracks, skipped, analysis: app.analysis_settings(), fs: "folder".into(), ..Default::default() };
        let got = std::sync::Mutex::new(None);
        let res = usb::run_export(
            req,
            &|p| {
                if let usb::Progress::Report(rep) = p {
                    *got.lock().unwrap() = Some(rep);
                }
            },
            &std::sync::atomic::AtomicBool::new(false),
        );
        if let Err(e) = res {
            eprintln!("shot export failed: {e}");
        }
        report = got.into_inner().ok().flatten();
    }
    app.scan_devices();
    app.side = SideMode::Devices;
    app.view = View::Device { root: r.clone(), node: None };
    match what {
        "devicelist" => {
            if let Some(lib) = app.devices.iter().find(|d| d.root == r).and_then(|d| d.lib.as_ref()) {
                app.device_ui.sel = lib.tracks.keys().copied().take(2).collect();
                app.device_ui.sel_root = r.clone();
            }
        }
        "deviceprop" => {
            app.open_device_property(&r);
            if let Some(p) = app.device_ui.property.as_mut() {
                p.tab = std::env::var("PREPDECK_SHOT_TAB").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
                if p.name.is_empty() {
                    p.name = "ADAM USB 1".into();
                }
            }
        }
        "devicesync" => {
            if app.nodes.is_empty() {
                let f = app.db.create_node(0, "Friday Set", NodeKind::Folder, None).unwrap_or(0);
                let p = app.db.create_node(f, "Warm Up", NodeKind::Playlist, None).unwrap_or(0);
                let ids: Vec<i64> = app.tracks.iter().take(5).map(|t| t.id).collect();
                let _ = app.db.set_playlist_tracks(p, &ids);
                let _ = app.db.create_node(0, "Closing", NodeKind::Playlist, None);
                app.reload_playlists();
            }
            open_sync_manager(app, &r);
            if let Some(s) = app.device_ui.sync.as_mut() {
                if s.cfg.playlists.is_empty() {
                    s.cfg.playlists = app.nodes.iter().filter(|n| n.kind != NodeKind::Folder).take(1).map(|n| n.id).collect();
                }
            }
        }
        "devicereport" => {
            app.dialogs.export_open = true;
            app.dialogs.export_root = Some(r.clone());
            app.export_report = report;
        }
        "hotcuebank" => {
            app.side = SideMode::Playlists;
            let mut banks = crate::export::hotcue_bank::Banks::from_json(&app.db.setting("hot_cue_banks").unwrap_or_default());
            if banks.0.is_empty() {
                banks.create();
            }
            let id = banks.0[0].id;
            let with_cues = app.tracks.iter().find(|t| app.db.cues(t.id).map(|c| c.iter().any(|c| c.kind == CueKind::Hot)).unwrap_or(false)).cloned();
            if let Some(t) = with_cues {
                let cues = app.db.cues(t.id).unwrap_or_default();
                banks.fill_from_track(id, &t, &cues);
                app.selected = [t.id].into();
            }
            app.db.set_setting("hot_cue_banks", &banks.to_json());
            app.device_ui.banks.open = Some(id);
        }
        _ => {}
    }
}
