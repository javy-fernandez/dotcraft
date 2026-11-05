# DotCraft working rules

- Keep the desktop application in native Rust/egui and follow `docs/architecture.md`.
- `FEATURES.md` is the status of record. Update the relevant English feature entry and
  evidence with each implementation, then run `python3 scripts/features_summary.py`.
- Deliver one feature per branch and pull request. Parallel agents must use isolated
  worktrees and coordinate Cargo target directories to prevent mixed build artifacts.
- Verify changes with appropriate tests and describe any remaining limitations in the PR.
  Do not mark broad Rider parity based only on a parser test or inherited source code.
- Every implementation and documentation commit must include this exact trailer, as
  explicitly requested by the user:

  `Co-authored-by: Codex <noreply@openai.com>`

- Reuse shared CrabCraft code and native UI where useful, preserving source/license
  attribution and adapting Rust-specific workflows to .NET.
