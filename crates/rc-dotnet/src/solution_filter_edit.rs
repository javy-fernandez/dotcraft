//! Validated `.slnf` JSON creation and editing without writing files.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::solution::Solution;

fn filter_dir(filter: &Path, workspace_root: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    anyhow::ensure!(filter.extension().is_some_and(|e| e.eq_ignore_ascii_case("slnf")), "Filter path must end in .slnf");
    let root = dunce::canonicalize(workspace_root)?;
    let dir = dunce::canonicalize(filter.parent().unwrap_or(Path::new(".")))?;
    anyhow::ensure!(dir.starts_with(&root), "Filter is outside the workspace");
    if std::fs::symlink_metadata(filter).is_ok() {
        anyhow::ensure!(dunce::canonicalize(filter)?.starts_with(&root), "Filter is outside the workspace");
    }
    Ok((dir, root))
}

fn relative(base: &Path, target: &Path) -> anyhow::Result<String> {
    let from: Vec<Component<'_>> = base.components().collect();
    let to: Vec<Component<'_>> = target.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts = vec!["..".to_string(); from.len() - common];
    for component in &to[common..] {
        parts.push(component.as_os_str().to_str().ok_or_else(|| anyhow::anyhow!("Filter paths must be UTF-8"))?.to_string());
    }
    Ok(parts.join("/"))
}

fn selected_paths(source: &Path, selected: &[PathBuf], root: &Path) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(selected.len() <= 10000, "A solution filter cannot select more than 10000 projects");
    anyhow::ensure!(source.starts_with(root) && source.extension().is_some_and(|e| e.eq_ignore_ascii_case("sln") || e.eq_ignore_ascii_case("slnx")), "Source must be a .sln or .slnx inside the workspace");
    let solution = Solution::load(source)?;
    let projects: Vec<_> = solution.projects().iter().map(|p| dunce::canonicalize(&p.path)).collect::<Result<_, _>>()?;
    let mut paths = Vec::new();
    let mut seen = Vec::new();
    for project in selected {
        let project = dunce::canonicalize(project)?;
        anyhow::ensure!(project.starts_with(root) && projects.contains(&project), "Selected project is not in the workspace solution: {}", project.display());
        anyhow::ensure!(!seen.contains(&project), "Duplicate project in solution filter: {}", project.display());
        seen.push(project.clone());
        paths.push(relative(source.parent().unwrap(), &project)?);
    }
    Ok(paths)
}

/// Creates `.slnf` JSON for projects in an existing source solution.
pub fn create_filter(source: &Path, filter: &Path, selected: &[PathBuf], workspace_root: &Path) -> anyhow::Result<String> {
    let (dir, root) = filter_dir(filter, workspace_root)?;
    let source = dunce::canonicalize(source)?;
    let projects = selected_paths(&source, selected, &root)?;
    let path = relative(&dir, &source)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&json!({"solution": {"path": path, "projects": projects}}))?))
}

/// Changes the selected projects of an existing `.slnf`, preserving unknown JSON fields.
pub fn update_filter(text: &str, filter: &Path, selected: &[PathBuf], workspace_root: &Path) -> anyhow::Result<String> {
    let (dir, root) = filter_dir(filter, workspace_root)?;
    let mut value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
    let solution = value.get_mut("solution").and_then(Value::as_object_mut).ok_or_else(|| anyhow::anyhow!("Filter must have a solution object"))?;
    let path = solution.get("path").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("Filter must have a solution path"))?;
    let source = dunce::canonicalize(dir.join(path.replace('\\', "/")))?;
    let projects = selected_paths(&source, selected, &root)?;
    solution.insert("projects".into(), json!(projects));
    let prefix = if text.starts_with('\u{feff}') { "\u{feff}" } else { "" };
    Ok(format!("{prefix}{}\n", serde_json::to_string_pretty(&value)?))
}
