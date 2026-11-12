//! Central Package Management against the real SDK and `tests/fixtures/CentralShop`: MSBuild
//! detection, then central file and project edits that `dotnet restore` must still accept.

use std::path::{Path, PathBuf};

use rc_dotnet::Dotnet;
use rc_dotnet::cpm::{self, CpmIssue, SolutionCpm, TextEdit};

fn dotnet() -> Dotnet {
    Dotnet::locate(None).expect("the dotnet SDK must be installed")
}

/// Copies a folder without `bin/` and `obj/`.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if name == "bin" || name == "obj" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &to.join(&name));
        } else {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

fn edit(path: &Path, f: impl FnOnce(&str) -> anyhow::Result<Vec<TextEdit>>) {
    let text = std::fs::read_to_string(path).unwrap();
    let edits = f(&text).unwrap();
    std::fs::write(path, cpm::apply_edits(&text, &edits)).unwrap();
}

fn restore(dotnet: &Dotnet, sln: &Path) {
    let out = std::process::Command::new(&dotnet.path).arg("restore").arg(sln).current_dir(sln.parent().unwrap()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "restore failed:\n{text}");
    assert!(!text.contains("error NU"), "{text}");
}

#[test]
fn central_shop_projects_restore_and_evaluate_with_sdk8() {
    let tmp = tempfile::Builder::new().prefix("dotcraft-cpm-sdk8-").tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("CentralShop");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/CentralShop"), &root);
    let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
    let cli = root.join("src/CentralShop.Cli/CentralShop.Cli.csproj");
    let dotnet = dotnet();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let list: Vec<_> = rt.block_on(cpm::detect_all(&dotnet, vec![core.clone(), cli.clone()], 2)).into_iter().map(|(_, result)| result.unwrap()).collect();
    let solution = SolutionCpm::new(list);
    let core_cpm = solution.project(&core).unwrap();
    let cli_cpm = solution.project(&cli).unwrap();
    assert!(core_cpm.enabled && cli_cpm.enabled && core_cpm.evaluated && cli_cpm.evaluated);
    assert_eq!(core_cpm.effective_version("Newtonsoft.Json").as_deref(), Some("13.0.1"));
    assert_eq!(cli_cpm.effective_version("Newtonsoft.Json").as_deref(), Some("13.0.3"));
    for project in [&core, &cli] {
        let output = dotnet.std_command().arg("restore").arg(project).arg("--nologo").current_dir(&root).output().unwrap();
        assert!(output.status.success(), "{}: {}\n{}", project.display(), String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn detects_and_edits_central_package_management() {
    let tmp = tempfile::Builder::new().prefix("dotcraft-cpm-").tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("CentralShop");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/CentralShop"), &root);
    let central = root.join("Directory.Packages.props");
    let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
    let cli = root.join("src/CentralShop.Cli/CentralShop.Cli.csproj");
    let sln = root.join("CentralShop.slnx");
    let dotnet = dotnet();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let detect = |projects: Vec<PathBuf>| {
        let mut list: Vec<_> = rt.block_on(cpm::detect_all(&dotnet, projects, 2)).into_iter().map(|(_, r)| r.unwrap()).collect();
        list.sort_by(|a, b| a.project.cmp(&b.project));
        SolutionCpm::new(list)
    };

    // CPM comes from Directory.Build.props: MSBuild sees it.
    let sol = detect(vec![core.clone(), cli.clone()]);
    let (c, l) = (sol.project(&core).unwrap(), sol.project(&cli).unwrap());
    assert!(c.enabled && l.enabled && c.evaluated);
    assert_eq!(c.central_file.as_deref(), Some(central.as_path()));
    assert_eq!(c.effective_version("Newtonsoft.Json").as_deref(), Some("13.0.1"));
    assert_eq!(l.reference("Newtonsoft.Json").and_then(|r| r.version_override.as_deref()), Some("13.0.3"));
    assert_eq!(l.effective_version("Newtonsoft.Json").as_deref(), Some("13.0.3"));
    assert!(c.is_central("Humanizer.Core"));
    assert_eq!(sol.issues(), [CpmIssue::UnusedVersion { file: central.clone(), id: "Dapper".into() }]);
    restore(&dotnet, &sln);

    // Move the override to central, install Dapper in the CLI, add a new package to the core.
    edit(&central, |t| cpm::set_central_version(t, "Newtonsoft.Json", None, "13.0.3"));
    edit(&cli, |t| cpm::remove_version_override(t, "Newtonsoft.Json"));
    edit(&cli, |t| cpm::add_reference(t, "Dapper"));
    edit(&central, |t| cpm::add_central_version(t, "Microsoft.Extensions.Logging.Abstractions", "8.0.0"));
    edit(&core, |t| cpm::add_reference(t, "Microsoft.Extensions.Logging.Abstractions"));
    let text = std::fs::read_to_string(&central).unwrap();
    assert!(text.contains("<!-- Versions shared by every project -->"));
    assert!(text.contains("    <PackageVersion Include=\"Humanizer.Core\" Version=\"2.14.1\" />\n    <PackageVersion Include=\"Microsoft.Extensions.Logging.Abstractions\" Version=\"8.0.0\" />\n    <PackageVersion Include=\"Newtonsoft.Json\" Version=\"13.0.3\" />"));
    restore(&dotnet, &sln);
    let sol = detect(vec![core.clone(), cli.clone()]);
    assert!(sol.issues().is_empty(), "{:?}", sol.issues());
    assert_eq!(sol.project(&cli).unwrap().effective_version("Newtonsoft.Json").as_deref(), Some("13.0.3"));
    assert_eq!(sol.project(&cli).unwrap().effective_version("Dapper").as_deref(), Some("2.1.66"));

    // Remove Humanizer from the core: nothing uses it, so its version goes too.
    edit(&core, |t| cpm::remove_reference(t, "Humanizer.Core"));
    let sol = detect(vec![core.clone(), cli.clone()]);
    assert!(!sol.is_used(&central, "Humanizer.Core"));
    edit(&central, |t| cpm::remove_central_version(t, "Humanizer.Core"));
    // The source no longer compiles without Humanizer, but restore must pass.
    restore(&dotnet, &sln);
    assert!(detect(vec![core, cli]).issues().is_empty());
}

#[test]
fn migrates_a_solution_to_central_package_management() {
    let tmp = tempfile::Builder::new().prefix("dotcraft-cpm-migrate-").tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("Shop");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop"), &root);
    let sln = root.join("Shop.sln");
    let projects: Vec<PathBuf> = [
        "src/Shop.Core/Shop.Core.csproj",
        "src/Shop.Cli/Shop.Cli.csproj",
        "src/Shop.Api/Shop.Api.csproj",
        "tests/Shop.Core.XunitTests/Shop.Core.XunitTests.csproj",
        "tests/Shop.Core.NunitTests/Shop.Core.NunitTests.csproj",
        "tests/Shop.Core.MSTests/Shop.Core.MSTests.csproj",
    ]
    .iter()
    .map(|p| root.join(p))
    .collect();
    // Files are read as the editor reads them (without BOM).
    let files: Vec<(PathBuf, String)> = cpm::migration_files(&root, &projects)
        .into_iter()
        .map(|p| {
            let t = std::fs::read_to_string(&p).unwrap();
            (p, t.trim_start_matches('\u{feff}').to_string())
        })
        .collect();
    let plan = cpm::plan_migration(&root, &files, &Default::default()).unwrap();
    assert!(plan.conflicts.is_empty() && plan.warnings.is_empty(), "{:?} {:?}", plan.conflicts, plan.warnings);
    assert_eq!(plan.versions.len(), 9, "{:?}", plan.versions);
    assert_eq!(plan.changes.len(), 4);
    std::fs::write(&plan.central_file, &plan.central_text).unwrap();
    for c in &plan.changes {
        let bom = std::fs::read(&c.path).unwrap().starts_with(&[0xEF, 0xBB, 0xBF]);
        std::fs::write(&c.path, format!("{}{}", if bom { "\u{feff}" } else { "" }, c.new)).unwrap();
    }
    let dotnet = dotnet();
    restore(&dotnet, &sln);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let list: Vec<_> = rt.block_on(cpm::detect_all(&dotnet, projects.clone(), 3)).into_iter().map(|(_, r)| r.unwrap()).collect();
    let sol = SolutionCpm::new(list);
    assert!(sol.projects.iter().all(|p| p.enabled), "every project uses CPM");
    assert!(sol.issues().is_empty(), "{:?}", sol.issues());
    let xunit = sol.project(&projects[3]).unwrap();
    assert_eq!(xunit.effective_version("xunit").as_deref(), Some("2.5.3"));
    assert!(xunit.is_central("Microsoft.NET.Test.Sdk"));
}
