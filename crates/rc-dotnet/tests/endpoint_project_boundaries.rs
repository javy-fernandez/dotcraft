use rc_dotnet::endpoints::{EndpointKind, scan_project, scan_project_cancellable};
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[test]
fn shop_api_fixture_reports_only_its_four_endpoints() {
    let project =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api");
    let endpoints = scan_project(&project);
    let routes: Vec<_> = endpoints
        .iter()
        .map(|endpoint| (endpoint.method.as_str(), endpoint.route.as_str()))
        .collect();
    assert_eq!(
        routes,
        vec![
            ("GET", "/Orders/sample"),
            ("POST", "/Orders/quote"),
            ("GET", "/WeatherForecast"),
            ("GET", "/health")
        ]
    );
    assert_eq!(endpoints[3].kind, EndpointKind::MinimalApi);
    assert!(endpoints.iter().all(|endpoint| endpoint.file.is_file()));
}

#[test]
fn nested_projects_of_each_dotnet_language_do_not_leak_endpoints() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("Web.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk.Web\" />\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Program.cs"),
        "app.MapGet(\"/alive\", () => true);\n",
    )
    .unwrap();
    std::fs::create_dir(root.join("loose")).unwrap();
    std::fs::write(
        root.join("loose/Extra.cs"),
        "app.MapGet(\"/extra\", () => true);\n",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("Web.csproj"), root.join("loose/Linked.csproj")).unwrap();
    for (name, extension) in [
        ("CSharp", "csproj"),
        ("FSharp", "fsproj"),
        ("VisualBasic", "VBPROJ"),
    ] {
        let nested = root.join(name);
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(
            nested.join(format!("{name}.{extension}")),
            "<Project Sdk=\"Microsoft.NET.Sdk\" />\n",
        )
        .unwrap();
        std::fs::write(
            nested.join("Ghost.cs"),
            format!("app.MapGet(\"/ghost-{name}\", () => true);\n"),
        )
        .unwrap();
    }
    let endpoints = scan_project(root);
    let routes: Vec<_> = endpoints
        .iter()
        .map(|endpoint| endpoint.route.as_str())
        .collect();
    assert_eq!(routes, vec!["/alive", "/extra"]);
    assert!(scan_project_cancellable(root, &AtomicBool::new(true)).is_empty());
}
