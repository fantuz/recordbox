//! recordbox's own interface motion (not rekordbox's: rekordbox snaps every state change).
//!
//! Time-based easing keyed by widget id. A widget asks for the value to show this frame
//! (`toward` for a 0..1 state such as hover, `tween` for any number that jumps, `follow` for a
//! number that moves continuously); the store remembers where each one was, eases it with
//! wall-clock time (so 60 Hz and 120 Hz look the same), and counts what has not arrived yet.
//! `end_frame` asks for the next frame only while something is still moving, so the app idles
//! at zero repaints. Settled values are exactly the target, so a screen at rest is drawn with
//! exactly the colours and positions it had before this module existed.
//!
//! Levels (Preferences > Advanced > Others > recordbox > Interface motion, stored under the
//! library setting `recordbox_motion`): Full; Reduced (colour and opacity fades stay, shorter;
//! movement, sliding and smooth scrolling go), also chosen when macOS "Reduce motion" is on;
//! Off (every value is its target: the app behaves and draws exactly as before).
//!
//! `--screenshot` runs: Full unless `PREPDECK_SHOT_MOTION=off|reduced|full` says otherwise,
//! and every value snaps to its target from five frames before the picture (`settle`).

use eframe::egui;
use egui::{Color32, Id, Rgba};
use std::cell::{Cell, RefCell};
use std::ops::Deref;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Full = 0,
    Reduced = 1,
    Off = 2,
}

impl Level {
    pub fn from_u8(v: u8) -> Level {
        match v {
            1 => Level::Reduced,
            2 => Level::Off,
            _ => Level::Full,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Level::Full => "Full",
            Level::Reduced => "Reduced",
            Level::Off => "Off",
        }
    }
}

/// What an animation changes: a colour or an opacity (kept, shorter, in Reduced) or a
/// position or size (dropped in Reduced).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Fade,
    Move,
}

// ---- Durations, chosen once (seconds).
/// Hover coming on: quick, so the pointer never waits for it.
pub const HOVER_IN: f32 = 0.07;
/// Hover going off and a press letting go: a little slower, so leaving feels soft.
pub const HOVER_OUT: f32 = 0.16;
/// A press released.
pub const RELEASE: f32 = 0.14;
/// Selection colour, drop-target highlights, read-outs.
pub const FEEDBACK: f32 = 0.11;
/// A control's drawn value travelling to a value set by a click or a reset.
pub const GLIDE: f32 = 0.12;
/// Panels, pages, modes and layouts appearing.
pub const PANEL: f32 = 0.18;
/// Smooth scrolling to a row, eased zoom.
pub const SCROLL: f32 = 0.14;
/// A refused drag going back to where it came from.
pub const SNAP_BACK: f32 = 0.22;
/// Nothing lasts longer than this.
pub const MAX: f32 = 0.25;

// Interaction state lives on the GUI thread. Tests and separately hosted GUI threads
// cannot change another window manager's effective level.
thread_local! {
    static SETTING: Cell<u8> = const { Cell::new(0) };
    static SYSTEM_REDUCE: Cell<bool> = const { Cell::new(false) };
    static LEVEL: Cell<u8> = const { Cell::new(2) };
    static SETTLE: Cell<bool> = const { Cell::new(false) };
    static LAST_ACTIVE: Cell<u32> = const { Cell::new(0) };
}
/// The level the user chose (before macOS "Reduce motion" is applied).
pub fn setting() -> Level {
    Level::from_u8(SETTING.get())
}

pub fn set_setting(l: Level) {
    SETTING.set(l as u8);
}

/// The level in effect this frame.
#[inline]
pub fn level() -> Level {
    Level::from_u8(LEVEL.get())
}

/// True when anything eases this frame (not Off, not settling for a screenshot).
#[inline]
pub fn on() -> bool {
    LEVEL.get() != Level::Off as u8 && !SETTLE.get()
}

/// True when things may move (Full only).
#[inline]
pub fn moves() -> bool {
    LEVEL.get() == Level::Full as u8 && !SETTLE.get()
}

/// True when recordbox's added feedback (row hover, read-outs, drop glow, cursors) is drawn:
/// any level but Off. Unlike `on`, it stays true while settling, so a screenshot taken with
/// the pointer over something shows the settled feedback.
#[inline]
pub fn extras() -> bool {
    LEVEL.get() != Level::Off as u8
}

/// Whether macOS asks apps to reduce motion (System Settings > Accessibility > Display).
pub fn system_reduce_motion() -> bool {
    SYSTEM_REDUCE.get()
}

