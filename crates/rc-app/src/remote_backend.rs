//! The back end of a remote folder: everything that runs on the remote machine through the
//! agent connection. Files are edited as local copies under a [`Mirror`] (fetched on open,
//! pushed back on save); Roslyn and netcoredbg are installed on the remote machine the
//! first time (`~/.dotcraft/remote/roslyn/<version>`, `~/.dotcraft/remote/netcoredbg/<release>`)
//! and run there over process streams, with their paths mapped to the mirror.

use rc_core::mirror::Mirror;
use rc_remote::{Client, Operation, RemoteProcess};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What the remote machine is, from one probe at connect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    /// `uname -sm`.
    pub uname: String,
    pub home: String,
    /// `glibc 2.35` on Linux.
    pub glibc: Option<String>,
}

/// Remote tools that are already in place: then nothing is installed. Tests run the
/// "remote" on this machine with the tools of `target/test-home`.
#[derive(Clone, Debug, Default)]
pub struct Tools {
    pub roslyn_dll: Option<String>,
    pub netcoredbg: Option<String>,
}

#[derive(Clone)]
pub struct Backend {
    pub client: Client,
    pub mirror: Mirror,
    /// The open remote folder.
    pub root: String,
    /// The remote `dotnet` (a name on its PATH or a path, maybe `~/`).
    pub dotnet: String,
    pub machine: Machine,
    pub tools: Tools,
}

/// What a remote command printed and how it ended.
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Captured {
    fn ok(self, what: &str) -> anyhow::Result<Vec<u8>> {
        if self.code == Some(0) {
            Ok(self.stdout)
        } else {
            let detail = self.stderr.trim();
            anyhow::bail!("{what} failed{}{detail}", if detail.is_empty() { "" } else { ": " })
        }
    }
}

fn spawn_op(program: &str, args: &[&str], cwd: &str, path: Vec<String>) -> Operation {
    Operation::Spawn {
        program: program.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: cwd.to_string(),
        env: vec![("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into()), ("DOTNET_NOLOGO".into(), "1".into()), ("DOTNET_CLI_UI_LANGUAGE".into(), "en".into())],
        path,
    }
}

/// Runs a remote command with `input` on its stdin and collects its output.
pub async fn capture(client: &Client, operation: Operation, input: Option<Vec<u8>>) -> anyhow::Result<Captured> {
    let mut process = client.spawn(operation).await?;
    let mut stdin = process.stdin.take().expect("stdin");
    let (mut out, mut err) = (process.stdout.take().expect("stdout"), process.stderr.take().expect("stderr"));
    let writer = async move {
        if let Some(bytes) = input {
            stdin.write_all(&bytes).await?;
        }
        stdin.shutdown().await
    };
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let (written, read_out, read_err) = tokio::join!(writer, out.read_to_end(&mut stdout), err.read_to_end(&mut stderr));
    written?;
    read_out?;
    read_err?;
    let code = process.wait().await;
    Ok(Captured { code, stdout, stderr: String::from_utf8_lossy(&stderr).into_owned() })
}

/// Its system, home folder and C library.
pub async fn probe(client: &Client) -> anyhow::Result<Machine> {
    let script = "uname -sm; printf '%s\\n' \"$HOME\"; getconf GNU_LIBC_VERSION 2>/dev/null || true";
    let out = capture(client, spawn_op("sh", &["-c", script], "/", Vec::new()), None).await?.ok("uname")?;
    let text = String::from_utf8_lossy(&out);
    let mut lines = text.lines();
    Ok(Machine {
        uname: lines.next().unwrap_or_default().trim().to_string(),
        home: lines.next().unwrap_or_default().trim().to_string(),
        glibc: lines.next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()),
    })
}

/// A remote path as a POSIX `sh` word (`~/` expanded).
fn posix_dir(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(dir, _)| if dir.is_empty() { "/" } else { dir })
}

/// A `tar.gz` of `files` (relative path, contents, executable).
pub fn tar_gz(files: &[(String, Vec<u8>, bool)]) -> anyhow::Result<Vec<u8>> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(encoder);
    for (path, bytes, exec) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(if *exec { 0o755 } else { 0o644 });
        header.set_cksum();
        tar.append_data(&mut header, path, bytes.as_slice())?;
    }
    Ok(tar.into_inner()?.finish()?)
}

/// Left out of the folder copy: build output, version control and tool caches.
const SYNC_SKIP: [&str; 8] = ["--exclude=bin", "--exclude=obj", "--exclude=.git", "--exclude=node_modules", "--exclude=.vs", "--exclude=.idea", "--exclude=TestResults", "--exclude=packages"];

