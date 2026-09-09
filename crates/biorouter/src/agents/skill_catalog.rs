//! The canonical skill catalog: **one** enumeration of the skill roots, **one**
//! discovery pass over them, and **one** composition of machine-wide with
//! per-session enablement.
//!
//! # Why this module exists (#113)
//!
//! Before it, three surfaces answered "which skills exist?" independently and
//! disagreed:
//!
//! * the backend scanned seven root kinds, including
//!   `~/.config/biorouter/extensions/<name>/skills`;
//! * the desktop renderer scanned **three** (`skillUtils.ts`'s `ALL_SKILL_DIRS`),
//!   so a skill bundled inside an installed extension was active for the model
//!   and invisible in the picker;
//! * the CLI scanned the backend's roots, and was therefore right — which is how
//!   `biorouter skill list` could show a skill the GUI had no row for.
//!
//! A second scanner with a different root list is not a bug you fix once; the
//! lists drift again the next time a root is added. So the roots are enumerated
//! **here**, [`SkillsClient::get_default_skill_directories`] delegates to
//! [`roots`], and the renderer is served this catalog over HTTP instead of
//! walking the filesystem itself.
//!
//! [`SkillsClient::get_default_skill_directories`]: super::skills_extension::SkillsClient::get_default_skill_directories
//!
//! # Why the catalog is refreshable, and what that fixes
//!
//! `SkillsClient` used to discover skills **in its constructor** and hold the
//! result for the life of the process. A skill installed afterwards was
//! therefore not in that client's map, so no amount of UI toggling could make
//! it loadable — the third root cause in #113, and the one that reads as "the
//! toggle does nothing" rather than as staleness.
//!
//! The catalog here is a process-global snapshot that every `SkillsClient`
//! reads through, so refreshing it reaches **every live conversation at once**
//! rather than only the one that triggered the install.
//!
//! Staleness is decided by two cheap tests, in this order:
//!
//! 1. the **root set** — recomputed on every read, because `Paths::config_dir`
//!    resolves `BIOROUTER_PATH_ROOT` at call time and a newly installed
//!    extension adds a root;
//! 2. the **modification time of every watched path** — each root, every bundle
//!    directory, each component parent, and every package record. Creating
//!    `<bundle>/<child>/` bumps the component parent; editing only an
//!    authoritative package record bumps neither directory.
//!
//! ⚠ **mtime has one-second granularity on some filesystems**, so a write in
//! the same second as the scan can be missed. That window is closed for every
//! change Biorouter makes itself: the importer, the session route and the
//! refresh route all call [`invalidate`]. It stays open only for a write by
//! *another process* — a `biorouter skill install` from a terminal — which the
//! interface's explicit refresh covers. Do not replace the explicit
//! invalidations with "the mtime check will catch it".

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::session_skills::{OverrideMatch, SessionSkillOverride};
use super::skills_extension::{self, Skill};
use crate::config::paths::Paths;

/// Which of the five kinds of root a skill came from.
///
/// ⚠ A **unit-variant** enum carrying no data, with the extension name held
/// beside it in [`SkillSource`] rather than inside an `Extension { .. }`
/// variant. The variant form is the more natural Rust, and it generates an
/// internally-tagged object that `serde(flatten)` and `utoipa` disagree about —
/// the spec emits an `allOf` the TypeScript client cannot narrow. A flat struct
/// crosses the wire unambiguously, which matters more here than the tidier
/// type, because this shape IS the contract the picker renders from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SkillSourceKind {
    /// `~/.claude/skills` — shared with other agents.
    ClaudeHome,
    /// `~/.config/agents/skills` — the portable cross-agent location.
    AgentsHome,
    /// `~/.config/biorouter/skills` — where Biorouter installs.
    Biorouter,
    /// `~/.config/biorouter/extensions/<extension>/skills` — skills that ship
    /// inside an installed extension bundle.
    Extension,
    /// `<cwd>/.claude/skills`, `<cwd>/.biorouter/skills`, `<cwd>/.agents/skills`.
    Project,
}

/// Where a skill came from, as the interface shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillSource {
    pub kind: SkillSourceKind,
    /// The extension's directory name, when `kind` is `extension`.
    pub extension: Option<String>,
    /// A short human label for the "where from" chip — the extension's name
    /// when it has one, else the root's own.
    pub label: String,
}

impl SkillSource {
    pub fn new(kind: SkillSourceKind, extension: Option<String>) -> Self {
        let label = match (&kind, &extension) {
            (SkillSourceKind::Extension, Some(extension)) => extension.clone(),
            (SkillSourceKind::ClaudeHome, _) => "Claude".to_string(),
            (SkillSourceKind::AgentsHome, _) => "Agents".to_string(),
            (SkillSourceKind::Biorouter, _) | (SkillSourceKind::Extension, None) => {
                "Biorouter".to_string()
            }
            (SkillSourceKind::Project, _) => "Project".to_string(),
        };
        Self {
            kind,
            extension,
            label,
        }
    }
}

/// One directory skills are discovered under, with its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillRoot {
    #[schema(value_type = String)]
    pub path: PathBuf,
    pub source: SkillSource,
}

