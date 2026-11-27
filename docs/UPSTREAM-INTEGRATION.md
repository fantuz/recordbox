# What changes in the upstream integration

The integration brings Adam Sardo’s EDIT timeline, additional Performance layouts, MIDI and keyboard mappings, recorder, sampler and effects into the fork. New beat analysis also uses his port of the AlphaTheta Rekordbox 7.2.19 Normal and Dynamic detector. The fork’s library and device-export tools continue to use its existing database and ANLZ checks. [Upstream beat detector](https://github.com/adamsardo/recordbox/blob/74ba5e9/app/src/analysis/rb_beat1.rs) · [Integration PR](https://github.com/fantuz/recordbox/pull/28)

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

## STEMS by device

| Device or build | Behavior |
| --- | --- |
| Vega via Windows DirectML or experimental Linux Vulkan, Quadro P2000 via CUDA 12, CPU and other devices outside the preview limits | Use completed, cached separations with the existing GPU limits, cancellation and CPU controls. |
| Eligible Apple Silicon Metal or CUDA 12 hardware | In **Preferences → Extensions → STEMS**, enable **Preview finished sections during new separations**. The deck can mute or solo parts in the finished portion. Seeking beyond it plays the original until that area is ready or the job finishes. |
| Apple GPU family 10 or later exposing Apple10 and Metal 4 support | Eligible matrix multiplies can use public Metal tensor operations. The app compiles and numerically checks the kernels on the actual device before use. Set `RECORDBOX_METAL_TENSOR_OPS=0` to use the existing Metal implementation. |

Preview is off by default. Apple Silicon requires 32 GiB of system RAM and 8 GiB available in the driver’s recommended working set. CUDA requires compute capability 8.0 or later, 24 GiB of host RAM, 8 GiB total device memory and 6 GiB free at job start. Preview holds at most 512 MiB of additional audio samples (about 6 minutes 20 seconds). In speed mode, completed sections can be heard during the single model pass. Quality mode makes them available during the fourth specialist pass, keeping one network resident at a time. Only completed results enter the reusable cache. Actual latency and overall separation speed depend on hardware and track length.

### How the later upstream STEMS engine compares

The 0.9.0 integration includes upstream through `3cc12d9`. The next upstream revision, `c5e839e`, uses the same HTDemucs checkpoints and four stereo outputs. Its changes affect **when the DJ can hear a requested section**, **how long the full track takes**, and **when the machine pays the cost of loading the models**. These are separate measurements. [Upstream changes under review](https://github.com/adamsardo/recordbox/compare/3cc12d9...c5e839e)

| Machine | recordbox 0.9.0 | What the later upstream code offers |
| --- | --- | --- |
| Recent Apple Silicon | Candle 0.11.1 with Metal. Eligible Apple10/Metal 4 devices check tensor-kernel results against a reference before enabling them. | Fused operations and revised weight preparation on the upstream Candle 0.9 path. These are candidates for a port and a full-track measurement on supported hardware. Some flash-attention variants appear only in upstream tests. |
| NVIDIA, including Quadro P2000 | Optional CUDA 12 build runs accelerated separation. The P2000 uses completed stems. Newer cards with sufficient headroom can preview during separation. | Upstream selects CPU on Windows and Linux. Its Apple GPU kernels cannot accelerate Pascal. Segment scheduling can be evaluated independently of CUDA. |
| Vega 8 on Windows | DirectML runs the neural network. The CPU handles audio transforms and combines overlapping segments. Shared-memory adapters run the network in 14 passes with smaller attention tiles. A device hang pauses GPU work. | Upstream has no DirectML path. Loading all four quality networks at launch would increase memory pressure before the first track is requested. |
| Vega 8 on Linux | Vulkan accelerates the transformer's linear layers. The CPU handles convolutions, attention scoring and audio transforms. Each offloaded projection uploads weights and reads its output back. | Upstream has no Vulkan path. Measuring transfer, GPU and CPU time separately would tell us whether a bounded cache of weights or buffers is worth implementing. |

In **speed mode**, upstream can move segments around the playhead, hot cues or a requested range ahead of ordinary background work. That can make a late cue usable sooner even if the whole song takes just as long. In **quality mode**, upstream runs all four specialists on each segment before moving to the next one. A well-equipped Mac can therefore hear the first complete four-part section early. The fork runs one specialist across the whole track at a time and begins publishing during the fourth pass. That saves model memory and delays the first preview. The fork publishes immutable blocks from the start and caps extra preview audio at 512 MiB. A useful follow-up would prioritize a requested range within that budget, keep overlap boundaries intact, and cancel a queued request when its track leaves the deck. [Upstream scheduler and specialist order](https://github.com/adamsardo/recordbox/blob/c5e839e/app/src/stems/mod.rs#L597-L655) · [Fork preview storage](../app/src/stems/live.rs)

Upstream can also run inference on silence at startup and preload all four fine-tuned networks. This moves work before the first click and can benefit repeated separations on a roomy Mac. For a DJ browsing a collection on Vega or CPU, it adds work even if they never open STEMS. The upstream cold-start benchmark includes two separations in the figure labelled time to first segment. Shader compilation and weight-packing timings alone cannot establish a whole-track speedup. [Warm-up code](https://github.com/adamsardo/recordbox/blob/c5e839e/app/src/stems/mod.rs#L113-L141) · [Upstream benchmark](https://github.com/adamsardo/recordbox/blob/c5e839e/app/src/stems/bench.rs#L276-L303)

**Models and saved work.** Speed runs one network for all four stems. Quality runs four specialized networks over the track. Both completed results contain four stereo parts and take about 169 MB per four minutes in the fork. The cache distinguishes the model and source audio, so speed and quality results can coexist. Switching the compute backend alone does not invalidate them. [Cache format and reuse](LIBRARY-SYNC.md#how-much-separation-is-saved)

**How to re-evaluate.** Use the same non-silent WAV and keep each machine's driver and power settings fixed. Record idle memory, cold time to the first audible stem, time to a cue near the end, full-track elapsed time, and peak memory. Repeat with a warm engine and measure a saved-cache reload separately. In the fork, `--compare-stems FILE` bypasses the result cache, times the single-checkpoint model against CPU and reports relative RMS differences per stem. It excludes model loading. The 3% tolerance measures agreement with CPU output. Evaluating musical quality also requires listening or isolated reference stems. Measure the four-checkpoint quality mode separately in the deck. [CUDA procedure](CUDA12.md) · [DirectML procedure](DIRECTML.md) · [Vulkan procedure](VULKAN.md)

## Library work, file writes and performance

- **Desktop and USB.** Desktop import uses a private database/WAL snapshot. Write-back previews the proposed edits, checks for concurrent changes and creates a recovery copy. USB export keeps supported classic and OneLibrary catalogs linked to their shared ANLZ files. Duplicate groups based on artist and title can be reviewed before the scoped cleanup step. [Desktop sync and field coverage](LIBRARY-SYNC.md) · [USB compatibility](USB-COMPATIBILITY.md) · [Collection cleanup](COLLECTION-CLEANUP.md)
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

Run `recordbox --midi-coverage "My Controller.midi.csv"` to see which input actions and output indicators recordbox handles in that file. Use `./recordbox` from the Linux portable folder or `.\recordbox.exe` in Windows PowerShell. The command reads one CSV and exits before the GUI or MIDI ports open. Each row identifies its source line, action and direction. A PLAY row can appear in both totals, and a mapping for several decks counts once per direction. A valid CSV can contain unsupported rows. Missing or invalid files produce an error exit.

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

MERGE FX can read factory sounds from your Music folder under `rekordbox/Sampler/MERGE FX`. You can also point `RECORDBOX_MERGE_FX_ARCHIVE` at your own `MERGE FX.zip`. The release contains the effect processors. Factory sample audio comes from your installation.
