//! J-P03: spawn presets for the `subagent` tool.
//!
//! A preset fixes what a child may use for its whole life, and only ever takes
//! away: it runs after `subagent_tool::judge_child` has narrowed and filtered
//! the child's extensions, so it can never give a child more than R4, DR-19
//! and DR-31 admitted, and it never touches the tier (the classification is
//! stamped before it, from the child's provider).
//!
//! * [`SpawnPreset::NoTools`]: a reviewer with no extension and no tool. It
//!   reads only what its instructions carry.
//! * [`SpawnPreset::OnlyExtensions`]: exactly the extensions the spawn names
//!   from its chat's live roster, with the tools they had at spawn, and none of
//!   Biorouter's own `platform__*` tools.
//! * [`SpawnPreset::ReadOnly`]: a verifier holding only tools that read:
//!   the acting extensions ([`ACTING_EXTENSION_KEYS`]) go whole, every other
//!   tool must be annotated read-only
//!   ([`crate::permission::tool_risk::grade_tool`]), and Developer's
//!   `text_editor` stays for `view` alone. Its parent records the working
//!   folder before and after the run ([`FolderSnapshot`]).
//!
//! The decision is one value, [`ToolClamp`], computed from the tools the child
//! was really offered once its extensions loaded, and held in ONE place: the
//! child's `ExtensionManager` (`ExtensionManager::seal`), set once and never
//! replaced. Holding it there seals the manager (nothing can be added later)
//! and makes the manager's own dispatch, the choke point a Code Execution
//! script, `POST /agent/call_tool` and the coding-agent bridge all pass,
//! refuse every tool outside it. The agent asks the same value for the model's
//! tool list, for its own `platform__*` tools at dispatch, for
//! `Agent::add_extension` and for the bridge plan. It is stored in the child's
//! runtime profile too, so a child continued after an eviction is held the
//! same way.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rmcp::model::{JsonObject, Tool};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::agents::extension::ExtensionConfig;
use crate::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME;
use crate::agents::platform_tools::{
    PLATFORM_EXTENSION_NAME, PLATFORM_READ_SESSION_BLOB_TOOL_NAME,
};

/// The three presets, by the names the spawn tool takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnPreset {
    NoTools,
    OnlyExtensions,
    ReadOnly,
}

impl SpawnPreset {
    /// Every preset's name, in the order the tool schema lists them.
    pub const NAMES: [&'static str; 3] = ["no_tools", "only_extensions", "read_only"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoTools => "no_tools",
            Self::OnlyExtensions => "only_extensions",
            Self::ReadOnly => "read_only",
        }
    }

    /// The paragraph the child's system prompt carries about its tools.
    pub fn child_note(self) -> &'static str {
        match self {
            Self::NoTools => {
                "You were started with no tools (the no_tools preset). You cannot read files, \
                 search or run anything: work only from your task instructions. If they do not \
                 contain what you need, say what is missing in your final message."
            }
            Self::OnlyExtensions => {
                "You were started with only the extensions your parent named (the \
                 only_extensions preset). The tools listed below are all you have, and none can \
                 be added while you run."
            }
            Self::ReadOnly => {
                "You were started as a read-only verifier (the read_only preset). You hold only \
                 tools marked read-only: nothing can write a file, run a command or act on \
                 anything, and nothing can be added while you run. Do not try to change \
                 anything; report what you find. Your parent compares the working folder before \
                 and after your run."
            }
        }
    }

    /// The sentence the parent's spawn result carries.
    pub fn parent_note(self) -> &'static str {
        match self {
            Self::NoTools => {
                "Preset no_tools: this subagent has no tools at all and works only from the \
                 instructions you gave it. No tool can be added to it later."
            }
            Self::OnlyExtensions => {
                "Preset only_extensions: this subagent holds only the extensions you named, \
                 with the tools they had when it started. Nothing can be added to it later."
            }
            Self::ReadOnly => {
                "Preset read_only: this subagent keeps only tools marked read-only (Developer's \
                 text_editor for view alone); read-only means it changes nothing, not that it \
                 stays on this computer, so a reader may still ask a decision model or read \
                 a remote source. Nothing that writes, runs commands or acts can be added to it \
                 later. Its result says whether any file in the working folder changed while \
                 it ran."
            }
        }
    }
}

/// Extensions a read-only verifier never holds, whatever their tools'
/// annotations say, because even their reading tools act or reach past the
/// working folder: Code Execution (a script reaches every tool in the
/// catalogue), Biorouter Copilot (observing the desktop needs its own
/// consent), the Biorouter Browser (logged-in pages), app control, Crew and
/// Workspace. By manager key.
pub const ACTING_EXTENSION_KEYS: &[&str] = &[
    "code_execution",
    "computercontroller",
    "browser",
    "appcontrol",
    "crew",
    "workspace",
];

/// The manager key an extension is stored under, which is also the prefix of
/// its tools (`<key>__<tool>`).
pub(crate) fn extension_key(config: &ExtensionConfig) -> String {
    crate::agents::extension_manager::normalize(&config.key())
}

