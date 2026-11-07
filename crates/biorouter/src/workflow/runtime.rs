use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::agents::extension::ExtensionConfig;
use crate::agents::Agent;
use crate::prompt_template::render_global_file;
use crate::workflow::{Workflow, WorkflowKnowledgeBases};

use biorouter_mcp::knowledge::service::{KnowledgeService, PrimaryUpdate};

/// What a workflow's `capabilities:` key may say.
///
/// A workflow that lists its own `extensions` still keeps every capability the
/// person turned on for all chats on this computer (Crew, Developer, Skills and
/// the rest), beside the ones it lists: a capability is part of the app, and a
/// list written for one task should not switch it off without saying so.
/// `capabilities: explicit` is how a workflow says so, and its chats get
/// exactly the list.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCapabilities {
    /// Exactly the extensions the workflow lists, and no capability beside them.
    Explicit,
}

/// The workflow's `capabilities:` key, read in this one place so that what a
/// session gets ([`ensure_required_extensions`]) and what its prompt says about
/// it ([`prepare_prompt`]) can never disagree.
fn capabilities_declared(workflow: &Workflow) -> Option<WorkflowCapabilities> {
    workflow.capabilities
}

/// Add each capability in `machine_wide` that `extensions` does not already
/// name. `machine_wide` is the person's enabled set
/// ([`crate::config::get_enabled_extensions`]).
///
/// Only capabilities (`ExtensionConfig::is_capability`): an installed
/// extension is not part of the app, and a list that leaves one out meant to.
/// A name the list already holds keeps the list's own entry, so a list that
/// narrows a capability to a few tools (`available_tools`) is never widened by
/// the person's unrestricted entry. A capability the person turned off is not
/// in `machine_wide`, so it stays off.
///
/// `workspace_open { new: { extensions } }` keeps them the same way
/// (`biorouter-server`'s `workspace/services.rs`).
pub fn keep_machine_wide_capabilities(
    extensions: &mut Vec<ExtensionConfig>,
    machine_wide: &[ExtensionConfig],
) {
    for config in machine_wide {
        let key = config.key();
        if config.is_capability() && !extensions.iter().any(|listed| listed.key() == key) {
            extensions.push(config.clone());
        }
    }
}

/// Complete the extension set a workflow's session starts with.
///
/// - A workflow that lists its own `extensions` keeps the capabilities the
///   person turned on machine-wide ([`keep_machine_wide_capabilities`]), unless
///   it sets `capabilities: explicit`. Without a list there is nothing to add:
///   the caller already started from the person's set, or from the person's
///   own choice for this one chat, which must not be overridden.
/// - Workflows that declare a knowledge selection require the Knowledge
///   capability even when an older persisted workflow omitted it, explicit or
///   not. This keeps managed Meditation copies from losing their ingest path
///   after capability disablement became authoritative.
///
/// The desktop's `/agent/start`, `biorouter run --workflow` and the scheduler
/// all come here, so a workflow gets the same tools wherever it runs.
pub fn ensure_required_extensions(workflow: &Workflow, extensions: &mut Vec<ExtensionConfig>) {
    ensure_required_extensions_with(
        workflow,
        capabilities_declared(workflow),
        extensions,
        crate::config::get_enabled_extensions,
    );
}

fn ensure_required_extensions_with(
    workflow: &Workflow,
    declared: Option<WorkflowCapabilities>,
    extensions: &mut Vec<ExtensionConfig>,
    machine_wide: impl FnOnce() -> Vec<ExtensionConfig>,
) {
    if workflow.extensions.is_some() && declared != Some(WorkflowCapabilities::Explicit) {
        keep_machine_wide_capabilities(extensions, &machine_wide());
    }
    ensure_knowledge_extension(workflow, extensions);
}

