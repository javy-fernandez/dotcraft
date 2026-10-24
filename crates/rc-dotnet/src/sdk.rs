//! Locating the `dotnet` host and listing its SDKs and runtimes.

use std::path::{Path, PathBuf};

/// Name of the host executable on this platform.
pub const DOTNET_EXE: &str = if cfg!(windows) { "dotnet.exe" } else { "dotnet" };

/// The `dotnet` host used for every command of this crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dotnet {
    pub path: PathBuf,
}

/// An installed SDK (`dotnet --list-sdks`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdkInfo {
    pub version: String,
    /// The `sdk` folder that contains it.
    pub path: PathBuf,
}

/// An installed shared runtime (`dotnet --list-runtimes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInfo {
    /// `Microsoft.NETCore.App`, `Microsoft.AspNetCore.App`…
    pub name: String,
    pub version: String,
    pub path: PathBuf,
}

impl SdkInfo {
    /// Major version (8 for `8.0.418`).
    pub fn major(&self) -> u32 {
        self.version.split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0)
    }
}

impl Dotnet {
    /// Finds `dotnet`: the explicit path (file or folder), `DOTNET_ROOT`, `PATH`, then the
    /// default install locations. `None` when no candidate exists.
    pub fn locate(explicit: Option<&Path>) -> Option<Dotnet> {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        candidates(explicit, env("DOTNET_ROOT"), std::env::var_os("PATH"), env("HOME").or_else(|| env("USERPROFILE")), env("ProgramFiles"))
            .into_iter()
            .find(|p| p.is_file())
            .map(|p| Dotnet { path: p.canonicalize().unwrap_or(p) })
    }

    pub fn new(path: impl Into<PathBuf>) -> Dotnet {
        Dotnet { path: path.into() }
    }

    /// The installation root (folder of the host).
    pub fn root(&self) -> Option<&Path> {
        self.path.parent()
    }

    /// A `dotnet` command with English, logo-free CLI output (the parsers rely on it).
    pub fn command(&self) -> tokio::process::Command {
        let mut c = tokio::process::Command::new(&self.path);
        for (k, v) in cli_env() {
            c.env(k, v);
        }
        c
    }

    /// Blocking variant of [`Dotnet::command`].
    pub fn std_command(&self) -> std::process::Command {
        let mut c = std::process::Command::new(&self.path);
        for (k, v) in cli_env() {
            c.env(k, v);
        }
        c
    }

    /// `dotnet --version`: the SDK that `global.json` / roll-forward selects (blocking).
    pub fn version(&self, dir: Option<&Path>) -> anyhow::Result<String> {
        let mut c = self.std_command();
        c.arg("--version");
        if let Some(d) = dir {
            c.current_dir(d);
        }
        Ok(capture(c)?.trim().to_string())
    }

    /// Installed SDKs (blocking: call off the UI thread).
    pub fn list_sdks(&self) -> anyhow::Result<Vec<SdkInfo>> {
        let mut c = self.std_command();
        c.arg("--list-sdks");
        Ok(parse_sdks(&capture(c)?))
    }

    /// Installed runtimes (blocking: call off the UI thread).
    pub fn list_runtimes(&self) -> anyhow::Result<Vec<RuntimeInfo>> {
        let mut c = self.std_command();
        c.arg("--list-runtimes");
        Ok(parse_runtimes(&capture(c)?))
    }
}

pub(crate) fn cli_env() -> [(&'static str, &'static str); 4] {
    [
        ("DOTNET_CLI_UI_LANGUAGE", "en"),
        ("DOTNET_NOLOGO", "1"),
        ("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1"),
        ("MSBUILDTERMINALLOGGER", "off"),
    ]
}

