//! The subagents of one chat, as every interface shows them.
//!
//! One function, [`session_subagents`], answers "which subagents has this chat
//! started, what is each one doing, and how is it set up". The daemon's
//! `GET /sessions/{id}/subagents` route, the desktop side panel (through that
//! route), `biorouter session subagents` and the terminal UI's subagent pane all
//! read it, so the four surfaces cannot disagree about a child's status, model or
//! setup.
//!
//! A row is assembled from three sources, in this order of authority:
//! 1. the live handle registry (`subagent_handle::list_for_session`), which knows
//!    whether a child is queued, running or being cancelled in THIS process;
//! 2. the child's durable outcome record ([`SUBAGENT_OUTCOME_STATE`] in its
//!    `extension_data`), written when the run ends, so a finished child keeps its
//!    status across a daemon restart;
//! 3. the child's session row (model, tokens, messages, privacy tier) and its
//!    spawn record ([`SUBAGENT_SPAWN_STATE`]: role, phase, skills, knowledge
//!    bases) plus the runtime profile's extension projection.
//!
//! A child with neither a live handle nor an outcome record was running when its
//! process went away: it reads as [`SubagentRunStatus::Interrupted`], never as
//! running, so a panel does not spin forever on a run nothing is executing.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::agents::orchestration_plan::OrchestrationSnapshot;
use crate::agents::subagent_handle;
use crate::agents::subagent_result::{SubagentResult, SubagentStatus};
use crate::privacy::SessionClassification;
use crate::session::session_manager::SubagentChildRow;
use crate::session::{
    EnabledExtensionsState, ExtensionData, ExtensionState, Session, SessionManager,
};

/// `extension_data` key (name, version) of a child's durable outcome record:
/// `{ "status": SubagentRunStatus, "ended_at": RFC3339, "result_line": string? }`.
pub const SUBAGENT_OUTCOME_STATE: &str = "subagent_outcome";
pub const SUBAGENT_OUTCOME_VERSION: &str = "v1";

/// `extension_data` key (name, version) of a child's structured spawn record:
/// `{ "role": string?, "phase_id": string?, "skills": [string], "knowledge_bases": [string] }`.
pub const SUBAGENT_SPAWN_STATE: &str = "subagent_spawn";
pub const SUBAGENT_SPAWN_VERSION: &str = "v1";

/// One child's run state. Ordered roughly by lifecycle; the order carries no
/// meaning for callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRunStatus {
    /// Registered, waiting for a concurrency slot or still initialising.
    Queued,
    /// Running a turn in this process.
    Running,
    /// A stop was requested and has not landed yet.
    Cancelling,
    /// Finished and reported a result.
    Completed,
    /// Stopped before acting to ask its parent a question (`BLOCKED:`).
    Blocked,
    /// Ended without finishing (a limit, a stop, an empty reply).
    Incomplete,
    /// Ended in an error.
    Failed,
    /// No live handle and no outcome record: its process went away mid-run.
    Interrupted,
    /// Not enough information to say (for example, an old child that predates
    /// outcome records and is not running here).
    Unknown,
}

impl SubagentRunStatus {
    /// Still doing (or about to do) work.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }
}

/// One subagent of a chat, as the side panel, the CLI and the TUI draw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRow {
    pub session_id: String,
    /// The session name without its `Subagent: ` prefix.
    pub name: String,
    /// Short role label the parent gave at spawn (`role` parameter), if any.
    pub role: Option<String>,
    /// Orchestration phase id the child was spawned into, if any.
    pub phase_id: Option<String>,
    pub status: SubagentRunStatus,
    /// The child's agent-loop phase while it runs in this process
    /// (`provider_open`, `tool_batch`, `approval_wait`, ...). `None` otherwise.
    pub loop_phase: Option<String>,
    /// Seconds since the child's last recorded activity, while it is active.
    /// Lets a surface show "quiet for 4m" without guessing.
    pub quiet_seconds: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Wall-clock run time: live elapsed while active, `ended_at - created_at`
    /// once finished.
    pub elapsed_seconds: Option<u64>,
    pub provider_name: Option<String>,
    pub model_name: Option<String>,
    /// Extension names the child may use (runtime-profile projection).
    pub extensions: Vec<String>,
    pub skills: Vec<String>,
    pub knowledge_bases: Vec<String>,
    pub privacy_tier: SessionClassification,
    /// Lifetime billed tokens (`accumulated_total_tokens`).
    pub total_tokens: Option<i64>,
    pub message_count: i64,
    pub tool_call_count: i64,
    /// First line of the final summary, the error, or the blocked question.
    pub result_line: Option<String>,
}

/// Everything a surface needs to draw a chat's subagents and its phase plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionSubagents {
    pub parent_session_id: String,
    /// Oldest first (spawn order).
    pub subagents: Vec<SubagentRow>,
    /// The chat's orchestration plan, when its agent declared one.
    pub plan: Option<OrchestrationSnapshot>,
}

