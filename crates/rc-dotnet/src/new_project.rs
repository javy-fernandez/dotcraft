//! New solutions and projects from the SDK's templates (`dotnet new`), as Rider's "New
//! Solution" and "Add New Project" do it: create the project, the solution when needed, add the
//! project to it, and optionally a Git repository with a .NET `.gitignore`.

use crate::sdk::Dotnet;
use std::path::{Path, PathBuf};

/// A template shipped with the .NET SDK (8 and later).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Template {
    /// `dotnet new` short name.
    pub short_name: &'static str,
    /// English name for `rc_kit::i18n::tr`.
    pub name: &'static str,
    pub description: &'static str,
    /// Web projects (their own icon in the UI).
    pub web: bool,
    pub test: bool,
}

pub const TEMPLATES: &[Template] = &[
    Template { short_name: "console", name: "Console Application", description: "A command-line application", web: false, test: false },
    Template { short_name: "classlib", name: "Class Library", description: "A library of reusable code", web: false, test: false },
    Template { short_name: "webapi", name: "ASP.NET Core Web API", description: "An HTTP API with controllers or minimal APIs", web: true, test: false },
    Template { short_name: "web", name: "ASP.NET Core Empty", description: "An empty ASP.NET Core application", web: true, test: false },
    Template { short_name: "mvc", name: "ASP.NET Core MVC", description: "A web application with views and controllers", web: true, test: false },
    Template { short_name: "blazor", name: "Blazor Web App", description: "Interactive web UI with Razor components", web: true, test: false },
    Template { short_name: "worker", name: "Worker Service", description: "A long-running background service", web: false, test: false },
    Template { short_name: "xunit", name: "xUnit Test Project", description: "Unit tests with xUnit", web: false, test: true },
    Template { short_name: "nunit", name: "NUnit Test Project", description: "Unit tests with NUnit", web: false, test: true },
    Template { short_name: "mstest", name: "MSTest Test Project", description: "Unit tests with MSTest", web: false, test: true },
];

/// What to create.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewProject {
    pub template: &'static str,
    pub project_name: String,
    /// `net8.0`, `net10.0`… (`None`: the template's default).
    pub framework: Option<String>,
    /// Web API: controllers instead of minimal APIs (`--use-controllers`).
    pub use_controllers: bool,
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A new solution `<location>/<name>/<name>.sln[x]` with the project in a folder of its own.
    NewSolution { location: PathBuf, solution_name: String, git: bool },
    /// Add the project to an existing solution, next to it.
    AddTo { solution: PathBuf },
}

impl NewProject {
    /// Folder of the new project.
    pub fn project_dir(&self) -> PathBuf {
        match &self.target {
            Target::NewSolution { location, solution_name, .. } => location.join(solution_name).join(&self.project_name),
            Target::AddTo { solution } => solution.parent().unwrap_or(Path::new(".")).join(&self.project_name),
        }
    }

    pub fn project_file(&self) -> PathBuf {
        self.project_dir().join(format!("{}.csproj", self.project_name))
    }