/// Whether a read-only verifier loses `config` whole.
pub(crate) fn is_acting_extension(config: &ExtensionConfig) -> bool {
    ACTING_EXTENSION_KEYS.contains(&extension_key(config).as_str())
}

/// Whether `key` is a tool-name prefix the agent serves itself: `platform`
/// (`platform__*`) and the structured-output tool's (`workflow__final_output`).
/// An extension stored under one of these keys would collide with the agent's
/// own tools, which dispatch answers by name before any extension, so no tool
/// is ever granted to a preset child for belonging to such an extension.
fn is_agent_owned_key(key: &str) -> bool {
    key == PLATFORM_EXTENSION_NAME
        || FINAL_OUTPUT_TOOL_NAME
            .split_once("__")
            .is_some_and(|(prefix, _)| prefix == key)
}

/// Whether `tool` is named in a namespace the agent serves itself.
pub(crate) fn is_agent_owned_tool(tool: &str) -> bool {
    tool.split_once("__")
        .is_some_and(|(prefix, _)| is_agent_owned_key(prefix))
}

/// Whether `config` is Biorouter's own Developer, whose `text_editor` a
/// verifier keeps for `view` alone.
fn is_biorouter_developer(config: &ExtensionConfig) -> bool {
    crate::agents::extension_manager::resolve_bundled_extension("developer")
        .is_some_and(|target| target.matches_config(config))
}

/// What an attempt to add an extension to a preset child is told.
pub const SEALED_REFUSAL: &str = "This subagent was started with a tool preset, which fixed its \
     tools when it started: no extension or tool can be added to it.";

impl crate::agents::Agent {
    /// Fix this agent's tools to `clamp`, which also seals its extension
    /// manager. Set once: installing the same clamp again (a restore on a live
    /// agent) is a no-op, and a different one is refused, so a preset can
    /// never be replaced or widened.
    pub(crate) fn install_spawn_tool_clamp(&self, clamp: ToolClamp) -> anyhow::Result<()> {
        clamp.validate()?;
        self.extension_manager.seal(clamp)
    }

    /// The preset child's fixed tools, or `None` for every other agent.
    pub(crate) fn spawn_tool_clamp(&self) -> Option<&Arc<ToolClamp>> {
        self.extension_manager.spawn_tool_clamp()
    }
}

/// The one command of a view-only tool a read-only verifier may call.
const VIEW_COMMAND: &str = "view";

/// The fixed set of tools a preset child may use, by prefixed name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolClamp {
    preset: SpawnPreset,
    /// Every tool the child may call, as the model sees its name.
    tools: BTreeSet<String>,
    /// Of those, the ones it may call only with `command: "view"`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    view_only: BTreeSet<String>,
}

impl ToolClamp {
    /// The clamp for `preset`, from the tools the child was offered once its
    /// extensions loaded (`offered`) and the extensions that loaded (`loaded`).
    /// A tool belongs to the loaded extension whose key is the longest prefix
    /// of its name, the rule the extension manager routes a call by, except a
    /// tool in a namespace the agent serves itself ([`is_agent_owned_tool`]),
    /// which belongs to no extension.
    pub fn for_preset(preset: SpawnPreset, offered: &[Tool], loaded: &[ExtensionConfig]) -> Self {
        let mut tools = BTreeSet::new();
        let mut view_only = BTreeSet::new();
        for tool in offered {
            let name = tool.name.as_ref();
            // The structured-output tool is how a sub-workflow with a response
            // schema reports, not a capability: every preset keeps it.
            if name == FINAL_OUTPUT_TOOL_NAME {
                tools.insert(name.to_string());
                continue;
            }
            let owner = owner_of(name, loaded);
            match preset {
                SpawnPreset::NoTools => {}
                SpawnPreset::OnlyExtensions => {
                    if owner.is_some() || name == PLATFORM_READ_SESSION_BLOB_TOOL_NAME {
                        tools.insert(name.to_string());
                    }
                }
                SpawnPreset::ReadOnly => {
                    // Reads back the child's own earlier results that were too
                    // large to keep inline; it reaches nothing else.
                    if name == PLATFORM_READ_SESSION_BLOB_TOOL_NAME {
                        tools.insert(name.to_string());
                        continue;
                    }
                    let Some(owner) = owner else { continue };
                    if is_acting_extension(owner) {
                        continue;
                    }
                    if is_biorouter_developer(owner)
                        && name == format!("{}__text_editor", extension_key(owner))
                    {
                        tools.insert(name.to_string());
                        view_only.insert(name.to_string());
                        continue;
                    }
                    if crate::permission::tool_risk::grade_tool(tool)
                        == crate::permission::tool_risk::ToolRisk::Low
                    {
                        tools.insert(name.to_string());
                    }
                }
            }
        }
        Self {
            preset,
            tools,
            view_only,
        }
    }

    pub fn preset(&self) -> SpawnPreset {
        self.preset
    }

    /// Whether the model may be offered `name`.
    pub fn offers(&self, name: &str) -> bool {
        self.tools.contains(name)
    }

