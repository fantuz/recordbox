"""Package an already-built native recordbox binary; Python 3.11+, no pip packages."""
from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import gzip
import hashlib
import json
from pathlib import Path
import plistlib
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile

import directml_runtime


class PackageError(ValueError):
    pass


def cuda_platforms(version):
    # Older release notes and manifests describe only the four original archives.
    numbers = version_tuple(re.split(r"[-+]", version, maxsplit=1)[0])
    return ("windows", "linux") if numbers >= (0, 5, 5) else ()


def has_directml(version):
    return version_tuple(re.split(r"[-+]", version, maxsplit=1)[0]) >= (0, 5, 6)


def release_archives(version):
    targets = (("macos", "arm64"), ("macos", "x86_64"), ("windows", "x86_64"), ("linux", "x86_64"))
    variants = [(platform, architecture, "") for platform, architecture in targets]
    variants.extend((platform, "x86_64", "cuda12") for platform in cuda_platforms(version))
    if has_directml(version):
        variants.append(("windows", "x86_64", "directml"))
    if version_tuple(re.split(r"[-+]", version, maxsplit=1)[0]) >= (0, 6, 0):
        variants.append(("linux", "x86_64", "vulkan"))
    for platform, architecture, variant in variants:
        suffix = f"-{variant}" if variant else ""
        ext = ".tar.gz" if platform == "linux" else ".zip"
        yield platform, architecture, variant, f"Recordbox-{version}-{platform}-{architecture}{suffix}{ext}"


def check_directml_binary(binary):
    if b"recordbox_htdemucs" not in binary.read_bytes():
        raise PackageError("DirectML package requires the compiled HTDemucs ONNX network")


def check_vulkan_binary(binary, source_root):
    shader = (source_root / "app/src/stems/vulkan/linear.wgsl").read_bytes()
    if not shader or shader not in binary.read_bytes():
        raise PackageError("The Vulkan build must contain the HTDemucs projection shader from this checkout")


CUDA_LIBRARIES = {
    "windows": {"nvcuda.dll", "cudart64_12.dll", "cublas64_12.dll", "cublaslt64_12.dll",
                "curand64_10.dll", "nvrtc64_120_0.dll"},
    "linux": {"libcuda.so.1", "libcudart.so.12", "libcublas.so.12", "libcublasLt.so.12",
              "libcurand.so.10", "libnvrtc.so.12"},
}


def check_cuda_binary(binary, dependencies, platform):
    names = set(dependencies)
    required = ({"nvcuda.dll", "cublas64_12.dll"} if platform == "windows"
                else {"libcuda.so.1", "libcublas.so.12"})
    code = binary.read_bytes()
    markers = (b".target sm_61", b".entry conv1d_f32(", b".entry softmax_f32(")
    if not required <= names or not all(marker in code for marker in markers):
        raise PackageError("CUDA archive requires NVIDIA driver, cuBLAS 12 and embedded Pascal FP32 kernels")


def read_release_version(source_root: Path) -> str:
    """Check the release marker and both Cargo declarations without building."""
    try:
        marker = (source_root / "VERSION").read_text(encoding="utf-8").strip()
        package = tomllib.loads((source_root / "app/Cargo.toml").read_text(encoding="utf-8"))["package"]
        locked = tomllib.loads((source_root / "app/Cargo.lock").read_text(encoding="utf-8")).get("package", [])
        version = package["version"]
    except (OSError, KeyError, tomllib.TOMLDecodeError) as error:
        raise PackageError(f"Missing or invalid release version metadata: {error}") from error
    number = r"(?:0|[1-9][0-9]*)"
    identifier = r"[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*"
    pattern = rf"{number}\.{number}\.{number}(?:-{identifier})?(?:\+{identifier})?"
    if not isinstance(version, str) or not re.fullmatch(pattern, version):
        raise PackageError("Cargo package version is not a safe release version")
    prerelease = version.split("+", 1)[0].partition("-")[2]
    if any(part.isdigit() and len(part) > 1 and part.startswith("0") for part in prerelease.split(".")):
        raise PackageError("Cargo package version is not a safe release version: numeric prerelease has a leading zero")
    if package.get("name") != "recordbox":
        raise PackageError("Expected the Cargo package to be named recordbox")
    if marker != version:
        raise PackageError(f"VERSION {marker!r} does not match Cargo.toml version {version!r}")
    local = [entry for entry in locked if entry.get("name") == "recordbox" and "source" not in entry]
    if len(local) != 1:
        raise PackageError("Cargo.lock must contain exactly one local recordbox package")
    if local[0].get("version") != version:
        raise PackageError(f"Cargo.lock recordbox version {local[0].get('version')!r} does not match VERSION {version!r}")
    return version