fn ensure_knowledge_extension(workflow: &Workflow, extensions: &mut Vec<ExtensionConfig>) {
    if workflow.knowledge_bases.is_none()
        || extensions
            .iter()
            .any(|config| config.name().eq_ignore_ascii_case("knowledge"))
    {
        return;
    }
    if let Some(target) = crate::agents::extension_manager::resolve_bundled_extension("knowledge") {
        extensions.push(target.into_config(
            "Read, search, validate, and update Biorouter knowledge bases".to_string(),
        ));
    }
}

/// Turn a workflow's declared knowledge selection into the session state the
/// knowledge service stores. The primary is never inferred from `visible`:
/// workflow authors must name a write target explicitly with `default`.
pub fn plan_knowledge_selection(
    selection: &WorkflowKnowledgeBases,
) -> (Vec<String>, Option<String>) {
    let mut visible = selection.visible.clone();
    if let Some(default) = selection.default.as_deref() {
        if !visible.iter().any(|id| id == default) {
            visible.push(default.to_string());
        }
    }
    visible.sort();
    visible.dedup();
    (visible, selection.default.clone())
}

/// Apply a workflow's knowledge selection before its first model call.
pub fn apply_knowledge_selection(
    service: &KnowledgeService,
    session_id: &str,
    workflow: &Workflow,
) -> Result<()> {
    let Some(selection) = workflow.knowledge_bases.as_ref() else {
        return Ok(());
    };

    let (visible, primary) = plan_knowledge_selection(selection);
    let primary = match primary.as_deref() {
        Some(id) => PrimaryUpdate::Set(id),
        None => PrimaryUpdate::Clear,
    };
    service
        .set_visible_kbs(Some(session_id), &visible, primary)
        .context("applying workflow knowledge-base selection")?;
    Ok(())
}

/// Install a workflow's components and return its rendered system-prompt
/// addition. Declared skills are resolved to their actual, session-enabled
/// bodies; naming a missing or disabled skill is an error rather than a hint
/// the model may silently ignore.
pub async fn apply_to_agent(
    agent: &Agent,
    session_id: &str,
    workflow: &Workflow,
    include_final_output_tool: bool,
) -> Result<Option<String>> {
    let prompt =
        prepare_prompt(agent.config.session_manager.as_ref(), session_id, workflow).await?;
    apply_prepared_to_agent(agent, workflow, include_final_output_tool, prompt.clone()).await;
    Ok(prompt)
}

/// Resolve every fallible prompt dependency without mutating the live agent.
pub async fn prepare_prompt(
    session_manager: &crate::session::SessionManager,
    session_id: &str,
    workflow: &Workflow,
) -> Result<Option<String>> {
    let skill_instructions = match workflow.skills.as_deref() {
        Some(skills) if !skills.is_empty() => {
            crate::agents::skills_extension::workflow_skill_instructions(
                session_manager,
                session_id,
                skills,
            )
            .await?
        }
        _ => String::new(),
    };

    let left_out = left_out_capabilities_note(
        workflow,
        capabilities_declared(workflow),
        crate::config::get_enabled_extensions,
    );
    let prompt = match standing_instructions(workflow, &skill_instructions, left_out) {
        None => None,
        Some(instructions) => {
            let mut context: HashMap<&str, Value> = HashMap::new();
            context.insert("workflow_instructions", Value::String(instructions));
            Some(
                render_global_file("desktop_workflow_instruction.md", &context)
                    .context("rendering workflow instructions")?,
            )
        }
    };

    Ok(prompt)
}

/// The workflow's own instructions, then its required skills' bodies, then the
/// note about capabilities it leaves out, each separated by a blank line.
/// `None` when there is none of the three, so a prompt-only workflow adds
/// nothing to the system prompt.
fn standing_instructions(
    workflow: &Workflow,
    skill_instructions: &str,
    left_out: Option<String>,
) -> Option<String> {
    if workflow.instructions.is_none() && skill_instructions.is_empty() && left_out.is_none() {
        return None;
    }
    let mut instructions = workflow.instructions.clone().unwrap_or_default();
    for section in [skill_instructions, left_out.as_deref().unwrap_or_default()] {
        if section.is_empty() {
            continue;
        }
        if !instructions.is_empty() {
            instructions.push_str("\n\n");
        }
        instructions.push_str(section);
    }
    Some(instructions)
}

