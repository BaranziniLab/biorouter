//! On-disk artifact store for Agent Drafter.
//!
//! Each artifact is a directory under the store root containing a
//! `manifest.json` plus the artifact's own files (entry HTML, CSS, JS, …).
//! Layout mirrors the conventions used by `memory`/`knowledge`:
//! `~/.config/biorouter/agent_drafter/<id>/`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use super::manifest::{
    Capabilities, GuardrailsConfig, ModelSettings, Orchestration, ReliabilityConfig, SurfaceDecl,
    ThemeConfig,
};

/// Whether an artifact embeds live agent capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    /// A plain interactive artifact (HTML/CSS/JS), no agent.
    Static,
    /// An artifact wired to a Biorouter agent (ACP / MCP-App bridge).
    Agentic,
}

impl ArtifactKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "static" | "" => Some(Self::Static),
            "agentic" | "agent" => Some(Self::Agentic),
            _ => None,
        }
    }
}

/// The provider + model an app's agent should run on. When absent, the app
/// falls back to Biorouter's globally-configured provider/model.
// Note: not `Eq` — `settings` carries `f32` fields (temperature/top_p).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelSelection {
    /// Provider name (e.g. "xiaomi_mimo", "anthropic", "openai").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model name (e.g. "mimo-v2.6-flash", "claude-opus-4-8").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Provider-agnostic generation settings (temperature, reasoning effort, …).
    /// Consumed by the per-app model surface (Phase 4); `None` → provider defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<ModelSettings>,
}

impl ModelSelection {
    pub fn is_set(&self) -> bool {
        self.provider.is_some() || self.model.is_some()
    }
}

/// Per-app agent configuration (present for `Agentic` apps). Captures everything
/// that distinguishes one app's Biorouter backend from another: the system
/// prompt/persona, which model runs it, and which extensions / skills /
/// knowledge base the agent may use.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConfig {
    /// System prompt that defines the embedded agent's behavior.
    #[serde(default)]
    pub system_prompt: String,
    /// Optional greeting shown when the chat panel mounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub greeting: Option<String>,
    /// Legacy free-form tool names (kept for back-compat; prefer `extensions`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// Provider + model the app's agent should run on. Defaults to the global
    /// Biorouter model when unset (the GUI/CLI seeds this with the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSelection>,
    /// Builtin / platform extension names the app's agent should load
    /// (e.g. "developer", "computercontroller", "autovisualiser", "knowledge").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// Skill ids the app's agent should have available.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// Knowledge base id the app's agent should be scoped to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_base: Option<String>,
    /// Bound on the agent's tool-calling loop per user message (a guardrail and
    /// a workflow control, like the knowledge sub-agent's `max_steps`). When
    /// unset, the server applies a safe default cap. Higher values let
    /// workflow-style apps chain more tool calls autonomously.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,

    // ───── BRSDK (all default; absence = denied / off) ─────
    /// Deny-by-default capability grants: files / data / compute / vault /
    /// memory / tracing / lifecycle-events (Phases 3–7).
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Declarative content guardrails + goal harness + HITL approvals (Phase 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrails: Option<GuardrailsConfig>,
    /// Reliability knobs: tool timeouts, stop conditions, error→output, etc.
    /// (Phase 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliability: Option<ReliabilityConfig>,
    /// Multi-agent orchestration: sub-agents-as-tools, handoffs, workflows,
    /// lazy tools (Phase 6).
    #[serde(default)]
    pub orchestration: Orchestration,
    /// JSON-Schema contract the final answer must validate against (Phase 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_type: Option<serde_json::Value>,
    /// Durable, resumable per-app sessions (Phase 1). `None` is treated as ON
    /// (the recovery default); set `Some(false)` to restore ephemeral
    /// per-connection sessions. Use [`AgentConfig::durable_session`] to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durable_session: Option<bool>,
    /// Platform capabilities this app *wants* that may or may not exist here.
    ///
    /// This is the vocabulary the manifest was missing. An app whose spec calls
    /// for a ClinVar knowledge base, on a machine with no ClinVar knowledge base,
    /// previously had exactly one way to express that: invent
    /// `knowledge_base: "clinvar"` — a lie that armed KB tools scoped to nothing
    /// and failed on turn 1. Now the honest statement is representable:
    /// `requires: [{kind: knowledge_base, id: "clinvar", reason: "…"}]` with the
    /// id left unset. An unmet requirement is a lint **warning** and a runtime
    /// banner, never a fabricated config.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
    /// When this config is a WORKER PROFILE: how many seconds it gets to answer a
    /// `consult` before it is cancelled.
    ///
    /// `max_turns` bounds tool CALLS, not wall clock — a worker can sit inside a
    /// single slow tool indefinitely. The deadline used to be a compile-time
    /// constant with no configuration path at all. Clamped 5..=600 at use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consult_timeout_s: Option<u64>,
}

