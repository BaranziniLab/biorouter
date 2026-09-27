//! Reading a `.brxt` bundle: validate its shape, parse its manifest, extract it,
//! and build its Python environment.
//!
//! This is the one implementation of "what is in this bundle", shared by
//! [`super::transaction::ExtensionInstallTransaction`] and therefore by the CLI,
//! the daemon and any agent tool. It was previously a private copy inside
//! `biorouter-cli`, with a second copy in the Electron main process; the CLI's
//! copy is gone and this is what replaced it.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::conversation::message::SecretKeyRequest;

/// One environment value an extension declares in its manifest.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct BrxtEnvVar {
    pub key: String,
    #[serde(default)]
    pub required: bool,
    /// Whether a machine-wide value of the same name should be reused.
    #[serde(default)]
    pub auto_propagate: bool,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub description: String,
    /// Whether the value is a credential. **This is the only thing that decides
    /// whether a value may be written to `config.yaml`**: a secret goes to the
    /// OS credential store and only its key name is recorded beside the config.
    #[serde(default)]
    pub secret: bool,
}

impl BrxtEnvVar {
    pub(super) fn has_stored_secret(&self) -> bool {
        self.secret && secret_already_stored(&self.key)
    }

    /// The card field for this variable. Carries the name, a label and help
    /// text — never a value, and never the `default`, because a card that
    /// pre-filled a secret would be showing the user something the surface had
    /// to read back out of the keyring first.
    pub fn as_key_request(&self) -> SecretKeyRequest {
        SecretKeyRequest {
            key: self.key.clone(),
            label: self.key.clone(),
            description: (!self.description.is_empty()).then(|| self.description.clone()),
            required: self.required,
        }
    }
}

/// A bundle's `manifest.json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BrxtManifest {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub entry_point: String,
    pub repository: String,
    #[serde(default)]
    pub tools_count: Option<u32>,
    #[serde(default)]
    pub env_vars: Vec<BrxtEnvVar>,
}

impl BrxtManifest {
    /// The variables the bundle cannot run without.
    pub fn required_vars(&self) -> impl Iterator<Item = &BrxtEnvVar> {
        self.env_vars.iter().filter(|v| v.required)
    }

    /// The declared variables still missing once `supplied` is applied.
    ///
    /// A value already in the credential store counts as supplied — an install
    /// that re-asks for a passcode the machine already holds is an install that
    /// trains the user to paste secrets they did not need to.
    pub fn unmet_requirements(&self, supplied: &HashMap<String, String>) -> Vec<&BrxtEnvVar> {
        self.required_vars()
            .filter(|v| !supplied.contains_key(&v.key) && !v.has_stored_secret())
            .collect()
    }
}

/// Whether the credential store already holds `key`.
///
/// The value is fetched and **immediately dropped**: nothing outside the store
/// ever learns what it is, only that it exists. A read failure reads as absent,
/// which is the direction that asks the user rather than the direction that
/// registers an extension that cannot authenticate.
pub fn secret_already_stored(key: &str) -> bool {
    crate::config::Config::global()
        .get_secret::<String>(key)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

/// A bundle a caller may read a manifest out of, extract, or refuse.
pub struct BrxtBundle {
    path: PathBuf,
    manifest: BrxtManifest,
    skills: Vec<BundledSkill>,
}

/// A skill shipped inside a bundle, as `skills/<slug>/SKILL.md`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BundledSkill {
    pub slug: String,
    pub name: String,
    pub description: String,
}

impl BrxtBundle {
    /// Open and validate `path`, returning a friendly message if it is not a
    /// bundle. The four structural checks and the required-field list mirror the
    /// desktop validator exactly — a bundle the GUI accepts and the CLI refuses
    /// (or the reverse) is a bug report nobody can reproduce.
    pub fn open(path: &Path) -> Result<Self> {
        if !path.exists() {
            bail!("File not found: {}", path.display());
        }
        let file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| anyhow!("Not a valid .brxt (zip) bundle: {}", e))?;

        let names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
            .collect();

        for (present, missing) in [
            (names.iter().any(|n| n == "manifest.json"), "manifest.json"),
            (
                names.iter().any(|n| n.eq_ignore_ascii_case("readme.md")),
                "README.md",
            ),
            (
                names.iter().any(|n| n == "pyproject.toml"),
                "pyproject.toml",
            ),
            (
                names.iter().any(|n| n.starts_with("src/")),
                "src/ directory",
            ),
        ] {
            if !present {
                bail!("Missing {missing}: not a valid .brxt bundle");
            }
        }

