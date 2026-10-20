"""Exercise the audit gate through its real CLI and a controlled cargo boundary."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "audit.py"
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def finding(name="h2", version="0.3.27", advisory="RUSTSEC-2026-0258", source=REGISTRY):
    return {
        "advisory": {"id": advisory, "package": name, "title": "Test vulnerability"},
        "versions": {"patched": [">=0.4.16"], "unaffected": []},
        "affected": None,
        "package": {
            "name": name,
            "version": version,
            "source": source,
            "checksum": None,
            "dependencies": [],
            "replace": None,
        },
    }


def report(findings=()):
    return {
        "database": {"advisory-count": 1, "last-commit": "test", "last-updated": None},
        "lockfile": {"dependency-count": 1},
        "settings": {
            "target_arch": [],
            "target_os": [],
            "severity": None,
            "ignore": [],
            "informational_warnings": ["unmaintained", "unsound", "notice"],
        },
        "vulnerabilities": {
            "found": bool(findings),
            "count": len(findings),
            "list": list(findings),
        },
        "warnings": {},
    }


class AuditCliTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.payload = self.directory / "report.json"
        cargo = self.directory / "cargo"
        cargo.write_text(
            f"#!{sys.executable}\n"
            "import os, pathlib, sys\n"
            "expected = ['audit', '--json', '--file', 'Cargo.lock']\n"
            "if sys.argv[1:] != expected:\n"
            "    sys.exit(95)\n"
            "pathlib.Path(os.environ['AUDIT_CWD']).write_text(os.getcwd())\n"
            "print(pathlib.Path(os.environ['AUDIT_REPORT']).read_text())\n"
            "sys.exit(int(os.environ['AUDIT_STATUS']))\n"
        )
        cargo.chmod(0o755)

    def audit(self, data, status=None, missing_cargo=False):
        self.payload.write_text(data if isinstance(data, str) else json.dumps(data))
        if status is None:
            status = int(bool(data["vulnerabilities"]["list"]))
        environment = os.environ.copy()
        environment.update(
            PATH=str(self.directory),
            AUDIT_REPORT=str(self.payload),
            AUDIT_STATUS=str(status),
            AUDIT_CWD=str(self.directory / "cwd"),
        )
        if missing_cargo:
            (self.directory / "cargo").unlink()
        return subprocess.run(
            [sys.executable, str(SCRIPT)],
            cwd=self.directory,
            env=environment,
            capture_output=True,
            text=True,
        )

    def test_clean_report_succeeds_and_audits_the_repository(self):
        result = self.audit(report())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("0 accepted", result.stdout)
        self.assertIn("0 blocked", result.stdout)
        self.assertEqual((self.directory / "cwd").read_text(), str(ROOT))

    def test_exact_exception_is_visible_and_succeeds(self):
        result = self.audit(report([finding()]))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Accepted", result.stdout)
        self.assertIn("RUSTSEC-2026-0258", result.stdout)
        self.assertIn("h2 0.3.27", result.stdout)
        self.assertIn("1 accepted", result.stdout)
        self.assertIn("0 blocked", result.stdout)

    def test_exception_does_not_cover_other_versions_sources_packages_or_advisories(self):
        cases = [
            finding(version="0.4.15"),
            finding(version="0.3.28"),
            finding(source="git+https://example.invalid/h2"),
            finding(source=None),
            finding(name="other-package"),
            finding(advisory="RUSTSEC-2026-9999"),
        ]
        for vulnerability in cases:
            with self.subTest(package=vulnerability["package"], advisory=vulnerability["advisory"]):
                result = self.audit(report([vulnerability]))
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("Blocked", result.stdout)
                self.assertIn(vulnerability["advisory"]["id"], result.stdout)
                self.assertIn("1 blocked", result.stdout)

    def test_exception_does_not_hide_an_additional_finding(self):
        result = self.audit(report([finding(), finding(name="rustls", version="0.23.42", advisory="RUSTSEC-2026-0285")]))
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("1 accepted", result.stdout)
        self.assertIn("1 blocked", result.stdout)
        self.assertIn("rustls 0.23.42", result.stdout)

    def test_warnings_remain_visible_without_becoming_vulnerabilities(self):
        data = report()
        data["warnings"]["yanked"] = [{"kind": "yanked", "package": {"name": "chacha20", "version": "0.10.1"}, "advisory": None, "affected": None, "versions": None}]
        result = self.audit(data)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("yanked", result.stdout)
        self.assertIn("chacha20 0.10.1", result.stdout)

    def test_malformed_or_missing_report_fields_fail_closed(self):
        cases = [(None, 0), ([], 0), ({}, 0), ({"error": "database unavailable"}, 0)]
        for section in ["vulnerabilities", "settings", "warnings"]:
            data = report()
            del data[section]
            cases.append((data, 0))
        for value in [None, {}, "bad"]:
            data = report()
            data["vulnerabilities"]["list"] = value
            cases.append((data, 0))
        for key, value in [("count", 1), ("count", False), ("found", True), ("found", "false")]:
            data = report()
            data["vulnerabilities"][key] = value
            cases.append((data, 0))
        for warnings in [None, [], {"yanked": None}, {"yanked": [{}]}]:
            data = report()
            data["warnings"] = warnings
            cases.append((data, 0))
        data = report([finding()])
        del data["vulnerabilities"]["list"][0]["package"]["source"]
        cases.append((data, 1))
        for data, status in cases:
            with self.subTest(report=data):
                result = self.audit(data, status=status)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("Invalid", result.stderr)

    def test_upstream_advisory_ignores_are_rejected(self):
        for ignored in [["RUSTSEC-2026-0285"], None, ""]:
            data = report()
            data["settings"]["ignore"] = ignored
            result = self.audit(data, status=0)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("ignore", result.stderr.lower())

    def test_filtered_reports_are_rejected(self):
        for key, value in [("severity", "high"), ("target_arch", ["x86_64"]), ("target_os", ["linux"])]:
            with self.subTest(setting=key):
                data = report()
                data["settings"][key] = value
                result = self.audit(data)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("filter", result.stderr.lower())

    def test_suppressed_or_malformed_informational_warning_settings_fail_closed(self):
        values = [
            [],
            ["unmaintained", "notice"],
            ["unsound", "notice"],
            ["unmaintained", "unsound"],
            None,
            "unmaintained,unsound,notice",
            {},
            ["unmaintained", "unsound", "notice", None],
        ]
        for findings in [[], [finding()]]:
            for value in values:
                with self.subTest(findings=findings, setting=value):
                    data = report(findings)
                    data["settings"]["informational_warnings"] = value
                    result = self.audit(data)
                    self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                    self.assertIn("informational", result.stderr.lower())
            with self.subTest(findings=findings, setting="missing"):
                data = report(findings)
                del data["settings"]["informational_warnings"]
                result = self.audit(data)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("informational", result.stderr.lower())

    def test_informational_warning_category_order_does_not_matter(self):
        data = report()
        data["settings"]["informational_warnings"] = ["notice", "unsound", "unmaintained"]
        result = self.audit(data)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_operational_failure_cannot_be_converted_to_success(self):
        for data, status in [(report(), 1), (report(), 2), (report([finding()]), 2), (report([finding()]), 0)]:
            with self.subTest(status=status, findings=data["vulnerabilities"]["list"]):
                result = self.audit(data, status=status)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("Cargo Audit", result.stderr)

    def test_invalid_json_fails_closed(self):
        result = self.audit("not JSON", status=0)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Invalid", result.stderr)

    def test_missing_cargo_fails_closed(self):
        result = self.audit(report(), missing_cargo=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("cargo", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