/// Every directory skills are discovered under, in override order (later wins).
///
/// This is the **single** definition of that list. Adding a root means editing
/// this function and nothing else: the backend extension, the CLI and the
/// desktop interface all reach it from here.
pub fn roots() -> Vec<SkillRoot> {
    let mut roots = Vec::new();

    // `Paths::home_dir`, not `dirs::home_dir`: the latter ignores the
    // environment on Windows, so a relocated home silently did not apply there
    // and this function kept reading the real profile's `.claude/skills`.
    if let Some(home) = crate::config::paths::Paths::home_dir() {
        roots.push(SkillRoot {
            path: home.join(".claude/skills"),
            source: SkillSource::new(SkillSourceKind::ClaudeHome, None),
        });
        roots.push(SkillRoot {
            path: home.join(".config/agents/skills"),
            source: SkillSource::new(SkillSourceKind::AgentsHome, None),
        });
    }

    roots.push(SkillRoot {
        path: crate::agents::skills_extension::skills_root(&Paths::config_dir()),
        source: SkillSource::new(SkillSourceKind::Biorouter, None),
    });

    // Skills bundled inside installed `.brxt` extensions. Sorted, so the
    // override order between two extensions that ship the same skill name is
    // stable rather than whatever order the filesystem hands back.
    let extensions_dir = Paths::config_dir().join("extensions");
    if let Ok(entries) = std::fs::read_dir(&extensions_dir) {
        let mut extension_roots: Vec<SkillRoot> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| {
                let skills_subdir = entry.path().join("skills");
                if !skills_subdir.is_dir() {
                    return None;
                }
                let extension = entry.file_name().to_string_lossy().to_string();
                Some(SkillRoot {
                    path: skills_subdir,
                    source: SkillSource::new(SkillSourceKind::Extension, Some(extension)),
                })
            })
            .collect();
        extension_roots.sort_by(|a, b| a.path.cmp(&b.path));
        roots.extend(extension_roots);
    }

    if let Ok(working_dir) = std::env::current_dir() {
        for relative in [".claude/skills", ".biorouter/skills", ".agents/skills"] {
            roots.push(SkillRoot {
                path: working_dir.join(relative),
                source: SkillSource::new(SkillSourceKind::Project, None),
            });
        }
    }

    roots
}

/// [`roots`] as a path → provenance lookup, so a caller projecting many skills
/// pays for the root walk (a `read_dir` of the extensions directory and a
/// `current_dir`) once instead of once per skill.
pub fn root_sources() -> HashMap<PathBuf, SkillSource> {
    root_sources_of(&roots())
}

fn root_sources_of(roots: &[SkillRoot]) -> HashMap<PathBuf, SkillSource> {
    roots
        .iter()
        .map(|root| (root.path.clone(), root.source.clone()))
        .collect()
}

/// The provenance of one discovered skill's `source_root`.
///
/// ⚠ **An unknown root reads as [`SkillSourceKind::Biorouter`]**, and that
/// fallback is the reason this is a shared function rather than one copy per
/// caller: [`SkillCatalog::source_of`] has always answered that way, and a
/// second spelling of it would be free to answer differently. In production the
/// fallback is unreachable — every `source_root` a skill carries came from
/// [`roots`] — so it only ever decides what a test fixture over a temporary
/// directory looks like.
pub fn source_in(sources: &HashMap<PathBuf, SkillSource>, source_root: &Path) -> SkillSource {
    sources
        .get(source_root)
        .cloned()
        .unwrap_or_else(|| SkillSource::new(SkillSourceKind::Biorouter, None))
}

/// How one session deviates from the machine-wide answer for a given skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    /// No per-chat opinion; the machine-wide answer stands.
    Default,
    /// Enabled for this chat even though machine-wide disabled.
    Added,
    /// Disabled for this chat even though machine-wide enabled.
    Removed,
}

/// The composed enablement of one catalog entry, with every input kept
/// separate so the interface can explain *why* a switch is where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillState {
    /// `skills-config.json` (`disabled[]`) says this is on. A bundle child is
    /// off when either its own name or its bundle's name is listed.
    pub machine_enabled: bool,
    /// This conversation's deviation.
    pub session: SessionState,
    /// The deviation was written against the BUNDLE's name, not this skill's.
    /// Lets the interface explain a member's switch instead of leaving it
    /// looking arbitrary.
    pub session_via_bundle: bool,
    /// A shipped **Context** the user switched off in Settings → Contexts. Such
    /// a skill is hidden from the catalog but stays loadable by exact name; see
    /// `skills_extension::hidden_contexts_in`.
    pub hidden_context: bool,
    /// What the model actually sees: the composition of the three above.
    pub effective: bool,
}

/// One skill, as the interface and the model both see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSkill {
    /// Frontmatter `name` — the identifier every enablement surface keys on.
    pub name: String,
    pub description: String,
    /// Root-relative logical path, `/`-separated on every platform
    /// (`superpowers/brainstorming`). What `biorouter skill list` prints.
    pub slug: String,
    #[schema(value_type = String)]
    pub directory: PathBuf,
    #[schema(value_type = String)]
    pub source_root: PathBuf,
    pub source: SkillSource,
    /// The bundle directory this skill sits in, when it is a bundle member.
    pub bundle: Option<String>,
    /// Shipped with Biorouter, so the interface offers no Delete for it.
    pub builtin: bool,
    pub state: SkillState,
}

/// A directory of skills installed and removed as one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogBundle {
    /// The bundle directory name. This is the identifier `skills-config.json`
    /// lists to disable the whole bundle.
    pub name: String,
    /// The package's own display name when a manifest supplied one, else the
    /// directory name.
    pub display_name: String,
    #[schema(value_type = String)]
    pub directory: PathBuf,
    #[schema(value_type = String)]
    pub source_root: PathBuf,
    pub source: SkillSource,
    /// Member skill names, sorted.
    pub skills: Vec<String>,
    /// The importer's record, when this bundle was installed as a package.
    pub package: Option<PackageSummary>,
    /// Shipped with Biorouter, so the interface offers no Delete for it.
    ///
    /// ⚠ **The bundle needs its own answer**, because `CatalogSkill.builtin`
    /// gates the Delete on a *skill* row and a bundle row is a different
    /// control over a different directory.
    ///
    /// ⚠ **This is defence in depth, not the live fix, and saying otherwise
    /// invites someone to delete the real one.** Today the only shipped bundle
    /// is a Context, and `pickerBundles` strips Contexts before `SkillsView`
    /// renders a row at all, so this flag cannot fire on it. What actually
    /// closed the "delete succeeds, toast confirms, next startup rewrites it"
    /// regression on every surface is `skill_package::refuse_shipped`, which
    /// `biorouter skill remove` bypassed entirely while the interface gate held.
    /// This field earns its place for the case the filter does not cover: a
    /// bundle that is seeded but not a Context, should one ever ship.
    pub builtin: bool,
    pub state: SkillState,
}

