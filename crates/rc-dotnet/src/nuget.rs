//! NuGet: a small v3 client (service index, search, versions from the flat container) over
//! rustls, plus adding, updating and removing package references through the `dotnet` CLI.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::diagnostics::{self, Diagnostic};
use crate::sdk::Dotnet;

/// The nuget.org v3 service index.
pub const NUGET_ORG: &str = "https://api.nuget.org/v3/index.json";

/// A resource of the service index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceResource {
    pub id: String,
    /// `SearchQueryService/3.5.0`, `PackageBaseAddress/3.0.0`…
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceIndex {
    pub resources: Vec<ServiceResource>,
}

impl ServiceIndex {
    /// URL of the first resource whose type is `kind` or a version of it (`kind/x.y.z`).
    pub fn find(&self, kind: &str) -> Option<&str> {
        self.resources.iter().find(|r| r.kind == kind || r.kind.strip_prefix(kind).is_some_and(|rest| rest.starts_with('/'))).map(|r| r.id.as_str())
    }
}

/// A package from search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageSummary {
    pub id: String,
    /// Latest version (prerelease only when asked for).
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub total_downloads: u64,
    pub verified: bool,
    pub icon_url: Option<String>,
    pub project_url: Option<String>,
    pub tags: Vec<String>,
    /// Versions listed by search, oldest first.
    pub versions: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub total_hits: u64,
    pub packages: Vec<PackageSummary>,
}

/// Client for one v3 source. Cheap to clone; caches the service index.
#[derive(Clone)]
pub struct NugetClient {
    http: reqwest::Client,
    index_url: String,
    index: std::sync::Arc<OnceCell<ServiceIndex>>,
    /// Credentials for a private feed, sent to its own host only (see `nuget_auth`).
    auth: Option<crate::nuget_auth::Credential>,
}

impl NugetClient {
    /// A client for nuget.org.
    pub fn new() -> anyhow::Result<Self> {
        Self::with_source(NUGET_ORG)
    }