@dataclass
class BinaryInfo:
    architecture: str
    dependencies: dict[str, str | None] = field(default_factory=dict)
    minimum_macos: str | None = None
    abi_versions: dict[str, str] = field(default_factory=dict)


def run_tool(*args: str) -> str:
    try:
        result = subprocess.run(args, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    except FileNotFoundError as error:
        raise PackageError(f"Required native inspection tool is missing: {args[0]}") from error
    if result.returncode:
        raise PackageError(f"{' '.join(args)} failed:\n{result.stdout}")
    return result.stdout


def parse_macos(architectures: str, libraries: str, commands: str) -> BinaryInfo:
    arches = architectures.strip().split()
    if len(arches) != 1 or arches[0] not in {"arm64", "x86_64"}:
        raise PackageError("Build a separate thin arm64 or x86_64 binary for each macOS archive")
    dependencies = {}
    for line in libraries.splitlines()[1:]:
        match = re.match(r"\s+(.+?)\s+\(compatibility version ", line)
        if match:
            path = match[1]
            dependencies[path] = path
    minimum = re.search(r"^\s*minos\s+([0-9.]+)\s*$", commands, re.M)
    if minimum is None:
        minimum = re.search(r"cmd LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version ([0-9.]+)", commands)
    if minimum is None or not dependencies:
        raise PackageError("otool did not report a macOS deployment target and linked libraries")
    return BinaryInfo(arches[0], dependencies, minimum[1])


def parse_windows(headers: str, dependents: str) -> BinaryInfo:
    if re.search(r"\b8664 machine \(x64\)", headers, re.I):
        architecture = "x86_64"
    elif re.search(r"\bAA64 machine \(ARM64\)", headers, re.I):
        architecture = "arm64"
    else:
        raise PackageError("dumpbin did not report an x64 or ARM64 PE binary")
    dependencies = {m[1].lower(): None for m in re.finditer(r"^\s+([\w.+-]+\.dll)\s*$", dependents, re.M | re.I)}
    return BinaryInfo(architecture, dependencies)


def version_tuple(value: str) -> tuple[int, ...]:
    return tuple(int(part) for part in value.split("."))


def parse_linux(headers: str, libraries: str, versions: str) -> BinaryInfo:
    machine = re.search(r"^\s*Machine:\s+(.+)$", headers, re.M)
    architecture = {"Advanced Micro Devices X86-64": "x86_64", "AArch64": "arm64"}.get(machine[1].strip() if machine else "")
    if architecture is None:
        raise PackageError("readelf did not report an x86_64 or AArch64 ELF binary")
    dependencies = {}
    for line in libraries.splitlines():
        match = re.match(r"\s*(\S+)\s+=>\s+(not found|\S+)", line)
        if match:
            dependencies[match[1]] = None if match[2] == "not found" else match[2]
        elif match := re.match(r"\s*(/\S+)\s+\(", line):
            dependencies[Path(match[1]).name] = match[1]
    abi = {}
    for name, version in re.findall(r"\b(GLIBC|GLIBCXX|CXXABI)_([0-9]+(?:\.[0-9]+)+)\b", versions):
        if name not in abi or version_tuple(version) > version_tuple(abi[name]):
            abi[name] = version
    return BinaryInfo(architecture, dependencies, abi_versions=abi)


def inspect_binary(path: Path, platform: str) -> BinaryInfo:
    # Commands inspect the actual built file, rather than trusting its filename.
    if platform == "macos":
        return parse_macos(run_tool("lipo", "-archs", str(path)), run_tool("otool", "-L", str(path)), run_tool("otool", "-l", str(path)))
    if platform == "windows":
        return parse_windows(run_tool("dumpbin", "/headers", str(path)), run_tool("dumpbin", "/dependents", str(path)))
    return parse_linux(run_tool("readelf", "-h", str(path)), run_tool("ldd", str(path)), run_tool("readelf", "--version-info", str(path)))


# OS DLLs stay external. VC++ redistributable DLLs deliberately are not on this list.
WINDOWS_SYSTEM_DLLS = set("""
advapi32 avrt bcrypt bcryptprimitives cfgmgr32 combase comctl32 comdlg32 crypt32
cryptbase d2d1 d3d11 d3d12 d3dcompiler_47 dbghelp dcomp dnsapi dsound dwmapi dwrite
dxcore dxgi gdi32 gdi32full hid imm32 iphlpapi kernel32 kernelbase mf mfplat mfreadwrite
mfuuid mmdevapi msimg32 msvcrt ncrypt netapi32 ntdll ole32 oleacc oleaut32 opengl32 powrprof
propsys psapi rpcrt4 sechost secur32 setupapi shell32 shcore shlwapi synchronization
ucrtbase uiautomationcore user32 userenv uxtheme version winhttp wininet winmm winspool wintrust
wldap32 ws2_32 wtsapi32
""".split())


def system_dependency(name: str, resolved: str | None, platform: str) -> bool:
    if platform == "windows":
        return name.lower().startswith(("api-ms-win-", "ext-ms-win-")) or name.lower().removesuffix(".dll") in WINDOWS_SYSTEM_DLLS
    if resolved is None or "/../" in resolved or "/./" in resolved:
        return False
    roots = ("/System/Library/", "/usr/lib/") if platform == "macos" else ("/lib/", "/lib64/", "/usr/lib/", "/usr/lib64/")
    return resolved.startswith(roots)


def library_candidates(files: list[Path], directories: list[Path], platform: str) -> dict[str, Path]:
    if platform == "macos" and (files or directories):
        raise PackageError("macOS packaging requires system-only dylibs; rebuild without external OpenSSL")
    for directory in directories:
        if not directory.is_dir():
            raise PackageError(f"Runtime directory does not exist: {directory}")
        files = [*files, *(p for p in directory.iterdir() if p.is_file() and (p.suffix.lower() == ".dll" if platform == "windows" else ".so" in p.name))]
    candidates = {}
    for path in files:
        if not path.is_file() or not re.fullmatch(r"[A-Za-z0-9_.+-]+", path.name):
            raise PackageError(f"Invalid runtime library: {path}")
        key = path.name.lower() if platform == "windows" else path.name
        if key in candidates and candidates[key].resolve() != path.resolve():
            raise PackageError(f"Ambiguous runtime library: {path.name}")
        candidates[key] = path
    return candidates


def runtime_closure(binary: Path, platform: str, architecture: str, candidates: dict[str, Path], cuda12=False, dynamic_libraries=None) -> tuple[BinaryInfo, dict[str, Path], list[str], dict[str, str]]:
    main = inspect_binary(binary, platform)
    bundled = dict(dynamic_libraries or {})
    pending = [(binary, main)] + [(path, inspect_binary(path, platform)) for path in bundled.values()]
    system = set(); abi = {}
    while pending:
        path, info = pending.pop()
        if info.architecture != architecture:
            raise PackageError(f"Architecture mismatch: {path.name} is {info.architecture}, requested {architecture}")
        for name, version in info.abi_versions.items():
            if name not in abi or version_tuple(version) > version_tuple(abi[name]):
                abi[name] = version
        for name, resolved in info.dependencies.items():
            key = name.lower() if platform == "windows" else name
            if cuda12 and key in CUDA_LIBRARIES[platform]:
                system.add(key)
            elif key in candidates and not system_dependency(name, resolved, platform):
                if key not in bundled:
                    dependency = candidates[key]
                    bundled[key] = dependency
                    if len(bundled) > 128:
                        raise PackageError("Unexpectedly large native runtime dependency graph")
                    pending.append((dependency, inspect_binary(dependency, platform)))
            elif system_dependency(name, resolved, platform):
                system.add(name if platform == "windows" else Path(name).name)
            else:
                raise PackageError(f"Unresolved/non-system dependency {name!r}; provide its exact runtime directory or library")
    return main, bundled, sorted(system), abi


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_file(path: Path, data: bytes, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data); path.chmod(mode)


def bundle_mp3_sources(source_root: Path, destination: Path, commit: str) -> None:
    locked = tomllib.loads((source_root / "app/Cargo.lock").read_text(encoding="utf-8"))["package"]
    names = {"mp3lame-encoder", "mp3lame-sys"}
    packages = [p for p in locked if p["name"] in names]
    if not packages:
        return
    if len(packages) != 2 or {p["name"] for p in packages} != names:
        raise PackageError("Expected one locked version of each LAME component")
    sources = source_root / "packaging/lame"
    for p in packages:
        if p.get("source") != "registry+https://github.com/rust-lang/crates.io-index" or not re.fullmatch(r"[0-9][0-9A-Za-z.+-]*", p["version"]):
            raise PackageError("MP3 codec source does not match the crates.io distribution recipe")
        name = f"{p['name']}-{p['version']}"
        archive = sources / f"{name}.crate"
        if not archive.is_file() or sha256(archive) != p.get("checksum"):
            raise PackageError(f"Missing or mismatched locked codec source {archive.name}")
        write_file(destination / archive.name, archive.read_bytes())
        if p["name"] == "mp3lame-sys":
            with tarfile.open(archive, "r:gz") as src:
                for item, notice in (("LICENSE", "LGPL-3.0.txt"), ("lame-4.0/COPYING", "LAME-COPYING.txt"), ("lame-4.0/LICENSE", "LAME-LICENSE.txt")):
                    write_file(destination / notice, src.extractfile(f"{name}/{item}").read())
    for notice in ("GPL-3.0.txt", "README.md"):
        write_file(destination / notice, (sources / notice).read_bytes())
    write_file(destination / "recordbox-source.txt", f"https://github.com/fantuz/recordbox/archive/{commit}.tar.gz\n".encode())


def distribution_mode(path: Path, platform: str) -> int:
    """Archive metadata; Windows stat/chmod do not describe POSIX permissions."""
    if path.is_dir(): return 0o755
    if platform == "windows": return 0o755 if path.name.lower() == "recordbox.exe" else 0o644
    return stat.S_IMODE(path.stat().st_mode)


def archive_bundle(bundle: Path, archive: Path, platform: str) -> None:
    paths = [bundle, *sorted(bundle.rglob("*"))]
    if any(path.is_symlink() for path in paths):
        raise PackageError("Bundle symlinks are not supported")
    if platform != "linux":
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
            for path in paths:
                name = path.relative_to(bundle.parent).as_posix() + ("/" if path.is_dir() else "")
                entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0)); entry.create_system = 3
                entry.external_attr = ((stat.S_IFDIR if path.is_dir() else stat.S_IFREG) | distribution_mode(path, platform)) << 16
                entry.compress_type = zipfile.ZIP_DEFLATED
                output.writestr(entry, b"" if path.is_dir() else path.read_bytes())
    else:
        with archive.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as output:
            for path in paths:
                entry = tarfile.TarInfo(path.relative_to(bundle.parent).as_posix()); entry.mtime = 0
                entry.mode = distribution_mode(path, platform)
                if path.is_dir():
                    entry.type = tarfile.DIRTYPE; output.addfile(entry)
                else:
                    entry.size = path.stat().st_size
                    with path.open("rb") as stream: output.addfile(entry, stream)