/// The part of an installed package's record the interface needs. The full
/// record lives beside the skills as `biorouter-package.json`; see
/// `crate::agents::skill_package`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageSummary {
    pub id: String,
    pub display_name: String,
    pub version: Option<String>,
    pub source_url: Option<String>,
    pub source_ref: Option<String>,
    pub resolved_commit: Option<String>,
    pub installer: Option<String>,
    pub installed_at: Option<String>,
    /// The router/entry-point skill a manifest declared, by frontmatter name.
    pub entry_point: Option<String>,
    /// Optional named groups, e.g. `core` / `on-demand`.
    #[serde(default)]
    #[schema(value_type = Object, additional_properties)]
    pub groups: BTreeMap<String, Vec<String>>,
}

/// The file an installed package writes at its bundle root. Read here rather
/// than in the importer so that a bundle a *previous* Biorouter version
/// installed, or a user assembled by hand, still appears — just without a
/// package record.
pub const PACKAGE_RECORD_FILE: &str = "biorouter-package.json";

#[derive(Deserialize)]
struct PackageComponentDirectory {
    name: String,
    directory: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PackageComponentDiscovery {
    Legacy,
    Invalid,
    Valid(Vec<PackageComponentSkillFile>),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PackageComponentSkillFile {
    pub name: String,
    pub path: PathBuf,
}

/// Exact component entry points declared by a package record.  This is the
/// bounded alternative to recursively searching an imported repository.
pub(crate) fn package_component_skill_files(bundle_dir: &Path) -> PackageComponentDiscovery {
    let record_path = bundle_dir.join(PACKAGE_RECORD_FILE);
    match std::fs::symlink_metadata(&record_path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return PackageComponentDiscovery::Legacy;
        }
        Err(_) => return PackageComponentDiscovery::Invalid,
    }
    let Ok(raw) = std::fs::read_to_string(&record_path) else {
        return PackageComponentDiscovery::Invalid;
    };
    let Ok(record) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return PackageComponentDiscovery::Invalid;
    };
    let Some(record) = record.as_object() else {
        return PackageComponentDiscovery::Invalid;
    };
    let Some(raw_components) = record.get("components") else {
        return PackageComponentDiscovery::Legacy;
    };
    let Ok(components) =
        serde_json::from_value::<Vec<PackageComponentDirectory>>(raw_components.clone())
    else {
        return PackageComponentDiscovery::Invalid;
    };
    if components.is_empty() {
        return PackageComponentDiscovery::Invalid;
    }

    let Ok(canonical_bundle) = std::fs::canonicalize(bundle_dir) else {
        return PackageComponentDiscovery::Invalid;
    };
    let mut skill_files = Vec::with_capacity(components.len());
    let mut names = HashSet::new();
    let mut directories = HashSet::new();
    for component in components {
        if component.name.is_empty()
            || !names.insert(component.name.clone())
            || component.directory.contains('\\')
            || !directories.insert(component.directory.clone())
        {
            return PackageComponentDiscovery::Invalid;
        }
        let relative = Path::new(&component.directory);
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return PackageComponentDiscovery::Invalid;
        }
        let skill_file = bundle_dir.join(relative).join("SKILL.md");
        let Ok(canonical_skill) = std::fs::canonicalize(&skill_file) else {
            return PackageComponentDiscovery::Invalid;
        };
        if !canonical_skill.starts_with(&canonical_bundle) || !canonical_skill.is_file() {
            return PackageComponentDiscovery::Invalid;
        }
        skill_files.push(PackageComponentSkillFile {
            name: component.name,
            path: skill_file,
        });
    }
    PackageComponentDiscovery::Valid(skill_files)
}

/// A complete answer to "what skills does this machine have, and which are on".
#[derive(Debug, Clone)]
pub struct SkillCatalog {
    /// Bumped on every rescan. The interface uses it to notice it is holding a
    /// stale answer without diffing the whole catalog.
    pub generation: u64,
    pub roots: Vec<SkillRoot>,
    /// Discovery keyed by frontmatter name, exactly as the extension loads it.
    /// `Arc` so a `SkillsClient` can hold the map across an await without
    /// pinning the whole snapshot or copying every skill body.
    skills: Arc<HashMap<String, Skill>>,
    /// Physical source root plus bundle directory name → provenance and members.
    bundles: BTreeMap<(PathBuf, String), BundleRecord>,
    /// Source per root path, for attributing a skill to where it came from.
    root_sources: HashMap<PathBuf, SkillSource>,
    /// (path, mtime) pairs whose change means this snapshot is stale.
    watched: Vec<(PathBuf, Option<SystemTime>)>,
}

#[derive(Debug, Clone)]
struct BundleRecord {
    directory: PathBuf,
    source_root: PathBuf,
    members: Vec<String>,
    package: Option<PackageSummary>,
}

