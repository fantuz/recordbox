"""Small fixtures exercise packaging/extraction without compiling Rust or opening a GUI."""
from pathlib import Path
import json
import os
import plistlib
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile
import zlib

import build_archive as package
import smoke_app


def png():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\0\xff\0\0\xff")) + chunk(b"IEND", b"")


class Archives(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="recordbox-package-test-")
        self.addCleanup(self.temporary.cleanup); self.root = Path(self.temporary.name)
        self.source = self.root / "source with spaces 日本"; self.source.mkdir()
        (self.source / "app/assets/fonts").mkdir(parents=True)
        (self.source / "app/Cargo.toml").write_text('[package]\nname="recordbox"\nversion="0.1.0"\n', encoding="utf-8")
        (self.source / "app/Cargo.lock").write_text("# fixture lock\n")
        (self.source / "LICENSE").write_text("MIT fixture license\n")
        (self.source / "app/assets/fonts/README.txt").write_text("Inter fixture font notice\n")
        (self.source / "packaging").mkdir(); (self.source / "packaging/Inter-OFL.txt").write_text("SIL Open Font License fixture\n")
        # Nothing except the explicit binary/notices may enter an archive.
        for name in ("master.db", "personal-song.mp3", "checkpoint.th", "private-token.txt"):
            (self.source / name).write_text("private fixture - exclude\n")
        self.binary = self.source / "recordbox"; self.binary.write_bytes(b"native executable fixture"); self.binary.chmod(0o755)
        self.output = self.root / "output"; self.commit = "a" * 40

    def build(self, platform="macos", architecture="arm64", inspector=None, **kwargs):
        info = package.BinaryInfo(architecture, {"/usr/lib/libSystem.B.dylib": "/usr/lib/libSystem.B.dylib"}, "11.0") if platform == "macos" else package.BinaryInfo(architecture)
        with patch.object(package, "inspect_binary", side_effect=inspector or (lambda *_: info)):
            return package.build_archive(self.binary, self.source, self.output, platform, architecture, self.commit, **kwargs)

    def test_macos_bundle_zip_manifest_permissions_and_exclusions(self):
        result = self.build(); bundle = Path(result["bundle"])
        plist = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
        self.assertEqual((plist["CFBundleExecutable"], plist["LSMinimumSystemVersion"], plist["CFBundleShortVersionString"]), ("recordbox", "11.0", "0.1.0"))
        manifest = json.loads((bundle / "Contents/Resources/runtime-manifest.json").read_text())
        self.assertEqual(manifest["source"]["commit"], self.commit)
        self.assertFalse(manifest["distribution"]["signing_performed"])
        self.assertFalse(manifest["distribution"]["notarization_performed"])
        self.assertEqual((bundle / "Contents/MacOS/recordbox").read_bytes(), self.binary.read_bytes())
        with zipfile.ZipFile(result["archive"]) as archive:
            self.assertEqual(archive.testzip(), None)
            self.assertEqual(stat.S_IMODE(archive.getinfo("Recordbox.app/Contents/MacOS/recordbox").external_attr >> 16), 0o755)
            self.assertEqual(stat.S_IMODE(archive.getinfo("Recordbox.app/Contents/Info.plist").external_attr >> 16), 0o644)
            self.assertTrue(all(name.startswith("Recordbox.app/") for name in archive.namelist()))
            self.assertFalse(any(private in name for name in archive.namelist() for private in ("master.db", ".mp3", ".th", "token", "target")))
        self.assertEqual(package.sha256(Path(result["archive"])), result["sha256"])
        self.assertEqual(Path(result["archive"] + ".sha256").read_text(), result["sha256"] + "  Recordbox-0.1.0-macos-arm64.zip\n")

    def test_macos_x86_64_is_a_separate_archive(self):
        result = self.build(architecture="x86_64")
        self.assertTrue(result["archive"].endswith("macos-x86_64.zip"))
        self.assertEqual(result["manifest"]["target"]["architecture"], "x86_64")

    def test_windows_bundles_only_recursive_required_dlls_and_notice(self):
        runtime = self.root / "vcpkg"; runtime.mkdir()
        for name in ("libcrypto-3-x64.dll", "vcruntime140.dll", "libssl-3-x64.dll", "secret.mp3"):
            (runtime / name).write_bytes(name.encode())
        notice = self.root / "OpenSSL.txt"; notice.write_text("OpenSSL fixture notice")
        def inspect(path, _):
            dependencies = {"libcrypto-3-x64.dll": None, "kernel32.dll": None} if path == self.binary else {"vcruntime140.dll": None} if path.name.startswith("libcrypto") else {"api-ms-win-crt-runtime-l1-1-0.dll": None}
            return package.BinaryInfo("x86_64", dependencies)
        result = self.build("windows", "x86_64", inspect, runtime_dirs=[runtime], runtime_notices=[notice])
        bundle = Path(result["bundle"])
        self.assertEqual(result["manifest"]["runtime"]["bundled_libraries"], ["libcrypto-3-x64.dll", "vcruntime140.dll"])
        self.assertTrue((bundle / "runtime-notices/1-OpenSSL.txt").is_file())
        self.assertFalse((bundle / "libssl-3-x64.dll").exists())
        self.assertFalse((bundle / "secret.mp3").exists())

    def test_windows_missing_dll_fails_instead_of_using_path(self):
        with self.assertRaisesRegex(package.PackageError, "libcrypto"):
            self.build("windows", "x86_64", lambda *_: package.BinaryInfo("x86_64", {"libcrypto-3-x64.dll": None}))

    def test_runtime_library_architecture_must_match(self):
        library = self.root / "custom.dll"; library.write_bytes(b"fixture")
        def inspect(path, _):
            return package.BinaryInfo("x86_64", {"custom.dll": None}) if path == self.binary else package.BinaryInfo("arm64")
        with self.assertRaisesRegex(package.PackageError, "Architecture mismatch"):
            self.build("windows", "x86_64", inspect, runtime_files=[library])

    def test_linux_tar_launcher_modes_runtime_and_abi(self):
        library = self.root / "libcustom.so.1"; library.write_bytes(b"custom native library")
        def inspect(path, _):
            if path == self.binary:
                return package.BinaryInfo("x86_64", {"libcustom.so.1": str(library), "libc.so.6": "/lib/x86_64-linux-gnu/libc.so.6"}, abi_versions={"GLIBC": "2.31"})
            return package.BinaryInfo("x86_64", {"libc.so.6": "/lib/x86_64-linux-gnu/libc.so.6"}, abi_versions={"GLIBC": "2.35"})
        result = self.build("linux", "x86_64", inspect, runtime_files=[library])
        self.assertEqual(result["manifest"]["target"]["abi_versions"], {"GLIBC": "2.35"})
        with tarfile.open(result["archive"]) as archive:
            prefix = "Recordbox-0.1.0-linux-x86_64/"
            self.assertEqual(archive.getmember(prefix + "recordbox").mode, 0o755)
            self.assertEqual(archive.getmember(prefix + "bin/recordbox").mode, 0o755)
            self.assertEqual(archive.getmember(prefix + "lib/libcustom.so.1").mode, 0o644)
            self.assertTrue(all(entry.uid == entry.gid == entry.mtime == 0 for entry in archive))
            self.assertTrue(all(entry.isfile() or entry.isdir() for entry in archive))
        self.assertIn(b'"$bundle_dir/bin/recordbox" "$@"', (Path(result["bundle"]) / "recordbox").read_bytes())

    def test_non_system_dependency_is_rejected(self):
        for platform, dependency in (("macos", "/opt/homebrew/opt/openssl/lib/libcrypto.3.dylib"), ("macos", "@rpath/libcrypto.3.dylib"), ("linux", "/home/builder/libcustom.so")):
            with self.subTest(platform=platform, dependency=dependency), self.assertRaises(package.PackageError):
                self.build(platform, "arm64", lambda *_: package.BinaryInfo("arm64", {dependency: dependency}, "11.0"))

    def test_false_system_path_is_rejected(self):
        self.assertFalse(package.system_dependency("bad", "/usr/lib/../local/libevil.dylib", "macos"))
        self.assertFalse(package.system_dependency("vcruntime140.dll", None, "windows"))

    def test_unsafe_version_and_short_commit_are_rejected(self):
        (self.source / "app/Cargo.toml").write_text('[package]\nversion="../../private"\n')
        with self.assertRaisesRegex(package.PackageError, "safe release version"): self.build()
        self.commit = "abc123"
        with self.assertRaisesRegex(package.PackageError, "full Git source commit"): self.build()
        self.assertFalse(self.output.exists())

    def test_existing_output_is_preserved(self):
        result = self.build(); digest = package.sha256(Path(result["archive"]))
        with self.assertRaisesRegex(package.PackageError, "Output already exists"): self.build()
        self.assertEqual(package.sha256(Path(result["archive"])), digest)

    def test_repeated_packaging_is_reproducible(self):
        first = self.build(); self.output = self.root / "output-two"; second = self.build()
        self.assertEqual(first["sha256"], second["sha256"])

    def test_inspection_output_parsers(self):
        mac = package.parse_macos("arm64\n", "recordbox:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1345.120.2)\n", "    cmd LC_BUILD_VERSION\n    minos 11.0\n")
        self.assertEqual((mac.architecture, mac.minimum_macos), ("arm64", "11.0"))
        with self.assertRaises(package.PackageError): package.parse_macos("arm64 x86_64", "", "")
        win = package.parse_windows(" 8664 machine (x64)\n", "    KERNEL32.dll\n    libcrypto-3-x64.dll\n")
        self.assertEqual(set(win.dependencies), {"kernel32.dll", "libcrypto-3-x64.dll"})
        linux = package.parse_linux(" Machine: Advanced Micro Devices X86-64\n", " libasound.so.2 => /lib/x86_64-linux-gnu/libasound.so.2 (0x1)\n libcustom.so => not found\n", "Name: GLIBC_2.9\nName: GLIBC_2.35\nName: GLIBCXX_3.4.29")
        self.assertEqual(linux.abi_versions, {"GLIBC": "2.35", "GLIBCXX": "3.4.29"})
        self.assertIsNone(linux.dependencies["libcustom.so"])

    def test_native_tool_failure_has_actionable_message(self):
        with patch.object(package.subprocess, "run", side_effect=FileNotFoundError), self.assertRaisesRegex(package.PackageError, "Required native inspection tool"):
            package.run_tool("missing-inspector", "fixture")

    def test_extraction_refuses_traversal_and_symlinks(self):
        for name, mode in (("../private", stat.S_IFREG | 0o644), ("safe/link", stat.S_IFLNK | 0o777)):
            archive = self.root / "malformed.zip"
            with zipfile.ZipFile(archive, "w") as output:
                entry = zipfile.ZipInfo(name); entry.create_system = 3; entry.external_attr = mode << 16
                output.writestr(entry, b"outside")
            with self.assertRaises(package.PackageError): smoke_app.extract_archive(archive, self.root / "unpacked")
        self.assertFalse((self.root / "private").exists())

    def test_png_requires_complete_image_and_valid_crc(self):
        image = self.root / "image.png"; image.write_bytes(png())
        self.assertEqual(smoke_app.png_dimensions(image), (1, 1))
        image.write_bytes(png()[:-4])
        with self.assertRaises(package.PackageError): smoke_app.png_dimensions(image)
        data = bytearray(png()); data[30] ^= 1; image.write_bytes(data)
        with self.assertRaises(package.PackageError): smoke_app.png_dimensions(image)

    def test_windows_dll_preflight_uses_a_short_lived_process(self):
        environment = {"PATH": "system-only"}
        with patch.object(smoke_app.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            smoke_app.windows_dll_preflight(self.source, ["libcrypto-3-x64.dll"], environment, self.root)
        command = run.call_args.args[0]
        self.assertEqual(command[:2], [sys.executable, "-c"])
        self.assertIn("ctypes.WinDLL", command[2])
        self.assertEqual(command[-1], str(self.source / "libcrypto-3-x64.dll"))
        self.assertEqual(run.call_args.kwargs["env"], environment)
        with patch.object(smoke_app.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)), self.assertRaisesRegex(package.PackageError, "Bundled DLL loading failed"):
            smoke_app.windows_dll_preflight(self.source, ["missing.dll"], environment, self.root)

    @unittest.skipUnless(sys.platform.startswith("linux"), "The fixture launcher is a Linux script")
    def test_smoke_runs_extracted_entrypoint_with_isolated_data(self):
        self.binary.write_text("#!/usr/bin/env python3\nimport os,sys\nfrom pathlib import Path\nassert 'PREPDECK_SHOT_SYNC' not in os.environ\nassert 'PREPDECK_SHOT_STEMS' not in os.environ\np=Path(os.environ['PREPDECK_DATA']);p.mkdir();(p/'synthetic.db').write_text('fixture')\nPath(sys.argv[2]).write_bytes(" + repr(png()) + ")\n")
        result = self.build("linux", "x86_64")
        private = self.root / "private-data"; private.mkdir(); sentinel = private / "library.db"; sentinel.write_text("keep")
        with patch.dict(os.environ, {"PREPDECK_DATA": str(private), "PREPDECK_SHOT_SYNC": "1", "PREPDECK_SHOT_STEMS": "1"}):
            report = smoke_app.smoke(Path(result["archive"]), self.root / "smoke", 20)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(sentinel.read_text(), "keep")
        self.assertFalse((private / "synthetic.db").exists())


if __name__ == "__main__": unittest.main()
