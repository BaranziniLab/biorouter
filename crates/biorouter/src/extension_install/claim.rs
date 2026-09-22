//! What a stopped install leaves behind on disk, so somebody can still finish it.
//!
//! # Why a file and not a process-global map
//!
//! [`super::transaction`] used to record a stopped install in a
//! `Mutex<HashMap>` that lived for the life of the process. Two things were
//! wrong with that, and they compound:
//!
//! * it died with the process, so the one case this exists for — "I closed the
//!   app before I had the passcode to hand" — was never recoverable; and
//! * it recorded a `bundle_path`, which for a marketplace install pointed
//!   inside a [`tempfile::TempDir`] dropped the moment the install returned.
//!   The field was a dangling path before anything could read it.
//!
//! A claim is written to `<config>/extension-installs/` instead, records the
//! **re-fetchable source** rather than a temp path, and is removed the moment
//! the install it describes succeeds or is fully rolled back. A claim that
//! outlives its install is a permanent phantom "pending install" on every
//! reader, which is why both ends of [`super::transaction`] delete it.
//!
//! # It holds key NAMES, never values
//!
//! The same rule as [`super::transaction::InstallReport`], and for a sharper
//! reason: this is a plaintext file in the user's config directory.
//! `pending_keys` says *which* variables a resume still needs. There is nowhere
//! in this struct a value can sit, and
//! `a_parked_claim_records_key_names_and_never_a_value` is what keeps it that
//! way.
//!
//! # Why not inside the extension's own tree
//!
//! [`super::brxt::BrxtBundle::extract_to`] writes any zip-slip-safe path inside
//! the install directory, so a marker kept in there could be forged by the very
//! bundle it claims to describe. The claims directory is a sibling of
//! `extensions/`, not a child.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::paths::Paths;

use super::transaction::InstallSource;

/// How far the install got before it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimPhase {
    /// The tree is being written. A claim left in this phase is an install that
    /// died between `extract_to` and its own rollback — process death, not an
    /// `Err`, because every `Err` path rolls back and deletes the claim.
    Extracting,
    /// Stopped at the credential step, waiting for a person. This is what
    /// `biorouter extension configure <name>` finishes.
    Parked,
}

/// Where the bundle came from, in a form that survives the process.
///
/// ⚠ This is deliberately **not** the path the bundle was read from. For a
/// marketplace install that path is inside a temp directory already deleted;
/// the URL is what a resume can actually fetch again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClaimSource {
    LocalFile { path: PathBuf },
    Marketplace { registry_id: String, url: String },
}

impl From<&InstallSource> for ClaimSource {
    fn from(source: &InstallSource) -> Self {
        match source {
            InstallSource::LocalFile { path } => Self::LocalFile { path: path.clone() },
            InstallSource::Marketplace { registry_id, url } => Self::Marketplace {
                registry_id: registry_id.clone(),
                url: url.clone(),
            },
        }
    }
}

/// One install's claim on one extension directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallClaim {
    /// The transaction's id. A resume reuses it, so the finished install is the
    /// same install rather than a second one.
    pub install_id: String,
    pub extension_name: String,
    pub display_name: String,
    /// The tree this run wrote, which is also where its `manifest.json` and its
    /// built `.venv` are.
    pub install_dir: PathBuf,
    /// Whether the tree was already there when this run started. A resume must
    /// not treat an upgrade's surviving tree as its own creation.
    pub existed_before: bool,
    pub phase: ClaimPhase,
    /// **Names only.** What a resume still has to collect.
    pub pending_keys: Vec<String>,
    pub source: ClaimSource,
    /// Seconds since the epoch, so the newest claim for a name wins.
    pub written_at: u64,
}

impl InstallClaim {
    /// A claim for a run that is about to write its tree.
    pub fn new(
        install_id: impl Into<String>,
        extension_name: impl Into<String>,
        display_name: impl Into<String>,
        install_dir: impl Into<PathBuf>,
        existed_before: bool,
        source: ClaimSource,
    ) -> Self {
        Self {
            install_id: install_id.into(),
            extension_name: extension_name.into(),
            display_name: display_name.into(),
            install_dir: install_dir.into(),
            existed_before,
            phase: ClaimPhase::Extracting,
            pending_keys: Vec::new(),
            source,
            written_at: now_secs(),
        }
    }

