//! Browse managed assemblies through the optional ILSpy command-line tool.

use rc_core::{Inbox, Waker};
use rc_kit::i18n::{tr, trf};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssemblyType {
    pub kind: String,
    pub full_name: String,
    pub namespace: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub line: usize,
}

enum ResultEvent {
    Package(u64, PathBuf),
    Frameworks(u64, Vec<PathBuf>),
    Types(u64, Vec<AssemblyType>),
    Source(u64, String, bool, String, Vec<Member>),
    Error(u64, String),
}

pub struct AssemblyExplorer {
    pub visible: bool,
    pub path: Option<PathBuf>,
    pub types: Vec<AssemblyType>,
    pub filter: String,
    pub framework_assemblies: Vec<PathBuf>,
    pub framework_filter: String,
    pub selected: Option<String>,
    pub members: Vec<Member>,
    pub selected_member: Option<usize>,
    pub source: String,
    pub show_il: bool,
    pub busy: bool,
    pub error: Option<String>,
    inbox: Inbox<ResultEvent>,
    generation: Arc<AtomicU64>,
}

impl AssemblyExplorer {
    pub fn new(waker: Waker) -> Self {
        Self {
            visible: false,
            path: None,
            types: Vec::new(),
            filter: String::new(),
            framework_assemblies: Vec::new(),
            framework_filter: String::new(),
            selected: None,
            members: Vec::new(),
            selected_member: None,
            source: String::new(),
            show_il: false,
            busy: false,
            error: None,
            inbox: Inbox::new(waker),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn open(&mut self, path: PathBuf) {
        self.visible = true;
        self.path = Some(path.clone());
        self.types.clear();
        self.selected = None;
        self.members.clear();
        self.selected_member = None;
        self.source.clear();
        self.error = None;
        self.busy = true;
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = self.generation.clone();
        let out = self.inbox.sender();
        std::thread::spawn(move || {
            let Some(tool) = find_ilspycmd() else {
                out.send(ResultEvent::Error(generation, tr("Install ilspycmd with `dotnet tool install --global ilspycmd` to browse assemblies").into()));
                return;
            };
            let mut types = Vec::new();
            for kind in ["c", "i", "s", "d", "e"] {
                let args = ["--list", kind];
                match run(&tool, &path, &args, &cancel, generation) {
                    Ok(output) => types.extend(parse_types(&output)),
                    Err(error) => {
                        if cancel.load(Ordering::Relaxed) == generation {
                            out.send(ResultEvent::Error(generation, error));
                        }
                        return;
                    }
                }
            }
            types.sort_by(|a, b| a.full_name.cmp(&b.full_name));
            types.dedup_by(|a, b| a.full_name == b.full_name);
            if cancel.load(Ordering::Relaxed) == generation {
                out.send(ResultEvent::Types(generation, types));
            }
        });
    }

    pub fn open_package(&mut self, project: PathBuf, name: String, version: String) {
        self.visible = true;
        self.path = None;
        self.types.clear();
        self.selected = None;
        self.members.clear();
        self.source.clear();
        self.error = None;
        self.busy = true;
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let out = self.inbox.sender();
        std::thread::spawn(
            move || match find_package_assembly(&project, &name, &version) {
                Some(path) => {
                    out.send(ResultEvent::Package(generation, path));
                }
                None => {
                    out.send(ResultEvent::Error(
                        generation,
                        trf(
                            "No installed assembly found for {name} {version}",
                            &[("name", &name), ("version", &version)],
                        ),
                    ));
                }
            },
        );
    }

    pub fn list_framework_assemblies(&mut self) {
        self.visible = true;
        self.error = None;
        self.busy = true;
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let out = self.inbox.sender();
        std::thread::spawn(move || {
            let Some(dotnet) = rc_dotnet::sdk::Dotnet::locate(None) else {
                out.send(ResultEvent::Error(
                    generation,
                    tr(".NET SDK was not found").into(),
                ));
                return;
            };
            let assemblies = dotnet
                .root()
                .map(framework_assemblies_in)
                .unwrap_or_default();
            out.send(ResultEvent::Frameworks(generation, assemblies));
        });
    }

    pub fn select(&mut self, full_name: String, show_il: bool) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let same_type = self.selected.as_deref() == Some(&full_name);
        self.selected = Some(full_name.clone());
        self.show_il = show_il;
        self.source.clear();
        if !same_type {
            self.members.clear();
            self.selected_member = None;
        }
        self.error = None;
        self.busy = true;
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = self.generation.clone();
        let out = self.inbox.sender();
        std::thread::spawn(move || {
            let Some(tool) = find_ilspycmd() else {
                out.send(ResultEvent::Error(
                    generation,
                    tr("ilspycmd is not installed").into(),
                ));
                return;
            };
            let args = if show_il {
                vec!["--ilcode", "--type", full_name.as_str()]
            } else {
                vec!["--type", full_name.as_str()]
            };
            match run(&tool, &path, &args, &cancel, generation) {
                Ok(source) => {
                    let members = if show_il {
                        Vec::new()
                    } else {
                        parse_members(&source)
                    };
                    if cancel.load(Ordering::Relaxed) == generation {
                        out.send(ResultEvent::Source(
                            generation, full_name, show_il, source, members,
                        ));
                    }
                }
                Err(error) => {
                    if cancel.load(Ordering::Relaxed) == generation {
                        out.send(ResultEvent::Error(generation, error));
                    }
                }
            }
        });
    }

    pub fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.busy = false;
    }

