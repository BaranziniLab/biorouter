use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

pub const UPSTREAM_VERSION: &str = "0.3.5";
pub const UPSTREAM_COMMIT: &str = "547b4ffb8ed731a8f16486e6d8a3b215484267d3";

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
    let root = root
        .canonicalize()
        .context("computer_use_missing_runtime: payload directory is unavailable")?;
    let bytes = std::fs::read(root.join("manifest.json"))
        .context("computer_use_missing_runtime: manifest.json is unavailable")?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .context("computer_use_incompatible_runtime: invalid manifest")?;
    if manifest.schema_version != 1
        || manifest.upstream_version != UPSTREAM_VERSION
        || manifest.upstream_commit != UPSTREAM_COMMIT
        || manifest.patch_revision != "biorouter-1"
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
        let file = root.join(&entry.path).canonicalize()?;
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
    let exe = executable.canonicalize()?;
    let parent = exe
        .parent()
        .context("computer_use_missing_runtime: executable has no parent")?;
    let mut candidates = vec![
        parent.join("computer-use"),
        parent.join("resources/computer-use"),
    ];
    if let Some(contents) = parent.parent() {
        candidates.push(contents.join("Resources/computer-use"));
        candidates.push(contents.join("computer-use"));
    }
    #[cfg(target_os = "linux")]
    candidates.push(PathBuf::from("/usr/libexec/biorouter/computer-use"));
    for root in candidates {
        if root.join("manifest.json").exists() {
            return inspect(&root, false, verify_hashes);
        }
    }
    bail!("computer_use_missing_runtime: install BioRouter's matching bundled Computer Use payload; source builds may explicitly set BIOROUTER_COMPUTER_USE_DIR")
}
