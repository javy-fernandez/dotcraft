//! Puts the agent of this DotCraft version on the remote machine, like VS Code Remote and
//! Rider's backend do: on connect DotCraft asks the remote for its OS and architecture,
//! looks for `~/.dotcraft/remote/<version>/dotcraft-remote-agent`, and when it is missing
//! installs the release asset of that version: downloaded by the remote machine itself
//! (curl or wget), or downloaded here and copied over the same SSH or WSL channel. The
//! archive is checked against its published SHA-256 before it is unpacked.
//!
//! Everything runs through [`RemoteShell`] (one `sh -c` per step) and [`Fetch`] (local
//! downloads), so tests drive the whole flow on this machine.

use crate::Endpoint;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{future::Future, path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

/// Where releases (and their agent archives) are published.
pub const RELEASES: &str = "https://github.com/javy-fernandez/dotcraft/releases/download";
/// The agent's executable inside the archive and the install folder.
pub const AGENT_FILE: &str = "dotcraft-remote-agent";
/// How to build the agent by hand on the remote machine (needs a Rust toolchain there).
pub fn manual_install_command(version: &str) -> String {
    format!("cargo install --locked --git https://github.com/javy-fernandez/dotcraft --tag v{version} rc-remote")
}

/// Longest an install step may take: a slow download of a few MiB.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// The machines release assets exist for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    LinuxX64,
    LinuxArm64,
    /// One universal binary for Intel and Apple silicon Macs.
    MacosUniversal,
}

impl Platform {
    /// Reads `uname -sm` (`Linux x86_64`, `Linux aarch64`, `Darwin arm64`…).
    pub fn from_uname(uname: &str) -> Result<Self> {
        let mut words = uname.split_whitespace();
        let (Some(os), Some(machine)) = (words.next(), words.next()) else {
            bail!("unexpected answer to uname: {:?}", uname.trim());
        };
        let machine = machine.to_ascii_lowercase();
        Ok(match (os, machine.as_str()) {
            ("Linux", "x86_64" | "amd64") => Platform::LinuxX64,
            ("Linux", "aarch64" | "arm64" | "armv8l" | "armv8b") => Platform::LinuxArm64,
            ("Darwin", "x86_64" | "arm64") => Platform::MacosUniversal,
            _ => bail!("there is no DotCraft agent for {os} {machine} (only Linux x64 and ARM64, and macOS)"),
        })
    }

    /// The suffix of the asset name.
    pub fn name(self) -> &'static str {
        match self {
            Platform::LinuxX64 => "linux-x64",
            Platform::LinuxArm64 => "linux-arm64",
            Platform::MacosUniversal => "macos-universal",
        }
    }
}

/// One DotCraft release: its version and where its assets are downloaded from.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    /// `RELEASES`, or a mirror; assets live in `<base>/v<version>/`.
    pub base_url: String,
}

impl Release {
    /// The release of this build: app and agent always have the same version.
    pub fn current() -> Self {
        Release { version: env!("CARGO_PKG_VERSION").into(), base_url: RELEASES.into() }
    }

    pub fn asset(&self, platform: Platform) -> String {
        format!("rc-remote-{}-{}.tar.gz", self.version, platform.name())
    }

    pub fn url(&self, platform: Platform) -> String {
        format!("{}/v{}/{}", self.base_url.trim_end_matches('/'), self.version, self.asset(platform))
    }

    pub fn checksum_url(&self, platform: Platform) -> String {
        format!("{}.sha256", self.url(platform))
    }

    /// The install folder, relative to the remote home: `.dotcraft/remote/<version>`.
    pub fn dir(&self) -> String {
        format!(".dotcraft/remote/{}", self.version)
    }

    /// The agent path to launch, `~/.dotcraft/remote/<version>/dotcraft-remote-agent`.
    pub fn agent_path(&self) -> String {
        format!("~/{}/{AGENT_FILE}", self.dir())
    }

