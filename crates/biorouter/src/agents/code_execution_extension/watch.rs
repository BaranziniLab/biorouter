//! `code_execution__watch_job`: a job watch the agent assigns, which fires on
//! its own and spends no model tokens while the job runs.
//!
//! Every model turn re-reads the whole conversation. `wait_for` already keeps
//! the checks out of the model, but it holds the turn open and returns after
//! at most ten minutes, so a job that runs for an hour still costs a model
//! turn every few minutes. A watch ends the turn instead:
//!
//! 1. `watch_job` checks the job once, in the turn, through the same admitted
//!    path as `wait_for`'s first check (boundary refusals, the script-call
//!    judge, `ExtensionManager::dispatch_tool_call`, output redaction). A job
//!    that is already done is answered at once and no watch is set.
//! 2. Otherwise it registers a watch (process memory, and a row in the active
//!    work registry the person can cancel) and returns its id. The model ends
//!    its turn.
//! 3. A background task repeats the SAME admitted call on a backoff: the first
//!    interval is the longer of 30 s and the job's own hint, and each later one
//!    is half again as long, up to five minutes. Each check samples the chat's
//!    capability afresh (the chat's model may have changed since the turn) and
//!    re-runs the boundary refusals, never the judge: a person is asked at most
//!    once per watch, as with `wait_for`.
//! 4. When the job is no longer active, a check keeps failing, or the watch
//!    reaches its time limit, the chat is resumed ONCE: a running turn gets a
//!    steer, an idle chat gets a new turn on its own bound model, and a chat
//!    that is paused by a usage limit (or whose approvals no window could
//!    show) gets a note instead of a turn.
//!
//! A deleted chat ends its watches silently: the chat's incarnation is
//! compared before every check and before the wake. Stop during the turn that
//! set the watch cancels it; so does the active-work cancel and
//! `watch_job {"cancel": id}`. Watches are not persisted: a daemon restart
//! drops them, and the tool result says so. See
//! `docs/agent-loop/waiting-on-jobs.md`.

use super::wait::{self, CheckReading, Until, WaitEnd, WaitForParams, WaitPlan, WaitUntilParam};
use super::{CodeExecutionClient, CollectedArtifacts, ManagedCheck};
use crate::agents::extension_manager::ExtensionManager;
use crate::privacy::CallCapability;
use crate::tool_monitor::job_status::JobActivity;
use crate::tool_monitor::poll_pacing::PacingMode;
use async_trait::async_trait;
use rmcp::model::{CallToolResult, Content, JsonObject};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// How long a watch lasts when no `max_wait_minutes` is given.
pub(super) const DEFAULT_MAX_WAIT_MINUTES: u64 = 60;
/// The longest a watch may last.
pub(super) const MAX_WAIT_MINUTES: u64 = 240;
/// The first background check comes no sooner than this.
pub(super) const FIRST_INTERVAL_FLOOR_SECS: u64 = 30;
/// The backoff grows toward this interval and stops there.
pub(super) const BACKOFF_CEILING_SECS: u64 = 300;
/// A job's own hint is honoured up to this interval.
pub(super) const HINT_CEILING_SECS: u64 = 900;
/// The longest note the model may leave for when it is resumed.
pub(super) const MAX_NOTE_CHARS: usize = 500;
/// The most of the job's last status the wake message carries.
pub(super) const MAX_STATE_CHARS: usize = 1500;
/// How much of the current status the tool result echoes.
const MAX_FIRST_STATE_CHARS: usize = 300;
/// Live watches one chat may hold.
pub(super) const MAX_WATCHES_PER_CHAT: usize = 8;
/// Live watches the whole process may hold.
pub(super) const MAX_WATCHES: usize = 64;
/// Consecutive failed checks before a watch gives up.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// How many times the wake retries a chat whose turn is closing.
const WAKE_ATTEMPTS: u32 = 5;
/// The name the wake message is attributed to.
const WATCH_SENDER: &str = "Biorouter job watch";

/// Arguments of `code_execution__watch_job`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub(super) struct WatchJobParams {
    /// Full name of the status tool to call, server__tool (for example
    /// "cdwagent__CDW-query_job_status").
    #[serde(default)]
    pub tool: Option<String>,
    /// The status tool's arguments (for example {"job_id": "abc"}).
    #[serde(default)]
    pub arguments: Option<JsonObject>,
    /// When the job counts as done: omit for "inactive", or "changed", or
    /// {"pointer": "/status", "values": ["completed", "failed"]}.
    #[serde(default)]
    pub until: Option<WaitUntilParam>,
    /// Seconds before the first background check (at least 30). Later checks
    /// slow down toward every 5 minutes. A longer recommended_poll_seconds,
    /// retry_after or poll_after in the status result is honoured.
    #[serde(default, deserialize_with = "wait::de_seconds")]
    #[schemars(with = "Option<f64>")]
    pub min_interval_seconds: Option<f64>,
    /// How long to watch, in minutes (default 60, at most 240). When it
    /// passes, the chat is resumed with the job still running.
    #[serde(default, deserialize_with = "wait::de_seconds")]
    #[schemars(with = "Option<f64>")]
    pub max_wait_minutes: Option<f64>,
    /// What to do when resumed, at most 500 characters. It comes back with
    /// the job's last status.
    #[serde(default)]
    pub note: Option<String>,
    /// A watch id (for example "watch-3") to cancel instead of starting one,
    /// or "all" for every watch of this chat. The other arguments are ignored.
    #[serde(default)]
    pub cancel: Option<String>,
}

/// Why a watch was cancelled rather than ending on the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopCause {
    /// The person cancelled it from the active work list.
    Person,
    /// The model cancelled it with `watch_job {"cancel": ...}`.
    Agent,
    /// Stop was pressed during the turn that set it.
    TurnStopped,
    /// The chat was deleted ([`cancel_watches_for_session`]).
    ChatDeleted,
    /// Another read of the same status call in the chat (the model's own
    /// check, a `wait_for`, a script's call) saw the job end
    /// ([`retire_watches_seen_ending`]). The model has that result already, so
    /// the chat is neither resumed nor told.
    SeenEnded,
}

/// One live watch, as the registry holds it.
struct WatchEntry {
    session_id: String,
    tool: String,
    /// The status call's arguments, canonical, to spot a duplicate watch.
    arguments: String,
    /// The arguments the first check was admitted with, canonical. The same as
    /// `arguments` unless the judge rewrote the call; a read of either one is
    /// a read of this job.
    admitted: String,
    /// When the job counts as done, to judge a status read made elsewhere.
    until: Until,
    stop: CancellationToken,
    cause: Arc<std::sync::Mutex<Option<StopCause>>>,
}

impl WatchEntry {
    fn cancel(&self, cause: StopCause) {
        let mut slot = self
            .cause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.get_or_insert(cause);
        drop(slot);
        self.stop.cancel();
    }
}

/// Every live watch in this process, by id. The id is the watch's active work
/// id, so the person and the model name it the same way.
static WATCHES: LazyLock<std::sync::Mutex<HashMap<String, WatchEntry>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn watches() -> std::sync::MutexGuard<'static, HashMap<String, WatchEntry>> {
    WATCHES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Cancel every watch of a chat, as for a deleted chat. Returns how many were
/// cancelled. Each ends without resuming the chat. (A watch whose chat was
/// deleted also notices on its own, before its next check.) Called by the
/// delete route through the `agents` re-export.
pub fn cancel_watches_for_session(session_id: &str) -> usize {
    let watches = watches();
    let mut cancelled = 0;
    for entry in watches.values().filter(|e| e.session_id == session_id) {
        entry.cancel(StopCause::ChatDeleted);
        cancelled += 1;
    }
    cancelled
}

/// Retire every watch of this chat on `tool` with these arguments when
/// `result`, read outside the watch, shows the job ended: its state reads
/// finished, or the watch's `until` pointer matches. Called for every status
/// result the dispatch choke point returns
/// (`tool_monitor::poll_pacing::after_dispatch`), except a watch's own checks.
/// Returns how many watches were retired. Cheap when the chat has no watch.
///
/// Without it a model that ignored "end your turn", read the job's end itself
/// and answered was woken again when the watch noticed the same end.
pub fn retire_watches_seen_ending(
    session_id: &str,
    tool: &str,
    arguments: Option<&JsonObject>,
    result: &CallToolResult,
) -> usize {
    if result.is_error == Some(true) {
        return 0;
    }
    // The registry lock is not held while the result is read: it is the one
    // every dispatch in the process takes on its way out.
    if !watches()
        .values()
        .any(|e| e.session_id == session_id && e.tool == tool)
    {
        return 0;
    }
    let key = canonical(&arguments.cloned().unwrap_or_default());
    let finished =
        crate::tool_monitor::job_status::activity_of_result(tool, result) == JobActivity::Finished;
    let watches = watches();
    let mut retired = 0;
    for (id, entry) in watches.iter().filter(|(_, e)| {
        e.session_id == session_id && e.tool == tool && (e.arguments == key || e.admitted == key)
    }) {
        let ended = finished
            || match &entry.until {
                Until::Pointer { pointer, values } => {
                    pointer_matches_result(result, pointer, values)
                }
                Until::Inactive | Until::Changed => false,
            };
        if ended {
            tracing::info!(
                session_id,
                watch = %id,
                tool,
                "a status read outside the watch saw the job end; retiring the watch"
            );
            entry.cancel(StopCause::SeenEnded);
            retired += 1;
        }
    }
    retired
}

/// Whether any JSON value in `result` holds one of `values` at `pointer`.
fn pointer_matches_result(result: &CallToolResult, pointer: &str, values: &[Value]) -> bool {
    let structured = result.structured_content.iter().cloned();
    let texts = result
        .content
        .iter()
        .filter_map(|content| content.as_text())
        .filter_map(|text| serde_json::from_str::<Value>(text.text.trim()).ok());
    structured.chain(texts).any(|json| {
        json.pointer(pointer)
            .is_some_and(|found| wait::pointer_matches(found, values))
    })
}

/// The id of this chat's live watch of `tool` with these arguments, if any.
pub(super) fn active_watch_for(
    session_id: &str,
    tool: &str,
    arguments: &JsonObject,
) -> Option<String> {
    let key = canonical(arguments);
    watches()
        .iter()
        .find(|(_, e)| {
            e.session_id == session_id
                && e.tool == tool
                && (e.arguments == key || e.admitted == key)
        })
        .map(|(id, _)| id.clone())
}

fn watch_number(id: &str) -> u64 {
    id.rsplit('-')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(u64::MAX)
}

