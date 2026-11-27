#![allow(dead_code, unused_imports)]
mod midi_csv;
mod midi_map;
use midi_csv::{Entry, Kind, Opts};
use midi_map::{Blinker, MidiMap, OutQueue};
use parking_lot::{Condvar, Mutex};
use std::{collections::HashMap, hint::black_box, sync::atomic::{AtomicU64, Ordering::Relaxed}, time::{Duration, Instant}};
include!("types.rs");
include!("production.rs");

const COMMON: [&str; 7] = ["JogPlayPauseIndicator", "JogPlayPauseIndicatorShort", "ChannelLevel",
    "LoadedIndicator", "GetLoopActive", "SlipStateForFlashing", "PlayingState"];

#[derive(Clone, Copy)]
struct Case {
    decks: u8,
    names: &'static [usize],
    fanout: usize,
    total: usize,
    tagged: bool,
    hires: bool,
}
impl Case {
    fn get(name: &str) -> Self {
        let (decks, names, fanout, total, tagged, hires): (_, &[usize], _, _, _, _) = match name {
            "unmapped" => (2, &[], 0, 64, false, false),
            "small" => (2, &[0, 2, 3], 1, 16, false, false),
            "broad" => (4, &[0, 1, 2, 3, 4, 5, 6], 1, 128, false, false),
            "fanout" => (4, &[0, 1, 2, 3, 4, 5, 6], 3, 256, false, false),
            "tagged" => (2, &[], 4, 64, true, false),
            // Encoding and queue parity probe only: the full app can calculate different
            // high-resolution ring/meter values, so this scenario is excluded from timing.
            "hires" => (2, &[0, 2], 1, 8, false, true),
            _ => panic!("unknown fixture {name}"),
        };
        Self { decks, names, fanout, total, tagged, hires }
    }

    fn value(&self, phase: bool, id: usize) -> u16 {
        if self.hires { return if phase { 0x2134 } else { 0x0342 }; }
        match id {
            0..=2 => if phase { 91 } else { 31 },
            _ => if phase { 127 } else { 0 },
        }
    }
    fn tick(&self, device: &Device, phase: bool) {
        for deck in 1..=self.decks {
            if self.tagged {
                device.set_led(1, deck, if phase { Led::On } else { Led::Off });
            } else {
                for (id, name) in COMMON.iter().enumerate() {
                    device.named_value(name, deck, self.value(phase, id));
                }
            }
        }
    }
}

// Third-party mappings and hardware are unnecessary. The setup emits valid channel
// statuses with unique addresses, leaving the lower 7 bits valid even for 14-bit pairs.
fn make(case: Case) -> (Device, [Vec<[u8; 3]>; 2]) {
    let mut map = MidiMap::new("Synthetic output fixture");
    let mut expected: [Vec<(i32, [u8; 3])>; 2] = Default::default();
    let mut row = |name: &str, deck: u8, kind: Kind, code: [u8; 2], priority: i32| {
        map.outputs.push(Entry {
            name: name.into(), function: name.into(), kind, deck,
            code: u16::from_be_bytes(code), opts: Opts { priority, ..Default::default() },
            comment: String::new(), user: false,
        });
    };
    let mut i = 0;
    for deck in 1..=case.decks {
        let ids = if case.tagged { &[0][..] } else { case.names };
        for &id in ids {
            for _ in 0..case.fanout {
                let status = (if case.tagged { 0x90 } else { 0xb0 }) + (i / 64) as u8;
                let data = ((i % 64) / 16 * 32 + i % 16) as u8;
                let kind = if case.tagged { Kind::Button } else if case.hires { Kind::IndicatorHiRes } else { Kind::Indicator };
                let priority = (i % 3) as i32 - 1;
                row(if case.tagged { "PlayPause" } else { COMMON[id] }, deck, kind, [status, data], priority);
                for (phase, exp) in expected.iter_mut().enumerate() {
                    let value = if case.tagged { if phase == 1 { 127 } else { 0 } } else { case.value(phase == 1, id) };
                    if case.hires {
                        exp.push((priority, [status, data, (value >> 7) as u8]));
                        exp.push((priority, [status, data + 16, (value & 127) as u8]));
                    } else {
                        exp.push((priority, [status, data, value as u8]));
                    }
                }
                i += 1;
            }
        }
    }
    // Actual, customary tagged lights. They are in the map, but aren't the target of the
    // named queries. Use channels 8–15 to avoid all tested status/data collisions.
    for f in i..case.total {
        row(if f % 2 == 0 || case.tagged { "Cue" } else { "PlayPause" },
            (f % case.decks as usize) as u8 + 1, Kind::Button,
            [0x98 + (f / 128) as u8, (f % 128) as u8], 0);
    }
    // Spread matching rows through the map. Production preserves matching order by
    // deck/name (or tag), so keep the relative order of each set of fan-out rows.
    let fillers = map.outputs.split_off(i);
    let named = std::mem::take(&mut map.outputs);
    let mut other = fillers.into_iter();
    let base = case.total.saturating_sub(i) / i.max(1);
    for entry in named {
        map.outputs.extend(other.by_ref().take(base));
        map.outputs.push(entry);
    }
    map.outputs.extend(other);
    assert_eq!(map.outputs.len(), case.total);
    let expected = expected.map(|mut values| {
        values.sort_by_key(|(p, _)| std::cmp::Reverse(*p));
        values.into_iter().map(|(_, bytes)| bytes).collect()
    });
    (Device::new(map), expected)
}

