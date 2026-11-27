"""Synthetic native archives exercise release verification without running code."""
import copy
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import stat
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile
import zlib

import build_cuda12
import build_archive as package
import prepare_release as release


class DownloadNotesTests(unittest.TestCase):
    def setUp(self):
        self.notes = (Path(release.__file__).resolve().parents[1] / "docs/releases/0.5.0.md").read_text()

    def test_current_release_links_all_six_assets_and_tagged_guide(self):
        body = release.render_release_downloads(self.notes, "fantuz/recordbox", "v0.5.0")
        names = {f"Recordbox-0.5.0-{platform}-{arch}" + (".tar.gz" if platform == "linux" else ".zip")
                 for platform, arch in release.TARGETS} | {"SHA256SUMS", "COMMIT-LOG.txt"}
        for name in names:
            self.assertIn(f"[`{name}`](https://github.com/fantuz/recordbox/releases/download/v0.5.0/{name})", body)
        self.assertIn("https://github.com/fantuz/recordbox/blob/v0.5.0/docs/BUILD-AND-PACKAGING.md", body)

    def test_preview_links_start_with_the_first_cuda_release(self):
        self.assertNotIn("-cuda12.", release.render_release_downloads(self.notes, "test/recordbox", "v0.5.4"))
        body = release.render_release_downloads(self.notes, "test/recordbox", "v0.5.5")
        for target in ("windows-x86_64-cuda12.zip", "linux-x86_64-cuda12.tar.gz"):
            self.assertIn(f"https://github.com/test/recordbox/releases/download/v0.5.5/Recordbox-0.5.5-{target}", body)

    def test_radeon_archive_and_platform_labels_follow_the_release_version(self):
        previous = release.render_release_downloads(self.notes, "test/recordbox", "v0.5.5")
        self.assertNotIn("-directml.zip", previous)
        self.assertIn("| Windows x64, experimental |", previous)
        body = release.render_release_downloads(self.notes, "test/recordbox", "v0.5.6")
        self.assertIn("| Windows x64 |", body)
        self.assertIn("| Linux x64, experimental |", body)
        self.assertNotIn("Windows x64, experimental", body)
        self.assertIn("https://github.com/test/recordbox/releases/download/v0.5.6/Recordbox-0.5.6-windows-x86_64-directml.zip", body)
        self.assertEqual(len(list(package.release_archives("0.5.6"))), 7)

    def test_linux_vulkan_download_starts_with_0_6_0(self):
        for version, expected_count in (("0.5.8", 7), ("0.6.0", 8), ("0.6.1", 8)):
            with self.subTest(version=version):
                archives = list(package.release_archives(version))
                body = release.render_release_downloads(self.notes, "test/recordbox", f"v{version}")
                self.assertEqual(len(archives), expected_count)
                name = f"Recordbox-{version}-linux-x86_64-vulkan.tar.gz"
                if version == "0.5.8":
                    self.assertNotIn(name, body)
                else:
                    self.assertIn(("linux", "x86_64", "vulkan", name), archives)
                    self.assertIn(f"| Linux x64, experimental, Vulkan + CPU STEMS | [`{name}`]"
                                  f"(https://github.com/test/recordbox/releases/download/v{version}/{name}) |", body)

    def test_radeon_runtime_verification_rejects_a_missing_or_modified_file(self):
        from unittest.mock import patch
        from hashlib import sha256
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            hashes = {"onnxruntime.dll": sha256(b"runtime").hexdigest(),
                      "ort-LICENSE.txt": sha256(b"license").hexdigest()}
            with patch.dict(package.directml_runtime.FILES, hashes, clear=True):
                (root / "onnxruntime.dll").write_bytes(b"runtime")
                with self.assertRaisesRegex(ValueError, "ort-LICENSE"):
                    package.directml_runtime.verify(root, packaged=True)
                (root / "runtime-notices").mkdir()
                (root / "runtime-notices/ort-LICENSE.txt").write_bytes(b"license")
                package.directml_runtime.verify(root, packaged=True)
                (root / "onnxruntime.dll").write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "onnxruntime"):
                    package.directml_runtime.verify(root, packaged=True)

    def test_cuda_source_override_recovers_partial_cache_and_reuses_a_complete_copy(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(build_cuda12, "pascal_kernel_copy", shutil.copytree):
            root = Path(tmp)
            source = root / "upstream"
            source.mkdir()
            (source / "Cargo.toml").write_text("[package]")
            (source / "kernel.cu").write_text("compiled kernel")
            partial = root / "partial"
            partial.mkdir()
            (partial / "kernel.cu").write_text("partial")
            with build_cuda12.kernel_override(source, partial) as used:
                self.assertNotEqual(used, partial)
                self.assertEqual((used / "Cargo.toml").read_text(), "[package]")
                self.assertEqual((used / "kernel.cu").read_text(), "compiled kernel")
            self.assertEqual((partial / "kernel.cu").read_text(), "partial")
            stable = root / "stable"
            for _ in range(2):
                with build_cuda12.kernel_override(source, stable) as used:
                    self.assertEqual(used, stable)

    def test_preview_requires_both_pascal_kernels_and_nvidia_imports(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "recordbox.exe"
            imports = ["nvcuda.dll", "cublas64_12.dll"]
            binary.write_bytes(b"MZ")
            with self.assertRaises(package.PackageError):
                package.check_cuda_binary(binary, imports, "windows")
            binary.write_bytes(b".target sm_61\n.entry conv1d_f32(\n.entry softmax_f32(")
            package.check_cuda_binary(binary, imports, "windows")
            with self.assertRaises(package.PackageError):
                package.check_cuda_binary(binary, ["nvcuda.dll"], "windows")

    def test_copied_download_block_refreshes_future_version_and_repository(self):
        old = release.render_release_downloads(self.notes, "fantuz/recordbox", "v0.5.0")
        body = release.render_release_downloads(old, "adamsardo/recordbox", "v0.6.0")
        block = body.split("<!-- recordbox-downloads:start -->", 1)[1].split("<!-- recordbox-downloads:end -->", 1)[0]
        self.assertNotIn("0.5.0", block); self.assertNotIn("fantuz/recordbox", block)
        for platform, arch in release.TARGETS:
            name = f"Recordbox-0.6.0-{platform}-{arch}" + (".tar.gz" if platform == "linux" else ".zip")
            self.assertIn(f"https://github.com/adamsardo/recordbox/releases/download/v0.6.0/{name}", block)
        self.assertIn("https://github.com/adamsardo/recordbox/blob/v0.6.0/docs/BUILD-AND-PACKAGING.md", block)

    def test_semver_build_metadata_is_encoded_in_tag_and_asset_urls(self):
        body = release.render_release_downloads("# Release\n", "another/recordbox", "v0.6.0-rc.1+build.2")
        self.assertIn("/download/v0.6.0-rc.1%2Bbuild.2/Recordbox-0.6.0-rc.1%2Bbuild.2-macos-arm64.zip", body)
        self.assertIn("/blob/v0.6.0-rc.1%2Bbuild.2/docs/BUILD-AND-PACKAGING.md", body)
        self.assertIn("[`Recordbox-0.6.0-rc.1+build.2-macos-arm64.zip`]", body)

    def test_rendering_is_idempotent_without_duplicate_tables(self):
        body = release.render_release_downloads(self.notes, "fantuz/recordbox", "v0.5.0")
        self.assertEqual(release.render_release_downloads(body, "fantuz/recordbox", "v0.5.0"), body)
        for text in ("## Downloads", "| Platform | Archive |", "<!-- recordbox-downloads:start -->", "<!-- recordbox-downloads:end -->"):
            self.assertEqual(body.count(text), 1)

    def test_first_launch_links_follow_the_target_tag_and_refresh_once(self):
        tag = "v0.8.2-rc.1+build.2"
        repo = "another/recordbox"
        notes = release.render_release_downloads(self.notes, "previous/recordbox", "v0.8.2-rc.1", has_macos_guide=True)
        body = release.render_release_downloads(notes, repo, tag, has_macos_guide=True)
        self.assertEqual(body, release.render_release_downloads(body, repo, tag, has_macos_guide=True))
        self.assertNotIn("previous/recordbox", body)
        self.assertEqual(body.count("https://github.com/another/recordbox/blob/v0.8.2-rc.1%2Bbuild.2/docs/MACOS-FIRST-LAUNCH.md"), 1)
        self.assertEqual(body.count("https://support.apple.com/en-us/102445"), 1)
        self.assertIn("System Settings → Privacy & Security → Open Anyway", body)
        self.assertLess(body.index("Open Anyway"), body.index("| macOS Apple Silicon |"))
        historical = release.render_release_downloads(body, "fantuz/recordbox", "v0.5.0")
        self.assertEqual(historical, release.render_release_downloads(self.notes, "fantuz/recordbox", "v0.5.0"))

    def test_missing_downloads_section_is_appended_and_other_notes_are_unchanged(self):
        notes = "# Next release\n\n## Changes\n\nKeep these changes.\n<!-- recordbox-release-source: source -->\n"
        body = release.render_release_downloads(notes, "fantuz/recordbox", "v0.6.0")
        self.assertTrue(body.startswith(notes + "\n<!-- recordbox-downloads:start -->\n## Downloads"))

    def test_legacy_section_replacement_preserves_surrounding_notes(self):
        prefix = "# Release\n\n<!-- recordbox-release-source: source -->\n\n"
        suffix = "## Known limitations\n\nKeep this exact text.\n"
        notes = prefix + "## Downloads\n\nOld table.\n\n" + suffix
        body = release.render_release_downloads(notes, "fantuz/recordbox", "v0.6.0")
        self.assertTrue(body.startswith(prefix)); self.assertTrue(body.endswith(suffix))
        self.assertNotIn("Old table.", body)

    def test_malformed_or_duplicate_markers_are_rejected(self):
        start, end = "<!-- recordbox-downloads:start -->", "<!-- recordbox-downloads:end -->"
        for notes in (start, end, end + start, start + start + end, start + end + end,
                      "<!-- recordbox-downloads:start-->" + end):
            with self.subTest(notes=notes), self.assertRaisesRegex(package.PackageError, "marker"):
                release.render_release_downloads(notes, "fantuz/recordbox", "v0.6.0")


def executable(platform, architecture):
    if platform == "macos":
        return struct.pack("<IIIIIIII", 0xFEEDFACF, 0x100000C if architecture == "arm64" else 0x1000007,
                           0, 2, 1, 72, 0, 0) + struct.pack("<II", 0x19, 72) + bytes(64)
    if platform == "windows":
        dos = bytearray(64); dos[:2] = b"MZ"; struct.pack_into("<I", dos, 60, 64)
        return bytes(dos) + b"PE\0\0" + struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, 112, 2) + b"\x0b\x02" + bytes(110 + 40)
    return (b"\x7fELF\x02\x01\x01" + bytes(9) + struct.pack("<HHIQQQIHHHHHH", 3, 62, 1, 0x400000,
            64, 0, 0, 64, 56, 1, 0, 0, 0) + struct.pack("<IIQQQQQQ", 1, 5, 0, 0x400000, 0, 120, 120, 4096))


def png():
    def chunk(kind, payload):
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\0\xff\0\0\xff")) + chunk(b"IEND", b"")


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="recordbox-release-tests-")
        self.addCleanup(temporary.cleanup); self.root = Path(temporary.name)
        self.source = self.root / "source"; (self.source / "app/assets/fonts").mkdir(parents=True)
        (self.source / "packaging").mkdir(); (self.source / "docs/releases").mkdir(parents=True)
        (self.source / "VERSION").write_text("0.5.0\n")
        (self.source / "app/Cargo.toml").write_text('[package]\nname="recordbox"\nversion="0.5.0"\n')
        (self.source / "app/Cargo.lock").write_text('version=4\n[[package]]\nname="recordbox"\nversion="0.5.0"\n')
        (self.source / "LICENSE").write_text("MIT fixture license\n")
        (self.source / "app/assets/fonts/README.txt").write_text("Inter font notice\n")
        shutil.copyfile(Path(package.__file__).parent / "Inter-OFL.txt", self.source / "packaging/Inter-OFL.txt")
        (self.source / "docs/releases/0.5.0.md").write_text("# Recordbox 0.5.0\n\nTested native archives.\n")
        self.commit = "a" * 40; self.input = self.root / "input"; self.output = self.root / "staged"
        self.packages = self.input / "packages"; self.smoke = self.input / "smoke"
        self.packages.mkdir(parents=True); self.smoke.mkdir()
        self.names = {}
        for platform, architecture in release.TARGETS:
            binary = self.root / "binary"; binary.write_bytes(executable(platform, architecture)); binary.chmod(0o755)
            info = package.BinaryInfo(architecture, {"/usr/lib/libSystem.B.dylib": "/usr/lib/libSystem.B.dylib"}, "11.0") if platform == "macos" else package.BinaryInfo(architecture)
            with patch.object(package, "inspect_binary", return_value=info):
                result = package.build_archive(binary, self.source, self.root / f"built-{platform}-{architecture}",
                                               platform, architecture, self.commit)
            archive = Path(result["archive"]); self.names[platform, architecture] = archive.name
            shutil.copyfile(archive, self.packages / archive.name)
            shutil.copyfile(str(archive) + ".sha256", self.packages / (archive.name + ".sha256"))
            directory = self.smoke / f"smoke-{platform}-{architecture}-{self.commit}"; directory.mkdir()
            report = {"status": "passed", "synthetic_data_only": True, "archive": archive.name,
                      "archive_sha256": result["sha256"], "source_commit": self.commit,
                      "target": result["manifest"]["target"], "process_returncode": 0,
                      "profile_created": True, "database_created": True, "screenshot": {"width": 1, "height": 1}}
            (directory / "smoke.json").write_text(json.dumps(report)); (directory / "synthetic-window.png").write_bytes(png())
            (directory / "application.log").write_text("synthetic native CI fixture\n")
        self.log = f"commit {self.commit}\nAuthor: Example <example@example.com>\nDate: 2026-11-19\n\nRelease fixtures\n"

    def stage(self, tag="v0.5.0"):
        with patch.object(release, "get_commit_log", return_value=self.log):
            return release.prepare_release(self.input, self.output, self.source, self.commit, tag, "fixture-owner/recordbox")

    def report_path(self, platform="macos", architecture="arm64"):
        return self.smoke / f"smoke-{platform}-{architecture}-{self.commit}" / "smoke.json"

    def refresh_checksum(self, platform="macos", architecture="arm64"):
        name = self.names[platform, architecture]; checksum = package.sha256(self.packages / name)
        (self.packages / (name + ".sha256")).write_text(f"{checksum}  {name}\n")
        path = self.report_path(platform, architecture); report = json.loads(path.read_text())
        report["archive_sha256"] = checksum; path.write_text(json.dumps(report))

    def change_zip(self, change, platform="macos", architecture="arm64"):
        path = self.packages / self.names[platform, architecture]
        with zipfile.ZipFile(path) as source:
            entries = [(copy.copy(member), source.read(member)) for member in source.infolist()]
        change(entries)
        with zipfile.ZipFile(path, "w") as target:
            for member, data in entries: target.writestr(member, data)
        self.refresh_checksum(platform, architecture)

    def change_manifest(self, change, platform="macos", architecture="arm64"):
        def mutate(entries):
            for i, (member, data) in enumerate(entries):
                if member.filename.endswith("/runtime-manifest.json"):
                    manifest = json.loads(data); change(manifest)
                    entries[i] = (member, json.dumps(manifest).encode())
        self.change_zip(mutate, platform, architecture)

    def replace_product_file(self, suffix, data):
        def mutate(entries):
            name = next(member.filename for member, _ in entries if member.filename.endswith(suffix))
            for i, (member, old) in enumerate(entries):
                if member.filename == name: entries[i] = (member, data)
                elif member.filename.endswith("/runtime-manifest.json"):
                    manifest = json.loads(old)
                    row = next(r for r in manifest["files"] if name.endswith("/" + r["path"]))
                    row.update(size=len(data), sha256=hashlib.sha256(data).hexdigest())
                    entries[i] = (member, json.dumps(manifest).encode())
        self.change_zip(mutate)

    def test_valid_four_platform_release_has_exact_assets_and_source_marker(self):
        self.log += "\n## Downloads\n<!-- recordbox-downloads:start -->\nCommit text must remain unchanged.\n"
        self.stage()
        assets = self.output / "assets"
        self.assertEqual({p.name for p in assets.iterdir()}, set(self.names.values()) | {"COMMIT-LOG.txt", "SHA256SUMS"})
        sums = (assets / "SHA256SUMS").read_text().splitlines(); self.assertEqual(len(sums), 5)
        for line in sums:
            digest, name = line.split("  "); self.assertEqual(digest, package.sha256(assets / name))
        body = (self.output / "body.md").read_text()
        self.assertNotIn("MACOS-FIRST-LAUNCH.md", body)
        self.assertIn(f"<!-- recordbox-release-source: {self.commit} -->", body)
        self.assertIn("Author: Example", body); self.assertIn("0.5.0", body)
        self.assertIn(self.log.rstrip(), body)
        for name in set(self.names.values()) | {"COMMIT-LOG.txt", "SHA256SUMS"}:
            self.assertIn(f"[`{name}`](https://github.com/fixture-owner/recordbox/releases/download/v0.5.0/{name})", body)

    def test_stage_links_to_mac_first_launch_guide_only_if_in_release_checkout(self):
        (self.source / "docs/MACOS-FIRST-LAUNCH.md").write_text("# Open recordbox on macOS\n")
        self.stage()
        body = (self.output / "body.md").read_text()
        self.assertIn("https://github.com/fixture-owner/recordbox/blob/v0.5.0/docs/MACOS-FIRST-LAUNCH.md", body)
        self.assertNotIn("Archives are unsigned", body)
        self.assertIn(f"<!-- recordbox-release-source: {self.commit} -->", body)

    def test_verify_linux_vulkan_archive_against_tagged_shader_and_guide(self):
        shader = self.source / "app/src/stems/vulkan/linear.wgsl"
        shader.parent.mkdir(parents=True)
        shader.write_text("test projection shader")
        guide = self.source / "docs/VULKAN.md"
        guide.write_text("Vulkan setup guide")
        binary = self.root / "vulkan-binary"
        binary.write_bytes(executable("linux", "x86_64") + shader.read_bytes())
        with patch.object(package, "inspect_binary", return_value=package.BinaryInfo("x86_64")):
            result = package.build_archive(binary, self.source, self.root / "vulkan-bundle",
                                           "linux", "x86_64", self.commit, vulkan=True)
        def verify(directory):
            return release.verify_package(Path(result["archive"]), self.root / directory, self.source,
                                          "0.5.0", self.commit, "linux", "x86_64", "vulkan")

        self.assertEqual(verify("valid")["target"]["stems"], "vulkan")
        guide.write_text("changed guide")
        with self.assertRaisesRegex(package.PackageError, "Wrong Vulkan setup guide"):
            verify("changed-guide")
        shader.write_text("changed shader")
        with self.assertRaisesRegex(package.PackageError, "projection shader"):
            verify("changed-shader")

    def test_tag_mismatch_or_existing_output_is_refused(self):
        with self.assertRaisesRegex(package.PackageError, "Tag"): self.stage("v0.5.1")
        self.output.mkdir(); (self.output / "keep").write_text("existing")
        with self.assertRaisesRegex(package.PackageError, "already exists"): self.stage()
        self.assertEqual((self.output / "keep").read_text(), "existing")

    def test_missing_extra_and_corrupted_package_inputs_are_refused(self):
        sidecar = self.packages / (next(iter(self.names.values())) + ".sha256")
        original = sidecar.read_bytes(); sidecar.write_text("wrong checksum\n")
        with self.assertRaisesRegex(package.PackageError, "checksum"): self.stage()
        sidecar.write_bytes(original); extra = self.packages / "other.zip"; extra.write_bytes(b"unexpected")
        with self.assertRaisesRegex(package.PackageError, "package files"): self.stage()
        extra.unlink(); sidecar.unlink()
        with self.assertRaisesRegex(package.PackageError, "package files"): self.stage()
        self.assertFalse(self.output.exists())

    def test_failed_mixed_or_incomplete_smoke_never_stages(self):
        path = self.report_path(); original = json.loads(path.read_text())
        for key, value in (("status", "failed"), ("source_commit", "b" * 40), ("archive_sha256", "0" * 64),
                           ("process_returncode", False), ("profile_created", False), ("database_created", False),
                           ("screenshot", {"width": 2, "height": 1}),
                           ("windows_opengl_test_driver", {"temporary_override": True})):
            with self.subTest(key=key):
                report = dict(original); report[key] = value; path.write_text(json.dumps(report))
                with self.assertRaises(package.PackageError): self.stage()
                self.assertFalse(self.output.exists())
        path.write_text(json.dumps(original))
        (path.parent / "synthetic-window.png").write_bytes(png()[:-2])
        with self.assertRaises(package.PackageError): self.stage()

    def test_forged_manifest_source_version_and_file_list_are_refused(self):
        path = self.packages / self.names["macos", "arm64"]; original = path.read_bytes()
        changes = [lambda m: m.update(version="0.4.0"), lambda m: m["source"].update(commit="b" * 40),
                   lambda m: m["source"].update(cargo_lock_sha256="0" * 64),
                   lambda m: m["files"].append(dict(m["files"][0])),
                   lambda m: m["files"][0].update(mode="0777")]
        for change in changes:
            path.write_bytes(original); self.refresh_checksum(); self.change_manifest(change)
            with self.assertRaises(package.PackageError): self.stage()
            self.assertFalse(self.output.exists())

    def test_windows_crlf_checkout_hashes_are_allowed_only_for_windows(self):
        def crlf(m):
            for name, key in (("app/Cargo.toml", "cargo_manifest_sha256"), ("app/Cargo.lock", "cargo_lock_sha256")):
                m["source"][key] = hashlib.sha256((self.source / name).read_bytes().replace(b"\n", b"\r\n")).hexdigest()
        self.change_manifest(crlf, "windows", "x86_64"); self.stage(); shutil.rmtree(self.output)
        self.change_manifest(crlf)
        with self.assertRaisesRegex(package.PackageError, "Cargo digest"): self.stage()

    def test_wrong_machine_still_fails_with_self_consistent_manifest_hashes(self):
        self.replace_product_file("/Contents/MacOS/recordbox", executable("macos", "x86_64"))
        with self.assertRaisesRegex(package.PackageError, "Mach-O"): self.stage()

    def test_changed_license_and_plist_are_refused_even_if_manifest_rehashed(self):
        path = self.packages / self.names["macos", "arm64"]; original = path.read_bytes()
        self.replace_product_file("/Inter-OFL.txt", b"abbreviated license")
        with self.assertRaisesRegex(package.PackageError, "license notice"): self.stage()
        path.write_bytes(original); self.refresh_checksum()
        plist = {"CFBundleVersion": "0.4.0", "CFBundleShortVersionString": "0.4.0", "CFBundleExecutable": "recordbox"}
        self.replace_product_file("/Contents/Info.plist", plistlib.dumps(plist))
        with self.assertRaisesRegex(package.PackageError, "plist version"): self.stage()

    def test_traversal_and_special_file_members_fail_before_extraction(self):
        path = self.packages / self.names["macos", "arm64"]; original = path.read_bytes()
        for name, mode in (("Recordbox.app/../escape", stat.S_IFREG | 0o644),
                           ("Recordbox.app/device", stat.S_IFCHR | 0o644),
                           ("Recordbox.app/sticky", stat.S_IFREG | 0o4755)):
            path.write_bytes(original); self.refresh_checksum()
            member = zipfile.ZipInfo(name); member.create_system = 3; member.external_attr = mode << 16
            self.change_zip(lambda entries: entries.append((member, b"unsafe")))
            with self.assertRaises(package.PackageError): self.stage()
            self.assertFalse((self.root / "escape").exists()); self.assertFalse(self.output.exists())

    def test_bad_native_headers_are_rejected_without_execution(self):
        binary = self.root / "bad"
        for platform, architecture in release.TARGETS:
            binary.write_bytes(executable(platform, architecture)[:20])
            with self.subTest(platform=platform), self.assertRaises(package.PackageError):
                release.verify_machine(binary, platform, architecture)


if __name__ == "__main__":
    unittest.main()
