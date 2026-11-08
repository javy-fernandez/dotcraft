//! Solution files: classic `.sln`, XML `.slnx`, and JSON `.slnf` filters.

use std::path::{Path, PathBuf};

/// GUID of solution folders in `.sln` files.
const FOLDER_TYPE: &str = "2150E333-8FDC-42A3-9474-1A3956D46DE8";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRef {
    pub name: String,
    /// Absolute path of the project file.
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolutionItem {
    Folder { name: String, children: Vec<SolutionItem> },
    Project(ProjectRef),
    /// A loose file of the solution ("Solution Items").
    File(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Solution {
    /// Build/test target and session identity, including a filter when one was opened.
    pub path: PathBuf,
    /// Underlying `.sln` or `.slnx` whose directory owns the workspace services.
    pub source_path: PathBuf,
    pub name: String,
    pub items: Vec<SolutionItem>,
}

impl Solution {
    pub fn load(path: &Path) -> anyhow::Result<Solution> {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("slnf")) {
            return load_filter(path);
        }
        let text = std::fs::read_to_string(path)?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let items = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("slnx")) { parse_slnx(&text, dir)? } else { parse_sln(&text, dir) };
        let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        Ok(Solution { path: path.to_path_buf(), source_path: path.to_path_buf(), name, items })
    }

    /// Every project, depth first in solution order.
    pub fn projects(&self) -> Vec<&ProjectRef> {
        fn walk<'a>(items: &'a [SolutionItem], out: &mut Vec<&'a ProjectRef>) {
            for i in items {
                match i {
                    SolutionItem::Project(p) => out.push(p),
                    SolutionItem::Folder { children, .. } => walk(children, out),
                    SolutionItem::File(_) => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.items, &mut out);
        out
    }
}

/// Filter paths are relative to the filter; project paths are relative to its solution.
/// Keep the filter as the command target so build/test continue to honor its selection.
fn load_filter(path: &Path) -> anyhow::Result<Solution> {
    use anyhow::Context;
    #[derive(serde::Deserialize)]
    struct Filter { solution: Selection }
    #[derive(serde::Deserialize)]
    struct Selection { path: String, projects: Vec<String> }

    let text = std::fs::read_to_string(path).with_context(|| format!("Could not read filter {}", path.display()))?;
    let filter: Filter = serde_json::from_str(&text).context("Invalid solution filter JSON")?;
    let source = resolve(path.parent().unwrap_or(Path::new(".")), &filter.solution.path);
    anyhow::ensure!(source.extension().is_some_and(|e| e.eq_ignore_ascii_case("sln") || e.eq_ignore_ascii_case("slnx")),
        "A solution filter must reference a .sln or .slnx file");
    let mut solution = Solution::load(&source).with_context(|| format!("Could not load filtered solution {}", source.display()))?;
    let dir = source.parent().unwrap_or(Path::new("."));
    let selected: Vec<PathBuf> = filter.solution.projects.iter().map(|p| resolve(dir, p)).collect();
    for project in &selected {
        anyhow::ensure!(solution.projects().iter().any(|p| p.path == *project),
            "Filtered project {} is not in the solution", project.display());
    }
    fn retain(items: &mut Vec<SolutionItem>, selected: &[PathBuf]) {
        items.retain_mut(|item| match item {
            SolutionItem::Project(p) => selected.contains(&p.path),
            SolutionItem::Folder { children, .. } => {
                retain(children, selected);
                !children.is_empty()
            }
            SolutionItem::File(_) => true,
        });
    }
    retain(&mut solution.items, &selected);
    solution.path = path.to_path_buf();
    solution.name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    Ok(solution)
}

/// Normalizes a path written in a solution (Windows separators) against its folder.
fn resolve(dir: &Path, rel: &str) -> PathBuf {
    let rel = rel.replace('\\', "/");
    let mut out = dir.to_path_buf();
    for part in Path::new(&rel).components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => { out.pop(); }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                out.push(part.as_os_str());
            }
            std::path::Component::Normal(p) => out.push(p),
        }
    }
    out
}

