#!/usr/bin/env python3
"""Adds or updates interface translations in every locale.

Usage: python3 scripts/i18n_add.py AREA strings.json
where strings.json maps each English text to {"es": ..., "fr": ..., "pt-BR": ..., "de": ...,
"pl": ..., "zh-Hans": ..., "ja": ...}. Entries keep their area; files stay sorted by area and
written with the 4-space layout the i18n tests expect (one entry per line).
"""
import json
import sys
from pathlib import Path

LOCALES = ["es", "fr", "pt-BR", "de", "pl", "zh-Hans", "ja"]
ROOT = Path(__file__).resolve().parent.parent / "locales"


def load(code):
    path = ROOT / f"{code}.json"
    if not path.exists():
        return {}
    return json.loads(path.read_text(encoding="utf-8"))


def save(code, data):
    lines = ["{"]
    areas = list(data.items())
    for ai, (area, entries) in enumerate(areas):
        lines.append(f"  {json.dumps(area, ensure_ascii=False)}: {{")
        items = list(entries.items())
        for i, (k, v) in enumerate(items):
            comma = "," if i + 1 < len(items) else ""
            lines.append(f"    {json.dumps(k, ensure_ascii=False)}: {json.dumps(v, ensure_ascii=False)}{comma}")
        lines.append("  }" + ("," if ai + 1 < len(areas) else ""))
    lines.append("}")
    (ROOT / f"{code}.json").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    area, src = sys.argv[1], sys.argv[2]
    strings = json.loads(Path(src).read_text(encoding="utf-8"))
    for code in LOCALES:
        data = load(code)
        # A key lives in one area only: move it if it was elsewhere.
        for other in data.values():
            for en in strings:
                other.pop(en, None)
        target = data.setdefault(area, {})
        for en, tr in strings.items():
            if code not in tr:
                sys.exit(f"missing {code} for {en!r}")
            target[en] = tr[code]
        data = {a: e for a, e in data.items() if e}
        save(code, data)


if __name__ == "__main__":
    main()
