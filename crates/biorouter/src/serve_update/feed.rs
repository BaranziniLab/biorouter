//! Where releases come from, and what a release says about its files.
//!
//! The feed is GitHub's release API for `BaranziniLab/biorouter`, read for two
//! documents: the latest release (`{base}/latest`) and one named release
//! (`{base}/tags/v{version}`), which is how the copy of the running version that
//! a package rollback reinstalls is found. Every asset in those documents
//! carries the `digest` GitHub publishes (`sha256:<hex>`), and nothing is
//! installed without matching it ([`super::download`]).
//!
//! [`FEED_ENV`] replaces the base with another feed in the same shape: a mirror,
//! or the static files a live test serves. It must be `https`, or `http` on a
//! loopback address, and so must every download URL it names, so the override
//! cannot quietly turn the integrity of a download into a plain-text request
//! across a network.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

/// The official release feed.
pub const DEFAULT_FEED: &str = "https://api.github.com/repos/BaranziniLab/biorouter/releases";

/// Replaces [`DEFAULT_FEED`]: a mirror, or a live test's local feed. Read by the
/// daemon once at start, from the environment `serve` was started in.
pub const FEED_ENV: &str = "BIOROUTER_SERVE_UPDATE_FEED_URL";

/// How long one feed request may take.
const FEED_TIMEOUT: Duration = Duration::from_secs(30);

/// The release feed this process reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    base: Url,
    /// True when [`FEED_ENV`] chose it, which the status reports so a test feed
    /// is never mistaken for the official one.
    pub overridden: bool,
}

impl Feed {
    /// The official feed.
    pub fn official() -> Self {
        Self {
            base: Url::parse(DEFAULT_FEED).expect("the official feed is a valid URL"),
            overridden: false,
        }
    }

    /// The feed [`FEED_ENV`] names, else the official one. A value that is not
    /// an allowed URL is an error rather than a silent fallback: an operator who
    /// set a mirror and reads updates from GitHub instead has been misled.
    pub fn from_env() -> Result<Self> {
        match std::env::var(FEED_ENV) {
            Ok(value) if !value.trim().is_empty() => Self::parse(value.trim()),
            _ => Ok(Self::official()),
        }
    }

    /// A feed at `base`, which must pass [`allowed_url`].
    pub fn parse(base: &str) -> Result<Self> {
        let url = Url::parse(base).with_context(|| format!("{FEED_ENV} is not a URL: {base}"))?;
        if !allowed_url(&url) {
            bail!("{FEED_ENV} must be an https address, or http on this machine (127.0.0.1, ::1 or localhost): {base}");
        }
        Ok(Self {
            base: url,
            overridden: true,
        })
    }

    fn document(&self, tail: &str) -> Result<Url> {
        let base = self.base.as_str().trim_end_matches('/');
        Url::parse(&format!("{base}/{tail}")).map_err(|e| anyhow!("bad feed address: {e}"))
    }

    /// The newest release.
    pub async fn latest(&self, client: &reqwest::Client) -> Result<Release> {
        self.fetch(client, self.document("latest")?).await
    }

    /// The release for `version`, or `None` when the feed has none.
    pub async fn release(
        &self,
        client: &reqwest::Client,
        version: &str,
    ) -> Result<Option<Release>> {
        let url = self.document(&format!("tags/v{}", super::normalize_version(version)))?;
        let response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .timeout(FEED_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("could not reach {}", url.host_str().unwrap_or("the feed")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = response.error_for_status()?.text().await?;
        Ok(Some(Release::from_json(&body)?))
    }

    async fn fetch(&self, client: &reqwest::Client, url: Url) -> Result<Release> {
        let response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .timeout(FEED_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("could not reach {}", url.host_str().unwrap_or("the feed")))?;
        let status = response.status();
        if !status.is_success() {
            bail!(
                "the release feed answered {status} from {}",
                url.host_str().unwrap_or("the feed")
            );
        }
        Release::from_json(&response.text().await?)
    }
}

/// `https` anywhere, or `http` to this machine. The rule for the feed and for
/// every download it names.
pub fn allowed_url(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => {
            matches!(
                url.host(),
                Some(url::Host::Ipv4(ip)) if ip.is_loopback()
            ) || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
                || url.host_str() == Some("localhost")
        }
        _ => false,
    }
}

/// The HTTP client every request of this module uses: redirects followed only to
/// addresses [`allowed_url`] admits (GitHub's downloads redirect to its object
/// store), and a product user agent with no machine or person in it.
pub fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if allowed_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("a download redirected to an address that is not https")
            }
        }))
        .user_agent(concat!(
            "Biorouter-serve-updater/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("could not build the update client")
}

/// One release, as much of it as an update needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Without the leading `v`.
    pub version: String,
    pub assets: Vec<Asset>,
}

/// One file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: Url,
    pub size: u64,
    /// The lowercase hex SHA-256 the feed publishes, or `None` when it publishes
    /// none, in which case the file is never installed.
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

