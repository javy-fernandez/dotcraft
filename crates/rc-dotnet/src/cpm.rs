//! Central Package Management (CPM): which projects use it and which `Directory.Packages.props`
//! applies to them (evaluated by MSBuild, with a quick static pass while that runs), the issues
//! NuGet reports for it (NU1008, NU1010, unused versions), and minimal text edits of central
//! files and project files that keep formatting, comments and conditions.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::sdk::Dotnet;

/// The central versions file.
pub const CENTRAL_FILE: &str = "Directory.Packages.props";

// ---------------------------------------------------------------- central files

/// A `PackageVersion` or `GlobalPackageReference` item of a central file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralItem {
    pub id: String,
    /// `Update` changes an imported item; `Include` introduces one.
    pub update: bool,
    /// `Version` as written (may be `$(Property)`).
    pub version: String,
    /// `Version` with `$(Property)` resolved from the file's own properties.
    pub resolved: String,
    /// Condition of the item or of its `ItemGroup`.
    pub condition: Option<String>,
    /// `GlobalPackageReference` (referenced by every project) rather than `PackageVersion`.
    pub global: bool,
    /// Zero-based line of the item.
    pub line: usize,
}

/// One `Directory.Packages.props`, read statically.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CentralFile {
    pub path: PathBuf,
    pub items: Vec<CentralItem>,
    /// Parent central files it imports (nested `Directory.Packages.props`).
    pub imports: Vec<PathBuf>,
    /// `ManagePackageVersionsCentrally` when the file sets it.
    pub manage: Option<bool>,
    /// `CentralPackageTransitivePinningEnabled` when the file sets it.
    pub transitive_pinning: Option<bool>,
}

impl CentralFile {
    pub fn load(path: &Path) -> anyhow::Result<CentralFile> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(path, &text)
    }

    pub fn parse(path: &Path, text: &str) -> anyhow::Result<CentralFile> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let doc = roxmltree::Document::parse(text)?;
        let props = properties(&doc);
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut file = CentralFile {
            path: path.to_path_buf(),
            manage: props.get("ManagePackageVersionsCentrally").map(|v| v.eq_ignore_ascii_case("true")),
            transitive_pinning: props.get("CentralPackageTransitivePinningEnabled").map(|v| v.eq_ignore_ascii_case("true")),
            ..Default::default()
        };
        for n in doc.descendants().filter(|n| n.is_element()) {
            match n.tag_name().name() {
                tag @ ("PackageVersion" | "GlobalPackageReference") => {
                    let (id, update) = match (n.attribute("Include"), n.attribute("Update")) {
                        (Some(id), _) => (id, false),
                        (None, Some(id)) => (id, true),
                        (None, None) => continue,
                    };
                    let version = version_of(n, "Version").unwrap_or_default();
                    file.items.push(CentralItem {
                        id: id.to_string(),
                        update,
                        resolved: resolve(&version, &props),
                        version,
                        condition: condition_of(n),
                        global: tag == "GlobalPackageReference",
                        line: doc.text_pos_at(n.range().start).row as usize - 1,
                    });
                }
                "Import" => {
                    if let Some(p) = n.attribute("Project").and_then(|p| resolve_import(dir, p)) {
                        file.imports.push(p);
                    }
                }
                _ => {}
            }
        }
        Ok(file)
    }

    /// The `PackageVersion` items of `id` (any condition).
    pub fn versions<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a CentralItem> + 'a {
        self.items.iter().filter(move |i| !i.global && i.id.eq_ignore_ascii_case(id))
    }

    /// The unconditional `PackageVersion` of `id`.
    pub fn unconditional(&self, id: &str) -> Option<&CentralItem> {
        self.items.iter().find(|i| !i.global && i.condition.is_none() && i.id.eq_ignore_ascii_case(id))
    }
}

/// The nearest `Directory.Packages.props` from `dir` up to the root.
pub fn find_central_file(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().map(|d| d.join(CENTRAL_FILE)).find(|p| p.is_file())
}

/// `path` and the central files it imports, nearest first (cycles are ignored).
pub fn central_chain(path: &Path) -> Vec<CentralFile> {
    let mut out: Vec<CentralFile> = Vec::new();
    let mut next = vec![path.to_path_buf()];
    while let Some(p) = next.pop() {
        if out.iter().any(|f| f.path == p) || out.len() > 16 {
            continue;
        }
        let Ok(f) = CentralFile::load(&p) else { continue };
        next.extend(f.imports.iter().rev().cloned());
        out.push(f);
    }
    out
}

/// `<Import Project="…"/>` of a central file: `GetPathOfFileAbove(Directory.Packages.props, …)`
/// is the parent central file; other paths are taken relative to the file.
fn resolve_import(dir: &Path, project: &str) -> Option<PathBuf> {
    if project.contains("GetPathOfFileAbove") {
        let file = if project.contains(CENTRAL_FILE) { CENTRAL_FILE } else { return None };
        return dir.parent().and_then(|parent| parent.ancestors().map(|d| d.join(file)).find(|p| p.is_file()));
    }
    let rel = project.replace("$(MSBuildThisFileDirectory)", "").replace('\\', "/");
    if rel.contains("$(") {
        return None;
    }
    let p = dir.join(rel);
    p.is_file().then(|| dunce::canonicalize(&p).unwrap_or(p))
}

fn properties(doc: &roxmltree::Document) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for g in doc.root_element().children().filter(|n| n.has_tag_name("PropertyGroup") && n.attribute("Condition").is_none()) {
        for p in g.children().filter(|n| n.is_element() && n.attribute("Condition").is_none()) {
            out.insert(p.tag_name().name().to_string(), p.text().unwrap_or("").trim().to_string());
        }
    }
    out
}

fn resolve(version: &str, props: &HashMap<String, String>) -> String {
    match property_ref(version) {
        Some(name) => props.get(name).cloned().unwrap_or_else(|| version.to_string()),
        None => version.to_string(),
    }
}

/// `$(Name)` → `Name`.
fn property_ref(value: &str) -> Option<&str> {
    value.trim().strip_prefix("$(")?.strip_suffix(')').filter(|n| !n.contains(['(', ')', '$']))
}

/// A version attribute or child element.
fn version_of(n: roxmltree::Node, name: &str) -> Option<String> {
    n.attribute(name).map(str::to_string).or_else(|| n.children().find(|c| c.has_tag_name(name)).map(|c| c.text().unwrap_or("").trim().to_string()))
}

/// The condition of an item, or of its `ItemGroup`.
fn condition_of(n: roxmltree::Node) -> Option<String> {
    n.attribute("Condition").or_else(|| n.parent_element().and_then(|g| g.attribute("Condition"))).map(|c| c.trim().to_string()).filter(|c| !c.is_empty())
}

// ---------------------------------------------------------------- projects

/// A `PackageReference` as CPM sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageUse {
    pub id: String,
    /// `Version` on the reference (NU1008 under CPM).
    pub version: Option<String>,
    pub version_override: Option<String>,
    /// The file that declares it: the project or an imported file (`Directory.Build.props`).
    pub defined_in: PathBuf,
}

/// A version the project gets from a central file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedVersion {
    pub id: String,
    pub version: String,
    /// The central file that defines it (a parent one for nested files).
    pub file: PathBuf,
}

/// What CPM means for one project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectCpm {
    pub project: PathBuf,
    /// `ManagePackageVersionsCentrally` is true.
    pub enabled: bool,
    /// The `Directory.Packages.props` that applies (nearest up the tree).
    pub central_file: Option<PathBuf>,
    pub transitive_pinning: bool,
    /// `CentralPackageVersionOverrideEnabled` is not false.
    pub override_enabled: bool,
    pub references: Vec<PackageUse>,
    /// The `PackageVersion` items in effect for the project.
    pub versions: Vec<ResolvedVersion>,
    pub globals: Vec<ResolvedVersion>,
    /// Evaluated by MSBuild (false: the quick static pass).
    pub evaluated: bool,
}

impl ProjectCpm {
    pub fn reference(&self, id: &str) -> Option<&PackageUse> {
        self.references.iter().find(|r| r.id.eq_ignore_ascii_case(id))
    }

    pub fn central_version(&self, id: &str) -> Option<&ResolvedVersion> {
        self.versions.iter().find(|v| v.id.eq_ignore_ascii_case(id))
    }

    pub fn global(&self, id: &str) -> Option<&ResolvedVersion> {
        self.globals.iter().find(|v| v.id.eq_ignore_ascii_case(id))
    }

    /// Whether the version of `id` comes from the central file (no `Version` on the reference).
    pub fn is_central(&self, id: &str) -> bool {
        self.enabled && (self.global(id).is_some() || (self.reference(id).is_some_and(|r| r.version.is_none()) && self.central_version(id).is_some()))
    }

    /// The version the project resolves for `id`: override, central version or its own.
    pub fn effective_version(&self, id: &str) -> Option<String> {
        if let Some(g) = self.global(id) {
            return Some(g.version.clone());
        }
        let r = self.reference(id)?;
        if !self.enabled {
            return r.version.clone();
        }
        r.version_override.clone().or_else(|| r.version.clone()).or_else(|| self.central_version(id).map(|v| v.version.clone()))
    }
}

/// Properties read by [`detect`].
pub const EVAL_PROPERTIES: &[&str] = &["ManagePackageVersionsCentrally", "DirectoryPackagesPropsPath", "CentralPackageTransitivePinningEnabled", "CentralPackageVersionOverrideEnabled"];
/// Items read by [`detect`].
pub const EVAL_ITEMS: &[&str] = &["PackageReference", "PackageVersion", "GlobalPackageReference"];

