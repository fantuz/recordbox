# recordbox

<!-- project-badges:start -->
<p align="center">
  <a href="https://github.com/fantuz/recordbox/actions/workflows/build.yml?query=branch%3Amain"><img src="https://github.com/fantuz/recordbox/actions/workflows/build.yml/badge.svg?branch=main" alt="Build and package on main" height="20"></a>
  <a href="https://github.com/fantuz/recordbox/actions/workflows/github-code-scanning/codeql?query=branch%3Amain"><img src="https://github.com/fantuz/recordbox/actions/workflows/github-code-scanning/codeql/badge.svg?branch=main" alt="CodeQL analysis on main" height="20"></a>
  <a href="https://github.com/fantuz/recordbox/releases/latest"><img src="https://img.shields.io/github/v/release/fantuz/recordbox?label=release" alt="Latest stable release" height="20"></a>
  <a href="https://github.com/fantuz/recordbox/releases"><img src="https://img.shields.io/github/downloads/fantuz/recordbox/total?label=downloads" alt="GitHub release downloads" height="20"></a>
  <br>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/fantuz/recordbox" alt="MIT license" height="20"></a>
  <a href="app/Cargo.toml"><img src="https://img.shields.io/badge/built%20with-Rust-555555?logo=rust" alt="Built with Rust" height="20"></a>
  <a href="#running-it"><img src="https://img.shields.io/badge/platforms-macOS%20%7C%20Windows%20%7C%20Linux-507269" alt="macOS, Windows and experimental Linux packages" height="20"></a>
  <br>
  <a href="#homebrew-on-macos"><img src="https://img.shields.io/badge/Homebrew-source%20install-FBB040?logo=homebrew" alt="Install recordbox using Homebrew" height="20"></a>
  <a href="https://github.com/fantuz/recordbox/issues"><img src="https://img.shields.io/github/issues/fantuz/recordbox" alt="Open issues in fantuz/recordbox" height="20"></a>
  <a href="https://github.com/fantuz/recordbox/pulls"><img src="https://img.shields.io/github/issues-pr/fantuz/recordbox" alt="Open pull requests in fantuz/recordbox" height="20"></a>
</p>
<!-- project-badges:end -->

recordbox is an open-source desktop app for preparing DJ sets. Organise and analyse your music, rehearse a mix across decks, and export playlists for CDJ/XDJ players that read the classic Device Library format.

