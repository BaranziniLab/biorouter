//! Chat-side handler for the `platform__ingest_conversation` tool.
//!
//! Lets the user, mid-conversation, fold chat history into a knowledge base by
//! "just saying the word". It resolves the target KB (existing / new / active),
//! loads the requested sessions (defaulting to the current one), and runs the
//! shared [`conversation_ingest`] pipeline. Normal chat-side ingestion uses the
//! agent's own provider; scheduled knowledge jobs prefer the target KB's default
//! model when one is configured.

use std::sync::Arc;

use rmcp::model::{Content, ErrorCode, ErrorData};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::Agent;
use crate::knowledge::conversation_ingest::{
    ingest_conversation_with_curation_profile, ConversationIngestArgs,
};
use crate::knowledge::ProviderCompleter;
use crate::mcp_utils::ToolResult;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use crate::session::session_manager::{Session, SessionType};
use biorouter_mcp::knowledge::caller::KbCaller;
use biorouter_mcp::knowledge::service::KnowledgeService;
use biorouter_mcp::knowledge::subagent::loop_::{Completer, SubAgentBounds};
use biorouter_mcp::knowledge::subagent::procedures::IngestCurationProfile;
use biorouter_mcp::knowledge::types::ModelRef;

impl Agent {
    pub async fn handle_ingest_conversation(
        &self,
        arguments: Value,
        session: &Session,
        cancel: Option<CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let chat_provider = self.provider().await.ok();
        let pinned_provider = std::sync::Arc::new(tokio::sync::Mutex::new(chat_provider.clone()));
        let chat_capability = crate::privacy::CallCapability::sample(&pinned_provider).await;
        handle_ingest_conversation_with_provider(
            arguments,
            session,
            cancel,
            chat_capability,
            chat_provider,
            Arc::clone(&self.config.session_manager),
        )
        .await
    }

    /// The completer this ingest will run on, **and** the tier of the provider
    /// behind it (issue #56) — from `ProviderCompleter::paired`, so the two can
    /// never come from different providers.
    #[cfg(test)]
    async fn conversation_ingest_completer(
        &self,
        svc: &KnowledgeService,
        kb_id: &str,
        session: &Session,
        cancel: Option<CancellationToken>,
    ) -> Result<
        (
            Box<dyn Completer>,
            ProviderTier,
            Option<crate::privacy::affiliation::ModelAffiliation>,
        ),
        ErrorData,
    > {
        conversation_ingest_completer_with_provider(
            svc,
            kb_id,
            session,
            cancel,
            self.provider().await.ok(),
        )
        .await
    }
}

pub(crate) async fn handle_ingest_conversation_with_provider(
    arguments: Value,
    session: &Session,
    cancel: Option<CancellationToken>,
    chat_capability: crate::privacy::CallCapability,
    chat_provider: Option<Arc<dyn crate::providers::base::Provider>>,
    session_manager: Arc<crate::session::SessionManager>,
) -> ToolResult<Vec<Content>> {
    let svc = KnowledgeService::new_default().map_err(internal)?;
    let session_ids: Vec<String> = arguments
        .get("session_ids")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .filter(|ids: &Vec<String>| !ids.is_empty())
        .unwrap_or_else(|| vec![session.id.clone()]);
    let kb_id = resolve_target_kb(&svc, &arguments, &session.id, &kb_caller(chat_capability))
        .map_err(invalid_params)?;
    let curation_profile = platform_curation_profile(session_manager.as_ref(), &session.id, &kb_id)
        .await
        .map_err(invalid_params)?;

    let mut sessions = Vec::with_capacity(session_ids.len());
    for session_id in &session_ids {
        let selected = session_manager
            .get_session(session_id, true)
            .await
            .map_err(|error| {
                invalid_params(format!("session '{session_id}' not found: {error}"))
            })?;
        sessions.push(selected);
    }
    let (completer, caller_capability, caller_affiliation) =
        conversation_ingest_completer_with_provider(
            &svc,
            &kb_id,
            session,
            cancel.clone(),
            chat_provider,
        )
        .await?;

    let result = ingest_conversation_with_curation_profile(
        &svc,
        ConversationIngestArgs {
            kb_id: kb_id.clone(),
            caller_capability,
            caller_affiliation,
            session_manager,
            sessions,
            completer,
            focus: arguments
                .get("focus")
                .and_then(Value::as_str)
                .map(str::to_owned),
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel,
        },
        curation_profile,
    )
    .await
    .map_err(internal)?;

    Ok(vec![Content::text(ingest_summary(
        session_ids.len().saturating_sub(result.refused),
        result.refused,
        &kb_id,
        &result.ingested.source_id,
        &result.ingested.commit_sha,
        result.ingested.steps,
    ))])
}

