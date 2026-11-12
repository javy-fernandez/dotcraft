use std::path::{Path, PathBuf};

use rc_dotnet::cpm::{CentralFile, SolutionCpm, apply_edits, detect_static, set_central_version};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/CentralShop")
}

#[test]
fn central_shop_static_versions_match_real_fixture() {
    let root = fixture().canonicalize().unwrap();
    let core = detect_static(&root.join("src/CentralShop.Core/CentralShop.Core.csproj"));
    let cli = detect_static(&root.join("src/CentralShop.Cli/CentralShop.Cli.csproj"));

    assert!(core.enabled && cli.enabled);
    assert_eq!(core.central_version("Dapper").unwrap().version, "2.1.66");
    assert_eq!(core.central_version("Humanizer.Core").unwrap().version, "2.14.1");
    assert_eq!(cli.central_version("Newtonsoft.Json").unwrap().version, "13.0.1");
    assert_eq!(cli.effective_version("Newtonsoft.Json").as_deref(), Some("13.0.3"));
}

#[test]
fn nested_updates_override_imported_items_without_creating_missing_items() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let fixture_root = fixture();
    std::fs::copy(fixture_root.join("Directory.Build.props"), root.join("Directory.Build.props")).unwrap();
    let parent = root.join("Directory.Packages.props");
    std::fs::copy(fixture_root.join("Directory.Packages.props"), &parent).unwrap();
    let nested_dir = root.join("src");
    std::fs::create_dir(&nested_dir).unwrap();
    let nested = nested_dir.join("Directory.Packages.props");
    let text = r#"<Project>
  <Import Project="../Directory.Packages.props" />
  <ItemGroup>
    <PackageVersion Update="newtonsoft.json" Version="13.0.3" />
    <PackageVersion Update="Dapper" PrivateAssets="all" />
    <PackageVersion Update="NotInParent" Version="9.0.0" />
    <PackageVersion Include="Polly" Version="8.0.0" />
  </ItemGroup>
</Project>"#;
    std::fs::write(&nested, text).unwrap();
    let project_dir = nested_dir.join("App");
    std::fs::create_dir(&project_dir).unwrap();
    let project = project_dir.join("App.csproj");
    std::fs::write(&project, r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Newtonsoft.Json" /><PackageReference Include="Polly" /><PackageReference Include="NotInParent" /></ItemGroup></Project>"#).unwrap();

    let parsed = CentralFile::load(&nested).unwrap();
    assert!(parsed.items[0].update && parsed.items[1].update && parsed.items[2].update);
    assert!(!parsed.items[3].update);
    let cpm = detect_static(&project);
    assert!(cpm.enabled);
    assert_eq!(cpm.central_version("Newtonsoft.Json").map(|v| (v.version.as_str(), v.file.as_path())), Some(("13.0.3", nested.as_path())));
    assert_eq!(cpm.central_version("Dapper").map(|v| (v.version.as_str(), v.file.as_path())), Some(("2.1.66", parent.as_path())));
    assert_eq!(cpm.central_version("Polly").map(|v| (v.version.as_str(), v.file.as_path())), Some(("8.0.0", nested.as_path())));
    assert!(cpm.central_version("NotInParent").is_none());

    let solution = SolutionCpm::new(vec![cpm]);
    assert_eq!(solution.file_for(&project, "Newtonsoft.Json"), Some(nested));
}

#[test]
fn central_version_edit_finds_update_item() {
    let text = r#"<Project><ItemGroup><PackageVersion Update="Newtonsoft.Json" Version="13.0.3" /></ItemGroup></Project>"#;
    let edited = apply_edits(text, &set_central_version(text, "newtonsoft.json", None, "13.0.4").unwrap());
    assert!(edited.contains("Update=\"Newtonsoft.Json\" Version=\"13.0.4\""));
}