        let manifest: BrxtManifest = {
            let mut entry = archive
                .by_name("manifest.json")
                .map_err(|e| anyhow!("Could not read manifest.json: {}", e))?;
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            serde_json::from_str(&buf).map_err(|e| anyhow!("Invalid manifest.json: {}", e))?
        };
        if manifest.name.trim().is_empty() {
            bail!("manifest.json missing required field: \"name\"");
        }
        validate_extension_name(&manifest.name)?;
        if manifest.entry_point.trim().is_empty() {
            bail!("manifest.json missing required field: \"entry_point\"");
        }

        let skills = read_bundled_skills(&mut archive, &names);
        Ok(Self {
            path: path.to_path_buf(),
            manifest,
            skills,
        })
    }

    pub fn manifest(&self) -> &BrxtManifest {
        &self.manifest
    }

    pub fn skills(&self) -> &[BundledSkill] {
        &self.skills
    }

    /// Extract every file under `dest`, refusing any entry whose path escapes it.
    pub fn extract_to(&self, dest: &Path) -> Result<()> {
        let file = fs::File::open(&self.path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            // `enclosed_name` is the zip-slip guard: it returns `None` for any
            // path that would leave `dest`, including `..` and absolute paths.
            let Some(rel) = entry.enclosed_name().map(Path::to_path_buf) else {
                bail!("Unsafe path in bundle: {}", entry.name());
            };
            let out_path = dest.join(&rel);
            if entry.is_dir() {
                fs::create_dir_all(&out_path)?;
                continue;
            }
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&out_path)
                .with_context(|| format!("writing {}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out)?;
        }
        Ok(())
    }
}

fn read_bundled_skills(
    archive: &mut zip::ZipArchive<fs::File>,
    names: &[String],
) -> Vec<BundledSkill> {
    let mut skills = Vec::new();
    for name in names {
        let Some(slug) = name
            .strip_prefix("skills/")
            .and_then(|rest| rest.strip_suffix("/SKILL.md"))
        else {
            continue;
        };
        if slug.is_empty() || slug.contains('/') {
            continue;
        }
        let mut body = String::new();
        if archive
            .by_name(name)
            .ok()
            .and_then(|mut e| e.read_to_string(&mut body).ok())
            .is_none()
        {
            continue;
        }
        if let Some((skill_name, description)) = skill_frontmatter(&body) {
            skills.push(BundledSkill {
                slug: slug.to_string(),
                name: skill_name,
                description,
            });
        }
    }
    skills.sort_by(|a, b| a.slug.cmp(&b.slug));
    skills
}

/// `name:` and `description:` out of a `SKILL.md` YAML frontmatter block.
///
/// Line-wise rather than by byte offset: the body is author-supplied text that
/// routinely carries non-ASCII, and slicing a `&str` at a found index panics if
/// the index lands inside a UTF-8 character.
fn skill_frontmatter(body: &str) -> Option<(String, String)> {
    let mut lines = body.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let (mut name, mut description) = (None, None);
    for line in lines {
        if line.trim() == "---" {
            return Some((name?, description.unwrap_or_default()));
        }
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            description = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

/// Whether `uv` — which every `.brxt` needs to build its environment — is here.
///
/// A probe that TIMED OUT counts as available: it did not disprove `uv`, and
/// refusing the install would assert an absence nobody established. The install
/// itself will fail loudly and accurately if `uv` really is missing, which is a
/// better error than a confident refusal built on an unanswered probe.
pub fn uv_available() -> bool {
    crate::system::status_of("uv")
        .map(|d| d.installed || d.timed_out)
        .unwrap_or(false)
}

/// The message to refuse an install with when `uv` is missing.
pub fn uv_missing_message() -> String {
    let cmd = crate::system::install_command("uv").unwrap_or_default();
    format!(
        "`uv` is required to install .brxt extensions, but it was not found.\n  \
         Install it:  {cmd}\n  \
         Then re-run, or run `biorouter doctor` to check prerequisites."
    )
}

/// Build the bundle's Python environment.
pub fn run_uv_sync(dir: &Path) -> Result<()> {
    let mut command = Command::new("uv");
    command.arg("sync").current_dir(dir);
    biorouter_mcp::developer::shell::no_console_window_std(&mut command);
    let output = command.output();
    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let detail = String::from_utf8_lossy(&out.stderr);
            // uv puts the root cause and its `help:` dependency-chain line at
            // the END of stderr, so keep the tail, not the head.
            let lines: Vec<&str> = detail.trim().lines().collect();
            let tail = if lines.len() > 15 {
                format!("…\n{}", lines[lines.len() - 15..].join("\n"))
            } else {
                lines.join("\n")
            };
            let hint = uv_sync_hint(&detail)
                .map(|h| format!("\n\nhint: {h}"))
                .unwrap_or_default();
            bail!("uv sync failed:\n{}{}", tail, hint)
        }
        Err(e) => bail!("Could not run `uv sync`: {e}"),
    }
}

