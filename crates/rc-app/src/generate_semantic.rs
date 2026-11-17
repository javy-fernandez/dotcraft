//! Generate candidates from other files, through Roslyn: the declarations of the type's base
//! types and of its fields' and properties' types (`textDocument/typeDefinition`), and the other
//! parts of a partial type (`textDocument/definition`). The requests go out once, when the
//! picker opens; the UI polls them each frame (cheap) and the files they point to are read and
//! parsed on a worker, never on the UI thread.

use crate::docs::Doc;
use crate::generate::{Dialog, Part, TargetMember, modifiers, named, text};
use rc_core::Runtime;
use rc_core::inbox::{Inbox, Waker};
use rc_lsp::lsp_types as lsp;
use rc_lsp::{LanguageSession, Pending, path_to_uri, uri_to_path};
use rc_syntax::tree_sitter::Node;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Files larger than this are not read (generated or vendored code).
const MAX_SOURCE: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
struct TypeReference {
    name: String,
    receiver: Option<String>,
    position: usize,
}

#[derive(Clone)]
enum Query {
    /// A base type, or the type of a field or property (the receiver) to delegate to.
    Type(TypeReference),
    /// The other declarations of the partial type at the caret.
    Partial { name: String, arity: usize },
}

struct PendingQuery {
    query: Query,
    pending: Pending<Value>,
}

enum Found {
    Targets(Vec<TargetMember>),
    Part(Part),
}

struct Resolved {
    doc: u64,
    version: u64,
    found: Found,
}

pub struct GenerateDiscovery {
    doc: u64,
    version: u64,
    /// The open document: its types were read from the editor's text, not from disk.
    path: PathBuf,
    pending: Vec<PendingQuery>,
    workers: usize,
    inbox: Inbox<Resolved>,
}

impl GenerateDiscovery {
    /// Sends the requests for the type at the caret; None without a ready Roslyn session or
    /// with nothing to resolve.
    pub fn start(doc: &Doc, dialog: &Dialog, session: Option<&LanguageSession>, waker: Waker) -> Option<Self> {
        let session = session.filter(|session| session.is_ready())?;
        let uri = path_to_uri(&doc.path);
        let rope = doc.editor.buffer.rope();
        let request = |method: &str, position: usize| {
            let position = crate::convert::lsp_position(rope, position);
            session.client.request_raw(method, json!({ "textDocument": { "uri": uri.as_str() }, "position": position }))
        };
        let (references, partial) = references(doc);
        let mut pending: Vec<PendingQuery> = references
            .into_iter()
            .map(|reference| PendingQuery { pending: request("textDocument/typeDefinition", reference.position), query: Query::Type(reference) })
            .collect();
        if let Some((name, arity, position)) = partial {
            pending.push(PendingQuery { pending: request("textDocument/definition", position), query: Query::Partial { name, arity } });
        }
        if pending.is_empty() {
            return None;
        }
        Some(Self { doc: dialog.doc_id, version: dialog.version, path: doc.path.clone(), pending, workers: 0, inbox: Inbox::new(waker) })
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || self.workers > 0
    }

