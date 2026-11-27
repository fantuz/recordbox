//! A controller's live mapping and the arithmetic around it, after rekordbox 7.2.19
//! (`checks/re-7.2.19/controllers-midi.md`): the lookup table by `status << 8 | data1`, the
//! value rule of every control type, LED values, the priority queue and Blink for output,
//! the MIDI Learn rules, tempo soft takeover and the jog wheel maths. Nothing here touches a
//! MIDI port or the app: `midi.rs` does that.

use crate::midi_csv::{entries, CsvFile, Entry, Kind, Opts, Row};
use std::collections::{HashMap, VecDeque};

/// The name rekordbox gives a code whose factory function was taken away (it stays switched
/// off, and the code may be given to something else).
pub const NO_FUNCTION: &str = "NoFunction";

/// What a message means once its row's type rule has been applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    /// Button, Pad, JogTouch: pressed / released, with the velocity byte.
    On(bool, u8),
    /// A pad's pressure while it is held (the pad families that report it).
    Velocity(u8),
    /// Absolute position `v` between `lo` and `hi`, with `mid` the value of the centre.
    Abs { v: i32, lo: i32, mid: i32, hi: i32 },
    /// The byte as sent (RawValue).
    Raw(u8),
    /// Relative steps (Rotary after acceleration, Difference).
    Rel(i32),
    /// Jog counts with the time of the message in milliseconds.
    Jog { delta: i32, t_ms: f64 },
}

impl Val {
    /// An absolute value as 0..1, the centre at exactly 0.5.
    pub fn unit(&self) -> Option<f32> {
        match *self {
            Val::Abs { v, lo, mid, hi } => {
                let v = v.clamp(lo, hi);
                Some(if hi <= lo {
                    0.0
                } else if mid <= lo || mid >= hi {
                    (v - lo) as f32 / (hi - lo) as f32
                } else if v <= mid {
                    0.5 * (v - lo) as f32 / (mid - lo) as f32
                } else {
                    0.5 + 0.5 * (v - mid) as f32 / (hi - mid) as f32
                })
            }
            Val::Raw(b) => Some(b as f32 / 127.0),
            _ => None,
        }
    }
}

/// A mapped message: which input entry, for which deck, and its value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub entry: usize,
    /// 0 = no deck, 1..4.
    pub deck: u8,
    pub val: Val,
    /// The row's `Fast` option, or a jog type: handled without waiting for the interface.
    pub fast: bool,
    /// What the app registered for this entry's function (`MidiMap::tag_inputs`).
    pub tag: u32,
}

/// Which family a `Pad` row belongs to, by device (rekordbox's switch types 29 to 33).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadFamily {
    Rz,
    Rx,
    Sz,
    Sx2,
    Sx,
}

/// The pad family of a device name. The comparisons are rekordbox's (`getSwitchType`
/// 0x101ecc400): DDJ-RZ and RZX; DDJ-RX, RR, XP1/XP2, 1000, 800, FLX10, REV5, REV7, XDJ-XZ,
/// AZ, RX2, RX3, OPUS-QUAD, DJM-S9 / S11 / S7, PLX-CRSS12; DDJ-SZ; DDJ-SX2; everything else.
pub fn pad_family(device: &str) -> PadFamily {
    let d = device.to_ascii_uppercase();
    let has = |s: &str| d.contains(s);
    if has("DDJ-RZ") {
        PadFamily::Rz
    } else if has("DDJ-SX2") {
        PadFamily::Sx2
    } else if has("DDJ-SZ") {
        PadFamily::Sz
    } else if ["DDJ-RX", "DDJ-RR", "DDJ-XP", "DDJ-1000", "DDJ-800", "DDJ-FLX10", "DDJ-REV5", "DDJ-REV7", "XDJ-XZ", "XDJ-AZ", "XDJ-RX2", "XDJ-RX3", "XDJ-RR", "OPUS-QUAD", "DJM-S9", "DJM-S11", "DJM-S7", "PLX-CRSS12"].iter().any(|s| has(s)) {
        PadFamily::Rx
    } else {
        PadFamily::Sx
    }
}

/// The value a pad LED is sent for the value the app asks for (`Midi*Pad::setCharValue`).
pub fn pad_led(family: PadFamily, v: u8) -> u8 {
    // The RX family's remap of the codes 31 to 41 (two of the device's codes are skipped).
    let rx = |v: u8| if (31..=41).contains(&v) { v + 1 + (v >= 32) as u8 - (v >= 35) as u8 + (v >= 40) as u8 - (v >= 41) as u8 } else { v };
    match family {
        PadFamily::Rz => match v {
            127 => 121,
            57 => 64,
            v => v,
        },
        PadFamily::Rx => rx(v),
        PadFamily::Sz => if v <= 64 { v } else { 0 },
        PadFamily::Sx2 => if v > 64 { 0 } else { rx(v) },
        PadFamily::Sx => if button_lit(v) { 127 } else { 0 },
    }
}

/// rekordbox's test for "this value lights a button" (`MidiButton::setCharValue`): 127, or 1
/// to 64 except 63.
fn button_lit(v: u8) -> bool {
    v == 127 || ((1..=64).contains(&v) && v != 63)
}

/// The byte a Button row's LED is sent: its `Max` when lit and `Min` when not, where the row
/// gives them ("Button(for Pad)"), else 127 / 0.
pub fn button_led(v: u8, o: &Opts) -> u8 {
    if button_lit(v) {
        if o.max != -1 { o.max as u8 } else { 127 }
    } else if o.min != -1 {
        o.min as u8
    } else {
        0
    }
}

/// The jog ring position for an angle of 0..359 degrees (`MidiJogIndicator::setLongValue`):
/// `Min` is the offset and `Max` the number of steps; other angles give 0.
pub fn jog_ring(angle: i32, o: &Opts) -> u8 {
    if !(0..360).contains(&angle) || o.max < 0 {
        return 0;
    }
    ((o.min.max(0) + (o.max * angle) / 360) & 255) as u8
}

/// Lower bound, upper bound and half span of a Value (7-bit default) or KnobSlider (14-bit
/// default) row, as `MidiData::createInstance` 0x1013204e0 sets them from `Min` / `Max`.
fn bounds(kind: Kind, o: &Opts) -> (i32, i32, f64) {
    let lo = if (o.min as u32) < 0x4000 { o.min } else { 0 };
    let hi = match kind {
        Kind::KnobSlider => if (o.max as u32) >= 0x3FFF { 0x3FFF } else { o.max },
        _ => if (o.max as u32) < 0x4000 { o.max } else { 0x7F },
    };
    // (rekordbox works this out in 16 bits and halves towards zero.)
    let span = (hi - lo + 1) as i16;
    (lo, hi, ((span as i32 + (span < 0) as i32) >> 1) as f64)
}

/// `MidiValue` / `MidiKnobSlider::handleInputMidiData`: 0..63 run from the lower bound to the
/// half span, 64 is the half span, 65..127 run on to the upper bound; truncated.
pub fn scale7(b: u8, lo: i32, hi: i32, half: f64) -> i32 {
    let b = b as i32;
    if b <= 63 {
        (lo as f64 + b as f64 * (0.015625 * half)) as i32
    } else {
        let mut v = half + lo as f64;
        if b != 64 {
            v += (b - 64) as f64 * (0.0158730159 * ((hi - lo) as f64 - half));
        }
        v as i32
    }
}

/// A relative encoder's byte as steps (`MidiRotary`, first stage): 1..30 as they are, 98..127
/// as two's complement, 34..63 and 65..94 around 64; other values pass.
pub fn rotary_steps(b: u8) -> i32 {
    let b = b as i32;
    if (1..=30).contains(&b) {
        b
    } else if b >= 98 {
        b - 128
    } else if (34..=63).contains(&b) || (65..=94).contains(&b) {
        b - 64
    } else {
        b
    }
}

/// The acceleration rekordbox applies after `rotary_steps`: `base^(n-1)` for n steps forward
/// (0..30), `-base^(-n-1)` backward (-29..0), truncated. `Value=` gives the base x 100
/// (default 2.22); a `Value` of 0 or less switches it off.
pub fn rotary_accel(steps: i32, value_opt: i32) -> i32 {
    let (base, on) = if value_opt == -1 { (2.22, true) } else { (value_opt as f64 * 0.01, value_opt > 0) };
    if !on {
        return steps;
    }
    if steps < 0 {
        ((-(base.powi(-steps - 1))) as i32).clamp(-29, 0)
    } else {
        (base.powi(steps - 1) as i32).clamp(0, 30)
    }
}

/// What the table holds for one code.
#[derive(Clone, Debug)]
pub struct Input {
    pub e: Entry,
    /// The other deck's code of a `Dual` row (1 with 3, 2 with 4), when it has one.
    pub peer: Option<u16>,
    pub tag: u32,
    /// 14-bit pair: the value so far (the MSB shifted up, the LSB or-ed in).
    cache: u16,
    /// RotaryAsKnob: the accumulated position.
    acc: i32,
    /// JogRotateAbsPos: the last position.
    last_pos: Option<u8>,
}

#[derive(Clone, Copy, PartialEq)]
enum Slot {
    Empty,
    /// The entry itself (for a 14-bit row: its MSB).
    Whole(u32),
    /// The LSB half of a 14-bit row.
    Lsb(u32),
}

