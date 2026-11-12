# Typed submodule status (I-23 backend slice)

DotCraft now exposes recursive submodule entries with their path, checked-out commit, status, and optional Git description. The status distinguishes a checkout at its recorded commit, an uninitialized checkout, a different commit, and a conflict. The existing raw status API remains available. Recursive updates now accept an optional cancellation token; the existing update method delegates to it.

The implementation extends the native `rc-vcs` Git CLI backend. It does not copy CrabCraft code or assets. It uses Git's `submodule status --recursive` output and runs updates through the existing child-process cancellation path.

Validation: `cargo test -p rc-vcs` in a dedicated Cargo target directory. Unit tests cover all four status markers, paths with spaces, optional descriptions, and malformed rows. A real local Git test verifies detection of a changed submodule commit, cancellation before update, and restoration to the recorded commit.

This is a backend slice. The app has no submodule UI yet. The parser assumes one status entry per line, matching Git's current text output; newline-bearing submodule paths are not supported. Live network, nested submodules, and other platforms remain untested. I-23 remains partial until UI and acceptance evidence exist.
