use rc_dotnet::{Solution, SolutionItem};
use std::path::Path;

fn folder<'a>(items: &'a [SolutionItem], name: &str) -> &'a [SolutionItem] {
    let item = items
        .iter()
        .find(|item| matches!(item, SolutionItem::Folder { name: found, .. } if found == name))
        .unwrap();
    let SolutionItem::Folder { children, .. } = item else {
        unreachable!()
    };
    children
}

#[test]
fn real_shop_and_tenprojects_solution_folders_preserve_project_membership() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let shop = Solution::load(&fixtures.join("Shop/Shop.sln")).unwrap();
    assert_eq!(shop.items.len(), 2);
    assert_eq!(folder(&shop.items, "src").len(), 3);
    assert_eq!(folder(&shop.items, "tests").len(), 3);
    assert_eq!(shop.projects().len(), 6);
    assert!(shop.projects().iter().all(|project| project.path.is_file()));

    let ten = Solution::load(&fixtures.join("TenProjects/TenProjects.slnx")).unwrap();
    assert_eq!(ten.items.len(), 1);
    assert_eq!(folder(&ten.items, "src").len(), 10);
    assert_eq!(ten.projects().len(), 10);
    assert!(ten.projects().iter().all(|project| project.path.is_file()));
}

#[test]
fn classic_solution_items_ignore_other_sections_and_stray_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let solution = dir.path().join("App.sln");
    std::fs::write(dir.path().join("README.md"), "# App\n").unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\" />\n",
    )
    .unwrap();
    std::fs::write(&solution, r#"
Microsoft Visual Studio Solution File, Format Version 12.00
Project("{2150E333-8FDC-42A3-9474-1A3956D46DE8}") = "Docs", "Docs", "{11111111-1111-1111-1111-111111111111}"
    ProjectSection(SolutionItems) = preProject
        README.md = README.md
    EndProjectSection
    ProjectSection(WebsiteProperties) = preProject
        Fake.cs = Fake.cs
    EndProjectSection
    Stray.cs = Stray.cs
EndProject
Project("{2150E333-8FDC-42A3-9474-1A3956D46DE8}") = "Nested", "Nested", "{22222222-2222-2222-2222-222222222222}"
EndProject
Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "App", "src\App.csproj", "{33333333-3333-3333-3333-333333333333}"
EndProject
Global
    GlobalSection(NestedProjects) = preSolution
        {22222222-2222-2222-2222-222222222222} = {11111111-1111-1111-1111-111111111111}
        {33333333-3333-3333-3333-333333333333} = {22222222-2222-2222-2222-222222222222}
    EndGlobalSection
EndGlobal
"#).unwrap();

    let loaded = Solution::load(&solution).unwrap();
    let docs = folder(&loaded.items, "Docs");
    assert_eq!(docs.len(), 2);
    assert!(docs.iter().any(
        |item| matches!(item, SolutionItem::File(path) if path == &dir.path().join("README.md"))
    ));
    let nested = folder(docs, "Nested");
    assert_eq!(nested.len(), 1);
    assert!(
        matches!(&nested[0], SolutionItem::Project(project) if project.path == dir.path().join("src/App.csproj"))
    );
    assert_eq!(loaded.projects().len(), 1);
}
