//! The ONE workflow management core.
//!
//! Every surface that lists, resolves, enriches, validates, saves or deletes a
//! workflow comes here: the HTTP routes in `biorouter-server`, the CLI's
//! `/workflow` command and `biorouter run`, the desktop's "create workflow from
//! this chat", and the `platform__manage_workflow` tool the model calls. That is
//! the point of the module, and it is a correction rather than a tidy-up — four
//! divergences of exactly this kind had already accumulated between the two
//! surfaces that existed before it:
//!
//! * **create** — the HTTP route enriched a generated workflow with the live
//!   session's extensions, knowledge selection and author; the CLI called the
//!   same `Agent::create_workflow` and saved the un-enriched result.
//! * **save** — the CLI hand-rolled `serde_yaml::to_writer` into
//!   `./workflow.yaml`, bypassing the library directory, the slug/de-dup
//!   filename rule and the block-scalar formatting that
//!   [`local_workflows::save_workflow_to_file`] applies.
//! * **run** — the desktop and the scheduler go through
//!   [`crate::workflow::runtime::prepare_prompt`], which inlines declared
//!   skills' bodies and applies the knowledge selection; the CLI read
//!   `workflow.prompt` and `workflow.instructions` directly and silently dropped
//!   both keys.
//! * **schedule** — one route resolved a workflow by id, another took an
//!   arbitrary path.
//!
//! A new caller that reimplements any of the above is the fifth copy, and the
//! reason this module exists is that nothing catches the fifth copy: each
//! divergence compiles, each surface's own tests pass, and the two YAMLs differ
//! only when a user compares them.
//!
//! ## Where the crate boundary put this
//!
//! `biorouter-server` depends on `biorouter`; `biorouter` does not depend on the
//! server. So anything both a route and an agent tool need has to live **here**,
//! down in the core, not up in the routes. That is why [`WorkflowManifest`],
//! [`short_id_from_path`] and the id→path resolution moved down out of
//! `routes/workflow_utils.rs` rather than being re-exported sideways.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::Serialize;
use utoipa::ToSchema;

use crate::agents::extension::ExtensionConfig;
use crate::workflow::build_workflow::{build_workflow_from_template, WorkflowError};
use crate::workflow::local_workflows::{
    get_workflow_library_dir, list_local_workflows, save_workflow_to_file,
};
use crate::workflow::validate_workflow::validate_workflow_template_from_content;
use crate::workflow::{Workflow, WorkflowKnowledgeBases};

/// One workflow as every surface sees it: the parsed document, where it came
/// from, and the stable id the interfaces address it by.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkflowManifest {
    pub id: String,
    pub workflow: Workflow,
    #[schema(value_type = String)]
    pub file_path: PathBuf,
    pub last_modified: String,
    pub schedule_cron: Option<String>,
    pub slash_command: Option<String>,
}

/// The id a workflow is addressed by: a hash of its absolute path.
///
/// ⚠ Path-derived, so it is stable only while the file stays where it is.
/// Renaming a workflow changes its id, which is why [`resolve_id`] re-checks the
/// filesystem rather than trusting a cached answer.
pub fn short_id_from_path(path: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Every workflow on disk, newest first.
pub fn list_manifests() -> Result<Vec<WorkflowManifest>> {
    let mut manifests = Vec::new();
    for (file_path, workflow) in list_local_workflows()? {
        let Ok(last_modified) = fs::metadata(&file_path)
            .and_then(|m| m.modified())
            .map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            })
        else {
            continue;
        };
        manifests.push(WorkflowManifest {
            id: short_id_from_path(file_path.to_string_lossy().as_ref()),
            workflow,
            file_path,
            last_modified,
            schedule_cron: None,
            slash_command: None,
        });
    }
    manifests.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    Ok(manifests)
}

/// Attach the scheduler cron and slash-command bindings a manifest list does not
/// know about on its own.
///
/// Kept separate from [`list_manifests`] because those two facts live in the
/// scheduler and the slash-command registry, which a plain filesystem listing
/// has no handle on. The HTTP list route supplies them; the model's `list`
/// action supplies the slash commands and leaves cron to
/// `platform__manage_schedule`.
pub fn attach_bindings(
    manifests: &mut [WorkflowManifest],
    schedules: &HashMap<PathBuf, String>,
    slash_commands: &HashMap<PathBuf, String>,
) {
    for manifest in manifests.iter_mut() {
        manifest.schedule_cron = schedules.get(&manifest.file_path).cloned();
        manifest.slash_command = slash_commands.get(&manifest.file_path).cloned();
    }
}

// ---------------------------------------------------------------------------
// id -> path, with invalidation
// ---------------------------------------------------------------------------

/// The id→path map, rebuilt when the workflow roots change underneath it.
///
/// The predecessor of this cache lived on `AppState`, was wholesale-replaced
/// only by `GET /workflows/list`, and had **no invalidation at all**: a stale
/// entry pointing at a renamed or deleted file was served happily to delete,
/// schedule, slash-command and save. The skill catalog had the same class of bug
/// and fixed it with a snapshot plus mtime staleness plus an explicit
/// `invalidate()`; workflows never got the same treatment. This is that
/// treatment.
///
/// Two things make an entry stale, and both are checked:
///
/// * the **root set** changed, or any root's mtime moved (a file was added,
///   renamed or removed in it), and
/// * the resolved path no longer exists — checked on every hit, because mtime
///   has a one-second granularity and an in-process writer can land inside it.
struct Catalog {
    roots: Vec<(PathBuf, Option<SystemTime>)>,
    by_id: HashMap<String, PathBuf>,
}

fn catalog() -> &'static Mutex<Option<Catalog>> {
    static CATALOG: OnceLock<Mutex<Option<Catalog>>> = OnceLock::new();
    CATALOG.get_or_init(|| Mutex::new(None))
}

