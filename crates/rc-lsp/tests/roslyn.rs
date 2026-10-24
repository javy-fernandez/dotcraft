//! Phase 2 closure against the real Roslyn server and .NET SDK: a two-project solution loads,
//! completion answers in under 150 ms, rename spans both projects, diagnostics arrive.
//! The server is downloaded once into `target/test-home` (or set `DOTCRAFT_ROSLYN`).

use lsp_types::{CompletionResponse, Position, WorkspaceEdit};
use rc_core::Waker;
use rc_lsp::roslyn::{self, ServerCommand};
use rc_lsp::{LanguageSession, LspEvent, ServerState, path_to_uri, uri_to_path};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn test_home() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home");
    std::fs::create_dir_all(&p).unwrap();
    p.canonicalize().unwrap()
}

fn server() -> ServerCommand {
    // SAFETY: set before any thread of this test reads it.
    unsafe { std::env::set_var("DOTCRAFT_HOME", test_home()) };
    let configured = std::env::var_os("DOTCRAFT_ROSLYN").map(PathBuf::from);
    if let Some(cmd) = roslyn::locate(configured.as_deref(), Path::new("dotnet")).filter(|c| c.origin != roslyn::Origin::VsCodeExtension) {
        return cmd;
    }
    let inbox = rc_core::Inbox::new(Waker::noop());
    rc_core::Runtime::block_on(roslyn::download(inbox.sender()));
    for ev in inbox.drain() {
        if let roslyn::DownloadEvent::Failed(e) = ev {
            panic!("download failed: {e}");
        }
    }
    roslyn::locate(None, Path::new("dotnet")).expect("downloaded server")
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// Lib (Greeter.Hello) and App (calls it), in a .slnx, restored.
fn solution(dir: &Path) -> PathBuf {
    let proj = |extra: &str| format!("<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><TargetFramework>net8.0</TargetFramework><Nullable>enable</Nullable>{extra}</PropertyGroup>\n</Project>\n");
    write(&dir.join("Lib/Lib.csproj"), &proj(""));
    write(&dir.join("Lib/Greeter.cs"), "namespace Lib;\n\npublic class Greeter\n{\n    public string Hello(string name) => $\"Hello {name}\";\n}\n");
    write(
        &dir.join("App/App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net8.0</TargetFramework></PropertyGroup>\n  <ItemGroup><ProjectReference Include=\"..\\Lib\\Lib.csproj\" /></ItemGroup>\n</Project>\n",
    );
    write(&dir.join("App/Program.cs"), "using Lib;\n\nvar greeter = new Greeter();\nSystem.Console.WriteLine(greeter.Hello(\"world\"));\n");
    let sln = dir.join("Demo.slnx");
    write(&sln, "<Solution>\n  <Project Path=\"App/App.csproj\" />\n  <Project Path=\"Lib/Lib.csproj\" />\n</Solution>\n");
    let out = std::process::Command::new("dotnet").args(["restore", sln.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "restore failed: {}", String::from_utf8_lossy(&out.stdout));
    sln
}

fn pump_until(s: &mut LanguageSession, timeout: Duration, mut done: impl FnMut(&LspEvent, &LanguageSession) -> bool) -> Vec<LspEvent> {
    let start = Instant::now();
    let mut all = Vec::new();
    while start.elapsed() < timeout {
        for e in s.pump() {
            let stop = done(&e, s);
            if let LspEvent::StateChanged(ServerState::Failed(why)) = &e {
                panic!("server failed: {why}");
            }
            all.push(e);
            if stop {
                return all;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out; state {:?}", s.state);
}

fn wait<T>(mut p: rc_lsp::Pending<T>) -> Result<T, rc_lsp::RpcError> {
    let start = Instant::now();
    loop {
        if let Some(r) = p.poll() {
            return r;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "request timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn roslyn_completion_rename_and_diagnostics() {
    let cmd = server();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let sln = solution(&root);
    let mut s = LanguageSession::start(&cmd, &root, Some(&sln), Vec::new(), None, Waker::noop()).unwrap();
    pump_until(&mut s, Duration::from_secs(180), |e, _| matches!(e, LspEvent::StateChanged(ServerState::Ready)));
    eprintln!("solution loaded in {:?}", s.started.elapsed());

    let program = root.join("App/Program.cs");
    let text = std::fs::read_to_string(&program).unwrap();
    s.did_open(&program, "csharp", &text);
    let uri = path_to_uri(&program);
    // Pulling diagnostics makes Roslyn attach the opened document to its project.
    let _: serde_json::Value = wait(s.client.request_raw("textDocument/diagnostic", json!({"textDocument": {"uri": uri.as_str()}}))).unwrap();

    // Completion right after "greeter." on line 3.
    let col = text.lines().nth(3).unwrap().find("greeter.").unwrap() as u32 + 8;
    let complete = |s: &LanguageSession| {
        s.client.request_raw::<Option<CompletionResponse>>(
            "textDocument/completion",
            json!({"textDocument": {"uri": uri.as_str()}, "position": {"line": 3, "character": col}, "context": {"triggerKind": 1}}),
        )
    };
    let first = Instant::now();
    let mut items = Vec::new();
    // Right after `didOpen` the document may still be attaching to its project.
    for attempt in 0..40 {
        let raw: serde_json::Value = wait(s.client.request_raw("textDocument/completion", json!({"textDocument": {"uri": uri.as_str()}, "position": {"line": 3, "character": col}, "context": {"triggerKind": 1}}))).unwrap();
        items = match serde_json::from_value::<Option<CompletionResponse>>(raw.clone()).unwrap() {
            Some(CompletionResponse::List(l)) => l.items,
            Some(CompletionResponse::Array(a)) => a,
            None => Vec::new(),
        };

        if !items.is_empty() {
            eprintln!("completion ready after {attempt} retries");
            break;
        }
        s.pump();
        std::thread::sleep(Duration::from_millis(250));
    }
    eprintln!("first completion: {:?}, {} items", first.elapsed(), items.len());
    assert!(items.iter().any(|i| i.label == "Hello"), "no Hello in {:?}", items.iter().map(|i| &i.label).take(20).collect::<Vec<_>>());
    // Warm completions: the editor's real case while typing.
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        wait(complete(&s)).unwrap();
        times.push(t.elapsed());
    }
    times.sort();
    let median = times[times.len() / 2];
    eprintln!("warm completion median: {median:?} ({times:?})");
    assert!(median < Duration::from_millis(150), "completion took {median:?}");

    // Rename Greeter.Hello from its use in App: the edit touches both projects.
    let edit: Option<WorkspaceEdit> = wait(s.client.request_raw(
        "textDocument/rename",
        json!({"textDocument": {"uri": uri.as_str()}, "position": Position { line: 3, character: col + 2 }, "newName": "Greet"}),
    ))
    .unwrap();
    let edit = edit.expect("rename edit");
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some(changes) = &edit.changes {
        files.extend(changes.keys().filter_map(uri_to_path));
    }
    if let Some(lsp_types::DocumentChanges::Edits(edits)) = &edit.document_changes {
        files.extend(edits.iter().filter_map(|e| uri_to_path(&e.text_document.uri)));
    }
    files.sort();
    files.dedup();
    assert_eq!(files, vec![root.join("App/Program.cs"), root.join("Lib/Greeter.cs")], "{edit:?}");

    // An error: pull diagnostics for a broken document.
    let broken = text.replace("greeter.Hello", "greeter.Nope");
    s.did_change_full(&program, &broken);
    let report: serde_json::Value = wait(s.client.request_raw("textDocument/diagnostic", json!({"textDocument": {"uri": uri.as_str()}}))).unwrap();
    let items = report.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    assert!(items.iter().any(|d| d.get("code").and_then(|c| c.as_str()) == Some("CS1061")), "{report}");
    s.stop();
}