    pub fn with_source(index_url: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("DotCraft/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            // Redirects are followed by `nuget_auth::get`, which drops credentials on other hosts.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(NugetClient { http, index_url: index_url.to_string(), index: Default::default(), auth: None })
    }

    /// A client for a private feed with its credentials (see `nuget_auth`).
    pub fn with_credentials(index_url: &str, auth: Option<crate::nuget_auth::Credential>) -> anyhow::Result<Self> {
        let mut c = Self::with_source(index_url)?;
        c.auth = auth;
        Ok(c)
    }

    pub fn source(&self) -> &str {
        &self.index_url
    }

    /// The service index (fetched once).
    pub async fn service_index(&self) -> anyhow::Result<&ServiceIndex> {
        self.index
            .get_or_try_init(|| async {
                let text = self.get_text(&self.index_url).await?;
                parse_service_index(&text)
            })
            .await
    }

    /// Searches packages (`SearchQueryService`), `skip`/`take` for paging.
    pub async fn search(&self, query: &str, prerelease: bool, skip: usize, take: usize) -> anyhow::Result<SearchResults> {
        let base = self.service_index().await?.find("SearchQueryService").ok_or_else(|| anyhow::anyhow!("the source has no SearchQueryService"))?.to_string();
        let url = reqwest::Url::parse_with_params(
            &base,
            &[("q", query), ("prerelease", if prerelease { "true" } else { "false" }), ("skip", &skip.to_string()), ("take", &take.to_string()), ("semVerLevel", "2.0.0")],
        )?;
        parse_search(&self.get_text(url.as_str()).await?)
    }

    /// Every published version of a package, oldest first (`PackageBaseAddress`).
    pub async fn versions(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let base = self.service_index().await?.find("PackageBaseAddress").ok_or_else(|| anyhow::anyhow!("the source has no PackageBaseAddress"))?.to_string();
        let url = format!("{}/{}/index.json", base.trim_end_matches('/'), id.to_lowercase());
        parse_versions(&self.get_text(&url).await?)
    }

    async fn get_text(&self, url: &str) -> anyhow::Result<String> {
        let resp = crate::nuget_auth::get(&self.http, &self.index_url, self.auth.as_ref(), url).await?;
        let status = resp.status();
        anyhow::ensure!(status.is_success(), "GET {url}: {status}");
        Ok(resp.text().await?)
    }
}

/// Parses `index.json` of a v3 source.
pub fn parse_service_index(json: &str) -> anyhow::Result<ServiceIndex> {
    #[derive(Deserialize)]
    struct Index {
        resources: Vec<Res>,
    }
    #[derive(Deserialize)]
    struct Res {
        #[serde(rename = "@id")]
        id: String,
        #[serde(rename = "@type")]
        kind: serde_json::Value,
    }
    let idx: Index = serde_json::from_str(json)?;
    let mut resources = Vec::new();
    for r in idx.resources {
        // `@type` is a string or an array of strings.
        let kinds: Vec<String> = match r.kind {
            serde_json::Value::String(s) => vec![s],
            serde_json::Value::Array(a) => a.into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        for kind in kinds {
            resources.push(ServiceResource { id: r.id.clone(), kind });
        }
    }
    Ok(ServiceIndex { resources })
}

/// Parses a search response.
pub fn parse_search(json: &str) -> anyhow::Result<SearchResults> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let strings = |v: Option<&serde_json::Value>| -> Vec<String> {
        match v {
            Some(serde_json::Value::String(s)) => s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
            Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        }
    };
    let s = |p: &serde_json::Value, k: &str| p.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let packages = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .map(|p| PackageSummary {
                    id: s(p, "id"),
                    version: s(p, "version"),
                    description: s(p, "description"),
                    authors: strings(p.get("authors")),
                    total_downloads: p.get("totalDownloads").and_then(|x| x.as_u64()).unwrap_or(0),
                    verified: p.get("verified").and_then(|x| x.as_bool()).unwrap_or(false),
                    icon_url: p.get("iconUrl").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string),
                    project_url: p.get("projectUrl").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string),
                    tags: strings(p.get("tags")),
                    versions: p.get("versions").and_then(|x| x.as_array()).map(|vs| vs.iter().filter_map(|x| x.get("version").and_then(|v| v.as_str()).map(str::to_string)).collect()).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(SearchResults { total_hits: v.get("totalHits").and_then(|x| x.as_u64()).unwrap_or(0), packages })
}

/// Parses the flat container's `{id}/index.json` (`{"versions": [...]}`).
pub fn parse_versions(json: &str) -> anyhow::Result<Vec<String>> {
    #[derive(Deserialize)]
    struct V {
        versions: Vec<String>,
    }
    Ok(serde_json::from_str::<V>(json)?.versions)
}

/// Output of a `dotnet` package command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    /// stdout and stderr together.
    pub output: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// `dotnet add <project> package <id> [--version v] [--prerelease]`; also updates an
/// existing reference to `version`.
pub async fn add_package(dotnet: &Dotnet, project: &Path, id: &str, version: Option<&str>, prerelease: bool) -> anyhow::Result<CommandOutput> {
    validate_package_command(project, id)?;
    if let Some(version) = version { validate_package_version(version)?; }
    let mut args = vec!["add".to_string(), project.to_string_lossy().into_owned(), "package".into(), id.to_string()];
    if let Some(v) = version {
        args.extend(["--version".into(), v.to_string()]);
    } else if prerelease {
        args.push("--prerelease".into());
    }
    run_cli(dotnet, project, &args).await
}

/// Updates a reference to `version` (same as adding it with that version).
pub async fn update_package(dotnet: &Dotnet, project: &Path, id: &str, version: &str) -> anyhow::Result<CommandOutput> {
    add_package(dotnet, project, id, Some(version), false).await
}

/// `dotnet remove <project> package <id>`.
pub async fn remove_package(dotnet: &Dotnet, project: &Path, id: &str) -> anyhow::Result<CommandOutput> {
    validate_package_command(project, id)?;
    run_cli(dotnet, project, &["remove".to_string(), project.to_string_lossy().into_owned(), "package".into(), id.to_string()]).await
}

fn validate_package_command(project: &Path, id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(project.is_file() && project.extension().is_some_and(|e| ["csproj", "fsproj", "vbproj"].iter().any(|ext| e.eq_ignore_ascii_case(ext))), "A .NET project file is required for package changes");
    anyhow::ensure!(!id.is_empty() && id.len() <= 100 && id.split(['.', '-']).all(|part| part.chars().next().is_some_and(char::is_alphanumeric) && part.chars().all(|c| c.is_alphanumeric() || c == '_')), "Invalid NuGet package ID");
    Ok(())
}

fn validate_package_version(version: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!version.is_empty() && version.len() <= 256 && version.trim() == version && !version.starts_with(['-', '/']) && !version.chars().any(char::is_control), "Invalid or option-like NuGet package version");
    Ok(())
}

