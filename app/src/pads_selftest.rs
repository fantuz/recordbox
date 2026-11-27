//! Scripted states and a scripted pointer run for the pad modes (with `--screenshot` and
//! `PREPDECK_SHOT_VIEW=performance` or `perf4`), a stand-in for a hand test where the screen
//! cannot be taken over.
//!
//! - `PREPDECK_SHOT_PADS=<mode>,<mode>,...[;pad][;cues][;scroll=<0..1>][;editor]`: the pad mode
//!   of decks 1 to 4 (hotcue, padfx, slicer, beatjump, beatloop, keyboard, keyshift, seqcall,
//!   censor, stems, memory, sampler, user1..user8) for the picture; `pad` = PAD Display,
//!   `cues` = a few hot and memory cues and a censored region on every deck, `scroll` = where
//!   the pad lists stand, `editor` = the Pad Editor open.
//! - `PREPDECK_SHOT_PLAYER=padtest` (with `PREPDECK_BENCH=<frames>` to keep the window up and
//!   deck 1 playing): presses every pad mode's pads on deck `PREPDECK_PADTEST_DECK` (1..4,
//!   default 1) with real pointer events and prints the deck's pad state after each step.

use crate::app::*;
use crate::model::*;
use crate::pads::*;
use eframe::egui;
use egui::{pos2, vec2, Pos2, Rect};

fn mode_named(s: &str) -> Option<PadMode> {
    Some(match s.trim() {
        "hotcue" => PadMode::HotCue,
        "padfx" => PadMode::PadFx,
        "slicer" => PadMode::Slicer,
        "beatjump" => PadMode::BeatJump,
        "beatloop" => PadMode::BeatLoop,
        "keyboard" => PadMode::Keyboard,
        "keyshift" => PadMode::KeyShift,
        "seqcall" => PadMode::SeqCall,
        "censor" => PadMode::ActiveCensor,
        "stems" => PadMode::Stems,
        "memory" => PadMode::MemoryCue,
        "sampler" => PadMode::Sampler,
        u if u.starts_with("user") => PadMode::User(u[4..].parse::<u8>().ok().filter(|n| (1..=8).contains(n))? - 1),
        _ => return None,
    })
}

enum Kind {
    Call(Box<dyn Fn(&mut App)>),
    Click(Pos2),
    Hold(Pos2),
    Drag(Pos2, Pos2),
    Wait,
}

struct Step {
    name: String,
    kind: Kind,
}

fn call(name: &str, f: impl Fn(&mut App) + 'static) -> Step {
    Step { name: name.into(), kind: Kind::Call(Box::new(f)) }
}
fn click(name: &str, p: Pos2) -> Step {
    Step { name: name.into(), kind: Kind::Click(p) }
}
fn hold(name: &str, p: Pos2) -> Step {
    Step { name: name.into(), kind: Kind::Hold(p) }
}
fn wait(name: &str) -> Step {
    Step { name: name.into(), kind: Kind::Wait }
}

