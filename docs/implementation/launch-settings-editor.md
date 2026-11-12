# Launch profile editor

## Behavior

The Run Configuration dialog opens an editor for `Properties/launchSettings.json`. It can add, edit, rename, and remove profiles. The form covers command name, application URLs, program arguments, launch URL, working directory, executable path, environment variables, and whether to launch a browser. Saving refreshes workspace run configurations. Loading and writing run on the shared background runtime.

The writer reads the latest file before modifying one profile, preserving other profiles and fields outside this form, including `$schema`, `iisSettings`, and profile-specific extensions. It writes a complete JSON document, so existing whitespace is reformatted. Existing profile order is retained except that a renamed profile moves to the end.

## Validation

- `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo check -p rc-app --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-137 --jobs 2` passed. The existing `open_files::push` dead-code warning remains.
- `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p rc-dotnet --lib launch::tests --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-137 --jobs 2`: three passed.
- All new UI strings are present in the seven locale files with explicit English fallback values. `git diff --check` passed.

## Provenance and limits

The implementation uses DotCraft's existing launch profile parser and run configuration UI. No third-party code or assets were copied. The repository's MIT OR Apache-2.0 license applies. DotCraft exposes `Project` profiles as run configurations; other command types remain editable in the file but are not offered in its run selector. Unsaved edits to an already open `launchSettings.json` tab are not merged with the form's changes.