/// Values that had not arrived at the end of the last frame.
pub fn last_active() -> u32 {
    LAST_ACTIVE.get()
}

#[derive(Clone, Copy)]
struct Anim {
    from: f32,
    to: f32,
    t0: f64,
    dur: f32,
    seen: u64,
}

struct Store {
    map: egui::IdMap<Anim>,
    now: f64,
    prev: f64,
    dt: f32,
    lead: f32,
    frame: u64,
    active: u32,
    last_poll: f64,
    /// The display clock: where the frame being built will be shown (see `display_time`).
    disp: f64,
    disp_step: f64,
    period: f64,
}

thread_local! {
    static LIBRARY_WINDOW: Cell<bool> = const { Cell::new(false) };
}

/// The two immediate native viewports run on the same UI thread. Identical widget IDs,
/// pointer coordinates and timing samples must only see the state of the window being drawn.
pub(crate) struct PerWindow<T>([T; 2]);

impl<T> PerWindow<T> {
    pub(crate) fn new(make: impl Fn() -> T) -> Self { Self([make(), make()]) }
}

impl<T> Deref for PerWindow<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.0[LIBRARY_WINDOW.with(Cell::get) as usize] }
}

pub fn in_library<T>(draw: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) { LIBRARY_WINDOW.with(|v| v.set(self.0)); }
    }
    let _reset = Reset(LIBRARY_WINDOW.with(|v| v.replace(true)));
    draw()
}

thread_local! {
    static STORE: PerWindow<RefCell<Store>> = PerWindow::new(|| RefCell::new(Store {
        map: egui::IdMap::default(), now: 0.0, prev: 0.0, dt: 1.0 / 60.0, lead: 1.0 / 60.0,
        frame: 0, active: 0, last_poll: -10.0, disp: 0.0, disp_step: 0.0, period: 1.0 / 120.0,
    }));
}

/// Start of a frame (main.rs, before anything is drawn). `shot`: the frame number of a
/// `--screenshot` run and whether its picture is about to be taken.
pub fn begin_frame(ctx: &egui::Context, db: &crate::db::Db, shot: Option<(u32, bool)>) {
    // This app's preference follows its library. A new GUI context performs a fresh read.
    let loaded = egui::Id::new("recordbox_motion_setting");
    if ctx.data(|d| d.get_temp::<bool>(loaded)) != Some(true) {
        let v = db.setting("recordbox_motion").and_then(|s| s.trim().parse::<u8>().ok()).unwrap_or(0);
        SETTING.set(v.min(2));
        ctx.data_mut(|d| d.insert_temp(loaded, true));
    }
    let poll = start_clock(ctx);
    if poll && shot.is_none() {
        SYSTEM_REDUCE.set(query_reduce_motion());
    }
    let mut l = setting();
    if let Some((_, settle)) = shot {
        l = shot_level();
        SETTLE.set(settle);
    } else {
        SETTLE.set(false);
        if l == Level::Full && SYSTEM_REDUCE.get() {
            l = Level::Reduced;
        }
    }
    if LEVEL.replace(l as u8) != l as u8 {
        STORE.with(|s| { for state in &s.0 { state.borrow_mut().map.clear(); } });
    }
}

const MAX_VALUES: usize = 8192;

impl Store {
    fn can_track(&self, id: Id) -> bool {
        self.map.len() < MAX_VALUES || self.map.contains_key(&id)
    }
}

pub fn start_clock(ctx: &egui::Context) -> bool {
    let (now, dt) = ctx.input(|i| (i.time, i.stable_dt));
    STORE.with(|cell| {
        let mut s = cell.borrow_mut();
        step_clock(&mut s, now, dt);
        if s.frame % 120 == 0 || s.map.len() >= MAX_VALUES {
            let frame = s.frame;
            let retain_for = if s.map.len() >= MAX_VALUES { 2 } else { 240 };
            s.map.retain(|_, a| frame.saturating_sub(a.seen) <= retain_for);
        }
        let due = now - s.last_poll > 2.0;
        if due { s.last_poll = now; }
        due
    })
}

