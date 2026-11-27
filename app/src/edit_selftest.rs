//! PREPDECK_SHOT_PLAYER=edittest (with --screenshot, PREPDECK_SHOT_VIEW=editloadpads and
//! PREPDECK_BENCH=<frames> to keep the window up): operates EDIT mode with real pointer and
//! key events, one control after another, and prints the state after each step. A stand-in
//! for a hand test where the screen cannot be taken over. Positions come from where the
//! controls were drawn in the frame before (`EditState::map`). Saving uses the file named by
//! PREPDECK_EDIT_SAVE instead of the save panel.

use crate::app::*;
use crate::edit_audio::EDIT;
use crate::edit_model::*;
use crate::ui_edit::*;
use eframe::egui;
use egui::{pos2, Pos2};

const STEP: u32 = 30;

const STEPS: &[&str] = &[
    "CALL:setup|palette shown, loop on",
    "WAVE:8|click at beat 8 (point, play position follows)",
    "DRAG:16:24|drag from beat 16 to beat 24",
    "SHIFT:32|shift-click at beat 32 (the nearer edge moves)",
    "CLICK:beat4|BEAT SELECT 4",
    "CLICK:beat16|BEAT SELECT 16",
    "CLICK:tab_select|SELECT tab",
    "CLICK:half|Half",
    "CLICK:double|Double",
    "CLICK:cancel|Cancel",
    "CLICK:tab_beat|BEAT SELECT tab",
    "CLICK:beat8|BEAT SELECT 8",
    "CLICK:clone|CLONE",
    "CLICK:copy|COPY",
    "CLICK:delete|DELETE",
    "CLICK:insert|INSERT",
    "CLICK:paste|PASTE (over the selected range)",
    "CLICK:clear|CLEAR",
    "CLICK:undo|UNDO",
    "CLICK:undo|UNDO",
    "CLICK:redo|REDO",
    "KEY:Cmd+Z|undo by key",
    "KEY:Cmd+Y|redo by key",
    "KEY:Cmd+D|clone by key",
    "KEY:Cmd+C|copy by key",
    "KEY:Cmd+X|cut by key",
    "KEY:Cmd+V|insert by key",
    "KEY:Cmd+P|paste by key",
    "KEY:Backspace|delete by key",
    "KEY:Slash|half by key (needs a range: refused on a point)",
    "KEY:W|zoom in",
    "KEY:E|zoom out",
    "KEY:E|zoom out",
    "KEY:E|zoom out (AUTO quantize is now 4 beats)",
    "WAVE:9|click at beat 9 with a unit of 4 (lands on a bar line)",
    "CLICK:zoom_rst|RST (zoom back to step 5)",
    "CLICK:zoom_in|zoom in button",
    "CLICK:zoom_out|zoom out button",
    "CLICK:quantize|quantize list",
    "CLICK:quantize2|quantize 4 (the 1 and 2 beat pads go off)",
    "CLICK:beat1|BEAT SELECT 1 with a unit of 4 (refused)",
    "CLICK:quantize|quantize list",
    "CLICK:quantize1|quantize 1",
    "CLICK:loop|loop off",
    "CLICK:loop|loop on",
    "DRAG:4:8|select beats 4 to 8",
    "CLICK:play|play (loops beats 4 to 8)",
    "WAIT|still playing inside the loop",
    "WAIT|still playing inside the loop",
    "KEY:Space|pause by key",
    "KEY:C|back to the top of the timeline",
    "CLICK:palette_add|+ (the selected range goes to the palette)",
    "CLICK:pal_play0|palette play",
    "CLICK:pal_play0|palette stop",
    "PALDROP:12|palette entry dragged to beat 12 (inserted there)",
    "SELDRAG|selected range dragged to the palette (second entry)",
    "CLICK:trash|palette delete mode",
    "CLICK:pal_del1|delete the second entry",
    "CLICK:whole|click in the middle of the whole waveform",
    "CLICK:save|Save Project (first save: the render tip)",
    "CLICK:dlg_ok|OK on the tip (the project is saved)",
    "KEY:Cmd+D|an edit after saving (modified again)",
    "CLICK:new|Create new project (asks to save)",
    "CLICK:dlg_no|No (closed without saving)",
    "CALL:reopen|open the saved project from the recent list",
    "WAIT|loading",
    "WAIT|loaded: as it was saved",
    "CALL:shorten|keep 16 beats so that the render is short",
    "CLICK:render|Render Audio",
    "CLICK:dlg_render|Render",
    "WAIT|rendering",
    "WAIT|rendered and imported",
    "CALL:rendered|the new track in the collection",
    "CLICK:render|Render Audio (again)",
    "CALL:fmt0|format: the dialog's pull-down set to its first entry",
    "CLICK:dlg_render|Render",
    "WAIT|rendering",
    "WAIT|rendered and imported",
    "CALL:rendered|the new track in the collection",
    "CLICK:render|Render Audio (again)",
    "CALL:fmt2|format: the dialog's pull-down set to its third entry",
    "CLICK:dlg_render|Render",
    "WAIT|rendering",
    "WAIT|rendered and imported",
    "CALL:rendered|the new track in the collection",
    "CLICK:render|Render Audio (again)",
    "CALL:fmt3|format: the dialog's pull-down set to its fourth entry",
    "CLICK:dlg_render|Render",
    "WAIT|rendering",
    "WAIT|rendered and imported",
    "CALL:rendered|the new track in the collection",
    "CLICK:eject|artwork: unload (asks to save)",
    "CLICK:dlg_cancel|Cancel (still loaded)",
    "CLICK:browser|BROWSER tab (browser shown)",
    "CLICK:browser|BROWSER tab (browser hidden)",
    "CLICK:tool_strip|tool area collapsed",
];

