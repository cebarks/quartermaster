#!/usr/bin/env python3
"""Run the security audit with one explicit, version-scoped risk acceptance."""

import json
from pathlib import Path
import subprocess
import sys


# Temporary acceptance of a real HTTP/2 memory-exhaustion risk in Actix's h2.
# Remove when Actix drops the vulnerable release; never extend to h2 0.4.
# https://github.com/actix/actix-web/issues/4199
# https://github.com/hyperium/h2/pull/948#issuecomment-5587273695
EXCEPTION = (
    "RUSTSEC-2026-0258",
    "h2",
    "0.3.27",
    "registry+https://github.com/rust-lang/crates.io-index",
)


def validate_report(report, returncode):
    if returncode not in (0, 1):
        raise ValueError(f"Cargo Audit exited with status {returncode}")
    vulnerabilities = report["vulnerabilities"]
    findings = vulnerabilities["list"]
    if not isinstance(findings, list):
        raise ValueError("vulnerabilities.list must be an array")
    if type(vulnerabilities["count"]) is not int or vulnerabilities["count"] != len(findings):
        raise ValueError("vulnerability count does not match the findings")
    if type(vulnerabilities["found"]) is not bool or vulnerabilities["found"] != bool(findings):
        raise ValueError("vulnerability found flag does not match the findings")
    if returncode != int(bool(findings)):
        raise ValueError("Cargo Audit exit status does not match the report")
    settings = report["settings"]
    if settings["ignore"] != []:
        raise ValueError("advisory-wide ignores are not allowed")
    if settings["severity"] is not None or settings["target_arch"] != [] or settings["target_os"] != []:
        raise ValueError("severity and target filters are not allowed")
    informational = settings["informational_warnings"]
    if (
        not isinstance(informational, list)
        or not all(isinstance(kind, str) and kind for kind in informational)
        or not {"unmaintained", "unsound", "notice"}.issubset(informational)
    ):
        raise ValueError("default informational warning categories must remain enabled")
    for finding in findings:
        advisory, package = finding["advisory"], finding["package"]
        for value in [advisory["id"], advisory["title"], package["name"], package["version"]]:
            if not isinstance(value, str) or not value:
                raise ValueError("invalid vulnerability identity")
        if package["source"] is not None and not isinstance(package["source"], str):
            raise ValueError("invalid package source")
    warnings = report["warnings"]
    if not isinstance(warnings, dict):
        raise ValueError("warnings must be an object")
    for entries in warnings.values():
        if not isinstance(entries, list):
            raise ValueError("warning entries must be an array")
        for warning in entries:
            for value in [warning["package"]["name"], warning["package"]["version"]]:
                if not isinstance(value, str) or not value:
                    raise ValueError("invalid warning identity")
    return findings, warnings


def evaluate_report(report, returncode):
    try:
        findings, warnings = validate_report(report, returncode)
    except (KeyError, TypeError, ValueError) as error:
        print(f"Invalid Cargo Audit report: {error}", file=sys.stderr)
        return 2

    accepted = 0
    for finding in findings:
        advisory, package = finding["advisory"], finding["package"]
        identity = (advisory["id"], package["name"], package["version"], package["source"])
        allowed = identity == EXCEPTION
        accepted += int(allowed)
        disposition = "Accepted (temporary HTTP/2 DoS risk)" if allowed else "Blocked"
        print(f"{disposition}: {advisory['id']} — {package['name']} {package['version']}: {advisory['title']}")
    for kind, entries in warnings.items():
        for warning in entries:
            package = warning["package"]
            print(f"Warning ({kind}): {package['name']} {package['version']}")
    blocked = len(findings) - accepted
    print(f"Security audit: {accepted} accepted, {blocked} blocked.")
    return int(blocked > 0)


def main():
    try:
        result = subprocess.run(
            ["cargo", "audit", "--json", "--file", "Cargo.lock"],
            cwd=Path(__file__).resolve().parent.parent,
            stdout=subprocess.PIPE,
            text=True,
        )
    except OSError as error:
        print(f"Failed to run cargo audit: {error}", file=sys.stderr)
        return 2
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        print(f"Invalid Cargo Audit JSON: {error}", file=sys.stderr)
        return 2
    return evaluate_report(report, result.returncode)


if __name__ == "__main__":
    sys.exit(main())