/// What a `capabilities: explicit` workflow's chat is told about the
/// capabilities it leaves out although the person turned them on for every
/// other chat (Crew among them), so the model can say why one is missing and
/// how to get it rather than act as if it did not exist. `None` for any other
/// workflow, and when nothing is left out.
fn left_out_capabilities_note(
    workflow: &Workflow,
    declared: Option<WorkflowCapabilities>,
    machine_wide: impl FnOnce() -> Vec<ExtensionConfig>,
) -> Option<String> {
    if declared != Some(WorkflowCapabilities::Explicit) {
        return None;
    }
    let listed = workflow.extensions.as_deref()?;
    // The set the session really gets (Knowledge included when the workflow
    // declares bases), so nothing it has is named as missing.
    let mut session = listed.to_vec();
    ensure_required_extensions_with(workflow, declared, &mut session, Vec::new);
    let mut with_machine_wide = session.clone();
    keep_machine_wide_capabilities(&mut with_machine_wide, &machine_wide());
    let names: Vec<String> = with_machine_wide[session.len()..]
        .iter()
        .map(capability_label)
        .collect();
    if names.is_empty() {
        return None;
    }
    Some(format!(
        "This workflow keeps to the extensions it lists (`capabilities: explicit`), so these \
         capabilities are off in this chat although they are on for other chats on this \
         computer: {}. If the user asks for one of them, say it is off in this workflow's \
         chats, and that they can use it in a new chat or add it to the workflow's \
         `extensions`.",
        names.join(", ")
    ))
}

/// A capability's name as the app shows it ("Crew", "Workspace Control").
fn capability_label(config: &ExtensionConfig) -> String {
    crate::config::extensions::bundled_extension_display_name(&config.key())
        .map(str::to_owned)
        .unwrap_or_else(|| config.name())
}

/// Commit a prompt that was already prepared successfully, then install the
/// workflow's in-memory tools. This stage has no fallible filesystem lookup.
pub async fn apply_prepared_to_agent(
    agent: &Agent,
    workflow: &Workflow,
    include_final_output_tool: bool,
    prompt: Option<String>,
) {
    agent
        .apply_workflow_components(
            workflow.sub_workflows.clone(),
            workflow.response.clone(),
            include_final_output_tool,
        )
        .await;
    agent.set_session_context_prompt(prompt).await;
}