fn capture(mut c: std::process::Command) -> anyhow::Result<String> {
    let out = c.output()?;
    if !out.status.success() {
        anyhow::bail!("{:?} failed: {}", c, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Candidate host paths in lookup order (pure, for tests).
pub fn candidates(explicit: Option<&Path>, dotnet_root: Option<PathBuf>, path_var: Option<std::ffi::OsString>, home: Option<PathBuf>, program_files: Option<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(e) = explicit {
        if e.extension().is_none() && !e.ends_with("dotnet") || e.is_dir() {
            out.push(e.join(DOTNET_EXE));
        } else {
            out.push(e.to_path_buf());
        }
    }
    if let Some(r) = dotnet_root {
        out.push(r.join(DOTNET_EXE));
    }
    if let Some(p) = path_var {
        out.extend(std::env::split_paths(&p).map(|d| d.join(DOTNET_EXE)));
    }
    if cfg!(windows) {
        if let Some(pf) = program_files {
            out.push(pf.join("dotnet").join(DOTNET_EXE));
        }
        out.push(PathBuf::from(r"C:\Program Files\dotnet\dotnet.exe"));
    } else {
        out.push(PathBuf::from("/usr/local/share/dotnet/dotnet"));
        out.push(PathBuf::from("/usr/share/dotnet/dotnet"));
        out.push(PathBuf::from("/usr/lib/dotnet/dotnet"));
    }
    if let Some(h) = home {
        out.push(h.join(".dotnet").join(DOTNET_EXE));
    }
    out
}

/// Parses `8.0.418 [/usr/local/share/dotnet/sdk]` lines.
pub fn parse_sdks(text: &str) -> Vec<SdkInfo> {
    text.lines()
        .filter_map(|l| {
            let (version, path) = split_bracket(l)?;
            Some(SdkInfo { version: version.to_string(), path })
        })
        .collect()
}

/// Parses `Microsoft.NETCore.App 8.0.24 [/usr/local/share/dotnet/shared/Microsoft.NETCore.App]`.
pub fn parse_runtimes(text: &str) -> Vec<RuntimeInfo> {
    text.lines()
        .filter_map(|l| {
            let (head, path) = split_bracket(l)?;
            let (name, version) = head.split_once(' ')?;
            Some(RuntimeInfo { name: name.to_string(), version: version.trim().to_string(), path })
        })
        .collect()
}

fn split_bracket(line: &str) -> Option<(&str, PathBuf)> {
    let line = line.trim();
    let open = line.find(" [")?;
    let path = line[open + 2..].strip_suffix(']')?;
    Some((line[..open].trim(), PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sdk_and_runtime_lists() {
        let sdks = parse_sdks("8.0.418 [/usr/local/share/dotnet/sdk]\n10.0.103 [/usr/local/share/dotnet/sdk]\n\n");
        assert_eq!(sdks.len(), 2);
        assert_eq!(sdks[1].version, "10.0.103");
        assert_eq!(sdks[1].major(), 10);
        assert_eq!(sdks[0].path, PathBuf::from("/usr/local/share/dotnet/sdk"));

        let rts = parse_runtimes(
            "Microsoft.AspNetCore.App 8.0.24 [/usr/local/share/dotnet/shared/Microsoft.AspNetCore.App]\r\nMicrosoft.NETCore.App 10.0.3 [C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App]\n",
        );
        assert_eq!(rts[0].name, "Microsoft.AspNetCore.App");
        assert_eq!(rts[0].version, "8.0.24");
        assert_eq!(rts[1].path, PathBuf::from("C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App"));
    }

    #[test]
    fn candidates_follow_the_lookup_order() {
        let path = std::env::join_paths(["/opt/a", "/opt/b"]).unwrap();
        let c = candidates(Some(Path::new("/custom")), Some("/root".into()), Some(path), Some("/home/u".into()), None);
        assert_eq!(c[0], Path::new("/custom").join(DOTNET_EXE));
        assert_eq!(c[1], Path::new("/root").join(DOTNET_EXE));
        assert_eq!(c[2], Path::new("/opt/a").join(DOTNET_EXE));
        assert_eq!(c.last().unwrap(), &Path::new("/home/u/.dotnet").join(DOTNET_EXE));
        let c = candidates(Some(Path::new("/x/dotnet")), None, None, None, None);
        assert_eq!(c[0], PathBuf::from("/x/dotnet"));
    }
}
