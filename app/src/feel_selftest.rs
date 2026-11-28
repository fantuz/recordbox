//! Tools for seeing recordbox's interface motion without taking over the screen (all with
//! `--screenshot` on a throwaway library copy):
//!
//! - `PREPDECK_SHOT_PLAYER=feeltest PREPDECK_FEEL=<scenario>`: a scripted interaction with
//!   real pointer, wheel and key events (`hover`, `press`, `fader`, `knob`, `xfader`, `scroll`,
//!   `wheel`, `keys`, `drag`, `refuse`, `panel`, `mode`, `play`), printing "feeltest:" lines.
//! - Every `PREPDECK_BENCH` run also prints the intervals between frames (a histogram, and how
//!   many were longer than 1.5 frame periods) and, while a deck plays, how evenly the drawn
//!   position advanced.
//! - `PREPDECK_IDLE_PROBE=<seconds>`: from frame 150 the run stops asking for frames and counts
//!   the frames drawn anyway for that long ("idle: ..."), then quits.

use crate::app::App;
use crate::motion;
use eframe::egui;
use egui::{pos2, Pos2};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;

#[derive(Default)]
struct Stats {
    last: Option<std::time::Instant>,
    intervals: Vec<f32>,
    /// Per frame while a deck plays: (drawn position step, real position step) in seconds.
    steps: Vec<(f64, f64)>,
    last_pos: Option<(f64, f64)>,
    window: Option<(u32, u32)>,
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);
static IDLE_COUNT: AtomicU64 = AtomicU64::new(0);
static IDLE_ON: AtomicBool = AtomicBool::new(false);

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn scenario() -> Option<&'static str> {
    static S: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let on = env("PREPDECK_SHOT_PLAYER").is_some_and(|v| v.split(',').any(|x| x.trim() == "feeltest"));
        if on { env("PREPDECK_FEEL") } else { None }
    }).as_deref()
}

/// True while the idle probe runs: main.rs then does not ask for the next frame.
pub fn idle_probe() -> bool {
    IDLE_ON.load(Relaxed)
}

/// Collect at most 36,000 frame samples per diagnostic run (ten minutes at 60 Hz).
pub fn bench_frames() -> u32 {
    static COUNT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *COUNT.get_or_init(|| env("PREPDECK_BENCH").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0).min(36_000))
}

fn at_env(name: &str) -> Option<Pos2> {
    let v = env(name)?;
    let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
    Some(pos2(it.next()?, it.next()?))
}

pub fn frame_start(app: &mut App, ctx: &egui::Context) {
    let Some((_, frame)) = app.screenshot.clone() else { return };
    IDLE_COUNT.fetch_add(1, Relaxed);
    if frame == 150 {
        if let Some(secs) = env("PREPDECK_IDLE_PROBE").and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && (0.0..=60.0).contains(v)) {
            IDLE_ON.store(true, Relaxed);
            IDLE_COUNT.store(0, Relaxed);
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs_f64(secs));
                let n = IDLE_COUNT.load(Relaxed);
                let causes = ctx2.repaint_causes();
                eprintln!("idle: {n} frames drawn in {secs:.1} s with nothing asking (last causes: {:?})", causes.iter().take(4).map(|c| c.to_string()).collect::<Vec<_>>());
                std::process::exit(0);
            });
        }
    }
    if frame == 2 {
        if bench_frames() > 0 {
            *STATS.lock().unwrap() = Some(Stats::default());
        }
    }
    // Frame intervals while the bench runs.
    if let Some(st) = STATS.lock().unwrap().as_mut() {
        let now = std::time::Instant::now();
        let inside = st.window.map_or(frame > 90, |(a, b)| frame >= a && frame <= b);
        if let Some(l) = st.last {
            if inside {
                st.intervals.push(now.duration_since(l).as_secs_f32() * 1000.0);
            }
        }
        st.last = Some(now);
    }
    if let Some(s) = scenario() {
        scenario_state(app, s, frame);
    }
}

pub fn frame_end(app: &mut App) {
    let Some((_, frame)) = app.screenshot.as_ref() else { return };
    let frame = *frame;
    // Play steadiness: the drawn and the real position of the active deck.
    if bench_frames() > 0 && app.playing() {
        let shown = crate::ui_feel::shown_pos(app);
        let real = app.pos();
        if let Some(st) = STATS.lock().unwrap().as_mut() {
            if let Some((ps, pr)) = st.last_pos {
                let inside = st.window.map_or(frame > 90, |(a, b)| frame >= a && frame <= b);
                if inside {
                    st.steps.push((shown - ps, real - pr));
                }
            }
            st.last_pos = Some((shown, real));
        }
    }
    let b = bench_frames();
    if b > 0 && frame == 119 + b {
        report_stats(app);
    }
}

