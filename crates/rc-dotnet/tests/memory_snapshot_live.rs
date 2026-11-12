//! Live Linux SDK acceptance for EventPipe memory snapshots. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test memory_snapshot_live -- --ignored`.

use std::path::Path;
use std::process::{Child, Stdio};
use std::time::Duration;

use rc_dotnet::Dotnet;
use rc_dotnet::gcdump::snapshot;

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
            std::fs::copy(&path, target).unwrap();
        }
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires a locally installed .NET 8 SDK and Shop.Api NuGet packages"]
fn captures_live_shop_api_heap_types() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src");
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    copy_tree(&fixture.join("Shop.Core"), &source.join("Shop.Core"));
    copy_tree(&fixture.join("Shop.Api"), &source.join("Shop.Api"));
    let project = source.join("Shop.Api/Shop.Api.csproj");
    let output = dotnet
        .std_command()
        .arg("build")
        .arg(&project)
        .args(["-c", "Debug", "-nologo", "-v:minimal"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Shop.Api build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let dll = source.join("Shop.Api/bin/Debug/net8.0/Shop.Api.dll");
    assert!(dll.is_file());
    let log_path = dir.path().join("api.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let child = dotnet
        .std_command()
        .arg(&dll)
        .current_dir(dll.parent().unwrap())
        .env("ASPNETCORE_URLS", "http://127.0.0.1:0")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    let mut process = ChildGuard(child);

    let heap = snapshot(process.0.id(), Duration::from_secs(60)).unwrap_or_else(|error| {
        panic!(
            "Shop.Api snapshot failed: {error:#}; process exit: {:?}; output:\n{}",
            process.0.try_wait().unwrap(),
            std::fs::read_to_string(&log_path).unwrap_or_default()
        )
    });
    assert!(
        process.0.try_wait().unwrap().is_none(),
        "Shop.Api exited during capture"
    );
    assert!(heap.total_objects > 0 && heap.total_bytes > 0);
    assert!(!heap.types.is_empty());
    assert!(
        heap.types
            .iter()
            .any(|item| item.name == "System.String" && item.count > 0)
    );
    assert!(
        heap.types
            .windows(2)
            .all(|pair| pair[0].size_bytes >= pair[1].size_bytes)
    );
    assert_eq!(
        heap.total_objects,
        heap.types.iter().map(|item| item.count).sum::<u64>()
    );
    assert_eq!(
        heap.total_bytes,
        heap.types.iter().map(|item| item.size_bytes).sum::<u64>()
    );
}
