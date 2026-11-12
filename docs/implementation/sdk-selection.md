# Workspace SDK selection

Settings > Tools > Manage project SDK opens a chooser for installed .NET SDKs. Selecting a version and explicitly saving creates or updates `global.json` in the displayed workspace root. Discovery alone does not write a policy or evaluate project code.

## Implemented behavior

- Enumerate installed SDKs on a background task with a 10-second timeout and cancellation when the chooser closes.
- Display the exact workspace policy path and current workspace-local SDK pin.
- Save only a version present in the installed SDK list.
- Change only `sdk.version`, preserving other SDK policies and unrelated JSON fields.
- Preserve existing file permissions and replace the policy through a temporary file in the same directory.
- Refuse invalid/commented JSON, non-object policies, invalid SDK versions, symlinked policy files, oversized files, and stale file snapshots.
- Leave ancestor `global.json` files untouched. Creating a workspace-local file shadows an ancestor policy.
- Return to the original Tools Settings draft after closing or saving. The SDK file save is independent of applying global Settings; Cancel still discards unapplied Settings changes.
- Keep Save and Close outside the chooser body's scroll area, including compact windows and long paths or diagnostics.

The existing .NET CLI workflows resolve SDKs through `global.json`; this feature does not replace their SDK-resolution rules.

## Limits

The snapshot comparison detects edits present before saving but does not lock out a concurrent external writer between comparison and replacement. JSON is rewritten with standard formatting; comments are rejected rather than removed. The chooser edits only the version, not roll-forward, prerelease, SDK-path, test-runner, or MSBuild SDK policies. Existing values for those fields are preserved.

The chooser does not display an effective ancestor pin, install SDKs, or restart a running Roslyn session after a version change. A saved policy can therefore require reopening the workspace or restarting language services before every tool reflects the new SDK. This is a partial SDK-management implementation.

## Validation

```bash
cargo test -p rc-dotnet --lib --offline global_json
cargo test -p rc-app --offline sdk_chooser
cargo test -p rc-app --offline sdk_settings_return_tests
cargo test -p rc-app -p rc-kit --offline i18n::tests
```

Backend tests cover creating a workspace pin without changing its ancestor, preserving policies and unrelated fields in a copied Shop.Api project, malformed/commented JSON, invalid versions, stale snapshots, symlinks, and cancellable SDK discovery.

Headless chooser tests use a controlled CLI executable. They verify that discovery does not write a policy, Save writes the selected version, compact windows keep actions reachable, and both Close and Save restore the original Settings draft. The controlled executable proves discovery/UI behavior and is not evidence of a working .NET installation.

Integration checks passed in the dedicated `/tmp/dotcraft-sdk-validation-target`: six backend policy/discovery tests, two chooser tests, one return-to-Settings test, two existing responsive Settings regressions, five translation checks, and the real two-version SDK integration test below.

The inherited real-SDK integration test is:

```bash
cargo test -p rc-dotnet --test sdk --offline global_json_selection_controls_the_real_sdk_and_preserves_policies
```

It requires both .NET 8 and .NET 10 SDKs. The test passed with installed SDKs 8.0.427 and 10.0.403: it saved each exact pin through the backend, verified `dotnet --version`, built a temporary project using the copied Shop.Core project file, and checked preserved roll-forward, prerelease, and test-runner fields. Validation used `/tmp/dotcraft-dotnet` with temporary CLI/NuGet homes and `RestoreSources=/tmp/dotcraft-empty-nuget-feed`, an empty local package source, so no external package source was needed. This checks SDK resolution and building the net8.0 project under both SDKs; it does not establish compatibility with every project type.

## Provenance and integration

This selectively integrates the existing [SDK selection PR #5](https://github.com/RafaelMendes1407/dotcraft/pull/5), source commit `31a42366b8d052d20a018adf0091029ea591abb6`. It preserves the current responsive Settings layout, attach dialog, HTTP client, and other tool windows.

- [`global_json.rs`](../../crates/rc-dotnet/src/global_json.rs) owns policy loading, saving, and cancellable discovery.
- [`sdk_selection.rs`](../../crates/rc-app/src/ui/sdk_selection.rs) owns chooser state and UI.
- `dialogs.rs` provides the Tools entry point and restores the parent Settings draft.
- Only the PR's SDK-selection locale keys are added to the existing catalogs.

The implementation retains DotCraft's MIT OR Apache-2.0 licensing. No additional network service or dependency is introduced.