async fn conversation_ingest_completer_with_provider(
    svc: &KnowledgeService,
    kb_id: &str,
    session: &Session,
    cancel: Option<CancellationToken>,
    chat_provider: Option<Arc<dyn crate::providers::base::Provider>>,
) -> Result<
    (
        Box<dyn Completer>,
        ProviderTier,
        Option<crate::privacy::affiliation::ModelAffiliation>,
    ),
    ErrorData,
> {
    if biorouter_mcp::knowledge::test_mode::env_enabled() {
        return Ok((
            Box::new(biorouter_mcp::knowledge::test_mode::TestModeCompleter),
            ProviderTier::Public,
            None,
        ));
    }
    if should_use_knowledge_default_model(session) {
        let manifest = svc.get_base(kb_id).map_err(internal)?;
        if let Some(model) = manifest.default_model {
            return build_model_ref_completer(&model, session.privacy_tier, cancel)
                .await
                .map_err(|error| {
                    internal(format!(
                        "the default knowledge model for '{kb_id}' could not be used: {error}"
                    ))
                });
        }
    }
    let provider = chat_provider
        .ok_or_else(|| internal("a model provider is required to digest conversations"))?;
    let (completer, tier, affiliation) = ProviderCompleter::paired(provider);
    let completer = match cancel {
        Some(cancel) => completer.cancelled_by(cancel),
        None => completer,
    };
    Ok((
        Box::new(completer.in_session(session.id.clone())),
        tier,
        affiliation,
    ))
}

pub(crate) async fn platform_curation_profile(
    session_manager: &crate::session::SessionManager,
    session_id: &str,
    kb_id: &str,
) -> anyhow::Result<Option<IngestCurationProfile>> {
    if kb_id != crate::knowledge::soul::SOUL_KB_ID {
        return Ok(None);
    }

    let skill_instructions = crate::agents::skills_extension::workflow_skill_instructions(
        session_manager,
        session_id,
        &[crate::knowledge::soul::SOUL_SKILL_DIR.to_string()],
    )
    .await?;
    Ok(Some(IngestCurationProfile::Soul { skill_instructions }))
}

/// This chat's capability in the vocabulary the KB barrier owns — the ONE
/// crossing between `biorouter`'s three-axis [`CallCapability`] and
/// `biorouter-mcp`'s [`KbCaller`] (issue #56, audit finding 17).
///
/// ⚠ **`enforced` is deliberately dropped.** DR-15's master toggle is read at
/// the choke point — inside `tier::assert_reachable`, which `KbCaller::can_reach`
/// delegates to — and nowhere above it. Passing the sampled `enforced` through
/// and *also* calling the barrier would be two reads of one switch, which is the
/// race `CallCapability` exists to prevent, reintroduced one layer down. Every
/// other KB filter in the tree (`KnowledgeServer`'s five, `Catalog::discover`)
/// does the same.
///
/// [`CallCapability`]: crate::privacy::CallCapability
pub(crate) fn kb_caller(cap: crate::privacy::CallCapability) -> KbCaller {
    KbCaller::new(
        cap.tier().is_private(),
        crate::privacy::affiliation::caller_affiliation(cap.affiliation()),
    )
}

