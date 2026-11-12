# B-10: Evaluated MSBuild project properties on Linux

## Behavior

`rc-dotnet::msbuild::evaluate` asks the installed .NET SDK for evaluated project properties. It first evaluates the project's outer build to discover its declared target frameworks. For a multi-target project it then evaluates the selected inner build, defaulting to the first declared framework. An explicitly requested framework must be declared by the project; an unknown framework now produces an error instead of an apparently valid `TargetPath`.

The result includes the selected framework, all declared frameworks, output type and assembly path, and whether the SDK identifies the project as a web or test project. A single-property `get_properties` call also supports the SDK's plain-text response.

## Validation and provenance

- Source projects: `tests/fixtures/Shop/src/Shop.Core/Shop.Core.csproj` and `tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj`. The acceptance test copies these projects into a temporary directory and changes only its copy of Shop.Core to declare `net8.0;netstandard2.0` for the multi-target cases.
- SDK: Microsoft's official `dotnet-install.sh` script with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`, yielding .NET SDK **8.0.428** on Linux. The test uses that exact host via `DOTCRAFT_TEST_DOTNET`.
- SDK behavior reference: [MSBuild `-getProperty`](https://learn.microsoft.com/en-us/visualstudio/msbuild/evaluate-items-and-properties) and [outer/inner builds for multiple target frameworks](https://learn.microsoft.com/en-us/visualstudio/msbuild/run-target-exactly-once).
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-msbuild-eval-target cargo test -p rc-dotnet --lib`: **132 passed**.
- `DOTCRAFT_TEST_DOTNET=/var/tmp/dotcraft-dotnet/dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-msbuild-eval-cli-home DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-msbuild-eval-target cargo test -p rc-dotnet --test msbuild_evaluation_live -- --ignored`: **1 passed**, covering the real library and web projects, Debug/Release, first and selected inner builds, target paths, single-property output, and rejection of undeclared frameworks.

## Scope and limits

This verifies B-10's backend property evaluation on Linux with .NET SDK 8.0.428. The live test is ignored by default because it needs a local SDK; CI can opt in with `DOTCRAFT_TEST_DOTNET`. It does not establish Windows/macOS SDK behavior, UI rendering, build success, or a restored NuGet dependency graph. Requested frameworks that are not visible in the outer build are rejected, including unusual projects that introduce their frameworks only through a `TargetFramework` global property.
