//! Actual SDK build of a filter in a private fixture; never writes shared Shop output.
use std::path::PathBuf;
use std::time::Duration;

use rc_core::{Inbox, Waker};
use rc_dotnet::{BuildAction, BuildEvent, BuildRequest, Dotnet, Solution};

#[test]
#[ignore = "Requires a locally installed .NET SDK and net8.0 reference pack"]
fn solution_filter_builds_only_selected_projects_from_an_external_directory() {
    let configured = std::env::var_os("DOTCRAFT_TEST_DOTNET").map(PathBuf::from);
    let dotnet = Dotnet::locate(configured.as_deref()).expect("install the .NET SDK or set DOTCRAFT_TEST_DOTNET");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("source");
    let filters = dir.path().join("filters");
    std::fs::create_dir_all(&filters).unwrap();
    for name in ["Selected", "Excluded"] {
        let project = root.join(name);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{name}.csproj")), "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>").unwrap();
        std::fs::write(project.join("Library.cs"), if name == "Selected" { "public class Selected {}" } else { "this excluded project deliberately does not compile" }).unwrap();
    }
    // No packages are required; prevent restore from contacting configured remote sources.
    std::fs::write(dir.path().join("NuGet.Config"), "<configuration><packageSources><clear/></packageSources></configuration>").unwrap();
    let selected = "{11111111-1111-1111-1111-111111111111}";
    let excluded = "{22222222-2222-2222-2222-222222222222}";
    let project_type = "{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}";
    let mut solution = "Microsoft Visual Studio Solution File, Format Version 12.00\n".to_string();
    for (name, id) in [("Selected", selected), ("Excluded", excluded)] {
        solution.push_str(&format!("Project(\"{project_type}\") = \"{name}\", \"{name}/{name}.csproj\", \"{id}\"\nEndProject\n"));
    }
    solution.push_str("Global\nGlobalSection(SolutionConfigurationPlatforms) = preSolution\nDebug|Any CPU = Debug|Any CPU\nEndGlobalSection\nGlobalSection(ProjectConfigurationPlatforms) = postSolution\n");
    for id in [selected, excluded] {
        solution.push_str(&format!("{id}.Debug|Any CPU.ActiveCfg = Debug|Any CPU\n{id}.Debug|Any CPU.Build.0 = Debug|Any CPU\n"));
    }
    solution.push_str("EndGlobalSection\nEndGlobal\n");
    std::fs::write(root.join("Filtered.sln"), solution).unwrap();
    let filter = filters.join("Subset.slnf");
    std::fs::write(&filter, serde_json::json!({"solution": {"path": "../source/Filtered.sln", "projects": ["Selected/Selected.csproj"]}}).to_string()).unwrap();
    let version = dotnet.version(Some(&root)).unwrap();
    std::fs::write(root.join("global.json"), serde_json::json!({"sdk":{"version":version,"rollForward":"disable"}}).to_string()).unwrap();
    std::fs::write(filters.join("global.json"), r#"{"sdk":{"version":"0.0.0","rollForward":"disable"}}"#).unwrap();
    assert!(dotnet.version(Some(&filters)).is_err(), "the filter directory deliberately has an unusable SDK policy");
    let loaded = Solution::load(&filter).unwrap();
    assert_eq!(loaded.path, filter);
    assert_eq!(loaded.source_path, root.join("Filtered.sln"));
    assert_eq!(loaded.projects().len(), 1);
    let inbox = Inbox::new(Waker::noop());
    let _build = rc_dotnet::start_build(&dotnet, BuildRequest::new(&filter, BuildAction::Build), inbox.sender());
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let mut output = Vec::new();
    loop {
        assert!(std::time::Instant::now() < deadline, "build timed out: {output:?}");
        let Some(event) = inbox.recv_timeout(Duration::from_millis(100)) else { continue };
        match event {
            BuildEvent::Finished(result) => {
                assert!(result.success, "filtered SDK build failed: {result:?}\n{output:?}");
                break;
            }
            BuildEvent::Failed(error) => panic!("could not build filter: {error}"),
            other => output.push(other),
        }
    }
    assert!(root.join("Selected/bin/Debug/net8.0/Selected.dll").exists());
    assert!(!root.join("Excluded/bin").exists(), "excluded invalid project must not be built");
}
