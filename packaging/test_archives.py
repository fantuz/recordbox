"""Small fixtures exercise packaging/extraction without compiling Rust or opening a GUI."""
from pathlib import Path
import json
import os
import plistlib
import shutil
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
        (self.source / "VERSION").write_text("0.1.0\n")
        (self.source / "app/Cargo.toml").write_text('[package]\nname="recordbox"\nversion="0.1.0"\n', encoding="utf-8")
        (self.source / "app/Cargo.lock").write_text('version = 4\n[[package]]\nname="recordbox"\nversion="0.1.0"\n')
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

    def test_locked_mp3_codec_sources_licenses_and_rebuild_instructions_ship_offline(self):
        sources = self.source / "packaging/lame"
        shutil.copytree(Path(package.__file__).parent / "lame", sources)
        archives = sorted(sources.glob("*.crate"))
        self.assertEqual(len(archives), 2)
        with (self.source / "app/Cargo.lock").open("a", encoding="utf-8") as lock:
            for archive in archives:
                name, version = archive.stem.rsplit("-", 1)
                lock.write(f'\n[[package]]\nname="{name}"\nversion="{version}"\n'
                           f'source="registry+https://github.com/rust-lang/crates.io-index"\nchecksum="{package.sha256(archive)}"\n')
        result = self.build()
        with zipfile.ZipFile(result["archive"]) as package_zip:
            location = "Recordbox.app/Contents/Resources/runtime-notices/LAME/"
            self.assertEqual(package_zip.read(location + "recordbox-source.txt").decode().strip(),
                             f"https://github.com/fantuz/recordbox/archive/{self.commit}.tar.gz")
            for archive in archives:
                self.assertEqual(package_zip.read(location + archive.name), archive.read_bytes())
            for notice in ("GPL-3.0.txt", "LGPL-3.0.txt", "LAME-COPYING.txt", "LAME-LICENSE.txt", "README.md"):
                self.assertTrue(package_zip.read(location + notice))
        archives[0].write_bytes(b"fixture substituted after compiling")
        self.output = self.root / "bad-output"
        with self.assertRaisesRegex(package.PackageError, "mismatched locked codec source"):
            self.build()
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

    def test_windows_reported_modes_are_normalized_in_zip_and_manifest(self):
        library = self.root / "libcrypto-3-x64.dll"; library.write_bytes(b"native DLL fixture")
        original_stat = Path.stat
        def windows_stat(path, *, follow_symlinks=True):
            result = original_stat(path, follow_symlinks=follow_symlinks); fields = list(result)
            kind = stat.S_IFMT(result.st_mode)
            reported = 0o777 if stat.S_ISDIR(result.st_mode) or path.name.lower() in {"recordbox", "recordbox.exe"} else 0o666
            fields[stat.ST_MODE] = kind | reported
            return os.stat_result(fields)
        def inspect(path, _):
            return package.BinaryInfo("x86_64", {library.name: None} if path == self.binary else {"kernel32.dll": None})
        with patch.object(Path, "stat", windows_stat):
            result = self.build("windows", "x86_64", inspect, runtime_files=[library])
        for entry in result["manifest"]["files"]:
            self.assertEqual(entry["mode"], "0755" if entry["path"] == "recordbox.exe" else "0644")
        with zipfile.ZipFile(result["archive"]) as archive:
            for entry in archive.infolist():
                expected = 0o755 if entry.is_dir() or Path(entry.filename).name == "recordbox.exe" else 0o644
                self.assertEqual(stat.S_IMODE(entry.external_attr >> 16), expected, entry.filename)

    @unittest.skipIf(os.name == "nt", "POSIX filesystem permission semantics")
    def test_non_windows_distribution_mode_preserves_file_permissions(self):
        self.binary.chmod(0o640)
        self.assertEqual(package.distribution_mode(self.binary, "macos"), 0o640)
        self.assertEqual(package.distribution_mode(self.binary, "linux"), 0o640)

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

    def test_linux_vulkan_preview_has_distinct_name_shader_and_guide(self):
        (self.source / "docs").mkdir()
        guide = self.source / "docs/VULKAN.md"
        guide.write_text("Vulkan preview fixture\n")
        shader = self.source / "app/src/stems/vulkan/linear.wgsl"
        shader.parent.mkdir(parents=True)
        shader.write_bytes(b"@compute fn fixture_projection() {}")
        with self.assertRaisesRegex(package.PackageError, "must contain the HTDemucs projection shader"):
            self.build("linux", "x86_64", vulkan=True)
        self.binary.write_bytes(self.binary.read_bytes() + shader.read_bytes())
        result = self.build("linux", "x86_64", vulkan=True)
        archive_name = Path(result["archive"]).name
        self.assertEqual(archive_name, "Recordbox-0.1.0-linux-x86_64-vulkan.tar.gz")
        self.assertEqual(result["manifest"]["target"]["stems"], "vulkan")
        self.assertEqual((Path(result["bundle"]) / "VULKAN.md").read_bytes(), guide.read_bytes())
        self.assertFalse(any(variant == "vulkan" for _, _, variant, _ in package.release_archives("0.5.8")))

    def test_vulkan_preview_rejects_other_targets_and_mixed_backends(self):
        for platform, architecture, options in (("macos", "arm64", {}), ("windows", "x86_64", {}),
                                                ("linux", "arm64", {}), ("linux", "x86_64", {"cuda12": True}),
                                                ("linux", "x86_64", {"directml": self.root})):
            with self.subTest(platform=platform, architecture=architecture, options=options):
                with self.assertRaisesRegex(package.PackageError, "separate Linux x86_64 build"):
                    self.build(platform, architecture, vulkan=True, **options)

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

    def test_version_mismatch_fails_before_inspection_or_output_creation(self):
        (self.source / "VERSION").write_text("0.5.0\n")
        with patch.object(package, "inspect_binary") as inspect, self.assertRaisesRegex(package.PackageError, "does not match Cargo.toml"):
            package.build_archive(self.binary, self.source, self.output, "macos", "arm64", self.commit)
        inspect.assert_not_called(); self.assertFalse(self.output.exists())
        (self.source / "VERSION").write_text("0.1.0\n")
        (self.source / "app/Cargo.lock").write_text('[[package]]\nname="recordbox"\nversion="0.5.0"\n')
        with self.assertRaisesRegex(package.PackageError, "Cargo.lock recordbox version"):
            package.read_release_version(self.source)

    def test_version_reader_requires_one_local_lock_entry_and_marker(self):
        lock = self.source / "app/Cargo.lock"
        entry = '[[package]]\nname="recordbox"\nversion="0.1.0"\n'
        for contents in ("", entry + entry, entry + 'source="registry+https://example.invalid"\n'):
            lock.write_text(contents)
            with self.subTest(contents=contents), self.assertRaisesRegex(package.PackageError, "exactly one local recordbox"):
                package.read_release_version(self.source)
        lock.write_text(entry); (self.source / "VERSION").unlink()
        with self.assertRaisesRegex(package.PackageError, "Missing or invalid release version metadata"):
            package.read_release_version(self.source)

    def test_semver_rejects_unsafe_names_empty_identifiers_and_leading_zeros(self):
        for version in ("../0.5.0", "01.5.0", "0.5.0-", "0.5.0-rc..1", "0.5.0-01", "0.5.0+build..x"):
            (self.source / "app/Cargo.toml").write_text(f'[package]\nname="recordbox"\nversion="{version}"\n')
            with self.subTest(version=version), self.assertRaisesRegex(package.PackageError, "safe release version"):
                package.read_release_version(self.source)

    def test_release_050_archive_plist_and_manifest_use_checked_version(self):
        (self.source / "VERSION").write_text("0.5.0\n")
        (self.source / "app/Cargo.toml").write_text('[package]\nname="recordbox"\nversion="0.5.0"\n')
        (self.source / "app/Cargo.lock").write_text('[[package]]\nname="recordbox"\nversion="0.5.0"\n')
        result = self.build()
        self.assertEqual(Path(result["archive"]).name, "Recordbox-0.5.0-macos-arm64.zip")
        self.assertEqual(result["manifest"]["version"], "0.5.0")
        plist = plistlib.loads((Path(result["bundle"]) / "Contents/Info.plist").read_bytes())
        self.assertEqual(plist["CFBundleShortVersionString"], "0.5.0")
        self.assertEqual(plist["CFBundleVersion"], "0.5.0")

    def test_check_version_cli_runs_without_native_tools_and_rejects_wrong_tag(self):
        command = [sys.executable, str(Path(package.__file__)), "--source-root", str(self.source), "--check-version"]
        result = subprocess.run([*command, "--tag", "v0.1.0"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["version"], "0.1.0")
        for tag in ("v0.5.0", "0.1.0", "refs/tags/v0.1.0", "v0.1.0\n"):
            result = subprocess.run([*command, "--tag", tag], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Release tag must be v0.1.0", result.stderr)
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

    def driver_fixture(self):
        directory = self.root / "Mesa x64"; directory.mkdir()
        for name in ("opengl32.dll", "libgallium_wgl.dll", "driverbridge.dll", "unused.dll"):
            (directory / name).write_bytes(("test driver " + name).encode())
        def inspect(path, _):
            dependencies = {"libgallium_wgl.dll": None} if path.name == "opengl32.dll" else {"driverbridge.dll": None} if path.name == "libgallium_wgl.dll" else {"kernel32.dll": None}
            return package.BinaryInfo("x86_64", dependencies)
        return directory, inspect

    def test_windows_software_gl_only_changes_the_temporary_smoke_copy(self):
        result = self.build("windows", "x86_64"); archive = Path(result["archive"])
        before = archive.read_bytes(); product = Path(result["bundle"])
        original_manifest = (product / "runtime-manifest.json").read_bytes()
        driver, inspect = self.driver_fixture(); launches = []
        def run(command, **kwargs):
            bundle = Path(command[0]).parent; launches.append(bundle)
            self.assertEqual(kwargs["env"]["GALLIUM_DRIVER"], "llvmpipe")
            self.assertEqual(kwargs["env"]["LIBGL_ALWAYS_SOFTWARE"], "1")
            self.assertEqual((bundle / "opengl32.dll").read_bytes(), (driver / "opengl32.dll").read_bytes())
            self.assertFalse((bundle / "unused.dll").exists())
            self.assertEqual((bundle / "runtime-manifest.json").read_bytes(), original_manifest)
            profile = Path(kwargs["env"]["PREPDECK_DATA"]); profile.mkdir(); (profile / "library.db").write_bytes(b"synthetic fixture")
            Path(command[2]).write_bytes(png())
            return subprocess.CompletedProcess(command, 0)
        with patch.object(smoke_app.sys, "platform", "win32"), patch.dict(os.environ, {"SystemRoot": str(self.root / "Windows")}), \
             patch.object(package, "inspect_binary", side_effect=inspect), patch.object(smoke_app, "windows_dll_preflight") as preflight, \
             patch.object(smoke_app.subprocess, "run", side_effect=run):
            report = smoke_app.smoke(archive, self.root / "driver-smoke", 20, driver)
        self.assertEqual(report["status"], "passed")
        self.assertTrue(report["profile_created"] and report["database_created"])
        self.assertEqual(report["process_returncode"], 0)
        self.assertTrue(report["windows_opengl_test_driver"]["temporary_override"])
        self.assertEqual({entry["name"] for entry in report["windows_opengl_test_driver"]["files"]}, {"opengl32.dll", "libgallium_wgl.dll", "driverbridge.dll"})
        self.assertIn("opengl32.dll", preflight.call_args.args[1])
        self.assertEqual(archive.read_bytes(), before)
        self.assertEqual((product / "runtime-manifest.json").read_bytes(), original_manifest)
        self.assertFalse((product / "opengl32.dll").exists())
        self.assertFalse(launches[0].exists())

    def test_windows_software_gl_refuses_wrong_architecture_and_missing_dependency(self):
        driver, inspect = self.driver_fixture(); bundle = self.root / "verified bundle"; bundle.mkdir()
        def wrong_arch(path, platform):
            info = inspect(path, platform)
            if path.name == "driverbridge.dll": info.architecture = "arm64"
            return info
        with patch.object(package, "inspect_binary", side_effect=wrong_arch), self.assertRaisesRegex(package.PackageError, "Architecture mismatch"):
            smoke_app.windows_opengl_override(bundle, driver, "x86_64")
        self.assertEqual(list(bundle.iterdir()), [])
        (driver / "driverbridge.dll").unlink()
        with patch.object(package, "inspect_binary", side_effect=inspect), self.assertRaisesRegex(package.PackageError, "Unresolved/non-system dependency"):
            smoke_app.windows_opengl_override(bundle, driver, "x86_64")
        self.assertEqual(list(bundle.iterdir()), [])

    def test_windows_software_gl_does_not_replace_verified_product_dll(self):
        driver, inspect = self.driver_fixture(); bundle = self.root / "verified bundle"; bundle.mkdir()
        (bundle / "driverbridge.dll").write_bytes(b"verified product runtime")
        with patch.object(package, "inspect_binary", side_effect=inspect), self.assertRaisesRegex(package.PackageError, "replace a verified application file"):
            smoke_app.windows_opengl_override(bundle, driver, "x86_64")
        self.assertEqual((bundle / "driverbridge.dll").read_bytes(), b"verified product runtime")
        self.assertFalse((bundle / "opengl32.dll").exists())

    def test_early_exit_without_png_preserves_startup_diagnostics(self):
        result = self.build("windows", "x86_64"); output = self.root / "early-exit"
        with patch.object(smoke_app.sys, "platform", "win32"), patch.dict(os.environ, {"SystemRoot": str(self.root / "Windows")}), \
             patch.object(smoke_app, "windows_dll_preflight"), \
             patch.object(smoke_app.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)), self.assertRaises(FileNotFoundError):
            smoke_app.smoke(Path(result["archive"]), output, 20)
        report = json.loads((output / "smoke.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["process_returncode"], 0)
        self.assertFalse(report["profile_created"])
        self.assertFalse(report["database_created"])

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
