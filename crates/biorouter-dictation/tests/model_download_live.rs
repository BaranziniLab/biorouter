//! The installer against A-1's real release (PR-11): downloads the 885,098 B voice
//! detector from `BaranziniLab/biorouter-models` end to end, through github.com's
//! redirect to its asset host, and checks the installed bytes against the pin.
//!
//! Ignored: it needs the network. Run it with
//! `cargo test -p biorouter-dictation --test model_download_live -- --ignored`.
//! The production network accepts a body only from `release-assets.githubusercontent.com`
//! or `objects.githubusercontent.com` over https, so an install that succeeds proves the
//! final host was one of the two.

use std::time::Instant;

use biorouter_dictation::catalog::{CATALOG, VAD_ID};
use biorouter_dictation::models::{self, InstallOptions, Network, Source, WriteMode};

#[tokio::test]
#[ignore = "downloads a pinned speech model"]
async fn the_detector_downloads_from_the_pinned_release() {
    let entry = CATALOG
        .entry(VAD_ID)
        .expect("the detector is in the catalog");
    let root = tempfile::tempdir().unwrap();
    let options = InstallOptions::new(Network::production(), WriteMode::User);
    let started = Instant::now();
    let outcomes = models::install_files(root.path(), &[entry], &options)
        .await
        .unwrap_or_else(|e| panic!("the detector did not install: {e}"));
    eprintln!("installed in {:?}", started.elapsed());
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].downloaded);
    assert_eq!(outcomes[0].source, Source::Download);
    let path = models::model_path(root.path(), entry);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), entry.size_bytes);
    assert_eq!(models::sha256_file(&path).unwrap(), entry.sha256);
    assert!(models::verify_writable(root.path(), entry).is_verified());
    assert!(root
        .path()
        .join(VAD_ID)
        .join("LICENSE-silero-vad.txt")
        .is_file());
    // Nothing is left of the partial download.
    assert!(!root.path().join(models::PARTIAL_DIR).exists());
}
