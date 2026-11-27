//! PREPDECK_PREFS_SELFTEST=1 (with --screenshot and PREPDECK_BENCH=<frames> to keep the window
//! up, about 700): goes through the Preferences window with real pointer events. In
//! Performance and then Export mode it clicks every category and every tab, and on each page
//! one control (a switch or check box, else the second radio button of the page), and prints
//! what the click changed in the settings. A stand-in for a hand test where the screen cannot
//! be taken over. The places clicked are the ones the page itself laid out
//! (`ui_prefs_item::TRACE`), so the run fails loudly when a page has nothing to click.

use crate::app::*;
use crate::prefs_rb::{self as rb, Mode};
use eframe::egui;

#[derive(Clone, Debug)]
enum Step {
    Mode(u8),
    Category(u8),
    Tab(u8, usize),
    Control,
}

struct Holder {
    controls: Vec<(String, &'static str, String, egui::Rect)>,
}

#[derive(Default)]
struct Run {
    steps: Vec<Step>,
    at: usize,
    sub: u8,
    target: Option<(egui::Pos2, String)>,
    before: String,
    clicked: u32,
    changed: u32,
    done: bool,
}

static RUN: parking_lot::Mutex<Option<Run>> = parking_lot::Mutex::new(None);

fn plan() -> Vec<Step> {
    let mut v = vec![];
    for m in [1u8, 0] {
        v.push(Step::Mode(m));
        let mode = Mode::of(m);
        for c in rb::categories(mode).iter().filter(|c| **c < 100) {
            v.push(Step::Category(*c));
            for (i, t) in rb::tabs(*c, mode).iter().enumerate() {
                if rb::tab_out_of_scope(*c, t).is_some() {
                    continue;
                }
                v.push(Step::Tab(*c, i));
                v.push(Step::Control);
            }
        }
    }
    v
}

/// The first place where two debug dumps differ, a little of each.
fn diff(a: &str, b: &str) -> String {
    let i = a.bytes().zip(b.bytes()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    let start = a[..i].rfind(|c: char| c == ',' || c == '{').map(|j| j + 1).unwrap_or(i.saturating_sub(40));
    let cut = |s: &str| {
        let end = s[i..].find(|c: char| c == ',' || c == '}').map(|j| i + j).unwrap_or(s.len().min(i + 40));
        s.get(start..end).unwrap_or("").trim().to_string()
    };
    format!("{} -> {}", cut(a), cut(b))
}

impl App {
    pub fn shot_prefs_events(&mut self, frame: u32, raw: &mut egui::RawInput) {
        if std::env::var("PREPDECK_PREFS_SELFTEST").is_err() || frame < 20 {
            return;
        }
        let mut guard = RUN.lock();
        let run = guard.get_or_insert_with(|| {
            crate::ui_prefs_item::record(true);
            self.dialogs.prefs_open = true;
            // (The run is only ever started on a throwaway data folder: begin from the defaults
            // so that every click has the same effect each time.)
            self.prefs = Prefs::default();
            Run { steps: plan(), ..Default::default() }
        });
        if run.done {
            return;
        }
        let Some(step) = run.steps.get(run.at).cloned() else {
            eprintln!("prefs selftest: done, {} controls clicked, {} changed a setting", run.clicked, run.changed);
            run.done = true;
            return;
        };
        let mode = Mode::of(self.prefs.app_mode);
        let page = self.dialogs.prefs_tab;
        match run.sub {
            0 => {
                // Find where to click.
                if let Step::Mode(m) = &step {
                    self.prefs.app_mode = *m;
                    self.dialogs.prefs_tab = rb::VIEW;
                    eprintln!("prefs selftest: == {} mode: categories {:?}", Mode::of(*m).name(), rb::categories(Mode::of(*m)).iter().map(|c| rb::category_name(*c)).collect::<Vec<_>>());
                    run.at += 1;
                    return;
                }
                let trace = crate::ui_prefs_item::TRACE.lock().controls.clone();
                let trace = Holder { controls: trace };
                let shell = |name: &str| trace.controls.iter().find(|c| c.0 == "#shell" && c.2 == name).map(|c| (c.3.center(), name.to_string()));
                run.target = match &step {
                    Step::Mode(m) => {
                        self.prefs.app_mode = *m;
                        self.dialogs.prefs_tab = rb::VIEW;
                        eprintln!("prefs selftest: == {} mode: categories {:?}", Mode::of(*m).name(), rb::categories(Mode::of(*m)).iter().map(|c| rb::category_name(*c)).collect::<Vec<_>>());
                        run.at += 1;
                        return;
                    }
                    Step::Category(c) => shell(rb::category_name(*c)),
                    Step::Tab(c, i) => shell(rb::tabs(*c, mode)[*i]),
                    Step::Control => {
                        let visible = |r: &egui::Rect| r.min.y > 44.0 && r.max.y < 750.0 && r.min.x > 160.0;
                        let of = |kind: &str| trace.controls.iter().filter(|c| c.0 != "#shell" && c.1 == kind && visible(&c.3)).collect::<Vec<_>>();
                        let pick = of("switch").first().copied().or(of("check").first().copied()).or(of("radio").get(1).copied());
                        let pick = pick.or(trace.controls.iter().find(|c| c.2.starts_with("keycat:") && visible(&c.3)));
                        let pick = pick.or(of("combo").first().copied());
                        pick.map(|c| (egui::pos2(c.3.min.x + 8.0, c.3.center().y), format!("{} [{}] \"{}\"", c.0, c.1, c.2.replace('\n', " "))))
                    }
                };
                match &run.target {
                    Some((p, _)) => {
                        raw.events.push(egui::Event::PointerMoved(*p));
                        run.before = format!("{:?}", self.prefs);
                        run.sub = 1;
                        if matches!(step, Step::Control) {
                            let items = crate::ui_prefs_item::TRACE.lock().items.iter().map(|i| format!("{}={}", i.0, i.1)).collect::<Vec<_>>().join(" ");
                            if !items.is_empty() {
                                eprintln!("prefs selftest:     items: {items}");
                            }
                        }
                    }
                    None => {
                        let pending = crate::ui_prefs_item::TRACE.lock().items.iter().any(|i| i.0 == "Pending");
                        if pending {
                            eprintln!("prefs selftest:     (this tab's page comes from another track of the rebuild: nothing to click yet)");
                        } else {
                            eprintln!("prefs selftest:     (nothing to click in {}: {step:?} found no switch, check box, radio button or dropdown that is enabled in view)", rb::category_name(page));
                        }
                        run.at += 1;
                    }
                }
            }
            1 | 2 => {
                if let Some((p, _)) = &run.target {
                    raw.events.push(egui::Event::PointerButton { pos: *p, button: egui::PointerButton::Primary, pressed: run.sub == 1, modifiers: Default::default() });
                }
                run.sub += 1;
            }
            3 => {
                // (A dropdown that the click opened is closed again.)
                raw.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
                run.sub = 4;
            }
            _ => {
                let after = format!("{:?}", self.prefs);
                let name = run.target.as_ref().map(|t| t.1.clone()).unwrap_or_default();
                match &step {
                    Step::Category(c) => eprintln!("prefs selftest: category {} -> open: {}", name, if self.dialogs.prefs_tab == *c { "yes" } else { "NO" }),
                    Step::Tab(c, i) => {
                        let key = egui::Id::new(format!("prefs_subtab_{c}"));
                        let cur = raw_tab(self, key);
                        eprintln!("prefs selftest:   tab {} -> open: {}", name, if cur == Some(*i) { "yes" } else { "NO" });
                    }
                    Step::Control => {
                        run.clicked += 1;
                        if after != run.before {
                            run.changed += 1;
                            eprintln!("prefs selftest:     {name}: {}", diff(&run.before, &after));
                        } else {
                            eprintln!("prefs selftest:     {name}: no stored setting changed (a list was opened, or the control belongs to something recordbox does not have)");
                        }
                    }
                    Step::Mode(_) => {}
                }
                run.sub = 0;
                run.at += 1;
            }
        }
    }
}

/// The tab the window has open for a category (kept in the UI's memory).
fn raw_tab(_app: &App, key: egui::Id) -> Option<usize> {
    crate::ui_prefs::last_ctx().and_then(|c| c.data(|d| d.get_temp::<usize>(key)))
}