/// `dotnet restore <project>` (after editing central package versions).
pub async fn restore(dotnet: &Dotnet, project: &Path) -> anyhow::Result<CommandOutput> {
    run_cli(dotnet, project, &["restore".to_string(), project.to_string_lossy().into_owned()]).await
}

async fn run_cli(dotnet: &Dotnet, project: &Path, args: &[String]) -> anyhow::Result<CommandOutput> {
    let mut cmd = dotnet.command();
    cmd.args(args).kill_on_drop(true);
    if let Some(d) = project.parent().filter(|p| !p.as_os_str().is_empty()) {
        cmd.current_dir(d);
    }
    let out = cmd.output().await?;
    let output = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok(CommandOutput { success: out.status.success(), exit_code: out.status.code(), diagnostics: diagnostics::parse_all(&output), output })
}

/// A package source of `nuget.config`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageSource {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    /// From `packageSourceCredentials` (`Username` and `ClearTextPassword`).
    pub credentials: Option<(String, String)>,
}

/// The sources that apply in `dir`: the `nuget.config` files from `dir` up to the root, then the
/// user's, merged like NuGet (closest first; `<clear/>` stops the merge), nuget.org when none.
pub fn package_sources(dir: &Path) -> Vec<PackageSource> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut d = Some(dir);
    while let Some(cur) = d {
        for name in ["nuget.config", "NuGet.Config", "NuGet.config"] {
            let p = cur.join(name);
            if p.is_file() {
                files.push(p);
                break;
            }
        }
        d = cur.parent();
    }
    if let Some(home) = dirs_home() {
        for p in [home.join(".nuget/NuGet/NuGet.Config"), home.join("AppData/Roaming/NuGet/NuGet.Config")] {
            if p.is_file() {
                files.push(p);
            }
        }
    }
    let mut out: Vec<PackageSource> = Vec::new();
    let mut disabled: Vec<String> = Vec::new();
    let mut creds: std::collections::HashMap<String, (String, String)> = Default::default();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let parsed = parse_nuget_config(&text);
        for s in parsed.sources {
            if !out.iter().any(|o| o.name.eq_ignore_ascii_case(&s.0)) {
                out.push(PackageSource { name: s.0, url: s.1, enabled: true, credentials: None });
            }
        }
        disabled.extend(parsed.disabled);
        for (k, v) in parsed.credentials {
            creds.entry(k.to_ascii_lowercase()).or_insert(v);
        }
        if parsed.clear {
            break;
        }
    }
    if out.is_empty() {
        out.push(PackageSource { name: "nuget.org".into(), url: NUGET_ORG.into(), enabled: true, credentials: None });
    }
    for s in &mut out {
        s.enabled = !disabled.iter().any(|d| d.eq_ignore_ascii_case(&s.name));
        s.credentials = creds.get(&s.name.to_ascii_lowercase()).cloned();
    }
    out
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// What one `nuget.config` says about sources.
#[derive(Debug, Default, PartialEq)]
pub struct NugetConfig {
    pub sources: Vec<(String, String)>,
    pub disabled: Vec<String>,
    pub credentials: Vec<(String, (String, String))>,
    /// `<packageSources><clear/>`: the files further away do not add sources.
    pub clear: bool,
}

