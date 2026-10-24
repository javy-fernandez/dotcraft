//! The `git` executable: finding it, checking its version and running it asynchronously
//! (stdin/stdout/stderr piped, killed on cancel or drop, progress lines streamed).

use crate::cancel::CancelToken;
use crate::error::{GitError, Result};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Oldest git DotCraft supports (`switch`/`restore`, porcelain v2, `blame --contents -`).
pub const MIN_VERSION: GitVersion = GitVersion { major: 2, minor: 25, patch: 0 };

/// A git version as printed by `git --version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl GitVersion {
    /// Parses `git version 2.45.1`, `git version 2.39.3 (Apple Git-146)`,
    /// `git version 2.45.1.windows.1`.
    pub fn parse(text: &str) -> Option<GitVersion> {
        let v = text.trim().strip_prefix("git version ")?.split_whitespace().next()?;
        let mut parts = v.split('.').map(|p| p.parse::<u32>().ok());
        Some(GitVersion { major: parts.next()??, minor: parts.next()??, patch: parts.next().flatten().unwrap_or(0) })
    }
}

impl std::fmt::Display for GitVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One progress line of a network operation (`Receiving objects:  45% (45/100)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub text: String,
    /// The percentage in the line, when there is one.
    pub percent: Option<u8>,
}

impl Progress {
    pub fn parse(line: &str) -> Progress {
        let percent = line.find('%').and_then(|at| {
            let digits: String = line[..at].chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
            digits.parse::<u8>().ok().filter(|p| *p <= 100)
        });
        Progress { text: line.trim_end().to_string(), percent }
    }
}

/// Receives progress lines while fetch/pull/push run (called from the runtime's threads).
pub type ProgressSink = Arc<dyn Fn(Progress) + Send + Sync>;

/// A progress sink that forwards every line to the UI through an outbox (which wakes it).
pub fn progress_to(outbox: rc_core::inbox::Outbox<Progress>) -> ProgressSink {
    Arc::new(move |p| {
        outbox.send(p);
    })
}

/// A located, version-checked git executable.
#[derive(Clone, Debug)]
pub struct Git {
    path: PathBuf,
    version: GitVersion,
}

impl Git {
    /// Looks for `git` in `PATH`, then in the usual install locations (Git for Windows,
    /// Homebrew, Xcode command line tools) that GUI apps may not have in their `PATH`.
    pub fn find_executable() -> Option<PathBuf> {
        let exe = if cfg!(windows) { "git.exe" } else { "git" };
        let mut candidates: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join(exe)).collect()).unwrap_or_default();
        if cfg!(windows) {
            for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                if let Some(dir) = std::env::var_os(var) {
                    candidates.push(Path::new(&dir).join("Git").join("cmd").join(exe));
                }
            }
            if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
                candidates.push(Path::new(&dir).join("Programs").join("Git").join("cmd").join(exe));
            }
            candidates.push(PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"));
        } else {
            candidates.extend(["/opt/homebrew/bin/git", "/usr/local/bin/git", "/usr/bin/git"].map(PathBuf::from));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    /// Checks the executable at `path` (runs `git --version`).
    pub async fn open(path: PathBuf) -> Result<Git> {
        let out = tokio::process::Command::new(&path).arg("--version").stdin(Stdio::null()).output().await.map_err(|_| GitError::GitNotFound)?;
        let text = String::from_utf8_lossy(&out.stdout);
        let version = GitVersion::parse(&text).ok_or_else(|| GitError::Parse(text.to_string()))?;
        if version < MIN_VERSION {
            return Err(GitError::UnsupportedVersion { found: version.to_string(), required: MIN_VERSION.to_string() });
        }
        Ok(Git { path, version })
    }

    /// The git found by [`Git::find_executable`], checked once and shared afterwards.
    pub async fn detect() -> Result<Arc<Git>> {
        static GIT: tokio::sync::OnceCell<Arc<Git>> = tokio::sync::OnceCell::const_new();
        GIT.get_or_try_init(|| async {
            let path = Git::find_executable().ok_or(GitError::GitNotFound)?;
            Git::open(path).await.map(Arc::new)
        })
        .await
        .cloned()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn version(&self) -> GitVersion {
        self.version
    }

    /// A git command run in `cwd`.
    pub fn command(&self, cwd: &Path) -> GitCommand<'_> {
        GitCommand { git: self, cwd: cwd.to_path_buf(), args: Vec::new(), stdin: None, cancel: None, ok_codes: vec![0] }
    }
}

/// Captured result of a git run.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub code: Option<i32>,
}

impl Output {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// Builder for one git invocation.
pub struct GitCommand<'a> {
    git: &'a Git,
    cwd: PathBuf,
    args: Vec<OsString>,
    stdin: Option<Vec<u8>>,
    cancel: Option<CancelToken>,
    ok_codes: Vec<i32>,
}

impl GitCommand<'_> {
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(mut self, args: I) -> Self {
        self.args.extend(args.into_iter().map(|a| a.as_ref().to_owned()));
        self
    }

    /// Bytes written to git's stdin.
    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self
    }

    pub fn cancel(mut self, token: Option<&CancelToken>) -> Self {
        self.cancel = token.cloned();
        self
    }

    /// Accepts another exit code as success (e.g. 1 for `diff --no-index`).
    pub fn ok_code(mut self, code: i32) -> Self {
        self.ok_codes.push(code);
        self
    }

    fn build(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.git.path);
        cmd.current_dir(&self.cwd)
            .args(["-c", "core.quotepath=false", "-c", "color.ui=false", "-c", "core.pager=cat", "-c", "advice.detachedHead=false"])
            .args(&self.args)
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_PAGER", "cat")
            .env("GIT_EDITOR", ":")
            .stdin(if self.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        cmd
    }

    fn arg_strings(&self) -> Vec<String> {
        self.args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn finish(&self, out: Output) -> Result<Output> {
        if out.code.is_some_and(|c| self.ok_codes.contains(&c)) {
            Ok(out)
        } else {
            Err(GitError::classify(&self.arg_strings(), out.code, &String::from_utf8_lossy(&out.stdout), &out.stderr))
        }
    }

    /// Runs git to completion; non-accepted exit codes become classified errors.
    pub async fn output(self) -> Result<Output> {
        self.streamed(None).await
    }

    /// Runs git and returns stdout as (lossy) UTF-8 text.
    pub async fn text(self) -> Result<String> {
        Ok(self.output().await?.stdout_text())
    }

    /// Runs git, sending every stderr line (split on `\r` and `\n`) to `progress`.
    pub async fn streamed(self, progress: Option<ProgressSink>) -> Result<Output> {
        if self.cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
            return Err(GitError::Cancelled);
        }
        let mut child = self.build().spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { GitError::GitNotFound } else { GitError::Io(e) })?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let input = self.stdin.clone();
        let work = async {
            let write = async {
                if let (Some(mut pipe), Some(bytes)) = (stdin, input) {
                    // git may exit before reading everything (e.g. on errors); that is fine.
                    let _ = pipe.write_all(&bytes).await;
                    let _ = pipe.shutdown().await;
                }
            };
            let read_out = async {
                let mut buf = Vec::new();
                stdout.read_to_end(&mut buf).await.map(|_| buf)
            };
            let read_err = async {
                let mut all = Vec::new();
                let mut line = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stderr.read(&mut chunk).await?;
                    if n == 0 {
                        break;
                    }
                    all.extend_from_slice(&chunk[..n]);
                    if let Some(sink) = &progress {
                        for &b in &chunk[..n] {
                            if b == b'\r' || b == b'\n' {
                                if !line.is_empty() {
                                    sink(Progress::parse(&String::from_utf8_lossy(&line)));
                                    line.clear();
                                }
                            } else {
                                line.push(b);
                            }
                        }
                    }
                }
                if let Some(sink) = &progress
                    && !line.is_empty()
                {
                    sink(Progress::parse(&String::from_utf8_lossy(&line)));
                }
                Ok::<_, std::io::Error>(all)
            };
            let ((), out, err) = tokio::join!(write, read_out, read_err);
            let status = child.wait().await?;
            Ok::<_, std::io::Error>(Output { stdout: out?, stderr: String::from_utf8_lossy(&err?).into_owned(), code: status.code() })
        };
        let out = match &self.cancel {
            Some(token) => tokio::select! {
                out = work => out?,
                // Dropping `work` drops the child, which kills it (`kill_on_drop`).
                () = token.cancelled() => return Err(GitError::Cancelled),
            },
            None => work.await?,
        };
        self.finish(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(GitVersion::parse("git version 2.39.3 (Apple Git-146)\n"), Some(GitVersion { major: 2, minor: 39, patch: 3 }));
        assert_eq!(GitVersion::parse("git version 2.45.1.windows.1"), Some(GitVersion { major: 2, minor: 45, patch: 1 }));
        assert_eq!(GitVersion::parse("git version 3.0"), Some(GitVersion { major: 3, minor: 0, patch: 0 }));
        assert!(GitVersion::parse("hello").is_none());
        assert!(GitVersion { major: 2, minor: 24, patch: 9 } < MIN_VERSION);
    }

    #[test]
    fn parses_progress() {
        assert_eq!(Progress::parse("Receiving objects:  45% (45/100), 1.2 MiB").percent, Some(45));
        assert_eq!(Progress::parse("remote: Counting objects: 100% (3/3), done.").percent, Some(100));
        assert_eq!(Progress::parse("From /tmp/x").percent, None);
    }
}
