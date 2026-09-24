use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

pub const UPSTREAM_VERSION: &str = "0.3.5";
pub const UPSTREAM_COMMIT: &str = "547b4ffb8ed731a8f16486e6d8a3b215484267d3";

#[derive(Deserialize)]
struct RuntimePin {
    schema_version: u32,
    upstream_version: String,
    upstream_commit: String,
    patch_revision: String,
}

static RUNTIME_PIN: LazyLock<RuntimePin> = LazyLock::new(|| {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../vendor/computer-use/pin.json"
    )))
    .expect("bundled Biorouter Copilot pin must be valid")
});

pub fn patch_revision() -> &'static str {
    &RUNTIME_PIN.patch_revision
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub upstream_version: String,
    pub upstream_commit: String,
    pub patch_revision: String,
    pub target: String,
    pub executable: PathBuf,
    pub files: Vec<ManifestFile>,
}
#[derive(Debug, Deserialize)]
pub struct ManifestFile {
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Debug)]
pub struct RuntimePayload {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub development_override: bool,
    pub payload_id: String,
}

pub fn target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    format!("{os}-{arch}")
}

fn relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

pub fn validate(root: &Path, development_override: bool) -> Result<RuntimePayload> {
    inspect(root, development_override, true)
}

fn inspect(root: &Path, development_override: bool, verify_hashes: bool) -> Result<RuntimePayload> {
    // ⚠ `dunce::canonicalize`, never `Path::canonicalize`. On Windows the latter
    // returns a verbatim path (`\\?\C:\…`), and this value does not stay local:
    // it becomes `RuntimePayload::root`/`executable`, which `diagnostics()`
    // serializes into `biorouter doctor --format json`. The installed-payload
    // acceptance check compares that against a path it spelled itself, and
    // Python's `Path.resolve()` preserves a verbatim prefix it was given, so the
    // two can never be equal and the check fails as "Doctor resolved outside the
    // installed payload" on EVERY Windows install.
    let root = dunce::canonicalize(root)
        .context("computer_use_missing_runtime: payload directory is unavailable")?;
    let bytes = std::fs::read(root.join("manifest.json"))
        .context("computer_use_missing_runtime: manifest.json is unavailable")?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .context("computer_use_incompatible_runtime: invalid manifest")?;
    if manifest.schema_version != RUNTIME_PIN.schema_version
        || manifest.upstream_version != RUNTIME_PIN.upstream_version
        || manifest.upstream_commit != RUNTIME_PIN.upstream_commit
        || manifest.patch_revision != RUNTIME_PIN.patch_revision
        || manifest.target != target()
    {
        bail!("computer_use_incompatible_runtime: runtime pin, patch or platform does not match BioRouter");
    }
    if !relative_path(&manifest.executable)
        || !manifest
            .files
            .iter()
            .any(|entry| entry.path == manifest.executable)
    {
        bail!(
            "computer_use_incompatible_runtime: executable must be a hashed relative payload file"
        );
    }
    for entry in &manifest.files {
        if !relative_path(&entry.path) {
            bail!("computer_use_incompatible_runtime: invalid payload path");
        }
        // ⚠ This MUST use the same canonicaliser as `root` above, and changing
        // one without the other is worse than changing neither. `starts_with`
        // compares path COMPONENTS, and a prefix component carries its kind:
        // `Prefix::Disk('C')` and `Prefix::VerbatimDisk('C')` are different
        // values even though they name the same volume. A simplified `root`
        // against a verbatim `file` therefore fails the containment check and
        // every Windows install would bail with "payload file escapes its
        // directory" — turning a reporting bug into a hard startup failure.
        let file = dunce::canonicalize(root.join(&entry.path))?;
        if !file.starts_with(&root) {
            bail!("computer_use_incompatible_runtime: payload file escapes its directory");
        }
        if verify_hashes {
            let digest = format!("{:x}", Sha256::digest(std::fs::read(file)?));
            if digest != entry.sha256 {
                bail!("computer_use_incompatible_runtime: payload checksum mismatch");
            }
        }
    }
    Ok(RuntimePayload {
        executable: root.join(manifest.executable),
        root,
        development_override,
        payload_id: format!("{:x}", Sha256::digest(&bytes)),
    })
}