/// The roots a workflow may live in, with their current mtimes.
///
/// Taken from [`local_workflows::workflow_scan_roots`] rather than reconstructed
/// here: the root set is `BIOROUTER_WORKFLOW_PATH` + the global library + an
/// optionally opted-in working directory, and a fingerprint over a *guess* at
/// that set silently stops noticing changes in the roots it left out.
fn root_fingerprint() -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut roots = crate::workflow::local_workflows::workflow_scan_roots();
    roots.sort();
    roots.dedup();
    roots
        .into_iter()
        .map(|root| {
            let mtime = fs::metadata(&root).and_then(|m| m.modified()).ok();
            (root, mtime)
        })
        .collect()
}

/// Drop the cached id→path map.
///
/// Call this from any in-process writer — save, delete, import. mtime has a
/// one-second window, so a writer that saves and immediately resolves would
/// otherwise read its own stale snapshot back.
pub fn invalidate() {
    if let Ok(mut guard) = catalog().lock() {
        *guard = None;
    }
}

fn rebuild_locked(guard: &mut Option<Catalog>) -> Result<()> {
    // ⚠ Fingerprint FIRST, scan second. Taken afterwards it would record the
    // mtime of a tree that changed *during* the scan, and the entry that
    // resulted — a fresh-looking stamp over a map missing whatever landed
    // mid-walk — would then satisfy every staleness check until something else
    // touched the same root. Stamping before means such a change leaves the
    // fingerprint OLD, so the next call rebuilds. The failure directions are
    // not symmetric: one rescans a directory, the other hides a workflow.
    let roots = root_fingerprint();
    let by_id = list_manifests()?
        .into_iter()
        .map(|manifest| (manifest.id, manifest.file_path))
        .collect();
    *guard = Some(Catalog { roots, by_id });
    Ok(())
}

/// Resolve a workflow id to the file that currently holds it.
///
/// A hit whose file has since disappeared is treated as a miss and forces one
/// rebuild, so a deleted or renamed workflow reports "not found" rather than
/// handing a mutating caller a path to nothing.
///
/// ⚠ A MISS forces one rebuild too, and for the mirror-image reason. mtime has
/// one-second granularity, so a workflow written by another process inside the
/// same second as the cached fingerprint leaves the entry looking fresh while
/// the map has never seen the file — and "not found" from a cache is
/// indistinguishable from "not found" in the tree. The hit path already paid a
/// rescan to avoid answering from a stale snapshot; answering a NEGATIVE from
/// one is the same mistake with a worse symptom, because it names a workflow
/// that does exist as one that does not. The cost is a directory walk on a
/// query that was going to fail anyway.
pub fn resolve_id(id: &str) -> Result<PathBuf> {
    let mut guard = catalog()
        .lock()
        .map_err(|_| anyhow::anyhow!("workflow catalog lock poisoned"))?;

    let fresh = guard
        .as_ref()
        .is_some_and(|cached| cached.roots == root_fingerprint());
    if !fresh {
        rebuild_locked(&mut guard)?;
    }

    let hit = guard.as_ref().and_then(|c| c.by_id.get(id)).cloned();
    if let Some(path) = hit.filter(|path| path.exists()) {
        return Ok(path);
    }
    // Neither a live hit nor a fresh map. Rebuild once — for a cached path that
    // no longer resolves AND for a miss — so the caller sees today's tree
    // rather than the snapshot's. `fresh` is the guard because the only way to
    // arrive here without having just rebuilt is to have passed the freshness
    // check.
    if fresh {
        rebuild_locked(&mut guard)?;
    }

    match guard.as_ref().and_then(|c| c.by_id.get(id)) {
        Some(path) if path.exists() => Ok(path.clone()),
        _ => Err(anyhow::anyhow!("Workflow not found: {id}")),
    }
}

/// Every id currently known, for an error message that can name the choices.
pub fn known_ids() -> Vec<String> {
    list_manifests()
        .map(|manifests| manifests.into_iter().map(|m| m.id).collect())
        .unwrap_or_default()
}

/// Load a workflow by the id the interfaces address it by.
pub fn load_by_id(id: &str) -> Result<Workflow> {
    let path = resolve_id(id)?;
    Workflow::from_file_path(&path).with_context(|| format!("Failed to load workflow: {id}"))
}

/// Resolve a workflow by id **or** by title, case-insensitively.
///
/// The model names a workflow the way the user does — by its title — while the
/// interfaces address it by id. Accepting both is what makes "delete the weekly
/// report workflow" work without a preceding `list` call. An ambiguous title is
/// an error naming every candidate rather than a coin flip.
pub fn resolve_reference(reference: &str) -> Result<WorkflowManifest> {
    let manifests = list_manifests()?;
    if let Some(found) = manifests.iter().find(|m| m.id == reference) {
        return Ok(found.clone());
    }

    let needle = reference.trim().to_lowercase();
    let by_title: Vec<&WorkflowManifest> = manifests
        .iter()
        .filter(|m| m.workflow.title.trim().to_lowercase() == needle)
        .collect();
    match by_title.as_slice() {
        [single] => return Ok((*single).clone()),
        [] => {}
        many => {
            let ids: Vec<String> = many
                .iter()
                .map(|m| format!("{} ({})", m.id, m.file_path.display()))
                .collect();
            return Err(anyhow::anyhow!(
                "{} workflows are titled '{reference}'. Name one by id: {}",
                many.len(),
                ids.join(", ")
            ));
        }
    }

    // A path is the third form the CLI and the scheduler already accept.
    let as_path = PathBuf::from(reference);
    if as_path.is_file() {
        if let Some(found) = manifests.iter().find(|m| m.file_path == as_path) {
            return Ok(found.clone());
        }
    }

    let known: Vec<String> = manifests
        .iter()
        .take(20)
        .map(|m| format!("{} ({})", m.workflow.title, m.id))
        .collect();
    Err(anyhow::anyhow!(
        "No workflow matches '{reference}'. Known workflows: {}",
        if known.is_empty() {
            "none".to_string()
        } else {
            known.join(", ")
        }
    ))
}

// ---------------------------------------------------------------------------
// validate
// ---------------------------------------------------------------------------

