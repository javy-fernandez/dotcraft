//! netcoredbg (Samsung, MIT): finding it, downloading the pinned release into the tools
//! folder and the command line that runs it as a DAP adapter over stdio.
//!
//! Microsoft's vsdbg is licensed for Microsoft products only, so DotCraft uses netcoredbg.

use rc_core::inbox::Outbox;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Release downloaded when none is installed (see [`release`] for Linux).
pub const VERSION: &str = "3.2.0-1092";

/// Official Linux builds, newest first, with the oldest glibc each one is downloaded for.
/// Samsung builds them on recent distributions, so each needs that host's glibc and
/// libstdc++ (amd64 and arm64 alike): 3.2.0 needs glibc 2.38 and GLIBCXX_3.4.32, 3.1.3 glibc
/// 2.34 and GLIBCXX_3.4.30, 3.0.0 glibc 2.17 and GLIBCXX_3.4.26. 3.1.3 starts at glibc 2.35
/// because the distributions on 2.34 (RHEL 9, Fedora 35) ship GCC 11's libstdc++, which
/// stops at GLIBCXX_3.4.29.
#[cfg(any(target_os = "linux", test))]
const LINUX_RELEASES: [(&str, (u32, u32)); 3] = [(VERSION, (2, 38)), ("3.1.3-1062", (2, 35)), ("3.0.0-1018", (2, 17))];

/// The release downloaded on this machine: [`VERSION`], or on a Linux system whose glibc is
/// too old for it, the newest official build that runs there (instead of an adapter that
/// fails to load).
pub fn release() -> &'static str {
    #[cfg(target_os = "linux")]
    if let Some(glibc) = glibc_version() {
        return release_for_glibc(glibc);
    }
    VERSION
}

#[cfg(any(target_os = "linux", test))]
fn release_for_glibc(glibc: (u32, u32)) -> &'static str {
    LINUX_RELEASES.iter().find(|(_, oldest)| glibc >= *oldest).unwrap_or(&LINUX_RELEASES[LINUX_RELEASES.len() - 1]).0
}

/// Whether `version` is one of our Linux downloads that needs a newer glibc than this
/// system's (left in the tools folder by an earlier DotCraft, it cannot start).
fn needs_newer_glibc(version: &str) -> bool {
    #[cfg(target_os = "linux")]
    if let (Some((_, oldest)), Some(glibc)) = (LINUX_RELEASES.iter().find(|(v, _)| *v == version), glibc_version()) {
        return glibc < *oldest;
    }
    let _ = version;
    false
}

/// The running glibc (`gnu_get_libc_version`).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn glibc_version() -> Option<(u32, u32)> {
    // SAFETY: glibc returns a static NUL-terminated string.
    let v = unsafe { std::ffi::CStr::from_ptr(libc::gnu_get_libc_version()) };
    parse_glibc_version(v.to_str().ok()?)
}

/// Another C library (musl): the newest build, as elsewhere.
#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
fn glibc_version() -> Option<(u32, u32)> {
    None
}

#[cfg(any(target_os = "linux", test))]
fn parse_glibc_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor: String = parts.next()?.chars().take_while(char::is_ascii_digit).collect();
    Some((major, minor.parse().ok()?))
}

/// What a Linux loader error on the adapter's stderr says is missing, worded for the user:
/// "libc.so.6: version `GLIBC_2.38' not found (required by netcoredbg)" gives "glibc 2.38";
/// a libstdc++ symbol version (GLIBCXX_3.4.32, CXXABI_1.3.15) gives "libstdc++ (GLIBCXX_3.4.32)".
#[cfg(any(target_os = "linux", test))]
pub fn missing_system_library(stderr_line: &str) -> Option<String> {
    let at = stderr_line.find("version ")? + "version ".len();
    let rest = stderr_line[at..].strip_prefix(['`', '\'', '‘'])?;
    let end = rest.find(['\'', '’', '`'])?;
    let (name, after) = (&rest[..end], &rest[end + 1..]);
    if !after.trim_start().starts_with("not found") {
        return None;
    }
    if let Some(version) = name.strip_prefix("GLIBC_") {
        return Some(format!("glibc {version}"));
    }
    (name.starts_with("GLIBCXX_") || name.starts_with("CXXABI_")).then(|| format!("libstdc++ ({name})"))
}
const RELEASES: &str = "https://github.com/Samsung/netcoredbg/releases/download";

