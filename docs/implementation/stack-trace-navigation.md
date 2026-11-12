# .NET stack trace links

DotCraft recognizes source locations printed by .NET stack traces, including
`at Namespace.Type.Method() in /repo/Program.cs:line 42` and Windows paths.
The file and location become one terminal link. Ctrl+click (Cmd+click on macOS) uses the
existing Build, Run, and Terminal navigation path to open the source at
the reported line. URL and existing compiler-style file links remain available.

## Validation

The parser regression covers .NET frames, Linux and Windows paths, punctuation,
frames without source locations, and earlier link formats. All six parser tests
passed with:

```sh
rustc --edition=2024 --test crates/rc-term/src/links.rs -o /tmp/dotcraft-stack-links-tests
/tmp/dotcraft-stack-links-tests
```

The application and terminal crates passed a locked, offline `cargo check`.
The full `rc-term` Cargo suite passed 17 unit and 9 session integration tests
with the dedicated `/tmp/dotcraft-stack-trace-target` directory; GPU captures
and the example doctest remain ignored. The Linux `winit` test-backend
declaration is identical to the one in
[terminal-search PR #152](https://github.com/javy-fernandez/dotcraft/pull/152).

## Provenance and limits

This is a native DotCraft Rust extension of the existing terminal link parser.
No third-party code or assets were copied. The file must exist on the local host
for navigation to open it. Frame symbol navigation without a source path and
remote source mapping remain separate work. A frame wrapped across terminal rows
is parsed row by row and will not form one link across the wrap.