pub fn parse_nuget_config(text: &str) -> NugetConfig {
    let mut cfg = NugetConfig::default();
    let Ok(doc) = roxmltree::Document::parse(text) else { return cfg };
    for section in doc.root_element().children().filter(|n| n.is_element()) {
        match section.tag_name().name() {
            "packageSources" => {
                for n in section.children().filter(|n| n.is_element()) {
                    match n.tag_name().name() {
                        "clear" => cfg.clear = true,
                        "add" => {
                            if let (Some(k), Some(v)) = (n.attribute("key"), n.attribute("value")) {
                                cfg.sources.push((k.to_string(), v.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            "disabledPackageSources" => {
                for n in section.children().filter(|n| n.is_element() && n.tag_name().name() == "add") {
                    if n.attribute("value").is_some_and(|v| v.eq_ignore_ascii_case("true"))
                        && let Some(k) = n.attribute("key")
                    {
                        cfg.disabled.push(k.to_string());
                    }
                }
            }
            "packageSourceCredentials" => {
                for src in section.children().filter(|n| n.is_element()) {
                    let get = |key: &str| src.children().filter(|n| n.is_element()).find(|n| n.attribute("key").is_some_and(|k| k.eq_ignore_ascii_case(key))).and_then(|n| n.attribute("value")).map(str::to_string);
                    if let (Some(user), Some(pass)) = (get("Username"), get("ClearTextPassword")) {
                        // Element names escape spaces as `_x0020_`.
                        cfg.credentials.push((src.tag_name().name().replace("_x0020_", " "), (user, pass)));
                    }
                }
            }
            _ => {}
        }
    }
    cfg
}

/// NuGet version order (SemVer 2): numeric parts, then a release after its prereleases.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |v: &str| {
        let v = v.split('+').next().unwrap_or("");
        let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p.to_string())));
        let nums: Vec<u64> = core.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        (nums, pre)
    };
    let (na, pa) = split(a);
    let (nb, pb) = split(b);
    for i in 0..na.len().max(nb.len()) {
        let (x, y) = (na.get(i).copied().unwrap_or(0), nb.get(i).copied().unwrap_or(0));
        if x != y {
            return x.cmp(&y);
        }
    }
    match (pa, pb) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => {
            let (xs, ys): (Vec<&str>, Vec<&str>) = (x.split('.').collect(), y.split('.').collect());
            for i in 0..xs.len().max(ys.len()) {
                let (p, q) = (xs.get(i), ys.get(i));
                let o = match (p, q) {
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    (Some(p), Some(q)) => match (p.parse::<u64>(), q.parse::<u64>()) {
                        (Ok(m), Ok(n)) => m.cmp(&n),
                        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                        _ => p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase()),
                    },
                    (None, None) => std::cmp::Ordering::Equal,
                };
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            std::cmp::Ordering::Equal
        }
    }
}

/// The packages a project references with their versions, resolved through central package
/// management (`Directory.Packages.props`) when the reference has none.
pub fn installed_packages(project: &Path) -> Vec<crate::project::PackageRef> {
    let Ok(info) = crate::ProjectInfo::load(project) else { return Vec::new() };
    let central = central_versions(project.parent().unwrap_or(Path::new(".")));
    info.packages
        .into_iter()
        .map(|mut p| {
            if p.version.is_empty()
                && let Some(v) = central.iter().find(|(id, _)| id.eq_ignore_ascii_case(&p.name)).map(|(_, v)| v.clone())
            {
                p.version = v;
            }
            p
        })
        .collect()
}

/// `PackageVersion` items of the nearest `Directory.Packages.props`.
fn central_versions(dir: &Path) -> Vec<(String, String)> {
    let mut d = Some(dir);
    while let Some(cur) = d {
        let f = cur.join("Directory.Packages.props");
        if let Ok(text) = std::fs::read_to_string(&f) {
            let Ok(doc) = roxmltree::Document::parse(&text) else { return Vec::new() };
            return doc
                .descendants()
                .filter(|n| n.has_tag_name("PackageVersion"))
                .filter_map(|n| Some((n.attribute("Include")?.to_string(), n.attribute("Version")?.to_string())))
                .collect();
        }
        d = cur.parent();
    }
    Vec::new()
}

// ---------------------------------------------------------------- audit

/// A security advisory of a package version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advisory {
    /// `Low`, `Moderate`, `High` or `Critical`.
    pub severity: String,
    pub url: String,
}

