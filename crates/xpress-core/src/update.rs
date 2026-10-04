//! Check GitHub Releases for a newer version.
//!
//! This only *checks* and downloads; replacing the running program lives in
//! the CLI's `update` command and the GUI. The GUI uses [`check`] to show an
//! update banner.
//!
//! The GitHub API allows 60 anonymous requests an hour per IP address. A
//! `GITHUB_TOKEN`/`GH_TOKEN` in the environment is used when set; without one,
//! or when the API refuses, the latest tag is read from the redirect of the
//! public `releases/latest` page and asset URLs follow the release naming.

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
    /// Direct download URL of the `.app` zip for the current platform, if found.
    pub download_url: Option<String>,
    /// Direct download URL of the CLI tarball for the current platform.
    pub cli_download_url: Option<String>,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
}

/// The release-asset target triple for the current macOS architecture.
pub fn current_target() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some("aarch64-apple-darwin")
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Some("x86_64-apple-darwin")
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The target triple of the CLI release tarball for this build.
pub fn cli_target() -> Option<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some("x86_64-unknown-linux-gnu")
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        current_target()
    }
}

/// The CLI tarball's name in a release (see .github/workflows/release.yml).
pub fn cli_asset_name(tag: &str, target: &str) -> String {
    format!("xpress-{tag}-{target}.tar.gz")
}

/// The desktop app zip's name in a release.
pub fn app_asset_name(tag: &str, target: &str) -> String {
    format!("xpress-{tag}-macos-{target}-app.zip")
}

/// Query the latest published release and compare it to `current` (e.g. the
/// crate version). Network + parse errors are returned as a message.
pub fn check(current: &str) -> Result<UpdateInfo, String> {
    let rel = match latest_from_api() {
        Ok(rel) => rel,
        // Rate limited, blocked or down: the public release page still works.
        Err(api) => latest_from_redirect().map_err(|e| format!("{api} (fallback: {e})"))?,
    };
    Ok(info_from(rel, current))
}

fn latest_from_api() -> Result<GhRelease, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut req = ureq::get(&url)
        .set("User-Agent", "xpress-updater")
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(10));
    if let Some(token) = ["GITHUB_TOKEN", "GH_TOKEN"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|t| !t.trim().is_empty())
    {
        req = req.set("Authorization", &format!("Bearer {}", token.trim()));
    }
    let body = req
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(403 | 429, _) => {
                "GitHub's API rate limit was reached (set GITHUB_TOKEN to raise it)".to_string()
            }
            e => e.to_string(),
        })?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// `https://github.com/<repo>/releases/latest` redirects to `.../tag/<tag>`.
fn latest_from_redirect() -> Result<GhRelease, String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let resp = agent
        .get(&format!("https://github.com/{REPO}/releases/latest"))
        .set("User-Agent", "xpress-updater")
        .call()
        .map_err(|e| e.to_string())?;
    let location = resp
        .header("location")
        .ok_or("no redirect to the latest release")?;
    let tag = tag_from_location(location).ok_or("unexpected release redirect")?;
    Ok(GhRelease {
        html_url: location.to_string(),
        tag_name: tag.to_string(),
        body: String::new(),
        assets: Vec::new(),
    })
}

fn tag_from_location(location: &str) -> Option<&str> {
    let tag = location
        .trim_end_matches('/')
        .rsplit_once("/releases/tag/")?
        .1;
    (!tag.is_empty() && !tag.contains('/')).then_some(tag)
}

fn info_from(rel: GhRelease, current: &str) -> UpdateInfo {
    let latest = rel.tag_name.trim_start_matches('v').trim().to_string();
    let newer = is_newer(&latest, current.trim_start_matches('v'));

    // The listed asset, or (without a listing) its conventional URL.
    let asset = |name: String| {
        if rel.assets.is_empty() {
            Some(format!(
                "https://github.com/{REPO}/releases/download/{}/{name}",
                rel.tag_name
            ))
        } else {
            rel.assets
                .iter()
                .find(|a| a.name == name)
                .map(|a| a.browser_download_url.clone())
        }
    };
    let download_url = current_target().and_then(|t| asset(app_asset_name(&rel.tag_name, t)));
    let cli_download_url = cli_target().and_then(|t| asset(cli_asset_name(&rel.tag_name, t)));

    UpdateInfo {
        current: current.trim_start_matches('v').to_string(),
        latest,
        newer,
        url: rel.html_url,
        notes: rel.body,
        download_url,
        cli_download_url,
    }
}