/// Unpacks a `tar.gz` of the remote folder `root` into its mirror; files already equal are
/// not rewritten (open editors do not reload). Returns each file's SHA-256.
fn unpack_into_mirror(archive: &[u8], mirror: &Mirror, root: &str) -> anyhow::Result<HashMap<PathBuf, String>> {
    let mut hashes = HashMap::new();
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let rel = entry.path()?.to_string_lossy().replace('\\', "/");
        let rel = rel.trim_start_matches("./");
        if rel.is_empty() || rel.split('/').any(|part| part == "..") {
            continue;
        }
        let Some(local) = mirror.local(&format!("{}/{rel}", root.trim_end_matches('/'))) else { continue };
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)?;
        if std::fs::read(&local).ok().as_deref() != Some(bytes.as_slice()) {
            rc_core::paths::write_atomic(&local, &bytes)?;
        }
        hashes.insert(local, rc_remote::deploy::sha256_hex(&bytes));
    }
    Ok(hashes)
}

/// Every file under `dir` (for packing a folder unpacked here).
fn files_under(dir: &Path) -> anyhow::Result<Vec<(String, Vec<u8>, bool)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(dir)?.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            #[cfg(unix)]
            let exec = {
                use std::os::unix::fs::PermissionsExt;
                entry.metadata()?.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let exec = rel == "netcoredbg" || rel.ends_with(".so") || rel.ends_with(".dylib");
            out.push((rel, std::fs::read(&path)?, exec));
        }
    }
    Ok(out)
}

impl Backend {
    /// The local copy of a remote file (fetching it is [`Backend::fetch`]).
    pub fn local(&self, remote: &str) -> Option<PathBuf> {
        self.mirror.local(remote)
    }

    /// Folders to put first on the PATH of remote tools: the configured `dotnet`'s, so the
    /// language server's project loader runs the same SDK.
    fn dotnet_path(&self) -> Vec<String> {
        posix_dir(&self.dotnet).filter(|_| self.dotnet.contains('/')).map(|d| vec![d.to_string()]).unwrap_or_default()
    }

    /// Copies a remote file into the mirror (left untouched when it already has those bytes,
    /// so open editors do not reload) and returns the local path and the bytes.
    pub async fn fetch(&self, remote: &str) -> anyhow::Result<(PathBuf, Vec<u8>)> {
        let local = self.local(remote).ok_or_else(|| anyhow::anyhow!("not an absolute remote path: {remote}"))?;
        let bytes = capture(&self.client, spawn_op("cat", &["--", remote], "/", Vec::new()), None).await?.ok(&format!("reading {remote}"))?;
        if std::fs::read(&local).ok().as_deref() != Some(bytes.as_slice()) {
            if let Some(parent) = local.parent() {
                std::fs::create_dir_all(parent)?;
            }
            rc_core::paths::write_atomic(&local, &bytes)?;
        }
        Ok((local, bytes))
    }

    /// Writes a local copy back to the remote machine and returns the SHA-256 of what was
    /// written. With `expected` (the remote file's SHA-256 when it was copied here), a file
    /// that changed on the remote machine since is left alone and the push fails.
    pub async fn push(&self, local: &Path, expected: Option<&str>) -> anyhow::Result<String> {
        let remote = self.mirror.remote(local).ok_or_else(|| anyhow::anyhow!("{} is not a remote file", local.display()))?;
        let bytes = std::fs::read(local)?;
        let hash = rc_remote::deploy::sha256_hex(&bytes);
        let script = "f=\"$1\"; e=\"$2\"; \
            if [ -n \"$e\" ] && [ -f \"$f\" ]; then \
            if command -v sha256sum >/dev/null 2>&1; then a=$(sha256sum \"$f\"); \
            elif command -v shasum >/dev/null 2>&1; then a=$(shasum -a 256 \"$f\"); else a=\"$e\"; fi; \
            a=${a%% *}; if [ \"$a\" != \"$e\" ]; then exit 75; fi; fi; \
            cat > \"$f\"";
        let out = capture(&self.client, spawn_op("sh", &["-c", script, "sh", &remote, expected.unwrap_or("")], "/", Vec::new()), Some(bytes)).await?;
        if out.code == Some(75) {
            anyhow::bail!("{remote} changed on the remote machine; open it again from the Explorer to get its new text");
        }
        out.ok(&format!("writing {remote}"))?;
        Ok(hash)
    }