/// One device's mapping: the factory file with the user's file on top.
pub struct MidiMap {
    pub device: String,
    pub inputs: Vec<Option<Input>>,
    pub outputs: Vec<Entry>,
    table: Vec<Slot>,
    /// `Parameter` rows by name (MidiOutInterval, JogIndicatorInterval ...): their `Value`.
    pub params: HashMap<String, i32>,
    pub family: PadFamily,
    /// JogTimestamp counts waiting for their position message, by deck.
    stamp: [u16; 5],
    /// Rows that were not taken because their code was already in use (factory files).
    pub rejected: usize,
}

fn key_of(msg: &[u8]) -> Option<usize> {
    if msg.len() < 3 || msg[0] < 0x80 || msg[0] == 0xF0 {
        return None;
    }
    Some(if msg[0] <= 0xDF { (msg[0] as usize) << 8 | msg[1] as usize } else { msg[0] as usize })
}

impl MidiMap {
    pub fn new(device: &str) -> MidiMap {
        MidiMap { device: device.to_string(), inputs: vec![], outputs: vec![], table: vec![Slot::Empty; 65536], params: HashMap::new(), family: pad_family(device), stamp: [0; 5], rejected: 0 }
    }

    /// The factory file, when the installed rekordbox has one for the device, then the user's
    /// file on top. `Err` when the user's file is not a mapping of this device.
    pub fn load(device: &str, factory: Option<&[u8]>, user: Option<&[u8]>) -> Result<MidiMap, ()> {
        let mut m = MidiMap::new(device);
        if let Some(f) = factory.and_then(|b| CsvFile::parse(b).ok()) {
            for r in f.rows() {
                m.add_row(r, false, false);
            }
        }
        if let Some(b) = user {
            let u = CsvFile::parse(b).map_err(|_| ())?;
            if !u.device.eq_ignore_ascii_case(device) {
                return Err(());
            }
            for r in u.rows() {
                m.add_row(r, true, true);
            }
        }
        Ok(m)
    }

    /// Put a row's codes in the tables. `overlay`: a code already there is replaced (the
    /// user's file); otherwise the first row to claim a code keeps it.
    pub fn add_row(&mut self, row: &Row, user: bool, overlay: bool) {
        if row.kind == Kind::Parameter {
            self.params.insert(row.name.clone(), row.opts.value);
            return;
        }
        let (ins, outs) = entries(row, user);
        for e in ins {
            if row.kind.output_only() {
                continue;
            }
            let peer = if e.opts.dual && e.deck >= 1 { row.input[[0, 3, 4, 1, 2][e.deck as usize]].filter(|c| *c != e.code) } else { None };
            if overlay {
                self.remove_input_code(e.code);
                if e.kind.is_14bit() {
                    self.remove_input_code(e.code.wrapping_add(0x20));
                }
            }
            if !self.add_input(e, peer) {
                self.rejected += 1;
            }
        }
        for e in outs {
            if overlay {
                self.outputs.retain(|o| !(o.code == e.code && o.deck == e.deck));
            }
            self.outputs.push(e);
        }
    }

    /// Register an input entry; false when its code (or, for a 14-bit row, the code + 0x20)
    /// is taken.
    pub fn add_input(&mut self, e: Entry, peer: Option<u16>) -> bool {
        let c = e.code as usize;
        let lsb = (e.code.wrapping_add(0x20)) as usize;
        if self.table[c] != Slot::Empty || (e.kind.is_14bit() && self.table[lsb] != Slot::Empty) {
            return false;
        }
        let i = self.inputs.len() as u32;
        self.table[c] = Slot::Whole(i);
        if e.kind.is_14bit() {
            self.table[lsb] = Slot::Lsb(i);
        }
        let acc = match (e.kind, e.opts.value) {
            (Kind::RotaryAsKnob, v) if v >= 1 => (v.min(0x3FFF) as f64 * 0.5).round() as i32,
            _ => 64,
        };
        self.inputs.push(Some(Input { e, peer, tag: 0, cache: 0, acc, last_pos: None }));
        true
    }

    /// Take the entry at a code out (both halves of a 14-bit pair).
    pub fn remove_input_code(&mut self, code: u16) -> Option<Entry> {
        let i = match self.table[code as usize] {
            Slot::Whole(i) | Slot::Lsb(i) => i,
            Slot::Empty => return None,
        };
        let e = self.inputs[i as usize].take()?.e;
        self.table[e.code as usize] = Slot::Empty;
        if e.kind.is_14bit() {
            self.table[e.code.wrapping_add(0x20) as usize] = Slot::Empty;
        }
        Some(e)
    }

    /// The entry a code belongs to (for a 14-bit pair: either of its two codes).
    pub fn input_at(&self, code: u16) -> Option<&Entry> {
        match self.table[code as usize] {
            Slot::Whole(i) | Slot::Lsb(i) => self.inputs[i as usize].as_ref().map(|x| &x.e),
            Slot::Empty => None,
        }
    }

    pub fn outputs_at(&self, code: u16) -> Vec<Entry> {
        self.outputs.iter().filter(|o| o.code == code).cloned().collect()
    }

    pub fn live_inputs(&self) -> impl Iterator<Item = &Input> {
        self.inputs.iter().flatten()
    }

    /// Let the app mark the entries of the functions it knows (0 = not known).
    pub fn tag_inputs(&mut self, f: impl Fn(&str) -> u32) {
        for i in self.inputs.iter_mut().flatten() {
            i.tag = f(&i.e.name);
        }
    }

    /// `MidiOutInterval` in milliseconds as the output thread uses it: 1 to 16, else 1.
    pub fn out_interval_ms(&self) -> u64 {
        match self.params.get("MidiOutInterval").copied().unwrap_or(1) {
            v @ 1..=16 => v as u64,
            _ => 1,
        }
    }

    /// Look a message up and apply its row's rule. `t_ms`: when it arrived.
    ///
    /// One deliberate difference from rekordbox: a real Note Off (8n) is looked up as the
    /// Note On of the same note with velocity 0, so pads of other makers release.
    pub fn handle(&mut self, msg: &[u8], t_ms: f64) -> Option<Event> {
        let mut m = [msg.first().copied()?, msg.get(1).copied().unwrap_or(0), msg.get(2).copied().unwrap_or(0)];
        if msg.len() < 3 && m[0] <= 0xDF {
            return None;
        }
        if m[0] & 0xF0 == 0x80 {
            m = [0x90 | (m[0] & 0x0F), m[1], 0];
        }
        let k = key_of(&m)?;
        let (i, lsb) = match self.table[k] {
            Slot::Whole(i) => (i as usize, false),
            Slot::Lsb(i) => (i as usize, true),
            Slot::Empty => return None,
        };
        let family = self.family;
        let stamp = self.stamp;
        let inp = self.inputs[i].as_mut()?;
        let (kind, o, deck) = (inp.e.kind, inp.e.opts, inp.e.deck);
        let b = m[2];
        let val = match kind {
            Kind::Button => {
                if b >= 0x80 {
                    return None;
                }
                Val::On(b != 0, b)
            }
            Kind::Pad => match family {
                // These families report pressure: 127 is the press, 0 the release.
                PadFamily::Rz | PadFamily::Rx => match b {
                    127 => Val::On(true, 127),
                    0 => Val::On(false, 0),
                    v => Val::Velocity(v),
                },
                _ => {
                    if b >= 0x80 {
                        return None;
                    }
                    Val::On(b != 0, b)
                }
            },
            Kind::JogTouch => Val::On(b == 127, b),
            Kind::KnobSlider | Kind::Value => {
                let (lo, hi, half) = bounds(kind, &o);
                Val::Abs { v: scale7(b, lo, hi, half), lo, mid: lo + half as i32, hi }
            }
            Kind::RawValue => Val::Raw(b),
            Kind::KnobSliderHiRes | Kind::Slider | Kind::Knob => {
                if !lsb {
                    // The MSB alone is kept and not passed on.
                    inp.cache = (b as u16) << 7;
                    return None;
                }
                inp.cache |= b as u16;
                Val::Abs { v: inp.cache as i32, lo: 0, mid: 8192, hi: 16383 }
            }
            Kind::Difference => Val::Jog { delta: b as i32 - 64, t_ms },
            Kind::JogRotate => Val::Jog { delta: b as i32 - 64, t_ms },
            Kind::Rotary => Val::Rel(rotary_accel(rotary_steps(b), o.value)),
            Kind::RotaryAsKnob => {
                let limit = if o.value >= 1 { o.value.min(0x3FFF) } else { 127 };
                let d = if b == 0 { 0 } else if b <= 63 { b as i32 } else { b as i32 - 128 };
                inp.acc = (inp.acc + d).clamp(0, limit);
                Val::Abs { v: (inp.acc as f64 * 16383.0 / limit as f64) as i32, lo: 0, mid: 8192, hi: 16383 }
            }
            Kind::JogTimestamp => {
                // A 14-bit count (0.0004 ms each) kept for the deck's next position message.
                let d = (m[0] & 0x0F) as usize;
                self.stamp[d.min(4)] = m[1] as u16 | (m[2] as u16) << 7;
                return None;
            }
            Kind::JogRotateAbsPos => {
                // The platter position, 0..127 around: the shortest way from the last one.
                let prev = inp.last_pos.replace(b);
                let Some(p) = prev else { return None };
                let mut d = b as i32 - p as i32;
                if d > 64 {
                    d -= 128;
                } else if d < -64 {
                    d += 128;
                }
                Val::Jog { delta: d, t_ms: stamp[(deck as usize).saturating_sub(1).min(4)] as f64 * 0.0004 }
            }
            Kind::Toggle | Kind::Indicator | Kind::IndicatorHiRes | Kind::JogIndicator | Kind::JogIndicatorHiRes | Kind::Parameter => return None,
        };
        let fast = o.fast || matches!(kind, Kind::JogRotate | Kind::JogRotateAbsPos | Kind::JogTouch | Kind::Difference);
        Some(Event { entry: i, deck, val, fast, tag: inp.tag })
    }

