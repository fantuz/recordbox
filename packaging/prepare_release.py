"""Verify native CI artifacts as data and stage a versioned GitHub release."""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path, PurePosixPath
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import tarfile
import tempfile
from urllib.parse import quote
import zipfile

import build_archive
from build_archive import PackageError, sha256
from check_advisories import load_json
from smoke_app import extract_archive, png_dimensions


TARGETS = (("macos", "arm64"), ("macos", "x86_64"), ("windows", "x86_64"), ("linux", "x86_64"))
MACOS_FIRST_LAUNCH = "docs/MACOS-FIRST-LAUNCH.md"


def require(condition, message):
    if not condition:
        raise PackageError(message)


def safe_path(name):
    require(isinstance(name, str) and bool(name) and "\\" not in name and ":" not in name and "\0" not in name, "Unsafe artifact path")
    path = PurePosixPath(name)
    require(not path.is_absolute() and str(path) == name and all(
        re.fullmatch(r"[A-Za-z0-9_.+-]+", part) and part not in {".", ".."}
        and not part.endswith(".") and not re.fullmatch(r"(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?", part, re.I)
        for part in path.parts), f"Unsafe artifact path: {name}")
    return path


def preflight_archive(archive, expected_root):
    """Validate names/types/modes and expansion limits before using the extractor."""
    seen = set(); total = 0
    def entry(name, directory, mode, size):
        nonlocal total
        clean = name[:-1] if directory and name.endswith("/") else name
        path = safe_path(clean)
        require(path.parts[0] == expected_root and clean.casefold() not in seen, "Wrong archive root or duplicate/case-colliding member")
        seen.add(clean.casefold())
        require(mode == 0o755 if directory else mode in {0o644, 0o755}, "Unsafe archive mode")
        require(type(size) is int and 0 <= size <= 512 * 1024**2, "Invalid or oversized archive member")
        require(not directory or size == 0, "Directory members must not contain payloads")
        total += size
        require(len(seen) <= 2048 and total <= 1024**3, "Archive expansion limit exceeded")
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as source:
            for member in source.infolist():
                require(member.orig_filename == member.filename and not member.flag_bits & 1, "NUL path or encrypted ZIP entry")
                mode = member.external_attr >> 16
                require(stat.S_IFMT(mode) == (stat.S_IFDIR if member.is_dir() else stat.S_IFREG), "ZIP links/devices are not supported")
                entry(member.filename, member.is_dir(), stat.S_IMODE(mode), member.file_size)
    else:
        with tarfile.open(archive, "r:gz") as source:
            for member in source:
                require(member.type in {tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE} and not member.sparse, "TAR links/devices/sparse files are not supported")
                entry(member.name, member.isdir(), member.mode, member.size)
    require(expected_root.casefold() in seen, "Archive has no application root")


def verify_machine(path, platform, architecture):
    """Check executable format/architecture; native CI already tested execution."""
    size = path.stat().st_size
    with path.open("rb") as stream:
        header = stream.read(64)
        if platform == "macos":
            require(len(header) >= 32 and header[:4] == b"\xcf\xfa\xed\xfe", "Expected a thin 64-bit Mach-O executable")
            cpu, _, kind, count, commands = struct.unpack_from("<IIIII", header, 4)
            require(cpu == {"arm64": 0x100000C, "x86_64": 0x1000007}[architecture] and kind == 2
                    and count > 0 and commands >= count * 8 and 32 + commands <= size, "Wrong Mach-O architecture/type or truncated commands")
        elif platform == "windows":
            require(len(header) == 64 and header[:2] == b"MZ", "Expected a PE executable")
            offset = struct.unpack_from("<I", header, 60)[0]
            require(64 <= offset <= size - 24, "Invalid PE header offset")
            stream.seek(offset); pe = stream.read(24)
            machine, sections = struct.unpack_from("<HH", pe, 4)
            optional, flags = struct.unpack_from("<HH", pe, 20)
            require(pe[:4] == b"PE\0\0" and machine == 0x8664 and sections > 0 and flags & 2
                    and not flags & 0x2000 and optional >= 112 and offset + 24 + optional + sections * 40 <= size
                    and stream.read(2) == b"\x0b\x02", "Wrong PE architecture/type or truncated headers")
        else:
            require(len(header) == 64 and header[:7] == b"\x7fELF\x02\x01\x01" and header[7] in {0, 3}, "Expected a little-endian ELF64 executable")
            kind, machine, version = struct.unpack_from("<HHI", header, 16)
            offset = struct.unpack_from("<Q", header, 32)[0]
            ehsize, phsize, count = struct.unpack_from("<HHH", header, 52)
            require(kind in {2, 3} and machine == 62 and version == 1 and ehsize == 64
                    and phsize == 56 and count > 0 and offset >= 64 and offset + count * phsize <= size,
                    "Wrong ELF architecture/type or truncated program headers")


