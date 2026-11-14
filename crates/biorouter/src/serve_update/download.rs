//! Downloading a release file and proving it is the one the release published.
//!
//! The file is written beside its final name as `.part`, hashed as it arrives,
//! and renamed into place only when its size and SHA-256 both match what the
//! feed published. A file that does not match is deleted, so nothing that was
//! not verified is ever left where [`super::apply`] looks.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::feed::{allowed_url, Asset};

/// Where downloads for this account are kept: one folder per version, so a
/// newer release never reuses a half-written older one.
pub fn downloads_dir() -> PathBuf {
    crate::config::paths::Paths::in_data_dir("serve-update")
}

/// Download `asset` into `dir` and verify it, reporting `(received, total)` as
/// it arrives. A file already there with the right digest is reused, so a
/// check that runs again after a restart does not fetch it twice.
pub async fn fetch_verified(
    client: &reqwest::Client,
    asset: &Asset,
    dir: &Path,
    mut progress: impl FnMut(u64, u64) + Send,
) -> Result<PathBuf> {
    let Some(expected) = asset.sha256.as_deref() else {
        bail!(
            "the release publishes no SHA-256 digest for {}, so it cannot be checked and is not installed",
            asset.name
        );
    };
    if !allowed_url(&asset.url) {
        bail!(
            "{} is offered from an address that is not https",
            asset.name
        );
    }
    if asset.name.contains('/') || asset.name.contains('\\') || asset.name.starts_with('.') {
        bail!(
            "the release names a file Biorouter will not write: {}",
            asset.name
        );
    }
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("could not create {}", dir.display()))?;
    let target = dir.join(&asset.name);
    if sha256_file(&target).await.ok().as_deref() == Some(expected) {
        progress(asset.size, asset.size);
        return Ok(target);
    }

    let partial = dir.join(format!("{}.part", asset.name));
    let response = client
        .get(asset.url.clone())
        .send()
        .await
        .with_context(|| format!("could not download {}", asset.name))?;
    let status = response.status();
    if !status.is_success() {
        bail!(
            "downloading {} failed: the server answered {status}",
            asset.name
        );
    }
    let total = response.content_length().unwrap_or(asset.size);
    let mut file = tokio::fs::File::create(&partial)
        .await
        .with_context(|| format!("could not write {}", partial.display()))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut stream = response.bytes_stream();
    let outcome: Result<()> = async {
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.with_context(|| format!("the download of {} was cut off", asset.name))?;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            received += chunk.len() as u64;
            progress(received, total);
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok(())
    }
    .await;
    drop(file);
    if let Err(error) = outcome {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(error);
    }
    if asset.size != 0 && received != asset.size {
        let _ = tokio::fs::remove_file(&partial).await;
        bail!(
            "{} arrived with {received} bytes where the release lists {}",
            asset.name,
            asset.size
        );
    }
    let actual = hex(&hasher.finalize());
    if actual != expected {
        let _ = tokio::fs::remove_file(&partial).await;
        // The two digests go to the log; the page gets one line.
        tracing::warn!(file = %asset.name, %expected, %actual, "a download did not match its published SHA-256");
        bail!(
            "{} does not match the SHA-256 the release publishes, so it was deleted",
            asset.name
        );
    }
    tokio::fs::rename(&partial, &target).await?;
    Ok(target)
}

/// The SHA-256 of a file on disk, lowercase hex.
pub async fn sha256_file(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1 << 16];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex(&hasher.finalize()))
    })
    .await?
}

/// Check `path` still has the digest it was verified with. Called again right
/// before installing, because the folder it sits in is this account's.
pub async fn still_verified(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)
        .await
        .with_context(|| format!("could not read {}", path.display()))?;
    if actual != expected {
        bail!(
            "{} changed after it was verified, so it was not installed",
            path.display()
        );
    }
    Ok(())
}

/// Remove every downloaded version except `keep`.
pub async fn prune(keep: &[&str]) {
    let Ok(mut entries) = tokio::fs::read_dir(downloads_dir()).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().await.is_ok_and(|t| t.is_dir()) && !keep.contains(&name.as_str()) {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn asset(server: &MockServer, body: &[u8], digest: Option<String>) -> Asset {
        Asset {
            name: "biorouter-cli_9.9.9_amd64.deb".into(),
            url: url::Url::parse(&format!("{}/dl/pkg.deb", server.uri())).unwrap(),
            size: body.len() as u64,
            sha256: digest,
        }
    }

    fn sha(body: &[u8]) -> String {
        hex(&Sha256::digest(body))
    }

    #[tokio::test]
    async fn a_file_that_matches_its_digest_is_kept() {
        let server = MockServer::start().await;
        let body = b"package bytes".to_vec();
        Mock::given(method("GET"))
            .and(path("/dl/pkg.deb"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let mut last = (0, 0);
        let saved = fetch_verified(
            &reqwest::Client::new(),
            &asset(&server, &body, Some(sha(&body))),
            dir.path(),
            |got, total| last = (got, total),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&saved).unwrap(), body);
        assert_eq!(last.0, body.len() as u64);
        assert!(!dir
            .path()
            .join("biorouter-cli_9.9.9_amd64.deb.part")
            .exists());
    }

    #[tokio::test]
    async fn a_file_that_does_not_match_is_deleted_and_refused() {
        let server = MockServer::start().await;
        let body = b"tampered".to_vec();
        Mock::given(method("GET"))
            .and(path("/dl/pkg.deb"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let error = fetch_verified(
            &reqwest::Client::new(),
            &asset(&server, &body, Some(sha(b"the real one"))),
            dir.path(),
            |_, _| {},
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not match"), "{error}");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "nothing left behind"
        );
    }

    #[tokio::test]
    async fn a_file_without_a_published_digest_is_never_downloaded() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let error = fetch_verified(
            &reqwest::Client::new(),
            &asset(&server, b"x", None),
            dir.path(),
            |_, _| {},
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("no SHA-256"), "{error}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_short_download_is_refused() {
        let server = MockServer::start().await;
        let body = b"short".to_vec();
        Mock::given(method("GET"))
            .and(path("/dl/pkg.deb"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let mut listed = asset(&server, &body, Some(sha(&body)));
        listed.size = 999;
        let error = fetch_verified(&reqwest::Client::new(), &listed, dir.path(), |_, _| {})
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("999"), "{error}");
    }

    #[tokio::test]
    async fn a_changed_file_is_caught_before_install() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.deb");
        std::fs::write(&file, b"verified").unwrap();
        let digest = sha(b"verified");
        still_verified(&file, &digest).await.unwrap();
        std::fs::write(&file, b"swapped").unwrap();
        assert!(still_verified(&file, &digest).await.is_err());
    }
}
