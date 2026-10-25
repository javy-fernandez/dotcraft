//! Quick, static reading of SDK-style project files (no MSBuild evaluation): enough for the
//! explorer's Dependencies node and for choosing run configurations. Evaluated properties
//! come from `dotnet msbuild -getProperty` in the build module.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageRef {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectInfo {
    pub path: PathBuf,
    pub name: String,
    pub sdk: String,
    pub target_frameworks: Vec<String>,
    /// `Exe`, `WinExe`, `Library`… (empty: the SDK default, a library).
    pub output_type: String,
    pub packages: Vec<PackageRef>,
    pub project_refs: Vec<PathBuf>,
    /// Test project (references a test SDK or sets `IsTestProject`).
    pub is_test: bool,
    /// .NET Aspire app host (runs the other projects).
    pub aspire_host: bool,
}

impl ProjectInfo {
    pub fn load(path: &Path) -> anyhow::Result<ProjectInfo> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text, path)
    }

    pub fn parse(text: &str, path: &Path) -> anyhow::Result<ProjectInfo> {
        let doc = roxmltree::Document::parse(text)?;
        let root = doc.root_element();
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut info = ProjectInfo {
            path: path.to_path_buf(),
            name: path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
            sdk: root.attribute("Sdk").unwrap_or("").to_string(),
            ..Default::default()
        };
        for n in root.descendants().filter(|n| n.is_element()) {
            let text = || n.text().unwrap_or("").trim().to_string();
            match n.tag_name().name() {
                "TargetFramework" => info.target_frameworks = vec![text()],
                "TargetFrameworks" => info.target_frameworks = text().split(';').filter(|s| !s.is_empty()).map(str::to_string).collect(),
                "OutputType" => info.output_type = text(),
                "IsTestProject" => info.is_test |= text().eq_ignore_ascii_case("true"),
                "IsAspireHost" => info.aspire_host |= text().eq_ignore_ascii_case("true"),
                // `<Sdk Name="…"/>`: the project's SDK when the root has none, or an extra one.
                "Sdk" => {
                    let name = n.attribute("Name").unwrap_or("");
                    info.is_test |= name.to_ascii_lowercase().starts_with("mstest.sdk");
                    info.aspire_host |= name.to_ascii_lowercase().starts_with("aspire.apphost.sdk");
                    if info.sdk.is_empty() {
                        info.sdk = name.to_string();
                    }
                }
                "PackageReference" => {
                    let name = n.attribute("Include").or(n.attribute("Update")).unwrap_or("").to_string();
                    let version = n
                        .attribute("Version")
                        .map(str::to_string)
                        .or_else(|| n.children().find(|c| c.has_tag_name("Version")).and_then(|c| c.text()).map(|t| t.trim().to_string()))
                        .unwrap_or_default();
                    if is_test_package(&name) {
                        info.is_test = true;
                    }
                    if !name.is_empty() {
                        info.packages.push(PackageRef { name, version });
                    }
                }
                "ProjectReference" => {
                    if let Some(inc) = n.attribute("Include") {
                        let mut p = dir.to_path_buf();
                        for part in inc.replace('\\', "/").split('/') {
                            match part {
                                ".." => {
                                    p.pop();
                                }
                                "" | "." => {}
                                x => p.push(x),
                            }
                        }
                        info.project_refs.push(p);
                    }
                }
                _ => {}
            }
        }
        info.aspire_host |= info.sdk.to_ascii_lowercase().starts_with("aspire.apphost.sdk");
        if info.sdk.to_ascii_lowercase().starts_with("mstest.sdk") {
            info.is_test = true;
        }
        if !info.is_test {
            info.is_test = inherits_test_settings(dir, &info.name);
        }
        Ok(info)
    }

    /// The SDK name without its version (`Microsoft.NET.Sdk.Web/8.0.0` → `Microsoft.NET.Sdk.Web`).
    pub fn sdk_name(&self) -> &str {
        self.sdk.split('/').next().unwrap_or("").trim()
    }

    /// Whether `dotnet run` can start it: an Exe, or a web, Blazor WebAssembly, worker or Aspire
    /// project (their SDKs make them executables).
    pub fn is_executable(&self) -> bool {
        let sdk = self.sdk_name().to_ascii_lowercase();
        self.output_type.eq_ignore_ascii_case("Exe")
            || self.output_type.eq_ignore_ascii_case("WinExe")
            || self.aspire_host
            || ["microsoft.net.sdk.web", "microsoft.net.sdk.blazorwebassembly", "microsoft.net.sdk.worker"].contains(&sdk.as_str())
    }

    pub fn is_web(&self) -> bool {
        let sdk = self.sdk_name().to_ascii_lowercase();
        sdk.starts_with("microsoft.net.sdk.web") || sdk.starts_with("microsoft.net.sdk.blazorwebassembly")
    }
}

/// Packages that make a project a test project (test SDK, frameworks, adapters, the testing
/// platform).
fn is_test_package(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "microsoft.net.test.sdk"
        || n.starts_with("microsoft.testing.platform")
        || n.starts_with("xunit")
        || n.starts_with("nunit")
        || n.starts_with("mstest")
        || n == "tunit"
        || n.starts_with("tunit.")
}