/// Why a package is deprecated and what replaces it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deprecation {
    pub reasons: Vec<String>,
    /// (id, version range) of the alternative package.
    pub alternative: Option<(String, String)>,
}

/// A package `dotnet list package` reports as vulnerable or deprecated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditPackage {
    pub id: String,
    /// The resolved version.
    pub version: String,
    /// Projects that reference it.
    pub direct: Vec<PathBuf>,
    /// Projects that get it through another package.
    pub transitive: Vec<PathBuf>,
    pub advisories: Vec<Advisory>,
    pub deprecation: Option<Deprecation>,
}

impl AuditPackage {
    /// The worst advisory: 4 critical, 3 high, 2 moderate, 1 low, 0 none.
    pub fn severity(&self) -> u8 {
        self.advisories.iter().map(|a| severity_rank(&a.severity)).max().unwrap_or(0)
    }
}

pub fn severity_rank(s: &str) -> u8 {
    match s.to_ascii_lowercase().as_str() {
        "critical" | "3" => 4,
        "high" | "2" => 3,
        "moderate" | "medium" | "1" => 2,
        "low" | "0" => 1,
        _ => 0,
    }
}

/// What `dotnet list package --vulnerable / --deprecated` found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditReport {
    /// Vulnerable first (worst first), then deprecated, by id.
    pub packages: Vec<AuditPackage>,
    /// Errors and warnings `dotnet list` reports (projects not restored…).
    pub problems: Vec<String>,
}

impl AuditReport {
    /// Adds another report (one package and version once, with every project).
    pub fn merge(&mut self, other: AuditReport) {
        for p in other.packages {
            match self.packages.iter_mut().find(|q| q.id.eq_ignore_ascii_case(&p.id) && q.version == p.version) {
                Some(q) => {
                    for d in p.direct {
                        if !q.direct.contains(&d) {
                            q.direct.push(d);
                        }
                    }
                    for t in p.transitive {
                        if !q.transitive.contains(&t) && !q.direct.contains(&t) {
                            q.transitive.push(t);
                        }
                    }
                    for a in p.advisories {
                        if !q.advisories.contains(&a) {
                            q.advisories.push(a);
                        }
                    }
                    if q.deprecation.is_none() {
                        q.deprecation = p.deprecation;
                    }
                }
                None => self.packages.push(p),
            }
        }
        for m in other.problems {
            if !self.problems.contains(&m) {
                self.problems.push(m);
            }
        }
        self.packages.sort_by(|a, b| b.severity().cmp(&a.severity()).then_with(|| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase())));
    }
}

/// Parses `dotnet list package --format json` (`--vulnerable` or `--deprecated`).
pub fn parse_list_packages(json: &str) -> anyhow::Result<AuditReport> {
    let start = json.find('{').ok_or_else(|| anyhow::anyhow!("no JSON in dotnet list output"))?;
    let v: serde_json::Value = serde_json::from_str(&json[start..])?;
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|y| y.as_str()).unwrap_or("").to_string();
    let arr = |x: &serde_json::Value, k: &str| x.get(k).and_then(|y| y.as_array()).cloned().unwrap_or_default();
    let mut report = AuditReport::default();
    let problem = |p: &serde_json::Value| {
        let text = s(p, "text");
        let project = s(p, "project");
        if project.is_empty() { text } else { format!("{project}: {text}") }
    };
    report.problems.extend(arr(&v, "problems").iter().map(problem));
    let mut found = AuditReport::default();
    for proj in arr(&v, "projects") {
        let path = PathBuf::from(s(&proj, "path"));
        report.problems.extend(arr(&proj, "problems").iter().map(problem));
        for fw in arr(&proj, "frameworks") {
            for (key, transitive) in [("topLevelPackages", false), ("transitivePackages", true)] {
                for p in arr(&fw, key) {
                    let advisories = arr(&p, "vulnerabilities").iter().map(|a| Advisory { severity: s(a, "severity"), url: s(a, "advisoryurl") }).collect();
                    let reasons: Vec<String> = arr(&p, "deprecationReasons").iter().filter_map(|r| r.as_str().map(str::to_string)).collect();
                    let alternative = p.get("alternativePackage").map(|a| (s(a, "id"), s(a, "versionRange"))).filter(|(id, _)| !id.is_empty());
                    let deprecation = (!reasons.is_empty() || alternative.is_some()).then_some(Deprecation { reasons, alternative });
                    let (direct, trans) = if transitive { (vec![], vec![path.clone()]) } else { (vec![path.clone()], vec![]) };
                    found.packages.push(AuditPackage { id: s(&p, "id"), version: s(&p, "resolvedVersion"), direct, transitive: trans, advisories, deprecation });
                }
            }
        }
    }
    report.merge(found);
    Ok(report)
}

