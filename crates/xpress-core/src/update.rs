//! Check GitHub Releases for a newer version.
//!
//! This only *checks* (a cheap API call); performing the self-replace lives in
//! the CLI's `update` command. The GUI uses [`check`] to show an update banner.

use serde::Deserialize;

/// `owner/repo` the releases are published under.
pub const REPO: &str = "kwhorne/xpress";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    /// Whether `latest` is strictly newer than `current`.
    pub newer: bool,
    /// The release's web page.
    pub url: String,
    /// Release notes (may be empty).
    pub notes: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
}

/// Query the latest published release and compare it to `current` (e.g. the
/// crate version). Network + parse errors are returned as a message.
pub fn check(current: &str) -> Result<UpdateInfo, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = ureq::get(&url)
        .set("User-Agent", "xpress-updater")
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let rel: GhRelease = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let latest = rel.tag_name.trim_start_matches('v').trim().to_string();
    let newer = is_newer(&latest, current.trim_start_matches('v'));
    Ok(UpdateInfo {
        current: current.trim_start_matches('v').to_string(),
        latest,
        newer,
        url: rel.html_url,
        notes: rel.body,
    })
}

fn parse(v: &str) -> (u64, u64, u64) {
    // Ignore any pre-release/build suffix (e.g. "1.2.3-rc1").
    let core = v.split(['-', '+']).next().unwrap_or(v);
    let mut it = core
        .split('.')
        .map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

/// Whether `latest` is a strictly newer semver than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    parse(latest) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.4.1", "0.4.0"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
        assert!(is_newer("v0.5.0", "v0.4.0")); // tolerant of leading text stripped by caller
        assert!(!is_newer("0.4.0-rc1", "0.4.0")); // pre-release ignored -> equal core
    }
}
