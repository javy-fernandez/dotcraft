# Offline spell checker

The checker parses C# with the existing tree-sitter grammar, checks comments and strings, splits identifiers at camel-case and acronym boundaries, and keeps source character ranges for corrections. C# keywords are skipped. The checker accepts user/project custom words, exposes Hunspell suggestions, discovers installed dictionary pairs by locale, and persists custom words in per-user or per-project files. The implementation uses the pure Rust [spellbook](https://docs.rs/spellbook/0.4.2/spellbook/) crate and does not require a running spelling service or network connection.

The persistent worker discovers and loads dictionaries, parses source snapshots, checks spelling, calculates suggestions, and writes custom words off the UI thread. Superseded editor and commit-message requests are cancelled. The editor displays warning-colored wavy underlines and right-click suggestions; the Commit window shows misspelled words with correction and dictionary actions. Editor settings offer an enable switch and an installed-language selector.

The feature requires a locally installed Hunspell `.aff`/`.dic` pair. Dictionary discovery currently searches the user data directory, the `DOTCRAFT_DICTIONARIES` path list, and common Linux locations. On macOS and Windows, setting `DOTCRAFT_DICTIONARIES` to a directory with pairs is required unless the user data directory contains one. The checker does not provide grammar checking, code-word allowlists beyond C# keywords, or automatic downloads.

## Validation

- `cargo check -p rc-app --locked --offline` with compact profiles passed.
- Four focused app tests passed: C# region/camel-case scanning, commit-message/custom-word behavior, persistent dictionary safety, and a live worker check against an installed `en_US` dictionary without network access.
- `git diff --check` passed.
- No live desktop visual test was run.

No CrabCraft code or assets were copied. Dictionaries are loaded from local installations rather than bundled; users need an installed `.aff`/`.dic` pair for each selected language. `spellbook` is an MPL-2.0 dependency; DotCraft's own source remains MIT OR Apache-2.0.