    /// Versions and URLs end up in shell scripts: only allow characters that need no quoting
    /// inside double quotes.
    fn check(&self) -> Result<()> {
        let version_ok = !self.version.is_empty()
            && self.version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
        let url_ok = !self.base_url.is_empty()
            && self.base_url.chars().all(|c| c.is_ascii_alphanumeric() || "/:._-~%@".contains(c));
        if !version_ok || !url_ok {
            bail!("invalid release version or download address");
        }
        Ok(())
    }
}

/// The first word of a `sha256sum` line, if it is a SHA-256 in hex.
pub fn parse_checksum(text: &str) -> Option<String> {
    let hash = text.split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What a remote command printed and how it ended.
#[derive(Clone, Debug, Default)]
pub struct ShellOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a POSIX `sh` script on the remote machine, with `input` on its stdin.
pub trait RemoteShell: Sync {
    fn run(&self, script: &str, input: Option<Vec<u8>>, timeout: Duration) -> impl Future<Output = Result<ShellOutput>> + Send;
}

/// Downloads a file on this machine.
pub trait Fetch: Sync {
    fn get(&self, url: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;
}

impl RemoteShell for Endpoint {
    async fn run(&self, script: &str, input: Option<Vec<u8>>, timeout: Duration) -> Result<ShellOutput> {
        run_command(self.shell_command(script)?, input, timeout).await
    }
}

/// Runs scripts on this machine with `home` as `$HOME`: the "remote" of the tests, through
/// the same steps as SSH and WSL.
pub struct LocalShell {
    pub home: PathBuf,
    /// Quote the script for a login shell first, like the SSH launcher does.
    pub like_ssh: bool,
}

impl RemoteShell for LocalShell {
    async fn run(&self, script: &str, input: Option<Vec<u8>>, timeout: Duration) -> Result<ShellOutput> {
        let script = if self.like_ssh { format!("sh -c {}", crate::shell_word(script)) } else { script.to_string() };
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script).env("HOME", &self.home);
        run_command(cmd, input, timeout).await
    }
}

async fn run_command(mut cmd: Command, input: Option<Vec<u8>>, timeout: Duration) -> Result<ShellOutput> {
    let mut child = cmd
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("could not start the remote launcher")?;
    // Write while the output is read, so neither side blocks on a full pipe.
    let writer = match (input, child.stdin.take()) {
        (Some(bytes), Some(mut stdin)) => Some(tokio::spawn(async move {
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.shutdown().await;
        })),
        _ => None,
    };
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(output) => output?,
        Err(_) => bail!("the remote machine did not answer in time"),
    };
    if let Some(writer) = writer {
        writer.abort();
    }
    Ok(ShellOutput {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: crate::launcher_text(&output.stderr).trim().to_string(),
    })
}

/// The remote machine could not be reached (ssh or wsl.exe failed before any script ran):
/// a connection problem, not an install problem.
#[derive(Debug)]
pub struct Unreachable(pub String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unreachable {}

/// What deployment is doing, for the connection dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Checking,
    /// The remote machine downloads the agent itself.
    DownloadingOnRemote,
    /// DotCraft downloads it here…
    Downloading,
    /// …and copies it over the SSH or WSL connection.
    Uploading,
}

/// What the remote machine has, from [`probe_script`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub uname: String,
    pub installed: bool,
    pub curl: bool,
    pub wget: bool,
    pub hasher: bool,
    pub tar: bool,
}

/// One round trip: the platform, whether this version's agent is there, and the tools the
/// install needs. Markers keep banners printed by login scripts out of the way.
pub fn probe_script(release: &Release) -> String {
    format!(
        "echo \"dotcraft:uname:$(uname -sm)\"; \
         if [ -x \"$HOME/{dir}/{AGENT_FILE}\" ]; then echo dotcraft:installed; fi; \
         for t in curl wget sha256sum shasum tar; do \
         if command -v \"$t\" >/dev/null 2>&1; then echo \"dotcraft:has:$t\"; fi; done",
        dir = release.dir()
    )
}