/// What a watch needs from the rest of Biorouter, behind one seam so the
/// tests can stand in for the daemon. Production is [`LiveHost`].
#[async_trait]
pub(super) trait WatchHost: Send + Sync {
    /// Whether a daemon is running that can start a turn for a chat.
    fn has_daemon(&self) -> bool;
    /// The capability a check runs on, sampled NOW from the chat's bound
    /// model, and the chat's live extension manager. `None` when the chat has
    /// no live agent (it was stopped).
    async fn check_context(
        &self,
        session_id: &str,
    ) -> Option<(CallCapability, Arc<ExtensionManager>)>;
    /// The chat's incarnation, the identity of the row holding its id.
    async fn incarnation(&self, session_id: &str) -> Option<i64>;
    /// Whether the chat still exists as the incarnation the watch was set in.
    async fn chat_is_current(&self, session_id: &str, incarnation: Option<i64>) -> bool;
    /// The stop notice of a usage limit the chat has reached, if any.
    async fn limit_notice(&self, session_id: &str) -> Option<String>;
    /// Whether a turn is running in the chat.
    fn turn_active(&self, session_id: &str) -> bool;
    /// Steer `text` into the chat's running turn.
    async fn try_steer(&self, session_id: &str, text: &str) -> Result<(), String>;
    /// Whether a turn started now could raise an approval no window can show.
    async fn approvals_would_go_unseen(&self, session_id: &str) -> bool;
    /// Start a turn in the chat on `text`. `Ok` is the turn id.
    async fn start_turn(&self, session_id: &str, text: &str) -> Result<String, String>;
    /// Store `text` in the chat as a note, with no turn.
    async fn append_note(&self, session_id: &str, text: &str) -> Result<(), String>;
    /// Store `text` as a notice the person reads and the model never does
    /// (a user-only inline message), with no turn.
    async fn append_notice(&self, session_id: &str, text: &str) -> Result<(), String>;
    /// Ask a window that shows this chat to follow it live, as
    /// `workspace_send_prompt` does for a chat it writes into: a turn or a
    /// message the watch adds is not one the window started, so without this
    /// its open tab shows it only after a reload. Best-effort.
    async fn show_in_open_tab(&self, session_id: &str);
}

/// The provenance every watch message carries: injected by Biorouter on this
/// chat's own behalf, so it is framed like other injected text (the status it
/// quotes came from an extension) and never counts as the person typing.
fn watch_provenance(session_id: &str) -> crate::conversation::message::MessageProvenance {
    crate::conversation::message::MessageProvenance {
        kind: crate::conversation::message::ProvenanceKind::AgentInjection,
        from_session_id: Some(session_id.to_string()),
        from_session_name: Some(WATCH_SENDER.to_string()),
    }
}

/// [`WatchHost`] over the daemon: the agent registry, the workspace services
/// (the turn lock and the detached-turn runner `workspace_send_prompt` uses),
/// the session store and the usage limits.
pub(super) struct LiveHost {
    session_manager: Arc<crate::session::SessionManager>,
}

impl LiveHost {
    async fn agent(&self, session_id: &str) -> Option<Arc<crate::agents::Agent>> {
        let manager = crate::execution::manager::AgentManager::instance()
            .await
            .ok()?;
        manager.peek_agent(session_id).await
    }

    fn message(session_id: &str, text: &str) -> crate::conversation::message::Message {
        let body =
            crate::conversation::message::frame_workspace_injection(Some(WATCH_SENDER), text);
        crate::conversation::message::Message::user()
            .with_text(body)
            .with_provenance(watch_provenance(session_id))
    }
}

#[async_trait]
impl WatchHost for LiveHost {
    fn has_daemon(&self) -> bool {
        crate::workspace_services::get().is_some()
    }

    async fn check_context(
        &self,
        session_id: &str,
    ) -> Option<(CallCapability, Arc<ExtensionManager>)> {
        let agent = self.agent(session_id).await?;
        // Issue #56: the ONE read of the chat's provider for this check,
        // carried through its boundary refusals and its dispatch. A check is
        // its own call, made long after the turn that admitted the watch, so
        // the turn's capability is stale: a chat moved onto a public model
        // must not keep fetching a private extension's status for it.
        let cap = CallCapability::sample(&agent.provider).await;
        Some((cap, Arc::clone(&agent.extension_manager)))
    }

    async fn incarnation(&self, session_id: &str) -> Option<i64> {
        self.session_manager
            .session_incarnation(session_id)
            .await
            .ok()
            .flatten()
    }

    async fn chat_is_current(&self, session_id: &str, incarnation: Option<i64>) -> bool {
        match self.session_manager.session_incarnation(session_id).await {
            Ok(Some(now)) => incarnation.is_none_or(|then| then == now),
            Ok(None) => false,
            // A database hiccup is not a deleted chat; the next check asks again.
            Err(_) => true,
        }
    }

    async fn limit_notice(&self, session_id: &str) -> Option<String> {
        crate::agents::usage_limit::reached_limit(&self.session_manager, session_id)
            .await
            .map(|hit| crate::agents::usage_limit::stop_notice(&hit))
    }

    fn turn_active(&self, session_id: &str) -> bool {
        crate::workspace_services::get().is_some_and(|services| services.is_turn_active(session_id))
    }

    async fn try_steer(&self, session_id: &str, text: &str) -> Result<(), String> {
        let agent = self
            .agent(session_id)
            .await
            .ok_or("the chat has no live agent")?;
        agent
            .try_queue_soft_interrupt(text.to_string(), Some(watch_provenance(session_id)))
            .map(|_| ())
            .map_err(|refused| refused.to_string())
    }

    async fn approvals_would_go_unseen(&self, session_id: &str) -> bool {
        let Some(services) = crate::workspace_services::get() else {
            return true;
        };
        if services.gui_attached() {
            return false;
        }
        match self.agent(session_id).await {
            Some(agent) => mode_requires_approval(agent.config.biorouter_mode),
            None => true,
        }
    }

    async fn start_turn(&self, session_id: &str, text: &str) -> Result<String, String> {
        let services = crate::workspace_services::get().ok_or("no Biorouter daemon is running")?;
        services
            .start_detached_turn(session_id, Self::message(session_id, text))
            .await
    }

    async fn append_note(&self, session_id: &str, text: &str) -> Result<(), String> {
        let mut message = Self::message(session_id, text);
        self.session_manager
            .add_message_adopting_uid(session_id, &mut message)
            .await
            .map_err(|e| e.to_string())?;
        // After the row is durable, with its own id, so an open tab shows it
        // now and reconciles it with the stored row later.
        crate::session_events::publish(
            session_id,
            crate::session_events::SessionBusEvent::Agent(crate::agents::AgentEvent::Message(
                message,
            )),
        );
        Ok(())
    }

    async fn append_notice(&self, session_id: &str, text: &str) -> Result<(), String> {
        let mut message = watching_notice(text);
        self.session_manager
            .add_message_adopting_uid(session_id, &mut message)
            .await
            .map_err(|e| e.to_string())?;
        crate::session_events::publish(
            session_id,
            crate::session_events::SessionBusEvent::Agent(crate::agents::AgentEvent::Message(
                message,
            )),
        );
        Ok(())
    }

    async fn show_in_open_tab(&self, session_id: &str) {
        let Some(services) = crate::workspace_services::get() else {
            return;
        };
        if !services.gui_attached() {
            return;
        }
        // The frame `workspace_send_prompt` sends (`reflect_in_target_tab`),
        // routed to the window that holds the chat. It names the chat and
        // nothing else: the tab then reads the chat through the observer
        // route, whose own reach gate decides what it may show.
        let _ = services
            .gui_command_near(
                serde_json::json!({
                    "type": "workspace", "cmd": "observe",
                    "session_id": session_id,
                }),
                false,
                session_id,
            )
            .await;
    }
}

/// The user-only notice a chat shows while a watch waits for its job.
fn watching_notice(text: &str) -> crate::conversation::message::Message {
    crate::conversation::message::Message::assistant()
        .with_system_notification(
            crate::conversation::message::SystemNotificationType::InlineMessage,
            text,
        )
        .user_only()
}

/// What the person reads once the turn that set a watch has ended.
pub(super) fn watching_text(tool: &str, id: &str) -> String {
    format!(
        "Watching {tool} ({id}). This chat will continue by itself when the job finishes; no \
         model requests are made meanwhile."
    )
}

/// Whether a chat in `mode` can raise an approval card. The same answer as
/// `workspace_extension`'s rule for an injected turn: Auto and Chat never ask.
fn mode_requires_approval(mode: crate::config::BioRouterMode) -> bool {
    use crate::config::BioRouterMode;
    match mode {
        BioRouterMode::Auto | BioRouterMode::Chat => false,
        BioRouterMode::Approve | BioRouterMode::SmartApprove => true,
    }
}

#[cfg(test)]
static TEST_HOSTS: LazyLock<std::sync::Mutex<HashMap<String, Arc<dyn WatchHost>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Stand in for the daemon for one chat's watches. Keyed by chat, so tests
/// running in parallel never see each other's host.
#[cfg(test)]
pub(super) fn set_test_host(session_id: &str, host: Arc<dyn WatchHost>) {
    TEST_HOSTS
        .lock()
        .unwrap()
        .insert(session_id.to_string(), host);
}

pub(super) fn host_for(
    session_id: &str,
    session_manager: &Arc<crate::session::SessionManager>,
) -> Arc<dyn WatchHost> {
    #[cfg(test)]
    {
        if let Some(host) = TEST_HOSTS.lock().unwrap().get(session_id) {
            return Arc::clone(host);
        }
    }
    let _ = session_id;
    Arc::new(LiveHost {
        session_manager: Arc::clone(session_manager),
    })
}

/// Seconds until the next background check. `previous` is the interval used
/// last (`None` before the first), `hint` the job's own recommended interval.
/// The first is the longer of 30 s and `min_interval`; each later one is half
/// again as long, up to five minutes; a longer hint wins, up to 15 minutes.
pub(super) fn next_interval(min_interval: u64, previous: Option<u64>, hint: Option<u64>) -> u64 {
    let floor = min_interval.max(FIRST_INTERVAL_FLOOR_SECS);
    let grown = match previous {
        None => floor,
        Some(previous) => previous.saturating_add(previous / 2).max(floor),
    };
    let ceiling = BACKOFF_CEILING_SECS.max(floor);
    grown
        .min(ceiling)
        .max(hint.unwrap_or(0).min(HINT_CEILING_SECS))
}

/// A validated watch.
#[derive(Debug, Clone)]
pub(super) struct WatchSpec {
    pub id: String,
    pub session_id: String,
    pub incarnation: Option<i64>,
    pub plan: WaitPlan,
    /// The arguments the first check was admitted with; every later check
    /// repeats exactly these.
    pub admitted: String,
    pub note: String,
    /// Seconds the watch may last.
    pub max_wait: u64,
}

