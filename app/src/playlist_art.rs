//! Playlist artwork, as rekordbox 7.2.19 has it (traced with IDA, notes in the track's
//! report): a playlist's right-click menu (`BrowsePopupMenuManager::showTreeViewPopupMenu`
//! 0x100118eec) has, between the Create items and Delete Playlist,
//! `addItemArtworkMenu` (0x1001140b4): "Add Artwork" when the playlist has none; else the
//! artwork itself as the menu's first item (`browse::ArtworkComponent`: the picture fitted
//! into 100 x 100 pt, centred, 10 pt from the item's top, item 120 pt tall), then "Change
//! Artwork" and "Delete Artwork". Add / Change open a file panel titled "Select Artwork" on the
//! pictures folder, filtered `*.jpg;*.jpeg;*.png` (`TreeViewer::treeMessageAddArtwork`
//! 0x101565de0), and the picture is stored with the list (`DatabaseIF::modifyListArtwork`).
//! That menu is the only place the artwork is seen in the local library: the other playlist
//! artwork views (`PlaylistDataPanel`, `PlaylistPage`) belong to the streaming services' pages.
//!
//! A playlist owns a uniquely named file inside the backed-up recordbox artwork directory.
//! The database stores its filename so restored backups can find it on another computer.

use crate::app::*;
use eframe::egui;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::path::{Path, PathBuf};

pub fn setting_key(id: i64) -> String {
    format!("playlist_art_{id}")
}

fn plain_entry(path: &Path, directory: bool) -> bool {
    let Ok(m) = path.symlink_metadata() else { return false };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 { return false; }
    }
    if directory { m.is_dir() } else { m.is_file() }
}

fn managed_art(root: &Path, id: i64, stored: &str) -> Option<PathBuf> {
    let path = Path::new(stored);
    let stem = path.file_stem()?.to_str()?;
    let (dir, prefix) = if path.parent() == Some(Path::new("")) {
        (root.join("artwork"), format!("playlist-{id}-"))
    } else {
        // Earlier upstream libraries recorded absolute paths for their local copies.
        let legacy = root.join("playlist_artwork");
        if path.parent()? != legacy { return None; }
        (legacy, format!("{id}-"))
    };
    let token = stem.strip_prefix(&prefix)?;
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_alphanumeric())
        || !matches!(path.extension()?.to_str()?, "jpg" | "jpeg" | "png")
        || !plain_entry(&dir, true)
    {
        return None;
    }
    let file = dir.join(path.file_name()?);
    plain_entry(&file, false).then_some(file)
}

fn replace_art(db: &mut crate::db::Db, root: &Path, id: i64, src: Option<&Path>) -> anyhow::Result<()> {
    let new_file = if let Some(src) = src {
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        anyhow::ensure!(matches!(ext.as_str(), "jpg" | "jpeg" | "png"), "Choose a JPEG or PNG picture.");
        anyhow::ensure!(src.is_file(), "The selected picture is unavailable.");
        let dir = root.join("artwork");
        std::fs::create_dir_all(&dir)?;
        anyhow::ensure!(plain_entry(&dir, true), "The artwork directory must be a local directory.");
        let mut temp = tempfile::Builder::new().prefix(&format!("playlist-{id}-")).suffix(&format!(".{ext}")).tempfile_in(dir)?;
        std::io::copy(&mut std::fs::File::open(src)?, temp.as_file_mut())?;
        temp.as_file_mut().sync_all()?;
        Some(temp.into_temp_path().keep()?)
    } else {
        None
    };
    let key = setting_key(id);
    let mut update = || -> anyhow::Result<Option<String>> {
        let tx = db.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = tx.query_row("SELECT value FROM settings WHERE key=?1", [&key], |r| r.get(0)).optional()?;
        let name = new_file.as_ref().and_then(|p| p.file_name()?.to_str()).unwrap_or("");
        tx.execute("INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)", params![key, name])?;
        tx.commit()?;
        Ok(previous)
    };
    match update() {
        Ok(previous) => {
            if let Some(old) = previous.and_then(|v| managed_art(root, id, &v)) {
                let _ = std::fs::remove_file(old);
            }
            Ok(())
        }
        Err(e) => {
            if let Some(unused) = new_file { let _ = std::fs::remove_file(unused); }
            Err(e)
        }
    }
}

