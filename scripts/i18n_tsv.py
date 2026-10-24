#!/usr/bin/env python3
"""Imports a tab-separated translation table into the locales.

Usage: python3 scripts/i18n_tsv.py AREA scripts/i18n/file.tsv
Columns: en, es, fr, pt-BR, de, pl, zh-Hans, ja. Lines starting with '#' are comments.
Leading spaces in a cell are kept (texts that follow a number, like " columns").
"""
import json, subprocess, sys, tempfile
from pathlib import Path

CODES = ["es", "fr", "pt-BR", "de", "pl", "zh-Hans", "ja"]
area, path = sys.argv[1], sys.argv[2]
out = {}
for n, line in enumerate(Path(path).read_text(encoding="utf-8").split("\n"), 1):
    if not line.strip() or line.startswith("#"):
        continue
    cols = line.split("\t")
    if len(cols) != 8:
        sys.exit(f"line {n}: {len(cols)} columns: {line!r}")
    out[cols[0]] = dict(zip(CODES, cols[1:]))
with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False)
subprocess.run([sys.executable, str(Path(__file__).parent / "i18n_add.py"), area, f.name], check=True)
print(f"{len(out)} entries")