    /// Why it cannot be created (an English message for `tr`), or `None`.
    pub fn problem(&self) -> Option<&'static str> {
        let bad = |s: &str| s.trim().is_empty() || s.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) || s.starts_with('.');
        if bad(&self.project_name) {
            return Some("Enter a valid project name");
        }
        if let Target::NewSolution { location, solution_name, .. } = &self.target {
            if bad(solution_name) {
                return Some("Enter a valid solution name");
            }
            if !location.is_dir() {
                return Some("The location does not exist");
            }
            if location.join(solution_name).exists() {
                return Some("A folder with that name already exists");
            }
        } else if self.project_dir().exists() {
            return Some("A folder with that name already exists");
        }
        None
    }

    /// The `dotnet` invocations, in order (working directory, arguments).
    pub fn commands(&self) -> Vec<(PathBuf, Vec<String>)> {
        let dir = self.project_dir();
        let mut project = vec!["new".to_string(), self.template.to_string(), "-n".into(), self.project_name.clone(), "-o".into(), dir.to_string_lossy().into_owned()];
        if let Some(f) = &self.framework {
            project.extend(["-f".into(), f.clone()]);
        }
        if self.use_controllers && self.template == "webapi" {
            project.push("--use-controllers".into());
        }
        match &self.target {
            Target::NewSolution { location, solution_name, git } => {
                let root = location.join(solution_name);
                let mut cmds = vec![(root.clone(), vec!["new".into(), "sln".into(), "-n".into(), solution_name.clone()])];
                if *git {
                    cmds.push((root.clone(), vec!["new".into(), "gitignore".into()]));
                }
                cmds.push((root.clone(), project));
                cmds.push((root, vec!["sln".into(), "add".into(), self.project_file().to_string_lossy().into_owned()]));
                cmds
            }
            Target::AddTo { solution } => {
                let root = solution.parent().unwrap_or(Path::new(".")).to_path_buf();
                vec![(root.clone(), project), (root, vec!["sln".into(), solution.to_string_lossy().into_owned(), "add".into(), self.project_file().to_string_lossy().into_owned()])]
            }
        }
    }

    /// Creates everything; returns the solution to open (or the one the project was added
    /// to). `progress` receives each command line.
    pub async fn create(&self, dotnet: &Dotnet, progress: &mut (dyn FnMut(String) + Send)) -> anyhow::Result<PathBuf> {
        if let Some(p) = self.problem() {
            anyhow::bail!(p);
        }
        if let Target::NewSolution { location, solution_name, .. } = &self.target {
            std::fs::create_dir_all(location.join(solution_name))?;
        }
        for (dir, args) in self.commands() {
            progress(format!("dotnet {}", args.join(" ")));
            let out = dotnet.command().args(&args).current_dir(&dir).kill_on_drop(true).output().await?;
            if !out.status.success() {
                let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                let line = text.lines().map(str::trim).rev().find(|l| !l.is_empty()).unwrap_or("dotnet failed").to_string();
                anyhow::bail!("dotnet {} {}: {line}", args[0], args.get(1).map(String::as_str).unwrap_or(""));
            }
        }
        match &self.target {
            Target::AddTo { solution } => Ok(solution.clone()),
            Target::NewSolution { location, solution_name, git } => {
                let root = location.join(solution_name);
                if *git {
                    progress("git init".into());
                    let _ = tokio::process::Command::new("git").arg("init").arg("-q").current_dir(&root).output().await;
                }
                // `dotnet new sln` writes .slnx from .NET 10 on, .sln before.
                ["slnx", "sln"].iter().map(|e| root.join(format!("{solution_name}.{e}"))).find(|p| p.exists()).ok_or_else(|| anyhow::anyhow!("the solution file was not created"))
            }
        }
    }
}

/// Target frameworks of the installed SDKs, newest first (`net10.0`, `net8.0`).
pub fn frameworks(dotnet: &Dotnet) -> Vec<String> {
    let mut majors: Vec<u32> = dotnet.list_sdks().unwrap_or_default().iter().map(|s| s.major()).filter(|m| *m >= 5).collect();
    majors.sort_unstable_by(|a, b| b.cmp(a));
    majors.dedup();
    majors.into_iter().map(|m| format!("net{m}.0")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_for_a_new_solution_and_for_adding_a_project() {
        let p = NewProject {
            template: "console",
            project_name: "App".into(),
            framework: Some("net8.0".into()),
            use_controllers: false,
            target: Target::NewSolution { location: "/w".into(), solution_name: "Demo".into(), git: true },
        };
        assert_eq!(p.project_file(), PathBuf::from("/w/Demo/App/App.csproj"));
        let cmds = p.commands();
        assert_eq!(cmds[0].1, ["new", "sln", "-n", "Demo"]);
        assert_eq!(cmds[1].1, ["new", "gitignore"]);
        assert_eq!(cmds[2].1, ["new", "console", "-n", "App", "-o", "/w/Demo/App", "-f", "net8.0"]);
        assert_eq!(cmds[3].1, ["sln", "add", "/w/Demo/App/App.csproj"]);
        assert!(cmds.iter().all(|(d, _)| d == Path::new("/w/Demo")));

        let add = NewProject { template: "xunit", project_name: "App.Tests".into(), framework: None, use_controllers: false, target: Target::AddTo { solution: "/w/Demo/Demo.slnx".into() } };
        assert_eq!(add.commands()[1].1, ["sln", "/w/Demo/Demo.slnx", "add", "/w/Demo/App.Tests/App.Tests.csproj"]);
        assert_eq!(NewProject { project_name: "a/b".into(), ..add.clone() }.problem(), Some("Enter a valid project name"));
        let api = NewProject { template: "webapi", use_controllers: true, ..add };
        assert!(api.commands()[0].1.contains(&"--use-controllers".to_string()));
    }
}