/// Map well-known `uv sync` failure signatures to an actionable hint appended
/// below the raw output. Checks run most-specific first.
fn uv_sync_hint(stderr: &str) -> Option<&'static str> {
    if stderr.contains("Symbol not found") && stderr.contains("librustc_driver") {
        // Homebrew's `rust` dynamically links `libLLVM.dylib`; when `llvm` is
        // upgraded the ABI mismatches and `rustc` aborts. `brew upgrade rust`
        // does NOT reliably fix this (there may be no rebuilt bottle yet), so
        // steer users to the self-contained rustup toolchain and tell them to
        // remove the Homebrew one so it wins on PATH.
        Some(
            "your Homebrew Rust toolchain is broken. `rustc` aborts because Homebrew's \
             `llvm` was upgraded out from under it (a known Homebrew issue). \
             `brew upgrade rust` usually does NOT fix this. Install the self-contained \
             rustup toolchain and remove the Homebrew one so it takes priority:\n    \
             brew uninstall rust\n    \
             curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh\n  \
             then fully restart Biorouter and retry.",
        )
    } else if stderr.contains("cryptography") && cryptography_built_from_source(stderr) {
        // cryptography ≥49 (2026-06-12) dropped x86_64 macOS wheels, so Intel
        // Macs must compile it (it is a Rust/maturin project) instead of
        // downloading a wheel.
        Some(
            "`cryptography` ≥49 no longer ships x86_64 (Intel) macOS wheels, so on an \
             Intel Mac it must be compiled from source, which needs a Rust toolchain. \
             Install rustup (https://rustup.rs) and retry, or ask the extension author \
             to cap `cryptography<49` (the last series with Intel-Mac wheels).",
        )
    } else if stderr.contains("maturin") || stderr.contains("rustc") {
        Some(
            "a dependency has no prebuilt package for your platform, so it was compiled \
             from source, which needs a working Rust toolchain. Install one via \
             https://rustup.rs (or repair your existing install) and retry.",
        )
    } else if stderr.contains("Failed to build") {
        Some(
            "a dependency has no prebuilt package for your platform, so uv tried to \
             compile it from source. Make sure a compiler toolchain is installed, or ask \
             the extension author to pin versions that ship prebuilt wheels.",
        )
    } else {
        None
    }
}

/// True when stderr indicates `cryptography` was being built from source
/// (rather than failing for some unrelated reason that merely mentions it).
fn cryptography_built_from_source(stderr: &str) -> bool {
    stderr.contains("Failed to build `cryptography")
        || stderr.contains("Building cryptography")
        || (stderr.contains("cryptography") && stderr.contains("maturin"))
}

/// `~/.config/biorouter/extensions/`.
pub fn extensions_root() -> PathBuf {
    crate::config::paths::Paths::config_dir().join("extensions")
}