    /// The message that sets an output entry's LED to `v` (0 = off, 127 = on, or a pad's
    /// colour value): status, data1, the value after the row's rule.
    pub fn led_message(&self, e: &Entry, v: u8) -> [u8; 3] {
        let val = match e.kind {
            Kind::Pad => pad_led(self.family, v),
            Kind::Button => button_led(v, &e.opts),
            _ => v,
        };
        [(e.code >> 8) as u8, (e.code & 0xFF) as u8, val & 0x7F]
    }

    // ------------------------------------------------------------------ MIDI Learn rules

    /// MIDI IN of a row changed from `old` to `new` (`modifyMidiData`, column 4).
    /// `learnable`: the name is one of the Learn window's commands.
    pub fn change_input(&mut self, row: &LearnEntry, old: Option<u16>, new: Option<u16>, learnable: &dyn Fn(&str) -> bool) -> Result<(), Reject> {
        if row.kind.output_only() {
            return Err(Reject::Revert);
        }
        if let Some(c) = new {
            for o in self.outputs_at(c) {
                if o.opts.ro || (o.name != NO_FUNCTION && !learnable(&o.name)) {
                    return Err(Reject::Taken(o));
                }
            }
            let codes: Vec<u16> = if row.kind.is_14bit() { vec![c, c.wrapping_add(0x20)] } else { vec![c] };
            for cc in codes {
                if let Some(i) = self.input_at(cc) {
                    if i.name.starts_with("Shift") {
                        return Err(Reject::Revert);
                    }
                    if i.name != NO_FUNCTION {
                        return Err(Reject::Taken(i.clone()));
                    }
                }
            }
        }
        // The old code is not freed: it stays, named NoFunction, so that the factory action
        // of that control stays switched off.
        if let Some(o) = old {
            if let Some(mut e) = self.remove_input_code(o) {
                e.name = NO_FUNCTION.to_string();
                e.user = true;
                self.add_input(e, None);
            }
        }
        if let Some(c) = new {
            self.remove_input_code(c);
            if row.kind.is_14bit() {
                self.remove_input_code(c.wrapping_add(0x20));
            }
            let e = row.entry(c, &self.device);
            self.add_input(e, None);
        }
        Ok(())
    }

    /// MIDI OUT of a row changed (`modifyMidiData`, column 7): only for types that can light.
    pub fn change_output(&mut self, row: &LearnEntry, old: Option<u16>, new: Option<u16>, learnable: &dyn Fn(&str) -> bool) -> Result<(), Reject> {
        if !row.kind.can_light() {
            return Err(Reject::Revert);
        }
        if let Some(c) = new {
            if let Some(i) = self.input_at(c) {
                if i.name.starts_with("Shift") {
                    return Err(Reject::Revert);
                }
                if i.opts.ro || (i.name != NO_FUNCTION && !learnable(&i.name)) {
                    return Err(Reject::Taken(i.clone()));
                }
            }
            for o in self.outputs_at(c) {
                if o.name.starts_with("Shift") {
                    return Err(Reject::Revert);
                }
                if o.name != NO_FUNCTION {
                    return Err(Reject::Taken(o));
                }
            }
        }
        if let Some(o) = old {
            for e in self.outputs.iter_mut().filter(|e| e.code == o) {
                e.name = NO_FUNCTION.to_string();
                e.user = true;
            }
        }
        if let Some(c) = new {
            self.outputs.retain(|o| o.code != c);
            self.outputs.push(row.entry(c, &self.device));
        }
        Ok(())
    }

    /// Deck, Type or Comment of a row changed: its entries are rewritten in place.
    pub fn rewrite(&mut self, row: &LearnEntry, input: Option<u16>, output: Option<u16>) {
        if let Some(c) = input {
            if let Some(old) = self.remove_input_code(c) {
                let mut e = row.entry(c, &self.device);
                e.opts = Opts { min: row.min, max: row.max, ..old.opts };
                let keep = old.clone();
                e.function = keep.function.clone();
                if !self.add_input(e, None) {
                    // (A 14-bit type whose second code is taken: the entry stays as it was.)
                    self.add_input(keep, None);
                }
            }
        }
        if let Some(c) = output {
            for e in self.outputs.iter_mut().filter(|e| e.code == c) {
                e.deck = row.deck;
                e.kind = row.kind;
                e.comment = row.comment.clone();
                e.opts.min = row.min;
                e.opts.max = row.max;
                e.user = true;
            }
        }
    }

    /// DELETE: the row's codes stay as NoFunction entries of the user.
    pub fn delete(&mut self, input: Option<u16>, output: Option<u16>) {
        if let Some(c) = input {
            if let Some(mut e) = self.remove_input_code(c) {
                e.name = NO_FUNCTION.to_string();
                e.user = true;
                self.add_input(e, None);
            }
        }
        if let Some(c) = output {
            for e in self.outputs.iter_mut().filter(|e| e.code == c) {
                e.name = NO_FUNCTION.to_string();
                e.user = true;
            }
        }
    }

    /// The user's part of the mapping as rekordbox saves it.
    pub fn save_text(&self) -> String {
        let ins: Vec<Entry> = self.live_inputs().map(|i| i.e.clone()).collect();
        crate::midi_csv::save_delta(&self.device, &ins, &self.outputs)
    }
}

/// A row of the Learn window as the map needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct LearnEntry {
    pub name: String,
    pub deck: u8,
    pub kind: Kind,
    /// "Button(for Pad)" keeps LED values here; -1 otherwise.
    pub min: i32,
    pub max: i32,
    pub comment: String,
}

impl LearnEntry {
    /// The entry a learned code gets: rekordbox's default options for the function
    /// (`setOption`), and Rotary rows of an XDJ-RX get `Value=130`.
    pub fn entry(&self, code: u16, device: &str) -> Entry {
        let mut opts = learn_opts(&self.name);
        if self.kind == Kind::Button && (self.min != -1 || self.max != -1) {
            opts.min = self.min;
            opts.max = self.max;
        }
        if self.kind == Kind::Rotary && device.contains("PIONEER XDJ-RX") {
            opts.value = 130;
        }
        Entry { name: self.name.clone(), function: self.name.clone(), kind: self.kind, deck: self.deck, code, opts, comment: self.comment.clone(), user: true }
    }
}

/// Why a Learn edit was put back.
#[derive(Clone, Debug, PartialEq)]
pub enum Reject {
    /// Nothing to say (an Indicator row's MIDI IN, a type that cannot light, a Shift code).
    Revert,
    /// The code belongs to this entry.
    Taken(Entry),
}

/// rekordbox's options for a learned row (`MidiLearnComponent::setOption` 0x10165cbb4).
pub fn learn_opts(name: &str) -> Opts {
    let mut o = Opts::default();
    const FAST: [&str; 33] = ["TempoSlider", "CrossFader", "Gain", "EQHigh", "EQMid", "EQLow", "CFXParameterCH1", "CFXParameterCH2", "CFXParameterCH3", "CFXParameterCH4", "CFXParameterSampler", "CFXParameterMaster", "HotCue", "BeatJump", "PadFx1", "PadFx2", "Slicer", "SlicerLoop", "Sampler", "VelocitySampler", "SequenceCall", "ActiveCensor", "BeatLoop", "KeyShift", "Keyboard", "SlipReverse", "4BeatLoop", "LoopIn", "LoopOut", "ReloopExit", "LoopHalf", "LoopDouble", "4BeatLoopLongPress"];
    match name {
        "PlayPause" => {
            o.fast = true;
            o.blink = 1000;
            o.priority = 50;
        }
        "Cue" => {
            o.fast = true;
            o.blink = 500;
            o.priority = 50;
        }
        "JumpToTrackStart" => o.priority = 50,
        "NeedleSearch" => o.max = 16383,
        "Sync" => o.blink = 600,
        "Reverse" | "SequencerRec" => o.blink = 500,
        "ReTrigger" | "LoopHalfDouble" => o.fast = true,
        n if FAST.contains(&n) => o.fast = true,
        n if is_pad_name(n) => o.fast = true,
        _ => {}
    }
    o
}

/// `PAD1_...` to `PAD16_...`.
pub fn is_pad_name(n: &str) -> bool {
    n.strip_prefix("PAD").and_then(|r| r.split_once('_')).map(|(d, _)| d.parse::<u8>().map(|v| (1..=16).contains(&v)).unwrap_or(false)).unwrap_or(false)
}

/// What MIDI Learn does with the messages it is shown (`eventPropertyChangedDelegate`
/// 0x10165af78): the second half of a 14-bit pair and the release of the button just learned
/// are passed over; anything else gives the code to write into the selected row.
#[derive(Default)]
pub struct LearnFilter {
    prev: [u8; 3],
}

impl LearnFilter {
    pub fn reset(&mut self) {
        self.prev = [0; 3];
    }
    pub fn feed(&mut self, msg: &[u8]) -> Option<u16> {
        if msg.len() != 3 || msg[0] >= 0xF0 {
            return None;
        }
        let (cur, prev) = (msg[0] & 0xF0, self.prev[0] & 0xF0);
        let lsb = cur == 0xB0 && prev == 0xB0 && self.prev[1] as u16 + 32 == msg[1] as u16;
        let release = prev == 0x90 && self.prev[2] != 0 && self.prev[1] == msg[1] && (cur == 0x80 || (cur == 0x90 && msg[2] == 0));
        self.prev = [msg[0], msg[1], msg[2]];
        if lsb || release {
            None
        } else {
            Some((msg[0] as u16) << 8 | msg[1] as u16)
        }
    }
}