/// The subagents of `parent_session_id`, oldest first, and its plan.
///
/// Does NOT apply privacy filtering: an HTTP caller must gate the parent with
/// `session_reach` and filter `subagents` through `HttpCaller::lists_session`.
/// In-process callers (the CLI, the TUI) share the child's tier by construction.
pub async fn session_subagents(
    session_manager: &SessionManager,
    parent_session_id: &str,
) -> anyhow::Result<SessionSubagents> {
    let children = session_manager
        .list_subagent_children(parent_session_id)
        .await?;

    // Live state, keyed by child. A child can own more than one handle (a
    // follow-up after its first handle was pruned registers a replacement,
    // `ensure_direct_followup_handle`); the registry is oldest first, so the
    // last one inserted is the current one.
    let mut live: HashMap<String, LiveState> = HashMap::new();
    for handle in subagent_handle::list_for_session(parent_session_id) {
        live.insert(handle.child_session_id.clone(), LiveState::of(&handle));
    }
    let services = crate::workspace_services::get();
    let now = Utc::now();

    let mut rows: Vec<SubagentRow> = children
        .into_iter()
        .map(|child| {
            let turn_active = services
                .as_ref()
                .is_some_and(|services| services.is_turn_active(&child.session.id));
            let live = live.get(&child.session.id);
            derive_row(child, live, turn_active, now)
        })
        .collect();

    // The loop phase is read off the live `Agent`, which only the daemon's agent
    // registry holds. Asked only when there is a daemon (services installed,
    // so the registry already exists and `instance()` is a lookup) and only for
    // rows that are doing something, so an idle chat's panel costs nothing.
    if services.is_some() && rows.iter().any(|row| row.status.is_active()) {
        if let Ok(manager) = crate::execution::manager::AgentManager::instance().await {
            for row in rows.iter_mut().filter(|row| row.status.is_active()) {
                if let Some(agent) = manager.peek_agent(&row.session_id).await {
                    let (phase, _age_ms) = agent.loop_phase_snapshot();
                    row.loop_phase = (phase != crate::agents::loop_phase::LoopPhase::Idle)
                        .then(|| phase.as_str().to_string());
                }
            }
        }
    }

    let plan =
        crate::agents::orchestration_plan::snapshot(session_manager, parent_session_id, &rows)
            .await;
    Ok(SessionSubagents {
        parent_session_id: parent_session_id.to_string(),
        subagents: rows,
        plan,
    })
}

// --- Durable records -------------------------------------------------------

/// The longest [`SubagentRow::result_line`] kept, in characters.
pub const MAX_RESULT_LINE_CHARS: usize = 200;

/// The child's structured spawn record, under [`SUBAGENT_SPAWN_STATE`].
///
/// Written in two halves, each a per-key transaction
/// (`SessionManager::update_extension_state`) that keeps the other half: the
/// role and phase at birth ([`record_spawn_identity`], so a queued child already
/// shows them), and the granted skills and knowledge bases once the child's
/// runtime is assembled ([`record_spawn_grants`]). The runtime-profile install
/// removes only its own keys, so it never touches this one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSpawnRecord {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub phase_id: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub knowledge_bases: Vec<String>,
}

/// The child's durable outcome record, under [`SUBAGENT_OUTCOME_STATE`],
/// written when its delegated run ends ([`record_outcome`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentOutcomeRecord {
    pub status: SubagentRunStatus,
    pub ended_at: DateTime<Utc>,
    #[serde(default)]
    pub result_line: Option<String>,
}

