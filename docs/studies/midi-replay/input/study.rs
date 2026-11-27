//! Test driver shared verbatim by the two executables. All application modules are copied by git object ID.
use crate::midi_csv::CsvFile;
use crate::midi_map::{Event, MidiMap, Val};
use std::hint::black_box;
use std::time::Instant;

const CSV: &[u8] = include_bytes!("DDJ-GRV6-two-deck-check.csv");
const DEVICE: &str = "DDJ-GRV6";
const CHANNEL_FADER: u32 = 3;
const JOG: u32 = 5;

#[derive(Clone, Copy)]
struct Packet {
    bytes: [u8; 3],
    time: f64,
}

fn map() -> MidiMap {
    let csv = CsvFile::parse(CSV).unwrap();
    assert_eq!(csv.device, DEVICE);
    assert_eq!(csv.rows().count(), 6);
    let mut map = MidiMap::load(DEVICE, None, Some(CSV)).unwrap();
    assert_eq!(map.rejected, 0);
    map.tag_inputs(|name| match name {
        "PlayPause" => 1,
        "Cue" => 2,
        "ChannelFader" => CHANNEL_FADER,
        "PAD1_HotCue" => 4,
        "JogPitchBend" => JOG,
        other => panic!("Unexpected mapping row {other}"),
    });
    map
}

#[derive(Copy, Clone)]
struct Expected {
    tag: u32,
    deck: u8,
    value: Val,
    fast: bool,
}
type Tape = Vec<(Packet, Option<Expected>)>;

fn push(t: &mut Tape, time: f64, bytes: [u8; 3], tag: u32, deck: u8, v: Val) {
    t.push((
        Packet { bytes, time },
        Some(Expected {
            tag,
            deck,
            value: v,
            fast: true,
        }),
    ));
}

// Two seconds at 1-ms ticks, overlapping deck movements and 14-bit fader pairs.
// Swing alternates the jog direction in phrases. Down/up controls are 25 ms apart.
// This is a synthetic workload using documented bytes, not recorded human traffic.
fn trace() -> Tape {
    let mut t = vec![];
    for tick in 0..2000 {
        let ms = f64::from(tick);
        for ch in 0..2u8 {
            if (tick + i32::from(ch)) % 3 == 0 {
                let cc = if (tick / 60 + i32::from(ch)) % 2 == 0 {
                    0x21
                } else {
                    0x23
                };
                let step = if (tick / 125 + i32::from(ch)) % 2 == 0 {
                    1
                } else {
                    -1
                };
                push(
                    &mut t,
                    ms,
                    [0xb0 | ch, cc, (64 + step) as u8],
                    JOG,
                    ch + 1,
                    Val::Jog {
                        delta: step,
                        t_ms: ms,
                    },
                );
            }
            if tick % 200 == 0 || tick % 200 == 25 {
                let is_down = tick % 200 == 0;
                let velocity = if is_down { 127 } else { 0 };
                let (status, note, tag) = match (tick / 200 + i32::from(ch)) % 3 {
                    0 => (0x90 | ch, 0x0b, 1),
                    1 => (0x90 | ch, 0x0c, 2),
                    _ => (0x97 + 2 * ch, 0, 4),
                };
                push(
                    &mut t,
                    ms,
                    [status, note, velocity],
                    tag,
                    ch + 1,
                    Val::On(is_down, velocity),
                );
            }
        }
        if tick % 10 == 0 {
            // Send on both decks before completing either pair. Both the fine and coarse bytes vary.
            let value = |ch: u8| (((tick / 10) * 257 + 8191 * i32::from(ch)) % 16384) as u16;
            for ch in 0..2u8 {
                t.push((
                    Packet {
                        bytes: [0xb0 | ch, 0x13, (value(ch) >> 7) as u8],
                        time: ms,
                    },
                    None,
                ));
            }
            for ch in (0..2u8).rev() {
                let raw = value(ch);
                push(
                    &mut t,
                    ms,
                    [0xb0 | ch, 0x33, (raw & 127) as u8],
                    CHANNEL_FADER,
                    ch + 1,
                    Val::Abs {
                        v: i32::from(raw),
                        lo: 0,
                        mid: 8192,
                        hi: 16383,
                    },
                );
            }
        }
    }
    t
}