pub fn parse_probe(stdout: &str) -> Probe {
    let mut probe = Probe::default();
    for line in stdout.lines().map(str::trim) {
        let Some(rest) = line.strip_prefix("dotcraft:") else { continue };
        match rest {
            "installed" => probe.installed = true,
            "has:curl" => probe.curl = true,
            "has:wget" => probe.wget = true,
            "has:sha256sum" | "has:shasum" => probe.hasher = true,
            "has:tar" => probe.tar = true,
            _ => {
                if let Some(uname) = rest.strip_prefix("uname:") {
                    probe.uname = uname.trim().to_string();
                }
            }
        }
    }
    probe
}

/// Where the install script gets the archive from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Curl,
    Wget,
    /// Read from stdin (copied by DotCraft, already checked here).
    Upload,
}

/// Exit codes of [`install_script`].
const EXIT_DOWNLOAD: i32 = 71;
const EXIT_NO_HASHER: i32 = 72;
const EXIT_MISMATCH: i32 = 73;
const EXIT_UNPACK: i32 = 74;

/// Installs the agent into `~/.dotcraft/remote/<version>/`: fetch the archive into a
/// temporary folder next to it, check its SHA-256 (`expected`, or the published `.sha256`
/// when DotCraft could not get it), unpack, make it executable and move it in place last, so
/// an interrupted install never leaves a half-written agent behind. One line, no `!`:
/// it goes through the user's login shell first.
pub fn install_script(release: &Release, platform: Platform, source: Source, expected: Option<&str>) -> String {
    let (url, checksum_url) = (release.url(platform), release.checksum_url(platform));
    let fetch = |url: &str, to: &str| match source {
        Source::Curl => format!("curl -fsSL --retry 2 --connect-timeout 20 -o \"$t/{to}\" \"{url}\""),
        Source::Wget => format!("wget -q -T 20 -O \"$t/{to}\" \"{url}\""),
        Source::Upload => format!("cat > \"$t/{to}\""),
    };
    let mut lines = vec![
        "set -e".to_string(),
        format!("d=\"$HOME/{}\"", release.dir()),
        "t=\"$d.part.$$\"".to_string(),
        "mkdir -p \"$t\"".to_string(),
        "trap 'rm -rf \"$t\"' EXIT".to_string(),
        format!("{} || exit {EXIT_DOWNLOAD}", fetch(&url, "agent.tar.gz")),
    ];
    match expected {
        Some(hash) => lines.push(format!("e={hash}")),
        None => {
            lines.push(format!("{} || exit {EXIT_DOWNLOAD}", fetch(&checksum_url, "agent.sha256")));
            lines.push("e=$(cut -d ' ' -f 1 < \"$t/agent.sha256\")".into());
        }
    }
    // An upload was checked before it left this machine: a remote without a hash tool is fine.
    let no_hasher = if source == Source::Upload { "s=$e".to_string() } else { format!("exit {EXIT_NO_HASHER}") };
    lines.extend([
        format!(
            "if command -v sha256sum >/dev/null 2>&1; then s=$(sha256sum \"$t/agent.tar.gz\"); \
             elif command -v shasum >/dev/null 2>&1; then s=$(shasum -a 256 \"$t/agent.tar.gz\"); \
             else {no_hasher}; fi"
        ),
        "s=${s%% *}".into(),
        format!("if [ \"$s\" = \"$e\" ]; then :; else echo \"expected $e, got $s\" >&2; exit {EXIT_MISMATCH}; fi"),
        format!("tar -xzf \"$t/agent.tar.gz\" -C \"$t\" {AGENT_FILE} || exit {EXIT_UNPACK}"),
        format!("chmod 755 \"$t/{AGENT_FILE}\""),
        "mkdir -p \"$d\"".into(),
        format!("mv -f \"$t/{AGENT_FILE}\" \"$d/{AGENT_FILE}\""),
    ]);
    lines.join("; ")
}

/// Why an install attempt failed, from its exit code.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// Worth trying the other way (download here and copy).
    Recoverable(String),
    Fatal(String),
}