impl App {
    pub fn playlist_art(&self, id: i64) -> Option<String> {
        let stored = self.db.setting(&setting_key(id))?;
        managed_art(&crate::db::data_dir(), id, &stored).map(|p| p.to_string_lossy().into_owned())
    }

    pub fn playlist_art_set(&mut self, id: i64, src: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(!self.prefs.rb.library_protection, "Library Protection is enabled.");
        replace_art(&mut self.db, &crate::db::data_dir(), id, Some(src))
    }

    pub fn playlist_art_delete(&mut self, id: i64) {
        if !crate::prefs_bridge::library_protected(self) {
            if let Err(e) = replace_art(&mut self.db, &crate::db::data_dir(), id, None) {
                self.toast_err(e.to_string());
            }
        }
    }

    /// "Add Artwork" / "Change Artwork": the file panel, then the picture is stored.
    pub fn playlist_art_choose(&mut self, id: i64) {
        let mut d = rfd::FileDialog::new().set_title("Select Artwork").add_filter("*.jpg;*.jpeg;*.png", &["jpg", "jpeg", "png"]);
        if let Some(pics) = dirs_pictures() {
            d = d.set_directory(pics);
        }
        if let Some(f) = d.pick_file() {
            if let Err(e) = self.playlist_art_set(id, &f) {
                self.toast_err(e.to_string());
            }
        }
    }
}

fn dirs_pictures() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Pictures")).filter(|p| p.is_dir())
}

/// The playlist menu's artwork items (called from `ui_library_ext::node_menu`).
pub fn menu(app: &mut App, ui: &mut egui::Ui, id: i64) {
    match app.playlist_art(id) {
        None => {
            if ui.button("Add Artwork").clicked() {
                ui.close_menu();
                app.playlist_art_choose(id);
            }
        }
        Some(path) => {
            // The artwork as the first item: fitted into 100 x 100, centred, 10 pt down.
            let w = ui.available_width().max(120.0);
            let (r, _) = ui.allocate_exact_size(egui::vec2(w, 120.0), egui::Sense::hover());
            if let Some(tex) = crate::ui_browser::artwork_texture(app, &path) {
                let [tw, th] = tex.size();
                let k = (100.0 / tw.max(1) as f32).min(100.0 / th.max(1) as f32);
                let size = egui::vec2(tw as f32 * k, th as f32 * k);
                let area = egui::Rect::from_min_size(egui::pos2(r.center().x - 50.0, r.min.y + 10.0), egui::vec2(100.0, 100.0));
                let img = egui::Rect::from_center_size(area.center(), size);
                ui.painter().image(tex.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            if ui.button("Change Artwork").clicked() {
                ui.close_menu();
                app.playlist_art_choose(id);
            }
            if ui.button("Delete Artwork").clicked() {
                ui.close_menu();
                app.playlist_art_delete(id);
            }
        }
    }
    ui.separator();
}

#[cfg(test)]
mod tests {
    #[test]
    fn artwork_replacement_preserves_source_and_rolls_back_on_save_failure() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut db = crate::db::Db::open(&root.join("library.db")).unwrap();
        let src = root.join("original.png");
        std::fs::write(&src, b"original").unwrap();
        super::replace_art(&mut db, root, 5, Some(&src)).unwrap();
        let saved = db.setting(&super::setting_key(5)).unwrap();
        let copy = super::managed_art(root, 5, &saved).unwrap();
        assert!(super::managed_art(root, 6, &saved).is_none());
        assert_eq!(copy.parent(), Some(root.join("artwork").as_path()));
        db.conn.execute_batch("CREATE TRIGGER reject_art BEFORE INSERT ON settings BEGIN SELECT RAISE(ABORT, 'test'); END").unwrap();
        assert!(super::replace_art(&mut db, root, 5, Some(&src)).is_err());
        assert_eq!(db.setting(&super::setting_key(5)).unwrap(), saved);
        assert_eq!(std::fs::read(&copy).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(copy.parent().unwrap()).unwrap().count(), 1);
        db.conn.execute_batch("DROP TRIGGER reject_art").unwrap();
        super::replace_art(&mut db, root, 5, None).unwrap();
        assert!(!copy.exists());
        db.set_setting(&super::setting_key(5), src.to_str().unwrap());
        super::replace_art(&mut db, root, 5, None).unwrap();
        assert_eq!(std::fs::read(src).unwrap(), b"original");
    }
}