impl App {
    fn edit_beat_x(&self, beat: f64) -> Pos2 {
        let w = self.edit.map.wave;
        pos2(w.min.x + ((beat - self.edit.view_start) as f32) * self.edit.pt_per_beat() + 0.5, w.center().y + 20.0)
    }

    pub fn shot_edit_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        if !std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "edittest")).unwrap_or(false) {
            return;
        }
        // The steps start once the track is in the editor.
        static BASE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        if BASE.get().is_none() {
            if self.edit.project.is_none() || frame < 130 {
                return;
            }
            let _ = BASE.set(frame + 5);
            eprintln!("edittest: loaded: {}", self.edit_state());
        }
        // The step clock stands still while a track is being decoded or a render runs.
        static HOLD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if self.edit.loading.is_some() || self.edit.job.is_some() {
            HOLD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        let Some(rel) = frame.checked_sub(*BASE.get().unwrap() + HOLD.load(std::sync::atomic::Ordering::Relaxed)) else { return };
        let (i, f) = ((rel / STEP) as usize, rel % STEP);
        let Some(step) = STEPS.get(i) else {
            if i == STEPS.len() && f == 0 {
                eprintln!("edittest: done");
            }
            return;
        };
        let (what, note) = step.split_once('|').unwrap_or((step, ""));
        let mut parts = what.split(':');
        let kind = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();
        let num = |k: usize| args.get(k).and_then(|a| a.parse::<f64>().ok()).unwrap_or(0.0);
        let at = |app: &App, name: &str| app.edit.map.at(name);
        // (start, end) of the pointer gesture of this step.
        let gesture: Option<(Pos2, Pos2)> = match kind {
            "CLICK" => match args[0] {
                "whole" => Some(self.edit.map.whole.center()).filter(|p| p.x.is_finite()).map(|p| (p, p)),
                name => at(self, name).map(|p| (p, p)),
            },
            "WAVE" | "SHIFT" => Some((self.edit_beat_x(num(0)), self.edit_beat_x(num(0)))),
            "DRAG" => Some((self.edit_beat_x(num(0)), self.edit_beat_x(num(1)))),
            "PALDROP" => at(self, "pal_wave0").map(|p| (p, self.edit_beat_x(num(0)))),
            "SELDRAG" => {
                let mid = self.edit.project.as_ref().and_then(|p| p.session.selection.range()).map(|(a, b)| (a + b) / 2.0).unwrap_or(0.0);
                Some((self.edit_beat_x(mid), self.edit.map.tool.center()))
            }
            _ => None,
        };
        let shift = kind == "SHIFT";
        if shift {
            raw.modifiers.shift = true;
        }
        let mods = if shift { egui::Modifiers::SHIFT } else { Default::default() };
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: mods };
        if let Some((a, b)) = gesture {
            match f {
                0 => raw.events.push(egui::Event::PointerMoved(a)),
                3 => raw.events.push(button(a, true)),
                4..=11 if a != b => raw.events.push(egui::Event::PointerMoved(a + (b - a) * ((f - 3) as f32 / 8.0))),
                12 => raw.events.push(button(b, false)),
                _ => {}
            }
        } else if kind == "CLICK" && f == 0 {
            eprintln!("edittest: ({} is not on screen)", args[0]);
        }
        if kind == "KEY" && (f == 3 || f == 5) {
            let name = args[0];
            let (cmd, key) = match name.strip_prefix("Cmd+") {
                Some(k) => (true, k),
                None => (false, name),
            };
            if let Some(key) = egui::Key::from_name(key) {
                let m = if cmd { egui::Modifiers { command: true, mac_cmd: true, ..Default::default() } } else { Default::default() };
                raw.modifiers = m;
                raw.events.push(egui::Event::Key { key, physical_key: None, pressed: f == 3, repeat: false, modifiers: m });
            }
        }
        if kind == "CALL" && f == 3 {
            match args[0] {
                "setup" => {
                    self.prefs.edit.show_tool = true;
                    self.prefs.edit.loop_mode = true;
                    self.prefs.edit.no_render_tip = false;
                    self.prefs.edit.render_format = 1;
                    self.prefs.edit.render_show = true;
                    // PREPDECK_EDIT_RENDER_DIR: renders go there instead of ~/Music/recordbox.
                    if let Ok(d) = std::env::var("PREPDECK_EDIT_RENDER_DIR") {
                        let _ = std::fs::create_dir_all(&d);
                        self.prefs.edit.render_dir = d;
                    }
                }
                f if f.starts_with("fmt") => {
                    // (What choosing the entry in the dialog's Format pull-down does.)
                    if let Ok(n) = f[3..].parse::<u8>() {
                        self.prefs.edit.render_format = n;
                    }
                    eprintln!("edittest: format now {:?}", crate::edit_render::FORMATS.get(self.prefs.edit.render_format as usize).map(|x| x.0));
                }
                "reopen" => {
                    if let Some(f) = self.prefs.edit.recent.first().cloned() {
                        self.edit.queue.push(Cmd::OpenRecent(f.into()));
                    }
                }
                "shorten" => {
                    if let Some(p) = self.edit.project.as_mut() {
                        let len = p.session.cur.len();
                        p.session.select_range(16.0, len, 1);
                    }
                    sync_engine(self);
                    self.edit.queue.push(Cmd::Edit(EditCmd::Delete));
                }
                "rendered" => {
                    let new = self.tracks.iter().filter(|t| self.db.setting(&edited_key(t.id)).is_some()).max_by_key(|t| t.id).cloned();
                    match new {
                        Some(t) => {
                            let cues = self.db.cues(t.id).unwrap_or_default();
                            let grid = self.db.grid(t.id).unwrap_or_default();
                            let size = std::fs::metadata(&t.path).map(|m| m.len()).unwrap_or(0);
                            eprintln!(
                                "edittest: rendered track: id {} \"{}\" by \"{}\" file {} ({} bytes) edited-from {:?} bpm {:.2} grid first beat {:.3} s at {:.2} BPM, cues {:?}, selected {} matching-with-source {}",
                                t.id,
                                t.title,
                                t.artist,
                                std::path::Path::new(&t.path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                                size,
                                self.db.setting(&edited_key(t.id)),
                                t.bpm,
                                grid.anchors.first().map(|a| a.time).unwrap_or(-1.0),
                                grid.first_bpm(),
                                cues.iter().map(|c| (c.slot, (c.start * 1000.0).round() / 1000.0)).collect::<Vec<_>>(),
                                self.selected.contains(&t.id),
                                self.edit.project.as_ref().map(|p| self.matching_of(p.track.id).contains(&t.id)).unwrap_or(false)
                            );
                        }
                        None => eprintln!("edittest: rendered track: none found"),
                    }
                }
                _ => {}
            }
        }
        if f == STEP - 4 {
            eprintln!("edittest: {note}: {}", self.edit_state());
        }
    }

    fn edit_state(&self) -> String {
        let modal = match &self.edit.modal {
            None => "-",
            Some(Modal::SavePrompt(_)) => "save prompt",
            Some(Modal::Alert { .. }) => "alert",
            Some(Modal::ConfirmNewer(..)) => "newer version",
            Some(Modal::RenderTip { .. }) => "render tip",
            Some(Modal::Render(_)) => "render dialog",
            Some(Modal::Rendering) => "rendering",
            Some(Modal::ConfirmStop) => "confirm stop",
        };
        let (pos, playing, loop_on, loop_range, pal_playing) = {
            let e = EDIT.lock();
            ((e.main.pos * 100.0).round() / 100.0, e.main.playing, e.main.loop_on, e.main.loop_range, e.pals.iter().filter(|v| v.playing).count())
        };
        let shell = format!("zoom {} unit {} browser {} tool {} pad tab {} modal {modal}", self.edit.zoom, self.edit.unit(), self.prefs.edit.show_browser, self.prefs.edit.show_tool, self.prefs.edit.pad_tab);
        let Some(p) = &self.edit.project else { return format!("not loaded | {shell}") };
        let s = &p.session;
        let secs: Vec<String> = s.cur.position.iter().take(6).map(|x| format!("{}-{}>{}", x.start, x.end, x.song_start)).collect();
        let muted: Vec<(f64, f64)> = s.cur.mute.iter().filter(|m| m.muted).map(|m| (m.start, m.end)).collect();
        format!(
            "sel {:?} len {} sections [{}]{} muted {:?} hot {} undo {} redo {} clip {:?} | pos {pos} playing {playing} loop {loop_on} {:?} | palette {:?} playing {pal_playing} delete-mode {} | {} status {} quantize {} | {shell}",
            s.selection,
            s.cur.len(),
            secs.join(" "),
            if s.cur.position.len() > 6 { " ..." } else { "" },
            muted,
            s.cur.prepared.hot.len(),
            s.undo.len(),
            s.redo.len(),
            s.clipboard.as_ref().map(|c| c.len()),
            loop_range,
            s.palettes.iter().map(|c| (c.id, c.clip.len())).collect::<Vec<_>>(),
            self.edit.palette_delete,
            p.name(),
            p.status(),
            s.quantize.label()
        )
    }
}