impl SkillCatalog {
    /// Scan the given roots. Public so a test can build a catalog over a
    /// temporary tree without touching the process-global one.
    pub fn scan(roots: Vec<SkillRoot>, generation: u64) -> Self {
        let existing: Vec<PathBuf> = roots
            .iter()
            .filter(|root| root.path.exists())
            .map(|root| root.path.clone())
            .collect();

        let mut skills = skills_extension::SkillsClient::discover_skills_in_directories(&existing);
        skills_extension::add_missing_shipped_skills(&mut skills);

        let root_sources = root_sources_of(&roots);

        // Bundles are derived from the discovery result rather than from a
        // second directory walk, so a bundle can never contain a member the
        // extension would not load (unparseable frontmatter, a shadowed name).
        let mut bundles: BTreeMap<(PathBuf, String), BundleRecord> = BTreeMap::new();
        for skill in skills.values() {
            let Some(bundle_name) = skill.bundle_name.clone() else {
                continue;
            };
            let directory = skill.source_root.join(&bundle_name);
            let key = (skill.source_root.clone(), bundle_name);
            let entry = bundles.entry(key).or_insert_with(|| BundleRecord {
                package: read_package_record(&directory),
                directory,
                source_root: skill.source_root.clone(),
                members: Vec::new(),
            });
            entry.members.push(skill.metadata.name.clone());
        }
        for record in bundles.values_mut() {
            record.members.sort();
            record.members.dedup();
        }

        let mut watched: Vec<PathBuf> = roots.iter().map(|root| root.path.clone()).collect();
        watched.extend(bundles.values().map(|record| record.directory.clone()));
        watched.extend(
            bundles
                .values()
                .map(|record| record.directory.join(PACKAGE_RECORD_FILE)),
        );
        for root in &existing {
            if let Ok(entries) = std::fs::read_dir(root) {
                watched.extend(entries.flatten().filter_map(|entry| {
                    let record = entry.path().join(PACKAGE_RECORD_FILE);
                    std::fs::symlink_metadata(&record).ok().map(|_| record)
                }));
            }
        }
        watched.extend(
            skills
                .values()
                .filter_map(|skill| skill.directory.parent().map(Path::to_path_buf)),
        );
        watched.sort();
        watched.dedup();
        let watched = watched
            .into_iter()
            .map(|path| {
                let mtime = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok());
                (path, mtime)
            })
            .collect();

        Self {
            generation,
            roots,
            skills: Arc::new(skills),
            bundles,
            root_sources,
            watched,
        }
    }

    /// Discovery as the extension consumes it — keyed by frontmatter name.
    pub fn skills(&self) -> Arc<HashMap<String, Skill>> {
        Arc::clone(&self.skills)
    }

    /// Would a fresh scan differ from this snapshot? See the module header for
    /// the two tests and the one-second window the second of them leaves open.
    fn is_stale(&self, current_roots: &[SkillRoot]) -> bool {
        if current_roots != self.roots.as_slice() {
            return true;
        }
        self.watched.iter().any(|(path, seen)| {
            let now = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
            now != *seen
        })
    }

    fn source_of(&self, source_root: &Path) -> SkillSource {
        source_in(&self.root_sources, source_root)
    }

    /// The catalog as one conversation sees it.
    ///
    /// `over` is that conversation's persisted override
    /// (`workspace_skills/v1`); pass [`SessionSkillOverride::default`] for the
    /// machine-wide view.
    pub fn view(&self, over: &SessionSkillOverride) -> CatalogView {
        let machine_disabled = skills_extension::disabled_skill_names();
        let hidden_contexts = skills_extension::hidden_context_names();

        let mut skills: Vec<CatalogSkill> = self
            .skills
            .values()
            .map(|skill| {
                let name = &skill.metadata.name;
                let state = compose_state(
                    name,
                    skill.bundle_name.as_deref(),
                    &machine_disabled,
                    &hidden_contexts,
                    over,
                );
                CatalogSkill {
                    name: name.clone(),
                    description: skill.metadata.description.clone(),
                    slug: slug_of(&skill.directory, &skill.source_root),
                    directory: skill.directory.clone(),
                    source_root: skill.source_root.clone(),
                    source: self.source_of(&skill.source_root),
                    bundle: skill.bundle_name.clone(),
                    builtin: skills_extension::is_builtin_skill_name(name),
                    state,
                }
            })
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));

        let bundles: Vec<CatalogBundle> = self
            .bundles
            .iter()
            .map(|((_, name), record)| {
                let state = compose_state(name, None, &machine_disabled, &hidden_contexts, over);
                CatalogBundle {
                    name: name.clone(),
                    display_name: record
                        .package
                        .as_ref()
                        .map(|p| p.display_name.clone())
                        .unwrap_or_else(|| name.clone()),
                    directory: record.directory.clone(),
                    source_root: record.source_root.clone(),
                    source: self.source_of(&record.source_root),
                    skills: record.members.clone(),
                    package: record.package.clone(),
                    // ⚠ `== KNOWLEDGE_BUNDLE`, not `is_shipped_entry_name`.
                    // That predicate also answers yes for the nine shipped
                    // SKILL names, and this is a bundle: a user package whose
                    // directory happens to be called `knowledge-lint` would
                    // read as built-in, lose its Delete control, and be refused
                    // by `refuse_shipped` — the same false-positive class the
                    // predicate's own test rules out one level down.
                    builtin: name == skills_extension::KNOWLEDGE_BUNDLE,
                    state,
                }
            })
            .collect();

        CatalogView {
            generation: self.generation,
            roots: self.roots.clone(),
            skills,
            bundles,
        }
    }
}

