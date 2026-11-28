# Track Suggestion and Related Tracks timing

Run from the repository root with Python 3 and the project's Rust toolchain. The runner builds a small release binary in a temporary directory. Include an upstream Git ref to check that its ranking functions agree with the earlier fork.

```sh
python3 docs/studies/track-suggestion/run.py --upstream fcffc8d --output ranking.csv
```

The study compares the working tree, the submitted multi-provider change at `4f4f71e`, the previous single-provider implementation at `801255f`, and the earlier implementation at `d656c95`. On a clone containing only the fork, omit `--upstream`. All fork revisions used here need to be available locally. The runner reads their scoring, candidate traversal and sorting code from Git. A shared in-memory fixture supplies catalog access, cached evidence and waveform previews. It uses the project's Unicode folding and key-distance functions.

The default fixture has 40,000 synthetic tracks. It includes collaborations, accented text, release and label gaps, missing files, user tags and three registered pairs. Six tracks in seven have a 400-column preview. Source 31 has a preview and source 35 has none. The three evidence cases contain no external data, one source on 90% of tracks, and three sources on 90% of tracks. The multi-source case includes repeated genres from different catalogs. A separate untagged-library pass keeps filenames and analyzed tempo, key and previews, with empty artist, album, genre, label, year and My Tags. Fixture construction finishes before timing starts.

Each trial measures a whole-collection Mix rank with the waveform-summary cache cleared, an explicit second Mix rank, Association, and Related Tracks with the default key and tempo filters. The runner rotates revision order across trials. It prints the median of nine trials and checks the ordered IDs and score bits for equal workloads across the fork revisions. It also compares the Related Tracks eligible IDs across implementations independently of their ordering. The earlier algorithm uses different weights and recomputes waveform summaries on every query. An open panel with no changes reuses its ranked rows, so its frame time differs from the warm recomputation measured here.

The measurements cover local ranking. They exclude network lookups, database writes, audio decoding, and window rendering.

## Recorded comparison

These are median wall times in milliseconds for 40,000 tracks and nine interleaved trials per revision, using Rust 1.100.0 on a Linux Intel Xeon Platinum 8573C cloud host. The benchmark was pinned to one logical CPU while a native application build shared the host's four-CPU quota. The working tree's ranking changes are in `43e1a9b`.

| Library and operation | Earlier fork / upstream | Before PR #44 | Initial PR #44 | Updated PR #44 |
| --- | ---: | ---: | ---: | ---: |
| Local metadata, cold Mix | 658.4 | 159.0 | 146.5 | 121.1 |
| Local metadata, warm Mix | 646.0 | 97.1 | 103.8 | 78.6 |
| Local metadata, Association | 650.3 | 80.9 | 75.5 | 40.7 |
| Local metadata, default Related | 2.10 | 7.10 | 6.48 | 4.96 |
| One provider, warm Mix | — | 168.2 | 164.3 | 131.1 |
| Three providers, cold Mix | — | — | 234.7 | 229.1 |
| Three providers, warm Mix | — | — | 219.1 | 194.0 |
| Three providers, default Related | — | — | 10.39 | 10.85 |
| Source has no preview, cold Mix | 513.6 | 122.2 | 116.3 | 48.5 |
| Filename-only library, warm Mix | 183.2 | 19.9 | 15.5 | 12.4 |

The earlier Related query was faster on the populated local catalog. It orders eligible tracks by an integer score derived from key and tempo. The updated query applies the same default eligibility criteria and uses additional evidence to order the survivors. The fixture confirmed identical eligible IDs, with a different ordering in the earlier implementation. The fork revisions produced identical ordered IDs and score bits for all common workloads.

The change removes repeated string scans and allocations for simple artist/title credits. When the source has no usable waveform, candidate previews can be skipped. An open results panel now coalesces saved evidence arriving within its existing one-second refresh interval. Source, target and criteria changes still refresh immediately. The elapsed times above measure explicit queries.

Dense Related timings were sensitive to machine load. In a second nine-trial pass with less contention, the initial and updated versions measured 5.50 and 4.32 ms, respectively. The two passes do not establish a consistent speed difference for that case. The updated warm Mix was faster with zero, one and three providers in both passes. These timings cannot predict total UI frame time on a DJ's machine.
