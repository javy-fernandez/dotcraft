//! Live SDK acceptance for evaluated project properties. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test msbuild_evaluation_live -- --ignored`.

use std::path::{Path, PathBuf};

use rc_dotnet::{msbuild, sdk::Dotnet};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/Shop/src")
        .join(name)
        .join(format!("{name}.csproj"))
}

fn copy_project(root: &Path, name: &str) -> PathBuf {
    let project = root.join("src").join(name).join(format!("{name}.csproj"));
    std::fs::create_dir_all(project.parent().unwrap()).unwrap();
    std::fs::copy(fixture(name), &project).unwrap();
    project
}

#[tokio::test]
#[ignore = "requires a locally installed .NET 8 SDK; set DOTCRAFT_TEST_DOTNET"]
async fn evaluates_real_shop_projects_and_declared_inner_builds() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let dir = tempfile::tempdir().unwrap();
    let core = copy_project(dir.path(), "Shop.Core");
    let api = copy_project(dir.path(), "Shop.Api");

    let core_props = msbuild::evaluate(&dotnet, &core, "Debug", None)
        .await
        .unwrap();
    assert_eq!(core_props.target_framework, "net8.0");
    assert_eq!(core_props.target_frameworks, ["net8.0"]);
    assert_eq!(core_props.output_type, "Library");
    assert_eq!(core_props.assembly_name, "Shop.Core");
    assert!(
        core_props
            .target_path
            .ends_with("bin/Debug/net8.0/Shop.Core.dll")
    );

    let api_props = msbuild::evaluate(&dotnet, &api, "Debug", None)
        .await
        .unwrap();
    assert_eq!(api_props.target_framework, "net8.0");
    assert_eq!(api_props.target_frameworks, ["net8.0"]);
    assert_eq!(api_props.assembly_name, "Shop.Api");
    assert!(api_props.is_web && api_props.is_executable());
    assert!(
        api_props
            .target_path
            .ends_with("bin/Debug/net8.0/Shop.Api.dll")
    );

    let release = msbuild::evaluate(&dotnet, &core, "Release", Some("net8.0"))
        .await
        .unwrap();
    assert!(
        release
            .target_path
            .ends_with("bin/Release/net8.0/Shop.Core.dll")
    );

    let single = msbuild::get_properties(&dotnet, &core, &["AssemblyName"], &[])
        .await
        .unwrap();
    assert_eq!(single["AssemblyName"], "Shop.Core");

    let source = std::fs::read_to_string(&core).unwrap();
    let multitarget = source.replace(
        "<TargetFramework>net8.0</TargetFramework>",
        "<TargetFrameworks>net8.0;netstandard2.0</TargetFrameworks>",
    );
    assert_ne!(source, multitarget);
    std::fs::write(&core, multitarget).unwrap();

    let first = msbuild::evaluate(&dotnet, &core, "Debug", None)
        .await
        .unwrap();
    assert_eq!(first.target_framework, "net8.0");
    assert_eq!(first.target_frameworks, ["net8.0", "netstandard2.0"]);
    assert!(
        first
            .target_path
            .ends_with("bin/Debug/net8.0/Shop.Core.dll")
    );

    let second = msbuild::evaluate(&dotnet, &core, "Debug", Some("netstandard2.0"))
        .await
        .unwrap();
    assert_eq!(second.target_framework, "netstandard2.0");
    assert_eq!(second.target_frameworks, ["net8.0", "netstandard2.0"]);
    assert!(
        second
            .target_path
            .ends_with("bin/Debug/netstandard2.0/Shop.Core.dll")
    );

    let unknown = msbuild::evaluate(&dotnet, &core, "Debug", Some("net99.0"))
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("not declared"), "{unknown:#}");
}