// ---------------------------------------------------------------------- output

/// The output queue of a port (`MidiOutputHelper::sendPriorityMessage`): a new message
/// replaces a queued one for the same status and data1, and higher priorities go first,
/// messages of one priority in the order they came.
#[derive(Default)]
pub struct OutQueue {
    items: VecDeque<(i32, [u8; 3])>,
}

impl OutQueue {
    pub fn push(&mut self, msg: [u8; 3], priority: i32) {
        self.items.retain(|(_, m)| !(m[0] == msg[0] && m[1] == msg[1]));
        let at = self.items.iter().position(|(p, _)| *p < priority).unwrap_or(self.items.len());
        self.items.insert(at, (priority, msg));
    }
    pub fn pop(&mut self) -> Option<[u8; 3]> {
        self.items.pop_front().map(|x| x.1)
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// Blinking LEDs (`BlinkManager`): a 50 ms tick; a cycle of `ms` is that many ticks / 50,
/// lit for its first half. An entry without a `Blink=` value cycles in 500 ms.
#[derive(Default)]
pub struct Blinker {
    /// (code, cycle in ticks, tick within the cycle).
    items: Vec<(u16, u32, u32)>,
}

impl Blinker {
    /// Start (or keep) blinking a code. Returns true when it was not blinking before.
    pub fn start(&mut self, code: u16, blink_opt: i32, op_ms: i32) -> bool {
        let ms = if op_ms > 0 { op_ms } else if blink_opt != 0 { blink_opt } else { 500 };
        let ticks = (ms / 50).max(2) as u32;
        match self.items.iter_mut().find(|i| i.0 == code) {
            Some(i) => {
                i.1 = ticks;
                false
            }
            None => {
                self.items.push((code, ticks, 0));
                true
            }
        }
    }
    pub fn stop(&mut self, code: u16) -> bool {
        let n = self.items.len();
        self.items.retain(|i| i.0 != code);
        n != self.items.len()
    }
    pub fn is_blinking(&self, code: u16) -> bool {
        self.items.iter().any(|i| i.0 == code)
    }
    /// One 50 ms tick: the codes whose light changes, with on / off.
    pub fn tick(&mut self) -> Vec<(u16, bool)> {
        let mut out = vec![];
        for (code, cycle, t) in self.items.iter_mut() {
            let half = (*cycle / 2).max(1);
            if *t == 0 {
                out.push((*code, true));
            } else if *t == half {
                out.push((*code, false));
            }
            *t = (*t + 1) % (*cycle).max(2);
        }
        out
    }
}

// ---------------------------------------------------------------------- soft takeover

/// The tempo slider's position as a tempo change in percent: the upper half of the 14-bit
/// travel is the range over 8191 steps, the lower half over 8192 (`UiPlayer::deviceTempo`).
pub fn tempo_slider_percent(raw14: i32, range_pct: f64) -> f64 {
    let off = raw14.clamp(0, 16383) - 8192;
    if off > 0 {
        off as f64 * range_pct / 8191.0
    } else {
        off as f64 * range_pct / 8192.0
    }
}

/// Tempo slider soft takeover (`UiPlayer::deviceUpdateTakeOverStatus` 0x102030ca8). States:
/// 0 armed, 1 slider below the deck's tempo, 2 the same while the deck follows a sync master,
/// 3 slider above, 4 the same while following, 5 matched (the slider sets the tempo).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TempoTakeover {
    pub state: u8,
    /// Where the hardware slider stands (percent), for the on-screen ghost marker.
    pub slider: Option<f64>,
}

impl TempoTakeover {
    /// Armed again: on sync changes, when the deck is switched, when the tempo was changed
    /// on screen.
    pub fn arm(&mut self) {
        self.state = 0;
    }

    /// The slider moved to `slider` percent while the deck is at `deck` percent. Returns the
    /// tempo to set, when the slider has the deck.
    pub fn feed(&mut self, slider: f64, deck: f64, loaded: bool, follower: bool) -> Option<f64> {
        const NEAR: f64 = 0.1;
        self.slider = Some(slider);
        if self.state == 0 {
            if !loaded {
                // Nothing to pick up: the slider sets the tempo of the empty deck.
                return Some(slider);
            }
            self.state = if (slider - deck).abs() <= NEAR {
                5
            } else if slider < deck {
                if follower { 2 } else { 1 }
            } else if follower {
                4
            } else {
                3
            };
        }
        match self.state {
            1 | 2 if !follower && slider >= deck - NEAR => self.state = 5,
            3 | 4 if !follower && slider <= deck + NEAR => self.state = 5,
            1 | 2 => self.state = if follower { 2 } else { 1 },
            3 | 4 => self.state = if follower { 4 } else { 3 },
            _ => {}
        }
        (self.state == 5 && !follower).then_some(slider)
    }

    /// The direction LEDs: (TempoDown, TempoUp) - which way to move the slider.
    pub fn leds(&self) -> (bool, bool) {
        (matches!(self.state, 3 | 4), matches!(self.state, 1 | 2))
    }
}

/// Pick-up of an on-screen knob or fader (`CustomSlider::setTakeOverStatus` 0x1015efbbc):
/// once started, the hardware has the control again when it comes within 2 % of the range of
/// the software value, or crosses it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pickup {
    pub waiting: bool,
    last: Option<f32>,
}

impl Pickup {
    pub fn start(&mut self) {
        self.waiting = true;
        self.last = None;
    }
    /// The hardware is at `hw` (0..1), the software at `sw`: does the hardware set the value?
    pub fn feed(&mut self, hw: f32, sw: f32) -> bool {
        if self.waiting {
            let crossed = self.last.map(|l| (l - sw) * (hw - sw) <= 0.0).unwrap_or(false);
            if (hw - sw).abs() < 0.02 || crossed {
                self.waiting = false;
            }
        }
        self.last = Some(hw);
        !self.waiting
    }
}

// ---------------------------------------------------------------------- jog wheel

/// Jog counts per platter rotation by device, from rekordbox's map constructors (the devices
/// whose constant was read; others use the processor's default of 2048).
pub fn jog_counts(device: &str) -> f64 {
    let d = device.to_ascii_uppercase();
    let has = |names: &[&str]| names.iter().any(|n| d.contains(n));
    if has(&["DDJ-200", "DDJ-FLX2", "DDJ-WEGO3", "DDJ-WEGO4"]) {
        420.0
    } else if has(&["DDJ-WEGO"]) {
        450.0
    } else if has(&["XDJ-R1"]) {
        594.0
    } else if has(&["DDJ-400", "DDJ-FLX4", "DDJ-SB", "DDJ-SR", "OMNIS-DUO", "XDJ-AN"]) {
        720.0
    } else if has(&["XDJ-AERO"]) {
        1200.0
    } else if has(&["DDJ-ERGO"]) {
        2300.0
    } else if has(&["DDJ-SZ"]) {
        3240.0
    } else if has(&["DDJ-FLX6", "DDJ-GRV6", "XDJ-RX3", "XDJ-XZ"]) {
        6480.0
    } else if has(&["DDJ-REV7"]) {
        10080.0
    } else if has(&["XDJ-RX", "XDJ-RR"]) {
        13920.0
    } else {
        2048.0
    }
}

/// What the jog wheel asks of the deck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JogOut {
    Nothing,
    /// Pitch bend as a fraction of the tempo (0.03 = +3 %); 0.0 ends it.
    Bend(f64),
    /// The platter has the record: play at this speed (1.0 = normal, negative = backwards).
    Scratch(f64),
    /// The platter let go and stands or runs slower than the record: back to normal play.
    Release,
    /// The platter let go while spinning: the record runs on at this speed (back spin).
    Spin(f64),
}

/// One deck's jog wheel (`DjController::ScratchProcessor`, ctor 0x101b4e828).
#[derive(Clone, Debug)]
pub struct Jog {
    pub cpr: f64,
    pub bend_ratio: f64,
    pub rpm: f64,
    pub vinyl: bool,
    pub spin_over: u32,
    /// 0 idle, 1 scratch, 2 spin, 4 bend.
    pub state: u8,
    touched: bool,
    last_t: Option<f64>,
    last_speed: f64,
    slow_releases: u32,
    /// Counts of the last 60 ms (time, delta), for the bend.
    window: VecDeque<(f64, i32)>,
}

impl Jog {
    pub fn new(cpr: f64) -> Jog {
        Jog { cpr, bend_ratio: 1.5, rpm: 100.0 / 3.0, vinyl: true, spin_over: 3, state: 0, touched: false, last_t: None, last_speed: 0.0, slow_releases: 0, window: VecDeque::new() }
    }

    /// Counts per millisecond to speed: 1.0 when the platter turns at its nominal rpm.
    pub fn k(&self) -> f64 {
        1000.0 / (self.cpr * self.rpm / 60.0)
    }

    /// The bend for a platter speed: nothing up to half speed, then `speed x ratio / 0.5`
    /// percent, at most +42 %.
    pub fn bend_for(&self, speed: f64) -> f64 {
        if speed.abs() > 0.5 {
            (speed * self.bend_ratio / (1.0 - 0.5)).min(42.0) * 0.01
        } else {
            0.0
        }
    }

