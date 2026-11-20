//! BR-71: the crate-boundary bridge. Platform extensions live in this crate,
//! but the turn lock, detached runner, and WorkspaceBridge live in
//! `biorouter-server`. The daemon implements this trait over its `AppState`
//! and installs it process-wide at bootstrap; the workspace extension reads it
//! lazily per call. When nothing is installed (CLI-direct, tests), workspace
//! tools degrade to session-level effects reachable through `SessionManager` /
//! `AgentManager` and say so — the design's headless requirement (§2.1).

use std::sync::{Arc, OnceLock};

use crate::conversation::message::Message;

/// An opaque hold on the server's per-session turn lock (BR-71 reconciliation
/// #2). Dropping it releases the session for the next turn — the same RAII
/// discipline as the server's own `TurnGuard`, which the daemon implementation
/// wraps. Held by `run_complete_subagent_task` for a glass-box child's whole
/// run so `is_turn_active`, `/agent/cancel`, and the one-turn-per-session
/// invariant all see the child's turn.
pub trait WorkspaceTurnLease: Send {
    /// The server-assigned turn id this lease owns.
    fn turn_id(&self) -> &str;
}

/// A session's knowledge-base set and its write target, as BR-71 needs them.
///
/// ⚠ **A deliberately narrower view of `KbSelection`** (`biorouter-mcp`'s
/// `knowledge::service::KbSelection`), not a re-export of it, and the reason is
/// recorded here so the next reader does not "simplify" it back:
///
/// * `KbSelection` also carries `hidden_kbs`, the subtractive representation
///   that *produced* `kb_ids`. BR-71 never needs it, and a `workspace_list` row
///   that serialized it would publish an implementation detail into a model-
///   facing payload where a model would then try to reason about it.
/// * `PrimaryUpdate<'a>` is borrowed. A trait whose implementors are behind an
///   `Arc<dyn …>` and whose callers are MCP tools holding owned `String`s is the
///   wrong place for a lifetime parameter.
/// * `biorouter` CAN see those types (`biorouter::knowledge::*` re-exports
///   `biorouter_mcp::knowledge`), so this is a choice, not a constraint. The
///   conversion happens exactly once, in `ServerWorkspaceServices`, which is
///   also the only place the subtractive model is visible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KbSelectionView {
    /// The session's knowledge bases, sorted. Empty headless.
    pub kb_ids: Vec<String>,
    /// The write target for KB-less mutating calls. Always a member of `kb_ids`,
    /// or `None`.
    pub primary_kb: Option<String>,
}

/// What a `set_knowledge_bases` call wants to happen to the write target.
///
/// The service enforces that a primary is a member of the RESULTING set, so
/// "assign this list" is not a complete instruction — every mutator must answer
/// this question. Mirrors `PrimaryUpdate` minus the borrow, plus `Auto`, which
/// is BR-71's stated default so no tool has to invent one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KbPrimaryChoice {
    /// Keep the current primary if it is still a member of the new set;
    /// otherwise pin the new set's first id; on an empty new set, clear it.
    /// The default for `workspace_set_tools` and `workspace_open`.
    Auto,
    /// Pin this id. Refused by the service if it is not a member of the result.
    Set(String),
    /// Explicitly "no primary for this session". KB-less writes then fail here
    /// even when the machine default names one — this is NOT `Inherit`.
    Clear,
}