/// How a watch ended.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum WatchEnd {
    /// The job ended (or the result changed or matched), a check kept
    /// failing, or the watch timed out.
    Job(WaitEnd),
    /// The chat was deleted, or is no longer the chat the watch was set in.
    ChatGone,
    /// The chat has no live agent any more (it was stopped).
    AgentGone,
    /// Cancelled; see [`StopCause`].
    Cancelled(StopCause),
}

#[derive(Debug, Clone)]
pub(super) struct WatchReport {
    pub end: WatchEnd,
    pub checks: u32,
    pub elapsed_secs: u64,
    pub last: Option<String>,
}

/// What the first check left for the background loop.
#[derive(Debug, Clone, Default)]
pub(super) struct FirstCheck {
    pub first: Option<String>,
    pub last: Option<String>,
    pub hint: Option<u64>,
}

fn seconds_up(duration: Duration, unit: Duration) -> u64 {
    let unit = unit.as_nanos().max(1);
    u64::try_from(duration.as_nanos().div_ceil(unit)).unwrap_or(u64::MAX)
}

/// One background check: the chat's capability sampled now, the boundary
/// refusals on the admitted arguments, then the dispatch. `None` when the
/// chat has no live agent.
async fn background_check(
    host: &dyn WatchHost,
    spec: &WatchSpec,
    stop: &CancellationToken,
) -> Option<CheckReading> {
    let (cap, manager) = host.check_context(&spec.session_id).await?;
    if let Err(refusal) =
        CodeExecutionClient::boundary_check(cap, &spec.session_id, &spec.plan.tool, &spec.admitted)
            .await
    {
        return Some(CheckReading {
            value: Err(refusal.error),
            activity: JobActivity::Unknown,
            hint_secs: None,
        });
    }
    // The executed-call records of a background check have no tool result to
    // ride on, so they are collected and dropped.
    let collected = Arc::new(Mutex::new(CollectedArtifacts::default()));
    let weak = Arc::downgrade(&manager);
    let (value, reading) = CodeExecutionClient::dispatch_and_record(
        &spec.session_id,
        cap,
        &spec.plan.tool,
        &spec.admitted,
        Some(&weak),
        &collected,
        stop,
        PacingMode::WatchCheck,
    )
    .await;
    Some(CheckReading {
        value: value.map_err(|error| error.message),
        activity: reading.activity,
        hint_secs: reading.hint_secs,
    })
}

fn stop_cause(cause: &std::sync::Mutex<Option<StopCause>>) -> StopCause {
    cause
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .unwrap_or(StopCause::Person)
}

/// Check in the background until the job is done, the watch times out, or it
/// is cancelled. `started` is when `watch_job` was called; the first check
/// (made in the turn) is check 1.
pub(super) async fn run_watch(
    host: &dyn WatchHost,
    spec: &WatchSpec,
    first: FirstCheck,
    started: tokio::time::Instant,
    unit: Duration,
    stop: &CancellationToken,
    cause: &std::sync::Mutex<Option<StopCause>>,
) -> WatchReport {
    let deadline = started + unit * u32::try_from(spec.max_wait).unwrap_or(u32::MAX);
    let mut checks = 1u32;
    let mut failures = 0u32;
    let FirstCheck {
        mut first,
        mut last,
        mut hint,
    } = first;
    let mut previous = None;
    let report = |end, checks, last| WatchReport {
        end,
        checks,
        elapsed_secs: seconds_up(started.elapsed(), unit),
        last,
    };
    loop {
        let interval = next_interval(spec.plan.min_interval, previous, hint);
        previous = Some(interval);
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let sleep = (unit * u32::try_from(interval).unwrap_or(u32::MAX)).min(remaining);
        tokio::select! {
            biased;
            () = stop.cancelled() => {
                return report(WatchEnd::Cancelled(stop_cause(cause)), checks, last);
            }
            () = tokio::time::sleep(sleep) => {}
        }
        if !host
            .chat_is_current(&spec.session_id, spec.incarnation)
            .await
        {
            return report(WatchEnd::ChatGone, checks, last);
        }
        let reading = tokio::select! {
            biased;
            () = stop.cancelled() => {
                return report(WatchEnd::Cancelled(stop_cause(cause)), checks, last);
            }
            reading = background_check(host, spec, stop) => reading,
        };
        let Some(reading) = reading else {
            return report(WatchEnd::AgentGone, checks, last);
        };
        checks += 1;
        match reading.value {
            Err(error) => {
                failures += 1;
                if failures >= MAX_CONSECUTIVE_FAILURES {
                    return report(WatchEnd::Job(WaitEnd::Failed(error)), checks, last);
                }
            }
            Ok(value) => {
                failures = 0;
                hint = reading.hint_secs;
                last = Some(value.clone());
                if let Some(end) = wait::end_of(
                    &spec.plan.until,
                    &value,
                    reading.activity,
                    checks,
                    &mut first,
                ) {
                    return report(WatchEnd::Job(end), checks, last);
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return report(WatchEnd::Job(WaitEnd::TimedOut), checks, last);
        }
    }
}

/// The message the chat is resumed with: a headline, the job's last state in
/// short, and the model's own note.
pub(super) fn wake_text(spec: &WatchSpec, report: &WatchReport) -> Option<String> {
    let over = format!(
        "{} over {}",
        wait::checks_words(report.checks),
        wait::duration_words(report.elapsed_secs)
    );
    let tool = &spec.plan.tool;
    let headline = match &report.end {
        WatchEnd::Job(WaitEnd::Finished | WaitEnd::NotAJob) => {
            format!("{tool}: the job is no longer active ({over}).")
        }
        WatchEnd::Job(WaitEnd::Changed) => format!("{tool}: the result changed ({over})."),
        WatchEnd::Job(WaitEnd::Matched(value)) => {
            let pointer = match &spec.plan.until {
                Until::Pointer { pointer, .. } => pointer.as_str(),
                _ => "the value",
            };
            format!("{tool}: {pointer} is {value} ({over}).")
        }
        WatchEnd::Job(WaitEnd::TimedOut) => format!(
            "{tool}: the job is still running after {}, the watch's limit, so the watch \
             stopped. Call code_execution__watch_job again to keep watching.",
            wait::duration_words(report.elapsed_secs)
        ),
        WatchEnd::Job(WaitEnd::Failed(error)) => format!(
            "{tool}: the watch stopped because its checks kept failing ({over}). The job may \
             still be running. Last error: {}",
            truncate_chars(error, 400)
        ),
        WatchEnd::AgentGone => format!(
            "{tool}: the watch ended because this chat was stopped ({over}). The job may still \
             be running; set a new watch to be resumed when it ends."
        ),
        WatchEnd::Cancelled(StopCause::Person) => format!(
            "{tool}: the watch was cancelled from the active work list ({over}); this chat \
             will not be resumed for it."
        ),
        WatchEnd::Cancelled(StopCause::TurnStopped) => {
            format!("{tool}: Stop ended the watch ({over}); this chat will not be resumed for it.")
        }
        // The model asked for it and was told; a deleted chat has nobody to
        // tell. (`run_watch` reports a cancellation as `WatchEnd::Cancelled`,
        // never as the wait's own `Cancelled`.)
        WatchEnd::Cancelled(StopCause::Agent | StopCause::ChatDeleted | StopCause::SeenEnded)
        | WatchEnd::ChatGone
        | WatchEnd::Job(WaitEnd::Cancelled) => return None,
    };
    let mut text = format!("[watch {}] {headline}", spec.id);
    if let Some(last) = &report.last {
        text.push_str("\nLast result: ");
        text.push_str(&wait::compact_state(last, MAX_STATE_CHARS));
    }
    if !spec.note.is_empty() {
        text.push_str("\nYour note: ");
        text.push_str(&spec.note);
    }
    Some(text)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}...", text.chars().take(max).collect::<String>())
    }
}

/// How the chat was told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Delivery {
    /// Steered into the running turn.
    Steered,
    /// A new turn was started, with this id.
    TurnStarted(String),
    /// Stored as a note, with no turn: the job end the chat could not act on
    /// (a reached usage limit, approvals nobody could see, a turn that could
    /// not start), or a cancellation.
    Noted,
    /// Nothing could be stored; the reason was logged.
    Lost,
}

fn is_busy(error: &str) -> bool {
    error.contains("already in flight")
}

/// Resume the chat once with `text`: a steer into a running turn, otherwise a
/// new turn, otherwise a note. A chat paused by a usage limit gets a note and
/// no turn.
pub(super) async fn deliver(
    host: &dyn WatchHost,
    session_id: &str,
    text: &str,
    unit: Duration,
) -> Delivery {
    let note = |extra: String| async move {
        let body = if extra.is_empty() {
            text.to_string()
        } else {
            format!("{text}\n{extra}")
        };
        match host.append_note(session_id, &body).await {
            Ok(()) => Delivery::Noted,
            Err(error) => {
                tracing::warn!(session_id, "a job watch could not store its note: {error}");
                Delivery::Lost
            }
        }
    };
    if let Some(notice) = host.limit_notice(session_id).await {
        return note(format!(
            "This chat is paused by a usage limit, so the watch did not resume it. {notice}"
        ))
        .await;
    }
    let mut last_error = String::new();
    for attempt in 0..WAKE_ATTEMPTS {
        if attempt > 0 {
            // A turn that is closing refuses a steer and still holds the turn
            // lock; it lets go within a moment.
            tokio::time::sleep(unit * 2).await;
        }
        if host.turn_active(session_id) {
            match host.try_steer(session_id, text).await {
                Ok(()) => return Delivery::Steered,
                Err(error) => last_error = error,
            }
            continue;
        }
        if host.approvals_would_go_unseen(session_id).await {
            return note(
                "The watch did not start a turn: this chat asks before running tools and no \
                 Biorouter window is open to ask in. Send any message to continue."
                    .to_string(),
            )
            .await;
        }
        // Asked before the turn starts as well as after, so a tab that
        // attaches in time sees the turn's start and every frame of it live.
        host.show_in_open_tab(session_id).await;
        match host.start_turn(session_id, text).await {
            Ok(turn) => return Delivery::TurnStarted(turn),
            Err(error) if is_busy(&error) => last_error = error,
            Err(error) => {
                return note(format!(
                    "The watch could not resume this chat ({error}). Send any message to continue."
                ))
                .await
            }
        }
    }
    note(format!(
        "The watch could not resume this chat ({last_error}). Send any message to continue."
    ))
    .await
}

/// Which watch a [`follow_creating_turn`] follows, and in which chat.
struct CreatingTurn {
    session_id: String,
    watch_id: String,
    tool: String,
}

