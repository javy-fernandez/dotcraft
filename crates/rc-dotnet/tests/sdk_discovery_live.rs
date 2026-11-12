use std::path::Path;

use rc_dotnet::sdk::{Dotnet, candidates};

#[test]
fn existing_extensionless_explicit_host_is_not_treated_as_a_directory() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("custom-dotnet-host");
    std::fs::write(&host, "host wrapper").unwrap();
    assert_eq!(candidates(Some(&host), None, None, None, None)[0], host);
    assert_eq!(Dotnet::locate(Some(&host)).unwrap().path, host.canonicalize().unwrap());
}

#[test]
#[ignore = "requires a local .NET SDK via DOTCRAFT_TEST_DOTNET"]
fn discovered_sdk_builds_a_copy_of_shop_core() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET").expect("set DOTCRAFT_TEST_DOTNET to a real dotnet host");
    let dotnet = Dotnet::locate(Some(Path::new(&host))).expect("the explicit dotnet host must exist");
    assert_eq!(Dotnet::locate(dotnet.root()).unwrap().path, dotnet.path);
    assert_eq!(Dotnet::locate(None).expect("put the local SDK on PATH").path, dotnet.path);
    let sdks = dotnet.list_sdks().unwrap();
    assert!(!sdks.is_empty());
    let selected = dotnet.version(None).unwrap();
    assert!(sdks.iter().any(|sdk| sdk.version == selected && sdk.path.join(&selected).is_dir()));
    if let Ok(expected) = std::env::var("DOTCRAFT_TEST_SDK_VERSION") {
        assert_eq!(selected, expected);
    }

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core");
    let temp = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(fixture).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "cs" || ext == "csproj") {
            std::fs::copy(&path, temp.path().join(path.file_name().unwrap())).unwrap();
        }
    }
    let project = temp.path().join("Shop.Core.csproj");
    assert!(project.is_file());
    assert_eq!(dotnet.version(Some(temp.path())).unwrap(), selected);
    let output = dotnet.std_command().arg("build").arg(&project).arg("--nologo").arg("--ignore-failed-sources").current_dir(temp.path()).env("DOTNET_CLI_HOME", temp.path()).output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(temp.path().join("bin/Debug/net8.0/Shop.Core.dll").is_file());
}
