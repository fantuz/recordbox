# Native builds and application archives

Recordbox currently starts as a desktop app. The packaging helpers use Python 3.11+
and its standard library to package an **already-built release executable**. They do
not change Rust code, install dependencies, import a library, or download models.

| Artifact | Native build/inspection tools | Status |
| --- | --- | --- |
| macOS ARM64 `.app` + ZIP | Rust, Xcode SDK/Clang, `lipo`, `otool` | Primary platform |
| macOS x86_64 `.app` + ZIP | Same, on a native x86_64 runner | Primary platform |
| Windows x86_64 portable ZIP | Rust MSVC, Visual Studio tools including `dumpbin`, OpenSSL runtime | Supported |
| Linux x86_64 TAR.GZ | Rust, C compiler, `pkg-config`, ALSA/OpenSSL development packages, `readelf`, `ldd` | Experimental |

The root `rust-toolchain.toml` pins Rust **1.100.0**, rustfmt and Clippy, matching
CI. The project has no independently validated minimum Rust version. Build each
architecture separately. There is no universal macOS binary or cross-platform
compatibility claim in an archive's name. See [Contributing](../CONTRIBUTING.md)
for developer commands, dependency policy and repository settings.

## Build checks worth keeping

Run native compilation and targeted tests as required checks, with logs and
artifacts retained even on failure:

```sh
cargo build --locked --release --manifest-path app/Cargo.toml
cargo test --locked --release --manifest-path app/Cargo.toml backup::tests:: -- --nocapture
python -m unittest discover -s packaging -p 'test_*.py' -v
```

The backup filter exercises round trips, refusal to overwrite existing music, and
mismatched ZIP header rejection. Dependency auditing belongs in a separate check
against the committed `app/Cargo.lock`; retain its detailed advisory report.
Formatting and Clippy report existing findings without blocking builds while
that debt is reviewed. Keep mechanical cleanup separate from behavioral fixes.

The workflow's **Build and security checks** job requires every native package,
the application tests and the advisory gate to succeed. Require this status on
`main` in a branch ruleset, alongside the existing CodeQL Rust analysis. In
repository settings, keep Dependabot alerts/security updates and CodeQL enabled;
the committed Dependabot configuration schedules weekly Cargo and Actions updates.
Build jobs have read-only contents access, Actions use immutable commits, and
builds use `--locked`. Reports/development artifacts expire after 14 days;
published release assets are retained separately. Only the fork's gated release
publication job receives contents-write access. Automation does not change
repository protection settings.

## Homebrew on macOS

The fork’s tap builds the tagged Rust source on the Mac where you install it. Install the Xcode Command Line Tools and Homebrew before building the app.

```sh
brew tap fantuz/recordbox https://github.com/fantuz/recordbox
brew install --build-from-source fantuz/recordbox/recordbox
brew test fantuz/recordbox/recordbox
open "$(brew --prefix fantuz/recordbox/recordbox)/libexec/Recordbox.app"
```

You can also start `recordbox` from the terminal. The build downloads its compiler and locked dependencies. The installed app uses macOS system libraries and carries a local ad-hoc signature. Developer ID signing and Apple notarization are separate distribution processes.

After publishing a new stable release, update the `tag` and full commit `revision` together in the tap’s formula. Use the commit identified in the release notes, and check that the release tag resolves to it. The Homebrew workflow installs the formula by its tap name and tests the installed bundle. Once the formula change reaches the tap’s default branch, users can run `brew update` and `brew upgrade fantuz/recordbox/recordbox`. [Homebrew formula](../Formula/recordbox.rb) · [Native installation check](../.github/workflows/homebrew.yml)

## Distributing a macOS app to DJs

The release ZIP already contains a standalone `Recordbox.app` for the chosen
architecture. The packager checks the linked libraries so someone downloading
the app can run it without installing Homebrew, Xcode or Rust. The Homebrew
formula is an alternative for users who already use Terminal and prefer
Homebrew-managed upgrades. It builds from source. Cargo is useful for testing
a branch or changing the code.