/// `dotnet list <target> package <args> --format json`.
pub async fn list_packages(dotnet: &Dotnet, target: &Path, args: &[&str]) -> anyhow::Result<AuditReport> {
    let mut cmd = dotnet.command();
    cmd.arg("list").arg(target).arg("package").args(args).args(["--format", "json"]).kill_on_drop(true);
    if let Some(d) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        cmd.current_dir(d);
    }
    let out = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    match parse_list_packages(&stdout) {
        Ok(r) => Ok(r),
        Err(_) => anyhow::bail!("dotnet list package failed: {}{}", stdout.trim(), String::from_utf8_lossy(&out.stderr).trim()),
    }
}

/// Vulnerable packages (with transitive ones) and deprecated ones of `target` (a solution or
/// a project). Slow: it asks every source.
pub async fn audit(dotnet: &Dotnet, target: &Path) -> anyhow::Result<AuditReport> {
    let mut report = list_packages(dotnet, target, &["--vulnerable", "--include-transitive"]).await?;
    report.merge(list_packages(dotnet, target, &["--deprecated"]).await?);
    Ok(report)
}

/// A version of a package's registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredVersion {
    pub version: String,
    pub vulnerable: bool,
    pub listed: bool,
}

/// Parses a registration index or page: its versions and the URLs of the pages it does not
/// inline.
pub fn parse_registration(json: &str) -> anyhow::Result<(Vec<RegisteredVersion>, Vec<String>)> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let (mut versions, mut pages) = (Vec::new(), Vec::new());
    let leaf = |it: &serde_json::Value| {
        let ce = it.get("catalogEntry")?;
        Some(RegisteredVersion {
            version: ce.get("version")?.as_str()?.to_string(),
            vulnerable: ce.get("vulnerabilities").and_then(|x| x.as_array()).is_some_and(|a| !a.is_empty()),
            listed: ce.get("listed").and_then(|x| x.as_bool()).unwrap_or(true),
        })
    };
    for it in v.get("items").and_then(|x| x.as_array()).into_iter().flatten() {
        if it.get("catalogEntry").is_some() {
            // A page: its items are versions.
            versions.extend(leaf(it));
        } else if let Some(items) = it.get("items").and_then(|x| x.as_array()) {
            versions.extend(items.iter().filter_map(leaf));
        } else if let Some(url) = it.get("@id").and_then(|x| x.as_str()) {
            pages.push(url.to_string());
        }
    }
    Ok((versions, pages))
}

/// The lowest listed version above `current` without known vulnerabilities (stable unless
/// `current` is a prerelease).
pub fn first_safe_version(versions: &[RegisteredVersion], current: &str) -> Option<String> {
    let pre = current.contains('-');
    versions
        .iter()
        .filter(|v| v.listed && !v.vulnerable && (pre || !v.version.contains('-')) && compare_versions(&v.version, current).is_gt())
        .map(|v| v.version.clone())
        .min_by(|a, b| compare_versions(a, b))
}