    pub fn poll(&mut self) {
        let active = self.generation.load(Ordering::Relaxed);
        let mut package = None;
        for event in self.inbox.drain() {
            match event {
                ResultEvent::Package(generation, path) if generation == active => {
                    package = Some(path)
                }
                ResultEvent::Frameworks(generation, paths) if generation == active => {
                    self.framework_assemblies = paths;
                    self.busy = false;
                }
                ResultEvent::Types(generation, types) if generation == active => {
                    self.types = types;
                    self.busy = false;
                }
                ResultEvent::Source(generation, name, il, source, members)
                    if generation == active
                        && self.selected.as_deref() == Some(&name)
                        && self.show_il == il =>
                {
                    self.source = source;
                    if !il {
                        self.members = members;
                    }
                    self.busy = false;
                }
                ResultEvent::Error(generation, error) if generation == active => {
                    self.error = Some(error);
                    self.busy = false;
                }
                _ => {}
            }
        }
        if let Some(path) = package {
            self.open(path);
        }
    }
}

fn framework_assemblies_in(root: &Path) -> Vec<PathBuf> {
    let mut assemblies = Vec::new();
    let Ok(packs) = std::fs::read_dir(root.join("packs")) else {
        return assemblies;
    };
    for pack in packs
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".Ref"))
    {
        let Ok(versions) = std::fs::read_dir(pack.path()) else {
            continue;
        };
        for version in versions.flatten() {
            let Ok(frameworks) = std::fs::read_dir(version.path().join("ref")) else {
                continue;
            };
            for framework in frameworks.flatten() {
                let Ok(files) = std::fs::read_dir(framework.path()) else {
                    continue;
                };
                assemblies.extend(files.flatten().map(|file| file.path()).filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
                }));
            }
        }
    }
    assemblies.sort();
    assemblies
}

fn find_package_assembly(project: &Path, name: &str, version: &str) -> Option<PathBuf> {
    let root = std::env::var_os("NUGET_PACKAGES")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".nuget").join("packages")))?;
    find_package_assembly_from_assets(project, &root, name, version)
        .or_else(|| find_package_assembly_in(&root, name, version))
}

