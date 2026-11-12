use std::path::{Path, PathBuf};

use rc_dotnet::solution::Solution;
use rc_dotnet::solution_filter_edit::{create_filter, update_filter};

fn ten_projects() -> (tempfile::TempDir, PathBuf, Vec<PathBuf>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("TenProjects");
    std::fs::create_dir(&root).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/TenProjects");
    let source = root.join("TenProjects.slnx");
    std::fs::copy(fixture.join("TenProjects.slnx"), &source).unwrap();
    let mut projects = Vec::new();
    for number in 1..=10 {
        let name = format!("Module{number:02}");
        let dir = root.join("src").join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join(format!("{name}.csproj"));
        std::fs::copy(fixture.join("src").join(&name).join(format!("{name}.csproj")), &project).unwrap();
        projects.push(project);
    }
    (temp, source, projects)
}

#[test]
fn creates_and_updates_ten_projects_filter_with_solution_relative_membership() {
    let (temp, source, projects) = ten_projects();
    let dir = temp.path().join("TenProjects/filters");
    std::fs::create_dir(&dir).unwrap();
    let filter = dir.join("Modules.slnf");
    let created = create_filter(&source, &filter, &[projects[0].clone(), projects[9].clone()], temp.path()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&created).unwrap();
    assert_eq!(value["solution"]["path"], "../TenProjects.slnx");
    assert_eq!(value["solution"]["projects"], serde_json::json!(["src/Module01/Module01.csproj", "src/Module10/Module10.csproj"]));
    std::fs::write(&filter, &created).unwrap();
    let loaded = Solution::load(&filter).unwrap();
    assert_eq!(loaded.projects().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Module01", "Module10"]);

    let mut with_metadata = value;
    with_metadata["metadata"] = serde_json::json!({"owner": "team"});
    with_metadata["solution"]["note"] = serde_json::json!("keep");
    let edited = update_filter(&format!("\u{feff}{}", with_metadata), &filter, &[projects[4].clone()], temp.path()).unwrap();
    assert!(edited.starts_with('\u{feff}'));
    let parsed: serde_json::Value = serde_json::from_str(edited.trim_start_matches('\u{feff}')).unwrap();
    assert_eq!(parsed["metadata"]["owner"], "team");
    assert_eq!(parsed["solution"]["note"], "keep");
    assert_eq!(parsed["solution"]["projects"], serde_json::json!(["src/Module05/Module05.csproj"]));
    std::fs::write(&filter, edited).unwrap();
    assert_eq!(Solution::load(&filter).unwrap().projects()[0].name, "Module05");
}

#[test]
fn rejects_projects_outside_the_source_solution_or_workspace() {
    let (temp, source, projects) = ten_projects();
    let root = temp.path().join("TenProjects");
    let filter = root.join("Subset.slnf");
    assert!(create_filter(&source, &filter, &[projects[0].clone(), projects[0].clone()], &root).is_err());
    let other = root.join("Other.csproj");
    std::fs::write(&other, "<Project />").unwrap();
    assert!(create_filter(&source, &filter, &[other], &root).is_err());
    let outside = temp.path().join("Outside.csproj");
    std::fs::write(&outside, "<Project />").unwrap();
    assert!(create_filter(&source, &filter, &[outside], &root).is_err());
    assert!(create_filter(&source, &temp.path().join("Outside.slnf"), &[], &root).is_err());
    assert!(update_filter(r#"{"solution":{"path":"../Outside.sln","projects":[]}}"#, &filter, &[], &root).is_err());
}
