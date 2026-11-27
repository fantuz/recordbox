#!/usr/bin/env python3
"""Create two pinned, independently optimized offline Rust feedback probes."""
import argparse
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent
REVS = {
    "fantuz": "9baccd5a09facd9bc4db04222a6d35aa2792c3e8",
    "adamsardo": "b55b775f91083e04f8732dfa4eae6cd1293866be",
}


def block(text, start):
    """Pinned functions/structs have closing braces at their declaration indentation."""
    lines = text.splitlines(keepends=True)
    positions = [i for i, line in enumerate(lines) if line.startswith(start)]
    if len(positions) != 1:
        raise ValueError(f"expected one declaration for {start!r}, found {len(positions)}")
    i = positions[0]
    end = re.match(r"^\s*", start).group() + "}"
    for j in range(i + 1, len(lines)):
        if lines[j].rstrip("\n") == end:
            return "".join(lines[i:j + 1])
    raise ValueError(f"cannot locate end of {start!r}")


def prepare(repo, label, sha):
    def source(file):
        return subprocess.check_output(["git", "-C", repo, "show", sha + ":app/src/" + file])

    dst = ROOT / "build" / label
    dst.mkdir(exist_ok=True, parents=True)
    for name in ("midi_map.rs", "midi_csv.rs"):
        (dst / name).write_bytes(source(name))
    src = source("midi.rs").decode("utf-8")
    names = [line for line in src.splitlines() if line.startswith("pub const NAMED_OUTPUTS:")]
    assert len(names) == 1
    code = [names[0], "#[derive(Clone,Copy,Debug,PartialEq)]", block(src, "pub enum Led {"),
            block(src, "struct OutState {")]
    if label == "fantuz":
        code.extend(["#[derive(Default)]", block(src, "struct NamedRows {"), block(src, "impl OutState {")])
    named_method = "    fn set_named(" if label == "fantuz" else "    pub fn set_named("
    code.extend(["impl Device {", block(src, "    pub fn reindex("),
                 block(src, "    pub fn set_led("), block(src, named_method), "}"])
    (dst / "production.rs").write_text("\n".join(code))
    (dst / "main.rs").write_bytes((ROOT / "fixture.rs").read_bytes())
    # Keep the production cfg per binary without editing any extracted methods.
    (dst / "types.rs").write_text((ROOT / "types.rs.in").read_text()
        .replace("NAMED_FIELDS", ("named: Mutex<NamedRows>," if label == "fantuz" else
            "named: Mutex<HashMap<u16, u16>>, out_names: Mutex<std::collections::HashSet<String>>,"))
        .replace("NAMED_INIT", ("named: Default::default()," if label == "fantuz" else
            "named: Default::default(), out_names: Default::default(),"))
        .replace("INVOKE", "self.set_named(name, deck, " + ("" if label == "fantuz" else "&") + "|_| value)"))
    return '[[bin]]\nname = "' + label + '"\npath = "' + label + '/main.rs"\n'


def main():
    parse = argparse.ArgumentParser(description=__doc__)
    parse.add_argument("repo", help="Git checkout containing both pinned commits")
    parse.add_argument("--offline", action="store_true")
    args = parse.parse_args()
    bins = "".join(prepare(args.repo, label, rev) for label, rev in REVS.items())
    manifest = ROOT / "build" / "Cargo.toml"
    manifest.write_text('''[package]
name="recordbox-midi-output-study"
version="0.0.0"
edition="2021"
publish=false

[dependencies]
dirs="=5.0.1"
parking_lot="=0.12.5"
sha2="=0.10.9"
tempfile="=3.27.0"

''' + bins)
    lock = ROOT / "Cargo.lock"
    if lock.exists():
        (manifest.parent / "Cargo.lock").write_bytes(lock.read_bytes())
    subprocess.run(["cargo", "build", "--release", "--manifest-path", str(manifest),
                    "--target-dir", str(ROOT / "build" / "target")] +
                    (["--locked"] if lock.exists() else []) +
                    (["--offline"] if args.offline else []), check=True)


if __name__ == "__main__":
    main()