    /// Takes the answers that arrived: starts a worker for each, and adds what the workers found.
    pub fn update(&mut self, dialog: &mut Dialog) {
        if dialog.doc_id != self.doc || dialog.version != self.version {
            return;
        }
        let sender = self.inbox.sender();
        let mut started = 0;
        self.pending.retain_mut(|query| {
            let Some(result) = query.pending.poll() else { return true };
            let mut paths = result.ok().map_or_else(Vec::new, |response| definition_paths(&response));
            paths.retain(|p| !same_file(p, &self.path));
            paths.dedup();
            if paths.is_empty() {
                return false;
            }
            started += 1;
            let query = query.query.clone();
            let known = dialog.known();
            let sender = sender.clone();
            let (doc, version) = (self.doc, self.version);
            Runtime::handle().spawn_blocking(move || {
                let sources = paths.iter().filter_map(|path| read_source(path));
                let found = match query {
                    Query::Type(reference) => Found::Targets(
                        sources
                            .map(|source| crate::generate_targets::collect_resolved_source(&source, &reference.name, reference.receiver.as_deref(), &known))
                            .find(|targets| !targets.is_empty())
                            .unwrap_or_default(),
                    ),
                    Query::Partial { name, arity } => {
                        let mut part = Part::default();
                        for source in sources {
                            let other = Part::of_partial(&source, &name, arity);
                            part.members.extend(other.members);
                            part.existing.extend(other.existing);
                            part.methods.extend(other.methods);
                        }
                        Found::Part(part)
                    }
                };
                sender.send(Resolved { doc, version, found });
            });
            false
        });
        self.workers += started;
        for result in self.inbox.drain() {
            self.workers = self.workers.saturating_sub(1);
            if result.doc != dialog.doc_id || result.version != dialog.version {
                continue;
            }
            match result.found {
                Found::Part(part) => dialog.merge(part),
                Found::Targets(targets) => {
                    // Checked again here: a partial part may have arrived after the worker began.
                    let known = dialog.known();
                    for candidate in targets {
                        let duplicate = dialog.targets.iter().any(|t| t.kind == candidate.kind && t.name == candidate.name && t.params == candidate.params);
                        if !duplicate && !known.has(&candidate.name, candidate.params.as_deref()) {
                            dialog.targets.push(candidate);
                        }
                    }
                }
            }
        }
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || dunce::canonicalize(a).ok().zip(dunce::canonicalize(b).ok()).is_some_and(|(a, b)| a == b)
}

fn read_source(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MAX_SOURCE {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The type references to resolve, and the name position of the type when it is partial.
#[allow(clippy::type_complexity)]
fn references(doc: &Doc) -> (Vec<TypeReference>, Option<(String, usize, usize)>) {
    let rope = doc.editor.buffer.rope();
    let fresh;
    let tree = match doc.editor.syntax.tree() {
        Some(tree) if doc.editor.syntax.is_current() => tree,
        _ => match rc_syntax::parse_now(rc_syntax::Lang::CSharp, rope, None) {
            Some(tree) => {
                fresh = tree;
                &fresh
            }
            None => return (Vec::new(), None),
        },
    };
    let source = rope.to_string();
    let byte = rope.char_to_byte(doc.editor.caret());
    let Some(mut current) = tree.root_node().descendant_for_byte_range(byte, byte) else { return (Vec::new(), None) };
    while !matches!(current.kind(), "class_declaration" | "struct_declaration" | "record_declaration") {
        let Some(parent) = current.parent() else { return (Vec::new(), None) };
        current = parent;
    }
    // The type's own type parameters resolve to nothing useful.
    let type_parameters: Vec<&str> = named(current, "type_parameter_list")
        .map(|list| {
            let mut cursor = list.walk();
            list.named_children(&mut cursor).filter_map(|p| p.child_by_field_name("name")).map(|n| text(n, &source)).collect()
        })
        .unwrap_or_default();
    let mut result = Vec::new();
    let mut add = |ty: Node<'_>, receiver: Option<String>| {
        let ty = if ty.kind() == "nullable_type" { ty.named_child(0).unwrap_or(ty) } else { ty };
        if matches!(ty.kind(), "identifier" | "generic_name" | "qualified_name" | "alias_qualified_name") && !type_parameters.contains(&text(ty, &source)) {
            result.push(reference(ty, &source, rope, receiver));
        }
    };
    if let Some(bases) = named(current, "base_list") {
        let mut cursor = bases.walk();
        for base in bases.named_children(&mut cursor) {
            let base = if base.kind() == "primary_constructor_base_type" { base.child_by_field_name("type").unwrap_or(base) } else { base };
            add(base, None);
        }
    }
    if let Some(body) = current.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.named_children(&mut cursor) {
            if modifiers(member, &source).iter().any(|m| m == "static" || m == "const") {
                continue;
            }
            match member.kind() {
                "field_declaration" => {
                    let Some(declaration) = named(member, "variable_declaration") else { continue };
                    let Some(ty) = declaration.child_by_field_name("type") else { continue };
                    let mut variables = declaration.walk();
                    for variable in declaration.named_children(&mut variables).filter(|node| node.kind() == "variable_declarator") {
                        if let Some(name) = variable.child_by_field_name("name") {
                            add(ty, Some(text(name, &source).to_string()));
                        }
                    }
                }
                "property_declaration" => {
                    if let (Some(ty), Some(name)) = (member.child_by_field_name("type"), member.child_by_field_name("name")) {
                        add(ty, Some(text(name, &source).to_string()));
                    }
                }
                _ => {}
            }
        }
    }
    let partial = modifiers(current, &source).iter().any(|m| m == "partial");
    let partial = current
        .child_by_field_name("name")
        .filter(|_| partial)
        .map(|name| (text(name, &source).to_string(), type_parameters.len(), rope.byte_to_char(name.start_byte() + 1)));
    (result, partial)
}

fn reference(node: Node<'_>, source: &str, rope: &rc_text::Rope, receiver: Option<String>) -> TypeReference {
    let name = text(node, source).to_string();
    let key = crate::generate_targets::type_key(&name);
    let offset = name.find(&key).unwrap_or(0);
    // Inside the name, not on its first character (some servers miss a position at the start).
    let inside = usize::from(key.len() > 1);
    TypeReference { name, receiver, position: rope.byte_to_char(node.start_byte() + offset + inside) }
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
            let uri = item.get("uri").or_else(|| item.get("targetUri"))?.as_str()?;
            uri.parse::<lsp::Uri>().ok().and_then(|uri| uri_to_path(&uri))
        })
        .collect()
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
    fn only_named_types_are_sent_to_roslyn() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Box.cs");
        let source = "public partial class Box<T> : Base<T>, Contracts.IThing\n{\n    private int _n;\n    private string? _s;\n    private T _t;\n    private Service? _service;\n    private static Cache _cache;\n    public List<int> Items { get; } = new();\n}\n";
        write(&path, source);
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&path);
        let doc = app.docs.active_doc_mut().unwrap();
        doc.editor.selection = rc_text::Selection::point(source.find("_n;").unwrap());
        let (references, partial) = references(app.docs.active_doc().unwrap());
        let names: Vec<(&str, Option<&str>)> = references.iter().map(|r| (r.name.as_str(), r.receiver.as_deref())).collect();
        assert_eq!(names, [("Base<T>", None), ("Contracts.IThing", None), ("Service", Some("_service")), ("List<int>", Some("Items"))]);
        // Positions are inside the type's name: `ase` of Base, `Thing` of IThing.
        assert_eq!(references[1].position, source.find("IThing").unwrap() + 1);
        let (name, arity, position) = partial.unwrap();
        assert_eq!((name.as_str(), arity, position), ("Box", 1, source.find("Box").unwrap() + 1));
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
        let initial = "using Contracts;\npublic partial class Derived : Base, IRepository<string>\n{\n    private Formatter _formatter = new();\n}\n";
        write(&app_source, initial);
        // The other part implements Name and adds a field.
        write(&root.join("App/Derived.Part.cs"), "public partial class Derived\n{\n    private int _count;\n    public string Name => _count.ToString();\n}\n");
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
            let outputs = app.insight.update(&mut app.docs, true, false);
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
            let outputs = app.insight.update(&mut app.docs, true, false);
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
            let outputs = app.insight.update(&mut app.docs, true, false);
            app.handle_insight(outputs);
            discovery.update(&mut dialog);
            if !discovery.busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!discovery.busy(), "semantic discovery timed out");
        assert!(dialog.members.iter().any(|m| m.name == "_count"), "{:?}", dialog.members);
        assert!(!dialog.targets.iter().any(|t| t.kind == Kind::Implement && t.name == "Name"), "{:?}", dialog.targets);
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
        // The closing brace starts its line: every edit is an insertion at the same point.
        let mut generated = String::new();
        let mut at = 0;
        for kind in [Kind::Implement, Kind::Override, Kind::Delegate] {
            dialog.kind = kind;
            let (range, text) = dialog.edit().unwrap();
            at = range.start;
            generated.push_str(&text);
        }
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