LINUX_LAUNCHER = b'''#!/bin/sh
set -eu
bundle_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export LD_LIBRARY_PATH="$bundle_dir/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$bundle_dir/bin/recordbox" "$@"
'''


def build_archive(binary: Path, source_root: Path, output: Path, platform: str, architecture: str, source_commit: str,
                  runtime_files: list[Path] | None = None, runtime_dirs: list[Path] | None = None,
                  runtime_notices: list[Path] | None = None, cuda12=False, directml=None, vulkan=False) -> dict:
    if platform not in {"macos", "windows", "linux"} or architecture not in {"arm64", "x86_64"}:
        raise PackageError("Unsupported platform or architecture")
    if not re.fullmatch(r"(?:[a-fA-F0-9]{40}|[a-fA-F0-9]{64})", source_commit):
        raise PackageError("Provide the full Git source commit")
    if not binary.is_file() or binary.is_symlink():
        raise PackageError("The built executable must be a regular file")
    manifest_path = source_root / "app/Cargo.toml"
    version = read_release_version(source_root)
    candidates = library_candidates(runtime_files or [], runtime_dirs or [], platform)
    if vulkan:
        if platform != "linux" or architecture != "x86_64" or cuda12 or directml is not None:
            raise PackageError("The Vulkan preview requires a separate Linux x86_64 build")
        check_vulkan_binary(binary, source_root)
    if cuda12 and (platform not in cuda_platforms(version) or architecture != "x86_64"):
        raise PackageError("Unsupported CUDA package target")
    dynamic = {}
    if directml is not None:
        if cuda12 or platform != "windows" or architecture != "x86_64" or not has_directml(version):
            raise PackageError("Unsupported DirectML package target")
        directml_runtime.verify(directml)
        check_directml_binary(binary)
        dynamic = {name: directml / name for name in directml_runtime.LIBRARIES}
        candidates.update(dynamic)
    info, bundled, system, abi = runtime_closure(binary, platform, architecture, candidates, cuda12, dynamic)
    if cuda12:
        check_cuda_binary(binary, info.dependencies, platform)
    label = f"Recordbox-{version}-{platform}-{architecture}"
    if vulkan:
        label += "-vulkan"
    elif cuda12:
        label += "-cuda12"
    elif directml:
        label += "-directml"
    bundle_name = "Recordbox.app" if platform == "macos" else label
    archive_name = label + (".tar.gz" if platform == "linux" else ".zip")
    output.mkdir(parents=True, exist_ok=True)
    for name in (bundle_name, archive_name, archive_name + ".sha256"):
        if (output / name).exists() or (output / name).is_symlink():
            raise PackageError(f"Output already exists; use a fresh output directory: {name}")
    with tempfile.TemporaryDirectory(prefix=".recordbox-package-", dir=output) as temporary:
        staging = Path(temporary); bundle = staging / bundle_name; bundle.mkdir()
        resources = bundle / "Contents/Resources" if platform == "macos" else bundle
        executable = "Contents/MacOS/recordbox" if platform == "macos" else "recordbox.exe" if platform == "windows" else "bin/recordbox"
        write_file(bundle / executable, binary.read_bytes(), 0o755)
        if platform == "macos":
            numeric_version = version.split("-", 1)[0].split("+", 1)[0]
            plist = {"CFBundleName": "Recordbox", "CFBundleDisplayName": "Recordbox", "CFBundleIdentifier": "io.github.adamsardo.recordbox",
                     "CFBundleExecutable": "recordbox", "CFBundlePackageType": "APPL", "CFBundleShortVersionString": numeric_version,
                     "CFBundleVersion": numeric_version, "LSMinimumSystemVersion": info.minimum_macos,
                     "LSApplicationCategoryType": "public.app-category.music", "NSHighResolutionCapable": True}
            write_file(bundle / "Contents/Info.plist", plistlib.dumps(plist))
            write_file(bundle / "Contents/PkgInfo", b"APPL????")
        if platform == "linux":
            write_file(bundle / "recordbox", LINUX_LAUNCHER, 0o755)
            (bundle / "lib").mkdir()
        for name, path in sorted(bundled.items()):
            write_file(bundle / ("lib" if platform == "linux" else "") / name, path.read_bytes(), 0o644)
        write_file(resources / "LICENSE.txt", (source_root / "LICENSE").read_bytes())
        if directml:
            write_file(resources / "DIRECTML.md", (source_root / "docs/DIRECTML.md").read_bytes())
            for name in directml_runtime.FILES:
                if name not in dynamic:
                    write_file(resources / "runtime-notices" / name, (directml / name).read_bytes())
        if cuda12:
            write_file(resources / "CUDA12.md", (source_root / "docs/CUDA12.md").read_bytes())
        if vulkan:
            write_file(resources / "VULKAN.md", (source_root / "docs/VULKAN.md").read_bytes())
        write_file(resources / "Inter-NOTICE.txt", (source_root / "app/assets/fonts/README.txt").read_bytes())
        write_file(resources / "Inter-OFL.txt", (source_root / "packaging/Inter-OFL.txt").read_bytes())
        bundle_mp3_sources(source_root, resources / "runtime-notices/LAME", source_commit.lower())
        for index, notice in enumerate(runtime_notices or [], 1):
            if not notice.is_file() or not re.fullmatch(r"[A-Za-z0-9_.+-]+", notice.name):
                raise PackageError(f"Invalid runtime notice: {notice}")
            write_file(resources / "runtime-notices" / f"{index}-{notice.name}", notice.read_bytes())
        write_file(resources / "DISTRIBUTION.txt", b"Built from the source revision in runtime-manifest.json. No signing or notarization step was performed by this packager.\nNo user library, audio, model checkpoints or cached data are included.\n")
        files = []
        for path in sorted(bundle.rglob("*")):
            if path.is_dir(): path.chmod(0o755)
            else: files.append({"path": path.relative_to(bundle).as_posix(), "sha256": sha256(path), "size": path.stat().st_size, "mode": f"{distribution_mode(path, platform):04o}"})
        bundle.chmod(0o755)
        metadata = {"schema": 1, "product": "Recordbox", "version": version,
                    "source": {"commit": source_commit.lower(), "cargo_manifest_sha256": sha256(manifest_path), "cargo_lock_sha256": sha256(source_root / "app/Cargo.lock")},
                    "target": {"platform": platform, "architecture": architecture, "minimum_macos": info.minimum_macos, "abi_versions": abi},
                    "entrypoint": executable if platform != "linux" else "recordbox",
                    "runtime": {"system_dependencies": system, "bundled_libraries": sorted(bundled)},
                    "distribution": {"signing_performed": False, "notarization_performed": False}, "files": files}
        if directml:
            metadata["target"]["stems"] = "directml-radeon"
        if cuda12:
            metadata["target"]["stems"] = "cuda12-sm61"
        if vulkan:
            metadata["target"]["stems"] = "vulkan"
        write_file(resources / "runtime-manifest.json", (json.dumps(metadata, indent=2, sort_keys=True) + "\n").encode())
        archive = staging / archive_name; archive_bundle(bundle, archive, platform)
        checksum = sha256(archive)
        shutil.move(str(bundle), output / bundle_name); archive.replace(output / archive_name)
        write_file(output / (archive_name + ".sha256"), f"{checksum}  {archive_name}\n".encode())
    return {"archive": str(output / archive_name), "sha256": checksum, "bundle": str(output / bundle_name), "manifest": metadata}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path); parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path); parser.add_argument("--platform", choices=["macos", "windows", "linux"])
    parser.add_argument("--arch", choices=["arm64", "x86_64"]); parser.add_argument("--source-commit")
    parser.add_argument("--directml-runtime", type=Path, help="Package Windows Radeon STEMS with the pinned runtime directory")
    parser.add_argument("--cuda12", action="store_true", help="Package the NVIDIA CUDA 12 STEMS variant")
    parser.add_argument("--vulkan", action="store_true", help="Package the experimental Linux Vulkan STEMS build")
    parser.add_argument("--check-version", action="store_true", help="Validate VERSION and Cargo versions without packaging")
    parser.add_argument("--tag", help="With --check-version, require the exact v<VERSION> release tag")
    parser.add_argument("--runtime-library", action="append", default=[], type=Path); parser.add_argument("--runtime-dir", action="append", default=[], type=Path)
    parser.add_argument("--runtime-notice", action="append", default=[], type=Path, help="License/notice accompanying a bundled runtime or embedded asset")
    args = parser.parse_args()
    packaging = (args.binary, args.output, args.platform, args.arch, args.source_commit)
    if args.tag is not None and not args.check_version: parser.error("--tag requires --check-version")
    if args.check_version and (any(value is not None for value in packaging) or args.runtime_library or args.runtime_dir or args.runtime_notice or args.cuda12 or args.directml_runtime or args.vulkan):
        parser.error("--check-version cannot be combined with packaging arguments")
    if not args.check_version and any(value is None for value in packaging):
        parser.error("packaging requires --binary, --output, --platform, --arch and --source-commit")
    try:
        if args.check_version:
            version = read_release_version(args.source_root.resolve())
            if args.tag is not None and not re.fullmatch("v" + re.escape(version), args.tag):
                raise PackageError(f"Release tag must be v{version}, received {args.tag!r}")
            print(json.dumps({"status": "passed", "version": version, "tag": args.tag}))
            return
        result = build_archive(args.binary.resolve(), args.source_root.resolve(), args.output.resolve(), args.platform, args.arch, args.source_commit, args.runtime_library, args.runtime_dir, args.runtime_notice, args.cuda12, args.directml_runtime, args.vulkan)
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, f"Packaging failed: {error}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__": main()
