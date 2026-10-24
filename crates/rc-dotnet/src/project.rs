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
                "PackageReference" => {
                    let name = n.attribute("Include").or(n.attribute("Update")).unwrap_or("").to_string();
                    let version = n
                        .attribute("Version")
                        .map(str::to_string)
                        .or_else(|| n.children().find(|c| c.has_tag_name("Version")).and_then(|c| c.text()).map(|t| t.trim().to_string()))
                        .unwrap_or_default();
                    if ["Microsoft.NET.Test.Sdk", "xunit", "NUnit", "MSTest.TestFramework", "MSTest"].iter().any(|t| name.eq_ignore_ascii_case(t)) {
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
        if info.sdk.eq_ignore_ascii_case("MSTest.Sdk") {
            info.is_test = true;
        }
        Ok(info)
    }

    pub fn is_executable(&self) -> bool {
        self.output_type.eq_ignore_ascii_case("Exe") || self.output_type.eq_ignore_ascii_case("WinExe") || self.sdk.eq_ignore_ascii_case("Microsoft.NET.Sdk.Web")
    }

    pub fn is_web(&self) -> bool {
        self.sdk.starts_with("Microsoft.NET.Sdk.Web") || self.sdk.starts_with("Microsoft.NET.Sdk.Razor")
    }
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
}