/// Quick pass without MSBuild: the project file, the nearest `Directory.Build.props` and the
/// central file chain (unconditional items only).
pub fn detect_static(project: &Path) -> ProjectCpm {
    let dir = project.parent().unwrap_or(Path::new("."));
    let mut cpm = ProjectCpm { project: project.to_path_buf(), override_enabled: true, ..Default::default() };
    let text = std::fs::read_to_string(project).unwrap_or_default();
    let doc = roxmltree::Document::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)).ok();
    let own = doc.as_ref().map(properties).unwrap_or_default();
    let build = dir.ancestors().map(|d| d.join("Directory.Build.props")).find(|p| p.is_file());
    let build_props = build.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| roxmltree::Document::parse(&t).ok().map(|d| properties(&d))).unwrap_or_default();
    cpm.central_file = find_central_file(dir).map(|p| dunce::canonicalize(&p).unwrap_or(p));
    let chain = cpm.central_file.as_deref().map(central_chain).unwrap_or_default();
    let flag = |name: &str, central: Option<bool>| own.get(name).map(|v| v.eq_ignore_ascii_case("true")).or(central).or_else(|| build_props.get(name).map(|v| v.eq_ignore_ascii_case("true")));
    cpm.enabled = flag("ManagePackageVersionsCentrally", chain.iter().find_map(|f| f.manage)).unwrap_or(false);
    cpm.transitive_pinning = flag("CentralPackageTransitivePinningEnabled", chain.iter().find_map(|f| f.transitive_pinning)).unwrap_or(false);
    cpm.override_enabled = !own.get("CentralPackageVersionOverrideEnabled").or(build_props.get("CentralPackageVersionOverrideEnabled")).is_some_and(|v| v.eq_ignore_ascii_case("false"));
    // Imported files establish items before the nearest file modifies them.
    for f in chain.iter().rev() {
        for i in f.items.iter().filter(|i| i.condition.is_none()) {
            let list = if i.global { &mut cpm.globals } else { &mut cpm.versions };
            if i.update && i.version.is_empty() {
                continue;
            }
            let resolved = ResolvedVersion { id: i.id.clone(), version: i.resolved.clone(), file: f.path.clone() };
            if let Some(existing) = list.iter_mut().find(|v| v.id.eq_ignore_ascii_case(&i.id)) {
                *existing = resolved;
            } else if !i.update {
                list.push(resolved);
            }
        }
    }
    if let Some(doc) = &doc {
        for n in doc.descendants().filter(|n| n.has_tag_name("PackageReference")) {
            if let Some(id) = n.attribute("Include") {
                cpm.references.push(PackageUse { id: id.to_string(), version: version_of(n, "Version"), version_override: version_of(n, "VersionOverride"), defined_in: project.to_path_buf() });
            }
        }
    }
    cpm
}

/// Evaluates `project` with MSBuild (`-getProperty`/`-getItem`), which also sees CPM turned on
/// in `Directory.Build.props` or imported files.
pub async fn detect(dotnet: &Dotnet, project: &Path) -> anyhow::Result<ProjectCpm> {
    let mut cmd = dotnet.command();
    cmd.arg("msbuild").arg(project).arg("-nologo");
    for p in EVAL_PROPERTIES {
        cmd.arg(format!("-getProperty:{p}"));
    }
    for i in EVAL_ITEMS {
        cmd.arg(format!("-getItem:{i}"));
    }
    if let Some(dir) = project.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    cmd.kill_on_drop(true);
    let out = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    anyhow::ensure!(out.status.success(), "dotnet msbuild failed for {}: {}", project.display(), stdout.trim());
    parse_evaluation(project, &stdout)
}

/// Evaluates several projects, `parallel` at a time.
pub async fn detect_all(dotnet: &Dotnet, projects: Vec<PathBuf>, parallel: usize) -> Vec<(PathBuf, anyhow::Result<ProjectCpm>)> {
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(parallel.max(1)));
    let mut set = tokio::task::JoinSet::new();
    for p in projects {
        let (dotnet, gate) = (dotnet.clone(), gate.clone());
        set.spawn(async move {
            let _permit = gate.acquire_owned().await;
            let r = detect(&dotnet, &p).await;
            (p, r)
        });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        if let Ok(r) = r {
            out.push(r);
        }
    }
    out
}

/// Builds the project's CPM view from the JSON of `-getProperty`/`-getItem` (pure, for tests).
pub fn parse_evaluation(project: &Path, json: &str) -> anyhow::Result<ProjectCpm> {
    let start = json.find('{').ok_or_else(|| anyhow::anyhow!("no JSON in msbuild output"))?;
    let v: serde_json::Value = serde_json::from_str(&json[start..])?;
    let prop = |k: &str| v.get("Properties").and_then(|p| p.get(k)).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let items = |k: &str| v.get("Items").and_then(|i| i.get(k)).and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let meta = |i: &serde_json::Value, k: &str| i.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let canon = |p: String| dunce::canonicalize(&p).unwrap_or_else(|_| PathBuf::from(p));
    let central = prop("DirectoryPackagesPropsPath");
    let me = dunce::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let mut cpm = ProjectCpm {
        project: project.to_path_buf(),
        enabled: prop("ManagePackageVersionsCentrally").eq_ignore_ascii_case("true"),
        central_file: (!central.is_empty() && Path::new(&central).is_file()).then(|| canon(central)),
        transitive_pinning: prop("CentralPackageTransitivePinningEnabled").eq_ignore_ascii_case("true"),
        override_enabled: !prop("CentralPackageVersionOverrideEnabled").eq_ignore_ascii_case("false"),
        evaluated: true,
        ..Default::default()
    };
    let resolved = |list: Vec<serde_json::Value>| -> Vec<ResolvedVersion> {
        let mut out: Vec<ResolvedVersion> = Vec::new();
        for i in list {
            let (Some(id), Some(file)) = (meta(&i, "Identity"), meta(&i, "DefiningProjectFullPath")) else { continue };
            // Later items replace earlier ones (a nested file after the parent it imports).
            out.retain(|v| !v.id.eq_ignore_ascii_case(&id));
            out.push(ResolvedVersion { id, version: meta(&i, "Version").unwrap_or_default(), file: canon(file) });
        }
        out
    };
    cpm.globals = resolved(items("GlobalPackageReference"));
    // NuGet.targets turns every global reference into a PackageVersion and a PackageReference.
    let is_sdk_file = |f: &str| Path::new(f).file_name().is_some_and(|n| n == "NuGet.targets");
    cpm.versions = resolved(items("PackageVersion").into_iter().filter(|i| !meta(i, "DefiningProjectFullPath").is_some_and(|f| is_sdk_file(&f))).collect());
    for i in items("PackageReference") {
        let (Some(id), Some(file)) = (meta(&i, "Identity"), meta(&i, "DefiningProjectFullPath")) else { continue };
        if is_sdk_file(&file) && cpm.global(&id).is_some() {
            continue;
        }
        // The project file under the caller's spelling of its path.
        let file = canon(file);
        let defined_in = if file == me { project.to_path_buf() } else { file };
        cpm.references.push(PackageUse { id, version: meta(&i, "Version"), version_override: meta(&i, "VersionOverride"), defined_in });
    }
    Ok(cpm)
}

// ---------------------------------------------------------------- cache

type Fingerprint = Vec<(PathBuf, Option<SystemTime>)>;

/// Detection results by project, valid while the project, the `Directory.*` files above it and
/// its central files are unchanged.
#[derive(Default)]
pub struct CpmCache {
    entries: HashMap<PathBuf, (Fingerprint, ProjectCpm)>,
}

impl CpmCache {
    /// The cached result when nothing it depends on changed.
    pub fn get(&self, project: &Path) -> Option<&ProjectCpm> {
        let (fp, cpm) = self.entries.get(project)?;
        (*fp == fingerprint(cpm)).then_some(cpm)
    }

    pub fn insert(&mut self, cpm: ProjectCpm) {
        self.entries.insert(cpm.project.clone(), (fingerprint(&cpm), cpm));
    }

    pub fn invalidate(&mut self, project: &Path) {
        self.entries.remove(project);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn fingerprint(cpm: &ProjectCpm) -> Fingerprint {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut files = vec![cpm.project.clone()];
    for d in cpm.project.parent().into_iter().flat_map(Path::ancestors) {
        for name in ["Directory.Build.props", "Directory.Build.targets", CENTRAL_FILE] {
            let p = d.join(name);
            if p.is_file() {
                files.push(p);
            }
        }
    }
    files.extend(cpm.versions.iter().chain(&cpm.globals).map(|v| v.file.clone()));
    files.sort();
    files.dedup();
    files.into_iter().map(|p| (mtime(&p), p)).map(|(t, p)| (p, t)).collect()
}

// ---------------------------------------------------------------- solution

/// A CPM problem with a one-click fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CpmIssue {
    /// A `PackageVersion` no project uses: remove it.
    UnusedVersion { file: PathBuf, id: String },
    /// NU1008: a reference with `Version` under CPM: move the version to the central file.
    VersionOnReference { project: PathBuf, id: String, version: String },
    /// NU1010: a reference without a central version: add one.
    MissingVersion { project: PathBuf, id: String },
}

/// CPM across the solution's projects.
#[derive(Clone, Debug, Default)]
pub struct SolutionCpm {
    pub projects: Vec<ProjectCpm>,
    /// Every central file the projects use, with their imports.
    pub files: BTreeMap<PathBuf, CentralFile>,
}

impl SolutionCpm {
    pub fn new(projects: Vec<ProjectCpm>) -> SolutionCpm {
        let mut files = BTreeMap::new();
        for p in projects.iter().filter(|p| p.enabled) {
            if let Some(c) = &p.central_file
                && !files.contains_key(c)
            {
                for f in central_chain(c) {
                    files.entry(f.path.clone()).or_insert(f);
                }
            }
        }
        SolutionCpm { projects, files }
    }