#[async_trait::async_trait]
pub trait WorkspaceServices: Send + Sync {
    /// True when at least one GUI window has a live workspace channel.
    fn gui_attached(&self) -> bool;
    /// The most recent merged layout echo from all attached windows (§4.3).
    fn layout_snapshot(&self) -> Option<serde_json::Value>;
    /// True while an interactive turn is in flight for the session (BR-33 lock).
    fn is_turn_active(&self, session_id: &str) -> bool;
    /// Trip the running turn's cancellation token; `None` when idle (BR-62).
    fn cancel_turn(&self, session_id: &str) -> Option<String>;
    /// Abandon daemon and in-process continuation admissions for a session
    /// that is being explicitly cancelled or stopped.
    fn abandon_pending_continuations(&self, session_id: &str) -> bool {
        crate::agents::subagent_handle::abandon_continuation(session_id)
    }
    /// Acquire the per-session turn lock for a turn this crate is about to run
    /// itself (a subagent run — reconciliation #2). `cancel` is registered as
    /// the token `cancel_turn` trips. Errors with the running turn's id when
    /// the session is already busy.
    fn begin_turn(
        &self,
        session_id: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Box<dyn WorkspaceTurnLease>, String>;
    /// Cancel any turn, then evict the agent from the registry (session survives).
    async fn stop_agent(&self, session_id: &str) -> Result<(), String>;
    /// Start a detached turn; `Ok(turn_id)` or an error naming the conflict.
    async fn start_detached_turn(
        &self,
        session_id: &str,
        message: Message,
    ) -> Result<String, String>;
    /// Create a session the way `POST /agent/start` does (extension names are
    /// resolved against the config registry). Returns the new session id.
    ///
    /// `primary` is not optional for the same reason it is not optional on
    /// `set_knowledge_bases`: a non-empty `knowledge_bases` with no write target
    /// is a session whose KB-less writes fail, which is not a state any caller
    /// means to create. `KbPrimaryChoice::Auto` is the answer for every caller
    /// that has not been told otherwise.
    async fn start_session(
        &self,
        working_dir: std::path::PathBuf,
        extensions: Option<Vec<String>>,
        knowledge_bases: Vec<String>,
        primary: KbPrimaryChoice,
    ) -> Result<String, String>;
    /// Replace the session's **visible** knowledge-base set, and say what should
    /// happen to its write target. An empty slice clears the set (and, with
    /// `KbPrimaryChoice::Auto`, clears the primary — an empty set has no legal
    /// member to point at).
    ///
    /// Returns the resulting selection, because the service may have moved the
    /// primary as part of the same operation and a caller that has to re-read to
    /// find out will race.
    fn set_knowledge_bases(
        &self,
        session_id: &str,
        kbs: &[String],
        primary: KbPrimaryChoice,
    ) -> Result<KbSelectionView, String>;
    /// The session's knowledge bases AND its write target (`workspace_list` §4.1,
    /// spawn-context grants §4.4). Both, in one call, because they are one
    /// claim: `primary_kb` is always a member of `kb_ids` or `None`, and
    /// composing them from two unlocked reads is how that claim gets falsified
    /// (the reason `KnowledgeService::selection` takes the root lock). Empty /
    /// `None` headless or when none are set.
    fn knowledge_selection(&self, session_id: &str) -> KbSelectionView;
    /// Every installed knowledge base's id, so `workspace_set_tools` can refuse
    /// a name that matches none of them BEFORE it raises an approval or changes
    /// anything (QA finding F4). [`Self::set_knowledge_bases`] silently drops an
    /// id it has no base for, so without this a typo is a partly-applied set.
    ///
    /// `None` when this host cannot enumerate them — then nothing is refused on
    /// those grounds, and the setter's own validation is the only check.
    fn installed_knowledge_bases(&self) -> Option<Vec<String>> {
        None
    }
    /// Push a workspace frame to the GUI (§4.3). `wait_result` parks for the
    /// renderer's `workspace_result`. Errors when no GUI is attached.
    async fn gui_command(
        &self,
        frame: serde_json::Value,
        wait_result: bool,
    ) -> Result<serde_json::Value, String>;

    /// Push a workspace frame to the window that holds `near_session`, falling
    /// back to [`Self::gui_command`]'s guess when that window cannot be found.
    ///
    /// ⚠ **Additive on purpose** (issue #78). The routing bug is that the
    /// spawn path never said which window it meant, so the daemon guessed with
    /// `focused_or_recent` — and before the renderer reported real focus, that
    /// guess could not even discriminate between two open windows and fell
    /// through to `HashMap` iteration order.
    ///
    /// A sibling with a default body, rather than a parameter added to
    /// `gui_command`, because the signature change would ripple through the
    /// trait, `NullServices`, the server impl and two test doubles for no gain:
    /// every existing caller genuinely has no target to name.
    ///
    /// The default IGNORES the hint. That is the honest behaviour for a
    /// headless host, and it keeps the fire-and-forget contract: a spawn must
    /// never fail because a window could not be located.
    async fn gui_command_near(
        &self,
        frame: serde_json::Value,
        wait_result: bool,
        near_session: &str,
    ) -> Result<serde_json::Value, String> {
        let _ = near_session;
        self.gui_command(frame, wait_result).await
    }

    /// One Biorouter Browser operation (BB-10), on the window that shows `session_id` and on no
    /// other.
    ///
    /// It differs from [`Self::gui_command_near`] in four ways, each a decision of the design:
    ///
    /// * **Strict routing.** Only the window whose echo holds the chat. There is no fallback to
    ///   the focused or most recent window, because acting in the wrong window is worse than
    ///   failing.
    /// * **A deadline per operation**, which the caller chooses, instead of the fixed round trip
    ///   of `gui_command`. The daemon waits 2 s beyond it.
    /// * **Cancellation.** The wait races `cancel` (the turn's token joined with the grant's).
    ///   On a cancel or a timeout the daemon tells the window to abandon the operation, and a
    ///   timed out mutation is never replayed.
    /// * **Typed failures**, each with a sentence written for the model.
    ///
    /// The default answers [`BrowserChannelError::NoDesktop`]: a host with no desktop window
    /// (the CLI in its own process, `biorouter serve`) has no browser to reach.
    async fn gui_browser_request(
        &self,
        frame: serde_json::Value,
        session_id: &str,
        deadline: std::time::Duration,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<serde_json::Value, BrowserChannelError> {
        let _ = (frame, session_id, deadline, cancel);
        Err(BrowserChannelError::NoDesktop)
    }

    /// Why a Biorouter Browser operation for `session_id` could not reach a window right
    /// now, by the same routing rule [`Self::gui_browser_request`] applies; `None` when a
    /// window holds the chat, or when this host cannot tell.
    ///
    /// Asked BEFORE the consent wait (`ExtensionManager::admit_browser`). The card that
    /// answers the wait is shown only by a window that holds the chat, so a chat no window
    /// holds (driven through `/reply` by `biorouter session send` or `workspace_send_prompt`)
    /// parked for the whole 900 s approval wait before the operation was refused anyway.
    ///
    /// The default answers `None`, which keeps the wait: a host that cannot tell must not
    /// refuse a call the window might have answered.
    fn browser_route_refusal(&self, session_id: &str) -> Option<BrowserChannelError> {
        let _ = session_id;
        None
    }
}

/// Why a Biorouter Browser operation never reached a page, or never came back (BB-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserChannelError {
    /// No desktop window is attached to this daemon.
    NoDesktop,
    /// A desktop window is attached, but none of them shows this chat.
    ChatNotOnScreen,
    /// The window that showed the chat closed or reloaded before it answered.
    WindowGone,
    /// No answer within the operation's deadline.
    Timeout,
    /// The turn was cancelled, or the person stopped the browser.
    Cancelled,
}

impl std::fmt::Display for BrowserChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoDesktop => {
                "The Biorouter Browser runs in the desktop app, and no desktop window is attached. \
                 For the plain text of a public page use webdocuments web_scrape; otherwise tell \
                 the person the browser is not available here."
            }
            Self::ChatNotOnScreen => {
                "Biorouter's browser runs in this chat's side panel, and the chat is not on \
                 screen. Ask the person to open the chat."
            }
            Self::WindowGone => {
                "The window showing this chat closed or reloaded before the browser answered. \
                 The operation may or may not have happened: take a browser_snapshot before \
                 doing anything else."
            }
            Self::Timeout => {
                "The browser did not answer in time. The operation may or may not have happened: \
                 take a browser_snapshot to see the page before acting again, and never repeat a \
                 click, fill or submit blindly."
            }
            Self::Cancelled => "The browser operation was canceled.",
        })
    }
}

