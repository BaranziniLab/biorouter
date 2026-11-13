//! Usage of provider calls a chat makes outside its turns: the stall check, a
//! prompt hook's judge, the permission judge, the chat's title, a knowledge
//! macro's sub-agent, MCP sampling, generating a workflow from the chat, and a
//! compaction that was billed but did not land.
//! Each is billed by the provider, so each is written to the usage ledger as a
//! side call ([`super::SideUsageEntry`]):
//! counted in the chat's tokens and dollars and in the Usage panel, never as a
//! turn, never moving the context gauge or `updated_at` (R-30).
//!
//! A call site names its chat in one of three ways, in this order: an explicit
//! session id, the chat scoped with [`with_side_usage_chat`], or
//! `session_context::current_session_id()`. A call with none of them is not
//! recorded, because no chat can be charged for it.

use super::{SessionManager, SessionStorage, SideUsageEntry};
use crate::providers::base::ProviderUsage;
use std::future::Future;
use std::sync::Arc;

/// Every side call's ledger key starts with this, which is how the usage
/// queries tell a side call from a turn (`side_request_sql!`).
pub const SIDE_EVENT_KEY_PREFIX: &str = "side:";

tokio::task_local! {
    static SIDE_USAGE_CHAT: (Arc<SessionStorage>, String);
}

/// What a side call was for. Part of its ledger key, so a row says which
/// kind of call it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideCall {
    StallCheck,
    PromptHook,
    PermissionJudge,
    SessionName,
    KnowledgeMacro,
    McpSampling,
    /// Generating a workflow from a chat (`Agent::create_workflow_for_session`).
    WorkflowCreation,
    /// A compaction whose summary was billed but did not land: it failed, or
    /// the history moved under it (`context_mgmt::record_unapplied_compaction_usage`).
    Compaction,
}

impl SideCall {
    pub fn as_str(self) -> &'static str {
        match self {
            SideCall::StallCheck => "stall_check",
            SideCall::PromptHook => "prompt_hook",
            SideCall::PermissionJudge => "permission_judge",
            SideCall::SessionName => "session_name",
            SideCall::KnowledgeMacro => "knowledge_macro",
            SideCall::McpSampling => "mcp_sampling",
            SideCall::WorkflowCreation => "workflow_creation",
            SideCall::Compaction => "compaction",
        }
    }
}

/// Run `f` with `session_id` (in `manager`'s store) as the chat side calls
/// inside it are charged to. Does not follow a `tokio::spawn`.
pub async fn with_side_usage_chat<F: Future>(
    manager: &SessionManager,
    session_id: &str,
    f: F,
) -> F::Output {
    SIDE_USAGE_CHAT
        .scope((Arc::clone(&manager.storage), session_id.to_string()), f)
        .await
}

/// Record one side call's usage against its chat. Best-effort: a call with no
/// chat, no reported tokens, or a failed write is skipped (and logged at
/// debug), never an error for the caller. `provider_name` is used when the
/// usage does not name the provider that served it.
pub async fn record_side_call_usage(
    session_id: Option<&str>,
    call: SideCall,
    provider_name: &str,
    usage: &ProviderUsage,
) {
    let resolved = match session_id.filter(|id| !id.is_empty()) {
        Some(id) => Some((None, id.to_string())),
        None => SIDE_USAGE_CHAT
            .try_with(|(storage, id)| (Some(Arc::clone(storage)), id.clone()))
            .ok()
            .or_else(|| crate::session_context::current_session_id().map(|id| (None, id))),
    };
    let Some((storage, session_id)) = resolved else {
        tracing::debug!(
            call = call.as_str(),
            "side call usage not recorded: no chat to charge"
        );
        return;
    };
    let manager = match storage {
        Some(storage) => SessionManager { storage },
        None => SessionManager::instance(),
    };
    if let Err(error) = record_with(&manager, &session_id, call, provider_name, usage).await {
        tracing::debug!(
            call = call.as_str(),
            "side call usage not recorded: {error}"
        );
    }
}

