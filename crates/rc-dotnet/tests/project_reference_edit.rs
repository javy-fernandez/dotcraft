use std::path::{Path, PathBuf};

use rc_dotnet::project::ProjectInfo;
use rc_dotnet::project_references::{add_project_reference, remove_project_reference};

fn copy_shop() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Shop");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src");
    for name in ["Shop.Api", "Shop.Cli", "Shop.Core"] {
        let dir = root.join("src").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(source.join(name).join(format!("{name}.csproj")), dir.join(format!("{name}.csproj"))).unwrap();
    }
    let api = root.join("src/Shop.Api/Shop.Api.csproj");
    let cli = root.join("src/Shop.Cli/Shop.Cli.csproj");
    let core = root.join("src/Shop.Core/Shop.Core.csproj");
    (temp, api, cli, core)
}

#[test]
fn adds_and_removes_references_in_real_shop_projects() {
    let (_temp, api, cli, core) = copy_shop();
    let root = api.parent().unwrap().parent().unwrap().parent().unwrap();
    let source = std::fs::read_to_string(&api).unwrap();
    assert!(add_project_reference(&source, &api, &core, root).is_err(), "Existing reference must not be duplicated");

    let added = add_project_reference(&source, &api, &cli, root).unwrap();
    assert!(added.contains("<ProjectReference Include=\"..\\Shop.Cli\\Shop.Cli.csproj\" />"));
    assert!(added.contains("<PackageReference Include=\"Swashbuckle.AspNetCore\""));
    let info = ProjectInfo::parse(&added, &api).unwrap();
    assert_eq!(info.project_refs.len(), 2);
    assert!(info.project_refs.contains(&cli));
    assert!(info.project_refs.contains(&core));

    let removed = remove_project_reference(&added, &api, &cli, root).unwrap();
    assert_eq!(removed, source);
    assert!(remove_project_reference(&source, &api, &cli, root).is_err());

    let core_source = std::fs::read_to_string(&core).unwrap();
    let tools = root.join("src/Shop.Tools/Shop.Tools.csproj");
    std::fs::create_dir_all(tools.parent().unwrap()).unwrap();
    std::fs::write(&tools, r#"<Project Sdk="Microsoft.NET.Sdk" />"#).unwrap();
    let core_added = add_project_reference(&core_source, &core, &tools, root).unwrap();
    assert!(core_added.starts_with('\u{feff}'));
    assert_eq!(ProjectInfo::parse(core_added.trim_start_matches('\u{feff}'), &core).unwrap().project_refs, [tools]);
}

#[test]
fn rejects_self_outside_and_non_project_targets() {
    let (temp, api, cli, _core) = copy_shop();
    let root = temp.path().join("Shop");
    let source = std::fs::read_to_string(&api).unwrap();
    assert!(add_project_reference(&source, &api, &api, &root).is_err());
    let outside = temp.path().join("Outside.csproj");
    std::fs::write(&outside, "<Project />").unwrap();
    assert!(add_project_reference(&source, &api, &outside, &root).is_err());
    let text = cli.with_extension("txt");
    std::fs::write(&text, "not a project").unwrap();
    assert!(add_project_reference(&source, &api, &text, &root).is_err());
    #[cfg(unix)] {
        use std::os::unix::fs::symlink;
        let linked = root.join("src/Shop.Api/Outside.csproj");
        symlink(&outside, &linked).unwrap();
        assert!(add_project_reference(&source, &api, &linked, &root).is_err());
    }
}
