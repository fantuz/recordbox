#!/usr/bin/env python3
"""Reject new RustSec findings while reporting the exact, unresolved baseline."""

import argparse
import json
from pathlib import Path
import re
import sys


BASELINE = Path(__file__).resolve().parents[1] / ".github/security/advisory-baseline.json"
INFORMATIONAL = {"notice", "unmaintained", "unsound"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def field(value, name):
    require(isinstance(value, str) and bool(value.strip()), f"Missing or invalid {name}")
    return value


def identity(value):
    require(isinstance(value, dict), "A finding must be an object")
    key = tuple(field(value.get(k), k) for k in ("advisory_id", "package", "version", "source"))
    require(re.fullmatch(r"RUSTSEC-\d{4}-\d{4}", key[0]) is not None, "Invalid RustSec advisory ID")
    return key


def finding(value, kind):
    require(isinstance(value, dict), "An audit finding must be an object")
    package, advisory = value.get("package"), value.get("advisory")
    require(isinstance(package, dict) and isinstance(advisory, dict), "Missing finding package or advisory")
    require(advisory.get("package") == package.get("name"), "Advisory and package names disagree")
    require("informational" in advisory
            and advisory["informational"] == (None if kind == "vulnerability" else kind),
            "Advisory informational category disagrees with its report section")
    result = dict(advisory_id=advisory.get("id"), package=package.get("name"),
                  version=package.get("version"), source=package.get("source"),
                  title=field(advisory.get("title"), "advisory title"), kind=kind)
    identity(result)
    return result


def assess(audit, baseline, exit_code):
    require(type(exit_code) is int and exit_code in (0, 1), f"cargo audit failed with exit status {exit_code}")
    require(isinstance(audit, dict) and "error" not in audit, "Invalid audit report or diagnostic error")
    require(isinstance(audit.get("database"), dict)
            and isinstance(audit["database"].get("last-commit"), str)
            and re.fullmatch(r"[0-9a-fA-F]{40}", audit["database"]["last-commit"]) is not None,
            "Missing advisory database revision")
    lockfile = audit.get("lockfile")
    require(isinstance(lockfile, dict) and type(lockfile.get("dependency-count")) is int
            and lockfile["dependency-count"] > 0, "Missing or empty audited lockfile")
    settings = audit.get("settings")
    require(isinstance(settings, dict), "Missing audit settings")
    require(all(settings.get(k) == [] for k in ("ignore", "target_arch", "target_os"))
            and "severity" in settings and settings["severity"] is None,
            "Audit must not ignore advisories or filter by target or severity")
    categories = settings.get("informational_warnings")
    require(isinstance(categories, list) and all(isinstance(v, str) for v in categories)
            and len(categories) == len(INFORMATIONAL) and set(categories) == INFORMATIONAL,
            "Audit must include notice, unmaintained and unsound warnings")

    vulnerabilities, warnings = audit.get("vulnerabilities"), audit.get("warnings")
    require(isinstance(vulnerabilities, dict) and isinstance(warnings, dict), "Missing vulnerability or warning sections")
    rows = vulnerabilities.get("list")
    require(isinstance(rows, list), "Vulnerability list must be an array")
    require(type(vulnerabilities.get("count")) is int and vulnerabilities["count"] == len(rows)
            and type(vulnerabilities.get("found")) is bool and vulnerabilities["found"] == bool(rows),
            "Vulnerability count or found flag disagrees with the list")
    blocking = [finding(row, "vulnerability") for row in rows]
    informational = []
    for kind, entries in warnings.items():
        require(kind in INFORMATIONAL and isinstance(entries, list), f"Unknown or invalid warning category: {kind}")
        for row in entries:
            require(isinstance(row, dict) and row.get("kind") == kind, "Warning category disagrees with its section")
            parsed = finding(row, kind)
            (blocking if kind == "unsound" else informational).append(parsed)
    keys = [identity(row) for row in blocking + informational]
    require(len(keys) == len(set(keys)), "Duplicate audit finding")
    require(exit_code != 0 or not rows, "cargo audit exited successfully despite reported vulnerabilities")
    require(exit_code != 1 or bool(blocking), "cargo audit failed without blocking findings")

    require(isinstance(baseline, dict) and type(baseline.get("schema_version")) is int
            and baseline["schema_version"] == 1 and isinstance(baseline.get("findings"), list), "Invalid advisory baseline schema")
    expected = {}
    for row in baseline["findings"]:
        key = identity(row)
        field(row.get("reason"), "baseline reason")
        require(key not in expected, "Duplicate baseline finding")
        expected[key] = row
    current = {identity(row): row for row in blocking}
    return dict(known=[row for key, row in current.items() if key in expected],
                new=[row for key, row in current.items() if key not in expected],
                stale=[row for key, row in expected.items() if key not in current],
                informational=informational, database=audit["database"]["last-commit"])


def cell(value):
    return str(value).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("|", "\\|").replace("\n", " ").replace("\r", " ")


def render(result):
    failed = bool(result["new"] or result["stale"])
    lines = [f"# Dependency advisory gate: {'FAIL' if failed else 'PASS'}", "",
             f"{len(result['known'])} existing blocking findings remain unresolved; "
             f"{len(result['new'])} new blocking findings; {len(result['stale'])} stale baseline entries.", "",
             f"Advisory database revision: {cell(result['database'])}."]
    for key, heading in (("known", "Existing unresolved findings"), ("new", "New blocking findings"),
                         ("stale", "Stale baseline entries — remove these explicitly"),
                         ("informational", "Other advisories — reported without blocking this gate")):
        lines.extend(["", f"## {heading}", ""])
        rows = result[key]
        if not rows:
            lines.append("None.")
            continue
        lines.extend(["| Advisory | Package | Version | Kind | Source | Detail |", "| --- | --- | --- | --- | --- | --- |"])
        for row in rows:
            values = [row.get(k, "baseline") for k in ("advisory_id", "package", "version", "kind", "source")]
            values.append(row.get("title", row.get("reason", "")))
            lines.append("| " + " | ".join(cell(value) for value in values) + " |")
    lines.extend(["", "A passing gate means no new blocking findings; it does not certify an advisory-free dependency tree.", ""])
    return "\n".join(lines)


def load_json(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f"Duplicate JSON key: {key}")
            result[key] = value
        return result

    def invalid_constant(value):
        raise ValueError(f"Invalid JSON constant: {value}")

    return json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=unique,
                      parse_constant=invalid_constant)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--audit", type=Path, required=True, help="Unfiltered cargo audit --json output")
    parser.add_argument("--audit-exit-code", type=int, required=True, help="Actual cargo-audit exit status")
    parser.add_argument("--baseline", type=Path, default=BASELINE)
    parser.add_argument("--report", type=Path, help="Also write the Markdown summary here")
    args = parser.parse_args(argv)
    try:
        result = assess(load_json(args.audit), load_json(args.baseline), args.audit_exit_code)
        report = render(result)
        status = 1 if result["new"] or result["stale"] else 0
    except (OSError, ValueError) as error:
        report = f"# Dependency advisory gate: ERROR\n\n{cell(error)}\n\nNo trustworthy audit result was established.\n"
        status = 2
    print(report)
    if args.report:
        try:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(report, encoding="utf-8")
        except OSError as error:
            print(f"Cannot write advisory report: {error}", file=sys.stderr)
            return 2
    return status


if __name__ == "__main__":
    sys.exit(main())
