# Assembly Explorer

The Tools menu can open a managed `.dll` or `.exe`. Package nodes in Solution Explorer can open a cached NuGet package assembly. DotCraft runs the optional `ilspycmd` tool on background threads to list classes, interfaces, structs, delegates and enums. The window groups types by namespace, lists members from the decompiled C# syntax tree, and shows C# or IL for the selected type. Switching the type or closing the window cancels an in-flight process.

`ilspycmd` must be installed separately (`dotnet tool install --global ilspycmd`) and available on `PATH`, in `~/.dotnet/tools`, or via `DOTCRAFT_ILSPYCMD`. It is an [MIT-licensed](https://github.com/icsharpcode/ILSpy/blob/master/LICENSE) external tool; no ILSpy or CrabCraft source or assets are included in DotCraft. The command-line options are from the [official ILSpy CLI documentation](https://github.com/icsharpcode/ILSpy/blob/master/ICSharpCode.ILSpyCmd/README.md).

Package resolution uses `NUGET_PACKAGES` or the default user cache and selects a matching DLL under `lib` or `ref`. It does not yet enumerate framework reference-pack assemblies or resolve which target-framework asset MSBuild chose. IL is shown for a type; member selection narrows the C# view only. These are remaining limits of issue #142.

## Validation

- Four focused app tests cover parsing, member browsing, package-cache resolution, and a live `ilspycmd` run against `Newtonsoft.Json.dll` from the .NET 10 SDK.
- The live test listed types and produced both decompiled C# and IL in an isolated `/tmp` tool installation.
- `git diff --check` passed. No desktop screenshot was captured.
