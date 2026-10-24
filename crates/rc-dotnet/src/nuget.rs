//! NuGet: a small v3 client (service index, search, versions from the flat container) over
//! rustls, plus adding, updating and removing package references through the `dotnet` CLI.

use std::path::Path;
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
            .build()?;
        Ok(NugetClient { http, index_url: index_url.to_string(), index: Default::default() })
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
        let resp = self.http.get(url).send().await?;
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
    run_cli(dotnet, project, &["remove".to_string(), project.to_string_lossy().into_owned(), "package".into(), id.to_string()]).await
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

#[cfg(test)]
mod tests {
    use super::*;

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
