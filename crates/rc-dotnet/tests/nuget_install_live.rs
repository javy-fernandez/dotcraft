//! Live SDK acceptance for NuGet project references. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test nuget_install_live -- --ignored`.

use std::path::{Path, PathBuf};

use rc_dotnet::{Dotnet, nuget};

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if matches!(
            entry.file_name().to_str(),
            Some("bin" | "obj" | "TestResults")
        ) {
            continue;
        }
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            std::fs::copy(path, target).unwrap();
        }
    }
}

fn prepare_fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let shop = dir.path().join("Shop");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop"),
        &shop,
    );
    let feed = dir.path().join("feed");
    std::fs::create_dir(&feed).unwrap();
    let cache = std::env::var_os("NUGET_PACKAGES")
        .expect("set NUGET_PACKAGES to the cached package directory");
    for version in ["13.0.1", "13.0.3"] {
        let name = format!("newtonsoft.json.{version}.nupkg");
        std::fs::copy(
            Path::new(&cache)
                .join("newtonsoft.json")
                .join(version)
                .join(&name),
            feed.join(name),
        )
        .unwrap();
    }
    std::fs::write(shop.join("NuGet.Config"), format!("<configuration><packageSources><clear/><add key=\"fixture\" value=\"{}\" /></packageSources></configuration>\n", feed.display())).unwrap();
    (dir, shop)
}

fn version(project: &Path) -> Option<String> {
    nuget::installed_packages(project)
        .into_iter()
        .find(|p| p.name == "Newtonsoft.Json")
        .map(|p| p.version)
}

#[tokio::test]
#[ignore = "requires a locally installed .NET 8 SDK and cached Newtonsoft.Json 13.0.1/13.0.3 packages"]
async fn adds_updates_and_removes_package_in_shop_core() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET").expect("set DOTCRAFT_TEST_DOTNET");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));
    let (_dir, shop) = prepare_fixture();
    let project = shop.join("src/Shop.Core/Shop.Core.csproj");
    assert_eq!(version(&project), None);

    let added = nuget::add_package(&dotnet, &project, "Newtonsoft.Json", Some("13.0.1"), false)
        .await
        .unwrap();
    assert!(added.success, "{}", added.output);
    assert_eq!(version(&project).as_deref(), Some("13.0.1"));

    let updated = nuget::update_package(&dotnet, &project, "Newtonsoft.Json", "13.0.3")
        .await
        .unwrap();
    assert!(updated.success, "{}", updated.output);
    assert_eq!(version(&project).as_deref(), Some("13.0.3"));
    let probe = shop.join("src/Shop.Core/NugetProbe.cs");
    std::fs::write(&probe, "namespace Shop.Core; public static class NugetProbe { public static string Json() => Newtonsoft.Json.JsonConvert.SerializeObject(new { Value = 42 }); }\n").unwrap();
    let built = dotnet
        .std_command()
        .arg("build")
        .arg(&project)
        .args(["--no-restore", "-nologo", "-v:minimal"])
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&built.stdout),
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(
        shop.join("src/Shop.Core/bin/Debug/net8.0/Shop.Core.dll")
            .is_file()
    );

    std::fs::remove_file(probe).unwrap();
    let removed = nuget::remove_package(&dotnet, &project, "Newtonsoft.Json")
        .await
        .unwrap();
    assert!(removed.success, "{}", removed.output);
    assert_eq!(version(&project), None);
    let text = std::fs::read_to_string(&project).unwrap();
    assert!(!text.contains("Newtonsoft.Json"), "{text}");
}