/// Validate a workflow the way every surface must.
///
/// Serializes and re-parses through the template validator, so a workflow that
/// passes here is one the runtime can actually build.
pub fn validate(workflow: &Workflow) -> Result<()> {
    let yaml = workflow
        .to_yaml()
        .context("Failed to serialize workflow for validation")?;
    validate_workflow_template_from_content(&yaml, None)
        .map(|_| ())
        .map_err(|err| anyhow::anyhow!("{err}"))
}

/// Drop the parameters no part of the document refers to, and return their keys.
///
/// [`validate`] refuses a parameter that appears in no `{{ key }}` anywhere in
/// the document ("Unnecessary parameter definitions"), and separately refuses an
/// `optional` parameter with no `default` ("Optional parameters missing default
/// values"). Both rules are right on their own, and together they leave an
/// unreferenced `optional` parameter with no accepted form at all: removing its
/// default trades one refusal for the other. A generator that emits one
/// therefore produces a workflow the user cannot save by any route — which is
/// what "Create workflow from this chat" did, because `workflow.md` asks for
/// parameters and for `{{ key }}` references as two separate instructions and a
/// model routinely obeys only the first.
///
/// So the generated document is normalised instead of the rules being relaxed. A
/// parameter nothing refers to is a value the run would collect and discard, and
/// dropping it is the same stance `Agent::create_workflow` already takes towards
/// a parameter list that does not parse: keep the expensive part, lose the part
/// that cannot work, say so in the log.
///
/// ⚠ **The reference set is read the way [`validate`] reads it** — every
/// `{{ … }}` in the serialized document, not just the ones in `prompt` and
/// `instructions` — so the two can never disagree about what "referenced"
/// means. A serialization failure drops nothing: an unpruned document still has
/// a chance of being valid, and a silently emptied `parameters` list does not.
pub fn drop_unreferenced_parameters(workflow: &mut Workflow) -> Vec<String> {
    let Some(parameters) = workflow.parameters.as_ref() else {
        return Vec::new();
    };
    if parameters.is_empty() {
        return Vec::new();
    }

    let yaml = match workflow.to_yaml() {
        Ok(yaml) => yaml,
        Err(err) => {
            tracing::warn!(
                "Keeping the generated parameters: the workflow would not serialize \
                 for a reference check: {err}"
            );
            return Vec::new();
        }
    };
    let referenced = match crate::workflow::template_workflow::parse_workflow_content(&yaml, None) {
        Ok((_, variables)) => variables,
        Err(err) => {
            tracing::warn!(
                "Keeping the generated parameters: the workflow would not parse \
                 for a reference check: {err}"
            );
            return Vec::new();
        }
    };

    let mut dropped = Vec::new();
    let kept: Vec<_> = parameters
        .iter()
        .filter(|parameter| {
            if referenced.contains(&parameter.key) {
                return true;
            }
            dropped.push(parameter.key.clone());
            false
        })
        .cloned()
        .collect();

    if dropped.is_empty() {
        return dropped;
    }
    tracing::warn!(
        "Dropping generated workflow parameters the document never refers to: {}",
        dropped.join(", ")
    );
    // `None`, not an empty list: `skip_serializing_if` keeps an empty `Vec` out
    // of the YAML anyway, and `None` is what every other "there is nothing to
    // say" path in this module means by it.
    workflow.parameters = if kept.is_empty() { None } else { Some(kept) };
    dropped
}

/// Substitute parameter values into a workflow template.
///
/// `Ok(None)` means required parameters are still missing — the caller is
/// expected to ask for them rather than treat it as a failure.
pub fn build_with_parameter_values(
    original: &Workflow,
    values: HashMap<String, String>,
) -> Result<Option<Workflow>> {
    let content = original.to_yaml()?;
    let dir = get_workflow_library_dir(true);
    match build_workflow_from_template(
        content,
        &dir,
        values.into_iter().collect(),
        None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
    ) {
        Ok(workflow) => Ok(Some(workflow)),
        Err(WorkflowError::MissingParams { .. }) => Ok(None),
        Err(err) => Err(anyhow::anyhow!(err)),
    }
}

// ---------------------------------------------------------------------------
// enrichment
// ---------------------------------------------------------------------------

/// What a generated workflow gains from the session it was generated in.
///
/// The generator writes prose; these three fields are facts about the live
/// session that no model should be guessing. They were computed inside the HTTP
/// handler, which is precisely why the CLI's copy of the same flow produced a
/// different document from the same conversation.
pub struct SessionEnrichment {
    /// Extensions read off the live agent, with their canonical descriptions
    /// restored.
    pub extensions: Vec<ExtensionConfig>,
    /// The session's knowledge selection, or `None` when there is nothing to say.
    pub knowledge_bases: Option<WorkflowKnowledgeBases>,
    /// Author details supplied by the caller (the desktop passes the user's).
    pub author: Option<crate::workflow::Author>,
}

/// Gather everything a live session contributes to a workflow generated from it.
///
/// ⚠ The ONE place these three facts are collected. Both the HTTP create route
/// and the CLI's `/workflow` command call this and then
/// [`apply_session_enrichment`]; before it existed only the route enriched, so
/// the same conversation produced a different document depending on which
/// surface asked. `create_from_session_produces_the_same_document_on_every_surface`
/// is the assertion that keeps them together.
pub async fn session_enrichment(
    agent: &crate::agents::Agent,
    knowledge: &biorouter_mcp::knowledge::service::KnowledgeService,
    session_id: &str,
    author: Option<crate::workflow::Author>,
) -> Result<SessionEnrichment> {
    let extensions = agent
        .get_extension_configs()
        .await
        .into_iter()
        // ⚠ REDACT BEFORE ENRICH, and never the other way round.
        //
        // A generated workflow leaves the machine twice, and both exits are
        // silent. `generate` returns the rendered YAML as TOOL CONTENT, so the
        // whole document is appended to the conversation and shipped to the
        // model provider on the next request — including a `claude_code` or
        // `codex` child on a consumer plan with no BAA. And `save` writes it in
        // plaintext to the workflow library, which is the thing users mail to
        // each other.
        //
        // The live config carries resolved connector auth: `StreamableHttp`
        // holds `headers` (a `Bearer` token typed into Settings -> Extensions is
        // stored verbatim, never keyring-migrated), `Stdio` holds `envs`, and
        // both hold locators that can embed credentials in userinfo. None of it
        // is needed to re-enable an extension: a consumer matches by NAME
        // against its own installed set, which is the only thing that could
        // work on another machine anyway.
        //
        // This is the same projection `session_manager` and `extension_data`
        // already apply when a session is exported, for the same reason. Doing
        // it here rather than at each call site is what makes it hold for the
        // HTTP route, the CLI and the tool at once.
        .map(|config| config.redacted_for_session_export())
        .map(enrich_extension_description)
        .collect();

    Ok(SessionEnrichment {
        extensions,
        knowledge_bases: knowledge_bases_for_session(knowledge, session_id)?,
        author,
    })
}