/// Install a workflow into the session about to run it: its knowledge
/// selection, its components, and the prompt [`prepare_prompt`] already
/// resolved.
///
/// ⚠ Takes an ALREADY-PREPARED prompt rather than calling `prepare_prompt`
/// itself, and that split is load-bearing. `prepare_prompt` is the fallible
/// half — it resolves declared skills and errors when one is missing or
/// disabled — so a caller that persists the workflow to its session row runs it
/// *before* the persist, and a workflow naming a skill that is not installed
/// fails without leaving a half-armed session behind. Collapsing the two would
/// persist first and fail second.
///
/// Every headless surface that starts a workflow comes here. The CLI did not,
/// which is why `biorouter run --workflow` silently ignored `skills:` and
/// `knowledge_bases:` while the desktop and the scheduler honoured both.
pub async fn install_prepared(
    agent: &Agent,
    session_id: &str,
    workflow: &Workflow,
    include_final_output_tool: bool,
    prompt: Option<String>,
) -> Result<()> {
    let knowledge = biorouter_mcp::knowledge::service::KnowledgeService::new_default()?;
    apply_knowledge_selection(&knowledge, session_id, workflow)?;
    apply_prepared_to_agent(agent, workflow, include_final_output_tool, prompt).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_default_is_visible_but_never_inferred() {
        let declared = WorkflowKnowledgeBases {
            default: Some("soul".into()),
            visible: vec!["research".into(), "research".into()],
        };
        assert_eq!(
            plan_knowledge_selection(&declared),
            (
                vec!["research".to_string(), "soul".to_string()],
                Some("soul".into())
            )
        );

        let visible_only = WorkflowKnowledgeBases {
            default: None,
            visible: vec!["soul".into()],
        };
        assert_eq!(
            plan_knowledge_selection(&visible_only),
            (vec!["soul".to_string()], None)
        );
    }

    #[test]
    fn knowledge_workflows_recover_their_required_capability_without_duplicates() {
        let mut workflow = Workflow::builder()
            .title("Meditation")
            .description("test")
            .instructions("test")
            .build()
            .unwrap();
        workflow.knowledge_bases = Some(WorkflowKnowledgeBases {
            default: Some("soul".into()),
            visible: vec!["soul".into()],
        });
        let mut extensions = Vec::new();
        ensure_required_extensions(&workflow, &mut extensions);
        ensure_required_extensions(&workflow, &mut extensions);
        assert_eq!(
            extensions
                .iter()
                .filter(|config| config.name() == "knowledge")
                .count(),
            1
        );
        assert!(matches!(
            extensions.first(),
            Some(ExtensionConfig::Builtin { .. })
        ));
    }

    fn platform(name: &str) -> ExtensionConfig {
        ExtensionConfig::Platform {
            name: name.to_string(),
            description: String::new(),
            bundled: Some(true),
            available_tools: Vec::new(),
        }
    }

    fn builtin(name: &str, tools: &[&str]) -> ExtensionConfig {
        ExtensionConfig::Builtin {
            name: name.to_string(),
            display_name: None,
            description: String::new(),
            timeout: None,
            bundled: Some(true),
            available_tools: tools.iter().map(|tool| tool.to_string()).collect(),
        }
    }

    /// An extension the person installed: on machine-wide, and not a capability.
    fn installed(name: &str) -> ExtensionConfig {
        ExtensionConfig::Stdio {
            name: name.to_string(),
            description: String::new(),
            cmd: "/usr/local/bin/lab-server".to_string(),
            args: vec![],
            envs: Default::default(),
            env_keys: vec![],
            timeout: None,
            bundled: None,
            available_tools: vec![],
        }
    }

    /// The person's enabled set in these tests: two capabilities (Crew and an
    /// unrestricted Developer), a capability whose name is spelled as config
    /// spells it, and an installed extension.
    fn machine_wide() -> Vec<ExtensionConfig> {
        vec![
            platform("crew"),
            builtin("developer", &[]),
            platform("Workspace"),
            installed("lab-mcp"),
        ]
    }

    fn workflow_listing(extensions: Vec<ExtensionConfig>) -> Workflow {
        let mut workflow = Workflow::builder()
            .title("Weekly report")
            .description("test")
            .instructions("write the report")
            .build()
            .unwrap();
        workflow.extensions = Some(extensions);
        workflow
    }

    fn keys(extensions: &[ExtensionConfig]) -> Vec<String> {
        extensions.iter().map(ExtensionConfig::key).collect()
    }

    /// WI-10. A workflow that lists its own extensions used to start with
    /// exactly that list, so its chat had no Crew (and no Developer, Skills…)
    /// although the person turned them on for every chat.
    #[test]
    fn a_workflow_listing_its_extensions_keeps_the_capabilities_turned_on_machine_wide() {
        let workflow = workflow_listing(vec![builtin("developer", &["shell"])]);
        let mut extensions = workflow.extensions.clone().unwrap();
        ensure_required_extensions_with(&workflow, None, &mut extensions, machine_wide);

        assert_eq!(keys(&extensions), ["developer", "crew", "workspace"]);
        // The list's own Developer, narrowed to one tool, is the one kept: the
        // person's unrestricted entry must never widen it.
        assert_eq!(extensions[0], builtin("developer", &["shell"]));
        // An installed extension is not part of the app; a list that leaves one
        // out meant to.
        assert!(!keys(&extensions).contains(&"lab-mcp".to_string()));
    }

    #[test]
    fn capabilities_explicit_keeps_exactly_the_list() {
        let workflow = workflow_listing(vec![builtin("developer", &[])]);
        let mut extensions = workflow.extensions.clone().unwrap();
        ensure_required_extensions_with(
            &workflow,
            Some(WorkflowCapabilities::Explicit),
            &mut extensions,
            || panic!("an explicit workflow never reads the machine-wide set"),
        );
        assert_eq!(keys(&extensions), ["developer"]);
    }

    /// Crew turned off for every chat stays off in a workflow's chat too.
    #[test]
    fn a_capability_turned_off_machine_wide_is_not_added() {
        let workflow = workflow_listing(vec![]);
        let mut extensions = Vec::new();
        ensure_required_extensions_with(&workflow, None, &mut extensions, || {
            vec![builtin("developer", &[])]
        });
        assert_eq!(keys(&extensions), ["developer"]);
    }

    /// Without a list the caller started from the person's set, or from the
    /// person's own choice for this one chat (`/agent/start`'s overrides), and
    /// re-adding a capability they switched off there would undo it.
    #[test]
    fn a_workflow_without_a_list_is_left_as_the_caller_started_it() {
        let workflow = Workflow::builder()
            .title("No list")
            .description("test")
            .instructions("test")
            .build()
            .unwrap();
        let mut extensions = vec![builtin("developer", &[])];
        ensure_required_extensions_with(&workflow, None, &mut extensions, || {
            panic!("a workflow without a list never reads the machine-wide set")
        });
        assert_eq!(keys(&extensions), ["developer"]);
    }

    /// Knowledge is required by a declared selection whatever the workflow says
    /// about capabilities, and it is never added twice.
    #[test]
    fn an_explicit_workflow_with_knowledge_bases_still_gets_knowledge() {
        let mut workflow = workflow_listing(vec![]);
        workflow.knowledge_bases = Some(WorkflowKnowledgeBases {
            default: Some("soul".into()),
            visible: vec![],
        });
        let mut extensions = Vec::new();
        ensure_required_extensions_with(
            &workflow,
            Some(WorkflowCapabilities::Explicit),
            &mut extensions,
            Vec::new,
        );
        assert_eq!(keys(&extensions), ["knowledge"]);

        let mut extensions = Vec::new();
        ensure_required_extensions_with(&workflow, None, &mut extensions, || {
            vec![builtin("knowledge", &[]), platform("crew")]
        });
        assert_eq!(keys(&extensions), ["knowledge", "crew"]);
    }

    /// The public entry point reads the person's real configuration (this test
    /// binary's sandboxed one, where the shipped capabilities are on). Other
    /// tests in this binary may switch entries while it runs, so it asserts
    /// what holds whatever they do: something was kept, and all of it is a
    /// capability.
    #[test]
    fn ensure_required_extensions_keeps_the_configured_capabilities() {
        let workflow = workflow_listing(vec![]);
        let mut extensions = Vec::new();
        ensure_required_extensions(&workflow, &mut extensions);
        assert!(
            !extensions.is_empty(),
            "the sandboxed config turns the shipped capabilities on, and a listing workflow keeps them"
        );
        assert!(
            extensions.iter().all(ExtensionConfig::is_capability),
            "only capabilities are kept: {:?}",
            keys(&extensions)
        );
    }

    #[test]
    fn an_explicit_workflow_says_which_capabilities_it_leaves_out_and_how_to_get_one() {
        let workflow = workflow_listing(vec![builtin("developer", &[])]);
        let note = left_out_capabilities_note(
            &workflow,
            Some(WorkflowCapabilities::Explicit),
            machine_wide,
        )
        .expect("Crew and Workspace Control are left out");
        assert!(
            note.contains(": Crew, Workspace Control."),
            "the note names what is off, as the app names it: {note}"
        );
        assert!(note.contains("new chat"), "and how to get one: {note}");
        assert!(!note.contains("Developer"), "Developer is listed: {note}");
        assert!(
            !note.contains("lab-mcp"),
            "an installed extension is no capability: {note}"
        );
        assert!(
            !note.contains('\u{2014}') && !note.contains('\u{2013}'),
            "no dashes: {note}"
        );
    }

    #[test]
    fn only_an_explicit_workflow_that_leaves_something_out_carries_the_note() {
        let workflow = workflow_listing(vec![builtin("developer", &[])]);
        assert_eq!(
            left_out_capabilities_note(&workflow, None, || panic!("not read")),
            None,
            "a workflow that keeps the capabilities has nothing to explain"
        );
        let everything = workflow_listing(machine_wide());
        assert_eq!(
            left_out_capabilities_note(
                &everything,
                Some(WorkflowCapabilities::Explicit),
                machine_wide
            ),
            None,
            "nothing is left out"
        );

        // Knowledge comes back for a declared selection, so it is not named.
        let mut with_bases = workflow_listing(vec![]);
        with_bases.knowledge_bases = Some(WorkflowKnowledgeBases {
            default: Some("soul".into()),
            visible: vec![],
        });
        let note =
            left_out_capabilities_note(&with_bases, Some(WorkflowCapabilities::Explicit), || {
                vec![builtin("knowledge", &[]), platform("crew")]
            })
            .unwrap();
        assert!(note.contains(": Crew."), "{note}");
        assert!(!note.contains("Knowledge"), "{note}");
    }

    #[test]
    fn the_note_follows_the_instructions_and_the_skills() {
        let workflow = workflow_listing(vec![]);
        assert_eq!(
            standing_instructions(&workflow, "SKILL", Some("NOTE".into())),
            Some("write the report\n\nSKILL\n\nNOTE".to_string())
        );
        assert_eq!(
            standing_instructions(&workflow, "", None),
            Some("write the report".to_string())
        );

        let mut prompt_only = workflow_listing(vec![]);
        prompt_only.instructions = None;
        assert_eq!(standing_instructions(&prompt_only, "", None), None);
        assert_eq!(
            standing_instructions(&prompt_only, "", Some("NOTE".into())),
            Some("NOTE".to_string())
        );
    }

    #[test]
    fn the_capabilities_key_reads_as_written() {
        assert_eq!(
            serde_yaml::from_str::<WorkflowCapabilities>("explicit").unwrap(),
            WorkflowCapabilities::Explicit
        );
        assert!(serde_yaml::from_str::<WorkflowCapabilities>("explict").is_err());
    }

    /// The key as a workflow file writes it reaches both decisions, and a
    /// workflow without it is read and written back without it.
    #[test]
    fn capabilities_explicit_in_a_workflow_file_keeps_exactly_its_list() {
        let explicit = Workflow::from_content(
            "title: Weekly report\ndescription: test\ninstructions: write it\n\
             capabilities: explicit\nextensions:\n  - type: builtin\n    name: developer\n",
        )
        .unwrap();
        assert_eq!(
            capabilities_declared(&explicit),
            Some(WorkflowCapabilities::Explicit)
        );
        let mut extensions = explicit.extensions.clone().unwrap();
        ensure_required_extensions(&explicit, &mut extensions);
        assert_eq!(keys(&extensions), ["developer"]);
        let note = left_out_capabilities_note(&explicit, capabilities_declared(&explicit), || {
            vec![platform("crew")]
        })
        .expect("the chat is told what the file leaves out");
        assert!(note.contains(": Crew."), "{note}");
        assert!(serde_yaml::to_string(&explicit)
            .unwrap()
            .contains("capabilities: explicit"));

        let plain = Workflow::from_content(
            "title: Weekly report\ndescription: test\ninstructions: write it\n\
             extensions:\n  - type: builtin\n    name: developer\n",
        )
        .unwrap();
        assert_eq!(capabilities_declared(&plain), None);
        assert!(!serde_yaml::to_string(&plain)
            .unwrap()
            .contains("capabilities"));
    }
}