    pub fn project(&self, project: &Path) -> Option<&ProjectCpm> {
        self.projects.iter().find(|p| p.project == project)
    }

    /// Some project has CPM on.
    pub fn in_use(&self) -> bool {
        self.projects.iter().any(|p| p.enabled)
    }

    /// The project manages `id` centrally.
    pub fn is_central(&self, project: &Path, id: &str) -> bool {
        self.project(project).is_some_and(|p| p.is_central(id))
    }

    /// CPM projects that reference `id`.
    pub fn users(&self, id: &str) -> Vec<&ProjectCpm> {
        self.projects.iter().filter(|p| p.enabled && (p.reference(id).is_some() || p.global(id).is_some())).collect()
    }

    /// The central files `project` reads, nearest first.
    pub fn chain(&self, project: &Path) -> Vec<&CentralFile> {
        let mut out: Vec<&CentralFile> = Vec::new();
        let mut next: Vec<PathBuf> = self.project(project).and_then(|p| p.central_file.clone()).into_iter().collect();
        while let Some(p) = next.pop() {
            if let Some(f) = self.files.get(&p)
                && !out.iter().any(|o| o.path == p)
            {
                next.extend(f.imports.iter().rev().cloned());
                out.push(f);
            }
        }
        out
    }

    /// The central file that holds (or should hold) the version of `id` for `project`: where its
    /// version is defined, otherwise the project's nearest central file.
    pub fn file_for(&self, project: &Path, id: &str) -> Option<PathBuf> {
        let p = self.project(project)?;
        p.central_version(id).map(|v| v.file.clone()).or_else(|| p.global(id).map(|v| v.file.clone())).or_else(|| p.central_file.clone())
    }

    /// Projects whose central chain includes `file`.
    fn readers(&self, file: &Path) -> Vec<&ProjectCpm> {
        self.projects.iter().filter(|p| p.enabled && self.chain(&p.project).iter().any(|f| f.path == file)).collect()
    }

    /// Whether some project still references `id` through `file`.
    pub fn is_used(&self, file: &Path, id: &str) -> bool {
        self.readers(file).iter().any(|p| p.reference(id).is_some())
    }

