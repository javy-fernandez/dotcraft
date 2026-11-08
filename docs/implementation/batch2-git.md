# Batch 2: repository tools

The Git tool window now includes **Repository Tools**. It exposes five explicitly
scoped workflows through native egui controls. Opening the tools does not run any
Git operation. Commands run through the existing serialized VCS worker and its
Cancel action; errors and captured output remain visible in the tools panel.

| Feature | Implemented scope | Evidence and limitations |
| --- | --- | --- |
| I-09 — Clone repositories | Clone an explicit URL or local repository into a new absolute directory, then open the completed clone. Existing destinations and option/helper transport injection are rejected. | Real local clone checks file content, HEAD, source preservation and existing-directory rejection. A headless click test verifies cloning starts only after a click and opening changes the workspace. No authentication manager, checkout options, recursive clone, or welcome-screen clone entry is included. Network transports use the user's existing Git configuration and were not tested against remote services. |
| I-17 — Cherry-pick | Apply one commit to a clean worktree; continue after resolving and staging conflicts; explicitly confirm abort. Dirty working trees, active operations, merge commits and revision ranges are refused. | Real repositories exercise success, conflict, failed unresolved continuation, resolved continuation, abort and dirty-file preservation. Mainline selection, ranges, skip and automatic conflict resolution are outside this scope. |
| I-22 — Worktrees | List registered worktrees, create a new branch and worktree from HEAD, and confirm removal of a linked worktree. Removal never uses force and refuses the current or primary worktree. | Real repository tests cover paths containing spaces, branch creation, listing, clean removal, dirty removal refusal and invalid branch names. These controls require Git 2.36+ for NUL-delimited porcelain output. Moving, locking, pruning, existing-branch checkout and non-UTF-8 path round trips are outside this scope. |
| I-23 — Submodules | Display recursive submodule status and explicitly initialize/update submodules to commits recorded by the parent repository. | A real local submodule test restores its recorded commit after a local deviation. Fixture setup alone permits local file transport; production commands retain Git's transport restrictions. Remote initialization, credentials, adding/removing submodules, tracking remote tips and nested network transfer are not validated here. |
| I-24 — Git LFS | Probe the installed extension, show status/tracked patterns, explicitly track a pattern in `.gitattributes`, and request download with `git lfs pull`. | Validated missing-extension errors and actual tracking/status with an isolated official Git LFS 3.8.1 binary. A local transfer test starts with a pointer file and empty object cache, then verifies `lfs_pull` downloads the complete object, hydrates the working tree and leaves it clean. HTTP/SSH transfers, product setup/install, locks, uploads and history migration remain outside the tested scope. |

Changes that can affect an open working tree refuse to start while editor
documents have unsaved changes. No buffers are silently saved. Aborting a
cherry-pick and removing a worktree require a second click; a headless test proves
cancelling the confirmation does not start a task. Standard Git worktree removal also
deletes ignored files; the confirmation makes that explicit. Another headless test proves
unsaved buffers and their disk contents are preserved.

On Unix, each Git command has a dedicated process group. Cancelling a token or
aborting its future kills that group so Git helpers do not keep running after the
UI reports cancellation. A controlled helper-process test exercises both paths.
On Windows, the existing direct-child kill-on-drop behavior remains; descendant
cleanup has not been validated. Cancellation is not transactional: an interrupted
clone or other mutation can leave partial files or Git operation state for the
user to inspect. The UI does not delete partial clone destinations automatically.
Output shown in the panel is limited to 65,536 characters after capture; full
process output is still collected by the existing Git runner before display.

## Validation

Validated on Linux with Git 2.53.0. No test depends on a remote network service.

```sh
cargo test -p rc-vcs --locked --offline \
  --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-git --jobs 1
cargo test --workspace --bins repository_tools --locked --offline \
  --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-git --jobs 1 -- --nocapture
git diff --check
```

The VCS suite passes 45 tests: 21 unit tests, 18 existing repository integration
tests and six new tools tests. The app selection passes three headless interaction
tests. An additional opt-in LFS transfer test passes when the extension is on PATH,
as documented below. These results establish the scopes above, not Rider parity or
cross-platform validation.

### Isolated Git LFS verification

The host initially had no Git LFS installation. The missing-extension path was
validated first. The official Linux AMD64 archive from
[Git LFS v3.8.1](https://github.com/git-lfs/git-lfs/releases/tag/v3.8.1) was then
downloaded under `/tmp`, and its SHA-256 was checked against both the official
release metadata and `sha256sums.asc` before extracting the executable:

```text
git-lfs-linux-amd64-v3.8.1.tar.gz
e72cf723a4a660438862282e8aecf3885fc8123e88bfcad534cf6b941a354b75
git-lfs/3.8.1 (GitHub; linux amd64; go 1.27.3; git a662a6db)
```

Only the executable was placed at `/tmp/dotcraft-git-lfs-v3.8.1/git-lfs`; the archive
installer was not run, and neither system installation nor global Git configuration
was changed. With that directory prepended to the test process PATH, these pass:

```sh
env PATH=/tmp/dotcraft-git-lfs-v3.8.1:/home/rafael-mendes/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  cargo test -p rc-vcs --test tools lfs_ --locked --offline \
  --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-git --jobs 1 -- --nocapture
env PATH=/tmp/dotcraft-git-lfs-v3.8.1:/home/rafael-mendes/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  cargo test -p rc-vcs --test tools lfs_pull_downloads_local_objects_and_replaces_pointer_files --locked --offline \
  --target-dir /home/rafael-mendes/Documentos/dotcraft/target/upstream-git --jobs 1 -- --ignored --nocapture
```

The transfer fixture installs filters only in temporary repositories with
`git lfs install --local --skip-repo --skip-smudge`. It commits an actual LFS
pointer, clones without checkout, confirms the clone has no LFS object, and calls
the production `Repository::lfs_pull` against the local source. Git LFS's built-in
local file transfer adapter supplies the object; no pushes or remote services are
used. The test is explicitly ignored in the default suite because Git LFS is an
optional external tool. Successful local retrieval does not establish hosted
HTTP/SSH authentication or network transfer behavior.

## Provenance and localization

This is original DotCraft implementation built on its existing VCS worker, Git
command builder, document model and egui controls. No new CrabCraft code or assets
were copied. Existing copyright notices and MIT OR Apache-2.0 licensing remain.
The Unix process cleanup uses `libc`, already present in the dependency lockfile;
`rc-vcs` now declares it directly for Unix targets.

All new UI source strings use `tr`/`trf`. `scripts/i18n/batch2-git.tsv` provides
English fallbacks for all seven supported non-English locales for the consolidated
batch integration. Captured Git diagnostics and repository paths remain data.