/// Watch the creating turn for Stop until it ends. A cancelled token after the
/// turn has ended is cleanup, not Stop, which is why the turn's state is asked.
///
/// When the turn ends and the watch is still live, the chat gets a user-only
/// notice saying it is being watched, and its open tab is asked to follow it:
/// the turn that set a watch otherwise ends with nothing on screen, and the
/// turn the watch starts later is not one the window started.
async fn follow_creating_turn(
    host: Arc<dyn WatchHost>,
    turn: CreatingTurn,
    creating: CancellationToken,
    stop: CancellationToken,
    cause: Arc<std::sync::Mutex<Option<StopCause>>>,
    unit: Duration,
) {
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            () = creating.cancelled() => {
                if host.turn_active(&turn.session_id) {
                    cause
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get_or_insert(StopCause::TurnStopped);
                    stop.cancel();
                    return;
                }
                break;
            }
            () = tokio::time::sleep(unit) => {
                if !host.turn_active(&turn.session_id) {
                    break;
                }
            }
        }
    }
    announce_watch(host.as_ref(), &turn, &stop).await;
}

/// The notice and the open-tab request for a watch whose creating turn has
/// ended. Nothing when the watch is already over (the model's own read of the
/// job's end retired it, or it was cancelled).
async fn announce_watch(host: &dyn WatchHost, turn: &CreatingTurn, stop: &CancellationToken) {
    if stop.is_cancelled() || !watches().contains_key(&turn.watch_id) {
        return;
    }
    if let Err(error) = host
        .append_notice(&turn.session_id, &watching_text(&turn.tool, &turn.watch_id))
        .await
    {
        tracing::warn!(session_id = %turn.session_id, watch = %turn.watch_id, "a job watch could not store its notice: {error}");
    }
    host.show_in_open_tab(&turn.session_id).await;
}

/// The background half of one watch, from the first sleep to the wake.
async fn watch_task(
    host: Arc<dyn WatchHost>,
    spec: WatchSpec,
    first: FirstCheck,
    started: tokio::time::Instant,
    stop: CancellationToken,
    cause: Arc<std::sync::Mutex<Option<StopCause>>>,
) {
    let unit = wait::wait_unit();
    let report = run_watch(host.as_ref(), &spec, first, started, unit, &stop, &cause).await;
    // Out of the registry first, so the active work list and a duplicate check
    // stop showing a watch that has ended, then end the creating-turn follower.
    watches().remove(&spec.id);
    // A read elsewhere that saw the job end can land between `run_watch`
    // returning on the same end and the line above; nothing can cancel the
    // watch after it. The model has read that end, so the wake is skipped
    // either way.
    let seen_ended = matches!(
        *cause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        Some(StopCause::SeenEnded)
    );
    biorouter_mcp::active_work::active_work().deregister(&spec.id);
    stop.cancel();
    tracing::info!(
        session_id = %spec.session_id,
        watch = %spec.id,
        tool = %spec.plan.tool,
        checks = report.checks,
        elapsed_secs = report.elapsed_secs,
        end = ?report.end,
        "job watch ended"
    );
    // A chat that is gone is not resumed, and neither is one whose model
    // cancelled the watch itself.
    if matches!(report.end, WatchEnd::ChatGone) {
        return;
    }
    if !host
        .chat_is_current(&spec.session_id, spec.incarnation)
        .await
    {
        return;
    }
    if seen_ended {
        return;
    }
    let Some(text) = wake_text(&spec, &report) else {
        return;
    };
    let resumes = matches!(report.end, WatchEnd::Job(_));
    let delivery = if resumes {
        deliver(host.as_ref(), &spec.session_id, &text, unit).await
    } else {
        match host.append_note(&spec.session_id, &text).await {
            Ok(()) => Delivery::Noted,
            Err(error) => {
                tracing::warn!(session_id = %spec.session_id, "a job watch could not store its note: {error}");
                Delivery::Lost
            }
        }
    };
    tracing::info!(session_id = %spec.session_id, watch = %spec.id, ?delivery, "job watch delivered");
    // What the watch added, a turn or a note, was not started by any window,
    // so a tab showing the chat is asked to follow it (a turn it is driving
    // itself keeps its own stream; the renderer refuses the attach then).
    if delivery != Delivery::Lost {
        host.show_in_open_tab(&spec.session_id).await;
    }
}

/// The status call's arguments in one canonical spelling.
fn canonical(arguments: &JsonObject) -> String {
    fn sort(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys: Vec<&String> = object.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), sort(&object[key])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    sort(&Value::Object(arguments.clone())).to_string()
}

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text)])
}

/// How [`start_watch`] answered.
#[derive(Debug, Clone)]
pub(super) enum WatchStarted {
    /// The first check already ended the wait (`Finished`, `Matched` or
    /// `NotAJob`); no watch was set.
    Done { end: WaitEnd, value: String },
    /// A watch is running.
    Watching {
        id: String,
        first_interval: u64,
        max_wait: u64,
        value: String,
    },
    /// The same tool with the same arguments is already watched.
    Already { id: String },
}

/// What [`start_watch`] needs from its caller: the top-level tool or a
/// script's `watch_job()`.
pub(super) struct WatchRequest<'a> {
    pub session_id: &'a str,
    pub cap: CallCapability,
    pub judge: Option<&'a super::ScriptJudge>,
    pub extension_manager: Option<&'a std::sync::Weak<ExtensionManager>>,
    pub collected: &'a Arc<Mutex<CollectedArtifacts>>,
    /// The token of the turn that sets the watch: Stop on it, while that
    /// turn runs, cancels the watch.
    pub turn: &'a CancellationToken,
    pub host: Arc<dyn WatchHost>,
}

/// A watch's plan, note and length in seconds, or why the arguments are
/// refused.
fn validate(params: WatchJobParams) -> Result<(WaitPlan, String, u64), String> {
    let note = params.note.as_deref().unwrap_or("").trim().to_string();
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(format!(
            "watch_job's note is at most {MAX_NOTE_CHARS} characters; say briefly what to do \
             when resumed"
        ));
    }
    let max_wait = match params.max_wait_minutes {
        Some(minutes) if !minutes.is_finite() || minutes <= 0.0 => {
            return Err("max_wait_minutes must be a positive number of minutes".to_string())
        }
        Some(minutes) => ((minutes * 60.0).ceil() as u64).clamp(1, MAX_WAIT_MINUTES * 60),
        None => DEFAULT_MAX_WAIT_MINUTES * 60,
    };
    let plan = WaitPlan::from_params(
        WaitForParams {
            tool: params.tool.unwrap_or_default(),
            arguments: params.arguments,
            until: params.until,
            timeout_seconds: Some(max_wait as f64),
            min_interval_seconds: params.min_interval_seconds,
        },
        MAX_WAIT_MINUTES * 60,
    )
    .map_err(|error| error.replacen("wait_for", "watch_job", 1))?;
    Ok((plan, note, max_wait))
}

/// Register a watch whose first check found the job still running, and
/// start its background task and its follower of the creating turn. Returns
/// the watch id (`spec.id` is filled in here).
fn launch(
    host: Arc<dyn WatchHost>,
    session_id: &str,
    turn: &CancellationToken,
    mut spec: WatchSpec,
    first_check: FirstCheck,
    started: tokio::time::Instant,
) -> String {
    let stop = CancellationToken::new();
    let cause = Arc::new(std::sync::Mutex::new(None));
    let id = {
        let stop = stop.clone();
        let cause = Arc::clone(&cause);
        // The chat is named on the row: `GET /active_work` shows it only to a
        // caller that could open this chat (issue #56).
        biorouter_mcp::active_work::active_work().register(
            biorouter_mcp::active_work::ActiveWorkKind::JobWatch,
            format!("Watching {}", spec.plan.tool),
            (!spec.note.is_empty()).then(|| truncate_chars(&spec.note, 200)),
            Some(session_id.to_string()),
            Some(Arc::new(move || {
                cause
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get_or_insert(StopCause::Person);
                stop.cancel();
            })),
        )
    };
    spec.id = id.clone();
    let admitted = serde_json::from_str::<Value>(&spec.admitted)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map_or_else(
            || canonical(&spec.plan.arguments),
            |object| canonical(&object),
        );
    watches().insert(
        id.clone(),
        WatchEntry {
            session_id: session_id.to_string(),
            tool: spec.plan.tool.clone(),
            arguments: canonical(&spec.plan.arguments),
            admitted,
            until: spec.plan.until.clone(),
            stop: stop.clone(),
            cause: Arc::clone(&cause),
        },
    );
    tokio::spawn(follow_creating_turn(
        Arc::clone(&host),
        CreatingTurn {
            session_id: session_id.to_string(),
            watch_id: id.clone(),
            tool: spec.plan.tool.clone(),
        },
        turn.clone(),
        stop.clone(),
        Arc::clone(&cause),
        wait::wait_unit(),
    ));
    tokio::spawn(watch_task(host, spec, first_check, started, stop, cause));
    id
}

/// Validate a watch, check the job once in the turn, and, when it is still
/// running, register the watch and start its background task. `params` must
/// not be a cancel request. The caller has already confirmed the tool is in
/// this chat's importable catalogue.
pub(super) async fn start_watch(
    request: WatchRequest<'_>,
    params: WatchJobParams,
) -> Result<WatchStarted, String> {
    let WatchRequest {
        session_id,
        cap,
        judge,
        extension_manager,
        collected,
        turn,
        host,
    } = request;
    if !host.has_daemon() {
        return Err(
            "watch_job needs the Biorouter app or daemon, which resumes this chat when the job \
             ends. Use code_execution__wait_for here instead."
                .to_string(),
        );
    }
    let (plan, note, max_wait) = validate(params)?;
    let arguments_key = canonical(&plan.arguments);
    {
        let watches = watches();
        if let Some((id, _)) = watches.iter().find(|(_, e)| {
            e.session_id == session_id && e.tool == plan.tool && e.arguments == arguments_key
        }) {
            return Ok(WatchStarted::Already { id: id.clone() });
        }
        let mine = watches
            .values()
            .filter(|e| e.session_id == session_id)
            .count();
        if mine >= MAX_WATCHES_PER_CHAT {
            return Err(format!(
                "this chat already has {mine} job watches, the most it may hold; cancel one with \
                 watch_job {{\"cancel\": \"<id>\"}} first"
            ));
        }
        if watches.len() >= MAX_WATCHES {
            return Err(
                "Biorouter is already watching as many jobs as it will; try again when one ends, \
                 or use code_execution__wait_for"
                    .to_string(),
            );
        }
    }
    let incarnation = host.incarnation(session_id).await;

    // The first check runs here, in the turn, admitted in full: boundary
    // refusals, then the judge. The judge is a task-local that does not
    // survive a spawn, which is why every later check repeats this admitted
    // call rather than asking again.
    let started = tokio::time::Instant::now();
    let mut checker = ManagedCheck {
        session_id,
        cap,
        judge,
        extension_manager,
        collected_artifacts: collected,
        cancellation_token: turn,
        tool: &plan.tool,
        arguments: Value::Object(plan.arguments.clone()).to_string(),
        admitted: None,
    };
    let reading = wait::StatusCheck::check(&mut checker).await;
    let admitted = checker.admitted.clone();
    let value = reading.value.map_err(|error| {
        format!(
            "watch_job's first check of {} failed, so no watch was set: {error}",
            plan.tool
        )
    })?;
    let mut first = None;
    if let Some(end) = wait::end_of(&plan.until, &value, reading.activity, 1, &mut first) {
        return Ok(WatchStarted::Done { end, value });
    }
    let admitted = admitted.ok_or("watch_job: the first check was not admitted")?;
    let first_interval = next_interval(plan.min_interval, None, reading.hint_secs);

    let id = launch(
        host,
        session_id,
        turn,
        WatchSpec {
            id: String::new(),
            session_id: session_id.to_string(),
            incarnation,
            plan,
            admitted,
            note,
            max_wait,
        },
        FirstCheck {
            first,
            last: Some(value.clone()),
            hint: reading.hint_secs,
        },
        started,
    );
    Ok(WatchStarted::Watching {
        id,
        first_interval,
        max_wait,
        value,
    })
}

