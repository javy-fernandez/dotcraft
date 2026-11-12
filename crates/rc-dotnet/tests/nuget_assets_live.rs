//! SDK acceptance for NuGet's resolved dependency graph.

use std::path::Path;

use rc_dotnet::{Dotnet, nuget_assets};

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if matches!(
            entry.file_name().to_str(),
            Some("bin" | "obj" | "TestResults")
        ) {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
#[ignore = "requires a local .NET 8 SDK and cached Shop packages"]
fn resolves_shop_api_package_edges_after_real_restore() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET").expect("set DOTCRAFT_TEST_DOTNET");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));
    let dir = tempfile::tempdir().unwrap();
    let shop = dir.path().join("Shop");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop"),
        &shop,
    );
    std::fs::write(
        shop.join("NuGet.Config"),
        "<configuration><packageSources><clear/></packageSources></configuration>\n",
    )
    .unwrap();
    let api = shop.join("src/Shop.Api/Shop.Api.csproj");
    assert!(nuget_assets::load(&api).is_err());
    let output = dotnet
        .std_command()
        .arg("restore")
        .arg(&api)
        .args(["-p:NuGetAudit=false", "--ignore-failed-sources", "-v:quiet"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let graph = nuget_assets::load(&api).unwrap();
    assert_eq!(graph.framework, "net8.0");
    let swashbuckle = graph
        .packages
        .iter()
        .position(|p| p.id == "Swashbuckle.AspNetCore" && p.version == "6.6.2" && p.direct)
        .unwrap();
    let swagger = graph
        .packages
        .iter()
        .position(|p| p.id == "Swashbuckle.AspNetCore.Swagger" && p.version == "6.6.2" && !p.direct)
        .unwrap();
    let openapi = graph
        .packages
        .iter()
        .position(|p| p.id == "Microsoft.OpenApi" && p.version == "1.6.14" && !p.direct)
        .unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|e| e.from == swashbuckle && e.to == swagger)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|e| e.from == swagger && e.to == openapi)
    );
    assert!(graph.packages.iter().all(|p| p.id != "Shop.Core"));
}