    /// Copies the whole open folder into the mirror in one archive (build output, `.git` and
    /// tool folders left out), so navigation, renames and searches find every file. Returns
    /// the SHA-256 of each file copied.
    pub async fn sync_folder(&self) -> anyhow::Result<HashMap<PathBuf, String>> {
        let mut args = vec!["-czf", "-"];
        for skip in SYNC_SKIP {
            args.push(skip);
        }
        args.extend(["-C", &self.root, "."]);
        let archive = capture(&self.client, spawn_op("tar", &args, "/", Vec::new()), None).await?.ok("copying the folder")?;
        let (mirror, root) = (self.mirror.clone(), self.root.clone());
        tokio::task::spawn_blocking(move || unpack_into_mirror(&archive, &mirror, &root)).await?
    }

    pub async fn exists(&self, remote: &str) -> anyhow::Result<bool> {
        Ok(capture(&self.client, spawn_op("sh", &["-c", "test -e \"$1\"", "sh", remote], "/", Vec::new()), None).await?.code == Some(0))
    }

    /// Unpacks `files` into the remote folder `dir` (replacing it; moved in place last).
    pub async fn install_tree(&self, dir: &str, files: &[(String, Vec<u8>, bool)]) -> anyhow::Result<()> {
        let archive = tar_gz(files)?;
        let script = "set -e; d=\"$1\"; rm -rf \"$d.part\"; mkdir -p \"$d.part\"; tar -xzf - -C \"$d.part\"; rm -rf \"$d\"; mv \"$d.part\" \"$d\"";
        capture(&self.client, spawn_op("sh", &["-c", script, "sh", dir], "/", Vec::new()), Some(archive)).await?.ok(&format!("unpacking into {dir}"))?;
        Ok(())
    }

    /// The Roslyn language server on the remote machine, installed there the first time
    /// (downloaded here for its platform and copied over).
    pub async fn ensure_roslyn(&self) -> anyhow::Result<String> {
        if let Some(dll) = &self.tools.roslyn_dll {
            return Ok(dll.clone());
        }
        let dll = rc_lsp::roslyn::remote_dll();
        if self.exists(&dll).await? {
            return Ok(dll);
        }
        let rid = rc_lsp::roslyn::rid_for_uname(&self.machine.uname).ok_or_else(|| anyhow::anyhow!("there is no C# language server for {}", self.machine.uname))?;
        let package = rc_lsp::roslyn::fetch_package(rid).await?;
        let files = tokio::task::spawn_blocking(move || rc_lsp::roslyn::package_files(&package, rid)).await??;
        self.install_tree(&rc_lsp::roslyn::remote_dir(), &files).await?;
        Ok(dll)
    }