/// A [`WatchRequest`] without its host, as the script's tool handler holds it.
pub(super) struct WatchRequestParts<'a> {
    pub session_id: &'a str,
    pub cap: CallCapability,
    pub judge: Option<&'a super::ScriptJudge>,
    pub extension_manager: Option<&'a std::sync::Weak<ExtensionManager>>,
    pub collected: &'a Arc<Mutex<CollectedArtifacts>>,
    pub turn: &'a CancellationToken,
}

/// Serve a script's `watch_job({...})`: the same watch as the top-level tool
/// (its first check admitted like the script's other calls), answered as JSON
/// the script receives. A cancel request cancels this chat's watch. The JS
/// binding has already confirmed the tool is importable.
pub(super) async fn serve_script_watch(
    parts: WatchRequestParts<'_>,
    host: Option<&Arc<dyn WatchHost>>,
    payload: &str,
) -> Result<String, super::ScriptFailure> {
    let params: WatchJobParams = serde_json::from_str(payload)
        .map_err(|error| format!("watch_job: invalid arguments: {error}"))?;
    if let Some(target) = params.cancel.as_deref() {
        return cancel_by_agent(parts.session_id, target.trim())
            .map(|text| serde_json::json!({ "cancelled": text }).to_string())
            .map_err(Into::into);
    }
    let host = host.ok_or_else(|| {
        "watch_job() cannot resume a chat from here; use wait_for() or sleep() instead".to_string()
    })?;
    start_watch(
        WatchRequest {
            session_id: parts.session_id,
            cap: parts.cap,
            judge: parts.judge,
            extension_manager: parts.extension_manager,
            collected: parts.collected,
            turn: parts.turn,
            host: Arc::clone(host),
        },
        params,
    )
    .await
    .map(|started| started_json(&started))
    .map_err(Into::into)
}

/// The sentence every started or found watch ends on.
const END_YOUR_TURN: &str = "End your turn; this chat resumes automatically when the job finishes.";

/// What the model reads back from the top-level tool.
fn started_text(tool: &str, until: &Until, started: &WatchStarted) -> String {
    match started {
        WatchStarted::Done {
            end: WaitEnd::NotAJob,
            value,
        } => format!(
            "watch_job set no watch: the result of {tool} does not name a job state (no status, \
             state or phase field reading queued, running, completed and so on). To watch this \
             result, call watch_job again with until: \"changed\" or until: {{\"pointer\": \
             \"/field\", \"values\": [...]}}.\nResult:\n{}",
            wait::shown_result(value)
        ),
        WatchStarted::Done { end, value } => {
            let what = match (end, until) {
                (WaitEnd::Matched(found), Until::Pointer { pointer, .. }) => {
                    format!("{pointer} is already {found}")
                }
                _ => "the job is already done".to_string(),
            };
            format!(
                "watch_job set no watch: {what}.\nLast result of {tool}:\n{}",
                wait::shown_result(value)
            )
        }
        WatchStarted::Already { id } => {
            format!("Already watching {tool} with these arguments as {id}. {END_YOUR_TURN}")
        }
        WatchStarted::Watching {
            id,
            first_interval,
            max_wait,
            value,
        } => format!(
            "Watching {tool} as {id}. Biorouter checks it in the background with no model turns: \
             first in {first_interval} s, then less often (up to every 5 min), for at most {} \
             min. {END_YOUR_TURN} Do not check the job yourself meanwhile. Cancel with \
             code_execution__watch_job {{\"cancel\": \"{id}\"}}. The watch lives in memory: if \
             Biorouter restarts it is dropped and this chat is not resumed.\nCurrent state: {}",
            max_wait.div_ceil(60),
            wait::compact_state(value, MAX_FIRST_STATE_CHARS),
        ),
    }
}

/// What a script's `watch_job({...})` returns, as JSON: `{state: "watching",
/// watch_id}` (the script should then end and the model end its turn), or
/// `{state: "finished" | "matched" | "not_a_job", last_result}`.
pub(super) fn started_json(started: &WatchStarted) -> String {
    let last = |value: &str| {
        serde_json::from_str::<Value>(value.trim()).unwrap_or_else(|_| Value::String(value.into()))
    };
    let json = match started {
        WatchStarted::Done { end, value } => serde_json::json!({
            "state": match end {
                WaitEnd::NotAJob => "not_a_job",
                WaitEnd::Matched(_) => "matched",
                WaitEnd::Changed => "changed",
                _ => "finished",
            },
            "last_result": last(value),
        }),
        WatchStarted::Already { id } => serde_json::json!({
            "state": "watching",
            "watch_id": id,
            "note": END_YOUR_TURN,
        }),
        WatchStarted::Watching { id, value, .. } => serde_json::json!({
            "state": "watching",
            "watch_id": id,
            "current": wait::compact_state(value, MAX_FIRST_STATE_CHARS),
            "note": END_YOUR_TURN,
        }),
    };
    json.to_string()
}

