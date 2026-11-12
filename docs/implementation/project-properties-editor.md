# Project properties editor

## Behavior

Solution Explorer's project context menu opens a Properties form for common SDK-style `.csproj` fields. It edits target framework(s), nullable context, C# language version, implicit usings, warnings as errors, package ID, version, authors, description, and SPDX license expression. Empty fields leave their values inherited or remove a local override. When a user changes one target framework to a semicolon-separated list, the form switches from `TargetFramework` to `TargetFrameworks` and removes the conflicting local element.

Loading and saving run on the background runtime. Saving uses DotCraft's minimal XML text-editing helpers, changing only form fields and preserving comments, other elements, line endings, and most surrounding whitespace. A project changed on disk after the form loaded is rejected so the user can reopen and review it. The file is atomically replaced with its permissions preserved.

## Validation

- Compact offline `cargo check -p rc-app` in the isolated `target/issue-123` directory passed (existing `open_files::push` dead-code warning).
- Four focused `rc-dotnet` project-properties tests passed, covering comments, CRLF, XML escaping, all requested fields, conditional fields, self-closing projects, and concurrent disk changes.
- All 19 existing `rc-dotnet::cpm` tests passed after its generic property setter was tightened to skip conditional property elements.
- New UI strings were imported to all seven locale files with explicit English fallback values. `git diff --check` passed.

## Provenance and limits

This is original DotCraft code reusing the repository's existing `rc-dotnet::cpm` text-editing helpers; no third-party code or assets were copied. The repository's MIT OR Apache-2.0 license applies. Property names and NuGet license semantics were checked against [Microsoft's SDK-style project property reference](https://learn.microsoft.com/en-us/dotnet/core/project-sdk/msbuild-props) and [NuGet pack property reference](https://learn.microsoft.com/en-us/nuget/reference/msbuild-targets).

The form shows local unconditional property values, not the result of MSBuild evaluation or values inherited from `Directory.Build.props`. Conditional property values remain in the file and are not edited by this form. Package license editing uses `PackageLicenseExpression`; file-based package licenses are removed when an expression is set. Unsaved edits to an already open project text tab are not merged with the form's changes. A live graphical UI interaction and cross-platform file replacement remain untested.
