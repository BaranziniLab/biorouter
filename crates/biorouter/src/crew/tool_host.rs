//! The crate-boundary bridge behind a chat's two person tools, `crew__read` and `crew__change`
//! (account ownership AO-12, "A chat can do what the Crew view does").
//!
//! The tools are advertised by the `crew` platform extension, which lives in this crate, but
//! most of what they do lives with the daemon's routes in `biorouter-server`: name resolution,
//! setup jobs, tasks, grants, transfers. The daemon implements [`CrewToolHost`] over its state
//! and [`install`]s it at start, exactly as it installs
//! [`crate::workspace_services::WorkspaceServices`]; the extension reads it lazily per call.
//! When nothing is installed (CLI-direct, an embedder, this crate's own tests) the person tools
//! are not offered at all, never offered and then refused.
//!
//! **The division of labour**, which is the contract between the two crates:
//!
//! * The **extension** (`agents::crew_extension`) decides WHICH chats get the tools and
//!   forwards. A chat is offered them only when it holds no Crew grant, is a person's own
//!   `SessionType::User` chat (not an app's chat, a subagent or a scheduled run), a host is
//!   installed, and, for `crew__change`, a person's proof can be obtained
//!   ([`crate::pending_user_action::user_proof_available`]). The same rule is applied where the
//!   model's tool list is built, at dispatch, and again in the tool itself. A Claude Code or
//!   Codex chat is such a chat too (owner ruling D3): its child is bridged the two tools from
//!   the chat's own roster, each change still asks the person on a card, and a read takes the
//!   bridge's capability (Public). The extension checks that `action` is one of the tool's
//!   actions and that `params` is an object, and nothing else.
//! * The **host** does everything else, with the same functions the HTTP routes call, so a
//!   chat, the Crew view and the CLI share one implementation: it validates `params`,
//!   resolves names to IDs, enforces the read consent and the privacy gate with
//!   [`PersonToolCall::capability`] (sampled once, never re-read), parks the proof-backed
//!   approvals through [`PersonToolCall::approvals`], re-checks that the chat is still
//!   ungranted after an approval ([`PersonToolCall::ensure_still_ungranted`]), runs the action,
//!   and stops waiting (never the job) when [`PersonToolCall::cancel`] fires.
//!
//! ⚠ **Person authority; needs human review.** Everything behind this trait acts as the person.

use std::future::Future;
use std::sync::{Arc, OnceLock};

use tokio_util::sync::CancellationToken;

use crate::pending_user_action::PendingUserActions;
use crate::privacy::CallCapability;

/// What `crew__read` can do: the read half of the Crew view (AO-12's table). Each name is its
/// `biorouter crew` path joined with `_`, so the CLI stays the vocabulary. `describe` returns
/// any action's parameters and an example, so the schemas every chat carries stay small.
pub const READ_ACTIONS: &[&str] = &[
    "describe",
    "status",
    "credentials_status",
    "connections_show",
    "connections_preview",
    "connections_invitation",
    "setup_check",
    "setup_status",
    "setup_list",
    "workspace_show",
    "members",
    "enroll_pending",
    "invites_list",
    "teams_list",
    "channels_list",
    "history",
    "search",
    "files_list",
    "files_status",
    "files_pending",
    "context_show",
    "privacy_show",
    "profile_show",
    "profile_devices",
    "defaults_show",
    "tasks_list",
    "tasks_watch",
    "grants_list",
];

/// What `crew__change` can do (AO-12's table, as the owner's D1 ruling amends it). Every one
/// waits for the person: an ordinary change parks one proof-backed approval and runs only when
/// they allow it, and `auth`, `credentials_unlock` and `credentials_init` park a card that opens
/// the person's own Sign in window or vault prompt, so what they type never reaches the chat.
/// Deliberately absent, each a person-only control: daemon start and stop,
/// `workspace bootstrap`, `enroll prepare` and `accept`, `connections prepare` and `save`, and
/// granting a chat Crew access.
pub const CHANGE_ACTIONS: &[&str] = &[
    "setup",
    "setup_cancel",
    "host",
    "open",
    "join",
    "rejoin",
    "connect",
    "disconnect",
    "auth",
    "credentials_unlock",
    "credentials_init",
    "credentials_lock",
    "connections_remove",
    "connections_update",
    "connections_set_access",
    "workspace_rename",
    "workspace_delete",
    "workspace_governance",
    "context_draft",
    "context_set",
    "enroll_invite",
    "enroll_cancel",
    "enroll_approve",
    "enroll_revoke",
    "members_add",
    "teams_create",
    "teams_rename",
    "teams_join",
    "teams_delete",
    "channels_create",
    "channels_rename",
    "channels_archive",
    "channels_join",
    "channels_delete",
    "channels_read",
    "invites_send",
    "invites_accept",
    "ownership_offer",
    "ownership_accept",
    "remove_member",
    "profile_set",
    "profile_revoke_device",
    "profile_devices_acknowledge",
    "send",
    "files_share",
    "files_download",
    "files_reference",
    "files_pause",
    "files_resume",
    "files_forget",
    "tasks_ask",
    "tasks_cancel",
    "grants_revoke",
    "privacy_set_personal",
    "privacy_set_workspace",
    "defaults_set",
    "defaults_clear",
];

