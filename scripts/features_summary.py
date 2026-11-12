#!/usr/bin/env python3
"""Generate the high-level feature checklist from FEATURES.md."""

from __future__ import annotations

import re
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "FEATURES.md"
OUTPUT = ROOT / "docs" / "features.md"
ISSUE = re.compile(r"\[#(\d+)\]\(https://github\.com/javy-fernandez/dotcraft/issues/\1\)")
PR = re.compile(r"\[#(\d+)\]\(https://github\.com/javy-fernandez/dotcraft/pull/\1\)")
STATUSES = {"Closing PR", "Partial PR"}


def records() -> list[tuple[int, str, str, str]]:
    entries = []
    seen = set()
    for line in SOURCE.read_text().splitlines():
        if not line.startswith("| [#"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) != 6:
            raise ValueError(f"Expected six cells: {line}")
        issue, feature, status, prs, _evidence, _remaining = cells
        match = ISSUE.fullmatch(issue)
        if not match or status not in STATUSES or not PR.search(prs):
            raise ValueError(f"Invalid issue, status, or PR: {line}")
        number = int(match.group(1))
        if number in seen:
            raise ValueError(f"Duplicate issue #{number}")
        seen.add(number)
        entries.append((number, feature, status, prs))
    if not entries:
        raise ValueError("No feature rows found in FEATURES.md")
    return entries


def main() -> None:
    entries = records()
    counts = Counter(status for _, _, status, _ in entries)
    lines = [
        "# Feature checklist and PR index",
        "",
        "Generated from [FEATURES.md](../FEATURES.md) by `python3 scripts/features_summary.py`.",
        "The issues below were open on 2026-11-12. Every listed PR is open against the original repository's `main`; unchecked means not merged and does not imply Rider parity.",
        "",
        f"**{len(entries)} open issues:** {counts['Closing PR']} with a closing PR pending review and merge; {counts['Partial PR']} with only partial PRs.",
        "",
        "| Issue | Requested feature | PR status | PR index |",
        "| --- | --- | --- | --- |",
    ]
    for number, feature, status, prs in entries:
        lines.append(
            f"| - [ ] [#{number}](https://github.com/javy-fernandez/dotcraft/issues/{number}) "
            f"| {feature.split(':', 1)[0]} | {status} | {prs} |"
        )
    lines += [
        "",
        "A closing PR uses `Closes #...` and may close its issue after merge. A partial PR uses `Refs #...` and leaves its issue open. Detailed scope, evidence paths, validation, and limitations are in [FEATURES.md](../FEATURES.md) and the linked PRs.",
        "",
    ]
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text("\n".join(lines))
    print(f"Wrote {OUTPUT.relative_to(ROOT)}: {len(entries)} issues, {counts['Closing PR']} closing, {counts['Partial PR']} partial")


if __name__ == "__main__":
    main()