impl CodeExecutionClient {
    /// `code_execution__watch_job`.
    pub(super) async fn handle_watch_job(
        &self,
        session_id: &str,
        cap: CallCapability,
        arguments: Option<JsonObject>,
        cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, String> {
        let params: WatchJobParams =
            serde_json::from_value(Value::Object(arguments.unwrap_or_default()))
                .map_err(|error| format!("invalid watch_job arguments: {error}"))?;
        if let Some(target) = params.cancel.as_deref() {
            return Ok(text_result(cancel_by_agent(session_id, target.trim())?));
        }
        let host = host_for(session_id, &self.context.session_manager);
        let tool = params.tool.clone().unwrap_or_default();
        // The daemon question first, so its answer is not hidden behind the
        // catalogue's; `start_watch` asks it again for a script's watch.
        if !host.has_daemon() {
            return Err(
                "watch_job needs the Biorouter app or daemon, which resumes this chat when the \
                 job ends. Use code_execution__wait_for here instead."
                    .to_string(),
            );
        }
        // The reach a script has, as for wait_for.
        let catalogue = self.get_catalogue(Some(cap)).await;
        let known = catalogue.iter().any(|t| t.name.as_ref() == tool.trim());
        // `start_watch` gives a better reason for a name it refuses outright
        // (code_execution's own tools, platform tools, a bare name).
        let judge = if known {
            Self::script_judge(session_id, &catalogue)?
        } else {
            None
        };
        let collected = Arc::new(Mutex::new(CollectedArtifacts::default()));
        // Only read for the wording of an answer that matched at once.
        let until = match &params.until {
            Some(WaitUntilParam::Pointer { pointer, values }) => Until::Pointer {
                pointer: pointer.clone(),
                values: values.clone(),
            },
            _ => Until::Inactive,
        };
        let started = if known {
            start_watch(
                WatchRequest {
                    session_id,
                    cap,
                    judge: judge.as_ref(),
                    extension_manager: self.context.extension_manager.as_ref(),
                    collected: &collected,
                    turn: &cancellation_token,
                    host,
                },
                params.clone(),
            )
            .await
        } else {
            // Validate first, so a code_execution or platform tool gets its
            // own refusal rather than "not a tool this chat can call".
            WaitPlan::from_params(
                WaitForParams {
                    tool,
                    arguments: None,
                    until: None,
                    timeout_seconds: None,
                    min_interval_seconds: None,
                },
                MAX_WAIT_MINUTES * 60,
            )
            .map_err(|error| error.replacen("wait_for", "watch_job", 1))
            .and_then(|plan| {
                Err(format!(
                    "watch_job: `{}` is not a tool this chat can call from code_execution. Name \
                     the status tool by its full name, server__tool, as search_modules lists it.",
                    plan.tool
                ))
            })
        };
        let meta = Self::collected_meta(&*collected.lock().await);
        let mut result = match started {
            Ok(started) => text_result(started_text(
                params.tool.as_deref().unwrap_or_default().trim(),
                &until,
                &started,
            )),
            Err(error) => CallToolResult::error(vec![Content::text(format!("Error: {error}"))]),
        };
        result.meta = meta;
        Ok(result)
    }
}

/// `watch_job {"cancel": target}`: cancel one of this chat's watches, or
/// "all" of them. Another chat's watch is not this chat's to cancel, and is
/// reported as unknown.
fn cancel_by_agent(session_id: &str, target: &str) -> Result<String, String> {
    let watches = watches();
    let mine: Vec<(&String, &WatchEntry)> = watches
        .iter()
        .filter(|(id, e)| e.session_id == session_id && (target == "all" || id.as_str() == target))
        .collect();
    if mine.is_empty() {
        let live = watches
            .iter()
            .filter(|(_, e)| e.session_id == session_id)
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>();
        return Err(if live.is_empty() {
            format!("watch_job: `{target}` is not a live watch of this chat; it has none")
        } else {
            format!(
                "watch_job: `{target}` is not a live watch of this chat; its watches are {}",
                live.join(", ")
            )
        });
    }
    let mut ids: Vec<String> = mine.iter().map(|(id, _)| (*id).clone()).collect();
    for (_, entry) in &mine {
        entry.cancel(StopCause::Agent);
    }
    ids.sort_by_key(|id| watch_number(id));
    Ok(format!(
        "Cancelled {}. This chat will not be resumed for {}.",
        ids.join(", "),
        if ids.len() == 1 { "it" } else { "them" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_starts_at_30_seconds_and_grows_toward_five_minutes() {
        let mut previous = None;
        let mut seen = Vec::new();
        for _ in 0..8 {
            let next = next_interval(10, previous, None);
            seen.push(next);
            previous = Some(next);
        }
        assert_eq!(seen, vec![30, 45, 67, 100, 150, 225, 300, 300]);
    }

    #[test]
    fn the_backoff_honours_a_longer_hint_and_a_longer_minimum() {
        // A job asking for 60 s gets 60 s from the start.
        assert_eq!(next_interval(30, None, Some(60)), 60);
        // A 5 s hint never shortens the 30 s floor.
        assert_eq!(next_interval(30, None, Some(5)), 30);
        // A hint longer than the ceiling is honoured up to 15 minutes.
        assert_eq!(next_interval(30, Some(300), Some(600)), 600);
        assert_eq!(next_interval(30, Some(300), Some(5000)), HINT_CEILING_SECS);
        // A minimum above the ceiling is kept rather than cut to it.
        assert_eq!(next_interval(400, Some(400), None), 400);
        // A hint that drops again lets the backoff carry on from where it was.
        assert_eq!(next_interval(30, Some(60), Some(5)), 90);
    }

    #[test]
    fn wake_text_is_short_and_carries_the_note() {
        let spec = WatchSpec {
            id: "watch-7".into(),
            session_id: "s".into(),
            incarnation: None,
            plan: WaitPlan {
                tool: "cdw__status".into(),
                arguments: JsonObject::new(),
                until: Until::Inactive,
                timeout: 3600,
                min_interval: 30,
            },
            admitted: "{}".into(),
            note: "Summarise the export and plot it.".into(),
            max_wait: 3600,
        };
        let big = serde_json::json!({
            "status": "completed",
            "rows": 12,
            "elapsed_seconds": 900,
            "preview": "x".repeat(5000),
        })
        .to_string();
        let report = WatchReport {
            end: WatchEnd::Job(WaitEnd::Finished),
            checks: 9,
            elapsed_secs: 905,
            last: Some(big),
        };
        let text = wake_text(&spec, &report).unwrap();
        assert!(text.starts_with("[watch watch-7] cdw__status: the job is no longer active (9 checks over 15 min 5 s)."), "{text}");
        assert!(text.contains("\"status\":\"completed\""), "{text}");
        assert!(text.contains("(long values cut)"), "{text}");
        assert!(
            !text.contains("elapsed_seconds"),
            "timing fields are left out: {text}"
        );
        assert!(
            text.ends_with("Your note: Summarise the export and plot it."),
            "{text}"
        );
        assert!(text.chars().count() < 2100, "{}", text.chars().count());

        for (end, silent) in [
            (WatchEnd::ChatGone, true),
            (WatchEnd::Cancelled(StopCause::Agent), true),
            (WatchEnd::Cancelled(StopCause::ChatDeleted), true),
            (WatchEnd::Cancelled(StopCause::Person), false),
            (WatchEnd::Cancelled(StopCause::TurnStopped), false),
            (WatchEnd::AgentGone, false),
            (WatchEnd::Job(WaitEnd::TimedOut), false),
        ] {
            let report = WatchReport {
                end: end.clone(),
                ..report.clone()
            };
            assert_eq!(wake_text(&spec, &report).is_none(), silent, "{end:?}");
        }
        let timed_out = wake_text(
            &spec,
            &WatchReport {
                end: WatchEnd::Job(WaitEnd::TimedOut),
                ..report.clone()
            },
        )
        .unwrap();
        assert!(timed_out.contains("still running"), "{timed_out}");
        assert!(timed_out.contains("watch_job again"), "{timed_out}");
    }

    #[test]
    fn arguments_compare_in_one_spelling() {
        let a = serde_json::json!({"b": 1, "a": {"y": 2, "x": 1}});
        let b = serde_json::json!({"a": {"x": 1, "y": 2}, "b": 1});
        assert_eq!(
            canonical(a.as_object().unwrap()),
            canonical(b.as_object().unwrap())
        );
    }
}

/// `watch_job` end to end: the tool call, the first check through the real
/// `ExtensionManager`, the background checks, and the wake, with a stand-in
/// for the daemon ([`WatchHost`]). Times run on the 10 ms test unit
/// (`wait::wait_unit`), so a 30 s interval is 300 ms.
#[cfg(test)]
mod watch_tests {
    use super::*;
    use crate::agents::extension::PlatformExtensionContext;
    use crate::agents::mcp_client::{Error, McpClientTrait, McpMeta};
    use rmcp::model::{InitializeResult, ListToolsResult, Tool as McpTool};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

    /// A status tool that answers with `results` in order, then repeats the
    /// last one.
    struct StatusFixture {
        results: std::sync::Mutex<VecDeque<CallToolResult>>,
        last: std::sync::Mutex<Option<CallToolResult>>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl McpClientTrait for StatusFixture {
        async fn list_tools(
            &self,
            _next_cursor: Option<String>,
            _cancel_token: CancellationToken,
        ) -> Result<ListToolsResult, Error> {
            Ok(ListToolsResult {
                tools: vec![McpTool::new(
                    "status".to_string(),
                    "Synthetic job status".to_string(),
                    Arc::new(
                        serde_json::json!({"type": "object", "properties": {
                            "job_id": {"type": "string"}
                        }})
                        .as_object()
                        .unwrap()
                        .clone(),
                    ),
                )],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            _name: &str,
            _arguments: Option<JsonObject>,
            _meta: McpMeta,
            _cancel_token: CancellationToken,
        ) -> Result<CallToolResult, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let next = self.results.lock().unwrap().pop_front();
            let mut last = self.last.lock().unwrap();
            if let Some(next) = next {
                *last = Some(next);
            }
            Ok(last.clone().expect("the fixture has at least one result"))
        }

        fn get_info(&self) -> Option<&InitializeResult> {
            None
        }
    }

    pub(super) fn job(status: &str) -> CallToolResult {
        CallToolResult::success(vec![Content::text(
            serde_json::json!({"job_id": "a", "status": status, "elapsed_seconds": 3}).to_string(),
        )])
    }

    /// The daemon, as far as a watch can tell: what it did is recorded.
    #[derive(Default)]
    pub(super) struct FakeHost {
        manager: std::sync::Mutex<Option<Arc<ExtensionManager>>>,
        no_daemon: AtomicBool,
        turn_active: AtomicBool,
        deleted: AtomicBool,
        agent_gone: AtomicBool,
        unseen_approvals: AtomicBool,
        busy_turns: AtomicU32,
        limit: std::sync::Mutex<Option<String>>,
        actions: std::sync::Mutex<Vec<String>>,
        notices: std::sync::Mutex<Vec<String>>,
        observes: AtomicUsize,
    }

    impl FakeHost {
        pub(super) fn actions(&self) -> Vec<String> {
            self.actions.lock().unwrap().clone()
        }
        pub(super) fn notices(&self) -> Vec<String> {
            self.notices.lock().unwrap().clone()
        }
        pub(super) fn observes(&self) -> usize {
            self.observes.load(Ordering::SeqCst)
        }
        fn act(&self, what: &str, text: &str) {
            self.actions.lock().unwrap().push(format!("{what}: {text}"));
        }
    }

    #[async_trait]
    impl WatchHost for FakeHost {
        fn has_daemon(&self) -> bool {
            !self.no_daemon.load(Ordering::SeqCst)
        }
        async fn check_context(
            &self,
            _session_id: &str,
        ) -> Option<(CallCapability, Arc<ExtensionManager>)> {
            if self.agent_gone.load(Ordering::SeqCst) {
                return None;
            }
            let manager = self.manager.lock().unwrap().clone()?;
            Some((CallCapability::for_test_restricted(), manager))
        }
        async fn incarnation(&self, _session_id: &str) -> Option<i64> {
            Some(7)
        }
        async fn chat_is_current(&self, _session_id: &str, incarnation: Option<i64>) -> bool {
            assert_eq!(
                incarnation,
                Some(7),
                "the watch keeps the chat it was set in"
            );
            !self.deleted.load(Ordering::SeqCst)
        }
        async fn limit_notice(&self, _session_id: &str) -> Option<String> {
            self.limit.lock().unwrap().clone()
        }
        fn turn_active(&self, _session_id: &str) -> bool {
            self.turn_active.load(Ordering::SeqCst)
        }
        async fn try_steer(&self, _session_id: &str, text: &str) -> Result<(), String> {
            self.act("steer", text);
            Ok(())
        }
        async fn approvals_would_go_unseen(&self, _session_id: &str) -> bool {
            self.unseen_approvals.load(Ordering::SeqCst)
        }
        async fn start_turn(&self, _session_id: &str, text: &str) -> Result<String, String> {
            if self
                .busy_turns
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err("a turn is already in flight for this session (running turn t)".into());
            }
            self.act("turn", text);
            Ok("turn-9".into())
        }
        async fn append_note(&self, _session_id: &str, text: &str) -> Result<(), String> {
            self.act("note", text);
            Ok(())
        }
        async fn append_notice(&self, _session_id: &str, text: &str) -> Result<(), String> {
            self.notices.lock().unwrap().push(text.to_string());
            Ok(())
        }
        async fn show_in_open_tab(&self, _session_id: &str) {
            self.observes.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub(super) struct Harness {
        pub(super) client: CodeExecutionClient,
        pub(super) session_id: String,
        pub(super) calls: Arc<AtomicUsize>,
        pub(super) host: Arc<FakeHost>,
        _manager: Arc<ExtensionManager>,
        _dir: tempfile::TempDir,
    }

    pub(super) async fn harness(results: Vec<CallToolResult>) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(crate::session::SessionManager::new(
            dir.path().to_path_buf(),
        ));
        let session = session_manager
            .create_session(
                dir.path().to_path_buf(),
                "watch_job".into(),
                crate::session::SessionType::User,
            )
            .await
            .unwrap();
        let manager = Arc::new(ExtensionManager::new(
            Arc::new(Mutex::new(None)),
            Arc::clone(&session_manager),
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        manager
            .add_client(
                "fixture".to_string(),
                crate::agents::extension::ExtensionConfig::Platform {
                    name: "fixture".into(),
                    description: "Job status fixture".into(),
                    bundled: Some(true),
                    available_tools: Vec::new(),
                },
                Arc::new(StatusFixture {
                    results: std::sync::Mutex::new(results.into()),
                    last: std::sync::Mutex::new(None),
                    calls: Arc::clone(&calls),
                }),
                None,
                None,
            )
            .await;
        let client = CodeExecutionClient::new(PlatformExtensionContext {
            extension_manager: Some(Arc::downgrade(&manager)),
            session_manager,
        })
        .unwrap();
        let host = Arc::new(FakeHost::default());
        *host.manager.lock().unwrap() = Some(Arc::clone(&manager));
        set_test_host(&session.id, Arc::clone(&host) as Arc<dyn WatchHost>);
        Harness {
            client,
            session_id: session.id,
            calls,
            host,
            _manager: manager,
            _dir: dir,
        }
    }

    pub(super) fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|text| text.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn watch_with(
        harness: &Harness,
        arguments: Value,
        token: CancellationToken,
    ) -> CallToolResult {
        harness
            .client
            .handle_watch_job(
                &harness.session_id,
                CallCapability::for_test_restricted(),
                arguments.as_object().cloned(),
                token,
            )
            .await
            .unwrap_or_else(|error| CallToolResult::error(vec![Content::text(error)]))
    }

    async fn watch(harness: &Harness, arguments: Value) -> CallToolResult {
        watch_with(harness, arguments, CancellationToken::new()).await
    }

    fn job_args(job_id: &str) -> Value {
        serde_json::json!({
            "tool": "fixture__status",
            "arguments": {"job_id": job_id},
            "note": "Plot the export.",
        })
    }

    pub(super) fn watch_id(text: &str) -> String {
        let start = text.find("watch-").expect("the result names the watch");
        text.get(start..)
            .unwrap_or_default()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect()
    }

    fn is_live(id: &str) -> bool {
        watches().contains_key(id)
    }

    /// Wait (real time) until the watch has ended and handed over.
    pub(super) async fn until_ended(id: &str) {
        for _ in 0..2000 {
            if !is_live(id) {
                // The wake runs right after the registry lets go.
                tokio::time::sleep(Duration::from_millis(100)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("watch {id} never ended");
    }

    #[tokio::test]
    async fn a_watch_returns_at_once_and_resumes_the_chat_once_when_the_job_ends() {
        let h = harness(vec![job("queued"), job("running"), job("completed")]).await;
        let result = watch(&h, job_args("j1")).await;
        let text = text_of(&result);
        assert_ne!(result.is_error, Some(true), "{text}");
        assert!(
            text.contains("End your turn; this chat resumes automatically when the job finishes."),
            "{text}"
        );
        assert!(
            text.contains("if Biorouter restarts it is dropped"),
            "{text}"
        );
        assert_eq!(h.calls.load(Ordering::SeqCst), 1, "one check in the turn");
        let id = watch_id(&text);
        let row = biorouter_mcp::active_work::active_work()
            .get(&id)
            .expect("the person sees the watch");
        assert_eq!(row.kind.as_str(), "job_watch");
        assert_eq!(row.session_id.as_deref(), Some(h.session_id.as_str()));
        assert!(row.cancellable);

        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        let wake = &actions[0];
        assert!(wake.starts_with("turn: [watch "), "{wake}");
        assert!(wake.contains("no longer active (3 checks"), "{wake}");
        assert!(wake.contains("\"completed\""), "{wake}");
        assert!(wake.contains("Your note: Plot the export."), "{wake}");
        assert_eq!(h.calls.load(Ordering::SeqCst), 3);
        assert!(biorouter_mcp::active_work::active_work().get(&id).is_none());

        // Nothing fires a second time.
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(h.host.actions().len(), 1);
        assert_eq!(h.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_job_already_done_sets_no_watch() {
        let h = harness(vec![job("completed")]).await;
        let text = text_of(&watch(&h, job_args("j2")).await);
        assert!(
            text.contains("set no watch: the job is already done"),
            "{text}"
        );
        assert!(!text.contains("watch-"), "{text}");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(h.host.actions().is_empty());
        assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_finished_job_steers_into_a_running_turn() {
        let h = harness(vec![job("running"), job("completed")]).await;
        h.host.turn_active.store(true, Ordering::SeqCst);
        let id = watch_id(&text_of(&watch(&h, job_args("j3")).await));
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("steer: [watch "), "{actions:?}");
    }

    #[tokio::test]
    async fn a_watch_times_out_with_a_message() {
        let h = harness(vec![job("running")]).await;
        let mut args = job_args("j4");
        args["max_wait_minutes"] = serde_json::json!(0.02); // 1.2 s → 2 test seconds
        let id = watch_id(&text_of(&watch(&h, args).await));
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("turn: "), "{actions:?}");
        assert!(actions[0].contains("still running"), "{actions:?}");
        assert!(actions[0].contains("watch_job again"), "{actions:?}");
    }

    #[tokio::test]
    async fn the_active_work_cancel_ends_the_watch_with_a_note_and_no_turn() {
        let h = harness(vec![job("running")]).await;
        let id = watch_id(&text_of(&watch(&h, job_args("j5")).await));
        assert!(biorouter_mcp::active_work::active_work().cancel(&id));
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("note: "), "{actions:?}");
        assert!(
            actions[0].contains("cancelled from the active work list"),
            "{actions:?}"
        );
        assert_eq!(
            h.calls.load(Ordering::SeqCst),
            1,
            "no check after the cancel"
        );
    }

    #[tokio::test]
    async fn stop_during_the_turn_that_set_the_watch_cancels_it() {
        let h = harness(vec![job("running")]).await;
        h.host.turn_active.store(true, Ordering::SeqCst);
        let turn = CancellationToken::new();
        let id = watch_id(&text_of(
            &watch_with(&h, job_args("j6"), turn.clone()).await,
        ));
        turn.cancel();
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("note: "), "{actions:?}");
        assert!(actions[0].contains("Stop ended the watch"), "{actions:?}");
    }

    #[tokio::test]
    async fn a_cancelled_token_after_the_turn_ended_is_not_stop() {
        let h = harness(vec![job("running"), job("running"), job("completed")]).await;
        let turn = CancellationToken::new();
        let id = watch_id(&text_of(
            &watch_with(&h, job_args("j7"), turn.clone()).await,
        ));
        // The turn has ended (no turn is active); a later cancel of its token
        // is cleanup.
        tokio::time::sleep(Duration::from_millis(50)).await;
        turn.cancel();
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("turn: "), "{actions:?}");
    }

    #[tokio::test]
    async fn the_model_cancels_its_own_watch_and_is_not_resumed() {
        let h = harness(vec![job("running")]).await;
        let id = watch_id(&text_of(&watch(&h, job_args("j8")).await));
        let text = text_of(&watch(&h, serde_json::json!({"cancel": id})).await);
        assert!(text.starts_with(&format!("Cancelled {id}.")), "{text}");
        until_ended(&id).await;
        assert!(h.host.actions().is_empty(), "{:?}", h.host.actions());
        // Another chat's watch is not this chat's to cancel.
        let other = harness(vec![job("running")]).await;
        let theirs = watch_id(&text_of(&watch(&other, job_args("j8b")).await));
        let refused = watch(&h, serde_json::json!({"cancel": theirs})).await;
        assert_eq!(refused.is_error, Some(true));
        assert!(is_live(&theirs));
        assert!(
            text_of(&watch(&other, serde_json::json!({"cancel": "all"})).await).contains(&theirs)
        );
    }

    #[tokio::test]
    async fn a_deleted_chat_ends_its_watch_silently() {
        let h = harness(vec![job("running")]).await;
        let id = watch_id(&text_of(&watch(&h, job_args("j9")).await));
        h.host.deleted.store(true, Ordering::SeqCst);
        until_ended(&id).await;
        assert!(h.host.actions().is_empty(), "{:?}", h.host.actions());
        assert_eq!(
            h.calls.load(Ordering::SeqCst),
            1,
            "no check of a deleted chat"
        );

        // And the delete route's hook ends one at once.
        let h = harness(vec![job("running")]).await;
        let id = watch_id(&text_of(&watch(&h, job_args("j10")).await));
        assert_eq!(cancel_watches_for_session(&h.session_id), 1);
        until_ended(&id).await;
        assert!(h.host.actions().is_empty(), "{:?}", h.host.actions());
    }

    #[tokio::test]
    async fn a_chat_paused_by_a_usage_limit_gets_a_note_and_no_turn() {
        let h = harness(vec![job("running"), job("completed")]).await;
        *h.host.limit.lock().unwrap() = Some("This chat reached its limit.".into());
        let id = watch_id(&text_of(&watch(&h, job_args("j11")).await));
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("note: [watch "), "{actions:?}");
        assert!(actions[0].contains("no longer active"), "{actions:?}");
        assert!(
            actions[0].contains("paused by a usage limit"),
            "{actions:?}"
        );
        assert!(
            actions[0].contains("This chat reached its limit."),
            "{actions:?}"
        );
    }

    #[tokio::test]
    async fn a_stopped_chat_ends_the_watch_with_a_note() {
        let h = harness(vec![job("running")]).await;
        let id = watch_id(&text_of(&watch(&h, job_args("j12")).await));
        h.host.agent_gone.store(true, Ordering::SeqCst);
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("note: "), "{actions:?}");
        assert!(actions[0].contains("this chat was stopped"), "{actions:?}");
    }

    #[tokio::test]
    async fn a_closing_turn_is_retried_and_unseen_approvals_get_a_note() {
        let h = harness(vec![job("running"), job("completed")]).await;
        h.host.busy_turns.store(2, Ordering::SeqCst);
        let id = watch_id(&text_of(&watch(&h, job_args("j13")).await));
        until_ended(&id).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("turn: "), "{actions:?}");

        let h = harness(vec![job("running"), job("completed")]).await;
        h.host.unseen_approvals.store(true, Ordering::SeqCst);
        let id = watch_id(&text_of(&watch(&h, job_args("j14")).await));
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("note: "), "{actions:?}");
        assert!(
            actions[0].contains("no Biorouter window is open"),
            "{actions:?}"
        );
    }

    #[tokio::test]
    async fn a_second_watch_of_the_same_job_is_the_first() {
        let h = harness(vec![job("running")]).await;
        let id = watch_id(&text_of(&watch(&h, job_args("j15")).await));
        let again = text_of(&watch(&h, job_args("j15")).await);
        assert!(again.starts_with("Already watching"), "{again}");
        assert_eq!(watch_id(&again), id);
        assert_eq!(cancel_watches_for_session(&h.session_id), 1);
        until_ended(&id).await;
    }

    #[tokio::test]
    async fn refusals_name_what_to_do_instead() {
        let h = harness(vec![job("running")]).await;
        h.host.no_daemon.store(true, Ordering::SeqCst);
        let text = text_of(&watch(&h, job_args("j16")).await);
        assert!(text.contains("code_execution__wait_for"), "{text}");
        h.host.no_daemon.store(false, Ordering::SeqCst);

        let text = text_of(&watch(&h, serde_json::json!({"tool": "other__status"})).await);
        assert!(text.contains("is not a tool this chat can call"), "{text}");
        let text = text_of(
            &watch(
                &h,
                serde_json::json!({"tool": "fixture__status", "note": "x".repeat(501)}),
            )
            .await,
        );
        assert!(text.contains("at most 500 characters"), "{text}");
        let text =
            text_of(&watch(&h, serde_json::json!({"tool": "code_execution__wait_for"})).await);
        assert!(
            text.contains("watch_job cannot wait on code_execution"),
            "{text}"
        );
        assert_eq!(h.calls.load(Ordering::SeqCst), 0, "nothing was checked");
    }

    fn object(value: Value) -> JsonObject {
        value.as_object().cloned().unwrap()
    }

    #[tokio::test]
    async fn a_resumed_turn_is_shown_in_the_open_tab() {
        let h = harness(vec![job("running"), job("completed")]).await;
        let id = watch_id(&text_of(&watch(&h, job_args("open-tab")).await));
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("turn: [watch "), "{actions:?}");
        // Once when the creating turn ended, once before the wake's turn
        // started and once after: the tab follows the chat before the turn
        // begins, so it sees the turn's start live.
        assert!(h.host.observes() >= 2, "observes: {}", h.host.observes());
    }

    #[tokio::test]
    async fn the_turn_that_sets_a_watch_ends_on_a_notice_for_the_person() {
        let h = harness(vec![job("running")]).await;
        h.host.turn_active.store(true, Ordering::SeqCst);
        let id = watch_id(&text_of(&watch(&h, job_args("notice")).await));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            h.host.notices().is_empty(),
            "nothing while the turn still runs"
        );
        assert_eq!(h.host.observes(), 0);

        h.host.turn_active.store(false, Ordering::SeqCst);
        for _ in 0..200 {
            if !h.host.notices().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let notices = h.host.notices();
        assert_eq!(notices, vec![watching_text("fixture__status", &id)]);
        assert!(
            notices[0].contains("will continue by itself"),
            "{notices:?}"
        );
        assert_eq!(h.host.observes(), 1, "the open tab is asked to follow");
        assert!(h.host.actions().is_empty(), "the job is still running");
        cancel_by_agent(&h.session_id, &id).unwrap();
        until_ended(&id).await;
    }

    #[test]
    fn the_notice_is_for_the_person_only() {
        let notice = watching_notice(&watching_text("cdw__status", "watch-1"));
        assert!(notice.is_user_visible());
        assert!(!notice.is_agent_visible());
    }

    #[tokio::test]
    async fn a_direct_read_of_the_job_s_end_retires_the_watch_without_a_wake() {
        let h = harness(vec![job("running")]).await;
        h.host.turn_active.store(true, Ordering::SeqCst);
        let id = watch_id(&text_of(&watch(&h, job_args("direct")).await));
        let args = object(serde_json::json!({"job_id": "direct"}));

        // A read that still shows the job running, of another job, or in
        // another chat, retires nothing.
        assert_eq!(
            retire_watches_seen_ending(
                &h.session_id,
                "fixture__status",
                Some(&args),
                &job("running")
            ),
            0
        );
        let other = object(serde_json::json!({"job_id": "else"}));
        assert_eq!(
            retire_watches_seen_ending(
                &h.session_id,
                "fixture__status",
                Some(&other),
                &job("completed")
            ),
            0
        );
        assert_eq!(
            retire_watches_seen_ending(
                "another-chat",
                "fixture__status",
                Some(&args),
                &job("completed")
            ),
            0
        );
        assert_eq!(
            retire_watches_seen_ending(
                &h.session_id,
                "fixture__status",
                Some(&args),
                &CallToolResult::error(vec![Content::text("completed")])
            ),
            0,
            "a failed lookup says nothing about the job"
        );
        assert!(is_live(&id));

        assert_eq!(
            retire_watches_seen_ending(
                &h.session_id,
                "fixture__status",
                Some(&args),
                &job("completed")
            ),
            1
        );
        h.host.turn_active.store(false, Ordering::SeqCst);
        until_ended(&id).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(h.host.actions().is_empty(), "{:?}", h.host.actions());
        assert!(
            h.host.notices().is_empty(),
            "a retired watch is not announced"
        );
        assert!(biorouter_mcp::active_work::active_work().get(&id).is_none());
    }

    #[tokio::test]
    async fn a_pointer_watch_retires_when_its_value_is_read_elsewhere() {
        let h = harness(vec![job("running")]).await;
        h.host.turn_active.store(true, Ordering::SeqCst);
        let mut args = job_args("pointer");
        args["until"] = serde_json::json!({"pointer": "/stage", "values": ["exported"]});
        let id = watch_id(&text_of(&watch(&h, args).await));
        let call = object(serde_json::json!({"job_id": "pointer"}));
        let read = |stage: &str| {
            CallToolResult::success(vec![Content::text(
                serde_json::json!({"job_id": "pointer", "status": "running", "stage": stage})
                    .to_string(),
            )])
        };
        assert_eq!(
            retire_watches_seen_ending(
                &h.session_id,
                "fixture__status",
                Some(&call),
                &read("plotting")
            ),
            0
        );
        assert_eq!(
            retire_watches_seen_ending(
                &h.session_id,
                "fixture__status",
                Some(&call),
                &read("EXPORTED")
            ),
            1
        );
        until_ended(&id).await;
        assert!(h.host.actions().is_empty(), "{:?}", h.host.actions());
    }

    #[tokio::test]
    async fn a_script_that_reads_the_end_itself_is_not_woken_again() {
        // Chat 9 of the live run: the model set a watch, ignored "end your
        // turn", read the job's end itself and answered; the watch then woke
        // the chat a second time for the same end.
        // No job id in the result, so the poll pacing does not hold the
        // script's own check (that hold is `poll_pacing`'s to test) and the
        // script reads the end before the watch's first background check.
        let bare = |status: &str| {
            CallToolResult::success(vec![Content::text(
                serde_json::json!({ "status": status }).to_string(),
            )])
        };
        let h = harness(vec![bare("running"), bare("completed")]).await;
        h.host.turn_active.store(true, Ordering::SeqCst);
        let result = h
            .client
            .handle_execute_code(
                &h.session_id,
                CallCapability::for_test_restricted(),
                serde_json::json!({ "code": r#"
                    import { status } from "fixture";
                    const w = watch_job({ tool: "fixture__status", arguments: { job_id: "s9" } });
                    const s = status({ job_id: "s9" });
                    record_result({ w, s });
                "# })
                .as_object()
                .cloned(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let text = text_of(&result);
        assert_ne!(result.is_error, Some(true), "{text}");
        assert!(text.contains("completed"), "{text}");
        let id = watch_id(&text);
        h.host.turn_active.store(false, Ordering::SeqCst);
        until_ended(&id).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(h.host.actions().is_empty(), "{:?}", h.host.actions());
        assert!(h.host.notices().is_empty(), "{:?}", h.host.notices());
    }

    #[tokio::test]
    async fn a_watch_s_own_checks_do_not_retire_it() {
        let h = harness(vec![job("running"), job("completed")]).await;
        let id = watch_id(&text_of(&watch(&h, job_args("own")).await));
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(
            actions.len(),
            1,
            "the watch still wakes the chat: {actions:?}"
        );
        assert!(actions[0].starts_with("turn: [watch "), "{actions:?}");
    }

    #[tokio::test]
    async fn a_wait_for_timeout_on_a_watched_call_does_not_offer_a_second_watch() {
        let h = harness(vec![job("running")]).await;
        h.host.turn_active.store(true, Ordering::SeqCst);
        let id = watch_id(&text_of(&watch(&h, job_args("timeout")).await));
        let result = h
            .client
            .handle_wait_for(
                &h.session_id,
                CallCapability::for_test_restricted(),
                serde_json::json!({
                    "tool": "fixture__status",
                    "arguments": {"job_id": "timeout"},
                    "timeout_seconds": 20,
                    "min_interval_seconds": 10,
                })
                .as_object()
                .cloned(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let text = text_of(&result);
        assert!(text.contains("timed out"), "{text}");
        assert!(
            text.contains(&format!("{id} already watches this job")),
            "{text}"
        );
        assert!(!text.contains("code_execution__watch_job"), "{text}");
        assert!(is_live(&id), "a running job's read retires nothing");
        cancel_by_agent(&h.session_id, &id).unwrap();
        until_ended(&id).await;
    }
}

/// A script's `watch_job()`: one script submits a job and watches it, so the
/// wait costs the model one request (the one that ends the turn).
#[cfg(test)]
mod script_watch_tests {
    use super::watch_tests::*;
    use super::*;

    #[tokio::test]
    async fn a_script_submits_and_watches_in_one_execution() {
        let h = harness(vec![job("running"), job("completed")]).await;
        let result = h
            .client
            .handle_execute_code(
                &h.session_id,
                CallCapability::for_test_restricted(),
                serde_json::json!({ "code": r#"
                    import { status } from "fixture";
                    const w = watch_job({ tool: "fixture__status", arguments: { job_id: "s1" },
                                          note: "Plot it." });
                    record_result(w);
                "# })
                .as_object()
                .cloned(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let text = text_of(&result);
        assert_ne!(result.is_error, Some(true), "{text}");
        assert!(text.contains("\"state\": \"watching\""), "{text}");
        let id = watch_id(&text);
        until_ended(&id).await;
        let actions = h.host.actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(actions[0].starts_with("turn: [watch "), "{actions:?}");
        assert!(actions[0].contains("Your note: Plot it."), "{actions:?}");
    }

    #[tokio::test]
    async fn a_script_s_watch_of_a_finished_job_returns_its_result() {
        let h = harness(vec![job("completed")]).await;
        let result = h
            .client
            .handle_execute_code(
                &h.session_id,
                CallCapability::for_test_restricted(),
                serde_json::json!({ "code": r#"
                    const w = watch_job({ tool: "fixture__status", arguments: { job_id: "s2" } });
                    record_result({ state: w.state, status: w.last_result.status });
                "# })
                .as_object()
                .cloned(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let text = text_of(&result);
        assert!(text.contains("\"state\": \"finished\""), "{text}");
        assert!(text.contains("\"status\": \"completed\""), "{text}");
        assert!(h.host.actions().is_empty());
    }

    #[tokio::test]
    async fn a_script_cannot_watch_a_tool_it_cannot_import() {
        let h = harness(vec![job("running")]).await;
        let result = h
            .client
            .handle_execute_code(
                &h.session_id,
                CallCapability::for_test_restricted(),
                serde_json::json!({ "code": r#"
                    record_result(watch_job({ tool: "other__status" }));
                "# })
                .as_object()
                .cloned(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let text = text_of(&result);
        assert!(
            text.contains("is not a tool this script can call"),
            "{text}"
        );
        assert_eq!(h.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
