import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

import native_checks


class NativeChecksTests(unittest.TestCase):
    def test_uses_only_the_requested_workspace_targets_and_requires_a_distinct_pair(self):
        with TemporaryDirectory() as root:
            base = Path(root).resolve()
            manifest = base / "Cargo.toml"
            def message(name, test=False, **overrides):
                path = base / name
                path.touch()
                return json.dumps(dict({"reason": "compiler-artifact", "manifest_path": str(manifest),
                    "target": {"name": "recordbox", "kind": ["bin"]}, "executable": str(path),
                    "features": ["directml"], "profile": {"test": test}}, **overrides))
            app, harness = message("app"), message("harness", True)
            decoy = message("foreign", True, manifest_path=str(base / "unrelated" / "Cargo.toml"))
            self.assertEqual(native_checks.targets([decoy, harness, app], manifest, "directml"),
                             {"TEST": base / "harness", "APP": base / "app"})
            for bad in [[app], [harness], [app, harness, harness],
                        [app, message("app", True)], [app, message("wrong", True, features=[])]]:
                with self.assertRaises(ValueError):
                    native_checks.targets(bad, manifest, "directml")

    @unittest.skipUnless(shutil.which("cargo"), "Cargo required for the native packaging pair")
    def test_real_cargo_build_produces_distinct_feature_specific_programs_and_harnesses(self):
        with TemporaryDirectory() as root:
            app = Path(root).resolve()
            (app / "src").mkdir()
            (app / "Cargo.toml").write_text('[package]\nname="recordbox"\nversion="0.1.0"\n'
                                            'edition="2021"\n[features]\ndirectml=[]\n')
            (app / "src/main.rs").write_text('fn main() { println!("{}", cfg!(feature="directml")); }\n'
                '#[test] fn release_build() { assert!(!cfg!(debug_assertions)); }\n'
                '#[test] #[ignore] fn manual_check() { assert!(cfg!(feature="directml")); }\n')
            env_file = app / "environment"
            environment = {"CARGO_NET_OFFLINE": "true", "CARGO_TARGET_DIR": str(app / "target"),
                           "GITHUB_ENV": str(env_file), "RUNNER_TEMP": str(app / "runner")}
            with patch.dict(os.environ, environment), patch.object(native_checks, "APP", app):
                subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=app, check=True)
                for variant in native_checks.VARIANTS:
                    with patch("sys.argv", ["native_checks", "build", "--variant", variant]):
                        native_checks.main()
                paths = dict(line.split("=", 1) for line in env_file.read_text().splitlines())
                for variant in native_checks.VARIANTS:
                    prefix = f"RECORDBOX_{variant.upper()}"
                    shipping = Path(paths[f"{prefix}_APP"])
                    harness = Path(paths[f"{prefix}_TEST"])
                    self.assertEqual(subprocess.check_output([shipping], text=True).strip(),
                                     str(variant == "directml").lower())
                    with contextlib.redirect_stdout(io.StringIO()):
                        native_checks.run_filters(harness, app, ["release_build"], ["--exact"])
                        with self.assertRaises(SystemExit):
                            native_checks.run_filters(harness, app, ["missing"], ["--exact"])
                        if variant == "directml":
                            native_checks.run_filters(harness, app, ["manual_check"], ["--exact", "--ignored"])
                        else:
                            with self.assertRaises(SystemExit):
                                native_checks.run_filters(harness, app, ["manual_check"], ["--include-ignored"])


if __name__ == "__main__":
    unittest.main()