/// Whether a `Directory.Build.props` above the project makes it a test project: `IsTestProject`
/// or a test package, unconditionally or under a condition on the project name such as
/// `$(MSBuildProjectName.EndsWith('Tests'))` (the usual way to share test settings).
fn inherits_test_settings(dir: &Path, project: &str) -> bool {
    let mut d = Some(dir);
    while let Some(cur) = d {
        let props = cur.join("Directory.Build.props");
        if let Ok(text) = std::fs::read_to_string(&props) {
            if let Ok(doc) = roxmltree::Document::parse(&text) {
                let applies = |n: roxmltree::Node| n.ancestors().filter_map(|a| a.attribute("Condition")).all(|c| condition_matches(c, project));
                let test = doc.descendants().filter(|n| n.is_element()).any(|n| match n.tag_name().name() {
                    "IsTestProject" => n.text().is_some_and(|t| t.trim().eq_ignore_ascii_case("true")) && applies(n),
                    "PackageReference" => n.attribute("Include").is_some_and(is_test_package) && applies(n),
                    _ => false,
                });
                if test {
                    return true;
                }
            }
            // MSBuild stops at the first Directory.Build.props unless it imports its parent.
            if !text.contains("GetPathOfFileAbove") {
                return false;
            }
        }
        d = cur.parent();
    }
    false
}

/// Evaluates the conditions on the project name that test settings use; anything else is
/// treated as not matching.
fn condition_matches(condition: &str, project: &str) -> bool {
    let c = condition.replace(' ', "");
    let arg = |f: &str| c.split(&format!("MSBuildProjectName.{f}('")).nth(1).and_then(|r| r.split('\'').next()).map(str::to_string);
    let negated = c.starts_with('!');
    let result = if let Some(x) = arg("EndsWith") {
        project.ends_with(&x)
    } else if let Some(x) = arg("Contains") {
        project.contains(&x)
    } else if let Some(x) = arg("StartsWith") {
        project.starts_with(&x)
    } else {
        return false;
    };
    result != negated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_frameworks_packages_and_references() {
        let p = ProjectInfo::parse(
            r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup><OutputType>Exe</OutputType><TargetFrameworks>net8.0;net10.0</TargetFrameworks></PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <PackageReference Include="xunit"><Version>2.9.0</Version></PackageReference>
    <ProjectReference Include="..\Lib\Lib.csproj" />
  </ItemGroup>
</Project>"#,
            Path::new("/w/App/App.csproj"),
        )
        .unwrap();
        assert_eq!(p.target_frameworks, vec!["net8.0", "net10.0"]);
        assert!(p.is_executable() && p.is_test);
        assert_eq!(p.packages[1], PackageRef { name: "xunit".into(), version: "2.9.0".into() });
        assert_eq!(p.project_refs, vec![PathBuf::from("/w/Lib/Lib.csproj")]);
    }

    #[test]
    fn recognises_test_projects_by_sdk_packages_and_shared_props() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("Directory.Build.props"),
            r#"<Project>
  <ItemGroup Condition="$(MSBuildProjectName.EndsWith('.Tests'))">
    <PackageReference Include="xunit.v3" />
  </ItemGroup>
</Project>"#,
        )
        .unwrap();
        let plain = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#;
        std::fs::create_dir_all(root.join("App.Tests")).unwrap();
        std::fs::create_dir_all(root.join("App")).unwrap();
        assert!(ProjectInfo::parse(plain, &root.join("App.Tests/App.Tests.csproj")).unwrap().is_test);
        assert!(!ProjectInfo::parse(plain, &root.join("App/App.csproj")).unwrap().is_test);
        let mstest = r#"<Project Sdk="MSTest.Sdk/3.8.2"></Project>"#;
        assert!(ProjectInfo::parse(mstest, &root.join("App/A.csproj")).unwrap().is_test);
        let tunit = r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="TUnit" Version="0.19" /></ItemGroup></Project>"#;
        assert!(ProjectInfo::parse(tunit, &root.join("App/B.csproj")).unwrap().is_test);
    }

    #[test]
    fn web_blazor_worker_and_aspire_projects_are_executables() {
        let parse = |t: &str| ProjectInfo::parse(t, Path::new("/w/P/P.csproj")).unwrap();
        assert!(parse(r#"<Project Sdk="Microsoft.NET.Sdk.Web/8.0.0"></Project>"#).is_executable());
        assert!(parse(r#"<Project Sdk="Microsoft.NET.Sdk.BlazorWebAssembly"></Project>"#).is_web());
        assert!(parse(r#"<Project><Sdk Name="Microsoft.NET.Sdk.Web" /></Project>"#).is_executable());
        assert!(parse(r#"<Project Sdk="Microsoft.NET.Sdk.Worker"></Project>"#).is_executable());
        assert!(parse(r#"<Project Sdk="Microsoft.NET.Sdk"><Sdk Name="Aspire.AppHost.Sdk" Version="9.0.0" /></Project>"#).is_executable());
        assert!(!parse(r#"<Project Sdk="Microsoft.NET.Sdk.Razor"></Project>"#).is_executable());
        assert!(!parse(r#"<Project Sdk="Microsoft.NET.Sdk"></Project>"#).is_executable());
    }
}