/// How to start the adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterCommand {
    pub program: PathBuf,
    /// Arguments before ours (a script for an interpreter, for instance).
    pub prefix: Vec<String>,
    /// Where it was found (for the status bar / settings page).
    pub origin: Origin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The path configured in the settings.
    Settings,
    /// Our download in the tools folder.
    Downloaded,
    /// Found on `PATH`.
    Path,
}

impl AdapterCommand {
    /// Arguments that make netcoredbg speak DAP on stdin/stdout.
    pub fn args(&self) -> Vec<String> {
        let mut a = self.prefix.clone();
        a.push("--interpreter=vscode".to_string());
        a
    }

    pub fn new(program: impl Into<PathBuf>, origin: Origin) -> Self {
        AdapterCommand { program: program.into(), prefix: Vec::new(), origin }
    }
}

pub fn exe_name() -> &'static str {
    if cfg!(windows) { "netcoredbg.exe" } else { "netcoredbg" }
}

/// Release asset for this machine; `None` where netcoredbg publishes no build (Intel Macs,
/// Windows on Arm).
pub fn asset_name() -> Option<&'static str> {
    asset_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn asset_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("netcoredbg-osx-arm64.zip"),
        ("windows", "x86_64") => Some("netcoredbg-win64.zip"),
        ("linux", "x86_64") => Some("netcoredbg-linux-amd64.tar.gz"),
        ("linux", "aarch64") => Some("netcoredbg-linux-arm64.tar.gz"),
        _ => None,
    }
}

pub fn download_url() -> Option<String> {
    asset_name().map(|a| format!("{RELEASES}/{}/{a}", release()))
}

/// Folder of the downloaded adapter (`<tools>/netcoredbg/<version>`).
pub fn install_dir() -> PathBuf {
    rc_core::paths::tools_dir().join("netcoredbg").join(release())
}

/// Finds netcoredbg: the configured path (the executable or its folder), our download (the
/// [`release`] first, then any other version in the tools folder that can run here), then
/// `PATH`.
pub fn locate(configured: Option<&Path>) -> Option<AdapterCommand> {
    if let Some(p) = configured {
        let p = if p.is_dir() { p.join(exe_name()) } else { p.to_path_buf() };
        if p.is_file() {
            return Some(AdapterCommand::new(p, Origin::Settings));
        }
    }
    let ours = install_dir().join(exe_name());
    if ours.is_file() {
        return Some(AdapterCommand::new(ours, Origin::Downloaded));
    }
    let root = rc_core::paths::tools_dir().join("netcoredbg");
    let mut others: Vec<PathBuf> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| !p.extension().is_some_and(|e| e == "partial"))
        .filter(|p| !p.file_name().is_some_and(|n| needs_newer_glibc(&n.to_string_lossy())))
        .map(|p| p.join(exe_name()))
        .chain(std::iter::once(root.join(exe_name())))
        .filter(|p| p.is_file())
        .collect();
    others.sort();
    if let Some(p) = others.pop() {
        return Some(AdapterCommand::new(p, Origin::Downloaded));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(exe_name())).find(|p| p.is_file()).map(|program| AdapterCommand::new(program, Origin::Path))
}

/// Download progress for the status bar.
#[derive(Clone, Debug, PartialEq)]
pub enum DownloadEvent {
    Progress { done: u64, total: Option<u64> },
    /// Download finished, unpacking.
    Unpacking,
    /// Path of the executable.
    Done(PathBuf),
    Failed(String),
}

/// A download running on the shared runtime; dropping it aborts the download.
pub struct DownloadHandle(Option<tokio::task::JoinHandle<()>>);

impl DownloadHandle {
    pub fn is_finished(&self) -> bool {
        self.0.as_ref().is_none_or(|t| t.is_finished())
    }

    pub fn cancel(mut self) {
        if let Some(t) = self.0.take() {
            t.abort();
        }
    }
}

impl Drop for DownloadHandle {
    fn drop(&mut self) {
        if let Some(t) = self.0.take() {
            t.abort();
        }
    }
}

/// Starts [`download`] on the shared runtime.
pub fn start_download(events: Outbox<DownloadEvent>) -> DownloadHandle {
    DownloadHandle(Some(rc_core::Runtime::spawn(download(events))))
}

/// Downloads and unpacks the pinned release into [`install_dir`] (~3.5 MB).
pub async fn download(events: Outbox<DownloadEvent>) {
    let ev = match download_inner(&events).await {
        Ok(p) => DownloadEvent::Done(p),
        Err(e) => DownloadEvent::Failed(format!("{e:#}")),
    };
    events.send(ev);
}