    /// The platter top was touched or let go. `playing`: the deck plays.
    pub fn touch(&mut self, on: bool, playing: bool) -> JogOut {
        self.touched = on;
        if on {
            self.last_t = None;
            self.window.clear();
            // Vinyl on: the hand has the record. Vinyl off: a bend while the deck plays.
            self.state = if self.vinyl || !playing { 1 } else { 4 };
            return if self.state == 1 { JogOut::Scratch(0.0) } else { JogOut::Nothing };
        }
        match self.state {
            1 => {
                let slow = self.last_speed >= 0.0 && self.last_speed < 1.0;
                if slow {
                    self.slow_releases += 1;
                }
                if slow || self.slow_releases >= self.spin_over {
                    self.slow_releases = 0;
                    self.state = 0;
                    JogOut::Release
                } else {
                    self.state = 2;
                    JogOut::Spin(self.last_speed)
                }
            }
            4 => {
                self.state = 0;
                JogOut::Bend(0.0)
            }
            _ => JogOut::Nothing,
        }
    }

    /// Counts from the platter top (`scratch`) or the outer ring at `t_ms`.
    pub fn rotate(&mut self, delta: i32, t_ms: f64, scratch_row: bool) -> JogOut {
        // The time since the last message, 56 ms for the first one and at most.
        let dt = match self.last_t {
            Some(l) if t_ms > l => (t_ms - l).min(56.0),
            Some(_) => 1.0,
            None => 56.0,
        };
        self.last_t = Some(t_ms);
        if scratch_row && self.state == 1 {
            let speed = self.k() * delta as f64 / dt;
            if speed.abs() > 25.0 {
                // "illegal scratch speed": dropped.
                return JogOut::Nothing;
            }
            self.last_speed = if speed.abs() < 1.19e-7 { 0.0 } else { speed };
            return JogOut::Scratch(self.last_speed);
        }
        // A bend: the counts of the last 60 ms.
        while self.window.front().map(|(t, _)| t_ms - *t > 60.0).unwrap_or(false) {
            self.window.pop_front();
        }
        self.window.push_back((t_ms, delta));
        let sum: i32 = self.window.iter().map(|x| x.1).sum();
        let speed = self.k() * sum as f64 / 60.0;
        if self.state == 0 {
            self.state = 4;
        }
        JogOut::Bend(self.bend_for(speed))
    }