/// Fold a session's facts into a freshly generated workflow.
///
/// ⚠ The ONLY enrichment. A route, a CLI command or a tool that sets
/// `workflow.extensions` itself is the divergence this module exists to remove —
/// `only_the_core_writes_a_generated_workflows_extensions` greps for exactly
/// that. (It was cited here under a name no test has ever had, which is the
/// failure mode of naming a guard in prose: the citation looks like evidence.)
pub fn apply_session_enrichment(workflow: &mut Workflow, enrichment: SessionEnrichment) {
    if !enrichment.extensions.is_empty() {
        workflow.extensions = Some(enrichment.extensions);
    }
    if workflow.knowledge_bases.is_none() {
        workflow.knowledge_bases = enrichment.knowledge_bases;
    }
    if enrichment.author.is_some() {
        workflow.author = enrichment.author;
    }
}

/// Restore an extension's canonical description when the live config carries an
/// empty or self-named one.
///
/// A workflow's `extensions` block is read by a human deciding whether to trust
/// it, so `description: developer` tells them nothing.
pub fn enrich_extension_description(mut config: ExtensionConfig) -> ExtensionConfig {
    if !needs_extension_description_enrichment(&config) {
        return config;
    }

    let name = config.name();
    if let Some(canonical) = crate::config::get_extension_by_name(&name) {
        let description = extension_description(&canonical).trim().to_string();
        if !description.is_empty() && description != name {
            set_extension_description(&mut config, description);
            return config;
        }
    }

    if let Some(def) = crate::agents::extension::PLATFORM_EXTENSIONS
        .get(crate::config::extensions::name_to_key(&name).as_str())
    {
        set_extension_description(&mut config, def.description.to_string());
    }

    config
}

pub fn extension_description(config: &ExtensionConfig) -> &str {
    match config {
        ExtensionConfig::Sse { description, .. }
        | ExtensionConfig::Stdio { description, .. }
        | ExtensionConfig::Builtin { description, .. }
        | ExtensionConfig::Platform { description, .. }
        | ExtensionConfig::StreamableHttp { description, .. }
        | ExtensionConfig::Frontend { description, .. }
        | ExtensionConfig::InlinePython { description, .. } => description,
    }
}

fn set_extension_description(config: &mut ExtensionConfig, value: String) {
    match config {
        ExtensionConfig::Sse { description, .. }
        | ExtensionConfig::Stdio { description, .. }
        | ExtensionConfig::Builtin { description, .. }
        | ExtensionConfig::Platform { description, .. }
        | ExtensionConfig::StreamableHttp { description, .. }
        | ExtensionConfig::Frontend { description, .. }
        | ExtensionConfig::InlinePython { description, .. } => *description = value,
    }
}

fn needs_extension_description_enrichment(config: &ExtensionConfig) -> bool {
    let description = extension_description(config).trim();
    description.is_empty() || description == config.name()
}

/// Capture a session's knowledge selection into the workflow being authored.
///
/// `None` means "this workflow has nothing to say about knowledge bases", which
/// is different from "it selects none": the runtime only touches the session's
/// selection when the key is present.
pub fn knowledge_bases_for_session(
    service: &biorouter_mcp::knowledge::service::KnowledgeService,
    session_id: &str,
) -> Result<Option<WorkflowKnowledgeBases>> {
    let bases = service
        .list_bases()
        .context("listing knowledge bases for workflow creation")?;
    if bases.is_empty() {
        return Ok(None);
    }

    // One locked snapshot rather than three unlocked reads. `selection` also
    // knows the difference between a session that never chose a primary (which
    // inherits the machine pointer) and one that explicitly holds none (which
    // must not).
    let selection = service
        .selection(Some(session_id))
        .context("reading the session knowledge selection for workflow creation")?;

    Ok(Some(WorkflowKnowledgeBases {
        default: selection.primary_kb,
        visible: selection.kb_ids,
    }))
}

// ---------------------------------------------------------------------------
// save / delete
// ---------------------------------------------------------------------------

/// Where a save should land.
pub enum SaveTarget {
    /// The user's workflow library, under a slug derived from the title with the
    /// de-dup suffix rule.
    Library,
    /// An explicit path. Used by the CLI's `/workflow <path>` and by tests.
    Path(PathBuf),
    /// Overwrite the file an existing workflow already occupies.
    ExistingId(String),
}

/// Save a workflow, through the one writer.
///
/// Validates first — a workflow that cannot be re-parsed is not written, because
/// the next `list` would drop it silently and the user would be told the save
/// succeeded.
pub fn save(workflow: &Workflow, target: SaveTarget) -> Result<PathBuf> {
    validate(workflow).context("refusing to save a workflow that does not validate")?;

    let path = match target {
        SaveTarget::Library => None,
        SaveTarget::Path(path) => Some(absolutize(path)?),
        SaveTarget::ExistingId(id) => Some(resolve_id(&id)?),
    };

    let written = save_workflow_to_file(workflow.clone(), path)?;
    invalidate();
    Ok(written)
}

fn absolutize(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    Ok(cwd.join(path))
}