    /// Whether any tool of the extension stored under `key` survived. Never
    /// for a key the agent's own tools are named under.
    pub fn keeps_extension(&self, key: &str) -> bool {
        if is_agent_owned_key(key) {
            return false;
        }
        let prefix = format!("{key}__");
        self.tools.iter().any(|tool| tool.starts_with(&prefix))
    }

    /// Why a call is refused, or `None` when the clamp admits it.
    pub fn refusal(&self, name: &str, arguments: Option<&JsonObject>) -> Option<String> {
        let preset = self.preset.as_str();
        if !self.tools.contains(name) {
            return Some(format!(
                "`{name}` is not available to this subagent: it was started with the {preset} \
                 preset, which fixed its tools when it started, and none can be added. Do not \
                 retry; do the task with the tools you have, or say in your final message what \
                 you could not do."
            ));
        }
        if self.view_only.contains(name) {
            let command = arguments
                .and_then(|arguments| arguments.get("command"))
                .and_then(serde_json::Value::as_str);
            if command != Some(VIEW_COMMAND) {
                return Some(format!(
                    "This subagent was started with the {preset} preset, so `{name}` may only \
                     view a file; it cannot write, edit, insert or undo. Do not retry; report \
                     what you found."
                ));
            }
        }
        None
    }

    /// [`Self::refusal`] for a call routed to an extension
    /// (`ExtensionManager::dispatch_tool_call`). A name in a namespace the
    /// agent serves itself is never an extension's to answer, so it is refused
    /// there even when the clamp grants the agent's own tool of that name.
    pub fn refusal_for_extension_call(
        &self,
        name: &str,
        arguments: Option<&JsonObject>,
    ) -> Option<String> {
        if is_agent_owned_tool(name) {
            return Some(format!(
                "`{name}` is one of Biorouter's own tool names, which no extension of this \
                 subagent may answer: it was started with the {} preset.",
                self.preset.as_str()
            ));
        }
        self.refusal(name, arguments)
    }

    /// Refuse a stored clamp that could not have been written by
    /// [`Self::for_preset`].
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.tools.iter().any(|tool| tool.trim().is_empty()) {
            anyhow::bail!("subagent tool clamp names an empty tool");
        }
        if !self.view_only.is_subset(&self.tools) {
            anyhow::bail!("subagent tool clamp limits a tool it does not grant");
        }
        if self.tools.iter().any(|tool| {
            is_agent_owned_tool(tool)
                && tool != FINAL_OUTPUT_TOOL_NAME
                && tool != PLATFORM_READ_SESSION_BLOB_TOOL_NAME
        }) {
            anyhow::bail!("subagent tool clamp grants one of Biorouter's own acting tools");
        }
        if self.preset == SpawnPreset::NoTools
            && self.tools.iter().any(|tool| tool != FINAL_OUTPUT_TOOL_NAME)
        {
            anyhow::bail!("a no_tools subagent's clamp grants a tool");
        }
        Ok(())
    }
}

/// The loaded extension whose key is the longest prefix of `tool`, never for
/// a tool in a namespace the agent serves itself.
fn owner_of<'a>(tool: &str, loaded: &'a [ExtensionConfig]) -> Option<&'a ExtensionConfig> {
    if is_agent_owned_tool(tool) {
        return None;
    }
    loaded
        .iter()
        .filter(|config| {
            let key = extension_key(config);
            !is_agent_owned_key(&key) && tool.starts_with(&format!("{key}__"))
        })
        .max_by_key(|config| extension_key(config).len())
}

// ─── The read-only verifier's folder check ──────────────────────────────────
//
// The walk follows the rules `workspace_summary` learned from a measured hang
// (a `read_dir` under `~/Library/Group Containers` that never returned, and
// that `Stop` could not end): no filesystem call on the async path, a refused
// home folder, filesystem root and opaque tree, the same opaque trees pruned
// during the walk, no crossing of a mount point, a finite budget raced
// against the run's cancellation token, and at most one presumed-wedged walk
// per folder at a time. Where it cannot compare something it says so, and it
// never says "no file changed" about files it did not compare.

/// At most this many files and links are recorded.
pub const FOLDER_CHECK_MAX_FILES: usize = 20_000;
/// At most this many entries (files, links and folders) are visited, so a tree
/// of empty folders cannot make the walk unbounded.
pub const FOLDER_CHECK_MAX_ENTRIES: usize = 100_000;
/// A file larger than this is compared by size and modification time.
pub const FOLDER_CHECK_MAX_HASHED_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Once this many bytes are hashed, the rest are compared by size and time.
pub const FOLDER_CHECK_MAX_TOTAL_HASHED_BYTES: u64 = 256 * 1024 * 1024;
/// How long one walk may take before it stops and says what it did not reach.
pub const FOLDER_CHECK_BUDGET: Duration = Duration::from_secs(10);
/// How much longer than [`FOLDER_CHECK_BUDGET`] the run waits for a walk
/// before it gives up on it: a walk still running then is presumed blocked in
/// a read that does not return.
const FOLDER_CHECK_GRACE: Duration = Duration::from_secs(5);
/// How many paths of each kind the parent is told by name.
pub const FOLDER_CHECK_LISTED: usize = 20;
/// Dependency, cache and environment folders the check leaves out wherever
/// they are, because they are large and other programs rewrite them. A folder
/// holding a `CACHEDIR.TAG` (Cargo's `target`, among others) is left out too.
pub const FOLDER_CHECK_LEFT_OUT_NAMES: &[&str] = &[
    "node_modules",
    "bower_components",
    "__pycache__",
    ".venv",
    ".tox",
    ".nox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".gradle",
    ".next",
    ".nuxt",
];
/// The Cache Directory Tagging marker.
const CACHE_DIR_TAG: &str = "CACHEDIR.TAG";
/// The only entries of a `.git` folder the check reads.
const GIT_ENTRIES_CHECKED: &[&str] = &["HEAD", "config", "hooks"];
/// How many unreadable paths the parent is told by name.
const UNREADABLE_LISTED: usize = 5;