/// The kind of platform capability a [`Requirement`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    KnowledgeBase,
    Skill,
    Extension,
    DataSource,
}

impl RequirementKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::KnowledgeBase => "knowledge_base",
            Self::Skill => "skill",
            Self::Extension => "extension",
            Self::DataSource => "data_source",
        }
    }
}

/// A platform capability the app needs, and whether this install can provide it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Requirement {
    pub kind: RequirementKind,
    /// The id the app would use *if it existed here* (e.g. `"clinvar"`). This is
    /// a statement of need, not a configuration — nothing is armed from it.
    pub id: String,
    /// Why the app needs it. Shown to the user in the degraded-capability banner.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl AgentConfig {
    /// Whether this app's sessions are durable + resumable (default: yes).
    pub fn durable_session(&self) -> bool {
        self.durable_session.unwrap_or(true)
    }
}

/// Metadata describing a single artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub kind: ArtifactKind,
    /// Entry file rendered for previews/exports.
    pub entry: String,
    /// Server-managed. `#[serde(default)]` because a model composing a manifest
    /// from scratch has no way to know these and no business inventing them —
    /// requiring them made `update_app` fail with `missing field created_at`,
    /// which is the error that kicked off the manifest-rewrite guessing loop.
    /// `update_app` restores the real values from disk after parsing.
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentConfig>,
    /// Preferred preview width in CSS px (None → fill the panel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Preferred preview height in CSS px (None → a comfortable default that
    /// then auto-grows to fit content).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Unix seconds of the last successful esbuild bundle, if any. `None` means
    /// the app has never been built (or its sources changed since).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_at: Option<u64>,
    /// Fingerprint of the App SDK the current `dist/app.js` was bundled from
    /// (`bundle::sdk_fingerprint`). Apps vendor their own `src/sdk.ts`, so a
    /// bundle built before an SDK upgrade would silently ignore frames the
    /// server now sends. When this doesn't match, the daemon rebuilds the app on
    /// the next serve. `None` = built before this was tracked → rebuild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_hash: Option<String>,
    /// Id of the chat session this app was created in, when known. Lets the GUI
    /// reopen the originating conversation so the user can keep iterating on the
    /// app there. Apps created before this was recorded have `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The declared app contract (Apps SDK v2): typed state schema, actions,
    /// signals, and custom components. Absent/empty → a v1 app with no declared
    /// surface, which deserializes and re-serializes unchanged.
    #[serde(default, skip_serializing_if = "SurfaceDecl::is_empty")]
    pub surface: SurfaceDecl,
    /// The app's theme selection (Apps SDK v2, Pillar 6): a curated pack plus
    /// optional accent / custom token overrides. Absent/default → the base
    /// `biorouter` look, so a v1 manifest deserializes and re-serializes
    /// unchanged.
    #[serde(default, skip_serializing_if = "ThemeConfig::is_default")]
    pub theme: ThemeConfig,
}