/// Resolve which KB an ingest targets: an explicit `kb_id`, else
/// `new_kb_name` creates one, else **this session's primary**.
///
/// It must be the session's primary, not the machine-wide pointer: every other
/// surface writes session-scoped state, so reading the machine default here
/// sent a workflow/Meditation session's transcript into an unrelated base.
///
/// `caller` is the identity of the model that will read the error text
/// (issue #56). This function is `kb_id_or_primary`'s twin one crate over —
/// Task 10C's fix lives in `biorouter-mcp`'s `KnowledgeServer` and cannot reach
/// an `impl Agent` in `biorouter` — so it takes the same value and asks the same
/// question.
///
/// ⚠ **Audit finding 17's second spelling lived here.** The filter below was
/// `caller.is_private() || !tier::is_private(root, id)`: the tier axis alone,
/// and not DR-15's master toggle either. Two consequences, mirror images of each
/// other and both user-visible:
///
///  * With tiers ON, a chat bound to a model covered by another institution's
///    agreements was handed the ids of bases whose content the barrier then
///    refused — and that id is the one argument that makes this function's
///    explicit-`kb_id` branch reachable.
///  * With tiers OFF, it went on hiding bases the very next call would serve in
///    full, which is the same inconsistency in the other direction and breaks
///    DR-15's promise that nothing is impacted when the feature is off.
///
/// It now asks [`KbCaller::can_reach`] — `tier::assert_reachable` negated,
/// exactly what `KnowledgeServer::kb_is_out_of_reach` asks. There is no
/// independent predicate left to keep in sync.
///
/// Explicit ids are checked here rather than left for the ingest pipeline. An
/// inaccessible id gets the same answer as an absent one; otherwise the later
/// barrier's privacy refusal confirms that the guessed base exists.
/// `new_kb_name` always creates a fresh opaque id, so asking for a display name
/// cannot probe whether a visible or invisible base already uses that name.
pub(crate) fn resolve_target_kb(
    svc: &KnowledgeService,
    arguments: &Value,
    session_id: &str,
    caller: &KbCaller,
) -> anyhow::Result<String> {
    if let Some(id) = arguments.get("kb_id").and_then(|v| v.as_str()) {
        let id = id.trim();
        let exists = svc.list_bases()?.iter().any(|base| base.id == id);
        if !exists || !caller.can_reach(svc.root(), id) {
            anyhow::bail!("knowledge base '{id}' does not exist");
        }
        return Ok(id.to_string());
    }
    if let Some(name) = arguments.get("new_kb_name").and_then(|v| v.as_str()) {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("new_kb_name cannot be empty");
        }
        loop {
            let id = format!("kb-{}", uuid::Uuid::new_v4().simple());
            if svc.list_bases()?.iter().any(|base| base.id == id) {
                continue;
            }
            svc.create_base(&id, name, None)?;
            return Ok(id);
        }
    }
    if let Some(primary) = svc.primary_for_session(Some(session_id))? {
        if caller.can_reach(svc.root(), &primary) {
            return Ok(primary);
        }
    }
    let ids: Vec<String> = svc
        .session_kb_ids(Some(session_id))?
        .into_iter()
        // Issue #56. Per id, and BEFORE the `is_empty` check below, so a chat
        // whose only base is private is told it has none rather than being
        // handed `(one of: )` — which is both useless and a tell. A barrier that
        // refuses a read and then hands over the identifier of the thing it
        // refused is not a barrier, and that identifier is the one argument that
        // makes the explicit-`kb_id` branch reachable.
        //
        // ⚠ Finding 17: the BARRIER, negated — never a re-derived condition.
        // Per id and not once over the set, so one unreachable base does not
        // cost the chat every other one.
        .filter(|id| caller.can_reach(svc.root(), id))
        .collect();
    if ids.is_empty() {
        anyhow::bail!(
            "no target knowledge base: this chat has none. Pass new_kb_name to create one, \
             or kb_id to name an existing base."
        );
    }
    anyhow::bail!(
        "no target knowledge base: pass kb_id (one of: {}) or new_kb_name, or call \
         kb_set_active to make one of them this chat's primary.",
        ids.join(", ")
    )
}

/// The success text for a conversation ingest. A KB-less write resolves its
/// target silently, so the result must name the base it landed in.
fn ingest_summary(
    session_count: usize,
    refused: usize,
    kb_id: &str,
    source_id: &str,
    commit_sha: &str,
    steps: usize,
) -> String {
    // Issue #56. A COUNT and nothing else — §11.4 classifies a session's id,
    // title and working directory as content, and this product's titles are
    // LLM-generated from the conversation itself.
    let refused_note = if refused == 0 {
        String::new()
    } else {
        format!(
            " {refused} private conversation(s) were skipped: this chat is running on a public \
             model. Ask the user to switch this chat to a private model to include them."
        )
    };
    format!(
        "Ingested {session_count} conversation(s) into knowledge base '{kb_id}'. \
         Source id: {source_id}, commit: {}, sub-agent steps: {steps}.{refused_note}",
        commit_sha.chars().take(8).collect::<String>()
    )
}

fn should_use_knowledge_default_model(session: &Session) -> bool {
    session.session_type == SessionType::Scheduled || session.schedule_id.is_some()
}

/// The completer behind a KB's `default_model`.
///
/// Issue #56 Gate H: `session` is the classification of the chat whose messages
/// this completer is about to digest. Required rather than optional — this
/// function's whole job is to build a provider the session row does not name, so
/// there is no bound provider for a later gate to consult.
async fn build_model_ref_completer(
    model: &ModelRef,
    session: crate::privacy::SessionClassification,
    cancel: Option<CancellationToken>,
) -> anyhow::Result<(
    Box<dyn Completer>,
    ProviderTier,
    Option<crate::privacy::affiliation::ModelAffiliation>,
)> {
    if biorouter_mcp::knowledge::test_mode::env_enabled() {
        // No provider exists on this path, so there is no instance to read a
        // tier from — the same fail-safe-for-a-ratchet reasoning as the two
        // `build_completer` test-mode branches. Nothing leaves the process on
        // this path either, so there is nothing for Gate H to refuse.
        return Ok((
            Box::new(biorouter_mcp::knowledge::test_mode::TestModeCompleter),
            ProviderTier::Public,
            // No provider, so no affiliation to read. `None` is what a public
            // model carries, and the tier beside it already says Public.
            None,
        ));
    }

    let provider = build_model_ref_provider(
        model,
        session,
        "digesting this conversation",
        "the knowledge base's default model",
    )
    .await?;
    let (completer, tier, affiliation) = ProviderCompleter::paired(provider);
    let completer = match cancel {
        Some(cancel) => completer.cancelled_by(cancel),
        None => completer,
    };
    Ok((Box::new(completer), tier, affiliation))
}

