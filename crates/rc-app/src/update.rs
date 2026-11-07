//! "A new version is out": the latest release on GitHub, read in the background. Nothing is
//! downloaded or installed (the builds aren't signed), the notice links to the release page.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// `0.11.0`, without the tag's `v`.
    pub version: String,
    /// The release's page in this repository.
    pub url: String,
}

/// Reads the latest release after `delay`, on a thread of its own; the result arrives through the
/// receiver and repaints the window.
pub fn check(ctx: egui::Context, delay: Duration) -> Receiver<Result<Release, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
            .and_then(|rt| rt.block_on(latest()).map_err(|e| format!("{e:#}")));
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

async fn latest() -> anyhow::Result<Release> {
    #[derive(serde::Deserialize)]
    struct Latest {
        tag_name: String,
    }
    let api = format!("{}/releases/latest", REPOSITORY.replace("https://github.com/", "https://api.github.com/repos/"));
    let latest: Latest = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("DotCraft/", env!("CARGO_PKG_VERSION")))
        .build()?
        .get(api)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    release(&latest.tag_name).ok_or_else(|| anyhow::anyhow!("unexpected release tag {:?}", latest.tag_name))
}

/// The release of a `v0.11.0` tag. The page is built here, not taken from the response, so the
/// notice only ever links to this repository.
fn release(tag: &str) -> Option<Release> {
    let version = tag.trim().trim_start_matches('v');
    parse(version)?;
    Some(Release { version: version.to_string(), url: format!("{REPOSITORY}/releases/tag/{}", tag.trim()) })
}

/// `major.minor.patch`; a pre-release suffix (`-beta.1`) is not a newer version.
fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let v = (next()?, next()?, next()?);
    parts.next().is_none().then_some(v)
}

/// Whether `latest` is newer than the running version.
pub fn is_newer(latest: &str) -> bool {
    newer(latest, env!("CARGO_PKG_VERSION"))
}

fn newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("0.11.0", "0.10.0"));
        assert!(newer("0.10.1", "0.10.0"));
        assert!(newer("1.0.0", "0.99.9"));
        assert!(!newer("0.10.0", "0.10.0"));
        assert!(!newer("0.9.9", "0.10.0"));
        assert!(!newer("0.11.0-beta.1", "0.10.0"));
        assert!(!newer("garbage", "0.10.0"));
    }

    #[test]
    fn links_only_to_this_repository() {
        let r = release("v0.11.0").unwrap();
        assert_eq!(r.version, "0.11.0");
        assert_eq!(r.url, format!("{REPOSITORY}/releases/tag/v0.11.0"));
        assert!(release("v0.11.0/../../evil").is_none());
        assert!(release("latest").is_none());
    }
}