/// One of the two person tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PersonTool {
    /// `crew__read`, annotated read-only.
    Read,
    /// `crew__change`, annotated destructive. Every action waits for a person's approval.
    Change,
}

impl PersonTool {
    /// Both tools, in the order the extension lists them.
    pub const ALL: [PersonTool; 2] = [PersonTool::Read, PersonTool::Change];

    /// The name the model calls it by: `crew__read` or `crew__change`.
    pub const fn name(self) -> &'static str {
        match self {
            PersonTool::Read => "crew__read",
            PersonTool::Change => "crew__change",
        }
    }

    /// The name the `crew` extension lists it under, without the extension's prefix.
    pub const fn tool_name(self) -> &'static str {
        match self {
            PersonTool::Read => "read",
            PersonTool::Change => "change",
        }
    }

    /// The tool's actions: [`READ_ACTIONS`] or [`CHANGE_ACTIONS`].
    pub const fn actions(self) -> &'static [&'static str] {
        match self {
            PersonTool::Read => READ_ACTIONS,
            PersonTool::Change => CHANGE_ACTIONS,
        }
    }

    /// The person tool the model-facing `name` names (`crew__read`), if any.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// The person tool the extension-local `name` names (`read`), if any.
    pub fn from_tool_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.tool_name() == name)
    }
}

/// One person-tool call, as the extension hands it to the host.
///
/// Everything the host needs to act and to park a proof-backed approval for this call:
/// the chat ([`Self::session_id`], the approval card's delivery scope), the registry to park
/// on ([`Self::approvals`]) and the turn's token ([`Self::cancel`]), which is what
/// `PendingUserAction::wait` takes. No bridge-lease owner is passed, so a card parks with
/// `owner: None`, as `extensionmanager__install_extension`'s does. A call bridged from a
/// coding agent (D3, see the module docs) is bounded instead by the bridge's call deadline:
/// the host caps every card, job wait and follow task to it, under the bridge's 570 seconds.
pub struct PersonToolCall {
    /// Which tool was called.
    pub tool: PersonTool,
    /// The action, already checked to be one of [`PersonTool::actions`].
    pub action: String,
    /// The action's parameters: always a JSON object, `{}` when the model sent none. Not
    /// validated; that is the host's.
    pub params: serde_json::Value,
    /// The chat the call came from. It held no Crew grant when the call was admitted.
    pub session_id: String,
    /// The agent loop's id for this tool call, when the call came from a turn; for a call a
    /// coding agent's child made over the tool bridge, the child's own id for it, which it sends
    /// again when it retries. `None` from `POST /agent/call_tool`, from a Code Execution
    /// script's sub-call, and from a bridged call the child named no id for.
    pub tool_call_id: Option<String>,
    /// The capability this call was ADMITTED on (issue #56): sampled once at dispatch and
    /// threaded here through `McpMeta`. The host's privacy gate asks this value, never a
    /// fresh sample of the provider.
    pub capability: CallCapability,
    /// The turn's cancellation token. Stop ends a wait and leaves a started job running.
    pub cancel: CancellationToken,
    /// Where approvals for this call are parked: [`PendingUserActions::global`] in
    /// production, a standalone registry in a test.
    pub approvals: Arc<PendingUserActions>,
}