/// Delete a workflow file.
pub fn delete(id: &str) -> Result<PathBuf> {
    let path = resolve_id(id)?;
    fs::remove_file(&path).with_context(|| format!("Failed to delete {}", path.display()))?;
    invalidate();
    Ok(path)
}

/// Read a workflow file's raw bytes as text, for a caller that wants the YAML
/// rather than the parsed document.
pub fn read_yaml(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Workflow {
        Workflow::builder()
            .title("Weekly Report")
            .description("a test workflow")
            .instructions("do the thing")
            .build()
            .unwrap()
    }

    #[test]
    fn an_id_is_derived_from_the_path_and_is_stable() {
        let a = short_id_from_path("/tmp/one.yaml");
        assert_eq!(a, short_id_from_path("/tmp/one.yaml"));
        assert_ne!(a, short_id_from_path("/tmp/two.yaml"));
        assert_eq!(a.len(), 16, "the id is a fixed-width hex hash");
    }

    /// A workflow that cannot be re-parsed is never written.
    ///
    /// Without this the save reports success and the next `list` silently omits
    /// the file — the user is told the opposite of what happened.
    #[test]
    fn a_save_refuses_a_workflow_that_does_not_validate() {
        let mut workflow = sample();
        workflow.title = String::new();
        workflow.description = String::new();
        workflow.instructions = None;
        workflow.prompt = None;

        let dir = tempfile::tempdir().unwrap();
        let target = SaveTarget::Path(dir.path().join("broken.yaml"));
        let err = save(&workflow, target).expect_err("an invalid workflow must not be written");
        assert!(
            err.to_string().contains("does not validate"),
            "the refusal must say why: {err}"
        );
        assert!(
            !dir.path().join("broken.yaml").exists(),
            "nothing may be written when validation fails"
        );
    }

    #[test]
    fn a_valid_workflow_round_trips_through_an_explicit_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("weekly.yaml");
        let written = save(&sample(), SaveTarget::Path(path.clone())).unwrap();
        assert_eq!(written, path);

        let text = read_yaml(&path).unwrap();
        assert!(text.contains("Weekly Report"));
        let reloaded = Workflow::from_file_path(&path).unwrap();
        assert_eq!(reloaded.title, "Weekly Report");
    }

    /// The enrichment must not clobber a knowledge selection the generator
    /// already produced, and must not invent an author.
    #[test]
    fn enrichment_fills_gaps_without_overwriting_what_is_already_there() {
        let mut workflow = sample();
        workflow.knowledge_bases = Some(WorkflowKnowledgeBases {
            default: Some("chosen".into()),
            visible: vec!["chosen".into()],
        });

        apply_session_enrichment(
            &mut workflow,
            SessionEnrichment {
                extensions: Vec::new(),
                knowledge_bases: Some(WorkflowKnowledgeBases {
                    default: Some("session".into()),
                    visible: vec!["session".into()],
                }),
                author: None,
            },
        );

        assert_eq!(
            workflow
                .knowledge_bases
                .as_ref()
                .unwrap()
                .default
                .as_deref(),
            Some("chosen"),
            "a selection the workflow already declares wins over the session's"
        );
        assert!(
            workflow.extensions.is_none(),
            "an empty extension list must not overwrite with an empty Some"
        );
        assert!(workflow.author.is_none());
    }

    #[test]
    fn enrichment_backfills_an_absent_knowledge_selection_and_the_author() {
        let mut workflow = sample();
        apply_session_enrichment(
            &mut workflow,
            SessionEnrichment {
                extensions: Vec::new(),
                knowledge_bases: Some(WorkflowKnowledgeBases {
                    default: Some("session".into()),
                    visible: vec!["session".into()],
                }),
                author: Some(crate::workflow::Author {
                    contact: Some("someone".into()),
                    metadata: None,
                }),
            },
        );
        assert_eq!(
            workflow.knowledge_bases.unwrap().default.as_deref(),
            Some("session")
        );
        assert_eq!(workflow.author.unwrap().contact.as_deref(), Some("someone"));
    }

    /// A description that merely repeats the extension's own name carries no
    /// information, so the canonical one replaces it; a real description is left
    /// alone.
    #[test]
    fn only_an_empty_or_self_named_description_is_replaced() {
        let self_named = ExtensionConfig::Builtin {
            name: "developer".into(),
            display_name: None,
            description: "developer".into(),
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        };
        assert!(needs_extension_description_enrichment(&self_named));

        let real = ExtensionConfig::Builtin {
            name: "developer".into(),
            display_name: None,
            description: "Edit files and run shell commands".into(),
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        };
        assert!(!needs_extension_description_enrichment(&real));
        assert_eq!(
            extension_description(&enrich_extension_description(real)),
            "Edit files and run shell commands"
        );
    }

    /// `invalidate` must actually clear the snapshot. Without it an in-process
    /// save inside mtime's one-second window resolves against the pre-save map —
    /// the stale-cache bug this module replaced, reproduced one layer down.
    #[test]
    fn invalidate_clears_the_cached_snapshot() {
        {
            let mut guard = catalog().lock().unwrap();
            *guard = Some(Catalog {
                roots: Vec::new(),
                by_id: HashMap::from([("deadbeef".to_string(), PathBuf::from("/nope.yaml"))]),
            });
        }
        invalidate();
        assert!(
            catalog().lock().unwrap().is_none(),
            "invalidate must drop the snapshot, not merely mark it"
        );
    }

    /// A cached id whose file has since vanished must not be handed to a
    /// mutating caller.
    #[test]
    fn a_cached_id_whose_file_is_gone_is_a_miss() {
        {
            let mut guard = catalog().lock().unwrap();
            *guard = Some(Catalog {
                // Matching the live fingerprint keeps the freshness check happy,
                // so the only thing that can reject this entry is the existence
                // check on the hit itself — which is the point of the test.
                roots: root_fingerprint(),
                by_id: HashMap::from([(
                    "ffffffffffffffff".to_string(),
                    PathBuf::from("/definitely/not/here.yaml"),
                )]),
            });
        }
        let err = resolve_id("ffffffffffffffff")
            .expect_err("a path that no longer exists must not resolve");
        assert!(err.to_string().contains("not found"), "{err}");
        invalidate();
    }

    /// A MISS forces one rescan, exactly as a dead hit does.
    ///
    /// mtime has one-second granularity, so a workflow written by another
    /// process inside the same second as the cached fingerprint leaves the
    /// entry looking fresh while the map has never seen the file — and the
    /// caller is told "Workflow not found" about a workflow that exists. The
    /// hit path already paid a rescan to avoid answering from a stale snapshot;
    /// answering a NEGATIVE from one is the same mistake with a worse symptom.
    ///
    /// Asserted through the catalog rather than through a file, because the
    /// roots are the user's real library plus `BIOROUTER_WORKFLOW_PATH` and
    /// mutating that env var in a parallel test binary is a race, not a test.
    /// The sentinel is an id no scan can ever produce, so its disappearance is
    /// proof that the map was rebuilt and not merely consulted.
    #[test]
    fn a_miss_against_a_fresh_looking_catalog_still_rescans() {
        const SENTINEL: &str = "sentinel-no-scan-can-mint-this";
        {
            let mut guard = catalog().lock().unwrap();
            *guard = Some(Catalog {
                // Matching the live fingerprint is the whole point: the
                // freshness check must pass, so the only thing that can rebuild
                // is the miss itself.
                roots: root_fingerprint(),
                by_id: HashMap::from([(
                    SENTINEL.to_string(),
                    PathBuf::from("/definitely/not/here.yaml"),
                )]),
            });
        }

        let _ = resolve_id("an-id-that-is-in-no-library");

        let survived = catalog()
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|cached| cached.by_id.contains_key(SENTINEL));
        invalidate();
        assert!(
            !survived,
            "the catalog was not rebuilt, so a workflow written inside mtime's one-second \
             window would report as missing until something else touched the root"
        );
    }
}

