//! The approval card a `platform__*` management tool parks before it changes the
//! user's own setup — and the one place that card is built.
//!
//! Two tools raise it: `platform__manage_workflow` (save, delete, import,
//! schedule) and `platform__manage_schedule` (create, run_now, pause, unpause,
//! delete, kill). They used to disagree. The workflow tool asked before writing a
//! YAML file, while the schedule tool created a standing daily agent run with no
//! card at all — the cheaper-to-undo action gated and the more consequential one
//! not (QA 2026-09-10, finding F1). One function is what stops the two drifting
//! apart again: the card's shape, its proof requirement and the way its answer is
//! read cannot differ between tools that both call it.
//!
//! ## Why the card appears in every permission mode
//!
//! It is parked by the handler itself, through [`PendingUserActions`], not raised
//! by the permission inspector, so the permission mode never decides whether it
//! appears. Autonomous mode — where the inspector asks nothing — still gets it,
//! and that is the point: an agent in Autonomous mode is the one best placed to
//! set up a standing run nobody asked for. `requires_user_proof` means only a
//! surface that can prove a person clicked may allow it, so a model cannot
//! approve its own schedule over daemon HTTP. A user who wants the change says
//! yes; an agent never gets it done on its own.

use std::time::Duration;

use rmcp::model::{ErrorCode, ErrorData};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::pending_user_action::{
    PendingUserActions, ToolApprovalRequest, UserActionOutcome, UserActionRequest,
};
use crate::permission::tool_risk::ToolRisk;

/// How long a platform-tool change waits for the user to answer its card.
const PLATFORM_MUTATION_APPROVAL_TTL: Duration = Duration::from_secs(300);

/// What the card asks about.
pub(crate) struct PlatformApproval<'a> {
    /// The tool the card names — `Run Manage Schedule?` is read off this.
    pub tool_name: &'a str,
    /// The verb, for the refusal text the model reads.
    pub action: &'a str,
    /// The chat the card is shown in.
    pub session_id: &'a str,
    /// The sentence at the top of the card. It carries the decision, so it says
    /// what will happen in words rather than restating the arguments.
    pub summary: &'a str,
    /// What the card shows under the sentence. Not necessarily the call's own
    /// arguments: a caller whose raw arguments are opaque (a base64 link, a bare
    /// path) passes the thing they stand for instead.
    pub arguments: &'a Value,
    pub risk: ToolRisk,
}

fn refused(message: String) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, message, None)
}

/// Park an approval card the user must actually answer, and return only once
/// they have allowed the change.
///
/// Every outcome other than an allow — a denial, a timeout, a Stop, a card that
/// could not be shown — is an error that says nothing was changed, because the
/// caller has not changed anything yet and must not.
pub(crate) async fn require_platform_approval(
    approval: PlatformApproval<'_>,
    cancellation_token: Option<&CancellationToken>,
) -> Result<(), ErrorData> {
    let PlatformApproval {
        tool_name,
        action,
        session_id,
        summary,
        arguments,
        risk,
    } = approval;

    if session_id.is_empty() {
        return Err(refused(format!(
            "`{action}` needs an active conversation so Biorouter can show its approval card"
        )));
    }

    let approval_arguments = arguments
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    let request = UserActionRequest::ToolApproval(ToolApprovalRequest {
        tool_name: tool_name.to_string(),
        arguments: approval_arguments.clone(),
        prompt: Some(summary.to_string()),
        risk: Some(risk),
        preview: crate::conversation::tool_preview::ToolPreview::for_tool_call(
            tool_name,
            &approval_arguments,
        ),
        requires_user_proof: true,
    });

    let parked = PendingUserActions::global().park(Some(session_id), None, request);
    let outcome = parked
        .wait(PLATFORM_MUTATION_APPROVAL_TTL, cancellation_token)
        .await;

    match outcome {
        UserActionOutcome::Approved { .. }
            if !cancellation_token.is_some_and(CancellationToken::is_cancelled) =>
        {
            Ok(())
        }
        UserActionOutcome::Approved { .. } => Err(refused(format!(
            "`{action}` was cancelled after approval and before anything changed"
        ))),
        UserActionOutcome::Denied { .. } => Err(refused(format!(
            "The user declined the `{action}`. Nothing was changed."
        ))),
        other => Err(refused(format!(
            "`{action}` was not approved ({other:?}). Nothing was changed."
        ))),
    }
}
