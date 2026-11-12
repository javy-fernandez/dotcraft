//! Bounded text edits for SDK-style project references within one workspace.

use std::path::{Component, Path, PathBuf};

fn canonical_paths(project: &Path, target: &Path, workspace_root: &Path) -> anyhow::Result<(PathBuf, PathBuf, PathBuf)> {
    let root = dunce::canonicalize(workspace_root)?;
    let project = dunce::canonicalize(project)?;
    let target = dunce::canonicalize(target)?;
    anyhow::ensure!(project.is_file() && target.is_file(), "Project references require existing project files");
    anyhow::ensure!(project.starts_with(&root) && target.starts_with(&root), "Project reference is outside the workspace");
    anyhow::ensure!(project != target, "A project cannot reference itself");
    for path in [&project, &target] {
        anyhow::ensure!(path.extension().is_some_and(|e| ["csproj", "fsproj", "vbproj"].iter().any(|ext| e.eq_ignore_ascii_case(ext))), "Project references require .csproj, .fsproj or .vbproj files");
    }
    Ok((project, target, root))
}

fn parse_project(text: &str) -> anyhow::Result<(roxmltree::Document<'_>, usize)> {
    let source = text.strip_prefix('\u{feff}').unwrap_or(text);
    let doc = roxmltree::Document::parse(source)?;
    let root = doc.root_element();
    anyhow::ensure!(root.has_tag_name("Project") && (root.attribute("Sdk").is_some() || root.children().any(|n| n.has_tag_name("Sdk"))), "An SDK-style project is required");
    Ok((doc, text.len() - source.len()))
}

fn relative_include(project: &Path, target: &Path) -> anyhow::Result<String> {
    let base: Vec<Component<'_>> = project.parent().unwrap().components().collect();
    let dest: Vec<Component<'_>> = target.components().collect();
    let common = base.iter().zip(&dest).take_while(|(a, b)| a == b).count();
    let mut parts = vec!["..".to_string(); base.len() - common];
    for component in &dest[common..] {
        parts.push(component.as_os_str().to_str().ok_or_else(|| anyhow::anyhow!("Project path must be UTF-8"))?.to_string());
    }
    Ok(parts.join("\\"))
}

fn references_target(node: roxmltree::Node<'_, '_>, project: &Path, target: &Path) -> bool {
    let Some(include) = node.attribute("Include") else { return false };
    let path = project.parent().unwrap().join(include.replace('\\', "/"));
    dunce::canonicalize(path).is_ok_and(|path| path == target)
}

fn escape_attr(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}

/// Adds a project reference without writing the file. Paths must resolve inside `workspace_root`.
pub fn add_project_reference(text: &str, project: &Path, target: &Path, workspace_root: &Path) -> anyhow::Result<String> {
    let (project, target, _) = canonical_paths(project, target, workspace_root)?;
    let (doc, offset) = parse_project(text)?;
    let root = doc.root_element();
    anyhow::ensure!(!doc.descendants().filter(|n| n.has_tag_name("ProjectReference")).any(|n| references_target(n, &project, &target)), "The project already references the target");
    let include = escape_attr(&relative_include(&project, &target)?);
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let element = format!("<ProjectReference Include=\"{include}\" />");

    for group in root.children().filter(|n| n.has_tag_name("ItemGroup") && n.attribute("Condition").is_none()) {
        let refs: Vec<_> = group.children().filter(|n| n.has_tag_name("ProjectReference") && n.attribute("Condition").is_none()).collect();
        if refs.is_empty() { continue; }
        let group_range = group.range();
        let group_text = &text[group_range.start + offset..group_range.end + offset];
        let Some(close_rel) = group_text.rfind("</ItemGroup>") else { continue };
        let close = group_range.start + offset + close_rel;
        let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
        if !text[line_start..close].trim().is_empty() { continue; }
        let last = refs.last().unwrap();
        let ref_start = last.range().start + offset;
        let ref_line = text[..ref_start].rfind('\n').map_or(0, |i| i + 1);
        let indent = if text[ref_line..ref_start].trim().is_empty() { &text[ref_line..ref_start] } else { "    " };
        let mut edited = text.to_string();
        edited.insert_str(line_start, &format!("{indent}{element}{newline}"));
        return Ok(edited);
    }

    let range = root.range();
    let root_text = &text[range.start + offset..range.end + offset];
    let close_rel = root_text.rfind("</Project>").ok_or_else(|| anyhow::anyhow!("Project must have a closing tag"))?;
    let close = range.start + offset + close_rel;
    let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
    let insertion = if text[line_start..close].trim().is_empty() { line_start } else { close };
    let block = format!("  <ItemGroup>{newline}    {element}{newline}  </ItemGroup>{newline}{newline}");
    let mut edited = text.to_string();
    edited.insert_str(insertion, &block);
    Ok(edited)
}

/// Removes every reference to an existing target project without writing the file.
pub fn remove_project_reference(text: &str, project: &Path, target: &Path, workspace_root: &Path) -> anyhow::Result<String> {
    let (project, target, _) = canonical_paths(project, target, workspace_root)?;
    let (doc, offset) = parse_project(text)?;
    let mut ranges: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("ProjectReference") && references_target(*n, &project, &target)).map(|n| {
        let range = n.range();
        let start = range.start + offset;
        let end = range.end + offset;
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i + 1);
        if text[line_start..start].trim().is_empty() && text[end..line_end].trim().is_empty() { line_start..line_end } else { start..end }
    }).collect();
    anyhow::ensure!(!ranges.is_empty(), "The project does not reference the target");
    ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut edited = text.to_string();
    for range in ranges { edited.replace_range(range, ""); }
    Ok(edited)
}
