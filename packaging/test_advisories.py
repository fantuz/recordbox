"""Offline tests for the audit gate; no provider, Rust toolchain or network needed."""

import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest

import check_advisories as gate


def row(advisory="RUSTSEC-2026-0001", package="example", version="1.2.3", kind="vulnerability"):
    value = {"advisory": {"id": advisory, "package": package, "title": "Example advisory",
                          "informational": None if kind == "vulnerability" else kind},
             "package": {"name": package, "version": version,
                         "source": "registry+https://github.com/rust-lang/crates.io-index"}}
    if kind != "vulnerability":
        value["kind"] = kind
    return value


def report(rows=None, warnings=None):
    rows = [] if rows is None else rows
    return {"database": {"last-commit": "a" * 40}, "lockfile": {"dependency-count": 620},
            "settings": {"ignore": [], "target_arch": [], "target_os": [], "severity": None,
                         "informational_warnings": ["unmaintained", "unsound", "notice"]},
            "vulnerabilities": {"found": bool(rows), "count": len(rows), "list": rows},
            "warnings": {} if warnings is None else warnings}


class AdvisoryGateTests(unittest.TestCase):
    def setUp(self):
        self.baseline = gate.load_json(gate.BASELINE)
        self.rows = [row(v["advisory_id"], v["package"], v["version"]) for v in self.baseline["findings"]]
        self.audit = report(self.rows)

    def test_exact_baseline_passes_and_reports_unresolved_findings(self):
        result = gate.assess(self.audit, self.baseline, 1)
        self.assertEqual(len(result["known"]), 3)
        self.assertFalse(result["new"] or result["stale"])
        rendered = gate.render(result)
        self.assertIn("3 existing blocking findings remain unresolved", rendered)
        for finding in self.baseline["findings"]:
            self.assertIn(finding["advisory_id"], rendered)

    def test_new_vulnerability_and_each_changed_identity_component_fail(self):
        result = gate.assess(report(self.rows + [row()]), self.baseline, 1)
        self.assertEqual(len(result["new"]), 1)
        for key, value in (("advisory_id", "RUSTSEC-2026-9999"), ("package", "different"),
                           ("version", "0.30.1"), ("source", "git+https://example.com/repo")):
            with self.subTest(key=key):
                rows = copy.deepcopy(self.rows)
                extra = rows[0]
                if key == "advisory_id":
                    extra["advisory"]["id"] = value
                elif key == "package":
                    extra["package"]["name"] = extra["advisory"]["package"] = value
                else:
                    extra["package"][key] = value
                result = gate.assess(report(rows), self.baseline, 1)
                self.assertEqual(len(result["new"]), 1)
                self.assertEqual(len(result["stale"]), 1)

    def test_removed_findings_make_baseline_stale(self):
        for i in range(3):
            result = gate.assess(report(self.rows[:i] + self.rows[i + 1:]), self.baseline, 1)
            self.assertEqual([v["advisory_id"] for v in result["stale"]],
                             [self.baseline["findings"][i]["advisory_id"]])

    def test_unsound_blocks_even_when_cargo_audit_exits_zero(self):
        result = gate.assess(report(warnings={"unsound": [row(kind="unsound")]}),
                             {"schema_version": 1, "findings": []}, 0)
        self.assertEqual(result["new"][0]["kind"], "unsound")

    def test_unmaintained_and_notice_remain_visible(self):
        warnings = {kind: [row(advisory=f"RUSTSEC-2026-{i:04}", kind=kind)]
                    for i, kind in enumerate(("unmaintained", "notice"), 10)}
        result = gate.assess(report(self.rows, warnings), self.baseline, 1)
        self.assertFalse(result["new"] or result["stale"])
        self.assertEqual(len(result["informational"]), 2)
        self.assertIn("RUSTSEC-2026-0010", gate.render(result))

    def test_bad_exit_status_and_inconsistent_success_fail_closed(self):
        for status in (-9, 2, 101, True):
            with self.subTest(status=status), self.assertRaises(ValueError):
                gate.assess(self.audit, self.baseline, status)
        with self.assertRaises(ValueError):
            gate.assess(self.audit, self.baseline, 0)
        with self.assertRaises(ValueError):
            gate.assess(report(), {"schema_version": 1, "findings": []}, 1)

    def test_malformed_and_filtered_reports_fail_closed(self):
        invalid = [None, [], {}, {**self.audit, "error": "fetch failed"}]
        for section in ("database", "lockfile", "settings", "vulnerabilities", "warnings"):
            invalid.append({**self.audit, section: None})
        for key, value in (("count", True), ("count", 4), ("found", False), ("list", None)):
            v = copy.deepcopy(self.audit)
            v["vulnerabilities"][key] = value
            invalid.append(v)
        for key, value in (("ignore", ["RUSTSEC-2026-0459"]), ("target_os", ["linux"]),
                           ("target_arch", ["x86_64"]), ("severity", "high"),
                           ("informational_warnings", ["unmaintained"])):
            v = copy.deepcopy(self.audit)
            v["settings"][key] = value
            invalid.append(v)
        for v in invalid:
            with self.subTest(value=v), self.assertRaises(ValueError):
                gate.assess(v, self.baseline, 1)

    def test_duplicate_and_malformed_findings_fail_closed(self):
        invalid = [report(self.rows + [self.rows[0]])]
        for section, key, value in (("package", "version", ""), ("package", "source", None),
                                    ("advisory", "package", "other"), ("advisory", "id", "*")):
            rows = copy.deepcopy(self.rows)
            rows[0][section][key] = value
            invalid.append(report(rows))
        invalid.append(report(self.rows, {"future-category": []}))
        bad_warning = row(kind="unmaintained")
        invalid.append(report(self.rows, {"unsound": [bad_warning]}))
        for v in invalid:
            with self.subTest(value=v), self.assertRaises(ValueError):
                gate.assess(v, self.baseline, 1)
        bad = copy.deepcopy(self.baseline)
        bad["findings"].append(bad["findings"][0])
        with self.assertRaises(ValueError):
            gate.assess(self.audit, bad, 1)

    def test_invalid_baseline_cannot_widen_the_allowance(self):
        invalid = [None, [], {}, {"schema_version": True, "findings": []},
                   {"schema_version": 1, "findings": None}]
        for key in ("advisory_id", "package", "version", "source", "reason"):
            value = copy.deepcopy(self.baseline)
            del value["findings"][0][key]
            invalid.append(value)
        for value in invalid:
            with self.subTest(value=value), self.assertRaises(ValueError):
                gate.assess(self.audit, value, 1)

    def test_cli_preserves_report_on_success_failure_and_invalid_json(self):
        with tempfile.TemporaryDirectory() as directory:
            audit, output = Path(directory) / "audit.json", Path(directory) / "summary.md"
            for data, expected in ((json.dumps(self.audit), 0),
                                   (json.dumps(report(self.rows + [row()])), 1), ("{", 2)):
                audit.write_text(data, encoding="utf-8")
                with contextlib.redirect_stdout(io.StringIO()):
                    status = gate.main(["--audit", str(audit), "--audit-exit-code", "1", "--report", str(output)])
                self.assertEqual(status, expected)
                self.assertIn(("PASS", "FAIL", "ERROR")[expected], output.read_text(encoding="utf-8"))

    def test_cli_execution_error_is_not_tolerated_with_valid_baseline(self):
        with tempfile.TemporaryDirectory() as directory:
            audit, output = Path(directory) / "audit.json", Path(directory) / "summary.md"
            audit.write_text(json.dumps(self.audit), encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()):
                status = gate.main(["--audit", str(audit), "--audit-exit-code", "2", "--report", str(output)])
            self.assertEqual(status, 2)
            self.assertIn("No trustworthy audit result", output.read_text(encoding="utf-8"))

    def test_duplicate_json_keys_and_non_json_constants_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.json"
            for data in ('{"warnings": {}, "warnings": {}}', '{"count": NaN}'):
                path.write_text(data, encoding="utf-8")
                with self.assertRaises(ValueError):
                    gate.load_json(path)


if __name__ == "__main__":
    unittest.main()