impl NugetClient {
    /// Every version of `id` with whether it has known vulnerabilities (`RegistrationsBaseUrl`).
    pub async fn registration(&self, id: &str) -> anyhow::Result<Vec<RegisteredVersion>> {
        let idx = self.service_index().await?;
        let base = idx.find("RegistrationsBaseUrl/3.6.0").or_else(|| idx.find("RegistrationsBaseUrl")).ok_or_else(|| anyhow::anyhow!("the source has no RegistrationsBaseUrl"))?.to_string();
        let url = format!("{}/{}/index.json", base.trim_end_matches('/'), id.to_lowercase());
        let (mut versions, pages) = parse_registration(&self.get_text(&url).await?)?;
        for p in pages {
            versions.extend(parse_registration(&self.get_text(&p).await?)?.0);
        }
        Ok(versions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vulnerable_packages() {
        let r = parse_list_packages(include_str!("../tests/data/list-package-vulnerable.json")).unwrap();
        let (a, b) = (PathBuf::from("/work/Audit/A/A.csproj"), PathBuf::from("/work/Audit/B/B.csproj"));
        assert_eq!(r.packages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["Newtonsoft.Json", "System.Net.Http", "System.Text.RegularExpressions"]);
        let json = &r.packages[0];
        assert_eq!((json.version.as_str(), json.severity(), json.direct.clone(), json.transitive.len()), ("12.0.1", 3, vec![a.clone(), b.clone()], 0));
        assert_eq!(json.advisories, [Advisory { severity: "High".into(), url: "https://github.com/advisories/GHSA-5crp-9r3c-p9vr".into() }]);
        // Direct in B, transitive in A.
        let http = &r.packages[1];
        assert_eq!((http.direct.clone(), http.transitive.clone()), (vec![b.clone()], vec![a.clone()]));
        assert_eq!(r.packages[2].transitive, [a]);
        assert!(r.problems.is_empty() && r.packages.iter().all(|p| p.deprecation.is_none()));
    }

    #[test]
    fn parses_deprecated_packages_and_problems() {
        let mut r = parse_list_packages(include_str!("../tests/data/list-package-deprecated.json")).unwrap();
        assert_eq!(r.packages.len(), 1);
        let p = &r.packages[0];
        assert_eq!((p.id.as_str(), p.version.as_str(), p.severity()), ("WindowsAzure.Storage", "9.3.3", 0));
        assert_eq!(p.deprecation, Some(Deprecation { reasons: vec!["Legacy".into()], alternative: Some(("Azure.Storage.Common".into(), ">= 0.0.0".into())) }));
        // Vulnerable ones sort first after a merge.
        r.merge(parse_list_packages(include_str!("../tests/data/list-package-vulnerable.json")).unwrap());
        assert_eq!(r.packages.last().unwrap().id, "WindowsAzure.Storage");
        let not_restored = r#"{ "version": 1, "problems": [ { "project": "/s/A/A.csproj", "level": "error", "text": "No assets file was found. Please run restore." } ], "projects": [] }"#;
        assert_eq!(parse_list_packages(not_restored).unwrap().problems, ["/s/A/A.csproj: No assets file was found. Please run restore."]);
        assert!(parse_list_packages("error: no").is_err());
    }

    #[test]
    fn finds_the_first_version_without_vulnerabilities() {
        let (versions, pages) = parse_registration(include_str!("../tests/data/registration-index.json")).unwrap();
        assert_eq!(pages, ["https://api.nuget.org/v3/registration5-gz-semver2/system.net.http/page/5.0.0/6.0.0.json"]);
        assert_eq!(versions.iter().filter(|v| !v.vulnerable).map(|v| v.version.as_str()).collect::<Vec<_>>(), ["4.3.4"]);
        assert_eq!(first_safe_version(&versions, "4.3.0").as_deref(), Some("4.3.4"));
        assert_eq!(first_safe_version(&versions, "4.3.4"), None);
        let more = [RegisteredVersion { version: "5.0.0-beta".into(), vulnerable: false, listed: true }, RegisteredVersion { version: "4.4.0".into(), vulnerable: false, listed: false }];
        assert_eq!(first_safe_version(&more, "4.3.0"), None);
        assert_eq!(first_safe_version(&more, "4.3.0-rc").as_deref(), Some("5.0.0-beta"));
    }

    #[test]
    fn versions_sort_like_nuget() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("13.0.3", "13.0.10"), Less);
        assert_eq!(compare_versions("2.0.0", "2.0.0-rc.1"), Greater);
        assert_eq!(compare_versions("2.0.0-rc.2", "2.0.0-rc.10"), Less);
        assert_eq!(compare_versions("1.0", "1.0.0"), Equal);
        assert_eq!(compare_versions("8.0.0-preview.1", "8.0.0-beta"), Greater);
    }

