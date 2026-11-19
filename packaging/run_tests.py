"""Run the full serial suite; allow only the documented upstream PDB overflow."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

KNOWN = "export::usb_fix_test::a1_a_row_too_big_for_a_page_is_an_error_not_a_panic"


def accepts_baseline(log, returncode):
    failed = re.findall(r"^test (\S+) \.\.\. FAILED$", log, re.MULTILINE)
    summaries = re.findall(r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;", log, re.MULTILINE)
    blocks = re.findall(
        r"^---- " + re.escape(KNOWN) + r" stdout ----\n(.*?)(?=^---- |^failures:\n|^test result:|\Z)",
        log, re.MULTILINE | re.DOTALL,
    )
    panic = (len(blocks) == 1 and re.search(
        r"^thread '" + re.escape(KNOWN) + r"'(?: \(\d+\))? panicked at "
        r"(?:[^\r\n]*[/\\])?src[/\\]export[/\\]pdb\.rs:\d+:\d+:\n"
        r"attempt to subtract with overflow(?:\n|\Z)", blocks[0], re.MULTILINE,
    ) is not None)
    return (returncode == 101 and failed == [KNOWN] and len(summaries) == 1
            and summaries[0][0] == "FAILED" and int(summaries[0][1]) > 0
            and summaries[0][2] == "1" and summaries[0][4:] == ("0", "0")
            and panic and "error: could not compile" not in log)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    root = Path(__file__).resolve().parents[1]
    result = subprocess.run(
        ["cargo", "test", "--manifest-path", str(root / "app/Cargo.toml"),
         "--locked", "--bin", "recordbox", "--", "--test-threads=1"],
        cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        encoding="utf-8", errors="replace", env={**os.environ, "CARGO_TERM_COLOR": "never"},
    )
    log = result.stdout
    print(log, end="")
    (args.output / "cargo-test.txt").write_text(log, encoding="utf-8")
    accepted = accepts_baseline(log, result.returncode)
    message = ("Full serial suite: only the known, unresolved upstream PDB subtraction overflow failed. "
               "This is a no-new-failures check, not an all-tests-pass result.\n"
               if accepted else "Application test baseline changed or the test run failed. "
               "Review cargo-test.txt; remove the baseline when the PDB bug is fixed.\n")
    (args.output / "tests.md").write_text(message, encoding="utf-8")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write("### Application tests\n\n" + message)
    print(message)
    return 0 if accepted else 1


if __name__ == "__main__":
    sys.exit(main())