/// The provider behind a [`ModelRef`], past **Gate H's ratcheting half**.
///
/// Split out of [`build_model_ref_completer`] so a caller that needs the
/// provider itself — a batch, which mints one completer per source from one
/// `Arc` — reaches the gate through this function instead of writing a second
/// copy of it. Gate H exists once in this file, and both knowledge paths run it.
///
/// `session` is the classification of the session whose content is about to
/// travel. It has to be passed: this function's whole job is to build a provider
/// the session row does not name, so there is no bound provider for a later gate
/// to consult. `what` and `env_key_to_name` are Gate H's own two strings — the
/// feature named in the refusal and the knob that fixes it — and they differ per
/// caller, which is why they are arguments rather than constants here.
///
/// ⚠ **It asks [`crate::privacy::assert_alt_provider_matches_session`], not its
/// laxer sibling, and that is the whole of the fix for the Gate H finding.** The
/// tier of the provider built here does not stay in this process: it becomes
/// `SourceIngestArgs::caller_capability` / `ConversationIngestArgs::caller_capability`,
/// which crosses to `caller_is_private` and lands in
/// `knowledge::tier::raise_unlocked` — a permanent, monotone ratchet on a
/// knowledge base. So the upward choice `bind_allowed` waves through (a PRIVATE
/// provider named by a PUBLIC chat) is not harmless here: it privatises that
/// chat's own base for good, after which every KB read choke point refuses the
/// chat with `tier::KB_PRIVATE_REFUSAL`. Measured before the fix: the tool
/// returned an ordinary report (*"Curated 0 of 1 source(s) … on ollama/qwen3"*)
/// while `tier::is_private` flipped to `true` and the public caller's
/// `can_reach` to `false` — a base lost to a failed ingest.
///
/// This is the choke point rather than the tool's own handler because both
/// knowledge paths reach an alternate provider through here: the `model`
/// argument of `platform__ingest_source` (a name the MODEL wrote) and a base's
/// stored `default_model` on a scheduled digest. Both end in the same ratchet,
/// so both take the same rule, and a third knowledge path cannot be added
/// without passing it.
pub(crate) async fn build_model_ref_provider(
    model: &ModelRef,
    session: crate::privacy::SessionClassification,
    what: &str,
    env_key_to_name: &str,
) -> anyhow::Result<std::sync::Arc<dyn crate::providers::base::Provider>> {
    let model_config = ModelConfig::new(&model.model)?;
    let provider = crate::providers::create(&model.provider, model_config).await?;
    // AFTER `create`: the tier belongs to the instance that was resolved, not to
    // the name the manifest asked for. Constructing it discloses nothing.
    crate::privacy::assert_alt_provider_matches_session(
        what,
        provider.as_ref(),
        session,
        env_key_to_name,
    )?;
    Ok(provider)
}

/// Slugify a display name into a valid KB id (lowercase, a-z0-9-, no leading /
/// trailing / doubled dashes, ≤64 chars). Mirrors the service's own rule.
pub fn slugify_kb_name(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    trimmed.chars().take(64).collect::<String>()
}

fn internal(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
}

