# B-06 Project templates: live .NET 8 acceptance

## Behavior and provenance

DotCraft's existing `NewProject` flow uses the installed .NET SDK templates, adds the generated project to a solution, and opens its primary source file from the desktop dialog. This change adds live acceptance coverage for both an existing solution and a new solution; it does not copy template code or assets from another repository.

The tests use Microsoft's official .NET SDK 8.0.428, installed locally with `dotnet-install.sh --channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`, on Linux x86_64. The existing `tests/fixtures/Shop` solution is copied into a temporary directory. The fixture and test outputs do not change the source tree.

## Validation

Run with `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet`, `DOTNET_ROOT=/var/tmp/dotcraft-dotnet`, an isolated `DOTNET_CLI_HOME`, and `CARGO_TARGET_DIR=/var/tmp/dotcraft-project-templates-live-target`:

```text
cargo test -p rc-dotnet --test project_templates_live -- --ignored --nocapture
cargo test -p rc-app project_template_dialog_creates_and_opens_project_in_shop -- --ignored --nocapture
cargo test -p rc-dotnet --lib
```

The backend test creates `Shop.Added` from the `classlib` template, checks the seventh solution member, builds its DLL, and rejects a duplicate without running a command. It also creates a new solution with `Fresh.Cli` from the `console` template, builds it, and runs its DLL to check the expected greeting. The app test clicks **Create** for a class library in a copied Shop solution, then checks that the workspace lists the new project and its generated `Class1.cs` is active.

## Limits

The live acceptance covers .NET 8 `classlib` and `console` templates on Linux. Other template families, SDK versions, platforms, Git initialization, and failure recovery after an SDK command fails still need separate acceptance. Invalid AddTo solution preflight is tracked separately in PR #187.