static WORKSPACE_SERVICES: OnceLock<Arc<dyn WorkspaceServices>> = OnceLock::new();

/// Test-only override of what [`get`] returns.
///
/// **Why this exists, and why tests must never call [`install`].** Every unit
/// test in this crate runs in ONE process (`cargo test -p biorouter --lib`
/// builds a single binary and libtest runs its tests as threads). `install`
/// writes a `OnceLock`, so a single test calling it would pin a daemon
/// stand-in for every *other* test in the binary, for the rest of the run —
/// and there is no way to undo it. Two Task-17 tests need "no daemon" and two
/// Task-16 tests need "a daemon"; a `OnceLock` written from a test makes those
/// mutually exclusive and the loser fails on thread-scheduling luck.
///
/// Three states, deliberately: outer `None` = no override (production);
/// `Some(None)` = "there is no daemon"; `Some(Some(svc))` = "this daemon".
/// A two-state `Option` cannot express the middle one once the real slot is set.
#[cfg(test)]
static TEST_SERVICES: std::sync::RwLock<Option<Option<Arc<dyn WorkspaceServices>>>> =
    std::sync::RwLock::new(None);

/// Install the daemon's implementation. First install wins; later calls are
/// no-ops (matters only to in-process test harnesses). **Production only** —
/// tests use [`set_for_tests`].
pub fn install(services: Arc<dyn WorkspaceServices>) {
    let _ = WORKSPACE_SERVICES.set(services);
}