/// The frame clock: the measured interval since the last frame, or (after idling, when there
/// was no last frame to measure from) egui's predicted one, never more than a 60 Hz frame. The
/// first frame of a change shows that much progress.
fn step_clock(s: &mut Store, now: f64, stable_dt: f32) {
    let gap = (now - s.now) as f32;
    s.prev = s.now;
    s.now = now;
    s.dt = if gap > 0.0 && gap <= 0.1 { gap } else { stable_dt.clamp(0.0, 1.0 / 60.0) };
    s.lead = s.dt.min(1.0 / 60.0);
    s.frame += 1;
    s.active = 0;
    // The display clock. Frames are started unevenly (measured with the bench: about a third
    // 0-4 ms apart, a third 8 ms, a third 12-15 ms at 120 Hz), while the display shows them one
    // refresh apart. Each frame advances this clock by whole refresh periods (at least one),
    // so something that moves with it moves by the same step every frame; it is pulled back
    // onto real time when it drifts by more than two periods.
    let g = gap as f64;
    if gap > 0.0 && gap <= 0.1 {
        s.period += (g.min(1.0 / 24.0) - s.period) * 0.02;
        // One period per frame; more only when the frame is late by over 1.4 periods (a frame
        // really missed), back onto real time when the clock got ahead of it.
        let mut next = s.disp + s.period;
        let e = now - next;
        if e > 1.4 * s.period {
            next += (e / s.period).round() * s.period;
        }
        if (next - now).abs() > 2.5 * s.period {
            next = now;
        }
        s.disp_step = next - s.disp;
        s.disp = next;
    } else {
        s.disp_step = 0.0;
        s.disp = now;
    }
}

/// The display clock (seconds, egui's time base): advances by whole refresh periods per frame.
pub fn display_time() -> f64 {
    STORE.with(|s| s.borrow().disp)
}

/// How far the display clock moved this frame (0 on the first frame after idling).
pub fn display_step() -> f64 {
    STORE.with(|s| s.borrow().disp_step)
}

/// The estimated refresh period (seconds).
pub fn period() -> f64 {
    STORE.with(|s| s.borrow().period)
}

/// End of a frame: the one place that asks for another frame while something moves.
pub fn end_frame(app: &crate::app::App, ctx: &egui::Context) {
    let active = STORE.with(|s| s.borrow().active);
    LAST_ACTIVE.set(active);
    if active > 0 {
        app.repaint_playback(ctx);
    }
}

fn shot_level() -> Level {
    static L: std::sync::OnceLock<Level> = std::sync::OnceLock::new();
    *L.get_or_init(|| match std::env::var("PREPDECK_SHOT_MOTION").as_deref() {
        Ok("off") => Level::Off,
        Ok("reduced") => Level::Reduced,
        _ => Level::Full,
    })
}

/// Seconds an animation of `kind` takes at the current level (0 = jump).
fn scaled(dur: f32, kind: Kind) -> f32 {
    if !on() {
        return 0.0;
    }
    match (level(), kind) {
        (Level::Full, _) => dur.min(MAX),
        (Level::Reduced, Kind::Fade) => (dur * 0.7).min(MAX),
        _ => 0.0,
    }
}

#[inline]
fn ease_out(p: f32) -> f32 {
    let q = 1.0 - p;
    1.0 - q * q * q
}

#[inline]
fn ease_in_out(p: f32) -> f32 {
    if p < 0.5 {
        4.0 * p * p * p
    } else {
        let q = -2.0 * p + 2.0;
        1.0 - q * q * q / 2.0
    }
}

fn sample(a: &Anim, now: f64) -> (f32, bool) {
    if a.dur <= 0.0 {
        return (a.to, true);
    }
    let p = ((now - a.t0) as f32 / a.dur).clamp(0.0, 1.0);
    if p >= 1.0 {
        (a.to, true)
    } else {
        (a.from + (a.to - a.from) * ease_out(p), false)
    }
}

/// A 0..1 state shown eased: `rise` seconds going up, `fall` coming down (a part of the way
/// takes that part of the time). Unseen ids start at their target.
pub fn toward(id: Id, target: f32, rise: f32, fall: f32, kind: Kind) -> f32 {
    if !on() || (kind == Kind::Move && !moves()) { return target; }
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.can_track(id) { return target; }
        let (now, lead, frame) = (s.now, s.lead, s.frame);
        let a = s.map.entry(id).or_insert(Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame });
        a.seen = frame;
        if a.to != target {
            let (cur, _) = sample(a, now);
            let d = if target > cur { scaled(rise, kind) } else { scaled(fall, kind) };
            *a = Anim { from: cur, to: target, t0: now - lead as f64, dur: d * (target - cur).abs().min(1.0), seen: frame };
        } else if !on() || (kind == Kind::Move && !moves()) {
            a.dur = 0.0;
        }
        let (v, done) = sample(a, now);
        if !done {
            s.active += 1;
        }
        v
    })
}

