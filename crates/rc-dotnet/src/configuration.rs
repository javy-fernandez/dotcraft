//! Build configuration and platform choices declared by classic solution files.

use std::path::Path;

/// A solution-level build choice. The platform spelling is kept as written in the
/// solution (for example, `Any CPU`); MSBuild maps it to each project's platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolutionConfiguration {
    pub name: String,
    pub platform: String,
}

/// Reads build choices from a `.sln` or from the source solution of a `.slnf`.
/// XML `.slnx` configuration discovery is not supported yet.
pub fn load_solution_configurations(path: &Path) -> anyhow::Result<Vec<SolutionConfiguration>> {
    let source = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("slnf"))
    {
        crate::Solution::load(path)?.source_path
    } else {
        path.to_path_buf()
    };
    anyhow::ensure!(
        source
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sln")),
        "Build configuration discovery requires a .sln source solution"
    );
    let text = std::fs::read_to_string(source)?;
    Ok(parse_solution_configurations(&text))
}

/// Parses only the solution-wide choices, not the per-project ActiveCfg mappings.
pub fn parse_solution_configurations(text: &str) -> Vec<SolutionConfiguration> {
    let mut choices = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("GlobalSection(") {
            in_section = line.starts_with("GlobalSection(SolutionConfigurationPlatforms)");
            continue;
        }
        if line == "EndGlobalSection" {
            in_section = false;
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((choice, _)) = line.split_once('=') else {
            continue;
        };
        let Some((name, platform)) = choice.trim().split_once('|') else {
            continue;
        };
        let (name, platform) = (name.trim(), platform.trim());
        if name.is_empty() || platform.is_empty() {
            continue;
        }
        let choice = SolutionConfiguration {
            name: name.to_string(),
            platform: platform.to_string(),
        };
        if !choices.contains(&choice) {
            choices.push(choice);
        }
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_solution_choices_without_project_mappings() {
        let text = "GlobalSection(ProjectConfigurationPlatforms) = postSolution\n\tDebug|Fake = Debug|Fake\nEndGlobalSection\n\
            GlobalSection(SolutionConfigurationPlatforms) = preSolution\n\tDebug|Any CPU = Debug|Any CPU\n\tRelease|x64 = Release|x64\n\tDebug|Any CPU = Debug|Any CPU\n\tBroken = Broken\nEndGlobalSection\n";
        assert_eq!(
            parse_solution_configurations(text),
            vec![
                SolutionConfiguration {
                    name: "Debug".into(),
                    platform: "Any CPU".into()
                },
                SolutionConfiguration {
                    name: "Release".into(),
                    platform: "x64".into()
                },
            ]
        );
    }

    #[test]
    fn reads_shop_fixture_and_filter_source() {
        let path = Path::new("../../tests/fixtures/Shop/Shop.sln");
        let choices = load_solution_configurations(path).unwrap();
        assert_eq!(choices.len(), 6);
        assert!(choices.contains(&SolutionConfiguration {
            name: "Release".into(),
            platform: "x86".into()
        }));
        let dir = tempfile::tempdir().unwrap();
        let filter = dir.path().join("Shop.slnf");
        let source = std::fs::canonicalize(path).unwrap();
        std::fs::write(
            &filter,
            serde_json::json!({"solution": {"path": source, "projects": []}}).to_string(),
        )
        .unwrap();
        assert_eq!(load_solution_configurations(&filter).unwrap(), choices);
    }
}