pub fn locate() -> Result<RuntimePayload> {
    locate_mode(true)
}

pub fn probe() -> Result<RuntimePayload> {
    locate_mode(false)
}

fn locate_mode(verify_hashes: bool) -> Result<RuntimePayload> {
    if let Some(root) = std::env::var_os("BIOROUTER_COMPUTER_USE_DIR") {
        return inspect(Path::new(&root), true, verify_hashes);
    }
    locate_executable(&std::env::current_exe()?, verify_hashes)
}

pub fn locate_for_executable(executable: &Path) -> Result<RuntimePayload> {
    locate_executable(executable, true)
}

fn locate_executable(executable: &Path, verify_hashes: bool) -> Result<RuntimePayload> {
    let exe = dunce::canonicalize(executable)?;
    let parent = exe
        .parent()
        .context("computer_use_missing_runtime: executable has no parent")?;
    let mut candidates = payload_candidates(parent);
    if let Some(origin) = install_origin(parent) {
        candidates.extend(payload_candidates(&origin));
    }
    #[cfg(target_os = "linux")]
    candidates.push(PathBuf::from("/usr/libexec/biorouter/computer-use"));
    for root in candidates {
        if root.join("manifest.json").exists() {
            return inspect(&root, false, verify_hashes);
        }
    }
    bail!("computer_use_missing_runtime: install the matching bundled Biorouter Copilot payload; source builds may explicitly set BIOROUTER_COMPUTER_USE_DIR")
}

fn payload_candidates(bin: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![bin.join("computer-use"), bin.join("resources/computer-use")];
    if let Some(parent) = bin.parent() {
        candidates.push(parent.join("Resources/computer-use"));
        candidates.push(parent.join("computer-use"));
    }
    candidates
}

fn install_origin(bin: &Path) -> Option<PathBuf> {
    // Keep the installer breadcrumb format aligned with biorouter::system::install_origin.
    // MCP cannot import the host crate because the host already depends on MCP.
    let raw = std::fs::read_to_string(bin.join(".biorouter-origin")).ok()?;
    let line = raw.trim_start_matches('\u{feff}').lines().next()?.trim();
    let path = Path::new(line);
    // `biorouter::system::install_origin`, which this says it mirrors, ends with
    // `dunce::simplified`. Dropping it here would let a breadcrumb written by an
    // installer that spelled the path verbatim seed a verbatim candidate root.
    path.is_absolute()
        .then(|| dunce::simplified(path).to_path_buf())
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    #[test]
    fn current_bundled_pin_is_accepted_and_other_patch_revisions_are_refused() {
        assert_eq!(RUNTIME_PIN.upstream_version, UPSTREAM_VERSION);
        assert_eq!(RUNTIME_PIN.upstream_commit, UPSTREAM_COMMIT);

        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("ocu");
        std::fs::write(&executable, b"fixture helper").unwrap();
        let digest = format!("{:x}", Sha256::digest(std::fs::read(&executable).unwrap()));
        let mut manifest = serde_json::json!({
            "schema_version": RUNTIME_PIN.schema_version,
            "upstream_version": RUNTIME_PIN.upstream_version,
            "upstream_commit": RUNTIME_PIN.upstream_commit,
            "patch_revision": patch_revision(),
            "target": target(),
            "executable": "ocu",
            "files": [{ "path": "ocu", "sha256": digest }],
        });
        std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
        inspect(dir.path(), false, true).expect("the bundled pin must be accepted");

        manifest["patch_revision"] = serde_json::json!("outdated-patch");
        std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
        assert!(inspect(dir.path(), false, true)
            .unwrap_err()
            .to_string()
            .contains("computer_use_incompatible_runtime"));
    }
}