    /// Nothing came for a while (`now_ms`): a bend ends 60 ms after its last count, and a
    /// held platter that stopped moving holds the record still.
    pub fn idle(&mut self, now_ms: f64) -> JogOut {
        let Some(l) = self.last_t else { return JogOut::Nothing };
        if now_ms - l <= 60.0 {
            return JogOut::Nothing;
        }
        match self.state {
            4 if !self.window.is_empty() || !self.touched => {
                self.window.clear();
                self.last_t = None;
                if !self.touched {
                    self.state = 0;
                }
                JogOut::Bend(0.0)
            }
            1 if self.last_speed != 0.0 => {
                self.last_speed = 0.0;
                JogOut::Scratch(0.0)
            }
            _ => JogOut::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(rows: &[&str]) -> MidiMap {
        let mut m = MidiMap::new("Test Device");
        for r in rows {
            m.add_row(&Row::parse(r).unwrap(), false, false);
        }
        m
    }

    #[test]
    fn button_and_pad_rows() {
        let mut m = map(&["PlayPause,PlayPause,Button,900B,0,1,,,900B,0,1,,,Fast;Priority=50,", "Sync,Sync,Button,9058,0,1,,,,,,,,,", "PAD1_HotCue,PAD1,Pad,9000,7,9,,,9000,7,9,,,Fast,"]);
        let e = m.handle(&[0x90, 0x0B, 0x7F], 0.0).unwrap();
        assert_eq!((e.deck, e.val, e.fast), (1, Val::On(true, 127), true));
        assert_eq!(m.handle(&[0x91, 0x0B, 0x00], 0.0).unwrap().val, Val::On(false, 0));
        assert_eq!(m.handle(&[0x91, 0x0B, 0x00], 0.0).unwrap().deck, 2);
        // Without Fast the row is not handled on the worker.
        assert!(!m.handle(&[0x90, 0x58, 0x7F], 0.0).unwrap().fast);
        // A real Note Off releases (recordbox's own addition).
        assert_eq!(m.handle(&[0x80, 0x0B, 0x40], 0.0).unwrap().val, Val::On(false, 0));
        // Pads sit on channels 8 and 10 for decks 1 and 2.
        let p = m.handle(&[0x97, 0x00, 0x7F], 0.0).unwrap();
        assert_eq!((p.deck, p.val), (1, Val::On(true, 127)));
        assert_eq!(m.handle(&[0x99, 0x00, 0x00], 0.0).unwrap().deck, 2);
        // Unmapped codes, short messages and SysEx find nothing.
        assert!(m.handle(&[0x90, 0x0C, 0x7F], 0.0).is_none());
        assert!(m.handle(&[0x90, 0x0B], 0.0).is_none());
        assert!(m.handle(&[0xF0, 0x00, 0x40, 0x05, 0xF7], 0.0).is_none());
        // One code, one function: a second row for a code already taken is not registered.
        let mut m = map(&["A,A,Button,9001,,,,,,,,,,,", "B,B,Button,9001,,,,,,,,,,,"]);
        assert_eq!(m.rejected, 1);
        assert_eq!(m.input_at(0x9001).unwrap().name, "A");
        assert!(m.handle(&[0x90, 0x01, 1], 0.0).is_some());
    }

    #[test]
    fn pressure_pads_of_the_rz_and_rx_families() {
        let mut m = MidiMap::new("DDJ-FLX10");
        assert_eq!(m.family, PadFamily::Rx);
        m.add_row(&Row::parse("PAD1_HotCue,PAD1,Pad,9000,7,,,,9000,7,,,,Fast,").unwrap(), false, false);
        assert_eq!(m.handle(&[0x97, 0x00, 127], 0.0).unwrap().val, Val::On(true, 127));
        assert_eq!(m.handle(&[0x97, 0x00, 60], 0.0).unwrap().val, Val::Velocity(60));
        assert_eq!(m.handle(&[0x97, 0x00, 0], 0.0).unwrap().val, Val::On(false, 0));
        assert_eq!(pad_family("DDJ-RZX"), PadFamily::Rz);
        assert_eq!(pad_family("PIONEER DDJ-SX2"), PadFamily::Sx2);
        assert_eq!(pad_family("DDJ-SZ"), PadFamily::Sz);
        assert_eq!(pad_family("DDJ-FLX4"), PadFamily::Sx);
        assert_eq!(pad_family("Launchpad Mini"), PadFamily::Sx);
    }

    #[test]
    fn led_values_per_family_and_for_button_rows() {
        // RZ: 127 becomes 121 and 57 becomes 64.
        assert_eq!([pad_led(PadFamily::Rz, 127), pad_led(PadFamily::Rz, 57), pad_led(PadFamily::Rz, 30)], [121, 64, 30]);
        // RX: the codes 31..41.
        let rx: Vec<u8> = (30..=42).map(|v| pad_led(PadFamily::Rx, v)).collect();
        assert_eq!(rx, vec![30, 32, 34, 35, 36, 36, 37, 38, 39, 40, 42, 42, 42]);
        // SZ: values above 64 are off. SX2: the same, with the RX remap. SX: on or off.
        assert_eq!([pad_led(PadFamily::Sz, 64), pad_led(PadFamily::Sz, 65)], [64, 0]);
        assert_eq!([pad_led(PadFamily::Sx2, 31), pad_led(PadFamily::Sx2, 127)], [32, 0]);
        assert_eq!([pad_led(PadFamily::Sx, 127), pad_led(PadFamily::Sx, 5), pad_led(PadFamily::Sx, 63), pad_led(PadFamily::Sx, 0), pad_led(PadFamily::Sx, 100)], [127, 127, 0, 0, 0]);
        // A Button row: 127 / 0, or its Max / Min ("Button(for Pad)").
        let plain = Opts::default();
        assert_eq!([button_led(127, &plain), button_led(0, &plain), button_led(1, &plain), button_led(63, &plain)], [127, 0, 127, 0]);
        let pad = Opts { min: 3, max: 45, ..Default::default() };
        assert_eq!([button_led(127, &pad), button_led(0, &pad)], [45, 3]);
        let m = map(&["PlayPause,PlayPause,Button,900B,0,1,,,900B,0,1,,,,"]);
        let out = m.outputs.iter().find(|o| o.deck == 2).unwrap();
        assert_eq!(m.led_message(out, 127), [0x91, 0x0B, 127]);
        assert_eq!(m.led_message(out, 0), [0x91, 0x0B, 0]);
        // The jog ring: offset + steps x angle / 360.
        let ring = Opts { min: 1, max: 72, ..Default::default() };
        assert_eq!([jog_ring(0, &ring), jog_ring(180, &ring), jog_ring(359, &ring), jog_ring(360, &ring), jog_ring(-1, &ring)], [1, 37, 72, 0, 0]);
    }

    #[test]
    fn seven_bit_and_value_scaling() {
        // A KnobSlider without Min / Max spans 0..16383 with 64 at the centre, 8192.
        let mut m = map(&["Gain,Trim,KnobSlider,B004,0,1,,,,,,,,,", "X,X,KnobSlider,B005,,,,,,,,,,Max=127,", "NeedleSearch,Needle,Value,B003,0,1,,,,,,,,Max=16383,", "V,V,Value,B006,,,,,,,,,,,", "W,W,Value,B007,,,,,,,,,,Min=1;Max=72,", "R,R,RawValue,B008,,,,,,,,,,,"]);
        let v = |m: &mut MidiMap, cc: u8, b: u8| match m.handle(&[0xB0, cc, b], 0.0).unwrap().val {
            Val::Abs { v, .. } => v,
            Val::Raw(b) => b as i32,
            x => panic!("{x:?}"),
        };
        assert_eq!([v(&mut m, 4, 0), v(&mut m, 4, 1), v(&mut m, 4, 63), v(&mut m, 4, 64), v(&mut m, 4, 65), v(&mut m, 4, 127)], [0, 128, 8064, 8192, 8322, 16383]);
        assert_eq!(m.handle(&[0xB0, 4, 64], 0.0).unwrap().val.unit(), Some(0.5));
        assert_eq!(m.handle(&[0xB0, 4, 127], 0.0).unwrap().val.unit(), Some(1.0));
        assert_eq!(m.handle(&[0xB0, 4, 0], 0.0).unwrap().val.unit(), Some(0.0));
        // With Max=127 the byte passes.
        for b in [0u8, 1, 63, 64, 65, 100, 127] {
            assert_eq!(v(&mut m, 5, b), b as i32);
            assert_eq!(v(&mut m, 6, b), b as i32, "a Value row is 0..127 by default");
        }
        // The needle strip: Max=16383.
        assert_eq!([v(&mut m, 3, 0), v(&mut m, 3, 64), v(&mut m, 3, 127)], [0, 8192, 16383]);
        // Min=1, Max=72: half span 36.
        assert_eq!([v(&mut m, 7, 0), v(&mut m, 7, 32), v(&mut m, 7, 64), v(&mut m, 7, 127)], [1, 19, 37, 72]);
        assert_eq!(v(&mut m, 8, 99), 99);
    }

    #[test]
    fn fourteen_bit_pairs() {
        let mut m = map(&["TempoSlider,Tempo,KnobSliderHiRes,B000,0,1,,,,,,,,Fast,", "CrossFader,CrossFader,KnobSliderHiRes,B61F,,,,,,,,,,Fast,"]);
        // The MSB alone gives nothing; the LSB completes the value.
        assert!(m.handle(&[0xB0, 0x00, 0x40], 0.0).is_none());
        let e = m.handle(&[0xB0, 0x20, 0x00], 0.0).unwrap();
        assert_eq!((e.deck, e.val), (1, Val::Abs { v: 8192, lo: 0, mid: 8192, hi: 16383 }));
        assert!(m.handle(&[0xB1, 0x00, 0x7F], 0.0).is_none());
        assert_eq!(m.handle(&[0xB1, 0x20, 0x7F], 0.0).unwrap().val.unit(), Some(1.0));
        // LSBs without a new MSB are or-ed into the value kept (as rekordbox does).
        m.handle(&[0xB6, 0x1F, 0x01], 0.0);
        assert_eq!(m.handle(&[0xB6, 0x3F, 0x01], 0.0).unwrap().val, Val::Abs { v: 129, lo: 0, mid: 8192, hi: 16383 });
        assert_eq!(m.handle(&[0xB6, 0x3F, 0x02], 0.0).unwrap().val, Val::Abs { v: 131, lo: 0, mid: 8192, hi: 16383 });
        // The row is refused when either of its two codes is taken.
        let m = map(&["A,A,Button,B020,,,,,,,,,,,", "T,T,KnobSliderHiRes,B000,,,,,,,,,,,"]);
        assert_eq!(m.rejected, 1);
        assert!(m.input_at(0xB000).is_none());
    }

    #[test]
    fn relative_encoders() {
        // First stage: both encodings.
        assert_eq!([rotary_steps(1), rotary_steps(30), rotary_steps(127), rotary_steps(98), rotary_steps(65), rotary_steps(94), rotary_steps(63), rotary_steps(34)], [1, 30, -1, -30, 1, 30, -1, -30]);
        assert_eq!([rotary_steps(0), rotary_steps(31), rotary_steps(64), rotary_steps(97)], [0, 31, 64, 97]);
        // Acceleration, base 2.22: 1 -> 1, 2 -> 2, 3 -> 4, 4 -> 10, 5 -> 24, 6 -> 30 (the cap).
        let acc: Vec<i32> = (1..=6).map(|n| rotary_accel(n, -1)).collect();
        assert_eq!(acc, vec![1, 2, 4, 10, 24, 30]);
        let back: Vec<i32> = (1..=6).map(|n| rotary_accel(-n, -1)).collect();
        assert_eq!(back, vec![-1, -2, -4, -10, -24, -29]);
        // Value=130: base 1.3. Value=0: no acceleration.
        assert_eq!([rotary_accel(3, 130), rotary_accel(10, 130), rotary_accel(-3, 130)], [1, 10, -1]);
        assert_eq!([rotary_accel(5, 0), rotary_accel(-5, 0)], [5, -5]);
        let mut m = map(&["Browse,Browse,Rotary,B640,,,,,,,,,,,", "K,K,RotaryAsKnob,B641,,,,,,,,,,Value=2048,", "L,L,RotaryAsKnob,B642,,,,,,,,,,,"]);
        assert_eq!(m.handle(&[0xB6, 0x40, 1], 0.0).unwrap().val, Val::Rel(1));
        assert_eq!(m.handle(&[0xB6, 0x40, 0x7F], 0.0).unwrap().val, Val::Rel(-1));
        assert_eq!(m.handle(&[0xB6, 0x40, 3], 0.0).unwrap().val, Val::Rel(4));
        // RotaryAsKnob: Value is the range and the knob starts in the middle.
        let abs = |m: &mut MidiMap, cc: u8, b: u8| match m.handle(&[0xB6, cc, b], 0.0).unwrap().val {
            Val::Abs { v, .. } => v,
            x => panic!("{x:?}"),
        };
        assert_eq!(abs(&mut m, 0x41, 0), 8191);
        assert_eq!(abs(&mut m, 0x41, 1), (1025.0 * 16383.0 / 2048.0) as i32);
        assert_eq!(abs(&mut m, 0x41, 127), 8191);
        for _ in 0..40 {
            abs(&mut m, 0x41, 63);
        }
        assert_eq!(abs(&mut m, 0x41, 63), 16383);
        // Default range 127, start 64.
        assert_eq!(abs(&mut m, 0x42, 0), (64.0 * 16383.0 / 127.0) as i32);
        for _ in 0..3 {
            abs(&mut m, 0x42, 64);
        }
        assert_eq!(abs(&mut m, 0x42, 64), 0);
    }

    #[test]
    fn jog_rows() {
        let mut m = map(&["JogScratch,JogScratch,JogRotate,B022,0,1,,,,,,,,RO,", "JogSearch,JogSearch,Difference,B029,0,1,,,,,,,,RO,", "JogTouch,JogTouch,JogTouch,9036,0,1,,,,,,,,RO,"]);
        let e = m.handle(&[0xB0, 0x22, 0x41], 12.5).unwrap();
        assert_eq!((e.val, e.fast), (Val::Jog { delta: 1, t_ms: 12.5 }, true));
        assert_eq!(m.handle(&[0xB1, 0x22, 0x3E], 0.0).unwrap().val, Val::Jog { delta: -2, t_ms: 0.0 });
        assert_eq!(m.handle(&[0xB0, 0x29, 0x45], 3.0).unwrap().val, Val::Jog { delta: 5, t_ms: 3.0 });
        // Touch: exactly 127 is on.
        assert_eq!(m.handle(&[0x90, 0x36, 0x7F], 0.0).unwrap().val, Val::On(true, 127));
        assert_eq!(m.handle(&[0x90, 0x36, 0x40], 0.0).unwrap().val, Val::On(false, 0x40));
        // The DDJ-REV7's absolute platter: the shortest way round, with its own timestamp.
        let mut m = map(&["JogScratch,Jog,JogRotateAbsPos,B022,0,,,,,,,,,RO,", "JogTimestamp,JogTimestamp,JogTimestamp,E0,0,,,,,,,,,RO,"]);
        assert!(m.handle(&[0xB0, 0x22, 120], 0.0).is_none());
        assert!(m.handle(&[0xE0, 0x10, 0x01], 0.0).is_none());
        assert_eq!(m.handle(&[0xB0, 0x22, 2], 0.0).unwrap().val, Val::Jog { delta: 10, t_ms: 144.0 * 0.0004 });
        assert_eq!(m.handle(&[0xB0, 0x22, 126], 0.0).unwrap().val, Val::Jog { delta: -4, t_ms: 144.0 * 0.0004 });
    }

    #[test]
    fn parameters_and_the_users_file_on_top() {
        let factory = b"@file,1,Test Device\r\nMidiOutInterval,MidiOutInterval,Parameter,FFF3,,,,,,,,,,Value=4,\r\nPlayPause,PlayPause,Button,900B,0,1,,,900B,0,1,,,Fast,\r\nCue,Cue,Button,900C,0,1,,,,,,,,Fast,\r\n";
        let m = MidiMap::load("Test Device", Some(factory), None).unwrap();
        assert_eq!(m.out_interval_ms(), 4);
        assert_eq!(MidiMap::new("x").out_interval_ms(), 1);
        // The user's row replaces the factory entry for its code; a NoFunction row switches a
        // factory control off.
        let user = b"@file,1,Test Device\r\nNoFunction,PlayPause,Button,,900B,,,,,,,,,,\r\nPlayPause,PlayPause,Button,,900C,,,,,,,,,Fast;,\r\n";
        let mut m = MidiMap::load("Test Device", Some(factory), Some(user)).unwrap();
        assert_eq!(m.input_at(0x900B).unwrap().name, NO_FUNCTION);
        assert_eq!(m.input_at(0x900C).unwrap().name, "PlayPause");
        assert_eq!(m.input_at(0x910C).unwrap().name, "Cue");
        m.tag_inputs(|n| (n == "PlayPause") as u32);
        assert_eq!(m.handle(&[0x90, 0x0C, 1], 0.0).unwrap().tag, 1);
        assert_eq!(m.handle(&[0x90, 0x0B, 1], 0.0).unwrap().tag, 0);
        // A file of another device, or one that is no mapping file, is refused.
        assert!(MidiMap::load("Other", None, Some(user)).is_err());
        assert!(MidiMap::load("Test Device", None, Some(b"hello")).is_err());
        // Only the user's rows are saved.
        let saved = m.save_text();
        assert_eq!(saved, "@file,1,Test Device\r\nNoFunction,PlayPause,Button,,900B,,,,,,,,,,\r\nPlayPause,PlayPause,Button,,900C,,,,,,,,,Fast;,\r\n");
        // Dual rows know the other deck's code.
        let m = map(&["Load,Load,Button,9046,0,1,2,3,,,,,,Dual,"]);
        let peer = |c: u16| m.live_inputs().find(|i| i.e.code == c).unwrap().peer;
        assert_eq!([peer(0x9046), peer(0x9146), peer(0x9246), peer(0x9346)], [Some(0x9246), Some(0x9346), Some(0x9046), Some(0x9146)]);
    }

    fn row(name: &str, deck: u8, kind: Kind) -> LearnEntry {
        LearnEntry { name: name.into(), deck, kind, min: -1, max: -1, comment: String::new() }
    }

    #[test]
    fn learn_filters_the_lsb_and_the_release() {
        let mut f = LearnFilter::default();
        // A button: the press is learned, its release (velocity 0, or a Note Off) is not.
        assert_eq!(f.feed(&[0x90, 0x0B, 0x7F]), Some(0x900B));
        assert_eq!(f.feed(&[0x90, 0x0B, 0x00]), None);
        assert_eq!(f.feed(&[0x90, 0x0B, 0x7F]), Some(0x900B));
        assert_eq!(f.feed(&[0x80, 0x0B, 0x40]), None);
        // Another note's zero velocity is taken as a message of its own.
        assert_eq!(f.feed(&[0x90, 0x0C, 0x00]), Some(0x900C));
        // A 14-bit pair: CC n, then CC n + 32 is passed over.
        assert_eq!(f.feed(&[0xB0, 0x00, 0x40]), Some(0xB000));
        assert_eq!(f.feed(&[0xB0, 0x20, 0x11]), None);
        assert_eq!(f.feed(&[0xB0, 0x00, 0x41]), Some(0xB000));
        // A CC that is not 32 above the last one is learned.
        assert_eq!(f.feed(&[0xB0, 0x21, 0x11]), Some(0xB021));
        // Two-byte messages and SysEx are not shown to Learn.
        assert_eq!(f.feed(&[0xC0, 0x05]), None);
        assert_eq!(f.feed(&[0xF0, 0x7E, 0xF7]), None);
    }

    #[test]
    fn learn_commit_rules() {
        let learnable = |n: &str| ["PlayPause", "Cue", "Sync", "TempoSlider", "TempoUp"].contains(&n);
        let mut m = map(&["PlayPause,PlayPause,Button,900B,0,1,,,900B,0,1,,,Fast,", "Shift,Shift,Button,903F,0,1,,,,,,,,RO,", "JogTouch,JogTouch,JogTouch,9036,0,1,,,,,,,,RO,", "MasterLevel,VU,Indicator,,,,,,B202,,,,,RO,", "LoadedIndicator,Loaded,Button,9060,,,,,,,,,,,"]);
        let cue = row("Cue", 1, Kind::Button);
        // A code of another learnable function: refused, the entry is reported.
        match m.change_input(&cue, None, Some(0x900B), &learnable) {
            Err(Reject::Taken(e)) => assert_eq!((e.name.as_str(), e.deck), ("PlayPause", 1)),
            x => panic!("{x:?}"),
        }
        // A read-only code and a code of a function Learn does not offer: refused too.
        assert!(matches!(m.change_input(&cue, None, Some(0x9036), &learnable), Err(Reject::Taken(e)) if e.opts.ro));
        assert!(matches!(m.change_input(&cue, None, Some(0x9060), &learnable), Err(Reject::Taken(e)) if e.name == "LoadedIndicator"));
        // Shift codes are put back without a word.
        assert_eq!(m.change_input(&cue, None, Some(0x903F), &learnable), Err(Reject::Revert));
        // A code whose output side is read-only cannot be taken either.
        assert!(matches!(m.change_input(&cue, None, Some(0xB202), &learnable), Err(Reject::Taken(e)) if e.name == "MasterLevel"));
        // A free code: learned with rekordbox's options for the function, marked as the user's.
        assert_eq!(m.change_input(&cue, None, Some(0x9010), &learnable), Ok(()));
        let e = m.input_at(0x9010).unwrap().clone();
        assert_eq!((e.name.as_str(), e.deck, e.user, e.opts.fast, e.opts.blink, e.opts.priority), ("Cue", 1, true, true, 500, 50));
        // Moving it: the old code stays as NoFunction, and a NoFunction code may be taken.
        assert_eq!(m.change_input(&cue, Some(0x9010), Some(0x9011), &learnable), Ok(()));
        assert_eq!(m.input_at(0x9010).unwrap().name, NO_FUNCTION);
        assert!(m.input_at(0x9010).unwrap().user);
        assert_eq!(m.change_input(&row("Sync", 1, Kind::Button), None, Some(0x9010), &learnable), Ok(()));
        assert_eq!(m.input_at(0x9010).unwrap().name, "Sync");
        // Re-learning a factory row moves it and switches its factory code off.
        let pp = row("PlayPause", 1, Kind::Button);
        assert_eq!(m.change_input(&pp, Some(0x900B), Some(0x9020), &learnable), Ok(()));
        assert_eq!(m.input_at(0x900B).unwrap().name, NO_FUNCTION);
        assert_eq!(m.input_at(0x9020).unwrap().name, "PlayPause");
        // An Indicator row takes no MIDI IN.
        assert_eq!(m.change_input(&row("TempoUp", 1, Kind::Indicator), None, Some(0x9030), &learnable), Err(Reject::Revert));
        // MIDI OUT: only for rows that can light; the same duplicate rules on the outputs.
        assert_eq!(m.change_output(&row("TempoSlider", 1, Kind::KnobSlider), None, Some(0x9031), &learnable), Err(Reject::Revert));
        assert!(matches!(m.change_output(&cue, None, Some(0x900B), &learnable), Err(Reject::Taken(e)) if e.name == "PlayPause"));
        assert!(matches!(m.change_output(&cue, None, Some(0xB202), &learnable), Err(Reject::Taken(e)) if e.name == "MasterLevel"));
        assert_eq!(m.change_output(&cue, None, Some(0x9011), &learnable), Ok(()));
        assert_eq!(m.outputs_at(0x9011)[0].name, "Cue");
        assert_eq!(m.change_output(&row("TempoUp", 1, Kind::Indicator), None, Some(0x9033), &learnable), Ok(()));
        // A 14-bit row needs both of its codes free.
        let tempo = row("TempoSlider", 1, Kind::KnobSliderHiRes);
        assert_eq!(m.change_input(&tempo, None, Some(0xB000), &learnable), Ok(()));
        assert_eq!(m.input_at(0xB020).unwrap().name, "TempoSlider");
        assert!(m.input_at(0xB000).unwrap().opts.fast);
        // Deck, type and comment are rewritten in place; switching 14-bit to 7-bit frees the LSB.
        let mut t7 = row("TempoSlider", 2, Kind::KnobSlider);
        t7.comment = "pitch".into();
        m.rewrite(&t7, Some(0xB000), None);
        let e = m.input_at(0xB000).unwrap();
        assert_eq!((e.deck, e.kind, e.comment.as_str(), e.user), (2, Kind::KnobSlider, "pitch", true));
        assert!(m.input_at(0xB020).is_none());
        // DELETE leaves NoFunction behind.
        m.delete(Some(0x9011), Some(0x9011));
        assert_eq!(m.input_at(0x9011).unwrap().name, NO_FUNCTION);
        assert_eq!(m.outputs_at(0x9011)[0].name, NO_FUNCTION);
        // What is saved: only what the user changed, in rekordbox's layout.
        let text = m.save_text();
        assert!(text.starts_with("@file,1,Test Device\r\n"));
        assert!(text.contains("PlayPause,PlayPause,Button,,9020,,,,,,,,,Fast;Blink=1000;Priority=50;,\r\n"), "{text}");
        assert!(text.contains("TempoSlider,TempoSlider,KnobSlider,,,B000,,,,,,,,Fast;,pitch\r\n"), "{text}");
        assert!(!text.contains("Shift"), "{text}");
        // And it loads back on top of the factory rows to the same mapping.
        let again = MidiMap::load("Test Device", None, Some(text.as_bytes())).unwrap();
        assert_eq!(again.input_at(0x9020).unwrap().name, "PlayPause");
        assert_eq!(again.input_at(0x900B).unwrap().name, NO_FUNCTION);
    }

    #[test]
    fn learned_rows_get_rekordboxs_default_options() {
        let o = learn_opts("PlayPause");
        assert_eq!((o.fast, o.blink, o.priority), (true, 1000, 50));
        assert_eq!(learn_opts("NeedleSearch").max, 16383);
        assert_eq!(learn_opts("Sync").blink, 600);
        assert_eq!(learn_opts("JumpToTrackStart"), Opts { priority: 50, ..Default::default() });
        for n in ["TempoSlider", "CrossFader", "EQLow", "PAD3_HotCue", "PAD16_Sampler+Shift", "LoopIn", "ReTrigger"] {
            assert!(learn_opts(n).fast, "{n}");
        }
        for n in ["Browse", "Master", "Load", "PADX_HotCue", "PAD17_HotCue"] {
            assert_eq!(learn_opts(n), Opts::default(), "{n}");
        }
        // The XDJ-RX's rotaries.
        assert_eq!(row("Browse", 0, Kind::Rotary).entry(0xB640, "PIONEER XDJ-RX").opts.value, 130);
        assert_eq!(row("Browse", 0, Kind::Rotary).entry(0xB640, "Other").opts.value, -1);
    }

    #[test]
    fn the_output_queue_orders_by_priority_and_keeps_the_newest_value() {
        let mut q = OutQueue::default();
        q.push([0x90, 1, 127], 0);
        q.push([0x90, 2, 127], 50);
        q.push([0x90, 3, 127], 0);
        q.push([0x90, 4, 127], 300);
        q.push([0x90, 5, 127], 50);
        // A newer value for a queued code replaces it (and goes to the back of its priority).
        q.push([0x90, 1, 0], 0);
        assert_eq!(q.len(), 5);
        let order: Vec<[u8; 3]> = std::iter::from_fn(|| q.pop()).collect();
        assert_eq!(order, vec![[0x90, 4, 127], [0x90, 2, 127], [0x90, 5, 127], [0x90, 3, 127], [0x90, 1, 0]]);
    }

    #[test]
    fn blink_cycles_in_50_ms_ticks() {
        let mut b = Blinker::default();
        assert!(b.start(0x900B, 1000, 0));
        assert!(!b.start(0x900B, 1000, 0));
        // 1000 ms: 20 ticks, lit for 10.
        let mut on_at = vec![];
        let mut off_at = vec![];
        for t in 0..40 {
            for (_, on) in b.tick() {
                if on { on_at.push(t) } else { off_at.push(t) }
            }
        }
        assert_eq!((on_at, off_at), (vec![0, 20], vec![10, 30]));
        // No Blink value: 500 ms. An interval given with the operation wins.
        let mut b = Blinker::default();
        b.start(1, 0, 0);
        b.start(2, 250, 0);
        b.start(3, 600, 200);
        let mut cycles = [0; 4];
        for _ in 0..60 {
            for (c, on) in b.tick() {
                if on {
                    cycles[c as usize] += 1;
                }
            }
        }
        assert_eq!(cycles, [0, 6, 12, 15]);
        assert!(b.stop(2) && !b.stop(2) && b.is_blinking(1));
    }

    #[test]
    fn tempo_slider_percent_and_soft_takeover() {
        // The two halves: 8191 steps up, 8192 down.
        assert_eq!(tempo_slider_percent(8192, 10.0), 0.0);
        assert_eq!(tempo_slider_percent(16383, 10.0), 10.0);
        assert_eq!(tempo_slider_percent(0, 10.0), -10.0);
        assert!((tempo_slider_percent(8193, 16.0) - 16.0 / 8191.0).abs() < 1e-12);
        assert!((tempo_slider_percent(8191, 16.0) + 16.0 / 8192.0).abs() < 1e-12);
        // Slider above the deck's tempo: it has to come down to within 0.1 % first.
        let mut t = TempoTakeover::default();
        assert_eq!(t.feed(4.0, 0.0, true, false), None);
        assert_eq!((t.state, t.leds()), (3, (true, false)));
        assert_eq!(t.feed(2.0, 0.0, true, false), None);
        assert_eq!(t.feed(0.1, 0.0, true, false), Some(0.1));
        assert_eq!((t.state, t.leds()), (5, (false, false)));
        assert_eq!(t.feed(-3.0, 0.1, true, false), Some(-3.0));
        // Slider below: it has to come up.
        t.arm();
        assert_eq!(t.feed(-5.0, 2.0, true, false), None);
        assert_eq!((t.state, t.leds()), (1, (false, true)));
        assert_eq!(t.feed(1.8, 2.0, true, false), None);
        assert_eq!(t.feed(1.95, 2.0, true, false), Some(1.95));
        // Armed and within 0.1 %: matched at once.
        t.arm();
        assert_eq!(t.feed(0.05, 0.0, true, false), Some(0.05));
        // A sync follower's slider does nothing; it is picked up once sync is off.
        t.arm();
        assert_eq!(t.feed(3.0, 1.0, true, true), None);
        assert_eq!(t.state, 4);
        assert_eq!(t.feed(1.0, 1.0, true, true), None);
        assert_eq!(t.feed(1.0, 1.0, true, false), Some(1.0));
        // An empty deck takes the slider as it is.
        t.arm();
        assert_eq!(t.feed(7.0, 0.0, false, false), Some(7.0));
        // On-screen pick-up: within 2 % of the range, or crossing the software value.
        let mut p = Pickup::default();
        assert!(p.feed(0.9, 0.2));
        p.start();
        assert!(!p.feed(0.9, 0.5));
        assert!(!p.feed(0.6, 0.5));
        assert!(p.feed(0.515, 0.5));
        p.start();
        assert!(!p.feed(0.9, 0.5));
        assert!(p.feed(0.3, 0.5), "crossing the value picks it up");
        assert!(p.feed(0.0, 0.5));
    }

    #[test]
    fn jog_wheel_maths() {
        // DDJ-FLX4: 720 counts a turn at 33 1/3 rpm = 400 counts a second.
        let mut j = Jog::new(jog_counts("DDJ-FLX4"));
        assert_eq!(j.cpr, 720.0);
        assert!((j.k() - 2.5).abs() < 1e-12);
        assert_eq!([jog_counts("DDJ-FLX2"), jog_counts("PIONEER DDJ-SX2"), jog_counts("DDJ-REV7"), jog_counts("XDJ-RX3"), jog_counts("Pioneer DJ XDJ-RX2"), jog_counts("PIONEER DDJ-WeGO2"), jog_counts("Launchpad")], [420.0, 2048.0, 10080.0, 6480.0, 13920.0, 450.0, 2048.0]);
        // The bend law: nothing up to half speed, 3 % at platter speed, capped at +42 %.
        assert_eq!(j.bend_for(0.5), 0.0);
        assert!((j.bend_for(1.0) - 0.03).abs() < 1e-12);
        assert!((j.bend_for(-1.0) + 0.03).abs() < 1e-12);
        assert_eq!(j.bend_for(20.0), 0.42);
        assert!((j.bend_for(-20.0) + 0.6).abs() < 1e-12, "the cap is upwards only");
        // Outer ring at platter speed: 4 counts every 10 ms -> 24 counts in the 60 ms window.
        let mut out = JogOut::Nothing;
        for i in 0..12 {
            out = j.rotate(4, i as f64 * 10.0, false);
        }
        match out {
            JogOut::Bend(b) => assert!((b - 0.03 * 28.0 / 24.0).abs() < 1e-9, "{b}"),
            x => panic!("{x:?}"),
        }
        // 60 ms without counts ends the bend.
        assert_eq!(j.idle(150.0), JogOut::Nothing);
        assert_eq!(j.idle(175.0), JogOut::Bend(0.0));
        assert_eq!(j.state, 0);
        // Vinyl on: a touch takes the record; counts are the record's speed.
        assert_eq!(j.touch(true, true), JogOut::Scratch(0.0));
        assert_eq!(j.rotate(4, 1000.0, true), JogOut::Scratch(2.5 * 4.0 / 56.0));
        assert_eq!(j.rotate(4, 1010.0, true), JogOut::Scratch(1.0));
        assert_eq!(j.rotate(-8, 1020.0, true), JogOut::Scratch(-2.0));
        // Speeds over 25 are dropped.
        assert_eq!(j.rotate(120, 1021.0, true), JogOut::Nothing);
        // A held platter that stops holds the record.
        assert_eq!(j.idle(1100.0), JogOut::Scratch(0.0));
        // Let go while standing: back to normal play.
        assert_eq!(j.touch(false, true), JogOut::Release);
        // Let go while spinning backwards: the record runs on (a back spin).
        j.touch(true, true);
        j.rotate(-40, 2000.0, true);
        j.rotate(-40, 2010.0, true);
        assert_eq!(j.touch(false, true), JogOut::Spin(-10.0));
        assert_eq!(j.state, 2);
        // Vinyl off on a playing deck: the platter top bends; on a stopped deck it scratches.
        let mut j = Jog::new(720.0);
        j.vinyl = false;
        assert_eq!(j.touch(true, true), JogOut::Nothing);
        assert!(matches!(j.rotate(4, 0.0, true), JogOut::Bend(_)));
        assert_eq!(j.touch(false, true), JogOut::Bend(0.0));
        assert_eq!(j.touch(true, false), JogOut::Scratch(0.0));
        // 45 rpm: the same counts are a slower platter.
        let mut j = Jog::new(720.0);
        j.rpm = 45.0;
        assert!((j.k() - 1000.0 / 540.0).abs() < 1e-12);
    }
}