// The knowledge-capture tests moved down here with the function they cover.
// They lived in `routes/workflow.rs` while the capture did; leaving them behind
// would have left the server crate asserting behaviour it no longer owns.
#[cfg(test)]
mod knowledge_capture_tests {
    use super::knowledge_bases_for_session;
    use crate::workflow::Workflow;
    use biorouter_mcp::knowledge::service::{KnowledgeService, PrimaryUpdate};

    fn service_with(ids: &[&str]) -> (tempfile::TempDir, KnowledgeService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        for id in ids {
            svc.create_base(id, id, None).unwrap();
        }
        (dir, svc)
    }

    /// "Every base hidden" is a *stated* selection, not an absent one. Captured
    /// as `None` it read as "the author had no opinion", so replay skipped the
    /// selection write entirely and the new session inherited whatever the
    /// replaying machine's defaults were — the opposite of what was authored.
    #[test]
    fn an_empty_visible_set_is_captured_not_dropped() {
        let (_d, svc) = service_with(&["alpha", "beta"]);
        svc.set_visible_kbs(Some("s1"), &[], PrimaryUpdate::Unchanged)
            .unwrap();

        let captured = knowledge_bases_for_session(&svc, "s1")
            .unwrap()
            .expect("a session that hid every base still has a selection to state");
        assert!(
            captured.visible.is_empty(),
            "the authored set was empty; capture must say so"
        );
        assert_eq!(captured.default, None);

        // Capturing it is only half the fix: both fields are
        // `skip_serializing_if`, so the empty selection serializes to a bare
        // `{}` and would be indistinguishable from an absent `knowledge_bases`
        // if the OUTER field ever gained a "skip if empty" rule. Replay reads
        // `Option::is_none` to decide whether to touch the session at all, so
        // pin the round trip here — a saved workflow that loses the empty set
        // on the way to disk defeats the capture silently.
        let mut workflow = Workflow::builder()
            .title("t")
            .description("d")
            .instructions("i")
            .build()
            .unwrap();
        workflow.knowledge_bases = Some(captured);
        let yaml = serde_yaml::to_string(&workflow).unwrap();
        let round_tripped: Workflow = serde_yaml::from_str(&yaml).unwrap();
        let replayed = round_tripped
            .knowledge_bases
            .expect("an explicitly empty set must survive being saved and reloaded");
        assert!(replayed.visible.is_empty());
        assert_eq!(replayed.default, None);
    }

    /// The one case with genuinely nothing to say: no bases exist at all, so
    /// there is no set to describe and replay should leave the target session
    /// alone.
    #[test]
    fn a_machine_with_no_bases_captures_nothing() {
        let (_d, svc) = service_with(&[]);
        assert!(knowledge_bases_for_session(&svc, "s1").unwrap().is_none());
    }

    /// A session that explicitly holds *no* primary must not be captured as
    /// holding the machine-wide one. `get_primary_for_session` collapses
    /// "never chose" and "chose nothing" into the same `None`, so the
    /// hand-rolled `.or_else(get_primary_persisted)` fallback could not tell
    /// them apart and handed the workflow a default the session had rejected —
    /// which replay would then arm as the KB-less write target.
    #[test]
    fn an_explicit_no_primary_is_not_backfilled_from_the_machine() {
        let (_d, svc) = service_with(&["alpha", "beta"]);
        svc.set_primary_persisted(Some("alpha")).unwrap();
        svc.set_selection(Some("s1"), None, PrimaryUpdate::Clear)
            .unwrap();

        let captured = knowledge_bases_for_session(&svc, "s1")
            .unwrap()
            .expect("bases exist, so there is a selection");
        assert_eq!(
            captured.default, None,
            "the session said it has no primary; the machine's is not a substitute"
        );
        assert_eq!(
            captured.visible,
            vec!["alpha".to_string(), "beta".to_string()],
            "clearing the primary must not narrow the set"
        );
    }

