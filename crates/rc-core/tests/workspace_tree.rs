use rc_core::workspace::{Workspace, WorkspaceKind, list_dir, list_dir_with};
use std::path::Path;

#[test]
fn shop_solution_and_source_tree_open_from_a_real_dotnet_fixture() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop");
    let root = dunce::canonicalize(&fixture).unwrap();
    let workspace = Workspace::open(&fixture).unwrap();
    assert_eq!(workspace.root, root);
    assert_eq!(
        workspace.kind,
        WorkspaceKind::Solution(root.join("Shop.sln"))
    );
    assert_eq!(workspace.name(), "Shop");
    assert_eq!(workspace.key_path(), root.join("Shop.sln"));

    let top = list_dir(&root);
    assert!(top.iter().any(|node| node.name == "src" && node.is_dir));
    assert!(top.iter().any(|node| node.name == "tests" && node.is_dir));
    assert!(
        top.iter()
            .any(|node| node.name == "Shop.sln" && !node.is_dir)
    );
    let api = list_dir(&root.join("src"))
        .into_iter()
        .find(|node| node.name == "Shop.Api")
        .unwrap();
    assert!(api.is_dir);
    assert!(
        list_dir(&api.path)
            .iter()
            .any(|node| node.name == "Shop.Api.csproj" && !node.is_dir)
    );
}

#[cfg(unix)]
#[test]
fn internal_directory_links_are_navigable_without_cycles_or_outside_traversal() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/Program.cs"), "class Program {}\n").unwrap();
    symlink(root.join("src"), root.join("alias_src")).unwrap();
    symlink(&root, root.join("cycle")).unwrap();
    symlink(outside.path(), root.join("outside")).unwrap();
    symlink(outside.path(), root.join(".git")).unwrap();

    let nodes = list_dir(&root);
    assert!(
        nodes
            .iter()
            .any(|node| node.name == "alias_src" && node.is_dir)
    );
    assert!(
        nodes
            .iter()
            .any(|node| node.name == "cycle" && !node.is_dir)
    );
    assert!(
        nodes
            .iter()
            .any(|node| node.name == "outside" && !node.is_dir)
    );
    assert!(!nodes.iter().any(|node| node.name == ".git"));
    assert!(
        !list_dir_with(&root, true)
            .iter()
            .any(|node| node.name == ".git")
    );
    assert!(
        list_dir(&root.join("alias_src"))
            .iter()
            .any(|node| node.name == "Program.cs")
    );
}