/// Refuse a `manifest.name` that would not stay inside the extensions root.
///
/// ⚠ **The bundle names its own install directory** — every installer does
/// `extensions_root().join(&manifest.name)` — and the install transaction's
/// `rollback` does `remove_dir_all` on the result. A bundle declaring
/// `"name": "../../evil"` therefore picks an arbitrary directory to write into
/// and, on any failure, to delete.
///
/// Deliberately identical to `validate_extension_name` in the daemon's
/// `routes::shell`, and to the check the desktop's `brxt:uninstall` handler
/// performs: a bundle one reader accepts and another refuses is a bug report
/// nobody can reproduce.
pub fn validate_extension_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        bail!("Invalid extension name in manifest.json: {name:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::with_config_overrides;

    /// A structurally valid `.brxt` whose manifest declares `name`, written
    /// into `dir` and nowhere else.
    fn bundle_declaring(dir: &Path, name: &str) -> PathBuf {
        use std::io::Write as _;
        let path = dir.join("fixture.brxt");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::FileOptions::default();
        let manifest = serde_json::json!({
            "name": name,
            "display_name": "Fixture",
            "description": "a fixture bundle",
            "version": "0.0.1",
            "entry_point": "main.py",
            "repository": "https://example.test/fixture",
            "env_vars": [],
        });
        for (entry, body) in [
            ("manifest.json", serde_json::to_vec(&manifest).unwrap()),
            ("README.md", b"# Fixture\n".to_vec()),
            (
                "pyproject.toml",
                b"[project]\nname = \"fixture\"\n".to_vec(),
            ),
            ("src/main.py", b"pass\n".to_vec()),
        ] {
            zip.start_file(entry, options).unwrap();
            zip.write_all(&body).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    /// ⚠ **The bundle names its own install directory.** Every installer does
    /// `extensions_root().join(&manifest.name)`, and the install transaction's
    /// `rollback` does `remove_dir_all` on the result — so a bundle declaring
    /// `"name": "../evil"` picks a directory outside the extensions root to
    /// write into and, on any failure, to delete. Refusing at `open` is what
    /// makes every reader of a bundle agree.
    #[test]
    fn a_bundle_name_with_a_separator_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../evil", "..", ".", "a/b", "a\\b", "/etc/passwd"] {
            let path = bundle_declaring(dir.path(), bad);
            let Err(err) = BrxtBundle::open(&path) else {
                panic!("{bad} was accepted as an extension name");
            };
            assert!(
                err.to_string().contains("Invalid extension name"),
                "{bad} was refused for the wrong reason: {err}"
            );
        }
        let good = bundle_declaring(dir.path(), "spokeagent");
        assert_eq!(
            BrxtBundle::open(&good).unwrap().manifest().name,
            "spokeagent",
            "an ordinary name must still open"
        );
    }

    #[test]
    fn hint_broken_homebrew_rust() {
        let stderr = "dyld[28466]: Symbol not found: __ZN4llvm10PGOOptionsC1E...\n\
                      Referenced from: /usr/local/Cellar/rust/1.89.0_3/lib/librustc_driver-bccb51ff.dylib";
        let hint = uv_sync_hint(stderr).unwrap();
        // Must steer to rustup + removing Homebrew rust, since field-testing
        // showed `brew upgrade rust` does not fix this.
        assert!(hint.contains("rustup"));
        assert!(hint.contains("brew uninstall rust"));
        assert!(hint.contains("does NOT fix"));
    }

    #[test]
    fn hint_cryptography_intel_wheel_removed() {
        let stderr = "× Failed to build `cryptography==49.0.0`\n\
                      ├─▶ Call to `maturin.build_wheel` failed";
        let hint = uv_sync_hint(stderr).unwrap();
        assert!(hint.contains("cryptography<49"));
        assert!(hint.contains("Intel"));
    }

    #[test]
    fn hint_rust_toolchain_needed() {
        let stderr = "error: process didn't exit successfully: `rustc -vV`\n💥 maturin failed";
        assert!(uv_sync_hint(stderr).unwrap().contains("Rust toolchain"));
    }

    #[test]
    fn hint_generic_source_build() {
        let stderr = "× Failed to build `pymssql==2.3.13`\n├─▶ The build backend returned an error";
        assert!(uv_sync_hint(stderr)
            .unwrap()
            .contains("compile it from source"));
    }

    #[test]
    fn no_hint_for_unrelated_failure() {
        assert!(uv_sync_hint("No solution found when resolving dependencies").is_none());
    }

    #[test]
    fn frontmatter_is_read_out_of_a_skill_file() {
        let body = "---\nname: Genomics\ndescription: Variant calling\n---\n\n# body\n";
        assert_eq!(
            skill_frontmatter(body),
            Some(("Genomics".to_string(), "Variant calling".to_string()))
        );
    }

    #[test]
    fn a_file_without_frontmatter_contributes_no_skill() {
        assert_eq!(skill_frontmatter("# just a heading\n"), None);
    }

    /// A card is the *ask*. Pre-filling it would mean reading the value back out
    /// of the credential store first, which is the one direction this whole
    /// feature exists to prevent.
    #[test]
    fn a_key_request_carries_no_value_not_even_the_default() {
        let var = BrxtEnvVar {
            key: "SPOKEAGENT_PASSCODE".to_string(),
            required: true,
            auto_propagate: false,
            default: Some("hunter2".to_string()),
            description: "From the UCSF wiki".to_string(),
            secret: true,
        };
        let request = var.as_key_request();
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("SPOKEAGENT_PASSCODE"));
        assert!(
            !json.contains("hunter2"),
            "a default must never ride on the card: {json}"
        );
    }

    #[tokio::test]
    async fn only_secret_variables_are_satisfied_by_a_stored_same_named_value() {
        let stored = HashMap::from([(String::from("SHARED_NAME"), String::from("stored"))]);
        let secret = BrxtEnvVar {
            key: "SHARED_NAME".to_string(),
            required: true,
            auto_propagate: false,
            default: None,
            description: String::new(),
            secret: true,
        };
        let ordinary = BrxtEnvVar {
            secret: false,
            ..secret.clone()
        };

        with_config_overrides(stored, async {
            let secret_manifest = BrxtManifest {
                name: "secret-case".to_string(),
                display_name: "Secret case".to_string(),
                description: String::new(),
                version: "1".to_string(),
                entry_point: "main.py".to_string(),
                repository: "https://example.test/secret".to_string(),
                tools_count: None,
                env_vars: vec![secret],
            };
            assert!(secret_manifest
                .unmet_requirements(&HashMap::new())
                .is_empty());

            let ordinary_manifest = BrxtManifest {
                env_vars: vec![ordinary],
                ..secret_manifest
            };
            let unmet = ordinary_manifest.unmet_requirements(&HashMap::new());
            assert_eq!(unmet.len(), 1);
            assert_eq!(unmet[0].key, "SHARED_NAME");
        })
        .await;
    }
}
