//! The Roslyn language server (`Microsoft.CodeAnalysis.LanguageServer`, MIT): finding or
//! downloading it, starting it over stdio, the `initialize` handshake and opening the
//! solution with Roslyn's `solution/open` / `project/open` notifications.

use crate::rpc::{Client, Incoming, Pending};
use crate::sync::path_to_uri;
use rc_core::inbox::Outbox;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Version downloaded when no server is installed (the `roslyn-language-server` tool package
/// of the `vs-impl` feed, the one the VS Code C# extension uses).
pub const PINNED_VERSION: &str = "5.13.0-1.26523.5";
const FEED: &str = "https://pkgs.dev.azure.com/azure-public/3ccf6661-f8ce-4e8a-bb2e-eff943ddd3c7/_packaging/3c18fd2c-cc7c-4cef-8ed7-20227ab3275b/nuget/v3/flat2";

/// How to start the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerCommand {
    pub program: PathBuf,
    /// Arguments before ours (`<dll>` when it runs through `dotnet`).
    pub prefix: Vec<String>,
    /// Where it was found, for the status bar ("downloaded", "VS Code C# extension"…).
    pub origin: Origin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Settings,
    Downloaded,
    DotnetTool,
    VsCodeExtension,
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// The runtime identifier of this machine for the tool packages.
pub fn rid() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "osx-arm64",
        ("macos", _) => "osx-x64",
        ("windows", "aarch64") => "win-arm64",
        ("windows", _) => "win-x64",
        ("linux", "aarch64") => "linux-arm64",
        _ => "linux-x64",
    }
}

/// Folder of the downloaded server.
pub fn download_dir() -> PathBuf {
    rc_core::paths::tools_dir().join("roslyn").join(PINNED_VERSION)
}

/// Finds a server: the configured path, our download, the `roslyn-language-server` dotnet
/// tool, or the copy inside an installed VS Code C# extension (newest first).
pub fn locate(configured: Option<&Path>, dotnet: &Path) -> Option<ServerCommand> {
    if let Some(p) = configured.filter(|p| p.exists()) {
        return Some(if p.extension().is_some_and(|e| e == "dll") {
            ServerCommand { program: dotnet.to_path_buf(), prefix: vec![p.display().to_string()], origin: Origin::Settings }
        } else {
            ServerCommand { program: p.to_path_buf(), prefix: Vec::new(), origin: Origin::Settings }
        });
    }
    let ours = download_dir().join(exe("Microsoft.CodeAnalysis.LanguageServer"));
    if ours.exists() {
        return Some(ServerCommand { program: ours, prefix: Vec::new(), origin: Origin::Downloaded });
    }
    let home = dirs_home();
    if let Some(home) = &home {
        let tool = home.join(".dotnet/tools").join(exe("roslyn-language-server"));
        if tool.exists() {
            return Some(ServerCommand { program: tool, prefix: Vec::new(), origin: Origin::DotnetTool });
        }
        for ext_dir in [".vscode/extensions", ".vscode-insiders/extensions", ".cursor/extensions", ".vscode-oss/extensions"] {
            let Ok(entries) = std::fs::read_dir(home.join(ext_dir)) else { continue };
            let mut found: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("ms-dotnettools.csharp-")))
                .map(|p| p.join(".roslyn").join(exe("Microsoft.CodeAnalysis.LanguageServer")))
                .filter(|p| p.exists())
                .collect();
            found.sort();
            if let Some(p) = found.pop() {
                return Some(ServerCommand { program: p, prefix: Vec::new(), origin: Origin::VsCodeExtension });
            }
        }
    }
    None
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// Download progress for the status bar.
#[derive(Clone, Debug, PartialEq)]
pub enum DownloadEvent {
    Progress { done: u64, total: Option<u64> },
    Done(PathBuf),
    Failed(String),
}

/// Downloads and unpacks the pinned server into [`download_dir`] (async; run it on the
/// shared runtime). The package is ~60 MB.
pub async fn download(events: Outbox<DownloadEvent>) {
    match download_inner(&events).await {
        Ok(p) => {
            events.send(DownloadEvent::Done(p));
        }
        Err(e) => {
            events.send(DownloadEvent::Failed(format!("{e:#}")));
        }
    }
}