For a Finder-first installation, the next step is to sign the finished bundle
with a **Developer ID Application** certificate using the hardened runtime
and a secure timestamp, then submit it to Apple using `notarytool`. After
Apple accepts it, staple the ticket to the app, validate it, and package that
exact app for download. Generate the release hashes from the final archive
and test a fresh browser download on a Mac with Gatekeeper enabled. This
route distributes directly from GitHub. A Homebrew **cask** could then
install the same prebuilt app. [Apple’s notarization guide](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) · [Homebrew cask documentation](https://docs.brew.sh/Cask-Cookbook)

This release process currently produces archives without Developer ID
signing or notarization. Enabling the signed path requires Apple Developer
Program access, a Developer ID signing identity with its private key, and
notary-service credentials. The current release validator requires unsigned
artifacts. For a signed release, update it to check the expected Developer ID
and notarization, and store hashes of the final signed executable outside
the sealed app bundle. This avoids changing a sealed resource after signing.
[Developer ID](https://developer.apple.com/developer-id/) · [Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)

## Fork release process

The first packaged release of `fantuz/recordbox` is **0.5.0**. Each release goes
through a pull request: update root `VERSION`, `app/Cargo.toml`, the matching
package entry in `app/Cargo.lock`, and `docs/releases/<version>.md` together. The
release notes distinguish delivered changes from the application's existing
feature baseline and disclose the tracked test/security findings below.

After the release pull request passes its checks, merge it to the fork's `main`
and wait for **Build and package** to finish successfully on that exact merged
revision. A maintainer then creates and pushes
the immutable `v<version>` tag at that validated commit. Merging to `main` does not
automatically create a tag or publish a release.

The tag starts the same **Build and package** workflow for that exact revision.
Before heavy builds, its version gate requires the tag, `VERSION`, Cargo manifest
and lockfile to agree. It can reuse all four native packages and matching smoke
artifacts from **one completed, successful `main` push** of this workflow in the
same repository, for the exact tag SHA and embedded `VERSION`. PR/fork runs,
other branches/revisions, incomplete or expired sets, and mixed runs are refused.
Immutable artifact IDs select the original archives and smoke reports; the old
release bundle is not used as a substitute for verifying them again.

The complete application suite and a **fresh dependency-advisory audit** still
run on the tag.
Publication requires those gates plus fresh verification of the four standard
archives and their matching smoke evidence. From v0.5.5, the Windows and Linux
package jobs also supply CUDA 12 previews. The verifier checks their native
executables, CUDA imports, Pascal FP32 kernels, setup guide and checksums.
The one-song comparison runs on a physical GPU. [CUDA setup and test](CUDA12.md)

Apple Silicon packaging requests the free native macOS 26 image. On the first retry the workflow requests macOS 15 and uses the matching build cache. Both images build the ARM64 archive and launch the extracted app.

From v0.5.6 the same Windows runner also builds the Radeon DirectML preview.
Release verification checks its executable, Microsoft runtime DLLs, distribution
notices and setup guide. [DirectML build and test](DIRECTML.md)

From v0.6.0 the Linux package job also supplies the experimental Vulkan + CPU
archive. It runs the projection tests with Mesa's software Vulkan driver and
checks the extracted launcher's diagnostics and CPU selection. Release
verification checks the embedded shader against the tagged source, the setup
guide and the archive checksum. Testing acceleration on a physical GPU requires
a Linux machine with a compatible driver. [Vulkan setup and test](VULKAN.md)

Only the native compilation/smoke
jobs are skipped when that complete trusted set was explicitly selected.
If discovery finds no complete set or the API is unavailable, every native build
and smoke job runs as before. Artifact expiry/deletion after selection fails
publication closed: use **Re-run all jobs** to run selection again and fall back
to native builds. A failed-jobs-only retry retains the previous selection.
Artifacts normally expire after 14 days. Waiting for the exact merged `main` run
to turn green before tagging avoids a second Apple Silicon build request; free
hosted runner availability remains best effort. Existing tags are never moved,
and this workflow improvement only applies to tags containing it.

`packaging/prepare_release.py` validates the notes, complete archive/checksum set,
and each archive's manifest, source revision, runtime/file hashes and target,
together with the native smoke evidence.
Its required `--repository owner/repo` argument sets the download destinations;
CI passes `--repository "$GITHUB_REPOSITORY"`. Before adding the complete commit
history, it regenerates the notes' `recordbox-downloads:start` / `:end` block
with links for that repository and version tag. Future release notes can copy
this block: archive, checksum, history and packaging-guide links are refreshed
for each release.

`packaging/publish_release.py` creates a draft GitHub Release, uploads the
platform archives, `COMMIT-LOG.txt` and `SHA256SUMS`, and verifies the uploads
before publishing. If the release is already public, a rerun uses read-only
requests to compare the tag, release notes and every asset's size and SHA-256
with the prepared bundle. A mismatch fails the job. Publication jobs run one
at a time, and only a newer stable version can become latest.
Pull-request runs and ordinary branch pushes do not
publish. A tag workflow publishes only to the repository where that tag was
pushed: a tag in this fork does not create an upstream tag or release. If upstream
adopts the workflow, its maintainer enables Actions and pushes a matching version
tag there to start that repository's own release.

Download the archive for your operating system and architecture and the release's
`SHA256SUMS` file. Verify the checksum before extraction. These
remain unsigned packages with the platform limitations documented below.

`python packaging/run_tests.py --output reports` runs the complete serial
application suite and requires a complete successful result with no failed
tests. The former PDB overflow failure allowance has been removed. Compilation
errors, failed tests, incomplete or filtered runs fail the check. Ignored tests
remain visible in the retained report; they are not counted as executed
coverage. Serial execution avoids existing shared test-fixture races.

**No new dependency advisories** retains the raw cargo-audit JSON, stderr and
exit status. `.github/security/advisory-baseline.json` records the exact unresolved
findings. A new vulnerability/unsoundness, changed package/version/source, stale
baseline, filtered report or execution error fails the check. Informational
findings remain visible. An accepted advisory baseline does not mean a clean
security audit; remaining entries still need separate remediation.

**Tests and lint report** includes a rustfmt report and Clippy warnings. Existing
formatting debt is advisory. Clippy uses `--cap-lints warn` so its existing
deny-by-default lint findings are also visible without requiring Rust source
cleanup; compilation/type errors still fail. Regular builds and tests retain
their normal compiler lint policy. After the baseline is
cleaned up, make rustfmt and `cargo clippy --all-targets -- -D warnings` strict.
Native GUI smoke is a rendering/startup check, not a substitute for exercising
real library imports, audio playback, USB devices or signed installers.
The full serial application suite and Clippy run on Linux; every native package
job runs backup, SQLCipher/WAL, audio-import and PDB-boundary regressions plus
GUI smoke. The optional USB real-export
oracle group is explicitly ignored and requires `RECORDBOX_TEST_ORACLES` fixtures. See
[Contributing](../CONTRIBUTING.md#optional-format-oracles) for the targeted command
and explicit fixture requirements.

On native macOS, also run:

```sh
set -o pipefail
cargo test --locked --manifest-path app/Cargo.toml --bin recordbox \
  stems::htdemucs::tests::metal_kernels_match_reference_graph \
  -- --exact --nocapture 2>&1 | tee metal-test.log
```

This existing test returns early when `Device::new_metal(0)` fails. Cargo's `ok`
alone therefore does not prove GPU execution. Actual execution prints all four
`fused kernels vs reference` lines for `drums`, `bass`, `other`, and `vocals`.
Report Metal execution as unverified if these are absent. No checkpoint or audio
download is needed; its inputs are synthetic. Candle 0.11.1 embeds shader source
and compiles it through Metal at runtime, so a separate Metal command-line
toolchain download is unnecessary.

## Package a build

For a native ARM64 macOS build, from the repository root:

```sh
python packaging/build_archive.py \
  --binary app/target/release/recordbox \
  --platform macos --arch arm64 \
  --source-commit "$(git rev-parse HEAD)" --output dist/macos-arm64
```

Use `--arch x86_64` on a native Intel build. `lipo` checks the actual architecture;
`otool` supplies the minimum macOS version and linked dylibs. External/Homebrew or
unresolved `@rpath` dylibs cause packaging to fail. The normal native SQLCipher
build uses Apple CommonCrypto/Security/CoreFoundation; leave OpenSSL overrides
unset for this build. The packager does not rewrite dylibs or sign/notarize an app.

The output contains `Recordbox.app`, a ZIP, and its SHA-256 sidecar. `Info.plist`
uses the Cargo version, bundle identifier `io.github.adamsardo.recordbox`, and the
executable's deployment target. Branding/fonts and shader text are already
embedded. No asset tree or vendor application's resources are copied.

For Windows, use an MSVC developer shell where `dumpbin` is available. Supply
only trusted build-runtime directories, for example the pinned vcpkg
`installed/x64-windows/bin` and Visual Studio's
`VC/Redist/MSVC/<version>/x64/Microsoft.VC143.CRT`:

```text
python packaging/build_archive.py --binary app/target/release/recordbox.exe
  --platform windows --arch x86_64 --source-commit <full-git-commit> --output dist/windows
  --runtime-dir <vcpkg>/installed/x64-windows/bin
  --runtime-dir <Visual-Studio-VC-redist>/x64/Microsoft.VC143.CRT
  --runtime-notice <vcpkg>/installed/x64-windows/share/openssl/copyright
```

The command above is shown across lines for readability; use your shell's line
continuation syntax. `dumpbin` is run on the EXE and each required DLL. Only the
recursive dependency closure is copied. This includes `libcrypto-3-x64.dll` and
any imported VC++ redistributable DLLs; it does not bulk-copy `libssl`, OpenSSL
providers, debug runtimes, or files found through arbitrary `PATH` entries.
Windows/API-set DLLs stay external. A missing DLL or architecture mismatch fails
packaging. Pass additional runtime license/notice files with `--runtime-notice`.
The workflow pins vcpkg to `9eec002df1cd109f8f104cd966a763daf9751489`
(OpenSSL 3.6.5, port revision 2). VC runtime DLLs come only from the official
Visual Studio redistributable directory; see Microsoft's
[redistribution guidance](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files?view=msvc-170)
and [VS 2022 redistributable list](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution).

For Linux:

```sh
python packaging/build_archive.py \
  --binary app/target/release/recordbox \
  --platform linux --arch x86_64 \
  --source-commit "$(git rev-parse HEAD)" --output dist/linux
```

`readelf` checks architecture and records the highest referenced GLIBC/GLIBCXX/
CXXABI versions. Build on the oldest supported distribution: a build against a
newer glibc will not become compatible with older systems by archiving it.
`ldd` records linked system dependencies. Unresolved or non-system libraries must
be supplied explicitly through `--runtime-library` or `--runtime-dir`; selected
non-system libraries go under `lib/`. A small launcher adds that directory to the
loader path. System glibc, graphics/audio drivers, ALSA and OpenSSL remain runtime
requirements rather than being copied from the build host.

On Ubuntu 22.04, the ordinary runtime baseline includes `libasound2` and `libssl3`
(glibc 2.35); newer distribution package names can differ. The archive manifest,
not that example, records the actual linked ABI baseline. X11/Wayland/OpenGL
libraries loaded dynamically do not appear in `ldd`: a desktop must provide
`libX11`, `libX11-xcb`, `libXcursor`, `libXi`, `libxkbcommon`,
`libxkbcommon-x11`, `libGL`/`libEGL`, or the selected Wayland client/cursor/EGL
stack. Native file dialogs also need a working XDG Desktop Portal and backend.

All archives contain the explicit executable/runtime files, MIT license, Inter
notice and full OFL, optional supplied runtime notices, and `runtime-manifest.json`. The
manifest records the caller-supplied source revision, Cargo manifest/lock hashes,
version, target, runtime requirements, file hashes and permission modes. This
metadata identifies the build; it does not recover source provenance from a
binary. The packager never scans a home directory or copies music, databases,
models, credentials, screenshots, or caches. It refuses to overwrite prior output.

## Smoke the extracted archive

```sh
python packaging/smoke_app.py \
  --archive "dist/macos-arm64/Recordbox-$(cat VERSION)-macos-arm64.zip" \
  --output smoke/macos-arm64 --timeout 240
```

Use the corresponding ZIP/TAR.GZ on Windows/Linux. This extracts the archive,
checks its manifest hashes, and runs the extracted entrypoint. `--help` lists
supported launch options and exits. To exercise the graphical app, the
helper uses `--screenshot` mode, removes inherited
`PREPDECK_*` research flags, and assigns a fresh temporary `PREPDECK_DATA` directory.
The app generates synthetic demos there; screenshot mode skips rekordbox library
sync and automatic stem downloads. Temporary demo audio/database files are removed
after the run. The archive remains free of these files. Startup may check whether
an existing rekordbox database path exists; it does not open it in this mode.

Smoke requires both a zero exit status and a complete PNG with valid chunk CRCs.
It writes a synthetic screenshot, application log, and `smoke.json`. Loader
overrides are removed; Windows uses a system-only child `PATH` and explicitly
loads bundled DLLs before GUI startup. A successful smoke demonstrates archive
startup and rendering, not audio hardware operation or feature parity. A runner
without a working graphics session must report that limitation as a smoke failure.
For headless Linux, use an X11 session such as `xvfb-run` with Mesa software GL.

If a hosted Windows runner cannot initialize desktop OpenGL, CI can supply a
pinned, SHA-256-verified Mesa x64 release directory only for the test:

```text
python packaging/smoke_app.py --archive <Windows-ZIP> --output smoke/windows
  --windows-opengl-dir <verified-Mesa-extraction>/x64 --timeout 240
```

This optional hook runs **after** the archive's manifest/hash verification. It
checks both `opengl32.dll` and `libgallium_wgl.dll` and their non-OS dependency
closure with `dumpbin`, requires x86_64 throughout, and copies only that closure
into the temporary extracted application. Unknown dependencies or conflicting
application DLLs fail the test. It sets `GALLIUM_DRIVER=llvmpipe` and
`LIBGL_ALWAYS_SOFTWARE=1` for the child processes. `smoke.json` records the temporary
override and each driver file's SHA-256. The original ZIP, application manifest,
and distribution files are unchanged; Mesa is never shipped in the product
archive. CI pins [Mesa 26.2.7 MinGW x64](https://github.com/pal1000/mesa-dist-win/releases/tag/26.2.7)
and verifies its download SHA-256 before extraction. That test build requires
SSSE3; hosted x64 runners provide it. Upstream Mesa/component and distribution
tool notices apply to this external test dependency.

The smoke report also records the child's exit status and whether its temporary
profile/database were created, even when no screenshot appears. These are startup
diagnostics, not substitutes for the required PNG. In the current application,
graphics initialization precedes profile creation; a zero exit with neither
profile nor PNG therefore does not count as a successful smoke test.

Windows x64 has a supported portable ZIP. Linux x64 is experimental.
Finder, Apple Music, Audio MIDI Setup, Metal and the `diskutil` eject action use
macOS integrations. USB auto-discovery scans `/Volumes`. Native menus attach on
macOS and the menu state can suppress some shortcuts on other platforms.
Windows reports physical memory through the system API.
No Developer ID signing, Windows certificate signing, or notarization
is performed or claimed.