impl Manifest {
    /// The theme pack this app actually renders with.
    ///
    /// Read this instead of `manifest.theme.pack`: the field is omitted from the
    /// serialized manifest when it holds the default, so its *absence* means
    /// "the default pack", never "no pack". An unknown pack on disk also resolves
    /// to the default here, matching what the renderer does.
    pub fn resolved_theme_pack(&self) -> &str {
        self.theme.resolved_pack()
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Turn a title into a filesystem-safe, URL-safe slug.
pub fn slugify(title: &str) -> String {
    const MAX_GENERATED_ID_BYTES: usize = 110;
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in title.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if out.len() >= MAX_GENERATED_ID_BYTES {
                break;
            }
            out.extend(ch.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() && out.len() < MAX_GENERATED_ID_BYTES {
            out.push('-');
            prev_dash = true;
        }
    }
    let slug = out.trim_matches('-').to_string();
    if slug.is_empty() {
        "artifact".to_string()
    } else {
        slug
    }
}

/// Reject paths that escape the artifact directory (absolute, `..`, or rooted).
fn safe_relative(path: &str) -> io::Result<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be relative to the artifact",
        ));
    }
    let mut clean = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Normal(c) => clean.push(c),
            Component::CurDir => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path may not contain '..' or root components",
                ))
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty path"));
    }
    Ok(clean)
}

/// App ids are both directory names and URL path segments. Keep the accepted
/// alphabet deliberately small so an id can never escape the store or become a
/// surprising route. Existing generated ids use the stricter lowercase + dash
/// subset; uppercase and underscores remain accepted for older installations.
pub fn validate_artifact_id(id: &str) -> io::Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "app id must be 1-128 ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

/// Refuse to traverse a symlink below the store root. Lexical `..` checks are
/// not enough when an app directory or one of its children is a symlink.
fn reject_store_symlinks(root: &Path, path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the app store root may not be a symlink",
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the app store root must be a directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path escapes the app store"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "app store paths may not contain symlinks",
                ));
            }
            Ok(metadata) if current == path && has_multiple_hard_links(&metadata) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "app store files may not have multiple hard links",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn has_multiple_hard_links(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    metadata.is_file() && metadata.nlink() > 1
}

#[cfg(not(unix))]
fn has_multiple_hard_links(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn write_atomically(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(content)?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

fn reject_tree_symlinks(dir: &Path) -> io::Result<()> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = match std::fs::read_dir(current) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "app store paths may not contain symlinks",
                ));
            }
            if has_multiple_hard_links(&entry.metadata()?) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "app store files may not have multiple hard links",
                ));
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

