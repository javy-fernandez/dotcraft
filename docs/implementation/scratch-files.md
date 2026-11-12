# Persistent scratch files

## Behavior

- File > New Scratch File creates a plain file in DotCraft's user data directory under `scratches/`. The name can include an extension such as `.cs`, `.json`, `.http`, `.sql`, or `.md`, so the existing editor chooses the matching language support.
- Solution Explorer shows a separate Scratches root beside the project tree. Files and folders below it can be opened, renamed, and deleted through the existing Explorer actions. Locate Opened File expands this tree for a scratch file.
- The directory is shared across workspaces and remains on disk across sessions. `DOTCRAFT_HOME` also redirects it for portable installs and tests. Scratch files are kept outside the project and are not added to its Git repository.

## Validation

- `cargo test -p rc-app --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-125 scratches_are_visible_outside_the_workspace_and_can_be_relocated -- --test-threads=1` passed.
- The focused `scratch_file_names_cannot_escape_the_data_directory` test passed in the same Cargo target directory.
- `git diff --check` passed.

## Provenance and limits

This is original DotCraft code. No third-party code or assets were copied. The repository's MIT OR Apache-2.0 license applies.

The Scratches node is shown in Solution Explorer while a workspace is open. File > New Scratch File also works without an open workspace, opening the created file in an editor tab.