fn report_stats(app: &App) {
    let Some(st) = STATS.lock().unwrap().take() else { return };
    let mut v = st.intervals.clone();
    if v.is_empty() {
        return;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = v[v.len() / 2];
    let long = v.iter().filter(|x| **x > 1.5 * med).count();
    let edges = [4.0, 6.0, 7.5, 8.5, 9.5, 12.0, 15.0, 17.5, 20.0, 25.0, 34.0, 50.0, f32::INFINITY];
    let mut counts = vec![0usize; edges.len()];
    for x in &v {
        let k = edges.iter().position(|e| x < e).unwrap_or(edges.len() - 1);
        counts[k] += 1;
    }
    let mut hist = String::new();
    let mut lo = 0.0;
    for (e, c) in edges.iter().zip(&counts) {
        if *c > 0 {
            hist.push_str(&format!(" [{lo:.1}-{}: {c}]", if e.is_finite() { format!("{e:.1}") } else { "inf".into() }));
        }
        lo = *e;
    }
    eprintln!("bench: frame intervals {} frames, median {med:.2} ms, p95 {:.2}, max {:.2}, longer than 1.5x median {long}, motion {} (still moving last frame {});{hist}", v.len(), v[v.len() * 95 / 100], v[v.len() - 1], motion::level().name(), motion::last_active());
    if st.steps.len() > 10 {
        let stat = |f: &dyn Fn(&(f64, f64)) -> f64| {
            let xs: Vec<f64> = st.steps.iter().map(f).collect();
            let m = xs.iter().sum::<f64>() / xs.len() as f64;
            let sd = (xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / xs.len() as f64).sqrt();
            let still = xs.iter().filter(|x| x.abs() < 1e-9).count();
            (m * 1000.0, sd * 1000.0, still)
        };
        let (ms, ss, zs) = stat(&|p| p.0);
        let (mr, sr, zr) = stat(&|p| p.1);
        eprintln!("bench: play steps per frame (ms of track): drawn mean {ms:.2} sd {ss:.2} frames without a move {zs}; audio position mean {mr:.2} sd {sr:.2} frames without a move {zr} ({} frames, zoom {})", st.steps.len(), app.prefs.zoom);
    }
}

pub fn raw_input(app: &mut App, raw: &mut egui::RawInput) {
    let Some((_, frame)) = app.screenshot.clone() else { return };
    if let Some(sc) = scenario() {
        scenario_events(app, sc, frame, raw);
    }
}

// ------------------------------------------------------------------ scenarios

/// Frame the scripted interaction starts at.
const T0: u32 = 160;

fn click(raw: &mut egui::RawInput, p: Pos2, down: bool) {
    raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: down, modifiers: Default::default() });
}

fn mv(raw: &mut egui::RawInput, p: Pos2) {
    raw.events.push(egui::Event::PointerMoved(p));
}

fn key(raw: &mut egui::RawInput, k: egui::Key, down: bool) {
    raw.events.push(egui::Event::Key { key: k, physical_key: None, pressed: down, repeat: false, modifiers: Default::default() });
}

fn wheel(raw: &mut egui::RawInput, unit: egui::MouseWheelUnit, dy: f32) {
    raw.events.push(egui::Event::MouseWheel { unit, delta: egui::vec2(0.0, dy), modifiers: Default::default() });
}

fn list_offset(app: &App) -> f32 {
    // The track list's scroll state (ui_browser.rs `vscroll`).
    let _ = app;
    LIST_OFF.with(|c| c.get())
}