fn parse_sln(text: &str, dir: &Path) -> Vec<SolutionItem> {
    struct Entry {
        guid: String,
        item: SolutionItem,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut nested: Vec<(String, String)> = Vec::new();
    let mut in_nested = false;
    let mut in_items_of: Option<usize> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Project(\"{") {
            // Project("{TYPE}") = "Name", "Path", "{GUID}"
            let type_guid = rest.split('}').next().unwrap_or("").to_ascii_uppercase();
            let parts: Vec<&str> = t.split('"').collect();
            if parts.len() >= 8 {
                let (name, path, guid) = (parts[3], parts[5], parts[7].trim_matches(['{', '}']).to_ascii_uppercase());
                let item = if type_guid == FOLDER_TYPE {
                    SolutionItem::Folder { name: name.to_string(), children: Vec::new() }
                } else {
                    SolutionItem::Project(ProjectRef { name: name.to_string(), path: resolve(dir, path) })
                };
                entries.push(Entry { guid, item });
                in_items_of = Some(entries.len() - 1);
            }
            continue;
        }
        if t == "EndProject" {
            in_items_of = None;
            continue;
        }
        if t.starts_with("ProjectSection(SolutionItems)") {
            continue;
        }
        if let Some(i) = in_items_of
            && let Some((a, _)) = t.split_once(" = ")
            && !t.starts_with("ProjectSection")
            && !t.starts_with("EndProjectSection")
        {
            let file = SolutionItem::File(resolve(dir, a.trim()));
            if let SolutionItem::Folder { children, .. } = &mut entries[i].item {
                children.push(file);
            }
            continue;
        }
        if t.starts_with("GlobalSection(NestedProjects)") {
            in_nested = true;
            continue;
        }
        if in_nested {
            if t.starts_with("EndGlobalSection") {
                in_nested = false;
            } else if let Some((child, parent)) = t.split_once(" = ") {
                nested.push((child.trim_matches(['{', '}', ' ']).to_ascii_uppercase(), parent.trim_matches(['{', '}', ' ']).to_ascii_uppercase()));
            }
        }
    }
    // Build the tree: children move under their parent folder.
    let parent_of = |guid: &str| nested.iter().find(|(c, _)| c == guid).map(|(_, p)| p.clone());
    fn build(guid: Option<&str>, entries: &[Entry], parent_of: &dyn Fn(&str) -> Option<String>) -> Vec<SolutionItem> {
        let mut out: Vec<SolutionItem> = Vec::new();
        for e in entries {
            if parent_of(&e.guid).as_deref() != guid {
                continue;
            }
            match &e.item {
                SolutionItem::Folder { name, children } => {
                    let mut kids = build(Some(&e.guid), entries, parent_of);
                    kids.extend(children.iter().cloned());
                    out.push(SolutionItem::Folder { name: name.clone(), children: kids });
                }
                other => out.push(other.clone()),
            }
        }
        sort_items(&mut out);
        out
    }
    build(None, &entries, &parent_of)
}

/// Folders first, then projects, then files; each by name (as Rider shows them).
fn sort_items(items: &mut [SolutionItem]) {
    let key = |i: &SolutionItem| -> (u8, String) {
        match i {
            SolutionItem::Folder { name, .. } => (0, name.to_lowercase()),
            SolutionItem::Project(p) => (1, p.name.to_lowercase()),
            SolutionItem::File(f) => (2, f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()),
        }
    };
    items.sort_by_key(key);
}