/// Bounds for one walk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FolderLimits {
    pub(crate) max_files: usize,
    pub(crate) max_entries: usize,
    pub(crate) budget: Duration,
}

impl Default for FolderLimits {
    fn default() -> Self {
        Self {
            max_files: FOLDER_CHECK_MAX_FILES,
            max_entries: FOLDER_CHECK_MAX_ENTRIES,
            budget: FOLDER_CHECK_BUDGET,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Stamp {
    File {
        len: u64,
        modified: Option<SystemTime>,
        hash: Option<[u8; 32]>,
    },
    Link(PathBuf),
}

impl Stamp {
    /// Equal content: by hash where both sides have one, else by size and
    /// modification time, so the hashing budget can never read as a change.
    fn same_as(&self, other: &Stamp) -> bool {
        match (self, other) {
            (
                Stamp::File {
                    len: a_len,
                    modified: a_time,
                    hash: a_hash,
                },
                Stamp::File {
                    len: b_len,
                    modified: b_time,
                    hash: b_hash,
                },
            ) => match (a_hash, b_hash) {
                (Some(a), Some(b)) => a_len == b_len && a == b,
                _ => a_len == b_len && a_time == b_time,
            },
            (Stamp::Link(a), Stamp::Link(b)) => a == b,
            _ => false,
        }
    }
}

/// Why a walk stopped before the end of the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stop {
    Files(usize),
    Entries(usize),
    Time(Duration),
    Cancelled,
}

impl Stop {
    /// What the stop left out, as a clause.
    fn describe(&self) -> String {
        match self {
            Stop::Files(max) => {
                format!("files past the first {max} were not compared (the check stops there)")
            }
            Stop::Entries(max) => {
                format!("entries past the first {max} were not compared (the check stops there)")
            }
            Stop::Time(budget) => format!(
                "reading the folder took longer than {} seconds, so the rest was not compared",
                budget.as_secs()
            ),
            Stop::Cancelled => "the check was stopped, so the rest was not compared".to_string(),
        }
    }
}

/// A path inside the walked folder, by components. Compared as the operating
/// system's own names, which is the order the walk visits them in.
type Key = Vec<OsString>;

fn path_of(key: &Key) -> String {
    key.iter()
        .map(|component| component.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The working folder as it stood at one moment: each regular file by size
/// and BLAKE3 hash, each symbolic link by its target. Links are never
/// followed and mount points are never crossed. Left out: what `.gitignore`,
/// `.ignore` and `.biorouterignore` (local and global) exclude, `.git` apart
/// from `HEAD`, `config` and `hooks`, dependency and cache folders
/// ([`FOLDER_CHECK_LEFT_OUT_NAMES`], `CACHEDIR.TAG`), opaque system and
/// cloud-sync trees, Biorouter's oversized-result folder and Biorouter's own
/// data folders, because Biorouter writes there itself.
#[derive(Debug, Clone)]
pub struct FolderSnapshot {
    root: PathBuf,
    entries: BTreeMap<Key, Stamp>,
    /// Folders (or files) whose contents could not be read.
    unreadable: BTreeSet<Key>,
    /// Errors that named no path; the check is then incomplete.
    pathless_errors: usize,
    /// The last entry the walk finished with, in walk order.
    last_visited: Option<Key>,
    stopped: Option<Stop>,
    /// Names of the dependency and cache folders the walk left out.
    left_out: BTreeSet<String>,
    /// Why the folder was not walked at all.
    not_checked: Option<String>,
}

impl FolderSnapshot {
    fn empty(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            entries: BTreeMap::new(),
            unreadable: BTreeSet::new(),
            pathless_errors: 0,
            last_visited: None,
            stopped: None,
            left_out: BTreeSet::new(),
            not_checked: None,
        }
    }

    /// A snapshot of a folder that was not walked, and why.
    pub fn not_checked(root: &Path, why: impl Into<String>) -> Self {
        Self {
            not_checked: Some(why.into()),
            ..Self::empty(root)
        }
    }

    /// Record `root` with the default limits, skipping the folders in `skip`
    /// (absolute) as well as the ones this module always skips. Blocking.
    pub fn take(root: &Path, skip: &[PathBuf]) -> Self {
        let home = crate::config::paths::Paths::home_dir();
        Self::take_with(
            root,
            skip,
            home.as_deref(),
            crate::agents::workspace_summary::global_biorouterignore().as_deref(),
            FolderLimits::default(),
            &AtomicBool::new(false),
        )
    }

    /// [`Self::take`] with every input injected. Blocking.
    pub(crate) fn take_with(
        root: &Path,
        skip: &[PathBuf],
        home: Option<&Path>,
        global_ignore: Option<&Path>,
        limits: FolderLimits,
        stop: &AtomicBool,
    ) -> Self {
        let deadline = Instant::now() + limits.budget;
        let mut snapshot = Self::empty(root);
        let canonical_root = match std::fs::canonicalize(root) {
            Ok(path) => path,
            Err(error) => return Self::not_checked(root, format!("it could not be read: {error}")),
        };
        if let Some(reason) =
            crate::agents::workspace_summary::skip_reason_with_home(&canonical_root, home)
        {
            return Self::not_checked(root, skip_sentence(reason));
        }
        let skip: Vec<PathBuf> = skip
            .iter()
            .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            .collect();
        let left_out = Arc::new(Mutex::new(BTreeSet::new()));
        let filter = EntryFilter {
            root: canonical_root.clone(),
            home: home.map(Path::to_path_buf),
            skip,
            left_out: Arc::clone(&left_out),
        };
        let mut builder = ignore::WalkBuilder::new(&canonical_root);
        builder
            .hidden(false)
            .parents(true)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .follow_links(false)
            .same_file_system(true)
            .sort_by_file_name(|a, b| a.cmp(b))
            .add_custom_ignore_filename(".biorouterignore")
            .filter_entry(move |entry| filter.keeps(entry));
        if let Some(global) = global_ignore {
            let _ = builder.add_ignore(global);
        }
        let mut hashed_bytes = 0u64;
        let mut visited = 0usize;
        for item in builder.build() {
            if stop.load(Ordering::Relaxed) {
                snapshot.stopped = Some(Stop::Cancelled);
                break;
            }
            if Instant::now() >= deadline {
                snapshot.stopped = Some(Stop::Time(limits.budget));
                break;
            }
            let entry = match item {
                Ok(entry) => entry,
                Err(error) => {
                    // A rule in an ignore file that does not parse is skipped
                    // by the walk itself and costs nothing here; a read that
                    // failed leaves its path out of the comparison.
                    if error.io_error().is_some() {
                        match error_path(&error).and_then(|path| key_of(&canonical_root, path)) {
                            Some(key) => {
                                snapshot.unreadable.insert(key);
                            }
                            None => snapshot.pathless_errors += 1,
                        }
                    }
                    continue;
                }
            };
            if entry.depth() == 0 {
                continue;
            }
            let Some(key) = key_of(&canonical_root, entry.path()) else {
                continue;
            };
            if visited >= limits.max_entries {
                snapshot.stopped = Some(Stop::Entries(limits.max_entries));
                break;
            }
            visited += 1;
            let Some(file_type) = entry.file_type() else {
                snapshot.last_visited = Some(key);
                continue;
            };
            if file_type.is_symlink() || file_type.is_file() {
                if snapshot.entries.len() >= limits.max_files {
                    snapshot.stopped = Some(Stop::Files(limits.max_files));
                    break;
                }
                if file_type.is_symlink() {
                    let target = std::fs::read_link(entry.path()).unwrap_or_default();
                    snapshot.entries.insert(key.clone(), Stamp::Link(target));
                } else {
                    match entry.metadata() {
                        Ok(metadata) => {
                            let len = metadata.len();
                            let hash = (len <= FOLDER_CHECK_MAX_HASHED_FILE_BYTES
                                && hashed_bytes + len <= FOLDER_CHECK_MAX_TOTAL_HASHED_BYTES)
                                .then(|| hash_file(entry.path()))
                                .flatten();
                            if hash.is_some() {
                                hashed_bytes += len;
                            }
                            snapshot.entries.insert(
                                key.clone(),
                                Stamp::File {
                                    len,
                                    modified: metadata.modified().ok(),
                                    hash,
                                },
                            );
                        }
                        Err(_) => {
                            snapshot.unreadable.insert(key.clone());
                        }
                    }
                }
            }
            snapshot.last_visited = Some(key);
        }
        if snapshot.unreadable.contains(&Key::new()) {
            return Self::not_checked(root, "it could not be read");
        }
        snapshot.left_out =
            std::mem::take(&mut *left_out.lock().unwrap_or_else(|p| p.into_inner()));
        snapshot
    }

    /// The folder this snapshot records.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// What changed between `self` (before the run) and `after`.
    ///
    /// Only paths both walks covered are compared: when either walk stopped
    /// early, nothing past the earlier stopping point, and nothing under a
    /// path either walk could not read (so a folder that became readable
    /// cannot read as added files). What was left out is named, and the
    /// result is `unchanged` only when nothing was.
    pub fn compare(&self, after: &FolderSnapshot) -> FolderCheck {
        let folder = self.root.display().to_string();
        if let Some(why) = self.not_checked.as_ref().or(after.not_checked.as_ref()) {
            return FolderCheck::not_checked(folder, why.clone());
        }
        let horizon: Option<Key> = [self, after]
            .into_iter()
            .filter(|snapshot| snapshot.stopped.is_some())
            .map(|snapshot| snapshot.last_visited.clone().unwrap_or_default())
            .min();
        let unreadable: BTreeSet<&Key> = self.unreadable.iter().chain(&after.unreadable).collect();
        let within = |key: &Key| {
            horizon.as_ref().is_none_or(|last| key <= last)
                && !unreadable.iter().any(|dir| key.starts_with(dir))
        };
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        let mut compared = 0usize;
        for (key, before) in self.entries.iter().filter(|(key, _)| within(key)) {
            compared += 1;
            match after.entries.get(key) {
                Some(now) if before.same_as(now) => {}
                Some(_) => changed.push(path_of(key)),
                None => removed.push(path_of(key)),
            }
        }
        let added: Vec<String> = after
            .entries
            .keys()
            .filter(|key| within(key) && !self.entries.contains_key(*key))
            .map(path_of)
            .collect();

        let mut not_compared = Vec::new();
        if let Some(stop) = self.stopped.as_ref().or(after.stopped.as_ref()) {
            not_compared.push(stop.describe());
        }
        // Named by the outermost path: a folder that cannot be read also fails
        // the reads of the ignore files inside it.
        let outermost: Vec<&Key> = unreadable
            .iter()
            .copied()
            .filter(|key| {
                !unreadable
                    .iter()
                    .any(|other| other.len() < key.len() && key.starts_with(other))
            })
            .collect();
        if !outermost.is_empty() {
            let mut names: Vec<String> = outermost.iter().map(|key| path_of(key)).collect();
            let more = names.len().saturating_sub(UNREADABLE_LISTED);
            names.truncate(UNREADABLE_LISTED);
            let more = if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            };
            not_compared.push(format!(
                "{} could not be read, so what is in {} was not compared",
                names.join(", ") + &more,
                if outermost.len() == 1 { "it" } else { "them" }
            ));
        }
        if self.pathless_errors + after.pathless_errors > 0 {
            not_compared.push("some entries could not be read and were not compared".to_string());
        }
        let left_out: Vec<String> = self.left_out.union(&after.left_out).cloned().collect();
        FolderCheck::from_lists(
            folder,
            compared,
            changed,
            added,
            removed,
            not_compared,
            left_out,
        )
    }
}

/// The walk's per-entry rule, shared with the walker's threads.
struct EntryFilter {
    root: PathBuf,
    home: Option<PathBuf>,
    skip: Vec<PathBuf>,
    left_out: Arc<Mutex<BTreeSet<String>>>,
}

impl EntryFilter {
    fn keeps(&self, entry: &ignore::DirEntry) -> bool {
        if entry.depth() == 0 {
            return true;
        }
        let path = entry.path();
        let Some(key) = key_of(&self.root, path) else {
            return false;
        };
        // `.git` itself is read, and of what is inside it only what changes
        // how the repository behaves: the checked-out branch, the settings and
        // the hooks.
        if let Some(at) = key.iter().position(|component| component == ".git") {
            if let Some(inside) = key.get(at + 1) {
                if !GIT_ENTRIES_CHECKED.iter().any(|name| inside == *name) {
                    return false;
                }
            }
        }
        if is_biorouter_output(&key) {
            return false;
        }
        if self.skip.iter().any(|dir| dir == path) {
            return false;
        }
        if crate::agents::workspace_summary::is_opaque_tree(path, self.home.as_deref()) {
            return false;
        }
        let is_dir = entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir());
        if is_dir {
            let name = entry.file_name().to_string_lossy();
            let tagged = || path.join(CACHE_DIR_TAG).is_file();
            if FOLDER_CHECK_LEFT_OUT_NAMES.contains(&name.as_ref()) || tagged() {
                if let Ok(mut left_out) = self.left_out.lock() {
                    left_out.insert(name.into_owned());
                }
                return false;
            }
        }
        true
    }
}

/// The folder Biorouter puts oversized tool results in
/// (`.biorouter/tool-output/`), and the `.biorouter/.gitignore` it writes, at
/// the root.
fn is_biorouter_output(key: &Key) -> bool {
    let handle_dir: Vec<&str> = crate::agents::large_response_handler::HANDLE_DIR
        .split('/')
        .collect();
    let starts_with_handle = key.len() >= handle_dir.len()
        && key
            .iter()
            .zip(&handle_dir)
            .all(|(component, name)| component == *name);
    let is_gitignore = key.len() == 2 && key[0] == *handle_dir[0] && key[1] == *".gitignore";
    starts_with_handle || is_gitignore
}

fn key_of(root: &Path, path: &Path) -> Option<Key> {
    let relative = path.strip_prefix(root).ok()?;
    Some(
        relative
            .components()
            .map(|component| component.as_os_str().to_os_string())
            .collect(),
    )
}

/// The path an error from the walk names, when it names one.
fn error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            error_path(err)
        }
        ignore::Error::Partial(errors) if errors.len() == 1 => error_path(&errors[0]),
        _ => None,
    }
}

