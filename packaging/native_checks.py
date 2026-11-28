"""Build the release application and its test harness together; run selected suites in isolation."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

APP = Path(__file__).resolve().parents[1] / "app"
VARIANTS = ("standard", "directml")


def targets(messages, manifest, variant):
    expected = set() if variant == "standard" else {variant}
    selected = {}
    for line in messages:
        item = json.loads(line)
        if item["reason"] != "compiler-artifact" or not item.get("executable"):
            continue
        if (Path(item["manifest_path"]).resolve() != manifest or item["target"]["kind"] != ["bin"]
                or item["target"]["name"] != "recordbox"):
            continue
        if set(item["features"]) != expected:
            raise ValueError(f"Unexpected features on the {variant} executable: {item['features']}")
        key = "TEST" if item["profile"]["test"] else "APP"
        path = Path(item["executable"]).resolve(strict=True)
        if key in selected or not path.is_file():
            raise ValueError(f"Expected one {variant} {key} executable from Cargo")
        selected[key] = path
    if set(selected) != {"TEST", "APP"} or selected["APP"] == selected["TEST"]:
        raise ValueError("Cargo must produce distinct shipping and test executables")
    return selected


def build(app, variant):
    manifest = (app / "Cargo.toml").resolve()
    args = ["cargo", "build", "--manifest-path", str(manifest), "--locked", "--release", "--bin",
            "recordbox", "--tests", "--message-format=json-render-diagnostics"]
    if variant != "standard":
        args.extend(["--features", variant])
    result = subprocess.run(args, cwd=app, stdout=subprocess.PIPE, check=True, text=True, encoding="utf-8")
    return targets(result.stdout.splitlines(), manifest, variant)


def run_filters(executable, cwd, filters, flags):
    for selector in filters:
        print(f"Running {selector}", flush=True)
        passed = False
        with subprocess.Popen([str(executable), selector, "--test-threads=1", *flags], cwd=cwd,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                              encoding="utf-8", errors="replace") as test:
            for line in test.stdout:
                print(line, end="", flush=True)
                if line.startswith("test result:"):
                    result = re.match(r"test result: ok\. ([1-9]\d*) passed; 0 failed;", line)
                    passed = bool(result and ("--exact" not in flags or result[1] == "1"))
            if test.wait() != 0 or not passed:
                raise SystemExit(f"Native test failed or selected no passing test: {selector}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    compile_targets = commands.add_parser("build")
    test = commands.add_parser("test")
    for command in (compile_targets, test):
        command.add_argument("--variant", choices=VARIANTS, default="standard")
    test.add_argument("--exact", action="store_true")
    test.add_argument("--nocapture", action="store_true")
    ignored = test.add_mutually_exclusive_group()
    ignored.add_argument("--ignored", action="store_true")
    ignored.add_argument("--include-ignored", action="store_true")
    test.add_argument("filter", nargs="+")
    args = parser.parse_args()
    prefix = f"RECORDBOX_{args.variant.upper()}"
    if args.command == "build":
        paths = build(APP, args.variant)
        # Cargo's unqualified release/recordbox is reused by subsequent accelerator builds.
        # Keep this invocation's production binary separate from both its harness and siblings.
        binary = paths["APP"]
        snapshot = Path(os.environ["RUNNER_TEMP"]) / "recordbox-native" / args.variant / binary.name
        snapshot.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(binary, snapshot)
        paths["APP"] = snapshot
        with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as environment:
            for kind, path in paths.items():
                print(f"{prefix}_{kind}={path.as_posix()}", file=environment)
    else:
        flags = [flag for flag, enabled in (("--exact", args.exact), ("--nocapture", args.nocapture),
                 ("--ignored", args.ignored), ("--include-ignored", args.include_ignored)) if enabled]
        run_filters(Path(os.environ[f"{prefix}_TEST"]), APP, args.filter, flags)


if __name__ == "__main__":
    main()
