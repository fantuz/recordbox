//! Preferences > Advanced (Database, Browse, Recordings, Others), Extensions > STEMS and the
//! DJ System items that were missing, laid out by rekordbox 7.2.19's own item bounds (tree
//! items 166-217, 249-256). recordbox's own settings, which rekordbox has no place for, are
//! at the end of Advanced > Others under a "recordbox" heading.

use crate::app::*;
use crate::prefs_rb::Mode;
use crate::theme::*;
use crate::ui_kit::Just;
use crate::ui_prefs_item::{item, item_bare, Item};
use crate::ui_prefs_perf::{heading, STORED};
use eframe::egui;
use egui::Ui;

fn stored(it: &mut Item, what: &str) {
    it.hint(format!("{what} {STORED}"));
}

const NO_STREAMING: &str = "recordbox has no streaming services.";
const NO_FILE: &str = "(no recently selected files)";

pub fn page_advanced(app: &mut App, ui: &mut Ui, tab: &str, mode: Mode) {
    let before = app.prefs.rb.clone();
    match tab {
        "Database" => database(app, ui, mode),
        "Browse" => browse(app, ui, mode),
        "Recordings" => crate::ui_rec::prefs_page(app, ui),
        _ => others(app, ui, mode),
    }
    if app.prefs.rb != before {
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Database

fn database(app: &mut App, ui: &mut Ui, mode: Mode) {
    let export = mode == Mode::Export;
    let mut dirty = false;
    let mut pick_itunes = false;
    item(ui, if export { "DetailiTunes1" } else { "DetailiTunes2" }, if export { 296.0 } else { 232.0 }, |it| {
        it.title("iTunes");
        it.sub(52.0, 56.0, "iTunes Library File");
        let p = &mut app.prefs;
        it.hint("Framework (Software): the Music app's library is read directly. iTunes Library XML: the file chosen below.");
        if it.radio(84.0, 90.0, p.rb.itunes_framework, "Framework (Software)") {
            p.rb.itunes_framework = true;
        }
        if it.radio(84.0, 116.0, !p.rb.itunes_framework, "iTunes Library XML") {
            p.rb.itunes_framework = false;
        }
        it.no_hint();
        it.en = !p.rb.itunes_framework;
        let shown = p.itunes_xml.clone().unwrap_or_else(|| NO_FILE.to_string());
        let mut clear = false;
        it.combo(84.0, 142.0, 414.0, &shown, |ui| {
            if ui.selectable_label(p.itunes_xml.is_none(), NO_FILE).clicked() {
                clear = true;
            }
        });
        if clear && p.itunes_xml.is_some() {
            p.itunes_xml = None;
            dirty = true;
        }
        pick_itunes = it.button(508.0, 142.0, 100.0, 22.0, "Browse").clicked();
        it.en = true;
        it.sep(180.0);
        stored(it, "");
        it.pill(84.0, 196.0, &mut p.rb.itunes_grouping_to_label, "Convert iTunes \"Grouping\" to rekordbox \"Label\"");
        if export {
            it.sep(228.0);
            it.pill_h(84.0, 244.0, 38.0, &mut p.rb.itunes_overwrite_sync, "Overwrite and update iTunes track information \nwhen synchronized with SYNC MANAGER");
        }
    });
    if pick_itunes {
        if let Some(f) = rfd::FileDialog::new().add_filter("iTunes Library", &["xml"]).pick_file() {
            app.prefs.itunes_xml = Some(f.to_string_lossy().to_string());
            dirty = true;
        }
    }
    let mut pick_xml = false;
    item(ui, "DetailrbXml1", 240.0, |it| {
        it.title("rekordbox xml");
        let p = &mut app.prefs;
        it.child(20.0, 56.0, |it| {
            it.sub(32.0, 0.0, "Export BeatGrid information");
            it.hint("No: the exported tracks have no TEMPO entries. BPM change points: one where the tempo or the beat's place changes (`bridgeExportBeatGrid` 0 / 1, `DetailrbXmlExportBeatInfoComponent::buttonClicked` 0x10143f39c).");
            if it.radio(64.0, 34.0, p.rb.xml_beatgrid == 0, "No") {
                p.rb.xml_beatgrid = 0;
            }
            if it.radio(64.0, 60.0, p.rb.xml_beatgrid >= 1, "BPM change points") {
                p.rb.xml_beatgrid = 1;
            }
            it.no_hint();
        });
        it.sep(148.0);
        it.child(20.0, 164.0, |it| {
            it.sub(32.0, 0.0, "Imported Library");
            let shown = p.rekordbox_xml_path.clone().unwrap_or_else(|| NO_FILE.to_string());
            let mut clear = false;
            it.hint("The rekordbox xml file shown under \"rekordbox xml\" in the Tree View.");
            it.combo(64.0, 34.0, 414.0, &shown, |ui| {
                if ui.selectable_label(p.rekordbox_xml_path.is_none(), NO_FILE).clicked() {
                    clear = true;
                }
            });
            if clear && p.rekordbox_xml_path.is_some() {
                p.rekordbox_xml_path = None;
                dirty = true;
            }
            pick_xml = it.button(488.0, 34.0, 100.0, 22.0, "Browse").clicked();
        });
    });
    if pick_xml {
        if let Some(f) = rfd::FileDialog::new().add_filter("XML", &["xml"]).pick_file() {
            app.prefs.rekordbox_xml_path = Some(f.to_string_lossy().to_string());
            dirty = true;
        }
    }
    let (mut add, mut del) = (false, false);
    let sel_id = egui::Id::new("prefs_relocate_sel");
    let mut sel: usize = ui.ctx().data(|d| d.get_temp(sel_id)).unwrap_or(0);
    item(ui, "DetailAutoRelocate", 202.0, |it| {
        it.title("Auto Relocate Search Folders");
        let p = &mut app.prefs;
        it.hint("Track > Relocate > Auto Relocate searches the ticked folders (and the listed ones with \"Specified user folders\").");
        it.check(52.0, 56.0, &mut p.rb.relocate_music, "Music");
        it.check(52.0, 82.0, &mut p.rb.relocate_video, "Video");
        it.check(52.0, 108.0, &mut p.rb.relocate_desktop, "Desktop");
        it.hint("Track > Relocate > Auto Relocate looks in the folders of this list (and the folders inside them) for missing files.");
        it.check(52.0, 134.0, &mut p.rb.relocate_user, "Specified user folders");
        sel = sel.min(p.relocate_folders.len().saturating_sub(1));
        let shown = p.relocate_folders.get(sel).cloned().unwrap_or_else(|| "(no choices)".to_string());
        let mut pick = None;
        it.combo(42.0, 160.0, 358.0, &shown, |ui| {
            for (i, f) in p.relocate_folders.iter().enumerate() {
                if ui.selectable_label(i == sel, f.as_str()).clicked() {
                    pick = Some(i);
                }
            }
        });
        if let Some(i) = pick {
            sel = i;
        }
        add = it.button(405.0, 160.0, 100.0, 22.0, "Add").clicked();
        it.en = !p.relocate_folders.is_empty();
        del = it.button(508.0, 160.0, 100.0, 22.0, "Del").clicked();
    });
    if add {
        if let Some(d) = rfd::FileDialog::new().pick_folder() {
            let s = d.to_string_lossy().to_string();
            if !app.prefs.relocate_folders.contains(&s) {
                app.prefs.relocate_folders.push(s);
                app.prefs.rb.relocate_user = true;
                sel = app.prefs.relocate_folders.len() - 1;
                dirty = true;
            }
        }
    }
    if del && sel < app.prefs.relocate_folders.len() {
        app.prefs.relocate_folders.remove(sel);
        dirty = true;
    }
    ui.ctx().data_mut(|d| d.insert_temp(sel_id, sel));
    item_bare(ui, "DetailExplanatoryMessage", 42.0, |it| {
        it.text(20.0, 20.0, 580.0, 22.0, "The settings below work when tracks in an external drive are used", 20.0, Just::TopLeft);
    });
    if export {
        item(ui, "DetailAutoExport", 92.0, |it| {
            it.title_tip("Auto Export", "When a device is connected, the playlists exported to it before are exported again with their changes.");
            it.hint("rekordbox acts on this after an import, for files inside folders linked to devices (`DatabaseMediator::executeImport` 0x101a0dba4); recordbox has no such links. recordbox keeps this setting; nothing acts on it yet.");
            it.pill2(52.0, 56.0, &mut app.prefs.rb.auto_export, "Enable", "Disable");
        });
    }
    if mode != Mode::Lighting {
        let mut show = false;
        item(ui, "DetailDatabaseManagement", 132.0, |it| {
            it.title_tip("Database management", "rekordbox can move its database to an external drive. recordbox keeps its library in its own folder on this computer.");
            it.sub(52.0, 56.0, "Select a drive");
            it.en = false;
            it.hint("recordbox's library stays in its own folder on this computer.");
            it.combo(88.0, 90.0, 360.0, "Macintosh HD", |_| {});
            it.button(458.0, 90.0, 150.0, 22.0, "Move Database");
            it.en = true;
            it.no_hint();
            let _ = &mut show;
        });
    }
    if dirty {
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Browse

fn browse(app: &mut App, ui: &mut Ui, mode: Mode) {
    let mut dirty = false;
    let mut sync_tags = false;
    item(ui, "DetailMyTag", 92.0, |it| {
        it.title("My Tag");
        it.hint("A track's My Tags are added at the end of its comment as /* Tag / Tag */ whenever its tags change.");
        if it.pill(52.0, 56.0, &mut app.prefs.mytag_to_comments, "Add \"My Tag\" to the \"Comments\"") {
            dirty = true;
            sync_tags = app.prefs.mytag_to_comments;
        }
    });
    if sync_tags {
        let ids: Vec<i64> = app.track_tags.iter().filter(|(_, s)| !s.is_empty()).map(|(id, _)| *id).collect();
        crate::prefs_ext::sync_mytag_comments(app, &ids);
        app.list_dirty = true;
    }
    // Color: the eight colour comments, two columns of four.
    item(ui, "DetailColor", 210.0, |it| {
        it.title("Color");
        for i in 0..8usize {
            let (x, y) = (52.0 + 280.0 * (i / 4) as f32, 56.0 + 34.0 * (i % 4) as f32);
            let o = it.painter().1;
            it.ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(o.x + x, o.y + y + 3.0), egui::vec2(16.0, 16.0)), 0.0, label_color(i as u8 + 1));
            let r = egui::Rect::from_min_size(egui::pos2(o.x + x + 26.0, o.y + y), egui::vec2(200.0, 22.0));
            it.ui.painter().rect_filled(r, 3.0, egui::Color32::BLACK);
            let resp = it.ui.put(r.shrink2(egui::vec2(4.0, 1.0)), egui::TextEdit::singleline(&mut app.prefs.color_names[i]).frame(false).font(crate::ui_kit::pref(16.0)).text_color(WHITE).hint_text(crate::model::COLOR_NAMES[i + 1]));
            dirty |= resp.changed();
        }
    });
    let p = &mut app.prefs.rb;
    item(ui, "DetailDisplaySpeed", 92.0, |it| {
        it.title_tip("Display speed", "Playlists are kept ready so that they open quicker.");
        it.hint("recordbox always keeps its lists ready, so this changes nothing (recordbox keeps the setting).");
        it.pill(52.0, 56.0, &mut p.cache_track_list, "Display playlists quicker.");
    });
    item(ui, "DetailContextMenu", 92.0, |it| {
        it.title("Drop-down menu");
        it.hint("On: holding a track for a second opens its menu, as a right click does.");
        it.pill(52.0, 56.0, &mut p.long_press_menu, "Display the menu when long-pressing a track for 1 second.");
    });
    item(ui, "DetailTagList", 118.0, |it| {
        it.title("Tag List");
        it.hint("A track loaded from the Tag List leaves it; all tracks leave it when recordbox quits. Both off by default.");
        // (Two check boxes of their own, as rekordbox's `DetailTagList` has them.)
        it.check(52.0, 56.0, &mut p.tag_list_on_load, "Delete a track from Tag List when the track is loaded.");
        it.check(52.0, 82.0, &mut p.tag_list_on_exit, "Delete all tracks from Tag List when exiting rekordbox.");
    });
    item(ui, "DetailLibraryProtection", 92.0, |it| {
        it.title("Library Protection");
        it.hint("On: edits, deletions and imports are refused (\"Unable to edit or delete because Library Protection is enabled.\").");
        it.pill(52.0, 56.0, &mut p.library_protection, "Protect library edit.");
    });
    item(ui, "DetailQuickLibraryEdit", 92.0, |it| {
        it.title_tip("Library Edit", "With this on, a double click on a cell of the track list edits it.");
        it.hint("On: a double click on a cell edits it (and a single click on the selected row no longer does).");
        it.pill(52.0, 56.0, &mut p.inline_edit_dblclick, "Double-click");
    });
    if mode == Mode::Performance {
        item(ui, "DetailPlaybackTime", 142.0, |it| {
            it.title("Playback time setting");
            it.text(52.0, 56.0, 580.0, 18.0, "The length of time before a track is added to Histories", 16.0, Just::Left);
            it.text(52.0, 72.0, 580.0, 18.0, "(Length of time before played tracks are displayed in color)", 16.0, Just::Left);
            it.text(52.0, 104.0, 100.0, 20.0, &format!("{} sec", p.played_time), 14.0, Just::Centre);
            it.hint("A track goes to the day's history after it has played this long without a pause.");
            let mut v = p.played_time.clamp(1, 60) as u32 - 1;
            if it.steps(160.0, 104.0, 400.0, &mut v, 59) {
                p.played_time = v as u8 + 1;
            }
        });
    }
    if dirty {
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Others

/// A card of a streaming service: its title and radio buttons, greyed.
fn streaming(ui: &mut Ui, name: &'static str, title: &str, h: f32, options: &[&str]) {
    item(ui, name, h, |it| {
        it.title(title);
        it.sub(52.0, 56.0, "Streaming Audio Quality");
        it.en = false;
        it.hint(NO_STREAMING);
        for (i, o) in options.iter().enumerate() {
            it.radio(88.0, 90.0 + 26.0 * i as f32, i == 0, o);
        }
    });
}

fn others(app: &mut App, ui: &mut Ui, mode: Mode) {
    let export = mode == Mode::Export;
    let mut dirty = false;
    if export {
        item(ui, "DetailPCName", 128.0, |it| {
            it.title("Computer Name");
            it.sub(52.0, 56.0, "Export Name");
            let o = it.painter().1;
            let r = egui::Rect::from_min_size(egui::pos2(o.x + 88.0, o.y + 90.0), egui::vec2(520.0, 22.0));
            it.ui.painter().rect_filled(r, 3.0, egui::Color32::BLACK);
            let host = it.ui.ctx().data_mut(|d| d.get_temp_mut_or_insert_with(egui::Id::new("prefs_host"), || {
                #[cfg(target_os = "macos")]
                if let Some(name) = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output().ok()
                    .filter(|out| out.status.success()).map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|name| !name.is_empty()) {
                    return name;
                }
                std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok().filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "This computer".into())
            }).clone());
            it.ui.put(r.shrink2(egui::vec2(4.0, 1.0)), egui::TextEdit::singleline(&mut app.prefs.rb.export_name).frame(false).font(crate::ui_kit::pref_bold(16.0)).text_color(WHITE).hint_text(host)).on_hover_text(format!("The name this computer has for players and devices. {STORED}"));
        });
        item(ui, "DetailQuantizeBeat", 98.0, |it| {
            it.title("QUANTIZE BEAT VALUE");
            it.hint("Cues and loops set while QUANTIZE is on go to the nearest line of this grid (1/1 = one beat).");
            let p = &mut app.prefs;
            let mut sel = (crate::prefs_rb::RbPrefs::quantize_id_of(p.quantize_div.max(1)).clamp(2, 6) - 2) as usize;
            if it.choice(52.0, 56.0, 556.0, &mut sel, &crate::prefs_rb::Q_BEATS[1..]) {
                p.rb.deck_quantize = sel as u8 + 2;
                p.quantize_div = crate::prefs_rb::RbPrefs::quantize_div_of(p.rb.deck_quantize);
                dirty = true;
            }
        });
        crate::ui_prefs_perf::sync_type_item(ui, &mut app.prefs);
        crate::ui_prefs_perf::hot_cue_item(ui, &mut app.prefs);
        item(ui, "DetailLoopWave", 152.0, |it| {
            it.title("Export Loop As WAV");
            it.sub(52.0, 56.0, "Sample Rate");
            it.hint("The sample rate of loops saved with Export Loop As WAV.");
            let p = &mut app.prefs.rb;
            if it.radio(88.0, 90.0, p.loop_wav_rate == 48000, "48kHz") {
                p.loop_wav_rate = 48000;
            }
            if it.radio(88.0, 116.0, p.loop_wav_rate == 44100, "44.1kHz") {
                p.loop_wav_rate = 44100;
            }
        });
    }
    // The services recordbox does not have, listed as rekordbox lists them, greyed.
    item(ui, "DetailTribeXR", 192.0, |it| {
        it.title("Tribe XR");
        it.sub(52.0, 56.0, "XML Auto Export");
        it.en = false;
        it.hint("recordbox has no Tribe XR link.");
        let mut off = false;
        it.pill2(88.0, 90.0, &mut off, "Enable", "Disable");
        it.en = true;
        it.sub(52.0, 116.0, "Location of the xml file on dropbox");
        it.en = false;
        it.combo(88.0, 150.0, 410.0, NO_FILE, |_| {});
        it.button(508.0, 150.0, 100.0, 22.0, "Browse");
    });
    item(ui, "DetailAppleMusic", 92.0, |it| {
        it.title("Apple Music");
        it.en = false;
        it.hint(NO_STREAMING);
        let mut off = false;
        it.pill(52.0, 56.0, &mut off, "Explicit");
    });
    if mode != Mode::Edit {
        streaming(ui, "DetailSpotify", "Spotify", 152.0, &["Default (Ogg 320kbps)", "Low (Ogg 96kbps)"]);
    }
    if matches!(mode, Mode::Performance | Mode::Lighting) {
        streaming(ui, "DetailSoundCloud", "SoundCloud", 152.0, &["High Quality 256kbps(AAC)", "Standard Quality 160kbps or 96kbps(AAC)"]);
    }
    if mode != Mode::Edit {
        streaming(ui, "DetailBeatport", "Beatport", 178.0, &["Lossless (FLAC)", "High (256 AAC)", "Standard (128 AAC)"]);
        streaming(ui, "DetailTidal", "Tidal", 152.0, &["Lossless (FLAC)", "High (AAC)"]);
        item(ui, "DetailAutoDelete", 150.0, |it| {
            it.title("Automatic removal of auditioned tracks");
            it.en = false;
            it.hint(NO_STREAMING);
            let mut off = false;
            it.pill(52.0, 64.0, &mut off, "");
            it.text(89.0, 56.0, 500.0, 36.0, "Remove streaming tracks that has been loaded only for\nauditioning from the Collection.", 16.0, Just::Left);
            it.pill_h(52.0, 98.0, 38.0, &mut off, "Tracks with CUE points set are also eligible \nfor removal");
        });
    }
    item(ui, "DetailTrackInformation", 180.0, |it| {
        it.title_tip("Track information", "rekordbox can make the tracks you play public on KUVO. recordbox publishes nothing.");
        it.sub(52.0, 56.0, "Collective settings of all tracks");
        it.en = false;
        it.hint("recordbox publishes nothing about your tracks.");
        it.button(88.0, 90.0, 200.0, 22.0, "Public");
        it.button(296.0, 90.0, 200.0, 22.0, "Private");
        let mut off = false;
        it.pill_h(88.0, 128.0, 38.0, &mut off, "Make the track information public \nwhen importing tracks");
    });
    item(ui, "DetailInflyte", 132.0, |it| {
        it.title("Inflyte");
        it.sub(52.0, 56.0, "Location of Files");
        it.en = false;
        it.hint("recordbox has no Inflyte link.");
        it.combo(88.0, 90.0, 410.0, NO_FILE, |_| {});
        it.button(508.0, 90.0, 100.0, 22.0, "Browse");
    });
    own(app, ui, &mut dirty);
    if dirty {
        app.save_prefs();
    }
}

/// recordbox's own settings: what rekordbox has no setting for.
fn own(app: &mut App, ui: &mut Ui, dirty: &mut bool) {
    heading(ui, "RecordboxText", "recordbox");
    let mut act: Option<&'static str> = None;
    item(ui, "RecordboxLibrary", 196.0, |it| {
        it.title("Library");
        it.hint("Startup import reads the AlphaTheta Rekordbox library into recordbox. To write back selected changes, review them in library sync and confirm the backup.");
        *dirty |= it.pill(52.0, 56.0, &mut app.prefs.import_rekordbox_at_startup, "Import the AlphaTheta Rekordbox library at startup");
        it.en = app.can_review_library_sync();
        if it.button(52.0, 88.0, 150.0, 22.0, "Review library sync…").clicked() {
            app.dialogs.prefs_open = false;
            act = Some("sync_rekordbox");
        }
        it.en = app.dialogs.ext.backup_job.is_none();
        it.hint("A backup is one .zip file with the database, the artwork and, if you choose, the music files.");
        if it.button(212.0, 88.0, 150.0, 22.0, "Backup Library").clicked() {
            act = Some("backup");
        }
        it.en &= app.dialogs.library_sync_review.is_none() && app.rb_job.is_none();
        if it.button(372.0, 88.0, 150.0, 22.0, "Restore Library").clicked() {
            act = Some("restore");
        }
        it.en = true;
        it.sep(126.0);
        it.hint("Edits are saved in this active recordbox library. Library sync can write reviewed changes back to AlphaTheta Rekordbox.");
        it.field(52.0, 142.0, 446.0, app.db.active_library_path());
        if it.button(508.0, 142.0, 100.0, 22.0, "Show").clicked() {
            act = Some("library_folder");
        }
    });
    if let Some(a) = act {
        crate::menus::handle(app, a);
    }
    item(ui, "RecordboxPlayer", 118.0, |it| {
        it.title("Time Mode");
        let p = &mut app.prefs;
        if it.radio(52.0, 56.0, !p.time_remain, "Elapsed") {
            p.time_remain = false;
            *dirty = true;
        }
        if it.radio(52.0, 82.0, p.time_remain, "Remaining") {
            p.time_remain = true;
            *dirty = true;
        }
    });
    let n = crate::prefs_ext::TREE_ITEMS_OWN.len() as f32;
    item(ui, "RecordboxBrowser", 56.0 + 26.0 * (n + 3.0) + 16.0 + 10.0, |it| {
        it.title("Browser");
        let p = &mut app.prefs;
        it.hint("Sources of the Tree View that rekordbox always shows.");
        for (i, name) in crate::prefs_ext::TREE_ITEMS_OWN.iter().enumerate() {
            let mut on = p.tree_item(name);
            if it.check(52.0, 56.0 + 26.0 * i as f32, &mut on, name) {
                p.tree_items.insert(name.to_string(), on);
                *dirty = true;
            }
        }
        it.no_hint();
        let y = 56.0 + 26.0 * n + 16.0;
        it.sep(y - 16.0);
        *dirty |= it.pill(52.0, y, &mut p.show_mytag, "My Tag Configuration Window");
        if it.pill(52.0, y + 26.0, &mut p.column_browser, "Column browser (Genre / Artist / Album)") {
            *dirty = true;
        }
        *dirty |= it.pill(52.0, y + 52.0, &mut p.sub_browser, "Sub-Browser Window");
    });
    if *dirty {
        app.list_dirty = true;
    }
}

// ------------------------------------------------------------------ DJ System additions

/// DJ System > General: the two items after "Type of the Overview Waveform".
pub fn dj_general_more(app: &mut App, ui: &mut Ui) {
    let p = &mut app.prefs.rb;
    let before = p.clone();
    item(ui, "DJSystemKeyOption", 152.0, |it| {
        it.title("Key display format");
        it.sub(52.0, 56.0, "The key display format displayed on CDJ/XDJ.");
        stored(it, "Where the players keep this choice has not been confirmed, so it is not written to the device.");
        if it.radio(88.0, 90.0, !p.cdj_key_alnum, "Classic") {
            p.cdj_key_alnum = false;
        }
        if it.radio(88.0, 116.0, p.cdj_key_alnum, "Alphanumeric") {
            p.cdj_key_alnum = true;
        }
    });
    item(ui, "DJSystemImageOnJogDisplay", 259.0, |it| {
        it.title("Image On-Jog Display");
        it.sub(52.0, 56.0, "Image displayed on the CDJ/XDJ jog display");
        stored(it, "recordbox writes each track's artwork for the jog display; a custom image is not written.");
        let mut art = !p.jog_custom_image;
        let mut custom = p.jog_custom_image;
        if it.pill(88.0, 90.0, &mut art, "Artwork") {
            p.jog_custom_image = !art;
        }
        if it.pill(88.0, 116.0, &mut custom, "Custom Image") {
            p.jog_custom_image = custom;
        }
        let o = it.painter().1;
        let r = egui::Rect::from_min_size(egui::pos2(o.x + 88.0, o.y + 142.0), egui::vec2(100.0, 100.0));
        it.ui.painter().rect_filled(r, 0.0, egui::Color32::BLACK);
        it.ui.painter().circle_stroke(r.center(), 46.0, egui::Stroke::new(1.0, hex(0x323232)));
        it.en = false;
        it.text(245.0, 163.0, 325.0, 20.0, "Import image file", 15.0, Just::Left);
        it.text(245.0, 201.0, 325.0, 20.0, "Delete image", 15.0, Just::Left);
    });
    if app.prefs.rb != before {
        app.save_prefs();
    }
}

/// DJ System > Others: LINK MONITOR.
pub fn dj_link_monitor(app: &mut App, ui: &mut Ui) {
    let before = app.prefs.rb.link_monitor;
    item(ui, "DJSystemLinkMonitor", 92.0, |it| {
        it.title("LINK MONITOR");
        it.hint(format!("recordbox has no PRO DJ LINK connection to a mixer. {STORED}"));
        it.pill(52.0, 56.0, &mut app.prefs.rb.link_monitor, "Use \"LINK MONITOR\" of DJ Mixers");
    });
    if app.prefs.rb.link_monitor != before {
        app.save_prefs();
    }
}

// ------------------------------------------------------------------ Extensions > STEMS

pub fn page_stems(app: &mut App, ui: &mut Ui) {
    let before = (app.prefs.rb.clone(), app.prefs.show_stems, app.prefs.stem_parts, app.prefs.stem_waveform, app.prefs.stem_reset_on_load);
    item_bare(ui, "ExtensionEnableSTEMS", 39.0, |it| {
        it.pill(20.0, 20.0, &mut app.prefs.show_stems, "Enable the STEMS Function");
    });
    item(ui, "ExtensionSTEMSMode", 118.0, |it| {
        it.title("STEMS mode");
        if it.radio(52.0, 56.0, app.prefs.stem_parts == 3, "3 Stems(VOCAL, INST, DRUMS)") {
            app.prefs.stem_parts = 3;
        }
        if it.radio(52.0, 82.0, app.prefs.stem_parts != 3, "4 Stems(VOCAL, INST, BASS, DRUMS)") {
            app.prefs.stem_parts = 4;
        }
    });
    let [quality, speed] = &crate::stems::CATALOG;
    let cur = app.stem.model.clone().or_else(|| app.prefs.stem_model.clone()).unwrap_or_default();
    let able = app.stem.hw.as_ref().is_none_or(|hw| hw.ram_gb >= quality.min_ram_gb && crate::stems::backend_status().0);
    let mut pick = None;
    item(ui, "ExtensionPartAnalysisQuality", 118.0, |it| {
        it.title_tip("STEMS Analysis Process", &format!("Prioritize sound quality: {} - {}\nPrioritize speed: {} - {}", quality.name, quality.about, speed.name, speed.about));
        if !able && cur != quality.id {
            it.en = false;
            it.hint(format!("{} requires GPU acceleration and {:.0} GB of system RAM. Selected backend: {}.", quality.name, quality.min_ram_gb, crate::stems::backend_status().1));
        }
        if it.radio(52.0, 54.0, cur == quality.id, "Prioritize sound quality") {
            pick = Some(quality);
        }
        it.en = true;
        it.no_hint();
        if it.radio(52.0, 80.0, cur != quality.id, "Prioritize speed") {
            pick = Some(speed);
        }
    });
    if let Some(m) = pick.filter(|m| m.id != cur) {
        app.select_stem_model(m);
    }
    let four = app.prefs.stem_parts != 3;
    item(ui, "ExtensionLayout", 210.0, |it| {
        it.title("Layout");
        it.sub(52.0, 56.0, "ACTIVE STEM/STEM ISO/STEM FX Layout");
        let p = &mut app.prefs.rb;
        stored(it, "recordbox has one layout for the part buttons.");
        if it.radio(88.0, 90.0, p.part_layout == 0, "Auto") {
            p.part_layout = 0;
        }
        if it.radio(88.0, 116.0, p.part_layout == 1, "Customize") {
            p.part_layout = 1;
        }
        // (The list for the number of stems in use, then the four custom layouts.)
        let names: [&str; 6] = if four { ["DRUMS-VOCAL-INST-BASS", "DRUMS-BASS-INST-VOCAL", "CUSTOM1", "CUSTOM2", "CUSTOM3", "CUSTOM4"] } else { ["DRUMS-VOCAL-INST", "DRUMS-INST-VOCAL", "CUSTOM1", "CUSTOM2", "CUSTOM3", "CUSTOM4"] };
        it.en = p.part_layout == 1;
        let mut sel = (p.custom_part_layout.clamp(1, 6) - 1) as usize;
        if it.choice(88.0, 142.0, 520.0, &mut sel, &names) {
            p.custom_part_layout = sel as u8 + 1;
        }
        it.en = true;
        if it.radio(88.0, 174.0, p.part_layout == 2, "Do not display") {
            p.part_layout = 2;
        }
    });
    item(ui, "ExtensionStemWaveform", 92.0, |it| {
        it.title_tip("STEM Waveform Display", "Enable: Displays the output Stem.\nDisable: Displays the original waveform.");
        it.pill2(52.0, 56.0, &mut app.prefs.stem_waveform, "Enable", "Disable");
    });
    let mut solo = None;
    item(ui, "ExtensionActiveSTEM", 144.0, |it| {
        it.title("ACTIVE STEM Setting");
        if it.radio(52.0, 56.0, !app.prefs.stem_solo, "MUTE") {
            solo = Some(false);
        }
        if it.radio(52.0, 82.0, app.prefs.stem_solo, "SOLO") {
            solo = Some(true);
        }
        it.pill(52.0, 108.0, &mut app.prefs.stem_reset_on_load, "Reset the MUTE/SOLO state while loading");
    });
    if let Some(s) = solo.filter(|s| *s != app.prefs.stem_solo) {
        crate::prefs_ext::set_stem_solo(app, s);
        app.save_prefs();
    }
    let p = &mut app.prefs.rb;
    item(ui, "ExtensionInstantDoubles", 118.0, |it| {
        it.title_tip("STEM Instant Doubles", "Which decks the parts of a track are doubled onto.");
        stored(it, "");
        if it.radio(52.0, 56.0, p.part_inst_doubles_lr, "Left/Right decks") {
            p.part_inst_doubles_lr = true;
        }
        if it.radio(52.0, 82.0, !p.part_inst_doubles_lr, "Up/Down decks") {
            p.part_inst_doubles_lr = false;
        }
    });
    item(ui, "ExtensionMemory", 92.0, |it| {
        it.title_tip("Memory", "Lets the analysis use more memory.");
        stored(it, "recordbox's separation sizes itself to this computer's memory.");
        it.pill(52.0, 56.0, &mut p.part_more_memory, "Increase the memory size of the analysis process");
    });
    item(ui, "ExtensionMultiThread", 92.0, |it| {
        it.title_tip("Multi-thread", "Runs the analysis on several processor cores.");
        stored(it, "recordbox's separation runs on the GPU where there is one.");
        it.pill(52.0, 56.0, &mut p.part_multithread, "Apply multi-thread to the analysis process");
    });
    item_bare(ui, "ExtensionEnableGrooveCircuit", 39.0, |it| {
        it.hint(format!("`prefs.rb.groove_circuit`. {STORED}"));
        it.pill(20.0, 20.0, &mut p.groove_circuit, "Enable the GROOVE CIRCUIT Function");
    });
    item(ui, "ExtensionDrumSwap", 208.0, |it| {
        it.title("DRUM SWAP");
        it.sub(52.0, 56.0, "Eject/Load Lock");
        stored(it, "");
        if it.radio(84.0, 90.0, p.drum_swap_lock, "Lock") {
            p.drum_swap_lock = true;
        }
        if it.radio(84.0, 116.0, !p.drum_swap_lock, "Unlock") {
            p.drum_swap_lock = false;
        }
        it.sep(156.0);
        it.pill(52.0, 172.0, &mut p.drum_swap_title, "Display track titles on DRUM SWAP SLOT");
    });
    let mut pick_dir = false;
    item(ui, "ExtensionDrumCapture", 214.0, |it| {
        it.title("DRUM CAPTURE");
        stored(it, "");
        if it.radio(52.0, 56.0, p.drum_capture_stem == 1, "DRUMS Stem") {
            p.drum_capture_stem = 1;
        }
        if it.radio(52.0, 82.0, p.drum_capture_stem == 2, "Original track") {
            p.drum_capture_stem = 2;
        }
        it.sep(122.0);
        it.sub(52.0, 138.0, "Location of Files");
        let shown = if p.drum_capture_dir.is_empty() { NO_FILE.to_string() } else { p.drum_capture_dir.clone() };
        let mut clear = false;
        it.combo(84.0, 172.0, 420.0, &shown, |ui| {
            if ui.selectable_label(p.drum_capture_dir.is_empty(), NO_FILE).clicked() {
                clear = true;
            }
        });
        if clear {
            p.drum_capture_dir.clear();
        }
        pick_dir = it.button(508.0, 172.0, 100.0, 22.0, "Browse").clicked();
    });
    if pick_dir {
        if let Some(d) = rfd::FileDialog::new().set_title("Location of Files").pick_folder() {
            p.drum_capture_dir = d.to_string_lossy().to_string();
        }
    }
    item(ui, "ExtensionDrumLoopAudio", 92.0, |it| {
        it.title("Drum Loop Audio");
        stored(it, "");
        it.pill(52.0, 56.0, &mut p.drum_auto_import, "Automatically import audio files");
    });
    crate::ui_prefs::stem_model_card(app, ui);
    let after = (app.prefs.rb.clone(), app.prefs.show_stems, app.prefs.stem_parts, app.prefs.stem_waveform, app.prefs.stem_reset_on_load);
    if after != before {
        app.save_prefs();
    }
}