async fn download_inner(events: &Outbox<DownloadEvent>) -> anyhow::Result<PathBuf> {
    let id = format!("roslyn-language-server.{}", rid());
    let url = format!("{FEED}/{id}/{PINNED_VERSION}/{id}.{PINNED_VERSION}.nupkg");
    let client = reqwest::Client::builder().build()?;
    let mut resp = client.get(&url).send().await?.error_for_status()?;
    let total = resp.content_length();
    let mut bytes: Vec<u8> = Vec::with_capacity(total.unwrap_or(64 << 20) as usize);
    let mut last = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        bytes.extend_from_slice(&chunk);
        let done = bytes.len() as u64;
        if done - last > 1 << 20 {
            last = done;
            events.send(DownloadEvent::Progress { done, total });
        }
    }
    let dest = download_dir();
    let tmp = dest.with_extension("partial");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let prefix = format!("tools/net10.0/{}/", rid());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let Some(name) = f.enclosed_name() else { continue };
        let name = name.to_string_lossy().replace('\\', "/");
        let Some(rel) = name.strip_prefix(&prefix) else { continue };
        if rel.is_empty() || f.is_dir() {
            continue;
        }
        let out = tmp.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut f, &mut w)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = f.unix_mode() {
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode | 0o644));
            }
            // NuGet packages carry no Unix modes: the apphosts must become executable.
            let is_host = rel == exe("Microsoft.CodeAnalysis.LanguageServer") || rel == exe("roslyn-language-server") || rel.ends_with(".dylib") || rel.ends_with(".exe");
            if is_host {
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&tmp, &dest)?;
    Ok(dest.join(exe("Microsoft.CodeAnalysis.LanguageServer")))
}

/// Command-line arguments for a stdio session.
pub fn args(cmd: &ServerCommand, log_dir: &Path) -> Vec<String> {
    let mut a = cmd.prefix.clone();
    a.extend([
        "--stdio".to_string(),
        "--logLevel".into(),
        "Information".into(),
        "--telemetryLevel".into(),
        "off".into(),
        "--extensionLogDirectory".into(),
        log_dir.display().to_string(),
        "--clientProcessId".into(),
        std::process::id().to_string(),
    ]);
    a
}

/// Starts the server; must run inside the tokio runtime.
pub fn start(cmd: &ServerCommand, workspace: &Path, dotnet_root: Option<&Path>, incoming: Outbox<Incoming>) -> std::io::Result<Client> {
    let log_dir = rc_core::paths::logs_dir().join("roslyn");
    let _ = std::fs::create_dir_all(&log_dir);
    let mut env = vec![("DOTNET_CLI_TELEMETRY_OPTOUT".to_string(), "1".to_string()), ("DOTNET_NOLOGO".into(), "1".into())];
    if let Some(root) = dotnet_root {
        env.push(("DOTNET_ROOT".into(), root.display().to_string()));
    }
    Client::spawn(&cmd.program, &args(cmd, &log_dir), workspace, &env, incoming)
}

/// The client capabilities DotCraft implements.
pub fn client_capabilities() -> Value {
    json!({
        "workspace": {
            "applyEdit": true,
            "workspaceEdit": { "documentChanges": true, "resourceOperations": ["create", "rename", "delete"] },
            "didChangeConfiguration": { "dynamicRegistration": true },
            "didChangeWatchedFiles": { "dynamicRegistration": true },
            "configuration": true,
            "workspaceFolders": true,
            "symbol": { "symbolKind": { "valueSet": (1..=26).collect::<Vec<i32>>() } },
            "semanticTokens": { "refreshSupport": true },
            "inlayHint": { "refreshSupport": true },
            "diagnostics": { "refreshSupport": true },
            "codeLens": { "refreshSupport": false }
        },
        "textDocument": {
            "synchronization": { "didSave": true, "willSave": false },
            "publishDiagnostics": { "relatedInformation": true, "tagSupport": { "valueSet": [1, 2] }, "versionSupport": true },
            "diagnostic": { "dynamicRegistration": true, "relatedDocumentSupport": false },
            "completion": {
                "completionItem": {
                    "snippetSupport": true,
                    "documentationFormat": ["markdown", "plaintext"],
                    "deprecatedSupport": true,
                    "tagSupport": { "valueSet": [1] },
                    "insertReplaceSupport": false,
                    "resolveSupport": { "properties": ["documentation", "detail", "additionalTextEdits", "textEdit"] },
                    "labelDetailsSupport": true
                },
                "completionItemKind": { "valueSet": (1..=25).collect::<Vec<i32>>() },
                "contextSupport": true,
                "completionList": { "itemDefaults": ["commitCharacters", "editRange", "insertTextFormat", "insertTextMode", "data"] }
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "signatureHelp": {
                "signatureInformation": { "documentationFormat": ["markdown", "plaintext"], "parameterInformation": { "labelOffsetSupport": true }, "activeParameterSupport": true },
                "contextSupport": true
            },
            "definition": { "linkSupport": false },
            "typeDefinition": { "linkSupport": false },
            "implementation": { "linkSupport": false },
            "references": {},
            "documentHighlight": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true, "symbolKind": { "valueSet": (1..=26).collect::<Vec<i32>>() } },
            "codeAction": {
                "codeActionLiteralSupport": { "codeActionKind": { "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports"] } },
                "resolveSupport": { "properties": ["edit"] },
                "dataSupport": true,
                "isPreferredSupport": true
            },
            "formatting": {},
            "rangeFormatting": {},
            "onTypeFormatting": {},
            "rename": { "prepareSupport": true },
            "foldingRange": { "lineFoldingOnly": true },
            "semanticTokens": {
                "requests": { "range": true, "full": { "delta": false } },
                "tokenTypes": crate::semantic::CLIENT_TOKEN_TYPES,
                "tokenModifiers": crate::semantic::CLIENT_TOKEN_MODIFIERS,
                "formats": ["relative"],
                "overlappingTokenSupport": false,
                "multilineTokenSupport": false
            },
            "inlayHint": { "resolveSupport": { "properties": [] } }
        },
        "window": { "workDoneProgress": true, "showMessage": {} },
        "general": { "positionEncodings": ["utf-16"] }
    })
}