fn parse_slnx(text: &str, dir: &Path) -> anyhow::Result<Vec<SolutionItem>> {
    let doc = roxmltree::Document::parse(text)?;
    let root = doc.root_element();
    // Folders are flat in the file ("/src/app/") and nest by their path.
    let mut top: Vec<SolutionItem> = Vec::new();
    fn insert(items: &mut Vec<SolutionItem>, path: &[&str], item: SolutionItem) {
        match path.split_first() {
            None => items.push(item),
            Some((first, rest)) => {
                let pos = items.iter().position(|i| matches!(i, SolutionItem::Folder { name, .. } if name == first));
                let idx = pos.unwrap_or_else(|| {
                    items.push(SolutionItem::Folder { name: first.to_string(), children: Vec::new() });
                    items.len() - 1
                });
                if let SolutionItem::Folder { children, .. } = &mut items[idx] {
                    insert(children, rest, item);
                }
            }
        }
    }
    let project = |n: roxmltree::Node| -> Option<SolutionItem> {
        let p = n.attribute("Path")?;
        let path = resolve(dir, p);
        let name = path.file_stem()?.to_string_lossy().into_owned();
        Some(SolutionItem::Project(ProjectRef { name, path }))
    };
    for n in root.children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "Project" => {
                if let Some(p) = project(n) {
                    top.push(p);
                }
            }
            "Folder" => {
                let name = n.attribute("Name").unwrap_or("/");
                let parts: Vec<&str> = name.split('/').filter(|s| !s.is_empty()).collect();
                if parts.is_empty() {
                    continue;
                }
                // Make sure empty folders exist too.
                insert(&mut top, &parts[..parts.len() - 1], SolutionItem::Folder { name: parts[parts.len() - 1].to_string(), children: Vec::new() });
                dedup_folders(&mut top);
                for c in n.children().filter(|c| c.is_element()) {
                    match c.tag_name().name() {
                        "Project" => {
                            if let Some(p) = project(c) {
                                insert(&mut top, &parts, p);
                            }
                        }
                        "File" => {
                            if let Some(p) = c.attribute("Path") {
                                insert(&mut top, &parts, SolutionItem::File(resolve(dir, p)));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    fn sort_rec(items: &mut [SolutionItem]) {
        sort_items(items);
        for i in items {
            if let SolutionItem::Folder { children, .. } = i {
                sort_rec(children);
            }
        }
    }
    sort_rec(&mut top);
    Ok(top)
}

/// Merges sibling folders with the same name (created twice while inserting).
fn dedup_folders(items: &mut Vec<SolutionItem>) {
    let mut out: Vec<SolutionItem> = Vec::new();
    for item in items.drain(..) {
        match item {
            SolutionItem::Folder { name, mut children } => {
                if let Some(SolutionItem::Folder { children: existing, .. }) =
                    out.iter_mut().find(|i| matches!(i, SolutionItem::Folder { name: n, .. } if *n == name))
                {
                    existing.append(&mut children);
                } else {
                    out.push(SolutionItem::Folder { name, children });
                }
            }
            other => out.push(other),
        }
    }
    for i in &mut out {
        if let SolutionItem::Folder { children, .. } = i {
            dedup_folders(children);
        }
    }
    *items = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLN: &str = r#"
Microsoft Visual Studio Solution File, Format Version 12.00
Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "App", "src\App\App.csproj", "{11111111-1111-1111-1111-111111111111}"
EndProject
Project("{2150E333-8FDC-42A3-9474-1A3956D46DE8}") = "src", "src", "{22222222-2222-2222-2222-222222222222}"
EndProject
Project("{9A19103F-16F7-4668-BE54-9A1E7A4F7556}") = "App.Tests", "tests\App.Tests\App.Tests.csproj", "{33333333-3333-3333-3333-333333333333}"
EndProject
Project("{2150E333-8FDC-42A3-9474-1A3956D46DE8}") = "Solution Items", "Solution Items", "{44444444-4444-4444-4444-444444444444}"
	ProjectSection(SolutionItems) = preProject
		README.md = README.md
	EndProjectSection
EndProject
Global
	GlobalSection(NestedProjects) = preSolution
		{11111111-1111-1111-1111-111111111111} = {22222222-2222-2222-2222-222222222222}
	EndGlobalSection
EndGlobal
"#;

    #[test]
    fn solution_filters_select_projects_relative_to_the_solution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("App.sln"), SLN).unwrap();
        std::fs::create_dir(dir.path().join("filters")).unwrap();
        let filter = dir.path().join("filters/Backend.slnf");
        std::fs::write(&filter, r#"{"solution":{"path":"../App.sln","projects":["src\\App\\App.csproj"]}}"#).unwrap();
        let loaded = Solution::load(&filter).unwrap();
        assert_eq!(loaded.path, filter);
        assert_eq!(loaded.source_path, dir.path().join("App.sln"));
        assert_eq!(loaded.name, "Backend");
        assert_eq!(loaded.projects().len(), 1);
        assert_eq!(loaded.projects()[0].path, dir.path().join("src/App/App.csproj"));
        assert!(matches!(&loaded.items[1], SolutionItem::Folder { name, .. } if name == "src"));
    }

    #[test]
    fn solution_filters_validate_input_and_support_slnx() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("App.slnx"), r#"<Solution><Folder Name="/src/"><Project Path="A/A.csproj" /></Folder><Project Path="B/B.csproj" /></Solution>"#).unwrap();
        let path = dir.path().join("Subset.slnf");
        for invalid in [
            r#"{"solution":{"path":"Subset.slnf","projects":[]}}"#,
            r#"{"solution":{"path":"App.slnx","projects":["Missing.csproj"]}}"#,
            r#"{"solution":{"path":"Missing.sln","projects":[]}}"#,
            r#"{"solution":{"path":"App.slnx"}}"#,
            "not json",
        ] {
            std::fs::write(&path, invalid).unwrap();
            assert!(Solution::load(&path).is_err(), "{invalid}");
        }
        std::fs::write(&path, r#"{"solution":{"path":"App.slnx","projects":["B/B.csproj","B/B.csproj"]}}"#).unwrap();
        let loaded = Solution::load(&path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.projects()[0].name, "B");
        std::fs::write(&path, r#"{"solution":{"path":"App.slnx","projects":[]}}"#).unwrap();
        assert!(Solution::load(&path).unwrap().projects().is_empty());
    }

    #[test]
    fn filters_load_real_fixture_projects() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/TenProjects");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Modules.slnf");
        let json = serde_json::json!({"solution": {
            "path": fixture.join("TenProjects.slnx"),
            "projects": ["src/Module01/Module01.csproj", "src/Module10/Module10.csproj"]
        }});
        std::fs::write(&path, json.to_string()).unwrap();
        let loaded = Solution::load(&path).unwrap();
        let projects = loaded.projects();
        assert_eq!(projects.len(), 2);
        for project in projects {
            let info = crate::ProjectInfo::load(&project.path).unwrap();
            assert_eq!(info.sdk, "Microsoft.NET.Sdk");
            assert!(!info.target_frameworks.is_empty());
        }
    }

    #[test]
    fn sln_folders_nest_projects() {
        let items = parse_sln(SLN, Path::new("/w"));
        assert_eq!(items.len(), 3);
        match &items[1] {
            SolutionItem::Folder { name, children } => {
                assert_eq!(name, "src");
                assert_eq!(children, &vec![SolutionItem::Project(ProjectRef { name: "App".into(), path: "/w/src/App/App.csproj".into() })]);
            }
            other => panic!("{other:?}"),
        }
        match &items[0] {
            SolutionItem::Folder { name, children } => {
                assert_eq!(name, "Solution Items");
                assert_eq!(children, &vec![SolutionItem::File("/w/README.md".into())]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&items[2], SolutionItem::Project(p) if p.name == "App.Tests"));
    }

    #[test]
    fn slnx_folders_nest_by_path() {
        let x = r#"<Solution>
  <Folder Name="/src/" />
  <Folder Name="/src/apps/">
    <Project Path="src/apps/Web/Web.csproj" />
  </Folder>
  <Folder Name="/tests/">
    <Project Path="tests/Web.Tests/Web.Tests.csproj" />
    <File Path="tests/README.md" />
  </Folder>
  <Project Path="tools/Tool.csproj" />
</Solution>"#;
        let items = parse_slnx(x, Path::new("/w")).unwrap();
        let names: Vec<String> = items
            .iter()
            .map(|i| match i {
                SolutionItem::Folder { name, .. } => format!("folder:{name}"),
                SolutionItem::Project(p) => format!("project:{}", p.name),
                SolutionItem::File(f) => format!("file:{}", f.display()),
            })
            .collect();
        assert_eq!(names, vec!["folder:src", "folder:tests", "project:Tool"]);
        let SolutionItem::Folder { children, .. } = &items[0] else { panic!() };
        let SolutionItem::Folder { name, children: apps } = &children[0] else { panic!("{children:?}") };
        assert_eq!(name, "apps");
        assert!(matches!(&apps[0], SolutionItem::Project(p) if p.path == Path::new("/w/src/apps/Web/Web.csproj")));
    }
}
