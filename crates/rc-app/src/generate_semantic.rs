//! Resolve Generate picker member sources through Roslyn without blocking the editor frame.

use crate::docs::Doc;
use crate::generate::{Dialog, TargetMember};
use rc_core::Runtime;
use rc_core::inbox::{Inbox, Waker};
use rc_lsp::lsp_types as lsp;
use rc_lsp::{LanguageSession, Pending, path_to_uri, uri_to_path};
use rc_syntax::tree_sitter::Node;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone)]
struct TypeReference {
    name: String,
    receiver: Option<String>,
    position: usize,
}

struct PendingDefinition {
    reference: TypeReference,
    pending: Pending<Value>,
}

struct Resolved {
    doc: u64,
    version: u64,
    targets: Vec<TargetMember>,
}

pub struct GenerateDiscovery {
    doc: u64,
    version: u64,
    pending: Vec<PendingDefinition>,
    workers: usize,
    inbox: Inbox<Resolved>,
}

impl GenerateDiscovery {
    pub fn start(
        doc: &Doc,
        dialog: &Dialog,
        session: Option<&LanguageSession>,
        waker: Waker,
    ) -> Option<Self> {
        let session = session.filter(|session| session.is_ready())?;
        let uri = path_to_uri(&doc.path);
        let rope = doc.editor.buffer.rope();
        let pending = references(doc)
            .into_iter()
            .map(|reference| {
                let position = crate::convert::lsp_position(rope, reference.position);
                let pending = session.client.request_raw(
                    "textDocument/typeDefinition",
                    json!({
                        "textDocument": { "uri": uri.as_str() }, "position": position,
                    }),
                );
                PendingDefinition { reference, pending }
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return None;
        }
        Some(Self {
            doc: dialog.doc_id,
            version: dialog.version,
            pending,
            workers: 0,
            inbox: Inbox::new(waker),
        })
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || self.workers > 0
    }

    pub fn update(&mut self, dialog: &mut Dialog) {
        if dialog.doc_id != self.doc || dialog.version != self.version {
            return;
        }
        let sender = self.inbox.sender();
        self.pending.retain_mut(|query| {
            let Some(result) = query.pending.poll() else {
                return true;
            };
            let paths = result
                .ok()
                .map_or_else(Vec::new, |response| definition_paths(&response));
            if !paths.is_empty() {
                self.workers += 1;
                let reference = query.reference.clone();
                let existing = dialog.existing.clone();
                let sender = sender.clone();
                let doc = self.doc;
                let version = self.version;
                Runtime::handle().spawn_blocking(move || {
                    let mut targets = Vec::new();
                    for path in paths {
                        let Ok(metadata) = std::fs::metadata(&path) else {
                            continue;
                        };
                        if metadata.len() > 2 * 1024 * 1024 {
                            continue;
                        }
                        let Ok(source) = std::fs::read_to_string(&path) else {
                            continue;
                        };
                        targets = crate::generate_targets::collect_resolved_source(
                            &source,
                            &reference.name,
                            reference.receiver.as_deref(),
                            &existing,
                        );
                        if !targets.is_empty() {
                            break;
                        }
                    }
                    sender.send(Resolved {
                        doc,
                        version,
                        targets,
                    });
                });
            }
            false
        });
        for result in self.inbox.drain() {
            self.workers = self.workers.saturating_sub(1);
            if result.doc != dialog.doc_id || result.version != dialog.version {
                continue;
            }
            for candidate in result.targets {
                if !dialog
                    .targets
                    .iter()
                    .any(|target| target.kind == candidate.kind && target.label == candidate.label)
                {
                    dialog.targets.push(candidate);
                }
            }
        }
    }
}

fn references(doc: &Doc) -> Vec<TypeReference> {
    let Some(tree) = doc.editor.syntax.tree() else {
        return Vec::new();
    };
    let rope = doc.editor.buffer.rope();
    let source = rope.to_string();
    let byte = rope.char_to_byte(doc.editor.caret());
    let Some(mut current) = tree.root_node().descendant_for_byte_range(byte, byte) else {
        return Vec::new();
    };
    while !matches!(current.kind(), "class_declaration" | "struct_declaration") {
        let Some(parent) = current.parent() else {
            return Vec::new();
        };
        current = parent;
    }
    let mut result = Vec::new();
    if let Some(bases) = named(current, "base_list") {
        let mut cursor = bases.walk();
        for base in bases.named_children(&mut cursor) {
            if is_type(base) {
                result.push(reference(base, &source, rope, None));
            }
        }
    }
    if let Some(body) = current.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.named_children(&mut cursor) {
            match member.kind() {
                "field_declaration" => {
                    let Some(declaration) = named(member, "variable_declaration") else {
                        continue;
                    };
                    let Some(ty) = declaration.named_child(0) else {
                        continue;
                    };
                    let mut variables = declaration.walk();
                    for variable in declaration
                        .named_children(&mut variables)
                        .filter(|node| node.kind() == "variable_declarator")
                    {
                        if let Some(name) = variable.child_by_field_name("name") {
                            result.push(reference(
                                ty,
                                &source,
                                rope,
                                Some(text(name, &source).to_string()),
                            ));
                        }
                    }
                }
                "property_declaration" => {
                    if let (Some(ty), Some(name)) = (
                        member.child_by_field_name("type"),
                        member.child_by_field_name("name"),
                    ) {
                        result.push(reference(
                            ty,
                            &source,
                            rope,
                            Some(text(name, &source).to_string()),
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    result
}

fn reference(
    node: Node<'_>,
    source: &str,
    rope: &rc_text::Rope,
    receiver: Option<String>,
) -> TypeReference {
    let name = text(node, source).to_string();
    let key = name
        .split('<')
        .next()
        .unwrap_or_default()
        .rsplit('.')
        .next()
        .unwrap_or_default();
    let offset = name.find(key).unwrap_or(0);
    let inside = usize::from(key.len() > 1);
    TypeReference {
        name,
        receiver,
        position: rope.byte_to_char(node.start_byte() + offset + inside),
    }
}

fn definition_paths(response: &Value) -> Vec<PathBuf> {
    let values = match response {
        Value::Array(items) => items.as_slice(),
        Value::Null => return Vec::new(),
        item => std::slice::from_ref(item),
    };
    values
        .iter()
        .filter_map(|item| {
            let uri = item
                .get("uri")
                .or_else(|| item.get("targetUri"))?
                .as_str()?;
            uri.parse::<lsp::Uri>()
                .ok()
                .and_then(|uri| uri_to_path(&uri))
        })
        .collect()
}

fn is_type(node: Node<'_>) -> bool {
    matches!(node.kind(), "type" | "identifier" | "generic_name") || node.kind().ends_with("_type")
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

fn named<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::DotCraftApp;
    use crate::generate::Kind;
    use crate::insight::InsightStatus;
    use rc_core::Settings;
    use std::path::Path;
    use std::time::{Duration, Instant};

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    #[ignore = "Requires DOTCRAFT_TEST_ROSLYN and DOTCRAFT_TEST_DOTNET for a live Roslyn session"]
    fn discovers_referenced_project_members_and_builds_generated_code() {
        let _lock = crate::screens::HOME_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let roslyn =
            PathBuf::from(std::env::var_os("DOTCRAFT_TEST_ROSLYN").expect("Roslyn server path"));
        let dotnet =
            PathBuf::from(std::env::var_os("DOTCRAFT_TEST_DOTNET").expect("dotnet SDK path"));
        let home = tempfile::tempdir().unwrap();
        // SAFETY: HOME_LOCK protects the process-wide test home override.
        unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let contracts = root.join("Contracts/Contracts.csproj");
        let app_project = root.join("App/App.csproj");
        let app_source = root.join("App/Derived.cs");
        let solution = root.join("Demo.slnx");
        write(
            &contracts,
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>\n",
        );
        write(
            &root.join("Contracts/Types.cs"),
            "namespace Contracts;\npublic interface IRepository<T> { T Find(T value); string Name { get; } }\npublic class Base { public virtual int Score() => 1; }\npublic class Formatter { public string Format(int value) => value.ToString(); }\n",
        );
        write(
            &app_project,
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup><ItemGroup><ProjectReference Include=\"../Contracts/Contracts.csproj\" /></ItemGroup></Project>\n",
        );
        let initial = "using Contracts;\npublic class Derived : Base, IRepository<string>\n{\n    private Formatter _formatter = new();\n}\n";
        write(&app_source, initial);
        write(
            &solution,
            "<Solution><Project Path=\"Contracts/Contracts.csproj\" /><Project Path=\"App/App.csproj\" /></Solution>\n",
        );
        let restore = std::process::Command::new(&dotnet)
            .arg("restore")
            .arg("--ignore-failed-sources")
            .arg(&app_project)
            .output()
            .unwrap();
        assert!(
            restore.status.success(),
            "status: {}\n{}\n{}",
            restore.status,
            String::from_utf8_lossy(&restore.stdout),
            String::from_utf8_lossy(&restore.stderr)
        );
        let mut settings = Settings::default();
        settings.roslyn_path = Some(roslyn);
        settings.dotnet_path = Some(dotnet.clone());
        let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
        app.open_workspace(&solution);
        assert_eq!(app.solution_projects().len(), 2);
        app.open_file(&app_source);
        let caret = initial.find("private Formatter").unwrap();
        app.docs.active_doc_mut().unwrap().editor.selection = rc_text::Selection::point(caret);
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(120)
            && app.insight.status != InsightStatus::Ready
        {
            let outputs = app.insight.update(&mut app.docs, true);
            app.handle_insight(outputs);
            app.docs.poll_parses();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(app.insight.status, InsightStatus::Ready);
        assert!(
            app.insight
                .session
                .as_ref()
                .unwrap()
                .docs
                .is_open(&app_source)
        );
        for _ in 0..50 {
            let outputs = app.insight.update(&mut app.docs, true);
            app.handle_insight(outputs);
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut dialog = Dialog::from_doc(app.docs.active_doc().unwrap()).unwrap();
        let mut discovery = GenerateDiscovery::start(
            app.docs.active_doc().unwrap(),
            &dialog,
            app.insight.session.as_ref(),
            Waker::noop(),
        )
        .unwrap();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            let outputs = app.insight.update(&mut app.docs, true);
            app.handle_insight(outputs);
            discovery.update(&mut dialog);
            if !discovery.busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!discovery.busy(), "semantic discovery timed out");
        assert!(
            dialog
                .targets
                .iter()
                .any(|target| target.kind == Kind::Implement
                    && target.code.contains("string Find(string value)")),
            "{:?}",
            dialog.targets
        );
        assert!(
            dialog
                .targets
                .iter()
                .any(|target| target.kind == Kind::Override
                    && target.code.contains("override int Score()")),
            "{:?}",
            dialog.targets
        );
        assert!(
            dialog
                .targets
                .iter()
                .any(|target| target.kind == Kind::Delegate
                    && target.code.contains("_formatter.Format(value)")),
            "{:?}",
            dialog.targets
        );
        let mut generated = String::new();
        for kind in [Kind::Implement, Kind::Override, Kind::Delegate] {
            dialog.kind = kind;
            generated.push_str(&dialog.render().unwrap());
        }
        let at = initial.rfind('}').unwrap();
        let complete = format!("{}{}{}", &initial[..at], generated, &initial[at..]);
        write(&app_source, &complete);
        let build = std::process::Command::new(&dotnet)
            .arg("build")
            .arg("--no-restore")
            .arg(&app_project)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        app.close_workspace();
    }
}
