# Verified revisions in Git reads (I-10 slice)

DotCraft now resolves caller-supplied commit references to full hashes before passing them to `git blame` and `git cat-file`. This keeps dash-leading text from being interpreted as a Git option. Normal references such as `HEAD~1` continue to work. A missing or invalid commit returns a `NotFound` error from blame and `None` from file lookup, matching each API's existing missing-content behavior.

This change uses DotCraft's existing `Repository::resolve` helper and native `rc-vcs` Git CLI integration. It does not copy CrabCraft code or assets.

Validation: `cargo test -p rc-vcs` in a dedicated Cargo target directory. A real temporary Git repository verifies historical blame and file content, rejects dash-leading option-shaped inputs, and rejects a blob expression where a commit is required.

The I-10 audit remains partial. Other read paths, filesystem boundary cases, unusual ref names, and platform-specific Git behavior still need review. No user interface changes are included.
