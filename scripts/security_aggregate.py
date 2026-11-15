#!/usr/bin/env python3
"""Combine independently uploaded CI security reports without hiding missing jobs."""

from __future__ import annotations

import datetime as dt
import json
from pathlib import Path
import sys


EXPECTED = {"dependencies", "licenses", "static", "unsafe", "secrets", "sbom", "tests"}


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print("usage: security_aggregate.py ARTIFACT_DIR [--expect-fuzz]", file=sys.stderr)
        return 2
    expected = EXPECTED | ({"fuzz"} if len(sys.argv) == 3 and sys.argv[2] == "--expect-fuzz" else set())
    source = Path(sys.argv[1])
    output = Path("security/reports")
    output.mkdir(parents=True, exist_ok=True)
    found = {}
    commits = set()
    evidence = {}
    for path in source.glob("*/security-audit.json"):
        try:
            report = json.loads(path.read_text())
            commits.add(report.get("commit"))
            evidence[path.parent.name] = {
                "rustc": report.get("rustc"), "host": report.get("host"),
                "tool_versions": report.get("tool_versions", {}),
                "exceptions": report.get("exceptions", []),
                "limitations": report.get("limitations", []),
                "platforms_verified": report.get("platforms_verified", []),
                "configurations_verified": report.get("configurations_verified", []),
                "recommendations": report.get("recommendations", []),
                "artifact": str(path.parent),
            }
            for item in report["checks"]:
                if item["status"] != "SKIPPED":
                    found[item["name"]] = item
        except (OSError, ValueError, KeyError) as error:
            found[path.parent.name] = {"name": path.parent.name, "status": "ERROR",
                                       "detail": f"Invalid job report: {error}"}
    for name in sorted(expected - found.keys()):
        found[name] = {"name": name, "status": "ERROR", "detail": "Expected CI artifact is missing"}
    if "fuzz" not in expected:
        found["fuzz"] = {"name": "fuzz", "status": "SKIPPED", "detail": "Scheduled/manual job only"}
    if len(commits) != 1:
        found["commit-consistency"] = {"name": "commit-consistency", "status": "ERROR",
                                       "detail": "Reports are missing or refer to different commits"}
    checks = [found[key] for key in sorted(found)]
    statuses = {item["status"] for item in checks}
    overall = next((status for status in ("ERROR", "FAIL", "WARNING", "PASS") if status in statuses), "SKIPPED")
    result = {"schema_version": 1, "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
              "commit": next(iter(commits)) if len(commits) == 1 else None,
              "status": overall, "checks": checks, "job_evidence": evidence,
              "limitations": ["Independent CI jobs run on Linux. The existing CI and release workflows cover Windows builds.",
                              "This is technical evidence, not a regulatory certification."]}
    (output / "security-audit.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    lines = ["# CI security audit", "", f"- Generated: {result['generated_at']}",
             f"- Commit: `{result['commit'] or 'unknown'}`", f"- Overall: **{overall}**", "",
             "| Check | Status |", "| --- | --- |"]
    lines.extend(f"| {item['name']} | {item['status']} |" for item in checks)
    lines.extend(["", "## Findings and evidence", ""])
    for item in checks:
        if item["status"] in ("FAIL", "WARNING", "ERROR"):
            lines.append(f"- **{item['name']} — {item['status']}**: see its job artifact and structured findings in JSON.")
        for finding in item.get("vulnerabilities", []):
            lines.append(f"  - {finding.get('id')}: {finding.get('package')} {finding.get('version')}; patched: {finding.get('patched')}.")
    lines.extend(["", "Each job artifact contains command logs, tool versions, exceptions and SBOM files when generated.", ""])
    (output / "security-audit.md").write_text("\n".join(lines))
    return 2 if overall == "ERROR" else 1 if overall == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
