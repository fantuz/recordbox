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

## Library work, file writes and performance

- **Desktop and USB.** Desktop import uses a private database/WAL snapshot. Write-back previews the proposed edits, checks for concurrent changes and creates a recovery copy. USB export keeps supported classic and OneLibrary catalogs linked to their shared ANLZ files. Duplicate groups based on artist and title can be reviewed before the scoped cleanup step. [Desktop sync and field coverage](LIBRARY-SYNC.md) · [USB compatibility](USB-COMPATIBILITY.md) · [Collection cleanup](COLLECTION-CLEANUP.md)
- **EDIT and recordings.** The EDIT renderer writes the score in small blocks to a private file beside the destination. A cancelled job or filename collision preserves the pre-existing recording. WAV, AIFF and FLAC share this output path, as does the intermediate WAV for macOS AAC. The already decoded source still occupies memory. Recording saves also protect against collisions, sampler imports validate archive contents, and playlist artwork replacement can roll back if saving fails.
- **Day-to-day performance.** The fork retains cached waveform geometry, configurable playback redraw, Linux idle handling, and parallel independent passes for eligible batch or single-track analysis. EDIT palette preview respects the same playback frame-rate setting. The fork’s published callback and redraw benchmarks describe those specific components. [Benchmark method and results](PLAYBACK-PERFORMANCE.md)

Upstream’s later phrase and effects changes at `753649a` are a separate integration. They change phrase inference during USB export, increase the memory needed for some analyses, and include a math call that needs a portable implementation. [Additional upstream changes](https://github.com/adamsardo/recordbox/compare/74ba5e9...753649a)