    /// The same claim, stopped at the credential step and waiting on
    /// `pending_keys`.
    pub fn parked(mut self, pending_keys: Vec<String>) -> Self {
        self.phase = ClaimPhase::Parked;
        self.pending_keys = pending_keys;
        self
    }
}

/// `<config>/extension-installs/`.
///
/// Beside `extensions/`, never inside it — see the module header.
pub fn claims_dir() -> PathBuf {
    Paths::in_config_dir("extension-installs")
}

/// Hex-encode the id byte-by-byte, exactly as
/// `privacy::provenance::pointer_filename` does, so no id — however it was
/// spelled — can name a file outside [`claims_dir`].
fn claim_filename(install_id: &str) -> String {
    install_id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Write (or rewrite) a claim atomically.
///
/// Temp-file-then-`persist`, copying `privacy::provenance::write_current_pointer`:
/// a reader must never see a half-written claim, and a claim rewritten from
/// `Extracting` to `Parked` must not pass through a state where it is neither.
pub fn write_claim(claim: &InstallClaim) -> std::io::Result<()> {
    write_claim_in(&claims_dir(), claim)
}

/// [`write_claim`], [`read_claims`] and [`remove_claim`] over `directory`
/// rather than [`claims_dir`]. The tests pass a directory of their own here
/// instead of pointing `BIOROUTER_PATH_ROOT` at one: every test in the binary
/// that resolves `Paths` without the env lock would otherwise follow it.
fn write_claim_in(directory: &std::path::Path, claim: &InstallClaim) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let body = serde_json::to_vec(claim)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    temp.write_all(&body)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(directory.join(claim_filename(&claim.install_id)))
        .map_err(|error| error.error)?;
    Ok(())
}

/// Every claim on this machine, newest first.
///
/// ⚠ **Never an `Err`, and never one bad file's fault.** A reader that `?`s on
/// the first unparseable entry reports *no* pending installs, which reads to the
/// user as "there is nothing to finish" — the exact opposite of the truth. A
/// file that cannot be parsed, and one whose tree has since been deleted,
/// describe nothing a resume could use, so they are dropped and cleaned up.
pub fn read_claims() -> Vec<InstallClaim> {
    read_claims_in(&claims_dir())
}

fn read_claims_in(directory: &std::path::Path) -> Vec<InstallClaim> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut claims: Vec<InstallClaim> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let parsed = std::fs::read(&path)
            .ok()
            .and_then(|body| serde_json::from_slice::<InstallClaim>(&body).ok())
            .filter(|claim| claim.install_dir.exists());
        match parsed {
            Some(claim) => claims.push(claim),
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    claims.sort_by(|a, b| b.written_at.cmp(&a.written_at));
    claims
}

/// Drop the claim for `install_id`, if there is one. Absent is success.
pub fn remove_claim(install_id: &str) {
    remove_claim_in(&claims_dir(), install_id)
}