/// The serialisable catalog: what `GET /skills/catalog` returns and what the
/// desktop picker renders. There is no second derivation of any of these
/// fields on the interface side — that separation is what #113 removed.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub generation: u64,
    pub roots: Vec<SkillRoot>,
    pub skills: Vec<CatalogSkill>,
    pub bundles: Vec<CatalogBundle>,
}

/// The one composition rule, shared by the model-facing filter and the
/// interface's switches.
///
/// ⚠ **`skills_extension` calls this too** — its
/// `SkillsClient::is_skill_enabled_for_session` is a thin wrapper over
/// `compose_state(..).effective`, and that is not tidiness. Two hand-written
/// copies of a precedence rule is exactly how a switch comes to disagree with
/// what the model sees, which is the class of bug #113 catalogues. There is one
/// copy, and both surfaces read it.
///
/// The order: a hidden Context is hidden **before** the session test, because a
/// per-chat grant must not put back something the user switched off in
/// Settings → Contexts.
pub(crate) fn compose_state(
    name: &str,
    bundle: Option<&str>,
    machine_disabled: &HashSet<String>,
    hidden_contexts: &HashSet<String>,
    over: &SessionSkillOverride,
) -> SkillState {
    let machine_enabled = !machine_disabled.contains(name)
        && !bundle.is_some_and(|bundle| machine_disabled.contains(bundle));
    // ⚠ Two keys, exactly as above. A Context row may name a whole bundle
    // (`skills_extension::context_ids`), and a member carries its own `name:` —
    // so a one-key test would leave the switch moving and the members visible.
    let hidden_context = hidden_contexts.contains(name)
        || bundle.is_some_and(|bundle| hidden_contexts.contains(bundle));

    let (session, via_bundle) = match over.resolve(name, bundle) {
        OverrideMatch::Added { via_bundle } => (SessionState::Added, via_bundle),
        OverrideMatch::Removed { via_bundle } => (SessionState::Removed, via_bundle),
        OverrideMatch::None => (SessionState::Default, false),
    };

    let effective = !hidden_context
        && match session {
            SessionState::Added => true,
            SessionState::Removed => false,
            SessionState::Default => machine_enabled,
        };

    SkillState {
        machine_enabled,
        session,
        session_via_bundle: via_bundle,
        hidden_context,
        effective,
    }
}