/// A value that jumps (a click, a reset, a new scroll target) shown travelling to it over
/// `dur` seconds; a new target mid-way starts from where the value is drawn.
pub fn tween(id: Id, target: f32, dur: f32, kind: Kind) -> f32 {
    if !on() || (kind == Kind::Move && !moves()) { return target; }
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.can_track(id) { return target; }
        let (now, lead, frame) = (s.now, s.lead, s.frame);
        let a = s.map.entry(id).or_insert(Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame });
        a.seen = frame;
        if a.to != target {
            let (cur, _) = sample(a, now);
            *a = Anim { from: cur, to: target, t0: now - lead as f64, dur: scaled(dur, kind), seen: frame };
        } else if !on() || (kind == Kind::Move && !moves()) {
            a.dur = 0.0;
        }
        let (v, done) = sample(a, now);
        if !done {
            s.active += 1;
        }
        v
    })
}

/// Like `tween`, but a change smaller than `jump` is followed at once (a drag tracks the
/// pointer exactly; only a jump, such as a click on the track, glides).
pub fn glide(id: Id, target: f32, jump: f32, dur: f32, kind: Kind) -> f32 {
    if !on() || (kind == Kind::Move && !moves()) { return target; }
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.can_track(id) { return target; }
        let (now, lead, frame) = (s.now, s.lead, s.frame);
        let a = s.map.entry(id).or_insert(Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame });
        a.seen = frame;
        if a.to != target {
            let (cur, done) = sample(a, now);
            if (target - a.to).abs() >= jump {
                *a = Anim { from: cur, to: target, t0: now - lead as f64, dur: scaled(dur, kind), seen: frame };
            } else if done || jump.is_infinite() {
                *a = Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame };
            } else {
                // A small move while gliding: keep gliding, to the new place.
                a.to = target;
            }
        } else if !on() || (kind == Kind::Move && !moves()) {
            a.dur = 0.0;
        }
        let (v, done) = sample(a, now);
        if !done {
            s.active += 1;
        }
        v
    })
}

/// A value that moves on its own every frame, shown with frame-rate independent exponential
/// smoothing (`tau` seconds to cover 63 % of the way); it arrives exactly once it is within
/// `eps`.
pub fn follow(id: Id, target: f32, tau: f32, eps: f32, kind: Kind) -> f32 {
    if !on() || (kind == Kind::Move && !moves()) { return target; }
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.can_track(id) { return target; }
        let (now, dt, frame) = (s.now, s.dt, s.frame);
        let a = s.map.entry(id).or_insert(Anim { from: target, to: target, t0: now, dur: 0.0, seen: frame });
        let tau = scaled(tau, kind);
        let gap = (frame - a.seen).max(1);
        a.seen = frame;
        if tau <= 0.0 || gap > 2 {
            a.from = target;
        } else {
            let k = 1.0 - (-dt / tau).exp();
            a.from += (target - a.from) * k;
            if (target - a.from).abs() <= eps {
                a.from = target;
            }
        }
        a.to = target;
        let v = a.from;
        if v != target {
            s.active += 1;
        }
        v
    })
}

/// Set a value at once (a press shows on its own frame).
pub fn set(id: Id, v: f32) {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.can_track(id) { return; }
        let (now, frame) = (s.now, s.frame);
        s.map.insert(id, Anim { from: v, to: v, t0: now, dur: 0.0, seen: frame });
    })
}

/// A one-off pulse: 1 at once, back to 0 over `dur` seconds (a drop accepted, a reset).
pub fn pulse(id: Id, dur: f32) {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.can_track(id) { return; }
        let (now, lead, frame) = (s.now, s.lead, s.frame);
        let d = scaled(dur, Kind::Fade);
        s.map.insert(id, Anim { from: 1.0, to: 0.0, t0: now - lead as f64, dur: d, seen: frame });
    })
}

/// The current value of a pulse (0 when none is running).
pub fn pulse_value(id: Id) -> f32 {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let (now, frame) = (s.now, s.frame);
        let Some(a) = s.map.get_mut(&id) else { return 0.0 };
        a.seen = frame;
        let (v, done) = sample(a, now);
        if !done {
            s.active += 1;
        }
        v
    })
}

/// The frame's time (egui's input clock, seconds).
pub fn now() -> f64 {
    STORE.with(|s| s.borrow().now)
}

