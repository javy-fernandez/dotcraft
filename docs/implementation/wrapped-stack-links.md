# Wrapped .NET stack frame links

## Scope

The terminal recognizes a `.NET` stack frame's source file and line when its
output wraps across terminal grid rows. Hovering or clicking the source text on
either visual row resolves to the same source location. Explicit line breaks
remain separate, and OSC 8 hyperlinks keep precedence over detected text links.

This extends the canonical stack frame parser already present in
`javy-fernandez/dotcraft` at `9d93af4`. The change is native DotCraft code; no
third-party code or assets are copied.

## Validation

The `rc-term` grid tests cover both sides of a soft wrap, a hard CRLF break,
OSC 8 hyperlink precedence, and the long-line fallback:

```sh
cargo test -p rc-term --lib view::tests --locked --offline --target-dir target/validation-wrapped-links --jobs 2
cargo test -p rc-term --lib --locked --offline --target-dir target/validation-wrapped-links --jobs 2
```

The focused group passed 4 tests. The full `rc-term` library group passed 21
tests, including the new grid tests.
The first dependency build required network access because `tiberius` was not
cached. The second command used the resulting local cache. A repository-wide
`cargo fmt --all -- --check` still reports formatting differences present in
the upstream baseline; this slice keeps unrelated files untouched.

## Remaining limitations

Remote source mapping, symbol-only frames, and live UI validation on Windows
and macOS remain outside this slice. Wrapped link detection examines at most
64 grid rows per lookup; longer logical lines retain ordinary single-row link
detection.
