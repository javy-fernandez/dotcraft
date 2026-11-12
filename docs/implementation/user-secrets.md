# User Secrets from Solution Explorer

## Behavior

- A C# project context menu offers **Manage User Secrets**. The action uses the selected .NET SDK's `dotnet user-secrets init --project` when the project lacks `UserSecretsId`, then opens the user's `secrets.json` in DotCraft's JSON editor.
- Initialization and file creation run in the background and can be cancelled by closing the window or starting the action for another project. Existing secret values are never overwritten.
- The file lives in the Secret Manager profile directory, outside the workspace. On Unix, newly created secret files have mode `0600` and their containing directory has mode `0700`. Atomic saves preserve an existing file's permissions.
- DotCraft excludes user-secrets files from Local History on save and external reload, avoiding another copy of their contents.

## Validation

- Focused tests cover `UserSecretsId` parsing, path validation, initial file creation, non-overwrite behavior, Unix file permissions, and permission preservation on atomic saves.
- `cargo check -p rc-app --locked --offline` passed with a separate compact target directory. Both focused `rc-app` tests and the private atomic-save `rc-core` test passed.
- The installed .NET SDK 10.0.404 successfully ran `dotnet user-secrets init --project` against a temporary C# project and added `UserSecretsId`.
- All seven locale JSON files parsed and `git diff --check` passed.

## Provenance and limitations

The command and profile locations follow the [Microsoft Secret Manager documentation](https://learn.microsoft.com/en-us/aspnet/core/security/app-secrets?view=aspnetcore-10.0). This is original DotCraft code; no third-party code or assets were copied. The repository's MIT OR Apache-2.0 licensing applies.

The implementation reads `UserSecretsId` from the selected `.csproj`; an ID defined only in an imported MSBuild file is not resolved here. Secret Manager is intended for development and does not encrypt values. The feature has been validated on Linux; a Windows or macOS UI run remains pending.
