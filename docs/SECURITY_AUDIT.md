# Security and compliance audit

This is a technical self-assessment pipeline, not a certification of the
application or of regulatory compliance. It leaves the native Rust app and its
build and release workflows intact.

## Architecture

`scripts/security-audit.sh` is the local entry point. Its Python orchestrator
runs each installed tool without a shell, captures stdout, stderr and exit
codes, and writes `security/reports/security-audit.json` plus a Markdown
summary after every check. `PASS`, `FAIL`, `WARNING`, `SKIPPED` and `ERROR`
remain distinct. An absent tool or malformed output is `ERROR`, never `PASS`.
Generated evidence is ignored by Git and should be reviewed before sharing.

The dedicated [workflow](../.github/workflows/security-audit.yml) runs on pull
requests to `main`, pushes to `main`, a Monday schedule and manual dispatch.
Separate Linux jobs run the dependency audit, policy checks, static analysis,
unsafe scan, secret scan, SBOM and security tests. Fuzz smoke tests run on the
schedule and manual dispatch only, so pull requests do not spend unbounded time
fuzzing. Every job uploads its report even when the check fails, and a final
job that always runs combines the job reports. A missing job artifact is `ERROR`.

## Local commands and tools

From the repository root:

```sh
./scripts/security-audit.sh all
./scripts/security-audit.sh dependencies licenses
./scripts/security-audit.sh static
./scripts/security-audit.sh unsafe
./scripts/security-audit.sh secrets
./scripts/security-audit.sh sbom
./scripts/security-audit.sh tests
./scripts/security-audit.sh fuzz
python3 -m unittest scripts.test_security_aggregate scripts.test_security_audit
```

Exit codes: `0` for PASS or WARNING, `1` for FAIL, `2` for ERROR. The script
needs Python 3, Rust and Cargo, and the relevant tools in `PATH`: cargo-audit
0.23.0, cargo-deny 0.20.2, cargo-geiger 0.14.0, cargo-cyclonedx 0.5.10,
Gitleaks 8.30.1 and cargo-fuzz 0.13.2 with a nightly toolchain for fuzzing. The
workflow installs pinned tool versions, and Gitleaks is downloaded with a
checked SHA-256. For a local scan with an already populated RustSec database,
set `SECURITY_AUDIT_OFFLINE=1`. `GITLEAKS_BIN` may point to a locally installed
binary. `SECURITY_FUZZ_TOOLCHAIN` selects the nightly toolchain; the workflow
uses `nightly-2026-11-11`.

