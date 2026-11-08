//! Explicit, atomic editing of a workspace-root global.json. Never evaluates project code.

use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Snapshot used to reject overwriting edits made after the SDK chooser opened.
#[derive(Clone, Debug)]
pub struct GlobalJson {
    pub path: PathBuf,
    original: Option<Vec<u8>>,
    value: Value,
}

impl GlobalJson {
    /// Read only the workspace file. Ancestor files are deliberately never edited.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("global.json");
        reject_symlink(&path)?;
        let original = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let value: Value = match &original {
            Some(bytes) => serde_json::from_slice(bytes).context("global.json must contain valid JSON; commented files must be edited manually")?,
            None => Value::Object(Map::new()),
        };
        ensure!(value.is_object(), "global.json must be an object");
        ensure!(value.get("sdk").is_none_or(Value::is_object), "global.json sdk must be an object");
        ensure!(value.pointer("/sdk/version").is_none_or(Value::is_string), "global.json sdk.version must be a string");
        Ok(Self { path, original, value })
    }

    pub fn version(&self) -> Option<&str> {
        self.value.pointer("/sdk/version").and_then(Value::as_str)
    }

    /// Replace only sdk.version; preserve other SDK policies and unrelated settings.
    /// Must run off the UI thread. Refuses stale snapshots and symlink targets.
    pub fn save_version(&self, version: &str) -> Result<()> {
        let (release, suffix) = version.split_once('-').map_or((version, None), |(a, b)| (a, Some(b)));
        let parts: Vec<_> = release.split('.').collect();
        ensure!(
            parts.len() == 3
                && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
                && suffix.is_none_or(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')),
            "SDK version must be a full version such as 10.0.100"
        );
        reject_symlink(&self.path)?;
        let current = match std::fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        ensure!(current == self.original, "global.json changed on disk; reopen the SDK chooser before saving");
        let mut value = self.value.clone();
        value["sdk"]["version"] = Value::String(version.to_owned());
        let parent = self.path.parent().context("global.json has no parent folder")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        if let Ok(metadata) = std::fs::metadata(&self.path) {
            file.as_file().set_permissions(metadata.permissions())?;
        }
        serde_json::to_writer_pretty(&mut file, &value)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|e| e.error).context("Could not replace global.json")?;
        Ok(())
    }
}

fn reject_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(meta.is_file() && !meta.file_type().is_symlink(), "global.json must be a regular, non-symlink file");
            ensure!(meta.len() <= 1024 * 1024, "global.json is too large to edit safely");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_sdk_pin_without_editing_parent_policy() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("global.json"), r#"{"sdk":{"version":"8.0.100"}}"#).unwrap();
        let child = root.path().join("project");
        std::fs::create_dir(&child).unwrap();
        let file = GlobalJson::load(&child).unwrap();
        assert_eq!(file.version(), None);
        file.save_version("10.0.100-preview.1").unwrap();
        assert_eq!(GlobalJson::load(&child).unwrap().version(), Some("10.0.100-preview.1"));
        assert_eq!(GlobalJson::load(root.path()).unwrap().version(), Some("8.0.100"));
    }

    #[test]
    fn preserves_sdk_policies_and_unrelated_fields_in_real_project() {
        let root = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj");
        std::fs::copy(fixture, root.path().join("Shop.Api.csproj")).unwrap();
        std::fs::write(root.path().join("global.json"), r#"{"sdk":{"version":"8.0.100","rollForward":"latestFeature","allowPrerelease":false,"paths":["$host$"]},"msbuild-sdks":{"Custom":"1.0"},"test":{"runner":"Microsoft.Testing.Platform"}}"#).unwrap();
        let before = GlobalJson::load(root.path()).unwrap();
        let mut expected = before.value.clone();
        expected["sdk"]["version"] = "10.0.100".into();
        before.save_version("10.0.100").unwrap();
        assert_eq!(GlobalJson::load(root.path()).unwrap().value, expected);
        assert_eq!(crate::ProjectInfo::load(&root.path().join("Shop.Api.csproj")).unwrap().sdk, "Microsoft.NET.Sdk.Web");
    }

    #[test]
    fn invalid_or_commented_files_are_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        for text in ["not json", "[]", r#"{"sdk":null}"#, r#"{"sdk":{"version":3}}"#, "{ // comment\n}"] {
            std::fs::write(root.path().join("global.json"), text).unwrap();
            assert!(GlobalJson::load(root.path()).is_err());
            assert_eq!(std::fs::read_to_string(root.path().join("global.json")).unwrap(), text);
        }
    }

    #[test]
    fn stale_snapshot_and_invalid_version_do_not_write() {
        let root = tempfile::tempdir().unwrap();
        let file = GlobalJson::load(root.path()).unwrap();
        for version in ["", "10", "10.0", "10.0.x", "10.0.100\n", "10.0.100-"] {
            assert!(file.save_version(version).is_err());
        }
        assert!(!file.path.exists());
        std::fs::write(&file.path, "{}").unwrap();
        assert!(file.save_version("10.0.100").is_err());
        assert_eq!(std::fs::read_to_string(&file.path).unwrap(), "{}");
    }

    #[cfg(unix)]
    #[test]
    fn sdk_scan_is_cancellable() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let host = root.path().join("dotnet");
        std::fs::write(&host, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755)).unwrap();
        let inbox = rc_core::Inbox::new(rc_core::Waker::noop());
        let scan = start_load(root.path().to_path_buf(), host, inbox.sender());
        scan.cancel();
        let result = inbox.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(!root.path().join("global.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("other.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("global.json")).unwrap();
        assert!(GlobalJson::load(root.path()).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "{}");
    }
}

/// Load the workspace policy and enumerate installed SDKs with timeout and cancellation.
pub fn start_load(root: PathBuf, dotnet: PathBuf, out: rc_core::inbox::Outbox<Result<(GlobalJson, Vec<crate::SdkInfo>), String>>) -> crate::OpHandle {
    let cancel = crate::CancelToken::new();
    let token = cancel.clone();
    let task = rc_core::Runtime::spawn(async move {
        let result: Result<(GlobalJson, Vec<crate::SdkInfo>)> = async {
            let file = tokio::task::spawn_blocking(move || GlobalJson::load(&root)).await??;
            let host = tokio::task::spawn_blocking(move || crate::Dotnet::locate(Some(&dotnet))).await?.context("The .NET SDK was not found")?;
            let mut cmd = host.command();
            cmd.arg("--list-sdks").kill_on_drop(true);
            let output = tokio::select! {
                _ = token.cancelled() => anyhow::bail!("SDK discovery cancelled"),
                output = tokio::time::timeout(std::time::Duration::from_secs(10), cmd.output()) => output.context("SDK discovery timed out")??,
            };
            ensure!(output.status.success(), "SDK discovery failed: {}", String::from_utf8_lossy(&output.stderr));
            Ok((file, crate::sdk::parse_sdks(&String::from_utf8_lossy(&output.stdout))))
        }
        .await;
        out.send(result.map_err(|e| format!("{e:#}")));
    });
    crate::OpHandle::new(cancel, task)
}