/// One frame's worth of time for the first step of a change (see `step_clock`).
pub fn frame_lead() -> f32 {
    STORE.with(|s| s.borrow().lead)
}

/// The previous frame's time.
pub fn now_prev() -> f64 {
    STORE.with(|s| s.borrow().prev)
}

/// Count something drawn this frame that is still moving outside the store (keeps frames
/// coming until it ends).
pub fn keep_alive() {
    STORE.with(|s| s.borrow_mut().active += 1);
}

/// Ease-in-out of a 0..1 progress, for callers that draw their own timeline.
pub fn ease(p: f32) -> f32 {
    ease_in_out(p.clamp(0.0, 1.0))
}

/// `a` to `b` by `t` in linear light (the endpoints are returned exactly).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    if t <= 0.0 || a == b {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let (x, y) = (Rgba::from(a).to_array(), Rgba::from(b).to_array());
    let l = |i: usize| x[i] + (y[i] - x[i]) * t;
    Color32::from(Rgba::from_rgba_premultiplied(l(0), l(1), l(2), l(3)))
}

/// A colour with its opacity multiplied by `t` (linear, premultiplied; exact at 0 and 1).
pub fn fade(c: Color32, t: f32) -> Color32 {
    if t >= 1.0 {
        c
    } else if t <= 0.0 {
        Color32::TRANSPARENT
    } else {
        c.linear_multiply(t)
    }
}

/// `NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion`.
#[cfg(target_os = "macos")]
fn query_reduce_motion() -> bool {
    use objc2::runtime::{AnyClass, AnyObject};
    let Some(cls) = AnyClass::get(c"NSWorkspace") else { return false };
    unsafe {
        let ws: *mut AnyObject = objc2::msg_send![cls, sharedWorkspace];
        if ws.is_null() {
            return false;
        }
        let on: bool = objc2::msg_send![ws, accessibilityDisplayShouldReduceMotion];
        on
    }
}

#[cfg(target_os = "windows")]
fn query_reduce_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION};
    let mut enabled = 1i32;
    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut enabled as *mut i32).cast(), 0) != 0 && enabled == 0 }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn query_reduce_motion() -> bool {
    false
}