fn skip_sentence(reason: crate::agents::workspace_summary::SkipReason) -> &'static str {
    use crate::agents::workspace_summary::SkipReason;
    match reason {
        SkipReason::Home => "it is the home folder, which is too large to compare",
        SkipReason::FilesystemRoot => "it is the root of a disk, which is too large to compare",
        SkipReason::OpaqueTree => {
            "it is inside a system or cloud-sync folder whose reads can stop answering"
        }
    }
}

fn hash_file(path: &Path) -> Option<[u8; 32]> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher).ok()?;
    Some(*hasher.finalize().as_bytes())
}

/// Biorouter's own folders, skipped when they sit inside a working folder:
/// the session store and logs change while any chat runs.
pub fn biorouter_folders() -> Vec<PathBuf> {
    use crate::config::paths::Paths;
    vec![
        Paths::config_dir(),
        Paths::data_dir(),
        Paths::state_dir(),
        Paths::local_data_dir(),
    ]
}

/// The working folder as it stands now, for a read-only verifier's check.
///
/// The walk runs on the blocking pool with [`FOLDER_CHECK_BUDGET`]; the wait
/// for it is raced against `cancel` and gives up a few seconds after the
/// budget, so neither a `Stop` nor a read that never returns holds the run.
/// The home folder, a filesystem root and opaque trees are refused without a
/// walk.
pub async fn snapshot_folder(root: PathBuf, cancel: Option<CancellationToken>) -> FolderSnapshot {
    let home = crate::config::paths::Paths::home_dir();
    if let Some(reason) =
        crate::agents::workspace_summary::skip_reason_with_home(&root, home.as_deref())
    {
        return FolderSnapshot::not_checked(&root, skip_sentence(reason));
    }
    let limits = FolderLimits::default();
    let walked = root.clone();
    snapshot_bounded(root, cancel, limits.budget, move |stop| {
        let global_ignore = crate::agents::workspace_summary::global_biorouterignore();
        FolderSnapshot::take_with(
            &walked,
            &biorouter_folders(),
            home.as_deref(),
            global_ignore.as_deref(),
            limits,
            &stop,
        )
    })
    .await
}