fn find_package_assembly_from_assets(
    project: &Path,
    cache: &Path,
    name: &str,
    version: &str,
) -> Option<PathBuf> {
    let assets = project.parent()?.join("obj/project.assets.json");
    let document: serde_json::Value = serde_json::from_slice(&std::fs::read(assets).ok()?).ok()?;
    let targets = document.get("targets")?.as_object()?;
    let package_id = format!("{name}/{version}");
    let package_root = cache
        .join(name.to_ascii_lowercase())
        .join(version.to_ascii_lowercase());
    let frameworks = rc_dotnet::ProjectInfo::load(project)
        .ok()
        .map(|info| info.target_frameworks)
        .unwrap_or_default();
    let mut target_names: Vec<_> = targets.keys().collect();
    target_names.sort_by_key(|target| {
        if frameworks
            .iter()
            .any(|framework| *target == framework || target.starts_with(&format!("{framework}/")))
        {
            0
        } else {
            1
        }
    });
    for target_name in target_names {
        let Some(packages) = targets[target_name].as_object() else {
            continue;
        };
        let Some((_, package)) = packages
            .iter()
            .find(|(id, _)| id.eq_ignore_ascii_case(&package_id))
        else {
            continue;
        };
        for group in ["runtime", "compile"] {
            let Some(files) = package.get(group).and_then(serde_json::Value::as_object) else {
                continue;
            };
            for relative in files.keys() {
                let candidate = Path::new(relative);
                if candidate
                    .extension()
                    .is_none_or(|extension| !extension.eq_ignore_ascii_case("dll"))
                    || !candidate
                        .components()
                        .all(|part| matches!(part, std::path::Component::Normal(_)))
                {
                    continue;
                }
                let path = package_root.join(candidate);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn find_package_assembly_in(root: &Path, name: &str, version: &str) -> Option<PathBuf> {
    let package = root
        .join(name.to_ascii_lowercase())
        .join(version.to_ascii_lowercase());
    for folder in ["lib", "ref"] {
        let base = package.join(folder);
        let Ok(frameworks) = std::fs::read_dir(base) else {
            continue;
        };
        let mut paths = Vec::new();
        for framework in frameworks.flatten() {
            let Ok(files) = std::fs::read_dir(framework.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("dll")
                }) {
                    paths.push(path);
                }
            }
        }
        paths.sort();
        if let Some(path) = paths.iter().find(|path| {
            path.file_stem()
                .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(name))
        }) {
            return Some(path.clone());
        }
        if let Some(path) = paths.into_iter().next() {
            return Some(path);
        }
    }
    None
}

fn find_ilspycmd() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOTCRAFT_ILSPYCMD").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    let name = if cfg!(windows) {
        "ilspycmd.exe"
    } else {
        "ilspycmd"
    };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    dirs::home_dir()
        .map(|home| home.join(".dotnet").join("tools").join(name))
        .filter(|path| path.is_file())
}

fn run(
    tool: &Path,
    assembly: &Path,
    args: &[&str],
    cancel: &AtomicU64,
    generation: u64,
) -> Result<String, String> {
    let stdout = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let stderr = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(tool);
    command
        .args(args)
        .arg(assembly)
        .stdout(stdout.reopen().map_err(|e| e.to_string())?)
        .stderr(stderr.reopen().map_err(|e| e.to_string())?);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    loop {
        if cancel.load(Ordering::Relaxed) != generation {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Cancelled".into());
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                let output = std::fs::read_to_string(stdout.path()).map_err(|e| e.to_string())?;
                if status.success() {
                    return Ok(output);
                }
                let error = std::fs::read_to_string(stderr.path()).unwrap_or_default();
                return Err(error.trim().to_string());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(30)),
        }
    }
}

fn parse_types(output: &str) -> Vec<AssemblyType> {
    output
        .lines()
        .filter_map(|line| {
            let (kind, full_name) = line.split_once(' ')?;
            if !["Class", "Interface", "Struct", "Delegate", "Enum"].contains(&kind)
                || full_name.starts_with('<')
            {
                return None;
            }
            let (namespace, name) = full_name.rsplit_once('.').unwrap_or(("", full_name));
            Some(AssemblyType {
                kind: kind.into(),
                full_name: full_name.into(),
                namespace: namespace.into(),
                name: name.into(),
            })
        })
        .collect()
}