/// Store the chosen level in the library database.
pub fn save_setting(db: &crate::db::Db, l: Level) {
    set_setting(l);
    db.set_setting("recordbox_motion", &(l as u8).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The level is one global: the tests that set it run one at a time.

    fn frame(ctx: &egui::Context, t: f64, f: impl FnOnce()) {
        let raw = egui::RawInput { time: Some(t), predicted_dt: 1.0 / 120.0, ..Default::default() };
        let mut f = Some(f);
        let _ = ctx.run(raw, |ctx| {
            let (now, dt) = ctx.input(|i| (i.time, i.stable_dt));
            STORE.with(|s| step_clock(&mut s.borrow_mut(), now, dt));
            if let Some(f) = f.take() {
                f();
            }
        });
    }

    #[test]
    fn hover_eases_and_settles_exactly() {
        LEVEL.set(Level::Full as u8);
        SETTLE.set(false);
        let ctx = egui::Context::default();
        let id = Id::new("t_hover");
        let mut v = 0.0;
        frame(&ctx, 0.0, || v = toward(id, 0.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert_eq!(v, 0.0);
        frame(&ctx, 1.0, || v = toward(id, 1.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert!(v > 0.0 && v < 1.0, "first frame shows some progress: {v}");
        frame(&ctx, 1.03, || v = toward(id, 1.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert!(v > 0.5 && v < 1.0, "{v}");
        frame(&ctx, 1.2, || v = toward(id, 1.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert_eq!(v, 1.0);
        // Off: always the target.
        LEVEL.set(Level::Off as u8);
        frame(&ctx, 1.21, || v = toward(id, 0.0, HOVER_IN, HOVER_OUT, Kind::Fade));
        assert_eq!(v, 0.0);
        LEVEL.set(Level::Full as u8);
    }

    #[test]
    fn reduced_keeps_fades_and_drops_moves() {
        LEVEL.set(Level::Reduced as u8);
        SETTLE.set(false);
        let ctx = egui::Context::default();
        let (a, b) = (Id::new("t_fade"), Id::new("t_move"));
        let (mut f, mut m) = (0.0, 0.0);
        frame(&ctx, 0.0, || {
            toward(a, 0.0, 0.1, 0.1, Kind::Fade);
            tween(b, 0.0, 0.1, Kind::Move);
        });
        frame(&ctx, 0.5, || {
            f = toward(a, 1.0, 0.1, 0.1, Kind::Fade);
            m = tween(b, 50.0, 0.1, Kind::Move);
        });
        assert!(f < 1.0);
        assert_eq!(m, 50.0);
        LEVEL.set(Level::Full as u8);
    }

    #[test]
    fn glide_tracks_small_moves_exactly() {
        LEVEL.set(Level::Full as u8);
        SETTLE.set(false);
        let ctx = egui::Context::default();
        let id = Id::new("t_glide");
        let mut v = 0.0;
        frame(&ctx, 0.0, || v = glide(id, 10.0, 4.0, GLIDE, Kind::Move));
        frame(&ctx, 0.01, || v = glide(id, 11.0, 4.0, GLIDE, Kind::Move));
        assert_eq!(v, 11.0, "a drag step is followed at once");
        frame(&ctx, 0.02, || v = glide(id, 40.0, 4.0, GLIDE, Kind::Move));
        assert!(v > 11.0 && v < 40.0, "a jump glides: {v}");
        frame(&ctx, 0.5, || v = glide(id, 40.0, 4.0, GLIDE, Kind::Move));
        assert_eq!(v, 40.0);
    }

    #[test]
    fn follow_is_frame_rate_independent() {
        LEVEL.set(Level::Full as u8);
        SETTLE.set(false);
        let run = |hz: f64| {
            let ctx = egui::Context::default();
            let id = Id::new(("t_follow", hz as i32));
            let mut v = 0.0;
            frame(&ctx, 0.0, || v = follow(id, 0.0, 0.03, 0.01, Kind::Move));
            let n = (0.05 * hz) as i32;
            for k in 1..=n {
                frame(&ctx, k as f64 / hz, || v = follow(id, 100.0, 0.03, 0.01, Kind::Move));
            }
            v
        };
        let (a, b) = (run(60.0), run(120.0));
        assert!((a - b).abs() < 3.0, "60 Hz {a} vs 120 Hz {b}");
    }

    #[test]
    fn root_and_library_targets_do_not_cross_or_repaint_a_settled_window() {
        let ctx = egui::Context::default();
        LEVEL.set(Level::Full as u8);
        let id = Id::new("same library row");
        for n in 0..15 {
            let t = n as f64 / 30.0;
            frame(&ctx, t, || {
                assert_eq!(toward(id, 1.0, HOVER_IN, HOVER_OUT, Kind::Fade), 1.0);
            });
            in_library(|| frame(&ctx, t, || {
                assert_eq!(toward(id, 0.0, HOVER_IN, HOVER_OUT, Kind::Fade), 0.0);
                assert_eq!(STORE.with(|s| s.borrow().active), 0);
            }));
            assert_eq!(STORE.with(|s| s.borrow().active), 0);
        }
    }

    #[test]
    fn external_cut_interrupts_a_running_mouse_reset_on_the_same_frame() {
        let ctx = egui::Context::default();
        LEVEL.set(Level::Full as u8);
        let id = Id::new("mix fader");
        frame(&ctx, 0.0, || { glide(id, 0.1, f32::INFINITY, GLIDE, Kind::Move); });
        frame(&ctx, 0.1, || { assert!(glide(id, 0.9, 0.0, GLIDE, Kind::Move) < 0.9); });
        frame(&ctx, 0.12, || {
            assert_eq!(glide(id, 0.0, f32::INFINITY, GLIDE, Kind::Move), 0.0);
            assert_eq!(STORE.with(|s| s.borrow().active), 0);
        });
    }

    #[test]
    fn an_unbounded_stream_of_new_ids_is_capped_per_viewport() {
        LEVEL.set(Level::Full as u8);
        for n in 0..MAX_VALUES + 128 { toward(Id::new(n), 0.0, HOVER_IN, HOVER_OUT, Kind::Fade); }
        assert!(STORE.with(|s| s.borrow().map.len()) <= MAX_VALUES);
    }

    #[test]
    fn mix_is_exact_at_the_ends_and_linear_between() {
        let (a, b) = (Color32::from_rgb(0x28, 0x28, 0x28), Color32::from_rgb(0x3C, 0x3C, 0x3C));
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        let m = mix(Color32::BLACK, Color32::WHITE, 0.5);
        // Half way in linear light is #BCBCBC in sRGB, not #808080.
        assert!((m.r() as i32 - 0xBC).abs() <= 1, "{m:?}");
    }
}
