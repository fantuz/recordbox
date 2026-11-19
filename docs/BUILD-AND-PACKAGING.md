# Native builds and application archives

Recordbox currently starts as a desktop app. The packaging helpers use Python 3.11+
and its standard library to package an **already-built release executable**. They do
not change Rust code, install dependencies, import a library, or download models.

| Artifact | Native build/inspection tools | Status |
| --- | --- | --- |
| macOS ARM64 `.app` + ZIP | Rust, Xcode SDK/Clang, `lipo`, `otool` | Primary platform |
| macOS x86_64 `.app` + ZIP | Same, on a native x86_64 runner | Primary platform |
| Windows x86_64 portable ZIP | Rust MSVC, Visual Studio tools including `dumpbin`, OpenSSL runtime | Experimental |
| Linux x86_64 TAR.GZ | Rust, C compiler, `pkg-config`, ALSA/OpenSSL development packages, `readelf`, `ldd` | Experimental |

CI pins Rust **1.100.0**. The project has no independently validated minimum Rust
version. Build each architecture separately. There is no universal macOS binary or cross-platform compatibility
claim in an archive's name.

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
Formatting and Clippy can initially report findings without blocking builds while
the project's existing baseline is reviewed. This avoids coupling an initial
packaging contribution to unrelated Rust cleanup.

The workflow's **Build and security checks** job requires every native package,
the application tests and the advisory gate to succeed. Require this status on
`main` in a branch ruleset, alongside the existing CodeQL Rust analysis. In
repository settings, keep Dependabot alerts/security updates and CodeQL enabled;
the committed Dependabot configuration schedules weekly Cargo and Actions updates.
Workflow tokens have read-only contents access, Actions use immutable commits,
builds use `--locked`, and reports/artifacts expire after 14 days. No job publishes
a release or changes repository protection settings.

Two existing baselines are explicit rather than silently ignored:

- `python packaging/run_tests.py --output reports` runs the complete serial
  application suite. At introduction it reports **235 passed, 1 failed, 25 ignored**.
  Only the existing subtraction overflow in `export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic`
  at `src/export/pdb.rs` is accepted. Another failure, compilation error, incomplete
  run or a now-passing baseline fails the check. Remove the baseline when that bug
  is fixed. Serial execution avoids an existing shared test-fixture race.
- **No new dependency advisories** retains the raw cargo-audit JSON, stderr and
  exit status. `.github/security/advisory-baseline.json` lists three unresolved
  pre-existing findings: `libsqlite3-sys 0.38.2` (RUSTSEC-2026-0459) and transitive
  `quick-xml 0.30.0` (RUSTSEC-2026-0194/0195). A new vulnerability/unsoundness,
  changed package/version/source, stale baseline, filtered report or execution
  error fails the check. Informational findings remain visible. This is not a
  clean security audit; these dependencies still need separate remediation.

**Tests and lint report** includes a rustfmt report and Clippy warnings. Existing
formatting debt is advisory; Clippy compilation errors fail. After the baseline is
cleaned up, make rustfmt and `cargo clippy --all-targets -- -D warnings` strict.
Native GUI smoke is a rendering/startup check, not a substitute for exercising
real library imports, audio playback, USB devices or signed installers.

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
  --archive dist/macos-arm64/Recordbox-0.1.0-macos-arm64.zip \
  --output smoke/macos-arm64 --timeout 240
```

Use the corresponding ZIP/TAR.GZ on Windows/Linux. This extracts the archive,
checks its manifest hashes, and runs the extracted entrypoint. **Do not use the
app's `--help`: it is not implemented and starts the normal application.**

The helper instead uses the existing `--screenshot` mode, removes inherited
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

Windows/Linux remain experimental: Finder/Apple Music/Audio MIDI Setup and
`diskutil` integrations are macOS-specific; USB auto-discovery scans `/Volumes`,
and Metal acceleration is macOS-only. These helpers do not alter that application
behavior. Native menus are currently attached only on macOS; some shortcuts on
other platforms may be suppressed by the existing menu state. Windows' hardware
memory estimate also uses an unsupported `/proc/meminfo` path. These are follow-up
application issues, rather than changes hidden inside the packaging contribution.
No Developer ID signing, Windows certificate signing, or notarization
is performed or claimed.
