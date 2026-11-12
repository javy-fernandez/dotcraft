# H-17: Coverage gutter on Linux

## Behavior

The editor's coverage gutter paints covered lines green, uncovered lines red, and partly covered branches amber. The Coverage window's “Show Coverage in Editor” control hides and restores these marks. A VCS change bar takes precedence when a line has both marks.

This PR strengthens the existing offscreen app acceptance test. It runs the real Shop solution's Coverlet-enabled tests, checks the editor's covered and uncovered marks, and then checks rendered pixels in the editor gutter. Green `#5FB865` and red `#E55353` bars appear while coverage is shown, disappear when `show_in_editor` is disabled, and return when enabled again. The real Shop report has no partly covered line, so the test inserts one 1/2 branch into that report and verifies that the app maps it to `CoverageMark::Partial` and renders an amber `#E0B040` bar.

Screenshots from the test: [visible](coverage-gutter-visible.png), [hidden](coverage-gutter-hidden.png), [partial](coverage-gutter-partial.png).

## Validation and provenance

- Source fixture: `tests/fixtures/Shop` from this repository. The test invokes the existing app coverage workflow; it does not copy code or assets from another project.
- SDK: Microsoft's official [dotnet-install script](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-install-script), invoked with `--channel 8.0 --install-dir /var/tmp/dotcraft-dotnet --no-path`; installed version **8.0.428** on Linux. The Shop xUnit/NUnit projects use `coverlet.collector` **6.0.0**, resolved from `/var/tmp/dotcraft-cli-home/.nuget/packages`.
- Collector semantics: Microsoft's [code coverage guide](https://learn.microsoft.com/en-us/dotnet/core/testing/unit-testing-code-coverage) describes Coverlet collection and Cobertura reports.
- `DOTNET_ROOT=/var/tmp/dotcraft-dotnet DOTNET_CLI_HOME=/var/tmp/dotcraft-gutter-cli-home NUGET_PACKAGES=/var/tmp/dotcraft-cli-home/.nuget/packages DOTNET_CLI_TELEMETRY_OPTOUT=1 CARGO_TARGET_DIR=/var/tmp/dotcraft-coverage-gutter-live-target cargo test -p rc-app --bin dotcraft screens_coverage -- --ignored --nocapture`: **1 passed**. The offscreen renderer used Vulkan (`radv`); screenshots were visually inspected.

## Scope and limits

This verifies H-17's covered, uncovered, and partial gutter rendering and show/hide behavior on Linux with a 1440×900 offscreen UI. The test is ignored by default because it requires an SDK, NuGet packages, and a GPU-capable renderer. Partial branch data is synthesized from the real Shop report because its merged test result contains no partial line. Pixel assertions use the established 1440×900 layout; other window sizes, VCS bar precedence, Windows/macOS, and theme variants were not exercised here.
