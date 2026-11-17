//! Assembly Explorer (Rider): the namespaces and types of .NET assemblies — a .dll or .exe, a
//! NuGet package of the solution, a shared framework assembly — read as C# or IL. The ILSpy
//! command-line tool (`ilspycmd`, a .NET global tool installed on demand) lists and decompiles
//! them on background threads; decompiled sources are cached per assembly build in DotCraft's
//! data folder and open as read-only editor tabs.

use rc_core::{Inbox, Waker};
use rc_kit::i18n::{tr, trf};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeKind {
    Class,
    Interface,
    Struct,
    Delegate,
    Enum,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssemblyType {
    pub kind: TypeKind,
    /// As ILSpy names it: `Acme.Tools.Widget`, `Acme.Tools.Cache`1`.
    pub full_name: String,
    pub namespace: String,
    /// For display: `Widget`, `Cache<T>`.
    pub name: String,
}

/// An assembly listed in the window.
#[derive(Clone, Debug)]
pub struct Assembly {
    pub path: PathBuf,
    /// `None` while the types are listed (or the tool is missing).
    pub types: Option<Vec<AssemblyType>>,
    pub error: Option<String>,
}

impl Assembly {
    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// The namespaces with their types, both sorted (the global namespace last).
    pub fn namespaces(&self) -> Vec<(&str, Vec<&AssemblyType>)> {
        let mut out: Vec<(&str, Vec<&AssemblyType>)> = Vec::new();
        for ty in self.types.iter().flatten() {
            match out.iter_mut().find(|(ns, _)| *ns == ty.namespace) {
                Some((_, list)) => list.push(ty),
                None => out.push((&ty.namespace, vec![ty])),
            }
        }
        out.sort_by(|a, b| (a.0.is_empty(), a.0).cmp(&(b.0.is_empty(), b.0)));
        for (_, list) in &mut out {
            list.sort_by_key(|t| t.name.to_lowercase());
        }
        out
    }
}

enum Event {
    Listed(u64, PathBuf, Result<Vec<AssemblyType>, String>),
    Resolved(u64, Result<Vec<PathBuf>, String>),
    Decompiled(u64, Result<PathBuf, String>),
    ToolMissing(u64),
}

pub struct AssemblyExplorer {
    pub assemblies: Vec<Assembly>,
    /// Expanded rows: `a:<path>` and `n:<path>|<namespace>`.
    pub expanded: HashSet<String>,
    /// The selected row's key (`t:<path>|<type>` for a type).
    pub selected: Option<String>,
    pub filter: String,
    /// `ilspycmd` was not found: the window offers to install it.
    pub tool_missing: bool,
    /// `dotnet tool install` runs in the Build window.
    pub installing: bool,
    /// The type being decompiled, for the status line.
    pub decompiling: Option<String>,
    /// A package being looked up in the NuGet cache.
    pub resolving: Option<String>,
    pub error: Option<String>,
    /// Decompiled files to open in editor tabs.
    pub opened: Vec<PathBuf>,
    /// The `dotnet` installation the tool runs on (DOTNET_ROOT for its apphost).
    pub dotnet_root: Option<PathBuf>,
    inbox: Inbox<Event>,
    /// Bumped when the list is cleared: late listings are dropped and their processes killed.
    session: Arc<AtomicU64>,
    /// Bumped per decompilation: only the last one runs.
    decompile: Arc<AtomicU64>,
}

impl AssemblyExplorer {
    pub fn new(waker: Waker) -> Self {
        Self {
            assemblies: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
            filter: String::new(),
            tool_missing: false,
            installing: false,
            decompiling: None,
            resolving: None,
            error: None,
            opened: Vec::new(),
            dotnet_root: None,
            inbox: Inbox::new(waker),
            session: Arc::new(AtomicU64::new(0)),
            decompile: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Adds an assembly (or selects it when listed already) and lists its types.
    pub fn add(&mut self, path: PathBuf) {
        let path = dunce::canonicalize(&path).unwrap_or(path);
        let key = format!("a:{}", path.display());
        self.selected = Some(key.clone());
        self.expanded.insert(key);
        if self.assemblies.iter().any(|a| a.path == path) {
            return;
        }
        self.assemblies.push(Assembly { path: path.clone(), types: None, error: None });
        self.list(path);
    }

    fn list(&mut self, path: PathBuf) {
        let session = self.session.load(Ordering::Relaxed);
        let current = self.session.clone();
        let out = self.inbox.sender();
        let root = self.dotnet_root.clone();
        std::thread::spawn(move || {
            if !is_managed(&path) {
                out.send(Event::Listed(session, path, Err(tr("This file is not a .NET assembly").into())));
                return;
            }
            let Some(tool) = find_ilspycmd() else {
                out.send(Event::ToolMissing(session));
                return;
            };
            let cancelled = || current.load(Ordering::Relaxed) != session;
            let result = run(&tool, root.as_deref(), &["--list", "cisde"], &path, &cancelled).map(|text| parse_types(&text));
            if !cancelled() {
                out.send(Event::Listed(session, path, result));
            }
        });
    }

    /// Adds the assemblies a restored NuGet package brings to `project` (for `framework`, or
    /// the first target framework).
    pub fn add_package(&mut self, project: PathBuf, framework: Option<String>, name: String, version: String) {
        self.resolving = Some(name.clone());
        self.error = None;
        let session = self.session.load(Ordering::Relaxed);
        let out = self.inbox.sender();
        std::thread::spawn(move || {
            let result = package_assemblies(&project, framework.as_deref(), &name, &version);
            out.send(Event::Resolved(session, result));
        });
    }

    /// Decompiles a type (as C#, or IL) into the cache; the file then opens in an editor tab.
    pub fn decompile(&mut self, assembly: &Path, ty: &AssemblyType, il: bool) {
        let generation = self.decompile.fetch_add(1, Ordering::Relaxed) + 1;
        self.decompiling = Some(ty.name.clone());
        self.error = None;
        let current = self.decompile.clone();
        let out = self.inbox.sender();
        let root = self.dotnet_root.clone();
        let (assembly, full_name) = (assembly.to_path_buf(), ty.full_name.clone());
        std::thread::spawn(move || {
            let Some(tool) = find_ilspycmd() else {
                out.send(Event::ToolMissing(u64::MAX));
                out.send(Event::Decompiled(generation, Err(String::new())));
                return;
            };
            prune_cache_once();
            let file = cache_file(&assembly, &tool, &full_name, il);
            if file.is_file() {
                out.send(Event::Decompiled(generation, Ok(file)));
                return;
            }
            let cancelled = || current.load(Ordering::Relaxed) != generation;
            let args: &[&str] = if il { &["--ilcode", "--type", &full_name] } else { &["--type", &full_name] };
            let result = run(&tool, root.as_deref(), args, &assembly, &cancelled).and_then(|text| {
                rc_core::paths::write_atomic(&file, text.as_bytes()).map_err(|e| e.to_string())?;
                Ok(file)
            });
            if !cancelled() {
                out.send(Event::Decompiled(generation, result));
            }
        });
    }

    /// Removes an assembly from the list.
    pub fn remove(&mut self, path: &Path) {
        self.assemblies.retain(|a| a.path != path);
        self.selected = None;
    }

    /// Empties the window and stops its work (closing the workspace).
    pub fn clear(&mut self) {
        self.session.fetch_add(1, Ordering::Relaxed);
        self.decompile.fetch_add(1, Ordering::Relaxed);
        self.assemblies.clear();
        self.expanded.clear();
        self.selected = None;
        self.filter.clear();
        self.decompiling = None;
        self.resolving = None;
        self.error = None;
        self.opened.clear();
        self.installing = false;
    }

    /// The tool install finished: list again what could not be listed without it.
    pub fn tool_installed(&mut self) {
        self.installing = false;
        self.tool_missing = false;
        let pending: Vec<PathBuf> = self.assemblies.iter().filter(|a| a.types.is_none() && a.error.is_none()).map(|a| a.path.clone()).collect();
        for path in pending {
            self.list(path);
        }
    }

    pub fn poll(&mut self) {
        let session = self.session.load(Ordering::Relaxed);
        let decompile = self.decompile.load(Ordering::Relaxed);
        for event in self.inbox.drain() {
            match event {
                Event::Listed(s, path, result) if s == session => {
                    if let Some(a) = self.assemblies.iter_mut().find(|a| a.path == path) {
                        match result {
                            Ok(types) => a.types = Some(types),
                            Err(e) => a.error = Some(e),
                        }
                    }
                }
                Event::Resolved(s, result) if s == session => {
                    self.resolving = None;
                    match result {
                        Ok(paths) => {
                            for path in paths {
                                self.add(path);
                            }
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Decompiled(g, result) if g == decompile => {
                    self.decompiling = None;
                    match result {
                        Ok(path) => self.opened.push(path),
                        Err(e) if !e.is_empty() => self.error = Some(e),
                        Err(_) => {}
                    }
                }
                Event::ToolMissing(s) if (s == session || s == u64::MAX) && !self.installing => self.tool_missing = true,
                _ => {}
            }
        }
    }
}

/// `dotnet tool install --global ilspycmd`.
pub fn install_args() -> Vec<String> {
    ["tool", "install", "--global", "ilspycmd"].map(String::from).to_vec()
}

/// Where decompiled sources are cached.
pub fn cache_dir() -> PathBuf {
    rc_core::paths::data_dir().join("decompiled")
}

/// Whether `path` is a decompiled source (opened read-only).
pub fn is_decompiled(path: &Path) -> bool {
    let dir = cache_dir();
    path.starts_with(&dir) || dunce::canonicalize(&dir).is_ok_and(|dir| path.starts_with(dir))
}

/// The cached source of a type: one folder per assembly build (and tool version), one folder
/// per namespace, so the editor tab shows the type's own name.
fn cache_file(assembly: &Path, tool: &Path, full_name: &str, il: bool) -> PathBuf {
    let stamp = |p: &Path| std::fs::metadata(p).map(|m| (m.len(), m.modified().ok())).ok();
    let key = format!("{}|{:?}|{:?}", assembly.display(), stamp(assembly), stamp(tool));
    let stem = assembly.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (namespace, name) = full_name.rsplit_once('.').unwrap_or(("_", full_name));
    let safe = |s: &str| s.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '`' | '_' | '-') { c } else { '_' }).collect::<String>();
    cache_dir().join(format!("{}-{:016x}", safe(&stem), fnv(&key))).join(safe(namespace)).join(format!("{}.{}", safe(name), if il { "il" } else { "cs" }))
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// Drops cached assemblies not used for a month, once per run.
fn prune_cache_once() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(cache_dir()) else { return };
    let month = std::time::Duration::from_secs(30 * 24 * 3600);
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > month);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Whether `path` is a .NET assembly: a PE file with a CLI header.
pub fn is_managed(path: &Path) -> bool {
    use std::io::Read;
    let mut head = Vec::with_capacity(4096);
    if std::fs::File::open(path).and_then(|f| f.take(4096).read_to_end(&mut head)).is_err() {
        return false;
    }
    let u16_at = |i: usize| head.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| head.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if head.get(0..2) != Some(b"MZ") {
        return false;
    }
    let Some(pe) = u32_at(0x3c).map(|v| v as usize) else { return false };
    if head.get(pe..pe + 4) != Some(b"PE\0\0") {
        return false;
    }
    let optional = pe + 24;
    let directories = match u16_at(optional) {
        Some(0x10b) => optional + 96,
        Some(0x20b) => optional + 112,
        _ => return false,
    };
    // Data directory 14: the CLI header.
    u32_at(directories + 14 * 8).is_some_and(|rva| rva != 0)
}

/// The assemblies of a restored package for a project: the runtime (or else compile) assets
/// restore picked in `obj/project.assets.json`, or the package's `lib`/`ref` folders when the
/// project was not restored.
fn package_assemblies(project: &Path, framework: Option<&str>, name: &str, version: &str) -> Result<Vec<PathBuf>, String> {
    if let Some(found) = assets_assemblies(project, framework, name) {
        return if found.is_empty() { Err(trf("The package {name} has no assemblies", &[("name", &name)])) } else { Ok(found) };
    }
    let root = std::env::var_os("NUGET_PACKAGES").map(PathBuf::from).or_else(|| dirs::home_dir().map(|home| home.join(".nuget").join("packages")));
    root.and_then(|root| package_folder_assembly(&root, framework, name, version))
        .map(|p| vec![p])
        .ok_or_else(|| trf("{name} {version} is not in the NuGet cache: restore the project first", &[("name", &name), ("version", &version)]))
}

/// `None` when the project was not restored or the package is not in its assets.
fn assets_assemblies(project: &Path, framework: Option<&str>, name: &str) -> Option<Vec<PathBuf>> {
    let assets = project.parent()?.join("obj").join("project.assets.json");
    let document: serde_json::Value = serde_json::from_slice(&std::fs::read(assets).ok()?).ok()?;
    let targets = document.get("targets")?.as_object()?;
    let folders: Vec<PathBuf> = document.get("packageFolders").and_then(|f| f.as_object()).map(|f| f.keys().map(PathBuf::from).collect()).unwrap_or_default();
    let libraries = document.get("libraries")?.as_object()?;
    // Targets with a runtime identifier (`net8.0/osx-arm64`) come after the plain ones.
    let mut target_names: Vec<&String> = targets.keys().filter(|t| framework.is_none_or(|f| *t == f || t.starts_with(&format!("{f}/")))).collect();
    target_names.sort_by_key(|t| t.contains('/'));
    for target in target_names {
        let Some((id, package)) = targets[target].as_object()?.iter().find(|(id, _)| id.split('/').next().is_some_and(|n| n.eq_ignore_ascii_case(name))) else {
            continue;
        };
        let folder = libraries.get(id).and_then(|l| l.get("path")).and_then(|p| p.as_str()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(id.to_ascii_lowercase()));
        for group in ["runtime", "compile"] {
            let dlls: Vec<&String> = package.get(group).and_then(|g| g.as_object()).map(|g| g.keys().filter(|k| is_dll(k) && safe_relative(k)).collect()).unwrap_or_default();
            if dlls.is_empty() {
                continue;
            }
            let mut found: Vec<PathBuf> = dlls.iter().filter_map(|rel| folders.iter().map(|root| root.join(&folder).join(rel)).find(|p| p.is_file())).collect();
            // The package's own assembly first.
            found.sort_by_key(|p| !p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name)));
            if !found.is_empty() {
                return Some(found);
            }
        }
        return Some(Vec::new());
    }
    None
}

fn is_dll(path: &str) -> bool {
    Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("dll"))
}

fn safe_relative(path: &str) -> bool {
    Path::new(path).components().all(|part| matches!(part, std::path::Component::Normal(_)))
}

fn package_folder_assembly(root: &Path, framework: Option<&str>, name: &str, version: &str) -> Option<PathBuf> {
    let package = root.join(name.to_ascii_lowercase()).join(version.to_ascii_lowercase());
    for folder in ["lib", "ref"] {
        let Ok(frameworks) = std::fs::read_dir(package.join(folder)) else { continue };
        let mut paths = Vec::new();
        for dir in frameworks.flatten() {
            if framework.is_some_and(|f| dir.file_name().to_string_lossy() != f) {
                continue;
            }
            let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
            paths.extend(files.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll"))));
        }
        paths.sort();
        if let Some(path) = paths.iter().find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name))) {
            return Some(path.clone());
        }
        if let Some(path) = paths.into_iter().next() {
            return Some(path);
        }
    }
    None
}