fn invalid_params(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, e.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::{
        ingest_summary, kb_caller, resolve_target_kb, should_use_knowledge_default_model,
        slugify_kb_name, KbCaller,
    };
    use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};
    use crate::privacy::{CallCapability, ProviderTier};
    use crate::session::session_manager::{Session, SessionType};
    use biorouter_mcp::knowledge::affiliation::CallerAffiliation;
    use biorouter_mcp::knowledge::service::KnowledgeService;
    use std::path::PathBuf;

    /// The two callers the pre-finding-17 filter could not tell apart: both are
    /// PRIVATE, so the tier axis says "reachable" for both, and only the
    /// affiliation axis separates them.
    fn private_at(institution: &str) -> KbCaller {
        KbCaller::new(
            true,
            CallerAffiliation::Institution(institution.to_string()),
        )
    }

    /// A private, LOCAL model — the caller DR-26 clears everywhere, because it
    /// transfers nothing. The pre-DR-26 meaning of "a private caller".
    fn private_local() -> KbCaller {
        KbCaller::new(true, CallerAffiliation::Local)
    }

    fn public_caller() -> KbCaller {
        KbCaller::restricted()
    }

    /// Pre-existing bug: the KB-less target came from the **machine-wide**
    /// `.active-kb`, while every other surface — the chat chip, kb_set_active,
    /// workflows, the apps platform — writes session-scoped state. A
    /// Meditation/workflow session whose KB was set per session therefore
    /// ingested into whatever the machine happened to point at.
    #[test]
    fn resolve_target_kb_uses_the_session_primary_not_the_machine_default() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("machine-kb", "Machine", None)?;
        svc.create_base("session-kb", "Session", None)?;
        svc.set_primary_persisted(Some("machine-kb"))?;
        svc.set_primary_for_session("chat-1", Some("session-kb"))?;

        let args = serde_json::json!({});
        let public = public_caller();
        assert_eq!(
            resolve_target_kb(&svc, &args, "chat-1", &public)?,
            "session-kb"
        );
        assert_eq!(
            resolve_target_kb(&svc, &args, "chat-2", &public)?,
            "machine-kb",
            "a chat that never chose one still inherits the machine pointer"
        );

        svc.set_primary_persisted(None)?;
        let err = resolve_target_kb(&svc, &args, "chat-9", &public)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("machine-kb, session-kb") && err.contains("kb_id"),
            "the error must list the candidates and the fix, got: {err}"
        );
        Ok(())
    }

    /// A KB-less write must name the base it wrote to, in the text the model
    /// and the user both read.
    #[test]
    fn ingest_summary_names_the_target_base() {
        let summary = ingest_summary(2, 0, "my-kb", "src-1", "abcdef1234567890", 7);
        assert!(summary.contains("'my-kb'"), "got: {summary}");
        assert!(summary.contains("abcdef12") && !summary.contains("abcdef123"));
        assert!(
            !summary.contains("skipped"),
            "a clean run must not mention a refusal: {summary}"
        );
    }

    /// Issue #56. When Gate G drops some of the requested chats, the model is
    /// told a COUNT and the fix — never an id, a title or a working directory
    /// (§11.4 classifies all three as content).
    #[test]
    fn ingest_summary_reports_refusals_as_a_count_and_names_nothing() {
        let summary = ingest_summary(1, 2, "my-kb", "src-1", "abcdef1234567890", 7);
        assert!(
            summary.contains("2 private conversation(s) were skipped"),
            "{summary}"
        );
        assert!(summary.contains("private model"), "{summary}");
    }

    #[test]
    fn slugify_produces_valid_ids() {
        assert_eq!(slugify_kb_name("My Research Notes!"), "my-research-notes");
        assert_eq!(slugify_kb_name("  Soul  "), "soul");
        assert_eq!(slugify_kb_name("a / b -- c"), "a-b-c");
        assert!(slugify_kb_name("***").is_empty());
    }

    #[test]
    fn an_inaccessible_explicit_id_is_indistinguishable_from_an_absent_one() -> anyhow::Result<()> {
        let present_tmp = tempfile::TempDir::new()?;
        let present = KnowledgeService::new(present_tmp.path().to_path_buf());
        present.create_base("restricted-fixture", "Restricted Fixture", None)?;
        crate::knowledge::tier::raise_unlocked(present_tmp.path(), "restricted-fixture", true)?;

        let absent_tmp = tempfile::TempDir::new()?;
        let absent = KnowledgeService::new(absent_tmp.path().to_path_buf());
        let args = serde_json::json!({ "kb_id": "restricted-fixture" });
        let present_error = resolve_target_kb(&present, &args, "chat-9", &public_caller())
            .unwrap_err()
            .to_string();
        let absent_error = resolve_target_kb(&absent, &args, "chat-9", &public_caller())
            .unwrap_err()
            .to_string();

        assert_eq!(present_error, absent_error);
        assert_eq!(
            present_error,
            "knowledge base 'restricted-fixture' does not exist"
        );
        assert_eq!(
            resolve_target_kb(&present, &args, "chat-9", &private_local())?,
            "restricted-fixture",
            "an authorized caller must retain explicit-id access"
        );
        Ok(())
    }

    #[test]
    fn a_new_name_has_the_same_success_shape_when_hidden_or_absent() -> anyhow::Result<()> {
        let private_tmp = tempfile::TempDir::new()?;
        let private = KnowledgeService::new(private_tmp.path().to_path_buf());
        private.create_base("restricted-cohort", "Restricted Cohort", None)?;
        crate::knowledge::tier::raise_unlocked(private_tmp.path(), "restricted-cohort", true)?;
        crate::knowledge::tier::raise_affiliation_unlocked(
            private_tmp.path(),
            "restricted-cohort",
            &CallerAffiliation::Institution("fixture-institution".to_string()),
        )?;

        let args = serde_json::json!({ "new_kb_name": "Restricted Cohort" });
        let alongside_hidden = resolve_target_kb(&private, &args, "chat-9", &public_caller())?;

        let absent_tmp = tempfile::TempDir::new()?;
        let absent = KnowledgeService::new(absent_tmp.path().to_path_buf());
        let alongside_absent = resolve_target_kb(&absent, &args, "chat-9", &public_caller())?;

        let public_tmp = tempfile::TempDir::new()?;
        let public = KnowledgeService::new(public_tmp.path().to_path_buf());
        public.create_base("restricted-cohort", "Restricted Cohort", None)?;
        let alongside_visible = resolve_target_kb(&public, &args, "chat-9", &public_caller())?;

        for created in [&alongside_hidden, &alongside_absent, &alongside_visible] {
            assert!(created.starts_with("kb-"), "non-opaque id: {created}");
            assert_eq!(created.len(), 35, "unexpected opaque-id shape: {created}");
            assert_ne!(created, "restricted-cohort");
        }
        assert_ne!(alongside_hidden, alongside_absent);
        assert_ne!(alongside_hidden, alongside_visible);
        assert_ne!(alongside_absent, alongside_visible);
        assert!(private
            .list_bases()?
            .iter()
            .any(|base| base.id == alongside_hidden && base.name == "Restricted Cohort"));
        assert!(absent
            .list_bases()?
            .iter()
            .any(|base| base.id == alongside_absent && base.name == "Restricted Cohort"));
        assert!(public
            .list_bases()?
            .iter()
            .any(|base| base.id == alongside_visible && base.name == "Restricted Cohort"));

        let selected = serde_json::json!({
            "kb_id": "restricted-cohort",
            "new_kb_name": "Restricted Cohort"
        });
        assert_eq!(
            resolve_target_kb(&public, &selected, "chat-9", &public_caller())?,
            "restricted-cohort",
            "an accessible base explicitly selected by id must not take the name-collision path"
        );
        Ok(())
    }

    #[test]
    fn an_inaccessible_primary_falls_through_to_the_filtered_candidates() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("default", "Default", None)?;
        svc.create_base("restricted-primary", "Restricted Primary", None)?;
        crate::knowledge::tier::raise_unlocked(tmp.path(), "restricted-primary", true)?;
        svc.set_primary_for_session("chat-9", Some("restricted-primary"))?;

        let public = resolve_target_kb(&svc, &serde_json::json!({}), "chat-9", &public_caller())
            .unwrap_err()
            .to_string();
        assert!(
            public.contains("default"),
            "the reachable candidate vanished: {public}"
        );
        assert!(!public.contains("restricted-primary"), "{public}");
        assert_eq!(
            resolve_target_kb(&svc, &serde_json::json!({}), "chat-9", &private_local())?,
            "restricted-primary"
        );
        Ok(())
    }

    /// Issue #56. `resolve_target_kb` is `kb_id_or_primary`'s twin one crate
    /// over, and Task 10C's fix cannot reach it: that one is in
    /// `biorouter-mcp`'s `KnowledgeServer`, this one is in `biorouter`'s
    /// `Agent`. Same rule — OMIT. A barrier that refuses a read and then hands
    /// the model the identifier of the thing it refused is not a barrier.
    #[test]
    fn the_no_target_error_names_only_the_bases_the_caller_may_reach() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("default", "Default", None)?;
        svc.create_base("omop", "OMOP", None)?;
        crate::knowledge::tier::raise_unlocked(tmp.path(), "omop", true)?;

        let args = serde_json::json!({});
        let public = resolve_target_kb(&svc, &args, "chat-9", &public_caller())
            .unwrap_err()
            .to_string();
        assert!(
            public.contains("default"),
            "the public base must still be offered: {public}"
        );
        assert!(
            !public.contains("omop"),
            "the no-target error enumerated a private base: {public}"
        );

        // Both directions: a private model still sees both, or the filter is
        // just "refuse everyone" and the feature has quietly stopped working.
        let private = resolve_target_kb(&svc, &args, "chat-9", &private_local())
            .unwrap_err()
            .to_string();
        assert!(
            private.contains("default") && private.contains("omop"),
            "a private model was denied its own bases: {private}"
        );
        Ok(())
    }

    /// The filter runs per id and BEFORE the emptiness check, so a chat whose
    /// only base is private is told it has none rather than handed
    /// `(one of: )` — which is both useless and a tell.
    #[test]
    fn a_chat_whose_only_base_is_private_is_told_it_has_none() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("omop", "OMOP", None)?;
        crate::knowledge::tier::raise_unlocked(tmp.path(), "omop", true)?;

        let err = resolve_target_kb(&svc, &serde_json::json!({}), "chat-9", &public_caller())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("this chat has none"),
            "expected the no-bases branch, got: {err}"
        );
        assert!(!err.contains("omop"), "{err}");
        assert!(!err.contains("one of"), "an empty candidate list: {err}");
        Ok(())
    }

    /// **Audit finding 17, second spelling.** The candidate list asked the tier
    /// axis alone, so both callers below — each of them PRIVATE — got the same
    /// answer, and the Stanford one was handed the id of a base the barrier then
    /// refused. That id is the one argument that makes the explicit-`kb_id`
    /// branch of this very function reachable.
    ///
    /// The discrimination is the point: `default` (unclaimed) must survive for
    /// BOTH, or the filter is just "refuse the second caller" and the test would
    /// pass against a fix that broke the feature.
    #[test]
    fn the_candidate_list_asks_the_affiliation_axis_not_only_the_tier() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("default", "Default", None)?;
        svc.create_base("omop", "OMOP", None)?;
        crate::knowledge::tier::raise_unlocked(tmp.path(), "omop", true)?;
        crate::knowledge::tier::raise_affiliation_unlocked(
            tmp.path(),
            "omop",
            &CallerAffiliation::Institution("ucsf".to_string()),
        )?;

        let args = serde_json::json!({});

        let ucsf = resolve_target_kb(&svc, &args, "chat-9", &private_at("ucsf"))
            .unwrap_err()
            .to_string();
        assert!(
            ucsf.contains("omop") && ucsf.contains("default"),
            "the institution that owns the base was denied its own id: {ucsf}"
        );

        let stanford = resolve_target_kb(&svc, &args, "chat-9", &private_at("stanford"))
            .unwrap_err()
            .to_string();
        assert!(
            !stanford.contains("omop"),
            "the candidate list named a base the barrier refuses across an \
             institutional boundary: {stanford}"
        );
        assert!(
            stanford.contains("default"),
            "the unclaimed base must still be offered: {stanford}"
        );

        // A private model that states nothing is DR-26's restrictive caller: it
        // mismatches every claimed base. `Unstated` reaching `omop` would mean
        // the filter is reading the tier bit and calling it an affiliation.
        let unstated_caller = KbCaller::new(true, CallerAffiliation::Unstated);
        let unstated = resolve_target_kb(&svc, &args, "chat-9", &unstated_caller)
            .unwrap_err()
            .to_string();
        assert!(!unstated.contains("omop"), "{unstated}");
        assert!(stanford.contains("default"), "{unstated}");

        // …and a LOCAL model transfers nothing, so it clears the base. Without
        // this row the fix could be "refuse every private caller".
        let local = resolve_target_kb(&svc, &args, "chat-9", &private_local())
            .unwrap_err()
            .to_string();
        assert!(local.contains("omop"), "a local model was denied: {local}");
        Ok(())
    }

    /// The wiring, not the filter. `resolve_target_kb` could ask the barrier
    /// perfectly and still be handed half a caller: the production call site
    /// used to read `p.tier()` and nothing else, so a correct filter would have
    /// received `Unstated` for every chat and quietly refused every claimed
    /// base. This drives the crossing the handler actually performs.
    #[test]
    fn the_production_crossing_carries_both_axes_off_one_sample() {
        let ucsf = CallCapability::for_test_affiliated(
            ProviderTier::Private,
            true,
            Some(ModelAffiliation::institution(InstitutionId::new("UCSF"))),
        );
        assert_eq!(
            kb_caller(ucsf),
            KbCaller::new(true, CallerAffiliation::Institution("ucsf".to_string())),
            "the affiliation axis was dropped on the way to the barrier"
        );

        // Public collapses to the restrictive pair on both fields — the
        // fail-closed direction an unbound provider takes.
        let public = CallCapability::for_test(ProviderTier::Public, true);
        assert_eq!(kb_caller(public), KbCaller::restricted());

        // ⚠ `enforced` must NOT ride along: the toggle is read once, inside
        // `tier::assert_reachable`. Two capabilities differing only in
        // `enforced` must cross to the same caller.
        assert_eq!(
            kb_caller(CallCapability::for_test(ProviderTier::Private, true)),
            kb_caller(CallCapability::for_test(ProviderTier::Private, false)),
        );
    }

    /// Issue #56 Gate H. A scheduled knowledge job prefers the target KB's
    /// `default_model` over the agent's own provider, so the transcripts it
    /// digests go to a provider the session row never records — and
    /// `build_model_ref_completer` is reached from neither
    /// `Agent::update_provider` nor `Agent::reply`.
    #[tokio::test]
    async fn the_knowledge_default_model_obeys_the_barrier() {
        use super::build_model_ref_completer;
        use crate::privacy::SessionClassification;
        use biorouter_mcp::knowledge::types::ModelRef;
        use std::collections::HashMap;

        fn ollama_at(host: &str) -> HashMap<String, String> {
            HashMap::from([("OLLAMA_HOST".to_string(), host.to_string())])
        }
        let model = ModelRef {
            provider: "ollama".to_string(),
            model: "qwen3".to_string(),
        };

        let err = crate::config::with_config_overrides(
            // Not this machine ⇒ `tier()` says Public, from a real provider.
            ollama_at("https://api.example-saas.invalid"),
            build_model_ref_completer(&model, SessionClassification::Private, None),
        )
        .await
        // `Completer` is not `Debug`, so the Ok side cannot be unwrapped for a
        // message; match instead.
        .err()
        .expect("a private chat may not digest itself on a public model")
        .to_string();
        assert!(
            err.to_lowercase().contains("private"),
            "the refusal has to say why, got: {err}"
        );

        // Both directions, or the gate is just "refuse everyone".
        assert!(crate::config::with_config_overrides(
            ollama_at("http://localhost:11434"),
            build_model_ref_completer(&model, SessionClassification::Private, None),
        )
        .await
        .is_ok());
        assert!(crate::config::with_config_overrides(
            ollama_at("https://api.example-saas.invalid"),
            build_model_ref_completer(&model, SessionClassification::Public, None),
        )
        .await
        .is_ok());

        // The fourth cell, and the one this path needs the STRICTER half of Gate
        // H for: a PUBLIC session on a PRIVATE default model. `bind_allowed`
        // permits it — nothing leaks upward — but the tier that comes back is
        // what ratchets the target base, so permitting it hands a public
        // scheduled digest the power to privatise the base it writes into. It is
        // refused here and the refusal names the base's own knob.
        let raise = crate::config::with_config_overrides(
            ollama_at("http://localhost:11434"),
            build_model_ref_completer(&model, SessionClassification::Public, None),
        )
        .await
        .err()
        .expect("a public chat may not digest itself on the base's private default model")
        .to_string();
        assert!(raise.contains("private model"), "{raise}");
        assert!(
            raise.contains("knowledge base's default model"),
            "the refusal must name the knob that fixes it: {raise}"
        );
    }

    /// Issue #56 Gate H, the *wiring*. The test above proves
    /// `build_model_ref_completer` refuses; it says nothing about what the one
    /// production caller passes it. Hardcoding `SessionClassification::Public`
    /// at `conversation_ingest_completer`'s call to it would leave that test
    /// green and the barrier dead, so this one drives the real caller and lets
    /// the session row supply the classification.
    #[tokio::test]
    async fn the_production_ingest_caller_passes_this_session_s_own_classification() {
        use crate::privacy::SessionClassification;
        use biorouter_mcp::knowledge::types::ModelRef;
        use std::collections::HashMap;

        fn ollama_at(host: &str) -> HashMap<String, String> {
            HashMap::from([("OLLAMA_HOST".to_string(), host.to_string())])
        }
        const OFF_MACHINE: &str = "https://api.example-saas.invalid";
        const THIS_MACHINE: &str = "http://localhost:11434";

        let tmp = tempfile::TempDir::new().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("kb", "KB", None).unwrap();
        svc.set_default_model(
            "kb",
            Some(ModelRef {
                provider: "ollama".to_string(),
                model: "qwen3".to_string(),
            }),
        )
        .unwrap();

        let agent = crate::agents::Agent::new();

        // A SCHEDULED session, or `should_use_knowledge_default_model` is false
        // and the KB's model is never consulted at all.
        let mut session = test_session(SessionType::Scheduled, None);
        session.privacy_tier = SessionClassification::Private;

        let err = crate::config::with_config_overrides(
            ollama_at(OFF_MACHINE),
            agent.conversation_ingest_completer(&svc, "kb", &session, None),
        )
        .await
        .err()
        .expect("a private chat may not be digested on the KB's public default model")
        .message
        .to_string();
        assert!(
            err.to_lowercase().contains("private"),
            "the refusal has to say why, got: {err}"
        );

        // Both directions, or the caller could simply be passing `Private` for
        // everyone — which is not "the session's classification" either.
        assert!(crate::config::with_config_overrides(
            ollama_at(THIS_MACHINE),
            agent.conversation_ingest_completer(&svc, "kb", &session, None),
        )
        .await
        .is_ok());

        session.privacy_tier = SessionClassification::Public;
        assert!(
            crate::config::with_config_overrides(
                ollama_at(OFF_MACHINE),
                agent.conversation_ingest_completer(&svc, "kb", &session, None),
            )
            .await
            .is_ok(),
            "a public chat is unaffected by the same public default model"
        );
    }

    #[test]
    fn knowledge_default_model_is_reserved_for_scheduled_contexts() {
        let user = test_session(SessionType::User, None);
        assert!(!should_use_knowledge_default_model(&user));

        let scheduled = test_session(SessionType::Scheduled, None);
        assert!(should_use_knowledge_default_model(&scheduled));

        let scheduled_by_id = test_session(SessionType::User, Some("daily-meditation"));
        assert!(should_use_knowledge_default_model(&scheduled_by_id));
    }

    fn test_session(session_type: SessionType, schedule_id: Option<&str>) -> Session {
        Session {
            id: "s".to_string(),
            working_dir: PathBuf::from("."),
            name: "Test".to_string(),
            user_set_name: false,
            session_type,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            extension_data: Default::default(),
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            schedule_id: schedule_id.map(ToOwned::to_owned),
            workflow: None,
            user_workflow_values: None,
            conversation: None,
            message_count: 0,
            provider_name: None,
            model_config: None,
            diverged_from: None,
            branch_point_msg_uid: None,
            parent_session_id: None,
            privacy_tier: crate::privacy::SessionClassification::Public,
            privacy_reason: None,
        }
    }
}
