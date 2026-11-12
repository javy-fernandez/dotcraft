//! Opens the .NET user-secrets file for a project without putting secrets in the workspace.

use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::Dotnet;
use std::path::{Path, PathBuf};

pub struct UserSecrets {
    inbox: Inbox<Result<PathBuf, String>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl UserSecrets {
    pub fn new(waker: Waker) -> Self {
        Self {
            inbox: Inbox::new(waker),
            task: None,
        }
    }

    pub fn open(&mut self, dotnet: Dotnet, project: PathBuf) {
        self.cancel();
        self.inbox.drain();
        let out = self.inbox.sender();
        self.task = Some(Runtime::spawn(async move {
            out.send(
                prepare(&dotnet, &project)
                    .await
                    .map_err(|error| format!("{error:#}")),
            );
        }));
    }

    pub fn drain(&mut self) -> Vec<Result<PathBuf, String>> {
        let results = self.inbox.drain();
        if !results.is_empty() {
            self.task.take();
        }
        results
    }

    pub fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.inbox.drain();
    }
}

impl Drop for UserSecrets {
    fn drop(&mut self) {
        self.cancel();
    }
}

async fn prepare(dotnet: &Dotnet, project: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        project.extension().is_some_and(|ext| ext == "csproj"),
        "User Secrets requires a C# project"
    );
    let xml = tokio::fs::read_to_string(project).await?;
    let mut id = secrets_id(&xml)?;
    if id.is_none() {
        let mut command = dotnet.command();
        command
            .args(["user-secrets", "init", "--project"])
            .arg(project)
            .kill_on_drop(true);
        let output = command.output().await?;
        anyhow::ensure!(
            output.status.success(),
            "dotnet user-secrets init failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        id = secrets_id(&tokio::fs::read_to_string(project).await?)?;
    }
    let id =
        id.ok_or_else(|| anyhow::anyhow!("The project has no UserSecretsId after initialization"))?;
    let path = secrets_path(&id)?;
    tokio::task::spawn_blocking({
        let path = path.clone();
        move || create_empty_secrets(&path)
    })
    .await??;
    Ok(path)
}

fn secrets_id(xml: &str) -> anyhow::Result<Option<String>> {
    let doc = roxmltree::Document::parse(xml)?;
    Ok(doc
        .descendants()
        .find(|node| node.has_tag_name("UserSecretsId"))
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string))
}

fn secrets_path(id: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        id != "."
            && id != ".."
            && !id
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':')),
        "UserSecretsId is not a safe file-name component"
    );
    #[cfg(windows)]
    let root = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?
        .join("Microsoft/UserSecrets");
    #[cfg(not(windows))]
    let root = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Home directory was not found"))?
        .join(".microsoft/usersecrets");
    Ok(root.join(id).join("secrets.json"))
}

fn create_empty_secrets(path: &Path) -> std::io::Result<()> {
    use std::io::{ErrorKind, Write};

    let parent = path.parent().expect("secrets path includes a directory");
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => file.write_all(b"{}\n"),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn is_user_secrets_file(path: &Path) -> bool {
    if path.file_name().is_none_or(|name| name != "secrets.json") {
        return false;
    }
    let mut components = path.components().rev();
    let _file = components.next();
    let Some(std::path::Component::Normal(_id)) = components.next() else {
        return false;
    };
    let Some(std::path::Component::Normal(root)) = components.next() else {
        return false;
    };
    root == "usersecrets" || root == "UserSecrets"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_existing_id_and_rejects_path_escape() {
        let xml = "<Project><PropertyGroup><UserSecretsId>my-app-42</UserSecretsId></PropertyGroup></Project>";
        assert_eq!(secrets_id(xml).unwrap().as_deref(), Some("my-app-42"));
        assert!(secrets_path("../other").is_err());
        assert!(secrets_path("a\\b").is_err());
    }

    #[test]
    fn creates_private_file_without_overwriting_existing_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("usersecrets/test/secrets.json");
        create_empty_secrets(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}\n");
        std::fs::write(&file, "{\"key\":\"value\"}").unwrap();
        create_empty_secrets(&file).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "{\"key\":\"value\"}"
        );
        assert!(is_user_secrets_file(&file));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