/// The folder of the newest shared framework (`shared/Microsoft.NETCore.App/10.0.1`), of
/// version `major` when given and installed: where the framework assembly dialog starts.
pub fn framework_dir(dotnet_root: &Path, framework: &str, major: Option<u64>) -> Option<PathBuf> {
    let mut versions: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(dotnet_root.join("shared").join(framework))
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect(), e.path()))
        .collect();
    versions.sort();
    let wanted = versions.iter().rev().find(|(v, _)| major.is_none_or(|m| v.first() == Some(&m)));
    wanted.or(versions.last()).map(|(_, p)| p.clone())
}

/// `ilspycmd`: `DOTCRAFT_ILSPYCMD`, then `PATH`, then the global tools folder (`~/.dotnet/tools`,
/// not on the PATH of an app started from the Finder or the Dock).
pub fn find_ilspycmd() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOTCRAFT_ILSPYCMD").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    let exe = if cfg!(windows) { "ilspycmd.exe" } else { "ilspycmd" };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("DOTNET_CLI_HOME").map(PathBuf::from) {
        candidates.push(home.join(".dotnet").join("tools").join(exe));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".dotnet").join("tools").join(exe));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Runs `ilspycmd <args> <assembly>` and returns its output; kills it once `cancelled`.
fn run(tool: &Path, dotnet_root: Option<&Path>, args: &[&str], assembly: &Path, cancelled: &dyn Fn() -> bool) -> Result<String, String> {
    // Output goes to temporary files: pipes could fill up while this thread polls.
    let stdout = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let stderr = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(tool);
    command.arg("--disable-updatecheck").args(args).arg(assembly);
    command.stdin(std::process::Stdio::null()).stdout(stdout.reopen().map_err(|e| e.to_string())?).stderr(stderr.reopen().map_err(|e| e.to_string())?);
    if let Some(root) = dotnet_root {
        // The tool's apphost looks for the runtime here (a Finder launch has no DOTNET_ROOT).
        command.env("DOTNET_ROOT", root);
    }
    if std::env::var_os("DOTNET_ROLL_FORWARD").is_none() {
        // Run on a newer runtime when the one the tool targets is not installed.
        command.env("DOTNET_ROLL_FORWARD", "Major");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|e| trf("Could not start ilspycmd: {error}", &[("error", &e)]))?;
    loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(String::new());
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                let output = std::fs::read(stdout.path()).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| e.to_string())?;
                if status.success() {
                    return Ok(output);
                }
                let error = std::fs::read(stderr.path()).map(|b| String::from_utf8_lossy(&b).trim().to_string()).unwrap_or_default();
                let error = if error.is_empty() { output.trim().to_string() } else { error };
                return Err(if error.is_empty() { trf("ilspycmd exited with {status}", &[("status", &status)]) } else { error });
            }
            None => std::thread::sleep(std::time::Duration::from_millis(30)),
        }
    }
}