thread_local! {
    static LIST_OFF: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// The track list reports its scroll offset (ui_browser.rs) for the scroll scenarios.
pub fn list_offset_seen(y: f32) {
    LIST_OFF.with(|c| c.set(y));
}

fn scenario_state(app: &mut App, s: &str, frame: u32) {
    if frame == T0 - 10 {
        match s {
            "play" => {
                app.prefs.zoom = 0;
            }
            _ => {}
        }
    }
    if frame == T0 && bench_frames() > 0 {
        if let Some(st) = STATS.lock().unwrap().as_mut() {
            // The histogram covers the interaction only.
            st.window = Some((T0, T0 + span(s)));
            st.intervals.clear();
            st.steps.clear();
        }
    }
    if (T0..T0 + span(s)).contains(&frame) && matches!(s, "scroll" | "wheel" | "keys") {
        eprintln!("feeltest: {s} frame {} list offset {:.2} selected {:?}", frame - T0, list_offset(app), app.anchor);
    }
}

fn span(s: &str) -> u32 {
    match s {
        "scroll" | "wheel" => 120,
        "keys" => 140,
        "play" => 600,
        "drag" | "refuse" => 80,
        _ => 60,
    }
}

fn scenario_events(app: &mut App, s: &str, frame: u32, raw: &mut egui::RawInput) {
    if frame < T0 {
        return;
    }
    let f = frame - T0;
    let at = at_env("PREPDECK_FEEL_AT");
    match s {
        "hover" => {
            let p = at.unwrap_or(pos2(700.0, 14.0));
            match f {
                0 => mv(raw, p + egui::vec2(0.0, 60.0)),
                4 => mv(raw, p),
                30 => mv(raw, p + egui::vec2(0.0, 60.0)),
                _ => {}
            }
        }
        "press" => {
            let p = at.unwrap_or(pos2(700.0, 14.0));
            match f {
                0 => mv(raw, p),
                10 => click(raw, p, true),
                24 => click(raw, p, false),
                40 => mv(raw, p + egui::vec2(0.0, 60.0)),
                _ => {}
            }
        }
        // Performance: channel fader 1 dragged down, then a double-click puts it back to the top.
        "fader" | "knob" | "xfader" => {
            let (a, b) = match s {
                "fader" => (at.unwrap_or(pos2(729.5, 28.0 + 143.0 + 169.5)), egui::vec2(0.0, 36.0)),
                "knob" => (at.unwrap_or(pos2(729.5, 28.0 + 143.0 + 51.0)), egui::vec2(0.0, 60.0)),
                _ => (at.unwrap_or(pos2(756.0, 28.0 + 368.0 + 1.5 + 21.5)), egui::vec2(40.0, 0.0)),
            };
            match f {
                0 => mv(raw, a),
                4 => click(raw, a, true),
                5..=16 => mv(raw, a + b * ((f - 4) as f32 / 12.0)),
                17 => click(raw, a + b, false),
                30 => {
                    eprintln!("feeltest: {s} after the drag: {}", mixer_line(app));
                }
                34 | 36 => click(raw, a + b, true),
                35 | 37 => click(raw, a + b, false),
                50 => eprintln!("feeltest: {s} after the double-click: {}", mixer_line(app)),
                56 => mv(raw, a + b + egui::vec2(120.0, 0.0)),
                _ => {}
            }
        }
        // A trackpad swipe: steps of 40 pt falling off like the system's momentum.
        "scroll" => {
            let p = at.unwrap_or(pos2(800.0, 650.0));
            if f == 0 {
                mv(raw, p);
            }
            if (2..62).contains(&f) {
                let k = (f - 2) as f32;
                let d = -40.0 * (-k / 14.0).exp();
                if d.abs() > 0.3 {
                    wheel(raw, egui::MouseWheelUnit::Point, d);
                }
            }
            if (70..100).contains(&f) {
                let k = (f - 70) as f32;
                wheel(raw, egui::MouseWheelUnit::Point, 30.0 * (-k / 10.0).exp());
            }
        }
        // A mouse wheel: three notches down, a pause, two up.
        "wheel" => {
            let p = at.unwrap_or(pos2(800.0, 650.0));
            match f {
                0 => mv(raw, p),
                4 | 12 | 20 => wheel(raw, egui::MouseWheelUnit::Line, -1.0),
                60 | 64 => wheel(raw, egui::MouseWheelUnit::Line, 1.0),
                _ => {}
            }
        }
        // Click a row, then the down arrow 30 times (selection moves, the list follows).
        "keys" => {
            let p = at.unwrap_or(pos2(800.0, 500.0));
            match f {
                0 => mv(raw, p),
                2 => click(raw, p, true),
                3 => click(raw, p, false),
                10..=130 if (f - 10) % 4 == 0 => key(raw, egui::Key::ArrowDown, true),
                11..=131 if (f - 11) % 4 == 0 => key(raw, egui::Key::ArrowDown, false),
                _ => {}
            }
        }
        // Drag the row under `PREPDECK_FEEL_AT` to `PREPDECK_FEEL_TO` (a playlist in the tree, or
        // for "refuse" a place that takes no drop) and let go there.
        "drag" | "refuse" => {
            let a = at.unwrap_or(pos2(800.0, 500.0));
            let b = at_env("PREPDECK_FEEL_TO").unwrap_or(if s == "drag" { pos2(150.0, 420.0) } else { pos2(1000.0, 14.0) });
            match f {
                0 => mv(raw, a),
                2 => click(raw, a, true),
                3..=40 => mv(raw, a + (b - a) * motion::ease((f - 3) as f32 / 37.0)),
                50 => click(raw, b, false),
                52 => eprintln!("feeltest: {s} released over {:?}: drag {:?}", b, app.drag.as_ref().map(|d| d.len())),
                _ => {}
            }
        }
        "panel" | "mode" => {
            let p = at.unwrap_or(pos2(700.0, 14.0));
            match f {
                0 => mv(raw, p),
                4 => click(raw, p, true),
                5 => click(raw, p, false),
                12 => eprintln!("feeltest: {s} after the click: performance {} perf_fx {} layout {}", app.performance(), app.prefs.perf_fx, app.prefs.player_layout),
                _ => {}
            }
        }
        _ => {}
    }
}

fn mixer_line(app: &App) -> String {
    let m = &app.px.mix[0];
    format!("fader 1 {:.3} eq {:?} crossfader {:.3}", m.fader, m.eq, app.px.crossfader)
}