impl Release {
    /// Parse one release document in GitHub's shape. A draft is refused: the
    /// API serves drafts only to an authenticated owner, so one here is a feed
    /// that is not what it says.
    pub fn from_json(body: &str) -> Result<Self> {
        let api: ApiRelease = serde_json::from_str(body)
            .context("the release feed sent something that is not a release")?;
        if api.draft {
            bail!("the release feed offered a draft release");
        }
        let version = super::normalize_version(&api.tag_name);
        // Dotted digits only: the version names a download folder and is
        // compared numerically, so anything else is a feed to distrust.
        if version.is_empty()
            || !version
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        {
            bail!(
                "the release feed named no usable version ({})",
                api.tag_name
            );
        }
        let assets = api
            .assets
            .into_iter()
            .filter_map(|asset| {
                let url = Url::parse(&asset.browser_download_url).ok()?;
                Some(Asset {
                    sha256: asset.digest.as_deref().and_then(parse_sha256_digest),
                    name: asset.name,
                    url,
                    size: asset.size,
                })
            })
            .collect();
        Ok(Self { version, assets })
    }

    /// The asset called `name`, if the release has it.
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// `sha256:<64 hex>` → the hex, lowercased. Any other algorithm or a malformed
/// value reads as no digest, which refuses the install rather than skipping the
/// check.
pub fn parse_sha256_digest(raw: &str) -> Option<String> {
    let hex = raw.trim().strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = r#"{
        "tag_name": "v1.95.0",
        "draft": false,
        "assets": [
            {"name": "biorouter-cli_1.95.0_amd64.deb",
             "browser_download_url": "https://github.com/BaranziniLab/biorouter/releases/download/v1.95.0/biorouter-cli_1.95.0_amd64.deb",
             "size": 143468010,
             "digest": "sha256:E578AE57538D77A910F1A1CF5C4502D48C802DA4BD3686A9602148D7B6404C3A"},
            {"name": "latest-mac.yml",
             "browser_download_url": "https://github.com/BaranziniLab/biorouter/releases/download/v1.95.0/latest-mac.yml",
             "size": 526}
        ]
    }"#;

    #[test]
    fn a_release_document_is_read_in_github_s_shape() {
        let release = Release::from_json(RELEASE).unwrap();
        assert_eq!(release.version, "1.95.0");
        let deb = release.asset("biorouter-cli_1.95.0_amd64.deb").unwrap();
        assert_eq!(deb.size, 143468010);
        assert_eq!(
            deb.sha256.as_deref(),
            Some("e578ae57538d77a910f1a1cf5c4502d48c802da4bd3686a9602148d7b6404c3a")
        );
        assert_eq!(release.asset("latest-mac.yml").unwrap().sha256, None);
    }

    #[test]
    fn a_version_must_be_dotted_digits() {
        for tag in ["v1.95.0/..", "latest", "v", "1..2", "1.x.0"] {
            let body = RELEASE.replace("v1.95.0", tag);
            assert!(Release::from_json(&body).is_err(), "{tag}");
        }
    }

    #[test]
    fn a_draft_is_not_a_release() {
        let draft = RELEASE.replace("\"draft\": false", "\"draft\": true");
        assert!(Release::from_json(&draft).is_err());
    }

    #[test]
    fn only_a_sha256_digest_counts() {
        assert_eq!(parse_sha256_digest("sha512:abcd"), None);
        assert_eq!(parse_sha256_digest("sha256:abcd"), None);
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{}", "g".repeat(64))),
            None
        );
        assert!(parse_sha256_digest(&format!("sha256:{}", "a".repeat(64))).is_some());
    }

    #[test]
    fn the_override_must_be_https_or_this_machine() {
        assert!(Feed::parse("https://mirror.example.org/releases").is_ok());
        assert!(Feed::parse("http://127.0.0.1:18741/releases").is_ok());
        assert!(Feed::parse("http://[::1]:18741/releases").is_ok());
        assert!(Feed::parse("http://localhost:18741/releases").is_ok());
        assert!(Feed::parse("http://mirror.example.org/releases").is_err());
        assert!(Feed::parse("http://10.0.0.5/releases").is_err());
        assert!(Feed::parse("file:///tmp/releases").is_err());
    }

    #[test]
    fn documents_hang_off_the_base_with_or_without_a_trailing_slash() {
        for base in ["https://x.test/r", "https://x.test/r/"] {
            let feed = Feed::parse(base).unwrap();
            assert_eq!(
                feed.document("latest").unwrap().as_str(),
                "https://x.test/r/latest"
            );
        }
    }

    #[test]
    fn the_official_feed_is_github_s_release_api() {
        let feed = Feed::official();
        assert!(!feed.overridden);
        assert_eq!(
            feed.document("latest").unwrap().as_str(),
            "https://api.github.com/repos/BaranziniLab/biorouter/releases/latest"
        );
    }
}
