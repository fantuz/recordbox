# What changes in the upstream integration

The integration brings Adam Sardo’s EDIT timeline, additional Performance layouts, MIDI and keyboard mappings, recorder, sampler and effects into the fork. New beat analysis also uses his port of the AlphaTheta Rekordbox 7.2.19 Normal and Dynamic detector. The fork’s library and device-export tools continue to use its existing database and ANLZ checks. [Upstream beat detector](https://github.com/adamsardo/recordbox/blob/74ba5e9/app/src/analysis/rb_beat1.rs) · [Integration PR](https://github.com/fantuz/recordbox/pull/28)

## Changes through upstream ae3d557

Eight upstream snapshots followed the November 27 comparison at `26a4cca`. This integration is based on fork main after the whole-library memory fix and the Homebrew update. It also includes earlier effect and beat-analyzer components on which the new snapshots depend. [Compared upstream revisions](https://github.com/adamsardo/recordbox/compare/26a4ccaebbe55dd0b2f8eb0b57bec525951208eb...ae3d557ed1551ee86211f7e305921bff14050fb2) · [Fork baseline](https://github.com/fantuz/recordbox/tree/fa8db4f247949884b5d868b7d547f6bedba2d5ee)

The follow-up audit covered 36 upstream commits. It recovered Low- and Mid-mood type-2 phrase analysis, additional mapped controller actions, and USB vocal-level encoding from earlier revisions. Six generated audio fixtures produced byte-identical phrase sections to upstream. Existing desktop and USB phrase data retain priority. The fork retains the virtualized library, native detached browser, and direct MIDI paths for time-sensitive controls. [Phrase revision](https://github.com/adamsardo/recordbox/commit/6604e0a) · [Controller revision](https://github.com/adamsardo/recordbox/commit/4c33514) · [USB protocol revision](https://github.com/adamsardo/recordbox/commit/9dac758)

| Upstream change | What it brings to the fork |
| --- | --- |
| `5d4889c` and `a913c9f` | Model the band-pass, high-pass, low-pass and crush echoes, including their feedback and modulation. Prepare first-use rack buffers outside the audio callback lock. The fork already did this for some effects. |
| `c6da4b0` and `921b571` | Update SPIRAL, HELIX, NOISE, reverb, filtering, stretch and vinyl-brake behavior. MERGE FX smooths discontinuities when clips restart. The master meter samples the signal before the limiter, so it can show peaks that the limiter subsequently reduces. |
| `4891eed` | Add a selectable Light appearance to EXPORT and PERFORMANCE. The fork applies the palette to the detached library as well. EDIT keeps its dark appearance. |
| `8e4f60f` | Add the legacy beat analyzer’s constant-versus-changing-tempo decision to the optional installed-model path. Extend its sample-rate coverage. The fork retains its classic and compatible estimators, optional transient refinement and memory-aware collection queue. |
| `3eae535` | Export vocals and instrumentals to standalone files, add the results to the collection and optionally to the source playlist. Extend contrast coverage across the browser and dialogs. |
| `ae3d557` | Extend the optional model to changing-tempo tracks. Run the downbeat network in sections, repair the joins and estimate local tempo. The fork limits the model’s working memory and falls back to the selected ordinary Dynamic estimator when a section cannot be resolved. |

The MIDI window gains the new palette. The controller protocol change in this integration comes from a separate fork contribution. The experimental OPUS-QUAD profile maps the manufacturer's published messages through the same input, feedback and map-restoration paths as the XDJ-RX3 trial. A physical OPUS-QUAD is still needed to check jog calibration, lamps and audio routing. Its standalone player reads OneLibrary. [OPUS-QUAD setup and hardware checks](studies/OPUS-QUAD.md) · [USB compatibility](USB-COMPATIBILITY.md)

On Linux/X11, the native detached library showed both sync directions and their source and destination paths in Light appearance. Choosing a vocal export in the child brought the main window forward for confirmation. The Light EXPORT and PERFORMANCE headers measured 18.43:1 text contrast on the rendered fixture. [Desktop sync and write-back](LIBRARY-SYNC.md)

### Interface motion from ef454ed

The next upstream commit adds visual feedback for mouse presses, selection, drops, knobs and faders, plus smoother waveform drawing and scrolling to a selected track. Choose **Preferences → Advanced → Others → recordbox → Interface motion** for Full, Reduced or Off. Full also follows the system’s reduced-motion setting on macOS and Windows. [Upstream implementation](https://github.com/adamsardo/recordbox/commit/ef454ed0536aad897bb1c4da1d6215c419de8595)

The fork keeps waveform repainting under the existing playback frame-rate setting. A hardware fader movement takes effect immediately. Waveform clicks use the geometry the user sees, and relative seeks and grid edits use the audio clock. Native wheel and trackpad events keep their original deltas and modifiers in both windows.

A headless egui test showed why each browser pane needs its own animation state. With the same track visible in two panes and selected in just one, upstream requested 120 repaints over 120 idle test frames. Using a separate key per pane brought that to zero after the transition settled. The library decodes visible album covers on a background thread even when motion is Off. It limits the request and result queues to 16 each, uploads at most four thumbnails per viewport pass, and keeps up to 256 list thumbnails. Replacing album art invalidates a pending read of the older file.

### Fullscreen and the detached library

Both implementations draw the library in a native child window using egui’s immediate viewport API. They share application state, so playlist selection and deck actions reach the same library. Upstream deliberately redocks the browser when the main window enters fullscreen. The fork keeps the two windows open and toggles both with F11 from either one. Closing the library returns its navigation and scroll position to the player. Space controls transport outside text fields. Both windows can be resized or maximized independently. [Upstream split-window implementation](https://github.com/adamsardo/recordbox/blob/ef454ed0536aad897bb1c4da1d6215c419de8595/app/src/split.rs)

The paired behavior has an implementation cost. The fork coordinates fullscreen commands and restores geometry after the window manager completes a transition. Its X11 path also handles focus and fullscreen through the window manager. Using a single application state avoids a second catalog, player or MIDI engine in the child.

A three-second X11 sample with a generated track and a clocked null audio sink recorded 59 window updates during docked playback. With the library detached and focused, it recorded 62 updates in the player and 61 in the library. Pressing Space in the library paused playback, after which there was one update in each window. The counts measure visible updates on that Linux test machine. They do not measure GPU utilization or predict frame times on Vega. In a separate empty-deck test, Full and Off each drew once in eight seconds after transient messages cleared.

### What the precision checks establish

With generated 44.1 kHz audio, the ported tempo classifier reported 124.00 BPM and constant tempo on a steady 124 BPM track. On a track stepping from 128 to 86 BPM it reported 128.00 BPM with the changing-tempo flag. Those match the two published reference observations. The 128.00 figure is the detector’s reported track tempo. The flag determines how Auto handles the changing section. The optional model can now convert eligible rates such as 32, 88.2, 96 and 192 kHz. A track that exceeds the 15-minute limit stays on the selected ordinary estimator. Dynamic and Auto can use the new changing-tempo pipeline. Too few beats, unresolved half-beat offsets and some half/double or fractional tempo interpretations still need the ordinary estimator. [Upstream tempo classifier](https://github.com/adamsardo/recordbox/blob/8e4f60f/app/src/analysis/rb_hp_bia2.rs)

The ordinary compatible estimator returned identical BPM and grid anchors on the fork baseline, upstream and the integration for the three synthetic tracks under **Preparing tracks** below. The median timing errors were 0.25–0.29 ms. Those fixtures test steady tempo.

A separate test compared 88 generated beat sequences at 44.1 and 48 kHz with upstream’s window-join repair. Timestamps and beat numbers matched on all 88. A generated 118–126 BPM ramp returned the same 240 beat positions and 122.00014 BPM at the dynamic focus stage. These tests begin with beat candidates and measure the stages downstream of the neural network.

The fork shares intermediate analysis data within the current track. Each track owns that data, which avoids reusing a prior song’s results when a worker receives a buffer at the same address. The grid keeps timestamp changes below one millisecond for formats that can store them. A synthetic 48 kHz test retains beat positions at 100000, 600020, 1100833 and 1600416 microseconds and checks their bar numbers. This measures how accurately the fork stores detected beats. Assessing musical accuracy across recordings requires reference grids and, for the full optional model, locally installed weights.

### Effect selection and playback cost

At 48 kHz, one selection of a preallocated single-mode BPF Echo cleared 68,464,000 bytes of delay buffers in the upstream implementation. The fork marks the old samples unreadable and reuses the allocated buffers. On one Linux Xeon core, selecting the effect took a median 2.744 ms upstream and 0.345 µs with this change. Ten batches of twenty selections were timed for each implementation after warm-up. Forward and reverse output matched sample for sample after a reset, including silence between the old and new input. The port keeps effect-specific kernels in their own modules. Control-thread preparation and the audio callback share an explicit set of prepared buffers. An optional external coefficient table requires an explicitly supplied compatible local file. Portable coefficients are the default.

The processing cost while an effect is playing also changed. These are mean microseconds to process 512 stereo frames at 48 kHz on one pinned Linux Xeon core, averaged over two runs for each revision in alternating order. The comparison is with the fork immediately before this integration.

| Active effect or combination | Previous fork (µs) | This integration (µs) |
| --- | ---: | ---: |
| MT DELAY | 82.7 | 25.1 |
| BPF ECHO | 71.8 | 27.1 |
| LPF ECHO | 74.6 | 28.0 |
| SPIRAL | 36.6 | 26.0 |
| REVERB | 20.1 | 47.4 |
| FLANGER | 11.5 | 38.0 |
| STRETCH | 50.8 | 108.1 |
| REVERB + PHASER + UP ECHO | 70.5 | 122.3 |

The three-effect combination used about 1.15% of a 10,667 µs block interval on that machine. Real audio deadlines also include other decks, mixing, drivers and scheduling. In a separate single-worker test, default analysis of four 30-second inputs took 1.498 s on the previous fork and 1.485 s on this revision. The small difference gives little reason to expect a change in everyday analysis throughput. Neither test ran on Vega.

### Exporting vocals and instrumentals

Use **Track → Export Stems**, the track context menu, or the deck’s STEMS strip. **Vocals** writes the separated vocal. **Instrumental** subtracts the vocal from the original mix, which preserves material the separator did not assign to a part. Each result gets an unused filename and its own recordbox entry. The first export explains the destination and offers playlist placement. Library Protection applies to this write.

- A completed separation can be reused from the deck or the cache for the chosen model. The worker can read the two vocal channels from disk in blocks. It may still decode the original to bounded stereo PCM. The export queue runs one track at a time.
- An uncached track already on a deck can use the existing separation engine and its device limits. For an uncached track elsewhere in the library, load it on a free deck and separate it before export. This keeps a bulk export from replacing a playing deck or launching competing GPU jobs.
- Choose WAV, AIFF or FLAC for lossless output. MP3 uses the bundled encoder. AAC uses the system encoder on macOS. With **Same as the original**, an unavailable AAC encoder produces FLAC with a format notice. For lossy exports, recordbox measures the codec delay on its own decoder when placing the copied cues and grid. Another player can use a different MP3 time origin.
- The application stages the file in the destination folder, then publishes it under an unused name. It imports metadata, cues, grid and playlist placement in a database transaction. If the database import fails after publication, the error gives the path of the finished audio.

The exported files are separate from the reusable four-part STEMS cache. Desktop sync and USB export keep their own review and write rules.

## Preparing tracks

In **Preferences → Analysis**, **Rekordbox-compatible** is the default beat grid estimator. Choose **recordbox classic** for the fork’s earlier estimator. Normal and Dynamic modes support both choices. **High precision: refine beat timing** aligns a detected grid to consistent transients. It is a separate option from the locally installed beat model. The compatible detector retries with classic when it cannot produce a usable grid or the track exceeds its 15-minute limit. The choice applies when analysis runs. A previously prepared collection can be explored as it is.

In a local comparison, we generated three 32-second, 44.1 kHz tracks with known kick times, then fed identical audio to each detector. The numbers are median absolute distances from the generated kick to the nearest grid beat.

| Fixture | Classic | Classic with transient refinement | Rekordbox-compatible, no refinement |
| --- | ---: | ---: | ---: |
| 120 BPM, first kick at 80 ms | 3.23 ms | 0.16 ms | 0.29 ms |
| 128 BPM, first kick at 137 ms | 3.63 ms | 0.05 ms | 0.27 ms |
| 124.5 BPM, offbeat hats, first kick at 137 ms | 3.58 ms | 0.05 ms | 0.25 ms |

All three settings found every kick. These measurements cover constant tempo and clean attacks. Recorded drums, downbeat labels, total analysis time and playback on a physical deck require different tests.

MP3 timing can be tried separately with `recordbox --mp3-timeline rekordbox`. This opens a separate recordbox collection. An unchanged cue timestamp can refer to a different sound when the decoder counts encoder delay differently. With a 44.1 kHz test impulse, switching clocks moved the impulse by 25.06 ms for one FFmpeg-tagged MP3 and 26.12 ms for the LAME-tagged and untagged samples. Starting recordbox normally opens the usual collection. [MP3 trial and cache details](PLAYBACK-PERFORMANCE.md#auditioning-rekordbox-mp3-timing)

### Tempo, key and vocal data in a prepared collection

The collection saves the track-wide BPM returned by Dynamic analysis. A track with a 118 BPM intro and a 126 BPM later section can display 122 BPM in the browser while the deck follows each local tempo. A waveform-only refresh preserves a previously saved collection BPM. USB preparation uses the same whole-track memory limit as library analysis, checks Stop during decoding and between model windows, and reports tracks it could not prepare. An existing device copy can still occupy its place in the playlist. Sync stops before deleting unreferenced audio if preparation fails. [Analysis persistence and USB changes](https://github.com/fantuz/recordbox/commit/361bec9fe164b64b153814341ed3579b52eb44c8)

At 44.1 and 48 kHz, recordbox feeds stereo frames directly to its key detector in one-second blocks. Its standard compatible beat detector uses the existing sinc converter at other rates. The optional beat model uses the reconstructed 29-tap converter on supported input. Upstream also changed key detection to use 16-bit reader samples and two-second blocks. Adopting those changes would require checking key labels against reference recordings across genres and sample rates. The synthetic kick and tempo tests above measure beat timing only. [Upstream reader and key changes](https://github.com/adamsardo/recordbox/commit/9dac7589c5c31eace36d94b40d1abfdf9788b31e)

Vocal detection uses the local graph selected through `RECORDBOX_VOCAL_GRAPH` when configured. The fork keeps existing vocal markers when upgrading. Upstream’s background conversion clears stored vocal levels and fills them again from the installed library or a new analysis. [Upstream vocal conversion](https://github.com/adamsardo/recordbox/commit/75d21a2b2d9f310e7fedc9b8fa4028428d37daeb)

## STEMS by device

| Device or build | Behavior |
| --- | --- |
| Vega via Windows DirectML or experimental Linux Vulkan, Quadro P2000 via CUDA 12, CPU and other devices outside the preview limits | Use completed, cached separations with the existing GPU limits, cancellation and CPU controls. |
| Eligible Apple Silicon Metal or CUDA 12 hardware | In **Preferences → Extensions → STEMS**, enable **Preview finished sections during new separations**. The deck can mute or solo parts in the finished portion. Seeking beyond it plays the original until that area is ready or the job finishes. |
| Apple GPU family 10 or later exposing Apple10 and Metal 4 support | Eligible matrix multiplies can use public Metal tensor operations. The app compiles and numerically checks the kernels on the actual device before use. Set `RECORDBOX_METAL_TENSOR_OPS=0` to use the existing Metal implementation. |

Preview is off by default. Apple Silicon requires 32 GiB of system RAM and 8 GiB available in the driver’s recommended working set. CUDA requires compute capability 8.0 or later, 24 GiB of host RAM, 8 GiB total device memory and 6 GiB free at job start. Preview holds at most 512 MiB of additional audio samples (about 6 minutes 20 seconds). In speed mode, completed sections can be heard during the single model pass. For tracks longer than 7.8 seconds, quality mode can run its four specialists on one section at a time. Once the first section finishes, roughly 5.76 seconds of the track is playable with mute and solo. If the GPU has less free memory, the deck begins publishing during the fourth full-track pass. Only completed results enter the reusable cache. Actual latency and overall separation speed depend on hardware and track length.

### How the later upstream STEMS engine compares

The 0.9.0 integration includes upstream through `3cc12d9`. The next upstream revision, `c5e839e`, uses the same HTDemucs checkpoints and four stereo outputs. Its changes affect **when the DJ can hear a requested section**, **how long the full track takes**, and **when the machine pays the cost of loading the models**. Later MIDI and Beat FX changes were ported individually. The GPU comparison below uses that specific upstream revision. [Upstream changes under review](https://github.com/adamsardo/recordbox/compare/3cc12d9...c5e839e)

| Machine | recordbox 0.9.0 | What the later upstream code offers |
| --- | --- | --- |
| Recent Apple Silicon | Candle 0.11.1 with Metal. Eligible Apple10/Metal 4 devices check tensor-kernel results against a reference before enabling them. | Fused operations and revised weight preparation on the upstream Candle 0.9 path. These are candidates for a port and a full-track measurement on supported hardware. Some flash-attention variants appear only in upstream tests. |
| NVIDIA, including Quadro P2000 | Optional CUDA 12 build runs accelerated separation. The P2000 uses completed stems. Newer cards with sufficient headroom can preview during separation. | Upstream selects CPU on Windows and Linux. Its Apple GPU kernels cannot accelerate Pascal. Segment scheduling can be evaluated independently of CUDA. |
| Vega 8 on Windows | DirectML runs the neural network. The CPU handles audio transforms and combines overlapping segments. Shared-memory adapters run the network in 14 passes with smaller attention tiles. A device hang pauses GPU work. | Upstream has no DirectML path. Loading all four quality networks at launch would increase memory pressure before the first track is requested. |
| Vega 8 on Linux | Vulkan accelerates the transformer's linear layers. The CPU handles convolutions, attention scoring and audio transforms. Each offloaded projection uploads weights and reads its output back. | Upstream has no Vulkan path. Measuring transfer, GPU and CPU time separately would tell us whether a bounded cache of weights or buffers is worth implementing. |

In **speed mode**, upstream can move windows containing the playhead, hot cues or a requested range ahead of ordinary background work. The fork and upstream process **quality mode** in track order. With early quality preview enabled and sufficient GPU memory, a four-minute track needs four network evaluations for the first playable section. The fallback needs 127. Both require 168 for the complete track. A late cue still waits for preceding sections. [Upstream scheduling](https://github.com/adamsardo/recordbox/blob/c5e839e/app/src/stems/mod.rs#L597-L649) · [Fork implementation](../app/src/stems/htdemucs.rs)

There is a concrete reason to defer playhead-first scheduling in the fork. The player currently publishes a continuous prefix of finished audio. Publishing a later window with the same progress counter could mark intervening audio ready before it has been separated. At 44.1 kHz a cue at 2:00 lies in the 21st speed-model window. Prioritizing it could make that cue audible after one window, provided the player can represent disjoint ready ranges. That estimate counts windows. It does not measure end-to-end GPU time. A production port needs seek and cancellation tests plus a comparison of completed caches.

Before admitting four quality networks, recordbox checks the driver's current budget. It reserves the normal preview scratch allowance, 512 MiB per unloaded network, 256 MiB for staging, and space for the full result, normalization weights and immutable preview. It checks again while loading. CPU, Windows DirectML, Linux Vulkan and Pascal use the completed-result path.

On a 12-second synthetic stereo input at 44.1 kHz with real checkpoints, one optimized CPU reference run took 22.48 s for the speed model’s first window and 79.66 s across the four quality specialists’ first windows, excluding model loading. These timings predate the scheduling change and measure CPU inference. The new schedule still needs a physical GPU measurement of time to first audible parts and time to a late cue. [Fixture and timing method](studies/onelibrary-grv6-quality-stems.md#fast-access-to-quality-stems)

Upstream can also run inference on silence at startup and preload all four fine-tuned networks. This moves work before the first click and can benefit repeated separations on a roomy Mac. For a DJ browsing a collection on Vega or CPU, it adds work even if they never open STEMS. The upstream cold-start benchmark includes two separations in the figure labelled time to first segment. Shader compilation and weight-packing timings alone cannot establish a whole-track speedup. [Warm-up code](https://github.com/adamsardo/recordbox/blob/c5e839e/app/src/stems/mod.rs#L113-L141) · [Upstream benchmark](https://github.com/adamsardo/recordbox/blob/c5e839e/app/src/stems/bench.rs#L276-L303)

**Models and saved work.** Speed runs one network for all four stems. Quality runs four specialized networks over the track. Both completed results contain four stereo parts and take about 169 MB per four minutes in the fork. The cache distinguishes the model and source audio, so speed and quality results can coexist. Switching the compute backend alone does not invalidate them. [Cache format and reuse](LIBRARY-SYNC.md#how-much-separation-is-saved)

**How to re-evaluate.** Use the same non-silent WAV and keep each machine's driver and power settings fixed. Record idle memory, cold time to the first audible stem, time to a cue near the end, full-track elapsed time, and peak memory. Repeat with a warm engine and measure a saved-cache reload separately. In the fork, `--compare-stems FILE` bypasses the result cache, times the single-checkpoint model against CPU and reports relative RMS differences per stem. It excludes model loading. The 3% tolerance measures agreement with CPU output. Evaluating musical quality also requires listening or isolated reference stems. Measure the four-checkpoint quality mode separately in the deck. [CUDA procedure](CUDA12.md) · [DirectML procedure](DIRECTML.md) · [Vulkan procedure](VULKAN.md)

## Library work, file writes and performance

- **Desktop and USB.** Desktop import uses a private database/WAL snapshot. Write-back previews the proposed edits, checks for concurrent changes and creates a recovery copy. In the public mixed-drive fixture, changing one classic rating preserved the OneLibrary catalog and shared ANLZ files byte for byte. For OneLibrary playlists, use **Devices → Inspect OneLibrary playlists…** on a closed export, or run `recordbox --inspect-onelibrary "/path/to/USB"`. The optional `rbox==0.3.1` reader opens private copies of schema version 1000. It can compare the order of resolved track paths against a classic playlist. Links pointing to missing tracks are outside that comparison. AlphaTheta Rekordbox handles OneLibrary writes. Duplicate groups based on artist and title can be reviewed before the scoped cleanup step. [Desktop sync and field coverage](LIBRARY-SYNC.md) · [OneLibrary inspector setup and USB compatibility](USB-COMPATIBILITY.md#inspect-onelibrary-playlists) · [Collection cleanup](COLLECTION-CLEANUP.md)
- **EDIT and recordings.** The EDIT renderer writes the score in small blocks to a private file beside the destination. A cancelled job or filename collision preserves the pre-existing recording. WAV, AIFF and FLAC share this output path, as does the intermediate WAV for macOS AAC. The already decoded source still occupies memory. Recording saves also protect against collisions, sampler imports validate archive contents, and playlist artwork replacement can roll back if saving fails.
- **Day-to-day performance.** The fork retains cached waveform geometry, configurable playback redraw, Linux idle handling, and parallel independent passes for eligible batch or single-track analysis. EDIT palette preview respects the same playback frame-rate setting. The fork’s published callback and redraw benchmarks describe those specific components. [Benchmark method and results](PLAYBACK-PERFORMANCE.md)

## Phrase analysis and USB export

An EXT file can hold a `PSSI` section with Rekordbox’s phrase boundaries, mood and fills. For an unchanged grid, deck loading and USB export prefer the existing phrase data. The decoder validates section lengths and handles the version-dependent masking. Saving an unedited section preserves its additional header and phrase fields. Edits invalidate derived fields that described the old phrase layout.

For missing phrases, **Preferences → Analysis → Track Analysis → Phrase estimator** selects the portable Rekordbox-compatible analyzer or **recordbox classic**. The compatible analyzer uses stereo audio and the beat grid to identify sections and fills. Some song structures still require the classic estimator. Both the kind of phrase and the timing can differ between estimators. [Upstream reference implementation](https://github.com/adamsardo/recordbox/blob/753649a/app/src/phrase_rb.rs)

The compatible analyzer processes stereo in blocks, uses a downsampled excerpt for bar alignment, and runs one compatible analysis at a time. Whole-track phrase inference is limited to 15 minutes and 4,096 beats. USB phrase decoding also limits its stereo buffer to 256 MiB. An over-budget track can still export with its existing supported phrases or without inferred phrases. The export summary reports how many tracks received new phrases when some could not be analyzed.

A sequential Linux x86-64 comparison used the same 211-second synthetic stereo signal and fixed 128 BPM grid. The upstream analyzer (with only the Linux math symbol adapted) and recordbox produced identical 200-byte PSSI sections with seven phrases. Peak process memory measured by GNU time fell from 250 MiB to 150 MiB. Process CPU time was 12.16 s upstream and 13.09 s here. This measurement covers one synthetic input with audio already decoded.

## Controllers and effects

A mapped MIDI controller can operate transport, jog wheels, hot cues, loops, the mixer and Pad FX in PERFORMANCE. MIDI Learn and CSV import/export are available in the MIDI window when a device is connected. Compatible output ports also receive LED feedback. Controllers that require a proprietary handshake, HID players and PRO DJ LINK remain outside the supported MIDI path.

For an existing AlphaTheta mapping, connect the controller in PERFORMANCE. A saved recordbox mapping takes precedence. To locate mappings outside the usual installation, set `RECORDBOX_MIDI_USER_DIR` to the directory containing your device’s user CSV, or `RECORDBOX_MIDI_FACTORY_DIR` for factory CSVs. Keep those directories separate from recordbox’s own mappings. Learn and save changes in the recordbox MIDI window.

Opening and closing MIDI Learn preserves the inherited mapping. On Linux, a learned mapping follows the named controller port when ALSA assigns it a new client number. If two connected inputs would use the same mapping identity, recordbox pauses those inputs and lists them in the MIDI tooltip. Disconnecting one lets the remaining input resume. Distinct numbered ports can operate together.

### Check a controller mapping before a set

For the XDJ-RX3, select its main input in the MIDI window and click **Try RX3 profile**. Twenty input rows expand to 39 bindings across the two decks and mixer, including eight Hot Cue pads per deck. Two output rows drive four PLAY/CUE lamps. **Stop RX3 profile** returns to the previous mapping. Software replay checked all 41 input addresses, including both halves of each tempo slider, and the four lamp addresses. Physical MIDI and audio behavior still need a controller check. [Setup, supported controls and hardware check](studies/XDJ-RX3.md)

Mappings with five separate channel meter lights can use `ChannelLevel.1` through `ChannelLevel.5`. They light cumulatively at 10%, 30%, 50%, 70% and 90% of full-scale linear amplitude, using the louder stereo channel after EQ and before the channel fader. The ordinary `ChannelLevel` row sends a continuous meter reading. A segmented `Indicator` sends 0 or 127, and an `IndicatorHiRes` sends 0 or 16383. These thresholds follow the upstream WeGO mapping convention. [Upstream MIDI implementation](https://github.com/adamsardo/recordbox/blob/4c3351486585c4db20528a59747bca87348bc153/app/src/midi.rs#L2081-L2092)

Run `recordbox --midi-coverage "My Controller.midi.csv"` to see which input actions and output indicators recordbox handles in that file. Use `./recordbox` from the Linux portable folder or `.\recordbox.exe` in Windows PowerShell. The command reads one CSV and exits before the GUI or MIDI ports open. Each row identifies its source line, action and direction. A PLAY row can appear in both totals, and a mapping for several decks counts once per direction. A valid CSV can contain unsupported rows. Missing or invalid files produce an error exit.

Fast fader updates stay in sequence until the UI acknowledges them. With 4,096 queued updates acknowledged one at a time, retiring the acknowledged prefix takes at most 8,191 sequence comparisons, down from 8,390,656. A separate two-deck GRV6 replay of 2,173 raw packets on one Xeon core measured median decoder times of 14.34 ns per packet at fork revision `9baccd5` and 40.38 ns at upstream `b55b775`. Both produced the expected 1,773 events. Those timings cover the mapper in memory. USB delivery and audio latency require a hardware measurement. [Input and feedback trials](studies/midi-replay/README.md)

For the DDJ-GRV6, the two-deck study CSV exercises transport, both jog surfaces, a 14-bit channel fader and the first Hot Cue pad. On the controller, press PLAY and then toggle it with the mouse to check the return LED independently. A passing CSV report establishes software support for the specified rows. Port selection, audio response and physical LED reception still need the controller. [Two-deck test and mapping](https://github.com/fantuz/recordbox/pull/29) · [AlphaTheta MIDI message list](https://downloads.support.alphatheta.com/software_info/dj-controllers/DDJ-GRV6/DDJ-GRV6_MIDI_Message_List_E1.pdf)

This follow-up incorporates Adam Sardo’s controller updates for pressure-pad types, jog handling, higher-resolution indicators and additional mapped deck, mixer and effects controls. MIDI Learn preserves device-specific pad types when editing. The output scheduler honors mapped timing for jog indicators. [Upstream MIDI changes](https://github.com/adamsardo/recordbox/commit/4b71b0158d89ab35dafa7d54636c684ba0053318)

### DDJ-GRV6 Groove Circuit

In **PERFORMANCE**, enable **STEMS** and **GROOVE CIRCUIT** under **Preferences → Extensions → STEMS**, then open the Groove Circuit panel in the top bar. With the corresponding rows in your controller mapping, the unshifted controls operate the selected deck.

| Control | Action | Feedback to the controller |
| --- | --- | --- |
| DRUM SWAP 1–4 | Play or stop the slot in the current bank. With Capture armed on that deck, a slot press saves the captured audio as a WAV in the configured capture folder and assigns it to the slot. | Four slot lights follow the deck’s slot state. |
| CAPTURE | Arm or disarm capture. The button alone does not create a file. | The light follows the armed deck. |
| GAIN | Adjust the Drum Swap level for the whole deck. The 14-bit knob covers the full range, with unity near the midpoint. Individual sample trims keep their values. | Input only. |
| DRUM RELEASE lever | Apply the selected release effect while the lever is held. | Input only. |

From the source checkout, run `recordbox --midi-coverage docs/studies/DDJ-GRV6-groove-check.csv` to check these seven inputs and five feedback rows. The file is a diagnostic excerpt. Use your full controller mapping for a set. To check the return path on hardware, arm Capture **with the mouse** and watch the controller light, then disarm it before trying the slot buttons. [Diagnostic CSV](studies/DDJ-GRV6-groove-check.csv) · [AlphaTheta MIDI message list, page 3](https://downloads.support.alphatheta.com/software_info/dj-controllers/DDJ-GRV6/DDJ-GRV6_MIDI_Message_List_E1.pdf#page=3) · [AlphaTheta user manual](https://downloads.support.alphatheta.com/manuals/dj-controllers/DDJ-GRV6/DDJ-GRV6_DRI1927A_manual.pdf) · [Upstream Groove Circuit mapping](https://github.com/adamsardo/recordbox/commit/4c3351486585c4db20528a59747bca87348bc153)

### Effects

This integration brings upstream’s SPACE and two-band GATE COMP processing, Beat FX including SPIRAL and HELIX, Pad FX combination pads and beat steps, and MERGE FX parameter and sample handling. Controller and effect behavior depends on the mapping, audio device and sample rate. An automated virtual-controller exchange exercises the mapping and LED paths on macOS. Physical-controller latency and listening comparisons with the original effects still require hardware testing. [Upstream changes](https://github.com/adamsardo/recordbox/compare/74ba5e9...3cc12d9)

In single-mode **LOW CUT ECHO**, SWING at center leaves both repeats on the selected beat. At either endpoint the repeat on that side arrives after three quarters of the selected delay. A 48 kHz stereo impulse at 120 BPM verified 18,000 versus 24,000 sample frames at full left. In single-mode **ROBOT**, LEVEL/DEPTH at zero is dry, 25% gives an even mix and 50% or higher is fully wet. MODULATION adjusts the pitch and flanger gains. [LOW CUT ECHO and ROBOT changes and tests](https://github.com/fantuz/recordbox/pull/34)

MERGE FX can read factory sounds from your Music folder under `rekordbox/Sampler/MERGE FX`. You can also point `RECORDBOX_MERGE_FX_ARCHIVE` at your own `MERGE FX.zip`. The release contains the effect processors. Factory sample audio comes from your installation.

## Installing the macOS download

Choose the ZIP for your Mac and verify its SHA-256 against `SHA256SUMS` from the same release before opening `Recordbox.app`. The first-launch guide explains Apple’s per-app approval for a trusted download when macOS cannot verify its developer. [recordbox first-launch guide](MACOS-FIRST-LAUNCH.md) · [Apple’s instructions](https://support.apple.com/en-us/102445)
