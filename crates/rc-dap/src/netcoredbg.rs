//! netcoredbg (Samsung, MIT): finding it, downloading the pinned release into the tools
//! folder and the command line that runs it as a DAP adapter over stdio.
//!
//! Microsoft's vsdbg is licensed for Microsoft products only, so DotCraft uses netcoredbg.

use rc_core::inbox::Outbox;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Release downloaded when none is installed.
pub const VERSION: &str = "3.2.0-1092";
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
    asset_name().map(|a| format!("{RELEASES}/{VERSION}/{a}"))
}

/// Folder of the downloaded adapter (`<tools>/netcoredbg/<version>`).
pub fn install_dir() -> PathBuf {
    rc_core::paths::tools_dir().join("netcoredbg").join(VERSION)
}

/// Finds netcoredbg: the configured path (the executable or its folder), our download (the
/// pinned version first, then any other version in the tools folder), then `PATH`.
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
    let url = format!("{RELEASES}/{VERSION}/{asset}");
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