/// Download a URL to bytes (follows redirects). For release assets.
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "xpress-updater")
        .timeout(std::time::Duration::from_secs(300))
        .call()
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Download a release asset and check it against the `<asset>.sha256` file
/// published next to it.
pub fn download_verified(url: &str) -> Result<Vec<u8>, String> {
    let sums = download(&format!("{url}.sha256"))
        .map_err(|e| format!("could not fetch the checksum: {e}"))?;
    let bytes = download(url)?;
    verify_sha256(&bytes, &String::from_utf8_lossy(&sums))?;
    Ok(bytes)
}

/// Compare `bytes` with the digest in a `shasum -a 256` line (`<hex>  <name>`).
fn verify_sha256(bytes: &[u8], sums: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let expected = sums
        .split_whitespace()
        .next()
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or("malformed checksum file")?
        .to_ascii_lowercase();
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual == expected {
        Ok(())
    } else {
        Err("checksum mismatch: the download is corrupt or was altered".into())
    }
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

    /// The release asset names (see .github/workflows/release.yml) must keep
    /// the CLI tarball the *first* asset, alphabetically (GitHub's order),
    /// containing the target: CLIs up to 0.5.0 pick the first match, and the
    /// desktop app's updater needs `<target>…-app.zip`.
    #[test]
    fn release_asset_names_keep_the_cli_tarball_first() {
        let tag = "v9.9.9";
        let target = "aarch64-apple-darwin";
        let mut names = [
            format!("xpress-{tag}-{target}.tar.gz"),
            format!("xpress-{tag}-{target}.tar.gz.sha256"),
            format!("xpress-{tag}-macos-{target}-app.zip"),
            format!("xpress-{tag}-macos-{target}-app.zip.sha256"),
            format!("xpress-{tag}-macos-{target}.dmg"),
            format!("xpress-{tag}-macos-{target}.dmg.sha256"),
        ];
        names.sort();
        let first = names.iter().find(|n| n.contains(target)).unwrap();
        assert_eq!(first, &format!("xpress-{tag}-{target}.tar.gz"));
        assert!(names
            .iter()
            .any(|n| n.contains(target) && n.ends_with("-app.zip")));
    }

    #[test]
    fn latest_tag_from_the_release_redirect() {
        let base = "https://github.com/kwhorne/xpress/releases";
        assert_eq!(
            tag_from_location(&format!("{base}/tag/v0.5.2")),
            Some("v0.5.2")
        );
        assert_eq!(
            tag_from_location(&format!("{base}/tag/v0.5.2/")),
            Some("v0.5.2")
        );
        assert_eq!(tag_from_location(base), None);
        assert_eq!(tag_from_location(&format!("{base}/tag/")), None);
    }

    #[test]
    fn asset_urls_without_an_api_listing() {
        let rel = GhRelease {
            tag_name: "v9.9.9".into(),
            html_url: String::new(),
            body: String::new(),
            assets: Vec::new(),
        };
        let info = info_from(rel, "0.5.2");
        assert!(info.newer);
        assert_eq!(info.latest, "9.9.9");
        if let Some(target) = cli_target() {
            assert_eq!(
                info.cli_download_url.unwrap(),
                format!(
                    "https://github.com/kwhorne/xpress/releases/download/v9.9.9/xpress-v9.9.9-{target}.tar.gz"
                )
            );
        }
        if let Some(target) = current_target() {
            assert!(info
                .download_url
                .unwrap()
                .ends_with(&format!("/v9.9.9/xpress-v9.9.9-macos-{target}-app.zip")));
        }
    }

    #[test]
    fn asset_urls_from_the_api_listing() {
        let Some(target) = cli_target() else { return };
        let asset = |name: String| GhAsset {
            browser_download_url: format!("https://example.test/{name}"),
            name,
        };
        let rel = GhRelease {
            tag_name: "v1.0.0".into(),
            html_url: String::new(),
            body: String::new(),
            assets: vec![
                asset(format!("xpress-v1.0.0-{target}.tar.gz.sha256")),
                asset(format!("xpress-v1.0.0-{target}.tar.gz")),
            ],
        };
        let info = info_from(rel, "0.5.2");
        assert_eq!(
            info.cli_download_url.unwrap(),
            format!("https://example.test/xpress-v1.0.0-{target}.tar.gz")
        );
    }

    #[test]
    fn checksum_verification() {
        // sha256("hello")
        let sums =
            "2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824  hello.tar.gz\n";
        assert!(verify_sha256(b"hello", sums).is_ok());
        assert!(verify_sha256(b"hellO", sums)
            .unwrap_err()
            .contains("mismatch"));
        assert!(verify_sha256(b"hello", "").is_err());
        assert!(verify_sha256(b"hello", "not-a-digest  x").is_err());
    }

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