fn classify(output: &ShellOutput, release: &Release, platform: Platform) -> Option<Failure> {
    let detail = |text: &str| if output.stderr.is_empty() { text.to_string() } else { format!("{text}: {}", output.stderr) };
    match output.code {
        Some(0) => None,
        Some(EXIT_DOWNLOAD) => Some(Failure::Recoverable(detail(&format!("could not download {}", release.url(platform))))),
        Some(EXIT_NO_HASHER) => Some(Failure::Recoverable("the remote machine has neither sha256sum nor shasum".into())),
        Some(EXIT_MISMATCH) => Some(Failure::Fatal(detail("the downloaded agent does not match its published SHA-256 checksum"))),
        Some(EXIT_UNPACK) => Some(Failure::Fatal(detail("could not unpack the agent (is tar installed?)"))),
        _ => Some(Failure::Fatal(detail("could not install the agent"))),
    }
}

/// Makes sure this version's agent is on the remote machine and returns the path to launch.
/// `progress` hears each step.
pub async fn ensure_agent<S: RemoteShell, F: Fetch>(shell: &S, fetch: &F, release: &Release, progress: &(dyn Fn(Step) + Sync)) -> Result<String> {
    release.check()?;
    progress(Step::Checking);
    let output = shell.run(&probe_script(release), None, PROBE_TIMEOUT).await.map_err(|e| Unreachable(format!("{e:#}")))?;
    let probe = parse_probe(&output.stdout);
    if probe.uname.is_empty() {
        let detail = if output.stderr.is_empty() { output.stdout.trim().to_string() } else { output.stderr };
        return Err(Unreachable(if detail.is_empty() { "could not reach the remote machine".into() } else { detail }).into());
    }
    if probe.installed {
        return Ok(release.agent_path());
    }
    let platform = Platform::from_uname(&probe.uname)?;
    if !probe.tar {
        bail!("the remote machine has no tar to unpack the agent");
    }
    // The checksum comes from GitHub over this machine's HTTPS connection when it can.
    let expected = match fetch.get(&release.checksum_url(platform)).await {
        Ok(bytes) => parse_checksum(&String::from_utf8_lossy(&bytes)),
        Err(_) => None,
    };
    let mut remote_error = None;
    let source = if probe.curl { Some(Source::Curl) } else if probe.wget { Some(Source::Wget) } else { None };
    if let Some(source) = source.filter(|_| probe.hasher) {
        progress(Step::DownloadingOnRemote);
        let output = shell.run(&install_script(release, platform, source, expected.as_deref()), None, INSTALL_TIMEOUT).await?;
        match classify(&output, release, platform) {
            None => return Ok(release.agent_path()),
            Some(Failure::Fatal(message)) => bail!(message),
            Some(Failure::Recoverable(message)) => remote_error = Some(message),
        }
    }
    progress(Step::Downloading);
    let local = async {
        let expected = expected.context("could not download the agent's checksum")?;
        let archive = fetch.get(&release.url(platform)).await.with_context(|| format!("could not download {}", release.url(platform)))?;
        let actual = sha256_hex(&archive);
        if actual != expected {
            bail!("the downloaded agent does not match its published SHA-256 checksum (expected {expected}, got {actual})");
        }
        Ok((archive, expected))
    }
    .await;
    let (archive, expected) = match (local, remote_error) {
        (Ok(found), _) => found,
        (Err(error), Some(remote)) => bail!("{remote}\n{error:#}"),
        (Err(error), None) => return Err(error),
    };
    progress(Step::Uploading);
    let output = shell.run(&install_script(release, platform, Source::Upload, Some(&expected)), Some(archive), INSTALL_TIMEOUT).await?;
    match classify(&output, release, platform) {
        None => Ok(release.agent_path()),
        Some(Failure::Fatal(message) | Failure::Recoverable(message)) => bail!(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> Release {
        Release { version: "0.7.1".into(), base_url: RELEASES.into() }
    }

    #[test]
    fn uname_maps_to_release_platforms() {
        for (uname, platform) in [
            ("Linux x86_64", Platform::LinuxX64),
            ("Linux amd64\n", Platform::LinuxX64),
            ("Linux aarch64", Platform::LinuxArm64),
            ("Linux arm64", Platform::LinuxArm64),
            ("Darwin arm64", Platform::MacosUniversal),
            ("Darwin x86_64", Platform::MacosUniversal),
        ] {
            assert_eq!(Platform::from_uname(uname).unwrap(), platform, "{uname}");
        }
        for uname in ["Linux armv7l", "Linux i686", "FreeBSD amd64", "", "Linux"] {
            assert!(Platform::from_uname(uname).is_err(), "{uname}");
        }
    }

    #[test]
    fn asset_urls_and_install_folder_follow_the_version() {
        let r = release();
        assert_eq!(r.asset(Platform::LinuxArm64), "rc-remote-0.7.1-linux-arm64.tar.gz");
        assert_eq!(
            r.url(Platform::LinuxX64),
            "https://github.com/javy-fernandez/dotcraft/releases/download/v0.7.1/rc-remote-0.7.1-linux-x64.tar.gz"
        );
        assert_eq!(r.checksum_url(Platform::MacosUniversal), format!("{}.sha256", r.url(Platform::MacosUniversal)));
        assert_eq!(r.agent_path(), "~/.dotcraft/remote/0.7.1/dotcraft-remote-agent");
        assert!(manual_install_command("0.7.1").contains("--tag v0.7.1"));
        assert_eq!(Release::current().version, env!("CARGO_PKG_VERSION"));
        assert!(r.check().is_ok());
        for (version, base) in [("0.7.1; rm -rf ~", RELEASES), ("", RELEASES), ("0.7.1", "https://x/$(id)"), ("0.7.1", "https://x/\"a")] {
            assert!(Release { version: version.into(), base_url: base.into() }.check().is_err(), "{version} {base}");
        }
    }

    #[test]
    fn checksums_are_read_from_sha256sum_lines() {
        let hash = sha256_hex(b"abc");
        assert_eq!(hash, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(parse_checksum(&format!("{}  rc-remote-0.7.1-linux-x64.tar.gz\n", hash.to_uppercase())), Some(hash.clone()));
        assert_eq!(parse_checksum(&hash), Some(hash));
        assert_eq!(parse_checksum("abc  file"), None);
        assert_eq!(parse_checksum(""), None);
        assert_eq!(parse_checksum(&"g".repeat(64)), None);
    }

    #[test]
    fn probe_ignores_login_banners() {
        let probe = parse_probe("Welcome to Ubuntu!\ndotcraft:uname:Linux x86_64\ndotcraft:has:wget\ndotcraft:has:shasum\ndotcraft:has:tar\n");
        assert_eq!(
            probe,
            Probe { uname: "Linux x86_64".into(), installed: false, curl: false, wget: true, hasher: true, tar: true }
        );
        assert!(parse_probe("dotcraft:installed\n").installed);
    }

    #[test]
    fn scripts_are_one_line_without_history_expansion() {
        let r = release();
        for script in [
            probe_script(&r),
            install_script(&r, Platform::LinuxX64, Source::Curl, None),
            install_script(&r, Platform::LinuxX64, Source::Wget, Some(&"a".repeat(64))),
            install_script(&r, Platform::LinuxArm64, Source::Upload, Some(&"a".repeat(64))),
        ] {
            assert!(!script.contains('\n') && !script.contains('!'), "{script}");
        }
    }

    #[test]
    fn failures_say_whether_the_other_way_is_worth_trying() {
        let r = release();
        let out = |code| ShellOutput { code: Some(code), stdout: String::new(), stderr: "curl: (6) Could not resolve host".into() };
        assert_eq!(classify(&out(0), &r, Platform::LinuxX64), None);
        assert!(matches!(classify(&out(EXIT_DOWNLOAD), &r, Platform::LinuxX64), Some(Failure::Recoverable(m)) if m.contains("Could not resolve host")));
        assert!(matches!(classify(&out(EXIT_NO_HASHER), &r, Platform::LinuxX64), Some(Failure::Recoverable(_))));
        assert!(matches!(classify(&out(EXIT_MISMATCH), &r, Platform::LinuxX64), Some(Failure::Fatal(m)) if m.contains("SHA-256")));
        assert!(matches!(classify(&out(1), &r, Platform::LinuxX64), Some(Failure::Fatal(_))));
    }

    /// ssh refused the login: nothing ran, and that is a connection problem.
    #[tokio::test]
    async fn an_unreachable_machine_is_not_an_install_failure() {
        struct Refused;
        impl RemoteShell for Refused {
            async fn run(&self, _: &str, _: Option<Vec<u8>>, _: Duration) -> Result<ShellOutput> {
                Ok(ShellOutput { code: Some(255), stdout: String::new(), stderr: "dev@box: Permission denied (publickey).".into() })
            }
        }
        struct Offline;
        impl Fetch for Offline {
            async fn get(&self, _: &str) -> Result<Vec<u8>> {
                bail!("offline")
            }
        }
        let error = ensure_agent(&Refused, &Offline, &release(), &|_| {}).await.unwrap_err();
        let unreachable = error.downcast_ref::<Unreachable>().expect("an Unreachable error");
        assert_eq!(unreachable.0, "dev@box: Permission denied (publickey).");
    }

    #[test]
    fn ssh_and_wsl_run_scripts_through_sh() {
        let args = |endpoint: Endpoint| -> Vec<String> {
            endpoint.shell_command("echo \"$HOME\"").unwrap().as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
        };
        let ssh = args(Endpoint::Ssh { host: "dev@box".into(), agent: String::new() });
        assert!(ssh.contains(&"BatchMode=yes".to_string()));
        assert_eq!(ssh[ssh.len() - 3..], ["--", "dev@box", "sh -c 'echo \"$HOME\"'"]);
        let wsl = args(Endpoint::Wsl { distribution: "Ubuntu".into(), agent: String::new() });
        assert_eq!(wsl, ["--distribution", "Ubuntu", "--exec", "sh", "-c", "echo \"$HOME\""]);
        assert!(Endpoint::Ssh { host: "-oProxyCommand=x".into(), agent: String::new() }.shell_command("true").is_err());
    }
}

/// The whole flow on this machine: a fake release folder served over `file://` (curl reads
/// it like a web server) and a fake home folder.
#[cfg(all(test, unix))]
mod local_tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    /// Serves files from memory and counts what was downloaded.
    struct Files(HashMap<String, Vec<u8>>, Mutex<Vec<String>>);

    impl Fetch for Files {
        async fn get(&self, url: &str) -> Result<Vec<u8>> {
            self.1.lock().unwrap().push(url.to_string());
            self.0.get(url).cloned().context("404")
        }
    }

    /// A release folder with an archive whose agent is a script that prints `version`.
    fn publish(dir: &std::path::Path, release: &Release, platform: Platform, version: &str) -> (Vec<u8>, String) {
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join(AGENT_FILE), format!("#!/bin/sh\necho {version}\n")).unwrap();
        let folder = dir.join("releases").join(format!("v{}", release.version));
        std::fs::create_dir_all(&folder).unwrap();
        let archive = folder.join(release.asset(platform));
        let status = std::process::Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&stage).arg(AGENT_FILE).status().unwrap();
        assert!(status.success());
        let bytes = std::fs::read(&archive).unwrap();
        let hash = sha256_hex(&bytes);
        std::fs::write(folder.join(format!("{}.sha256", release.asset(platform))), format!("{hash}  {}\n", release.asset(platform))).unwrap();
        (bytes, hash)
    }

    fn local_platform() -> Platform {
        let out = std::process::Command::new("uname").arg("-sm").output().unwrap();
        Platform::from_uname(&String::from_utf8_lossy(&out.stdout)).unwrap()
    }

    fn has(tool: &str) -> bool {
        std::process::Command::new("sh").args(["-c", &format!("command -v {tool}")]).output().is_ok_and(|o| o.status.success())
    }

    fn run_agent(home: &std::path::Path, release: &Release) -> String {
        let path = home.join(release.dir()).join(AGENT_FILE);
        String::from_utf8(std::process::Command::new(path).output().unwrap().stdout).unwrap().trim().to_string()
    }

    #[tokio::test]
    async fn the_remote_downloads_checks_and_installs_the_agent_once() {
        if !has("curl") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let mut release = Release { version: "9.8.7".into(), base_url: String::new() };
        release.base_url = format!("file://{}", dir.path().join("releases").display());
        let platform = local_platform();
        let (_, hash) = publish(dir.path(), &release, platform, "first");
        let files = Files(HashMap::from([(release.checksum_url(platform), format!("{hash}  x").into_bytes())]), Mutex::default());
        let steps = Mutex::new(Vec::new());
        let shell = LocalShell { home: home.clone(), like_ssh: false };
        let agent = ensure_agent(&shell, &files, &release, &|s| steps.lock().unwrap().push(s)).await.unwrap();
        assert_eq!(agent, "~/.dotcraft/remote/9.8.7/dotcraft-remote-agent");
        assert_eq!(*steps.lock().unwrap(), [Step::Checking, Step::DownloadingOnRemote]);
        assert_eq!(run_agent(&home, &release), "first");
        // Installed: the next connection only checks.
        steps.lock().unwrap().clear();
        ensure_agent(&shell, &files, &release, &|s| steps.lock().unwrap().push(s)).await.unwrap();
        assert_eq!(*steps.lock().unwrap(), [Step::Checking]);
        // No temporary folders left behind.
        let left: Vec<_> = std::fs::read_dir(home.join(".dotcraft/remote")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["9.8.7"]);
    }

    #[tokio::test]
    async fn a_tampered_download_is_refused() {
        if !has("curl") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let release = Release { version: "9.8.7".into(), base_url: format!("file://{}", dir.path().join("releases").display()) };
        let platform = local_platform();
        publish(dir.path(), &release, platform, "evil");
        let files = Files(HashMap::from([(release.checksum_url(platform), "a".repeat(64).into_bytes())]), Mutex::default());
        let error = ensure_agent(&LocalShell { home: home.clone(), like_ssh: true }, &files, &release, &|_| {}).await.unwrap_err();
        assert!(error.to_string().contains("SHA-256"), "{error:#}");
        assert!(!home.join(release.dir()).join(AGENT_FILE).exists());
    }

    /// The remote cannot reach GitHub: DotCraft downloads the archive and copies it over
    /// (with the scripts quoted for a login shell, like over SSH).
    #[tokio::test]
    async fn an_offline_remote_gets_the_agent_copied_over() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        // Nothing is published at this address: every remote download fails.
        let release = Release { version: "9.8.7".into(), base_url: format!("file://{}", dir.path().join("nowhere").display()) };
        let platform = local_platform();
        let (bytes, hash) = publish(dir.path(), &release, platform, "copied");
        let files = Files(
            HashMap::from([(release.checksum_url(platform), format!("{hash}  x").into_bytes()), (release.url(platform), bytes)]),
            Mutex::default(),
        );
        let steps = Mutex::new(Vec::new());
        ensure_agent(&LocalShell { home: home.clone(), like_ssh: true }, &files, &release, &|s| steps.lock().unwrap().push(s)).await.unwrap();
        assert_eq!(steps.lock().unwrap().last(), Some(&Step::Uploading));
        assert_eq!(run_agent(&home, &release), "copied");
        // And with nothing anywhere, the error names what was tried.
        let empty = Files(HashMap::new(), Mutex::default());
        let other = Release { version: "9.8.8".into(), ..release };
        let error = ensure_agent(&LocalShell { home, like_ssh: true }, &empty, &other, &|_| {}).await.unwrap_err();
        assert!(format!("{error:#}").contains("checksum"), "{error:#}");
    }
}