`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
covers the entire workspace on Linux; the crates have no Cargo feature flags.
DotCraft is formatted by hand and has no rustfmt configuration, so `cargo fmt
--check` is not part of the gate: running it would rewrite most source files.
macOS-only Objective-C code and Windows-only credential code are not compiled
by this Linux workflow and need a separate platform review. The release
workflows still build the macOS, Windows and Linux packages.

## Policies and exceptions

`deny.toml` checks RustSec advisories, the licenses present in the resolved
dependency graphs, crates.io as the only permitted registry, unknown Git sources
and wildcard or duplicate versions. Parallel versions are recorded as warnings
because the cross-platform GUI graph contains several upstream major versions.
Local `rc-*` path dependencies appear as wildcards in Cargo metadata and are
also warnings. Any new external Git source or registry fails. `NCSA` is allowed
for `libfuzzer-sys` in the separate fuzz graph. The allow list is a distribution
policy, not proof that a package is legally compatible in every use; review
`docs/third-party/` and asset notices before changing it.

Critical, high and medium vulnerability advisories block, as do advisories
without a reliable severity. The cargo-audit report records low and
informational findings as warnings, but cargo-deny 0.20.2 treats every
vulnerability advisory as an error, so the combined workflow stays blocked until
such a finding is reviewed. Only an explicit numerical CVSS score is classified:
critical 9.0–10, high 7.0–8.9, medium 4.0–6.9, low 0.1–3.9, informational 0. A
CVSS vector or title alone does not provide a score. Unmaintained and unsound
notices are warnings.

Exceptions require an identifier, narrow scope, justification, owner, review
date and expiry in `security/exceptions.json`. The script checks expiry and
requires `.gitleaksignore` to match the Gitleaks entries exactly. There are no
exceptions at introduction, and no RustSec advisory is ignored. Adding a broad
allowlist or discarding a failed command is not acceptable.

## Evidence and coverage

The root `Cargo.lock` and the independent `fuzz/Cargo.lock` are both audited,
and both Cargo graphs are checked by cargo-deny. `cargo-geiger` scans Rust
`unsafe` use in `rc-app` and its dependencies, and the audit writes an
own-source inventory. See [unsafe-review](../security/unsafe-review.md) for the
safety arguments and platform review priorities. The presence of `unsafe` is not
itself a vulnerability. Gitleaks scans the Git history with redaction and no
persisted checkout credentials; it does not see ignored or untracked files.

`cargo-cyclonedx` creates one CycloneDX JSON per workspace crate. The `rc-app`
SBOM is copied to `security/reports/sbom.cdx.json`, and all crate SBOMs remain
under `sbom-components/`. These files inventory Cargo packages, not a complete
installed system: the binary also contains bundled SQLite, tree-sitter grammars,
platform libraries and the fonts and icons in `assets/`. It downloads external
tools such as netcoredbg at run time. Release `ldd` output and the Windows DLL
check supply separate evidence.

Security tests run the `rc-core` and `rc-dotnet` library tests (session and
settings persistence, atomic writes, project, NuGet and report parsers) and the
SQLite tests in `rc-app`'s `database` module. The fuzz targets exercise
untrusted session and settings JSON, project files and `launchSettings.json`,
and TRX and Cobertura reports. Inputs are capped at 16 KiB and each target runs
for ten seconds in the smoke job. AddressSanitizer stays enabled; LeakSanitizer
is disabled because some container ptrace policies prevent it from completing.
Deep fuzzing and corpus triage are manual follow-up work.

`Session::clear(workspace)` deletes a workspace's persisted session, and
`Settings::clear()` deletes preferences and the backup of an unreadable settings
file. A complete data erasure workflow still needs to cover local history, logs,
downloaded tools and credentials kept in the operating system's store.

## Baseline at introduction

`cargo audit` reported RUSTSEC-2026-0459 in `libsqlite3-sys` 0.38.2 and
RUSTSEC-2026-0433 in `pulldown-cmark` 0.13.5, plus the unsound RUSTSEC-2026-0486
in `rustix` 1.1.5. `rusqlite` moved from an exact `=0.40.1` pin to 0.41.1
(`libsqlite3-sys` 0.39.1), `pulldown-cmark` to 0.13.6 and `rustix` 1.x to 1.1.6.
The SQLite reader in `rc-app/src/database.rs` was adapted to the 0.41 API. One
warning remains: RUSTSEC-2026-0097 (unsound) in `rand` 0.7.3, reached only
through `tiberius` → `winauth` for SQL Server Windows authentication. No patched
`winauth` release is available, so the warning is recorded rather than ignored.

Strict Clippy reported 15 findings across `rc-app`, `rc-dotnet`, `rc-kit` and
`rc-text`. They were corrected without changing behavior: test modules moved to
the end of their files, a large debugger enum variant boxed, a macOS-only helper
gated by platform, a test that holds the home-directory lock across `await`
documented, and smaller idiom fixes.

## Maintenance

When updating dependencies or tools, review upstream release notes and
licenses, update the pinned version and checksum or SHA, rerun the local audit,
and inspect the CI artifacts. Source checksums in `Cargo.lock` and locked Cargo
resolution provide registry integrity; they do not attest build scripts or
external native libraries. GitHub Actions in the security and release workflows
are pinned to commit SHAs, checkout does not persist credentials, the audit
jobs use read-only repository permission, and pull request code receives no
secrets. Dependabot checks Cargo and GitHub Actions weekly.

## Provenance

The audit tooling is adapted from CrabCraft (MIT OR Apache-2.0; see
`docs/third-party/CrabCraft-LICENSE-MIT.txt`),
`RafaelMendes1407/CrabCraft` at commit `203b372f0c7f7a9f2d80a5220f13e34fe3d7ce86`,
introduced there in `a7208ee`, `87f1139`, `b935ad5`, `2f0b307`, `2b809e3` and
`75ec333`. DotCraft changes: fuzz targets for .NET project, launch settings and
test report parsers; `rc-dotnet` and `database` security tests; rustfmt removed
from the static gate; the unused static-baseline hook removed.

Primary references: [RustSec cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit),
[cargo-deny](https://embarkstudios.github.io/cargo-deny/),
[cargo-geiger](https://github.com/rust-secure-code/cargo-geiger),
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz),
[CycloneDX Rust](https://github.com/CycloneDX/cyclonedx-rust-cargo),
[Gitleaks](https://github.com/gitleaks/gitleaks).
