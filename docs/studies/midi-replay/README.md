# MIDI input and controller feedback on the fork and upstream

On 27 November 2026, both revisions decoded the same two-deck GRV6 sequence correctly. The fork took 14.34 ns per raw packet and upstream took 40.38 ns. The largest feedback difference appeared on a synthetic controller with many named output rows. Ordinary Play-button feedback had overlapping timings. The measurement used Linux on one Intel Xeon Platinum 8573C core and optimized Rust 1.100.0 binaries. [Fork revision](https://github.com/fantuz/recordbox/tree/9baccd5a09facd9bc4db04222a6d35aa2792c3e8) · [Upstream revision](https://github.com/adamsardo/recordbox/tree/b55b775f91083e04f8732dfa4eae6cd1293866be)

## Input

The synthetic two-second sequence contains 2,173 packets on two decks. It interleaves jog movement with Play, Cue, pads and 14-bit channel faders. Both decoders produced the expected 1,773 events. Each fader's first byte waits for its partner. The oracle checked the tag, deck, value, applicable timestamp and fast-routing flag. Six LED on/off pairs also matched. The input benchmark exercised the preloaded mapper. [Mapping fixture and provenance](../DDJ-GRV6-two-deck-check.csv) · [Input trial data](input/results.json)

| Time per raw packet | fantuz | adamsardo |
| --- | ---: | ---: |
| Bulk median in ns (middle 50% of trials) | 14.34 (14.24–14.59) | 40.38 (39.09–42.07) |
| Individually timed calls, p50 / p95 / p99 in ns | 42 / 49 / 52 | 61 / 67 / 84 |
| Empty clock bracket, p50 / p95 / p99 in ns | 22 / 23 / 24 | 22 / 24 / 24 |

Bulk and individual-call timing each used 12 trials per revision with alternating start order. The individual percentiles include the clock cost. One difference visible in the source is controller identification. The fork determines whether it is a DDJ-RX once when loading the map. Upstream uppercases and searches the device name when handling an incoming packet. Several other changes separate the two commits. At the replay rate of about 1,087 packets/s, the measured decoder difference amounts to 0.028 ms of processing time per second. [Fork map construction](https://github.com/fantuz/recordbox/blob/9baccd5/app/src/midi_map.rs#L307) · [Upstream input handling](https://github.com/adamsardo/recordbox/blob/b55b775/app/src/midi_map.rs#L505)

## Feedback

Each tick visits the seven shared named outputs on every fixture deck, or the tagged Play button for the control case. The test calls the production feedback functions with known values and drains their actual priority queue. It checks exact wire bytes, unchanged-value suppression, pending-value replacement and priority order. Six fixtures also cover 14-bit pairs on the encoding path. Timings use seven-bit rows with the same requested values on both revisions. [Wire packets](output/measured/packets.csv) · [Raw trials](output/measured/raw.csv) · [Paired ratios](output/measured/summary.csv)

| Mapped outputs / total rows | Stable tick, µs (fantuz / adamsardo) | Changing tick, µs (fantuz / adamsardo) |
| --- | ---: | ---: |
| 0 / 64, two decks | 0.28 / 0.41 | — |
| 6 / 16, two decks | 0.61 / 1.08 | 0.74 / 1.19 |
| 28 / 128, four decks | 1.59 / 5.71 | 2.73 / 7.21 |
| 84 / 256, four decks, three addresses per function and deck | 2.15 / 11.66 | 8.78 / 19.38 |
| Tagged Play control, 8 / 64, two decks | 0.068 / 0.068 | 0.774 / 0.761 |

Times are medians of eight paired trials per case with alternating process order. The fork indexes named outputs by function and deck when the mapping changes. Upstream can reject absent names immediately, then scans the output list and clones matching entries for a present name. The fork saves roughly 0.27 ms of host time per second at 60 changing ticks/s in the 28-row fixture. The port worker, OS scheduling, USB, controller firmware and audio buffers sit outside both benchmarks. Measuring pad-to-audio or app-to-lamp latency requires a physical controller. [Fork feedback](https://github.com/fantuz/recordbox/blob/9baccd5/app/src/midi.rs#L1026) · [Upstream feedback](https://github.com/adamsardo/recordbox/blob/b55b775/app/src/midi.rs#L1163)

## Repeat on Linux

From this study branch, fetch the two source revisions. The fixture comes from the history of this branch. Each probe builds the sources from Git objects and checks expected output before timing. Choose one allowed, otherwise idle logical CPU and use the same release toolchain for both binaries. The commands use CPU 1.

```sh
git fetch https://github.com/fantuz/recordbox.git 9baccd5a09facd9bc4db04222a6d35aa2792c3e8
git fetch https://github.com/adamsardo/recordbox.git b55b775f91083e04f8732dfa4eae6cd1293866be
repo=$(git rev-parse --show-toplevel)
cd "$repo/docs/studies/midi-replay/input"
python3 prepare.py "$repo"
CARGO_TARGET_DIR="$PWD/target" cargo build --release --locked --bins
python3 run.py --cpu 1 --blocks 6 --out /tmp/recordbox-midi-input.json

cd ../output
python3 prepare.py "$repo"
python3 run.py --cpu 1 --rounds 8 --sample-ms 40 --out /tmp/recordbox-midi-feedback
```

The feedback runner requires an empty results directory. Input compiles each revision's MIDI CSV and map modules intact. Feedback additionally extracts the pinned device methods from the application and supplies a minimal fixture registry, map and device without a physical port. The registry is used in setup. Packet encoding, caches, locks, queue replacement and dequeue ordering execute the application code. [Toolchain and host](output/measured/environment.txt)