fn remove_claim_in(directory: &std::path::Path, install_id: &str) {
    let _ = std::fs::remove_file(directory.join(claim_filename(install_id)));
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A config tree of the test's own and the claims directory inside it,
    /// handed to the `_in` functions. It used to be `BIOROUTER_PATH_ROOT`,
    /// relocated under the env lock, which every unlocked `Paths` reader in the
    /// binary then followed; a directory passed in is nobody's ambient root.
    /// Being a fresh `TempDir`, it cannot be the developer's real
    /// `~/.config/biorouter`, which holds live extensions.
    fn sandbox() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("a temp dir");
        let claims = tmp.path().join("config").join("extension-installs");
        (tmp, claims)
    }

    /// Production keeps the claims where the fixture's shape puts them:
    /// `<config>/extension-installs/`. Paths only.
    #[test]
    fn claims_live_in_the_config_dir() {
        let _root = crate::test_sandbox::pin_sandbox_path_root();
        assert_eq!(
            claims_dir(),
            std::path::Path::new(crate::test_sandbox::sandbox_path_root())
                .join("config")
                .join("extension-installs")
        );
    }

    fn claim_in(tree: &Path, install_id: &str) -> InstallClaim {
        std::fs::create_dir_all(tree).unwrap();
        InstallClaim::new(
            install_id,
            "spokeagent",
            "SPOKE Agent",
            tree,
            false,
            ClaimSource::LocalFile {
                path: PathBuf::from("/bundles/spokeagent.brxt"),
            },
        )
        .parked(vec!["SPOKEAGENT_PASSCODE".to_string()])
    }

    /// One corrupt file must not hide every real pending install.
    #[test]
    fn an_unparseable_claim_is_skipped_not_fatal() {
        let (tmp, claims_at) = sandbox();
        let tree = tmp.path().join("config/extensions/spokeagent");
        write_claim_in(&claims_at, &claim_in(&tree, "i-good")).unwrap();
        let junk = claims_at.join("6465616462656566");
        std::fs::write(&junk, b"{ this is not json").unwrap();

        let claims = read_claims_in(&claims_at);
        assert_eq!(claims.len(), 1, "{claims:?}");
        assert_eq!(claims[0].install_id, "i-good");
        assert_eq!(claims[0].phase, ClaimPhase::Parked);
        assert_eq!(claims[0].pending_keys, vec!["SPOKEAGENT_PASSCODE"]);
        assert!(!junk.exists(), "the unreadable file should be cleaned up");
    }

    /// A claim whose tree has been deleted describes nothing a resume can
    /// finish — the manifest it would read its variables out of is gone.
    #[test]
    fn a_claim_whose_tree_is_gone_is_dropped() {
        let (tmp, claims_at) = sandbox();
        let tree = tmp.path().join("config/extensions/spokeagent");
        write_claim_in(&claims_at, &claim_in(&tree, "i-stale")).unwrap();
        std::fs::remove_dir_all(&tree).unwrap();

        assert!(read_claims_in(&claims_at).is_empty());
        assert!(
            std::fs::read_dir(&claims_at).unwrap().next().is_none(),
            "the stale claim should be cleaned up too"
        );
    }

    #[test]
    fn a_claim_is_rewritten_in_place_and_removable() {
        let (tmp, claims_at) = sandbox();
        let tree = tmp.path().join("config/extensions/spokeagent");
        let claim = claim_in(&tree, "i-1");
        write_claim_in(&claims_at, &claim).unwrap();
        write_claim_in(
            &claims_at,
            &InstallClaim {
                phase: ClaimPhase::Extracting,
                ..claim
            },
        )
        .unwrap();

        let claims = read_claims_in(&claims_at);
        assert_eq!(claims.len(), 1, "a rewrite must not add a second file");
        assert_eq!(claims[0].phase, ClaimPhase::Extracting);

        remove_claim_in(&claims_at, "i-1");
        assert!(read_claims_in(&claims_at).is_empty());
        // Removing what is not there is success, not an error.
        remove_claim_in(&claims_at, "i-1");
    }

    /// The filename is derived, not taken. An id spelled as a path must not be
    /// able to name a file outside the claims directory.
    ///
    /// Deliberately free of any filesystem or `claims_dir()` access: this is a
    /// property of the encoding, and reading the (env-derived) claims directory
    /// twice in one assertion is a race against every other test in the binary
    /// that relocates the config root.
    #[test]
    fn a_traversing_install_id_cannot_escape_the_claims_directory() {
        use std::path::Component;

        // A raw id used as a filename would walk out of the directory.
        assert!(Path::new("../../evil").components().count() > 1);

        let encoded = claim_filename("../../evil");
        assert!(encoded.chars().all(|c| c.is_ascii_hexdigit()), "{encoded}");
        let components: Vec<Component<'_>> = Path::new(&encoded).components().collect();
        assert_eq!(
            components.len(),
            1,
            "not a single path component: {encoded}"
        );
        assert!(matches!(components[0], Component::Normal(_)));
    }
}