    #[test]
    fn reads_sources_disabled_ones_and_credentials() {
        let cfg = parse_nuget_config(
            r#"<configuration>
  <packageSources><clear /><add key="nuget.org" value="https://api.nuget.org/v3/index.json" /><add key="My Feed" value="https://pkgs.dev.azure.com/x/_packaging/x/nuget/v3/index.json" /></packageSources>
  <disabledPackageSources><add key="nuget.org" value="true" /></disabledPackageSources>
  <packageSourceCredentials><My_x0020_Feed><add key="Username" value="me" /><add key="ClearTextPassword" value="pat" /></My_x0020_Feed></packageSourceCredentials>
</configuration>"#,
        );
        assert!(cfg.clear);
        assert_eq!(cfg.sources.len(), 2);
        assert_eq!(cfg.disabled, ["nuget.org"]);
        assert_eq!(cfg.credentials, [("My Feed".to_string(), ("me".to_string(), "pat".to_string()))]);
    }

    #[test]
    fn central_package_versions_resolve_installed_references() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Directory.Packages.props"), r#"<Project><ItemGroup><PackageVersion Include="Serilog" Version="4.0.0" /></ItemGroup></Project>"#).unwrap();
        std::fs::create_dir(dir.path().join("App")).unwrap();
        std::fs::write(dir.path().join("App/App.csproj"), r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Serilog" /><PackageReference Include="Dapper" Version="2.1.0" /></ItemGroup></Project>"#).unwrap();
        let p = installed_packages(&dir.path().join("App/App.csproj"));
        assert_eq!(p.iter().map(|p| (p.name.as_str(), p.version.as_str())).collect::<Vec<_>>(), [("Serilog", "4.0.0"), ("Dapper", "2.1.0")]);
    }

    #[test]
    fn parses_service_index() {
        let idx = parse_service_index(
            r#"{"version":"3.0.0","resources":[
              {"@id":"https://azuresearch-usnc.nuget.org/query","@type":"SearchQueryService","comment":"x"},
              {"@id":"https://azuresearch-usnc.nuget.org/query","@type":"SearchQueryService/3.5.0"},
              {"@id":"https://api.nuget.org/v3-flatcontainer/","@type":["PackageBaseAddress/3.0.0"]}]}"#,
        )
        .unwrap();
        assert_eq!(idx.find("SearchQueryService"), Some("https://azuresearch-usnc.nuget.org/query"));
        assert_eq!(idx.find("PackageBaseAddress"), Some("https://api.nuget.org/v3-flatcontainer/"));
        assert_eq!(idx.find("Search"), None);
    }

    #[test]
    fn parses_search_results() {
        let r = parse_search(
            r#"{"totalHits":2,"data":[
              {"id":"Newtonsoft.Json","version":"13.0.3","description":"Json.NET","authors":["James Newton-King"],
               "totalDownloads":5000000000,"verified":true,"iconUrl":"https://i/x.png","tags":["json"],
               "versions":[{"version":"12.0.1","downloads":1},{"version":"13.0.3","downloads":2}]},
              {"id":"Other","version":"1.0.0-beta","authors":"A, B","tags":"","versions":[]}]}"#,
        )
        .unwrap();
        assert_eq!(r.total_hits, 2);
        let p = &r.packages[0];
        assert_eq!((p.id.as_str(), p.version.as_str()), ("Newtonsoft.Json", "13.0.3"));
        assert!(p.verified);
        assert_eq!(p.total_downloads, 5_000_000_000);
        assert_eq!(p.versions, vec!["12.0.1", "13.0.3"]);
        assert_eq!(r.packages[1].authors, vec!["A", "B"]);
        assert!(r.packages[1].tags.is_empty());
        assert_eq!(r.packages[1].project_url, None);
    }

    #[test]
    fn parses_flat_container_versions() {
        assert_eq!(parse_versions(r#"{"versions":["1.0.0","2.0.0-rc.1"]}"#).unwrap(), vec!["1.0.0", "2.0.0-rc.1"]);
    }
}