/// Sends `initialize` for `root`.
pub fn initialize(client: &Client, root: &Path) -> Pending<lsp_types::InitializeResult> {
    let uri = path_to_uri(root);
    let name = root.file_name().map_or_else(|| "workspace".to_string(), |n| n.to_string_lossy().into_owned());
    client.request_raw(
        "initialize",
        json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "DotCraft", "version": env!("CARGO_PKG_VERSION") },
            "locale": "en",
            "rootUri": uri.as_str(),
            "workspaceFolders": [{ "uri": uri.as_str(), "name": name }],
            "capabilities": client_capabilities(),
            // Projects are opened explicitly with `solution/open`.
            "initializationOptions": { "autoLoadProjects": 0 }
        }),
    )
}

/// After `initialized`: loads a solution, or a set of projects.
pub fn open_solution(client: &Client, solution: Option<&Path>, projects: &[PathBuf]) {
    if let Some(sln) = solution {
        client.notify_raw("solution/open", json!({ "solution": path_to_uri(sln).as_str() }));
    } else if !projects.is_empty() {
        let uris: Vec<String> = projects.iter().map(|p| path_to_uri(p).as_str().to_string()).collect();
        client.notify_raw("project/open", json!({ "projects": uris }));
    }
}

/// Answer for a server→client request the IDE handles without asking the user.
pub fn default_reply(method: &str, params: &Value) -> Option<Value> {
    match method {
        // One value per requested setting (`null`: Roslyn's default).
        "workspace/configuration" => {
            let items = params.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
            Some(Value::Array(items.iter().map(|i| setting(i.get("section").and_then(Value::as_str).unwrap_or(""))).collect()))
        }
        "client/registerCapability" | "client/unregisterCapability" | "window/workDoneProgress/create" => Some(Value::Null),
        "workspace/semanticTokens/refresh" | "workspace/inlayHint/refresh" | "workspace/diagnostic/refresh" | "workspace/codeLens/refresh" => Some(Value::Null),
        "window/showMessageRequest" => Some(Value::Null),
        // Roslyn asks before running `dotnet restore`; the IDE restores through its build.
        "workspace/_roslyn_projectNeedsRestore" => Some(Value::Null),
        _ => None,
    }
}

/// Roslyn options that differ from its defaults, to behave like Rider: parameter-name hints
/// for literals, completion of types from namespaces not imported yet, analysis of open files.
fn setting(section: &str) -> Value {
    let key = section.rsplit(['|', '.']).next().unwrap_or(section);
    match key {
        "dotnet_enable_inlay_hints_for_parameters" | "dotnet_enable_inlay_hints_for_literal_parameters" | "dotnet_enable_inlay_hints_for_object_creation_parameters" => json!(true),
        "dotnet_enable_inlay_hints_for_indexer_parameters" | "dotnet_enable_inlay_hints_for_other_parameters" => json!(false),
        "dotnet_suppress_inlay_hints_for_parameters_that_differ_only_by_suffix" | "dotnet_suppress_inlay_hints_for_parameters_that_match_method_intent" | "dotnet_suppress_inlay_hints_for_parameters_that_match_argument_name" => json!(true),
        "csharp_enable_inlay_hints_for_types" | "csharp_enable_inlay_hints_for_implicit_variable_types" | "csharp_enable_inlay_hints_for_lambda_parameter_types" | "csharp_enable_inlay_hints_for_implicit_object_creation" => json!(false),
        "dotnet_show_completion_items_from_unimported_namespaces" | "dotnet_provide_regex_completions" | "dotnet_show_name_completion_suggestions" => json!(true),
        "dotnet_enable_references_code_lens" | "dotnet_enable_tests_code_lens" => json!(false),
        "dotnet_analyzer_diagnostics_scope" | "dotnet_compiler_diagnostics_scope" => json!("openFiles"),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_requests_get_one_null_per_item() {
        let v = default_reply("workspace/configuration", &json!({"items": [{}, {"section": "csharp|inlay_hints.dotnet_enable_inlay_hints_for_parameters"}, {"section": "x"}]})).unwrap();
        assert_eq!(v, json!([null, true, null]));
        assert!(default_reply("workspace/applyEdit", &json!({})).is_none());
    }

    #[test]
    fn stdio_arguments() {
        let cmd = ServerCommand { program: "/x/dotnet".into(), prefix: vec!["/x/LS.dll".into()], origin: Origin::Settings };
        let a = args(&cmd, Path::new("/logs"));
        assert_eq!(&a[..2], ["/x/LS.dll", "--stdio"]);
        assert!(a.windows(2).any(|w| w == ["--telemetryLevel", "off"]));
    }
}
