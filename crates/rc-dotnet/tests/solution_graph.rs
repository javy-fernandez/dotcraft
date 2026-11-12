use std::path::Path;

use rc_dotnet::solution::Solution;
use rc_dotnet::solution_graph::SolutionGraph;

#[test]
fn shop_solution_graph_has_expected_project_dependencies() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop");
    let solution = Solution::load(&fixture.join("Shop.sln")).unwrap();
    let graph = SolutionGraph::load(&solution).unwrap();
    assert_eq!(graph.nodes.len(), 6);
    assert_eq!(graph.edges.len(), 5);
    assert!(graph.external_references.is_empty());
    assert!(graph.cycle().is_none());
    let names: Vec<_> = graph.edges.iter().map(|e| (graph.nodes[e.from].name.as_str(), graph.nodes[e.to].name.as_str())).collect();
    for from in ["Shop.Cli", "Shop.Api", "Shop.Core.XunitTests", "Shop.Core.NunitTests", "Shop.Core.MSTests"] {
        assert!(names.contains(&(from, "Shop.Core")));
    }
    let dot = graph.to_dot();
    assert!(dot.starts_with("digraph DotCraft {\n"));
    assert_eq!(dot.matches("->").count(), 5);
    assert!(dot.contains("label=\"Shop.Core\""));
}

#[test]
fn reports_cycles_and_external_references_without_following_them() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for name in ["A", "B"] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    std::fs::write(root.join("Demo.slnx"), r#"<Solution><Project Path="A/A.csproj" /><Project Path="B/B.csproj" /></Solution>"#).unwrap();
    std::fs::write(root.join("A/A.csproj"), r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><ProjectReference Include="../B/B.csproj" /><ProjectReference Include="../Missing/Missing.csproj" /></ItemGroup></Project>"#).unwrap();
    std::fs::write(root.join("B/B.csproj"), r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><ProjectReference Include="../A/A.csproj" /></ItemGroup></Project>"#).unwrap();
    let solution = Solution::load(&root.join("Demo.slnx")).unwrap();
    let mut graph = SolutionGraph::load(&solution).unwrap();
    assert_eq!(graph.edges.len(), 2);
    assert_eq!(graph.external_references.len(), 1);
    assert!(graph.external_references[0].1.ends_with("Missing/Missing.csproj"));
    assert_eq!(graph.cycle(), Some(vec![0, 1, 0]));
    graph.nodes[0].name = "A\";\nunsafe".into();
    assert!(graph.to_dot().contains("label=\"A\\\"; unsafe\""));
    assert!(!graph.to_dot().contains("\nunsafe"));
}

#[test]
fn filtered_shop_graph_tracks_references_to_excluded_projects() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop").canonicalize().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let filter = temp.path().join("Api.slnf");
    let json = serde_json::json!({"solution": {"path": fixture.join("Shop.sln"), "projects": ["src/Shop.Api/Shop.Api.csproj"]}});
    std::fs::write(&filter, json.to_string()).unwrap();
    let solution = Solution::load(&filter).unwrap();
    let graph = SolutionGraph::load(&solution).unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert!(graph.edges.is_empty());
    assert_eq!(graph.external_references.len(), 1);
    assert_eq!(graph.nodes[0].name, "Shop.Api");
    assert!(graph.external_references[0].1.ends_with("Shop.Core/Shop.Core.csproj"));
}