/// The spawn record in `extension_data`, if one was written and parses.
pub fn read_spawn_record(extension_data: &ExtensionData) -> Option<SubagentSpawnRecord> {
    extension_data
        .get_extension_state(SUBAGENT_SPAWN_STATE, SUBAGENT_SPAWN_VERSION)
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// The name a person-facing sentence gives a subagent: the `role` it was
/// spawned with when it has one, else `fallback` (its handle's title, the task
/// on one line). C-2 (live QA 2026-11-10): a monitor's stop named a subagent
/// by its task ("Use the Developer shell tool to write a short synthetic note
/// to…") while the side panel, its tab and History all call it "mon-1".
pub fn display_name(extension_data: &ExtensionData, fallback: &str) -> String {
    read_spawn_record(extension_data)
        .and_then(|record| record.role)
        .map(|role| role.trim().to_string())
        .filter(|role| !role.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// [`display_name`] for a stored child, read from its session; `fallback`
/// when it cannot be read.
pub async fn display_name_of(
    session_manager: &SessionManager,
    child_session_id: &str,
    fallback: &str,
) -> String {
    match session_manager.get_session(child_session_id, false).await {
        Ok(session) => display_name(&session.extension_data, fallback),
        Err(_) => fallback.to_string(),
    }
}

/// The outcome record in `extension_data`, if one was written and parses.
pub fn read_outcome_record(extension_data: &ExtensionData) -> Option<SubagentOutcomeRecord> {
    extension_data
        .get_extension_state(SUBAGENT_OUTCOME_STATE, SUBAGENT_OUTCOME_VERSION)
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Merge into the spawn record, keeping whatever the closure does not set.
async fn update_spawn_record(
    session_manager: &SessionManager,
    child_session_id: &str,
    apply: impl FnOnce(&mut SubagentSpawnRecord) + Send,
) -> anyhow::Result<()> {
    let written = session_manager
        .update_extension_state(
            child_session_id,
            SUBAGENT_SPAWN_STATE,
            SUBAGENT_SPAWN_VERSION,
            move |current| {
                let mut record: SubagentSpawnRecord = current
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
                    .unwrap_or_default();
                apply(&mut record);
                Ok(serde_json::to_value(record)?)
            },
        )
        .await?;
    if written.is_none() {
        anyhow::bail!("cannot record the subagent spawn: no session {child_session_id}");
    }
    Ok(())
}

/// Record the role and phase a child was spawned with. Called at birth, for
/// every child, so the record's presence also says "this child was created by a
/// build that writes outcome records" (see [`SubagentRunStatus::Interrupted`]).
pub async fn record_spawn_identity(
    session_manager: &SessionManager,
    child_session_id: &str,
    role: Option<&str>,
    phase_id: Option<&str>,
) -> anyhow::Result<()> {
    let role = role.map(str::to_string);
    let phase_id = phase_id.map(str::to_string);
    update_spawn_record(session_manager, child_session_id, move |record| {
        record.role = role;
        record.phase_id = phase_id;
    })
    .await
}

/// Record the skills and knowledge bases the child was granted: the same names
/// its spawn-context message lists.
pub async fn record_spawn_grants(
    session_manager: &SessionManager,
    child_session_id: &str,
    skills: &[String],
    knowledge_bases: &[String],
) -> anyhow::Result<()> {
    let skills = skills.to_vec();
    let knowledge_bases = knowledge_bases.to_vec();
    update_spawn_record(session_manager, child_session_id, move |record| {
        record.skills = skills;
        record.knowledge_bases = knowledge_bases;
    })
    .await
}

/// The run status a finished [`SubagentResult`] reads as. A cancelled run is
/// already `Incomplete` by the time it gets here (`SubagentResult::mark_cancelled`).
pub fn outcome_status(result: &SubagentResult) -> SubagentRunStatus {
    match result.status {
        SubagentStatus::Completed => SubagentRunStatus::Completed,
        SubagentStatus::Blocked => SubagentRunStatus::Blocked,
        SubagentStatus::Incomplete => SubagentRunStatus::Incomplete,
        SubagentStatus::Error => SubagentRunStatus::Failed,
    }
}

/// The first non-empty line of what a surface should show for a finished run:
/// the blocked question, the error, or the summary, in that order of
/// preference for the status that carries each. Control characters dropped
/// (it is printed to terminals), cut at [`MAX_RESULT_LINE_CHARS`].
pub fn result_line(result: &SubagentResult) -> Option<String> {
    let preferred = match result.status {
        SubagentStatus::Blocked => result.question.as_deref(),
        SubagentStatus::Error => result.error.as_deref(),
        SubagentStatus::Completed | SubagentStatus::Incomplete => None,
    };
    preferred
        .and_then(first_line)
        .or_else(|| first_line(&result.summary))
}

fn first_line(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(|line| {
            line.chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .find(|line| !line.is_empty())?;
    if line.chars().count() <= MAX_RESULT_LINE_CHARS {
        return Some(line);
    }
    let mut cut: String = line.chars().take(MAX_RESULT_LINE_CHARS - 1).collect();
    cut.push('…');
    Some(cut)
}

/// Write the child's outcome record. Only ever the CHILD's row: a parent's
/// `updated_at` is its own activity (R-30), and a child finishing is not one of
/// the parent's activities.
pub async fn record_outcome(
    session_manager: &SessionManager,
    child_session_id: &str,
    result: &SubagentResult,
) -> anyhow::Result<()> {
    record_outcome_at(session_manager, child_session_id, result, Utc::now()).await
}

async fn record_outcome_at(
    session_manager: &SessionManager,
    child_session_id: &str,
    result: &SubagentResult,
    ended_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let record = SubagentOutcomeRecord {
        status: outcome_status(result),
        ended_at,
        result_line: result_line(result),
    };
    let value = serde_json::to_value(record)?;
    let written = session_manager
        .update_extension_state(
            child_session_id,
            SUBAGENT_OUTCOME_STATE,
            SUBAGENT_OUTCOME_VERSION,
            move |_| Ok(value),
        )
        .await?;
    if written.is_none() {
        anyhow::bail!("cannot record the subagent outcome: no session {child_session_id}");
    }
    Ok(())
}

// --- Row derivation --------------------------------------------------------

/// What the live handle registry knows about one child, copied out of the
/// handle so [`derive_row`] is a pure function of plain data.
#[derive(Debug, Clone)]
struct LiveState {
    /// The delegated initial run has not finished.
    running: bool,
    /// A stop was requested.
    cancelling: bool,
    /// Registered, but its runtime is not assembled yet (queued for a
    /// concurrency slot, or still initialising).
    initializing: bool,
    /// The latest finished turn's result: a later turn's when the person
    /// continued the child, else the delegated run's.
    result: Option<SubagentResult>,
    /// A turn after the delegated run has begun (the retained result is history).
    later_turn: bool,
    elapsed_seconds: u64,
}

impl LiveState {
    fn of(handle: &subagent_handle::BackgroundSubagent) -> Self {
        let result = handle
            .terminal_generation()
            .map(|terminal| terminal.result)
            .or_else(|| handle.result());
        Self {
            running: handle.is_running(),
            cancelling: handle.is_cancelled(),
            initializing: handle.is_initializing(),
            result,
            later_turn: !handle.result_is_current(),
            elapsed_seconds: handle.elapsed().as_secs(),
        }
    }
}

fn seconds_between(from: DateTime<Utc>, to: DateTime<Utc>) -> u64 {
    u64::try_from((to - from).num_seconds()).unwrap_or(0)
}

/// One row from a child's stored session, its live state (if this process runs
/// it) and whether the daemon holds a turn on it. Status follows the module
/// doc's order of authority: live handle, then outcome record, then
/// interrupted (a record-writing child with neither) or unknown (an older one).
fn derive_row(
    child: SubagentChildRow,
    live: Option<&LiveState>,
    turn_active: bool,
    now: DateTime<Utc>,
) -> SubagentRow {
    let SubagentChildRow {
        session,
        tool_call_count,
    } = child;
    let spawn = read_spawn_record(&session.extension_data);
    let outcome = read_outcome_record(&session.extension_data);

    let (status, result_line) = match live {
        Some(live) if live.running => {
            let status = if live.cancelling {
                SubagentRunStatus::Cancelling
            } else if live.initializing {
                SubagentRunStatus::Queued
            } else {
                SubagentRunStatus::Running
            };
            (status, None)
        }
        // The delegated run finished but the person (or a Stop-and-Send) has
        // the child working on a later turn.
        Some(_) if turn_active => (SubagentRunStatus::Running, None),
        Some(live) => match &live.result {
            Some(result) => (outcome_status(result), self::result_line(result)),
            None => outcome
                .as_ref()
                .map_or((SubagentRunStatus::Unknown, None), |o| {
                    (o.status, o.result_line.clone())
                }),
        },
        None if turn_active => (SubagentRunStatus::Running, None),
        None => match &outcome {
            Some(outcome) => (outcome.status, outcome.result_line.clone()),
            None if spawn.is_some() => (SubagentRunStatus::Interrupted, None),
            None => (SubagentRunStatus::Unknown, None),
        },
    };

    let active = status.is_active();
    let ended_at = if active {
        None
    } else {
        let recorded = outcome.as_ref().map(|outcome| outcome.ended_at);
        match live {
            // A later turn ended after the record was written: the row's last
            // activity is the closer answer.
            Some(live) if live.later_turn && live.result.is_some() => {
                Some(recorded.map_or(session.updated_at, |r| r.max(session.updated_at)))
            }
            _ => recorded,
        }
    };
    let elapsed_seconds = if active {
        Some(live.map_or_else(
            || seconds_between(session.created_at, now),
            |live| live.elapsed_seconds,
        ))
    } else {
        ended_at.map(|ended| seconds_between(session.created_at, ended))
    };
    let quiet_seconds = active.then(|| seconds_between(session.updated_at, now));

    let extensions = subagent_extension_names(&session);
    let spawn = spawn.unwrap_or_default();
    let name = session
        .name
        .strip_prefix(SUBAGENT_NAME_PREFIX)
        .unwrap_or(&session.name)
        .to_string();

    SubagentRow {
        session_id: session.id,
        name,
        role: spawn.role,
        phase_id: spawn.phase_id,
        status,
        loop_phase: None,
        quiet_seconds,
        created_at: session.created_at,
        updated_at: session.updated_at,
        ended_at,
        elapsed_seconds,
        provider_name: session.provider_name,
        model_name: session.model_config.map(|config| config.model_name),
        extensions,
        skills: spawn.skills,
        knowledge_bases: spawn.knowledge_bases,
        privacy_tier: session.privacy_tier,
        total_tokens: session.accumulated_total_tokens,
        message_count: i64::try_from(session.message_count).unwrap_or(i64::MAX),
        tool_call_count,
        result_line,
    }
}

/// The prefix `subagent_session_label` puts on every child's name.
const SUBAGENT_NAME_PREFIX: &str = "Subagent: ";

/// The extension names a child may use, from its runtime profile, falling back
/// to a pre-profile child's recorded roster. Never the machine's global
/// defaults: a child that recorded nothing is shown with nothing rather than
/// with extensions it may not hold.
fn subagent_extension_names(session: &Session) -> Vec<String> {
    let configs =
        match crate::agents::persisted_subagent_extension_projection(&session.extension_data) {
            Ok(Some(configs)) => configs,
            Ok(None) | Err(_) => {
                EnabledExtensionsState::from_extension_data(&session.extension_data)
                    .map(|state| state.extensions)
                    .unwrap_or_default()
            }
        };
    configs.iter().map(|config| config.name()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::subagent_runtime_profile::SubagentRuntimeProfile;
    use crate::agents::ExtensionConfig;
    use crate::conversation::message::Message;
    use crate::session::session_manager::SessionType;
    use chrono::Duration;
    use tokio_util::sync::CancellationToken;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_800_000_000 + seconds, 0).unwrap()
    }

    fn child_row(extension_data: ExtensionData) -> SubagentChildRow {
        SubagentChildRow {
            session: Session {
                id: "child".to_string(),
                name: "Subagent: Read the logs".to_string(),
                session_type: SessionType::SubAgent,
                created_at: at(0),
                updated_at: at(30),
                extension_data,
                parent_session_id: Some("parent".to_string()),
                ..Default::default()
            },
            tool_call_count: 3,
        }
    }

    fn with_spawn(record: &SubagentSpawnRecord) -> ExtensionData {
        let mut data = ExtensionData::new();
        data.set_extension_state(
            SUBAGENT_SPAWN_STATE,
            SUBAGENT_SPAWN_VERSION,
            serde_json::to_value(record).unwrap(),
        );
        data
    }

    fn with_outcome(mut data: ExtensionData, record: &SubagentOutcomeRecord) -> ExtensionData {
        data.set_extension_state(
            SUBAGENT_OUTCOME_STATE,
            SUBAGENT_OUTCOME_VERSION,
            serde_json::to_value(record).unwrap(),
        );
        data
    }

    fn live(running: bool) -> LiveState {
        LiveState {
            running,
            cancelling: false,
            initializing: false,
            result: None,
            later_turn: false,
            elapsed_seconds: 42,
        }
    }

    fn completed(summary: &str) -> SubagentResult {
        SubagentResult {
            status: SubagentStatus::Completed,
            summary: summary.to_string(),
            error: None,
            question: None,
            artifacts: Vec::new(),
            tokens: None,
            human_intervened: false,
            stopped_by_monitor: None,
            decisions: None,
        }
    }

    /// The live handle outranks everything: a child this process runs is
    /// queued, running or cancelling whatever its row says.
    #[test]
    fn a_live_handle_decides_an_active_status() {
        let now = at(100);
        let row = derive_row(
            child_row(ExtensionData::new()),
            Some(&live(true)),
            false,
            now,
        );
        assert_eq!(row.status, SubagentRunStatus::Running);
        assert_eq!(row.elapsed_seconds, Some(42), "live elapsed while active");
        assert_eq!(row.quiet_seconds, Some(70), "now minus the last activity");
        assert_eq!(row.ended_at, None);

        let queued = LiveState {
            initializing: true,
            ..live(true)
        };
        let row = derive_row(child_row(ExtensionData::new()), Some(&queued), false, now);
        assert_eq!(row.status, SubagentRunStatus::Queued);

        let cancelling = LiveState {
            cancelling: true,
            initializing: true,
            ..live(true)
        };
        let row = derive_row(
            child_row(ExtensionData::new()),
            Some(&cancelling),
            false,
            now,
        );
        assert_eq!(row.status, SubagentRunStatus::Cancelling);

        // Even a stale outcome record from an earlier run does not override it.
        let stale = with_outcome(
            ExtensionData::new(),
            &SubagentOutcomeRecord {
                status: SubagentRunStatus::Completed,
                ended_at: at(10),
                result_line: Some("old".into()),
            },
        );
        let row = derive_row(child_row(stale), Some(&live(true)), false, now);
        assert_eq!(row.status, SubagentRunStatus::Running);
        assert_eq!(row.result_line, None);
    }

    /// A finished handle reports its result's status and first line, unless a
    /// later turn is running on the child right now.
    #[test]
    fn a_finished_handle_reports_its_result_unless_a_later_turn_runs() {
        let now = at(100);
        let done = LiveState {
            result: Some(completed("\n\nParsed 12 files.\nDetails follow.")),
            ..live(false)
        };
        let outcome = with_outcome(
            with_spawn(&SubagentSpawnRecord::default()),
            &SubagentOutcomeRecord {
                status: SubagentRunStatus::Completed,
                ended_at: at(50),
                result_line: Some("Parsed 12 files.".into()),
            },
        );
        let row = derive_row(child_row(outcome.clone()), Some(&done), false, now);
        assert_eq!(row.status, SubagentRunStatus::Completed);
        assert_eq!(row.result_line.as_deref(), Some("Parsed 12 files."));
        assert_eq!(row.ended_at, Some(at(50)));
        assert_eq!(row.elapsed_seconds, Some(50), "ended_at minus created_at");
        assert_eq!(row.quiet_seconds, None, "quiet is only for active rows");

        let row = derive_row(child_row(outcome), Some(&done), true, now);
        assert_eq!(row.status, SubagentRunStatus::Running);
        assert_eq!(row.result_line, None);
    }

    /// With no live handle (another process, or after a restart): a turn the
    /// daemon holds, then the outcome record, then interrupted for a child that
    /// has a spawn record, then unknown for one that predates both records.
    #[test]
    fn without_a_handle_the_records_decide() {
        let now = at(100);
        let outcome = with_outcome(
            with_spawn(&SubagentSpawnRecord::default()),
            &SubagentOutcomeRecord {
                status: SubagentRunStatus::Blocked,
                ended_at: at(20),
                result_line: Some("Which config file?".into()),
            },
        );
        let row = derive_row(child_row(outcome.clone()), None, false, now);
        assert_eq!(row.status, SubagentRunStatus::Blocked);
        assert_eq!(row.result_line.as_deref(), Some("Which config file?"));
        assert_eq!(row.ended_at, Some(at(20)));
        assert_eq!(row.elapsed_seconds, Some(20));

        let row = derive_row(child_row(outcome), None, true, now);
        assert_eq!(row.status, SubagentRunStatus::Running);
        assert_eq!(row.elapsed_seconds, Some(100), "now minus created_at");

        let spawned_only = with_spawn(&SubagentSpawnRecord::default());
        let row = derive_row(child_row(spawned_only), None, false, now);
        assert_eq!(row.status, SubagentRunStatus::Interrupted);
        assert_eq!(row.ended_at, None);
        assert_eq!(row.elapsed_seconds, None);
        assert_eq!(row.quiet_seconds, None, "interrupted is not active");

        let legacy = derive_row(child_row(ExtensionData::new()), None, false, now);
        assert_eq!(legacy.status, SubagentRunStatus::Unknown);
    }

    /// Every field a surface draws comes from the place the module doc names.
    #[test]
    fn a_row_maps_every_field_from_its_source() {
        let mut data = with_spawn(&SubagentSpawnRecord {
            role: Some("reviewer".into()),
            phase_id: Some("review".into()),
            skills: vec!["code-review".into()],
            knowledge_bases: vec!["soul".into(), "lab-notes".into()],
        });
        let developer = ExtensionConfig::Builtin {
            name: "developer".into(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        };
        let profile = SubagentRuntimeProfile::new(
            "prompt".into(),
            None,
            Vec::new(),
            &[developer],
            &["developer__shell".to_string()],
        )
        .unwrap();
        data.set_extension_state(
            "subagent_runtime_profile",
            "v2",
            serde_json::to_value(&profile).unwrap(),
        );
        let mut child = child_row(data);
        child.session.provider_name = Some("anthropic".into());
        child.session.model_config = Some(crate::model::ModelConfig::new_or_fail("claude-x"));
        child.session.accumulated_total_tokens = Some(1234);
        child.session.message_count = 7;
        child.session.privacy_tier = SessionClassification::Private;

        let row = derive_row(child, None, false, at(100));
        assert_eq!(row.session_id, "child");
        assert_eq!(row.name, "Read the logs", "the Subagent: prefix is dropped");
        assert_eq!(row.role.as_deref(), Some("reviewer"));
        assert_eq!(row.phase_id.as_deref(), Some("review"));
        assert_eq!(row.skills, vec!["code-review".to_string()]);
        assert_eq!(
            row.knowledge_bases,
            vec!["soul".to_string(), "lab-notes".to_string()]
        );
        assert_eq!(row.extensions, vec!["developer".to_string()]);
        assert_eq!(row.provider_name.as_deref(), Some("anthropic"));
        assert_eq!(row.model_name.as_deref(), Some("claude-x"));
        assert_eq!(row.total_tokens, Some(1234));
        assert_eq!(row.message_count, 7);
        assert_eq!(row.tool_call_count, 3);
        assert_eq!(row.privacy_tier, SessionClassification::Private);
        assert_eq!(row.created_at, at(0));
        assert_eq!(row.updated_at, at(30));
        assert_eq!(row.loop_phase, None);
    }

    /// A child with no profile and no recorded roster shows no extensions,
    /// never the machine's global defaults.
    #[test]
    fn a_child_with_no_recorded_extensions_shows_none() {
        let row = derive_row(child_row(ExtensionData::new()), None, false, at(1));
        assert!(row.extensions.is_empty());
        // A name without the prefix is kept as it is.
        let mut child = child_row(ExtensionData::new());
        child.session.name = "Subagent task".into();
        assert_eq!(derive_row(child, None, false, at(1)).name, "Subagent task");
    }

    #[test]
    fn outcome_status_and_result_line_follow_the_result() {
        assert_eq!(
            outcome_status(&completed("ok")),
            SubagentRunStatus::Completed
        );
        let failed = SubagentResult::from_error("provider refused\nstack trace");
        assert_eq!(outcome_status(&failed), SubagentRunStatus::Failed);
        assert_eq!(result_line(&failed).as_deref(), Some("provider refused"));

        let mut cancelled = completed("half done");
        cancelled.mark_cancelled();
        assert_eq!(outcome_status(&cancelled), SubagentRunStatus::Incomplete);
        assert_eq!(
            result_line(&cancelled).as_deref(),
            Some("Subagent was cancelled before completion. Best-effort recap from its work:")
        );

        let blocked = SubagentResult {
            status: SubagentStatus::Blocked,
            summary: "BLOCKED: which file?\ncandidates: a, b".into(),
            question: Some("which file?".into()),
            ..completed("")
        };
        assert_eq!(outcome_status(&blocked), SubagentRunStatus::Blocked);
        assert_eq!(result_line(&blocked).as_deref(), Some("which file?"));

        let long = completed(&"x".repeat(500));
        let line = result_line(&long).unwrap();
        assert_eq!(line.chars().count(), MAX_RESULT_LINE_CHARS);
        assert!(line.ends_with('…'));

        let hostile = completed("\u{1b}[2Jcleared\r");
        assert_eq!(result_line(&hostile).as_deref(), Some("[2Jcleared"));
        assert_eq!(result_line(&completed("  \n ")), None);
    }

    async fn store() -> (tempfile::TempDir, SessionManager) {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        (temp, sm)
    }

    async fn spawn_child(sm: &SessionManager, parent: &str, name: &str) -> String {
        let child = sm
            .create_session(
                std::env::temp_dir(),
                format!("Subagent: {name}"),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        sm.update(&child.id)
            .parent_session_id(Some(parent.to_string()))
            .apply()
            .await
            .unwrap();
        child.id
    }

    fn tool_call(name: &str) -> rmcp::model::CallToolRequestParams {
        rmcp::model::CallToolRequestParams {
            task: None,
            name: name.to_string().into(),
            arguments: None,
            meta: None,
        }
    }

    /// C-2: a subagent with a role is named by it; one without keeps the
    /// caller's fallback, and so does a blank role or a missing session.
    #[tokio::test]
    async fn a_subagent_is_named_by_its_role_when_it_has_one() {
        let (_temp, sm) = store().await;
        let child = spawn_child(&sm, "parent-names", "names").await;
        assert_eq!(
            display_name_of(&sm, &child, "Write a note").await,
            "Write a note"
        );
        record_spawn_identity(&sm, &child, Some(" mon-1 "), None)
            .await
            .unwrap();
        assert_eq!(display_name_of(&sm, &child, "Write a note").await, "mon-1");
        record_spawn_identity(&sm, &child, Some("  "), None)
            .await
            .unwrap();
        assert_eq!(
            display_name_of(&sm, &child, "Write a note").await,
            "Write a note"
        );
        assert_eq!(display_name_of(&sm, "no-such-child", "task").await, "task");
    }

    /// The spawn record's two halves merge, and the runtime-profile install
    /// that runs between them (removing its own keys and the legacy roster in
    /// one transaction) leaves it intact.
    #[tokio::test]
    async fn the_spawn_record_survives_the_runtime_profile_install() {
        let (_temp, sm) = store().await;
        let child = spawn_child(&sm, "parent-survive", "survive").await;

        record_spawn_identity(&sm, &child, Some("loader"), Some("execution"))
            .await
            .unwrap();
        SubagentRuntimeProfile::new("child prompt".into(), None, Vec::new(), &[], &[])
            .unwrap()
            .persist(&sm, &child)
            .await
            .unwrap();
        record_spawn_grants(&sm, &child, &["r-scripting".into()], &["soul".into()])
            .await
            .unwrap();

        let session = sm.get_session(&child, false).await.unwrap();
        let record = read_spawn_record(&session.extension_data).unwrap();
        assert_eq!(
            record,
            SubagentSpawnRecord {
                role: Some("loader".into()),
                phase_id: Some("execution".into()),
                skills: vec!["r-scripting".into()],
                knowledge_bases: vec!["soul".into()],
            }
        );
        // …and the profile is still there beside it.
        assert!(session
            .extension_data
            .get_extension_state("subagent_runtime_profile", "v2")
            .is_some());

        // A re-install (a cold restore persists again) keeps it too.
        SubagentRuntimeProfile::new("child prompt".into(), None, Vec::new(), &[], &[])
            .unwrap()
            .persist(&sm, &child)
            .await
            .unwrap();
        let session = sm.get_session(&child, false).await.unwrap();
        assert_eq!(read_spawn_record(&session.extension_data), Some(record));
    }

    /// The outcome lands on the CHILD's row and nowhere else.
    #[tokio::test]
    async fn the_outcome_record_is_written_to_the_child_only() {
        let (_temp, sm) = store().await;
        let parent = sm
            .create_session(std::env::temp_dir(), "parent".into(), SessionType::User)
            .await
            .unwrap();
        let child = spawn_child(&sm, &parent.id, "only the child").await;
        let before = sm.get_session(&parent.id, false).await.unwrap();

        record_outcome_at(&sm, &child, &completed("All good.\nmore"), at(5))
            .await
            .unwrap();

        let child_row = sm.get_session(&child, false).await.unwrap();
        assert_eq!(
            read_outcome_record(&child_row.extension_data),
            Some(SubagentOutcomeRecord {
                status: SubagentRunStatus::Completed,
                ended_at: at(5),
                result_line: Some("All good.".into()),
            })
        );
        let after = sm.get_session(&parent.id, false).await.unwrap();
        assert_eq!(read_outcome_record(&after.extension_data), None);
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(
            serde_json::to_value(&after.extension_data).unwrap(),
            serde_json::to_value(&before.extension_data).unwrap()
        );

        // A missing child is an error the caller logs, not a silent no-op.
        assert!(record_outcome(&sm, "no-such-child", &completed("x"))
            .await
            .is_err());
    }

    /// The whole roster from a real store: only this parent's `sub_agent`
    /// children, oldest first, with message and tool-call counts from one
    /// grouped query, and statuses merged from the live registry.
    #[tokio::test]
    #[serial_test::serial(workspace_services)]
    async fn session_subagents_lists_the_children_with_their_counts_and_status() {
        // "There is no daemon": no turn is active, and the loop-phase lookup is
        // skipped, whatever another test installed before this one.
        crate::workspace_services::set_for_tests(None);
        let (_temp, sm) = store().await;
        let parent = sm
            .create_session(std::env::temp_dir(), "parent".into(), SessionType::User)
            .await
            .unwrap();
        let first = spawn_child(&sm, &parent.id, "first").await;
        let second = spawn_child(&sm, &parent.id, "second").await;
        let third = spawn_child(&sm, &parent.id, "third").await;
        // Not this parent's: another chat's child, and a user chat that names
        // the parent (a diverged copy carries the column, a sub_agent it is not).
        spawn_child(&sm, "someone-else", "foreign").await;
        let user_row = sm
            .create_session(std::env::temp_dir(), "user".into(), SessionType::User)
            .await
            .unwrap();
        sm.update(&user_row.id)
            .parent_session_id(Some(parent.id.clone()))
            .apply()
            .await
            .unwrap();

        // first: two tool calls in one message, one in another, one frontend
        // call, and plain text that must not count.
        sm.add_message(
            &first,
            &Message::assistant()
                .with_text("working")
                .with_tool_request("a", Ok(tool_call("developer__shell")))
                .with_tool_request("b", Ok(tool_call("developer__text_editor"))),
        )
        .await
        .unwrap();
        sm.add_message(
            &first,
            &Message::assistant().with_tool_request("c", Ok(tool_call("todo__add"))),
        )
        .await
        .unwrap();
        sm.add_message(
            &first,
            &Message::assistant().with_frontend_tool_request("d", Ok(tool_call("frontend__pick"))),
        )
        .await
        .unwrap();
        sm.add_message(
            &first,
            &Message::user().with_text("toolRequest is just a word here"),
        )
        .await
        .unwrap();
        // second: a finished run with an outcome record and no handle.
        record_spawn_identity(&sm, &second, Some("writer"), None)
            .await
            .unwrap();
        record_outcome_at(&sm, &second, &completed("Wrote the report."), Utc::now())
            .await
            .unwrap();
        // third: live, still queued for a concurrency slot.
        let _queued = subagent_handle::BackgroundSubagent::register_initializing(
            parent.id.clone(),
            third.clone(),
            "third",
            CancellationToken::new(),
        );

        let roster = session_subagents(&sm, &parent.id).await.unwrap();
        assert_eq!(roster.parent_session_id, parent.id);
        let ids: Vec<&str> = roster
            .subagents
            .iter()
            .map(|row| row.session_id.as_str())
            .collect();
        assert_eq!(ids, vec![first.as_str(), second.as_str(), third.as_str()]);

        let first_row = &roster.subagents[0];
        assert_eq!(first_row.name, "first");
        assert_eq!(first_row.message_count, 4);
        assert_eq!(first_row.tool_call_count, 4);
        assert_eq!(first_row.status, SubagentRunStatus::Unknown);

        let second_row = &roster.subagents[1];
        assert_eq!(second_row.tool_call_count, 0);
        assert_eq!(second_row.status, SubagentRunStatus::Completed);
        assert_eq!(second_row.role.as_deref(), Some("writer"));
        assert_eq!(second_row.result_line.as_deref(), Some("Wrote the report."));

        let third_row = &roster.subagents[2];
        assert_eq!(third_row.status, SubagentRunStatus::Queued);
        assert!(third_row.quiet_seconds.is_some());

        // No plan declared, so no plan snapshot.
        assert_eq!(roster.plan, None);

        // A parent with no children is an empty roster, not an error.
        let empty = session_subagents(&sm, "nobody").await.unwrap();
        assert!(empty.subagents.is_empty());
        crate::workspace_services::clear_test_override();
    }

    /// A child whose delegated run ended but whose handle is gone (a restart)
    /// keeps its final status from the record; one with a spawn record and
    /// neither reads as interrupted, not running.
    #[tokio::test]
    #[serial_test::serial(workspace_services)]
    async fn after_a_restart_a_finished_child_keeps_its_status_and_a_lost_one_is_interrupted() {
        crate::workspace_services::set_for_tests(None);
        let (_temp, sm) = store().await;
        let parent = "parent-after-restart";
        let finished = spawn_child(&sm, parent, "finished").await;
        let lost = spawn_child(&sm, parent, "lost").await;
        record_spawn_identity(&sm, &finished, None, None)
            .await
            .unwrap();
        record_spawn_identity(&sm, &lost, None, None).await.unwrap();
        record_outcome_at(
            &sm,
            &finished,
            &SubagentResult::from_error("the model refused"),
            Utc::now() - Duration::seconds(5),
        )
        .await
        .unwrap();

        let roster = session_subagents(&sm, parent).await.unwrap();
        assert_eq!(roster.subagents[0].status, SubagentRunStatus::Failed);
        assert_eq!(
            roster.subagents[0].result_line.as_deref(),
            Some("the model refused")
        );
        assert!(roster.subagents[0].ended_at.is_some());
        assert_eq!(roster.subagents[1].status, SubagentRunStatus::Interrupted);
        crate::workspace_services::clear_test_override();
    }

    /// A subagent stopped by a monitor, or stopped because its parent was,
    /// shows why on its row: the side panel, `biorouter session subagents` and
    /// the terminal UI all draw `result_line`, live and after a restart.
    #[test]
    fn a_stopped_subagents_row_says_why() {
        use crate::agents::subagent_handle::ChildStopCause;
        use crate::decision::settings::{MonitorOwner, MonitorStopReason, MonitorStopReport};

        let mut stopped = completed("Adults usually take 200 to 400 mg.");
        stopped.apply_monitor_stop(&MonitorStopReport {
            session_id: "child".into(),
            monitor_id: "gives-a-dose".into(),
            name: "gives a dose".into(),
            set_by: MonitorOwner::Agent,
            set_by_session: Some("parent".into()),
            inherited_from: None,
            question: "Has the assistant stated a specific numeric drug dose?".into(),
            threshold: 0.6,
            below: false,
            p: Some(0.93),
            reason: MonitorStopReason::Fired,
        });
        assert_eq!(outcome_status(&stopped), SubagentRunStatus::Incomplete);
        let line = result_line(&stopped).unwrap();
        assert!(
            line.starts_with(
                "Stopped by monitor 'gives a dose' (set by this chat's agent): P 0.93 reached \
                 its threshold 0.60."
            ),
            "{line}"
        );
        assert!(line.chars().count() <= MAX_RESULT_LINE_CHARS);

        // Live: a finished handle's result.
        let row = derive_row(
            child_row(ExtensionData::new()),
            Some(&LiveState {
                result: Some(stopped.clone()),
                ..live(false)
            }),
            false,
            at(100),
        );
        assert_eq!(row.status, SubagentRunStatus::Incomplete);
        assert_eq!(row.result_line.as_deref(), Some(line.as_str()));

        // The parent was stopped, so the child was.
        let mut cancelled = completed("counted to 12");
        cancelled.mark_cancelled();
        cancelled.note_stop_cause(&ChildStopCause::ParentStopped);
        assert_eq!(outcome_status(&cancelled), SubagentRunStatus::Incomplete);
        assert_eq!(
            result_line(&cancelled).as_deref(),
            Some("Stopped because the person stopped the chat that started this subagent.")
        );
    }

    /// After a restart the durable outcome record carries the same line.
    #[tokio::test]
    async fn a_stopped_subagents_reason_survives_in_its_outcome_record() {
        use crate::agents::subagent_handle::ChildStopCause;

        let (_temp, sm) = store().await;
        let child = spawn_child(&sm, "parent-stopped-record", "stopped").await;
        let mut cancelled = completed("partial");
        cancelled.mark_cancelled();
        cancelled.note_stop_cause(&ChildStopCause::ParentMonitor {
            monitor: "fanout".into(),
        });
        record_outcome_at(&sm, &child, &cancelled, at(5))
            .await
            .unwrap();

        let session = sm.get_session(&child, false).await.unwrap();
        let record = read_outcome_record(&session.extension_data).unwrap();
        assert_eq!(record.status, SubagentRunStatus::Incomplete);
        assert_eq!(
            record.result_line.as_deref(),
            Some("Stopped because monitor 'fanout' stopped the chat that started this subagent.")
        );
    }
}