/// Filesystem-backed artifact store rooted at a single directory.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, id: &str) -> io::Result<PathBuf> {
        validate_artifact_id(id)?;
        let dir = self.root.join(id);
        reject_store_symlinks(&self.root, &dir)?;
        Ok(dir)
    }

    fn manifest_path(&self, id: &str) -> io::Result<PathBuf> {
        let path = self.dir(id)?.join("manifest.json");
        reject_store_symlinks(&self.root, &path)?;
        Ok(path)
    }

    pub fn exists(&self, id: &str) -> bool {
        self.manifest_path(id).is_ok_and(|path| path.is_file())
    }

    /// Atomically allocate a unique directory derived from the title.
    fn create_unique_dir(&self, title: &str) -> io::Result<(String, PathBuf)> {
        std::fs::create_dir_all(&self.root)?;
        reject_store_symlinks(&self.root, &self.root)?;
        let base = slugify(title);
        for n in 1..10_000 {
            let id = if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            let dir = self.dir(&id)?;
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok((id, dir)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique app id",
        ))
    }

    /// Create a new artifact directory with a manifest and the given files.
    pub fn create(
        &self,
        title: &str,
        description: &str,
        kind: ArtifactKind,
        entry: &str,
        files: &[(String, String)],
    ) -> io::Result<Manifest> {
        let (id, dir) = self.create_unique_dir(title)?;
        reject_store_symlinks(&self.root, &dir)?;
        let now = now_secs();
        let manifest = Manifest {
            id: id.clone(),
            title: title.to_string(),
            description: description.to_string(),
            kind,
            entry: entry.to_string(),
            created_at: now,
            updated_at: now,
            agent: if kind == ArtifactKind::Agentic {
                Some(AgentConfig::default())
            } else {
                None
            },
            width: None,
            height: None,
            built_at: None,
            sdk_hash: None,
            session_id: None,
            surface: SurfaceDecl::default(),
            theme: ThemeConfig::default(),
        };
        self.save_manifest(&manifest)?;
        for (path, content) in files {
            self.write_file(&id, path, content)?;
        }
        Ok(manifest)
    }

    /// Create an artifact at an **explicit** id (slugified). If one already
    /// exists at that id it is replaced, so re-authoring the same app is
    /// idempotent (no `-2` duplicates) and apps are stably addressable.
    pub fn create_with_id(
        &self,
        id: &str,
        title: &str,
        description: &str,
        kind: ArtifactKind,
        entry: &str,
        files: &[(String, String)],
    ) -> io::Result<Manifest> {
        let id = slugify(id);
        if self.dir(&id)?.exists() {
            self.delete(&id)?;
        }
        let dir = self.dir(&id)?;
        std::fs::create_dir_all(&dir)?;
        reject_store_symlinks(&self.root, &dir)?;
        let now = now_secs();
        let manifest = Manifest {
            id: id.clone(),
            title: title.to_string(),
            description: description.to_string(),
            kind,
            entry: entry.to_string(),
            created_at: now,
            updated_at: now,
            agent: if kind == ArtifactKind::Agentic {
                Some(AgentConfig::default())
            } else {
                None
            },
            width: None,
            height: None,
            built_at: None,
            sdk_hash: None,
            session_id: None,
            surface: SurfaceDecl::default(),
            theme: ThemeConfig::default(),
        };
        self.save_manifest(&manifest)?;
        for (path, content) in files {
            self.write_file(&id, path, content)?;
        }
        Ok(manifest)
    }

    pub fn save_manifest(&self, manifest: &Manifest) -> io::Result<()> {
        let dir = self.dir(&manifest.id)?;
        std::fs::create_dir_all(&dir)?;
        reject_store_symlinks(&self.root, &dir)?;
        let json = serde_json::to_string_pretty(manifest)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let manifest_path = self.manifest_path(&manifest.id)?;
        write_atomically(&manifest_path, json.as_bytes())
    }

    pub fn load_manifest(&self, id: &str) -> io::Result<Manifest> {
        let raw = std::fs::read_to_string(self.manifest_path(id)?)?;
        let manifest: Manifest = serde_json::from_str(&raw)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        validate_artifact_id(&manifest.id)?;
        if manifest.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "manifest id does not match its app directory",
            ));
        }
        Ok(manifest)
    }

    pub fn touch(&self, id: &str) -> io::Result<()> {
        let mut m = self.load_manifest(id)?;
        m.updated_at = now_secs();
        self.save_manifest(&m)
    }

    pub fn list(&self) -> Vec<Manifest> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let id = entry.file_name().to_string_lossy().to_string();
                    if let Ok(m) = self.load_manifest(&id) {
                        out.push(m);
                    }
                }
            }
        }
        out.sort_by_key(|manifest| std::cmp::Reverse(manifest.updated_at));
        out
    }

    pub fn write_file(&self, id: &str, path: &str, content: &str) -> io::Result<()> {
        let rel = safe_relative(path)?;
        let full = self.dir(id)?.join(&rel);
        reject_store_symlinks(&self.root, &full)?;
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        reject_store_symlinks(&self.root, &full)?;
        write_atomically(&full, content.as_bytes())
    }

    pub fn read_file(&self, id: &str, path: &str) -> io::Result<String> {
        let rel = safe_relative(path)?;
        let full = self.dir(id)?.join(rel);
        reject_store_symlinks(&self.root, &full)?;
        std::fs::read_to_string(full)
    }

    /// Read a file's raw bytes (for serving binary assets like images/fonts).
    pub fn read_bytes(&self, id: &str, path: &str) -> io::Result<Vec<u8>> {
        let rel = safe_relative(path)?;
        let full = self.dir(id)?.join(rel);
        reject_store_symlinks(&self.root, &full)?;
        std::fs::read(full)
    }

    /// Absolute path to an artifact's directory (used by the bundler/server).
    pub fn artifact_dir(&self, id: &str) -> io::Result<PathBuf> {
        let dir = self.dir(id)?;
        reject_tree_symlinks(&dir)?;
        Ok(dir)
    }

    /// Absolute path to a file within an artifact, path-traversal checked.
    pub fn file_path(&self, id: &str, path: &str) -> io::Result<PathBuf> {
        let rel = safe_relative(path)?;
        let full = self.dir(id)?.join(rel);
        reject_store_symlinks(&self.root, &full)?;
        Ok(full)
    }

    /// Whether a file exists within an artifact (path-traversal checked).
    pub fn file_exists(&self, id: &str, path: &str) -> bool {
        self.file_path(id, path).is_ok_and(|file| file.is_file())
    }

    pub fn delete(&self, id: &str) -> io::Result<()> {
        let dir = self.dir(id)?;
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, ArtifactStore) {
        let dir = tempdir().unwrap();
        let store = ArtifactStore::new(dir.path().to_path_buf());
        (dir, store)
    }

    #[test]
    fn slugify_handles_messy_titles() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  Trim   Me  "), "trim-me");
        assert_eq!(slugify("***"), "artifact");
        assert_eq!(slugify("Café Méta 2"), "caf-m-ta-2");
        assert_eq!(slugify(&"a".repeat(256)).len(), 110);
    }

    #[test]
    fn create_then_load_roundtrips() {
        let (_d, s) = store();
        let m = s
            .create(
                "My App",
                "does things",
                ArtifactKind::Static,
                "index.html",
                &[],
            )
            .unwrap();
        assert_eq!(m.id, "my-app");
        assert_eq!(m.kind, ArtifactKind::Static);
        assert!(m.agent.is_none());
        let loaded = s.load_manifest("my-app").unwrap();
        assert_eq!(loaded.title, "My App");
        assert_eq!(loaded.description, "does things");
    }

    #[test]
    fn artifact_ids_cannot_escape_the_store() {
        let (root, s) = store();
        let outside = root.path().with_extension("outside-app");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("manifest.json"), "do not touch").unwrap();

        for id in ["", ".", "..", "../outside-app", "a/b", "a\\b"] {
            assert!(!s.exists(id));
            assert!(s.load_manifest(id).is_err());
            assert!(s.write_file(id, "index.html", "owned").is_err());
            assert!(s.artifact_dir(id).is_err());
            assert!(s.delete(id).is_err());
        }

        assert_eq!(
            std::fs::read_to_string(outside.join("manifest.json")).unwrap(),
            "do not touch"
        );
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn manifest_id_must_match_its_directory() {
        let (_root, s) = store();
        s.create("Safe", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        let path = s.root().join("safe").join("manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        value["id"] = serde_json::json!("different");
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();

        let error = s.load_manifest("safe").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_an_app_are_rejected() {
        use std::os::unix::fs::symlink;

        let (root, s) = store();
        s.create("Safe", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        let outside = root.path().join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();
        symlink(&outside, s.root().join("safe").join("linked.txt")).unwrap();

        assert!(s.read_file("safe", "linked.txt").is_err());
        assert!(s.write_file("safe", "linked.txt", "overwrite").is_err());
        assert!(s.artifact_dir("safe").is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "secret");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_app_directories_are_rejected() {
        use std::os::unix::fs::symlink;

        let (root, s) = store();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("manifest.json"), "outside").unwrap();
        symlink(outside.path(), root.path().join("linked-app")).unwrap();

        assert!(!s.exists("linked-app"));
        assert!(s.load_manifest("linked-app").is_err());
        assert!(s
            .write_file("linked-app", "index.html", "overwrite")
            .is_err());
        assert!(s.delete("linked-app").is_err());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("manifest.json")).unwrap(),
            "outside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_store_roots_are_rejected() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = base.path().join("store");
        symlink(outside.path(), &root).unwrap();
        let store = ArtifactStore::new(root);

        assert!(store
            .create("Unsafe", "", ArtifactKind::Static, "index.html", &[])
            .is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_files_are_rejected() {
        let (_root, s) = store();
        s.create("Safe", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        let outside = s.root().join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();
        std::fs::hard_link(&outside, s.root().join("safe").join("linked.txt")).unwrap();

        assert!(s.read_file("safe", "linked.txt").is_err());
        assert!(s.write_file("safe", "linked.txt", "overwrite").is_err());
        assert!(s.artifact_dir("safe").is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "secret");
    }

    #[test]
    fn pre_brsdk_manifest_deserializes_with_defaults() {
        // A manifest written before BRSDK existed (no capabilities/guardrails/
        // reliability/orchestration/output_type/durable_session) must still load.
        let legacy = r#"{
            "id": "legacy-app",
            "title": "Legacy App",
            "description": "made before BRSDK",
            "kind": "agentic",
            "entry": "index.html",
            "created_at": 100,
            "updated_at": 200,
            "agent": {
                "system_prompt": "be helpful",
                "model": { "provider": "anthropic", "model": "claude-opus-4-8" },
                "extensions": ["developer"],
                "max_turns": 24
            }
        }"#;
        let m: Manifest = serde_json::from_str(legacy).expect("legacy manifest must load");
        let agent = m.agent.expect("agent present");
        // New fields fall back to deny-by-default / off.
        assert!(agent.capabilities.files.is_none());
        assert!(agent.capabilities.data.is_none());
        assert!(agent.capabilities.compute.is_none());
        assert_eq!(
            agent.capabilities.memory.mode,
            crate::agent_drafter::manifest::MemoryMode::Off
        );
        assert!(!agent.capabilities.tracing.enabled);
        assert!(
            agent.capabilities.tracing.redact,
            "tracing redaction defaults ON"
        );
        assert!(agent.guardrails.is_none());
        assert!(agent.reliability.is_none());
        assert!(agent.output_type.is_none());
        assert!(agent.orchestration.sub_agents.is_empty());
        // durable_session: absent → treated as ON (the recovery default).
        assert!(agent.durable_session(), "durable sessions default ON");
        // Everything that reaches outside the page stays deny-by-default…
        for denied in ["files", "data", "compute", "vault", "memory", "tracing"] {
            assert!(
                !agent
                    .capabilities
                    .advertised()
                    .contains(&denied.to_string()),
                "{denied} must stay denied for a legacy manifest"
            );
        }
        // …but UI control is confined to the app's own page, so a pre-BRSDK app
        // picks it up on the next connect rather than staying a chat box.
        assert!(agent.capabilities.ui.enabled, "ui control defaults ON");
        assert_eq!(agent.capabilities.advertised(), vec!["ui".to_string()]);
    }

    #[test]
    fn ui_capability_can_be_switched_off_per_app() {
        let json = r#"{
            "id": "a", "title": "A", "description": "", "kind": "agentic",
            "entry": "index.html", "created_at": 0, "updated_at": 0,
            "agent": { "system_prompt": "p", "capabilities": { "ui": { "enabled": false } } }
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        let agent = m.agent.unwrap();
        assert!(!agent.capabilities.ui.enabled);
        assert!(agent.capabilities.advertised().is_empty());
    }

    #[test]
    fn theme_persists_and_defaults_are_omitted() {
        use crate::agent_drafter::manifest::ThemeConfig;
        let (_d, s) = store();
        let mut m = s
            .create("Themed", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        // A default theme is not serialized, so a v1 manifest stays clean.
        assert!(m.theme.is_default());
        let json = std::fs::read_to_string(s.root().join("themed").join("manifest.json")).unwrap();
        assert!(
            !json.contains("\"theme\""),
            "default theme must be omitted: {json}"
        );

        // A customised theme survives save → load.
        let mut tokens = std::collections::HashMap::new();
        tokens.insert("--br-radius".to_string(), "4px".to_string());
        m.theme = ThemeConfig {
            pack: "terminal".into(),
            accent: Some("#3ddc84".into()),
            tokens,
        };
        s.save_manifest(&m).unwrap();
        let loaded = s.load_manifest("themed").unwrap();
        assert_eq!(loaded.theme.pack, "terminal");
        assert_eq!(loaded.theme.accent.as_deref(), Some("#3ddc84"));
        assert_eq!(
            loaded.theme.tokens.get("--br-radius").map(String::as_str),
            Some("4px")
        );

        // A legacy manifest with no theme block loads with the base pack.
        let legacy: Manifest = serde_json::from_str(
            r#"{"id":"x","title":"X","description":"","kind":"static",
                "entry":"index.html","created_at":0,"updated_at":0}"#,
        )
        .unwrap();
        assert!(legacy.theme.is_default());
        assert_eq!(legacy.theme.resolved_pack(), "biorouter");
    }

    #[test]
    fn ui_capability_sub_switches_default_on_and_are_individually_revocable() {
        let json = r#"{
            "id": "a", "title": "A", "description": "", "kind": "agentic",
            "entry": "index.html", "created_at": 0, "updated_at": 0,
            "agent": { "system_prompt": "p", "capabilities": { "ui": { "allow_theme": false } } }
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        let ui = m.agent.unwrap().capabilities.ui;
        assert!(ui.enabled, "omitted `enabled` still defaults on");
        assert!(!ui.allow_theme);
        assert!(ui.allow_layout && ui.allow_ask, "other switches unaffected");
    }

    #[test]
    fn full_brsdk_manifest_roundtrips_and_advertises() {
        use crate::agent_drafter::manifest::*;
        let caps = Capabilities {
            files: Some(FilesCapability::default()),
            compute: Some(ComputeCapability {
                sandbox: "docker".into(),
                timeout_s: 120,
                network: "none".into(),
                max_mem: Some("1g".into()),
                cpus: Some(2.0),
                image: None,
            }),
            memory: MemoryCapability {
                kb: Some("lab".into()),
                mode: MemoryMode::ReadWrite,
                shared_kb: None,
                distill: true,
            },
            tracing: TracingCapability {
                enabled: true,
                redact: true,
                processor: None,
            },
            events: vec!["tool".into(), "llm".into()],
            ..Default::default()
        };

        let agent = AgentConfig {
            system_prompt: "clinical assistant".into(),
            model: Some(ModelSelection {
                provider: Some("anthropic".into()),
                model: Some("claude-opus-4-8".into()),
                settings: Some(ModelSettings {
                    temperature: Some(0.2),
                    reasoning_effort: Some("high".into()),
                    ..Default::default()
                }),
            }),
            capabilities: caps,
            guardrails: Some(GuardrailsConfig {
                goal: Some("cite the KB".into()),
                pii: PiiMode::Mask,
                needs_approval: vec!["send_email".into()],
                ..Default::default()
            }),
            reliability: Some(ReliabilityConfig {
                tool_timeout_s: Some(30),
                parallel_tools: true,
                ..Default::default()
            }),
            output_type: Some(serde_json::json!({"type":"object"})),
            durable_session: Some(true),
            ..Default::default()
        };

        // Round-trips through JSON without loss.
        let json = serde_json::to_string(&agent).unwrap();
        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.guardrails.as_ref().unwrap().pii, PiiMode::Mask);
        // The goal one-liner (consumed by configure_agent → set_goal) round-trips.
        assert_eq!(
            back.guardrails.as_ref().unwrap().goal.as_deref(),
            Some("cite the KB")
        );
        assert_eq!(
            back.guardrails.as_ref().unwrap().needs_approval,
            vec!["send_email"]
        );
        assert_eq!(
            back.model
                .as_ref()
                .unwrap()
                .settings
                .as_ref()
                .unwrap()
                .temperature,
            Some(0.2)
        );
        assert_eq!(
            back.capabilities.compute.as_ref().unwrap().sandbox,
            "docker"
        );
        assert!(back.reliability.as_ref().unwrap().parallel_tools);

        // Advertises exactly the granted capabilities (deny-by-default elsewhere).
        let adv = back.capabilities.advertised();
        assert!(adv.contains(&"files".to_string()));
        assert!(adv.contains(&"compute".to_string()));
        assert!(adv.contains(&"memory".to_string()));
        assert!(adv.contains(&"tracing".to_string()));
        assert!(adv.contains(&"event:tool".to_string()));
        assert!(
            !adv.contains(&"data".to_string()),
            "data not granted → not advertised"
        );
    }

    #[test]
    fn agentic_artifact_gets_agent_config() {
        let (_d, s) = store();
        let m = s
            .create("Bot", "", ArtifactKind::Agentic, "index.html", &[])
            .unwrap();
        assert!(m.agent.is_some());
    }

    #[test]
    fn ids_are_unique() {
        let (_d, s) = store();
        let a = s
            .create("Same", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        let b = s
            .create("Same", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(a.id, "same");
        assert_eq!(b.id, "same-2");
    }

    #[test]
    fn concurrent_creates_allocate_distinct_ids() {
        use std::collections::HashSet;
        use std::sync::{Arc, Barrier};

        let (_root, store) = store();
        let store = Arc::new(store);
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .create("Same", "", ArtifactKind::Static, "index.html", &[])
                        .unwrap()
                        .id
                })
            })
            .collect::<Vec<_>>();
        let ids = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), 8);
        assert!(ids.contains("same"));
    }

    #[test]
    fn write_and_read_files() {
        let (_d, s) = store();
        s.create(
            "Files",
            "",
            ArtifactKind::Static,
            "index.html",
            &[
                ("index.html".to_string(), "<h1>hi</h1>".to_string()),
                ("css/app.css".to_string(), "body{}".to_string()),
            ],
        )
        .unwrap();
        assert_eq!(s.read_file("files", "index.html").unwrap(), "<h1>hi</h1>");
        assert_eq!(s.read_file("files", "css/app.css").unwrap(), "body{}");
    }

    #[test]
    fn rejects_path_traversal() {
        let (_d, s) = store();
        s.create("Safe", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        assert!(s.write_file("safe", "../escape.txt", "x").is_err());
        assert!(s.write_file("safe", "/etc/passwd", "x").is_err());
        assert!(s.write_file("safe", "a/../../b.txt", "x").is_err());
        assert!(s.write_file("safe", "", "x").is_err());
    }

    #[test]
    fn list_sorts_by_updated_desc() {
        let (_d, s) = store();
        let a = s
            .create("Alpha", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let b = s
            .create("Beta", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        let list = s.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id);
        assert_eq!(list[1].id, a.id);
    }

    #[test]
    fn delete_removes_artifact() {
        let (_d, s) = store();
        s.create("Gone", "", ArtifactKind::Static, "index.html", &[])
            .unwrap();
        assert!(s.exists("gone"));
        s.delete("gone").unwrap();
        assert!(!s.exists("gone"));
    }
}
