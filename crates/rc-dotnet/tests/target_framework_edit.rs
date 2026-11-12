use std::path::Path;

use rc_dotnet::project::{ProjectInfo, set_target_frameworks};

fn shop_project(relative: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src").join(relative)).unwrap()
}

#[test]
fn edits_shop_core_framework_and_preserves_bom_and_other_properties() {
    let original = shop_project("Shop.Core/Shop.Core.csproj");
    assert!(original.starts_with('\u{feff}'));
    let edited = set_target_frameworks(&original, &["net9.0"]).unwrap();
    assert_eq!(edited, original.replacen("<TargetFramework>net8.0</TargetFramework>", "<TargetFramework>net9.0</TargetFramework>", 1));
    assert!(edited.starts_with('\u{feff}'));
    let info = ProjectInfo::parse(edited.trim_start_matches('\u{feff}'), Path::new("Shop.Core.csproj")).unwrap();
    assert_eq!(info.target_frameworks, ["net9.0"]);
}

#[test]
fn converts_shop_api_to_multiple_frameworks_without_touching_other_xml() {
    let original = shop_project("Shop.Api/Shop.Api.csproj");
    let edited = set_target_frameworks(&original, &["net8.0", "net9.0-windows"]).unwrap();
    assert_eq!(edited, original.replacen("<TargetFramework>net8.0</TargetFramework>", "<TargetFrameworks>net8.0;net9.0-windows</TargetFrameworks>", 1));
    let back = set_target_frameworks(&edited, &["net10.0"]).unwrap();
    assert_eq!(ProjectInfo::parse(back.trim_start_matches('\u{feff}'), Path::new("Shop.Api.csproj")).unwrap().target_frameworks, ["net10.0"]);
}

#[test]
fn rejects_ambiguous_conditional_or_invalid_framework_edits() {
    let project = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>"#;
    for frameworks in [vec![], vec!["net8.0", "NET8.0"], vec!["net8.0;bad"], vec!["net8.0<bad"]] {
        assert!(set_target_frameworks(project, &frameworks).is_err());
    }
    for source in [
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup Condition="'$(Configuration)' == 'Debug'"><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>"#,
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework><TargetFrameworks>net8.0;net9.0</TargetFrameworks></PropertyGroup></Project>"#,
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup /></Project>"#,
        r#"<Project><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>"#,
    ] {
        assert!(set_target_frameworks(source, &["net9.0"]).is_err());
    }
}
