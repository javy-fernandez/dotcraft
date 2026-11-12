#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use rc_dotnet::nuget::{add_package, remove_package, update_package};
use rc_dotnet::Dotnet;

#[tokio::test]
async fn invalid_package_commands_do_not_spawn_and_valid_commands_keep_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("Shop.Api.csproj");
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj"), &project).unwrap();
    let marker = temp.path().join("spawned");
    let host = temp.path().join("dotnet");
    std::fs::write(&host, format!("#!/bin/sh\ntouch '{}'\nprintf '%s\\n' \"$@\"\n", marker.display())).unwrap();
    let mut mode = std::fs::metadata(&host).unwrap().permissions();
    mode.set_mode(0o700);
    std::fs::set_permissions(&host, mode).unwrap();
    let dotnet = Dotnet::new(&host);

    for id in ["", "--help", "-A", ".A", "A..B", "A B", "A/Bad", "A-"] {
        assert!(add_package(&dotnet, &project, id, None, false).await.is_err(), "{id}");
        assert!(remove_package(&dotnet, &project, id).await.is_err(), "{id}");
    }
    for version in ["", "--help", "/help", " 1.0.0", "1.0.0\n--source"] {
        assert!(update_package(&dotnet, &project, "Newtonsoft.Json", version).await.is_err(), "{version}");
    }
    assert!(add_package(&dotnet, &temp.path().join("Missing.csproj"), "Newtonsoft.Json", None, false).await.is_err());
    assert!(!marker.exists(), "Invalid commands must not start the host");

    let add = add_package(&dotnet, &project, "Newtonsoft.Json", Some("13.0.3"), false).await.unwrap();
    assert!(add.success);
    assert!(marker.exists());
    assert_eq!(add.output.lines().collect::<Vec<_>>(), ["add", project.to_str().unwrap(), "package", "Newtonsoft.Json", "--version", "13.0.3"]);
    let remove = remove_package(&dotnet, &project, "Newtonsoft.Json").await.unwrap();
    assert!(remove.success);
    assert_eq!(remove.output.lines().collect::<Vec<_>>(), ["remove", project.to_str().unwrap(), "package", "Newtonsoft.Json"]);
    // NuGet ids may start with an underscore (`\w+([.-]\w+)*`).
    let underscore = add_package(&dotnet, &project, "_Internal.Tools-Core_2", None, false).await.unwrap();
    assert!(underscore.success);
    assert_eq!(underscore.output.lines().nth(3), Some("_Internal.Tools-Core_2"));
}