#[cfg(all(test, windows))]
mod windows_path_spelling_tests {
    use super::*;
    use std::path::Prefix;

    /// Build a payload directory that `inspect` will accept, so the test
    /// exercises the real function rather than a re-implementation of it.
    fn payload(dir: &Path) -> std::io::Result<()> {
        let exe = dir.join("ocu.exe");
        std::fs::write(&exe, b"not a real helper")?;
        let digest = format!("{:x}", Sha256::digest(std::fs::read(&exe)?));
        let manifest = serde_json::json!({
            "schema_version": 1,
            "upstream_version": UPSTREAM_VERSION,
            "upstream_commit": UPSTREAM_COMMIT,
            "patch_revision": patch_revision(),
            "target": target(),
            "executable": "ocu.exe",
            "files": [{ "path": "ocu.exe", "sha256": digest }],
        });
        std::fs::write(dir.join("manifest.json"), manifest.to_string())
    }

    /// ⚠ The bug this pins is a REPORTING bug with a remote blast radius, which
    /// is why it survived: nothing on the Rust side misbehaves. `inspect`
    /// canonicalizes, and on Windows `Path::canonicalize` returns a verbatim
    /// path (`\?\C:\…`). That value becomes `RuntimePayload::executable`, which
    /// `diagnostics()` serializes into `biorouter doctor --format json`, which
    /// the installed-package acceptance check compares against a path it
    /// spelled itself. Python's `Path.resolve()` PRESERVES a verbatim prefix it
    /// is given, so the two strings can never match and the job fails with
    /// "Doctor resolved outside the installed payload".
    ///
    /// It is unconditional on Windows. The acceptance job's install directory
    /// happens to contain a space and a non-ASCII character, which makes the
    /// failure look like an encoding problem; it is not, and chasing that is
    /// how this stays unfixed.
    #[test]
    fn no_reported_payload_path_is_a_windows_verbatim_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        payload(dir.path()).expect("write payload");

        let found = inspect(dir.path(), false, true).expect("payload must validate");

        for (label, path) in [("root", &found.root), ("executable", &found.executable)] {
            // Ask the path what KIND of prefix it has rather than matching the
            // literal characters. The first draft of this test compared against
            // a raw string and the assertion silently tested the wrong prefix,
            // so it passed with the bug reinstated — a gate that cannot fail is
            // worse than no gate. `Prefix::kind()` cannot be mis-spelled.
            let verbatim = matches!(
                path.components().next(),
                Some(Component::Prefix(prefix)) if matches!(
                    prefix.kind(),
                    Prefix::Verbatim(_) | Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(..)
                )
            );
            assert!(
                !verbatim,
                "{label} is a verbatim path ({}). It is serialized into \
                 `biorouter doctor --format json`, where the installed-payload \
                 check compares it against a plainly-spelled path and can never \
                 match. Use `dunce::canonicalize`, not `Path::canonicalize`.",
                path.display()
            );
        }
    }

    /// The containment check in `inspect` compares path COMPONENTS, and a
    /// prefix component carries its kind — `Prefix::Disk('C')` is not equal to
    /// `Prefix::VerbatimDisk('C')` even though both name drive C. So
    /// simplifying `root` while leaving the per-file canonicalize verbatim does
    /// not half-fix the bug, it converts it into a hard failure: every payload
    /// file would "escape its directory" and no Windows install would start.
    /// This test fails in exactly that case, which the test above cannot see.
    #[test]
    fn a_payload_validates_at_all_so_the_two_canonicalisers_agree() {
        let dir = tempfile::tempdir().expect("tempdir");
        payload(dir.path()).expect("write payload");

        let found = inspect(dir.path(), false, true).expect(
            "payload must validate: if this fails with 'payload file escapes its \
             directory', `root` and the per-file path were canonicalised by \
             different functions and their Prefix components disagree",
        );
        assert!(found.executable.starts_with(&found.root));
    }
}
