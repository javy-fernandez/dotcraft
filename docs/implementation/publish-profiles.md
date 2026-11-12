# Folder publish profiles

## Behavior

Build > Publish Project opens a form for selecting a project and creating or editing a folder publish profile in `Properties/PublishProfiles/<name>.pubxml`. The form covers output folder, build configuration, runtime identifier, self-contained deployment, single-file output, ReadyToRun, and trimming. Save writes the profile; Publish saves it and starts `dotnet publish <project> -c <configuration> -p:PublishProfile=<name>` through DotCraft's existing cancellable Build window process.

Profile discovery, loading, and saving run off the UI thread. Editing an existing profile retains unrecognized XML elements while replacing the fields exposed by the form. New profiles use the `FileSystem` publish method and the `PublishDir` property, which the .NET CLI honors. Profile names are restricted to ASCII letters, digits, hyphens, and underscores so they stay inside the project's publish profile directory.

## Validation

- Compact offline `cargo check -p rc-app` in the isolated `target/issue-121` directory passed (existing `open_files::push` dead-code warning).
- Two focused `rc-dotnet` publish profile tests passed, covering `.pubxml` round-trip, unknown XML retention, and command planning.
- A local .NET SDK 10.0.404 folder-publish smoke test with the generated profile shape passed: `dotnet publish App.csproj -c Release -p:PublishProfile=Folder --no-restore` wrote to the configured folder, and the published app printed `DOTCRAFT_PUBLISH_READY`.
- All seven locale JSON files parsed after the new keys were imported with explicit English fallback values. `git diff --check` passed.

## Provenance and limits

The implementation is original DotCraft code. No third-party code or assets were copied; the repository's MIT OR Apache-2.0 license applies. Options and property names were checked against [Microsoft's `dotnet publish` reference](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-publish) and [single-file deployment guide](https://learn.microsoft.com/en-us/dotnet/core/deploying/single-file/).

The form supports folder publishing only. It does not configure Azure, container, FTP, or Web Deploy targets. Editing an existing profile preserves unknown XML elements but does not preserve formatting around fields that it changes. A live graphical UI interaction and Windows/macOS publish runs remain untested.