async fn download_inner(events: &Outbox<DownloadEvent>) -> anyhow::Result<PathBuf> {
    let asset = asset_name().ok_or_else(|| anyhow::anyhow!("netcoredbg has no build for {}-{}", std::env::consts::OS, std::env::consts::ARCH))?;
    let url = format!("{RELEASES}/{}/{asset}", release());
    let client = reqwest::Client::builder().build()?;
    let mut resp = client.get(&url).send().await?.error_for_status()?;
    let total = resp.content_length();
    let mut bytes: Vec<u8> = Vec::with_capacity(total.unwrap_or(4 << 20) as usize);
    let mut last = 0u64;
    events.send(DownloadEvent::Progress { done: 0, total });
    while let Some(chunk) = resp.chunk().await? {
        bytes.extend_from_slice(&chunk);
        let done = bytes.len() as u64;
        if done - last >= 256 << 10 {
            last = done;
            events.send(DownloadEvent::Progress { done, total });
        }
    }
    events.send(DownloadEvent::Progress { done: bytes.len() as u64, total });
    events.send(DownloadEvent::Unpacking);
    let dest = install_dir();
    tokio::task::spawn_blocking(move || install(&bytes, asset, &dest)).await?
}

/// Unpacks an archive of the release into `dest` (replacing it) and returns the executable.
pub fn install(archive: &[u8], asset: &str, dest: &Path) -> anyhow::Result<PathBuf> {
    let tmp = dest.with_extension("partial");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    if asset.ends_with(".zip") {
        unpack_zip(archive, &tmp)?;
    } else {
        unpack_tar_gz(archive, &tmp)?;
    }
    let exe = tmp.join(exe_name());
    anyhow::ensure!(exe.is_file(), "the archive has no {}", exe_name());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Zips made on macOS carry no reliable Unix modes: the executable and the native
        // libraries must be made executable by hand.
        for entry in std::fs::read_dir(&tmp)?.flatten() {
            let p = entry.path();
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if name == exe_name() || name.ends_with(".dylib") || name.ends_with(".so") {
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    let _ = std::fs::remove_dir_all(dest);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(dest.join(exe_name()))
}

/// Path inside the release folder: drops the top `netcoredbg/` folder and macOS metadata.
fn relative_entry(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    let name = name.trim_start_matches("./");
    if name.starts_with("__MACOSX/") || name.split('/').any(|c| c == ".." || c.starts_with("._")) {
        return None;
    }
    let rel = name.strip_prefix("netcoredbg/").unwrap_or(name);
    (!rel.is_empty() && !rel.ends_with('/')).then(|| PathBuf::from(rel))
}

fn unpack_zip(archive: &[u8], to: &Path) -> anyhow::Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let Some(name) = f.enclosed_name() else { continue };
        let Some(rel) = relative_entry(&name.to_string_lossy()) else { continue };
        write_entry(&mut f, &to.join(rel))?;
    }
    Ok(())
}

fn unpack_tar_gz(archive: &[u8], to: &Path) -> anyhow::Result<()> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.to_string_lossy().into_owned();
        let Some(rel) = relative_entry(&path) else { continue };
        write_entry(&mut entry, &to.join(rel))?;
    }
    Ok(())
}

