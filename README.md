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
  <a href="#running-it">Installation details</a> · <a href="https://github.com/fantuz/recordbox/releases">All releases</a>
</p>

## Project scope

recordbox focuses on local libraries and audio files you own or have permission to use. Streaming-service integration is outside this fork’s roadmap. Dropbox and Google Drive are possible future options for accessing a DJ’s own files. Today, audio from those services needs to be available on the local filesystem.

## Running it

Choose your package from the **Downloads** table on the latest release. Check its SHA-256 digest against `SHA256SUMS`, then extract the whole archive.
The packages are unsigned, and macOS builds ship without Apple notarization. [Latest release and downloads](https://github.com/fantuz/recordbox/releases/latest) · [Runtime requirements and verification](docs/BUILD-AND-PACKAGING.md)

| Platform | Start recordbox | STEMS processing |
| --- | --- | --- |
| macOS | Choose the Apple Silicon or Intel ZIP for your Mac and open `Recordbox.app`. | The standard app uses Metal when available. |
| Windows x64 | Run `recordbox.exe` from the extracted portable ZIP. | The standard ZIP uses the CPU. Choose the CUDA 12 preview for a compatible NVIDIA GPU such as the Quadro P2000, or the DirectML preview for a supported Radeon such as Vega 8. [CUDA and driver requirements](docs/CUDA12.md) · [Radeon drivers and setup](docs/DIRECTML.md) |
| Linux x64 (experimental) | Run `./recordbox` from the extracted tarball. | The standard build uses the CPU. Optional builds offer CUDA 12 for compatible NVIDIA GPUs or Vulkan for partial GPU acceleration alongside the CPU. [CUDA setup](docs/CUDA12.md) · [Vulkan setup](docs/VULKAN.md) |

On Linux, `./recordbox --disable-load` disables deck loading for the session. Use `./recordbox --disable-play` to browse and load tracks with audio playback and previews disabled. [Playback and analysis guide](docs/PLAYBACK-PERFORMANCE.md)

### Homebrew on macOS

On Apple Silicon or Intel, you can build recordbox locally with Homebrew and the Xcode Command Line Tools. The first install downloads the pinned source and build dependencies, then compiles the app on your Mac.

```sh
brew tap fantuz/recordbox https://github.com/fantuz/recordbox
brew install --build-from-source fantuz/recordbox/recordbox
recordbox
```

To open the installed macOS app bundle, run `open "$(brew --prefix fantuz/recordbox/recordbox)/libexec/Recordbox.app"`. Homebrew manages the bundle and its command-line launcher. The local build uses an ad-hoc signature. Apple Developer ID signing and notarization are separate steps for distributing prebuilt downloads.

## Prepare and play

- **Build a collection.** Import local tracks and folders, M3U playlists or a rekordbox XML collection. Rate and tag tracks, organise playlists, and back up or restore your recordbox library.
- **Prepare a track in EXPORT.** Analyse BPM, key and waveforms, edit the beat grid, and set loops, memory cues and hot cues. New analysis uses the Rekordbox-compatible beat detector. Choose **recordbox classic** in **Preferences → Analysis → Beat grid estimator** to use the earlier detector. Optional high-precision refinement works with either choice.
  By default, press **1–8** to set or recall hot cues A–H. Hold **Shift** and press the same number to clear that slot.
- **Mix in PERFORMANCE.** Use the performance layouts, MIDI mappings, mixer, Beat FX and sampler to rehearse transitions or record a set.
- **Edit an arrangement.** Use the EDIT timeline to arrange, audition and render a local track. WAV, AIFF and FLAC are available across platforms. M4A uses the macOS AAC encoder.
- **Use the library on a second screen.** The overlapping-windows button at the far right of the top bar detaches the library browser. Your playlist, search, selection and scroll position follow it. Close the library window to reattach it.
  **F11** toggles fullscreen for both windows together. **Space** plays or pauses the active deck when you’re outside a text field.
- **Separate a track into STEMS.** In **Preferences → Extensions → STEMS**, download HTDemucs for speed or HTDemucs fine-tuned for sound quality. Both save drums, bass, instruments and vocals as four stereo parts. Completed separations are available across decks and restarts.
  Saved parts from both models share a 20 GB budget by default. Choose **Keep all saved stems** to retain completed results without a size limit. [STEMS storage and backups](docs/LIBRARY-SYNC.md#how-much-separation-is-saved)

See the [integration report](docs/UPSTREAM-INTEGRATION.md) for measured beat timing, live STEMS availability and the separate MP3 timing trial.

## Working with AlphaTheta libraries and USB drives

Library preparation runs locally without an account. recordbox saves routine edits in its own `library.db` and displays its path in the collection footer.
Desktop sync reads the original audio files without modifying them. Use the sync arrows at the lower left to review the source, destination and direction.

- **Import · AlphaTheta Rekordbox → recordbox** reads a private snapshot of the installed desktop `master.db` and its write-ahead log. It imports locally accessible tracks and their preparation data into recordbox. Each sync rebuilds the imported playlist folder.
- **Write back · recordbox → AlphaTheta Rekordbox** is a pilot for recognised local desktop libraries. Try it with a disposable local profile first. Import to establish a baseline, then review eligible changes to ratings, BPM, key, existing cues and beat grids.
  Close AlphaTheta Rekordbox and its agent before applying changes. **Back up and apply** checks for conflicts and saves a verified recovery copy before updating the original database and any affected analysis files. For newly created or retimed cues, use collection XML export. [Supported edits, limits and recovery](docs/LIBRARY-SYNC.md) · [ANLZ import compatibility](docs/ANLZ-IMPORT-INTEGRITY.md)

For **USB export**, use **Device Sync Manager**. It writes the classic Device Library catalog, music and analysis files to the selected drive.
On a drive with both catalogs, recordbox updates the classic library. To update OneLibrary (formerly Device Library Plus), use AlphaTheta Rekordbox.
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
