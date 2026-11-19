"""Extract and smoke-test a native archive using only temporary synthetic data."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import zipfile
import zlib

from build_archive import PackageError, sha256


def relative_path(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if not name or path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name:
        raise PackageError(f"Unsafe archive pathname: {name!r}")
    return path


def extract_archive(archive: Path, destination: Path) -> Path:
    names = set()
    def extract(name, size, directory, mode, stream):
        path = relative_path(name)
        if path.as_posix() in names or size > 1024**3:
            raise PackageError("Duplicate archive member or unexpectedly large file")
        names.add(path.as_posix()); target = destination.joinpath(*path.parts)
        if directory:
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            with target.open("xb") as output: shutil.copyfileobj(stream, output)
            target.chmod(mode & 0o777)
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as source:
            for entry in source.infolist():
                mode = entry.external_attr >> 16
                if stat.S_ISLNK(mode): raise PackageError("Archive symlinks are not supported")
                with source.open(entry) as stream:
                    extract(entry.filename, entry.file_size, entry.is_dir(), mode or 0o644, stream)
    else:
        with tarfile.open(archive, "r:gz") as source:
            for entry in source:
                if not (entry.isfile() or entry.isdir()): raise PackageError("Archive links/devices are not supported")
                extract(entry.name, entry.size, entry.isdir(), entry.mode, source.extractfile(entry) if entry.isfile() else None)
    roots = {relative_path(name).parts[0] for name in names}
    if len(roots) != 1: raise PackageError("Expected one application directory in the archive")
    return destination / roots.pop()


def png_dimensions(path: Path) -> tuple[int, int]:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n": raise PackageError("Smoke output is not a PNG")
    offset = 8; dimensions = None; image_data = False
    while offset + 12 <= len(data):
        size = struct.unpack_from(">I", data, offset)[0]; end = offset + 12 + size
        if end > len(data): raise PackageError("Truncated smoke PNG")
        kind = data[offset + 4:offset + 8]; payload = data[offset + 8:end - 4]
        if zlib.crc32(kind + payload) != struct.unpack_from(">I", data, end - 4)[0]:
            raise PackageError("Invalid smoke PNG checksum")
        if kind == b"IHDR" and offset == 8 and size == 13:
            dimensions = struct.unpack_from(">II", payload)
        if kind == b"IDAT": image_data = True
        if kind == b"IEND":
            if end == len(data) and dimensions and min(dimensions) > 0 and image_data and size == 0:
                return dimensions
            break
        offset = end
    raise PackageError("Smoke PNG is incomplete")


def windows_dll_preflight(bundle: Path, names: list[str], environment: dict, output: Path) -> None:
    # DLLs loaded by ctypes remain process-loaded on Windows. Use a child so
    # its exit releases file locks before the temporary bundle is removed.
    check = "import ctypes,os,sys\nwith os.add_dll_directory(sys.argv[1]):\n for path in sys.argv[2:]: ctypes.WinDLL(path)\n"
    with (output / "dll-preflight.log").open("wb") as log:
        result = subprocess.run([sys.executable, "-c", check, str(bundle), *(str(bundle / name) for name in names)],
                                env=environment, stdout=log, stderr=subprocess.STDOUT, timeout=30, check=False)
    if result.returncode: raise PackageError("Bundled DLL loading failed; inspect dll-preflight.log")


def smoke(archive: Path, output: Path, timeout: int) -> dict:
    output.mkdir(parents=True, exist_ok=True)
    report = {"archive": archive.name, "archive_sha256": sha256(archive), "status": "failed", "synthetic_data_only": True}
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="recordbox-smoke-") as temporary:
            root = Path(temporary); bundle = extract_archive(archive, root / "unpacked application")
            manifests = list(bundle.rglob("runtime-manifest.json"))
            if len(manifests) != 1: raise PackageError("Expected one runtime manifest")
            manifest = json.loads(manifests[0].read_text(encoding="utf-8"))
            expected_platform = {"darwin": "macos", "win32": "windows"}.get(sys.platform, "linux")
            if manifest["target"]["platform"] != expected_platform:
                raise PackageError("Run the smoke test on the archive's native operating system")
            expected_files = {entry["path"] for entry in manifest["files"]} | {manifests[0].relative_to(bundle).as_posix()}
            if {path.relative_to(bundle).as_posix() for path in bundle.rglob("*") if path.is_file()} != expected_files:
                raise PackageError("Archive contains files outside its runtime manifest")
            for entry in manifest["files"]:
                path = bundle.joinpath(*relative_path(entry["path"]).parts)
                if path.stat().st_size != entry["size"] or sha256(path) != entry["sha256"]:
                    raise PackageError(f"Packaged file does not match its manifest: {entry['path']}")
            executable = bundle.joinpath(*relative_path(manifest["entrypoint"]).parts)
            screenshot = root / "synthetic-window.png"
            # In screenshot mode the existing app skips library sync and model
            # downloads unless one of these inherited research flags opts in.
            environment = {key: value for key, value in os.environ.items() if not key.startswith("PREPDECK_")}
            for key in ("LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"):
                environment.pop(key, None)
            if expected_platform == "windows":
                system_root = Path(os.environ["SystemRoot"])
                environment["PATH"] = os.pathsep.join(map(str, (system_root / "System32", system_root)))
                windows_dll_preflight(bundle, manifest["runtime"]["bundled_libraries"], environment, output)
                report["bundled_dll_load_preflight"] = "passed"
            environment["PREPDECK_DATA"] = str(root / "synthetic-data")
            with (output / "application.log").open("wb") as log:
                try:
                    result = subprocess.run([str(executable), "--screenshot", str(screenshot)], cwd=root, env=environment,
                                            stdout=log, stderr=subprocess.STDOUT, timeout=timeout, check=False)
                except subprocess.TimeoutExpired as error:
                    raise PackageError(f"Native screenshot smoke exceeded {timeout} seconds") from error
            if result.returncode: raise PackageError(f"Native screenshot smoke exited {result.returncode}; inspect application.log")
            width, height = png_dimensions(screenshot)
            shutil.copyfile(screenshot, output / "synthetic-window.png")
            report.update(status="passed", source_commit=manifest["source"]["commit"], target=manifest["target"], screenshot={"width": width, "height": height})
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        report["elapsed_seconds"] = round(time.monotonic() - started, 2)
        (output / "smoke.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True); parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=180); args = parser.parse_args()
    if args.timeout < 1: parser.error("--timeout must be positive")
    try: result = smoke(args.archive.resolve(), args.output.resolve(), args.timeout)
    except (PackageError, OSError, KeyError, ValueError) as error: parser.exit(1, f"Smoke failed: {error}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__": main()
