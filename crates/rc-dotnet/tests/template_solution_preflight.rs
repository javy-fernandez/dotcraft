use rc_dotnet::new_project::{NewProject, Target};
use std::path::{Path, PathBuf};

fn request(solution: PathBuf) -> NewProject {
    NewProject {
        template: "console",
        project_name: "CodexPreview".into(),
        framework: None,
        use_controllers: false,
        target: Target::AddTo { solution },
    }
}

#[test]
fn real_classic_and_xml_solutions_produce_valid_add_to_plans() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    for solution in [
        fixtures.join("Shop/Shop.sln"),
        fixtures.join("TenProjects/TenProjects.slnx"),
    ] {
        let project = request(solution.clone());
        assert_eq!(project.problem(), None);
        let commands = project.commands();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].1[..2], ["new", "console"]);
        assert_eq!(
            commands[1].1[..3],
            ["sln", solution.to_str().unwrap(), "add"]
        );
        assert_eq!(commands[1].1[3], project.project_file().to_string_lossy());
        assert!(!project.project_dir().exists());
    }
}

#[test]
fn invalid_add_to_targets_are_rejected_before_any_project_directory_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let existing_filter = dir.path().join("Filtered.slnf");
    std::fs::write(
        &existing_filter,
        r#"{"solution":{"path":"Base.sln","projects":[]}}"#,
    )
    .unwrap();
    let directory_with_extension = dir.path().join("Folder.slnx");
    std::fs::create_dir(&directory_with_extension).unwrap();
    for solution in [
        dir.path().join("Missing.sln"),
        existing_filter,
        directory_with_extension,
        dir.path().join("Other.txt"),
    ] {
        let project = request(solution);
        assert_eq!(
            project.problem(),
            Some("Select an existing .sln or .slnx solution")
        );
        assert!(!project.project_dir().exists());
    }

    let solution = dir.path().join("Valid.SLNX");
    std::fs::write(&solution, "<Solution />\n").unwrap();
    let project = request(solution);
    assert_eq!(project.problem(), None);
    std::fs::create_dir(project.project_dir()).unwrap();
    assert_eq!(
        project.problem(),
        Some("A folder with that name already exists")
    );
}
