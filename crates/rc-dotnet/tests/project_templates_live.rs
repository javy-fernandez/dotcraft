//! Live SDK acceptance for project templates. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test project_templates_live -- --ignored`.

use std::path::Path;

use rc_dotnet::new_project::{NewProject, Target};
use rc_dotnet::{Dotnet, Solution};

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

fn build_project(dotnet: &Dotnet, project: &Path) {
    let output = dotnet
        .std_command()
        .arg("build")
        .arg(project)
        .args(["-c", "Debug", "-nologo", "-v:minimal"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "generated project did not build:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "requires a locally installed .NET 8 SDK; set DOTCRAFT_TEST_DOTNET"]
async fn creates_projects_in_real_and_new_solutions() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let dir = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop");
    let shop = dir.path().join("Shop");
    copy_tree(&fixture, &shop);
    let solution = shop.join("Shop.sln");
    let before = Solution::load(&solution).unwrap();
    assert_eq!(before.projects().len(), 6);

    let added = NewProject {
        template: "classlib",
        project_name: "Shop.Added".into(),
        framework: Some("net8.0".into()),
        use_controllers: false,
        target: Target::AddTo {
            solution: solution.clone(),
        },
    };
    let mut commands = Vec::new();
    let returned = added
        .create(&dotnet, &mut |command| commands.push(command))
        .await
        .unwrap();
    assert_eq!(returned, solution);
    assert_eq!(commands.len(), 2);
    assert!(commands[0].contains("dotnet new classlib"));
    assert!(commands[1].contains("dotnet sln"));
    assert!(added.project_file().is_file());
    assert!(added.project_dir().join("Class1.cs").is_file());
    let after = Solution::load(&solution).unwrap();
    assert_eq!(after.projects().len(), 7);
    assert!(
        after
            .projects()
            .iter()
            .any(|project| project.path == added.project_file())
    );
    build_project(&dotnet, &added.project_file());
    assert!(
        added
            .project_dir()
            .join("bin/Debug/net8.0/Shop.Added.dll")
            .is_file()
    );

    let mut duplicate_commands = Vec::new();
    assert!(
        added
            .create(&dotnet, &mut |command| duplicate_commands.push(command))
            .await
            .is_err()
    );
    assert!(duplicate_commands.is_empty());

    let fresh = NewProject {
        template: "console",
        project_name: "Fresh.Cli".into(),
        framework: Some("net8.0".into()),
        use_controllers: false,
        target: Target::NewSolution {
            location: dir.path().to_path_buf(),
            solution_name: "FreshShop".into(),
            git: false,
        },
    };
    let mut commands = Vec::new();
    let fresh_solution = fresh
        .create(&dotnet, &mut |command| commands.push(command))
        .await
        .unwrap();
    assert!(fresh_solution.ends_with("FreshShop.sln"));
    assert!(fresh_solution.is_file());
    assert_eq!(commands.len(), 3);
    assert!(commands[0].contains("dotnet new sln"));
    assert!(commands[1].contains("dotnet new console"));
    assert!(commands[2].contains("dotnet sln add"));
    assert!(
        Solution::load(&fresh_solution)
            .unwrap()
            .projects()
            .iter()
            .any(|project| project.path == fresh.project_file())
    );
    assert!(fresh.project_dir().join("Program.cs").is_file());
    build_project(&dotnet, &fresh.project_file());
    let dll = fresh.project_dir().join("bin/Debug/net8.0/Fresh.Cli.dll");
    let output = dotnet.std_command().arg(&dll).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Hello, World!"));
}