    /// A session that has simply never chosen still follows the machine
    /// pointer — the fallback itself is correct, only its blindness was not.
    #[test]
    fn a_session_that_never_chose_still_inherits_the_machine_primary() {
        let (_d, svc) = service_with(&["alpha", "beta"]);
        svc.set_primary_persisted(Some("alpha")).unwrap();

        let captured = knowledge_bases_for_session(&svc, "s1")
            .unwrap()
            .expect("bases exist, so there is a selection");
        assert_eq!(captured.default.as_deref(), Some("alpha"));
    }

    /// The ordinary path still round-trips the set and its primary.
    #[test]
    fn a_narrowed_session_captures_its_set_and_primary() {
        let (_d, svc) = service_with(&["alpha", "beta", "gamma"]);
        svc.set_visible_kbs(
            Some("s1"),
            &["alpha".to_string(), "beta".to_string()],
            PrimaryUpdate::Set("beta"),
        )
        .unwrap();

        let captured = knowledge_bases_for_session(&svc, "s1")
            .unwrap()
            .expect("a session with bases has a selection");
        assert_eq!(
            captured.visible,
            vec!["alpha".to_string(), "beta".to_string()]
        );
        assert_eq!(captured.default.as_deref(), Some("beta"));
    }
}

/// The repo-wide guards that keep "one core" true.
///
/// Every assertion here is a grep over the real tree rather than over a fixture,
/// because the failure being prevented is a *new call site* — something a
/// fixture cannot contain by construction. Each one also fails loudly when it
/// can read nothing, so a moved file turns the guard off with a failure rather
/// than a vacuous pass.
#[cfg(test)]
mod one_core_guards {
    use std::path::{Path, PathBuf};