fn drain(device: &Device, mut accept: impl FnMut([u8; 3])) -> usize {
    let mut out = device.out.lock();
    let mut count = 0;
    while let Some(bytes) = out.queue.pop() {
        accept(bytes);
        count += 1;
    }
    count
}
fn verify(case: Case) {
    let (device, expected) = make(case);
    let collect = || {
        let mut v = vec![];
        drain(&device, |b| v.push(b));
        v
    };
    case.tick(&device, false);
    assert_eq!(collect(), expected[0], "first frame");
    case.tick(&device, false);
    assert!(collect().is_empty(), "suppress unchanged state");
    case.tick(&device, true);
    assert_eq!(collect(), expected[1], "next state");
    case.tick(&device, false);
    case.tick(&device, true);
    assert_eq!(collect(), expected[1], "replace pending values and preserve priority and order");
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["oracle"] {
        println!("case,phase,position,status,data1,data2");
        for name in ["unmapped", "small", "broad", "fanout", "tagged", "hires"] {
            let case = Case::get(name);
            verify(case);
            let (dev, expected) = make(case);
            for phase in 0..2 {
                case.tick(&dev, phase != 0);
                let mut observed = vec![];
                drain(&dev, |b| observed.push(b));
                assert_eq!(observed, expected[phase]);
                for (i, [s, a, b]) in observed.iter().enumerate() {
                    println!("{name},{phase},{i},{s:02x},{a:02x},{b:02x}");
                }
            }
        }
        return;
    }
    if args.as_slice() == ["verify"] {
        for name in ["unmapped", "small", "broad", "fanout", "tagged", "hires"] {
            verify(Case::get(name));
        }
        println!("6 packet-oracle scenarios passed");
        return;
    }
    assert!(args.len() == 3, "expected CASE stable|changed TICKS or verify");
    assert!(args[0] != "hires", "different higher-level semantics; correctness check only");
    let case = Case::get(&args[0]);
    let changed = match args[1].as_str() { "stable" => false, "changed" => true, _ => panic!("unknown pattern") };
    let ticks: usize = args[2].parse().unwrap();
    assert!(ticks > 0);
    let (device, expected) = make(case);
    let tick = |phase| {
        case.tick(&device, phase);
        drain(&device, |bytes| { black_box(bytes); })
    };
    for n in 0..128 { tick(n % 2 == 0 && changed); }
    // Last warm-up frame is phase zero. Alternating starts at phase one.
    let start = Instant::now();
    let mut packets = 0usize;
    for n in 0..ticks { packets += tick(black_box(changed && n % 2 == 0)); }
    let ns = start.elapsed().as_nanos();
    assert_eq!(packets, if changed { ticks * expected[0].len() } else { 0 });
    println!("{},{},{},{},{}", args[0], args[1], ticks, packets, ns);
}