impl std::fmt::Debug for PersonToolCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonToolCall")
            .field("tool", &self.tool)
            .field("action", &self.action)
            .field("params", &self.params)
            .field("session_id", &self.session_id)
            .field("tool_call_id", &self.tool_call_id)
            .field("capability", &self.capability)
            .field("cancelled", &self.cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// Said when a chat gained a Crew grant between admitting a person-tool call and acting on it.
pub const GRANTED_MEANWHILE: &str = "This chat was given Crew access while this was waiting, so it can't act as you any more. Nothing was changed.";

impl PersonToolCall {
    /// AO-12: "The tool checks that the chat still has no grant at dispatch and again after the
    /// approval." The extension made the first check; the host calls this after every approval
    /// it parks and before it acts. A chat whose standing cannot be read counts as granted:
    /// [`crate::crew::CrewManager::is_scoped_session`] fails closed.
    pub async fn ensure_still_ungranted(&self) -> anyhow::Result<()> {
        if crate::crew::manager()?
            .is_scoped_session(&self.session_id)
            .await
        {
            anyhow::bail!(GRANTED_MEANWHILE);
        }
        Ok(())
    }
}

/// What the daemon implements to run a person-tool call.
#[async_trait::async_trait]
pub trait CrewToolHost: Send + Sync {
    /// Run `call` and answer with what the model is shown, as JSON. A refusal is an `Err`
    /// whose text is a sentence for the person (the extension turns a broker's envelope into
    /// one with [`crate::crew::agent_error_text`]); it never carries names from a workspace
    /// the call may not read.
    async fn call(&self, call: PersonToolCall) -> anyhow::Result<serde_json::Value>;
}

static HOST: OnceLock<Arc<dyn CrewToolHost>> = OnceLock::new();

// Test-only override of what [`get`] returns, and why tests must never call [`install`]: every
// unit test in this crate runs in ONE process, and `install` writes a `OnceLock`, so one test
// installing a host would offer the person tools to every other test's chats for the rest of
// the run, with no way to undo it.
//
// ⚠ A task-local, not `workspace_services`' process-global `RwLock`. Whether a host is
// installed changes the tool roster every `Agent::list_tools` reads, and a test that lists
// tools is not in any `#[serial]` group a host test could join, so a global override would
// leak `crew__read` into unrelated tests' rosters while it was set. Scoped to the test's own
// future, the override reaches the roster, the dispatch and the tool that test drives, and
// nothing else. Not being in a scope means "no override": [`get`] reads the real slot, which
// no unit test writes.
#[cfg(test)]
tokio::task_local! {
    static TEST_HOST: Option<Arc<dyn CrewToolHost>>;
}

/// Install the daemon's host. First install wins; later calls are no-ops. **Production only**:
/// a unit test uses [`with_host_for_tests`]. An integration test binary, which is a process of
/// its own, may install one.
pub fn install(host: Arc<dyn CrewToolHost>) {
    let _ = HOST.set(host);
}

/// The installed host, or `None` when this process has none (the person tools are then not
/// offered).
pub fn get() -> Option<Arc<dyn CrewToolHost>> {
    #[cfg(test)]
    {
        if let Ok(over) = TEST_HOST.try_with(Clone::clone) {
            return over;
        }
    }
    HOST.get().cloned()
}

/// Run `f` as though `host` were installed (`None`: as though none were), for this future only.
#[cfg(test)]
pub(crate) async fn with_host_for_tests<F: Future>(
    host: Option<Arc<dyn CrewToolHost>>,
    f: F,
) -> F::Output {
    TEST_HOST.scope(host, f).await
}

tokio::task_local! {
    /// The id of the tool call running on this task. A task-local because the extension's
    /// `call_tool` receives no request id (`McpMeta` carries none). Two callers have one: the
    /// agent loop for its own calls, and the coding-agent bridge for a child's call
    /// (`crew_extension::with_bridged_call_id`).
    static TOOL_CALL_ID: String;
}

/// Run `f` with `tool_call_id` as the current tool call's id.
pub(crate) async fn with_tool_call_id<F: Future>(tool_call_id: String, f: F) -> F::Output {
    TOOL_CALL_ID.scope(tool_call_id, f).await
}

/// The id of the tool call running on this task, when the agent loop set one.
pub(crate) fn current_tool_call_id() -> Option<String> {
    TOOL_CALL_ID.try_with(Clone::clone).ok()
}

/// The agent loop's tool-call future, with `request_id` as its tool-call id when `name` is a
/// person tool, so the host knows which call it serves. Any other tool's future is returned
/// unchanged.
pub(crate) fn scope_tool_call_id<T: Send + 'static>(
    name: &str,
    request_id: &str,
    future: Box<dyn Future<Output = T> + Send + Unpin>,
) -> Box<dyn Future<Output = T> + Send + Unpin> {
    if PersonTool::from_name(name).is_none() {
        return future;
    }
    Box::new(Box::pin(with_tool_call_id(request_id.to_owned(), future)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Answering;

    #[async_trait::async_trait]
    impl CrewToolHost for Answering {
        async fn call(&self, call: PersonToolCall) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({ "action": call.action }))
        }
    }

    /// AO-12's table, as the owner's D1 ruling amends it: `describe` reads, signing in and the
    /// vault are changes the person finishes in their own window, and the person-only controls
    /// are in neither list.
    #[test]
    fn the_actions_are_the_contract_s_table() {
        assert_eq!(READ_ACTIONS.len(), 28);
        assert_eq!(CHANGE_ACTIONS.len(), 57);
        assert_eq!(READ_ACTIONS[0], "describe");
        for actions in [READ_ACTIONS, CHANGE_ACTIONS] {
            let unique: std::collections::BTreeSet<_> = actions.iter().collect();
            assert_eq!(unique.len(), actions.len(), "a duplicated action");
        }
        assert!(READ_ACTIONS
            .iter()
            .all(|read| !CHANGE_ACTIONS.contains(read)));
        for person_only in [
            "daemon_start",
            "daemon_stop",
            "workspace_bootstrap",
            "enroll_prepare",
            "enroll_accept",
            "connections_prepare",
            "connections_save",
            "grants_create",
            "grant",
        ] {
            assert!(
                !READ_ACTIONS.contains(&person_only) && !CHANGE_ACTIONS.contains(&person_only),
                "{person_only} is a person-only control"
            );
        }
        // A chat never grants Crew access: of the grant actions, it may list and revoke only.
        let grants: Vec<_> = READ_ACTIONS
            .iter()
            .chain(CHANGE_ACTIONS)
            .filter(|action| action.starts_with("grant"))
            .collect();
        assert_eq!(grants, [&"grants_list", &"grants_revoke"]);
        // D1: the person's own steps a chat may now open, and the parity gaps it now has.
        for read in [
            "credentials_status",
            "connections_invitation",
            "setup_list",
            "files_pending",
        ] {
            assert!(READ_ACTIONS.contains(&read), "{read}");
        }
        for change in [
            "auth",
            "credentials_unlock",
            "credentials_init",
            "credentials_lock",
            "context_draft",
            "profile_devices_acknowledge",
            "files_pause",
            "files_resume",
            "files_forget",
        ] {
            assert!(CHANGE_ACTIONS.contains(&change), "{change}");
        }
    }

    #[test]
    fn each_tool_knows_its_names_and_actions() {
        for tool in PersonTool::ALL {
            assert_eq!(PersonTool::from_name(tool.name()), Some(tool));
            assert_eq!(PersonTool::from_tool_name(tool.tool_name()), Some(tool));
            assert_eq!(tool.name(), format!("crew__{}", tool.tool_name()));
        }
        assert_eq!(PersonTool::Read.actions(), READ_ACTIONS);
        assert_eq!(PersonTool::Change.actions(), CHANGE_ACTIONS);
        for other in [
            "crew__request",
            "crew__connections",
            "read",
            "crew__",
            "todo__read",
        ] {
            assert_eq!(PersonTool::from_name(other), None, "{other}");
        }
        assert_eq!(PersonTool::from_tool_name("request"), None);
    }

    /// NOTE what this test does NOT do: call `install`. See [`TEST_HOST`].
    #[tokio::test]
    async fn the_override_is_what_get_returns_inside_its_scope_only() {
        let answering: Arc<dyn CrewToolHost> = Arc::new(Answering);
        with_host_for_tests(Some(answering), async {
            let host = get().expect("the override");
            let call = PersonToolCall {
                tool: PersonTool::Read,
                action: "status".into(),
                params: serde_json::json!({}),
                session_id: "s".into(),
                tool_call_id: None,
                capability: CallCapability::for_test_restricted(),
                cancel: CancellationToken::new(),
                approvals: Arc::new(PendingUserActions::default()),
            };
            assert_eq!(
                host.call(call).await.unwrap(),
                serde_json::json!({"action": "status"})
            );
            // A nested `None` is "there is no host", whatever an outer scope says.
            with_host_for_tests(None, async { assert!(get().is_none()) }).await;
        })
        .await;
        // Outside every scope, the real slot, which no unit test writes.
        assert!(get().is_none());
    }

    #[tokio::test]
    async fn only_a_person_tool_s_future_carries_the_tool_call_id() {
        assert_eq!(current_tool_call_id(), None);
        let probe = || -> Box<dyn Future<Output = Option<String>> + Send + Unpin> {
            Box::new(Box::pin(async { current_tool_call_id() }))
        };
        for tool in PersonTool::ALL {
            let scoped = scope_tool_call_id(tool.name(), "call_7", probe());
            assert_eq!(scoped.await.as_deref(), Some("call_7"), "{tool:?}");
        }
        for other in ["crew__request", "developer__shell"] {
            assert_eq!(scope_tool_call_id(other, "call_7", probe()).await, None);
        }
    }
}
