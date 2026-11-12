# Assembly Explorer

The Tools menu can open a managed `.dll` or `.exe`. Package nodes in Solution Explorer can open a cached NuGet package assembly. The Assembly Explorer can also list installed .NET reference-pack DLLs for direct selection. DotCraft runs the optional `ilspycmd` tool on background threads to list classes, interfaces, structs, delegates and enums. The window groups types by namespace, lists members from the decompiled C# syntax tree, and shows C# or IL for the selected type. Switching the type or closing the window cancels an in-flight process.

`ilspycmd` must be installed separately (`dotnet tool install --global ilspycmd`) and available on `PATH`, in `~/.dotnet/tools`, or via `DOTCRAFT_ILSPYCMD`. It is an [MIT-licensed](https://github.com/icsharpcode/ILSpy/blob/master/LICENSE) external tool; no ILSpy or CrabCraft source or assets are included in DotCraft. The command-line options are from the [official ILSpy CLI documentation](https://github.com/icsharpcode/ILSpy/blob/master/ICSharpCode.ILSpyCmd/README.md).

Package resolution uses `NUGET_PACKAGES` or the default user cache. When `obj/project.assets.json` exists, the selected runtime or compile asset follows restore's target entry; multi-target projects offer a framework choice in the package context menu. Otherwise DotCraft selects a matching DLL under `lib` or `ref`. Reference-pack discovery uses the installed `dotnet` host's `packs` directory. Member selection narrows C# and method IL; IL for properties and fields remains at type scope. This is a remaining limit of issue #142.

## Validation

- Seven focused app tests cover parsing, C#/IL member browsing, package-cache and multi-target restored-asset resolution, reference-pack discovery, and a live `ilspycmd` run against `Newtonsoft.Json.dll` from the .NET 10 SDK.
- The live test listed types and produced both decompiled C# and IL in an isolated `/tmp` tool installation.
- `git diff --check` passed. No desktop screenshot was captured.