/// Walks of a folder that have started and not returned, by start time.
static WALKS_IN_FLIGHT: LazyLock<Mutex<Vec<(u64, PathBuf, Instant)>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static NEXT_WALK: AtomicU64 = AtomicU64::new(0);

/// Removes its walk from [`WALKS_IN_FLIGHT`] when the walk returns. A walk
/// blocked forever never drops it, which is what keeps a second walk of the
/// same folder from blocking another thread.
struct WalkInFlight(u64);

impl WalkInFlight {
    fn start(root: &Path) -> Self {
        let id = NEXT_WALK.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut walks) = WALKS_IN_FLIGHT.lock() {
            walks.push((id, root.to_path_buf(), Instant::now()));
        }
        Self(id)
    }
}

impl Drop for WalkInFlight {
    fn drop(&mut self) {
        if let Ok(mut walks) = WALKS_IN_FLIGHT.lock() {
            walks.retain(|(id, _, _)| *id != self.0);
        }
    }
}

/// Whether a walk of `root` started more than `patience` ago and has not
/// returned.
fn walk_presumed_blocked(root: &Path, patience: Duration) -> bool {
    WALKS_IN_FLIGHT.lock().is_ok_and(|walks| {
        walks
            .iter()
            .any(|(_, walked, started)| walked == root && started.elapsed() > patience)
    })
}