/// A slug is a LOGICAL identifier, not a path, so its separator is `/` on every
/// platform. Same rule, and the same reasoning, as
/// `biorouter-cli`'s `slug_from_relative_path`: a `\` separator prints a slug
/// the user cannot type back.
fn slug_of(directory: &Path, source_root: &Path) -> String {
    directory
        .strip_prefix(source_root)
        .unwrap_or(directory)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn read_package_record(bundle_dir: &Path) -> Option<PackageSummary> {
    let raw = std::fs::read_to_string(bundle_dir.join(PACKAGE_RECORD_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

// ---------------------------------------------------------------------------
// The process-global snapshot.
// ---------------------------------------------------------------------------

struct CatalogCell {
    snapshot: Option<Arc<SkillCatalog>>,
    generation: u64,
}

fn cell() -> &'static Mutex<CatalogCell> {
    static CELL: OnceLock<Mutex<CatalogCell>> = OnceLock::new();
    CELL.get_or_init(|| {
        Mutex::new(CatalogCell {
            snapshot: None,
            generation: 0,
        })
    })
}

/// The current catalog, rescanning only if it has gone stale.
///
/// Every `SkillsClient` reads through this, so a refresh reaches every live
/// conversation rather than only the one that asked.
pub fn current() -> Arc<SkillCatalog> {
    let roots = roots();
    let mut guard = cell().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(snapshot) = guard.snapshot.as_ref() {
        if !snapshot.is_stale(&roots) {
            return Arc::clone(snapshot);
        }
    }
    guard.generation += 1;
    let generation = guard.generation;
    let scanned = Arc::new(SkillCatalog::scan(roots, generation));
    guard.snapshot = Some(Arc::clone(&scanned));
    scanned
}

/// Rescan unconditionally and publish the result.
///
/// This is what an install calls. Prefer it over [`invalidate`] when the caller
/// wants the new catalog back — for example to report how many skills a package
/// added.
pub fn refresh() -> Arc<SkillCatalog> {
    let roots = roots();
    let mut guard = cell().lock().unwrap_or_else(|e| e.into_inner());
    guard.generation += 1;
    let generation = guard.generation;
    let scanned = Arc::new(SkillCatalog::scan(roots, generation));
    guard.snapshot = Some(Arc::clone(&scanned));
    scanned
}

/// Drop the cached snapshot so the next [`current`] rescans.
///
/// The entry point for anything that changes skills on disk and does not need
/// the result — notably an extension install, which changes the **root set**
/// (see [`roots`]) as well as its contents.
pub fn invalidate() {
    let mut guard = cell().lock().unwrap_or_else(|e| e.into_inner());
    guard.snapshot = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(root: &Path, relative: &str, name: &str, description: &str) {
        let dir = root.join(relative);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\nBody.\n"),
        )
        .unwrap();
    }

    fn root_at(path: &Path, source: SkillSource) -> SkillRoot {
        SkillRoot {
            path: path.to_path_buf(),
            source,
        }
    }

    fn biorouter_root() -> SkillSource {
        SkillSource::new(SkillSourceKind::Biorouter, None)
    }

    /// ⚠ **`compose_state`'s bundle arm, tested where it lives.**
    ///
    /// `SkillsClient::is_hidden_context` is a *different function on a
    /// different path*: `is_skill_enabled_for_session` passes an empty hidden
    /// set, so the two arms are genuinely independent and a test of one says
    /// nothing about the other. An earlier draft tested only that one and
    /// claimed both — delete the `|| bundle.is_some_and(...)` clause below and
    /// every `CatalogSkill` row for a bundle member reports
    /// `hiddenContext: false, effective: true`, which is the answer the
    /// INTERFACE renders, with the whole suite green.
    #[test]
    fn a_hidden_context_that_names_a_bundle_hides_the_bundles_members() {
        let none = SessionSkillOverride::default();
        let machine = HashSet::new();
        let hidden = HashSet::from(["knowledge-bases".to_string()]);

        let member = compose_state(
            "knowledge-lint",
            Some("knowledge-bases"),
            &machine,
            &hidden,
            &none,
        );
        assert!(member.hidden_context, "the member escaped its bundle's row");
        assert!(!member.effective);
        // The switch is a Context, not a machine-wide disable: the member is
        // still ENABLED, just not surfaced. Conflating the two is what would
        // make `handle_load_skill` refuse a skill the system prompt asks for.
        assert!(member.machine_enabled);

        // The bundle row itself, which `view()` composes with `bundle: None`.
        let row = compose_state("knowledge-bases", None, &machine, &hidden, &none);
        assert!(row.hidden_context && !row.effective);

        // A member of some other bundle is untouched, and so is a skill that
        // merely carries the same name with no bundle on it.
        assert!(
            !compose_state(
                "brainstorming",
                Some("superpowers"),
                &machine,
                &hidden,
                &none
            )
            .hidden_context
        );
        assert!(!compose_state("knowledge-lint", None, &machine, &hidden, &none).hidden_context);
    }

    /// A per-chat grant must not put back something switched off in Settings —
    /// the `hidden_context` test sits OUTSIDE the session match, and adding the
    /// bundle arm must not have moved it.
    #[test]
    fn a_per_chat_grant_cannot_resurrect_a_hidden_context_bundle() {
        let machine = HashSet::new();
        let hidden = HashSet::from(["knowledge-bases".to_string()]);
        let granted = SessionSkillOverride {
            add: vec!["knowledge-lint".to_string()],
            ..Default::default()
        };
        let state = compose_state(
            "knowledge-lint",
            Some("knowledge-bases"),
            &machine,
            &hidden,
            &granted,
        );
        assert_eq!(state.session, SessionState::Added);
        assert!(
            !state.effective,
            "a session grant overrode a Settings switch"
        );
    }

    /// The bundles a temporary root actually contains.
    ///
    /// ⚠ **`add_missing_shipped_skills` injects the shipped skills into every
    /// scan**, and since they became a bundle that injection contributes a
    /// `KNOWLEDGE_BUNDLE` row over a directory the temp root does not have. It
    /// is the right behaviour — the in-memory fallback must place a skill where
    /// the seeder would have — but it means `view.bundles[0]` no longer names
    /// what a test wrote. Index by name, not by position.
    fn authored_bundles(view: &CatalogView) -> Vec<&CatalogBundle> {
        view.bundles
            .iter()
            .filter(|bundle| bundle.name != skills_extension::KNOWLEDGE_BUNDLE)
            .collect()
    }

    #[test]
    fn a_bundle_is_derived_from_discovery_not_from_a_second_walk() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        write_skill(&root, "solo", "solo", "A single skill");
        write_skill(&root, "pack/alpha", "alpha", "First");
        write_skill(&root, "pack/beta", "beta", "Second");
        // Unparseable frontmatter: the extension will not load it, so the
        // bundle must not claim it either.
        let broken = root.join("pack/broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("SKILL.md"), "no frontmatter here").unwrap();

        let catalog = SkillCatalog::scan(vec![root_at(&root, biorouter_root())], 1);
        let view = catalog.view(&SessionSkillOverride::default());

        let bundles = authored_bundles(&view);
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].name, "pack");
        assert_eq!(bundles[0].skills, vec!["alpha", "beta"]);
        assert!(view.skills.iter().any(|s| s.name == "solo"));
        assert!(!view.skills.iter().any(|s| s.name == "broken"));
    }

    #[test]
    fn a_slug_is_slash_separated_and_root_relative() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        write_skill(&root, "pack/alpha", "alpha", "First");

        let catalog = SkillCatalog::scan(vec![root_at(&root, biorouter_root())], 1);
        let view = catalog.view(&SessionSkillOverride::default());
        let alpha = view.skills.iter().find(|s| s.name == "alpha").unwrap();
        assert_eq!(alpha.slug, "pack/alpha");
        assert_eq!(alpha.bundle.as_deref(), Some("pack"));
    }

    /// The defect in #113's root cause 2, stated as a test: a skill that lives
    /// under an installed extension is a first-class catalog entry and says so.
    #[test]
    fn an_extension_bundled_skill_is_attributed_to_its_extension() {
        let temp = TempDir::new().unwrap();
        let user = temp.path().join("skills");
        let extension = temp.path().join("extensions/BiorOffice/skills");
        write_skill(&user, "mine", "mine", "User skill");
        write_skill(&extension, "word", "word", "Write a document");

        let catalog = SkillCatalog::scan(
            vec![
                root_at(&user, biorouter_root()),
                root_at(
                    &extension,
                    SkillSource::new(SkillSourceKind::Extension, Some("BiorOffice".to_string())),
                ),
            ],
            1,
        );
        let view = catalog.view(&SessionSkillOverride::default());
        let word = view.skills.iter().find(|s| s.name == "word").unwrap();
        assert_eq!(word.source.kind, SkillSourceKind::Extension);
        assert_eq!(word.source.extension.as_deref(), Some("BiorOffice"));
        assert_eq!(word.source.label, "BiorOffice");
    }

    #[test]
    fn a_session_override_composes_over_the_machine_answer() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        write_skill(&root, "alpha", "alpha", "First");
        write_skill(&root, "beta", "beta", "Second");

        let catalog = SkillCatalog::scan(vec![root_at(&root, biorouter_root())], 1);
        let over = SessionSkillOverride {
            add: vec![],
            remove: vec!["beta".to_string()],
        };
        let view = catalog.view(&over);

        let alpha = view.skills.iter().find(|s| s.name == "alpha").unwrap();
        assert_eq!(alpha.state.session, SessionState::Default);
        assert!(alpha.state.effective);

        let beta = view.skills.iter().find(|s| s.name == "beta").unwrap();
        assert_eq!(beta.state.session, SessionState::Removed);
        assert!(
            beta.state.machine_enabled,
            "machine-wide state is unchanged"
        );
        assert!(!beta.state.effective, "but this chat does not see it");
    }

    #[test]
    fn a_package_record_beside_the_skills_is_read_into_the_bundle() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        write_skill(&root, "hyperframes/hyperframes", "hyperframes", "Router");
        write_skill(&root, "hyperframes/media-use", "media-use", "Media");
        fs::write(
            root.join("hyperframes").join(PACKAGE_RECORD_FILE),
            serde_json::json!({
                "id": "hyperframes",
                "displayName": "HyperFrames",
                "version": "0.8.12",
                "entryPoint": "hyperframes",
            })
            .to_string(),
        )
        .unwrap();

        let catalog = SkillCatalog::scan(vec![root_at(&root, biorouter_root())], 1);
        let view = catalog.view(&SessionSkillOverride::default());
        let bundle = authored_bundles(&view)[0];
        assert_eq!(bundle.name, "hyperframes");
        assert_eq!(bundle.display_name, "HyperFrames");
        let package = bundle.package.as_ref().expect("package record read");
        assert_eq!(package.version.as_deref(), Some("0.8.12"));
        assert_eq!(package.entry_point.as_deref(), Some("hyperframes"));
        // A member that shares the package's own name is the router, and it
        // keeps that name — no prefix is added as a grouping mechanism (#115).
        assert!(bundle.skills.contains(&"media-use".to_string()));
        assert!(bundle.skills.contains(&"hyperframes".to_string()));
    }

    #[test]
    fn same_named_bundles_keep_physical_ownership_and_metadata_separate() {
        let temp = TempDir::new().unwrap();
        let first_root = temp.path().join("a/skills");
        let second_root = temp.path().join("z/skills");
        write_skill(&first_root, "pack/alpha", "alpha", "First root");
        write_skill(&second_root, "pack/beta", "beta", "Second root");
        for (root, display_name, source_url) in [
            (&first_root, "First Pack", "https://example.invalid/first"),
            (
                &second_root,
                "Second Pack",
                "https://example.invalid/second",
            ),
        ] {
            fs::write(
                root.join("pack").join(PACKAGE_RECORD_FILE),
                serde_json::json!({
                    "id": "pack",
                    "displayName": display_name,
                    "sourceUrl": source_url,
                })
                .to_string(),
            )
            .unwrap();
        }

        let catalog = SkillCatalog::scan(
            vec![
                root_at(&second_root, biorouter_root()),
                root_at(&first_root, biorouter_root()),
            ],
            1,
        );
        let view = catalog.view(&SessionSkillOverride::default());
        let bundles: Vec<_> = authored_bundles(&view)
            .into_iter()
            .filter(|bundle| bundle.name == "pack")
            .collect();

        assert_eq!(bundles.len(), 2);
        assert_eq!(bundles[0].directory, first_root.join("pack"));
        assert_eq!(bundles[0].source_root, first_root);
        assert_eq!(bundles[0].display_name, "First Pack");
        assert_eq!(
            bundles[0].package.as_ref().unwrap().source_url.as_deref(),
            Some("https://example.invalid/first")
        );
        assert_eq!(bundles[1].directory, second_root.join("pack"));
        assert_eq!(bundles[1].source_root, second_root);
        assert_eq!(bundles[1].display_name, "Second Pack");
        assert_eq!(
            bundles[1].package.as_ref().unwrap().source_url.as_deref(),
            Some("https://example.invalid/second")
        );
    }

    #[test]
    fn package_component_paths_cannot_escape_the_package_root() {
        let temp = TempDir::new().unwrap();
        let bundle = temp.path().join("bundle");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&bundle).unwrap();
        write_skill(temp.path(), "outside", "outside", "Must remain outside");
        fs::write(
            bundle.join(PACKAGE_RECORD_FILE),
            serde_json::json!({
                "components": [{"name": "outside", "directory": "../outside"}],
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            package_component_skill_files(&bundle),
            PackageComponentDiscovery::Invalid,
            "an invalid package record must not fall back to broader discovery"
        );
        assert!(outside.join("SKILL.md").is_file());
    }

    #[test]
    fn package_record_presence_distinguishes_legacy_from_invalid() {
        let temp = TempDir::new().unwrap();
        let bundle = temp.path().join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        assert_eq!(
            package_component_skill_files(&bundle),
            PackageComponentDiscovery::Legacy
        );

        fs::write(
            bundle.join(PACKAGE_RECORD_FILE),
            r#"{"id":"legacy-metadata-only"}"#,
        )
        .unwrap();
        assert_eq!(
            package_component_skill_files(&bundle),
            PackageComponentDiscovery::Legacy
        );

        for non_object_record in ["null", "[]", r#""scalar""#, "1", "true"] {
            fs::write(bundle.join(PACKAGE_RECORD_FILE), non_object_record).unwrap();
            assert_eq!(
                package_component_skill_files(&bundle),
                PackageComponentDiscovery::Invalid,
                "a non-object package record must be invalid: {non_object_record}"
            );
        }

        fs::write(
            bundle.join(PACKAGE_RECORD_FILE),
            r#"{"id":"invalid","components":[]}"#,
        )
        .unwrap();
        assert_eq!(
            package_component_skill_files(&bundle),
            PackageComponentDiscovery::Invalid
        );

        fs::write(
            bundle.join(PACKAGE_RECORD_FILE),
            r#"{"id":"invalid","components":null}"#,
        )
        .unwrap();
        assert_eq!(
            package_component_skill_files(&bundle),
            PackageComponentDiscovery::Invalid
        );

        fs::write(bundle.join(PACKAGE_RECORD_FILE), "{not-json").unwrap();
        assert_eq!(
            package_component_skill_files(&bundle),
            PackageComponentDiscovery::Invalid
        );

        #[cfg(unix)]
        {
            fs::remove_file(bundle.join(PACKAGE_RECORD_FILE)).unwrap();
            std::os::unix::fs::symlink(
                bundle.join("missing-record.json"),
                bundle.join(PACKAGE_RECORD_FILE),
            )
            .unwrap();
            assert_eq!(
                package_component_skill_files(&bundle),
                PackageComponentDiscovery::Invalid
            );
        }
    }

    #[test]
    fn changing_only_the_authoritative_package_record_makes_the_catalog_stale() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        let package = root.join("package");
        write_skill(&root, "package/alpha", "alpha", "First");
        write_skill(&root, "package/beta", "beta", "Second");
        let record = package.join(PACKAGE_RECORD_FILE);
        fs::write(
            &record,
            serde_json::json!({"components": [
                {"name": "alpha", "directory": "alpha"}
            ]})
            .to_string(),
        )
        .unwrap();
        let roots = vec![root_at(&root, biorouter_root())];
        let catalog = SkillCatalog::scan(roots.clone(), 1);
        assert!(!catalog.is_stale(&roots));

        fs::write(
            &record,
            serde_json::json!({"components": [
                {"name": "beta", "directory": "beta"}
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&record)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::now() + std::time::Duration::from_secs(2)),
            )
            .unwrap();

        assert!(catalog.is_stale(&roots));
    }

    #[test]
    fn an_invalid_package_record_is_watched_until_it_is_repaired() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        let package = root.join("package");
        write_skill(&root, "package/alpha", "alpha", "First");
        let record = package.join(PACKAGE_RECORD_FILE);
        fs::write(&record, "{not-json").unwrap();
        let roots = vec![root_at(&root, biorouter_root())];
        let catalog = SkillCatalog::scan(roots.clone(), 1);
        assert!(catalog.skills().get("alpha").is_none());
        assert!(!catalog.is_stale(&roots));

        fs::write(
            &record,
            serde_json::json!({"components": [
                {"name": "alpha", "directory": "alpha"}
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&record)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::now() + std::time::Duration::from_secs(2)),
            )
            .unwrap();

        assert!(catalog.is_stale(&roots));
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_package_record_symlink_is_invalid_and_watched_for_repair() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        let package = root.join("package");
        write_skill(&root, "package/alpha", "alpha", "First");
        let target = package.join("repaired-record.json");
        let record = package.join(PACKAGE_RECORD_FILE);
        std::os::unix::fs::symlink(&target, &record).unwrap();
        let roots = vec![root_at(&root, biorouter_root())];
        let catalog = SkillCatalog::scan(roots.clone(), 1);
        assert!(catalog.skills().get("alpha").is_none());
        assert!(!catalog.is_stale(&roots));

        fs::write(
            target,
            serde_json::json!({"components": [
                {"name": "alpha", "directory": "alpha"}
            ]})
            .to_string(),
        )
        .unwrap();

        assert!(catalog.is_stale(&roots));
    }

    /// A bundle with no record is still a bundle. The importer is not a
    /// precondition for the two-level layout that predates it.
    #[test]
    fn a_hand_assembled_bundle_has_no_package_record_and_still_appears() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        write_skill(&root, "pack/alpha", "alpha", "First");

        let catalog = SkillCatalog::scan(vec![root_at(&root, biorouter_root())], 1);
        let view = catalog.view(&SessionSkillOverride::default());
        assert_eq!(authored_bundles(&view)[0].display_name, "pack");
        assert!(authored_bundles(&view)[0].package.is_none());
    }

    #[test]
    fn a_new_bundle_member_makes_the_snapshot_stale() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        write_skill(&root, "pack/alpha", "alpha", "First");
        let roots = vec![root_at(&root, biorouter_root())];

        let catalog = SkillCatalog::scan(roots.clone(), 1);
        assert!(!catalog.is_stale(&roots));

        // Creating `<bundle>/<child>/` bumps the BUNDLE's mtime, not the
        // root's — which is why bundle directories are watched too.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write_skill(&root, "pack/beta", "beta", "Second");
        assert!(catalog.is_stale(&roots));
    }

    #[test]
    fn an_added_root_makes_the_snapshot_stale_even_with_no_file_change() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("skills");
        write_skill(&root, "alpha", "alpha", "First");
        let roots = vec![root_at(&root, biorouter_root())];
        let catalog = SkillCatalog::scan(roots.clone(), 1);

        let extension = temp.path().join("extensions/New/skills");
        fs::create_dir_all(&extension).unwrap();
        let wider = vec![
            root_at(&root, biorouter_root()),
            root_at(
                &extension,
                SkillSource::new(SkillSourceKind::Extension, Some("New".to_string())),
            ),
        ];
        assert!(
            catalog.is_stale(&wider),
            "installing an extension adds a root, and that alone is staleness"
        );
    }
}
