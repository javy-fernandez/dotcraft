#!/usr/bin/env python3
"""Run reproducible, non-mutating security checks and write evidence to disk."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
REPORTS = ROOT / "security" / "reports"
CATEGORIES = ("dependencies", "licenses", "static", "unsafe", "secrets", "sbom", "tests", "fuzz")
TIMEOUT = 45 * 60


def version(command: list[str]) -> str | None:
    try:
        result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, timeout=15, check=False)
        if result.returncode != 0:
            return None
        lines = (result.stdout or result.stderr).strip().splitlines()
        if command == ["rustc", "-vV"]:
            return next((line.removeprefix("host: ") for line in lines if line.startswith("host: ")), None)
        return lines[0] if lines else None
    except (OSError, subprocess.TimeoutExpired, IndexError):
        return None


def command_result(name: str, command: list[str], *, env: dict[str, str] | None = None,
                   timeout: int = TIMEOUT, success_codes: tuple[int, ...] = (0,)) -> dict:
    """Record stdout/stderr even on failure. Never turn a missing tool into PASS."""
    out = REPORTS / f"{name}.stdout.log"
    err = REPORTS / f"{name}.stderr.log"
    result = {"name": name, "command": command, "stdout": out.name, "stderr": err.name,
              "status": "ERROR", "exit_code": None}
    if shutil.which(command[0]) is None:
        err.write_text(f"Missing executable: {command[0]}\n")
        out.write_text("")
        result["detail"] = f"Missing executable: {command[0]}"
        return result
    try:
        with out.open("wb") as stdout, err.open("wb") as stderr:
            completed = subprocess.run(command, cwd=ROOT, env=env, stdout=stdout, stderr=stderr,
                                       timeout=timeout, check=False)
        result["exit_code"] = completed.returncode
        result["status"] = "PASS" if completed.returncode in success_codes else "FAIL"
    except subprocess.TimeoutExpired:
        result["detail"] = f"Timed out after {timeout}s"
    except OSError as exc:
        result["detail"] = f"Could not run command: {exc}"
    return result


def load_json(path: Path):
    try:
        return json.loads(path.read_text())
    except (OSError, UnicodeError, json.JSONDecodeError):
        return None


def category(name: str, checks: list[dict], **extra) -> dict:
    statuses = {check["status"] for check in checks}
    status = next((s for s in ("ERROR", "FAIL", "WARNING", "PASS") if s in statuses), "SKIPPED")
    return {"name": name, "status": status, "checks": checks, **extra}


def advisory_severity(advisory: dict) -> str:
    """Only use an explicit numerical CVSS score; a vector is not a score."""
    score = advisory.get("cvss")
    if isinstance(score, bool) or not isinstance(score, (int, float)) or not math.isfinite(score):
        return "UNSPECIFIED"
    if not 0 <= score <= 10:
        return "UNSPECIFIED"
    if score >= 9:
        return "CRITICAL"
    if score >= 7:
        return "HIGH"
    if score >= 4:
        return "MEDIUM"
    if score > 0:
        return "LOW"
    return "INFORMATIONAL"


def dependencies() -> dict:
    checks = []
    findings = []
    warnings = {}
    for label, lockfile in (("workspace", "Cargo.lock"), ("fuzz", "fuzz/Cargo.lock")):
        command = ["cargo", "audit", "--json", "--file", lockfile]
        if os.environ.get("SECURITY_AUDIT_OFFLINE") == "1":
            command.insert(2, "--no-fetch")
        check = command_result(f"cargo-audit-{label}", command)
        data = load_json(REPORTS / check["stdout"])
        if isinstance(data, dict):
            for item in data.get("vulnerabilities", {}).get("list", []):
                advisory = item.get("advisory", {})
                package = item.get("package", {})
                findings.append({"id": advisory.get("id"), "title": advisory.get("title"),
                                 "lockfile": lockfile, "package": package.get("name"),
                                 "version": package.get("version"),
                                 "patched": item.get("versions", {}).get("patched", []),
                                 "severity": advisory_severity(advisory)})
            warnings[label] = {kind: len(items) for kind, items in data.get("warnings", {}).items()}
            if check["exit_code"] == 0 and any(warnings[label].values()):
                check["status"] = "WARNING"
            relevant = [finding for finding in findings if finding["lockfile"] == lockfile]
            if any(f["severity"] in ("CRITICAL", "HIGH", "MEDIUM", "UNSPECIFIED") for f in relevant):
                check["status"] = "FAIL"
            elif relevant:
                check["status"] = "WARNING"
            elif check["exit_code"] != 0:
                check["status"] = "ERROR"
                check["detail"] = "cargo-audit failed without a vulnerability finding"
        else:
            check["status"] = "ERROR"
            check["detail"] = "cargo-audit did not produce parseable JSON"
        checks.append(check)
    return category("dependencies", checks, vulnerabilities=findings, warnings=warnings)


def licenses() -> dict:
    checks = []
    diagnostics = []
    for label, manifest in (("workspace", "Cargo.toml"), ("fuzz", "fuzz/Cargo.toml")):
        command = ["cargo", "deny", "--manifest-path", manifest, "--config", "deny.toml", "-f", "json",
                   "check", "advisories", "licenses", "sources", "bans"]
        if os.environ.get("SECURITY_AUDIT_OFFLINE") == "1":
            command.insert(8, "--offline")
        check = command_result(f"cargo-deny-{label}", command)
        local = []
        try:
            for line in ((REPORTS / check["stdout"]).read_text() + "\n" +
                         (REPORTS / check["stderr"]).read_text()).splitlines():
                if line.startswith("{"):
                    event = json.loads(line)
                    if event.get("type") == "diagnostic":
                        fields = event.get("fields", {})
                        local.append({"graph": label, "code": fields.get("code"),
                                      "message": fields.get("message"), "severity": fields.get("severity")})
                    elif event.get("type") == "summary":
                        local.append({"graph": label, "summary": event.get("fields", {})})
        except (OSError, UnicodeError, json.JSONDecodeError):
            check["status"] = "ERROR"
            check["detail"] = "cargo-deny diagnostics could not be parsed"
        if check["exit_code"] not in (None, 0) and any(d.get("severity") == "error" for d in local):
            check["status"] = "FAIL"
        elif check["exit_code"] not in (None, 0):
            check["status"] = "ERROR"
            check["detail"] = "cargo-deny failed without policy diagnostics"
        elif check["exit_code"] == 0 and any(d.get("severity") == "warning" for d in local):
            check["status"] = "WARNING"
        diagnostics.extend(local)
        checks.append(check)
    limitations = []
    if any(d.get("severity") == "bug" for d in diagnostics):
        for check in checks:
            if check["status"] == "PASS":
                check["status"] = "WARNING"
        limitations.append("cargo-deny emitted internal 'bug' diagnostics; inspect its raw log before relying on the bans result.")
    return category("licenses", checks, diagnostic_count=len(diagnostics), diagnostics=diagnostics,
                    limitations=limitations)


def static() -> dict:
    checks = [command_result("cargo-clippy", ["cargo", "clippy", "--workspace", "--all-targets",
                                              "--all-features", "--locked", "--", "-D", "warnings"])]
    return category("static", checks,
                    limitations=["DotCraft is formatted by hand and has no rustfmt configuration; "
                                 "formatting is not part of this gate."])


def unsafe() -> dict:
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(ROOT / "target" / "security-geiger")
    command = ["cargo", "geiger", "--manifest-path", str(ROOT / "crates" / "rc-app" / "Cargo.toml"),
               "--output-format", "Json", "--locked", "--all-targets", "--all-features"]
    if os.environ.get("SECURITY_AUDIT_OFFLINE") == "1":
        command.append("--offline")
    check = command_result("cargo-geiger", command, env=env)
    data = load_json(REPORTS / check["stdout"])
    if data is None:
        check["status"] = "ERROR"
        check["detail"] = "cargo-geiger did not produce parseable JSON"
    packages = []
    if isinstance(data, dict):
        for item in data.get("packages", []):
            package = item.get("package", {}).get("id", {})
            count = item.get("unsafety", {}).get("used", {}).get("exprs", {}).get("unsafe_", 0)
            packages.append({"name": package.get("name"), "version": package.get("version"),
                             "own_code": str(package.get("name", "")).startswith("rc-"),
                             "unsafe_expressions": count})
    own = []
    pattern = re.compile(r"\bunsafe\s*(?:\{|impl\b|extern\b)|\bextern\s*\"C(?:-unwind)?\"")
    for source in sorted((ROOT / "crates").glob("*/src/**/*.rs")):
        for number, line in enumerate(source.read_text(errors="replace").splitlines(), 1):
            if pattern.search(line) and not line.lstrip().startswith("//"):
                own.append({"path": str(source.relative_to(ROOT)), "line": number,
                            "kind": "ffi" if "extern" in line or "libc::" in line else "unsafe",
                            "note": "Source inventory; review context and SAFETY comment manually"})
    (REPORTS / "unsafe-own-code.json").write_text(json.dumps(own, indent=2) + "\n")
    return category("unsafe", [check], own_code=own, packages=packages,
                    packages_without_metrics=len(data.get("packages_without_metrics", [])) if isinstance(data, dict) else None,
                    limitations=["Source inventory is lexical and may include test-only code or string literals.",
                                 "cargo-geiger counts unsafe in Rust dependencies, not native C/C++ internals."])


def secrets() -> dict:
    policy = load_json(ROOT / "security" / "exceptions.json")
    entries = policy.get("exceptions", []) if isinstance(policy, dict) else []
    ignore_path = ROOT / ".gitleaksignore"
    ignored = {line.strip() for line in ignore_path.read_text().splitlines()
               if line.strip() and not line.lstrip().startswith("#")}
    expected = {entry.get("fingerprint") for entry in entries if entry.get("tool") == "gitleaks"}
    invalid = []
    if ignored != expected:
        invalid.append("Gitleaks ignore entries differ from reviewed exceptions")
    today = dt.datetime.now(dt.timezone.utc).date()
    for entry in entries:
        if not all(entry.get(key) for key in ("id", "tool", "reason", "owner", "reviewed_at", "expires_at")):
            invalid.append(f"Incomplete exception: {entry.get('id', 'unknown')}")
            continue
        try:
            if dt.date.fromisoformat(entry["expires_at"]) < today:
                invalid.append(f"Expired exception: {entry['id']}")
        except ValueError:
            invalid.append(f"Invalid expiry: {entry['id']}")
    command = [os.environ.get("GITLEAKS_BIN", "gitleaks"), "git", "--redact", "--report-format", "json",
               "--report-path", str(REPORTS / "gitleaks.json"), str(ROOT)]
    check = command_result("gitleaks", command)
    findings = load_json(REPORTS / "gitleaks.json")
    if check["exit_code"] == 1 and isinstance(findings, list):
        check["status"] = "FAIL"
    elif check["exit_code"] not in (0, 1):
        check["status"] = "ERROR"
    elif check["exit_code"] == 1:
        check["status"] = "ERROR"
        check["detail"] = "Gitleaks failed without a structured finding report"
    # Never copy secret contents, matching lines, or credentials into the summary.
    safe = [{"rule": f.get("RuleID"), "file": f.get("File"), "line": f.get("StartLine"),
             "fingerprint": f.get("Fingerprint")} for f in findings] if isinstance(findings, list) else []
    if invalid:
        check["status"] = "FAIL"
        check["detail"] = "; ".join(invalid)
    return category("secrets", [check], finding_count=len(safe), findings=safe, exceptions=entries,
                    limitations=["Git history scan does not include ignored, untracked or external files."])


def sbom() -> dict:
    existing = list((ROOT / "crates").glob("*/**/*.cdx.json"))
    if existing:
        return category("sbom", [{"name": "cargo-cyclonedx", "status": "ERROR",
                                  "detail": "Existing .cdx.json files would be overwritten; move them first"}])
    check = command_result("cargo-cyclonedx", ["cargo", "cyclonedx", "--format", "json", "--all", "--target", "all"])
    generated = sorted((ROOT / "crates").glob("*/**/*.cdx.json"))
    destination = REPORTS / "sbom-components"
    destination.mkdir(exist_ok=True)
    copied = []
    try:
        for source in generated:
            target = destination / source.name
            shutil.copy2(source, target)
            copied.append(str(target.relative_to(REPORTS)))
        app = destination / "rc-app.cdx.json"
        if app.exists():
            shutil.copy2(app, REPORTS / "sbom.cdx.json")
        else:
            check["status"] = "ERROR"
            check["detail"] = "No rc-app SBOM generated"
    finally:
        for source in generated:
            source.unlink()
    if not copied:
        check["status"] = "ERROR"
        check["detail"] = "No CycloneDX files generated"
    metadata = command_result("cargo-metadata", ["cargo", "metadata", "--locked", "--format-version", "1"])
    metadata_data = load_json(REPORTS / metadata["stdout"])
    native = []
    if isinstance(metadata_data, dict):
        for package in metadata_data.get("packages", []):
            if package.get("links"):
                native.append({"name": package.get("name"), "version": package.get("version"),
                               "links": package["links"], "source": package.get("source")})
        (REPORTS / "native-cargo-components.json").write_text(json.dumps(native, indent=2) + "\n")
    else:
        metadata["status"] = "ERROR"
        metadata["detail"] = "cargo metadata did not produce parseable JSON"
    return category("sbom", [check, metadata], files=copied, native_cargo_components=native,
                    limitations=["Cargo SBOM does not enumerate system libraries, bundled C code, fonts or packaged tools."])


def tests() -> dict:
    checks = [command_result("security-core-tests", ["cargo", "test", "--locked", "-p", "rc-core", "--lib"]),
              command_result("security-dotnet-parser-tests", ["cargo", "test", "--locked", "-p", "rc-dotnet", "--lib"]),
              command_result("security-database-tests", ["cargo", "test", "--locked", "-p", "rc-app", "--bin", "dotcraft",
                                                       "database::tests", "--", "--test-threads=1"])]
    return category("tests", checks)


def fuzz() -> dict:
    if sys.platform not in ("linux", "darwin"):
        return category("fuzz", [{"name": "cargo-fuzz", "status": "SKIPPED",
                                  "detail": "libFuzzer job is supported on Unix; use the Linux CI runner"}])
    checks = []
    toolchain = os.environ.get("SECURITY_FUZZ_TOOLCHAIN", "nightly")
    env = dict(os.environ)
    # LeakSanitizer cannot inspect the process under some CI/container ptrace policies.
    # Keep AddressSanitizer enabled; only disable its leak subcheck.
    env["ASAN_OPTIONS"] = "detect_leaks=0"
    for target in ("project_file", "session_input", "test_reports"):
        command = ["cargo", f"+{toolchain}", "fuzz", "run", "--fuzz-dir", "fuzz", target,
                   "--", "-max_total_time=10", "-timeout=2", "-max_len=16384"]
        checks.append(command_result(f"fuzz-{target}", command, env=env, timeout=20 * 60))
    return category("fuzz", checks, limitations=["Ten seconds per target is a smoke test, not exhaustive fuzzing."])


RUNNERS = {"dependencies": dependencies, "licenses": licenses, "static": static,
           "unsafe": unsafe, "secrets": secrets, "sbom": sbom, "tests": tests, "fuzz": fuzz}


def write_reports(results: list[dict]) -> None:
    commit = version(["git", "rev-parse", "HEAD"])
    tool_versions = {name: version(command) for name, command in {
        "cargo-audit": ["cargo", "audit", "--version"],
        "cargo-deny": ["cargo", "deny", "--version"],
        "cargo-cyclonedx": ["cargo", "cyclonedx", "--version"],
        "cargo-geiger": ["cargo", "geiger", "--version"],
        "cargo-fuzz": ["cargo", "fuzz", "--version"],
        "gitleaks": [os.environ.get("GITLEAKS_BIN", "gitleaks"), "version"],
    }.items()}
    statuses = {r["status"] for r in results}
    overall = next((s for s in ("ERROR", "FAIL", "WARNING", "PASS") if s in statuses), "SKIPPED")
    rust_details = version(["rustc", "-vV"])
    configurations = ["workspace/all-targets/all-features"] if any(
        item["name"] == "static" and item["status"] != "SKIPPED" for item in results) else []
    recommendations = []
    for item in results:
        if item["status"] in ("FAIL", "WARNING", "ERROR"):
            guidance = {
                "dependencies": "Review each advisory and upgrade affected packages or document a narrow, expiring exception.",
                "licenses": "Review cargo-deny diagnostics, source origins and license compatibility.",
                "static": "Resolve reported Clippy findings in a separate application-code change.",
                "unsafe": "Review each own-code SAFETY argument and native/FFI boundary manually.",
                "secrets": "Revoke real credentials, remove exposure and review exact-match exceptions.",
                "sbom": "Inspect SBOM generation logs and inventory native/distributed components.",
                "tests": "Investigate failing security-focused tests before release.",
                "fuzz": "Triage crash artifacts and rerun with a longer budget.",
            }
            recommendations.append({"check": item["name"], "action": guidance[item["name"]]})
    exceptions = (load_json(ROOT / "security" / "exceptions.json") or {}).get("exceptions", [])
    report = {"schema_version": 1, "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
              "commit": commit, "dirty_worktree": bool(version(["git", "status", "--porcelain"])),
              "rustc": version(["rustc", "--version"]), "host": rust_details,
              "platforms_verified": [rust_details] if rust_details else [],
              "configurations_verified": configurations,
              "tool_versions": tool_versions, "status": overall, "checks": results,
              "recommendations": recommendations,
              "exceptions": exceptions,
              "policy": "Critical/high/medium/unknown vulnerabilities block; low/informational findings warn. Warnings never count as PASS.",
              "limitations": ["This is technical evidence, not a regulatory certification.",
                              "Linux local runs cannot validate the Windows release binary or system DLL inventory.",
                              "External tools, native components and distributed assets require separate inventory review."]}
    (REPORTS / "security-audit.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    lines = ["# Security audit", "", f"- Generated: {report['generated_at']}",
             f"- Commit: `{commit or 'unknown'}`", f"- Rust: `{report['rustc'] or 'unknown'}`",
             f"- Overall: **{overall}**", "", "| Check | Status | Evidence |", "| --- | --- | --- |"]
    for item in results:
        evidence = ", ".join(f"`{c.get('stdout', '')}`" for c in item.get("checks", []) if c.get("stdout"))
        lines.append(f"| {item['name']} | {item['status']} | {evidence or 'see JSON'} |")
    lines.extend(["", "## Findings", ""])
    for item in results:
        if item["status"] in ("FAIL", "WARNING", "ERROR"):
            lines.append(f"- **{item['name']} — {item['status']}**: inspect its command logs and JSON evidence.")
        for finding in item.get("vulnerabilities", []):
            lines.append(f"  - {finding['id']}: {finding['package']} {finding['version']}; patched: {finding['patched']}.")
    lines.extend(["", "## Limitations", ""])
    lines.extend(f"- {limitation}" for limitation in report["limitations"])
    for item in results:
        lines.extend(f"- {limitation}" for limitation in item.get("limitations", []))
    lines.extend(["", "## Recommendations", ""])
    lines.extend(f"- **{entry['check']}**: {entry['action']}" for entry in recommendations)
    (REPORTS / "security-audit.md").write_text("\n".join(lines) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checks", nargs="*", default=["all"], choices=[*CATEGORIES, "all"])
    arguments = parser.parse_args()
    REPORTS.mkdir(parents=True, exist_ok=True)
    selected = set(CATEGORIES if "all" in arguments.checks else arguments.checks)
    results = []
    for name in CATEGORIES:
        if name in selected:
            print(f"[security-audit] {name}", flush=True)
            try:
                results.append(RUNNERS[name]())
            except Exception as exc:  # Preserve a report even if a tool integration fails.
                results.append(category(name, [{"name": name, "status": "ERROR", "detail": str(exc)}]))
        else:
            results.append({"name": name, "status": "SKIPPED", "checks": []})
        write_reports(results)
    overall = next((s for s in ("ERROR", "FAIL", "WARNING", "PASS")
                    if s in {r["status"] for r in results}), "SKIPPED")
    print(f"[security-audit] {overall}: {REPORTS / 'security-audit.md'}", flush=True)
    return 2 if overall == "ERROR" else 1 if overall == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