fn write_entry(r: &mut impl Read, out: &Path) -> anyhow::Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut w = std::fs::File::create(out)?;
    std::io::copy(r, &mut w)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn picks_the_release_asset_per_platform() {
        assert_eq!(asset_for("macos", "aarch64"), Some("netcoredbg-osx-arm64.zip"));
        assert_eq!(asset_for("windows", "x86_64"), Some("netcoredbg-win64.zip"));
        assert_eq!(asset_for("linux", "aarch64"), Some("netcoredbg-linux-arm64.tar.gz"));
        assert_eq!(asset_for("macos", "x86_64"), None);
        assert!(VERSION.contains('-'));
    }

    #[test]
    fn picks_the_newest_linux_build_the_system_glibc_runs() {
        assert_eq!(release_for_glibc((2, 39)), VERSION);
        assert_eq!(release_for_glibc((2, 38)), VERSION);
        // Ubuntu 22.04, Debian 12.
        assert_eq!(release_for_glibc((2, 35)), "3.1.3-1062");
        assert_eq!(release_for_glibc((2, 36)), "3.1.3-1062");
        // RHEL 9 (GCC 11's libstdc++), Ubuntu 20.04, Debian 11, and anything older.
        assert_eq!(release_for_glibc((2, 34)), "3.0.0-1018");
        assert_eq!(release_for_glibc((2, 31)), "3.0.0-1018");
        assert_eq!(release_for_glibc((2, 12)), "3.0.0-1018");
        assert!(LINUX_RELEASES.windows(2).all(|w| w[0].1 > w[1].1));
        if !cfg!(target_os = "linux") {
            assert_eq!(release(), VERSION);
            assert!(!needs_newer_glibc(VERSION));
        }
    }

    #[test]
    fn parses_the_glibc_version() {
        assert_eq!(parse_glibc_version("2.35"), Some((2, 35)));
        assert_eq!(parse_glibc_version("2.38.9000"), Some((2, 38)));
        assert_eq!(parse_glibc_version("2.17-RHEL"), Some((2, 17)));
        assert_eq!(parse_glibc_version("unknown"), None);
    }

    #[test]
    fn reads_what_the_linux_loader_misses() {
        let glibc = "/home/u/.local/share/DotCraft/tools/netcoredbg/3.2.0-1092/netcoredbg: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.38' not found (required by /home/u/.local/share/DotCraft/tools/netcoredbg/3.2.0-1092/netcoredbg)";
        assert_eq!(missing_system_library(glibc).as_deref(), Some("glibc 2.38"));
        let cxx = "./netcoredbg: /lib64/libstdc++.so.6: version 'GLIBCXX_3.4.32' not found (required by ./netcoredbg)";
        assert_eq!(missing_system_library(cxx).as_deref(), Some("libstdc++ (GLIBCXX_3.4.32)"));
        let abi = "netcoredbg: /usr/lib/libstdc++.so.6: version `CXXABI_1.3.15' not found (required by netcoredbg)";
        assert_eq!(missing_system_library(abi).as_deref(), Some("libstdc++ (CXXABI_1.3.15)"));
        for other in ["info: version `GLIBC_2.38' found", "netcoredbg: version 3.2.0", "error: libdbgshim.so not found", ""] {
            assert_eq!(missing_system_library(other), None, "{other}");
        }
    }

    #[test]
    fn unpacks_a_zip_skipping_macos_metadata() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o: zip::write::SimpleFileOptions = Default::default();
            z.add_directory("netcoredbg/", o).unwrap();
            z.start_file(format!("netcoredbg/{}", exe_name()), o).unwrap();
            z.write_all(b"#!/bin/sh\necho hi\n").unwrap();
            z.start_file("netcoredbg/libdbgshim.dylib", o).unwrap();
            z.write_all(b"lib").unwrap();
            z.start_file("netcoredbg/ManagedPart.dll", o).unwrap();
            z.write_all(b"dll").unwrap();
            z.start_file("__MACOSX/netcoredbg/._ManagedPart.dll", o).unwrap();
            z.write_all(b"junk").unwrap();
            z.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("netcoredbg").join(VERSION);
        let exe = install(buf.get_ref(), "netcoredbg-osx-arm64.zip", &dest).unwrap();
        assert_eq!(exe, dest.join(exe_name()));
        let mut names: Vec<String> = std::fs::read_dir(&dest).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, vec!["ManagedPart.dll".to_string(), "libdbgshim.dylib".into(), exe_name().into()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755);
            assert_eq!(std::fs::metadata(dest.join("ManagedPart.dll")).unwrap().permissions().mode() & 0o111, 0);
        }
        assert!(!dest.with_extension("partial").exists());
    }

    #[test]
    fn unpacks_a_tar_gz() {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut t = tar::Builder::new(&mut gz);
            for (name, data) in [(format!("netcoredbg/{}", exe_name()), &b"bin"[..]), ("netcoredbg/libdbgshim.so".into(), b"so")] {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                t.append_data(&mut h, name, data).unwrap();
            }
            t.finish().unwrap();
        }
        let bytes = gz.finish().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let exe = install(&bytes, "netcoredbg-linux-amd64.tar.gz", &dir.path().join("v")).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"bin");
        assert!(dir.path().join("v/libdbgshim.so").is_file());
    }

    #[test]
    fn locates_a_configured_folder_or_file() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(exe_name());
        std::fs::write(&exe, b"x").unwrap();
        assert_eq!(locate(Some(dir.path())).unwrap(), AdapterCommand::new(exe.clone(), Origin::Settings));
        assert_eq!(locate(Some(&exe)).unwrap().program, exe);
    }
}