/// The budget and cancellation machinery, with the walk injected so a test
/// can supply one that blocks.
async fn snapshot_bounded<F>(
    root: PathBuf,
    cancel: Option<CancellationToken>,
    budget: Duration,
    walk: F,
) -> FolderSnapshot
where
    F: FnOnce(Arc<AtomicBool>) -> FolderSnapshot + Send + 'static,
{
    let patience = budget + FOLDER_CHECK_GRACE;
    if walk_presumed_blocked(&root, patience) {
        return FolderSnapshot::not_checked(
            &root,
            "an earlier check of it has not finished, which usually means something in it does \
             not answer when read",
        );
    }
    let stop = Arc::new(AtomicBool::new(false));
    let in_flight = WalkInFlight::start(&root);
    let walk_stop = Arc::clone(&stop);
    let mut handle = tokio::task::spawn_blocking(move || {
        let _in_flight = in_flight;
        walk(walk_stop)
    });
    enum Wait {
        Done(Result<FolderSnapshot, tokio::task::JoinError>),
        Cancelled,
        Late,
    }
    // `biased` puts cancellation first: a `Stop` returns on the next poll.
    let outcome = match cancel {
        Some(token) => tokio::select! {
            biased;
            _ = token.cancelled() => Wait::Cancelled,
            joined = &mut handle => Wait::Done(joined),
            _ = tokio::time::sleep(patience) => Wait::Late,
        },
        None => tokio::select! {
            joined = &mut handle => Wait::Done(joined),
            _ = tokio::time::sleep(patience) => Wait::Late,
        },
    };
    match outcome {
        Wait::Done(Ok(snapshot)) => snapshot,
        Wait::Done(Err(error)) => {
            FolderSnapshot::not_checked(&root, format!("the check failed: {error}"))
        }
        Wait::Cancelled => {
            stop.store(true, Ordering::Relaxed);
            FolderSnapshot::not_checked(&root, "the check was stopped")
        }
        Wait::Late => {
            stop.store(true, Ordering::Relaxed);
            FolderSnapshot::not_checked(
                &root,
                format!(
                    "reading it did not finish within {} seconds, which usually means something \
                     in it does not answer when read",
                    patience.as_secs()
                ),
            )
        }
    }
}