fn parse_members(source: &str) -> Vec<Member> {
    let rope = rc_text::Rope::from_str(source);
    let Some(tree) = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None) else {
        return Vec::new();
    };
    let bytes = source.as_bytes();
    let mut members = Vec::new();
    let mut cursor = tree.walk();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let is_field = node.kind() == "variable_declarator"
            && node
                .parent()
                .and_then(|parent| parent.parent())
                .is_some_and(|parent| {
                    matches!(
                        parent.kind(),
                        "field_declaration" | "event_field_declaration"
                    )
                });
        if is_field
            || matches!(
                node.kind(),
                "method_declaration"
                    | "constructor_declaration"
                    | "property_declaration"
                    | "event_declaration"
                    | "indexer_declaration"
            )
        {
            if let Some(name) = node.child_by_field_name("name").or_else(|| {
                (node.kind() == "variable_declarator")
                    .then(|| node.named_child(0))
                    .flatten()
            }) {
                let label = String::from_utf8_lossy(&bytes[name.byte_range()]).into_owned();
                members.push(Member {
                    name: label,
                    line: node.start_position().row,
                });
            } else if node.kind() == "indexer_declaration" {
                members.push(Member {
                    name: "this[]".into(),
                    line: node.start_position().row,
                });
            }
        }
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    members.sort_by_key(|m| m.line);
    members
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaces_types_and_members() {
        let types =
            parse_types("Class Acme.Tools.Widget\nInterface Acme.Tools.IWidget\nClass <Module>\n");
        assert_eq!(types.len(), 2);
        assert_eq!(
            (&types[0].namespace[..], &types[0].name[..]),
            ("Acme.Tools", "Widget")
        );
        let members = parse_members(
            "namespace Acme { class Widget { private int Count; public int Value { get; set; } public void Run() { int local = 1; } } }",
        );
        assert!(members.iter().any(|member| member.name == "Value"));
        assert!(members.iter().any(|member| member.name == "Run"));
        assert!(members.iter().any(|member| member.name == "Count"));
        assert!(!members.iter().any(|member| member.name == "local"));
    }

    #[test]
    fn locates_a_cached_package_assembly() {
        let root = tempfile::tempdir().unwrap();
        let dll = root
            .path()
            .join("newtonsoft.json/13.0.3/lib/net6.0/Newtonsoft.Json.dll");
        std::fs::create_dir_all(dll.parent().unwrap()).unwrap();
        std::fs::write(&dll, []).unwrap();
        assert_eq!(
            find_package_assembly_in(root.path(), "Newtonsoft.Json", "13.0.3"),
            Some(dll)
        );
    }

    #[test]
    fn selects_the_asset_chosen_by_restore() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("App/App.csproj");
        let cache = root.path().join("packages");
        let assembly = cache.join("example/1.0.0/lib/net10.0/Example.dll");
        std::fs::create_dir_all(assembly.parent().unwrap()).unwrap();
        std::fs::create_dir_all(project.parent().unwrap().join("obj")).unwrap();
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>").unwrap();
        std::fs::write(&assembly, []).unwrap();
        std::fs::write(project.parent().unwrap().join("obj/project.assets.json"), r#"{"targets":{"net10.0":{"Example/1.0.0":{"runtime":{"lib/net10.0/Example.dll":{}},"compile":{"ref/net10.0/Example.dll":{}}}}}}"#).unwrap();
        assert_eq!(
            find_package_assembly_from_assets(&project, &cache, "Example", "1.0.0"),
            Some(assembly)
        );
    }

    #[test]
    fn lists_reference_pack_assemblies() {
        let root = tempfile::tempdir().unwrap();
        let assembly = root
            .path()
            .join("packs/Microsoft.NETCore.App.Ref/10.0.0/ref/net10.0/System.Text.Json.dll");
        std::fs::create_dir_all(assembly.parent().unwrap()).unwrap();
        std::fs::write(&assembly, []).unwrap();
        assert_eq!(framework_assemblies_in(root.path()), vec![assembly]);
    }

    #[test]
    fn live_ilspycmd_lists_and_decompiles_when_configured() {
        let (Ok(tool), Ok(assembly)) = (
            std::env::var("DOTCRAFT_ILSPYCMD"),
            std::env::var("DOTCRAFT_TEST_ASSEMBLY"),
        ) else {
            return;
        };
        let cancel = AtomicU64::new(1);
        let types = parse_types(
            &run(
                Path::new(&tool),
                Path::new(&assembly),
                &["--list", "c"],
                &cancel,
                1,
            )
            .unwrap(),
        );
        let ty = types
            .iter()
            .find(|ty| !ty.full_name.contains('<'))
            .expect("assembly has a named class");
        let csharp = run(
            Path::new(&tool),
            Path::new(&assembly),
            &["--type", &ty.full_name],
            &cancel,
            1,
        )
        .unwrap();
        assert!(csharp.contains("class "));
        let il = run(
            Path::new(&tool),
            Path::new(&assembly),
            &["--ilcode", "--type", &ty.full_name],
            &cancel,
            1,
        )
        .unwrap();
        assert!(il.contains(".class "));
    }
}
