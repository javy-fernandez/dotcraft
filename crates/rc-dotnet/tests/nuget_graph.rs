use std::path::Path;

use rc_dotnet::cpm::detect_static;
use rc_dotnet::nuget_graph::{NugetGraph, VersionSource};

#[test]
fn central_shop_graph_tracks_central_and_override_versions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/CentralShop").canonicalize().unwrap();
    let core = detect_static(&root.join("src/CentralShop.Core/CentralShop.Core.csproj"));
    let cli = detect_static(&root.join("src/CentralShop.Cli/CentralShop.Cli.csproj"));
    assert!(core.enabled && cli.enabled);
    let graph = NugetGraph::from_projects(&[core, cli]).unwrap();
    assert_eq!(graph.projects.len(), 2);
    assert_eq!(graph.packages.len(), 3);
    assert_eq!(graph.edges.len(), 3);
    let core_packages: Vec<_> = graph.packages_for(0).collect();
    assert!(core_packages.iter().any(|(p, source)| p.id == "Humanizer.Core" && p.version.as_deref() == Some("2.14.1") && matches!(source, VersionSource::Central(_))));
    assert!(core_packages.iter().any(|(p, source)| p.id == "Newtonsoft.Json" && p.version.as_deref() == Some("13.0.1") && matches!(source, VersionSource::Central(_))));
    let cli_packages: Vec<_> = graph.packages_for(1).collect();
    assert_eq!(cli_packages.len(), 1);
    assert_eq!(cli_packages[0].0.id, "Newtonsoft.Json");
    assert_eq!(cli_packages[0].0.version.as_deref(), Some("13.0.3"));
    assert_eq!(cli_packages[0].1, &VersionSource::Override);
}

#[test]
fn shop_api_direct_package_and_missing_cpm_version_are_distinct() {
    let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj").canonicalize().unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("Directory.Build.props"), "<Project><PropertyGroup><ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally></PropertyGroup></Project>").unwrap();
    std::fs::write(temp.path().join("Directory.Packages.props"), "<Project><ItemGroup><GlobalPackageReference Include=\"Global.Tool\" Version=\"1.2.3\" /></ItemGroup></Project>").unwrap();
    let project = temp.path().join("App.csproj");
    std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\"><ItemGroup><PackageReference Include=\"Missing.Package\" /></ItemGroup></Project>").unwrap();
    let graph = NugetGraph::from_projects(&[detect_static(&shop), detect_static(&project)]).unwrap();
    let shop_packages: Vec<_> = graph.packages_for(0).collect();
    assert_eq!(shop_packages.len(), 1);
    assert_eq!(shop_packages[0].0.id, "Swashbuckle.AspNetCore");
    assert_eq!(shop_packages[0].0.version.as_deref(), Some("6.6.2"));
    assert_eq!(shop_packages[0].1, &VersionSource::Project);

    let app_packages: Vec<_> = graph.packages_for(1).collect();
    assert!(app_packages.iter().any(|(p, source)| p.id == "Missing.Package" && p.version.is_none() && source == &&VersionSource::Unresolved));
    assert!(app_packages.iter().any(|(p, source)| p.id == "Global.Tool" && p.version.as_deref() == Some("1.2.3") && matches!(source, VersionSource::Central(_))));
}