def source_hashes(path, platform):
    data = path.read_bytes()
    hashes = {hashlib.sha256(data).hexdigest()}
    if platform == "windows":
        # checkout's core.autocrlf conversion is the only permitted source-byte variant.
        hashes.add(hashlib.sha256(data.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")).hexdigest())
    return hashes


def verify_package(archive, destination, source, version, commit, platform, architecture, variant=""):
    root = "Recordbox.app" if platform == "macos" else f"Recordbox-{version}-{platform}-{architecture}"
    if variant:
        root += "-" + variant
    preflight_archive(archive, root)
    bundle = extract_archive(archive, destination)
    resource = bundle / "Contents/Resources" if platform == "macos" else bundle
    manifest_path = resource / "runtime-manifest.json"
    require(list(bundle.rglob("runtime-manifest.json")) == [manifest_path], "Expected one runtime manifest")
    manifest = load_json(manifest_path)
    require(isinstance(manifest, dict) and type(manifest.get("schema")) is int and manifest["schema"] == 1
            and manifest.get("product") == "Recordbox" and manifest.get("version") == version, "Wrong product/version manifest")
    require(manifest.get("distribution") == {"signing_performed": False, "notarization_performed": False}, "Unexpected signing/notarization provenance")
    provenance, target = manifest.get("source"), manifest.get("target")
    require(isinstance(provenance, dict) and provenance.get("commit") == commit and isinstance(target, dict)
            and target.get("platform") == platform and target.get("architecture") == architecture, "Mixed source or target provenance")
    for file, key in (("app/Cargo.toml", "cargo_manifest_sha256"), ("app/Cargo.lock", "cargo_lock_sha256")):
        require(provenance.get(key) in source_hashes(source / file, platform), "Source Cargo digest mismatch")
    rows = manifest.get("files"); require(isinstance(rows, list) and rows, "Missing manifest file list")
    listed = set()
    for row in rows:
        require(isinstance(row, dict), "Invalid manifest file row")
        name = str(safe_path(row.get("path")))
        require(name.casefold() not in listed and name != manifest_path.relative_to(bundle).as_posix(), "Duplicate/self-listed manifest file")
        listed.add(name.casefold()); path = bundle / name
        require(row.get("mode") in {"0644", "0755"} and type(row.get("size")) is int and path.is_file()
                and path.stat().st_size == row["size"] and sha256(path) == row.get("sha256")
                and stat.S_IMODE(path.stat().st_mode) == int(row["mode"], 8), f"Manifest file mismatch: {name}")
        require(path.name.lower() not in {"opengl32.dll", "libgallium_wgl.dll"}, "CI Mesa driver must not enter a product archive")
    actual = {p.relative_to(bundle).as_posix().casefold() for p in bundle.rglob("*") if p.is_file()}
    require(actual == listed | {manifest_path.relative_to(bundle).as_posix().casefold()}, "Unmanifested archive files")
    executable = {"macos": "Contents/MacOS/recordbox", "windows": "recordbox.exe", "linux": "bin/recordbox"}[platform]
    require(manifest.get("entrypoint") == ("recordbox" if platform == "linux" else executable), "Wrong product entrypoint")
    require(stat.S_IMODE((bundle / executable).stat().st_mode) == 0o755, "Executable permission is missing")
    verify_machine(bundle / executable, platform, architecture)
    backends = {"": None, "cuda12": "cuda12-sm61", "directml": "directml-radeon", "vulkan": "vulkan"}
    require(variant in backends and target.get("stems") == backends[variant], "Wrong STEMS build variant")
    if variant == "vulkan":
        build_archive.check_vulkan_binary(bundle / executable, source)
        require(sha256(resource / "VULKAN.md") in source_hashes(source / "docs/VULKAN.md", platform), "Wrong Vulkan setup guide")
    elif variant == "directml":
        build_archive.check_directml_binary(bundle / executable)
        build_archive.directml_runtime.verify(bundle, packaged=True)
        require(set(build_archive.directml_runtime.LIBRARIES) <= set(manifest["runtime"]["bundled_libraries"]), "Missing DirectML libraries")
        require(sha256(resource / "DIRECTML.md") in source_hashes(source / "docs/DIRECTML.md", platform), "Wrong DirectML setup guide")
    elif variant == "cuda12":
        build_archive.check_cuda_binary(bundle / executable, manifest["runtime"]["system_dependencies"], platform)
        require(sha256(resource / "CUDA12.md") in source_hashes(source / "docs/CUDA12.md", platform), "Wrong CUDA setup guide")
    for packaged, original in (("LICENSE.txt", "LICENSE"), ("Inter-OFL.txt", "packaging/Inter-OFL.txt"), ("Inter-NOTICE.txt", "app/assets/fonts/README.txt")):
        require(sha256(resource / packaged) in source_hashes(source / original, platform), f"Missing/mismatched license notice: {packaged}")
    if platform == "macos":
        plist = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
        numeric = version.split("-", 1)[0].split("+", 1)[0]
        require(plist.get("CFBundleShortVersionString") == numeric and plist.get("CFBundleVersion") == numeric
                and plist.get("CFBundleExecutable") == "recordbox", "macOS plist version/entrypoint mismatch")
    return manifest


def verify_smoke(directory, archive, manifest, commit, platform):
    require(directory.is_dir() and not directory.is_symlink(), "Missing native smoke artifact")
    allowed = {"smoke.json", "synthetic-window.png", "application.log"} | ({"dll-preflight.log"} if platform == "windows" else set())
    require(all(p.is_file() and not p.is_symlink() and p.name in allowed for p in directory.iterdir()), "Unexpected smoke artifact files")
    report = load_json(directory / "smoke.json")
    require(isinstance(report, dict) and report.get("status") == "passed" and report.get("synthetic_data_only") is True
            and type(report.get("process_returncode")) is int and report["process_returncode"] == 0
            and report.get("profile_created") is True and report.get("database_created") is True,
            "Native smoke did not establish a successful isolated application launch")
    require(report.get("archive") == archive.name and report.get("archive_sha256") == sha256(archive)
            and report.get("source_commit") == commit and report.get("target") == manifest["target"], "Smoke/archive provenance mismatch")
    dimensions = png_dimensions(directory / "synthetic-window.png")
    screenshot = report.get("screenshot")
    require(isinstance(screenshot, dict) and all(type(screenshot.get(k)) is int for k in ("width", "height"))
            and screenshot == dict(zip(("width", "height"), dimensions)), "Smoke PNG dimensions disagree with report")
    if "windows_opengl_test_driver" in report:
        driver = report["windows_opengl_test_driver"]
        require(platform == "windows" and isinstance(driver, dict) and driver.get("temporary_override") is True, "Unexpected test-driver report")


def get_commit_log(source, commit):
    head = subprocess.run(["git", "-C", str(source), "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    require(head == commit, "Release source checkout does not match requested commit")
    return subprocess.run(["git", "-C", str(source), "log", "--reverse", "--topo-order", "--date=iso-strict",
                           "--format=commit %H%nAuthor: %an <%ae>%nDate: %ad%n%n%B", commit, "--"],
                          check=True, capture_output=True, text=True, encoding="utf-8").stdout


def render_release_downloads(notes, repository, tag, *, has_macos_guide=False):
    """Refresh the complete Downloads section for this repository and version."""
    require(isinstance(repository, str) and re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository),
            "Require a GitHub repository in owner/repo form")
    number = r"(?:0|[1-9][0-9]*)"; identifier = r"[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*"
    require(isinstance(tag, str) and re.fullmatch(rf"v{number}\.{number}\.{number}(?:-{identifier})?(?:\+{identifier})?", tag),
            "Require a safe v<version> release tag")
    version = tag[1:]; prerelease = version.split("+", 1)[0].partition("-")[2]
    require(not any(part.isdigit() and len(part) > 1 and part.startswith("0") for part in prerelease.split(".")),
            "Require a safe v<version> release tag")
    start, end = "<!-- recordbox-downloads:start -->", "<!-- recordbox-downloads:end -->"
    counts = notes.count(start), notes.count(end)
    require(notes.count("recordbox-downloads:") == sum(counts) and counts in {(0, 0), (1, 1)},
            "Malformed or duplicate release download markers")
    if counts == (1, 1):
        require(notes.index(start) < notes.index(end), "Malformed release download marker order")
    base = f"https://github.com/{repository}"; download = f"{base}/releases/download/{quote(tag, safe='')}"
    def link(name):
        return f"[`{name}`]({download}/{quote(name, safe='')})"
    rows = [start, "## Downloads", ""]
    if has_macos_guide:
        rows.extend([
            "**First launch on macOS.** The Mac ZIPs ship without Developer ID signing or Apple notarization. "
            "Verify your ZIP against `SHA256SUMS`, extract it and open `Recordbox.app`. "
            "If macOS cannot verify the developer and you trust this copy, choose "
            "**System Settings → Privacy & Security → Open Anyway** for recordbox and confirm.",
            f"[recordbox first-launch guide]({base}/blob/{quote(tag, safe='')}/{MACOS_FIRST_LAUNCH}) · "
            "[Apple’s instructions](https://support.apple.com/en-us/102445)",
            "",
        ])
    rows.extend(["| Platform | Archive |", "| --- | --- |"])
    labels = {("macos", "arm64"): "macOS Apple Silicon", ("macos", "x86_64"): "macOS Intel",
              ("windows", "x86_64"): "Windows x64" if build_archive.has_directml(version) else "Windows x64, experimental",
              ("linux", "x86_64"): "Linux x64, experimental"}
    for platform, architecture, variant, name in build_archive.release_archives(version):
        label = labels[platform, architecture]
        if variant == "cuda12":
            if not build_archive.has_directml(version):
                label = f"{platform.capitalize()} x64"
            label = f"{label}, CUDA 12 STEMS preview"
        elif variant == "directml":
            label = f"{label}, Radeon DirectML STEMS preview"
        elif variant == "vulkan":
            label = f"{label}, Vulkan + CPU STEMS"
        rows.append(f"| {label} | {link(name)} |")
    rows.extend(["", f"Download {link('SHA256SUMS')} to verify the release archives and {link('COMMIT-LOG.txt')}",
                 "for the complete source history. See the",
                 f"[build and packaging guide]({base}/blob/{quote(tag, safe='')}/docs/BUILD-AND-PACKAGING.md) for runtime requirements",
                 "and verification.", end])
    block = "\n".join(rows)
    if counts == (1, 1):
        return notes[:notes.index(start)] + block + notes[notes.index(end) + len(end):]
    section = re.search(r"(?m)^##[ \t]+Downloads[ \t]*\r?$", notes)
    if section:
        following = re.search(r"(?m)^##[ \t]+", notes[section.end():])
        boundary = section.end() + following.start() if following else len(notes)
        return notes[:section.start()] + block + ("\n\n" if following else "\n") + notes[boundary:]
    separator = "" if not notes or notes.endswith("\n\n") else "\n" if notes.endswith("\n") else "\n\n"
    return notes + separator + block + "\n"


def prepare_release(input_root, output, source, commit, tag, repository):
    require(re.fullmatch(r"[0-9a-f]{40}", commit) is not None, "Require a full lowercase source commit SHA")
    version = build_archive.read_release_version(source)
    require(tag == "v" + version, "Tag, VERSION and Cargo versions must agree")
    require(not output.exists() and not output.is_symlink(), "Release output already exists")
    require(input_root.is_dir() and not input_root.is_symlink() and {p.name for p in input_root.iterdir()} == {"packages", "smoke"}, "Unexpected release input layout")
    archives = list(build_archive.release_archives(version))
    expected = {name + suffix for _, _, _, name in archives for suffix in ("", ".sha256")}
    packages = input_root / "packages"; smoke = input_root / "smoke"
    require(packages.is_dir() and not packages.is_symlink() and {p.name for p in packages.iterdir()} == expected
            and all(p.is_file() and not p.is_symlink() for p in packages.iterdir()), "Missing, duplicate or unexpected package files")
    require(smoke.is_dir() and not smoke.is_symlink() and {p.name for p in smoke.iterdir()} == {f"smoke-{p}-{a}-{commit}" for p, a in TARGETS}, "Missing/mixed native smoke artifacts")
    verified = {}
    with tempfile.TemporaryDirectory(prefix="recordbox-release-verify-") as temporary:
        for platform, arch, variant, name in archives:
            archive = packages / name; checksum = sha256(archive)
            require((packages / (name + ".sha256")).read_text(encoding="utf-8") == f"{checksum}  {name}\n", "Native archive checksum sidecar mismatch")
            manifest = verify_package(archive, Path(temporary) / name, source, version, commit, platform, arch, variant)
            if not variant:
                verify_smoke(smoke / f"smoke-{platform}-{arch}-{commit}", archive, manifest, commit, platform)
            verified[name] = checksum
    notes = (source / f"docs/releases/{version}.md").read_text(encoding="utf-8").strip()
    require(bool(notes), "Release notes are empty")
    # Keep older tags reproducible and link only to a guide shipped by this release.
    has_macos_guide = (source / MACOS_FIRST_LAUNCH).is_file()
    notes = render_release_downloads(notes, repository, tag, has_macos_guide=has_macos_guide)
    signing_note = "" if has_macos_guide else "Archives are unsigned and macOS builds are not notarized. "
    log = get_commit_log(source, commit); require(bool(log.strip()), "Commit log is empty")
    fence = "`" * max(3, 1 + max((len(run) for run in re.findall(r"`+", log)), default=0))
    body = (f"{notes}\n\n<!-- recordbox-release-source: {commit} -->\n\n"
            f"Source commit: `{commit}`. The four standard archives passed native screenshot checks. "
            f"{signing_note}Verify downloads with `SHA256SUMS`.\n\n"
            f"<details>\n<summary>Complete commit history</summary>\n\n{fence}text\n{log.rstrip()}\n{fence}\n\n</details>\n")
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".recordbox-release-", dir=output.parent))
    try:
        assets = staging / "assets"; assets.mkdir()
        for _, _, _, name in archives:
            shutil.copyfile(packages / name, assets / name)
            require(sha256(assets / name) == verified[name], "Archive changed while staging release")
        (assets / "COMMIT-LOG.txt").write_text(log, encoding="utf-8")
        sums = "".join(f"{sha256(p)}  {p.name}\n" for p in sorted(assets.iterdir()))
        (assets / "SHA256SUMS").write_text(sums, encoding="utf-8")
        (staging / "body.md").write_text(body, encoding="utf-8")
        require(not output.exists() and not output.is_symlink(), "Release output appeared while staging")
        staging.replace(output)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path); parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-commit", required=True); parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True, help="GitHub owner/repo used for release download links")
    parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        prepare_release(args.input, args.output, args.source_root, args.source_commit, args.tag, args.repository)
    except (OSError, ValueError, KeyError, TypeError, struct.error, zipfile.BadZipFile, tarfile.TarError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Release verification failed: {error}\n")


if __name__ == "__main__":
    main()