    pub fn issues(&self) -> Vec<CpmIssue> {
        let mut out = Vec::new();
        for (path, f) in &self.files {
            let readers = self.readers(path);
            // Versions pinned for transitive packages are used without a reference.
            if readers.is_empty() || readers.iter().any(|p| p.transitive_pinning) {
                continue;
            }
            let mut seen: Vec<&str> = Vec::new();
            for i in f.items.iter().filter(|i| !i.global) {
                if !seen.iter().any(|s| s.eq_ignore_ascii_case(&i.id)) && !readers.iter().any(|p| p.reference(&i.id).is_some()) {
                    seen.push(&i.id);
                    out.push(CpmIssue::UnusedVersion { file: path.clone(), id: i.id.clone() });
                }
            }
        }
        for p in self.projects.iter().filter(|p| p.enabled) {
            for r in p.references.iter().filter(|r| r.defined_in == p.project) {
                if let Some(v) = &r.version {
                    out.push(CpmIssue::VersionOnReference { project: p.project.clone(), id: r.id.clone(), version: v.clone() });
                } else if r.version_override.is_none() && p.central_version(&r.id).is_none() && p.global(&r.id).is_none() {
                    out.push(CpmIssue::MissingVersion { project: p.project.clone(), id: r.id.clone() });
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------- text edits

/// A replacement of a byte range of the file's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
}

/// Applies non-overlapping edits.
pub fn apply_edits(text: &str, edits: &[TextEdit]) -> String {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    let mut out = text.to_string();
    for e in sorted {
        out.replace_range(e.range.clone(), &e.text);
    }
    out
}

/// Sets the central version of `id`: the `PackageVersion` with `condition` (`None`: the
/// unconditional one, or the only one), or its `GlobalPackageReference`. A `$(Property)` version
/// changes the property in the same file.
pub fn set_central_version(text: &str, id: &str, condition: Option<&str>, version: &str) -> anyhow::Result<Vec<TextEdit>> {
    let doc = parse(text)?;
    let items: Vec<roxmltree::Node> = doc.descendants().filter(|n| n.has_tag_name("PackageVersion") && central_item_matches(n, id)).collect();
    let item = match condition {
        Some(c) => items.iter().find(|n| condition_of(**n).as_deref() == Some(c.trim())).copied(),
        None => items.iter().find(|n| condition_of(**n).is_none()).copied().or_else(|| (items.len() == 1).then(|| items[0])),
    }
    .or_else(|| doc.descendants().find(|n| n.has_tag_name("GlobalPackageReference") && central_item_matches(&n, id)))
    .ok_or_else(|| anyhow::anyhow!("{id} has no PackageVersion to update"))?;
    set_version_on(&doc, text, item, "Version", version)
}

/// Adds `<PackageVersion Include="id" Version="v" />`, sorted by id within the first
/// unconditional `ItemGroup` of versions (a new `ItemGroup` when there is none).
pub fn add_central_version(text: &str, id: &str, version: &str) -> anyhow::Result<Vec<TextEdit>> {
    let doc = parse(text)?;
    anyhow::ensure!(!doc.descendants().any(|n| n.has_tag_name("PackageVersion") && includes(&n, id) && condition_of(n).is_none()), "{id} already has a PackageVersion");
    let elem = format!(r#"<PackageVersion Include="{}" Version="{}" />"#, escape(id), escape(version));
    Ok(vec![insert_item(&doc, text, "PackageVersion", id, &elem)])
}

/// Removes every `PackageVersion` of `id` (whole lines when they are alone on them).
pub fn remove_central_version(text: &str, id: &str) -> anyhow::Result<Vec<TextEdit>> {
    let doc = parse(text)?;
    let edits: Vec<TextEdit> = doc.descendants().filter(|n| n.has_tag_name("PackageVersion") && central_item_matches(n, id)).map(|n| TextEdit { range: line_span(text, n.range()), text: String::new() }).collect();
    anyhow::ensure!(!edits.is_empty(), "{id} has no PackageVersion");
    Ok(edits)
}

/// Adds `<PackageReference Include="id" />` (no version: CPM supplies it) to a project, sorted
/// within its first unconditional `ItemGroup` of references.
pub fn add_reference(text: &str, id: &str) -> anyhow::Result<Vec<TextEdit>> {
    let doc = parse(text)?;
    anyhow::ensure!(!doc.descendants().any(|n| n.has_tag_name("PackageReference") && includes(&n, id)), "the project already references {id}");
    let elem = format!(r#"<PackageReference Include="{}" />"#, escape(id));
    Ok(vec![insert_item(&doc, text, "PackageReference", id, &elem)])
}

/// Removes the project's references to `id`, and their `ItemGroup` when nothing else is left
/// in it.
pub fn remove_reference(text: &str, id: &str) -> anyhow::Result<Vec<TextEdit>> {
    let doc = parse(text)?;
    let refs: Vec<roxmltree::Node> = doc.descendants().filter(|n| n.has_tag_name("PackageReference") && includes(n, id)).collect();
    anyhow::ensure!(!refs.is_empty(), "the project does not reference {id}");
    let mut edits = Vec::new();
    let mut groups_done: Vec<roxmltree::NodeId> = Vec::new();
    for r in &refs {
        let Some(g) = r.parent_element() else { continue };
        let removable = g.has_tag_name("ItemGroup") && g.children().all(|c| c.is_text() && c.text().is_some_and(|t| t.trim().is_empty()) || refs.contains(&c));
        if removable {
            if !groups_done.contains(&g.id()) {
                groups_done.push(g.id());
                edits.push(TextEdit { range: group_span(text, g.range()), text: String::new() });
            }
        } else {
            edits.push(TextEdit { range: line_span(text, r.range()), text: String::new() });
        }
    }
    Ok(edits)
}

/// Removes `VersionOverride` from the project's references to `id` (they follow the central
/// version again).
pub fn remove_version_override(text: &str, id: &str) -> anyhow::Result<Vec<TextEdit>> {
    remove_metadata(text, id, "VersionOverride")
}

/// Removes `Version` from the project's references to `id`; returns the version it had.
pub fn remove_reference_version(text: &str, id: &str) -> anyhow::Result<(Vec<TextEdit>, String)> {
    let doc = parse(text)?;
    let version = doc.descendants().filter(|n| n.has_tag_name("PackageReference") && includes(n, id)).find_map(|n| version_of(n, "Version")).ok_or_else(|| anyhow::anyhow!("{id} has no Version"))?;
    Ok((remove_metadata(text, id, "Version")?, version))
}

/// Sets `VersionOverride` (or `Version` when `name` says so) on the project's reference to `id`.
pub fn set_reference_metadata(text: &str, id: &str, name: &str, value: &str) -> anyhow::Result<Vec<TextEdit>> {
    let doc = parse(text)?;
    let r = doc.descendants().find(|n| n.has_tag_name("PackageReference") && includes(n, id)).ok_or_else(|| anyhow::anyhow!("the project does not reference {id}"))?;
    set_version_on(&doc, text, r, name, value)
}

fn remove_metadata(text: &str, id: &str, name: &str) -> anyhow::Result<Vec<TextEdit>> {
    let doc = parse(text)?;
    let mut edits = Vec::new();
    for n in doc.descendants().filter(|n| n.has_tag_name("PackageReference") && includes(n, id)) {
        if let Some(a) = n.attributes().find(|a| a.name() == name) {
            // The attribute and the blanks before it.
            let r = a.range();
            let start = text[..r.start].trim_end_matches([' ', '\t', '\r', '\n']).len();
            edits.push(TextEdit { range: start..r.end, text: String::new() });
        }
        for c in n.children().filter(|c| c.has_tag_name(name)) {
            edits.push(TextEdit { range: line_span(text, c.range()), text: String::new() });
        }
    }
    anyhow::ensure!(!edits.is_empty(), "{id} has no {name}");
    Ok(edits)
}

fn parse(text: &str) -> anyhow::Result<roxmltree::Document<'_>> {
    Ok(roxmltree::Document::parse(text)?)
}

fn includes(n: &roxmltree::Node, id: &str) -> bool {
    n.attribute("Include").is_some_and(|i| i.trim().eq_ignore_ascii_case(id))
}

fn central_item_matches(n: &roxmltree::Node, id: &str) -> bool {
    includes(n, id) || n.attribute("Update").is_some_and(|i| i.trim().eq_ignore_ascii_case(id))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}

/// Sets attribute or child element `name` of `node` to `value`; a `$(Property)` value sets the
/// property defined in the same file instead.
fn set_version_on(doc: &roxmltree::Document, text: &str, node: roxmltree::Node, name: &str, value: &str) -> anyhow::Result<Vec<TextEdit>> {
    if let Some(a) = node.attributes().find(|a| a.name() == name) {
        if let Some(prop) = property_ref(a.value()) {
            return set_property(doc, prop, value);
        }
        return Ok(vec![TextEdit { range: a.range_value(), text: escape(value) }]);
    }
    if let Some(c) = node.children().find(|c| c.has_tag_name(name)) {
        let current = c.text().unwrap_or("").trim();
        if let Some(prop) = property_ref(current) {
            return set_property(doc, prop, value);
        }
        return Ok(vec![element_text_edit(text, c, value)]);
    }
    // No version yet: add the attribute after `Include`.
    let at = node.attributes().find(|a| a.name() == "Include").map(|a| a.range().end).ok_or_else(|| anyhow::anyhow!("item without Include"))?;
    Ok(vec![TextEdit { range: at..at, text: format!(r#" {name}="{}""#, escape(value)) }])
}

fn set_property(doc: &roxmltree::Document, prop: &str, value: &str) -> anyhow::Result<Vec<TextEdit>> {
    let p = doc
        .descendants()
        .rfind(|n| n.has_tag_name(prop) && n.parent_element().is_some_and(|g| g.has_tag_name("PropertyGroup")))
        .ok_or_else(|| anyhow::anyhow!("the version comes from $({prop}), which is not defined in this file"))?;
    Ok(vec![element_text_edit(doc.input_text(), p, value)])
}

/// Replaces the text inside `<X>…</X>` (keeping surrounding blanks).
fn element_text_edit(text: &str, n: roxmltree::Node, value: &str) -> TextEdit {
    match n.first_child().filter(|c| c.is_text()) {
        Some(t) => {
            let r = t.range();
            let raw = &text[r.clone()];
            let lead = raw.len() - raw.trim_start().len();
            let trail = raw.len() - raw.trim_end().len();
            TextEdit { range: r.start + lead..r.end - trail, text: escape(value) }
        }
        None => {
            // `<X></X>` or `<X/>`.
            let r = n.range();
            let src = &text[r.clone()];
            match src.find("></") {
                Some(i) => TextEdit { range: r.start + i + 1..r.start + i + 1, text: escape(value) },
                None => TextEdit { range: r.clone(), text: format!("<{0}>{1}</{0}>", n.tag_name().name(), escape(value)) },
            }
        }
    }
}

/// The line ending the file uses.
fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// The blanks between the start of the line and `pos`, or `None` when other text precedes it.
fn indent_before(text: &str, pos: usize) -> Option<&str> {
    let start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let ws = &text[start..pos];
    ws.chars().all(|c| c == ' ' || c == '\t').then_some(ws)
}

/// `range` widened to whole lines (with the line break) when it is alone on its lines.
fn line_span(text: &str, range: Range<usize>) -> Range<usize> {
    let Some(ws) = indent_before(text, range.start) else { return range };
    let rest = &text[range.end..];
    let tail = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let after = &rest[tail..];
    let end = if let Some(s) = after.strip_prefix("\r\n") {
        range.end + tail + (after.len() - s.len())
    } else if after.starts_with('\n') {
        range.end + tail + 1
    } else if after.is_empty() {
        range.end + tail
    } else {
        return range;
    };
    range.start - ws.len()..end
}

/// Like [`line_span`] for a whole `ItemGroup`, also taking one of two blank lines around it.
fn group_span(text: &str, range: Range<usize>) -> Range<usize> {
    let r = line_span(text, range);
    if r.start == 0 || !text[..r.start].ends_with('\n') {
        return r;
    }
    let before = &text[..r.start];
    let prev_line_start = before[..before.len() - 1].rfind('\n').map_or(0, |i| i + 1);
    let prev_blank = before[prev_line_start..].trim().is_empty();
    let rest = &text[r.end..];
    let next_end = rest.find('\n').map(|i| i + 1);
    match next_end {
        Some(n) if prev_blank && rest[..n].trim().is_empty() => r.start..r.end + n,
        _ => r,
    }
}

/// The blanks of one indentation level, from the first child of the root.
fn indent_unit(doc: &roxmltree::Document, text: &str) -> String {
    doc.root_element()
        .children()
        .find(|n| n.is_element())
        .and_then(|c| indent_before(text, c.range().start))
        .filter(|s| !s.is_empty())
        .unwrap_or("  ")
        .to_string()
}

/// Inserts `elem` (an item of `tag` for `id`) sorted by `Include` in the first unconditional
/// `ItemGroup` holding unconditional items of `tag`, or in a new `ItemGroup` before `</Project>`.
fn insert_item(doc: &roxmltree::Document, text: &str, tag: &str, id: &str, elem: &str) -> TextEdit {
    let nl = newline(text);
    let group = doc.descendants().find(|g| {
        g.has_tag_name("ItemGroup") && g.attribute("Condition").is_none() && g.children().any(|c| c.has_tag_name(tag) && c.attribute("Condition").is_none() && c.attribute("Include").is_some())
    });
    if let Some(g) = group {
        let items: Vec<roxmltree::Node> = g.children().filter(|c| c.has_tag_name(tag) && c.attribute("Include").is_some()).collect();
        let key = id.to_ascii_lowercase();
        if let Some(next) = items.iter().find(|n| n.attribute("Include").unwrap_or("").to_ascii_lowercase() > key) {
            let at = next.range().start;
            return match indent_before(text, at) {
                Some(ind) => TextEdit { range: at..at, text: format!("{elem}{nl}{ind}") },
                None => TextEdit { range: at..at, text: elem.to_string() },
            };
        }
        let last = items.last().expect("group has items");
        let at = last.range().end;
        return match indent_before(text, last.range().start) {
            Some(ind) => TextEdit { range: at..at, text: format!("{nl}{ind}{elem}") },
            None => TextEdit { range: at..at, text: elem.to_string() },
        };
    }
    // A new ItemGroup before `</Project>`.
    let root = doc.root_element().range();
    let close = text[..root.end].rfind("</").unwrap_or(root.end);
    let unit = indent_unit(doc, text);
    match indent_before(text, close) {
        Some(ws) => {
            let line_start = close - ws.len();
            let before = &text[..line_start];
            let prev = before.strip_suffix('\n').map(|b| b.strip_suffix('\r').unwrap_or(b)).unwrap_or(before);
            let prev_blank = prev.rsplit('\n').next().is_some_and(|l| l.trim().is_empty()) && line_start > 0;
            let mut s = format!("{unit}<ItemGroup>{nl}{unit}{unit}{elem}{nl}{unit}</ItemGroup>{nl}");
            if prev_blank {
                s.push_str(nl);
            }
            TextEdit { range: line_start..line_start, text: s }
        }
        None => TextEdit { range: close..close, text: format!("<ItemGroup>{elem}</ItemGroup>") },
    }
}

/// Sets property `name` of a central file to `value`: its element when the file defines it,
/// otherwise a new element at the end of the first unconditional `PropertyGroup` (a new group
/// after `<Project>` when there is none).
pub fn set_central_property(text: &str, name: &str, value: &str) -> anyhow::Result<Vec<TextEdit>> {
    let doc = parse(text)?;
    if let Some(p) = doc.descendants().find(|n| n.has_tag_name(name) && n.parent_element().is_some_and(|g| g.has_tag_name("PropertyGroup") && g.attribute("Condition").is_none())) {
        if p.text().unwrap_or("").trim() == value {
            return Ok(Vec::new());
        }
        return Ok(vec![element_text_edit(text, p, value)]);
    }
    let nl = newline(text);
    let unit = indent_unit(&doc, text);
    let elem = format!("<{name}>{}</{name}>", escape(value));
    if let Some(g) = doc.root_element().children().find(|g| g.has_tag_name("PropertyGroup") && g.attribute("Condition").is_none())
        && let Some(last) = g.children().rfind(|c| c.is_element())
    {
        let at = last.range().end;
        return Ok(vec![match indent_before(text, last.range().start) {
            Some(ind) => TextEdit { range: at..at, text: format!("{nl}{ind}{elem}") },
            None => TextEdit { range: at..at, text: elem },
        }]);
    }
    // A new PropertyGroup right after `<Project>`.
    let root = doc.root_element();
    let open_end = text[root.range()].find('>').map(|i| root.range().start + i + 1).ok_or_else(|| anyhow::anyhow!("malformed <Project>"))?;
    Ok(vec![TextEdit { range: open_end..open_end, text: format!("{nl}{unit}<PropertyGroup>{nl}{unit}{unit}{elem}{nl}{unit}</PropertyGroup>") }])
}

// ---------------------------------------------------------------- migration

/// Something to check after migrating a solution to CPM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationWarning {
    /// A conditional reference: kept with its condition (its version becomes a
    /// `VersionOverride` when it differs from the central one).
    Conditional { file: PathBuf, id: String, condition: String },
    /// A floating (`1.*`) or range (`[1.0,2.0)`) version, kept as written.
    Floating { file: PathBuf, id: String, version: String },
    /// A `$(Property)` version not defined in the same file: kept as `VersionOverride`.
    Property { file: PathBuf, id: String, version: String },
    /// A reference without version: CPM needs a `PackageVersion` for it (NU1010).
    NoVersion { file: PathBuf, id: String },
    /// A project outside the folder of the new central file: left alone.
    Outside { file: PathBuf },
}

/// Projects that reference different versions of a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionConflict {
    pub id: String,
    /// The highest version: the central one.
    pub chosen: String,
    /// (file, version) of the unconditional references with a plain version.
    pub uses: Vec<(PathBuf, String)>,
}

/// One file changed by a migration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub old: String,
    pub new: String,
    /// The edits that turn `old` into `new`.
    pub edits: Vec<TextEdit>,
}

/// Moving every `PackageReference` version of a solution to a new `Directory.Packages.props`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MigrationPlan {
    pub central_file: PathBuf,
    pub central_text: String,
    /// (id, version) of the new `PackageVersion` items, sorted.
    pub versions: Vec<(String, String)>,
    pub changes: Vec<FileChange>,
    pub conflicts: Vec<VersionConflict>,
    pub warnings: Vec<MigrationWarning>,
}

/// The files a migration reads: the projects, then the `Directory.Build.props` and
/// `Directory.Build.targets` above them inside `root`.
pub fn migration_files(root: &Path, projects: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = projects.to_vec();
    for p in projects {
        for name in ["Directory.Build.props", "Directory.Build.targets"] {
            if let Some(f) = p.parent().into_iter().flat_map(Path::ancestors).take_while(|d| d.starts_with(root)).map(|d| d.join(name)).find(|f| f.is_file())
                && !out.contains(&f)
            {
                out.push(f);
            }
        }
    }
    out
}

/// Floating (`1.*`) or range (`[1.0,2.0)`) versions are kept as written.
pub fn is_floating_or_range(v: &str) -> bool {
    let v = v.trim();
    v.contains('*') || v.starts_with('[') || v.starts_with('(')
}

/// Where a reference's version is written.
#[derive(Clone, Debug)]
enum VersionSite {
    Attr(Range<usize>),
    Elem(Range<usize>),
}

#[derive(Clone, Debug)]
struct MigRef {
    file: usize,
    id: String,
    /// As written.
    version: String,
    /// `$(Property)` resolved from the file's own properties.
    resolved: String,
    condition: Option<String>,
    site: VersionSite,
}

/// The condition of an item, its `ItemGroup` or an enclosing `When`.
fn any_condition(n: roxmltree::Node) -> Option<String> {
    n.ancestors().filter(|a| a.is_element()).find_map(|a| a.attribute("Condition")).map(|c| c.trim().to_string()).filter(|c| !c.is_empty())
}

/// Plans the migration of `files` ((path, text), as [`migration_files`] lists them) to a new
/// `Directory.Packages.props` in `dir`. Every plain version moves to it (the highest when
/// projects disagree); `keep_lower` holds the ids (lowercase) whose lower versions stay in their
/// projects as `VersionOverride`.
pub fn plan_migration(dir: &Path, files: &[(PathBuf, String)], keep_lower: &std::collections::BTreeSet<String>) -> anyhow::Result<MigrationPlan> {
    use crate::nuget::compare_versions;
    let central_file = dir.join(CENTRAL_FILE);
    anyhow::ensure!(find_central_file(dir).is_none(), "{} already exists", find_central_file(dir).unwrap_or_default().display());
    let mut warnings = Vec::new();
    let mut refs: Vec<MigRef> = Vec::new();
    for (fi, (path, text)) in files.iter().enumerate() {
        if !path.starts_with(dir) {
            warnings.push(MigrationWarning::Outside { file: path.clone() });
            continue;
        }
        let doc = roxmltree::Document::parse(text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let props = properties(&doc);
        for n in doc.descendants().filter(|n| n.has_tag_name("PackageReference")) {
            let Some(id) = n.attribute("Include").map(str::trim).filter(|s| !s.is_empty()) else { continue };
            let condition = any_condition(n);
            if let Some(c) = &condition {
                warnings.push(MigrationWarning::Conditional { file: path.clone(), id: id.to_string(), condition: c.clone() });
            }
            let site = match (n.attributes().find(|a| a.name() == "Version"), n.children().find(|c| c.has_tag_name("Version"))) {
                (Some(a), _) => (a.value().trim().to_string(), VersionSite::Attr(a.range())),
                (None, Some(c)) => (c.text().unwrap_or("").trim().to_string(), VersionSite::Elem(c.range())),
                (None, None) => {
                    warnings.push(MigrationWarning::NoVersion { file: path.clone(), id: id.to_string() });
                    continue;
                }
            };
            let resolved = resolve(&site.0, &props);
            refs.push(MigRef { file: fi, id: id.to_string(), version: site.0, resolved, condition, site: site.1 });
        }
    }
    // One central version per package.
    let mut ids: Vec<String> = Vec::new();
    for r in &refs {
        if !ids.iter().any(|i| i.eq_ignore_ascii_case(&r.id)) {
            ids.push(r.id.clone());
        }
    }
    ids.sort_by_key(|i| i.to_ascii_lowercase());
    let plain = |r: &MigRef| !r.resolved.contains("$(") && !is_floating_or_range(&r.resolved) && !r.resolved.is_empty();
    let mut versions = Vec::new();
    let mut conflicts = Vec::new();
    // (ref index, keep as VersionOverride)
    let mut decisions: Vec<(usize, bool)> = Vec::new();
    for id in &ids {
        let mine: Vec<usize> = (0..refs.len()).filter(|&i| refs[i].id.eq_ignore_ascii_case(id)).collect();
        let max = |cond: bool| mine.iter().map(|&i| &refs[i]).filter(|r| plain(r) && r.condition.is_none() == cond).map(|r| r.resolved.clone()).max_by(|a, b| compare_versions(a, b));
        let central = max(true)
            .or_else(|| max(false))
            .or_else(|| mine.iter().map(|&i| &refs[i]).find(|r| !r.resolved.contains("$(")).map(|r| r.resolved.clone()));
        let mut distinct: Vec<String> = Vec::new();
        for &i in &mine {
            let r = &refs[i];
            if plain(r) && r.condition.is_none() && !distinct.iter().any(|d| compare_versions(d, &r.resolved).is_eq()) {
                distinct.push(r.resolved.clone());
            }
            if !plain(r) {
                warnings.push(if r.resolved.contains("$(") {
                    MigrationWarning::Property { file: files[r.file].0.clone(), id: r.id.clone(), version: r.version.clone() }
                } else {
                    MigrationWarning::Floating { file: files[r.file].0.clone(), id: r.id.clone(), version: r.version.clone() }
                });
            }
        }
        if let (Some(c), true) = (&central, distinct.len() > 1) {
            let uses = mine.iter().map(|&i| &refs[i]).filter(|r| plain(r) && r.condition.is_none()).map(|r| (files[r.file].0.clone(), r.resolved.clone())).collect();
            conflicts.push(VersionConflict { id: id.clone(), chosen: c.clone(), uses });
        }
        let keep = keep_lower.contains(&id.to_ascii_lowercase());
        for &i in &mine {
            let r = &refs[i];
            let same = central.as_deref().is_some_and(|c| if plain(r) { compare_versions(c, &r.resolved).is_eq() && !is_floating_or_range(c) } else { c == r.resolved });
            let over = !same && (r.condition.is_some() || !plain(r) || keep);
            decisions.push((i, over));
        }
        if let Some(c) = central {
            versions.push((id.clone(), c));
        }
    }
    // Edits per file.
    let mut changes = Vec::new();
    for (fi, (path, text)) in files.iter().enumerate() {
        let mut edits = Vec::new();
        let rename = |at: usize| TextEdit { range: at..at + "Version".len(), text: "VersionOverride".into() };
        for &(i, over) in decisions.iter().filter(|(i, _)| refs[*i].file == fi) {
            match (&refs[i].site, over) {
                (VersionSite::Attr(r), true) => edits.push(rename(r.start)),
                (VersionSite::Attr(r), false) => {
                    // The attribute and the blanks before it.
                    let start = text[..r.start].trim_end_matches([' ', '\t', '\r', '\n']).len();
                    edits.push(TextEdit { range: start..r.end, text: String::new() });
                }
                (VersionSite::Elem(r), true) => {
                    let close = text[r.clone()].rfind("</").map_or(r.end, |c| r.start + c);
                    edits.push(rename(r.start + 1));
                    edits.push(rename(close + 2));
                }
                (VersionSite::Elem(r), false) => edits.push(TextEdit { range: line_span(text, r.clone()), text: String::new() }),
            }
        }
        if !edits.is_empty() {
            changes.push(FileChange { path: path.clone(), old: text.clone(), new: apply_edits(text, &edits), edits });
        }
    }
    let nl = files.iter().find(|(_, t)| t.contains("\r\n")).map_or("\n", |_| "\r\n");
    let floating = versions.iter().any(|(_, v)| v.contains('*')) || refs.iter().any(|r| r.resolved.contains('*'));
    let mut central_text = format!("<Project>{nl}  <PropertyGroup>{nl}    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>{nl}");
    if floating {
        central_text.push_str(&format!("    <CentralPackageFloatingVersionsEnabled>true</CentralPackageFloatingVersionsEnabled>{nl}"));
    }
    central_text.push_str(&format!("  </PropertyGroup>{nl}"));
    if !versions.is_empty() {
        central_text.push_str(&format!("  <ItemGroup>{nl}"));
        for (id, v) in &versions {
            central_text.push_str(&format!("    <PackageVersion Include=\"{}\" Version=\"{}\" />{nl}", escape(id), escape(v)));
        }
        central_text.push_str(&format!("  </ItemGroup>{nl}"));
    }
    central_text.push_str(&format!("</Project>{nl}"));
    Ok(MigrationPlan { central_file, central_text, versions, changes, conflicts, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTRAL: &str = r#"<Project>
  <PropertyGroup>
    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>
    <SerilogVersion>4.0.0</SerilogVersion>
  </PropertyGroup>
  <!-- Shared versions -->
  <ItemGroup>
    <PackageVersion Include="Dapper" Version="2.1.35" />
    <PackageVersion Include="Newtonsoft.Json" Version="13.0.3" /> <!-- keep -->
    <PackageVersion Include="Serilog" Version="$(SerilogVersion)" />
  </ItemGroup>
  <ItemGroup Condition="'$(TargetFramework)' == 'net8.0'">
    <PackageVersion Include="Microsoft.EntityFrameworkCore" Version="8.0.0" />
  </ItemGroup>
  <ItemGroup>
    <PackageVersion Include="Microsoft.EntityFrameworkCore" Version="9.0.0" Condition="'$(TargetFramework)' == 'net9.0'" />
    <GlobalPackageReference Include="Nerdbank.GitVersioning" Version="3.6.0" />
  </ItemGroup>
</Project>
"#;

    fn edit(text: &str, edits: anyhow::Result<Vec<TextEdit>>) -> String {
        apply_edits(text, &edits.unwrap())
    }

    #[test]
    fn parses_central_items_conditions_and_properties() {
        let f = CentralFile::parse(Path::new("/s/Directory.Packages.props"), CENTRAL).unwrap();
        assert_eq!(f.manage, Some(true));
        let serilog = f.unconditional("serilog").unwrap();
        assert_eq!((serilog.version.as_str(), serilog.resolved.as_str()), ("$(SerilogVersion)", "4.0.0"));
        let ef: Vec<_> = f.versions("Microsoft.EntityFrameworkCore").map(|i| (i.resolved.as_str(), i.condition.as_deref())).collect();
        assert_eq!(ef, [("8.0.0", Some("'$(TargetFramework)' == 'net8.0'")), ("9.0.0", Some("'$(TargetFramework)' == 'net9.0'"))]);
        assert!(f.unconditional("Microsoft.EntityFrameworkCore").is_none());
        let g = f.items.iter().find(|i| i.global).unwrap();
        assert_eq!((g.id.as_str(), g.line), ("Nerdbank.GitVersioning", 16));
    }

    #[test]
    fn updating_a_version_touches_only_its_value() {
        let out = edit(CENTRAL, set_central_version(CENTRAL, "newtonsoft.json", None, "13.0.4"));
        assert_eq!(out, CENTRAL.replace(r#""Newtonsoft.Json" Version="13.0.3""#, r#""Newtonsoft.Json" Version="13.0.4""#));
        assert!(out.contains("<!-- keep -->") && out.contains("<!-- Shared versions -->"));
    }

    #[test]
    fn updating_a_property_version_changes_the_property() {
        let out = edit(CENTRAL, set_central_version(CENTRAL, "Serilog", None, "4.1.0"));
        assert!(out.contains("<SerilogVersion>4.1.0</SerilogVersion>"));
        assert!(out.contains(r#"Version="$(SerilogVersion)""#));
    }

    #[test]
    fn updating_a_conditional_version_needs_its_condition() {
        let net9 = "'$(TargetFramework)' == 'net9.0'";
        let out = edit(CENTRAL, set_central_version(CENTRAL, "Microsoft.EntityFrameworkCore", Some(net9), "9.0.1"));
        assert!(out.contains(r#"Version="9.0.1" Condition="'$(TargetFramework)' == 'net9.0'""#));
        assert!(out.contains(r#"Version="8.0.0""#));
        // Two conditional versions and no unconditional one: ambiguous.
        assert!(set_central_version(CENTRAL, "Microsoft.EntityFrameworkCore", None, "1.0.0").is_err());
        // Global references update in place.
        let out = edit(CENTRAL, set_central_version(CENTRAL, "Nerdbank.GitVersioning", None, "3.7.0"));
        assert!(out.contains(r#"<GlobalPackageReference Include="Nerdbank.GitVersioning" Version="3.7.0" />"#));
    }

    #[test]
    fn adds_versions_sorted_with_the_group_indentation() {
        let out = edit(CENTRAL, add_central_version(CENTRAL, "Humanizer.Core", "2.14.1"));
        assert!(out.contains("    <PackageVersion Include=\"Dapper\" Version=\"2.1.35\" />\n    <PackageVersion Include=\"Humanizer.Core\" Version=\"2.14.1\" />\n    <PackageVersion Include=\"Newtonsoft.Json\""));
        let first = edit(CENTRAL, add_central_version(CENTRAL, "Autofac", "8.0.0"));
        assert!(first.contains("<ItemGroup>\n    <PackageVersion Include=\"Autofac\" Version=\"8.0.0\" />\n    <PackageVersion Include=\"Dapper\""));
        let last = edit(CENTRAL, add_central_version(CENTRAL, "Xunit", "2.9.0"));
        assert!(last.contains("Version=\"$(SerilogVersion)\" />\n    <PackageVersion Include=\"Xunit\" Version=\"2.9.0\" />\n  </ItemGroup>"));
        assert!(add_central_version(CENTRAL, "dapper", "1.0.0").is_err());
        // Tabs and CRLF are kept.
        let tabs = "<Project>\r\n\t<ItemGroup>\r\n\t\t<PackageVersion Include=\"B\" Version=\"1.0.0\" />\r\n\t</ItemGroup>\r\n</Project>\r\n";
        assert_eq!(edit(tabs, add_central_version(tabs, "A", "2.0.0")), tabs.replace("\t\t<PackageVersion Include=\"B\"", "\t\t<PackageVersion Include=\"A\" Version=\"2.0.0\" />\r\n\t\t<PackageVersion Include=\"B\""));
    }

    #[test]
    fn adds_a_group_when_the_file_has_none() {
        let empty = "<Project>\n  <PropertyGroup>\n    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>\n  </PropertyGroup>\n</Project>\n";
        let out = edit(empty, add_central_version(empty, "Dapper", "2.1.35"));
        assert_eq!(out, "<Project>\n  <PropertyGroup>\n    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageVersion Include=\"Dapper\" Version=\"2.1.35\" />\n  </ItemGroup>\n</Project>\n");
        let one_line = r#"<Project></Project>"#;
        assert_eq!(edit(one_line, add_central_version(one_line, "A", "1.0.0")), r#"<Project><ItemGroup><PackageVersion Include="A" Version="1.0.0" /></ItemGroup></Project>"#);
    }

    #[test]
    fn removes_versions_with_their_lines() {
        let out = edit(CENTRAL, remove_central_version(CENTRAL, "Dapper"));
        assert_eq!(out, CENTRAL.replace("    <PackageVersion Include=\"Dapper\" Version=\"2.1.35\" />\n", ""));
        // A trailing comment keeps the line.
        let out = edit(CENTRAL, remove_central_version(CENTRAL, "Newtonsoft.Json"));
        assert!(out.contains("\n     <!-- keep -->\n"));
        assert!(!out.contains("Newtonsoft"));
        assert_eq!(edit(CENTRAL, remove_central_version(CENTRAL, "Microsoft.EntityFrameworkCore")).matches("EntityFrameworkCore").count(), 0);
    }

    const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">

  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>

  <ItemGroup>
    <PackageReference Include="Dapper" />
    <PackageReference Include="Newtonsoft.Json" VersionOverride="13.0.1" />
    <PackageReference Include="Serilog" Version="4.0.0" />
  </ItemGroup>

  <ItemGroup>
    <ProjectReference Include="..\Core\Core.csproj" />
  </ItemGroup>

</Project>
"#;

    #[test]
    fn edits_project_references() {
        let out = edit(PROJECT, add_reference(PROJECT, "Humanizer.Core"));
        assert!(out.contains("<PackageReference Include=\"Dapper\" />\n    <PackageReference Include=\"Humanizer.Core\" />\n    <PackageReference Include=\"Newtonsoft.Json\""));
        assert!(add_reference(PROJECT, "dapper").is_err());

        let out = edit(PROJECT, remove_version_override(PROJECT, "Newtonsoft.Json"));
        assert!(out.contains("<PackageReference Include=\"Newtonsoft.Json\" />\n"));

        let (edits, v) = remove_reference_version(PROJECT, "Serilog").unwrap();
        assert_eq!(v, "4.0.0");
        assert!(apply_edits(PROJECT, &edits).contains("<PackageReference Include=\"Serilog\" />\n"));

        let out = edit(PROJECT, set_reference_metadata(PROJECT, "Dapper", "VersionOverride", "2.0.0"));
        assert!(out.contains(r#"<PackageReference Include="Dapper" VersionOverride="2.0.0" />"#));

        let out = edit(PROJECT, remove_reference(PROJECT, "Dapper"));
        assert_eq!(out, PROJECT.replace("    <PackageReference Include=\"Dapper\" />\n", ""));
    }

    #[test]
    fn removing_the_last_reference_drops_its_group() {
        let one = "<Project Sdk=\"Microsoft.NET.Sdk\">\n\n  <PropertyGroup>\n    <TargetFramework>net8.0</TargetFramework>\n  </PropertyGroup>\n\n  <ItemGroup>\n    <PackageReference Include=\"Dapper\" />\n  </ItemGroup>\n\n</Project>\n";
        let out = edit(one, remove_reference(one, "Dapper"));
        assert_eq!(out, "<Project Sdk=\"Microsoft.NET.Sdk\">\n\n  <PropertyGroup>\n    <TargetFramework>net8.0</TargetFramework>\n  </PropertyGroup>\n\n</Project>\n");
        // And adding it back makes a new group with the same layout.
        assert_eq!(edit(&out, add_reference(&out, "Dapper")), one);
    }

    #[test]
    fn element_versions_are_edited_too() {
        let t = "<Project>\n  <ItemGroup>\n    <PackageVersion Include=\"A\">\n      <Version>1.0.0</Version>\n    </PackageVersion>\n  </ItemGroup>\n</Project>\n";
        assert!(edit(t, set_central_version(t, "A", None, "2.0.0")).contains("<Version>2.0.0</Version>"));
        let p = "<Project>\n  <ItemGroup>\n    <PackageReference Include=\"A\">\n      <VersionOverride>1.0.0</VersionOverride>\n    </PackageReference>\n  </ItemGroup>\n</Project>\n";
        assert_eq!(edit(p, remove_version_override(p, "A")), "<Project>\n  <ItemGroup>\n    <PackageReference Include=\"A\">\n    </PackageReference>\n  </ItemGroup>\n</Project>\n");
    }

    fn write(dir: &Path, rel: &str, text: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn static_detection_follows_build_props_and_nested_files() {
        let tmp = tempfile::Builder::new().prefix("dotcraft-cpm").tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        write(&root, "Directory.Build.props", "<Project><PropertyGroup><ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally></PropertyGroup></Project>");
        let parent = write(&root, CENTRAL_FILE, r#"<Project><ItemGroup><PackageVersion Include="Dapper" Version="2.1.35" /><PackageVersion Include="Serilog" Version="3.0.0" /><GlobalPackageReference Include="G" Version="1.0.0" /></ItemGroup></Project>"#);
        let nested = write(
            &root,
            "src/Directory.Packages.props",
            r#"<Project><Import Project="$([MSBuild]::GetPathOfFileAbove(Directory.Packages.props, $(MSBuildThisFileDirectory)..))" /><ItemGroup><PackageVersion Include="Serilog" Version="4.0.0" /></ItemGroup></Project>"#,
        );
        let app = write(&root, "src/App/App.csproj", r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Dapper" /><PackageReference Include="Serilog" VersionOverride="4.1.0" /><PackageReference Include="Polly" Version="8.0.0" /><PackageReference Include="Missing" /></ItemGroup></Project>"#);
        let off = write(&root, "src/Off/Off.csproj", r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><ManagePackageVersionsCentrally>false</ManagePackageVersionsCentrally></PropertyGroup><ItemGroup><PackageReference Include="Dapper" Version="1.0.0" /></ItemGroup></Project>"#);

        let cpm = detect_static(&app);
        assert!(cpm.enabled && !cpm.evaluated);
        assert_eq!(cpm.central_file.as_deref(), Some(nested.as_path()));
        assert_eq!(cpm.central_version("dapper").map(|v| (v.version.as_str(), v.file.as_path())), Some(("2.1.35", parent.as_path())));
        assert_eq!(cpm.central_version("Serilog").map(|v| (v.version.as_str(), v.file.as_path())), Some(("4.0.0", nested.as_path())));
        assert_eq!(cpm.effective_version("Serilog").as_deref(), Some("4.1.0"));
        assert_eq!(cpm.effective_version("G").as_deref(), Some("1.0.0"));
        assert!(cpm.is_central("Dapper") && !cpm.is_central("Polly"));
        assert!(!detect_static(&off).enabled);

        let sol = SolutionCpm::new(vec![cpm, detect_static(&off)]);
        assert_eq!(sol.files.len(), 2);
        assert_eq!(sol.chain(&app).iter().map(|f| f.path.clone()).collect::<Vec<_>>(), [nested.clone(), parent.clone()]);
        assert_eq!(sol.file_for(&app, "Dapper"), Some(parent.clone()));
        assert_eq!(sol.file_for(&app, "Humanizer"), Some(nested.clone()));
        let issues = sol.issues();
        assert!(issues.contains(&CpmIssue::VersionOnReference { project: app.clone(), id: "Polly".into(), version: "8.0.0".into() }));
        assert!(issues.contains(&CpmIssue::MissingVersion { project: app.clone(), id: "Missing".into() }));
        // Serilog of the parent file is shadowed but still referenced: not reported.
        assert!(!issues.iter().any(|i| matches!(i, CpmIssue::UnusedVersion { .. })));
        assert_eq!(sol.users("dapper").len(), 1);
    }

    #[test]
    fn unused_versions_are_reported_unless_pinning_transitives() {
        let tmp = tempfile::Builder::new().prefix("dotcraft-cpm").tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        let central = write(&root, CENTRAL_FILE, r#"<Project><PropertyGroup><ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally></PropertyGroup><ItemGroup><PackageVersion Include="Dapper" Version="2.1.35" /><PackageVersion Include="Unused" Version="1.0.0" /></ItemGroup></Project>"#);
        let app = write(&root, "App/App.csproj", r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Dapper" /></ItemGroup></Project>"#);
        let sol = SolutionCpm::new(vec![detect_static(&app)]);
        assert_eq!(sol.issues(), [CpmIssue::UnusedVersion { file: central.clone(), id: "Unused".into() }]);
        assert!(sol.is_used(&central, "Dapper") && !sol.is_used(&central, "Unused"));
        let mut pinned = detect_static(&app);
        pinned.transitive_pinning = true;
        assert!(SolutionCpm::new(vec![pinned]).issues().is_empty());
    }

    #[test]
    fn parses_an_msbuild_evaluation() {
        let json = r#"{
  "Properties": { "ManagePackageVersionsCentrally": "true", "DirectoryPackagesPropsPath": "", "CentralPackageTransitivePinningEnabled": "", "CentralPackageVersionOverrideEnabled": "" },
  "Items": {
    "PackageReference": [
      { "Identity": "Newtonsoft.Json", "VersionOverride": "13.0.1", "DefiningProjectFullPath": "/s/A/A.csproj" },
      { "Identity": "Analyzers", "DefiningProjectFullPath": "/s/Directory.Build.props" },
      { "Identity": "Humanizer.Core", "Version": "", "DefiningProjectFullPath": "/sdk/NuGet.targets" }
    ],
    "PackageVersion": [
      { "Identity": "Newtonsoft.Json", "Version": "13.0.0", "DefiningProjectFullPath": "/s/Directory.Packages.props" },
      { "Identity": "Newtonsoft.Json", "Version": "13.0.3", "DefiningProjectFullPath": "/s/src/Directory.Packages.props" },
      { "Identity": "Humanizer.Core", "Version": "2.14.1", "DefiningProjectFullPath": "/sdk/NuGet.targets" }
    ],
    "GlobalPackageReference": [ { "Identity": "Humanizer.Core", "Version": "2.14.1", "DefiningProjectFullPath": "/s/Directory.Packages.props" } ]
  }
}"#;
        let cpm = parse_evaluation(Path::new("/s/A/A.csproj"), json).unwrap();
        assert!(cpm.enabled && cpm.evaluated && cpm.override_enabled && cpm.central_file.is_none());
        assert_eq!(cpm.references.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["Newtonsoft.Json", "Analyzers"]);
        assert_eq!(cpm.references[1].defined_in, Path::new("/s/Directory.Build.props"));
        assert_eq!(cpm.central_version("Newtonsoft.Json").map(|v| (v.version.as_str(), v.file.as_path())), Some(("13.0.3", Path::new("/s/src/Directory.Packages.props"))));
        assert!(cpm.central_version("Humanizer.Core").is_none());
        assert_eq!(cpm.effective_version("Newtonsoft.Json").as_deref(), Some("13.0.1"));
        assert_eq!(cpm.effective_version("Humanizer.Core").as_deref(), Some("2.14.1"));
    }

    #[test]
    fn cache_is_dropped_when_a_file_changes() {
        let tmp = tempfile::Builder::new().prefix("dotcraft-cpm").tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        let central = write(&root, CENTRAL_FILE, r#"<Project><ItemGroup><PackageVersion Include="Dapper" Version="2.1.35" /></ItemGroup></Project>"#);
        let app = write(&root, "App/App.csproj", r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Dapper" /></ItemGroup></Project>"#);
        let mut cache = CpmCache::default();
        cache.insert(detect_static(&app));
        assert!(cache.get(&app).is_some());
        let f = std::fs::File::options().write(true).open(&central).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        assert!(cache.get(&app).is_none());
    }

    #[test]
    fn sets_central_properties() {
        let out = edit(CENTRAL, set_central_property(CENTRAL, "CentralPackageTransitivePinningEnabled", "true"));
        assert!(out.contains("    <SerilogVersion>4.0.0</SerilogVersion>\n    <CentralPackageTransitivePinningEnabled>true</CentralPackageTransitivePinningEnabled>\n  </PropertyGroup>"), "{out}");
        assert!(set_central_property(&out, "CentralPackageTransitivePinningEnabled", "true").unwrap().is_empty());
        let off = out.replace(">true</CentralPackageTransitivePinningEnabled>", ">false</CentralPackageTransitivePinningEnabled>");
        assert_eq!(edit(&off, set_central_property(&off, "CentralPackageTransitivePinningEnabled", "true")), out);
        let bare = "<Project>\n  <ItemGroup>\n    <PackageVersion Include=\"A\" Version=\"1.0.0\" />\n  </ItemGroup>\n</Project>\n";
        assert_eq!(
            edit(bare, set_central_property(bare, "ManagePackageVersionsCentrally", "true")),
            "<Project>\n  <PropertyGroup>\n    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageVersion Include=\"A\" Version=\"1.0.0\" />\n  </ItemGroup>\n</Project>\n"
        );
    }

    fn migrate(root: &Path, files: &[(&str, &str)], keep: &[&str]) -> MigrationPlan {
        let files: Vec<(PathBuf, String)> = files.iter().map(|(p, t)| (root.join(p), t.to_string())).collect();
        plan_migration(root, &files, &keep.iter().map(|s| s.to_ascii_lowercase()).collect()).unwrap()
    }

    fn changed<'a>(plan: &'a MigrationPlan, root: &Path, rel: &str) -> &'a str {
        &plan.changes.iter().find(|c| c.path == root.join(rel)).unwrap_or_else(|| panic!("{rel} unchanged")).new
    }

    const APP: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.0.1\" />\n    <PackageReference Include=\"Serilog\">\n      <Version>4.0.0</Version>\n    </PackageReference>\n  </ItemGroup>\n</Project>\n";
    const LIB: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <PackageReference Include=\"newtonsoft.json\" Version=\"13.0.3\" />\n    <PackageReference Include=\"Dapper\" Version=\"2.1.35\" PrivateAssets=\"all\" />\n  </ItemGroup>\n</Project>\n";

    #[test]
    fn migration_moves_versions_and_picks_the_highest() {
        let root = Path::new("/nonexistent/sol");
        let plan = migrate(root, &[("App/App.csproj", APP), ("Lib/Lib.csproj", LIB)], &[]);
        assert_eq!(plan.central_file, root.join(CENTRAL_FILE));
        assert_eq!(plan.versions, [("Dapper".to_string(), "2.1.35".to_string()), ("Newtonsoft.Json".into(), "13.0.3".into()), ("Serilog".into(), "4.0.0".into())]);
        assert_eq!(
            plan.central_text,
            "<Project>\n  <PropertyGroup>\n    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageVersion Include=\"Dapper\" Version=\"2.1.35\" />\n    <PackageVersion Include=\"Newtonsoft.Json\" Version=\"13.0.3\" />\n    <PackageVersion Include=\"Serilog\" Version=\"4.0.0\" />\n  </ItemGroup>\n</Project>\n"
        );
        assert_eq!(plan.conflicts, [VersionConflict { id: "Newtonsoft.Json".into(), chosen: "13.0.3".into(), uses: vec![(root.join("App/App.csproj"), "13.0.1".into()), (root.join("Lib/Lib.csproj"), "13.0.3".into())] }]);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        assert_eq!(
            changed(&plan, root, "App/App.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" />\n    <PackageReference Include=\"Serilog\">\n    </PackageReference>\n  </ItemGroup>\n</Project>\n"
        );
        assert!(changed(&plan, root, "Lib/Lib.csproj").contains("<PackageReference Include=\"Dapper\" PrivateAssets=\"all\" />"));
        // Keeping the lower version: it becomes an override.
        let plan = migrate(root, &[("App/App.csproj", APP), ("Lib/Lib.csproj", LIB)], &["Newtonsoft.Json"]);
        assert!(changed(&plan, root, "App/App.csproj").contains("<PackageReference Include=\"Newtonsoft.Json\" VersionOverride=\"13.0.1\" />"));
        assert!(changed(&plan, root, "Lib/Lib.csproj").contains("<PackageReference Include=\"newtonsoft.json\" />"));
    }

    #[test]
    fn migration_handles_build_props_conditions_floating_versions_and_properties() {
        let root = Path::new("/nonexistent/sol");
        let build = "<Project>\n  <ItemGroup>\n    <PackageReference Include=\"StyleCop.Analyzers\" Version=\"1.1.118\" PrivateAssets=\"all\" />\n  </ItemGroup>\n</Project>\n";
        let app = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <EfVersion>8.0.0</EfVersion>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"EF\" Version=\"$(EfVersion)\" />\n    <PackageReference Include=\"Polly\" Version=\"8.*\" />\n    <PackageReference Include=\"Range\" Version=\"[1.0,2.0)\" />\n    <PackageReference Include=\"Ext\" Version=\"$(ExtVersion)\" />\n    <PackageReference Include=\"Implicit\" />\n  </ItemGroup>\n  <ItemGroup Condition=\"'$(TargetFramework)' == 'net48'\">\n    <PackageReference Include=\"EF\" Version=\"6.0.0\" />\n  </ItemGroup>\n</Project>\n";
        let other = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <PackageReference Include=\"Polly\" Version=\"8.4.0\" />\n  </ItemGroup>\n</Project>\n";
        let plan = migrate(root, &[("App/App.csproj", app), ("Other/Other.csproj", other), ("Directory.Build.props", build)], &[]);
        let v = |id: &str| plan.versions.iter().find(|(i, _)| i == id).map(|(_, v)| v.as_str());
        assert_eq!((v("StyleCop.Analyzers"), v("EF"), v("Polly"), v("Range"), v("Ext")), (Some("1.1.118"), Some("8.0.0"), Some("8.4.0"), Some("[1.0,2.0)"), None));
        // Directory.Build.props loses its version too.
        assert!(changed(&plan, root, "Directory.Build.props").contains("<PackageReference Include=\"StyleCop.Analyzers\" PrivateAssets=\"all\" />"));
        let a = changed(&plan, root, "App/App.csproj");
        assert!(a.contains("<PackageReference Include=\"EF\" />"), "{a}");
        assert!(a.contains("<PackageReference Include=\"Polly\" VersionOverride=\"8.*\" />"), "{a}");
        assert!(a.contains("<PackageReference Include=\"Range\" />"), "{a}");
        assert!(a.contains("<PackageReference Include=\"Ext\" VersionOverride=\"$(ExtVersion)\" />"), "{a}");
        assert!(a.contains("<ItemGroup Condition=\"'$(TargetFramework)' == 'net48'\">\n    <PackageReference Include=\"EF\" VersionOverride=\"6.0.0\" />"), "{a}");
        // A floating override needs floating versions on.
        assert!(plan.central_text.contains("<CentralPackageFloatingVersionsEnabled>true</CentralPackageFloatingVersionsEnabled>"));
        let app_path = root.join("App/App.csproj");
        for w in [
            MigrationWarning::Conditional { file: app_path.clone(), id: "EF".into(), condition: "'$(TargetFramework)' == 'net48'".into() },
            MigrationWarning::Floating { file: app_path.clone(), id: "Polly".into(), version: "8.*".into() },
            MigrationWarning::Floating { file: app_path.clone(), id: "Range".into(), version: "[1.0,2.0)".into() },
            MigrationWarning::Property { file: app_path.clone(), id: "Ext".into(), version: "$(ExtVersion)".into() },
            MigrationWarning::NoVersion { file: app_path.clone(), id: "Implicit".into() },
        ] {
            assert!(plan.warnings.contains(&w), "{w:?} not in {:?}", plan.warnings);
        }
        // Conditional or floating references are not conflicts.
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
        // The edits rebuild each new text.
        for c in &plan.changes {
            assert_eq!(apply_edits(&c.old, &c.edits), c.new);
            roxmltree::Document::parse(&c.new).unwrap();
        }
    }

    #[test]
    fn migration_keeps_line_endings_and_skips_outside_projects() {
        let root = Path::new("/nonexistent/sol");
        let crlf = APP.replace('\n', "\r\n");
        let plan = migrate(root, &[("App/App.csproj", &crlf), ("/elsewhere/E.csproj", LIB)], &[]);
        assert!(plan.central_text.contains("\r\n") && !plan.central_text.replace("\r\n", "").contains('\n'));
        assert_eq!(changed(&plan, root, "App/App.csproj"), crlf.replace(" Version=\"13.0.1\"", "").replace("      <Version>4.0.0</Version>\r\n", ""));
        assert!(plan.warnings.iter().any(|w| matches!(w, MigrationWarning::Outside { .. })));
        assert_eq!(plan.versions.len(), 2);
    }

    #[test]
    fn migration_files_include_build_files_inside_the_root() {
        let tmp = tempfile::Builder::new().prefix("dotcraft-cpm").tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        write(&root, "Directory.Build.props", "<Project />");
        let sol = root.join("sol");
        write(&sol, "src/Directory.Build.targets", "<Project />");
        let app = write(&sol, "src/App/App.csproj", "<Project />");
        assert_eq!(migration_files(&sol, std::slice::from_ref(&app)), [app.clone(), sol.join("src/Directory.Build.targets")]);
        assert_eq!(migration_files(&root, std::slice::from_ref(&app)), [app.clone(), root.join("Directory.Build.props"), sol.join("src/Directory.Build.targets")]);
        // A solution that already has a central file is not migrated.
        write(&sol, CENTRAL_FILE, "<Project />");
        assert!(plan_migration(&sol, &[], &Default::default()).is_err());
    }
}