    /// netcoredbg on the remote machine (the release its glibc runs), installed there the
    /// first time.
    pub async fn ensure_netcoredbg(&self) -> anyhow::Result<String> {
        if let Some(exe) = &self.tools.netcoredbg {
            return Ok(exe.clone());
        }
        let (release, asset) = rc_dap::remote::remote_release(&self.machine.uname, self.machine.glibc.as_deref())
            .ok_or_else(|| anyhow::anyhow!("netcoredbg has no build for {}", self.machine.uname))?;
        let dir = rc_dap::remote::remote_dir(release);
        let exe = format!("{dir}/netcoredbg");
        if self.exists(&exe).await? {
            return Ok(exe);
        }
        let url = rc_dap::netcoredbg::asset_url(release, asset);
        let archive = reqwest::Client::builder().build()?.get(&url).send().await?.error_for_status()?.bytes().await?.to_vec();
        let files = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let unpacked = tempfile::tempdir()?;
            let dest = unpacked.path().join("netcoredbg");
            rc_dap::netcoredbg::install(&archive, asset, &dest)?;
            files_under(&dest)
        })
        .await??;
        self.install_tree(&dir, &files).await?;
        Ok(exe)
    }

    /// Starts the language server (`dll`) on the remote machine, in the folder.
    pub fn lsp_transport(&self, dll: String) -> impl std::future::Future<Output = std::io::Result<rc_lsp::Transport>> + Send + 'static {
        let args = rc_lsp::roslyn::remote_args(&dll, "~/.dotcraft/remote/logs/roslyn");
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let operation = spawn_op(&self.dotnet, &args, &self.root, self.dotnet_path());
        let client = self.client.clone();
        async move {
            let mut process = client.spawn(operation).await.map_err(std::io::Error::other)?;
            let (stdin, stdout, stderr) = pipes(&mut process);
            let killer = process.killer();
            Ok(rc_lsp::Transport { stdin, stdout, stderr, exit: Box::pin(async move { process.wait().await }), kill: Arc::new(move || killer.kill()) })
        }
    }

    /// Starts netcoredbg (`exe`) on the remote machine as a DAP adapter.
    pub fn dap_transport(&self, exe: String) -> impl std::future::Future<Output = std::io::Result<rc_dap::Transport>> + Send + 'static {
        let operation = spawn_op(&exe, &["--interpreter=vscode"], &self.root, self.dotnet_path());
        let client = self.client.clone();
        async move {
            let mut process = client.spawn(operation).await.map_err(std::io::Error::other)?;
            let (stdin, stdout, stderr) = pipes(&mut process);
            let killer = process.killer();
            Ok(rc_dap::Transport { stdin, stdout, stderr, exit: Box::pin(async move { process.wait().await }), kill: Arc::new(move || killer.kill()) })
        }
    }

    /// The full path of the remote `dotnet` (netcoredbg starts it without a shell).
    pub async fn dotnet_program(&self) -> anyhow::Result<String> {
        if let Some(rest) = self.dotnet.strip_prefix("~/") {
            return Ok(format!("{}/{rest}", self.machine.home.trim_end_matches('/')));
        }
        if self.dotnet.starts_with('/') {
            return Ok(self.dotnet.clone());
        }
        let out = capture(&self.client, spawn_op("sh", &["-c", "command -v \"$1\"", "sh", &self.dotnet], "/", Vec::new()), None).await?.ok("finding dotnet")?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// What netcoredbg launches for a project (already built): `dotnet <TargetPath>` in its
    /// run folder.
    pub async fn launch_config(&self, project: &str) -> anyhow::Result<rc_dap::LaunchConfig> {
        let args = ["msbuild", project, "-getProperty:TargetPath", "-getProperty:RunWorkingDirectory", "-getProperty:OutputType"];
        let out = capture(&self.client, spawn_op(&self.dotnet, &args, &self.root, self.dotnet_path()), None).await?.ok("dotnet msbuild")?;
        let json: serde_json::Value = serde_json::from_slice(&out).map_err(|e| anyhow::anyhow!("unexpected dotnet msbuild output: {e}"))?;
        let property = |name: &str| json["Properties"][name].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        let output = property("OutputType").unwrap_or_default();
        anyhow::ensure!(output.eq_ignore_ascii_case("exe") || output.eq_ignore_ascii_case("winexe"), "{project} is not an executable project");
        let target = property("TargetPath").ok_or_else(|| anyhow::anyhow!("{project} has no TargetPath"))?;
        let cwd = property("RunWorkingDirectory").unwrap_or_else(|| posix_dir(project).unwrap_or("/").to_string());
        Ok(rc_dap::LaunchConfig { program: self.dotnet_program().await?.into(), args: vec![target], cwd: cwd.into(), env: Vec::new() })
    }

    /// `dotnet <args>` in the folder with its output streamed: the process, whose stdout and
    /// stderr the caller reads.
    pub async fn dotnet_stream(&self, args: &[&str]) -> anyhow::Result<RemoteProcess> {
        self.client.spawn(spawn_op(&self.dotnet, args, &self.root, self.dotnet_path())).await
    }
}

type Pipes = (Box<dyn tokio::io::AsyncWrite + Send + Unpin>, Box<dyn tokio::io::AsyncRead + Send + Unpin>, Box<dyn tokio::io::AsyncRead + Send + Unpin>);