This is the **fantuz/recordbox** fork of Adam Sardo’s original macOS app. It builds on his work on the music library, track analysis and USB export. [Original project](https://github.com/adamsardo/recordbox)

> **Not affiliated with AlphaTheta or Pioneer DJ.** rekordbox is a trademark of AlphaTheta Corporation.

<p align="center">
  <a href="https://github.com/fantuz/recordbox/releases/latest"><img src="https://img.shields.io/badge/Download-latest%20release-2e7d32?style=for-the-badge&logo=github" alt="Download the latest recordbox release" height="32"></a>
  <br>
  <a href="#running-it">Installation details</a> · <a href="docs/MACOS-FIRST-LAUNCH.md">First launch on macOS</a> · <a href="https://github.com/fantuz/recordbox/releases">All releases</a>
</p>

## Project scope

recordbox focuses on local libraries and audio files you own or have permission to use. Streaming-service integration is outside this fork’s roadmap. Dropbox and Google Drive are possible future options for accessing a DJ’s own files. Today, audio from those services needs to be available on the local filesystem.

## Running it

Choose your package from the **Downloads** table on the latest release. Check its SHA-256 digest against `SHA256SUMS`, then extract the whole archive.
The macOS downloads currently ship without Developer ID signing or Apple notarization. Windows downloads are unsigned. [Latest release and downloads](https://github.com/fantuz/recordbox/releases/latest) · [Runtime requirements and verification](docs/BUILD-AND-PACKAGING.md)

| Platform | Start recordbox | STEMS processing |
| --- | --- | --- |
| macOS | Choose the Apple Silicon or Intel ZIP for your Mac, extract it and move `Recordbox.app` to Applications. The app runs without Rust or Homebrew. If macOS blocks the first launch, check the warning against [Apple’s instructions for opening a trusted app](https://support.apple.com/en-us/102445). | The standard app uses Metal when available. |
| Windows x64 | Run `recordbox.exe` from the extracted portable ZIP. | The standard ZIP uses the CPU. Choose the CUDA 12 preview for a compatible NVIDIA GPU such as the Quadro P2000, or the DirectML preview for a supported Radeon such as Vega 8. [CUDA and driver requirements](docs/CUDA12.md) · [Radeon drivers and setup](docs/DIRECTML.md) |
| Linux x64 (experimental) | Run `./recordbox` from the extracted tarball. | The standard build uses the CPU. Optional builds offer CUDA 12 for compatible NVIDIA GPUs or Vulkan for partial GPU acceleration alongside the CPU. [CUDA setup](docs/CUDA12.md) · [Vulkan setup](docs/VULKAN.md) |

On Linux, `./recordbox --disable-load` disables deck loading for the session. Use `./recordbox --disable-play` to browse and load tracks with audio playback and previews disabled. [Playback and analysis guide](docs/PLAYBACK-PERFORMANCE.md)

### Homebrew on macOS

If you already use Homebrew and have the Xcode Command Line Tools, the formula builds the tagged release on your Mac and manages upgrades. The first install downloads the Rust toolchain and compiles the app. For a prebuilt app, use the ZIP above.

```sh
brew tap fantuz/recordbox https://github.com/fantuz/recordbox
brew install --build-from-source fantuz/recordbox/recordbox
recordbox
```

To open the Homebrew app in Finder, run `open "$(brew --prefix fantuz/recordbox/recordbox)/libexec/Recordbox.app"`. Homebrew applies a local ad-hoc signature. Use the Cargo instructions below when developing or testing a particular revision. [macOS distribution and notarization](docs/BUILD-AND-PACKAGING.md#distributing-a-macos-app-to-djs)

## Prepare and play

- **Adjust the interface.** EXPORT and PERFORMANCE offer a Light appearance and a detachable library. Choose Full, Reduced or Off under **Preferences → Advanced → Others → recordbox → Interface motion**. [Interface behavior and measurements](docs/UPSTREAM-INTEGRATION.md#interface-motion-from-ef454ed)
- **Build a collection.** Import local tracks and folders, M3U playlists or a rekordbox XML collection. Rate and tag tracks, organise playlists, and back up or restore your recordbox library.
- **Prepare a track in EXPORT.** Analyse BPM, key and waveforms, edit the beat grid, and set loops, memory cues and hot cues. New analysis uses the Rekordbox-compatible beat detector. Choose **recordbox classic** in **Preferences → Analysis → Beat grid estimator** to use the earlier detector. Optional high-precision refinement works with either choice.
  By default, press **1–8** to set or recall hot cues A–H. Hold **Shift** and press the same number to clear that slot.
- **Prepare phrases.** Deck loading and USB export can reuse Rekordbox phrase data. To generate missing phrases, choose **Rekordbox-compatible** or **recordbox classic** under **Preferences → Analysis → Phrase estimator**.
- **Mix in PERFORMANCE.** Use MIDI Learn to map the decks, mixer and pads, or try the experimental XDJ-RX3 or OPUS-QUAD profile for one connection. The DDJ-GRV6 Groove Circuit mappings cover drum swap, capture, gain and release. SPACE, GATE COMP, Beat FX, Pad FX and MERGE FX are available for rehearsing and recording a set. [XDJ-RX3 setup](docs/studies/XDJ-RX3.md) · [OPUS-QUAD setup](docs/studies/OPUS-QUAD.md) · [Controller support and effects](docs/UPSTREAM-INTEGRATION.md#controllers-and-effects)
- **Edit an arrangement.** Use the EDIT timeline to arrange, audition and render a local track. WAV, AIFF and FLAC are available across platforms. M4A uses the macOS AAC encoder.
- **Use the library on a second screen.** The overlapping-windows button at the far right of the top bar detaches the library browser. Your playlist, search, selection and scroll position follow it. Close the library window to reattach it.
  **F11** toggles fullscreen for both windows together. **Space** plays or pauses the active deck when you’re outside a text field.
- **Find tracks for a mix.** Open **Related Tracks** to filter the collection or a playlist by your saved criteria. **Track Suggestion** ranks candidates using tempo, harmonic key, waveform and catalog metadata. Hover a result to see why it matched. For missing tags, select up to 100 tracks, choose **Look up related track tags…**, and pick the sources to consult. MusicBrainz supplies recording genres, Apple iTunes (US) supplies song genre, and Deezer can supply the matched release’s genres and label. The compatible OneTagger CLI sends the artist and full title, including the mix name. Results are saved in the recordbox library for offline matching. Matching uses the library’s label first and counts a shared genre once across sources. Additional services require CLI support for `lookup --json --provider`. [OneTagger CLI lookup](https://github.com/fantuz/onetagger/pull/93)
- **Check this computer’s hardware setup.** On first launch of a new version, recordbox checks the CPU, memory and available acceleration. **Preferences → Analysis** shows the automatic worker limit and **Extensions → STEMS** shows the backend and its availability. [Hardware setup and analysis workers](docs/PLAYBACK-PERFORMANCE.md#hardware-setup)
- **Separate a track into STEMS.** In **Preferences → Extensions → STEMS**, download HTDemucs for speed or HTDemucs fine-tuned for sound quality. Both save drums, bass, instruments and vocals as four stereo parts. Completed separations are available across decks and restarts.
  In **Export → Dual Player**, each deck has its own **STEMS** control. Request A and B in either order. One full separation runs at a time, and the other deck can play while its request waits.
  Saved parts from both models share a **50 GB** budget by default. Every save leaves at least **10% free on the cache drive**, including when **Keep all saved stems** is selected. [STEMS storage and backups](docs/LIBRARY-SYNC.md#how-much-separation-is-saved)
- **Export vocals or an instrumental.** Use **Track → Export Stems** to make a new local audio file from a separation and add it to your collection with its cues and grid. Choose WAV, FLAC or AIFF for lossless output. The first export shows the destination and playlist option. [Export behavior and format support](docs/UPSTREAM-INTEGRATION.md#exporting-vocals-and-instrumentals)

If an imported MP3 cue sounds early or late, you can audition the same timestamp using a separate collection. [Compare MP3 cue timing](docs/PLAYBACK-PERFORMANCE.md#auditioning-rekordbox-mp3-timing) · [Integration report and measurements](docs/UPSTREAM-INTEGRATION.md)

## Working with AlphaTheta libraries and USB drives

Library preparation runs locally without an account. recordbox saves routine edits in its own `library.db` and displays its path in the collection footer.
Desktop sync reads the original audio files without modifying them. Use the sync arrows at the lower left to review the source, destination and direction.

- **Import · AlphaTheta Rekordbox → recordbox** reads a private snapshot of the installed desktop `master.db` and its write-ahead log. It imports locally accessible tracks and their preparation data into recordbox. Each sync rebuilds the imported playlist folder.
- **Write back · recordbox → AlphaTheta Rekordbox** is a pilot for recognised local desktop libraries. Try it with a disposable local profile first. Import to establish a baseline, then review eligible changes to ratings, BPM, key, existing cues and beat grids.
  Close AlphaTheta Rekordbox and its agent before applying changes. Each write-back preview starts with **Keep a recovery backup** checked. You can clear it for that operation. The final review shows the destination, the selected changes and whether a recovery copy will be kept. For newly created or retimed cues, use collection XML export. [Supported edits, limits and recovery](docs/LIBRARY-SYNC.md) · [ANLZ import compatibility](docs/ANLZ-IMPORT-INTEGRITY.md)

For **USB export**, use **Device Sync Manager**. It writes the classic Device Library catalog, music and analysis files to the selected drive.
On a drive with both catalogs, recordbox updates the classic library. To inspect OneLibrary playlists, right-click the device and choose **Inspect OneLibrary playlists…**. The optional reader can compare the two catalogs’ track order. Use AlphaTheta Rekordbox to update OneLibrary. [Reader setup](docs/USB-COMPATIBILITY.md#inspect-onelibrary-playlists)
Check the player model and firmware when choosing a format. [USB formats and player compatibility](docs/USB-COMPATIBILITY.md)

## Review missing tracks and duplicates

Right-click **All** under **Collection** and choose **Review collection cleanup…**. The report groups references by drive or volume and protects references to recognised cloud locations, external disks and paths from another operating system.
On Windows and macOS, recordbox backs up the collection before moving selected eligible references to **Removed Tracks**. The audio files stay on disk. Linux provides a read-only review. [Cleanup criteria and recovery](docs/COLLECTION-CLEANUP.md)

## Build from source

Install Rust with rustup and the native prerequisites for your platform. The repository selects the toolchain used by CI. [Toolchain setup](CONTRIBUTING.md#toolchain-and-dependencies) · [Build guide](docs/BUILD-AND-PACKAGING.md)

```bash
git clone https://github.com/fantuz/recordbox.git
cd recordbox/app
cargo run --release --locked
```

## The brief

> the goal is to replace rekordbox for everything i do before a dj set.
>
> — Adam Sardo, [original recordbox brief](https://github.com/adamsardo/recordbox/blob/8e9f30ec516836739137ff191d89be10549da081/README.md#the-brief)

Thank you to Adam for building the original recordbox in the open with Claude Code through Craft Anything and sharing the source under the MIT licence.

## Feedback

Report bugs and feature requests in this fork. For a bug, include the recordbox version, operating system and steps to reproduce it. To propose a fix, open a pull request against `fantuz/recordbox`. [Open an issue](https://github.com/fantuz/recordbox/issues/new) · [Pull requests](https://github.com/fantuz/recordbox/pulls) · [Contributor guide](CONTRIBUTING.md)

## Licence

MIT. Copyright (c) 2026 Adam Sardo. [Licence text](LICENSE)
