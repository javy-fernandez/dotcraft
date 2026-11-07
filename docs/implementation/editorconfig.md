# EditorConfig indentation

The implementation in existing [PR 3](https://github.com/RafaelMendes1407/dotcraft/pull/3),
commit `b6a20eed032ee479f24d4c1a7e3f8fea461b8ba4`, resolves per-file indentation from
`.editorconfig` files. It applies matching indentation preferences to open editors without
reformatting or marking their contents modified.

## Implemented scope

- Read matching sections from the file's directory and ancestors, stopping at `root=true`.
- Apply nearer configurations and later matching sections after earlier properties.
- Resolve `indent_style`, numeric `indent_size`, `indent_size=tab`, `tab_width`, and `unset`.
- Accept numeric indentation values from 1 through 32; ignore invalid values and unsupported
  properties.
- Support tested filename/path globs, brace alternatives, character classes, and case-sensitive
  matching. Embedded `**` matches zero characters or spans multiple directories without
  changing the meaning of single stars, escapes, or character classes.
- Represent indentation levels using tabs and remaining spaces when `indent_size` differs
  from `tab_width`. Tab, nested newlines, Backspace, line outdenting, closing braces, and
  indentation guides use logical columns rather than character counts.
- Preserve tab preference when a logical level is narrower than a physical tab stop.
  For size 2/tab width 4, the first nested level uses two spaces and the next uses a tab.
  The status indicator and LSP formatting requests retain that preference.
- Read configuration files on one cancellable background worker per document collection.
  Only regular files up to 256 KiB are read; Unix reads use nonblocking file opening.
- Apply results by document identity and style revision, discarding stale results.
- Refresh indentation after configuration notifications, focus restoration, disk reloads,
  and file/folder renames.
- Restore indentation detected from the current disk contents when configuration stops
  applying, while preserving effective settings during asynchronous reload resolution.

The rename hook updates document paths and requests indentation for the new location.
Changing the syntax grammar after a filename extension changes is a separate existing gap.

## Validation

The original PR source and dependency changes were tested independently on pristine
DotCraft commit `f30ac7180ce8dd492633235f3c3dbf2f4107e61b`:

```sh
cargo test -p rc-core --lib --offline
cargo test -p rc-app docs::tests --offline -- --nocapture
```

Results: nine core library tests and six application document tests passed. Coverage includes
nested overrides, `unset`, invalid values, cancellation, bounded/nonregular file handling,
real C# fixture edits, style refresh without dirtying documents, reload baselines, renames,
and preservation of existing BOM/CRLF, preview-tab, and disk-reload behavior.

The compatibility fixes extend that implementation with these regression commands:

```sh
cargo test -p rc-core -p rc-text --lib --offline
cargo test -p rc-app docs::tests --offline -- --nocapture
cargo test -p rc-app -p rc-editor mixed_ --offline -- --nocapture
```

Ten core tests, 26 text tests, eight document tests, and the editor closing-brace regression
pass. The glob regression covers embedded and standalone globstars, directory boundaries,
escaped stars, positive/negative character classes, alternatives, marker collisions, Unicode,
and newline-containing names. Editing regressions cover size 4/tab width 3, size 5/width 2,
size 6/width 3, and size 2/width 4. Nested block indentation reaches column 8 from column 4
with size 4/width 3; Tab, Backspace, outdenting, and undo retain correct logical positions. An additional case verifies that space indentation with `indent_size=tab` and no explicit `tab_width` uses the editor tab width rather than a detected two-space indent.

## Compatibility limits

This is an indentation integration rather than full EditorConfig conformance. End-of-line,
encoding, trailing-whitespace, final-newline, and language-specific formatting properties are
outside its scope. Automatic refresh depends on the application's existing watcher and
focus notifications; ancestor files outside watched roots may refresh only on focus.

The built-in JSON/XML formatters still repeat an indentation unit rather than recomputing
tab alignment at each depth. Full-document mixed-indentation formatting remains separate
from the typing, navigation, and editing behavior validated here.

Embedded globstars and mixed indentation follow the relevant behavior in the
[EditorConfig specification](https://spec.editorconfig.org/). Numeric-range globs remain
outside the supported subset. The tests do not establish complete EditorConfig conformance.