    fn crates_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/biorouter has a parent")
            .to_path_buf()
    }

    /// Every `.rs` file in the workspace, with its path.
    fn rust_sources() -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "target") {
                        continue;
                    }
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        out.push((path, text));
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&crates_dir(), &mut out);
        assert!(
            out.len() > 100,
            "the guard must not pass vacuously: only {} sources were read",
            out.len()
        );
        out
    }

    /// Production halves only. A guard that also reads `#[cfg(test)]` code
    /// counts a test's own fixture as a violation, and one that slices on the
    /// first `#[cfg(test)]` truncates a file whose tests sit mid-way and then
    /// misses everything after them. Splitting on the LAST occurrence keeps the
    /// whole production body in view in both layouts.
    fn production(text: &str) -> String {
        // ⚠ `rsplit_once`, not `split_once`: the LAST `#[cfg(test)]`, so a file
        // whose tests sit mid-way (this one does) keeps its whole production
        // body in view. Splitting on the first truncates everything after an
        // early test module and invents a clean bill of health for it.
        //
        // `rsplit_once` rather than `rfind` + slice because the slice form trips
        // `clippy::string_slice`, which this repo denies; the index is a char
        // boundary either way, both needles being ASCII.
        let body = text
            .rsplit_once("#[cfg(test)]")
            .map_or(text, |(before, _)| before);
        // Comments are dropped, and that is not cosmetic: every one of these
        // guards forbids a pattern, so the prose explaining WHY it is forbidden
        // names it — and a guard that reads its own rationale as a violation
        // reports the fix as the bug. Both of these fired on their own doc
        // comments before this line existed.
        body.lines()
            .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rel(path: &Path) -> String {
        path.strip_prefix(crates_dir())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// The catalog's freshness stamp is taken BEFORE the scan it describes.
    ///
    /// Taken after, it records the mtime of a tree that changed *during* the
    /// walk: a fresh-looking stamp over a map missing whatever landed mid-scan,
    /// which then satisfies every staleness check until something else touches
    /// the same root. Stamped first, such a change leaves the fingerprint old
    /// and the next call rebuilds. The two failure directions are not
    /// symmetric — one rescans a directory, the other hides a workflow — so the
    /// order is the fix and cannot be tested from the outside without racing a
    /// real filesystem against a real scan.
    #[test]
    fn the_catalog_fingerprint_is_taken_before_the_scan() {
        let text = std::fs::read_to_string(crates_dir().join("biorouter/src/workflow/service.rs"))
            .expect("service.rs is readable");
        let body = production(&text);
        let rebuild = body
            .split_once("fn rebuild_locked")
            .expect("rebuild_locked exists")
            .1;
        let rebuild = rebuild
            .split_once("\n}\n")
            .map_or(rebuild, |(inside, _)| inside);

        let stamp = rebuild
            .find("root_fingerprint()")
            .expect("rebuild_locked must take a fingerprint");
        let scan = rebuild
            .find("list_manifests()")
            .expect("rebuild_locked must scan for manifests");
        assert!(
            stamp < scan,
            "`root_fingerprint()` must be called before `list_manifests()`, or the cache \
             records a stamp newer than the map it stamps. Body seen:\n{rebuild}"
        );
    }

    /// A generated workflow's `extensions` are written in ONE place.
    ///
    /// The HTTP route used to set this field inline while the CLI, running the
    /// same generator over the same conversation, set nothing — the divergence
    /// this module exists to end. A second assignment is how it comes back.
    #[test]
    fn only_the_core_writes_a_generated_workflows_extensions() {
        let offenders: Vec<String> = rust_sources()
            .into_iter()
            .filter(|(path, _)| !rel(path).ends_with("workflow/service.rs"))
            .filter(|(_, text)| production(text).contains("workflow.extensions = Some("))
            .map(|(path, _)| rel(&path))
            .collect();
        assert!(
            offenders.is_empty(),
            "`workflow.extensions` must only be set by \
             `workflow::service::apply_session_enrichment`; also set in: {offenders:?}"
        );
    }

    /// `save_workflow_to_file` is the physical writer, and only the core may
    /// call it.
    ///
    /// Everything a caller wants around a save — validation, the library
    /// directory, the de-dup filename, cache invalidation — lives in
    /// [`super::save`]. The CLI called the writer directly (in fact it did not
    /// even do that: it called `serde_yaml::to_writer`) and got none of it.
    #[test]
    fn only_the_core_calls_the_physical_workflow_writer() {
        let sources = rust_sources();
        let definition_seen = sources
            .iter()
            .any(|(path, _)| rel(path).ends_with("workflow/local_workflows.rs"));
        assert!(
            definition_seen,
            "the guard must not pass vacuously: local_workflows.rs was not read"
        );

        let offenders: Vec<String> = sources
            .into_iter()
            .filter(|(path, _)| {
                let rel = rel(path);
                !rel.ends_with("workflow/service.rs")
                    && !rel.ends_with("workflow/local_workflows.rs")
            })
            .filter(|(_, text)| production(text).contains("save_workflow_to_file("))
            .map(|(path, _)| rel(&path))
            .collect();
        assert!(
            offenders.is_empty(),
            "workflows are written through `workflow::service::save`; \
             `save_workflow_to_file` is also called in: {offenders:?}"
        );
    }

    /// No surface may serialize a workflow straight to a file handle.
    ///
    /// This is the shape the CLI's save actually had, and it is invisible to the
    /// guard above because it never names the writer at all.
    #[test]
    fn no_surface_serializes_a_workflow_directly_to_a_file() {
        let offenders: Vec<String> = rust_sources()
            .into_iter()
            .filter(|(_, text)| {
                let production = production(text);
                production.contains("serde_yaml::to_writer")
                    && production.to_lowercase().contains("workflow")
            })
            .map(|(path, _)| rel(&path))
            .collect();
        assert!(
            offenders.is_empty(),
            "a workflow must be written through `workflow::service::save`, which \
             validates it and applies the block-scalar formatting; direct \
             serialization in: {offenders:?}"
        );
    }

    /// Every surface that starts a workflow installs it the same way.
    ///
    /// `prepare_prompt` is what inlines a workflow's declared skills and renders
    /// its instructions; `install_prepared` is what applies its knowledge
    /// selection. A surface that reads `workflow.instructions` and hands it
    /// straight to the model has silently dropped both `skills:` and
    /// `knowledge_bases:` — which is exactly what `biorouter run --workflow`
    /// did.
    #[test]
    fn every_surface_that_runs_a_workflow_goes_through_the_shared_install() {
        let sources = rust_sources();
        let installers: Vec<String> = sources
            .iter()
            .filter(|(_, text)| production(text).contains("runtime::install_prepared"))
            .map(|(path, _)| rel(path))
            .collect();

        for expected in [
            "biorouter/src/scheduler.rs",
            "biorouter-cli/src/session/builder.rs",
        ] {
            assert!(
                installers.iter().any(|found| found.ends_with(expected)),
                "{expected} must install a workflow through \
                 `runtime::install_prepared`; installers found: {installers:?}"
            );
        }
    }

    /// ⚠ A generated workflow leaves the machine TWICE, and both exits are
    /// silent: `generate` returns the rendered YAML as tool content (so the
    /// whole document is appended to the conversation and shipped to the model
    /// provider on the next request), and `save` writes it in plaintext to the
    /// library users mail around. The live `ExtensionConfig` carries resolved
    /// connector auth — a `Bearer` token typed into Settings -> Extensions is
    /// stored verbatim in `StreamableHttp::headers` and is never
    /// keyring-migrated — so the capture must project it away BEFORE either
    /// exit.
    ///
    /// This guards the choke point rather than the symptom. `session_enrichment`
    /// is documented as "the ONLY enrichment", so a redaction there holds for
    /// the HTTP route, the CLI and the tool at once; a future refactor that
    /// drops the call re-opens all three.
    #[test]
    fn the_session_capture_redacts_connector_auth_before_enriching() {
        let text = std::fs::read_to_string(crates_dir().join("biorouter/src/workflow/service.rs"))
            .expect("service.rs is readable");
        let body = production(&text);
        let capture = body
            .split_once("pub async fn session_enrichment")
            .expect("session_enrichment exists")
            .1;
        let capture = capture
            .split_once("\n}\n")
            .map_or(capture, |(inside, _)| inside);
        assert!(
            capture.contains("redacted_for_session_export"),
            "`session_enrichment` must project connector auth out of the captured \n\
             extension configs before they reach a generated workflow. Without it a \n\
             live Authorization header rides the YAML to the model provider and into \n\
             a shareable file. Body seen:\n{capture}"
        );

        let agent = std::fs::read_to_string(crates_dir().join("biorouter/src/agents/agent.rs"))
            .expect("agent.rs is readable");
        let agent_body = production(&agent);
        let create = agent_body
            .split_once("pub async fn create_workflow")
            .expect("create_workflow exists")
            .1;
        assert!(
            create
                .split_once("let author")
                .map_or(create, |(before, _)| before)
                .contains("redacted_for_session_export"),
            "`Agent::create_workflow` sets `extensions` itself before any \n\
             enrichment runs, so a caller that does not enrich (the CLI did not) \n\
             would ship the unredacted set on its own."
        );
    }

    /// The projection is only worth guarding if it actually removes the secret,
    /// so pin that too — against a config shaped like the one Settings writes.
    #[test]
    fn the_projection_drops_a_bearer_token_from_the_rendered_yaml() {
        use crate::agents::extension::ExtensionConfig;
        use std::collections::HashMap;

        const TOKEN: &str = "Bearer eyJhbGciOi-SECRET-DO-NOT-SHIP";
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), TOKEN.to_string());

        let live = ExtensionConfig::StreamableHttp {
            name: "internal-agent".to_string(),
            description: "an internal connector".to_string(),
            uri: "https://internal.example/mcp".to_string(),
            envs: Default::default(),
            env_keys: vec![],
            headers,
            timeout: Some(300),
            bundled: None,
            available_tools: vec![],
        };

        let rendered = serde_yaml::to_string(&live.redacted_for_session_export())
            .expect("a redacted config serializes");
        assert!(
            !rendered.contains(TOKEN) && !rendered.contains("eyJhbGciOi"),
            "the redacted projection still carries the bearer token:\n{rendered}"
        );
        assert!(
            rendered.contains("internal-agent"),
            "redaction must keep the NAME — a consumer re-enables by matching it \n\
             against its own installed set:\n{rendered}"
        );
    }
}