/// `ilspycmd --list` output: `Class Acme.Tools.Widget`, one per line. Compiler-generated and
/// nested types are left out (nested ones show in their parent's source).
fn parse_types(output: &str) -> Vec<AssemblyType> {
    let mut types: Vec<AssemblyType> = output
        .lines()
        .filter_map(|line| {
            let (kind, full_name) = line.trim().split_once(' ')?;
            let kind = match kind {
                "Class" => TypeKind::Class,
                "Interface" => TypeKind::Interface,
                "Struct" => TypeKind::Struct,
                "Delegate" => TypeKind::Delegate,
                "Enum" => TypeKind::Enum,
                _ => return None,
            };
            if full_name.contains(['<', '+']) {
                return None;
            }
            let (namespace, name) = full_name.rsplit_once('.').unwrap_or(("", full_name));
            Some(AssemblyType { kind, full_name: full_name.into(), namespace: namespace.into(), name: display_name(name) })
        })
        .collect();
    types.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    types.dedup_by(|a, b| a.full_name == b.full_name);
    types
}

/// `Cache`2` → `Cache<T1, T2>`.
fn display_name(name: &str) -> String {
    match name.split_once('`').and_then(|(n, arity)| Some((n, arity.parse::<usize>().ok()?))) {
        Some((n, 1)) => format!("{n}<T>"),
        Some((n, arity)) => format!("{n}<{}>", (1..=arity).map(|i| format!("T{i}")).collect::<Vec<_>>().join(", ")),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_types_without_generated_or_nested_ones() {
        let types = parse_types("Class Acme.Tools.Widget\nInterface Acme.Tools.IWidget\nClass <Module>\nStruct Acme.Tools.Widget+Enumerator\nClass Acme.Cache`2\nDelegate Run\n");
        let names: Vec<_> = types.iter().map(|t| (t.namespace.as_str(), t.name.as_str(), t.kind)).collect();
        assert_eq!(names, [("Acme", "Cache<T1, T2>", TypeKind::Class), ("Acme.Tools", "IWidget", TypeKind::Interface), ("Acme.Tools", "Widget", TypeKind::Class), ("", "Run", TypeKind::Delegate)]);
        let assembly = Assembly { path: "A.dll".into(), types: Some(types), error: None };
        let namespaces: Vec<_> = assembly.namespaces().into_iter().map(|(ns, list)| (ns, list.len())).collect();
        assert_eq!(namespaces, [("Acme", 1), ("Acme.Tools", 2), ("", 1)]);
    }

    #[test]
    fn locates_a_cached_package_assembly() {
        let root = tempfile::tempdir().unwrap();
        let dll = root.path().join("newtonsoft.json/13.0.3/lib/net6.0/Newtonsoft.Json.dll");
        std::fs::create_dir_all(dll.parent().unwrap()).unwrap();
        std::fs::write(&dll, []).unwrap();
        assert_eq!(package_folder_assembly(root.path(), None, "Newtonsoft.Json", "13.0.3"), Some(dll));
    }

    #[test]
    fn takes_the_assets_restore_picked_for_the_framework() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("App/App.csproj");
        let cache = root.path().join("packages");
        let current = cache.join("example/1.0.0/lib/net10.0/Example.dll");
        let older = cache.join("example/1.0.0/lib/net8.0/Example.dll");
        let extra = cache.join("example/1.0.0/lib/net10.0/Example.Extra.dll");
        for file in [&current, &older, &extra] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, []).unwrap();
        }
        std::fs::create_dir_all(project.parent().unwrap().join("obj")).unwrap();
        let assets = serde_json::json!({
            "targets": {
                "net8.0": { "Example/1.0.0": { "type": "package", "runtime": { "lib/net8.0/Example.dll": {} } }, "Shim/2.0.0": { "type": "package", "runtime": { "lib/net8.0/_._": {} } } },
                "net10.0": { "Example/1.0.0": { "type": "package", "runtime": { "lib/net10.0/Example.dll": {}, "lib/net10.0/Example.Extra.dll": {} }, "compile": { "ref/net10.0/Example.dll": {} } } }
            },
            "libraries": { "Example/1.0.0": { "path": "example/1.0.0" }, "Shim/2.0.0": { "path": "shim/2.0.0" } },
            "packageFolders": { format!("{}/", cache.display()): {} }
        });
        std::fs::write(project.parent().unwrap().join("obj/project.assets.json"), assets.to_string()).unwrap();
        assert_eq!(assets_assemblies(&project, Some("net10.0"), "example"), Some(vec![current, extra]));
        assert_eq!(assets_assemblies(&project, Some("net8.0"), "Example"), Some(vec![older]));
        // A package without assemblies of its own, and one the project does not use.
        assert_eq!(assets_assemblies(&project, Some("net8.0"), "Shim"), Some(vec![]));
        assert_eq!(assets_assemblies(&project, None, "Other"), None);
    }

    #[test]
    fn tells_managed_assemblies_from_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("readme.dll");
        std::fs::write(&text, "not a PE file").unwrap();
        assert!(!is_managed(&text));
        // A minimal PE32+ header with a CLI data directory, and the same without it.
        let mut pe = vec![0u8; 512];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        pe[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
        let cli = 0x98 + 112 + 14 * 8;
        let native = dir.path().join("native.dll");
        std::fs::write(&native, &pe).unwrap();
        assert!(!is_managed(&native));
        pe[cli..cli + 4].copy_from_slice(&0x2008u32.to_le_bytes());
        let managed = dir.path().join("managed.dll");
        std::fs::write(&managed, &pe).unwrap();
        assert!(is_managed(&managed));
    }

    #[test]
    fn caches_sources_per_assembly_build_and_namespace() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let assembly = dir.path().join("Acme.dll");
        std::fs::write(&assembly, "v1").unwrap();
        let tool = dir.path().join("ilspycmd");
        let first = cache_file(&assembly, &tool, "Acme.Tools.Cache`1", false);
        assert!(first.starts_with(cache_dir()) && first.ends_with("Acme.Tools/Cache`1.cs"), "{}", first.display());
        assert_eq!(cache_file(&assembly, &tool, "Acme.Tools.Cache`1", true).extension().unwrap(), "il");
        std::fs::write(&assembly, "version 2").unwrap();
        assert_ne!(cache_file(&assembly, &tool, "Acme.Tools.Cache`1", false), first);
        assert!(is_decompiled(&first));
    }

    #[test]
    fn finds_the_newest_shared_framework() {
        let root = tempfile::tempdir().unwrap();
        for v in ["8.0.24", "10.0.14", "10.0.3"] {
            std::fs::create_dir_all(root.path().join("shared/Microsoft.NETCore.App").join(v)).unwrap();
        }
        let shared = root.path().join("shared/Microsoft.NETCore.App");
        assert_eq!(framework_dir(root.path(), "Microsoft.NETCore.App", None), Some(shared.join("10.0.14")));
        assert_eq!(framework_dir(root.path(), "Microsoft.NETCore.App", Some(8)), Some(shared.join("8.0.24")));
        assert_eq!(framework_dir(root.path(), "Microsoft.NETCore.App", Some(9)), Some(shared.join("10.0.14")));
        assert_eq!(framework_dir(root.path(), "Microsoft.AspNetCore.App", None), None);
    }

    /// Runs the real tool: `DOTCRAFT_ILSPYCMD=…/ilspycmd DOTCRAFT_TEST_ASSEMBLY=…/Some.dll`.
    #[test]
    fn live_ilspycmd_lists_and_decompiles_when_configured() {
        let (Ok(tool), Ok(assembly)) = (std::env::var("DOTCRAFT_ILSPYCMD"), std::env::var("DOTCRAFT_TEST_ASSEMBLY")) else {
            return;
        };
        let (tool, assembly) = (Path::new(&tool), Path::new(&assembly));
        assert!(is_managed(assembly));
        let root = rc_dotnet::sdk::Dotnet::locate(None).and_then(|d| d.root().map(Path::to_path_buf));
        let never = || false;
        let types = parse_types(&run(tool, root.as_deref(), &["--list", "cisde"], assembly, &never).unwrap());
        let ty = types.iter().find(|t| t.kind == TypeKind::Class).expect("assembly has a class");
        let csharp = run(tool, root.as_deref(), &["--type", &ty.full_name], assembly, &never).unwrap();
        assert!(csharp.contains("class "), "{csharp}");
        let il = run(tool, root.as_deref(), &["--ilcode", "--type", &ty.full_name], assembly, &never).unwrap();
        assert!(il.contains(".class "), "{il}");
        let cancelled = || true;
        assert_eq!(run(tool, root.as_deref(), &["--list", "c"], assembly, &cancelled), Err(String::new()));
    }
}
