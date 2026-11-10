use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::agents::extension::ExtensionConfig;
use crate::agents::Agent;
use crate::prompt_template::render_global_file;
use crate::workflow::supervision::{self, SeedOutcome, SeedRefusal, SeedTarget, SeedTargetKind};
use crate::workflow::{Workflow, WorkflowKnowledgeBases};

use biorouter_mcp::knowledge::service::{KnowledgeService, PrimaryUpdate};

/// Workflows that declare a knowledge selection require the Knowledge
/// capability even when an older persisted workflow omitted it. This keeps
/// managed Meditation copies from losing their ingest path after capability
/// disablement became authoritative.
pub fn ensure_required_extensions(workflow: &Workflow, extensions: &mut Vec<ExtensionConfig>) {
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

    let prompt = if workflow.instructions.is_none() && skill_instructions.is_empty() {
        None
    } else {
        let mut instructions = workflow.instructions.clone().unwrap_or_default();
        if !skill_instructions.is_empty() {
            if !instructions.is_empty() {
                instructions.push_str("\n\n");
            }
            instructions.push_str(&skill_instructions);
        }

        let mut context: HashMap<&str, Value> = HashMap::new();
        context.insert("workflow_instructions", Value::String(instructions));
        Some(
            render_global_file("desktop_workflow_instruction.md", &context)
                .context("rendering workflow instructions")?,
        )
    };

    Ok(prompt)
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
///
/// **The supervision the workflow declares is seeded FIRST**
/// ([`supervision::seed`]): its `decision_settings` (default decision model,
/// monitors) and its `orchestration_plan` (phases, gates the workflow owns),
/// applied once to a chat that has no messages yet, before anything else is
/// installed. The seed asks with what this agent really runs on: its approval
/// mode (`config.biorouter_mode`), its own decision service when one was
/// installed ([`Agent::custom_decision_service`]) or else the process's, and
/// the capability tier of the chat model it is bound to. `enforced` is the
/// privacy master switch as the caller sampled it.
///
/// A seed that cannot apply what the workflow declares writes nothing and
/// fails the install with an error that carries the [`SeedRefusal`] (downcast
/// it) under the context [`WORKFLOW_NOT_STARTED`], so `{:#}` reads "The
/// workflow was not started: <sentence>". The caller must not start the run.
/// [`SeedOutcome::ChatHasMessages`] is not an error: a resumed chat keeps what
/// it has, and the caller says the blocks were not applied.
pub async fn install_prepared(
    agent: &Agent,
    session_id: &str,
    workflow: &Workflow,
    include_final_output_tool: bool,
    prompt: Option<String>,
    enforced: bool,
) -> Result<SeedOutcome> {
    let outcome = if supervision::declares_supervision(workflow) {
        let custom = agent.custom_decision_service();
        let service = match custom.as_deref() {
            Some(service) => service,
            None => crate::decision::global(),
        };
        // What the chat is bound to now: a chat still Public but bound to a
        // private model becomes private at its first turn, so a public
        // decision model must be refused for it before that turn.
        let bound_tier = agent.provider().await.ok().map(|provider| provider.tier());
        let target = SeedTarget {
            session_id: session_id.to_string(),
            kind: SeedTargetKind::Chat,
            mode: agent.config.biorouter_mode,
            enforced,
            bound_tier,
        };
        // Boxed, so the seed's state (its settings and plan checks) lives on
        // the heap rather than inside every future that awaits this install.
        // Measured in a debug build: `scheduler::execute_job`'s future pinned
        // on a 2 MiB test thread already needed about 2000 KiB, and inlining
        // the seed into it overflowed that stack.
        Box::pin(supervision::seed(
            agent.config.session_manager.as_ref(),
            service,
            workflow,
            target,
        ))
        .await
        .map_err(|refusal| anyhow::Error::new(refusal).context(WORKFLOW_NOT_STARTED))?
    } else {
        SeedOutcome::NothingDeclared
    };
    let knowledge = biorouter_mcp::knowledge::service::KnowledgeService::new_default()?;
    apply_knowledge_selection(&knowledge, session_id, workflow)?;
    apply_prepared_to_agent(agent, workflow, include_final_output_tool, prompt).await;
    Ok(outcome)
}

/// What every door that refuses to start a workflow says first, before the
/// refusal's own sentence ([`SeedRefusal::sentence`]).
pub const WORKFLOW_NOT_STARTED: &str = "The workflow was not started";

/// What a door says when a workflow's supervision was not applied because
/// the chat it runs in already has messages ([`SeedOutcome::ChatHasMessages`])
/// and this workflow never seeded it.
pub const SUPERVISION_NOT_APPLIED: &str = "This chat already has messages, so the monitors and \
     phase plan this workflow declares were not applied to it. Run the workflow in a new chat to \
     get them.";

/// What a door says when the chat already has messages and this same
/// workflow seeded it when it started: the blocks are there, as the person
/// has left or changed them since, and are not applied a second time.
pub const SUPERVISION_APPLIED_AT_START: &str = "This chat already has messages. The monitors and \
     phase plan this workflow declares were applied when the chat started and are not applied \
     again, so the chat keeps them as they are now.";

/// What a door says when a workflow's supervision was not applied because
/// another workflow seeded the chat before its first message
/// ([`SeedOutcome::SeededByAnotherWorkflow`]): the chat has no messages yet
/// and keeps that workflow's monitors and phase plan.
pub const SUPERVISION_SEEDED_BY_ANOTHER_WORKFLOW: &str = "Another workflow already gave this chat \
     its monitors and phase plan before its first message, so the ones this workflow declares were \
     not applied to it. Run the workflow in a new chat to get them.";

/// The line a door shows about `workflow`'s supervision after
/// [`install_prepared`] returned `outcome` for the chat `session_id`, or
/// `None` when there is nothing to say.
///
/// Two outcomes earn a line. [`SeedOutcome::SeededByAnotherWorkflow`] (an
/// empty chat another workflow seeded) is told this workflow's blocks were
/// not applied ([`SUPERVISION_SEEDED_BY_ANOTHER_WORKFLOW`]). And
/// [`SeedOutcome::ChatHasMessages`], which the seed returns for every chat
/// that has started, before it looks at the seed marker. So for it the
/// marker is read here: a chat this workflow seeded when it started (the
/// marker carries the workflow's title) is told the blocks are not applied
/// AGAIN ([`SUPERVISION_APPLIED_AT_START`]), and any other chat, never seeded
/// or seeded by another workflow, that they were not applied
/// ([`SUPERVISION_NOT_APPLIED`]). A marker that cannot be read counts as no
/// marker, with a warning in the log. A retried start of the same workflow
/// ([`SeedOutcome::AlreadySeeded`]) says nothing.
pub async fn supervision_notice(
    manager: &crate::session::SessionManager,
    session_id: &str,
    workflow: &Workflow,
    outcome: &SeedOutcome,
) -> Option<&'static str> {
    match outcome {
        SeedOutcome::SeededByAnotherWorkflow { .. } => {
            return Some(SUPERVISION_SEEDED_BY_ANOTHER_WORKFLOW);
        }
        SeedOutcome::ChatHasMessages => {}
        SeedOutcome::NothingDeclared | SeedOutcome::Seeded { .. } | SeedOutcome::AlreadySeeded => {
            return None
        }
    }
    let marker = manager
        .get_extension_state(
            session_id,
            supervision::WORKFLOW_SUPERVISION_STATE,
            supervision::WORKFLOW_SUPERVISION_VERSION,
        )
        .await;
    let seeded_by_this_workflow = match marker {
        Ok(Some(value)) => serde_json::from_value::<supervision::SeedMarker>(value)
            .is_ok_and(|marker| marker.title == workflow.title),
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                session_id,
                "could not read whether a workflow seeded this chat: {error:#}"
            );
            false
        }
    };
    Some(if seeded_by_this_workflow {
        SUPERVISION_APPLIED_AT_START
    } else {
        SUPERVISION_NOT_APPLIED
    })
}

/// The sentence a door shows for `error` when it is a refused seed, or
/// `None` for any other failure: "The workflow was not started: <sentence>".
pub fn seed_refusal_text(error: &anyhow::Error) -> Option<String> {
    error
        .downcast_ref::<SeedRefusal>()
        .map(|refusal| format!("{WORKFLOW_NOT_STARTED}: {}", refusal.sentence))
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
}
