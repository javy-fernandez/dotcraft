# Godot 4 .NET integration

## Scope and behavior

- Opening a folder or `project.godot` recognizes a Godot workspace. The Explorer hides the generated `.godot` cache by default, while Show Hidden Files still reveals it.
- A project using `Godot.NET.Sdk` remains a C# library rather than an invalid `dotnet run` target. DotCraft offers Run Godot Project, Open Godot Editor, and Run Current Godot Scene configurations. Scene paths must stay inside the project, and the selected scene is restored with the workspace session.
- Running a C# project checks that the selected Godot executable is a .NET build, builds the C# project through the existing cancellable .NET process, then starts Godot. Build output goes to the Run console. The Build command and related menu actions target the C# project.
- Debug builds the C# project first, then asks the existing netcoredbg debugger to launch Godot. This supports Linux systems that restrict attaching to sibling processes. The existing breakpoint, step, watch, and variable UI is used. Stop terminates the launched Godot process and debugger.
- The Godot executable is detected from `godot-mono`, `godot4-mono`, `godot`, or `godot4` on `PATH`, or set explicitly in Settings > Tools > Godot .NET executable. A configured macOS `.app` resolves to its executable. DotCraft passes the selected SDK root to Godot as `DOTNET_ROOT` and places the SDK executable on its process `PATH`.

## External editor setup

Godot's C# editor supports **Dotnet > Editor > External Editor > Custom**. Set **Custom Exec Path** to the DotCraft executable and **Custom Exec Path Args** to:

```text
--godot-csharp "{project}" "{file}" {line} {col}
```

The C# placeholders use zero-based coordinates. For **Text Editor > External**, use these flags instead; that setting supplies one-based coordinates:

```text
--godot "{project}" "{file}" {line} {col}
```

On Linux and macOS, a subsequent invocation sends the file and position to an existing DotCraft window through a per-user Unix socket. On Windows, it opens another DotCraft window. Enable Godot's **Text Editor > Behavior > Auto Reload Scripts on External Change** to see external edits promptly in Godot.

## Validation

- `cargo check -p rc-app --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/godot-validation --jobs 2` passed on Linux with Rust 1.99.1.
- All 22 `rc-core` library tests and five focused `rc-dotnet` tests passed, including Godot workspace detection, SDK detection, executable discovery, command planning, and a simulated cancellable process with `DOTNET_ROOT` forwarding.
- The fixture in `tests/fixtures/godot-dotnet` restored and built with .NET SDK 10.0.404 and the local NuGet packages shipped in the official Godot .NET 4.7.2 Linux archive. A headless Godot 4.7.2 run printed `DOTCRAFT_GODOT_READY` and exited successfully. Without `DOTNET_ROOT`, that same engine could not load the C# runtime, which validates the environment forwarding.
- A direct DAP probe with netcoredbg 3.2.0-1092 launched the official Godot .NET 4.7.2 binary and hit a verified C# breakpoint in `Main.cs` line 7 (`LAUNCH_SUCCESS=True`, `BREAKPOINT_STOPPED=True`).
- Nine focused `rc-app` tests passed, covering the external editor argument formats, Unix socket handoff, scene selection and persistence, and build-before-debug selection. The socket test ran outside the sandbox, which blocks socket binding.

## Provenance and limits

The command flags and custom editor settings follow the [Godot command-line reference](https://docs.godotengine.org/en/4.7/tutorials/editor/command_line_tutorial.html), [external editor guide](https://docs.godotengine.org/en/4.7/tutorials/editor/external_editor.html), and [Godot 4.5 C# editor implementation](https://github.com/godotengine/godot/blob/4.5/modules/mono/editor/GodotTools/GodotTools/GodotSharpEditor.cs). No Godot or CrabCraft code or assets were copied. The fixture and integration code are original DotCraft work under the repository's MIT OR Apache-2.0 license.

This integration targets Godot 4 C# projects. GDScript files can be opened as text, but GDScript language-server and debugger protocols, scene-tree inspection, and export presets are not implemented here. Real Windows and macOS runs remain untested.