fn verify(tape: &Tape, map: &mut MidiMap) -> usize {
    let mut count = 0;
    for (idx, (p, expected)) in tape.iter().enumerate() {
        let actual = map.handle(&p.bytes, p.time);
        match (actual, expected) {
            (None, None) => {}
            (Some(event), Some(want)) => {
                assert_eq!(
                    (event.tag, event.deck, event.val, event.fast),
                    (want.tag, want.deck, want.value, want.fast),
                    "packet {idx}: {:02x?}",
                    p.bytes
                );
                count += 1;
            }
            _ => panic!("packet {idx}: presence mismatch {:02x?}", p.bytes),
        }
    }
    // The three shared button rows return Note On with velocity 0 or 127 for both decks.
    for (name, note, status_a, status_b) in [
        ("Cue", 0x0c, 0x90, 0x91),
        ("PlayPause", 0x0b, 0x90, 0x91),
        ("PAD1_HotCue", 0, 0x97, 0x99),
    ] {
        for (deck, status) in [(1, status_a), (2, status_b)] {
            let output = map
                .outputs
                .iter()
                .find(|out| out.name == name && out.deck == deck)
                .unwrap();
            for v in [0, 127] {
                assert_eq!(map.led_message(output, v), [status, note, v]);
            }
        }
    }
    count
}

#[inline(never)]
fn bulk(map: &mut MidiMap, tape: &[Packet], loops: usize) -> usize {
    let mut delivered = 0;
    for _ in 0..loops {
        for p in tape {
            delivered += usize::from(black_box(map.handle(black_box(&p.bytes), p.time)).is_some());
        }
    }
    delivered
}

fn ns_quantiles(mut v: Vec<u64>) -> [u64; 3] {
    v.sort_unstable();
    [50, 95, 99].map(|pct| v[pct * v.len() / 100])
}

pub fn run() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "verify".into());
    let packet_budget = args
        .next()
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(if mode == "latency" { 60_000 } else { 4_000_000 });
    let oracle = trace();
    let packets: Vec<Packet> = oracle.iter().map(|(packet, _)| *packet).collect();
    let mut m = map();
    let mapped = verify(&oracle, &mut m);
    assert!(mapped < packets.len());
    let loops = packet_budget.div_ceil(packets.len()).max(1);
    let total = loops * packets.len();
    if mode == "verify" {
        println!(
            "{{\"packets\":{},\"events\":{},\"led_cases\":12,\"oracle\":\"passed\"}}",
            packets.len(),
            mapped
        );
        return;
    }
    assert_eq!(bulk(&mut m, &packets, 50), 50 * mapped);
    match mode.as_str() {
        "bulk" => {
            let start = Instant::now();
            let delivered = bulk(&mut m, &packets, loops);
            let elapsed = start.elapsed();
            assert_eq!(delivered, loops * mapped);
            assert_eq!(verify(&oracle, &mut m), mapped);
            println!("{{\"raw_packets\":{total},\"events\":{delivered},\"wall_ns\":{},\"ns_per_packet\":{:.4}}}",
                elapsed.as_nanos(), elapsed.as_nanos() as f64/total as f64);
        }
        "latency" => {
            let mut samples = Vec::with_capacity(total);
            let mut empty = Vec::with_capacity(total);
            let mut delivered = 0;
            for _ in 0..loops {
                for p in &packets {
                    let i = Instant::now();
                    let event = black_box(m.handle(black_box(&p.bytes), p.time));
                    let ns = i.elapsed().as_nanos();
                    delivered += usize::from(event.is_some());
                    samples.push(ns as u64);
                    let i = Instant::now();
                    black_box(None::<Event>);
                    empty.push(i.elapsed().as_nanos() as u64);
                }
            }
            assert_eq!(delivered, loops * mapped);
            assert_eq!(verify(&oracle, &mut m), mapped);
            println!("{{\"timed_calls\":{total},\"map_ns_p50_p95_p99\":{:?},\"blank_ns_p50_p95_p99\":{:?}}}",
                ns_quantiles(samples),ns_quantiles(empty));
        }
        _ => panic!("usage: fork|upstream verify|bulk|latency [packet-count]"),
    }
}