impl App {
    /// Called for every frame of a `--screenshot` run, before the frame is built.
    pub fn shot_pads_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        let spec = std::env::var("PREPDECK_SHOT_PADS").unwrap_or_default();
        let test = std::env::var("PREPDECK_SHOT_PLAYER").map(|v| v.split(',').any(|x| x.trim() == "padtest")).unwrap_or(false);
        if spec.is_empty() && !test {
            return;
        }
        let count = self.deck_count();
        // Every deck gets a track (deck 1 has the screenshot's own).
        if frame == 3 {
            let ids: Vec<i64> = self.tracks.iter().filter(|t| t.analyzed == 1 && !t.missing).map(|t| t.id).take(8).collect();
            for n in 1..count {
                if self.deck_of(n).track.is_none() {
                    if let Some(id) = ids.get(n as usize).or(ids.first()).copied() {
                        self.with_deck(n, |a| a.load_track_here(id));
                    }
                }
            }
        }
        if frame == 50 && !spec.is_empty() {
            let mut parts = spec.split(';');
            let modes: Vec<Option<PadMode>> = parts.next().unwrap_or("").split(',').map(mode_named).collect();
            let opts: Vec<&str> = parts.collect();
            if opts.iter().any(|o| o.trim() == "pad") {
                self.prefs.pads.layout = 3;
            }
            if opts.iter().any(|o| o.trim() == "editor") {
                self.pads.editor_open = true;
            }
            let cues = opts.iter().any(|o| o.trim() == "cues");
            let scroll = opts.iter().find_map(|o| o.trim().strip_prefix("scroll=").and_then(|v| v.parse::<f32>().ok()));
            for n in 0..count {
                self.with_deck(n, |a| {
                    if cues && a.deck.track.is_some() {
                        let bl = a.beat_len();
                        let t0 = a.deck.beats.first().map(|b| b.time).unwrap_or(0.0);
                        for (slot, beats) in [(1, 16.0), (2, 32.0), (5, 64.0)] {
                            if !a.deck.cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == slot) {
                                a.set_hot_cue_at(slot, t0 + beats * bl);
                            }
                        }
                        a.seek(t0 + 8.0 * bl);
                        a.censor_load();
                        if a.pads.deck[n as usize].censor.regions.is_empty() {
                            a.pads.deck[n as usize].censor.pending_in = Some(t0 + 9.0 * bl);
                            a.seek(t0 + 10.0 * bl);
                            a.censor_out();
                            a.censor_set_kind(1);
                            a.censor_onoff();
                            a.seek(t0 + 8.0 * bl);
                        }
                    }
                    if let Some(Some(m)) = modes.get(n as usize) {
                        if let PadMode::User(u) = m {
                            a.prefs.pads.user_shown[*u as usize] = true;
                        }
                        if !a.set_pad_mode(*m) {
                            eprintln!("padshot: deck {} refused {}", n + 1, m.label());
                        }
                    }
                    if let Some(s) = scroll {
                        a.pads.deck[n as usize].scroll = [Some(s); 21];
                    }
                });
            }
        }
        if test {
            self.padtest(frame, raw);
        }
    }

    fn padtest(&mut self, frame: u32, raw: &mut egui::RawInput) {
        const START: u32 = 150;
        const SPAN: u32 = 30;
        if frame < START {
            return;
        }
        let d = std::env::var("PREPDECK_PADTEST_DECK").ok().and_then(|v| v.parse::<u8>().ok()).unwrap_or(1).clamp(1, self.deck_count()) - 1;
        if frame == START {
            // The deck under test plays (deck 1 does already, muted, by PREPDECK_BENCH).
            if !self.px.muted {
                self.toggle_mute();
            }
            self.with_deck(d, |a| {
                if !a.playing() {
                    a.play_pause();
                }
                a.prefs.quantize = true;
            });
            if let Some(e) = crate::sampler::SAMPLER.lock().as_mut() {
                e.silent = true;
            }
        }
        let steps = self.padtest_steps(d);
        let i = ((frame - START) / SPAN) as usize;
        let f = (frame - START) % SPAN;
        let Some(step) = steps.get(i) else {
            if i == steps.len() && f == 0 {
                eprintln!("padtest: done ({} steps on deck {})", steps.len(), d + 1);
            }
            return;
        };
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let state = |a: &mut App| a.with_deck(d, |a| a.pads_state_text());
        match &step.kind {
            Kind::Call(func) => {
                if f == 3 {
                    self.with_deck(d, |a| func(a));
                }
            }
            Kind::Click(p) => match f {
                0 => raw.events.push(egui::Event::PointerMoved(*p)),
                3 => raw.events.push(button(*p, true)),
                // (Let go two frames later: a window that loses the focus in between, as happens
                // when other instances start, drops the press.)
                5 => raw.events.push(button(*p, false)),
                _ => {}
            },
            Kind::Hold(p) => match f {
                0 => raw.events.push(egui::Event::PointerMoved(*p)),
                3 => raw.events.push(button(*p, true)),
                3..=19 => std::thread::sleep(std::time::Duration::from_millis(16)),
                20 => raw.events.push(button(*p, false)),
                _ => {}
            },
            Kind::Drag(a, b) => match f {
                0 => raw.events.push(egui::Event::PointerMoved(*a)),
                3 => raw.events.push(button(*a, true)),
                4..=11 => raw.events.push(egui::Event::PointerMoved(*a + (*b - *a) * ((f - 3) as f32 / 8.0))),
                12 => raw.events.push(button(*b, false)),
                _ => {}
            },
            Kind::Wait => {
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }
        if f == 16 && matches!(step.kind, Kind::Hold(_)) {
            eprintln!("padtest: {} (while held): {}", step.name, state(self));
        }
        if f == 26 {
            let toast = self.toasts.last().filter(|t| t.until > std::time::Instant::now()).map(|t| t.text.clone()).unwrap_or_default();
            eprintln!("padtest: {}: {}{}", step.name, state(self), if toast.is_empty() { String::new() } else { format!(" | message: {toast}") });
        }
    }

    /// The run's steps for deck `d`, at the places the pad panel was last drawn.
    fn padtest_steps(&self, d: u8) -> Vec<Step> {
        let r = self.pads.rects[(d as usize).min(3)];
        let (area, pull) = crate::ui_pads::layout_of(r, d, self.pads.c4);
        let left = d % 2 == 0;
        let (w, h) = (area.width(), area.height());
        let cw = ((w - 2.0) / 2.0).floor();
        let ch = ((h - 5.0 - 6.0) / 4.0).floor();
        // A pad of a list (two columns showing), and of the slicer's / censor's eight.
        let cell = |col: usize, row: usize| Rect::from_min_size(area.min + vec2(col as f32 * (cw + 2.0), row as f32 * (ch + 2.0)), vec2(cw, ch));
        let ch8 = ((h + 23.0 - 29.0) / 4.0).floor();
        let cell8 = |col: usize, row: usize| Rect::from_min_size(area.min + vec2(col as f32 * (cw + 2.0), row as f32 * (ch8 + 2.0)), vec2(cw, ch8)).center();
        let hcw = ((2.0 * w + 1.0 - 6.0) / 4.0).floor();
        let hot = |col: usize, row: usize| Rect::from_min_size(area.min + vec2(col as f32 * (hcw + 2.0), row as f32 * (ch + 2.0)), vec2(hcw, ch));
        let ctrl = |cw: f32| if left { pos2(area.max.x - cw, area.max.y + 5.0) } else { pos2(area.min.x, area.max.y + 5.0) };
        let beside = |off: f32, bw: f32| if left { pos2(area.min.x + off + bw / 2.0, area.max.y + 14.0) } else { pos2(area.max.x - off - bw / 2.0, area.max.y + 14.0) };
        let bar = (pos2(area.min.x + 30.0, area.max.y - 2.0), pos2(area.max.x - 4.0, area.max.y - 2.0));
        let fwd = |c: Rect| pos2(c.min.x + c.width() * 0.75, c.center().y);
        let back = |c: Rect| pos2(c.min.x + c.width() * 0.25, c.center().y);
        let set = |m: PadMode| {
            call(&format!("pad mode {}", m.label()), move |a| {
                a.set_pad_mode(m);
            })
        };
        let s194 = ctrl(194.0);
        let s120 = ctrl(120.0);
        let s50 = ctrl(50.0);
        let pcw = ((w - 2.0) / 2.0).floor();
        let pch = ((h - 2.0) / 2.0).floor();
        vec![
            // HOT CUE
            set(PadMode::HotCue),
            click("HOT CUE pad C (empty: sets the cue)", hot(0, 2).center()),
            wait("a moment later"),
            hold("HOT CUE pad C held (jumps to the cue)", hot(0, 2).center()),
            click("HOT CUE pad C's cross (clears it)", pos2(hot(0, 2).max.x - 17.0, hot(0, 2).center().y)),
            // BEAT JUMP
            set(PadMode::BeatJump),
            click("BEAT JUMP 1 forward", fwd(cell(1, 0))),
            click("BEAT JUMP 4 back", back(cell(1, 2))),
            click("BEAT JUMP FINE forward", fwd(cell(0, 0))),
            Step { name: "BEAT JUMP list dragged to its right end".into(), kind: Kind::Drag(bar.0, bar.1) },
            click("BEAT JUMP 16 forward (third column)", fwd(Rect::from_min_size(pos2(area.max.x - cw, area.min.y), vec2(cw, ch)))),
            call("a 4-beat loop, for the next step", |a| a.auto_loop(4.0)),
            click("BEAT JUMP 32 forward with the loop running (the loop moves)", fwd(Rect::from_min_size(pos2(area.max.x - cw, area.min.y + ch + 2.0), vec2(cw, ch)))),
            call("loop off", |a| a.loop_exit()),
            // BEAT LOOP
            set(PadMode::BeatLoop),
            click("BEAT LOOP pad 1", cell(1, 2).center()),
            click("BEAT LOOP pad 1 again (ends the loop)", cell(1, 2).center()),
            Step { name: "BEAT LOOP list dragged to its right end".into(), kind: Kind::Drag(bar.0, bar.1) },
            click("BEAT LOOP pad 8 (third column)", pos2(area.max.x - cw - 2.0 - cw / 2.0, area.min.y + ch + 2.0 + ch / 2.0)),
            call("loop off", |a| a.loop_exit()),
            // KEY SHIFT
            set(PadMode::KeyShift),
            click("KEY SHIFT pad +11", cell(0, 1).center()),
            click("KEY SHIFT pad +12", cell(0, 0).center()),
            call("SEMITONE UP at +12 (refused: the limit)", |a| a.pad_key(33)),
            call("KEY RESET without Master Tempo (refused, with rekordbox's message)", |a| {
                if a.deck.x.master_tempo {
                    a.toggle_master_tempo();
                }
                a.pad_key(36)
            }),
            call("Master Tempo on, KEY RESET", |a| {
                a.toggle_master_tempo();
                a.pad_key(36)
            }),
            call("SEMITONE DOWN", |a| a.pad_key(37)),
            // KEYBOARD
            set(PadMode::Keyboard),
            call("a hot cue A for the key cue, if there is none", |a| {
                if !a.deck.cues.iter().any(|c| c.kind == CueKind::Hot && c.slot == 0) {
                    let t = a.deck.beats.first().map(|b| b.time).unwrap_or(0.5);
                    a.set_hot_cue_at(0, t);
                }
            }),
            click("KEYBOARD edit button (the hot cue pads show)", beside(84.0, 20.0)),
            click("KEYBOARD edit: pad A (becomes the key cue)", hot(0, 0).center()),
            click("KEYBOARD edit button (back to the keys)", beside(84.0, 20.0)),
            hold("KEYBOARD pad +10 held (the key cue plays 10 semitones up)", cell(0, 2).center()),
            call("key back to 0", |a| a.pad_key(36)),
            // SLICER
            set(PadMode::Slicer),
            hold("SLICER pad 6 held", cell8(1, 1)),
            wait("after letting go"),
            wait("a moment later (back at the background position)"),
            click("SLICER shift forward", s194 + vec2(58.0, 9.0)),
            click("SLICER LENGTH down (8 -> 4, ROLL 1/4)", s194 + vec2(71.0 + 10.0, 9.0)),
            click("SLICER ROLL up (1/4 -> 1/2)", s194 + vec2(133.0 + 61.0 - 10.0, 9.0)),
            click("SLICER LOOP on", s194 + vec2(12.0, 9.0)),
            wait("SLICER LOOP: playing round the domain"),
            wait("SLICER LOOP: still inside"),
            wait("SLICER LOOP: still inside"),
            call("Slicer Capture (eight slices into the sampler bank of the preferences)", |a| {
                let ok = a.slicer_capture();
                let bank = (a.prefs.sampler.slicer_bank.clamp(1, 4) - 1) as usize;
                let names: Vec<String> = (0..8).filter_map(|i| a.prefs.sampler.slots.get(bank * 16 + i).and_then(|s| s.as_ref()).map(|s| s.name.clone())).collect();
                eprintln!("padtest: slicer capture ok {ok}, bank {} slots 1-8: {names:?}", bank + 1);
            }),
            click("SLICER LOOP off", s194 + vec2(12.0, 9.0)),
            // ACT. CENSR
            set(PadMode::ActiveCensor),
            click("ACT. CENSR IN", cell8(1, 0)),
            wait("playing on"),
            wait("playing on"),
            wait("playing on"),
            click("ACT. CENSR OUT (the region is saved)", cell8(1, 1)),
            click("ACT. CENSR TRANS", cell8(0, 1)),
            click("ACT. CENSR beat down (TRANS 1/4 -> 1/8)", s120 + vec2(82.0, 9.0)),
            click("ACT. CENSR ON/OFF (on)", cell8(1, 3)),
            click("ACT. CENSR previous in point (playback is inside the region: the effect runs)", s120 + vec2(15.0, 9.0)),
            wait("still inside"),
            wait("a moment later"),
            wait("past the region (the effect is off again)"),
            wait("past the region"),
            click("ACT. CENSR previous in point", s120 + vec2(15.0, 9.0)),
            // (perfrest track) The adjust mode: inside a region IN / OUT read IN ADJ / OUT ADJ;
            // a press starts the adjust (the playhead goes to that edge, which then follows
            // it), the next press ends it and saves the region.
            click("ACT. CENSR IN ADJ (adjust starts at the in point)", cell8(1, 0)),
            wait("adjusting: the in point follows the playhead"),
            click("ACT. CENSR IN ADJ (adjust ends, saved)", cell8(1, 0)),
            click("ACT. CENSR OUT ADJ (adjust starts at the out point)", cell8(1, 1)),
            wait("adjusting: the out point follows the playhead"),
            click("ACT. CENSR OUT ADJ (adjust ends, saved)", cell8(1, 1)),
            click("ACT. CENSR previous in point", s120 + vec2(15.0, 9.0)),
            click("ACT. CENSR CLEAR inside the region (it is removed)", cell8(1, 2)),
            click("ACT. CENSR ON/OFF (off)", cell8(1, 3)),
            // USER 1
            call("USER 1 listed and chosen", |a| {
                a.prefs.pads.user_shown[0] = true;
                a.set_pad_mode(PadMode::User(0));
            }),
            hold("USER 1 pad 1 (HOT CUE A) held", cell(0, 0).center()),
            click("USER 1 pad 6 (SAMPLER 2)", cell(1, 1).center()),
            // (perfrest track) Transport and PAD FX items on a USER pad, set through the Pad
            // Editor's call, then pressed on screen.
            call("USER 1 pad 1 := TRANSPORT BPM + MEDIUM", |a| {
                a.user_set_item(0, 0, Some(("Transport", "BPM + MEDIUM")));
            }),
            click("USER 1 pad 1 (BPM + MEDIUM: +0.50 %)", cell(0, 0).center()),
            call("USER 1 pad 1 := PAD FX1 1", |a| {
                a.user_set_item(0, 0, Some(("PadFx1", "1")));
            }),
            hold("USER 1 pad 1 (PAD FX1 pad 1) held", cell(0, 0).center()),
            call("USER 1 pad 1 back to its template; the Pad Editor opened", |a| {
                a.user_set_item(0, 0, None);
                a.pads.editor_open = true;
            }),
            call("Pad Editor closed; SP.SCRATCH slot 1 (an empty slot: nothing happens)", |a| {
                a.pads.editor_open = false;
                a.sample_scratch_pad(0);
            }),
            // PAD Display
            call("PAD Display, BEAT JUMP", |a| {
                a.prefs.pads.layout = 3;
                a.set_pad_mode(PadMode::BeatJump);
            }),
            click("BEAT JUMP page up (5 -> 6)", s50 + vec2(38.0, 9.0)),
            click("BEAT JUMP first cell of page 6 forward (2 beats)", fwd(Rect::from_min_size(area.min, vec2(pcw, pch)))),
            call("PAD Display, KEY SHIFT", |a| {
                a.set_pad_mode(PadMode::KeyShift);
            }),
            click("KEY SHIFT page 2, first pad (+4)", Rect::from_min_size(area.min, vec2(((w - 6.0) / 4.0).floor(), pch)).center()),
            click("KEY SHIFT page up (2 -> 3)", s50 + vec2(38.0, 9.0)),
            call("LIST Display again, key 0, HOT CUE", |a| {
                a.prefs.pads.layout = 1;
                a.pad_key(12);
                a.set_pad_mode(PadMode::HotCue);
            }),
            // The pull-down itself.
            click("the pad mode pull-down (opens)", pull.center()),
            click("its fifth entry (BEAT LOOP)", pos2(pull.min.x + 40.0, pull.max.y + 18.5 + 4.0 * 24.5)),
            wait("end"),
            click("the pad mode pull-down again (left open for the picture)", pull.center()),
        ]
    }
}