/// [`record_side_call_usage`] into an explicit store. `Ok(false)` when there
/// was nothing to record or the chat does not exist.
pub async fn record_with(
    manager: &SessionManager,
    session_id: &str,
    call: SideCall,
    provider_name: &str,
    usage: &ProviderUsage,
) -> anyhow::Result<bool> {
    let tokens = &usage.usage;
    let cache_read = i64::from(tokens.cache_read_input_tokens.unwrap_or(0));
    let cache_creation = i64::from(tokens.cache_creation_input_tokens.unwrap_or(0));
    if tokens.input_tokens.is_none() && tokens.output_tokens.is_none() && cache_read == 0 {
        return Ok(false);
    }
    let provider = usage
        .provider
        .clone()
        .filter(|provider| !provider.is_empty())
        .unwrap_or_else(|| provider_name.to_string());
    let entry = SideUsageEntry {
        event_key: format!(
            "{SIDE_EVENT_KEY_PREFIX}{}:{}",
            call.as_str(),
            uuid::Uuid::new_v4()
        ),
        session_id: session_id.to_string(),
        provider,
        model_id: usage.model.clone(),
        input_tokens: tokens.input_tokens.map(i64::from),
        output_tokens: tokens.output_tokens.map(i64::from),
    };
    manager
        .storage
        .apply_side_usage_with_cache(entry, cache_read, cache_creation)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::Usage;
    use crate::session::session_manager::{SessionType, UsageLedgerEntry};
    use tempfile::TempDir;

    fn judge_usage() -> ProviderUsage {
        ProviderUsage::new(
            "gpt-5.6-sol-2026-07-09".to_string(),
            Usage::new(Some(1_000), Some(10), Some(1_510)).with_cache(Some(500), None),
        )
    }

    #[tokio::test]
    async fn a_side_call_is_billed_but_is_not_a_turn_and_is_not_activity() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let chat = sm
            .create_session(temp.path().to_path_buf(), "c".into(), SessionType::User)
            .await
            .unwrap();
        let before = sm.get_session(&chat.id, false).await.unwrap().updated_at;

        let written = record_with(
            &sm,
            &chat.id,
            SideCall::StallCheck,
            "versa_azure",
            &judge_usage(),
        )
        .await
        .unwrap();
        assert!(written);

        let session = sm.get_session(&chat.id, false).await.unwrap();
        assert_eq!(session.updated_at, before, "a side call is not activity");
        assert_eq!(session.accumulated_total_tokens, Some(1_510));
        assert_eq!(
            session.total_tokens, None,
            "the context gauge does not move"
        );

        let rows = sm.get_session_model_usage(&chat.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].turns, 0);
        assert_eq!(rows[0].total_tokens, Some(1_510));
        assert_eq!(rows[0].cache_read_tokens, Some(500));
        let want = (1_000.0 * 4.0 + 500.0 * 0.40 + 10.0 * 20.0) / 1e6;
        assert!((rows[0].cost.unwrap() - want).abs() < 1e-12, "{rows:?}");

        sm.apply_usage_event(UsageLedgerEntry {
            event_key: "turn-1".into(),
            session_id: chat.id.clone(),
            schedule_id: None,
            current_total_tokens: Some(100),
            current_input_tokens: Some(90),
            current_output_tokens: Some(10),
            billed_total_tokens: Some(100),
            input_tokens: Some(90),
            output_tokens: Some(10),
            model_id: Some("gpt-5.6-sol-2026-07-09".into()),
            provider: Some("versa_azure".into()),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
        })
        .await
        .unwrap();
        let rows = sm.get_session_model_usage(&chat.id).await.unwrap();
        assert_eq!(rows[0].turns, 1);
        let summary = sm.get_usage_summary().await.unwrap();
        assert_eq!(summary.all_time.turns, 1);
        assert_eq!(summary.all_time.total_tokens, Some(1_610));
    }

    #[tokio::test]
    async fn a_scoped_chat_is_charged_and_a_call_with_no_chat_is_not() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let chat = sm
            .create_session(temp.path().to_path_buf(), "c".into(), SessionType::User)
            .await
            .unwrap();

        with_side_usage_chat(&sm, &chat.id, async {
            record_side_call_usage(None, SideCall::SessionName, "versa_azure", &judge_usage())
                .await;
        })
        .await;
        let rows = sm.get_session_model_usage(&chat.id).await.unwrap();
        assert_eq!(rows[0].total_tokens, Some(1_510));
        assert_eq!(rows[0].provider.as_deref(), Some("versa_azure"));

        // No usage reported: nothing is written.
        let empty = ProviderUsage::new("m".into(), Usage::default());
        assert!(
            !record_with(&sm, &chat.id, SideCall::PromptHook, "p", &empty)
                .await
                .unwrap()
        );
        // A chat that does not exist: nothing is written.
        assert!(
            !record_with(&sm, "20990101_1", SideCall::PromptHook, "p", &judge_usage())
                .await
                .unwrap()
        );
    }
}