fn pipes(process: &mut RemoteProcess) -> Pipes {
    (Box::new(process.stdin.take().expect("stdin")), process.stdout.take().expect("stdout"), process.stderr.take().expect("stderr"))
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    /// An agent served in this process: the "remote machine" is this one.
    pub fn local_client() -> Client {
        let _guard = rc_core::Runtime::handle().enter();
        let (client, server) = tokio::io::duplex(1 << 20);
        let (read, write) = tokio::io::split(client);
        let (server_read, server_write) = tokio::io::split(server);
        rc_core::Runtime::spawn(async move {
            let _ = rc_remote::serve(server_read, server_write).await;
        });
        Client::new(read, write)
    }

    /// The Roslyn and netcoredbg of `target/test-home`, used as the remote ones.
    pub fn test_tools() -> Tools {
        let home = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home").canonicalize().expect("run the rc-lsp and rc-dap tests first");
        let roslyn = home.join("tools/roslyn").join(rc_lsp::roslyn::PINNED_VERSION).join("Microsoft.CodeAnalysis.LanguageServer.dll");
        let netcoredbg = std::fs::read_dir(home.join("tools/netcoredbg")).ok().and_then(|d| d.flatten().map(|e| e.path().join("netcoredbg")).find(|p| p.is_file()));
        Tools { roslyn_dll: roslyn.is_file().then(|| roslyn.display().to_string()), netcoredbg: netcoredbg.map(|p| p.display().to_string()) }
    }

    pub fn backend(root: &Path, mirror: &Path) -> Backend {
        let client = local_client();
        let machine = rc_core::Runtime::block_on(probe(&client)).unwrap();
        let dotnet = rc_dotnet::Dotnet::locate(None).map(|d| d.path.display().to_string()).unwrap_or_else(|| "dotnet".into());
        Backend { client, mirror: Mirror::new(mirror), root: root.display().to_string(), dotnet, machine, tools: test_tools() }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::test_support::*;
    use super::*;
    use rc_core::Runtime;

    #[test]
    fn files_round_trip_through_the_mirror() {
        let remote = tempfile::tempdir().unwrap();
        let mirror = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(remote.path()).unwrap();
        std::fs::write(root.join("Program.cs"), "class A {}\n").unwrap();
        let backend = backend(&root, mirror.path());
        let remote_file = format!("{}/Program.cs", root.display());
        let (local, bytes) = Runtime::block_on(backend.fetch(&remote_file)).unwrap();
        assert!(local.starts_with(mirror.path()));
        assert_eq!(bytes, b"class A {}\n");
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "class A {}\n");
        std::fs::write(&local, "class B {}\n").unwrap();
        let first = rc_remote::deploy::sha256_hex(&bytes);
        let second = Runtime::block_on(backend.push(&local, Some(&first))).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("Program.cs")).unwrap(), "class B {}\n");
        // Changed on the remote machine meanwhile: the push leaves it alone.
        std::fs::write(root.join("Program.cs"), "class Remote {}\n").unwrap();
        std::fs::write(&local, "class C {}\n").unwrap();
        let error = Runtime::block_on(backend.push(&local, Some(&second))).unwrap_err();
        assert!(error.to_string().contains("changed on the remote machine"), "{error:#}");
        assert_eq!(std::fs::read_to_string(root.join("Program.cs")).unwrap(), "class Remote {}\n");
        // The whole folder in one go, without build output.
        std::fs::create_dir_all(root.join("src/bin")).unwrap();
        std::fs::write(root.join("src/App.cs"), "class App {}\n").unwrap();
        std::fs::write(root.join("src/bin/App.dll"), "binary").unwrap();
        let hashes = Runtime::block_on(backend.sync_folder()).unwrap();
        let app = backend.local(&format!("{}/src/App.cs", root.display())).unwrap();
        assert_eq!(std::fs::read_to_string(&app).unwrap(), "class App {}\n");
        assert_eq!(hashes.get(&app).map(String::as_str), Some(rc_remote::deploy::sha256_hex(b"class App {}\n").as_str()));
        assert!(!backend.local(&format!("{}/src/bin/App.dll", root.display())).unwrap().exists());
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "class Remote {}\n");
        assert!(Runtime::block_on(backend.exists(&remote_file)).unwrap());
        assert!(!Runtime::block_on(backend.exists(&format!("{}/Missing.cs", root.display()))).unwrap());
        let error = Runtime::block_on(backend.fetch(&format!("{}/Missing.cs", root.display()))).unwrap_err();
        assert!(error.to_string().contains("Missing.cs"), "{error:#}");
        assert!(!backend.machine.uname.is_empty() && !backend.machine.home.is_empty());
    }

    #[test]
    fn folders_are_installed_whole() {
        let remote = tempfile::tempdir().unwrap();
        let mirror = tempfile::tempdir().unwrap();
        let backend = backend(remote.path(), mirror.path());
        let dir = remote.path().join("tools/x");
        let files = vec![("bin/tool".to_string(), b"#!/bin/sh\necho tool\n".to_vec(), true), ("lib/data.txt".to_string(), b"data".to_vec(), false)];
        Runtime::block_on(backend.install_tree(&dir.display().to_string(), &files)).unwrap();
        let out = std::process::Command::new(dir.join("bin/tool")).output().unwrap();
        assert_eq!(out.stdout, b"tool\n");
        assert_eq!(std::fs::read(dir.join("lib/data.txt")).unwrap(), b"data");
        // Installing again replaces the folder.
        Runtime::block_on(backend.install_tree(&dir.display().to_string(), &files[1..])).unwrap();
        assert!(!dir.join("bin/tool").exists());
    }
}
