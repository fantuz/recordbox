"""Compile historical and working-tree ranking code into one small release binary."""
import argparse
import csv
import io
import os
import re
import shutil
import statistics
import subprocess
import tempfile
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
MATCHING = "app/src/browser_ext/matching.rs"
OLD = "d656c95"
BEFORE = "801255f"
SUBMITTED = "4f4f71e"


def at(revision, path):
    return subprocess.check_output(["git", "show", f"{revision}:{path}"], cwd=ROOT, text=True)


def rust_fn(source, signature):
    """Extract one top-level function or four-space-indented method from these revisions."""
    start = source.index(signature)
    indent = signature[:len(signature) - len(signature.lstrip())]
    end = source.index(f"\n{indent}}}\n", start) + len(indent) + 2
    return source[start:end]


def old_ranking(revision):
    browser = at(revision, "app/src/browser_ext.rs")
    scores = browser[browser.index("fn year_of("):browser.index("impl App {", browser.index("fn year_of("))]
    suggestions = rust_fn(browser, "    pub fn suggestions(")
    related = rust_fn(at(revision, "app/src/app.rs"), "    pub fn related_tracks_of(")
    return scores, suggestions, related


def prepare(work, upstream):
    src = work / "src"
    src.mkdir()
    for name in ("Cargo.toml", "Cargo.lock"):
        shutil.copy(HERE / name, work / name)
    shutil.copy(HERE / "fixture.rs", src / "main.rs")
    shutil.copy(ROOT / MATCHING, src / "candidate.rs")
    scores, suggestions, related = old_ranking(OLD)
    if upstream:
        if (scores, suggestions, related) != old_ranking(upstream):
            raise SystemExit(f"The ranking code differs between {OLD} and {upstream}. Compare the new upstream separately.")
    old = (
        "use crate::{app::{App, key_distance}, model::Track};\nuse std::collections::HashSet;\n"
        + scores + "impl App {\n"
        + suggestions.replace("fn suggestions(", "fn legacy_suggestions(") + "\n"
        + related.replace("fn related_tracks_of(", "fn legacy_related_tracks_of(") + "\n}\n"
    )
    (src / "legacy.rs").write_text(old)
    for module, revision, prefix in [("before", BEFORE, "before"), ("submit", SUBMITTED, "submit")]:
        methods = ["match_sound", "match_rows", "suggestions", "related_for", "match_explanation"]
        if module == "before":
            methods.append("match_tags")
        # Distinct method names let the revisions share the same App and fixture.
        pattern = r"\b(" + "|".join(methods) + r")\b"
        (src / f"{module}.rs").write_text(re.sub(pattern, lambda m: prefix + "_" + m[0], at(revision, MATCHING)))
    for path, signature, target in [
        ("app/src/app.rs", "pub fn key_distance(", "key_distance.rs"),
        ("app/src/smart.rs", "pub fn fold(", "smart.rs"),
    ]:
        (src / target).write_text(rust_fn((ROOT / path).read_text(), signature))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tracks", type=int, default=40_000)
    parser.add_argument("--trials", type=int, default=9)
    parser.add_argument("--upstream", help="also verify exact legacy scoring and Related filtering against this local Git ref")
    parser.add_argument("--output", type=Path, help="write every sample as CSV")
    args = parser.parse_args()
    if args.tracks < 40 or args.trials < 1:
        parser.error("Use at least 40 tracks and one trial.")
    fields = "tracks,catalog,providers,source,trial,revision,mode,rows,digest,milliseconds"
    measurements = []
    with tempfile.TemporaryDirectory(prefix="recordbox-ranking-") as temp:
        work = Path(temp)
        prepare(work, args.upstream)
        subprocess.run(["cargo", "build", "--locked", "--release", "--target-dir", "target"], cwd=work, check=True)
        program = "recordbox-matchbench.exe" if os.name == "nt" else "recordbox-matchbench"
        binary = work / "target" / "release" / program
        for catalog, providers, source in [("tagged", 0, 31), ("tagged", 1, 31), ("tagged", 3, 31), ("tagged", 0, 35), ("untagged", 0, 31)]:
            report = subprocess.check_output([binary, str(args.tracks), str(providers), str(args.trials), str(source), catalog], text=True)
            measurements.extend(csv.DictReader(io.StringIO(report), fieldnames=fields.split(",")))
    if args.output:
        with args.output.open("w", newline="") as dest:
            writer = csv.DictWriter(dest, fieldnames=fields.split(","))
            writer.writeheader()
            writer.writerows(measurements)
    digests = defaultdict(set)
    timings = defaultdict(list)
    for row in measurements:
        cohort = (row["catalog"], row["providers"], row["source"], row["mode"])
        if row["revision"] != "upstream":
            digests[cohort].add((row["rows"], row["digest"]))
        timings[cohort + (row["revision"],)].append(float(row["milliseconds"]))
    if any(len(d) != 1 for d in digests.values()):
        raise SystemExit("Fork revisions differ in ordered IDs or scores. Inspect the row counts and digests.")
    print("catalog,providers,source,mode,revision,median_ms")
    for cohort, samples in sorted(timings.items()):
        print(*cohort, f"{statistics.median(samples):.2f}", sep=",")


if __name__ == "__main__":
    main()