/// Force what [`get`] returns for the duration of one test. ALWAYS pair with
/// `#[serial_test::serial(workspace_services)]` — the slot is process-global —
/// and clear it before returning.
#[cfg(test)]
pub(crate) fn set_for_tests(services: Option<Arc<dyn WorkspaceServices>>) {
    *TEST_SERVICES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(services);
}

#[cfg(test)]
pub(crate) fn clear_test_override() {
    *TEST_SERVICES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// The installed services, or `None` when running without the daemon.
pub fn get() -> Option<Arc<dyn WorkspaceServices>> {
    #[cfg(test)]
    {
        if let Some(over) = TEST_SERVICES
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return over;
        }
    }
    WORKSPACE_SERVICES.get().cloned()
}

/// A `WorkspaceServices` that answers "headless, nothing running, no GUI" —
/// the shared daemon stand-in for tests in this crate. Lives here (not in a
/// test module) so `agents::workspace_extension`'s tests can use it.
#[cfg(test)]
pub(crate) struct NullServices;

#[cfg(test)]
pub(crate) struct NullLease;

#[cfg(test)]
impl WorkspaceTurnLease for NullLease {
    fn turn_id(&self) -> &str {
        "turn-fake"
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl WorkspaceServices for NullServices {
    fn gui_attached(&self) -> bool {
        false
    }
    fn layout_snapshot(&self) -> Option<serde_json::Value> {
        None
    }
    fn is_turn_active(&self, _session_id: &str) -> bool {
        false
    }
    fn cancel_turn(&self, _session_id: &str) -> Option<String> {
        None
    }
    fn begin_turn(
        &self,
        _session_id: &str,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Box<dyn WorkspaceTurnLease>, String> {
        Ok(Box::new(NullLease))
    }
    async fn stop_agent(&self, _session_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn start_detached_turn(
        &self,
        _session_id: &str,
        _message: crate::conversation::message::Message,
    ) -> Result<String, String> {
        Ok("turn-1".into())
    }
    async fn start_session(
        &self,
        _working_dir: std::path::PathBuf,
        _extensions: Option<Vec<String>>,
        _knowledge_bases: Vec<String>,
        _primary: KbPrimaryChoice,
    ) -> Result<String, String> {
        Ok("s-new".into())
    }
    fn set_knowledge_bases(
        &self,
        _session_id: &str,
        _kbs: &[String],
        _primary: KbPrimaryChoice,
    ) -> Result<KbSelectionView, String> {
        Ok(KbSelectionView::default())
    }
    fn knowledge_selection(&self, _session_id: &str) -> KbSelectionView {
        KbSelectionView::default()
    }
    async fn gui_command(
        &self,
        _frame: serde_json::Value,
        _wait_result: bool,
    ) -> Result<serde_json::Value, String> {
        Err("no GUI attached".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NOTE what this test does NOT do: call `install`. `WORKSPACE_SERVICES` is a
    /// `OnceLock` and every `--lib` test shares one process, so a test that
    /// installed would pin a daemon stand-in for the rest of the run and make
    /// Task 16's "with a daemon" tests and Task 17's "without a daemon" tests
    /// mutually exclusive, with the loser decided by thread scheduling.
    #[test]
    #[serial_test::serial(workspace_services)]
    fn the_test_override_is_what_get_returns() {
        set_for_tests(Some(std::sync::Arc::new(NullServices)));
        let got = get().expect("get() returns the overridden services");
        // Prove we got a real implementation back: its methods answer.
        assert!(!got.gui_attached());
        assert!(got.layout_snapshot().is_none());
        let lease = got
            .begin_turn("s-any", tokio_util::sync::CancellationToken::new())
            .expect("fake lease");
        assert_eq!(lease.turn_id(), "turn-fake");

        // The post-#45 KB pair: a SELECTION, not a bare list, and a mutator that
        // must be told what to do with the write target. A stand-in that answers
        // both is what Tasks 16 and 17 reuse.
        assert_eq!(got.knowledge_selection("s-any"), KbSelectionView::default());
        assert!(got
            .set_knowledge_bases("s-any", &["kb-a".to_string()], KbPrimaryChoice::Auto)
            .is_ok());

        // `Some(None)` is "there is no daemon" — the state a two-valued
        // override could not express once the real slot had been written.
        set_for_tests(None);
        assert!(get().is_none());

        clear_test_override();
    }
}