/// What a read-only verifier's parent found when it compared the working
/// folder before and after the run. Stored on the result as `folder_check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderCheck {
    /// The folder compared.
    pub folder: String,
    /// Every file in scope was compared and none changed, appeared or
    /// disappeared. False whenever anything in scope was not compared.
    pub unchanged: bool,
    /// How many files were compared.
    pub files_compared: usize,
    /// Whether every file in scope was compared.
    pub complete: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Paths that differed but are not listed above (each list names at most
    /// [`FOLDER_CHECK_LISTED`]).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unlisted: usize,
    /// What in scope was not compared, and why, in words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_compared: Vec<String>,
    /// Dependency and cache folders left out of scope, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_out: Vec<String>,
    /// Why the folder was not compared at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_checked: Option<String>,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl FolderCheck {
    fn not_checked(folder: String, why: String) -> Self {
        Self {
            folder,
            unchanged: false,
            files_compared: 0,
            complete: false,
            changed: Vec::new(),
            added: Vec::new(),
            removed: Vec::new(),
            unlisted: 0,
            not_compared: Vec::new(),
            left_out: Vec::new(),
            not_checked: Some(why),
        }
    }

    fn from_lists(
        folder: String,
        files_compared: usize,
        changed: Vec<String>,
        added: Vec<String>,
        removed: Vec<String>,
        not_compared: Vec<String>,
        left_out: Vec<String>,
    ) -> Self {
        let complete = not_compared.is_empty();
        let unchanged = complete && changed.is_empty() && added.is_empty() && removed.is_empty();
        let mut unlisted = 0;
        let mut cap = |mut list: Vec<String>| {
            unlisted += list.len().saturating_sub(FOLDER_CHECK_LISTED);
            list.truncate(FOLDER_CHECK_LISTED);
            list
        };
        let changed = cap(changed);
        let added = cap(added);
        let removed = cap(removed);
        Self {
            folder,
            unchanged,
            files_compared,
            complete,
            changed,
            added,
            removed,
            unlisted,
            not_compared,
            left_out,
            not_checked: None,
        }
    }

    /// What was out of scope, as a clause after the count.
    fn scope(&self) -> String {
        if self.left_out.is_empty() {
            "; ignored files were left out".to_string()
        } else {
            format!(
                "; ignored files and {} were left out",
                self.left_out.join(", ")
            )
        }
    }

    /// The sentence the parent's model reads.
    pub fn sentence(&self) -> String {
        if let Some(why) = &self.not_checked {
            return format!(
                "Working folder check: {} was not checked ({why}), so whether anything in it \
                 changed while the subagent ran is unknown.",
                self.folder
            );
        }
        let scope = self.scope();
        let incomplete = if self.complete {
            String::new()
        } else {
            format!(
                " Not everything was compared: {}.",
                self.not_compared.join("; ")
            )
        };
        let total = self.changed.len() + self.added.len() + self.removed.len() + self.unlisted;
        if total == 0 {
            if self.unchanged {
                return format!(
                    "Working folder check: no file in {} changed while the subagent ran ({} \
                     files compared{scope}).",
                    self.folder, self.files_compared
                );
            }
            return format!(
                "Working folder check: no change was found in the part of {} that was checked \
                 ({} files compared{scope}).{incomplete} This is not proof that nothing changed.",
                self.folder, self.files_compared
            );
        }
        let mut parts = Vec::new();
        for (label, list) in [
            ("changed", &self.changed),
            ("added", &self.added),
            ("removed", &self.removed),
        ] {
            if !list.is_empty() {
                parts.push(format!("{label} {}", list.join(", ")));
            }
        }
        let more = if self.unlisted > 0 {
            format!(" (and {} more)", self.unlisted)
        } else {
            String::new()
        };
        let noun = if total == 1 { "file" } else { "files" };
        format!(
            "Working folder check: {total} {noun} in {} changed while the subagent ran: {}{more} \
             ({} files compared{scope}). The subagent held no tool that writes, so this chat, \
             another subagent or another program changed them, or one of its tools wrote \
             although it is marked read-only. Check them before relying on its \
             result.{incomplete}",
            self.folder,
            parts.join("; "),
            self.files_compared
        )
    }
}

#[cfg(test)]
#[path = "subagent_presets_unit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "subagent_presets_agent_tests.rs"]
mod agent_tests;
